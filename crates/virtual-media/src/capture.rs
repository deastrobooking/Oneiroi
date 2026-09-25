//! Camera and capture-card discovery and input configuration.

use std::ffi::CStr;
#[cfg(not(target_os = "macos"))]
use std::ffi::CString;
use std::path::PathBuf;
#[cfg(not(target_os = "macos"))]
use std::ptr;

use ffmpeg_next as ffmpeg;
use thiserror::Error;
use virtual_core::MediaTime;

use crate::{AlphaMode, DecodePath, FrameRate, MediaHealth, MovieMetadata};

pub const CAMERA_SCHEME: &str = "camera://";
const NATIVE_DEVICE_PREFIX: &str = "avf-id/";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CapturePixelFormat {
    #[default]
    Auto,
    Nv12,
    Uyvy422,
    Yuyv422,
    Bgra,
}

impl CapturePixelFormat {
    pub const ALL: [Self; 5] = [
        Self::Auto,
        Self::Nv12,
        Self::Uyvy422,
        Self::Yuyv422,
        Self::Bgra,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Nv12 => "nv12",
            Self::Uyvy422 => "uyvy422",
            Self::Yuyv422 => "yuyv422",
            Self::Bgra => "bgra",
        }
    }
    pub fn from_id(id: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|format| format.id() == id)
            .unwrap_or_default()
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Nv12 => "NV12",
            Self::Uyvy422 => "UYVY 4:2:2",
            Self::Yuyv422 => "YUYV 4:2:2",
            Self::Bgra => "BGRA",
        }
    }
}

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
    pub fps_denominator: u32,
    pub pixel_format: CapturePixelFormat,
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

    pub fn requested_pixel_format(&self) -> Option<&'static str> {
        (self.pixel_format != CapturePixelFormat::Auto).then_some(self.pixel_format.id())
    }

    pub fn frame_rate_option(&self) -> Option<String> {
        self.requested_fps
            .map(|fps| format!("{fps}/{}", self.fps_denominator.max(1)))
    }

    pub(crate) fn resolved_input_name(&self) -> Result<String, String> {
        if self.device.backend != "avfoundation"
            || !self.device.id.starts_with(NATIVE_DEVICE_PREFIX)
        {
            return Ok(self.input_name());
        }
        let devices = discover_cameras().map_err(|error| error.to_string())?;
        resolve_native_device(&self.device.id, &devices)
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
                denominator: self.fps_denominator.max(1) as i32,
            }),
            duration: None,
            frame_count: None,
            alpha: AlphaMode::Absent,
            decode_path: DecodePath::FfmpegVideo,
            health: MediaHealth::Usable,
            health_reason: "Live video input; latency depends on capture hardware.".to_owned(),
            keyframes: crate::KeyframeIndex::default(),
        }
    }
}

