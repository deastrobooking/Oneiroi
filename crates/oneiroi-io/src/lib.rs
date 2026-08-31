//! Project persistence and future live-control I/O adapters.

mod audio;
mod midi;
mod midi_out;
mod project;

pub use audio::{
    AudioInput, AudioInputDevice, AudioInputError, AudioInputSnapshot, discover_audio_inputs,
};
pub use midi::{
    MidiInputConnection, MidiInputDevice, MidiInputError, MidiInputEvent, MidiInputMessage,
    MidiInputStats, discover_midi_inputs, parse_midi_input, parse_midi_message,
    parse_realtime_message,
};
pub use midi_out::{
    MidiClockSender, MidiOutputDevice, MidiOutputError, MidiOutputStats, discover_midi_outputs,
};
pub use project::{
    AudioAnalysisProject, BlendModeProject, CameraProject, ClipLaunchModeProject,
    ClipPlaybackProject, ClockSourceProject, ControlTargetProject, CrossfadeBusProject,
    DeckPackageModulationRouteProject, DeckPackageProject, DeckProject, EffectGroupProject,
    EffectParameterValueProject, EffectProject, EffectSlotProject, EffectTargetProject,
    EndModeProject, LfoProject, LfoWaveformProject, MappingModeProject, MasterEffectKindProject,
    MasterEffectSlotProject, MasterEffectsProject, MasterLfoProject, MasterModulationProject,
    MasterModulationRouteProject, MidiClockProject, MidiMappingProject, MidiMessageProject,
    ModRouteProject, OutputProject, PROJECT_VERSION, ProjectError, ProjectFile, ProjectSettings,
    QuantizationProject, SourceModeProject, TakeMetadataProject, ThemeProject, TransformProject,
    TransportProject, autosave_path, load_project, new_project_id, recovery_is_newer,
    save_project_atomic,
};
