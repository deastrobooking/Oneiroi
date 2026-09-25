//! Audio-reactive control mappings: spectrum bands driving control targets.
//!
//! A binding reads one analysis source (a spectrum band, overall level or the
//! transient detector) and turns it into [`ControlUpdate`]s for the same
//! targets MIDI can drive.

use crate::audio::{AudioSnapshot, SPECTRUM_BAND_LABELS, SPECTRUM_BANDS};
use crate::control::{ControlTarget, ControlUpdate};

/// Eight spectrum bands, then level and transient.
pub const AUDIO_MAP_SOURCES: usize = SPECTRUM_BANDS + 2;
pub const AUDIO_MAP_LEVEL: u8 = SPECTRUM_BANDS as u8;
pub const AUDIO_MAP_TRANSIENT: u8 = SPECTRUM_BANDS as u8 + 1;
/// Upper bound on persisted bindings.
pub const MAX_AUDIO_BINDINGS: usize = 64;
/// Gate and trigger re-arm this far below the threshold so a signal hovering
/// on the threshold does not chatter.
const HYSTERESIS: f32 = 0.05;

pub fn audio_map_source_label(source: u8) -> &'static str {
    match source {
        AUDIO_MAP_LEVEL => "Level",
        AUDIO_MAP_TRANSIENT => "Transient",
        band => SPECTRUM_BAND_LABELS
            .get(usize::from(band))
            .copied()
            .unwrap_or("Unknown"),
    }
}

