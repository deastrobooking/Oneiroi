//! Per-deck controls: transport, transform, effects, LFOs and modulation.

use super::*;

pub(super) struct DeckControls<'a> {
    pub(super) accent: egui::Color32,
    pub(super) transport: &'a mut DeckTransport,
    pub(super) transform: &'a mut DeckTransform,
    pub(super) blend_mode: &'a mut LayerBlendMode,
    pub(super) solo: &'a mut bool,
    pub(super) bypassed: &'a mut bool,
    pub(super) effects: &'a mut DeckEffects,
    pub(super) lfos: &'a mut DeckLfos,
}

pub(super) fn draw_deck(
    ui: &mut egui::Ui,
    mixer: &mut FourDeckMixer,
    id: DeckId,
    controls: DeckControls<'_>,
    actions: &mut Vec<UiAction>,
) {
    let DeckControls {
        accent,
        transport,
        transform,
        blend_mode,
        solo,
        bypassed,
        effects,
        lfos,
    } = controls;
    let selected = mixer.selected() == id;
    let frame = egui::Frame::group(ui.style())
        .fill(if selected {
            accent.linear_multiply(0.12)
        } else {
            ui.visuals().faint_bg_color
        })
        .stroke(egui::Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected {
                accent
            } else {
                ui.visuals().window_stroke.color
            },
        ))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(10.0);

    frame.show(ui, |ui| {
        // 340 keeps a strip usable inside the cascade's 380 px column; the
        // grid cells simply grow past it.
        ui.set_min_size([340.0, 165.0].into());
        // Channel banding: every strip carries its deck colour whether or not
        // it is selected, so an operator can find deck C without reading.
        let (band, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 4.0), egui::Sense::hover());
        ui.painter().rect_filled(band, 2.0, accent);
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new(format!("DECK {}", id.label()))
                            .strong()
                            .color(if selected {
                                accent
                            } else {
                                egui::Color32::WHITE
                            }),
                    )
                    .selected(selected),
                )
                .clicked()
            {
                mixer.select(id);
            }
            ui.weak(if selected {
                "drop target"
            } else {
                "click to target"
            });
            let eject_enabled = !matches!(mixer.deck(id).state, DeckState::Empty);
            if ui
                .add_enabled(eject_enabled, egui::Button::new("Eject"))
                .clicked()
            {
                actions.push(UiAction::Eject(id));
            }
        });
        ui.separator();

        match &mixer.deck(id).state {
            DeckState::Empty => {
                ui.label("Empty");
                ui.weak("Select this deck and drop MOV, MP4, MKV, AVI, WebM, or MXF footage.");
            }
            DeckState::Loading { path } => {
                ui.spinner();
                ui.label(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Loading movie…"),
                );
                ui.weak("Probing codec and performance metadata…");
            }
            DeckState::Ready(movie) => {
                ui.strong(&movie.display_name);
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} × {}",
                        movie.visible_extent[0], movie.visible_extent[1]
                    ));
                    ui.label(movie.codec.to_uppercase());
                    if let Some(rate) = movie.frame_rate {
                        ui.label(format!(
                            "{:.2} fps",
                            rate.numerator as f64 / rate.denominator as f64
                        ));
                    }
                    if let Some(duration) = movie.duration {
                        ui.label(format!("{:.1}s", duration.as_seconds()));
                    }
                    if movie.decode_path == oneiroi_media::DecodePath::FfmpegVideo {
                        ui.label(format!("{} keys", movie.keyframes.len()));
                    }
                });
                let (label, color) = match movie.health {
                    MediaHealth::StageReady => ("STAGE READY", egui::Color32::LIGHT_GREEN),
                    MediaHealth::Usable => ("USABLE", egui::Color32::from_rgb(130, 210, 255)),
                    MediaHealth::Caution => ("CAUTION", egui::Color32::YELLOW),
                    MediaHealth::Problem => ("PROBLEM", egui::Color32::LIGHT_RED),
                };
                ui.colored_label(color, label);
                ui.weak(&movie.health_reason);
            }
            DeckState::Live(config) => {
                ui.colored_label(egui::Color32::LIGHT_GREEN, "● LIVE CAMERA");
                ui.strong(&config.device.label);
                ui.horizontal(|ui| {
                    ui.label(config.device.backend.to_uppercase());
                    if let Some([width, height]) = config.requested_extent {
                        ui.label(format!("{width} × {height}"));
                    }
                    if let Some(fps) = config.requested_fps {
                        ui.label(format!("{fps} fps requested"));
                    }
                });
                ui.weak("Non-seekable low-latency source");
            }
            DeckState::Error { path, message } => {
                ui.colored_label(egui::Color32::LIGHT_RED, "IMPORT ERROR");
                ui.label(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Unknown file"),
                );
                ui.weak(message);
            }
        }

        let deck = mixer.deck_mut(id);
        ui.horizontal(|ui| {
            ui.add(
                egui::Slider::new(&mut deck.level, 0.0..=1.0)
                    .text("level")
                    .clamping(egui::SliderClamping::Always),
            );
            ui.selectable_value(&mut deck.bus, CrossfadeBus::Left, "Bus A");
            ui.selectable_value(&mut deck.bus, CrossfadeBus::Right, "Bus B");
        });
        ui.horizontal(|ui| {
            if ui.selectable_label(*solo, "Solo").clicked() {
                *solo = !*solo;
            }
            if ui.selectable_label(*bypassed, "Bypass").clicked() {
                *bypassed = !*bypassed;
            }
            egui::ComboBox::from_id_salt(format!("blend-mode-{}", id.label()))
                .selected_text(blend_mode_label(*blend_mode))
                .show_ui(ui, |ui| {
                    ui.set_min_width(190.0);
                    for group in BlendModeGroup::ALL {
                        ui.label(egui::RichText::new(group.label()).weak().small());
                        for mode in LayerBlendMode::ALL
                            .into_iter()
                            .filter(|mode| mode.group() == group)
                        {
                            ui.selectable_value(blend_mode, mode, blend_mode_label(mode))
                                .on_hover_text(mode.hint());
                        }
                        ui.separator();
                    }
                });
            if *bypassed {
                ui.weak("Layer excluded from composition");
            } else if *solo {
                ui.weak("Other non-solo decks isolated");
            }
        });
        let live = matches!(mixer.deck(id).state, DeckState::Live(_));
        if live {
            ui.horizontal(|ui| {
                ui.checkbox(&mut transport.frozen, "Freeze live frame");
                ui.weak("seek, loop and speed are unavailable for cameras");
            });
        } else {
            ui.horizontal(|ui| {
                if ui
                    .button(if transport.playing { "Pause" } else { "Play" })
                    .clicked()
                {
                    transport.playing = !transport.playing;
                }
                if ui.button("Restart").clicked() {
                    transport.restart();
                    actions.push(UiAction::Restart(id));
                }
                ui.checkbox(&mut transport.frozen, "Freeze");
                let mut looping = transport.end_mode == EndMode::Loop;
                if ui.checkbox(&mut looping, "Loop").changed() {
                    transport.end_mode = if looping {
                        EndMode::Loop
                    } else {
                        EndMode::OneShot
                    };
                }
                ui.add(
                    egui::Slider::new(&mut transport.speed, 0.25..=4.0)
                        .text("speed")
                        .logarithmic(true),
                );
            });
        }
        if let Some(duration) = transport.duration.filter(|duration| *duration > 0.0) {
            let range = (duration - transport.in_point).max(f64::EPSILON);
            let mut progress =
                ((transport.position - transport.in_point) / range).clamp(0.0, 1.0) as f32;
            if ui
                .add(egui::Slider::new(&mut progress, 0.0..=1.0).text("playhead"))
                .changed()
            {
                transport.seek_normalized(progress);
                actions.push(UiAction::Seek(id));
            }
        }
        egui::CollapsingHeader::new("Layer transform")
            .id_salt(format!("transform-{}", id.label()))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Source mode");
                    ui.selectable_value(&mut transform.source_mode, SourceMode::Fit, "Fit");
                    ui.selectable_value(&mut transform.source_mode, SourceMode::Fill, "Fill");
                    ui.selectable_value(&mut transform.source_mode, SourceMode::Stretch, "Stretch");
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(&mut transform.position[0], -2.0..=2.0)
                            .text("position X"),
                    );
                    ui.add(
                        egui::Slider::new(&mut transform.position[1], -2.0..=2.0)
                            .text("position Y"),
                    );
                });
                ui.add(
                    egui::Slider::new(&mut transform.scale, 0.05..=4.0)
                        .text("scale")
                        .logarithmic(true),
                );
                let mut degrees = transform.rotation * 360.0;
                if ui
                    .add(egui::Slider::new(&mut degrees, -360.0..=360.0).text("rotation°"))
                    .changed()
                {
                    transform.rotation = degrees / 360.0;
                }
                ui.horizontal(|ui| {
                    ui.checkbox(&mut transform.flip_horizontal, "Flip horizontal");
                    ui.checkbox(&mut transform.flip_vertical, "Flip vertical");
                    if ui.button("Reset transform").clicked() {
                        *transform = DeckTransform::default();
                    }
                });
                ui.label("Crop");
                ui.columns(2, |columns| {
                    columns[0]
                        .add(egui::Slider::new(&mut transform.crop[0], 0.0..=0.95).text("left"));
                    columns[0]
                        .add(egui::Slider::new(&mut transform.crop[1], 0.0..=0.95).text("right"));
                    columns[1]
                        .add(egui::Slider::new(&mut transform.crop[2], 0.0..=0.95).text("top"));
                    columns[1]
                        .add(egui::Slider::new(&mut transform.crop[3], 0.0..=0.95).text("bottom"));
                });
                *transform = transform.sanitized();
            });
        egui::CollapsingHeader::new("GPU effects")
            .id_salt(format!("effects-{}", id.label()))
            .show(ui, |ui| {
                ui.strong("Effect chain");
                let mut reorder = None;
                let slot_count = effects.slots.len();
                for index in 0..slot_count {
                    ui.horizontal(|ui| {
                        let slot = &mut effects.slots[index];
                        ui.monospace(format!("{}", index + 1));
                        ui.label(slot.group.label());
                        ui.checkbox(&mut slot.bypassed, "Bypass");
                        ui.add(
                            egui::Slider::new(&mut slot.mix, 0.0..=1.0)
                                .text("wet")
                                .show_value(true),
                        );
                        if ui
                            .add_enabled(index > 0, egui::Button::new("↑"))
                            .on_hover_text("Move earlier")
                            .clicked()
                        {
                            reorder = Some((index, index - 1));
                        }
                        if ui
                            .add_enabled(index + 1 < slot_count, egui::Button::new("↓"))
                            .on_hover_text("Move later")
                            .clicked()
                        {
                            reorder = Some((index, index + 1));
                        }
                    });
                }
                if let Some((from, to)) = reorder {
                    effects.slots.swap(from, to);
                }
                ui.horizontal(|ui| {
                    ui.menu_button("Load preset", |ui| {
                        for preset in EffectPreset::ALL {
                            if ui.button(preset.label()).clicked() {
                                *effects = DeckEffects::preset(preset);
                                ui.close();
                            }
                        }
                    });
                    if ui.button("Reset chain").clicked() {
                        *effects = DeckEffects::default();
                    }
                    ui.weak("Top to bottom · each slot has bypass and dry/wet.");
                });
                ui.separator();
                ui.columns(2, |columns| {
                    columns[0].label("Color");
                    columns[0].add(egui::Slider::new(&mut effects.hue, -1.0..=1.0).text("hue"));
                    columns[0]
                        .add(egui::Slider::new(&mut effects.contrast, 0.0..=4.0).text("contrast"));
                    columns[0].add(
                        egui::Slider::new(&mut effects.saturation, 0.0..=4.0).text("saturation"),
                    );
                    columns[0].add(
                        egui::Slider::new(&mut effects.black_level, 0.0..=0.95).text("black level"),
                    );
                    columns[0].add(
                        egui::Slider::new(&mut effects.white_level, 0.01..=1.0).text("white level"),
                    );
                    if effects.white_level <= effects.black_level {
                        effects.white_level = (effects.black_level + 0.01).min(1.0);
                    }
                    columns[0].add(egui::Slider::new(&mut effects.gamma, 0.1..=4.0).text("gamma"));
                    columns[0].add(
                        egui::Slider::new(&mut effects.bit_reduction, 0.0..=1.0)
                            .text("bit reduction"),
                    );
                    columns[0].add(
                        egui::Slider::new(&mut effects.blacklight, 0.0..=1.0).text("black light"),
                    );

                    columns[1].label("Geometry / stylize");
                    columns[1].checkbox(&mut effects.mirror, "mirror");
                    columns[1]
                        .add(egui::Slider::new(&mut effects.neon, 0.0..=1.0).text("neon glow"));
                    columns[1].add(
                        egui::Slider::new(&mut effects.fractal, 0.0..=1.0).text("fractal fold"),
                    );
                    columns[1]
                        .add(egui::Slider::new(&mut effects.jitter, 0.0..=1.0).text("jitter"));
                    columns[1].add(
                        egui::Slider::new(&mut effects.find_edges, 0.0..=1.0).text("find edges"),
                    );
                    columns[1]
                        .add(egui::Slider::new(&mut effects.pixelate, 0.0..=0.1).text("pixelate"));
                    columns[1]
                        .add(egui::Slider::new(&mut effects.bloom, 0.0..=1.0).text("bloom"))
                        .on_hover_text(
                            "Scatters light from the brightest parts of this layer only.",
                        );
                    // The shaping controls only matter once there is bloom to shape.
                    columns[1].add_enabled_ui(effects.bloom > 0.0, |ui| {
                        ui.add(
                            egui::Slider::new(&mut effects.bloom_threshold, 0.0..=1.0)
                                .text("bloom threshold"),
                        );
                        ui.add(
                            egui::Slider::new(&mut effects.bloom_radius, 0.02..=1.0)
                                .text("bloom radius"),
                        );
                        ui.add(
                            egui::Slider::new(&mut effects.bloom_chroma, 0.0..=1.0)
                                .text("bloom chroma"),
                        )
                        .on_hover_text("Spreads red further than blue, like real diffusion.");
                    });
                    columns[1]
                        .add(egui::Slider::new(&mut effects.luma_key, 0.0..=1.0).text("luma key"));
                });
                ui.horizontal(|ui| {
                    if ui.button("Reset effects").clicked() {
                        *effects = DeckEffects::default();
                    }
                    ui.weak("Effects run independently on this deck before mixing.");
                });
            });
        egui::CollapsingHeader::new("LFOs + Mod Matrix")
            .id_salt(format!("lfos-{}", id.label()))
            .show(ui, |ui| {
                ui.strong("Sources");
                for (index, lfo) in lfos.lanes.iter_mut().enumerate() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut lfo.enabled, format!("LFO {}", index + 1));
                            ui.checkbox(&mut lfo.direct_enabled, "Direct");
                            ui.add_enabled_ui(lfo.direct_enabled, |ui| {
                                egui::ComboBox::from_id_salt(format!(
                                    "lfo-target-{}-{index}",
                                    id.label()
                                ))
                                .selected_text(effect_target_label(lfo.target))
                                .show_ui(ui, |ui| {
                                    for target in EFFECT_TARGETS {
                                        ui.selectable_value(
                                            &mut lfo.target,
                                            target,
                                            effect_target_label(target),
                                        );
                                    }
                                });
                            });
                            egui::ComboBox::from_id_salt(format!(
                                "lfo-wave-{}-{index}",
                                id.label()
                            ))
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
                        });
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut lfo.tempo_sync, "Sync");
                            if lfo.tempo_sync {
                                egui::ComboBox::from_id_salt(format!(
                                    "lfo-division-{}-{index}",
                                    id.label()
                                ))
                                .selected_text(beat_division_label(lfo.beats_per_cycle))
                                .show_ui(ui, |ui| {
                                    for (beats, label) in BEAT_DIVISIONS {
                                        ui.selectable_value(&mut lfo.beats_per_cycle, beats, label);
                                    }
                                });
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
                ui.separator();
                ui.horizontal(|ui| {
                    ui.strong("Modulation routes");
                    ui.weak("One source can drive multiple destinations; negative amounts invert.");
                    if ui.button("Clear routes").clicked() {
                        lfos.routes.fill(Default::default());
                    }
                });
                egui::Grid::new(format!("mod-matrix-{}", id.label()))
                    .num_columns(4)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("On");
                        ui.strong("Source");
                        ui.strong("Destination");
                        ui.strong("Amount");
                        ui.end_row();
                        for (index, route) in lfos.routes.iter_mut().enumerate() {
                            ui.checkbox(&mut route.enabled, "");
                            egui::ComboBox::from_id_salt(format!(
                                "mod-source-{}-{index}",
                                id.label()
                            ))
                            .selected_text(mod_source_label(route.source))
                            .show_ui(ui, |ui| {
                                for source in 0..10 {
                                    ui.selectable_value(
                                        &mut route.source,
                                        source,
                                        mod_source_label(source),
                                    );
                                }
                            });
                            egui::ComboBox::from_id_salt(format!(
                                "mod-target-{}-{index}",
                                id.label()
                            ))
                            .selected_text(effect_target_label(route.target))
                            .show_ui(ui, |ui| {
                                for target in EFFECT_TARGETS {
                                    ui.selectable_value(
                                        &mut route.target,
                                        target,
                                        effect_target_label(target),
                                    );
                                }
                            });
                            ui.add(
                                egui::Slider::new(&mut route.amount, -1.0..=1.0).show_value(true),
                            );
                            ui.end_row();
                        }
                    });
            });
    });
}

