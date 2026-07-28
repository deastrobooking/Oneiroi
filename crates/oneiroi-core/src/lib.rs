//! Clock, parameters, modulation, scene graph.
//!
//! This crate must stay free of GPU and I/O dependencies so it can be tested
//! on a machine with no display and no audio device.

pub mod clock;
pub mod media_time;

pub use clock::{Clock, FrameTime};
pub use media_time::{MediaTime, MediaTimeError};
