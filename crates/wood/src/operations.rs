//! Parametric furniture modeling operations for SolidCraft Wood:
//! - `Kubatura` (parametric cabinet bounding box with semi-transparent appearance)
//! - `Formatka Automatyczna` (face extrusion by `Grubosc`)
//! - `Wolna Formatka` (3-step wizard: 1. two side faces -> 2. two front/back faces or 1 face + width -> 3. attachment face + offset)
//! - `Złożenie i Jointy` (rigid `joint.as_built` assembly into `Korpus_Baza`)
//! - `Tabela Rozkroju` (cutting list / BOM dimension extraction)

use serde_json::{Value, json};

use crate::{CuttingRow, FacePick, WoodHost, WoodState};

pub fn parse_f64_or(text: &str, default: f64) -> f64 {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(default)
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Adds or updates a user parameter in the document (`parameters.add` or `parameters.change`).
pub fn set_user_param(host: &mut dyn WoodHost, name: &str, expr: &str) -> Result<(), String> {
    let add_res = host.execute(
        "parameters.add",
        json!({
            "name": name,
            "expression": expr,
            "unit": "mm"
        }),
    );
    if add_res.is_err() {
        host.execute(
            "parameters.change",
            json!({
                "name": name,
                "expression": expr
            }),
        )
        .map_err(|e| format!("Invalid parameter {name}: {e}"))?;
    }
    Ok(())
}

/// Creates only the parametric `Kubatura` envelope and registers global parameters.
pub fn create_kubatura(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let szer = if state.szerokosc.trim().is_empty() { "800".to_string() } else { state.szerokosc.trim().to_string() };
    let gleb = if state.glebokosc.trim().is_empty() { "560".to_string() } else { state.glebokosc.trim().to_string() };
    let wys = if state.wysokosc.trim().is_empty() { "720".to_string() } else { state.wysokosc.trim().to_string() };
    let grub = if state.grubosc.trim().is_empty() { "18".to_string() } else { state.grubosc.trim().to_string() };
    let opacity = parse_f64_or(&state.kubatura_opacity, 0.18).clamp(0.05, 0.95);

    // 1. Create or update global parameters
    set_user_param(host, "Szerokosc", &szer)?;
    set_user_param(host, "Glebokosc", &gleb)?;
    set_user_param(host, "Wysokosc", &wys)?;
    set_user_param(host, "Grubosc", &grub)?;

    // 2. Create the parent component for the cabinet if not already existing
    let comp_list = host.execute("component.list", json!({})).unwrap_or(Value::Null);
    let comps = comp_list.get("components").and_then(Value::as_array).cloned().unwrap_or_default();
    let has_kub = comps.iter().any(|c| {
        c.get("name").and_then(Value::as_str).is_some_and(|n| n == "Kubatura" || n.starts_with("Kubatura:"))
    });
    if !has_kub {
        let _ = host.execute(
            "component.create",
            json!({
                "name": "Kubatura",
                "parent": "root",
                "activate": false,
                "grounded": true
            }),
        );
    }

    // 3. Create the Kubatura box solid
    host.execute(
        "solid.box",
        json!({
            "name": "Kubatura",
            "body_name": "Kubatura",
            "length": "Szerokosc",
            "width": "Glebokosc",
            "height": "Wysokosc",
            "operation": "new"
        }),
    )
    .map_err(|e| format!("Failed to create bounding box body: {e}"))?;

    // 4. Move Kubatura body into the parent component
    let _ = host.execute(
        "component.move_bodies",
        json!({
            "bodies": ["Kubatura"],
            "component": "Kubatura"
        }),
    );

    // 5. Assign semi-transparent appearance
    let _ = host.execute(
        "appearance.assign",
        json!({
            "bodies": ["Kubatura"],
            "color": "#4aa3df",
            "opacity": opacity
        }),
    );

    // 6. Reset active component to root and fit view
    let _ = host.execute("component.activate", json!({"component": "root"}));
    let _ = host.execute("view.fit", json!({}));

    state.created_panels.clear();
    state.panels.clear();
    let _ = refresh_cutting_list(state, host);
    Ok(format!("Created bounding box ({szer}×{gleb}×{wys} mm, thickness {grub} mm)."))
}

/// Updates existing `Szerokosc`, `Glebokosc`, `Wysokosc`, `Grubosc` parameters so all panels recalculate.
pub fn update_kubatura(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let szer = if state.szerokosc.trim().is_empty() { "800".to_string() } else { state.szerokosc.trim().to_string() };
    let gleb = if state.glebokosc.trim().is_empty() { "560".to_string() } else { state.glebokosc.trim().to_string() };
    let wys = if state.wysokosc.trim().is_empty() { "720".to_string() } else { state.wysokosc.trim().to_string() };
    let grub = if state.grubosc.trim().is_empty() { "18".to_string() } else { state.grubosc.trim().to_string() };

    set_user_param(host, "Szerokosc", &szer)?;
    set_user_param(host, "Glebokosc", &gleb)?;
    set_user_param(host, "Wysokosc", &wys)?;
    set_user_param(host, "Grubosc", &grub)?;

    let _ = host.execute("view.fit", json!({}));
    let _ = refresh_cutting_list(state, host);
    Ok(format!("Updated cabinet dimensions ({szer}×{gleb}×{wys} mm, thickness {grub} mm)."))
}

/// Fills `state.auto_face_pick` with one of the standard `Kubatura` faces (`left`, `right`, `bottom`, `top`).
pub fn quick_pick_kubatura(state: &mut WoodState, side: &str) {
    let w = parse_f64_or(&state.szerokosc, 800.0);
    let d = parse_f64_or(&state.glebokosc, 560.0);
    let h = parse_f64_or(&state.wysokosc, 720.0);
    let (name, point, index) = match side {
        "right" => ("Bok_P", [round1(w), round1(d * 0.5), round1(h * 0.5)], 1),
        "bottom" => ("Wieniec_D", [round1(w * 0.5), round1(d * 0.5), 0.0], 4),
        "top" => ("Wieniec_G", [round1(w * 0.5), round1(d * 0.5), round1(h)], 5),
        _ => ("Bok_L", [0.0, round1(d * 0.5), round1(h * 0.5)], 0),
    };
    state.auto_name = name.to_string();
    state.auto_face_pick = Some(FacePick {
        body: "Kubatura".to_string(),
        index,
        point,
    });
}

/// Parsed representation of a furniture panel's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPanelName {
    /// Cabinet prefix (e.g. `"S01"`, `"S05"`, or empty for legacy `"Formatka_30"`).
    pub cabinet: String,
    /// Panel number (e.g. `Some(30)`), or `None` if the body has no panel number yet (e.g. `"Body1"`).
    pub number: Option<usize>,
    /// Optional custom description after the number (e.g. `"wieniec_gora"`).
    pub custom: String,
    /// True if the name ends with a duplicate suffix from Split/Mirror like `" (1)"` or `" (2)"`.
    pub is_duplicate_copy: bool,
}

/// Strips a trailing SolidCraft duplicate suffix like `" (1)"` or `" (2)"`.
pub fn strip_copy_suffix(raw: &str) -> (&str, bool) {
    let trimmed = raw.trim();
    if let Some(open_idx) = trimmed.rfind(" (")
        && let Some(inside) = trimmed.get(open_idx + 2..)
        && let Some(num_part) = inside.strip_suffix(')')
        && !num_part.is_empty()
        && num_part.chars().all(|c| c.is_ascii_digit())
    {
        return (trimmed.get(..open_idx).unwrap_or(trimmed).trim(), true);
    }
    (trimmed, false)
}

/// Sanitizes a single name segment (cabinet code or custom panel description).
pub fn sanitize_name_segment(s: &str) -> String {
    s.trim()
        .chars()
        .map(|c| {
            if c.is_whitespace() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Builds a panel name in the new standard `<Cabinet>_<Number>` (e.g. `S01_05`)
/// or `<Cabinet>_<Number>_<Custom>` (e.g. `S05_30_wieniec_gora`).
pub fn build_panel_name(cabinet: &str, number: usize, custom: &str) -> String {
    let cab = sanitize_name_segment(cabinet);
    let cab = if cab.is_empty() { "S01".to_string() } else { cab };
    let cust = sanitize_name_segment(custom);
    if cust.is_empty() {
        format!("{cab}_{:02}", number)
    } else {
        format!("{cab}_{:02}_{cust}", number)
    }
}

/// Parses a panel or component name, supporting both the new format (`S01_05`, `S05_30_wieniec_gora`)
/// and reading legacy `Formatka_30` names.
pub fn parse_panel_name(raw: &str) -> ParsedPanelName {
    let (without_copy, is_duplicate_copy) = strip_copy_suffix(raw);
    let base = without_copy.split(':').next().unwrap_or(without_copy).trim();
    let base = base.strip_suffix("_K").unwrap_or(base).trim();

    // 1. Read-only support for legacy `Formatka_30` / `Formatka_30_opis`
    if let Some(rest) = base.strip_prefix("Formatka_").or_else(|| base.strip_prefix("Formatka")) {
        let rest = rest.trim_start_matches('_').trim();
        let mut parts = rest.splitn(2, '_');
        let first = parts.next().unwrap_or("").trim();
        let after = parts.next().unwrap_or("").trim();
        if !first.is_empty()
            && first.chars().all(|c| c.is_ascii_digit())
            && let Ok(num) = first.parse::<usize>()
            && num > 0
        {
            return ParsedPanelName {
                cabinet: String::new(),
                number: Some(num),
                custom: after.to_string(),
                is_duplicate_copy,
            };
        }
    }

    // 2. New format `<Cab>_<Nr>` or `<Cab>_<Nr>_<Custom>` (also handles spaces around `_` like `"S01 _05"`)
    let tokens: Vec<&str> = base.split('_').map(str::trim).filter(|s| !s.is_empty()).collect();
    if tokens.len() >= 2 {
        let first_upper = tokens[0].to_ascii_uppercase();
        let reserved = matches!(
            first_upper.as_str(),
            "FORMATKA"
                | "OTWOR"
                | "OTWÓR"
                | "SZYK"
                | "JOINT"
                | "BODY"
                | "BODYTRANSFER"
                | "SKETCH"
                | "EXTRUDE"
                | "SPLIT"
                | "MIRROR"
                | "MOVE"
                | "KUBATURA"
        );
        if !reserved
            && tokens[1].chars().all(|c| c.is_ascii_digit())
            && let Ok(num) = tokens[1].parse::<usize>()
            && num > 0
        {
            let custom = if tokens.len() > 2 {
                tokens[2..].join("_")
            } else {
                String::new()
            };
            return ParsedPanelName {
                cabinet: tokens[0].replace(' ', ""),
                number: Some(num),
                custom,
                is_duplicate_copy,
            };
        }
    }

    ParsedPanelName {
        cabinet: String::new(),
        number: None,
        custom: base.to_string(),
        is_duplicate_copy,
    }
}

/// Extracts a panel number from any body, component, joint, or letter-suffixed hole/pattern feature name
/// so that even if a panel was deleted, its number is detected and never reused.
pub fn extract_panel_number_any(name: &str) -> Option<usize> {
    let parsed = parse_panel_name(name);
    if let Some(n) = parsed.number {
        return Some(n);
    }
    if let Some(after_joint) = name.trim().strip_prefix("Joint_")
        && let Some(n) = parse_panel_name(after_joint).number
    {
        return Some(n);
    }
    for prefix in [
        "Hole_Edge_",
        "Hole_Face_",
        "Hole_Cup_",
        "Hole_Screw_",
        "Hole_Plate_",
        "Otwor_Brzeg_",
        "Otwor_Sciana_",
        "Szyk_Zlacz_",
        "Otwor_Puszka_",
        "Otwor_wkret_",
        "Otwor_prowadnik_",
    ] {
        if let Some(rest) = name.trim().strip_prefix(prefix) {
            let mut parts = rest.split('_');
            let first = parts.next().unwrap_or("").trim();
            let second = parts.next().unwrap_or("").trim();
            if !first.is_empty()
                && first.chars().all(|c| c.is_ascii_digit())
                && !second.is_empty()
                && second.chars().all(|c| c.is_ascii_uppercase())
                && let Ok(num) = first.parse::<usize>()
                && num > 0
            {
                return Some(num);
            }
        }
    }
    None
}

const WOOD_DOC_META_KEY: &str = "com.solidcraft.wood";

fn read_wood_doc_metadata(host: &mut dyn WoodHost) -> serde_json::Map<String, Value> {
    if let Ok(meta_val) = host.execute("document.metadata", json!({}))
        && let Some(obj) = meta_val
            .get("metadata")
            .and_then(|m| m.get(WOOD_DOC_META_KEY))
            .and_then(Value::as_object)
    {
        return obj.clone();
    }
    serde_json::Map::new()
}

fn write_wood_doc_metadata(
    host: &mut dyn WoodHost,
    patch: impl FnOnce(&mut serde_json::Map<String, Value>),
) {
    let mut obj = read_wood_doc_metadata(host);
    let before = obj.clone();
    patch(&mut obj);
    if obj != before {
        let _ = host.execute(
            "document.metadata.set",
            json!({
                "key": WOOD_DOC_META_KEY,
                "value": Value::Object(obj),
            }),
        );
    }
}

/// Scans the entire document (bodies, components, timeline features, metadata) and `WoodState`
/// to find the highest panel number ever assigned in this file.
pub fn max_used_panel_number(state: &WoodState, host: &mut dyn WoodHost) -> usize {
    let mut max_idx = state.highest_panel_number;

    for p in &state.created_panels {
        if let Some(n) = extract_panel_number_any(p) {
            max_idx = max_idx.max(n);
        }
    }
    for k in state.panels.keys() {
        if let Some(n) = extract_panel_number_any(k) {
            max_idx = max_idx.max(n);
        }
    }
    for r in &state.cutting_list {
        max_idx = max_idx.max(r.panel_number);
        if let Some(n) = extract_panel_number_any(&r.name) {
            max_idx = max_idx.max(n);
        }
    }

    let wood_meta = read_wood_doc_metadata(host);
    if let Some(max_val) = wood_meta.get("max_panel_nr") {
        if let Some(v) = max_val.as_u64() {
            max_idx = max_idx.max(v as usize);
        } else if let Some(s) = max_val.as_str()
            && let Ok(v) = s.trim().parse::<usize>()
        {
            max_idx = max_idx.max(v);
        }
    }

    if let Ok(meta_val) = host.execute("component.metadata", json!({ "component": "root" })) {
        let prop_str = meta_val
            .get("properties")
            .and_then(|p| p.get("wood_max_panel_nr"))
            .and_then(Value::as_str)
            .or_else(|| {
                meta_val
                    .get("metadata")
                    .and_then(|m| m.get("properties"))
                    .and_then(|p| p.get("wood_max_panel_nr"))
                    .and_then(Value::as_str)
            });
        if let Some(s) = prop_str
            && let Ok(v) = s.trim().parse::<usize>()
        {
            max_idx = max_idx.max(v);
        }
    }

    if let Ok(comp_val) = host.execute("component.list", json!({}))
        && let Some(comps) = comp_val.get("components").and_then(Value::as_array)
    {
        for c in comps {
            let name = c.get("name").and_then(Value::as_str).unwrap_or("");
            if let Some(n) = extract_panel_number_any(name) {
                max_idx = max_idx.max(n);
            }
            if let Some(bodies) = c.get("bodies").and_then(Value::as_array) {
                for b in bodies {
                    if let Some(bname) = b.as_str()
                        && let Some(n) = extract_panel_number_any(bname)
                    {
                        max_idx = max_idx.max(n);
                    }
                }
            }
        }
    }

    if let Ok(meas) = host.execute("inspect.measure", json!({ "names_only": true }))
        && let Some(bodies) = meas.get("bodies").and_then(Value::as_array)
    {
        for b in bodies {
            if let Some(bname) = b.get("name").and_then(Value::as_str)
                && let Some(n) = extract_panel_number_any(bname)
            {
                max_idx = max_idx.max(n);
            }
        }
    }

    if let Ok(doc_info) = host.inspect_document(false) {
        if let Some(bodies) = doc_info.get("bodies").and_then(Value::as_array) {
            for b in bodies {
                if let Some(bname) = b.get("name").and_then(Value::as_str)
                    && let Some(n) = extract_panel_number_any(bname)
                {
                    max_idx = max_idx.max(n);
                }
            }
        }
        if let Some(timeline) = doc_info.get("timeline").and_then(Value::as_array) {
            for f in timeline {
                if let Some(fname) = f.get("name").and_then(Value::as_str)
                    && let Some(n) = extract_panel_number_any(fname)
                {
                    max_idx = max_idx.max(n);
                }
            }
        }
    }

    max_idx
}

/// Allocates the next strictly increasing panel number (`max_used + 1`),
/// persisting it in `state.highest_panel_number` and in the active `.sc` file's `document.metadata`
/// (`com.solidcraft.wood.max_panel_nr`) so deleted numbers are never reused within that file
/// while staying strictly isolated per `.sc` document.
pub fn allocate_panel_number(state: &mut WoodState, host: &mut dyn WoodHost) -> usize {
    let next_num = max_used_panel_number(state, host) + 1;
    state.highest_panel_number = state.highest_panel_number.max(next_num);
    write_wood_doc_metadata(host, |obj| {
        obj.insert("max_panel_nr".into(), json!(next_num));
    });
    next_num
}

/// Returns the preview string for the next panel name (e.g. `S01_05`) without consuming the number yet.
pub fn next_formatka_name_with_state(state: &WoodState, host: &mut dyn WoodHost) -> String {
    let next_num = max_used_panel_number(state, host) + 1;
    build_panel_name(&state.cabinet_prefix, next_num, "")
}

/// Computes the next sequential panel name in the new `S01_01` format.
pub fn next_formatka_name(host: &mut dyn WoodHost) -> String {
    let dummy = WoodState::default();
    next_formatka_name_with_state(&dummy, host)
}

/// Resolves the final new panel name when creating a panel from `Quick Formatka` or `FORMATKA`,
/// guaranteeing a fresh, never-reused panel number in the `<Szafka>_<Nr>` or `<Szafka>_<Nr>_<WłasnaNazwa>` scheme.
pub fn resolve_new_panel_name(input_name: &str, state: &mut WoodState, host: &mut dyn WoodHost) -> String {
    let num = allocate_panel_number(state, host);
    let trimmed = input_name.trim();
    if trimmed.is_empty() {
        return build_panel_name(&state.cabinet_prefix, num, "");
    }
    let parsed = parse_panel_name(trimmed);
    if parsed.number.is_some() {
        let cab = if parsed.cabinet.is_empty() {
            state.cabinet_prefix.clone()
        } else {
            state.cabinet_prefix = parsed.cabinet.clone();
            parsed.cabinet
        };
        build_panel_name(&cab, num, &parsed.custom)
    } else {
        build_panel_name(&state.cabinet_prefix, num, trimmed)
    }
}

/// Converts a 0-based index into an uppercase letter suffix (`0 -> "A"`, `1 -> "B"`, ..., `25 -> "Z"`, `26 -> "AA"`).
pub fn index_to_letter_suffix(mut idx: usize) -> String {
    let mut chars = Vec::new();
    loop {
        let rem = (idx % 26) as u8;
        chars.push(char::from(b'A' + rem));
        if idx < 26 {
            break;
        }
        idx = (idx / 26) - 1;
    }
    chars.into_iter().rev().collect()
}

/// Collects all existing feature names from the document timeline.
pub fn collect_timeline_feature_names(host: &mut dyn WoodHost) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    if let Ok(doc_info) = host.inspect_document(false)
        && let Some(timeline) = doc_info.get("timeline").and_then(Value::as_array)
    {
        for f in timeline {
            if let Some(name) = f.get("name").and_then(Value::as_str) {
                set.insert(name.to_string());
            }
        }
    }
    set
}

/// Finds the next available letter suffix (`A`, `B`, `C`...) for connector holes on `panel_num`.
pub fn next_connector_letter_for_panel(panel_num: usize, host: &mut dyn WoodHost) -> String {
    let existing = collect_timeline_feature_names(host);
    let num_pad = format!("{:02}", panel_num);
    let num_raw = panel_num.to_string();
    for k in 0..1000 {
        let letter = index_to_letter_suffix(k);
        let candidates = [
            format!("Hole_Edge_{num_pad}_{letter}"),
            format!("Hole_Face_{num_pad}_{letter}"),
            format!("Hole_Edge_{num_raw}_{letter}"),
            format!("Hole_Face_{num_raw}_{letter}"),
            format!("Otwor_Brzeg_{num_pad}_{letter}"),
            format!("Otwor_Sciana_{num_pad}_{letter}"),
            format!("Szyk_Zlacz_{num_pad}_{letter}"),
            format!("Otwor_Brzeg_{num_raw}_{letter}"),
            format!("Otwor_Sciana_{num_raw}_{letter}"),
            format!("Szyk_Zlacz_{num_raw}_{letter}"),
        ];
        if !candidates.iter().any(|c| existing.contains(c)) {
            return letter;
        }
    }
    "A".to_string()
}

/// Finds the next available letter suffix (`A`, `B`, `C`...) for Lamello P-14 features on both panels.
pub fn next_lamello_letter_for_panel(
    edge_panel_num: usize,
    face_panel_num: usize,
    host: &mut dyn WoodHost,
) -> String {
    let existing = collect_timeline_feature_names(host);
    let edge_pad = format!("{:02}", edge_panel_num);
    let face_pad = format!("{:02}", face_panel_num);
    for k in 0..1000 {
        let letter = index_to_letter_suffix(k);
        let candidates = [
            format!("Lamello_Edge_{edge_pad}_{letter}"),
            format!("Lamello_Hex_{edge_pad}_{letter}"),
            format!("Lamello_Face_{face_pad}_{letter}"),
            format!("Lamello_Pattern_Edge_{edge_pad}_{letter}"),
            format!("Lamello_Pattern_Face_{face_pad}_{letter}"),
        ];
        if !candidates.iter().any(|c| existing.contains(c)) {
            return letter;
        }
    }
    "A".to_string()
}

/// Finds the next available letter suffix (`A`, `B`, `C`...) for hinge holes on `panel_num`.
pub fn next_hinge_letter_for_panel(
    panel_num: usize,
    used_in_batch: &[String],
    host: &mut dyn WoodHost,
) -> String {
    let existing = collect_timeline_feature_names(host);
    let num_pad = format!("{:02}", panel_num);
    let num_raw = panel_num.to_string();
    for k in 0..1000 {
        let letter = index_to_letter_suffix(k);
        if used_in_batch.contains(&letter) {
            continue;
        }
        let candidates = [
            format!("Hole_Cup_{num_pad}_{letter}"),
            format!("Hole_Screw_{num_pad}_{letter}_1"),
            format!("Hole_Screw_{num_pad}_{letter}_2"),
            format!("Hole_Plate_{num_pad}_{letter}_1"),
            format!("Hole_Plate_{num_pad}_{letter}_2"),
            format!("Hole_Cup_{num_raw}_{letter}"),
            format!("Hole_Screw_{num_raw}_{letter}_1"),
            format!("Hole_Screw_{num_raw}_{letter}_2"),
            format!("Hole_Plate_{num_raw}_{letter}_1"),
            format!("Hole_Plate_{num_raw}_{letter}_2"),
            format!("Otwor_Puszka_{num_pad}_{letter}"),
            format!("Otwor_wkret_{num_pad}_{letter}_1"),
            format!("Otwor_wkret_{num_pad}_{letter}_2"),
            format!("Otwor_prowadnik_{num_pad}_{letter}_1"),
            format!("Otwor_prowadnik_{num_pad}_{letter}_2"),
            format!("Otwor_Puszka_{num_raw}_{letter}"),
            format!("Otwor_wkret_{num_raw}_{letter}_1"),
            format!("Otwor_wkret_{num_raw}_{letter}_2"),
            format!("Otwor_prowadnik_{num_raw}_{letter}_1"),
            format!("Otwor_prowadnik_{num_raw}_{letter}_2"),
        ];
        if !candidates.iter().any(|c| existing.contains(c)) {
            return letter;
        }
    }
    "A".to_string()
}

/// Ensures that `body_name` has a valid, unique panel number.
/// - If `body_name` already has a panel number (e.g. `S01_30...` or legacy `Formatka_30` without duplicate suffix `(2)`),
///   returns `(body_name, number)`.
/// - If `body_name` has no panel number yet (e.g. `Body1` or split/mirrored `Formatka_07 (2)`),
///   allocates the next unused panel number, renames the body to `S01_<NR>` in the model, nests it into `Kubatura`,
///   and returns `(new_name, new_number)`.
pub fn ensure_body_is_numbered_panel(
    body_name: &str,
    state: &mut WoodState,
    host: &mut dyn WoodHost,
) -> (String, usize) {
    let parsed = parse_panel_name(body_name);
    if let Some(num) = parsed.number
        && !parsed.is_duplicate_copy
    {
        state.highest_panel_number = state.highest_panel_number.max(num);
        return (body_name.to_string(), num);
    }

    let num = allocate_panel_number(state, host);
    let new_name = build_panel_name(&state.cabinet_prefix, num, "");
    if host
        .execute("body.rename", json!({ "body": body_name, "name": new_name }))
        .is_ok()
    {
        let _ = bind_single_panel_joint(&new_name, host);
        if !state.created_panels.contains(&new_name) {
            state.created_panels.push(new_name.clone());
        }
        return (new_name, num);
    }
    (body_name.to_string(), num)
}

/// Result of computing 1 or N panels in `Quick Formatka` (1-face mode or 2-face division mode).
#[derive(Clone, Debug)]
pub struct ComputedAutoPanelPlan {
    pub panels: Vec<ComputedPanelBounds>,
    pub clearances_mm: Vec<f64>,
    pub total_span_mm: f64,
    pub axis_name: &'static str,
    pub normal_sign: f64,
}

/// Formats the calculated clear opening dimensions (`pola światła`) from selection 1 to selection 2.
pub fn format_clearances_text(clearances: &[f64]) -> String {
    let parts: Vec<String> = clearances
        .iter()
        .map(|&v| {
            let r = (v * 10.0).round() / 10.0;
            if (r - r.round()).abs() < 1e-4 {
                format!("{:.0}", r)
            } else {
                format!("{:.1}", r)
            }
        })
        .collect();
    format!("Clearance (1→2): {} mm", parts.join(", "))
}

/// Resolves the bounding box `[min, max]` of `body_name` across root and nested components,
/// falling back to `[0, 0, 0]..[default_w, default_d, default_h]`.
fn resolve_reference_body_bbox(
    body_name: &str,
    default_w: f64,
    default_d: f64,
    default_h: f64,
    host: &mut dyn WoodHost,
) -> ([f64; 3], [f64; 3]) {
    let mut b_min = [0.0, 0.0, 0.0];
    let mut b_max = [default_w, default_d, default_h];

    if let Ok(single_meas) = host.execute("inspect.measure", json!({ "bodies": [body_name] }))
        && let Some(sb) = single_meas.get("bodies").and_then(Value::as_array).and_then(|a| a.first())
        && let Some(bb) = sb.get("bbox")
        && let (Some(mn), Some(mx)) = (bb.get("min").and_then(Value::as_array), bb.get("max").and_then(Value::as_array))
        && let (Some(x0), Some(y0), Some(z0), Some(x1), Some(y1), Some(z1)) = (
            mn.first().and_then(Value::as_f64),
            mn.get(1).and_then(Value::as_f64),
            mn.get(2).and_then(Value::as_f64),
            mx.first().and_then(Value::as_f64),
            mx.get(1).and_then(Value::as_f64),
            mx.get(2).and_then(Value::as_f64),
        )
    {
        return ([x0.min(x1), y0.min(y1), z0.min(z1)], [x0.max(x1), y0.max(y1), z0.max(z1)]);
    }

    if let Ok(doc_info) = host.inspect_document(true)
        && let Some(bodies) = doc_info.get("bodies").and_then(Value::as_array)
        && let Some(b) = bodies.iter().find(|b| b.get("name").and_then(Value::as_str) == Some(body_name))
        && let Some(bb) = b.get("bbox").or_else(|| b.get("bounds"))
        && let (Some(mn), Some(mx)) = (bb.get("min").and_then(Value::as_array), bb.get("max").and_then(Value::as_array))
        && let (Some(x0), Some(y0), Some(z0), Some(x1), Some(y1), Some(z1)) = (
            mn.first().and_then(Value::as_f64),
            mn.get(1).and_then(Value::as_f64),
            mn.get(2).and_then(Value::as_f64),
            mx.first().and_then(Value::as_f64),
            mx.get(1).and_then(Value::as_f64),
            mx.get(2).and_then(Value::as_f64),
        )
    {
        b_min = [x0.min(x1), y0.min(y1), z0.min(z1)];
        b_max = [x0.max(x1), y0.max(y1), z0.max(z1)];
    }

    (b_min, b_max)
}

/// Resolves the outward face normal axis (`"X"`, `"Y"`, `"Z"`) and sign (`+1.0` or `-1.0`) for `pick`,
/// using `model.faces` when available and falling back to normalized box-face classification on `(b_min, b_max)`.
fn resolve_auto_face_axis_and_sign(
    pick: &FacePick,
    b_min: [f64; 3],
    b_max: [f64; 3],
    host: &mut dyn WoodHost,
) -> (&'static str, f64) {
    if let Ok(faces_val) = host.execute("model.faces", json!({ "body": pick.body }))
        && let Some(faces_arr) = faces_val.get("faces").and_then(Value::as_array)
        && !faces_arr.is_empty()
    {
        let (_norm, axis_name, sign) =
            crate::geometry::resolve_face_normal(&pick.body, pick.index, pick.point, host);
        return (axis_name, sign);
    }

    let pbox = PanelBox {
        name: pick.body.clone(),
        min: b_min,
        max: b_max,
    };
    let resolved = resolve_face_from_box(&pbox, pick.point);
    let axis_name = match resolved.axis {
        0 => "X",
        1 => "Y",
        _ => "Z",
    };
    (axis_name, resolved.normal_sign)
}

/// Computes the full geometry plan for `Quick Formatka`:
/// - When `pick2` is `None`: creates 1 panel on `pick1` shifted according to `direction` (`Inside`, `Outside`, `Center`).
/// - When `pick2` is `Some`: verifies both faces have normals along the same axis, and divides the span between
///   `pick1` and `pick2` into `division_count` panels (`1` panel with `Inside`/`Outside`/`Center`, or `2..=N` equal divisions).
#[allow(clippy::too_many_arguments)]
pub fn compute_auto_panel_plan(
    pick1: &FacePick,
    pick2: Option<&FacePick>,
    thick_val: f64,
    offset_val: f64,
    margins: [f64; 4],
    direction: crate::AutoPanelDirection,
    division_count: u32,
    default_w: f64,
    default_d: f64,
    default_h: f64,
    host: &mut dyn WoodHost,
) -> Result<ComputedAutoPanelPlan, String> {
    let (b_min, b_max) = resolve_reference_body_bbox(&pick1.body, default_w, default_d, default_h, host);
    let (axis_name_1, sign1) = resolve_auto_face_axis_and_sign(pick1, b_min, b_max, host);

    let (axis3, axis1, axis2, plane) = match axis_name_1 {
        "X" => (0, 1, 2, "YZ"),
        "Y" => (1, 0, 2, "XZ"),
        _ => (2, 0, 1, "XY"),
    };

    let min_ax1 = round1(b_min[axis1] + margins[0]);
    let max_ax1 = round1(b_max[axis1] - margins[1]);
    let min_ax2 = round1(b_min[axis2] + margins[2]);
    let max_ax2 = round1(b_max[axis2] - margins[3]);

    if let Some(p2) = pick2 {
        let (b2_min, b2_max) = resolve_reference_body_bbox(&p2.body, default_w, default_d, default_h, host);
        let (axis_name_2, _sign2) = resolve_auto_face_axis_and_sign(p2, b2_min, b2_max, host);
        if axis_name_1 != axis_name_2 {
            return Err(format!(
                "Selected faces do not share the same normal axis (1: {axis_name_1} axis, 2: {axis_name_2} axis). Pick two parallel faces."
            ));
        }

        let c1 = pick1.point[axis3];
        let c2 = p2.point[axis3];
        let span = (c2 - c1).abs();
        if span < 1.0 {
            return Err("Selected two faces lie on the same plane (distance 0 mm). Pick two separated faces.".into());
        }

        let dir_1_to_2 = if c2 >= c1 { 1.0 } else { -1.0 };
        let count = division_count.clamp(1, 50);

        if span <= f64::from(count) * thick_val {
            return Err(format!(
                "Distance between faces ({:.1} mm) is too small for {}× panel of thickness {:.1} mm.",
                span, count, thick_val
            ));
        }

        let mut panels = Vec::with_capacity(count as usize);
        let mut clearances_mm = Vec::with_capacity((count + 1) as usize);

        if count == 1 {
            let mid_d = 0.5 * span + offset_val;
            let (d_start, d_end) = match direction {
                crate::AutoPanelDirection::Center => (mid_d - 0.5 * thick_val, mid_d + 0.5 * thick_val),
                crate::AutoPanelDirection::Outside => (mid_d, mid_d + thick_val),
                crate::AutoPanelDirection::Inside => (mid_d - thick_val, mid_d),
            };

            clearances_mm.push(round1(d_start));
            clearances_mm.push(round1(span - d_end));

            let p_a = c1 + dir_1_to_2 * d_start;
            let p_b = c1 + dir_1_to_2 * d_end;

            let mut coord_min = [0.0; 3];
            let mut coord_max = [0.0; 3];
            coord_min[axis3] = round1(p_a.min(p_b));
            coord_max[axis3] = round1(p_a.max(p_b));
            coord_min[axis1] = min_ax1;
            coord_max[axis1] = max_ax1;
            coord_min[axis2] = min_ax2;
            coord_max[axis2] = max_ax2;

            panels.push(ComputedPanelBounds {
                min: coord_min,
                max: coord_max,
                plane,
                thickness_axis: axis3,
                axis1,
                axis2,
            });
        } else {
            // Dla 2, 3, 4... kierunek jest zablokowany na Środek — równe światło między zaznaczeniami i formatkami
            let gap = (span - f64::from(count) * thick_val) / f64::from(count + 1);
            for k in 0..count {
                let d_start = f64::from(k + 1) * gap + f64::from(k) * thick_val + offset_val;
                let d_end = d_start + thick_val;

                let p_a = c1 + dir_1_to_2 * d_start;
                let p_b = c1 + dir_1_to_2 * d_end;

                let mut coord_min = [0.0; 3];
                let mut coord_max = [0.0; 3];
                coord_min[axis3] = round1(p_a.min(p_b));
                coord_max[axis3] = round1(p_a.max(p_b));
                coord_min[axis1] = min_ax1;
                coord_max[axis1] = max_ax1;
                coord_min[axis2] = min_ax2;
                coord_max[axis2] = max_ax2;

                panels.push(ComputedPanelBounds {
                    min: coord_min,
                    max: coord_max,
                    plane,
                    thickness_axis: axis3,
                    axis1,
                    axis2,
                });
            }

            for k in 0..=count {
                let g = if k == 0 {
                    gap + offset_val
                } else if k == count {
                    gap - offset_val
                } else {
                    gap
                };
                clearances_mm.push(round1(g));
            }
        }

        return Ok(ComputedAutoPanelPlan {
            panels,
            clearances_mm,
            total_span_mm: round1(span),
            axis_name: axis_name_1,
            normal_sign: sign1,
        });
    }

    // Tryb 1 ściany:
    let (start, end) = match direction {
        crate::AutoPanelDirection::Inside => {
            let dir = -sign1;
            let base = pick1.point[axis3] + dir * offset_val;
            (base, base + dir * thick_val)
        }
        crate::AutoPanelDirection::Outside => {
            let dir = sign1;
            let base = pick1.point[axis3] + dir * offset_val;
            (base, base + dir * thick_val)
        }
        crate::AutoPanelDirection::Center => {
            let dir = -sign1;
            let mid = pick1.point[axis3] + dir * offset_val;
            (mid - 0.5 * thick_val, mid + 0.5 * thick_val)
        }
    };

    let mut coord_min = [0.0; 3];
    let mut coord_max = [0.0; 3];
    coord_min[axis3] = round1(start.min(end));
    coord_max[axis3] = round1(start.max(end));
    coord_min[axis1] = min_ax1;
    coord_max[axis1] = max_ax1;
    coord_min[axis2] = min_ax2;
    coord_max[axis2] = max_ax2;

    Ok(ComputedAutoPanelPlan {
        panels: vec![ComputedPanelBounds {
            min: coord_min,
            max: coord_max,
            plane,
            thickness_axis: axis3,
            axis1,
            axis2,
        }],
        clearances_mm: Vec::new(),
        total_span_mm: 0.0,
        axis_name: axis_name_1,
        normal_sign: sign1,
    })
}

/// Dynamically computes panel bounds for Quick Formatka from picked face, offset and margins.
pub fn compute_auto_panel_geometry(
    pick: &FacePick,
    thick_val: f64,
    offset_val: f64,
    margins: [f64; 4],
    inward: bool,
    default_w: f64,
    default_d: f64,
    default_h: f64,
    host: &mut dyn WoodHost,
) -> Result<ComputedPanelBounds, String> {
    let dir_mode = if inward {
        crate::AutoPanelDirection::Inside
    } else {
        crate::AutoPanelDirection::Outside
    };
    let plan = compute_auto_panel_plan(
        pick,
        None,
        thick_val,
        offset_val,
        margins,
        dir_mode,
        1,
        default_w,
        default_d,
        default_h,
        host,
    )?;
    plan.panels
        .into_iter()
        .next()
        .ok_or_else(|| "No computed panel geometry available.".to_string())
}

/// Creates 1 or N panel solids (`solid.box` with `"operation": "new"`) for Quick Panel respecting offsets, margins, and 2-face division.
pub fn create_auto_panel(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let Some(pick1) = state.auto_face_pick.clone() else {
        return Err("Click the 1st base face in the 3D viewport first.".into());
    };
    let pick2 = state.auto_face_pick_2.clone();
    let raw_input_name = state.auto_name.clone();
    let thick = if state.auto_thickness.trim().is_empty() {
        "Grubosc".to_string()
    } else {
        state.auto_thickness.trim().to_string()
    };
    let thick_val = parse_f64_or(&thick, 18.0);
    let offset_val = parse_f64_or(&state.auto_plane_offset, 0.0);
    let margins = [
        parse_f64_or(&state.auto_margins[0], 0.0),
        parse_f64_or(&state.auto_margins[1], 0.0),
        parse_f64_or(&state.auto_margins[2], 0.0),
        parse_f64_or(&state.auto_margins[3], 0.0),
    ];
    let default_w = parse_f64_or(&state.szerokosc, 800.0);
    let default_d = parse_f64_or(&state.glebokosc, 560.0);
    let default_h = parse_f64_or(&state.wysokosc, 720.0);
    let div_count = parse_f64_or(&state.auto_division_count, 1.0)
        .round()
        .clamp(1.0, 50.0) as u32;

    if pick2.is_some() && div_count >= 2 {
        state.auto_direction = crate::AutoPanelDirection::Center;
    }
    state.auto_inward = state.auto_direction != crate::AutoPanelDirection::Outside;

    let plan = compute_auto_panel_plan(
        &pick1,
        pick2.as_ref(),
        thick_val,
        offset_val,
        margins,
        state.auto_direction,
        div_count,
        default_w,
        default_d,
        default_h,
        host,
    )?;

    let _ = host.execute("component.activate", json!({ "component": "root" }));

    let parsed_input = parse_panel_name(&raw_input_name);
    let cab = if parsed_input.cabinet.is_empty() {
        state.cabinet_prefix.clone()
    } else {
        parsed_input.cabinet.clone()
    };

    let mut created_names = Vec::new();
    for (idx, geom) in plan.panels.iter().enumerate() {
        let len_x = round1(geom.max[0] - geom.min[0]);
        let len_y = round1(geom.max[1] - geom.min[1]);
        let len_z = round1(geom.max[2] - geom.min[2]);

        if len_x <= 0.0 || len_y <= 0.0 || len_z <= 0.0 {
            return Err("Computed panel dimensions are zero or negative. Check offset and margins.".into());
        }

        let name = if idx == 0 {
            resolve_new_panel_name(&raw_input_name, state, host)
        } else {
            let num = allocate_panel_number(state, host);
            build_panel_name(&cab, num, &parsed_input.custom)
        };

        host.execute(
            "solid.box",
            json!({
                "name": name,
                "body_name": name,
                "corner": geom.min,
                "length": len_x,
                "width": len_y,
                "height": len_z,
                "operation": "new",
            }),
        )?;

        if !state.created_panels.iter().any(|p| p == &name) {
            state.created_panels.push(name.clone());
        }
        state.auto_panel_counter += 1;

        let role = match geom.plane {
            "XY" => if pick2.is_some() { "Polka".into() } else { "Wieniec".into() },
            "XZ" => "Plecy".into(),
            _ => if pick2.is_some() { "Przegroda".into() } else { "Bok".into() },
        };

        state.panels.insert(
            name.clone(),
            crate::PanelData {
                name: name.clone(),
                role,
                thickness: thick_val,
                length: 0.0,
                width: 0.0,
                axis: plan.axis_name.to_string(),
                normal_sign: plan.normal_sign,
                inward: state.auto_inward,
                is_flipped: false,
                offset: offset_val,
                reference_body: pick1.body.clone(),
                reference_point: pick1.point,
            },
        );

        let _ = bind_single_panel_joint(&name, host);
        created_names.push(name);
    }

    state.ghost_box = None;
    state.auto_face_pick = None;
    state.auto_face_pick_2 = None;
    state.auto_active_slot = 0;
    state.auto_clearances_info.clear();
    state.auto_clearances_error = false;
    state.auto_name = next_formatka_name_with_state(state, host);
    host.enable_face_selection();
    let _ = refresh_cutting_list(state, host);

    if created_names.len() == 1 {
        Ok(format!("Created panel '{}'.", created_names[0]))
    } else {
        Ok(format!(
            "Created {}× panels: {}.",
            created_names.len(),
            created_names.join(", ")
        ))
    }
}

/// Structure holding the computed 3D bounds and plane description for a panel.
#[derive(Clone, Copy, Debug)]
pub struct ComputedPanelBounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub plane: &'static str,
    pub thickness_axis: usize, // 0 for X, 1 for Y, 2 for Z
    pub axis1: usize,
    pub axis2: usize,
}

/// Dynamically detects the orthogonal axes based on the selected face directions (normals and point offsets)
/// and computes the 3D bounds for the ghost preview or panel creation.
pub fn compute_free_panel_geometry(
    slot0: &[FacePick],
    slot1: &[FacePick],
    slot2: &[FacePick],
    width_val: f64,
    thick_val: f64,
    plane_off_val: f64,
    margins: [f64; 4],
    inward: bool,
    default_w: f64,
    default_d: f64,
    default_h: f64,
    host: &mut dyn WoodHost,
) -> Result<ComputedPanelBounds, String> {
    let (Some(p0), Some(p1)) = (slot0.first(), slot0.get(1)) else {
        return Err("Step 1: Pick two opposite side faces in the 3D viewport.".into());
    };

    // 1. Rozpoznanie osi Kroku 1 (np. Prawo-Lewo -> X, Przód-Tył -> Y, Góra-Dół -> Z)
    let d0 = (p0.point[0] - p1.point[0]).abs();
    let d1 = (p0.point[1] - p1.point[1]).abs();
    let d2 = (p0.point[2] - p1.point[2]).abs();

    let axis1 = if d0 > 20.0 && d0 >= d1 && d0 >= d2 {
        0 // Rozpiętość X
    } else if d1 > 20.0 && d1 >= d0 && d1 >= d2 {
        1 // Rozpiętość Y
    } else if d2 > 20.0 && d2 >= d0 && d2 >= d1 {
        2 // Rozpiętość Z
    } else {
        let (_n0, ax0_name, _) = crate::geometry::resolve_face_normal(&p0.body, p0.index, p0.point, host);
        match ax0_name {
            "X" => 0,
            "Y" => 1,
            _ => 2,
        }
    };

    // 2. Rozpoznanie osi Kroku 2 (musi być prostopadła do osi Kroku 1!)
    let axis2 = if let (Some(fp), Some(bp)) = (slot1.first(), slot1.get(1)) {
        let d2_0 = (fp.point[0] - bp.point[0]).abs();
        let d2_1 = (fp.point[1] - bp.point[1]).abs();
        let d2_2 = (fp.point[2] - bp.point[2]).abs();
        match axis1 {
            0 => if d2_1 >= d2_2 { 1 } else { 2 },
            1 => if d2_0 >= d2_2 { 0 } else { 2 },
            _ => if d2_0 >= d2_1 { 0 } else { 1 },
        }
    } else if let Some(fp) = slot1.first() {
        let (_n_fp, ax_fp, _) = crate::geometry::resolve_face_normal(&fp.body, fp.index, fp.point, host);
        let cand = match ax_fp {
            "X" => 0,
            "Y" => 1,
            _ => 2,
        };
        if cand != axis1 {
            cand
        } else {
            match axis1 {
                0 => 1,
                1 => 2,
                _ => 1,
            }
        }
    } else {
        // Domyślna druga oś zanim użytkownik wskaże Krok 2
        match axis1 {
            0 => 1, // Krok 1 = X -> domyślnie Y (półka XY)
            1 => 2, // Krok 1 = Y -> domyślnie Z (przegroda YZ)
            _ => 1, // Krok 1 = Z -> domyślnie Y
        }
    };

    // 3. Trzecia oś (grubość / powierzchnia przyklejenia) to pozostała oś:
    let axis3 = 3 - axis1 - axis2;

    let plane = match axis3 {
        2 => "XY",
        1 => "XZ",
        _ => "YZ",
    };

    let default_dim = [default_w, default_d, default_h];
    let mut coord_min = [0.0; 3];
    let mut coord_max = [0.0; 3];

    // Rozpiętość wzdłuż osi Kroku 1
    coord_min[axis1] = p0.point[axis1].min(p1.point[axis1]) + margins[0];
    coord_max[axis1] = p0.point[axis1].max(p1.point[axis1]) - margins[1];

    // Rozpiętość wzdłuż osi Kroku 2
    if let (Some(fp), Some(bp)) = (slot1.first(), slot1.get(1)) {
        let c_a = fp.point[axis2].min(bp.point[axis2]) + margins[2];
        let c_b = fp.point[axis2].max(bp.point[axis2]) - margins[3];
        coord_min[axis2] = c_a.min(c_b);
        coord_max[axis2] = c_a.max(c_b);
    } else if let Some(fp) = slot1.first() {
        let (_n, _ax, sign) = crate::geometry::resolve_face_normal(&fp.body, fp.index, fp.point, host);
        let c_f = fp.point[axis2];
        let start = c_f + sign * margins[2];
        let end = start + sign * width_val;
        coord_min[axis2] = start.min(end);
        coord_max[axis2] = start.max(end);
    } else {
        coord_min[axis2] = 0.0 + margins[2];
        coord_max[axis2] = default_dim[axis2] - margins[3];
    }

    // Rozpiętość wzdłuż osi Kroku 3 (grubość formatki)
    if let Some(tp) = slot2.first() {
        let (_norm, _ax_name, sign) = crate::geometry::resolve_face_normal(&tp.body, tp.index, tp.point, host);
        let dir = if inward { -sign } else { sign };
        let base = tp.point[axis3];
        let start = base + dir * plane_off_val;
        let end = start + dir * thick_val;
        coord_min[axis3] = start.min(end);
        coord_max[axis3] = start.max(end);
    } else {
        // Pozycja uśredniona przed wybraniem Kroku 3
        let avg = (p0.point[axis3] + p1.point[axis3]) * 0.5;
        coord_min[axis3] = avg - thick_val * 0.5;
        coord_max[axis3] = avg + thick_val * 0.5;
    }

    for i in 0..3 {
        coord_min[i] = round1(coord_min[i]);
        coord_max[i] = round1(coord_max[i]);
    }

    Ok(ComputedPanelBounds {
        min: coord_min,
        max: coord_max,
        plane,
        thickness_axis: axis3,
        axis1,
        axis2,
    })
}

/// Core 3-step solver for `Wolna Formatka` (Free Panel).
#[allow(clippy::too_many_arguments)]
pub fn create_free_panel_from_picks(
    name: &str,
    slot0: &[FacePick],
    slot1: &[FacePick],
    slot2: &[FacePick],
    thick: &str,
    plane_off_str: &str,
    margins: [f64; 4],
    width_str: &str,
    inward: bool,
    default_w: f64,
    default_d: f64,
    default_h: f64,
    host: &mut dyn WoodHost,
) -> Result<ComputedPanelBounds, String> {
    if slot2.is_empty() {
        return Err("Step 3: Pick an attachment face in the 3D viewport.".into());
    }

    let width_val = parse_f64_or(width_str, 100.0);
    let thick_val = parse_f64_or(thick, 18.0);
    let plane_off_val = parse_f64_or(plane_off_str, 0.0);

    let geom = compute_free_panel_geometry(
        slot0,
        slot1,
        slot2,
        width_val,
        thick_val,
        plane_off_val,
        margins,
        inward,
        default_w,
        default_d,
        default_h,
        host,
    )?;

    let x_min = geom.min[0];
    let y_min = geom.min[1];
    let z_min = geom.min[2];
    let x_max = geom.max[0];
    let y_max = geom.max[1];
    let z_max = geom.max[2];

    let len_x = round1(x_max - x_min);
    let len_y = round1(y_max - y_min);
    let len_z = round1(z_max - z_min);

    if len_x <= 0.0 || len_y <= 0.0 || len_z <= 0.0 {
        return Err("Computed panel dimensions are zero or negative. Check selected faces.".into());
    }

    host.execute(
        "solid.box",
        json!({
            "name": name,
            "body_name": name,
            "corner": [x_min, y_min, z_min],
            "length": len_x,
            "width": len_y,
            "height": len_z,
            "operation": "new",
        }),
    )?;

    // Automatycznie łączymy formatkę Jointem do bazy mebla
    let _ = bind_single_panel_joint(name, host);

    Ok(geom)
}

/// Przełącza widoczność bryły `Kubatura` w oknie 3D (ukrycie ułatwia klikanie wewnętrznych ścian mebla).
pub fn toggle_kubatura_visibility(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    state.kubatura_hidden = !state.kubatura_hidden;
    host.execute(
        "browser.visibility",
        json!({
            "items": ["Kubatura"],
            "visible": !state.kubatura_hidden,
        }),
    )?;
    Ok(if state.kubatura_hidden {
        "Bounding box hidden (you can now pick interior cabinet faces freely).".to_string()
    } else {
        "Bounding box shown.".to_string()
    })
}

/// Przełącza delikatną przezroczystość (`0.60` vs `1.0`) wszystkich formatek mebla w oknie 3D,
/// aby widoczne były wewnętrzne krawędzie oraz nawiercane otwory na kołki i wkręty.
pub fn toggle_panels_translucency(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    set_panels_translucency(state, host, !state.panels_translucent)
}

/// Bezpośrednio włącza lub wyłącza przezroczystość formatek (0.60 vs 1.0) z zachowaniem oryginalnego koloru.
pub fn set_panels_translucency(state: &mut WoodState, host: &mut dyn WoodHost, translucent: bool) -> Result<String, String> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(doc_info) = host.inspect_document(false)
        && let Some(bodies) = doc_info.get("bodies").and_then(Value::as_array)
    {
        for b in bodies {
            if let Some(n) = b.get("name").and_then(Value::as_str) {
                if !n.is_empty() && n != "Kubatura" && !names.contains(&n.to_string()) {
                    names.push(n.to_string());
                }
            }
        }
    }
    for p in &state.created_panels {
        if p != "Kubatura" && !names.contains(p) {
            names.push(p.clone());
        }
    }
    if names.is_empty() {
        state.panels_translucent = false;
        return Ok("No panels in the scene to toggle translucency.".into());
    }

    state.panels_translucent = translucent;

    if state.panels_translucent {
        // Ustawiamy styl przezroczystości identyczny z Kubaturą (#4aa3df) z mocą 0.50
        for n in &names {
            let _ = host.execute(
                "appearance.assign",
                json!({
                    "bodies": [n],
                    "color": "#4aa3df",
                    "opacity": 0.50,
                }),
            );
        }
        Ok("Panel translucency enabled (0.50).".to_string())
    } else {
        // Przywróć 1.0: wyczyszczenie override'u przywraca naturalny materiał bryły
        for n in &names {
            let _ = host.execute(
                "appearance.assign",
                json!({
                    "bodies": [n],
                }),
            );
        }
        Ok("Restored full panel opacity (1.0).".to_string())
    }
}

