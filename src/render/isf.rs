//! ISF (Interactive Shader Format) support.
//!
//! Loads ISF `.fs` shaders from an `isf/` directory: parses the JSON header in
//! the leading comment, injects a preamble that maps ISF's conventions
//! (`RENDERSIZE`, `TIME`, `IMG_PIXEL`, `gl_FragColor`, …) onto our GLSL flavor,
//! and runs the shader single- or multi-pass (with persistent buffers for
//! feedback). The declared `INPUTS` map onto a fixed pool of float parameters,
//! so they are live-controllable and modulatable; per-shader labels/ranges are
//! sent to the web UI separately.
//!
//! Reference: <https://isf.video/>. Many shaders that rely on GLSL 3 features,
//! derivatives or custom buffer sizes will not compile on real GLES2 hardware;
//! the compile error is surfaced rather than failing silently.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::engine::Engine;
use crate::params::{ParamId, ParamKind, ParamSpec};

use super::gl::{self, FullscreenQuad, Gl, GlslFlavor, PingPong, Program, RenderTexture};

/// Number of float parameter slots ISF inputs map onto.
pub const POOL: usize = 32;
/// Upper bound on selectable ISF files (stable param max for live rescans).
const MAX_ISF: i64 = 63;

/// The kinds of ISF input we map to parameters (image/audio bind textures).
#[derive(Clone)]
enum InputKind {
    Float,
    Bool,
    Long { values: Vec<i32> },
    Event,
    Color,
    Point2D,
    Image,
    Audio,
}

/// One declared ISF input plus how it maps onto the parameter pool.
#[derive(Clone)]
pub struct Input {
    pub name: String,
    pub label: String,
    kind: InputKind,
    /// First pool slot (image/audio inputs use `usize::MAX`).
    slot: usize,
    min: f32,
    max: f32,
}

/// One render pass.
struct PassDef {
    target: Option<String>,
    persistent: bool,
}

/// A compiled, ready-to-run ISF shader.
struct Compiled {
    prog: Program,
    inputs: Vec<Input>,
    passes: Vec<PassDef>,
    /// Render targets keyed by name (ping-pong: read previous / write current).
    buffers: HashMap<String, PingPong>,
}

pub struct IsfBank {
    gl: Gl,
    flavor: GlslFlavor,
    /// `isf.shader`: 0 = off, 1.. = a loaded file.
    shader_param: ParamId,
    /// Parameter pool the inputs map onto (`isf.0` .. `isf.{POOL-1}`).
    pool: Vec<ParamId>,
    files: Vec<PathBuf>,
    names: Vec<String>,
    compiled: Option<Compiled>,
    last_src: i64,
    /// 1x1 black texture for unbound image inputs.
    black: glow::Texture,
    width: i32,
    height: i32,
    /// Last compile error (surfaced to the UI).
    error: Option<String>,
    /// Whether the input descriptors changed (so the backend re-publishes them).
    inputs_dirty: bool,
}

impl IsfBank {
    pub fn new(gl: &Gl, flavor: GlslFlavor, engine: &mut Engine, width: i32, height: i32) -> Result<Self, String> {
        let (files, names) = scan_isf_dir();
        let store = engine.params_mut();
        let shader_param = store.register(ParamSpec::new(
            "isf.shader",
            "Shader",
            "ISF",
            ParamKind::Int { min: 0, max: MAX_ISF, default: 0 },
        ));
        let mut pool = Vec::with_capacity(POOL);
        for i in 0..POOL {
            pool.push(store.register(ParamSpec::new(
                format!("isf.{i}"),
                format!("isf {i}"),
                "ISF",
                ParamKind::Float { min: 0.0, max: 1.0, default: 0.5 },
            )));
        }
        let black = gl::make_texture(gl, 1, 1, Some(&[0, 0, 0, 255]), false);

        Ok(Self {
            gl: gl.clone(),
            flavor,
            shader_param,
            pool,
            files,
            names,
            compiled: None,
            last_src: -1,
            black,
            width: width.max(1),
            height: height.max(1),
            error: None,
            inputs_dirty: false,
        })
    }

    /// Dropdown labels for the shader source (index 0 = off).
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Re-scan the ISF directory.
    pub fn rescan(&mut self) {
        let (files, names) = scan_isf_dir();
        self.files = files;
        self.names = names;
        self.last_src = -1;
    }

