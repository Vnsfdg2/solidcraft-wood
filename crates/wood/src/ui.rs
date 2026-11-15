//! Native `egui` user interface for SolidCraft Wood:
//! - Ribbon toolbar for the `"WOOD"` tab with vector icons
//! - Feature dialog cards (`Kubatura`, `Formatka Automatyczna`, `Wolna Formatka 1→2→3`, `Złożenie i Jointy`)
//! - Cutting list table (`Tabela Rozkroju` / BOM)

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::operations::{
    adapt_selected_bodies, apply_bom_range_cabinet, apply_bom_renames, apply_edgebanding,
    bind_all_joints, build_panel_name, clear_panel_edgebanding, compute_edgeband_code_and_length,
    compute_tdogbone_plan, consume_adapt_picks, create_auto_panel, create_connectors,
    create_drawer, create_free_panel, create_hinges, create_kubatura, create_tdogbone,
    cutting_list_to_csv, export_panels_to_step, extract_panel_boxes, handle_decor_face_pick,
    handle_edgeband_face_pick, load_decors_from_metadata, load_edgebands_from_metadata,
    narrow_edge_side_label, next_formatka_name_with_state, parse_f64_or, refresh_cutting_list,
    scan_adapt_candidates, set_panels_translucency, toggle_kubatura_visibility,
    toggle_panels_translucency, update_ghost_preview, update_kubatura,
};
use crate::{
    ActiveDialog, AutoPanelDirection, CncJointLibrary, ConnectorLibrary, DecorLibrary, DecorMode,
    DrawerLibrary, EdgebandAutoRule, EdgebandLibrary, EdgebandMode, FreePanelMode, HingeLibrary,
    WoodHost, WoodState,
};

// SolidCraft dark theme semantic colors
const COL_BG_HOVER: Color32 = Color32::from_rgb(35, 41, 51);
const COL_BG_ACTIVE: Color32 = Color32::from_rgb(30, 50, 74);
const COL_BG_INPUT: Color32 = Color32::from_rgb(18, 22, 28);
const COL_BORDER: Color32 = Color32::from_rgb(42, 49, 61);
const COL_BLUE: Color32 = Color32::from_rgb(59, 130, 246);
const COL_TEXT: Color32 = Color32::from_rgb(229, 233, 240);
const COL_LABEL: Color32 = Color32::from_rgb(154, 167, 184);
const COL_MUTED: Color32 = Color32::from_rgb(108, 120, 136);
const COL_GREEN: Color32 = Color32::from_rgb(74, 222, 128);
const COL_RED: Color32 = Color32::from_rgb(248, 113, 113);
const COL_GOLD: Color32 = Color32::from_rgb(243, 198, 119);

#[derive(Clone, Copy, PartialEq, Eq)]
enum RibbonTool {
    Kubatura,
    ToggleKubatura,
    AutoPanel,
    FreePanel,
    AdaptBodies,
    Edgebanding,
    DecorGrain,
    Connectors,
    Hinges,
    Drawer,
    TDogbone,
    Joints,
    CuttingList,
    Parameters,
}

/// Consumes any newly selected 3D faces or edges from the viewport into the active Wood dialog's pick slots.
pub fn consume_viewport_picks(state: &mut WoodState, host: &mut dyn WoodHost) {
    if state.active_dialog == Some(ActiveDialog::AdaptBodies) {
        consume_adapt_picks(state, host);
        return;
    }

    if state.active_dialog == Some(ActiveDialog::Edgebanding) {
        let faces = host.take_selected_faces();
        if let Some(last) = faces.last() {
            match handle_edgeband_face_pick(last, state, host) {
                Ok(msg) => {
                    state.status_message = msg.clone();
                    state.status_error = false;
                    host.set_status(&msg, false);
                }
                Err(err) => {
                    state.status_message = err.clone();
                    state.status_error = true;
                    host.set_status(&err, true);
                }
            }
        }
        return;
    }

    if state.active_dialog == Some(ActiveDialog::DecorGrain) {
        let faces = host.take_selected_faces();
        if let Some(last) = faces.last() {
            match handle_decor_face_pick(last, state, host) {
                Ok(msg) => {
                    state.status_message = msg.clone();
                    state.status_error = false;
                    host.set_status(&msg, false);
                }
                Err(err) => {
                    state.status_message = err.clone();
                    state.status_error = true;
                    host.set_status(&err, true);
                }
            }
        }
        return;
    }

    if state.active_dialog == Some(ActiveDialog::Connectors) {
        if state.connector_active_slot == 0 {
            let edges = host.take_selected_edges();
            if let Some(last) = edges.last() {
                state.connector_edge_pick = Some(last.clone());
                // Po wybraniu krawędzi automatycznie przejdź do Kroku 2 (wybór ściany)
                state.connector_active_slot = 1;
                host.enable_face_selection();
                update_ghost_preview(state, host);
            }
        } else {
            let faces = host.take_selected_faces();
            let valid = faces.into_iter().filter(|f| f.body != "Kubatura").collect::<Vec<_>>();
            if let Some(last) = valid.last() {
                state.connector_face_pick = Some(last.clone());
                update_ghost_preview(state, host);
            }
        }
        return;
    }

    if state.active_dialog == Some(ActiveDialog::TDogbone) {
        if state.tdogbone_active_slot == 0 {
            let edges = host.take_selected_edges();
            if let Some(last) = edges.last() {
                state.tdogbone_edge_pick = Some(last.clone());
                state.tdogbone_active_slot = 1;
                host.enable_face_selection();
                update_ghost_preview(state, host);
            }
        } else {
            let faces = host.take_selected_faces();
            let valid = faces.into_iter().filter(|f| f.body != "Kubatura").collect::<Vec<_>>();
            if let Some(last) = valid.last() {
                state.tdogbone_face_pick = Some(last.clone());
                update_ghost_preview(state, host);
            }
        }
        return;
    }

    if state.active_dialog == Some(ActiveDialog::Hinges) {
        let faces = host.take_selected_faces();
        let valid = faces.into_iter().filter(|f| f.body != "Kubatura").collect::<Vec<_>>();
        if let Some(first) = valid.into_iter().next() {
            let slot = state.hinge_active_slot;
            state.hinge_face_picks[slot] = Some(first);
            if slot == 0 {
                state.hinge_active_slot = 1;
                host.enable_face_selection();
            }
            update_ghost_preview(state, host);
        }
        return;
    }

    if state.active_dialog == Some(ActiveDialog::Drawer) {
        let faces = host.take_selected_faces();
        let valid = faces.into_iter().filter(|f| f.body != "Kubatura").collect::<Vec<_>>();
        if let Some(first) = valid.into_iter().next() {
            let slot = state.drawer_active_slot.min(3);
            state.drawer_face_picks[slot] = Some(first);
            if slot < 3 {
                state.drawer_active_slot = slot + 1;
            }
            host.enable_face_selection();
            update_ghost_preview(state, host);
        }
        return;
    }

    if !matches!(state.active_dialog, Some(ActiveDialog::AutoPanel | ActiveDialog::FreePanel)) {
        return;
    }
    let picks = host.take_selected_faces();
    if picks.is_empty() {
        return;
    }

    match state.active_dialog {
        Some(ActiveDialog::AutoPanel) => {
            if let Some(last) = picks.last() {
                if state.auto_active_slot == 0 || state.auto_face_pick.is_none() {
                    state.auto_face_pick = Some(last.clone());
                    state.auto_active_slot = 1;
                } else {
                    state.auto_face_pick_2 = Some(last.clone());
                    state.auto_direction = AutoPanelDirection::Center;
                }
            }
            update_ghost_preview(state, host);
        }
        Some(ActiveDialog::FreePanel) => {
            let slot = state.active_pick_slot.min(2);
            let is_two_faces = state.free_mode == FreePanelMode::TwoFaces;
            let max_for_slot = match slot {
                0 => 2,
                1 => if is_two_faces { 2 } else { 1 },
                _ => 1,
            };

            if slot == 1 && !is_two_faces {
                // Tryb "1 bok + wymiar": natychmiast ustaw 1 ścianę i przejdź do kroku 3
                if let Some(pick) = picks.last() {
                    state.free_picks[1] = vec![pick.clone()];
                    state.active_pick_slot = 2;
                }
                update_ghost_preview(state, host);
                return;
            }

            if slot == 2 {
                // Krok 3: powierzchnia przyklejenia zawsze 1 ściana
                if let Some(pick) = picks.last() {
                    state.free_picks[2] = vec![pick.clone()];
                }
                update_ghost_preview(state, host);
                return;
            }

            if let Some(slot_vec) = state.free_picks.get_mut(slot) {
                for pick in picks {
                    let already = slot_vec.iter().any(|p| p.body == pick.body && p.index == pick.index);
                    if !already && slot_vec.len() < max_for_slot {
                        slot_vec.push(pick);
                    }
                }
            }

            let cur_len = state.free_picks.get(slot).map_or(0, Vec::len);
            if cur_len >= max_for_slot {
                let need1 = if is_two_faces { 2 } else { 1 };
                if slot == 0 && state.free_picks[1].len() < need1 {
                    state.active_pick_slot = 1;
                } else if (slot == 0 || slot == 1) && state.free_picks[2].is_empty() {
                    state.active_pick_slot = 2;
                }
            }
            update_ghost_preview(state, host);
        }
        _ => {}
    }
}

/// Renders the `"WOOD"` ribbon toolbar inside `rect`.
pub fn draw_ribbon(ui: &mut Ui, rect: Rect, state: &mut WoodState, host: &mut dyn WoodHost) {
    let painter = ui.painter().clone();
    let sections: &[(&str, &[(RibbonTool, &str)])] = &[
        (
            "Gabaryt",
            &[
                (RibbonTool::Kubatura, "Kubatura"),
                (RibbonTool::ToggleKubatura, "Pokaż/Ukryj"),
            ],
        ),
        (
            "Formatki",
            &[
                (RibbonTool::AutoPanel, "Quick Formatka"),
                (RibbonTool::FreePanel, "FORMATKA"),
                (RibbonTool::AdaptBodies, "Adaptuj Bryły"),
                (RibbonTool::Edgebanding, "Obrzeża"),
                (RibbonTool::DecorGrain, "Dekor / Słoje"),
            ],
        ),
        (
            "Złożenie",
            &[
                (RibbonTool::Connectors, "Złącza"),
                (RibbonTool::TDogbone, "T-Dogbone"),
                (RibbonTool::Hinges, "Zawiasy"),
                (RibbonTool::Drawer, "DRAWER"),
                (RibbonTool::Joints, "Połącz Jointami"),
            ],
        ),
        (
            "Zestawienie",
            &[
                (RibbonTool::CuttingList, "Tabela Rozkroju"),
                (RibbonTool::Parameters, "Parametry fx"),
            ],
        ),
    ];

    let mut x = rect.left() + 10.0;
    let top_y = rect.top() + 3.0;
    let btn_w = 78.0;
    let btn_h = 58.0;
    let label_y = rect.top() + 72.0;

    for (sec_label, items) in sections {
        let sec_start_x = x;
        for (tool, title) in *items {
            let btn_rect = Rect::from_min_size(pos2(x, top_y), vec2(btn_w, btn_h));
            let resp = ui.interact(btn_rect, ui.id().with(("wood_ribbon_btn", *title)), Sense::click());

            let is_active = match tool {
                RibbonTool::Kubatura => state.active_dialog == Some(ActiveDialog::Kubatura),
                RibbonTool::ToggleKubatura => state.kubatura_hidden,
                RibbonTool::AutoPanel => state.active_dialog == Some(ActiveDialog::AutoPanel),
                RibbonTool::FreePanel => state.active_dialog == Some(ActiveDialog::FreePanel),
                RibbonTool::AdaptBodies => state.active_dialog == Some(ActiveDialog::AdaptBodies),
                RibbonTool::Edgebanding => state.active_dialog == Some(ActiveDialog::Edgebanding),
                RibbonTool::DecorGrain => state.active_dialog == Some(ActiveDialog::DecorGrain),
                RibbonTool::Connectors => state.active_dialog == Some(ActiveDialog::Connectors),
                RibbonTool::TDogbone => state.active_dialog == Some(ActiveDialog::TDogbone),
                RibbonTool::Hinges => state.active_dialog == Some(ActiveDialog::Hinges),
                RibbonTool::Drawer => state.active_dialog == Some(ActiveDialog::Drawer),
                RibbonTool::Joints => state.active_dialog == Some(ActiveDialog::Joints),
                RibbonTool::CuttingList => state.bom_open,
                RibbonTool::Parameters => false,
            };

            if is_active {
                painter.rect(btn_rect, 4.0, COL_BG_ACTIVE, Stroke::new(1.0, COL_BLUE), StrokeKind::Inside);
            } else if resp.hovered() {
                painter.rect(
                    btn_rect,
                    4.0,
                    COL_BG_HOVER,
                    Stroke::new(1.0, Color32::from_rgb(52, 61, 75)),
                    StrokeKind::Inside,
                );
            }

            let center = pos2(btn_rect.center().x, top_y + 20.0);
            paint_tool_icon(&painter, *tool, center);

            painter.text(
                pos2(btn_rect.center().x, top_y + 45.0),
                Align2::CENTER_CENTER,
                *title,
                FontId::proportional(11.0),
                COL_TEXT,
            );

            if resp.clicked() {
                trigger_ribbon_tool(*tool, state, host);
            }

            x += btn_w + 4.0;
        }

        let sec_end_x = x - 4.0;
        let sec_cx = (sec_start_x + sec_end_x) * 0.5;
        painter.text(
            pos2(sec_cx, label_y),
            Align2::CENTER_CENTER,
            format!("{sec_label} \u{25be}"),
            FontId::proportional(10.5),
            COL_MUTED,
        );

        let sep_x = sec_end_x + 8.0;
        painter.line_segment(
            [pos2(sep_x, rect.top() + 5.0), pos2(sep_x, rect.top() + 76.0)],
            Stroke::new(1.0, COL_BORDER),
        );
        x = sep_x + 10.0;
    }

    // Hot-reload status indicator on the right side of the Wood ribbon
    if rect.right() - x > 140.0 {
        let badge_rect = Rect::from_min_size(pos2(rect.right() - 150.0, top_y + 8.0), vec2(138.0, 26.0));
        let badge_resp = ui.interact(badge_rect, ui.id().with("wood_hot_reload_btn"), Sense::click());
        let bg = if badge_resp.hovered() { COL_BG_HOVER } else { COL_BG_INPUT };
        painter.rect(badge_rect, 4.0, bg, Stroke::new(1.0, COL_BORDER), StrokeKind::Inside);
        let label = if state.hot_reload_generation > 0 {
            format!("DLL Reload #{} (F5)", state.hot_reload_generation)
        } else {
            "Wood Native (F5)".to_string()
        };
        painter.text(
            badge_rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(10.5),
            COL_LABEL,
        );
        if badge_resp.on_hover_text("Przeładuj dynamicznie bibliotekę target/debug/wood.dll (Skrót: F5)").clicked() {
            state.reload_requested = true;
        }
    }
}

