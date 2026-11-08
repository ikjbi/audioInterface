//! OS integration: listing devices and changing system defaults and volumes.
//!
//! `cpal` (used for metering) can't change system settings, so each OS gets
//! a small native backend that implements [`AudioBackend`].

use crate::model::{Device, Direction};
use anyhow::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
pub mod mock;
#[cfg(windows)]
mod windows;

/// Native control over the OS audio device configuration.
///
/// Implementations are used from a single worker thread and don't need to be `Send`
/// (COM objects on Windows, for example, are thread-affine).
pub trait AudioBackend {
    /// Short name of the underlying API, e.g. `"CoreAudio"`.
    fn api_name(&self) -> &'static str;

    /// All active devices, inputs and outputs.
    fn devices(&mut self) -> Result<Vec<Device>>;

    /// Make `id` the system default device for `direction`.
    fn set_default(&mut self, id: &str, direction: Direction) -> Result<()>;

    /// Whether the OS has a separate default device for communications (Windows).
    fn supports_comms_role(&self) -> bool {
        false
    }

    /// Make `id` the default *communications* device only (Windows).
    fn set_comms_default(&mut self, _id: &str, _direction: Direction) -> Result<()> {
        anyhow::bail!("this OS has no separate communications device")
    }

    /// Sample rates the device supports switching to (empty = can't be changed here).
    fn sample_rates(&mut self, _id: &str, _direction: Direction) -> Vec<u32> {
        Vec::new()
    }

    /// Change the device sample rate (macOS).
    fn set_sample_rate(&mut self, _id: &str, _direction: Direction, _rate: u32) -> Result<()> {
        anyhow::bail!("changing the sample rate is not supported on this OS")
    }

    /// Set volume/gain as a scalar in `0.0..=1.0`.
    fn set_volume(&mut self, id: &str, direction: Direction, volume: f32) -> Result<()>;

    fn set_mute(&mut self, id: &str, direction: Direction, muted: bool) -> Result<()>;

    /// Native peak meter that works without opening a capture stream, if the OS
    /// has one (Windows `IAudioMeterInformation`). Returns linear peak 0..=1.
    fn native_peak(&mut self, _id: &str, _direction: Direction) -> Option<f32> {
        None
    }

    /// Whether [`AudioBackend::native_peak`] is meaningful for this direction.
    fn has_native_meter(&self, _direction: Direction) -> bool {
        false
    }

    /// The `cpal` device ID to capture from to meter this device, if any.
    /// For inputs that is the device itself. For outputs, a loopback/monitor
    /// source where the OS has one (PulseAudio `*.monitor`).
    fn capture_id(&self, id: &str, direction: Direction) -> Option<String> {
        match direction {
            Direction::Input => Some(id.to_string()),
            Direction::Output => None,
        }
    }
}

/// Create the backend for the current OS.
pub fn create() -> Result<Box<dyn AudioBackend>> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::CoreAudioBackend::new()))
    }
    #[cfg(windows)]
    {
        Ok(Box::new(windows::WasapiBackend::new()?))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(linux::PulseBackend::connect()?))
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        anyhow::bail!("this operating system is not supported yet")
    }
}

/// Clamp and sanitise a volume scalar.
pub(crate) fn clamp_volume(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}
