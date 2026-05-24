//! The post-processing chain.
//!
//! Runs after compositing: takes the composited frame texture and applies a
//! sequence of full-screen effect passes, ping-ponging between two buffers,
//! then presents the result to the screen with master brightness. Each effect
//! owns its program and parameters and is skipped entirely when disabled, so on
//! a weak GPU you pay only for the effects you turn on.
//!
//! This milestone ships the analog/VHS pass; the glitch/datamosh bank adds more
//! passes to the same chain later.

use crate::engine::Engine;
use crate::params::{ParamId, ParamKind, ParamSpec};

use super::gl::{self, FullscreenQuad, Gl, GlslFlavor, PingPong, Program};

/// Audio/clock signals shared with post effects, mirroring the generator set.
#[derive(Debug, Clone, Copy, Default)]
struct PostSignals {
    audio: (f32, f32, f32),
    beat: f32,
}

/// One ordered effect in the chain.
trait PostEffect {
    /// Whether the effect should run this frame.
    fn enabled(&self, engine: &Engine) -> bool;
    /// Bind the program and upload uniforms. The source texture is the previous
    /// stage's output; the target framebuffer is already bound by the chain.
    fn setup(&self, src: glow::Texture, engine: &Engine, time: f32, res: (f32, f32), sig: PostSignals);
}

pub struct PostChain {
    gl: Gl,
    effects: Vec<Box<dyn PostEffect>>,
    pp: PingPong,
    present: Program,
    brightness: Option<ParamId>,
    signals: PostSignals,
    width: i32,
    height: i32,
}

impl PostChain {
    pub fn new(gl: &Gl, flavor: GlslFlavor, engine: &mut Engine, width: i32, height: i32) -> Result<Self, String> {
        let vert = include_str!("shaders/fullscreen.vert");
        let present = Program::new(gl, flavor, vert, include_str!("shaders/composite/copy.frag"))?;

        // Order: analog wash first, then digital corruption on top.
        let effects: Vec<Box<dyn PostEffect>> =
            vec![Box::new(Vhs::new(gl, flavor, engine)?), Box::new(Glitch::new(gl, flavor, engine)?)];
        let brightness = engine.params().id_of("global.brightness");

        let (w, h) = (width.max(1), height.max(1));
        Ok(Self {
            gl: gl.clone(),
            effects,
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
            self.width = w;
            self.height = h;
        }
        Ok(())
    }

    /// Update the audio/onset signals post effects react to.
    pub fn set_audio(&mut self, low: f32, mid: f32, high: f32, beat: f32) {
        self.signals = PostSignals { audio: (low, mid, high), beat };
    }

    /// Process `input` through the enabled effects and present to the screen.
    pub fn process(&mut self, quad: &FullscreenQuad, input: glow::Texture, engine: &Engine, time: f32, out_w: i32, out_h: i32) {
        let res = (self.width as f32, self.height as f32);
        let mut current = input;

        for effect in &self.effects {
            if !effect.enabled(engine) {
                continue;
            }
            self.pp.write_target().bind_as_target();
            effect.setup(current, engine, time, res, self.signals);
            quad.draw();
            self.pp.swap();
            current = self.pp.read();
        }

        // Present to the screen with master brightness.
        let brightness = self.brightness.map(|id| engine.params().get_f32(id)).unwrap_or(1.0);
        gl::bind_screen(&self.gl, out_w, out_h);
        self.present.bind();
        self.present.set_texture("u_tex", 0, current);
        self.present.set_f32("u_brightness", brightness);
        quad.draw();
    }
}

/// The analog / VHS look. One pass, many knobs (see `shaders/post/vhs.frag`).
struct Vhs {
    prog: Program,
    enable: ParamId,
    aberration: ParamId,
    bleed: ParamId,
    scan: ParamId,
    noise: ParamId,
    wobble: ParamId,
    vignette: ParamId,
    sat: ParamId,
}

impl Vhs {
    fn new(gl: &Gl, flavor: GlslFlavor, engine: &mut Engine) -> Result<Self, String> {
        let lib = include_str!("shaders/lib.glsl");
        let vert = include_str!("shaders/fullscreen.vert");
        let body = format!("{lib}\n{}", include_str!("shaders/post/vhs.frag"));
        let prog = Program::new(gl, flavor, vert, &body).map_err(|e| format!("vhs: {e}"))?;

        let store = engine.params_mut();
        let g = "VHS";
        let f = |lo: f32, hi: f32, def: f32| ParamKind::Float { min: lo, max: hi, default: def };
        // Mild analog wash by default - present but not destroyed.
        let enable = store.register(ParamSpec::new("post.vhs.enable", "Enable", g, ParamKind::Bool { default: true }));
        let aberration = store.register(ParamSpec::new("post.vhs.aberration", "Aberration", g, f(0.0, 1.0, 0.35)));
        let bleed = store.register(ParamSpec::new("post.vhs.bleed", "Chroma bleed", g, f(0.0, 1.0, 0.3)));
        let scan = store.register(ParamSpec::new("post.vhs.scanline", "Scanlines", g, f(0.0, 1.0, 0.3)));
        let noise = store.register(ParamSpec::new("post.vhs.noise", "Tape noise", g, f(0.0, 1.0, 0.15)));
        let wobble = store.register(ParamSpec::new("post.vhs.wobble", "Tracking", g, f(0.0, 1.0, 0.2)));
        let vignette = store.register(ParamSpec::new("post.vhs.vignette", "Vignette", g, f(0.0, 1.0, 0.4)));
        let sat = store.register(ParamSpec::new("post.vhs.saturation", "Saturation", g, f(-1.0, 1.0, 0.1)));

        Ok(Self { prog, enable, aberration, bleed, scan, noise, wobble, vignette, sat })
    }
}

impl PostEffect for Vhs {
    fn enabled(&self, engine: &Engine) -> bool {
        engine.params().get_bool(self.enable)
    }

