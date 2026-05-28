//! Thin helpers over `glow` that keep the rest of the renderer readable.
//!
//! Everything here targets the common subset of OpenGL ES 2.0 (GLSL 1.00) and
//! desktop OpenGL 2.1 (GLSL 1.20): no VAOs, no integer textures, no MRT. That is
//! the price of running on a Pi Zero's VideoCore IV or a C.H.I.P.'s Mali-400,
//! and it is plenty for full-screen fragment-shader visuals.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use glow::HasContext;

/// Shared GL context handle. `Rc` because the backend, pipeline and resources
/// all hold the same single-threaded context.
pub type Gl = Rc<glow::Context>;

/// Which GLSL dialect the active context speaks. The shader bodies are written
/// once against a small macro vocabulary (`ATTRIBUTE`, `VARYING`, `FRAG_COLOR`,
/// `TEX2D`); we prepend the right header for the dialect at compile time.
///
/// macOS only hands out Core profile contexts, so the desktop dialect is GL 3.3
/// Core (which spells things `in`/`out`/`texture()`); the boards run real GLES2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlslFlavor {
    /// OpenGL ES 2.0, GLSL ES 1.00 (the small ARM boards, DRM backend).
    Es2,
    /// Desktop OpenGL 3.3 Core, GLSL 3.30 (macOS/Linux window backend).
    GlCore,
}

impl GlslFlavor {
    /// Core profile requires a bound Vertex Array Object for every draw; GLES2
    /// does not (and may lack the extension), so we only create one for Core.
    pub fn needs_vao(self) -> bool {
        matches!(self, GlslFlavor::GlCore)
    }
}

#[derive(Clone, Copy)]
enum Stage {
    Vertex,
    Fragment,
}

impl GlslFlavor {
    /// The header injected ahead of a shader body. Note `VARYING` differs by
    /// stage on Core (`out` in the vertex shader, `in` in the fragment shader).
    fn header(self, stage: Stage) -> &'static str {
        match (self, stage) {
            (GlslFlavor::Es2, Stage::Vertex) => {
                "#version 100\n\
                 #define ATTRIBUTE attribute\n\
                 #define VARYING varying\n"
            }
            (GlslFlavor::Es2, Stage::Fragment) => {
                "#version 100\n\
                 precision highp float;\n\
                 #define VARYING varying\n\
                 #define FRAG_COLOR gl_FragColor\n\
                 #define TEX2D texture2D\n"
            }
            (GlslFlavor::GlCore, Stage::Vertex) => {
                "#version 330 core\n\
                 #define ATTRIBUTE in\n\
                 #define VARYING out\n"
            }
            (GlslFlavor::GlCore, Stage::Fragment) => {
                "#version 330 core\n\
                 #define VARYING in\n\
                 #define TEX2D texture\n\
                 out vec4 av_fragColor;\n\
                 #define FRAG_COLOR av_fragColor\n"
            }
        }
    }
}

/// A linked shader program plus a lazy cache of uniform locations.
pub struct Program {
    prog: glow::Program,
    gl: Gl,
    locs: RefCell<HashMap<String, Option<glow::UniformLocation>>>,
}

impl Program {
    /// Compile and link a vertex+fragment program. The vertex position
    /// attribute is bound to slot 0 under the name `a_pos`, matching
    /// [`FullscreenQuad`].
    pub fn new(gl: &Gl, flavor: GlslFlavor, vert_body: &str, frag_body: &str) -> Result<Self, String> {
        unsafe {
            let vert = compile(gl, glow::VERTEX_SHADER, &flavor.header(Stage::Vertex), vert_body)?;
            let frag = compile(gl, glow::FRAGMENT_SHADER, &flavor.header(Stage::Fragment), frag_body)?;

            let prog = gl.create_program()?;
            gl.attach_shader(prog, vert);
            gl.attach_shader(prog, frag);
            gl.bind_attrib_location(prog, 0, "a_pos");
            gl.link_program(prog);

            // Shaders can be detached/deleted once linked.
            gl.detach_shader(prog, vert);
            gl.detach_shader(prog, frag);
            gl.delete_shader(vert);
            gl.delete_shader(frag);

            if !gl.get_program_link_status(prog) {
                let log = gl.get_program_info_log(prog);
                gl.delete_program(prog);
                return Err(format!("program link failed: {log}"));
            }

            Ok(Self { prog, gl: gl.clone(), locs: RefCell::new(HashMap::new()) })
        }
    }

    pub fn bind(&self) {
        unsafe { self.gl.use_program(Some(self.prog)) }
    }

    fn loc(&self, name: &str) -> Option<glow::UniformLocation> {
        if let Some(cached) = self.locs.borrow().get(name) {
            return *cached;
        }
        let loc = unsafe { self.gl.get_uniform_location(self.prog, name) };
        self.locs.borrow_mut().insert(name.to_string(), loc);
        loc
    }

