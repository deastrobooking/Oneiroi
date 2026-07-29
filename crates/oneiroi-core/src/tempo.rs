//! Deterministic internal tempo clock and launch-boundary calculation.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Quantization {
    Immediate,
    Beat,
    Bar,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempoClock {
    bpm: f64,
    beats_per_bar: u32,
    anchor_seconds: f64,
    anchor_beat: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TapTempo {
    taps: [f64; 5],
    count: usize,
}

impl TapTempo {
    /// Records a monotonic timestamp and returns a stable BPM after two taps.
    pub fn tap(&mut self, seconds: f64) -> Option<f64> {
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        if self.count > 0 {
            let gap = seconds - self.taps[self.count.min(self.taps.len()) - 1];
            if !(0.15..=3.0).contains(&gap) {
                self.reset();
            }
        }
        if self.count < self.taps.len() {
            self.taps[self.count] = seconds;
            self.count += 1;
        } else {
            self.taps.copy_within(1.., 0);
            self.taps[self.taps.len() - 1] = seconds;
        }
        if self.count < 2 {
            return None;
        }
        let first = self.taps[0];
        let last = self.taps[self.count - 1];
        let average_interval = (last - first) / (self.count - 1) as f64;
        (average_interval > 0.0).then(|| sanitize_bpm(60.0 / average_interval))
    }

    pub fn reset(&mut self) {
        self.count = 0;
    }
}

impl Default for TempoClock {
    fn default() -> Self {
        Self {
            bpm: 120.0,
            beats_per_bar: 4,
            anchor_seconds: 0.0,
            anchor_beat: 0.0,
        }
    }
}

impl TempoClock {
    pub fn new(bpm: f64, beats_per_bar: u32) -> Self {
        Self {
            bpm: sanitize_bpm(bpm),
            beats_per_bar: beats_per_bar.max(1),
            ..Self::default()
        }
    }

    pub fn bpm(self) -> f64 {
        self.bpm
    }

    pub fn beats_per_bar(self) -> u32 {
        self.beats_per_bar
    }

    /// Change tempo without moving the musical position at `now_seconds`.
    pub fn set_bpm(&mut self, bpm: f64, now_seconds: f64) {
        self.anchor_beat = self.beat_at(now_seconds);
        self.anchor_seconds = now_seconds.max(0.0);
        self.bpm = sanitize_bpm(bpm);
    }

    pub fn set_beats_per_bar(&mut self, beats: u32) {
        self.beats_per_bar = beats.max(1);
    }

    pub fn beat_at(self, seconds: f64) -> f64 {
        self.anchor_beat
            + (seconds.max(self.anchor_seconds) - self.anchor_seconds) * self.bpm / 60.0
    }

    pub fn beat_phase(self, seconds: f64) -> f64 {
        self.beat_at(seconds).rem_euclid(1.0)
    }

    pub fn bar_phase(self, seconds: f64) -> f64 {
        (self.beat_at(seconds) / f64::from(self.beats_per_bar)).rem_euclid(1.0)
    }

    pub fn launch_beat(self, quantization: Quantization, seconds: f64) -> f64 {
        let beat = self.beat_at(seconds);
        match quantization {
            Quantization::Immediate => beat,
            Quantization::Beat => next_boundary(beat, 1.0),
            Quantization::Bar => next_boundary(beat, f64::from(self.beats_per_bar)),
        }
    }
}

fn sanitize_bpm(bpm: f64) -> f64 {
    if bpm.is_finite() {
        bpm.clamp(20.0, 400.0)
    } else {
        120.0
    }
}

fn next_boundary(position: f64, quantum: f64) -> f64 {
    let boundary = (position / quantum).ceil() * quantum;
    if (boundary - position).abs() < 1e-9 {
        boundary + quantum
    } else {
        boundary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_beat_and_bar_phase() {
        let clock = TempoClock::new(120.0, 4);
        assert_eq!(clock.beat_at(1.0), 2.0);
        assert_eq!(clock.beat_phase(1.25), 0.5);
        assert_eq!(clock.bar_phase(1.0), 0.5);
    }

    #[test]
    fn schedules_strictly_next_quantized_boundary() {
        let clock = TempoClock::new(120.0, 4);
        assert_eq!(clock.launch_beat(Quantization::Beat, 0.1), 1.0);
        assert_eq!(clock.launch_beat(Quantization::Beat, 0.5), 2.0);
        assert_eq!(clock.launch_beat(Quantization::Bar, 0.5), 4.0);
    }

    #[test]
    fn tempo_change_preserves_musical_position() {
        let mut clock = TempoClock::new(120.0, 4);
        clock.set_bpm(60.0, 1.0);
        assert_eq!(clock.beat_at(1.0), 2.0);
        assert_eq!(clock.beat_at(2.0), 3.0);
    }

    #[test]
    fn tap_tempo_averages_recent_valid_intervals() {
        let mut taps = TapTempo::default();
        assert_eq!(taps.tap(10.0), None);
        assert_eq!(taps.tap(10.5), Some(120.0));
        assert_eq!(taps.tap(11.0), Some(120.0));
        assert_eq!(taps.tap(11.5), Some(120.0));
    }

    #[test]
    fn tap_tempo_resets_after_a_long_pause() {
        let mut taps = TapTempo::default();
        taps.tap(1.0);
        taps.tap(1.5);
        assert_eq!(taps.tap(5.0), None);
        assert_eq!(taps.tap(5.75), Some(80.0));
    }
}
