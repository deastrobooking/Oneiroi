//! Milestone 1: wgpu + winit + egui on this machine, proving the stack works
//! before any media code exists.

mod actions;
mod devices;
mod effects;
mod media;
mod osc;
mod output;
mod playback;
mod project;
mod project_io;
mod recovery;
mod runtime;
mod structural;
mod ui;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use oneiroi_core::{
    Clock, ControlTarget, FIXED_DECK_EFFECT_PARAMETER_COUNT, MediaTime, MidiMapper, Quantization,
    TapTempo, TempoClock, effect_parameter_key,
};
use oneiroi_io::{
    AudioInput, AudioInputDevice, AudioInputSnapshot, MidiInputConnection, MidiInputDevice,
    ProjectFile, TakeMetadataProject, autosave_path, discover_audio_inputs, discover_midi_inputs,
    new_project_id,
};
use oneiroi_media::{
    CameraConfig, CameraDevice, ClipAddress, ClipBank, ClipRestorer, CrossfadeBus, DeckDecoder,
    DeckId, DeckState, DeckTransport, DiscontinuityPolicy, FolderScanner, FourDeckMixer,
    FrameScheduler, LaunchQueue, MediaImporter, ThumbnailWorker, VideoFramePayload,
    crossfade_gains, discover_cameras,
};
use oneiroi_render::{
    BuiltInRenderStage, DeckEffects, FourDeckCompositor, Gpu, MasterEffectProcessor, MixerBus,
    MixerParams, PROGRAM_FORMAT, PresentationOptions, ProgramPresenter, ProgramTarget,
};
use oneiroi_session::{CommandOperation, CommandOrigin, ShowTime};
use output::{OutputLifecycle, describe_monitors, monitor_id};
use structural::StructuralSnapshot;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowId};

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let event_loop = EventLoop::new().context("create event loop")?;
    // Poll rather than Wait: the render loop is continuous and paced by vsync
    // on present, not by incoming input events.
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App {
        state: None,
        initial_files: std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .take(4)
            .collect(),
    };
    event_loop.run_app(&mut app).context("event loop")?;
    Ok(())
}

/// Everything that only exists once a window and GPU device are alive.
struct State {
    window: Arc<Window>,
    output: OutputLifecycle,
    gpu: Gpu,
    program: ProgramTarget,
    master_effect_processor: MasterEffectProcessor,
    operator_presenter: ProgramPresenter,
    compositor: FourDeckCompositor,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    clock: Clock,
    ui: ui::UiState,
    gpu_info: String,
    mixer: FourDeckMixer,
    clips: ClipBank,
    launches: LaunchQueue,
    tempo: TempoClock,
    tap_tempo: TapTempo,
    session_started: Instant,
    performance_started: Instant,
    import_slots: [Option<(u64, usize)>; 4],
    importer: MediaImporter,
    folder_scanner: FolderScanner,
    folder_request_id: u64,
    folder_scan_start: ClipAddress,
    folder_pending: HashSet<ClipAddress>,
    relink_pending: HashSet<ClipAddress>,
    relink_active: HashSet<ClipAddress>,
    recording_pending: HashSet<ClipAddress>,
    camera_recordings: [Option<media::ActiveCameraRecording>; 4],
    folder_status: String,
    decoders: [DeckDecoder; 4],
    schedulers: [FrameScheduler<VideoFramePayload>; 4],
    transports: [DeckTransport; 4],
    last_transport_updates: [Instant; 4],
    media_origins: [Option<MediaTime>; 4],
    playback_generations: [u64; 4],
    modifiers: ModifiersState,
    project_path: Option<PathBuf>,
    project_id: String,
    project_takes: Vec<TakeMetadataProject>,
    last_saved_project: Option<ProjectFile>,
    recovery_path: Option<PathBuf>,
    workspace: PathBuf,
    project_status: String,
    last_autosave: Instant,
    project_epoch: u64,
    restorer: ClipRestorer,
    restore_active: [Option<usize>; 4],
    restore_selected: [usize; 4],
    restore_transport: [Option<DeckTransport>; 4],
    midi: MidiMapper,
    midi_inputs: Vec<MidiInputDevice>,
    midi_connections: Vec<MidiInputConnection>,
    midi_device_stats: Vec<ui::MidiDeviceStatus>,
    midi_status: String,
    /// Devices the operator wants connected; reconnected automatically when
    /// they reappear and saved with the project.
    midi_wanted: std::collections::BTreeSet<String>,
    last_midi_refresh: Instant,
    osc_input: Option<osc::OscInput>,
    osc_stats: osc::OscStats,
    osc_status: String,
    osc_pending: Vec<(Instant, osc::OscEvent)>,
    osc_schedule_dropped: u64,
    osc_output: Option<osc::OscOutput>,
    osc_output_stats: osc::OscOutputStats,
    osc_output_status: String,
    thumbnails: ThumbnailWorker,
    thumbnail_request_id: u64,
    thumbnail_requests: HashMap<ClipAddress, (u64, PathBuf)>,
    cameras: Vec<CameraDevice>,
    camera_status: String,
    live_configs: [Option<CameraConfig>; 4],
    audio_inputs: Vec<AudioInputDevice>,
    audio_input: Option<AudioInput>,
    audio_snapshot: AudioInputSnapshot,
    audio_status: String,
    session_recoveries: Vec<recovery::RecoveryEntry>,
    session_recovery_status: String,
    performance_runtime: runtime::PerformanceRuntime,
}

