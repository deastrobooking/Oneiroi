//! egui overlay.
//!
//! The UI never touches the GPU or mutates render state directly. It edits
//! plain values that get read into a per-frame snapshot, which is the same
//! path the parameter/modulation system takes later.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oneiroi_core::{FrameTime, Quantization, TempoClock};
use oneiroi_media::{
    CLIPS_PER_DECK, CameraDevice, ClipAddress, ClipBank, CrossfadeBus, DeckId, DeckState,
    DeckTransport, EndMode, FourDeckMixer, LaunchQueue, MediaHealth,
};
use oneiroi_render::{
    DeckEffects, DeckLfos, DeckTransform, EffectTarget, LayerBlendMode, LfoWaveform, SourceMode,
};

/// Everything the overlay owns. All plain data — no GPU handles, no channels.
pub struct UiState {
    pub master_opacity: f32,
    pub blackout: bool,
    pub master_freeze: bool,
    pub crossfader: f32,
    pub equal_power: bool,
    pub output_enabled: bool,
    pub output_fullscreen: bool,
    pub output_display_id: String,
    pub output_test_card: bool,
    pub output_identify: bool,
    pub composition_extent: [u32; 2],
    pub custom_composition_extent: [u32; 2],
    pub effects: [DeckEffects; 4],
    pub transforms: [DeckTransform; 4],
    pub blend_modes: [LayerBlendMode; 4],
    pub solo: [bool; 4],
    pub bypassed: [bool; 4],
    pub lfos: [DeckLfos; 4],
    pub bpm: f64,
    pub quantization: Quantization,
    pub project_path: String,
    pub camera_device_id: String,
    pub camera_width: u32,
    pub camera_height: u32,
    pub camera_fps: u32,
    thumbnails: HashMap<ClipAddress, CachedThumbnail>,
    thumbnail_failures: HashMap<ClipAddress, (PathBuf, String)>,
    fps: FpsMeter,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            master_opacity: 1.0,
            blackout: false,
            master_freeze: false,
            crossfader: 0.5,
            equal_power: true,
            output_enabled: true,
            output_fullscreen: false,
            output_display_id: String::new(),
            output_test_card: false,
            output_identify: false,
            composition_extent: [1920, 1080],
            custom_composition_extent: [1920, 1080],
            effects: [DeckEffects::default(); 4],
            transforms: [DeckTransform::default(); 4],
            blend_modes: [LayerBlendMode::Normal; 4],
            solo: [false; 4],
            bypassed: [false; 4],
            lfos: [DeckLfos::default(); 4],
            bpm: 120.0,
            quantization: Quantization::Immediate,
            project_path: "show.oneiroi".to_owned(),
            camera_device_id: "0".to_owned(),
            camera_width: 1280,
            camera_height: 720,
            camera_fps: 30,
            thumbnails: HashMap::new(),
            thumbnail_failures: HashMap::new(),
            fps: FpsMeter::default(),
        }
    }
}

struct CachedThumbnail {
    path: PathBuf,
    texture: egui::TextureHandle,
}

impl UiState {
    pub fn install_thumbnail(
        &mut self,
        ctx: &egui::Context,
        address: ClipAddress,
        path: PathBuf,
        thumbnail: oneiroi_media::Thumbnail,
    ) {
        let [width, height] = thumbnail.extent;
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            &thumbnail.rgba,
        );
        let texture = ctx.load_texture(
            format!("clip-thumbnail-{}-{}", address.deck.label(), address.slot),
            image,
            egui::TextureOptions::LINEAR,
        );
        self.thumbnails
            .insert(address, CachedThumbnail { path, texture });
        self.thumbnail_failures.remove(&address);
    }

    pub fn mark_thumbnail_failed(&mut self, address: ClipAddress, path: PathBuf, message: String) {
        self.thumbnails.remove(&address);
        self.thumbnail_failures.insert(address, (path, message));
    }

    pub fn clear_thumbnail(&mut self, address: ClipAddress) {
        self.thumbnails.remove(&address);
        self.thumbnail_failures.remove(&address);
    }

    pub fn clear_thumbnails(&mut self) {
        self.thumbnails.clear();
        self.thumbnail_failures.clear();
    }

    fn thumbnail(&self, address: ClipAddress, path: Option<&Path>) -> Option<&egui::TextureHandle> {
        let cached = self.thumbnails.get(&address)?;
        (Some(cached.path.as_path()) == path).then_some(&cached.texture)
    }

    fn thumbnail_failure(&self, address: ClipAddress, path: Option<&Path>) -> Option<&str> {
        let (failed_path, message) = self.thumbnail_failures.get(&address)?;
        (Some(failed_path.as_path()) == path).then_some(message.as_str())
    }
}

