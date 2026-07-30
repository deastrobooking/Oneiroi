//! Milestone 1: wgpu + winit + egui on this machine, proving the stack works
//! before any media code exists.

mod project;
mod ui;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use oneiroi_core::{
    Clock, ControlTarget, ControlUpdate, MediaTime, MidiMapper, TapTempo, TempoClock,
};
use oneiroi_io::{
    AudioInput, AudioInputDevice, AudioInputSnapshot, MidiInputConnection, MidiInputDevice,
    MidiInputStats, ProjectFile, autosave_path, discover_audio_inputs, discover_midi_inputs,
    load_project, recovery_is_newer, save_project_atomic,
};
use oneiroi_media::{
    CLIPS_PER_DECK, CameraConfig, CameraDevice, ClipAddress, ClipBank, ClipRestoreRequest,
    ClipRestorer, CrossfadeBus, DeckDecoder, DeckId, DeckState, DeckTransport, DecoderEvent,
    DiscontinuityPolicy, FolderScanRequest, FolderScanner, FourDeckMixer, FrameScheduler,
    FrameSelection, LaunchQueue, MediaImporter, SubmitError, ThumbnailRequest, ThumbnailWorker,
    TransportEvent, VideoFramePayload, crossfade_gains, discover_cameras,
};
use oneiroi_render::{
    DeckEffects, FourDeckCompositor, Gpu, MasterEffectProcessor, MixerBus, MixerParams,
    PROGRAM_FORMAT, PresentSurface, PresentationOptions, ProgramPresenter, ProgramTarget,
    SurfaceAcquireStatus, discover_effect_packages,
};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowId};

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
    output_window: Arc<Window>,
    output_monitors: Vec<OutputMonitor>,
    output_displays: Vec<ui::OutputDisplay>,
    output_current_display: String,
    output_health: OutputHealth,
    last_display_refresh: Instant,
    gpu: Gpu,
    output_surface: PresentSurface,
    program: ProgramTarget,
    master_effect_processor: MasterEffectProcessor,
    operator_presenter: ProgramPresenter,
    output_presenter: ProgramPresenter,
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
    performance_started: Instant,
    import_slots: [Option<(u64, usize)>; 4],
    importer: MediaImporter,
    folder_scanner: FolderScanner,
    folder_request_id: u64,
    folder_scan_start: ClipAddress,
    folder_pending: HashSet<ClipAddress>,
    relink_pending: HashSet<ClipAddress>,
    relink_active: HashSet<ClipAddress>,
    folder_status: String,
    decoders: [DeckDecoder; 4],
    schedulers: [FrameScheduler<VideoFramePayload>; 4],
    transports: [DeckTransport; 4],
    last_transport_updates: [Instant; 4],
    media_origins: [Option<MediaTime>; 4],
    playback_generations: [u64; 4],
    modifiers: ModifiersState,
    project_path: Option<PathBuf>,
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
    midi_input: Option<MidiInputConnection>,
    midi_stats: MidiInputStats,
    midi_status: String,
    midi_reconnect: bool,
    last_midi_refresh: Instant,
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
}

struct OutputMonitor {
    id: String,
    handle: MonitorHandle,
}

struct OutputHealth {
    status: &'static str,
    presented: u64,
    skipped: u64,
    reconfigurations: u64,
    recoveries: u64,
    timeouts: u64,
    occlusions: u64,
    validation_errors: u64,
    topology_changes: u64,
    awaiting_recovery: bool,
}

impl Default for OutputHealth {
    fn default() -> Self {
        Self {
            status: "Waiting for first frame",
            presented: 0,
            skipped: 0,
            reconfigurations: 0,
            recoveries: 0,
            timeouts: 0,
            occlusions: 0,
            validation_errors: 0,
            topology_changes: 0,
            awaiting_recovery: false,
        }
    }
}

