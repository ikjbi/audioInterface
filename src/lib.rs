//! Audio Interface: a cross-platform audio device manager.
//!
//! * [`backend`]: native OS control (default device, volume, mute)
//! * [`meter`]: live level metering via capture streams
//! * [`levels`]: level math and meter ballistics
//! * [`service`]: background worker that ties it all together (gain lock, auto-switch)

pub mod backend;
pub mod bluetooth;
pub mod levels;
pub mod model;
