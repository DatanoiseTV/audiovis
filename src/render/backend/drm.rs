//! Linux DRM/KMS backend: render straight to a display with no X11/Wayland.
//!
//! This is the path for headless boards (Raspberry Pi, etc.) booted to a bare
//! console. The pieces:
//!
//! - **DRM/KMS** ([`drm`]) - open `/dev/dri/cardN`, find a connected connector,
//!   pick its mode, and bind a CRTC. Modeset + page flips go through the legacy
//!   API (no atomic), which every KMS driver supports.
//! - **GBM** ([`gbm`]) - a buffer allocator the GPU can scan out from. We make a
//!   GBM *surface* (a small swap-chain of scanout-capable buffers) and let EGL
//!   render into it.
//! - **EGL** (via [`glutin`]) - create the GL context + window surface on the
//!   GBM device/surface, then load `glow` from EGL like the window backend does.
//!
//! Frame loop: render -> `eglSwapBuffers` (queues a buffer on the GBM surface)
//! -> lock the front buffer -> wrap it in a DRM framebuffer -> page-flip the
//! CRTC to it -> block on the flip-complete event (this is the vsync). The
//! buffer shown by the *previous* flip is released back to the GBM pool only
//! after the new flip completes, so a buffer is never freed while scanning out.
//!
//! Everything between frames (control input, modulation, scripting, web state)
//! is shared with the window backend in [`super::driver::Driver`].

use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroU32;
use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use drm::control::{connector, crtc, framebuffer, Device as ControlDevice, Event, Mode, ModeTypeFlags, PageFlipFlags};
use drm::Device as DrmDevice;
use gbm::{AsRaw, BufferObject, BufferObjectFlags, Device as GbmDevice, Format, Surface as GbmSurface};
use glutin::config::{Api, Config, ConfigTemplateBuilder};
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, Version};
use glutin::display::{Display, DisplayApiPreference, GlDisplay};
use glutin::surface::{GlSurface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::{GbmDisplayHandle, GbmWindowHandle, RawDisplayHandle, RawWindowHandle};

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

/// The scanout pixel format. XRGB8888 (no alpha for scanout) is universally
/// supported by KMS drivers and is what GBM/EGL hand us by default.
const SCANOUT_FORMAT: Format = Format::Xrgb8888;

/// A DRM device node. Wrapping the fd in an `Arc` lets the same open device be
/// shared by KMS calls here and by the GBM allocator (which takes ownership of
/// something `AsFd`), without dup-ing the descriptor.
#[derive(Clone)]
struct Card(Arc<std::fs::File>);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

// `AsFd` is the only prerequisite for the DRM device traits.
impl DrmDevice for Card {}
impl ControlDevice for Card {}

impl Card {
    fn open(path: &str) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("open DRM device {path}"))?;
        Ok(Card(Arc::new(file)))
    }
}

/// The KMS objects we drove modeset onto, resolved at startup.
struct Display0 {
    connector: connector::Handle,
    crtc: crtc::Handle,
    mode: Mode,
    /// The CRTC's state before we touched it, restored on a clean exit so the
    /// console comes back instead of being left on our last frame.
    saved_crtc: crtc::Info,
}

