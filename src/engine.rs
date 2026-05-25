//! The engine core: the single owner of authoritative visual state.
//!
//! It holds the parameter store and the mapping table, and turns the stream of
//! [`ControlEvent`]s into parameter changes. It records outbound [`EngineNotice`]s
//! so the web UI can stay in sync, and exposes per-frame trigger handling.
//!
//! Rendering is layered on top of this in later milestones; the engine itself is
//! deliberately render-agnostic and fully unit-testable on its own.

use std::time::Instant;

use std::collections::HashMap;

use crate::config::Preset;
use crate::control::{ControlEvent, Transport};
use crate::params::{MapAction, Mapping, ModMatrix, ParamKind, ParamStore, ParamValue, SourceKey};

/// Musical cycle lengths in beats, used by the tempo-synced LFOs. Every value
/// divides [`BEATS_WRAP`] so LFO phase stays continuous across the wrap.
pub const LFO_DIVISIONS: &[f32] = &[32.0, 16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.25];
/// Human labels matching `LFO_DIVISIONS`, surfaced in the UI.
pub const LFO_DIVISION_LABELS: &[&str] = &["8 bars", "4 bars", "2 bars", "1 bar", "1/2", "1/4", "1/8", "1/16"];
/// The musical position wraps here (16 bars) to keep `f32` phase precise.
const BEATS_WRAP: f64 = 64.0;

/// Beat clock with a free-running musical position (in beats). It advances by
/// `clock.bpm` every frame so tempo-synced LFOs move even with no external
/// clock; incoming MIDI clock (24 ppqn) re-derives the tempo and resyncs the
/// position, so a sequencer or DJ rig locks it tight.
struct BeatClock {
    last_pulse: Option<Instant>,
    /// Smoothed seconds between MIDI pulses.
    pulse_dt: f32,
    pulses: u64,
    running: bool,
    /// Free-running position in beats, wrapped at [`BEATS_WRAP`].
    beats: f64,
}

impl BeatClock {
    fn new() -> Self {
        // Default to free-running so LFOs animate out of the box.
        Self { last_pulse: None, pulse_dt: 0.5 / 24.0, pulses: 0, running: true, beats: 0.0 }
    }

    /// One MIDI clock pulse: refine the tempo estimate and hard-sync position.
    fn pulse(&mut self, now: Instant) {
        if let Some(prev) = self.last_pulse {
            let dt = now.duration_since(prev).as_secs_f32();
            // Ignore implausible gaps (>250 ms/pulse ~ <10 BPM) to ride dropouts.
            if dt > 0.0 && dt < 0.25 {
                self.pulse_dt = self.pulse_dt * 0.9 + dt * 0.1;
            }
        }
        self.last_pulse = Some(now);
        self.pulses = self.pulses.wrapping_add(1);
        self.beats = (self.pulses as f64 / 24.0) % BEATS_WRAP; // resync to external
    }

    fn transport(&mut self, t: Transport) {
        match t {
            Transport::Start => {
                self.pulses = 0;
                self.beats = 0.0;
                self.running = true;
            }
            Transport::Continue => self.running = true,
            Transport::Stop => self.running = false,
        }
    }

    /// Tempo from MIDI pulse spacing.
    fn bpm(&self) -> f32 {
        (60.0 / (self.pulse_dt * 24.0)).clamp(20.0, 300.0)
    }

    /// Advance the free-running position by `dt` seconds at `bpm`.
    fn advance(&mut self, dt: f32, bpm: f32) {
        if self.running {
            self.beats = (self.beats + dt as f64 * (bpm as f64 / 60.0)) % BEATS_WRAP;
        }
    }

    fn beats(&self) -> f64 {
        self.beats
    }
}

