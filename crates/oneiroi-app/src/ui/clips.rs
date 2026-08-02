//! Clip grid: slots, health, launch and thumbnail rendering.

use super::*;

pub(super) fn draw_clip_grid(
    ui: &mut egui::Ui,
    state: &UiState,
    mixer: &mut FourDeckMixer,
    clips: &mut ClipBank,
    launches: &LaunchQueue,
    midi_map: &MidiMapUi,
    actions: &mut Vec<UiAction>,
) {
    let palette = state.theme.palette();
    egui::Grid::new("clip-grid")
        .num_columns(CLIPS_PER_DECK + 1)
        .spacing([5.0, 5.0])
        .show(ui, |ui| {
            ui.strong("SCENE");
            for slot in 0..CLIPS_PER_DECK {
                let scene = mappable(
                    ui,
                    midi_map,
                    ControlTarget::SceneLaunch(slot as u8),
                    actions,
                    |ui| {
                        ui.add_sized(
                            [96.0, 28.0],
                            egui::Button::new(
                                egui::RichText::new(format!("SCENE {}", slot + 1)).strong(),
                            )
                            .fill(palette.control_tint(palette.secondary, 0.22)),
                        )
                    },
                );
                if scene
                    .on_hover_text(format!(
                        "Launch scene {} on the next quantized boundary",
                        slot + 1
                    ))
                    .clicked()
                {
                    actions.push(UiAction::LaunchScene(slot));
                }
            }
            ui.end_row();

            for deck in DeckId::ALL {
                ui.strong(format!("DECK {}", deck.label()));
                for slot in 0..CLIPS_PER_DECK {
                    let address = ClipAddress { deck, slot };
                    let selected = clips.selected(deck) == slot && mixer.selected() == deck;
                    let active = clips.active(deck) == Some(slot);
                    let queued = launches.queued(address);
                    let slot_state = clips
                        .slot(address)
                        .cloned()
                        .expect("valid clip-grid address");
                    let first_frame_ready = state
                        .preloaded_frame(address, clips.path(address))
                        .is_some();
                    let label = if let Some(movie) = &slot_state.movie {
                        let name = movie
                            .display_name
                            .split('.')
                            .next()
                            .unwrap_or(&movie.display_name);
                        let short: String = name.chars().take(8).collect();
                        if queued {
                            format!("◷ {short}")
                        } else if active {
                            format!("▶ {short}")
                        } else if first_frame_ready {
                            format!("● {short}")
                        } else {
                            format!("○ {short}")
                        }
                    } else if slot_state.error.is_some() {
                        format!("⚠ {}{}", deck.label(), slot + 1)
                    } else if let Some(path) = &slot_state.pending_path {
                        format!(
                            "… {}",
                            path.file_stem()
                                .and_then(|name| name.to_str())
                                .unwrap_or("loading")
                        )
                    } else {
                        format!("{}{}", deck.label(), slot + 1)
                    };
                    let button =
                        if let Some(thumbnail) = state.thumbnail(address, clips.path(address)) {
                            egui::Button::image_and_text(thumbnail, label)
                        } else {
                            let label = if state
                                .thumbnail_failure(address, clips.path(address))
                                .is_some()
                            {
                                format!("□ {label}")
                            } else {
                                label
                            };
                            egui::Button::new(label)
                        }
                        .selected(selected || active)
                        .fill(if active {
                            palette.control_tint(palette.success, 0.24)
                        } else if queued {
                            palette.control_tint(palette.warning, 0.22)
                        } else if selected {
                            palette.control_tint(palette.accent, 0.24)
                        } else {
                            palette.control
                        })
                        .stroke(egui::Stroke::new(
                            if active || queued { 2.0 } else { 1.0 },
                            if active {
                                palette.success
                            } else if queued {
                                palette.warning
                            } else {
                                palette.stroke
                            },
                        ));
                    let draggable = !midi_map.active
                        && (slot_state.movie.is_some()
                            || slot_state.pending_path.is_some()
                            || slot_state.error.is_some());
                    let response = if midi_map.active {
                        mappable(
                            ui,
                            midi_map,
                            ControlTarget::ClipLaunch {
                                deck: deck.index() as u8,
                                slot: slot as u8,
                            },
                            actions,
                            |ui| ui.add_sized([96.0, 46.0], button),
                        )
                    } else if draggable {
                        // Drag moves the clip to another slot; a plain click
                        // still selects and launches because the drag only
                        // starts past egui's drag threshold.
                        ui.dnd_drag_source(
                            egui::Id::new(("clip-slot-drag", deck.index(), slot)),
                            address,
                            |ui| ui.add_sized([96.0, 46.0], button),
                        )
                        .inner
                    } else {
                        ui.add_sized([96.0, 46.0], button)
                    };
                    let response = if draggable {
                        response.on_hover_cursor(egui::CursorIcon::Grab)
                    } else {
                        response
                    };
                    if let Some(source) = response.dnd_hover_payload::<ClipAddress>()
                        && *source != address
                    {
                        ui.painter().rect_stroke(
                            response.rect.expand(2.0),
                            6.0,
                            egui::Stroke::new(2.0, palette.accent),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if let Some(source) = response.dnd_release_payload::<ClipAddress>()
                        && *source != address
                    {
                        actions.push(UiAction::MoveClip {
                            from: *source,
                            to: address,
                        });
                    }
                    if response.clicked() {
                        clips.select(address);
                        mixer.select(deck);
                        if clips.movie(address).is_some() {
                            actions.push(UiAction::Launch(address));
                        }
                    }
                    response
                        .on_hover_text(if let Some(movie) = clips.movie(address) {
                            let mut details = format!(
                                "{}\n{}×{} · {}",
                                movie.display_name,
                                movie.visible_extent[0],
                                movie.visible_extent[1],
                                movie.codec
                            );
                            if movie.decode_path == oneiroi_media::DecodePath::FfmpegVideo {
                                details.push_str(&format!(
                                    "\n{} keyframe(s) indexed{}",
                                    movie.keyframes.len(),
                                    if movie.keyframes.is_complete() {
                                        ""
                                    } else {
                                        " · capped"
                                    }
                                ));
                            }
                            if let Some(error) =
                                state.thumbnail_failure(address, clips.path(address))
                            {
                                details.push_str(&format!("\nThumbnail unavailable: {error}"));
                            } else if first_frame_ready {
                                details.push_str("\nFirst frame ready for immediate launch");
                            } else {
                                details.push_str("\nFirst frame is still preloading");
                            }
                            details
                                .push_str("\nDrag onto another slot to move; occupied slots swap");
                            details
                        } else if let Some(error) = &slot_state.error {
                            format!(
                                "{}\n{error}",
                                slot_state.pending_path.as_deref().map_or_else(
                                    || "Missing media".to_owned(),
                                    |path| path.display().to_string()
                                )
                            )
                        } else if let Some(path) = &slot_state.pending_path {
                            format!("Restoring {}", path.display())
                        } else {
                            "Empty slot · select then drop a movie".to_owned()
                        })
                        .context_menu(|ui| {
                            if clips.path(address).is_some() && ui.button("Relink media…").clicked()
                            {
                                actions.push(UiAction::BrowseRelink(address));
                                ui.close();
                            }
                            if clips.path(address).is_some() && ui.button("Clear slot").clicked() {
                                actions.push(UiAction::ClearSlot(address));
                                ui.close();
                            }
                        });
                }
                ui.end_row();
            }
        });

    let deck = mixer.selected();
    let address = ClipAddress {
        deck,
        slot: clips.selected(deck),
    };
    if let Some(movie) = clips.movie(address) {
        let name = movie.display_name.clone();
        let media_duration = movie.duration.map(oneiroi_core::MediaTime::as_seconds);
        let mut playback = clips.playback(address).unwrap_or_default();
        let mut changed = false;
        egui::CollapsingHeader::new(format!("Selected clip playback · {name}"))
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Launch");
                    changed |= ui
                        .selectable_value(
                            &mut playback.launch_mode,
                            ClipLaunchMode::Restart,
                            "Restart at In",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut playback.launch_mode,
                            ClipLaunchMode::Resume,
                            "Resume last position",
                        )
                        .changed();
                });
                let maximum = media_duration.unwrap_or(86_400.0).max(0.001);
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut playback.in_point)
                                .range(0.0..=maximum)
                                .speed(0.05)
                                .suffix(" s")
                                .prefix("In "),
                        )
                        .changed();
                    let mut out_enabled = playback.out_point.is_some();
                    if ui.checkbox(&mut out_enabled, "Out").changed() {
                        playback.out_point = out_enabled.then_some(maximum);
                        changed = true;
                    }
                    if let Some(out_point) = &mut playback.out_point {
                        changed |= ui
                            .add(
                                egui::DragValue::new(out_point)
                                    .range(0.001..=maximum)
                                    .speed(0.05)
                                    .suffix(" s"),
                            )
                            .changed();
                    }
                });
                ui.horizontal(|ui| {
                    let mut beat_enabled = playback.beat_duration.is_some();
                    if ui
                        .checkbox(&mut beat_enabled, "BPM-relative duration")
                        .changed()
                    {
                        playback.beat_duration = beat_enabled.then_some(4.0);
                        changed = true;
                    }
                    if let Some(beats) = &mut playback.beat_duration {
                        changed |= ui
                            .add(
                                egui::DragValue::new(beats)
                                    .range(0.0625..=256.0)
                                    .speed(0.25)
                                    .suffix(" beats"),
                            )
                            .changed();
                    }
                    if let Some(beats) = playback.beat_duration {
                        ui.weak(format!(
                            "{:.3} s at {:.1} BPM",
                            beats * 60.0 / state.bpm,
                            state.bpm
                        ));
                    }
                });
                let (start, end) = playback.range(media_duration, state.bpm);
                ui.weak(match end {
                    Some(end) => format!("Effective range {start:.3}–{end:.3} s"),
                    None => format!("Effective range starts at {start:.3} s"),
                });
            });
        if changed {
            clips.set_playback(address, playback);
        }
    } else if let Some(path) = clips.path(address) {
        let path = path.to_path_buf();
        egui::CollapsingHeader::new(format!(
            "Missing media · Deck {} slot {}",
            deck.label(),
            address.slot + 1
        ))
        .default_open(true)
        .show(ui, |ui| {
            ui.colored_label(palette.danger, path.display().to_string());
            if ui.button("Browse and relink…").clicked() {
                actions.push(UiAction::BrowseRelink(address));
            }
            ui.weak("Trim, launch mode and beat-duration settings will be preserved.");
        });
    }
}
