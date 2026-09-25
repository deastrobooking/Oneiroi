//! Device-neutral audio analysis.
//!
//! Platform callbacks live in `virtual-io`. This module only consumes mono
//! sample windows, making spectral behavior deterministic and testable without
//! an audio device.

use std::sync::Arc;

use rustfft::{Fft, FftPlanner, num_complex::Complex};

pub const AUDIO_ANALYSIS_SIZE: usize = 1024;
/// Window for the 8-band spectrum. Longer than the 1024-sample hop so the
/// sub and bass bands span several FFT bins (about 11.7 Hz each at 48 kHz).
pub const SPECTRUM_ANALYSIS_SIZE: usize = 4096;
pub const SPECTRUM_BANDS: usize = 8;
pub const SPECTRUM_BAND_EDGES_HZ: [f32; SPECTRUM_BANDS + 1] = [
    20.0, 60.0, 150.0, 400.0, 1_000.0, 2_500.0, 5_000.0, 10_000.0, 20_000.0,
];
pub const SPECTRUM_BAND_LABELS: [&str; SPECTRUM_BANDS] = [
    "Sub",
    "Bass",
    "Low mid",
    "Mid",
    "Upper mid",
    "Presence",
    "Brilliance",
    "Air",
];
/// RMS, bass, mid, high, transient, then the eight spectrum bands.
pub const AUDIO_MOD_SOURCES: usize = 5 + SPECTRUM_BANDS;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioAnalysisSettings {
    pub gain: f32,
    pub noise_floor: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub transient_sensitivity: f32,
    pub normalization: bool,
    pub normalization_target: f32,
    pub normalization_speed_ms: f32,
    /// Per-band boost or cut applied before the band is published.
    pub band_gains_db: [f32; SPECTRUM_BANDS],
    /// Publish bands on a decibel scale, which keeps quiet high bands visible.
    pub spectrum_decibels: bool,
    /// Decibels below full scale that map to zero in decibel mode.
    pub spectrum_range_db: f32,
}

impl Default for AudioAnalysisSettings {
    fn default() -> Self {
        Self {
            gain: 1.0,
            noise_floor: 0.01,
            attack_ms: 20.0,
            release_ms: 180.0,
            transient_sensitivity: 2.0,
            normalization: false,
            normalization_target: 0.5,
            normalization_speed_ms: 1_000.0,
            band_gains_db: [0.0; SPECTRUM_BANDS],
            spectrum_decibels: true,
            spectrum_range_db: 60.0,
        }
    }
}