/// Scan `/dev/dri/card*` for the first device with a connected connector, and
/// resolve a (connector, CRTC, mode) we can drive. Honours `AV_DRM_CARD` /
/// `card.dev` if the caller pinned a specific node.
fn open_display(forced: &str) -> Result<(Card, Display0)> {
    let candidates: Vec<String> = if !forced.is_empty() {
        vec![forced.to_string()]
    } else {
        // card0..card7 covers every realistic seat; nodes are sparse so we just
        // try them and skip the ones that don't exist.
        (0..8).map(|n| format!("/dev/dri/card{n}")).collect()
    };

    let mut last_err: Option<anyhow::Error> = None;
    for path in &candidates {
        if !std::path::Path::new(path).exists() {
            continue;
        }
        match try_open_display(path) {
            Ok(found) => return Ok(found),
            Err(e) => {
                tracing::debug!("{path}: {e:#}");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("no usable DRM device found (looked at {candidates:?})")))
}

fn try_open_display(path: &str) -> Result<(Card, Display0)> {
    let card = Card::open(path)?;
    let res = card.resource_handles().with_context(|| format!("{path}: not a KMS device"))?;

    // Find a connected connector with at least one mode.
    let connector = res
        .connectors()
        .iter()
        .filter_map(|&handle| card.get_connector(handle, true).ok())
        .filter(|info| info.state() == connector::State::Connected && !info.modes().is_empty())
        .max_by_key(|info| {
            // Prefer the connector whose best mode has the most pixels (drive
            // the biggest attached display when several are connected).
            best_mode(info).map(|m| m.size().0 as u32 * m.size().1 as u32).unwrap_or(0)
        })
        .ok_or_else(|| anyhow!("{path}: no connected connector with a mode"))?;

    let mode = best_mode(&connector).ok_or_else(|| anyhow!("{path}: connector has no modes"))?;

    // Resolve a CRTC: reuse the one already lit on this connector if any,
    // otherwise pick the first CRTC any of the connector's encoders can drive.
    let crtc = resolve_crtc(&card, &res, &connector)
        .ok_or_else(|| anyhow!("{path}: no CRTC available for connector"))?;

    let saved_crtc = card.get_crtc(crtc).with_context(|| format!("{path}: get_crtc"))?;

    let (w, h) = mode.size();
    tracing::info!(
        "drm: {path} connector {:?} {}x{}@{} on crtc {:?}",
        connector.interface(),
        w,
        h,
        mode.vrefresh(),
        crtc,
    );

    Ok((
        card,
        Display0 { connector: connector.handle(), crtc, mode, saved_crtc },
    ))
}

/// The connector's preferred mode, else its first (highest-res) listed mode.
fn best_mode(info: &connector::Info) -> Option<Mode> {
    info.modes()
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or_else(|| info.modes().first())
        .copied()
}

fn resolve_crtc(card: &Card, res: &drm::control::ResourceHandles, connector: &connector::Info) -> Option<crtc::Handle> {
    // Already-bound CRTC (connector currently lit, e.g. fbcon) - reuse it.
    if let Some(enc) = connector.current_encoder() {
        if let Ok(info) = card.get_encoder(enc) {
            if let Some(crtc) = info.crtc() {
                return Some(crtc);
            }
        }
    }
    // Otherwise the first CRTC any of the connector's encoders can drive.
    for &enc in connector.encoders() {
        if let Ok(info) = card.get_encoder(enc) {
            if let Some(&crtc) = res.filter_crtcs(info.possible_crtcs()).first() {
                return Some(crtc);
            }
        }
    }
    res.crtcs().first().copied()
}

/// Run the DRM backend. Blocks until the frame budget is reached (or forever).
pub fn run(
    cli: Cli,
    engine: Engine,
    bus: ControlBus,
    audio: AudioEngine,
    midi: MidiInputs,
    video: VideoEngine,
    web: Option<WebHandle>,
) -> Result<()> {
    let forced = std::env::var("AV_DRM_CARD").unwrap_or_default();
    let (card, disp) = open_display(&forced)?;
    let (mode_w, mode_h) = mode_dims(&disp.mode);

    // GBM allocator on the same device. The surface is a small swap chain of
    // scanout-capable render buffers EGL will draw into.
    let gbm = GbmDevice::new(card.clone()).context("create GBM device")?;
    let gbm_surface: GbmSurface<()> = gbm
        .create_surface::<()>(mode_w, mode_h, SCANOUT_FORMAT, BufferObjectFlags::SCANOUT | BufferObjectFlags::RENDERING)
        .context("create GBM surface")?;

    // EGL display + GLES2 context + window surface on the GBM objects, via
    // glutin (same GL loader path as the window backend).
    let gl_display = unsafe {
        let handle = RawDisplayHandle::Gbm(GbmDisplayHandle::new(
            NonNull::new(gbm.as_raw() as *mut c_void).ok_or_else(|| anyhow!("null gbm device"))?,
        ));
        Display::new(handle, DisplayApiPreference::Egl).context("create EGL display")?
    };

    let raw_window = RawWindowHandle::Gbm(GbmWindowHandle::new(
        NonNull::new(gbm_surface.as_raw() as *mut c_void).ok_or_else(|| anyhow!("null gbm surface"))?,
    ));

    // Match a config that is GLES2-renderable and whose native visual is our
    // scanout fourcc - on the GBM platform EGL_NATIVE_VISUAL_ID *is* the DRM
    // format, and a mismatch makes surface creation fail with EGL_BAD_MATCH.
    let template = ConfigTemplateBuilder::new().with_api(Api::GLES2).with_alpha_size(0).build();
    let want_visual = SCANOUT_FORMAT as u32;
    let gl_config = unsafe { gl_display.find_configs(template) }
        .context("enumerate EGL configs")?
        .reduce(|acc, cfg| {
            // Prefer a config whose native visual is our scanout fourcc; among
            // equals keep the first.
            if config_visual(&cfg) == Some(want_visual) && config_visual(&acc) != Some(want_visual) {
                cfg
            } else {
                acc
            }
        })
        .ok_or_else(|| anyhow!("no EGL config found"))?;
    if config_visual(&gl_config) != Some(want_visual) {
        tracing::warn!("no EGL config matched scanout visual {want_visual:#x}; surface creation may fail");
    }

    // GLES2 to match the shader flavour the boards run (and what this backend
    // targets). The macro layer keeps the shader bodies identical to desktop.
    let ctx_attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
        .build(Some(raw_window));
    let not_current = unsafe { gl_display.create_context(&gl_config, &ctx_attrs) }
        .context("create GLES2 context")?;

    let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        raw_window,
        NonZeroU32::new(mode_w).unwrap(),
        NonZeroU32::new(mode_h).unwrap(),
    );
    let gl_surface = unsafe { gl_display.create_window_surface(&gl_config, &surface_attrs) }
        .context("create EGL window surface")?;

    let context = not_current.make_current(&gl_surface).context("make EGL context current")?;

    // GBM swap doesn't block; the page-flip event is our vsync, so don't ask
    // EGL to also wait.
    let _ = gl_surface.set_swap_interval(&context, SwapInterval::DontWait);

    let gl: Gl = Rc::new(unsafe {
        glow::Context::from_loader_function_cstr(|s| gl_display.get_proc_address(s).cast())
    });

    let mut driver = Driver::new(cli, engine, bus, audio, midi, video, web);
    let mut pipeline = Pipeline::new(&gl, GlslFlavor::Es2, &mut driver.engine, mode_w, mode_h, driver.cli.render_scale)
        .map_err(|e| anyhow!("pipeline init failed: {e}"))?;
    driver.on_pipeline_ready(&pipeline);

    tracing::info!("drm backend up: {}x{} GLES2 via {}", mode_w, mode_h, renderer_name(&gl));

    // The presenter owns the GBM/KMS plumbing and the per-bo framebuffer cache.
    let mut presenter = Presenter {
        card: &card,
        crtc: disp.crtc,
        connector: disp.connector,
        mode: disp.mode,
        gbm_surface: &gbm_surface,
        fb_cache: HashMap::new(),
        displayed: None,
        first: true,
    };

    let frame_interval = Duration::from_secs_f32(1.0 / driver.fps() as f32);
    let result = (|| -> Result<()> {
        loop {
            let frame_start = Instant::now();

            let (time, dt) = driver.prepare_frame(&mut pipeline);
            let fc = FrameContext { time, dt, width: mode_w, height: mode_h, frame: driver.frame() };
            pipeline.render(&fc, &driver.engine);
            driver.finish_frame(&mut pipeline);

            if let Some(path) = driver.screenshot_path() {
                let pixels = gl::read_rgba(&gl, mode_w as i32, mode_h as i32);
                match super::save_ppm(&path, mode_w, mode_h, &pixels) {
                    Ok(()) => tracing::info!("wrote screenshot {path}"),
                    Err(e) => tracing::warn!("screenshot failed: {e}"),
                }
            }

            gl_surface.swap_buffers(&context).context("eglSwapBuffers")?;
            presenter.present()?;

            if driver.advance_frame() {
                break;
            }

            // The page-flip wait paces us to the panel's refresh; if the target
            // fps is lower (e.g. 24/30 on a 60 Hz panel) sleep the remainder.
            if let Some(rem) = frame_interval.checked_sub(frame_start.elapsed()) {
                std::thread::sleep(rem);
            }
        }
        Ok(())
    })();

    // Restore the console's CRTC config so we don't leave the panel frozen on
    // our last frame; drop the presenter first to free our framebuffers.
    let saved = disp.saved_crtc;
    drop(presenter);
    if let Err(e) = card.set_crtc(
        disp.crtc,
        saved.framebuffer(),
        saved.position(),
        &[disp.connector],
        saved.mode(),
    ) {
        tracing::debug!("could not restore original CRTC: {e}");
    }

    result
}

