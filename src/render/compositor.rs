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
use super::media::MediaBank;
use super::sim::SimBank;

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
    zoom: ParamId,
    rot: ParamId,
    posx: ParamId,
    posy: ParamId,
}

pub struct Compositor {
    gl: Gl,
    bank: GeneratorBank,
    sim_bank: SimBank,
    /// Image/SVG input layers, blended over the generator stack.
    media: MediaBank,
    layer_targets: Vec<RenderTexture>,
    /// Per-layer simulation state (only used while a layer runs a stateful gen).
    state: Vec<PingPong>,
    /// Last generator index seen per layer, to detect a switch and reseed.
    last_gen: Vec<i64>,
    acc: PingPong,
    blend_prog: Program,
    layers: Vec<LayerParams>,
    width: i32,
    height: i32,
    /// Low/mid/high audio energy + onset pulse, updated each frame.
    audio: (f32, f32, f32),
    beat: f32,
    /// Waveform texture for the scope generator (owned by the pipeline).
    wave_tex: Option<glow::Texture>,
}

impl Compositor {
    pub fn new(gl: &Gl, flavor: GlslFlavor, engine: &mut Engine, width: i32, height: i32) -> Result<Self, String> {
        let bank = GeneratorBank::new(gl, flavor)?;
        let sim_bank = SimBank::new(gl, flavor)?;
        let media = MediaBank::new(gl, flavor, engine)?;
        // Generators and simulations share one index space.
        let gen_max = (bank.len() + sim_bank.len()).saturating_sub(1) as i64;

        // Register the per-layer control surface.
        let mut layers = Vec::with_capacity(NUM_LAYERS);
        {
            let store = engine.params_mut();
            for i in 0..NUM_LAYERS {
                let g = format!("Layer {}", i + 1);
                let pre = format!("layer.{i}");
                let f = |_n: &str, lo: f32, hi: f32, def: f32| ParamKind::Float { min: lo, max: hi, default: def };

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
                // Per-layer transform modifiers, wired into every generator via av_coord.
                let zoom = store.register(ParamSpec::new(format!("{pre}.zoom"), "Zoom", &g, f("", 0.1, 4.0, 1.0)));
                let rot = store.register(ParamSpec::new(format!("{pre}.rotate"), "Rotate", &g, f("", -1.0, 1.0, 0.0)));
                let posx = store.register(ParamSpec::new(format!("{pre}.posx"), "Pan X", &g, f("", -1.0, 1.0, 0.0)));
                let posy = store.register(ParamSpec::new(format!("{pre}.posy"), "Pan Y", &g, f("", -1.0, 1.0, 0.0)));

                layers.push(LayerParams { generator, opacity, blend, speed, scale, warp, hue, p1, p2, zoom, rot, posx, posy });
            }
        }
        let vert = include_str!("shaders/fullscreen.vert");
        let blend_prog = Program::new(gl, flavor, vert, include_str!("shaders/composite/blend.frag"))?;

        let (w, h) = (width.max(1), height.max(1));
        let mut layer_targets = Vec::with_capacity(NUM_LAYERS);
        let mut state = Vec::with_capacity(NUM_LAYERS);
        let mut float_state = true;
        for _ in 0..NUM_LAYERS {
            layer_targets.push(RenderTexture::new(gl, w, h)?);
            let (pp, is_float) = PingPong::new_sim(gl, w, h);
            float_state &= is_float;
            state.push(pp);
        }
        if !float_state {
            tracing::warn!("float render targets unavailable; simulations run at 8-bit precision");
        }
        let acc = PingPong::new(gl, w, h)?;

        Ok(Self {
            gl: gl.clone(),
            bank,
            sim_bank,
            media,
            layer_targets,
            state,
            last_gen: vec![-1; NUM_LAYERS],
            acc,
            blend_prog,
            layers,
            width: w,
            height: h,
            audio: (0.0, 0.0, 0.0),
            beat: 0.0,
            wave_tex: None,
        })
    }

    /// Set the waveform texture sampled by the scope generator.
    pub fn set_wave_tex(&mut self, tex: glow::Texture) {
        self.wave_tex = Some(tex);
    }

    pub fn generator_count(&self) -> usize {
        self.bank.len() + self.sim_bank.len()
    }

    /// Resize the internal render targets (the internal size may be smaller
    /// than the output when render-scale < 1).
    pub fn resize(&mut self, width: i32, height: i32) -> Result<(), String> {
        let (w, h) = (width.max(1), height.max(1));
        if w == self.width && h == self.height {
            return Ok(());
        }
        self.layer_targets.clear();
        self.state.clear();
        for _ in 0..NUM_LAYERS {
            self.layer_targets.push(RenderTexture::new(&self.gl, w, h)?);
            self.state.push(PingPong::new_sim(&self.gl, w, h).0);
        }
        self.last_gen = vec![-1; NUM_LAYERS]; // force reseed at the new size
        self.acc = PingPong::new(&self.gl, w, h)?;
        self.width = w;
        self.height = h;
        Ok(())
    }

    /// Update the audio band energies + onset pulse generators react to.
    pub fn set_audio(&mut self, low: f32, mid: f32, high: f32, beat: f32) {
        self.audio = (low, mid, high);
        self.beat = beat;
    }

    /// Texture holding the most recently composited frame (post chain input).
    pub fn result(&self) -> glow::Texture {
        self.acc.read()
    }

    /// Render the full stack into the internal accumulator. The result is left
    /// in [`result`]; presenting it to the screen is the post chain's job.
    pub fn render(&mut self, quad: &FullscreenQuad, engine: &Engine, time: f32) {
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
                beat: self.beat,
                zoom: p.get_f32(lp.zoom),
                rot: p.get_f32(lp.rot) * std::f32::consts::PI,
                pan: (p.get_f32(lp.posx), p.get_f32(lp.posy)),
            };
            let gen = p.get(lp.generator).as_i64().max(0) as usize;
            let stateless = self.bank.len();
            if gen < stateless {
                self.layer_targets[i].bind_as_target();
                self.bank.draw(gen, quad, &u, self.wave_tex.unwrap_or(self.layer_targets[i].texture()));
                self.last_gen[i] = gen as i64;
            } else {
                // Stateful simulation: (re)seed on selection, step, then render.
                let sim = gen - stateless;
                if self.last_gen[i] != gen as i64 {
                    self.state[i].write_target().bind_as_target();
                    self.sim_bank.seed(sim, quad, &u);
                    self.state[i].swap();
                    self.last_gen[i] = gen as i64;
                }
                let texel = (1.0 / self.width as f32, 1.0 / self.height as f32);
                for _ in 0..self.sim_bank.iters(sim) {
                    self.state[i].write_target().bind_as_target();
                    self.sim_bank.step(sim, quad, self.state[i].read(), &u, texel);
                    self.state[i].swap();
                }
                self.layer_targets[i].bind_as_target();
                self.sim_bank.render(sim, quad, self.state[i].read(), &u, texel);
            }
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

        // 3. Blend the media (image/SVG) layers on top of the generator stack.
        self.media.render(quad, engine, &mut self.acc, self.width, self.height);
        // The composited frame now sits in `self.acc.read()` (see `result`).
    }

    /// Dropdown labels for the media source params (index 0 = none).
    pub fn media_names(&self) -> Vec<String> {
        self.media.names().to_vec()
    }
}
