//! Audio and MIDI device lifecycle, and control-update dispatch.

use std::time::{Duration, Instant};

use oneiroi_core::{ControlTarget, ControlUpdate, effect_parameter_key};
use oneiroi_io::{
    AudioInput, AudioInputSnapshot, MidiInputConnection, discover_audio_inputs,
    discover_midi_inputs,
};
use oneiroi_media::{ClipAddress, DeckId};

use super::{State, current_control_value, deck_id, set_effect_parameter};

impl State {
    pub(crate) fn refresh_audio_inputs(&mut self) {
        match discover_audio_inputs() {
            Ok(inputs) => {
                self.audio_inputs = inputs;
                if !self
                    .audio_inputs
                    .iter()
                    .any(|device| device.id == self.ui.audio_device_id)
                {
                    self.ui.audio_device_id = self
                        .audio_inputs
                        .iter()
                        .find(|device| device.is_default)
                        .or_else(|| self.audio_inputs.first())
                        .map(|device| device.id.clone())
                        .unwrap_or_default();
                }
                self.audio_status = format!("{} audio input(s) available", self.audio_inputs.len());
            }
            Err(error) => self.audio_status = format!("Audio discovery failed: {error}"),
        }
    }

    pub(crate) fn connect_audio_input(&mut self, device_id: String) {
        self.audio_input = None;
        match AudioInput::connect(&device_id, self.ui.audio_analysis) {
            Ok(input) => {
                self.ui.audio_device_id = device_id;
                self.audio_snapshot = input.snapshot();
                self.audio_input = Some(input);
                self.audio_status = "Audio input connected".to_owned();
            }
            Err(error) => self.audio_status = format!("Audio connection failed: {error}"),
        }
    }

    pub(crate) fn disconnect_audio_input(&mut self) {
        self.audio_input = None;
        self.audio_snapshot = AudioInputSnapshot::default();
        self.audio_status = "Audio input disconnected".to_owned();
    }

    pub(crate) fn refresh_midi_inputs(&mut self) {
        match discover_midi_inputs() {
            Ok(inputs) => {
                let connected_id = self
                    .midi_input
                    .as_ref()
                    .map(|input| input.device_id().to_owned());
                self.midi_inputs = inputs;
                if let Some(connected_id) = connected_id
                    && !self
                        .midi_inputs
                        .iter()
                        .any(|device| device.id == connected_id)
                {
                    self.midi_input = None;
                    self.midi_status =
                        format!("{connected_id} disconnected · waiting to reconnect");
                }
                if self.ui.midi_device_id.is_empty() {
                    self.ui.midi_device_id = self
                        .midi_inputs
                        .first()
                        .map(|device| device.id.clone())
                        .unwrap_or_default();
                }
                if self.midi_input.is_none()
                    && self.midi_reconnect
                    && self
                        .midi_inputs
                        .iter()
                        .any(|device| device.id == self.ui.midi_device_id)
                {
                    self.connect_midi_input(self.ui.midi_device_id.clone());
                } else if self.midi_input.is_none() && !self.midi_reconnect {
                    self.midi_status =
                        format!("{} MIDI input(s) available", self.midi_inputs.len());
                }
            }
            Err(error) => self.midi_status = format!("MIDI discovery failed: {error}"),
        }
        self.last_midi_refresh = Instant::now();
    }

    pub(crate) fn connect_midi_input(&mut self, device_id: String) {
        self.midi_input = None;
        match MidiInputConnection::connect(&device_id) {
            Ok(input) => {
                self.ui.midi_device_id = device_id.clone();
                self.midi_stats = input.stats();
                self.midi_input = Some(input);
                self.midi_reconnect = true;
                self.midi_status = format!("{device_id} connected");
            }
            Err(error) => {
                self.midi_reconnect = true;
                self.midi_status = format!("MIDI connection failed: {error}");
            }
        }
    }

    pub(crate) fn disconnect_midi_input(&mut self) {
        self.midi_input = None;
        self.midi_reconnect = false;
        self.midi.cancel_learn();
        self.midi_status = "MIDI input disconnected".to_owned();
    }

    pub(crate) fn poll_midi(&mut self, now: Instant) {
        if now.saturating_duration_since(self.last_midi_refresh) >= Duration::from_secs(2) {
            self.refresh_midi_inputs();
        }
        let Some(input) = &self.midi_input else {
            return;
        };
        let device = input.device_id().to_owned();
        let events: Vec<_> = input.try_iter().collect();
        self.midi_stats = input.stats();
        for event in events {
            let updates = {
                let ui = &self.ui;
                let mixer = &self.mixer;
                let transports = &self.transports;
                self.midi.ingest(&device, event.message, |target| {
                    current_control_value(ui, mixer, transports, target)
                })
            };
            for update in updates {
                self.apply_control_update(update, now);
            }
            self.midi_status = format!(
                "{device} · {:?} · {} µs",
                event.message, event.timestamp_micros
            );
        }
    }