/// Exponentially smoothed frame rate.
///
/// Instantaneous 1/delta is unreadable and a rolling window costs an
/// allocation; neither is worth it for a number a human reads.
#[derive(Default)]
struct FpsMeter {
    smoothed_delta: f64,
}

impl FpsMeter {
    fn push(&mut self, delta: f64) {
        if delta <= 0.0 {
            return;
        }
        if self.smoothed_delta == 0.0 {
            self.smoothed_delta = delta;
        } else {
            self.smoothed_delta += (delta - self.smoothed_delta) * 0.1;
        }
    }

    fn fps(&self) -> f64 {
        if self.smoothed_delta > 0.0 {
            1.0 / self.smoothed_delta
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug)]
pub enum UiAction {
    Restart(DeckId),
    Seek(DeckId),
    Launch(ClipAddress),
    LaunchScene(usize),
    ClearSlot(ClipAddress),
    Eject(DeckId),
    SaveProject,
    OpenProject,
    RecoverProject,
    TapTempo,
    HalfTempo,
    DoubleTempo,
    SetOutputEnabled(bool),
    SetOutputFullscreen(bool),
    SetOutputDisplay(String),
    SetCompositionExtent([u32; 2]),
    RefreshDisplays,
    RefreshCameras,
    ConnectCamera {
        deck: DeckId,
        device_id: String,
        label: String,
        extent: [u32; 2],
        fps: u32,
    },
}

#[derive(Clone, Debug)]
pub struct OutputDisplay {
    pub id: String,
    pub label: String,
}

pub struct OutputHealthMetrics<'a> {
    pub status: &'a str,
    pub current_display: &'a str,
    pub surface_extent: [u32; 2],
    pub presented: u64,
    pub skipped: u64,
    pub reconfigurations: u64,
    pub recoveries: u64,
    pub timeouts: u64,
    pub occlusions: u64,
    pub validation_errors: u64,
    pub topology_changes: u64,
}

pub struct PerformanceMetrics<'a> {
    pub tempo: TempoClock,
    pub now_seconds: f64,
    pub scheduler_stats: [oneiroi_media::SchedulerStats; 4],
    pub frame_time: &'a FrameTime,
    pub gpu_info: &'a str,
    pub project_dirty: bool,
    pub project_status: &'a str,
    pub recovery_available: bool,
    pub cameras: &'a [CameraDevice],
    pub camera_status: &'a str,
    pub output_displays: &'a [OutputDisplay],
    pub output_health: OutputHealthMetrics<'a>,
}

