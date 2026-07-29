//! General movie probing and live-performance suitability classification.

use std::path::{Path, PathBuf};

use ffmpeg_next as ffmpeg;
use oneiroi_core::{MediaTime, MediaTimeError};
use thiserror::Error;

use crate::FrameRate;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodePath {
    DirectHap,
    FfmpegVideo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaHealth {
    StageReady,
    Usable,
    Caution,
    Problem,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlphaMode {
    Present,
    Absent,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MovieMetadata {
    pub path: PathBuf,
    pub display_name: String,
    pub container: String,
    pub stream_index: usize,
    pub codec: String,
    pub codec_tag: [u8; 4],
    pub visible_extent: [u32; 2],
    pub frame_rate: Option<FrameRate>,
    pub duration: Option<MediaTime>,
    pub frame_count: Option<u64>,
    pub alpha: AlphaMode,
    pub decode_path: DecodePath,
    pub health: MediaHealth,
    pub health_reason: String,
}

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("initialize FFmpeg: {0}")]
    Initialize(ffmpeg::Error),
    #[error("open media file {path}: {source}")]
    Open {
        path: PathBuf,
        source: ffmpeg::Error,
    },
    #[error("file contains no video stream")]
    NoVideoStream,
    #[error("video stream has invalid dimensions {width}x{height}")]
    InvalidDimensions { width: i32, height: i32 },
    #[error("invalid media timestamp: {0}")]
    Timestamp(#[from] MediaTimeError),
}

pub fn probe_movie(path: impl AsRef<Path>) -> Result<MovieMetadata, ProbeError> {
    let path = path.as_ref();
    ffmpeg::init().map_err(ProbeError::Initialize)?;
    let input = ffmpeg::format::input(path).map_err(|source| ProbeError::Open {
        path: path.to_path_buf(),
        source,
    })?;

    let stream = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or(ProbeError::NoVideoStream)?;
    let stream_index = stream.index();
    let parameters = stream.parameters();
    let codec_id = parameters.id();
    // SAFETY: parameters owns a live AVCodecParameters for this scope.
    let (width, height, codec_tag) = unsafe {
        let parameters = &*parameters.as_ptr();
        (parameters.width, parameters.height, parameters.codec_tag)
    };
    if width <= 0 || height <= 0 {
        return Err(ProbeError::InvalidDimensions { width, height });
    }

    let time_base = stream.time_base();
    let duration = if stream.duration() != ffmpeg::ffi::AV_NOPTS_VALUE
        && stream.duration() >= 0
        && time_base.numerator() > 0
        && time_base.denominator() > 0
    {
        Some(MediaTime::from_time_base(
            stream.duration(),
            time_base.numerator(),
            time_base.denominator(),
        )?)
    } else if input.duration() != ffmpeg::ffi::AV_NOPTS_VALUE && input.duration() >= 0 {
        Some(MediaTime::new(
            input.duration(),
            i64::from(ffmpeg::ffi::AV_TIME_BASE),
        )?)
    } else {
        None
    };
    let frame_rate = valid_rate(stream.avg_frame_rate()).or_else(|| valid_rate(stream.rate()));
    let codec_tag = codec_tag.to_le_bytes();
    let has_decoder = ffmpeg::codec::decoder::find(codec_id).is_some();
    let (decode_path, health, health_reason) =
        classify(codec_id, has_decoder, width as u32, height as u32);

    Ok(MovieMetadata {
        path: path.to_path_buf(),
        display_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled movie")
            .to_owned(),
        container: input.format().name().to_owned(),
        stream_index,
        codec: codec_id.name().to_owned(),
        codec_tag,
        visible_extent: [width as u32, height as u32],
        frame_rate,
        duration,
        frame_count: u64::try_from(stream.frames())
            .ok()
            .filter(|count| *count > 0),
        alpha: alpha_mode(codec_id, codec_tag),
        decode_path,
        health,
        health_reason,
    })
}

fn valid_rate(rate: ffmpeg::Rational) -> Option<FrameRate> {
    (rate.numerator() > 0 && rate.denominator() > 0).then_some(FrameRate {
        numerator: rate.numerator(),
        denominator: rate.denominator(),
    })
}

fn classify(
    codec: ffmpeg::codec::Id,
    has_decoder: bool,
    width: u32,
    height: u32,
) -> (DecodePath, MediaHealth, String) {
    if !has_decoder && codec != ffmpeg::codec::Id::HAP {
        return (
            DecodePath::FfmpegVideo,
            MediaHealth::Problem,
            "No FFmpeg decoder is available for this codec.".to_owned(),
        );
    }
    if width > 8_192 || height > 8_192 {
        return (
            if codec == ffmpeg::codec::Id::HAP {
                DecodePath::DirectHap
            } else {
                DecodePath::FfmpegVideo
            },
            MediaHealth::Problem,
            format!("{width}×{height} exceeds the current 8K import limit."),
        );
    }
    match codec {
        ffmpeg::codec::Id::HAP => (
            DecodePath::DirectHap,
            MediaHealth::StageReady,
            "GPU-native HAP blocks use the direct upload path.".to_owned(),
        ),
        ffmpeg::codec::Id::PRORES | ffmpeg::codec::Id::DNXHD => (
            DecodePath::FfmpegVideo,
            MediaHealth::Usable,
            "Intra-frame production codec; suitable for responsive playback.".to_owned(),
        ),
        ffmpeg::codec::Id::PNG | ffmpeg::codec::Id::MJPEG => (
            DecodePath::FfmpegVideo,
            MediaHealth::Usable,
            "Still image is decoded once and held without continuous codec load.".to_owned(),
        ),
        ffmpeg::codec::Id::H264 | ffmpeg::codec::Id::HEVC => (
            DecodePath::FfmpegVideo,
            MediaHealth::Caution,
            "Long-GOP footage may trigger and seek slowly; optimize to HAP for stage use."
                .to_owned(),
        ),
        _ => (
            DecodePath::FfmpegVideo,
            MediaHealth::Caution,
            format!(
                "{} will use FFmpeg fallback decoding; verify performance before a show.",
                codec.name()
            ),
        ),
    }
}

fn alpha_mode(codec: ffmpeg::codec::Id, tag: [u8; 4]) -> AlphaMode {
    if codec == ffmpeg::codec::Id::HAP {
        return match &tag {
            b"Hap5" | b"HapA" | b"HapM" | b"Hap7" => AlphaMode::Present,
            b"Hap1" | b"HapY" | b"HapH" => AlphaMode::Absent,
            _ => AlphaMode::Unknown,
        };
    }
    if codec == ffmpeg::codec::Id::PRORES {
        return match &tag {
            b"ap4h" | b"ap4x" => AlphaMode::Present,
            _ => AlphaMode::Unknown,
        };
    }
    AlphaMode::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_hap_as_direct_and_stage_ready() {
        let (path, health, _) = classify(ffmpeg::codec::Id::HAP, true, 1920, 1080);
        assert_eq!(path, DecodePath::DirectHap);
        assert_eq!(health, MediaHealth::StageReady);
    }

    #[test]
    fn classifies_long_gop_codecs_as_caution() {
        for codec in [ffmpeg::codec::Id::H264, ffmpeg::codec::Id::HEVC] {
            let (path, health, reason) = classify(codec, true, 1920, 1080);
            assert_eq!(path, DecodePath::FfmpegVideo);
            assert_eq!(health, MediaHealth::Caution);
            assert!(reason.contains("Long-GOP"));
        }
    }

    #[test]
    fn classifies_supported_stills_as_single_decode_media() {
        for codec in [ffmpeg::codec::Id::PNG, ffmpeg::codec::Id::MJPEG] {
            let (path, health, reason) = classify(codec, true, 1920, 1080);
            assert_eq!(path, DecodePath::FfmpegVideo);
            assert_eq!(health, MediaHealth::Usable);
            assert!(reason.contains("decoded once"));
        }
    }

    #[test]
    fn rejects_extreme_dimensions_before_stage_use() {
        let (_, health, _) = classify(ffmpeg::codec::Id::PRORES, true, 16_384, 1080);
        assert_eq!(health, MediaHealth::Problem);
    }

    #[test]
    fn recognizes_hap_alpha_tags() {
        assert_eq!(
            alpha_mode(ffmpeg::codec::Id::HAP, *b"HapM"),
            AlphaMode::Present
        );
        assert_eq!(
            alpha_mode(ffmpeg::codec::Id::HAP, *b"Hap1"),
            AlphaMode::Absent
        );
    }
}
