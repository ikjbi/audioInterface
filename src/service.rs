//! Background worker that owns the OS backend and the meter engine.
//!
//! The UI never touches the OS directly. It sends [`Request`]s and reads a shared
//! [`Snapshot`]. The worker:
//! * refreshes the device list a few times per second (so hot-plug is picked up),
//! * enforces gain locks and auto-switch rules,
//! * keeps capture streams running for the devices being metered,
//! * polls native peak meters (Windows outputs),
//! * records a human-readable event log.

use crate::analysis::{Report, Sample};
use crate::backend::AudioBackend;
use crate::levels::{Consumer, LevelCell};
use crate::meter::MeterEngine;
use crate::model::{Device, DeviceKey, Direction, Transport};
use crate::recorder::{RecordingInfo, SessionRecorder};
use crate::rules::{AutoSwitcher, GainLocker};
use crate::settings::Settings;
use anyhow::Result;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

const TICK: Duration = Duration::from_millis(30);
const REFRESH_EVERY: Duration = Duration::from_millis(500);
const RECONNECT_EVERY: Duration = Duration::from_secs(3);
const MAX_EVENTS: usize = 200;

#[derive(Debug, Clone)]
pub enum Request {
    SetDefault(DeviceKey),
    /// Windows: make this the default communications device only.
    SetCommsDefault(DeviceKey),
    /// Per-app volume / mute (stream id from `AppStream::id`).
    SetAppVolume(String, f32),
    SetAppMute(String, bool),
    /// Switch the card behind a device to a profile (Linux), e.g. A2DP ↔ headset.
    SetCardProfile(DeviceKey, String),
    /// Left/right balance (-1 ..= 1).
    SetBalance(DeviceKey, f32),
    /// Change a device's sample rate (macOS).
    SetSampleRate(DeviceKey, u32),
    /// Toggle mute on the default device of a direction (hotkey / tray).
    ToggleDefaultMute(Direction),
    /// Switch the default to the next device (hotkey / tray).
    CycleDefault(Direction),
    SetVolume(DeviceKey, f32),
    SetMute(DeviceKey, bool),
    /// Start/stop metering a device on user request.
    SetMonitored(DeviceKey, bool),
    UpdateSettings(Settings),
    /// Apply a profile: switch defaults, volumes and mutes.
    ApplyProfile(crate::profiles::Profile),
    /// Start logging a device (optionally with audio).
    StartRecording {
        key: DeviceKey,
        audio: bool,
    },
    StopRecording(DeviceKey),
    Refresh,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Info,
    Action,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub at: SystemTime,
    pub kind: EventKind,
    pub text: String,
}

#[derive(Default)]
pub struct Snapshot {
    pub api: &'static str,
    /// The OS has a separate communications default (Windows).
    pub supports_comms: bool,
    pub meter_host: &'static str,
    pub devices: Vec<Device>,
    /// Backend-level error (e.g. sound server not running).
    pub error: Option<String>,
    pub events: VecDeque<Event>,
    /// Level cells for every metered device.
    pub levels: HashMap<DeviceKey, Arc<LevelCell>>,
    /// Per-application streams (Windows, Linux).
    pub app_streams: Vec<crate::model::AppStream>,
    /// Card profiles per device (Linux; empty elsewhere).
    pub card_profiles: HashMap<DeviceKey, Vec<crate::model::DeviceProfile>>,
    /// Sample rates each device can be switched to (macOS; empty elsewhere).
    pub sample_rates: HashMap<DeviceKey, Vec<u32>>,
    /// Recent raw samples of capture-metered devices (spectrum view).
    pub windows: HashMap<DeviceKey, Arc<crate::levels::SampleWindow>>,
    /// Why metering failed for a device.
    pub meter_errors: HashMap<DeviceKey, String>,
    /// Devices the user asked to meter (on top of the automatic ones).
    pub monitored: BTreeSet<DeviceKey>,
    /// Devices whose meter comes from a native OS peak meter (no capture stream).
    pub native_metered: BTreeSet<DeviceKey>,
    /// Incremented on every device refresh.
    pub generation: u64,
    /// Total number of events ever logged (to find new ones).
    pub total_events: u64,
    /// Recordings in progress.
    pub recordings: Vec<ActiveRecording>,
    /// Recently finished recordings with their analysis, newest last.
    pub finished: VecDeque<FinishedRecording>,
}

#[derive(Clone, Debug)]
pub struct ActiveRecording {
    pub info: RecordingInfo,
    pub elapsed_s: f64,
    pub audio: bool,
    /// The most recent logged seconds (for a live chart).
    pub recent: Vec<Sample>,
}

#[derive(Clone, Debug)]
pub struct FinishedRecording {
    pub info: RecordingInfo,
    pub report: Report,
    pub report_path: std::path::PathBuf,
}

impl Snapshot {
    fn log(&mut self, kind: EventKind, text: impl Into<String>) {
        let text = text.into();
        match kind {
            EventKind::Error => log::error!("{text}"),
            EventKind::Warning => log::warn!("{text}"),
            _ => log::info!("{text}"),
        }
        self.events.push_back(Event { at: SystemTime::now(), kind, text });
        self.total_events += 1;
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
    }
}

