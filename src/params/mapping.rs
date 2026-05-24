//! The mapping matrix: how a raw control input becomes a parameter change.
//!
//! This is what makes control "flexible": any MIDI CC, MIDI note or OSC address
//! can be bound to any parameter, with a response curve, range window, polarity
//! and mode (absolute fader, relative encoder, toggle, momentary trigger). A
//! "learn" mode lets the performer arm a parameter and then wiggle a control to
//! bind it, with no config editing.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Identifies a physical/raw control independent of what it's bound to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "src", rename_all = "snake_case")]
pub enum SourceKey {
    MidiCc { channel: u8, cc: u8 },
    MidiNote { channel: u8, note: u8 },
    Osc { addr: String },
}

/// How an incoming control value is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MapMode {
    /// The control sets the value directly (a fader/knob).
    #[default]
    Absolute,
    /// The control nudges the value up/down (an endless encoder).
    Relative,
    /// A press flips a boolean parameter.
    Toggle,
    /// A press fires a momentary trigger.
    Trigger,
}

/// Response curve applied to the normalised input before remapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Curve {
    #[default]
    Linear,
    /// Eased toward the top - good for brightness/intensity.
    Exponential,
    /// Eased toward the bottom - good for frequencies.
    Logarithmic,
}

impl Curve {
    fn shape(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Curve::Linear => x,
            Curve::Exponential => x * x,
            Curve::Logarithmic => x.sqrt(),
        }
    }
}

/// One binding from a raw control to a parameter path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mapping {
    pub source: SourceKey,
    /// Target parameter path (resolved against the [`ParamStore`] when applied).
    pub target: String,
    #[serde(default)]
    pub mode: MapMode,
    #[serde(default)]
    pub curve: Curve,
    /// Output window within the parameter's normalised 0..1 range.
    #[serde(default = "zero")]
    pub lo: f32,
    #[serde(default = "one")]
    pub hi: f32,
    /// Flip the polarity (a fader that opens as it goes down).
    #[serde(default)]
    pub invert: bool,
    /// Step size for [`MapMode::Relative`], as a fraction of the full range.
    #[serde(default = "default_step")]
    pub step: f32,
}

fn zero() -> f32 {
    0.0
}
fn one() -> f32 {
    1.0
}
fn default_step() -> f32 {
    0.02
}

/// The result of feeding a raw input through a mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum MapAction {
    /// Set the target parameter to this normalised position.
    SetNormalized { target: String, norm: f32 },
    /// Flip the target boolean parameter.
    Toggle { target: String },
    /// Fire the target trigger parameter.
    Trigger { target: String },
    /// Nothing to do (e.g. a note-off for a non-toggle binding).
    None,
}

impl Mapping {
    /// Turn a raw normalised input (`raw` in 0..1; for notes, velocity/127 with
    /// `pressed` marking note-on) plus the current normalised value of the
    /// target into a concrete action.
    pub fn apply(&self, raw: f32, pressed: bool, current_norm: f32) -> MapAction {
        match self.mode {
            MapMode::Absolute => {
                let shaped = self.curve.shape(raw);
                let shaped = if self.invert { 1.0 - shaped } else { shaped };
                let norm = self.lo + (self.hi - self.lo) * shaped;
                MapAction::SetNormalized { target: self.target.clone(), norm }
            }
            MapMode::Relative => {
                // Standard "offset 64" encoder convention: >64 is up, <64 is down.
                let dir = (raw * 127.0 - 64.0).signum();
                let next = (current_norm + dir * self.step).clamp(0.0, 1.0);
                MapAction::SetNormalized { target: self.target.clone(), norm: next }
            }
            MapMode::Toggle => {
                if pressed {
                    MapAction::Toggle { target: self.target.clone() }
                } else {
                    MapAction::None
                }
            }
            MapMode::Trigger => {
                if pressed {
                    MapAction::Trigger { target: self.target.clone() }
                } else {
                    MapAction::None
                }
            }
        }
    }
}

/// A parameter armed for learning, plus the binding mode the new mapping should
/// take once a control is captured.
#[derive(Debug, Clone)]
struct Armed {
    target: String,
    mode: MapMode,
}

/// The full set of bindings, with learn support.
#[derive(Default, Serialize, Deserialize)]
pub struct MappingTable {
    maps: Vec<Mapping>,
    #[serde(skip)]
    by_source: HashMap<SourceKey, usize>,
    #[serde(skip)]
    armed: Option<Armed>,
}

