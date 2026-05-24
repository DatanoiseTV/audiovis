//! The layer compositor.
//!
//! A fixed stack of layers, each running any generator with its own knobs, a
//! blend mode and an opacity. Layers render into their own targets, then blend
//! over a ping-pong accumulator, which is finally presented to the screen with
//! a master brightness. The analog/glitch post chain slots in between the
//! accumulator and the present pass in a later milestone.

use crate::engine::Engine;
use crate::params::{ParamId, ParamKind, ParamSpec};

use super::generators::{CommonUniforms, GeneratorBank};
use super::gl::{self, FullscreenQuad, Gl, GlslFlavor, PingPong, Program, RenderTexture};

/// How many layers the stack exposes. Three is a good live balance: a base, a
/// texture/pattern, and an accent - without overwhelming the control surface.
pub const NUM_LAYERS: usize = 3;

/// Number of blend modes (must match `blend.frag`).
const NUM_BLENDS: i64 = 5;

/// Parameter handles for one layer, resolved once at construction.
struct LayerParams {
    generator: ParamId,
    opacity: ParamId,
    blend: ParamId,
    speed: ParamId,
    scale: ParamId,
    warp: ParamId,
    hue: ParamId,
    p1: ParamId,
    p2: ParamId,
}

pub struct Compositor {
    gl: Gl,
    bank: GeneratorBank,
    layer_targets: Vec<RenderTexture>,
    acc: PingPong,
    blend_prog: Program,
    copy_prog: Program,
    layers: Vec<LayerParams>,
    brightness: Option<ParamId>,
    width: i32,
    height: i32,
    /// Low/mid/high audio energy, updated by the engine each frame.
    audio: (f32, f32, f32),
}

impl Compositor {
    pub fn new(gl: &Gl, flavor: GlslFlavor, engine: &mut Engine, width: i32, height: i32) -> Result<Self, String> {
        let bank = GeneratorBank::new(gl, flavor)?;
        let gen_max = bank.len().saturating_sub(1) as i64;

        // Register the per-layer control surface.
        let mut layers = Vec::with_capacity(NUM_LAYERS);
        {
            let store = engine.params_mut();
            for i in 0..NUM_LAYERS {
                let g = format!("Layer {}", i + 1);
                let pre = format!("layer.{i}");
                let f = |name: &str, lo: f32, hi: f32, def: f32| ParamKind::Float { min: lo, max: hi, default: def };

                let generator = store.register(ParamSpec::new(
                    format!("{pre}.generator"),
                    "Generator",
                    &g,
                    ParamKind::Int { min: 0, max: gen_max, default: (i as i64).min(gen_max) },
                ));
                let opacity = store.register(ParamSpec::new(
                    format!("{pre}.opacity"),
                    "Opacity",
                    &g,
                    f("", 0.0, 1.0, if i == 0 { 1.0 } else { 0.0 }),
                ));
                let blend = store.register(ParamSpec::new(
                    format!("{pre}.blend"),
                    "Blend",
                    &g,
                    ParamKind::Int { min: 0, max: NUM_BLENDS - 1, default: if i == 0 { 0 } else { 1 } },
                ));
                let speed = store.register(ParamSpec::new(format!("{pre}.speed"), "Speed", &g, f("", 0.0, 4.0, 1.0)));
                let scale = store.register(ParamSpec::new(format!("{pre}.scale"), "Scale", &g, f("", 0.1, 8.0, 1.0)));
                let warp = store.register(ParamSpec::new(format!("{pre}.warp"), "Warp", &g, f("", 0.0, 1.0, 0.0)));
                let hue = store.register(ParamSpec::new(format!("{pre}.hue"), "Hue", &g, f("", 0.0, 1.0, (i as f32) * 0.33)));
                let p1 = store.register(ParamSpec::new(format!("{pre}.p1"), "Param 1", &g, f("", 0.0, 1.0, 0.5)));
                let p2 = store.register(ParamSpec::new(format!("{pre}.p2"), "Param 2", &g, f("", 0.0, 1.0, 0.5)));

                layers.push(LayerParams { generator, opacity, blend, speed, scale, warp, hue, p1, p2 });
            }
        }
        let brightness = engine.params().id_of("global.brightness");

        let vert = include_str!("shaders/fullscreen.vert");
        let blend_prog = Program::new(gl, flavor, vert, include_str!("shaders/composite/blend.frag"))?;
        let copy_prog = Program::new(gl, flavor, vert, include_str!("shaders/composite/copy.frag"))?;

        let (w, h) = (width.max(1), height.max(1));
        let mut layer_targets = Vec::with_capacity(NUM_LAYERS);
        for _ in 0..NUM_LAYERS {
            layer_targets.push(RenderTexture::new(gl, w, h)?);
        }
        let acc = PingPong::new(gl, w, h)?;

        Ok(Self {
            gl: gl.clone(),
            bank,
            layer_targets,
            acc,
            blend_prog,
            copy_prog,
            layers,
            brightness,
            width: w,
            height: h,
            audio: (0.0, 0.0, 0.0),
        })
    }

