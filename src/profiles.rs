//! Profiles: named snapshots of "how my audio should be set up" (default
//! devices, volumes, mutes, gain locks) that can be re-applied in one click,
//! e.g. "Calls" (headset mic, gain locked at 80 %) vs "Music" (speakers).

use crate::model::{Device, DeviceKey, DeviceRef, Direction};
use crate::settings::{GainLock, Settings};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceLevel {
    pub device: DeviceRef,
    pub direction: Direction,
    pub volume: Option<f32>,
    pub muted: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub default_input: Option<DeviceRef>,
    pub default_output: Option<DeviceRef>,
    /// Volume/mute per device (only devices present when the profile was saved).
    #[serde(default)]
    pub levels: Vec<DeviceLevel>,
    /// Gain locks to activate with this profile (replaces the current ones).
    #[serde(default)]
    pub gain_locks: Vec<GainLock>,
}

/// One change needed to apply a profile.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    SetDefault(DeviceKey),
    SetVolume(DeviceKey, f32),
    SetMute(DeviceKey, bool),
}

/// Capture the current setup as a profile.
pub fn capture(name: &str, devices: &[Device], settings: &Settings) -> Profile {
    let default_of = |dir: Direction| devices.iter().find(|d| d.direction == dir && d.is_default).map(DeviceRef::of);
    Profile {
        name: name.trim().to_string(),
        default_input: default_of(Direction::Input),
        default_output: default_of(Direction::Output),
        levels: devices
            .iter()
            .filter(|d| d.volume.is_some() || d.muted.is_some())
            .map(|d| DeviceLevel { device: DeviceRef::of(d), direction: d.direction, volume: d.volume, muted: d.muted })
            .collect(),
        gain_locks: settings.gain_locks.clone(),
    }
}

/// The minimal list of changes that makes `devices` match `profile`.
/// Devices from the profile that are not connected are skipped and listed in
/// the second return value.
pub fn plan(profile: &Profile, devices: &[Device]) -> (Vec<Action>, Vec<String>) {
    let mut actions = Vec::new();
    let mut missing = Vec::new();
    for (dir, r) in [(Direction::Input, &profile.default_input), (Direction::Output, &profile.default_output)] {
        let Some(r) = r else { continue };
        match r.find(devices, dir) {
            Some(d) if !d.is_default => actions.push(Action::SetDefault(d.key())),
            Some(_) => {}
            None => missing.push(r.name.clone()),
        }
    }
    for l in &profile.levels {
        let Some(d) = l.device.find(devices, l.direction) else { continue };
        if let (Some(want), Some(have)) = (l.volume, d.volume)
            && (want - have).abs() > 0.005
        {
            actions.push(Action::SetVolume(d.key(), want));
        }
        if let (Some(want), Some(have)) = (l.muted, d.muted)
            && want != have
        {
            actions.push(Action::SetMute(d.key(), want));
        }
    }
    (actions, missing)
}

impl Settings {
    /// Add or replace a profile by name (case-insensitive).
    pub fn save_profile(&mut self, profile: Profile) {
        self.profiles.retain(|p| !p.name.eq_ignore_ascii_case(&profile.name));
        self.profiles.push(profile);
    }

    pub fn delete_profile(&mut self, name: &str) {
        self.profiles.retain(|p| !p.name.eq_ignore_ascii_case(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::test_device;

    fn devs() -> Vec<Device> {
        let mut a = test_device("mic-a", "Headset Mic", Direction::Input);
        a.is_default = true;
        a.volume = Some(0.8);
        let b = test_device("mic-b", "Laptop Mic", Direction::Input);
        let mut o = test_device("spk", "Speakers", Direction::Output);
        o.is_default = true;
        o.muted = Some(false);
        vec![a, b, o]
    }

    #[test]
    fn capture_then_plan_is_a_noop() {
        let d = devs();
        let p = capture(" Calls ", &d, &Settings::default());
        assert_eq!(p.name, "Calls");
        assert_eq!(p.default_input.as_ref().unwrap().id, "mic-a");
        let (actions, missing) = plan(&p, &d);
        assert!(actions.is_empty(), "{actions:?}");
        assert!(missing.is_empty());
    }

    #[test]
    fn plan_restores_changes_and_reports_missing() {
        let p = capture("Calls", &devs(), &Settings::default());
        let mut now = devs();
        now[0].is_default = false; // someone switched to the laptop mic
        now[1].is_default = true;
        now[0].volume = Some(0.3); // and lowered the headset gain
        now[2].muted = Some(true);
        let (actions, _) = plan(&p, &now);
        assert_eq!(
            actions,
            vec![
                Action::SetDefault(DeviceKey { id: "mic-a".into(), direction: Direction::Input }),
                Action::SetVolume(DeviceKey { id: "mic-a".into(), direction: Direction::Input }, 0.8),
                Action::SetMute(DeviceKey { id: "spk".into(), direction: Direction::Output }, false),
            ]
        );
        // Headset unplugged: reported as missing, others still applied.
        let (actions, missing) = plan(&p, &now[1..]);
        assert_eq!(missing, vec!["Headset Mic".to_string()]);
        assert_eq!(actions, vec![Action::SetMute(DeviceKey { id: "spk".into(), direction: Direction::Output }, false)]);
    }

    #[test]
    fn save_replaces_by_name() {
        let mut s = Settings::default();
        s.save_profile(capture("Calls", &devs(), &Settings::default()));
        s.save_profile(capture("calls", &devs()[1..], &Settings::default()));
        assert_eq!(s.profiles.len(), 1);
        assert_eq!(s.profiles[0].name, "calls");
        s.delete_profile("CALLS");
        assert!(s.profiles.is_empty());
    }
}
