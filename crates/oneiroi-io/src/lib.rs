//! Project persistence and future live-control I/O adapters.

mod project;

pub use project::{
    CameraProject, ControlTargetProject, CrossfadeBusProject, DeckProject, EffectProject,
    EffectTargetProject, EndModeProject, LfoProject, LfoWaveformProject, MappingModeProject,
    MidiMappingProject, MidiMessageProject, ModRouteProject, OutputProject, ProjectError,
    ProjectFile, ProjectSettings, QuantizationProject, TransformProject, TransportProject,
    autosave_path, load_project, recovery_is_newer, save_project_atomic,
};
