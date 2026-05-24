//! On-disk presets.
//!
//! A preset captures both the parameter values and the control mappings, so a
//! performer can recall a complete patch - what it looks like *and* how their
//! controller is wired to it.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::params::{Mapping, ModRoute, ParamValue};

/// A saved patch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Preset {
    /// Format version, so we can migrate older presets if the schema changes.
    #[serde(default = "preset_version")]
    pub version: u32,
    /// Free-text name shown in the UI.
    #[serde(default)]
    pub name: String,
    /// Parameter values keyed by path. Unknown paths are ignored on load.
    #[serde(default)]
    pub params: HashMap<String, ParamValue>,
    /// Control bindings.
    #[serde(default)]
    pub mappings: Vec<Mapping>,
    /// Modulation routes (source -> param with amount).
    #[serde(default)]
    pub mod_routes: Vec<ModRoute>,
}

fn preset_version() -> u32 {
    1
}

impl Preset {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading preset {}", path.display()))?;
        let preset: Preset = serde_json::from_str(&text)
            .with_context(|| format!("parsing preset {}", path.display()))?;
        Ok(preset)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text)
            .with_context(|| format!("writing preset {}", path.display()))?;
        Ok(())
    }
}
