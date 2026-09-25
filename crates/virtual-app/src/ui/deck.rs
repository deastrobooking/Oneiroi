//! Per-deck controls: transport, transform, effects, LFOs and modulation.

use super::*;

pub(super) struct DeckControls<'a> {
    pub(super) palette: ThemePalette,
    pub(super) midi_map: &'a MidiMapUi,
    pub(super) show_mode: bool,
    pub(super) transport: &'a mut DeckTransport,
    pub(super) transform: &'a mut DeckTransform,
    pub(super) blend_mode: &'a mut LayerBlendMode,
    pub(super) solo: &'a mut bool,
    pub(super) bypassed: &'a mut bool,
    pub(super) effects: &'a mut DeckEffects,
    pub(super) lfos: &'a mut DeckLfos,
    /// Last rendered value of each modulation source, for the meters.
    pub(super) mod_sources: [f32; MODULATION_SOURCES],
    pub(super) package: &'a mut DeckPackageSlot,
    pub(super) packages: &'a [EffectDescriptor],
}

pub(super) fn draw_deck(
    ui: &mut egui::Ui,
    mixer: &mut FourDeckMixer,
    id: DeckId,
    controls: DeckControls<'_>,
    actions: &mut Vec<UiAction>,
) {
    let DeckControls {
        palette,
        midi_map,
        show_mode,
        transport,
        transform,
        blend_mode,
        solo,
        bypassed,
        effects,
        lfos,
        mod_sources,
        package,
        packages,
    } = controls;
    let accent = palette.deck_color(id);
    let selected = mixer.selected() == id;
    let frame = egui::Frame::group(ui.style())
        .fill(if selected {
            palette.surface_tint(accent, if palette.dark { 0.14 } else { 0.08 })
        } else {
            ui.visuals().faint_bg_color
        })
        .stroke(egui::Stroke::new(
            if selected {
                palette.outline_width + 1.0
            } else {
                palette.outline_width
            },
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
                                ui.visuals().text_color()
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
            let eject_enabled = !show_mode && !matches!(mixer.deck(id).state, DeckState::Empty);
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
                    if movie.decode_path == virtual_media::DecodePath::FfmpegVideo {
                        ui.label(format!("{} keys", movie.keyframes.len()));
                    }
                });
                let (label, color) = match movie.health {
                    MediaHealth::StageReady => ("STAGE READY", palette.success),
                    MediaHealth::Usable => ("USABLE", palette.accent),
                    MediaHealth::Caution => ("CAUTION", palette.warning),
                    MediaHealth::Problem => ("PROBLEM", palette.danger),
                };
                ui.colored_label(color, label);
                ui.weak(&movie.health_reason);
            }
            DeckState::Live(config) => {
                ui.colored_label(palette.success, "● LIVE CAMERA");
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
                ui.colored_label(palette.danger, "IMPORT ERROR");
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
            mappable(
                ui,
                midi_map,
                ControlTarget::DeckLevel(id.index() as u8),
                actions,
                |ui| {
                    ui.add(
                        egui::Slider::new(&mut deck.level, 0.0..=1.0)
                            .text("level")
                            .clamping(egui::SliderClamping::Always),
                    )
                },
            );
            if show_mode {
                ui.weak(match deck.bus {
                    CrossfadeBus::Left => "Bus A",
                    CrossfadeBus::Right => "Bus B",
                });
            } else {
                ui.selectable_value(&mut deck.bus, CrossfadeBus::Left, "Bus A");
                ui.selectable_value(&mut deck.bus, CrossfadeBus::Right, "Bus B");
            }
        });
        ui.horizontal(|ui| {
            if ui.selectable_label(*solo, "Solo").clicked() {
                *solo = !*solo;
            }
            if ui.selectable_label(*bypassed, "Bypass").clicked() {
                *bypassed = !*bypassed;
            }
            if show_mode {
                ui.weak(format!("Blend · {}", blend_mode_label(*blend_mode)));
            } else {
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
            }
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
                let play = mappable(
                    ui,
                    midi_map,
                    ControlTarget::DeckPlay(id.index() as u8),
                    actions,
                    |ui| ui.button(if transport.playing { "Pause" } else { "Play" }),
                );
                if play.clicked() {
                    transport.playing = !transport.playing;
                }
                let restart = mappable(
                    ui,
                    midi_map,
                    ControlTarget::DeckRestart(id.index() as u8),
                    actions,
                    |ui| ui.button("Restart"),
                );
                if restart.clicked() {
                    transport.restart();
                    actions.push(UiAction::Restart(id));
                }
                mappable(
                    ui,
                    midi_map,
                    ControlTarget::DeckFreeze(id.index() as u8),
                    actions,
                    |ui| ui.checkbox(&mut transport.frozen, "Freeze"),
                );
                let mut looping = transport.end_mode == EndMode::Loop;
                if ui.checkbox(&mut looping, "Loop").changed() {
                    transport.end_mode = if looping {
                        EndMode::Loop
                    } else {
                        EndMode::OneShot
                    };
                }
                mappable(
                    ui,
                    midi_map,
                    ControlTarget::DeckSpeed(id.index() as u8),
                    actions,
                    |ui| {
                        ui.add(
                            egui::Slider::new(&mut transport.speed, 0.25..=4.0)
                                .text("speed")
                                .logarithmic(true),
                        )
                    },
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
        if !show_mode {
            egui::CollapsingHeader::new("Layer transform")
                .id_salt(format!("transform-{}", id.label()))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Source mode");
                        ui.selectable_value(&mut transform.source_mode, SourceMode::Fit, "Fit");
                        ui.selectable_value(&mut transform.source_mode, SourceMode::Fill, "Fill");
                        ui.selectable_value(
                            &mut transform.source_mode,
                            SourceMode::Stretch,
                            "Stretch",
                        );
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
                        columns[0].add(
                            egui::Slider::new(&mut transform.crop[0], 0.0..=0.95).text("left"),
                        );
                        columns[0].add(
                            egui::Slider::new(&mut transform.crop[1], 0.0..=0.95).text("right"),
                        );
                        columns[1]
                            .add(egui::Slider::new(&mut transform.crop[2], 0.0..=0.95).text("top"));
                        columns[1].add(
                            egui::Slider::new(&mut transform.crop[3], 0.0..=0.95).text("bottom"),
                        );
                    });
                    *transform = transform.sanitized();
                });
        }
        let effect_controls = |ui: &mut egui::Ui| {
            draw_deck_package(ui, id, show_mode, palette, accent, package, packages, actions);
            ui.add_space(4.0);
            if show_mode {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("LIVE DECK EFFECTS");
                    ui.weak("Chain order and resets are locked in Show Mode.");
                });
            } else {
                ui.strong("Effect chain");
            }
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
                    if !show_mode {
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
                    }
                });
            }
            if let Some((from, to)) = reorder {
                effects.slots.swap(from, to);
            }
            if !show_mode {
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
                    ui.weak(
                        "Geometry is the UV prepass · Color and Stylize follow their relative order.",
                    );
                });
            }
            ui.separator();
            // Every slider is a modulation and MIDI destination; `fx`
            // pairs each with its stable effect-parameter index so map
            // mode can arm it in place. The indices match
            // `effect_parameter` in the app shell and must not be
            // renumbered.
            let mut fx = |ui: &mut egui::Ui, effect: u8, slider: egui::Slider<'_>| {
                mappable(
                    ui,
                    midi_map,
                    ControlTarget::EffectParameter {
                        deck: id.index() as u8,
                        effect,
                        parameter: 0,
                    },
                    actions,
                    |ui| ui.add(slider),
                )
            };
            ui.columns(2, |columns| {
                columns[0].label("Color");
                fx(
                    &mut columns[0],
                    0,
                    egui::Slider::new(&mut effects.hue, -1.0..=1.0).text("hue"),
                );
                fx(
                    &mut columns[0],
                    1,
                    egui::Slider::new(&mut effects.contrast, 0.0..=4.0).text("contrast"),
                );
                fx(
                    &mut columns[0],
                    2,
                    egui::Slider::new(&mut effects.saturation, 0.0..=4.0).text("saturation"),
                );
                fx(
                    &mut columns[0],
                    3,
                    egui::Slider::new(&mut effects.black_level, 0.0..=0.95).text("black level"),
                );
                fx(
                    &mut columns[0],
                    4,
                    egui::Slider::new(&mut effects.white_level, 0.01..=1.0).text("white level"),
                );
                if effects.white_level <= effects.black_level {
                    effects.white_level = (effects.black_level + 0.01).min(1.0);
                }
                fx(
                    &mut columns[0],
                    5,
                    egui::Slider::new(&mut effects.gamma, 0.1..=4.0).text("gamma"),
                );
                fx(
                    &mut columns[0],
                    12,
                    egui::Slider::new(&mut effects.bit_reduction, 0.0..=1.0).text("bit reduction"),
                );
                fx(
                    &mut columns[0],
                    13,
                    egui::Slider::new(&mut effects.blacklight, 0.0..=1.0).text("black light"),
                );

                columns[1].label("Geometry / stylize");
                columns[1].checkbox(&mut effects.mirror, "mirror");
                fx(
                    &mut columns[1],
                    8,
                    egui::Slider::new(&mut effects.neon, 0.0..=1.0).text("neon glow"),
                );
                fx(
                    &mut columns[1],
                    9,
                    egui::Slider::new(&mut effects.fractal, 0.0..=1.0).text("fractal fold"),
                );
                fx(
                    &mut columns[1],
                    10,
                    egui::Slider::new(&mut effects.jitter, 0.0..=1.0).text("jitter"),
                );
                fx(
                    &mut columns[1],
                    11,
                    egui::Slider::new(&mut effects.find_edges, 0.0..=1.0).text("find edges"),
                );
                fx(
                    &mut columns[1],
                    6,
                    egui::Slider::new(&mut effects.pixelate, 0.0..=0.1).text("pixelate"),
                );
                fx(
                    &mut columns[1],
                    14,
                    egui::Slider::new(&mut effects.bloom, 0.0..=1.0).text("bloom"),
                )
                .on_hover_text("Scatters light from the brightest parts of this layer only.");
                // The shaping controls only matter once there is bloom to shape.
                columns[1].add_enabled_ui(effects.bloom > 0.0, |ui| {
                    fx(
                        ui,
                        15,
                        egui::Slider::new(&mut effects.bloom_threshold, 0.0..=1.0)
                            .text("bloom threshold"),
                    );
                    fx(
                        ui,
                        16,
                        egui::Slider::new(&mut effects.bloom_radius, 0.02..=1.0)
                            .text("bloom radius"),
                    );
                    fx(
                        ui,
                        17,
                        egui::Slider::new(&mut effects.bloom_chroma, 0.0..=1.0)
                            .text("bloom chroma"),
                    )
                    .on_hover_text("Spreads red further than blue, like real diffusion.");
                });
                fx(
                    &mut columns[1],
                    7,
                    egui::Slider::new(&mut effects.luma_key, 0.0..=1.0).text("luma key"),
                );
            });
            ui.horizontal(|ui| {
                if !show_mode && ui.button("Reset effects").clicked() {
                    *effects = DeckEffects::default();
                }
                ui.weak("Effects run independently on this deck before mixing.");
            });
        };
        if show_mode || selected {
            ui.group(effect_controls);
        } else {
            egui::CollapsingHeader::new("GPU effects")
                .id_salt(format!("effects-{}", id.label()))
                .show(ui, effect_controls);
        }
        if !show_mode {
            let active_lfos = lfos.lanes.iter().filter(|lfo| lfo.enabled).count();
            let active_routes = lfos.routes.iter().filter(|route| route.enabled).count();
            egui::CollapsingHeader::new(format!(
                "LFOs + Mod Matrix · {active_lfos}/3 LFOs · {active_routes}/{MOD_ROUTES_PER_DECK} routes"
            ))
            .id_salt(format!("lfos-{}", id.label()))
            .show(ui, |ui| {
                ui.strong("Sources");
                for (index, lfo) in lfos.lanes.iter_mut().enumerate() {
                    let mut add_route = None;
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut lfo.enabled, format!("LFO {}", index + 1));
                            ui.checkbox(&mut lfo.direct_enabled, "Direct")
                                .on_hover_text("Drive the destination below without a matrix route");
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
                            if ui
                                .small_button("+ Route")
                                .on_hover_text("Send this LFO to another destination through the matrix")
                                .clicked()
                            {
                                add_route = Some(lfo.target);
                            }
                            if ui.small_button("Reset").clicked() {
                                *lfo = EffectLfo {
                                    enabled: lfo.enabled,
                                    target: lfo.target,
                                    ..EffectLfo::default()
                                };
                            }
                        });
                        draw_lfo_shape(
                            ui,
                            egui::Id::new(("deck-lfo", id.index(), index)),
                            accent,
                            lfo.enabled,
                            mod_sources[index],
                            LfoFields {
                                waveform: &mut lfo.waveform,
                                tempo_sync: &mut lfo.tempo_sync,
                                rate_hz: &mut lfo.rate_hz,
                                beats_per_cycle: &mut lfo.beats_per_cycle,
                                depth: &mut lfo.depth,
                                phase: &mut lfo.phase,
                                offset: &mut lfo.offset,
                                unipolar: &mut lfo.unipolar,
                                invert: &mut lfo.invert,
                            },
                        );
                    });
                    if let Some(target) = add_route
                        && let Some(route) = lfos.routes.iter_mut().find(|route| !route.enabled)
                    {
                        *route = ModulationRoute {
                            enabled: true,
                            source: index as u8,
                            target,
                            amount: 0.5,
                        };
                        lfo.enabled = true;
                    }
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.strong("Modulation routes");
                    ui.weak("One source can drive multiple destinations; negative amounts invert.");
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            lfos.routes.iter().any(|route| !route.enabled),
                            egui::Button::new("Add route"),
                        )
                        .clicked()
                        && let Some(route) = lfos.routes.iter_mut().find(|route| !route.enabled)
                    {
                        route.enabled = true;
                    }
                    if ui.button("Enable all").clicked() {
                        for route in &mut lfos.routes {
                            route.enabled = true;
                        }
                    }
                    if ui.button("Mute all").clicked() {
                        for route in &mut lfos.routes {
                            route.enabled = false;
                        }
                    }
                    if ui.button("Clear routes").clicked() {
                        lfos.routes.fill(Default::default());
                    }
                });
                egui::Grid::new(format!("mod-matrix-{}", id.label()))
                    .num_columns(7)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("On");
                        ui.strong("Source");
                        ui.strong("Destination");
                        ui.strong("Amount");
                        ui.strong("");
                        ui.strong("Live");
                        ui.strong("");
                        ui.end_row();
                        for (index, route) in lfos.routes.iter_mut().enumerate() {
                            ui.checkbox(&mut route.enabled, format!("{}", index + 1));
                            mod_source_combo(
                                ui,
                                format!("mod-source-{}-{index}", id.label()),
                                &mut route.source,
                            );
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
                                egui::Slider::new(&mut route.amount, -1.0..=1.0)
                                    .show_value(true),
                            );
                            if ui
                                .small_button("±")
                                .on_hover_text("Invert this route")
                                .clicked()
                            {
                                route.amount = -route.amount;
                            }
                            let live = mod_sources
                                .get(usize::from(route.source))
                                .copied()
                                .unwrap_or_default()
                                * route.amount;
                            modulation_meter(
                                ui,
                                if route.enabled { live } else { 0.0 },
                                accent,
                                70.0,
                            );
                            if ui
                                .small_button("✕")
                                .on_hover_text("Clear this route")
                                .clicked()
                            {
                                *route = ModulationRoute::default();
                            }
                            ui.end_row();
                        }
                    });
            });
        }
    });
}

