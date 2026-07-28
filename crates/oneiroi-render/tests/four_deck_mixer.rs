use oneiroi_media::{RgbaFrame, VideoFramePayload};
use oneiroi_render::{DeckEffects, FourDeckCompositor, MixerParams};

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
        data: pixel.repeat((SIZE * SIZE) as usize),
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
