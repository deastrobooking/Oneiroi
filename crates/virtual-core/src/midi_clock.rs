//! MIDI beat-clock synchronisation: following an external 24 PPQN clock, and
//! generating one for downstream gear.
//!
//! Both halves are pure state machines driven by seconds passed in from the
//! caller. Following is fed by the platform's MIDI timestamps and generating
//! is driven by a sender thread's own monotonic clock, so neither depends on
//! the render loop's frame rate — a 60 Hz frame would quantise pulses to 16 ms
//! and every synced device downstream would hear it as swing.

use serde::{Deserialize, Serialize};

/// MIDI beat clock runs at 24 pulses per quarter note. This is fixed by the
/// specification; every device on the wire assumes it.
pub const PULSES_PER_QUARTER_NOTE: u64 = 24;

/// Song Position Pointer counts sixteenth notes, which is six clock pulses.
const PULSES_PER_SONG_POSITION: u64 = 6;

/// Pulse intervals averaged for the tempo estimate — one quarter note.
const TEMPO_WINDOW: usize = 24;

/// No pulse for this long means the source stopped clocking us.
const LOCK_TIMEOUT_SECONDS: f64 = 0.5;

/// Smallest tempo change worth reporting upward, in BPM. Clock jitter moves
/// the estimate constantly; reporting every wobble would spam the show log.
const TEMPO_REPORT_EPSILON: f64 = 0.05;

/// Where the transport takes its tempo from.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClockSource {
    /// The internal tempo clock, driven by the operator and tap tempo.
    #[default]
    Internal,
    /// An external 24 PPQN MIDI clock arriving on a connected input.
    MidiInput,
}

/// A MIDI System Real-Time or Song Position message.
///
/// These carry no channel and never participate in learn/mapping, so they stay
/// out of [`crate::MidiMessage`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MidiRealtime {
    /// 0xF8 — one 24 PPQN tick.
    Clock,
    /// 0xFA — rewind to the top and run.
    Start,
    /// 0xFB — run from the current position.
    Continue,
    /// 0xFC — halt; clock pulses usually keep arriving.
    Stop,
    /// 0xF2 — position in sixteenth notes from the top of the song.
    SongPosition(u16),
}

/// What one real-time message changed for the follower's consumer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MidiClockUpdate {
    /// A tempo estimate meaningfully different from the last reported one.
    pub bpm: Option<f64>,
    /// A musical position to re-anchor the local tempo clock to.
    pub beat: Option<f64>,
    /// The source's transport began or resumed.
    pub started: bool,
    /// The source's transport halted.
    pub stopped: bool,
}

impl MidiClockUpdate {
    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// Tracks an incoming MIDI beat clock and estimates its tempo.
///
/// Pulse intervals are averaged over a full quarter note. A single interval is
/// far too noisy to drive a tempo — USB MIDI jitter alone is a few
/// milliseconds, which at 24 PPQN reads as tens of BPM.
#[derive(Clone, Debug)]
pub struct MidiClockFollower {
    /// The device currently trusted as the clock master, if any.
    source: Option<String>,
    intervals: [f64; TEMPO_WINDOW],
    filled: usize,
    next: usize,
    last_pulse_seconds: Option<f64>,
    /// Pulses since the top of the song, advanced only while running.
    pulse: u64,
    running: bool,
    bpm: Option<f64>,
    reported_bpm: Option<f64>,
    jitter_seconds: f64,
    pulses: u64,
    /// Times the interval window was thrown away after a dropout or a jump.
    resyncs: u64,
}

impl Default for MidiClockFollower {
    fn default() -> Self {
        Self {
            source: None,
            intervals: [0.0; TEMPO_WINDOW],
            filled: 0,
            next: 0,
            last_pulse_seconds: None,
            pulse: 0,
            running: false,
            bpm: None,
            reported_bpm: None,
            jitter_seconds: 0.0,
            pulses: 0,
            resyncs: 0,
        }
    }
}

impl MidiClockFollower {
    /// Feed one real-time message observed from `device` at `seconds`.
    ///
    /// `seconds` must come from the same timebase for the whole session; the
    /// platform MIDI timestamp is the right source because it is captured in
    /// the driver callback rather than whenever the render thread polls.
    pub fn apply(&mut self, device: &str, message: MidiRealtime, seconds: f64) -> MidiClockUpdate {
        if !self.accepts(device, seconds) {
            return MidiClockUpdate::default();
        }
        if self.source.as_deref() != Some(device) {
            self.adopt(device);
        }
        match message {
            MidiRealtime::Clock => self.pulse(seconds),
            MidiRealtime::Start => {
                self.pulse = 0;
                self.running = true;
                MidiClockUpdate {
                    beat: Some(0.0),
                    started: true,
                    ..MidiClockUpdate::default()
                }
            }
            MidiRealtime::Continue => {
                self.running = true;
                MidiClockUpdate {
                    beat: Some(self.beat()),
                    started: true,
                    ..MidiClockUpdate::default()
                }
            }
            MidiRealtime::Stop => {
                self.running = false;
                MidiClockUpdate {
                    stopped: true,
                    ..MidiClockUpdate::default()
                }
            }
            MidiRealtime::SongPosition(position) => {
                self.pulse = u64::from(position) * PULSES_PER_SONG_POSITION;
                MidiClockUpdate {
                    beat: Some(self.beat()),
                    ..MidiClockUpdate::default()
                }
            }
        }
    }

