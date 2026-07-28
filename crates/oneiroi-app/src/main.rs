//! Milestone 1: wgpu + winit + egui on this machine, proving the stack works
//! before any media code exists.

mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use oneiroi_core::{Clock, MediaTime};
use oneiroi_media::{
    DeckDecoder, DeckId, DeckState, DecoderEvent, DiscontinuityPolicy, FourDeckMixer,
    FrameScheduler, FrameSelection, MediaImporter, SubmitError, VideoFramePayload,
};
use oneiroi_render::{FourDeckCompositor, Gpu, MixerParams};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
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
    gpu: Gpu,
    compositor: FourDeckCompositor,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    clock: Clock,
    ui: ui::UiState,
    gpu_info: String,
    mixer: FourDeckMixer,
    importer: MediaImporter,
    decoders: [DeckDecoder; 4],
    schedulers: [FrameScheduler<VideoFramePayload>; 4],
    launch_times: [Instant; 4],
    media_origins: [Option<MediaTime>; 4],
    playback_generations: [u64; 4],
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
                for path in self.initial_files.drain(..) {
                    state.import_movie(path);
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
        if state.window.id() != id {
            return;
        }

        // egui sees every event first so it can claim clicks and keys that
        // land on the overlay.
        let response = state.egui_state.on_window_event(&state.window, &event);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.gpu.resize(size.width, size.height),
            WindowEvent::DroppedFile(path) => state.import_movie(path),
            WindowEvent::RedrawRequested => state.render(),
            _ => {}
        }

        if response.repaint {
            state.window.request_redraw();
        }
    }
}

impl State {
    fn new(event_loop: &ActiveEventLoop) -> Result<Self> {
        let attrs = Window::default_attributes()
            .with_title("oneiroi")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .context("create main window")?,
        );

        let size = window.inner_size();
        // Blocking here is fine: it happens once, before the loop is running.
        let gpu = pollster::block_on(Gpu::new(window.clone(), size.width, size.height))?;

        let info = gpu.adapter_info();
        let bc_support = if gpu.supports_bc_textures() {
            "BC textures"
        } else {
            "no BC textures"
        };
        let gpu_info = format!("{} · {:?} · {bc_support}", info.name, info.backend);

        let compositor = FourDeckCompositor::new(&gpu.device, &gpu.queue, gpu.content_format());

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

        Ok(Self {
            window,
            gpu,
            compositor,
            egui_state,
            egui_renderer,
            clock: Clock::new(Instant::now()),
            ui: ui::UiState::default(),
            gpu_info,
            mixer: FourDeckMixer::default(),
            importer: MediaImporter::new(8),
            decoders: std::array::from_fn(|_| DeckDecoder::spawn(4)),
            schedulers: std::array::from_fn(|_| {
                FrameScheduler::new(4, 0, DiscontinuityPolicy::Blank).expect("non-zero frame queue")
            }),
            launch_times: [Instant::now(); 4],
            media_origins: [None; 4],
            playback_generations: [0; 4],
        })
    }

    fn import_movie(&mut self, path: PathBuf) {
        let deck = self.mixer.selected();
        let request = self.mixer.begin_import(deck, path);
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
                )
            });
            if self.mixer.complete_import(result)
                && let Some((deck, generation, path, decode_path)) = playback
            {
                self.reset_playback(deck, generation);
                self.decoders[deck.index()].load(path, decode_path, generation);
            }
        }
    }

    fn reset_playback(&mut self, deck: DeckId, generation: u64) {
        let index = deck.index();
        self.decoders[index].stop();
        self.compositor.clear_deck(index);
        self.schedulers[index] = FrameScheduler::new(4, generation, DiscontinuityPolicy::Blank)
            .expect("non-zero frame queue");
        self.launch_times[index] = Instant::now();
        self.media_origins[index] = None;
        self.playback_generations[index] = generation;
    }

    fn update_playback(&mut self, now: Instant) {
        for deck in DeckId::ALL {
            let index = deck.index();
            let generation = self.mixer.deck(deck).generation;
            if generation != self.playback_generations[index] {
                self.reset_playback(deck, generation);
            }
            while let Ok(event) = self.decoders[index].try_event() {
                if let DecoderEvent::Error {
                    generation: failed_generation,
                    message,
                } = event
                    && failed_generation == generation
                {
                    let path = match &self.mixer.deck(deck).state {
                        DeckState::Ready(movie) => movie.path.clone(),
                        DeckState::Loading { path } | DeckState::Error { path, .. } => path.clone(),
                        DeckState::Empty => PathBuf::new(),
                    };
                    self.mixer.deck_mut(deck).state = DeckState::Error { path, message };
                    self.compositor.clear_deck(index);
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
            let elapsed = now
                .saturating_duration_since(self.launch_times[index])
                .as_micros()
                .min(i64::MAX as u128) as i64;
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
        let now = Instant::now();
        self.update_playback(now);
        let time = self.clock.tick(now);

        // --- UI pass: pure CPU, produces geometry for the GPU pass below.
        let ctx = self.egui_state.egui_ctx().clone();
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let output = ctx.run_ui(raw_input, |ui| {
            ui::draw(
                ui.ctx(),
                &mut self.ui,
                &mut self.mixer,
                &time,
                &self.gpu_info,
            );
        });
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

        // --- GPU pass.
        let Some(frame) = self.gpu.acquire() else {
            // Surface was stale and has been reconfigured; skip this frame and
            // keep the loop alive.
            self.window.request_redraw();
            return;
        };
        let content_view = self.gpu.content_view(&frame.texture);
        let ui_view = self.gpu.surface_view(&frame.texture);
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
            &content_view,
            MixerParams {
                levels: std::array::from_fn(|index| {
                    let deck = self.mixer.deck(DeckId::ALL[index]);
                    if matches!(deck.state, DeckState::Ready(_)) {
                        deck.level
                    } else {
                        0.0
                    }
                }),
                master_opacity: self.ui.master_opacity,
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

        {
            // Load, don't clear: the overlay composites over the triangle.
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

        self.gpu
            .queue
            .submit(upload_cmds.into_iter().chain([encoder.finish()]));
        frame.present();

        // Continuous redraw. Presentation is Fifo, so this paces to vsync
        // rather than spinning.
        self.window.request_redraw();
    }
}
