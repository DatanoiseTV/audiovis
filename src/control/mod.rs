//! The control bus.
//!
//! Every control source - MIDI, OSC, the web UI - is a producer of
//! [`ControlEvent`]s. They all funnel into a single multi-producer channel that
//! the engine drains once per frame. This keeps the authoritative state on one
//! thread while letting input run on its own threads/async tasks.
//!
//! The MIDI ([`midi`]) and OSC ([`osc`]) producers live in their own modules;
//! they push the raw `Midi*`/`Osc` variants below onto this bus.

pub mod midi;
pub mod osc;

use crossbeam_channel::{Receiver, Sender};

// `ControlEvent` + `Transport` are the engine's command set, defined in the
// render-core crate so the same types travel across any transport (this bus,
// the websocket, the in-browser wasm renderer driving its own engine).
pub use audiovis_render_core::events::{ControlEvent, Transport};

/// Owns the channel endpoints. Sources clone the [`Sender`]; the engine holds
/// the [`Receiver`].
pub struct ControlBus {
    tx: Sender<ControlEvent>,
    rx: Receiver<ControlEvent>,
}

impl Default for ControlBus {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlBus {
    pub fn new() -> Self {
        // Unbounded: control traffic is light, and we never want an input
        // thread to block the moment the engine is busy rendering a frame.
        let (tx, rx) = crossbeam_channel::unbounded();
        Self { tx, rx }
    }

    /// A cloneable handle for a control source to send on.
    pub fn sender(&self) -> Sender<ControlEvent> {
        self.tx.clone()
    }

    /// Non-blocking drain of everything queued since the last call.
    pub fn drain(&self) -> impl Iterator<Item = ControlEvent> + '_ {
        self.rx.try_iter()
    }
}
