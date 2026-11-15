//! Dynamic Hot-Reloading connector (`cdylib` + `libloading`) with static fallback for production builds.
//!
//! In development (`--features dynamic-wood`), `WoodRuntime` loads `target/debug/wood.dll`
//! (or `libwood.so` / `libwood.dylib`), copying it first to a shadow file (`wood_hot_<gen>.dll`)
//! so Windows never locks `target/debug/wood.dll` while `cargo build -p wood` is running.
//! Pressing `F5` (or clicking the ribbon badge) reloads the DLL in-place while preserving `WoodState`.

use crate::{WoodHost, WoodPluginVTable, WoodState};

/// Manages the `WoodState` and dispatches UI & modeling calls either to a dynamically loaded
/// `wood.dll` (`cdylib`) or to the statically linked crate (`rlib`).
pub struct WoodRuntime {
    pub state: WoodState,
    vtable: WoodPluginVTable,
    #[cfg(all(feature = "dynamic-wood", not(target_arch = "wasm32")))]
    dynamic: Option<LoadedWoodLibrary>,
}

#[cfg(all(feature = "dynamic-wood", not(target_arch = "wasm32")))]
struct LoadedWoodLibrary {
    _lib: libloading::Library,
    shadow_path: std::path::PathBuf,
    source_modified: Option<std::time::SystemTime>,
}

#[cfg(all(feature = "dynamic-wood", not(target_arch = "wasm32")))]
impl Drop for LoadedWoodLibrary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.shadow_path);
    }
}

impl Default for WoodRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl WoodRuntime {
    pub fn new() -> Self {
        Self {
            state: WoodState::default(),
            vtable: WoodPluginVTable::static_vtable(),
            #[cfg(all(feature = "dynamic-wood", not(target_arch = "wasm32")))]
            dynamic: None,
        }
    }

    /// Checks whether `F5` or the UI reload button was pressed and hot-reloads `wood.dll` if requested.
    pub fn poll_hot_reload(&mut self, f5_pressed: bool, host: &mut dyn WoodHost) {
        let manual = f5_pressed || self.state.reload_requested;
        self.state.reload_requested = false;

        if !manual {
            return;
        }

        #[cfg(all(feature = "dynamic-wood", not(target_arch = "wasm32")))]
        {
            match self.try_reload_from_disk(true) {
                Ok(msg) => {
                    if !msg.is_empty() {
                        self.state.status_message = msg.clone();
                        self.state.status_error = false;
                        host.set_status(&msg, false);
                    }
                }
                Err(err) => {
                    self.state.status_message = err.clone();
                    self.state.status_error = true;
                    host.set_status(&err, true);
                }
            }
        }

        #[cfg(not(all(feature = "dynamic-wood", not(target_arch = "wasm32"))))]
        {
            let msg = "Wood działa w trybie statycznym.";
            self.state.status_message = msg.to_string();
            self.state.status_error = false;
            host.set_status(msg, false);
        }
    }

    pub fn consume_viewport_picks(&mut self, host: &mut dyn WoodHost) {
        (self.vtable.consume_viewport_picks)(&mut self.state, host);
    }

    pub fn draw_ribbon(&mut self, ui: &mut egui::Ui, rect: egui::Rect, host: &mut dyn WoodHost) {
        (self.vtable.draw_ribbon)(ui, rect, &mut self.state, host);
    }

    pub fn draw_dialog_body(&mut self, ui: &mut egui::Ui, host: &mut dyn WoodHost) -> bool {
        (self.vtable.draw_dialog_body)(ui, &mut self.state, host)
    }

    pub fn draw_bom_body(&mut self, ui: &mut egui::Ui, host: &mut dyn WoodHost) {
        (self.vtable.draw_bom_body)(ui, &mut self.state, host);
    }

