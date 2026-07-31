//! Device-neutral MIDI learn and parameter mapping.
//!
//! I/O adapters translate platform MIDI packets into [`MidiMessage`]. The
//! render thread only consumes normalized [`ControlUpdate`] snapshots.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum MidiMessageKind {
    Note,
    ControlChange,
    PitchBend,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MidiMessage {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    PitchBend {
        channel: u8,
        value: u16,
    },
}

impl MidiMessage {
    fn identity(self) -> (u8, MidiMessageKind, u8) {
        match self {
            Self::NoteOn { channel, note, .. } | Self::NoteOff { channel, note } => {
                (channel, MidiMessageKind::Note, note)
            }
            Self::ControlChange {
                channel,
                controller,
                ..
            } => (channel, MidiMessageKind::ControlChange, controller),
            Self::PitchBend { channel, .. } => (channel, MidiMessageKind::PitchBend, 0),
        }
    }

    fn normalized(self) -> f32 {
        match self {
            Self::NoteOn { velocity, .. } => f32::from(velocity) / 127.0,
            Self::NoteOff { .. } => 0.0,
            Self::ControlChange { value, .. } => f32::from(value) / 127.0,
            Self::PitchBend { value, .. } => f32::from(value.min(16_383)) / 16_383.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "target", content = "address")]
pub enum ControlTarget {
    Crossfader,
    MasterOpacity,
    MasterBlackout,
    MasterFreeze,
    TapTempo,
    DeckLevel(u8),
    DeckPlay(u8),
    DeckFreeze(u8),
    DeckSpeed(u8),
    DeckSelect(u8),
    DeckRestart(u8),
    ClipLaunch { deck: u8, slot: u8 },
    SceneLaunch(u8),
    EffectParameter { deck: u8, effect: u8, parameter: u8 },
    LfoParameter { deck: u8, lfo: u8, parameter: u8 },
    ModRouteParameter { deck: u8, route: u8, parameter: u8 },
    MasterEffectParameter { slot: u8, parameter_key: u64 },
}

/// Stable identity for a package parameter across manifest reordering.
pub fn effect_parameter_key(package_id: &str, parameter_id: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in package_id
        .bytes()
        .chain(std::iter::once(0xff))
        .chain(parameter_id.bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappingMode {
    Continuous,
    Momentary,
    Toggle,
    RelativeBinaryOffset,
    RelativeTwosComplement,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MidiBinding {
    pub device: String,
    pub channel: u8,
    pub kind: MidiMessageKind,
    pub number: u8,
    pub target: ControlTarget,
    pub input_range: [f32; 2],
    pub output_range: [f32; 2],
    pub invert: bool,
    pub mode: MappingMode,
    pub soft_takeover: bool,
    latched: bool,
    picked_up: bool,
}

impl MidiBinding {
    pub fn learned(device: impl Into<String>, message: MidiMessage, target: ControlTarget) -> Self {
        let (channel, kind, number) = message.identity();
        Self {
            device: device.into(),
            channel,
            kind,
            number,
            target,
            input_range: [0.0, 1.0],
            output_range: [0.0, 1.0],
            invert: false,
            mode: MappingMode::Continuous,
            soft_takeover: false,
            latched: false,
            picked_up: false,
        }
    }

    pub fn apply(&mut self, message: MidiMessage, current: f32) -> Option<f32> {
        if message.identity() != (self.channel, self.kind, self.number) {
            return None;
        }
        let raw = message.normalized();
        if self.mode == MappingMode::Toggle {
            if raw <= 0.0 {
                return None;
            }
            self.latched = !self.latched;
            let normalized = if self.invert {
                f32::from(!self.latched)
            } else {
                f32::from(self.latched)
            };
            return Some(self.map_output(normalized));
        }
        if self.mode == MappingMode::Momentary {
            let active = raw > 0.0;
            let normalized = if self.invert {
                f32::from(!active)
            } else {
                f32::from(active)
            };
            return Some(self.map_output(normalized));
        }
        if matches!(
            self.mode,
            MappingMode::RelativeBinaryOffset | MappingMode::RelativeTwosComplement
        ) {
            let MidiMessage::ControlChange { value, .. } = message else {
                return None;
            };
            let mut delta = match self.mode {
                MappingMode::RelativeBinaryOffset => i16::from(value) - 64,
                MappingMode::RelativeTwosComplement => {
                    if value == 0 || value == 64 {
                        0
                    } else if value < 64 {
                        i16::from(value)
                    } else {
                        i16::from(value) - 128
                    }
                }
                _ => unreachable!(),
            };
            if self.invert {
                delta = -delta;
            }
            if delta == 0 {
                return None;
            }
            let span = self.output_range[1] - self.output_range[0];
            return Some((current + f32::from(delta) * span / 63.0).clamp(
                self.output_range[0].min(self.output_range[1]),
                self.output_range[0].max(self.output_range[1]),
            ));
        }
        let input_span = (self.input_range[1] - self.input_range[0]).max(f32::EPSILON);
        let mut normalized = ((raw - self.input_range[0]) / input_span).clamp(0.0, 1.0);
        if self.invert {
            normalized = 1.0 - normalized;
        }
        let mapped = self.map_output(normalized);
        if self.soft_takeover && !self.picked_up {
            let tolerance = (self.output_range[1] - self.output_range[0]).abs() / 127.0 * 2.0;
            if (mapped - current).abs() > tolerance.max(0.01) {
                return None;
            }
            self.picked_up = true;
        }
        Some(mapped)
    }

    fn map_output(&self, normalized: f32) -> f32 {
        self.output_range[0] + normalized * (self.output_range[1] - self.output_range[0])
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlUpdate {
    pub target: ControlTarget,
    pub value: f32,
}

#[derive(Default)]
pub struct MidiMapper {
    pub bindings: Vec<MidiBinding>,
    learning: Option<ControlTarget>,
}

impl MidiMapper {
    pub fn learn(&mut self, target: ControlTarget) {
        self.learning = Some(target);
    }

    pub fn cancel_learn(&mut self) {
        self.learning = None;
    }

    pub fn learning(&self) -> Option<ControlTarget> {
        self.learning
    }

    pub fn clear_target(&mut self, target: ControlTarget) {
        self.bindings.retain(|binding| binding.target != target);
        if self.learning == Some(target) {
            self.learning = None;
        }
    }

    pub fn ingest(
        &mut self,
        device: &str,
        message: MidiMessage,
        current_value: impl Fn(ControlTarget) -> f32,
    ) -> Vec<ControlUpdate> {
        if let Some(target) = self.learning.take() {
            self.bindings
                .retain(|binding| binding.target != target || binding.device != device);
            self.bindings
                .push(MidiBinding::learned(device, message, target));
        }
        self.bindings
            .iter_mut()
            .filter(|binding| binding.device == device)
            .filter_map(|binding| {
                let value = binding.apply(message, current_value(binding.target))?;
                Some(ControlUpdate {
                    target: binding.target,
                    value,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_and_maps_a_control_change() {
        let message = MidiMessage::ControlChange {
            channel: 2,
            controller: 7,
            value: 64,
        };
        let mut mapper = MidiMapper::default();
        mapper.learn(ControlTarget::Crossfader);
        let updates = mapper.ingest("controller", message, |_| 0.0);
        assert_eq!(updates.len(), 1);
        assert!((updates[0].value - 64.0 / 127.0).abs() < 1e-6);
    }

    #[test]
    fn toggle_responds_only_to_press_edges() {
        let mut binding = MidiBinding::learned(
            "controller",
            MidiMessage::NoteOn {
                channel: 0,
                note: 1,
                velocity: 127,
            },
            ControlTarget::MasterBlackout,
        );
        binding.mode = MappingMode::Toggle;
        assert_eq!(
            binding.apply(
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 1,
                    velocity: 127,
                },
                0.0
            ),
            Some(1.0)
        );
        assert_eq!(
            binding.apply(
                MidiMessage::NoteOff {
                    channel: 0,
                    note: 1,
                },
                1.0
            ),
            None
        );
    }

    #[test]
    fn soft_takeover_waits_until_hardware_reaches_parameter() {
        let mut binding = MidiBinding::learned(
            "controller",
            MidiMessage::ControlChange {
                channel: 0,
                controller: 1,
                value: 0,
            },
            ControlTarget::MasterOpacity,
        );
        binding.soft_takeover = true;
        let low = MidiMessage::ControlChange {
            channel: 0,
            controller: 1,
            value: 10,
        };
        let near = MidiMessage::ControlChange {
            channel: 0,
            controller: 1,
            value: 101,
        };
        assert_eq!(binding.apply(low, 0.8), None);
        assert!(binding.apply(near, 0.8).is_some());
    }

    #[test]
    fn relative_binary_offset_moves_around_64() {
        let mut binding = MidiBinding::learned(
            "controller",
            MidiMessage::ControlChange {
                channel: 0,
                controller: 1,
                value: 64,
            },
            ControlTarget::Crossfader,
        );
        binding.mode = MappingMode::RelativeBinaryOffset;
        let up = binding.apply(
            MidiMessage::ControlChange {
                channel: 0,
                controller: 1,
                value: 65,
            },
            0.5,
        );
        let down = binding.apply(
            MidiMessage::ControlChange {
                channel: 0,
                controller: 1,
                value: 63,
            },
            0.5,
        );
        assert!(up.is_some_and(|value| value > 0.5));
        assert!(down.is_some_and(|value| value < 0.5));
    }

    #[test]
    fn relative_twos_complement_handles_increment_and_decrement() {
        let mut binding = MidiBinding::learned(
            "controller",
            MidiMessage::ControlChange {
                channel: 0,
                controller: 1,
                value: 0,
            },
            ControlTarget::Crossfader,
        );
        binding.mode = MappingMode::RelativeTwosComplement;
        assert!(
            binding
                .apply(
                    MidiMessage::ControlChange {
                        channel: 0,
                        controller: 1,
                        value: 1,
                    },
                    0.5,
                )
                .is_some_and(|value| value > 0.5)
        );
        assert!(
            binding
                .apply(
                    MidiMessage::ControlChange {
                        channel: 0,
                        controller: 1,
                        value: 127,
                    },
                    0.5,
                )
                .is_some_and(|value| value < 0.5)
        );
    }

    #[test]
    fn clear_target_removes_bindings_and_cancels_matching_learn() {
        let mut mapper = MidiMapper::default();
        mapper.bindings.push(MidiBinding::learned(
            "controller",
            MidiMessage::ControlChange {
                channel: 0,
                controller: 7,
                value: 0,
            },
            ControlTarget::DeckLevel(0),
        ));
        mapper.learn(ControlTarget::DeckLevel(0));
        mapper.clear_target(ControlTarget::DeckLevel(0));
        assert!(mapper.bindings.is_empty());
        assert_eq!(mapper.learning(), None);
    }

    #[test]
    fn momentary_mode_honors_inversion_and_output_range() {
        let mut binding = MidiBinding::learned(
            "controller",
            MidiMessage::NoteOn {
                channel: 0,
                note: 10,
                velocity: 127,
            },
            ControlTarget::DeckFreeze(0),
        );
        binding.mode = MappingMode::Momentary;
        binding.invert = true;
        binding.output_range = [-1.0, 1.0];
        assert_eq!(
            binding.apply(
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 10,
                    velocity: 127,
                },
                0.0,
            ),
            Some(-1.0)
        );
        assert_eq!(
            binding.apply(
                MidiMessage::NoteOff {
                    channel: 0,
                    note: 10,
                },
                0.0,
            ),
            Some(1.0)
        );
    }

    #[test]
    fn effect_parameter_keys_are_stable_and_namespaced() {
        assert_eq!(
            effect_parameter_key("chromatic-split", "amount"),
            effect_parameter_key("chromatic-split", "amount")
        );
        assert_ne!(
            effect_parameter_key("chromatic-split", "amount"),
            effect_parameter_key("chromatic-split", "angle")
        );
        assert_ne!(
            effect_parameter_key("chromatic-split", "amount"),
            effect_parameter_key("other", "amount")
        );
    }
}
