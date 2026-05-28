//! Filesystem-backed implementation of [`audiovis_render_core::Resources`].
//!
//! The native binary scans the `media/`, `meshes/`, and `isf/` directories
//! (each overridable via `AV_*_DIR`) so the render-core's media / mesh / ISF
//! banks can stay portable: they ask this provider for the byte content of a
//! named file, without caring whether it lives on disk, was preloaded into RAM,
//! or arrived over the network (as it does in the wasm build).

use std::path::PathBuf;

use audiovis_render_core::Resources;

/// Reads resources from the local filesystem.
pub struct DiskResources;

impl DiskResources {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DiskResources {
    fn default() -> Self {
        Self::new()
    }
}

impl Resources for DiskResources {
    fn media_names(&self) -> Vec<String> {
        list_dir(env_or("AV_MEDIA_DIR", "media"), &["png", "jpg", "jpeg", "svg"])
    }

    fn read_media(&self, name: &str) -> Option<Vec<u8>> {
        let path = PathBuf::from(env_or("AV_MEDIA_DIR", "media")).join(name);
        std::fs::read(path).ok()
    }

    fn mesh_names(&self) -> Vec<String> {
        list_dir(env_or("AV_MESH_DIR", "meshes"), &["obj"])
    }

    fn read_mesh(&self, name: &str) -> Option<String> {
        let path = PathBuf::from(env_or("AV_MESH_DIR", "meshes")).join(name);
        std::fs::read_to_string(path).ok()
    }

    fn isf_names(&self) -> Vec<String> {
        list_dir(env_or("AV_ISF_DIR", "isf"), &["fs", "frag", "glsl"])
    }

    fn read_isf(&self, name: &str) -> Option<String> {
        let path = PathBuf::from(env_or("AV_ISF_DIR", "isf")).join(name);
        std::fs::read_to_string(path).ok()
    }
}

/// `AV_*_DIR` override or the given default.
fn env_or(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

/// File names in `dir` whose extension is in `exts` (lowercase), sorted.
fn list_dir(dir: String, exts: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| exts.iter().any(|w| e.eq_ignore_ascii_case(w)))
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out
}
