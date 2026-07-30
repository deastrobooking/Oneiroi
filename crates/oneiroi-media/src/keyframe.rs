//! Bounded conventional-codec keyframe index.

use oneiroi_core::MediaTime;

pub const MAX_KEYFRAME_ENTRIES: usize = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyframeIndex {
    entries: Vec<MediaTime>,
    complete: bool,
}

impl Default for KeyframeIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyframeIndex {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            complete: true,
        }
    }

    pub fn push(&mut self, timestamp: MediaTime) -> bool {
        if self.entries.len() >= MAX_KEYFRAME_ENTRIES {
            self.complete = false;
            return false;
        }
        if self.entries.last().is_some_and(|last| *last == timestamp) {
            return true;
        }
        self.entries.push(timestamp);
        true
    }

    pub fn finish(&mut self) {
        self.entries.sort_unstable();
        self.entries.dedup();
    }

    pub fn nearest_preceding(&self, target: MediaTime) -> Option<MediaTime> {
        match self.entries.binary_search(&target) {
            Ok(index) => self.entries.get(index).copied(),
            Err(0) => None,
            Err(index) => self.entries.get(index - 1).copied(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn estimated_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<MediaTime>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(seconds: i64) -> MediaTime {
        MediaTime::new(seconds, 1).unwrap()
    }

    #[test]
    fn selects_exact_or_preceding_keyframe() {
        let mut index = KeyframeIndex::new();
        for seconds in [10, 0, 5, 5] {
            index.push(time(seconds));
        }
        index.finish();
        assert_eq!(index.len(), 3);
        assert_eq!(index.nearest_preceding(time(7)), Some(time(5)));
        assert_eq!(index.nearest_preceding(time(5)), Some(time(5)));
        assert_eq!(index.nearest_preceding(time(0)), Some(time(0)));
    }

    #[test]
    fn returns_none_before_first_keyframe() {
        let mut index = KeyframeIndex::new();
        index.push(time(2));
        index.finish();
        assert_eq!(index.nearest_preceding(time(1)), None);
    }

    #[test]
    fn index_stops_at_the_memory_bound() {
        let mut index = KeyframeIndex::new();
        for entry in 0..=MAX_KEYFRAME_ENTRIES {
            if !index.push(MediaTime::new(entry as i64, 1_000).unwrap()) {
                break;
            }
        }
        assert_eq!(index.len(), MAX_KEYFRAME_ENTRIES);
        assert!(!index.is_complete());
        assert!(index.estimated_bytes() <= MAX_KEYFRAME_ENTRIES * std::mem::size_of::<MediaTime>());
    }
}
