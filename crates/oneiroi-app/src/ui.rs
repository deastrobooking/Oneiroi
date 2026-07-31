//! egui overlay.
//!
//! The UI never touches the GPU or mutates render state directly. It edits
//! plain values that get read into a per-frame snapshot, which is the same
//! path the parameter/modulation system takes later.

mod clips;
mod deck;
mod master_fx;
mod midi;

use clips::draw_clip_grid;
use deck::{DeckControls, draw_deck};
use master_fx::{draw_custom_effect, draw_master_modulation};
use midi::draw_midi;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oneiroi_core::{
    AudioAnalysisSettings, ControlTarget, FrameTime, MappingMode, MidiMapper, Quantization,
    TempoClock, effect_parameter_key,
};
use oneiroi_io::{AudioInputDevice, AudioInputSnapshot, MidiInputDevice, MidiInputStats};
use oneiroi_media::{
    CLIPS_PER_DECK, CameraDevice, ClipAddress, ClipBank, ClipLaunchMode, CrossfadeBus, DeckId,
    DeckState, DeckTransport, EndMode, FourDeckMixer, LaunchQueue, MediaHealth,
};
use oneiroi_render::{
    DeckEffects, DeckLfos, DeckTransform, EffectDescriptor, EffectHistoryResource,
    EffectParameterValue, EffectPreset, EffectTarget, LayerBlendMode, LfoWaveform,
    MasterEffectChain, MasterEffectKind, MasterEffectSlot, MasterModulation, SourceMode,
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
    pub master_effects: MasterEffectChain,
    pub effect_manifest_path: String,
    pub effect_reload_status: String,
    pub effect_packages: Vec<EffectDescriptor>,
    pub effect_registry_status: String,
    pub master_modulation: MasterModulation,
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
    pub audio_device_id: String,
    pub audio_analysis: AudioAnalysisSettings,
    pub midi_device_id: String,
    pub midi_target: ControlTarget,
    pub session_recovery_selected: usize,
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
            master_effects: MasterEffectChain::default(),
            effect_manifest_path: "effects/master-effects/effect.json".to_owned(),
            effect_reload_status: "Built-in master effect pipeline".to_owned(),
            effect_packages: Vec::new(),
            effect_registry_status: "Effect registry not scanned".to_owned(),
            master_modulation: MasterModulation::default(),
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
            audio_device_id: String::new(),
            audio_analysis: AudioAnalysisSettings::default(),
            midi_device_id: String::new(),
            midi_target: ControlTarget::Crossfader,
            session_recovery_selected: 0,
            thumbnails: HashMap::new(),
            thumbnail_failures: HashMap::new(),
            fps: FpsMeter::default(),
        }
    }
}

struct CachedThumbnail {
    path: PathBuf,
    texture: egui::TextureHandle,
    preload: oneiroi_media::RgbaFrame,
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
        self.thumbnails.insert(
            address,
            CachedThumbnail {
                path,
                texture,
                preload: thumbnail.preload,
            },
        );
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

    pub fn preloaded_frame(
        &self,
        address: ClipAddress,
        path: Option<&Path>,
    ) -> Option<&oneiroi_media::RgbaFrame> {
        let cached = self.thumbnails.get(&address)?;
        (Some(cached.path.as_path()) == path).then_some(&cached.preload)
    }

