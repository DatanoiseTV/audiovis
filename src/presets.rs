//! Preset storage and resolution.
//!
//! Presets come from two places: a set of curated builtins embedded in the
//! binary (`assets/presets/`), and user presets saved at runtime under
//! `presets/` next to the working directory. A user preset of the same name
//! shadows a builtin, so the bundled ones can be tweaked and re-saved. The last
//! loaded/saved preset name is remembered so it can be auto-recalled on launch.

use std::path::PathBuf;

use audiovis_render_core::config::Preset;

/// Curated presets shipped inside the binary.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/presets/"]
struct Builtin;

pub struct PresetStore {
    dir: PathBuf,
}

impl PresetStore {
    /// User presets live in `presets/` relative to the working directory.
    pub fn new() -> Self {
        Self { dir: PathBuf::from("presets") }
    }

    fn user_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    fn last_path(&self) -> PathBuf {
        self.dir.join(".last")
    }

    /// All preset names: builtins plus user files, de-duplicated and sorted.
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = Builtin::iter()
            .filter_map(|f| f.strip_suffix(".json").map(str::to_string))
            .collect();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for entry in rd.flatten() {
                if let Some(stem) = entry.path().file_stem().and_then(|s| s.to_str()) {
                    if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                        names.push(stem.to_string());
                    }
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }

    /// Resolve a preset by name: a user file wins, otherwise the builtin.
    pub fn load(&self, name: &str) -> Option<Preset> {
        let user = self.user_path(name);
        if user.exists() {
            match Preset::load(&user) {
                Ok(p) => return Some(p),
                Err(e) => tracing::warn!("preset '{name}' failed to load: {e:#}"),
            }
        }
        let file = Builtin::get(&format!("{name}.json"))?;
        match serde_json::from_slice::<Preset>(&file.data) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!("builtin preset '{name}' is invalid: {e}");
                None
            }
        }
    }

    /// Save a preset to the user directory.
    pub fn save(&self, name: &str, preset: &Preset) -> anyhow::Result<()> {
        preset.save(self.user_path(name))
    }

    pub fn set_last(&self, name: &str) {
        if std::fs::create_dir_all(&self.dir).is_ok() {
            let _ = std::fs::write(self.last_path(), name);
        }
    }

    pub fn last(&self) -> Option<String> {
        std::fs::read_to_string(self.last_path()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }
}

impl Default for PresetStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a builtin of this name exists (used to pick a sane default).
pub fn builtin_exists(name: &str) -> bool {
    Builtin::get(&format!("{name}.json")).is_some()
}

/// Validate every embedded builtin parses - guards against shipping a broken one.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtins_parse() {
        let mut count = 0;
        for f in Builtin::iter() {
            let data = Builtin::get(&f).unwrap();
            serde_json::from_slice::<Preset>(&data.data).unwrap_or_else(|e| panic!("builtin {f} invalid: {e}"));
            count += 1;
        }
        assert!(count > 0, "no builtin presets embedded");
    }
}
