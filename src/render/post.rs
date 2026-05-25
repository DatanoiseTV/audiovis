//! The post-processing chain.
//!
//! Runs after compositing: takes the composited frame texture and applies a
//! reorderable chain of effect instances, ping-ponging between two buffers,
//! then presents the result to the screen with master brightness.
//!
//! Effects are dynamic: there are several **instances** of each effect type
//! (feedback, mirror, hue-cycle, lo-fi, VHS, glitch, bloom), each with its own
//! `enable`, an `order` (the chain runs enabled instances sorted by it) and its
//! own parameters - so you can stack, say, two independent glitches. Instance 0
//! of each type keeps the original `post.<name>.*` paths so existing presets and
//! modulation routes are unchanged; further instances live under
//! `post.<name>.<n>.*`. Programs are compiled once per type and shared.

use crate::engine::Engine;
use crate::params::{ParamId, ParamKind, ParamSpec};

use super::gl::{self, FullscreenQuad, Gl, GlslFlavor, PingPong, Program, RenderTexture};

/// How many instances of each effect type the surface exposes.
pub const FX_INSTANCES: usize = 2;

/// Audio/clock signals shared with post effects, mirroring the generator set.
#[derive(Debug, Clone, Copy, Default)]
struct PostSignals {
    audio: (f32, f32, f32),
    beat: f32,
}

/// The effect types, in their default chain order.
#[derive(Clone, Copy, PartialEq)]
enum FxType {
    Feedback,
    Mirror,
    Hue,
    LoFi,
    Vhs,
    Glitch,
    Bloom,
}

impl FxType {
    const ALL: [FxType; 7] =
        [FxType::Feedback, FxType::Mirror, FxType::Hue, FxType::LoFi, FxType::Vhs, FxType::Glitch, FxType::Bloom];

    /// Short id used in parameter paths.
    fn id(self) -> &'static str {
        match self {
            FxType::Feedback => "feedback",
            FxType::Mirror => "mirror",
            FxType::Hue => "hue",
            FxType::LoFi => "lofi",
            FxType::Vhs => "vhs",
            FxType::Glitch => "glitch",
            FxType::Bloom => "bloom",
        }
    }

    /// Display name for the UI group.
    fn label(self) -> &'static str {
        match self {
            FxType::Feedback => "Feedback",
            FxType::Mirror => "Mirror",
            FxType::Hue => "Hue cycle",
            FxType::LoFi => "Lo-fi",
            FxType::Vhs => "VHS",
            FxType::Glitch => "Glitch",
            FxType::Bloom => "Bloom",
        }
    }
}

/// Parameter handles common to every effect instance.
struct Common {
    enable: ParamId,
    order: ParamId,
}

/// One configured effect instance: its type, common controls, the type-specific
/// parameters and (for feedback) its own history buffer.
struct FxInstance {
    ty: FxType,
    common: Common,
    params: FxParams,
    history: Option<RenderTexture>,
}

/// Type-specific parameter handles.
enum FxParams {
    Feedback { amount: ParamId, zoom: ParamId, rotate: ParamId },
    Mirror { mode: ParamId },
    Hue { shift: ParamId, rate: ParamId },
    LoFi { pixels: ParamId, levels: ParamId },
    Vhs { aberration: ParamId, bleed: ParamId, scan: ParamId, noise: ParamId, wobble: ParamId, vignette: ParamId, sat: ParamId },
    Glitch { intensity: ParamId, blocks: ParamId, shift: ParamId, crush: ParamId, rate: ParamId },
    Bloom { amount: ParamId, threshold: ParamId },
}

/// Shared, compile-once programs (one per effect type, plus feedback's blit).
struct Programs {
    feedback: Program,
    blit: Program,
    mirror: Program,
    hue: Program,
    lofi: Program,
    vhs: Program,
    glitch: Program,
    bloom: Program,
}

pub struct PostChain {
    gl: Gl,
    progs: Programs,
    fx: Vec<FxInstance>,
    pp: PingPong,
    present: Program,
    brightness: Option<ParamId>,
    signals: PostSignals,
    width: i32,
    height: i32,
}

/// Build a parameter path: instance 0 keeps the legacy `post.<id>.<param>`
/// scheme (so presets are unchanged); later instances get a `.<n>` segment.
fn fx_path(id: &str, inst: usize, param: &str) -> String {
    if inst == 0 {
        format!("post.{id}.{param}")
    } else {
        format!("post.{id}.{inst}.{param}")
    }
}

