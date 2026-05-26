//! Desktop window backend: winit for the window/event loop, glutin for the GL
//! context. We ask for OpenGL 2.1 so the shader dialect matches what the GLES2
//! boards run, keeping one shader code path across desktop preview and hardware.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentGlContext, PossiblyCurrentContext, Version};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

use std::sync::Arc;

use crate::audio::{AudioEngine, AudioShared};
use crate::cli::Cli;
use crate::control::midi::MidiInputs;
use crate::control::{ControlBus, ControlEvent};
use crate::engine::Engine;
use crate::params::ParamValue;
use crate::presets::PresetStore;
use crate::script::{ScriptAction, ScriptEngine, ScriptSignals, ScriptStore};
use crate::web::WebHandle;
use crate::render::gl::{self, Gl};
use crate::render::pipeline::Pipeline;
use crate::render::{FrameContext, GlslFlavor};

/// Run the desktop window backend. Blocks until the window is closed (or the
/// frame budget set by `--frames`/`--screenshot` is reached).
pub fn run(
    cli: Cli,
    engine: Engine,
    bus: ControlBus,
    audio: AudioEngine,
    midi: MidiInputs,
    web: Option<WebHandle>,
) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    // The renderer reads the shared feature block; the engine itself is held so
    // the device can be switched live.
    let audio_shared = audio.shared();

    let max_frames = if cli.frames > 0 {
        Some(cli.frames)
    } else if cli.screenshot.is_some() {
        // Render a couple of frames so animation has advanced, then capture.
        Some(2)
    } else {
        None
    };

    let mut app = WindowApp {
        cli,
        engine,
        bus,
        audio,
        audio_shared,
        midi,
        web,
        presets: PresetStore::new(),
        current_preset: String::new(),
        script: ScriptEngine::new(),
        scripts: ScriptStore::new(),
        script_buf: Vec::new(),
        wave_buf: Vec::new(),
        gfx: None,
        start: Instant::now(),
        last: Instant::now(),
        frame: 0,
        max_frames,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

/// Live GL state, created once the event loop resumes.
struct Gfx {
    window: Window,
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    gl: Gl,
    pipeline: Pipeline,
}

struct WindowApp {
    cli: Cli,
    engine: Engine,
    bus: ControlBus,
    /// Owns the capture stream so the input device can be switched live.
    audio: AudioEngine,
    /// Stable handle to the latest analysis result, survives a device switch.
    audio_shared: Arc<AudioShared>,
    /// Owns the MIDI connections so the hardware port can be switched live.
    midi: MidiInputs,
    web: Option<WebHandle>,
    presets: PresetStore,
    current_preset: String,
    /// The embedded JS scripting runtime and its store of saved/example scripts.
    script: ScriptEngine,
    scripts: ScriptStore,
    /// Reused buffer for the script's RGBA pixel surface.
    script_buf: Vec<u8>,
    /// Reused buffer for the latest stereo waveform (interleaved L,R).
    wave_buf: Vec<f32>,
    gfx: Option<Gfx>,
    start: Instant,
    last: Instant,
    frame: u64,
    max_frames: Option<u64>,
}

impl WindowApp {
    /// Build window, GL config, context, surface and pipeline. Runs on first
    /// resume (and again only if the platform tore the surface down).
    fn init(&mut self, el: &ActiveEventLoop) -> Result<()> {
        let attrs = Window::default_attributes()
            .with_title("audiovis")
            .with_inner_size(winit::dpi::LogicalSize::new(self.cli.width, self.cli.height));

        let template = ConfigTemplateBuilder::new().with_alpha_size(8);
        let display_builder = DisplayBuilder::new().with_window_attributes(Some(attrs));

        let (window, gl_config) = display_builder
            .build(el, template, |configs| {
                // Prefer the config with the most samples; fall back to first.
                configs
                    .reduce(|a, b| if b.num_samples() > a.num_samples() { b } else { a })
                    .expect("no GL configs")
            })
            .map_err(|e| anyhow!("GL display build failed: {e}"))?;
        let window = window.ok_or_else(|| anyhow!("no window from display builder"))?;

        let raw_handle = window.window_handle()?.as_raw();
        let gl_display = gl_config.display();

        // macOS only grants Core profile contexts, so target GL 3.3 Core; the
        // shader macro layer keeps the bodies identical to the GLES2 boards.
        let ctx_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .with_profile(GlProfile::Core)
            .build(Some(raw_handle));
        let not_current = unsafe { gl_display.create_context(&gl_config, &ctx_attrs) }
            .map_err(|e| anyhow!("create GL context failed: {e}"))?;

        let surface_attrs = window
            .build_surface_attributes(SurfaceAttributesBuilder::<WindowSurface>::new())
            .map_err(|e| anyhow!("surface attrs failed: {e}"))?;
        let surface = unsafe { gl_display.create_window_surface(&gl_config, &surface_attrs) }
            .map_err(|e| anyhow!("create surface failed: {e}"))?;

        let context = not_current
            .make_current(&surface)
            .map_err(|e| anyhow!("make current failed: {e}"))?;

        let gl: Gl = Rc::new(unsafe {
            glow::Context::from_loader_function_cstr(|s| gl_display.get_proc_address(s).cast())
        });

        // Vsync when possible; harmless if the platform ignores it.
        let _ = surface.set_swap_interval(&context, SwapInterval::Wait(NonZeroU32::new(1).unwrap()));

        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        let pipeline = Pipeline::new(&gl, GlslFlavor::GlCore, &mut self.engine, w, h, self.cli.render_scale)
            .map_err(|e| anyhow!("pipeline init failed: {e}"))?;

        // The pipeline has now registered all layer/effect parameters, so a
        // preset can safely resolve their paths. Priority: an explicit
        // --preset file, else the last-used preset, else the "init" builtin.
        if let Some(path) = self.cli.preset.clone() {
            if let Err(e) = self.engine.load_preset(&path) {
                tracing::warn!("could not load preset {path}: {e:#}");
            }
        } else {
            let startup = self.presets.last().filter(|n| self.presets.load(n).is_some());
            let name = startup.unwrap_or_else(|| "init".to_string());
            self.load_preset_named(&name);
        }

        // Publish the now-complete parameter schema + preset list to the web UI.
        if let Some(web) = &self.web {
            // Generators and simulations share the layer.N.generator index space.
            let mut generators: Vec<String> = crate::render::generators::GENERATORS.iter().map(|g| g.name.to_string()).collect();
            generators.extend(crate::render::sim::SIMS.iter().map(|s| s.name.to_string()));
            web.set_schema(&self.engine, generators, pipeline.media_names(), pipeline.mesh_names(), pipeline.isf_names());
            web.publish_presets(self.presets.list(), &self.current_preset);
            web.publish_text(self.engine.text_slots());
            web.publish_mappings(self.engine.mappings_list());
            web.publish_scripts(self.scripts.list());
        }
        self.publish_devices();

        tracing::info!("window backend up: {}x{} GL 3.3 Core via {}", w, h, renderer_name(&gl));

        self.gfx = Some(Gfx { window, surface, context, gl, pipeline });
        Ok(())
    }

    /// Compile a new script source and report any error to the web UI.
    fn apply_script(&mut self, source: &str) {
        let err = self.script.set_source(source).err().unwrap_or_default();
        if !err.is_empty() {
            tracing::warn!("script error: {err}");
        }
        if let Some(web) = &self.web {
            web.publish_script_error(&err);
        }
    }

    /// Publish the available + selected audio/MIDI input devices to the web UI.
    fn publish_devices(&self) {
        if let Some(web) = &self.web {
            web.publish_devices(
                AudioEngine::list_devices(),
                self.audio.current_device(),
                MidiInputs::list_ports(),
                self.midi.current_filter(),
            );
        }
    }

    /// Load a preset by name (user file or builtin), remember it as the last,
    /// and tell the web UI which one is current.
    fn load_preset_named(&mut self, name: &str) {
        match self.presets.load(name) {
            Some(preset) => {
                self.engine.apply_preset(&preset);
                self.presets.set_last(name);
                self.current_preset = name.to_string();
                tracing::info!("loaded preset '{name}'");
                if let Some(web) = &self.web {
                    web.publish_presets(self.presets.list(), &self.current_preset);
                }
            }
            None => tracing::warn!("preset '{name}' not found"),
        }
    }

    /// Save the current state as a named user preset and refresh the list.
    fn save_preset_named(&mut self, name: &str) {
        let preset = self.engine.to_preset(name);
        match self.presets.save(name, &preset) {
            Ok(()) => {
                self.presets.set_last(name);
                self.current_preset = name.to_string();
                tracing::info!("saved preset '{name}'");
                if let Some(web) = &self.web {
                    web.publish_presets(self.presets.list(), &self.current_preset);
                }
            }
            Err(e) => tracing::warn!("could not save preset '{name}': {e:#}"),
        }
    }

    /// One rendered frame: pump control input, advance time, draw, present.
    fn draw(&mut self, el: &ActiveEventLoop) {
        // Drain queued control events into the authoritative engine state.
        // Preset load/save are resolved by name against the preset store rather
        // than the engine's path-based handling.
        let events: Vec<_> = self.bus.drain().collect();
        for ev in events {
            match ev {
                ControlEvent::LoadPreset(name) => self.load_preset_named(&name),
                ControlEvent::SavePreset(name) => self.save_preset_named(&name),
                ControlEvent::SetAudioDevice(name) => {
                    tracing::info!("switching audio input to '{}'", if name.is_empty() { "default" } else { &name });
                    self.audio.set_device(&name);
                    self.publish_devices();
                }
                ControlEvent::SetMidiPort(name) => {
                    tracing::info!("switching MIDI input to '{}'", if name.is_empty() { "all" } else { &name });
                    self.midi.set_port(&name);
                    self.publish_devices();
                }
                ControlEvent::RescanMedia => {
                    if let Some(gfx) = self.gfx.as_mut() {
                        gfx.pipeline.rescan_media();
                        let names = gfx.pipeline.media_names();
                        let meshes = gfx.pipeline.mesh_names();
                        let isf = gfx.pipeline.isf_names();
                        tracing::info!("rescanned: {} media, {} mesh, {} isf", names.len().saturating_sub(1), meshes.len().saturating_sub(1), isf.len().saturating_sub(1));
                        if let Some(web) = &self.web {
                            web.publish_media(names, meshes, isf);
                        }
                    }
                }
                ControlEvent::SetScript(source) => self.apply_script(&source),
                ControlEvent::SaveScript { name, source } => {
                    if let Err(e) = self.scripts.save(&name, &source) {
                        tracing::warn!("script '{name}' save failed: {e}");
                    } else {
                        tracing::info!("saved script '{name}'");
                    }
                    self.apply_script(&source);
                    if let Some(web) = &self.web {
                        web.publish_scripts(self.scripts.list());
                    }
                }
                ControlEvent::LoadScript(name) => {
                    if let Some(source) = self.scripts.load(&name) {
                        self.apply_script(&source);
                        if let Some(web) = &self.web {
                            web.publish_script_source(&source);
                        }
                    } else {
                        tracing::warn!("script '{name}' not found");
                    }
                }
                other => self.engine.handle(other),
            }
        }

        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32();
        self.last = now;
        let time = now.duration_since(self.start).as_secs_f32();

        // Advance the musical clock so tempo-synced LFOs and clock phases move.
        self.engine.tick_clock(dt);

        // Push the live analyzer controls from the audio.* params, so tuning
        // them in the UI retunes the response without restarting capture.
        {
            let p = self.engine.params();
            let read = |path: &str, d: f32| p.id_of(path).map(|id| p.get_f32(id)).unwrap_or(d);
            self.audio_shared.set_controls(
                read("audio.gain", 1.0),
                read("audio.attack", 0.99),
                read("audio.release", 0.5),
                read("audio.sensitivity", 1.6),
            );
        }

        // Feed the latest audio energies to the generators.
        let (low, mid, high) = self.audio_shared.bands();
        let beat = self.audio_shared.beat();
        self.audio_shared.copy_waveform(&mut self.wave_buf);
        let rms = self.audio_shared.rms();

        // Modulation pass: assemble the signal sources and route them onto
        // parameters before anything reads the params for rendering.
        let sources = build_mod_sources(&self.engine, low, mid, high, rms, beat);

        // JS script: read the live signals, drive base params and the 2D buffer
        // before modulation layers its offsets on top.
        if self.script.has_script() {
            let lfo = |n: usize| sources.get(&format!("lfo.{n}")).copied().unwrap_or(0.0);
            let sig = {
                let p = self.engine.params();
                let readf = |path: &str, d: f32| p.id_of(path).map(|id| p.get_f32(id)).unwrap_or(d);
                ScriptSignals {
                    t: time,
                    dt,
                    frame: self.frame as f64,
                    low,
                    mid,
                    high,
                    rms,
                    onset: beat,
                    beat: readf("clock.beat", 0.0),
                    bar: readf("clock.bar", 0.0),
                    bpm: readf("clock.bpm", 120.0),
                    lfos: [lfo(1), lfo(2), lfo(3), lfo(4), lfo(5), lfo(6)],
                }
            };
            let outcome = self.script.run(&sig, self.engine.params());
            for action in outcome.actions {
                match action {
                    ScriptAction::Set(path, v) => self.engine.handle(ControlEvent::SetParam { path, value: ParamValue::Float(v) }),
                    ScriptAction::SetNorm(path, norm) => self.engine.handle(ControlEvent::SetParamNorm { path, norm }),
                    ScriptAction::Trigger(path) => self.engine.handle(ControlEvent::Trigger { path }),
                }
            }
            if outcome.buffer_used {
                self.script.buffer(&mut self.script_buf);
                if let Some(gfx) = self.gfx.as_ref() {
                    gfx.pipeline.set_script_buffer(&self.script_buf);
                }
            }
            if let Some(err) = outcome.error {
                if let Some(web) = &self.web {
                    web.publish_script_error(&err);
                }
            }
        }

        self.engine.apply_modulation(&sources);

        // Push state to the web UI: param changes every frame, telemetry at a
        // calmer rate. Always drain notices so they cannot accumulate.
        let notices = self.engine.take_notices();
        if let Some(web) = &self.web {
            if !notices.is_empty() {
                web.publish_notices(&notices);
            }
            if self.frame % 4 == 0 {
                let p = self.engine.params();
                let read = |path: &str, dflt: f32| p.id_of(path).map(|id| p.get_f32(id)).unwrap_or(dflt);
                web.publish_telemetry(low, mid, high, rms, beat, read("clock.bpm", 120.0), read("clock.beat", 0.0), read("clock.bar", 0.0), self.engine.musical_beats() as f32);
                web.publish_mod_routes(&self.engine);
                web.publish_text(self.engine.text_slots());
                web.publish_mappings(self.engine.mappings_list());
            }
        }

        let Some(gfx) = self.gfx.as_mut() else { return };
        gfx.pipeline.set_audio(low, mid, high, beat);
        gfx.pipeline.set_waveform(&self.wave_buf);
        let size = gfx.window.inner_size();
        let fc = FrameContext {
            time,
            dt,
            width: size.width.max(1),
            height: size.height.max(1),
            frame: self.frame,
        };

        gfx.pipeline.render(&fc, &self.engine);
        self.engine.end_frame();

        // When an ISF shader (re)loads, publish its inputs + any compile error.
        if let Some((error, inputs)) = gfx.pipeline.isf_take_dirty() {
            if let Some(web) = &self.web {
                web.publish_isf_inputs(inputs, &error);
            }
        }

        // Stream a downscaled live preview to the web monitor at ~10 fps.
        if self.frame % 6 == 0 {
            if let Some(web) = &self.web {
                let rgba = gfx.pipeline.read_preview();
                let mut jpeg = Vec::new();
                let enc = jpeg_encoder::Encoder::new(&mut jpeg, 55);
                if enc
                    .encode(&rgba, crate::render::pipeline::PREVIEW_W as u16, crate::render::pipeline::PREVIEW_H as u16, jpeg_encoder::ColorType::Rgba)
                    .is_ok()
                {
                    web.publish_preview(jpeg);
                }
            }
        }

        // Capture before presenting so we read the freshly drawn back buffer.
        let last_frame = self.max_frames.map(|m| self.frame + 1 >= m).unwrap_or(false);
        if last_frame {
            if let Some(path) = self.cli.screenshot.clone() {
                let pixels = gl::read_rgba(&gfx.gl, fc.width as i32, fc.height as i32);
                match super::save_ppm(&path, fc.width, fc.height, &pixels) {
                    Ok(()) => tracing::info!("wrote screenshot {path}"),
                    Err(e) => tracing::warn!("screenshot failed: {e}"),
                }
            }
        }

        if let Err(e) = gfx.surface.swap_buffers(&gfx.context) {
            tracing::warn!("swap_buffers failed: {e}");
        }

        self.frame += 1;
        if self.max_frames.map(|m| self.frame >= m).unwrap_or(false) {
            tracing::info!("frame budget reached ({} frames), exiting", self.frame);
            el.exit();
        }
    }
}

impl ApplicationHandler for WindowApp {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_none() {
            if let Err(e) = self.init(el) {
                tracing::error!("backend init failed: {e:#}");
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::KeyboardInput {
                event: KeyEvent { logical_key, state: ElementState::Pressed, .. },
                ..
            } => match logical_key {
                Key::Named(NamedKey::Escape) => el.exit(),
                // 'f' toggles borderless fullscreen on the monitor the window is
                // currently on, so it can be dragged to an external display first.
                Key::Character(s) if s.eq_ignore_ascii_case("f") => {
                    if let Some(gfx) = self.gfx.as_ref() {
                        let next = match gfx.window.fullscreen() {
                            Some(_) => None,
                            None => Some(Fullscreen::Borderless(None)),
                        };
                        gfx.window.set_fullscreen(next);
                    }
                }
                _ => {}
            },
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
                        gfx.surface.resize(&gfx.context, w, h);
                        gfx.pipeline.resize(size.width, size.height);
                    }
                }
            }
            WindowEvent::RedrawRequested => self.draw(el),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // Pace to the target fps. Only request a redraw once a frame is actually
        // due, then sleep until the next one - requesting every wake would wake
        // the loop immediately and defeat the WaitUntil, free-running at hundreds
        // of fps (wasted CPU, flooded web feed; bad on the Pi).
        let fps = self.cli.fps.max(1);
        let frame = std::time::Duration::from_secs_f32(1.0 / fps as f32);
        let next = self.last + frame;
        if std::time::Instant::now() >= next {
            if let Some(gfx) = self.gfx.as_ref() {
                gfx.window.request_redraw();
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(next));
    }
}

