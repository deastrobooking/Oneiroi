use oneiroi_hap::{CompressedPlaneFormat, DecodedPlane};
use oneiroi_render::CompressedTexture;

const SIZE: u32 = 4;
const ROW_BYTES: u32 = 256;

fn bc_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;
    if !adapter
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    {
        return None;
    }
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bc-upload-test-device"),
        required_features: wgpu::Features::TEXTURE_COMPRESSION_BC,
        ..Default::default()
    }))
    .ok()
}

#[test]
fn uploads_and_samples_known_bc1_block() {
    let Some((device, queue)) = bc_device() else {
        eprintln!("no BC-capable GPU adapter available; skipping");
        return;
    };

    // BC1 endpoint 0 is RGB565 red; all selectors choose endpoint 0.
    let plane = DecodedPlane {
        format: CompressedPlaneFormat::Bc1Rgb,
        coded_extent: [SIZE, SIZE],
        visible_extent: [SIZE, SIZE],
        data: vec![0x00, 0xf8, 0x00, 0x00, 0, 0, 0, 0],
    };
    let source = CompressedTexture::upload(&device, &queue, &plane, Some("known-red-bc1")).unwrap();

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("bc-sample-shader"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
                @group(0) @binding(0) var source: texture_2d<f32>;
                @group(0) @binding(1) var source_sampler: sampler;

                @vertex
                fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
                    let positions = array(
                        vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0)
                    );
                    return vec4(positions[index], 0.0, 1.0);
                }

                @fragment
                fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
                    return textureSample(source, source_sampler, position.xy / 4.0);
                }
            "#
            .into(),
        ),
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("bc-sample-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bc-sample-bind-group"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source.view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("bc-sample-pipeline-layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("bc-sample-pipeline"),
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
                format: wgpu::TextureFormat::Rgba8Unorm,
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

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bc-sample-target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bc-sample-readback"),
        size: u64::from(ROW_BYTES * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("bc-sample-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
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
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |result| {
        result.expect("map BC sample readback");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();

    let pixels = readback.slice(..).get_mapped_range();
    let centre = &pixels[(2 * ROW_BYTES + 2 * 4) as usize..][..4];
    assert!(centre[0] > 245, "red channel was {}", centre[0]);
    assert!(centre[1] < 10, "green channel was {}", centre[1]);
    assert!(centre[2] < 10, "blue channel was {}", centre[2]);
    assert_eq!(centre[3], 255);
}
