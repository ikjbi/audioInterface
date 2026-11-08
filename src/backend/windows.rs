//! Windows backend: MMDevice API + WASAPI endpoint volume/metering.
//!
//! Changing the default device uses `IPolicyConfig`. Windows has no documented
//! API for that. This COM interface has been stable since Windows 7 and is what
//! the Sound control panel itself uses, as do tools like EarTrumpet and
//! SoundSwitch.

#![allow(non_snake_case)]

use super::{AudioBackend, clamp_volume};
use crate::bluetooth::{self, Hints};
use crate::model::{AppStream, Device, Direction, Transport};
use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::ffi::c_void;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, DEVICE_STATE_ACTIVE, EDataFlow, ERole, IAudioClient,
    IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume,
    MMDeviceEnumerator, PKEY_AudioEndpoint_FormFactor, eCapture, eCommunications, eConsole, eMultimedia, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::{GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, Interface, PCWSTR, interface};

/// Undocumented `IPolicyConfig` (Windows 7+). Only `SetDefaultEndpoint` is
/// called. The other methods are declared to keep the vtable layout right.
#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: IUnknown {
    fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut c_void) -> HRESULT;
    fn GetDeviceFormat(&self, device: PCWSTR, default: i32, format: *mut *mut c_void) -> HRESULT;
    fn ResetDeviceFormat(&self, device: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(&self, device: PCWSTR, endpoint: *mut c_void, mix: *mut c_void) -> HRESULT;
    fn GetProcessingPeriod(
        &self,
        device: PCWSTR,
        default: i32,
        default_period: *mut i64,
        min_period: *mut i64,
    ) -> HRESULT;
    fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> HRESULT;
    fn GetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
    fn SetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
    fn GetPropertyValue(&self, device: PCWSTR, key: *const PROPERTYKEY, value: *mut c_void) -> HRESULT;
    fn SetPropertyValue(&self, device: PCWSTR, key: *const PROPERTYKEY, value: *mut c_void) -> HRESULT;
    fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
    fn SetEndpointVisibility(&self, device: PCWSTR, visible: i32) -> HRESULT;
}

/// CLSID of `CPolicyConfigClient`.
const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// Instance path of the endpoint's parent device, e.g. `{1}.BTHHFENUM\...`.
/// This is how we tell Bluetooth hands-free endpoints apart.
const PKEY_ENDPOINT_PARENT_PATH: PROPERTYKEY =
    PROPERTYKEY { fmtid: GUID::from_u128(0xb3f8fa53_0004_438e_9003_51a46e139bfc), pid: 2 };
/// `PKEY_Device_EnumeratorName` (`BTHENUM`, `USB`, `HDAUDIO`, ...).
const PKEY_DEVICE_ENUMERATOR: PROPERTYKEY =
    PROPERTYKEY { fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 24 };
/// `EndpointFormFactor::DigitalAudioDisplayDevice` (HDMI / DisplayPort monitor).
const FORM_FACTOR_DISPLAY: u32 = 9;

fn flow(direction: Direction) -> EDataFlow {
    match direction {
        Direction::Input => eCapture,
        Direction::Output => eRender,
    }
}

pub struct WasapiBackend {
    enumerator: IMMDeviceEnumerator,
    meters: HashMap<(String, Direction), IAudioMeterInformation>,
    /// Shared-mode mix format per endpoint (rate, channels), refreshed every 10 s.
    formats: HashMap<String, (std::time::Instant, Option<(u32, u16)>)>,
}

/// The endpoint's shared-mode mix format: (sample rate, channels).
fn mix_format(dev: &IMMDevice) -> Option<(u32, u16)> {
    unsafe {
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).ok()?;
        let p = client.GetMixFormat().ok()?;
        if p.is_null() {
            return None;
        }
        let f = std::ptr::read_unaligned(p);
        CoTaskMemFree(Some(p as *const c_void));
        let (rate, ch) = (f.nSamplesPerSec, f.nChannels);
        Some((rate, ch))
    }
}

impl WasapiBackend {
    pub fn new() -> Result<Self> {
        unsafe {
            // S_FALSE / RPC_E_CHANGED_MODE just mean COM is already initialised
            // on this thread, possibly in another apartment. Either is fine.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).context("create MMDeviceEnumerator")?;
            Ok(WasapiBackend { enumerator, meters: HashMap::new(), formats: HashMap::new() })
        }
    }