/// Interactive handler for `Wolna Formatka` button in dialog.
pub fn create_free_panel(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let raw_input_name = state.free_name.clone();
    let thick = if state.free_thickness.trim().is_empty() {
        "Grubosc".to_string()
    } else {
        state.free_thickness.trim().to_string()
    };
    let plane_off_str = if state.free_plane_offset.trim().is_empty() {
        "0".to_string()
    } else {
        state.free_plane_offset.trim().to_string()
    };
    let margins = [
        parse_f64_or(&state.free_margins[0], 0.0),
        parse_f64_or(&state.free_margins[1], 0.0),
        parse_f64_or(&state.free_margins[2], 0.0),
        parse_f64_or(&state.free_margins[3], 0.0),
    ];
    let width_str = if state.free_width.trim().is_empty() {
        "100".to_string()
    } else {
        state.free_width.trim().to_string()
    };

    let slot0 = state.free_picks[0].clone();
    let slot1 = state.free_picks[1].clone();
    let slot2 = state.free_picks[2].clone();

    let default_w = parse_f64_or(&state.szerokosc, 800.0);
    let default_d = parse_f64_or(&state.glebokosc, 560.0);
    let default_h = parse_f64_or(&state.wysokosc, 720.0);

    let name = resolve_new_panel_name(&raw_input_name, state, host);

    let geom = create_free_panel_from_picks(
        &name,
        &slot0,
        &slot1,
        &slot2,
        &thick,
        &plane_off_str,
        margins,
        &width_str,
        state.free_inward,
        default_w,
        default_d,
        default_h,
        host,
    )?;

    if !state.created_panels.iter().any(|p| p == &name) {
        state.created_panels.push(name.clone());
    }
    state.free_panel_counter += 1;

    // Zapisz właściwości / atrybuty formatki do state.panels
    if let Some(target_pick) = slot2.first() {
        let (_norm, _axis_name, sign) = crate::geometry::resolve_face_normal(
            &target_pick.body,
            target_pick.index,
            target_pick.point,
            host,
        );

        let axis_str = match geom.thickness_axis {
            0 => "X",
            1 => "Y",
            _ => "Z",
        };
        let role = match geom.plane {
            "XY" => if name.contains("Trawers") { "Trawers".into() } else { "Wieniec".into() },
            "XZ" => "Plecy".into(),
            _ => "Przegroda".into(),
        };

        state.panels.insert(name.clone(), crate::PanelData {
            name: name.clone(),
            role,
            thickness: parse_f64_or(&thick, 18.0),
            length: 0.0,
            width: 0.0,
            axis: axis_str.to_string(),
            normal_sign: sign,
            inward: state.free_inward,
            is_flipped: false,
            offset: parse_f64_or(&plane_off_str, 0.0),
            reference_body: target_pick.body.clone(),
            reference_point: target_pick.point,
        });
    }

    state.ghost_box = None;
    state.free_picks = [Vec::new(), Vec::new(), Vec::new()];
    state.active_pick_slot = 0;
    state.free_name = next_formatka_name_with_state(state, host);
    host.enable_face_selection();
    let _ = refresh_cutting_list(state, host);

    Ok(format!("Created Free Panel '{name}'."))
}

/// Dynamicznie wylicza bryłę "Ducha formatki" oraz cylindrów złączy przed zatwierdzeniem.
pub fn update_ghost_preview(state: &mut WoodState, host: &mut dyn WoodHost) {
    state.ghost_box = None;
    state.ghost_cylinders = None;
    state.ghost_lamello.clear();
    state.ghost_faces.clear();
    state.ghost_arrows.clear();
    match state.active_dialog {
        Some(crate::ActiveDialog::DecorGrain) => {
            let boxes = extract_panel_boxes(host);
            for panel in &boxes {
                let Some(decor) = state.panel_decors.get(&panel.name) else {
                    continue;
                };
                if !decor.has_grain {
                    continue;
                }
                let thin_axis = panel.thin_axis();
                let (axis1, axis2) = panel_in_plane_axes(thin_axis);
                let ext = panel.extents();
                let (long_axis, short_axis) = if ext[axis1] >= ext[axis2] {
                    (axis1, axis2)
                } else {
                    (axis2, axis1)
                };
                let grain_axis = if decor.grain_rotated {
                    short_axis
                } else {
                    long_axis
                };
                let perp_axis = if grain_axis == long_axis {
                    short_axis
                } else {
                    long_axis
                };
                let mut dir = [0.0; 3];
                dir[grain_axis] = 1.0;
                let mut perp = [0.0; 3];
                perp[perp_axis] = 1.0;

                let span = ext[grain_axis];
                let half_len = (span * 0.28).clamp(18.0, 220.0).min(span * 0.44);

                let lum = 0.299 * f64::from(decor.rgb[0])
                    + 0.587 * f64::from(decor.rgb[1])
                    + 0.114 * f64::from(decor.rgb[2]);
                let arrow_col = if lum > 140.0 {
                    [15, 23, 42, 245]
                } else {
                    [250, 204, 21, 250]
                };
                let label = if decor.grain_rotated {
                    "90° (Poprzek)".to_string()
                } else {
                    "0° (Along)".to_string()
                };

                let box_center = panel.center();
                for normal_sign in [-1.0_f64, 1.0_f64] {
                    let mut center = box_center;
                    center[thin_axis] = if normal_sign < 0.0 {
                        panel.min[thin_axis] - 0.2
                    } else {
                        panel.max[thin_axis] + 0.2
                    };
                    let mut normal = [0.0; 3];
                    normal[thin_axis] = normal_sign;
                    state.ghost_arrows.push(crate::GhostGrainArrow {
                        panel_name: panel.name.clone(),
                        center,
                        dir,
                        perp,
                        normal,
                        half_len,
                        label: label.clone(),
                        color: arrow_col,
                    });
                }
            }
        }
        Some(crate::ActiveDialog::Edgebanding) => {
            let Ok(lib) = crate::EdgebandLibrary::load() else {
                return;
            };
            let boxes = extract_panel_boxes(host);
            for panel in &boxes {
                let Some(eb) = state.staged_edgebands.get(&panel.name) else {
                    continue;
                };
                for (side_idx, opt_preset_id) in eb.edges.iter().enumerate() {
                    let Some(preset_id) = opt_preset_id else {
                        continue;
                    };
                    let rgb = lib
                        .find_preset(preset_id)
                        .map(|p| p.rgb)
                        .unwrap_or([56, 189, 248]);
                    let (corners, _probe, _n_ax, _n_sign, _r_ax, _len) =
                        narrow_edge_quad_and_probe(panel, side_idx);
                    state.ghost_faces.push(crate::GhostFacePreview {
                        corners,
                        // Delikatny obrys i bardzo subtelne wypełnienie (alpha 38 ≈ 15%),
                        // aby wiele podświetlonych brzegów na szafce nie zlewało się w paski.
                        color_stroke: [rgb[0], rgb[1], rgb[2], 185],
                        color_fill: [rgb[0], rgb[1], rgb[2], 38],
                    });
                }
            }
        }
        Some(crate::ActiveDialog::Connectors) => {
            let Some(edge) = &state.connector_edge_pick else {
                return;
            };
            let Ok(lib) = crate::ConnectorLibrary::load() else {
                return;
            };
            let Some((_, preset)) = lib.find_preset(&state.connector_preset_id) else {
                return;
            };
            let offset_x = parse_f64_or(&state.connector_offset_x, 50.0).max(5.0);
            let shift_total = parse_f64_or(&state.connector_offset_shift, 0.0);
            let shift_offset = (shift_total * 0.5) * state.connector_shift_sign;
            let count = parse_f64_or(&state.connector_count, 2.0).round().clamp(1.0, 100.0) as u32;

            let boxes = extract_panel_boxes(host);
            if let Ok(plan) = compute_connector_plan(edge, state.connector_face_pick.as_ref(), &boxes, offset_x, shift_offset, count) {
                let mut centers = Vec::new();
                for i in 0..plan.count {
                    let mut p = plan.first_hole_pos;
                    for a in 0..3 {
                        p[a] += f64::from(i) * plan.spacing * plan.pattern_dir[a];
                    }
                    centers.push(p);
                }
                let is_screw = state.connector_category_id == "screws" || state.connector_category_id == "wkrety";
                let radius = preset.edge_hole.diameter * 0.5;
                let depth_edge = preset.edge_hole.depth;
                let depth_face = if state.connector_through_face || preset.face_hole.depth <= 0.0 {
                    plan.face_thickness
                } else {
                    preset.face_hole.depth
                };
                state.ghost_cylinders = Some(crate::GhostCylinderData {
                    items: Vec::new(),
                    centers,
                    axis: plan.dir_into_edge,
                    radius,
                    depth_edge,
                    depth_face,
                    is_screw,
                });
            }
        }
        Some(crate::ActiveDialog::Hinges) => {
            let Some(front_pick) = state.hinge_face_picks[0].clone() else {
                return;
            };
            let Some(side_pick) = state.hinge_face_picks[1].clone() else {
                return;
            };
            let Ok(lib) = crate::HingeLibrary::load() else {
                return;
            };
            let Some(standard) = lib.find_standard(&state.hinge_standard_id) else {
                return;
            };
            let offset_z = parse_f64_or(&state.hinge_offset_x, 100.0).max(20.0);
            let count = parse_f64_or(&state.hinge_count, 2.0).round().clamp(2.0, 50.0) as u32;

            let boxes = extract_panel_boxes(host);
            if let Ok(plan) = compute_hinge_plan(
                &front_pick,
                &side_pick,
                standard,
                &state.hinge_cup_fixing,
                &state.hinge_plate_screw,
                offset_z,
                count,
                &boxes,
                host,
            ) {
                let mut items = Vec::new();
                for inst in &plan.instances {
                    // 1. Puszka zawiasu na froncie (Cylinder bursztynowo-złoty)
                    items.push(crate::GhostCylinderItem {
                        base_center: inst.cup_hole.position,
                        axis: inst.cup_hole.direction,
                        radius: inst.cup_hole.diameter * 0.5,
                        depth: inst.cup_hole.depth,
                        color_stroke: [245, 158, 11, 255],
                        color_fill: [245, 158, 11, 70],
                        label: inst.cup_hole.label.clone(),
                    });

                    // 2. Dwa otwory mocujące puszkę na froncie (wkręty błękitne)
                    let (stroke_col, fill_col) = ([56, 189, 248, 255], [56, 189, 248, 60]);
                    for fix in &inst.cup_fixing_holes {
                        items.push(crate::GhostCylinderItem {
                            base_center: fix.position,
                            axis: fix.direction,
                            radius: fix.diameter * 0.5,
                            depth: fix.depth,
                            color_stroke: stroke_col,
                            color_fill: fill_col,
                            label: String::new(),
                        });
                    }

                    // 3. Dwa otwory pod prowadnik na boku mebla (błękitne)
                    for (p_idx, plate) in inst.plate_holes.iter().enumerate() {
                        items.push(crate::GhostCylinderItem {
                            base_center: plate.position,
                            axis: plate.direction,
                            radius: plate.diameter * 0.5,
                            depth: plate.depth,
                            color_stroke: [56, 189, 248, 255],
                            color_fill: [56, 189, 248, 60],
                            label: if p_idx == 0 { plate.label.clone() } else { String::new() },
                        });
                    }
                }
                state.ghost_cylinders = Some(crate::GhostCylinderData {
                    items,
                    ..Default::default()
                });
            }
        }
        Some(crate::ActiveDialog::Drawer) => {
            state.drawer_validation_info.clear();
            state.drawer_validation_error = false;
            let (Some(p0), Some(p1), Some(p2), Some(p3)) = (
                state.drawer_face_picks[0].clone(),
                state.drawer_face_picks[1].clone(),
                state.drawer_face_picks[2].clone(),
                state.drawer_face_picks[3].clone(),
            ) else {
                return;
            };
            let Ok(lib) = crate::DrawerLibrary::load() else {
                return;
            };
            let Some(system) = lib.find_system(&state.drawer_system_id) else {
                return;
            };
            let Some(height_var) = system.find_height(&state.drawer_height_id) else {
                return;
            };
            let nl_val = parse_f64_or(&state.drawer_nl_mm, 450.0);
            let Some(length_var) = system.find_length(nl_val) else {
                return;
            };
            let lift_z = parse_f64_or(&state.drawer_lift_z, 0.0).max(0.0);

            let boxes = extract_panel_boxes(host);
            match compute_drawer_plan(
                &p0,
                &p1,
                &p2,
                &p3,
                system,
                height_var,
                length_var,
                lift_z,
                &boxes,
                host,
            ) {
                Ok(plan) => {
                    if !plan.fits_depth || !plan.fits_height {
                        state.drawer_validation_error = true;
                        let mut msgs = Vec::new();
                        if !plan.fits_depth {
                            msgs.push(format!(
                                "⚠ Cabinet depth too small: min. {:.0} mm required (NL {:.0} + {:.0}), available {:.1} mm!",
                                plan.required_depth_mm,
                                plan.nl_mm,
                                system.min_depth_clearance_mm + plan.inset_setback_mm,
                                plan.avail_depth_mm
                            ));
                        }
                        if !plan.fits_height {
                            msgs.push(format!(
                                "⚠ Height too small: min. {:.0} mm required, available {:.1} mm above bottom panel (+lift)!",
                                plan.required_height_mm,
                                plan.avail_height_mm
                            ));
                        }
                        state.drawer_validation_info = msgs.join("\n");
                    } else {
                        let mode_lbl = if plan.is_inset_front {
                            format!("Inset front (+{:.0} mm)", plan.inset_setback_mm)
                        } else if system.push_open_front_gap_mm > 0.0 {
                            format!("Push-Open (min. {:.0} mm gap)", system.push_open_front_gap_mm)
                        } else {
                            "Overlay front".to_string()
                        };
                        state.drawer_validation_info = format!(
                            "✔ Fits in cabinet ({mode_lbl} | LW={:.1} mm | depth {:.0}/{:.0} mm | height {:.0}/{:.0} mm)\nBottom: {:.1} × {:.1} × {:.0} mm  |  Back: {:.1} × {:.1} × {:.0} mm",
                            plan.lw_mm,
                            plan.required_depth_mm,
                            plan.avail_depth_mm,
                            plan.required_height_mm,
                            plan.avail_height_mm,
                            plan.bottom_panel_dims_mm.0,
                            plan.bottom_panel_dims_mm.1,
                            plan.bottom_panel_dims_mm.2,
                            plan.back_panel_dims_mm.0,
                            plan.back_panel_dims_mm.1,
                            plan.back_panel_dims_mm.2,
                        );
                    }

                    // Zielony box: maksymalne wymiary zajmowane przez szufladę bez frontu
                    state.ghost_box = Some(crate::GhostPreviewData {
                        min: plan.envelope_min,
                        max: plan.envelope_max,
                        extra_boxes: Vec::new(),
                        plane: plan.envelope_plane,
                        margins: [0.0; 4],
                        axis1: plan.width_axis,
                        axis2: plan.depth_axis,
                    });

                    // Błękitne cylindry: otwory znaczące w bokach, froncie i plecach szuflady
                    let (stroke_col, fill_col) = ([56, 189, 248, 255], [56, 189, 248, 70]);
                    let mut items = Vec::new();
                    for h in plan
                        .side1_holes
                        .iter()
                        .chain(plan.side2_holes.iter())
                        .chain(plan.front_holes.iter())
                        .chain(plan.back_holes.iter())
                    {
                        items.push(crate::GhostCylinderItem {
                            base_center: h.position,
                            axis: h.direction,
                            radius: h.diameter * 0.5,
                            depth: h.depth,
                            color_stroke: stroke_col,
                            color_fill: fill_col,
                            label: h.label.clone(),
                        });
                    }
                    state.ghost_cylinders = Some(crate::GhostCylinderData {
                        items,
                        is_screw: true,
                        ..Default::default()
                    });
                }
                Err(err) => {
                    state.drawer_validation_info = err;
                    state.drawer_validation_error = true;
                }
            }
        }
        Some(crate::ActiveDialog::TDogbone) => {
            let Some(edge) = state.tdogbone_edge_pick.as_ref() else {
                return;
            };
            let joint_width = parse_f64_or(&state.tdogbone_width, 100.0).max(16.0);
            let offset_x = parse_f64_or(&state.tdogbone_offset_x, 50.0).max(0.0);
            let count = parse_f64_or(&state.tdogbone_count, 1.0).round().clamp(1.0, 20.0) as u32;

            let boxes = extract_panel_boxes(host);
            if let Ok(plan) = compute_tdogbone_plan(
                edge,
                state.tdogbone_face_pick.as_ref(),
                &boxes,
                joint_width,
                offset_x,
                count,
            ) {
                let tenon_boxes = plan.tenon_bounding_boxes();
                if let Some(&(first_min, first_max)) = tenon_boxes.first() {
                    let extra_boxes = tenon_boxes[1..].to_vec();
                    let plane = match plan.drill_axis {
                        0 => "YZ",
                        1 => "XZ",
                        _ => "XY",
                    };
                    state.ghost_box = Some(crate::GhostPreviewData {
                        min: first_min,
                        max: first_max,
                        extra_boxes,
                        plane,
                        margins: [0.0; 4],
                        axis1: plan.edge_axis,
                        axis2: plan.width_axis,
                    });
                }
            }
        }
        Some(crate::ActiveDialog::LamelloP14) => {
            let Some(edge) = state.lamello_edge_pick.as_ref() else {
                return;
            };
            let z_offset = parse_f64_or(&state.lamello_z_offset, 9.0).max(1.0);
            let offset_x = parse_f64_or(&state.lamello_offset_x, 50.0).max(0.0);
            let count = parse_f64_or(&state.lamello_count, 2.0).round().clamp(1.0, 20.0) as u32;

            let boxes = extract_panel_boxes(host);
            if let Ok(plan) = compute_lamello_p14_plan(
                edge,
                state.lamello_face_pick.as_ref(),
                &boxes,
                z_offset,
                offset_x,
                count,
            ) {
                let mut edge_dir = [0.0; 3];
                edge_dir[plan.edge_axis] = 1.0;
                let mut width_dir = [0.0; 3];
                width_dir[plan.width_axis] = 1.0;

                for &c_u in &plan.joint_centers {
                    let mut center = [0.0; 3];
                    center[plan.edge_axis] = c_u;
                    center[plan.width_axis] = plan.w_center;
                    center[plan.drill_axis] = plan.contact_d;

                    state.ghost_lamello.push(crate::GhostLamelloSlot {
                        contact_center: center,
                        edge_dir,
                        width_dir,
                        into_edge_dir: plan.dir_into_edge,
                        radius: 0.5 * LAMELLO_BLADE_DIA_MM,
                        thickness: LAMELLO_SLOT_WIDTH_MM,
                        depth: LAMELLO_SLOT_DEPTH_MM,
                    });
                }
            }
        }
        Some(crate::ActiveDialog::FreePanel) => {
            let slot0 = &state.free_picks[0];
            if slot0.len() < 2 {
                return;
            }

            let thick_val = parse_f64_or(&state.free_thickness, 18.0);
            let margins = [
                parse_f64_or(&state.free_margins[0], 0.0),
                parse_f64_or(&state.free_margins[1], 0.0),
                parse_f64_or(&state.free_margins[2], 0.0),
                parse_f64_or(&state.free_margins[3], 0.0),
            ];
            let width_val = parse_f64_or(&state.free_width, 100.0);
            let plane_off_val = parse_f64_or(&state.free_plane_offset, 0.0);

            let slot1 = &state.free_picks[1];
            let slot2 = &state.free_picks[2];

            let default_w = parse_f64_or(&state.szerokosc, 800.0);
            let default_d = parse_f64_or(&state.glebokosc, 560.0);
            let default_h = parse_f64_or(&state.wysokosc, 720.0);

            if let Ok(geom) = compute_free_panel_geometry(
                slot0,
                slot1,
                slot2,
                width_val,
                thick_val,
                plane_off_val,
                margins,
                state.free_inward,
                default_w,
                default_d,
                default_h,
                host,
            ) {
                state.ghost_box = Some(crate::GhostPreviewData {
                    min: geom.min,
                    max: geom.max,
                    extra_boxes: Vec::new(),
                    plane: geom.plane,
                    margins,
                    axis1: geom.axis1,
                    axis2: geom.axis2,
                });
            }
        }

        Some(crate::ActiveDialog::AutoPanel) => {
            state.auto_clearances_info.clear();
            state.auto_clearances_error = false;
            if let Some(pick1) = state.auto_face_pick.clone() {
                let pick2 = state.auto_face_pick_2.clone();
                let thick_val = parse_f64_or(&state.auto_thickness, 18.0);
                let offset_val = parse_f64_or(&state.auto_plane_offset, 0.0);
                let margins = [
                    parse_f64_or(&state.auto_margins[0], 0.0),
                    parse_f64_or(&state.auto_margins[1], 0.0),
                    parse_f64_or(&state.auto_margins[2], 0.0),
                    parse_f64_or(&state.auto_margins[3], 0.0),
                ];
                let default_w = parse_f64_or(&state.szerokosc, 800.0);
                let default_d = parse_f64_or(&state.glebokosc, 560.0);
                let default_h = parse_f64_or(&state.wysokosc, 720.0);
                let div_count = parse_f64_or(&state.auto_division_count, 1.0)
                    .round()
                    .clamp(1.0, 50.0) as u32;

                if pick2.is_some() && div_count >= 2 {
                    state.auto_direction = crate::AutoPanelDirection::Center;
                }

                match compute_auto_panel_plan(
                    &pick1,
                    pick2.as_ref(),
                    thick_val,
                    offset_val,
                    margins,
                    state.auto_direction,
                    div_count,
                    default_w,
                    default_d,
                    default_h,
                    host,
                ) {
                    Ok(plan) => {
                        if !plan.clearances_mm.is_empty() {
                            state.auto_clearances_info = format_clearances_text(&plan.clearances_mm);
                        }
                        if let Some(first) = plan.panels.first() {
                            let extra_boxes = plan
                                .panels
                                .iter()
                                .skip(1)
                                .map(|p| (p.min, p.max))
                                .collect();
                            state.ghost_box = Some(crate::GhostPreviewData {
                                min: first.min,
                                max: first.max,
                                extra_boxes,
                                plane: first.plane,
                                margins,
                                axis1: first.axis1,
                                axis2: first.axis2,
                            });
                        }
                    }
                    Err(err) => {
                        state.auto_clearances_info = err;
                        state.auto_clearances_error = true;
                    }
                }
            }
        }

        _ => {}
    }
}

