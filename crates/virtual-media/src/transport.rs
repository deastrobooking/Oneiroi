//! Render-thread transport state with explicit loop and one-shot boundaries.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndMode {
    Loop,
    OneShot,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransportEvent {
    None,
    Loop { overshoot: f64 },
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckTransport {
    pub playing: bool,
    pub frozen: bool,
    pub end_mode: EndMode,
    pub speed: f32,
    pub position: f64,
    pub duration: Option<f64>,
    pub in_point: f64,
}

impl Default for DeckTransport {
    fn default() -> Self {
        Self {
            playing: true,
            frozen: false,
            end_mode: EndMode::Loop,
            speed: 1.0,
            position: 0.0,
            duration: None,
            in_point: 0.0,
        }
    }
}

impl DeckTransport {
    pub fn reset(&mut self, duration: Option<f64>) {
        self.reset_range(0.0, duration);
    }

    pub fn reset_range(&mut self, in_point: f64, duration: Option<f64>) {
        let in_point = in_point.max(0.0);
        *self = Self {
            duration,
            in_point,
            position: in_point,
            ..Self::default()
        };
    }

    pub fn restart(&mut self) {
        self.position = self.in_point;
        self.playing = true;
    }

    pub fn seek_normalized(&mut self, normalized: f32) {
        let Some(duration) = self.duration else {
            return;
        };
        self.position = self.in_point
            + (duration - self.in_point).max(0.0) * f64::from(normalized.clamp(0.0, 1.0));
    }

    pub fn advance(&mut self, delta_seconds: f64) -> TransportEvent {
        if !self.playing || self.frozen || delta_seconds <= 0.0 {
            return TransportEvent::None;
        }
        self.speed = self.speed.clamp(0.25, 4.0);
        self.position += delta_seconds * f64::from(self.speed);
        let Some(duration) = self.duration.filter(|duration| *duration > 0.0) else {
            return TransportEvent::None;
        };
        if self.position < duration {
            return TransportEvent::None;
        }
        match self.end_mode {
            EndMode::Loop => {
                let range = (duration - self.in_point).max(f64::EPSILON);
                let overshoot = (self.position - self.in_point).rem_euclid(range);
                self.position = self.in_point + overshoot;
                TransportEvent::Loop { overshoot }
            }
            EndMode::OneShot => {
                self.position = duration;
                self.playing = false;
                TransportEvent::Ended
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_and_freeze_hold_position() {
        let mut transport = DeckTransport {
            playing: false,
            ..DeckTransport::default()
        };
        transport.advance(1.0);
        assert_eq!(transport.position, 0.0);
        transport.playing = true;
        transport.frozen = true;
        transport.advance(1.0);
        assert_eq!(transport.position, 0.0);
    }

    #[test]
    fn loop_preserves_fractional_overshoot() {
        let mut transport = DeckTransport {
            duration: Some(2.0),
            position: 1.9,
            ..DeckTransport::default()
        };
        let TransportEvent::Loop { overshoot } = transport.advance(0.25) else {
            panic!("expected loop");
        };
        assert!((overshoot - 0.15).abs() < 1e-9);
        assert!((transport.position - 0.15).abs() < 1e-9);
    }

    #[test]
    fn one_shot_stops_at_end() {
        let mut transport = DeckTransport {
            duration: Some(2.0),
            position: 1.9,
            end_mode: EndMode::OneShot,
            ..DeckTransport::default()
        };
        assert_eq!(transport.advance(0.25), TransportEvent::Ended);
        assert_eq!(transport.position, 2.0);
        assert!(!transport.playing);
    }

    #[test]
    fn clamps_speed_and_normalized_seek() {
        let mut transport = DeckTransport {
            duration: Some(8.0),
            speed: 99.0,
            ..DeckTransport::default()
        };
        transport.seek_normalized(0.5);
        transport.advance(0.5);
        assert_eq!(transport.speed, 4.0);
        assert_eq!(transport.position, 6.0);
    }

    #[test]
    fn trimmed_range_restarts_seeks_and_loops_from_in_point() {
        let mut transport = DeckTransport::default();
        transport.reset_range(2.0, Some(6.0));
        assert_eq!(transport.position, 2.0);
        transport.seek_normalized(0.5);
        assert_eq!(transport.position, 4.0);
        transport.position = 5.9;
        let TransportEvent::Loop { overshoot } = transport.advance(0.25) else {
            panic!("expected loop");
        };
        assert!((overshoot - 0.15).abs() < 1e-9);
        assert!((transport.position - 2.15).abs() < 1e-9);
        transport.restart();
        assert_eq!(transport.position, 2.0);
    }
}