pub fn draw(
    ctx: &egui::Context,
    state: &mut UiState,
    mixer: &mut FourDeckMixer,
    clips: &mut ClipBank,
    launches: &LaunchQueue,
    transports: &mut [DeckTransport; 4],
    metrics: PerformanceMetrics<'_>,
) -> Vec<UiAction> {
    state.fps.push(metrics.frame_time.delta);
    let mut actions = Vec::new();

    egui::Window::new("oneiroi")
        .default_pos([16.0, 16.0])
        .default_size([920.0, 520.0])
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("ONEIROI · FOUR DECK VIDEO MIXER");
                ui.separator();
                ui.label(metrics.gpu_info);
                ui.separator();
                if ui
                    .checkbox(&mut state.output_enabled, "Program output")
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
            });
            ui.horizontal(|ui| {
                ui.label("Output");
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
            egui::CollapsingHeader::new("Output health")
                .default_open(false)
                .show(ui, |ui| {
                    let health = &metrics.output_health;
                    let (status, color) = if !state.output_enabled {
                        ("Disabled", egui::Color32::GRAY)
                    } else if metrics.output_displays.is_empty() {
                        ("No connected display", egui::Color32::RED)
                    } else if health.status == "Healthy" {
                        (health.status, egui::Color32::LIGHT_GREEN)
                    } else if health.validation_errors > 0 {
                        (health.status, egui::Color32::RED)
                    } else {
                        (health.status, egui::Color32::YELLOW)
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(color, status);
                        ui.separator();
                        ui.label(format!(
                            "surface {} × {} · composition {} × {} · FIFO",
                            health.surface_extent[0],
                            health.surface_extent[1],
                            state.composition_extent[0],
                            state.composition_extent[1]
                        ));
                    });
                    ui.label(format!("Display: {}", health.current_display));
                    ui.label(format!(
                        "presented {} · skipped {} · recovered {} · reconfigured {}",
                        health.presented,
                        health.skipped,
                        health.recoveries,
                        health.reconfigurations
                    ));
                    ui.weak(format!(
                        "timeouts {} · occluded {} · validation errors {} · display changes {}",
                        health.timeouts,
                        health.occlusions,
                        health.validation_errors,
                        health.topology_changes
                    ));
                });
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
            ui.separator();

            ui.horizontal(|ui| {
                ui.label(format!("{:.1} fps", state.fps.fps()));
                ui.label(format!("frame {}", metrics.frame_time.frame));
                ui.label(format!("{:.2}s", metrics.frame_time.elapsed));
                ui.separator();
                ui.label("Select a deck, then drag a movie onto this window.");
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut state.bpm)
                        .range(20.0..=400.0)
                        .speed(0.25)
                        .suffix(" BPM"),
                );
                if ui.button("Tap").clicked() {
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
                let dropped: u64 = metrics
                    .scheduler_stats
                    .iter()
                    .map(|stats| stats.dropped)
                    .sum();
                let repeated: u64 = metrics
                    .scheduler_stats
                    .iter()
                    .map(|stats| stats.repeated)
                    .sum();
                let late: u64 = metrics.scheduler_stats.iter().map(|stats| stats.late).sum();
                ui.separator();
                ui.label(format!("drop {dropped} · repeat {repeated} · late {late}"));
            });
            draw_clip_grid(ui, state, mixer, clips, launches, &mut actions);

            ui.separator();
            egui::Grid::new("four-decks")
                .num_columns(2)
                .spacing([12.0, 12.0])
                .show(ui, |ui| {
                    for (index, deck_id) in DeckId::ALL.into_iter().enumerate() {
                        draw_deck(
                            ui,
                            mixer,
                            deck_id,
                            DeckControls {
                                transport: &mut transports[deck_id.index()],
                                transform: &mut state.transforms[deck_id.index()],
                                blend_mode: &mut state.blend_modes[deck_id.index()],
                                solo: &mut state.solo[deck_id.index()],
                                bypassed: &mut state.bypassed[deck_id.index()],
                                effects: &mut state.effects[deck_id.index()],
                                lfos: &mut state.lfos[deck_id.index()],
                            },
                            &mut actions,
                        );
                        if index % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("A");
                ui.add(
                    egui::Slider::new(&mut state.crossfader, 0.0..=1.0)
                        .text("crossfader")
                        .clamping(egui::SliderClamping::Always),
                );
                ui.label("B");
                ui.checkbox(&mut state.equal_power, "equal power");
                if ui.button("Center").clicked() {
                    state.crossfader = 0.5;
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::Slider::new(&mut state.master_opacity, 0.0..=1.0)
                        .text("master")
                        .clamping(egui::SliderClamping::Always),
                );
                if ui.selectable_label(state.blackout, "BLACKOUT").clicked() {
                    state.blackout = !state.blackout;
                }
                ui.checkbox(&mut state.master_freeze, "master freeze");
            });
        });
    actions
}

fn draw_clip_grid(
    ui: &mut egui::Ui,
    state: &UiState,
    mixer: &mut FourDeckMixer,
    clips: &mut ClipBank,
    launches: &LaunchQueue,
    actions: &mut Vec<UiAction>,
) {
    egui::Grid::new("clip-grid")
        .num_columns(CLIPS_PER_DECK + 1)
        .spacing([5.0, 5.0])
        .show(ui, |ui| {
            ui.strong("SCENE");
            for slot in 0..CLIPS_PER_DECK {
                if ui.small_button(format!("{}", slot + 1)).clicked() {
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
                        } else {
                            short
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
                        .selected(selected || active);
                    let response = ui.add_sized([96.0, 38.0], button);
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
                            if let Some(error) =
                                state.thumbnail_failure(address, clips.path(address))
                            {
                                details.push_str(&format!("\nThumbnail unavailable: {error}"));
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
                            if clips.path(address).is_some() && ui.button("Clear slot").clicked() {
                                actions.push(UiAction::ClearSlot(address));
                                ui.close();
                            }
                        });
                }
                ui.end_row();
            }
        });
}

