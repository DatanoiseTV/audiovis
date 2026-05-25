//! Wireframe mesh rendering.
//!
//! Loads OBJ models from a `meshes/` directory and draws them as rotating
//! wireframes with GL_LINES, so the "wireframe" generator can show real geometry
//! (any OBJ you drop in) as well as its built-in procedural shapes. Faces are
//! reduced to a unique edge set on load; vertices are normalised to a unit
//! sphere so any model frames sensibly.

use std::collections::HashSet;
use std::path::PathBuf;

use glow::HasContext as _;

use crate::engine::Engine;
use crate::params::{ParamId, ParamKind, ParamSpec};

use super::gl::{self, Gl, GlslFlavor, Program};

/// Upper bound on selectable meshes (the source param is registered with this
/// max so files added later and picked up by a rescan stay selectable).
const MAX_MESHES: i64 = 31;

/// A loaded mesh uploaded to the GPU as a line list.
struct Loaded {
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    vao: Option<glow::VertexArray>,
    index_count: i32,
}

pub struct MeshBank {
    gl: Gl,
    prog: Program,
    needs_vao: bool,
    /// `wire.mesh`: 0 = procedural shapes, 1.. = a loaded OBJ.
    mesh_param: ParamId,
    files: Vec<PathBuf>,
    names: Vec<String>,
    loaded: Option<Loaded>,
    last_src: i64,
}

impl MeshBank {
    pub fn new(gl: &Gl, flavor: GlslFlavor, engine: &mut Engine) -> Result<Self, String> {
        let vert = include_str!("shaders/mesh/wire.vert");
        let frag = include_str!("shaders/mesh/wire.frag");
        let prog = Program::new(gl, flavor, vert, frag).map_err(|e| format!("wire mesh: {e}"))?;

        let (files, names) = scan_mesh_dir();
        let mesh_param = engine.params_mut().register(ParamSpec::new(
            "wire.mesh",
            "Mesh",
            "Wireframe",
            ParamKind::Int { min: 0, max: MAX_MESHES, default: 0 },
        ));

        Ok(Self {
            gl: gl.clone(),
            prog,
            needs_vao: flavor.needs_vao(),
            mesh_param,
            files,
            names,
            loaded: None,
            last_src: -1,
        })
    }

    /// Dropdown labels for the mesh source param (index 0 = procedural shapes).
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Re-scan the mesh directory for newly added files.
    pub fn rescan(&mut self) {
        let (files, names) = scan_mesh_dir();
        self.files = files;
        self.names = names;
        self.last_src = -1;
    }

    /// Whether a loaded OBJ (not the procedural shapes) is currently selected.
    pub fn active(&self, engine: &Engine) -> bool {
        engine.params().get(self.mesh_param).as_i64() > 0
    }

    /// Draw the selected mesh into the currently-bound target. `hue` tints the
    /// lines, `spin` scales the rotation speed, `res` is the target size.
    pub fn render(&mut self, engine: &Engine, time: f32, hue: f32, spin: f32, audio_low: f32, res: (f32, f32)) {
        let src = engine.params().get(self.mesh_param).as_i64().max(0);
        if src != self.last_src {
            self.load(src);
            self.last_src = src;
        }
        let Some(m) = self.loaded.as_ref() else {
            gl::clear(&self.gl, 0.0, 0.0, 0.0);
            return;
        };

        gl::clear(&self.gl, 0.0, 0.0, 0.0);
        let yaw = time * (0.2 + spin * 1.2);
        let pitch = time * 0.17 + 0.4;
        unsafe {
            self.prog.bind();
            self.prog.set_vec2("u_rot", yaw, pitch);
            self.prog.set_vec2("u_res", res.0, res.1);
            self.prog.set_f32("u_hue", hue);
            self.prog.set_f32("u_audio", audio_low);
            self.gl.line_width(2.0);
            if let Some(vao) = m.vao {
                self.gl.bind_vertex_array(Some(vao));
            } else {
                self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(m.vbo));
                self.gl.enable_vertex_attrib_array(0);
                self.gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 0, 0);
                self.gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(m.ebo));
            }
            self.gl.draw_elements(glow::LINES, m.index_count, glow::UNSIGNED_SHORT, 0);
            if m.vao.is_some() {
                self.gl.bind_vertex_array(None);
            }
        }
    }

    /// Load (or clear) the GPU buffers for a mesh when the selection changes.
    fn load(&mut self, src: i64) {
        // Drop the previous buffers first.
        if let Some(m) = self.loaded.take() {
            unsafe {
                self.gl.delete_buffer(m.vbo);
                self.gl.delete_buffer(m.ebo);
                if let Some(v) = m.vao {
                    self.gl.delete_vertex_array(v);
                }
            }
        }
        let idx = src as usize;
        if idx == 0 || idx > self.files.len() {
            return; // procedural shapes (handled by the fragment generator)
        }
        let path = &self.files[idx - 1];
        match load_obj(path) {
            Some((positions, indices)) if !indices.is_empty() => {
                self.loaded = Some(self.upload(&positions, &indices));
                tracing::info!("wireframe: loaded {} ({} edges)", path.display(), indices.len() / 2);
            }
            _ => tracing::warn!("wireframe: could not load {}", path.display()),
        }
    }

    fn upload(&self, positions: &[f32], indices: &[u16]) -> Loaded {
        unsafe {
            let vao = if self.needs_vao {
                let v = self.gl.create_vertex_array().expect("vao");
                self.gl.bind_vertex_array(Some(v));
                Some(v)
            } else {
                None
            };
            let vbo = self.gl.create_buffer().expect("vbo");
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            self.gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(positions), glow::STATIC_DRAW);

            let ebo = self.gl.create_buffer().expect("ebo");
            self.gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            self.gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, as_bytes(indices), glow::STATIC_DRAW);

            if vao.is_some() {
                self.gl.enable_vertex_attrib_array(0);
                self.gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 0, 0);
                self.gl.bind_vertex_array(None);
            }
            Loaded { vbo, ebo, vao, index_count: indices.len() as i32 }
        }
    }
}

