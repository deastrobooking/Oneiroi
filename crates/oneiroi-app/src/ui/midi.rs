//! MIDI device panel, learn controls and mapping targets.

use super::*;

pub(super) fn draw_midi(
    ui: &mut egui::Ui,
    state: &mut UiState,
    metrics: &mut MidiMetrics<'_>,
    actions: &mut Vec<UiAction>,
) {
    let midi = &mut *metrics.mapper;
    egui::CollapsingHeader::new("MIDI control")
        .default_open(false)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Input");
                let selected = metrics
                    .inputs
                    .iter()
                    .find(|device| device.id == state.midi_device_id)
                    .map_or("No controller selected", |device| device.label.as_str());
                egui::ComboBox::from_id_salt("midi-input-device")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for device in metrics.inputs {
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
                if metrics.connected {
                    if ui.button("Disconnect").clicked() {
                        actions.push(UiAction::DisconnectMidiInput);
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
                ui.weak(metrics.status);
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
                    ui.colored_label(egui::Color32::YELLOW, "Move a control…");
                    if ui.button("Cancel learn").clicked() {
                        actions.push(UiAction::MidiCancelLearn);
                    }
                } else if ui
                    .add_enabled(metrics.connected, egui::Button::new("Learn"))
                    .clicked()
                {
                    actions.push(UiAction::MidiLearn(state.midi_target));
                }
                if ui.button("Clear target").clicked() {
                    actions.push(UiAction::MidiClearTarget(state.midi_target));
                }
            });

            ui.weak(format!(
                "events {} · dropped {} · parse errors {} · {} mapping(s)",
                metrics.stats.received,
                metrics.stats.dropped,
                metrics.stats.parse_errors,
                midi.bindings.len()
            ));

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
        for effect in 0..14 {
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
        ControlTarget::MasterEffectParameter {
            slot,
            parameter_key,
        } => format!("Master slot {} · custom {:016x}", slot + 1, parameter_key),
    }
}

pub(super) fn midi_target_label_for_state(target: ControlTarget, state: &UiState) -> String {
    let ControlTarget::MasterEffectParameter {
        slot,
        parameter_key,
    } = target
    else {
        return midi_target_label(target);
    };
    let slot_index = usize::from(slot);
    let Some(effect) = state.master_effects.slots.get(slot_index) else {
        return midi_target_label(target);
    };
    let label = state
        .effect_packages
        .iter()
        .find(|package| package.id == effect.package_id)
        .and_then(|package| {
            package
                .parameters
                .iter()
                .find(|parameter| effect_parameter_key(&package.id, &parameter.id) == parameter_key)
        });
    label.map_or_else(
        || midi_target_label(target),
        |parameter| format!("Master slot {} · {}", slot_index + 1, parameter.label),
    )
}
