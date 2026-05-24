//! MIDI input.
//!
//! Opens a virtual input port named "audiovis" (so other apps - Ableton, a
//! sequencer - can send notes/CC/clock straight to us) and connects to matching
//! hardware ports. Every message is parsed to a [`ControlEvent`] and pushed onto
//! the control bus. Note/CC go through the mapping matrix; clock and transport
//! drive the beat clock.

use crossbeam_channel::Sender;
use midir::{MidiInput, MidiInputConnection};

use super::{ControlEvent, Transport};

const CLIENT: &str = "audiovis";
const VIRTUAL_PORT: &str = "audiovis";

/// Holds the open connections alive; dropping it closes them.
pub struct MidiInputs {
    _conns: Vec<MidiInputConnection<()>>,
}

impl MidiInputs {
    /// Open the virtual port plus hardware ports. `filter` empty connects to all
    /// hardware inputs; otherwise only ports whose name contains `filter`.
    pub fn start(filter: &str, tx: Sender<ControlEvent>) -> Self {
        let mut conns = Vec::new();

        match open_virtual(tx.clone()) {
            Ok(c) => {
                tracing::info!("MIDI virtual input port '{VIRTUAL_PORT}' open");
                conns.push(c);
            }
            Err(e) => tracing::warn!("could not open virtual MIDI port: {e}"),
        }

        match connect_hardware(filter, &tx) {
            Ok(mut hw) => conns.append(&mut hw),
            Err(e) => tracing::warn!("MIDI hardware scan failed: {e}"),
        }

        if conns.is_empty() {
            tracing::warn!("no MIDI inputs available");
        }
        MidiInputs { _conns: conns }
    }
}

fn open_virtual(tx: Sender<ControlEvent>) -> anyhow::Result<MidiInputConnection<()>> {
    use midir::os::unix::VirtualInput;
    let input = MidiInput::new(CLIENT)?;
    let conn = input
        .create_virtual(VIRTUAL_PORT, move |_t, msg, _| dispatch(msg, &tx), ())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(conn)
}

fn connect_hardware(filter: &str, tx: &Sender<ControlEvent>) -> anyhow::Result<Vec<MidiInputConnection<()>>> {
    // A throwaway input just to enumerate ports.
    let scan = MidiInput::new(CLIENT)?;
    let mut conns = Vec::new();
    for port in scan.ports() {
        let name = scan.port_name(&port).unwrap_or_default();
        // Skip our own virtual port to avoid a feedback loop.
        if name.contains(VIRTUAL_PORT) {
            continue;
        }
        if !filter.is_empty() && !name.contains(filter) {
            continue;
        }
        // Each connection needs its own MidiInput instance.
        let input = MidiInput::new(CLIENT)?;
        let tx = tx.clone();
        match input.connect(&port, "audiovis-in", move |_t, msg, _| dispatch(msg, &tx), ()) {
            Ok(c) => {
                tracing::info!("MIDI connected: {name}");
                conns.push(c);
            }
            Err(e) => tracing::warn!("MIDI connect '{name}' failed: {e}"),
        }
    }
    Ok(conns)
}

/// Parse one raw MIDI message and forward it as a control event.
fn dispatch(msg: &[u8], tx: &Sender<ControlEvent>) {
    if msg.is_empty() {
        return;
    }
    let status = msg[0];

    // System Real-Time messages are single status bytes (>= 0xF8).
    let ev = match status {
        0xF8 => Some(ControlEvent::MidiClock),
        0xFA => Some(ControlEvent::Transport(Transport::Start)),
        0xFB => Some(ControlEvent::Transport(Transport::Continue)),
        0xFC => Some(ControlEvent::Transport(Transport::Stop)),
        _ => channel_message(status, msg),
    };
    if let Some(ev) = ev {
        let _ = tx.send(ev);
    }
}

fn channel_message(status: u8, msg: &[u8]) -> Option<ControlEvent> {
    let channel = status & 0x0F;
    match status & 0xF0 {
        0x90 if msg.len() >= 3 => {
            let velocity = msg[2];
            // A note-on with zero velocity is the common note-off encoding.
            Some(ControlEvent::MidiNote { channel, note: msg[1], velocity, on: velocity > 0 })
        }
        0x80 if msg.len() >= 3 => {
            Some(ControlEvent::MidiNote { channel, note: msg[1], velocity: msg[2], on: false })
        }
        0xB0 if msg.len() >= 3 => Some(ControlEvent::MidiCc { channel, cc: msg[1], value: msg[2] }),
        _ => None,
    }
}