fn mode_dims(mode: &Mode) -> (u32, u32) {
    let (w, h) = mode.size();
    (w as u32, h as u32)
}

/// `EGL_NATIVE_VISUAL_ID` of a config. On the GBM platform this is the DRM
/// format fourcc, which must match the scanout buffer's format. Only the EGL
/// backend exposes it (the only one this backend creates).
fn config_visual(config: &Config) -> Option<u32> {
    #[allow(unreachable_patterns)]
    match config {
        Config::Egl(egl) => Some(egl.native_visual()),
        _ => None,
    }
}

/// Drives KMS presentation for one CRTC: turns each freshly-rendered GBM front
/// buffer into a scanout via modeset (first frame) then page flips.
struct Presenter<'a> {
    card: &'a Card,
    crtc: crtc::Handle,
    connector: connector::Handle,
    mode: Mode,
    gbm_surface: &'a GbmSurface<()>,
    /// DRM framebuffer per distinct GBM buffer (keyed by GEM handle). The GBM
    /// pool recycles a handful of buffers, so this stays tiny and we never
    /// re-wrap the same buffer twice.
    fb_cache: HashMap<u32, framebuffer::Handle>,
    /// The buffer currently being scanned out - held so it isn't released back
    /// to the pool (and reused) while the display is reading from it.
    displayed: Option<BufferObject<()>>,
    first: bool,
}

