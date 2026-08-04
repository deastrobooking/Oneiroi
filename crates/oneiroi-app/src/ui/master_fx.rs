//! Custom master effect controls and master modulation routing.

use super::*;

pub(super) fn draw_custom_effect(
    ui: &mut egui::Ui,
    slot_index: usize,
    slot: &mut MasterEffectSlot,
    packages: &[EffectDescriptor],
    actions: &mut Vec<UiAction>,
) {
    let selected = packages
        .iter()
        .find(|package| package.id == slot.package_id)
        .map_or("Missing package", |package| package.name.as_str());
    let previous_id = slot.package_id.clone();
    egui::ComboBox::from_id_salt(format!("master-custom-package-{slot_index}"))
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for package in packages {
                if ui
                    .selectable_label(slot.package_id == package.id, &package.name)
                    .clicked()
                {
                    slot.package_id.clone_from(&package.id);
                }
            }
        });
    if slot.package_id != previous_id
        && let Some(package) = packages
            .iter()
            .find(|package| package.id == slot.package_id)
    {
        slot.parameters = package
            .parameters
            .iter()
            .map(|parameter| EffectParameterValue {
                id: parameter.id.clone(),
                value: parameter.default,
            })
            .collect();
    }
    let Some(package) = packages
        .iter()
        .find(|package| package.id == slot.package_id)
    else {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!(
                "Package {:?} is unavailable; this slot passes through.",
                slot.package_id
            ),
        );
        return;
    };
    if package.pass_count == 1 {
        ui.weak("1 bounded render pass");
    } else {
        ui.weak(format!("{} bounded render passes", package.pass_count));
    }
    if package.history == EffectHistoryResource::PreviousSlotOutput {
        ui.weak("Persistent previous-slot-output history");
    }
    if !package.description.is_empty() {
        ui.label(&package.description);
    }
    if !package.presets.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Looks");
            for preset in &package.presets {
                let response = ui.small_button(&preset.label);
                if response.clicked() {
                    for (parameter_id, preset_value) in &preset.values {
                        if let Some(value) = slot
                            .parameters
                            .iter_mut()
                            .find(|value| value.id == *parameter_id)
                        {
                            value.value = *preset_value;
                        }
                    }
                }
                if !preset.description.is_empty() {
                    response.on_hover_text(&preset.description);
                }
            }
            if ui.small_button("Reset controls").clicked() {
                for parameter in &package.parameters {
                    if let Some(value) = slot
                        .parameters
                        .iter_mut()
                        .find(|value| value.id == parameter.id)
                    {
                        value.value = parameter.default;
                    }
                }
            }
        });
    }
    let mut previous_group = None::<&str>;
    for parameter in &package.parameters {
        let group = parameter.group.trim();
        if !group.is_empty() && previous_group != Some(group) {
            ui.add_space(3.0);
            ui.strong(group);
            previous_group = Some(group);
        }
        let value_index = slot
            .parameters
            .iter()
            .position(|value| value.id == parameter.id);
        let index = value_index.unwrap_or_else(|| {
            slot.parameters.push(EffectParameterValue {
                id: parameter.id.clone(),
                value: parameter.default,
            });
            slot.parameters.len() - 1
        });
        ui.horizontal(|ui| {
            match parameter.control {
                EffectParameterControl::Slider => {
                    ui.add(
                        egui::Slider::new(
                            &mut slot.parameters[index].value,
                            parameter.minimum..=parameter.maximum,
                        )
                        .text(&parameter.label),
                    );
                }
                EffectParameterControl::Toggle => {
                    let mut enabled = slot.parameters[index].value >= 0.5;
                    if ui.checkbox(&mut enabled, &parameter.label).changed() {
                        slot.parameters[index].value = if enabled { 1.0 } else { 0.0 };
                    }
                }
                EffectParameterControl::Choice => {
                    let selected = parameter
                        .options
                        .iter()
                        .min_by(|left, right| {
                            (left.value - slot.parameters[index].value)
                                .abs()
                                .total_cmp(&(right.value - slot.parameters[index].value).abs())
                        })
                        .map_or("Choose", |option| option.label.as_str());
                    ui.label(&parameter.label);
                    egui::ComboBox::from_id_salt((
                        "master-custom-choice",
                        slot_index,
                        parameter.id.as_str(),
                    ))
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for option in &parameter.options {
                            ui.selectable_value(
                                &mut slot.parameters[index].value,
                                option.value,
                                &option.label,
                            );
                        }
                    });
                }
            }
            let target = ControlTarget::MasterEffectParameter {
                slot: slot_index as u8,
                parameter_key: effect_parameter_key(&slot.package_id, &parameter.id),
            };
            if ui.small_button("MIDI learn").clicked() {
                actions.push(UiAction::MidiLearn(target));
            }
            if ui.small_button("Clear").clicked() {
                actions.push(UiAction::MidiClearTarget(target));
            }
        });
    }
}

