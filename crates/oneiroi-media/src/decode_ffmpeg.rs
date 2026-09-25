//! Conventional codec fallback through FFmpeg and libswscale.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ffmpeg_next as ffmpeg;
use oneiroi_core::{MediaTime, MediaTimeError};
use thiserror::Error;

use crate::capture_interrupt::{CaptureCancellation, CaptureInterrupt};

use crate::{CameraConfig, FrameBufferPool, RgbaFrame};

const LIVE_READ_RETRY_DELAY: Duration = Duration::from_millis(10);

fn is_temporary_live_read_error(live: bool, error: &ffmpeg::Error) -> bool {
    live && matches!(
        error,
        ffmpeg::Error::Other { errno } if *errno == ffmpeg::error::EAGAIN
    )
}

// Returning EOF separately lets the caller drain the codec. Cancellation and
// deadlines also cover backends that repeatedly return EAGAIN without invoking
// FFmpeg's interrupt callback.
fn read_packet_with_retry(
    live: bool,
    interrupt: Option<&CaptureInterrupt>,
    mut read: impl FnMut() -> Result<(), ffmpeg::Error>,
) -> Result<bool, FfmpegDecodeError> {
    loop {
        if let Some(interrupt) = interrupt {
            interrupt.check()?;
        }
        let result = read();
        if let Some(interrupt) = interrupt {
            interrupt.check()?;
        }
        match result {
            Ok(()) => return Ok(true),
            Err(ffmpeg::Error::Eof) => return Ok(false),
            Err(error) if is_temporary_live_read_error(live, &error) => {
                std::thread::sleep(LIVE_READ_RETRY_DELAY);
            }
            Err(error) => return Err(FfmpegDecodeError::Read(error)),
        }
    }
}

#[derive(Debug, Error)]
pub enum FfmpegDecodeError {
    #[error("video input configuration: {0}")]
    CaptureConfiguration(String),
    #[error("camera operation canceled")]
    CaptureCanceled,
    #[error("camera stopped responding before the read/open deadline")]
    CaptureTimeout,
    #[error("initialize FFmpeg: {0}")]
    Initialize(ffmpeg::Error),
    #[error("open media file {path}: {source}")]
    Open {
        path: PathBuf,
        source: ffmpeg::Error,
    },
    #[error("open camera {device}: {source}")]
    OpenCamera {
        device: String,
        source: ffmpeg::Error,
    },
    #[error("camera backend {0} is unavailable")]
    CameraBackendUnavailable(String),
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
    #[error("seek media input: {0}")]
    Seek(ffmpeg::Error),
    #[error("seek timestamp is outside the supported range")]
    SeekTimestampOverflow,
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
    #[error("media produced no frame for a thumbnail")]
    MissingThumbnailFrame,
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
    // Field order matters: close the input before freeing its callback state.
    input: ffmpeg::format::context::Input,
    capture_interrupt: Option<Box<CaptureInterrupt>>,
    stream_index: usize,
    time_base: ffmpeg::Rational,
    average_duration: Option<MediaTime>,
    decoder: ffmpeg::decoder::Video,
    scaler: ffmpeg::software::scaling::Context,
    decoded: ffmpeg::frame::Video,
    converted: ffmpeg::frame::Video,
    draining: bool,
    sequence: u64,
    allow_missing_timestamp: bool,
    fallback_fps: u32,
    fallback_fps_denominator: u32,
    live: bool,
    frame_pool: FrameBufferPool,
}

impl FfmpegVideoDecoder {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FfmpegDecodeError> {
        Self::open_with_pool(path, FrameBufferPool::new(8))
    }

    pub fn open_with_pool(
        path: impl AsRef<Path>,
        frame_pool: FrameBufferPool,
    ) -> Result<Self, FfmpegDecodeError> {
        Self::open_internal(path.as_ref(), false, frame_pool)
    }

    pub fn open_at(
        path: impl AsRef<Path>,
        seek_to: Option<MediaTime>,
    ) -> Result<Self, FfmpegDecodeError> {
        Self::open_at_with_pool(path, seek_to, FrameBufferPool::new(8))
    }

    pub fn open_at_with_pool(
        path: impl AsRef<Path>,
        seek_to: Option<MediaTime>,
        frame_pool: FrameBufferPool,
    ) -> Result<Self, FfmpegDecodeError> {
        let mut decoder = Self::open_with_pool(path, frame_pool)?;
        if let Some(seek_to) = seek_to {
            decoder.seek(seek_to)?;
        }
        Ok(decoder)
    }

    /// Thumbnail generation may use FFmpeg's HAP pixel decoder because it is
    /// off the playback path and produces only one small cached image.
    pub fn open_for_thumbnail(path: impl AsRef<Path>) -> Result<Self, FfmpegDecodeError> {
        Self::open_internal(path.as_ref(), true, FrameBufferPool::new(2))
    }

