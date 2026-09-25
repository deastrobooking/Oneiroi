//! Safe HAP frame inspection and decode.
//!
//! This crate deliberately knows nothing about MOV or FFmpeg. A demuxer hands
//! it one encoded HAP packet plus the visible dimensions; it returns the
//! original BC texture blocks without expanding them to pixels.

mod decode;
mod format;

pub use decode::{DecodeLimits, DecodedFrame, DecodedPlane, Decoder, HapError};
pub use format::CompressedPlaneFormat;