    /// Whether an ISF shader (not "off") is selected.
    pub fn active(&self, engine: &Engine) -> bool {
        engine.params().get(self.shader_param).as_i64() > 0
    }

    /// True once after a shader (re)loads, so the backend re-publishes the
    /// inputs + any compile error to the UI.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.inputs_dirty)
    }

    /// The last compile error ("" if none).
    pub fn error(&self) -> String {
        self.error.clone().unwrap_or_default()
    }

    /// The current shader's input descriptors (for the UI), if loaded.
    pub fn input_descriptors(&self) -> Vec<(String, String, usize)> {
        // (label, type-tag, first pool slot)
        self.compiled
            .as_ref()
            .map(|c| c.inputs.iter().filter(|i| i.slot != usize::MAX).map(|i| (i.label.clone(), kind_tag(&i.kind).to_string(), i.slot)).collect())
            .unwrap_or_default()
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        // Buffers are reallocated on next load.
        self.last_src = -1;
        if let Some(c) = self.compiled.as_mut() {
            c.buffers.clear();
        }
    }

    /// Render the selected shader into `output` (the layer target). Audio bands
    /// feed any "audio"-typed input slot indirectly via the params.
    pub fn render(&mut self, quad: &FullscreenQuad, output: &RenderTexture, engine: &Engine, time: f32, dt: f32, frame: u64) {
        let src = engine.params().get(self.shader_param).as_i64().max(0);
        if src != self.last_src {
            self.load(src);
            self.last_src = src;
        }
        if self.compiled.is_none() {
            output.bind_as_target();
            gl::clear(&self.gl, 0.0, 0.0, 0.0);
            return;
        }

        let p = engine.params();
        let res = (self.width as f32, self.height as f32);
        // Per-pass target list (clone so we can swap buffers mutably between passes).
        let targets: Vec<Option<String>> =
            self.compiled.as_ref().unwrap().passes.iter().map(|pass| pass.target.clone()).collect();

        for (i, target) in targets.iter().enumerate() {
            {
                let c = self.compiled.as_ref().unwrap();
                match target {
                    Some(name) => c.buffers[name].write_target().bind_as_target(),
                    None => output.bind_as_target(),
                }
                c.prog.bind();
                c.prog.set_vec2("u_renderSize", res.0, res.1);
                c.prog.set_f32("u_time", time);
                c.prog.set_f32("u_timedelta", dt);
                c.prog.set_i32("u_frameindex", frame as i32);
                c.prog.set_i32("u_passindex", i as i32);
                c.prog.set_vec4("u_date", 2026.0, 1.0, 1.0, time);

                // Buffer samplers (readable side), then input uniforms.
                let mut unit = 1u32;
                for (name, pp) in &c.buffers {
                    c.prog.set_texture(name, unit, pp.read());
                    unit += 1;
                }
                for input in &c.inputs {
                    self.set_input(c, input, p, &mut unit);
                }
                quad.draw();
            }
            // Swap this pass's target so later passes (and the next frame, for
            // persistent buffers) read the freshly written content.
            if let Some(name) = target {
                if let Some(pp) = self.compiled.as_mut().unwrap().buffers.get_mut(name) {
                    pp.swap();
                }
            }
        }
    }

    /// Set one input's uniform(s) from the parameter pool (or bind a texture).
    fn set_input(&self, c: &Compiled, input: &Input, p: &crate::params::ParamStore, unit: &mut u32) {
        let norm = |slot: usize| p.get_f32(self.pool[slot]);
        match &input.kind {
            InputKind::Float => {
                let v = input.min + norm(input.slot) * (input.max - input.min);
                c.prog.set_f32(&input.name, v);
            }
            InputKind::Bool | InputKind::Event => {
                c.prog.set_i32(&input.name, if norm(input.slot) >= 0.5 { 1 } else { 0 });
            }
            InputKind::Long { values } => {
                let idx = (norm(input.slot) * (values.len().max(1) - 1) as f32).round() as usize;
                let v = values.get(idx).copied().unwrap_or(0);
                c.prog.set_i32(&input.name, v);
            }
            InputKind::Color => {
                c.prog.set_vec4(&input.name, norm(input.slot), norm(input.slot + 1), norm(input.slot + 2), norm(input.slot + 3));
            }
            InputKind::Point2D => {
                c.prog.set_vec2(&input.name, norm(input.slot), norm(input.slot + 1));
            }
            InputKind::Image | InputKind::Audio => {
                // No external image source for the generator path: bind black.
                c.prog.set_texture(&input.name, *unit, self.black);
                *unit += 1;
            }
        }
    }

    /// Compile the selected file and allocate its pass buffers. Inputs map onto
    /// the parameter pool (which sits at its neutral default until adjusted).
    fn load(&mut self, src: i64) {
        self.compiled = None;
        self.error = None;
        self.inputs_dirty = true;
        let idx = src as usize;
        if idx == 0 || idx > self.files.len() {
            return;
        }
        let path = self.files[idx - 1].clone();
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(format!("read failed: {e}"));
                return;
            }
        };
        let (meta, body) = match parse_header(&raw) {
            Some(v) => v,
            None => {
                self.error = Some("no ISF JSON header found".into());
                return;
            }
        };

        // Assign pool slots to param-backed inputs.
        let mut inputs = meta.inputs;
        let mut slot = 0usize;
        for inp in inputs.iter_mut() {
            let n = match inp.kind {
                InputKind::Color => 4,
                InputKind::Point2D => 2,
                InputKind::Image | InputKind::Audio => 0,
                _ => 1,
            };
            if n == 0 {
                inp.slot = usize::MAX;
            } else if slot + n <= POOL {
                inp.slot = slot;
                slot += n;
            } else {
                inp.slot = usize::MAX; // out of pool space; left at default
            }
        }

        // Build the program (preamble + body).
        let preamble = build_preamble(self.flavor, &inputs, &meta.passes);
        let vert = include_str!("shaders/fullscreen.vert");
        let frag = format!("{preamble}\n{body}");
        let prog = match Program::new(&self.gl, self.flavor, vert, &frag) {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };

        // Allocate a ping-pong buffer per named pass target, cleared to black so
        // persistent feedback starts from a clean slate (not GPU garbage).
        let mut buffers = HashMap::new();
        for pass in &meta.passes {
            if let Some(name) = &pass.target {
                if !buffers.contains_key(name) {
                    if let Ok(mut pp) = PingPong::new(&self.gl, self.width, self.height) {
                        for _ in 0..2 {
                            pp.write_target().bind_as_target();
                            gl::clear(&self.gl, 0.0, 0.0, 0.0);
                            pp.swap();
                        }
                        buffers.insert(name.clone(), pp);
                    }
                }
            }
        }

        self.compiled = Some(Compiled { prog, inputs, passes: meta.passes, buffers });
        tracing::info!("ISF: loaded {}", path.display());
    }
}

