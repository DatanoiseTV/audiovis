//! Application wiring: pick an output backend and hand it the engine + control
//! bus. Backend selection follows `--backend`, defaulting per platform.

use anyhow::Result;

use crate::audio::AudioEngine;
use crate::cli::{Backend, Cli};
use crate::control::midi::MidiInputs;
use crate::control::osc::OscInput;
use crate::control::ControlBus;
use crate::engine::Engine;
use crate::render::backend;
use crate::web::WebHandle;

/// Resolve `auto` to a concrete backend for the current platform.
fn resolve(backend: Backend) -> Backend {
    match backend {
        Backend::Auto => {
            // The DRM backend only exists on Linux; everywhere else (and on a
            // Linux desktop session) the window backend is the sane default.
            Backend::Window
        }
        other => other,
    }
}

/// Run the application to completion.
pub fn run(cli: Cli, engine: Engine, bus: ControlBus) -> Result<()> {
    // Start audio capture before the render loop and keep it alive for the
    // whole run; the renderer reads the shared feature block each frame.
    let audio = AudioEngine::start(&cli.audio_device, cli.audio_gain);
    let audio_shared = audio.shared();

    // Start control inputs. Each holds a sender onto the bus and stays alive
    // for the run (the bindings own the OS connections/threads).
    let _midi = MidiInputs::start(&cli.midi_port, bus.sender());
    let _osc = if cli.osc_listen.is_empty() {
        None
    } else {
        match OscInput::start(&cli.osc_listen, bus.sender()) {
            Ok(o) => Some(o),
            Err(e) => {
                tracing::warn!("OSC disabled: {e}");
                None
            }
        }
    };

    // Start the web control surface unless disabled.
    let web = if cli.web_listen.is_empty() {
        None
    } else {
        WebHandle::start(&cli.web_listen, bus.sender())
    };

    match resolve(cli.backend) {
        Backend::Window | Backend::Auto => backend::window::run(cli, engine, bus, audio_shared, web),
        Backend::Drm => {
            // Wired up in the Linux DRM/KMS milestone; until then, be explicit
            // rather than silently falling back.
            anyhow::bail!("the drm backend is not yet built; use --backend window")
        }
    }
}
