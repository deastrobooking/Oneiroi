//! Clip decode: demux, HAP, ffmpeg fallbacks, frame ring buffers.
//!
//! Deliberately separate from `oneiroi-render` so the decode side can hand
//! over block-compressed bytes without knowing what a `wgpu::Device` is.
//! HAP packet decoding itself lives in `oneiroi-hap`; container demux and
//! timestamped scheduling will be assembled here.

mod decode_ffmpeg;
mod demux;
mod frame;
mod mixer;
mod probe;
mod schedule;
mod worker;

pub use decode_ffmpeg::{DecodedRgbaFrame, FfmpegDecodeError, FfmpegVideoDecoder};
pub use demux::{
    DemuxError, DemuxedHapFrame, EncodedHapPacket, FrameRate, HapDemuxer, HapStreamMetadata,
};
pub use frame::{RgbaFrame, VideoFrame, VideoFramePayload};
pub use mixer::{
    Deck, DeckId, DeckState, FourDeckMixer, ImportRequest, ImportResult, MediaImporter, SubmitError,
};
pub use probe::{AlphaMode, DecodePath, MediaHealth, MovieMetadata, ProbeError, probe_movie};
pub use schedule::{
    DiscontinuityPolicy, EnqueueError, FrameScheduler, FrameSelection, ScheduledFrame,
    SchedulerError, SchedulerStats,
};
pub use worker::{DeckDecoder, DecoderEvent};
