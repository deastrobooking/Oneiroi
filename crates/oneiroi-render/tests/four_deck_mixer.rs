use oneiroi_media::{RgbaFrame, VideoFramePayload};
use oneiroi_render::{
    DeckEffects, DeckTransform, FourDeckCompositor, LayerBlendMode, MixerBus, MixerParams,
    SourceMode,
};

const SIZE: u32 = 4;
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

fn solid(pixel: [u8; 4]) -> VideoFramePayload {
    VideoFramePayload::Rgba8(RgbaFrame {
        extent: [SIZE, SIZE],
        data: pixel.repeat((SIZE * SIZE) as usize).into(),
    })
}

fn pattern() -> VideoFramePayload {
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            data.extend_from_slice(&[
                (x * 53 + y * 17) as u8,
                (x * 19 + y * 61) as u8,
                (x * 73 + y * 11) as u8,
                255,
            ]);
        }
    }
    VideoFramePayload::Rgba8(RgbaFrame {
        extent: [SIZE, SIZE],
        data: data.into(),
    })
}

fn wide_pattern() -> VideoFramePayload {
    let mut data = Vec::with_capacity((SIZE * 2 * 4) as usize);
    for y in 0..2 {
        for x in 0..SIZE {
            data.extend_from_slice(&[
                40 + x as u8 * 50,
                60 + y as u8 * 140,
                220 - x as u8 * 40,
                255,
            ]);
        }
    }
    VideoFramePayload::Rgba8(RgbaFrame {
        extent: [SIZE, 2],
        data: data.into(),
    })
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    compositor: &mut FourDeckCompositor,
    params: MixerParams,
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mixer-test-target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mixer-test-readback"),
        size: u64::from(ROW_BYTES * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    compositor.draw(device, queue, &mut encoder, &view, params);
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
        result.expect("map mixer readback");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    readback.slice(..).get_mapped_range().to_vec()
}

#[test]
fn composites_decks_in_linear_light_and_honors_blackout() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut mixer = FourDeckCompositor::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    mixer
        .upload(&device, &queue, 0, &solid([255, 0, 0, 255]))
        .unwrap();
    mixer
        .upload(&device, &queue, 1, &solid([0, 255, 0, 128]))
        .unwrap();

    let mixed = render(&device, &queue, &mut mixer, MixerParams::default());
    let pixel = &mixed[..4];
    assert!((180..=195).contains(&pixel[0]), "red was {}", pixel[0]);
    assert!((180..=195).contains(&pixel[1]), "green was {}", pixel[1]);
    assert!(pixel[2] < 5, "blue was {}", pixel[2]);

    let black = render(
        &device,
        &queue,
        &mut mixer,
        MixerParams {
            blackout: true,
            ..Default::default()
        },
    );
    assert_eq!(&black[..4], &[0, 0, 0, 255]);

    // Same-sized frames update the existing GPU texture allocation.
    mixer.clear_deck(1);
    mixer
        .upload(&device, &queue, 0, &solid([0, 0, 255, 255]))
        .unwrap();
    let blue = render(&device, &queue, &mut mixer, MixerParams::default());
    assert_eq!(&blue[..4], &[0, 0, 255, 255]);

    let monochrome = render(
        &device,
        &queue,
        &mut mixer,
        MixerParams {
            effects: std::array::from_fn(|index| {
                if index == 0 {
                    DeckEffects {
                        saturation: 0.0,
                        ..Default::default()
                    }
                } else {
                    DeckEffects::default()
                }
            }),
            ..Default::default()
        },
    );
    assert!(
        monochrome[0].abs_diff(monochrome[1]) <= 1 && monochrome[1].abs_diff(monochrome[2]) <= 1
    );
}