pub type BackendFactory = Box<dyn Fn() -> Result<Box<dyn AudioBackend>> + Send>;

pub struct Service {
    tx: Sender<Request>,
    shared: Arc<Mutex<Snapshot>>,
    thread: Option<JoinHandle<()>>,
}

impl Service {
    pub fn start(settings: Settings, factory: BackendFactory) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let worker_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("audio-service".into())
            .spawn(move || Worker::new(settings, factory, worker_shared, rx).run())
            .expect("spawn audio service thread");
        Service { tx, shared, thread: Some(thread) }
    }

    pub fn send(&self, req: Request) {
        let _ = self.tx.send(req);
    }

    pub fn snapshot(&self) -> MutexGuard<'_, Snapshot> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Worker {
    settings: Settings,
    factory: BackendFactory,
    backend: Option<Box<dyn AudioBackend>>,
    last_connect_attempt: Option<Instant>,
    meters: MeterEngine,
    native_cells: HashMap<DeviceKey, Arc<LevelCell>>,
    switcher: AutoSwitcher,
    locker: GainLocker,
    monitored: BTreeSet<DeviceKey>,
    /// Devices whose automatic meter the user hid this session.
    hidden_meters: BTreeSet<DeviceKey>,
    devices: Vec<Device>,
    have_listed: bool,
    recorders: HashMap<DeviceKey, SessionRecorder>,
    /// Recordings that should include audio (attached once the stream is up).
    want_audio: BTreeSet<DeviceKey>,
    /// Auto-started recordings, and since when their device has been idle.
    auto_recording: HashMap<DeviceKey, Option<Instant>>,
    events_forwarded: u64,
    rates_checked: Option<Instant>,
    /// When we (user request, gain lock, profile) last changed a volume, so
    /// changes made by other apps can be told apart.
    our_volume_change: HashMap<DeviceKey, Instant>,
    /// Rate limiting for desktop notifications, by title.
    notified: Mutex<HashMap<String, Instant>>,
    shared: Arc<Mutex<Snapshot>>,
    rx: Receiver<Request>,
}

impl Worker {
    fn new(settings: Settings, factory: BackendFactory, shared: Arc<Mutex<Snapshot>>, rx: Receiver<Request>) -> Self {
        let meters = MeterEngine::new();
        shared.lock().unwrap().meter_host = meters.host_name();
        Worker {
            settings,
            factory,
            backend: None,
            last_connect_attempt: None,
            meters,
            native_cells: HashMap::new(),
            switcher: AutoSwitcher::default(),
            locker: GainLocker::default(),
            monitored: BTreeSet::new(),
            hidden_meters: BTreeSet::new(),
            devices: Vec::new(),
            have_listed: false,
            recorders: HashMap::new(),
            want_audio: BTreeSet::new(),
            auto_recording: HashMap::new(),
            events_forwarded: 0,
            rates_checked: None,
            our_volume_change: HashMap::new(),
            notified: Mutex::new(HashMap::new()),
            shared,
            rx,
        }
    }