    fn preloaded_count(&self) -> usize {
        self.thumbnails.len()
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
    BrowseRelink(ClipAddress),
    Eject(DeckId),
    SaveProject,
    OpenProject,
    RecoverProject,
    RefreshSessionRecoveries,
    RestoreSessionRecovery(usize),
    TapTempo,
    HalfTempo,
    DoubleTempo,
    SetOutputEnabled(bool),
    SetOutputFullscreen(bool),
    SetOutputDisplay(String),
    SetCompositionExtent([u32; 2]),
    WatchEffectManifest,
    ReloadEffectManifest,
    RefreshEffectRegistry,
    RefreshDisplays,
    RefreshCameras,
    RefreshAudioInputs,
    ConnectAudioInput(String),
    DisconnectAudioInput,
    RefreshMidiInputs,
    ConnectMidiInput(String),
    DisconnectMidiInput,
    MidiLearn(ControlTarget),
    MidiCancelLearn,
    MidiClearTarget(ControlTarget),
    MidiRemoveBinding(usize),
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
    pub frame_pool_stats: [oneiroi_media::FramePoolStats; 4],
    pub frame_time: &'a FrameTime,
    pub gpu_info: &'a str,
    pub runtime_status: &'a str,
    pub project_dirty: bool,
    pub project_status: &'a str,
    pub folder_status: &'a str,
    pub recovery_available: bool,
    pub session_recoveries: &'a [crate::recovery::RecoveryEntry],
    pub session_recovery_status: &'a str,
    pub cameras: &'a [CameraDevice],
    pub camera_status: &'a str,
    pub audio_inputs: &'a [AudioInputDevice],
    pub audio_status: &'a str,
    pub audio_connected: bool,
    pub audio_snapshot: AudioInputSnapshot,
    pub midi: MidiMetrics<'a>,
    pub output_displays: &'a [OutputDisplay],
    pub output_health: OutputHealthMetrics<'a>,
}

pub struct MidiMetrics<'a> {
    pub inputs: &'a [MidiInputDevice],
    pub status: &'a str,
    pub connected: bool,
    pub stats: MidiInputStats,
    pub mapper: &'a mut MidiMapper,
}

pub fn draw(
    ctx: &egui::Context,
    state: &mut UiState,
    mixer: &mut FourDeckMixer,
    clips: &mut ClipBank,
    launches: &LaunchQueue,
    transports: &mut [DeckTransport; 4],
    mut metrics: PerformanceMetrics<'_>,
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
            ui.weak(metrics.runtime_status);
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
                            if ui.button("Restore take").clicked() {
                                actions.push(UiAction::RestoreSessionRecovery(
                                    state.session_recovery_selected,
                                ));
                            }
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
            draw_midi(ui, state, &mut metrics.midi, &mut actions);
            ui.separator();

            ui.horizontal(|ui| {
                ui.label(format!("{:.1} fps", state.fps.fps()));
                ui.label(format!("frame {}", metrics.frame_time.frame));
                ui.label(format!("{:.2}s", metrics.frame_time.elapsed));
                ui.separator();
                ui.label("Select a slot, then drag a movie or folder onto this window.");
                if !metrics.folder_status.is_empty() {
                    ui.separator();
                    ui.weak(metrics.folder_status);
                }
                ui.separator();
                ui.label(format!("first-frame ready {}/32", state.preloaded_count()));
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
                let allocated: u64 = metrics
                    .frame_pool_stats
                    .iter()
                    .map(|stats| stats.allocations)
                    .sum();
                let reused: u64 = metrics
                    .frame_pool_stats
                    .iter()
                    .map(|stats| stats.reuses)
                    .sum();
                let in_flight: u64 = metrics
                    .frame_pool_stats
                    .iter()
                    .map(|stats| stats.in_flight)
                    .sum();
                let discarded: u64 = metrics
                    .frame_pool_stats
                    .iter()
                    .map(|stats| stats.discarded)
                    .sum();
                let bytes: u64 = metrics
                    .frame_pool_stats
                    .iter()
                    .map(|stats| stats.allocated_bytes)
                    .sum();
                ui.separator();
                ui.label(format!(
                    "RGBA pool alloc {allocated} · reuse {reused} · live {in_flight} · discard {discarded} · {:.1} MiB",
                    bytes as f64 / (1024.0 * 1024.0)
                ));
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
            egui::CollapsingHeader::new("Master effects")
                .default_open(false)
                .show(ui, |ui| {
                    let effect_packages = &state.effect_packages;
                    let master_effects = &mut state.master_effects;
                    let slot_count = master_effects.slots.len();
                    let mut reorder = None;
                    for index in 0..slot_count {
                        ui.group(|ui| {
                            let slot = &mut master_effects.slots[index];
                            ui.horizontal(|ui| {
                                ui.monospace(format!("{}", index + 1));
                                egui::ComboBox::from_id_salt(format!("master-fx-kind-{index}"))
                                    .selected_text(slot.kind.label())
                                    .show_ui(ui, |ui| {
                                        for kind in MasterEffectKind::ALL {
                                            ui.selectable_value(
                                                &mut slot.kind,
                                                kind,
                                                kind.label(),
                                            );
                                        }
                                    });
                                ui.checkbox(&mut slot.bypassed, "Bypass");
                                ui.add(
                                    egui::Slider::new(&mut slot.mix, 0.0..=1.0).text("wet"),
                                );
                                if ui
                                    .add_enabled(index > 0, egui::Button::new("↑"))
                                    .clicked()
                                {
                                    reorder = Some((index, index - 1));
                                }
                                if ui
                                    .add_enabled(
                                        index + 1 < slot_count,
                                        egui::Button::new("↓"),
                                    )
                                    .clicked()
                                {
                                    reorder = Some((index, index + 1));
                                }
                            });
                            if slot.kind == MasterEffectKind::Blur {
                                ui.add(
                                    egui::Slider::new(&mut slot.amount, 0.0..=32.0)
                                        .text("radius px"),
                                );
                            } else if slot.kind == MasterEffectKind::Feedback {
                                ui.add(
                                    egui::Slider::new(&mut slot.feedback, 0.0..=0.99)
                                        .text("persistence"),
                                );
                            } else if slot.kind == MasterEffectKind::Custom {
                                draw_custom_effect(
                                    ui,
                                    index,
                                    slot,
                                    effect_packages,
                                    &mut actions,
                                );
                            }
                        });
                    }
                    if let Some((from, to)) = reorder {
                        master_effects.slots.swap(from, to);
                    }
                    if ui.button("Reset master effects").clicked() {
                        *master_effects = MasterEffectChain::default();
                    }
                    master_effects.sanitize();
                    ui.weak(
                        "Blur uses fixed ping-pong textures allocated with the composition target.",
                    );
                    draw_master_modulation(
                        ui,
                        &mut state.master_modulation,
                        master_effects,
                        effect_packages,
                    );
                    ui.separator();
                    ui.label("Effect package");
                    ui.text_edit_singleline(&mut state.effect_manifest_path);
                    ui.horizontal(|ui| {
                        if ui.button("Refresh registry").clicked() {
                            actions.push(UiAction::RefreshEffectRegistry);
                        }
                        if ui.button("Watch").clicked() {
                            actions.push(UiAction::WatchEffectManifest);
                        }
                        if ui.button("Reload now").clicked() {
                            actions.push(UiAction::ReloadEffectManifest);
                        }
                    });
                    ui.weak(&state.effect_registry_status);
                    if state.effect_reload_status.contains("rejected") {
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 190, 80),
                            &state.effect_reload_status,
                        );
                    } else {
                        ui.weak(&state.effect_reload_status);
                    }
                });
        });
    actions
}

