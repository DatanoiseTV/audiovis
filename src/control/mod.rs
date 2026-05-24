//! The control bus.
//!
//! Every control source - MIDI, OSC, the web UI - is a producer of
//! [`ControlEvent`]s. They all funnel into a single multi-producer channel that
//! the engine drains once per frame. This keeps the authoritative state on one
//! thread while letting input run on its own threads/async tasks.
//!
//! The MIDI and OSC producers live in their own modules, wired up in a later
//! milestone; they push the raw `Midi*`/`Osc` variants below onto this bus.

use crossbeam_channel::{Receiver, Sender};

use crate::params::{MapMode, ParamValue};

/// A single control message headed for the engine.
#[derive(Debug, Clone)]
pub enum ControlEvent {
    // --- Raw inputs (pass through the mapping matrix / learn) ---
    MidiCc { channel: u8, cc: u8, value: u8 },
    MidiNote { channel: u8, note: u8, velocity: u8, on: bool },
    /// A single-value OSC message. `pressed` distinguishes button-style presses
    /// (value > 0) for toggle/trigger bindings.
    Osc { addr: String, value: f32, pressed: bool },

    // --- Direct parameter control (the web UI already knows the target) ---
    SetParam { path: String, value: ParamValue },
    SetParamNorm { path: String, norm: f32 },
    Trigger { path: String },

    // --- Mapping / learn management ---
    Arm { path: String, mode: MapMode },
    Disarm,
    ClearMappingsFor { path: String },

    // --- Transport / presets ---
    LoadPreset(String),
    SavePreset(String),
}

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
