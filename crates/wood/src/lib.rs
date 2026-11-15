//! SolidCraft Wood — native furniture modeling crate (`cdylib` + `rlib`).
//!
//! Provides TopSolid Wood-inspired parametric cabinet modeling tools:
//! 1. **Kubatura** (`operations::create_kubatura`) — semi-transparent bounding box (`Szerokosc`, `Glebokosc`, `Wysokosc`, `Grubosc`).
//! 2. **Quick Formatka** (`operations::create_auto_panel`) — 1-click face extrusion inward/outward by `Grubosc`.
//! 3. **FORMATKA (1 → 2 → 3)** (`operations::create_free_panel`) — 3-step parametric panel fitted between two side faces,
//!    bounded by front/back faces (or 1 face + width for stretchers), and attached/offset from a reference face.
//! 4. **Złożenie i Jointy** (`operations::bind_all_joints`) — rigid `joint.as_built` assembly into grounded `Korpus_Baza`.
//! 5. **Tabela Rozkroju** (`operations::refresh_cutting_list`) — live cutting list / BOM table with dimensions and CSV export.
//!
//! Supports instant hot-reloading via `cargo build -p wood` (`target/debug/wood.dll`) and `F5` in development,
//! while compiling statically into `solidcraft` for production releases.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod geometry;
pub mod loader;
pub mod operations;
pub mod ui;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current ABI version for the `wood.dll` hot-reload vtable.
pub const WOOD_ABI_VERSION: u32 = 5;

/// Folders in which `connectors.json` / `hinges.json` are searched, in order: the working
/// directory, `assets/hardware/` and the folder of the running executable.
fn hardware_candidates(file_name: &str) -> Vec<std::path::PathBuf> {
    let mut out = vec![
        std::path::PathBuf::from(file_name),
        std::path::PathBuf::from("assets/hardware").join(file_name),
        std::path::PathBuf::from("..").join(file_name),
        std::path::PathBuf::from("../assets/hardware").join(file_name),
    ];
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = std::path::PathBuf::from(manifest);
        out.push(p.join(file_name));
        out.push(p.join("assets/hardware").join(file_name));
        out.push(p.join("../..").join(file_name));
        out.push(p.join("../../assets/hardware").join(file_name));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(std::path::Path::to_path_buf)) {
        out.push(dir.join(file_name));
        out.push(dir.join("assets/hardware").join(file_name));
    }
    out
}

/// Reads and parses the first existing `file_name` among [`hardware_candidates`].
/// There is deliberately no built-in copy: a missing file is reported with every path searched.
fn load_hardware_json<T: serde::de::DeserializeOwned>(file_name: &str) -> Result<T, String> {
    let candidates = hardware_candidates(file_name);
    for path in &candidates {
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Cannot read file '{}': {e}", path.display()))?;
        return serde_json::from_str::<T>(&text)
            .map_err(|e| format!("File '{}' has invalid format: {e}", path.display()));
    }
    let searched = candidates
        .iter()
        .map(|p| std::path::absolute(p).unwrap_or_else(|_| p.clone()).display().to_string())
        .collect::<Vec<_>>()
        .join("\n  • ");
    Err(format!("Nie znaleziono pliku '{file_name}'. Szukano w:\n  • {searched}"))
}

/// A 3D face picked in the SolidCraft viewport.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FacePick {
    pub body: String,
    pub index: u64,
    pub point: [f64; 3],
}

/// A 3D edge picked in the SolidCraft viewport (with start/end points and length).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgePick {
    pub body: String,
    pub index: u64,
    pub point: [f64; 3],
    pub start: [f64; 3],
    pub end: [f64; 3],
    pub length: f64,
}

/// Hole specification inside a connector preset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectorHoleSpec {
    pub diameter: f64,
    #[serde(default)]
    pub depth: f64,
}

/// A single connector preset (dowel or screw/confirmat).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectorPreset {
    pub id: String,
    pub name: String,
    pub edge_hole: ConnectorHoleSpec,
    pub face_hole: ConnectorHoleSpec,
}

/// A category of connectors (e.g. `kolki`, `wkrety`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectorCategory {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub through_face_default: bool,
    pub items: Vec<ConnectorPreset>,
}

/// Root JSON library of furniture connectors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConnectorLibrary {
    pub categories: Vec<ConnectorCategory>,
}

impl ConnectorLibrary {
    /// Loads `connectors.json`. A missing, unreadable or empty file is an error naming the paths searched.
    pub fn load() -> Result<Self, String> {
        let lib: ConnectorLibrary = load_hardware_json("connectors.json")?;
        if lib.categories.is_empty() {
            return Err("File 'connectors.json' contains no connector categories.".into());
        }
        Ok(lib)
    }

    pub fn find_preset(&self, preset_id: &str) -> Option<(&ConnectorCategory, &ConnectorPreset)> {
        for cat in &self.categories {
            for item in &cat.items {
                if item.id == preset_id {
                    return Some((cat, item));
                }
            }
        }
        self.categories
            .first()
            .and_then(|cat| cat.items.first().map(|item| (cat, item)))
    }
}