    /// Whether a live tempo is currently being followed.
    pub fn is_locked(&self, seconds: f64) -> bool {
        self.bpm.is_some()
            && self
                .last_pulse_seconds
                .is_some_and(|last| seconds - last < LOCK_TIMEOUT_SECONDS)
    }

    /// Whether the source's transport is running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn bpm(&self) -> Option<f64> {
        self.bpm
    }

    /// Musical position in quarter notes since the top of the song.
    pub fn beat(&self) -> f64 {
        self.pulse as f64 / PULSES_PER_QUARTER_NOTE as f64
    }

    /// The device currently trusted as clock master.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    pub fn pulses(&self) -> u64 {
        self.pulses
    }

    pub fn resyncs(&self) -> u64 {
        self.resyncs
    }

    /// Worst deviation from the mean pulse interval in the current window.
    pub fn jitter_micros(&self) -> u64 {
        (self.jitter_seconds * 1_000_000.0).round().max(0.0) as u64
    }

    /// Forget the source and the tempo estimate, keeping nothing but counters.
    pub fn reset(&mut self) {
        let pulses = self.pulses;
        let resyncs = self.resyncs;
        *self = Self::default();
        self.pulses = pulses;
        self.resyncs = resyncs;
    }

    /// A device may drive the clock if it is the pinned source, or if no
    /// source is locked right now. Two devices clocking at once would fight;
    /// first one to arrive after a dropout wins until it goes quiet.
    fn accepts(&self, device: &str, seconds: f64) -> bool {
        match self.source.as_deref() {
            Some(current) => current == device || !self.is_locked(seconds),
            None => true,
        }
    }

    fn adopt(&mut self, device: &str) {
        self.reset();
        self.source = Some(device.to_owned());
    }

    fn pulse(&mut self, seconds: f64) -> MidiClockUpdate {
        self.pulses = self.pulses.saturating_add(1);
        let previous = self.last_pulse_seconds.replace(seconds);
        match previous {
            Some(previous) if plausible_interval(seconds - previous) => {
                self.push_interval(seconds - previous);
            }
            Some(_) => {
                self.clear_window();
                self.resyncs = self.resyncs.saturating_add(1);
            }
            None => {}
        }
        let mut update = MidiClockUpdate::default();
        if let Some(bpm) = self.estimate()
            && self
                .reported_bpm
                .is_none_or(|reported| (reported - bpm).abs() >= TEMPO_REPORT_EPSILON)
        {
            self.reported_bpm = Some(bpm);
            update.bpm = Some(bpm);
        }
        if self.running {
            self.pulse += 1;
            // Re-anchoring on every pulse would chase jitter. A quarter-note
            // boundary is frequent enough that drift never accumulates and
            // rare enough that the correction is inaudible in modulation.
            if self.pulse.is_multiple_of(PULSES_PER_QUARTER_NOTE) {
                update.beat = Some(self.beat());
            }
        }
        update
    }

    fn push_interval(&mut self, interval: f64) {
        self.intervals[self.next] = interval;
        self.next = (self.next + 1) % TEMPO_WINDOW;
        self.filled = (self.filled + 1).min(TEMPO_WINDOW);
    }

    fn clear_window(&mut self) {
        self.filled = 0;
        self.next = 0;
        self.bpm = None;
        self.reported_bpm = None;
        self.jitter_seconds = 0.0;
    }

    /// Mean of the window once it holds at least a sixteenth note of history.
    fn estimate(&mut self) -> Option<f64> {
        if self.filled < PULSES_PER_SONG_POSITION as usize {
            return None;
        }
        let window = &self.intervals[..self.filled];
        let mean = window.iter().sum::<f64>() / self.filled as f64;
        if mean <= 0.0 {
            return None;
        }
        self.jitter_seconds = window
            .iter()
            .map(|interval| (interval - mean).abs())
            .fold(0.0, f64::max);
        let bpm = sanitize_bpm(60.0 / (mean * PULSES_PER_QUARTER_NOTE as f64))?;
        self.bpm = Some(bpm);
        Some(bpm)
    }
}

