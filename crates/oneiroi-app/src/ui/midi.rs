//! MIDI device panel, learn controls and mapping targets.

use super::*;

pub(super) fn draw_midi(
    ui: &mut egui::Ui,
    state: &mut UiState,
    metrics: &mut MidiMetrics<'_>,
    actions: &mut Vec<UiAction>,
) {
    let devices = metrics.devices;
    let inputs = metrics.inputs;
    let status = metrics.status;
    let selected_connected = metrics.device_connected(&state.midi_device_id);
    let any_connected = metrics.any_connected();
    let clock = metrics.clock;
    let midi = &mut *metrics.mapper;
    egui::CollapsingHeader::new("MIDI control")
        .default_open(false)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Input");
                let selected = inputs
                    .iter()
                    .find(|device| device.id == state.midi_device_id)
                    .map_or("No controller selected", |device| device.label.as_str());
                egui::ComboBox::from_id_salt("midi-input-device")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for device in inputs {
                            ui.selectable_value(
                                &mut state.midi_device_id,
                                device.id.clone(),
                                &device.label,
                            );
                        }
                    });
                if ui.button("Refresh MIDI").clicked() {
                    actions.push(UiAction::RefreshMidiInputs);
                }
                if selected_connected {
                    if ui.button("Disconnect").clicked() {
                        actions.push(UiAction::DisconnectMidiInput(state.midi_device_id.clone()));
                    }
                } else if ui
                    .add_enabled(
                        !state.midi_device_id.is_empty(),
                        egui::Button::new("Connect"),
                    )
                    .clicked()
                {
                    actions.push(UiAction::ConnectMidiInput(state.midi_device_id.clone()));
                }
                ui.weak(status);
            });

            ui.horizontal(|ui| {
                ui.label("Learn target");
                egui::ComboBox::from_id_salt("midi-learn-target")
                    .selected_text(midi_target_label(state.midi_target))
                    .show_ui(ui, |ui| {
                        for target in midi_targets() {
                            ui.selectable_value(
                                &mut state.midi_target,
                                target,
                                midi_target_label(target),
                            );
                        }
                    });
                if midi.learning().is_some() {
                    ui.colored_label(ui.visuals().warn_fg_color, "Move a control…");
                    if ui.button("Cancel learn").clicked() {
                        actions.push(UiAction::MidiCancelLearn);
                    }
                } else if ui
                    .add_enabled(any_connected, egui::Button::new("Learn"))
                    .clicked()
                {
                    actions.push(UiAction::MidiLearn(state.midi_target));
                }
                if ui.button("Clear target").clicked() {
                    actions.push(UiAction::MidiClearTarget(state.midi_target));
                }
            });

            let (received, dropped, parse_errors) =
                devices.iter().fold((0, 0, 0), |sums, device| {
                    (
                        sums.0 + device.stats.received,
                        sums.1 + device.stats.dropped,
                        sums.2 + device.stats.parse_errors,
                    )
                });
            ui.weak(format!(
                "{} device(s) live · events {received} · dropped {dropped} · parse errors \
                 {parse_errors} · {} mapping(s)",
                devices.iter().filter(|d| d.connected).count(),
                midi.bindings.len()
            ));

            draw_clock_sync(ui, state, inputs, &clock, actions);

            let mut remove = None;
            egui::Grid::new("midi-mappings")
                .striped(true)
                .num_columns(8)
                .show(ui, |ui| {
                    ui.strong("Source");
                    ui.strong("Target");
                    ui.strong("Mode");
                    ui.strong("Range");
                    ui.strong("Invert");
                    ui.strong("Pickup");
                    ui.strong("");
                    ui.end_row();
                    for (index, binding) in midi.bindings.iter_mut().enumerate() {
                        ui.label(format!(
                            "{} · ch {} · {:?} {}",
                            binding.device,
                            binding.channel + 1,
                            binding.kind,
                            binding.number
                        ));
                        ui.label(midi_target_label_for_state(binding.target, state));
                        egui::ComboBox::from_id_salt(("midi-mode", index))
                            .selected_text(mapping_mode_label(binding.mode))
                            .show_ui(ui, |ui| {
                                for mode in [
                                    MappingMode::Continuous,
                                    MappingMode::Momentary,
                                    MappingMode::Toggle,
                                    MappingMode::RelativeBinaryOffset,
                                    MappingMode::RelativeTwosComplement,
                                ] {
                                    ui.selectable_value(
                                        &mut binding.mode,
                                        mode,
                                        mapping_mode_label(mode),
                                    );
                                }
                            });
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut binding.output_range[0])
                                    .speed(0.01)
                                    .range(-8.0..=8.0),
                            );
                            ui.label("–");
                            ui.add(
                                egui::DragValue::new(&mut binding.output_range[1])
                                    .speed(0.01)
                                    .range(-8.0..=8.0),
                            );
                        });
                        ui.checkbox(&mut binding.invert, "");
                        ui.checkbox(&mut binding.soft_takeover, "");
                        if ui.small_button("Remove").clicked() {
                            remove = Some(index);
                        }
                        ui.end_row();
                    }
                });
            if let Some(index) = remove {
                actions.push(UiAction::MidiRemoveBinding(index));
            }
        });
}

