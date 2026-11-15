//! Connector between `SolveApp` (`solidcraft-ui-egui`) and `wood::loader::WoodRuntime`.
//!
//! Bridges the native `egui` ribbon tab `"WOOD"`, its feature dialogs, the cutting list window,
//! and the `F5` hot-reload trigger (`target/debug/wood.dll`).

use serde_json::{Value, json};
use solidcraft_engine::Sel;
use wood::{ActiveDialog, EdgePick, FacePick, WoodHost, loader::WoodRuntime, ui::active_dialog_meta};

use crate::SolveApp;

struct SolveAppHost<'a>(&'a mut SolveApp);

impl WoodHost for SolveAppHost<'_> {
    fn execute(&mut self, command: &str, params: Value) -> Result<Value, String> {
        self.0.run(command, params)
    }

    fn batch(&mut self, commands: &[(String, Value)], rollback: bool) -> Result<Value, String> {
        let backup = rollback.then(|| self.0.session.scratch());
        let mut last = Value::Null;
        for (idx, (cmd, params)) in commands.iter().enumerate() {
            match self.0.run(cmd, params.clone()) {
                Ok(val) => last = val,
                Err(err) => {
                    if let Some(prev) = backup {
                        self.0.session = prev;
                    }
                    return Err(format!("Błąd w kroku {idx} ({cmd}): {err}"));
                }
            }
        }
        Ok(last)
    }

    fn inspect_document(&mut self, measure: bool) -> Result<Value, String> {
        self.0.run("document.inspect", json!({ "measure": measure }))
    }

    fn take_selected_faces(&mut self) -> Vec<FacePick> {
        let mut out = Vec::new();
        for item in &self.0.session.selection {
            if let Sel::Face { body, index, point } = item {
                let round3 = |v: f64| (v * 1000.0).round() / 1000.0;
                out.push(FacePick {
                    body: body.clone(),
                    index: *index as u64,
                    point: [round3(point.x), round3(point.y), round3(point.z)],
                });
            }
        }
        if !out.is_empty() {
            let _ = self.0.run("select.clear", json!({}));
        }
        out
    }

    fn enable_face_selection(&mut self) {
        let _ = self.0.run("select.filter", json!({ "mode": "face" }));
        let _ = self.0.run("select.clear", json!({}));
    }

    fn take_selected_edges(&mut self) -> Vec<EdgePick> {
        let mut raw = Vec::new();
        for item in &self.0.session.selection {
            if let Sel::Edge { body, index, point } = item {
                raw.push((body.clone(), *index as u64, *point));
            }
        }
        if raw.is_empty() {
            return Vec::new();
        }
        let _ = self.0.run("select.clear", json!({}));
        let round3 = |v: f64| (v * 1000.0).round() / 1000.0;
        let mut out = Vec::new();
        for (body, index, pt) in raw {
            let mut start = [round3(pt.x), round3(pt.y), round3(pt.z)];
            let mut end = start;
            let mut length = 0.0;
            if let Ok(edges_val) = self.0.run("model.edges", json!({ "body": body }))
                && let Some(edges) = edges_val.get("edges").and_then(Value::as_array)
                && let Some(e) = edges
                    .iter()
                    .find(|e| e.get("index").and_then(Value::as_u64) == Some(index))
            {
                length = e.get("length").and_then(Value::as_f64).unwrap_or(0.0);
                if let Some(s) = e.get("start").and_then(Value::as_array)
                    && s.len() >= 3
                {
                    start = [
                        round3(s[0].as_f64().unwrap_or(pt.x)),
                        round3(s[1].as_f64().unwrap_or(pt.y)),
                        round3(s[2].as_f64().unwrap_or(pt.z)),
                    ];
                }
                if let Some(en) = e.get("end").and_then(Value::as_array)
                    && en.len() >= 3
                {
                    end = [
                        round3(en[0].as_f64().unwrap_or(pt.x)),
                        round3(en[1].as_f64().unwrap_or(pt.y)),
                        round3(en[2].as_f64().unwrap_or(pt.z)),
                    ];
                }
            }
            out.push(EdgePick {
                body,
                index,
                point: [round3(pt.x), round3(pt.y), round3(pt.z)],
                start,
                end,
                length: round3(length),
            });
        }
        out
    }

    fn enable_edge_selection(&mut self) {
        let _ = self.0.run("select.clear", json!({}));
        self.0.ui.pick_bodies = false;
        self.0.ui.pick_components = false;
        self.0.ui.pick_off.retain(|k| k != "edges");
        if !self.0.ui.pick_off.iter().any(|k| k == "faces") {
            self.0.ui.pick_off.push("faces".to_string());
        }
    }

    fn set_status(&mut self, message: &str, is_error: bool) {
        self.0.set_status(message.to_string(), is_error);
    }

    fn open_parameters_dialog(&mut self) {
        self.0.start("parameters.change");
    }
}