    #[cfg(all(feature = "dynamic-wood", not(target_arch = "wasm32")))]
    #[allow(unsafe_code)]
    fn try_reload_from_disk(&mut self, force: bool) -> Result<String, String> {
        let Some(dll_path) = discover_wood_dll_path() else {
            return Err("Nie znaleziono pliku target/debug/wood.dll. Zbuduj go poleceniem: cargo build -p wood".into());
        };

        let modified = std::fs::metadata(&dll_path).and_then(|m| m.modified()).ok();
        if !force
            && let Some(loaded) = &self.dynamic
            && loaded.source_modified == modified
        {
            return Ok(String::new());
        }

        // Preserve serialized state across hot-reload
        let state_snapshot = serde_json::to_string(&self.state).ok();
        let next_gen = self.state.hot_reload_generation + 1;

        let ext = dll_path.extension().and_then(|e| e.to_str()).unwrap_or("dll");
        let pid = std::process::id();
        let time_salt = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let shadow_path = std::env::temp_dir().join(format!("solidcraft_wood_hot_{pid}_{next_gen}_{time_salt}.{ext}"));

        std::fs::copy(&dll_path, &shadow_path)
            .map_err(|e| format!("Nie udało się skopiować {} do {}: {e}", dll_path.display(), shadow_path.display()))?;

        // Drop previous library before loading the new generation
        self.dynamic = None;

        // SAFETY: `shadow_path` is a freshly compiled `wood` cdylib from this workspace exporting
        // `solidcraft_wood_vtable_v1` with the matching `WoodPluginVTable` ABI.
        let (lib, vtable) = unsafe {
            let lib = libloading::Library::new(&shadow_path)
                .map_err(|e| format!("Błąd ładowania biblioteki {}: {e}", shadow_path.display()))?;
            let entry: libloading::Symbol<'_, unsafe extern "C" fn() -> WoodPluginVTable> = lib
                .get(b"solidcraft_wood_vtable_v1\0")
                .map_err(|e| format!("Brak symbolu `solidcraft_wood_vtable_v1` w {}: {e}", shadow_path.display()))?;
            let vt = entry();
            if vt.abi_version != crate::WOOD_ABI_VERSION {
                return Err(format!(
                    "Niezgodna wersja ABI wood.dll (otrzymano {}, oczekiwano {}).",
                    vt.abi_version,
                    crate::WOOD_ABI_VERSION
                ));
            }
            (lib, vt)
        };

        if let Some(json_str) = state_snapshot
            && let Ok(restored) = serde_json::from_str::<WoodState>(&json_str)
        {
            self.state = restored;
        }
        self.state.hot_reload_generation = next_gen;
        self.vtable = vtable;
        self.dynamic = Some(LoadedWoodLibrary {
            _lib: lib,
            shadow_path,
            source_modified: modified,
        });

        log::info!("Hot-reloaded wood plugin (#{}) from {}", next_gen, dll_path.display());
        Ok(format!("Przeładowano dynamicznie wood.dll (#{next_gen})"))
    }
}

#[cfg(all(feature = "dynamic-wood", not(target_arch = "wasm32")))]
fn discover_wood_dll_path() -> Option<std::path::PathBuf> {
    if let Some(custom) = std::env::var_os("SOLIDCRAFT_WOOD_DLL") {
        let p = std::path::PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }

    let filename = if cfg!(target_os = "windows") {
        "wood.dll"
    } else if cfg!(target_os = "macos") {
        "libwood.dylib"
    } else {
        "libwood.so"
    };

    let mut candidates = vec![
        std::path::PathBuf::from("target").join("debug").join(filename),
        std::path::PathBuf::from("target").join("release").join(filename),
    ];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            std::path::PathBuf::from(local)
                .join("SolidCraft")
                .join("plugins")
                .join("solidcraft-wood")
                .join(filename),
        );
    }
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let beside_exe = dir.join(filename);
        if beside_exe.is_file() {
            return Some(beside_exe);
        }
    }

    None
}