    pub(crate) fn apply_control_update(&mut self, update: ControlUpdate, now: Instant) {
        match update.target {
            ControlTarget::Crossfader => self.ui.crossfader = update.value.clamp(0.0, 1.0),
            ControlTarget::MasterOpacity => {
                self.ui.master_opacity = update.value.clamp(0.0, 1.0);
            }
            ControlTarget::MasterBlackout => self.ui.blackout = update.value >= 0.5,
            ControlTarget::MasterFreeze => self.ui.master_freeze = update.value >= 0.5,
            ControlTarget::TapTempo => {
                if update.value >= 0.5 {
                    let elapsed = now
                        .saturating_duration_since(self.performance_started)
                        .as_secs_f64();
                    if let Some(bpm) = self.tap_tempo.tap(elapsed) {
                        self.ui.bpm = bpm;
                        self.tempo.set_bpm(bpm, elapsed);
                    }
                }
            }
            ControlTarget::DeckLevel(deck) => {
                if let Some(deck) = deck_id(deck) {
                    self.mixer.deck_mut(deck).level = update.value.clamp(0.0, 1.0);
                }
            }
            ControlTarget::DeckPlay(deck) => {
                if let Some(deck) = deck_id(deck) {
                    self.transports[deck.index()].playing = update.value >= 0.5;
                    self.last_transport_updates[deck.index()] = now;
                }
            }
            ControlTarget::DeckFreeze(deck) => {
                if let Some(deck) = deck_id(deck) {
                    self.transports[deck.index()].frozen = update.value >= 0.5;
                }
            }
            ControlTarget::DeckSpeed(deck) => {
                if let Some(deck) = deck_id(deck) {
                    self.transports[deck.index()].speed = update.value.clamp(0.25, 4.0);
                }
            }
            ControlTarget::DeckSelect(deck) => {
                if update.value >= 0.5
                    && let Some(deck) = deck_id(deck)
                {
                    self.mixer.select(deck);
                }
            }
            ControlTarget::DeckRestart(deck) => {
                if update.value >= 0.5
                    && let Some(deck) = deck_id(deck)
                {
                    self.transports[deck.index()].restart();
                    self.seek_deck(deck);
                }
            }
            ControlTarget::ClipLaunch { deck, slot } => {
                if update.value >= 0.5
                    && let Some(deck) = deck_id(deck)
                    && usize::from(slot) < oneiroi_media::CLIPS_PER_DECK
                {
                    self.queue_clip(
                        ClipAddress {
                            deck,
                            slot: usize::from(slot),
                        },
                        now,
                    );
                }
            }
            ControlTarget::SceneLaunch(slot) => {
                if update.value >= 0.5 && usize::from(slot) < oneiroi_media::CLIPS_PER_DECK {
                    for deck in DeckId::ALL {
                        self.queue_clip(
                            ClipAddress {
                                deck,
                                slot: usize::from(slot),
                            },
                            now,
                        );
                    }
                }
            }
            ControlTarget::EffectParameter {
                deck,
                effect,
                parameter: _,
            } => {
                if let Some(deck) = deck_id(deck) {
                    set_effect_parameter(&mut self.ui.effects[deck.index()], effect, update.value);
                }
            }
            ControlTarget::LfoParameter {
                deck,
                lfo,
                parameter,
            } => {
                if let Some(deck) = deck_id(deck)
                    && let Some(lfo) = self.ui.lfos[deck.index()].lanes.get_mut(usize::from(lfo))
                {
                    match parameter {
                        0 => lfo.enabled = update.value >= 0.5,
                        1 => lfo.rate_hz = update.value.clamp(0.01, 20.0),
                        2 => lfo.depth = update.value.clamp(0.0, 1.0),
                        3 => lfo.phase = update.value.rem_euclid(1.0),
                        _ => {}
                    }
                }
            }
            ControlTarget::ModRouteParameter {
                deck,
                route,
                parameter,
            } => {
                if let Some(deck) = deck_id(deck)
                    && let Some(route) = self.ui.lfos[deck.index()]
                        .routes
                        .get_mut(usize::from(route))
                {
                    match parameter {
                        0 => route.enabled = update.value >= 0.5,
                        1 => route.amount = update.value.clamp(-1.0, 1.0),
                        _ => {}
                    }
                }
            }
            ControlTarget::MasterEffectParameter {
                slot,
                parameter_key,
            } => {
                if let Some(effect) = self.ui.master_effects.slots.get_mut(usize::from(slot))
                    && let Some(parameter) = effect.parameters.iter_mut().find(|parameter| {
                        effect_parameter_key(&effect.package_id, &parameter.id) == parameter_key
                    })
                {
                    parameter.value = update.value;
                }
            }
        }
    }
}
