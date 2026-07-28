//! GPU device ownership, surface management, and render passes.
//!
//! Knows nothing about winit or egui: the surface is created from an opaque
//! `wgpu::SurfaceTarget`, so the windowing layer stays in `oneiroi-app`.

pub mod gpu;
pub mod mixer;
pub mod triangle;
pub mod upload;

pub use gpu::Gpu;
pub use mixer::{FourDeckCompositor, MixerParams, MixerUploadError};
pub use triangle::{Globals, TrianglePass};
pub use upload::{CompressedTexture, UploadError};
