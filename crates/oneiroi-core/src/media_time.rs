//! Exact rational timestamps used at codec and transport boundaries.

use std::cmp::Ordering;
use std::fmt;

use thiserror::Error;

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum MediaTimeError {
    #[error("media timescale must be positive")]
    InvalidTimescale,
    #[error("media timestamp arithmetic overflow")]
    Overflow,
}

/// An exact number of seconds represented as `ticks / timescale`.
///
/// Values are normalized on construction, so structural equality and hashing
/// have the same meaning as rational equality.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct MediaTime {
    ticks: i64,
    timescale: i64,
}

impl MediaTime {
    pub const ZERO: Self = Self {
        ticks: 0,
        timescale: 1,
    };

    pub fn new(ticks: i64, timescale: i64) -> Result<Self, MediaTimeError> {
        if timescale <= 0 {
            return Err(MediaTimeError::InvalidTimescale);
        }
        if ticks == 0 {
            return Ok(Self::ZERO);
        }

        let divisor = gcd(ticks.unsigned_abs(), timescale as u64);
        Ok(Self {
            ticks: ticks / divisor as i64,
            timescale: timescale / divisor as i64,
        })
    }

    /// Convert a timestamp expressed in units of `numerator / denominator`
    /// seconds, which is FFmpeg's stream time-base representation.
    pub fn from_time_base(
        timestamp: i64,
        numerator: i32,
        denominator: i32,
    ) -> Result<Self, MediaTimeError> {
        if numerator <= 0 || denominator <= 0 {
            return Err(MediaTimeError::InvalidTimescale);
        }
        let ticks = timestamp
            .checked_mul(i64::from(numerator))
            .ok_or(MediaTimeError::Overflow)?;
        Self::new(ticks, i64::from(denominator))
    }

    pub fn ticks(self) -> i64 {
        self.ticks
    }

    pub fn timescale(self) -> i64 {
        self.timescale
    }

    pub fn as_seconds(self) -> f64 {
        self.ticks as f64 / self.timescale as f64
    }

    pub fn checked_add(self, other: Self) -> Result<Self, MediaTimeError> {
        let ticks = i128::from(self.ticks) * i128::from(other.timescale)
            + i128::from(other.ticks) * i128::from(self.timescale);
        let timescale = i128::from(self.timescale) * i128::from(other.timescale);
        let divisor = gcd_u128(ticks.unsigned_abs(), timescale as u128);
        let ticks = ticks / divisor as i128;
        let timescale = timescale / divisor as i128;
        Self::new(
            i64::try_from(ticks).map_err(|_| MediaTimeError::Overflow)?,
            i64::try_from(timescale).map_err(|_| MediaTimeError::Overflow)?,
        )
    }
}

impl fmt::Debug for MediaTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}s", self.ticks, self.timescale)
    }
}

impl Ord for MediaTime {
    fn cmp(&self, other: &Self) -> Ordering {
        (i128::from(self.ticks) * i128::from(other.timescale))
            .cmp(&(i128::from(other.ticks) * i128::from(self.timescale)))
    }
}

impl PartialOrd for MediaTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

const fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

const fn gcd_u128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_equivalent_times() {
        assert_eq!(
            MediaTime::new(48, 24).unwrap(),
            MediaTime::new(2, 1).unwrap()
        );
        assert_eq!(MediaTime::new(0, 90_000).unwrap(), MediaTime::ZERO);
    }

    #[test]
    fn converts_non_unit_ffmpeg_time_base() {
        let time = MediaTime::from_time_base(50, 1, 25).unwrap();
        assert_eq!(time, MediaTime::new(2, 1).unwrap());
    }

    #[test]
    fn compares_different_timescales_exactly() {
        assert!(MediaTime::new(1, 24).unwrap() < MediaTime::new(1, 23).unwrap());
        assert!(MediaTime::new(-1, 24).unwrap() < MediaTime::ZERO);
    }

    #[test]
    fn adds_and_reduces_exactly() {
        let sum = MediaTime::new(1, 24)
            .unwrap()
            .checked_add(MediaTime::new(1, 30).unwrap())
            .unwrap();
        assert_eq!(sum, MediaTime::new(3, 40).unwrap());
    }
}