impl AudioAnalysisSettings {
    pub fn sanitized(mut self) -> Self {
        self.gain = finite_or(self.gain, 1.0).clamp(0.0, 16.0);
        self.noise_floor = finite_or(self.noise_floor, 0.01).clamp(0.0, 0.5);
        self.attack_ms = finite_or(self.attack_ms, 20.0).clamp(1.0, 2_000.0);
        self.release_ms = finite_or(self.release_ms, 180.0).clamp(1.0, 5_000.0);
        self.transient_sensitivity = finite_or(self.transient_sensitivity, 2.0).clamp(0.0, 16.0);
        self.normalization_target = finite_or(self.normalization_target, 0.5).clamp(0.05, 1.0);
        self.normalization_speed_ms =
            finite_or(self.normalization_speed_ms, 1_000.0).clamp(10.0, 10_000.0);
        self.band_gains_db = self
            .band_gains_db
            .map(|gain| finite_or(gain, 0.0).clamp(-24.0, 24.0));
        self.spectrum_range_db = finite_or(self.spectrum_range_db, 60.0).clamp(12.0, 96.0);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioSnapshot {
    pub rms: f32,
    pub peak: f32,
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
    pub transient: f32,
    /// Eight log-spaced bands, see [`SPECTRUM_BAND_EDGES_HZ`].
    pub bands: [f32; SPECTRUM_BANDS],
}

impl AudioSnapshot {
    /// Values in [`AUDIO_MOD_SOURCES`] order.
    pub fn modulation_sources(&self) -> [f32; AUDIO_MOD_SOURCES] {
        let mut sources = [0.0; AUDIO_MOD_SOURCES];
        sources[..5].copy_from_slice(&[self.rms, self.bass, self.mid, self.high, self.transient]);
        sources[5..].copy_from_slice(&self.bands);
        sources
    }
}

pub struct AudioAnalyzer {
    sample_rate: u32,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    band_fft: Arc<dyn Fft<f32>>,
    band_window: Vec<f32>,
    band_spectrum: Vec<Complex<f32>>,
    history: Vec<f32>,
    smoothed: AudioSnapshot,
    previous_input_rms: f32,
    normalization_gain: f32,
}

impl AudioAnalyzer {
    pub fn new(sample_rate: u32) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(AUDIO_ANALYSIS_SIZE);
        Self {
            sample_rate: sample_rate.max(1),
            fft,
            window: hann(AUDIO_ANALYSIS_SIZE),
            spectrum: vec![Complex::ZERO; AUDIO_ANALYSIS_SIZE],
            band_fft: planner.plan_fft_forward(SPECTRUM_ANALYSIS_SIZE),
            band_window: hann(SPECTRUM_ANALYSIS_SIZE),
            band_spectrum: vec![Complex::ZERO; SPECTRUM_ANALYSIS_SIZE],
            history: vec![0.0; SPECTRUM_ANALYSIS_SIZE],
            smoothed: AudioSnapshot::default(),
            previous_input_rms: 0.0,
            normalization_gain: 1.0,
        }
    }

    pub fn analyze(&mut self, samples: &[f32], settings: AudioAnalysisSettings) -> AudioSnapshot {
        let settings = settings.sanitized();
        let start = samples.len().saturating_sub(AUDIO_ANALYSIS_SIZE);
        let samples = &samples[start..];
        let padding = AUDIO_ANALYSIS_SIZE - samples.len();
        let mut sum_squares = 0.0;
        let mut peak = 0.0_f32;
        for index in 0..AUDIO_ANALYSIS_SIZE {
            let sample = if index < padding {
                0.0
            } else {
                finite_or(samples[index - padding], 0.0).clamp(-1.0, 1.0)
            };
            sum_squares += sample * sample;
            peak = peak.max(sample.abs());
            self.spectrum[index] = Complex::new(sample * self.window[index], 0.0);
        }
        self.fft.process(&mut self.spectrum);

        let rms = (sum_squares / AUDIO_ANALYSIS_SIZE as f32).sqrt();
        let window_sum: f32 = self.window.iter().sum();
        let bin_hz = self.sample_rate as f32 / AUDIO_ANALYSIS_SIZE as f32;
        let mut band_power = [0.0_f32; 3];
        for bin in 1..=AUDIO_ANALYSIS_SIZE / 2 {
            let frequency = bin as f32 * bin_hz;
            let band = if (20.0..250.0).contains(&frequency) {
                Some(0)
            } else if (250.0..2_000.0).contains(&frequency) {
                Some(1)
            } else if (2_000.0..=16_000.0).contains(&frequency) {
                Some(2)
            } else {
                None
            };
            if let Some(band) = band {
                let amplitude = self.spectrum[bin].norm() * 2.0 / window_sum.max(1.0);
                band_power[band] += amplitude * amplitude * 0.5;
            }
        }

        self.push_history(samples);
        let band_amplitudes = self.band_amplitudes();

        let frame_seconds = AUDIO_ANALYSIS_SIZE as f32 / self.sample_rate as f32;
        let denoised_rms = (rms - settings.noise_floor).max(0.0);
        if settings.normalization && denoised_rms > 0.001 {
            let desired_gain = (settings.normalization_target / denoised_rms).clamp(0.1, 16.0);
            let coefficient = (-frame_seconds / (settings.normalization_speed_ms * 0.001)).exp();
            self.normalization_gain =
                desired_gain + (self.normalization_gain - desired_gain) * coefficient;
        } else if !settings.normalization {
            self.normalization_gain = 1.0;
        }
        let effective_gain = settings.gain * self.normalization_gain;
        let normalize =
            |value: f32| ((value - settings.noise_floor).max(0.0) * effective_gain).clamp(0.0, 1.0);
        let bands = std::array::from_fn(|band| {
            let gain = 10.0_f32.powf(settings.band_gains_db[band] / 20.0);
            let amplitude = band_amplitudes[band] * gain;
            if settings.spectrum_decibels {
                let level_db = 20.0 * (amplitude * effective_gain).max(1.0e-9).log10();
                ((level_db + settings.spectrum_range_db) / settings.spectrum_range_db)
                    .clamp(0.0, 1.0)
            } else {
                normalize(amplitude)
            }
        });
        let input = AudioSnapshot {
            rms: normalize(rms),
            peak: normalize(peak),
            bass: normalize(band_power[0].sqrt()),
            mid: normalize(band_power[1].sqrt()),
            high: normalize(band_power[2].sqrt()),
            transient: ((normalize(rms) - self.previous_input_rms).max(0.0)
                * settings.transient_sensitivity)
                .clamp(0.0, 1.0),
            bands,
        };
        self.previous_input_rms = input.rms;

        self.smoothed.rms = smooth(
            self.smoothed.rms,
            input.rms,
            frame_seconds,
            settings.attack_ms,
            settings.release_ms,
        );
        self.smoothed.peak = smooth(
            self.smoothed.peak,
            input.peak,
            frame_seconds,
            settings.attack_ms,
            settings.release_ms,
        );
        self.smoothed.bass = smooth(
            self.smoothed.bass,
            input.bass,
            frame_seconds,
            settings.attack_ms,
            settings.release_ms,
        );
        self.smoothed.mid = smooth(
            self.smoothed.mid,
            input.mid,
            frame_seconds,
            settings.attack_ms,
            settings.release_ms,
        );
        self.smoothed.high = smooth(
            self.smoothed.high,
            input.high,
            frame_seconds,
            settings.attack_ms,
            settings.release_ms,
        );
        self.smoothed.transient = input.transient;
        for band in 0..SPECTRUM_BANDS {
            self.smoothed.bands[band] = smooth(
                self.smoothed.bands[band],
                input.bands[band],
                frame_seconds,
                settings.attack_ms,
                settings.release_ms,
            );
        }
        self.smoothed
    }

    fn push_history(&mut self, samples: &[f32]) {
        let samples = &samples[samples.len().saturating_sub(SPECTRUM_ANALYSIS_SIZE)..];
        self.history.copy_within(samples.len().., 0);
        let start = SPECTRUM_ANALYSIS_SIZE - samples.len();
        for (slot, sample) in self.history[start..].iter_mut().zip(samples) {
            *slot = finite_or(*sample, 0.0).clamp(-1.0, 1.0);
        }
    }

    /// RMS amplitude of each spectrum band over the rolling history window.
    fn band_amplitudes(&mut self) -> [f32; SPECTRUM_BANDS] {
        for (index, bin) in self.band_spectrum.iter_mut().enumerate() {
            *bin = Complex::new(self.history[index] * self.band_window[index], 0.0);
        }
        self.band_fft.process(&mut self.band_spectrum);
        let window_sum: f32 = self.band_window.iter().sum();
        let bin_hz = self.sample_rate as f32 / SPECTRUM_ANALYSIS_SIZE as f32;
        let mut power = [0.0_f32; SPECTRUM_BANDS];
        for bin in 1..=SPECTRUM_ANALYSIS_SIZE / 2 {
            let frequency = bin as f32 * bin_hz;
            let Some(band) = SPECTRUM_BAND_EDGES_HZ
                .windows(2)
                .position(|edges| (edges[0]..edges[1]).contains(&frequency))
            else {
                continue;
            };
            let amplitude = self.band_spectrum[bin].norm() * 2.0 / window_sum.max(1.0);
            power[band] += amplitude * amplitude * 0.5;
        }
        power.map(f32::sqrt)
    }
}

fn hann(size: usize) -> Vec<f32> {
    (0..size)
        .map(|index| {
            let phase = index as f32 / (size - 1) as f32;
            0.5 - 0.5 * (std::f32::consts::TAU * phase).cos()
        })
        .collect()
}

fn smooth(current: f32, target: f32, frame_seconds: f32, attack_ms: f32, release_ms: f32) -> f32 {
    let milliseconds = if target > current {
        attack_ms
    } else {
        release_ms
    };
    let coefficient = (-frame_seconds / (milliseconds * 0.001)).exp();
    target + (current - target) * coefficient
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frequency: f32, sample_rate: u32, amplitude: f32) -> Vec<f32> {
        (0..AUDIO_ANALYSIS_SIZE)
            .map(|index| {
                (std::f32::consts::TAU * frequency * index as f32 / sample_rate as f32).sin()
                    * amplitude
            })
            .collect()
    }

