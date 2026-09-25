//! Link is accessed only on the application thread, never an audio callback.
use rusty_link::{AblLink, SessionState};
use std::time::Instant;
use virtual_core::ClockSource;

pub(crate) struct LinkClock {
    link: AblLink,
    state: SessionState,
    enabled: bool,
}

impl LinkClock {
    pub(crate) fn new() -> Self {
        Self {
            link: AblLink::new(120.0),
            state: SessionState::new(),
            enabled: false,
        }
    }

    pub(crate) fn set_tempo(&mut self, bpm: f64) {
        if !bpm.is_finite() {
            return;
        }
        self.link.capture_app_session_state(&mut self.state);
        self.state.set_tempo(bpm, self.link.clock_micros());
        self.link.commit_app_session_state(&self.state);
    }

    fn enable(&mut self, enabled: bool, bpm: f64) {
        if enabled != self.enabled {
            if enabled {
                self.set_tempo(bpm);
            }
            self.link.enable(enabled);
            self.enabled = enabled;
        }
    }
}

impl super::State {
    pub(crate) fn poll_link(&mut self) {
        let enabled = self.ui.midi_clock_source == ClockSource::AbletonLink;
        self.link.enable(enabled, self.ui.bpm);
        if !enabled {
            self.ui.link_peers = 0;
            return;
        }
        self.link
            .link
            .capture_app_session_state(&mut self.link.state);
        // Pair Link's monotonic time with the application's time origin.
        let time = self.link.link.clock_micros();
        let elapsed = Instant::now()
            .saturating_duration_since(self.performance_started)
            .as_secs_f64();
        let bpm = self.link.state.tempo();
        let beat = self
            .link
            .state
            .beat_at_time(time, f64::from(self.tempo.beats_per_bar()));
        self.ui.link_peers = self.link.link.num_peers();
        // The application's supported tempo range is explicit: never silently
        // clamp a peer's tempo and then claim to be synchronized.
        if !bpm.is_finite() || !(20.0..=400.0).contains(&bpm) || !beat.is_finite() {
            self.midi_clock_status = "Link tempo outside supported 20–400 BPM".to_owned();
            return;
        }
        if (self.ui.bpm - bpm).abs() > 0.000001 {
            self.record_show_operation(
                virtual_session::CommandOrigin::Remote("ableton-link".to_owned()),
                Instant::now(),
                virtual_session::CommandOperation::SetTempo { bpm },
            );
            self.ui.bpm = bpm;
            if let Some(sender) = &self.midi_clock_sender {
                sender.set_bpm(bpm);
            }
            self.publish_osc_value("/vjx/tempo", bpm as f32);
        }
        self.tempo.set_bpm(bpm, elapsed);
        self.tempo.anchor_beat(beat, elapsed);
        self.midi_clock_status = "Link tempo and phase synchronized".to_owned();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires local-network multicast discovery"]
    fn two_peers_discover_and_share_tempo() {
        let mut a = LinkClock::new();
        let mut b = LinkClock::new();
        a.enable(true, 120.0);
        b.enable(true, 120.0);
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while a.link.num_peers() == 0 || b.link.num_peers() == 0 {
            assert!(Instant::now() < deadline, "Link peers did not discover each other");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        a.set_tempo(133.0);
        loop {
            b.link.capture_app_session_state(&mut b.state);
            if (b.state.tempo() - 133.0).abs() < 1e-6 { break; }
            assert!(Instant::now() < deadline, "Peer tempo did not synchronize");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        a.enable(false, 133.0);
        b.enable(false, 133.0);
    }

    #[test]
    fn local_session_tempo_roundtrip_and_clock_mapping() {
        let mut clock = LinkClock::new();
        assert!(!clock.link.is_enabled());
        clock.set_tempo(137.0);
        clock.link.capture_app_session_state(&mut clock.state);
        assert!((clock.state.tempo() - 137.0).abs() < 1e-6);
        let time = clock.link.clock_micros();
        let beat = clock.state.beat_at_time(time, 4.0);
        let later = clock.state.beat_at_time(time + 1_000_000, 4.0);
        assert!((later - beat - 137.0 / 60.0).abs() < 1e-5);
        clock.set_tempo(f64::NAN);
        clock.link.capture_app_session_state(&mut clock.state);
        assert!((clock.state.tempo() - 137.0).abs() < 1e-6);
    }

    #[test]
    fn link_beat_maps_to_existing_quantized_launch_clock() {
        let mut link = LinkClock::new();
        link.set_tempo(120.0);
        link.link.capture_app_session_state(&mut link.state);
        let time = link.link.clock_micros();
        let beat = link.state.beat_at_time(time, 4.0);
        let mut tempo = virtual_core::TempoClock::new(120.0, 4);
        tempo.anchor_beat(beat, 10.0);
        assert!((tempo.beat_at(10.5) - link.state.beat_at_time(time + 500_000, 4.0)).abs() < 1e-5);
        let target = tempo.launch_beat(virtual_core::Quantization::Bar, 10.0);
        assert!(target > beat && target <= beat + 4.0);
        assert!(target.rem_euclid(4.0).abs() < 1e-6);
    }
}