/// Find the mesh directory (`AV_MESH_DIR`, default `meshes`) and list OBJ files.
fn scan_mesh_dir() -> (Vec<PathBuf>, Vec<String>) {
    let dir = std::env::var("AV_MESH_DIR").unwrap_or_else(|_| "meshes".to_string());
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("obj")).unwrap_or(false) {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut names = vec!["(shapes)".to_string()];
    names.extend(files.iter().map(|p| p.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string()));
    (files, names)
}

/// Parse an OBJ file into normalised vertex positions and a unique edge list.
fn load_obj(path: &std::path::Path) -> Option<(Vec<f32>, Vec<u16>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut verts: Vec<[f32; 3]> = Vec::new();
    let mut edges: HashSet<(u16, u16)> = HashSet::new();

    // Resolve an OBJ index token ("v", "v/vt", "v/vt/vn", possibly negative).
    let resolve = |tok: &str, nverts: usize| -> Option<u16> {
        let i: i64 = tok.split('/').next()?.parse().ok()?;
        let idx = if i < 0 { nverts as i64 + i } else { i - 1 };
        if idx >= 0 && (idx as usize) < nverts {
            Some(idx as u16)
        } else {
            None
        }
    };

    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let c: Vec<f32> = it.take(3).filter_map(|t| t.parse().ok()).collect();
                if c.len() == 3 {
                    verts.push([c[0], c[1], c[2]]);
                }
            }
            Some("f") | Some("l") => {
                if verts.len() > 65535 {
                    return None; // u16 indices cannot address this many vertices
                }
                let n = verts.len();
                let ring: Vec<u16> = it.filter_map(|t| resolve(t, n)).collect();
                let closed = line.starts_with('f'); // faces wrap, polylines do not
                let count = ring.len();
                if count >= 2 {
                    let segs = if closed { count } else { count - 1 };
                    for k in 0..segs {
                        let (a, b) = (ring[k], ring[(k + 1) % count]);
                        edges.insert(if a < b { (a, b) } else { (b, a) });
                    }
                }
            }
            _ => {}
        }
    }

    if verts.is_empty() {
        return None;
    }
    // Centre on the bounding-box midpoint and scale to a unit sphere.
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for v in &verts {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    let center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
    let mut radius = 0.0f32;
    for v in &verts {
        let d = ((v[0] - center[0]).powi(2) + (v[1] - center[1]).powi(2) + (v[2] - center[2]).powi(2)).sqrt();
        radius = radius.max(d);
    }
    let inv = if radius > 1e-6 { 1.0 / radius } else { 1.0 };

    let positions: Vec<f32> = verts
        .iter()
        .flat_map(|v| [(v[0] - center[0]) * inv, (v[1] - center[1]) * inv, (v[2] - center[2]) * inv])
        .collect();
    let indices: Vec<u16> = edges.iter().flat_map(|&(a, b)| [a, b]).collect();
    Some((positions, indices))
}

/// View a slice of plain data as bytes for buffer uploads.
fn as_bytes<T>(s: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, std::mem::size_of_val(s)) }
}
