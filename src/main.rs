//! audiovis - a live audio-reactive VJ visualizer.
//!
//! This file wires together the command line, logging and the application
//! entry point. The actual work lives in focused modules so each concern
//! (rendering, audio, control, web) can evolve independently.

mod app;
mod audio;
mod cli;
mod config;
mod control;
mod engine;
mod params;
mod render;
mod version;

use anyhow::Result;
use clap::Parser;
use cli::Cli;
use control::ControlBus;
use engine::Engine;

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

    // Stand up the core: the control bus that every input feeds, and the engine
    // that owns the authoritative parameter state. The render loop and input
    // sources attach to these in later milestones.
    let bus = ControlBus::new();
    let mut engine = Engine::new();
    seed_demo_params(&mut engine);

    // Note: a startup preset is applied by the backend *after* the render
    // pipeline registers its parameters, otherwise the layer/effect paths it
    // references would not exist yet.
    tracing::info!(params = engine.params().len(), "core engine ready");

    // Hand off to the selected output backend; it owns the frame loop and
    // drains the control bus each frame.
    app::run(args, engine, bus)
}

/// Register a small set of global parameters so the engine has a real surface to
/// work with before the generators land. These are the kind of top-level knobs a
/// performer always wants on a fader.
fn seed_demo_params(engine: &mut Engine) {
    use params::{ParamKind, ParamSpec};
    let p = engine.params_mut();
    p.register(ParamSpec::new(
        "global.brightness",
        "Brightness",
        "Global",
        ParamKind::Float { min: 0.0, max: 1.0, default: 1.0 },
    ));
    p.register(ParamSpec::new(
        "global.crossfade",
        "Crossfade",
        "Global",
        ParamKind::Float { min: 0.0, max: 1.0, default: 0.0 },
    ));
    p.register(ParamSpec::new(
        "global.speed",
        "Speed",
        "Global",
        ParamKind::Float { min: 0.0, max: 4.0, default: 1.0 },
    ));

    // Beat-clock telemetry, written from incoming MIDI clock.
    p.register(ParamSpec::new(
        "clock.bpm",
        "BPM",
        "Clock",
        ParamKind::Float { min: 20.0, max: 300.0, default: 120.0 },
    ));
    p.register(ParamSpec::new(
        "clock.beat",
        "Beat phase",
        "Clock",
        ParamKind::Float { min: 0.0, max: 1.0, default: 0.0 },
    ));
    p.register(ParamSpec::new(
        "clock.bar",
        "Bar phase",
        "Clock",
        ParamKind::Float { min: 0.0, max: 1.0, default: 0.0 },
    ));
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
