//! Persistent user settings, stored as JSON in the OS config directory:
//! * macOS: `~/Library/Application Support/com.ikjbi.AudioInterface/settings.json`
//! * Windows: `%APPDATA%\ikjbi\AudioInterface\config\settings.json`
//! * Linux: `~/.config/audiointerface/settings.json`

use crate::model::{DeviceKey, DeviceRef, Direction};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GainLock {
    pub device: DeviceRef,
    pub direction: Direction,
    /// Volume scalar 0..=1 to hold.
    pub volume: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Volumes to hold. If anything else changes them, we put them back.
    pub gain_locks: Vec<GainLock>,
    /// Priority-ordered preferred inputs (first = most preferred).
    pub preferred_inputs: Vec<DeviceRef>,
    /// Priority-ordered preferred outputs.
    pub preferred_outputs: Vec<DeviceRef>,
    /// Switch the default device automatically when a preferred one connects.
    pub auto_switch: bool,
    /// Always meter the current default input.
    pub monitor_default_input: bool,
    /// Also auto-meter the default input when it is a Bluetooth mic. Off by
    /// default because opening the mic forces the headset into hands-free mode.
    pub monitor_bluetooth_inputs: bool,
    /// Lowest dBFS value shown on the meters.
    pub meter_floor_db: f32,
    /// Hide virtual/aggregate devices in the list.
    pub hide_virtual: bool,
    /// Start recording an input automatically when another app starts using
    /// it (e.g. a Teams call), and stop when the app releases it.
    pub auto_record_calls: bool,
    /// Only auto-record for apps whose name contains one of these
    /// (case-insensitive). Empty = any app.
    pub auto_record_apps: Vec<String>,
    /// Include audio (WAV) in recordings by default.
    pub record_audio: bool,
    /// Where recordings are saved (`None` = Documents/Audio Interface Recordings).
    pub recordings_dir: Option<std::path::PathBuf>,
    /// Saved profiles (presets).
    pub profiles: Vec<crate::profiles::Profile>,
    /// User-chosen display names for devices.
    pub aliases: Vec<DeviceAlias>,
    /// Devices hidden from the list.
    pub hidden: Vec<HiddenDevice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceAlias {
    pub device: DeviceRef,
    pub direction: Direction,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HiddenDevice {
    pub device: DeviceRef,
    pub direction: Direction,
}

fn same(r: &DeviceRef, d: &crate::model::Device) -> bool {
    r.id == d.id || r.name == d.name
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            gain_locks: Vec::new(),
            preferred_inputs: Vec::new(),
            preferred_outputs: Vec::new(),
            auto_switch: true,
            monitor_default_input: true,
            monitor_bluetooth_inputs: false,
            meter_floor_db: -60.0,
            hide_virtual: false,
            auto_record_calls: false,
            auto_record_apps: Vec::new(),
            record_audio: false,
            recordings_dir: None,
            profiles: Vec::new(),
            aliases: Vec::new(),
            hidden: Vec::new(),
        }
    }
}

