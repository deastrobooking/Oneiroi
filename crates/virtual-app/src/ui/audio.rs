//! Audio input, 8-band spectrum EQ and band-to-control mappings.

use super::*;
use virtual_core::{
    AUDIO_MAP_LEVEL, AUDIO_MAP_TRANSIENT, MAX_AUDIO_BINDINGS, default_output_range,
};

pub(super) struct AudioPanelContext<'a> {
    pub inputs: &'a [AudioInputDevice],
    pub status: &'a str,
    pub connected: bool,
    pub snapshot: AudioInputSnapshot,
    pub palette: ThemePalette,
}

pub(super) fn draw_audio_panel(
    ui: &mut egui::Ui,
    state: &mut UiState,
    context: AudioPanelContext<'_>,
    actions: &mut Vec<UiAction>,
) {
    decay_peaks(ui, state, &context.snapshot.analysis.bands);
    let id = ui.make_persistent_id("audio-spectrum-panel");
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
        .show_header(ui, |ui| {
            ui.label(
                egui::RichText::new("AUDIO SPECTRUM")
                    .strong()
                    .color(context.palette.accent),
            );
            mini_spectrum(ui, &context.snapshot.analysis.bands, context.connected);
            let mapped = state
                .audio_map
                .bindings
                .iter()
                .filter(|binding| binding.enabled)
                .count();
            if context.connected {
                ui.weak(format!("{mapped} mapping(s) live"));
            } else {
                ui.colored_label(context.palette.warning, "input not connected");
            }
        })
        .body(|ui| {
            draw_input_row(ui, state, &context, actions);
            ui.separator();
            draw_spectrum(ui, state, &context);
            ui.separator();
            draw_response(ui, state, &context);
            ui.separator();
            draw_mappings(ui, state, &context);
        });
    if let Some(learn) = state.audio_learn {
        ui.horizontal(|ui| {
            ui.colored_label(
                context.palette.success,
                format!(
                    "Mapping {} · click any highlighted control to drive it from this band",
                    audio_map_source_label(learn.source)
                ),
            );
            if ui.button("Cancel").clicked() {
                cancel_learn(state);
            }
        });
    }
}

fn decay_peaks(ui: &egui::Ui, state: &mut UiState, bands: &[f32; SPECTRUM_BANDS]) {
    let dt = ui.input(|input| input.stable_dt).clamp(0.0, 0.1);
    for (peak, value) in state.spectrum_peaks.iter_mut().zip(bands) {
        *peak = (*peak - dt * 0.6).max(*value).clamp(0.0, 1.0);
    }
}

fn band_color(band: usize) -> egui::Color32 {
    let hue = band as f32 / SPECTRUM_BANDS as f32 * 0.78;
    egui::ecolor::Hsva::new(hue, 0.72, 0.95, 1.0).into()
}

fn mini_spectrum(ui: &mut egui::Ui, bands: &[f32; SPECTRUM_BANDS], connected: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(96.0, 16.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let width = rect.width() / SPECTRUM_BANDS as f32;
    for (band, value) in bands.iter().enumerate() {
        let value = if connected {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let left = rect.left() + band as f32 * width;
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(left + 1.0, rect.bottom() - value * rect.height()),
                egui::pos2(left + width - 1.0, rect.bottom()),
            ),
            1.0,
            band_color(band),
        );
    }
}

