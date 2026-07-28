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
        }
    }
}

impl DeckTransport {
    pub fn reset(&mut self, duration: Option<f64>) {
        *self = Self {
            duration,
            ..Self::default()
        };
    }

    pub fn restart(&mut self) {
        self.position = 0.0;
        self.playing = true;
    }

    pub fn seek_normalized(&mut self, normalized: f32) {
        let Some(duration) = self.duration else {
            return;
        };
        self.position = duration * f64::from(normalized.clamp(0.0, 1.0));
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
                let overshoot = self.position % duration;
                self.position = overshoot;
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
}