/// Parametry puszki i otworów pod wkręty mocujące na froncie.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HingeFrontSpec {
    #[serde(alias = "srednica_puszki_mm", alias = "cup_diameter")]
    pub cup_diameter_mm: f64,
    #[serde(alias = "glebokosc_puszki_mm", alias = "cup_depth")]
    pub cup_depth_mm: f64,
    #[serde(alias = "odleglosc_k_mm", alias = "odleglosc_od_krawedzi_k_mm", alias = "edge_distance_k", alias = "default_edge_distance_K_mm")]
    pub edge_distance_k_mm: f64,
    #[serde(alias = "rozstaw_wkretow_mm", alias = "screw_spacing_x", alias = "spacing_X_mm")]
    pub screw_spacing_x_mm: f64,
    #[serde(alias = "odsuniecie_wkretow_mm", alias = "screw_offset_y", alias = "offset_Y_mm")]
    pub screw_offset_y_mm: f64,
    #[serde(alias = "srednica_wkretu_mm", alias = "screw_diameter", alias = "screw_hole_diameter")]
    pub screw_hole_diameter_mm: f64,
    #[serde(alias = "glebokosc_wkretu_mm", alias = "screw_depth", alias = "screw_hole_depth")]
    pub screw_hole_depth_mm: f64,
}

/// Mounting plate and screw hole parameters on the cabinet side panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HingeSideSpec {
    #[serde(alias = "odleglosc_od_krawedzi_mm", alias = "edge_distance", alias = "distance_from_cabinet_edge_overlay_mm")]
    pub edge_distance_mm: f64,
    #[serde(alias = "rozstaw_otworow_mm", alias = "rozstaw_wkretow_mm", alias = "spacing_z", alias = "vertical_spacing_Z_mm")]
    pub spacing_z_mm: f64,
    #[serde(alias = "srednica_wkretu_mm", alias = "screw_diameter", alias = "screw_hole_diameter_mm")]
    pub screw_hole_diameter_mm: f64,
    #[serde(alias = "glebokosc_wkretu_mm", alias = "screw_depth", alias = "screw_hole_depth_mm")]
    pub screw_hole_depth_mm: f64,
}

/// A complete hinge standard entry (Blum, Hettich, Häfele, Grass, Mini).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HingeStandard {
    pub id: String,
    pub name: String,
    pub front: HingeFrontSpec,
    #[serde(alias = "bok")]
    pub side: HingeSideSpec,
}

/// Library of concealed cup hinges (rozstawy zawiasów puszkowych).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HingeLibrary {
    #[serde(default)]
    pub description: String,
    pub standards: Vec<HingeStandard>,
}

impl HingeLibrary {
    /// Loads `hinges.json`. A missing, unreadable or empty file is an error naming the paths searched.
    pub fn load() -> Result<Self, String> {
        let lib: HingeLibrary = load_hardware_json("hinges.json")?;
        if lib.standards.is_empty() {
            return Err("File 'hinges.json' contains no hinge standards.".into());
        }
        Ok(lib)
    }

    pub fn find_standard(&self, standard_id: &str) -> Option<&HingeStandard> {
        self.standards
            .iter()
            .find(|s| s.id == standard_id)
            .or_else(|| self.standards.first())
    }
}

/// A single furniture edgebanding preset from `edgebanding.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgebandPreset {
    pub id: String,
    pub name: String,
    pub thickness_mm: f64,
    pub color: String,
    #[serde(default = "default_edgeband_rgb")]
    pub rgb: [u8; 3],
}

fn default_edgeband_rgb() -> [u8; 3] {
    [56, 189, 248]
}

/// Library of furniture edgebanding presets (`edgebanding.json`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EdgebandLibrary {
    #[serde(default)]
    pub description: String,
    pub presets: Vec<EdgebandPreset>,
}

impl EdgebandLibrary {
    /// Loads `edgebanding.json`.
    pub fn load() -> Result<Self, String> {
        let lib: EdgebandLibrary = load_hardware_json("edgebanding.json")?;
        if lib.presets.is_empty() {
            return Err("File 'edgebanding.json' contains no edgeband presets.".into());
        }
        Ok(lib)
    }

    pub fn find_preset(&self, preset_id: &str) -> Option<&EdgebandPreset> {
        self.presets
            .iter()
            .find(|p| p.id == preset_id)
            .or_else(|| self.presets.first())
    }
}

/// A single furniture panel decor preset from `decors.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecorPreset {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub has_grain: bool,
    pub color: String,
    #[serde(default = "default_decor_rgb")]
    pub rgb: [u8; 3],
}

fn default_decor_rgb() -> [u8; 3] {
    [200, 157, 102]
}

/// Library of furniture panel decors (`decors.json`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DecorLibrary {
    #[serde(default)]
    pub description: String,
    pub presets: Vec<DecorPreset>,
}

impl DecorLibrary {
    /// Loads `decors.json`.
    pub fn load() -> Result<Self, String> {
        let lib: DecorLibrary = load_hardware_json("decors.json")?;
        if lib.presets.is_empty() {
            return Err("File 'decors.json' contains no decor presets.".into());
        }
        Ok(lib)
    }

    pub fn find_preset(&self, preset_id: &str) -> Option<&DecorPreset> {
        self.presets
            .iter()
            .find(|p| p.id == preset_id)
            .or_else(|| self.presets.first())
    }