    fn open_internal(
        path: &Path,
        allow_hap: bool,
        frame_pool: FrameBufferPool,
    ) -> Result<Self, FfmpegDecodeError> {
        ffmpeg::init().map_err(FfmpegDecodeError::Initialize)?;
        let input = ffmpeg::format::input(path).map_err(|source| FfmpegDecodeError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_input(input, allow_hap, allow_hap, 30, false, frame_pool)
    }

    pub fn open_camera(config: &CameraConfig) -> Result<Self, FfmpegDecodeError> {
        Self::open_camera_with_pool(config, FrameBufferPool::new(8))
    }

    pub fn open_camera_with_pool(
        config: &CameraConfig,
        frame_pool: FrameBufferPool,
    ) -> Result<Self, FfmpegDecodeError> {
        Self::open_camera_cancellable(config, frame_pool, CaptureCancellation::default())
    }

    pub(crate) fn open_camera_cancellable(
        config: &CameraConfig,
        frame_pool: FrameBufferPool,
        cancellation: CaptureCancellation,
    ) -> Result<Self, FfmpegDecodeError> {
        let mut interrupt = Box::new(CaptureInterrupt::new(cancellation));
        interrupt.check()?;
        ffmpeg::init().map_err(FfmpegDecodeError::Initialize)?;
        ffmpeg::device::register_all();
        let backend = std::ffi::CString::new(config.device.backend.as_str()).map_err(|_| {
            FfmpegDecodeError::CameraBackendUnavailable(config.device.backend.clone())
        })?;
        // SAFETY: backend is a live NUL-terminated string for this call.
        let format_ptr = unsafe { ffmpeg::ffi::av_find_input_format(backend.as_ptr()) };
        if format_ptr.is_null() {
            return Err(FfmpegDecodeError::CameraBackendUnavailable(
                config.device.backend.clone(),
            ));
        }
        let mut options = ffmpeg::Dictionary::new();
        options.set("fflags", "nobuffer");
        options.set("flags", "low_delay");
        options.set("rtbufsize", "16M");
        if let Some(pixel_format) = config.requested_pixel_format() {
            options.set("pixel_format", pixel_format);
        }
        if let Some([width, height]) = config.requested_extent {
            options.set("video_size", &format!("{width}x{height}"));
        }
        if let Some(rate) = config.frame_rate_option() {
            options.set("framerate", &rate);
        }
        let resolved_name = config
            .resolved_input_name()
            .map_err(FfmpegDecodeError::CaptureConfiguration)?;
        let input_name = std::ffi::CString::new(resolved_name).map_err(|_| {
            FfmpegDecodeError::CameraBackendUnavailable("invalid camera ID".to_owned())
        })?;
        // The safe wrapper cannot combine a device format, dictionary and owned
        // interrupt callback. Keep ownership explicit on every failure path.
        // SAFETY: All pointers refer to live local values or FFmpeg allocations.
        // On success Input owns the context; its callback's box moves with it.
        let input = unsafe {
            let mut context = ffmpeg::ffi::avformat_alloc_context();
            if context.is_null() {
                return Err(FfmpegDecodeError::OpenCamera {
                    device: config.device.label.clone(),
                    source: ffmpeg::Error::Other {
                        errno: ffmpeg::error::ENOMEM,
                    },
                });
            }
            (*context).interrupt_callback = interrupt.callback();
            let mut dictionary = options.disown();
            let opened = ffmpeg::ffi::avformat_open_input(
                &mut context,
                input_name.as_ptr(),
                format_ptr,
                &mut dictionary,
            );
            drop(ffmpeg::Dictionary::own(dictionary));
            if opened < 0 {
                // avformat_open_input frees and nulls the context on failure.
                interrupt.check()?;
                return Err(FfmpegDecodeError::OpenCamera {
                    device: config.device.label.clone(),
                    source: ffmpeg::Error::from(opened),
                });
            }
            let input = ffmpeg::format::context::Input::wrap(context);
            let found = ffmpeg::ffi::avformat_find_stream_info(context, std::ptr::null_mut());
            interrupt.check()?;
            if found < 0 {
                return Err(FfmpegDecodeError::OpenCamera {
                    device: config.device.label.clone(),
                    source: ffmpeg::Error::from(found),
                });
            }
            input
        };
        let mut decoder = Self::from_input(
            input,
            true,
            true,
            config.requested_fps.unwrap_or(30),
            true,
            frame_pool,
        )?;
        decoder.capture_interrupt = Some(interrupt);
        decoder.fallback_fps_denominator = config.fps_denominator.max(1);
        Ok(decoder)
    }

    fn from_input(
        input: ffmpeg::format::context::Input,
        allow_hap: bool,
        allow_missing_timestamp: bool,
        fallback_fps: u32,
        live: bool,
        frame_pool: FrameBufferPool,
    ) -> Result<Self, FfmpegDecodeError> {
        let (stream_index, time_base, average_duration, decoder, scaler) = {
            let stream = input
                .streams()
                .best(ffmpeg::media::Type::Video)
                .ok_or(FfmpegDecodeError::NoVideoStream)?;
            let stream_index = stream.index();
            let time_base = stream.time_base();
            let parameters = stream.parameters();
            if parameters.id() == ffmpeg::codec::Id::HAP && !allow_hap {
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
            capture_interrupt: None,
            stream_index,
            time_base,
            average_duration,
            decoder,
            scaler,
            decoded: ffmpeg::frame::Video::empty(),
            converted: ffmpeg::frame::Video::empty(),
            draining: false,
            sequence: 0,
            allow_missing_timestamp,
            fallback_fps,
            fallback_fps_denominator: 1,
            live,
            frame_pool,
        })
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    fn seek(&mut self, target: MediaTime) -> Result<(), FfmpegDecodeError> {
        let timestamp = i128::from(target.ticks())
            .checked_mul(i128::from(ffmpeg::ffi::AV_TIME_BASE))
            .and_then(|value| value.checked_div(i128::from(target.timescale())))
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(FfmpegDecodeError::SeekTimestampOverflow)?;
        self.input
            .seek(timestamp, ..timestamp.saturating_add(1))
            .map_err(FfmpegDecodeError::Seek)?;
        self.decoder.flush();
        self.draining = false;
        Ok(())
    }

    pub fn next_frame(&mut self) -> Result<Option<DecodedRgbaFrame>, FfmpegDecodeError> {
        if let Some(interrupt) = &mut self.capture_interrupt {
            interrupt.begin_read();
        }
        loop {
            self.check_capture_interrupt()?;
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
                let has_packet =
                    read_packet_with_retry(self.live, self.capture_interrupt.as_deref(), || {
                        packet.read(&mut self.input)
                    })?;
                if !has_packet {
                    self.decoder.send_eof().map_err(FfmpegDecodeError::Submit)?;
                    self.draining = true;
                    break;
                }
                if packet.stream() == self.stream_index {
                    break;
                }
            }
            if !self.draining {
                self.decoder
                    .send_packet(&packet)
                    .map_err(FfmpegDecodeError::Submit)?;
            }
        }
    }

    fn check_capture_interrupt(&self) -> Result<(), FfmpegDecodeError> {
        self.capture_interrupt
            .as_ref()
            .map_or(Ok(()), |interrupt| interrupt.check())
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
        let mut data = self.frame_pool.acquire(total_bytes);
        for row in 0..height as usize {
            data[row * row_bytes..(row + 1) * row_bytes]
                .copy_from_slice(&source[row * stride..row * stride + row_bytes]);
        }

        let timestamp = self.decoded.timestamp().or_else(|| self.decoded.pts());
        let pts = if let Some(timestamp) = timestamp {
            MediaTime::from_time_base(
                timestamp,
                self.time_base.numerator(),
                self.time_base.denominator(),
            )?
        } else if self.allow_missing_timestamp {
            let ticks = self
                .sequence
                .checked_mul(u64::from(self.fallback_fps_denominator))
                .and_then(|ticks| i64::try_from(ticks).ok())
                .ok_or(MediaTimeError::Overflow)?;
            MediaTime::new(ticks, i64::from(self.fallback_fps.max(1)))?
        } else {
            return Err(FfmpegDecodeError::MissingTimestamp);
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_live_eagain_packet_reads_are_temporary() {
        let eagain = ffmpeg::Error::Other {
            errno: ffmpeg::error::EAGAIN,
        };
        let eio = ffmpeg::Error::Other {
            errno: ffmpeg::error::EIO,
        };

        assert!(is_temporary_live_read_error(true, &eagain));
        assert!(!is_temporary_live_read_error(false, &eagain));
        assert!(!is_temporary_live_read_error(true, &eio));
    }

    #[test]
    fn stalled_live_read_can_be_canceled_without_waiting_for_another_frame() {
        let owner = CaptureCancellation::default();
        let cancellation = owner.next();
        let (entered, started) = std::sync::mpsc::sync_channel(1);
        let (finished, completion) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let mut interrupt = CaptureInterrupt::new(cancellation);
            interrupt.begin_read();
            let result = read_packet_with_retry(true, Some(&interrupt), || {
                let _ = entered.try_send(());
                Err(ffmpeg::Error::Other {
                    errno: ffmpeg::error::EAGAIN,
                })
            });
            finished.send(result).unwrap();
        });
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        let _ = owner.next();
        let result = completion.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(result, Err(FfmpegDecodeError::CaptureCanceled)));
        worker.join().unwrap();
    }

    #[test]
    fn live_retry_keeps_temporary_underruns_distinct_from_end_of_stream() {
        let mut reads = 0;
        assert!(
            read_packet_with_retry(true, None, || {
                reads += 1;
                if reads == 1 {
                    Err(ffmpeg::Error::Other {
                        errno: ffmpeg::error::EAGAIN,
                    })
                } else {
                    Ok(())
                }
            })
            .unwrap()
        );
        assert!(!read_packet_with_retry(true, None, || Err(ffmpeg::Error::Eof)).unwrap());
        assert!(matches!(
            read_packet_with_retry(false, None, || {
                Err(ffmpeg::Error::Other {
                    errno: ffmpeg::error::EAGAIN,
                })
            }),
            Err(FfmpegDecodeError::Read(_))
        ));
    }
}
