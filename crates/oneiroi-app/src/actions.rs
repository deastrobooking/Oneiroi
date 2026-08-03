//! Translation from transient UI intent into app-state mutations.
//!
//! Keeping this boundary outside the frame renderer makes the command path
//! explicit while preserving the existing show-log and control semantics.

use std::time::Instant;

use oneiroi_core::ControlTarget;
use oneiroi_session::{CommandOperation, CommandOrigin};

use crate::{State, ui};

impl State {
    pub(super) fn dispatch_ui_actions(&mut self, actions: Vec<ui::UiAction>, now: Instant) {
        for action in actions {
            match action {
                ui::UiAction::Restart(deck) => {
                    self.dispatch_control_update(
                        oneiroi_core::ControlUpdate {
                            target: ControlTarget::DeckRestart(deck.index() as u8),
                            value: 1.0,
                        },
                        CommandOrigin::Operator,
                        now,
                    );
                }
                ui::UiAction::Seek(deck) => {
                    self.record_show_operation(
                        CommandOrigin::Operator,
                        now,
                        CommandOperation::SeekDeck {
                            deck: deck.index() as u8,
                            position_seconds: self.transports[deck.index()].position,
                        },
                    );
                    self.seek_deck(deck);
                }
                ui::UiAction::Launch(address) => {
                    self.dispatch_control_update(
                        oneiroi_core::ControlUpdate {
                            target: ControlTarget::ClipLaunch {
                                deck: address.deck.index() as u8,
                                slot: address.slot as u8,
                            },
                            value: 1.0,
                        },
                        CommandOrigin::Operator,
                        now,
                    );
                }
                ui::UiAction::LaunchScene(slot) => {
                    self.dispatch_control_update(
                        oneiroi_core::ControlUpdate {
                            target: ControlTarget::SceneLaunch(slot as u8),
                            value: 1.0,
                        },
                        CommandOrigin::Operator,
                        now,
                    );
                }
                ui::UiAction::MoveClip { from, to } => self.move_clip(from, to, now),
                ui::UiAction::ClearSlot(address) => {
                    self.clear_clip(address, now, CommandOrigin::Operator);
                }
                ui::UiAction::BrowseRelink(address) => self.browse_relink(address),
                ui::UiAction::Eject(deck) => {
                    self.record_show_operation(
                        CommandOrigin::Operator,
                        now,
                        CommandOperation::EjectDeck {
                            deck: deck.index() as u8,
                        },
                    );
                    self.master_effect_processor.reset_history();
                    self.clips
                        .remember_position(deck, self.transports[deck.index()].position);
                    self.mixer.eject(deck);
                    self.live_configs[deck.index()] = None;
                    self.clips.deactivate(deck);
                    self.launches.cancel(deck);
                    let generation = self.mixer.deck(deck).generation;
                    self.reset_playback(deck, generation);
                }
                ui::UiAction::SaveProject => self.save_project_from_ui(),
                ui::UiAction::OpenProject => self.open_project_from_ui(),
                ui::UiAction::TapTempo => {
                    let elapsed = now
                        .saturating_duration_since(self.performance_started)
                        .as_secs_f64();
                    if let Some(bpm) = self.tap_tempo.tap(elapsed) {
                        self.record_show_operation(
                            CommandOrigin::Operator,
                            now,
                            CommandOperation::SetTempo { bpm },
                        );
                        self.ui.bpm = bpm;
                        self.tempo.set_bpm(bpm, elapsed);
                        self.publish_osc_value("/vjx/tempo", bpm as f32);
                    }
                }
                ui::UiAction::HalfTempo => {
                    let bpm = (self.ui.bpm * 0.5).clamp(20.0, 400.0);
                    self.record_show_operation(
                        CommandOrigin::Operator,
                        now,
                        CommandOperation::SetTempo { bpm },
                    );
                    self.ui.bpm = bpm;
                    self.tempo.set_bpm(
                        bpm,
                        now.saturating_duration_since(self.performance_started)
                            .as_secs_f64(),
                    );
                    self.tap_tempo.reset();
                    self.publish_osc_value("/vjx/tempo", bpm as f32);
                }
                ui::UiAction::DoubleTempo => {
                    let bpm = (self.ui.bpm * 2.0).clamp(20.0, 400.0);
                    self.record_show_operation(
                        CommandOrigin::Operator,
                        now,
                        CommandOperation::SetTempo { bpm },
                    );
                    self.ui.bpm = bpm;
                    self.tempo.set_bpm(
                        bpm,
                        now.saturating_duration_since(self.performance_started)
                            .as_secs_f64(),
                    );
                    self.tap_tempo.reset();
                    self.publish_osc_value("/vjx/tempo", bpm as f32);
                }
                ui::UiAction::SetOutputEnabled(enabled) => {
                    self.record_show_operation(
                        CommandOrigin::Operator,
                        now,
                        CommandOperation::SetOutputEnabled { enabled },
                    );
                    self.output.window.set_visible(enabled);
                    if enabled {
                        self.output.window.request_redraw();
                    }
                    self.publish_osc_value("/vjx/output/enabled", f32::from(enabled));
                }
                ui::UiAction::SetOutputFullscreen(fullscreen) => {
                    self.record_show_operation(
                        CommandOrigin::Operator,
                        now,
                        CommandOperation::SetOutputFullscreen { fullscreen },
                    );
                    self.ui.output_fullscreen = fullscreen;
                    self.apply_output_monitor();
                    self.publish_osc_value("/vjx/output/fullscreen", f32::from(fullscreen));
                }
                ui::UiAction::SetOutputDisplay(id) => {
                    self.record_show_operation(
                        CommandOrigin::Operator,
                        now,
                        CommandOperation::SetParameter {
                            path: "output.display_id".to_owned(),
                            value: oneiroi_graph::ParameterValue::Text(id.clone()),
                        },
                    );
                    self.ui.output_display_id = id;
                    self.apply_output_monitor();
                }
                ui::UiAction::SetCompositionExtent(extent) => {
                    self.ui.composition_extent = extent;
                    self.ui.custom_composition_extent = extent;
                    if self.apply_output_settings() {
                        self.record_show_operation(
                            CommandOrigin::Operator,
                            now,
                            CommandOperation::SetOutputExtent { extent },
                        );
                    }
                }
                ui::UiAction::WatchEffectManifest => self.watch_effect_manifest(),
                ui::UiAction::ReloadEffectManifest => {
                    self.master_effect_processor.reload_effect_manifest();
                    self.ui.effect_reload_status =
                        self.master_effect_processor.reload_status().to_owned();
                }
                ui::UiAction::RefreshEffectRegistry => self.refresh_effect_registry(),
                ui::UiAction::RefreshDisplays => self.refresh_output_displays(),
                ui::UiAction::RecoverProject => {
                    if let Some(path) = self.recovery_path.clone() {
                        self.open_project(path, true);
                    }
                }
                ui::UiAction::RefreshSessionRecoveries => self.refresh_session_recoveries(),
                ui::UiAction::RestoreSessionRecovery(index) => {
                    self.restore_session_recovery(index, now);
                }
                ui::UiAction::RestoreSessionRecoveryAt {
                    index,
                    monotonic_ns,
                } => self.restore_session_recovery_at(index, monotonic_ns, now),
                ui::UiAction::StartNamedTake => self.start_named_take(now),
                ui::UiAction::SetRandomSeed => {
                    let scope = self.ui.random_seed_scope.trim().to_owned();
                    if !scope.is_empty() && scope.len() <= 128 {
                        self.record_show_operation(
                            CommandOrigin::Operator,
                            now,
                            CommandOperation::SetRandomSeed {
                                scope,
                                seed: self.ui.random_seed_value,
                            },
                        );
                    }
                }
                ui::UiAction::RenameProjectTake(index) => self.rename_project_take(index),
                ui::UiAction::RemoveProjectTake(index) => self.remove_project_take(index),
                ui::UiAction::AddTimelineMarker => self.add_timeline_marker(now),
                ui::UiAction::ExportProjectTake(index) => self.export_project_take(index, false),
                ui::UiAction::ArchiveProjectTake(index) => self.export_project_take(index, true),
                ui::UiAction::RefreshCameras => self.refresh_cameras(),
                ui::UiAction::RefreshAudioInputs => self.refresh_audio_inputs(),
                ui::UiAction::ConnectAudioInput(device_id) => self.connect_audio_input(device_id),
                ui::UiAction::DisconnectAudioInput => self.disconnect_audio_input(),
                ui::UiAction::RefreshMidiInputs => self.refresh_midi_inputs(),
                ui::UiAction::ConnectMidiInput(device_id) => self.connect_midi_input(device_id),
                ui::UiAction::DisconnectMidiInput(device_id) => {
                    self.disconnect_midi_input(&device_id);
                }
                ui::UiAction::MidiClearDevice(device_id) => {
                    self.midi
                        .bindings
                        .retain(|binding| binding.device != device_id);
                    self.midi_status = format!("Cleared all mappings for {device_id}");
                }
                ui::UiAction::ConnectOscInput => self.connect_osc_input(),
                ui::UiAction::DisconnectOscInput => self.disconnect_osc_input(),
                ui::UiAction::ConnectOscOutput => self.connect_osc_output(),
                ui::UiAction::DisconnectOscOutput => self.disconnect_osc_output(),
                ui::UiAction::MidiLearn(target) => self.midi.learn(target),
                ui::UiAction::MidiCancelLearn => self.midi.cancel_learn(),
                ui::UiAction::MidiClearTarget(target) => self.midi.clear_target(target),
                ui::UiAction::MidiRemoveBinding(index) => {
                    if index < self.midi.bindings.len() {
                        self.midi.bindings.remove(index);
                    }
                }
                ui::UiAction::ConnectCamera {
                    deck,
                    device_id,
                    label,
                    extent,
                    fps,
                } => {
                    self.record_show_operation(
                        CommandOrigin::Operator,
                        now,
                        CommandOperation::SetParameter {
                            path: format!("deck.{}.camera", deck.index()),
                            value: oneiroi_graph::ParameterValue::Text(format!(
                                "{device_id}|{}x{}@{fps}",
                                extent[0], extent[1]
                            )),
                        },
                    );
                    self.connect_camera(deck, device_id, label, extent, fps);
                }
            }
        }
    }
}
