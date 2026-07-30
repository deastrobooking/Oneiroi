//! Four-source GPU compositor for unified media frames.

use bytemuck::{Pod, Zeroable};
use oneiroi_hap::CompressedPlaneFormat;
use oneiroi_media::{RgbaFrame, VideoFramePayload};
use thiserror::Error;

use crate::{CompressedTexture, UploadError};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MixerGlobals {
    levels: [f32; 4],
    source_kinds: [u32; 4],
    contrast: [f32; 4],
    saturation: [f32; 4],
    hue: [f32; 4],
    black_level: [f32; 4],
    white_level: [f32; 4],
    gamma: [f32; 4],
    pixelate: [f32; 4],
    luma_key: [f32; 4],
    neon: [f32; 4],
    fractal: [f32; 4],
    jitter: [f32; 4],
    find_edges: [f32; 4],
    bit_reduction: [f32; 4],
    blacklight: [f32; 4],
    mirror: [u32; 4],
    position_x: [f32; 4],
    position_y: [f32; 4],
    scale: [f32; 4],
    rotation: [f32; 4],
    flip_horizontal: [u32; 4],
    flip_vertical: [u32; 4],
    crop_left: [f32; 4],
    crop_right: [f32; 4],
    crop_top: [f32; 4],
    crop_bottom: [f32; 4],
    source_modes: [u32; 4],
    blend_modes: [u32; 4],
    bus_assignments: [u32; 4],
    crossfade_gains: [f32; 2],
    master_opacity: f32,
    time_seconds: f32,
    output_aspect: f32,
    blackout: u32,
    _padding_a: u32,
    _padding_b: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckEffects {
    /// 1.0 is neutral.
    pub contrast: f32,
    /// 1.0 is neutral, 0.0 is monochrome.
    pub saturation: f32,
    /// Hue rotation in turns.
    pub hue: f32,
    pub black_level: f32,
    pub white_level: f32,
    /// 1.0 is neutral.
    pub gamma: f32,
    /// Normalized block size. Zero disables pixelation.
    pub pixelate: f32,
    /// Luma threshold. Zero disables the key.
    pub luma_key: f32,
    pub neon: f32,
    pub fractal: f32,
    pub jitter: f32,
    pub find_edges: f32,
    pub bit_reduction: f32,
    pub blacklight: f32,
    pub mirror: bool,
}

impl Default for DeckEffects {
    fn default() -> Self {
        Self {
            contrast: 1.0,
            saturation: 1.0,
            hue: 0.0,
            black_level: 0.0,
            white_level: 1.0,
            gamma: 1.0,
            pixelate: 0.0,
            luma_key: 0.0,
            neon: 0.0,
            fractal: 0.0,
            jitter: 0.0,
            find_edges: 0.0,
            bit_reduction: 0.0,
            blacklight: 0.0,
            mirror: false,
        }
    }
}

impl DeckEffects {
    pub fn sanitized(mut self) -> Self {
        self.contrast = self.contrast.clamp(0.0, 4.0);
        self.saturation = self.saturation.clamp(0.0, 4.0);
        self.hue = self.hue.clamp(-1.0, 1.0);
        self.black_level = self.black_level.clamp(0.0, 0.95);
        self.white_level = self.white_level.clamp(self.black_level + 0.01, 1.0);
        self.gamma = self.gamma.clamp(0.1, 4.0);
        self.pixelate = self.pixelate.clamp(0.0, 0.5);
        self.luma_key = self.luma_key.clamp(0.0, 1.0);
        self.neon = self.neon.clamp(0.0, 1.0);
        self.fractal = self.fractal.clamp(0.0, 1.0);
        self.jitter = self.jitter.clamp(0.0, 1.0);
        self.find_edges = self.find_edges.clamp(0.0, 1.0);
        self.bit_reduction = self.bit_reduction.clamp(0.0, 1.0);
        self.blacklight = self.blacklight.clamp(0.0, 1.0);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LfoWaveform {
    #[default]
    Sine,
    Triangle,
    Saw,
    SawDown,
    Square,
}

impl LfoWaveform {
    pub fn sample(self, phase: f32) -> f32 {
        let phase = phase.rem_euclid(1.0);
        match self {
            Self::Sine => (phase * std::f32::consts::TAU).sin(),
            Self::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
            Self::Saw => phase * 2.0 - 1.0,
            Self::SawDown => 1.0 - phase * 2.0,
            Self::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EffectTarget {
    #[default]
    Hue,
    Contrast,
    Saturation,
    BlackLevel,
    WhiteLevel,
    Gamma,
    Pixelate,
    LumaKey,
    Neon,
    Fractal,
    Jitter,
    FindEdges,
    BitReduction,
    Blacklight,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectLfo {
    pub enabled: bool,
    pub direct_enabled: bool,
    pub target: EffectTarget,
    pub waveform: LfoWaveform,
    pub rate_hz: f32,
    pub tempo_sync: bool,
    pub beats_per_cycle: f32,
    pub depth: f32,
    pub phase: f32,
}

impl Default for EffectLfo {
    fn default() -> Self {
        Self {
            enabled: false,
            direct_enabled: true,
            target: EffectTarget::Hue,
            waveform: LfoWaveform::Sine,
            rate_hz: 0.25,
            tempo_sync: false,
            beats_per_cycle: 1.0,
            depth: 0.5,
            phase: 0.0,
        }
    }
}

pub const MOD_ROUTES_PER_DECK: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModulationRoute {
    pub enabled: bool,
    pub source: u8,
    pub target: EffectTarget,
    /// Bipolar route amount. Negative values invert the source.
    pub amount: f32,
}

impl Default for ModulationRoute {
    fn default() -> Self {
        Self {
            enabled: false,
            source: 0,
            target: EffectTarget::Hue,
            amount: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeckLfos {
    pub lanes: [EffectLfo; 3],
    pub routes: [ModulationRoute; MOD_ROUTES_PER_DECK],
}

impl DeckLfos {
    pub fn apply(
        self,
        mut effects: DeckEffects,
        time_seconds: f32,
        beat_position: f32,
    ) -> DeckEffects {
        let mut source_values = [0.0; 3];
        for (index, lfo) in self.lanes.into_iter().enumerate() {
            if !lfo.enabled {
                continue;
            }
            let cycle = if lfo.tempo_sync {
                beat_position / lfo.beats_per_cycle.clamp(0.0625, 8.0)
            } else {
                time_seconds * lfo.rate_hz.clamp(0.01, 20.0)
            };
            let value = lfo.waveform.sample(cycle + lfo.phase) * lfo.depth.clamp(0.0, 1.0);
            source_values[index] = value;
            if lfo.direct_enabled {
                modulate(&mut effects, lfo.target, value);
            }
        }
        for route in self.routes {
            if !route.enabled {
                continue;
            }
            let Some(source) = source_values.get(usize::from(route.source)) else {
                continue;
            };
            modulate(
                &mut effects,
                route.target,
                *source * route.amount.clamp(-1.0, 1.0),
            );
        }
        effects.sanitized()
    }
}

fn modulate(effects: &mut DeckEffects, target: EffectTarget, value: f32) {
    match target {
        EffectTarget::Hue => effects.hue += value * 0.5,
        EffectTarget::Contrast => effects.contrast += value * 2.0,
        EffectTarget::Saturation => effects.saturation += value * 2.0,
        EffectTarget::BlackLevel => effects.black_level += value * 0.45,
        EffectTarget::WhiteLevel => effects.white_level += value * 0.45,
        EffectTarget::Gamma => effects.gamma += value * 1.5,
        EffectTarget::Pixelate => effects.pixelate += value * 0.1,
        EffectTarget::LumaKey => effects.luma_key += value,
        EffectTarget::Neon => effects.neon += value,
        EffectTarget::Fractal => effects.fractal += value,
        EffectTarget::Jitter => effects.jitter += value,
        EffectTarget::FindEdges => effects.find_edges += value,
        EffectTarget::BitReduction => effects.bit_reduction += value,
        EffectTarget::Blacklight => effects.blacklight += value,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MixerParams {
    pub levels: [f32; 4],
    pub solo: [bool; 4],
    pub bypassed: [bool; 4],
    pub buses: [MixerBus; 4],
    pub crossfade_gains: [f32; 2],
    pub transforms: [DeckTransform; 4],
    pub blend_modes: [LayerBlendMode; 4],
    pub output_aspect: f32,
    pub effects: [DeckEffects; 4],
    pub master_opacity: f32,
    pub time_seconds: f32,
    pub blackout: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckTransform {
    /// Position in normalized output coordinates. `1.0` moves the layer by
    /// half the output width or height.
    pub position: [f32; 2],
    pub scale: f32,
    /// Clockwise rotation in turns.
    pub rotation: f32,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    /// Normalized left, right, top and bottom crop amounts.
    pub crop: [f32; 4],
    pub source_mode: SourceMode,
}

impl Default for DeckTransform {
    fn default() -> Self {
        Self {
            position: [0.0; 2],
            scale: 1.0,
            rotation: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
            crop: [0.0; 4],
            source_mode: SourceMode::Stretch,
        }
    }
}

impl DeckTransform {
    pub fn sanitized(mut self) -> Self {
        self.position = self.position.map(|value| {
            if value.is_finite() {
                value.clamp(-2.0, 2.0)
            } else {
                0.0
            }
        });
        self.scale = if self.scale.is_finite() {
            self.scale.clamp(0.05, 4.0)
        } else {
            1.0
        };
        self.rotation = if self.rotation.is_finite() {
            self.rotation.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.crop = self.crop.map(|value| {
            if value.is_finite() {
                value.clamp(0.0, 0.95)
            } else {
                0.0
            }
        });
        normalize_crop_pair(&mut self.crop, 0, 1);
        normalize_crop_pair(&mut self.crop, 2, 3);
        self
    }
}

fn normalize_crop_pair(crop: &mut [f32; 4], first: usize, second: usize) {
    let sum = crop[first] + crop[second];
    if sum > 0.98 {
        let scale = 0.98 / sum;
        crop[first] *= scale;
        crop[second] *= scale;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SourceMode {
    Fit,
    Fill,
    #[default]
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LayerBlendMode {
    #[default]
    Normal,
    Add,
    Screen,
    Multiply,
    Difference,
    Lighten,
    Darken,
    Overlay,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MixerBus {
    #[default]
    A,
    B,
}

impl Default for MixerParams {
    fn default() -> Self {
        Self {
            levels: [1.0; 4],
            solo: [false; 4],
            bypassed: [false; 4],
            buses: [MixerBus::A; 4],
            crossfade_gains: [1.0, 0.0],
            transforms: [DeckTransform::default(); 4],
            blend_modes: [LayerBlendMode::Normal; 4],
            output_aspect: 1.0,
            effects: [DeckEffects::default(); 4],
            master_opacity: 1.0,
            time_seconds: 0.0,
            blackout: false,
        }
    }
}

#[derive(Debug, Error)]
pub enum MixerUploadError {
    #[error("deck index {0} is outside the four-deck mixer")]
    InvalidDeck(usize),
    #[error("RGBA frame data has {actual} bytes; expected {expected}")]
    RgbaSize { actual: usize, expected: usize },
    #[error("RGBA frame extent is invalid")]
    InvalidRgbaExtent,
    #[error("upload HAP texture: {0}")]
    Compressed(#[from] UploadError),
    #[error("unsupported HAP plane combination")]
    UnsupportedHapPlanes,
}

enum TextureResource {
    Rgba {
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        extent: [u32; 2],
    },
    Compressed(CompressedTexture),
}

impl TextureResource {
    fn view(&self) -> &wgpu::TextureView {
        match self {
            Self::Rgba { view, .. } => view,
            Self::Compressed(texture) => &texture.view,
        }
    }
}

struct DeckSource {
    primary: TextureResource,
    secondary: Option<TextureResource>,
    kind: u32,
}

pub struct FourDeckCompositor {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    globals: wgpu::Buffer,
    transparent: TextureResource,
    opaque_alpha: TextureResource,
    sources: [Option<DeckSource>; 4],
    bind_group: Option<wgpu::BindGroup>,
}

impl FourDeckCompositor {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("four-deck-mixer"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/mixer.wgsl").into()),
        });
        let mut entries = Vec::with_capacity(10);
        entries.push(sampler_layout_entry(0));
        entries.extend((1..=8).map(texture_layout_entry));
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 9,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("four-deck-mixer-layout"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("four-deck-mixer-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("four-deck-mixer-pipeline"),
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
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("four-deck-mixer-globals"),
            size: size_of::<MixerGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("four-deck-mixer-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let transparent = solid_texture(device, queue, [0, 0, 0, 0], "transparent-source");
        let opaque_alpha = solid_texture(device, queue, [255; 4], "opaque-alpha");

        Self {
            pipeline,
            layout,
            sampler,
            globals,
            transparent,
            opaque_alpha,
            sources: std::array::from_fn(|_| None),
            bind_group: None,
        }
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        deck: usize,
        payload: &VideoFramePayload,
    ) -> Result<(), MixerUploadError> {
        if deck >= 4 {
            return Err(MixerUploadError::InvalidDeck(deck));
        }
        if let VideoFramePayload::Rgba8(frame) = payload
            && let Some(DeckSource {
                primary:
                    TextureResource::Rgba {
                        texture, extent, ..
                    },
                secondary: None,
                kind: 0,
            }) = self.sources[deck].as_mut()
            && *extent == frame.extent
        {
            validate_rgba(frame)?;
            write_rgba(queue, texture, frame);
            return Ok(());
        }
        if let VideoFramePayload::BlockCompressed(frame) = payload
            && let Some(source) = self.sources[deck].as_mut()
            && update_hap_source(queue, source, frame)?
        {
            return Ok(());
        }
        let source = match payload {
            VideoFramePayload::Rgba8(frame) => DeckSource {
                primary: upload_rgba(device, queue, frame)?,
                secondary: None,
                kind: 0,
            },
            VideoFramePayload::BlockCompressed(frame) => {
                let color = frame
                    .planes
                    .iter()
                    .find(|plane| plane.format != CompressedPlaneFormat::Bc4Alpha)
                    .or_else(|| frame.planes.first())
                    .ok_or(MixerUploadError::UnsupportedHapPlanes)?;
                let alpha = frame
                    .planes
                    .iter()
                    .find(|plane| plane.format == CompressedPlaneFormat::Bc4Alpha);
                let kind = match (color.format, alpha) {
                    (CompressedPlaneFormat::Bc3ScaledYCoCg, Some(_)) => 3,
                    (CompressedPlaneFormat::Bc3ScaledYCoCg, None) => 1,
                    (CompressedPlaneFormat::Bc4Alpha, _) => 2,
                    (_, None) if frame.planes.len() == 1 => 0,
                    _ => return Err(MixerUploadError::UnsupportedHapPlanes),
                };
                DeckSource {
                    primary: TextureResource::Compressed(CompressedTexture::upload(
                        device,
                        queue,
                        color,
                        Some("hap-primary"),
                    )?),
                    secondary: alpha
                        .map(|plane| {
                            CompressedTexture::upload(device, queue, plane, Some("hap-alpha"))
                                .map(TextureResource::Compressed)
                        })
                        .transpose()?,
                    kind,
                }
            }
        };
        self.sources[deck] = Some(source);
        self.bind_group = None;
        Ok(())
    }

    pub fn clear_deck(&mut self, deck: usize) {
        if deck < 4 {
            self.sources[deck] = None;
            self.bind_group = None;
        }
    }

    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        params: MixerParams,
    ) {
        let kinds = std::array::from_fn(|index| {
            self.sources[index].as_ref().map_or(0, |source| source.kind)
        });
        let effects = params.effects.map(DeckEffects::sanitized);
        let transforms = params.transforms.map(DeckTransform::sanitized);
        let any_solo = params.solo.into_iter().any(|solo| solo);
        let levels = std::array::from_fn(|index| {
            if params.bypassed[index] || (any_solo && !params.solo[index]) {
                0.0
            } else {
                params.levels[index]
            }
        });
        let float_values = |read: fn(DeckEffects) -> f32| effects.map(read);
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&MixerGlobals {
                levels,
                source_kinds: kinds,
                contrast: float_values(|effect| effect.contrast),
                saturation: float_values(|effect| effect.saturation),
                hue: float_values(|effect| effect.hue),
                black_level: float_values(|effect| effect.black_level),
                white_level: float_values(|effect| effect.white_level),
                gamma: float_values(|effect| effect.gamma),
                pixelate: float_values(|effect| effect.pixelate),
                luma_key: float_values(|effect| effect.luma_key),
                neon: float_values(|effect| effect.neon),
                fractal: float_values(|effect| effect.fractal),
                jitter: float_values(|effect| effect.jitter),
                find_edges: float_values(|effect| effect.find_edges),
                bit_reduction: float_values(|effect| effect.bit_reduction),
                blacklight: float_values(|effect| effect.blacklight),
                mirror: effects.map(|effect| u32::from(effect.mirror)),
                position_x: transforms.map(|transform| transform.position[0]),
                position_y: transforms.map(|transform| transform.position[1]),
                scale: transforms.map(|transform| transform.scale),
                rotation: transforms.map(|transform| transform.rotation),
                flip_horizontal: transforms.map(|transform| u32::from(transform.flip_horizontal)),
                flip_vertical: transforms.map(|transform| u32::from(transform.flip_vertical)),
                crop_left: transforms.map(|transform| transform.crop[0]),
                crop_right: transforms.map(|transform| transform.crop[1]),
                crop_top: transforms.map(|transform| transform.crop[2]),
                crop_bottom: transforms.map(|transform| transform.crop[3]),
                source_modes: transforms.map(|transform| match transform.source_mode {
                    SourceMode::Fit => 0,
                    SourceMode::Fill => 1,
                    SourceMode::Stretch => 2,
                }),
                blend_modes: params.blend_modes.map(|mode| match mode {
                    LayerBlendMode::Normal => 0,
                    LayerBlendMode::Add => 1,
                    LayerBlendMode::Screen => 2,
                    LayerBlendMode::Multiply => 3,
                    LayerBlendMode::Difference => 4,
                    LayerBlendMode::Lighten => 5,
                    LayerBlendMode::Darken => 6,
                    LayerBlendMode::Overlay => 7,
                }),
                bus_assignments: params.buses.map(|bus| match bus {
                    MixerBus::A => 0,
                    MixerBus::B => 1,
                }),
                crossfade_gains: params.crossfade_gains.map(|gain| gain.clamp(0.0, 1.0)),
                master_opacity: params.master_opacity.clamp(0.0, 1.0),
                time_seconds: params.time_seconds,
                output_aspect: if params.output_aspect.is_finite() {
                    params.output_aspect.clamp(0.01, 100.0)
                } else {
                    1.0
                },
                blackout: u32::from(params.blackout),
                _padding_a: 0,
                _padding_b: 0,
            }),
        );
        if self.bind_group.is_none() {
            self.bind_group = Some(self.create_bind_group(device));
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("four-deck-mixer-pass"),
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
        pass.set_bind_group(0, self.bind_group.as_ref().unwrap(), &[]);
        pass.draw(0..3, 0..1);
    }

    fn create_bind_group(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        let primary = std::array::from_fn::<_, 4, _>(|i| self.primary_view(i));
        let alpha = std::array::from_fn::<_, 4, _>(|i| self.alpha_view(i));
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("four-deck-mixer-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                texture_entry(1, primary[0]),
                texture_entry(2, primary[1]),
                texture_entry(3, primary[2]),
                texture_entry(4, primary[3]),
                texture_entry(5, alpha[0]),
                texture_entry(6, alpha[1]),
                texture_entry(7, alpha[2]),
                texture_entry(8, alpha[3]),
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: self.globals.as_entire_binding(),
                },
            ],
        })
    }

    fn primary_view(&self, index: usize) -> &wgpu::TextureView {
        self.sources[index]
            .as_ref()
            .map_or(self.transparent.view(), |source| source.primary.view())
    }

    fn alpha_view(&self, index: usize) -> &wgpu::TextureView {
        self.sources[index]
            .as_ref()
            .and_then(|source| source.secondary.as_ref())
            .map_or(self.opaque_alpha.view(), TextureResource::view)
    }
}

fn update_hap_source(
    queue: &wgpu::Queue,
    source: &mut DeckSource,
    frame: &oneiroi_hap::DecodedFrame,
) -> Result<bool, MixerUploadError> {
    let color = frame
        .planes
        .iter()
        .find(|plane| plane.format != CompressedPlaneFormat::Bc4Alpha)
        .or_else(|| frame.planes.first())
        .ok_or(MixerUploadError::UnsupportedHapPlanes)?;
    let alpha = frame
        .planes
        .iter()
        .find(|plane| plane.format == CompressedPlaneFormat::Bc4Alpha);
    let kind = match (color.format, alpha) {
        (CompressedPlaneFormat::Bc3ScaledYCoCg, Some(_)) => 3,
        (CompressedPlaneFormat::Bc3ScaledYCoCg, None) => 1,
        (CompressedPlaneFormat::Bc4Alpha, _) => 2,
        (_, None) if frame.planes.len() == 1 => 0,
        _ => return Err(MixerUploadError::UnsupportedHapPlanes),
    };
    if source.kind != kind {
        return Ok(false);
    }
    let TextureResource::Compressed(primary) = &mut source.primary else {
        return Ok(false);
    };
    if !primary.update(queue, color)? {
        return Ok(false);
    }
    match (alpha, source.secondary.as_mut()) {
        (None, None) => Ok(true),
        (Some(plane), Some(TextureResource::Compressed(secondary))) => {
            secondary.update(queue, plane).map_err(Into::into)
        }
        _ => Ok(false),
    }
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

fn solid_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pixel: [u8; 4],
    label: &str,
) -> TextureResource {
    upload_rgba(
        device,
        queue,
        &RgbaFrame {
            extent: [1, 1],
            data: pixel.to_vec(),
        },
    )
    .unwrap_or_else(|_| panic!("create {label}"))
}

fn upload_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &RgbaFrame,
) -> Result<TextureResource, MixerUploadError> {
    validate_rgba(frame)?;
    let [width, height] = frame.extent;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rgba-deck-source"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_rgba(queue, &texture, frame);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(TextureResource::Rgba {
        texture,
        view,
        extent: frame.extent,
    })
}

fn validate_rgba(frame: &RgbaFrame) -> Result<(), MixerUploadError> {
    let [width, height] = frame.extent;
    let expected = usize::try_from(width)
        .ok()
        .and_then(|w| w.checked_mul(4))
        .and_then(|row| usize::try_from(height).ok()?.checked_mul(row))
        .ok_or(MixerUploadError::InvalidRgbaExtent)?;
    if width == 0 || height == 0 {
        return Err(MixerUploadError::InvalidRgbaExtent);
    }
    if frame.data.len() != expected {
        return Err(MixerUploadError::RgbaSize {
            actual: frame.data.len(),
            expected,
        });
    }
    Ok(())
}

fn write_rgba(queue: &wgpu::Queue, texture: &wgpu::Texture, frame: &RgbaFrame) {
    let [width, height] = frame.extent;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &frame.data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lfo_waveforms_have_expected_quarter_cycle_values() {
        assert!((LfoWaveform::Sine.sample(0.25) - 1.0).abs() < 0.0001);
        assert_eq!(LfoWaveform::Triangle.sample(0.25), 0.0);
        assert_eq!(LfoWaveform::Saw.sample(0.25), -0.5);
        assert_eq!(LfoWaveform::SawDown.sample(0.25), 0.5);
        assert_eq!(LfoWaveform::Square.sample(0.25), 1.0);
        assert_eq!(LfoWaveform::Square.sample(0.75), -1.0);
    }

    #[test]
    fn enabled_lfo_modulates_a_copy_and_clamps_the_result() {
        let base = DeckEffects::default();
        let lfos = DeckLfos {
            lanes: [
                EffectLfo {
                    enabled: true,
                    direct_enabled: true,
                    target: EffectTarget::Hue,
                    waveform: LfoWaveform::Sine,
                    rate_hz: 1.0,
                    tempo_sync: false,
                    beats_per_cycle: 1.0,
                    depth: 1.0,
                    phase: 0.0,
                },
                EffectLfo {
                    enabled: true,
                    direct_enabled: true,
                    target: EffectTarget::Neon,
                    waveform: LfoWaveform::Square,
                    rate_hz: 1.0,
                    tempo_sync: false,
                    beats_per_cycle: 1.0,
                    depth: 2.0,
                    phase: 0.0,
                },
                EffectLfo::default(),
            ],
            ..Default::default()
        };
        let resolved = lfos.apply(base, 0.25, 0.0);
        assert!((resolved.hue - 0.5).abs() < 0.0001);
        assert_eq!(resolved.neon, 1.0);
        assert_eq!(base, DeckEffects::default());
    }

    #[test]
    fn matrix_routes_one_source_to_multiple_bipolar_destinations() {
        let mut lfos = DeckLfos::default();
        lfos.lanes[0] = EffectLfo {
            enabled: true,
            direct_enabled: false,
            target: EffectTarget::Hue,
            waveform: LfoWaveform::Square,
            rate_hz: 1.0,
            tempo_sync: false,
            beats_per_cycle: 1.0,
            depth: 1.0,
            phase: 0.0,
        };
        lfos.routes[0] = ModulationRoute {
            enabled: true,
            source: 0,
            target: EffectTarget::Contrast,
            amount: 0.5,
        };
        lfos.routes[1] = ModulationRoute {
            enabled: true,
            source: 0,
            target: EffectTarget::Saturation,
            amount: -0.25,
        };

        let resolved = lfos.apply(DeckEffects::default(), 0.25, 0.0);
        assert_eq!(resolved.hue, 0.0);
        assert_eq!(resolved.contrast, 2.0);
        assert_eq!(resolved.saturation, 0.5);
    }

    #[test]
    fn tempo_synced_lfo_uses_musical_position_not_wall_time() {
        let mut lfos = DeckLfos::default();
        lfos.lanes[0] = EffectLfo {
            enabled: true,
            direct_enabled: true,
            target: EffectTarget::Hue,
            waveform: LfoWaveform::Sine,
            rate_hz: 20.0,
            tempo_sync: true,
            beats_per_cycle: 2.0,
            depth: 1.0,
            phase: 0.0,
        };
        let first = lfos.apply(DeckEffects::default(), 100.0, 0.5);
        let later_wall_time = lfos.apply(DeckEffects::default(), 900.0, 0.5);
        assert!((first.hue - 0.5).abs() < 0.0001);
        assert_eq!(first, later_wall_time);
    }
}
