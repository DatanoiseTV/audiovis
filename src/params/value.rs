//! Parameter values and the metadata that describes them.
//!
//! A parameter is the fundamental unit of control in audiovis: every generator
//! and effect exposes a set of them, and every control source (MIDI, OSC, web,
//! audio modulation) ultimately resolves to "set this parameter to this value".

use serde::{Deserialize, Serialize};

/// A concrete parameter value. Kept deliberately small - the four shapes here
/// cover everything the visual modules need.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum ParamValue {
    Float(f32),
    Bool(bool),
    Int(i64),
}

impl ParamValue {
    /// Best-effort conversion to `f32`, useful for shader uniforms where bools
    /// and ints are uploaded as floats anyway.
    pub fn as_f32(self) -> f32 {
        match self {
            ParamValue::Float(v) => v,
            ParamValue::Bool(b) => b as i32 as f32,
            ParamValue::Int(i) => i as f32,
        }
    }

    pub fn as_bool(self) -> bool {
        match self {
            ParamValue::Float(v) => v >= 0.5,
            ParamValue::Bool(b) => b,
            ParamValue::Int(i) => i != 0,
        }
    }

    pub fn as_i64(self) -> i64 {
        match self {
            ParamValue::Float(v) => v.round() as i64,
            ParamValue::Bool(b) => b as i64,
            ParamValue::Int(i) => i,
        }
    }
}

/// Describes the shape, range and default of a parameter. The range lets us map
/// a normalised 0..1 control (a fader, a MIDI CC) onto the real value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ParamKind {
    /// Continuous value within `[min, max]`.
    Float { min: f32, max: f32, default: f32 },
    /// On/off.
    Bool { default: bool },
    /// Integer within `[min, max]`, also used to select among enum options.
    Int { min: i64, max: i64, default: i64 },
    /// A momentary action (a button). Carries no stored value.
    Trigger,
}

impl ParamKind {
    /// The value a freshly-registered parameter starts at.
    pub fn default_value(&self) -> ParamValue {
        match *self {
            ParamKind::Float { default, .. } => ParamValue::Float(default),
            ParamKind::Bool { default } => ParamValue::Bool(default),
            ParamKind::Int { default, .. } => ParamValue::Int(default),
            // Triggers have no resting value; represent as false.
            ParamKind::Trigger => ParamValue::Bool(false),
        }
    }

    /// Clamp an incoming value to this kind's valid range and type.
    pub fn coerce(&self, v: ParamValue) -> ParamValue {
        match *self {
            ParamKind::Float { min, max, .. } => ParamValue::Float(v.as_f32().clamp(min, max)),
            ParamKind::Bool { .. } => ParamValue::Bool(v.as_bool()),
            ParamKind::Int { min, max, .. } => ParamValue::Int(v.as_i64().clamp(min, max)),
            ParamKind::Trigger => ParamValue::Bool(v.as_bool()),
        }
    }

    /// Map a normalised control position in `[0, 1]` onto a real value.
    pub fn from_normalized(&self, norm: f32) -> ParamValue {
        let n = norm.clamp(0.0, 1.0);
        match *self {
            ParamKind::Float { min, max, .. } => ParamValue::Float(min + (max - min) * n),
            ParamKind::Bool { .. } => ParamValue::Bool(n >= 0.5),
            ParamKind::Int { min, max, .. } => {
                let span = (max - min) as f32;
                ParamValue::Int(min + (span * n).round() as i64)
            }
            ParamKind::Trigger => ParamValue::Bool(n >= 0.5),
        }
    }

    /// Inverse of [`from_normalized`]: where the current value sits in `[0, 1]`.
    /// Used to drive UI widgets from the authoritative engine state.
    pub fn to_normalized(&self, v: ParamValue) -> f32 {
        match *self {
            ParamKind::Float { min, max, .. } if max > min => {
                ((v.as_f32() - min) / (max - min)).clamp(0.0, 1.0)
            }
            ParamKind::Int { min, max, .. } if max > min => {
                ((v.as_i64() - min) as f32 / (max - min) as f32).clamp(0.0, 1.0)
            }
            ParamKind::Bool { .. } | ParamKind::Trigger => {
                if v.as_bool() {
                    1.0
                } else {
                    0.0
                }
            }
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_normalization_roundtrips() {
        let k = ParamKind::Float { min: 20.0, max: 20_000.0, default: 1000.0 };
        let v = k.from_normalized(0.5);
        assert_eq!(v, ParamValue::Float(10_010.0));
        let n = k.to_normalized(v);
        assert!((n - 0.5).abs() < 1e-6);
    }

    #[test]
    fn coerce_clamps_into_range() {
        let k = ParamKind::Float { min: 0.0, max: 1.0, default: 0.0 };
        assert_eq!(k.coerce(ParamValue::Float(5.0)), ParamValue::Float(1.0));
        assert_eq!(k.coerce(ParamValue::Float(-5.0)), ParamValue::Float(0.0));
    }

    #[test]
    fn int_normalization_picks_discrete_steps() {
        let k = ParamKind::Int { min: 0, max: 4, default: 0 };
        assert_eq!(k.from_normalized(0.0), ParamValue::Int(0));
        assert_eq!(k.from_normalized(1.0), ParamValue::Int(4));
        assert_eq!(k.from_normalized(0.5), ParamValue::Int(2));
    }
}