fn fx_group(ty: FxType, inst: usize) -> String {
    if inst == 0 {
        ty.label().to_string()
    } else {
        format!("{} {}", ty.label(), inst + 1)
    }
}

impl PostChain {
    pub fn new(gl: &Gl, flavor: GlslFlavor, engine: &mut Engine, width: i32, height: i32) -> Result<Self, String> {
        let vert = include_str!("shaders/fullscreen.vert");
        let lib = include_str!("shaders/lib.glsl");
        let prog = |frag: &str| {
            let body = format!("{lib}\n{frag}");
            Program::new(gl, flavor, vert, &body)
        };
        let progs = Programs {
            feedback: prog(include_str!("shaders/post/feedback.frag")).map_err(|e| format!("feedback: {e}"))?,
            blit: Program::new(gl, flavor, vert, include_str!("shaders/composite/copy.frag"))?,
            mirror: prog(include_str!("shaders/post/mirror.frag")).map_err(|e| format!("mirror: {e}"))?,
            hue: prog(include_str!("shaders/post/huecycle.frag")).map_err(|e| format!("huecycle: {e}"))?,
            lofi: prog(include_str!("shaders/post/lofi.frag")).map_err(|e| format!("lofi: {e}"))?,
            vhs: prog(include_str!("shaders/post/vhs.frag")).map_err(|e| format!("vhs: {e}"))?,
            glitch: prog(include_str!("shaders/post/glitch.frag")).map_err(|e| format!("glitch: {e}"))?,
            bloom: prog(include_str!("shaders/post/bloom.frag")).map_err(|e| format!("bloom: {e}"))?,
        };
        let present = Program::new(gl, flavor, vert, include_str!("shaders/composite/copy.frag"))?;

        let (w, h) = (width.max(1), height.max(1));
        let mut fx = Vec::new();
        for (ti, ty) in FxType::ALL.iter().enumerate() {
            for inst in 0..FX_INSTANCES {
                fx.push(register_instance(engine, *ty, inst, ti));
            }
        }

        let brightness = engine.params().id_of("global.brightness");
        Ok(Self {
            gl: gl.clone(),
            progs,
            fx,
            pp: PingPong::new(gl, w, h)?,
            present,
            brightness,
            signals: PostSignals::default(),
            width: w,
            height: h,
        })
    }

    pub fn resize(&mut self, width: i32, height: i32) -> Result<(), String> {
        let (w, h) = (width.max(1), height.max(1));
        if w != self.width || h != self.height {
            self.pp = PingPong::new(&self.gl, w, h)?;
            for inst in &mut self.fx {
                if inst.history.is_some() {
                    inst.history = RenderTexture::new(&self.gl, w, h).ok();
                }
            }
            self.width = w;
            self.height = h;
        }
        Ok(())
    }

    /// Update the audio/onset signals post effects react to.
    pub fn set_audio(&mut self, low: f32, mid: f32, high: f32, beat: f32) {
        self.signals = PostSignals { audio: (low, mid, high), beat };
    }