    fn setup(&self, src: glow::Texture, engine: &Engine, time: f32, res: (f32, f32), _sig: PostSignals) {
        let p = engine.params();
        self.prog.bind();
        self.prog.set_texture("u_tex", 0, src);
        self.prog.set_f32("u_time", time);
        self.prog.set_vec2("u_res", res.0, res.1);
        self.prog.set_f32("u_aberration", p.get_f32(self.aberration));
        self.prog.set_f32("u_bleed", p.get_f32(self.bleed));
        self.prog.set_f32("u_scan", p.get_f32(self.scan));
        self.prog.set_f32("u_noise", p.get_f32(self.noise));
        self.prog.set_f32("u_wobble", p.get_f32(self.wobble));
        self.prog.set_f32("u_vignette", p.get_f32(self.vignette));
        self.prog.set_f32("u_sat", p.get_f32(self.sat));
    }
}

/// Digital glitch / datamosh corruption, beat-gated (see `shaders/post/glitch.frag`).
struct Glitch {
    prog: Program,
    enable: ParamId,
    intensity: ParamId,
    blocks: ParamId,
    shift: ParamId,
    crush: ParamId,
    rate: ParamId,
}

impl Glitch {
    fn new(gl: &Gl, flavor: GlslFlavor, engine: &mut Engine) -> Result<Self, String> {
        let lib = include_str!("shaders/lib.glsl");
        let vert = include_str!("shaders/fullscreen.vert");
        let body = format!("{lib}\n{}", include_str!("shaders/post/glitch.frag"));
        let prog = Program::new(gl, flavor, vert, &body).map_err(|e| format!("glitch: {e}"))?;

        let store = engine.params_mut();
        let g = "Glitch";
        let f = |lo: f32, hi: f32, def: f32| ParamKind::Float { min: lo, max: hi, default: def };
        // Off by default; it is an effect to throw in, not a constant wash.
        let enable = store.register(ParamSpec::new("post.glitch.enable", "Enable", g, ParamKind::Bool { default: false }));
        let intensity = store.register(ParamSpec::new("post.glitch.intensity", "Intensity", g, f(0.0, 1.0, 0.5)));
        let blocks = store.register(ParamSpec::new("post.glitch.blocks", "Blocks", g, f(0.0, 1.0, 0.4)));
        let shift = store.register(ParamSpec::new("post.glitch.shift", "RGB tear", g, f(0.0, 1.0, 0.5)));
        let crush = store.register(ParamSpec::new("post.glitch.crush", "Bitcrush", g, f(0.0, 1.0, 0.3)));
        let rate = store.register(ParamSpec::new("post.glitch.rate", "Rate", g, f(0.0, 1.0, 0.3)));

        Ok(Self { prog, enable, intensity, blocks, shift, crush, rate })
    }
}

impl PostEffect for Glitch {
    fn enabled(&self, engine: &Engine) -> bool {
        engine.params().get_bool(self.enable)
    }

    fn setup(&self, src: glow::Texture, engine: &Engine, time: f32, res: (f32, f32), sig: PostSignals) {
        let p = engine.params();
        self.prog.bind();
        self.prog.set_texture("u_tex", 0, src);
        self.prog.set_f32("u_time", time);
        self.prog.set_vec2("u_res", res.0, res.1);
        self.prog.set_f32("u_beat", sig.beat);
        self.prog.set_vec3("u_audio", sig.audio.0, sig.audio.1, sig.audio.2);
        self.prog.set_f32("u_intensity", p.get_f32(self.intensity));
        self.prog.set_f32("u_blocks", p.get_f32(self.blocks));
        self.prog.set_f32("u_shift", p.get_f32(self.shift));
        self.prog.set_f32("u_crush", p.get_f32(self.crush));
        self.prog.set_f32("u_rate", p.get_f32(self.rate));
    }
}
