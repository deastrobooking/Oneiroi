//! egui overlay.
//!
//! The UI never touches the GPU or mutates render state directly. It edits
//! plain values that get read into a per-frame snapshot, which is the same
//! path the parameter/modulation system takes later.

mod clips;
mod deck;
mod diagnostics;
mod master_fx;
mod midi;
mod midi_manager;
mod setup;
pub mod theme;
mod toolbar;

use clips::{ClipGridContext, draw_clip_grid};
use deck::{DeckControls, draw_deck};
use master_fx::{draw_custom_effect, draw_master_modulation};
use midi::draw_midi;
use midi_manager::draw_midi_manager;
use setup::draw_setup;
use theme::{CASCADE_STRIP_WIDTH, ResolvedDeckLayout, ThemePalette, ThemeState};
use toolbar::draw_toolbar;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oneiroi_core::{
    AudioAnalysisSettings, ControlTarget, FIXED_DECK_EFFECT_PARAMETER_COUNT, FrameTime,
    MappingMode, MidiMapper, Quantization, TempoClock, effect_parameter_key,
};
use oneiroi_io::{AudioInputDevice, AudioInputSnapshot, MidiInputDevice, MidiInputStats};
use oneiroi_media::{
    CLIPS_PER_DECK, CameraDevice, ClipAddress, ClipBank, ClipLaunchMode, CrossfadeBus, DeckId,
    DeckState, DeckTransport, EndMode, FourDeckMixer, LaunchQueue, MediaHealth,
};
use oneiroi_render::{
    BlendModeGroup, DeckEffects, DeckLfos, DeckTransform, EffectDescriptor, EffectHistoryResource,
    EffectParameterControl, EffectParameterValue, EffectPreset, EffectTarget, LayerBlendMode,
    LfoWaveform, MasterEffectChain, MasterEffectKind, MasterEffectSlot, MasterModulation,
    SourceMode,
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
    /// Packages executable by the current master-v1 runtime.
    pub effect_packages: Vec<EffectDescriptor>,
    /// Validated deck-v1 catalog entries. Selection stays hidden until the
    /// independently executable deck branch is implemented.
    pub deck_effect_packages: Vec<EffectDescriptor>,
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
    pub osc_bind_address: String,
    pub osc_feedback_address: String,
    pub session_recovery_selected: usize,
    pub take_name_input: String,
    pub random_seed_scope: String,
    pub random_seed_value: u64,
    pub session_replay_seconds: f64,
    pub project_take_selected: usize,
    pub timeline_marker_input: String,
    pub take_export_directory: String,
    pub theme: ThemeState,
    /// Performance lock: keeps launch/mix/transport controls live while
    /// hiding setup and structural editors that can destabilize a show.
    pub show_mode: bool,
    pub midi_map_mode: bool,
    pub midi_manager_open: bool,
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
            deck_effect_packages: Vec::new(),
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
            osc_bind_address: "0.0.0.0:9000".to_owned(),
            osc_feedback_address: "127.0.0.1:9001".to_owned(),
            session_recovery_selected: 0,
            take_name_input: "Take 1".to_owned(),
            random_seed_scope: "visuals".to_owned(),
            random_seed_value: 1,
            session_replay_seconds: 0.0,
            project_take_selected: 0,
            timeline_marker_input: String::new(),
            take_export_directory: "take-exports".to_owned(),
            theme: ThemeState::default(),
            show_mode: false,
            midi_map_mode: false,
            midi_manager_open: false,
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

    /// Exchange the cached previews of two slots after a clip move. Entries
    /// stay validated by media path, so a mismatch simply re-requests.
    pub fn swap_thumbnails(&mut self, a: ClipAddress, b: ClipAddress) {
        let first = self.thumbnails.remove(&a);
        let second = self.thumbnails.remove(&b);
        if let Some(entry) = first {
            self.thumbnails.insert(b, entry);
        }
        if let Some(entry) = second {
            self.thumbnails.insert(a, entry);
        }
        let first = self.thumbnail_failures.remove(&a);
        let second = self.thumbnail_failures.remove(&b);
        if let Some(entry) = first {
            self.thumbnail_failures.insert(b, entry);
        }
        if let Some(entry) = second {
            self.thumbnail_failures.insert(a, entry);
        }
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
    MoveClip {
        from: ClipAddress,
        to: ClipAddress,
    },
    BrowseRelink(ClipAddress),
    Eject(DeckId),
    SaveProject,
    OpenProject,
    RecoverProject,
    RefreshSessionRecoveries,
    RestoreSessionRecovery(usize),
    RestoreSessionRecoveryAt {
        index: usize,
        monotonic_ns: u64,
    },
    StartNamedTake,
    SetRandomSeed,
    RenameProjectTake(usize),
    RemoveProjectTake(usize),
    AddTimelineMarker,
    ExportProjectTake(usize),
    ArchiveProjectTake(usize),
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
    DisconnectMidiInput(String),
    MidiClearDevice(String),
    ConnectOscInput,
    DisconnectOscInput,
    ConnectOscOutput,
    DisconnectOscOutput,
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
    StartCameraRecording(ClipAddress),
    StopCameraRecording(DeckId),
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
    pub project_takes: &'a [oneiroi_io::TakeMetadataProject],
    pub cameras: &'a [CameraDevice],
    pub camera_status: &'a str,
    pub camera_recordings: [CameraRecordingStatus; 4],
    pub audio_inputs: &'a [AudioInputDevice],
    pub audio_status: &'a str,
    pub audio_connected: bool,
    pub audio_snapshot: AudioInputSnapshot,
    pub midi: MidiMetrics<'a>,
    pub osc: OscMetrics<'a>,
    pub output_displays: &'a [OutputDisplay],
    pub output_health: OutputHealthMetrics<'a>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CameraRecordingStatus {
    pub address: Option<ClipAddress>,
    pub finalizing: bool,
    pub elapsed_seconds: f64,
    pub dropped_frames: u64,
}

pub struct MidiMetrics<'a> {
    pub inputs: &'a [MidiInputDevice],
    pub status: &'a str,
    pub devices: &'a [MidiDeviceStatus],
    pub mapper: &'a mut MidiMapper,
}

