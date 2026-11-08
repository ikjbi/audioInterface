//! Bluetooth heuristics.
//!
//! Classic Bluetooth headsets have two modes:
//! * **A2DP**: high-quality stereo playback, no microphone.
//! * **HFP/HSP** ("hands-free" / "headset"): the microphone works, but both mic and
//!   playback drop to narrow-band, phone-call quality.
//!
//! Using the headset's mic switches it into HFP, which is a common cause of
//! "my audio suddenly sounds terrible". This module detects the mode from the
//! hints each OS gives us, so the UI can warn about it.

use crate::model::Transport;

/// Hints from the platform's device names, PulseAudio properties, Windows
/// enumerator paths and so on.
#[derive(Default, Debug, Clone)]
pub struct Hints<'a> {
    pub name: &'a str,
    /// Windows: device instance path of the endpoint's parent (e.g. `BTHHFENUM\...`).
    pub instance_path: Option<&'a str>,
    /// PulseAudio/PipeWire: `device.bus`.
    pub bus: Option<&'a str>,
    /// PulseAudio/PipeWire: `bluetooth.protocol` / `api.bluez5.profile` / active profile name.
    pub profile: Option<&'a str>,
}

/// Best-effort transport detection from textual hints (used where the OS
/// does not report a transport type directly).
pub fn guess_transport(h: &Hints) -> Transport {
    let path = h.instance_path.unwrap_or("").to_ascii_uppercase();
    let bus = h.bus.unwrap_or("").to_ascii_lowercase();
    let name = h.name.to_ascii_lowercase();
    if path.contains("BTHENUM") || path.contains("BTHHFENUM") || path.contains("BTHLE") || bus == "bluetooth" {
        Transport::Bluetooth
    } else if path.starts_with("USB") || path.contains("\\USB") || path.contains("{1}.USB") || bus == "usb" {
        Transport::Usb
    } else if path.contains("HDAUDIO") || path.contains("INTELAUDIO") || bus == "pci" {
        if name.contains("hdmi") {
            Transport::Hdmi
        } else if name.contains("displayport") || name.contains("display port") {
            Transport::DisplayPort
        } else {
            Transport::Pci
        }
    } else if name.contains("bluetooth") || name.contains("hands-free") || name.contains("handsfree") {
        Transport::Bluetooth
    } else if name.contains("hdmi") {
        Transport::Hdmi
    } else if name.contains("usb") {
        Transport::Usb
    } else if name.contains("virtual") || name.contains("cable") || name.contains("blackhole") || name.contains("loopback") {
        Transport::Virtual
    } else {
        Transport::Unknown
    }
}

/// Is this (Bluetooth) endpoint in narrow-band hands-free/headset mode?
pub fn is_hands_free(transport: Transport, h: &Hints) -> bool {
    if transport != Transport::Bluetooth {
        return false;
    }
    let path = h.instance_path.unwrap_or("").to_ascii_uppercase();
    if path.contains("BTHHFENUM") {
        // Windows exposes HFP as its own "Hands-Free AG Audio" endpoint.
        return true;
    }
    if let Some(p) = h.profile {
        let p = p.to_ascii_lowercase();
        return p.contains("hfp") || p.contains("hsp") || p.contains("headset") || p.contains("handsfree") || p.contains("hands-free");
    }
    let name = h.name.to_ascii_lowercase();
    name.contains("hands-free") || name.contains("handsfree") || name.contains("headset head unit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths() {
        let hf = Hints {
            name: "Headset (WH-1000XM4 Hands-Free AG Audio)",
            instance_path: Some("{1}.BTHHFENUM\\{0000111E-0000-1000-8000-00805F9B34FB}_VID&..."),
            ..Default::default()
        };
        let t = guess_transport(&hf);
        assert_eq!(t, Transport::Bluetooth);
        assert!(is_hands_free(t, &hf));

        let a2dp = Hints {
            name: "Headphones (WH-1000XM4 Stereo)",
            instance_path: Some("{1}.BTHENUM\\{0000110B-0000-1000-8000-00805F9B34FB}"),
            ..Default::default()
        };
        let t = guess_transport(&a2dp);
        assert_eq!(t, Transport::Bluetooth);
        assert!(!is_hands_free(t, &a2dp));

        let usb = Hints { name: "Microphone (Yeti)", instance_path: Some("{1}.USB\\VID_046D"), ..Default::default() };
        assert_eq!(guess_transport(&usb), Transport::Usb);

        let hdmi = Hints { name: "LG HDMI", instance_path: Some("{1}.HDAUDIO\\FUNC_01"), ..Default::default() };
        assert_eq!(guess_transport(&hdmi), Transport::Hdmi);
    }

    #[test]
    fn pulse_props() {
        let src = Hints {
            name: "WH-1000XM4",
            bus: Some("bluetooth"),
            profile: Some("headset-head-unit-msbc"),
            ..Default::default()
        };
        let t = guess_transport(&src);
        assert_eq!(t, Transport::Bluetooth);
        assert!(is_hands_free(t, &src));

        let sink = Hints { name: "WH-1000XM4", bus: Some("bluetooth"), profile: Some("a2dp-sink"), ..Default::default() };
        assert!(!is_hands_free(Transport::Bluetooth, &sink));
    }

    #[test]
    fn name_fallbacks() {
        assert_eq!(guess_transport(&Hints { name: "BlackHole 2ch", ..Default::default() }), Transport::Virtual);
        assert_eq!(guess_transport(&Hints { name: "Speakers", ..Default::default() }), Transport::Unknown);
        assert!(!is_hands_free(Transport::Usb, &Hints { name: "Hands-Free", ..Default::default() }));
    }
}