    /// Adds or updates a decor preset by normalized name/id and returns its preset ID.
    pub fn upsert_preset(&mut self, name: &str, has_grain: bool, rgb: [u8; 3]) -> String {
        let trimmed = name.trim();
        let hex = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
        if let Some(existing) = self
            .presets
            .iter_mut()
            .find(|p| p.name.eq_ignore_ascii_case(trimmed))
        {
            existing.name = trimmed.to_string();
            existing.has_grain = has_grain;
            existing.color = hex;
            existing.rgb = rgb;
            return existing.id.clone();
        }
        let mut slug = String::new();
        for ch in trimmed.chars() {
            let mapped = match ch {
                'ą' | 'Ą' => 'A',
                'ć' | 'Ć' => 'C',
                'ę' | 'Ę' => 'E',
                'ł' | 'Ł' => 'L',
                'ń' | 'Ń' => 'N',
                'ó' | 'Ó' => 'O',
                'ś' | 'Ś' => 'S',
                'ź' | 'Ź' | 'ż' | 'Ż' => 'Z',
                c if c.is_ascii_alphanumeric() => c.to_ascii_uppercase(),
                _ => '_',
            };
            if mapped == '_' {
                if !slug.ends_with('_') && !slug.is_empty() {
                    slug.push('_');
                }
            } else {
                slug.push(mapped);
            }
        }
        let slug = slug.trim_matches('_').to_string();
        let base_id = if slug.is_empty() {
            "DEKOR_WLASNY".to_string()
        } else {
            slug
        };
        let mut id = base_id.clone();
        let mut suffix = 2usize;
        while self.presets.iter().any(|p| p.id == id) {
            id = format!("{base_id}_{suffix}");
            suffix += 1;
        }
        self.presets.push(DecorPreset {
            id: id.clone(),
            name: trimmed.to_string(),
            has_grain,
            color: hex,
            rgb,
        });
        id
    }

    /// Saves updated library back to the existing `decors.json` location(s).
    pub fn save(&self) -> Result<std::path::PathBuf, String> {
        let json_text = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize 'decors.json': {e}"))?;
        let mut seen = std::collections::BTreeSet::new();
        let mut first_written: Option<std::path::PathBuf> = None;
        for path in hardware_candidates("decors.json") {
            if !path.is_file() {
                continue;
            }
            let abs = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
            if !seen.insert(abs.clone()) {
                continue;
            }
            std::fs::write(&abs, &json_text)
                .map_err(|e| format!("Cannot write '{}': {e}", abs.display()))?;
            if first_written.is_none() {
                first_written = Some(abs);
            }
        }
        if let Some(p) = first_written {
            return Ok(p);
        }
        let fallback = std::path::PathBuf::from("decors.json");
        std::fs::write(&fallback, &json_text)
            .map_err(|e| format!("Cannot write '{}': {e}", fallback.display()))?;
        Ok(fallback)
    }
}

/// A single CNC T-Dogbone joint preset from `cnc_joints.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CncJointPreset {
    pub id: String,
    pub name: String,
    pub joint_width_mm: f64,
    pub edge_offset_mm: f64,
    #[serde(default = "default_one_u32")]
    pub default_count: u32,
    pub relief_width_mm: f64,
    pub relief_depth_mm: f64,
    pub relief_radius_mm: f64,
    pub dogbone_drill_dia_mm: f64,
    pub dogbone_corner_offset_mm: f64,
    #[serde(default = "default_45_deg")]
    pub dogbone_angle_deg: f64,
    #[serde(default = "default_12_mm")]
    pub contour_tool_dia_mm: f64,
}

fn default_one_u32() -> u32 {
    1
}

fn default_45_deg() -> f64 {
    45.0
}

fn default_12_mm() -> f64 {
    12.0
}

/// Library of CNC T-Dogbone joint presets (`cnc_joints.json`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CncJointLibrary {
    #[serde(default)]
    pub description: String,
    pub presets: Vec<CncJointPreset>,
}

impl CncJointLibrary {
    /// Loads `cnc_joints.json`.
    pub fn load() -> Result<Self, String> {
        let lib: CncJointLibrary = load_hardware_json("cnc_joints.json")?;
        if lib.presets.is_empty() {
            return Err("File 'cnc_joints.json' contains no joint presets.".into());
        }
        Ok(lib)
    }

    pub fn find_preset(&self, preset_id: &str) -> Option<&CncJointPreset> {
        self.presets
            .iter()
            .find(|p| p.id == preset_id)
            .or_else(|| self.presets.first())
    }
}

/// A single drawer height variant in a general-purpose drawer system (`drawers.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawerHeightVariant {
    pub id: String,
    pub name: String,
    pub drawer_height_mm: f64,
    pub min_space_height_mm: f64,
    pub back_height_mm: f64,
    pub front_hole_z_offsets_mm: Vec<f64>,
    #[serde(default)]
    pub back_hole_top_offsets_mm: Vec<f64>,
}

/// A single nominal length (`NL`) variant in a general-purpose drawer system (`drawers.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawerLengthVariant {
    pub nl_mm: f64,
    pub runner_hole_x_offsets_mm: Vec<f64>,
}

