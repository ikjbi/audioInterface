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
    /// Other applications currently recording from (input) or playing to
    /// (output) this device, as friendly names. Our own streams are excluded.
    #[serde(default)]
    pub in_use_by: Vec<String>,
    /// Windows only: this is the default *communications* device (used by
    /// Teams, Zoom, …), which can differ from the normal default.
    #[serde(default)]
    pub is_comms_default: bool,
    /// Current sample rate in Hz, if known.
    #[serde(default)]
    pub sample_rate: Option<u32>,
    /// Channel count, if known.
    #[serde(default)]
    pub channels: Option<u16>,
    /// Left/right balance, -1 (left) ..= 1 (right), for stereo devices with
    /// per-channel volume.
    #[serde(default)]
    pub balance: Option<f32>,
}

/// A hardware profile of the card behind a device (Linux/PulseAudio), e.g.
/// Bluetooth "A2DP" (high quality, no mic) vs "Headset" (HFP, mic + call quality).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub name: String,
    pub description: String,
    pub active: bool,
}

/// Balance from left/right channel volumes: -1 = only left, 0 = centred, 1 = only right.
pub fn balance_from(left: f32, right: f32) -> f32 {
    let m = left.max(right);
    if m <= 0.0 { 0.0 } else { ((right - left) / m).clamp(-1.0, 1.0) }
}

/// Left/right volumes for a master volume and balance (the louder side gets the master).
pub fn channel_gains(master: f32, balance: f32) -> (f32, f32) {
    let b = balance.clamp(-1.0, 1.0);
    (master * (1.0 - b.max(0.0)), master * (1.0 + b.min(0.0)))
}

/// Scale per-channel volumes so the loudest becomes `master`, keeping their ratios
/// (i.e. keeping the balance).
pub fn scale_channels(current: &[f32], master: f32) -> Vec<f32> {
    let m = current.iter().copied().fold(0.0f32, f32::max);
    if m <= 0.0 { vec![master; current.len()] } else { current.iter().map(|c| c / m * master).collect() }
}

