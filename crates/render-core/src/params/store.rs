//! The parameter registry.
//!
//! Generators and effects register their parameters here at construction time
//! and get back a [`ParamId`] handle. Per-frame reads go through that handle,
//! which is just a `Vec` index, so reading a uniform every frame is cheap and
//! never touches a string hash.
//!
//! The store is owned by the engine thread; control sources do not mutate it
//! directly - they post events that the engine applies. That keeps the
//! authoritative visual state in one place with no locking on the hot path.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::value::{ParamKind, ParamValue};

/// Opaque, cheap handle to a registered parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParamId(pub usize);

/// Static description of one parameter, surfaced to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamSpec {
    /// Unique dotted path, e.g. `layer.0.plasma.speed`. The stable identity used
    /// by mappings, presets and the web UI.
    pub path: String,
    /// Human label for the control surface.
    pub name: String,
    /// Group/section the UI uses to lay controls out.
    pub group: String,
    pub kind: ParamKind,
    /// Optional unit string for display (e.g. `Hz`, `%`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

impl ParamSpec {
    pub fn new(path: impl Into<String>, name: impl Into<String>, group: impl Into<String>, kind: ParamKind) -> Self {
        Self { path: path.into(), name: name.into(), group: group.into(), kind, unit: None }
    }

    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }
}

/// Registry of all live parameters and their current values.
#[derive(Default)]
pub struct ParamStore {
    specs: Vec<ParamSpec>,
    /// Base values: what the user sets, what presets save.
    values: Vec<ParamValue>,
    /// Effective float values after modulation (base + routed sources). The
    /// renderer reads these via `get_f32`; non-float kinds mirror the base.
    modulated: Vec<f32>,
    /// Per-frame accumulated normalised modulation offset, applied in `commit`.
    mod_offset: Vec<f32>,
    by_path: HashMap<String, usize>,
}

impl ParamStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a parameter, returning its handle. If the path already exists
    /// the existing handle is returned and the spec is left unchanged, which
    /// makes re-registration on reload idempotent.
    pub fn register(&mut self, spec: ParamSpec) -> ParamId {
        if let Some(&idx) = self.by_path.get(&spec.path) {
            return ParamId(idx);
        }
        let idx = self.specs.len();
        self.by_path.insert(spec.path.clone(), idx);
        let default = spec.kind.default_value();
        self.values.push(default);
        self.modulated.push(default.as_f32());
        self.mod_offset.push(0.0);
        self.specs.push(spec);
        ParamId(idx)
    }

    pub fn id_of(&self, path: &str) -> Option<ParamId> {
        self.by_path.get(path).copied().map(ParamId)
    }

    pub fn spec(&self, id: ParamId) -> &ParamSpec {
        &self.specs[id.0]
    }

    /// Base value (what the UI shows and presets save).
    pub fn get(&self, id: ParamId) -> ParamValue {
        self.values[id.0]
    }

    /// Effective float read for the render path - includes modulation.
    pub fn get_f32(&self, id: ParamId) -> f32 {
        self.modulated[id.0]
    }

    pub fn get_bool(&self, id: ParamId) -> bool {
        self.values[id.0].as_bool()
    }

    /// Set the base value, coercing into the parameter's kind and range. The
    /// modulated value follows immediately so a change is visible before the
    /// next modulation pass.
    pub fn set(&mut self, id: ParamId, value: ParamValue) {
        let coerced = self.specs[id.0].kind.coerce(value);
        self.values[id.0] = coerced;
        self.modulated[id.0] = coerced.as_f32();
    }

    /// Set from a normalised 0..1 control position.
    pub fn set_normalized(&mut self, id: ParamId, norm: f32) {
        let v = self.specs[id.0].kind.from_normalized(norm);
        self.values[id.0] = v;
        self.modulated[id.0] = v.as_f32();
    }

    // --- Modulation pass (called once per frame by the engine) ---

    /// Clear last frame's modulation: effective = base, offsets zeroed.
    pub fn reset_modulation(&mut self) {
        for i in 0..self.values.len() {
            self.modulated[i] = self.values[i].as_f32();
            self.mod_offset[i] = 0.0;
        }
    }

    /// Accumulate a normalised modulation offset onto a parameter.
    pub fn add_mod_offset(&mut self, id: ParamId, delta: f32) {
        self.mod_offset[id.0] += delta;
    }

    /// Apply accumulated offsets to float parameters, in normalised space so the
    /// amount is range-independent. Non-float kinds are left at their base.
    pub fn commit_modulation(&mut self) {
        for i in 0..self.values.len() {
            if self.mod_offset[i] == 0.0 {
                continue;
            }
            if let ParamKind::Float { .. } = self.specs[i].kind {
                let base_norm = self.specs[i].kind.to_normalized(self.values[i]);
                let eff = (base_norm + self.mod_offset[i]).clamp(0.0, 1.0);
                self.modulated[i] = self.specs[i].kind.from_normalized(eff).as_f32();
            }
        }
    }

    /// Set by path; returns the resolved handle, or `None` if unknown.
    pub fn set_path(&mut self, path: &str, value: ParamValue) -> Option<ParamId> {
        let id = self.id_of(path)?;
        self.set(id, value);
        Some(id)
    }

    /// Normalised position of a parameter, for driving UI widgets.
    pub fn normalized(&self, id: ParamId) -> f32 {
        self.specs[id.0].kind.to_normalized(self.values[id.0])
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// Iterate specs alongside their current values, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = (ParamId, &ParamSpec, ParamValue)> {
        self.specs
            .iter()
            .enumerate()
            .map(move |(i, s)| (ParamId(i), s, self.values[i]))
    }

    /// Snapshot every value keyed by path - the form used for presets.
    pub fn snapshot(&self) -> HashMap<String, ParamValue> {
        self.specs
            .iter()
            .enumerate()
            .map(|(i, s)| (s.path.clone(), self.values[i]))
            .collect()
    }

    /// Apply a snapshot, ignoring unknown paths (so presets survive across
    /// versions that add or remove generators).
    pub fn apply_snapshot(&mut self, snap: &HashMap<String, ParamValue>) {
        for (path, value) in snap {
            self.set_path(path, *value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn float(path: &str, default: f32) -> ParamSpec {
        ParamSpec::new(path, path, "test", ParamKind::Float { min: 0.0, max: 1.0, default })
    }

    #[test]
    fn register_is_idempotent_by_path() {
        let mut s = ParamStore::new();
        let a = s.register(float("a", 0.2));
        let b = s.register(float("a", 0.9));
        assert_eq!(a, b);
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(a), ParamValue::Float(0.2));
    }

    #[test]
    fn set_coerces_and_reads_back() {
        let mut s = ParamStore::new();
        let id = s.register(float("a", 0.0));
        s.set(id, ParamValue::Float(2.0));
        assert_eq!(s.get_f32(id), 1.0);
    }

    #[test]
    fn snapshot_roundtrips_and_ignores_unknown() {
        let mut s = ParamStore::new();
        let id = s.register(float("a", 0.0));
        s.set(id, ParamValue::Float(0.7));
        let snap = s.snapshot();

        let mut s2 = ParamStore::new();
        let id2 = s2.register(float("a", 0.0));
        s2.set_path("ghost", ParamValue::Float(1.0)); // unknown, ignored
        s2.apply_snapshot(&snap);
        assert!((s2.get_f32(id2) - 0.7).abs() < 1e-6);
    }
}
