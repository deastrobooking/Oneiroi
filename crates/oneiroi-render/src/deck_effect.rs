//! Bounded stateless per-deck effect-package execution.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use bytemuck::{Pod, Zeroable};
use oneiroi_core::effect_parameter_key;

use crate::{
    EffectPackageAbi, EffectPackageTarget, EffectParameterSchema, EffectParameterValue,
    ValidatedEffectPackage, load_effect_package,
};

pub const DECK_EFFECT_PARAMETER_CAPACITY: usize = 32;
pub const DECK_PACKAGE_MODULATION_ROUTES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckPackageModulationRoute {
    pub enabled: bool,
    pub source: u8,
    pub parameter_key: u64,
    pub amount: f32,
}

impl Default for DeckPackageModulationRoute {
    fn default() -> Self {
        Self {
            enabled: false,
            source: 0,
            parameter_key: 0,
            amount: 0.5,
        }
    }
}

/// One stateless package stage placed after a deck's built-in processing and
/// before its layer blend.
#[derive(Clone, Debug, PartialEq)]
pub struct DeckPackageSlot {
    pub bypassed: bool,
    pub mix: f32,
    pub package_id: String,
    pub parameters: Vec<EffectParameterValue>,
    pub modulation: [DeckPackageModulationRoute; DECK_PACKAGE_MODULATION_ROUTES],
}

impl Default for DeckPackageSlot {
    fn default() -> Self {
        Self {
            bypassed: false,
            mix: 1.0,
            package_id: String::new(),
            parameters: Vec::new(),
            modulation: [DeckPackageModulationRoute::default(); DECK_PACKAGE_MODULATION_ROUTES],
        }
    }
}

impl DeckPackageSlot {
    pub fn active(&self) -> bool {
        !self.bypassed && self.mix.is_finite() && self.mix > 0.0 && !self.package_id.is_empty()
    }

    pub fn sanitize(&mut self) {
        self.mix = if self.mix.is_finite() {
            self.mix.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.parameters.retain(|parameter| {
            !parameter.id.is_empty() && parameter.id.len() <= 64 && parameter.value.is_finite()
        });
        self.parameters.truncate(DECK_EFFECT_PARAMETER_CAPACITY);
        for route in &mut self.modulation {
            if route.source >= 10 || route.parameter_key == 0 {
                route.enabled = false;
            }
            route.amount = if route.amount.is_finite() {
                route.amount.clamp(-1.0, 1.0)
            } else {
                0.5
            };
        }
    }

    pub fn modulated(&self, sources: [f32; 10], schema: &[EffectParameterSchema]) -> Self {
        let mut slot = self.clone();
        for parameter in schema {
            let key = effect_parameter_key(&slot.package_id, &parameter.id);
            let modulation = slot
                .modulation
                .iter()
                .filter(|route| route.enabled && route.parameter_key == key)
                .filter_map(|route| {
                    sources
                        .get(usize::from(route.source))
                        .map(|source| source * route.amount.clamp(-1.0, 1.0))
                })
                .sum::<f32>();
            if let Some(value) = slot
                .parameters
                .iter_mut()
                .find(|value| value.id == parameter.id)
            {
                value.value = (value.value
                    + modulation * (parameter.maximum - parameter.minimum) * 0.5)
                    .clamp(parameter.minimum, parameter.maximum);
            }
        }
        slot
    }
}

/// `deck-v1` extends the append-only master-v1 physical layout. Shaders that
/// only declare the original fields remain valid, while deck-aware shaders can
/// read placement and extent metadata from the appended fields.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DeckEffectGlobals {
    direction: [f32; 2],
    texel_size: [f32; 2],
    radius: f32,
    mix_amount: f32,
    mode: u32,
    feedback: f32,
    time_seconds: f32,
    parameter_count: u32,
    pass_index: u32,
    pass_count: u32,
    parameters: [f32; DECK_EFFECT_PARAMETER_CAPACITY],
    history_valid: u32,
    deck_index: u32,
    source_extent: [u32; 2],
    composition_extent: [u32; 2],
    _padding: [u32; 2],
}

pub(crate) struct DeckEffectPass {
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
}

struct RegisteredDeckPipeline {
    manifest_path: PathBuf,
    pipeline: wgpu::RenderPipeline,
    parameters: Vec<EffectParameterSchema>,
}