impl MappingTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild the source index after deserialisation or bulk edits.
    pub fn reindex(&mut self) {
        self.by_source.clear();
        for (i, m) in self.maps.iter().enumerate() {
            self.by_source.insert(m.source.clone(), i);
        }
    }

    /// Arm learn for a parameter. The next raw control that arrives will bind
    /// to it. Arming a different target replaces the previous arm.
    pub fn arm(&mut self, target: impl Into<String>, mode: MapMode) {
        self.armed = Some(Armed { target: target.into(), mode });
    }

    pub fn disarm(&mut self) {
        self.armed = None;
    }

    pub fn is_armed(&self) -> bool {
        self.armed.is_some()
    }

    /// Insert or replace a binding for a source.
    pub fn upsert(&mut self, mapping: Mapping) {
        if let Some(&i) = self.by_source.get(&mapping.source) {
            self.maps[i] = mapping;
        } else {
            self.by_source.insert(mapping.source.clone(), self.maps.len());
            self.maps.push(mapping);
        }
    }

    /// Remove any binding from this source.
    pub fn remove_source(&mut self, source: &SourceKey) {
        if let Some(i) = self.maps.iter().position(|m| &m.source == source) {
            self.maps.remove(i);
            self.reindex();
        }
    }

    /// Remove all bindings to a parameter path.
    pub fn remove_target(&mut self, target: &str) {
        self.maps.retain(|m| m.target != target);
        self.reindex();
    }

    pub fn mappings(&self) -> &[Mapping] {
        &self.maps
    }

    /// Resolve a raw control arrival. If learn is armed, this captures the
    /// binding and returns the freshly-created mapping (so the engine can apply
    /// it immediately). Otherwise it returns the existing binding, if any.
    ///
    /// Returns `(was_learned, Option<&Mapping>)`.
    pub fn resolve(&mut self, source: &SourceKey) -> (bool, Option<Mapping>) {
        if let Some(armed) = self.armed.take() {
            let mapping = Mapping {
                source: source.clone(),
                target: armed.target,
                mode: armed.mode,
                curve: Curve::Linear,
                lo: 0.0,
                hi: 1.0,
                invert: false,
                step: default_step(),
            };
            self.upsert(mapping.clone());
            return (true, Some(mapping));
        }
        let existing = self.by_source.get(source).map(|&i| self.maps[i].clone());
        (false, existing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc(ch: u8, n: u8) -> SourceKey {
        SourceKey::MidiCc { channel: ch, cc: n }
    }

    #[test]
    fn absolute_maps_through_window_and_curve() {
        let m = Mapping {
            source: cc(0, 1),
            target: "x".into(),
            mode: MapMode::Absolute,
            curve: Curve::Linear,
            lo: 0.25,
            hi: 0.75,
            invert: false,
            step: 0.02,
        };
        let a = m.apply(0.5, true, 0.0);
        assert_eq!(a, MapAction::SetNormalized { target: "x".into(), norm: 0.5 });
        let a0 = m.apply(0.0, true, 0.0);
        assert_eq!(a0, MapAction::SetNormalized { target: "x".into(), norm: 0.25 });
    }

    #[test]
    fn invert_flips_polarity() {
        let m = Mapping {
            source: cc(0, 1),
            target: "x".into(),
            mode: MapMode::Absolute,
            curve: Curve::Linear,
            lo: 0.0,
            hi: 1.0,
            invert: true,
            step: 0.02,
        };
        assert_eq!(m.apply(0.0, true, 0.0), MapAction::SetNormalized { target: "x".into(), norm: 1.0 });
    }

    #[test]
    fn relative_encoder_nudges_from_current() {
        let m = Mapping {
            source: cc(0, 1),
            target: "x".into(),
            mode: MapMode::Relative,
            curve: Curve::Linear,
            lo: 0.0,
            hi: 1.0,
            invert: false,
            step: 0.1,
        };
        // value 65/127 -> just above center -> up by one step.
        let up = m.apply(65.0 / 127.0, true, 0.5);
        assert_eq!(up, MapAction::SetNormalized { target: "x".into(), norm: 0.6 });
        // value 1/127 -> below center -> down by one step.
        let down = m.apply(1.0 / 127.0, true, 0.5);
        assert_eq!(down, MapAction::SetNormalized { target: "x".into(), norm: 0.4 });
    }

    #[test]
    fn learn_captures_next_control() {
        let mut t = MappingTable::new();
        t.arm("brightness", MapMode::Absolute);
        assert!(t.is_armed());
        let (learned, mapping) = t.resolve(&cc(2, 7));
        assert!(learned);
        assert_eq!(mapping.unwrap().target, "brightness");
        assert!(!t.is_armed());
        // Now the same control resolves to the stored binding.
        let (learned2, mapping2) = t.resolve(&cc(2, 7));
        assert!(!learned2);
        assert_eq!(mapping2.unwrap().target, "brightness");
    }

    #[test]
    fn remove_target_drops_all_its_bindings() {
        let mut t = MappingTable::new();
        t.arm("x", MapMode::Absolute);
        t.resolve(&cc(0, 1));
        t.arm("x", MapMode::Absolute);
        t.resolve(&cc(0, 2));
        assert_eq!(t.mappings().len(), 2);
        t.remove_target("x");
        assert_eq!(t.mappings().len(), 0);
    }
}
