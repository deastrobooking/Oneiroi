//! Conventional codec fallback through FFmpeg and libswscale.

use std::path::{Path, PathBuf};

use ffmpeg_next as ffmpeg;
use oneiroi_core::{MediaTime, MediaTimeError};
use thiserror::Error;

use crate::RgbaFrame;

#[derive(Debug, Error)]
pub enum FfmpegDecodeError {
    #[error("initialize FFmpeg: {0}")]
    Initialize(ffmpeg::Error),
    #[error("open media file {path}: {source}")]
    Open {
        path: PathBuf,
        source: ffmpeg::Error,
    },
    #[error("file contains no video stream")]
    NoVideoStream,
    #[error("HAP must use the direct block-compressed decoder")]
    HapRequiresDirectDecoder,
    #[error("create video decoder: {0}")]
    CreateDecoder(ffmpeg::Error),
    #[error("create RGBA conversion pipeline: {0}")]
    CreateScaler(ffmpeg::Error),
    #[error("read encoded packet: {0}")]
    Read(ffmpeg::Error),
    #[error("submit encoded packet: {0}")]
    Submit(ffmpeg::Error),
    #[error("receive decoded frame: {0}")]
    Receive(ffmpeg::Error),
    #[error("convert decoded frame to RGBA: {0}")]
    Convert(ffmpeg::Error),
    #[error("decoded frame has invalid dimensions {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("decoded RGBA frame size overflow")]
    FrameSizeOverflow,
    #[error("decoded frame has no presentation timestamp")]
    MissingTimestamp,
    #[error("invalid decoded timestamp: {0}")]
    Timestamp(#[from] MediaTimeError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedRgbaFrame {
    pub pts: MediaTime,
    pub duration: Option<MediaTime>,
    pub sequence: u64,
    pub pixels: RgbaFrame,
}

pub struct FfmpegVideoDecoder {
    input: ffmpeg::format::context::Input,
    stream_index: usize,
    time_base: ffmpeg::Rational,
    average_duration: Option<MediaTime>,
    decoder: ffmpeg::decoder::Video,
    scaler: ffmpeg::software::scaling::Context,
    decoded: ffmpeg::frame::Video,
    converted: ffmpeg::frame::Video,
    draining: bool,
    sequence: u64,
}

impl FfmpegVideoDecoder {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FfmpegDecodeError> {
        let path = path.as_ref();
        ffmpeg::init().map_err(FfmpegDecodeError::Initialize)?;
        let input = ffmpeg::format::input(path).map_err(|source| FfmpegDecodeError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        let (stream_index, time_base, average_duration, decoder, scaler) = {
            let stream = input
                .streams()
                .best(ffmpeg::media::Type::Video)
                .ok_or(FfmpegDecodeError::NoVideoStream)?;
            let stream_index = stream.index();
            let time_base = stream.time_base();
            let parameters = stream.parameters();
            if parameters.id() == ffmpeg::codec::Id::HAP {
                return Err(FfmpegDecodeError::HapRequiresDirectDecoder);
            }
            let decoder = ffmpeg::codec::Context::from_parameters(parameters)
                .and_then(|context| context.decoder().video())
                .map_err(FfmpegDecodeError::CreateDecoder)?;
            let width = decoder.width();
            let height = decoder.height();
            if width == 0 || height == 0 {
                return Err(FfmpegDecodeError::InvalidDimensions { width, height });
            }
            let scaler = ffmpeg::software::scaling::Context::get(
                decoder.format(),
                width,
                height,
                ffmpeg::format::Pixel::RGBA,
                width,
                height,
                ffmpeg::software::scaling::Flags::BILINEAR,
            )
            .map_err(FfmpegDecodeError::CreateScaler)?;
            let average_duration = {
                let rate = stream.avg_frame_rate();
                if rate.numerator() > 0 && rate.denominator() > 0 {
                    Some(MediaTime::new(
                        i64::from(rate.denominator()),
                        i64::from(rate.numerator()),
                    )?)
                } else {
                    None
                }
            };

            (stream_index, time_base, average_duration, decoder, scaler)
        };

        Ok(Self {
            input,
            stream_index,
            time_base,
            average_duration,
            decoder,
            scaler,
            decoded: ffmpeg::frame::Video::empty(),
            converted: ffmpeg::frame::Video::empty(),
            draining: false,
            sequence: 0,
        })
    }