/// A general-purpose drawer system definition (`drawers.json`), supporting any brand/series
/// (e.g. Sevroll V-BOX, Blum Tandembox/Merivobox, Hettich AvanTech, GTV Axis Pro).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawerSystem {
    pub id: String,
    #[serde(default)]
    pub brand: String,
    pub name: String,
    pub short_code: String,
    pub board_thickness_mm: f64,
    pub bottom_width_deduction_mm: f64,
    pub bottom_depth_deduction_mm: f64,
    pub back_width_deduction_mm: f64,
    pub min_depth_clearance_mm: f64,
    #[serde(default)]
    pub push_open_front_gap_mm: f64,
    pub runner_first_hole_from_edge_mm: f64,
    pub runner_hole_z_from_bottom_mm: f64,
    #[serde(default = "default_15_mm")]
    pub bottom_panel_z_from_bottom_mm: f64,
    pub front_hole_side_offset_mm: f64,
    pub front_first_hole_z_from_bottom_mm: f64,
    #[serde(default = "default_9_mm")]
    pub back_hole_side_offset_mm: f64,
    #[serde(default = "default_2_5_mm")]
    pub screw_hole_diameter_mm: f64,
    #[serde(default = "default_2_5_mm")]
    pub screw_hole_depth_mm: f64,
    pub heights: Vec<DrawerHeightVariant>,
    pub lengths: Vec<DrawerLengthVariant>,
}

fn default_15_mm() -> f64 {
    15.0
}

fn default_9_mm() -> f64 {
    9.0
}

fn default_2_5_mm() -> f64 {
    2.5
}

impl DrawerSystem {
    pub fn find_height(&self, height_id: &str) -> Option<&DrawerHeightVariant> {
        self.heights
            .iter()
            .find(|h| h.id == height_id)
            .or_else(|| self.heights.first())
    }

    pub fn find_length(&self, nl_mm: f64) -> Option<&DrawerLengthVariant> {
        self.lengths
            .iter()
            .find(|l| (l.nl_mm - nl_mm).abs() < 0.5)
            .or_else(|| self.lengths.first())
    }
}

/// Library of general-purpose furniture drawer systems (`drawers.json`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DrawerLibrary {
    #[serde(default)]
    pub description: String,
    pub systems: Vec<DrawerSystem>,
}

impl DrawerLibrary {
    /// Loads `drawers.json`.
    pub fn load() -> Result<Self, String> {
        let lib: DrawerLibrary = load_hardware_json("drawers.json")?;
        if lib.systems.is_empty() {
            return Err("File 'drawers.json' contains no drawer systems.".into());
        }
        Ok(lib)
    }

    pub fn find_system(&self, system_id: &str) -> Option<&DrawerSystem> {
        self.systems
            .iter()
            .find(|s| s.id == system_id)
            .or_else(|| self.systems.first())
    }
}


/// Operating mode of the Edgebanding (`Obrzeża`) tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgebandMode {
    #[default]
    Manual,
    Auto,
}

/// Predefined automatic edgebanding rule in `EdgebandMode::Auto`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgebandAutoRule {
    #[default]
    ExposedOnly,
    FrontOnly,
    TopOnly,
    BottomOnly,
    FourSides,
    LongOnly,
    ShortOnly,
    CornerPair,
}

impl EdgebandAutoRule {
    pub const ALL: [EdgebandAutoRule; 8] = [
        EdgebandAutoRule::ExposedOnly,
        EdgebandAutoRule::FrontOnly,
        EdgebandAutoRule::TopOnly,
        EdgebandAutoRule::BottomOnly,
        EdgebandAutoRule::FourSides,
        EdgebandAutoRule::LongOnly,
        EdgebandAutoRule::ShortOnly,
        EdgebandAutoRule::CornerPair,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EdgebandAutoRule::ExposedOnly => "Exposed edges only",
            EdgebandAutoRule::FrontOnly => "Front edge only",
            EdgebandAutoRule::TopOnly => "Top edge only (+Z)",
            EdgebandAutoRule::BottomOnly => "Bottom edge only (-Z)",
            EdgebandAutoRule::FourSides => "All 4 sides (2L2W)",
            EdgebandAutoRule::LongOnly => "Two parallel: Long edges only (2L0W)",
            EdgebandAutoRule::ShortOnly => "Two parallel: Short edges only (0L2W)",
            EdgebandAutoRule::CornerPair => "Corner pair by clicked corner (1L1W)",
        }
    }
}

/// Edgebanding assignment for the 4 narrow edges of a rectangular panel.
/// Indexed by `[0] = MinAxis1`, `[1] = MaxAxis1`, `[2] = MinAxis2`, `[3] = MaxAxis2`,
/// where `(axis1, axis2)` with `axis1 < axis2` are the two axes perpendicular to `thin_axis`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelEdgebanding {
    pub edges: [Option<String>; 4],
}

impl PanelEdgebanding {
    pub fn any_banded(&self) -> bool {
        self.edges.iter().any(Option::is_some)
    }

    pub fn count_banded(&self) -> usize {
        self.edges.iter().filter(|e| e.is_some()).count()
    }
}

/// Operating mode of the Decor & Grain (`Dekor i Usłojenie`) tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecorMode {
    #[default]
    Add,
    Remove,
}

