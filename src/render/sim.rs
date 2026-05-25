//! Stateful generators (simulations).
//!
//! Unlike the stateless generators, these evolve a state texture frame to frame:
//! the compositor keeps a per-layer ping-pong buffer, runs the `step` shader N
//! times (reading the previous state, writing the next), then the `render`
//! shader colourises the state into the layer. A `seed` shader initialises the
//! state when the simulation is first selected.
//!
//! All three share the generator uniform vocabulary, so their knobs
//! (Param 1/2 = the regime, Scale/Warp = flow, plus audio) are fully
//! modulatable through the matrix like everything else.

use super::generators::{apply_common, CommonUniforms};
use super::gl::{FullscreenQuad, Gl, GlslFlavor, Program};

pub struct SimDef {
    pub name: &'static str,
    step: &'static str,
    render: &'static str,
    seed: &'static str,
    /// Sub-steps per frame (reaction-diffusion wants several).
    pub iters: u32,
}

macro_rules! sim {
    ($name:literal, $step:literal, $render:literal, $seed:literal, $iters:literal) => {
        SimDef {
            name: $name,
            step: include_str!($step),
            render: include_str!($render),
            seed: include_str!($seed),
            iters: $iters,
        }
    };
}

/// The simulation library. Indices follow the stateless generators in the
/// combined `layer.N.generator` space.
pub static SIMS: &[SimDef] = &[
    sim!("reaction", "shaders/sim/rd_step.frag", "shaders/sim/rd_render.frag", "shaders/sim/rd_seed.frag", 10),
    sim!("spirals", "shaders/sim/ca_step.frag", "shaders/sim/ca_render.frag", "shaders/sim/ca_seed.frag", 2),
    sim!("smoke", "shaders/sim/smoke_step.frag", "shaders/sim/smoke_render.frag", "shaders/sim/smoke_seed.frag", 1),
];

struct SimProgs {
    step: Program,
    render: Program,
    seed: Program,
    iters: u32,
}

pub struct SimBank {
    sims: Vec<SimProgs>,
}

impl SimBank {
    pub fn new(gl: &Gl, flavor: GlslFlavor) -> Result<Self, String> {
        let lib = include_str!("shaders/lib.glsl");
        let vert = include_str!("shaders/fullscreen.vert");
        let build = |src: &str, what: &str| -> Result<Program, String> {
            Program::new(gl, flavor, vert, &format!("{lib}\n{src}")).map_err(|e| format!("{what}: {e}"))
        };
        let mut sims = Vec::with_capacity(SIMS.len());
        for s in SIMS {
            sims.push(SimProgs {
                step: build(s.step, s.name)?,
                render: build(s.render, s.name)?,
                seed: build(s.seed, s.name)?,
                iters: s.iters,
            });
        }
        tracing::info!("compiled {} simulations", sims.len());
        Ok(Self { sims })
    }

    pub fn len(&self) -> usize {
        self.sims.len()
    }

    pub fn name(index: usize) -> &'static str {
        SIMS.get(index).map(|s| s.name).unwrap_or("?")
    }

    pub fn iters(&self, index: usize) -> u32 {
        self.sims.get(index).map(|s| s.iters).unwrap_or(1)
    }

    /// Seed pass: target already bound. Initialises the state.
    pub fn seed(&self, index: usize, quad: &FullscreenQuad, u: &CommonUniforms) {
        let s = &self.sims[index.min(self.sims.len() - 1)];
        s.seed.bind();
        apply_common(&s.seed, u);
        quad.draw();
    }

    /// One simulation step: reads `state`, writes the bound target.
    pub fn step(&self, index: usize, quad: &FullscreenQuad, state: glow::Texture, u: &CommonUniforms, texel: (f32, f32)) {
        let s = &self.sims[index.min(self.sims.len() - 1)];
        s.step.bind();
        apply_common(&s.step, u);
        s.step.set_vec2("u_texel", texel.0, texel.1);
        s.step.set_texture("u_state", 0, state);
        quad.draw();
    }

    /// Colourise the state into the bound layer target.
    pub fn render(&self, index: usize, quad: &FullscreenQuad, state: glow::Texture, u: &CommonUniforms, texel: (f32, f32)) {
        let s = &self.sims[index.min(self.sims.len() - 1)];
        s.render.bind();
        apply_common(&s.render, u);
        s.render.set_vec2("u_texel", texel.0, texel.1);
        s.render.set_texture("u_state", 0, state);
        quad.draw();
    }
}