fn draw_input_row(
    ui: &mut egui::Ui,
    state: &mut UiState,
    context: &AudioPanelContext<'_>,
    actions: &mut Vec<UiAction>,
) {
    let selected_device = context
        .inputs
        .iter()
        .find(|device| device.id == state.audio_device_id);
    ui.horizontal_wrapped(|ui| {
        ui.label("Input");
        egui::ComboBox::from_id_salt("audio-input-device")
            .selected_text(
                selected_device.map_or("No input selected", |device| device.label.as_str()),
            )
            .width(240.0)
            .show_ui(ui, |ui| {
                for device in context.inputs {
                    let mut label = device.label.clone();
                    if device.channels > 0 {
                        label.push_str(&format!(" · {} ch", device.channels));
                    }
                    if device.is_default {
                        label.push_str(" · default");
                    }
                    if ui
                        .selectable_value(&mut state.audio_device_id, device.id.clone(), label)
                        .clicked()
                    {
                        state.audio_channel = None;
                    }
                }
            });
        let channels = selected_device.map_or(0, |device| device.channels);
        let previous_channel = state.audio_channel;
        egui::ComboBox::from_id_salt("audio-input-channel")
            .selected_text(match state.audio_channel {
                Some(channel) => format!("Channel {}", channel + 1),
                None => "All channels (mono mix)".to_owned(),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.audio_channel, None, "All channels (mono mix)");
                for channel in 0..channels {
                    ui.selectable_value(
                        &mut state.audio_channel,
                        Some(channel),
                        format!("Channel {}", channel + 1),
                    );
                }
            })
            .response
            .on_hover_text("Pick one input of a multi-channel interface, or mix them all");
        if context.connected && state.audio_channel != previous_channel {
            actions.push(UiAction::ConnectAudioInput(state.audio_device_id.clone()));
        }
        if ui.button("Refresh").clicked() {
            actions.push(UiAction::RefreshAudioInputs);
        }
        if context.connected {
            if ui.button("Disconnect").clicked() {
                actions.push(UiAction::DisconnectAudioInput);
            }
        } else if ui
            .add_enabled(
                !state.audio_device_id.is_empty(),
                egui::Button::new("Connect")
                    .fill(context.palette.control_tint(context.palette.success, 0.3)),
            )
            .clicked()
        {
            actions.push(UiAction::ConnectAudioInput(state.audio_device_id.clone()));
        }
        ui.weak(context.status);
    });
    let snapshot = context.snapshot;
    if context.connected {
        ui.weak(format!(
            "{} Hz · {} channel(s) · analysing {} · queue overruns {} · callback errors {}",
            snapshot.sample_rate,
            snapshot.channels,
            snapshot.input_channel.map_or_else(
                || "mono mix".to_owned(),
                |channel| format!("channel {}", channel + 1)
            ),
            snapshot.queue_overruns,
            snapshot.callback_errors
        ));
    } else {
        ui.weak(
            "Pick the built-in microphone or an audio interface. macOS asks for microphone access the first time.",
        );
    }
}

fn draw_spectrum(ui: &mut egui::Ui, state: &mut UiState, context: &AudioPanelContext<'_>) {
    let bands = context.snapshot.analysis.bands;
    let decibels = state.audio_analysis.spectrum_decibels;
    let range_db = state.audio_analysis.spectrum_range_db;
    let width = ui.available_width().clamp(420.0, 880.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            ui.columns(SPECTRUM_BANDS, |columns| {
                for (band, ui) in columns.iter_mut().enumerate() {
                    ui.vertical_centered(|ui| {
                        let thresholds: Vec<f32> = state
                            .audio_map
                            .bindings
                            .iter()
                            .filter(|binding| {
                                binding.enabled
                                    && usize::from(binding.source) == band
                                    && binding.mode != AudioMapMode::Continuous
                            })
                            .map(|binding| {
                                let [low, high] = binding.input_range;
                                low + binding.threshold * (high - low)
                            })
                            .collect();
                        spectrum_bar(
                            ui,
                            band,
                            if context.connected { bands[band] } else { 0.0 },
                            state.spectrum_peaks[band],
                            &thresholds,
                            decibels.then_some(range_db),
                        );
                        ui.label(
                            egui::RichText::new(SPECTRUM_BAND_LABELS[band])
                                .strong()
                                .small(),
                        );
                        ui.label(egui::RichText::new(band_range_label(band)).weak().small());
                        let gain = &mut state.audio_analysis.band_gains_db[band];
                        ui.spacing_mut().slider_width = 80.0;
                        let response = ui
                            .add(
                                egui::Slider::new(gain, -24.0..=24.0)
                                    .vertical()
                                    .step_by(0.5)
                                    .suffix(" dB")
                                    .show_value(true),
                            )
                            .on_hover_text("Band gain · double-click to reset");
                        if response.double_clicked() {
                            *gain = 0.0;
                        }
                        map_button(ui, state, band as u8, context.palette);
                    });
                }
            });
        },
    );
    ui.horizontal_wrapped(|ui| {
        for source in [AUDIO_MAP_LEVEL, AUDIO_MAP_TRANSIENT] {
            let value = if source == AUDIO_MAP_LEVEL {
                context.snapshot.analysis.rms
            } else {
                context.snapshot.analysis.transient
            };
            ui.add(
                egui::ProgressBar::new(if context.connected { value } else { 0.0 })
                    .text(format!("{} {value:.2}", audio_map_source_label(source)))
                    .desired_width(150.0),
            );
            map_button(ui, state, source, context.palette);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Scale");
        ui.selectable_value(&mut state.audio_analysis.spectrum_decibels, true, "dB");
        ui.selectable_value(&mut state.audio_analysis.spectrum_decibels, false, "Linear");
        ui.add_enabled(
            state.audio_analysis.spectrum_decibels,
            egui::Slider::new(&mut state.audio_analysis.spectrum_range_db, 12.0..=96.0)
                .suffix(" dB")
                .text("range"),
        )
        .on_hover_text("How far below full scale a band reads as zero");
        if ui.button("Flat EQ").clicked() {
            state.audio_analysis.band_gains_db = [0.0; SPECTRUM_BANDS];
        }
        if ui
            .button("Music tilt")
            .on_hover_text("Lift the quieter upper bands so each band moves on typical music")
            .clicked()
        {
            state.audio_analysis.band_gains_db = [-3.0, -3.0, 0.0, 2.0, 4.0, 6.0, 8.0, 10.0];
        }
    });
}

