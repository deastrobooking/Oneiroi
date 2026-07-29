//! Milestone 1: wgpu + winit + egui on this machine, proving the stack works
//! before any media code exists.

mod project;
mod ui;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use oneiroi_core::{Clock, MediaTime, MidiMapper, TapTempo, TempoClock};
use oneiroi_io::{
    ProjectFile, autosave_path, load_project, recovery_is_newer, save_project_atomic,
};
use oneiroi_media::{
    CameraConfig, CameraDevice, ClipAddress, ClipBank, ClipRestoreRequest, ClipRestorer,
    CrossfadeBus, DeckDecoder, DeckId, DeckState, DeckTransport, DecoderEvent, DiscontinuityPolicy,
    FourDeckMixer, FrameScheduler, FrameSelection, LaunchQueue, MediaImporter, SubmitError,
    ThumbnailRequest, ThumbnailWorker, TransportEvent, VideoFramePayload, crossfade_gains,
    discover_cameras,
};
use oneiroi_render::{
    FourDeckCompositor, Gpu, MixerBus, MixerParams, PROGRAM_FORMAT, PresentSurface,
    PresentationOptions, ProgramPresenter, ProgramTarget, SurfaceAcquireStatus,
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
    thumbnails: ThumbnailWorker,
    thumbnail_request_id: u64,
    thumbnail_requests: HashMap<ClipAddress, (u64, PathBuf)>,
    cameras: Vec<CameraDevice>,
    camera_status: String,
    live_configs: [Option<CameraConfig>; 4],
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
                        state.import_movie(path);
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
            WindowEvent::DroppedFile(path) => state.import_movie(path),
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
        let mut ui = ui::UiState::default();
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
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
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
            thumbnails: ThumbnailWorker::new(32),
            thumbnail_request_id: 0,
            thumbnail_requests: HashMap::new(),
            cameras,
            camera_status,
            live_configs: std::array::from_fn(|_| None),
        })
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
        let path = movie.path.clone();
        let decode_path = movie.decode_path;
        let generation = self.mixer.activate(address.deck, movie);
        self.live_configs[address.deck.index()] = None;
        self.clips.activate(address);
        self.reset_playback(address.deck, generation);
        self.decoders[address.deck.index()].load(path, decode_path, generation);
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
        self.project_epoch = self.project_epoch.wrapping_add(1);
        self.clips = ClipBank::default();
        self.ui.clear_thumbnails();
        self.thumbnail_requests.clear();
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
                let Some(path) = path.clone() else {
                    continue;
                };
                let address = ClipAddress { deck, slot };
                self.clips.begin_restore(address, path.clone());
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
            match result.metadata {
                Ok(movie) => {
                    let address = result.address;
                    let duration = movie.duration.map(MediaTime::as_seconds);
                    self.clips.restore(address, movie);
                    self.request_thumbnail(address, result.path.clone());
                    if self.restore_active[address.deck.index()] == Some(address.slot) {
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
                }
                Err(error) => {
                    self.clips
                        .fail_restore(result.address, result.path, error.to_string());
                }
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
        self.decoders[index].load_at(path, decode_path, epoch, target);
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

    fn render(&mut self) {
        self.poll_imports();
        self.poll_restores();
        self.poll_thumbnails();
        let now = Instant::now();
        if now.saturating_duration_since(self.last_display_refresh) >= Duration::from_secs(2) {
            self.refresh_output_displays();
        }
        self.maybe_autosave(now);
        self.process_launches(now);
        self.update_playback(now);
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
                    frame_time: &time,
                    gpu_info: &self.gpu_info,
                    project_dirty,
                    project_status: &self.project_status,
                    recovery_available: self.recovery_path.is_some(),
                    cameras: &self.cameras,
                    camera_status: &self.camera_status,
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
                    self.clips.clear(address);
                    self.ui.clear_thumbnail(address);
                    self.thumbnail_requests.remove(&address);
                }
                ui::UiAction::Eject(deck) => {
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
                ui::UiAction::RefreshDisplays => self.refresh_output_displays(),
                ui::UiAction::RecoverProject => {
                    if let Some(path) = self.recovery_path.clone() {
                        self.open_project(path, true);
                    }
                }
                ui::UiAction::RefreshCameras => self.refresh_cameras(),
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

        self.compositor.draw(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &self.program.view,
            MixerParams {
                levels: std::array::from_fn(|index| {
                    let deck = self.mixer.deck(DeckId::ALL[index]);
                    if matches!(deck.state, DeckState::Ready(_) | DeckState::Live(_)) {
                        deck.level
                    } else {
                        0.0
                    }
                }),
                buses: std::array::from_fn(|index| match self.mixer.deck(DeckId::ALL[index]).bus {
                    CrossfadeBus::Left => MixerBus::A,
                    CrossfadeBus::Right => MixerBus::B,
                }),
                crossfade_gains: crossfade_gains(self.ui.crossfader, self.ui.equal_power),
                transforms: self.ui.transforms,
                blend_modes: self.ui.blend_modes,
                output_aspect: self.ui.composition_extent[0] as f32
                    / self.ui.composition_extent[1].max(1) as f32,
                effects: std::array::from_fn(|index| {
                    self.ui.lfos[index].apply(self.ui.effects[index], effect_time, beat_position)
                }),
                master_opacity: self.ui.master_opacity,
                time_seconds: effect_time,
                blackout: self.ui.blackout,
            },
        );

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
