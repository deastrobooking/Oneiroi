//! Upload of HAP's GPU-native block-compressed planes.

use oneiroi_hap::{CompressedPlaneFormat, DecodedPlane};
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum UploadError {
    #[error("adapter does not support BC texture compression")]
    BcTexturesUnsupported,
    #[error("plane data has {actual} bytes; expected {expected}")]
    DataSize { actual: usize, expected: usize },
    #[error("coded extent must contain the visible extent and be aligned to 4x4 blocks")]
    InvalidExtent,
    #[error("row layout cannot be represented by wgpu")]
    LayoutOverflow,
}

/// A sampled GPU texture retaining both coded and visible dimensions.
pub struct CompressedTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub format: CompressedPlaneFormat,
    pub coded_extent: [u32; 2],
    pub visible_extent: [u32; 2],
}

impl CompressedTexture {
    /// Create a BC texture and upload one decoded HAP plane without expanding
    /// it to RGBA on the CPU.
    pub fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        plane: &DecodedPlane,
        label: Option<&str>,
    ) -> Result<Self, UploadError> {
        if !device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
        {
            return Err(UploadError::BcTexturesUnsupported);
        }
        let [coded_width, coded_height] = plane.coded_extent;
        let [visible_width, visible_height] = plane.visible_extent;
        if coded_width == 0
            || coded_height == 0
            || coded_width % 4 != 0
            || coded_height % 4 != 0
            || visible_width == 0
            || visible_height == 0
            || visible_width > coded_width
            || visible_height > coded_height
        {
            return Err(UploadError::InvalidExtent);
        }

        let expected = plane
            .format
            .expected_bytes(visible_width, visible_height)
            .map_err(|_| UploadError::InvalidExtent)?;
        if plane.data.len() != expected {
            return Err(UploadError::DataSize {
                actual: plane.data.len(),
                expected,
            });
        }

        let format = wgpu_format(plane.format);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label,
            size: wgpu::Extent3d {
                width: coded_width,
                height: coded_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let bytes_per_row = (coded_width / 4)
            .checked_mul(plane.format.bytes_per_block() as u32)
            .ok_or(UploadError::LayoutOverflow)?;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &plane.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(coded_height / 4),
            },
            wgpu::Extent3d {
                width: coded_width,
                height: coded_height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Ok(Self {
            texture,
            view,
            format: plane.format,
            coded_extent: plane.coded_extent,
            visible_extent: plane.visible_extent,
        })
    }
}

fn wgpu_format(format: CompressedPlaneFormat) -> wgpu::TextureFormat {
    match format {
        CompressedPlaneFormat::Bc1Rgb => wgpu::TextureFormat::Bc1RgbaUnormSrgb,
        CompressedPlaneFormat::Bc3Rgba => wgpu::TextureFormat::Bc3RgbaUnormSrgb,
        // YCoCg channels are data, not RGB, and must not be hardware-decoded
        // through an sRGB transfer function before the conversion shader.
        CompressedPlaneFormat::Bc3ScaledYCoCg => wgpu::TextureFormat::Bc3RgbaUnorm,
        CompressedPlaneFormat::Bc4Alpha => wgpu::TextureFormat::Bc4RUnorm,
        CompressedPlaneFormat::Bc7Rgba => wgpu::TextureFormat::Bc7RgbaUnormSrgb,
        CompressedPlaneFormat::Bc6hUnsigned => wgpu::TextureFormat::Bc6hRgbUfloat,
        CompressedPlaneFormat::Bc6hSigned => wgpu::TextureFormat::Bc6hRgbFloat,
    }
}
