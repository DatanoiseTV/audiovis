//! Spectral analysis: a windowed real FFT reduced through a mel filterbank to
//! perceptual band energies, plus loudness and spectral flux for onsets.
//!
//! The structure follows scottlawsonbc/audio-reactive-led-strip: a mel-spaced
//! triangular filterbank (instead of a few linear bins) and downstream rolling
//! auto-gain (done in the analysis thread, see `mod.rs`) so the response
//! self-calibrates to any source. One deliberate change: the low edge sits at
//! 40 Hz, not 200 Hz, so a techno kick drum actually drives the low band.
//!
//! Pure DSP, no I/O - unit-tested against synthetic signals.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// Mel band count. Split into thirds for low/mid/high.
pub const N_MEL: usize = 24;
const MIN_FREQ: f32 = 40.0;
const MAX_FREQ: f32 = 12_000.0;

/// Instantaneous features for one analysis window.
#[derive(Debug, Clone, Copy, Default)]
pub struct Features {
    /// Broadband loudness (RMS of the windowed block).
    pub rms: f32,
    /// Low-band energy (lowest third of the mel bands).
    pub low: f32,
    /// Mid-band energy (middle third).
    pub mid: f32,
    /// High-band energy (upper third).
    pub high: f32,
    /// Spectral flux: summed positive magnitude change since the last block.
    pub flux: f32,
}

fn hz_to_mel(f: f32) -> f32 {
    2595.0 * (1.0 + f / 700.0).log10()
}
fn mel_to_hz(m: f32) -> f32 {
    700.0 * (10f32.powf(m / 2595.0) - 1.0)
}

/// Reusable analyzer for a fixed window size and sample rate.
pub struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    prev_mag: Vec<f32>,
    /// Triangular mel filters; each is a list of (bin, weight).
    melbank: Vec<Vec<(usize, f32)>>,
    size: usize,
}

impl Analyzer {
    pub fn new(size: usize, sample_rate: f32) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(size);
        let input = fft.make_input_vec();
        let spectrum = fft.make_output_vec();
        let nbins = spectrum.len();

        // Hann window tames spectral leakage from the block edges.
        let window = (0..size)
            .map(|i| {
                let x = i as f32 / (size as f32 - 1.0);
                0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
            })
            .collect();

        let melbank = build_melbank(nbins, size, sample_rate);

        Self { prev_mag: vec![0.0; nbins], fft, window, input, spectrum, melbank, size }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Analyze one block of mono samples (exactly the window size).
    pub fn analyze(&mut self, samples: &[f32]) -> Features {
        debug_assert_eq!(samples.len(), self.size);

        let mut sum_sq = 0.0f32;
        for i in 0..self.size {
            let s = samples[i];
            sum_sq += s * s;
            self.input[i] = s * self.window[i];
        }
        let rms = (sum_sq / self.size as f32).sqrt();

        if self.fft.process(&mut self.input, &mut self.spectrum).is_err() {
            return Features { rms, ..Default::default() };
        }

        let norm = 2.0 / self.size as f32;
        let mut flux = 0.0f32;
        let mut mags = vec![0.0f32; self.spectrum.len()];
        for (i, c) in self.spectrum.iter().enumerate() {
            let m = c.norm() * norm;
            flux += (m - self.prev_mag[i]).max(0.0);
            mags[i] = m;
        }
        self.prev_mag = mags.clone();

        // Project magnitudes through the mel filterbank.
        let mut mel = [0.0f32; N_MEL];
        for (j, filt) in self.melbank.iter().enumerate() {
            let mut acc = 0.0f32;
            for &(bin, w) in filt {
                acc += mags[bin] * w;
            }
            mel[j] = acc;
        }

        let third = N_MEL / 3;
        let mean = |s: &[f32]| s.iter().sum::<f32>() / s.len().max(1) as f32;
        let low = mean(&mel[0..third]);
        let mid = mean(&mel[third..2 * third]);
        let high = mean(&mel[2 * third..N_MEL]);

        Features { rms, low, mid, high, flux }
    }
}

/// Build triangular mel filters over the rfft bins, mel-spaced across
/// `[MIN_FREQ, MAX_FREQ]`.
fn build_melbank(nbins: usize, size: usize, sample_rate: f32) -> Vec<Vec<(usize, f32)>> {
    let mel_lo = hz_to_mel(MIN_FREQ);
    let mel_hi = hz_to_mel(MAX_FREQ.min(sample_rate / 2.0));
    // N_MEL+2 edge points -> N_MEL triangles sharing edges.
    let points: Vec<usize> = (0..N_MEL + 2)
        .map(|i| {
            let mel = mel_lo + (mel_hi - mel_lo) * i as f32 / (N_MEL + 1) as f32;
            let hz = mel_to_hz(mel);
            ((hz * size as f32 / sample_rate).round() as usize).min(nbins - 1)
        })
        .collect();

    let mut bank = Vec::with_capacity(N_MEL);
    for j in 0..N_MEL {
        let (lo, ctr, hi) = (points[j], points[j + 1], points[j + 2]);
        let mut filt = Vec::new();
        // Rising edge.
        for b in lo..ctr {
            let w = (b - lo) as f32 / (ctr - lo).max(1) as f32;
            filt.push((b, w));
        }
        // Falling edge.
        for b in ctr..=hi {
            let w = (hi - b) as f32 / (hi - ctr).max(1) as f32;
            filt.push((b, w));
        }
        if filt.is_empty() {
            filt.push((ctr, 1.0));
        }
        bank.push(filt);
    }
    bank
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(freq: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (TAU * freq * i as f32 / sr).sin()).collect()
    }

    #[test]
    fn low_tone_drives_low_band() {
        let sr = 44_100.0;
        let n = 2048;
        let mut a = Analyzer::new(n, sr);
        let f = a.analyze(&sine(60.0, sr, n));
        assert!(f.low > f.mid && f.low > f.high, "low {} mid {} high {}", f.low, f.mid, f.high);
    }

    #[test]
    fn high_tone_drives_high_band() {
        let sr = 44_100.0;
        let n = 2048;
        let mut a = Analyzer::new(n, sr);
        let f = a.analyze(&sine(6000.0, sr, n));
        assert!(f.high > f.low && f.high > f.mid, "low {} mid {} high {}", f.low, f.mid, f.high);
    }

    #[test]
    fn silence_is_quiet() {
        let mut a = Analyzer::new(1024, 44_100.0);
        let f = a.analyze(&vec![0.0; 1024]);
        assert_eq!(f.rms, 0.0);
        assert_eq!(f.low, 0.0);
    }

    #[test]
    fn flux_rises_on_a_new_onset() {
        let sr = 44_100.0;
        let n = 1024;
        let mut a = Analyzer::new(n, sr);
        a.analyze(&vec![0.0; n]);
        let f = a.analyze(&sine(440.0, sr, n));
        assert!(f.flux > 0.0);
    }
}
