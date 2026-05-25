//! Media input layers.
//!
//! Two extra layers that load a still image (PNG/JPEG) or an SVG from a media
//! directory and composite it over the generator stack. Each layer has the same
//! transform vocabulary as a generator (zoom/rotate/pan) plus a hue rotate and
//! brightness, and blends with the usual mode set. SVGs are rasterised once on
//! load; rasters are decoded once - there is no per-frame decode cost.

use std::path::PathBuf;

use crate::engine::Engine;
use crate::params::{ParamId, ParamKind, ParamSpec};

use super::gl::{self, FullscreenQuad, Gl, GlslFlavor, PingPong, Program};

/// How many media layers the surface exposes.
pub const NUM_MEDIA: usize = 2;

/// Blend mode count, kept in sync with `blend.frag` / `media.frag`.
const NUM_BLENDS: i64 = 5;

/// Longest side an SVG is rasterised to; keeps memory sane on weak ARM.
const SVG_MAX_SIDE: f32 = 1024.0;

/// Control handles for one media layer, resolved once at construction.
struct MediaLayer {
    source: ParamId,
    opacity: ParamId,
    blend: ParamId,
    zoom: ParamId,
    rot: ParamId,
    posx: ParamId,
    posy: ParamId,
    hue: ParamId,
    bright: ParamId,
}

/// A decoded image uploaded to the GPU, with its native aspect ratio.
struct Loaded {
    tex: glow::Texture,
    aspect: f32,
}

pub struct MediaBank {
    gl: Gl,
    prog: Program,
    layers: Vec<MediaLayer>,
    /// Files discovered in the media directory (parallel to `names[1..]`).
    files: Vec<PathBuf>,
    /// Dropdown labels for the source param: index 0 is "(none)".
    names: Vec<String>,
    /// Currently loaded texture per layer, if any.
    loaded: Vec<Option<Loaded>>,
    /// Last source index per layer, to detect a change and (re)load.
    last_src: Vec<i64>,
}

impl MediaBank {
    pub fn new(gl: &Gl, flavor: GlslFlavor, engine: &mut Engine) -> Result<Self, String> {
        let (files, names) = scan_media_dir();

        // Register the per-layer control surface.
        let mut layers = Vec::with_capacity(NUM_MEDIA);
        {
            let store = engine.params_mut();
            let src_max = (names.len() as i64 - 1).max(0);
            for i in 0..NUM_MEDIA {
                let g = format!("Media {}", i + 1);
                let pre = format!("media.{i}");
                let f = |lo: f32, hi: f32, def: f32| ParamKind::Float { min: lo, max: hi, default: def };

                let source = store.register(ParamSpec::new(
                    format!("{pre}.source"),
                    "Source",
                    &g,
                    ParamKind::Int { min: 0, max: src_max, default: 0 },
                ));
                let opacity = store.register(ParamSpec::new(format!("{pre}.opacity"), "Opacity", &g, f(0.0, 1.0, 0.0)));
                let blend = store.register(ParamSpec::new(
                    format!("{pre}.blend"),
                    "Blend",
                    &g,
                    ParamKind::Int { min: 0, max: NUM_BLENDS - 1, default: 0 },
                ));
                let zoom = store.register(ParamSpec::new(format!("{pre}.zoom"), "Zoom", &g, f(0.1, 4.0, 1.0)));
                let rot = store.register(ParamSpec::new(format!("{pre}.rotate"), "Rotate", &g, f(-1.0, 1.0, 0.0)));
                let posx = store.register(ParamSpec::new(format!("{pre}.posx"), "Pan X", &g, f(-1.0, 1.0, 0.0)));
                let posy = store.register(ParamSpec::new(format!("{pre}.posy"), "Pan Y", &g, f(-1.0, 1.0, 0.0)));
                let hue = store.register(ParamSpec::new(format!("{pre}.hue"), "Hue", &g, f(-1.0, 1.0, 0.0)));
                let bright = store.register(ParamSpec::new(format!("{pre}.bright"), "Bright", &g, f(0.0, 2.0, 1.0)));

                layers.push(MediaLayer { source, opacity, blend, zoom, rot, posx, posy, hue, bright });
            }
        }

        let vert = include_str!("shaders/fullscreen.vert");
        let prog = Program::new(gl, flavor, vert, include_str!("shaders/composite/media.frag"))?;

        Ok(Self {
            gl: gl.clone(),
            prog,
            layers,
            files,
            names,
            loaded: (0..NUM_MEDIA).map(|_| None).collect(),
            last_src: vec![-1; NUM_MEDIA],
        })
    }

