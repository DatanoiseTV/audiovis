//! In-browser WebGL renderer for audiovis.
//!
//! Loads a `<canvas>` from JS, pulls a WebGL1 context off it, hands that to
//! `glow`, and renders the *same* generator fragment shaders the native app
//! does. There is no JS / WebGL reimplementation of any shader: the GLSL ES
//! 1.00 sources are the host's, served through the shared
//! [`audiovis-render-core`] crate.
//!
//! This first cut is the architecture proof: it draws a single full-screen
//! generator from values passed in per frame (no compositor, FX, sims, ISF,
//! mesh, or media yet). Once this loop is live in a browser, those layers
//! follow as more of render-core is wired up - all of them share the same
//! glow context.

use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use audiovis_render_core::generators::{CommonUniforms, GeneratorBank};
use audiovis_render_core::gl::{self as core_gl, FullscreenQuad, Gl, GlslFlavor};

/// The renderer surface JS sees. Constructed once from a `<canvas>`; the JS
/// side then drives `render()` on each `requestAnimationFrame`.
#[wasm_bindgen]
pub struct Renderer {
    gl: Gl,
    bank: GeneratorBank,
    quad: FullscreenQuad,
    /// 256x1 placeholder waveform texture (the scope generator samples it; for
    /// generators that ignore it, having it bound is harmless).
    wave_tex: glow::Texture,
}

#[wasm_bindgen]
impl Renderer {
    /// Create the renderer against a `<canvas>` element. Errors are JS strings
    /// so the caller can put them straight on screen.
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: web_sys::HtmlCanvasElement) -> Result<Renderer, JsValue> {
        // Forward Rust panics to the browser console instead of an opaque trap.
        console_error_panic_hook::set_once();

        // WebGL1 = GLSL ES 1.00, which is exactly the `Es2` shader flavor the
        // boards run. No shader translation needed.
        let ctx = canvas
            .get_context("webgl")
            .map_err(|e| e)?
            .ok_or_else(|| JsValue::from_str("canvas: no WebGL context (need WebGL1 support)"))?
            .dyn_into::<web_sys::WebGlRenderingContext>()
            .map_err(|_| JsValue::from_str("canvas context is not a WebGLRenderingContext"))?;

        let gl: Gl = Rc::new(glow::Context::from_webgl1_context(ctx));
        let bank = GeneratorBank::new(&gl, GlslFlavor::Es2).map_err(jserr)?;
        let quad = FullscreenQuad::new(&gl, GlslFlavor::Es2).map_err(jserr)?;
        let wave_tex = core_gl::make_texture(&gl, 256, 1, None, false);

        Ok(Renderer { gl, bank, quad, wave_tex })
    }

    /// How many generators are compiled in (so JS can populate a dropdown).
    pub fn generator_count(&self) -> u32 {
        self.bank.len() as u32
    }

    /// Name of generator `idx` (out-of-range returns "?").
    pub fn generator_name(&self, idx: u32) -> String {
        GeneratorBank::name(idx as usize).to_string()
    }

    /// Resize the GL viewport to match the current canvas backing size.
    pub fn resize(&self, width: u32, height: u32) {
        use glow::HasContext as _;
        unsafe {
            self.gl.viewport(0, 0, width.max(1) as i32, height.max(1) as i32);
        }
    }

    /// Draw one frame of the chosen generator with the supplied state.
    ///
    /// Parameters mirror [`CommonUniforms`] - `time` and band energies are the
    /// per-frame inputs JS streams from the host (or fakes for the spike).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        generator: u32,
        time: f32,
        width: f32,
        height: f32,
        speed: f32,
        scale: f32,
        warp: f32,
        hue: f32,
        p1: f32,
        p2: f32,
        audio_low: f32,
        audio_mid: f32,
        audio_high: f32,
        beat: f32,
    ) {
        use glow::HasContext as _;
        unsafe {
            self.gl.viewport(0, 0, width.max(1.0) as i32, height.max(1.0) as i32);
            self.gl.clear_color(0.0, 0.0, 0.0, 1.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
        let u = CommonUniforms {
            time,
            res: (width, height),
            speed,
            scale,
            warp,
            hue,
            p1,
            p2,
            audio: (audio_low, audio_mid, audio_high),
            beat,
            zoom: 1.0,
            rot: 0.0,
            pan: (0.0, 0.0),
        };
        self.bank.draw(generator as usize, &self.quad, &u, self.wave_tex);
    }
}

fn jserr(msg: String) -> JsValue {
    JsValue::from_str(&msg)
}