    /// Process `input` through the enabled effects (sorted by their order) and
    /// present to the screen. If `preview` is given, the final frame is also
    /// drawn into it (for the web monitor).
    pub fn process(
        &mut self,
        quad: &FullscreenQuad,
        input: glow::Texture,
        engine: &Engine,
        time: f32,
        out_w: i32,
        out_h: i32,
        preview: Option<&RenderTexture>,
    ) {
        let res = (self.width as f32, self.height as f32);
        let signals = self.signals;
        let p = engine.params();

        // Decide the active chain: enabled instances, sorted by their order.
        let mut chain: Vec<usize> = (0..self.fx.len()).filter(|&i| p.get_bool(self.fx[i].common.enable)).collect();
        chain.sort_by_key(|&i| p.get(self.fx[i].common.order).as_i64());

        let mut current = input;
        for idx in chain {
            self.pp.write_target().bind_as_target();
            self.setup(idx, current, engine, time, res, signals);
            quad.draw();
            self.pp.swap();
            current = self.pp.read();
            // Feedback keeps the just-produced frame as next frame's history.
            if matches!(self.fx[idx].params, FxParams::Feedback { .. }) {
                let (w, h, gl) = (self.width, self.height, self.gl.clone());
                let hist = self.fx[idx].history.get_or_insert_with(|| RenderTexture::new(&gl, w, h).expect("history"));
                hist.bind_as_target();
                self.progs.blit.bind();
                self.progs.blit.set_texture("u_tex", 0, current);
                self.progs.blit.set_f32("u_brightness", 1.0);
                quad.draw();
            }
        }

        // Present to the screen with master brightness.
        let brightness = self.brightness.map(|id| p.get_f32(id)).unwrap_or(1.0);
        gl::bind_screen(&self.gl, out_w, out_h);
        self.present.bind();
        self.present.set_texture("u_tex", 0, current);
        self.present.set_f32("u_brightness", brightness);
        quad.draw();

        if let Some(pv) = preview {
            pv.bind_as_target();
            self.present.bind();
            self.present.set_texture("u_tex", 0, current);
            self.present.set_f32("u_brightness", brightness);
            quad.draw();
            // Re-bind the window framebuffer so later passes target the screen.
            gl::bind_screen(&self.gl, out_w, out_h);
        }
    }

    /// Bind the right program for instance `idx` and upload its uniforms.
    fn setup(&mut self, idx: usize, src: glow::Texture, engine: &Engine, time: f32, res: (f32, f32), sig: PostSignals) {
        let p = engine.params();
        let inst = &self.fx[idx];
        match &inst.params {
            FxParams::Feedback { amount, zoom, rotate } => {
                let prog = &self.progs.feedback;
                prog.bind();
                prog.set_texture("u_tex", 0, src);
                let hist = inst.history.as_ref().map(|h| h.texture()).unwrap_or(src);
                prog.set_texture("u_history", 1, hist);
                prog.set_f32("u_amount", p.get_f32(*amount));
                prog.set_f32("u_zoom", p.get_f32(*zoom));
                prog.set_f32("u_rotate", p.get_f32(*rotate));
            }
            FxParams::Mirror { mode } => {
                let prog = &self.progs.mirror;
                prog.bind();
                prog.set_texture("u_tex", 0, src);
                prog.set_i32("u_mode", p.get(*mode).as_i64() as i32);
            }
            FxParams::Hue { shift, rate } => {
                let prog = &self.progs.hue;
                prog.bind();
                prog.set_texture("u_tex", 0, src);
                prog.set_f32("u_time", time);
                prog.set_f32("u_shift", p.get_f32(*shift));
                prog.set_f32("u_rate", p.get_f32(*rate));
            }
            FxParams::LoFi { pixels, levels } => {
                let prog = &self.progs.lofi;
                prog.bind();
                prog.set_texture("u_tex", 0, src);
                prog.set_f32("u_pixels", p.get_f32(*pixels));
                prog.set_f32("u_levels", p.get_f32(*levels));
            }
            FxParams::Vhs { aberration, bleed, scan, noise, wobble, vignette, sat } => {
                let prog = &self.progs.vhs;
                prog.bind();
                prog.set_texture("u_tex", 0, src);
                prog.set_f32("u_time", time);
                prog.set_vec2("u_res", res.0, res.1);
                prog.set_f32("u_aberration", p.get_f32(*aberration));
                prog.set_f32("u_bleed", p.get_f32(*bleed));
                prog.set_f32("u_scan", p.get_f32(*scan));
                prog.set_f32("u_noise", p.get_f32(*noise));
                prog.set_f32("u_wobble", p.get_f32(*wobble));
                prog.set_f32("u_vignette", p.get_f32(*vignette));
                prog.set_f32("u_sat", p.get_f32(*sat));
            }
            FxParams::Glitch { intensity, blocks, shift, crush, rate } => {
                let prog = &self.progs.glitch;
                prog.bind();
                prog.set_texture("u_tex", 0, src);
                prog.set_f32("u_time", time);
                prog.set_vec2("u_res", res.0, res.1);
                prog.set_f32("u_beat", sig.beat);
                prog.set_vec3("u_audio", sig.audio.0, sig.audio.1, sig.audio.2);
                prog.set_f32("u_intensity", p.get_f32(*intensity));
                prog.set_f32("u_blocks", p.get_f32(*blocks));
                prog.set_f32("u_shift", p.get_f32(*shift));
                prog.set_f32("u_crush", p.get_f32(*crush));
                prog.set_f32("u_rate", p.get_f32(*rate));
            }
            FxParams::Bloom { amount, threshold } => {
                let prog = &self.progs.bloom;
                prog.bind();
                prog.set_texture("u_tex", 0, src);
                prog.set_vec2("u_res", res.0, res.1);
                prog.set_f32("u_amount", p.get_f32(*amount));
                prog.set_f32("u_threshold", p.get_f32(*threshold));
            }
        }
    }
}