struct App {
    state: Option<State>,
    initial_files: Vec<PathBuf>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // `resumed` can fire again after a suspend on some platforms; the
        // window and device we already have stay valid.
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop) {
            Ok(mut state) => {
                if self.initial_files.len() == 1
                    && self.initial_files[0]
                        .extension()
                        .is_some_and(|extension| extension == "oneiroi")
                {
                    state.open_project(self.initial_files.remove(0), false);
                } else {
                    for path in self.initial_files.drain(..) {
                        state.import_path(path);
                    }
                }
                self.state = Some(state);
            }
            Err(e) => {
                log::error!("startup failed: {e:#}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if state.output.window.id() == id {
            match event {
                WindowEvent::CloseRequested => {
                    state.record_show_operation(
                        CommandOrigin::Operator,
                        Instant::now(),
                        CommandOperation::SetOutputEnabled { enabled: false },
                    );
                    state.ui.output_enabled = false;
                    state.output.window.set_visible(false);
                }
                WindowEvent::Resized(size) => {
                    state
                        .output
                        .surface
                        .resize(&state.gpu.device, size.width, size.height);
                }
                WindowEvent::Moved(_) => state.update_current_output_display(),
                WindowEvent::ModifiersChanged(modifiers) => {
                    state.modifiers = modifiers.state();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Pressed && !event.repeat =>
                {
                    if let PhysicalKey::Code(code) = event.physical_key
                        && !matches!(code, KeyCode::Delete | KeyCode::Backspace)
                    {
                        state.handle_key(code);
                    }
                }
                _ => {}
            }
            return;
        }
        if state.window.id() != id {
            return;
        }

        // egui sees every event first so it can claim clicks and keys that
        // land on the overlay.
        let response = state.egui_state.on_window_event(&state.window, &event);

        match event {
            WindowEvent::CloseRequested => {
                state.autosave_recovery();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => state.gpu.resize(size.width, size.height),
            WindowEvent::DroppedFile(path) => {
                if state.ui.show_mode {
                    state.project_status = format!(
                        "Show Mode locked · ignored dropped file {}",
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("media")
                    );
                } else {
                    state.import_path(path);
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => state.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                if let PhysicalKey::Code(code) = event.physical_key
                    && (!matches!(code, KeyCode::Delete | KeyCode::Backspace) || !response.consumed)
                {
                    state.handle_key(code);
                }
            }
            WindowEvent::RedrawRequested => state.render(),
            _ => {}
        }

        if response.repaint {
            state.window.request_redraw();
        }
    }
}

impl State {
    pub(crate) fn show_time_at(&self, now: Instant) -> ShowTime {
        let session_elapsed = now
            .saturating_duration_since(self.session_started)
            .as_secs_f64();
        let performance_elapsed = now
            .saturating_duration_since(self.performance_started)
            .as_secs_f64();
        ShowTime {
            monotonic_ns: (session_elapsed * 1_000_000_000.0).round() as u64,
            frame_id: self.clock.frame(),
            beat_ticks: (self.tempo.beat_at(performance_elapsed) * 960.0).round() as i64,
            timecode: None,
        }
    }

    pub(crate) fn record_show_operation(
        &mut self,
        origin: CommandOrigin,
        now: Instant,
        operation: CommandOperation,
    ) {
        let at = self.show_time_at(now);
        if let Err(error) = self.performance_runtime.record(origin, at, operation) {
            log::error!("record show command: {error:#}");
        }
    }

    fn handle_key(&mut self, code: KeyCode) {
        let now = Instant::now();
        match code {
            KeyCode::KeyB => self.dispatch_control_update(
                oneiroi_core::ControlUpdate {
                    target: ControlTarget::MasterBlackout,
                    value: f32::from(!self.ui.blackout),
                },
                CommandOrigin::Keyboard,
                now,
            ),
            KeyCode::Space => self.dispatch_control_update(
                oneiroi_core::ControlUpdate {
                    target: ControlTarget::MasterFreeze,
                    value: f32::from(!self.ui.master_freeze),
                },
                CommandOrigin::Keyboard,
                now,
            ),
            KeyCode::ArrowLeft => {
                self.dispatch_control_update(
                    oneiroi_core::ControlUpdate {
                        target: ControlTarget::Crossfader,
                        value: (self.ui.crossfader - 0.05).max(0.0),
                    },
                    CommandOrigin::Keyboard,
                    now,
                );
            }
            KeyCode::ArrowRight => {
                self.dispatch_control_update(
                    oneiroi_core::ControlUpdate {
                        target: ControlTarget::Crossfader,
                        value: (self.ui.crossfader + 0.05).min(1.0),
                    },
                    CommandOrigin::Keyboard,
                    now,
                );
            }
            KeyCode::Home => self.dispatch_control_update(
                oneiroi_core::ControlUpdate {
                    target: ControlTarget::Crossfader,
                    value: 0.5,
                },
                CommandOrigin::Keyboard,
                now,
            ),
            KeyCode::Escape if self.ui.output_fullscreen => {
                self.record_show_operation(
                    CommandOrigin::Keyboard,
                    now,
                    CommandOperation::SetOutputFullscreen { fullscreen: false },
                );
                self.ui.output_fullscreen = false;
                self.output.window.set_fullscreen(None);
            }
            KeyCode::KeyO if !self.modifiers.control_key() && !self.modifiers.super_key() => {
                let enabled = !self.ui.output_enabled;
                self.record_show_operation(
                    CommandOrigin::Keyboard,
                    now,
                    CommandOperation::SetOutputEnabled { enabled },
                );
                self.ui.output_enabled = enabled;
                self.output.window.set_visible(enabled);
            }
            KeyCode::KeyS if self.modifiers.control_key() || self.modifiers.super_key() => {
                self.save_project_from_ui();
            }
            KeyCode::KeyO if self.modifiers.control_key() || self.modifiers.super_key() => {
                self.open_project_from_ui();
            }
            KeyCode::Delete | KeyCode::Backspace if !self.ui.show_mode => {
                let deck = self.mixer.selected();
                let address = ClipAddress {
                    deck,
                    slot: self.clips.selected(deck),
                };
                self.clear_clip(address, now, CommandOrigin::Keyboard);
            }
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8 => {
                let slot = match code {
                    KeyCode::Digit1 => 0,
                    KeyCode::Digit2 => 1,
                    KeyCode::Digit3 => 2,
                    KeyCode::Digit4 => 3,
                    KeyCode::Digit5 => 4,
                    KeyCode::Digit6 => 5,
                    KeyCode::Digit7 => 6,
                    KeyCode::Digit8 => 7,
                    _ => unreachable!(),
                };
                self.dispatch_control_update(
                    oneiroi_core::ControlUpdate {
                        target: ControlTarget::SceneLaunch(slot as u8),
                        value: 1.0,
                    },
                    CommandOrigin::Keyboard,
                    now,
                );
            }
            _ => return,
        }
        self.window.request_redraw();
    }

    fn new(event_loop: &ActiveEventLoop) -> Result<Self> {
        let primary_monitor = event_loop.primary_monitor();
        let monitor_handles: Vec<_> = event_loop.available_monitors().collect();
        let preferred_monitor = monitor_handles
            .iter()
            .find(|monitor| primary_monitor.as_ref() != Some(*monitor))
            .or(primary_monitor.as_ref())
            .or(monitor_handles.first());
        let preferred_display_id = preferred_monitor.map(monitor_id);
        let preferred_position = preferred_monitor.map(MonitorHandle::position);
        let (output_monitors, output_displays) = describe_monitors(monitor_handles);
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut ui = ui::UiState::default();
        let effect_roots = effects::effect_resource_roots(&workspace);
        ui.effect_manifest_path = effects::bundled_processor_manifest(&effect_roots)
            .or_else(|| {
                effect_roots
                    .first()
                    .map(|root| root.join("master-effects/effect.json"))
            })
            .unwrap_or_else(|| PathBuf::from("effects/master-effects/effect.json"))
            .to_string_lossy()
            .into_owned();
        let effect_registry = effects::discover_effect_registry(&effect_roots);
        ui.effect_registry_status =
            effects::effect_registry_status(&effect_registry, &effect_roots);
        ui.effect_packages = effect_registry.effects;
        if let Some(id) = preferred_display_id {
            ui.output_display_id = id;
        }
        let output_current_display = output_displays
            .iter()
            .find(|display| display.id == ui.output_display_id)
            .map(|display| display.label.clone())
            .unwrap_or_else(|| "No connected display".to_owned());
        let attrs = Window::default_attributes()
            .with_title("oneiroi")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .context("create main window")?,
        );
        let mut output_attrs = Window::default_attributes()
            .with_title("oneiroi · PROGRAM")
            .with_inner_size(winit::dpi::LogicalSize::new(960.0, 540.0));
        if let Some(position) = preferred_position {
            output_attrs = output_attrs.with_position(PhysicalPosition::new(
                position.x.saturating_add(40),
                position.y.saturating_add(40),
            ));
        }
        let output_window = Arc::new(
            event_loop
                .create_window(output_attrs)
                .context("create program output window")?,
        );

        let size = window.inner_size();
        // Blocking here is fine: it happens once, before the loop is running.
        let gpu = pollster::block_on(Gpu::new(window.clone(), size.width, size.height))?;
        let output_size = output_window.inner_size();
        let output_surface =
            gpu.create_surface(output_window.clone(), output_size.width, output_size.height)?;
        let program = ProgramTarget::new(&gpu.device, [1920, 1080]);
        let mut master_effect_processor = MasterEffectProcessor::new(&gpu.device, &program);
        let mut effect_manifest_paths = vec![PathBuf::from(&ui.effect_manifest_path)];
        effect_manifest_paths.extend(
            ui.effect_packages
                .iter()
                .map(|effect| effect.manifest_path.clone()),
        );
        master_effect_processor.watch_effect_manifests(effect_manifest_paths);
        ui.effect_reload_status = master_effect_processor.reload_status().to_owned();
        let operator_presenter = ProgramPresenter::new(&gpu.device, &program, gpu.content_format());
        let output_presenter =
            ProgramPresenter::new(&gpu.device, &program, output_surface.content_format());

        let info = gpu.adapter_info();
        let bc_support = if gpu.supports_bc_textures() {
            "BC textures"
        } else {
            "no BC textures"
        };
        let project_id = new_project_id();
        let mut performance_runtime = runtime::PerformanceRuntime::new(ui.composition_extent)?;
        if let Err(error) =
            performance_runtime.enable_journal(&workspace.join(".oneiroi/session"), &project_id)
        {
            log::error!("session journal disabled: {error:#}");
        }
        let gpu_info = format!("{} · {:?} · {bc_support}", info.name, info.backend);

        let compositor = FourDeckCompositor::new(&gpu.device, &gpu.queue, PROGRAM_FORMAT);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx,
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(
            &gpu.device,
            gpu.surface_format(),
            egui_wgpu::RendererOptions::default(),
        );

        window.request_redraw();
        let untitled_recovery = autosave_path(None, &workspace);
        let recovery_path = untitled_recovery.exists().then_some(untitled_recovery);
        let (cameras, camera_status) = match discover_cameras() {
            Ok(cameras) if cameras.is_empty() => (
                cameras,
                "No cameras discovered; enter an AVFoundation device ID.".to_owned(),
            ),
            Ok(cameras) => {
                let count = cameras.len();
                (cameras, format!("{count} camera(s) available"))
            }
            Err(error) => (Vec::new(), format!("Camera discovery: {error}")),
        };
        let (audio_inputs, audio_status) = match discover_audio_inputs() {
            Ok(inputs) if inputs.is_empty() => {
                (inputs, "No audio input devices discovered".to_owned())
            }
            Ok(inputs) => {
                let count = inputs.len();
                (inputs, format!("{count} audio input(s) available"))
            }
            Err(error) => (Vec::new(), format!("Audio discovery: {error}")),
        };
        if ui.audio_device_id.is_empty()
            && let Some(device) = audio_inputs
                .iter()
                .find(|device| device.is_default)
                .or_else(|| audio_inputs.first())
        {
            ui.audio_device_id = device.id.clone();
        }
        let (midi_inputs, midi_status) = match discover_midi_inputs() {
            Ok(inputs) if inputs.is_empty() => {
                (inputs, "No MIDI input devices discovered".to_owned())
            }
            Ok(inputs) => {
                let count = inputs.len();
                (inputs, format!("{count} MIDI input(s) available"))
            }
            Err(error) => (Vec::new(), format!("MIDI discovery: {error}")),
        };
        if let Some(device) = midi_inputs.first() {
            ui.midi_device_id = device.id.clone();
        }

        let started = Instant::now();
        let output = OutputLifecycle::new(
            output_window,
            output_monitors,
            output_displays,
            output_current_display,
            output_surface,
            output_presenter,
        );

        Ok(Self {
            window,
            output,
            gpu,
            program,
            master_effect_processor,
            operator_presenter,
            compositor,
            egui_state,
            egui_renderer,
            clock: Clock::new(Instant::now()),
            ui,
            gpu_info,
            mixer: FourDeckMixer::default(),
            clips: ClipBank::default(),
            launches: LaunchQueue::default(),
            tempo: TempoClock::default(),
            tap_tempo: TapTempo::default(),
            session_started: started,
            performance_started: started,
            import_slots: [None; 4],
            importer: MediaImporter::new(8),
            folder_scanner: FolderScanner::new(),
            folder_request_id: 0,
            folder_scan_start: ClipAddress {
                deck: DeckId::A,
                slot: 0,
            },
            folder_pending: HashSet::new(),
            relink_pending: HashSet::new(),
            relink_active: HashSet::new(),
            recording_pending: HashSet::new(),
            camera_recordings: std::array::from_fn(|_| None),
            folder_status: String::new(),
            decoders: std::array::from_fn(|_| DeckDecoder::spawn(4)),
            schedulers: std::array::from_fn(|_| {
                FrameScheduler::new(4, 0, DiscontinuityPolicy::Blank).expect("non-zero frame queue")
            }),
            transports: [DeckTransport::default(); 4],
            last_transport_updates: [Instant::now(); 4],
            media_origins: [None; 4],
            playback_generations: [0; 4],
            modifiers: ModifiersState::empty(),
            project_path: None,
            project_id,
            project_takes: Vec::new(),
            last_saved_project: None,
            recovery_path,
            workspace,
            project_status: String::new(),
            last_autosave: Instant::now(),
            project_epoch: 0,
            restorer: ClipRestorer::new(32),
            restore_active: [None; 4],
            restore_selected: [0; 4],
            restore_transport: [None; 4],
            midi: MidiMapper::default(),
            midi_inputs,
            midi_connections: Vec::new(),
            midi_device_stats: Vec::new(),
            midi_status,
            midi_wanted: std::collections::BTreeSet::new(),
            last_midi_refresh: Instant::now(),
            osc_input: None,
            osc_stats: osc::OscStats::default(),
            osc_status: "OSC input disconnected".to_owned(),
            osc_pending: Vec::new(),
            osc_schedule_dropped: 0,
            osc_output: None,
            osc_output_stats: osc::OscOutputStats::default(),
            osc_output_status: "OSC feedback disconnected".to_owned(),
            thumbnails: ThumbnailWorker::new(32),
            thumbnail_request_id: 0,
            thumbnail_requests: HashMap::new(),
            cameras,
            camera_status,
            live_configs: std::array::from_fn(|_| None),
            audio_inputs,
            audio_input: None,
            audio_snapshot: AudioInputSnapshot::default(),
            audio_status,
            session_recoveries: Vec::new(),
            session_recovery_status: "Session recovery catalog not scanned".to_owned(),
            performance_runtime,
        })
    }

    fn render(&mut self) {
        if self.master_effect_processor.poll_effect_reload() {
            self.ui.effect_reload_status = self.master_effect_processor.reload_status().to_owned();
        }
        self.poll_imports();
        self.poll_folder_scans();
        self.poll_camera_recordings();
        self.poll_restores();
        self.poll_thumbnails();
        let now = Instant::now();
        self.poll_midi(now);
        self.poll_osc(now);
        if now.saturating_duration_since(self.output.last_display_refresh) >= Duration::from_secs(2)
        {
            self.refresh_output_displays();
        }
        self.maybe_autosave(now);
        self.process_launches(now);
        self.update_playback(now);
        if let Some(input) = &self.audio_input {
            input.set_settings(self.ui.audio_analysis);
            self.audio_snapshot = input.snapshot();
            if self.audio_snapshot.callback_errors > 0 {
                self.audio_status = format!(
                    "Audio callback errors: {}",
                    self.audio_snapshot.callback_errors
                );
            }
        }
        let time = self.clock.tick(now);
        let show_time = ShowTime {
            frame_id: time.frame,
            ..self.show_time_at(now)
        };
        if let Err(error) = self.performance_runtime.tick(show_time) {
            log::error!("performance runtime: {error:#}");
        }
        let runtime_status = self.performance_runtime.status();
        let project_dirty = self.project_dirty();

        // --- UI pass: pure CPU, produces geometry for the GPU pass below.
        let ctx = self.egui_state.egui_ctx().clone();
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let controls_before = performance_control_snapshot(&self.ui, &self.mixer, &self.transports);
        let structure_before = StructuralSnapshot::capture(&self.ui, &self.mixer);
        let bpm_before = self.ui.bpm;
        let quantization_before = self.ui.quantization;
        let mut actions = Vec::new();
        let output = ctx.run_ui(raw_input, |ui| {
            actions = ui::draw(
                ui.ctx(),
                &mut self.ui,
                &mut self.mixer,
                &mut self.clips,
                &self.launches,
                &mut self.transports,
                ui::PerformanceMetrics {
                    tempo: self.tempo,
                    now_seconds: now
                        .saturating_duration_since(self.performance_started)
                        .as_secs_f64(),
                    scheduler_stats: std::array::from_fn(|index| self.schedulers[index].stats()),
                    frame_pool_stats: std::array::from_fn(|index| {
                        self.decoders[index].frame_pool_stats()
                    }),
                    frame_time: &time,
                    gpu_info: &self.gpu_info,
                    runtime_status: &runtime_status,
                    project_dirty,
                    project_status: &self.project_status,
                    folder_status: &self.folder_status,
                    recovery_available: self.recovery_path.is_some(),
                    session_recoveries: &self.session_recoveries,
                    session_recovery_status: &self.session_recovery_status,
                    project_takes: &self.project_takes,
                    cameras: &self.cameras,
                    camera_status: &self.camera_status,
                    camera_recordings: std::array::from_fn(|index| {
                        self.camera_recordings[index].as_ref().map_or(
                            ui::CameraRecordingStatus::default(),
                            |recording| ui::CameraRecordingStatus {
                                address: Some(recording.address),
                                finalizing: recording.finalizing,
                                elapsed_seconds: now
                                    .saturating_duration_since(recording.started)
                                    .as_secs_f64(),
                                dropped_frames: recording.recorder.dropped_frames(),
                            },
                        )
                    }),
                    audio_inputs: &self.audio_inputs,
                    audio_status: &self.audio_status,
                    audio_connected: self.audio_input.is_some(),
                    audio_snapshot: self.audio_snapshot,
                    midi: ui::MidiMetrics {
                        inputs: &self.midi_inputs,
                        status: &self.midi_status,
                        devices: &self.midi_device_stats,
                        mapper: &mut self.midi,
                    },
                    osc: ui::OscMetrics {
                        status: &self.osc_status,
                        connected: self.osc_input.is_some(),
                        stats: self.osc_stats,
                        pending: self.osc_pending.len(),
                        schedule_dropped: self.osc_schedule_dropped,
                        output_status: &self.osc_output_status,
                        output_connected: self.osc_output.is_some(),
                        output_stats: self.osc_output_stats,
                    },
                    output_displays: &self.output.displays,
                    output_health: ui::OutputHealthMetrics {
                        status: self.output.health.status,
                        current_display: &self.output.current_display,
                        surface_extent: {
                            let (width, height) = self.output.surface.size();
                            [width, height]
                        },
                        presented: self.output.health.presented,
                        skipped: self.output.health.skipped,
                        reconfigurations: self.output.health.reconfigurations,
                        recoveries: self.output.health.recoveries,
                        timeouts: self.output.health.timeouts,
                        occlusions: self.output.health.occlusions,
                        validation_errors: self.output.health.validation_errors,
                        topology_changes: self.output.health.topology_changes,
                    },
                },
            );
        });
        let structure_after = StructuralSnapshot::capture(&self.ui, &self.mixer);
        structure_before.apply(&mut self.ui, &mut self.mixer);
        for command in structure_before.commands_to(&structure_after) {
            self.record_show_operation(CommandOrigin::Operator, now, command);
        }
        structure_after.apply(&mut self.ui, &mut self.mixer);
        let controls_after = performance_control_snapshot(&self.ui, &self.mixer, &self.transports);
        for (target, value) in controls_after {
            let Some(previous) = controls_before.get(&target).copied() else {
                continue;
            };
            if value.to_bits() == previous.to_bits() {
                continue;
            }
            // Deck selection is represented by four one-hot trigger controls.
            // Restoring the old selected deck before dispatching each changed
            // target makes the final result depend on BTreeMap iteration order:
            // switching D -> B would select B, then restore D while processing
            // D's falling edge. Trigger updates have no continuous value to
            // roll back, so only dispatch their changed edge.
            let Some(rollback) = rollback_update_for_ui_change(target, previous) else {
                self.dispatch_control_update(
                    oneiroi_core::ControlUpdate { target, value },
                    CommandOrigin::Operator,
                    now,
                );
                continue;
            };
            self.apply_control_update_unrecorded(rollback, now);
            self.dispatch_control_update(
                oneiroi_core::ControlUpdate { target, value },
                CommandOrigin::Operator,
                now,
            );
        }
        if self.ui.bpm.to_bits() != bpm_before.to_bits() {
            let bpm = self.ui.bpm.clamp(20.0, 400.0);
            self.ui.bpm = bpm_before;
            self.record_show_operation(
                CommandOrigin::Operator,
                now,
                CommandOperation::SetTempo { bpm },
            );
            self.ui.bpm = bpm;
            self.tempo.set_bpm(
                bpm,
                now.saturating_duration_since(self.performance_started)
                    .as_secs_f64(),
            );
            self.publish_osc_value("/vjx/tempo", bpm as f32);
        }
        if self.ui.quantization != quantization_before {
            self.record_show_operation(
                CommandOrigin::Operator,
                now,
                CommandOperation::SetParameter {
                    path: "launch.quantization".to_owned(),
                    value: oneiroi_graph::ParameterValue::Text(
                        match self.ui.quantization {
                            Quantization::Immediate => "immediate",
                            Quantization::Beat => "beat",
                            Quantization::Bar => "bar",
                        }
                        .to_owned(),
                    ),
                },
            );
        }
        self.dispatch_ui_actions(actions, now);
        self.egui_state
            .handle_platform_output(&self.window, output.platform_output);
        let pixels_per_point = ctx.pixels_per_point();
        let paint_jobs = ctx.tessellate(output.shapes, pixels_per_point);

        // Texture deltas are applied before the surface is acquired, and
        // therefore before anything can make us bail out of this frame. egui
        // hands each delta over exactly once; dropping one on a skipped frame
        // loses the allocation permanently and the next partial update panics.
        for (id, delta) in &output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.gpu.device, &self.gpu.queue, *id, delta);
        }
        for id in &output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }

        // --- GPU pass. Compose once offscreen, then present the same program
        // texture to the operator preview and clean output surfaces.
        let effect_time = now
            .saturating_duration_since(self.performance_started)
            .as_secs_f32();
        let beat_position = self.tempo.beat_at(f64::from(effect_time)) as f32;
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        let master_effects_active = !self.ui.blackout && self.ui.master_effects.active();
        if !master_effects_active {
            self.master_effect_processor.reset_history();
        }
        let freeze_program = self.ui.master_freeze && !self.ui.blackout;
        let render_plan = self.performance_runtime.render_plan().clone();
        if !freeze_program {
            for stage in render_plan.stages() {
                match stage {
                    BuiltInRenderStage::FourDeckComposite { .. } => {
                        let composition_target = if master_effects_active {
                            self.program.composition_view()
                        } else {
                            &self.program.view
                        };
                        let audio = self.audio_snapshot.analysis;
                        self.compositor.draw(
                            &self.gpu.device,
                            &self.gpu.queue,
                            &mut encoder,
                            composition_target,
                            MixerParams {
                                levels: std::array::from_fn(|index| {
                                    let deck = self.mixer.deck(DeckId::ALL[index]);
                                    if matches!(
                                        deck.state,
                                        DeckState::Ready(_) | DeckState::Live(_)
                                    ) {
                                        deck.level
                                    } else {
                                        0.0
                                    }
                                }),
                                solo: self.ui.solo,
                                bypassed: self.ui.bypassed,
                                buses: std::array::from_fn(|index| {
                                    match self.mixer.deck(DeckId::ALL[index]).bus {
                                        CrossfadeBus::Left => MixerBus::A,
                                        CrossfadeBus::Right => MixerBus::B,
                                    }
                                }),
                                crossfade_gains: crossfade_gains(
                                    self.ui.crossfader,
                                    self.ui.equal_power,
                                ),
                                transforms: self.ui.transforms,
                                blend_modes: self.ui.blend_modes,
                                output_aspect: self.ui.composition_extent[0] as f32
                                    / self.ui.composition_extent[1].max(1) as f32,
                                effects: std::array::from_fn(|index| {
                                    self.ui.lfos[index].apply_with_audio(
                                        self.ui.effects[index],
                                        effect_time,
                                        beat_position,
                                        [
                                            audio.rms,
                                            audio.bass,
                                            audio.mid,
                                            audio.high,
                                            audio.transient,
                                        ],
                                    )
                                }),
                                master_opacity: self.ui.master_opacity,
                                time_seconds: effect_time,
                                blackout: self.ui.blackout,
                            },
                        );
                    }
                    BuiltInRenderStage::MasterEffects { .. } if master_effects_active => {
                        let audio = self.audio_snapshot.analysis;
                        self.master_effect_processor.draw_modulated_at(
                            &self.gpu.queue,
                            &mut encoder,
                            &self.program,
                            &self.ui.master_effects,
                            &self.ui.master_modulation,
                            effect_time,
                            beat_position,
                            [
                                audio.rms,
                                audio.bass,
                                audio.mid,
                                audio.high,
                                audio.transient,
                            ],
                        );
                    }
                    BuiltInRenderStage::MasterEffects { .. }
                    | BuiltInRenderStage::ProgramOutput { .. } => {}
                }
            }
        }

        let (width, height) = self.gpu.size();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point,
        };
        let upload_cmds = self.egui_renderer.update_buffers(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &paint_jobs,
            &screen,
        );

        let operator_frame = self.gpu.acquire();
        let presentation = PresentationOptions {
            test_card: self.ui.output_test_card,
            identify: self.ui.output_identify,
        };
        if render_plan.has_program_output()
            && let Some(frame) = operator_frame.as_ref()
        {
            let content_view = self.gpu.content_view(&frame.texture);
            let ui_view = self.gpu.surface_view(&frame.texture);
            let (width, height) = self.gpu.size();
            self.operator_presenter.draw(
                &self.gpu.queue,
                &mut encoder,
                &content_view,
                [width, height],
                presentation,
            );
            let pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &ui_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            let mut pass = pass;
            self.egui_renderer.render(&mut pass, &paint_jobs, &screen);
        }

        let output_frame = if self.ui.output_enabled {
            let acquisition = self.output.surface.acquire_with_status(&self.gpu.device);
            self.output.health.observe(acquisition.status);
            acquisition.frame
        } else {
            None
        };
        if render_plan.has_program_output()
            && let Some(frame) = output_frame.as_ref()
        {
            let view = self.output.surface.content_view(&frame.texture);
            let (width, height) = self.output.surface.size();
            self.output.presenter.draw(
                &self.gpu.queue,
                &mut encoder,
                &view,
                [width, height],
                presentation,
            );
        }

        self.gpu
            .queue
            .submit(upload_cmds.into_iter().chain([encoder.finish()]));
        if let Some(frame) = operator_frame {
            frame.present();
        }
        if let Some(frame) = output_frame {
            frame.present();
        }

        // Continuous redraw. Presentation is Fifo, so this paces to vsync
        // rather than spinning.
        self.window.request_redraw();
    }
}