/// Assigned decor and grain direction stored per panel in document metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelDecor {
    pub decor_name: String,
    #[serde(default)]
    pub has_grain: bool,
    /// `false` = grain along panel length (0°); `true` = rotated 90° across width.
    #[serde(default)]
    pub grain_rotated: bool,
    pub color: String,
    #[serde(default = "default_decor_rgb")]
    pub rgb: [u8; 3],
}

/// Step 2 mode in the Free Panel (`Wolna Formatka`) 1 -> 2 -> 3 wizard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreePanelMode {
    #[default]
    TwoFaces,
    OneFaceAndWidth,
}

/// Direction mode for `Quick Formatka` (`Wewnątrz`, `Zewnątrz`, `Środek`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoPanelDirection {
    #[default]
    Inside,
    Outside,
    Center,
}

/// Which Wood feature dialog is currently open in the top-right of the 3D viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveDialog {
    Kubatura,
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
}

/// A 3D body detected in the model that is not yet nested in its own dedicated panel component under `Kubatura`
/// (e.g. loose body in `root`, body outside `Kubatura`, or a 2nd/split/mirrored body sharing an existing component).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdaptCandidate {
    pub body_name: String,
    pub current_component: String,
    pub reason: String,
    pub dims_mm: [f64; 3],
    pub selected: bool,
    pub cabinet_prefix: String,
    pub custom_name: String,
}

/// Metadata and parametric attributes attached to a furniture panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelData {
    pub name: String,
    pub role: String,
    pub thickness: f64,
    pub length: f64,
    pub width: f64,
    pub axis: String,
    pub normal_sign: f64,
    pub inward: bool,
    pub is_flipped: bool,
    pub offset: f64,
    pub reference_body: String,
    pub reference_point: [f64; 3],
}

/// One row of the furniture cutting list (`Tabela Rozkroju` / BOM).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CuttingRow {
    pub name: String,
    #[serde(default)]
    pub panel_number: usize,
    #[serde(default)]
    pub cabinet_edit: String,
    #[serde(default)]
    pub custom_name_edit: String,
    pub length_mm: f64,
    pub width_mm: f64,
    pub thickness_mm: f64,
    pub quantity: u32,
    pub area_m2: f64,
    #[serde(default = "default_zero_edgeband")]
    pub edgeband_code: String,
    #[serde(default)]
    pub edgeband_length_mm: f64,
    #[serde(default)]
    pub decor_name: String,
    #[serde(default)]
    pub grain_label: String,
}

fn default_zero_edgeband() -> String {
    "0D0K".into()
}

/// Crisp 3D directional arrow rendered on broad faces of panels when the Decor/Grain tool is active.
#[derive(Clone, Debug, PartialEq)]
pub struct GhostGrainArrow {
    pub panel_name: String,
    pub center: [f64; 3],
    pub dir: [f64; 3],
    pub perp: [f64; 3],
    pub normal: [f64; 3],
    pub half_len: f64,
    pub label: String,
    pub color: [u8; 4],
}

/// Subtle 3D quad overlay for a narrow edge face staged in the Edgebanding (`Obrzeża`) tool.
#[derive(Clone, Debug, PartialEq)]
pub struct GhostFacePreview {
    pub corners: [[f64; 3]; 4],
    pub color_stroke: [u8; 4],
    pub color_fill: [u8; 4],
}

/// 3D ghost box preview data including coordinate bounds, plane orientation ("XY", "XZ", "YZ"),
/// active margins and the orthogonal axes of the panel plane.
#[derive(Clone, Debug, PartialEq)]
pub struct GhostPreviewData {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub extra_boxes: Vec<([f64; 3], [f64; 3])>,
    pub plane: &'static str,
    pub margins: [f64; 4],
    pub axis1: usize,
    pub axis2: usize,
}

/// An individual 3D cylinder for ghost visualization (cup, screws, dowels, mounting plates).
#[derive(Clone, Debug, PartialEq)]
pub struct GhostCylinderItem {
    pub base_center: [f64; 3],  // punkt środka otworu na powierzchni bryły
    pub axis: [f64; 3],         // znormalizowany wektor w głąb materiału
    pub radius: f64,
    pub depth: f64,
    pub color_stroke: [u8; 4],
    pub color_fill: [u8; 4],
    pub label: String,
}

/// 3D ghost cylinder preview data for connectors (dowels/screws) and hinges (zawiasy).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GhostCylinderData {
    pub items: Vec<GhostCylinderItem>,
    pub centers: Vec<[f64; 3]>,
    pub axis: [f64; 3],
    pub radius: f64,
    pub depth_edge: f64,
    pub depth_face: f64,
    pub is_screw: bool, // false = kołek (zielony), true = wkręt (błękitny)
}