#[test]
fn composites_buses_independently_before_crossfading() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut mixer = FourDeckCompositor::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    mixer
        .upload(&device, &queue, 0, &solid([255, 0, 0, 255]))
        .unwrap();
    mixer
        .upload(&device, &queue, 1, &solid([0, 255, 0, 255]))
        .unwrap();
    let params = |crossfade_gains| MixerParams {
        levels: [1.0, 1.0, 0.0, 0.0],
        buses: [MixerBus::A, MixerBus::B, MixerBus::A, MixerBus::B],
        crossfade_gains,
        ..Default::default()
    };

    let bus_a = render(&device, &queue, &mut mixer, params([1.0, 0.0]));
    assert_eq!(&bus_a[..4], &[255, 0, 0, 255]);
    let bus_b = render(&device, &queue, &mut mixer, params([0.0, 1.0]));
    assert_eq!(&bus_b[..4], &[0, 255, 0, 255]);

    let center = render(&device, &queue, &mut mixer, params([0.5, 0.5]));
    assert!(
        (185..=190).contains(&center[0]) && (185..=190).contains(&center[1]),
        "linear-light center was {:?}",
        &center[..4]
    );
    assert!(center[2] < 5);
}

#[test]
fn blend_modes_match_known_opaque_primary_colors() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut mixer = FourDeckCompositor::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    mixer
        .upload(&device, &queue, 0, &solid([255, 0, 0, 255]))
        .unwrap();
    mixer
        .upload(&device, &queue, 1, &solid([0, 255, 0, 255]))
        .unwrap();
    let cases = [
        (LayerBlendMode::Normal, [0, 255, 0, 255]),
        (LayerBlendMode::Add, [255, 255, 0, 255]),
        (LayerBlendMode::Screen, [255, 255, 0, 255]),
        (LayerBlendMode::Multiply, [0, 0, 0, 255]),
        (LayerBlendMode::Difference, [255, 255, 0, 255]),
        (LayerBlendMode::Lighten, [255, 255, 0, 255]),
        (LayerBlendMode::Darken, [0, 0, 0, 255]),
        (LayerBlendMode::Overlay, [255, 0, 0, 255]),
    ];
    for (mode, expected) in cases {
        let output = render(
            &device,
            &queue,
            &mut mixer,
            MixerParams {
                levels: [1.0, 1.0, 0.0, 0.0],
                blend_modes: [
                    LayerBlendMode::Normal,
                    mode,
                    LayerBlendMode::Normal,
                    LayerBlendMode::Normal,
                ],
                ..Default::default()
            },
        );
        assert_eq!(&output[..4], &expected, "{mode:?}");
    }
}

#[test]
fn solo_isolates_decks_and_bypass_excludes_without_changing_level() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut mixer = FourDeckCompositor::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    mixer
        .upload(&device, &queue, 0, &solid([255, 0, 0, 255]))
        .unwrap();
    mixer
        .upload(&device, &queue, 1, &solid([0, 255, 0, 255]))
        .unwrap();
    let params = MixerParams {
        levels: [1.0, 1.0, 0.0, 0.0],
        ..Default::default()
    };

    let normal = render(&device, &queue, &mut mixer, params);
    assert_eq!(&normal[..4], &[0, 255, 0, 255]);

    let bypassed = render(
        &device,
        &queue,
        &mut mixer,
        MixerParams {
            bypassed: [false, true, false, false],
            ..params
        },
    );
    assert_eq!(&bypassed[..4], &[255, 0, 0, 255]);

    let solo_a = render(
        &device,
        &queue,
        &mut mixer,
        MixerParams {
            solo: [true, false, false, false],
            ..params
        },
    );
    assert_eq!(&solo_a[..4], &[255, 0, 0, 255]);

    let multi_solo = render(
        &device,
        &queue,
        &mut mixer,
        MixerParams {
            solo: [true, true, false, false],
            ..params
        },
    );
    assert_eq!(&multi_solo[..4], &[0, 255, 0, 255]);
}

