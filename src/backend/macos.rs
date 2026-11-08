//! macOS backend: CoreAudio's `AudioObject*` property API.
//!
//! The handful of C functions and constants we need are declared here directly.
//! They are stable, documented CoreAudio APIs, and declaring them ourselves keeps
//! the dependency surface small.

use super::{AudioBackend, clamp_volume};
use crate::bluetooth;
use crate::model::{Device, Direction, Transport};
use anyhow::{Result, anyhow, bail};
use std::ffi::{c_char, c_void};
use std::ptr;

type AudioObjectID = u32;
type OSStatus = i32;
type CFStringRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectHasProperty(id: AudioObjectID, addr: *const AudioObjectPropertyAddress) -> u8;
    fn AudioObjectIsPropertySettable(
        id: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
        out_settable: *mut u8,
    ) -> OSStatus;
    fn AudioObjectGetPropertyDataSize(
        id: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        out_size: *mut u32,
    ) -> OSStatus;
    fn AudioObjectGetPropertyData(
        id: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        io_size: *mut u32,
        out_data: *mut c_void,
    ) -> OSStatus;
    fn AudioObjectSetPropertyData(
        id: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringGetLength(s: CFStringRef) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFRelease(cf: *const c_void);
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

const SYSTEM_OBJECT: AudioObjectID = 1;
const ELEMENT_MAIN: u32 = 0;
const SCOPE_GLOBAL: u32 = fourcc(b"glob");
const SCOPE_INPUT: u32 = fourcc(b"inpt");
const SCOPE_OUTPUT: u32 = fourcc(b"outp");

const HW_DEVICES: u32 = fourcc(b"dev#");
const HW_DEFAULT_INPUT: u32 = fourcc(b"dIn ");
const HW_DEFAULT_OUTPUT: u32 = fourcc(b"dOut");
const HW_DEFAULT_SYSTEM_OUTPUT: u32 = fourcc(b"sOut");
const OBJ_NAME: u32 = fourcc(b"lnam");
const DEV_UID: u32 = fourcc(b"uid ");
const DEV_TRANSPORT: u32 = fourcc(b"tran");
const DEV_STREAMS: u32 = fourcc(b"stm#");
const DEV_VOLUME_SCALAR: u32 = fourcc(b"volm");
const DEV_MUTE: u32 = fourcc(b"mute");
const DEV_STEREO_CHANNELS: u32 = fourcc(b"dch2");
const DEV_NOMINAL_RATE: u32 = fourcc(b"nsrt");
const DEV_CAN_BE_DEFAULT: u32 = fourcc(b"dflt");
const DEV_CAN_BE_SYSTEM_DEFAULT: u32 = fourcc(b"sflt");

fn addr(selector: u32, scope: u32, element: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { selector, scope, element }
}

fn scope_of(direction: Direction) -> u32 {
    match direction {
        Direction::Input => SCOPE_INPUT,
        Direction::Output => SCOPE_OUTPUT,
    }
}

fn check(status: OSStatus, what: &str) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        let bytes = status.to_be_bytes();
        let code = if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
            format!("'{}'", String::from_utf8_lossy(&bytes))
        } else {
            status.to_string()
        };
        Err(anyhow!("CoreAudio {what} failed (OSStatus {code})"))
    }
}

fn has(id: AudioObjectID, a: &AudioObjectPropertyAddress) -> bool {
    unsafe { AudioObjectHasProperty(id, a) != 0 }
}

fn settable(id: AudioObjectID, a: &AudioObjectPropertyAddress) -> bool {
    let mut s: u8 = 0;
    has(id, a) && unsafe { AudioObjectIsPropertySettable(id, a, &mut s) } == 0 && s != 0
}

fn get<T: Copy + Default>(id: AudioObjectID, a: &AudioObjectPropertyAddress) -> Result<T> {
    let mut value = T::default();
    let mut size = size_of::<T>() as u32;
    let st = unsafe {
        AudioObjectGetPropertyData(id, a, 0, ptr::null(), &mut size, &mut value as *mut T as *mut c_void)
    };
    check(st, "GetPropertyData")?;
    Ok(value)
}

