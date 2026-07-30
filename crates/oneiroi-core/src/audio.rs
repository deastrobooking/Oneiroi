//! Device-neutral audio analysis.
//!
//! Platform callbacks live in `oneiroi-io`. This module only consumes mono
//! sample windows, making spectral behavior deterministic and testable without
//! an audio device.

use std::sync::Arc;

use rustfft::{Fft, FftPlanner, num_complex::Complex};

pub const AUDIO_ANALYSIS_SIZE: usize = 1024;

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
}

pub struct AudioAnalyzer {
    sample_rate: u32,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    smoothed: AudioSnapshot,
    previous_input_rms: f32,
    normalization_gain: f32,
}

impl AudioAnalyzer {
    pub fn new(sample_rate: u32) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(AUDIO_ANALYSIS_SIZE);
        let window = (0..AUDIO_ANALYSIS_SIZE)
            .map(|index| {
                let phase = index as f32 / (AUDIO_ANALYSIS_SIZE - 1) as f32;
                0.5 - 0.5 * (std::f32::consts::TAU * phase).cos()
            })
            .collect();
        Self {
            sample_rate: sample_rate.max(1),
            fft,
            window,
            spectrum: vec![Complex::ZERO; AUDIO_ANALYSIS_SIZE],
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
        let input = AudioSnapshot {
            rms: normalize(rms),
            peak: normalize(peak),
            bass: normalize(band_power[0].sqrt()),
            mid: normalize(band_power[1].sqrt()),
            high: normalize(band_power[2].sqrt()),
            transient: ((normalize(rms) - self.previous_input_rms).max(0.0)
                * settings.transient_sensitivity)
                .clamp(0.0, 1.0),
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
        self.smoothed
    }
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