/// Register the parameters for one effect instance and return its handle.
/// `default_order` seeds the chain order so the initial chain matches the old
/// fixed order; instance 0 of VHS is enabled by default (the mild analog wash).
fn register_instance(engine: &mut Engine, ty: FxType, inst: usize, type_index: usize) -> FxInstance {
    let store = engine.params_mut();
    let g = fx_group(ty, inst);
    let id = ty.id();
    let f = |lo: f32, hi: f32, def: f32| ParamKind::Float { min: lo, max: hi, default: def };
    let reg = |store: &mut crate::params::ParamStore, param: &str, name: &str, kind| {
        store.register(ParamSpec::new(fx_path(id, inst, param), name, &g, kind))
    };

    let enable_default = inst == 0 && ty == FxType::Vhs;
    let enable = reg(store, "enable", "Enable", ParamKind::Bool { default: enable_default });
    // Order: spread types out, leaving room for re-sequencing between them.
    let order_default = (type_index * FX_INSTANCES + inst) as i64;
    let order = reg(store, "order", "Order", ParamKind::Int { min: 0, max: 31, default: order_default });
    let common = Common { enable, order };

    let params = match ty {
        FxType::Feedback => FxParams::Feedback {
            amount: reg(store, "amount", "Trail", f(0.0, 0.99, 0.92)),
            zoom: reg(store, "zoom", "Zoom", f(-1.0, 1.0, 0.25)),
            rotate: reg(store, "rotate", "Rotate", f(-1.0, 1.0, 0.0)),
        },
        FxType::Mirror => FxParams::Mirror {
            mode: reg(store, "mode", "Mode", ParamKind::Int { min: 0, max: 3, default: 3 }),
        },
        FxType::Hue => FxParams::Hue {
            shift: reg(store, "shift", "Shift", f(0.0, 1.0, 0.0)),
            rate: reg(store, "rate", "Cycle", f(0.0, 1.0, 0.0)),
        },
        FxType::LoFi => FxParams::LoFi {
            pixels: reg(store, "pixels", "Pixelate", f(0.0, 1.0, 0.5)),
            levels: reg(store, "levels", "Posterize", f(0.0, 1.0, 0.4)),
        },
        FxType::Vhs => FxParams::Vhs {
            aberration: reg(store, "aberration", "Aberration", f(0.0, 1.0, 0.35)),
            bleed: reg(store, "bleed", "Chroma bleed", f(0.0, 1.0, 0.3)),
            scan: reg(store, "scanline", "Scanlines", f(0.0, 1.0, 0.3)),
            noise: reg(store, "noise", "Tape noise", f(0.0, 1.0, 0.15)),
            wobble: reg(store, "wobble", "Tracking", f(0.0, 1.0, 0.2)),
            vignette: reg(store, "vignette", "Vignette", f(0.0, 1.0, 0.4)),
            sat: reg(store, "saturation", "Saturation", f(-1.0, 1.0, 0.1)),
        },
        FxType::Glitch => FxParams::Glitch {
            intensity: reg(store, "intensity", "Intensity", f(0.0, 1.0, 0.5)),
            blocks: reg(store, "blocks", "Blocks", f(0.0, 1.0, 0.4)),
            shift: reg(store, "shift", "RGB tear", f(0.0, 1.0, 0.5)),
            crush: reg(store, "crush", "Bitcrush", f(0.0, 1.0, 0.3)),
            rate: reg(store, "rate", "Rate", f(0.0, 1.0, 0.3)),
        },
        FxType::Bloom => FxParams::Bloom {
            amount: reg(store, "amount", "Amount", f(0.0, 1.0, 0.5)),
            threshold: reg(store, "threshold", "Threshold", f(0.0, 1.0, 0.6)),
        },
    };

    FxInstance { ty, common, params, history: None }
}