/// Beat-clock sync: follow an external clock, send one downstream, or both.
fn draw_clock_sync(
    ui: &mut egui::Ui,
    state: &mut UiState,
    inputs: &[MidiInputDevice],
    clock: &MidiClockMetrics<'_>,
    actions: &mut Vec<UiAction>,
) {
    egui::CollapsingHeader::new("Clock sync")
        .default_open(false)
        .show(ui, |ui| {
            // --- Follow ---------------------------------------------------
            ui.horizontal(|ui| {
                ui.label("Tempo from");
                for (source, label, hint) in [
                    (
                        ClockSource::Internal,
                        "Internal",
                        "Tempo follows the BPM field and tap tempo",
                    ),
                    (
                        ClockSource::MidiInput,
                        "MIDI clock in",
                        "Tempo and beat phase follow an incoming 24 PPQN clock",
                    ),
                ] {
                    if ui
                        .selectable_label(state.midi_clock_source == source, label)
                        .on_hover_text(hint)
                        .clicked()
                        && state.midi_clock_source != source
                    {
                        actions.push(UiAction::SetMidiClockSource(source));
                    }
                }
                ui.separator();
                ui.label("From");
                let selected = if state.midi_clock_input_device.is_empty() {
                    "Any connected device"
                } else {
                    state.midi_clock_input_device.as_str()
                };
                egui::ComboBox::from_id_salt("midi-clock-input")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut state.midi_clock_input_device,
                            String::new(),
                            "Any connected device",
                        );
                        for device in inputs {
                            ui.selectable_value(
                                &mut state.midi_clock_input_device,
                                device.id.clone(),
                                &device.label,
                            );
                        }
                    });
            });
            ui.horizontal(|ui| {
                let (color, text) = if state.midi_clock_source != ClockSource::MidiInput {
                    (ui.visuals().weak_text_color(), "Not following".to_owned())
                } else if clock.locked {
                    (
                        ui.visuals().selection.bg_fill,
                        format!(
                            "LOCKED to {} · {:.2} BPM · {}",
                            clock.following.unwrap_or("clock"),
                            clock.follower_bpm.unwrap_or_default(),
                            if clock.follower_running {
                                "running"
                            } else {
                                "stopped"
                            }
                        ),
                    )
                } else {
                    (
                        ui.visuals().warn_fg_color,
                        "Waiting for clock · check the source is sending MIDI clock".to_owned(),
                    )
                };
                ui.colored_label(color, text);
            });
            ui.weak(format!(
                "pulses {} · jitter {:.2} ms · resyncs {}",
                clock.pulses,
                clock.jitter_micros as f64 / 1_000.0,
                clock.resyncs
            ));
            if !clock.status.is_empty() {
                ui.weak(clock.status);
            }

            ui.separator();

            // --- Send -----------------------------------------------------
            ui.horizontal(|ui| {
                ui.label("Clock out");
                let selected = clock
                    .outputs
                    .iter()
                    .find(|device| device.id == state.midi_output_device_id)
                    .map_or("No output selected", |device| device.label.as_str());
                egui::ComboBox::from_id_salt("midi-clock-output")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for device in clock.outputs {
                            ui.selectable_value(
                                &mut state.midi_output_device_id,
                                device.id.clone(),
                                &device.label,
                            );
                        }
                    });
                if ui.button("Refresh outputs").clicked() {
                    actions.push(UiAction::RefreshMidiOutputs);
                }
                if clock.output_connected {
                    if ui.button("Disconnect").clicked() {
                        actions.push(UiAction::DisconnectMidiClockOutput);
                    }
                } else if ui
                    .add_enabled(
                        !state.midi_output_device_id.is_empty(),
                        egui::Button::new("Connect"),
                    )
                    .clicked()
                {
                    actions.push(UiAction::ConnectMidiClockOutput(
                        state.midi_output_device_id.clone(),
                    ));
                }
            });
            ui.horizontal(|ui| {
                let mut send = state.midi_clock_send;
                if ui
                    .add_enabled(
                        clock.output_connected,
                        egui::Checkbox::new(&mut send, "Send clock"),
                    )
                    .on_hover_text("Sends Start, then 24 PPQN pulses, until switched off")
                    .changed()
                {
                    actions.push(UiAction::SetMidiClockSend(send));
                }
                if ui
                    .add_enabled(
                        clock.output_connected && !clock.output_running,
                        egui::Button::new("Continue"),
                    )
                    .on_hover_text("Resume downstream gear in place instead of rewinding")
                    .clicked()
                {
                    actions.push(UiAction::MidiClockContinue);
                }
                ui.weak(clock.output_status);
            });
            ui.weak(format!(
                "pulses {} · transport {} · late {} · worst {:.2} ms · resyncs {} · errors {}",
                clock.output_stats.pulses,
                clock.output_stats.transport,
                clock.output_stats.late,
                clock.output_stats.worst_late_micros as f64 / 1_000.0,
                clock.output_stats.resyncs,
                clock.output_stats.errors
            ));
        });
}

