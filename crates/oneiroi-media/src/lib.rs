//! Clip decode: demux, HAP, ffmpeg fallbacks, frame ring buffers.
//!
//! Deliberately separate from `oneiroi-render` so the decode side can hand
//! over block-compressed bytes without knowing what a `wgpu::Device` is.
//! HAP packet decoding itself lives in `oneiroi-hap`; container demux and
//! timestamped scheduling will be assembled here.

mod capture;
mod clips;
mod decode_ffmpeg;
mod demux;
mod folder;
mod frame;
mod keyframe;
mod mixer;
mod probe;
mod schedule;
mod thumbnail;
mod transport;
mod worker;

pub use capture::{
    CAMERA_SCHEME, CameraConfig, CameraDevice, CameraDiscoveryError, camera_pts, discover_cameras,
};
pub use clips::{
    CLIPS_PER_DECK, ClipAddress, ClipBank, ClipLaunchMode, ClipPlayback, ClipRestoreRequest,
    ClipRestoreResult, ClipRestorer, ClipSlot, LaunchQueue,
};
pub use decode_ffmpeg::{DecodedRgbaFrame, FfmpegDecodeError, FfmpegVideoDecoder};
pub use demux::{
    DemuxError, DemuxedHapFrame, EncodedHapPacket, FrameRate, HapDemuxer, HapStreamMetadata,
};
pub use folder::{FolderScanRequest, FolderScanResult, FolderScanner, is_supported_media_path};
pub use frame::{
    FrameBufferPool, FrameData, FramePoolStats, RgbaFrame, VideoFrame, VideoFramePayload,
};
pub use keyframe::{KeyframeIndex, MAX_KEYFRAME_ENTRIES};
pub use mixer::{
    CrossfadeBus, Deck, DeckId, DeckState, FourDeckMixer, ImportRequest, ImportResult,
    MediaImporter, SubmitError, crossfade_gains,
};
pub use probe::{AlphaMode, DecodePath, MediaHealth, MovieMetadata, ProbeError, probe_movie};
pub use schedule::{
    DiscontinuityPolicy, EnqueueError, FrameScheduler, FrameSelection, ScheduledFrame,
    SchedulerError, SchedulerStats,
};
pub use thumbnail::{
    PRELOAD_MAX_BYTES_PER_FRAME, PRELOAD_MAX_EXTENT, THUMBNAIL_MAX_EXTENT, Thumbnail,
    ThumbnailRequest, ThumbnailResult, ThumbnailWorker,
};
pub use transport::{DeckTransport, EndMode, TransportEvent};
pub use worker::{DeckDecoder, DecoderEvent, DecoderFailureInjection};
