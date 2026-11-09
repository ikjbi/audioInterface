//! Audio Interface: a cross-platform audio device manager.
//!
//! * [`backend`]: native OS control (default device, volume, mute)
//! * [`meter`]: live level metering via capture streams
//! * [`levels`]: level math and meter ballistics
//! * [`rules`]: gain lock and auto-switch decisions
//! * [`service`]: background worker that ties it all together
//! * [`gui`]: the egui desktop interface

pub mod analysis;
pub mod audio_tools;
pub mod autostart;
pub mod backend;
pub mod bluetooth;
pub mod desktop;
pub mod diagnostics;
pub mod gui;
pub mod gui_recordings;
pub mod help;
pub mod icon;
pub mod levels;
pub mod meter;
pub mod model;
pub mod profiles;
pub mod recorder;
pub mod rules;
pub mod service;
pub mod settings;
pub mod single_instance;
pub mod spectrum;
pub mod splash;
pub mod updater;