impl Presenter<'_> {
    /// Lock the just-swapped front buffer, bind it to the CRTC, and wait for the
    /// flip to complete. Must be called exactly once per `eglSwapBuffers`.
    fn present(&mut self) -> Result<()> {
        if !self.gbm_surface.has_free_buffers() {
            // Should not happen with our release-after-flip discipline, but if
            // the pool is exhausted, locking would return null below.
            tracing::warn!("GBM surface has no free buffers");
        }

        // SAFETY: called exactly once after the eglSwapBuffers in the run loop.
        let bo = unsafe { self.gbm_surface.lock_front_buffer() }
            .map_err(|e| anyhow!("lock GBM front buffer: {e}"))?;
        let fb = self.framebuffer_for(&bo)?;

        if self.first {
            // First frame establishes the mode; this lights the display.
            self.card
                .set_crtc(self.crtc, Some(fb), (0, 0), &[self.connector], Some(self.mode))
                .context("set_crtc")?;
            self.first = false;
        } else {
            // Queue a flip and block on the completion event = vsync.
            match self.card.page_flip(self.crtc, fb, PageFlipFlags::EVENT, None) {
                Ok(()) => self.wait_for_flip()?,
                Err(e) => {
                    // Some drivers reject a flip right after modeset; fall back
                    // to a blocking modeset so we still present this frame.
                    tracing::debug!("page_flip failed ({e}); using set_crtc");
                    self.card
                        .set_crtc(self.crtc, Some(fb), (0, 0), &[self.connector], Some(self.mode))
                        .context("set_crtc fallback")?;
                }
            }
            // The flip is done: the buffer shown before it is now free to reuse.
            // Dropping it releases it back to the GBM pool.
            self.displayed = None;
        }

        self.displayed = Some(bo);
        Ok(())
    }

    /// DRM framebuffer wrapping this GBM buffer, created once per buffer.
    fn framebuffer_for(&mut self, bo: &BufferObject<()>) -> Result<framebuffer::Handle> {
        // SAFETY: a scanout GBM buffer always has a valid GEM handle here.
        let key = unsafe { bo.handle().u32_ };
        if let Some(&fb) = self.fb_cache.get(&key) {
            return Ok(fb);
        }
        // XRGB8888: depth 24, 32 bpp.
        let fb = self.card.add_framebuffer(bo, 24, 32).context("add_framebuffer")?;
        self.fb_cache.insert(key, fb);
        Ok(fb)
    }

    /// Block until the page-flip-complete event for our CRTC arrives. A 1 s poll
    /// acts as a watchdog: a successfully-queued flip is guaranteed an event at
    /// the next vblank, so a timeout means the CRTC is wedged - we log and keep
    /// waiting rather than racing ahead and queueing a flip the kernel rejects.
    fn wait_for_flip(&self) -> Result<()> {
        let raw = self.card.as_fd().as_raw_fd();
        loop {
            let mut pfd = libc::pollfd { fd: raw, events: libc::POLLIN, revents: 0 };
            let r = unsafe { libc::poll(&mut pfd, 1, 1000) };
            if r < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(anyhow!("poll DRM fd: {err}"));
            }
            if r == 0 {
                tracing::warn!("page-flip event not received within 1s; still waiting");
                continue;
            }
            for event in self.card.receive_events().context("receive DRM events")? {
                if let Event::PageFlip(flip) = event {
                    if flip.crtc == self.crtc {
                        return Ok(());
                    }
                }
            }
        }
    }
}

impl Drop for Presenter<'_> {
    fn drop(&mut self) {
        // Release the displayed buffer before its framebuffer, then tear down
        // every framebuffer we created.
        self.displayed = None;
        for (_, fb) in self.fb_cache.drain() {
            let _ = self.card.destroy_framebuffer(fb);
        }
    }
}

/// Best-effort GL_RENDERER string for the startup log.
fn renderer_name(gl: &Gl) -> String {
    use glow::HasContext;
    unsafe { gl.get_parameter_string(glow::RENDERER) }
}