    pub fn generator_count(&self) -> usize {
        self.bank.len()
    }

    /// Resize the internal render targets (the internal size may be smaller
    /// than the output when render-scale < 1).
    pub fn resize(&mut self, width: i32, height: i32) -> Result<(), String> {
        let (w, h) = (width.max(1), height.max(1));
        if w == self.width && h == self.height {
            return Ok(());
        }
        self.layer_targets.clear();
        for _ in 0..NUM_LAYERS {
            self.layer_targets.push(RenderTexture::new(&self.gl, w, h)?);
        }
        self.acc = PingPong::new(&self.gl, w, h)?;
        self.width = w;
        self.height = h;
        Ok(())
    }

    /// Update the audio band energies generators react to.
    pub fn set_audio(&mut self, low: f32, mid: f32, high: f32) {
        self.audio = (low, mid, high);
    }

    /// Render the full stack. `out_w`/`out_h` are the real output viewport;
    /// the internal targets may be smaller. The output framebuffer must be the
    /// default (screen) when this returns.
    pub fn render(&mut self, quad: &FullscreenQuad, engine: &Engine, time: f32, out_w: i32, out_h: i32) {
        let p = engine.params();
        let res = (self.width as f32, self.height as f32);

        // 1. Render each visible layer into its own target.
        for (i, lp) in self.layers.iter().enumerate() {
            let opacity = p.get_f32(lp.opacity);
            if opacity <= 0.001 {
                continue;
            }
            let u = CommonUniforms {
                time,
                res,
                speed: p.get_f32(lp.speed),
                scale: p.get_f32(lp.scale),
                warp: p.get_f32(lp.warp),
                hue: p.get_f32(lp.hue),
                p1: p.get_f32(lp.p1),
                p2: p.get_f32(lp.p2),
                audio: self.audio,
            };
            let gen = p.get(lp.generator).as_i64().max(0) as usize;
            self.layer_targets[i].bind_as_target();
            self.bank.draw(gen, quad, &u);
        }

        // 2. Clear the accumulator to black, then blend visible layers over it.
        self.acc.write_target().bind_as_target();
        gl::clear(&self.gl, 0.0, 0.0, 0.0);
        self.acc.swap();

        for (i, lp) in self.layers.iter().enumerate() {
            let opacity = p.get_f32(lp.opacity);
            if opacity <= 0.001 {
                continue;
            }
            self.acc.write_target().bind_as_target();
            self.blend_prog.bind();
            self.blend_prog.set_texture("u_base", 0, self.acc.read());
            self.blend_prog.set_texture("u_top", 1, self.layer_targets[i].texture());
            self.blend_prog.set_f32("u_opacity", opacity);
            self.blend_prog.set_i32("u_mode", p.get(lp.blend).as_i64() as i32);
            quad.draw();
            self.acc.swap();
        }

        // 3. Present the accumulator to the screen with master brightness.
        let brightness = self.brightness.map(|id| p.get_f32(id)).unwrap_or(1.0);
        gl::bind_screen(&self.gl, out_w, out_h);
        self.copy_prog.bind();
        self.copy_prog.set_texture("u_tex", 0, self.acc.read());
        self.copy_prog.set_f32("u_brightness", brightness);
        quad.draw();
    }
}
