//! audiovis - a live audio-reactive VJ visualizer.
//!
//! This file wires together the command line, logging and the application
//! entry point. The actual work lives in focused modules so each concern
//! (rendering, audio, control, web) can evolve independently.

mod cli;
mod version;

use anyhow::Result;
use clap::Parser;
use cli::Cli;

fn main() -> Result<()> {
    let args = Cli::parse();
    init_logging(&args.log);

    // Startup banner: prints the build identity so any running instance can be
    // traced back to its source commit.
    for line in version::banner().lines() {
        tracing::info!("{line}");
    }

    tracing::info!(
        backend = ?args.backend,
        size = format!("{}x{}", args.width, args.height),
        fps = args.fps,
        "configuration loaded"
    );

    // The engine itself is built out across subsequent milestones; for now the
    // binary validates its configuration and reports a clean startup.
    tracing::warn!("engine not yet wired up - this is the scaffold milestone");
    Ok(())
}

/// Initialise tracing. An explicit `RUST_LOG` always wins so power users can
/// target individual modules; otherwise the `--log` level applies crate-wide.
fn init_logging(level: &str) {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("audiovis={level},warn")));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