    fn shared(&self) -> MutexGuard<'_, Snapshot> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn log(&self, kind: EventKind, text: impl Into<String>) {
        self.shared().log(kind, text);
    }

    fn name_of(&self, key: &DeviceKey) -> String {
        self.devices.iter().find(|d| d.key() == *key).map(|d| d.name.clone()).unwrap_or_else(|| key.id.clone())
    }

    fn run(mut self) {
        let mut last_refresh: Option<Instant> = None;
        loop {
            let mut force = false;
            match self.rx.recv_timeout(TICK) {
                Ok(Request::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(req) => {
                    self.handle(req);
                    // Drain anything else queued (e.g. a burst of slider moves).
                    while let Ok(req) = self.rx.try_recv() {
                        if matches!(req, Request::Shutdown) {
                            self.stop_all_recordings();
                            self.meters.stop_all();
                            return;
                        }
                        self.handle(req);
                    }
                    force = true;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if force || last_refresh.is_none_or(|t| t.elapsed() >= REFRESH_EVERY) {
                self.refresh();
                last_refresh = Some(Instant::now());
            }
            self.poll_native_meters();
            self.tick_recorders();
        }
        self.stop_all_recordings();
        self.meters.stop_all();
    }

    fn ensure_backend(&mut self) -> bool {
        if self.backend.is_some() {
            return true;
        }
        if self.last_connect_attempt.is_some_and(|t| t.elapsed() < RECONNECT_EVERY) {
            return false;
        }
        self.last_connect_attempt = Some(Instant::now());
        match (self.factory)() {
            Ok(b) => {
                let api = b.api_name();
                let comms = b.supports_comms_role();
                self.backend = Some(b);
                let mut s = self.shared();
                s.api = api;
                s.supports_comms = comms;
                s.error = None;
                s.log(EventKind::Info, format!("Connected to {api}"));
                true
            }
            Err(e) => {
                let msg = format!("{e:#}");
                let mut s = self.shared();
                if s.error.as_deref() != Some(msg.as_str()) {
                    s.log(EventKind::Error, format!("Audio system unavailable: {msg}"));
                }
                s.error = Some(msg);
                false
            }
        }
    }

    /// Show a desktop notification (off in tests and when disabled in settings).
    fn notify(&self, title: &str, body: &str) {
        if !self.settings.notifications || cfg!(test) {
            return;
        }
        {
            let mut seen = self.notified.lock().unwrap_or_else(|e| e.into_inner());
            if seen.get(title).is_some_and(|t| t.elapsed() < Duration::from_secs(15)) {
                return;
            }
            seen.insert(title.to_string(), Instant::now());
        }
        let (title, body) = (title.to_string(), body.to_string());
        // Showing can block briefly (D-Bus / Notification Center), so never on this thread.
        let _ = std::thread::Builder::new().name("notify".into()).spawn(move || {
            if let Err(e) =
                notify_rust::Notification::new().appname("Audio Interface").summary(&title).body(&body).show()
            {
                log::warn!("notification failed: {e}");
            }
        });
    }

    fn handle(&mut self, req: Request) {
        match req {
            Request::UpdateSettings(s) => {
                self.settings = s;
                return;
            }
            Request::SetMonitored(key, on) => {
                if on {
                    self.hidden_meters.remove(&key);
                    self.monitored.insert(key);
                } else {
                    // Also hides the automatic default-input meter (until shown again).
                    self.monitored.remove(&key);
                    self.hidden_meters.insert(key);
                }
                self.sync_meters();
                return;
            }
            Request::ToggleDefaultMute(dir) => {
                let Some(d) = self.devices.iter().find(|d| d.direction == dir && d.is_default).cloned() else {
                    return;
                };
                let muted = !d.muted.unwrap_or(false);
                self.handle(Request::SetMute(d.key(), muted));
                let what = if dir == Direction::Input { "Microphone" } else { "Output" };
                self.notify(&format!("{what} {}", if muted { "muted" } else { "unmuted" }), &self.settings_name(&d));
                return;
            }
            Request::CycleDefault(dir) => {
                let Some(next) = self.settings.next_device(&self.devices, dir).cloned() else { return };
                self.handle(Request::SetDefault(next.key()));
                self.notify(&format!("Default {dir}"), &self.settings_name(&next));
                return;
            }
            Request::ApplyProfile(profile) => {
                self.apply_profile(&profile);
                return;
            }
            Request::StartRecording { key, audio } => {
                self.start_recording(key, audio, false);
                return;
            }
            Request::StopRecording(key) => {
                self.auto_recording.remove(&key);
                self.stop_recording(&key);
                return;
            }
            Request::SetAppVolume(id, v) => {
                if self.ensure_backend()
                    && let Err(e) = self.backend.as_mut().unwrap().set_app_volume(&id, v)
                {
                    self.log(EventKind::Error, format!("App volume: {e:#}"));
                }
                return;
            }
            Request::SetAppMute(id, m) => {
                if self.ensure_backend()
                    && let Err(e) = self.backend.as_mut().unwrap().set_app_mute(&id, m)
                {
                    self.log(EventKind::Error, format!("App mute: {e:#}"));
                }
                return;
            }
            Request::Refresh | Request::Shutdown => return,
            _ => {}
        }
        if !self.ensure_backend() {
            self.log(EventKind::Error, "Cannot apply change: audio system unavailable");
            return;
        }
        if let Request::SetVolume(k, _) = &req {
            self.our_volume_change.insert(k.clone(), Instant::now());
        }
        let name = match &req {
            Request::SetDefault(k)
            | Request::SetSampleRate(k, _)
            | Request::SetBalance(k, _)
            | Request::SetCardProfile(k, _)
            | Request::SetCommsDefault(k)
            | Request::SetVolume(k, _)
            | Request::SetMute(k, _) => self.name_of(k),
            _ => String::new(),
        };
        let backend = self.backend.as_mut().unwrap();
        let result = match &req {
            Request::SetDefault(k) => {
                backend.set_default(&k.id, k.direction).map(|_| format!("Default {} set to {name}", k.direction))
            }
            Request::SetCardProfile(k, p) => {
                let r = backend.set_card_profile(&k.id, k.direction, p).map(|_| format!("{name}: profile set to {p}"));
                self.rates_checked = None; // re-read profiles
                r
            }
            Request::SetBalance(k, b) => backend.set_balance(&k.id, k.direction, *b).map(|_| String::new()),
            Request::SetSampleRate(k, rate) => backend
                .set_sample_rate(&k.id, k.direction, *rate)
                .map(|_| format!("{name} sample rate set to {rate} Hz")),
            Request::SetCommsDefault(k) => backend
                .set_comms_default(&k.id, k.direction)
                .map(|_| format!("Default communications {} set to {name}", k.direction)),
            Request::SetVolume(k, v) => backend.set_volume(&k.id, k.direction, *v).map(|_| String::new()),
            Request::SetMute(k, m) => backend
                .set_mute(&k.id, k.direction, *m)
                .map(|_| format!("{} {name}", if *m { "Muted" } else { "Unmuted" })),
            _ => unreachable!(),
        };
        match result {
            Ok(msg) if !msg.is_empty() => self.log(EventKind::Action, msg),
            Ok(_) => {}
            Err(e) => self.log(EventKind::Error, format!("Failed to change {name}: {e:#}")),
        }
    }

    fn refresh(&mut self) {
        if !self.ensure_backend() {
            self.meters.sync(&HashMap::new());
            let mut s = self.shared();
            s.devices.clear();
            s.levels.clear();
            return;
        }
        let devices = match self.backend.as_mut().unwrap().devices() {
            Ok(d) => d,
            Err(e) => {
                self.log(EventKind::Error, format!("Listing devices failed: {e:#}"));
                // Drop the backend so we reconnect (e.g. the sound server restarted).
                self.backend = None;
                self.shared().error = Some(format!("{e:#}"));
                return;
            }
        };
        let devices = self.apply_rules(devices);
        self.log_changes(&devices);
        self.devices = devices;
        self.have_listed = true;
        self.auto_record();
        for (key, rec) in self.recorders.iter_mut() {
            rec.update_device(self.devices.iter().find(|d| d.key() == *key));
        }
        self.sync_meters();

        let levels: HashMap<DeviceKey, Arc<LevelCell>> = self
            .devices
            .iter()
            .filter_map(|d| {
                let k = d.key();
                self.meters.cell(&k).or_else(|| self.native_cells.get(&k).cloned()).map(|c| (k, c))
            })
            .collect();
        let meter_errors =
            self.devices.iter().filter_map(|d| self.meters.error(&d.key()).map(|e| (d.key(), e))).collect();
        // Supported sample rates change rarely; re-query every 10 s.
        let rates = if self.rates_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
            self.rates_checked = Some(Instant::now());
            let backend = self.backend.as_mut().unwrap();
            let rates = self
                .devices
                .iter()
                .map(|d| (d.key(), backend.sample_rates(&d.id, d.direction)))
                .filter(|(_, r)| !r.is_empty())
                .collect::<HashMap<_, _>>();
            let profiles = self
                .devices
                .iter()
                .map(|d| (d.key(), backend.card_profiles(&d.id, d.direction)))
                .filter(|(_, p)| !p.is_empty())
                .collect::<HashMap<_, _>>();
            Some((rates, profiles))
        } else {
            None
        };
        let app_streams = self.backend.as_mut().unwrap().app_streams();
        let windows: HashMap<DeviceKey, Arc<crate::levels::SampleWindow>> =
            self.devices.iter().filter_map(|d| self.meters.window(&d.key()).map(|w| (d.key(), w))).collect();
        let native: BTreeSet<DeviceKey> = self.native_cells.keys().cloned().collect();
        let devices = self.devices.clone();
        let monitored = self.monitored.clone();
        let recordings: Vec<ActiveRecording> = self
            .recorders
            .values()
            .map(|r| ActiveRecording {
                info: r.info.clone(),
                elapsed_s: r.elapsed().as_secs_f64(),
                audio: r.records_audio(),
                recent: r.rows()[r.rows().len().saturating_sub(600)..].to_vec(),
            })
            .collect();
        let mut s = self.shared();
        s.recordings = recordings;
        s.devices = devices;
        s.levels = levels;
        s.windows = windows;
        s.app_streams = app_streams;
        if let Some((r, p)) = rates {
            s.sample_rates = r;
            s.card_profiles = p;
        }
        s.meter_errors = meter_errors;
        s.monitored = monitored;
        s.native_metered = native;
        s.error = None;
        s.generation += 1;
    }

    /// Enforce gain locks and auto-switching, re-listing if anything changed.
    fn apply_rules(&mut self, mut devices: Vec<Device>) -> Vec<Device> {
        let backend = self.backend.as_mut().unwrap();
        let mut changed = false;
        let mut msgs = Vec::new();
        let mut notes: Vec<(String, String)> = Vec::new();

        let mut applied = Vec::new();
        for c in self.locker.corrections(&devices, &self.settings) {
            self.our_volume_change.insert(c.key.clone(), Instant::now());
            match backend.set_volume(&c.key.id, c.key.direction, c.to) {
                Ok(()) => {
                    notes.push((
                        "Mic gain restored".to_string(),
                        format!(
                            "{}: something changed it to {:.0}%. Gain lock put it back to {:.0}%.",
                            c.name,
                            c.from * 100.0,
                            c.to * 100.0
                        ),
                    ));
                    msgs.push((
                        EventKind::Action,
                        format!("Gain lock: restored {} from {:.0}% to {:.0}%", c.name, c.from * 100.0, c.to * 100.0),
                    ));
                    changed = true;
                    applied.push(c);
                }
                Err(e) => msgs.push((EventKind::Error, format!("Gain lock on {} failed: {e:#}", c.name))),
            }
        }

        for key in self.switcher.check(&devices, &self.settings) {
            let name = devices.iter().find(|d| d.key() == key).map(|d| d.name.clone()).unwrap_or_default();
            match backend.set_default(&key.id, key.direction) {
                Ok(()) => {
                    msgs.push((EventKind::Action, format!("Auto-switched default {} to {name}", key.direction)));
                    changed = true;
                }
                Err(e) => msgs.push((EventKind::Error, format!("Auto-switch to {name} failed: {e:#}"))),
            }
        }

        if changed && let Ok(d) = backend.devices() {
            devices = d;
        }
        for (c, actual) in self.locker.observe(&applied, &devices) {
            msgs.push((
                EventKind::Warning,
                format!(
                    "{} can't be set to exactly {:.0}% (it uses volume steps); holding it at {:.0}%",
                    c.name,
                    c.to * 100.0,
                    actual * 100.0
                ),
            ));
        }
        for (kind, m) in msgs {
            self.log(kind, m);
        }
        for (title, body) in notes {
            self.notify(&title, &body);
        }
        devices
    }

    fn log_changes(&self, new: &[Device]) {
        if !self.have_listed {
            self.log(EventKind::Info, format!("Found {} audio devices", new.len()));
            return;
        }
        let old = &self.devices;
        for d in new {
            if !old.iter().any(|o| o.key() == d.key()) {
                self.log(EventKind::Info, format!("Connected: {} ({})", d.name, d.direction));
                if d.direction == Direction::Output && d.hands_free {
                    self.log(EventKind::Warning, format!("{} is in hands-free mode: audio quality is reduced", d.name));
                }
            }
        }
        for o in old {
            if !new.iter().any(|d| d.key() == o.key()) {
                self.log(EventKind::Warning, format!("Disconnected: {} ({})", o.name, o.direction));
                if o.is_default {
                    self.notify(&format!("{} disconnected", o.name), &format!("Your default {} is gone.", o.direction));
                }
            }
        }
        // Gain lowered by someone else (e.g. an app's automatic mic adjustment).
        for d in new {
            let Some(o) = old.iter().find(|o| o.key() == d.key()) else { continue };
            let (Some(before), Some(now)) = (o.volume, d.volume) else { continue };
            let ours = self.our_volume_change.get(&d.key()).is_some_and(|t| t.elapsed() < Duration::from_secs(3));
            let locked = self.settings.gain_lock_for(&d.key(), &d.name).is_some();
            if d.direction == Direction::Input && !ours && !locked && now < before - 0.10 {
                let apps = if d.in_use_by.is_empty() {
                    String::new()
                } else {
                    format!(" ({} is using it)", d.in_use_by.join(", "))
                };
                self.log(
                    EventKind::Warning,
                    format!(
                        "Mic gain of {} lowered from {:.0}% to {:.0}% by another app{apps}",
                        d.name,
                        before * 100.0,
                        now * 100.0
                    ),
                );
                self.notify(
                    "Mic gain lowered by another app",
                    &format!(
                        "{}: {:.0}% → {:.0}%{apps}. Use the gain lock (🔒) to prevent this.",
                        d.name,
                        before * 100.0,
                        now * 100.0
                    ),
                );
            }
        }
        for dir in Direction::ALL {
            let before = old.iter().find(|d| d.direction == dir && d.is_default).map(|d| &d.id);
            let after = new.iter().find(|d| d.direction == dir && d.is_default);
            if let Some(after) = after
                && before != Some(&after.id)
            {
                self.log(EventKind::Info, format!("Default {dir} is now {}", after.name));
            }
        }
        for d in new {
            if let Some(o) = old.iter().find(|o| o.key() == d.key())
                && d.direction == Direction::Output
                && d.hands_free
                && !o.hands_free
            {
                self.log(
                    EventKind::Warning,
                    format!("{} switched to hands-free mode: audio quality is reduced", d.name),
                );
                self.notify(
                    "Headset switched to hands-free mode",
                    &format!("{}: playback is now call quality because its mic is in use.", d.name),
                );
            }
        }
    }

    fn sync_meters(&mut self) {
        let backend = self.backend.as_ref().unwrap();
        let mut wanted: HashMap<DeviceKey, String> = HashMap::new();
        let mut native: HashMap<DeviceKey, Arc<LevelCell>> = HashMap::new();
        for d in &self.devices {
            let key = d.key();
            if backend.has_native_meter(d.direction) {
                let cell = self.native_cells.get(&key).cloned().unwrap_or_default();
                native.insert(key, cell);
                continue;
            }
            let auto = d.direction == Direction::Input
                && d.is_default
                && self.settings.monitor_default_input
                && (d.transport != Transport::Bluetooth || self.settings.monitor_bluetooth_inputs)
                && !self.hidden_meters.contains(&key);
            if (auto || self.monitored.contains(&key) || self.recorders.contains_key(&key))
                && let Some(cap) = backend.capture_id(&d.id, d.direction)
            {
                wanted.insert(key, cap);
            }
        }
        self.native_cells = native;
        self.meters.sync(&wanted);
    }

    fn apply_profile(&mut self, profile: &crate::profiles::Profile) {
        use crate::profiles::Action;
        if !self.ensure_backend() {
            self.log(EventKind::Error, "Cannot apply profile: audio system unavailable");
            return;
        }
        let devices = match self.backend.as_mut().unwrap().devices() {
            Ok(d) => d,
            Err(e) => {
                self.log(EventKind::Error, format!("Cannot apply profile: {e:#}"));
                return;
            }
        };
        let (actions, missing) = crate::profiles::plan(profile, &devices);
        for a in &actions {
            if let Action::SetVolume(k, _) = a {
                self.our_volume_change.insert(k.clone(), Instant::now());
            }
        }
        let backend = self.backend.as_mut().unwrap();
        let mut errors = Vec::new();
        for a in &actions {
            let r = match a {
                Action::SetDefault(k) => backend.set_default(&k.id, k.direction),
                Action::SetVolume(k, v) => backend.set_volume(&k.id, k.direction, *v),
                Action::SetMute(k, m) => backend.set_mute(&k.id, k.direction, *m),
            };
            if let Err(e) = r {
                errors.push(format!("{e:#}"));
            }
        }
        let mut msg = format!("Profile \"{}\" applied ({} change(s))", profile.name, actions.len() - errors.len());
        if !missing.is_empty() {
            msg += &format!("; not connected: {}", missing.join(", "));
        }
        self.log(if errors.is_empty() { EventKind::Action } else { EventKind::Warning }, msg);
        for e in errors {
            self.log(EventKind::Error, format!("Profile: {e}"));
        }
    }

    fn settings_name(&self, d: &Device) -> String {
        self.settings.display_name(d).to_string()
    }

    fn start_recording(&mut self, key: DeviceKey, audio: bool, auto: bool) {
        if self.recorders.contains_key(&key) {
            return;
        }
        let Some(device) = self.devices.iter().find(|d| d.key() == key).cloned() else {
            self.log(EventKind::Error, "Cannot record: device not found");
            return;
        };
        let dir = self.settings.recordings_dir();
        match SessionRecorder::start(&dir, &device, None, auto) {
            Ok(rec) => {
                let path = rec.info.csv_path.display().to_string();
                self.recorders.insert(key.clone(), rec);
                if audio {
                    self.want_audio.insert(key.clone());
                }
                if auto {
                    self.auto_recording.insert(key.clone(), None);
                }
                if auto {
                    self.notify(
                        "Recording call",
                        &format!("{} is in use by {}; logging levels.", device.name, device.in_use_by.join(", ")),
                    );
                }
                let why =
                    if auto { format!(" (call detected: {})", device.in_use_by.join(", ")) } else { String::new() };
                self.log(EventKind::Action, format!("Recording {}{why} → {path}", device.name));
                // Make sure a capture stream is running for it.
                self.sync_meters();
            }
            Err(e) => self.log(EventKind::Error, format!("Cannot start recording {}: {e:#}", device.name)),
        }
    }

    fn stop_recording(&mut self, key: &DeviceKey) {
        self.want_audio.remove(key);
        let Some(rec) = self.recorders.remove(key) else { return };
        let name = rec.info.device_name.clone();
        match rec.finish() {
            Ok((info, report, report_path)) => {
                let verdict = report.verdict.clone();
                {
                    let mut s = self.shared();
                    s.finished.push_back(FinishedRecording { info, report, report_path: report_path.clone() });
                    while s.finished.len() > 20 {
                        s.finished.pop_front();
                    }
                }
                self.notify(&format!("Recording of {name} saved"), &verdict);
                self.log(EventKind::Action, format!("Recording of {name} saved. {verdict}"));
            }
            Err(e) => self.log(EventKind::Error, format!("Finishing recording of {name} failed: {e:#}")),
        }
    }

    fn stop_all_recordings(&mut self) {
        let keys: Vec<DeviceKey> = self.recorders.keys().cloned().collect();
        for k in keys {
            self.stop_recording(&k);
        }
    }

    /// Start/stop automatic call recordings based on which apps use the inputs.
    fn auto_record(&mut self) {
        const GRACE: Duration = if cfg!(test) { Duration::from_millis(600) } else { Duration::from_secs(10) };
        if self.settings.auto_record_calls {
            let candidates: Vec<DeviceKey> = self
                .devices
                .iter()
                .filter(|d| d.direction == Direction::Input && self.settings.auto_record_matches(&d.in_use_by))
                .map(|d| d.key())
                .filter(|k| !self.recorders.contains_key(k))
                .collect();
            for key in candidates {
                let audio = self.settings.record_audio;
                self.start_recording(key, audio, true);
            }
        }
        // Stop auto recordings once the device has been idle (or gone) for a while.
        let now = Instant::now();
        let mut to_stop = Vec::new();
        for (key, idle_since) in self.auto_recording.iter_mut() {
            let busy = self.devices.iter().any(|d| d.key() == *key && !d.in_use_by.is_empty());
            if busy {
                *idle_since = None;
            } else {
                let since = *idle_since.get_or_insert(now);
                if now.duration_since(since) >= GRACE {
                    to_stop.push(key.clone());
                }
            }
        }
        for key in to_stop {
            self.auto_recording.remove(&key);
            self.log(EventKind::Info, "Call ended, stopping automatic recording");
            self.stop_recording(&key);
        }
    }

    fn tick_recorders(&mut self) {
        // Forward new event-log lines into the recordings (read under one lock).
        let forwarded = self.events_forwarded;
        let (new_events, total): (Vec<String>, u64) = {
            let s = self.shared();
            let n = (s.total_events - forwarded).min(s.events.len() as u64) as usize;
            let texts = s
                .events
                .iter()
                .skip(s.events.len() - n)
                // The recorder logs its own start/stop.
                .filter(|e| !e.text.starts_with("Recording "))
                .map(|e| e.text.clone())
                .collect();
            (texts, s.total_events)
        };
        self.events_forwarded = total;
        if self.recorders.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut failed = Vec::new();
        for (key, rec) in self.recorders.iter_mut() {
            for e in &new_events {
                rec.event(e.clone());
            }
            // Attach (or re-attach after a stream restart) the audio tap.
            if self.want_audio.contains(key)
                && let Some(tap) = self.meters.tap(key)
                && rec.audio_tap().is_none_or(|t| !Arc::ptr_eq(t, &tap))
            {
                let restarted = rec.audio_tap().is_some();
                if let Err(e) = rec.attach_audio(tap) {
                    failed.push((key.clone(), format!("audio: {e:#}")));
                } else if restarted {
                    rec.event("Audio stream restarted (new WAV part)");
                }
            }
            let reading = self
                .meters
                .cell(key)
                .or_else(|| self.native_cells.get(key).cloned())
                .map(|c| c.take_for(Consumer::Recorder))
                .unwrap_or_default();
            if let Err(e) = rec.tick(reading, now) {
                failed.push((key.clone(), format!("{e:#}")));
            }
        }
        for (key, err) in failed {
            self.log(EventKind::Error, format!("Recording error: {err}"));
            if !err.starts_with("audio:") {
                self.stop_recording(&key);
            }
        }
    }

    fn poll_native_meters(&mut self) {
        let Some(backend) = self.backend.as_mut() else { return };
        for (key, cell) in &self.native_cells {
            if let Some(p) = backend.native_peak(&key.id, key.direction) {
                cell.push_levels(p, p * std::f32::consts::FRAC_1_SQRT_2);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use crate::model::DeviceRef;

    fn wait_for(service: &Service, mut pred: impl FnMut(&Snapshot) -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if pred(&service.snapshot()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("condition not reached");
    }

    fn mock_factory(mock: MockBackend) -> BackendFactory {
        shared_mock_factory(Arc::new(Mutex::new(mock)))
    }

    fn shared_mock_factory(shared: Arc<Mutex<MockBackend>>) -> BackendFactory {
        Box::new(move || Ok(Box::new(SharedMock(shared.clone())) as Box<dyn AudioBackend>))
    }

    /// Mock wrapper whose state outlives backend reconnects.
    struct SharedMock(Arc<Mutex<MockBackend>>);
    impl AudioBackend for SharedMock {
        fn api_name(&self) -> &'static str {
            "Mock"
        }
        fn devices(&mut self) -> Result<Vec<Device>> {
            self.0.lock().unwrap().devices()
        }
        fn set_default(&mut self, id: &str, d: Direction) -> Result<()> {
            self.0.lock().unwrap().set_default(id, d)
        }
        fn set_volume(&mut self, id: &str, d: Direction, v: f32) -> Result<()> {
            self.0.lock().unwrap().set_volume(id, d, v)
        }
        fn set_mute(&mut self, id: &str, d: Direction, m: bool) -> Result<()> {
            self.0.lock().unwrap().set_mute(id, d, m)
        }
        // The mock devices have no real capture device behind them.
        fn capture_id(&self, _id: &str, _d: Direction) -> Option<String> {
            None
        }
    }

    #[test]
    fn service_lists_and_applies_requests() {
        let service = Service::start(Settings::default(), mock_factory(MockBackend::demo()));
        wait_for(&service, |s| s.devices.len() == 6);
        assert_eq!(service.snapshot().api, "Mock");

        let key = DeviceKey { id: "usb-mic".into(), direction: Direction::Input };
        service.send(Request::SetDefault(key.clone()));
        wait_for(&service, |s| s.devices.iter().any(|d| d.key() == key && d.is_default));
        service.send(Request::SetVolume(key.clone(), 0.25));
        wait_for(&service, |s| s.devices.iter().any(|d| d.key() == key && d.volume == Some(0.25)));
        assert!(service.snapshot().events.iter().any(|e| e.text.contains("Default input set to USB Microphone")));
    }

    #[test]
    fn service_enforces_gain_lock_and_auto_switch() {
        let mut settings = Settings::default();
        settings.set_gain_lock(
            DeviceRef { id: "builtin-mic".into(), name: "Built-in Microphone".into() },
            Direction::Input,
            Some(0.9),
        );
        settings.preferred_outputs = vec![DeviceRef { id: "hdmi-out".into(), name: "LG Monitor".into() }];
        let service = Service::start(settings, mock_factory(MockBackend::demo()));
        wait_for(&service, |s| {
            s.devices.iter().any(|d| d.id == "builtin-mic" && d.volume == Some(0.9))
                && s.devices.iter().any(|d| d.id == "hdmi-out" && d.is_default)
        });
        // Something else lowers the gain: it gets restored.
        let key = DeviceKey { id: "builtin-mic".into(), direction: Direction::Input };
        service.send(Request::SetVolume(key, 0.3));
        wait_for(&service, |s| s.events.iter().any(|e| e.text.contains("from 30% to 90%")));
        wait_for(&service, |s| s.devices.iter().any(|d| d.id == "builtin-mic" && d.volume == Some(0.9)));
    }

    #[test]
    fn service_reports_backend_errors() {
        let service = Service::start(Settings::default(), Box::new(|| anyhow::bail!("no sound server")));
        wait_for(&service, |s| s.error.as_deref() == Some("no sound server"));
        assert!(service.snapshot().devices.is_empty());
    }

    #[test]
    fn manual_recording_writes_files_and_report() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings { recordings_dir: Some(dir.path().to_path_buf()), ..Settings::default() };
        let service = Service::start(settings, mock_factory(MockBackend::demo()));
        wait_for(&service, |s| s.devices.len() == 6);
        let key = DeviceKey { id: "usb-mic".into(), direction: Direction::Input };
        service.send(Request::StartRecording { key: key.clone(), audio: false });
        wait_for(&service, |s| s.recordings.len() == 1);
        std::thread::sleep(Duration::from_millis(1300));
        // A gain change during the recording ends up in the log.
        service.send(Request::SetVolume(key.clone(), 0.2));
        wait_for(&service, |s| s.recordings.first().is_some_and(|r| r.recent.iter().any(|x| x.gain_pct == Some(20.0))));
        service.send(Request::StopRecording(key));
        wait_for(&service, |s| s.finished.len() == 1 && s.recordings.is_empty());
        let f = service.snapshot().finished[0].clone();
        assert!(f.info.csv_path.exists());
        assert!(f.report_path.exists());
        assert!(f.report.seconds_logged >= 2);
        let rows = crate::analysis::read_csv(&f.info.csv_path).unwrap();
        assert!(rows.iter().any(|r| r.gain_pct == Some(75.0)) && rows.iter().any(|r| r.gain_pct == Some(20.0)));
    }

    #[test]
    fn auto_records_while_another_app_uses_the_mic() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            recordings_dir: Some(dir.path().to_path_buf()),
            auto_record_calls: true,
            auto_record_apps: vec!["teams".into()],
            ..Settings::default()
        };
        let mock = Arc::new(Mutex::new(MockBackend::demo()));
        let service = Service::start(settings, shared_mock_factory(mock.clone()));
        wait_for(&service, |s| s.devices.len() == 6);

        // Zoom uses the mic: filtered out, no recording.
        mock.lock().unwrap().devices[0].in_use_by = vec!["Zoom".into()];
        std::thread::sleep(Duration::from_millis(1200));
        assert!(service.snapshot().recordings.is_empty());

        // A Teams call starts.
        mock.lock().unwrap().devices[0].in_use_by = vec!["Microsoft Teams".into()];
        wait_for(&service, |s| s.recordings.len() == 1 && s.recordings[0].info.auto);
        assert!(service.snapshot().events.iter().any(|e| e.text.contains("call detected: Microsoft Teams")));

        // Call ends: recording stops after the grace period.
        mock.lock().unwrap().devices[0].in_use_by.clear();
        wait_for(&service, |s| s.recordings.is_empty() && s.finished.len() == 1);
        assert!(service.snapshot().finished[0].info.auto);
    }

    #[test]
    fn detects_gain_lowered_by_another_app() {
        let mock = Arc::new(Mutex::new(MockBackend::demo()));
        let service = Service::start(Settings::default(), shared_mock_factory(mock.clone()));
        wait_for(&service, |s| s.devices.len() == 6);
        // Our own change: not reported.
        service.send(Request::SetVolume(DeviceKey { id: "usb-mic".into(), direction: Direction::Input }, 0.2));
        wait_for(&service, |s| s.devices.iter().any(|d| d.id == "usb-mic" && d.volume == Some(0.2)));
        // Another app (behind our back) lowers the built-in mic from 75% to 30%.
        {
            let mut m = mock.lock().unwrap();
            m.devices[0].volume = Some(0.3);
            m.devices[0].in_use_by = vec!["Microsoft Teams".into()];
        }
        wait_for(&service, |s| {
            s.events
                .iter()
                .any(|e| e.text.contains("lowered from 75% to 30% by another app (Microsoft Teams is using it)"))
        });
        assert!(!service.snapshot().events.iter().any(|e| e.text.contains("USB Microphone lowered")));
    }
}
