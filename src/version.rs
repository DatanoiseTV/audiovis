//! Build identity, surfaced in the startup banner and `--version`.

/// Crate semantic version from Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Git branch this build came from (filled in by `build.rs`).
pub const GIT_BRANCH: &str = env!("AV_GIT_BRANCH");
/// Short git commit hash, with a trailing `+` if the tree was dirty.
pub const GIT_COMMIT: &str = env!("AV_GIT_COMMIT");

/// One-line build identifier, e.g. `0.1.0 (feature/scaffold a1b2c3d)`. clap
/// prepends the binary name, so this intentionally omits it.
pub fn long() -> String {
    format!("{VERSION} ({GIT_BRANCH} {GIT_COMMIT})")
}

/// Same as [`long`] but interned to a `'static` string, which is the shape
/// clap wants for its `version` attribute.
pub fn long_static() -> &'static str {
    use std::sync::OnceLock;
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(long)
}

/// Multi-line startup banner shown when the app launches.
pub fn banner() -> String {
    format!(
        "audiovis {VERSION}\n  build : {GIT_BRANCH} @ {GIT_COMMIT}\n  a live audio-reactive VJ visualizer"
    )
}
