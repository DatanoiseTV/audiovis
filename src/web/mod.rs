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
    mod_sources: Vec<String>,
    mod_routes: Vec<proto::ModRoute>,
    presets: Vec<String>,
    current_preset: String,
    mappings: Vec<proto::MidiMap>,
    media: Vec<String>,
    audio_devices: Vec<String>,
    audio_device: String,
    midi_ports: Vec<String>,
    midi_port: String,
    camera_devices: Vec<String>,
    camera_device: String,
    scripts: Vec<String>,
    meshes: Vec<String>,
    isf_shaders: Vec<String>,
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
    pub fn set_schema(&self, engine: &Engine, generators: Vec<String>, media: Vec<String>, meshes: Vec<String>, isf_shaders: Vec<String>) {
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
        let (schema_c, generators_c, changes_c, media_c, meshes_c, isf_c);
        let mod_sources: Vec<String> = crate::params::MOD_SOURCES.iter().map(|s| s.to_string()).collect();
        let sources_c;
        if let Ok(mut s) = self.snapshot.write() {
            s.schema = schema;
            s.generators = generators;
            s.values = values;
            s.mod_sources = mod_sources;
            s.media = media;
            s.meshes = meshes;
            s.isf_shaders = isf_shaders;
            schema_c = s.schema.clone();
            generators_c = s.generators.clone();
            changes_c = s.values.values().cloned().collect();
            sources_c = s.mod_sources.clone();
            media_c = s.media.clone();
            meshes_c = s.meshes.clone();
            isf_c = s.isf_shaders.clone();
        } else {
            return;
        }
        // Broadcast to any client that connected before the schema was ready,
        // so it gets its controls without needing to reconnect.
        let msg = proto::ServerMsg {
            schema: schema_c,
            generators: generators_c,
            changes: changes_c,
            mod_sources: sources_c,
            media: media_c,
            meshes: meshes_c,
            isf_shaders: isf_c,
            isf_present: true,
            ..Default::default()
        };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Publish the lettering slots so the UI's text fields populate.
    pub fn publish_text(&self, slots: &[String]) {
        let text: Vec<proto::TextSlot> =
            slots.iter().enumerate().map(|(i, t)| proto::TextSlot { id: i as i32, text: t.clone() }).collect();
        if let Ok(mut s) = self.snapshot.write() {
            if s.text == text {
                return;
            }
            s.text = text.clone();
        }
        let msg = proto::ServerMsg { text, ..Default::default() };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Publish the available + selected audio/MIDI/camera input devices.
    #[allow(clippy::too_many_arguments)]
    pub fn publish_devices(
        &self,
        audio_devices: Vec<String>,
        audio_device: &str,
        midi_ports: Vec<String>,
        midi_port: &str,
        camera_devices: Vec<String>,
        camera_device: &str,
    ) {
        if let Ok(mut s) = self.snapshot.write() {
            s.audio_devices = audio_devices.clone();
            s.audio_device = audio_device.to_string();
            s.midi_ports = midi_ports.clone();
            s.midi_port = midi_port.to_string();
            s.camera_devices = camera_devices.clone();
            s.camera_device = camera_device.to_string();
        }
        let msg = proto::ServerMsg {
            audio_devices,
            audio_device: audio_device.to_string(),
            midi_ports,
            midi_port: midi_port.to_string(),
            camera_devices,
            camera_device: camera_device.to_string(),
            devices_present: true,
            ..Default::default()
        };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Publish the media + mesh + ISF file labels (after a rescan).
    pub fn publish_media(&self, media: Vec<String>, meshes: Vec<String>, isf_shaders: Vec<String>) {
        if let Ok(mut s) = self.snapshot.write() {
            s.media = media.clone();
            s.meshes = meshes.clone();
            s.isf_shaders = isf_shaders.clone();
        }
        let msg = proto::ServerMsg { media, meshes, isf_shaders, isf_present: true, ..Default::default() };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Publish the selected ISF shader's inputs + any compile error.
    pub fn publish_isf_inputs(&self, inputs: Vec<(String, String, usize)>, error: &str) {
        let isf_inputs: Vec<proto::IsfInput> =
            inputs.into_iter().map(|(label, kind, slot)| proto::IsfInput { label, kind, slot: slot as i32 }).collect();
        let msg = proto::ServerMsg {
            isf_inputs,
            isf_inputs_present: true,
            isf_error: error.to_string(),
            ..Default::default()
        };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Publish the list of available script names.
    pub fn publish_scripts(&self, scripts: Vec<String>) {
        if let Ok(mut s) = self.snapshot.write() {
            s.scripts = scripts.clone();
        }
        let msg = proto::ServerMsg { scripts, script_present: true, ..Default::default() };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Push a script's source into the editor (after a load).
    pub fn publish_script_source(&self, source: &str) {
        let msg = proto::ServerMsg { script: source.to_string(), script_present: true, ..Default::default() };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Report a script compile/runtime error ("" clears it).
    pub fn publish_script_error(&self, error: &str) {
        let msg =
            proto::ServerMsg { script_error: error.to_string(), script_error_present: true, ..Default::default() };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Publish a JPEG preview frame for the web monitor (transient, not stored).
    pub fn publish_preview(&self, jpeg: Vec<u8>) {
        let msg = proto::ServerMsg { preview: jpeg, ..Default::default() };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Publish the active MIDI/OSC bindings for the mapping list.
    pub fn publish_mappings(&self, list: Vec<(String, String, String)>) {
        let maps: Vec<proto::MidiMap> =
            list.into_iter().map(|(source, target, mode)| proto::MidiMap { source, target, mode }).collect();
        if let Ok(mut s) = self.snapshot.write() {
            if s.mappings == maps {
                return;
            }
            s.mappings = maps.clone();
        }
        let msg = proto::ServerMsg { mappings: maps, mappings_present: true, ..Default::default() };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Publish the preset list and the currently-loaded preset name.
    pub fn publish_presets(&self, names: Vec<String>, current: &str) {
        if let Ok(mut s) = self.snapshot.write() {
            s.presets = names.clone();
            s.current_preset = current.to_string();
        }
        let msg = proto::ServerMsg { presets: names, current_preset: current.to_string(), ..Default::default() };
        let _ = self.out.send(msg.encode_to_vec());
    }

    /// Publish the current modulation routes (call at the telemetry rate; cheap
    /// and keeps every client's matrix view in sync).
    pub fn publish_mod_routes(&self, engine: &Engine) {
        let routes: Vec<proto::ModRoute> = engine
            .modmatrix()
            .routes()
            .iter()
            .map(|r| proto::ModRoute { source: r.source.clone(), target: r.target.clone(), amount: r.amount, smooth: r.smooth })
            .collect();
        if let Ok(mut s) = self.snapshot.write() {
            if s.mod_routes == routes {
                return; // unchanged, skip the broadcast
            }
            s.mod_routes = routes.clone();
        }
        let msg = proto::ServerMsg { mod_routes: routes, mod_routes_present: true, ..Default::default() };
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
    pub fn publish_telemetry(&self, low: f32, mid: f32, high: f32, rms: f32, beat: f32, bpm: f32, beat_phase: f32, bar_phase: f32, beats: f32) {
        let t = proto::Telemetry { low, mid, high, rms, beat, bpm, beat_phase, bar_phase, beats };
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
        if set.release {
            out.push(ControlEvent::Release { path: set.path });
        } else if set.trigger {
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
    if let Some(m) = msg.r#mod {
        out.push(ControlEvent::SetModRoute { source: m.source, target: m.target, amount: m.amount, smooth: m.smooth });
    }
    if let Some(t) = msg.text {
        out.push(ControlEvent::SetText { slot: t.id as u32, text: t.text });
    }
    if let Some(d) = msg.device {
        match d.kind.as_str() {
            "audio" => out.push(ControlEvent::SetAudioDevice(d.name)),
            "midi" => out.push(ControlEvent::SetMidiPort(d.name)),
            "camera" => out.push(ControlEvent::SetVideoDevice(d.name)),
            _ => {}
        }
    }
    if msg.rescan_media {
        out.push(ControlEvent::RescanMedia);
    }
    if let Some(sc) = msg.script {
        match sc.action.as_str() {
            "apply" => out.push(ControlEvent::SetScript(sc.source)),
            "save" => out.push(ControlEvent::SaveScript { name: sc.name, source: sc.source }),
            "load" => out.push(ControlEvent::LoadScript(sc.name)),
            _ => {}
        }
    }
    out
}