/// Per-frame hot-reload check (`F5`) and 3D face selection consumption for the Wood module.
pub fn tick(app: &mut SolveApp, ctx: &egui::Context) {
    let f5_pressed = ctx.input(|i| i.key_pressed(egui::Key::F5));
    let mut runtime = std::mem::take(&mut app.wood);
    {
        let mut host = SolveAppHost(app);
        runtime.poll_hot_reload(f5_pressed, &mut host);
        runtime.consume_viewport_picks(&mut host);
    }
    app.wood = runtime;
}

/// Draws the `"WOOD"` ribbon buttons when the `"WOOD"` tab is active.
pub fn render_ribbon(app: &mut SolveApp, ui: &mut egui::Ui, rect: egui::Rect) {
    let mut runtime = std::mem::take(&mut app.wood);
    {
        let mut host = SolveAppHost(app);
        runtime.draw_ribbon(ui, rect, &mut host);
    }
    if runtime.state.active_dialog.is_some() {
        app.dialog = None;
    }
    app.wood = runtime;
}

/// Renders the active Wood feature dialog (top-right of the 3D viewport) and the Cutting List (`Tabela Rozkroju`) window.
pub fn show_dialogs(app: &mut SolveApp, ctx: &egui::Context) {
    let mut runtime = std::mem::take(&mut app.wood);

    // Wzajemne wykluczanie okien: tylko jedno narzędzie naraz (jak Extrude i Fillet)
    if app.dialog.is_some() && runtime.state.active_dialog.is_some() {
        runtime.state.active_dialog = None;
    }

    if let Some(dialog) = runtime.state.active_dialog {
        app.dialog = None;
        let (title, icon) = active_dialog_meta(dialog);

        let vp = ctx.content_rect();
        let side = crate::frame::Side::Right;
        let top = crate::shell::panel_top(app, vp, vp.bottom(), side);
        let drag_key = side.dialog_offset_key();
        let moved = crate::frame::dock_offset(ctx, drag_key);
        let docked = egui::pos2(side.anchor_x(vp) - 12.0, top);
        let drag_bounds = crate::shell::tool_drag_rect(app, vp);
        let anchor = crate::frame::offset_anchor(docked, moved, drag_bounds, side);

        let mut body_closed = false;
        let card_w = crate::theme::Tokens::get().form_card_width;
        let dialog_w = match dialog {
            ActiveDialog::AdaptBodies => 300.0,
            ActiveDialog::Edgebanding => 290.0,
            ActiveDialog::DecorGrain => 295.0,
            ActiveDialog::Drawer => 295.0,
            ActiveDialog::TDogbone => 285.0,
            _ => 270.0,
        };
        let cards_count = dialog_w / card_w;
        let header_closed = crate::ui_kit::Window::new(egui::Id::new("sc_wood_dialog"), icon, title)
            .placed(egui::Align2::RIGHT_TOP, anchor)
            .cards(cards_count)
            .show_body(ctx, |ui| {
                let mut host = SolveAppHost(app);
                body_closed = runtime.draw_dialog_body(ui, &mut host);
            });
        if header_closed || body_closed {
            if dialog == ActiveDialog::Connectors
                || dialog == ActiveDialog::TDogbone
                || dialog == ActiveDialog::Hinges
                || dialog == ActiveDialog::Drawer
            {
                let mut host = SolveAppHost(app);
                host.enable_face_selection();
                if runtime.state.panels_translucent {
                    let _ = wood::operations::set_panels_translucency(&mut runtime.state, &mut host, false);
                }
                runtime.state.connector_last_edge_pick = None;
                runtime.state.connector_last_face_pick = None;
                runtime.state.tdogbone_last_edge_pick = None;
                runtime.state.tdogbone_last_face_pick = None;
                runtime.state.hinge_face_picks = [None, None];
                runtime.state.drawer_face_picks = [None, None, None, None];
                runtime.state.drawer_validation_info.clear();
                runtime.state.drawer_validation_error = false;
                runtime.state.ghost_cylinders = None;
            }
            runtime.state.ghost_box = None;
            runtime.state.ghost_faces.clear();
            runtime.state.ghost_arrows.clear();
            runtime.state.active_dialog = None;
        }
    }

    if runtime.state.bom_open {
        let closed = crate::ui_kit::Window::new(
            egui::Id::new("sc_wood_bom"),
            "config_table",
            "Tabela Rozkroju (SolidCraft Wood)",
        )
        .cards(3.8)
        .show_body(ctx, |ui| {
            let mut host = SolveAppHost(app);
            runtime.draw_bom_body(ui, &mut host);
        });
        if closed {
            runtime.state.bom_open = false;
        }
    }

    app.wood = runtime;
}

