//! The frame pipeline.
//!
//! Owns the shared full-screen quad and the layer compositor, and computes the
//! internal render size from the output size and the render-scale knob (lower
//! scale = fewer pixels shaded, the main lever for weak GPUs). The analog and
//! glitch post chains attach inside the compositor in later milestones.

use crate::audio::WAVE;
use crate::engine::Engine;

use super::compositor::Compositor;
use super::gl::{self, FullscreenQuad, Gl, GlslFlavor};
use super::post::PostChain;
use super::text::TextOverlay;
use super::FrameContext;

/// Owns the GL resources for the render target and draws frames into it.
pub struct Pipeline {
    gl: Gl,
    quad: FullscreenQuad,
    compositor: Compositor,
    post: PostChain,
    text: TextOverlay,
    /// Waveform texture (256x1, R=L G=R) uploaded each frame for the scope.
    wave_tex: glow::Texture,
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
        let mut compositor = Compositor::new(gl, flavor, engine, iw, ih)?;
        let post = PostChain::new(gl, flavor, engine, iw, ih)?;
        let text = TextOverlay::new(gl, flavor)?;
        let wave_tex = gl::make_texture(gl, WAVE as i32, 1, None, false);
        compositor.set_wave_tex(wave_tex);
        Ok(Self {
            gl: gl.clone(),
            quad,
            compositor,
            post,
            text,
            wave_tex,
            render_scale: scale,
            out_w: width.max(1),
            out_h: height.max(1),
        })
    }

    /// Upload the latest stereo waveform (interleaved L,R) for the scope.
    pub fn set_waveform(&self, samples: &[f32]) {
        let n = WAVE;
        let mut bytes = vec![0u8; n * 4];
        for i in 0..n {
            let l = samples.get(2 * i).copied().unwrap_or(0.0);
            let r = samples.get(2 * i + 1).copied().unwrap_or(0.0);
            bytes[i * 4] = ((l * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
            bytes[i * 4 + 1] = ((r * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
            bytes[i * 4 + 3] = 255;
        }
        gl::update_texture(&self.gl, self.wave_tex, n as i32, 1, &bytes);
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
            tracing::warn!("compositor resize failed: {e}");
        }
        if let Err(e) = self.post.resize(iw, ih) {
            tracing::warn!("post resize failed: {e}");
        }
    }

    /// Push the latest audio band energies + onset to generators and post fx.
    pub fn set_audio(&mut self, low: f32, mid: f32, high: f32, beat: f32) {
        self.compositor.set_audio(low, mid, high, beat);
        self.post.set_audio(low, mid, high, beat);
    }

    /// Render one frame, ending with the result on the screen framebuffer.
    pub fn render(&mut self, frame: &FrameContext, engine: &Engine) {
        self.compositor.render(&self.quad, engine, frame.time);
        self.post.process(
            &self.quad,
            self.compositor.result(),
            engine,
            frame.time,
            self.out_w as i32,
            self.out_h as i32,
        );
        // Lettering goes on last, over the finished frame on the screen.
        self.text.draw(&self.quad, engine, frame.time, self.out_w as i32, self.out_h as i32);
    }
}

/// Internal render resolution from the output size and scale.
fn internal_size(width: u32, height: u32, scale: f32) -> (i32, i32) {
    let w = ((width as f32 * scale).round() as i32).max(1);
    let h = ((height as f32 * scale).round() as i32).max(1);
    (w, h)
}
