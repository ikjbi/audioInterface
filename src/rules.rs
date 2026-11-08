//! Decision logic for the "smart" features, kept free of I/O so it can be
//! tested exhaustively. The service applies the returned actions.

use crate::model::{Device, DeviceKey, Direction};
use crate::settings::Settings;
use std::collections::{BTreeSet, HashMap};

/// Volume difference below which we consider a gain lock "held".
pub const GAIN_TOLERANCE: f32 = 0.01;

#[derive(Clone, Debug, PartialEq)]
pub struct GainCorrection {
    pub key: DeviceKey,
    pub name: String,
    pub from: f32,
    pub to: f32,
}

/// Devices whose volume drifted away from their locked value.
pub fn gain_corrections(devices: &[Device], settings: &Settings) -> Vec<GainCorrection> {
    devices
        .iter()
        .filter_map(|d| {
            let lock = settings.gain_lock_for(&d.key(), &d.name)?;
            let current = d.volume?;
            ((current - lock.volume).abs() > GAIN_TOLERANCE).then(|| GainCorrection {
                key: d.key(),
                name: d.name.clone(),
                from: current,
                to: lock.volume,
            })
        })
        .collect()
}

/// Gain-lock enforcement that copes with devices whose volume moves in steps.
///
/// Some hardware (many USB and Bluetooth devices on macOS) only accepts
/// discrete steps. Asked for 73% it settles on 75%. Without care we would
/// "restore" it every refresh forever. After a correction we check what the
/// device actually took. If that differs from the lock, it becomes the
/// accepted value for that lock.
#[derive(Default, Debug)]
pub struct GainLocker {
    /// key -> (locked value, value the device actually holds for it)
    settled: HashMap<DeviceKey, (f32, f32)>,
}

impl GainLocker {
    /// Corrections to apply now.
    pub fn corrections(&mut self, devices: &[Device], settings: &Settings) -> Vec<GainCorrection> {
        gain_corrections(devices, settings)
            .into_iter()
            .filter(|c| match self.settled.get(&c.key) {
                Some(&(lock, actual)) if (lock - c.to).abs() <= f32::EPSILON => {
                    (c.from - actual).abs() > GAIN_TOLERANCE
                }
                _ => true,
            })
            .collect()
    }

    /// Call with the device list re-read after applying `applied`. Returns the
    /// corrections the device could not hold exactly, with the value it settled on.
    pub fn observe(&mut self, applied: &[GainCorrection], after: &[Device]) -> Vec<(GainCorrection, f32)> {
        let mut quantized = Vec::new();
        for c in applied {
            let Some(now) = after.iter().find(|d| d.key() == c.key).and_then(|d| d.volume) else { continue };
            if (now - c.to).abs() > GAIN_TOLERANCE {
                self.settled.insert(c.key.clone(), (c.to, now));
                quantized.push((c.clone(), now));
            } else {
                self.settled.remove(&c.key);
            }
        }
        quantized
    }
}

/// The most preferred device that is currently present.
pub fn best_preferred<'a>(devices: &'a [Device], settings: &Settings, direction: Direction) -> Option<&'a Device> {
    settings.preferred(direction).iter().find_map(|r| r.find(devices, direction))
}

/// Watches the set of connected devices and decides when to switch defaults.
///
/// Switching only happens when the set of connected devices *changes* (or on the
/// first observation). If the user then picks another device by hand, we
/// leave their choice alone until something is plugged in or out.
#[derive(Default, Debug)]
pub struct AutoSwitcher {
    seen: [Option<BTreeSet<String>>; 2],
}

fn slot(direction: Direction) -> usize {
    match direction {
        Direction::Input => 0,
        Direction::Output => 1,
    }
}

impl AutoSwitcher {
    /// Returns the devices that should become the default.
    pub fn check(&mut self, devices: &[Device], settings: &Settings) -> Vec<DeviceKey> {
        let mut out = Vec::new();
        for direction in Direction::ALL {
            let present: BTreeSet<String> =
                devices.iter().filter(|d| d.direction == direction).map(|d| d.id.clone()).collect();
            let changed = self.seen[slot(direction)].as_ref() != Some(&present);
            self.seen[slot(direction)] = Some(present);
            if !changed || !settings.auto_switch {
                continue;
            }
            if let Some(best) = best_preferred(devices, settings, direction)
                && !best.is_default
            {
                out.push(best.key());
            }
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::model::{DeviceRef, test_device};

    fn dev(id: &str, dir: Direction, default: bool, vol: f32) -> Device {
        let mut d = test_device(id, &format!("name-{id}"), dir);
        d.is_default = default;
        d.volume = Some(vol);
        d
    }

    fn pref(id: &str) -> DeviceRef {
        DeviceRef { id: id.into(), name: format!("name-{id}") }
    }

    #[test]
    fn gain_lock_detects_drift_only() {
        let mut s = Settings::default();
        s.set_gain_lock(pref("mic"), Direction::Input, Some(0.8));
        let held = [dev("mic", Direction::Input, true, 0.805)];
        assert!(gain_corrections(&held, &s).is_empty());
        let drifted = [dev("mic", Direction::Input, true, 0.4), dev("other", Direction::Input, false, 0.1)];
        let c = gain_corrections(&drifted, &s);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].key.id, "mic");
        assert_eq!((c[0].from, c[0].to), (0.4, 0.8));
        // Same id, other direction: not locked.
        let out = [dev("mic", Direction::Output, true, 0.2)];
        assert!(gain_corrections(&out, &s).is_empty());
    }

