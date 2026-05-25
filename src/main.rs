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
mod presets;
mod render;
mod version;
mod web;

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

    // Lettering bank: a trigger per slot (bind these to MIDI notes), a clear,
    // and style knobs. The slot text itself is edited in the web UI.
    for n in 0..8 {
        p.register(ParamSpec::new(
            format!("text.{n}.trigger"),
            format!("Show {}", n + 1),
            "Text",
            ParamKind::Trigger,
        ));
    }
    p.register(ParamSpec::new("text.clear", "Clear", "Text", ParamKind::Trigger));
    p.register(ParamSpec::new("text.size", "Size", "Text", ParamKind::Float { min: 0.02, max: 0.4, default: 0.1 }));
    p.register(ParamSpec::new("text.posx", "Pos X", "Text", ParamKind::Float { min: -1.0, max: 1.0, default: 0.0 }));
    p.register(ParamSpec::new("text.posy", "Pos Y", "Text", ParamKind::Float { min: -1.0, max: 1.0, default: 0.0 }));
    p.register(ParamSpec::new("text.hue", "Hue", "Text", ParamKind::Float { min: 0.0, max: 1.0, default: 0.0 }));
    p.register(ParamSpec::new("text.font", "Font", "Text", ParamKind::Int { min: 0, max: 3, default: 0 }));
    // fx: 0 none, 1 dissolve, 2 wave, 3 tear, 4 scanlines
    p.register(ParamSpec::new("text.fx", "FX", "Text", ParamKind::Int { min: 0, max: 4, default: 0 }));
    p.register(ParamSpec::new("text.fxamt", "FX amt", "Text", ParamKind::Float { min: 0.0, max: 1.0, default: 0.5 }));

    // Tempo-synced LFOs, available as modulation sources (lfo.1 .. lfo.3). The
    // rate is a musical division of the measure, not free Hz, so they always
    // lock to the beat clock. Defaults: 1 bar, 1/2, 1/4.
    let div_defaults = [3, 4, 5];
    for n in 1..=3 {
        let g = "LFO";
        // div: index into LFO_DIVISIONS (8 bars .. 1/16).
        p.register(ParamSpec::new(
            format!("lfo.{n}.div"),
            format!("LFO {n} div"),
            g,
            ParamKind::Int { min: 0, max: 7, default: div_defaults[n - 1] },
        ));
        // shape: 0 sine, 1 triangle, 2 saw, 3 square, 4 sample-and-hold
        p.register(ParamSpec::new(
            format!("lfo.{n}.shape"),
            format!("LFO {n} shape"),
            g,
            ParamKind::Int { min: 0, max: 4, default: 0 },
        ));
    }
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
