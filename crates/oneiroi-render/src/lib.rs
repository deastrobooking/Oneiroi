//! GPU device ownership, surface management, and render passes.
//!
//! Knows nothing about winit or egui: the surface is created from an opaque
//! `wgpu::SurfaceTarget`, so the windowing layer stays in `oneiroi-app`.

pub mod gpu;
pub mod triangle;

pub use gpu::Gpu;
pub use triangle::{Globals, TrianglePass};
