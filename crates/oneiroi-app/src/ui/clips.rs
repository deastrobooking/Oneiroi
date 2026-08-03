//! Clip grid: slots, health, launch and thumbnail rendering.

use super::*;

pub(super) struct ClipGridContext<'a> {
    pub launches: &'a LaunchQueue,
    pub midi_map: &'a MidiMapUi,
    pub cameras: &'a [CameraDevice],
    pub camera_status: &'a str,
    pub camera_recordings: [CameraRecordingStatus; 4],
}

pub(super) fn draw_clip_grid(
    ui: &mut egui::Ui,
    state: &mut UiState,
    mixer: &mut FourDeckMixer,
    clips: &mut ClipBank,
    context: ClipGridContext<'_>,
    actions: &mut Vec<UiAction>,
) {
    let launches = context.launches;
    let midi_map = context.midi_map;
    let cameras = context.cameras;
    let camera_status = context.camera_status;
    let camera_recordings = context.camera_recordings;
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
                if ui
                    .selectable_label(
                        mixer.selected() == deck,
                        egui::RichText::new(format!("DECK {}", deck.label())).strong(),
                    )
                    .on_hover_text("Select this deck's performance controls")
                    .clicked()
                {
                    mixer.select(deck);
                }
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
                    let draggable = !state.show_mode
                        && !midi_map.active
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
                            if !state.show_mode {
                                details.push_str(
                                    "\nDrag onto another slot to move; occupied slots swap",
                                );
                            }
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
                            if state.show_mode {
                                ui.weak("Show Mode locks media management");
                                return;
                            }
                            if clips.path(address).is_some() && ui.button("Relink media…").clicked()
                            {
                                actions.push(UiAction::BrowseRelink(address));
                                ui.close();
                            }
                            if clips.path(address).is_some() && ui.button("Delete clip").clicked() {
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
    let selected_occupied = clips.slot(address).is_some_and(|slot| {
        slot.movie.is_some() || slot.pending_path.is_some() || slot.error.is_some()
    });
    ui.horizontal(|ui| {
        ui.strong(format!(
            "Selected · Deck {} clip {}",
            deck.label(),
            address.slot + 1
        ));
        let delete = ui.add_enabled(
            selected_occupied && !state.show_mode,
            egui::Button::new("Delete selected clip")
                .fill(palette.control_tint(palette.danger, 0.28)),
        );
        if delete
            .on_hover_text(if state.show_mode {
                "Exit Show Mode to manage clips"
            } else {
                "Delete this slot · keyboard: Delete or Backspace"
            })
            .clicked()
        {
            actions.push(UiAction::ClearSlot(address));
        }
    });

    let recording = camera_recordings[deck.index()];
    ui.horizontal_wrapped(|ui| {
        ui.strong("Deck input");
        egui::ComboBox::from_id_salt(("clip-camera-device", deck.index()))
            .selected_text(
                cameras
                    .iter()
                    .find(|camera| camera.id == state.camera_device_id)
                    .map_or(state.camera_device_id.as_str(), |camera| {
                        camera.label.as_str()
                    }),
            )
            .show_ui(ui, |ui| {
                for camera in cameras {
                    ui.selectable_value(
                        &mut state.camera_device_id,
                        camera.id.clone(),
                        &camera.label,
                    );
                }
            });
        if ui.button("Refresh").clicked() {
            actions.push(UiAction::RefreshCameras);
        }
        let can_switch = !state.camera_device_id.trim().is_empty();
        if ui
            .add_enabled(
                can_switch,
                egui::Button::new(format!("Switch Deck {}", deck.label())),
            )
            .on_hover_text("Use the selected camera as this deck's live input")
            .clicked()
        {
            let label = cameras
                .iter()
                .find(|camera| camera.id == state.camera_device_id)
                .map_or_else(
                    || format!("Camera {}", state.camera_device_id),
                    |camera| camera.label.clone(),
                );
            actions.push(UiAction::ConnectCamera {
                deck,
                device_id: state.camera_device_id.clone(),
                label,
                extent: [state.camera_width, state.camera_height],
                fps: state.camera_fps,
            });
        }
        if recording.address.is_some() {
            let label = if recording.finalizing {
                "Finalizing…".to_owned()
            } else {
                format!("■ Stop · {:.1}s", recording.elapsed_seconds)
            };
            if ui
                .add_enabled(!recording.finalizing, egui::Button::new(label))
                .clicked()
            {
                actions.push(UiAction::StopCameraRecording(deck));
            }
            if recording.dropped_frames > 0 {
                ui.colored_label(
                    palette.warning,
                    format!("{} dropped", recording.dropped_frames),
                );
            }
        } else {
            let live = matches!(mixer.deck(deck).state, DeckState::Live(_));
            let can_record = live && !selected_occupied;
            if ui
                .add_enabled(
                    can_record,
                    egui::Button::new("● Record clip")
                        .fill(palette.control_tint(palette.danger, 0.28)),
                )
                .on_hover_text(if !live {
                    "Switch this deck to a camera first"
                } else if selected_occupied {
                    "Select an empty clip slot to record into"
                } else {
                    "Record the live camera into the selected clip slot"
                })
                .clicked()
            {
                actions.push(UiAction::StartCameraRecording(address));
            }
        }
        if !camera_status.is_empty() {
            ui.weak(camera_status);
        }
    });

    if state.show_mode {
        return;
    }
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
