//! Live camera discovery and capture descriptors.

use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::ptr;

use ffmpeg_next as ffmpeg;
use oneiroi_core::MediaTime;
use thiserror::Error;

use crate::{AlphaMode, DecodePath, FrameRate, MediaHealth, MovieMetadata};

pub const CAMERA_SCHEME: &str = "camera://";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CameraDevice {
    /// Backend-specific stable identifier accepted by AVFoundation.
    pub id: String,
    pub label: String,
    pub backend: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CameraConfig {
    pub device: CameraDevice,
    pub requested_extent: Option<[u32; 2]>,
    pub requested_fps: Option<u32>,
}

impl CameraConfig {
    pub fn input_name(&self) -> String {
        if self.device.backend == "avfoundation" {
            format!("{}:none", self.device.id)
        } else {
            self.device.id.clone()
        }
    }

    pub fn virtual_path(&self) -> PathBuf {
        PathBuf::from(format!("{CAMERA_SCHEME}{}", self.device.id))
    }

    pub fn metadata(&self) -> MovieMetadata {
        MovieMetadata {
            path: self.virtual_path(),
            display_name: self.device.label.clone(),
            container: self.device.backend.clone(),
            stream_index: 0,
            codec: "live".to_owned(),
            codec_tag: [0; 4],
            visible_extent: self.requested_extent.unwrap_or([0, 0]),
            frame_rate: self.requested_fps.map(|fps| FrameRate {
                numerator: fps as i32,
                denominator: 1,
            }),
            duration: None,
            frame_count: None,
            alpha: AlphaMode::Absent,
            decode_path: DecodePath::FfmpegVideo,
            health: MediaHealth::Usable,
            health_reason: "Live camera feed; latency depends on capture hardware.".to_owned(),
        }
    }
}

#[derive(Debug, Error)]
pub enum CameraDiscoveryError {
    #[error("initialize FFmpeg: {0}")]
    Initialize(ffmpeg::Error),
    #[error("AVFoundation input support is unavailable")]
    BackendUnavailable,
    #[error("list AVFoundation cameras: {0}")]
    List(ffmpeg::Error),
}

pub fn discover_cameras() -> Result<Vec<CameraDevice>, CameraDiscoveryError> {
    ffmpeg::init().map_err(CameraDiscoveryError::Initialize)?;
    ffmpeg::device::register_all();
    let backend = CString::new("avfoundation").expect("static backend name");
    // SAFETY: the backend string is NUL terminated and lives for this call.
    let format = unsafe { ffmpeg::ffi::av_find_input_format(backend.as_ptr()) };
    if format.is_null() {
        return Err(CameraDiscoveryError::BackendUnavailable);
    }
    let mut list = ptr::null_mut();
    // SAFETY: format is a live libavformat input descriptor. FFmpeg owns the
    // returned list until avdevice_free_list_devices below.
    let count = unsafe {
        ffmpeg::ffi::avdevice_list_input_sources(format, ptr::null(), ptr::null_mut(), &mut list)
    };
    if count < 0 {
        return Err(CameraDiscoveryError::List(ffmpeg::Error::from(count)));
    }
    let mut cameras = Vec::with_capacity(count as usize);
    if !list.is_null() {
        // SAFETY: FFmpeg reports nb_devices entries in the devices array.
        unsafe {
            for index in 0..(*list).nb_devices {
                let info = *(*list).devices.add(index as usize);
                if info.is_null() {
                    continue;
                }
                if !provides_video(&*info) {
                    continue;
                }
                let id = c_string((*info).device_name);
                let label = c_string((*info).device_description);
                if let Some(id) = id {
                    cameras.push(CameraDevice {
                        label: label.unwrap_or_else(|| id.clone()),
                        id,
                        backend: "avfoundation".to_owned(),
                    });
                }
            }
            ffmpeg::ffi::avdevice_free_list_devices(&mut list);
        }
    }
    Ok(cameras)
}

unsafe fn provides_video(info: &ffmpeg::ffi::AVDeviceInfo) -> bool {
    if info.media_types.is_null() || info.nb_media_types <= 0 {
        return false;
    }
    // SAFETY: FFmpeg reports nb_media_types entries in this device's array.
    unsafe {
        std::slice::from_raw_parts(info.media_types, info.nb_media_types as usize)
            .contains(&ffmpeg::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO)
    }
}

unsafe fn c_string(pointer: *const std::ffi::c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: the caller passes FFmpeg-owned NUL-terminated strings.
    Some(
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned(),
    )
}

pub fn camera_pts(sequence: u64, fps: u32) -> MediaTime {
    MediaTime::new(
        i64::try_from(sequence).unwrap_or(i64::MAX),
        i64::from(fps.max(1)),
    )
    .expect("positive camera timescale")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_avfoundation_video_only_input() {
        let config = CameraConfig {
            device: CameraDevice {
                id: "0".to_owned(),
                label: "Camera".to_owned(),
                backend: "avfoundation".to_owned(),
            },
            requested_extent: Some([1920, 1080]),
            requested_fps: Some(30),
        };
        assert_eq!(config.input_name(), "0:none");
        assert_eq!(config.virtual_path(), PathBuf::from("camera://0"));
        assert!(config.metadata().duration.is_none());
    }

    #[test]
    fn preserves_non_avfoundation_input_names() {
        let config = CameraConfig {
            device: CameraDevice {
                id: "testsrc=size=16x16:rate=30".to_owned(),
                label: "Test pattern".to_owned(),
                backend: "lavfi".to_owned(),
            },
            requested_extent: None,
            requested_fps: None,
        };
        assert_eq!(config.input_name(), config.device.id);
    }

    #[test]
    fn synthesizes_monotonic_camera_time_when_backend_has_no_pts() {
        assert_eq!(camera_pts(60, 30), MediaTime::new(2, 1).unwrap());
    }
}
