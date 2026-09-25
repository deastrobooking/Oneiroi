//! Owned FFmpeg interrupt callback state for camera open/read operations.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::FfmpegDecodeError;

pub(crate) const CAMERA_OPEN_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const CAMERA_READ_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default)]
pub(crate) struct CaptureCancellation {
    epoch: Arc<AtomicU64>,
    expected: u64,
}

impl CaptureCancellation {
    pub fn next(&self) -> Self {
        let expected = self.epoch.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        Self {
            epoch: Arc::clone(&self.epoch),
            expected,
        }
    }

    pub fn is_canceled(&self) -> bool {
        self.epoch.load(Ordering::Acquire) != self.expected
    }
}

pub(crate) struct CaptureInterrupt {
    cancellation: CaptureCancellation,
    deadline: Instant,
}

impl CaptureInterrupt {
    pub fn new(cancellation: CaptureCancellation) -> Self {
        Self {
            cancellation,
            deadline: Instant::now() + CAMERA_OPEN_TIMEOUT,
        }
    }

    pub fn begin_read(&mut self) {
        self.deadline = Instant::now() + CAMERA_READ_TIMEOUT;
    }

    pub fn check(&self) -> Result<(), FfmpegDecodeError> {
        if self.cancellation.is_canceled() {
            Err(FfmpegDecodeError::CaptureCanceled)
        } else if Instant::now() >= self.deadline {
            Err(FfmpegDecodeError::CaptureTimeout)
        } else {
            Ok(())
        }
    }

    pub fn callback(&mut self) -> ffmpeg_next::ffi::AVIOInterruptCB {
        ffmpeg_next::ffi::AVIOInterruptCB {
            callback: Some(capture_interrupted),
            opaque: (self as *mut Self).cast(),
        }
    }
}

unsafe extern "C" fn capture_interrupted(opaque: *mut std::ffi::c_void) -> std::ffi::c_int {
    // SAFETY: The decoder keeps the boxed CaptureInterrupt at a stable address
    // until after its AVFormatContext is closed. FFmpeg invokes this callback
    // synchronously during open/read/close; deadline changes occur between calls.
    let interrupt = unsafe { &*opaque.cast::<CaptureInterrupt>() };
    i32::from(interrupt.check().is_err())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_cancels_old_and_queued_captures_without_canceling_the_new_one() {
        let owner = CaptureCancellation::default();
        let first = owner.next();
        let second = owner.next();
        assert!(first.is_canceled());
        assert!(!second.is_canceled());
        let _ = owner.next(); // Stop or shutdown.
        assert!(second.is_canceled());
    }

    #[test]
    fn ffmpeg_callback_observes_cancellation_and_expired_deadlines() {
        let owner = CaptureCancellation::default();
        let mut interrupt = Box::new(CaptureInterrupt::new(owner.next()));
        let callback = interrupt.callback();
        // SAFETY: The callback state remains allocated for all calls below.
        let check = || unsafe { callback.callback.unwrap()(callback.opaque) };
        assert_eq!(check(), 0);
        interrupt.deadline = Instant::now();
        assert_eq!(check(), 1);
        assert!(matches!(
            interrupt.check(),
            Err(FfmpegDecodeError::CaptureTimeout)
        ));
        interrupt.begin_read();
        assert_eq!(check(), 0);
        let _ = owner.next();
        assert_eq!(check(), 1);
        assert!(matches!(
            interrupt.check(),
            Err(FfmpegDecodeError::CaptureCanceled)
        ));
    }
}
