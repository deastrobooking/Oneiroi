//! Headless proof that the render pipeline actually produces pixels.
//!
//! Draws into an offscreen texture and reads it back, so `cargo test` catches
//! shader and pipeline regressions without anyone having to look at a window.
//! Skips rather than fails where no adapter exists, so CI without a GPU stays
//! green.

use oneiroi_render::{Globals, TrianglePass};

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("test-device"),
        ..Default::default()
    }))
    .ok()
}

/// Render one frame and return it as RGBA8 rows.
fn render_frame(device: &wgpu::Device, queue: &wgpu::Queue, globals: Globals) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test-target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // 256 px * 4 bytes is already a multiple of the 256-byte row alignment
    // required by copy_texture_to_buffer, so no padding maths is needed.
    let bytes_per_row = SIZE * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test-readback"),
        size: (bytes_per_row * SIZE) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let pass = TrianglePass::new(device, FORMAT);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test-encoder"),
    });
    pass.draw(queue, &mut encoder, &view, globals);
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
                bytes_per_row: Some(bytes_per_row),
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

    readback.slice(..).map_async(wgpu::MapMode::Read, |r| {
        r.expect("map readback buffer");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("poll device to completion");

    let pixels = readback.slice(..).get_mapped_range().to_vec();
    readback.unmap();
    pixels
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

#[test]
fn triangle_fills_the_centre_and_leaves_the_corner_clear() {
    let Some((device, queue)) = headless_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // spin = 0 puts a vertex straight up, so the triangle is centred on the
    // origin and the corners fall outside it.
    let pixels = render_frame(&device, &queue, Globals::new(0.0, 0.0, 1.0));

    let centre = pixel(&pixels, SIZE / 2, SIZE / 2);
    let corner = pixel(&pixels, 2, 2);

    let brightness = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
    assert!(
        brightness(centre) > brightness(corner) + 90,
        "centre {centre:?} should be visibly brighter than the cleared corner {corner:?}"
    );
    assert_eq!(centre[3], 255, "centre should be opaque");
}

#[test]
fn spin_changes_the_image() {
    let Some((device, queue)) = headless_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Quarter turn: proves the uniform buffer actually reaches the shader
    // rather than the pipeline drawing the same triangle regardless.
    let still = render_frame(&device, &queue, Globals::new(0.0, 0.0, 1.0));
    let turned = render_frame(
        &device,
        &queue,
        Globals::new(1.0, std::f32::consts::FRAC_PI_2, 1.0),
    );

    assert_ne!(still, turned, "spin uniform had no effect on the output");
}