fn deck_id(index: u8) -> Option<DeckId> {
    DeckId::ALL.get(usize::from(index)).copied()
}

fn media_time_from_seconds(seconds: f64) -> Option<MediaTime> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    let micros = (seconds * 1_000_000.0).round();
    if !(0.0..=i64::MAX as f64).contains(&micros) {
        return None;
    }
    MediaTime::new(micros as i64, 1_000_000).ok()
}

fn current_control_value(
    ui: &ui::UiState,
    mixer: &FourDeckMixer,
    transports: &[DeckTransport; 4],
    target: ControlTarget,
) -> f32 {
    match target {
        ControlTarget::Crossfader => ui.crossfader,
        ControlTarget::MasterOpacity => ui.master_opacity,
        ControlTarget::MasterBlackout => f32::from(ui.blackout),
        ControlTarget::MasterFreeze => f32::from(ui.master_freeze),
        ControlTarget::TapTempo => 0.0,
        ControlTarget::DeckLevel(deck) => deck_id(deck)
            .map(|deck| mixer.deck(deck).level)
            .unwrap_or_default(),
        ControlTarget::DeckPlay(deck) => deck_id(deck)
            .map(|deck| f32::from(transports[deck.index()].playing))
            .unwrap_or_default(),
        ControlTarget::DeckFreeze(deck) => deck_id(deck)
            .map(|deck| f32::from(transports[deck.index()].frozen))
            .unwrap_or_default(),
        ControlTarget::DeckSpeed(deck) => deck_id(deck)
            .map(|deck| transports[deck.index()].speed)
            .unwrap_or(1.0),
        ControlTarget::DeckSelect(deck) => deck_id(deck)
            .map(|deck| f32::from(mixer.selected() == deck))
            .unwrap_or_default(),
        ControlTarget::DeckRestart(_)
        | ControlTarget::ClipLaunch { .. }
        | ControlTarget::SceneLaunch(_) => 0.0,
        ControlTarget::EffectParameter {
            deck,
            effect,
            parameter: _,
        } => deck_id(deck)
            .map(|deck| effect_parameter(ui.effects[deck.index()], effect))
            .unwrap_or_default(),
        ControlTarget::LfoParameter {
            deck,
            lfo,
            parameter,
        } => deck_id(deck)
            .and_then(|deck| ui.lfos[deck.index()].lanes.get(usize::from(lfo)))
            .map(|lfo| match parameter {
                0 => f32::from(lfo.enabled),
                1 => lfo.rate_hz,
                2 => lfo.depth,
                3 => lfo.phase,
                _ => 0.0,
            })
            .unwrap_or_default(),
        ControlTarget::ModRouteParameter {
            deck,
            route,
            parameter,
        } => deck_id(deck)
            .and_then(|deck| ui.lfos[deck.index()].routes.get(usize::from(route)))
            .map(|route| match parameter {
                0 => f32::from(route.enabled),
                1 => route.amount,
                _ => 0.0,
            })
            .unwrap_or_default(),
        ControlTarget::MasterEffectParameter {
            slot,
            parameter_key,
        } => ui
            .master_effects
            .slots
            .get(usize::from(slot))
            .and_then(|effect| {
                effect.parameters.iter().find(|parameter| {
                    effect_parameter_key(&effect.package_id, &parameter.id) == parameter_key
                })
            })
            .map_or(0.0, |parameter| parameter.value),
    }
}