/// Emits a 24 PPQN clock for downstream gear.
///
/// The generator owns the pulse schedule; the caller only tells it what time
/// it is and sends the pulses it hands back. Keeping the schedule here means a
/// tempo change never shifts an already-scheduled pulse.
#[derive(Clone, Copy, Debug)]
pub struct MidiClockGenerator {
    bpm: f64,
    running: bool,
    /// Time of the pulse the emitted count is measured from.
    origin_seconds: f64,
    emitted: u64,
    resyncs: u64,
}

/// Pulses more than this far behind are dropped rather than emitted in a
/// burst. A stalled sender thread must not machine-gun a drum machine.
const MAX_CATCH_UP_SECONDS: f64 = 0.25;

impl Default for MidiClockGenerator {
    fn default() -> Self {
        Self {
            bpm: 120.0,
            running: false,
            origin_seconds: 0.0,
            emitted: 0,
            resyncs: 0,
        }
    }
}

impl MidiClockGenerator {
    pub fn new(bpm: f64) -> Self {
        Self {
            bpm: sanitize_bpm(bpm).unwrap_or(120.0),
            ..Self::default()
        }
    }

    pub fn bpm(self) -> f64 {
        self.bpm
    }

    pub fn is_running(self) -> bool {
        self.running
    }

    pub fn resyncs(self) -> u64 {
        self.resyncs
    }

