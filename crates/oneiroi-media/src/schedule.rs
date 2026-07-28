//! Render-time frame selection, independent of decode timing.

use std::collections::VecDeque;

use oneiroi_core::MediaTime;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledFrame<T> {
    pub pts: MediaTime,
    pub duration: Option<MediaTime>,
    pub generation: u64,
    pub sequence: u64,
    pub payload: T,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SchedulerStats {
    /// Eligible frames skipped because a newer eligible frame was available.
    pub dropped: u64,
    /// Newly selected frames whose declared display interval had expired.
    pub late: u64,
    /// Render selections that held the previously selected frame.
    pub repeated: u64,
    /// Queued frames discarded because their generation was obsolete.
    pub invalidated: u64,
    /// Frames refused because the bounded queue was full.
    pub queue_full: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscontinuityPolicy {
    HoldLastFrame,
    Blank,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SchedulerError {
    #[error("frame queue capacity must be greater than zero")]
    ZeroCapacity,
}

#[derive(Debug, Eq, PartialEq)]
pub enum EnqueueError<T> {
    Full(ScheduledFrame<T>),
    OutOfOrder(ScheduledFrame<T>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameSelection<'a, T> {
    Advanced(&'a ScheduledFrame<T>),
    Held(&'a ScheduledFrame<T>),
    Missing,
}

/// A bounded timestamp queue and hold/drop selector for one active clip.
pub struct FrameScheduler<T> {
    queue: VecDeque<ScheduledFrame<T>>,
    capacity: usize,
    generation: u64,
    current: Option<ScheduledFrame<T>>,
    discontinuity_policy: DiscontinuityPolicy,
    stats: SchedulerStats,
}

impl<T> FrameScheduler<T> {
    pub fn new(
        capacity: usize,
        generation: u64,
        discontinuity_policy: DiscontinuityPolicy,
    ) -> Result<Self, SchedulerError> {
        if capacity == 0 {
            return Err(SchedulerError::ZeroCapacity);
        }
        Ok(Self {
            queue: VecDeque::with_capacity(capacity),
            capacity,
            generation,
            current: None,
            discontinuity_policy,
            stats: SchedulerStats::default(),
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn activate_generation(&mut self, generation: u64) {
        self.generation = generation;
        if self.discontinuity_policy == DiscontinuityPolicy::Blank {
            self.current = None;
        }
    }

    pub fn enqueue(&mut self, frame: ScheduledFrame<T>) -> Result<(), EnqueueError<T>> {
        if self.queue.len() == self.capacity {
            self.stats.queue_full = self.stats.queue_full.saturating_add(1);
            return Err(EnqueueError::Full(frame));
        }
        if self
            .queue
            .back()
            .is_some_and(|back| back.generation == frame.generation && back.pts > frame.pts)
        {
            return Err(EnqueueError::OutOfOrder(frame));
        }
        self.queue.push_back(frame);
        Ok(())
    }

    /// Select the newest frame whose PTS is not later than `target`.
    ///
    /// Early frames stay queued. An underrun holds the current frame. Frames
    /// from any non-active generation are discarded before consideration.
    pub fn select(&mut self, target: MediaTime) -> FrameSelection<'_, T> {
        while self
            .queue
            .front()
            .is_some_and(|frame| frame.generation != self.generation)
        {
            self.queue.pop_front();
            self.stats.invalidated = self.stats.invalidated.saturating_add(1);
        }

        let mut advanced = false;
        while self
            .queue
            .front()
            .is_some_and(|frame| frame.generation == self.generation && frame.pts <= target)
        {
            let frame = self.queue.pop_front().expect("front checked above");
            if advanced {
                self.stats.dropped = self.stats.dropped.saturating_add(1);
            }
            self.current = Some(frame);
            advanced = true;
        }

        if advanced {
            let current = self.current.as_ref().expect("set while advancing");
            if current
                .duration
                .and_then(|duration| current.pts.checked_add(duration).ok())
                .is_some_and(|end| end < target)
            {
                self.stats.late = self.stats.late.saturating_add(1);
            }
            FrameSelection::Advanced(current)
        } else if let Some(current) = self.current.as_ref() {
            self.stats.repeated = self.stats.repeated.saturating_add(1);
            FrameSelection::Held(current)
        } else {
            FrameSelection::Missing
        }
    }

    pub fn current(&self) -> Option<&ScheduledFrame<T>> {
        self.current.as_ref()
    }

    pub fn queued_len(&self) -> usize {
        self.queue.len()
    }

    pub fn stats(&self) -> SchedulerStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(ticks: i64) -> MediaTime {
        MediaTime::new(ticks, 30).unwrap()
    }

    fn frame(sequence: u64, generation: u64) -> ScheduledFrame<u64> {
        ScheduledFrame {
            pts: time(sequence as i64),
            duration: Some(time(1)),
            generation,
            sequence,
            payload: sequence,
        }
    }

    fn scheduler(policy: DiscontinuityPolicy) -> FrameScheduler<u64> {
        FrameScheduler::new(4, 1, policy).unwrap()
    }

    #[test]
    fn drains_to_newest_eligible_frame_and_counts_drop() {
        let mut scheduler = scheduler(DiscontinuityPolicy::HoldLastFrame);
        scheduler.enqueue(frame(0, 1)).unwrap();
        scheduler.enqueue(frame(1, 1)).unwrap();
        scheduler.enqueue(frame(2, 1)).unwrap();

        let selected = scheduler.select(time(1));

        assert!(matches!(
            selected,
            FrameSelection::Advanced(frame) if frame.sequence == 1
        ));
        assert_eq!(scheduler.queued_len(), 1);
        assert_eq!(scheduler.stats().dropped, 1);
    }

    #[test]
    fn retains_early_frame_and_holds_on_underrun() {
        let mut scheduler = scheduler(DiscontinuityPolicy::HoldLastFrame);
        scheduler.enqueue(frame(0, 1)).unwrap();
        scheduler.enqueue(frame(2, 1)).unwrap();
        assert!(matches!(
            scheduler.select(time(0)),
            FrameSelection::Advanced(_)
        ));

        let selected = scheduler.select(time(1));

        assert!(matches!(
            selected,
            FrameSelection::Held(frame) if frame.sequence == 0
        ));
        assert_eq!(scheduler.queued_len(), 1);
        assert_eq!(scheduler.stats().repeated, 1);
    }

    #[test]
    fn obsolete_generation_can_never_flash_after_seek() {
        let mut scheduler = scheduler(DiscontinuityPolicy::HoldLastFrame);
        scheduler.enqueue(frame(0, 1)).unwrap();
        scheduler.select(time(0));
        scheduler.activate_generation(2);
        scheduler.enqueue(frame(1, 1)).unwrap();
        scheduler.enqueue(frame(0, 2)).unwrap();

        let selected = scheduler.select(time(1));

        assert!(matches!(
            selected,
            FrameSelection::Advanced(frame)
                if frame.generation == 2 && frame.sequence == 0
        ));
        assert_eq!(scheduler.stats().invalidated, 1);
    }

    #[test]
    fn blank_policy_drops_current_frame_at_discontinuity() {
        let mut scheduler = scheduler(DiscontinuityPolicy::Blank);
        scheduler.enqueue(frame(0, 1)).unwrap();
        scheduler.select(time(0));

        scheduler.activate_generation(2);

        assert!(matches!(scheduler.select(time(0)), FrameSelection::Missing));
    }

    #[test]
    fn bounded_queue_refuses_growth() {
        let mut scheduler = FrameScheduler::new(1, 1, DiscontinuityPolicy::HoldLastFrame).unwrap();
        scheduler.enqueue(frame(0, 1)).unwrap();

        assert!(matches!(
            scheduler.enqueue(frame(1, 1)),
            Err(EnqueueError::Full(_))
        ));
        assert_eq!(scheduler.stats().queue_full, 1);
    }
}