/// Odwraca stronę przytulenia formatki do powierzchni odniesienia ("Odwróć Stronę").
pub fn flip_panel_side(panel_name: &str, state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    if panel_name.is_empty() || panel_name == "Kubatura" {
        return Err("Select a valid panel to flip.".into());
    }

    // Pobierz informacje o bryle z dokumentu
    let doc_info = host.inspect_document(true)?;
    let bodies = doc_info.get("bodies").and_then(Value::as_array).cloned().unwrap_or_default();
    let body_info = bodies.iter().find(|b| b.get("name").and_then(Value::as_str) == Some(panel_name));
    let Some(b) = body_info else {
        return Err(format!("Nie znaleziono formatki '{panel_name}' w dokumencie."));
    };

    let bbox = b.get("bbox").or_else(|| b.get("bounds"));
    let (dx_box, dy_box, dz_box) = if let Some(bb) = bbox {
        let min = bb.get("min").and_then(Value::as_array);
        let max = bb.get("max").and_then(Value::as_array);
        if let (Some(mn), Some(mx)) = (min, max)
            && let (Some(x0), Some(y0), Some(z0), Some(x1), Some(y1), Some(z1)) = (
                mn.first().and_then(Value::as_f64),
                mn.get(1).and_then(Value::as_f64),
                mn.get(2).and_then(Value::as_f64),
                mx.first().and_then(Value::as_f64),
                mx.get(1).and_then(Value::as_f64),
                mx.get(2).and_then(Value::as_f64),
            )
        {
            ((x1 - x0).abs(), (y1 - y0).abs(), (z1 - z0).abs())
        } else {
            (18.0, 18.0, 18.0)
        }
    } else {
        (18.0, 18.0, 18.0)
    };

    // Ustal oś grubości, grubość i aktualny stan odwrócenia
    let (axis, thickness, is_flipped) = if let Some(p) = state.panels.get(panel_name) {
        (p.axis.clone(), p.thickness, p.is_flipped)
    } else {
        let ax = if dx_box <= dy_box && dx_box <= dz_box {
            "X"
        } else if dy_box <= dx_box && dy_box <= dz_box {
            "Y"
        } else {
            "Z"
        };
        let t = match ax {
            "X" => dx_box,
            "Y" => dy_box,
            _ => dz_box,
        };
        (ax.to_string(), t, false)
    };

    let dir = if is_flipped { 1.0 } else { -1.0 };
    let shift = dir * thickness;
    let translate = match axis.as_str() {
        "X" => [shift, 0.0, 0.0],
        "Y" => [0.0, shift, 0.0],
        _ => [0.0, 0.0, shift],
    };

    host.execute(
        "solid.move",
        json!({
            "bodies": [panel_name],
            "translate": translate,
        }),
    ).map_err(|e| format!("Failed to flip panel '{panel_name}': {e}"))?;

    // Zaktualizuj stan formatki
    let new_flipped = !is_flipped;
    if let Some(p) = state.panels.get_mut(panel_name) {
        p.is_flipped = new_flipped;
    } else {
        state.panels.insert(panel_name.to_string(), crate::PanelData {
            name: panel_name.to_string(),
            role: "Formatka".to_string(),
            thickness,
            length: dx_box.max(dy_box).max(dz_box),
            width: (dx_box + dy_box + dz_box) - dx_box.max(dy_box).max(dz_box) - thickness,
            axis: axis.clone(),
            normal_sign: 1.0,
            inward: true,
            is_flipped: new_flipped,
            offset: 0.0,
            reference_body: "Kubatura".to_string(),
            reference_point: [0.0, 0.0, 0.0],
        });
    }

    let _ = refresh_cutting_list(state, host);

    Ok(format!(
        "Flipped panel '{panel_name}' ({axis} axis, offset {:.1} mm).",
        shift
    ))
}

/// Binds a single panel body into a component `<panel>_K` attached via `joint.as_built` (`rigid`) to the parent cabinet component.
pub fn bind_single_panel_joint(panel_name: &str, host: &mut dyn WoodHost) -> Result<(), String> {
    // 1. Sprawdź listę istniejących komponentów w dokumencie
    let list_val = host.execute("component.list", json!({})).unwrap_or(Value::Null);
    let comps = list_val.get("components").and_then(Value::as_array).cloned().unwrap_or_default();

    // Jeśli bryła już znajduje się w dedykowanym komponencie szuflady (*_Drawer_* / *_Szuflada_*), nie przenoś jej do osobnego komponentu _K
    let already_in_drawer_comp = comps.iter().any(|c| {
        let cname = c.get("name").and_then(Value::as_str).unwrap_or("");
        let is_drawer = cname.contains("_Drawer_")
            || cname.starts_with("Drawer_")
            || cname.contains("_Szuflada_")
            || cname.starts_with("Szuflada_");
        is_drawer
            && c.get("bodies")
                .and_then(Value::as_array)
                .is_some_and(|arr| arr.iter().any(|b| b.as_str() == Some(panel_name)))
    });
    if already_in_drawer_comp {
        let _ = host.execute("component.activate", json!({"component": "root"}));
        return Ok(());
    }
    
    // Ustal czy istnieje komponent "Kubatura"
    let kub_comp_exists = comps.iter().any(|c| {
        c.get("name").and_then(Value::as_str).is_some_and(|n| n == "Kubatura" || n.starts_with("Kubatura:"))
    });

    if !kub_comp_exists {
        let _ = host.execute(
            "component.create",
            json!({
                "name": "Kubatura",
                "parent": "root",
                "activate": false,
                "grounded": true
            }),
        );
    }

    let comp_name = format!("{panel_name}_K");
    let has_comp = comps.iter().any(|c| {
        c.get("name").and_then(Value::as_str).is_some_and(|n| n == comp_name || n.starts_with(&format!("{comp_name}:")))
    });

    if !has_comp {
        // Tworzymy komponent formatki bezpośrednio pod rodzicem "Kubatura" (bez aktywacji, aby nie zagnieżdżać)
        let _ = host.execute(
            "component.create",
            json!({
                "name": comp_name,
                "parent": "Kubatura",
                "activate": false,
                "grounded": false
            }),
        );
        // Przenosimy bryłę formatki do jej komponentu
        let _ = host.execute(
            "component.move_bodies",
            json!({
                "bodies": [panel_name],
                "component": comp_name
            }),
        );
        // Wiążemy komponent as-built rigid jointem do Kubatury ("a" i "b")
        let _ = host.execute(
            "joint.as_built",
            json!({
                "name": format!("Joint_{panel_name}"),
                "a": "Kubatura:1",
                "b": format!("{comp_name}:1"),
                "type": "rigid",
            }),
        );
        // Nadajemy metadane BOM
        let _ = host.execute(
            "component.metadata.set",
            json!({
                "component": comp_name,
                "values": {
                    "description": format!("Formatka meblowa ({panel_name})"),
                    "part_number": panel_name,
                }
            }),
        );
    } else {
        let _ = host.execute(
            "component.move_bodies",
            json!({"bodies": [panel_name], "component": comp_name}),
        );
    }

    // ZAWSZE przywracamy aktywny komponent na "root", aby kolejne operacje CAD nie tworzyły się wewnątrz podkomponentów
    let _ = host.execute("component.activate", json!({"component": "root"}));
    Ok(())
}

/// Binds all furniture panels in the current document with rigid joints into `Korpus_Baza`.
pub fn bind_all_joints(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let doc_info = host.inspect_document(false)?;
    let all_bodies: Vec<String> = doc_info
        .get("bodies")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|b| b.get("name").and_then(Value::as_str))
                .filter(|&name| !name.is_empty() && name != "Kubatura")
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut panels = state.created_panels.clone();
    for b in all_bodies {
        if !panels.contains(&b) {
            panels.push(b);
        }
    }
    if panels.is_empty() {
        return Err("No panels in document.".into());
    }
    for p in &panels {
        let _ = bind_single_panel_joint(p, host);
    }
    state.created_panels = panels.clone();
    Ok(format!("Bound {} panels with rigid joints.", panels.len()))
}



/// Extracts a sorted cutting list (`Tabela Rozkroju`) from the model's 3D bodies (excluding `Kubatura`).
/// Robustly scans `inspect.measure`, all components (including `Kubatura`), and `document.inspect`,
/// accurately detecting bodies created by sketch extrusions and free/auto panels.
pub fn refresh_cutting_list(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let mut detected_boxes: std::collections::BTreeMap<String, (f64, f64, f64)> = std::collections::BTreeMap::new();

    // 1. Sprawdź inspect.measure (całe world_state: wszystkie bryły, w tym wytłoczone szkice i komponenty)
    if let Ok(meas) = host.execute("inspect.measure", json!({})) {
        if let Some(bodies_arr) = meas.get("bodies").and_then(Value::as_array) {
            for b in bodies_arr {
                let name = b.get("name").and_then(Value::as_str).unwrap_or("");
                if name.is_empty() || name == "Kubatura" {
                    continue;
                }
                if let Some(bb) = b.get("bbox") {
                    let min = bb.get("min").and_then(Value::as_array);
                    let max = bb.get("max").and_then(Value::as_array);
                    if let (Some(mn), Some(mx)) = (min, max)
                        && let (Some(x0), Some(y0), Some(z0), Some(x1), Some(y1), Some(z1)) = (
                            mn.first().and_then(Value::as_f64),
                            mn.get(1).and_then(Value::as_f64),
                            mn.get(2).and_then(Value::as_f64),
                            mx.first().and_then(Value::as_f64),
                            mx.get(1).and_then(Value::as_f64),
                            mx.get(2).and_then(Value::as_f64),
                        )
                    {
                        detected_boxes.insert(name.to_string(), ((x1 - x0).abs(), (y1 - y0).abs(), (z1 - z0).abs()));
                    }
                }
            }
        }
    }

    // 2. Sprawdź component.list (odczytuje wszystkie nazwy komponentów i zawarte w nich bryły, np. w komponencie Kubatura)
    if let Ok(comp_val) = host.execute("component.list", json!({})) {
        if let Some(comps) = comp_val.get("components").and_then(Value::as_array) {
            for c in comps {
                if let Some(cbodies) = c.get("bodies").and_then(Value::as_array) {
                    for b_val in cbodies {
                        let name = b_val.as_str().unwrap_or("");
                        if name.is_empty() || name == "Kubatura" || detected_boxes.contains_key(name) {
                            continue;
                        }
                        if let Ok(single_meas) = host.execute("inspect.measure", json!({ "bodies": [name] }))
                            && let Some(sb) = single_meas.get("bodies").and_then(Value::as_array).and_then(|a| a.first())
                            && let Some(bb) = sb.get("bbox")
                            && let (Some(mn), Some(mx)) = (bb.get("min").and_then(Value::as_array), bb.get("max").and_then(Value::as_array))
                            && let (Some(x0), Some(y0), Some(z0), Some(x1), Some(y1), Some(z1)) = (
                                mn.first().and_then(Value::as_f64),
                                mn.get(1).and_then(Value::as_f64),
                                mn.get(2).and_then(Value::as_f64),
                                mx.first().and_then(Value::as_f64),
                                mx.get(1).and_then(Value::as_f64),
                                mx.get(2).and_then(Value::as_f64),
                            )
                        {
                            detected_boxes.insert(name.to_string(), ((x1 - x0).abs(), (y1 - y0).abs(), (z1 - z0).abs()));
                        } else {
                            detected_boxes.insert(name.to_string(), (0.0, 0.0, 0.0));
                        }
                    }
                }
            }
        }
    }

    // 3. Sprawdź inspect_document
    if let Ok(doc_info) = host.inspect_document(true) {
        if let Some(bodies) = doc_info.get("bodies").and_then(Value::as_array) {
            for b in bodies {
                let name = b.get("name").and_then(Value::as_str).unwrap_or("");
                if name.is_empty() || name == "Kubatura" || detected_boxes.contains_key(name) {
                    continue;
                }
                let bbox = b.get("bbox").or_else(|| b.get("bounds"));
                if let Some(bb) = bbox {
                    let min = bb.get("min").and_then(Value::as_array);
                    let max = bb.get("max").and_then(Value::as_array);
                    if let (Some(mn), Some(mx)) = (min, max)
                        && let (Some(x0), Some(y0), Some(z0), Some(x1), Some(y1), Some(z1)) = (
                            mn.first().and_then(Value::as_f64),
                            mn.get(1).and_then(Value::as_f64),
                            mn.get(2).and_then(Value::as_f64),
                            mx.first().and_then(Value::as_f64),
                            mx.get(1).and_then(Value::as_f64),
                            mx.get(2).and_then(Value::as_f64),
                        )
                    {
                        detected_boxes.insert(name.to_string(), ((x1 - x0).abs(), (y1 - y0).abs(), (z1 - z0).abs()));
                    } else {
                        detected_boxes.insert(name.to_string(), (0.0, 0.0, 0.0));
                    }
                } else {
                    detected_boxes.insert(name.to_string(), (0.0, 0.0, 0.0));
                }
            }
        }
    }

    // 4. Fallback: dla brył bez pobranego bbox, pobierz centroidy z model.faces
    for (name, dims) in detected_boxes.iter_mut() {
        if *dims == (0.0, 0.0, 0.0) {
            if let Ok(faces_val) = host.execute("model.faces", json!({ "body": name }))
                && let Some(faces) = faces_val.get("faces").and_then(Value::as_array)
            {
                let mut min_pt = [f64::INFINITY; 3];
                let mut max_pt = [f64::NEG_INFINITY; 3];
                for f in faces {
                    if let Some(c) = f.get("centroid").or_else(|| f.get("point")).and_then(Value::as_array)
                        && let (Some(x), Some(y), Some(z)) = (
                            c.first().and_then(Value::as_f64),
                            c.get(1).and_then(Value::as_f64),
                            c.get(2).and_then(Value::as_f64),
                        )
                    {
                        min_pt[0] = min_pt[0].min(x);
                        min_pt[1] = min_pt[1].min(y);
                        min_pt[2] = min_pt[2].min(z);
                        max_pt[0] = max_pt[0].max(x);
                        max_pt[1] = max_pt[1].max(y);
                        max_pt[2] = max_pt[2].max(z);
                    }
                }
                if min_pt[0].is_finite() {
                    *dims = (
                        (max_pt[0] - min_pt[0]).abs(),
                        (max_pt[1] - min_pt[1]).abs(),
                        (max_pt[2] - min_pt[2]).abs(),
                    );
                }
            }
        }
    }

    // Wczytaj zapisane obrzeża i dekory z metadanych dokumentu (jeśli w sesji słownik jest pusty)
    if state.panel_edgebands.is_empty() {
        load_edgebands_from_metadata(state, host);
    }
    if state.panel_decors.is_empty() {
        load_decors_from_metadata(state, host);
    }

    // Zachowaj ewentualne niezapisane edycje użytkownika z poprzedniego stanu tabeli
    let prev_edits: std::collections::HashMap<String, (String, String)> = state
        .cutting_list
        .iter()
        .map(|r| (r.name.clone(), (r.cabinet_edit.clone(), r.custom_name_edit.clone())))
        .collect();

    // 5. Zbuduj wiersze tabeli rozkroju
    let mut rows = Vec::new();
    for (name, (dx, dy, dz)) in detected_boxes {
        let mut d = [dx, dy, dz];
        d.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let length_mm = round1(d[0]);
        let width_mm = round1(d[1]);
        let thickness_mm = round1(d[2]);
        let area_m2 = ((length_mm * width_mm) / 1_000_000.0 * 1000.0).round() / 1000.0;
        let (edgeband_code, edgeband_length_mm) =
            compute_edgeband_code_and_length(dx, dy, dz, state.panel_edgebands.get(&name));
        let (decor_name, grain_label) = decor_bom_fields(state.panel_decors.get(&name));

        let parsed = parse_panel_name(&name);
        let panel_number = if parsed.is_duplicate_copy {
            0
        } else {
            parsed.number.unwrap_or(0)
        };
        if panel_number > 0 {
            state.highest_panel_number = state.highest_panel_number.max(panel_number);
        }

        let (cabinet_edit, custom_name_edit) = if let Some((prev_cab, prev_cust)) = prev_edits.get(&name) {
            (prev_cab.clone(), prev_cust.clone())
        } else {
            let cab = if parsed.cabinet.is_empty() {
                state.cabinet_prefix.clone()
            } else {
                parsed.cabinet
            };
            let cust = if parsed.number.is_some() && !parsed.is_duplicate_copy {
                parsed.custom
            } else {
                String::new()
            };
            (cab, cust)
        };

        rows.push(CuttingRow {
            name,
            panel_number,
            cabinet_edit,
            custom_name_edit,
            length_mm,
            width_mm,
            thickness_mm,
            quantity: 1,
            area_m2,
            edgeband_code,
            edgeband_length_mm,
            decor_name,
            grain_label,
        });
    }

    rows.sort_by(|a, b| {
        let ka = if a.panel_number == 0 { usize::MAX } else { a.panel_number };
        let kb = if b.panel_number == 0 { usize::MAX } else { b.panel_number };
        ka.cmp(&kb).then_with(|| a.name.cmp(&b.name))
    });

    state.cutting_list = rows;
    Ok(format!("Refreshed cutting list ({} items).", state.cutting_list.len()))
}

/// Ustawia oznaczenie szafki (np. `S02`) w polach edycji tabeli dla wierszy o numerach formatek z podanego zakresu `[from..=to]`.
pub fn apply_bom_range_cabinet(state: &mut WoodState) -> Result<String, String> {
    let from_nr = state
        .bom_range_from
        .trim()
        .parse::<usize>()
        .map_err(|_| "Enter a valid start panel number (e.g. 7).".to_string())?;
    let to_nr = state
        .bom_range_to
        .trim()
        .parse::<usize>()
        .map_err(|_| "Enter a valid end panel number (e.g. 11).".to_string())?;
    let lo = from_nr.min(to_nr);
    let hi = from_nr.max(to_nr);
    let cab = sanitize_name_segment(&state.bom_range_cabinet);
    if cab.is_empty() {
        return Err("Podaj oznaczenie szafki (np. S02).".into());
    }

    let mut matched = 0usize;
    for row in &mut state.cutting_list {
        if row.panel_number >= lo && row.panel_number <= hi {
            row.cabinet_edit = cab.clone();
            matched += 1;
        }
    }
    if matched == 0 {
        return Err(format!("No panels with numbers in range {lo}–{hi} found in the table."));
    }
    Ok(format!(
        "Assigned cabinet '{cab}' to {matched} panels (numbers {lo}–{hi}). Click 'Apply Names to Model' to save to the tree."
    ))
}

/// Przelicza i zatwierdza w modelu 3D (drzewku SolidCraft) nazwy formatek (`<Szafka>_<Nr>_<NazwaWłasna>`)
/// na podstawie pól edytowanych w Tabeli Rozkroju. Wywoływane wyłącznie przyciskiem.
pub fn apply_bom_renames(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    if state.cutting_list.is_empty() {
        return Err("Tabela rozkroju jest pusta.".into());
    }

    let mut renamed_count = 0usize;
    let mut errors = Vec::new();
    let row_snapshots = state.cutting_list.clone();

    for row in row_snapshots {
        let num = if row.panel_number > 0 {
            state.highest_panel_number = state.highest_panel_number.max(row.panel_number);
            row.panel_number
        } else {
            allocate_panel_number(state, host)
        };

        let target_name = build_panel_name(&row.cabinet_edit, num, &row.custom_name_edit);
        if target_name == row.name {
            continue;
        }

        match host.execute("body.rename", json!({ "body": row.name, "name": target_name })) {
            Ok(_) => {
                // Jeśli istniał komponent `<stara_nazwa>_K` i nie był współdzielony przez inną bryłę, przemianuj go
                let old_comp = format!("{}_K", row.name);
                let new_comp = format!("{target_name}_K");
                let rename_comp_res = host.execute(
                    "component.rename",
                    json!({
                        "component": old_comp,
                        "name": new_comp
                    }),
                );
                if rename_comp_res.is_ok() {
                    let _ = host.execute(
                        "component.metadata.set",
                        json!({
                            "component": new_comp,
                            "values": {
                                "description": format!("Formatka meblowa ({target_name})"),
                                "part_number": target_name,
                            }
                        }),
                    );
                } else {
                    let _ = bind_single_panel_joint(&target_name, host);
                }

                if let Some(pdata) = state.panels.remove(&row.name) {
                    let mut updated = pdata;
                    updated.name = target_name.clone();
                    state.panels.insert(target_name.clone(), updated);
                }
                if let Some(eb) = state.panel_edgebands.remove(&row.name) {
                    state.panel_edgebands.insert(target_name.clone(), eb);
                }
                if let Some(eb) = state.staged_edgebands.remove(&row.name) {
                    state.staged_edgebands.insert(target_name.clone(), eb);
                }
                if let Some(dec) = state.panel_decors.remove(&row.name) {
                    state.panel_decors.insert(target_name.clone(), dec);
                }
                for cp in &mut state.created_panels {
                    if *cp == row.name {
                        *cp = target_name.clone();
                    }
                }
                renamed_count += 1;
            }
            Err(e) => {
                errors.push(format!("{} -> {}: {}", row.name, target_name, e));
            }
        }
    }

    if renamed_count > 0 {
        save_edgebands_to_metadata(state, host);
        save_decors_to_metadata(state, host);
    }

    let _ = refresh_cutting_list(state, host);

    if !errors.is_empty() && renamed_count == 0 {
        return Err(format!("Rename error: {}", errors.join("; ")));
    }
    let mut msg = format!("Recalculated and updated names for {renamed_count} panels in the model tree.");
    if !errors.is_empty() {
        msg.push_str(&format!(" (Skipped/errors: {})", errors.join(", ")));
    }
    Ok(msg)
}

/// Skanuje całe drzewko modelu w poszukiwaniu brył:
/// 1. nieprzypisanych do komponentów (leżących luzem w `root`),
/// 2. leżących poza komponentem `Kubatura`,
/// 3. będących drugą/kolejną bryłą w istniejącym komponencie (np. po `Mirror` lub `Split`),
/// 4. niemających nadanego numeru formatki.
pub fn scan_adapt_candidates(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let prev_map: std::collections::HashMap<String, (bool, String, String)> = state
        .adapt_candidates
        .iter()
        .map(|c| (c.body_name.clone(), (c.selected, c.cabinet_prefix.clone(), c.custom_name.clone())))
        .collect();

    let boxes = extract_panel_boxes(host);
    let comp_val = host.execute("component.list", json!({})).unwrap_or(Value::Null);
    let comps = comp_val.get("components").and_then(Value::as_array).cloned().unwrap_or_default();

    let kub_id = comps
        .iter()
        .find(|c| {
            c.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n == "Kubatura" || n.starts_with("Kubatura:"))
        })
        .and_then(|c| c.get("id").and_then(Value::as_u64));

    // Mapowanie: body_name -> (comp_name, is_root_or_kub, is_child_of_kub, index_in_comp, total_in_comp)
    let mut body_comp_info: std::collections::HashMap<String, (String, bool, bool, usize, usize)> =
        std::collections::HashMap::new();

    for c in &comps {
        let cid = c.get("id").and_then(Value::as_u64);
        let raw_cname = c.get("name").and_then(Value::as_str).unwrap_or("");
        let cname = raw_cname.split(':').next().unwrap_or(raw_cname).trim().to_string();
        let parent_id = c.get("parent").and_then(Value::as_u64);
        let parent_str = c.get("parent").and_then(Value::as_str).unwrap_or("");

        let is_root_or_kub = cid == Some(0)
            || cname.is_empty()
            || cname.eq_ignore_ascii_case("root")
            || cname == "Kubatura";
        let is_drawer_comp = cname.contains("_Drawer_")
            || cname.starts_with("Drawer_")
            || cname.contains("_Szuflada_")
            || cname.starts_with("Szuflada_");
        let is_child_of_kub = !is_root_or_kub
            && ((kub_id.is_some() && parent_id == kub_id)
                || parent_str == "Kubatura"
                || parent_str.starts_with("Kubatura:")
                || cname.ends_with("_K")
                || is_drawer_comp);

        if let Some(cbodies) = c.get("bodies").and_then(Value::as_array) {
            let total = cbodies.len();
            for (idx, bval) in cbodies.iter().enumerate() {
                if let Some(bname) = bval.as_str()
                    && !bname.is_empty()
                    && bname != "Kubatura"
                {
                    // Prefer dedicated child component record if a body appears in multiple entries
                    let should_insert = match body_comp_info.get(bname) {
                        None => true,
                        Some((_, prev_root, _, _, _)) => *prev_root && !is_root_or_kub,
                    };
                    if should_insert {
                        body_comp_info.insert(
                            bname.to_string(),
                            (cname.clone(), is_root_or_kub, is_child_of_kub, idx, total),
                        );
                    }
                }
            }
        }
    }

    let mut candidates = Vec::new();
    for pb in &boxes {
        if pb.name == "Kubatura" {
            continue;
        }
        let mut ext = pb.extents();
        ext.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let dims_mm = [round1(ext[2]), round1(ext[1]), round1(ext[0])]; // [Grubość, Szerokość, Długość]

        let parsed = parse_panel_name(&pb.name);
        let needs_number = parsed.number.is_none() || parsed.is_duplicate_copy;

        let (current_comp, reason_opt) = match body_comp_info.get(&pb.name) {
            None => (
                "root (loose)".to_string(),
                Some("Body unassigned to a component".to_string()),
            ),
            Some((cname, is_root_or_kub, is_child_of_kub, idx_in_comp, total_in_comp)) => {
                if *is_root_or_kub {
                    (
                        cname.clone(),
                        Some("Loose body outside a panel component".to_string()),
                    )
                } else if *total_in_comp > 1
                    && !(cname.contains("_Drawer_")
                        || cname.starts_with("Drawer_")
                        || cname.contains("_Szuflada_")
                        || cname.starts_with("Szuflada_"))
                    && (*idx_in_comp > 0 || parsed.is_duplicate_copy)
                {
                    (
                        cname.clone(),
                        Some(format!("Second body in component '{cname}' (after mirror/split)")),
                    )
                } else if !*is_child_of_kub {
                    (
                        cname.clone(),
                        Some(format!("Outside bounding box component (in '{cname}')")),
                    )
                } else if needs_number {
                    (
                        cname.clone(),
                        Some("Missing panel number in body name".to_string()),
                    )
                } else {
                    (cname.clone(), None)
                }
            }
        };

        if let Some(reason) = reason_opt {
            let (selected, cabinet_prefix, custom_name) =
                if let Some((sel, cab, cust)) = prev_map.get(&pb.name) {
                    (*sel, cab.clone(), cust.clone())
                } else {
                    (true, state.cabinet_prefix.clone(), String::new())
                };
            candidates.push(crate::AdaptCandidate {
                body_name: pb.name.clone(),
                current_component: current_comp,
                reason,
                dims_mm,
                selected,
                cabinet_prefix,
                custom_name,
            });
        }
    }

    let count = candidates.len();
    state.adapt_candidates = candidates;
    Ok(if count == 0 {
        "All bodies in the model are already nested as panels in the bounding box. You can also click any body in the tree.".to_string()
    } else {
        format!("Detected {count} bodies to adopt as panels.")
    })
}

/// Przechwytuje bryły kliknięte przez użytkownika w drzewku (Browser) lub w oknie 3D,
/// gdy otwarte jest okno `Adaptuj Bryły`, i dodaje/zaznacza je na liście do adaptacji.
pub fn consume_adapt_picks(state: &mut WoodState, host: &mut dyn WoodHost) {
    let mut picked_bodies = Vec::new();

    if let Ok(doc_info) = host.inspect_document(false)
        && let Some(sel_arr) = doc_info.get("selection").and_then(Value::as_array)
    {
        for item in sel_arr {
            let stype = item.get("type").and_then(Value::as_str).unwrap_or("");
            let bname = match stype {
                "body" => item.get("name").and_then(Value::as_str),
                "face" | "edge" => item.get("body").and_then(Value::as_str),
                _ => None,
            };
            if let Some(b) = bname
                && !b.is_empty()
                && b != "Kubatura"
                && !picked_bodies.contains(&b.to_string())
            {
                picked_bodies.push(b.to_string());
            }
        }
    }

    for fp in host.take_selected_faces() {
        if fp.body != "Kubatura" && !picked_bodies.contains(&fp.body) {
            picked_bodies.push(fp.body);
        }
    }
    for ep in host.take_selected_edges() {
        if ep.body != "Kubatura" && !picked_bodies.contains(&ep.body) {
            picked_bodies.push(ep.body);
        }
    }

    if picked_bodies.is_empty() {
        return;
    }

    let _ = host.execute("select.clear", json!({}));
    let boxes = extract_panel_boxes(host);

    for bname in picked_bodies {
        if let Some(existing) = state.adapt_candidates.iter_mut().find(|c| c.body_name == bname) {
            existing.selected = true;
        } else {
            let dims_mm = boxes
                .iter()
                .find(|b| b.name == bname)
                .map(|pb| {
                    let mut ext = pb.extents();
                    ext.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
                    [round1(ext[2]), round1(ext[1]), round1(ext[0])]
                })
                .unwrap_or([18.0, 0.0, 0.0]);
            state.adapt_candidates.push(crate::AdaptCandidate {
                body_name: bname,
                current_component: "Wskazano z drzewka/3D".to_string(),
                reason: "Manually picked by user".to_string(),
                dims_mm,
                selected: true,
                cabinet_prefix: state.cabinet_prefix.clone(),
                custom_name: String::new(),
            });
        }
    }
}

/// Zagnieżdża zaznaczone na liście bryły jako pełnoprawne formatki w komponencie `Kubatura`,
/// nadając im unikalne numery i nazwy w układzie `S01_05` / `S01_05_nazwa`.
pub fn adapt_selected_bodies(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let selected: Vec<crate::AdaptCandidate> = state
        .adapt_candidates
        .iter()
        .filter(|c| c.selected)
        .cloned()
        .collect();

    if selected.is_empty() {
        return Err("Select at least one body from the list (or click a body in the tree).".into());
    }

    let mut adapted_names = Vec::new();
    let mut errors = Vec::new();

    for cand in selected {
        let parsed = parse_panel_name(&cand.body_name);
        let num = if let Some(n) = parsed.number
            && !parsed.is_duplicate_copy
        {
            state.highest_panel_number = state.highest_panel_number.max(n);
            n
        } else {
            allocate_panel_number(state, host)
        };

        let cab = if cand.cabinet_prefix.trim().is_empty() {
            state.cabinet_prefix.clone()
        } else {
            cand.cabinet_prefix.trim().to_string()
        };
        let cust = if !cand.custom_name.trim().is_empty() {
            cand.custom_name.trim().to_string()
        } else if parsed.number.is_some() && !parsed.is_duplicate_copy {
            parsed.custom
        } else {
            String::new()
        };

        let new_name = build_panel_name(&cab, num, &cust);
        if new_name != cand.body_name {
            if let Err(e) = host.execute("body.rename", json!({ "body": cand.body_name, "name": new_name })) {
                errors.push(format!("{}: {}", cand.body_name, e));
                continue;
            }
        }

        if let Err(e) = bind_single_panel_joint(&new_name, host) {
            errors.push(format!("{new_name}: {e}"));
            continue;
        }

        if !state.created_panels.contains(&new_name) {
            state.created_panels.push(new_name.clone());
        }
        adapted_names.push(new_name);
    }

    let _ = scan_adapt_candidates(state, host);
    let _ = refresh_cutting_list(state, host);

    if adapted_names.is_empty() && !errors.is_empty() {
        return Err(format!("Body adaptation error: {}", errors.join("; ")));
    }

    let mut msg = format!(
        "Adopted {} bodies as panels in the bounding box ({}).",
        adapted_names.len(),
        adapted_names.join(", ")
    );
    if !errors.is_empty() {
        msg.push_str(&format!(" Errors: {}", errors.join(", ")));
    }
    Ok(msg)
}

/// Eksportuje każdą formatkę z tabeli rozkroju do osobnego pliku STEP oraz zapisuje plik CSV z tabelą rozkroju w tym samym folderze.
/// Nazwa pliku STEP nie zawiera numeracji Lp z tabeli BOM i zaczyna się od kodu formatki (`S05_30_...`), np. `S05_30_wieniec_18x560x764.step`.
pub fn export_panels_to_step(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    if state.cutting_list.is_empty() {
        let _ = refresh_cutting_list(state, host);
    }
    if state.cutting_list.is_empty() {
        return Err("Brak formatek w tabeli rozkroju do wyeksportowania.".into());
    }

    let raw_dir = state.export_step_dir.trim();
    let export_dir = if raw_dir.is_empty() {
        std::path::PathBuf::from("C:/SolidCraft_Export/STEP")
    } else {
        std::path::PathBuf::from(raw_dir)
    };

    std::fs::create_dir_all(&export_dir)
        .map_err(|e| format!("Cannot create target folder '{}': {e}", export_dir.display()))?;

    let sanitize = |name: &str| -> String {
        name.chars()
            .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ' | '\t') { '_' } else { c })
            .collect()
    };

    let mut exported_count = 0;
    let mut errors = Vec::new();

    let hid_kubatura_temp = !state.kubatura_hidden;
    if hid_kubatura_temp {
        let _ = host.execute(
            "browser.visibility",
            serde_json::json!({ "items": ["Kubatura"], "visible": false }),
        );
    }

    for row in &state.cutting_list {
        let clean_name = sanitize(&row.name);
        let file_name = format!(
            "{}_{:.0}x{:.0}x{:.0}.step",
            clean_name,
            row.thickness_mm,
            row.width_mm,
            row.length_mm
        );
        let target_path = export_dir.join(&file_name);
        let target_str = target_path.to_string_lossy().to_string();

        let res = host.execute(
            "file.export_step",
            serde_json::json!({
                "path": target_str,
                "bodies": [row.name.clone()]
            }),
        );

        match res {
            Ok(_) => {
                exported_count += 1;
            }
            Err(e) => {
                errors.push(format!("{}: {}", row.name, e));
            }
        }
    }

    if hid_kubatura_temp {
        let _ = host.execute(
            "browser.visibility",
            serde_json::json!({ "items": ["Kubatura"], "visible": true }),
        );
    }

    let doc_stem = host
        .inspect_document(false)
        .ok()
        .and_then(|info| {
            info.get("path")
                .and_then(Value::as_str)
                .and_then(|p| std::path::Path::new(p).file_stem())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .or_else(|| {
                    info.get("name")
                        .and_then(Value::as_str)
                        .filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("Untitled"))
                        .map(str::to_string)
                })
        })
        .map(|s| sanitize(&s))
        .filter(|s| !s.is_empty());

    let csv_file_name = match doc_stem {
        Some(stem) => format!("{stem}_rozkroj.csv"),
        None => "Tabela_Rozkroju.csv".to_string(),
    };
    let csv_path = export_dir.join(&csv_file_name);
    let csv_with_bom = format!("\u{FEFF}{}", cutting_list_to_csv(state));
    let mut csv_saved = false;
    match std::fs::write(&csv_path, csv_with_bom.as_bytes()) {
        Ok(()) => csv_saved = true,
        Err(e) => errors.push(format!("{csv_file_name}: {e}")),
    }

    if exported_count == 0 && !csv_saved && !errors.is_empty() {
        return Err(format!("STEP/CSV export error: {}", errors.join("; ")));
    }

    let mut msg = format!(
        "Exported {}/{} STEP panels and {} in folder:\n{}",
        exported_count,
        state.cutting_list.len(),
        if csv_saved { csv_file_name.as_str() } else { "(CSV write error)" },
        export_dir.display()
    );
    if !errors.is_empty() {
        msg.push_str(&format!("\n(Warnings: {})", errors.join(", ")));
    }
    Ok(msg)
}

