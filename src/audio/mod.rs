//! Live audio capture and the shared feature block the renderer reads.
//!
//! cpal captures the input device on its own callback thread and pushes mono
//! samples into a ring. A dedicated analysis thread runs the FFT
//! ([`analysis::Analyzer`]), smooths the result with an envelope follower and
//! publishes it into [`AudioShared`], which the render loop reads each frame.
//!
//! If no input device is available (a common headless case) the engine starts
//! in an inactive state and simply reports silence, so nothing downstream has
//! to special-case the absence of audio.

pub mod analysis;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use analysis::Analyzer;

/// FFT window size. 1024 at ~44.1 kHz is ~23 ms - responsive without being noisy.
const WINDOW: usize = 1024;

/// Below this RMS the input is treated as silence (avoids the auto-gain
/// amplifying noise floor into visible flicker).
const MIN_VOLUME: f32 = 1e-4;

/// An `f32` stored atomically (bit-cast through a `u32`). Plenty for telemetry
/// where the renderer just wants the latest value, lock-free.
#[derive(Default)]
struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
    fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

/// The latest analysis result, shared between the analysis thread and the
/// renderer. All fields are normalised to roughly 0..1.
/// Recent stereo waveform window length (frames). Drives the scope generator.
pub const WAVE: usize = 256;

#[derive(Default)]
pub struct AudioShared {
    low: AtomicF32,
    mid: AtomicF32,
    high: AtomicF32,
    rms: AtomicF32,
    /// A 0..1 value that spikes on detected onsets and decays.
    beat: AtomicF32,
    active: AtomicBool,
    /// Recent stereo waveform, interleaved L,R (length WAVE*2).
    waveform: Mutex<Vec<f32>>,
    /// Live analyzer controls the analysis thread reads each pass, so the web UI
    /// can tune the response without restarting capture.
    gain: AtomicF32,
    attack: AtomicF32,
    release: AtomicF32,
    onset: AtomicF32,
}

impl AudioShared {
    /// Low/mid/high band energies.
    pub fn bands(&self) -> (f32, f32, f32) {
        (self.low.load(), self.mid.load(), self.high.load())
    }
    pub fn rms(&self) -> f32 {
        self.rms.load()
    }
    pub fn beat(&self) -> f32 {
        self.beat.load()
    }
    /// Whether a capture stream is actually running.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
    /// Copy the latest stereo waveform (interleaved L,R) into `out`.
    pub fn copy_waveform(&self, out: &mut Vec<f32>) {
        out.clear();
        if let Ok(w) = self.waveform.lock() {
            out.extend_from_slice(&w);
        }
    }

    /// Update the live analyzer controls (called by the render thread from the
    /// `audio.*` parameters). `attack`/`release` are envelope coefficients in
    /// 0..1; `onset` is the flux-over-average multiplier for beat detection.
    pub fn set_controls(&self, gain: f32, attack: f32, release: f32, onset: f32) {
        self.gain.store(gain);
        self.attack.store(attack);
        self.release.store(release);
        self.onset.store(onset);
    }
}

/// Owns the capture stream and analysis thread. Drop it to stop cleanly.
pub struct AudioEngine {
    shared: Arc<AudioShared>,
    stop: Arc<AtomicBool>,
    // Kept alive for the engine's lifetime; dropping stops capture. The cpal
    // stream is not `Send` on every backend, hence kept on the creating thread.
    _stream: Option<cpal::Stream>,
    analysis: Option<JoinHandle<()>>,
    /// Name of the device currently captured (empty = system default).
    device: String,
}

impl AudioEngine {
    /// Start capture+analysis. `device_name` empty selects the default input.
    /// `gain` scales raw band energy before the saturating 0..1 mapping.
    pub fn start(device_name: &str, gain: f32) -> Self {
        let shared = Arc::new(AudioShared::default());
        // Seed the live controls; the render thread overwrites these from the
        // `audio.*` parameters once it is up.
        shared.set_controls(gain, 0.99, 0.5, 1.6);
        let stop = Arc::new(AtomicBool::new(false));

        let mut engine =
            AudioEngine { shared, stop, _stream: None, analysis: None, device: device_name.to_string() };
        engine.open(device_name);
        engine
    }

    pub fn shared(&self) -> Arc<AudioShared> {
        self.shared.clone()
    }

    /// The device currently selected (empty string means system default).
    pub fn current_device(&self) -> &str {
        &self.device
    }