impl Device {
    /// e.g. "48 kHz · 2 ch".
    pub fn format_label(&self) -> Option<String> {
        let rate = self
            .sample_rate
            .map(|r| if r % 1000 == 0 { format!("{} kHz", r / 1000) } else { format!("{:.1} kHz", r as f32 / 1000.0) });
        let ch = self.channels.map(|c| format!("{c} ch"));
        match (rate, ch) {
            (Some(r), Some(c)) => Some(format!("{r} · {c}")),
            (Some(r), None) => Some(r),
            (None, Some(c)) => Some(c),
            (None, None) => None,
        }
    }
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

/// Turn a raw app identifier (bundle ID, executable path, PulseAudio
/// application name) into something friendly, e.g. `com.microsoft.teams2` or
/// `C:\...\ms-teams.exe` -> "Microsoft Teams".
pub fn friendly_app_name(raw: &str) -> String {
    let raw = raw.trim();
    // Last path component, without extension.
    let base = raw.rsplit(['\\', '/']).next().unwrap_or(raw);
    let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
    let lower = raw.to_lowercase();
    const KNOWN: &[(&str, &str)] = &[
        ("teams", "Microsoft Teams"),
        ("zoom", "Zoom"),
        ("slack", "Slack"),
        ("discord", "Discord"),
        ("webex", "Webex"),
        ("skype", "Skype"),
        ("facetime", "FaceTime"),
        ("com.google.chrome", "Google Chrome"),
        ("chrome", "Google Chrome"),
        ("firefox", "Firefox"),
        ("msedge", "Microsoft Edge"),
        ("com.microsoft.edgemac", "Microsoft Edge"),
        ("safari", "Safari"),
        ("obsproject", "OBS"),
        ("obs64", "OBS"),
        ("audacity", "Audacity"),
        ("signal", "Signal"),
        ("whatsapp", "WhatsApp"),
        ("telegram", "Telegram"),
    ];
    for (needle, name) in KNOWN {
        if lower.contains(needle) {
            return (*name).to_string();
        }
    }
    // Bundle IDs: com.vendor.App -> App
    if !raw.contains(['/', '\\']) && raw.matches('.').count() >= 2 {
        return raw.rsplit('.').next().unwrap_or(raw).to_string();
    }
    base.to_string()
}

/// Extract the executable path from a Windows audio session identifier, e.g.
/// `{0.0.1.00000000}.{guid}|\Device\HarddiskVolume3\...\ms-teams.exe%b{guid}`.
pub fn exe_from_windows_session_id(id: &str) -> Option<String> {
    let after = id.split_once('|')?.1;
    let exe = after.split("%b").next()?.trim();
    (!exe.is_empty()).then(|| exe.to_string())
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
        in_use_by: Vec::new(),
        is_comms_default: false,
        sample_rate: None,
        channels: None,
        balance: None,
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
    fn balance_math() {
        assert_eq!(balance_from(0.8, 0.8), 0.0);
        assert!((balance_from(0.8, 0.4) - -0.5).abs() < 1e-6);
        assert_eq!(balance_from(0.0, 0.5), 1.0);
        assert_eq!(balance_from(0.0, 0.0), 0.0);
        assert_eq!(channel_gains(0.8, 0.0), (0.8, 0.8));
        assert_eq!(channel_gains(0.8, -0.5), (0.8, 0.4));
        assert_eq!(channel_gains(0.8, 1.0), (0.0, 0.8));
        let (l, r) = channel_gains(0.6, 0.25);
        assert!((balance_from(l, r) - 0.25).abs() < 1e-6, "round trip");
        assert_eq!(scale_channels(&[0.8, 0.4], 0.5), vec![0.5, 0.25]);
        assert_eq!(scale_channels(&[0.0, 0.0], 0.5), vec![0.5, 0.5]);
    }

    #[test]
    fn format_labels() {
        let mut d = test_device("x", "X", Direction::Input);
        assert_eq!(d.format_label(), None);
        d.sample_rate = Some(48_000);
        d.channels = Some(2);
        assert_eq!(d.format_label().unwrap(), "48 kHz · 2 ch");
        d.sample_rate = Some(44_100);
        d.channels = None;
        assert_eq!(d.format_label().unwrap(), "44.1 kHz");
    }

    #[test]
    fn windows_session_ids() {
        let id = r"{0.0.1.00000000}.{8b5bd1c5-46b5-4ae0-bb3d-1d1a3f6ed5b4}|\Device\HarddiskVolume3\Program Files\WindowsApps\MSTeams_24\ms-teams.exe%b{00000000-0000-0000-0000-000000000000}";
        let exe = exe_from_windows_session_id(id).unwrap();
        assert!(exe.ends_with("ms-teams.exe"), "{exe}");
        assert_eq!(friendly_app_name(&exe), "Microsoft Teams");
        assert_eq!(exe_from_windows_session_id("{0.0.1.00000000}.{guid}|#%b{guid}").as_deref(), Some("#"));
        assert_eq!(exe_from_windows_session_id("no separator"), None);
    }

    #[test]
    fn friendly_names() {
        assert_eq!(friendly_app_name("com.microsoft.teams2"), "Microsoft Teams");
        assert_eq!(
            friendly_app_name(r"\Device\HarddiskVolume3\Program Files\WindowsApps\MSTeams_x64\ms-teams.exe"),
            "Microsoft Teams"
        );
        assert_eq!(friendly_app_name("us.zoom.xos"), "Zoom");
        assert_eq!(friendly_app_name("com.example.Recorder"), "Recorder");
        assert_eq!(friendly_app_name(r"C:\Tools\mytool.exe"), "mytool");
        assert_eq!(friendly_app_name("parecord"), "parecord");
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