/// Serializable state of the Wood module, preserved seamlessly across DLL hot-reloads.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WoodState {
    pub active_dialog: Option<ActiveDialog>,
    pub bom_open: bool,
    pub created_panels: Vec<String>,
    pub panels: std::collections::HashMap<String, PanelData>,
    pub selected_panel_for_flip: String,
    pub cutting_list: Vec<CuttingRow>,
    pub export_step_dir: String,
    pub auto_panel_counter: u32,
    pub free_panel_counter: u32,

    // Selection state for Auto Panel and Free Panel (1 -> 2 -> 3)
    pub auto_face_pick: Option<FacePick>,
    #[serde(default)]
    pub auto_face_pick_2: Option<FacePick>,
    #[serde(default)]
    pub auto_active_slot: usize,
    pub free_picks: [Vec<FacePick>; 3],
    pub active_pick_slot: usize,

    // Status message inside the active Wood dialog
    pub status_message: String,
    pub status_error: bool,

    // Kubatura form fields
    pub szerokosc: String,
    pub glebokosc: String,
    pub wysokosc: String,
    pub grubosc: String,
    pub kubatura_opacity: String,
    pub auto_create_sides: bool,

    // Quick Formatka form fields
    pub auto_name: String,
    pub auto_inward: bool,
    #[serde(default)]
    pub auto_direction: AutoPanelDirection,
    #[serde(default = "default_one_str")]
    pub auto_division_count: String,
    #[serde(skip)]
    pub auto_clearances_info: String,
    #[serde(skip)]
    pub auto_clearances_error: bool,
    pub auto_thickness: String,
    pub auto_plane_offset: String,
    pub auto_margins: [String; 4],
    pub auto_margins_linked: bool,

    // FORMATKA (1 -> 2 -> 3) form fields
    pub free_name: String,
    pub free_mode: FreePanelMode,
    pub free_width: String,
    /// Inward margin from each picked face: [step 1 face A, step 1 face B, step 2 face A, step 2 face B].
    pub free_margins: [String; 4],
    pub free_margins_linked: bool,
    pub free_plane_offset: String,
    pub free_inward: bool,
    pub free_thickness: String,

    // Złącza (Kołki / Wkręty) form fields
    pub connector_edge_pick: Option<EdgePick>,
    pub connector_face_pick: Option<FacePick>,
    #[serde(skip)]
    pub connector_last_edge_pick: Option<EdgePick>,
    #[serde(skip)]
    pub connector_last_face_pick: Option<FacePick>,
    pub connector_active_slot: usize,
    pub connector_category_id: String,
    pub connector_preset_id: String,
    pub connector_through_face: bool,
    pub connector_offset_x: String,
    pub connector_offset_shift: String,
    pub connector_shift_sign: f64,
    pub connector_count: String,
    pub connector_counter: u32,

    // Zawiasy (Hinges) form fields
    pub hinge_face_picks: [Option<FacePick>; 2],
    pub hinge_active_slot: usize,
    pub hinge_standard_id: String,
    pub hinge_cup_fixing: String,
    pub hinge_plate_screw: String,
    pub hinge_offset_x: String,
    pub hinge_count: String,
    pub hinge_counter: u32,

    // Hot-reload tracking
    pub hot_reload_generation: u32,
    pub reload_requested: bool,

    // Kubatura visibility & panel translucency (0.60)
    pub kubatura_hidden: bool,
    #[serde(default)]
    pub panels_translucent: bool,

    // Naming & AdaptBodies & BOM editing state
    #[serde(default = "default_cabinet_prefix")]
    pub cabinet_prefix: String,
    #[serde(default)]
    pub highest_panel_number: usize,
    #[serde(default)]
    pub adapt_candidates: Vec<AdaptCandidate>,
    #[serde(default)]
    pub bom_range_from: String,
    #[serde(default)]
    pub bom_range_to: String,
    #[serde(default = "default_cabinet_prefix")]
    pub bom_range_cabinet: String,

    // Obrzeża (Edgebanding) state
    #[serde(default = "default_edgeband_preset")]
    pub edgeband_preset_id: String,
    #[serde(default)]
    pub edgeband_mode: EdgebandMode,
    #[serde(default)]
    pub edgeband_auto_rule: EdgebandAutoRule,
    #[serde(default)]
    pub panel_edgebands: std::collections::BTreeMap<String, PanelEdgebanding>,
    #[serde(default)]
    pub staged_edgebands: std::collections::BTreeMap<String, PanelEdgebanding>,
    #[serde(default)]
    pub edgeband_last_panel: String,

    // Dekor i Usłojenie (Decor & Grain) state
    #[serde(default = "default_decor_preset")]
    pub decor_preset_id: String,
    #[serde(default)]
    pub decor_mode: DecorMode,
    #[serde(default)]
    pub decor_custom_name: String,
    #[serde(default = "default_true_bool")]
    pub decor_custom_has_grain: bool,
    #[serde(default = "default_decor_rgb")]
    pub decor_custom_rgb: [u8; 3],
    #[serde(default)]
    pub panel_decors: std::collections::BTreeMap<String, PanelDecor>,
    #[serde(default)]
    pub decor_last_panel: String,

    // Połączenie T-Dogbone (CNC Mortise & Tenon) state
    #[serde(default = "default_tdogbone_preset")]
    pub tdogbone_preset_id: String,
    #[serde(default)]
    pub tdogbone_edge_pick: Option<EdgePick>,
    #[serde(default)]
    pub tdogbone_face_pick: Option<FacePick>,
    #[serde(skip)]
    pub tdogbone_last_edge_pick: Option<EdgePick>,
    #[serde(skip)]
    pub tdogbone_last_face_pick: Option<FacePick>,
    #[serde(default)]
    pub tdogbone_active_slot: usize,
    #[serde(default = "default_hundred_str")]
    pub tdogbone_width: String,
    #[serde(default = "default_hundred_str")]
    pub tdogbone_offset_x: String,
    #[serde(default = "default_one_str")]
    pub tdogbone_count: String,
    #[serde(default = "default_one_u32")]
    pub tdogbone_counter: u32,

    // DRAWER (Systemy Szuflad) state
    #[serde(default)]
    pub drawer_face_picks: [Option<FacePick>; 4],
    #[serde(default)]
    pub drawer_active_slot: usize,
    #[serde(default = "default_drawer_system")]
    pub drawer_system_id: String,
    #[serde(default = "default_drawer_height")]
    pub drawer_height_id: String,
    #[serde(default = "default_drawer_nl")]
    pub drawer_nl_mm: String,
    #[serde(default = "default_zero_str")]
    pub drawer_lift_z: String,
    #[serde(default = "default_one_u32")]
    pub drawer_counter: u32,
    #[serde(skip)]
    pub drawer_validation_info: String,
    #[serde(skip)]
    pub drawer_validation_error: bool,

    // Dynamic 3D ghost box preview ([min, max, plane])
    #[serde(skip)]
    pub ghost_box: Option<GhostPreviewData>,
    #[serde(skip)]
    pub ghost_cylinders: Option<GhostCylinderData>,
    #[serde(skip)]
    pub ghost_faces: Vec<GhostFacePreview>,
    #[serde(skip)]
    pub ghost_arrows: Vec<GhostGrainArrow>,
}

