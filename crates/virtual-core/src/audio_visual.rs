//! Waveform history and oscilloscope data for the operator's audio display.

use std::collections::VecDeque;

use crate::audio::{AudioAnalyzer, SPECTRUM_CURVE_POINTS};

/// Samples in the oscilloscope view (about 21 ms at 48 kHz).
pub const SCOPE_SAMPLES: usize = 1024;
/// Samples folded into one min/max column of the waveform history.
pub const WAVEFORM_BLOCK: usize = 256;
/// Columns of waveform history (about 4 s at 48 kHz).
pub const WAVEFORM_COLUMNS: usize = 768;

/// Everything the audio display draws, published by the analysis worker.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioVisual {
    pub sample_rate: u32,
    /// Latest samples, aligned to a rising zero crossing when there is one
    /// so a steady tone holds still.
    pub scope: Vec<f32>,
    /// Oldest to newest `[min, max]` per [`WAVEFORM_BLOCK`] samples.
    pub waveform: Vec<[f32; 2]>,
    /// dBFS at `spectrum_curve_frequency` points.
    pub spectrum_db: Vec<f32>,
}

impl Default for AudioVisual {
    fn default() -> Self {
        Self {
            sample_rate: 0,
            scope: vec![0.0; SCOPE_SAMPLES],
            waveform: Vec::new(),
            spectrum_db: vec![-120.0; SPECTRUM_CURVE_POINTS],
        }
    }
}

/// Accumulates incoming samples into the display buffers.
pub struct AudioScope {
    sample_rate: u32,
    recent: VecDeque<f32>,
    columns: VecDeque<[f32; 2]>,
    block: [f32; 2],
    block_len: usize,
}

impl AudioScope {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            recent: VecDeque::from(vec![0.0; SCOPE_SAMPLES * 2]),
            columns: VecDeque::with_capacity(WAVEFORM_COLUMNS),
            block: [0.0; 2],
            block_len: 0,
        }
    }

    pub fn push(&mut self, samples: &[f32]) {
        for &sample in samples {
            let sample = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            self.recent.pop_front();
            self.recent.push_back(sample);
            if self.block_len == 0 {
                self.block = [sample, sample];
            } else {
                self.block = [self.block[0].min(sample), self.block[1].max(sample)];
            }
            self.block_len += 1;
            if self.block_len == WAVEFORM_BLOCK {
                if self.columns.len() == WAVEFORM_COLUMNS {
                    self.columns.pop_front();
                }
                self.columns.push_back(self.block);
                self.block_len = 0;
            }
        }
    }

    pub fn visual(&self, analyzer: &AudioAnalyzer) -> AudioVisual {
        // Search the older half for a rising zero crossing, so the window
        // that follows it is always complete.
        let start = (1..SCOPE_SAMPLES)
            .find(|&index| self.recent[index - 1] < 0.0 && self.recent[index] >= 0.0)
            .unwrap_or(SCOPE_SAMPLES);
        AudioVisual {
            sample_rate: self.sample_rate,
            scope: self
                .recent
                .range(start..start + SCOPE_SAMPLES)
                .copied()
                .collect(),
            waveform: self.columns.iter().copied().collect(),
            spectrum_db: analyzer.spectrum_curve().to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_history_is_bounded_and_tracks_block_extremes() {
        let analyzer = AudioAnalyzer::new(48_000);
        let mut scope = AudioScope::new(48_000);
        let mut samples = vec![0.1; WAVEFORM_BLOCK];
        samples[10] = -0.4;
        samples[20] = 0.9;
        scope.push(&samples);
        assert_eq!(scope.visual(&analyzer).waveform, vec![[-0.4, 0.9]]);
        scope.push(&vec![0.0; WAVEFORM_BLOCK * (WAVEFORM_COLUMNS + 5)]);
        let visual = scope.visual(&analyzer);
        assert_eq!(visual.waveform.len(), WAVEFORM_COLUMNS);
        assert_eq!(visual.scope.len(), SCOPE_SAMPLES);
    }

    #[test]
    fn scope_starts_on_a_rising_zero_crossing() {
        let analyzer = AudioAnalyzer::new(48_000);
        let mut scope = AudioScope::new(48_000);
        let sine: Vec<f32> = (0..SCOPE_SAMPLES * 3)
            .map(|index| (std::f32::consts::TAU * 440.0 * (index as f32 + 17.0) / 48_000.0).sin())
            .collect();
        scope.push(&sine);
        let visual = scope.visual(&analyzer);
        assert!(
            visual.scope[0] >= 0.0 && visual.scope[0] < 0.1,
            "{}",
            visual.scope[0]
        );
        assert!(visual.scope[1] > visual.scope[0]);
    }
}
