//! Platform-independent types shared by every backend, the CLI and the GUI.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Whether a device records (input / microphone) or plays (output / speakers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Input,
    Output,
}

impl Direction {
    pub const ALL: [Direction; 2] = [Direction::Input, Direction::Output];

    pub fn label(self) -> &'static str {
        match self {
            Direction::Input => "input",
            Direction::Output => "output",
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// How the device is connected to the computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    BuiltIn,
    Usb,
    Bluetooth,
    Hdmi,
    DisplayPort,
    Pci,
    Network,
    Virtual,
    Aggregate,
    #[default]
    Unknown,
}

impl Transport {
    pub fn label(self) -> &'static str {
        match self {
            Transport::BuiltIn => "built-in",
            Transport::Usb => "USB",
            Transport::Bluetooth => "Bluetooth",
            Transport::Hdmi => "HDMI",
            Transport::DisplayPort => "DisplayPort",
            Transport::Pci => "PCI",
            Transport::Network => "network",
            Transport::Virtual => "virtual",
            Transport::Aggregate => "aggregate",
            Transport::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One audio endpoint as seen by the OS.
///
/// A physical device that can both record and play (for example a Bluetooth
/// headset on macOS) shows up as two `Device`s, one per direction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Device {
    /// Stable native identifier: the endpoint ID on Windows, the device UID on
    /// macOS, and the sink/source name on PulseAudio. The same string is used by
    /// `cpal`, so it can be used to open capture streams.
    pub id: String,
    /// Human readable name, as shown in the OS sound settings.
    pub name: String,
    pub direction: Direction,
    pub transport: Transport,
    /// True if this is the system default for its direction.
    pub is_default: bool,
    /// Volume / gain as a scalar in `0.0..=1.0` (`None` = not adjustable).
    pub volume: Option<f32>,
    /// Mute state (`None` = device has no mute control).
    pub muted: Option<bool>,
    /// True when a Bluetooth device is in its low-quality "hands-free" /
    /// headset (HFP/HSP) mode, which is what happens when its mic is in use.
    pub hands_free: bool,
}

impl Device {
    pub fn key(&self) -> DeviceKey {
        DeviceKey { id: self.id.clone(), direction: self.direction }
    }
}

/// Uniquely identifies a device endpoint (id + direction).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeviceKey {
    pub id: String,
    pub direction: Direction,
}

/// A remembered reference to a device, used in settings (preferences, gain
/// locks). Bluetooth devices can change native ID between reconnects, so we
/// store the name too and fall back to matching on it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceRef {
    pub id: String,
    pub name: String,
}

impl DeviceRef {
    pub fn of(device: &Device) -> Self {
        DeviceRef { id: device.id.clone(), name: device.name.clone() }
    }

    /// Find the device this reference points to: exact ID first, then name.
    pub fn find<'a>(&self, devices: &'a [Device], direction: Direction) -> Option<&'a Device> {
        let candidates = || devices.iter().filter(move |d| d.direction == direction);
        candidates().find(|d| d.id == self.id).or_else(|| candidates().find(|d| d.name == self.name))
    }
}

/// Resolve a user-supplied string (from the CLI) to a device: exact ID, exact
/// name, then case-insensitive substring of the name. Errors if the substring
/// matches several devices.
pub fn resolve_device<'a>(devices: &'a [Device], direction: Direction, query: &str) -> anyhow::Result<&'a Device> {
    let in_dir: Vec<&Device> = devices.iter().filter(|d| d.direction == direction).collect();
    if let Some(d) = in_dir.iter().find(|d| d.id == query) {
        return Ok(d);
    }
    if let Some(d) = in_dir.iter().find(|d| d.name == query) {
        return Ok(d);
    }
    let q = query.to_lowercase();
    let matches: Vec<&&Device> = in_dir.iter().filter(|d| d.name.to_lowercase().contains(&q)).collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => anyhow::bail!("no {direction} device matches \"{query}\""),
        many => anyhow::bail!(
            "\"{query}\" matches several {direction} devices: {}",
            many.iter().map(|d| format!("\"{}\"", d.name)).collect::<Vec<_>>().join(", ")
        ),
    }
}

#[cfg(test)]
pub(crate) fn test_device(id: &str, name: &str, direction: Direction) -> Device {
    Device {
        id: id.into(),
        name: name.into(),
        direction,
        transport: Transport::Unknown,
        is_default: false,
        volume: Some(0.5),
        muted: Some(false),
        hands_free: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices() -> Vec<Device> {
        vec![
            test_device("a", "MacBook Pro Microphone", Direction::Input),
            test_device("b", "AirPods Pro", Direction::Input),
            test_device("c", "AirPods Pro", Direction::Output),
            test_device("d", "USB Microphone", Direction::Input),
        ]
    }

    #[test]
    fn resolve_by_id_name_and_substring() {
        let d = devices();
        assert_eq!(resolve_device(&d, Direction::Input, "b").unwrap().id, "b");
        assert_eq!(resolve_device(&d, Direction::Output, "AirPods Pro").unwrap().id, "c");
        assert_eq!(resolve_device(&d, Direction::Input, "airpods").unwrap().id, "b");
        assert_eq!(resolve_device(&d, Direction::Input, "usb").unwrap().id, "d");
    }

    #[test]
    fn resolve_ambiguous_and_missing() {
        let d = devices();
        let err = resolve_device(&d, Direction::Input, "micro").unwrap_err().to_string();
        assert!(err.contains("several"), "{err}");
        assert!(resolve_device(&d, Direction::Output, "usb").is_err());
    }

    #[test]
    fn device_ref_falls_back_to_name() {
        let d = devices();
        let r = DeviceRef { id: "stale-id".into(), name: "AirPods Pro".into() };
        assert_eq!(r.find(&d, Direction::Output).unwrap().id, "c");
        let exact = DeviceRef { id: "a".into(), name: "renamed".into() };
        assert_eq!(exact.find(&d, Direction::Input).unwrap().id, "a");
        assert!(r.find(&[], Direction::Input).is_none());
    }
}
