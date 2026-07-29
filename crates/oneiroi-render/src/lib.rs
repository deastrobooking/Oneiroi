//! GPU device ownership, surface management, and render passes.
//!
//! Knows nothing about winit or egui: the surface is created from an opaque
//! `wgpu::SurfaceTarget`, so the windowing layer stays in `oneiroi-app`.

pub mod gpu;
pub mod mixer;
pub mod program;
pub mod triangle;
pub mod upload;

pub use gpu::{Gpu, PresentSurface, SurfaceAcquireStatus, SurfaceAcquisition};
pub use mixer::{
    DeckEffects, DeckLfos, DeckTransform, EffectLfo, EffectTarget, FourDeckCompositor, LfoWaveform,
    MOD_ROUTES_PER_DECK, MixerBus, MixerParams, MixerUploadError, ModulationRoute,
};
pub use program::{PROGRAM_FORMAT, PresentationOptions, ProgramPresenter, ProgramTarget};
pub use triangle::{Globals, TrianglePass};
pub use upload::{CompressedTexture, UploadError};