pub(super) struct LfoFields<'a> {
    pub(super) waveform: &'a mut LfoWaveform,
    pub(super) tempo_sync: &'a mut bool,
    pub(super) rate_hz: &'a mut f32,
    pub(super) beats_per_cycle: &'a mut f32,
    pub(super) depth: &'a mut f32,
    pub(super) phase: &'a mut f32,
    pub(super) offset: &'a mut f32,
    pub(super) unipolar: &'a mut bool,
    pub(super) invert: &'a mut bool,
}

/// Waveform, rate and output-shaping controls shared by deck and master LFOs.
pub(super) fn draw_lfo_shape(
    ui: &mut egui::Ui,
    id: egui::Id,
    color: egui::Color32,
    enabled: bool,
    live: f32,
    lfo: LfoFields<'_>,
) {
    let LfoFields {
        waveform,
        tempo_sync,
        rate_hz,
        beats_per_cycle,
        depth,
        phase,
        offset,
        unipolar,
        invert,
    } = lfo;
    ui.horizontal(|ui| {
        lfo_preview(
            ui,
            *waveform,
            LfoShaping {
                depth: *depth,
                offset: *offset,
                unipolar: *unipolar,
                invert: *invert,
            },
            *phase,
            live,
            color,
            enabled,
        );
        ui.vertical(|ui| {
            egui::ComboBox::from_id_salt(id.with("wave"))
                .selected_text(waveform_label(*waveform))
                .show_ui(ui, |ui| {
                    for candidate in LFO_WAVEFORMS {
                        ui.selectable_value(waveform, candidate, waveform_label(candidate));
                    }
                });
            ui.horizontal(|ui| {
                ui.toggle_value(unipolar, "Unipolar")
                    .on_hover_text("Output 0…1 instead of −1…1, so it only pushes one way");
                ui.toggle_value(invert, "Invert");
            });
            modulation_meter(ui, if enabled { live } else { 0.0 }, color, 120.0);
        });
    });
    ui.horizontal(|ui| {
        ui.toggle_value(tempo_sync, "Sync")
            .on_hover_text("Lock the cycle to the tempo clock");
        if *tempo_sync {
            egui::ComboBox::from_id_salt(id.with("division"))
                .selected_text(beat_division_label(*beats_per_cycle))
                .show_ui(ui, |ui| {
                    for (beats, label) in BEAT_DIVISIONS {
                        ui.selectable_value(beats_per_cycle, beats, label);
                    }
                });
        } else {
            ui.add(
                egui::Slider::new(rate_hz, 0.01..=20.0)
                    .logarithmic(true)
                    .text("Hz"),
            );
        }
        if ui.small_button("÷2").on_hover_text("Half speed").clicked() {
            if *tempo_sync {
                *beats_per_cycle = (*beats_per_cycle * 2.0).min(8.0);
            } else {
                *rate_hz = (*rate_hz * 0.5).max(0.01);
            }
        }
        if ui
            .small_button("×2")
            .on_hover_text("Double speed")
            .clicked()
        {
            if *tempo_sync {
                *beats_per_cycle = (*beats_per_cycle * 0.5).max(0.0625);
            } else {
                *rate_hz = (*rate_hz * 2.0).min(20.0);
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::Slider::new(depth, 0.0..=1.0).text("depth"));
        ui.add(egui::Slider::new(phase, 0.0..=1.0).text("phase"));
        if ui
            .add(egui::Slider::new(offset, -1.0..=1.0).text("offset"))
            .on_hover_text("Shifts the whole wave; double-click to reset")
            .double_clicked()
        {
            *offset = 0.0;
        }
    });
}

/// One cycle of the shaped wave (four for the random shapes) with the
/// current output marked on the right edge.
fn lfo_preview(
    ui: &mut egui::Ui,
    waveform: LfoWaveform,
    shaping: LfoShaping,
    phase: f32,
    live: f32,
    color: egui::Color32,
    enabled: bool,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(120.0, 44.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
    let weak = visuals.weak_text_color();
    painter.line_segment(
        [rect.left_center(), rect.right_center()],
        egui::Stroke::new(1.0, weak.gamma_multiply(0.5)),
    );
    let inner = rect.shrink2(egui::vec2(4.0, 4.0));
    let to_y = |value: f32| inner.center().y - value.clamp(-1.0, 1.0) * inner.height() * 0.5;
    let cycles = if matches!(
        waveform,
        LfoWaveform::SampleHold | LfoWaveform::SmoothRandom
    ) {
        4.0
    } else {
        1.0
    };
    let points = (0..=120)
        .map(|step| {
            let t = step as f32 / 120.0;
            let value = shaping.output(waveform.sample(t * cycles + phase));
            egui::pos2(inner.left() + t * inner.width(), to_y(value))
        })
        .collect();
    let stroke_color = if enabled { color } else { weak };
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5, stroke_color),
    ));
    if enabled {
        painter.circle_filled(egui::pos2(inner.right(), to_y(live)), 3.5, color);
    }
}