    fn immediate() -> AudioAnalysisSettings {
        AudioAnalysisSettings {
            noise_floor: 0.0,
            attack_ms: 1.0,
            release_ms: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn silence_produces_zero_analysis() {
        let mut analyzer = AudioAnalyzer::new(48_000);
        assert_eq!(
            analyzer.analyze(&[0.0; AUDIO_ANALYSIS_SIZE], immediate()),
            AudioSnapshot::default()
        );
    }

    #[test]
    fn sine_fixtures_land_in_the_expected_bands() {
        for (frequency, expected) in [(100.0, 0), (1_000.0, 1), (8_000.0, 2)] {
            let mut analyzer = AudioAnalyzer::new(48_000);
            let snapshot = analyzer.analyze(&sine(frequency, 48_000, 0.8), immediate());
            let bands = [snapshot.bass, snapshot.mid, snapshot.high];
            assert!(
                bands[expected] > 0.4,
                "{frequency} Hz expected band was {bands:?}"
            );
            assert!(
                bands[expected] > bands[(expected + 1) % 3] * 3.0,
                "{frequency} Hz leaked across bands: {bands:?}"
            );
        }
    }

    fn continuous_sine(frequency: f32, sample_rate: u32, amplitude: f32) -> Vec<f32> {
        (0..SPECTRUM_ANALYSIS_SIZE)
            .map(|index| {
                (std::f32::consts::TAU * frequency * index as f32 / sample_rate as f32).sin()
                    * amplitude
            })
            .collect()
    }

    #[test]
    fn each_band_centre_lands_in_its_own_spectrum_band() {
        let settings = AudioAnalysisSettings {
            spectrum_decibels: false,
            ..immediate()
        };
        for band in 0..SPECTRUM_BANDS {
            let centre = (SPECTRUM_BAND_EDGES_HZ[band] * SPECTRUM_BAND_EDGES_HZ[band + 1]).sqrt();
            let mut analyzer = AudioAnalyzer::new(48_000);
            let signal = continuous_sine(centre, 48_000, 0.8);
            let mut snapshot = AudioSnapshot::default();
            for chunk in signal.chunks(AUDIO_ANALYSIS_SIZE) {
                snapshot = analyzer.analyze(chunk, settings);
            }
            let loudest = (0..SPECTRUM_BANDS)
                .max_by(|a, b| snapshot.bands[*a].total_cmp(&snapshot.bands[*b]))
                .unwrap();
            assert_eq!(loudest, band, "{centre:.0} Hz gave {:?}", snapshot.bands);
            assert!(
                snapshot.bands[band] > 0.4,
                "{centre:.0} Hz: {:?}",
                snapshot.bands
            );
        }
    }

    #[test]
    fn decibel_bands_and_band_gain_scale_quiet_signals() {
        let signal = continuous_sine(3_500.0, 48_000, 0.01);
        let run = |settings: AudioAnalysisSettings| {
            let mut analyzer = AudioAnalyzer::new(48_000);
            let mut snapshot = AudioSnapshot::default();
            for chunk in signal.chunks(AUDIO_ANALYSIS_SIZE) {
                snapshot = analyzer.analyze(chunk, settings);
            }
            snapshot.bands[5]
        };
        let decibels = run(immediate());
        // 0.01 peak is about -43 dBFS RMS, so roughly 0.28 of a 60 dB range.
        assert!((0.2..0.4).contains(&decibels), "decibel band {decibels}");
        let mut boosted = immediate();
        boosted.band_gains_db[5] = 12.0;
        assert!((run(boosted) - decibels - 0.2).abs() < 0.02);
        let mut cut = immediate();
        cut.band_gains_db[4] = -24.0;
        assert!(
            (run(cut) - decibels).abs() < 1.0e-6,
            "gain only touches its band"
        );
    }

    #[test]
    fn a_rising_signal_publishes_a_transient_once() {
        let mut analyzer = AudioAnalyzer::new(48_000);
        analyzer.analyze(&[0.0; AUDIO_ANALYSIS_SIZE], immediate());
        let first = analyzer.analyze(&sine(1_000.0, 48_000, 0.8), immediate());
        let second = analyzer.analyze(&sine(1_000.0, 48_000, 0.8), immediate());
        assert!(first.transient > 0.5);
        assert_eq!(second.transient, 0.0);
    }

    #[test]
    fn adaptive_normalization_converges_different_levels_toward_the_target() {
        let settings = AudioAnalysisSettings {
            noise_floor: 0.0,
            attack_ms: 1.0,
            release_ms: 1.0,
            normalization: true,
            normalization_target: 0.5,
            normalization_speed_ms: 10.0,
            ..Default::default()
        };
        let converged = |amplitude| {
            let mut analyzer = AudioAnalyzer::new(48_000);
            let signal = sine(1_000.0, 48_000, amplitude);
            let mut snapshot = AudioSnapshot::default();
            for _ in 0..8 {
                snapshot = analyzer.analyze(&signal, settings);
            }
            snapshot.rms
        };
        let quiet = converged(0.2);
        let loud = converged(0.8);
        assert!((quiet - 0.5).abs() < 0.03, "quiet normalized to {quiet}");
        assert!((loud - 0.5).abs() < 0.03, "loud normalized to {loud}");
    }
}
