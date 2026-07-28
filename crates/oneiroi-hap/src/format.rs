use crate::HapError;

/// GPU texture block format stored by a HAP plane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompressedPlaneFormat {
    Bc1Rgb,
    Bc3Rgba,
    Bc3ScaledYCoCg,
    Bc4Alpha,
    Bc7Rgba,
    Bc6hUnsigned,
    Bc6hSigned,
}

impl CompressedPlaneFormat {
    pub fn bytes_per_block(self) -> usize {
        match self {
            Self::Bc1Rgb | Self::Bc4Alpha => 8,
            Self::Bc3Rgba
            | Self::Bc3ScaledYCoCg
            | Self::Bc7Rgba
            | Self::Bc6hUnsigned
            | Self::Bc6hSigned => 16,
        }
    }

    pub fn expected_bytes(self, width: u32, height: u32) -> Result<usize, HapError> {
        let blocks_x = width.checked_add(3).ok_or(HapError::DimensionOverflow)? / 4;
        let blocks_y = height.checked_add(3).ok_or(HapError::DimensionOverflow)? / 4;
        usize::try_from(blocks_x)
            .ok()
            .and_then(|x| {
                usize::try_from(blocks_y)
                    .ok()
                    .and_then(|y| x.checked_mul(y))
            })
            .and_then(|blocks| blocks.checked_mul(self.bytes_per_block()))
            .ok_or(HapError::FrameSizeOverflow)
    }

    pub(crate) fn from_raw(raw: u32) -> Result<Self, HapError> {
        use oneiroi_hap_sys as sys;
        match raw {
            sys::HAP_TEXTURE_FORMAT_RGB_DXT1 => Ok(Self::Bc1Rgb),
            sys::HAP_TEXTURE_FORMAT_RGBA_DXT5 => Ok(Self::Bc3Rgba),
            sys::HAP_TEXTURE_FORMAT_YCOCG_DXT5 => Ok(Self::Bc3ScaledYCoCg),
            sys::HAP_TEXTURE_FORMAT_A_RGTC1 => Ok(Self::Bc4Alpha),
            sys::HAP_TEXTURE_FORMAT_RGBA_BPTC_UNORM => Ok(Self::Bc7Rgba),
            sys::HAP_TEXTURE_FORMAT_RGB_BPTC_UNSIGNED_FLOAT => Ok(Self::Bc6hUnsigned),
            sys::HAP_TEXTURE_FORMAT_RGB_BPTC_SIGNED_FLOAT => Ok(Self::Bc6hSigned),
            value => Err(HapError::UnsupportedTextureFormat(value)),
        }
    }
}