pub(super) fn midi_targets() -> Vec<ControlTarget> {
    let mut targets = vec![
        ControlTarget::Crossfader,
        ControlTarget::MasterOpacity,
        ControlTarget::MasterBlackout,
        ControlTarget::MasterFreeze,
        ControlTarget::TapTempo,
    ];
    for slot in 0..8 {
        targets.push(ControlTarget::SceneLaunch(slot));
    }
    for deck in 0..4 {
        targets.extend([
            ControlTarget::DeckLevel(deck),
            ControlTarget::DeckPlay(deck),
            ControlTarget::DeckFreeze(deck),
            ControlTarget::DeckSpeed(deck),
            ControlTarget::DeckSelect(deck),
            ControlTarget::DeckRestart(deck),
        ]);
        for slot in 0..8 {
            targets.push(ControlTarget::ClipLaunch { deck, slot });
        }
        for effect in 0..FIXED_DECK_EFFECT_PARAMETER_COUNT {
            targets.push(ControlTarget::EffectParameter {
                deck,
                effect,
                parameter: 0,
            });
        }
        for lfo in 0..3 {
            for parameter in 0..4 {
                targets.push(ControlTarget::LfoParameter {
                    deck,
                    lfo,
                    parameter,
                });
            }
        }
        for route in 0..8 {
            for parameter in 0..2 {
                targets.push(ControlTarget::ModRouteParameter {
                    deck,
                    route,
                    parameter,
                });
            }
        }
    }
    targets
}