    pub fn set_f32(&self, name: &str, v: f32) {
        if let Some(l) = self.loc(name) {
            unsafe { self.gl.uniform_1_f32(Some(&l), v) }
        }
    }

    pub fn set_vec2(&self, name: &str, x: f32, y: f32) {
        if let Some(l) = self.loc(name) {
            unsafe { self.gl.uniform_2_f32(Some(&l), x, y) }
        }
    }

    pub fn set_vec3(&self, name: &str, x: f32, y: f32, z: f32) {
        if let Some(l) = self.loc(name) {
            unsafe { self.gl.uniform_3_f32(Some(&l), x, y, z) }
        }
    }

    pub fn set_vec4(&self, name: &str, x: f32, y: f32, z: f32, w: f32) {
        if let Some(l) = self.loc(name) {
            unsafe { self.gl.uniform_4_f32(Some(&l), x, y, z, w) }
        }
    }

    pub fn set_i32(&self, name: &str, v: i32) {
        if let Some(l) = self.loc(name) {
            unsafe { self.gl.uniform_1_i32(Some(&l), v) }
        }
    }

    /// Bind `tex` to a texture unit and point the named sampler at it.
    pub fn set_texture(&self, name: &str, unit: u32, tex: glow::Texture) {
        unsafe {
            self.gl.active_texture(glow::TEXTURE0 + unit);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        }
        self.set_i32(name, unit as i32);
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        unsafe { self.gl.delete_program(self.prog) }
    }
}

unsafe fn compile(gl: &Gl, kind: u32, header: &str, body: &str) -> Result<glow::Shader, String> {
    let shader = gl.create_shader(kind)?;
    gl.shader_source(shader, &format!("{header}{body}"));
    gl.compile_shader(shader);
    if !gl.get_shader_compile_status(shader) {
        let log = gl.get_shader_info_log(shader);
        gl.delete_shader(shader);
        return Err(format!("shader compile failed: {log}"));
    }
    Ok(shader)
}

/// A single triangle that covers the whole viewport. Cheaper than a quad and
/// avoids a diagonal seam. The vertex shader derives UVs from clip position.
pub struct FullscreenQuad {
    vbo: glow::Buffer,
    /// Present only on Core profile, where a VAO is mandatory for draws.
    vao: Option<glow::VertexArray>,
    gl: Gl,
}

impl FullscreenQuad {
    pub fn new(gl: &Gl, flavor: GlslFlavor) -> Result<Self, String> {
        // Oversized triangle: clip coords -1..3 cover the -1..1 screen.
        let verts: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
        unsafe {
            let vao = if flavor.needs_vao() {
                let v = gl.create_vertex_array()?;
                gl.bind_vertex_array(Some(v));
                Some(v)
            } else {
                None
            };

            let vbo = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&verts), glow::STATIC_DRAW);

            // On Core the attribute layout is captured into the VAO now; on
            // GLES2 it is set per draw (see `draw`).
            if vao.is_some() {
                gl.enable_vertex_attrib_array(0);
                gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 0, 0);
                gl.bind_vertex_array(None);
            }

            Ok(Self { vbo, vao, gl: gl.clone() })
        }
    }

    /// Draw the triangle with the currently-bound program.
    pub fn draw(&self) {
        unsafe {
            if let Some(vao) = self.vao {
                self.gl.bind_vertex_array(Some(vao));
                self.gl.draw_arrays(glow::TRIANGLES, 0, 3);
            } else {
                self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
                self.gl.enable_vertex_attrib_array(0);
                self.gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 0, 0);
                self.gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }
        }
    }
}

impl Drop for FullscreenQuad {
    fn drop(&mut self) {
        unsafe {
            if let Some(vao) = self.vao {
                self.gl.delete_vertex_array(vao);
            }
            self.gl.delete_buffer(self.vbo);
        }
    }
}

/// An off-screen RGBA8 render target. The building block for multi-pass effects
/// and feedback (see [`PingPong`]).
pub struct RenderTexture {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    pub width: i32,
    pub height: i32,
    gl: Gl,
}

impl RenderTexture {
    /// Standard 8-bit RGBA target.
    pub fn new(gl: &Gl, width: i32, height: i32) -> Result<Self, String> {
        Self::with_format(gl, width, height, glow::RGBA as i32, glow::UNSIGNED_BYTE)
    }

    /// 16-bit float RGBA target, for simulation state that needs precision
    /// (reaction-diffusion). Errors if the context can't render to it (the
    /// caller then falls back to [`new`]).
    pub fn new_float(gl: &Gl, width: i32, height: i32) -> Result<Self, String> {
        Self::with_format(gl, width, height, glow::RGBA16F as i32, glow::HALF_FLOAT)
    }

