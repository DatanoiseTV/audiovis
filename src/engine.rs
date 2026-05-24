//! The engine core: the single owner of authoritative visual state.
//!
//! It holds the parameter store and the mapping table, and turns the stream of
//! [`ControlEvent`]s into parameter changes. It records outbound [`EngineNotice`]s
//! so the web UI can stay in sync, and exposes per-frame trigger handling.
//!
//! Rendering is layered on top of this in later milestones; the engine itself is
//! deliberately render-agnostic and fully unit-testable on its own.

use crate::config::Preset;
use crate::control::ControlEvent;
use crate::params::{MapAction, Mapping, ParamKind, ParamStore, ParamValue, SourceKey};

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
}

/// The authoritative state container.
pub struct Engine {
    params: ParamStore,
    mappings: crate::params::MappingTable,
    notices: Vec<EngineNotice>,
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
            notices: Vec::new(),
        }
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
            ControlEvent::Arm { path, mode } => self.mappings.arm(path, mode),
            ControlEvent::Disarm => self.mappings.disarm(),
            ControlEvent::ClearMappingsFor { path } => self.mappings.remove_target(&path),
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
            MapAction::None => {}
        }
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
        }
    }

    pub fn apply_preset(&mut self, preset: &Preset) {
        self.params.apply_snapshot(&preset.params);
        for m in &preset.mappings {
            self.mappings.upsert(m.clone());
        }
        self.mappings.reindex();
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
    fn preset_roundtrip_through_engine() {
        let (mut e, path) = engine_with_float();
        e.handle(ControlEvent::SetParamNorm { path: path.into(), norm: 0.5 });
        let preset = e.to_preset("p");

        let (mut e2, _) = engine_with_float();
        e2.apply_preset(&preset);
        assert!((e2.params().get_f32(e2.params().id_of(path).unwrap()) - 0.5).abs() < 1e-6);
    }
}