    fn device(&self, id: &str) -> Result<IMMDevice> {
        unsafe { self.enumerator.GetDevice(&HSTRING::from(id)) }.with_context(|| format!("no device with id {id}"))
    }

    fn default_id(&self, direction: Direction, role: ERole) -> Option<String> {
        unsafe {
            let dev = self.enumerator.GetDefaultAudioEndpoint(flow(direction), role).ok()?;
            device_id(&dev).ok()
        }
    }

    fn set_roles(&mut self, id: &str, direction: Direction, roles: &[ERole]) -> Result<()> {
        // Make sure it exists (and is the right direction) for a clear error.
        let exists = self.devices()?.iter().any(|d| d.id == id && d.direction == direction);
        if !exists {
            return Err(anyhow!("no active {direction} device with id {id}"));
        }
        unsafe {
            let policy: IPolicyConfig =
                CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL).context("create PolicyConfig")?;
            let wide = HSTRING::from(id);
            for role in roles {
                policy
                    .SetDefaultEndpoint(PCWSTR(wide.as_ptr()), *role)
                    .ok()
                    .with_context(|| format!("SetDefaultEndpoint role {}", role.0))?;
            }
        }
        Ok(())
    }

    fn endpoint_volume(&self, id: &str) -> Result<IAudioEndpointVolume> {
        let dev = self.device(id)?;
        unsafe { dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.context("activate IAudioEndpointVolume")
    }
}

unsafe fn device_id(dev: &IMMDevice) -> Result<String> {
    unsafe {
        let pw = dev.GetId()?;
        let s = pw.to_string();
        CoTaskMemFree(Some(pw.0 as *const c_void));
        Ok(s?)
    }
}

/// Other processes with an active audio session on this endpoint (e.g. Teams
/// capturing from the mic during a call). Our own process is excluded.
fn session_users(dev: &IMMDevice) -> Vec<String> {
    let own = std::process::id();
    let mut out: Vec<String> = Vec::new();
    unsafe {
        let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { return out };
        let Ok(sessions) = mgr.GetSessionEnumerator() else { return out };
        for i in 0..sessions.GetCount().unwrap_or(0) {
            let Ok(ctl) = sessions.GetSession(i) else { continue };
            if ctl.GetState().ok() != Some(AudioSessionStateActive) {
                continue;
            }
            let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
            if ctl2.IsSystemSoundsSession() == S_OK {
                continue;
            }
            let pid = ctl2.GetProcessId().unwrap_or(0);
            if pid == own {
                continue;
            }
            let raw = ctl2
                .GetSessionIdentifier()
                .ok()
                .and_then(|p| {
                    let s = p.to_string().ok();
                    CoTaskMemFree(Some(p.0 as *const c_void));
                    s
                })
                .and_then(|id| crate::model::exe_from_windows_session_id(&id))
                .unwrap_or_else(|| format!("pid {pid}"));
            let name = crate::model::friendly_app_name(&raw);
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// Friendly app name for a session.
fn session_app_name(ctl2: &IAudioSessionControl2) -> String {
    unsafe {
        if ctl2.IsSystemSoundsSession() == S_OK {
            return "System sounds".into();
        }
        let pid = ctl2.GetProcessId().unwrap_or(0);
        let raw = ctl2
            .GetSessionIdentifier()
            .ok()
            .and_then(|p| {
                let s = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as *const c_void));
                s
            })
            .and_then(|id| crate::model::exe_from_windows_session_id(&id))
            .unwrap_or_else(|| format!("pid {pid}"));
        crate::model::friendly_app_name(&raw)
    }
}

/// Unique id of a session (instance identifier).
fn session_instance_id(ctl2: &IAudioSessionControl2) -> Option<String> {
    unsafe {
        let p = ctl2.GetSessionInstanceIdentifier().ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const c_void));
        s
    }
}