struct DeckControls<'a> {
    transport: &'a mut DeckTransport,
    transform: &'a mut DeckTransform,
    blend_mode: &'a mut LayerBlendMode,
    solo: &'a mut bool,
    bypassed: &'a mut bool,
    effects: &'a mut DeckEffects,
    lfos: &'a mut DeckLfos,
}

fn draw_deck(
    ui: &mut egui::Ui,
    mixer: &mut FourDeckMixer,
    id: DeckId,
    controls: DeckControls<'_>,
    actions: &mut Vec<UiAction>,
) {
    let DeckControls {
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
            ui.visuals().selection.bg_fill.linear_multiply(0.35)
        } else {
            ui.visuals().faint_bg_color
        })
        .inner_margin(10.0);

    frame.show(ui, |ui| {
        ui.set_min_size([420.0, 165.0].into());
        ui.horizontal(|ui| {
            if ui
                .selectable_label(selected, format!("DECK {}", id.label()))
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
                    for mode in [
                        LayerBlendMode::Normal,
                        LayerBlendMode::Add,
                        LayerBlendMode::Screen,
                        LayerBlendMode::Multiply,
                        LayerBlendMode::Difference,
                        LayerBlendMode::Lighten,
                        LayerBlendMode::Darken,
                        LayerBlendMode::Overlay,
                    ] {
                        ui.selectable_value(blend_mode, mode, blend_mode_label(mode));
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
            let mut progress = (transport.position / duration).clamp(0.0, 1.0) as f32;
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
                            .selected_text(format!("LFO {}", route.source + 1))
                            .show_ui(ui, |ui| {
                                for source in 0..3 {
                                    ui.selectable_value(
                                        &mut route.source,
                                        source,
                                        format!("LFO {}", source + 1),
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

const EFFECT_TARGETS: [EffectTarget; 14] = [
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
];

const LFO_WAVEFORMS: [LfoWaveform; 5] = [
    LfoWaveform::Sine,
    LfoWaveform::Triangle,
    LfoWaveform::Saw,
    LfoWaveform::SawDown,
    LfoWaveform::Square,
];

const BEAT_DIVISIONS: [(f32, &str); 8] = [
    (0.0625, "1/16 beat"),
    (0.125, "1/8 beat"),
    (0.25, "1/4 beat"),
    (0.5, "1/2 beat"),
    (1.0, "1 beat"),
    (2.0, "2 beats"),
    (4.0, "4 beats"),
    (8.0, "8 beats"),
];

fn effect_target_label(target: EffectTarget) -> &'static str {
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
    }
}

fn blend_mode_label(mode: LayerBlendMode) -> &'static str {
    match mode {
        LayerBlendMode::Normal => "Normal",
        LayerBlendMode::Add => "Add",
        LayerBlendMode::Screen => "Screen",
        LayerBlendMode::Multiply => "Multiply",
        LayerBlendMode::Difference => "Difference",
        LayerBlendMode::Lighten => "Lighten",
        LayerBlendMode::Darken => "Darken",
        LayerBlendMode::Overlay => "Overlay",
    }
}

fn waveform_label(waveform: LfoWaveform) -> &'static str {
    match waveform {
        LfoWaveform::Sine => "Sine",
        LfoWaveform::Triangle => "Triangle",
        LfoWaveform::Saw => "Saw up",
        LfoWaveform::SawDown => "Saw down",
        LfoWaveform::Square => "Square",
    }
}

fn beat_division_label(beats: f32) -> &'static str {
    BEAT_DIVISIONS
        .iter()
        .find(|(candidate, _)| (*candidate - beats).abs() < f32::EPSILON)
        .map_or("Custom", |(_, label)| *label)
}
