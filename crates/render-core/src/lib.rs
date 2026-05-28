//! Platform-independent render core for audiovis.
//!
//! Everything that turns engine state into pixels lives here: the GL
//! abstraction, the generator/sim/post-FX shader banks, the layer compositor,
//! the mesh + ISF runtimes, and the parameter store + engine that own the live
//! state they read from. There are deliberately no native I/O dependencies
//! (no `cpal`, `midir`, `winit`, `drm`, `nokhwa`, no `std::fs` for resource
//! discovery), so the same code compiles to wasm and runs in the browser on a
//! WebGL context.
//!
//! Resources the renderer needs but does not own (image files, OBJ meshes,
//! ISF shader sources) are pulled through the [`Resources`] trait. The native
//! binary implements it against the filesystem; the wasm build implements it
//! against an in-memory cache populated from the network.

pub mod compositor;
pub mod config;
pub mod engine;
pub mod events;
pub mod generators;
pub mod gl;
pub mod isf;
pub mod media;
pub mod mesh;
pub mod params;
pub mod pipeline;
pub mod post;
pub mod sim;
pub mod text;

// Re-exports for the shapes consumers reach for most often.
pub use config::Preset;
pub use engine::{Engine, EngineNotice, LFO_DIVISIONS};
pub use events::{ControlEvent, Transport};
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

/// Width of the waveform texture (samples per channel) the renderer expects.
/// The bin's audio capture produces stereo blocks of this many samples.
pub const WAVE: usize = 256;

/// Dimensions of the JS-script pixel buffer the script generator samples.
pub const SCRIPT_W: usize = 160;
pub const SCRIPT_H: usize = 90;

/// A pluggable source of named, on-disk-style resources the renderer needs to
/// build itself but does not own. Native side reads from disk; wasm side reads
/// from an in-memory cache populated by JS.
///
/// Implementations are expected to be cheap to call repeatedly (resource banks
/// build their caches at construction time and at explicit rescans).
pub trait Resources: Send + Sync {
    /// Sorted list of media file names available (PNG/JPEG/SVG).
    fn media_names(&self) -> Vec<String>;
    /// Raw bytes of a media file by name.
    fn read_media(&self, name: &str) -> Option<Vec<u8>>;

    /// Sorted list of OBJ mesh names available.
    fn mesh_names(&self) -> Vec<String>;
    /// OBJ text by name.
    fn read_mesh(&self, name: &str) -> Option<String>;

    /// Sorted list of ISF shader names available.
    fn isf_names(&self) -> Vec<String>;
    /// ISF shader source (with its JSON header) by name.
    fn read_isf(&self, name: &str) -> Option<String>;
}