/// Creates a fresh default `WoodRuntime` instance for `SolveApp`.
pub fn new_runtime() -> WoodRuntime {
    WoodRuntime::new()
}

/// Renders a dynamic green translucent 3D ghost box preview for Wood panels before acceptance,
/// or 3D ghost cylinder previews for connectors (dowels = green, screws = cyan).
pub fn paint_viewport_preview(app: &SolveApp, painter: &egui::Painter, proj: &crate::viewport::Proj) {
    use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, StrokeKind, vec2};
    use solidcore_record::Vec3;

    // 0a. Strzałki kierunku usłojenia (widoczne wyłącznie gdy aktywne narzędzie Dekor / Słoje)
    if app.wood.state.active_dialog == Some(ActiveDialog::DecorGrain)
        && !app.wood.state.ghost_arrows.is_empty()
    {
        for arr in &app.wood.state.ghost_arrows {
            let center = Vec3::new(arr.center[0], arr.center[1], arr.center[2]);
            let normal = Vec3::new(arr.normal[0], arr.normal[1], arr.normal[2]);
            let to_cam = if proj.cam.fov > 0.0 {
                proj.cam.eye() - center
            } else {
                proj.cam.back()
            };
            let dot_view =
                normal.x * to_cam.x + normal.y * to_cam.y + normal.z * to_cam.z;
            if dot_view <= 0.0 {
                continue;
            }

            let dir = Vec3::new(arr.dir[0], arr.dir[1], arr.dir[2]);
            let perp = Vec3::new(arr.perp[0], arr.perp[1], arr.perp[2]);
            let half = arr.half_len;
            let head_len = (half * 0.24).clamp(8.0, 26.0);
            let head_w = (half * 0.14).clamp(5.0, 16.0);
            let side_gap = (half * 0.22).clamp(6.0, 22.0);
            let side_half = half * 0.65;

            let col = Color32::from_rgba_unmultiplied(
                arr.color[0],
                arr.color[1],
                arr.color[2],
                arr.color[3],
            );
            let halo_col = if arr.color[0] < 128 {
                Color32::from_rgba_unmultiplied(255, 255, 255, 190)
            } else {
                Color32::from_rgba_unmultiplied(15, 23, 42, 200)
            };
            let stroke_halo = Stroke::new(4.2, halo_col);
            let stroke_main = Stroke::new(2.2, col);
            let stroke_sub = Stroke::new(1.3, col.linear_multiply(0.85));

            let p_a = center - dir * half;
            let p_b = center + dir * half;
            if let (Some(sa), Some(sb)) = (proj.to_screen(p_a), proj.to_screen(p_b)) {
                painter.line_segment([sa, sb], stroke_halo);
                painter.line_segment([sa, sb], stroke_main);

                // Groty na obu końcach osi usłojenia
                let heads = [
                    (p_a, p_a + dir * head_len + perp * head_w, sa),
                    (p_a, p_a + dir * head_len - perp * head_w, sa),
                    (p_b, p_b - dir * head_len + perp * head_w, sb),
                    (p_b, p_b - dir * head_len - perp * head_w, sb),
                ];
                for (_tip, wing, s_tip) in heads {
                    if let Some(sw) = proj.to_screen(wing) {
                        painter.line_segment([s_tip, sw], stroke_halo);
                        painter.line_segment([s_tip, sw], stroke_main);
                    }
                }
            }

            // Dwie pomocnicze linie równoległe podkreślające kierunek słojów
            for sign in [-1.0_f64, 1.0_f64] {
                let off = perp * (side_gap * sign);
                if let (Some(s1), Some(s2)) = (
                    proj.to_screen(center + off - dir * side_half),
                    proj.to_screen(center + off + dir * side_half),
                ) {
                    painter.line_segment([s1, s2], stroke_sub);
                }
            }

            if let Some(sc) = proj.to_screen(center) {
                let galley = painter.layout_no_wrap(
                    arr.label.clone(),
                    FontId::proportional(10.5),
                    col,
                );
                let pill_rect = Rect::from_center_size(sc, galley.size() + vec2(8.0, 3.0));
                painter.rect_filled(
                    pill_rect,
                    3.0,
                    if arr.color[0] < 128 {
                        Color32::from_rgba_unmultiplied(248, 250, 252, 225)
                    } else {
                        Color32::from_rgba_unmultiplied(15, 23, 42, 220)
                    },
                );
                painter.rect_stroke(pill_rect, 3.0, Stroke::new(1.0, col), StrokeKind::Inside);
                painter.galley(pill_rect.min + vec2(4.0, 1.5), galley, col);
            }
        }
        return;
    }

    // 0. Delikatne podświetlenie brzegów formatek w narzędziu Obrzeża (Oklejanie)
    if !app.wood.state.ghost_faces.is_empty() {
        for gf in &app.wood.state.ghost_faces {
            let color_fill = Color32::from_rgba_unmultiplied(
                gf.color_fill[0],
                gf.color_fill[1],
                gf.color_fill[2],
                gf.color_fill[3],
            );
            let color_stroke = Color32::from_rgba_unmultiplied(
                gf.color_stroke[0],
                gf.color_stroke[1],
                gf.color_stroke[2],
                gf.color_stroke[3],
            );
            let stroke = Stroke::new(1.4, color_stroke);
            let s0 = proj.to_screen(Vec3::new(gf.corners[0][0], gf.corners[0][1], gf.corners[0][2]));
            let s1 = proj.to_screen(Vec3::new(gf.corners[1][0], gf.corners[1][1], gf.corners[1][2]));
            let s2 = proj.to_screen(Vec3::new(gf.corners[2][0], gf.corners[2][1], gf.corners[2][2]));
            let s3 = proj.to_screen(Vec3::new(gf.corners[3][0], gf.corners[3][1], gf.corners[3][2]));
            if let (Some(p0), Some(p1), Some(p2), Some(p3)) = (s0, s1, s2, s3) {
                for shape in crate::viewport::projected_polygon(&[p0, p1, p2, p3], color_fill, Stroke::NONE) {
                    painter.add(shape);
                }
                painter.line_segment([p0, p1], stroke);
                painter.line_segment([p1, p2], stroke);
                painter.line_segment([p2, p3], stroke);
                painter.line_segment([p3, p0], stroke);
            }
        }
        return;
    }

    // 1. Podgląd cylindrów złączy (Kołki = zielone, Wkręty = błękitne) oraz zawiasów
    if let Some(ghost_cyl) = &app.wood.state.ghost_cylinders {
        let cross_prod = |a: Vec3, b: Vec3| -> Vec3 {
            Vec3::new(
                a.y * b.z - a.z * b.y,
                a.z * b.x - a.x * b.z,
                a.x * b.y - a.y * b.x,
            )
        };

        if !ghost_cyl.items.is_empty() {
            for item in &ghost_cyl.items {
                let color_stroke = Color32::from_rgba_unmultiplied(
                    item.color_stroke[0],
                    item.color_stroke[1],
                    item.color_stroke[2],
                    item.color_stroke[3],
                );
                let color_fill = Color32::from_rgba_unmultiplied(
                    item.color_fill[0],
                    item.color_fill[1],
                    item.color_fill[2],
                    item.color_fill[3],
                );
                let stroke = Stroke::new(1.8, color_stroke);

                let len_sq = item.axis[0] * item.axis[0] + item.axis[1] * item.axis[1] + item.axis[2] * item.axis[2];
                let axis_v = if len_sq > 1e-8 {
                    let inv_len = 1.0 / len_sq.sqrt();
                    Vec3::new(item.axis[0] * inv_len, item.axis[1] * inv_len, item.axis[2] * inv_len)
                } else {
                    Vec3::Z
                };

                let (perp1, perp2) = {
                    let p1_raw = if axis_v.z.abs() < 0.9 {
                        cross_prod(axis_v, Vec3::Z)
                    } else {
                        cross_prod(axis_v, Vec3::X)
                    };
                    let p1_len = (p1_raw.x * p1_raw.x + p1_raw.y * p1_raw.y + p1_raw.z * p1_raw.z).sqrt();
                    let p1 = if p1_len > 1e-8 { p1_raw * (1.0 / p1_len) } else { Vec3::X };

                    let p2_raw = cross_prod(axis_v, p1);
                    let p2_len = (p2_raw.x * p2_raw.x + p2_raw.y * p2_raw.y + p2_raw.z * p2_raw.z).sqrt();
                    let p2 = if p2_len > 1e-8 { p2_raw * (1.0 / p2_len) } else { Vec3::Y };
                    (p1, p2)
                };

                let bot_pt = Vec3::new(item.base_center[0], item.base_center[1], item.base_center[2]);
                let top_pt = bot_pt + axis_v * item.depth;

                const SEGMENTS: usize = 12;
                let mut bot_pts = Vec::with_capacity(SEGMENTS);
                let mut top_pts = Vec::with_capacity(SEGMENTS);

                for i in 0..SEGMENTS {
                    let theta = (i as f64) * std::f64::consts::TAU / (SEGMENTS as f64);
                    let radial = perp1 * (theta.cos() * item.radius) + perp2 * (theta.sin() * item.radius);
                    bot_pts.push(bot_pt + radial);
                    top_pts.push(top_pt + radial);
                }

                let bot_screen: Vec<Option<Pos2>> = bot_pts.iter().map(|p| proj.to_screen(*p)).collect();
                let top_screen: Vec<Option<Pos2>> = top_pts.iter().map(|p| proj.to_screen(*p)).collect();

                // Wypełnienie podstaw cylindra (dolna i górna podstawa)
                let valid_bot: Vec<Pos2> = bot_screen.iter().filter_map(|&p| p).collect();
                if valid_bot.len() == SEGMENTS {
                    for shape in crate::viewport::projected_polygon(&valid_bot, color_fill, Stroke::NONE) {
                        painter.add(shape);
                    }
                }
                let valid_top: Vec<Pos2> = top_screen.iter().filter_map(|&p| p).collect();
                if valid_top.len() == SEGMENTS {
                    for shape in crate::viewport::projected_polygon(&valid_top, color_fill, Stroke::NONE) {
                        painter.add(shape);
                    }
                }

                // Rysuj ścianki boczne cylindra
                for i in 0..SEGMENTS {
                    let next = (i + 1) % SEGMENTS;
                    if let (Some(b0), Some(b1), Some(t1), Some(t0)) = (
                        bot_screen[i],
                        bot_screen[next],
                        top_screen[next],
                        top_screen[i],
                    ) {
                        for shape in crate::viewport::projected_polygon(&[b0, b1, t1, t0], color_fill, Stroke::NONE) {
                            painter.add(shape);
                        }
                        painter.line_segment([b0, b1], stroke);
                        painter.line_segment([t0, t1], stroke);
                        if i % 3 == 0 {
                            painter.line_segment([b0, t0], stroke);
                        }
                    }
                }

                if !item.label.is_empty() {
                    let mid_pt = bot_pt + axis_v * (item.depth * 0.5);
                    if let Some(cp) = proj.to_screen(mid_pt) {
                        painter.text(
                            cp,
                            Align2::CENTER_CENTER,
                            &item.label,
                            FontId::proportional(11.0),
                            color_stroke,
                        );
                    }
                }
            }
            if app.wood.state.ghost_box.is_none() {
                return;
            }
        } else {

        let (color_fill, color_stroke, label_prefix) = if ghost_cyl.is_screw {
            // Wkręty: błękitny
            (
                Color32::from_rgba_unmultiplied(56, 189, 248, 60),
                Color32::from_rgb(56, 189, 248),
                "Wkręt",
            )
        } else {
            // Kołki: zielony
            (
                Color32::from_rgba_unmultiplied(34, 197, 94, 60),
                Color32::from_rgb(34, 197, 94),
                "Kołek",
            )
        };
        let stroke = Stroke::new(1.8, color_stroke);

        let cross_prod = |a: Vec3, b: Vec3| -> Vec3 {
            Vec3::new(
                a.y * b.z - a.z * b.y,
                a.z * b.x - a.x * b.z,
                a.x * b.y - a.y * b.x,
            )
        };

        let len_sq = ghost_cyl.axis[0] * ghost_cyl.axis[0] + ghost_cyl.axis[1] * ghost_cyl.axis[1] + ghost_cyl.axis[2] * ghost_cyl.axis[2];
        let axis_v = if len_sq > 1e-8 {
            let inv_len = 1.0 / len_sq.sqrt();
            Vec3::new(ghost_cyl.axis[0] * inv_len, ghost_cyl.axis[1] * inv_len, ghost_cyl.axis[2] * inv_len)
        } else {
            Vec3::Z
        };

        let (perp1, perp2) = {
            let p1_raw = if axis_v.z.abs() < 0.9 {
                cross_prod(axis_v, Vec3::Z)
            } else {
                cross_prod(axis_v, Vec3::X)
            };
            let p1_len = (p1_raw.x * p1_raw.x + p1_raw.y * p1_raw.y + p1_raw.z * p1_raw.z).sqrt();
            let p1 = if p1_len > 1e-8 { p1_raw * (1.0 / p1_len) } else { Vec3::X };

            let p2_raw = cross_prod(axis_v, p1);
            let p2_len = (p2_raw.x * p2_raw.x + p2_raw.y * p2_raw.y + p2_raw.z * p2_raw.z).sqrt();
            let p2 = if p2_len > 1e-8 { p2_raw * (1.0 / p2_len) } else { Vec3::Y };
            (p1, p2)
        };

        for (idx, &c_arr) in ghost_cyl.centers.iter().enumerate() {
            let center_seam = Vec3::new(c_arr[0], c_arr[1], c_arr[2]);
            let bot_pt = center_seam - axis_v * ghost_cyl.depth_face;
            let top_pt = center_seam + axis_v * ghost_cyl.depth_edge;

            const SEGMENTS: usize = 12;
            let mut bot_pts = Vec::with_capacity(SEGMENTS);
            let mut top_pts = Vec::with_capacity(SEGMENTS);

            for i in 0..SEGMENTS {
                let theta = (i as f64) * std::f64::consts::TAU / (SEGMENTS as f64);
                let radial = perp1 * (theta.cos() * ghost_cyl.radius) + perp2 * (theta.sin() * ghost_cyl.radius);
                bot_pts.push(bot_pt + radial);
                top_pts.push(top_pt + radial);
            }

            let bot_screen: Vec<Option<Pos2>> = bot_pts.iter().map(|p| proj.to_screen(*p)).collect();
            let top_screen: Vec<Option<Pos2>> = top_pts.iter().map(|p| proj.to_screen(*p)).collect();

            // Wypełnienie podstaw cylindra (dolna i górna podstawa)
            let valid_bot: Vec<Pos2> = bot_screen.iter().filter_map(|&p| p).collect();
            if valid_bot.len() == SEGMENTS {
                for shape in crate::viewport::projected_polygon(&valid_bot, color_fill, Stroke::NONE) {
                    painter.add(shape);
                }
            }
            let valid_top: Vec<Pos2> = top_screen.iter().filter_map(|&p| p).collect();
            if valid_top.len() == SEGMENTS {
                for shape in crate::viewport::projected_polygon(&valid_top, color_fill, Stroke::NONE) {
                    painter.add(shape);
                }
            }

            // Rysuj ścianki boczne cylindra
            for i in 0..SEGMENTS {
                let next = (i + 1) % SEGMENTS;
                if let (Some(b0), Some(b1), Some(t1), Some(t0)) = (
                    bot_screen[i],
                    bot_screen[next],
                    top_screen[next],
                    top_screen[i],
                ) {
                    for shape in crate::viewport::projected_polygon(&[b0, b1, t1, t0], color_fill, Stroke::NONE) {
                        painter.add(shape);
                    }
                    painter.line_segment([b0, b1], stroke);
                    painter.line_segment([t0, t1], stroke);
                    if i % 3 == 0 {
                        painter.line_segment([b0, t0], stroke);
                    }
                }
            }

            let mid_pt = center_seam + axis_v * ((ghost_cyl.depth_edge - ghost_cyl.depth_face) * 0.5);
            if let Some(cp) = proj.to_screen(mid_pt) {
                painter.text(
                    cp,
                    Align2::CENTER_CENTER,
                    format!("{} #{}", label_prefix, idx + 1),
                    FontId::proportional(11.0),
                    color_stroke,
                );
            }
        }
        }
        if app.wood.state.ghost_box.is_none() {
            return;
        }
    }

    // 2. Podgląd formatki (Duch formatki - box lub wiele półek przy podziale)
    let Some(ghost) = &app.wood.state.ghost_box else {
        return;
    };
    let plane = ghost.plane;

    // 0.2 opacity green fill: (34, 197, 94, 51)
    let fill = Color32::from_rgba_unmultiplied(34, 197, 94, 51);
    let stroke = Stroke::new(1.8, Color32::from_rgb(34, 197, 94));
    let text_color = Color32::from_rgb(22, 128, 61); // ciemniejszy zielony niż obrys (34, 197, 94)

    let faces = [
        [0, 1, 2, 3], // Bottom
        [4, 5, 6, 7], // Top
        [0, 1, 5, 4], // Front
        [3, 2, 6, 7], // Back
        [0, 3, 7, 4], // Left
        [1, 2, 6, 5], // Right
    ];

    let edges = [
        (0, 1), (1, 2), (2, 3), (3, 0),
        (4, 5), (5, 6), (6, 7), (7, 4),
        (0, 4), (1, 5), (2, 6), (3, 7),
    ];

    let mut all_boxes = Vec::with_capacity(1 + ghost.extra_boxes.len());
    all_boxes.push((ghost.min, ghost.max));
    all_boxes.extend_from_slice(&ghost.extra_boxes);

    for (b_idx, &(b_min, b_max)) in all_boxes.iter().enumerate() {
        let [x0, y0, z0] = b_min;
        let [x1, y1, z1] = b_max;

        let pts = [
            Vec3::new(x0, y0, z0), // 0
            Vec3::new(x1, y0, z0), // 1
            Vec3::new(x1, y1, z0), // 2
            Vec3::new(x0, y1, z0), // 3
            Vec3::new(x0, y0, z1), // 4
            Vec3::new(x1, y0, z1), // 5
            Vec3::new(x1, y1, z1), // 6
            Vec3::new(x0, y1, z1), // 7
        ];

        let s: Vec<Option<Pos2>> = pts.iter().map(|p| proj.to_screen(*p)).collect();

        for f in &faces {
            if let (Some(p0), Some(p1), Some(p2), Some(p3)) = (s[f[0]], s[f[1]], s[f[2]], s[f[3]]) {
                for shape in crate::viewport::projected_polygon(&[p0, p1, p2, p3], fill, Stroke::NONE) {
                    painter.add(shape);
                }
            }
        }

        for (a, b) in edges {
            if let (Some(p0), Some(p1)) = (s[a], s[b]) {
                painter.line_segment([p0, p1], stroke);
            }
        }

        let center = Vec3::new((x0 + x1) * 0.5, (y0 + y1) * 0.5, (z0 + z1) * 0.5);

        if let Some(cp) = proj.to_screen(center) {
            let lx = (x1 - x0).abs();
            let ly = (y1 - y0).abs();
            let lz = (z1 - z0).abs();
            let label = if all_boxes.len() > 1 {
                format!("#{} [{plane}]  {:.0} × {:.0} × {:.0} mm", b_idx + 1, lx, ly, lz)
            } else {
                format!("[{plane}]  {:.0} × {:.0} × {:.0} mm", lx, ly, lz)
            };
            let galley = painter.layout_no_wrap(label, FontId::proportional(12.5), text_color);
            let pill_rect = Rect::from_center_size(cp, galley.size() + vec2(10.0, 4.0));
            painter.rect_filled(pill_rect, 4.0, Color32::from_rgba_unmultiplied(20, 24, 30, 210));
            painter.rect_stroke(pill_rect, 4.0, Stroke::new(1.0, text_color), StrokeKind::Inside);
            painter.galley(pill_rect.min + vec2(5.0, 2.0), galley, text_color);
        }
    }

    let [x0, y0, z0] = ghost.min;
    let [x1, y1, z1] = ghost.max;
    let center = Vec3::new((x0 + x1) * 0.5, (y0 + y1) * 0.5, (z0 + z1) * 0.5);

    // Dodatkowe napisy na brzegach gdy margines > 0
    let min_a = ghost.min[ghost.axis1];
    let max_a = ghost.max[ghost.axis1];
    let min_b = ghost.min[ghost.axis2];
    let max_b = ghost.max[ghost.axis2];

    let margin_specs = [
        (ghost.margins[0], ghost.axis1, min_a),
        (ghost.margins[1], ghost.axis1, max_a),
        (ghost.margins[2], ghost.axis2, min_b),
        (ghost.margins[3], ghost.axis2, max_b),
    ];

    for (val, axis, pos_on_axis) in margin_specs {
        if val > 0.05 {
            let mut edge_pt = center;
            match axis {
                0 => edge_pt.x = pos_on_axis,
                1 => edge_pt.y = pos_on_axis,
                _ => edge_pt.z = pos_on_axis,
            }
            if let Some(sp) = proj.to_screen(edge_pt) {
                let badge = format!("-{:.0} mm", val);
                let badge_galley = painter.layout_no_wrap(badge, FontId::proportional(11.0), text_color);
                let badge_rect = Rect::from_center_size(sp, badge_galley.size() + vec2(8.0, 3.0));
                painter.rect_filled(badge_rect, 3.0, Color32::from_rgba_unmultiplied(20, 24, 30, 220));
                painter.rect_stroke(badge_rect, 3.0, Stroke::new(1.0, text_color), StrokeKind::Inside);
                painter.galley(badge_rect.min + vec2(4.0, 1.5), badge_galley, text_color);
            }
        }
    }
}
