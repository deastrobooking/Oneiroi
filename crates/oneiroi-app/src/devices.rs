//! Audio, MIDI and OSC lifecycle, and control-update dispatch.

use std::time::{Duration, Instant, SystemTime};

use oneiroi_core::{ControlTarget, ControlUpdate, effect_parameter_key};
use oneiroi_io::{
    AudioInput, AudioInputSnapshot, MidiInputConnection, discover_audio_inputs,
    discover_midi_inputs,
};
use oneiroi_media::{ClipAddress, DeckId};
use oneiroi_session::{CommandOperation, CommandOrigin};

use super::{State, current_control_value, deck_id, set_effect_parameter};
use crate::osc::{OscAction, OscInput, OscOutput};

const MAX_SCHEDULED_OSC: usize = 1_024;
const MAX_OSC_SCHEDULE_AHEAD: Duration = Duration::from_secs(24 * 60 * 60);

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
                self.midi_inputs = inputs;
                // Drop connections whose hardware vanished; they stay in the
                // wanted set so they reconnect the moment they reappear.
                let available: std::collections::BTreeSet<String> = self
                    .midi_inputs
                    .iter()
                    .map(|device| device.id.clone())
                    .collect();
                let before = self.midi_connections.len();
                self.midi_connections
                    .retain(|connection| available.contains(connection.device_id()));
                if self.midi_connections.len() < before {
                    self.midi_status = "MIDI device unplugged · waiting to reconnect".to_owned();
                }
                for wanted in self.midi_wanted.clone() {
                    let connected = self
                        .midi_connections
                        .iter()
                        .any(|connection| connection.device_id() == wanted);
                    if !connected && available.contains(&wanted) {
                        self.connect_midi_input(wanted);
                    }
                }
                if self.ui.midi_device_id.is_empty() {
                    self.ui.midi_device_id = self
                        .midi_inputs
                        .first()
                        .map(|device| device.id.clone())
                        .unwrap_or_default();
                }
            }
            Err(error) => self.midi_status = format!("MIDI discovery failed: {error}"),
        }
        self.refresh_midi_device_stats();
        self.last_midi_refresh = Instant::now();
    }

    /// Connect one more device; existing connections stay up.
    pub(crate) fn connect_midi_input(&mut self, device_id: String) {
        if self
            .midi_connections
            .iter()
            .any(|connection| connection.device_id() == device_id)
        {
            self.midi_wanted.insert(device_id);
            return;
        }
        match MidiInputConnection::connect(&device_id) {
            Ok(input) => {
                self.midi_status = format!(
                    "{device_id} connected · {} device(s) live",
                    self.midi_connections.len() + 1
                );
                self.midi_connections.push(input);
                self.midi_wanted.insert(device_id);
            }
            Err(error) => {
                self.midi_status = format!("{device_id} failed: {error}");
                self.midi_wanted.remove(&device_id);
            }
        }
        self.refresh_midi_device_stats();
    }

    pub(crate) fn disconnect_midi_input(&mut self, device_id: &str) {
        self.midi_connections
            .retain(|connection| connection.device_id() != device_id);
        self.midi_wanted.remove(device_id);
        if self.midi_connections.is_empty() {
            self.midi.cancel_learn();
        }
        self.midi_status = format!("{device_id} disconnected");
        self.refresh_midi_device_stats();
    }

    /// Snapshot per-device state for the manager window.
    pub(crate) fn refresh_midi_device_stats(&mut self) {
        self.midi_device_stats = self
            .midi_inputs
            .iter()
            .map(|device| {
                let connection = self
                    .midi_connections
                    .iter()
                    .find(|connection| connection.device_id() == device.id);
                crate::ui::MidiDeviceStatus {
                    id: device.id.clone(),
                    label: device.label.clone(),
                    connected: connection.is_some(),
                    wanted: self.midi_wanted.contains(&device.id),
                    stats: connection.map(|c| c.stats()).unwrap_or_default(),
                }
            })
            .collect();
    }

    pub(crate) fn poll_midi(&mut self, now: Instant) {
        if now.saturating_duration_since(self.last_midi_refresh) >= Duration::from_secs(2) {
            self.refresh_midi_inputs();
        }
        // Collect first: ingest needs &mut self.midi while connections are
        // borrowed, so the events leave the borrow before dispatch.
        let batches: Vec<(String, Vec<_>)> = self
            .midi_connections
            .iter()
            .map(|input| (input.device_id().to_owned(), input.try_iter().collect()))
            .collect();
        for (device, events) in batches {
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
                    self.dispatch_control_update(update, CommandOrigin::Midi(device.clone()), now);
                }
                self.midi_status = format!(
                    "{device} · {:?} · {} µs",
                    event.message, event.timestamp_micros
                );
            }
        }
        self.refresh_midi_device_stats();
    }

    pub(crate) fn connect_osc_input(&mut self) {
        self.osc_input = None;
        let address = self.ui.osc_bind_address.trim();
        match OscInput::bind(address) {
            Ok(input) => {
                self.ui.osc_bind_address = input.local_address().to_owned();
                self.osc_stats = input.stats();
                self.osc_status = format!("Listening on UDP {}", input.local_address());
                self.osc_input = Some(input);
            }
            Err(error) => self.osc_status = format!("OSC bind failed: {error:#}"),
        }
    }

    pub(crate) fn disconnect_osc_input(&mut self) {
        self.osc_input = None;
        self.osc_pending.clear();
        self.osc_status = "OSC input disconnected".to_owned();
    }

    pub(crate) fn connect_osc_output(&mut self) {
        self.osc_output = None;
        let destination = self.ui.osc_feedback_address.trim();
        match OscOutput::connect(destination) {
            Ok(output) => {
                self.ui.osc_feedback_address = output.destination().to_owned();
                self.osc_output_stats = output.stats();
                self.osc_output_status = format!("Sending to UDP {}", output.destination());
                self.osc_output = Some(output);
                self.publish_osc_snapshot();
            }
            Err(error) => {
                self.osc_output_status = format!("OSC feedback connection failed: {error:#}");
            }
        }
    }

    pub(crate) fn disconnect_osc_output(&mut self) {
        self.osc_output = None;
        self.osc_output_status = "OSC feedback disconnected".to_owned();
    }

    pub(crate) fn poll_osc(&mut self, now: Instant) {
        if let Some(output) = &self.osc_output {
            self.osc_output_stats = output.stats();
        }
        let events = self.osc_input.as_ref().map_or_else(Vec::new, |input| {
            self.osc_stats = input.stats();
            input.try_iter().collect()
        });
        let system_now = SystemTime::now();
        let mut ready = Vec::new();
        for event in events {
            let deadline = crate::osc::instant_for_timetag(event.message.timetag, system_now, now);
            let delay = deadline.saturating_duration_since(now);
            if delay.is_zero() {
                ready.push((deadline, event));
            } else if delay <= MAX_OSC_SCHEDULE_AHEAD && self.osc_pending.len() < MAX_SCHEDULED_OSC
            {
                self.osc_pending.push((deadline, event));
            } else {
                self.osc_schedule_dropped = self.osc_schedule_dropped.saturating_add(1);
            }
        }
        let pending = std::mem::take(&mut self.osc_pending);
        for scheduled in pending {
            if scheduled.0 <= now {
                ready.push(scheduled);
            } else {
                self.osc_pending.push(scheduled);
            }
        }
        ready.sort_by_key(|(deadline, _)| *deadline);
        for (_, event) in ready {
            self.apply_osc_event(event, now);
        }
    }

    fn apply_osc_event(&mut self, event: crate::osc::OscEvent, now: Instant) {
        let address = event.message.address.clone();
        let Some(action) = crate::osc::map_message(&event.message) else {
            self.osc_status = format!("Ignored {address} from {}", event.peer);
            return;
        };
        let origin = CommandOrigin::Osc(event.peer.clone());
        match action {
            OscAction::Control(update) => self.dispatch_control_update(update, origin, now),
            OscAction::Tempo(bpm) => {
                self.record_show_operation(origin, now, CommandOperation::SetTempo { bpm });
                self.ui.bpm = bpm;
                self.tempo.set_bpm(
                    bpm,
                    now.saturating_duration_since(self.performance_started)
                        .as_secs_f64(),
                );
                self.tap_tempo.reset();
                self.publish_osc_value("/vjx/tempo", bpm as f32);
            }
            OscAction::OutputEnabled(enabled) => {
                self.record_show_operation(
                    origin,
                    now,
                    CommandOperation::SetOutputEnabled { enabled },
                );
                self.ui.output_enabled = enabled;
                self.output_window.set_visible(enabled);
                if enabled {
                    self.output_window.request_redraw();
                }
                self.publish_osc_value("/vjx/output/enabled", f32::from(enabled));
            }
            OscAction::OutputFullscreen(fullscreen) => {
                self.record_show_operation(
                    origin,
                    now,
                    CommandOperation::SetOutputFullscreen { fullscreen },
                );
                self.ui.output_fullscreen = fullscreen;
                self.apply_output_monitor();
                self.publish_osc_value("/vjx/output/fullscreen", f32::from(fullscreen));
            }
        }
        self.osc_status = format!("{address} · {}", event.peer);
    }

    pub(crate) fn publish_osc_value(&self, address: &str, value: f32) {
        if let Some(output) = &self.osc_output {
            output.try_feedback(address.to_owned(), value);
        }
    }

    fn publish_osc_control(&self, update: ControlUpdate) {
        if let Some((address, value)) = crate::osc::feedback_for_control(update) {
            self.publish_osc_value(&address, value);
        }
    }

    fn publish_osc_snapshot(&self) {
        for target in [
            ControlTarget::Crossfader,
            ControlTarget::MasterOpacity,
            ControlTarget::MasterBlackout,
            ControlTarget::MasterFreeze,
        ]
        .into_iter()
        .chain((0..4).flat_map(|deck| {
            [
                ControlTarget::DeckLevel(deck),
                ControlTarget::DeckPlay(deck),
                ControlTarget::DeckFreeze(deck),
                ControlTarget::DeckSpeed(deck),
                ControlTarget::DeckSelect(deck),
            ]
        })) {
            let value = current_control_value(&self.ui, &self.mixer, &self.transports, target);
            self.publish_osc_control(ControlUpdate { target, value });
        }
        self.publish_osc_value("/vjx/tempo", self.ui.bpm as f32);
        self.publish_osc_value("/vjx/output/enabled", f32::from(self.ui.output_enabled));
        self.publish_osc_value(
            "/vjx/output/fullscreen",
            f32::from(self.ui.output_fullscreen),
        );
    }

    pub(crate) fn dispatch_control_update(
        &mut self,
        update: ControlUpdate,
        origin: CommandOrigin,
        now: Instant,
    ) {
        if update.target == ControlTarget::TapTempo {
            if update.value >= 0.5 {
                let elapsed = now
                    .saturating_duration_since(self.performance_started)
                    .as_secs_f64();
                if let Some(bpm) = self.tap_tempo.tap(elapsed) {
                    self.record_show_operation(origin, now, CommandOperation::SetTempo { bpm });
                    self.ui.bpm = bpm;
                    self.tempo.set_bpm(bpm, elapsed);
                    self.publish_osc_value("/vjx/tempo", bpm as f32);
                }
            }
            return;
        }
        let Some(operation) = command_for_control(update) else {
            return;
        };
        self.record_show_operation(origin, now, operation);
        self.apply_control_update_unrecorded(update, now);
        let feedback_value = match update.target {
            ControlTarget::DeckRestart(_)
            | ControlTarget::ClipLaunch { .. }
            | ControlTarget::SceneLaunch(_) => update.value,
            target => current_control_value(&self.ui, &self.mixer, &self.transports, target),
        };
        self.publish_osc_control(ControlUpdate {
            target: update.target,
            value: feedback_value,
        });
    }

    pub(crate) fn apply_control_update_unrecorded(&mut self, update: ControlUpdate, now: Instant) {
        match update.target {
            ControlTarget::Crossfader => self.ui.crossfader = update.value.clamp(0.0, 1.0),
            ControlTarget::MasterOpacity => {
                self.ui.master_opacity = update.value.clamp(0.0, 1.0);
            }
            ControlTarget::MasterBlackout => self.ui.blackout = update.value >= 0.5,
            ControlTarget::MasterFreeze => self.ui.master_freeze = update.value >= 0.5,
            ControlTarget::TapTempo => unreachable!("tap tempo is handled by the gateway"),
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

fn command_for_control(update: ControlUpdate) -> Option<CommandOperation> {
    if update.value < 0.5
        && matches!(
            update.target,
            ControlTarget::DeckSelect(_)
                | ControlTarget::DeckRestart(_)
                | ControlTarget::ClipLaunch { .. }
                | ControlTarget::SceneLaunch(_)
        )
    {
        return None;
    }
    Some(match update.target {
        ControlTarget::ClipLaunch { deck, slot } => CommandOperation::LaunchClip { deck, slot },
        ControlTarget::SceneLaunch(slot) => CommandOperation::LaunchScene { slot },
        ControlTarget::MasterBlackout => CommandOperation::SetBlackout {
            enabled: update.value >= 0.5,
        },
        _ => CommandOperation::ControlValue {
            target: update.target,
            value: update.value,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_maps_semantic_launch_and_emergency_commands() {
        assert_eq!(
            command_for_control(ControlUpdate {
                target: ControlTarget::ClipLaunch { deck: 2, slot: 7 },
                value: 1.0,
            }),
            Some(CommandOperation::LaunchClip { deck: 2, slot: 7 })
        );
        assert_eq!(
            command_for_control(ControlUpdate {
                target: ControlTarget::MasterBlackout,
                value: 0.8,
            }),
            Some(CommandOperation::SetBlackout { enabled: true })
        );
    }

    #[test]
    fn gateway_does_not_record_release_edges_that_do_not_mutate_state() {
        assert_eq!(
            command_for_control(ControlUpdate {
                target: ControlTarget::SceneLaunch(3),
                value: 0.0,
            }),
            None
        );
    }
}
