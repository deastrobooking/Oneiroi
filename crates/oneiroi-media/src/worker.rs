//! Bounded decoder actor used by each active mixer deck.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use oneiroi_core::MediaTime;
use oneiroi_hap::Decoder as HapDecoder;

use crate::{DecodePath, FfmpegVideoDecoder, HapDemuxer, ScheduledFrame, VideoFramePayload};

#[derive(Debug)]
enum DecoderCommand {
    Load {
        path: PathBuf,
        decode_path: DecodePath,
        generation: u64,
        start_at: Option<MediaTime>,
    },
    Stop,
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecoderEvent {
    Loaded { generation: u64 },
    Ended { generation: u64 },
    Error { generation: u64, message: String },
}

pub struct DeckDecoder {
    commands: mpsc::Sender<DecoderCommand>,
    frames: Receiver<ScheduledFrame<VideoFramePayload>>,
    events: Receiver<DecoderEvent>,
    worker: Option<JoinHandle<()>>,
}

impl DeckDecoder {
    pub fn spawn(frame_capacity: usize) -> Self {
        let (commands_tx, commands_rx) = mpsc::channel();
        let (frames_tx, frames_rx) = mpsc::sync_channel(frame_capacity.max(1));
        let (events_tx, events_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("oneiroi-deck-decoder".to_owned())
            .spawn(move || decoder_loop(commands_rx, frames_tx, events_tx))
            .expect("spawn deck decoder");
        Self {
            commands: commands_tx,
            frames: frames_rx,
            events: events_rx,
            worker: Some(worker),
        }
    }

    pub fn load(&self, path: PathBuf, decode_path: DecodePath, generation: u64) {
        self.load_at(path, decode_path, generation, None);
    }

    /// Reopens the source and discards decoded frames before `start_at`.
    ///
    /// Reopening is intentionally performed on the worker. It provides a
    /// codec-independent seek path while container-specific random access is
    /// introduced, without ever blocking the render thread.
    pub fn load_at(
        &self,
        path: PathBuf,
        decode_path: DecodePath,
        generation: u64,
        start_at: Option<MediaTime>,
    ) {
        let _ = self.commands.send(DecoderCommand::Load {
            path,
            decode_path,
            generation,
            start_at,
        });
    }

    pub fn stop(&self) {
        let _ = self.commands.send(DecoderCommand::Stop);
    }

    pub fn try_frame(&self) -> Result<ScheduledFrame<VideoFramePayload>, TryRecvError> {
        self.frames.try_recv()
    }

    pub fn try_event(&self) -> Result<DecoderEvent, TryRecvError> {
        self.events.try_recv()
    }

    pub fn recv_frame_timeout(
        &self,
        timeout: Duration,
    ) -> Result<ScheduledFrame<VideoFramePayload>, mpsc::RecvTimeoutError> {
        self.frames.recv_timeout(timeout)
    }

    pub fn recv_event_timeout(
        &self,
        timeout: Duration,
    ) -> Result<DecoderEvent, mpsc::RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }
}

impl Drop for DeckDecoder {
    fn drop(&mut self) {
        let _ = self.commands.send(DecoderCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum Session {
    Hap {
        demuxer: HapDemuxer,
        decoder: HapDecoder,
        generation: u64,
        skip_before: Option<MediaTime>,
    },
    Ffmpeg {
        decoder: FfmpegVideoDecoder,
        generation: u64,
        skip_before: Option<MediaTime>,
    },
}

impl Session {
    fn generation(&self) -> u64 {
        match self {
            Self::Hap { generation, .. } | Self::Ffmpeg { generation, .. } => *generation,
        }
    }

    fn next_frame(&mut self) -> Result<Option<ScheduledFrame<VideoFramePayload>>, String> {
        match self {
            Self::Hap {
                demuxer,
                decoder,
                generation,
                skip_before,
            } => loop {
                let frame = demuxer
                    .next_scheduled(decoder, *generation)
                    .map_err(|error| error.to_string())?;
                let Some(frame) = frame else {
                    return Ok(None);
                };
                if skip_before.is_some_and(|target| frame.pts < target) {
                    continue;
                }
                *skip_before = None;
                return Ok(Some(ScheduledFrame {
                    pts: frame.pts,
                    duration: frame.duration,
                    generation: frame.generation,
                    sequence: frame.sequence,
                    payload: VideoFramePayload::BlockCompressed(frame.payload),
                }));
            },
            Self::Ffmpeg {
                decoder,
                generation,
                skip_before,
            } => loop {
                let frame = decoder.next_frame().map_err(|error| error.to_string())?;
                let Some(frame) = frame else {
                    return Ok(None);
                };
                if skip_before.is_some_and(|target| frame.pts < target) {
                    continue;
                }
                *skip_before = None;
                return Ok(Some(ScheduledFrame {
                    pts: frame.pts,
                    duration: frame.duration,
                    generation: *generation,
                    sequence: frame.sequence,
                    payload: VideoFramePayload::Rgba8(frame.pixels),
                }));
            },
        }
    }
}

fn decoder_loop(
    commands: Receiver<DecoderCommand>,
    frames: SyncSender<ScheduledFrame<VideoFramePayload>>,
    events: mpsc::Sender<DecoderEvent>,
) {
    let mut session = None;
    let mut pending = None;
    loop {
        match commands.try_recv() {
            Ok(command) => {
                if handle_command(command, &mut session, &mut pending, &events) {
                    break;
                }
            }
            Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }

        if pending.is_none()
            && let Some(active) = session.as_mut()
        {
            match active.next_frame() {
                Ok(Some(frame)) => pending = Some(frame),
                Ok(None) => {
                    let generation = active.generation();
                    let _ = events.send(DecoderEvent::Ended { generation });
                    session = None;
                }
                Err(message) => {
                    let generation = active.generation();
                    let _ = events.send(DecoderEvent::Error {
                        generation,
                        message,
                    });
                    session = None;
                }
            }
        }

        if let Some(frame) = pending.take() {
            match frames.try_send(frame) {
                Ok(()) => {}
                Err(TrySendError::Full(frame)) => {
                    pending = Some(frame);
                    match commands.recv_timeout(Duration::from_millis(2)) {
                        Ok(command) => {
                            if handle_command(command, &mut session, &mut pending, &events) {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
                Err(TrySendError::Disconnected(_)) => break,
            }
        } else if session.is_none() {
            match commands.recv() {
                Ok(command) => {
                    if handle_command(command, &mut session, &mut pending, &events) {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }
}

fn handle_command(
    command: DecoderCommand,
    session: &mut Option<Session>,
    pending: &mut Option<ScheduledFrame<VideoFramePayload>>,
    events: &mpsc::Sender<DecoderEvent>,
) -> bool {
    match command {
        DecoderCommand::Load {
            path,
            decode_path,
            generation,
            start_at,
        } => {
            *pending = None;
            let opened = match decode_path {
                DecodePath::DirectHap => HapDemuxer::open(&path)
                    .map(|demuxer| Session::Hap {
                        demuxer,
                        decoder: HapDecoder::default(),
                        generation,
                        skip_before: start_at,
                    })
                    .map_err(|error| error.to_string()),
                DecodePath::FfmpegVideo => FfmpegVideoDecoder::open(&path)
                    .map(|decoder| Session::Ffmpeg {
                        decoder,
                        generation,
                        skip_before: start_at,
                    })
                    .map_err(|error| error.to_string()),
            };
            match opened {
                Ok(opened) => {
                    *session = Some(opened);
                    let _ = events.send(DecoderEvent::Loaded { generation });
                }
                Err(message) => {
                    *session = None;
                    let _ = events.send(DecoderEvent::Error {
                        generation,
                        message,
                    });
                }
            }
            false
        }
        DecoderCommand::Stop => {
            *session = None;
            *pending = None;
            false
        }
        DecoderCommand::Shutdown => true,
    }
}
