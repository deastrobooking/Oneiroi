//! Render a bundled effect over a synthetic test chart to a 512x512 PPM.
//! `cargo run -p oneiroi-render --example effect_preview -- analog-crt > preview.ppm`

use std::io::Write;
use std::time::{Duration, Instant};

use oneiroi_media::{RgbaFrame, VideoFramePayload};
use oneiroi_render::{
    EffectParameterValue, FourDeckCompositor, MasterEffectChain, MasterEffectKind,
    MasterEffectProcessor, MasterEffectSlot, MixerParams, PROGRAM_FORMAT, PresentationOptions,
    ProgramPresenter, ProgramTarget, load_effect_package,
};

const SIZE: u32 = 512;

fn main() -> anyhow::Result<()> {
    let id = std::env::args().nth(1).unwrap_or_else(|| "none".to_owned());
    anyhow::ensure!(
        id.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        "expected a bundled package ID"
    );
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    let program = ProgramTarget::new(&device, [SIZE, SIZE]);
    let presenter = ProgramPresenter::new(&device, &program, PROGRAM_FORMAT);
    let mut processor = MasterEffectProcessor::new(&device, &program);
    let mut chain = MasterEffectChain::default();
    if id != "none" {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../effects")
            .join(&id)
            .join("effect.json");
        let package = load_effect_package(&path)?;
        processor.watch_effect_manifests(vec![path]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !processor.custom_effect_loaded(&id) && Instant::now() < deadline {
            processor.poll_effect_reload();
            std::thread::sleep(Duration::from_millis(10));
        }
        anyhow::ensure!(
            processor.custom_effect_loaded(&id),
            "{}",
            processor.reload_status()
        );
        chain.slots[0] = MasterEffectSlot {
            kind: MasterEffectKind::Custom,
            package_id: id,
            parameters: package
                .manifest
                .parameters
                .iter()
                .map(|p| EffectParameterValue {
                    id: p.id.clone(),
                    value: p.default,
                })
                .collect(),
            ..MasterEffectSlot::default()
        };
    }
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let u = x as f32 / SIZE as f32;
            let v = y as f32 / SIZE as f32;
            let ring = (((u - 0.5).hypot(v - 0.5) * 65.0).sin() * 0.5 + 0.5) * 0.4;
            let checker = if (x / 32 + y / 32) % 2 == 0 {
                0.16
            } else {
                0.0
            };
            let mut rgb = [
                0.08 + u * 0.65 + ring,
                0.1 + v * 0.65,
                0.2 + (1.0 - u) * 0.5 + checker,
            ];
            if (u - 0.3).hypot(v - 0.38) < 0.035 || (u - 0.72).hypot(v - 0.62) < 0.025 {
                rgb = [1.0; 3];
            }
            pixels.extend(rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8));
            pixels.push(255);
        }
    }
    let mut mixer = FourDeckCompositor::new(&device, &queue, PROGRAM_FORMAT);
    mixer.set_output_extent(&device, [SIZE, SIZE]);
    mixer.upload(
        &device,
        &queue,
        0,
        &VideoFramePayload::Rgba8(RgbaFrame {
            extent: [SIZE, SIZE],
            data: pixels.into(),
        }),
    )?;
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("effect-preview"),
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
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("effect-preview-readback"),
        size: u64::from(SIZE * SIZE * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    mixer.draw(
        &device,
        &queue,
        &mut encoder,
        program.composition_view(),
        MixerParams::default(),
    );
    processor.draw(&queue, &mut encoder, &program, &chain);
    presenter.draw(
        &queue,
        &mut encoder,
        &output.create_view(&Default::default()),
        [SIZE, SIZE],
        PresentationOptions::default(),
    );
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
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
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.expect("map preview"));
    device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    })?;
    let bytes = readback.slice(..).get_mapped_range();
    let mut stdout = std::io::BufWriter::new(std::io::stdout().lock());
    write!(stdout, "P6\n{SIZE} {SIZE}\n255\n")?;
    for pixel in bytes.chunks_exact(4) {
        stdout.write_all(&pixel[..3])?;
    }
    Ok(())
}
