//! Unified frames crossing from decoder workers to render scheduling.

use oneiroi_core::MediaTime;
use oneiroi_hap::DecodedFrame as DecodedHapFrame;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbaFrame {
    pub extent: [u32; 2],
    /// Tightly packed, top-to-bottom RGBA8 rows.
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VideoFramePayload {
    BlockCompressed(DecodedHapFrame),
    Rgba8(RgbaFrame),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoFrame {
    pub pts: MediaTime,
    pub duration: Option<MediaTime>,
    pub generation: u64,
    pub sequence: u64,
    pub payload: VideoFramePayload,
}
