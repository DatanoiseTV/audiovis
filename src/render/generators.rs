//! The generator library.
//!
//! Each generator is a full-screen fragment shader sharing one helper lib and
//! one uniform vocabulary. Adding a generator is two lines: drop a `.frag` in
//! `shaders/gen/` and add an entry to [`GENERATORS`]. Every generator reads the
//! same generic knobs (speed/scale/warp/hue/p1/p2) and audio bands, so a single
//! control surface drives all of them.

use super::gl::{FullscreenQuad, Gl, GlslFlavor, Program};

/// A named generator and its fragment source.
pub struct GenDef {
    pub name: &'static str,
    src: &'static str,
}

macro_rules! gen {
    ($name:literal, $path:literal) => {
        GenDef { name: $name, src: include_str!($path) }
    };
}

/// The library. Order is stable: it is the index space used by the
/// `layer.N.generator` parameter and the web UI dropdown.
pub static GENERATORS: &[GenDef] = &[
    gen!("plasma", "shaders/gen/plasma.frag"),
    gen!("tunnel", "shaders/gen/tunnel.frag"),
    gen!("flow", "shaders/gen/flow.frag"),
    gen!("kaleido", "shaders/gen/kaleido.frag"),
    gen!("metaballs", "shaders/gen/metaballs.frag"),
    gen!("voronoi", "shaders/gen/voronoi.frag"),
    gen!("moire", "shaders/gen/moire.frag"),
    gen!("rings", "shaders/gen/rings.frag"),
    gen!("starfield", "shaders/gen/starfield.frag"),
    gen!("warpgrid", "shaders/gen/warpgrid.frag"),
    gen!("lissajous", "shaders/gen/lissajous.frag"),
    gen!("spectrum", "shaders/gen/spectrum.frag"),
    gen!("colorbars", "shaders/gen/colorbars.frag"),
    // Demoscene batch.
    gen!("rotozoom", "shaders/gen/rotozoom.frag"),
    gen!("fire", "shaders/gen/fire.frag"),
    gen!("copperbars", "shaders/gen/copperbars.frag"),
    gen!("twister", "shaders/gen/twister.frag"),
    gen!("mandelbrot", "shaders/gen/mandelbrot.frag"),
    gen!("julia", "shaders/gen/julia.frag"),
    gen!("raymarch", "shaders/gen/raymarch.frag"),
    gen!("truchet", "shaders/gen/truchet.frag"),
    gen!("hexgrid", "shaders/gen/hexgrid.frag"),
    gen!("spiral", "shaders/gen/spiral.frag"),
    gen!("phyllotaxis", "shaders/gen/phyllotaxis.frag"),
    gen!("interference", "shaders/gen/interference.frag"),
    gen!("cylinder", "shaders/gen/cylinder.frag"),
    gen!("sierpinski", "shaders/gen/sierpinski.frag"),
    gen!("floorgrid", "shaders/gen/floorgrid.frag"),
    gen!("mandala", "shaders/gen/mandala.frag"),
    gen!("lightning", "shaders/gen/lightning.frag"),
    gen!("clouds", "shaders/gen/clouds.frag"),
    gen!("wormhole", "shaders/gen/wormhole.frag"),
    gen!("bobs", "shaders/gen/bobs.frag"),
    gen!("scope", "shaders/gen/scope.frag"),
    gen!("wireframe", "shaders/gen/wireframe.frag"),
];

/// The values fed to a generator for one draw.
#[derive(Debug, Clone, Copy)]
pub struct CommonUniforms {
    pub time: f32,
    pub res: (f32, f32),
    pub speed: f32,
    pub scale: f32,
    pub warp: f32,
    pub hue: f32,
    pub p1: f32,
    pub p2: f32,
    /// Low / mid / high band energy in 0..1.
    pub audio: (f32, f32, f32),
    /// Onset pulse, 0..1.
    pub beat: f32,
    /// Per-layer transform: zoom (1 = none), rotation (radians), pan (x, y).
    pub zoom: f32,
    pub rot: f32,
    pub pan: (f32, f32),
}

impl Default for CommonUniforms {
    fn default() -> Self {
        Self {
            time: 0.0,
            res: (1.0, 1.0),
            speed: 1.0,
            scale: 1.0,
            warp: 0.0,
            hue: 0.0,
            p1: 0.5,
            p2: 0.5,
            audio: (0.0, 0.0, 0.0),
            beat: 0.0,
            zoom: 1.0,
            rot: 0.0,
            pan: (0.0, 0.0),
        }
    }
}

/// All generator programs, compiled once for the active GLSL flavor.
pub struct GeneratorBank {
    programs: Vec<Program>,
}

impl GeneratorBank {
    pub fn new(gl: &Gl, flavor: GlslFlavor) -> Result<Self, String> {
        let lib = include_str!("shaders/lib.glsl");
        let vert = include_str!("shaders/fullscreen.vert");
        let mut programs = Vec::with_capacity(GENERATORS.len());
        for g in GENERATORS {
            // The helper lib is prepended; the flavor header is added inside
            // Program::new ahead of both.
            let body = format!("{lib}\n{}", g.src);
            let prog = Program::new(gl, flavor, vert, &body)
                .map_err(|e| format!("generator '{}': {e}", g.name))?;
            programs.push(prog);
        }
        tracing::info!("compiled {} generators", programs.len());
        Ok(Self { programs })
    }

    pub fn len(&self) -> usize {
        self.programs.len()
    }

    pub fn name(index: usize) -> &'static str {
        GENERATORS.get(index).map(|g| g.name).unwrap_or("?")
    }

    /// Bind the generator at `index`, upload uniforms and draw the quad. The
    /// caller has already bound the target framebuffer. `wave` is the waveform
    /// texture (sampled by the scope generator).
    pub fn draw(&self, index: usize, quad: &FullscreenQuad, u: &CommonUniforms, wave: glow::Texture) {
        let i = index.min(self.programs.len().saturating_sub(1));
        let p = &self.programs[i];
        p.bind();
        apply_common(p, u);
        p.set_texture("u_wave", 2, wave);
        quad.draw();
    }
}

/// Upload the shared uniform set to a bound program. Used by generators and the
/// simulation bank so both speak the same vocabulary.
pub fn apply_common(p: &Program, u: &CommonUniforms) {
    p.set_f32("u_time", u.time);
    p.set_vec2("u_res", u.res.0, u.res.1);
    p.set_f32("u_speed", u.speed);
    p.set_f32("u_scale", u.scale);
    p.set_f32("u_warp", u.warp);
    p.set_f32("u_hue", u.hue);
    p.set_f32("u_p1", u.p1);
    p.set_f32("u_p2", u.p2);
    p.set_vec3("u_audio", u.audio.0, u.audio.1, u.audio.2);
    p.set_f32("u_beat", u.beat);
    p.set_f32("u_xzoom", u.zoom);
    p.set_f32("u_xrot", u.rot);
    p.set_vec2("u_xoff", u.pan.0, u.pan.1);
}
