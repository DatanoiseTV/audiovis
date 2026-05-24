//! The frame pipeline.
//!
//! Owns the shared full-screen quad and the layer compositor, and computes the
//! internal render size from the output size and the render-scale knob (lower
//! scale = fewer pixels shaded, the main lever for weak GPUs). The analog and
//! glitch post chains attach inside the compositor in later milestones.

use crate::engine::Engine;

use super::compositor::Compositor;
use super::gl::{FullscreenQuad, Gl, GlslFlavor};
use super::FrameContext;

/// Owns the GL resources for the render target and draws frames into it.
pub struct Pipeline {
    quad: FullscreenQuad,
    compositor: Compositor,
    render_scale: f32,
    out_w: u32,
    out_h: u32,
}

impl Pipeline {
    pub fn new(
        gl: &Gl,
        flavor: GlslFlavor,
        engine: &mut Engine,
        width: u32,
        height: u32,
        render_scale: f32,
    ) -> Result<Self, String> {
        let quad = FullscreenQuad::new(gl, flavor)?;
        let scale = render_scale.clamp(0.1, 1.0);
        let (iw, ih) = internal_size(width, height, scale);
        let compositor = Compositor::new(gl, flavor, engine, iw, ih)?;
        Ok(Self { quad, compositor, render_scale: scale, out_w: width.max(1), out_h: height.max(1) })
    }

    /// Number of generators available, for the UI.
    pub fn generator_count(&self) -> usize {
        self.compositor.generator_count()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.out_w = width.max(1);
        self.out_h = height.max(1);
        let (iw, ih) = internal_size(width, height, self.render_scale);
        if let Err(e) = self.compositor.resize(iw, ih) {
            tracing::warn!("resize failed: {e}");
        }
    }

    /// Push the latest audio band energies through to the generators.
    pub fn set_audio(&mut self, low: f32, mid: f32, high: f32) {
        self.compositor.set_audio(low, mid, high);
    }

    /// Render one frame, ending with the result on the screen framebuffer.
    pub fn render(&mut self, frame: &FrameContext, engine: &Engine) {
        self.compositor
            .render(&self.quad, engine, frame.time, self.out_w as i32, self.out_h as i32);
    }
}

/// Internal render resolution from the output size and scale.
fn internal_size(width: u32, height: u32, scale: f32) -> (i32, i32) {
    let w = ((width as f32 * scale).round() as i32).max(1);
    let h = ((height as f32 * scale).round() as i32).max(1);
    (w, h)
}