/// Centre-zero bar for a bipolar modulation value.
pub(super) fn modulation_meter(ui: &mut egui::Ui, value: f32, color: egui::Color32, width: f32) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 10.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    let value = if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let centre = rect.center().x;
    let end = centre + value * rect.width() * 0.5;
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(centre.min(end), rect.top() + 2.0),
            egui::pos2(centre.max(end), rect.bottom() - 2.0),
        ),
        2.0,
        color,
    );
    painter.line_segment(
        [
            egui::pos2(centre, rect.top()),
            egui::pos2(centre, rect.bottom()),
        ],
        egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
    );
    response.on_hover_text(format!("{value:+.2}"));
}

/// Large selectable tiles for picking an algorithmic effect package.
/// Returns true when the selection changed.
pub(super) fn algorithm_tiles(
    ui: &mut egui::Ui,
    selected_id: &mut String,
    packages: &[EffectDescriptor],
    palette: ThemePalette,
    accent: egui::Color32,
    allow_none: bool,
    enabled: bool,
) -> bool {
    let previous = selected_id.clone();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        if allow_none
            && algorithm_tile(ui, "Off", selected_id.is_empty(), palette, accent, enabled)
                .on_hover_text("No algorithmic effect")
                .clicked()
        {
            selected_id.clear();
        }
        for package in packages {
            let response = algorithm_tile(
                ui,
                &package.name,
                *selected_id == package.id,
                palette,
                accent,
                enabled,
            );
            let response = if package.description.is_empty() {
                response
            } else {
                response.on_hover_text(&package.description)
            };
            if response.clicked() {
                selected_id.clone_from(&package.id);
            }
        }
    });
    *selected_id != previous
}