const LFO_WAVEFORMS: [LfoWaveform; 5] = [
    LfoWaveform::Sine,
    LfoWaveform::Triangle,
    LfoWaveform::Saw,
    LfoWaveform::SawDown,
    LfoWaveform::Square,
];

fn waveform_label(waveform: LfoWaveform) -> &'static str {
    match waveform {
        LfoWaveform::Sine => "Sine",
        LfoWaveform::Triangle => "Triangle",
        LfoWaveform::Saw => "Saw up",
        LfoWaveform::SawDown => "Saw down",
        LfoWaveform::Square => "Square",
    }
}

fn deck_label(deck: u8) -> char {
    char::from(b'A'.saturating_add(deck.min(3)))
}

fn effect_parameter_label(effect: u8) -> &'static str {
    [
        "Hue",
        "Contrast",
        "Saturation",
        "Black level",
        "White level",
        "Gamma",
        "Pixelate",
        "Luma key",
        "Neon",
        "Fractal",
        "Jitter",
        "Find edges",
        "Bit reduction",
        "Black light",
    ]
    .get(usize::from(effect))
    .copied()
    .unwrap_or("Unknown")
}

fn mapping_mode_label(mode: MappingMode) -> &'static str {
    match mode {
        MappingMode::Continuous => "Absolute",
        MappingMode::Momentary => "Momentary",
        MappingMode::Toggle => "Toggle",
        MappingMode::RelativeBinaryOffset => "Relative offset",
        MappingMode::RelativeTwosComplement => "Relative 2's comp",
    }
}
