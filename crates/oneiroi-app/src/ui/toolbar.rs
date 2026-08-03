//! Always-visible show controls and release preflight summary.

use oneiroi_media::{CLIPS_PER_DECK, ClipAddress, ClipBank, DeckId};

use super::theme::ThemePalette;
use super::{PerformanceMetrics, UiAction, UiState};

pub(super) fn draw_toolbar(
    ui: &mut egui::Ui,
    state: &mut UiState,
    clips: &ClipBank,
    metrics: &PerformanceMetrics<'_>,
    palette: ThemePalette,
    actions: &mut Vec<UiAction>,
) {
    let mut missing_media = 0;
    let mut loading_media = 0;
    for deck in DeckId::ALL {
        for slot in 0..CLIPS_PER_DECK {
            let address = ClipAddress { deck, slot };
            if let Some(slot_state) = clips.slot(address) {
                if slot_state.error.is_some() {
                    missing_media += 1;
                } else if slot_state.movie.is_none() && slot_state.pending_path.is_some() {
                    loading_media += 1;
                }
            }
        }
    }
    let midi_waiting = metrics
        .midi
        .devices
        .iter()
        .filter(|device| device.wanted && !device.connected)
        .count();
    let output_ready = state.output_enabled
        && !metrics.output_displays.is_empty()
        && metrics.output_health.status == "Healthy";
    let effect_rejected = state.effect_reload_status.contains("rejected");
    let preflight_ready = output_ready
        && missing_media == 0
        && loading_media == 0
        && !effect_rejected
        && !metrics.project_dirty;

    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("ONEIROI")
                    .size(22.0)
                    .strong()
                    .color(palette.accent),
            );
            ui.weak("LIVE VISUAL PERFORMANCE SYSTEM");
        });
        ui.add_space(12.0);
        status_dot(
            ui,
            &palette,
            "PROGRAM",
            state.output_enabled,
            state.output_enabled && metrics.output_health.status != "Healthy",
        );
        status_dot(ui, &palette, "AUDIO", metrics.audio_connected, false);
        status_dot(ui, &palette, "MIDI", metrics.midi.any_connected(), false);
        status_dot(ui, &palette, "OSC", metrics.osc.connected, false);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let blackout_fill = if state.blackout {
                palette.danger
            } else {
                palette.control_tint(palette.danger, 0.18)
            };
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("BLACKOUT")
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(blackout_fill)
                    .min_size(egui::vec2(104.0, 34.0)),
                )
                .on_hover_text("Emergency program blackout")
                .clicked()
            {
                state.blackout = !state.blackout;
            }
            if ui
                .add(
                    egui::Button::new(if state.master_freeze {
                        "Resume master"
                    } else {
                        "Freeze master"
                    })
                    .selected(state.master_freeze),
                )
                .clicked()
            {
                state.master_freeze = !state.master_freeze;
            }
            let show_label = if state.show_mode {
                "EXIT SHOW MODE"
            } else {
                "SHOW MODE"
            };
            if ui
                .add(
                    egui::Button::new(egui::RichText::new(show_label).strong()).fill(
                        if state.show_mode {
                            palette.control_tint(palette.success, 0.36)
                        } else {
                            palette.control
                        },
                    ),
                )
                .on_hover_text("Lock setup, media management and structural effect editors")
                .clicked()
            {
                state.show_mode = !state.show_mode;
                if state.show_mode {
                    state.midi_map_mode = false;
                    state.midi_manager_open = false;
                    actions.push(UiAction::MidiCancelLearn);
                }
            }
            if !state.show_mode {
                ui.menu_button("Theme", |ui| {
                    state.theme.picker_ui(ui);
                });
                if ui
                    .selectable_label(state.midi_manager_open, "MIDI")
                    .on_hover_text("Open the MIDI Manager window")
                    .clicked()
                {
                    state.midi_manager_open = !state.midi_manager_open;
                }
                if state.midi_map_mode {
                    let response = ui.selectable_label(
                        true,
                        egui::RichText::new("MAP").strong().color(palette.accent),
                    );
                    if response
                        .on_hover_text("MIDI map mode armed · click to exit")
                        .clicked()
                    {
                        state.midi_map_mode = false;
                        actions.push(UiAction::MidiCancelLearn);
                    }
                }
            }
            ui.weak(format!("{:.0} fps", state.fps.fps()));
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.weak(metrics.gpu_info);
        ui.separator();
        ui.weak(metrics.runtime_status);
    });
    ui.horizontal_wrapped(|ui| {
        let (label, color) = if preflight_ready {
            ("● PREFLIGHT READY", palette.success)
        } else {
            ("● PREFLIGHT ATTENTION", palette.warning)
        };
        ui.colored_label(color, egui::RichText::new(label).strong());
        ui.separator();
        ui.label(if output_ready {
            "output healthy"
        } else {
            "output not ready"
        });
        ui.label(format!("missing {missing_media}"));
        ui.label(format!("loading {loading_media}"));
        ui.label(format!("MIDI waiting {midi_waiting}"));
        ui.label(if metrics.project_dirty {
            "project modified"
        } else {
            "project saved"
        });
        if effect_rejected {
            ui.colored_label(palette.danger, "effect reload rejected");
        }
        if state.show_mode {
            ui.separator();
            ui.colored_label(palette.success, "SHOW LOCKED");
        }
    });
}

fn status_dot(ui: &mut egui::Ui, palette: &ThemePalette, label: &str, active: bool, warning: bool) {
    let color = if warning {
        palette.danger
    } else if active {
        palette.success
    } else {
        palette.idle
    };
    ui.label(egui::RichText::new("●").color(color));
    ui.weak(label);
}
