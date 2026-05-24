//! Parameters: the typed control surface every visual module exposes.
//!
//! - [`value`] - the value/kind types and normalisation maths.
//! - [`store`] - the registry of live parameters (handles + current values).
//! - [`mapping`] - the matrix binding raw controls to parameters, with learn.

pub mod mapping;
pub mod modmatrix;
pub mod store;
pub mod value;

pub use mapping::{MapAction, MapMode, Mapping, MappingTable, SourceKey};
pub use modmatrix::{ModMatrix, ModRoute, MOD_SOURCES};
pub use store::{ParamId, ParamSpec, ParamStore};
pub use value::{ParamKind, ParamValue};