fn set<T: Copy>(id: AudioObjectID, a: &AudioObjectPropertyAddress, value: T) -> Result<()> {
    let st = unsafe {
        AudioObjectSetPropertyData(id, a, 0, ptr::null(), size_of::<T>() as u32, &value as *const T as *const c_void)
    };
    check(st, "SetPropertyData")
}

fn data_size(id: AudioObjectID, a: &AudioObjectPropertyAddress) -> Result<u32> {
    let mut size = 0u32;
    check(
        unsafe { AudioObjectGetPropertyDataSize(id, a, 0, ptr::null(), &mut size) },
        "GetPropertyDataSize",
    )?;
    Ok(size)
}

fn get_array<T: Copy + Default>(id: AudioObjectID, a: &AudioObjectPropertyAddress) -> Result<Vec<T>> {
    let size = data_size(id, a)?;
    let n = size as usize / size_of::<T>();
    let mut v = vec![T::default(); n];
    if n == 0 {
        return Ok(v);
    }
    let mut io = size;
    check(
        unsafe { AudioObjectGetPropertyData(id, a, 0, ptr::null(), &mut io, v.as_mut_ptr() as *mut c_void) },
        "GetPropertyData",
    )?;
    v.truncate(io as usize / size_of::<T>());
    Ok(v)
}

/// Read a CFString property and convert it to a Rust `String`.
fn get_string(id: AudioObjectID, a: &AudioObjectPropertyAddress) -> Result<String> {
    let cf: usize = get::<usize>(id, a)?; // CFStringRef, returned +1 retained
    let cf = cf as CFStringRef;
    if cf.is_null() {
        bail!("null CFString");
    }
    let result = unsafe {
        let len = CFStringGetLength(cf);
        let cap = CFStringGetMaximumSizeForEncoding(len, K_CF_STRING_ENCODING_UTF8) + 1;
        let mut buf = vec![0u8; cap.max(1) as usize];
        let ok = CFStringGetCString(cf, buf.as_mut_ptr() as *mut c_char, cap, K_CF_STRING_ENCODING_UTF8);
        CFRelease(cf);
        if ok == 0 {
            Err(anyhow!("CFStringGetCString failed"))
        } else {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            Ok(String::from_utf8_lossy(&buf[..end]).into_owned())
        }
    };
    result
}

fn transport_of(code: u32) -> Transport {
    match &code.to_be_bytes() {
        b"bltn" => Transport::BuiltIn,
        b"usb " => Transport::Usb,
        b"blue" | b"blea" => Transport::Bluetooth,
        b"hdmi" => Transport::Hdmi,
        b"dprt" => Transport::DisplayPort,
        b"pci " | b"thun" | b"1394" => Transport::Pci,
        b"airp" | b"eavb" | b"ccwl" => Transport::Network,
        b"virt" => Transport::Virtual,
        b"grup" | b"agrp" => Transport::Aggregate,
        b"ccwd" => Transport::Usb,
        _ => Transport::Unknown,
    }
}

/// Channels to use for volume/mute when the device has no main-element control.
fn volume_elements(id: AudioObjectID, direction: Direction) -> Vec<u32> {
    let scope = scope_of(direction);
    if has(id, &addr(DEV_VOLUME_SCALAR, scope, ELEMENT_MAIN)) {
        return vec![ELEMENT_MAIN];
    }
    let stereo = get::<[u32; 2]>(id, &addr(DEV_STEREO_CHANNELS, scope, ELEMENT_MAIN)).unwrap_or([1, 2]);
    let mut chans: Vec<u32> = stereo
        .into_iter()
        .filter(|&c| has(id, &addr(DEV_VOLUME_SCALAR, scope, c)))
        .collect();
    chans.dedup();
    chans
}

