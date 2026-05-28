//! Backends + thin native render-side helpers.
//!
//! The platform-independent render path (pipeline, shaders, banks, engine,
//! params) lives in the `audiovis-render-core` crate. This module only holds
//! the host-side backends (window via winit/glutin; Linux DRM) that own the GL
//! context and drive the frame loop on top of the shared `Pipeline`.

pub mod backend;

// Re-export the few render-core shapes the backends and other bin modules
// name often, so the rest of the bin keeps reading as if these still lived
// here. (Adding a new render-core re-export is a one-liner, not a deep import
// rewrite at every callsite.)
pub use audiovis_render_core::{FrameContext, GlslFlavor};
