//! The MIDI Manager: a separate operator window for multi-device management
//! and an assignment browser grouped by device.
//!
//! The main window's MIDI section stays the quick path; this window is where
//! an operator prepares a rig — connect several controllers, arm map mode,
//! audit every assignment each device owns, and prune stale ones.

use super::midi::midi_target_label;
use super::*;

pub(super) fn draw_midi_manager(
    ctx: &egui::Context,
    state: &mut UiState,
    metrics: &mut MidiMetrics<'_>,
    palette: &ThemePalette,
    actions: &mut Vec<UiAction>,
) {
    if !state.midi_manager_open {
        return;
    }
    let devices = metrics.devices;
    let inputs = metrics.inputs;
    let midi = &mut *metrics.mapper;

    let mut open = state.midi_manager_open;
    egui::Window::new("MIDI Manager")
        .open(&mut open)
        .default_size([560.0, 480.0])
        .min_size([420.0, 280.0])
        .scroll([false, true])
        .show(ctx, |ui| {
            // --- Map mode -------------------------------------------------
            ui.horizontal(|ui| {
                let armed = state.midi_map_mode;
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(if armed {
                                "EXIT MAP MODE"
                            } else {
                                "MIDI MAP MODE"
                            })
                            .strong(),
                        )
                        .fill(if armed {
                            palette.accent.linear_multiply(0.45)
                        } else {
                            palette.control
                        }),
                    )
                    .clicked()
                {
                    state.midi_map_mode = !state.midi_map_mode;
                    if !state.midi_map_mode {
                        actions.push(UiAction::MidiCancelLearn);
                    }
                }
                if let Some(target) = midi.learning() {
                    ui.colored_label(
                        palette.accent,
                        format!("Armed: {} · move a control", midi_target_label(target)),
                    );
                } else if armed {
                    ui.weak("Click any highlighted control, then move a knob");
                }
            });
            ui.separator();

            // --- Devices --------------------------------------------------
            ui.strong("Devices");
            if devices.is_empty() {
                ui.weak("No MIDI inputs detected. Connect hardware and refresh.");
            }
            egui::Grid::new("midi-manager-devices")
                .striped(true)
                .num_columns(5)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    for device in devices {
                        let color = if device.connected {
                            palette.success
                        } else {
                            palette.idle
                        };
                        ui.label(egui::RichText::new("●").color(color));
                        ui.label(&device.label);
                        ui.weak(format!(
                            "events {} · dropped {}",
                            device.stats.received, device.stats.dropped
                        ));
                        ui.label(if device.wanted && !device.connected {
                            "waiting for hardware"
                        } else if device.wanted {
                            "auto-reconnect"
                        } else {
                            ""
                        });
                        if device.connected {
                            if ui.button("Disconnect").clicked() {
                                actions.push(UiAction::DisconnectMidiInput(device.id.clone()));
                            }
                        } else if ui.button("Connect").clicked() {
                            actions.push(UiAction::ConnectMidiInput(device.id.clone()));
                        }
                        ui.end_row();
                    }
                });
            ui.horizontal(|ui| {
                if ui.button("Refresh devices").clicked() {
                    actions.push(UiAction::RefreshMidiInputs);
                }
                ui.weak(format!(
                    "{} detected · {} connected",
                    inputs.len(),
                    devices.iter().filter(|device| device.connected).count()
                ));
            });
            ui.separator();

            // --- Assignments, grouped by device ---------------------------
            ui.strong("Assignments by device");
            if midi.bindings.is_empty() {
                ui.weak("No assignments yet. Arm map mode, click a control, move a knob.");
            }

            // Bindings keep their mapper index so deletion stays exact even
            // though the display is grouped. Offline devices still list their
            // assignments — that is the point of the browser.
            let mut by_device: Vec<(String, Vec<usize>)> = Vec::new();
            for (index, binding) in midi.bindings.iter().enumerate() {
                match by_device
                    .iter_mut()
                    .find(|(device, _)| *device == binding.device)
                {
                    Some((_, indices)) => indices.push(index),
                    None => by_device.push((binding.device.clone(), vec![index])),
                }
            }

            for (device, indices) in by_device {
                let online = devices
                    .iter()
                    .any(|status| status.connected && status.id == device);
                let title = format!(
                    "{device} · {} assignment(s){}",
                    indices.len(),
                    if online { "" } else { " · offline" }
                );
                egui::CollapsingHeader::new(title)
                    .id_salt(("midi-manager-device", &device))
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::Grid::new(("midi-manager-bindings", &device))
                            .striped(true)
                            .num_columns(4)
                            .spacing([12.0, 4.0])
                            .show(ui, |ui| {
                                for index in &indices {
                                    let binding = &midi.bindings[*index];
                                    ui.label(format!(
                                        "ch{} {:?} #{}",
                                        binding.channel + 1,
                                        binding.kind,
                                        binding.number
                                    ));
                                    ui.label(midi_target_label(binding.target));
                                    ui.weak(mapping_mode_label(binding.mode));
                                    if ui.button("Delete").clicked() {
                                        actions.push(UiAction::MidiRemoveBinding(*index));
                                    }
                                    ui.end_row();
                                }
                            });
                        if ui.button("Clear this device").clicked() {
                            actions.push(UiAction::MidiClearDevice(device.clone()));
                        }
                    });
            }
        });
    state.midi_manager_open = open;
}