pub fn audio_map_sources(snapshot: &AudioSnapshot) -> [f32; AUDIO_MAP_SOURCES] {
    let mut sources = [0.0; AUDIO_MAP_SOURCES];
    sources[..SPECTRUM_BANDS].copy_from_slice(&snapshot.bands);
    sources[usize::from(AUDIO_MAP_LEVEL)] = snapshot.rms;
    sources[usize::from(AUDIO_MAP_TRANSIENT)] = snapshot.transient;
    sources
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AudioMapMode {
    /// Follows the source across the output range every frame.
    #[default]
    Continuous,
    /// Sends the high output once each time the source crosses the threshold.
    Trigger,
    /// High while the source is above the threshold, low once it falls back.
    Gate,
}

impl AudioMapMode {
    pub const ALL: [Self; 3] = [Self::Continuous, Self::Trigger, Self::Gate];

    /// Launch-style targets act on every value at or above 0.5, so following
    /// a band continuously would re-fire them; they start as triggers.
    pub fn default_for(target: ControlTarget) -> Self {
        match target {
            ControlTarget::TapTempo
            | ControlTarget::DeckRestart(_)
            | ControlTarget::DeckSelect(_)
            | ControlTarget::ClipLaunch { .. }
            | ControlTarget::SceneLaunch(_) => Self::Trigger,
            ControlTarget::MasterBlackout
            | ControlTarget::MasterFreeze
            | ControlTarget::DeckPlay(_)
            | ControlTarget::DeckFreeze(_) => Self::Gate,
            _ => Self::Continuous,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Continuous => "Continuous",
            Self::Trigger => "Trigger",
            Self::Gate => "Gate",
        }
    }
}

/// Output range suited to a target's natural domain.
pub fn default_output_range(target: ControlTarget) -> [f32; 2] {
    match target {
        ControlTarget::DeckSpeed(_) => [0.5, 2.0],
        // LFO rate in hertz.
        ControlTarget::LfoParameter { parameter: 1, .. } => [0.1, 8.0],
        // LFO offset and mod-route amount are bipolar.
        ControlTarget::LfoParameter { parameter: 4, .. }
        | ControlTarget::ModRouteParameter { parameter: 1, .. } => [-1.0, 1.0],
        _ => [0.0, 1.0],
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioBinding {
    pub enabled: bool,
    pub source: u8,
    pub target: ControlTarget,
    /// Source span that maps onto the full output range.
    pub input_range: [f32; 2],
    pub output_range: [f32; 2],
    pub invert: bool,
    pub mode: AudioMapMode,
    pub threshold: f32,
    above: bool,
    last_output: Option<f32>,
}

impl AudioBinding {
    pub fn new(source: u8, target: ControlTarget) -> Self {
        Self {
            enabled: true,
            source: source.min(AUDIO_MAP_TRANSIENT),
            target,
            input_range: [0.0, 1.0],
            output_range: default_output_range(target),
            invert: false,
            mode: AudioMapMode::default_for(target),
            threshold: 0.6,
            above: false,
            last_output: None,
        }
    }

    /// Forget edge and last-sent state, e.g. after the input reconnects.
    pub fn reset(&mut self) {
        self.above = false;
        self.last_output = None;
    }

    /// The source value after range and invert, in `0..=1`.
    pub fn normalized(&self, sources: &[f32; AUDIO_MAP_SOURCES]) -> f32 {
        let raw = sources
            .get(usize::from(self.source))
            .copied()
            .filter(|value| value.is_finite())
            .unwrap_or_default();
        let span = self.input_range[1] - self.input_range[0];
        let normalized = if span.abs() < f32::EPSILON {
            f32::from(raw >= self.input_range[0])
        } else {
            ((raw - self.input_range[0]) / span).clamp(0.0, 1.0)
        };
        if self.invert {
            1.0 - normalized
        } else {
            normalized
        }
    }

    /// The value to send this frame, if the target should change.
    pub fn apply(&mut self, sources: &[f32; AUDIO_MAP_SOURCES]) -> Option<f32> {
        if !self.enabled {
            return None;
        }
        let normalized = self.normalized(sources);
        let output = match self.mode {
            AudioMapMode::Continuous => self.map_output(normalized),
            AudioMapMode::Trigger | AudioMapMode::Gate => {
                let was_above = self.above;
                if normalized >= self.threshold {
                    self.above = true;
                } else if normalized < self.threshold - HYSTERESIS {
                    self.above = false;
                }
                match (self.mode, was_above, self.above) {
                    (AudioMapMode::Trigger, false, true) => {
                        // Every onset fires, even when the last one sent the
                        // same value.
                        self.last_output = None;
                        self.map_output(1.0)
                    }
                    (AudioMapMode::Trigger, ..) => return None,
                    (_, _, above) => self.map_output(f32::from(above)),
                }
            }
        };
        if self
            .last_output
            .is_some_and(|last| (last - output).abs() < 1.0e-4)
        {
            return None;
        }
        self.last_output = Some(output);
        Some(output)
    }

    fn map_output(&self, normalized: f32) -> f32 {
        self.output_range[0] + normalized * (self.output_range[1] - self.output_range[0])
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioMapper {
    pub bindings: Vec<AudioBinding>,
}

impl AudioMapper {
    /// Updates for every binding whose output changed, paired with its mode
    /// so the caller can journal discrete events but not continuous streams.
    pub fn update(&mut self, snapshot: &AudioSnapshot) -> Vec<(ControlUpdate, AudioMapMode)> {
        let sources = audio_map_sources(snapshot);
        self.bindings
            .iter_mut()
            .filter_map(|binding| {
                let value = binding.apply(&sources)?;
                Some((
                    ControlUpdate {
                        target: binding.target,
                        value,
                    },
                    binding.mode,
                ))
            })
            .collect()
    }

    pub fn reset(&mut self) {
        for binding in &mut self.bindings {
            binding.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band(index: usize, value: f32) -> [f32; AUDIO_MAP_SOURCES] {
        let mut sources = [0.0; AUDIO_MAP_SOURCES];
        sources[index] = value;
        sources
    }

    #[test]
    fn continuous_maps_ranges_and_skips_unchanged_values() {
        let mut binding = AudioBinding::new(1, ControlTarget::Crossfader);
        binding.input_range = [0.2, 0.6];
        binding.output_range = [0.25, 0.75];
        assert_eq!(binding.apply(&band(1, 0.4)), Some(0.5));
        assert_eq!(binding.apply(&band(1, 0.4)), None);
        assert_eq!(binding.apply(&band(1, 1.0)), Some(0.75));
        binding.invert = true;
        assert_eq!(binding.apply(&band(1, 1.0)), Some(0.25));
        binding.enabled = false;
        assert_eq!(binding.apply(&band(1, 0.0)), None);
    }

    #[test]
    fn trigger_fires_once_per_onset_with_hysteresis() {
        let mut binding = AudioBinding::new(0, ControlTarget::SceneLaunch(0));
        binding.mode = AudioMapMode::Trigger;
        binding.threshold = 0.5;
        assert_eq!(binding.apply(&band(0, 0.2)), None);
        assert_eq!(binding.apply(&band(0, 0.7)), Some(1.0));
        assert_eq!(binding.apply(&band(0, 0.9)), None);
        // Dipping inside the hysteresis band does not re-arm.
        assert_eq!(binding.apply(&band(0, 0.48)), None);
        assert_eq!(binding.apply(&band(0, 0.6)), None);
        assert_eq!(binding.apply(&band(0, 0.1)), None);
        assert_eq!(binding.apply(&band(0, 0.6)), Some(1.0));
    }

    #[test]
    fn launch_targets_default_to_discrete_modes() {
        assert_eq!(
            AudioBinding::new(0, ControlTarget::SceneLaunch(2)).mode,
            AudioMapMode::Trigger
        );
        assert_eq!(
            AudioBinding::new(0, ControlTarget::TapTempo).mode,
            AudioMapMode::Trigger
        );
        assert_eq!(
            AudioBinding::new(0, ControlTarget::MasterBlackout).mode,
            AudioMapMode::Gate
        );
        assert_eq!(
            AudioBinding::new(0, ControlTarget::DeckLevel(1)).mode,
            AudioMapMode::Continuous
        );
    }

    #[test]
    fn gate_follows_threshold_edges() {
        let mut binding = AudioBinding::new(AUDIO_MAP_TRANSIENT, ControlTarget::MasterBlackout);
        binding.mode = AudioMapMode::Gate;
        binding.threshold = 0.5;
        let transient = usize::from(AUDIO_MAP_TRANSIENT);
        assert_eq!(binding.apply(&band(transient, 0.1)), Some(0.0));
        assert_eq!(binding.apply(&band(transient, 0.8)), Some(1.0));
        assert_eq!(binding.apply(&band(transient, 0.9)), None);
        assert_eq!(binding.apply(&band(transient, 0.2)), Some(0.0));
    }

    #[test]
    fn mapper_reads_bands_level_and_transient_from_a_snapshot() {
        let mut mapper = AudioMapper {
            bindings: vec![
                AudioBinding::new(7, ControlTarget::MasterOpacity),
                AudioBinding::new(AUDIO_MAP_LEVEL, ControlTarget::Crossfader),
            ],
        };
        let mut snapshot = AudioSnapshot {
            rms: 0.3,
            ..AudioSnapshot::default()
        };
        snapshot.bands[7] = 0.9;
        let updates = mapper.update(&snapshot);
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].0.value, 0.9);
        assert_eq!(updates[1].0.target, ControlTarget::Crossfader);
        assert!((updates[1].0.value - 0.3).abs() < 1.0e-6);
        assert!(mapper.update(&snapshot).is_empty());
    }
}