fn trigger_ribbon_tool(tool: RibbonTool, state: &mut WoodState, host: &mut dyn WoodHost) {
    // Jeśli przełączamy ze Złączy, T-Dogbone lub Zawiasów na inne narzędzie, przywróć nieprzezroczystość 1.0 i wyczyść duchy/pamięć
    if (state.active_dialog == Some(ActiveDialog::Connectors) && tool != RibbonTool::Connectors)
        || (state.active_dialog == Some(ActiveDialog::TDogbone) && tool != RibbonTool::TDogbone)
        || (state.active_dialog == Some(ActiveDialog::Hinges) && tool != RibbonTool::Hinges)
        || (state.active_dialog == Some(ActiveDialog::Drawer) && tool != RibbonTool::Drawer)
    {
        if state.panels_translucent {
            let _ = set_panels_translucency(state, host, false);
        }
        state.connector_last_edge_pick = None;
        state.connector_last_face_pick = None;
        state.tdogbone_last_edge_pick = None;
        state.tdogbone_last_face_pick = None;
        state.hinge_face_picks = [None, None];
        state.drawer_face_picks = [None, None, None, None];
        state.drawer_validation_info.clear();
        state.drawer_validation_error = false;
        state.ghost_cylinders = None;
        state.ghost_box = None;
        host.enable_face_selection();
    }

    if state.active_dialog == Some(ActiveDialog::DecorGrain) && tool != RibbonTool::DecorGrain {
        state.ghost_arrows.clear();
    }

    state.status_message.clear();
    state.status_error = false;
    match tool {
        RibbonTool::Kubatura => {
            state.active_dialog = Some(ActiveDialog::Kubatura);
        }
        RibbonTool::ToggleKubatura => {
            run_and_report(state, host, |s, h| toggle_kubatura_visibility(s, h).map(|msg| msg));
        }
        RibbonTool::AutoPanel => {
            state.auto_face_pick = None;
            state.auto_face_pick_2 = None;
            state.auto_active_slot = 0;
            state.auto_direction = AutoPanelDirection::Inside;
            state.auto_division_count = "1".to_string();
            state.auto_clearances_info.clear();
            state.auto_clearances_error = false;
            state.auto_name = next_formatka_name_with_state(state, host);
            host.enable_face_selection();
            state.active_dialog = Some(ActiveDialog::AutoPanel);
        }
        RibbonTool::FreePanel => {
            state.free_picks = [Vec::new(), Vec::new(), Vec::new()];
            state.active_pick_slot = 0;
            state.free_name = next_formatka_name_with_state(state, host);
            host.enable_face_selection();
            state.active_dialog = Some(ActiveDialog::FreePanel);
        }
        RibbonTool::AdaptBodies => {
            host.enable_face_selection();
            let _ = run_and_report(state, host, scan_adapt_candidates);
            state.active_dialog = Some(ActiveDialog::AdaptBodies);
        }
        RibbonTool::Edgebanding => {
            if state.panel_edgebands.is_empty() {
                load_edgebands_from_metadata(state, host);
            }
            state.staged_edgebands = state.panel_edgebands.clone();
            state.ghost_box = None;
            state.ghost_cylinders = None;
            if !state.kubatura_hidden {
                let _ = toggle_kubatura_visibility(state, host);
            }
            host.enable_face_selection();
            state.active_dialog = Some(ActiveDialog::Edgebanding);
            update_ghost_preview(state, host);
        }
        RibbonTool::DecorGrain => {
            if state.panel_decors.is_empty() {
                load_decors_from_metadata(state, host);
            }
            state.ghost_box = None;
            state.ghost_cylinders = None;
            state.ghost_faces.clear();
            if !state.kubatura_hidden {
                let _ = toggle_kubatura_visibility(state, host);
            }
            host.enable_face_selection();
            state.active_dialog = Some(ActiveDialog::DecorGrain);
            update_ghost_preview(state, host);
        }
        RibbonTool::Connectors => {
            state.connector_edge_pick = None;
            state.connector_face_pick = None;
            state.connector_active_slot = 0;
            state.ghost_box = None;
            // Ukryj kubaturę by nie zasłaniała krawędzi i ścian formatek
            if !state.kubatura_hidden {
                let _ = toggle_kubatura_visibility(state, host);
            }
            host.enable_edge_selection();
            state.active_dialog = Some(ActiveDialog::Connectors);
        }
        RibbonTool::TDogbone => {
            state.tdogbone_edge_pick = None;
            state.tdogbone_face_pick = None;
            state.tdogbone_active_slot = 0;
            state.ghost_box = None;
            state.ghost_cylinders = None;
            if !state.kubatura_hidden {
                let _ = toggle_kubatura_visibility(state, host);
            }
            host.enable_edge_selection();
            state.active_dialog = Some(ActiveDialog::TDogbone);
        }
        RibbonTool::Hinges => {
            state.hinge_face_picks = [None, None];
            state.hinge_active_slot = 0;
            state.ghost_box = None;
            state.ghost_cylinders = None;
            if !state.kubatura_hidden {
                let _ = toggle_kubatura_visibility(state, host);
            }
            host.enable_face_selection();
            state.active_dialog = Some(ActiveDialog::Hinges);
        }
        RibbonTool::Drawer => {
            state.drawer_face_picks = [None, None, None, None];
            state.drawer_active_slot = 0;
            state.drawer_validation_info.clear();
            state.drawer_validation_error = false;
            state.ghost_box = None;
            state.ghost_cylinders = None;
            if !state.kubatura_hidden {
                let _ = toggle_kubatura_visibility(state, host);
            }
            host.enable_face_selection();
            state.active_dialog = Some(ActiveDialog::Drawer);
        }
        RibbonTool::Joints => {
            state.active_dialog = Some(ActiveDialog::Joints);
        }
        RibbonTool::CuttingList => {
            let _ = refresh_cutting_list(state, host);
            state.bom_open = !state.bom_open;
        }
        RibbonTool::Parameters => {
            host.open_parameters_dialog();
        }
    }
}

fn run_and_report(
    state: &mut WoodState,
    host: &mut dyn WoodHost,
    op: fn(&mut WoodState, &mut dyn WoodHost) -> Result<String, String>,
) -> bool {
    match op(state, host) {
        Ok(msg) => {
            state.status_message = msg.clone();
            state.status_error = false;
            host.set_status(&msg, false);
            true
        }
        Err(err) => {
            state.status_message = err.clone();
            state.status_error = true;
            host.set_status(&err, true);
            false
        }
    }
}

fn paint_tool_icon(painter: &egui::Painter, tool: RibbonTool, c: Pos2) {
    let (cx, cy) = (c.x, c.y);
    match tool {
        RibbonTool::Kubatura => {
            let top = vec![pos2(cx, cy - 13.0), pos2(cx + 13.0, cy - 6.0), pos2(cx, cy + 1.0), pos2(cx - 13.0, cy - 6.0)];
            let left = vec![pos2(cx - 13.0, cy - 6.0), pos2(cx, cy + 1.0), pos2(cx, cy + 14.0), pos2(cx - 13.0, cy + 7.0)];
            let right = vec![pos2(cx, cy + 1.0), pos2(cx + 13.0, cy - 6.0), pos2(cx + 13.0, cy + 7.0), pos2(cx, cy + 14.0)];
            let stroke = Stroke::new(1.0, Color32::from_rgb(108, 182, 255));
            painter.add(egui::Shape::convex_polygon(top, Color32::from_rgb(43, 91, 132), stroke));
            painter.add(egui::Shape::convex_polygon(left, Color32::from_rgb(35, 73, 107), stroke));
            painter.add(egui::Shape::convex_polygon(right, Color32::from_rgb(27, 56, 84), stroke));
        }
        RibbonTool::ToggleKubatura => {
            let col = Color32::from_rgb(108, 182, 255);
            painter.circle(pos2(cx, cy), 8.0, Color32::from_rgb(28, 44, 64), Stroke::new(1.5, col));
            painter.circle(pos2(cx, cy), 3.5, Color32::WHITE, Stroke::NONE);
        }
        RibbonTool::AutoPanel => {
            let p1 = vec![pos2(cx - 12.0, cy - 10.0), pos2(cx - 4.0, cy - 14.0), pos2(cx - 4.0, cy + 8.0), pos2(cx - 12.0, cy + 12.0)];
            let p2 = vec![pos2(cx - 4.0, cy - 14.0), pos2(cx + 1.0, cy - 12.0), pos2(cx + 1.0, cy + 10.0), pos2(cx - 4.0, cy + 8.0)];
            let border = Stroke::new(1.0, Color32::from_rgb(243, 198, 119));
            painter.add(egui::Shape::convex_polygon(p1, Color32::from_rgb(212, 163, 89), border));
            painter.add(egui::Shape::convex_polygon(p2, Color32::from_rgb(176, 130, 62), border));
            let blue = Color32::from_rgb(108, 182, 255);
            painter.line_segment([pos2(cx + 3.0, cy - 1.0), pos2(cx + 13.0, cy - 1.0)], Stroke::new(2.0, blue));
            painter.add(egui::Shape::convex_polygon(
                vec![pos2(cx + 13.0, cy - 1.0), pos2(cx + 8.0, cy - 5.0), pos2(cx + 8.0, cy + 3.0)],
                blue,
                Stroke::NONE,
            ));
        }
        RibbonTool::FreePanel => {
            let s_side = Stroke::new(1.0, Color32::from_rgb(184, 196, 208));
            painter.rect(Rect::from_min_max(pos2(cx - 14.0, cy - 12.0), pos2(cx - 9.0, cy + 12.0)), 1.0, Color32::from_rgb(140, 153, 168), s_side, StrokeKind::Inside);
            painter.rect(Rect::from_min_max(pos2(cx + 9.0, cy - 12.0), pos2(cx + 14.0, cy + 12.0)), 1.0, Color32::from_rgb(140, 153, 168), s_side, StrokeKind::Inside);
            painter.rect(
                Rect::from_min_max(pos2(cx - 9.0, cy - 3.0), pos2(cx + 9.0, cy + 3.0)),
                1.0,
                Color32::from_rgb(224, 159, 62),
                Stroke::new(1.0, Color32::from_rgb(255, 209, 102)),
                StrokeKind::Inside,
            );
        }
        RibbonTool::AdaptBodies => {
            // Ikona zagnieżdżania brył w drzewku komponentu
            let s_box = Stroke::new(1.0, Color32::from_rgb(124, 192, 255));
            painter.rect(
                Rect::from_min_max(pos2(cx - 13.0, cy - 11.0), pos2(cx + 13.0, cy + 11.0)),
                2.0,
                Color32::from_rgb(28, 44, 64),
                s_box,
                StrokeKind::Inside,
            );
            painter.rect(
                Rect::from_min_max(pos2(cx - 9.0, cy - 7.0), pos2(cx + 2.0, cy + 7.0)),
                1.0,
                Color32::from_rgb(224, 159, 62),
                Stroke::new(1.0, Color32::from_rgb(255, 209, 102)),
                StrokeKind::Inside,
            );
            painter.circle(
                pos2(cx + 7.0, cy),
                3.5,
                Color32::from_rgb(74, 222, 128),
                Stroke::new(1.0, Color32::WHITE),
            );
        }
        RibbonTool::Edgebanding => {
            // Ikona formatki z kolorowym oklejonym brzegiem przednim i górnym (1D1K / 2D2K)
            painter.rect(
                Rect::from_min_max(pos2(cx - 12.0, cy - 9.0), pos2(cx + 12.0, cy + 9.0)),
                1.5,
                Color32::from_rgb(42, 54, 70),
                Stroke::new(1.0, Color32::from_rgb(130, 145, 165)),
                StrokeKind::Inside,
            );
            let band_col = Color32::from_rgb(56, 189, 248);
            let band_gold = Color32::from_rgb(245, 158, 11);
            painter.line_segment(
                [pos2(cx - 12.0, cy - 9.0), pos2(cx + 12.0, cy - 9.0)],
                Stroke::new(2.8, band_col),
            );
            painter.line_segment(
                [pos2(cx - 12.0, cy + 9.0), pos2(cx + 12.0, cy + 9.0)],
                Stroke::new(2.8, band_col),
            );
            painter.line_segment(
                [pos2(cx - 12.0, cy - 9.0), pos2(cx - 12.0, cy + 9.0)],
                Stroke::new(2.8, band_gold),
            );
            painter.text(
                pos2(cx + 1.0, cy),
                Align2::CENTER_CENTER,
                "2D",
                FontId::proportional(9.5),
                Color32::WHITE,
            );
        }
        RibbonTool::DecorGrain => {
            // Ikona formatki w kolorze dekoru ze strzałką kierunku usłojenia
            painter.rect(
                Rect::from_min_max(pos2(cx - 12.0, cy - 9.0), pos2(cx + 12.0, cy + 9.0)),
                1.5,
                Color32::from_rgb(181, 134, 84),
                Stroke::new(1.1, Color32::from_rgb(243, 198, 119)),
                StrokeKind::Inside,
            );
            let arrow_col = Color32::from_rgb(24, 28, 36);
            let s_arr = Stroke::new(1.8, arrow_col);
            painter.line_segment([pos2(cx - 8.0, cy), pos2(cx + 8.0, cy)], s_arr);
            painter.line_segment([pos2(cx - 8.0, cy), pos2(cx - 4.5, cy - 3.5)], s_arr);
            painter.line_segment([pos2(cx - 8.0, cy), pos2(cx - 4.5, cy + 3.5)], s_arr);
            painter.line_segment([pos2(cx + 8.0, cy), pos2(cx + 4.5, cy - 3.5)], s_arr);
            painter.line_segment([pos2(cx + 8.0, cy), pos2(cx + 4.5, cy + 3.5)], s_arr);
        }
        RibbonTool::Connectors => {
            // Ikona kołka / wkrętu łączącego dwie formatki
            painter.rect(
                Rect::from_min_max(pos2(cx - 13.0, cy - 11.0), pos2(cx - 5.0, cy + 11.0)),
                1.0,
                Color32::from_rgb(140, 153, 168),
                Stroke::new(1.0, Color32::from_rgb(184, 196, 208)),
                StrokeKind::Inside,
            );
            painter.rect(
                Rect::from_min_max(pos2(cx - 5.0, cy - 3.0), pos2(cx + 13.0, cy + 5.0)),
                1.0,
                Color32::from_rgb(224, 159, 62),
                Stroke::new(1.0, Color32::from_rgb(255, 209, 102)),
                StrokeKind::Inside,
            );
            // Kołek / konfirmat przechodzący przez styk
            painter.rect(
                Rect::from_min_max(pos2(cx - 11.0, cy - 1.0), pos2(cx + 6.0, cy + 3.0)),
                1.0,
                Color32::from_rgb(74, 222, 128),
                Stroke::new(1.0, Color32::WHITE),
                StrokeKind::Inside,
            );
        }
        RibbonTool::TDogbone => {
            // Formatka z gniazdem (pionowa po lewej)
            painter.rect(
                Rect::from_min_max(pos2(cx - 13.0, cy - 11.0), pos2(cx - 4.0, cy + 11.0)),
                1.0,
                Color32::from_rgb(140, 153, 168),
                Stroke::new(1.0, Color32::from_rgb(184, 196, 208)),
                StrokeKind::Inside,
            );
            // Formatka z czopem (pozioma po prawej)
            painter.rect(
                Rect::from_min_max(pos2(cx - 4.0, cy - 5.0), pos2(cx + 13.0, cy + 5.0)),
                1.0,
                Color32::from_rgb(224, 159, 62),
                Stroke::new(1.0, Color32::from_rgb(255, 209, 102)),
                StrokeKind::Inside,
            );
            // Czop wchodzący na przelot w gniazdo + narożniki dogbone
            painter.rect(
                Rect::from_min_max(pos2(cx - 13.0, cy - 3.5), pos2(cx - 4.0, cy + 3.5)),
                1.0,
                Color32::from_rgb(74, 222, 128),
                Stroke::new(1.0, Color32::WHITE),
                StrokeKind::Inside,
            );
            painter.circle_filled(pos2(cx - 11.5, cy - 3.5), 1.8, Color32::from_rgb(56, 189, 248));
            painter.circle_filled(pos2(cx - 11.5, cy + 3.5), 1.8, Color32::from_rgb(56, 189, 248));
        }
        RibbonTool::Hinges => {
            // Drzwi / front (pomarańczowo-złoty)
            painter.rect(
                Rect::from_min_max(pos2(cx - 13.0, cy - 11.0), pos2(cx - 3.0, cy + 11.0)),
                1.0,
                Color32::from_rgb(224, 159, 62),
                Stroke::new(1.0, Color32::from_rgb(255, 209, 102)),
                StrokeKind::Inside,
            );
            // Puszka zawiasu (okrąg fi 8px w drzwiach)
            painter.circle(
                pos2(cx - 5.0, cy),
                4.0,
                Color32::from_rgb(245, 158, 11),
                Stroke::new(1.0, Color32::WHITE),
            );
            // Ramię zawiasu
            painter.rect(
                Rect::from_min_max(pos2(cx - 4.0, cy - 2.0), pos2(cx + 6.0, cy + 2.0)),
                1.0,
                Color32::from_rgb(180, 195, 215),
                Stroke::new(1.0, Color32::WHITE),
                StrokeKind::Inside,
            );
            // Prowadnik na boku szafki
            painter.rect(
                Rect::from_min_max(pos2(cx + 6.0, cy - 9.0), pos2(cx + 12.0, cy + 9.0)),
                1.0,
                Color32::from_rgb(92, 158, 173),
                Stroke::new(1.0, Color32::from_rgb(142, 202, 230)),
                StrokeKind::Inside,
            );
        }
        RibbonTool::Drawer => {
            // Skrzynia szuflady (dno + plecy w zielonym obrysie ducha)
            painter.rect(
                Rect::from_min_max(pos2(cx - 11.0, cy - 8.0), pos2(cx + 12.0, cy + 8.0)),
                1.5,
                Color32::from_rgba_unmultiplied(34, 197, 94, 70),
                Stroke::new(1.2, Color32::from_rgb(74, 222, 128)),
                StrokeKind::Inside,
            );
            // Front szuflady (pomarańczowo-złoty na dole ikony)
            painter.rect(
                Rect::from_min_max(pos2(cx - 13.0, cy + 5.0), pos2(cx + 13.0, cy + 11.0)),
                1.0,
                Color32::from_rgb(224, 159, 62),
                Stroke::new(1.0, Color32::from_rgb(255, 209, 102)),
                StrokeKind::Inside,
            );
            // Błękitne punkty nawierceń prowadnic i frontu
            let cyan = Color32::from_rgb(56, 189, 248);
            painter.circle_filled(pos2(cx - 9.0, cy - 3.0), 1.8, cyan);
            painter.circle_filled(pos2(cx - 9.0, cy + 2.0), 1.8, cyan);
            painter.circle_filled(pos2(cx + 9.0, cy - 3.0), 1.8, cyan);
            painter.circle_filled(pos2(cx + 9.0, cy + 2.0), 1.8, cyan);
        }
        RibbonTool::Joints => {
            painter.rect(
                Rect::from_min_max(pos2(cx - 13.0, cy - 11.0), pos2(cx - 7.0, cy + 9.0)),
                1.0,
                Color32::from_rgb(92, 158, 173),
                Stroke::new(1.0, Color32::from_rgb(142, 202, 230)),
                StrokeKind::Inside,
            );
            painter.rect(
                Rect::from_min_max(pos2(cx - 7.0, cy + 3.0), pos2(cx + 12.0, cy + 9.0)),
                1.0,
                Color32::from_rgb(224, 159, 62),
                Stroke::new(1.0, Color32::from_rgb(255, 209, 102)),
                StrokeKind::Inside,
            );
            painter.circle(pos2(cx - 7.0, cy + 3.0), 3.5, Color32::from_rgb(239, 71, 111), Stroke::new(1.0, Color32::WHITE));
        }
        RibbonTool::CuttingList => {
            let r = Rect::from_min_max(pos2(cx - 12.0, cy - 10.0), pos2(cx + 12.0, cy + 10.0));
            let s = Stroke::new(1.2, Color32::from_rgb(124, 192, 255));
            painter.rect(r, 1.5, Color32::from_rgb(30, 44, 62), s, StrokeKind::Inside);
            painter.line_segment([pos2(cx - 12.0, cy - 3.0), pos2(cx + 12.0, cy - 3.0)], s);
            painter.line_segment([pos2(cx - 12.0, cy + 3.0), pos2(cx + 12.0, cy + 3.0)], s);
            painter.line_segment([pos2(cx - 3.0, cy - 10.0), pos2(cx - 3.0, cy + 10.0)], s);
        }
        RibbonTool::Parameters => {
            painter.text(
                pos2(cx, cy),
                Align2::CENTER_CENTER,
                "f(x)",
                FontId::proportional(14.0),
                Color32::from_rgb(124, 192, 255),
            );
        }
    }
}