#[test]
fn expanded_effects_each_change_a_patterned_source() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut mixer = FourDeckCompositor::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    mixer.upload(&device, &queue, 0, &pattern()).unwrap();
    let base = render(&device, &queue, &mut mixer, MixerParams::default());

    let effects = [
        (
            "mirror",
            DeckEffects {
                mirror: true,
                ..Default::default()
            },
        ),
        (
            "neon",
            DeckEffects {
                neon: 1.0,
                ..Default::default()
            },
        ),
        (
            "fractal",
            DeckEffects {
                fractal: 0.8,
                ..Default::default()
            },
        ),
        (
            "jitter",
            DeckEffects {
                jitter: 1.0,
                ..Default::default()
            },
        ),
        (
            "find edges",
            DeckEffects {
                find_edges: 1.0,
                ..Default::default()
            },
        ),
        (
            "bit reduction",
            DeckEffects {
                bit_reduction: 1.0,
                ..Default::default()
            },
        ),
        (
            "black light",
            DeckEffects {
                blacklight: 1.0,
                ..Default::default()
            },
        ),
        (
            "hue",
            DeckEffects {
                hue: 0.25,
                ..Default::default()
            },
        ),
        (
            "contrast",
            DeckEffects {
                contrast: 1.8,
                ..Default::default()
            },
        ),
        (
            "levels",
            DeckEffects {
                black_level: 0.2,
                white_level: 0.8,
                gamma: 1.5,
                ..Default::default()
            },
        ),
    ];
    for (label, effect) in effects {
        let changed = render(
            &device,
            &queue,
            &mut mixer,
            MixerParams {
                effects: [
                    effect,
                    DeckEffects::default(),
                    DeckEffects::default(),
                    DeckEffects::default(),
                ],
                time_seconds: 0.37,
                ..Default::default()
            },
        );
        assert_ne!(changed, base, "{label} did not change the rendered output");
    }
}

#[test]
fn layer_transforms_change_an_asymmetric_source() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut mixer = FourDeckCompositor::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    mixer.upload(&device, &queue, 0, &pattern()).unwrap();
    let base = render(&device, &queue, &mut mixer, MixerParams::default());
    let transforms = [
        (
            "position",
            DeckTransform {
                position: [0.5, -0.25],
                ..Default::default()
            },
        ),
        (
            "scale",
            DeckTransform {
                scale: 0.5,
                ..Default::default()
            },
        ),
        (
            "rotation",
            DeckTransform {
                rotation: 0.25,
                ..Default::default()
            },
        ),
        (
            "horizontal flip",
            DeckTransform {
                flip_horizontal: true,
                ..Default::default()
            },
        ),
        (
            "vertical flip",
            DeckTransform {
                flip_vertical: true,
                ..Default::default()
            },
        ),
    ];
    for (label, transform) in transforms {
        let changed = render(
            &device,
            &queue,
            &mut mixer,
            MixerParams {
                transforms: [
                    transform,
                    DeckTransform::default(),
                    DeckTransform::default(),
                    DeckTransform::default(),
                ],
                ..Default::default()
            },
        );
        assert_ne!(changed, base, "{label} did not change the rendered output");
    }
}

#[test]
fn crop_and_source_modes_use_the_source_aspect_ratio() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut mixer = FourDeckCompositor::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    mixer.upload(&device, &queue, 0, &wide_pattern()).unwrap();
    let transformed = |transform| MixerParams {
        transforms: [
            transform,
            DeckTransform::default(),
            DeckTransform::default(),
            DeckTransform::default(),
        ],
        output_aspect: 1.0,
        ..Default::default()
    };

    let stretch = render(
        &device,
        &queue,
        &mut mixer,
        transformed(DeckTransform::default()),
    );
    let fit = render(
        &device,
        &queue,
        &mut mixer,
        transformed(DeckTransform {
            source_mode: SourceMode::Fit,
            ..Default::default()
        }),
    );
    assert_eq!(&fit[..4], &[0, 0, 0, 255]);
    assert_ne!(fit, stretch);

    let fill = render(
        &device,
        &queue,
        &mut mixer,
        transformed(DeckTransform {
            source_mode: SourceMode::Fill,
            ..Default::default()
        }),
    );
    assert_ne!(fill, fit);
    assert_ne!(fill, stretch);

    let cropped = render(
        &device,
        &queue,
        &mut mixer,
        transformed(DeckTransform {
            crop: [0.4, 0.0, 0.0, 0.0],
            ..Default::default()
        }),
    );
    assert_ne!(cropped, stretch);
}