fn band_range_label(band: usize) -> String {
    let format = |hz: f32| {
        if hz >= 1_000.0 {
            format!("{}k", hz / 1_000.0)
        } else {
            format!("{hz}")
        }
    };
    format!(
        "{}–{} Hz",
        format(SPECTRUM_BAND_EDGES_HZ[band]),
        format(SPECTRUM_BAND_EDGES_HZ[band + 1])
    )
}

fn spectrum_bar(
    ui: &mut egui::Ui,
    band: usize,
    value: f32,
    peak: f32,
    thresholds: &[f32],
    range_db: Option<f32>,
) {
    let width = (ui.available_width() - 6.0).clamp(24.0, 90.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 150.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
    let grid = egui::Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.25));
    let y_for = |value: f32| rect.bottom() - value.clamp(0.0, 1.0) * rect.height();
    if let Some(range_db) = range_db {
        let mut db = -12.0;
        while db > -range_db {
            let y = y_for((db + range_db) / range_db);
            painter.hline(rect.x_range(), y, grid);
            db -= 12.0;
        }
    } else {
        for quarter in 1..4 {
            painter.hline(rect.x_range(), y_for(quarter as f32 * 0.25), grid);
        }
    }
    let color = band_color(band);
    let fill = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 3.0, y_for(value)),
        egui::pos2(rect.right() - 3.0, rect.bottom()),
    );
    painter.rect_filled(fill, 2.0, color.gamma_multiply(0.85));
    painter.hline(
        (rect.left() + 3.0)..=(rect.right() - 3.0),
        y_for(peak),
        egui::Stroke::new(2.0, color),
    );
    for threshold in thresholds {
        let y = y_for(*threshold);
        painter.hline(
            rect.x_range(),
            y,
            egui::Stroke::new(1.5, visuals.strong_text_color()),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(rect.left(), y - 4.0),
                egui::pos2(rect.left() + 6.0, y),
                egui::pos2(rect.left(), y + 4.0),
            ],
            visuals.strong_text_color(),
            egui::Stroke::NONE,
        ));
    }
    let readout = match range_db {
        Some(range_db) => format!("{:.0} dB", value * range_db - range_db),
        None => format!("{value:.2}"),
    };
    response.on_hover_text(format!(
        "{} · {} · {readout} · peak {peak:.2}",
        SPECTRUM_BAND_LABELS[band],
        band_range_label(band)
    ));
}

fn map_button(ui: &mut egui::Ui, state: &mut UiState, source: u8, palette: ThemePalette) {
    let armed = state
        .audio_learn
        .is_some_and(|learn| learn.source == source);
    let count = state
        .audio_map
        .bindings
        .iter()
        .filter(|binding| binding.source == source)
        .count();
    let label = match (armed, count) {
        (true, _) => "Cancel".to_owned(),
        (false, 0) => "Map".to_owned(),
        (false, count) => format!("Map · {count}"),
    };
    let button = egui::Button::new(label).selected(armed).fill(if armed {
        palette.control_tint(palette.success, 0.4)
    } else {
        palette.control
    });
    if ui
        .add(button)
        .on_hover_text(if armed {
            "Stop mapping".to_owned()
        } else {
            format!(
                "Click, then click any control to drive it from {}",
                audio_map_source_label(source)
            )
        })
        .clicked()
    {
        if armed {
            cancel_learn(state);
        } else {
            let restore_map_mode_off = match state.audio_learn {
                Some(learn) => learn.restore_map_mode_off,
                None => !state.midi_map_mode,
            };
            state.audio_learn = Some(AudioLearn {
                source,
                restore_map_mode_off,
            });
            state.midi_map_mode = true;
        }
    }
}

