//! Wall-clock time source for the render loop.
//!
//! Time is kept in `f64` seconds rather than `f32`. A show runs for hours, and
//! `f32` seconds loses sub-millisecond resolution after roughly two hours —
//! long enough that nobody catches it in testing and short enough that
//! tempo-synced modulation visibly stutters by the end of a set.

use std::time::Instant;

/// A single frame's timing snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameTime {
    /// Seconds since the clock started.
    pub elapsed: f64,
    /// Seconds since the previous tick.
    pub delta: f64,
    /// Frames ticked since the clock started, starting at 0.
    pub frame: u64,
}

/// Monotonic frame clock.
///
/// `now` is passed in rather than read internally so tests can drive it with
/// synthetic instants, and so a future offline render can advance it at a
/// fixed step instead of wall-clock rate.
#[derive(Debug, Clone)]
pub struct Clock {
    start: Instant,
    last: Instant,
    frame: u64,
}

impl Clock {
    pub fn new(now: Instant) -> Self {
        Self {
            start: now,
            last: now,
            frame: 0,
        }
    }

    /// Advance to `now` and return the resulting frame timing.
    ///
    /// `now` earlier than the previous tick yields a zero delta rather than a
    /// negative one; `Instant` is monotonic, but clamping keeps a stale
    /// timestamp from feeding a negative step into modulation.
    pub fn tick(&mut self, now: Instant) -> FrameTime {
        let delta = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        let frame = self.frame;
        self.frame += 1;
        FrameTime {
            elapsed: now.saturating_duration_since(self.start).as_secs_f64(),
            delta,
            frame,
        }
    }

    /// Frames ticked so far.
    pub fn frame(&self) -> u64 {
        self.frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn first_tick_has_zero_delta_and_frame_zero() {
        let base = Instant::now();
        let mut clock = Clock::new(base);

        let t = clock.tick(base);

        assert_eq!(t.frame, 0);
        assert_eq!(t.delta, 0.0);
        assert_eq!(t.elapsed, 0.0);
    }

    #[test]
    fn accumulates_elapsed_across_ticks() {
        let base = Instant::now();
        let mut clock = Clock::new(base);
        let step = Duration::from_millis(16);

        clock.tick(base + step);
        let t = clock.tick(base + step * 2);

        assert_eq!(t.frame, 1);
        assert!((t.delta - 0.016).abs() < 1e-9);
        assert!((t.elapsed - 0.032).abs() < 1e-9);
        assert_eq!(clock.frame(), 2);
    }

    #[test]
    fn backwards_tick_clamps_to_zero_delta() {
        let base = Instant::now();
        let mut clock = Clock::new(base);
        clock.tick(base + Duration::from_millis(100));

        let t = clock.tick(base + Duration::from_millis(50));

        assert_eq!(t.delta, 0.0);
    }
}