impl MidiMetrics<'_> {
    pub fn any_connected(&self) -> bool {
        self.devices.iter().any(|device| device.connected)
    }

    pub fn device_connected(&self, id: &str) -> bool {
        self.devices
            .iter()
            .any(|device| device.connected && device.id == id)
    }
}

/// One MIDI device as the manager window sees it.
#[derive(Clone, Debug, Default)]
pub struct MidiDeviceStatus {
    pub id: String,
    pub label: String,
    /// Present in the latest native discovery snapshot.
    pub available: bool,
    pub connected: bool,
    /// Whether the operator asked for this device; wanted devices reconnect
    /// automatically when the hardware reappears.
    pub wanted: bool,
    pub stats: MidiInputStats,
}

pub struct OscMetrics<'a> {
    pub status: &'a str,
    pub connected: bool,
    pub stats: crate::osc::OscStats,
    pub pending: usize,
    pub schedule_dropped: u64,
    pub output_status: &'a str,
    pub output_connected: bool,
    pub output_stats: crate::osc::OscOutputStats,
}

/// Everything the click-to-arm overlay needs, resolved once per frame.
///
/// Ableton-style mapping: arm the mode, click any highlighted control, then
/// move a knob on any connected device. While the mode is armed the wrapped
/// widgets are disabled so browsing for a control cannot change the mix.
pub(super) struct MidiMapUi {
    pub active: bool,
    pub learning: Option<ControlTarget>,
    /// (target, device) pairs for every existing binding.
    pub mapped: Vec<(ControlTarget, String)>,
    pub palette: ThemePalette,
}

impl MidiMapUi {
    fn devices_for(&self, target: ControlTarget) -> Vec<&str> {
        self.mapped
            .iter()
            .filter(|(mapped, _)| *mapped == target)
            .map(|(_, device)| device.as_str())
            .collect()
    }
}