    /// List the names of available input devices (for the web UI dropdown).
    pub fn list_devices() -> Vec<String> {
        let host = cpal::default_host();
        let mut names = Vec::new();
        if let Ok(devices) = host.input_devices() {
            for d in devices {
                if let Ok(n) = d.name() {
                    names.push(n);
                }
            }
        }
        names
    }

    /// Switch to a different input device at runtime, reusing the shared feature
    /// block so the renderer's handle stays valid. Empty selects the default.
    pub fn set_device(&mut self, device_name: &str) {
        // Stop the current session first (drop stream, join thread).
        self.stop.store(true, Ordering::Relaxed);
        self._stream.take();
        if let Some(h) = self.analysis.take() {
            let _ = h.join();
        }
        self.shared.active.store(false, Ordering::Relaxed);
        self.stop = Arc::new(AtomicBool::new(false));
        self.device = device_name.to_string();
        self.open(device_name);
    }

    /// Open a capture session into the existing shared block.
    fn open(&mut self, device_name: &str) {
        match Self::try_start(device_name, &self.shared, &self.stop) {
            Ok((stream, handle)) => {
                self.shared.active.store(true, Ordering::Relaxed);
                self._stream = Some(stream);
                self.analysis = Some(handle);
            }
            Err(e) => {
                tracing::warn!("audio capture unavailable ({e:#}); running without audio reactivity");
                self._stream = None;
                self.analysis = None;
            }
        }
    }

    fn try_start(
        device_name: &str,
        shared: &Arc<AudioShared>,
        stop: &Arc<AtomicBool>,
    ) -> anyhow::Result<(cpal::Stream, JoinHandle<()>)> {
        let host = cpal::default_host();
        let device = pick_device(&host, device_name)?;
        let name = device.name().unwrap_or_else(|_| "?".into());
        let config = device.default_input_config()?;
        let sample_rate = config.sample_rate().0 as f32;
        let channels = config.channels() as usize;
        tracing::info!("audio input: {name} @ {sample_rate} Hz, {channels} ch");

        // Ring of recent interleaved-stereo samples shared with the analysis
        // thread (mono is derived for the FFT; L/R feed the scope).
        let ring: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::with_capacity(WINDOW * 4)));
        let ring_cb = ring.clone();

        let push = move |stereo: &[f32]| {
            let mut r = ring_cb.lock().unwrap();
            for &s in stereo {
                if r.len() >= WINDOW * 4 {
                    r.pop_front();
                }
                r.push_back(s);
            }
        };

        let err_fn = |e| tracing::warn!("audio stream error: {e}");
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config.into(), channels, push, err_fn)?,
            cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config.into(), channels, push, err_fn)?,
            cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config.into(), channels, push, err_fn)?,
            other => anyhow::bail!("unsupported sample format {other:?}"),
        };
        stream.play()?;

        let handle = spawn_analysis(sample_rate, ring, shared.clone(), stop.clone());
        Ok((stream, handle))
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self._stream.take(); // stop capture first
        if let Some(h) = self.analysis.take() {
            let _ = h.join();
        }
    }
}

fn pick_device(host: &cpal::Host, name: &str) -> anyhow::Result<cpal::Device> {
    if !name.is_empty() {
        for d in host.input_devices()? {
            if d.name().map(|n| n.contains(name)).unwrap_or(false) {
                return Ok(d);
            }
        }
        anyhow::bail!("no input device matching '{name}'");
    }
    host.default_input_device()
        .ok_or_else(|| anyhow::anyhow!("no default input device"))
}

/// Build an input stream for sample type `T`, mixing channels down to mono and
/// handing each block to `push`.
fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    mut push: impl FnMut(&[f32]) + Send + 'static,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> anyhow::Result<cpal::Stream>
where
    T: cpal::SizedSample + ToMono,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            // Emit interleaved L,R per frame (mono input duplicates to both).
            let ch = channels.max(1);
            let mut out = Vec::with_capacity(data.len() / ch * 2);
            for frame in data.chunks(ch) {
                let l = frame[0].to_mono();
                let r = if ch > 1 { frame[1].to_mono() } else { l };
                out.push(l);
                out.push(r);
            }
            push(&out);
        },
        err_fn,
        None,
    )?;
    Ok(stream)
}