#[derive(Debug, Error)]
pub enum CameraDiscoveryError {
    #[error("macOS video-input discovery failed")]
    NativeDiscovery,
    #[error("initialize FFmpeg: {0}")]
    Initialize(ffmpeg::Error),
    #[error("AVFoundation input support is unavailable")]
    BackendUnavailable,
    #[error("list AVFoundation cameras: {0}")]
    List(ffmpeg::Error),
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
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

// FFmpeg's AVFoundation backend accepts names/indices, not native unique IDs.
// Persist the unique ID and resolve its current name on the decoder worker.
// Fail explicitly if FFmpeg's prefix matching could select another device.
fn resolve_native_device(id: &str, devices: &[CameraDevice]) -> Result<String, String> {
    let device = devices
        .iter()
        .find(|device| device.id == id)
        .ok_or_else(|| {
            "Saved video input is disconnected. Reconnect it or select another input.".to_owned()
        })?;
    if device.label.is_empty()
        || device.label.contains(':')
        || device
            .label
            .trim_start()
            .starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+')
        || device.label.starts_with("default")
        || device.label.starts_with("none")
        || devices
            .iter()
            .filter(|other| other.label.starts_with(&device.label))
            .count()
            != 1
    {
        return Err("Video input name is ambiguous. Use its current AVFoundation device index in the manual input field.".to_owned());
    }
    Ok(format!("{}:none", device.label))
}

#[cfg(target_os = "macos")]
pub fn discover_cameras() -> Result<Vec<CameraDevice>, CameraDiscoveryError> {
    unsafe extern "C" {
        fn virtual_video_inputs(
            context: *mut std::ffi::c_void,
            visit: unsafe extern "C" fn(
                *mut std::ffi::c_void,
                *const std::ffi::c_char,
                *const std::ffi::c_char,
            ),
        ) -> i32;
    }
    unsafe extern "C" fn visit(
        context: *mut std::ffi::c_void,
        id: *const std::ffi::c_char,
        name: *const std::ffi::c_char,
    ) {
        // SAFETY: The native enumerator calls synchronously with temporary UTF-8
        // strings. Copy them before returning; context is our exclusive Vec.
        let devices = unsafe { &mut *context.cast::<Vec<CameraDevice>>() };
        if let (Some(id), Some(label)) = (unsafe { c_string(id) }, unsafe { c_string(name) }) {
            devices.push(CameraDevice {
                id: format!("{NATIVE_DEVICE_PREFIX}{id}"),
                label,
                backend: "avfoundation".to_owned(),
            });
        }
    }
    let mut devices = Vec::new();
    // SAFETY: The callback and Vec remain alive throughout this synchronous call.
    let status =
        unsafe { virtual_video_inputs((&mut devices as *mut Vec<CameraDevice>).cast(), visit) };
    if status != 0 {
        return Err(CameraDiscoveryError::NativeDiscovery);
    }
    Ok(devices)
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
            fps_denominator: 1,
            pixel_format: CapturePixelFormat::Nv12,
        };
        assert_eq!(config.input_name(), "0:none");
        assert_eq!(config.requested_pixel_format(), Some("nv12"));
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
            fps_denominator: 1,
            pixel_format: CapturePixelFormat::Auto,
        };
        assert_eq!(config.input_name(), config.device.id);
        assert_eq!(config.requested_pixel_format(), None);
    }

    #[test]
    fn synthesizes_monotonic_camera_time_when_backend_has_no_pts() {
        assert_eq!(camera_pts(60, 30), MediaTime::new(2, 1).unwrap());
    }

    #[test]
    fn capture_cards_use_exact_fractional_rates_and_explicit_pixel_formats() {
        let config = CameraConfig {
            device: CameraDevice {
                id: "USB Capture HDMI".to_owned(),
                label: "USB Capture HDMI".to_owned(),
                backend: "avfoundation".to_owned(),
            },
            requested_extent: Some([1920, 1080]),
            requested_fps: Some(60000),
            fps_denominator: 1001,
            pixel_format: CapturePixelFormat::Uyvy422,
        };
        assert_eq!(config.frame_rate_option().as_deref(), Some("60000/1001"));
        assert_eq!(config.requested_pixel_format(), Some("uyvy422"));
        assert_eq!(config.input_name(), "USB Capture HDMI:none");
        assert_eq!(config.metadata().frame_rate.unwrap().denominator, 1001);
    }

    #[test]
    fn native_ids_survive_order_changes_and_reject_ambiguous_or_missing_devices() {
        let device = |id: &str, label: &str| CameraDevice {
            id: format!("avf-id/{id}"),
            label: label.to_owned(),
            backend: "avfoundation".to_owned(),
        };
        let mut devices = vec![device("camera", "Camera"), device("hdmi", "USB HDMI")];
        assert_eq!(
            resolve_native_device("avf-id/hdmi", &devices).unwrap(),
            "USB HDMI:none"
        );
        devices.reverse();
        assert_eq!(
            resolve_native_device("avf-id/hdmi", &devices).unwrap(),
            "USB HDMI:none"
        );
        assert!(resolve_native_device("avf-id/missing", &devices).is_err());
        devices.push(device("other", "USB HDMI Pro"));
        assert!(resolve_native_device("avf-id/hdmi", &devices).is_err());
    }
}
