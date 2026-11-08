//! In-memory backend for tests, and for exercising the UI without touching real
//! devices (`AUDIO_INTERFACE_MOCK=1`).

use super::{AudioBackend, clamp_volume};
use crate::model::{Device, Direction, Transport};
use anyhow::{Result, bail};

#[derive(Default, Debug, Clone)]
pub struct MockBackend {
    pub devices: Vec<Device>,
    /// Every mutating call, for assertions: e.g. `"set_default input b"`.
    pub calls: Vec<String>,
}

impl MockBackend {
    pub fn new(devices: Vec<Device>) -> Self {
        MockBackend { devices, calls: Vec::new() }
    }

    /// A believable set of devices, including a Bluetooth headset.
    pub fn demo() -> Self {
        let dev = |id: &str, name: &str, direction, transport, is_default, hands_free| Device {
            id: id.into(),
            name: name.into(),
            direction,
            transport,
            is_default,
            volume: Some(0.75),
            muted: Some(false),
            hands_free,
        };
        MockBackend::new(vec![
            dev("builtin-mic", "Built-in Microphone", Direction::Input, Transport::BuiltIn, true, false),
            dev("bt-headset-in", "WH-1000XM4", Direction::Input, Transport::Bluetooth, false, true),
            dev("usb-mic", "USB Microphone", Direction::Input, Transport::Usb, false, false),
            dev("builtin-out", "Built-in Speakers", Direction::Output, Transport::BuiltIn, true, false),
            dev("bt-headset-out", "WH-1000XM4", Direction::Output, Transport::Bluetooth, false, false),
            dev("hdmi-out", "LG Monitor", Direction::Output, Transport::Hdmi, false, false),
        ])
    }

    fn find_mut(&mut self, id: &str, direction: Direction) -> Result<&mut Device> {
        match self.devices.iter_mut().find(|d| d.id == id && d.direction == direction) {
            Some(d) => Ok(d),
            None => bail!("no such {direction} device: {id}"),
        }
    }
}

impl AudioBackend for MockBackend {
    fn api_name(&self) -> &'static str {
        "Mock"
    }

    fn devices(&mut self) -> Result<Vec<Device>> {
        Ok(self.devices.clone())
    }

    fn set_default(&mut self, id: &str, direction: Direction) -> Result<()> {
        self.find_mut(id, direction)?;
        for d in self.devices.iter_mut().filter(|d| d.direction == direction) {
            d.is_default = d.id == id;
        }
        self.calls.push(format!("set_default {direction} {id}"));
        Ok(())
    }

    fn set_volume(&mut self, id: &str, direction: Direction, volume: f32) -> Result<()> {
        self.find_mut(id, direction)?.volume = Some(clamp_volume(volume));
        self.calls.push(format!("set_volume {direction} {id} {:.2}", clamp_volume(volume)));
        Ok(())
    }

    fn set_mute(&mut self, id: &str, direction: Direction, muted: bool) -> Result<()> {
        self.find_mut(id, direction)?.muted = Some(muted);
        self.calls.push(format!("set_mute {direction} {id} {muted}"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_set_default_is_exclusive() {
        let mut b = MockBackend::demo();
        b.set_default("usb-mic", Direction::Input).unwrap();
        let defaults: Vec<_> = b
            .devices()
            .unwrap()
            .into_iter()
            .filter(|d| d.is_default)
            .map(|d| d.id)
            .collect();
        assert_eq!(defaults, vec!["usb-mic", "builtin-out"]);
        assert!(b.set_default("usb-mic", Direction::Output).is_err());
    }

    #[test]
    fn mock_volume_clamps() {
        let mut b = MockBackend::demo();
        b.set_volume("usb-mic", Direction::Input, 1.7).unwrap();
        b.set_volume("builtin-mic", Direction::Input, f32::NAN).unwrap();
        let d = b.devices().unwrap();
        assert_eq!(d[2].volume, Some(1.0));
        assert_eq!(d[0].volume, Some(0.0));
    }
}