/// Formats the current cutting list (`Tabela Rozkroju`) as CSV text.
pub fn cutting_list_to_csv(state: &WoodState) -> String {
    let mut out = String::from("No;Cabinet;PanelNo;CustomName;PanelCode;Length [mm];Width [mm];Thickness [mm];Edgeband;Decor;Grain;Qty [pcs];Area [m2]\n");
    for (idx, r) in state.cutting_list.iter().enumerate() {
        out.push_str(&format!(
            "{};{};{:02};{};{};{:.1};{:.1};{:.1};{};{};{};{};{:.3}\n",
            idx + 1,
            r.cabinet_edit,
            r.panel_number,
            r.custom_name_edit,
            r.name,
            r.length_mm,
            r.width_mm,
            r.thickness_mm,
            r.edgeband_code,
            r.decor_name,
            r.grain_label,
            r.quantity,
            r.area_m2
        ));
    }
    out
}

/// Axis-aligned bounding box of a 3D panel body used to resolve connector contact geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelBox {
    pub name: String,
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl PanelBox {
    pub fn extents(&self) -> [f64; 3] {
        [
            (self.max[0] - self.min[0]).abs(),
            (self.max[1] - self.min[1]).abs(),
            (self.max[2] - self.min[2]).abs(),
        ]
    }

    /// Returns the index (0=X, 1=Y, 2=Z) of the smallest dimension (panel thickness).
    pub fn thin_axis(&self) -> usize {
        let ext = self.extents();
        if ext[0] <= ext[1] && ext[0] <= ext[2] {
            0
        } else if ext[1] <= ext[0] && ext[1] <= ext[2] {
            1
        } else {
            2
        }
    }

    pub fn center(&self) -> [f64; 3] {
        [
            0.5 * (self.min[0] + self.max[0]),
            0.5 * (self.min[1] + self.max[1]),
            0.5 * (self.min[2] + self.max[2]),
        ]
    }

    /// Distance from a 3D point `p` to this axis-aligned bounding box (0.0 if inside or on surface).
    pub fn dist_to_point(&self, p: [f64; 3]) -> f64 {
        let mut d2 = 0.0;
        for a in 0..3 {
            if p[a] < self.min[a] {
                let d = self.min[a] - p[a];
                d2 += d * d;
            } else if p[a] > self.max[a] {
                let d = p[a] - self.max[a];
                d2 += d * d;
            }
        }
        d2.sqrt()
    }
}

/// Computed geometric drilling plan for a connector line (`Złącza`: Kołki / Wkręty).
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectorPlan {
    /// Name of the panel drilled into its narrow edge (`w brzegu/czole`, e.g. `Wieniec_D` or `Trawers`).
    pub edge_panel: String,
    /// Name of the panel drilled perpendicularly through its flat face (`w ścianie`, e.g. `Bok_L`).
    pub face_panel: String,
    /// Position of the first hole `[x, y, z]` on the contact plane, centered across `edge_panel` thickness.
    pub first_hole_pos: [f64; 3],
    /// Unit vector pointing into `edge_panel` (for the edge hole).
    pub dir_into_edge: [f64; 3],
    /// Unit vector pointing into `face_panel` (for the perpendicular face hole).
    pub dir_into_face: [f64; 3],
    /// Unit vector along the contact edge for `solid.pattern.rectangular`.
    pub pattern_dir: [f64; 3],
    /// Distance between consecutive holes in the linear pattern.
    pub spacing: f64,
    /// Number of holes in the linear pattern.
    pub count: u32,
    /// Effective contact length along which the connectors are distributed.
    pub contact_length: f64,
    /// Thickness of `face_panel` along `drill_axis` (for through-hole drilling depth).
    pub face_thickness: f64,
}

/// Extracts `PanelBox` bounding boxes for all non-Kubatura bodies across the entire model.
pub fn extract_panel_boxes(host: &mut dyn WoodHost) -> Vec<PanelBox> {
    let mut map: std::collections::BTreeMap<String, ([f64; 3], [f64; 3])> = std::collections::BTreeMap::new();

    // 1. inspect.measure (całe world_state)
    if let Ok(meas) = host.execute("inspect.measure", json!({})) {
        if let Some(bodies_arr) = meas.get("bodies").and_then(Value::as_array) {
            for b in bodies_arr {
                let name = b.get("name").and_then(Value::as_str).unwrap_or("");
                if name.is_empty() || name == "Kubatura" {
                    continue;
                }
                if let Some(bb) = b.get("bbox") {
                    let min = bb.get("min").and_then(Value::as_array);
                    let max = bb.get("max").and_then(Value::as_array);
                    if let (Some(mn), Some(mx)) = (min, max)
                        && let (Some(x0), Some(y0), Some(z0), Some(x1), Some(y1), Some(z1)) = (
                            mn.first().and_then(Value::as_f64),
                            mn.get(1).and_then(Value::as_f64),
                            mn.get(2).and_then(Value::as_f64),
                            mx.first().and_then(Value::as_f64),
                            mx.get(1).and_then(Value::as_f64),
                            mx.get(2).and_then(Value::as_f64),
                        )
                    {
                        map.insert(name.to_string(), ([x0.min(x1), y0.min(y1), z0.min(z1)], [x0.max(x1), y0.max(y1), z0.max(z1)]));
                    }
                }
            }
        }
    }

    // 2. component.list (komponenty w tym 'Kubatura')
    if let Ok(comp_val) = host.execute("component.list", json!({})) {
        if let Some(comps) = comp_val.get("components").and_then(Value::as_array) {
            for c in comps {
                if let Some(cbodies) = c.get("bodies").and_then(Value::as_array) {
                    for b_val in cbodies {
                        let name = b_val.as_str().unwrap_or("");
                        if name.is_empty() || name == "Kubatura" || map.contains_key(name) {
                            continue;
                        }
                        if let Ok(single_meas) = host.execute("inspect.measure", json!({ "bodies": [name] }))
                            && let Some(sb) = single_meas.get("bodies").and_then(Value::as_array).and_then(|a| a.first())
                            && let Some(bb) = sb.get("bbox")
                            && let (Some(mn), Some(mx)) = (bb.get("min").and_then(Value::as_array), bb.get("max").and_then(Value::as_array))
                            && let (Some(x0), Some(y0), Some(z0), Some(x1), Some(y1), Some(z1)) = (
                                mn.first().and_then(Value::as_f64),
                                mn.get(1).and_then(Value::as_f64),
                                mn.get(2).and_then(Value::as_f64),
                                mx.first().and_then(Value::as_f64),
                                mx.get(1).and_then(Value::as_f64),
                                mx.get(2).and_then(Value::as_f64),
                            )
                        {
                            map.insert(name.to_string(), ([x0.min(x1), y0.min(y1), z0.min(z1)], [x0.max(x1), y0.max(y1), z0.max(z1)]));
                        }
                    }
                }
            }
        }
    }

    // 3. inspect_document
    if let Ok(doc_info) = host.inspect_document(true) {
        if let Some(bodies) = doc_info.get("bodies").and_then(Value::as_array) {
            for b in bodies {
                let name = b.get("name").and_then(Value::as_str).unwrap_or("");
                if name.is_empty() || name == "Kubatura" || map.contains_key(name) {
                    continue;
                }
                let bbox = b.get("bbox").or_else(|| b.get("bounds"));
                if let Some(bb) = bbox {
                    let min = bb.get("min").and_then(Value::as_array);
                    let max = bb.get("max").and_then(Value::as_array);
                    if let (Some(mn), Some(mx)) = (min, max)
                        && let (Some(x0), Some(y0), Some(z0), Some(x1), Some(y1), Some(z1)) = (
                            mn.first().and_then(Value::as_f64),
                            mn.get(1).and_then(Value::as_f64),
                            mn.get(2).and_then(Value::as_f64),
                            mx.first().and_then(Value::as_f64),
                            mx.get(1).and_then(Value::as_f64),
                            mx.get(2).and_then(Value::as_f64),
                        )
                    {
                        map.insert(name.to_string(), ([x0.min(x1), y0.min(y1), z0.min(z1)], [x0.max(x1), y0.max(y1), z0.max(z1)]));
                    }
                }
            }
        }
    }

    // 4. Fallback model.faces
    for (name, (min_pt, max_pt)) in map.iter_mut() {
        if *min_pt == [0.0, 0.0, 0.0] && *max_pt == [0.0, 0.0, 0.0] {
            if let Ok(faces_val) = host.execute("model.faces", json!({ "body": name }))
                && let Some(faces) = faces_val.get("faces").and_then(Value::as_array)
            {
                let mut mn = [f64::INFINITY; 3];
                let mut mx = [f64::NEG_INFINITY; 3];
                for f in faces {
                    let pt = f.get("centroid").or_else(|| f.get("point")).and_then(Value::as_array);
                    if let Some(arr) = pt && arr.len() == 3 {
                        for i in 0..3 {
                            if let Some(v) = arr[i].as_f64() {
                                mn[i] = mn[i].min(v);
                                mx[i] = mx[i].max(v);
                            }
                        }
                    }
                }
                if mn[0].is_finite() {
                    *min_pt = mn;
                    *max_pt = mx;
                }
            }
        }
    }

    map.into_iter()
        .map(|(name, (min, max))| PanelBox { name, min, max })
        .collect()
}

