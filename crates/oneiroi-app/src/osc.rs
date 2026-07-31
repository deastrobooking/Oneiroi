//! Bounded OSC 1.0 UDP input and VJX route mapping.

use std::io;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use oneiroi_core::{ControlTarget, ControlUpdate};

const MAX_PACKET_BYTES: usize = 65_535;
const MAX_BUNDLE_DEPTH: usize = 8;
const EVENT_QUEUE_CAPACITY: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OscArgument {
    Int(i32),
    Float(f32),
    String(String),
    Bool(bool),
    Double(f64),
}

impl OscArgument {
    fn number(&self) -> Option<f64> {
        match self {
            Self::Int(value) => Some(f64::from(*value)),
            Self::Float(value) => Some(f64::from(*value)),
            Self::Double(value) => Some(*value),
            Self::Bool(value) => Some(f64::from(*value)),
            Self::String(_) => None,
        }
    }

    fn boolean(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => self.number().map(|value| value >= 0.5),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OscMessage {
    pub address: String,
    pub arguments: Vec<OscArgument>,
}

#[derive(Clone, Debug)]
pub(crate) struct OscEvent {
    pub peer: String,
    pub message: OscMessage,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OscStats {
    pub packets: u64,
    pub messages: u64,
    pub malformed: u64,
    pub dropped: u64,
}

#[derive(Default)]
struct SharedStats {
    packets: AtomicU64,
    messages: AtomicU64,
    malformed: AtomicU64,
    dropped: AtomicU64,
}

pub(crate) struct OscInput {
    receiver: mpsc::Receiver<OscEvent>,
    stop: Arc<AtomicBool>,
    stats: Arc<SharedStats>,
    worker: Option<JoinHandle<()>>,
    local_address: String,
}

impl OscInput {
    pub(crate) fn bind(address: &str) -> Result<Self> {
        let socket = UdpSocket::bind(address).with_context(|| format!("bind OSC UDP {address}"))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .context("set OSC socket timeout")?;
        let local_address = socket
            .local_addr()
            .context("read OSC bind address")?
            .to_string();
        let (sender, receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(SharedStats::default());
        let worker_stop = Arc::clone(&stop);
        let worker_stats = Arc::clone(&stats);
        let worker = thread::Builder::new()
            .name("oneiroi-osc-input".to_owned())
            .spawn(move || receive_loop(socket, sender, worker_stop, worker_stats))
            .context("spawn OSC input worker")?;
        Ok(Self {
            receiver,
            stop,
            stats,
            worker: Some(worker),
            local_address,
        })
    }

    pub(crate) fn local_address(&self) -> &str {
        &self.local_address
    }

    pub(crate) fn try_iter(&self) -> mpsc::TryIter<'_, OscEvent> {
        self.receiver.try_iter()
    }

    pub(crate) fn stats(&self) -> OscStats {
        OscStats {
            packets: self.stats.packets.load(Ordering::Relaxed),
            messages: self.stats.messages.load(Ordering::Relaxed),
            malformed: self.stats.malformed.load(Ordering::Relaxed),
            dropped: self.stats.dropped.load(Ordering::Relaxed),
        }
    }
}

impl Drop for OscInput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn receive_loop(
    socket: UdpSocket,
    sender: mpsc::SyncSender<OscEvent>,
    stop: Arc<AtomicBool>,
    stats: Arc<SharedStats>,
) {
    let mut bytes = [0_u8; MAX_PACKET_BYTES];
    while !stop.load(Ordering::Acquire) {
        match socket.recv_from(&mut bytes) {
            Ok((size, peer)) => {
                stats.packets.fetch_add(1, Ordering::Relaxed);
                let mut messages = Vec::new();
                if decode_packet(&bytes[..size], &mut messages, 0).is_err() {
                    stats.malformed.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                for message in messages {
                    let event = OscEvent {
                        peer: peer.to_string(),
                        message,
                    };
                    match sender.try_send(event) {
                        Ok(()) => {
                            stats.messages.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(mpsc::TrySendError::Full(_)) => {
                            stats.dropped.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(mpsc::TrySendError::Disconnected(_)) => return,
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                stats.malformed.fetch_add(1, Ordering::Relaxed);
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn decode_packet(bytes: &[u8], output: &mut Vec<OscMessage>, depth: usize) -> Result<(), ()> {
    if depth > MAX_BUNDLE_DEPTH {
        return Err(());
    }
    let mut cursor = 0;
    let head = read_string(bytes, &mut cursor)?;
    if head == "#bundle" {
        cursor = cursor
            .checked_add(8)
            .filter(|cursor| *cursor <= bytes.len())
            .ok_or(())?;
        while cursor < bytes.len() {
            let size = usize::try_from(read_i32(bytes, &mut cursor)?).map_err(|_| ())?;
            let end = cursor
                .checked_add(size)
                .filter(|end| *end <= bytes.len())
                .ok_or(())?;
            decode_packet(&bytes[cursor..end], output, depth + 1)?;
            cursor = end;
        }
        return Ok(());
    }
    if !head.starts_with('/') {
        return Err(());
    }
    let tags = read_string(bytes, &mut cursor)?;
    let tags = tags.strip_prefix(',').ok_or(())?;
    let mut arguments = Vec::with_capacity(tags.len());
    for tag in tags.chars() {
        arguments.push(match tag {
            'i' => OscArgument::Int(read_i32(bytes, &mut cursor)?),
            'f' => OscArgument::Float(f32::from_bits(read_u32(bytes, &mut cursor)?)),
            'd' => OscArgument::Double(f64::from_bits(read_u64(bytes, &mut cursor)?)),
            's' => OscArgument::String(read_string(bytes, &mut cursor)?.to_owned()),
            'T' => OscArgument::Bool(true),
            'F' => OscArgument::Bool(false),
            _ => return Err(()),
        });
    }
    output.push(OscMessage {
        address: head.to_owned(),
        arguments,
    });
    Ok(())
}

fn read_string<'a>(bytes: &'a [u8], cursor: &mut usize) -> Result<&'a str, ()> {
    let remaining = bytes.get(*cursor..).ok_or(())?;
    let length = remaining.iter().position(|byte| *byte == 0).ok_or(())?;
    let value = std::str::from_utf8(&remaining[..length]).map_err(|_| ())?;
    let consumed = length.checked_add(1).ok_or(())?;
    let padded = consumed.checked_add(3).ok_or(())? & !3;
    *cursor = cursor
        .checked_add(padded)
        .filter(|end| *end <= bytes.len())
        .ok_or(())?;
    Ok(value)
}

fn read_i32(bytes: &[u8], cursor: &mut usize) -> Result<i32, ()> {
    Ok(i32::from_be_bytes(read_array(bytes, cursor)?))
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, ()> {
    Ok(u32::from_be_bytes(read_array(bytes, cursor)?))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, ()> {
    Ok(u64::from_be_bytes(read_array(bytes, cursor)?))
}

fn read_array<const N: usize>(bytes: &[u8], cursor: &mut usize) -> Result<[u8; N], ()> {
    let end = cursor
        .checked_add(N)
        .filter(|end| *end <= bytes.len())
        .ok_or(())?;
    let value = bytes[*cursor..end].try_into().map_err(|_| ())?;
    *cursor = end;
    Ok(value)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OscAction {
    Control(ControlUpdate),
    Tempo(f64),
    OutputEnabled(bool),
    OutputFullscreen(bool),
}

pub(crate) fn map_message(message: &OscMessage) -> Option<OscAction> {
    let value = message.arguments.first();
    let control = |target, default| {
        Some(OscAction::Control(ControlUpdate {
            target,
            value: value.and_then(OscArgument::number).unwrap_or(default) as f32,
        }))
    };
    match message.address.as_str() {
        "/vjx/crossfader" => control(ControlTarget::Crossfader, 0.0),
        "/vjx/master/opacity" => control(ControlTarget::MasterOpacity, 0.0),
        "/vjx/master/blackout" => control(ControlTarget::MasterBlackout, 1.0),
        "/vjx/master/freeze" => control(ControlTarget::MasterFreeze, 1.0),
        "/vjx/tempo" => value
            .and_then(OscArgument::number)
            .filter(|bpm| bpm.is_finite())
            .map(|bpm| OscAction::Tempo(bpm.clamp(20.0, 400.0))),
        "/vjx/output/enabled" => value
            .and_then(OscArgument::boolean)
            .map(OscAction::OutputEnabled),
        "/vjx/output/fullscreen" => value
            .and_then(OscArgument::boolean)
            .map(OscAction::OutputFullscreen),
        address => map_indexed_route(address, value),
    }
}

fn map_indexed_route(address: &str, value: Option<&OscArgument>) -> Option<OscAction> {
    let parts: Vec<_> = address.trim_matches('/').split('/').collect();
    if let ["vjx", "scene", slot, "launch"] = parts.as_slice() {
        return trigger(ControlTarget::SceneLaunch(one_based_index(slot, 8)?), value);
    }
    let ["vjx", "deck", deck, tail @ ..] = parts.as_slice() else {
        return None;
    };
    let deck = one_based_index(deck, 4)?;
    match tail {
        ["level"] => continuous(ControlTarget::DeckLevel(deck), value),
        ["play"] => continuous(ControlTarget::DeckPlay(deck), value),
        ["freeze"] => continuous(ControlTarget::DeckFreeze(deck), value),
        ["speed"] => continuous(ControlTarget::DeckSpeed(deck), value),
        ["select"] => trigger(ControlTarget::DeckSelect(deck), value),
        ["restart"] => trigger(ControlTarget::DeckRestart(deck), value),
        ["clip", slot, "launch"] => trigger(
            ControlTarget::ClipLaunch {
                deck,
                slot: one_based_index(slot, 8)?,
            },
            value,
        ),
        _ => None,
    }
}

fn one_based_index(value: &str, maximum: u8) -> Option<u8> {
    let value = value.parse::<u8>().ok()?;
    (1..=maximum).contains(&value).then_some(value - 1)
}

fn continuous(target: ControlTarget, value: Option<&OscArgument>) -> Option<OscAction> {
    Some(OscAction::Control(ControlUpdate {
        target,
        value: value?.number()? as f32,
    }))
}

fn trigger(target: ControlTarget, value: Option<&OscArgument>) -> Option<OscAction> {
    Some(OscAction::Control(ControlUpdate {
        target,
        value: value.and_then(OscArgument::number).unwrap_or(1.0) as f32,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osc_string(value: &str) -> Vec<u8> {
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        bytes
    }

    fn float_message(address: &str, value: f32) -> Vec<u8> {
        let mut bytes = osc_string(address);
        bytes.extend(osc_string(",f"));
        bytes.extend(value.to_bits().to_be_bytes());
        bytes
    }

    #[test]
    fn decodes_standard_float_message() {
        let mut output = Vec::new();
        decode_packet(&float_message("/vjx/crossfader", 0.75), &mut output, 0).unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].address, "/vjx/crossfader");
        assert_eq!(output[0].arguments, vec![OscArgument::Float(0.75)]);
    }

    #[test]
    fn maps_human_numbered_decks_clips_and_scenes() {
        let clip = OscMessage {
            address: "/vjx/deck/4/clip/8/launch".to_owned(),
            arguments: Vec::new(),
        };
        assert_eq!(
            map_message(&clip),
            Some(OscAction::Control(ControlUpdate {
                target: ControlTarget::ClipLaunch { deck: 3, slot: 7 },
                value: 1.0,
            }))
        );
        let scene = OscMessage {
            address: "/vjx/scene/1/launch".to_owned(),
            arguments: vec![OscArgument::Int(1)],
        };
        assert_eq!(
            map_message(&scene),
            Some(OscAction::Control(ControlUpdate {
                target: ControlTarget::SceneLaunch(0),
                value: 1.0,
            }))
        );
    }

    #[test]
    fn rejects_truncated_and_out_of_range_messages() {
        let mut output = Vec::new();
        assert!(decode_packet(b"/vjx/bad", &mut output, 0).is_err());
        assert_eq!(
            map_message(&OscMessage {
                address: "/vjx/deck/5/level".to_owned(),
                arguments: vec![OscArgument::Float(0.5)],
            }),
            None
        );
    }
}
