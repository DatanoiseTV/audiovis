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

use crate::params::{MapMode, ParamValue};

/// MIDI transport messages (System Real-Time), used to drive the beat clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Start,
    Stop,
    Continue,
}

/// A single control message headed for the engine.
#[derive(Debug, Clone)]
pub enum ControlEvent {
    // --- Raw inputs (pass through the mapping matrix / learn) ---
    MidiCc { channel: u8, cc: u8, value: u8 },
    MidiNote { channel: u8, note: u8, velocity: u8, on: bool },
    /// A single-value OSC message. `pressed` distinguishes button-style presses
    /// (value > 0) for toggle/trigger bindings.
    Osc { addr: String, value: f32, pressed: bool },
    /// One MIDI clock pulse (24 per quarter note); drives the beat clock.
    MidiClock,
    /// A MIDI transport message.
    Transport(Transport),

    // --- Direct parameter control (the web UI already knows the target) ---
    SetParam { path: String, value: ParamValue },
    SetParamNorm { path: String, norm: f32 },
    Trigger { path: String },

    // --- Mapping / learn management ---
    Arm { path: String, mode: MapMode },
    Disarm,
    ClearMappingsFor { path: String },

    // --- Modulation matrix ---
    /// Add/update (amount != 0) or remove (amount == 0) a modulation route.
    SetModRoute { source: String, target: String, amount: f32, smooth: f32 },

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
