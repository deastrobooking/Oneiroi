//! Project persistence and future live-control I/O adapters.

mod audio;
mod midi;
mod project;

pub use audio::{
    AudioInput, AudioInputDevice, AudioInputError, AudioInputSnapshot, discover_audio_inputs,
};
pub use midi::{
    MidiInputConnection, MidiInputDevice, MidiInputError, MidiInputEvent, MidiInputStats,
    discover_midi_inputs, parse_midi_message,
};
pub use project::{
    AudioAnalysisProject, BlendModeProject, CameraProject, ClipLaunchModeProject,
    ClipPlaybackProject, ControlTargetProject, CrossfadeBusProject, DeckProject,
    EffectGroupProject, EffectParameterValueProject, EffectProject, EffectSlotProject,
    EffectTargetProject, EndModeProject, LfoProject, LfoWaveformProject, MappingModeProject,
    MasterEffectKindProject, MasterEffectSlotProject, MasterEffectsProject, MidiMappingProject,
    MidiMessageProject, ModRouteProject, OutputProject, ProjectError, ProjectFile, ProjectSettings,
    QuantizationProject, SourceModeProject, TransformProject, TransportProject, autosave_path,
    load_project, recovery_is_newer, save_project_atomic,
};
