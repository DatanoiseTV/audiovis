//! Desktop window backend: winit for the window/event loop, glutin for the GL
//! context. We ask for OpenGL 3.3 Core so macOS hands us a context at all; the
//! shader macro layer keeps the bodies identical to the GLES2 boards.
//!
//! All the per-frame work (control input, modulation, scripting, web state) is
//! shared with the DRM backend in [`super::driver::Driver`]; this file is only
//! the window + event-loop + present plumbing.

use std::num::NonZeroU32;
use std::rc::Rc;

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
use winit::window::{Fullscreen, Window, WindowId};

use crate::audio::AudioEngine;
use crate::cli::Cli;
use crate::control::midi::MidiInputs;
use crate::control::ControlBus;
use crate::engine::Engine;
use crate::render::backend::driver::Driver;
use crate::render::gl::{self, Gl};
use crate::render::pipeline::Pipeline;
use crate::render::{FrameContext, GlslFlavor};
use crate::video::VideoEngine;
use crate::web::WebHandle;

/// Run the desktop window backend. Blocks until the window is closed (or the
/// frame budget set by `--frames`/`--screenshot` is reached).
pub fn run(
    cli: Cli,
    engine: Engine,
    bus: ControlBus,
    audio: AudioEngine,
    midi: MidiInputs,
    video: VideoEngine,
    web: Option<WebHandle>,
) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = WindowApp {
        driver: Driver::new(cli, engine, bus, audio, midi, video, web),
        gfx: None,
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
    driver: Driver,
    gfx: Option<Gfx>,
}

impl WindowApp {
    /// Build window, GL config, context, surface and pipeline. Runs on first
    /// resume (and again only if the platform tore the surface down).
    fn init(&mut self, el: &ActiveEventLoop) -> Result<()> {
        let attrs = Window::default_attributes()
            .with_title("audiovis")
            .with_inner_size(winit::dpi::LogicalSize::new(self.driver.cli.width, self.driver.cli.height));

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
        let pipeline = Pipeline::new(&gl, GlslFlavor::GlCore, &mut self.driver.engine, w, h, self.driver.cli.render_scale)
            .map_err(|e| anyhow!("pipeline init failed: {e}"))?;

        // The pipeline has now registered all layer/effect parameters, so the
        // startup preset can resolve their paths and the schema can be published.
        self.driver.on_pipeline_ready(&pipeline);

        tracing::info!("window backend up: {}x{} GL 3.3 Core via {}", w, h, renderer_name(&gl));

        self.gfx = Some(Gfx { window, surface, context, gl, pipeline });
        Ok(())
    }

    /// One rendered frame: pump shared state, draw, capture, present.
    fn draw(&mut self, el: &ActiveEventLoop) {
        let Some(gfx) = self.gfx.as_mut() else { return };

        let (time, dt) = self.driver.prepare_frame(&mut gfx.pipeline);
        let size = gfx.window.inner_size();
        let fc = FrameContext {
            time,
            dt,
            width: size.width.max(1),
            height: size.height.max(1),
            frame: self.driver.frame(),
        };

        gfx.pipeline.render(&fc, &self.driver.engine);
        self.driver.finish_frame(&mut gfx.pipeline);

        // Capture before presenting so we read the freshly drawn back buffer.
        if let Some(path) = self.driver.screenshot_path() {
            let pixels = gl::read_rgba(&gfx.gl, fc.width as i32, fc.height as i32);
            match super::save_ppm(&path, fc.width, fc.height, &pixels) {
                Ok(()) => tracing::info!("wrote screenshot {path}"),
                Err(e) => tracing::warn!("screenshot failed: {e}"),
            }
        }

        if let Err(e) = gfx.surface.swap_buffers(&gfx.context) {
            tracing::warn!("swap_buffers failed: {e}");
        }

        if self.driver.advance_frame() {
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
                event: KeyEvent { logical_key, state: ElementState::Pressed, .. },
                ..
            } => match logical_key {
                Key::Named(NamedKey::Escape) => el.exit(),
                // 'f' toggles borderless fullscreen on the monitor the window is
                // currently on, so it can be dragged to an external display first.
                Key::Character(s) if s.eq_ignore_ascii_case("f") => {
                    if let Some(gfx) = self.gfx.as_ref() {
                        let next = match gfx.window.fullscreen() {
                            Some(_) => None,
                            None => Some(Fullscreen::Borderless(None)),
                        };
                        gfx.window.set_fullscreen(next);
                    }
                }
                _ => {}
            },
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

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // Pace to the target fps. Only request a redraw once a frame is actually
        // due, then sleep until the next one - requesting every wake would wake
        // the loop immediately and defeat the WaitUntil, free-running at hundreds
        // of fps (wasted CPU, flooded web feed; bad on the Pi).
        let fps = self.driver.fps();
        let frame = std::time::Duration::from_secs_f32(1.0 / fps as f32);
        let next = self.driver.last_present() + frame;
        if std::time::Instant::now() >= next {
            if let Some(gfx) = self.gfx.as_ref() {
                gfx.window.request_redraw();
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(next));
    }
}

/// Best-effort GL_RENDERER string for the startup log.
fn renderer_name(gl: &Gl) -> String {
    use glow::HasContext;
    unsafe { gl.get_parameter_string(glow::RENDERER) }
}