pub(super) fn cancel_learn(state: &mut UiState) {
    if let Some(learn) = state.audio_learn.take()
        && learn.restore_map_mode_off
    {
        state.midi_map_mode = false;
    }
}

fn draw_response(ui: &mut egui::Ui, state: &mut UiState, context: &AudioPanelContext<'_>) {
    egui::CollapsingHeader::new("Input response · gain, smoothing, normalization")
        .id_salt("audio-response")
        .default_open(false)
        .show(ui, |ui| {
            let analysis = &mut state.audio_analysis;
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::Slider::new(&mut analysis.gain, 0.0..=16.0).text("gain"));
                ui.add(egui::Slider::new(&mut analysis.noise_floor, 0.0..=0.5).text("noise floor"));
                ui.add(
                    egui::Slider::new(&mut analysis.attack_ms, 1.0..=2_000.0)
                        .text("attack ms")
                        .logarithmic(true),
                );
                ui.add(
                    egui::Slider::new(&mut analysis.release_ms, 1.0..=5_000.0)
                        .text("release ms")
                        .logarithmic(true),
                );
                ui.add(
                    egui::Slider::new(&mut analysis.transient_sensitivity, 0.0..=16.0)
                        .text("transient"),
                );
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut analysis.normalization, "Adaptive normalization");
                ui.add_enabled_ui(analysis.normalization, |ui| {
                    ui.add(
                        egui::Slider::new(&mut analysis.normalization_target, 0.05..=1.0)
                            .text("target RMS"),
                    );
                    ui.add(
                        egui::Slider::new(&mut analysis.normalization_speed_ms, 10.0..=10_000.0)
                            .text("adapt ms")
                            .logarithmic(true),
                    );
                });
            });
            let snapshot = context.snapshot.analysis;
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [
                    ("Bass", snapshot.bass),
                    ("Mid", snapshot.mid),
                    ("High", snapshot.high),
                ] {
                    ui.add(
                        egui::ProgressBar::new(value)
                            .text(format!("{label} {value:.2}"))
                            .desired_width(120.0),
                    );
                }
            });
            ui.weak(
                "Attack and release smooth every band. Bands and the three broad meters are also mod-matrix sources on every deck and the master.",
            );
        });
    state.audio_analysis = state.audio_analysis.sanitized();
}

/// Everything an audio binding can drive, with labels, including the
/// parameters of the algorithms currently loaded.
fn mappable_targets(state: &UiState) -> Vec<(ControlTarget, String)> {
    let mut targets = super::midi::midi_targets();
    for (deck, slot) in state.deck_packages.iter().enumerate() {
        if let Some(package) = state
            .deck_effect_packages
            .iter()
            .find(|package| package.id == slot.package_id)
        {
            targets.extend(package.parameters.iter().map(|parameter| {
                ControlTarget::DeckEffectParameter {
                    deck: deck as u8,
                    parameter_key: effect_parameter_key(&package.id, &parameter.id),
                }
            }));
        }
    }
    for (index, slot) in state.master_effects.slots.iter().enumerate() {
        if slot.kind != MasterEffectKind::Custom {
            continue;
        }
        if let Some(package) = state
            .effect_packages
            .iter()
            .find(|package| package.id == slot.package_id)
        {
            targets.extend(package.parameters.iter().map(|parameter| {
                ControlTarget::MasterEffectParameter {
                    slot: index as u8,
                    parameter_key: effect_parameter_key(&package.id, &parameter.id),
                }
            }));
        }
    }
    targets
        .into_iter()
        .map(|target| {
            (
                target,
                super::midi::midi_target_label_for_state(target, state),
            )
        })
        .collect()
}