    pub fn next_frame(&mut self) -> Result<Option<DecodedRgbaFrame>, FfmpegDecodeError> {
        loop {
            match self.decoder.receive_frame(&mut self.decoded) {
                Ok(()) => return self.copy_decoded().map(Some),
                Err(ffmpeg::Error::Eof) => return Ok(None),
                Err(ffmpeg::Error::Other {
                    errno: ffmpeg::error::EAGAIN,
                }) => {}
                Err(error) => return Err(FfmpegDecodeError::Receive(error)),
            }

            if self.draining {
                return Ok(None);
            }
            let mut packet = ffmpeg::Packet::empty();
            loop {
                match packet.read(&mut self.input) {
                    Ok(()) if packet.stream() == self.stream_index => break,
                    Ok(()) => continue,
                    Err(ffmpeg::Error::Eof) => {
                        self.decoder.send_eof().map_err(FfmpegDecodeError::Submit)?;
                        self.draining = true;
                        break;
                    }
                    Err(error) => return Err(FfmpegDecodeError::Read(error)),
                }
            }
            if !self.draining {
                self.decoder
                    .send_packet(&packet)
                    .map_err(FfmpegDecodeError::Submit)?;
            }
        }
    }

    fn copy_decoded(&mut self) -> Result<DecodedRgbaFrame, FfmpegDecodeError> {
        let width = self.decoded.width();
        let height = self.decoded.height();
        if width == 0 || height == 0 {
            return Err(FfmpegDecodeError::InvalidDimensions { width, height });
        }
        if self.scaler.input().format != self.decoded.format()
            || self.scaler.input().width != width
            || self.scaler.input().height != height
        {
            self.scaler.cached(
                self.decoded.format(),
                width,
                height,
                ffmpeg::format::Pixel::RGBA,
                width,
                height,
                ffmpeg::software::scaling::Flags::BILINEAR,
            );
            self.converted = ffmpeg::frame::Video::empty();
        }
        self.scaler
            .run(&self.decoded, &mut self.converted)
            .map_err(FfmpegDecodeError::Convert)?;

        let row_bytes = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(4))
            .ok_or(FfmpegDecodeError::FrameSizeOverflow)?;
        let total_bytes = row_bytes
            .checked_mul(usize::try_from(height).map_err(|_| FfmpegDecodeError::FrameSizeOverflow)?)
            .ok_or(FfmpegDecodeError::FrameSizeOverflow)?;
        let stride = self.converted.stride(0);
        let source = self.converted.data(0);
        let mut data = vec![0_u8; total_bytes];
        for row in 0..height as usize {
            data[row * row_bytes..(row + 1) * row_bytes]
                .copy_from_slice(&source[row * stride..row * stride + row_bytes]);
        }

        let timestamp = self
            .decoded
            .timestamp()
            .or_else(|| self.decoded.pts())
            .ok_or(FfmpegDecodeError::MissingTimestamp)?;
        let pts = MediaTime::from_time_base(
            timestamp,
            self.time_base.numerator(),
            self.time_base.denominator(),
        )?;
        let raw_duration = self.decoded.packet().duration;
        let duration = if raw_duration > 0 {
            Some(MediaTime::from_time_base(
                raw_duration,
                self.time_base.numerator(),
                self.time_base.denominator(),
            )?)
        } else {
            self.average_duration
        };
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);

        Ok(DecodedRgbaFrame {
            pts,
            duration,
            sequence,
            pixels: RgbaFrame {
                extent: [width, height],
                data,
            },
        })
    }
}