fn performance_control_snapshot(
    ui: &ui::UiState,
    mixer: &FourDeckMixer,
    transports: &[DeckTransport; 4],
) -> BTreeMap<ControlTarget, f32> {
    let mut targets = vec![
        ControlTarget::Crossfader,
        ControlTarget::MasterOpacity,
        ControlTarget::MasterBlackout,
        ControlTarget::MasterFreeze,
    ];
    for deck in 0..4_u8 {
        targets.extend([
            ControlTarget::DeckLevel(deck),
            ControlTarget::DeckPlay(deck),
            ControlTarget::DeckFreeze(deck),
            ControlTarget::DeckSpeed(deck),
            ControlTarget::DeckSelect(deck),
        ]);
        for effect in 0..FIXED_DECK_EFFECT_PARAMETER_COUNT {
            targets.push(ControlTarget::EffectParameter {
                deck,
                effect,
                parameter: 0,
            });
        }
        for lfo in 0..3_u8 {
            for parameter in 0..4_u8 {
                targets.push(ControlTarget::LfoParameter {
                    deck,
                    lfo,
                    parameter,
                });
            }
        }
        for route in 0..8_u8 {
            for parameter in 0..2_u8 {
                targets.push(ControlTarget::ModRouteParameter {
                    deck,
                    route,
                    parameter,
                });
            }
        }
    }
    for (slot, effect) in ui.master_effects.slots.iter().enumerate() {
        for parameter in &effect.parameters {
            targets.push(ControlTarget::MasterEffectParameter {
                slot: slot as u8,
                parameter_key: effect_parameter_key(&effect.package_id, &parameter.id),
            });
        }
    }
    targets
        .into_iter()
        .map(|target| (target, current_control_value(ui, mixer, transports, target)))
        .collect()
}

