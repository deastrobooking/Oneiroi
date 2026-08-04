//! Offscreen program target and presentation pass shared by operator/output windows.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use bytemuck::{Pod, Zeroable};
use oneiroi_core::effect_parameter_key;

use crate::{
    EffectHistoryResource, EffectManifest, EffectPackageAbi, EffectPackageRole,
    EffectPackageTarget, EffectParameterSchema, ValidatedEffectPackage, load_effect_package,
    mixer::LfoWaveform,
};

pub const PROGRAM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const MASTER_EFFECT_SLOTS: usize = 2;
pub const EFFECT_PARAMETER_CAPACITY: usize = 32;
pub const MASTER_MODULATION_ROUTES: usize = 8;
pub const MASTER_MODULATION_SOURCES: usize = 10;

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
    custom_history_textures: [wgpu::Texture; MASTER_EFFECT_SLOTS],
    custom_history_views: [wgpu::TextureView; MASTER_EFFECT_SLOTS],
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
        let custom_history_textures = [
            make_texture("oneiroi-custom-fx-slot-1-history"),
            make_texture("oneiroi-custom-fx-slot-2-history"),
        ];
        let custom_history_views = custom_history_textures
            .each_ref()
            .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));
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
            custom_history_textures,
            custom_history_views,
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

    fn slot_output_texture(&self, slot: usize) -> &wgpu::Texture {
        if slot == 0 {
            &self._ping_a_texture
        } else {
            &self._texture
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MasterEffectKind {
    #[default]
    None,
    Blur,
    Feedback,
    Custom,
}

impl MasterEffectKind {
    pub const ALL: [Self; 4] = [Self::None, Self::Blur, Self::Feedback, Self::Custom];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "Empty",
            Self::Blur => "Separable blur",
            Self::Feedback => "Feedback / trails",
            Self::Custom => "Custom package",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectParameterValue {
    pub id: String,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MasterEffectSlot {
    pub kind: MasterEffectKind,
    pub bypassed: bool,
    pub mix: f32,
    pub amount: f32,
    pub feedback: f32,
    pub package_id: String,
    pub parameters: Vec<EffectParameterValue>,
}

impl Default for MasterEffectSlot {
    fn default() -> Self {
        Self {
            kind: MasterEffectKind::None,
            bypassed: false,
            mix: 1.0,
            amount: 8.0,
            feedback: 0.85,
            package_id: String::new(),
            parameters: Vec::new(),
        }
    }
}

impl MasterEffectSlot {
    pub fn sanitized(mut self) -> Self {
        self.sanitize();
        self
    }

    pub fn sanitize(&mut self) {
        self.mix = finite_clamp(self.mix, 0.0, 1.0, 1.0);
        self.amount = finite_clamp(self.amount, 0.0, 32.0, 8.0);
        self.feedback = finite_clamp(self.feedback, 0.0, 0.99, 0.85);
        self.parameters.retain(|parameter| {
            !parameter.id.is_empty() && parameter.id.len() <= 64 && parameter.value.is_finite()
        });
        self.parameters.truncate(EFFECT_PARAMETER_CAPACITY);
    }

    fn active(&self) -> bool {
        if self.bypassed || self.mix <= 0.0001 {
            return false;
        }
        match self.kind {
            MasterEffectKind::None => false,
            MasterEffectKind::Blur => self.amount > 0.0001,
            MasterEffectKind::Feedback => self.feedback > 0.0001,
            MasterEffectKind::Custom => !self.package_id.is_empty(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterEffectChain {
    pub slots: [MasterEffectSlot; MASTER_EFFECT_SLOTS],
}

impl MasterEffectChain {
    pub fn sanitized(mut self) -> Self {
        self.sanitize();
        self
    }

    pub fn sanitize(&mut self) {
        for slot in &mut self.slots {
            slot.sanitize();
        }
    }

    pub fn active(&self) -> bool {
        self.slots.iter().any(MasterEffectSlot::active)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasterLfo {
    pub enabled: bool,
    pub waveform: LfoWaveform,
    pub rate_hz: f32,
    pub tempo_sync: bool,
    pub beats_per_cycle: f32,
    pub depth: f32,
    pub phase: f32,
}

impl Default for MasterLfo {
    fn default() -> Self {
        Self {
            enabled: false,
            waveform: LfoWaveform::Sine,
            rate_hz: 0.25,
            tempo_sync: false,
            beats_per_cycle: 1.0,
            depth: 0.5,
            phase: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasterModulationRoute {
    pub enabled: bool,
    pub source: u8,
    pub target_slot: u8,
    pub parameter_key: u64,
    pub amount: f32,
}

impl Default for MasterModulationRoute {
    fn default() -> Self {
        Self {
            enabled: false,
            source: 0,
            target_slot: 0,
            parameter_key: 0,
            amount: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MasterModulation {
    pub lfos: [MasterLfo; 3],
    pub routes: [MasterModulationRoute; MASTER_MODULATION_ROUTES],
}

impl MasterModulation {
    fn source_values(
        self,
        time_seconds: f32,
        beat_position: f32,
        audio: [f32; 5],
    ) -> [f32; MASTER_MODULATION_SOURCES] {
        let mut sources = [0.0; MASTER_MODULATION_SOURCES];
        for (index, lfo) in self.lfos.into_iter().enumerate() {
            if !lfo.enabled {
                continue;
            }
            let cycle = if lfo.tempo_sync {
                beat_position / lfo.beats_per_cycle.clamp(0.0625, 8.0)
            } else {
                time_seconds * lfo.rate_hz.clamp(0.01, 20.0)
            };
            sources[index] = lfo.waveform.sample(cycle + lfo.phase) * lfo.depth.clamp(0.0, 1.0);
        }
        sources[3..8].copy_from_slice(&audio.map(|value| value.clamp(0.0, 1.0)));
        sources[8] = beat_position.rem_euclid(1.0);
        sources[9] = (beat_position / 4.0).rem_euclid(1.0);
        sources
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
    time_seconds: f32,
    parameter_count: u32,
    pass_index: u32,
    pass_count: u32,
    parameters: [f32; EFFECT_PARAMETER_CAPACITY],
    history_valid: u32,
    deck_index: u32,
    source_extent: [u32; 2],
    composition_extent: [u32; 2],
    _resource_padding: [u32; 2],
}

#[derive(Clone, Copy)]
struct CustomPassContext<'a> {
    slot_index: usize,
    modulation: &'a MasterModulation,
    sources: [f32; MASTER_MODULATION_SOURCES],
    time_seconds: f32,
    pass_index: usize,
    pass_count: usize,
    history_valid: bool,
}

struct MasterEffectPass {
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
}

enum EffectReloadCommand {
    Watch {
        generation: u64,
        path: PathBuf,
    },
    WatchMany {
        generation: u64,
        paths: Vec<PathBuf>,
    },
    Reload {
        generation: u64,
    },
    Shutdown,
}

struct CompiledEffectPipeline {
    pipelines: Vec<wgpu::RenderPipeline>,
    id: String,
    name: String,
    role: EffectPackageRole,
    parameters: Vec<EffectParameterSchema>,
    history: EffectHistoryResource,
    fingerprint: u64,
}

struct EffectReloadResult {
    generation: u64,
    path: PathBuf,
    result: Result<CompiledEffectPipeline, EffectReloadFailure>,
}

struct EffectReloadFailure {
    message: String,
    /// A syntactically and semantically valid package that intentionally moved
    /// away from the master target must retire the old master pipeline at the
    /// same path. Invalid edits continue to use last-known-good.
    retire_custom_pipeline: bool,
}

struct EffectReloadDiagnostic {
    message: String,
    fallback: EffectReloadFallback,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum EffectReloadFallback {
    LastKnownGood,
    BuiltInProcessor,
    Neutral,
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
    built_in_pipeline: wgpu::RenderPipeline,
    pipeline: wgpu::RenderPipeline,
    processor_manifest_path: Option<PathBuf>,
    custom_pipelines: HashMap<String, RegisteredEffectPipeline>,
    watched_effect_paths: HashSet<PathBuf>,
    watch_generation: u64,
    passes: [MasterEffectPass; 4],
    extent: [u32; 2],
    history_valid: bool,
    custom_history_valid: [bool; MASTER_EFFECT_SLOTS],
    custom_history_identity: [u64; MASTER_EFFECT_SLOTS],
    reload_worker: EffectReloadWorker,
    reload_status: String,
    reload_errors: HashMap<PathBuf, EffectReloadDiagnostic>,
}

struct RegisteredEffectPipeline {
    manifest_path: PathBuf,
    pipelines: Vec<wgpu::RenderPipeline>,
    parameters: Vec<EffectParameterSchema>,
    history: EffectHistoryResource,
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
                texture_layout_entry(5),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("oneiroi-master-effects-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let built_in_pipeline = create_master_effect_pipeline(
            device,
            &pipeline_layout,
            &shader,
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
        let make_pass = |label,
                         original: &wgpu::TextureView,
                         effect: &wgpu::TextureView,
                         custom_history: &wgpu::TextureView| {
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
                    texture_entry(5, custom_history),
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
                &program.custom_history_views[0],
            ),
            make_pass(
                "oneiroi-master-slot-1-vertical",
                &program.composition_view,
                &program.scratch_a_view,
                &program.custom_history_views[0],
            ),
            make_pass(
                "oneiroi-master-slot-2-horizontal",
                &program.ping_a_view,
                &program.ping_a_view,
                &program.custom_history_views[1],
            ),
            make_pass(
                "oneiroi-master-slot-2-vertical",
                &program.ping_a_view,
                &program.scratch_a_view,
                &program.custom_history_views[1],
            ),
        ];
        Self {
            pipeline: built_in_pipeline.clone(),
            built_in_pipeline,
            processor_manifest_path: None,
            custom_pipelines: HashMap::new(),
            watched_effect_paths: HashSet::new(),
            watch_generation: 0,
            passes,
            extent: program.extent,
            history_valid: false,
            custom_history_valid: [false; MASTER_EFFECT_SLOTS],
            custom_history_identity: [0; MASTER_EFFECT_SLOTS],
            reload_worker,
            reload_status: "Built-in master effect pipeline".to_owned(),
            reload_errors: HashMap::new(),
        }
    }

    pub fn watch_effect_manifest(&mut self, path: PathBuf) {
        self.retain_watched_effects(std::slice::from_ref(&path));
        let generation = self.next_watch_generation();
        self.reload_status = format!("Watching {}", path.display());
        let _ = self
            .reload_worker
            .commands
            .send(EffectReloadCommand::Watch { generation, path });
    }

    pub fn watch_effect_manifests(&mut self, paths: Vec<PathBuf>) {
        self.retain_watched_effects(&paths);
        let generation = self.next_watch_generation();
        self.reload_status = format!("Watching {} effect package(s)", paths.len());
        let _ = self
            .reload_worker
            .commands
            .send(EffectReloadCommand::WatchMany { generation, paths });
    }

    fn next_watch_generation(&mut self) -> u64 {
        self.watch_generation = self.watch_generation.wrapping_add(1).max(1);
        self.watch_generation
    }

    fn retain_watched_effects(&mut self, paths: &[PathBuf]) {
        let watched: HashSet<_> = paths.iter().cloned().collect();
        let previous_count = self.custom_pipelines.len();
        self.custom_pipelines
            .retain(|_, effect| watched.contains(&effect.manifest_path));
        self.reload_errors.retain(|path, _| watched.contains(path));
        if self
            .processor_manifest_path
            .as_ref()
            .is_some_and(|path| !watched.contains(path))
        {
            self.pipeline = self.built_in_pipeline.clone();
            self.processor_manifest_path = None;
        }
        self.watched_effect_paths = watched;
        if self.custom_pipelines.len() != previous_count {
            self.custom_history_valid = [false; MASTER_EFFECT_SLOTS];
            self.custom_history_identity = [0; MASTER_EFFECT_SLOTS];
        }
    }

    pub fn reload_effect_manifest(&mut self) {
        let generation = self.next_watch_generation();
        self.reload_status = "Effect reload requested…".to_owned();
        let _ = self
            .reload_worker
            .commands
            .send(EffectReloadCommand::Reload { generation });
    }

    pub fn poll_effect_reload(&mut self) -> bool {
        let mut changed = false;
        let mut loaded = Vec::new();
        while let Ok(result) = self.reload_worker.results.try_recv() {
            if result.generation != self.watch_generation
                || !self.watched_effect_paths.contains(&result.path)
            {
                continue;
            }
            changed = true;
            let path = result.path;
            match result.result {
                Ok(compiled) => {
                    self.reload_errors.remove(&path);
                    if compiled.role == EffectPackageRole::MasterProcessor {
                        let previous_count = self.custom_pipelines.len();
                        self.custom_pipelines
                            .retain(|_, effect| effect.manifest_path != path);
                        let mut pipelines = compiled.pipelines;
                        self.pipeline = pipelines.remove(0);
                        self.processor_manifest_path = Some(path.clone());
                        if self.custom_pipelines.len() != previous_count {
                            self.custom_history_valid = [false; MASTER_EFFECT_SLOTS];
                            self.custom_history_identity = [0; MASTER_EFFECT_SLOTS];
                        }
                    } else {
                        if self.processor_manifest_path.as_ref() == Some(&path) {
                            self.pipeline = self.built_in_pipeline.clone();
                            self.processor_manifest_path = None;
                        }
                        self.custom_pipelines.retain(|id, effect| {
                            effect.manifest_path != path || id == &compiled.id
                        });
                        self.custom_pipelines.insert(
                            compiled.id.clone(),
                            RegisteredEffectPipeline {
                                manifest_path: path.clone(),
                                pipelines: compiled.pipelines,
                                parameters: compiled.parameters,
                                history: compiled.history,
                            },
                        );
                        self.custom_history_valid = [false; MASTER_EFFECT_SLOTS];
                        self.custom_history_identity = [0; MASTER_EFFECT_SLOTS];
                    }
                    loaded.push(format!("{} · {:016x}", compiled.name, compiled.fingerprint));
                }
                Err(error) => {
                    let had_custom_pipeline = self
                        .custom_pipelines
                        .values()
                        .any(|effect| effect.manifest_path == path);
                    let had_processor_pipeline =
                        self.processor_manifest_path.as_ref() == Some(&path);
                    let fallback = if !error.retire_custom_pipeline
                        && (had_custom_pipeline || had_processor_pipeline)
                    {
                        EffectReloadFallback::LastKnownGood
                    } else if had_processor_pipeline {
                        EffectReloadFallback::BuiltInProcessor
                    } else {
                        EffectReloadFallback::Neutral
                    };
                    if error.retire_custom_pipeline {
                        let previous_count = self.custom_pipelines.len();
                        self.custom_pipelines
                            .retain(|_, effect| effect.manifest_path != path);
                        if self.custom_pipelines.len() != previous_count {
                            self.custom_history_valid = [false; MASTER_EFFECT_SLOTS];
                            self.custom_history_identity = [0; MASTER_EFFECT_SLOTS];
                        }
                        if self.processor_manifest_path.as_ref() == Some(&path) {
                            self.pipeline = self.built_in_pipeline.clone();
                            self.processor_manifest_path = None;
                        }
                    }
                    self.reload_errors.insert(
                        path,
                        EffectReloadDiagnostic {
                            message: error.message,
                            fallback,
                        },
                    );
                }
            }
        }
        if changed && !self.reload_errors.is_empty() {
            let mut rejected: Vec<_> = self
                .reload_errors
                .iter()
                .map(|(path, error)| format!("{} · {}", path.display(), error.message))
                .collect();
            rejected.sort();
            let mut fallbacks = Vec::new();
            for (kind, label) in [
                (EffectReloadFallback::LastKnownGood, "last known good"),
                (EffectReloadFallback::BuiltInProcessor, "built-in processor"),
                (EffectReloadFallback::Neutral, "neutral fallback"),
            ] {
                if self
                    .reload_errors
                    .values()
                    .any(|error| error.fallback == kind)
                {
                    fallbacks.push(label);
                }
            }
            let fallback = if fallbacks.len() == 1 {
                format!("using {}", fallbacks[0])
            } else {
                format!("using {} as applicable", fallbacks.join(" / "))
            };
            self.reload_status =
                format!("Reload rejected · {} · {fallback}", rejected.join(" · "),);
        } else if changed && !loaded.is_empty() {
            self.reload_status = format!("Loaded {}", loaded.join(" · "));
        }
        changed
    }

    pub fn reload_status(&self) -> &str {
        &self.reload_status
    }

    pub fn custom_effect_loaded(&self, id: &str) -> bool {
        self.custom_pipelines.contains_key(id)
    }

    pub fn custom_effect_pass_count(&self, id: &str) -> Option<usize> {
        self.custom_pipelines
            .get(id)
            .map(|effect| effect.pipelines.len())
    }

    pub fn reset_history(&mut self) {
        self.history_valid = false;
        self.custom_history_valid = [false; MASTER_EFFECT_SLOTS];
        self.custom_history_identity = [0; MASTER_EFFECT_SLOTS];
    }

    pub fn history_is_valid(&self) -> bool {
        self.history_valid
    }

    pub fn custom_history_is_valid(&self, slot: usize) -> bool {
        self.custom_history_valid
            .get(slot)
            .copied()
            .unwrap_or(false)
    }

    pub fn draw(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        program: &ProgramTarget,
        chain: &MasterEffectChain,
    ) {
        self.draw_at(queue, encoder, program, chain, 0.0);
    }

    pub fn draw_at(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        program: &ProgramTarget,
        chain: &MasterEffectChain,
        time_seconds: f32,
    ) {
        self.draw_modulated_at(
            queue,
            encoder,
            program,
            chain,
            &MasterModulation::default(),
            time_seconds,
            0.0,
            [0.0; 5],
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_modulated_at(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        program: &ProgramTarget,
        chain: &MasterEffectChain,
        modulation: &MasterModulation,
        time_seconds: f32,
        beat_position: f32,
        audio: [f32; 5],
    ) {
        let modulation_sources = modulation.source_values(time_seconds, beat_position, audio);
        let feedback_active = chain
            .slots
            .iter()
            .any(|slot| slot.active() && slot.kind == MasterEffectKind::Feedback);
        if !feedback_active {
            self.history_valid = false;
        }
        let use_history = feedback_active && self.history_valid;
        let targets = [
            (&program.scratch_a_view, &program.ping_a_view),
            (&program.scratch_a_view, &program.view),
        ];
        for (index, slot) in chain.slots.iter().enumerate() {
            let horizontal = index * 2;
            let vertical = horizontal + 1;
            match slot.kind {
                MasterEffectKind::Blur if slot.active() => {
                    self.draw_pass(
                        queue,
                        encoder,
                        horizontal,
                        targets[index].0,
                        self.globals(
                            [1.0, 0.0],
                            finite_clamp(slot.amount, 0.0, 32.0, 8.0),
                            1.0,
                            0,
                            0.0,
                            time_seconds,
                        ),
                    );
                    self.draw_pass(
                        queue,
                        encoder,
                        vertical,
                        targets[index].1,
                        self.globals(
                            [0.0, 1.0],
                            finite_clamp(slot.amount, 0.0, 32.0, 8.0),
                            finite_clamp(slot.mix, 0.0, 1.0, 1.0),
                            0,
                            0.0,
                            time_seconds,
                        ),
                    );
                }
                MasterEffectKind::Feedback if slot.active() && use_history => {
                    self.draw_pass(
                        queue,
                        encoder,
                        vertical,
                        targets[index].1,
                        self.globals(
                            [0.0, 0.0],
                            0.0,
                            finite_clamp(slot.mix, 0.0, 1.0, 1.0),
                            1,
                            finite_clamp(slot.feedback, 0.0, 0.99, 0.85),
                            time_seconds,
                        ),
                    );
                }
                MasterEffectKind::Custom if slot.active() => {
                    if let Some(effect) = self.custom_pipelines.get(&slot.package_id) {
                        let uses_history =
                            effect.history == EffectHistoryResource::PreviousSlotOutput;
                        let history_identity =
                            effect_parameter_key(&slot.package_id, "previous-slot-output");
                        if self.custom_history_identity[index] != history_identity {
                            self.custom_history_valid[index] = false;
                            self.custom_history_identity[index] = history_identity;
                        }
                        let history_valid = uses_history && self.custom_history_valid[index];
                        let pass_count = effect.pipelines.len();
                        if pass_count == 2 {
                            let globals = self.custom_globals(
                                slot,
                                &effect.parameters,
                                CustomPassContext {
                                    slot_index: index,
                                    modulation,
                                    sources: modulation_sources,
                                    time_seconds,
                                    pass_index: 0,
                                    pass_count,
                                    history_valid,
                                },
                            );
                            self.draw_pass_with_pipeline(
                                queue,
                                encoder,
                                horizontal,
                                targets[index].0,
                                globals,
                                &effect.pipelines[0],
                            );
                        }
                        let final_pass = pass_count.saturating_sub(1);
                        let globals = self.custom_globals(
                            slot,
                            &effect.parameters,
                            CustomPassContext {
                                slot_index: index,
                                modulation,
                                sources: modulation_sources,
                                time_seconds,
                                pass_index: final_pass,
                                pass_count,
                                history_valid,
                            },
                        );
                        self.draw_pass_with_pipeline(
                            queue,
                            encoder,
                            vertical,
                            targets[index].1,
                            globals,
                            &effect.pipelines[final_pass],
                        );
                        if uses_history {
                            encoder.copy_texture_to_texture(
                                wgpu::TexelCopyTextureInfo {
                                    texture: program.slot_output_texture(index),
                                    mip_level: 0,
                                    origin: wgpu::Origin3d::ZERO,
                                    aspect: wgpu::TextureAspect::All,
                                },
                                wgpu::TexelCopyTextureInfo {
                                    texture: &program.custom_history_textures[index],
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
                            self.custom_history_valid[index] = true;
                        } else {
                            self.custom_history_valid[index] = false;
                            self.custom_history_identity[index] = 0;
                        }
                    } else {
                        self.custom_history_valid[index] = false;
                        self.custom_history_identity[index] = 0;
                        self.draw_pass(
                            queue,
                            encoder,
                            vertical,
                            targets[index].1,
                            self.globals([0.0, 0.0], 0.0, 0.0, 0, 0.0, time_seconds),
                        );
                    }
                }
                MasterEffectKind::None
                | MasterEffectKind::Blur
                | MasterEffectKind::Feedback
                | MasterEffectKind::Custom => {
                    self.custom_history_valid[index] = false;
                    self.custom_history_identity[index] = 0;
                    self.draw_pass(
                        queue,
                        encoder,
                        vertical,
                        targets[index].1,
                        self.globals([0.0, 0.0], 0.0, 0.0, 0, 0.0, time_seconds),
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
        time_seconds: f32,
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
            time_seconds,
            parameter_count: 0,
            pass_index: 0,
            pass_count: 1,
            parameters: [0.0; EFFECT_PARAMETER_CAPACITY],
            history_valid: 0,
            deck_index: u32::MAX,
            source_extent: self.extent,
            composition_extent: self.extent,
            _resource_padding: [0; 2],
        }
    }

    fn custom_globals(
        &self,
        slot: &MasterEffectSlot,
        schema: &[EffectParameterSchema],
        context: CustomPassContext<'_>,
    ) -> MasterEffectGlobals {
        let mut globals = self.globals(
            [0.0, 0.0],
            0.0,
            finite_clamp(slot.mix, 0.0, 1.0, 1.0),
            2,
            0.0,
            context.time_seconds,
        );
        globals.pass_index = context.pass_index as u32;
        globals.pass_count = context.pass_count as u32;
        globals.history_valid = u32::from(context.history_valid);
        for (index, parameter) in schema.iter().take(EFFECT_PARAMETER_CAPACITY).enumerate() {
            globals.parameters[index] = modulated_parameter_value(
                context.slot_index,
                slot,
                parameter,
                context.modulation,
                context.sources,
            );
            globals.parameter_count += 1;
        }
        globals
    }

    fn draw_pass(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass_index: usize,
        target: &wgpu::TextureView,
        globals: MasterEffectGlobals,
    ) {
        self.draw_pass_with_pipeline(queue, encoder, pass_index, target, globals, &self.pipeline);
    }

    fn draw_pass_with_pipeline(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass_index: usize,
        target: &wgpu::TextureView,
        globals: MasterEffectGlobals,
        pipeline: &wgpu::RenderPipeline,
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &pass_state.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn modulated_parameter_value(
    slot_index: usize,
    slot: &MasterEffectSlot,
    parameter: &EffectParameterSchema,
    modulation: &MasterModulation,
    sources: [f32; MASTER_MODULATION_SOURCES],
) -> f32 {
    let value = slot
        .parameters
        .iter()
        .find(|value| value.id == parameter.id)
        .map_or(parameter.default, |value| value.value);
    let key = effect_parameter_key(&slot.package_id, &parameter.id);
    let modulation_value = modulation
        .routes
        .iter()
        .filter(|route| {
            route.enabled
                && usize::from(route.target_slot) == slot_index
                && route.parameter_key == key
        })
        .filter_map(|route| {
            sources
                .get(usize::from(route.source))
                .map(|source| source * route.amount.clamp(-1.0, 1.0))
        })
        .sum::<f32>();
    finite_clamp(
        value + modulation_value * (parameter.maximum - parameter.minimum) * 0.5,
        parameter.minimum,
        parameter.maximum,
        parameter.default,
    )
}

fn effect_reload_loop(
    device: wgpu::Device,
    layout: wgpu::PipelineLayout,
    commands: Receiver<EffectReloadCommand>,
    results: Sender<EffectReloadResult>,
) {
    let mut watched = Vec::new();
    let mut last_fingerprints = HashMap::new();
    let mut generation = 0;
    loop {
        match commands.recv_timeout(Duration::from_millis(500)) {
            Ok(EffectReloadCommand::Watch {
                generation: next_generation,
                path,
            }) => {
                generation = next_generation;
                watched = vec![path];
                last_fingerprints.clear();
            }
            Ok(EffectReloadCommand::WatchMany {
                generation: next_generation,
                paths,
            }) => {
                generation = next_generation;
                watched = paths;
                watched.sort();
                watched.dedup();
                last_fingerprints.clear();
            }
            Ok(EffectReloadCommand::Reload {
                generation: next_generation,
            }) => {
                generation = next_generation;
                last_fingerprints.clear();
            }
            Ok(EffectReloadCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        for path in &watched {
            let fingerprint = unchecked_package_fingerprint(path);
            if last_fingerprints.get(path) == Some(&fingerprint) {
                continue;
            }
            last_fingerprints.insert(path.clone(), fingerprint);
            let result = compile_effect_package(&device, &layout, path);
            let _ = results.send(EffectReloadResult {
                generation,
                path: path.clone(),
                result,
            });
        }
    }
}

fn compile_effect_package(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    manifest_path: &Path,
) -> Result<CompiledEffectPipeline, EffectReloadFailure> {
    let package = load_effect_package(manifest_path).map_err(|error| EffectReloadFailure {
        message: error.to_string(),
        retire_custom_pipeline: false,
    })?;
    let targets = package.manifest.resolved_targets();
    let master_compatible = package.manifest.role == EffectPackageRole::MasterProcessor
        || (matches!(
            package.manifest.resolved_abi(),
            EffectPackageAbi::MasterV1 | EffectPackageAbi::DeckV1
        ) && targets.contains(&EffectPackageTarget::Master));
    if !master_compatible {
        return Err(EffectReloadFailure {
            message: "package does not target the master shader runtime".to_owned(),
            retire_custom_pipeline: true,
        });
    }
    compile_validated_effect_package(device, layout, package).map_err(|message| {
        EffectReloadFailure {
            message,
            retire_custom_pipeline: false,
        }
    })
}

fn compile_validated_effect_package(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    package: ValidatedEffectPackage,
) -> Result<CompiledEffectPipeline, String> {
    let pass_entries: Vec<_> = package
        .manifest
        .pass_entries()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&package.manifest.name),
        source: wgpu::ShaderSource::Wgsl(package.shader_source.clone().into()),
    });
    let pipelines = pass_entries
        .iter()
        .enumerate()
        .map(|(index, fragment_entry)| {
            create_master_effect_pipeline(
                device,
                layout,
                &shader,
                &package.manifest.vertex_entry,
                fragment_entry,
                &format!("{} pass {}", package.manifest.name, index + 1),
            )
        })
        .collect();
    if let Some(error) = pollster::block_on(scope.pop()) {
        return Err(format!("GPU pipeline validation failed: {error}"));
    }
    Ok(CompiledEffectPipeline {
        pipelines,
        id: package.manifest.id,
        name: package.manifest.name,
        role: package.manifest.role,
        parameters: package.manifest.parameters,
        history: package.manifest.resources.history,
        fingerprint: package.fingerprint,
    })
}

fn create_master_effect_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    vertex_entry: &str,
    fragment_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vertex_entry),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
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
    fn history_extension_preserves_the_master_v1_parameter_offset() {
        assert_eq!(std::mem::offset_of!(MasterEffectGlobals, parameters), 48);
        assert_eq!(
            std::mem::offset_of!(MasterEffectGlobals, history_valid),
            176
        );
        assert_eq!(std::mem::offset_of!(MasterEffectGlobals, deck_index), 180);
        assert_eq!(std::mem::offset_of!(MasterEffectGlobals, source_extent), 184);
        assert_eq!(
            std::mem::offset_of!(MasterEffectGlobals, composition_extent),
            192
        );
        assert_eq!(size_of::<MasterEffectGlobals>(), 208);
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
            ..MasterEffectSlot::default()
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

        sanitized.slots[0].kind = MasterEffectKind::Custom;
        sanitized.slots[0].package_id = "chromatic-split".to_owned();
        sanitized.slots[0].parameters = vec![EffectParameterValue {
            id: "amount".to_owned(),
            value: 0.02,
        }];
        assert!(sanitized.active());
    }

    #[test]
    fn master_modulation_targets_stable_parameter_keys_and_clamps() {
        let slot = MasterEffectSlot {
            kind: MasterEffectKind::Custom,
            package_id: "test-effect".to_owned(),
            parameters: vec![EffectParameterValue {
                id: "amount".to_owned(),
                value: 0.25,
            }],
            ..MasterEffectSlot::default()
        };
        let schema = EffectParameterSchema {
            id: "amount".to_owned(),
            label: "Amount".to_owned(),
            minimum: 0.0,
            maximum: 1.0,
            default: 0.5,
            group: String::new(),
            control: crate::EffectParameterControl::Slider,
            options: Vec::new(),
        };
        let key = effect_parameter_key("test-effect", "amount");
        let mut modulation = MasterModulation::default();
        modulation.routes[0] = MasterModulationRoute {
            enabled: true,
            source: 0,
            target_slot: 0,
            parameter_key: key,
            amount: 1.0,
        };
        let mut sources = [0.0; MASTER_MODULATION_SOURCES];
        sources[0] = 1.0;
        assert_eq!(
            modulated_parameter_value(0, &slot, &schema, &modulation, sources),
            0.75
        );
        assert_eq!(
            modulated_parameter_value(1, &slot, &schema, &modulation, sources),
            0.25
        );
        modulation.routes[1] = modulation.routes[0];
        assert_eq!(
            modulated_parameter_value(0, &slot, &schema, &modulation, sources),
            1.0
        );
    }

    #[test]
    fn master_lfos_share_audio_beat_and_bar_source_layout() {
        let mut modulation = MasterModulation::default();
        modulation.lfos[0].enabled = true;
        modulation.lfos[0].waveform = LfoWaveform::Square;
        let sources = modulation.source_values(0.0, 5.5, [0.1, 0.2, 0.3, 0.4, 0.5]);
        assert_eq!(sources[0], 0.5);
        assert_eq!(&sources[3..8], &[0.1, 0.2, 0.3, 0.4, 0.5]);
        assert_eq!(sources[8], 0.5);
        assert_eq!(sources[9], 0.375);
    }
}