impl Settings {
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("com", "ikjbi", "AudioInterface").map(|d| d.config_dir().join("settings.json"))
    }

    /// Load settings. A missing file gives defaults. A corrupt file is backed up
    /// and replaced by defaults, so a bad edit never stops the app starting.
    pub fn load(path: &Path) -> Settings {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("settings file {} is invalid ({e}); using defaults", path.display());
                    let _ = std::fs::rename(path, path.with_extension("json.bak"));
                    Settings::default()
                }
            },
            Err(_) => Settings::default(),
        }
    }

    /// Save atomically (write a temp file, then rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Display name: the user's alias if set, else the OS name.
    pub fn display_name<'a>(&'a self, d: &'a crate::model::Device) -> &'a str {
        self.aliases
            .iter()
            .find(|a| a.direction == d.direction && same(&a.device, d))
            .map(|a| a.name.as_str())
            .unwrap_or(&d.name)
    }

    /// Set (or with an empty name, clear) a device alias.
    pub fn set_alias(&mut self, d: &crate::model::Device, name: &str) {
        self.aliases.retain(|a| !(a.direction == d.direction && same(&a.device, d)));
        let name = name.trim();
        if !name.is_empty() && name != d.name {
            self.aliases.push(DeviceAlias { device: DeviceRef::of(d), direction: d.direction, name: name.to_string() });
        }
    }

    pub fn is_hidden(&self, d: &crate::model::Device) -> bool {
        self.hidden.iter().any(|h| h.direction == d.direction && same(&h.device, d))
    }

    pub fn set_hidden(&mut self, d: &crate::model::Device, hide: bool) {
        self.hidden.retain(|h| !(h.direction == d.direction && same(&h.device, d)));
        if hide {
            self.hidden.push(HiddenDevice { device: DeviceRef::of(d), direction: d.direction });
        }
    }

    pub fn recordings_dir(&self) -> std::path::PathBuf {
        self.recordings_dir.clone().unwrap_or_else(crate::recorder::recordings_dir)
    }

    /// Should auto-record trigger for a device used by these apps?
    pub fn auto_record_matches(&self, apps: &[String]) -> bool {
        if apps.is_empty() {
            return false;
        }
        let filters: Vec<String> =
            self.auto_record_apps.iter().map(|f| f.trim().to_lowercase()).filter(|f| !f.is_empty()).collect();
        filters.is_empty() || apps.iter().any(|a| filters.iter().any(|f| a.to_lowercase().contains(f)))
    }

    pub fn preferred(&self, direction: Direction) -> &Vec<DeviceRef> {
        match direction {
            Direction::Input => &self.preferred_inputs,
            Direction::Output => &self.preferred_outputs,
        }
    }

    pub fn preferred_mut(&mut self, direction: Direction) -> &mut Vec<DeviceRef> {
        match direction {
            Direction::Input => &mut self.preferred_inputs,
            Direction::Output => &mut self.preferred_outputs,
        }
    }

    pub fn gain_lock_for(&self, key: &DeviceKey, name: &str) -> Option<&GainLock> {
        self.gain_locks
            .iter()
            .filter(|l| l.direction == key.direction)
            .find(|l| l.device.id == key.id)
            .or_else(|| self.gain_locks.iter().filter(|l| l.direction == key.direction).find(|l| l.device.name == name))
    }

    pub fn set_gain_lock(&mut self, device: DeviceRef, direction: Direction, volume: Option<f32>) {
        self.gain_locks
            .retain(|l| !(l.direction == direction && (l.device.id == device.id || l.device.name == device.name)));
        if let Some(volume) = volume {
            self.gain_locks.push(GainLock { device, direction, volume });
        }
    }

    /// Toggle a device's membership in the preference list (appended last).
    pub fn toggle_preferred(&mut self, device: DeviceRef, direction: Direction) {
        let list = self.preferred_mut(direction);
        if let Some(pos) = list.iter().position(|r| r.id == device.id || r.name == device.name) {
            list.remove(pos);
        } else {
            list.push(device);
        }
    }

    /// Move a preferred device up (`-1`) or down (`+1`) in priority.
    pub fn move_preferred(&mut self, direction: Direction, index: usize, delta: isize) {
        let list = self.preferred_mut(direction);
        let target = index as isize + delta;
        if index < list.len() && target >= 0 && (target as usize) < list.len() {
            list.swap(index, target as usize);
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    fn r(id: &str, name: &str) -> DeviceRef {
        DeviceRef { id: id.into(), name: name.into() }
    }

    #[test]
    fn roundtrip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
        let mut s = Settings::default();
        s.auto_switch = false;
        s.toggle_preferred(r("a", "Mic A"), Direction::Input);
        s.set_gain_lock(r("a", "Mic A"), Direction::Input, Some(0.8));
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
    }

    #[test]
    fn partial_and_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"auto_switch": false}"#).unwrap();
        let s = Settings::load(&path);
        assert!(!s.auto_switch);
        assert!(s.monitor_default_input, "missing fields take defaults");

        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        assert!(path.with_extension("json.bak").exists(), "corrupt file is backed up");
    }

    #[test]
    fn preferences_toggle_and_reorder() {
        let mut s = Settings::default();
        s.toggle_preferred(r("a", "A"), Direction::Output);
        s.toggle_preferred(r("b", "B"), Direction::Output);
        s.toggle_preferred(r("c", "C"), Direction::Output);
        s.move_preferred(Direction::Output, 2, -1);
        let ids: Vec<_> = s.preferred_outputs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["a", "c", "b"]);
        s.move_preferred(Direction::Output, 0, -1); // no-op at the top
        s.move_preferred(Direction::Output, 2, 1); // no-op at the bottom
        s.toggle_preferred(r("other-id", "C"), Direction::Output); // removes by name
        let ids: Vec<_> = s.preferred_outputs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn aliases_and_hidden_devices() {
        let mut s = Settings::default();
        let d = crate::model::test_device("x", "Headset (WH-1000XM4 Hands-Free AG Audio)", Direction::Input);
        assert_eq!(s.display_name(&d), d.name);
        s.set_alias(&d, "  Headset mic ");
        assert_eq!(s.display_name(&d), "Headset mic");
        s.set_alias(&d, "");
        assert_eq!(s.display_name(&d), d.name, "empty name clears the alias");
        assert!(!s.is_hidden(&d));
        s.set_hidden(&d, true);
        s.set_hidden(&d, true);
        assert!(s.is_hidden(&d));
        assert_eq!(s.hidden.len(), 1);
        s.set_hidden(&d, false);
        assert!(!s.is_hidden(&d));
    }

    #[test]
    fn auto_record_app_filter() {
        let mut s = Settings::default();
        assert!(!s.auto_record_matches(&[]));
        assert!(s.auto_record_matches(&["Zoom".into()]), "empty filter = any app");
        s.auto_record_apps = vec!["teams".into(), " ".into()];
        assert!(s.auto_record_matches(&["Microsoft Teams".into()]));
        assert!(!s.auto_record_matches(&["Zoom".into()]));
    }

    #[test]
    fn gain_lock_lookup_by_id_or_name() {
        let mut s = Settings::default();
        s.set_gain_lock(r("bt-1", "Headset"), Direction::Input, Some(0.9));
        let key = DeviceKey { id: "bt-2".into(), direction: Direction::Input };
        assert_eq!(s.gain_lock_for(&key, "Headset").unwrap().volume, 0.9);
        let out_key = DeviceKey { id: "bt-1".into(), direction: Direction::Output };
        assert!(s.gain_lock_for(&out_key, "Headset").is_none());
        s.set_gain_lock(r("bt-2", "Headset"), Direction::Input, Some(0.5));
        assert_eq!(s.gain_locks.len(), 1, "replaces the existing lock");
        s.set_gain_lock(r("bt-2", "Headset"), Direction::Input, None);
        assert!(s.gain_locks.is_empty());
    }
}
