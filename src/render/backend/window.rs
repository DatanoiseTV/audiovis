//! Desktop window backend: winit for the window/event loop, glutin for the GL
//! context. We ask for OpenGL 2.1 so the shader dialect matches what the GLES2
//! boards run, keeping one shader code path across desktop preview and hardware.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentGlContext, PossiblyCurrentContext, Version};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use std::sync::Arc;

use crate::audio::AudioShared;
use crate::cli::Cli;
use crate::control::ControlBus;
use crate::engine::Engine;
use crate::web::WebHandle;
use crate::render::gl::{self, Gl};
use crate::render::pipeline::Pipeline;
use crate::render::{FrameContext, GlslFlavor};

/// Run the desktop window backend. Blocks until the window is closed (or the
/// frame budget set by `--frames`/`--screenshot` is reached).
pub fn run(cli: Cli, engine: Engine, bus: ControlBus, audio: Arc<AudioShared>, web: Option<WebHandle>) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let max_frames = if cli.frames > 0 {
        Some(cli.frames)
    } else if cli.screenshot.is_some() {
        // Render a couple of frames so animation has advanced, then capture.
        Some(2)
    } else {
        None
    };

    let mut app = WindowApp {
        cli,
        engine,
        bus,
        audio,
        web,
        gfx: None,
        start: Instant::now(),
        last: Instant::now(),
        frame: 0,
        max_frames,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

/// Live GL state, created once the event loop resumes.
struct Gfx {
    window: Window,
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    gl: Gl,
    pipeline: Pipeline,
}

struct WindowApp {
    cli: Cli,
    engine: Engine,
    bus: ControlBus,
    audio: Arc<AudioShared>,
    web: Option<WebHandle>,
    gfx: Option<Gfx>,
    start: Instant,
    last: Instant,
    frame: u64,
    max_frames: Option<u64>,
}

impl WindowApp {
    /// Build window, GL config, context, surface and pipeline. Runs on first
    /// resume (and again only if the platform tore the surface down).
    fn init(&mut self, el: &ActiveEventLoop) -> Result<()> {
        let attrs = Window::default_attributes()
            .with_title("audiovis")
            .with_inner_size(winit::dpi::LogicalSize::new(self.cli.width, self.cli.height));

        let template = ConfigTemplateBuilder::new().with_alpha_size(8);
        let display_builder = DisplayBuilder::new().with_window_attributes(Some(attrs));

        let (window, gl_config) = display_builder
            .build(el, template, |configs| {
                // Prefer the config with the most samples; fall back to first.
                configs
                    .reduce(|a, b| if b.num_samples() > a.num_samples() { b } else { a })
                    .expect("no GL configs")
            })
            .map_err(|e| anyhow!("GL display build failed: {e}"))?;
        let window = window.ok_or_else(|| anyhow!("no window from display builder"))?;

        let raw_handle = window.window_handle()?.as_raw();
        let gl_display = gl_config.display();

        // macOS only grants Core profile contexts, so target GL 3.3 Core; the
        // shader macro layer keeps the bodies identical to the GLES2 boards.
        let ctx_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .with_profile(GlProfile::Core)
            .build(Some(raw_handle));
        let not_current = unsafe { gl_display.create_context(&gl_config, &ctx_attrs) }
            .map_err(|e| anyhow!("create GL context failed: {e}"))?;

        let surface_attrs = window
            .build_surface_attributes(SurfaceAttributesBuilder::<WindowSurface>::new())
            .map_err(|e| anyhow!("surface attrs failed: {e}"))?;
        let surface = unsafe { gl_display.create_window_surface(&gl_config, &surface_attrs) }
            .map_err(|e| anyhow!("create surface failed: {e}"))?;

        let context = not_current
            .make_current(&surface)
            .map_err(|e| anyhow!("make current failed: {e}"))?;

        let gl: Gl = Rc::new(unsafe {
            glow::Context::from_loader_function_cstr(|s| gl_display.get_proc_address(s).cast())
        });

        // Vsync when possible; harmless if the platform ignores it.
        let _ = surface.set_swap_interval(&context, SwapInterval::Wait(NonZeroU32::new(1).unwrap()));

        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        let pipeline = Pipeline::new(&gl, GlslFlavor::GlCore, &mut self.engine, w, h, self.cli.render_scale)
            .map_err(|e| anyhow!("pipeline init failed: {e}"))?;

        // The pipeline has now registered all layer/effect parameters, so a
        // startup preset can safely resolve their paths.
        if let Some(preset) = self.cli.preset.clone() {
            if let Err(e) = self.engine.load_preset(&preset) {
                tracing::warn!("could not load preset {preset}: {e:#}");
            }
        }

        // Publish the now-complete parameter schema to the web UI.
        if let Some(web) = &self.web {
            let generators = crate::render::generators::GENERATORS.iter().map(|g| g.name.to_string()).collect();
            web.set_schema(&self.engine, generators);
        }

        tracing::info!("window backend up: {}x{} GL 3.3 Core via {}", w, h, renderer_name(&gl));

        self.gfx = Some(Gfx { window, surface, context, gl, pipeline });
        Ok(())
    }

    /// One rendered frame: pump control input, advance time, draw, present.
    fn draw(&mut self, el: &ActiveEventLoop) {
        // Drain queued control events into the authoritative engine state.
        let events: Vec<_> = self.bus.drain().collect();
        for ev in events {
            self.engine.handle(ev);
        }

        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32();
        self.last = now;

        // Feed the latest audio energies to the generators.
        let (low, mid, high) = self.audio.bands();

        // Push state to the web UI: param changes every frame, telemetry at a
        // calmer rate. Always drain notices so they cannot accumulate.
        let notices = self.engine.take_notices();
        if let Some(web) = &self.web {
            if !notices.is_empty() {
                web.publish_notices(&notices);
            }
            if self.frame % 4 == 0 {
                let p = self.engine.params();
                let read = |path: &str, dflt: f32| p.id_of(path).map(|id| p.get_f32(id)).unwrap_or(dflt);
                web.publish_telemetry(
                    low,
                    mid,
                    high,
                    self.audio.rms(),
                    self.audio.beat(),
                    read("clock.bpm", 120.0),
                    read("clock.beat", 0.0),
                );
            }
        }

        let Some(gfx) = self.gfx.as_mut() else { return };
        gfx.pipeline.set_audio(low, mid, high);
        let size = gfx.window.inner_size();
        let fc = FrameContext {
            time: now.duration_since(self.start).as_secs_f32(),
            dt,
            width: size.width.max(1),
            height: size.height.max(1),
            frame: self.frame,
        };

        gfx.pipeline.render(&fc, &self.engine);
        self.engine.end_frame();

        // Capture before presenting so we read the freshly drawn back buffer.
        let last_frame = self.max_frames.map(|m| self.frame + 1 >= m).unwrap_or(false);
        if last_frame {
            if let Some(path) = self.cli.screenshot.clone() {
                let pixels = gl::read_rgba(&gfx.gl, fc.width as i32, fc.height as i32);
                match super::save_ppm(&path, fc.width, fc.height, &pixels) {
                    Ok(()) => tracing::info!("wrote screenshot {path}"),
                    Err(e) => tracing::warn!("screenshot failed: {e}"),
                }
            }
        }

        if let Err(e) = gfx.surface.swap_buffers(&gfx.context) {
            tracing::warn!("swap_buffers failed: {e}");
        }

        self.frame += 1;
        if self.max_frames.map(|m| self.frame >= m).unwrap_or(false) {
            tracing::info!("frame budget reached ({} frames), exiting", self.frame);
            el.exit();
        }
    }
}

impl ApplicationHandler for WindowApp {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_none() {
            if let Err(e) = self.init(el) {
                tracing::error!("backend init failed: {e:#}");
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::KeyboardInput {
                event: KeyEvent { logical_key: Key::Named(NamedKey::Escape), state: ElementState::Pressed, .. },
                ..
            } => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
                        gfx.surface.resize(&gfx.context, w, h);
                        gfx.pipeline.resize(size.width, size.height);
                    }
                }
            }
            WindowEvent::RedrawRequested => self.draw(el),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        // Drive continuous animation; vsync (swap interval) paces it.
        if let Some(gfx) = self.gfx.as_ref() {
            gfx.window.request_redraw();
        }
    }
}

/// Best-effort GL_RENDERER string for the startup log.
fn renderer_name(gl: &Gl) -> String {
    use glow::HasContext;
    unsafe { gl.get_parameter_string(glow::RENDERER) }
}
