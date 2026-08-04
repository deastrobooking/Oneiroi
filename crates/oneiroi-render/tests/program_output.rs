use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use oneiroi_render::{
    EffectParameterValue, MasterEffectChain, MasterEffectKind, MasterEffectProcessor,
    MasterEffectSlot, PROGRAM_FORMAT, PresentationOptions, ProgramPresenter, ProgramTarget,
    load_effect_package,
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
    let bundled =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../effects/chromatic-split");
    fs::copy(bundled.join("effect.json"), &manifest_path).unwrap();
    fs::copy(
        bundled.join("chromatic_split.wgsl"),
        directory.join("chromatic_split.wgsl"),
    )
    .unwrap();

    let program = ProgramTarget::new(&device, [SIZE, SIZE]);
    let presenter = ProgramPresenter::new(&device, &program, PROGRAM_FORMAT);
    let mut processor = MasterEffectProcessor::new(&device, &program);
    processor.watch_effect_manifest(manifest_path.clone());
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

    fs::write(&manifest_path, "{not valid json").unwrap();
    processor.watch_effect_manifest(manifest_path);
    assert!(
        processor.custom_effect_loaded("chromatic-split"),
        "watching the same package discarded its last-known-good pipeline"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while !processor.reload_status().contains("using last known good") && Instant::now() < deadline
    {
        processor.poll_effect_reload();
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        processor.reload_status().contains("using last known good"),
        "{}",
        processor.reload_status()
    );
    assert!(processor.custom_effect_loaded("chromatic-split"));

    let loaded_custom = MasterEffectChain {
        slots: [
            MasterEffectSlot {
                kind: MasterEffectKind::Custom,
                package_id: "chromatic-split".to_owned(),
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
        &loaded_custom,
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

#[test]
fn bundled_algorithmic_effects_compile_and_render_through_the_master_slot() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let effect_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../effects");
    let packages = [
        ("recursive-2d", "recursive-2d/effect.json"),
        ("fractal-volume", "fractal-volume/effect.json"),
        ("hyper-recursion", "hyper-recursion/effect.json"),
    ];
    let manifest_paths: Vec<_> = packages
        .iter()
        .map(|(_, relative)| effect_root.join(relative))
        .collect();
    let program = ProgramTarget::new(&device, [SIZE, SIZE]);
    let presenter = ProgramPresenter::new(&device, &program, PROGRAM_FORMAT);
    let mut processor = MasterEffectProcessor::new(&device, &program);
    processor.watch_effect_manifests(manifest_paths.clone());
    let deadline = Instant::now() + Duration::from_secs(4);
    while packages
        .iter()
        .any(|(id, _)| !processor.custom_effect_loaded(id))
        && Instant::now() < deadline
    {
        processor.poll_effect_reload();
        thread::sleep(Duration::from_millis(10));
    }

    for ((id, _), manifest_path) in packages.iter().zip(&manifest_paths) {
        assert!(
            processor.custom_effect_loaded(id),
            "{id}: {}",
            processor.reload_status()
        );
        assert_eq!(processor.custom_effect_pass_count(id), Some(1), "{id}");
        let package = load_effect_package(manifest_path).unwrap();
        let parameters = package
            .manifest
            .parameters
            .iter()
            .map(|parameter| EffectParameterValue {
                id: parameter.id.clone(),
                value: parameter.default,
            })
            .collect();
        let slot = MasterEffectSlot {
            kind: MasterEffectKind::Custom,
            package_id: (*id).to_owned(),
            parameters,
            ..MasterEffectSlot::default()
        };
        let dry = MasterEffectChain {
            slots: [
                MasterEffectSlot {
                    mix: 0.0,
                    ..slot.clone()
                },
                MasterEffectSlot::default(),
            ],
        };
        assert_eq!(
            render_master_color(
                &device,
                &queue,
                &program,
                &mut processor,
                &presenter,
                wgpu::Color::GREEN,
                &dry,
            ),
            [0, 255, 0, 255],
            "{id} dry path is not identity"
        );
        let wet = MasterEffectChain {
            slots: [slot, MasterEffectSlot::default()],
        };
        let effected = render_master_color(
            &device,
            &queue,
            &program,
            &mut processor,
            &presenter,
            wgpu::Color::GREEN,
            &wet,
        );
        assert_ne!(effected, [0, 255, 0, 255], "{id} rendered as identity");
        assert_eq!(effected[3], 255, "{id} damaged output alpha");
    }
}

#[test]
fn registered_multipass_effect_compiles_as_one_atomic_pipeline_set() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../effects/spectral-echo/effect.json");
    let program = ProgramTarget::new(&device, [SIZE, SIZE]);
    let presenter = ProgramPresenter::new(&device, &program, PROGRAM_FORMAT);
    let mut processor = MasterEffectProcessor::new(&device, &program);
    processor.watch_effect_manifest(manifest_path);
    let deadline = Instant::now() + Duration::from_secs(2);
    while processor.custom_effect_pass_count("spectral-echo") != Some(2)
        && Instant::now() < deadline
    {
        processor.poll_effect_reload();
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        processor.custom_effect_pass_count("spectral-echo"),
        Some(2),
        "{}",
        processor.reload_status()
    );

    let chain = MasterEffectChain {
        slots: [
            MasterEffectSlot {
                kind: MasterEffectKind::Custom,
                package_id: "spectral-echo".to_owned(),
                parameters: vec![
                    EffectParameterValue {
                        id: "spread".to_owned(),
                        value: 0.02,
                    },
                    EffectParameterValue {
                        id: "echo".to_owned(),
                        value: 0.65,
                    },
                    EffectParameterValue {
                        id: "rotation".to_owned(),
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

#[test]
fn custom_history_seeds_blends_and_resets_deterministically() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../effects/temporal-melt/effect.json");
    let program = ProgramTarget::new(&device, [SIZE, SIZE]);
    let presenter = ProgramPresenter::new(&device, &program, PROGRAM_FORMAT);
    let mut processor = MasterEffectProcessor::new(&device, &program);
    processor.watch_effect_manifest(manifest_path);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !processor.custom_effect_loaded("temporal-melt") && Instant::now() < deadline {
        processor.poll_effect_reload();
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        processor.custom_effect_loaded("temporal-melt"),
        "{}",
        processor.reload_status()
    );
    let chain = MasterEffectChain {
        slots: [
            MasterEffectSlot {
                kind: MasterEffectKind::Custom,
                package_id: "temporal-melt".to_owned(),
                parameters: vec![
                    EffectParameterValue {
                        id: "persistence".to_owned(),
                        value: 0.5,
                    },
                    EffectParameterValue {
                        id: "drift".to_owned(),
                        value: 0.0,
                    },
                    EffectParameterValue {
                        id: "bleed".to_owned(),
                        value: 0.0,
                    },
                ],
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
        &chain,
    );
    assert_eq!(red, [255, 0, 0, 255]);
    assert!(processor.custom_history_is_valid(0));

    let melted = render_master_color(
        &device,
        &queue,
        &program,
        &mut processor,
        &presenter,
        wgpu::Color::BLUE,
        &chain,
    );
    assert!(melted[0] > 100 && melted[2] > 100, "{melted:?}");

    processor.reset_history();
    let green = render_master_color(
        &device,
        &queue,
        &program,
        &mut processor,
        &presenter,
        wgpu::Color::GREEN,
        &chain,
    );
    assert_eq!(green, [0, 255, 0, 255]);
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
