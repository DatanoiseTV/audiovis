//! Control events: the command set the engine consumes.
//!
//! These are pure data types so they sit on the render-core side and can travel
//! across any transport - the native `ControlBus` (crossbeam channel), the web
//! socket, MIDI/OSC handlers in the bin, or the wasm renderer driving its own
//! mirrored engine state.

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
    /// Release a gated/held target (web pointer-up, mirrors a note-off).
    Release { path: String },

    // --- Mapping / learn management ---
    Arm { path: String, mode: MapMode },
    Disarm,
    ClearMappingsFor { path: String },

    // --- Modulation matrix ---
    /// Add/update (amount != 0) or remove (amount == 0) a modulation route.
    SetModRoute { source: String, target: String, amount: f32, smooth: f32 },

    // --- Lettering bank ---
    /// Set the text of a lettering slot.
    SetText { slot: u32, text: String },

    // --- Transport / presets ---
    LoadPreset(String),
    SavePreset(String),

    // --- I/O device selection (handled by the window backend) ---
    /// Switch the audio input device (empty selects the system default).
    SetAudioDevice(String),
    /// Switch the MIDI hardware input filter (empty connects to all ports).
    SetMidiPort(String),
    /// Switch the camera/video input device (empty = first available).
    SetVideoDevice(String),
    /// Enable/disable Ableton Link tempo sync.
    SetLink(bool),
    /// Re-scan the media directory for newly added image/SVG files.
    RescanMedia,

    // --- JS scripting (handled by the window backend) ---
    /// Compile + run a new script source live (without saving).
    SetScript(String),
    /// Save the source as a named script, then run it.
    SaveScript { name: String, source: String },
    /// Load a named script (builtin or user), run it and echo it to the UI.
    LoadScript(String),
}
