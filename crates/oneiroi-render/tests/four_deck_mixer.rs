use oneiroi_media::{RgbaFrame, VideoFramePayload};
use oneiroi_render::{
    DeckEffects, DeckPackageSlot, DeckTransform, EffectParameterValue, FourDeckCompositor,
    LayerBlendMode, MixerBus, MixerParams, SourceMode,
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
    render_with_packages(
        device,
        queue,
        compositor,
        params,
        &std::array::from_fn(|_| DeckPackageSlot::default()),
    )
}

fn render_with_packages(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    compositor: &mut FourDeckCompositor,
    params: MixerParams,
    packages: &[DeckPackageSlot; 4],
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
    compositor.draw_with_deck_packages(device, queue, &mut encoder, &view, params, packages);
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
fn deck_package_executes_before_blend_and_bypass_restores_the_fused_path() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut mixer = FourDeckCompositor::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    mixer.set_output_extent(&device, [SIZE, SIZE]);
    mixer.upload(&device, &queue, 0, &pattern()).unwrap();
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../effects/chromatic-split/effect.json");
    mixer.watch_deck_effect_manifests(vec![manifest]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !mixer.deck_effect_loaded("chromatic-split") && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
        mixer.poll_deck_effect_reload();
    }
    assert!(
        mixer.deck_effect_loaded("chromatic-split"),
        "{}",
        mixer.deck_effect_reload_status()
    );

    let baseline = render(&device, &queue, &mut mixer, MixerParams::default());
    let selected = DeckPackageSlot {
        package_id: "chromatic-split".to_owned(),
        parameters: vec![
            EffectParameterValue {
                id: "amount".to_owned(),
                value: 0.08,
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
        ..DeckPackageSlot::default()
    };
    let packages = std::array::from_fn(|index| {
        if index == 0 {
            selected.clone()
        } else {
            DeckPackageSlot::default()
        }
    });
    let effected = render_with_packages(
        &device,
        &queue,
        &mut mixer,
        MixerParams::default(),
        &packages,
    );
    assert_ne!(effected, baseline, "deck package did not alter its source");

    let mut bypassed = packages;
    bypassed[0].bypassed = true;
    let bypassed = render_with_packages(
        &device,
        &queue,
        &mut mixer,
        MixerParams::default(),
        &bypassed,
    );
    assert_eq!(bypassed, baseline, "bypass must retain the fused fast path");
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
fn effect_slots_apply_bypass_dry_wet_and_order() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut mixer = FourDeckCompositor::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    mixer.upload(&device, &queue, 0, &pattern()).unwrap();
    let render_effect = |mixer: &mut FourDeckCompositor, effect| {
        render(
            &device,
            &queue,
            mixer,
            MixerParams {
                effects: [
                    effect,
                    DeckEffects::default(),
                    DeckEffects::default(),
                    DeckEffects::default(),
                ],
                ..Default::default()
            },
        )
    };

    let base = render_effect(&mut mixer, DeckEffects::default());
    let full_effect = DeckEffects {
        saturation: 0.0,
        blacklight: 1.0,
        ..Default::default()
    };
    let full = render_effect(&mut mixer, full_effect);
    assert_ne!(full, base);

    let mut bypassed = full_effect;
    bypassed.slots[1].bypassed = true;
    bypassed.slots[2].bypassed = true;
    assert_eq!(render_effect(&mut mixer, bypassed), base);

    let mut dry = full_effect;
    dry.slots[1].mix = 0.0;
    dry.slots[2].mix = 0.0;
    assert_eq!(render_effect(&mut mixer, dry), base);

    let mut reordered = full_effect;
    reordered.slots.swap(1, 2);
    assert_ne!(render_effect(&mut mixer, reordered), full);
}

#[test]
fn luma_keyed_upper_layer_reveals_the_lower_layer() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut mixer = FourDeckCompositor::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    mixer
        .upload(&device, &queue, 0, &solid([255, 0, 0, 255]))
        .unwrap();
    mixer
        .upload(&device, &queue, 1, &solid([0, 0, 0, 255]))
        .unwrap();
    let base = MixerParams {
        levels: [1.0, 1.0, 0.0, 0.0],
        ..Default::default()
    };

    let opaque_upper = render(&device, &queue, &mut mixer, base);
    assert_eq!(&opaque_upper[..4], &[0, 0, 0, 255]);

    let keyed = render(
        &device,
        &queue,
        &mut mixer,
        MixerParams {
            effects: [
                DeckEffects::default(),
                DeckEffects {
                    luma_key: 0.5,
                    ..Default::default()
                },
                DeckEffects::default(),
                DeckEffects::default(),
            ],
            ..base
        },
    );
    assert_eq!(&keyed[..4], &[255, 0, 0, 255]);
}

#[test]
fn upper_layer_effects_are_applied_before_the_layer_blend() {
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
    let base = MixerParams {
        levels: [1.0, 1.0, 0.0, 0.0],
        blend_modes: [
            LayerBlendMode::Normal,
            LayerBlendMode::Difference,
            LayerBlendMode::Normal,
            LayerBlendMode::Normal,
        ],
        ..Default::default()
    };
    let ungraded = render(&device, &queue, &mut mixer, base);

    let graded = render(
        &device,
        &queue,
        &mut mixer,
        MixerParams {
            effects: [
                DeckEffects::default(),
                DeckEffects {
                    saturation: 0.0,
                    ..Default::default()
                },
                DeckEffects::default(),
                DeckEffects::default(),
            ],
            ..base
        },
    );
    assert_ne!(graded, ungraded);

    let gray = srgb_byte(0.7152);
    mixer
        .upload(&device, &queue, 1, &solid([gray, gray, gray, 255]))
        .unwrap();
    let pregraded_reference = render(&device, &queue, &mut mixer, base);
    for channel in 0..4 {
        assert!(
            graded[channel].abs_diff(pregraded_reference[channel]) <= 1,
            "channel {channel}: effect-before-blend was {}, pregraded reference was {}",
            graded[channel],
            pregraded_reference[channel]
        );
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

/// Encode a linear value the way an `Rgba8UnormSrgb` render target does, so
/// expectations below can be written in the space the shader actually works in.
fn srgb_byte(linear: f32) -> u8 {
    let clamped = linear.clamp(0.0, 1.0);
    let encoded = if clamped <= 0.0031308 {
        clamped * 12.92
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0 + 0.5) as u8
}

/// Red under green, which is the pair the original eight modes are checked
/// with. Every expectation is derived from the W3C formulas by hand.
const BLEND_EXPECTATIONS: [(LayerBlendMode, [f32; 3]); 35] = [
    (LayerBlendMode::Normal, [0.0, 1.0, 0.0]),
    (LayerBlendMode::Add, [1.0, 1.0, 0.0]),
    (LayerBlendMode::Screen, [1.0, 1.0, 0.0]),
    (LayerBlendMode::Multiply, [0.0, 0.0, 0.0]),
    (LayerBlendMode::Difference, [1.0, 1.0, 0.0]),
    (LayerBlendMode::Lighten, [1.0, 1.0, 0.0]),
    (LayerBlendMode::Darken, [0.0, 0.0, 0.0]),
    (LayerBlendMode::Overlay, [1.0, 0.0, 0.0]),
    (LayerBlendMode::ColorDodge, [1.0, 0.0, 0.0]),
    (LayerBlendMode::ColorBurn, [1.0, 0.0, 0.0]),
    (LayerBlendMode::HardLight, [0.0, 1.0, 0.0]),
    (LayerBlendMode::SoftLight, [1.0, 0.0, 0.0]),
    (LayerBlendMode::Exclusion, [1.0, 1.0, 0.0]),
    (LayerBlendMode::LinearBurn, [0.0, 0.0, 0.0]),
    (LayerBlendMode::VividLight, [1.0, 0.0, 0.0]),
    (LayerBlendMode::LinearLight, [0.0, 1.0, 0.0]),
    (LayerBlendMode::PinLight, [0.0, 1.0, 0.0]),
    (LayerBlendMode::HardMix, [1.0, 1.0, 0.0]),
    (LayerBlendMode::Subtract, [1.0, 0.0, 0.0]),
    (LayerBlendMode::Divide, [1.0, 0.0, 0.0]),
    // Non-separable: the layer's hue at the backdrop's luminosity, which for
    // red under green lands on a mid green.
    (LayerBlendMode::Hue, [0.0, 0.5085, 0.0]),
    (LayerBlendMode::Saturation, [1.0, 0.0, 0.0]),
    (LayerBlendMode::Color, [0.0, 0.5085, 0.0]),
    (LayerBlendMode::Luminosity, [1.0, 0.4143, 0.4143]),
    // Red is the darker of the two by the spec's luminance weights.
    (LayerBlendMode::DarkerColor, [1.0, 0.0, 0.0]),
    (LayerBlendMode::LighterColor, [0.0, 1.0, 0.0]),
    (LayerBlendMode::Negation, [1.0, 1.0, 0.0]),
    (LayerBlendMode::Invert, [0.41, 0.59, 0.59]),
    (LayerBlendMode::Reflect, [1.0, 0.0, 0.0]),
    (LayerBlendMode::Glow, [0.0, 1.0, 0.0]),
    (LayerBlendMode::Phoenix, [0.0, 0.0, 1.0]),
    // A 120 degree hue rotation carries red to green.
    (LayerBlendMode::HueShift, [0.0, 1.0, 0.0]),
    (LayerBlendMode::FractalFold, [0.0, 1.0, 1.0]),
    (LayerBlendMode::XorCrush, [1.0, 1.0, 0.0]),
    (LayerBlendMode::Solarize, [0.0, 0.0, 0.0]),
];

#[test]
fn every_blend_mode_matches_its_hand_derived_result() {
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

    // Every mode must be covered, so a new variant fails here until it has a
    // derived expectation rather than silently going untested.
    assert_eq!(BLEND_EXPECTATIONS.len(), LayerBlendMode::ALL.len());
    for mode in LayerBlendMode::ALL {
        assert!(
            BLEND_EXPECTATIONS.iter().any(|(listed, _)| *listed == mode),
            "{mode:?} has no expectation"
        );
    }

    for (mode, expected) in BLEND_EXPECTATIONS {
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
        let expected_bytes = expected.map(srgb_byte);
        for channel in 0..3 {
            let difference = i32::from(output[channel]) - i32::from(expected_bytes[channel]);
            assert!(
                difference.abs() <= 2,
                "{mode:?} channel {channel}: got {}, expected {} (linear {})",
                output[channel],
                expected_bytes[channel],
                expected[channel],
            );
        }
        assert_eq!(output[3], 255, "{mode:?} alpha");
    }
}

/// Blend codes are written into saved projects, so renumbering one would
/// silently repaint every show that used it.
#[test]
fn blend_mode_codes_and_names_are_stable_and_unique() {
    let mut codes: Vec<u32> = LayerBlendMode::ALL.iter().map(|mode| mode.code()).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), LayerBlendMode::ALL.len(), "duplicate codes");
    assert_eq!(codes.first().copied(), Some(0));
    assert_eq!(
        codes.last().copied(),
        Some(LayerBlendMode::ALL.len() as u32 - 1)
    );

    for mode in LayerBlendMode::ALL {
        assert_eq!(LayerBlendMode::from_name(mode.name()), Some(mode));
    }

    // The original eight predate this table and must keep their values.
    assert_eq!(LayerBlendMode::Normal.code(), 0);
    assert_eq!(LayerBlendMode::Add.code(), 1);
    assert_eq!(LayerBlendMode::Screen.code(), 2);
    assert_eq!(LayerBlendMode::Multiply.code(), 3);
    assert_eq!(LayerBlendMode::Difference.code(), 4);
    assert_eq!(LayerBlendMode::Lighten.code(), 5);
    assert_eq!(LayerBlendMode::Darken.code(), 6);
    assert_eq!(LayerBlendMode::Overlay.code(), 7);
}

const BLOOM_SIZE: u32 = 64;

/// Left half black, right half white: bloom has to carry light across the seam.
fn split_frame() -> VideoFramePayload {
    let mut data = Vec::with_capacity((BLOOM_SIZE * BLOOM_SIZE * 4) as usize);
    for _ in 0..BLOOM_SIZE {
        for x in 0..BLOOM_SIZE {
            let value = if x < BLOOM_SIZE / 2 { 0 } else { 255 };
            data.extend_from_slice(&[value, value, value, 255]);
        }
    }
    VideoFramePayload::Rgba8(RgbaFrame {
        extent: [BLOOM_SIZE, BLOOM_SIZE],
        data: data.into(),
    })
}

fn render_bloom(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    compositor: &mut FourDeckCompositor,
    params: MixerParams,
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bloom-test-target"),
        size: wgpu::Extent3d {
            width: BLOOM_SIZE,
            height: BLOOM_SIZE,
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
    // 64 px * 4 bytes is exactly the 256-byte copy alignment.
    let row_bytes = BLOOM_SIZE * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bloom-test-readback"),
        size: u64::from(row_bytes * BLOOM_SIZE),
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
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(BLOOM_SIZE),
            },
        },
        wgpu::Extent3d {
            width: BLOOM_SIZE,
            height: BLOOM_SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |result| {
        result.expect("map bloom readback");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let pixels = readback.slice(..).get_mapped_range().to_vec();
    readback.unmap();
    pixels
}

fn bloom_pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let index = ((y * BLOOM_SIZE + x) * 4) as usize;
    [
        pixels[index],
        pixels[index + 1],
        pixels[index + 2],
        pixels[index + 3],
    ]
}

#[test]
fn bloom_spreads_light_from_bright_regions_and_falls_off_with_distance() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut mixer = FourDeckCompositor::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    mixer.upload(&device, &queue, 0, &split_frame()).unwrap();

    let base = MixerParams {
        levels: [1.0, 0.0, 0.0, 0.0],
        ..Default::default()
    };

    let unlit = render_bloom(&device, &queue, &mut mixer, base);
    assert_eq!(
        bloom_pixel(&unlit, 28, 32)[0],
        0,
        "the dark half must stay black with bloom disabled"
    );

    let mut effects = DeckEffects {
        bloom: 1.0,
        bloom_threshold: 0.1,
        bloom_radius: 1.0,
        ..Default::default()
    };
    let lit = render_bloom(
        &device,
        &queue,
        &mut mixer,
        MixerParams {
            effects: [effects, effects, effects, effects],
            ..base
        },
    );

    let near = bloom_pixel(&lit, 28, 32)[0];
    let far = bloom_pixel(&lit, 2, 32)[0];
    assert!(near > 0, "bloom should carry light into the dark half");
    assert!(
        near > far,
        "bloom must fall off with distance: near {near}, far {far}"
    );
    assert_eq!(
        bloom_pixel(&lit, 60, 32)[0],
        255,
        "already-white pixels stay white"
    );

    // Chroma spreads red further than blue, so the fringe warms up.
    effects.bloom_chroma = 1.0;
    let chromatic = render_bloom(
        &device,
        &queue,
        &mut mixer,
        MixerParams {
            effects: [effects, effects, effects, effects],
            ..base
        },
    );
    let fringe = bloom_pixel(&chromatic, 28, 32);
    assert!(
        fringe[0] > fringe[2],
        "chromatic bloom should push red past blue: {fringe:?}"
    );
}
