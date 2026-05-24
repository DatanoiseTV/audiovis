//! The post-processing chain.
//!
//! Runs after compositing: takes the composited frame texture and applies a
//! sequence of full-screen effect passes, ping-ponging between two buffers,
//! then presents the result to the screen with master brightness. Each effect
//! owns its program and parameters and is skipped entirely when disabled, so on
//! a weak GPU you pay only for the effects you turn on.
//!
//! Passes, in order: video feedback (trails), mirror/kaleidoscope, analog/VHS,
//! and digital glitch. Feedback is stateful (keeps a history texture); the rest
//! are pure functions of their input.

use crate::engine::Engine;
use crate::params::{ParamId, ParamKind, ParamSpec};

use super::gl::{self, FullscreenQuad, Gl, GlslFlavor, PingPong, Program, RenderTexture};

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
    /// Called after this effect's draw with its output texture. Stateful
    /// effects (feedback) use it to update their history buffer.
    fn after_draw(&mut self, _quad: &FullscreenQuad, _output: glow::Texture) {}
    /// Resize any internal buffers when the render target changes.
    fn resize(&mut self, _gl: &Gl, _width: i32, _height: i32) {}
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

        // Order: feedback trails, then mirror, then analog wash, then glitch.
        let effects: Vec<Box<dyn PostEffect>> = vec![
            Box::new(Feedback::new(gl, flavor, engine, width.max(1), height.max(1))?),
            Box::new(Mirror::new(gl, flavor, engine)?),
            Box::new(Vhs::new(gl, flavor, engine)?),
            Box::new(Glitch::new(gl, flavor, engine)?),
        ];
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
            for effect in &mut self.effects {
                effect.resize(&self.gl, w, h);
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

    /// Process `input` through the enabled effects and present to the screen.
    pub fn process(&mut self, quad: &FullscreenQuad, input: glow::Texture, engine: &Engine, time: f32, out_w: i32, out_h: i32) {
        let res = (self.width as f32, self.height as f32);
        let signals = self.signals;
        let mut current = input;

        // Disjoint borrows: iterate the effects while the ping-pong is mutated.
        let pp = &mut self.pp;
        for effect in self.effects.iter_mut() {
            if !effect.enabled(engine) {
                continue;
            }
            pp.write_target().bind_as_target();
            effect.setup(current, engine, time, res, signals);
            quad.draw();
            pp.swap();
            current = pp.read();
            effect.after_draw(quad, current);
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

/// Video feedback / infinite-zoom trails. Stateful: keeps the previous output
/// in a history texture and blends it back (see `shaders/post/feedback.frag`).
struct Feedback {
    blend: Program,
    /// Plain copy used to write the current output into the history buffer.
    blit: Program,
    history: RenderTexture,
    enable: ParamId,
    amount: ParamId,
    zoom: ParamId,
    rotate: ParamId,
}

impl Feedback {
    fn new(gl: &Gl, flavor: GlslFlavor, engine: &mut Engine, w: i32, h: i32) -> Result<Self, String> {
        let lib = include_str!("shaders/lib.glsl");
        let vert = include_str!("shaders/fullscreen.vert");
        let body = format!("{lib}\n{}", include_str!("shaders/post/feedback.frag"));
        let blend = Program::new(gl, flavor, vert, &body).map_err(|e| format!("feedback: {e}"))?;
        let blit = Program::new(gl, flavor, vert, include_str!("shaders/composite/copy.frag"))?;
        let history = RenderTexture::new(gl, w, h)?;

        let store = engine.params_mut();
        let g = "Feedback";
        let f = |lo: f32, hi: f32, def: f32| ParamKind::Float { min: lo, max: hi, default: def };
        let enable = store.register(ParamSpec::new("post.feedback.enable", "Enable", g, ParamKind::Bool { default: false }));
        let amount = store.register(ParamSpec::new("post.feedback.amount", "Trail", g, f(0.0, 0.99, 0.92)));
        let zoom = store.register(ParamSpec::new("post.feedback.zoom", "Zoom", g, f(-1.0, 1.0, 0.25)));
        let rotate = store.register(ParamSpec::new("post.feedback.rotate", "Rotate", g, f(-1.0, 1.0, 0.0)));
        Ok(Self { blend, blit, history, enable, amount, zoom, rotate })
    }
}

impl PostEffect for Feedback {
    fn enabled(&self, engine: &Engine) -> bool {
        engine.params().get_bool(self.enable)
    }

    fn setup(&self, src: glow::Texture, engine: &Engine, _time: f32, _res: (f32, f32), _sig: PostSignals) {
        let p = engine.params();
        self.blend.bind();
        self.blend.set_texture("u_tex", 0, src);
        self.blend.set_texture("u_history", 1, self.history.texture());
        self.blend.set_f32("u_amount", p.get_f32(self.amount));
        self.blend.set_f32("u_zoom", p.get_f32(self.zoom));
        self.blend.set_f32("u_rotate", p.get_f32(self.rotate));
    }

    fn after_draw(&mut self, quad: &FullscreenQuad, output: glow::Texture) {
        // Copy this frame's output into the history for the next frame.
        self.history.bind_as_target();
        self.blit.bind();
        self.blit.set_texture("u_tex", 0, output);
        self.blit.set_f32("u_brightness", 1.0);
        quad.draw();
    }

    fn resize(&mut self, gl: &Gl, width: i32, height: i32) {
        if let Ok(rt) = RenderTexture::new(gl, width, height) {
            self.history = rt;
        }
    }
}

/// Mirror / kaleidoscope of the whole frame (see `shaders/post/mirror.frag`).
struct Mirror {
    prog: Program,
    enable: ParamId,
    mode: ParamId,
}

impl Mirror {
    fn new(gl: &Gl, flavor: GlslFlavor, engine: &mut Engine) -> Result<Self, String> {
        let lib = include_str!("shaders/lib.glsl");
        let vert = include_str!("shaders/fullscreen.vert");
        let body = format!("{lib}\n{}", include_str!("shaders/post/mirror.frag"));
        let prog = Program::new(gl, flavor, vert, &body).map_err(|e| format!("mirror: {e}"))?;
        let store = engine.params_mut();
        let g = "Mirror";
        let enable = store.register(ParamSpec::new("post.mirror.enable", "Enable", g, ParamKind::Bool { default: false }));
        // 0 mirror X, 1 mirror Y, 2 quad, 3 kaleidoscope
        let mode = store.register(ParamSpec::new("post.mirror.mode", "Mode", g, ParamKind::Int { min: 0, max: 3, default: 3 }));
        Ok(Self { prog, enable, mode })
    }
}

impl PostEffect for Mirror {
    fn enabled(&self, engine: &Engine) -> bool {
        engine.params().get_bool(self.enable)
    }

    fn setup(&self, src: glow::Texture, engine: &Engine, _time: f32, _res: (f32, f32), _sig: PostSignals) {
        self.prog.bind();
        self.prog.set_texture("u_tex", 0, src);
        self.prog.set_i32("u_mode", engine.params().get(self.mode).as_i64() as i32);
    }
}