impl OutputHealth {
    fn observe(&mut self, status: SurfaceAcquireStatus) {
        match status {
            SurfaceAcquireStatus::Healthy => {
                self.presented = self.presented.saturating_add(1);
                if self.awaiting_recovery {
                    self.recoveries = self.recoveries.saturating_add(1);
                    self.awaiting_recovery = false;
                }
                self.status = "Healthy";
            }
            SurfaceAcquireStatus::Suboptimal => {
                self.presented = self.presented.saturating_add(1);
                self.reconfigurations = self.reconfigurations.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Suboptimal · reconfigured";
            }
            SurfaceAcquireStatus::Outdated => {
                self.skipped = self.skipped.saturating_add(1);
                self.reconfigurations = self.reconfigurations.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Outdated · reconfiguring";
            }
            SurfaceAcquireStatus::Lost => {
                self.skipped = self.skipped.saturating_add(1);
                self.reconfigurations = self.reconfigurations.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Surface lost · reconfiguring";
            }
            SurfaceAcquireStatus::Timeout => {
                self.skipped = self.skipped.saturating_add(1);
                self.timeouts = self.timeouts.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Presentation timeout";
            }
            SurfaceAcquireStatus::Occluded => {
                self.skipped = self.skipped.saturating_add(1);
                self.occlusions = self.occlusions.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Output occluded";
            }
            SurfaceAcquireStatus::Validation => {
                self.skipped = self.skipped.saturating_add(1);
                self.validation_errors = self.validation_errors.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Surface validation error";
            }
        }
    }
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
        if state.output_window.id() == id {
            match event {
                WindowEvent::CloseRequested => {
                    state.ui.output_enabled = false;
                    state.output_window.set_visible(false);
                }
                WindowEvent::Resized(size) => {
                    state
                        .output_surface
                        .resize(&state.gpu.device, size.width, size.height);
                }
                WindowEvent::Moved(_) => state.update_current_output_display(),
                WindowEvent::ModifiersChanged(modifiers) => {
                    state.modifiers = modifiers.state();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Pressed && !event.repeat =>
                {
                    if let PhysicalKey::Code(code) = event.physical_key {
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
            WindowEvent::DroppedFile(path) => state.import_path(path),
            WindowEvent::ModifiersChanged(modifiers) => state.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                if let PhysicalKey::Code(code) = event.physical_key {
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
    fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::KeyB => self.ui.blackout = !self.ui.blackout,
            KeyCode::Space => self.ui.master_freeze = !self.ui.master_freeze,
            KeyCode::ArrowLeft => {
                self.ui.crossfader = (self.ui.crossfader - 0.05).max(0.0);
            }
            KeyCode::ArrowRight => {
                self.ui.crossfader = (self.ui.crossfader + 0.05).min(1.0);
            }
            KeyCode::Home => self.ui.crossfader = 0.5,
            KeyCode::Escape if self.ui.output_fullscreen => {
                self.ui.output_fullscreen = false;
                self.output_window.set_fullscreen(None);
            }
            KeyCode::KeyO if !self.modifiers.control_key() && !self.modifiers.super_key() => {
                self.ui.output_enabled = !self.ui.output_enabled;
                self.output_window.set_visible(self.ui.output_enabled);
            }
            KeyCode::KeyS if self.modifiers.control_key() || self.modifiers.super_key() => {
                self.save_project_from_ui();
            }
            KeyCode::KeyO if self.modifiers.control_key() || self.modifiers.super_key() => {
                self.open_project_from_ui();
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
                let now = Instant::now();
                for deck in DeckId::ALL {
                    self.queue_clip(ClipAddress { deck, slot }, now);
                }
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
        ui.effect_manifest_path = workspace
            .join("effects/master-effects/effect.json")
            .to_string_lossy()
            .into_owned();
        let effect_registry = discover_effect_packages(workspace.join("effects"));
        ui.effect_registry_status = if effect_registry.errors.is_empty() {
            format!("{} custom effect package(s)", effect_registry.effects.len())
        } else {
            format!(
                "{} custom effect package(s), {} rejected",
                effect_registry.effects.len(),
                effect_registry.errors.len()
            )
        };
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

        Ok(Self {
            window,
            output_window,
            output_monitors,
            output_displays,
            output_current_display,
            output_health: OutputHealth::default(),
            last_display_refresh: Instant::now(),
            gpu,
            output_surface,
            program,
            master_effect_processor,
            operator_presenter,
            output_presenter,
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
            performance_started: Instant::now(),
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
            midi_input: None,
            midi_stats: MidiInputStats::default(),
            midi_status,
            midi_reconnect: false,
            last_midi_refresh: Instant::now(),
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
        })
    }

    fn import_path(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.import_folder(path);
        } else {
            self.import_movie(path);
        }
    }

    fn browse_relink(&mut self, address: ClipAddress) {
        let current = self.clips.path(address).map(PathBuf::from);
        let mut dialog = rfd::FileDialog::new().add_filter(
            "Video and still media",
            &[
                "mov", "mp4", "m4v", "mkv", "avi", "webm", "mxf", "png", "jpg", "jpeg",
            ],
        );
        if let Some(parent) = current.as_deref().and_then(std::path::Path::parent)
            && parent.exists()
        {
            dialog = dialog.set_directory(parent);
        }
        if let Some(name) = current
            .as_deref()
            .and_then(std::path::Path::file_name)
            .and_then(|name| name.to_str())
        {
            dialog = dialog.set_file_name(name);
        }
        if let Some(path) = dialog.pick_file() {
            self.relink_slot(address, path);
        } else {
            self.project_status = "Relink cancelled".to_owned();
        }
    }

    fn relink_slot(&mut self, address: ClipAddress, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        if path.is_dir() {
            self.project_status = "Relink requires a media file, not a folder".to_owned();
            return;
        }
        if self.clips.active(address.deck) == Some(address.slot) {
            self.relink_active.insert(address);
        }
        self.relink_pending.insert(address);
        self.ui.clear_thumbnail(address);
        self.thumbnail_requests.remove(&address);
        self.clips.begin_relink(address, path.clone());
        match self.restorer.submit(ClipRestoreRequest {
            address,
            path: path.clone(),
            project_epoch: self.project_epoch,
        }) {
            Ok(()) => {
                self.project_status = format!(
                    "Relinking Deck {} slot {} to {}…",
                    address.deck.label(),
                    address.slot + 1,
                    display_path(&path)
                );
            }
            Err(request) => {
                self.relink_pending.remove(&address);
                self.relink_active.remove(&address);
                self.clips.fail_restore(
                    request.address,
                    request.path,
                    "Relink probe queue is full.".to_owned(),
                );
                self.project_status = "Relink queue is full".to_owned();
            }
        }
    }

    fn import_folder(&mut self, path: PathBuf) {
        let start = ClipAddress {
            deck: self.mixer.selected(),
            slot: self.clips.selected(self.mixer.selected()),
        };
        let available = self.clips.available_slots_from(start, CLIPS_PER_DECK * 4);
        if available.is_empty() {
            self.folder_status = "Folder import skipped · all 32 slots are occupied".to_owned();
            return;
        }
        let request_id = self.folder_request_id.wrapping_add(1);
        let request = FolderScanRequest {
            root: path.clone(),
            request_id,
            project_epoch: self.project_epoch,
            max_files: available.len(),
        };
        match self.folder_scanner.submit(request) {
            Ok(()) => {
                self.folder_request_id = request_id;
                self.folder_scan_start = start;
                self.folder_status = format!("Scanning {}…", display_path(&path));
            }
            Err(_) => {
                self.folder_status = "Folder scan is busy · wait for the current folder".to_owned();
            }
        }
    }

    fn poll_folder_scans(&mut self) {
        while let Ok(result) = self.folder_scanner.try_recv() {
            if result.project_epoch != self.project_epoch
                || result.request_id != self.folder_request_id
            {
                continue;
            }
            let paths = match result.paths {
                Ok(paths) => paths,
                Err(error) => {
                    self.folder_status = format!("Folder scan failed: {error}");
                    continue;
                }
            };
            let slots = self
                .clips
                .available_slots_from(self.folder_scan_start, paths.len());
            let mut submitted = 0;
            for (address, path) in slots.into_iter().zip(paths) {
                self.clips.begin_restore(address, path.clone());
                match self.restorer.submit(ClipRestoreRequest {
                    address,
                    path,
                    project_epoch: self.project_epoch,
                }) {
                    Ok(()) => {
                        self.folder_pending.insert(address);
                        submitted += 1;
                    }
                    Err(request) => {
                        self.clips.fail_restore(
                            request.address,
                            request.path,
                            "Folder probe queue is full.".to_owned(),
                        );
                    }
                }
            }
            self.folder_status = if submitted == 0 {
                format!("No supported media found in {}", display_path(&result.root))
            } else {
                format!(
                    "Importing {submitted} file(s) from {}{}",
                    display_path(&result.root),
                    if result.truncated {
                        " · limited by available slots"
                    } else {
                        ""
                    }
                )
            };
        }
    }

    fn import_movie(&mut self, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        let deck = self.mixer.selected();
        self.live_configs[deck.index()] = None;
        let address = ClipAddress {
            deck,
            slot: self.clips.selected(deck),
        };
        self.ui.clear_thumbnail(address);
        self.thumbnail_requests.remove(&address);
        let request = self.mixer.begin_import(deck, path);
        self.import_slots[deck.index()] = Some((request.generation, self.clips.selected(deck)));
        self.reset_playback(deck, request.generation);
        match self.importer.submit(request) {
            Ok(()) => {
                self.mixer.select(deck.next());
                self.window.request_redraw();
            }
            Err(SubmitError::Busy(request)) | Err(SubmitError::Disconnected(request)) => {
                let target = self.mixer.deck_mut(request.deck);
                if target.generation == request.generation {
                    target.state = DeckState::Error {
                        path: request.path,
                        message: "The media import worker is unavailable.".to_owned(),
                    };
                }
            }
        }
    }

    fn poll_imports(&mut self) {
        while let Ok(result) = self.importer.try_recv() {
            let playback = result.metadata.as_ref().ok().map(|movie| {
                (
                    result.deck,
                    result.generation,
                    movie.path.clone(),
                    movie.decode_path,
                    movie.clone(),
                )
            });
            if self.mixer.complete_import(result)
                && let Some((deck, generation, path, decode_path, movie)) = playback
            {
                if let Some((slot_generation, slot)) = self.import_slots[deck.index()]
                    && slot_generation == generation
                {
                    let address = ClipAddress { deck, slot };
                    self.clips.assign(address, movie);
                    self.clips.activate(address);
                    self.request_thumbnail(address, path.clone());
                }
                self.reset_playback(deck, generation);
                self.decoders[deck.index()].load(path, decode_path, generation);
            }
        }
    }

    fn launch_clip(&mut self, address: ClipAddress) {
        let Some(movie) = self.clips.movie(address).cloned() else {
            return;
        };
        self.master_effect_processor.reset_history();
        self.clips
            .remember_position(address.deck, self.transports[address.deck.index()].position);
        let media_duration = movie.duration.map(MediaTime::as_seconds);
        let playback = self.clips.playback(address).unwrap_or_default();
        let launch_position = self
            .clips
            .launch_position(address, media_duration, self.ui.bpm)
            .unwrap_or(playback.in_point);
        let (in_point, out_point) = playback.range(media_duration, self.ui.bpm);
        let path = movie.path.clone();
        let preload = self
            .ui
            .preloaded_frame(address, Some(path.as_path()))
            .cloned();
        let decode_path = movie.decode_path;
        let start_at = media_time_from_seconds(launch_position);
        let seek_to = if decode_path == oneiroi_media::DecodePath::FfmpegVideo {
            start_at.and_then(|target| movie.keyframes.nearest_preceding(target))
        } else {
            None
        };
        let generation = self.mixer.activate(address.deck, movie);
        self.live_configs[address.deck.index()] = None;
        self.clips.activate(address);
        self.reset_playback(address.deck, generation);
        self.transports[address.deck.index()].reset_range(in_point, out_point);
        self.transports[address.deck.index()].position = launch_position;
        if let Some(preload) = preload
            && let Err(error) = self.compositor.upload(
                &self.gpu.device,
                &self.gpu.queue,
                address.deck.index(),
                &VideoFramePayload::Rgba8(preload),
            )
        {
            log::error!(
                "deck {} first-frame preload upload failed: {error}",
                address.deck.label()
            );
        }
        self.decoders[address.deck.index()].load_indexed(
            path,
            decode_path,
            generation,
            start_at,
            seek_to,
        );
    }

    fn queue_clip(&mut self, address: ClipAddress, now: Instant) {
        if self.clips.movie(address).is_none() {
            return;
        }
        let elapsed = now
            .saturating_duration_since(self.performance_started)
            .as_secs_f64();
        self.launches
            .queue(address, self.ui.quantization, self.tempo, elapsed);
    }

    fn process_launches(&mut self, now: Instant) {
        let elapsed = now
            .saturating_duration_since(self.performance_started)
            .as_secs_f64();
        if (self.tempo.bpm() - self.ui.bpm).abs() > f64::EPSILON {
            self.tempo.set_bpm(self.ui.bpm, elapsed);
        }
        for address in self.launches.take_due(self.tempo, elapsed) {
            self.launch_clip(address);
        }
    }

    fn refresh_cameras(&mut self) {
        match discover_cameras() {
            Ok(cameras) => {
                let count = cameras.len();
                self.cameras = cameras;
                self.camera_status = if count == 0 {
                    "No cameras discovered; check macOS camera permission or enter a device ID."
                        .to_owned()
                } else {
                    format!("{count} camera(s) available")
                };
            }
            Err(error) => self.camera_status = format!("Camera discovery failed: {error}"),
        }
    }

    fn connect_camera(
        &mut self,
        deck: DeckId,
        device_id: String,
        label: String,
        extent: [u32; 2],
        fps: u32,
    ) {
        self.master_effect_processor.reset_history();
        self.clips
            .remember_position(deck, self.transports[deck.index()].position);
        let config = CameraConfig {
            device: CameraDevice {
                id: device_id,
                label,
                backend: "avfoundation".to_owned(),
            },
            requested_extent: Some(extent),
            requested_fps: Some(fps),
        };
        self.launches.cancel(deck);
        self.clips.deactivate(deck);
        let generation = self.mixer.connect_camera(deck, config.clone());
        self.live_configs[deck.index()] = Some(config.clone());
        self.reset_playback(deck, generation);
        self.transports[deck.index()].end_mode = oneiroi_media::EndMode::OneShot;
        self.decoders[deck.index()].connect_camera(config, generation);
        self.camera_status = format!("Connecting Deck {}…", deck.label());
    }

    fn refresh_audio_inputs(&mut self) {
        match discover_audio_inputs() {
            Ok(inputs) => {
                self.audio_inputs = inputs;
                if !self
                    .audio_inputs
                    .iter()
                    .any(|device| device.id == self.ui.audio_device_id)
                {
                    self.ui.audio_device_id = self
                        .audio_inputs
                        .iter()
                        .find(|device| device.is_default)
                        .or_else(|| self.audio_inputs.first())
                        .map(|device| device.id.clone())
                        .unwrap_or_default();
                }
                self.audio_status = format!("{} audio input(s) available", self.audio_inputs.len());
            }
            Err(error) => self.audio_status = format!("Audio discovery failed: {error}"),
        }
    }

    fn connect_audio_input(&mut self, device_id: String) {
        self.audio_input = None;
        match AudioInput::connect(&device_id, self.ui.audio_analysis) {
            Ok(input) => {
                self.ui.audio_device_id = device_id;
                self.audio_snapshot = input.snapshot();
                self.audio_input = Some(input);
                self.audio_status = "Audio input connected".to_owned();
            }
            Err(error) => self.audio_status = format!("Audio connection failed: {error}"),
        }
    }

    fn disconnect_audio_input(&mut self) {
        self.audio_input = None;
        self.audio_snapshot = AudioInputSnapshot::default();
        self.audio_status = "Audio input disconnected".to_owned();
    }

    fn refresh_midi_inputs(&mut self) {
        match discover_midi_inputs() {
            Ok(inputs) => {
                let connected_id = self
                    .midi_input
                    .as_ref()
                    .map(|input| input.device_id().to_owned());
                self.midi_inputs = inputs;
                if let Some(connected_id) = connected_id
                    && !self
                        .midi_inputs
                        .iter()
                        .any(|device| device.id == connected_id)
                {
                    self.midi_input = None;
                    self.midi_status =
                        format!("{connected_id} disconnected · waiting to reconnect");
                }
                if self.ui.midi_device_id.is_empty() {
                    self.ui.midi_device_id = self
                        .midi_inputs
                        .first()
                        .map(|device| device.id.clone())
                        .unwrap_or_default();
                }
                if self.midi_input.is_none()
                    && self.midi_reconnect
                    && self
                        .midi_inputs
                        .iter()
                        .any(|device| device.id == self.ui.midi_device_id)
                {
                    self.connect_midi_input(self.ui.midi_device_id.clone());
                } else if self.midi_input.is_none() && !self.midi_reconnect {
                    self.midi_status =
                        format!("{} MIDI input(s) available", self.midi_inputs.len());
                }
            }
            Err(error) => self.midi_status = format!("MIDI discovery failed: {error}"),
        }
        self.last_midi_refresh = Instant::now();
    }

    fn connect_midi_input(&mut self, device_id: String) {
        self.midi_input = None;
        match MidiInputConnection::connect(&device_id) {
            Ok(input) => {
                self.ui.midi_device_id = device_id.clone();
                self.midi_stats = input.stats();
                self.midi_input = Some(input);
                self.midi_reconnect = true;
                self.midi_status = format!("{device_id} connected");
            }
            Err(error) => {
                self.midi_reconnect = true;
                self.midi_status = format!("MIDI connection failed: {error}");
            }
        }
    }

    fn disconnect_midi_input(&mut self) {
        self.midi_input = None;
        self.midi_reconnect = false;
        self.midi.cancel_learn();
        self.midi_status = "MIDI input disconnected".to_owned();
    }

    fn poll_midi(&mut self, now: Instant) {
        if now.saturating_duration_since(self.last_midi_refresh) >= Duration::from_secs(2) {
            self.refresh_midi_inputs();
        }
        let Some(input) = &self.midi_input else {
            return;
        };
        let device = input.device_id().to_owned();
        let events: Vec<_> = input.try_iter().collect();
        self.midi_stats = input.stats();
        for event in events {
            let updates = {
                let ui = &self.ui;
                let mixer = &self.mixer;
                let transports = &self.transports;
                self.midi.ingest(&device, event.message, |target| {
                    current_control_value(ui, mixer, transports, target)
                })
            };
            for update in updates {
                self.apply_control_update(update, now);
            }
            self.midi_status = format!(
                "{device} · {:?} · {} µs",
                event.message, event.timestamp_micros
            );
        }
    }

    fn apply_control_update(&mut self, update: ControlUpdate, now: Instant) {
        match update.target {
            ControlTarget::Crossfader => self.ui.crossfader = update.value.clamp(0.0, 1.0),
            ControlTarget::MasterOpacity => {
                self.ui.master_opacity = update.value.clamp(0.0, 1.0);
            }
            ControlTarget::MasterBlackout => self.ui.blackout = update.value >= 0.5,
            ControlTarget::MasterFreeze => self.ui.master_freeze = update.value >= 0.5,
            ControlTarget::TapTempo => {
                if update.value >= 0.5 {
                    let elapsed = now
                        .saturating_duration_since(self.performance_started)
                        .as_secs_f64();
                    if let Some(bpm) = self.tap_tempo.tap(elapsed) {
                        self.ui.bpm = bpm;
                        self.tempo.set_bpm(bpm, elapsed);
                    }
                }
            }
            ControlTarget::DeckLevel(deck) => {
                if let Some(deck) = deck_id(deck) {
                    self.mixer.deck_mut(deck).level = update.value.clamp(0.0, 1.0);
                }
            }
            ControlTarget::DeckPlay(deck) => {
                if let Some(deck) = deck_id(deck) {
                    self.transports[deck.index()].playing = update.value >= 0.5;
                    self.last_transport_updates[deck.index()] = now;
                }
            }
            ControlTarget::DeckFreeze(deck) => {
                if let Some(deck) = deck_id(deck) {
                    self.transports[deck.index()].frozen = update.value >= 0.5;
                }
            }
            ControlTarget::DeckSpeed(deck) => {
                if let Some(deck) = deck_id(deck) {
                    self.transports[deck.index()].speed = update.value.clamp(0.25, 4.0);
                }
            }
            ControlTarget::DeckSelect(deck) => {
                if update.value >= 0.5
                    && let Some(deck) = deck_id(deck)
                {
                    self.mixer.select(deck);
                }
            }
            ControlTarget::DeckRestart(deck) => {
                if update.value >= 0.5
                    && let Some(deck) = deck_id(deck)
                {
                    self.transports[deck.index()].restart();
                    self.seek_deck(deck);
                }
            }
            ControlTarget::ClipLaunch { deck, slot } => {
                if update.value >= 0.5
                    && let Some(deck) = deck_id(deck)
                    && usize::from(slot) < oneiroi_media::CLIPS_PER_DECK
                {
                    self.queue_clip(
                        ClipAddress {
                            deck,
                            slot: usize::from(slot),
                        },
                        now,
                    );
                }
            }
            ControlTarget::SceneLaunch(slot) => {
                if update.value >= 0.5 && usize::from(slot) < oneiroi_media::CLIPS_PER_DECK {
                    for deck in DeckId::ALL {
                        self.queue_clip(
                            ClipAddress {
                                deck,
                                slot: usize::from(slot),
                            },
                            now,
                        );
                    }
                }
            }
            ControlTarget::EffectParameter {
                deck,
                effect,
                parameter: _,
            } => {
                if let Some(deck) = deck_id(deck) {
                    set_effect_parameter(&mut self.ui.effects[deck.index()], effect, update.value);
                }
            }
            ControlTarget::LfoParameter {
                deck,
                lfo,
                parameter,
            } => {
                if let Some(deck) = deck_id(deck)
                    && let Some(lfo) = self.ui.lfos[deck.index()].lanes.get_mut(usize::from(lfo))
                {
                    match parameter {
                        0 => lfo.enabled = update.value >= 0.5,
                        1 => lfo.rate_hz = update.value.clamp(0.01, 20.0),
                        2 => lfo.depth = update.value.clamp(0.0, 1.0),
                        3 => lfo.phase = update.value.rem_euclid(1.0),
                        _ => {}
                    }
                }
            }
            ControlTarget::ModRouteParameter {
                deck,
                route,
                parameter,
            } => {
                if let Some(deck) = deck_id(deck)
                    && let Some(route) = self.ui.lfos[deck.index()]
                        .routes
                        .get_mut(usize::from(route))
                {
                    match parameter {
                        0 => route.enabled = update.value >= 0.5,
                        1 => route.amount = update.value.clamp(-1.0, 1.0),
                        _ => {}
                    }
                }
            }
        }
    }

    fn request_thumbnail(&mut self, address: ClipAddress, path: PathBuf) {
        self.thumbnail_request_id = self.thumbnail_request_id.wrapping_add(1);
        let request_id = self.thumbnail_request_id;
        self.thumbnail_requests
            .insert(address, (request_id, path.clone()));
        if self
            .thumbnails
            .submit(ThumbnailRequest {
                address,
                path,
                request_id,
            })
            .is_err()
        {
            self.thumbnail_requests.remove(&address);
        }
    }

    fn poll_thumbnails(&mut self) {
        let context = self.egui_state.egui_ctx().clone();
        while let Ok(result) = self.thumbnails.try_recv() {
            let current = self.thumbnail_requests.get(&result.address);
            if !current.is_some_and(|(request_id, path)| {
                *request_id == result.request_id && *path == result.path
            }) || self.clips.path(result.address) != Some(result.path.as_path())
            {
                continue;
            }
            self.thumbnail_requests.remove(&result.address);
            match result.thumbnail {
                Ok(thumbnail) => {
                    self.ui
                        .install_thumbnail(&context, result.address, result.path, thumbnail);
                }
                Err(message) => {
                    self.ui
                        .mark_thumbnail_failed(result.address, result.path, message);
                }
            }
        }
    }

    fn project_snapshot(&self) -> ProjectFile {
        project::snapshot(
            &self.ui,
            &self.mixer,
            &self.clips,
            &self.transports,
            &self.midi,
            &self.live_configs,
        )
    }

    fn project_dirty(&self) -> bool {
        project::is_dirty(&self.project_snapshot(), self.last_saved_project.as_ref())
    }

    fn path_from_ui(&self) -> Option<PathBuf> {
        let value = self.ui.project_path.trim();
        if value.is_empty() {
            return None;
        }
        let path = PathBuf::from(value);
        Some(if path.is_absolute() {
            path
        } else {
            self.workspace.join(path)
        })
    }

    fn save_project_from_ui(&mut self) {
        let Some(path) = self.path_from_ui() else {
            self.project_status = "Enter a project path first.".to_owned();
            return;
        };
        let snapshot = self.project_snapshot();
        match save_project_atomic(&path, &snapshot) {
            Ok(()) => {
                self.project_path = Some(path.clone());
                self.last_saved_project = Some(snapshot);
                self.recovery_path = None;
                self.project_status = format!("Saved {}", display_path(&path));
            }
            Err(error) => self.project_status = format!("Save failed: {error}"),
        }
    }

    fn open_project_from_ui(&mut self) {
        let Some(path) = self.path_from_ui() else {
            self.project_status = "Enter a project path first.".to_owned();
            return;
        };
        self.open_project(path, false);
    }

    fn open_project(&mut self, path: PathBuf, recovered: bool) {
        match load_project(&path) {
            Ok(mut project_file) => {
                let base = path.parent().unwrap_or(&self.workspace);
                resolve_project_paths(&mut project_file, base);
                self.apply_project(project_file, recovered);
                if recovered {
                    self.project_path = None;
                    self.recovery_path = None;
                    self.ui.project_path = "recovered-show.oneiroi".to_owned();
                    self.project_status =
                        format!("Recovered autosave from {}", display_path(&path));
                } else {
                    self.project_path = Some(path.clone());
                    self.ui.project_path = path.to_string_lossy().into_owned();
                    let recovery = autosave_path(Some(&path), &self.workspace);
                    self.recovery_path = recovery_is_newer(&path, &recovery).then_some(recovery);
                    self.project_status = format!("Opened {}", display_path(&path));
                }
            }
            Err(error) => self.project_status = format!("Open failed: {error}"),
        }
    }

    fn apply_project(&mut self, project_file: ProjectFile, recovered: bool) {
        self.master_effect_processor.reset_history();
        self.project_epoch = self.project_epoch.wrapping_add(1);
        self.clips = ClipBank::default();
        self.ui.clear_thumbnails();
        self.thumbnail_requests.clear();
        self.folder_pending.clear();
        self.relink_pending.clear();
        self.relink_active.clear();
        self.folder_status.clear();
        self.live_configs = std::array::from_fn(|_| None);
        self.launches = LaunchQueue::default();
        self.restore_active = [None; 4];
        self.restore_selected = [0; 4];
        self.restore_transport = [None; 4];
        project::apply_master(&project_file, &mut self.ui);
        self.apply_output_settings();
        self.midi = project::apply_midi(&project_file);

        for deck in DeckId::ALL {
            let index = deck.index();
            self.mixer.eject(deck);
            let generation = self.mixer.deck(deck).generation;
            self.reset_playback(deck, generation);
            let deck_project = &project_file.decks[index];
            let transport = project::apply_deck(deck, deck_project, &mut self.mixer, &mut self.ui);
            self.transports[index] = transport;
            self.clips.select(ClipAddress {
                deck,
                slot: deck_project.selected_slot,
            });
            self.restore_selected[index] = deck_project.selected_slot;
            self.restore_active[index] = deck_project.active_slot;
            self.clips.restore_active(deck, deck_project.active_slot);
            self.restore_transport[index] = deck_project.active_slot.map(|_| transport);

            for (slot, path) in deck_project.clips.iter().enumerate() {
                let address = ClipAddress { deck, slot };
                let path = path.clone();
                if let Some(path) = &path {
                    self.clips.begin_restore(address, path.clone());
                }
                if let Some(playback) = deck_project.clip_playback.get(slot) {
                    self.clips
                        .set_playback(address, project::clip_playback_from_project(*playback));
                }
                let Some(path) = path else {
                    continue;
                };
                if let Err(request) = self.restorer.submit(ClipRestoreRequest {
                    address,
                    path,
                    project_epoch: self.project_epoch,
                }) {
                    self.clips.fail_restore(
                        request.address,
                        request.path,
                        "Restore queue is full.".to_owned(),
                    );
                }
            }
            if let Some(camera) = &deck_project.camera {
                let config = project::camera_from_project(camera);
                let generation = self.mixer.connect_camera(deck, config.clone());
                self.live_configs[index] = Some(config.clone());
                self.reset_playback(deck, generation);
                self.transports[index] = transport;
                self.transports[index].end_mode = oneiroi_media::EndMode::OneShot;
                self.decoders[index].connect_camera(config, generation);
            }
        }

        self.last_saved_project = (!recovered).then_some(project_file);
        self.performance_started = Instant::now();
        self.last_autosave = Instant::now();
    }

    fn apply_output_settings(&mut self) {
        if self.program.extent() != self.ui.composition_extent {
            self.program = ProgramTarget::new(&self.gpu.device, self.ui.composition_extent);
            self.master_effect_processor =
                MasterEffectProcessor::new(&self.gpu.device, &self.program);
            let manifest_paths = self.effect_manifest_paths();
            self.master_effect_processor
                .watch_effect_manifests(manifest_paths);
            self.ui.effect_reload_status = self.master_effect_processor.reload_status().to_owned();
            self.operator_presenter =
                ProgramPresenter::new(&self.gpu.device, &self.program, self.gpu.content_format());
            self.output_presenter = ProgramPresenter::new(
                &self.gpu.device,
                &self.program,
                self.output_surface.content_format(),
            );
        }
        self.output_window.set_visible(self.ui.output_enabled);
        self.apply_output_monitor();
    }

    fn resolved_effect_manifest_path(&self) -> PathBuf {
        let path = PathBuf::from(&self.ui.effect_manifest_path);
        if path.is_absolute() {
            path
        } else {
            self.workspace.join(path)
        }
    }

    fn watch_effect_manifest(&mut self) {
        let paths = self.effect_manifest_paths();
        self.master_effect_processor.watch_effect_manifests(paths);
        self.ui.effect_reload_status = self.master_effect_processor.reload_status().to_owned();
    }

    fn effect_manifest_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.resolved_effect_manifest_path()];
        paths.extend(
            self.ui
                .effect_packages
                .iter()
                .map(|effect| effect.manifest_path.clone()),
        );
        paths.sort();
        paths.dedup();
        paths
    }

    fn refresh_effect_registry(&mut self) {
        let registry = discover_effect_packages(self.workspace.join("effects"));
        self.ui.effect_registry_status = if registry.errors.is_empty() {
            format!("{} custom effect package(s)", registry.effects.len())
        } else {
            format!(
                "{} custom effect package(s), {} rejected · {}",
                registry.effects.len(),
                registry.errors.len(),
                registry.errors.join(" · ")
            )
        };
        self.ui.effect_packages = registry.effects;
        self.watch_effect_manifest();
    }

    fn apply_output_monitor(&mut self) {
        if !self
            .output_monitors
            .iter()
            .any(|monitor| monitor.id == self.ui.output_display_id)
        {
            let current_id = self
                .output_window
                .current_monitor()
                .map(|monitor| monitor_id(&monitor));
            self.ui.output_display_id = current_id
                .filter(|id| self.output_monitors.iter().any(|monitor| &monitor.id == id))
                .or_else(|| {
                    self.output_monitors
                        .first()
                        .map(|monitor| monitor.id.clone())
                })
                .unwrap_or_default();
        }
        let monitor = self
            .output_monitors
            .iter()
            .find(|monitor| monitor.id == self.ui.output_display_id)
            .map(|monitor| monitor.handle.clone());
        if self.ui.output_fullscreen {
            self.output_window
                .set_fullscreen(Some(Fullscreen::Borderless(monitor)));
        } else {
            self.output_window.set_fullscreen(None);
            if let Some(monitor) = monitor {
                let position = monitor.position();
                self.output_window.set_outer_position(PhysicalPosition::new(
                    position.x.saturating_add(40),
                    position.y.saturating_add(40),
                ));
            }
        }
        self.output_current_display = self
            .output_displays
            .iter()
            .find(|display| display.id == self.ui.output_display_id)
            .map(|display| display.label.clone())
            .unwrap_or_else(|| "No connected display".to_owned());
    }

    fn refresh_output_displays(&mut self) {
        let previous_ids: Vec<_> = self
            .output_monitors
            .iter()
            .map(|monitor| monitor.id.clone())
            .collect();
        let handles: Vec<_> = self.output_window.available_monitors().collect();
        (self.output_monitors, self.output_displays) = describe_monitors(handles);
        let current_ids: Vec<_> = self
            .output_monitors
            .iter()
            .map(|monitor| monitor.id.clone())
            .collect();
        if current_ids != previous_ids {
            self.output_health.topology_changes =
                self.output_health.topology_changes.saturating_add(1);
            self.apply_output_monitor();
        } else {
            self.update_current_output_display();
        }
        self.last_display_refresh = Instant::now();
    }

    fn update_current_output_display(&mut self) {
        self.output_current_display = self
            .output_window
            .current_monitor()
            .map(|monitor| {
                let id = monitor_id(&monitor);
                self.output_displays
                    .iter()
                    .find(|display| display.id == id)
                    .map(|display| display.label.clone())
                    .unwrap_or_else(|| monitor_label(&monitor))
            })
            .unwrap_or_else(|| "No connected display".to_owned());
    }

    fn poll_restores(&mut self) {
        while let Ok(result) = self.restorer.try_recv() {
            if result.project_epoch != self.project_epoch {
                continue;
            }
            let folder_result = self.folder_pending.remove(&result.address);
            if self.clips.path(result.address) != Some(result.path.as_path()) {
                if folder_result && self.folder_pending.is_empty() {
                    self.folder_status = "Folder import complete".to_owned();
                }
                continue;
            }
            let relink_result = self.relink_pending.remove(&result.address);
            let relink_active = self.relink_active.remove(&result.address);
            match result.metadata {
                Ok(movie) => {
                    let address = result.address;
                    let duration = movie.duration.map(MediaTime::as_seconds);
                    self.clips.restore(address, movie);
                    self.request_thumbnail(address, result.path.clone());
                    if relink_active
                        || self.restore_active[address.deck.index()] == Some(address.slot)
                    {
                        let desired = self.restore_transport[address.deck.index()].take();
                        self.launch_clip(address);
                        self.clips.select(ClipAddress {
                            deck: address.deck,
                            slot: self.restore_selected[address.deck.index()],
                        });
                        if let Some(mut transport) = desired {
                            transport.duration = duration;
                            self.transports[address.deck.index()] = transport;
                            if transport.position > 0.0 {
                                self.seek_deck(address.deck);
                            }
                        }
                    }
                    if relink_result {
                        self.project_status = format!(
                            "Relinked Deck {} slot {}",
                            address.deck.label(),
                            address.slot + 1
                        );
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    self.clips
                        .fail_restore(result.address, result.path, message.clone());
                    if relink_result {
                        self.project_status = format!("Relink failed: {message}");
                    }
                }
            }
            if folder_result && self.folder_pending.is_empty() {
                self.folder_status = "Folder import complete".to_owned();
            } else if folder_result {
                self.folder_status = format!(
                    "Folder import · {} file(s) remaining",
                    self.folder_pending.len()
                );
            }
        }
    }

    fn maybe_autosave(&mut self, now: Instant) {
        if now.saturating_duration_since(self.last_autosave) < Duration::from_secs(5) {
            return;
        }
        self.last_autosave = now;
        self.autosave_recovery();
    }

    fn autosave_recovery(&mut self) {
        if !self.project_dirty() {
            return;
        }
        let path = autosave_path(self.project_path.as_deref(), &self.workspace);
        match save_project_atomic(&path, &self.project_snapshot()) {
            Ok(()) => {
                self.recovery_path = Some(path);
                self.project_status = "Autosaved recovery snapshot.".to_owned();
            }
            Err(error) => self.project_status = format!("Autosave failed: {error}"),
        }
    }

    fn reset_playback(&mut self, deck: DeckId, generation: u64) {
        let index = deck.index();
        self.decoders[index].stop();
        self.compositor.clear_deck(index);
        self.schedulers[index] = FrameScheduler::new(4, generation, DiscontinuityPolicy::Blank)
            .expect("non-zero frame queue");
        let duration = match &self.mixer.deck(deck).state {
            DeckState::Ready(movie) => movie.duration.map(MediaTime::as_seconds),
            DeckState::Live(_)
            | DeckState::Empty
            | DeckState::Loading { .. }
            | DeckState::Error { .. } => None,
        };
        self.transports[index].reset(duration);
        self.last_transport_updates[index] = Instant::now();
        self.media_origins[index] = None;
        self.playback_generations[index] = generation;
    }

    fn seek_deck(&mut self, deck: DeckId) {
        let index = deck.index();
        let DeckState::Ready(movie) = &self.mixer.deck(deck).state else {
            return;
        };
        let path = movie.path.clone();
        let decode_path = movie.decode_path;
        let epoch = self.playback_generations[index].wrapping_add(1);
        self.playback_generations[index] = epoch;
        self.schedulers[index] = FrameScheduler::new(4, epoch, DiscontinuityPolicy::HoldLastFrame)
            .expect("non-zero frame queue");
        let target = self.media_origins[index].and_then(|origin| {
            let micros =
                (self.transports[index].position * 1_000_000.0).clamp(0.0, i64::MAX as f64) as i64;
            origin
                .checked_add(MediaTime::new(micros, 1_000_000).ok()?)
                .ok()
        });
        let seek_to = if decode_path == oneiroi_media::DecodePath::FfmpegVideo {
            target.and_then(|target| movie.keyframes.nearest_preceding(target))
        } else {
            None
        };
        self.decoders[index].load_indexed(path, decode_path, epoch, target, seek_to);
        self.last_transport_updates[index] = Instant::now();
    }

    fn update_playback(&mut self, now: Instant) {
        for deck in DeckId::ALL {
            let index = deck.index();
            let media_generation = self.mixer.deck(deck).generation;
            if media_generation != self.playback_generations[index]
                && !matches!(self.mixer.deck(deck).state, DeckState::Ready(_))
            {
                self.reset_playback(deck, media_generation);
            }
            self.sync_clip_range(deck);
            let delta = now
                .saturating_duration_since(self.last_transport_updates[index])
                .as_secs_f64();
            self.last_transport_updates[index] = now;
            if !self.ui.master_freeze
                && matches!(
                    self.transports[index].advance(delta),
                    TransportEvent::Loop { .. }
                )
            {
                self.seek_deck(deck);
            }
            let generation = self.playback_generations[index];
            while let Ok(event) = self.decoders[index].try_event() {
                match event {
                    DecoderEvent::Loaded {
                        generation: loaded_generation,
                    } if loaded_generation == generation && self.live_configs[index].is_some() => {
                        self.camera_status = format!("Deck {} camera is live", deck.label());
                    }
                    DecoderEvent::Error {
                        generation: failed_generation,
                        message,
                    } if failed_generation == generation => {
                        let path = match &self.mixer.deck(deck).state {
                            DeckState::Ready(movie) => movie.path.clone(),
                            DeckState::Live(config) => config.virtual_path(),
                            DeckState::Loading { path } | DeckState::Error { path, .. } => {
                                path.clone()
                            }
                            DeckState::Empty => PathBuf::new(),
                        };
                        if self.live_configs[index].is_some() {
                            self.camera_status =
                                format!("Deck {} camera error: {message}", deck.label());
                        }
                        self.mixer.deck_mut(deck).state = DeckState::Error { path, message };
                        self.compositor.clear_deck(index);
                    }
                    DecoderEvent::Ended {
                        generation: ended_generation,
                    } if ended_generation == generation && self.live_configs[index].is_some() => {
                        self.camera_status = format!("Deck {} camera disconnected", deck.label());
                    }
                    DecoderEvent::Loaded { .. }
                    | DecoderEvent::Ended { .. }
                    | DecoderEvent::Error { .. } => {}
                }
            }

            while let Ok(frame) = self.decoders[index].try_frame() {
                if frame.generation != generation {
                    continue;
                }
                self.media_origins[index].get_or_insert(frame.pts);
                if self.schedulers[index].enqueue(frame).is_err() {
                    break;
                }
            }

            let Some(origin) = self.media_origins[index] else {
                continue;
            };
            let elapsed =
                (self.transports[index].position * 1_000_000.0).clamp(0.0, i64::MAX as f64) as i64;
            let Ok(target) =
                origin.checked_add(MediaTime::new(elapsed, 1_000_000).expect("positive timescale"))
            else {
                continue;
            };
            if let FrameSelection::Advanced(frame) = self.schedulers[index].select(target)
                && let Err(error) =
                    self.compositor
                        .upload(&self.gpu.device, &self.gpu.queue, index, &frame.payload)
            {
                log::error!("deck {} upload failed: {error}", deck.label());
            }
        }
    }

    fn sync_clip_range(&mut self, deck: DeckId) {
        let Some(slot) = self.clips.active(deck) else {
            return;
        };
        let address = ClipAddress { deck, slot };
        let Some(movie) = self.clips.movie(address) else {
            return;
        };
        let playback = self.clips.playback(address).unwrap_or_default();
        let media_duration = movie.duration.map(MediaTime::as_seconds);
        let (in_point, out_point) = playback.range(media_duration, self.ui.bpm);
        let transport = &mut self.transports[deck.index()];
        transport.in_point = in_point;
        transport.duration = out_point;
        if transport.position < in_point {
            transport.position = in_point;
            self.seek_deck(deck);
        }
    }

    fn render(&mut self) {
        if self.master_effect_processor.poll_effect_reload() {
            self.ui.effect_reload_status = self.master_effect_processor.reload_status().to_owned();
        }
        self.poll_imports();
        self.poll_folder_scans();
        self.poll_restores();
        self.poll_thumbnails();
        let now = Instant::now();
        self.poll_midi(now);
        if now.saturating_duration_since(self.last_display_refresh) >= Duration::from_secs(2) {
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
        let project_dirty = self.project_dirty();

        // --- UI pass: pure CPU, produces geometry for the GPU pass below.
        let ctx = self.egui_state.egui_ctx().clone();
        let raw_input = self.egui_state.take_egui_input(&self.window);
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
                    project_dirty,
                    project_status: &self.project_status,
                    folder_status: &self.folder_status,
                    recovery_available: self.recovery_path.is_some(),
                    cameras: &self.cameras,
                    camera_status: &self.camera_status,
                    audio_inputs: &self.audio_inputs,
                    audio_status: &self.audio_status,
                    audio_connected: self.audio_input.is_some(),
                    audio_snapshot: self.audio_snapshot,
                    midi: ui::MidiMetrics {
                        inputs: &self.midi_inputs,
                        status: &self.midi_status,
                        connected: self.midi_input.is_some(),
                        stats: self.midi_stats,
                        mapper: &mut self.midi,
                    },
                    output_displays: &self.output_displays,
                    output_health: ui::OutputHealthMetrics {
                        status: self.output_health.status,
                        current_display: &self.output_current_display,
                        surface_extent: {
                            let (width, height) = self.output_surface.size();
                            [width, height]
                        },
                        presented: self.output_health.presented,
                        skipped: self.output_health.skipped,
                        reconfigurations: self.output_health.reconfigurations,
                        recoveries: self.output_health.recoveries,
                        timeouts: self.output_health.timeouts,
                        occlusions: self.output_health.occlusions,
                        validation_errors: self.output_health.validation_errors,
                        topology_changes: self.output_health.topology_changes,
                    },
                },
            );
        });
        for action in actions {
            match action {
                ui::UiAction::Restart(deck) | ui::UiAction::Seek(deck) => {
                    self.seek_deck(deck);
                }
                ui::UiAction::Launch(address) => self.queue_clip(address, now),
                ui::UiAction::LaunchScene(slot) => {
                    for deck in DeckId::ALL {
                        self.queue_clip(ClipAddress { deck, slot }, now);
                    }
                }
                ui::UiAction::ClearSlot(address) => {
                    if self.clips.active(address.deck) == Some(address.slot) {
                        self.master_effect_processor.reset_history();
                    }
                    self.clips.clear(address);
                    self.folder_pending.remove(&address);
                    self.relink_pending.remove(&address);
                    self.relink_active.remove(&address);
                    self.ui.clear_thumbnail(address);
                    self.thumbnail_requests.remove(&address);
                }
                ui::UiAction::BrowseRelink(address) => self.browse_relink(address),
                ui::UiAction::Eject(deck) => {
                    self.master_effect_processor.reset_history();
                    self.clips
                        .remember_position(deck, self.transports[deck.index()].position);
                    self.mixer.eject(deck);
                    self.live_configs[deck.index()] = None;
                    self.clips.deactivate(deck);
                    self.launches.cancel(deck);
                    let generation = self.mixer.deck(deck).generation;
                    self.reset_playback(deck, generation);
                }
                ui::UiAction::SaveProject => self.save_project_from_ui(),
                ui::UiAction::OpenProject => self.open_project_from_ui(),
                ui::UiAction::TapTempo => {
                    let elapsed = now
                        .saturating_duration_since(self.performance_started)
                        .as_secs_f64();
                    if let Some(bpm) = self.tap_tempo.tap(elapsed) {
                        self.ui.bpm = bpm;
                        self.tempo.set_bpm(bpm, elapsed);
                    }
                }
                ui::UiAction::HalfTempo => {
                    let bpm = (self.ui.bpm * 0.5).clamp(20.0, 400.0);
                    self.ui.bpm = bpm;
                    self.tempo.set_bpm(
                        bpm,
                        now.saturating_duration_since(self.performance_started)
                            .as_secs_f64(),
                    );
                    self.tap_tempo.reset();
                }
                ui::UiAction::DoubleTempo => {
                    let bpm = (self.ui.bpm * 2.0).clamp(20.0, 400.0);
                    self.ui.bpm = bpm;
                    self.tempo.set_bpm(
                        bpm,
                        now.saturating_duration_since(self.performance_started)
                            .as_secs_f64(),
                    );
                    self.tap_tempo.reset();
                }
                ui::UiAction::SetOutputEnabled(enabled) => {
                    self.output_window.set_visible(enabled);
                    if enabled {
                        self.output_window.request_redraw();
                    }
                }
                ui::UiAction::SetOutputFullscreen(fullscreen) => {
                    self.ui.output_fullscreen = fullscreen;
                    self.apply_output_monitor();
                }
                ui::UiAction::SetOutputDisplay(id) => {
                    self.ui.output_display_id = id;
                    self.apply_output_monitor();
                }
                ui::UiAction::SetCompositionExtent(extent) => {
                    self.ui.composition_extent = extent;
                    self.ui.custom_composition_extent = extent;
                    self.apply_output_settings();
                }
                ui::UiAction::WatchEffectManifest => self.watch_effect_manifest(),
                ui::UiAction::ReloadEffectManifest => {
                    self.master_effect_processor.reload_effect_manifest();
                    self.ui.effect_reload_status =
                        self.master_effect_processor.reload_status().to_owned();
                }
                ui::UiAction::RefreshEffectRegistry => self.refresh_effect_registry(),
                ui::UiAction::RefreshDisplays => self.refresh_output_displays(),
                ui::UiAction::RecoverProject => {
                    if let Some(path) = self.recovery_path.clone() {
                        self.open_project(path, true);
                    }
                }
                ui::UiAction::RefreshCameras => self.refresh_cameras(),
                ui::UiAction::RefreshAudioInputs => self.refresh_audio_inputs(),
                ui::UiAction::ConnectAudioInput(device_id) => {
                    self.connect_audio_input(device_id);
                }
                ui::UiAction::DisconnectAudioInput => self.disconnect_audio_input(),
                ui::UiAction::RefreshMidiInputs => self.refresh_midi_inputs(),
                ui::UiAction::ConnectMidiInput(device_id) => {
                    self.connect_midi_input(device_id);
                }
                ui::UiAction::DisconnectMidiInput => self.disconnect_midi_input(),
                ui::UiAction::MidiLearn(target) => self.midi.learn(target),
                ui::UiAction::MidiCancelLearn => self.midi.cancel_learn(),
                ui::UiAction::MidiClearTarget(target) => self.midi.clear_target(target),
                ui::UiAction::MidiRemoveBinding(index) => {
                    if index < self.midi.bindings.len() {
                        self.midi.bindings.remove(index);
                    }
                }
                ui::UiAction::ConnectCamera {
                    deck,
                    device_id,
                    label,
                    extent,
                    fps,
                } => self.connect_camera(deck, device_id, label, extent, fps),
            }
        }
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
        if !freeze_program {
            let composition_target = if master_effects_active {
                self.program.composition_view()
            } else {
                &self.program.view
            };
            self.compositor.draw(
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                composition_target,
                MixerParams {
                    levels: std::array::from_fn(|index| {
                        let deck = self.mixer.deck(DeckId::ALL[index]);
                        if matches!(deck.state, DeckState::Ready(_) | DeckState::Live(_)) {
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
                    crossfade_gains: crossfade_gains(self.ui.crossfader, self.ui.equal_power),
                    transforms: self.ui.transforms,
                    blend_modes: self.ui.blend_modes,
                    output_aspect: self.ui.composition_extent[0] as f32
                        / self.ui.composition_extent[1].max(1) as f32,
                    effects: std::array::from_fn(|index| {
                        let audio = self.audio_snapshot.analysis;
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
            if master_effects_active {
                self.master_effect_processor.draw_at(
                    &self.gpu.queue,
                    &mut encoder,
                    &self.program,
                    &self.ui.master_effects,
                    effect_time,
                );
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
        if let Some(frame) = operator_frame.as_ref() {
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
            let acquisition = self.output_surface.acquire_with_status(&self.gpu.device);
            self.output_health.observe(acquisition.status);
            acquisition.frame
        } else {
            None
        };
        if let Some(frame) = output_frame.as_ref() {
            let view = self.output_surface.content_view(&frame.texture);
            let (width, height) = self.output_surface.size();
            self.output_presenter.draw(
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
    }
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
        _ => {}
    }
    *effects = effects.sanitized();
}

fn monitor_id(monitor: &MonitorHandle) -> String {
    let name = monitor.name().unwrap_or_else(|| "Display".to_owned());
    let size = monitor.size();
    let position = monitor.position();
    format!(
        "{name}|{}x{}|{},{}",
        size.width, size.height, position.x, position.y
    )
}

fn monitor_label(monitor: &MonitorHandle) -> String {
    let name = monitor.name().unwrap_or_else(|| "Display".to_owned());
    let size = monitor.size();
    let refresh = monitor
        .refresh_rate_millihertz()
        .map(|millihertz| format!(" · {:.1} Hz", millihertz as f64 / 1000.0))
        .unwrap_or_default();
    format!("{name} · {} × {}{refresh}", size.width, size.height)
}

fn describe_monitors(handles: Vec<MonitorHandle>) -> (Vec<OutputMonitor>, Vec<ui::OutputDisplay>) {
    let mut monitors = Vec::with_capacity(handles.len());
    let mut displays = Vec::with_capacity(handles.len());
    for handle in handles {
        let id = monitor_id(&handle);
        displays.push(ui::OutputDisplay {
            id: id.clone(),
            label: monitor_label(&handle),
        });
        monitors.push(OutputMonitor { id, handle });
    }
    monitors.sort_by(|left, right| left.id.cmp(&right.id));
    displays.sort_by(|left, right| left.id.cmp(&right.id));
    (monitors, displays)
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
    fn counts_surface_failures_and_the_next_healthy_recovery() {
        let mut health = OutputHealth::default();
        health.observe(SurfaceAcquireStatus::Lost);
        health.observe(SurfaceAcquireStatus::Timeout);
        assert_eq!(health.skipped, 2);
        assert_eq!(health.reconfigurations, 1);
        assert_eq!(health.timeouts, 1);
        assert_eq!(health.recoveries, 0);

        health.observe(SurfaceAcquireStatus::Healthy);
        assert_eq!(health.presented, 1);
        assert_eq!(health.recoveries, 1);
        assert_eq!(health.status, "Healthy");

        health.observe(SurfaceAcquireStatus::Healthy);
        assert_eq!(health.recoveries, 1);
    }

    #[test]
    fn suboptimal_frames_are_presented_and_reconfigured() {
        let mut health = OutputHealth::default();
        health.observe(SurfaceAcquireStatus::Suboptimal);
        assert_eq!(health.presented, 1);
        assert_eq!(health.skipped, 0);
        assert_eq!(health.reconfigurations, 1);
        assert!(health.awaiting_recovery);
    }
}