/// Wrap a widget so it participates in MIDI map mode.
///
/// Outside map mode this is a transparent pass-through. Inside it, the widget
/// draws disabled and an overlay takes the clicks: primary arms the target
/// for the next incoming message, secondary clears its bindings.
pub(super) fn mappable(
    ui: &mut egui::Ui,
    map: &MidiMapUi,
    target: ControlTarget,
    actions: &mut Vec<UiAction>,
    add: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    if !map.active {
        return add(ui);
    }
    let response = ui.add_enabled_ui(false, add).inner;
    let armed = map.learning == Some(target);
    let devices = map.devices_for(target);
    let (tint, stroke) = if armed {
        (
            map.palette.control_tint(map.palette.accent, 0.34),
            egui::Stroke::new(2.0, map.palette.accent),
        )
    } else if devices.is_empty() {
        (
            map.palette.control_tint(map.palette.stroke, 0.10),
            egui::Stroke::new(1.0, map.palette.stroke),
        )
    } else {
        (
            map.palette.control_tint(map.palette.secondary, 0.28),
            egui::Stroke::new(1.0, map.palette.secondary),
        )
    };
    let rect = response.rect.expand(2.0);
    ui.painter()
        .rect(rect, 4.0, tint, stroke, egui::StrokeKind::Outside);
    let hit = ui.interact(
        rect,
        response.id.with("midi-map-overlay"),
        egui::Sense::click(),
    );
    let hit = hit.on_hover_text(if armed {
        "Armed · move a control on any connected device".to_owned()
    } else if devices.is_empty() {
        "Click to arm, then move a control · right-click clears".to_owned()
    } else {
        format!(
            "Mapped to {} · click to remap · right-click clears",
            devices.join(", ")
        )
    });
    if hit.clicked() {
        actions.push(if armed {
            UiAction::MidiCancelLearn
        } else {
            UiAction::MidiLearn(target)
        });
    }
    if hit.secondary_clicked() {
        actions.push(UiAction::MidiClearTarget(target));
    }
    response
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
    state.theme.ensure_applied(ctx);
    let palette = state.theme.palette();
    state.fps.push(metrics.frame_time.delta);
    let mut actions = Vec::new();
    let midi_map = MidiMapUi {
        active: state.midi_map_mode,
        learning: metrics.midi.mapper.learning(),
        mapped: metrics
            .midi
            .mapper
            .bindings
            .iter()
            .map(|binding| (binding.target, binding.device.clone()))
            .collect(),
        palette,
    };

    egui::Window::new("oneiroi")
        .default_pos([16.0, 16.0])
        .default_size([1180.0, 760.0])
        .min_size([560.0, 420.0])
        .resizable(true)
        .scroll([true, true])
        .show(ctx, |ui| {
            draw_toolbar(ui, state, clips, &metrics, palette, &mut actions);
            ui.separator();
            // Everything an operator sets up before the show - output,
            // project, devices, diagnostics - lives in one collapsible
            // region with its own scrollbar, so on a small screen it can
            // be scrolled through or folded away entirely while the
            // performance surface below keeps the space.
            draw_setup(ui, state, mixer, &midi_map, &mut actions, &mut metrics);
            draw_clip_grid(
                ui,
                state,
                mixer,
                clips,
                ClipGridContext {
                    launches,
                    midi_map: &midi_map,
                    cameras: metrics.cameras,
                    camera_status: metrics.camera_status,
                    camera_recordings: metrics.camera_recordings,
                },
                &mut actions,
            );

            ui.separator();
            {
                let layout = state.theme.deck_layout.resolve(ui.available_width());
                let selected_deck = mixer.selected();
                let transforms_ref = &mut state.transforms;
                let blend_modes_ref = &mut state.blend_modes;
                let solo_ref = &mut state.solo;
                let bypassed_ref = &mut state.bypassed;
                let effects_ref = &mut state.effects;
                let lfos_ref = &mut state.lfos;
                let actions_ref = &mut actions;
                let mut deck_strip = |ui: &mut egui::Ui, deck_id: DeckId| {
                    draw_deck(
                        ui,
                        mixer,
                        deck_id,
                        DeckControls {
                            palette,
                            midi_map: &midi_map,
                            show_mode: state.show_mode,
                            transport: &mut transports[deck_id.index()],
                            transform: &mut transforms_ref[deck_id.index()],
                            blend_mode: &mut blend_modes_ref[deck_id.index()],
                            solo: &mut solo_ref[deck_id.index()],
                            bypassed: &mut bypassed_ref[deck_id.index()],
                            effects: &mut effects_ref[deck_id.index()],
                            lfos: &mut lfos_ref[deck_id.index()],
                        },
                        actions_ref,
                    );
                };
                ui.strong(format!(
                    "SELECTED DECK {} · PERFORMANCE CONTROLS & FX",
                    selected_deck.label()
                ));
                if state.master_freeze {
                    ui.colored_label(
                        palette.warning,
                        "PROGRAM FROZEN · Deck FX and source changes are staged until master freeze is released.",
                    );
                }
                deck_strip(ui, selected_deck);

                if !state.show_mode {
                    egui::CollapsingHeader::new("Other deck editors")
                        .default_open(false)
                        .show(ui, |ui| match layout {
                            ResolvedDeckLayout::Cascade => {
                                egui::ScrollArea::horizontal()
                                    .id_salt("other-deck-cascade")
                                    .show(ui, |ui| {
                                        ui.horizontal_top(|ui| {
                                            for deck_id in DeckId::ALL
                                                .into_iter()
                                                .filter(|id| *id != selected_deck)
                                            {
                                                ui.allocate_ui_with_layout(
                                                    egui::vec2(CASCADE_STRIP_WIDTH, 10.0),
                                                    egui::Layout::top_down(egui::Align::Min),
                                                    |ui| {
                                                        ui.set_width(CASCADE_STRIP_WIDTH);
                                                        deck_strip(ui, deck_id);
                                                    },
                                                );
                                            }
                                        });
                                    });
                            }
                            ResolvedDeckLayout::Grid => {
                                egui::Grid::new("other-decks")
                                    .num_columns(2)
                                    .spacing([12.0, 12.0])
                                    .show(ui, |ui| {
                                        for (index, deck_id) in DeckId::ALL
                                            .into_iter()
                                            .filter(|id| *id != selected_deck)
                                            .enumerate()
                                        {
                                            deck_strip(ui, deck_id);
                                            if index % 2 == 1 {
                                                ui.end_row();
                                            }
                                        }
                                    });
                            }
                            ResolvedDeckLayout::Stack => {
                                for deck_id in DeckId::ALL
                                    .into_iter()
                                    .filter(|id| *id != selected_deck)
                                {
                                    deck_strip(ui, deck_id);
                                }
                            }
                        });
                }
            }

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("A");
                mappable(ui, &midi_map, ControlTarget::Crossfader, &mut actions, |ui| {
                    ui.add(
                        egui::Slider::new(&mut state.crossfader, 0.0..=1.0)
                            .text("crossfader")
                            .clamping(egui::SliderClamping::Always),
                    )
                });
                ui.label("B");
                ui.checkbox(&mut state.equal_power, "equal power");
                if ui.button("Center").clicked() {
                    state.crossfader = 0.5;
                }
            });
            ui.horizontal(|ui| {
                mappable(ui, &midi_map, ControlTarget::MasterOpacity, &mut actions, |ui| {
                    ui.add(
                        egui::Slider::new(&mut state.master_opacity, 0.0..=1.0)
                            .text("master")
                            .clamping(egui::SliderClamping::Always),
                    )
                });
                let blackout = mappable(
                    ui,
                    &midi_map,
                    ControlTarget::MasterBlackout,
                    &mut actions,
                    |ui| ui.selectable_label(state.blackout, "BLACKOUT"),
                );
                if blackout.clicked() {
                    state.blackout = !state.blackout;
                }
                mappable(
                    ui,
                    &midi_map,
                    ControlTarget::MasterFreeze,
                    &mut actions,
                    |ui| ui.checkbox(&mut state.master_freeze, "master freeze"),
                );
            });
            if !state.show_mode {
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
                        ui.colored_label(palette.warning, &state.effect_reload_status);
                    } else {
                        ui.weak(&state.effect_reload_status);
                    }
                });
            } else {
                egui::CollapsingHeader::new("LIVE MASTER EFFECTS")
                    .default_open(true)
                    .show(ui, |ui| {
                        for (index, slot) in state.master_effects.slots.iter_mut().enumerate() {
                            let label = if slot.kind == MasterEffectKind::Custom {
                                state
                                    .effect_packages
                                    .iter()
                                    .find(|package| package.id == slot.package_id)
                                    .map_or("Missing custom package", |package| {
                                        package.name.as_str()
                                    })
                            } else {
                                slot.kind.label()
                            };
                            ui.horizontal(|ui| {
                                ui.monospace(format!("{}", index + 1));
                                ui.strong(label);
                                ui.checkbox(&mut slot.bypassed, "Bypass");
                                ui.add(egui::Slider::new(&mut slot.mix, 0.0..=1.0).text("wet"));
                            });
                        }
                        state.master_effects.sanitize();
                        ui.weak(
                            "Effect choice, order and advanced parameters are locked in Show Mode.",
                        );
                    });
            }
        });
    draw_midi_manager(ctx, state, &mut metrics.midi, &palette, &mut actions);
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
        "Bloom",
        "Bloom threshold",
        "Bloom radius",
        "Bloom chroma",
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
