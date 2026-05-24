//! OSC input over UDP.
//!
//! A listener thread receives datagrams, decodes them with `rosc`, and forwards
//! each message to the control bus. Two conventions:
//!
//! - An address beginning with `/p/` sets a parameter directly by path, e.g.
//!   `/p/layer.0.opacity 0.8`. This is the easy path for TouchOSC-style layouts.
//! - Any other address is forwarded raw, so it can be bound through the mapping
//!   matrix / learn just like a MIDI control.

use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::Sender;
use rosc::{OscMessage, OscPacket, OscType};

use super::ControlEvent;

/// Owns the listener thread; dropping it stops the listener.
pub struct OscInput {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl OscInput {
    /// Bind `addr` (e.g. `0.0.0.0:9000`) and start listening.
    pub fn start(addr: &str, tx: Sender<ControlEvent>) -> anyhow::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;
        tracing::info!("OSC listening on {addr} (use /p/<param.path> <value> to set directly)");

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let handle = std::thread::Builder::new()
            .name("osc-input".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                while !stop_thread.load(Ordering::Relaxed) {
                    match socket.recv_from(&mut buf) {
                        Ok((n, _)) => {
                            if let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..n]) {
                                handle_packet(packet, &tx);
                            }
                        }
                        // Timeout just loops so we can observe the stop flag.
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
                        Err(e) => {
                            tracing::warn!("OSC recv error: {e}");
                            break;
                        }
                    }
                }
            })?;

        Ok(OscInput { stop, handle: Some(handle) })
    }
}

impl Drop for OscInput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn handle_packet(packet: OscPacket, tx: &Sender<ControlEvent>) {
    match packet {
        OscPacket::Message(m) => handle_message(m, tx),
        OscPacket::Bundle(b) => {
            for p in b.content {
                handle_packet(p, tx);
            }
        }
    }
}

fn handle_message(m: OscMessage, tx: &Sender<ControlEvent>) {
    let value = m.args.iter().find_map(as_f32).unwrap_or(0.0);

    if let Some(path) = m.addr.strip_prefix("/p/") {
        // Direct parameter set by normalised value.
        let _ = tx.send(ControlEvent::SetParamNorm { path: path.to_string(), norm: value });
    } else {
        let _ = tx.send(ControlEvent::Osc { addr: m.addr, value, pressed: value > 0.5 });
    }
}

/// Best-effort numeric extraction from an OSC argument.
fn as_f32(t: &OscType) -> Option<f32> {
    match t {
        OscType::Float(v) => Some(*v),
        OscType::Double(v) => Some(*v as f32),
        OscType::Int(v) => Some(*v as f32),
        OscType::Long(v) => Some(*v as f32),
        OscType::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}
