//! Shared frame driver: the backend-agnostic half of the render loop.
//!
//! Both the desktop window backend and the Linux DRM backend create their own
//! GL context and present mechanism, but the work *between* frames is identical:
//! drain control input, advance the musical clock, run modulation + scripts,
//! publish state to the web UI, and feed the pipeline this frame's audio and
//! camera. That logic lives here so the two backends differ only in how a
//! rendered frame reaches the display.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::audio::{AudioEngine, AudioShared};
use crate::cli::Cli;
use crate::control::midi::MidiInputs;
use crate::control::{ControlBus, ControlEvent};
use audiovis_render_core::engine::{Engine, LFO_DIVISIONS};
use crate::link::LinkEngine;
use audiovis_render_core::params::ParamValue;
use crate::presets::PresetStore;
use audiovis_render_core::pipeline::Pipeline;
use crate::script::{ScriptAction, ScriptEngine, ScriptSignals, ScriptStore};
use crate::video::{VideoEngine, VideoShared};
use crate::web::WebHandle;

/// Owns every piece of per-run state that is not the GL context: the engine,
/// the input devices, the web surface, and the preset/script stores. A backend
/// constructs one, builds its own pipeline, then calls [`Driver::prepare_frame`]
/// / [`Driver::finish_frame`] around its own present step each frame.
pub struct Driver {
    pub cli: Cli,
    pub engine: Engine,
    bus: ControlBus,
    /// Owns the capture stream so the input device can be switched live.
    audio: AudioEngine,
    /// Stable handle to the latest analysis result, survives a device switch.
    audio_shared: Arc<AudioShared>,
    /// Owns the MIDI connections so the hardware port can be switched live.
    midi: MidiInputs,
    /// Owns the camera capture so the device can be switched live.
    video: VideoEngine,
    video_shared: Arc<VideoShared>,
    last_video_seq: u64,
    /// Ableton Link tempo sync (drives the beat clock when enabled).
    link: LinkEngine,
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
    start: Instant,
    last: Instant,
    frame: u64,
    max_frames: Option<u64>,
}

impl Driver {
    pub fn new(
        cli: Cli,
        engine: Engine,
        bus: ControlBus,
        audio: AudioEngine,
        midi: MidiInputs,
        video: VideoEngine,
        web: Option<WebHandle>,
    ) -> Self {
        let audio_shared = audio.shared();
        let video_shared = video.shared();
        let mut link = LinkEngine::new();
        link.set_enabled(cli.link);

        let max_frames = if cli.frames > 0 {
            Some(cli.frames)
        } else if cli.screenshot.is_some() {
            // Render a couple of frames so animation has advanced, then capture.
            Some(2)
        } else {
            None
        };

        Self {
            cli,
            engine,
            bus,
            audio,
            audio_shared,
            midi,
            video,
            video_shared,
            last_video_seq: 0,
            link,
            web,
            presets: PresetStore::new(),
            current_preset: String::new(),
            script: ScriptEngine::new(),
            scripts: ScriptStore::new(),
            script_buf: Vec::new(),
            wave_buf: Vec::new(),
            start: Instant::now(),
            last: Instant::now(),
            frame: 0,
            max_frames,
        }
    }

    /// Monotonic frame counter (pre-increment value of the frame in flight).
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Target frames per second from the CLI (at least 1).
    pub fn fps(&self) -> u32 {
        self.cli.fps.max(1)
    }

    /// Timestamp of the most recent frame start, for the window backend's
    /// frame-pacing `WaitUntil`. (The DRM backend paces on the vblank event.)
    pub fn last_present(&self) -> Instant {
        self.last
    }

    /// The startup preset has now-complete parameter paths to resolve against,
    /// so this runs once, right after the pipeline is built. It loads the
    /// startup preset (explicit `--preset`, else last-used, else `init`) and
    /// publishes the full schema + device list to the web UI.
    pub fn on_pipeline_ready(&mut self, pipeline: &Pipeline) {
        if let Some(path) = self.cli.preset.clone() {
            if let Err(e) = self.engine.load_preset(&path) {
                tracing::warn!("could not load preset {path}: {e:#}");
            }
        } else {
            let startup = self.presets.last().filter(|n| self.presets.load(n).is_some());
            let name = startup.unwrap_or_else(|| "init".to_string());
            self.load_preset_named(&name);
        }

        if let Some(web) = &self.web {
            // Generators and simulations share the layer.N.generator index space.
            let mut generators: Vec<String> = audiovis_render_core::generators::GENERATORS.iter().map(|g| g.name.to_string()).collect();
            generators.extend(audiovis_render_core::sim::SIMS.iter().map(|s| s.name.to_string()));
            web.set_schema(&self.engine, generators, pipeline.media_names(), pipeline.mesh_names(), pipeline.isf_names());
            web.publish_presets(self.presets.list(), &self.current_preset);
            web.publish_text(self.engine.text_slots());
            web.publish_mappings(self.engine.mappings_list());
            web.publish_scripts(self.scripts.list());
        }
        self.publish_devices();
    }

