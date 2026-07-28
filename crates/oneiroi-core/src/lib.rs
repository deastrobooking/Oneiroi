//! Clock, parameters, modulation, scene graph.
//!
//! This crate must stay free of GPU and I/O dependencies so it can be tested
//! on a machine with no display and no audio device.

pub mod clock;

pub use clock::{Clock, FrameTime};
