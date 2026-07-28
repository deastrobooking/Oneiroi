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
    master_opacity: f32,
    blackout: u32,
    _padding: [u32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct MixerParams {
    pub levels: [f32; 4],
    pub master_opacity: f32,
    pub blackout: bool,
}

impl Default for MixerParams {
    fn default() -> Self {
        Self {
            levels: [1.0; 4],
            master_opacity: 1.0,
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
        _texture: wgpu::Texture,
        view: wgpu::TextureView,
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
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&MixerGlobals {
                levels: params.levels,
                source_kinds: kinds,
                master_opacity: params.master_opacity,
                blackout: u32::from(params.blackout),
                _padding: [0; 2],
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
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
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
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(TextureResource::Rgba {
        _texture: texture,
        view,
    })
}
