//! GPU device ownership, surface management, and render passes.
//!
//! Knows nothing about winit or egui: the surface is created from an opaque
//! `wgpu::SurfaceTarget`, so the windowing layer stays in `oneiroi-app`.

pub mod effect_manifest;
pub mod gpu;
pub mod mixer;
pub mod program;
pub mod triangle;
pub mod upload;

pub use effect_manifest::{
    EFFECT_MANIFEST_FORMAT, EFFECT_MANIFEST_VERSION, EffectDescriptor, EffectManifest,
    EffectManifestError, EffectPackageRole, EffectParameterSchema, EffectRegistry,
    ValidatedEffectPackage, discover_effect_packages, load_effect_package,
};
pub use gpu::{Gpu, PresentSurface, SurfaceAcquireStatus, SurfaceAcquisition};
pub use mixer::{
    DeckEffects, DeckLfos, DeckTransform, EFFECT_SLOTS_PER_DECK, EffectGroup, EffectLfo,
    EffectPreset, EffectSlot, EffectTarget, FourDeckCompositor, LayerBlendMode, LfoWaveform,
    MOD_ROUTES_PER_DECK, MixerBus, MixerParams, MixerUploadError, ModulationRoute, SourceMode,
};
pub use program::{
    EFFECT_PARAMETER_CAPACITY, EffectParameterValue, MASTER_EFFECT_SLOTS, MasterEffectChain,
    MasterEffectKind, MasterEffectProcessor, MasterEffectSlot, PROGRAM_FORMAT, PresentationOptions,
    ProgramPresenter, ProgramTarget,
};
pub use triangle::{Globals, TrianglePass};
pub use upload::{CompressedTexture, UploadError};