fn algorithm_tile(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    palette: ThemePalette,
    accent: egui::Color32,
    enabled: bool,
) -> egui::Response {
    let text = egui::RichText::new(label).size(15.0).strong();
    let text = if selected {
        text.color(ui.visuals().strong_text_color())
    } else {
        text
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(text)
            .min_size(egui::vec2(136.0, 44.0))
            .selected(selected)
            .fill(if selected {
                palette.control_tint(accent, 0.55)
            } else {
                palette.control
            })
            .stroke(egui::Stroke::new(
                if selected {
                    palette.outline_width + 1.5
                } else {
                    palette.outline_width
                },
                if selected { accent } else { palette.stroke },
            )),
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_deck_package(
    ui: &mut egui::Ui,
    id: DeckId,
    show_mode: bool,
    palette: ThemePalette,
    accent: egui::Color32,
    slot: &mut DeckPackageSlot,
    packages: &[EffectDescriptor],
    actions: &mut Vec<UiAction>,
) {
    egui::Frame::group(ui.style())
        .fill(palette.surface_tint(accent, if palette.dark { 0.18 } else { 0.10 }))
        .stroke(egui::Stroke::new(palette.outline_width + 1.0, accent))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            draw_deck_package_body(ui, id, show_mode, palette, accent, slot, packages, actions);
        });
}