/// A short tag describing an input kind for the UI.
fn kind_tag(k: &InputKind) -> &'static str {
    match k {
        InputKind::Float => "float",
        InputKind::Bool => "bool",
        InputKind::Event => "event",
        InputKind::Long { .. } => "long",
        InputKind::Color => "color",
        InputKind::Point2D => "point2D",
        InputKind::Image => "image",
        InputKind::Audio => "audio",
    }
}

/// Scan the ISF directory (`AV_ISF_DIR`, default `isf`) for shader files.
fn scan_isf_dir() -> (Vec<PathBuf>, Vec<String>) {
    let dir = std::env::var("AV_ISF_DIR").unwrap_or_else(|_| "isf".to_string());
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            let ok = p
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| matches!(x.to_ascii_lowercase().as_str(), "fs" | "frag" | "glsl"))
                .unwrap_or(false);
            if ok {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut names = vec!["(off)".to_string()];
    names.extend(files.iter().map(|p| p.file_stem().and_then(|n| n.to_str()).unwrap_or("?").to_string()));
    (files, names)
}

/// Parsed header metadata.
struct Meta {
    inputs: Vec<Input>,
    passes: Vec<PassDef>,
}

/// Find the leading `/* { ... } */` JSON header, parse it, and return the
/// metadata plus the shader body that follows.
fn parse_header(src: &str) -> Option<(Meta, String)> {
    let start = src.find("/*")?;
    let end_rel = src[start + 2..].find("*/")?;
    let end = start + 2 + end_rel;
    let inner = src[start + 2..end].trim();
    let json: Value = serde_json::from_str(inner).ok()?;
    let body = src[end + 2..].to_string();

    let mut inputs = Vec::new();
    if let Some(arr) = json.get("INPUTS").and_then(Value::as_array) {
        for v in arr {
            if let Some(inp) = parse_input(v) {
                inputs.push(inp);
            }
        }
    }
    let mut passes = Vec::new();
    if let Some(arr) = json.get("PASSES").and_then(Value::as_array) {
        for v in arr {
            let target = v.get("TARGET").and_then(Value::as_str).map(str::to_string);
            let persistent = match v.get("PERSISTENT") {
                Some(Value::Bool(b)) => *b,
                Some(Value::String(s)) => s != "false" && s != "0" && !s.is_empty(),
                _ => false,
            };
            passes.push(PassDef { target, persistent });
        }
    }
    if passes.is_empty() {
        passes.push(PassDef { target: None, persistent: false });
    }
    Some((Meta { inputs, passes }, body))
}

