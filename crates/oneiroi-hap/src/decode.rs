use std::ffi::{c_uint, c_ulong, c_void};
use std::ptr;

use smallvec::SmallVec;
use thiserror::Error;

use crate::CompressedPlaneFormat;
use oneiroi_hap_sys as sys;

unsafe extern "C" fn serial_decode_callback(
    function: sys::HapDecodeWorkFunction,
    context: *mut c_void,
    count: c_uint,
    _info: *mut c_void,
) {
    if let Some(function) = function {
        for index in 0..count {
            // SAFETY: the reference decoder owns context and asks this
            // callback to invoke exactly these indexed work items.
            unsafe { function(context, index) };
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DecodeLimits {
    pub max_dimension: u32,
    pub max_frame_bytes: usize,
    pub max_chunks_per_plane: u32,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_dimension: 16_384,
            max_frame_bytes: 512 * 1024 * 1024,
            max_chunks_per_plane: 16_384,
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum HapError {
    #[error("visible dimensions must be non-zero")]
    ZeroDimension,
    #[error("dimensions exceed the configured limit of {limit}: {width}x{height}")]
    DimensionLimit { width: u32, height: u32, limit: u32 },
    #[error("dimension arithmetic overflow")]
    DimensionOverflow,
    #[error("frame-size arithmetic overflow")]
    FrameSizeOverflow,
    #[error("decoded frame requires {actual} bytes, above the {limit}-byte limit")]
    FrameByteLimit { actual: usize, limit: usize },
    #[error("HAP frame contains {0} textures; only one or two are valid")]
    InvalidTextureCount(u32),
    #[error("HAP plane {plane} contains invalid chunk count {count}")]
    InvalidChunkCount { plane: u32, count: i32 },
    #[error("HAP plane {plane} has {count} chunks, above the configured limit of {limit}")]
    ChunkLimit { plane: u32, count: u32, limit: u32 },
    #[error("unsupported HAP texture format 0x{0:08x}")]
    UnsupportedTextureFormat(u32),
    #[error("unsupported two-plane HAP format combination")]
    UnsupportedPlaneCombination,
    #[error("HAP reference decoder rejected the {operation}: {result:?}")]
    Reference {
        operation: &'static str,
        result: ReferenceError,
    },
    #[error("HAP plane {plane} decoded to {actual} bytes; expected {expected}")]
    DecodedSizeMismatch {
        plane: u32,
        actual: usize,
        expected: usize,
    },
    #[error("platform cannot represent this packet or frame length")]
    PlatformLengthOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceError {
    BadArguments,
    BufferTooSmall,
    BadFrame,
    Internal,
    Unknown(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedPlane {
    pub format: CompressedPlaneFormat,
    pub coded_extent: [u32; 2],
    pub visible_extent: [u32; 2],
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedFrame {
    pub planes: SmallVec<[DecodedPlane; 2]>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Decoder {
    limits: DecodeLimits,
}

impl Decoder {
    pub fn new(limits: DecodeLimits) -> Self {
        Self { limits }
    }

    pub fn decode(
        &self,
        packet: &[u8],
        visible_width: u32,
        visible_height: u32,
    ) -> Result<DecodedFrame, HapError> {
        self.validate_dimensions(visible_width, visible_height)?;
        // The reference implementation accepts unsigned long publicly but
        // narrows packet lengths to uint32_t internally.
        u32::try_from(packet.len()).map_err(|_| HapError::PlatformLengthOverflow)?;
        let packet_len =
            c_ulong::try_from(packet.len()).map_err(|_| HapError::PlatformLengthOverflow)?;

        let texture_count = self.texture_count(packet, packet_len)?;
        let mut formats = SmallVec::<[CompressedPlaneFormat; 2]>::new();
        let mut total_bytes = 0usize;

        for plane in 0..texture_count {
            let format = self.texture_format(packet, packet_len, plane)?;
            self.validate_chunk_count(packet, packet_len, plane)?;
            let plane_bytes = format.expected_bytes(visible_width, visible_height)?;
            total_bytes = total_bytes
                .checked_add(plane_bytes)
                .ok_or(HapError::FrameSizeOverflow)?;
            if total_bytes > self.limits.max_frame_bytes {
                return Err(HapError::FrameByteLimit {
                    actual: total_bytes,
                    limit: self.limits.max_frame_bytes,
                });
            }
            formats.push(format);
        }
        validate_plane_combination(&formats)?;

        let coded_extent = [
            visible_width
                .checked_add(3)
                .ok_or(HapError::DimensionOverflow)?
                & !3,
            visible_height
                .checked_add(3)
                .ok_or(HapError::DimensionOverflow)?
                & !3,
        ];
        let mut planes = SmallVec::new();
        for (plane, format) in formats.into_iter().enumerate() {
            let expected = format.expected_bytes(visible_width, visible_height)?;
            let mut data = vec![0_u8; expected];
            let mut used = 0;
            let mut decoded_format = 0;
            // SAFETY: all buffers are valid for the supplied lengths and stay
            // alive for the duration of the synchronous reference call.
            let result = unsafe {
                sys::HapDecode(
                    packet.as_ptr().cast::<c_void>(),
                    packet_len,
                    plane as c_uint,
                    Some(serial_decode_callback),
                    ptr::null_mut(),
                    data.as_mut_ptr().cast::<c_void>(),
                    c_ulong::try_from(data.len()).map_err(|_| HapError::PlatformLengthOverflow)?,
                    &mut used,
                    &mut decoded_format,
                )
            };
            check_reference("decode", result)?;
            let actual = usize::try_from(used).map_err(|_| HapError::PlatformLengthOverflow)?;
            if actual != expected {
                return Err(HapError::DecodedSizeMismatch {
                    plane: plane as u32,
                    actual,
                    expected,
                });
            }
            if CompressedPlaneFormat::from_raw(decoded_format)? != format {
                return Err(HapError::Reference {
                    operation: "decode format",
                    result: ReferenceError::BadFrame,
                });
            }
            planes.push(DecodedPlane {
                format,
                coded_extent,
                visible_extent: [visible_width, visible_height],
                data,
            });
        }
        Ok(DecodedFrame { planes })
    }

    fn validate_dimensions(&self, width: u32, height: u32) -> Result<(), HapError> {
        if width == 0 || height == 0 {
            return Err(HapError::ZeroDimension);
        }
        if width > self.limits.max_dimension || height > self.limits.max_dimension {
            return Err(HapError::DimensionLimit {
                width,
                height,
                limit: self.limits.max_dimension,
            });
        }
        Ok(())
    }

    fn texture_count(&self, packet: &[u8], packet_len: c_ulong) -> Result<u32, HapError> {
        let mut count = 0;
        // SAFETY: packet is readable for packet_len bytes and count is writable.
        let result = unsafe {
            sys::HapGetFrameTextureCount(packet.as_ptr().cast::<c_void>(), packet_len, &mut count)
        };
        check_reference("texture count", result)?;
        if !(1..=2).contains(&count) {
            return Err(HapError::InvalidTextureCount(count));
        }
        Ok(count)
    }

    fn texture_format(
        &self,
        packet: &[u8],
        packet_len: c_ulong,
        plane: u32,
    ) -> Result<CompressedPlaneFormat, HapError> {
        let mut raw_format = 0;
        // SAFETY: packet is readable and raw_format is writable.
        let result = unsafe {
            sys::HapGetFrameTextureFormat(
                packet.as_ptr().cast::<c_void>(),
                packet_len,
                plane,
                &mut raw_format,
            )
        };
        check_reference("texture format", result)?;
        CompressedPlaneFormat::from_raw(raw_format)
    }

    fn validate_chunk_count(
        &self,
        packet: &[u8],
        packet_len: c_ulong,
        plane: u32,
    ) -> Result<(), HapError> {
        let mut count = 0;
        // SAFETY: packet is readable and count is writable.
        let result = unsafe {
            sys::HapGetFrameTextureChunkCount(
                packet.as_ptr().cast::<c_void>(),
                packet_len,
                plane,
                &mut count,
            )
        };
        check_reference("chunk count", result)?;
        if count <= 0 {
            return Err(HapError::InvalidChunkCount { plane, count });
        }
        if count as u32 > self.limits.max_chunks_per_plane {
            return Err(HapError::ChunkLimit {
                plane,
                count: count as u32,
                limit: self.limits.max_chunks_per_plane,
            });
        }
        Ok(())
    }
}

fn validate_plane_combination(formats: &[CompressedPlaneFormat]) -> Result<(), HapError> {
    match formats {
        [_] => Ok(()),
        [
            CompressedPlaneFormat::Bc3ScaledYCoCg,
            CompressedPlaneFormat::Bc4Alpha,
        ]
        | [
            CompressedPlaneFormat::Bc4Alpha,
            CompressedPlaneFormat::Bc3ScaledYCoCg,
        ] => Ok(()),
        _ => Err(HapError::UnsupportedPlaneCombination),
    }
}

fn check_reference(operation: &'static str, result: u32) -> Result<(), HapError> {
    if result == sys::HAP_RESULT_NO_ERROR {
        return Ok(());
    }
    let result = match result {
        sys::HAP_RESULT_BAD_ARGUMENTS => ReferenceError::BadArguments,
        sys::HAP_RESULT_BUFFER_TOO_SMALL => ReferenceError::BufferTooSmall,
        sys::HAP_RESULT_BAD_FRAME => ReferenceError::BadFrame,
        sys::HAP_RESULT_INTERNAL_ERROR => ReferenceError::Internal,
        value => ReferenceError::Unknown(value),
    };
    Err(HapError::Reference { operation, result })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_footprints_round_up_to_four_by_four() {
        assert_eq!(
            CompressedPlaneFormat::Bc1Rgb
                .expected_bytes(1920, 1080)
                .unwrap(),
            1_036_800
        );
        assert_eq!(
            CompressedPlaneFormat::Bc3Rgba
                .expected_bytes(1919, 1079)
                .unwrap(),
            2_073_600
        );
        assert_eq!(
            CompressedPlaneFormat::Bc4Alpha
                .expected_bytes(1, 1)
                .unwrap(),
            8
        );
    }

    #[test]
    fn rejects_dimensions_before_inspecting_packet() {
        let decoder = Decoder::default();
        assert_eq!(decoder.decode(&[], 0, 1080), Err(HapError::ZeroDimension));
        assert!(matches!(
            decoder.decode(&[], 20_000, 1080),
            Err(HapError::DimensionLimit { .. })
        ));
    }
}