#[allow(clippy::too_many_arguments)]
fn draw_deck_package_body(
    ui: &mut egui::Ui,
    id: DeckId,
    show_mode: bool,
    palette: ThemePalette,
    accent: egui::Color32,
    slot: &mut DeckPackageSlot,
    packages: &[EffectDescriptor],
    actions: &mut Vec<UiAction>,
) {
    let selected = packages
        .iter()
        .find(|candidate| candidate.id == slot.package_id);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("◆ ALGORITHMIC FX")
                .size(20.0)
                .strong()
                .color(accent),
        );
        ui.label(
            egui::RichText::new(selected.map_or("Off", |package| package.name.as_str()))
                .size(18.0)
                .strong(),
        );
    });
    if show_mode {
        ui.weak("Algorithm choice is locked in Show Mode.");
    } else if algorithm_tiles(
        ui,
        &mut slot.package_id,
        packages,
        palette,
        accent,
        true,
        true,
    ) {
        slot.parameters = packages
            .iter()
            .find(|candidate| candidate.id == slot.package_id)
            .map(|package| {
                package
                    .parameters
                    .iter()
                    .map(|parameter| EffectParameterValue {
                        id: parameter.id.clone(),
                        value: parameter.default,
                    })
                    .collect()
            })
            .unwrap_or_default();
        slot.modulation.fill(DeckPackageModulationRoute::default());
    }

    if slot.package_id.is_empty() {
        ui.weak("Select a deck-v1 package to run it before this layer is blended.");
        return;
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let bypass = egui::Button::new(
            egui::RichText::new(if slot.bypassed { "BYPASSED" } else { "ACTIVE" })
                .size(15.0)
                .strong(),
        )
        .min_size(egui::vec2(110.0, 34.0))
        .fill(if slot.bypassed {
            palette.control_tint(palette.warning, 0.35)
        } else {
            palette.control_tint(palette.success, 0.35)
        });
        if ui
            .add(bypass)
            .on_hover_text("Toggle the algorithm without losing its settings")
            .clicked()
        {
            slot.bypassed = !slot.bypassed;
        }
        ui.spacing_mut().slider_width = (ui.available_width() - 60.0).clamp(120.0, 320.0);
        ui.add(egui::Slider::new(&mut slot.mix, 0.0..=1.0).text("wet"));
    });
    let Some(package) = packages
        .iter()
        .find(|candidate| candidate.id == slot.package_id)
    else {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!(
                "Package {:?} is unavailable; deck passes through.",
                slot.package_id
            ),
        );
        return;
    };
    if !package.description.is_empty() && !show_mode {
        ui.weak(&package.description);
    }
    if !package.presets.is_empty() && !show_mode {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Looks");
            for preset in &package.presets {
                let response = ui.add(
                    egui::Button::new(egui::RichText::new(&preset.label).size(14.0))
                        .min_size(egui::vec2(0.0, 28.0)),
                );
                let response = if preset.description.is_empty() {
                    response
                } else {
                    response.on_hover_text(&preset.description)
                };
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
            }
        });
    }
    if !show_mode {
        let mut previous_group = None::<&str>;
        for parameter in &package.parameters {
            let group = parameter.group.trim();
            if !group.is_empty() && previous_group != Some(group) {
                ui.strong(group);
                previous_group = Some(group);
            }
            let value_index = slot
                .parameters
                .iter()
                .position(|value| value.id == parameter.id)
                .unwrap_or_else(|| {
                    slot.parameters.push(EffectParameterValue {
                        id: parameter.id.clone(),
                        value: parameter.default,
                    });
                    slot.parameters.len() - 1
                });
            match parameter.control {
                EffectParameterControl::Slider => {
                    ui.add(
                        egui::Slider::new(
                            &mut slot.parameters[value_index].value,
                            parameter.minimum..=parameter.maximum,
                        )
                        .text(&parameter.label),
                    );
                }
                EffectParameterControl::Toggle => {
                    let mut enabled = slot.parameters[value_index].value >= 0.5;
                    if ui.checkbox(&mut enabled, &parameter.label).changed() {
                        slot.parameters[value_index].value = f32::from(enabled);
                    }
                }
                EffectParameterControl::Choice => {
                    let selected = parameter
                        .options
                        .iter()
                        .min_by(|left, right| {
                            (left.value - slot.parameters[value_index].value)
                                .abs()
                                .total_cmp(
                                    &(right.value - slot.parameters[value_index].value).abs(),
                                )
                        })
                        .map_or("Choose", |option| option.label.as_str());
                    ui.horizontal(|ui| {
                        ui.label(&parameter.label);
                        egui::ComboBox::from_id_salt((
                            "deck-package-choice",
                            id.index(),
                            parameter.id.as_str(),
                        ))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for option in &parameter.options {
                                ui.selectable_value(
                                    &mut slot.parameters[value_index].value,
                                    option.value,
                                    &option.label,
                                );
                            }
                        });
                    });
                }
            }
            let target = ControlTarget::DeckEffectParameter {
                deck: id.index() as u8,
                parameter_key: effect_parameter_key(&slot.package_id, &parameter.id),
            };
            ui.horizontal(|ui| {
                if ui.small_button("MIDI learn").clicked() {
                    actions.push(UiAction::MidiLearn(target));
                }
                if ui.small_button("Clear").clicked() {
                    actions.push(UiAction::MidiClearTarget(target));
                }
            });
        }
        egui::CollapsingHeader::new("Package modulation")
            .id_salt(("deck-package-modulation", id.index()))
            .default_open(false)
            .show(ui, |ui| {
                for (route_index, route) in slot.modulation.iter_mut().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut route.enabled, format!("Route {}", route_index + 1));
                        mod_source_combo(
                            ui,
                            ("deck-package-mod-source", id.index(), route_index),
                            &mut route.source,
                        );
                        let selected = package
                            .parameters
                            .iter()
                            .find(|parameter| {
                                effect_parameter_key(&package.id, &parameter.id)
                                    == route.parameter_key
                            })
                            .map_or("Choose parameter", |parameter| parameter.label.as_str());
                        egui::ComboBox::from_id_salt((
                            "deck-package-mod-target",
                            id.index(),
                            route_index,
                        ))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for parameter in &package.parameters {
                                ui.selectable_value(
                                    &mut route.parameter_key,
                                    effect_parameter_key(&package.id, &parameter.id),
                                    &parameter.label,
                                );
                            }
                        });
                        ui.add(egui::Slider::new(&mut route.amount, -1.0..=1.0).text("amount"));
                    });
                }
            });
    }
    slot.sanitize();
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

