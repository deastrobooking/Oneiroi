//! Unified frames crossing from decoder workers to render scheduling.

use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

use oneiroi_core::MediaTime;
use oneiroi_hap::DecodedFrame as DecodedHapFrame;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FramePoolStats {
    pub allocations: u64,
    pub reuses: u64,
    pub returned: u64,
    pub discarded: u64,
    pub in_flight: u64,
    pub allocated_bytes: u64,
}

#[derive(Default)]
struct FramePoolTelemetry {
    allocations: AtomicU64,
    reuses: AtomicU64,
    returned: AtomicU64,
    discarded: AtomicU64,
    in_flight: AtomicU64,
    allocated_bytes: AtomicU64,
}

#[derive(Clone)]
pub struct FrameBufferPool {
    sender: SyncSender<Vec<u8>>,
    receiver: Arc<Mutex<Receiver<Vec<u8>>>>,
    telemetry: Arc<FramePoolTelemetry>,
}

impl FrameBufferPool {
    pub fn new(capacity: usize) -> Self {
        let (sender, receiver) = sync_channel(capacity.max(1));
        Self {
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
            telemetry: Arc::new(FramePoolTelemetry::default()),
        }
    }

    pub fn acquire(&self, length: usize) -> FrameData {
        let recycled = self
            .receiver
            .lock()
            .expect("frame pool receiver poisoned")
            .try_recv()
            .ok();
        let mut data = if let Some(data) = recycled {
            self.telemetry.reuses.fetch_add(1, Ordering::Relaxed);
            data
        } else {
            self.telemetry.allocations.fetch_add(1, Ordering::Relaxed);
            self.telemetry
                .allocated_bytes
                .fetch_add(length as u64, Ordering::Relaxed);
            Vec::with_capacity(length)
        };
        if data.capacity() < length {
            let before = data.capacity();
            data.reserve_exact(length - data.len());
            self.telemetry.allocations.fetch_add(1, Ordering::Relaxed);
            self.telemetry.allocated_bytes.fetch_add(
                data.capacity().saturating_sub(before) as u64,
                Ordering::Relaxed,
            );
        }
        data.resize(length, 0);
        self.telemetry.in_flight.fetch_add(1, Ordering::Relaxed);
        FrameData {
            inner: Arc::new(FrameDataInner {
                data,
                recycler: Some(FrameRecycler {
                    sender: self.sender.clone(),
                    telemetry: self.telemetry.clone(),
                }),
            }),
        }
    }

    pub fn stats(&self) -> FramePoolStats {
        FramePoolStats {
            allocations: self.telemetry.allocations.load(Ordering::Relaxed),
            reuses: self.telemetry.reuses.load(Ordering::Relaxed),
            returned: self.telemetry.returned.load(Ordering::Relaxed),
            discarded: self.telemetry.discarded.load(Ordering::Relaxed),
            in_flight: self.telemetry.in_flight.load(Ordering::Relaxed),
            allocated_bytes: self.telemetry.allocated_bytes.load(Ordering::Relaxed),
        }
    }
}

struct FrameRecycler {
    sender: SyncSender<Vec<u8>>,
    telemetry: Arc<FramePoolTelemetry>,
}

struct FrameDataInner {
    data: Vec<u8>,
    recycler: Option<FrameRecycler>,
}

impl Drop for FrameDataInner {
    fn drop(&mut self) {
        let Some(recycler) = self.recycler.take() else {
            return;
        };
        recycler.telemetry.in_flight.fetch_sub(1, Ordering::Relaxed);
        let mut data = std::mem::take(&mut self.data);
        data.clear();
        match recycler.sender.try_send(data) {
            Ok(()) => {
                recycler.telemetry.returned.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                recycler.telemetry.discarded.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[derive(Clone)]
pub struct FrameData {
    inner: Arc<FrameDataInner>,
}

impl FrameData {
    pub fn as_slice(&self) -> &[u8] {
        &self.inner.data
    }
}

impl From<Vec<u8>> for FrameData {
    fn from(data: Vec<u8>) -> Self {
        Self {
            inner: Arc::new(FrameDataInner {
                data,
                recycler: None,
            }),
        }
    }
}

impl Deref for FrameData {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl DerefMut for FrameData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut Arc::get_mut(&mut self.inner)
            .expect("cannot mutate shared frame data")
            .data
    }
}

impl fmt::Debug for FrameData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrameData")
            .field("len", &self.len())
            .finish()
    }
}

impl PartialEq for FrameData {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for FrameData {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbaFrame {
    pub extent: [u32; 2],
    /// Tightly packed, top-to-bottom RGBA8 rows.
    pub data: FrameData,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returned_buffer_is_reused_without_another_allocation() {
        let pool = FrameBufferPool::new(2);
        {
            let first = pool.acquire(1024);
            assert_eq!(first.len(), 1024);
            assert_eq!(pool.stats().allocations, 1);
            assert_eq!(pool.stats().in_flight, 1);
        }
        assert_eq!(pool.stats().returned, 1);
        let second = pool.acquire(1024);
        let stats = pool.stats();
        assert_eq!(second.len(), 1024);
        assert_eq!(stats.allocations, 1);
        assert_eq!(stats.reuses, 1);
        assert_eq!(stats.in_flight, 1);
    }

    #[test]
    fn clones_return_storage_only_after_the_last_owner_drops() {
        let pool = FrameBufferPool::new(1);
        let first = pool.acquire(16);
        let clone = first.clone();
        drop(first);
        assert_eq!(pool.stats().in_flight, 1);
        assert_eq!(pool.stats().returned, 0);
        drop(clone);
        assert_eq!(pool.stats().in_flight, 0);
        assert_eq!(pool.stats().returned, 1);
    }

    #[test]
    fn full_return_queue_discards_without_blocking() {
        let pool = FrameBufferPool::new(1);
        let first = pool.acquire(16);
        let second = pool.acquire(16);
        drop(first);
        drop(second);
        let stats = pool.stats();
        assert_eq!(stats.returned, 1);
        assert_eq!(stats.discarded, 1);
        assert_eq!(stats.in_flight, 0);
    }

    #[test]
    fn accelerated_soak_keeps_fixed_size_frame_storage_bounded() {
        const CYCLES: u64 = 100_000;
        let pool = FrameBufferPool::new(2);

        for _ in 0..CYCLES {
            drop(pool.acquire(4096));
        }

        let stats = pool.stats();
        assert_eq!(stats.allocations, 1);
        assert_eq!(stats.reuses, CYCLES - 1);
        assert_eq!(stats.returned, CYCLES);
        assert_eq!(stats.discarded, 0);
        assert_eq!(stats.in_flight, 0);
        assert_eq!(stats.allocated_bytes, 4096);
    }
}
