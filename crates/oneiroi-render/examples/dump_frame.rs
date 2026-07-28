//! Render one frame headlessly and write raw RGBA to stdout.
//!
//! Useful for eyeballing shader changes without launching the app:
//!
//! ```sh
//! cargo run -p oneiroi-render --example dump_frame > frame.raw
//! ffmpeg -f rawvideo -pix_fmt rgba -s 512x512 -i frame.raw -y frame.png
//! ```

use std::io::Write;

use oneiroi_render::{Globals, TrianglePass};

const SIZE: u32 = 512;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn main() {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("dump-frame"),
        ..Default::default()
    }))
    .expect("no device");

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-target"),
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

    let bytes_per_row = SIZE * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dump-readback"),
        size: (bytes_per_row * SIZE) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let pass = TrianglePass::new(&device, FORMAT);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("dump-encoder"),
    });
    pass.draw(&queue, &mut encoder, &view, Globals::new(1.7, 0.6, 1.0));
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
        r.expect("map readback");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("poll");

    let pixels = readback.slice(..).get_mapped_range().to_vec();
    readback.unmap();

    std::io::stdout().write_all(&pixels).expect("write frame");
}