fn default_decor_preset() -> String {
    "DAB_CRAFT_ZLOTY".into()
}

fn default_true_bool() -> bool {
    true
}

fn default_tdogbone_preset() -> String {
    "T_DOGBONE_100_F12_W8".into()
}

fn default_hundred_str() -> String {
    "100".into()
}

fn default_edgeband_preset() -> String {
    "ABS_08".into()
}

fn default_cabinet_prefix() -> String {
    "S01".into()
}

fn default_one_str() -> String {
    "1".into()
}

fn default_zero_str() -> String {
    "0".into()
}

fn default_drawer_system() -> String {
    "SEVROLL_VBOX_18_3D_SLIM".into()
}

fn default_drawer_height() -> String {
    "H127".into()
}

fn default_drawer_nl() -> String {
    "450".into()
}

impl Default for WoodState {
    fn default() -> Self {
        Self {
            active_dialog: None,
            bom_open: false,
            created_panels: Vec::new(),
            panels: std::collections::HashMap::new(),
            selected_panel_for_flip: String::new(),
            cutting_list: Vec::new(),
            export_step_dir: "C:/SolidCraft_Export/STEP".into(),
            auto_panel_counter: 1,
            free_panel_counter: 1,
            auto_face_pick: None,
            auto_face_pick_2: None,
            auto_active_slot: 0,
            free_picks: [Vec::new(), Vec::new(), Vec::new()],
            active_pick_slot: 0,
            status_message: String::new(),
            status_error: false,
            szerokosc: "800".into(),
            glebokosc: "560".into(),
            wysokosc: "720".into(),
            grubosc: "18".into(),
            kubatura_opacity: "0.18".into(),
            auto_create_sides: false,
            auto_name: "S01_01".into(),
            auto_inward: true,
            auto_direction: AutoPanelDirection::Inside,
            auto_division_count: "1".into(),
            auto_clearances_info: String::new(),
            auto_clearances_error: false,
            auto_thickness: "Grubosc".into(),
            auto_plane_offset: "0".into(),
            auto_margins: ["0".into(), "0".into(), "0".into(), "0".into()],
            auto_margins_linked: false,
            free_name: "S01_01".into(),
            free_mode: FreePanelMode::TwoFaces,
            free_width: "100".into(),
            free_margins: ["0".into(), "0".into(), "0".into(), "0".into()],
            free_margins_linked: false,
            free_plane_offset: "0".into(),
            free_inward: true,
            free_thickness: "Grubosc".into(),
            connector_edge_pick: None,
            connector_face_pick: None,
            connector_last_edge_pick: None,
            connector_last_face_pick: None,
            connector_active_slot: 0,
            connector_category_id: "dowels".into(),
            connector_preset_id: "dowel_8x35".into(),
            connector_through_face: false,
            connector_offset_x: "50".into(),
            connector_offset_shift: "0".into(),
            connector_shift_sign: 1.0,
            connector_count: "2".into(),
            connector_counter: 1,
            hinge_face_picks: [None, None],
            hinge_active_slot: 0,
            hinge_standard_id: "BLUM_45_9.5".into(),
            hinge_cup_fixing: "dowel".into(),
            hinge_plate_screw: "euro".into(),
            hinge_offset_x: "100".into(),
            hinge_count: "2".into(),
            hinge_counter: 1,
            hot_reload_generation: 0,
            reload_requested: false,
            kubatura_hidden: false,
            panels_translucent: false,
            cabinet_prefix: "S01".into(),
            highest_panel_number: 0,
            adapt_candidates: Vec::new(),
            bom_range_from: "1".into(),
            bom_range_to: "10".into(),
            bom_range_cabinet: "S01".into(),
            edgeband_preset_id: "ABS_08".into(),
            edgeband_mode: EdgebandMode::Manual,
            edgeband_auto_rule: EdgebandAutoRule::ExposedOnly,
            panel_edgebands: std::collections::BTreeMap::new(),
            staged_edgebands: std::collections::BTreeMap::new(),
            edgeband_last_panel: String::new(),
            decor_preset_id: default_decor_preset(),
            decor_mode: DecorMode::Add,
            decor_custom_name: String::new(),
            decor_custom_has_grain: true,
            decor_custom_rgb: default_decor_rgb(),
            panel_decors: std::collections::BTreeMap::new(),
            decor_last_panel: String::new(),
            tdogbone_preset_id: "T_DOGBONE_100_F12_W8".into(),
            tdogbone_edge_pick: None,
            tdogbone_face_pick: None,
            tdogbone_last_edge_pick: None,
            tdogbone_last_face_pick: None,
            tdogbone_active_slot: 0,
            tdogbone_width: "100".into(),
            tdogbone_offset_x: "100".into(),
            tdogbone_count: "1".into(),
            tdogbone_counter: 1,
            drawer_face_picks: [None, None, None, None],
            drawer_active_slot: 0,
            drawer_system_id: default_drawer_system(),
            drawer_height_id: default_drawer_height(),
            drawer_nl_mm: default_drawer_nl(),
            drawer_lift_z: default_zero_str(),
            drawer_counter: 1,
            drawer_validation_info: String::new(),
            drawer_validation_error: false,
            ghost_box: None,
            ghost_cylinders: None,
            ghost_faces: Vec::new(),
            ghost_arrows: Vec::new(),
        }
    }
}