/// Something the engine did that the outside world (chiefly the web UI) may want
/// to hear about, so controllers and the on-screen state stay consistent.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineNotice {
    /// A parameter's value changed. `norm` is its position in 0..1 for widgets.
    ParamChanged { path: String, value: ParamValue, norm: f32 },
    /// A control was just bound to a parameter via learn.
    Learned { mapping: Mapping },
    /// A trigger parameter fired this frame.
    Triggered { path: String },
    /// A lettering slot's text changed (the UI should refresh its fields).
    TextChanged,
}

/// The authoritative state container.
/// Number of triggerable lettering slots.
pub const TEXT_SLOTS: usize = 8;

pub struct Engine {
    params: ParamStore,
    mappings: crate::params::MappingTable,
    modmatrix: ModMatrix,
    notices: Vec<EngineNotice>,
    clock: BeatClock,
    /// Lettering bank: saved sentences and which one is currently shown.
    text_slots: Vec<String>,
    text_active: Option<usize>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self {
            params: ParamStore::new(),
            mappings: crate::params::MappingTable::new(),
            modmatrix: ModMatrix::new(),
            notices: Vec::new(),
            clock: BeatClock::new(),
            text_slots: vec![String::new(); TEXT_SLOTS],
            text_active: None,
        }
    }

    /// Currently-shown lettering slot, if any.
    pub fn text_active(&self) -> Option<usize> {
        self.text_active
    }

    /// The sentence stored in a slot.
    pub fn text_slot(&self, i: usize) -> &str {
        self.text_slots.get(i).map(String::as_str).unwrap_or("")
    }

    pub fn text_slots(&self) -> &[String] {
        &self.text_slots
    }

    pub fn modmatrix(&self) -> &ModMatrix {
        &self.modmatrix
    }

    /// Run the modulation pass for one frame: reset, accumulate every route's
    /// `amount * source`, then apply. Call before reading params for rendering.
    pub fn apply_modulation(&mut self, sources: &HashMap<String, f32>) {
        self.params.reset_modulation();
        for route in self.modmatrix.routes_mut() {
            let src = sources.get(&route.source).copied().unwrap_or(0.0);
            let raw = route.amount * src;
            // Per-route slew: 0 smooth = instant, higher = a slower one-pole
            // toward the target offset, so audio jitter does not snap params.
            let coeff = 1.0 - route.smooth.clamp(0.0, 0.99) * 0.96;
            route.smoothed += (raw - route.smoothed) * coeff;
            if route.smoothed.abs() < 1e-5 {
                continue;
            }
            if let Some(id) = self.params.id_of(&route.target) {
                self.params.add_mod_offset(id, route.smoothed);
            }
        }
        self.params.commit_modulation();
    }

    pub fn params(&self) -> &ParamStore {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut ParamStore {
        &mut self.params
    }

    pub fn mappings(&self) -> &crate::params::MappingTable {
        &self.mappings
    }

    /// Drain notices accumulated since the last call, for publishing to the UI.
    pub fn take_notices(&mut self) -> Vec<EngineNotice> {
        std::mem::take(&mut self.notices)
    }

    /// Apply one control event.
    pub fn handle(&mut self, ev: ControlEvent) {
        match ev {
            ControlEvent::MidiCc { channel, cc, value } => {
                self.route_raw(SourceKey::MidiCc { channel, cc }, value as f32 / 127.0, value > 0);
            }
            ControlEvent::MidiNote { channel, note, velocity, on } => {
                let raw = velocity as f32 / 127.0;
                self.route_raw(SourceKey::MidiNote { channel, note }, raw, on);
            }
            ControlEvent::Osc { addr, value, pressed } => {
                self.route_raw(SourceKey::Osc { addr }, value, pressed);
            }
            ControlEvent::SetParam { path, value } => self.set_path(&path, value),
            ControlEvent::SetParamNorm { path, norm } => self.set_path_norm(&path, norm),
            ControlEvent::Trigger { path } => self.fire_trigger(&path),
            ControlEvent::Release { path } => self.release(&path),
            ControlEvent::MidiClock => {
                self.clock.pulse(Instant::now());
                // Sync the tempo param from the detected MIDI tempo (silent: the
                // BPM display rides telemetry, not a per-pulse notice flood).
                let bpm = self.clock.bpm();
                self.params.set_path("clock.bpm", ParamValue::Float(bpm));
            }
            ControlEvent::Transport(t) => self.clock.transport(t),
            ControlEvent::Arm { path, mode } => {
                // Trigger-kind params (e.g. lettering slots) learn as Gate, so a
                // MIDI note shows on note-on and hides on note-off.
                let mode = match self.params.id_of(&path).map(|id| &self.params.spec(id).kind) {
                    Some(ParamKind::Trigger) => crate::params::MapMode::Gate,
                    _ => mode,
                };
                self.mappings.arm(path, mode);
            }
            ControlEvent::Disarm => self.mappings.disarm(),
            ControlEvent::ClearMappingsFor { path } => self.mappings.remove_target(&path),
            ControlEvent::SetModRoute { source, target, amount, smooth } => {
                self.modmatrix.set(source, target, amount, smooth);
            }
            ControlEvent::SetText { slot, text } => {
                let i = slot as usize;
                if i < self.text_slots.len() {
                    self.text_slots[i] = text;
                    self.notices.push(EngineNotice::TextChanged);
                }
            }
            ControlEvent::LoadPreset(path) => {
                if let Err(e) = self.load_preset(&path) {
                    tracing::warn!("preset load failed: {e:#}");
                }
            }
            ControlEvent::SavePreset(path) => {
                if let Err(e) = self.save_preset(&path) {
                    tracing::warn!("preset save failed: {e:#}");
                }
            }
            // I/O device selection, media rescan and scripting are owned by the
            // window backend, which intercepts these before they reach the engine.
            ControlEvent::SetAudioDevice(_)
            | ControlEvent::SetMidiPort(_)
            | ControlEvent::RescanMedia
            | ControlEvent::SetScript(_)
            | ControlEvent::SaveScript { .. }
            | ControlEvent::LoadScript(_) => {}
        }
    }

    /// Route a raw control through the mapping matrix (honouring learn).
    fn route_raw(&mut self, source: SourceKey, raw: f32, pressed: bool) {
        let (learned, mapping) = self.mappings.resolve(&source);
        let Some(mapping) = mapping else { return };
        if learned {
            self.notices.push(EngineNotice::Learned { mapping: mapping.clone() });
        }
        let current = self
            .params
            .id_of(&mapping.target)
            .map(|id| self.params.normalized(id))
            .unwrap_or(0.0);
        match mapping.apply(raw, pressed, current) {
            MapAction::SetNormalized { target, norm } => self.set_path_norm(&target, norm),
            MapAction::Toggle { target } => {
                if let Some(id) = self.params.id_of(&target) {
                    let now = !self.params.get_bool(id);
                    self.params.set(id, ParamValue::Bool(now));
                    self.emit_change(&target);
                }
            }
            MapAction::Trigger { target } => self.fire_trigger(&target),
            MapAction::Release { target } => self.release(&target),
            MapAction::None => {}
        }
    }

    /// Release a gated target: hide the lettering slot it shows, or turn a
    /// boolean off. Mirrors a MIDI note-off / pointer-up.
    fn release(&mut self, path: &str) {
        if let Some(rest) = path.strip_prefix("text.").and_then(|s| s.strip_suffix(".trigger")) {
            if let Ok(n) = rest.parse::<usize>() {
                if self.text_active == Some(n) {
                    self.text_active = None;
                }
                return;
            }
        }
        if let Some(id) = self.params.id_of(path) {
            if matches!(self.params.spec(id).kind, ParamKind::Bool { .. }) {
                self.params.set(id, ParamValue::Bool(false));
                self.emit_change(path);
            }
        }
    }

    /// Current MIDI/OSC bindings as (source label, target path, mode label),
    /// for the web mapping list.
    pub fn mappings_list(&self) -> Vec<(String, String, String)> {
        self.mappings
            .mappings()
            .iter()
            .map(|m| (m.source.desc(), m.target.clone(), format!("{:?}", m.mode).to_lowercase()))
            .collect()
    }

    fn set_path(&mut self, path: &str, value: ParamValue) {
        if self.params.set_path(path, value).is_some() {
            self.emit_change(path);
        }
    }

    fn set_path_norm(&mut self, path: &str, norm: f32) {
        if let Some(id) = self.params.id_of(path) {
            self.params.set_normalized(id, norm);
            self.emit_change(path);
        }
    }

    fn fire_trigger(&mut self, path: &str) {
        if let Some(id) = self.params.id_of(path) {
            self.params.set(id, ParamValue::Bool(true));
            self.notices.push(EngineNotice::Triggered { path: path.to_string() });
        }
        // Lettering triggers steer the active slot.
        if path == "text.clear" {
            self.text_active = None;
        } else if let Some(rest) = path.strip_prefix("text.").and_then(|s| s.strip_suffix(".trigger")) {
            if let Ok(n) = rest.parse::<usize>() {
                if n < self.text_slots.len() {
                    self.text_active = Some(n);
                }
            }
        }
    }

    /// Advance the free-running musical clock by one frame and refresh the
    /// `clock.beat`/`clock.bar` phase params. Driven at the frame rate; uses the
    /// `clock.bpm` param as the free-run tempo (MIDI clock writes that param).
    /// Updates are silent (no notices) since the phase changes every frame; the
    /// UI reads it from telemetry instead.
    pub fn tick_clock(&mut self, dt: f32) {
        let bpm = self.params.id_of("clock.bpm").map(|id| self.params.get_f32(id)).unwrap_or(120.0);
        self.clock.advance(dt, bpm);
        let beats = self.clock.beats();
        self.params.set_path("clock.beat", ParamValue::Float(beats.rem_euclid(1.0) as f32));
        self.params.set_path("clock.bar", ParamValue::Float((beats / 4.0).rem_euclid(1.0) as f32));
    }

    /// Free-running musical position in beats (wrapped), for tempo-synced LFOs.
    pub fn musical_beats(&self) -> f64 {
        self.clock.beats()
    }

    fn emit_change(&mut self, path: &str) {
        if let Some(id) = self.params.id_of(path) {
            self.notices.push(EngineNotice::ParamChanged {
                path: path.to_string(),
                value: self.params.get(id),
                norm: self.params.normalized(id),
            });
        }
    }

    /// Reset momentary triggers back to their resting state. Call once at the
    /// end of each frame, after generators have had a chance to read them.
    pub fn end_frame(&mut self) {
        // Collect first to avoid borrowing the store mutably while iterating it.
        let trigger_ids: Vec<_> = self
            .params
            .iter()
            .filter(|(_, spec, _)| matches!(spec.kind, ParamKind::Trigger))
            .map(|(id, _, _)| id)
            .collect();
        for id in trigger_ids {
            self.params.set(id, ParamValue::Bool(false));
        }
    }

    /// Capture the current state into a preset.
    pub fn to_preset(&self, name: impl Into<String>) -> Preset {
        Preset {
            version: 1,
            name: name.into(),
            params: self.params.snapshot(),
            mappings: self.mappings.mappings().to_vec(),
            mod_routes: self.modmatrix.routes().to_vec(),
            text: self.text_slots.clone(),
        }
    }

    pub fn apply_preset(&mut self, preset: &Preset) {
        self.params.apply_snapshot(&preset.params);
        for m in &preset.mappings {
            self.mappings.upsert(m.clone());
        }
        self.mappings.reindex();
        self.modmatrix.replace_all(preset.mod_routes.clone());
        // Restore lettering slots (keep the fixed slot count).
        for (i, t) in preset.text.iter().take(self.text_slots.len()).enumerate() {
            self.text_slots[i] = t.clone();
        }
        if !preset.text.is_empty() {
            self.notices.push(EngineNotice::TextChanged);
        }
        // The whole surface changed; notify every known parameter.
        let paths: Vec<String> = self.params.iter().map(|(_, s, _)| s.path.clone()).collect();
        for p in paths {
            self.emit_change(&p);
        }
    }

    pub fn load_preset(&mut self, path: &str) -> anyhow::Result<()> {
        let preset = Preset::load(path)?;
        self.apply_preset(&preset);
        tracing::info!("loaded preset {path}");
        Ok(())
    }

    pub fn save_preset(&mut self, path: &str) -> anyhow::Result<()> {
        let name = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("patch")
            .to_string();
        self.to_preset(name).save(path)?;
        tracing::info!("saved preset {path}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{MapMode, ParamSpec};

    fn engine_with_float() -> (Engine, &'static str) {
        let mut e = Engine::new();
        e.params_mut().register(ParamSpec::new(
            "x",
            "X",
            "test",
            ParamKind::Float { min: 0.0, max: 1.0, default: 0.0 },
        ));
        (e, "x")
    }

    #[test]
    fn midi_cc_through_learn_then_drives_param() {
        let (mut e, path) = engine_with_float();
        e.handle(ControlEvent::Arm { path: path.into(), mode: MapMode::Absolute });
        // First CC binds via learn and applies immediately.
        e.handle(ControlEvent::MidiCc { channel: 0, cc: 10, value: 127 });
        assert!((e.params().get_f32(e.params().id_of(path).unwrap()) - 1.0).abs() < 1e-6);
        let notices = e.take_notices();
        assert!(notices.iter().any(|n| matches!(n, EngineNotice::Learned { .. })));

        // A subsequent CC moves the bound parameter without re-learning.
        e.handle(ControlEvent::MidiCc { channel: 0, cc: 10, value: 0 });
        assert_eq!(e.params().get_f32(e.params().id_of(path).unwrap()), 0.0);
    }

    #[test]
    fn triggers_reset_at_end_of_frame() {
        let mut e = Engine::new();
        e.params_mut().register(ParamSpec::new("t", "T", "test", ParamKind::Trigger));
        e.handle(ControlEvent::Trigger { path: "t".into() });
        let id = e.params().id_of("t").unwrap();
        assert!(e.params().get_bool(id));
        e.end_frame();
        assert!(!e.params().get_bool(id));
    }

    #[test]
    fn modulation_offsets_effective_value_not_base() {
        let (mut e, path) = engine_with_float();
        e.handle(ControlEvent::SetParamNorm { path: path.into(), norm: 0.5 });
        // Route a source onto the param at +0.5 depth.
        e.handle(ControlEvent::SetModRoute { source: "audio.low".into(), target: path.into(), amount: 0.5, smooth: 0.0 });

        let mut sources = HashMap::new();
        sources.insert("audio.low".into(), 1.0);
        e.apply_modulation(&sources);

        let id = e.params().id_of(path).unwrap();
        // Effective (render) value pushed to the top: 0.5 base + 0.5*1.0.
        assert!((e.params().get_f32(id) - 1.0).abs() < 1e-6);
        // Base (UI / preset) is untouched.
        assert!((e.params().get(id).as_f32() - 0.5).abs() < 1e-6);

        // With no signal, effective falls back to base.
        sources.insert("audio.low".into(), 0.0);
        e.apply_modulation(&sources);
        assert!((e.params().get_f32(id) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn preset_roundtrip_through_engine() {
        let (mut e, path) = engine_with_float();
        e.handle(ControlEvent::SetParamNorm { path: path.into(), norm: 0.5 });
        let preset = e.to_preset("p");

        let (mut e2, _) = engine_with_float();
        e2.apply_preset(&preset);
        assert!((e2.params().get_f32(e2.params().id_of(path).unwrap()) - 0.5).abs() < 1e-6);
    }
}
