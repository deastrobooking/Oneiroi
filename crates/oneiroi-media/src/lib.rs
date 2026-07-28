//! Clip decode: demux, HAP, ffmpeg fallbacks, frame ring buffers.
//!
//! Empty until milestone 2. Deliberately separate from `oneiroi-render` so the
//! decode side can hand over block-compressed bytes without knowing what a
//! `wgpu::Device` is.