/// Best-effort GL_RENDERER string for the startup log.
fn renderer_name(gl: &Gl) -> String {
    use glow::HasContext;
    unsafe { gl.get_parameter_string(glow::RENDERER) }
}

/// Assemble the modulation source values for this frame: audio bands (0..1),
/// the beat clock phase, and the tempo-synced LFOs (bipolar -1..1).
fn build_mod_sources(engine: &Engine, low: f32, mid: f32, high: f32, rms: f32, beat: f32) -> std::collections::HashMap<String, f32> {
    use crate::engine::LFO_DIVISIONS;
    let mut s = std::collections::HashMap::new();
    s.insert("audio.low".into(), low);
    s.insert("audio.mid".into(), mid);
    s.insert("audio.high".into(), high);
    s.insert("audio.rms".into(), rms);
    s.insert("audio.level".into(), rms);
    s.insert("audio.beat".into(), beat);

    let p = engine.params();
    let read = |path: &str| p.id_of(path).map(|id| p.get_f32(id)).unwrap_or(0.0);
    s.insert("clock.beat".into(), read("clock.beat"));
    s.insert("clock.bar".into(), read("clock.bar"));

    // LFOs run off the musical position so they stay locked to the measure. A
    // disabled LFO contributes nothing (its source value is 0).
    let beats = engine.musical_beats();
    for n in 1..=8 {
        let enabled = p.id_of(&format!("lfo.{n}.enable")).map(|id| p.get_bool(id)).unwrap_or(false);
        let value = if enabled {
            let div_idx = p.id_of(&format!("lfo.{n}.div")).map(|id| p.get(id).as_i64()).unwrap_or(3);
            let shape = p.id_of(&format!("lfo.{n}.shape")).map(|id| p.get(id).as_i64()).unwrap_or(0);
            let bpc = LFO_DIVISIONS.get(div_idx as usize).copied().unwrap_or(4.0) as f64;
            let phase = (beats / bpc) as f32;
            lfo(shape, phase)
        } else {
            0.0
        };
        s.insert(format!("lfo.{n}"), value);
    }
    s
}