/// Convert a cpal sample to a normalised `f32` in -1..1.
trait ToMono {
    fn to_mono(&self) -> f32;
}
impl ToMono for f32 {
    fn to_mono(&self) -> f32 {
        *self
    }
}
impl ToMono for i16 {
    fn to_mono(&self) -> f32 {
        *self as f32 / 32768.0
    }
}
impl ToMono for u16 {
    fn to_mono(&self) -> f32 {
        (*self as f32 - 32768.0) / 32768.0
    }
}

/// The analysis loop: take the most recent window, analyze, smooth, publish.
fn spawn_analysis(
    sample_rate: f32,
    ring: Arc<Mutex<VecDeque<f32>>>,
    shared: Arc<AudioShared>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("audio-analysis".into())
        .spawn(move || {
            let mut analyzer = Analyzer::new(WINDOW, sample_rate);
            let mut block = vec![0.0f32; WINDOW];
            // Smoothed band envelopes, a rolling auto-gain, and an adaptive
            // flux baseline (the gain idea is from audio-reactive-led-strip:
            // track the peak with a fast-rise/slow-decay follower and divide).
            let (mut env_low, mut env_mid, mut env_high, mut env_rms) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            let mut mel_gain = 1e-3f32;
            let mut flux_avg = 0.0f32;
            let mut beat = 0.0f32;

            let mut wave = vec![0.0f32; WAVE * 2];
            while !stop.load(Ordering::Relaxed) {
                let have = {
                    let r = ring.lock().unwrap();
                    if r.len() >= WINDOW * 2 {
                        // The ring is interleaved stereo; average pairs for the
                        // mono FFT block (most recent WINDOW frames, overlapping).
                        let start = r.len() - WINDOW * 2;
                        for i in 0..WINDOW {
                            block[i] = (r[start + 2 * i] + r[start + 2 * i + 1]) * 0.5;
                        }
                        // Snapshot the last WAVE stereo frames for the scope.
                        let ws = r.len().saturating_sub(WAVE * 2);
                        for (i, s) in r.iter().skip(ws).take(WAVE * 2).enumerate() {
                            wave[i] = *s;
                        }
                        if let Ok(mut w) = shared.waveform.lock() {
                            *w = wave.clone();
                        }
                        true
                    } else {
                        false
                    }
                };

                if have {
                    let f = analyzer.analyze(&block);
                    // Live controls, tunable from the web UI each pass.
                    let gain = shared.gain.load();
                    let attack = shared.attack.load().clamp(0.01, 1.0);
                    let release = shared.release.load().clamp(0.01, 1.0);
                    let onset = shared.onset.load().max(1.0);
                    let follow = |prev: f32, target: f32| exp_filter(prev, target, attack, release);

                    if f.rms < MIN_VOLUME {
                        // Treat as silence; let the envelopes decay to rest.
                        env_low = follow(env_low, 0.0);
                        env_mid = follow(env_mid, 0.0);
                        env_high = follow(env_high, 0.0);
                        env_rms = follow(env_rms, 0.0);
                    } else {
                        // Rolling auto-gain off the loudest current band.
                        let peak = f.low.max(f.mid).max(f.high);
                        mel_gain = exp_filter(mel_gain, peak, 0.99, 0.01).max(1e-4);
                        let normalize = |v: f32| (v / mel_gain * gain).clamp(0.0, 1.0);

                        env_low = follow(env_low, normalize(f.low));
                        env_mid = follow(env_mid, normalize(f.mid));
                        env_high = follow(env_high, normalize(f.high));
                        env_rms = follow(env_rms, (f.rms * 4.0 * gain).clamp(0.0, 1.0));
                    }

                    // Onset: flux clearly above its slow running average.
                    flux_avg = flux_avg * 0.95 + f.flux * 0.05;
                    if f.flux > flux_avg * onset + 1e-4 {
                        beat = 1.0;
                    }
                    beat *= 0.85;

                    shared.low.store(env_low);
                    shared.mid.store(env_mid);
                    shared.high.store(env_high);
                    shared.rms.store(env_rms);
                    shared.beat.store(beat);
                }

                std::thread::sleep(std::time::Duration::from_millis(8));
            }
        })
        .expect("spawn audio analysis thread")
}

/// Asymmetric exponential filter: `prev + alpha*(target-prev)`, with a separate
/// alpha for rising vs falling - the ExpFilter from audio-reactive-led-strip.
fn exp_filter(prev: f32, target: f32, alpha_rise: f32, alpha_decay: f32) -> f32 {
    let a = if target > prev { alpha_rise } else { alpha_decay };
    prev + a * (target - prev)
}

