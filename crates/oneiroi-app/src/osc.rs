//! Bounded OSC 1.0 UDP input and VJX route mapping.

use std::io;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use oneiroi_core::{ControlTarget, ControlUpdate};

const MAX_PACKET_BYTES: usize = 65_535;
const MAX_BUNDLE_DEPTH: usize = 8;
const EVENT_QUEUE_CAPACITY: usize = 256;
const NTP_UNIX_EPOCH_OFFSET: u64 = 2_208_988_800;

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
    pub timetag: Option<u64>,
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
                if decode_packet(&bytes[..size], &mut messages, 0, None).is_err() {
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

fn decode_packet(
    bytes: &[u8],
    output: &mut Vec<OscMessage>,
    depth: usize,
    inherited_timetag: Option<u64>,
) -> Result<(), ()> {
    if depth > MAX_BUNDLE_DEPTH {
        return Err(());
    }
    let mut cursor = 0;
    let head = read_string(bytes, &mut cursor)?;
    if head == "#bundle" {
        let bundle_timetag = read_u64(bytes, &mut cursor)?;
        let timetag = (bundle_timetag != 1)
            .then_some(bundle_timetag)
            .or(inherited_timetag);
        while cursor < bytes.len() {
            let size = usize::try_from(read_i32(bytes, &mut cursor)?).map_err(|_| ())?;
            let end = cursor
                .checked_add(size)
                .filter(|end| *end <= bytes.len())
                .ok_or(())?;
            decode_packet(&bytes[cursor..end], output, depth + 1, timetag)?;
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
        timetag: inherited_timetag,
    });
    Ok(())
}

pub(crate) fn instant_for_timetag(
    timetag: Option<u64>,
    system_now: SystemTime,
    instant_now: Instant,
) -> Instant {
    let Some(timetag) = timetag else {
        return instant_now;
    };
    let seconds = timetag >> 32;
    if seconds < NTP_UNIX_EPOCH_OFFSET {
        return instant_now;
    }
    let fractional = timetag as u32;
    let unix_seconds = seconds - NTP_UNIX_EPOCH_OFFSET;
    let nanos = ((u64::from(fractional) * 1_000_000_000) >> 32) as u32;
    let target = UNIX_EPOCH + Duration::new(unix_seconds, nanos);
    target
        .duration_since(system_now)
        .ok()
        .and_then(|delay| instant_now.checked_add(delay))
        .unwrap_or(instant_now)
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OscOutputStats {
    pub sent: u64,
    pub dropped: u64,
    pub errors: u64,
}

#[derive(Default)]
struct SharedOutputStats {
    sent: AtomicU64,
    dropped: AtomicU64,
    errors: AtomicU64,
}

#[derive(Clone, Debug)]
struct FeedbackMessage {
    address: String,
    value: f32,
}

pub(crate) struct OscOutput {
    sender: Option<mpsc::SyncSender<FeedbackMessage>>,
    stats: Arc<SharedOutputStats>,
    worker: Option<JoinHandle<()>>,
    destination: String,
}

impl OscOutput {
    pub(crate) fn connect(destination: &str) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").context("bind OSC feedback socket")?;
        socket
            .connect(destination)
            .with_context(|| format!("connect OSC feedback to {destination}"))?;
        let destination = socket
            .peer_addr()
            .context("read OSC feedback destination")?
            .to_string();
        let (sender, receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let stats = Arc::new(SharedOutputStats::default());
        let worker_stats = Arc::clone(&stats);
        let worker = thread::Builder::new()
            .name("oneiroi-osc-output".to_owned())
            .spawn(move || feedback_loop(socket, receiver, worker_stats))
            .context("spawn OSC output worker")?;
        Ok(Self {
            sender: Some(sender),
            stats,
            worker: Some(worker),
            destination,
        })
    }

    pub(crate) fn destination(&self) -> &str {
        &self.destination
    }

    pub(crate) fn try_feedback(&self, address: String, value: f32) {
        let Some(sender) = &self.sender else {
            return;
        };
        if sender.try_send(FeedbackMessage { address, value }).is_err() {
            self.stats.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn stats(&self) -> OscOutputStats {
        OscOutputStats {
            sent: self.stats.sent.load(Ordering::Relaxed),
            dropped: self.stats.dropped.load(Ordering::Relaxed),
            errors: self.stats.errors.load(Ordering::Relaxed),
        }
    }
}

impl Drop for OscOutput {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn feedback_loop(
    socket: UdpSocket,
    receiver: mpsc::Receiver<FeedbackMessage>,
    stats: Arc<SharedOutputStats>,
) {
    while let Ok(message) = receiver.recv() {
        let packet = encode_float_message(&message.address, message.value);
        match socket.send(&packet) {
            Ok(size) if size == packet.len() => {
                stats.sent.fetch_add(1, Ordering::Relaxed);
            }
            Ok(_) | Err(_) => {
                stats.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn encode_float_message(address: &str, value: f32) -> Vec<u8> {
    let mut packet = encode_string(address);
    packet.extend(encode_string(",f"));
    packet.extend(value.to_bits().to_be_bytes());
    packet
}

fn encode_string(value: &str) -> Vec<u8> {
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
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

pub(crate) fn feedback_for_control(update: ControlUpdate) -> Option<(String, f32)> {
    let address = match update.target {
        ControlTarget::Crossfader => "/vjx/crossfader".to_owned(),
        ControlTarget::MasterOpacity => "/vjx/master/opacity".to_owned(),
        ControlTarget::MasterBlackout => "/vjx/master/blackout".to_owned(),
        ControlTarget::MasterFreeze => "/vjx/master/freeze".to_owned(),
        ControlTarget::DeckLevel(deck) => format!("/vjx/deck/{}/level", deck + 1),
        ControlTarget::DeckPlay(deck) => format!("/vjx/deck/{}/play", deck + 1),
        ControlTarget::DeckFreeze(deck) => format!("/vjx/deck/{}/freeze", deck + 1),
        ControlTarget::DeckSpeed(deck) => format!("/vjx/deck/{}/speed", deck + 1),
        ControlTarget::DeckSelect(deck) => format!("/vjx/deck/{}/select", deck + 1),
        ControlTarget::DeckRestart(deck) => format!("/vjx/deck/{}/restart", deck + 1),
        ControlTarget::ClipLaunch { deck, slot } => {
            format!("/vjx/deck/{}/clip/{}/launch", deck + 1, slot + 1)
        }
        ControlTarget::SceneLaunch(slot) => format!("/vjx/scene/{}/launch", slot + 1),
        ControlTarget::DeckEffectParameter {
            deck,
            parameter_key,
        } => format!("/vjx/deck/{}/package/{parameter_key:016x}", deck + 1),
        ControlTarget::TapTempo
        | ControlTarget::EffectParameter { .. }
        | ControlTarget::LfoParameter { .. }
        | ControlTarget::ModRouteParameter { .. }
        | ControlTarget::MasterEffectParameter { .. } => return None,
    };
    Some((address, update.value))
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
        ["package", parameter_key] => continuous(
            ControlTarget::DeckEffectParameter {
                deck,
                parameter_key: u64::from_str_radix(parameter_key, 16)
                    .ok()
                    .filter(|key| *key != 0)?,
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
        decode_packet(
            &float_message("/vjx/crossfader", 0.75),
            &mut output,
            0,
            None,
        )
        .unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].address, "/vjx/crossfader");
        assert_eq!(output[0].arguments, vec![OscArgument::Float(0.75)]);
    }

    #[test]
    fn maps_human_numbered_decks_clips_and_scenes() {
        let clip = OscMessage {
            address: "/vjx/deck/4/clip/8/launch".to_owned(),
            arguments: Vec::new(),
            timetag: None,
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
            timetag: None,
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
    fn maps_and_feeds_back_stable_deck_package_parameters() {
        let key = oneiroi_core::effect_parameter_key("recursive-2d", "iterations");
        let message = OscMessage {
            address: format!("/vjx/deck/3/package/{key:016x}"),
            arguments: vec![OscArgument::Float(0.75)],
            timetag: None,
        };
        let update = ControlUpdate {
            target: ControlTarget::DeckEffectParameter {
                deck: 2,
                parameter_key: key,
            },
            value: 0.75,
        };
        assert_eq!(map_message(&message), Some(OscAction::Control(update)));
        assert_eq!(
            feedback_for_control(update),
            Some((message.address, update.value))
        );
    }

    #[test]
    fn rejects_truncated_and_out_of_range_messages() {
        let mut output = Vec::new();
        assert!(decode_packet(b"/vjx/bad", &mut output, 0, None).is_err());
        assert_eq!(
            map_message(&OscMessage {
                address: "/vjx/deck/5/level".to_owned(),
                arguments: vec![OscArgument::Float(0.5)],
                timetag: None,
            }),
            None
        );
    }

    #[test]
    fn bundle_timetag_is_inherited_by_contained_messages() {
        let message = float_message("/vjx/tempo", 128.0);
        let timetag = ((NTP_UNIX_EPOCH_OFFSET + 100) << 32) | 7;
        let mut bundle = osc_string("#bundle");
        bundle.extend(timetag.to_be_bytes());
        bundle.extend((message.len() as i32).to_be_bytes());
        bundle.extend(message);
        let mut output = Vec::new();

        decode_packet(&bundle, &mut output, 0, None).unwrap();

        assert_eq!(output.len(), 1);
        assert_eq!(output[0].timetag, Some(timetag));
    }

    #[test]
    fn ntp_timetag_maps_to_a_monotonic_deadline() {
        let system_now = UNIX_EPOCH + Duration::from_secs(1_000);
        let instant_now = Instant::now();
        let target_seconds = NTP_UNIX_EPOCH_OFFSET + 1_002;
        let deadline = instant_for_timetag(Some(target_seconds << 32), system_now, instant_now);
        assert_eq!(deadline.duration_since(instant_now), Duration::from_secs(2));
    }

    #[test]
    fn feedback_encoder_round_trips_through_the_decoder() {
        let packet = encode_float_message("/vjx/deck/2/level", 0.625);
        let mut output = Vec::new();
        decode_packet(&packet, &mut output, 0, None).unwrap();
        assert_eq!(
            output,
            vec![OscMessage {
                address: "/vjx/deck/2/level".to_owned(),
                arguments: vec![OscArgument::Float(0.625)],
                timetag: None,
            }]
        );
    }
}