pub(super) const BEAT_DIVISIONS: [(f32, &str); 14] = [
    (0.0625, "1/16 beat"),
    (0.125, "1/8 beat"),
    (1.0 / 6.0, "1/6 beat (triplet)"),
    (0.25, "1/4 beat"),
    (1.0 / 3.0, "1/3 beat (triplet)"),
    (0.375, "3/8 beat (dotted)"),
    (0.5, "1/2 beat"),
    (2.0 / 3.0, "2/3 beat (triplet)"),
    (0.75, "3/4 beat (dotted)"),
    (1.0, "1 beat"),
    (1.5, "1.5 beats (dotted)"),
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

/// Labels in `virtual_render::modulation_sources` order; indices are persisted.
pub(super) const MOD_SOURCE_LABELS: [&str; MODULATION_SOURCES] = [
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
    "Band · Sub",
    "Band · Bass",
    "Band · Low mid",
    "Band · Mid",
    "Band · Upper mid",
    "Band · Presence",
    "Band · Brilliance",
    "Band · Air",
];

pub(super) fn mod_source_label(source: u8) -> &'static str {
    MOD_SOURCE_LABELS
        .get(usize::from(source))
        .copied()
        .unwrap_or("Invalid source")
}

/// Source picker shared by every modulation matrix.
pub(super) fn mod_source_combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    source: &mut u8,
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(mod_source_label(*source))
        .height(420.0)
        .show_ui(ui, |ui| {
            for (index, label) in MOD_SOURCE_LABELS.iter().enumerate() {
                if index == SPECTRUM_SOURCE_OFFSET {
                    ui.separator();
                }
                ui.selectable_value(source, index as u8, *label);
            }
        });
}

pub(super) fn beat_division_label(beats: f32) -> &'static str {
    BEAT_DIVISIONS
        .iter()
        .find(|(candidate, _)| (*candidate - beats).abs() < 1.0e-4)
        .map_or("Custom", |(_, label)| *label)
}