    /// Publish the available + selected audio/MIDI/camera input devices.
    fn publish_devices(&self) {
        if let Some(web) = &self.web {
            web.publish_devices(
                AudioEngine::list_devices(),
                self.audio.current_device(),
                MidiInputs::list_ports(),
                self.midi.current_filter(),
                VideoEngine::list_devices(),
                self.video.current_device(),
            );
        }
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

    /// Drain control input, advance the clock + modulation + script, publish
    /// per-frame web state, and feed the pipeline this frame's audio/camera.
    /// Returns the `(time, dt)` the backend needs to build its [`FrameContext`].
    pub fn prepare_frame(&mut self, pipeline: &mut Pipeline) -> (f32, f32) {
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
                ControlEvent::SetVideoDevice(name) => {
                    tracing::info!("switching camera to '{}'", if name.is_empty() { "default" } else { &name });
                    self.video.set_device(&name);
                    self.publish_devices();
                }
                ControlEvent::SetLink(on) => self.link.set_enabled(on),
                ControlEvent::RescanMedia => {
                    pipeline.rescan_media();
                    let names = pipeline.media_names();
                    let meshes = pipeline.mesh_names();
                    let isf = pipeline.isf_names();
                    tracing::info!("rescanned: {} media, {} mesh, {} isf", names.len().saturating_sub(1), meshes.len().saturating_sub(1), isf.len().saturating_sub(1));
                    if let Some(web) = &self.web {
                        web.publish_media(names, meshes, isf);
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

        // If Ableton Link is on, let the session drive the beat clock.
        match self.link.state(4.0) {
            Some((bpm, beats)) => self.engine.sync_link(beats, bpm),
            None => self.engine.clear_link(),
        }
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
                pipeline.set_script_buffer(&self.script_buf);
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
                web.publish_telemetry(low, mid, high, rms, beat, read("clock.bpm", 120.0), read("clock.beat", 0.0), read("clock.bar", 0.0), self.engine.musical_beats() as f32, self.link.peers() as i32, self.link.enabled());
                web.publish_mod_routes(&self.engine);
                web.publish_text(self.engine.text_slots());
                web.publish_mappings(self.engine.mappings_list());
            }
        }

        // Feed the pipeline the per-frame inputs it samples in shaders.
        pipeline.set_audio(low, mid, high, beat);
        pipeline.set_waveform(&self.wave_buf);
        // Upload the latest camera frame (only when a new one has arrived).
        let vseq = self.video_shared.seq();
        if vseq != self.last_video_seq {
            self.last_video_seq = vseq;
            self.video_shared.with_frame(|f| pipeline.set_camera_frame(f.w, f.h, &f.rgba));
        }

        (time, dt)
    }

    /// After the pipeline has rendered into the back buffer: advance engine
    /// state and publish the ISF inputs + the live web preview. Call this
    /// *before* the backend presents, then [`Driver::advance_frame`].
    pub fn finish_frame(&mut self, pipeline: &mut Pipeline) {
        self.engine.end_frame();

        // When an ISF shader (re)loads, publish its inputs + any compile error.
        if let Some((error, inputs)) = pipeline.isf_take_dirty() {
            if let Some(web) = &self.web {
                web.publish_isf_inputs(inputs, &error);
            }
        }

        // Stream a downscaled live preview to the web monitor at ~10 fps.
        if self.frame % 6 == 0 {
            if let Some(web) = &self.web {
                let rgba = pipeline.read_preview();
                let mut jpeg = Vec::new();
                let enc = jpeg_encoder::Encoder::new(&mut jpeg, 55);
                if enc
                    .encode(&rgba, audiovis_render_core::pipeline::PREVIEW_W as u16, audiovis_render_core::pipeline::PREVIEW_H as u16, jpeg_encoder::ColorType::Rgba)
                    .is_ok()
                {
                    web.publish_preview(jpeg);
                }
            }
        }
    }

    /// If this is the final frame and `--screenshot` was given, the path to
    /// write. The backend reads the freshly drawn buffer and saves it.
    pub fn screenshot_path(&self) -> Option<String> {
        let last = self.max_frames.map(|m| self.frame + 1 >= m).unwrap_or(false);
        if last {
            self.cli.screenshot.clone()
        } else {
            None
        }
    }

    /// Advance the frame counter; returns `true` once the `--frames` budget is
    /// spent and the backend should exit.
    pub fn advance_frame(&mut self) -> bool {
        self.frame += 1;
        let done = self.max_frames.map(|m| self.frame >= m).unwrap_or(false);
        if done {
            tracing::info!("frame budget reached ({} frames), exiting", self.frame);
        }
        done
    }
}

/// Assemble the modulation source values for this frame: audio bands (0..1),
/// the beat clock phase, and the tempo-synced LFOs (bipolar -1..1).
fn build_mod_sources(engine: &Engine, low: f32, mid: f32, high: f32, rms: f32, beat: f32) -> HashMap<String, f32> {
    let mut s = HashMap::new();
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