    fn with_format(gl: &Gl, width: i32, height: i32, internal: i32, ty: u32) -> Result<Self, String> {
        unsafe {
            let tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal,
                width.max(1),
                height.max(1),
                0,
                glow::RGBA,
                ty,
                glow::PixelUnpackData::Slice(None),
            );
            // Linear + clamp keeps non-power-of-two sizes valid on GLES2.
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);

            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if status != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(tex);
                return Err(format!("incomplete framebuffer: {status:#x}"));
            }
            Ok(Self { fbo, tex, width: width.max(1), height: height.max(1), gl: gl.clone() })
        }
    }

    /// Clear this target to a solid colour.
    pub fn clear_to(&self, r: f32, g: f32, b: f32, a: f32) {
        self.bind_as_target();
        unsafe {
            self.gl.clear_color(r, g, b, a);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    pub fn texture(&self) -> glow::Texture {
        self.tex
    }

    /// Make this texture the render target and set the viewport to match.
    pub fn bind_as_target(&self) {
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            self.gl.viewport(0, 0, self.width, self.height);
        }
    }
}

impl Drop for RenderTexture {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_framebuffer(self.fbo);
            self.gl.delete_texture(self.tex);
        }
    }
}

/// Two render textures swapped each pass, the standard way to read last frame
/// while writing this one (feedback, blur, reaction-diffusion).
pub struct PingPong {
    front: RenderTexture,
    back: RenderTexture,
}

impl PingPong {
    pub fn new(gl: &Gl, width: i32, height: i32) -> Result<Self, String> {
        Ok(Self {
            front: RenderTexture::new(gl, width, height)?,
            back: RenderTexture::new(gl, width, height)?,
        })
    }

    /// Float-precision pair, falling back to 8-bit if the context can't render
    /// to float (some GLES2 boards) - simulations still run, just coarser.
    pub fn new_sim(gl: &Gl, width: i32, height: i32) -> (Self, bool) {
        match (RenderTexture::new_float(gl, width, height), RenderTexture::new_float(gl, width, height)) {
            (Ok(front), Ok(back)) => (Self { front, back }, true),
            _ => (
                Self {
                    front: RenderTexture::new(gl, width, height).expect("rgba8 rt"),
                    back: RenderTexture::new(gl, width, height).expect("rgba8 rt"),
                },
                false,
            ),
        }
    }

    /// Clear both buffers to a colour (seed a simulation to a flat state).
    pub fn clear_to(&self, r: f32, g: f32, b: f32, a: f32) {
        self.front.clear_to(r, g, b, a);
        self.back.clear_to(r, g, b, a);
    }

    pub fn front(&self) -> &RenderTexture {
        &self.front
    }

    /// Texture holding the previous result (the one to sample).
    pub fn read(&self) -> glow::Texture {
        self.front.texture()
    }

    /// Target to render the new result into.
    pub fn write_target(&self) -> &RenderTexture {
        &self.back
    }

    /// After rendering into the write target, swap so it becomes readable.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.front, &mut self.back);
    }
}

/// Create a plain sampled RGBA8 texture (not a render target). `nearest` keeps
/// pixel art crisp. `data` may be `None` to allocate uninitialised storage.
pub fn make_texture(gl: &Gl, width: i32, height: i32, data: Option<&[u8]>, nearest: bool) -> glow::Texture {
    unsafe {
        let tex = gl.create_texture().expect("create texture");
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            width,
            height,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(data),
        );
        let filter = if nearest { glow::NEAREST } else { glow::LINEAR } as i32;
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, filter);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, filter);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        tex
    }
}

/// Replace the contents of an existing RGBA8 texture.
pub fn update_texture(gl: &Gl, tex: glow::Texture, width: i32, height: i32, data: &[u8]) {
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            width,
            height,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(data)),
        );
    }
}

/// Alpha-blend subsequent draws over the framebuffer (for overlays like text).
pub fn set_blend(gl: &Gl, on: bool) {
    unsafe {
        if on {
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        } else {
            gl.disable(glow::BLEND);
        }
    }
}

/// Bind the window/default framebuffer and set the viewport.
pub fn bind_screen(gl: &Gl, width: i32, height: i32) {
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.viewport(0, 0, width.max(1), height.max(1));
    }
}

pub fn clear(gl: &Gl, r: f32, g: f32, b: f32) {
    unsafe {
        gl.clear_color(r, g, b, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
    }
}

/// Read the bound framebuffer back to CPU as RGBA8, bottom-up (GL origin).
pub fn read_rgba(gl: &Gl, width: i32, height: i32) -> Vec<u8> {
    let mut buf = vec![0u8; (width * height * 4) as usize];
    unsafe {
        gl.read_pixels(
            0,
            0,
            width,
            height,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut buf)),
        );
    }
    buf
}

/// Reinterpret a `f32` slice as bytes for buffer uploads.
fn bytes<T>(data: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) }
}
