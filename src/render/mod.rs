//! Rendering: the GL abstraction, the frame pipeline and the output backends.
//!
//! The pipeline is backend-agnostic - it only needs a live `glow` context and a
//! viewport size. The backends ([`backend::window`] for desktop, and the
//! Linux-only DRM backend) create that context and drive the frame loop.

pub mod backend;
pub mod compositor;
pub mod generators;
pub mod gl;
pub mod media;
pub mod mesh;
pub mod pipeline;
pub mod post;
pub mod sim;
pub mod text;

pub use gl::GlslFlavor;

/// Per-frame timing and sizing handed to the pipeline.
#[derive(Debug, Clone, Copy)]
pub struct FrameContext {
    /// Seconds since the engine started.
    pub time: f32,
    /// Seconds since the previous frame.
    pub dt: f32,
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Monotonic frame counter.
    pub frame: u64,
}
