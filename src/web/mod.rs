//! The web control surface: an axum server with a websocket pub/sub channel
//! carrying protobuf messages, plus the embedded browser app.
//!
//! Threading: the server runs its own tokio runtime on a dedicated thread. The
//! render thread (which owns the engine) publishes state through [`WebHandle`]:
//! a `broadcast` channel fans encoded [`proto::ServerMsg`] deltas out to all
//! connected clients, and a shared snapshot lets a late-joining client get the
//! full current state on connect. Inbound client messages convert to
//! [`ControlEvent`]s and go onto the same control bus MIDI/OSC use.

mod server;

/// Prost types generated from `proto/control.proto` at build time.
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/audiovis.rs"));
}

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;

use crossbeam_channel::Sender;
use prost::Message;
use tokio::sync::broadcast;

use crate::control::ControlEvent;
use crate::engine::{Engine, EngineNotice};
use crate::params::{ParamKind, ParamValue};

/// The full current state, so a fresh client can be brought up to date at once.
#[derive(Default)]
struct Snapshot {
    schema: Vec<proto::ParamSpec>,
    generators: Vec<String>,
    values: HashMap<String, proto::ParamValue>,
    telemetry: proto::Telemetry,
    text: Vec<proto::TextSlot>,
}

/// Shared between the server tasks and the publisher.
#[derive(Clone)]
struct AppState {
    bus_tx: Sender<ControlEvent>,
    out: broadcast::Sender<Vec<u8>>,
    snapshot: Arc<RwLock<Snapshot>>,
}

/// Handle held by the render thread to publish state to the web UI.
pub struct WebHandle {
    out: broadcast::Sender<Vec<u8>>,
    snapshot: Arc<RwLock<Snapshot>>,
    _thread: JoinHandle<()>,
}

impl WebHandle {
    /// Start the server on `addr`. Returns `None` if the thread cannot spawn.
    pub fn start(addr: &str, bus_tx: Sender<ControlEvent>) -> Option<Self> {
        let (out, _) = broadcast::channel(512);
        let snapshot = Arc::new(RwLock::new(Snapshot::default()));
        let state = AppState { bus_tx, out: out.clone(), snapshot: snapshot.clone() };

        let addr = addr.to_string();
        let thread = std::thread::Builder::new()
            .name("web".into())
            .spawn(move || server::run(addr, state))
            .ok()?;

        Some(WebHandle { out, snapshot, _thread: thread })
    }

    /// Publish the parameter schema and the generator names once, after the
    /// pipeline has registered everything. Also seeds the initial value set.
    pub fn set_schema(&self, engine: &Engine, generators: Vec<String>) {
        let mut schema = Vec::new();
        let mut values = HashMap::new();
        for (id, spec, value) in engine.params().iter() {
            let (kind, min, max) = match spec.kind {
                ParamKind::Float { min, max, .. } => ("float", min, max),
                ParamKind::Int { min, max, .. } => ("int", min as f32, max as f32),
                ParamKind::Bool { .. } => ("bool", 0.0, 1.0),
                ParamKind::Trigger => ("trigger", 0.0, 1.0),
            };
            schema.push(proto::ParamSpec {
                path: spec.path.clone(),
                name: spec.name.clone(),
                group: spec.group.clone(),
                kind: kind.into(),
                min,
                max,
                unit: spec.unit.clone().unwrap_or_default(),
            });
            values.insert(
                spec.path.clone(),
                proto::ParamValue {
                    path: spec.path.clone(),
                    value: value.as_f32(),
                    norm: engine.params().normalized(id),
                },
            );
        }
        let (schema_c, generators_c, changes_c);
        if let Ok(mut s) = self.snapshot.write() {
            s.schema = schema;
            s.generators = generators;
            s.values = values;
            schema_c = s.schema.clone();
            generators_c = s.generators.clone();
            changes_c = s.values.values().cloned().collect();
        } else {
            return;
        }
        // Broadcast to any client that connected before the schema was ready,
        // so it gets its controls without needing to reconnect.
        let msg = proto::ServerMsg { schema: schema_c, generators: generators_c, changes: changes_c, ..Default::default() };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Forward engine notices (param changes) to clients as a delta.
    pub fn publish_notices(&self, notices: &[EngineNotice]) {
        let mut changes = Vec::new();
        for n in notices {
            if let EngineNotice::ParamChanged { path, value, norm } = n {
                changes.push(proto::ParamValue { path: path.clone(), value: value.as_f32(), norm: *norm });
            }
        }
        if changes.is_empty() {
            return;
        }
        if let Ok(mut s) = self.snapshot.write() {
            for c in &changes {
                s.values.insert(c.path.clone(), c.clone());
            }
        }
        let msg = proto::ServerMsg { changes, ..Default::default() };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Publish audio/clock telemetry (call at a modest rate, not every frame).
    #[allow(clippy::too_many_arguments)]
    pub fn publish_telemetry(&self, low: f32, mid: f32, high: f32, rms: f32, beat: f32, bpm: f32, beat_phase: f32) {
        let t = proto::Telemetry { low, mid, high, rms, beat, bpm, beat_phase };
        if let Ok(mut s) = self.snapshot.write() {
            s.telemetry = t.clone();
        }
        let msg = proto::ServerMsg { telemetry: Some(t), ..Default::default() };
        let _ = self.out.send(msg.encode_to_vec());
    }
}

/// Translate an inbound client message to control-bus events.
fn client_to_events(msg: proto::ClientMsg) -> Vec<ControlEvent> {
    let mut out = Vec::new();
    if let Some(set) = msg.set {
        if set.trigger {
            out.push(ControlEvent::Trigger { path: set.path });
        } else if set.is_norm {
            out.push(ControlEvent::SetParamNorm { path: set.path, norm: set.value });
        } else {
            out.push(ControlEvent::SetParam { path: set.path, value: ParamValue::Float(set.value) });
        }
    }
    if let Some(learn) = msg.learn {
        if learn.clear {
            out.push(ControlEvent::ClearMappingsFor { path: learn.path });
        } else if learn.arm {
            out.push(ControlEvent::Arm { path: learn.path, mode: crate::params::MapMode::Absolute });
        } else {
            out.push(ControlEvent::Disarm);
        }
    }
    if let Some(p) = msg.preset {
        match p.action.as_str() {
            "save" => out.push(ControlEvent::SavePreset(p.path)),
            "load" => out.push(ControlEvent::LoadPreset(p.path)),
            _ => {}
        }
    }
    // TextCmd is handled by the lettering bank milestone.
    out
}