impl WoodState {
    /// Resets all per-document (`.sc` file) Wood state (panel numbering, panels, BOM, edgebands,
    /// tool picks, counters) while preserving process-wide DLL hot-reload and export path settings.
    pub fn reset_for_document(&mut self) {
        let hot_reload_generation = self.hot_reload_generation;
        let reload_requested = self.reload_requested;
        let export_step_dir = std::mem::take(&mut self.export_step_dir);
        *self = Self::default();
        self.hot_reload_generation = hot_reload_generation;
        self.reload_requested = reload_requested;
        if !export_step_dir.is_empty() {
            self.export_step_dir = export_step_dir;
        }
    }
}

/// Host callbacks implemented by `solidcraft-ui-egui` (`SolveApp`) so `crates/wood` can execute
/// engine commands, inspect the 3D model, read viewport face/edge selections, and update the status bar
/// without depending on the heavy B-rep kernel crates.
pub trait WoodHost {
    /// Executes a single SolidCraft engine command (e.g. `solid.extrude`, `solid.hole`).
    fn execute(&mut self, command: &str, params: Value) -> Result<Value, String>;

    /// Executes a sequence of commands atomically (rolling back on error when `rollback` is `true`).
    fn batch(&mut self, commands: &[(String, Value)], rollback: bool) -> Result<Value, String>;

    /// Runs `document.inspect` with `measure` to query bodies, faces, normals, bounding boxes, and components.
    fn inspect_document(&mut self, measure: bool) -> Result<Value, String>;

    /// Drains any newly clicked 3D faces from the viewport selection and clears the viewport selection.
    fn take_selected_faces(&mut self) -> Vec<FacePick>;

    /// Drains any newly clicked 3D edges from the viewport selection and clears the viewport selection.
    fn take_selected_edges(&mut self) -> Vec<EdgePick>;

    /// Switches the viewport selection filter to `"face"` mode and clears current selection.
    fn enable_face_selection(&mut self);

    /// Switches the viewport selection filter to `"edge"` mode (disabling face picks so edges are easy to click).
    fn enable_edge_selection(&mut self);

    /// Displays a status message in SolidCraft's bottom status bar.
    fn set_status(&mut self, message: &str, is_error: bool);

    /// Opens SolidCraft's built-in Change Parameters (`parameters.change`) dialog.
    fn open_parameters_dialog(&mut self);
}

/// Function pointer table exported by `wood.dll` (`cdylib`) for hot-reloading and also available statically (`rlib`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WoodPluginVTable {
    pub abi_version: u32,
    pub consume_viewport_picks: fn(&mut WoodState, &mut dyn WoodHost),
    pub draw_ribbon: fn(&mut egui::Ui, egui::Rect, &mut WoodState, &mut dyn WoodHost),
    pub draw_dialog_body: fn(&mut egui::Ui, &mut WoodState, &mut dyn WoodHost) -> bool,
    pub draw_bom_body: fn(&mut egui::Ui, &mut WoodState, &mut dyn WoodHost),
}

impl WoodPluginVTable {
    pub const fn static_vtable() -> Self {
        Self {
            abi_version: WOOD_ABI_VERSION,
            consume_viewport_picks: ui::consume_viewport_picks,
            draw_ribbon: ui::draw_ribbon,
            draw_dialog_body: ui::draw_dialog_body,
            draw_bom_body: ui::draw_bom_body,
        }
    }
}

/// Exported entry point for dynamic hot-reloading (`target/debug/wood.dll`).
#[allow(unsafe_code)]
#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn solidcraft_wood_vtable_v1() -> WoodPluginVTable {
    WoodPluginVTable::static_vtable()
}