pub(super) fn midi_target_label(target: ControlTarget) -> String {
    match target {
        ControlTarget::Crossfader => "Mixer · Crossfader".to_owned(),
        ControlTarget::MasterOpacity => "Master · Opacity".to_owned(),
        ControlTarget::MasterBlackout => "Master · Blackout".to_owned(),
        ControlTarget::MasterFreeze => "Master · Freeze".to_owned(),
        ControlTarget::TapTempo => "Tempo · Tap".to_owned(),
        ControlTarget::DeckLevel(deck) => format!("Deck {} · Level", deck_label(deck)),
        ControlTarget::DeckPlay(deck) => format!("Deck {} · Play", deck_label(deck)),
        ControlTarget::DeckFreeze(deck) => format!("Deck {} · Freeze", deck_label(deck)),
        ControlTarget::DeckSpeed(deck) => format!("Deck {} · Speed", deck_label(deck)),
        ControlTarget::DeckSelect(deck) => format!("Deck {} · Select", deck_label(deck)),
        ControlTarget::DeckRestart(deck) => format!("Deck {} · Restart", deck_label(deck)),
        ControlTarget::ClipLaunch { deck, slot } => {
            format!("Deck {} · Launch clip {}", deck_label(deck), slot + 1)
        }
        ControlTarget::SceneLaunch(slot) => format!("Scene · Launch {}", slot + 1),
        ControlTarget::EffectParameter { deck, effect, .. } => format!(
            "Deck {} · FX {}",
            deck_label(deck),
            effect_parameter_label(effect)
        ),
        ControlTarget::LfoParameter {
            deck,
            lfo,
            parameter,
        } => format!(
            "Deck {} · LFO {} · {}",
            deck_label(deck),
            lfo + 1,
            ["Enabled", "Rate", "Depth", "Phase"]
                .get(usize::from(parameter))
                .copied()
                .unwrap_or("Unknown")
        ),
        ControlTarget::ModRouteParameter {
            deck,
            route,
            parameter,
        } => format!(
            "Deck {} · Matrix {} · {}",
            deck_label(deck),
            route + 1,
            ["Enabled", "Amount"]
                .get(usize::from(parameter))
                .copied()
                .unwrap_or("Unknown")
        ),
        ControlTarget::DeckEffectParameter {
            deck,
            parameter_key,
        } => format!("Deck {} · custom {:016x}", deck_label(deck), parameter_key),
        ControlTarget::MasterEffectParameter {
            slot,
            parameter_key,
        } => format!("Master slot {} · custom {:016x}", slot + 1, parameter_key),
    }
}

pub(super) fn midi_target_label_for_state(target: ControlTarget, state: &UiState) -> String {
    match target {
        ControlTarget::DeckEffectParameter {
            deck,
            parameter_key,
        } => {
            let deck_index = usize::from(deck);
            let label = state
                .deck_packages
                .get(deck_index)
                .and_then(|effect| {
                    state
                        .deck_effect_packages
                        .iter()
                        .find(|package| package.id == effect.package_id)
                })
                .and_then(|package| {
                    package.parameters.iter().find(|parameter| {
                        effect_parameter_key(&package.id, &parameter.id) == parameter_key
                    })
                });
            label.map_or_else(
                || midi_target_label(target),
                |parameter| format!("Deck {} · {}", deck_label(deck), parameter.label),
            )
        }
        ControlTarget::MasterEffectParameter {
            slot,
            parameter_key,
        } => {
            let slot_index = usize::from(slot);
            let label = state
                .master_effects
                .slots
                .get(slot_index)
                .and_then(|effect| {
                    state
                        .effect_packages
                        .iter()
                        .find(|package| package.id == effect.package_id)
                })
                .and_then(|package| {
                    package.parameters.iter().find(|parameter| {
                        effect_parameter_key(&package.id, &parameter.id) == parameter_key
                    })
                });
            label.map_or_else(
                || midi_target_label(target),
                |parameter| format!("Master slot {} · {}", slot_index + 1, parameter.label),
            )
        }
        _ => midi_target_label(target),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_targets_include_the_last_built_in_effect_parameter() {
        assert!(midi_targets().contains(&ControlTarget::EffectParameter {
            deck: 3,
            effect: FIXED_DECK_EFFECT_PARAMETER_COUNT - 1,
            parameter: 0,
        }));
    }
}
