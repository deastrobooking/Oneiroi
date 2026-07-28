//! Clip decode: demux, HAP, ffmpeg fallbacks, frame ring buffers.
//!
//! Deliberately separate from `oneiroi-render` so the decode side can hand
//! over block-compressed bytes without knowing what a `wgpu::Device` is.
//! HAP packet decoding itself lives in `oneiroi-hap`; container demux and
//! timestamped scheduling will be assembled here.

mod demux;
mod schedule;

pub use demux::{
    DemuxError, DemuxedHapFrame, EncodedHapPacket, FrameRate, HapDemuxer, HapStreamMetadata,
};
pub use schedule::{
    DiscontinuityPolicy, EnqueueError, FrameScheduler, FrameSelection, ScheduledFrame,
    SchedulerError, SchedulerStats,
};