fn parse_input(v: &Value) -> Option<Input> {
    let name = v.get("NAME").and_then(Value::as_str)?.to_string();
    let label = v.get("LABEL").and_then(Value::as_str).unwrap_or(&name).to_string();
    let ty = v.get("TYPE").and_then(Value::as_str).unwrap_or("float");
    let min = v.get("MIN").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let max = v.get("MAX").and_then(Value::as_f64).unwrap_or(1.0) as f32;
    let kind = match ty {
        "bool" => InputKind::Bool,
        "event" => InputKind::Event,
        "color" => InputKind::Color,
        "point2D" => InputKind::Point2D,
        "image" => InputKind::Image,
        "audio" | "audioFFT" => InputKind::Audio,
        "long" => {
            let values: Vec<i32> = v
                .get("VALUES")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.as_i64().map(|i| i as i32)).collect())
                .unwrap_or_default();
            InputKind::Long { values: if values.is_empty() { vec![0, 1] } else { values } }
        }
        _ => InputKind::Float,
    };
    Some(Input { name, label, kind, slot: usize::MAX, min, max })
}

/// Build the GLSL preamble that maps ISF onto our flavor + declares uniforms.
fn build_preamble(flavor: GlslFlavor, inputs: &[Input], passes: &[PassDef]) -> String {
    let _ = flavor; // the macros below work on both flavors via TEX2D/FRAG_COLOR
    let mut s = String::new();
    s.push_str("VARYING vec2 v_uv;\n");
    s.push_str("uniform vec2 u_renderSize;\n");
    s.push_str("uniform float u_time;\n");
    s.push_str("uniform float u_timedelta;\n");
    s.push_str("uniform int u_frameindex;\n");
    s.push_str("uniform int u_passindex;\n");
    s.push_str("uniform vec4 u_date;\n");
    s.push_str("#define RENDERSIZE u_renderSize\n");
    s.push_str("#define TIME u_time\n");
    s.push_str("#define TIMEDELTA u_timedelta\n");
    s.push_str("#define FRAMEINDEX u_frameindex\n");
    s.push_str("#define PASSINDEX u_passindex\n");
    s.push_str("#define DATE u_date\n");
    s.push_str("#define isf_FragNormCoord v_uv\n");
    // ISF shaders are written GL2/ES2-style; map onto our flavor macros.
    s.push_str("#define gl_FragColor FRAG_COLOR\n");
    s.push_str("#define texture2D TEX2D\n");
    s.push_str("#define IMG_NORM_PIXEL(img, nc) TEX2D(img, nc)\n");
    s.push_str("#define IMG_PIXEL(img, pc) TEX2D(img, (pc) / RENDERSIZE)\n");
    s.push_str("#define IMG_THIS_NORM_PIXEL(img) TEX2D(img, isf_FragNormCoord)\n");
    s.push_str("#define IMG_THIS_PIXEL(img) TEX2D(img, isf_FragNormCoord)\n");
    s.push_str("#define IMG_SIZE(img) RENDERSIZE\n");

    // Pass-target buffers become readable samplers.
    let mut seen = std::collections::HashSet::new();
    for pass in passes {
        if let Some(name) = &pass.target {
            if seen.insert(name.clone()) {
                s.push_str(&format!("uniform sampler2D {name};\n"));
            }
        }
    }
    // Declared inputs become uniforms of the matching type.
    for inp in inputs {
        let gl_ty = match inp.kind {
            InputKind::Float => "float",
            InputKind::Bool | InputKind::Event => "bool",
            InputKind::Long { .. } => "int",
            InputKind::Color => "vec4",
            InputKind::Point2D => "vec2",
            InputKind::Image | InputKind::Audio => "sampler2D",
        };
        s.push_str(&format!("uniform {gl_ty} {};\n", inp.name));
    }
    s
}
