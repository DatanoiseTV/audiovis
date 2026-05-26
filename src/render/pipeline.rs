//! The frame pipeline.
//!
//! Owns the shared full-screen quad and the layer compositor, and computes the
//! internal render size from the output size and the render-scale knob (lower
//! scale = fewer pixels shaded, the main lever for weak GPUs). The analog and
//! glitch post chains attach inside the compositor in later milestones.

use crate::audio::WAVE;
use crate::engine::Engine;

use super::compositor::Compositor;
use super::gl::{self, FullscreenQuad, Gl, GlslFlavor, RenderTexture};
use super::post::PostChain;
use super::text::TextOverlay;
use super::FrameContext;

/// Live web-monitor preview size.
pub const PREVIEW_W: i32 = 256;
pub const PREVIEW_H: i32 = 144;

/// Owns the GL resources for the render target and draws frames into it.
pub struct Pipeline {
    gl: Gl,
    quad: FullscreenQuad,
    compositor: Compositor,
    post: PostChain,
    text: TextOverlay,
    /// Waveform texture (256x1, R=L G=R) uploaded each frame for the scope.
    wave_tex: glow::Texture,
    /// Pixel-buffer texture the JS script draws into (nearest-filtered).
    script_tex: glow::Texture,
    /// Live camera frame texture (re-uploaded when a new frame arrives).
    camera_tex: glow::Texture,
    /// Small target the final frame is copied into for the web monitor.
    preview: RenderTexture,
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
        let script_tex = gl::make_texture(gl, crate::script::SCRIPT_W as i32, crate::script::SCRIPT_H as i32, None, true);
        compositor.set_script_tex(script_tex);
        let camera_tex = gl::make_texture(gl, 1, 1, Some(&[0, 0, 0, 255]), false);
        compositor.set_camera_tex(camera_tex);
        let preview = RenderTexture::new(gl, PREVIEW_W, PREVIEW_H)?;
        Ok(Self {
            gl: gl.clone(),
            quad,
            compositor,
            post,
            text,
            wave_tex,
            script_tex,
            camera_tex,
            preview,
            render_scale: scale,
            out_w: width.max(1),
            out_h: height.max(1),
        })
    }

    /// Read the latest preview frame as top-down RGBA (PREVIEW_W x PREVIEW_H).
    pub fn read_preview(&self) -> Vec<u8> {
        self.preview.bind_as_target();
        let bottom_up = gl::read_rgba(&self.gl, PREVIEW_W, PREVIEW_H);
        // Restore the default framebuffer so a following screenshot read-back
        // (or any direct screen access) does not pick up the preview buffer.
        gl::bind_screen(&self.gl, self.out_w as i32, self.out_h as i32);
        // Flip vertically: GL returns bottom-up, image consumers want top-down.
        let (w, h) = (PREVIEW_W as usize, PREVIEW_H as usize);
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            let src = &bottom_up[(h - 1 - y) * w * 4..(h - y) * w * 4];
            out[y * w * 4..(y + 1) * w * 4].copy_from_slice(src);
        }
        out
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

    /// Upload the JS script's RGBA pixel buffer (SCRIPT_W x SCRIPT_H) for the
    /// script generator to display.
    pub fn set_script_buffer(&self, rgba: &[u8]) {
        gl::update_texture(&self.gl, self.script_tex, crate::script::SCRIPT_W as i32, crate::script::SCRIPT_H as i32, rgba);
    }

    /// Upload a camera frame (RGBA, top-down) for the camera generator.
    pub fn set_camera_frame(&self, w: u32, h: u32, rgba: &[u8]) {
        gl::update_texture(&self.gl, self.camera_tex, w as i32, h as i32, rgba);
    }

    /// Number of generators available, for the UI.
    pub fn generator_count(&self) -> usize {
        self.compositor.generator_count()
    }

    /// Dropdown labels for the media source params (index 0 = none).
    pub fn media_names(&self) -> Vec<String> {
        self.compositor.media_names()
    }

    /// Dropdown labels for the wireframe mesh source (index 0 = procedural).
    pub fn mesh_names(&self) -> Vec<String> {
        self.compositor.mesh_names()
    }

    /// Dropdown labels for the ISF shader source (index 0 = off).
    pub fn isf_names(&self) -> Vec<String> {
        self.compositor.isf_names()
    }

    /// Pending ISF input/error update to publish, if a shader just (re)loaded.
    pub fn isf_take_dirty(&mut self) -> Option<(String, Vec<(String, String, usize)>)> {
        self.compositor.isf_take_dirty()
    }

    /// Re-scan the media directory (picks up newly added files).
    pub fn rescan_media(&mut self) {
        self.compositor.rescan_media();
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
        self.compositor.render(&self.quad, engine, frame.time, frame.dt, frame.frame);
        self.post.process(
            &self.quad,
            self.compositor.result(),
            engine,
            frame.time,
            self.out_w as i32,
            self.out_h as i32,
            Some(&self.preview),
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