/// Returns the title and icon name for the currently active Wood feature dialog.
pub fn active_dialog_meta(dialog: ActiveDialog) -> (&'static str, &'static str) {
    match dialog {
        ActiveDialog::Kubatura => ("Kubatura (Gabaryt)", "box"),
        ActiveDialog::AutoPanel => ("Quick Formatka", "extrude"),
        ActiveDialog::FreePanel => ("FORMATKA", "plane"),
        ActiveDialog::AdaptBodies => ("Adaptuj Bryły", "component"),
        ActiveDialog::Edgebanding => ("Obrzeża (Oklejanie)", "appearance"),
        ActiveDialog::DecorGrain => ("Dekor i Usłojenie", "appearance"),
        ActiveDialog::Connectors => ("Złącza (Kołki / Wkręty)", "hole"),
        ActiveDialog::TDogbone => ("Połączenie T-Dogbone (CNC)", "combine"),
        ActiveDialog::Hinges => ("Zawiasy Puszkowe", "hole"),
        ActiveDialog::Drawer => ("DRAWER (Systemy Szuflad)", "box"),
        ActiveDialog::Joints => ("Złożenie i Jointy", "joint"),
    }
}

/// Renders the active Wood dialog's contents inside a SolidCraft `ui_kit::Window`.
/// Returns `true` if the dialog requested to close itself (e.g. Cancel clicked).
pub fn draw_dialog_body(ui: &mut Ui, state: &mut WoodState, host: &mut dyn WoodHost) -> bool {
    let Some(dialog) = state.active_dialog else {
        return false;
    };

    let mut close_requested = false;

    match dialog {
        ActiveDialog::Kubatura => {
            ui.add_space(2.0);
            egui::Grid::new("wood_kubatura_grid")
                .num_columns(2)
                .spacing([12.0, 7.0])
                .min_col_width(135.0)
                .striped(false)
                .show(ui, |ui| {
                    form_entry_row(ui, "Szerokość (X)", &mut state.szerokosc, "mm");
                    form_entry_row(ui, "Głębokość (Y)", &mut state.glebokosc, "mm");
                    form_entry_row(ui, "Wysokość (Z)", &mut state.wysokosc, "mm");
                    form_entry_row(ui, "Grubość płyty", &mut state.grubosc, "mm");
                    form_entry_row(ui, "Przezroczystość", &mut state.kubatura_opacity, "0.0 - 1.0");
                });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button("Aktualizuj wymiary").on_hover_text("Przelicza parametry mebla bez usuwania/tworzenia bryły").clicked() {
                    let _ = run_and_report(state, host, update_kubatura);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("OK").strong()).on_hover_text("Tworzy bryłę Kubatury i zamyka okno").clicked() {
                        if run_and_report(state, host, create_kubatura) {
                            close_requested = true;
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        close_requested = true;
                    }
                });
            });
        }

        ActiveDialog::AutoPanel => {
            if state.auto_name.is_empty()
                || state.auto_name == "Bok_L"
                || state.auto_name.starts_with("Formatka_")
            {
                state.auto_name = next_formatka_name_with_state(state, host);
            }
            if state.auto_division_count.trim().is_empty() {
                state.auto_division_count = "1".to_string();
            }

            let eye_lbl = if state.kubatura_hidden {
                "👁 Pokaż Kubaturę"
            } else {
                "👁 Ukryj Kubaturę"
            };
            let eye_col = if state.kubatura_hidden { COL_GOLD } else { COL_BLUE };
            if ui
                .add_sized(
                    [ui.available_width(), 24.0],
                    egui::Button::new(RichText::new(eye_lbl).color(eye_col)),
                )
                .clicked()
            {
                let _ = toggle_kubatura_visibility(state, host);
            }
            ui.add_space(4.0);

            let pick1_text = match &state.auto_face_pick {
                Some(p) => format!("{} [f{}]", p.body, p.index),
                None => "Select (1. ściana w 3D)".to_string(),
            };
            let has_pick1 = state.auto_face_pick.is_some();

            let pick2_text = match &state.auto_face_pick_2 {
                Some(p) => format!("{} [f{}]", p.body, p.index),
                None => "Select (opcja: 2. ściana)".to_string(),
            };
            let has_pick2 = state.auto_face_pick_2.is_some();

            let div_count_val = parse_f64_or(&state.auto_division_count, 1.0)
                .round()
                .clamp(1.0, 50.0) as u32;
            let direction_locked = has_pick2 && div_count_val >= 2;
            if direction_locked && state.auto_direction != AutoPanelDirection::Center {
                state.auto_direction = AutoPanelDirection::Center;
            }

            ui.add_space(2.0);
            let mut inputs_changed = false;
            egui::Grid::new("wood_auto_grid")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .min_col_width(82.0)
                .show(ui, |ui| {
                    // 1. Ściana bazowa (obowiązkowa)
                    let slot0_active = state.auto_active_slot == 0;
                    ui.add(
                        egui::Label::new(RichText::new("1. Ściana bazowa").color(COL_LABEL))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    ui.horizontal(|ui| {
                        let col = if slot0_active {
                            COL_BLUE
                        } else if has_pick1 {
                            COL_GREEN
                        } else {
                            COL_TEXT
                        };
                        let mut txt = RichText::new(&pick1_text).color(col);
                        if slot0_active {
                            txt = txt.strong();
                        }
                        if ui.button(txt).clicked() {
                            state.auto_active_slot = 0;
                            host.enable_face_selection();
                        }
                        if has_pick1 && ui.small_button("\u{00d7}").clicked() {
                            state.auto_face_pick = None;
                            state.auto_active_slot = 0;
                            state.ghost_box = None;
                            state.auto_clearances_info.clear();
                            state.auto_clearances_error = false;
                            host.enable_face_selection();
                        }
                    });
                    ui.end_row();

                    // 2. Druga ściana (opcjonalna — wstawia formatkę/półki w świetle między dwiema ścianami)
                    let slot1_active = state.auto_active_slot == 1 && has_pick1;
                    ui.add(
                        egui::Label::new(RichText::new("2. Druga ściana").color(COL_LABEL))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    ui.horizontal(|ui| {
                        let col = if slot1_active {
                            COL_BLUE
                        } else if has_pick2 {
                            COL_GREEN
                        } else {
                            COL_MUTED
                        };
                        let mut txt = RichText::new(&pick2_text).color(col);
                        if slot1_active {
                            txt = txt.strong();
                        }
                        if ui
                            .button(txt)
                            .on_hover_text("Opcjonalnie wskaż drugą równoległą ścianę, aby wstawić formatkę (lub półki) pomiędzy nimi")
                            .clicked()
                        {
                            state.auto_active_slot = 1;
                            host.enable_face_selection();
                        }
                        if has_pick2 && ui.small_button("\u{00d7}").clicked() {
                            state.auto_face_pick_2 = None;
                            state.auto_division_count = "1".to_string();
                            state.auto_direction = AutoPanelDirection::Inside;
                            state.auto_active_slot = 1;
                            inputs_changed = true;
                            host.enable_face_selection();
                        }
                    });
                    ui.end_row();

                    form_entry_row(ui, "Nazwa formatki", &mut state.auto_name, "");

                    // Liczba podziału (pojawia się tylko gdy wskazano dwie powierzchnie)
                    if has_pick2 {
                        ui.add(
                            egui::Label::new(RichText::new("Liczba podziału").color(COL_LABEL))
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                        ui.horizontal(|ui| {
                            if ui.button(" - ").clicked() {
                                let next = div_count_val.saturating_sub(1).max(1);
                                state.auto_division_count = next.to_string();
                                if next >= 2 {
                                    state.auto_direction = AutoPanelDirection::Center;
                                }
                                inputs_changed = true;
                            }
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut state.auto_division_count)
                                        .desired_width(36.0),
                                )
                                .changed()
                            {
                                let next = parse_f64_or(&state.auto_division_count, 1.0)
                                    .round()
                                    .clamp(1.0, 50.0) as u32;
                                if next >= 2 {
                                    state.auto_direction = AutoPanelDirection::Center;
                                }
                                inputs_changed = true;
                            }
                            if ui.button(" + ").clicked() {
                                let next = (div_count_val + 1).min(50);
                                state.auto_division_count = next.to_string();
                                if next >= 2 {
                                    state.auto_direction = AutoPanelDirection::Center;
                                }
                                inputs_changed = true;
                            }
                            ui.label(RichText::new("szt").small().color(COL_MUTED));
                        });
                        ui.end_row();
                    }

                    ui.add(
                        egui::Label::new(RichText::new("Kierunek").color(COL_LABEL))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    let dir_label = match state.auto_direction {
                        AutoPanelDirection::Inside => "Wewnątrz",
                        AutoPanelDirection::Outside => "Zewnątrz",
                        AutoPanelDirection::Center => "Środek",
                    };
                    ui.add_enabled_ui(!direction_locked, |ui| {
                        egui::ComboBox::from_id_salt("wood_auto_dir")
                            .width(125.0)
                            .selected_text(dir_label)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_value(
                                        &mut state.auto_direction,
                                        AutoPanelDirection::Inside,
                                        "Wewnątrz",
                                    )
                                    .changed()
                                    || ui
                                        .selectable_value(
                                            &mut state.auto_direction,
                                            AutoPanelDirection::Outside,
                                            "Zewnątrz",
                                        )
                                        .changed()
                                    || ui
                                        .selectable_value(
                                            &mut state.auto_direction,
                                            AutoPanelDirection::Center,
                                            "Środek",
                                        )
                                        .changed()
                                {
                                    state.auto_inward =
                                        state.auto_direction != AutoPanelDirection::Outside;
                                    inputs_changed = true;
                                }
                            });
                    });
                    ui.end_row();

                    if form_entry_row(ui, "Grubość płyty", &mut state.auto_thickness, "mm") {
                        inputs_changed = true;
                    }
                });

            if inputs_changed {
                update_ghost_preview(state, host);
            }

            // Mały tekst informujący o wymiarach pól światła po podziale (przed rozwijanym segmentem opcji)
            if has_pick2 && !state.auto_clearances_info.is_empty() {
                ui.add_space(4.0);
                let info_col = if state.auto_clearances_error {
                    COL_RED
                } else {
                    COL_GOLD
                };
                ui.label(RichText::new(&state.auto_clearances_info).small().color(info_col));
            }

            ui.add_space(4.0);
            egui::CollapsingHeader::new(RichText::new("OPCJE ODSUNIĘCIA / MARGINESY").small().strong().color(COL_LABEL))
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let link_lbl = if state.auto_margins_linked { "🔗 Marginesy: 1 wspólny" } else { "🔓 Marginesy: 4 strony" };
                        if ui.button(RichText::new(link_lbl).small().color(COL_BLUE)).clicked() {
                            state.auto_margins_linked = !state.auto_margins_linked;
                            if state.auto_margins_linked {
                                let v = state.auto_margins[0].clone();
                                state.auto_margins[1] = v.clone();
                                state.auto_margins[2] = v.clone();
                                state.auto_margins[3] = v;
                                inputs_changed = true;
                            }
                        }
                    });
                    ui.add_space(2.0);

                    egui::Grid::new("wood_auto_margins_grid")
                        .num_columns(2)
                        .spacing([8.0, 5.0])
                        .min_col_width(70.0)
                        .show(ui, |ui| {
                            let mut row_entry = |label: &str, val: &mut String, unit: &str| {
                                ui.add(egui::Label::new(RichText::new(label).color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                                ui.horizontal(|ui| {
                                    if ui.add(egui::TextEdit::singleline(val).desired_width(100.0)).changed() {
                                        inputs_changed = true;
                                    }
                                    if !unit.is_empty() {
                                        ui.label(RichText::new(unit).small().color(COL_MUTED));
                                    }
                                });
                                ui.end_row();
                            };

                            row_entry("Odsunięcie ściany", &mut state.auto_plane_offset, "mm");

                            if state.auto_margins_linked {
                                let mut common = state.auto_margins[0].clone();
                                ui.add(egui::Label::new(RichText::new("Margines (4 strony)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                                ui.horizontal(|ui| {
                                    if ui.add(egui::TextEdit::singleline(&mut common).desired_width(100.0)).changed() {
                                        state.auto_margins[0] = common.clone();
                                        state.auto_margins[1] = common.clone();
                                        state.auto_margins[2] = common.clone();
                                        state.auto_margins[3] = common;
                                        inputs_changed = true;
                                    }
                                    ui.label(RichText::new("mm").small().color(COL_MUTED));
                                });
                                ui.end_row();
                            } else {
                                row_entry("Margines Bok A", &mut state.auto_margins[0], "mm");
                                row_entry("Margines Bok B", &mut state.auto_margins[1], "mm");
                                row_entry("Margines Przód", &mut state.auto_margins[2], "mm");
                                row_entry("Margines Tył", &mut state.auto_margins[3], "mm");
                            }
                        });
                });

            if inputs_changed {
                update_ghost_preview(state, host);
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button("Wyczyść").on_hover_text("Czyści wybrane ściany").clicked() {
                    state.auto_face_pick = None;
                    state.auto_face_pick_2 = None;
                    state.auto_active_slot = 0;
                    state.auto_division_count = "1".to_string();
                    state.auto_direction = AutoPanelDirection::Inside;
                    state.auto_clearances_info.clear();
                    state.auto_clearances_error = false;
                    state.ghost_box = None;
                    host.enable_face_selection();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("Zatwierdź").color(COL_GREEN).strong())
                        .on_hover_text("Tworzy formatkę (lub zestaw półek/przegród) i pozostawia okno otwarte na kolejną")
                        .clicked()
                    {
                        let _ = run_and_report(state, host, create_auto_panel);
                    }
                    if ui.button("Zamknij").clicked() {
                        close_requested = true;
                    }
                });
            });
        }

        ActiveDialog::FreePanel => {
            if state.free_name.is_empty()
                || state.free_name == "Wieniec_D"
                || state.free_name.starts_with("Formatka_")
            {
                state.free_name = next_formatka_name_with_state(state, host);
            }
            let mut pending_pick_slot: Option<usize> = None;
            let mut clear_pick_slot: Option<usize> = None;
            let mut request_face_sel = false;

            let is_two_faces = state.free_mode == FreePanelMode::TwoFaces;
            let txt0 = format_picks(&state.free_picks[0], 2, "Select (2 boki w 3D)");
            let need1 = if is_two_faces { 2 } else { 1 };
            let txt1 = format_picks(
                &state.free_picks[1],
                need1,
                if is_two_faces {
                    "Select (2 ściany w 3D)"
                } else {
                    "Select (1 ściana w 3D)"
                },
            );
            let txt2 = format_picks(&state.free_picks[2], 1, "Select (1 ściana w 3D)");

            // Przycisk Ukryj/Pokaż Kubaturę rozciągnięty na całą szerokość o stałym rozmiarze
            let eye_lbl = if state.kubatura_hidden { "👁 Pokaż Kubaturę" } else { "👁 Ukryj Kubaturę" };
            let eye_col = if state.kubatura_hidden { COL_GOLD } else { COL_BLUE };
            if ui.add_sized([ui.available_width(), 24.0], egui::Button::new(RichText::new(eye_lbl).color(eye_col))).clicked() {
                let _ = toggle_kubatura_visibility(state, host);
            }
            ui.add_space(4.0);

            let mut inputs_changed = false;
            egui::Grid::new("wood_free_grid")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .min_col_width(75.0)
                .show(ui, |ui| {
                    form_entry_row(ui, "Nazwa formatki", &mut state.free_name, "");

                    let slot0_active = state.active_pick_slot == 0;
                    let slot0_has = !state.free_picks[0].is_empty();
                    let (p0, c0) = select_slot_row(
                        ui,
                        "1. Dwa boki",
                        &txt0,
                        slot0_active,
                        slot0_has,
                    );
                    if p0 {
                        pending_pick_slot = Some(0);
                        request_face_sel = true;
                    }
                    if c0 {
                        clear_pick_slot = Some(0);
                        request_face_sel = true;
                    }

                    ui.add(egui::Label::new(RichText::new("Tryb Kroku 2").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    egui::ComboBox::from_id_salt("wood_free_mode")
                        .width(125.0)
                        .selected_text(match state.free_mode {
                            FreePanelMode::TwoFaces => "Dwa boki (Przód i Tył)",
                            FreePanelMode::OneFaceAndWidth => "1 bok + wymiar (Trawers)",
                        })
                        .show_ui(ui, |ui| {
                            if ui.selectable_value(&mut state.free_mode, FreePanelMode::TwoFaces, "Dwa boki (Przód i Tył)").changed() {
                                state.free_picks[1].clear();
                                inputs_changed = true;
                            }
                            if ui.selectable_value(
                                &mut state.free_mode,
                                FreePanelMode::OneFaceAndWidth,
                                "1 bok + wymiar (Trawers)",
                            ).changed() {
                                state.free_picks[1].clear();
                                inputs_changed = true;
                            }
                        });
                    ui.end_row();

                    let slot1_active = state.active_pick_slot == 1;
                    let slot1_has = !state.free_picks[1].is_empty();
                    let (p1, c1) = select_slot_row(
                        ui,
                        "2. Przód / Tył",
                        &txt1,
                        slot1_active,
                        slot1_has,
                    );
                    if p1 {
                        if !is_two_faces {
                            state.free_picks[1].clear();
                        }
                        pending_pick_slot = Some(1);
                        request_face_sel = true;
                    }
                    if c1 {
                        clear_pick_slot = Some(1);
                        request_face_sel = true;
                    }

                    if !is_two_faces {
                        if form_entry_row(ui, "Szerokość od boku", &mut state.free_width, "mm") {
                            inputs_changed = true;
                        }
                    }

                    let slot2_active = state.active_pick_slot == 2;
                    let slot2_has = !state.free_picks[2].is_empty();
                    let (p2, c2) = select_slot_row(
                        ui,
                        "3. Przyklejenie",
                        &txt2,
                        slot2_active,
                        slot2_has,
                    );
                    if p2 {
                        pending_pick_slot = Some(2);
                        request_face_sel = true;
                    }
                    if c2 {
                        clear_pick_slot = Some(2);
                        request_face_sel = true;
                    }

                    ui.add(egui::Label::new(RichText::new("Kierunek").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    egui::ComboBox::from_id_salt("wood_free_dir")
                        .width(125.0)
                        .selected_text(if state.free_inward {
                            "Do wnętrza kubatury"
                        } else {
                            "Na zewnątrz"
                        })
                        .show_ui(ui, |ui| {
                            if ui.selectable_value(&mut state.free_inward, true, "Do wnętrza kubatury").changed()
                                || ui.selectable_value(&mut state.free_inward, false, "Na zewnątrz").changed()
                            {
                                inputs_changed = true;
                            }
                        });
                    ui.end_row();

                    if form_entry_row(ui, "Grubość płyty", &mut state.free_thickness, "mm") {
                        inputs_changed = true;
                    }
                });

            ui.add_space(4.0);
            egui::CollapsingHeader::new(RichText::new("OPCJE ODSUNIĘCIA / MARGINESY").small().strong().color(COL_LABEL))
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let link_lbl = if state.free_margins_linked { "🔗 Marginesy: 1 wspólny" } else { "🔓 Marginesy: 4 strony" };
                        if ui.button(RichText::new(link_lbl).small().color(COL_BLUE)).clicked() {
                            state.free_margins_linked = !state.free_margins_linked;
                            if state.free_margins_linked {
                                let v = state.free_margins[0].clone();
                                state.free_margins[1] = v.clone();
                                state.free_margins[2] = v.clone();
                                state.free_margins[3] = v;
                                inputs_changed = true;
                            }
                        }
                    });
                    ui.add_space(2.0);

                    egui::Grid::new("wood_free_margins_grid")
                        .num_columns(2)
                        .spacing([8.0, 5.0])
                        .min_col_width(70.0)
                        .show(ui, |ui| {
                            let mut row_entry = |label: &str, val: &mut String, unit: &str| {
                                ui.add(egui::Label::new(RichText::new(label).color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                                ui.horizontal(|ui| {
                                    if ui.add(egui::TextEdit::singleline(val).desired_width(100.0)).changed() {
                                        inputs_changed = true;
                                    }
                                    if !unit.is_empty() {
                                        ui.label(RichText::new(unit).small().color(COL_MUTED));
                                    }
                                });
                                ui.end_row();
                            };
                            row_entry("Odsunięcie (Krok 3)", &mut state.free_plane_offset, "mm");

                            if state.free_margins_linked {
                                let mut common = state.free_margins[0].clone();
                                ui.add(egui::Label::new(RichText::new("Margines (4 strony)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                                ui.horizontal(|ui| {
                                    if ui.add(egui::TextEdit::singleline(&mut common).desired_width(100.0)).changed() {
                                        state.free_margins[0] = common.clone();
                                        state.free_margins[1] = common.clone();
                                        state.free_margins[2] = common.clone();
                                        state.free_margins[3] = common;
                                        inputs_changed = true;
                                    }
                                    ui.label(RichText::new("mm").small().color(COL_MUTED));
                                });
                                ui.end_row();
                            } else {
                                row_entry("Margines Bok A (Krok 1)", &mut state.free_margins[0], "mm");
                                row_entry("Margines Bok B (Krok 1)", &mut state.free_margins[1], "mm");
                                row_entry("Margines Przód (Krok 2)", &mut state.free_margins[2], "mm");
                                row_entry("Margines Tył (Krok 2)", &mut state.free_margins[3], "mm");
                            }
                        });
                });
            if inputs_changed {
                update_ghost_preview(state, host);
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button("Wyczyść").on_hover_text("Czyści wybrane ściany i zaczyna od Kroku 1").clicked() {
                    state.ghost_box = None;
                    state.free_picks = [Vec::new(), Vec::new(), Vec::new()];
                    state.active_pick_slot = 0;
                    host.enable_face_selection();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("Zatwierdź").color(COL_GREEN).strong())
                        .on_hover_text("Tworzy formatkę i pozostawia okno otwarte na kolejną")
                        .clicked()
                    {
                        let _ = run_and_report(state, host, create_free_panel);
                    }
                    if ui.button("Zamknij").clicked() {
                        close_requested = true;
                    }
                });
            });

            if let Some(slot) = pending_pick_slot {
                state.active_pick_slot = slot;
            }
            if let Some(slot) = clear_pick_slot {
                if let Some(v) = state.free_picks.get_mut(slot) {
                    v.clear();
                }
                state.active_pick_slot = slot;
                update_ghost_preview(state, host);
            }
            if request_face_sel {
                host.enable_face_selection();
            }
        }

        ActiveDialog::Connectors => {
            const CTRL_H: f32 = 24.0;

            // Przycisk przezroczystości rozciągnięty na całą szerokość okna (Item 8)
            let trans_lbl = if state.panels_translucent {
                "👁 Przezroczystość formatek (WŁ: 0.50)"
            } else {
                "👁 Przezroczystość formatek (WYŁ: 1.0)"
            };
            let trans_col = if state.panels_translucent { COL_GOLD } else { COL_BLUE };
            if ui.add_sized(
                [ui.available_width(), CTRL_H],
                egui::Button::new(RichText::new(trans_lbl).color(trans_col)),
            )
            .on_hover_text("Włącza delikatną przezroczystość (0.50) wszystkich formatek mebla, aby widzieć wewnętrzne krawędzie i otwory")
            .clicked()
            {
                let _ = toggle_panels_translucency(state, host);
            }
            ui.add_space(6.0);

            let lib = match ConnectorLibrary::load() {
                Ok(l) => l,
                Err(err) => {
                    ui.colored_label(COL_RED, &err);
                    return false;
                }
            };
            let current_cat_name = lib
                .categories
                .iter()
                .find(|c| c.id == state.connector_category_id)
                .map(|c| c.label.clone())
                .unwrap_or_else(|| state.connector_category_id.clone());

            let current_preset = lib
                .find_preset(&state.connector_preset_id)
                .map(|(_, p)| p.clone());
            let current_preset_name = current_preset
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| state.connector_preset_id.clone());

            let edge_text = match &state.connector_edge_pick {
                Some(e) => format!("{} [e{}]", e.body, e.index),
                None => "Select (1. krawędź)".to_string(),
            };
            let has_edge = state.connector_edge_pick.is_some();

            let face_text = match &state.connector_face_pick {
                Some(f) => format!("{} [f{}]", f.body, f.index),
                None => "Select (2. ściana)".to_string(),
            };
            let has_face = state.connector_face_pick.is_some();

            // Zwiększona odległość między etykietą a polem: spacing([14.0, 6.0]) (Item 2)
            egui::Grid::new("wood_connectors_grid")
                .num_columns(2)
                .spacing([14.0, 6.0])
                .min_col_width(82.0)
                .show(ui, |ui| {
                    // 1. Grupa złącz
                    ui.add(egui::Label::new(RichText::new("Grupa").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    egui::ComboBox::from_id_salt("wood_conn_cat")
                        .selected_text(&current_cat_name)
                        .width(136.0)
                        .show_ui(ui, |ui| {
                            for cat in &lib.categories {
                                if ui
                                    .selectable_value(
                                        &mut state.connector_category_id,
                                        cat.id.clone(),
                                        &cat.label,
                                    )
                                    .changed()
                                {
                                    state.connector_through_face = cat.through_face_default;
                                    if let Some(first) = cat.items.first() {
                                        state.connector_preset_id = first.id.clone();
                                    }
                                    update_ghost_preview(state, host);
                                }
                            }
                        });
                    ui.end_row();

                    // 2. Okucie
                    ui.add(egui::Label::new(RichText::new("Okucie").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    egui::ComboBox::from_id_salt("wood_conn_preset")
                        .selected_text(&current_preset_name)
                        .width(136.0)
                        .show_ui(ui, |ui| {
                            if let Some(cat) = lib
                                .categories
                                .iter()
                                .find(|c| c.id == state.connector_category_id)
                            {
                                for p in &cat.items {
                                    if ui.selectable_value(
                                        &mut state.connector_preset_id,
                                        p.id.clone(),
                                        &p.name,
                                    ).changed() {
                                        update_ghost_preview(state, host);
                                    }
                                }
                            }
                        });
                    ui.end_row();

                    // Krok 1: Krawędź (Item 1: wysokość 24.0)
                    let slot0_active = state.connector_active_slot == 0;
                    ui.add(egui::Label::new(RichText::new("1. Krawędź").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        let color = if slot0_active { COL_BLUE } else if has_edge { COL_GREEN } else { COL_TEXT };
                        let mut text = RichText::new(&edge_text).color(color);
                        if slot0_active { text = text.strong(); }
                        let btn_w = if has_edge { 110.0 } else { 136.0 };
                        if ui.add_sized([btn_w, CTRL_H], egui::Button::new(text)).clicked() {
                            state.connector_active_slot = 0;
                            host.enable_edge_selection();
                        }
                        if has_edge && ui.add_sized([22.0, CTRL_H], egui::Button::new("\u{00d7}")).clicked() {
                            state.connector_edge_pick = None;
                            state.connector_active_slot = 0;
                            host.enable_edge_selection();
                            update_ghost_preview(state, host);
                        }
                    });
                    ui.end_row();

                    // Krok 2: Ściana (Item 1: wysokość 24.0)
                    let slot1_active = state.connector_active_slot == 1;
                    ui.add(egui::Label::new(RichText::new("2. Ściana").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        let color = if slot1_active { COL_BLUE } else if has_face { COL_GREEN } else { COL_TEXT };
                        let mut text = RichText::new(&face_text).color(color);
                        if slot1_active { text = text.strong(); }
                        let btn_w = if has_face { 110.0 } else { 136.0 };
                        if ui.add_sized([btn_w, CTRL_H], egui::Button::new(text)).clicked() {
                            state.connector_active_slot = 1;
                            host.enable_face_selection();
                        }
                        if has_face && ui.add_sized([22.0, CTRL_H], egui::Button::new("\u{00d7}")).clicked() {
                            state.connector_face_pick = None;
                            state.connector_active_slot = 1;
                            host.enable_face_selection();
                            update_ghost_preview(state, host);
                        }
                    });
                    ui.end_row();

                    // 4. Od brzegu (X) (Item 1: wysokość 24.0)
                    ui.add(egui::Label::new(RichText::new("Od brzegu (X)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        let x_resp = ui.add_sized([80.0, CTRL_H], egui::TextEdit::singleline(&mut state.connector_offset_x));
                        if x_resp.changed() {
                            update_ghost_preview(state, host);
                        }
                        ui.label(RichText::new("mm").small().color(COL_MUTED));
                    });
                    ui.end_row();

                    // Offset wzajemny (+/-) (Item 1: wysokość 24.0)
                    ui.add(egui::Label::new(RichText::new("Offset (+/-)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        let shift_resp = ui.add_sized([55.0, CTRL_H], egui::TextEdit::singleline(&mut state.connector_offset_shift));
                        if shift_resp.changed() {
                            update_ghost_preview(state, host);
                        }
                        ui.label(RichText::new("mm").small().color(COL_MUTED));
                        let sign_str = if state.connector_shift_sign > 0.0 { "+" } else { "-" };
                        let sign_col = if state.connector_shift_sign > 0.0 { COL_GREEN } else { COL_GOLD };
                        if ui.add_sized([26.0, CTRL_H], egui::Button::new(RichText::new(sign_str).strong().color(sign_col)))
                            .on_hover_text("Odwróć zwrot przesunięcia (+/-)")
                            .clicked()
                        {
                            state.connector_shift_sign = -state.connector_shift_sign;
                            update_ghost_preview(state, host);
                        }
                    });
                    ui.end_row();

                    // Ilość (N) ze stepperami [-] i [+] (Item 1: wysokość 24.0)
                    ui.add(egui::Label::new(RichText::new("Ilość (N)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        if ui.add_sized([26.0, CTRL_H], egui::Button::new(" - ")).clicked() {
                            let mut cnt = parse_f64_or(&state.connector_count, 2.0).round() as i64;
                            cnt = (cnt - 1).max(1);
                            state.connector_count = cnt.to_string();
                            update_ghost_preview(state, host);
                        }
                        let count_resp = ui.add_sized([36.0, CTRL_H], egui::TextEdit::singleline(&mut state.connector_count));
                        if count_resp.changed() {
                            update_ghost_preview(state, host);
                        }
                        if ui.add_sized([26.0, CTRL_H], egui::Button::new(" + ")).clicked() {
                            let mut cnt = parse_f64_or(&state.connector_count, 2.0).round() as i64;
                            cnt = (cnt + 1).min(100);
                            state.connector_count = cnt.to_string();
                            update_ghost_preview(state, host);
                        }
                        ui.label(RichText::new("szt").small().color(COL_MUTED));
                    });
                    ui.end_row();
                });

            // Wymiary nawierceń w dedykowanej ramce rozciągniętej na całą szerokość okna (Item 4)
            if let Some(p) = &current_preset {
                ui.add_space(6.0);
                let avail_w = ui.available_width();
                egui::Frame::NONE
                    .fill(Color32::from_rgb(26, 31, 39))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(45, 53, 66)))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_min_width(avail_w.max(220.0) - 16.0);
                        ui.set_width(avail_w.max(220.0) - 16.0);
                        ui.label(RichText::new("Wymiary nawierceń:").strong().color(COL_TEXT).size(11.5));
                        ui.add_space(2.0);
                        let face_desc = if state.connector_through_face {
                            format!("fi {:.1} mm (przelot)", p.face_hole.diameter)
                        } else {
                            format!("fi {:.1}×{:.0} mm (ślepy)", p.face_hole.diameter, p.face_hole.depth)
                        };
                        ui.label(
                            RichText::new(format!("Brzeg: fi {:.1}×{:.0} mm  |  Ściana: {}", p.edge_hole.diameter, p.edge_hole.depth, face_desc))
                                .size(11.0)
                                .color(Color32::from_rgb(180, 195, 215)),
                        );
                    });
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            // Rząd 1 narzędziowy: Wyczyść i Poprzednie (obok siebie, jednakowa wysokość) (Item 5 & 7)
            ui.horizontal(|ui| {
                if ui.add_sized([110.0, CTRL_H], egui::Button::new("Wyczyść"))
                    .on_hover_text("Czyści wybraną krawędź oraz ścianę i wraca do Kroku 1")
                    .clicked()
                {
                    state.connector_edge_pick = None;
                    state.connector_face_pick = None;
                    state.connector_active_slot = 0;
                    state.ghost_cylinders = None;
                    host.enable_edge_selection();
                }

                let has_prev = state.connector_last_edge_pick.is_some();
                let prev_btn = egui::Button::new(
                    RichText::new("↺ Poprzednie")
                        .color(if has_prev { COL_BLUE } else { Color32::from_rgb(110, 115, 125) }),
                );
                if ui.add_enabled(has_prev, prev_btn.min_size(vec2(130.0, CTRL_H)))
                    .on_hover_text("Wczytuje ostatnio użytą krawędź i ścianę (np. do dodania wkrętów po kołkach)")
                    .clicked()
                {
                    state.connector_edge_pick = state.connector_last_edge_pick.clone();
                    state.connector_face_pick = state.connector_last_face_pick.clone();
                    update_ghost_preview(state, host);
                }
            });

            ui.add_space(4.0);

            // Rząd 2 zatwierdzający: Zamknij i Zatwierdź (jednakowa wysokość) (Item 5)
            ui.horizontal(|ui| {
                if ui.add_sized([110.0, CTRL_H], egui::Button::new("Zamknij")).clicked() {
                    close_requested = true;
                }
                if ui.add_sized([130.0, CTRL_H], egui::Button::new(RichText::new("Zatwierdź").strong().color(COL_GREEN)))
                    .on_hover_text("Wierci otwory dla wybranej krawędzi i ściany, czyści zaznaczenie i pozwala wskazać kolejną parę")
                    .clicked()
                {
                    let _ = run_and_report(state, host, create_connectors);
                }
            });
        }

        ActiveDialog::TDogbone => {
            const CTRL_H: f32 = 24.0;

            let trans_lbl = if state.panels_translucent {
                "👁 Przezroczystość formatek (WŁ: 0.50)"
            } else {
                "👁 Przezroczystość formatek (WYŁ: 1.0)"
            };
            let trans_col = if state.panels_translucent { COL_GOLD } else { COL_BLUE };
            if ui.add_sized(
                [ui.available_width(), CTRL_H],
                egui::Button::new(RichText::new(trans_lbl).color(trans_col)),
            )
            .on_hover_text("Włącza delikatną przezroczystość (0.50) wszystkich formatek mebla, aby widzieć czopy i gniazda")
            .clicked()
            {
                let _ = toggle_panels_translucency(state, host);
            }
            ui.add_space(6.0);

            let lib = match CncJointLibrary::load() {
                Ok(l) => l,
                Err(err) => {
                    ui.colored_label(COL_RED, &err);
                    return false;
                }
            };
            let current_preset = lib.find_preset(&state.tdogbone_preset_id).cloned();
            let current_preset_name = current_preset
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| state.tdogbone_preset_id.clone());

            let edge_text = match &state.tdogbone_edge_pick {
                Some(e) => format!("{} [e{}]", e.body, e.index),
                None => "Select (1. krawędź czopa)".to_string(),
            };
            let has_edge = state.tdogbone_edge_pick.is_some();

            let face_text = match &state.tdogbone_face_pick {
                Some(f) => format!("{} [f{}]", f.body, f.index),
                None => "Select (2. ściana gniazda)".to_string(),
            };
            let has_face = state.tdogbone_face_pick.is_some();

            egui::Grid::new("wood_tdogbone_grid")
                .num_columns(2)
                .spacing([14.0, 6.0])
                .min_col_width(82.0)
                .show(ui, |ui| {
                    // 1. Preset z cnc_joints.json
                    ui.add(egui::Label::new(RichText::new("Preset CNC").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    egui::ComboBox::from_id_salt("wood_tdogbone_preset")
                        .selected_text(&current_preset_name)
                        .width(136.0)
                        .show_ui(ui, |ui| {
                            for p in &lib.presets {
                                if ui
                                    .selectable_value(
                                        &mut state.tdogbone_preset_id,
                                        p.id.clone(),
                                        &p.name,
                                    )
                                    .changed()
                                {
                                    state.tdogbone_width = format!("{:.0}", p.joint_width_mm);
                                    state.tdogbone_offset_x = format!("{:.0}", p.edge_offset_mm);
                                    state.tdogbone_count = p.default_count.to_string();
                                    update_ghost_preview(state, host);
                                }
                            }
                        });
                    ui.end_row();

                    // Krok 1: Krawędź formatki z czopem
                    let slot0_active = state.tdogbone_active_slot == 0;
                    ui.add(egui::Label::new(RichText::new("1. Krawędź (Czop)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        let color = if slot0_active { COL_BLUE } else if has_edge { COL_GREEN } else { COL_TEXT };
                        let mut text = RichText::new(&edge_text).color(color);
                        if slot0_active { text = text.strong(); }
                        let btn_w = if has_edge { 110.0 } else { 136.0 };
                        if ui.add_sized([btn_w, CTRL_H], egui::Button::new(text)).clicked() {
                            state.tdogbone_active_slot = 0;
                            host.enable_edge_selection();
                        }
                        if has_edge && ui.add_sized([22.0, CTRL_H], egui::Button::new("\u{00d7}")).clicked() {
                            state.tdogbone_edge_pick = None;
                            state.tdogbone_active_slot = 0;
                            host.enable_edge_selection();
                            update_ghost_preview(state, host);
                        }
                    });
                    ui.end_row();

                    // Krok 2: Ściana formatki z gniazdem
                    let slot1_active = state.tdogbone_active_slot == 1;
                    ui.add(egui::Label::new(RichText::new("2. Ściana (Gniazdo)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        let color = if slot1_active { COL_BLUE } else if has_face { COL_GREEN } else { COL_TEXT };
                        let mut text = RichText::new(&face_text).color(color);
                        if slot1_active { text = text.strong(); }
                        let btn_w = if has_face { 110.0 } else { 136.0 };
                        if ui.add_sized([btn_w, CTRL_H], egui::Button::new(text)).clicked() {
                            state.tdogbone_active_slot = 1;
                            host.enable_face_selection();
                        }
                        if has_face && ui.add_sized([22.0, CTRL_H], egui::Button::new("\u{00d7}")).clicked() {
                            state.tdogbone_face_pick = None;
                            state.tdogbone_active_slot = 1;
                            host.enable_face_selection();
                            update_ghost_preview(state, host);
                        }
                    });
                    ui.end_row();

                    // Szerokość połączenia (L)
                    ui.add(egui::Label::new(RichText::new("Szerokość (L)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        let w_resp = ui.add_sized([80.0, CTRL_H], egui::TextEdit::singleline(&mut state.tdogbone_width));
                        if w_resp.changed() {
                            update_ghost_preview(state, host);
                        }
                        ui.label(RichText::new("mm").small().color(COL_MUTED));
                    });
                    ui.end_row();

                    // Odsunięcie od boków formatki z czopem (X)
                    ui.add(egui::Label::new(RichText::new("Od boków (X)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        let x_resp = ui.add_sized([80.0, CTRL_H], egui::TextEdit::singleline(&mut state.tdogbone_offset_x));
                        if x_resp.changed() {
                            update_ghost_preview(state, host);
                        }
                        ui.label(RichText::new("mm").small().color(COL_MUTED));
                    });
                    ui.end_row();

                    // Ilość połączeń (N)
                    ui.add(egui::Label::new(RichText::new("Ilość (N)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        if ui.add_sized([26.0, CTRL_H], egui::Button::new(" - ")).clicked() {
                            let mut cnt = parse_f64_or(&state.tdogbone_count, 1.0).round() as i64;
                            cnt = (cnt - 1).max(1);
                            state.tdogbone_count = cnt.to_string();
                            update_ghost_preview(state, host);
                        }
                        let count_resp = ui.add_sized([36.0, CTRL_H], egui::TextEdit::singleline(&mut state.tdogbone_count));
                        if count_resp.changed() {
                            update_ghost_preview(state, host);
                        }
                        if ui.add_sized([26.0, CTRL_H], egui::Button::new(" + ")).clicked() {
                            let mut cnt = parse_f64_or(&state.tdogbone_count, 1.0).round() as i64;
                            cnt = (cnt + 1).min(20);
                            state.tdogbone_count = cnt.to_string();
                            update_ghost_preview(state, host);
                        }
                        ui.label(RichText::new("szt").small().color(COL_MUTED));
                    });
                    ui.end_row();
                });

            if let Some(p) = &current_preset {
                ui.add_space(6.0);
                let avail_w = ui.available_width();
                egui::Frame::NONE
                    .fill(Color32::from_rgb(26, 31, 39))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(45, 53, 66)))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_min_width(avail_w.max(220.0) - 16.0);
                        ui.set_width(avail_w.max(220.0) - 16.0);
                        ui.label(RichText::new("Parametry geometrii CNC:").strong().color(COL_TEXT).size(11.5));
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(format!(
                                "Frez: fi {:.0} mm | Podcięcie HSM: {:.0}×{:.0} mm (R{:.1})",
                                p.contour_tool_dia_mm,
                                p.relief_width_mm,
                                p.relief_depth_mm,
                                p.relief_radius_mm
                            ))
                            .size(10.8)
                            .color(Color32::from_rgb(180, 195, 215)),
                        );
                        ui.label(
                            RichText::new(format!(
                                "Dogbone gniazda: 4× fi {:.1} mm (odsunięcie {:.1} mm / {:.0}°)",
                                p.dogbone_drill_dia_mm,
                                p.dogbone_corner_offset_mm,
                                p.dogbone_angle_deg
                            ))
                            .size(10.8)
                            .color(COL_GOLD),
                        );
                        if let Some(edge) = &state.tdogbone_edge_pick {
                            let boxes = extract_panel_boxes(host);
                            let joint_w = parse_f64_or(&state.tdogbone_width, p.joint_width_mm).max(10.0);
                            let offset_x = parse_f64_or(&state.tdogbone_offset_x, p.edge_offset_mm).max(0.0);
                            let count = parse_f64_or(&state.tdogbone_count, p.default_count as f64)
                                .round()
                                .clamp(1.0, 20.0) as u32;
                            match compute_tdogbone_plan(
                                edge,
                                state.tdogbone_face_pick.as_ref(),
                                &boxes,
                                joint_w,
                                offset_x,
                                count,
                            ) {
                                Ok(plan) => {
                                    ui.add_space(2.0);
                                    ui.label(
                                        RichText::new(format!(
                                            "Gotowe: {}× czop {:.0}×{:.0} mm ({} → {})",
                                            plan.joint_centers.len(),
                                            plan.joint_width,
                                            plan.face_thickness,
                                            plan.edge_panel,
                                            plan.face_panel
                                        ))
                                        .size(10.8)
                                        .color(COL_GREEN),
                                    );
                                }
                                Err(err) => {
                                    ui.add_space(2.0);
                                    ui.label(RichText::new(err).size(10.5).color(COL_RED));
                                }
                            }
                        }
                    });
            }

            if !state.status_message.is_empty() && !state.status_error {
                ui.add_space(4.0);
                ui.label(RichText::new(&state.status_message).small().color(COL_GREEN));
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                if ui.add_sized([110.0, CTRL_H], egui::Button::new("Wyczyść"))
                    .on_hover_text("Czyści wybraną krawędź oraz ścianę i wraca do Kroku 1")
                    .clicked()
                {
                    state.tdogbone_edge_pick = None;
                    state.tdogbone_face_pick = None;
                    state.tdogbone_active_slot = 0;
                    state.ghost_box = None;
                    host.enable_edge_selection();
                }

                let has_prev = state.tdogbone_last_edge_pick.is_some();
                let prev_btn = egui::Button::new(
                    RichText::new("↺ Poprzednie")
                        .color(if has_prev { COL_BLUE } else { Color32::from_rgb(110, 115, 125) }),
                );
                if ui.add_enabled(has_prev, prev_btn.min_size(vec2(130.0, CTRL_H)))
                    .on_hover_text("Wczytuje ostatnio użytą krawędź i ścianę")
                    .clicked()
                {
                    state.tdogbone_edge_pick = state.tdogbone_last_edge_pick.clone();
                    state.tdogbone_face_pick = state.tdogbone_last_face_pick.clone();
                    update_ghost_preview(state, host);
                }
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.add_sized([110.0, CTRL_H], egui::Button::new("Zamknij")).clicked() {
                    close_requested = true;
                }
                if ui.add_sized([130.0, CTRL_H], egui::Button::new(RichText::new("Zatwierdź").strong().color(COL_GREEN)))
                    .on_hover_text("Tworzy czop z podcięciami HSM na 1. formatce (ukrywając 2.), a następnie wycina gniazdo z 4 otworami dogbone na 2. formatce (ukrywając 1.)")
                    .clicked()
                {
                    let _ = run_and_report(state, host, create_tdogbone);
                }
            });
        }

        ActiveDialog::Hinges => {
            const CTRL_H: f32 = 24.0;

            // Przycisk przezroczystości rozciągnięty na całą szerokość okna
            let trans_lbl = if state.panels_translucent {
                "👁 Przezroczystość formatek (WŁ: 0.50)"
            } else {
                "👁 Przezroczystość formatek (WYŁ: 1.0)"
            };
            let trans_col = if state.panels_translucent { COL_GOLD } else { COL_BLUE };
            if ui.add_sized(
                [ui.available_width(), CTRL_H],
                egui::Button::new(RichText::new(trans_lbl).color(trans_col)),
            )
            .on_hover_text("Włącza delikatną przezroczystość (0.50) wszystkich formatek mebla, aby widzieć wnętrze i krawędzie")
            .clicked()
            {
                let _ = toggle_panels_translucency(state, host);
            }
            ui.add_space(6.0);

            let lib = match HingeLibrary::load() {
                Ok(l) => l,
                Err(err) => {
                    ui.colored_label(COL_RED, &err);
                    return false;
                }
            };
            let current_standard = lib.find_standard(&state.hinge_standard_id).cloned();
            let current_standard_name = current_standard
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_else(|| state.hinge_standard_id.clone());

            let face0_text = match &state.hinge_face_picks[0] {
                Some(f) => format!("{} [f{}]", f.body, f.index),
                None => "Select (1. plecy frontu)".to_string(),
            };
            let has_face0 = state.hinge_face_picks[0].is_some();

            let face1_text = match &state.hinge_face_picks[1] {
                Some(f) => format!("{} [f{}]", f.body, f.index),
                None => "Select (2. bok korpusu)".to_string(),
            };
            let has_face1 = state.hinge_face_picks[1].is_some();

            egui::Grid::new("wood_hinges_grid")
                .num_columns(2)
                .spacing([14.0, 6.0])
                .min_col_width(82.0)
                .show(ui, |ui| {
                    // 1. Model / Standard zawiasu
                    ui.add(egui::Label::new(RichText::new("Model").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    egui::ComboBox::from_id_salt("wood_hinge_standard")
                        .selected_text(&current_standard_name)
                        .width(136.0)
                        .show_ui(ui, |ui| {
                            for std in &lib.standards {
                                if ui
                                    .selectable_value(
                                        &mut state.hinge_standard_id,
                                        std.id.clone(),
                                        &std.name,
                                    )
                                    .changed()
                                {
                                    update_ghost_preview(state, host);
                                }
                            }
                        });
                    ui.end_row();

                    // Krok 1: Plecy frontu
                    let slot0_active = state.hinge_active_slot == 0;
                    ui.add(egui::Label::new(RichText::new("1. Plecy frontu").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        let color = if slot0_active { COL_BLUE } else if has_face0 { COL_GREEN } else { COL_TEXT };
                        let mut text = RichText::new(&face0_text).color(color);
                        if slot0_active { text = text.strong(); }
                        let btn_w = if has_face0 { 110.0 } else { 136.0 };
                        if ui.add_sized([btn_w, CTRL_H], egui::Button::new(text)).clicked() {
                            state.hinge_active_slot = 0;
                            host.enable_face_selection();
                        }
                        if has_face0 && ui.add_sized([22.0, CTRL_H], egui::Button::new("\u{00d7}")).clicked() {
                            state.hinge_face_picks[0] = None;
                            state.hinge_active_slot = 0;
                            host.enable_face_selection();
                            update_ghost_preview(state, host);
                        }
                    });
                    ui.end_row();

                    // Krok 2: Bok korpusu
                    let slot1_active = state.hinge_active_slot == 1;
                    ui.add(egui::Label::new(RichText::new("2. Bok korpusu").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        let color = if slot1_active { COL_BLUE } else if has_face1 { COL_GREEN } else { COL_TEXT };
                        let mut text = RichText::new(&face1_text).color(color);
                        if slot1_active { text = text.strong(); }
                        let btn_w = if has_face1 { 110.0 } else { 136.0 };
                        if ui.add_sized([btn_w, CTRL_H], egui::Button::new(text)).clicked() {
                            state.hinge_active_slot = 1;
                            host.enable_face_selection();
                        }
                        if has_face1 && ui.add_sized([22.0, CTRL_H], egui::Button::new("\u{00d7}")).clicked() {
                            state.hinge_face_picks[1] = None;
                            state.hinge_active_slot = 1;
                            host.enable_face_selection();
                            update_ghost_preview(state, host);
                        }
                    });
                    ui.end_row();

                    // Od brzegu (Z) - domyślnie 100 mm
                    ui.add(egui::Label::new(RichText::new("Od brzegu (Z)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        let z_resp = ui.add_sized([80.0, CTRL_H], egui::TextEdit::singleline(&mut state.hinge_offset_x));
                        if z_resp.changed() {
                            update_ghost_preview(state, host);
                        }
                        ui.label(RichText::new("mm").small().color(COL_MUTED));
                    });
                    ui.end_row();

                    // Ilość (N) - stepper [-] i [+], minimum 2 sztuki!
                    ui.add(egui::Label::new(RichText::new("Ilość (N)").color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
                    ui.horizontal(|ui| {
                        if ui.add_sized([26.0, CTRL_H], egui::Button::new(" - ")).clicked() {
                            let mut cnt = parse_f64_or(&state.hinge_count, 2.0).round() as i64;
                            cnt = (cnt - 1).max(2);
                            state.hinge_count = cnt.to_string();
                            update_ghost_preview(state, host);
                        }
                        let count_resp = ui.add_sized([36.0, CTRL_H], egui::TextEdit::singleline(&mut state.hinge_count));
                        if count_resp.changed() {
                            let cnt = parse_f64_or(&state.hinge_count, 2.0).round().max(2.0) as i64;
                            state.hinge_count = cnt.to_string();
                            update_ghost_preview(state, host);
                        }
                        if ui.add_sized([26.0, CTRL_H], egui::Button::new(" + ")).clicked() {
                            let mut cnt = parse_f64_or(&state.hinge_count, 2.0).round() as i64;
                            cnt = (cnt + 1).min(50);
                            state.hinge_count = cnt.to_string();
                            update_ghost_preview(state, host);
                        }
                        ui.label(RichText::new("szt").small().color(COL_MUTED));
                    });
                    ui.end_row();
                });

            // Ciemna belka informacji o nawierceniach
            if let Some(std) = &current_standard {
                ui.add_space(6.0);
                let avail_w = ui.available_width();
                egui::Frame::NONE
                    .fill(Color32::from_rgb(26, 31, 39))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(45, 53, 66)))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_min_width(avail_w.max(220.0) - 16.0);
                        ui.set_width(avail_w.max(220.0) - 16.0);
                        ui.label(RichText::new("Wymiary nawierceń:").strong().color(COL_TEXT).size(11.5));
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(format!(
                                "Front (Puszka): fi {:.0}×{:.0} mm (K={:.1}) + 2× wkręty fi {:.1}×{:.0} mm\nBok (Prowadnik): 2× wkręty fi {:.1}×{:.0} mm (rozstaw {:.0} mm, od brzegu {:.0} mm)",
                                std.front.cup_diameter_mm, std.front.cup_depth_mm, std.front.edge_distance_k_mm,
                                std.front.screw_hole_diameter_mm, std.front.screw_hole_depth_mm,
                                std.bok.screw_hole_diameter_mm, std.bok.screw_hole_depth_mm,
                                std.bok.spacing_z_mm, std.bok.edge_distance_mm,
                            ))
                            .size(11.0)
                            .color(Color32::from_rgb(180, 195, 215)),
                        );
                    });
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            // Rząd 1 narzędziowy: Wyczyść
            ui.horizontal(|ui| {
                if ui.add_sized([110.0, CTRL_H], egui::Button::new("Wyczyść"))
                    .on_hover_text("Czyści wybrane ściany i wraca do wyboru pleców frontu")
                    .clicked()
                {
                    state.hinge_face_picks = [None, None];
                    state.hinge_active_slot = 0;
                    state.ghost_cylinders = None;
                    host.enable_face_selection();
                }
            });

            ui.add_space(4.0);

            // Rząd 2 zatwierdzający: Zamknij i Zatwierdź
            ui.horizontal(|ui| {
                if ui.add_sized([110.0, CTRL_H], egui::Button::new("Zamknij")).clicked() {
                    close_requested = true;
                }
                if ui.add_sized([130.0, CTRL_H], egui::Button::new(RichText::new("Zatwierdź").strong().color(COL_GREEN)))
                    .on_hover_text("Wierci puszki na froncie oraz otwory prowadników na boku mebla")
                    .clicked()
                {
                    let _ = run_and_report(state, host, create_hinges);
                }
            });
        }

        ActiveDialog::Drawer => {
            const CTRL_H: f32 = 24.0;

            let trans_lbl = if state.panels_translucent {
                "👁 Przezroczystość formatek (WŁ: 0.50)"
            } else {
                "👁 Przezroczystość formatek (WYŁ: 1.0)"
            };
            let trans_col = if state.panels_translucent { COL_GOLD } else { COL_BLUE };
            if ui
                .add_sized(
                    [ui.available_width(), CTRL_H],
                    egui::Button::new(RichText::new(trans_lbl).color(trans_col)),
                )
                .on_hover_text("Włącza delikatną przezroczystość (0.50) wszystkich formatek mebla, aby łatwo wskazać wewnętrzne ściany i widzieć ducha szuflady")
                .clicked()
            {
                let _ = toggle_panels_translucency(state, host);
            }
            ui.add_space(6.0);

            let lib = match DrawerLibrary::load() {
                Ok(l) => l,
                Err(err) => {
                    ui.colored_label(COL_RED, &err);
                    return false;
                }
            };
            let current_sys = lib.find_system(&state.drawer_system_id).cloned();
            let current_sys_name = current_sys
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_else(|| state.drawer_system_id.clone());

            let current_height = current_sys
                .as_ref()
                .and_then(|s| s.find_height(&state.drawer_height_id).cloned());
            let current_height_name = current_height
                .as_ref()
                .map(|h| h.name.clone())
                .unwrap_or_else(|| state.drawer_height_id.clone());

            let nl_val = parse_f64_or(&state.drawer_nl_mm, 450.0);
            let current_length = current_sys
                .as_ref()
                .and_then(|s| s.find_length(nl_val).cloned());
            let current_nl_label = current_length
                .as_ref()
                .map(|l| format!("{:.0} mm", l.nl_mm))
                .unwrap_or_else(|| format!("{} mm", state.drawer_nl_mm));

            egui::Grid::new("wood_drawer_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .min_col_width(88.0)
                .show(ui, |ui| {
                    // System szuflady (ogólny wybór z drawers.json)
                    ui.add(
                        egui::Label::new(RichText::new("System").color(COL_LABEL))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    egui::ComboBox::from_id_salt("wood_drawer_system")
                        .selected_text(&current_sys_name)
                        .width(155.0)
                        .show_ui(ui, |ui| {
                            for sys in &lib.systems {
                                if ui
                                    .selectable_value(
                                        &mut state.drawer_system_id,
                                        sys.id.clone(),
                                        &sys.name,
                                    )
                                    .changed()
                                {
                                    if sys.find_height(&state.drawer_height_id).is_none()
                                        && let Some(first_h) = sys.heights.first()
                                    {
                                        state.drawer_height_id = first_h.id.clone();
                                    }
                                    update_ghost_preview(state, host);
                                }
                            }
                        });
                    ui.end_row();

                    // Lista 1: Wysokość szuflady
                    ui.add(
                        egui::Label::new(RichText::new("Wysokość").color(COL_LABEL))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    egui::ComboBox::from_id_salt("wood_drawer_height")
                        .selected_text(&current_height_name)
                        .width(155.0)
                        .show_ui(ui, |ui| {
                            if let Some(sys) = &current_sys {
                                for h in &sys.heights {
                                    if ui
                                        .selectable_value(
                                            &mut state.drawer_height_id,
                                            h.id.clone(),
                                            &h.name,
                                        )
                                        .changed()
                                    {
                                        update_ghost_preview(state, host);
                                    }
                                }
                            }
                        });
                    ui.end_row();

                    // Lista 2: Głębokość nominalna (NL)
                    ui.add(
                        egui::Label::new(RichText::new("Głębokość (NL)").color(COL_LABEL))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    egui::ComboBox::from_id_salt("wood_drawer_nl")
                        .selected_text(&current_nl_label)
                        .width(155.0)
                        .show_ui(ui, |ui| {
                            if let Some(sys) = &current_sys {
                                for l in &sys.lengths {
                                    let val_str = format!("{:.0}", l.nl_mm);
                                    let lbl_str = format!("{:.0} mm (korpus min. {:.0} mm)", l.nl_mm, l.nl_mm + sys.min_depth_clearance_mm);
                                    if ui
                                        .selectable_value(
                                            &mut state.drawer_nl_mm,
                                            val_str,
                                            lbl_str,
                                        )
                                        .changed()
                                    {
                                        update_ghost_preview(state, host);
                                    }
                                }
                            }
                        });
                    ui.end_row();

                    // 4 kroki wskazania ścian w oknie 3D
                    let step_labels = [
                        ("1. Lewy bok", "Select (wewn. bok L)"),
                        ("2. Prawy bok", "Select (wewn. bok P)"),
                        ("3. Wieniec dolny", "Select (góra wieńca)"),
                        ("4. Plecy frontu", "Select (plecy frontu)"),
                    ];
                    for (slot_idx, (row_lbl, placeholder)) in step_labels.into_iter().enumerate() {
                        let is_active = state.drawer_active_slot == slot_idx;
                        let has_pick = state.drawer_face_picks[slot_idx].is_some();
                        let btn_txt = match &state.drawer_face_picks[slot_idx] {
                            Some(f) => format!("{} [f{}]", f.body, f.index),
                            None => placeholder.to_string(),
                        };

                        ui.add(
                            egui::Label::new(RichText::new(row_lbl).color(COL_LABEL))
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                        ui.horizontal(|ui| {
                            let color = if is_active {
                                COL_BLUE
                            } else if has_pick {
                                COL_GREEN
                            } else {
                                COL_TEXT
                            };
                            let mut text = RichText::new(&btn_txt).color(color);
                            if is_active {
                                text = text.strong();
                            }
                            let btn_w = if has_pick { 129.0 } else { 155.0 };
                            if ui
                                .add_sized([btn_w, CTRL_H], egui::Button::new(text))
                                .clicked()
                            {
                                state.drawer_active_slot = slot_idx;
                                host.enable_face_selection();
                            }
                            if has_pick
                                && ui
                                    .add_sized([22.0, CTRL_H], egui::Button::new("\u{00d7}"))
                                    .clicked()
                            {
                                state.drawer_face_picks[slot_idx] = None;
                                state.drawer_active_slot = slot_idx;
                                host.enable_face_selection();
                                update_ghost_preview(state, host);
                            }
                        });
                        ui.end_row();
                    }

                    // Podniesienie szuflady od górnego boku dolnego wieńca
                    ui.add(
                        egui::Label::new(RichText::new("Podniesienie (Z)").color(COL_LABEL))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    ui.horizontal(|ui| {
                        let z_resp = ui.add_sized(
                            [95.0, CTRL_H],
                            egui::TextEdit::singleline(&mut state.drawer_lift_z),
                        );
                        if z_resp.changed() {
                            update_ghost_preview(state, host);
                        }
                        ui.label(RichText::new("mm od wieńca").small().color(COL_MUTED));
                    });
                    ui.end_row();
                });

            // Belka weryfikacji wymiarów i parametrów formatki DNO + PLECY
            ui.add_space(6.0);
            let avail_w = ui.available_width();
            let frame_stroke_col = if state.drawer_validation_error {
                COL_RED
            } else if !state.drawer_validation_info.is_empty() {
                COL_GREEN
            } else {
                Color32::from_rgb(45, 53, 66)
            };
            egui::Frame::NONE
                .fill(Color32::from_rgb(26, 31, 39))
                .stroke(Stroke::new(1.0, frame_stroke_col))
                .corner_radius(4)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_min_width(avail_w.max(230.0) - 16.0);
                    ui.set_width(avail_w.max(230.0) - 16.0);
                    if !state.drawer_validation_info.is_empty() {
                        let col = if state.drawer_validation_error {
                            COL_RED
                        } else {
                            COL_GREEN
                        };
                        ui.label(
                            RichText::new(&state.drawer_validation_info)
                                .size(11.0)
                                .color(col),
                        );
                    } else if let (Some(sys), Some(h)) = (&current_sys, &current_height) {
                        ui.label(
                            RichText::new("Wzory i nawiercenia systemu:")
                                .strong()
                                .color(COL_TEXT)
                                .size(11.5),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(format!(
                                "Dno: (LW - {:.0}) × (NL - {:.0}) × {:.0} mm\nPlecy: (LW - {:.0}) × {:.0} × {:.0} mm\nOtwory: fi {:.1}×{:.1} mm (prowadnice Z=+{:.0}, front Z=+{:.1}, plecy 9 mm od boku)",
                                sys.bottom_width_deduction_mm,
                                sys.bottom_depth_deduction_mm,
                                sys.board_thickness_mm,
                                sys.back_width_deduction_mm,
                                h.back_height_mm,
                                sys.board_thickness_mm,
                                sys.screw_hole_diameter_mm,
                                sys.screw_hole_depth_mm,
                                sys.runner_hole_z_from_bottom_mm,
                                sys.front_first_hole_z_from_bottom_mm,
                            ))
                            .size(11.0)
                            .color(Color32::from_rgb(180, 195, 215)),
                        );
                    }
                });

            if !state.status_message.is_empty() && !state.status_error {
                ui.add_space(4.0);
                ui.label(RichText::new(&state.status_message).small().color(COL_GREEN));
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                if ui
                    .add_sized([110.0, CTRL_H], egui::Button::new("Wyczyść"))
                    .on_hover_text("Czyści wybrane ściany i wraca do Kroku 1")
                    .clicked()
                {
                    state.drawer_face_picks = [None, None, None, None];
                    state.drawer_active_slot = 0;
                    state.drawer_validation_info.clear();
                    state.drawer_validation_error = false;
                    state.ghost_box = None;
                    state.ghost_cylinders = None;
                    host.enable_face_selection();
                }
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui
                    .add_sized([110.0, CTRL_H], egui::Button::new("Zamknij"))
                    .clicked()
                {
                    close_requested = true;
                }
                if ui
                    .add_sized(
                        [148.0, CTRL_H],
                        egui::Button::new(RichText::new("Zatwierdź").strong().color(COL_GREEN)),
                    )
                    .on_hover_text("Tworzy komponent szuflady z formatkami DNA i PLECÓW oraz wierci otwory znaczące w bokach, froncie i plecach")
                    .clicked()
                {
                    let _ = run_and_report(state, host, create_drawer);
                }
            });
        }

        ActiveDialog::AdaptBodies => {
            const CTRL_H: f32 = 24.0;

            ui.label(
                RichText::new(
                    "Wyszukuje bryły nieprzypisane do komponentów, poza Kubaturą lub powstałe przez odbicie lustrzane / przecięcie. Możesz też kliknąć bryłę w drzewku lub oknie 3D.",
                )
                .size(11.0)
                .color(COL_LABEL),
            );
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("Oznaczenie szafki:").color(COL_LABEL));
                ui.add_sized([60.0, CTRL_H], egui::TextEdit::singleline(&mut state.cabinet_prefix));
                if ui
                    .add_sized([115.0, CTRL_H], egui::Button::new("🔍 Skanuj drzewko"))
                    .on_hover_text("Przeszukuje model w poszukiwaniu luźnych brył lub duplikatów (np. po Mirror/Split)")
                    .clicked()
                {
                    let _ = run_and_report(state, host, scan_adapt_candidates);
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.small_button("☑ Zaznacz wszystkie").clicked() {
                    for c in &mut state.adapt_candidates {
                        c.selected = true;
                    }
                }
                if ui.small_button("☐ Odznacz wszystkie").clicked() {
                    for c in &mut state.adapt_candidates {
                        c.selected = false;
                    }
                }
                let sel_cnt = state.adapt_candidates.iter().filter(|c| c.selected).count();
                ui.label(
                    RichText::new(format!(
                        "Wybrano: {} / {}",
                        sel_cnt,
                        state.adapt_candidates.len()
                    ))
                    .small()
                    .color(COL_GOLD),
                );
            });

            ui.add_space(4.0);
            ui.separator();

            if state.adapt_candidates.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Brak luźnych brył do adaptacji. Kliknij bryłę w drzewku po lewej stronie (lub ścianę w 3D), aby dodać ją ręcznie.",
                    )
                    .small()
                    .color(COL_MUTED),
                );
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("wood_adapt_scroll")
                    .max_height(210.0)
                    .show(ui, |ui| {
                        for (idx, cand) in state.adapt_candidates.iter_mut().enumerate() {
                            egui::Frame::NONE
                                .fill(if cand.selected {
                                    Color32::from_rgb(25, 38, 56)
                                } else {
                                    Color32::from_rgb(20, 24, 31)
                                })
                                .stroke(Stroke::new(
                                    1.0,
                                    if cand.selected { COL_BLUE } else { COL_BORDER },
                                ))
                                .corner_radius(4)
                                .inner_margin(egui::Margin::symmetric(6, 5))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.checkbox(&mut cand.selected, "");
                                        ui.label(
                                            RichText::new(&cand.body_name)
                                                .strong()
                                                .color(if cand.selected { COL_GREEN } else { COL_TEXT }),
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "[{:.0}×{:.0}×{:.0} mm]",
                                                cand.dims_mm[0], cand.dims_mm[1], cand.dims_mm[2]
                                            ))
                                            .small()
                                            .color(COL_GOLD),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "Komponent: {} | Status: {}",
                                                cand.current_component, cand.reason
                                            ))
                                            .small()
                                            .color(COL_MUTED),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("Własna nazwa:").small().color(COL_LABEL));
                                        ui.add(
                                            egui::TextEdit::singleline(&mut cand.custom_name)
                                                .id_salt(("adapt_custom_name", idx))
                                                .desired_width(145.0)
                                                .hint_text("np. wieniec_gora"),
                                        );
                                    });
                                });
                            ui.add_space(3.0);
                        }
                    });
            }

            if !state.status_message.is_empty() && !state.status_error {
                ui.add_space(4.0);
                ui.label(RichText::new(&state.status_message).small().color(COL_GREEN));
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.add_sized([95.0, CTRL_H], egui::Button::new("Zamknij")).clicked() {
                    close_requested = true;
                }
                if ui
                    .add_sized(
                        [155.0, CTRL_H],
                        egui::Button::new(
                            RichText::new("Zagnieźdź jako formatkę")
                                .strong()
                                .color(COL_GREEN),
                        ),
                    )
                    .on_hover_text("Nadaje zaznaczonym bryłom unikalny numer S01_<NR>, tworzy komponenty <Nazwa>_K i zagnieżdża je w Kubaturze")
                    .clicked()
                {
                    let _ = run_and_report(state, host, adapt_selected_bodies);
                }
            });
        }

        ActiveDialog::Edgebanding => {
            const CTRL_H: f32 = 24.0;

            let lib = match EdgebandLibrary::load() {
                Ok(l) => l,
                Err(err) => {
                    ui.colored_label(COL_RED, &err);
                    return false;
                }
            };
            let current_preset = lib.find_preset(&state.edgeband_preset_id).cloned();
            let current_preset_name = current_preset
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| state.edgeband_preset_id.clone());
            let preset_rgb = current_preset
                .as_ref()
                .map(|p| p.rgb)
                .unwrap_or([56, 189, 248]);
            let preset_col32 = Color32::from_rgb(preset_rgb[0], preset_rgb[1], preset_rgb[2]);

            egui::Grid::new("wood_edgeband_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .min_col_width(82.0)
                .show(ui, |ui| {
                    // 1. Wybór grubości obrzeża z pliku edgebanding.json
                    ui.add(
                        egui::Label::new(RichText::new("Obrzeże").color(COL_LABEL))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    ui.horizontal(|ui| {
                        let (swatch_rect, _) =
                            ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        ui.painter().rect(
                            swatch_rect,
                            3.0,
                            preset_col32,
                            Stroke::new(1.0, Color32::WHITE),
                            StrokeKind::Inside,
                        );
                        egui::ComboBox::from_id_salt("wood_edgeband_preset")
                            .selected_text(&current_preset_name)
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                for p in &lib.presets {
                                    let p_col = Color32::from_rgb(p.rgb[0], p.rgb[1], p.rgb[2]);
                                    if ui
                                        .selectable_value(
                                            &mut state.edgeband_preset_id,
                                            p.id.clone(),
                                            RichText::new(&p.name).color(p_col),
                                        )
                                        .changed()
                                    {
                                        update_ghost_preview(state, host);
                                    }
                                }
                            });
                    });
                    ui.end_row();

                    // 2. Tryb pracy: Ręczny vs Automatyczny
                    ui.add(
                        egui::Label::new(RichText::new("Tryb edycji").color(COL_LABEL))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    ui.horizontal(|ui| {
                        for (mode, label) in [
                            (EdgebandMode::Manual, "Ręczny (Brzeg)"),
                            (EdgebandMode::Auto, "Automatyczny"),
                        ] {
                            let is_sel = state.edgeband_mode == mode;
                            let btn_text = RichText::new(label).size(11.0).color(if is_sel {
                                COL_GREEN
                            } else {
                                COL_TEXT
                            });
                            if ui
                                .add_sized([82.0, CTRL_H], egui::Button::new(btn_text).selected(is_sel))
                                .clicked()
                            {
                                state.edgeband_mode = mode;
                                host.enable_face_selection();
                            }
                        }
                    });
                    ui.end_row();

                    // 3. Reguła automatyczna (gdy wybrano Tryb Automatyczny)
                    if state.edgeband_mode == EdgebandMode::Auto {
                        ui.add(
                            egui::Label::new(RichText::new("Reguła auto").color(COL_LABEL))
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                        egui::ComboBox::from_id_salt("wood_edgeband_auto_rule")
                            .selected_text(state.edgeband_auto_rule.label())
                            .width(168.0)
                            .show_ui(ui, |ui| {
                                for rule in EdgebandAutoRule::ALL {
                                    ui.selectable_value(
                                        &mut state.edgeband_auto_rule,
                                        rule,
                                        rule.label(),
                                    );
                                }
                            });
                        ui.end_row();
                    }
                });

            ui.add_space(4.0);
            let hint_msg = match state.edgeband_mode {
                EdgebandMode::Manual => {
                    "Kliknij płaski brzeg formatki w 3D, aby włączyć/wyłączyć okleinę."
                }
                EdgebandMode::Auto => match state.edgeband_auto_rule {
                    EdgebandAutoRule::CornerPair => {
                        "Kliknij ścianę formatki blisko wybranego narożnika (4 obszary → 1D1K)."
                    }
                    _ => "Kliknij dowolną ścianę formatki w 3D, aby nadać wybraną regułę oklejania.",
                },
            };
            ui.label(RichText::new(hint_msg).small().color(COL_MUTED));

            // Podgląd aktualnie wskazanej formatki i szybkie przełączniki 4 brzegów
            if !state.edgeband_last_panel.is_empty() {
                let boxes = extract_panel_boxes(host);
                if let Some(panel) = boxes.iter().find(|b| b.name == state.edgeband_last_panel) {
                    let ext = panel.extents();
                    let thin_axis = panel.thin_axis();
                    let eb = state
                        .staged_edgebands
                        .get(&panel.name)
                        .cloned()
                        .unwrap_or_default();
                    let (code, len_mm) =
                        compute_edgeband_code_and_length(ext[0], ext[1], ext[2], Some(&eb));

                    ui.add_space(4.0);
                    let avail_w = ui.available_width();
                    egui::Frame::NONE
                        .fill(Color32::from_rgb(24, 30, 39))
                        .stroke(Stroke::new(1.0, COL_BORDER))
                        .corner_radius(4)
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.set_min_width(avail_w.max(220.0) - 16.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&panel.name).strong().color(COL_TEXT));
                                ui.label(
                                    RichText::new(format!("→ {code} ({:.0} mm)", len_mm))
                                        .strong()
                                        .color(if code == "0D0K" { COL_MUTED } else { preset_col32 }),
                                );
                            });
                            ui.add_space(3.0);
                            let mut changed_side = None;
                            ui.horizontal(|ui| {
                                for side_idx in 0..4 {
                                    let is_on = eb.edges[side_idx].is_some();
                                    let lbl = narrow_edge_side_label(thin_axis, side_idx);
                                    let txt = RichText::new(lbl).size(10.5).color(if is_on {
                                        preset_col32
                                    } else {
                                        COL_MUTED
                                    });
                                    if ui
                                        .add_sized([58.0, 22.0], egui::Button::new(txt).selected(is_on))
                                        .clicked()
                                    {
                                        changed_side = Some(side_idx);
                                    }
                                }
                            });
                            if let Some(s_idx) = changed_side {
                                let entry = state
                                    .staged_edgebands
                                    .entry(panel.name.clone())
                                    .or_default();
                                if entry.edges[s_idx].is_some() {
                                    entry.edges[s_idx] = None;
                                } else {
                                    entry.edges[s_idx] = Some(state.edgeband_preset_id.clone());
                                }
                                update_ghost_preview(state, host);
                            }
                        });
                }
            }

            let staged_panels_cnt = state
                .staged_edgebands
                .values()
                .filter(|eb| eb.any_banded())
                .count();
            let staged_edges_cnt: usize = state
                .staged_edgebands
                .values()
                .map(|eb| eb.count_banded())
                .sum();
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "W buforze: {} brzegów na {} formatkach",
                    staged_edges_cnt, staged_panels_cnt
                ))
                .small()
                .color(COL_LABEL),
            );

            if !state.status_message.is_empty() && !state.status_error {
                ui.add_space(2.0);
                ui.label(RichText::new(&state.status_message).small().color(COL_GREEN));
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                let has_last = !state.edgeband_last_panel.is_empty();
                if ui
                    .add_enabled(
                        has_last,
                        egui::Button::new("Wyczyść formatkę").min_size(vec2(120.0, CTRL_H)),
                    )
                    .on_hover_text("Usuwa obrzeża z ostatnio wskazanej formatki")
                    .clicked()
                {
                    let last = state.edgeband_last_panel.clone();
                    clear_panel_edgebanding(&last, state, host);
                }
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.add_sized([110.0, CTRL_H], egui::Button::new("Zamknij")).clicked() {
                    close_requested = true;
                }
                if ui
                    .add_sized(
                        [130.0, CTRL_H],
                        egui::Button::new(RichText::new("Zatwierdź").strong().color(COL_GREEN)),
                    )
                    .on_hover_text("Koloruje na stałe oklejone brzegi formatek i zapisuje oznaczenia 2D2K w Tabeli Rozkroju")
                    .clicked()
                {
                    let _ = run_and_report(state, host, apply_edgebanding);
                }
            });
        }

        ActiveDialog::DecorGrain => {
            const CTRL_H: f32 = 24.0;

            let mut lib = match DecorLibrary::load() {
                Ok(l) => l,
                Err(err) => {
                    ui.colored_label(COL_RED, &err);
                    return false;
                }
            };

            let current_preset = lib.find_preset(&state.decor_preset_id).cloned();
            let current_name = current_preset
                .as_ref()
                .map(|p| {
                    let kind = if p.has_grain { "Słoje" } else { "Jednolity" };
                    format!("{} ({})", p.name, kind)
                })
                .unwrap_or_else(|| state.decor_preset_id.clone());
            let preset_rgb = current_preset
                .as_ref()
                .map(|p| p.rgb)
                .unwrap_or([200, 157, 102]);
            let preset_col32 = Color32::from_rgb(preset_rgb[0], preset_rgb[1], preset_rgb[2]);

            // 1. Tryb pracy: DODAJ / USUŃ
            egui::Grid::new("wood_decor_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .min_col_width(80.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new("Tryb").color(COL_LABEL))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    ui.horizontal(|ui| {
                        let add_sel = state.decor_mode == DecorMode::Add;
                        let rem_sel = state.decor_mode == DecorMode::Remove;
                        if ui
                            .add_sized(
                                [82.0, CTRL_H],
                                egui::Button::new(
                                    RichText::new("DODAJ")
                                        .strong()
                                        .size(11.0)
                                        .color(if add_sel { COL_GREEN } else { COL_TEXT }),
                                )
                                .selected(add_sel),
                            )
                            .on_hover_text("Kliknij formatkę, aby nadać kolor dekoru i usłojenie; ponowny klik obraca o 90°")
                            .clicked()
                        {
                            state.decor_mode = DecorMode::Add;
                            host.enable_face_selection();
                        }
                        if ui
                            .add_sized(
                                [82.0, CTRL_H],
                                egui::Button::new(
                                    RichText::new("USUŃ")
                                        .strong()
                                        .size(11.0)
                                        .color(if rem_sel { COL_RED } else { COL_TEXT }),
                                )
                                .selected(rem_sel),
                            )
                            .on_hover_text("Kliknij formatkę, aby usunąć dekor i usłojenie (nie zmienia brzegów)")
                            .clicked()
                        {
                            state.decor_mode = DecorMode::Remove;
                            host.enable_face_selection();
                        }
                    });
                    ui.end_row();

                    // 2. Lista rozwijana dekorów z pliku decors.json
                    ui.add(
                        egui::Label::new(RichText::new("Dekor").color(COL_LABEL))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    ui.horizontal(|ui| {
                        let (swatch_rect, _) =
                            ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        ui.painter().rect(
                            swatch_rect,
                            3.0,
                            preset_col32,
                            Stroke::new(1.0, Color32::WHITE),
                            StrokeKind::Inside,
                        );
                        egui::ComboBox::from_id_salt("wood_decor_preset")
                            .selected_text(&current_name)
                            .width(166.0)
                            .show_ui(ui, |ui| {
                                for p in &lib.presets {
                                    let kind = if p.has_grain { "Słoje" } else { "Jednolity" };
                                    let label = format!("{} [{}]", p.name, kind);
                                    let p_col = Color32::from_rgb(p.rgb[0], p.rgb[1], p.rgb[2]);
                                    ui.horizontal(|ui| {
                                        let (sw, _) = ui.allocate_exact_size(
                                            vec2(11.0, 11.0),
                                            Sense::hover(),
                                        );
                                        ui.painter().rect(
                                            sw,
                                            2.0,
                                            p_col,
                                            Stroke::new(0.8, Color32::WHITE),
                                            StrokeKind::Inside,
                                        );
                                        ui.selectable_value(
                                            &mut state.decor_preset_id,
                                            p.id.clone(),
                                            label,
                                        );
                                    });
                                }
                            });
                    });
                    ui.end_row();
                });

            ui.add_space(4.0);

            // 3. Dodawanie nowego dekoru do pliku decors.json
            egui::Frame::new()
                .fill(Color32::from_rgb(24, 29, 37))
                .stroke(Stroke::new(1.0, COL_BORDER))
                .corner_radius(4.0)
                .inner_margin(6.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Nowy dekor (zapis w decors.json)")
                            .small()
                            .strong()
                            .color(COL_LABEL),
                    );
                    ui.add_space(3.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Nazwa:").small().color(COL_MUTED));
                        ui.add(
                            egui::TextEdit::singleline(&mut state.decor_custom_name)
                                .desired_width(155.0)
                                .hint_text("np. Dąb Evoke K365"),
                        );
                        ui.color_edit_button_srgb(&mut state.decor_custom_rgb);
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut state.decor_custom_has_grain, "Ma usłojenie (słoje)");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(RichText::new("+ Dodaj").small().strong().color(COL_BLUE))
                                .on_hover_text("Dodaje wpis do pliku decors.json i ustawia jako aktywny dekor")
                                .clicked()
                            {
                                let name_trimmed = state.decor_custom_name.trim().to_string();
                                if name_trimmed.is_empty() {
                                    state.status_message = "Podaj nazwę nowego dekoru.".into();
                                    state.status_error = true;
                                } else {
                                    let new_id = lib.upsert_preset(
                                        &name_trimmed,
                                        state.decor_custom_has_grain,
                                        state.decor_custom_rgb,
                                    );
                                    match lib.save() {
                                        Ok(_) => {
                                            state.decor_preset_id = new_id;
                                            state.decor_custom_name.clear();
                                            state.decor_mode = DecorMode::Add;
                                            let kind = if state.decor_custom_has_grain {
                                                "z usłojeniem"
                                            } else {
                                                "jednolity"
                                            };
                                            let msg = format!(
                                                "Zapisano dekor '{}' ({}) do decors.json.",
                                                name_trimmed, kind
                                            );
                                            state.status_message = msg.clone();
                                            state.status_error = false;
                                            host.set_status(&msg, false);
                                        }
                                        Err(err) => {
                                            state.status_message = err.clone();
                                            state.status_error = true;
                                            host.set_status(&err, true);
                                        }
                                    }
                                }
                            }
                        });
                    });
                });

            ui.add_space(4.0);

            if state.decor_mode == DecorMode::Add {
                ui.label(
                    RichText::new("Kliknij dowolną ścianę formatki: nada kolor głównym płaszczyznom; ponowny klik obraca słoje o 90°.")
                        .small()
                        .color(COL_MUTED),
                );
            } else {
                ui.label(
                    RichText::new("Kliknij formatkę, aby usunąć jej dekor i usłojenie (krawędzie z obrzeżem zostaną zachowane).")
                        .small()
                        .color(COL_RED),
                );
            }

            if !state.decor_last_panel.is_empty() {
                ui.add_space(4.0);
                let info_txt = match state.panel_decors.get(&state.decor_last_panel) {
                    Some(d) => {
                        let g = if !d.has_grain {
                            "Jednolity"
                        } else if d.grain_rotated {
                            "90° (W poprzek)"
                        } else {
                            "0° (Wzdłuż)"
                        };
                        format!("Ostatnia: {} → {} [{}]", state.decor_last_panel, d.decor_name, g)
                    }
                    None => format!("Ostatnia: {} → brak dekoru", state.decor_last_panel),
                };
                ui.label(RichText::new(info_txt).small().color(COL_GOLD));
            }

            if !state.status_message.is_empty() && !state.status_error {
                ui.add_space(2.0);
                ui.label(RichText::new(&state.status_message).small().color(COL_GREEN));
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add_sized([248.0, CTRL_H], egui::Button::new("Zamknij")).clicked() {
                    close_requested = true;
                }
            });
        }

        ActiveDialog::Joints => {
            ui.label(
                RichText::new(
                    "Tworzy główny komponent Korpus_Baza (Grounded) oraz komponenty dla wszystkich formatek i łączy je wiązaniami sztywnymi (joint.as_built type: rigid).",
                )
                .color(COL_LABEL),
            );
            ui.add_space(6.0);
            let panels_str = if state.created_panels.is_empty() {
                "Brak".to_string()
            } else {
                state.created_panels.join(", ")
            };
            ui.label(RichText::new(format!("Formatki ({}): {}", state.created_panels.len(), panels_str)).color(COL_TEXT));

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("Połącz Jointami").strong()).clicked() {
                        run_and_report(state, host, bind_all_joints);
                    }
                    if ui.button("Cancel").clicked() {
                        close_requested = true;
                    }
                });
            });
        }
    }

    // Tylko komunikaty błędów na dole okna (Item 10)
    if state.status_error && !state.status_message.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new(&state.status_message).small().color(COL_RED));
    }

    if close_requested {
        if dialog == ActiveDialog::Connectors
            || dialog == ActiveDialog::TDogbone
            || dialog == ActiveDialog::Hinges
            || dialog == ActiveDialog::Drawer
        {
            if state.panels_translucent {
                let _ = set_panels_translucency(state, host, false);
            }
            state.connector_last_edge_pick = None;
            state.connector_last_face_pick = None;
            state.tdogbone_last_edge_pick = None;
            state.tdogbone_last_face_pick = None;
            state.hinge_face_picks = [None, None];
            state.drawer_face_picks = [None, None, None, None];
            state.drawer_validation_info.clear();
            state.drawer_validation_error = false;
            state.ghost_cylinders = None;
            host.enable_face_selection();
        }
        state.ghost_box = None;
        state.ghost_faces.clear();
        state.ghost_arrows.clear();
        state.active_dialog = None;
    }
    close_requested
}

