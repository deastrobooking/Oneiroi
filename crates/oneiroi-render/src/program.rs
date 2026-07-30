//! Offscreen program target and presentation pass shared by operator/output windows.

use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use bytemuck::{Pod, Zeroable};

use crate::{EffectManifest, ValidatedEffectPackage, load_effect_package};

pub const PROGRAM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const MASTER_EFFECT_SLOTS: usize = 2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PresentGlobals {
    content_scale: [f32; 2],
    test_card: u32,
    identify: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresentationOptions {
    pub test_card: bool,
    pub identify: bool,
}

pub struct ProgramTarget {
    _composition_texture: wgpu::Texture,
    composition_view: wgpu::TextureView,
    _scratch_a_texture: wgpu::Texture,
    scratch_a_view: wgpu::TextureView,
    _ping_a_texture: wgpu::Texture,
    ping_a_view: wgpu::TextureView,
    history_texture: wgpu::Texture,
    history_view: wgpu::TextureView,
    _texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    extent: [u32; 2],
}

impl ProgramTarget {
    pub fn new(device: &wgpu::Device, extent: [u32; 2]) -> Self {
        let extent = [extent[0].max(1), extent[1].max(1)];
        let make_texture = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: extent[0],
                    height: extent[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: PROGRAM_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let composition_texture = make_texture("oneiroi-program-composition");
        let composition_view =
            composition_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let scratch_a_texture = make_texture("oneiroi-master-fx-scratch-a");
        let scratch_a_view = scratch_a_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let ping_a_texture = make_texture("oneiroi-master-fx-ping-a");
        let ping_a_view = ping_a_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let history_texture = make_texture("oneiroi-master-fx-history");
        let history_view = history_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let texture = make_texture("oneiroi-program-target");
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _composition_texture: composition_texture,
            composition_view,
            _scratch_a_texture: scratch_a_texture,
            scratch_a_view,
            _ping_a_texture: ping_a_texture,
            ping_a_view,
            history_texture,
            history_view,
            _texture: texture,
            view,
            extent,
        }
    }

    pub fn extent(&self) -> [u32; 2] {
        self.extent
    }

    pub fn composition_view(&self) -> &wgpu::TextureView {
        &self.composition_view
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MasterEffectKind {
    #[default]
    None,
    Blur,
    Feedback,
}

impl MasterEffectKind {
    pub const ALL: [Self; 3] = [Self::None, Self::Blur, Self::Feedback];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "Empty",
            Self::Blur => "Separable blur",
            Self::Feedback => "Feedback / trails",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasterEffectSlot {
    pub kind: MasterEffectKind,
    pub bypassed: bool,
    pub mix: f32,
    pub amount: f32,
    pub feedback: f32,
}

impl Default for MasterEffectSlot {
    fn default() -> Self {
        Self {
            kind: MasterEffectKind::None,
            bypassed: false,
            mix: 1.0,
            amount: 8.0,
            feedback: 0.85,
        }
    }
}

impl MasterEffectSlot {
    pub fn sanitized(mut self) -> Self {
        self.mix = finite_clamp(self.mix, 0.0, 1.0, 1.0);
        self.amount = finite_clamp(self.amount, 0.0, 32.0, 8.0);
        self.feedback = finite_clamp(self.feedback, 0.0, 0.99, 0.85);
        self
    }

    fn active(self) -> bool {
        if self.bypassed || self.mix <= 0.0001 {
            return false;
        }
        match self.kind {
            MasterEffectKind::None => false,
            MasterEffectKind::Blur => self.amount > 0.0001,
            MasterEffectKind::Feedback => self.feedback > 0.0001,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MasterEffectChain {
    pub slots: [MasterEffectSlot; MASTER_EFFECT_SLOTS],
}

impl MasterEffectChain {
    pub fn sanitized(mut self) -> Self {
        self.slots = self.slots.map(MasterEffectSlot::sanitized);
        self
    }

    pub fn active(self) -> bool {
        self.slots.into_iter().any(MasterEffectSlot::active)
    }
}

fn finite_clamp(value: f32, minimum: f32, maximum: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(minimum, maximum)
    } else {
        fallback
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MasterEffectGlobals {
    direction: [f32; 2],
    texel_size: [f32; 2],
    radius: f32,
    mix: f32,
    mode: u32,
    feedback: f32,
}

struct MasterEffectPass {
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
}

enum EffectReloadCommand {
    Watch(PathBuf),
    Reload,
    Shutdown,
}

struct CompiledEffectPipeline {
    pipeline: wgpu::RenderPipeline,
    name: String,
    fingerprint: u64,
}

struct EffectReloadResult {
    path: PathBuf,
    result: Result<CompiledEffectPipeline, String>,
}

struct EffectReloadWorker {
    commands: Sender<EffectReloadCommand>,
    results: Receiver<EffectReloadResult>,
    thread: Option<JoinHandle<()>>,
}

impl EffectReloadWorker {
    fn new(device: wgpu::Device, layout: wgpu::PipelineLayout) -> Self {
        let (commands_tx, commands_rx) = mpsc::channel();
        let (results_tx, results_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("oneiroi-effect-reload".to_owned())
            .spawn(move || effect_reload_loop(device, layout, commands_rx, results_tx))
            .expect("spawn effect reload worker");
        Self {
            commands: commands_tx,
            results: results_rx,
            thread: Some(thread),
        }
    }
}

impl Drop for EffectReloadWorker {
    fn drop(&mut self) {
        let _ = self.commands.send(EffectReloadCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct MasterEffectProcessor {
    pipeline: wgpu::RenderPipeline,
    passes: [MasterEffectPass; 4],
    extent: [u32; 2],
    history_valid: bool,
    reload_worker: EffectReloadWorker,
    reload_status: String,
}

impl MasterEffectProcessor {
    pub fn new(device: &wgpu::Device, program: &ProgramTarget) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oneiroi-master-effects"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/master_effects.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oneiroi-master-effects-layout"),
            entries: &[
                sampler_layout_entry(0),
                texture_layout_entry(1),
                texture_layout_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_layout_entry(4),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("oneiroi-master-effects-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = create_master_effect_pipeline(
            device,
            &pipeline_layout,
            shader,
            "vs_main",
            "fs_main",
            "oneiroi-master-effects-pipeline",
        );
        let reload_worker = EffectReloadWorker::new(device.clone(), pipeline_layout);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("oneiroi-master-effects-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let make_pass = |label, original: &wgpu::TextureView, effect: &wgpu::TextureView| {
            let globals = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size_of::<MasterEffectGlobals>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    texture_entry(1, original),
                    texture_entry(2, effect),
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: globals.as_entire_binding(),
                    },
                    texture_entry(4, &program.history_view),
                ],
            });
            MasterEffectPass {
                bind_group,
                globals,
            }
        };
        let passes = [
            make_pass(
                "oneiroi-master-slot-1-horizontal",
                &program.composition_view,
                &program.composition_view,
            ),
            make_pass(
                "oneiroi-master-slot-1-vertical",
                &program.composition_view,
                &program.scratch_a_view,
            ),
            make_pass(
                "oneiroi-master-slot-2-horizontal",
                &program.ping_a_view,
                &program.ping_a_view,
            ),
            make_pass(
                "oneiroi-master-slot-2-vertical",
                &program.ping_a_view,
                &program.scratch_a_view,
            ),
        ];
        Self {
            pipeline,
            passes,
            extent: program.extent,
            history_valid: false,
            reload_worker,
            reload_status: "Built-in master effect pipeline".to_owned(),
        }
    }

    pub fn watch_effect_manifest(&mut self, path: PathBuf) {
        self.reload_status = format!("Watching {}", path.display());
        let _ = self
            .reload_worker
            .commands
            .send(EffectReloadCommand::Watch(path));
    }

    pub fn reload_effect_manifest(&mut self) {
        self.reload_status = "Effect reload requested…".to_owned();
        let _ = self
            .reload_worker
            .commands
            .send(EffectReloadCommand::Reload);
    }

    pub fn poll_effect_reload(&mut self) -> bool {
        let mut changed = false;
        while let Ok(result) = self.reload_worker.results.try_recv() {
            changed = true;
            match result.result {
                Ok(compiled) => {
                    self.pipeline = compiled.pipeline;
                    self.reload_status =
                        format!("Loaded {} · {:016x}", compiled.name, compiled.fingerprint);
                }
                Err(error) => {
                    self.reload_status = format!(
                        "Reload rejected for {} · {error} · using last known good",
                        result.path.display()
                    );
                }
            }
        }
        changed
    }

    pub fn reload_status(&self) -> &str {
        &self.reload_status
    }

    pub fn reset_history(&mut self) {
        self.history_valid = false;
    }

    pub fn history_is_valid(&self) -> bool {
        self.history_valid
    }

    pub fn draw(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        program: &ProgramTarget,
        chain: MasterEffectChain,
    ) {
        let chain = chain.sanitized();
        let feedback_active = chain
            .slots
            .into_iter()
            .any(|slot| slot.active() && slot.kind == MasterEffectKind::Feedback);
        if !feedback_active {
            self.history_valid = false;
        }
        let use_history = feedback_active && self.history_valid;
        let targets = [
            (&program.scratch_a_view, &program.ping_a_view),
            (&program.scratch_a_view, &program.view),
        ];
        for (index, slot) in chain.slots.into_iter().enumerate() {
            let horizontal = index * 2;
            let vertical = horizontal + 1;
            match slot.kind {
                MasterEffectKind::Blur if slot.active() => {
                    self.draw_pass(
                        queue,
                        encoder,
                        horizontal,
                        targets[index].0,
                        self.globals([1.0, 0.0], slot.amount, 1.0, 0, 0.0),
                    );
                    self.draw_pass(
                        queue,
                        encoder,
                        vertical,
                        targets[index].1,
                        self.globals([0.0, 1.0], slot.amount, slot.mix, 0, 0.0),
                    );
                }
                MasterEffectKind::Feedback if slot.active() && use_history => {
                    self.draw_pass(
                        queue,
                        encoder,
                        vertical,
                        targets[index].1,
                        self.globals([0.0, 0.0], 0.0, slot.mix, 1, slot.feedback),
                    );
                }
                MasterEffectKind::None | MasterEffectKind::Blur | MasterEffectKind::Feedback => {
                    self.draw_pass(
                        queue,
                        encoder,
                        vertical,
                        targets[index].1,
                        self.globals([0.0, 0.0], 0.0, 0.0, 0, 0.0),
                    );
                }
            }
        }
        if feedback_active {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &program._texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &program.history_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: self.extent[0],
                    height: self.extent[1],
                    depth_or_array_layers: 1,
                },
            );
            self.history_valid = true;
        }
    }

    fn globals(
        &self,
        direction: [f32; 2],
        radius: f32,
        mix: f32,
        mode: u32,
        feedback: f32,
    ) -> MasterEffectGlobals {
        MasterEffectGlobals {
            direction,
            texel_size: [
                1.0 / self.extent[0].max(1) as f32,
                1.0 / self.extent[1].max(1) as f32,
            ],
            radius,
            mix,
            mode,
            feedback,
        }
    }

    fn draw_pass(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass_index: usize,
        target: &wgpu::TextureView,
        globals: MasterEffectGlobals,
    ) {
        let pass_state = &self.passes[pass_index];
        queue.write_buffer(&pass_state.globals, 0, bytemuck::bytes_of(&globals));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("oneiroi-master-effect-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &pass_state.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn effect_reload_loop(
    device: wgpu::Device,
    layout: wgpu::PipelineLayout,
    commands: Receiver<EffectReloadCommand>,
    results: Sender<EffectReloadResult>,
) {
    let mut watched = None;
    let mut last_fingerprint = None;
    let mut force_reload = false;
    loop {
        match commands.recv_timeout(Duration::from_millis(500)) {
            Ok(EffectReloadCommand::Watch(path)) => {
                watched = Some(path);
                last_fingerprint = None;
                force_reload = true;
            }
            Ok(EffectReloadCommand::Reload) => force_reload = true,
            Ok(EffectReloadCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let Some(path) = watched.as_deref() else {
            continue;
        };
        let fingerprint = unchecked_package_fingerprint(path);
        if !force_reload && last_fingerprint == Some(fingerprint) {
            continue;
        }
        force_reload = false;
        last_fingerprint = Some(fingerprint);
        let result = compile_effect_package(&device, &layout, path);
        let _ = results.send(EffectReloadResult {
            path: path.to_path_buf(),
            result,
        });
    }
}

fn compile_effect_package(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    manifest_path: &Path,
) -> Result<CompiledEffectPipeline, String> {
    let package = load_effect_package(manifest_path).map_err(|error| error.to_string())?;
    compile_validated_effect_package(device, layout, package)
}

fn compile_validated_effect_package(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    package: ValidatedEffectPackage,
) -> Result<CompiledEffectPipeline, String> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&package.manifest.name),
        source: wgpu::ShaderSource::Wgsl(package.shader_source.clone().into()),
    });
    let pipeline = create_master_effect_pipeline(
        device,
        layout,
        shader,
        &package.manifest.vertex_entry,
        &package.manifest.fragment_entry,
        &package.manifest.name,
    );
    if let Some(error) = pollster::block_on(scope.pop()) {
        return Err(format!("GPU pipeline validation failed: {error}"));
    }
    Ok(CompiledEffectPipeline {
        pipeline,
        name: package.manifest.name,
        fingerprint: package.fingerprint,
    })
}

fn create_master_effect_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    vertex_entry: &str,
    fragment_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(vertex_entry),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: PROGRAM_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn unchecked_package_fingerprint(manifest_path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    manifest_path.hash(&mut hasher);
    let manifest_source = fs::read(manifest_path);
    manifest_source
        .as_ref()
        .map_or(0, Vec::len)
        .hash(&mut hasher);
    if let Ok(source) = &manifest_source {
        source.hash(&mut hasher);
        if let Ok(manifest) = serde_json::from_slice::<EffectManifest>(source)
            && manifest
                .shader
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            let shader_path = manifest_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(manifest.shader);
            if let Ok(shader) = fs::read(shader_path) {
                shader.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

fn sampler_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

pub struct ProgramPresenter {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    program_extent: [u32; 2],
}

impl ProgramPresenter {
    pub fn new(
        device: &wgpu::Device,
        program: &ProgramTarget,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oneiroi-program-presenter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/present.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oneiroi-program-presenter-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("oneiroi-program-presenter-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("oneiroi-program-presenter-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("oneiroi-program-presenter-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oneiroi-program-presenter-globals"),
            size: size_of::<PresentGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oneiroi-program-presenter-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&program.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: globals.as_entire_binding(),
                },
            ],
        });
        Self {
            pipeline,
            bind_group,
            globals,
            program_extent: program.extent(),
        }
    }

    pub fn draw(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        target_extent: [u32; 2],
        options: PresentationOptions,
    ) {
        let program_aspect = self.program_extent[0] as f32 / self.program_extent[1].max(1) as f32;
        let target_aspect = target_extent[0] as f32 / target_extent[1].max(1) as f32;
        let content_scale = if target_aspect > program_aspect {
            [program_aspect / target_aspect, 1.0]
        } else {
            [1.0, target_aspect / program_aspect]
        };
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&PresentGlobals {
                content_scale,
                test_card: u32::from(options.test_card),
                identify: u32::from(options.identify),
            }),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("oneiroi-program-present-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_effect_shader_parses_and_validates_without_a_gpu_adapter() {
        let module = naga::front::wgsl::parse_str(include_str!("../shaders/master_effects.wgsl"))
            .expect("parse master effect shader");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("validate master effect shader");
    }

    #[test]
    fn master_effect_activity_and_values_are_sanitized() {
        let mut chain = MasterEffectChain::default();
        assert!(!chain.active());
        chain.slots[0] = MasterEffectSlot {
            kind: MasterEffectKind::Blur,
            bypassed: false,
            mix: 0.75,
            amount: 12.0,
            feedback: 0.85,
        };
        assert!(chain.active());
        chain.slots[0].bypassed = true;
        assert!(!chain.active());

        chain.slots[0].mix = f32::NAN;
        chain.slots[0].amount = 100.0;
        chain.slots[0].feedback = 2.0;
        let mut sanitized = chain.sanitized();
        assert_eq!(sanitized.slots[0].mix, 1.0);
        assert_eq!(sanitized.slots[0].amount, 32.0);
        assert_eq!(sanitized.slots[0].feedback, 0.99);

        sanitized.slots[0].kind = MasterEffectKind::Feedback;
        sanitized.slots[0].bypassed = false;
        assert!(sanitized.active());
    }
}