fn draw_mappings(ui: &mut egui::Ui, state: &mut UiState, context: &AudioPanelContext<'_>) {
    let targets = mappable_targets(state);
    let label_for = |target: ControlTarget| {
        targets
            .iter()
            .find(|(candidate, _)| *candidate == target)
            .map_or_else(
                || super::midi::midi_target_label(target),
                |(_, label)| label.clone(),
            )
    };
    ui.horizontal_wrapped(|ui| {
        ui.strong(format!(
            "Band mappings · {}/{MAX_AUDIO_BINDINGS}",
            state.audio_map.bindings.len()
        ));
        if ui
            .add_enabled(
                state.audio_map.bindings.len() < MAX_AUDIO_BINDINGS,
                egui::Button::new("Add mapping"),
            )
            .clicked()
        {
            state
                .audio_map
                .bindings
                .push(AudioBinding::new(1, ControlTarget::DeckLevel(0)));
        }
        if ui.button("Mute all").clicked() {
            for binding in &mut state.audio_map.bindings {
                binding.enabled = false;
            }
        }
        if ui.button("Clear all").clicked() {
            state.audio_map.bindings.clear();
        }
    });
    if state.audio_map.bindings.is_empty() {
        ui.weak(
            "Press Map under a band, then click any control, or use Add mapping. Continuous follows the band; Trigger fires on each hit; Gate holds while the band is above the threshold.",
        );
        return;
    }
    let sources = audio_map_sources(&context.snapshot.analysis);
    let mut remove = None;
    egui::ScrollArea::horizontal()
        .id_salt("audio-mapping-scroll")
        .show(ui, |ui| {
            egui::Grid::new("audio-mappings")
                .num_columns(10)
                .striped(true)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    for heading in [
                        "On",
                        "Band",
                        "Control",
                        "Mode",
                        "Threshold",
                        "Band in",
                        "Output",
                        "Inv",
                        "Live",
                        "",
                    ] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for (index, binding) in state.audio_map.bindings.iter_mut().enumerate() {
                        ui.checkbox(&mut binding.enabled, "");
                        egui::ComboBox::from_id_salt(("audio-map-source", index))
                            .selected_text(audio_map_source_label(binding.source))
                            .width(96.0)
                            .show_ui(ui, |ui| {
                                for source in 0..AUDIO_MAP_SOURCES as u8 {
                                    ui.selectable_value(
                                        &mut binding.source,
                                        source,
                                        audio_map_source_label(source),
                                    );
                                }
                            });
                        let previous_target = binding.target;
                        egui::ComboBox::from_id_salt(("audio-map-target", index))
                            .selected_text(label_for(binding.target))
                            .width(220.0)
                            .height(360.0)
                            .show_ui(ui, |ui| {
                                for (target, label) in &targets {
                                    ui.selectable_value(&mut binding.target, *target, label);
                                }
                            });
                        if binding.target != previous_target {
                            binding.mode = AudioMapMode::default_for(binding.target);
                            binding.output_range = default_output_range(binding.target);
                            binding.reset();
                        }
                        egui::ComboBox::from_id_salt(("audio-map-mode", index))
                            .selected_text(binding.mode.label())
                            .width(92.0)
                            .show_ui(ui, |ui| {
                                for mode in AudioMapMode::ALL {
                                    if ui
                                        .selectable_value(&mut binding.mode, mode, mode.label())
                                        .changed()
                                    {
                                        binding.reset();
                                    }
                                }
                            });
                        ui.add_enabled(
                            binding.mode != AudioMapMode::Continuous,
                            egui::Slider::new(&mut binding.threshold, 0.0..=1.0).show_value(true),
                        );
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut binding.input_range[0])
                                    .range(0.0..=1.0)
                                    .speed(0.01),
                            );
                            ui.add(
                                egui::DragValue::new(&mut binding.input_range[1])
                                    .range(0.0..=1.0)
                                    .speed(0.01),
                            );
                        })
                        .response
                        .on_hover_text("Band span mapped onto the full output");
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut binding.output_range[0]).speed(0.01));
                            ui.add(egui::DragValue::new(&mut binding.output_range[1]).speed(0.01));
                        });
                        ui.checkbox(&mut binding.invert, "");
                        let live = binding.normalized(&sources);
                        let lit =
                            binding.mode == AudioMapMode::Continuous || live >= binding.threshold;
                        ui.add(
                            egui::ProgressBar::new(if context.connected { live } else { 0.0 })
                                .desired_width(70.0)
                                .fill(if lit {
                                    band_color(usize::from(binding.source).min(SPECTRUM_BANDS - 1))
                                } else {
                                    context.palette.idle
                                }),
                        );
                        if ui
                            .small_button("✕")
                            .on_hover_text("Remove mapping")
                            .clicked()
                        {
                            remove = Some(index);
                        }
                        ui.end_row();
                    }
                });
        });
    if let Some(index) = remove {
        state.audio_map.bindings.remove(index);
    }
}
