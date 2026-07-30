use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use oneiroi_render::{
    EffectParameterValue, MasterEffectChain, MasterEffectKind, MasterEffectProcessor,
    MasterEffectSlot, PROGRAM_FORMAT, PresentationOptions, ProgramPresenter, ProgramTarget,
};

const SIZE: u32 = 64;
const ROW_BYTES: u32 = 256;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

#[test]
fn presents_the_offscreen_program_texture() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let program = ProgramTarget::new(&device, [SIZE, SIZE]);
    assert_eq!(program.extent(), [SIZE, SIZE]);
    let presenter = ProgramPresenter::new(&device, &program, PROGRAM_FORMAT);
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("program-present-test-output"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: PROGRAM_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let output_view = output.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("program-present-test-readback"),
        size: u64::from(ROW_BYTES * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear-program-red"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &program.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    presenter.draw(
        &queue,
        &mut encoder,
        &output_view,
        [SIZE, SIZE],
        PresentationOptions::default(),
    );
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &output,
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
        result.expect("map program readback");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    assert_eq!(&bytes[..4], &[255, 0, 0, 255]);
    drop(bytes);
    readback.unmap();

    let mut encoder = device.create_command_encoder(&Default::default());
    presenter.draw(
        &queue,
        &mut encoder,
        &output_view,
        [SIZE, SIZE],
        PresentationOptions {
            test_card: true,
            identify: false,
        },
    );
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &output,
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
        result.expect("map test-card readback");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    assert_ne!(&bytes[..4], &[255, 0, 0, 255]);
}

#[test]
fn separable_master_blur_runs_through_bounded_ping_pong_targets() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let program = ProgramTarget::new(&device, [SIZE, SIZE]);
    let mut processor = MasterEffectProcessor::new(&device, &program);
    let presenter = ProgramPresenter::new(&device, &program, PROGRAM_FORMAT);
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("master-blur-test-output"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: PROGRAM_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let output_view = output.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("master-blur-test-readback"),
        size: u64::from(ROW_BYTES * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear-master-blur-input"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: program.composition_view(),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    processor.draw(
        &queue,
        &mut encoder,
        &program,
        &MasterEffectChain {
            slots: [
                MasterEffectSlot {
                    kind: MasterEffectKind::Blur,
                    bypassed: false,
                    mix: 1.0,
                    amount: 16.0,
                    feedback: 0.85,
                    ..MasterEffectSlot::default()
                },
                MasterEffectSlot::default(),
            ],
        },
    );
    presenter.draw(
        &queue,
        &mut encoder,
        &output_view,
        [SIZE, SIZE],
        PresentationOptions::default(),
    );
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &output,
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
        result.expect("map master blur readback");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    assert_eq!(&bytes[..4], &[255, 0, 0, 255]);
}

#[test]
fn feedback_uses_previous_final_frame_and_reset_discards_history() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let program = ProgramTarget::new(&device, [SIZE, SIZE]);
    let mut processor = MasterEffectProcessor::new(&device, &program);
    let presenter = ProgramPresenter::new(&device, &program, PROGRAM_FORMAT);
    let feedback = MasterEffectChain {
        slots: [
            MasterEffectSlot {
                kind: MasterEffectKind::Feedback,
                bypassed: false,
                mix: 1.0,
                amount: 8.0,
                feedback: 0.5,
                ..MasterEffectSlot::default()
            },
            MasterEffectSlot::default(),
        ],
    };

    let red = render_master_color(
        &device,
        &queue,
        &program,
        &mut processor,
        &presenter,
        wgpu::Color::RED,
        &feedback,
    );
    assert_eq!(red, [255, 0, 0, 255]);
    assert!(processor.history_is_valid());

    let trailed = render_master_color(
        &device,
        &queue,
        &program,
        &mut processor,
        &presenter,
        wgpu::Color {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        },
        &feedback,
    );
    assert!(trailed[0] > 100 && trailed[2] > 100, "{trailed:?}");

    processor.reset_history();
    let green = render_master_color(
        &device,
        &queue,
        &program,
        &mut processor,
        &presenter,
        wgpu::Color {
            r: 0.0,
            g: 1.0,
            b: 0.0,
            a: 1.0,
        },
        &feedback,
    );
    assert_eq!(green, [0, 255, 0, 255]);
}

#[test]
fn rejected_effect_reload_preserves_the_last_good_pipeline() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let directory =
        std::env::temp_dir().join(format!("oneiroi-effect-reload-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let manifest_path = directory.join("effect.json");
    fs::write(&manifest_path, "{not valid json").unwrap();

    let program = ProgramTarget::new(&device, [SIZE, SIZE]);
    let presenter = ProgramPresenter::new(&device, &program, PROGRAM_FORMAT);
    let mut processor = MasterEffectProcessor::new(&device, &program);
    processor.watch_effect_manifest(manifest_path);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !processor.poll_effect_reload() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        processor.reload_status().contains("using last known good"),
        "{}",
        processor.reload_status()
    );

    let missing_custom = MasterEffectChain {
        slots: [
            MasterEffectSlot {
                kind: MasterEffectKind::Custom,
                package_id: "missing-package".to_owned(),
                ..MasterEffectSlot::default()
            },
            MasterEffectSlot::default(),
        ],
    };
    let color = render_master_color(
        &device,
        &queue,
        &program,
        &mut processor,
        &presenter,
        wgpu::Color::GREEN,
        &missing_custom,
    );
    assert_eq!(color, [0, 255, 0, 255]);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn registered_custom_effect_compiles_and_runs_through_the_master_slot() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../effects/chromatic-split/effect.json");
    let program = ProgramTarget::new(&device, [SIZE, SIZE]);
    let presenter = ProgramPresenter::new(&device, &program, PROGRAM_FORMAT);
    let mut processor = MasterEffectProcessor::new(&device, &program);
    processor.watch_effect_manifest(manifest_path);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !processor.custom_effect_loaded("chromatic-split") && Instant::now() < deadline {
        processor.poll_effect_reload();
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        processor.custom_effect_loaded("chromatic-split"),
        "{}",
        processor.reload_status()
    );

    let chain = MasterEffectChain {
        slots: [
            MasterEffectSlot {
                kind: MasterEffectKind::Custom,
                package_id: "chromatic-split".to_owned(),
                parameters: vec![
                    EffectParameterValue {
                        id: "amount".to_owned(),
                        value: 0.02,
                    },
                    EffectParameterValue {
                        id: "angle".to_owned(),
                        value: 0.0,
                    },
                    EffectParameterValue {
                        id: "pulse".to_owned(),
                        value: 0.0,
                    },
                ],
                ..MasterEffectSlot::default()
            },
            MasterEffectSlot::default(),
        ],
    };
    let color = render_master_color(
        &device,
        &queue,
        &program,
        &mut processor,
        &presenter,
        wgpu::Color::GREEN,
        &chain,
    );
    assert_eq!(color, [0, 255, 0, 255]);
}

fn render_master_color(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    program: &ProgramTarget,
    processor: &mut MasterEffectProcessor,
    presenter: &ProgramPresenter,
    color: wgpu::Color,
    chain: &MasterEffectChain,
) -> [u8; 4] {
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("feedback-test-output"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: PROGRAM_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let output_view = output.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("feedback-test-readback"),
        size: u64::from(ROW_BYTES * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear-feedback-input"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: program.composition_view(),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    processor.draw(queue, &mut encoder, program, chain);
    presenter.draw(
        queue,
        &mut encoder,
        &output_view,
        [SIZE, SIZE],
        PresentationOptions::default(),
    );
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &output,
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
        result.expect("map feedback readback");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}