pub struct CoreAudioBackend;

impl CoreAudioBackend {
    pub fn new() -> Self {
        CoreAudioBackend
    }

    fn device_ids(&self) -> Result<Vec<AudioObjectID>> {
        get_array::<AudioObjectID>(SYSTEM_OBJECT, &addr(HW_DEVICES, SCOPE_GLOBAL, ELEMENT_MAIN))
    }

    fn default_device(&self, direction: Direction) -> Option<AudioObjectID> {
        let sel = match direction {
            Direction::Input => HW_DEFAULT_INPUT,
            Direction::Output => HW_DEFAULT_OUTPUT,
        };
        get::<AudioObjectID>(SYSTEM_OBJECT, &addr(sel, SCOPE_GLOBAL, ELEMENT_MAIN))
            .ok()
            .filter(|&id| id != 0)
    }

    fn has_streams(&self, id: AudioObjectID, direction: Direction) -> bool {
        data_size(id, &addr(DEV_STREAMS, scope_of(direction), ELEMENT_MAIN)).map(|s| s > 0).unwrap_or(false)
    }

    /// Map a UID back to the current AudioObjectID.
    fn object_for_uid(&self, uid: &str) -> Result<AudioObjectID> {
        for id in self.device_ids()? {
            if get_string(id, &addr(DEV_UID, SCOPE_GLOBAL, ELEMENT_MAIN)).ok().as_deref() == Some(uid) {
                return Ok(id);
            }
        }
        bail!("no audio device with UID {uid}")
    }

    fn read_volume(&self, id: AudioObjectID, direction: Direction) -> Option<f32> {
        let scope = scope_of(direction);
        let elems = volume_elements(id, direction);
        let vals: Vec<f32> = elems
            .iter()
            .filter_map(|&e| get::<f32>(id, &addr(DEV_VOLUME_SCALAR, scope, e)).ok())
            .collect();
        if vals.is_empty() {
            None
        } else {
            Some(vals.iter().sum::<f32>() / vals.len() as f32)
        }
    }

    fn read_mute(&self, id: AudioObjectID, direction: Direction) -> Option<bool> {
        let a = addr(DEV_MUTE, scope_of(direction), ELEMENT_MAIN);
        if has(id, &a) { get::<u32>(id, &a).ok().map(|m| m != 0) } else { None }
    }

    fn describe(&self, id: AudioObjectID, direction: Direction, default: Option<AudioObjectID>) -> Option<Device> {
        let uid = get_string(id, &addr(DEV_UID, SCOPE_GLOBAL, ELEMENT_MAIN)).ok()?;
        let name = get_string(id, &addr(OBJ_NAME, SCOPE_GLOBAL, ELEMENT_MAIN)).unwrap_or_else(|_| uid.clone());
        let transport = get::<u32>(id, &addr(DEV_TRANSPORT, SCOPE_GLOBAL, ELEMENT_MAIN))
            .map(transport_of)
            .unwrap_or(Transport::Unknown);
        // macOS has no explicit "hands-free" flag. A Bluetooth device running at
        // a telephony sample rate (8/16/24 kHz) is in HFP mode. A2DP runs at
        // 44.1 or 48 kHz.
        let hands_free = transport == Transport::Bluetooth
            && get::<f64>(id, &addr(DEV_NOMINAL_RATE, SCOPE_GLOBAL, ELEMENT_MAIN))
                .map(|r| r > 0.0 && r <= 24_000.0)
                .unwrap_or_else(|_| bluetooth::is_hands_free(transport, &bluetooth::Hints { name: &name, ..Default::default() }));
        Some(Device {
            id: uid,
            name,
            direction,
            transport,
            is_default: Some(id) == default,
            volume: self.read_volume(id, direction),
            muted: self.read_mute(id, direction),
            hands_free,
        })
    }
}

