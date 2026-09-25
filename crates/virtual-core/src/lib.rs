//! Clock, parameters, modulation, scene graph.
//!
//! This crate must stay free of GPU and I/O dependencies so it can be tested
//! on a machine with no display and no audio device.

pub mod audio;
pub mod audio_map;
pub mod automation;
pub mod clock;
pub mod control;
pub mod media_time;
pub mod midi_clock;
pub mod tempo;

pub use audio::{
    AUDIO_ANALYSIS_SIZE, AUDIO_MOD_SOURCES, AudioAnalysisSettings, AudioAnalyzer, AudioSnapshot,
    SPECTRUM_ANALYSIS_SIZE, SPECTRUM_BAND_EDGES_HZ, SPECTRUM_BAND_LABELS, SPECTRUM_BANDS,
};
pub use audio_map::{
    AUDIO_MAP_LEVEL, AUDIO_MAP_SOURCES, AUDIO_MAP_TRANSIENT, AudioBinding, AudioMapMode,
    AudioMapper, MAX_AUDIO_BINDINGS, audio_map_source_label, audio_map_sources,
    default_output_range,
};
pub use automation::{
    AutomationKeyframe, ClipAutomation, ClipAutomationLane, CurveType, MAX_AUTOMATION_KEYFRAMES,
    MAX_AUTOMATION_LANES, clip_position,
};
pub use clock::{Clock, FrameTime};
pub use control::{
    ControlTarget, ControlUpdate, FIXED_DECK_EFFECT_PARAMETER_COUNT, MappingMode, MidiBinding,
    MidiMapper, MidiMessage, MidiMessageKind, effect_parameter_key,
};
pub use media_time::{MediaTime, MediaTimeError};
pub use midi_clock::{
    ClockSource, MidiClockFollower, MidiClockGenerator, MidiClockUpdate, MidiRealtime,
    PULSES_PER_QUARTER_NOTE,
};
pub use tempo::{Quantization, TapTempo, TempoClock};
