//! MOV demux through libavformat without invoking a video decoder.

use std::path::{Path, PathBuf};

use ffmpeg_next as ffmpeg;
use oneiroi_core::{MediaTime, MediaTimeError};
use oneiroi_hap::{DecodedFrame, Decoder, HapError};
use thiserror::Error;

use crate::ScheduledFrame;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameRate {
    pub numerator: i32,
    pub denominator: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HapStreamMetadata {
    pub path: PathBuf,
    pub stream_index: usize,
    pub visible_extent: [u32; 2],
    pub codec_tag: [u8; 4],
    /// Exact number of seconds represented by one container timestamp tick.
    pub time_base: FrameRate,
    pub average_frame_rate: Option<FrameRate>,
    pub duration: Option<MediaTime>,
    pub frame_count: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedHapPacket {
    pub pts: Option<MediaTime>,
    pub dts: Option<MediaTime>,
    pub duration: Option<MediaTime>,
    pub sequence: u64,
    pub keyframe: bool,
    pub file_position: Option<i64>,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DemuxedHapFrame {
    pub pts: Option<MediaTime>,
    pub dts: Option<MediaTime>,
    pub duration: Option<MediaTime>,
    pub sequence: u64,
    pub frame: DecodedFrame,
}

#[derive(Debug, Error)]
pub enum DemuxError {
    #[error("initialize FFmpeg: {0}")]
    Initialize(ffmpeg::Error),
    #[error("open media file {path}: {source}")]
    Open {
        path: PathBuf,
        source: ffmpeg::Error,
    },
    #[error("expected a QuickTime/MOV container, found {0}")]
    UnsupportedContainer(String),
    #[error("no HAP video stream found")]
    NoHapStream,
    #[error("HAP stream has invalid dimensions {width}x{height}")]
    InvalidDimensions { width: i32, height: i32 },
    #[error("HAP stream has invalid time base {numerator}/{denominator}")]
    InvalidTimeBase { numerator: i32, denominator: i32 },
    #[error("read HAP packet: {0}")]
    Read(ffmpeg::Error),
    #[error("HAP packet {sequence} is marked corrupt")]
    CorruptPacket { sequence: u64 },
    #[error("HAP packet {sequence} has no payload")]
    EmptyPacket { sequence: u64 },
    #[error("invalid packet timestamp: {0}")]
    Timestamp(#[from] MediaTimeError),
    #[error("decode HAP packet {sequence}: {source}")]
    Decode { sequence: u64, source: HapError },
    #[error("HAP packet {sequence} has neither a presentation nor decode timestamp")]
    MissingTimestamp { sequence: u64 },
}

/// An opened HAP MOV stream.
///
/// FFmpeg is used only for container probing and `AVPacket` reads. Packet
/// payloads go directly to `oneiroi-hap`; no `AVCodecContext` is created.
pub struct HapDemuxer {
    input: ffmpeg::format::context::Input,
    metadata: HapStreamMetadata,
    sequence: u64,
}

impl HapDemuxer {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DemuxError> {
        let path = path.as_ref();
        ffmpeg::init().map_err(DemuxError::Initialize)?;
        let input = ffmpeg::format::input(path).map_err(|source| DemuxError::Open {
            path: path.to_path_buf(),
            source,
        })?;

        let container = input.format().name().to_owned();
        if !container.split(',').any(|name| name == "mov") {
            return Err(DemuxError::UnsupportedContainer(container));
        }

        let stream = input
            .streams()
            .find(|stream| {
                let parameters = stream.parameters();
                parameters.medium() == ffmpeg::media::Type::Video
                    && parameters.id() == ffmpeg::codec::Id::HAP
            })
            .ok_or(DemuxError::NoHapStream)?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
        if time_base.numerator() <= 0 || time_base.denominator() <= 0 {
            return Err(DemuxError::InvalidTimeBase {
                numerator: time_base.numerator(),
                denominator: time_base.denominator(),
            });
        }

        let parameters = stream.parameters();
        // SAFETY: parameters owns a live AVCodecParameters for the duration
        // of this scope. FFmpeg exposes no safe width/tag accessors.
        let (width, height, codec_tag) = unsafe {
            let parameters = &*parameters.as_ptr();
            (parameters.width, parameters.height, parameters.codec_tag)
        };
        if width <= 0 || height <= 0 {
            return Err(DemuxError::InvalidDimensions { width, height });
        }

        let average_frame_rate = rational_if_valid(stream.avg_frame_rate());
        let duration = if stream.duration() == ffmpeg::ffi::AV_NOPTS_VALUE || stream.duration() < 0
        {
            None
        } else {
            Some(media_time(stream.duration(), time_base)?)
        };
        let frame_count = u64::try_from(stream.frames())
            .ok()
            .filter(|count| *count > 0);
        let metadata = HapStreamMetadata {
            path: path.to_path_buf(),
            stream_index,
            visible_extent: [width as u32, height as u32],
            codec_tag: codec_tag.to_le_bytes(),
            time_base: FrameRate {
                numerator: time_base.numerator(),
                denominator: time_base.denominator(),
            },
            average_frame_rate,
            duration,
            frame_count,
        };

        Ok(Self {
            input,
            metadata,
            sequence: 0,
        })
    }

    pub fn metadata(&self) -> &HapStreamMetadata {
        &self.metadata
    }

    pub fn next_packet(&mut self) -> Result<Option<EncodedHapPacket>, DemuxError> {
        loop {
            let mut packet = ffmpeg::Packet::empty();
            match packet.read(&mut self.input) {
                Ok(()) => {}
                Err(ffmpeg::Error::Eof) => return Ok(None),
                Err(error) => return Err(DemuxError::Read(error)),
            }
            if packet.stream() != self.metadata.stream_index {
                continue;
            }

            let sequence = self.sequence;
            self.sequence = self.sequence.wrapping_add(1);
            if packet.is_corrupt() {
                return Err(DemuxError::CorruptPacket { sequence });
            }
            let data = packet
                .data()
                .filter(|data| !data.is_empty())
                .ok_or(DemuxError::EmptyPacket { sequence })?
                .to_vec();
            let time_base = self.metadata.time_base;
            let convert = |timestamp| {
                MediaTime::from_time_base(timestamp, time_base.numerator, time_base.denominator)
            };

            return Ok(Some(EncodedHapPacket {
                pts: packet.pts().map(convert).transpose()?,
                dts: packet.dts().map(convert).transpose()?,
                duration: (packet.duration() > 0)
                    .then(|| convert(packet.duration()))
                    .transpose()?,
                sequence,
                keyframe: packet.is_key(),
                file_position: i64::try_from(packet.position())
                    .ok()
                    .filter(|position| *position >= 0),
                data,
            }));
        }
    }

    pub fn next_decoded(
        &mut self,
        decoder: &Decoder,
    ) -> Result<Option<DemuxedHapFrame>, DemuxError> {
        let Some(packet) = self.next_packet()? else {
            return Ok(None);
        };
        let [width, height] = self.metadata.visible_extent;
        let frame = decoder
            .decode(&packet.data, width, height)
            .map_err(|source| DemuxError::Decode {
                sequence: packet.sequence,
                source,
            })?;
        Ok(Some(DemuxedHapFrame {
            pts: packet.pts,
            dts: packet.dts,
            duration: packet.duration,
            sequence: packet.sequence,
            frame,
        }))
    }

    /// Demux and decode the next frame into the generation-tagged shape used
    /// by the render-time scheduler.
    pub fn next_scheduled(
        &mut self,
        decoder: &Decoder,
        generation: u64,
    ) -> Result<Option<ScheduledFrame<DecodedFrame>>, DemuxError> {
        let Some(frame) = self.next_decoded(decoder)? else {
            return Ok(None);
        };
        // HAP is intra-frame and has no frame reordering, so DTS is a valid
        // fallback for malformed files that omit PTS.
        let pts = frame
            .pts
            .or(frame.dts)
            .ok_or(DemuxError::MissingTimestamp {
                sequence: frame.sequence,
            })?;
        Ok(Some(ScheduledFrame {
            pts,
            duration: frame.duration,
            generation,
            sequence: frame.sequence,
            payload: frame.frame,
        }))
    }
}

fn media_time(timestamp: i64, time_base: ffmpeg::Rational) -> Result<MediaTime, MediaTimeError> {
    MediaTime::from_time_base(timestamp, time_base.numerator(), time_base.denominator())
}

fn rational_if_valid(value: ffmpeg::Rational) -> Option<FrameRate> {
    (value.numerator() > 0 && value.denominator() > 0).then_some(FrameRate {
        numerator: value.numerator(),
        denominator: value.denominator(),
    })
}
