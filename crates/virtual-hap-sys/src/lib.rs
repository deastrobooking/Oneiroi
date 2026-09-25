//! Raw bindings to Vidvox's HAP reference implementation.
//!
//! The vendored C source is pinned to upstream commit
//! `d847f6bbd3be88575dd4ef33a877243780e3be76`. It expects the Snappy C ABI;
//! this crate supplies that ABI with Rust's memory-safe `snap` implementation
//! so consumers do not need a system `libsnappy`.

#![allow(non_camel_case_types, non_snake_case)]

use std::ffi::{c_int, c_uint, c_ulong, c_void};
use std::slice;

pub const HAP_TEXTURE_FORMAT_RGB_DXT1: c_uint = 0x83F0;
pub const HAP_TEXTURE_FORMAT_RGBA_DXT5: c_uint = 0x83F3;
pub const HAP_TEXTURE_FORMAT_YCOCG_DXT5: c_uint = 0x01;
pub const HAP_TEXTURE_FORMAT_A_RGTC1: c_uint = 0x8DBB;
pub const HAP_TEXTURE_FORMAT_RGBA_BPTC_UNORM: c_uint = 0x8E8C;
pub const HAP_TEXTURE_FORMAT_RGB_BPTC_UNSIGNED_FLOAT: c_uint = 0x8E8F;
pub const HAP_TEXTURE_FORMAT_RGB_BPTC_SIGNED_FLOAT: c_uint = 0x8E8E;

pub const HAP_COMPRESSOR_NONE: c_uint = 0;
pub const HAP_COMPRESSOR_SNAPPY: c_uint = 1;

pub const HAP_RESULT_NO_ERROR: c_uint = 0;
pub const HAP_RESULT_BAD_ARGUMENTS: c_uint = 1;
pub const HAP_RESULT_BUFFER_TOO_SMALL: c_uint = 2;
pub const HAP_RESULT_BAD_FRAME: c_uint = 3;
pub const HAP_RESULT_INTERNAL_ERROR: c_uint = 4;

pub type HapDecodeWorkFunction = Option<unsafe extern "C" fn(*mut c_void, c_uint)>;
pub type HapDecodeCallback =
    Option<unsafe extern "C" fn(HapDecodeWorkFunction, *mut c_void, c_uint, *mut c_void)>;

unsafe extern "C" {
    pub fn HapMaxEncodedLength(
        count: c_uint,
        lengths: *mut c_ulong,
        texture_formats: *mut c_uint,
        chunk_counts: *mut c_uint,
    ) -> c_ulong;

    pub fn HapEncode(
        count: c_uint,
        input_buffers: *mut *const c_void,
        input_buffer_bytes: *mut c_ulong,
        texture_formats: *mut c_uint,
        compressors: *mut c_uint,
        chunk_counts: *mut c_uint,
        output_buffer: *mut c_void,
        output_buffer_bytes: c_ulong,
        output_buffer_bytes_used: *mut c_ulong,
    ) -> c_uint;

    pub fn HapDecode(
        input_buffer: *const c_void,
        input_buffer_bytes: c_ulong,
        index: c_uint,
        callback: HapDecodeCallback,
        info: *mut c_void,
        output_buffer: *mut c_void,
        output_buffer_bytes: c_ulong,
        output_buffer_bytes_used: *mut c_ulong,
        output_buffer_texture_format: *mut c_uint,
    ) -> c_uint;

    pub fn HapGetFrameTextureCount(
        input_buffer: *const c_void,
        input_buffer_bytes: c_ulong,
        output_texture_count: *mut c_uint,
    ) -> c_uint;

    pub fn HapGetFrameTextureFormat(
        input_buffer: *const c_void,
        input_buffer_bytes: c_ulong,
        index: c_uint,
        output_buffer_texture_format: *mut c_uint,
    ) -> c_uint;

    pub fn HapGetFrameTextureChunkCount(
        input_buffer: *const c_void,
        input_buffer_bytes: c_ulong,
        index: c_uint,
        chunk_count: *mut c_int,
    ) -> c_uint;
}

const SNAPPY_OK: c_int = 0;
const SNAPPY_INVALID_INPUT: c_int = 1;
const SNAPPY_BUFFER_TOO_SMALL: c_int = 2;

#[unsafe(no_mangle)]
extern "C" fn snappy_max_compressed_length(source_length: usize) -> usize {
    snap::raw::max_compress_len(source_length)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn snappy_compress(
    input: *const i8,
    input_length: usize,
    compressed: *mut i8,
    compressed_length: *mut usize,
) -> c_int {
    if input.is_null() || compressed.is_null() || compressed_length.is_null() {
        return SNAPPY_INVALID_INPUT;
    }

    // SAFETY: the C caller promises buffers of the supplied lengths. The
    // output capacity is read before constructing its mutable slice.
    let (input, output) = unsafe {
        (
            slice::from_raw_parts(input.cast::<u8>(), input_length),
            slice::from_raw_parts_mut(compressed.cast::<u8>(), *compressed_length),
        )
    };
    match snap::raw::Encoder::new().compress(input, output) {
        Ok(written) => {
            // SAFETY: checked non-null above.
            unsafe { *compressed_length = written };
            SNAPPY_OK
        }
        Err(snap::Error::BufferTooSmall { .. }) => SNAPPY_BUFFER_TOO_SMALL,
        Err(_) => SNAPPY_INVALID_INPUT,
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn snappy_uncompressed_length(
    compressed: *const i8,
    compressed_length: usize,
    result: *mut usize,
) -> c_int {
    if compressed.is_null() || result.is_null() {
        return SNAPPY_INVALID_INPUT;
    }
    // SAFETY: the C caller promises a readable input buffer.
    let input = unsafe { slice::from_raw_parts(compressed.cast::<u8>(), compressed_length) };
    match snap::raw::decompress_len(input) {
        Ok(length) => {
            // SAFETY: checked non-null above.
            unsafe { *result = length };
            SNAPPY_OK
        }
        Err(_) => SNAPPY_INVALID_INPUT,
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn snappy_uncompress(
    compressed: *const i8,
    compressed_length: usize,
    uncompressed: *mut i8,
    uncompressed_length: *mut usize,
) -> c_int {
    if compressed.is_null() || uncompressed.is_null() || uncompressed_length.is_null() {
        return SNAPPY_INVALID_INPUT;
    }
    // SAFETY: the C caller promises buffers of the supplied lengths.
    let (input, output) = unsafe {
        (
            slice::from_raw_parts(compressed.cast::<u8>(), compressed_length),
            slice::from_raw_parts_mut(uncompressed.cast::<u8>(), *uncompressed_length),
        )
    };
    match snap::raw::Decoder::new().decompress(input, output) {
        Ok(written) => {
            // SAFETY: checked non-null above.
            unsafe { *uncompressed_length = written };
            SNAPPY_OK
        }
        Err(snap::Error::BufferTooSmall { .. }) => SNAPPY_BUFFER_TOO_SMALL,
        Err(_) => SNAPPY_INVALID_INPUT,
    }
}