/// Renders the Cutting List / BOM table (`Tabela Rozkroju`) inside a SolidCraft `ui_kit::Window`.
pub fn draw_bom_body(ui: &mut Ui, state: &mut WoodState, host: &mut dyn WoodHost) {
    // Wyłącz domyślne ucinanie tekstu (Truncate) z ui_kit::Window, aby pełne nazwy formatek były zawsze widoczne
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);

    ui.horizontal(|ui| {
        ui.label(RichText::new("Katalog zapisu STEP:").color(COL_LABEL));
        ui.add(egui::TextEdit::singleline(&mut state.export_step_dir).desired_width(320.0));
        if ui.button("📁 Otwórz").on_hover_text("Otwórz katalog w Eksploratorze Windows").clicked() {
            let p = state.export_step_dir.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("explorer").arg(p).spawn();
            });
        }
        ui.separator();
        ui.label(RichText::new("Domyślna szafka:").color(COL_LABEL));
        ui.add(egui::TextEdit::singleline(&mut state.cabinet_prefix).desired_width(52.0));
    });

    ui.add_space(3.0);

    ui.horizontal(|ui| {
        if ui.button(RichText::new("⟳ Przelicz tabelę z modelu").strong()).clicked() {
            run_and_report(state, host, refresh_cutting_list);
        }
        if ui
            .button(
                RichText::new("✔ Przelicz nazwy w modelu")
                    .strong()
                    .color(COL_GOLD),
            )
            .on_hover_text("Zatwierdza wpisane w tabeli oznaczenia szafek (np. S02, S05) i nazwy własne — zmienia nazwy brył i komponentów w modelu (Enter w polu tekstowym nie przelicza modelu)")
            .clicked()
        {
            run_and_report(state, host, apply_bom_renames);
        }
        if ui.button("📋 Kopiuj CSV").clicked() {
            let csv = cutting_list_to_csv(state);
            ui.ctx().copy_text(csv);
            state.status_message = "Skopiowano tabelę rozkroju (CSV) do schowka.".into();
            state.status_error = false;
            host.set_status("Skopiowano tabelę rozkroju (CSV) do schowka.", false);
        }
        if ui
            .button(
                RichText::new("⬇ Eksportuj STEP + CSV")
                    .strong()
                    .color(COL_GREEN),
            )
            .on_hover_text("Zapisuje każdą formatkę jako osobny plik STEP ({KodFormatki}_{G}x{S}x{D}.step, bez numeru Lp) oraz plik CSV z tabelą rozkroju w tym samym folderze")
            .clicked()
        {
            match export_panels_to_step(state, host) {
                Ok(msg) => {
                    state.status_message = msg.clone();
                    state.status_error = false;
                    host.set_status(&msg, false);
                }
                Err(err) => {
                    state.status_message = err.clone();
                    state.status_error = true;
                    host.set_status(&err, true);
                }
            }
        }

        let total_area: f64 = state.cutting_list.iter().map(|r| r.area_m2).sum();
        let total_edgeband_mb: f64 = state
            .cutting_list
            .iter()
            .map(|r| r.edgeband_length_mm * f64::from(r.quantity) / 1000.0)
            .sum();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!(
                    "Razem: {} szt. | {:.3} m² | Obrzeże: {:.2} mb",
                    state.cutting_list.len(),
                    total_area,
                    total_edgeband_mb
                ))
                .strong()
                .color(COL_GREEN),
            );
        });
    });

    ui.add_space(3.0);

    // Szybkie przypisanie numeru szafki dla zakresu numerów formatek (np. formatki 7-11 -> S02)
    ui.horizontal(|ui| {
        ui.label(RichText::new("Przypisz szafkę dla zakresu nr formatek:").small().color(COL_LABEL));
        ui.label(RichText::new("Od nr").small().color(COL_MUTED));
        ui.add(egui::TextEdit::singleline(&mut state.bom_range_from).desired_width(36.0));
        ui.label(RichText::new("Do nr").small().color(COL_MUTED));
        ui.add(egui::TextEdit::singleline(&mut state.bom_range_to).desired_width(36.0));
        ui.label(RichText::new("→ Szafka:").small().color(COL_MUTED));
        ui.add(egui::TextEdit::singleline(&mut state.bom_range_cabinet).desired_width(52.0));
        if ui
            .button(RichText::new("Wpisz do zakresu").color(COL_BLUE))
            .on_hover_text("Wpisuje podany kod szafki (np. S02) do wierszy z zakresu numerów formatek. Aby zapisać w drzewku modelu, kliknij '✔ Przelicz nazwy w modelu'.")
            .clicked()
        {
            match apply_bom_range_cabinet(state) {
                Ok(msg) => {
                    state.status_message = msg.clone();
                    state.status_error = false;
                    host.set_status(&msg, false);
                }
                Err(err) => {
                    state.status_message = err.clone();
                    state.status_error = true;
                    host.set_status(&err, true);
                }
            }
        }
    });

    if !state.status_message.is_empty() {
        ui.add_space(2.0);
        let col = if state.status_error { COL_RED } else { COL_GREEN };
        egui::Frame::new()
            .fill(Color32::from_black_alpha(50))
            .stroke(Stroke::new(1.0, col))
            .inner_margin(4.0)
            .show(ui, |ui| {
                ui.label(RichText::new(&state.status_message).small().color(col));
            });
    }

    ui.separator();

    let avail_w = ui.available_width();
    egui::ScrollArea::both()
        .id_salt("wood_bom_scroll")
        .max_height(380.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_min_width(avail_w);
            egui::Grid::new("wood_bom_table_grid")
                .num_columns(12)
                .striped(true)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    for header in [
                        "Lp.",
                        "Nr",
                        "Szafka",
                        "Nazwa własna (opis)",
                        "Pełna nazwa formatki w modelu",
                        "Długość [mm]",
                        "Szerokość [mm]",
                        "Grubość [mm]",
                        "Obrzeże",
                        "Dekor",
                        "Usłojenie",
                        "Pow. [m²]",
                    ] {
                        ui.label(RichText::new(header).strong().color(COL_LABEL));
                    }
                    ui.end_row();

                    for (idx, row) in state.cutting_list.iter_mut().enumerate() {
                        ui.label(format!("{}", idx + 1));

                        if row.panel_number > 0 {
                            ui.label(RichText::new(format!("{:02}", row.panel_number)).strong().color(COL_BLUE));
                        } else {
                            ui.label(RichText::new("-").color(COL_MUTED));
                        }

                        // Edytowalne pole Szafka (np. S01, S02, S05) — przeliczane wyłącznie przyciskiem!
                        ui.add(
                            egui::TextEdit::singleline(&mut row.cabinet_edit)
                                .id_salt(("bom_cab", idx))
                                .desired_width(58.0),
                        );

                        // Edytowalne pole Nazwa własna (np. wieniec_gora) — przeliczane wyłącznie przyciskiem!
                        ui.add(
                            egui::TextEdit::singleline(&mut row.custom_name_edit)
                                .id_salt(("bom_cust", idx))
                                .desired_width(175.0)
                                .hint_text("np. wieniec_gora"),
                        );

                        // Pełna nazwa formatki w modelu + podgląd po zmianie
                        let preview_num = if row.panel_number > 0 { row.panel_number } else { idx + 1 };
                        let preview_name = build_panel_name(&row.cabinet_edit, preview_num, &row.custom_name_edit);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&row.name).strong().color(COL_TEXT));
                            if preview_name != row.name {
                                ui.label(RichText::new(format!("→ {preview_name}")).small().color(COL_GOLD));
                            }
                        });

                        ui.label(format!("{:.1}", row.length_mm));
                        ui.label(format!("{:.1}", row.width_mm));
                        ui.label(format!("{:.1}", row.thickness_mm));

                        let eb_col = if row.edgeband_code == "0D0K" || row.edgeband_code.is_empty() {
                            COL_MUTED
                        } else {
                            COL_GOLD
                        };
                        ui.label(RichText::new(&row.edgeband_code).strong().color(eb_col));

                        if row.decor_name.is_empty() {
                            ui.label(RichText::new("-").color(COL_MUTED));
                            ui.label(RichText::new("-").color(COL_MUTED));
                        } else {
                            ui.label(RichText::new(&row.decor_name).color(COL_TEXT));
                            let g_col = if row.grain_label.contains("90°") {
                                COL_BLUE
                            } else if row.grain_label.contains("0°") {
                                COL_GREEN
                            } else {
                                COL_MUTED
                            };
                            ui.label(RichText::new(&row.grain_label).strong().color(g_col));
                        }

                        ui.label(format!("{:.3}", row.area_m2));
                        ui.end_row();
                    }
                });
        });

    if state.cutting_list.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("Brak formatek w tabeli. Kliknij '⟳ Przelicz tabelę z modelu' aby odczytać bryły ze sceny.").color(COL_MUTED));
    }
}

