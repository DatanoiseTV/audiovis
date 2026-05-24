//! Command-line interface.
//!
//! Every tunable here has a sane default and an `AV_*` environment fallback, so
//! nothing operational is hard-coded into the program logic.

use clap::Parser;

/// Live audio-reactive VJ visualizer (VHS / analog-video aesthetics).
#[derive(Debug, Clone, Parser)]
#[command(name = "audiovis", version = crate::version::long_static(), about)]
pub struct Cli {
    /// Output backend. `window` opens a desktop window; `drm` renders straight
    /// to a Linux framebuffer via KMS (no X11/Wayland); `auto` picks per platform.
    #[arg(long, env = "AV_BACKEND", default_value = "auto")]
    pub backend: Backend,

    /// Render width in pixels (the internal render target may be scaled down).
    #[arg(long, env = "AV_WIDTH", default_value_t = 1280)]
    pub width: u32,

    /// Render height in pixels.
    #[arg(long, env = "AV_HEIGHT", default_value_t = 720)]
    pub height: u32,

    /// Internal render scale (0.25..=1.0). Lower values trade detail for speed
    /// on weak GPUs; the result is upscaled to the output.
    #[arg(long, env = "AV_RENDER_SCALE", default_value_t = 1.0)]
    pub render_scale: f32,

    /// Target frames per second. The Pi Zero class is happiest around 24-30.
    #[arg(long, env = "AV_FPS", default_value_t = 60)]
    pub fps: u32,

    /// Audio input device name. Empty selects the system default.
    #[arg(long, env = "AV_AUDIO_DEVICE", default_value = "")]
    pub audio_device: String,

    /// MIDI input port name substring to auto-connect (empty = first available).
    #[arg(long, env = "AV_MIDI_PORT", default_value = "")]
    pub midi_port: String,

    /// UDP address to listen on for OSC control messages.
    #[arg(long, env = "AV_OSC_LISTEN", default_value = "0.0.0.0:9000")]
    pub osc_listen: String,

    /// TCP address for the web control UI. Empty disables the web server.
    #[arg(long, env = "AV_WEB_LISTEN", default_value = "0.0.0.0:8080")]
    pub web_listen: String,

    /// Optional preset file to load on startup.
    #[arg(long, env = "AV_PRESET")]
    pub preset: Option<String>,

    /// Render this many frames then exit. Useful for smoke tests and capture.
    /// Zero (the default) runs until closed.
    #[arg(long, env = "AV_FRAMES", default_value_t = 0)]
    pub frames: u64,

    /// Write a PPM screenshot of the final rendered frame to this path, then
    /// exit. Implies a short headless-style run.
    #[arg(long, env = "AV_SCREENSHOT")]
    pub screenshot: Option<String>,

    /// Log verbosity (`error`, `warn`, `info`, `debug`, `trace`). Overridable
    /// per-module via the standard `RUST_LOG` env var.
    #[arg(long, env = "AV_LOG", default_value = "info")]
    pub log: String,
}

/// Selectable output backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Backend {
    /// Choose automatically based on the platform and environment.
    Auto,
    /// Desktop window (winit + glutin).
    Window,
    /// Linux direct framebuffer via DRM/KMS + GBM + EGL.
    Drm,
}