fn prop_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let v = unsafe { store.GetValue(key) }.ok()?;
    let s = v.to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn prop_u32(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<u32> {
    let v = unsafe { store.GetValue(key) }.ok()?;
    u32::try_from(&v).ok()
}

impl WasapiBackend {
    /// All non-expired sessions on all active endpoints, excluding our own.
    fn all_sessions(&self) -> Vec<(String, Direction, IAudioSessionControl2)> {
        let own = std::process::id();
        let mut out = Vec::new();
        for direction in Direction::ALL {
            let Ok(collection) = (unsafe { self.enumerator.EnumAudioEndpoints(flow(direction), DEVICE_STATE_ACTIVE) })
            else {
                continue;
            };
            for i in 0..unsafe { collection.GetCount() }.unwrap_or(0) {
                let Ok(dev) = (unsafe { collection.Item(i) }) else { continue };
                let Ok(dev_id) = (unsafe { device_id(&dev) }) else { continue };
                let Ok(mgr) = (unsafe { dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }) else { continue };
                let Ok(sessions) = (unsafe { mgr.GetSessionEnumerator() }) else { continue };
                for j in 0..unsafe { sessions.GetCount() }.unwrap_or(0) {
                    let Ok(ctl) = (unsafe { sessions.GetSession(j) }) else { continue };
                    if unsafe { ctl.GetState() }.ok() == Some(AudioSessionStateExpired) {
                        continue;
                    }
                    let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
                    if unsafe { ctl2.GetProcessId() }.unwrap_or(0) == own {
                        continue;
                    }
                    out.push((dev_id.clone(), direction, ctl2));
                }
            }
        }
        out
    }

    fn find_session_volume(&self, stream_id: &str) -> Result<ISimpleAudioVolume> {
        for (_, _, ctl2) in self.all_sessions() {
            if session_instance_id(&ctl2).as_deref() == Some(stream_id) {
                return ctl2.cast::<ISimpleAudioVolume>().context("session has no volume");
            }
        }
        Err(anyhow!("audio session not found (the app may have closed)"))
    }
}

impl AudioBackend for WasapiBackend {
    fn api_name(&self) -> &'static str {
        "WASAPI"
    }

    fn devices(&mut self) -> Result<Vec<Device>> {
        let mut out = Vec::new();
        for direction in Direction::ALL {
            let default = self.default_id(direction, eConsole);
            let comms_default = self.default_id(direction, eCommunications);
            let collection = unsafe { self.enumerator.EnumAudioEndpoints(flow(direction), DEVICE_STATE_ACTIVE)? };
            let count = unsafe { collection.GetCount()? };
            for i in 0..count {
                let Ok(dev) = (unsafe { collection.Item(i) }) else { continue };
                let Ok(id) = (unsafe { device_id(&dev) }) else { continue };
                let store = unsafe { dev.OpenPropertyStore(STGM_READ) }.ok();
                let name = store
                    .as_ref()
                    .and_then(|s| prop_string(s, &PKEY_Device_FriendlyName))
                    .unwrap_or_else(|| id.clone());
                let enumerator = store.as_ref().and_then(|s| prop_string(s, &PKEY_DEVICE_ENUMERATOR));
                let parent = store.as_ref().and_then(|s| prop_string(s, &PKEY_ENDPOINT_PARENT_PATH));
                let form_factor = store.as_ref().and_then(|s| prop_u32(s, &PKEY_AudioEndpoint_FormFactor));
                let path = format!("{}|{}", enumerator.unwrap_or_default(), parent.unwrap_or_default());
                let hints = Hints { name: &name, instance_path: Some(&path), ..Default::default() };
                let mut transport = bluetooth::guess_transport(&hints);
                if form_factor == Some(FORM_FACTOR_DISPLAY) && transport != Transport::Bluetooth {
                    transport = if name.to_ascii_lowercase().contains("displayport") {
                        Transport::DisplayPort
                    } else {
                        Transport::Hdmi
                    };
                }
                let hands_free = bluetooth::is_hands_free(transport, &hints);

                let ev = unsafe { dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok();
                let volume = ev.as_ref().and_then(|e| unsafe { e.GetMasterVolumeLevelScalar() }.ok());
                let muted = ev.as_ref().and_then(|e| unsafe { e.GetMute() }.ok()).map(|b| b.as_bool());
                let balance = ev.as_ref().and_then(|e| unsafe {
                    if e.GetChannelCount().ok()? < 2 {
                        return None;
                    }
                    let l = e.GetChannelVolumeLevelScalar(0).ok()?;
                    let r = e.GetChannelVolumeLevelScalar(1).ok()?;
                    Some(crate::model::balance_from(l, r))
                });

                let is_comms_default = comms_default.as_deref() == Some(id.as_str());
                let fmt = match self.formats.get(&id) {
                    Some((at, f)) if at.elapsed() < std::time::Duration::from_secs(10) => *f,
                    _ => {
                        let f = mix_format(&dev);
                        self.formats.insert(id.clone(), (std::time::Instant::now(), f));
                        f
                    }
                };
                out.push(Device {
                    is_default: default.as_deref() == Some(id.as_str()),
                    id,
                    name,
                    direction,
                    transport,
                    volume,
                    muted,
                    hands_free,
                    in_use_by: session_users(&dev),
                    is_comms_default,
                    balance,
                    sample_rate: fmt.map(|f| f.0),
                    channels: fmt.map(|f| f.1),
                });
            }
        }
        Ok(out)
    }

    fn set_default(&mut self, id: &str, direction: Direction) -> Result<()> {
        // All three roles, which is what "Set as Default Device" in the Sound
        // control panel does.
        self.set_roles(id, direction, &[eConsole, eMultimedia, eCommunications])
    }

    fn supports_comms_role(&self) -> bool {
        true
    }

    fn set_comms_default(&mut self, id: &str, direction: Direction) -> Result<()> {
        self.set_roles(id, direction, &[eCommunications])
    }

    fn set_volume(&mut self, id: &str, _direction: Direction, volume: f32) -> Result<()> {
        let ev = self.endpoint_volume(id)?;
        unsafe { ev.SetMasterVolumeLevelScalar(clamp_volume(volume), std::ptr::null())? };
        Ok(())
    }

    fn app_streams(&mut self) -> Vec<AppStream> {
        self.all_sessions()
            .into_iter()
            .filter_map(|(device_id, direction, ctl2)| {
                let vol = ctl2.cast::<ISimpleAudioVolume>().ok()?;
                let (volume, muted) = unsafe { (vol.GetMasterVolume().ok()?, vol.GetMute().ok()?.as_bool()) };
                Some(AppStream {
                    id: session_instance_id(&ctl2)?,
                    app: session_app_name(&ctl2),
                    direction,
                    device_id,
                    volume,
                    muted,
                })
            })
            .collect()
    }

    fn set_app_volume(&mut self, stream_id: &str, volume: f32) -> Result<()> {
        let v = self.find_session_volume(stream_id)?;
        unsafe { v.SetMasterVolume(clamp_volume(volume), std::ptr::null())? };
        Ok(())
    }

    fn set_app_mute(&mut self, stream_id: &str, muted: bool) -> Result<()> {
        let v = self.find_session_volume(stream_id)?;
        unsafe { v.SetMute(muted, std::ptr::null())? };
        Ok(())
    }

    fn set_balance(&mut self, id: &str, _direction: Direction, balance: f32) -> Result<()> {
        let ev = self.endpoint_volume(id)?;
        unsafe {
            if ev.GetChannelCount()? < 2 {
                return Err(anyhow!("this device is mono"));
            }
            let l = ev.GetChannelVolumeLevelScalar(0)?;
            let r = ev.GetChannelVolumeLevelScalar(1)?;
            let (nl, nr) = crate::model::channel_gains(l.max(r), balance);
            ev.SetChannelVolumeLevelScalar(0, nl, std::ptr::null())?;
            ev.SetChannelVolumeLevelScalar(1, nr, std::ptr::null())?;
        }
        Ok(())
    }

    fn set_mute(&mut self, id: &str, _direction: Direction, muted: bool) -> Result<()> {
        let ev = self.endpoint_volume(id)?;
        unsafe { ev.SetMute(muted, std::ptr::null())? };
        Ok(())
    }

    fn native_peak(&mut self, id: &str, direction: Direction) -> Option<f32> {
        let key = (id.to_string(), direction);
        if !self.meters.contains_key(&key) {
            let dev = self.device(id).ok()?;
            let meter = unsafe { dev.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None) }.ok()?;
            self.meters.insert(key.clone(), meter);
        }
        let meter = self.meters.get(&key)?;
        match unsafe { meter.GetPeakValue() } {
            Ok(v) => Some(v),
            Err(_) => {
                // Device probably went away. Drop the cached meter.
                self.meters.remove(&key);
                None
            }
        }
    }

    /// Render endpoints report peaks for whatever is playing. Capture endpoints
    /// only report while some app has a capture stream open, so inputs are
    /// metered with a cpal stream instead.
    fn has_native_meter(&self, direction: Direction) -> bool {
        direction == Direction::Output
    }
}