pub(super) fn draw_master_modulation(
    ui: &mut egui::Ui,
    modulation: &mut MasterModulation,
    effects: &MasterEffectChain,
    packages: &[EffectDescriptor],
) {
    egui::CollapsingHeader::new("Master modulation matrix")
        .default_open(false)
        .show(ui, |ui| {
            for (index, lfo) in modulation.lfos.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut lfo.enabled, format!("LFO {}", index + 1));
                        egui::ComboBox::from_id_salt(("master-lfo-wave", index))
                            .selected_text(waveform_label(lfo.waveform))
                            .show_ui(ui, |ui| {
                                for waveform in LFO_WAVEFORMS {
                                    ui.selectable_value(
                                        &mut lfo.waveform,
                                        waveform,
                                        waveform_label(waveform),
                                    );
                                }
                            });
                        ui.checkbox(&mut lfo.tempo_sync, "Sync");
                    });
                    ui.horizontal(|ui| {
                        if lfo.tempo_sync {
                            ui.add(
                                egui::Slider::new(&mut lfo.beats_per_cycle, 0.0625..=8.0)
                                    .text("beats"),
                            );
                        } else {
                            ui.add(
                                egui::Slider::new(&mut lfo.rate_hz, 0.01..=20.0)
                                    .logarithmic(true)
                                    .text("Hz"),
                            );
                        }
                        ui.add(egui::Slider::new(&mut lfo.depth, 0.0..=1.0).text("depth"));
                        ui.add(egui::Slider::new(&mut lfo.phase, 0.0..=1.0).text("phase"));
                    });
                });
            }

            for (index, route) in modulation.routes.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut route.enabled, format!("{}", index + 1));
                    egui::ComboBox::from_id_salt(("master-mod-source", index))
                        .selected_text(master_mod_source_label(route.source))
                        .show_ui(ui, |ui| {
                            for source in 0..10 {
                                ui.selectable_value(
                                    &mut route.source,
                                    source,
                                    master_mod_source_label(source),
                                );
                            }
                        });
                    egui::ComboBox::from_id_salt(("master-mod-target", index))
                        .selected_text(master_mod_target_label(route, effects, packages))
                        .show_ui(ui, |ui| {
                            for (slot_index, slot) in effects.slots.iter().enumerate() {
                                if slot.kind != MasterEffectKind::Custom {
                                    continue;
                                }
                                let Some(package) = packages
                                    .iter()
                                    .find(|package| package.id == slot.package_id)
                                else {
                                    continue;
                                };
                                for parameter in &package.parameters {
                                    let key = effect_parameter_key(&package.id, &parameter.id);
                                    if ui
                                        .selectable_label(
                                            usize::from(route.target_slot) == slot_index
                                                && route.parameter_key == key,
                                            format!(
                                                "Slot {} · {}",
                                                slot_index + 1,
                                                parameter.label
                                            ),
                                        )
                                        .clicked()
                                    {
                                        route.target_slot = slot_index as u8;
                                        route.parameter_key = key;
                                    }
                                }
                            }
                        });
                    ui.add(
                        egui::Slider::new(&mut route.amount, -1.0..=1.0)
                            .text("amount")
                            .show_value(true),
                    );
                });
            }
            ui.weak("Sources: three master LFOs, audio analysis, beat and bar phase.");
        });
}

pub(super) fn master_mod_source_label(source: u8) -> &'static str {
    [
        "LFO 1",
        "LFO 2",
        "LFO 3",
        "Audio RMS",
        "Audio bass",
        "Audio mid",
        "Audio high",
        "Audio transient",
        "Beat phase",
        "Bar phase",
    ]
    .get(usize::from(source))
    .copied()
    .unwrap_or("Unknown")
}

pub(super) fn master_mod_target_label(
    route: &oneiroi_render::MasterModulationRoute,
    effects: &MasterEffectChain,
    packages: &[EffectDescriptor],
) -> String {
    let slot_index = usize::from(route.target_slot);
    let Some(slot) = effects.slots.get(slot_index) else {
        return "Choose target".to_owned();
    };
    if slot.kind != MasterEffectKind::Custom {
        return "Missing target".to_owned();
    }
    let Some(package) = packages
        .iter()
        .find(|package| package.id == slot.package_id)
    else {
        return "Missing target".to_owned();
    };
    package
        .parameters
        .iter()
        .find(|parameter| effect_parameter_key(&package.id, &parameter.id) == route.parameter_key)
        .map_or_else(
            || "Choose target".to_owned(),
            |parameter| format!("Slot {} · {}", slot_index + 1, parameter.label),
        )
}