impl AudioBackend for CoreAudioBackend {
    fn api_name(&self) -> &'static str {
        "CoreAudio"
    }

    fn devices(&mut self) -> Result<Vec<Device>> {
        let ids = self.device_ids()?;
        let mut out = Vec::new();
        for direction in Direction::ALL {
            let default = self.default_device(direction);
            for &id in &ids {
                if !self.has_streams(id, direction) {
                    continue;
                }
                // Skip devices the system refuses to use as default (e.g. some
                // private aggregate devices). If the property is missing, keep the device.
                let can = addr(DEV_CAN_BE_DEFAULT, scope_of(direction), ELEMENT_MAIN);
                if has(id, &can) && get::<u32>(id, &can).map(|v| v == 0).unwrap_or(false) {
                    continue;
                }
                if let Some(d) = self.describe(id, direction, default) {
                    out.push(d);
                }
            }
        }
        Ok(out)
    }

    fn set_default(&mut self, uid: &str, direction: Direction) -> Result<()> {
        let id = self.object_for_uid(uid)?;
        if !self.has_streams(id, direction) {
            bail!("device has no {direction} streams");
        }
        let sel = match direction {
            Direction::Input => HW_DEFAULT_INPUT,
            Direction::Output => HW_DEFAULT_OUTPUT,
        };
        set(SYSTEM_OBJECT, &addr(sel, SCOPE_GLOBAL, ELEMENT_MAIN), id)?;
        if direction == Direction::Output {
            // Move alert/system sounds too, like the Sound settings pane does,
            // when the device allows it.
            let can = addr(DEV_CAN_BE_SYSTEM_DEFAULT, SCOPE_OUTPUT, ELEMENT_MAIN);
            if get::<u32>(id, &can).map(|v| v != 0).unwrap_or(false) {
                let _ = set(SYSTEM_OBJECT, &addr(HW_DEFAULT_SYSTEM_OUTPUT, SCOPE_GLOBAL, ELEMENT_MAIN), id);
            }
        }
        Ok(())
    }

    fn set_volume(&mut self, uid: &str, direction: Direction, volume: f32) -> Result<()> {
        let id = self.object_for_uid(uid)?;
        let scope = scope_of(direction);
        let elems: Vec<u32> = volume_elements(id, direction)
            .into_iter()
            .filter(|&e| settable(id, &addr(DEV_VOLUME_SCALAR, scope, e)))
            .collect();
        if elems.is_empty() {
            bail!("this device has no adjustable {direction} volume");
        }
        let v = clamp_volume(volume);
        for e in elems {
            set(id, &addr(DEV_VOLUME_SCALAR, scope, e), v)?;
        }
        Ok(())
    }

    fn set_mute(&mut self, uid: &str, direction: Direction, muted: bool) -> Result<()> {
        let id = self.object_for_uid(uid)?;
        let a = addr(DEV_MUTE, scope_of(direction), ELEMENT_MAIN);
        if !settable(id, &a) {
            bail!("this device has no {direction} mute control");
        }
        set(id, &a, muted as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_matches_coreaudio_headers() {
        assert_eq!(fourcc(b"dev#"), 0x6465_7623);
        assert_eq!(fourcc(b"dIn "), 0x6449_6e20);
        assert_eq!(fourcc(b"volm"), 0x766f_6c6d);
        assert_eq!(transport_of(0x626c_7565), Transport::Bluetooth);
        assert_eq!(transport_of(0x626c_746e), Transport::BuiltIn);
    }

    /// Talks to the real CoreAudio daemon. Read-only, so it is safe to run anywhere.
    #[test]
    fn lists_devices_without_error() {
        let mut b = CoreAudioBackend::new();
        let devices = b.devices().expect("CoreAudio device list");
        for d in &devices {
            assert!(!d.id.is_empty());
            if let Some(v) = d.volume {
                assert!((0.0..=1.0).contains(&v));
            }
        }
        for dir in Direction::ALL {
            let n = devices.iter().filter(|d| d.direction == dir && d.is_default).count();
            assert!(n <= 1, "at most one default {dir}");
        }
    }
}
