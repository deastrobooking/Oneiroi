//! GPU device ownership, surface management, and render passes.
//!
//! Knows nothing about winit or egui: the surface is created from an opaque
//! `wgpu::SurfaceTarget`, so the windowing layer stays in `oneiroi-app`.

pub mod effect_manifest;
pub mod gpu;
pub mod graph_plan;
pub mod mixer;
pub mod program;
pub mod triangle;
pub mod upload;

pub use effect_manifest::{
    EFFECT_MANIFEST_FORMAT, EFFECT_MANIFEST_VERSION, EffectDescriptor, EffectHistoryResource,
    EffectManifest, EffectManifestError, EffectPackageRole, EffectParameterSchema,
    EffectPassSchema, EffectRegistry, EffectResourceSchema, MAX_EFFECT_PASSES,
    ValidatedEffectPackage, discover_effect_packages, load_effect_package,
};
pub use gpu::{Gpu, PresentSurface, SurfaceAcquireStatus, SurfaceAcquisition};
pub use graph_plan::{BuiltInRenderStage, FusedDeckNodes, LoweredPlanError, LoweredRenderPlan};
pub use mixer::{
    BlendModeGroup, DeckEffects, DeckLfos, DeckTransform, EFFECT_SLOTS_PER_DECK, EffectGroup,
    EffectLfo, EffectPreset, EffectSlot, EffectTarget, FourDeckCompositor, LayerBlendMode,
    LfoWaveform, MOD_ROUTES_PER_DECK, MixerBus, MixerParams, MixerUploadError, ModulationRoute,
    SourceMode,
};
pub use program::{
    EFFECT_PARAMETER_CAPACITY, EffectParameterValue, MASTER_EFFECT_SLOTS, MASTER_MODULATION_ROUTES,
    MASTER_MODULATION_SOURCES, MasterEffectChain, MasterEffectKind, MasterEffectProcessor,
    MasterEffectSlot, MasterLfo, MasterModulation, MasterModulationRoute, PROGRAM_FORMAT,
    PresentationOptions, ProgramPresenter, ProgramTarget,
};
pub use triangle::{Globals, TrianglePass};
pub use upload::{CompressedTexture, UploadError};
