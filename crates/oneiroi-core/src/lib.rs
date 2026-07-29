//! Clock, parameters, modulation, scene graph.
//!
//! This crate must stay free of GPU and I/O dependencies so it can be tested
//! on a machine with no display and no audio device.

pub mod clock;
pub mod control;
pub mod media_time;
pub mod tempo;

pub use clock::{Clock, FrameTime};
pub use control::{
    ControlTarget, ControlUpdate, MappingMode, MidiBinding, MidiMapper, MidiMessage,
    MidiMessageKind,
};
pub use media_time::{MediaTime, MediaTimeError};
pub use tempo::{Quantization, TapTempo, TempoClock};