    /// Dropdown labels for the media source param (index 0 = none).
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Blend each enabled media layer over the accumulator, loading any newly
    /// selected file first. `acc` holds the generator stack on entry.
    pub fn render(&mut self, quad: &FullscreenQuad, engine: &Engine, acc: &mut PingPong, width: i32, height: i32) {
        let p = engine.params();
        for i in 0..self.layers.len() {
            let src = p.get(self.layers[i].source).as_i64().max(0);
            if src != self.last_src[i] {
                self.load_layer(i, src);
                self.last_src[i] = src;
            }
            let opacity = p.get_f32(self.layers[i].opacity);
            if opacity <= 0.001 {
                continue;
            }
            let Some(media) = self.loaded[i].as_ref() else { continue };
            let lp = &self.layers[i];

            acc.write_target().bind_as_target();
            self.prog.bind();
            self.prog.set_texture("u_base", 0, acc.read());
            self.prog.set_texture("u_tex", 1, media.tex);
            self.prog.set_vec2("u_res", width as f32, height as f32);
            self.prog.set_f32("u_aspect", media.aspect);
            self.prog.set_f32("u_zoom", p.get_f32(lp.zoom));
            self.prog.set_f32("u_rot", p.get_f32(lp.rot) * std::f32::consts::PI);
            self.prog.set_vec2("u_pan", p.get_f32(lp.posx), p.get_f32(lp.posy));
            self.prog.set_f32("u_hue", p.get_f32(lp.hue) * std::f32::consts::TAU);
            self.prog.set_f32("u_bright", p.get_f32(lp.bright));
            self.prog.set_f32("u_opacity", opacity);
            self.prog.set_i32("u_mode", p.get(lp.blend).as_i64() as i32);
            quad.draw();
            acc.swap();
        }
    }

    /// Load (or clear) the texture for a layer when its source index changes.
    fn load_layer(&mut self, layer: usize, src: i64) {
        // index 0 = none; files are 1-based against `names`.
        let idx = src as usize;
        if idx == 0 || idx > self.files.len() {
            self.loaded[layer] = None;
            return;
        }
        let path = &self.files[idx - 1];
        match decode(path) {
            Some((rgba, w, h)) => {
                let tex = gl::make_texture(&self.gl, w as i32, h as i32, Some(&rgba), false);
                let aspect = w as f32 / h.max(1) as f32;
                self.loaded[layer] = Some(Loaded { tex, aspect });
                tracing::info!("media {layer}: loaded {} ({w}x{h})", path.display());
            }
            None => {
                tracing::warn!("media {layer}: could not load {}", path.display());
                self.loaded[layer] = None;
            }
        }
    }
}

/// Find the media directory (`AV_MEDIA_DIR`, default `media`) and list the
/// supported files in it, sorted by name. Returns (paths, dropdown labels).
fn scan_media_dir() -> (Vec<PathBuf>, Vec<String>) {
    let dir = std::env::var("AV_MEDIA_DIR").unwrap_or_else(|_| "media".to_string());
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ok = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "svg"))
                .unwrap_or(false);
            if ok {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut names = vec!["(none)".to_string()];
    names.extend(files.iter().map(|p| p.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string()));
    (files, names)
}

/// Decode an image or SVG file to straight-alpha RGBA8 bytes.
fn decode(path: &std::path::Path) -> Option<(Vec<u8>, u32, u32)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext == "svg" {
        decode_svg(path)
    } else {
        let img = image::open(path).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        Some((img.into_raw(), w, h))
    }
}

/// Rasterise an SVG at a resolution that keeps its longest side near
/// `SVG_MAX_SIDE`, then un-premultiply tiny-skia's output to straight alpha.
fn decode_svg(path: &std::path::Path) -> Option<(Vec<u8>, u32, u32)> {
    use resvg::tiny_skia;
    use resvg::usvg;

    let data = std::fs::read(path).ok()?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let longest = size.width().max(size.height()).max(1.0);
    let scale = (SVG_MAX_SIDE / longest).clamp(0.1, 8.0);
    let w = (size.width() * scale).ceil().max(1.0) as u32;
    let h = (size.height() * scale).ceil().max(1.0) as u32;

    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());

    let mut buf = pixmap.take();
    for px in buf.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            px[0] = ((px[0] as u32 * 255) / a).min(255) as u8;
            px[1] = ((px[1] as u32 * 255) / a).min(255) as u8;
            px[2] = ((px[2] as u32 * 255) / a).min(255) as u8;
        }
    }
    Some((buf, w, h))
}
