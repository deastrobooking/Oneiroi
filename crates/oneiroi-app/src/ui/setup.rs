//! Pre-show output, project, device, and diagnostics workspace.

use oneiroi_core::{ControlTarget, Quantization};
use oneiroi_media::FourDeckMixer;

use super::diagnostics::{draw_output_health, draw_pipeline_health, draw_runtime_summary};
use super::{MidiMapUi, PerformanceMetrics, UiAction, UiState, draw_midi, mappable};

pub(super) fn draw_setup(
    ui: &mut egui::Ui,
    state: &mut UiState,
    mixer: &FourDeckMixer,
    midi_map: &MidiMapUi,
    actions: &mut Vec<UiAction>,
    metrics: &mut PerformanceMetrics<'_>,
) {
    let palette = midi_map.palette;
    if !state.show_mode {
        let setup_height = (ui.available_height() * 0.45).clamp(160.0, 420.0);
        egui::CollapsingHeader::new("Setup & diagnostics · output, project, devices")
        .default_open(false)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("setup-region")
                .max_height(setup_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Output");
                if ui
                    .checkbox(&mut state.output_enabled, "Enabled")
                    .changed()
                {
                    actions.push(UiAction::SetOutputEnabled(state.output_enabled));
                }
                if ui
                    .add_enabled(
                        state.output_enabled,
                        egui::Checkbox::new(&mut state.output_fullscreen, "Fullscreen"),
                    )
                    .changed()
                {
                    actions.push(UiAction::SetOutputFullscreen(state.output_fullscreen));
                }
                let display_label = metrics
                    .output_displays
                    .iter()
                    .find(|display| display.id == state.output_display_id)
                    .map_or("No display", |display| display.label.as_str());
                egui::ComboBox::from_id_salt("output-display")
                    .selected_text(display_label)
                    .show_ui(ui, |ui| {
                        for display in metrics.output_displays {
                            if ui
                                .selectable_value(
                                    &mut state.output_display_id,
                                    display.id.clone(),
                                    &display.label,
                                )
                                .changed()
                            {
                                actions.push(UiAction::SetOutputDisplay(display.id.clone()));
                            }
                        }
                    });
                if ui.button("Refresh displays").clicked() {
                    actions.push(UiAction::RefreshDisplays);
                }
                ui.separator();
                egui::ComboBox::from_id_salt("composition-resolution")
                    .selected_text(format!(
                        "{} × {}",
                        state.composition_extent[0], state.composition_extent[1]
                    ))
                    .show_ui(ui, |ui| {
                        for (label, extent) in [
                            ("720p", [1280, 720]),
                            ("1080p", [1920, 1080]),
                            ("UHD", [3840, 2160]),
                        ] {
                            if ui
                                .selectable_value(
                                    &mut state.composition_extent,
                                    extent,
                                    format!("{label} · {} × {}", extent[0], extent[1]),
                                )
                                .changed()
                            {
                                state.custom_composition_extent = extent;
                                actions.push(UiAction::SetCompositionExtent(extent));
                            }
                        }
                    });
                ui.label("Custom");
                ui.add(
                    egui::DragValue::new(&mut state.custom_composition_extent[0])
                        .range(320..=7680)
                        .speed(8),
                );
                ui.label("×");
                ui.add(
                    egui::DragValue::new(&mut state.custom_composition_extent[1])
                        .range(180..=4320)
                        .speed(8),
                );
                if ui.button("Apply").clicked() {
                    actions.push(UiAction::SetCompositionExtent(
                        state.custom_composition_extent,
                    ));
                }
                ui.separator();
                ui.checkbox(&mut state.output_test_card, "Test card");
                ui.checkbox(&mut state.output_identify, "Identify");
            });
            draw_output_health(ui, state, metrics, palette);
            ui.horizontal(|ui| {
                ui.label(if metrics.project_dirty {
                    "● Modified"
                } else {
                    "Saved"
                });
                ui.add_sized(
                    [320.0, 22.0],
                    egui::TextEdit::singleline(&mut state.project_path)
                        .hint_text("project.oneiroi"),
                );
                if ui.button("Open").clicked() {
                    actions.push(UiAction::OpenProject);
                }
                if ui.button("Save").clicked() {
                    actions.push(UiAction::SaveProject);
                }
                if metrics.recovery_available && ui.button("Recover autosave").clicked() {
                    actions.push(UiAction::RecoverProject);
                }
                if !metrics.project_status.is_empty() {
                    ui.weak(metrics.project_status);
                }
            });
            egui::CollapsingHeader::new("Session recovery")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Scan journals").clicked() {
                            actions.push(UiAction::RefreshSessionRecoveries);
                        }
                        if !metrics.session_recoveries.is_empty() {
                            let selected = state
                                .session_recovery_selected
                                .min(metrics.session_recoveries.len() - 1);
                            state.session_recovery_selected = selected;
                            egui::ComboBox::from_id_salt("session-recovery-select")
                                .selected_text(metrics.session_recoveries[selected].file_name())
                                .show_ui(ui, |ui| {
                                    for (index, entry) in
                                        metrics.session_recoveries.iter().enumerate()
                                    {
                                        ui.selectable_value(
                                            &mut state.session_recovery_selected,
                                            index,
                                            entry.file_name(),
                                        );
                                    }
                                });
                            if ui.button("Restore latest as branch").clicked() {
                                actions.push(UiAction::RestoreSessionRecovery(
                                    state.session_recovery_selected,
                                ));
                            }
                        }
                    });
                    if !metrics.project_takes.is_empty() {
                        let selected = state
                            .project_take_selected
                            .min(metrics.project_takes.len() - 1);
                        state.project_take_selected = selected;
                        ui.horizontal(|ui| {
                            ui.label("Project take catalog");
                            egui::ComboBox::from_id_salt("project-take-select")
                                .selected_text(&metrics.project_takes[selected].name)
                                .show_ui(ui, |ui| {
                                    for (index, take) in metrics.project_takes.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut state.project_take_selected,
                                            index,
                                            format!("{} · {}", take.name, take.journal_file),
                                        );
                                    }
                                });
                            if ui.button("Rename metadata").clicked() {
                                actions.push(UiAction::RenameProjectTake(
                                    state.project_take_selected,
                                ));
                            }
                            if ui.button("Remove metadata").clicked() {
                                actions.push(UiAction::RemoveProjectTake(
                                    state.project_take_selected,
                                ));
                            }
                            if ui.button("Export copy").clicked() {
                                actions.push(UiAction::ExportProjectTake(
                                    state.project_take_selected,
                                ));
                            }
                            if ui.button("Archive copy").clicked() {
                                actions.push(UiAction::ArchiveProjectTake(
                                    state.project_take_selected,
                                ));
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Export directory");
                            ui.text_edit_singleline(&mut state.take_export_directory);
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label("Take / branch name");
                        ui.text_edit_singleline(&mut state.take_name_input);
                        if ui.button("Start named take").clicked() {
                            actions.push(UiAction::StartNamedTake);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Deterministic seed");
                        ui.text_edit_singleline(&mut state.random_seed_scope);
                        ui.add(egui::DragValue::new(&mut state.random_seed_value));
                        if ui.button("Set seed").clicked() {
                            actions.push(UiAction::SetRandomSeed);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Timeline marker");
                        ui.text_edit_singleline(&mut state.timeline_marker_input);
                        if ui.button("Add marker").clicked() {
                            actions.push(UiAction::AddTimelineMarker);
                        }
                    });
                    if let Some(entry) = metrics
                        .session_recoveries
                        .get(state.session_recovery_selected)
                    {
                        ui.label(format!(
                            "{} · {} command(s) · {:.1}s{}{}{}",
                            entry.take_name,
                            entry.command_count,
                            entry.latest_time.monotonic_ns as f64 / 1_000_000_000.0,
                            if entry.checkpointed { " · checkpoint" } else { "" },
                            if entry.ignored_partial_tail {
                                " · torn tail ignored"
                            } else {
                                ""
                            },
                            if entry.project_linked {
                                " · project linked"
                            } else {
                                " · legacy/unlinked"
                            }
                        ));
                        let maximum = entry.latest_time.monotonic_ns as f64 / 1_000_000_000.0;
                        state.session_replay_seconds =
                            state.session_replay_seconds.clamp(0.0, maximum);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Slider::new(
                                    &mut state.session_replay_seconds,
                                    0.0..=maximum.max(0.001),
                                )
                                .text("timeline seconds"),
                            );
                            if ui.button("Restore cursor as branch").clicked() {
                                actions.push(UiAction::RestoreSessionRecoveryAt {
                                    index: state.session_recovery_selected,
                                    monotonic_ns: (state.session_replay_seconds
                                        * 1_000_000_000.0)
                                        .round()
                                        as u64,
                                });
                            }
                        });
                        if !entry.markers().is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Markers");
                                for marker in entry.markers() {
                                    let seconds =
                                        marker.at.monotonic_ns as f64 / 1_000_000_000.0;
                                    if ui
                                        .small_button(format!("{} · {:.1}s", marker.label, seconds))
                                        .clicked()
                                    {
                                        state.session_replay_seconds = seconds;
                                    }
                                }
                            });
                        }
                    }
                    ui.weak(metrics.session_recovery_status);
                    ui.weak(
                        "Restore starts a fresh journal and applies recovered mixer, output, effect and modulation state.",
                    );
                    ui.weak(
                        "Load the matching project first so recovered clip launches resolve against the same media slots.",
                    );
                });
            ui.horizontal(|ui| {
                ui.label("Camera");
                egui::ComboBox::from_id_salt("camera-device")
                    .selected_text(
                        metrics
                            .cameras
                            .iter()
                            .find(|camera| camera.id == state.camera_device_id)
                            .map_or(state.camera_device_id.as_str(), |camera| {
                                camera.label.as_str()
                            }),
                    )
                    .show_ui(ui, |ui| {
                        for camera in metrics.cameras {
                            ui.selectable_value(
                                &mut state.camera_device_id,
                                camera.id.clone(),
                                &camera.label,
                            );
                        }
                    });
                ui.add_sized(
                    [90.0, 22.0],
                    egui::TextEdit::singleline(&mut state.camera_device_id).hint_text("device ID"),
                );
                ui.add(
                    egui::DragValue::new(&mut state.camera_width)
                        .range(160..=7680)
                        .suffix("w"),
                );
                ui.add(
                    egui::DragValue::new(&mut state.camera_height)
                        .range(120..=4320)
                        .suffix("h"),
                );
                ui.add(
                    egui::DragValue::new(&mut state.camera_fps)
                        .range(1..=240)
                        .suffix(" fps"),
                );
                if ui.button("Refresh").clicked() {
                    actions.push(UiAction::RefreshCameras);
                }
                if ui
                    .button(format!("Connect to Deck {}", mixer.selected().label()))
                    .clicked()
                {
                    let label = metrics
                        .cameras
                        .iter()
                        .find(|camera| camera.id == state.camera_device_id)
                        .map_or_else(
                            || format!("Camera {}", state.camera_device_id),
                            |camera| camera.label.clone(),
                        );
                    actions.push(UiAction::ConnectCamera {
                        deck: mixer.selected(),
                        device_id: state.camera_device_id.clone(),
                        label,
                        extent: [state.camera_width, state.camera_height],
                        fps: state.camera_fps,
                    });
                }
                if !metrics.camera_status.is_empty() {
                    ui.weak(metrics.camera_status);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Audio");
                let selected = metrics
                    .audio_inputs
                    .iter()
                    .find(|device| device.id == state.audio_device_id)
                    .map_or("No input selected", |device| device.label.as_str());
                egui::ComboBox::from_id_salt("audio-input-device")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for device in metrics.audio_inputs {
                            ui.selectable_value(
                                &mut state.audio_device_id,
                                device.id.clone(),
                                if device.is_default {
                                    format!("{} · default", device.label)
                                } else {
                                    device.label.clone()
                                },
                            );
                        }
                    });
                if ui.button("Refresh audio").clicked() {
                    actions.push(UiAction::RefreshAudioInputs);
                }
                if metrics.audio_connected {
                    if ui.button("Disconnect").clicked() {
                        actions.push(UiAction::DisconnectAudioInput);
                    }
                } else if ui
                    .add_enabled(
                        !state.audio_device_id.is_empty(),
                        egui::Button::new("Connect"),
                    )
                    .clicked()
                {
                    actions.push(UiAction::ConnectAudioInput(state.audio_device_id.clone()));
                }
                ui.weak(metrics.audio_status);
            });
            egui::CollapsingHeader::new("Audio analysis")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Slider::new(&mut state.audio_analysis.gain, 0.0..=16.0)
                                .text("gain"),
                        );
                        ui.add(
                            egui::Slider::new(&mut state.audio_analysis.noise_floor, 0.0..=0.5)
                                .text("noise floor"),
                        );
                        ui.add(
                            egui::Slider::new(&mut state.audio_analysis.attack_ms, 1.0..=2_000.0)
                                .text("attack ms")
                                .logarithmic(true),
                        );
                        ui.add(
                            egui::Slider::new(&mut state.audio_analysis.release_ms, 1.0..=5_000.0)
                                .text("release ms")
                                .logarithmic(true),
                        );
                        ui.add(
                            egui::Slider::new(
                                &mut state.audio_analysis.transient_sensitivity,
                                0.0..=16.0,
                            )
                            .text("transient"),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(
                            &mut state.audio_analysis.normalization,
                            "Adaptive normalization",
                        );
                        ui.add_enabled_ui(state.audio_analysis.normalization, |ui| {
                            ui.add(
                                egui::Slider::new(
                                    &mut state.audio_analysis.normalization_target,
                                    0.05..=1.0,
                                )
                                .text("target RMS"),
                            );
                            ui.add(
                                egui::Slider::new(
                                    &mut state.audio_analysis.normalization_speed_ms,
                                    10.0..=10_000.0,
                                )
                                .text("adapt ms")
                                .logarithmic(true),
                            );
                        });
                    });
                    state.audio_analysis = state.audio_analysis.sanitized();
                    let snapshot = metrics.audio_snapshot;
                    ui.horizontal(|ui| {
                        for (label, value) in [
                            ("RMS", snapshot.analysis.rms),
                            ("Bass", snapshot.analysis.bass),
                            ("Mid", snapshot.analysis.mid),
                            ("High", snapshot.analysis.high),
                            ("Transient", snapshot.analysis.transient),
                        ] {
                            ui.add(
                                egui::ProgressBar::new(value)
                                    .text(format!("{label} {value:.2}"))
                                    .desired_width(120.0),
                            );
                        }
                    });
                    ui.weak(format!(
                        "{} Hz · {} channel(s) · queue overruns {} · callback errors {}",
                        snapshot.sample_rate,
                        snapshot.channels,
                        snapshot.queue_overruns,
                        snapshot.callback_errors
                    ));
                });
            draw_midi(ui, state, &mut metrics.midi, actions);
            egui::CollapsingHeader::new("OSC input")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("UDP bind");
                        ui.add_enabled(
                            !metrics.osc.connected,
                            egui::TextEdit::singleline(&mut state.osc_bind_address)
                                .desired_width(170.0),
                        );
                        if metrics.osc.connected {
                            if ui.button("Disconnect").clicked() {
                                actions.push(UiAction::DisconnectOscInput);
                            }
                        } else if ui.button("Listen").clicked() {
                            actions.push(UiAction::ConnectOscInput);
                        }
                        ui.weak(metrics.osc.status);
                    });
                    ui.weak(format!(
                        "packets {} · messages {} · malformed {} · dropped {} · scheduled {} · schedule drops {}",
                        metrics.osc.stats.packets,
                        metrics.osc.stats.messages,
                        metrics.osc.stats.malformed,
                        metrics.osc.stats.dropped,
                        metrics.osc.pending,
                        metrics.osc.schedule_dropped
                    ));
                    ui.horizontal(|ui| {
                        ui.label("Feedback target");
                        ui.add_enabled(
                            !metrics.osc.output_connected,
                            egui::TextEdit::singleline(&mut state.osc_feedback_address)
                                .desired_width(170.0),
                        );
                        if metrics.osc.output_connected {
                            if ui.button("Stop feedback").clicked() {
                                actions.push(UiAction::DisconnectOscOutput);
                            }
                        } else if ui.button("Send feedback").clicked() {
                            actions.push(UiAction::ConnectOscOutput);
                        }
                        ui.weak(metrics.osc.output_status);
                    });
                    ui.weak(format!(
                        "feedback sent {} · dropped {} · errors {}",
                        metrics.osc.output_stats.sent,
                        metrics.osc.output_stats.dropped,
                        metrics.osc.output_stats.errors
                    ));
                    ui.weak("Routes use /vjx; deck and clip numbers are 1-based.");
                });
            ui.separator();

            draw_runtime_summary(ui, state, metrics);

            ui.separator();
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut state.bpm)
                        .range(20.0..=400.0)
                        .speed(0.25)
                        .suffix(" BPM"),
                );
                let tap =
                    mappable(ui, midi_map, ControlTarget::TapTempo, actions, |ui| {
                        ui.button("Tap")
                    });
                if tap.clicked() {
                    actions.push(UiAction::TapTempo);
                }
                if ui.button("½").on_hover_text("Half tempo").clicked() {
                    actions.push(UiAction::HalfTempo);
                }
                if ui.button("×2").on_hover_text("Double tempo").clicked() {
                    actions.push(UiAction::DoubleTempo);
                }
                ui.selectable_value(
                    &mut state.quantization,
                    Quantization::Immediate,
                    "Immediate",
                );
                ui.selectable_value(&mut state.quantization, Quantization::Beat, "Next beat");
                ui.selectable_value(&mut state.quantization, Quantization::Bar, "Next bar");
                ui.separator();
                ui.label(format!(
                    "beat {:.2} · phase {:.2} · bar {:.2}",
                    metrics.tempo.beat_at(metrics.now_seconds),
                    metrics.tempo.beat_phase(metrics.now_seconds),
                    metrics.tempo.bar_phase(metrics.now_seconds)
                ));
                draw_pipeline_health(ui, metrics);
            });
                });
        });
    }
}