pub(super) const EFFECT_TARGETS: [EffectTarget; 18] = [
    EffectTarget::Hue,
    EffectTarget::Contrast,
    EffectTarget::Saturation,
    EffectTarget::BlackLevel,
    EffectTarget::WhiteLevel,
    EffectTarget::Gamma,
    EffectTarget::Pixelate,
    EffectTarget::LumaKey,
    EffectTarget::Neon,
    EffectTarget::Fractal,
    EffectTarget::Jitter,
    EffectTarget::FindEdges,
    EffectTarget::BitReduction,
    EffectTarget::Blacklight,
    EffectTarget::Bloom,
    EffectTarget::BloomThreshold,
    EffectTarget::BloomRadius,
    EffectTarget::BloomChroma,
];

pub(super) const BEAT_DIVISIONS: [(f32, &str); 8] = [
    (0.0625, "1/16 beat"),
    (0.125, "1/8 beat"),
    (0.25, "1/4 beat"),
    (0.5, "1/2 beat"),
    (1.0, "1 beat"),
    (2.0, "2 beats"),
    (4.0, "4 beats"),
    (8.0, "8 beats"),
];

pub(super) fn effect_target_label(target: EffectTarget) -> &'static str {
    match target {
        EffectTarget::Hue => "Hue",
        EffectTarget::Contrast => "Contrast",
        EffectTarget::Saturation => "Saturation",
        EffectTarget::BlackLevel => "Black level",
        EffectTarget::WhiteLevel => "White level",
        EffectTarget::Gamma => "Gamma",
        EffectTarget::Pixelate => "Pixelate",
        EffectTarget::LumaKey => "Luma key",
        EffectTarget::Neon => "Neon",
        EffectTarget::Fractal => "Fractal",
        EffectTarget::Jitter => "Jitter",
        EffectTarget::FindEdges => "Find edges",
        EffectTarget::BitReduction => "Bit reduction",
        EffectTarget::Blacklight => "Black light",
        EffectTarget::Bloom => "Bloom",
        EffectTarget::BloomThreshold => "Bloom threshold",
        EffectTarget::BloomRadius => "Bloom radius",
        EffectTarget::BloomChroma => "Bloom chroma",
    }
}

pub(super) fn blend_mode_label(mode: LayerBlendMode) -> &'static str {
    mode.label()
}

pub(super) fn mod_source_label(source: u8) -> &'static str {
    match source {
        0 => "LFO 1",
        1 => "LFO 2",
        2 => "LFO 3",
        3 => "Audio RMS",
        4 => "Audio bass",
        5 => "Audio mid",
        6 => "Audio high",
        7 => "Audio transient",
        8 => "Beat phase",
        9 => "Bar phase",
        _ => "Invalid source",
    }
}

pub(super) fn beat_division_label(beats: f32) -> &'static str {
    BEAT_DIVISIONS
        .iter()
        .find(|(candidate, _)| (*candidate - beats).abs() < f32::EPSILON)
        .map_or("Custom", |(_, label)| *label)
}