fn rollback_update_for_ui_change(
    target: ControlTarget,
    previous: f32,
) -> Option<oneiroi_core::ControlUpdate> {
    (!matches!(target, ControlTarget::DeckSelect(_))).then_some(oneiroi_core::ControlUpdate {
        target,
        value: previous,
    })
}

fn effect_parameter(effects: DeckEffects, effect: u8) -> f32 {
    match effect {
        0 => effects.hue,
        1 => effects.contrast,
        2 => effects.saturation,
        3 => effects.black_level,
        4 => effects.white_level,
        5 => effects.gamma,
        6 => effects.pixelate,
        7 => effects.luma_key,
        8 => effects.neon,
        9 => effects.fractal,
        10 => effects.jitter,
        11 => effects.find_edges,
        12 => effects.bit_reduction,
        13 => effects.blacklight,
        14 => effects.bloom,
        15 => effects.bloom_threshold,
        16 => effects.bloom_radius,
        17 => effects.bloom_chroma,
        _ => 0.0,
    }
}

fn set_effect_parameter(effects: &mut DeckEffects, effect: u8, value: f32) {
    match effect {
        0 => effects.hue = value,
        1 => effects.contrast = value,
        2 => effects.saturation = value,
        3 => effects.black_level = value,
        4 => effects.white_level = value,
        5 => effects.gamma = value,
        6 => effects.pixelate = value,
        7 => effects.luma_key = value,
        8 => effects.neon = value,
        9 => effects.fractal = value,
        10 => effects.jitter = value,
        11 => effects.find_edges = value,
        12 => effects.bit_reduction = value,
        13 => effects.blacklight = value,
        14 => effects.bloom = value,
        15 => effects.bloom_threshold = value,
        16 => effects.bloom_radius = value,
        17 => effects.bloom_chroma = value,
        _ => {}
    }
    *effects = effects.sanitized();
}