/// Deterministic per-cycle random in -1..1 (synced sample-and-hold).
fn cycle_rand(cycle: f32) -> f32 {
    ((cycle * 12.9898).sin() * 43758.547).fract() * 2.0 - 1.0
}

/// One LFO sample. `phase` is the musical position / division; bipolar -1..1.
/// Shapes match the UI: sine, triangle, saw up/down, square, pulse, rand,
/// smooth noise, steps.
pub fn lfo(shape: i64, phase: f32) -> f32 {
    let f = phase.rem_euclid(1.0);
    match shape {
        1 => 4.0 * (f - 0.5).abs() - 1.0,             // triangle
        2 => 2.0 * f - 1.0,                             // saw up (ramp)
        3 => 1.0 - 2.0 * f,                             // saw down
        4 => if f < 0.5 { 1.0 } else { -1.0 },          // square
        5 => if f < 0.25 { 1.0 } else { -1.0 },         // pulse (25%)
        6 => cycle_rand(phase.floor()),                 // sample & hold (synced random)
        7 => {
            // Smooth value-noise random, interpolated between cycle samples.
            let c = phase.floor();
            let t = f * f * (3.0 - 2.0 * f);
            cycle_rand(c) * (1.0 - t) + cycle_rand(c + 1.0) * t
        }
        8 => (f * 8.0).floor() / 3.5 - 1.0,             // 8-step staircase
        _ => (std::f32::consts::TAU * f).sin(),         // sine
    }
}
