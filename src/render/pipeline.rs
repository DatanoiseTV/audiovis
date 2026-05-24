//! The frame pipeline.
//!
//! At this milestone it renders a single full-screen pass to prove the GL path
//! works on the target dialects. The generator stack, layer compositor and the
//! analog/glitch post chain are layered in here in following milestones; the
//! shape (resolve params once, then draw passes each frame) stays the same.

use crate::engine::Engine;
use crate::params::ParamId;

use super::gl::{self, FullscreenQuad, Gl, GlslFlavor, Program};
use super::FrameContext;

/// Handles to the global parameters the pipeline reads every frame. Resolved
/// once at construction so the hot path is index lookups, not string hashing.
struct Globals {
    brightness: Option<ParamId>,
}

impl Globals {
    fn resolve(engine: &Engine) -> Self {
        Self { brightness: engine.params().id_of("global.brightness") }
    }

    fn brightness(&self, engine: &Engine) -> f32 {
        self.brightness.map(|id| engine.params().get_f32(id)).unwrap_or(1.0)
    }
}

/// Owns the GL resources for a render target and draws frames into it.
pub struct Pipeline {
    gl: Gl,
    quad: FullscreenQuad,
    test: Program,
    globals: Globals,
    width: u32,
    height: u32,
}

impl Pipeline {
    pub fn new(gl: &Gl, flavor: GlslFlavor, engine: &Engine, width: u32, height: u32) -> Result<Self, String> {
        let quad = FullscreenQuad::new(gl, flavor)?;
        let test = Program::new(
            gl,
            flavor,
            include_str!("shaders/fullscreen.vert"),
            include_str!("shaders/test_plasma.frag"),
        )?;
        Ok(Self {
            gl: gl.clone(),
            quad,
            test,
            globals: Globals::resolve(engine),
            width,
            height,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
    }

    /// Render one frame straight to the bound output framebuffer.
    pub fn render(&mut self, frame: &FrameContext, engine: &Engine) {
        gl::bind_screen(&self.gl, self.width as i32, self.height as i32);

        self.test.bind();
        self.test.set_f32("u_time", frame.time);
        self.test.set_vec2("u_res", self.width as f32, self.height as f32);
        self.test.set_f32("u_brightness", self.globals.brightness(engine));
        self.quad.draw();
    }
}