    #[test]
    fn gain_locker_accepts_quantized_volume() {
        let mut s = Settings::default();
        s.set_gain_lock(pref("mic"), Direction::Input, Some(0.73));
        let mut locker = GainLocker::default();

        let before = [dev("mic", Direction::Input, true, 0.40)];
        let c = locker.corrections(&before, &s);
        assert_eq!(c.len(), 1);
        // The device snaps to 0.75 instead of 0.73.
        let after = [dev("mic", Direction::Input, true, 0.75)];
        let q = locker.observe(&c, &after);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].1, 0.75);
        // Next refresh: 0.75 counts as held, no endless corrections.
        assert!(locker.corrections(&after, &s).is_empty());
        // But a real change by another app is still corrected.
        let lowered = [dev("mic", Direction::Input, true, 0.20)];
        assert_eq!(locker.corrections(&lowered, &s).len(), 1);
        // A new lock value resets the accepted value.
        s.set_gain_lock(pref("mic"), Direction::Input, Some(0.5));
        assert_eq!(locker.corrections(&after, &s).len(), 1);
    }

    #[test]
    fn gain_locker_exact_devices_are_not_marked() {
        let mut s = Settings::default();
        s.set_gain_lock(pref("mic"), Direction::Input, Some(0.6));
        let mut locker = GainLocker::default();
        let c = locker.corrections(&[dev("mic", Direction::Input, true, 0.1)], &s);
        assert!(locker.observe(&c, &[dev("mic", Direction::Input, true, 0.6)]).is_empty());
    }

    #[test]
    fn auto_switch_on_connect_only() {
        let mut s = Settings::default();
        s.preferred_inputs = vec![pref("headset"), pref("usb")];
        let mut sw = AutoSwitcher::default();

        // Startup: builtin is default, usb present -> switch to usb (headset absent).
        let d1 = vec![dev("builtin", Direction::Input, true, 0.5), dev("usb", Direction::Input, false, 0.5)];
        assert_eq!(sw.check(&d1, &s), vec![DeviceKey { id: "usb".into(), direction: Direction::Input }]);

        // Nothing changed, user switched back to builtin by hand: leave it alone.
        let d2 = vec![dev("builtin", Direction::Input, true, 0.5), dev("usb", Direction::Input, false, 0.5)];
        assert!(sw.check(&d2, &s).is_empty());

        // Headset connects -> it wins.
        let mut d3 = d2.clone();
        d3.push(dev("headset", Direction::Input, false, 0.5));
        assert_eq!(sw.check(&d3, &s)[0].id, "headset");

        // Headset already default after a change -> nothing to do.
        let d4 = vec![dev("headset", Direction::Input, true, 0.5)];
        assert!(sw.check(&d4, &s).is_empty());
    }

    #[test]
    fn auto_switch_respects_toggle_and_empty_prefs() {
        let mut s = Settings::default();
        let mut sw = AutoSwitcher::default();
        let d = vec![dev("a", Direction::Output, true, 0.5), dev("b", Direction::Output, false, 0.5)];
        assert!(sw.check(&d, &s).is_empty(), "no preferences");
        s.preferred_outputs = vec![pref("b")];
        s.auto_switch = false;
        let mut sw = AutoSwitcher::default();
        assert!(sw.check(&d, &s).is_empty(), "disabled");
    }

    #[test]
    fn best_preferred_matches_by_name_after_id_change() {
        let mut s = Settings::default();
        s.preferred_outputs = vec![DeviceRef { id: "old-bt-id".into(), name: "name-new".into() }];
        let d = vec![dev("new", Direction::Output, false, 0.5)];
        assert_eq!(best_preferred(&d, &s, Direction::Output).unwrap().id, "new");
    }
}