/// Computes the exact drilling and linear pattern plan from a picked contact edge and target face.
pub fn compute_connector_plan(
    edge: &crate::EdgePick,
    face: Option<&crate::FacePick>,
    boxes: &[PanelBox],
    offset_x: f64,
    shift_offset: f64,
    count: u32,
) -> Result<ConnectorPlan, String> {
    let dv = [
        edge.end[0] - edge.start[0],
        edge.end[1] - edge.start[1],
        edge.end[2] - edge.start[2],
    ];
    let ad = [dv[0].abs(), dv[1].abs(), dv[2].abs()];
    let edge_axis = if ad[0] >= ad[1] && ad[0] >= ad[2] {
        0
    } else if ad[1] >= ad[0] && ad[1] >= ad[2] {
        1
    } else {
        2
    };
    let raw_len = (dv[0] * dv[0] + dv[1] * dv[1] + dv[2] * dv[2]).sqrt();
    if raw_len < 1.0 {
        return Err("Selected edge is too short (< 1 mm).".into());
    }

    let mid = [
        0.5 * (edge.start[0] + edge.end[0]),
        0.5 * (edge.start[1] + edge.end[1]),
        0.5 * (edge.start[2] + edge.end[2]),
    ];

    // Find panels:
    // If the user explicitly provided `face` (Step 2: Ściana do nawierceń prostopadłych):
    let (face_panel, edge_panel) = if let Some(fp) = face {
        // 1. Unambiguously resolve `face_panel` from the picked face
        let f_panel = boxes
            .iter()
            .find(|b| b.name == fp.body)
            .or_else(|| {
                boxes
                    .iter()
                    .filter(|b| b.dist_to_point(fp.point) <= 5.0)
                    .min_by(|a, b| {
                        a.dist_to_point(fp.point)
                            .partial_cmp(&b.dist_to_point(fp.point))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            })
            .ok_or_else(|| format!("Panel not found for selected face '{}'.", fp.body))?;

        // 2. Unambiguously resolve `edge_panel` from the edge (excluding `face_panel`)
        let e_panel = boxes
            .iter()
            .find(|b| b.name != f_panel.name && b.name == edge.body)
            .or_else(|| {
                boxes
                    .iter()
                    .filter(|b| b.name != f_panel.name)
                    .min_by(|a, b| {
                        a.dist_to_point(mid)
                            .partial_cmp(&b.dist_to_point(mid))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            })
            .ok_or_else(|| "Panel with edge not found at the selected edge.".to_string())?;

        (f_panel, e_panel)
    } else {
        // Fallback: 1-pick heuristic
        let primary = boxes
            .iter()
            .find(|b| b.name == edge.body)
            .or_else(|| {
                boxes
                    .iter()
                    .filter(|b| b.dist_to_point(mid) <= 5.0)
                    .min_by(|a, b| {
                        a.dist_to_point(mid)
                            .partial_cmp(&b.dist_to_point(mid))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            })
            .ok_or_else(|| "Panel not found at the selected edge.".to_string())?;

        let secondary = boxes
            .iter()
            .filter(|b| b.name != primary.name)
            .filter(|b| {
                let mut perp_dist2 = 0.0;
                for a in 0..3 {
                    if a == edge_axis {
                        continue;
                    }
                    if mid[a] < b.min[a] {
                        let d = b.min[a] - mid[a];
                        perp_dist2 += d * d;
                    } else if mid[a] > b.max[a] {
                        let d = mid[a] - b.max[a];
                        perp_dist2 += d * d;
                    }
                }
                let e_min = edge.start[edge_axis].min(edge.end[edge_axis]);
                let e_max = edge.start[edge_axis].max(edge.end[edge_axis]);
                let overlap = e_max.min(b.max[edge_axis]) - e_min.max(b.min[edge_axis]);
                perp_dist2.sqrt() <= 5.0 && overlap > 5.0
            })
            .min_by(|a, b| {
                a.dist_to_point(mid)
                    .partial_cmp(&b.dist_to_point(mid))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .ok_or_else(|| {
                format!(
                    "No second panel detected touching '{}' at this edge.",
                    primary.name
                )
            })?;

        let t_pri = primary.thin_axis();
        let t_sec = secondary.thin_axis();
        let c_pri = primary.center();
        let c_sec = secondary.center();

        let pri_is_face = c_sec[t_pri] < primary.min[t_pri] - 0.5 || c_sec[t_pri] > primary.max[t_pri] + 0.5;
        let sec_is_face = c_pri[t_sec] < secondary.min[t_sec] - 0.5 || c_pri[t_sec] > secondary.max[t_sec] + 0.5;

        if pri_is_face && !sec_is_face {
            (primary, secondary)
        } else if sec_is_face && !pri_is_face {
            (secondary, primary)
        } else if primary.extents()[t_pri] <= secondary.extents()[t_sec] {
            (primary, secondary)
        } else {
            (secondary, primary)
        }
    };

    let drill_axis = face_panel.thin_axis();
    // The third axis perpendicular to both `edge_axis` and `drill_axis` is the thickness axis of `edge_panel`.
    let width_axis = (0..3)
        .find(|&a| a != edge_axis && a != drill_axis)
        .unwrap_or_else(|| edge_panel.thin_axis());

    // Contact coordinate along `drill_axis` and drill directions:
    let edge_center_d = edge_panel.center()[drill_axis];
    let face_center_d = face_panel.center()[drill_axis];

    let (contact_d, sign_into_edge) = if edge_center_d >= face_center_d {
        // `edge_panel` is on the +drill_axis side of `face_panel` (e.g. Bok_L at X=0..18, Wieniec_D at X=18..782)
        (edge_panel.min[drill_axis], 1.0_f64)
    } else {
        // `edge_panel` is on the -drill_axis side of `face_panel` (e.g. Wieniec_D at X=18..782, Bok_P at X=782..800)
        (edge_panel.max[drill_axis], -1.0_f64)
    };

    let mut dir_into_edge = [0.0; 3];
    dir_into_edge[drill_axis] = sign_into_edge;

    let mut dir_into_face = [0.0; 3];
    dir_into_face[drill_axis] = -sign_into_edge;

    // Mid-thickness of `edge_panel` along `width_axis`:
    let mid_thickness_w = 0.5 * (edge_panel.min[width_axis] + edge_panel.max[width_axis]);

    // Effective contact interval along `edge_axis`:
    // Clipped to the overlap of the picked edge, `edge_panel`, and `face_panel`!
    // This guarantees that for a 200mm Trawers on a 560mm Bok, the length is ALWAYS 200mm (the Trawers span)!
    let pick_min = edge.start[edge_axis].min(edge.end[edge_axis]);
    let pick_max = edge.start[edge_axis].max(edge.end[edge_axis]);
    let s_min = pick_min
        .max(edge_panel.min[edge_axis])
        .max(face_panel.min[edge_axis]);
    let s_max = pick_max
        .min(edge_panel.max[edge_axis])
        .min(face_panel.max[edge_axis]);

    let (s_min, s_max) = if s_max > s_min + 1.0 {
        (s_min, s_max)
    } else {
        (
            edge_panel.min[edge_axis].max(face_panel.min[edge_axis]),
            edge_panel.max[edge_axis].min(face_panel.max[edge_axis]),
        )
    };

    let contact_length = (s_max - s_min).abs();
    if contact_length < 10.0 {
        return Err(format!(
            "Panel contact length ({:.1} mm) is too short to place connectors.",
            contact_length
        ));
    }

    let n_holes = count.max(1);
    let eff_offset = if n_holes >= 2 && contact_length <= 2.0 * offset_x + 4.0 {
        // If edge is shorter than 2*offset_x, fall back to 20% margin so holes still fit inside the panel
        (contact_length * 0.2).max(10.0)
    } else {
        offset_x.max(5.0)
    };

    let (s_first, spacing) = if n_holes == 1 {
        (0.5 * (s_min + s_max) + shift_offset, 0.0)
    } else {
        let span = (contact_length - 2.0 * eff_offset).max(1.0);
        let base_step = span / f64::from(n_holes - 1);
        // Przesunięcie o połowę offsetu wzajemnego (shift_offset):
        // Pierwszy otwór przesuwa się o +shift_offset, a rozstaw o -2*shift_offset / (N-1),
        // dzięki czemu ostatni otwór automatycznie cofa się o -shift_offset od drugiego brzegu!
        let eff_spacing = if n_holes > 1 {
            (span - 2.0 * shift_offset) / f64::from(n_holes - 1)
        } else {
            base_step
        };
        (s_min + eff_offset + shift_offset, eff_spacing)
    };

    let round3 = |v: f64| (v * 1000.0).round() / 1000.0;

    let mut first_hole_pos = [0.0; 3];
    first_hole_pos[drill_axis] = round3(contact_d);
    first_hole_pos[width_axis] = round3(mid_thickness_w);
    first_hole_pos[edge_axis] = round3(s_first);

    let mut pattern_dir = [0.0; 3];
    pattern_dir[edge_axis] = 1.0;

    let face_thickness = (face_panel.max[drill_axis] - face_panel.min[drill_axis]).abs().max(10.0);

    Ok(ConnectorPlan {
        edge_panel: edge_panel.name.clone(),
        face_panel: face_panel.name.clone(),
        first_hole_pos,
        dir_into_edge,
        dir_into_face,
        pattern_dir,
        spacing: round3(spacing),
        count: n_holes,
        contact_length: round3(contact_length),
        face_thickness: round3(face_thickness),
    })
}

/// Interactive handler for `Złącza` (Kołki / Wkręty): drills edge + face holes and creates a linear pattern on the timeline.
pub fn create_connectors(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let Some(mut edge) = state.connector_edge_pick.clone() else {
        return Err("Step 1: Click a contact edge (panel edge) in the 3D viewport first.".into());
    };

    let mut face_pick = state.connector_face_pick.clone();

    let lib = crate::ConnectorLibrary::load()?;
    let (_, preset) = lib
        .find_preset(&state.connector_preset_id)
        .ok_or_else(|| "Selected connector preset not found in library.".to_string())?;

    let offset_x = parse_f64_or(&state.connector_offset_x, 50.0).max(5.0);
    let shift_total = parse_f64_or(&state.connector_offset_shift, 0.0);
    let shift_offset = (shift_total * 0.5) * state.connector_shift_sign;
    let count = parse_f64_or(&state.connector_count, 2.0).round().clamp(1.0, 100.0) as u32;

    let boxes = extract_panel_boxes(host);
    let mut plan = compute_connector_plan(&edge, face_pick.as_ref(), &boxes, offset_x, shift_offset, count)?;

    // Ensure we author timeline features at the root assembly level
    let _ = host.execute("component.activate", json!({ "component": "root" }));

    // 1. Upewnij się, że formatka ze ścianą (drugie zaznaczenie — jej numer idzie do nazw otworów i szyku)
    //    ma przypisany unikalny numer formatki (w razie potrzeby nadaj jej nowy numer S01_<NR>).
    let (resolved_face_panel, face_panel_num) =
        ensure_body_is_numbered_panel(&plan.face_panel, state, host);
    if resolved_face_panel != plan.face_panel {
        if edge.body == plan.face_panel {
            edge.body = resolved_face_panel.clone();
        }
        if let Some(fp) = face_pick.as_mut()
            && fp.body == plan.face_panel
        {
            fp.body = resolved_face_panel.clone();
        }
        plan.face_panel = resolved_face_panel;
    }

    // 2. Upewnij się również, że druga formatka (z brzegiem) ma swój numer, jeśli była niezdefiniowaną bryłą.
    let (resolved_edge_panel, _edge_panel_num) =
        ensure_body_is_numbered_panel(&plan.edge_panel, state, host);
    if resolved_edge_panel != plan.edge_panel {
        if edge.body == plan.edge_panel {
            edge.body = resolved_edge_panel.clone();
        }
        if let Some(fp) = face_pick.as_mut()
            && fp.body == plan.edge_panel
        {
            fp.body = resolved_edge_panel.clone();
        }
        plan.edge_panel = resolved_edge_panel;
    }

    // Temporarily hide Kubatura if it is visible so `solid.hole` surface checks never hit Kubatura
    let hid_kubatura_temp = !state.kubatura_hidden;
    if hid_kubatura_temp {
        let _ = host.execute(
            "browser.visibility",
            json!({ "items": ["Kubatura"], "visible": false }),
        );
    }

    let letter = next_connector_letter_for_panel(face_panel_num, host);
    state.connector_counter += 1;

    let edge_hole_name = format!("Hole_Edge_{:02}_{letter}", face_panel_num);
    let face_hole_name = format!("Hole_Face_{:02}_{letter}", face_panel_num);
    let pattern_name = format!("Connector_Pattern_{:02}_{letter}", face_panel_num);

    // 1. Otwór w brzegu (czole) formatki (np. Wieniec / Trawers / Półka)
    let res_edge = host.execute(
        "solid.hole",
        json!({
            "name": edge_hole_name,
            "position": plan.first_hole_pos,
            "direction": plan.dir_into_edge,
            "diameter": preset.edge_hole.diameter,
            "depth": preset.edge_hole.depth,
            "targets": [plan.edge_panel],
        }),
    );

    if let Err(e) = res_edge {
        if hid_kubatura_temp {
            let _ = host.execute(
                "browser.visibility",
                json!({ "items": ["Kubatura"], "visible": true }),
            );
        }
        return Err(format!("Failed to drill edge hole in '{}': {e}", plan.edge_panel));
    }

    // 2. Otwór prostopadły w ścianie formatki (np. Bok) — przelotowy (gdy zaznaczono / wkręty) lub ślepy (kołki)
    let face_hole_params = if state.connector_through_face || preset.face_hole.depth <= 0.0 {
        // Oblicz bezpieczną głębokość przelotową na podstawie rzeczywistej grubości płyty + margines (2mm)
        let through_depth = (plan.face_thickness + 2.0).max(20.0);
        json!({
            "name": face_hole_name,
            "position": plan.first_hole_pos,
            "direction": plan.dir_into_face,
            "diameter": preset.face_hole.diameter,
            "depth": through_depth,
            "targets": [plan.face_panel],
        })
    } else {
        json!({
            "name": face_hole_name,
            "position": plan.first_hole_pos,
            "direction": plan.dir_into_face,
            "diameter": preset.face_hole.diameter,
            "depth": preset.face_hole.depth,
            "targets": [plan.face_panel],
        })
    };

    let res_face = host.execute("solid.hole", face_hole_params);
    if let Err(e) = res_face {
        if hid_kubatura_temp {
            let _ = host.execute(
                "browser.visibility",
                json!({ "items": ["Kubatura"], "visible": true }),
            );
        }
        return Err(format!("Failed to drill face hole in '{}': {e}", plan.face_panel));
    }

    // 3. Szyk liniowy na osi czasu (Timeline)
    if plan.count >= 2 {
        let _ = host.execute(
            "solid.pattern.rectangular",
            json!({
                "name": pattern_name,
                "features": [edge_hole_name, face_hole_name],
                "dir1": plan.pattern_dir,
                "count1": plan.count,
                "spacing1": plan.spacing,
            }),
        );
    }

    if hid_kubatura_temp {
        let _ = host.execute(
            "browser.visibility",
            json!({ "items": ["Kubatura"], "visible": true }),
        );
    }

    // Zapisujemy ostatnie poprawne zaznaczenie do bufora pamięci (do powtórzenia np. dla wkrętów)
    state.connector_last_edge_pick = Some(edge);
    state.connector_last_face_pick = face_pick;

    // Czyścimy aktywne zaznaczenie oraz podgląd duchów (Item 4: duchy znikają po umieszczeniu)
    state.connector_edge_pick = None;
    state.connector_face_pick = None;
    state.ghost_cylinders = None;
    state.connector_active_slot = 0;
    host.enable_edge_selection();

    let mode_desc = if state.connector_through_face {
        "through"
    } else {
        "blind"
    };
    Ok(format!(
        "Added {}× {} [#{:02}_{}] (edge: '{}', face {}: '{}', L={:.0} mm, spacing {:.1} mm). Pick next pair (Edge + Face).",
        plan.count,
        preset.name,
        face_panel_num,
        letter,
        plan.edge_panel,
        mode_desc,
        plan.face_panel,
        plan.contact_length,
        plan.spacing
    ))
}

/// Plan geometryczny dla połączenia gniazdowo-czopowego CNC `T-Dogbone`.
#[derive(Clone, Debug, PartialEq)]
pub struct TDogbonePlan {
    pub edge_panel: String,
    pub face_panel: String,
    pub edge_axis: usize,
    pub drill_axis: usize,
    pub width_axis: usize,
    pub contact_d: f64,
    pub sign_into_edge: f64,
    pub sign_into_face: f64,
    pub dir_into_edge: [f64; 3],
    pub dir_into_face: [f64; 3],
    pub w_min: f64,
    pub w_max: f64,
    pub edge_thickness: f64,
    pub face_thickness: f64,
    pub s_min: f64,
    pub s_max: f64,
    pub contact_length: f64,
    pub joint_width: f64,
    pub eff_offset_x: f64,
    pub spacing: f64,
    pub joint_centers: Vec<f64>,
}

impl TDogbonePlan {
    /// Zwraca listę prostopadłościanów 3D `(min, max)` dla wszystkich czopów (do podglądu ducha w 3D).
    pub fn tenon_bounding_boxes(&self) -> Vec<([f64; 3], [f64; 3])> {
        let half_w = 0.5 * self.joint_width;
        let d0 = self.contact_d;
        let d1 = self.contact_d + self.sign_into_face * self.face_thickness;
        let d_min = d0.min(d1);
        let d_max = d0.max(d1);

        let mut out = Vec::with_capacity(self.joint_centers.len());
        for &c_u in &self.joint_centers {
            let mut b_min = [0.0; 3];
            let mut b_max = [0.0; 3];
            b_min[self.edge_axis] = c_u - half_w;
            b_max[self.edge_axis] = c_u + half_w;
            b_min[self.width_axis] = self.w_min;
            b_max[self.width_axis] = self.w_max;
            b_min[self.drill_axis] = d_min;
            b_max[self.drill_axis] = d_max;
            out.push((b_min, b_max));
        }
        out
    }
}

/// Oblicza plan rozmieszczenia połączeń `T-Dogbone` na podstawie wskazanej krawędzi (formatka z czopem)
/// oraz ściany docelowej (formatka z gniazdem).
pub fn compute_tdogbone_plan(
    edge: &crate::EdgePick,
    face: Option<&crate::FacePick>,
    boxes: &[PanelBox],
    joint_width: f64,
    offset_x: f64,
    count: u32,
) -> Result<TDogbonePlan, String> {
    let dv = [
        edge.end[0] - edge.start[0],
        edge.end[1] - edge.start[1],
        edge.end[2] - edge.start[2],
    ];
    let ad = [dv[0].abs(), dv[1].abs(), dv[2].abs()];
    let edge_axis = if ad[0] >= ad[1] && ad[0] >= ad[2] {
        0
    } else if ad[1] >= ad[0] && ad[1] >= ad[2] {
        1
    } else {
        2
    };
    let raw_len = (dv[0] * dv[0] + dv[1] * dv[1] + dv[2] * dv[2]).sqrt();
    if raw_len < 1.0 {
        return Err("Selected edge is too short (< 1 mm).".into());
    }

    let mid = [
        0.5 * (edge.start[0] + edge.end[0]),
        0.5 * (edge.start[1] + edge.end[1]),
        0.5 * (edge.start[2] + edge.end[2]),
    ];

    let (face_panel, edge_panel) = if let Some(fp) = face {
        let f_panel = boxes
            .iter()
            .find(|b| b.name == fp.body)
            .or_else(|| {
                boxes
                    .iter()
                    .filter(|b| b.dist_to_point(fp.point) <= 5.0)
                    .min_by(|a, b| {
                        a.dist_to_point(fp.point)
                            .partial_cmp(&b.dist_to_point(fp.point))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            })
            .ok_or_else(|| format!("Panel not found for selected face '{}'.", fp.body))?;

        let e_panel = boxes
            .iter()
            .find(|b| b.name != f_panel.name && b.name == edge.body)
            .or_else(|| {
                boxes
                    .iter()
                    .filter(|b| b.name != f_panel.name)
                    .min_by(|a, b| {
                        a.dist_to_point(mid)
                            .partial_cmp(&b.dist_to_point(mid))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            })
            .ok_or_else(|| "Panel with tenon not found at the selected edge.".to_string())?;

        (f_panel, e_panel)
    } else {
        let primary = boxes
            .iter()
            .find(|b| b.name == edge.body)
            .or_else(|| {
                boxes
                    .iter()
                    .filter(|b| b.dist_to_point(mid) <= 5.0)
                    .min_by(|a, b| {
                        a.dist_to_point(mid)
                            .partial_cmp(&b.dist_to_point(mid))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            })
            .ok_or_else(|| "Panel not found at the selected edge.".to_string())?;

        let secondary = boxes
            .iter()
            .filter(|b| b.name != primary.name)
            .filter(|b| {
                let mut perp_dist2 = 0.0;
                for a in 0..3 {
                    if a == edge_axis {
                        continue;
                    }
                    if mid[a] < b.min[a] {
                        let d = b.min[a] - mid[a];
                        perp_dist2 += d * d;
                    } else if mid[a] > b.max[a] {
                        let d = mid[a] - b.max[a];
                        perp_dist2 += d * d;
                    }
                }
                let e_min = edge.start[edge_axis].min(edge.end[edge_axis]);
                let e_max = edge.start[edge_axis].max(edge.end[edge_axis]);
                let overlap = e_max.min(b.max[edge_axis]) - e_min.max(b.min[edge_axis]);
                perp_dist2.sqrt() <= 5.0 && overlap > 5.0
            })
            .min_by(|a, b| {
                a.dist_to_point(mid)
                    .partial_cmp(&b.dist_to_point(mid))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .ok_or_else(|| {
                format!(
                    "No second panel detected touching '{}' at this edge.",
                    primary.name
                )
            })?;

        let t_pri = primary.thin_axis();
        let t_sec = secondary.thin_axis();
        let c_pri = primary.center();
        let c_sec = secondary.center();

        let pri_is_face = c_sec[t_pri] < primary.min[t_pri] - 0.5 || c_sec[t_pri] > primary.max[t_pri] + 0.5;
        let sec_is_face = c_pri[t_sec] < secondary.min[t_sec] - 0.5 || c_pri[t_sec] > secondary.max[t_sec] + 0.5;

        if pri_is_face && !sec_is_face {
            (primary, secondary)
        } else if sec_is_face && !pri_is_face {
            (secondary, primary)
        } else if primary.extents()[t_pri] <= secondary.extents()[t_sec] {
            (primary, secondary)
        } else {
            (secondary, primary)
        }
    };

    let drill_axis = face_panel.thin_axis();
    let width_axis = (0..3)
        .find(|&a| a != edge_axis && a != drill_axis)
        .unwrap_or_else(|| edge_panel.thin_axis());

    let edge_center_d = edge_panel.center()[drill_axis];
    let face_center_d = face_panel.center()[drill_axis];

    let (contact_d, sign_into_edge) = if edge_center_d >= face_center_d {
        (edge_panel.min[drill_axis], 1.0_f64)
    } else {
        (edge_panel.max[drill_axis], -1.0_f64)
    };
    let sign_into_face = -sign_into_edge;

    let mut dir_into_edge = [0.0; 3];
    dir_into_edge[drill_axis] = sign_into_edge;
    let mut dir_into_face = [0.0; 3];
    dir_into_face[drill_axis] = sign_into_face;

    let w_min = edge_panel.min[width_axis];
    let w_max = edge_panel.max[width_axis];
    let edge_thickness = (w_max - w_min).abs().max(4.0);
    let face_thickness = (face_panel.max[drill_axis] - face_panel.min[drill_axis])
        .abs()
        .max(4.0);

    let pick_min = edge.start[edge_axis].min(edge.end[edge_axis]);
    let pick_max = edge.start[edge_axis].max(edge.end[edge_axis]);
    let s_min = pick_min
        .max(edge_panel.min[edge_axis])
        .max(face_panel.min[edge_axis]);
    let s_max = pick_max
        .min(edge_panel.max[edge_axis])
        .min(face_panel.max[edge_axis]);

    let (s_min, s_max) = if s_max > s_min + 1.0 {
        (s_min, s_max)
    } else {
        (
            edge_panel.min[edge_axis].max(face_panel.min[edge_axis]),
            edge_panel.max[edge_axis].min(face_panel.max[edge_axis]),
        )
    };

    let contact_length = (s_max - s_min).abs();
    let eff_width = joint_width.max(16.0);
    if contact_length < eff_width + 4.0 {
        return Err(format!(
            "Contact edge ({:.1} mm) is shorter than required joint width ({:.1} mm).",
            contact_length, eff_width
        ));
    }

    let n_joints = count.clamp(1, 20);
    let half_w = 0.5 * eff_width;

    let (eff_offset_x, spacing, joint_centers) = if n_joints == 1 {
        let sym_offset = 0.5 * (contact_length - eff_width);
        let center = 0.5 * (s_min + s_max);
        (sym_offset, 0.0, vec![center])
    } else {
        let max_possible_offset = ((contact_length - f64::from(n_joints) * eff_width) * 0.5).max(0.0);
        let off = offset_x.max(0.0).min(max_possible_offset);
        let first_c = s_min + off + half_w;
        let last_c = s_max - off - half_w;
        let step = if n_joints > 1 {
            (last_c - first_c) / f64::from(n_joints - 1)
        } else {
            0.0
        };
        if step < eff_width + 1.0 {
            return Err(format!(
                "Tenons overlap (center spacing {:.1} mm < tenon width {:.1} mm). Reduce count, width, or end offset.",
                step, eff_width
            ));
        }
        let mut centers = Vec::with_capacity(n_joints as usize);
        for i in 0..n_joints {
            centers.push(first_c + f64::from(i) * step);
        }
        (off, step, centers)
    };

    Ok(TDogbonePlan {
        edge_panel: edge_panel.name.clone(),
        face_panel: face_panel.name.clone(),
        edge_axis,
        drill_axis,
        width_axis,
        contact_d,
        sign_into_edge,
        sign_into_face,
        dir_into_edge,
        dir_into_face,
        w_min,
        w_max,
        edge_thickness,
        face_thickness,
        s_min,
        s_max,
        contact_length,
        joint_width: eff_width,
        eff_offset_x,
        spacing,
        joint_centers,
    })
}

/// Tworzy połączenie gniazdowo-czopowe `T-Dogbone`:
/// 1. Ukrywa formatkę z gniazdem i wykonuje na formatce z czopem szkic czopów + trójkątnych odciążeń HSM z łukiem R=7mm,
///    po czym scala czopy (`join`) i wycina odciążenia (`cut`) wyłącznie w formatce z czopem.
/// 2. Ukrywa formatkę z czopem, odkrywa formatkę z gniazdem, wierci po 4 otwory Dogbone (`solid.hole`, fi 8mm, 4mm pod kątem 45°)
///    oraz wycina prostokątne gniazda (`cut`) wyłącznie w formatce z gniazdem, dzięki czemu nigdy nie wycina czopa ani nie scala obu płyt.
/// 3. Przywraca widoczność obu formatek.
pub fn create_tdogbone(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let Some(mut edge) = state.tdogbone_edge_pick.clone() else {
        return Err("Step 1: Click the edge of the panel that will receive the tenon first.".into());
    };
    let mut face_pick = state.tdogbone_face_pick.clone();

    let lib = crate::CncJointLibrary::load()?;
    let preset = lib
        .find_preset(&state.tdogbone_preset_id)
        .cloned()
        .ok_or_else(|| "Joint preset not found in cnc_joints.json.".to_string())?;

    let joint_width = parse_f64_or(&state.tdogbone_width, preset.joint_width_mm).max(16.0);
    let offset_x = parse_f64_or(&state.tdogbone_offset_x, preset.edge_offset_mm).max(0.0);
    let count = parse_f64_or(&state.tdogbone_count, f64::from(preset.default_count))
        .round()
        .clamp(1.0, 20.0) as u32;

    let boxes = extract_panel_boxes(host);
    let mut plan = compute_tdogbone_plan(&edge, face_pick.as_ref(), &boxes, joint_width, offset_x, count)?;

    let _ = host.execute("component.activate", json!({ "component": "root" }));

    // Upewnij się, że obie formatki posiadają swoje numery w strukturze szafki
    let (resolved_face_panel, face_panel_num) =
        ensure_body_is_numbered_panel(&plan.face_panel, state, host);
    if resolved_face_panel != plan.face_panel {
        if edge.body == plan.face_panel {
            edge.body = resolved_face_panel.clone();
        }
        if let Some(fp) = face_pick.as_mut()
            && fp.body == plan.face_panel
        {
            fp.body = resolved_face_panel.clone();
        }
        plan.face_panel = resolved_face_panel;
    }

    let (resolved_edge_panel, edge_panel_num) =
        ensure_body_is_numbered_panel(&plan.edge_panel, state, host);
    if resolved_edge_panel != plan.edge_panel {
        if edge.body == plan.edge_panel {
            edge.body = resolved_edge_panel.clone();
        }
        if let Some(fp) = face_pick.as_mut()
            && fp.body == plan.edge_panel
        {
            fp.body = resolved_edge_panel.clone();
        }
        plan.edge_panel = resolved_edge_panel;
    }

    let kubatura_was_visible = !state.kubatura_hidden;
    let restore_visibility = |host: &mut dyn WoodHost, edge_p: &str, face_p: &str, show_kub: bool| {
        let mut items = vec![edge_p.to_string(), face_p.to_string()];
        if show_kub {
            items.push("Kubatura".to_string());
        }
        let _ = host.execute("browser.visibility", json!({ "items": items, "visible": true }));
    };

    let letter = next_connector_letter_for_panel(face_panel_num, host);
    state.tdogbone_counter += 1;
    let round3 = |v: f64| (v * 1000.0).round() / 1000.0;

    // =========================================================================
    // KROK A: Operacje na formatce z czopem (`plan.edge_panel`)
    //         Ukrywamy `plan.face_panel` (oraz `Kubatura`), czyścimy zaznaczenie
    //         i jawnie wskazujemy `targets: [plan.edge_panel]`, aby czop nigdy
    //         nie scalił dwóch formatek razem!
    // =========================================================================
    let mut hide_for_edge = vec![plan.face_panel.clone()];
    if kubatura_was_visible {
        hide_for_edge.push("Kubatura".to_string());
    }
    let _ = host.execute(
        "browser.visibility",
        json!({ "items": hide_for_edge, "visible": false }),
    );
    let _ = host.execute(
        "browser.visibility",
        json!({ "items": [plan.edge_panel], "visible": true }),
    );
    let _ = host.execute("select.clear", json!({}));

    let mut x_dir = [0.0; 3];
    x_dir[plan.edge_axis] = 1.0;
    let y_dir = plan.dir_into_edge;

    let n = [
        x_dir[1] * y_dir[2] - x_dir[2] * y_dir[1],
        x_dir[2] * y_dir[0] - x_dir[0] * y_dir[2],
        x_dir[0] * y_dir[1] - x_dir[1] * y_dir[0],
    ];
    let origin_w = if n[plan.width_axis] >= 0.0 {
        plan.w_min
    } else {
        plan.w_max
    };
    let mut sketch_origin = [0.0; 3];
    sketch_origin[plan.width_axis] = round3(origin_w);
    sketch_origin[plan.edge_axis] = 0.0;
    sketch_origin[plan.drill_axis] = round3(plan.contact_d);

    let sketch_name = format!("Szkic_Czop_{:02}_{letter}", edge_panel_num);
    if let Err(e) = host.execute(
        "sketch.create",
        json!({
            "name": sketch_name,
            "plane": {
                "origin": sketch_origin,
                "x_dir": x_dir,
                "y_dir": y_dir,
            }
        }),
    ) {
        restore_visibility(host, &plan.edge_panel, &plan.face_panel, kubatura_was_visible);
        return Err(format!("Failed to create tenon sketch: {e}"));
    }

    let half_w = 0.5 * plan.joint_width;
    let relief_w_nom = preset.relief_width_mm.max(10.0);
    let relief_d = preset.relief_depth_mm.max(4.0);
    let relief_r = preset.relief_radius_mm.max(2.0);
    let v_bot = round3(-plan.face_thickness);

    let mut tenon_profile_picks = Vec::new();
    let mut hsm_profile_picks = Vec::new();

    for (i, &c_u) in plan.joint_centers.iter().enumerate() {
        let idx1 = i + 1;
        let u_left = round3(c_u - half_w);
        let u_right = round3(c_u + half_w);

        // 1. Prostokąt czopa (ADD-CZOP)
        if let Err(e) = host.execute(
            "sketch.line",
            json!({
                "points": [
                    [u_left, 0.0],
                    [u_left, v_bot],
                    [u_right, v_bot],
                    [u_right, 0.0]
                ],
                "closed": true,
                "ids": [
                    format!("t{idx1}_l"),
                    format!("t{idx1}_b"),
                    format!("t{idx1}_r"),
                    format!("t{idx1}_t")
                ]
            }),
        ) {
            let _ = host.execute("sketch.cancel", json!({}));
            restore_visibility(host, &plan.edge_panel, &plan.face_panel, kubatura_was_visible);
            return Err(format!("Failed to draw tenon profile #{idx1}: {e}"));
        }
        tenon_profile_picks.push(json!({
            "point": [round3(c_u), round3(-0.5 * plan.face_thickness)]
        }));

        // 2. Lewe odciążenie barku HSM (REMOVER1-CZOP): trójkąt prostokątny + zaokrąglenie R=7mm
        let left_bound = if i == 0 {
            plan.s_min + 1.0
        } else {
            let prev_right = plan.joint_centers[i - 1] + half_w;
            0.5 * (prev_right + u_left) + 0.5
        };
        let eff_left_w = relief_w_nom.min((u_left - left_bound).max(0.0));
        if eff_left_w >= relief_r + 2.0 {
            let lv = format!("hl{idx1}_v");
            let lh = format!("hl{idx1}_h");
            let le = format!("hl{idx1}_e");
            if host
                .execute(
                    "sketch.line",
                    json!({
                        "points": [
                            [u_left, 0.0],
                            [u_left, round3(relief_d)],
                            [round3(u_left - eff_left_w), 0.0]
                        ],
                        "closed": true,
                        "ids": [lv, lh, le]
                    }),
                )
                .is_ok()
                && host
                    .execute(
                        "sketch.fillet",
                        json!({
                            "a": lv,
                            "b": lh,
                            "radius": relief_r
                        }),
                    )
                    .is_ok()
            {
                hsm_profile_picks.push(json!({
                    "point": [round3(u_left - 0.5 * eff_left_w), round3(0.2 * relief_d)]
                }));
            }
        }

        // 3. Prawe odciążenie barku HSM (REMOVER2-CZOP): trójkąt prostokątny + zaokrąglenie R=7mm
        let right_bound = if i + 1 == plan.joint_centers.len() {
            plan.s_max - 1.0
        } else {
            let next_left = plan.joint_centers[i + 1] - half_w;
            0.5 * (u_right + next_left) - 0.5
        };
        let eff_right_w = relief_w_nom.min((right_bound - u_right).max(0.0));
        if eff_right_w >= relief_r + 2.0 {
            let rv = format!("hr{idx1}_v");
            let rh = format!("hr{idx1}_h");
            let re = format!("hr{idx1}_e");
            if host
                .execute(
                    "sketch.line",
                    json!({
                        "points": [
                            [u_right, 0.0],
                            [u_right, round3(relief_d)],
                            [round3(u_right + eff_right_w), 0.0]
                        ],
                        "closed": true,
                        "ids": [rv, rh, re]
                    }),
                )
                .is_ok()
                && host
                    .execute(
                        "sketch.fillet",
                        json!({
                            "a": rv,
                            "b": rh,
                            "radius": relief_r
                        }),
                    )
                    .is_ok()
            {
                hsm_profile_picks.push(json!({
                    "point": [round3(u_right + 0.5 * eff_right_w), round3(0.2 * relief_d)]
                }));
            }
        }
    }

    let _ = host.execute("sketch.finish", json!({}));

    // Scal czopy z formatką z czopem (`operation: "join"`, `targets: [plan.edge_panel]`)
    if let Err(e) = host.execute(
        "solid.extrude",
        json!({
            "name": format!("Czop_Add_{:02}_{letter}", edge_panel_num),
            "sketch": sketch_name,
            "profiles": tenon_profile_picks,
            "direction": "positive",
            "distance": round3(plan.edge_thickness),
            "operation": "join",
            "targets": [plan.edge_panel],
            "participants": [plan.edge_panel]
        }),
    ) {
        restore_visibility(host, &plan.edge_panel, &plan.face_panel, kubatura_was_visible);
        return Err(format!(
            "Failed to join tenon to panel '{}': {e}",
            plan.edge_panel
        ));
    }

    // Wytnij odciążenia HSM z formatki z czopem (`operation: "cut"`, `targets: [plan.edge_panel]`)
    if !hsm_profile_picks.is_empty()
        && let Err(e) = host.execute(
            "solid.extrude",
            json!({
                "name": format!("Czop_HSM_Cut_{:02}_{letter}", edge_panel_num),
                "sketch": sketch_name,
                "profiles": hsm_profile_picks,
                "direction": "positive",
                "distance": round3(plan.edge_thickness),
                "operation": "cut",
                "targets": [plan.edge_panel],
                "participants": [plan.edge_panel]
            }),
        )
    {
        restore_visibility(host, &plan.edge_panel, &plan.face_panel, kubatura_was_visible);
        return Err(format!(
            "Failed to cut HSM relief in panel '{}': {e}",
            plan.edge_panel
        ));
    }

    // =========================================================================
    // KROK B: Operacje na formatce z gniazdem (`plan.face_panel`)
    //         Ukrywamy `plan.edge_panel` (z gotowym czopem) i odkrywamy `plan.face_panel`,
    //         aby wycinanie gniazd i wiercenie dogbone nigdy nie wycięło czopa!
    // =========================================================================
    let _ = host.execute(
        "browser.visibility",
        json!({ "items": [plan.edge_panel], "visible": false }),
    );
    let _ = host.execute(
        "browser.visibility",
        json!({ "items": [plan.face_panel], "visible": true }),
    );
    let _ = host.execute("select.clear", json!({}));

    let angle_rad = preset.dogbone_angle_deg.to_radians();
    let d_w = preset.dogbone_corner_offset_mm * angle_rad.cos();
    let d_u = preset.dogbone_corner_offset_mm * angle_rad.sin();
    let d0 = plan.contact_d;
    let d1 = plan.contact_d + plan.sign_into_face * plan.face_thickness;
    let d_min = d0.min(d1);

    for (i, &c_u) in plan.joint_centers.iter().enumerate() {
        let idx1 = i + 1;
        let u_left = round3(c_u - half_w);
        let u_right = round3(c_u + half_w);

        // 1. 4 otwory Dogbone wykonane za pomocą `solid.hole` (fi 8mm, odsunięte o 4mm pod kątem 45° od narożników)
        let hole_corners = [
            (plan.w_min + d_w, u_left + d_u),
            (plan.w_max - d_w, u_left + d_u),
            (plan.w_max - d_w, u_right - d_u),
            (plan.w_min + d_w, u_right - d_u),
        ];
        for (h_idx, (w_pos, u_pos)) in hole_corners.iter().enumerate() {
            let mut pt = [0.0; 3];
            pt[plan.width_axis] = round3(*w_pos);
            pt[plan.edge_axis] = round3(*u_pos);
            pt[plan.drill_axis] = round3(plan.contact_d);

            if let Err(e) = host.execute(
                "solid.hole",
                json!({
                    "name": format!("Dogbone_{face_panel_num:02}_{letter}_{idx1}_{}", h_idx + 1),
                    "position": pt,
                    "direction": plan.dir_into_face,
                    "diameter": preset.dogbone_drill_dia_mm,
                    "depth": round3(plan.face_thickness),
                    "targets": [plan.face_panel]
                }),
            ) {
                restore_visibility(host, &plan.edge_panel, &plan.face_panel, kubatura_was_visible);
                return Err(format!(
                    "Failed to drill Dogbone hole #{} in '{}': {e}",
                    h_idx + 1,
                    plan.face_panel
                ));
            }
        }

        // 2. Wycięcie prostokątnego gniazda przelotowego wyłącznie w `plan.face_panel`
        let mut box_min = [0.0; 3];
        let mut box_size = [0.0; 3];
        box_min[plan.width_axis] = round3(plan.w_min);
        box_min[plan.edge_axis] = round3(u_left);
        box_min[plan.drill_axis] = round3(d_min);

        box_size[plan.width_axis] = round3(plan.edge_thickness);
        box_size[plan.edge_axis] = round3(plan.joint_width);
        box_size[plan.drill_axis] = round3(plan.face_thickness);

        if let Err(e) = host.execute(
            "solid.box",
            json!({
                "name": format!("Gniazdo_{:02}_{letter}_{idx1}", face_panel_num),
                "corner": box_min,
                "length": box_size[0],
                "width": box_size[1],
                "height": box_size[2],
                "operation": "cut",
                "targets": [plan.face_panel],
                "participants": [plan.face_panel]
            }),
        ) {
            restore_visibility(host, &plan.edge_panel, &plan.face_panel, kubatura_was_visible);
            return Err(format!(
                "Failed to cut mortise #{idx1} in panel '{}': {e}",
                plan.face_panel
            ));
        }
    }

    // =========================================================================
    // KROK C: Przywrócenie widoczności obu formatek
    // =========================================================================
    restore_visibility(host, &plan.edge_panel, &plan.face_panel, kubatura_was_visible);

    state.tdogbone_last_edge_pick = Some(edge);
    state.tdogbone_last_face_pick = face_pick;
    state.tdogbone_edge_pick = None;
    state.tdogbone_face_pick = None;
    state.ghost_box = None;
    state.tdogbone_active_slot = 0;
    host.enable_edge_selection();

    Ok(format!(
        "Added {}× T-Dogbone {:.0}×{:.0} mm [#{:02}_{}] (tenon + HSM R={:.0} in '{}', mortise + 4×dia {:.0} in '{}'). Pick next pair.",
        plan.joint_centers.len(),
        plan.joint_width,
        plan.edge_thickness,
        face_panel_num,
        letter,
        preset.relief_radius_mm,
        plan.edge_panel,
        preset.dogbone_drill_dia_mm,
        plan.face_panel
    ))
}

pub const LAMELLO_BLADE_DIA_MM: f64 = 100.4;
pub const LAMELLO_SLOT_WIDTH_MM: f64 = 10.0;
pub const LAMELLO_SLOT_DEPTH_MM: f64 = 14.0;

/// Plan geometryczny dla połączenia Lamello P-14 (dysk Ø100.4 × 10 mm, głębokość 14 mm w obu formatkach).
#[derive(Clone, Debug, PartialEq)]
pub struct LamelloP14Plan {
    pub edge_panel: String,
    pub face_panel: String,
    pub edge_axis: usize,
    pub drill_axis: usize,
    pub width_axis: usize,
    pub contact_d: f64,
    pub sign_into_edge: f64,
    pub sign_into_face: f64,
    pub dir_into_edge: [f64; 3],
    pub dir_into_face: [f64; 3],
    pub pattern_dir: [f64; 3],
    pub w_min: f64,
    pub w_max: f64,
    pub w_center: f64,
    pub hex_start_w: f64,
    pub hex_dir_w: f64,
    pub z_offset_eff: f64,
    pub edge_thickness: f64,
    pub face_thickness: f64,
    pub s_min: f64,
    pub s_max: f64,
    pub contact_length: f64,
    pub chord_length: f64,
    pub eff_offset_x: f64,
    pub count: u32,
    pub spacing: f64,
    pub joint_centers: Vec<f64>,
}

/// Oblicza plan rozmieszczenia gniazd Lamello P-14 na podstawie wskazanej krawędzi i ściany przylegającej formatki.
pub fn compute_lamello_p14_plan(
    edge: &crate::EdgePick,
    face: Option<&crate::FacePick>,
    boxes: &[PanelBox],
    z_offset: f64,
    offset_x: f64,
    count: u32,
) -> Result<LamelloP14Plan, String> {
    let base = compute_tdogbone_plan(edge, face, boxes, 16.0, 0.0, 1)?;

    let r = 0.5 * LAMELLO_BLADE_DIA_MM;
    let dy = r - LAMELLO_SLOT_DEPTH_MM;
    let chord_length = 2.0 * (r * r - dy * dy).max(0.0).sqrt();

    if base.contact_length < chord_length + 4.0 {
        return Err(format!(
            "Contact edge ({:.1} mm) is shorter than Lamello P-14 arc length ({:.1} mm).",
            base.contact_length, chord_length
        ));
    }

    let half_chord = 0.5 * chord_length;
    let n_joints = count.clamp(1, 20);
    let (eff_offset_x, spacing, joint_centers) = if n_joints == 1 {
        let center = 0.5 * (base.s_min + base.s_max);
        (0.5 * base.contact_length, 0.0, vec![center])
    } else {
        let min_center_off = half_chord + 2.0;
        let max_center_off = (0.5 * base.contact_length - 1.0).max(min_center_off);
        let off = offset_x.clamp(min_center_off, max_center_off);
        let first_c = base.s_min + off;
        let last_c = base.s_max - off;
        let step = (last_c - first_c) / f64::from(n_joints - 1);
        if step < chord_length + 1.0 {
            return Err(format!(
                "Lamello slots overlap (spacing {:.1} mm < arc length {:.1} mm). Reduce count or end offset.",
                step, chord_length
            ));
        }
        let mut centers = Vec::with_capacity(n_joints as usize);
        for i in 0..n_joints {
            centers.push(first_c + f64::from(i) * step);
        }
        (off, step, centers)
    };

    let half_slot_w = 0.5 * LAMELLO_SLOT_WIDTH_MM;
    let clamped_z = z_offset.clamp(
        half_slot_w + 0.5,
        (base.edge_thickness - half_slot_w - 0.5).max(half_slot_w + 0.5),
    );
    let dist_to_min = (edge.point[base.width_axis] - base.w_min).abs();
    let dist_to_max = (edge.point[base.width_axis] - base.w_max).abs();
    let (hex_start_w, hex_dir_w, w_center) = if dist_to_max < dist_to_min {
        (base.w_max, -1.0_f64, base.w_max - clamped_z)
    } else {
        (base.w_min, 1.0_f64, base.w_min + clamped_z)
    };

    let mut pattern_dir = [0.0; 3];
    pattern_dir[base.edge_axis] = 1.0;

    Ok(LamelloP14Plan {
        edge_panel: base.edge_panel,
        face_panel: base.face_panel,
        edge_axis: base.edge_axis,
        drill_axis: base.drill_axis,
        width_axis: base.width_axis,
        contact_d: base.contact_d,
        sign_into_edge: base.sign_into_edge,
        sign_into_face: base.sign_into_face,
        dir_into_edge: base.dir_into_edge,
        dir_into_face: base.dir_into_face,
        pattern_dir,
        w_min: base.w_min,
        w_max: base.w_max,
        w_center,
        hex_start_w,
        hex_dir_w,
        z_offset_eff: clamped_z,
        edge_thickness: base.edge_thickness,
        face_thickness: base.face_thickness,
        s_min: base.s_min,
        s_max: base.s_max,
        contact_length: base.contact_length,
        chord_length,
        eff_offset_x,
        count: n_joints,
        spacing,
        joint_centers,
    })
}

fn cut_single_lamello_disc_on_panel(
    host: &mut dyn WoodHost,
    feature_name: &str,
    target_panel: &str,
    plan: &LamelloP14Plan,
    center_u: f64,
    sign_into_panel: f64,
) -> Result<(), String> {
    let round3 = |v: f64| (v * 1000.0).round() / 1000.0;
    let r = 0.5 * LAMELLO_BLADE_DIA_MM;
    let offset_outside = r - LAMELLO_SLOT_DEPTH_MM;

    let mut base = [0.0; 3];
    base[plan.edge_axis] = round3(center_u);
    base[plan.width_axis] = round3(plan.w_center - 0.5 * LAMELLO_SLOT_WIDTH_MM);
    base[plan.drill_axis] = round3(plan.contact_d - sign_into_panel * offset_outside);

    let mut axis = [0.0; 3];
    axis[plan.width_axis] = 1.0;

    host.execute(
        "solid.cylinder",
        json!({
            "name": feature_name,
            "base": base,
            "axis": axis,
            "diameter": LAMELLO_BLADE_DIA_MM,
            "height": LAMELLO_SLOT_WIDTH_MM,
            "operation": "cut",
            "targets": [target_panel],
            "participants": [target_panel]
        }),
    )
    .map_err(|e| format!("Failed to cut Lamello cylinder in '{target_panel}': {e}"))?;

    Ok(())
}

/// Tworzy połączenie Lamello P-14 na dwóch przylegających formatkach:
/// wycina pierwsze gniazdo dyskiem Ø100.4 × 10 mm na głębokość 14 mm w dwóch podejściach
/// (formatka 1 i przylegająca formatka 2), a następnie powiela szykiem prostokątnym (`N >= 2`).
pub fn create_lamello_p14(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let Some(mut edge) = state.lamello_edge_pick.clone() else {
        return Err("Step 1: Click the contact edge of the first panel first.".into());
    };
    let mut face_pick = state.lamello_face_pick.clone();

    let z_offset = parse_f64_or(&state.lamello_z_offset, 9.0).max(1.0);
    let offset_x = parse_f64_or(&state.lamello_offset_x, 50.0).max(0.0);
    let count = parse_f64_or(&state.lamello_count, 2.0).round().clamp(1.0, 20.0) as u32;

    let boxes = extract_panel_boxes(host);
    let mut plan = compute_lamello_p14_plan(
        &edge,
        face_pick.as_ref(),
        &boxes,
        z_offset,
        offset_x,
        count,
    )?;

    let first_center = plan
        .joint_centers
        .first()
        .copied()
        .ok_or_else(|| "No Lamello slot center computed.".to_string())?;

    let _ = host.execute("component.activate", json!({ "component": "root" }));

    let (resolved_face_panel, face_panel_num) =
        ensure_body_is_numbered_panel(&plan.face_panel, state, host);
    if resolved_face_panel != plan.face_panel {
        if edge.body == plan.face_panel {
            edge.body = resolved_face_panel.clone();
        }
        if let Some(fp) = face_pick.as_mut()
            && fp.body == plan.face_panel
        {
            fp.body = resolved_face_panel.clone();
        }
        plan.face_panel = resolved_face_panel;
    }

    let (resolved_edge_panel, edge_panel_num) =
        ensure_body_is_numbered_panel(&plan.edge_panel, state, host);
    if resolved_edge_panel != plan.edge_panel {
        if edge.body == plan.edge_panel {
            edge.body = resolved_edge_panel.clone();
        }
        if let Some(fp) = face_pick.as_mut()
            && fp.body == plan.edge_panel
        {
            fp.body = resolved_edge_panel.clone();
        }
        plan.edge_panel = resolved_edge_panel;
    }

    let kubatura_was_visible = !state.kubatura_hidden;
    let restore_visibility = |host: &mut dyn WoodHost, edge_p: &str, face_p: &str, show_kub: bool| {
        let mut items = vec![edge_p.to_string(), face_p.to_string()];
        if show_kub {
            items.push("Kubatura".to_string());
        }
        let _ = host.execute("browser.visibility", json!({ "items": items, "visible": true }));
    };

    let letter = next_lamello_letter_for_panel(edge_panel_num, face_panel_num, host);
    state.lamello_counter += 1;

    let edge_feat_name = format!("Lamello_Edge_{:02}_{letter}", edge_panel_num);
    let hex_feat_name = format!("Lamello_Hex_{:02}_{letter}", edge_panel_num);
    let face_feat_name = format!("Lamello_Face_{:02}_{letter}", face_panel_num);
    let edge_pattern_name = format!("Lamello_Pattern_Edge_{:02}_{letter}", edge_panel_num);
    let face_pattern_name = format!("Lamello_Pattern_Face_{:02}_{letter}", face_panel_num);

    // Podejście 1: Wycięcie cylindrem Ø100.4 × 10 mm + otwór Ø6 w formatce 1 (`plan.edge_panel`, formatka 2 ukryta)
    let mut hide_for_edge = vec![plan.face_panel.clone()];
    if kubatura_was_visible {
        hide_for_edge.push("Kubatura".to_string());
    }
    let _ = host.execute("browser.visibility", json!({ "items": hide_for_edge, "visible": false }));
    let _ = host.execute("browser.visibility", json!({ "items": [plan.edge_panel], "visible": true }));
    let _ = host.execute("select.clear", json!({}));

    if let Err(e) = cut_single_lamello_disc_on_panel(
        host,
        &edge_feat_name,
        &plan.edge_panel,
        &plan,
        first_center,
        plan.sign_into_edge,
    ) {
        restore_visibility(host, &plan.edge_panel, &plan.face_panel, kubatura_was_visible);
        return Err(e);
    }

    // Otwór Ø6 w środku szerokości gniazda, od strony zaznaczonej krawędzi, odsunięty o 7.5 mm od brzegu
    let round3 = |v: f64| (v * 1000.0).round() / 1000.0;
    let mut hex_pos = [0.0; 3];
    hex_pos[plan.edge_axis] = round3(first_center);
    hex_pos[plan.width_axis] = round3(plan.hex_start_w);
    hex_pos[plan.drill_axis] = round3(plan.contact_d + plan.sign_into_edge * 7.5);
    let mut hex_dir = [0.0; 3];
    hex_dir[plan.width_axis] = plan.hex_dir_w;

    if let Err(e) = host.execute(
        "solid.hole",
        json!({
            "name": hex_feat_name,
            "position": hex_pos,
            "direction": hex_dir,
            "diameter": 6.0,
            "depth": round3(plan.z_offset_eff),
            "targets": [plan.edge_panel]
        }),
    ) {
        restore_visibility(host, &plan.edge_panel, &plan.face_panel, kubatura_was_visible);
        return Err(format!(
            "Failed to drill Lamello Ø6 hole in '{}': {e}",
            plan.edge_panel
        ));
    }

    if plan.count >= 2 {
        let _ = host.execute(
            "solid.pattern.rectangular",
            json!({
                "name": edge_pattern_name,
                "features": [edge_feat_name, hex_feat_name],
                "dir1": plan.pattern_dir,
                "count1": plan.count,
                "spacing1": plan.spacing,
            }),
        );
    }

    // Podejście 2: Wycięcie cylindrem Ø100.4 × 10 mm w przylegającej formatce 2 (`plan.face_panel`, formatka 1 ukryta)
    let _ = host.execute("browser.visibility", json!({ "items": [plan.edge_panel], "visible": false }));
    let _ = host.execute("browser.visibility", json!({ "items": [plan.face_panel], "visible": true }));
    let _ = host.execute("select.clear", json!({}));

    if let Err(e) = cut_single_lamello_disc_on_panel(
        host,
        &face_feat_name,
        &plan.face_panel,
        &plan,
        first_center,
        plan.sign_into_face,
    ) {
        restore_visibility(host, &plan.edge_panel, &plan.face_panel, kubatura_was_visible);
        return Err(e);
    }

    if plan.count >= 2 {
        let _ = host.execute(
            "solid.pattern.rectangular",
            json!({
                "name": face_pattern_name,
                "features": [face_feat_name],
                "dir1": plan.pattern_dir,
                "count1": plan.count,
                "spacing1": plan.spacing,
            }),
        );
    }

    restore_visibility(host, &plan.edge_panel, &plan.face_panel, kubatura_was_visible);

    state.lamello_last_edge_pick = Some(edge);
    state.lamello_last_face_pick = face_pick;
    state.lamello_edge_pick = None;
    state.lamello_face_pick = None;
    state.ghost_lamello.clear();
    state.lamello_active_slot = 0;
    host.enable_edge_selection();

    Ok(format!(
        "Added {}× Lamello P-14 (Ø{:.1} × {:.0} mm, depth {:.0} mm) [#{:02}_{}] in '{}' & '{}'. Pick next pair.",
        plan.count,
        LAMELLO_BLADE_DIA_MM,
        LAMELLO_SLOT_WIDTH_MM,
        LAMELLO_SLOT_DEPTH_MM,
        face_panel_num,
        letter,
        plan.edge_panel,
        plan.face_panel
    ))
}


/// Definition of a single drilling hole for a hinge feature.
#[derive(Clone, Debug, PartialEq)]
pub struct HingeHoleDef {
    pub position: [f64; 3],  // punkt na powierzchni bryły
    pub direction: [f64; 3], // wektor w głąb bryły
    pub diameter: f64,
    pub depth: f64,
    pub target_body: String,
    pub label: String,
}

/// A complete hinge set: 3 holes on the front door (cup + 2 fixings) and 2 holes on the side panel (mounting plate).
#[derive(Clone, Debug, PartialEq)]
pub struct HingeInstance {
    pub cup_hole: HingeHoleDef,
    pub cup_fixing_holes: Vec<HingeHoleDef>,
    pub plate_holes: Vec<HingeHoleDef>,
}

/// Plan for drilling hinges across front and side panels.
#[derive(Clone, Debug, PartialEq)]
pub struct HingePlan {
    pub front_body: String,
    pub side_body: String,
    pub instances: Vec<HingeInstance>,
    pub pattern_dir: [f64; 3],
    pub spacing: f64,
    pub count: u32,
    pub height_span: f64,
}

/// Rozpoznana płaszczyzna, zwrot normalnej i współrzędna ściany prostopadłościennej formatki.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedPanelFace {
    pub axis: usize,        // 0=X, 1=Y, 2=Z
    pub normal_sign: f64,   // +1.0 lub -1.0 (wektor na zewnątrz bryły)
    pub surface_coord: f64, // współrzędna płaszczyzny ściany
}

/// Rozpoznaje płaszczyznę i wektor normalny ściany, korzystając najpierw z dokładnego wektora normalnego
/// z jądra B-rep (`model.faces`), a w razie potrzeby z fallbacku opartego na znormalizowanej odległości do środka formatki.
pub fn resolve_panel_face(
    panel: &PanelBox,
    pick: &crate::FacePick,
    host: &mut dyn WoodHost,
) -> ResolvedPanelFace {
    let (_norm, axis_name, sign) = crate::geometry::resolve_face_normal(&pick.body, pick.index, pick.point, host);
    let axis = match axis_name {
        "X" => 0,
        "Y" => 1,
        _ => 2,
    };
    if sign.abs() > 0.05 {
        let surface_coord = if sign > 0.0 {
            panel.max[axis]
        } else {
            panel.min[axis]
        };
        if (pick.point[axis] - surface_coord).abs() <= 2.0 {
            return ResolvedPanelFace {
                axis,
                normal_sign: sign,
                surface_coord,
            };
        }
    }

    resolve_face_from_box(panel, pick.point)
}

/// Rozpoznaje, na której z 6 płaszczyzn prostopadłościanu formatki leży wskazany punkt `pick_pt`.
/// Wykorzystuje znormalizowaną odległość od środka prostopadłościanu (score = 1.0 dokładnie na ścianie),
/// dzięki czemu kliknięcia blisko krawędzi formatki nie zakłócają wyboru osi normalnej.
pub fn resolve_face_from_box(panel: &PanelBox, pick_pt: [f64; 3]) -> ResolvedPanelFace {
    let mut best_axis = 0;
    let mut best_sign = 1.0;
    let mut best_coord = panel.max[0];
    let mut best_score = -f64::INFINITY;

    for a in 0..3 {
        let span = (panel.max[a] - panel.min[a]).abs();
        if span < 1e-4 {
            continue;
        }
        let half_span = span * 0.5;
        let center = (panel.min[a] + panel.max[a]) * 0.5;

        let score_min = (center - pick_pt[a]) / half_span;
        let score_max = (pick_pt[a] - center) / half_span;

        if score_min > best_score {
            best_score = score_min;
            best_axis = a;
            best_sign = -1.0;
            best_coord = panel.min[a];
        }
        if score_max > best_score {
            best_score = score_max;
            best_axis = a;
            best_sign = 1.0;
            best_coord = panel.max[a];
        }
    }

    ResolvedPanelFace {
        axis: best_axis,
        normal_sign: best_sign,
        surface_coord: best_coord,
    }
}

pub fn compute_hinge_plan(
    front_pick: &crate::FacePick,
    side_pick: &crate::FacePick,
    standard: &crate::HingeStandard,
    _cup_fixing: &str,
    _plate_screw: &str,
    offset_z: f64,
    count: u32,
    boxes: &[PanelBox],
    host: &mut dyn WoodHost,
) -> Result<HingePlan, String> {
    if front_pick.body == side_pick.body {
        return Err("Same panel selected for both front and side. Pick the front back face first, then the cabinet side panel.".into());
    }

    let front_box = boxes
        .iter()
        .find(|b| b.name == front_pick.body)
        .or_else(|| {
            boxes
                .iter()
                .filter(|b| b.dist_to_point(front_pick.point) <= 10.0)
                .min_by(|a, b| {
                    a.dist_to_point(front_pick.point)
                        .partial_cmp(&b.dist_to_point(front_pick.point))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .ok_or_else(|| format!("Front body '{}' not found.", front_pick.body))?;

    let side_box = boxes
        .iter()
        .find(|b| b.name == side_pick.body)
        .or_else(|| {
            boxes
                .iter()
                .filter(|b| b.dist_to_point(side_pick.point) <= 10.0)
                .min_by(|a, b| {
                    a.dist_to_point(side_pick.point)
                        .partial_cmp(&b.dist_to_point(side_pick.point))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .ok_or_else(|| format!("Side body '{}' not found.", side_pick.body))?;

    // Wyznacz dokładnie płaszczyznę i wektor normalny wskazanej ściany frontu i boku
    let front_face = resolve_panel_face(front_box, front_pick, host);
    let side_face = resolve_panel_face(side_box, side_pick, host);

    if front_face.axis == side_face.axis {
        return Err("Selected faces must be perpendicular (Face 1: front back face, Face 2: cabinet side).".into());
    }

    // Wektor wiercenia w głąb materiału jest ściśle prostopadły do wskazanej ściany
    let mut dir_into_front = [0.0; 3];
    dir_into_front[front_face.axis] = -front_face.normal_sign;

    let mut dir_into_side = [0.0; 3];
    dir_into_side[side_face.axis] = -side_face.normal_sign;

    // Oś wysokości mebla (pion) to trzecia prostopadła oś
    let height_axis = [0, 1, 2]
        .into_iter()
        .find(|&a| a != front_face.axis && a != side_face.axis)
        .unwrap_or(2);

    let mut pattern_dir = [0.0; 3];
    pattern_dir[height_axis] = 1.0;

    let z_min = front_box.min[height_axis];
    let z_max = front_box.max[height_axis];
    let height_span = (z_max - z_min).abs();
    if height_span < 60.0 {
        return Err(format!("Selected front is too short ({:.1} mm) for hinge placement.", height_span));
    }

    let n_hinges = count.max(2);
    let eff_offset = if 2.0 * offset_z >= height_span - 20.0 {
        (height_span * 0.15).max(25.0)
    } else {
        offset_z.max(25.0)
    };

    let z_first = z_min + eff_offset;
    let z_last = z_max - eff_offset;
    let spacing = if n_hinges > 1 {
        (z_last - z_first) / f64::from(n_hinges - 1)
    } else {
        0.0
    };

    // Oś szerokości frontu to oś normalnej boku (side_face.axis)
    let width_axis = side_face.axis;

    // Która pionowa krawędź frontu styka się z bokiem korpusu
    let d_w_min = (front_box.min[width_axis] - side_face.surface_coord).abs();
    let d_w_max = (front_box.max[width_axis] - side_face.surface_coord).abs();
    let (w_hinge, u_door_in) = if d_w_min < d_w_max {
        (front_box.min[width_axis], 1.0)
    } else {
        (front_box.max[width_axis], -1.0)
    };

    // Oś głębokości boku to oś normalnej frontu (front_face.axis)
    let depth_axis = front_face.axis;

    // Przednia krawędź boku korpusu (tam gdzie przylega front)
    let d_depth_min = (side_box.min[depth_axis] - front_face.surface_coord).abs();
    let d_depth_max = (side_box.max[depth_axis] - front_face.surface_coord).abs();
    let (depth_front_edge, u_side_depth) = if d_depth_min < d_depth_max {
        (side_box.min[depth_axis], 1.0)
    } else {
        (side_box.max[depth_axis], -1.0)
    };

    let center_from_edge = standard.front.cup_diameter_mm * 0.5 + standard.front.edge_distance_k_mm;
    let fix_diam = standard.front.screw_hole_diameter_mm;
    let fix_depth = standard.front.screw_hole_depth_mm;
    let half_screw_x = standard.front.screw_spacing_x_mm * 0.5;

    let plate_dist_from_edge = standard.side.edge_distance_mm;
    let plate_depth_coord = depth_front_edge + u_side_depth * plate_dist_from_edge;
    let plate_diam = standard.side.screw_hole_diameter_mm;
    let plate_depth = standard.side.screw_hole_depth_mm;
    let half_z = standard.side.spacing_z_mm * 0.5;

    let round3 = |v: f64| (v * 1000.0).round() / 1000.0;

    let mut instances = Vec::new();
    for i in 0..n_hinges {
        let z_i = z_first + f64::from(i) * spacing;

        // 1. Puszka zawiasu na froncie
        let mut p_cup = [0.0; 3];
        p_cup[height_axis] = round3(z_i);
        p_cup[width_axis] = round3(w_hinge + u_door_in * center_from_edge);
        p_cup[front_face.axis] = round3(front_face.surface_coord);

        let cup_hole = HingeHoleDef {
            position: p_cup,
            direction: dir_into_front,
            diameter: standard.front.cup_diameter_mm,
            depth: standard.front.cup_depth_mm,
            target_body: front_box.name.clone(),
            label: format!("Cup dia {:.0} #{}", standard.front.cup_diameter_mm, i + 1),
        };

        // 2. Dwa otwory mocujące puszkę na froncie (wkręty)
        let mut cup_fixing_holes = Vec::new();
        for sign in [-1.0, 1.0] {
            let mut p_fix = p_cup;
            p_fix[height_axis] = round3(z_i + sign * half_screw_x);
            p_fix[width_axis] = round3(p_cup[width_axis] + u_door_in * standard.front.screw_offset_y_mm);
            cup_fixing_holes.push(HingeHoleDef {
                position: p_fix,
                direction: dir_into_front,
                diameter: fix_diam,
                depth: fix_depth,
                target_body: front_box.name.clone(),
                label: String::new(),
            });
        }

        // 3. Dwa otwory pod prowadnik na boku korpusu (wkręty)
        let mut plate_holes = Vec::new();
        for sign in [-1.0, 1.0] {
            let mut p_plate = [0.0; 3];
            p_plate[height_axis] = round3(z_i + sign * half_z);
            p_plate[depth_axis] = round3(plate_depth_coord);
            p_plate[side_face.axis] = round3(side_face.surface_coord);
            plate_holes.push(HingeHoleDef {
                position: p_plate,
                direction: dir_into_side,
                diameter: plate_diam,
                depth: plate_depth,
                target_body: side_box.name.clone(),
                label: format!("Plate #{}", i + 1),
            });
        }

        instances.push(HingeInstance {
            cup_hole,
            cup_fixing_holes,
            plate_holes,
        });
    }

    Ok(HingePlan {
        front_body: front_box.name.clone(),
        side_body: side_box.name.clone(),
        instances,
        pattern_dir,
        spacing: round3(spacing),
        count: n_hinges,
        height_span: round3(height_span),
    })
}

pub fn create_hinges(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let Some(front_pick) = state.hinge_face_picks[0].clone() else {
        return Err("Step 1: Pick the front back face (inner side of the door) in the 3D viewport first.".into());
    };
    let Some(side_pick) = state.hinge_face_picks[1].clone() else {
        return Err("Step 2: Pick the cabinet side panel (inner side face) in the 3D viewport.".into());
    };

    let lib = crate::HingeLibrary::load()?;
    let standard = lib
        .find_standard(&state.hinge_standard_id)
        .ok_or_else(|| "Nie znaleziono wybranego standardu zawiasu.".to_string())?
        .clone();

    let offset_z = parse_f64_or(&state.hinge_offset_x, 100.0).max(20.0);
    let count = parse_f64_or(&state.hinge_count, 2.0).round().clamp(2.0, 50.0) as u32;

    let boxes = extract_panel_boxes(host);
    let mut plan = compute_hinge_plan(
        &front_pick,
        &side_pick,
        &standard,
        &state.hinge_cup_fixing,
        &state.hinge_plate_screw,
        offset_z,
        count,
        &boxes,
        host,
    )?;

    let _ = host.execute("component.activate", json!({ "component": "root" }));

    // 1. Upewnij się, że formatka frontu (ściana, na której dodawane są otwory na puszki) ma unikalny numer formatki
    let (resolved_front, front_panel_num) =
        ensure_body_is_numbered_panel(&plan.front_body, state, host);
    if resolved_front != plan.front_body {
        for inst in &mut plan.instances {
            inst.cup_hole.target_body = resolved_front.clone();
            for fix in &mut inst.cup_fixing_holes {
                fix.target_body = resolved_front.clone();
            }
        }
        plan.front_body = resolved_front;
    }

    // 2. Upewnij się, że formatka boku również ma nadany numer formatki
    let (resolved_side, _side_panel_num) =
        ensure_body_is_numbered_panel(&plan.side_body, state, host);
    if resolved_side != plan.side_body {
        for inst in &mut plan.instances {
            for plate in &mut inst.plate_holes {
                plate.target_body = resolved_side.clone();
            }
        }
        plan.side_body = resolved_side;
    }

    let hid_kubatura_temp = !state.kubatura_hidden;
    if hid_kubatura_temp {
        let _ = host.execute(
            "browser.visibility",
            json!({ "items": ["Kubatura"], "visible": false }),
        );
    }

    state.hinge_counter += 1;
    let mut used_letters = Vec::new();

    for inst in &plan.instances {
        let letter = next_hinge_letter_for_panel(front_panel_num, &used_letters, host);
        used_letters.push(letter.clone());

        // 1. Puszka: Otwor_Puszka_<NR>_<Litera>
        let cup_name = format!("Hole_Cup_{:02}_{letter}", front_panel_num);
        let res_cup = host.execute(
            "solid.hole",
            json!({
                "name": cup_name,
                "position": inst.cup_hole.position,
                "direction": inst.cup_hole.direction,
                "diameter": inst.cup_hole.diameter,
                "depth": inst.cup_hole.depth,
                "targets": [inst.cup_hole.target_body],
            }),
        );
        if let Err(e) = res_cup {
            if hid_kubatura_temp {
                let _ = host.execute("browser.visibility", json!({ "items": ["Kubatura"], "visible": true }));
            }
            return Err(format!("Failed to drill hinge cup in '{}': {e}", inst.cup_hole.target_body));
        }

        // 2. Cup fixing holes: Hole_Screw_<NO>_<Letter>_1, Hole_Screw_<NO>_<Letter>_2
        for (f_i, fix) in inst.cup_fixing_holes.iter().enumerate() {
            let fix_name = format!("Hole_Screw_{:02}_{letter}_{}", front_panel_num, f_i + 1);
            let _ = host.execute(
                "solid.hole",
                json!({
                    "name": fix_name,
                    "position": fix.position,
                    "direction": fix.direction,
                    "diameter": fix.diameter,
                    "depth": fix.depth,
                    "targets": [fix.target_body],
                }),
            );
        }

        // 3. Prowadnik na boku: Otwor_prowadnik_<NR>_<Litera>_1, Otwor_prowadnik_<NR>_<Litera>_2
        for (p_i, plate) in inst.plate_holes.iter().enumerate() {
            let plate_name = format!("Hole_Plate_{:02}_{letter}_{}", front_panel_num, p_i + 1);
            let res_plate = host.execute(
                "solid.hole",
                json!({
                    "name": plate_name,
                    "position": plate.position,
                    "direction": plate.direction,
                    "diameter": plate.diameter,
                    "depth": plate.depth,
                    "targets": [plate.target_body],
                }),
            );
            if let Err(e) = res_plate {
                if hid_kubatura_temp {
                    let _ = host.execute("browser.visibility", json!({ "items": ["Kubatura"], "visible": true }));
                }
                return Err(format!("Failed to drill mounting plate hole in '{}': {e}", plate.target_body));
            }
        }
    }

    if hid_kubatura_temp {
        let _ = host.execute(
            "browser.visibility",
            json!({ "items": ["Kubatura"], "visible": true }),
        );
    }

    state.hinge_face_picks = [None, None];
    state.ghost_cylinders = None;
    state.hinge_active_slot = 0;
    host.enable_face_selection();

    let letters_str = used_letters.join(", ");
    Ok(format!(
        "Dodano {}× Zawias '{}' [nr {:02}: {}] (front: '{}', bok: '{}', puszka fi {:.0} mm, prowadnik System 32).",
        plan.count,
        standard.name,
        front_panel_num,
        letters_str,
        plan.front_body,
        plan.side_body,
        standard.front.cup_diameter_mm
    ))
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawerHoleDef {
    pub position: [f64; 3],
    pub direction: [f64; 3],
    pub diameter: f64,
    pub depth: f64,
    pub target_body: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawerPlan {
    pub side1_body: String,
    pub side2_body: String,
    pub bottom_ref_body: String,
    pub front_body: String,
    pub lw_mm: f64,
    pub nl_mm: f64,
    pub is_inset_front: bool,
    pub inset_setback_mm: f64,
    pub avail_depth_mm: f64,
    pub required_depth_mm: f64,
    pub avail_height_mm: f64,
    pub required_height_mm: f64,
    pub fits_depth: bool,
    pub fits_height: bool,
    pub envelope_min: [f64; 3],
    pub envelope_max: [f64; 3],
    pub envelope_plane: &'static str,
    pub width_axis: usize,
    pub depth_axis: usize,
    pub height_axis: usize,
    pub bottom_panel_min: [f64; 3],
    pub bottom_panel_max: [f64; 3],
    pub bottom_panel_dims_mm: (f64, f64, f64),
    pub back_panel_min: [f64; 3],
    pub back_panel_max: [f64; 3],
    pub back_panel_dims_mm: (f64, f64, f64),
    pub side1_holes: Vec<DrawerHoleDef>,
    pub side2_holes: Vec<DrawerHoleDef>,
    pub front_holes: Vec<DrawerHoleDef>,
    pub back_holes: Vec<DrawerHoleDef>,
}

fn find_panel_box<'a>(boxes: &'a [PanelBox], pick: &crate::FacePick, role: &str) -> Result<&'a PanelBox, String> {
    boxes
        .iter()
        .find(|b| b.name == pick.body)
        .or_else(|| {
            boxes
                .iter()
                .filter(|b| b.dist_to_point(pick.point) <= 10.0)
                .min_by(|a, b| {
                    a.dist_to_point(pick.point)
                        .partial_cmp(&b.dist_to_point(pick.point))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .ok_or_else(|| format!("Body for '{role}' ('{}') not found.", pick.body))
}

pub fn compute_drawer_plan(
    side1_pick: &crate::FacePick,
    side2_pick: &crate::FacePick,
    bottom_pick: &crate::FacePick,
    front_pick: &crate::FacePick,
    system: &crate::DrawerSystem,
    height_var: &crate::DrawerHeightVariant,
    length_var: &crate::DrawerLengthVariant,
    lift_z: f64,
    boxes: &[PanelBox],
    host: &mut dyn WoodHost,
) -> Result<DrawerPlan, String> {
    if side1_pick.body == side2_pick.body {
        return Err("Same cabinet side picked twice. Pick the inner face of the left and right side panels.".into());
    }

    let side1_box = find_panel_box(boxes, side1_pick, "1. Lewy bok")?;
    let side2_box = find_panel_box(boxes, side2_pick, "2. Prawy bok")?;
    let bottom_box = find_panel_box(boxes, bottom_pick, "3. Wieniec dolny")?;
    let front_box = find_panel_box(boxes, front_pick, "4. Plecy frontu")?;

    let side1_face = resolve_panel_face(side1_box, side1_pick, host);
    let side2_face = resolve_panel_face(side2_box, side2_pick, host);
    let bottom_face = resolve_panel_face(bottom_box, bottom_pick, host);
    let front_face = resolve_panel_face(front_box, front_pick, host);

    if side1_face.axis != side2_face.axis {
        return Err("Cabinet side faces (Step 1 and 2) must be parallel to each other.".into());
    }
    let width_axis = side1_face.axis;

    if bottom_face.axis == width_axis {
        return Err("Bottom panel top face (Step 3) must be perpendicular to the cabinet sides.".into());
    }
    let height_axis = bottom_face.axis;

    if front_face.axis == width_axis || front_face.axis == height_axis {
        return Err("Front back face (Step 4) must be perpendicular to both the sides and the bottom panel.".into());
    }
    let depth_axis = front_face.axis;

    let w_min = side1_face.surface_coord.min(side2_face.surface_coord);
    let w_max = side1_face.surface_coord.max(side2_face.surface_coord);
    let lw_mm = (w_max - w_min).abs();
    if lw_mm <= system.back_width_deduction_mm + 20.0 {
        return Err(format!(
            "Internal cabinet width LW = {:.1} mm is too small for the selected drawer system.",
            lw_mm
        ));
    }

    // Kierunek wiercenia w głąb boku 1 i boku 2 (od wewnętrznej ściany na zewnątrz korpusu)
    let mut dir_into_side1 = [0.0; 3];
    let mut dir_into_side2 = [0.0; 3];
    if side1_face.surface_coord < side2_face.surface_coord {
        dir_into_side1[width_axis] = -1.0;
        dir_into_side2[width_axis] = 1.0;
    } else {
        dir_into_side1[width_axis] = 1.0;
        dir_into_side2[width_axis] = -1.0;
    }

    // Kierunek w górę (od górnej powierzchni wieńca dolnego w stronę wnętrza szafki)
    let side_mid_h = 0.5 * (side1_box.min[height_axis] + side1_box.max[height_axis]);
    let u_up = if side_mid_h >= bottom_face.surface_coord {
        1.0
    } else {
        -1.0
    };
    let z_base = bottom_face.surface_coord + u_up * lift_z;

    // Kierunek w głąb korpusu (od pleców frontu w głąb boków)
    let side_mid_d = 0.5 * (side1_box.min[depth_axis] + side1_box.max[depth_axis]);
    let u_depth_in = if side_mid_d >= front_face.surface_coord {
        1.0
    } else {
        -1.0
    };
    let mut dir_into_front = [0.0; 3];
    dir_into_front[depth_axis] = -u_depth_in;

    // Przednia i tylna krawędź boków korpusu
    let (depth_cabinet_front_edge, depth_cabinet_back_edge) = if u_depth_in > 0.0 {
        (
            side1_box.min[depth_axis].max(side2_box.min[depth_axis]),
            side1_box.max[depth_axis].min(side2_box.max[depth_axis]),
        )
    } else {
        (
            side1_box.max[depth_axis].min(side2_box.max[depth_axis]),
            side1_box.min[depth_axis].max(side2_box.min[depth_axis]),
        )
    };

    // Sprawdź czy w głębi korpusu między bokami znajduje się płyta pleców korpusu / ogranicznik głębokości
    let mut effective_back_limit = depth_cabinet_back_edge;
    for b in boxes {
        if b.name == "Kubatura"
            || b.name == side1_box.name
            || b.name == side2_box.name
            || b.name == bottom_box.name
            || b.name == front_box.name
        {
            continue;
        }
        if b.thin_axis() != depth_axis {
            continue;
        }
        let overlaps_w = b.max[width_axis] > w_min + 2.0 && b.min[width_axis] < w_max - 2.0;
        let z_low = z_base.min(z_base + u_up * height_var.min_space_height_mm);
        let z_high = z_base.max(z_base + u_up * height_var.min_space_height_mm);
        let overlaps_h = b.max[height_axis] > z_low + 2.0 && b.min[height_axis] < z_high - 2.0;
        if overlaps_w && overlaps_h {
            let inner_face_d = if u_depth_in > 0.0 {
                b.min[depth_axis]
            } else {
                b.max[depth_axis]
            };
            if (inner_face_d - depth_cabinet_front_edge) * u_depth_in > 50.0 {
                if u_depth_in > 0.0 {
                    effective_back_limit = effective_back_limit.min(inner_face_d);
                } else {
                    effective_back_limit = effective_back_limit.max(inner_face_d);
                }
            }
        }
    }

    // Wykryj czy front jest wpuszczany (plecy frontu są cofnięte w głąb względem przedniej krawędzi boku)
    let front_setback_raw = (front_face.surface_coord - depth_cabinet_front_edge) * u_depth_in;
    let (is_inset_front, inset_setback_mm, effective_front_ref) = if front_setback_raw > 2.0 {
        (true, front_setback_raw, front_face.surface_coord)
    } else {
        (false, 0.0, depth_cabinet_front_edge)
    };

    let nl_mm = length_var.nl_mm;
    let avail_depth_mm = ((effective_back_limit - depth_cabinet_front_edge) * u_depth_in).max(0.0);
    let required_depth_mm = nl_mm + system.min_depth_clearance_mm + inset_setback_mm;
    let fits_depth = avail_depth_mm + 0.05 >= required_depth_mm;

    // Sprawdź dostępną wysokość od z_base w górę (do końca boków lub do najbliższego wieńca górnego / półki)
    let side_top_limit = if u_up > 0.0 {
        side1_box.max[height_axis].min(side2_box.max[height_axis])
    } else {
        side1_box.min[height_axis].max(side2_box.min[height_axis])
    };
    let mut effective_top_limit = side_top_limit;
    for b in boxes {
        if b.name == "Kubatura"
            || b.name == side1_box.name
            || b.name == side2_box.name
            || b.name == bottom_box.name
            || b.name == front_box.name
        {
            continue;
        }
        if b.thin_axis() != height_axis {
            continue;
        }
        let overlaps_w = b.max[width_axis] > w_min + 2.0 && b.min[width_axis] < w_max - 2.0;
        let d_start = effective_front_ref.min(effective_front_ref + u_depth_in * nl_mm);
        let d_end = effective_front_ref.max(effective_front_ref + u_depth_in * nl_mm);
        let overlaps_d = b.max[depth_axis] > d_start + 2.0 && b.min[depth_axis] < d_end - 2.0;
        if overlaps_w && overlaps_d {
            let under_face_h = if u_up > 0.0 {
                b.min[height_axis]
            } else {
                b.max[height_axis]
            };
            if (under_face_h - z_base) * u_up > 10.0 {
                if u_up > 0.0 {
                    effective_top_limit = effective_top_limit.min(under_face_h);
                } else {
                    effective_top_limit = effective_top_limit.max(under_face_h);
                }
            }
        }
    }

    let avail_height_mm = ((effective_top_limit - z_base) * u_up).max(0.0);
    let required_height_mm = height_var.min_space_height_mm;
    let fits_height = avail_height_mm + 0.05 >= required_height_mm;

    let round3 = |v: f64| (v * 1000.0).round() / 1000.0;

    // 1. Bryła maksymalnych wymiarów zajmowanych przez szufladę bez frontu (dla zielonego ducha)
    let env_d0 = effective_front_ref;
    let env_d1 = effective_front_ref + u_depth_in * (nl_mm + system.min_depth_clearance_mm);
    let env_h0 = z_base;
    let env_h1 = z_base + u_up * required_height_mm;

    let mut envelope_min = [0.0; 3];
    let mut envelope_max = [0.0; 3];
    envelope_min[width_axis] = round3(w_min.min(w_max));
    envelope_max[width_axis] = round3(w_min.max(w_max));
    envelope_min[depth_axis] = round3(env_d0.min(env_d1));
    envelope_max[depth_axis] = round3(env_d0.max(env_d1));
    envelope_min[height_axis] = round3(env_h0.min(env_h1));
    envelope_max[height_axis] = round3(env_h0.max(env_h1));

    let envelope_plane = match height_axis {
        0 => "YZ",
        1 => "XZ",
        _ => "XY",
    };

    // 2. Wymiary i położenie formatek: Bottom (A) oraz Back (B)
    let board_t = system.board_thickness_mm;
    let bottom_w = round1(lw_mm - system.bottom_width_deduction_mm);
    let bottom_d = round1(nl_mm - system.bottom_depth_deduction_mm);
    let back_w = round1(lw_mm - system.back_width_deduction_mm);
    let back_h = round1(height_var.back_height_mm);

    if bottom_w <= 10.0 || bottom_d <= 10.0 || back_w <= 10.0 || back_h <= 10.0 {
        return Err("Computed bottom or back panel dimensions are invalid (<= 10 mm).".into());
    }

    let w_center = 0.5 * (w_min + w_max);
    // Dno zaczyna się 12 mm za płaszczyzną frontową szuflady i kończy na 12 + (NL - 24) = NL - 12 mm
    let bottom_front_coord = effective_front_ref + u_depth_in * 12.0;
    let bottom_back_coord = bottom_front_coord + u_depth_in * bottom_d;
    let bottom_z0 = z_base + u_up * system.bottom_panel_z_from_bottom_mm;
    let bottom_z1 = bottom_z0 + u_up * board_t;

    let mut bottom_panel_min = [0.0; 3];
    let mut bottom_panel_max = [0.0; 3];
    bottom_panel_min[width_axis] = round3(w_center - 0.5 * bottom_w);
    bottom_panel_max[width_axis] = round3(w_center + 0.5 * bottom_w);
    bottom_panel_min[depth_axis] = round3(bottom_front_coord.min(bottom_back_coord));
    bottom_panel_max[depth_axis] = round3(bottom_front_coord.max(bottom_back_coord));
    bottom_panel_min[height_axis] = round3(bottom_z0.min(bottom_z1));
    bottom_panel_max[height_axis] = round3(bottom_z0.max(bottom_z1));

    // Plecy stoją na dnie (od bottom_z1 do bottom_z1 + back_h) i licują z tylną krawędzią dna (bottom_back_coord)
    let back_front_coord = bottom_back_coord - u_depth_in * board_t;
    let back_back_coord = bottom_back_coord;
    let back_z0 = bottom_z1;
    let back_z1 = back_z0 + u_up * back_h;

    let mut back_panel_min = [0.0; 3];
    let mut back_panel_max = [0.0; 3];
    back_panel_min[width_axis] = round3(w_center - 0.5 * back_w);
    back_panel_max[width_axis] = round3(w_center + 0.5 * back_w);
    back_panel_min[depth_axis] = round3(back_front_coord.min(back_back_coord));
    back_panel_max[depth_axis] = round3(back_front_coord.max(back_back_coord));
    back_panel_min[height_axis] = round3(back_z0.min(back_z1));
    back_panel_max[height_axis] = round3(back_z0.max(back_z1));

    let hole_dia = system.screw_hole_diameter_mm;
    let hole_depth = system.screw_hole_depth_mm;

    // 3. Otwory w bokach korpusu pod prowadnice (względem z_base + runner_hole_z_from_bottom_mm)
    let runner_z = round3(z_base + u_up * system.runner_hole_z_from_bottom_mm);
    let runner_first_d = effective_front_ref + u_depth_in * system.runner_first_hole_from_edge_mm;
    let mut side1_holes = Vec::new();
    let mut side2_holes = Vec::new();

    for (idx, &dx) in length_var.runner_hole_x_offsets_mm.iter().enumerate() {
        let d_coord = round3(runner_first_d + u_depth_in * dx);

        let mut p1 = [0.0; 3];
        p1[width_axis] = round3(side1_face.surface_coord);
        p1[depth_axis] = d_coord;
        p1[height_axis] = runner_z;
        side1_holes.push(DrawerHoleDef {
            position: p1,
            direction: dir_into_side1,
            diameter: hole_dia,
            depth: hole_depth,
            target_body: side1_box.name.clone(),
            label: if idx == 0 { "Slide L".to_string() } else { String::new() },
        });

        let mut p2 = [0.0; 3];
        p2[width_axis] = round3(side2_face.surface_coord);
        p2[depth_axis] = d_coord;
        p2[height_axis] = runner_z;
        side2_holes.push(DrawerHoleDef {
            position: p2,
            direction: dir_into_side2,
            diameter: hole_dia,
            depth: hole_depth,
            target_body: side2_box.name.clone(),
            label: if idx == 0 { "Slide R".to_string() } else { String::new() },
        });
    }

    // 4. Otwory w plecach frontu (15.5 mm od wewnętrznych ścian boków, od z_base + 47.5 mm)
    let mut front_holes = Vec::new();
    let front_w_left = round3(w_min + system.front_hole_side_offset_mm);
    let front_w_right = round3(w_max - system.front_hole_side_offset_mm);
    let front_first_z = z_base + u_up * system.front_first_hole_z_from_bottom_mm;

    for (idx, &dz) in height_var.front_hole_z_offsets_mm.iter().enumerate() {
        let z_coord = round3(front_first_z + u_up * dz);
        for (col_idx, w_coord) in [front_w_left, front_w_right].into_iter().enumerate() {
            let mut pf = [0.0; 3];
            pf[width_axis] = w_coord;
            pf[depth_axis] = round3(front_face.surface_coord);
            pf[height_axis] = z_coord;
            front_holes.push(DrawerHoleDef {
                position: pf,
                direction: dir_into_front,
                diameter: hole_dia,
                depth: hole_depth,
                target_body: front_box.name.clone(),
                label: if idx == 0 && col_idx == 0 {
                    "Front".to_string()
                } else {
                    String::new()
                },
            });
        }
    }

    // 5. Otwory w ściance tylnej (Back: 9 mm od bocznych krawędzi pleców, liczone od górnej krawędzi pleców w dół)
    let mut back_holes = Vec::new();
    let back_w_left = round3((w_center - 0.5 * back_w) + system.back_hole_side_offset_mm);
    let back_w_right = round3((w_center + 0.5 * back_w) - system.back_hole_side_offset_mm);
    let mut dir_into_back = [0.0; 3];
    dir_into_back[depth_axis] = -u_depth_in;

    for (idx, &top_off) in height_var.back_hole_top_offsets_mm.iter().enumerate() {
        let z_coord = round3(back_z1 - u_up * top_off);
        for (col_idx, w_coord) in [back_w_left, back_w_right].into_iter().enumerate() {
            let mut pb = [0.0; 3];
            pb[width_axis] = w_coord;
            pb[depth_axis] = round3(back_back_coord);
            pb[height_axis] = z_coord;
            back_holes.push(DrawerHoleDef {
                position: pb,
                direction: dir_into_back,
                diameter: hole_dia,
                depth: hole_depth,
                target_body: String::new(),
                label: if idx == 0 && col_idx == 0 {
                    "Back".to_string()
                } else {
                    String::new()
                },
            });
        }
    }

    Ok(DrawerPlan {
        side1_body: side1_box.name.clone(),
        side2_body: side2_box.name.clone(),
        bottom_ref_body: bottom_box.name.clone(),
        front_body: front_box.name.clone(),
        lw_mm: round1(lw_mm),
        nl_mm,
        is_inset_front,
        inset_setback_mm: round1(inset_setback_mm),
        avail_depth_mm: round1(avail_depth_mm),
        required_depth_mm: round1(required_depth_mm),
        avail_height_mm: round1(avail_height_mm),
        required_height_mm: round1(required_height_mm),
        fits_depth,
        fits_height,
        envelope_min,
        envelope_max,
        envelope_plane,
        width_axis,
        depth_axis,
        height_axis,
        bottom_panel_min,
        bottom_panel_max,
        bottom_panel_dims_mm: (bottom_w, bottom_d, board_t),
        back_panel_min,
        back_panel_max,
        back_panel_dims_mm: (back_w, back_h, board_t),
        side1_holes,
        side2_holes,
        front_holes,
        back_holes,
    })
}

pub fn create_drawer(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let Some(p0) = state.drawer_face_picks[0].clone() else {
        return Err("Step 1: Pick the inner face of the left cabinet side in the 3D viewport.".into());
    };
    let Some(p1) = state.drawer_face_picks[1].clone() else {
        return Err("Step 2: Pick the inner face of the right cabinet side in the 3D viewport.".into());
    };
    let Some(p2) = state.drawer_face_picks[2].clone() else {
        return Err("Step 3: Pick the top face of the bottom panel in the 3D viewport.".into());
    };
    let Some(p3) = state.drawer_face_picks[3].clone() else {
        return Err("Step 4: Pick the inner (back) face of the drawer front in the 3D viewport.".into());
    };

    let lib = crate::DrawerLibrary::load()?;
    let system = lib
        .find_system(&state.drawer_system_id)
        .ok_or_else(|| "Selected drawer system not found.".to_string())?
        .clone();
    let height_var = system
        .find_height(&state.drawer_height_id)
        .ok_or_else(|| "Selected drawer height not found.".to_string())?
        .clone();
    let nl_val = parse_f64_or(&state.drawer_nl_mm, 450.0);
    let length_var = system
        .find_length(nl_val)
        .ok_or_else(|| "Selected nominal length NL not found.".to_string())?
        .clone();
    let lift_z = parse_f64_or(&state.drawer_lift_z, 0.0).max(0.0);

    let boxes = extract_panel_boxes(host);
    let plan = compute_drawer_plan(
        &p0,
        &p1,
        &p2,
        &p3,
        &system,
        &height_var,
        &length_var,
        lift_z,
        &boxes,
        host,
    )?;

    if !plan.fits_depth {
        return Err(format!(
            "Selected drawer depth NL {:.0} mm does not fit in the cabinet! Required min. {:.0} mm, available {:.1} mm.",
            plan.nl_mm, plan.required_depth_mm, plan.avail_depth_mm
        ));
    }
    if !plan.fits_height {
        return Err(format!(
            "Selected drawer height {:.0} mm does not fit in the cabinet! Required min. {:.0} mm, available {:.1} mm.",
            height_var.drawer_height_mm, plan.required_height_mm, plan.avail_height_mm
        ));
    }

    let _ = host.execute("component.activate", json!({ "component": "root" }));

    // 1. Upewnij się, że oba boki korpusu i front mają nadane numery formatek
    let (resolved_side1, _s1_num) = ensure_body_is_numbered_panel(&plan.side1_body, state, host);
    let (resolved_side2, _s2_num) = ensure_body_is_numbered_panel(&plan.side2_body, state, host);
    let (resolved_front, front_num) = ensure_body_is_numbered_panel(&plan.front_body, state, host);

    let parsed_cab = parse_panel_name(&resolved_side1).cabinet;
    let cab = if parsed_cab.is_empty() {
        sanitize_name_segment(&state.cabinet_prefix)
    } else {
        parsed_cab
    };
    let cab = if cab.is_empty() { "S01".to_string() } else { cab };

    // Wyznacz kolejny numer szuflady w dokumencie
    let comp_list = host.execute("component.list", json!({})).unwrap_or(Value::Null);
    let comps = comp_list
        .get("components")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let existing_drawers = comps
        .iter()
        .filter(|c| {
            c.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| {
                    n.contains("_Drawer_")
                        || n.starts_with("Drawer_")
                        || n.contains("_Szuflada_")
                        || n.starts_with("Szuflada_")
                })
        })
        .count() as u32;
    let drawer_idx = existing_drawers + 1;
    state.drawer_counter = drawer_idx + 1;

    // 2. Przydziel numery formatek dla Bottom i Back zgodnie z konwencją S05_30_...
    let num_bottom = allocate_panel_number(state, host);
    let num_back = allocate_panel_number(state, host);

    let bottom_custom = format!(
        "drawer_{:02}_bottom_{}_{:.0}_h{:.0}",
        drawer_idx, system.short_code, plan.nl_mm, height_var.drawer_height_mm
    );
    let back_custom = format!(
        "drawer_{:02}_back_{}_{:.0}_h{:.0}",
        drawer_idx, system.short_code, plan.nl_mm, height_var.drawer_height_mm
    );

    let bottom_name = build_panel_name(&cab, num_bottom, &bottom_custom);
    let back_name = build_panel_name(&cab, num_back, &back_custom);
    let drawer_comp_name = format!(
        "{cab}_Drawer_{:02}_{}_{:.0}_h{:.0}",
        drawer_idx, system.short_code, plan.nl_mm, height_var.drawer_height_mm
    );

    let hid_kubatura_temp = !state.kubatura_hidden;
    if hid_kubatura_temp {
        let _ = host.execute(
            "browser.visibility",
            json!({ "items": ["Kubatura"], "visible": false }),
        );
    }

    // 3. Utwórz bryłę DNA szuflady (solid.box)
    let bottom_lx = round1(plan.bottom_panel_max[0] - plan.bottom_panel_min[0]);
    let bottom_ly = round1(plan.bottom_panel_max[1] - plan.bottom_panel_min[1]);
    let bottom_lz = round1(plan.bottom_panel_max[2] - plan.bottom_panel_min[2]);
    if let Err(e) = host.execute(
        "solid.box",
        json!({
            "name": bottom_name,
            "body_name": bottom_name,
            "corner": plan.bottom_panel_min,
            "length": bottom_lx,
            "width": bottom_ly,
            "height": bottom_lz,
            "operation": "new",
        }),
    ) {
        if hid_kubatura_temp {
            let _ = host.execute("browser.visibility", json!({ "items": ["Kubatura"], "visible": true }));
        }
        return Err(format!("Failed to create drawer bottom panel '{bottom_name}': {e}"));
    }

    // 4. Utwórz bryłę PLECÓW szuflady (solid.box)
    let back_lx = round1(plan.back_panel_max[0] - plan.back_panel_min[0]);
    let back_ly = round1(plan.back_panel_max[1] - plan.back_panel_min[1]);
    let back_lz = round1(plan.back_panel_max[2] - plan.back_panel_min[2]);
    if let Err(e) = host.execute(
        "solid.box",
        json!({
            "name": back_name,
            "body_name": back_name,
            "corner": plan.back_panel_min,
            "length": back_lx,
            "width": back_ly,
            "height": back_lz,
            "operation": "new",
        }),
    ) {
        if hid_kubatura_temp {
            let _ = host.execute("browser.visibility", json!({ "items": ["Kubatura"], "visible": true }));
        }
        return Err(format!("Failed to create drawer back panel '{back_name}': {e}"));
    }

    // 5. Wierć płytkie otwory znaczące w lewym boku, prawym boku, froncie oraz w plecach szuflady
    for (i, h) in plan.side1_holes.iter().enumerate() {
        let hole_name = format!("Hole_slide_L_{:02}_{:02}_{}", front_num, drawer_idx, i + 1);
        let _ = host.execute(
            "solid.hole",
            json!({
                "name": hole_name,
                "position": h.position,
                "direction": h.direction,
                "diameter": h.diameter,
                "depth": h.depth,
                "targets": [resolved_side1],
            }),
        );
    }

    for (i, h) in plan.side2_holes.iter().enumerate() {
        let hole_name = format!("Hole_slide_R_{:02}_{:02}_{}", front_num, drawer_idx, i + 1);
        let _ = host.execute(
            "solid.hole",
            json!({
                "name": hole_name,
                "position": h.position,
                "direction": h.direction,
                "diameter": h.diameter,
                "depth": h.depth,
                "targets": [resolved_side2],
            }),
        );
    }

    for (i, h) in plan.front_holes.iter().enumerate() {
        let hole_name = format!("Hole_drawer_front_{:02}_{:02}_{}", front_num, drawer_idx, i + 1);
        let _ = host.execute(
            "solid.hole",
            json!({
                "name": hole_name,
                "position": h.position,
                "direction": h.direction,
                "diameter": h.diameter,
                "depth": h.depth,
                "targets": [resolved_front],
            }),
        );
    }

    for (i, h) in plan.back_holes.iter().enumerate() {
        let hole_name = format!("Hole_drawer_back_{:02}_{:02}_{}", num_back, drawer_idx, i + 1);
        let _ = host.execute(
            "solid.hole",
            json!({
                "name": hole_name,
                "position": h.position,
                "direction": h.direction,
                "diameter": h.diameter,
                "depth": h.depth,
                "targets": [back_name],
            }),
        );
    }

    // 6. Utwórz dedykowany komponent szuflady i przenieś do niego OBIE formatki (Bottom + Back)
    let kub_exists = comps.iter().any(|c| {
        c.get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| n == "Kubatura" || n.starts_with("Kubatura:"))
    });
    if !kub_exists {
        let _ = host.execute(
            "component.create",
            json!({
                "name": "Kubatura",
                "parent": "root",
                "activate": false,
                "grounded": true,
            }),
        );
    }

    let _ = host.execute(
        "component.create",
        json!({
            "name": drawer_comp_name,
            "parent": "Kubatura",
            "activate": false,
            "grounded": false,
        }),
    );
    let _ = host.execute(
        "component.move_bodies",
        json!({
            "bodies": [bottom_name, back_name],
            "component": drawer_comp_name,
        }),
    );
    let _ = host.execute(
        "joint.as_built",
        json!({
            "name": format!("Joint_{drawer_comp_name}"),
            "a": "Kubatura:1",
            "b": format!("{drawer_comp_name}:1"),
            "type": "rigid",
        }),
    );
    let _ = host.execute(
        "component.metadata.set",
        json!({
            "component": drawer_comp_name,
            "values": {
                "description": format!("Drawer {} NL={:.0} H={:.0}", system.name, plan.nl_mm, height_var.drawer_height_mm),
                "part_number": drawer_comp_name,
            }
        }),
    );
    let _ = host.execute("component.activate", json!({ "component": "root" }));

    if hid_kubatura_temp {
        let _ = host.execute(
            "browser.visibility",
            json!({ "items": ["Kubatura"], "visible": true }),
        );
    }

    if !state.created_panels.contains(&bottom_name) {
        state.created_panels.push(bottom_name.clone());
    }
    if !state.created_panels.contains(&back_name) {
        state.created_panels.push(back_name.clone());
    }
    let _ = refresh_cutting_list(state, host);

    state.drawer_face_picks = [None, None, None, None];
    state.drawer_active_slot = 0;
    state.drawer_validation_info.clear();
    state.drawer_validation_error = false;
    state.ghost_box = None;
    state.ghost_cylinders = None;
    host.enable_face_selection();

    Ok(format!(
        "Created drawer in component '{}' (Bottom: '{}', Back: '{}') and drilled pilot holes (sides: 2×{}, front: {}, back: {}).",
        drawer_comp_name,
        bottom_name,
        back_name,
        plan.side1_holes.len(),
        plan.front_holes.len(),
        plan.back_holes.len(),
    ))
}

/// Zwraca dwie osie leżące w płaszczyźnie formatki (`axis1 < axis2`), prostopadłe do osi jej grubości (`thin_axis`).
pub fn panel_in_plane_axes(thin_axis: usize) -> (usize, usize) {
    match thin_axis {
        0 => (1, 2), // Y, Z
        1 => (0, 2), // X, Z
        _ => (0, 1), // X, Y
    }
}

/// Zwraca krótką polską etykietę dla indeksu brzegu `0..4` formatki o zadanej osi grubości.
pub fn narrow_edge_side_label(thin_axis: usize, side_idx: usize) -> &'static str {
    let (axis1, axis2) = panel_in_plane_axes(thin_axis);
    let (ax, is_max) = match side_idx {
        0 => (axis1, false),
        1 => (axis1, true),
        2 => (axis2, false),
        _ => (axis2, true),
    };
    match (ax, is_max) {
        (0, false) => "-X (Lewy)",
        (0, true) => "+X (Prawy)",
        (1, false) => "-Y (Front)",
        (1, true) => "+Y (Back)",
        (2, false) => "-Z (Bottom)",
        _ => "+Z (Top)",
    }
}

/// Wyznacza 4 narożniki 3D wąskiego brzegu formatki (`side_idx` w `0..4`), punkt próbkowania dla `appearance.assign`,
/// oś normalnej brzegu, zwrot normalnej (`-1.0` lub `+1.0`), oś wzdłuż której biegnie brzeg oraz jego długość w mm.
pub fn narrow_edge_quad_and_probe(
    panel: &PanelBox,
    side_idx: usize,
) -> ([[f64; 3]; 4], [f64; 3], usize, f64, usize, f64) {
    let thin_axis = panel.thin_axis();
    let (axis1, axis2) = panel_in_plane_axes(thin_axis);

    let (normal_axis, normal_sign, run_axis) = match side_idx {
        0 => (axis1, -1.0, axis2),
        1 => (axis1, 1.0, axis2),
        2 => (axis2, -1.0, axis1),
        _ => (axis2, 1.0, axis1),
    };

    let surface_coord = if normal_sign < 0.0 {
        panel.min[normal_axis]
    } else {
        panel.max[normal_axis]
    };

    let t0 = panel.min[thin_axis];
    let t1 = panel.max[thin_axis];
    let r0 = panel.min[run_axis];
    let r1 = panel.max[run_axis];
    let length_mm = (r1 - r0).abs();

    let make_pt = |t: f64, r: f64| -> [f64; 3] {
        let mut p = [0.0; 3];
        p[normal_axis] = surface_coord;
        p[thin_axis] = t;
        p[run_axis] = r;
        p
    };

    let corners = [
        make_pt(t0, r0),
        make_pt(t1, r0),
        make_pt(t1, r1),
        make_pt(t0, r1),
    ];

    // Punkt próbkowania dla `appearance.assign`: w połowie grubości i blisko narożnika (3 mm lub 15% długości),
    // aby nigdy nie trafić w wywiercony na środku otwór na kołek/konfirmat.
    let mid_t = 0.5 * (t0 + t1);
    let r_probe = r0.min(r1) + 3.0_f64.min(length_mm * 0.15);
    let probe = make_pt(mid_t, r_probe);

    (corners, probe, normal_axis, normal_sign, run_axis, round1(length_mm))
}

/// Oblicza oznaczenie oklejenia w standardzie `xDyK` (np. `2D2K`, `2D0K`, `1D1K`, `0D0K`) oraz sumaryczną długość
/// oklejonych brzegów formatki [mm], klasyfikując brzegi wyłącznie na podstawie optycznej (geometrycznej) długości krawędzi:
/// para dłuższych brzegów = `D`, para krótszych brzegów = `K`.
pub fn compute_edgeband_code_and_length(
    dx: f64,
    dy: f64,
    dz: f64,
    eb: Option<&crate::PanelEdgebanding>,
) -> (String, f64) {
    let Some(eb) = eb else {
        return ("0D0K".into(), 0.0);
    };
    if !eb.any_banded() {
        return ("0D0K".into(), 0.0);
    }

    let ext = [dx.abs(), dy.abs(), dz.abs()];
    let thin_axis = if ext[0] <= ext[1] && ext[0] <= ext[2] {
        0
    } else if ext[1] <= ext[0] && ext[1] <= ext[2] {
        1
    } else {
        2
    };
    let (axis1, axis2) = panel_in_plane_axes(thin_axis);

    // Krawędzie 0 i 1 leżą na min/max[axis1] i biegną wzdłuż axis2 (ich długość to ext[axis2]).
    // Krawędzie 2 i 3 leżą na min/max[axis2] i biegną wzdłuż axis1 (ich długość to ext[axis1]).
    let len_pair01 = round1(ext[axis2]);
    let len_pair23 = round1(ext[axis1]);

    let count01 = usize::from(eb.edges[0].is_some()) + usize::from(eb.edges[1].is_some());
    let count23 = usize::from(eb.edges[2].is_some()) + usize::from(eb.edges[3].is_some());

    let (d_count, k_count) = if len_pair01 >= len_pair23 {
        (count01, count23)
    } else {
        (count23, count01)
    };

    let total_mm = round1((count01 as f64) * len_pair01 + (count23 as f64) * len_pair23);
    (format!("{d_count}D{k_count}K"), total_mm)
}

/// Sprawdza, czy wąski brzeg `side_idx` (`0..4`) formatki `panel` jest zasłonięty (chociaż częściowo)
/// przez inną formatkę w scenie (`boxes`).
pub fn is_narrow_edge_covered(panel: &PanelBox, side_idx: usize, boxes: &[PanelBox]) -> bool {
    let thin_axis = panel.thin_axis();
    let (_corners, _probe, normal_axis, normal_sign, run_axis, _len) =
        narrow_edge_quad_and_probe(panel, side_idx);
    let edge_coord = if normal_sign < 0.0 {
        panel.min[normal_axis]
    } else {
        panel.max[normal_axis]
    };

    for other in boxes {
        if other.name == panel.name || other.name == "Kubatura" {
            continue;
        }
        // 1. Sprawdź czy `other` styka się z płaszczyzną brzegu `edge_coord` wzdłuż `normal_axis` (tolerancja 1.5 mm)
        //    oraz znajduje się po zewnętrznej stronie tego brzegu.
        let touches_normal = other.min[normal_axis] - 1.5 <= edge_coord
            && edge_coord <= other.max[normal_axis] + 1.5;
        if !touches_normal {
            continue;
        }
        let other_center_n = other.center()[normal_axis];
        let panel_center_n = panel.center()[normal_axis];
        if (other_center_n - panel_center_n) * normal_sign <= 0.0 {
            continue;
        }

        // 2. Sprawdź nakładanie się w osi grubości (`thin_axis` > 2 mm)
        let overlap_thin = panel.max[thin_axis].min(other.max[thin_axis])
            - panel.min[thin_axis].max(other.min[thin_axis]);
        if overlap_thin <= 2.0 {
            continue;
        }

        // 3. Sprawdź nakładanie się wzdłuż długości brzegu (`run_axis` > 5 mm)
        let overlap_run = panel.max[run_axis].min(other.max[run_axis])
            - panel.min[run_axis].max(other.min[run_axis]);
        if overlap_run > 5.0 {
            return true;
        }
    }
    false
}

/// Weryfikuje klikniętą ścianę w trybie ręcznym (`EdgebandMode::Manual`):
/// - odrzuca krawędzie zakrzywione (wycięcia łukiem, otwory — brak `plane_normal`),
/// - odrzuca główne płaszczyzny płyty (`axis == thin_axis`),
/// - odrzuca wewnętrzne wycięcia nieleżące na zewnętrznym obrysie formatki,
/// i zwraca indeks wąskiego brzegu `0..4`.
pub fn resolve_narrow_edge_side(
    panel: &PanelBox,
    pick: &FacePick,
    host: &mut dyn WoodHost,
) -> Result<usize, String> {
    // 1. Sprawdź w `model.faces`, czy wskazana ściana jest płaska i prostopadła do osi układu
    if let Ok(faces_val) = host.execute("model.faces", json!({ "body": pick.body }))
        && let Some(faces) = faces_val.get("faces").and_then(Value::as_array)
        && let Some(face_obj) = faces
            .iter()
            .find(|f| f.get("index").and_then(Value::as_u64) == Some(pick.index))
    {
        let normal_arr = face_obj
            .get("plane_normal")
            .or_else(|| face_obj.get("normal"))
            .and_then(Value::as_array);
        let Some(narr) = normal_arr else {
            return Err("Selected face is curved (arc/hole). Pick a flat edge.".into());
        };
        if narr.len() >= 3 {
            let nx = narr[0].as_f64().unwrap_or(0.0).abs();
            let ny = narr[1].as_f64().unwrap_or(0.0).abs();
            let nz = narr[2].as_f64().unwrap_or(0.0).abs();
            let max_comp = nx.max(ny).max(nz);
            if max_comp < 0.98 {
                return Err("Selected face is not a perpendicular panel edge.".into());
            }
        }
    }

    // 2. Wyznacz oś normalnej i zwrot
    let resolved = resolve_panel_face(panel, pick, host);
    let thin_axis = panel.thin_axis();
    if resolved.axis == thin_axis {
        return Err("Main board face selected. Click a narrow edge of the panel.".into());
    }

    // 3. Sprawdź, czy kliknięta ściana leży na zewnętrznym obrysie formatki (a nie wewnątrz wycięcia)
    if (pick.point[resolved.axis] - resolved.surface_coord).abs() > 2.5 {
        return Err("Selected face is inside a cutout, not on the panel perimeter.".into());
    }

    let (axis1, axis2) = panel_in_plane_axes(thin_axis);
    let side_idx = if resolved.axis == axis1 {
        if resolved.normal_sign < 0.0 { 0 } else { 1 }
    } else if resolved.axis == axis2 {
        if resolved.normal_sign < 0.0 { 2 } else { 3 }
    } else {
        return Err("Main board face selected. Click a narrow edge of the panel.".into());
    };

    Ok(side_idx)
}

/// Wylicza zestaw oklejonych brzegów formatki `panel` w trybie automatycznym (`EdgebandMode::Auto`)
/// według wybranej reguły `rule` i punktu kliknięcia `pick`.
pub fn compute_auto_edgebanding_for_panel(
    panel: &PanelBox,
    pick: &FacePick,
    rule: crate::EdgebandAutoRule,
    preset_id: &str,
    boxes: &[PanelBox],
) -> Result<crate::PanelEdgebanding, String> {
    let thin_axis = panel.thin_axis();
    let (axis1, axis2) = panel_in_plane_axes(thin_axis);
    let ext = panel.extents();
    let len_pair01 = ext[axis2]; // brzegi 0 i 1 (na min/max[axis1], biegną wzdłuż axis2)
    let len_pair23 = ext[axis1]; // brzegi 2 i 3 (na min/max[axis2], biegną wzdłuż axis1)

    let mut edges: [Option<String>; 4] = [None, None, None, None];
    let preset = Some(preset_id.to_string());

    match rule {
        crate::EdgebandAutoRule::ExposedOnly => {
            for side_idx in 0..4 {
                if !is_narrow_edge_covered(panel, side_idx, boxes) {
                    edges[side_idx] = preset.clone();
                }
            }
        }
        crate::EdgebandAutoRule::FrontOnly => {
            if thin_axis == 1 {
                return Err("No front edge (panel thickness axis is Y).".into());
            }
            let (idx_min_y, idx_max_y) = if axis1 == 1 { (0, 1) } else { (2, 3) };
            let min_covered = is_narrow_edge_covered(panel, idx_min_y, boxes);
            let max_covered = is_narrow_edge_covered(panel, idx_max_y, boxes);
            let front_idx = if min_covered && !max_covered {
                idx_max_y
            } else {
                idx_min_y
            };
            edges[front_idx] = preset;
        }
        crate::EdgebandAutoRule::TopOnly => {
            if thin_axis == 2 {
                return Err("No top edge (panel thickness axis is Z).".into());
            }
            // Dla thin_axis != 2 zawsze axis2 == 2 (Z), więc +Z to indeks 3
            edges[3] = preset;
        }
        crate::EdgebandAutoRule::BottomOnly => {
            if thin_axis == 2 {
                return Err("No bottom edge (panel thickness axis is Z).".into());
            }
            // Dla thin_axis != 2 zawsze axis2 == 2 (Z), więc -Z to indeks 2
            edges[2] = preset;
        }
        crate::EdgebandAutoRule::FourSides => {
            edges = [preset.clone(), preset.clone(), preset.clone(), preset];
        }
        crate::EdgebandAutoRule::LongOnly => {
            if len_pair01 >= len_pair23 {
                edges[0] = preset.clone();
                edges[1] = preset;
            } else {
                edges[2] = preset.clone();
                edges[3] = preset;
            }
        }
        crate::EdgebandAutoRule::ShortOnly => {
            if len_pair01 < len_pair23 {
                edges[0] = preset.clone();
                edges[1] = preset;
            } else {
                edges[2] = preset.clone();
                edges[3] = preset;
            }
        }
        crate::EdgebandAutoRule::CornerPair => {
            // Podział formatki na 4 obszary narożne względem jej środka w osiach (axis1, axis2)
            let center = panel.center();
            let side_a1 = if pick.point[axis1] < center[axis1] { 0 } else { 1 };
            let side_a2 = if pick.point[axis2] < center[axis2] { 2 } else { 3 };
            edges[side_a1] = preset.clone();
            edges[side_a2] = preset;
        }
    }

    Ok(crate::PanelEdgebanding { edges })
}

/// Obsługuje kliknięcie ściany w oknie 3D, gdy aktywne jest narzędzie `Obrzeża` (`ActiveDialog::Edgebanding`).
pub fn handle_edgeband_face_pick(
    pick: &FacePick,
    state: &mut WoodState,
    host: &mut dyn WoodHost,
) -> Result<String, String> {
    if pick.body == "Kubatura" {
        return Err("Pick a furniture panel, not the bounding box.".into());
    }

    let boxes = extract_panel_boxes(host);
    let panel = boxes
        .iter()
        .find(|b| b.name == pick.body)
        .or_else(|| {
            boxes
                .iter()
                .filter(|b| b.dist_to_point(pick.point) <= 5.0)
                .min_by(|a, b| {
                    a.dist_to_point(pick.point)
                        .partial_cmp(&b.dist_to_point(pick.point))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .ok_or_else(|| format!("Nie znaleziono formatki '{}'.", pick.body))?;

    let ext = panel.extents();
    match state.edgeband_mode {
        crate::EdgebandMode::Manual => {
            let side_idx = resolve_narrow_edge_side(panel, pick, host)?;
            let entry = state.staged_edgebands.entry(panel.name.clone()).or_default();
            let toggled_on = if entry.edges[side_idx].as_deref() == Some(state.edgeband_preset_id.as_str()) {
                entry.edges[side_idx] = None;
                false
            } else {
                entry.edges[side_idx] = Some(state.edgeband_preset_id.clone());
                true
            };
            let (code, len_mm) = compute_edgeband_code_and_length(ext[0], ext[1], ext[2], Some(entry));
            state.edgeband_last_panel = panel.name.clone();
            update_ghost_preview(state, host);

            let side_lbl = narrow_edge_side_label(panel.thin_axis(), side_idx);
            if toggled_on {
                Ok(format!(
                    "{}: zaznaczono brzeg {} → {} ({:.0} mm).",
                    panel.name, side_lbl, code, len_mm
                ))
            } else {
                Ok(format!(
                    "{}: odznaczono brzeg {} → {} ({:.0} mm).",
                    panel.name, side_lbl, code, len_mm
                ))
            }
        }
        crate::EdgebandMode::Auto => {
            let eb = compute_auto_edgebanding_for_panel(
                panel,
                pick,
                state.edgeband_auto_rule,
                &state.edgeband_preset_id,
                &boxes,
            )?;
            let (code, len_mm) = compute_edgeband_code_and_length(ext[0], ext[1], ext[2], Some(&eb));
            state.staged_edgebands.insert(panel.name.clone(), eb);
            state.edgeband_last_panel = panel.name.clone();
            update_ghost_preview(state, host);

            Ok(format!(
                "{}: ustawiono {} ({:.0} mm).",
                panel.name, code, len_mm
            ))
        }
    }
}

/// Czyści oklejenie wybranej formatki w buforze roboczym narzędzia `Obrzeża`.
pub fn clear_panel_edgebanding(panel_name: &str, state: &mut WoodState, host: &mut dyn WoodHost) {
    if !panel_name.is_empty() {
        state.staged_edgebands.insert(
            panel_name.to_string(),
            crate::PanelEdgebanding::default(),
        );
        update_ghost_preview(state, host);
    }
}

/// Zapisuje słownik `state.panel_edgebands` w metadanych dokumentu (`com.solidcraft.wood.edgebands`),
/// dzięki czemu informacje o oklejeniu (`2D2K`) są zachowane po zapisaniu i ponownym otwarciu pliku `.sc`.
pub fn save_edgebands_to_metadata(state: &WoodState, host: &mut dyn WoodHost) {
    if let Ok(eb_val) = serde_json::to_value(&state.panel_edgebands) {
        write_wood_doc_metadata(host, |obj| {
            obj.insert("edgebands".into(), eb_val);
        });
    }
}

/// Wczytuje zapisane informacje o obrzeżach z metadanych dokumentu (jeśli występują).
pub fn load_edgebands_from_metadata(state: &mut WoodState, host: &mut dyn WoodHost) {
    let wood_meta = read_wood_doc_metadata(host);
    if let Some(eb_val) = wood_meta.get("edgebands") {
        if let Ok(parsed) =
            serde_json::from_value::<std::collections::BTreeMap<String, crate::PanelEdgebanding>>(eb_val.clone())
        {
            state.panel_edgebands = parsed;
            return;
        }
    }
    if let Ok(meta_val) = host.execute("component.metadata", json!({ "component": "root" })) {
        let prop_str = meta_val
            .get("properties")
            .and_then(|p| p.get("wood_edgebands"))
            .and_then(Value::as_str)
            .or_else(|| {
                meta_val
                    .get("metadata")
                    .and_then(|m| m.get("properties"))
                    .and_then(|p| p.get("wood_edgebands"))
                    .and_then(Value::as_str)
            });
        if let Some(s) = prop_str
            && let Ok(parsed) =
                serde_json::from_str::<std::collections::BTreeMap<String, crate::PanelEdgebanding>>(s)
        {
            state.panel_edgebands = parsed;
        }
    }
}

/// Zatwierdza oklejenie brzegów (`Zatwierdź` w oknie `Obrzeża`):
/// koloruje na stałe oklejone brzegi formatek kolorem danej grubości okleiny (`appearance.assign`),
/// czyści kolor z brzegów nieoklejonych, zapisuje metadane i odświeża Tabelę Rozkroju (`2D2K`, `mb`).
pub fn apply_edgebanding(state: &mut WoodState, host: &mut dyn WoodHost) -> Result<String, String> {
    let lib = crate::EdgebandLibrary::load()?;
    let boxes = extract_panel_boxes(host);

    let mut colored_edges_count = 0usize;
    let mut panels_count = 0usize;

    for panel in &boxes {
        let staged = state
            .staged_edgebands
            .get(&panel.name)
            .cloned()
            .unwrap_or_default();
        let prev = state
            .panel_edgebands
            .get(&panel.name)
            .cloned()
            .unwrap_or_default();

        if staged.any_banded() {
            panels_count += 1;
        }

        for side_idx in 0..4 {
            let (_corners, probe, _n_ax, _n_sign, _r_ax, _len) =
                narrow_edge_quad_and_probe(panel, side_idx);

            match &staged.edges[side_idx] {
                Some(preset_id) => {
                    colored_edges_count += 1;
                    let color_hex = lib
                        .find_preset(preset_id)
                        .map(|p| p.color.clone())
                        .unwrap_or_else(|| "#38bdf8".to_string());
                    let _ = host.execute(
                        "appearance.assign",
                        json!({
                            "body": panel.name,
                            "faces": [probe],
                            "color": color_hex,
                        }),
                    );
                }
                None => {
                    if prev.edges[side_idx].is_some() {
                        // Usunięto obrzeże z tego brzegu -> wyczyść kolor ściany
                        let _ = host.execute(
                            "appearance.assign",
                            json!({
                                "body": panel.name,
                                "faces": [probe],
                            }),
                        );
                    }
                }
            }
        }
    }

    state.panel_edgebands = state
        .staged_edgebands
        .iter()
        .filter(|(_, eb)| eb.any_banded())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    state.staged_edgebands = state.panel_edgebands.clone();

    save_edgebands_to_metadata(state, host);
    update_ghost_preview(state, host);
    let _ = refresh_cutting_list(state, host);

    let total_mb: f64 = state
        .cutting_list
        .iter()
        .map(|r| r.edgeband_length_mm * f64::from(r.quantity) / 1000.0)
        .sum();

    Ok(format!(
        "Applied edgebanding: {} edges on {} panels (total {:.2} m).",
        colored_edges_count, panels_count, total_mb
    ))
}

/// Returns `(Decor, Grain)` columns for the Cutting List and CSV export.
pub fn decor_bom_fields(opt_decor: Option<&crate::PanelDecor>) -> (String, String) {
    let Some(d) = opt_decor else {
        return (String::new(), String::new());
    };
    let grain = if !d.has_grain {
        "Solid".to_string()
    } else if d.grain_rotated {
        "Cross (90°)".to_string()
    } else {
        "Along (0°)".to_string()
    };
    (d.decor_name.clone(), grain)
}

/// Zapisuje słownik `state.panel_decors` w metadanych dokumentu (`com.solidcraft.wood.decors`).
pub fn save_decors_to_metadata(state: &WoodState, host: &mut dyn WoodHost) {
    if let Ok(val) = serde_json::to_value(&state.panel_decors) {
        write_wood_doc_metadata(host, |obj| {
            obj.insert("decors".into(), val);
        });
    }
}

/// Wczytuje zapisane dekory i kierunek usłojenia z metadanych dokumentu.
pub fn load_decors_from_metadata(state: &mut WoodState, host: &mut dyn WoodHost) {
    let wood_meta = read_wood_doc_metadata(host);
    if let Some(val) = wood_meta.get("decors")
        && let Ok(parsed) =
            serde_json::from_value::<std::collections::BTreeMap<String, crate::PanelDecor>>(
                val.clone(),
            )
    {
        state.panel_decors = parsed;
    }
}

/// Wyznacza punkty próbkowania wyłącznie dla dwóch szerokich płaszczyzn formatki (`±thin_axis`),
/// tak aby `appearance.assign` nigdy nie nadpisał kolorów wąskich krawędzi (obrzeży).
fn broad_face_probes_for_panel(panel: &PanelBox, host: &mut dyn WoodHost) -> Vec<[f64; 3]> {
    let thin_axis = panel.thin_axis();
    let mut probes = Vec::new();

    if let Ok(faces_val) = host.execute("model.faces", json!({ "body": panel.name }))
        && let Some(faces) = faces_val.get("faces").and_then(Value::as_array)
    {
        for f in faces {
            let Some(narr) = f
                .get("plane_normal")
                .or_else(|| f.get("normal"))
                .and_then(Value::as_array)
            else {
                continue;
            };
            if narr.len() < 3 {
                continue;
            }
            let n_thin = narr[thin_axis].as_f64().unwrap_or(0.0).abs();
            if n_thin < 0.98 {
                continue;
            }
            let Some(carr) = f
                .get("centroid")
                .or_else(|| f.get("point"))
                .and_then(Value::as_array)
            else {
                continue;
            };
            if carr.len() < 3 {
                continue;
            }
            let pt = [
                carr[0].as_f64().unwrap_or(0.0),
                carr[1].as_f64().unwrap_or(0.0),
                carr[2].as_f64().unwrap_or(0.0),
            ];
            let on_min = (pt[thin_axis] - panel.min[thin_axis]).abs() <= 0.8;
            let on_max = (pt[thin_axis] - panel.max[thin_axis]).abs() <= 0.8;
            if on_min || on_max {
                probes.push(pt);
            }
        }
    }

    if probes.is_empty() {
        let c = panel.center();
        let mut p_min = c;
        p_min[thin_axis] = panel.min[thin_axis];
        let mut p_max = c;
        p_max[thin_axis] = panel.max[thin_axis];
        probes.push(p_min);
        probes.push(p_max);
    }
    probes
}

/// Obsługuje kliknięcie ściany w oknie 3D, gdy aktywne jest narzędzie `Dekor i Usłojenie` (`ActiveDialog::DecorGrain`).
/// - Tryb `DODAJ`: ustawia kolor z `appearance.assign` na górnej i dolnej powierzchni płyty (nie zmieniając brzegów).
///   Ponowne kliknięcie tej samej formatki z dekorem posiadającym usłojenie obraca kierunek o 90° (`0° ↔ 90°`).
/// - Tryb `USUŃ`: usuwa przypisanie dekoru/usłojenia i czyści kolor wyłącznie z dwóch szerokich ścian formatki.
pub fn handle_decor_face_pick(
    pick: &FacePick,
    state: &mut WoodState,
    host: &mut dyn WoodHost,
) -> Result<String, String> {
    if pick.body == "Kubatura" {
        return Err("Pick a furniture panel, not the bounding box.".into());
    }

    let boxes = extract_panel_boxes(host);
    let panel = boxes
        .iter()
        .find(|b| b.name == pick.body)
        .or_else(|| {
            boxes
                .iter()
                .filter(|b| b.dist_to_point(pick.point) <= 5.0)
                .min_by(|a, b| {
                    a.dist_to_point(pick.point)
                        .partial_cmp(&b.dist_to_point(pick.point))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .ok_or_else(|| format!("Nie znaleziono formatki '{}'.", pick.body))?;

    let probes = broad_face_probes_for_panel(panel, host);
    state.decor_last_panel = panel.name.clone();

    match state.decor_mode {
        crate::DecorMode::Remove => {
            let had_decor = state.panel_decors.remove(&panel.name).is_some();
            host.execute(
                "appearance.assign",
                json!({
                    "body": panel.name,
                    "faces": probes,
                }),
            )?;
            save_decors_to_metadata(state, host);
            update_ghost_preview(state, host);
            let _ = refresh_cutting_list(state, host);
            if had_decor {
                Ok(format!(
                    "{}: removed decor and grain (edgebanding unchanged).",
                    panel.name
                ))
            } else {
                Ok(format!(
                    "{}: restored default appearance on main faces.",
                    panel.name
                ))
            }
        }
        crate::DecorMode::Add => {
            let lib = crate::DecorLibrary::load()?;
            let preset = lib
                .find_preset(&state.decor_preset_id)
                .ok_or_else(|| "Selected decor not found in 'decors.json'.".to_string())?;

            let prev = state.panel_decors.get(&panel.name).cloned();
            let same_decor = prev
                .as_ref()
                .is_some_and(|p| p.decor_name.eq_ignore_ascii_case(&preset.name) && p.has_grain == preset.has_grain);

            let grain_rotated = if !preset.has_grain {
                false
            } else if same_decor {
                !prev.as_ref().is_some_and(|p| p.grain_rotated)
            } else {
                false
            };

            host.execute(
                "appearance.assign",
                json!({
                    "body": panel.name,
                    "faces": probes,
                    "color": preset.color,
                }),
            )?;

            state.panel_decors.insert(
                panel.name.clone(),
                crate::PanelDecor {
                    decor_name: preset.name.clone(),
                    has_grain: preset.has_grain,
                    grain_rotated,
                    color: preset.color.clone(),
                    rgb: preset.rgb,
                },
            );

            save_decors_to_metadata(state, host);
            update_ghost_preview(state, host);
            let _ = refresh_cutting_list(state, host);

            if !preset.has_grain {
                Ok(format!(
                    "{}: assigned decor '{}' (solid, no grain).",
                    panel.name, preset.name
                ))
            } else if same_decor {
                let dir_txt = if grain_rotated {
                    "90° (cross)"
                } else {
                    "0° (along)"
                };
                Ok(format!(
                    "{}: rotated grain for '{}' → {}.",
                    panel.name, preset.name, dir_txt
                ))
            } else {
                Ok(format!(
                    "{}: assigned decor '{}' (grain 0° — along; click again to rotate 90°).",
                    panel.name, preset.name
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EdgePick;

    struct DummyHost;
    impl WoodHost for DummyHost {
        fn execute(&mut self, command: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
            if command == "model.faces" {
                let body = params.get("body").and_then(Value::as_str).unwrap_or("");
                if body == "Front" {
                    return Ok(serde_json::json!({
                        "faces": [
                            { "index": 0, "centroid": [200.0, 0.0, 360.0], "plane_normal": [0.0, 1.0, 0.0] },
                            { "index": 1, "centroid": [200.0, -18.0, 360.0], "plane_normal": [0.0, -1.0, 0.0] },
                        ]
                    }));
                } else if body == "Bok_L" {
                    return Ok(serde_json::json!({
                        "faces": [
                            { "index": 0, "centroid": [18.0, 280.0, 360.0], "plane_normal": [1.0, 0.0, 0.0] },
                            { "index": 1, "centroid": [0.0, 280.0, 360.0], "plane_normal": [-1.0, 0.0, 0.0] },
                        ]
                    }));
                }
            }
            Ok(serde_json::Value::Null)
        }
        fn batch(&mut self, _commands: &[(String, serde_json::Value)], _rollback: bool) -> Result<serde_json::Value, String> {
            Ok(serde_json::Value::Null)
        }
        fn inspect_document(&mut self, _measure: bool) -> Result<serde_json::Value, String> {
            Ok(serde_json::Value::Null)
        }
        fn take_selected_faces(&mut self) -> Vec<FacePick> { Vec::new() }
        fn take_selected_edges(&mut self) -> Vec<EdgePick> { Vec::new() }
        fn enable_face_selection(&mut self) {}
        fn enable_edge_selection(&mut self) {}
        fn set_status(&mut self, _message: &str, _is_error: bool) {}
        fn open_parameters_dialog(&mut self) {}
    }

    #[test]
    fn test_connector_plan_short_trawers_on_long_bok() {
        // Bok_L: 18 × 560 × 720 mm
        // Trawers: 764 × 200 × 18 mm (od Y=360 do Y=560 przy górze Z=702..720)
        let panels = vec![
            PanelBox {
                name: "Bok_L".into(),
                min: [0.0, 0.0, 0.0],
                max: [18.0, 560.0, 720.0],
            },
            PanelBox {
                name: "Trawers_G".into(),
                min: [18.0, 360.0, 702.0],
                max: [782.0, 560.0, 720.0],
            },
        ];

        // Użytkownik kliknął krawędź styku X=18, Z=702 (nawet jeśli kliknął długą krawędź Boku 0..560!)
        let edge = EdgePick {
            body: "Bok_L".into(),
            index: 4,
            point: [18.0, 450.0, 702.0],
            start: [18.0, 0.0, 702.0],
            end: [18.0, 560.0, 702.0],
            length: 560.0,
        };

        let plan = compute_connector_plan(&edge, None, &panels, 50.0, 0.0, 2).unwrap();
        assert_eq!(plan.edge_panel, "Trawers_G");
        assert_eq!(plan.face_panel, "Bok_L");
        // Długość styku musi wynosić 200 mm (z Trawersa 360..560), a NIE 560 mm z Boku!
        assert!((plan.contact_length - 200.0).abs() < 1e-6);
        // Rozstaw: (200 - 2*50) / (2 - 1) = 100 mm
        assert!((plan.spacing - 100.0).abs() < 1e-6);
        // Pierwszy otwór: 360 + 50 = 410 mm w osi Y, wyśrodkowany w grubości Trawersa Z = 711 mm
        assert!((plan.first_hole_pos[0] - 18.0).abs() < 1e-6);
        assert!((plan.first_hole_pos[1] - 410.0).abs() < 1e-6);
        assert!((plan.first_hole_pos[2] - 711.0).abs() < 1e-6);
        // Kierunek wiercenia w brzeg Trawersa (+X), w ścianę Boku (-X)
        assert_eq!(plan.dir_into_edge, [1.0, 0.0, 0.0]);
        assert_eq!(plan.dir_into_face, [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn test_dynamic_orientation_xy_shelf() {
        let mut host = DummyHost;
        // Prawy i Lewy (rozpiętość wzdłuż X)
        let slot0 = vec![
            FacePick { body: "Bok_L".into(), index: 0, point: [18.0, 280.0, 360.0] },
            FacePick { body: "Bok_P".into(), index: 0, point: [782.0, 280.0, 360.0] },
        ];
        // Przód i Tył (rozpiętość wzdłuż Y)
        let slot1 = vec![
            FacePick { body: "Kubatura".into(), index: 1, point: [400.0, 560.0, 360.0] },
            FacePick { body: "Kubatura".into(), index: 2, point: [400.0, 0.0, 360.0] },
        ];
        // Płaszczyzna Z=0 (wieniec dolny)
        let slot2 = vec![
            FacePick { body: "Kubatura".into(), index: 3, point: [400.0, 280.0, 0.0] },
        ];

        let geom = compute_free_panel_geometry(
            &slot0, &slot1, &slot2,
            100.0, 18.0, 0.0, [0.0; 4], true,
            800.0, 560.0, 720.0,
            &mut host,
        ).unwrap();

        assert_eq!(geom.plane, "XY");
        assert_eq!(geom.thickness_axis, 2); // Z
        assert_eq!(geom.min[0], 18.0);
        assert_eq!(geom.max[0], 782.0);
        assert_eq!(geom.min[1], 0.0);
        assert_eq!(geom.max[1], 560.0);
    }

    #[test]
    fn test_dynamic_orientation_xz_back_panel() {
        let mut host = DummyHost;
        // Prawo i Lewo (rozpiętość wzdłuż X)
        let slot0 = vec![
            FacePick { body: "Bok_L".into(), index: 0, point: [18.0, 280.0, 360.0] },
            FacePick { body: "Bok_P".into(), index: 0, point: [782.0, 280.0, 360.0] },
        ];
        // Góra i Dół (rozpiętość wzdłuż Z)
        let slot1 = vec![
            FacePick { body: "Kubatura".into(), index: 4, point: [400.0, 280.0, 720.0] },
            FacePick { body: "Kubatura".into(), index: 5, point: [400.0, 280.0, 0.0] },
        ];
        // Tył Y=0 (plecy)
        let slot2 = vec![
            FacePick { body: "Kubatura".into(), index: 6, point: [400.0, 0.0, 360.0] },
        ];

        let geom = compute_free_panel_geometry(
            &slot0, &slot1, &slot2,
            100.0, 18.0, 0.0, [0.0; 4], true,
            800.0, 560.0, 720.0,
            &mut host,
        ).unwrap();

        assert_eq!(geom.plane, "XZ");
        assert_eq!(geom.thickness_axis, 1); // Y
        assert_eq!(geom.min[0], 18.0);
        assert_eq!(geom.max[0], 782.0);
        assert_eq!(geom.min[2], 0.0);
        assert_eq!(geom.max[2], 720.0);
    }

    #[test]
    fn test_dynamic_orientation_yz_vertical_divider() {
        let mut host = DummyHost;
        // Przód i Tył (rozpiętość wzdłuż Y)
        let slot0 = vec![
            FacePick { body: "Kubatura".into(), index: 1, point: [400.0, 560.0, 360.0] },
            FacePick { body: "Kubatura".into(), index: 2, point: [400.0, 0.0, 360.0] },
        ];
        // Góra i Dół (rozpiętość wzdłuż Z)
        let slot1 = vec![
            FacePick { body: "Kubatura".into(), index: 4, point: [400.0, 280.0, 720.0] },
            FacePick { body: "Kubatura".into(), index: 5, point: [400.0, 280.0, 0.0] },
        ];
        // Płaszczyzna X=400 (przegroda pionowa)
        let slot2 = vec![
            FacePick { body: "Kubatura".into(), index: 7, point: [400.0, 280.0, 360.0] },
        ];

        let geom = compute_free_panel_geometry(
            &slot0, &slot1, &slot2,
            100.0, 18.0, 0.0, [0.0; 4], true,
            800.0, 560.0, 720.0,
            &mut host,
        ).unwrap();

        assert_eq!(geom.plane, "YZ");
        assert_eq!(geom.thickness_axis, 0); // X
        assert_eq!(geom.min[1], 0.0);
        assert_eq!(geom.max[1], 560.0);
        assert_eq!(geom.min[2], 0.0);
        assert_eq!(geom.max[2], 720.0);
    }

    #[test]
    fn test_hinge_plan_blum_standard() {
        let mut host = DummyHost;
        let panels = vec![
            PanelBox {
                name: "Front".into(),
                min: [0.0, -18.0, 0.0],
                max: [400.0, 0.0, 720.0],
            },
            PanelBox {
                name: "Bok_L".into(),
                min: [0.0, 0.0, 0.0],
                max: [18.0, 560.0, 720.0],
            },
        ];

        let front_pick = FacePick {
            body: "Front".into(),
            index: 0,
            point: [200.0, 0.0, 360.0],
        };
        let side_pick = FacePick {
            body: "Bok_L".into(),
            index: 0,
            point: [18.0, 280.0, 360.0],
        };

        let lib = crate::HingeLibrary::load().unwrap();
        let standard = lib.find_standard("BLUM_45_9.5").unwrap();

        let plan = compute_hinge_plan(
            &front_pick,
            &side_pick,
            standard,
            "dowel",
            "euro",
            100.0,
            2,
            &panels,
            &mut host,
        )
        .unwrap();

        assert_eq!(plan.count, 2);
        assert_eq!(plan.instances.len(), 2);
        assert_eq!(plan.front_body, "Front");
        assert_eq!(plan.side_body, "Bok_L");

        // Zawias 1 (dół: Z=100)
        let h1 = &plan.instances[0];
        // Puszka: Z=100, X=22, Y=0
        assert!((h1.cup_hole.position[2] - 100.0).abs() < 1e-4);
        assert!((h1.cup_hole.position[0] - 22.0).abs() < 1e-4);
        assert!((h1.cup_hole.position[1] - 0.0).abs() < 1e-4);
        assert_eq!(h1.cup_hole.diameter, 35.0);
        assert_eq!(h1.cup_hole.depth, 13.0);
        assert_eq!(h1.cup_hole.direction, [0.0, -1.0, 0.0]); // w głąb frontu (-Y)

        // Mocowania puszki (2 otwory pod wkręty fi 2.5 mm):
        assert_eq!(h1.cup_fixing_holes.len(), 2);
        assert_eq!(h1.cup_fixing_holes[0].diameter, 2.5);
        assert_eq!(h1.cup_fixing_holes[0].depth, 2.5);
        // X = 22 + 9.5 = 31.5
        assert!((h1.cup_fixing_holes[0].position[0] - 31.5).abs() < 1e-4);
        assert!((h1.cup_fixing_holes[1].position[0] - 31.5).abs() < 1e-4);
        // Z = 100 - 22.5 = 77.5 oraz Z = 100 + 22.5 = 122.5
        let z_fixes = [h1.cup_fixing_holes[0].position[2], h1.cup_fixing_holes[1].position[2]];
        assert!(z_fixes.iter().any(|&z| (z - 77.5).abs() < 1e-4));
        assert!(z_fixes.iter().any(|&z| (z - 122.5).abs() < 1e-4));

        // Prowadnik na boku (wkręty fi 3 mm, rozstaw Z = 100 +/- 16 = 84 i 116, Y = 0 + 37 = 37, X = 18):
        assert_eq!(h1.plate_holes.len(), 2);
        assert_eq!(h1.plate_holes[0].diameter, 3.0);
        assert_eq!(h1.plate_holes[0].direction, [-1.0, 0.0, 0.0]); // w głąb boku (-X)
        assert!((h1.plate_holes[0].position[0] - 18.0).abs() < 1e-4);
        assert!((h1.plate_holes[0].position[1] - 37.0).abs() < 1e-4);
        let z_plates = [h1.plate_holes[0].position[2], h1.plate_holes[1].position[2]];
        assert!(z_plates.iter().any(|&z| (z - 84.0).abs() < 1e-4));
        assert!(z_plates.iter().any(|&z| (z - 116.0).abs() < 1e-4));

        // Zawias 2 (góra: Z=620)
        let h2 = &plan.instances[1];
        assert!((h2.cup_hole.position[2] - 620.0).abs() < 1e-4);
    }

    #[test]
    fn test_refresh_cutting_list_with_components_and_sketch_extrusions() {
        struct MockHost {
            exported: Vec<String>,
        }
        impl WoodHost for MockHost {
            fn execute(&mut self, command: &str, params: Value) -> Result<Value, String> {
                if command == "inspect.measure" {
                    if let Some(want) = params.get("bodies").and_then(Value::as_array) {
                        if want.iter().any(|v| v.as_str() == Some("Wytloczona_Polka")) {
                            return Ok(serde_json::json!({
                                "body_count": 1,
                                "bodies": [
                                    {
                                        "name": "Wytloczona_Polka",
                                        "bbox": { "min": [18.0, 0.0, 360.0], "max": [782.0, 560.0, 378.0] }
                                    }
                                ]
                            }));
                        }
                    }
                    return Ok(serde_json::json!({
                        "body_count": 2,
                        "bodies": [
                            {
                                "name": "Bok_L",
                                "bbox": { "min": [0.0, 0.0, 0.0], "max": [18.0, 560.0, 720.0] }
                            },
                            {
                                "name": "Kubatura",
                                "bbox": { "min": [0.0, 0.0, 0.0], "max": [800.0, 560.0, 720.0] }
                            }
                        ]
                    }));
                }
                if command == "component.list" {
                    return Ok(serde_json::json!({
                        "components": [
                            {
                                "id": 1,
                                "name": "Kubatura",
                                "bodies": ["Kubatura", "Wytloczona_Polka"]
                            }
                        ]
                    }));
                }
                if command == "model.faces" {
                    let b = params.get("body").and_then(Value::as_str).unwrap_or("");
                    if b == "Wytloczona_Polka" {
                        return Ok(serde_json::json!({
                            "faces": [
                                { "centroid": [18.0, 0.0, 360.0] },
                                { "centroid": [782.0, 560.0, 378.0] }
                            ]
                        }));
                    }
                }
                if command == "file.export_step" {
                    let path = params.get("path").and_then(Value::as_str).unwrap_or("");
                    self.exported.push(path.to_string());
                    return Ok(serde_json::json!({ "path": path }));
                }
                Ok(Value::Null)
            }
            fn batch(&mut self, _commands: &[(String, Value)], _rollback: bool) -> Result<Value, String> { Ok(Value::Null) }
            fn inspect_document(&mut self, _measure: bool) -> Result<Value, String> { Ok(Value::Null) }
            fn take_selected_faces(&mut self) -> Vec<FacePick> { Vec::new() }
            fn take_selected_edges(&mut self) -> Vec<EdgePick> { Vec::new() }
            fn enable_face_selection(&mut self) {}
            fn enable_edge_selection(&mut self) {}
            fn set_status(&mut self, _message: &str, _is_error: bool) {}
            fn open_parameters_dialog(&mut self) {}
        }

        let mut host = MockHost { exported: Vec::new() };
        let mut state = WoodState::default();
        let tmp = std::env::temp_dir().join("solidcraft_test_step_export");
        state.export_step_dir = tmp.to_string_lossy().to_string();

        let res = refresh_cutting_list(&mut state, &mut host).unwrap();
        assert!(res.contains("2 items"));
        assert_eq!(state.cutting_list.len(), 2);
        assert_eq!(state.cutting_list[0].name, "Bok_L");
        assert_eq!(state.cutting_list[0].length_mm, 720.0);
        assert_eq!(state.cutting_list[0].width_mm, 560.0);
        assert_eq!(state.cutting_list[0].thickness_mm, 18.0);

        assert_eq!(state.cutting_list[1].name, "Wytloczona_Polka");
        assert_eq!(state.cutting_list[1].length_mm, 764.0);
        assert_eq!(state.cutting_list[1].width_mm, 560.0);
        assert_eq!(state.cutting_list[1].thickness_mm, 18.0);

        let exp_msg = export_panels_to_step(&mut state, &mut host).unwrap();
        assert!(exp_msg.contains("2/2 STEP panels"));
        assert_eq!(host.exported.len(), 2);
        assert!(host.exported[0].ends_with("Bok_L_18x560x720.step"));
        assert!(host.exported[1].ends_with("Wytloczona_Polka_18x560x764.step"));
        let csv_path = tmp.join("Tabela_Rozkroju.csv");
        assert!(csv_path.is_file());
        let csv_txt = std::fs::read_to_string(&csv_path).unwrap();
        assert!(csv_txt.contains("Bok_L;"));
        assert!(csv_txt.contains("Wytloczona_Polka;"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_next_formatka_name_and_auto_geometry() {
        let mut host = DummyHost;
        let name = next_formatka_name(&mut host);
        assert_eq!(name, "S01_01");

        let pick = FacePick {
            body: "Kubatura".into(),
            index: 0,
            point: [0.0, 280.0, 360.0],
        };
        let geom = compute_auto_panel_geometry(
            &pick,
            18.0,
            0.0,
            [3.0, 3.0, 3.0, 3.0],
            true,
            800.0,
            560.0,
            720.0,
            &mut host,
        ).unwrap();

        assert_eq!(geom.plane, "YZ");
        assert_eq!(geom.thickness_axis, 0);
        assert_eq!(geom.min[0], 0.0);
        assert_eq!(geom.max[0], 18.0);
        assert_eq!(geom.min[1], 3.0);
        assert_eq!(geom.max[1], 557.0);
        assert_eq!(geom.min[2], 3.0);
        assert_eq!(geom.max[2], 717.0);
    }

    #[test]
    fn test_panel_naming_and_monotonic_numbering() {
        // Legacy Formatka_30 is read-only
        let legacy = parse_panel_name("Formatka_30");
        assert_eq!(legacy.number, Some(30));
        assert_eq!(legacy.cabinet, "");
        assert!(!legacy.is_duplicate_copy);

        // Split/Mirror copy suffix is detected
        let split_copy = parse_panel_name("Formatka_07 (2)");
        assert_eq!(split_copy.number, Some(7));
        assert!(split_copy.is_duplicate_copy);

        // New format S05_30_wieniec_gora
        let new_fmt = parse_panel_name("S05_30_wieniec_gora");
        assert_eq!(new_fmt.cabinet, "S05");
        assert_eq!(new_fmt.number, Some(30));
        assert_eq!(new_fmt.custom, "wieniec_gora");
        assert!(!new_fmt.is_duplicate_copy);

        // Monotonic numbering: panels 1,2,3,4, 6,7,8,9 (panel 5 deleted) -> next must be 10, never 5
        let mut state = WoodState::default();
        for n in [1, 2, 3, 4, 6, 7, 8, 9] {
            state.created_panels.push(format!("S01_{:02}", n));
        }
        let mut host = DummyHost;
        let next_num = allocate_panel_number(&mut state, &mut host);
        assert_eq!(next_num, 10);
        assert_eq!(build_panel_name(&state.cabinet_prefix, next_num, "wieniec gora"), "S01_10_wieniec_gora");

        // Letter suffixes
        assert_eq!(index_to_letter_suffix(0), "A");
        assert_eq!(index_to_letter_suffix(1), "B");
        assert_eq!(index_to_letter_suffix(25), "Z");
        assert_eq!(index_to_letter_suffix(26), "AA");
    }

    struct MetaDocHost {
        meta: serde_json::Map<String, Value>,
    }

    impl MetaDocHost {
        fn new() -> Self {
            Self {
                meta: serde_json::Map::new(),
            }
        }
    }

    impl WoodHost for MetaDocHost {
        fn execute(&mut self, command: &str, params: Value) -> Result<Value, String> {
            match command {
                "document.metadata" => Ok(json!({ "metadata": Value::Object(self.meta.clone()) })),
                "document.metadata.set" => {
                    if let (Some(k), Some(v)) = (params.get("key").and_then(Value::as_str), params.get("value")) {
                        self.meta.insert(k.to_string(), v.clone());
                    }
                    Ok(json!({ "metadata": Value::Object(self.meta.clone()) }))
                }
                _ => Ok(Value::Null),
            }
        }
        fn batch(&mut self, _commands: &[(String, Value)], _rollback: bool) -> Result<Value, String> {
            Ok(Value::Null)
        }
        fn inspect_document(&mut self, _measure: bool) -> Result<Value, String> {
            Ok(Value::Null)
        }
        fn take_selected_faces(&mut self) -> Vec<FacePick> { Vec::new() }
        fn take_selected_edges(&mut self) -> Vec<EdgePick> { Vec::new() }
        fn enable_face_selection(&mut self) {}
        fn enable_edge_selection(&mut self) {}
        fn set_status(&mut self, _message: &str, _is_error: bool) {}
        fn open_parameters_dialog(&mut self) {}
    }

    #[test]
    fn test_panel_numbering_is_isolated_per_sc_document() {
        let mut doc_a = MetaDocHost::new();
        let mut doc_b = MetaDocHost::new();
        let mut state = WoodState::default();
        state.hot_reload_generation = 4;

        // W pierwszym pliku .sc tworzymy 3 formatki (1, 2, 3)
        assert_eq!(allocate_panel_number(&mut state, &mut doc_a), 1);
        assert_eq!(allocate_panel_number(&mut state, &mut doc_a), 2);
        assert_eq!(allocate_panel_number(&mut state, &mut doc_a), 3);
        assert_eq!(
            doc_a.meta.get("com.solidcraft.wood").and_then(|v| v.get("max_panel_nr")).and_then(Value::as_u64),
            Some(3)
        );

        // Otwieramy drugi plik .sc -> reset stanu dokumentu w RAM, ale zachowanie hot_reload_generation
        state.reset_for_document();
        assert_eq!(state.hot_reload_generation, 4);
        assert_eq!(state.highest_panel_number, 0);
        // W drugim pliku numeracja zaczyna się od 1 (nie od 4!)
        assert_eq!(allocate_panel_number(&mut state, &mut doc_b), 1);

        // Ponowne otwarcie pierwszego pliku .sc w świeżym WoodState odczytuje max_panel_nr=3 z jego metadanych -> 4
        state.reset_for_document();
        assert_eq!(allocate_panel_number(&mut state, &mut doc_a), 4);
    }

    #[test]
    fn test_auto_panel_two_faces_and_division() {
        let mut host = DummyHost;
        // Dwie powierzchnie w osi X oddalone o 1000 mm (X=0 i X=1000)
        let pick1 = FacePick {
            body: "Kubatura".into(),
            index: 0,
            point: [0.0, 280.0, 360.0],
        };
        let pick2 = FacePick {
            body: "Kubatura".into(),
            index: 1,
            point: [1000.0, 280.0, 360.0],
        };

        // 1. Kierunek "Środek" (1 formatka): 491 i 491 mm
        let plan_center = compute_auto_panel_plan(
            &pick1,
            Some(&pick2),
            18.0,
            0.0,
            [0.0; 4],
            crate::AutoPanelDirection::Center,
            1,
            1000.0,
            560.0,
            720.0,
            &mut host,
        )
        .unwrap();
        assert_eq!(plan_center.panels.len(), 1);
        assert_eq!(plan_center.clearances_mm, vec![491.0, 491.0]);
        assert_eq!(format_clearances_text(&plan_center.clearances_mm), "Clearance (1→2): 491, 491 mm");
        assert_eq!(plan_center.panels[0].min[0], 491.0);
        assert_eq!(plan_center.panels[0].max[0], 509.0);

        // 2. Kierunek "Zewnątrz" (1 formatka): 500 i 482 mm
        let plan_out = compute_auto_panel_plan(
            &pick1,
            Some(&pick2),
            18.0,
            0.0,
            [0.0; 4],
            crate::AutoPanelDirection::Outside,
            1,
            1000.0,
            560.0,
            720.0,
            &mut host,
        )
        .unwrap();
        assert_eq!(plan_out.clearances_mm, vec![500.0, 482.0]);
        assert_eq!(format_clearances_text(&plan_out.clearances_mm), "Clearance (1→2): 500, 482 mm");
        assert_eq!(plan_out.panels[0].min[0], 500.0);
        assert_eq!(plan_out.panels[0].max[0], 518.0);

        // 3. Kierunek "Wewnątrz" (1 formatka): 482 i 500 mm
        let plan_in = compute_auto_panel_plan(
            &pick1,
            Some(&pick2),
            18.0,
            0.0,
            [0.0; 4],
            crate::AutoPanelDirection::Inside,
            1,
            1000.0,
            560.0,
            720.0,
            &mut host,
        )
        .unwrap();
        assert_eq!(plan_in.clearances_mm, vec![482.0, 500.0]);
        assert_eq!(format_clearances_text(&plan_in.clearances_mm), "Clearance (1→2): 482, 500 mm");
        assert_eq!(plan_in.panels[0].min[0], 482.0);
        assert_eq!(plan_in.panels[0].max[0], 500.0);

        // 4. Podział na 2 formatki (3 równe światła: (1000 - 36)/3 = 321.3 mm)
        let plan_div2 = compute_auto_panel_plan(
            &pick1,
            Some(&pick2),
            18.0,
            0.0,
            [0.0; 4],
            crate::AutoPanelDirection::Center,
            2,
            1000.0,
            560.0,
            720.0,
            &mut host,
        )
        .unwrap();
        assert_eq!(plan_div2.panels.len(), 2);
        assert_eq!(plan_div2.clearances_mm, vec![321.3, 321.3, 321.3]);
        assert_eq!(format_clearances_text(&plan_div2.clearances_mm), "Clearance (1→2): 321.3, 321.3, 321.3 mm");

        // 5. Niezgodna oś normalnej drugiej powierzchni (np. Z zamiast X) -> błąd
        let pick_z = FacePick {
            body: "Kubatura".into(),
            index: 5,
            point: [500.0, 280.0, 720.0],
        };
        let err_res = compute_auto_panel_plan(
            &pick1,
            Some(&pick_z),
            18.0,
            0.0,
            [0.0; 4],
            crate::AutoPanelDirection::Center,
            1,
            1000.0,
            560.0,
            720.0,
            &mut host,
        );
        assert!(err_res.is_err());
    }

    #[test]
    fn test_edgebanding_optical_code_and_auto_rules() {
        // Formatka pozioma (wieniec/półka): X=764, Y=560, Z=18 (thin_axis = 2 -> axis1=0 (X, 764), axis2=1 (Y, 560))
        // Brzegi 0 i 1 (lewy/prawy, na min/max X) biegną wzdłuż Y i mają długość 560 mm (krótsze -> K).
        // Brzegi 2 i 3 (przód/tył, na min/max Y) biegną wzdłuż X i mają długość 764 mm (dłuższe -> D).
        let shelf = PanelBox {
            name: "S01_03_polka".into(),
            min: [18.0, 0.0, 350.0],
            max: [782.0, 560.0, 368.0],
        };
        let bok_l = PanelBox {
            name: "S01_01_bok_l".into(),
            min: [0.0, 0.0, 0.0],
            max: [18.0, 560.0, 720.0],
        };
        let bok_p = PanelBox {
            name: "S01_02_bok_p".into(),
            min: [782.0, 0.0, 0.0],
            max: [800.0, 560.0, 720.0],
        };
        let boxes = vec![shelf.clone(), bok_l.clone(), bok_p.clone()];

        // 1. Tryb "Tylko odsłonięte brzegi" na półce między dwoma bokami:
        //    Brzegi lewy (-X, 560mm) i prawy (+X, 560mm) są zasłonięte przez Bok_L i Bok_P!
        //    Odsłonięte są tylko przód (-Y, 764mm) i tył (+Y, 764mm) -> czyli dwie długie krawędzie: 2D0K!
        let pick_center = FacePick {
            body: shelf.name.clone(),
            index: 5,
            point: [400.0, 280.0, 368.0],
        };
        let eb_exposed = compute_auto_edgebanding_for_panel(
            &shelf,
            &pick_center,
            crate::EdgebandAutoRule::ExposedOnly,
            "ABS_08",
            &boxes,
        )
        .unwrap();
        let (code_exp, len_exp) = compute_edgeband_code_and_length(764.0, 560.0, 18.0, Some(&eb_exposed));
        assert_eq!(code_exp, "2D0K");
        assert_eq!(len_exp, 764.0 * 2.0);

        // 2. Tryb "Tylko górny brzeg (+Z)" na półce leżącej w osi Z -> krótki błąd walidacji normalnej!
        let err_top_on_z = compute_auto_edgebanding_for_panel(
            &shelf,
            &pick_center,
            crate::EdgebandAutoRule::TopOnly,
            "ABS_08",
            &boxes,
        );
        assert_eq!(
            err_top_on_z.unwrap_err(),
            "No top edge (panel thickness axis is Z)."
        );

        // 3. Tryb "Tylko górny brzeg (+Z)" na pionowym boku (18 × 560 × 720) -> 1 krótki brzeg górny (560 mm) = 0D1K!
        let pick_bok = FacePick {
            body: bok_l.name.clone(),
            index: 0,
            point: [18.0, 280.0, 360.0],
        };
        let eb_top_bok = compute_auto_edgebanding_for_panel(
            &bok_l,
            &pick_bok,
            crate::EdgebandAutoRule::TopOnly,
            "ABS_08",
            &boxes,
        )
        .unwrap();
        let (code_top, len_top) = compute_edgeband_code_and_length(18.0, 560.0, 720.0, Some(&eb_top_bok));
        assert_eq!(code_top, "0D1K");
        assert_eq!(len_top, 560.0);

        // 4. Tryb "Narożne (1D1K)" — podział na 4 ćwiartki narożne:
        //    Kliknięcie w lewym-przednim rogu półki (X=100 < 400, Y=50 < 280) zaznacza brzeg lewy (-X, 560mm) i przedni (-Y, 764mm) -> 1D1K!
        let pick_corner = FacePick {
            body: shelf.name.clone(),
            index: 5,
            point: [100.0, 50.0, 368.0],
        };
        let eb_corner = compute_auto_edgebanding_for_panel(
            &shelf,
            &pick_corner,
            crate::EdgebandAutoRule::CornerPair,
            "ABS_20",
            &boxes,
        )
        .unwrap();
        assert!(eb_corner.edges[0].is_some()); // -X (560 mm)
        assert!(eb_corner.edges[2].is_some()); // -Y (764 mm)
        assert!(eb_corner.edges[1].is_none());
        assert!(eb_corner.edges[3].is_none());
        let (code_corner, len_corner) = compute_edgeband_code_and_length(764.0, 560.0, 18.0, Some(&eb_corner));
        assert_eq!(code_corner, "1D1K");
        assert_eq!(len_corner, 764.0 + 560.0);
    }

    #[test]
    fn test_compute_tdogbone_plan_single_and_multiple() {
        let face_panel = PanelBox {
            name: "FORMATKA_DOGBONE".into(),
            min: [0.0, 0.0, 0.0],
            max: [300.0, 300.0, 18.0],
        };
        let edge_panel = PanelBox {
            name: "FORMATKA_Z_CZOPEM".into(),
            min: [182.0, 0.0, 18.0],
            max: [200.0, 300.0, 318.0],
        };
        let boxes = vec![face_panel, edge_panel];

        let edge = EdgePick {
            body: "FORMATKA_Z_CZOPEM".into(),
            index: 1,
            point: [200.0, 150.0, 18.0],
            start: [200.0, 0.0, 18.0],
            end: [200.0, 300.0, 18.0],
            length: 300.0,
        };
        let face = FacePick {
            body: "FORMATKA_DOGBONE".into(),
            index: 5,
            point: [190.0, 150.0, 18.0],
        };

        // Pojedyncze połączenie (N=1, szerokość 100 mm, od boków 100 mm)
        let plan1 = compute_tdogbone_plan(&edge, Some(&face), &boxes, 100.0, 100.0, 1).unwrap();
        assert_eq!(plan1.edge_panel, "FORMATKA_Z_CZOPEM");
        assert_eq!(plan1.face_panel, "FORMATKA_DOGBONE");
        assert_eq!(plan1.edge_axis, 1); // Y
        assert_eq!(plan1.drill_axis, 2); // Z
        assert_eq!(plan1.width_axis, 0); // X
        assert_eq!(plan1.contact_d, 18.0);
        assert_eq!(plan1.sign_into_face, -1.0);
        assert_eq!(plan1.face_thickness, 18.0);
        assert_eq!(plan1.edge_thickness, 18.0);
        assert_eq!(plan1.joint_centers, vec![150.0]);
        let bbox1 = plan1.tenon_bounding_boxes();
        assert_eq!(bbox1.len(), 1);
        assert_eq!(bbox1[0], ([182.0, 100.0, 0.0], [200.0, 200.0, 18.0]));

        // Dwa połączenia (N=2, szerokość 60 mm, od boków 40 mm na odcinku 300 mm -> u_first=70, u_last=230)
        let plan2 = compute_tdogbone_plan(&edge, Some(&face), &boxes, 60.0, 40.0, 2).unwrap();
        assert_eq!(plan2.joint_centers, vec![70.0, 230.0]);
    }

    #[test]
    fn test_compute_lamello_p14_plan_and_chord() {
        let face_panel = PanelBox {
            name: "PANEL_FACE".into(),
            min: [0.0, 0.0, 0.0],
            max: [300.0, 200.0, 18.0],
        };
        let edge_panel = PanelBox {
            name: "PANEL_EDGE".into(),
            min: [182.0, 0.0, 18.0],
            max: [200.0, 200.0, 318.0],
        };
        let boxes = vec![face_panel, edge_panel];

        let edge = EdgePick {
            body: "PANEL_EDGE".into(),
            index: 1,
            point: [200.0, 100.0, 18.0],
            start: [200.0, 0.0, 18.0],
            end: [200.0, 200.0, 18.0],
            length: 200.0,
        };
        let face = FacePick {
            body: "PANEL_FACE".into(),
            index: 5,
            point: [190.0, 100.0, 18.0],
        };

        let plan = compute_lamello_p14_plan(&edge, Some(&face), &boxes, 9.0, 50.0, 2).unwrap();

        assert_eq!(plan.joint_centers, vec![50.0, 150.0]);
        assert!((plan.w_center - 191.0).abs() < 1e-6);
        assert!((plan.chord_length - 69.55861).abs() < 1e-3);
        assert_eq!(plan.count, 2);
        assert!((plan.spacing - 100.0).abs() < 1e-6);

        let plan_86 = compute_lamello_p14_plan(&edge, Some(&face), &boxes, 8.6, 50.0, 2).unwrap();
        assert!((plan_86.w_center - 191.4).abs() < 1e-6);
        assert!((plan_86.hex_start_w - 200.0).abs() < 1e-6);
        assert_eq!(plan_86.hex_dir_w, -1.0);
        assert!((plan_86.z_offset_eff - 8.6).abs() < 1e-6);

        let mut edge_min_side = edge;
        edge_min_side.point[0] = 182.0;
        let plan_min = compute_lamello_p14_plan(&edge_min_side, Some(&face), &boxes, 8.6, 50.0, 2).unwrap();
        assert!((plan_min.w_center - 190.6).abs() < 1e-6);
        assert!((plan_min.hex_start_w - 182.0).abs() < 1e-6);
        assert_eq!(plan_min.hex_dir_w, 1.0);
    }

    #[test]
    fn test_decor_grain_assign_rotate_and_remove() {
        struct DecorMockHost {
            meta: serde_json::Map<String, Value>,
            appearances: Vec<(String, Value)>,
        }
        impl WoodHost for DecorMockHost {
            fn execute(&mut self, command: &str, params: Value) -> Result<Value, String> {
                match command {
                    "document.metadata" => Ok(json!({ "metadata": Value::Object(self.meta.clone()) })),
                    "document.metadata.set" => {
                        if let (Some(k), Some(v)) =
                            (params.get("key").and_then(Value::as_str), params.get("value"))
                        {
                            self.meta.insert(k.to_string(), v.clone());
                        }
                        Ok(json!({ "metadata": Value::Object(self.meta.clone()) }))
                    }
                    "model.faces" => Ok(json!({
                        "faces": [
                            { "index": 0, "centroid": [400.0, 280.0, 18.0], "plane_normal": [0.0, 0.0, 1.0] },
                            { "index": 1, "centroid": [400.0, 280.0, 0.0], "plane_normal": [0.0, 0.0, -1.0] },
                            { "index": 2, "centroid": [400.0, 0.0, 9.0], "plane_normal": [0.0, -1.0, 0.0] },
                        ]
                    })),
                    "inspect.measure" => Ok(json!({
                        "bodies": [{
                            "name": "S01_01_polka",
                            "bbox": { "min": [0.0, 0.0, 0.0], "max": [800.0, 560.0, 18.0] }
                        }]
                    })),
                    "appearance.assign" => {
                        self.appearances.push((command.to_string(), params));
                        Ok(json!({ "ok": true }))
                    }
                    _ => Ok(Value::Null),
                }
            }
            fn batch(&mut self, _commands: &[(String, Value)], _rollback: bool) -> Result<Value, String> {
                Ok(Value::Null)
            }
            fn inspect_document(&mut self, _measure: bool) -> Result<Value, String> {
                Ok(json!({
                    "bodies": [{
                        "name": "S01_01_polka",
                        "bbox": { "min": [0.0, 0.0, 0.0], "max": [800.0, 560.0, 18.0] }
                    }]
                }))
            }
            fn take_selected_faces(&mut self) -> Vec<FacePick> { Vec::new() }
            fn take_selected_edges(&mut self) -> Vec<EdgePick> { Vec::new() }
            fn enable_face_selection(&mut self) {}
            fn enable_edge_selection(&mut self) {}
            fn set_status(&mut self, _message: &str, _is_error: bool) {}
            fn open_parameters_dialog(&mut self) {}
        }

        let mut lib = crate::DecorLibrary::load().expect("decors.json must load");
        let custom_id = lib.upsert_preset("Dąb Testowy X1", true, [190, 140, 80]);
        assert!(!custom_id.is_empty());

        let mut host = DecorMockHost {
            meta: serde_json::Map::new(),
            appearances: Vec::new(),
        };
        let mut state = WoodState::default();
        state.active_dialog = Some(crate::ActiveDialog::DecorGrain);
        state.decor_mode = crate::DecorMode::Add;
        state.decor_preset_id = "DAB_CRAFT_ZLOTY".into();

        let pick = FacePick {
            body: "S01_01_polka".into(),
            index: 0,
            point: [400.0, 280.0, 18.0],
        };

        // 1. Pierwsze kliknięcie: nadaje dekor z usłojeniem (0° wzdłuż X) tylko na 2 szerokie ściany (Z=0 i Z=18)
        let msg1 = handle_decor_face_pick(&pick, &mut state, &mut host).unwrap();
        assert!(msg1.contains("0°"), "msg1={msg1}");
        assert_eq!(state.ghost_arrows.len(), 2);
        assert_eq!(state.ghost_arrows[0].dir, [1.0, 0.0, 0.0]);
        let app_faces = host.appearances[0].1.get("faces").and_then(Value::as_array).unwrap();
        assert_eq!(app_faces.len(), 2);
        assert!(host.appearances[0].1.get("color").is_some());

        // 2. Drugie kliknięcie tej samej formatki: obraca usłojenie o 90° (wzdłuż Y)
        let msg2 = handle_decor_face_pick(&pick, &mut state, &mut host).unwrap();
        assert!(msg2.contains("90°"), "msg2={msg2}");
        assert!(state.panel_decors["S01_01_polka"].grain_rotated);
        assert_eq!(state.ghost_arrows[0].dir, [0.0, 1.0, 0.0]);

        // Sprawdź kolumny w CSV
        let csv = cutting_list_to_csv(&state);
        assert!(csv.contains("Decor;Grain"), "csv header={csv}");
        assert!(csv.contains("Cross (90°)"), "csv row={csv}");

        // 3. Solid decor (no grain): color assigned, no arrows
        state.decor_preset_id = "BIEL_ALPEJSKA".into();
        let msg3 = handle_decor_face_pick(&pick, &mut state, &mut host).unwrap();
        assert!(msg3.contains("solid"), "msg3={msg3}");
        assert!(state.ghost_arrows.is_empty());

        // 4. Switch to REMOVE: clears decor and calls appearance.assign without color on 2 broad faces
        state.decor_mode = crate::DecorMode::Remove;
        let msg4 = handle_decor_face_pick(&pick, &mut state, &mut host).unwrap();
        assert!(msg4.contains("removed"), "msg4={msg4}");
        assert!(state.panel_decors.is_empty());
        let last_app = &host.appearances.last().unwrap().1;
        assert!(last_app.get("color").is_none());
        assert_eq!(last_app.get("faces").and_then(Value::as_array).unwrap().len(), 2);
    }

    #[test]
    fn test_drawer_library_and_compute_drawer_plan() {
        struct DummyHost;
        impl WoodHost for DummyHost {
            fn execute(&mut self, _command: &str, _params: Value) -> Result<Value, String> {
                Ok(Value::Null)
            }
            fn batch(&mut self, _commands: &[(String, Value)], _rollback: bool) -> Result<Value, String> {
                Ok(Value::Null)
            }
            fn inspect_document(&mut self, _measure: bool) -> Result<Value, String> {
                Ok(Value::Null)
            }
            fn take_selected_faces(&mut self) -> Vec<FacePick> { Vec::new() }
            fn take_selected_edges(&mut self) -> Vec<EdgePick> { Vec::new() }
            fn enable_face_selection(&mut self) {}
            fn enable_edge_selection(&mut self) {}
            fn set_status(&mut self, _message: &str, _is_error: bool) {}
            fn open_parameters_dialog(&mut self) {}
        }

        let lib = crate::DrawerLibrary::load().expect("drawers.json must load");
        assert!(lib.systems.len() >= 2);
        let sys = lib.find_system("SEVROLL_VBOX_18_3D_SLIM").unwrap();
        let h178 = sys.find_height("H178").unwrap();
        let l450 = sys.find_length(450.0).unwrap();

        // Szafka o szerokości zewnętrznej 600 mm (LW = 600 - 2*18 = 564 mm), głębokości 500 mm, wysokości 720 mm
        let boxes = vec![
            PanelBox {
                name: "S05_01_bok_lewy".into(),
                min: [0.0, 0.0, 0.0],
                max: [18.0, 500.0, 720.0],
            },
            PanelBox {
                name: "S05_02_bok_prawy".into(),
                min: [582.0, 0.0, 0.0],
                max: [600.0, 500.0, 720.0],
            },
            PanelBox {
                name: "S05_03_wieniec_dolny".into(),
                min: [18.0, 0.0, 0.0],
                max: [582.0, 500.0, 18.0],
            },
            PanelBox {
                name: "S05_04_front_szuflady".into(),
                min: [0.0, -18.0, 0.0],
                max: [600.0, 0.0, 220.0],
            },
        ];

        let p_left = FacePick {
            body: "S05_01_bok_lewy".into(),
            index: 1,
            point: [18.0, 200.0, 100.0],
        };
        let p_right = FacePick {
            body: "S05_02_bok_prawy".into(),
            index: 0,
            point: [582.0, 200.0, 100.0],
        };
        let p_bottom = FacePick {
            body: "S05_03_wieniec_dolny".into(),
            index: 5,
            point: [300.0, 200.0, 18.0],
        };
        let p_front = FacePick {
            body: "S05_04_front_szuflady".into(),
            index: 3,
            point: [300.0, 0.0, 80.0],
        };

        let mut host = DummyHost;
        let plan = compute_drawer_plan(
            &p_left,
            &p_right,
            &p_bottom,
            &p_front,
            sys,
            h178,
            l450,
            10.0, // podniesienie +10 mm od górnego boku dolnego wieńca (Z=18 + 10 = 28 mm)
            &boxes,
            &mut host,
        )
        .unwrap();

        assert_eq!(plan.lw_mm, 564.0);
        assert!(plan.fits_depth);
        assert!(plan.fits_height);
        // Dno: LW - 75 = 489 mm, NL - 24 = 426 mm, grubość 18 mm
        assert_eq!(plan.bottom_panel_dims_mm, (489.0, 426.0, 18.0));
        // Plecy dla H178: LW - 87 = 477 mm, wysokość 169 mm, grubość 18 mm
        assert_eq!(plan.back_panel_dims_mm, (477.0, 169.0, 18.0));

        // Otwory prowadnicy w bokach: Z = 18 + 10 + 33 = 61.0 mm, Y = 37.0 i 37+224 = 261.0 mm
        assert_eq!(plan.side1_holes.len(), 2);
        assert_eq!(plan.side1_holes[0].position, [18.0, 37.0, 61.0]);
        assert_eq!(plan.side1_holes[1].position, [18.0, 261.0, 61.0]);
        assert_eq!(plan.side2_holes[0].position, [582.0, 37.0, 61.0]);

        // Otwory we froncie dla H178 (4 wiersze × 2 strony = 8 otworów): pierwszy Z = 18 + 10 + 47.5 = 75.5 mm, X = 18+15.5=33.5 oraz 582-15.5=566.5
        assert_eq!(plan.front_holes.len(), 8);
        assert_eq!(plan.front_holes[0].position, [33.5, 0.0, 75.5]);
        assert_eq!(plan.front_holes[1].position, [566.5, 0.0, 75.5]);

        // Otwory w plecach dla H178 (4 wiersze × 2 strony = 8 otworów)
        assert_eq!(plan.back_holes.len(), 8);

        // Walidacja zbyt głębokiej szuflady (NL = 600 w korpusie o głębokości 500 mm -> fits_depth == false)
        let l600 = sys.find_length(600.0).unwrap();
        let plan_too_deep = compute_drawer_plan(
            &p_left,
            &p_right,
            &p_bottom,
            &p_front,
            sys,
            h178,
            l600,
            10.0,
            &boxes,
            &mut host,
        )
        .unwrap();
        assert!(!plan_too_deep.fits_depth);
    }
}