fn format_picks(picks: &[crate::FacePick], required: usize, placeholder: &str) -> String {
    if picks.is_empty() {
        return placeholder.to_string();
    }
    let mut s = picks
        .iter()
        .map(|p| format!("{}[f{}]", p.body, p.index))
        .collect::<Vec<_>>()
        .join(", ");
    if picks.len() < required {
        s.push_str(" (kliknij 2. ścianę)");
    }
    s
}

fn form_entry_row(ui: &mut Ui, label: &str, value: &mut String, unit: &str) -> bool {
    let mut changed = false;
    ui.add(egui::Label::new(RichText::new(label).color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
    ui.horizontal(|ui| {
        if ui.add(egui::TextEdit::singleline(value).desired_width(105.0)).changed() {
            changed = true;
        }
        if !unit.is_empty() {
            ui.label(RichText::new(unit).small().color(COL_MUTED));
        }
    });
    ui.end_row();
    changed
}


fn select_slot_row(
    ui: &mut Ui,
    label: &str,
    value_text: &str,
    is_active: bool,
    has_items: bool,
) -> (bool, bool) {
    let mut picked = false;
    let mut cleared = false;
    ui.add(egui::Label::new(RichText::new(label).color(COL_LABEL)).wrap_mode(egui::TextWrapMode::Extend));
    ui.horizontal(|ui| {
        let color = if is_active {
            COL_BLUE
        } else if has_items {
            COL_GREEN
        } else {
            COL_TEXT
        };
        let mut text = RichText::new(value_text).color(color);
        if is_active {
            text = text.strong();
        }
        if ui.button(text).clicked() {
            picked = true;
        }
        if has_items && ui.small_button("\u{00d7}").clicked() {
            cleared = true;
        }
    });
    ui.end_row();
    (picked, cleared)
}