fn display_path(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| path.display().to_string(), ToOwned::to_owned)
}

fn resolve_project_paths(project: &mut ProjectFile, base: &std::path::Path) {
    for deck in &mut project.decks {
        for path in deck.clips.iter_mut().flatten() {
            if path.is_relative() {
                *path = base.join(&*path);
            }
        }
    }
}

#[cfg(test)]
mod output_health_tests {
    use super::*;

    #[test]
    fn performance_snapshot_covers_mixer_deck_effect_lfo_and_matrix_controls() {
        let ui = ui::UiState::default();
        let mixer = FourDeckMixer::default();
        let transports = [DeckTransport::default(); 4];

        let snapshot = performance_control_snapshot(&ui, &mixer, &transports);

        assert_eq!(snapshot.len(), 208);
        assert!(snapshot.contains_key(&ControlTarget::Crossfader));
        assert!(snapshot.contains_key(&ControlTarget::EffectParameter {
            deck: 3,
            effect: 17,
            parameter: 0,
        }));
        assert!(snapshot.contains_key(&ControlTarget::ModRouteParameter {
            deck: 3,
            route: 7,
            parameter: 1,
        }));
    }

    #[test]
    fn deck_selection_trigger_is_not_rolled_back_during_ui_capture() {
        assert!(rollback_update_for_ui_change(ControlTarget::DeckSelect(1), 0.0).is_none());
        assert!(rollback_update_for_ui_change(ControlTarget::DeckSelect(3), 1.0).is_none());

        let rollback = rollback_update_for_ui_change(ControlTarget::DeckLevel(1), 0.75)
            .expect("continuous deck controls still need rollback before journaling");
        assert_eq!(rollback.target, ControlTarget::DeckLevel(1));
        assert_eq!(rollback.value, 0.75);
    }
}
