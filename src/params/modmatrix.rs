//! The modulation matrix: route signal sources onto parameters with an amount.
//!
//! Sources are continuous signals - audio bands, the onset pulse, the beat
//! clock phase, and free-running LFOs. Each route adds `amount * source` (in
//! normalised parameter space) onto a target's base value every frame, so the
//! same parameter can be hand-set *and* pumped by the music. Amount is bipolar
//! so a route can open or close a parameter.

use serde::{Deserialize, Serialize};

/// The signal sources the matrix can route from. Names are stable ids shared
/// with the web UI. Audio sources are 0..1; LFOs are bipolar -1..1.
pub const MOD_SOURCES: &[&str] = &[
    "audio.low",
    "audio.mid",
    "audio.high",
    "audio.rms",
    "audio.level",
    "audio.beat",
    "clock.beat",
    "clock.bar",
    "lfo.1",
    "lfo.2",
    "lfo.3",
];

/// One modulation route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModRoute {
    pub source: String,
    /// Target parameter path.
    pub target: String,
    /// Bipolar depth, typically -1..1.
    pub amount: f32,
    /// Smoothing / slew, 0 = instant, 1 = very smooth. Tames jittery audio.
    #[serde(default)]
    pub smooth: f32,
    /// Runtime smoothed offset (not persisted).
    #[serde(skip)]
    pub smoothed: f32,
}

/// The set of active routes.
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct ModMatrix {
    routes: Vec<ModRoute>,
}

impl ModMatrix {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn routes(&self) -> &[ModRoute] {
        &self.routes
    }

    pub fn routes_mut(&mut self) -> &mut [ModRoute] {
        &mut self.routes
    }

    /// Add a route, or update it if one already links this source and target.
    /// Setting amount to zero removes the route.
    pub fn set(&mut self, source: impl Into<String>, target: impl Into<String>, amount: f32, smooth: f32) {
        let (source, target) = (source.into(), target.into());
        if amount == 0.0 {
            self.remove(&source, &target);
            return;
        }
        if let Some(r) = self.routes.iter_mut().find(|r| r.source == source && r.target == target) {
            r.amount = amount;
            r.smooth = smooth;
        } else {
            self.routes.push(ModRoute { source, target, amount, smooth, smoothed: 0.0 });
        }
    }

    pub fn remove(&mut self, source: &str, target: &str) {
        self.routes.retain(|r| !(r.source == source && r.target == target));
    }

    /// Drop every route pointing at a parameter (used when clearing it).
    pub fn remove_target(&mut self, target: &str) {
        self.routes.retain(|r| r.target != target);
    }

    pub fn replace_all(&mut self, routes: Vec<ModRoute>) {
        self.routes = routes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_upserts_and_zero_removes() {
        let mut m = ModMatrix::new();
        m.set("audio.low", "layer.0.opacity", 0.5, 0.0);
        assert_eq!(m.routes().len(), 1);
        m.set("audio.low", "layer.0.opacity", 0.8, 0.3);
        assert_eq!(m.routes().len(), 1);
        assert_eq!(m.routes()[0].amount, 0.8);
        assert_eq!(m.routes()[0].smooth, 0.3);
        m.set("audio.low", "layer.0.opacity", 0.0, 0.0);
        assert_eq!(m.routes().len(), 0);
    }
}