struct CompiledDeckPipeline {
    id: String,
    name: String,
    manifest_path: PathBuf,
    pipeline: wgpu::RenderPipeline,
    parameters: Vec<EffectParameterSchema>,
    fingerprint: u64,
}

struct DeckReloadFailure {
    message: String,
    retire_pipeline: bool,
}

struct DeckReloadResult {
    generation: u64,
    path: PathBuf,
    result: Result<CompiledDeckPipeline, DeckReloadFailure>,
}

enum DeckReloadCommand {
    WatchMany {
        generation: u64,
        paths: Vec<PathBuf>,
    },
    Shutdown,
}

struct DeckReloadWorker {
    commands: Sender<DeckReloadCommand>,
    results: Receiver<DeckReloadResult>,
    thread: Option<JoinHandle<()>>,
}

impl DeckReloadWorker {
    fn new(
        device: wgpu::Device,
        layout: wgpu::PipelineLayout,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        let (commands_tx, commands_rx) = mpsc::channel();
        let (results_tx, results_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("oneiroi-deck-effect-reload".to_owned())
            .spawn(move || {
                deck_reload_loop(device, layout, output_format, commands_rx, results_tx);
            })
            .expect("spawn deck effect reload worker");
        Self {
            commands: commands_tx,
            results: results_rx,
            thread: Some(thread),
        }
    }
}

impl Drop for DeckReloadWorker {
    fn drop(&mut self) {
        let _ = self.commands.send(DeckReloadCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) struct DeckEffectRuntime {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipelines: HashMap<String, RegisteredDeckPipeline>,
    watched_paths: HashSet<PathBuf>,
    generation: u64,
    worker: DeckReloadWorker,
    status: String,
    errors: HashMap<PathBuf, (String, bool)>,
}

impl DeckEffectRuntime {
    pub(crate) fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oneiroi-deck-effect-layout"),
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
            label: Some("oneiroi-deck-effect-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("oneiroi-deck-effect-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            layout,
            sampler,
            pipelines: HashMap::new(),
            watched_paths: HashSet::new(),
            generation: 0,
            worker: DeckReloadWorker::new(device.clone(), pipeline_layout, output_format),
            status: "No deck effect packages watched".to_owned(),
            errors: HashMap::new(),
        }
    }

    pub(crate) fn create_passes(
        &self,
        device: &wgpu::Device,
        input: &wgpu::TextureView,
    ) -> [DeckEffectPass; 4] {
        std::array::from_fn(|index| {
            let globals = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("oneiroi-deck-effect-globals"),
                size: size_of::<DeckEffectGlobals>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("oneiroi-deck-effect-bind-group-{index}")),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    texture_entry(1, input),
                    texture_entry(2, input),
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: globals.as_entire_binding(),
                    },
                    texture_entry(4, input),
                    texture_entry(5, input),
                ],
            });
            DeckEffectPass {
                bind_group,
                globals,
            }
        })
    }

    pub(crate) fn watch_manifests(&mut self, mut paths: Vec<PathBuf>) {
        paths.sort();
        paths.dedup();
        let watched: HashSet<_> = paths.iter().cloned().collect();
        self.pipelines
            .retain(|_, pipeline| watched.contains(&pipeline.manifest_path));
        self.errors.retain(|path, _| watched.contains(path));
        self.watched_paths = watched;
        self.generation = self.generation.wrapping_add(1);
        let _ = self.worker.commands.send(DeckReloadCommand::WatchMany {
            generation: self.generation,
            paths,
        });
        self.status = format!(
            "Watching {} deck package manifest(s)",
            self.watched_paths.len()
        );
    }

    pub(crate) fn poll_reload(&mut self) -> bool {
        let mut changed = false;
        let mut loaded = Vec::new();
        while let Ok(result) = self.worker.results.try_recv() {
            if result.generation != self.generation || !self.watched_paths.contains(&result.path) {
                continue;
            }
            changed = true;
            match result.result {
                Ok(compiled) => {
                    self.errors.remove(&result.path);
                    self.pipelines.retain(|id, pipeline| {
                        pipeline.manifest_path != result.path || id == &compiled.id
                    });
                    self.pipelines.insert(
                        compiled.id.clone(),
                        RegisteredDeckPipeline {
                            manifest_path: compiled.manifest_path,
                            pipeline: compiled.pipeline,
                            parameters: compiled.parameters,
                        },
                    );
                    loaded.push(format!("{} · {:016x}", compiled.name, compiled.fingerprint));
                }
                Err(error) => {
                    let retained = self
                        .pipelines
                        .values()
                        .any(|pipeline| pipeline.manifest_path == result.path)
                        && !error.retire_pipeline;
                    if error.retire_pipeline {
                        self.pipelines
                            .retain(|_, pipeline| pipeline.manifest_path != result.path);
                    }
                    self.errors.insert(result.path, (error.message, retained));
                }
            }
        }
        if changed && !self.errors.is_empty() {
            let mut rejected = self
                .errors
                .iter()
                .map(|(path, (message, _))| format!("{} · {message}", path.display()))
                .collect::<Vec<_>>();
            rejected.sort();
            let retained = self.errors.values().any(|(_, retained)| *retained);
            let neutral = self.errors.values().any(|(_, retained)| !retained);
            let fallback = match (retained, neutral) {
                (true, false) => "using last known good",
                (false, true) => "using neutral fallback",
                (true, true) => "using last known good where available; neutral otherwise",
                (false, false) => "reload unchanged",
            };
            self.status = format!(
                "Deck reload rejected · {} · {fallback}",
                rejected.join(" · ")
            );
        } else if changed && !loaded.is_empty() {
            self.status = format!("Loaded deck packages: {}", loaded.join(" · "));
        }
        changed
    }

    pub(crate) fn status(&self) -> &str {
        &self.status
    }

    pub(crate) fn is_loaded(&self, id: &str) -> bool {
        self.pipelines.contains_key(id)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &DeckEffectPass,
        target: &wgpu::TextureView,
        slot: &DeckPackageSlot,
        deck_index: usize,
        source_extent: [u32; 2],
        composition_extent: [u32; 2],
        time_seconds: f32,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) -> bool {
        let Some(registered) = self.pipelines.get(&slot.package_id) else {
            return false;
        };
        let mut globals = DeckEffectGlobals {
            direction: [0.0; 2],
            texel_size: [
                1.0 / composition_extent[0].max(1) as f32,
                1.0 / composition_extent[1].max(1) as f32,
            ],
            radius: 0.0,
            mix_amount: if slot.mix.is_finite() {
                slot.mix.clamp(0.0, 1.0)
            } else {
                1.0
            },
            mode: 2,
            feedback: 0.0,
            time_seconds,
            parameter_count: 0,
            pass_index: 0,
            pass_count: 1,
            parameters: [0.0; DECK_EFFECT_PARAMETER_CAPACITY],
            history_valid: 0,
            deck_index: deck_index as u32,
            source_extent,
            composition_extent,
            _padding: [0; 2],
        };
        for (index, schema) in registered
            .parameters
            .iter()
            .take(DECK_EFFECT_PARAMETER_CAPACITY)
            .enumerate()
        {
            let value = slot
                .parameters
                .iter()
                .find(|value| value.id == schema.id)
                .map_or(schema.default, |value| value.value);
            globals.parameters[index] = if value.is_finite() {
                value.clamp(schema.minimum, schema.maximum)
            } else {
                schema.default
            };
            globals.parameter_count += 1;
        }
        queue.write_buffer(&pass.globals, 0, bytemuck::bytes_of(&globals));
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("oneiroi-deck-effect-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        render_pass.set_pipeline(&registered.pipeline);
        render_pass.set_bind_group(0, &pass.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        true
    }
}

fn deck_reload_loop(
    device: wgpu::Device,
    layout: wgpu::PipelineLayout,
    output_format: wgpu::TextureFormat,
    commands: Receiver<DeckReloadCommand>,
    results: Sender<DeckReloadResult>,
) {
    let mut generation = 0;
    let mut watched = Vec::new();
    let mut fingerprints = HashMap::new();
    loop {
        match commands.recv_timeout(Duration::from_millis(500)) {
            Ok(DeckReloadCommand::WatchMany {
                generation: next_generation,
                paths,
            }) => {
                generation = next_generation;
                watched = paths;
                fingerprints.clear();
            }
            Ok(DeckReloadCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        for path in &watched {
            let fingerprint = unchecked_package_fingerprint(path);
            if fingerprints.get(path) == Some(&fingerprint) {
                continue;
            }
            fingerprints.insert(path.clone(), fingerprint);
            let result = compile_deck_effect_package(&device, &layout, output_format, path);
            let _ = results.send(DeckReloadResult {
                generation,
                path: path.clone(),
                result,
            });
        }
    }
}

fn compile_deck_effect_package(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    output_format: wgpu::TextureFormat,
    manifest_path: &Path,
) -> Result<CompiledDeckPipeline, DeckReloadFailure> {
    let package = load_effect_package(manifest_path).map_err(|error| DeckReloadFailure {
        message: error.to_string(),
        retire_pipeline: false,
    })?;
    let compatible = package.manifest.resolved_abi() == EffectPackageAbi::DeckV1
        && package
            .manifest
            .resolved_targets()
            .contains(&EffectPackageTarget::Deck);
    if !compatible {
        return Err(DeckReloadFailure {
            message: "package does not target the deck-v1 shader runtime".to_owned(),
            retire_pipeline: true,
        });
    }
    compile_validated_deck_effect(device, layout, output_format, package).map_err(|message| {
        DeckReloadFailure {
            message,
            retire_pipeline: false,
        }
    })
}

fn compile_validated_deck_effect(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    output_format: wgpu::TextureFormat,
    package: ValidatedEffectPackage,
) -> Result<CompiledDeckPipeline, String> {
    let fragment_entry = package.manifest.pass_entries()[0].to_owned();
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&package.manifest.name),
        source: wgpu::ShaderSource::Wgsl(package.shader_source.clone().into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&format!("{} deck-v1", package.manifest.name)),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(&package.manifest.vertex_entry),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(&fragment_entry),
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
    if let Some(error) = pollster::block_on(scope.pop()) {
        return Err(format!("GPU deck pipeline validation failed: {error}"));
    }
    Ok(CompiledDeckPipeline {
        id: package.manifest.id,
        name: package.manifest.name,
        manifest_path: package.manifest_path,
        pipeline,
        parameters: package.manifest.parameters,
        fingerprint: package.fingerprint,
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
    if let Ok(bytes) = manifest_source
        && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
        && let Some(shader) = value.get("shader").and_then(serde_json::Value::as_str)
    {
        let shader_path = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(shader);
        shader_path.hash(&mut hasher);
        if let Ok(shader) = fs::read(shader_path) {
            shader.len().hash(&mut hasher);
            shader.hash(&mut hasher);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_v1_keeps_master_v1_offsets_append_only() {
        assert_eq!(std::mem::offset_of!(DeckEffectGlobals, parameters), 48);
        assert_eq!(std::mem::offset_of!(DeckEffectGlobals, history_valid), 176);
        assert_eq!(std::mem::offset_of!(DeckEffectGlobals, deck_index), 180);
        assert_eq!(std::mem::offset_of!(DeckEffectGlobals, source_extent), 184);
        assert_eq!(
            std::mem::offset_of!(DeckEffectGlobals, composition_extent),
            192
        );
        assert_eq!(size_of::<DeckEffectGlobals>(), 208);
    }

    #[test]
    fn package_modulation_uses_stable_keys_and_declared_ranges() {
        let schema = EffectParameterSchema {
            id: "amount".to_owned(),
            label: "Amount".to_owned(),
            minimum: -2.0,
            maximum: 2.0,
            default: 0.0,
            group: String::new(),
            control: crate::EffectParameterControl::Slider,
            options: Vec::new(),
        };
        let mut slot = DeckPackageSlot {
            package_id: "test-package".to_owned(),
            parameters: vec![EffectParameterValue {
                id: "amount".to_owned(),
                value: 0.25,
            }],
            ..DeckPackageSlot::default()
        };
        slot.modulation[0] = DeckPackageModulationRoute {
            enabled: true,
            source: 3,
            parameter_key: effect_parameter_key("test-package", "amount"),
            amount: 0.5,
        };
        let mut sources = [0.0; 10];
        sources[3] = 1.0;

        let modulated = slot.modulated(sources, &[schema]);

        assert_eq!(modulated.parameters[0].value, 1.25);
        assert_eq!(slot.parameters[0].value, 0.25);
    }
}