    /// Begin pulsing, with the first pulse due immediately at `seconds`.
    pub fn start(&mut self, seconds: f64) {
        self.origin_seconds = seconds;
        self.emitted = 0;
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Change tempo without moving the pulse that is already scheduled.
    ///
    /// The new period is measured from the most recent emitted pulse, so a
    /// tempo nudge never emits two pulses back to back or swallows one.
    pub fn set_bpm(&mut self, bpm: f64, seconds: f64) {
        let Some(bpm) = sanitize_bpm(bpm) else {
            return;
        };
        // With nothing emitted yet the next pulse is the origin itself, and
        // the origin does not move with tempo. Only a train already in flight
        // needs rebasing onto its last emitted pulse.
        if self.running && self.emitted > 0 {
            self.origin_seconds = self.last_pulse_seconds().min(seconds);
            self.emitted = 1;
        }
        self.bpm = bpm;
    }

    /// Seconds between pulses at the current tempo.
    pub fn pulse_period(self) -> f64 {
        60.0 / (self.bpm * PULSES_PER_QUARTER_NOTE as f64)
    }

    /// When the next pulse is due, or `None` while stopped.
    pub fn next_pulse_seconds(self) -> Option<f64> {
        self.running
            .then(|| self.origin_seconds + self.emitted as f64 * self.pulse_period())
    }

    /// How many pulses are due at `seconds`, advancing the schedule past them.
    pub fn pulses_due(&mut self, seconds: f64) -> u32 {
        if !self.running {
            return 0;
        }
        let period = self.pulse_period();
        let elapsed = seconds - self.origin_seconds;
        if elapsed < 0.0 {
            return 0;
        }
        let reached = (elapsed / period).floor() as u64 + 1;
        if reached <= self.emitted {
            return 0;
        }
        let mut due = reached - self.emitted;
        if (due as f64) * period > MAX_CATCH_UP_SECONDS {
            // The sender lost the CPU. Re-base rather than flood the port;
            // a listener recovers from a gap, not from a burst.
            self.origin_seconds = seconds;
            self.emitted = 0;
            self.resyncs = self.resyncs.saturating_add(1);
            due = 1;
        }
        self.emitted += due;
        due.min(u64::from(u32::MAX)) as u32
    }

    fn last_pulse_seconds(self) -> f64 {
        self.origin_seconds + self.emitted.saturating_sub(1) as f64 * self.pulse_period()
    }
}

/// A pulse interval only counts if it could belong to a 20–400 BPM clock.
fn plausible_interval(interval: f64) -> bool {
    const FASTEST: f64 = 60.0 / (400.0 * PULSES_PER_QUARTER_NOTE as f64);
    const SLOWEST: f64 = 60.0 / (20.0 * PULSES_PER_QUARTER_NOTE as f64);
    interval.is_finite() && (FASTEST..=SLOWEST).contains(&interval)
}

fn sanitize_bpm(bpm: f64) -> Option<f64> {
    (bpm.is_finite() && (20.0..=400.0).contains(&bpm)).then_some(bpm)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `count` pulses spaced for `bpm`, returning the last update.
    fn clock_for(
        follower: &mut MidiClockFollower,
        bpm: f64,
        count: usize,
        start_seconds: f64,
    ) -> (MidiClockUpdate, f64) {
        let period = 60.0 / (bpm * PULSES_PER_QUARTER_NOTE as f64);
        let mut seconds = start_seconds;
        let mut update = MidiClockUpdate::default();
        for _ in 0..count {
            update = follower.apply("clock-source", MidiRealtime::Clock, seconds);
            seconds += period;
        }
        (update, seconds)
    }

    #[test]
    fn estimates_tempo_from_a_steady_pulse_train() {
        let mut follower = MidiClockFollower::default();
        clock_for(&mut follower, 128.0, 48, 10.0);

        let bpm = follower.bpm().expect("tempo after a full window");
        assert!((bpm - 128.0).abs() < 0.01, "estimated {bpm}");
        assert!(follower.is_locked(10.5));
        assert_eq!(follower.source(), Some("clock-source"));
    }

    #[test]
    fn tempo_is_reported_once_per_meaningful_change() {
        let mut follower = MidiClockFollower::default();
        let (_, seconds) = clock_for(&mut follower, 120.0, 48, 0.0);
        // A steady train stops reporting once the estimate settles.
        let (steady, seconds) = clock_for(&mut follower, 120.0, 24, seconds);
        assert_eq!(steady.bpm, None);

        // A real tempo move is reported while the estimate slides toward it.
        // `clock_for` leaves `seconds` on the next due pulse, so the first
        // faster interval starts one 120 BPM period earlier.
        let start = seconds - 60.0 / (120.0 * PULSES_PER_QUARTER_NOTE as f64);
        let period = 60.0 / (140.0 * PULSES_PER_QUARTER_NOTE as f64);
        let mut reports = 0;
        for pulse in 1..=48 {
            if follower
                .apply(
                    "clock-source",
                    MidiRealtime::Clock,
                    start + f64::from(pulse) * period,
                )
                .bpm
                .is_some()
            {
                reports += 1;
            }
        }

        assert!(reports > 0, "a tempo change has to reach the consumer");
        assert!(follower.bpm().is_some_and(|bpm| (bpm - 140.0).abs() < 0.01));
    }

    #[test]
    fn start_rewinds_and_continue_keeps_position() {
        let mut follower = MidiClockFollower::default();
        follower.apply("clock-source", MidiRealtime::Start, 0.0);
        let (_, seconds) = clock_for(&mut follower, 120.0, 48, 0.0);
        assert_eq!(follower.beat(), 2.0);

        let stopped = follower.apply("clock-source", MidiRealtime::Stop, seconds);
        assert!(stopped.stopped);
        assert!(!follower.is_running());

        // Pulses while stopped keep the tempo estimate but not the position.
        clock_for(&mut follower, 120.0, 24, seconds);
        assert_eq!(follower.beat(), 2.0);

        let resumed = follower.apply("clock-source", MidiRealtime::Continue, seconds + 1.0);
        assert!(resumed.started);
        assert_eq!(resumed.beat, Some(2.0));

        let restarted = follower.apply("clock-source", MidiRealtime::Start, seconds + 2.0);
        assert_eq!(restarted.beat, Some(0.0));
        assert_eq!(follower.beat(), 0.0);
    }

    #[test]
    fn song_position_moves_to_the_addressed_sixteenth() {
        let mut follower = MidiClockFollower::default();

        // Sixteenth 16 is bar 2 beat 1 in 4/4.
        let update = follower.apply("clock-source", MidiRealtime::SongPosition(16), 0.0);

        assert_eq!(update.beat, Some(4.0));
        assert_eq!(follower.beat(), 4.0);
    }

    #[test]
    fn quarter_note_boundaries_publish_an_anchor() {
        let mut follower = MidiClockFollower::default();
        follower.apply("clock-source", MidiRealtime::Start, 0.0);
        let period = 60.0 / (120.0 * PULSES_PER_QUARTER_NOTE as f64);
        let mut anchors = Vec::new();
        for pulse in 0..48 {
            let update = follower.apply("clock-source", MidiRealtime::Clock, pulse as f64 * period);
            if let Some(beat) = update.beat {
                anchors.push(beat);
            }
        }
        assert_eq!(anchors, vec![1.0, 2.0]);
    }

    #[test]
    fn a_dropout_unlocks_and_a_second_device_can_take_over() {
        let mut follower = MidiClockFollower::default();
        clock_for(&mut follower, 120.0, 48, 0.0);
        let locked_at = 1.0;
        assert!(follower.is_locked(locked_at));

        // A rival device is ignored while the master is live.
        follower.apply("rival", MidiRealtime::Clock, locked_at);
        assert_eq!(follower.source(), Some("clock-source"));

        // After the master goes quiet the rival becomes the source.
        let after_timeout = locked_at + LOCK_TIMEOUT_SECONDS + 0.1;
        assert!(!follower.is_locked(after_timeout));
        follower.apply("rival", MidiRealtime::Clock, after_timeout);
        assert_eq!(follower.source(), Some("rival"));
        assert_eq!(follower.bpm(), None, "the window restarts on a new source");
    }

    #[test]
    fn implausible_intervals_discard_the_window_instead_of_skewing_it() {
        let mut follower = MidiClockFollower::default();
        clock_for(&mut follower, 120.0, 48, 0.0);
        assert!(follower.bpm().is_some());

        // A ten-second gap is a dropout, not a 0.25 BPM tempo.
        follower.apply("clock-source", MidiRealtime::Clock, 20.0);
        assert_eq!(follower.bpm(), None);
        assert_eq!(follower.resyncs(), 1);
    }

    #[test]
    fn generator_emits_twenty_four_pulses_per_quarter_note() {
        let mut generator = MidiClockGenerator::new(120.0);
        generator.start(0.0);

        // One quarter note at 120 BPM is 0.5 s. The pulse at t=0 counts.
        let mut pulses = 0;
        for step in 0..=500 {
            pulses += generator.pulses_due(f64::from(step) / 1_000.0);
        }

        assert_eq!(pulses, 25, "24 pulses plus the boundary pulse at 0.5 s");
    }

    #[test]
    fn generator_is_silent_until_started_and_after_stopping() {
        let mut generator = MidiClockGenerator::new(120.0);
        assert_eq!(generator.pulses_due(1.0), 0);
        assert_eq!(generator.next_pulse_seconds(), None);

        generator.start(1.0);
        assert_eq!(generator.pulses_due(1.0), 1);
        generator.stop();
        assert_eq!(generator.pulses_due(2.0), 0);
    }

    #[test]
    fn tempo_change_does_not_double_or_swallow_the_next_pulse() {
        let mut generator = MidiClockGenerator::new(120.0);
        generator.start(0.0);
        // Consume the pulse at t=0 and the one at t=1/48.
        assert_eq!(generator.pulses_due(0.0), 1);
        let period = generator.pulse_period();
        assert_eq!(generator.pulses_due(period), 1);

        generator.set_bpm(240.0, period + 0.001);
        let next = generator
            .next_pulse_seconds()
            .expect("a running generator has a next pulse");

        assert!(next > period, "the next pulse stays in the future: {next}");
        assert!((next - (period + generator.pulse_period())).abs() < 1e-9);
        assert_eq!(generator.pulses_due(period + 0.001), 0);
    }

    #[test]
    fn a_stalled_sender_drops_missed_pulses_rather_than_bursting() {
        let mut generator = MidiClockGenerator::new(120.0);
        generator.start(0.0);
        generator.pulses_due(0.0);

        // The thread lost the CPU for two seconds: ~96 pulses' worth.
        let due = generator.pulses_due(2.0);

        assert_eq!(due, 1);
        assert_eq!(generator.resyncs(), 1);
        assert!(
            generator
                .next_pulse_seconds()
                .is_some_and(|next| (next - (2.0 + generator.pulse_period())).abs() < 1e-9)
        );
    }

    #[test]
    fn out_of_range_tempo_is_rejected_rather_than_clamped_silently() {
        let mut generator = MidiClockGenerator::new(120.0);
        generator.set_bpm(f64::NAN, 0.0);
        generator.set_bpm(10_000.0, 0.0);
        assert_eq!(generator.bpm(), 120.0);
    }
}
