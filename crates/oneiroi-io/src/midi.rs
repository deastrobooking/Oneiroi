//! Non-blocking platform MIDI input.
//!
//! The platform callback parses a complete MIDI packet and attempts to place
//! it in a bounded queue. It never waits on the render/UI thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use midir::{Ignore, MidiInput, MidiInputConnection as PlatformConnection, MidiInputPort};
use oneiroi_core::MidiMessage;

const MIDI_QUEUE_CAPACITY: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MidiInputDevice {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MidiInputEvent {
    pub timestamp_micros: u64,
    pub message: MidiMessage,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MidiInputStats {
    pub received: u64,
    pub dropped: u64,
    pub parse_errors: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum MidiInputError {
    #[error("could not initialize MIDI input: {0}")]
    Initialize(String),
    #[error("MIDI input device is no longer available: {0}")]
    DeviceMissing(String),
    #[error("could not connect MIDI input: {0}")]
    Connect(String),
}

pub fn discover_midi_inputs() -> Result<Vec<MidiInputDevice>, MidiInputError> {
    let input = MidiInput::new("oneiroi-discovery")
        .map_err(|error| MidiInputError::Initialize(error.to_string()))?;
    describe_ports(&input)
}

pub struct MidiInputConnection {
    device_id: String,
    receiver: Receiver<MidiInputEvent>,
    received: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    parse_errors: Arc<AtomicU64>,
    _connection: PlatformConnection<()>,
}

impl MidiInputConnection {
    pub fn connect(device_id: &str) -> Result<Self, MidiInputError> {
        let mut input = MidiInput::new("oneiroi-input")
            .map_err(|error| MidiInputError::Initialize(error.to_string()))?;
        input.ignore(Ignore::None);
        let devices = describe_ports_with_handles(&input)?;
        let Some((_, port)) = devices
            .into_iter()
            .find(|(device, _)| device.id == device_id)
        else {
            return Err(MidiInputError::DeviceMissing(device_id.to_owned()));
        };

        let (sender, receiver) = sync_channel(MIDI_QUEUE_CAPACITY);
        let received = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let parse_errors = Arc::new(AtomicU64::new(0));
        let callback_received = received.clone();
        let callback_dropped = dropped.clone();
        let callback_parse_errors = parse_errors.clone();
        let connection = input
            .connect(
                &port,
                "oneiroi-capture",
                move |timestamp_micros, bytes, ()| {
                    if !bytes
                        .first()
                        .is_some_and(|status| matches!(status & 0xf0, 0x80 | 0x90 | 0xb0 | 0xe0))
                    {
                        return;
                    }
                    let Some(message) = parse_midi_message(bytes) else {
                        callback_parse_errors.fetch_add(1, Ordering::Relaxed);
                        return;
                    };
                    callback_received.fetch_add(1, Ordering::Relaxed);
                    enqueue(
                        &sender,
                        MidiInputEvent {
                            timestamp_micros,
                            message,
                        },
                        &callback_dropped,
                    );
                },
                (),
            )
            .map_err(|error| MidiInputError::Connect(error.to_string()))?;
        Ok(Self {
            device_id: device_id.to_owned(),
            receiver,
            received,
            dropped,
            parse_errors,
            _connection: connection,
        })
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn try_iter(&self) -> impl Iterator<Item = MidiInputEvent> + '_ {
        self.receiver.try_iter()
    }

    pub fn stats(&self) -> MidiInputStats {
        MidiInputStats {
            received: self.received.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            parse_errors: self.parse_errors.load(Ordering::Relaxed),
        }
    }
}

fn enqueue(sender: &SyncSender<MidiInputEvent>, event: MidiInputEvent, dropped: &AtomicU64) {
    if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) = sender.try_send(event) {
        dropped.fetch_add(1, Ordering::Relaxed);
    }
}

fn describe_ports(input: &MidiInput) -> Result<Vec<MidiInputDevice>, MidiInputError> {
    describe_ports_with_handles(input)
        .map(|devices| devices.into_iter().map(|(device, _)| device).collect())
}

fn describe_ports_with_handles(
    input: &MidiInput,
) -> Result<Vec<(MidiInputDevice, MidiInputPort)>, MidiInputError> {
    let mut labels = std::collections::HashMap::<String, usize>::new();
    input
        .ports()
        .into_iter()
        .map(|port| {
            let label = input
                .port_name(&port)
                .map_err(|error| MidiInputError::Initialize(error.to_string()))?;
            let duplicate = labels.entry(label.clone()).or_default();
            let id = if *duplicate == 0 {
                label.clone()
            } else {
                format!("{label} #{}", *duplicate + 1)
            };
            *duplicate += 1;
            Ok((MidiInputDevice { id, label }, port))
        })
        .collect()
}

pub fn parse_midi_message(bytes: &[u8]) -> Option<MidiMessage> {
    let (&status, data) = bytes.split_first()?;
    let channel = status & 0x0f;
    match status & 0xf0 {
        0x80 => Some(MidiMessage::NoteOff {
            channel,
            note: *data.first()? & 0x7f,
        }),
        0x90 => {
            let note = *data.first()? & 0x7f;
            let velocity = *data.get(1)? & 0x7f;
            if velocity == 0 {
                Some(MidiMessage::NoteOff { channel, note })
            } else {
                Some(MidiMessage::NoteOn {
                    channel,
                    note,
                    velocity,
                })
            }
        }
        0xb0 => Some(MidiMessage::ControlChange {
            channel,
            controller: *data.first()? & 0x7f,
            value: *data.get(1)? & 0x7f,
        }),
        0xe0 => Some(MidiMessage::PitchBend {
            channel,
            value: u16::from(*data.first()? & 0x7f) | (u16::from(*data.get(1)? & 0x7f) << 7),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_on_note_off_and_velocity_zero() {
        assert_eq!(
            parse_midi_message(&[0x92, 60, 100]),
            Some(MidiMessage::NoteOn {
                channel: 2,
                note: 60,
                velocity: 100,
            })
        );
        assert_eq!(
            parse_midi_message(&[0x92, 60, 0]),
            Some(MidiMessage::NoteOff {
                channel: 2,
                note: 60,
            })
        );
        assert_eq!(
            parse_midi_message(&[0x82, 60, 40]),
            Some(MidiMessage::NoteOff {
                channel: 2,
                note: 60,
            })
        );
    }

    #[test]
    fn parses_control_change_and_pitch_bend() {
        assert_eq!(
            parse_midi_message(&[0xb7, 74, 127]),
            Some(MidiMessage::ControlChange {
                channel: 7,
                controller: 74,
                value: 127,
            })
        );
        assert_eq!(
            parse_midi_message(&[0xe0, 0, 64]),
            Some(MidiMessage::PitchBend {
                channel: 0,
                value: 8192,
            })
        );
    }

    #[test]
    fn rejects_short_and_unsupported_packets() {
        assert_eq!(parse_midi_message(&[]), None);
        assert_eq!(parse_midi_message(&[0xb0, 1]), None);
        assert_eq!(parse_midi_message(&[0xf8]), None);
    }
}
