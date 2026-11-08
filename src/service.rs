//! Background worker that owns the OS backend and the meter engine.
//!
//! The UI never touches the OS directly. It sends [`Request`]s and reads a shared
//! [`Snapshot`]. The worker:
//! * refreshes the device list a few times per second (so hot-plug is picked up),
//! * enforces gain locks and auto-switch rules,
//! * keeps capture streams running for the devices being metered,
//! * polls native peak meters (Windows outputs),
//! * records a human-readable event log.

use crate::backend::AudioBackend;
use crate::levels::LevelCell;
use crate::meter::MeterEngine;
use crate::model::{Device, DeviceKey, Direction, Transport};
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
    SetVolume(DeviceKey, f32),
    SetMute(DeviceKey, bool),
    /// Start/stop metering a device on user request.
    SetMonitored(DeviceKey, bool),
    UpdateSettings(Settings),
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
    pub meter_host: &'static str,
    pub devices: Vec<Device>,
    /// Backend-level error (e.g. sound server not running).
    pub error: Option<String>,
    pub events: VecDeque<Event>,
    /// Level cells for every metered device.
    pub levels: HashMap<DeviceKey, Arc<LevelCell>>,
    /// Why metering failed for a device.
    pub meter_errors: HashMap<DeviceKey, String>,
    /// Devices the user asked to meter (on top of the automatic ones).
    pub monitored: BTreeSet<DeviceKey>,
    /// Devices whose meter comes from a native OS peak meter (no capture stream).
    pub native_metered: BTreeSet<DeviceKey>,
    /// Incremented on every device refresh.
    pub generation: u64,
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
    devices: Vec<Device>,
    have_listed: bool,
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
            devices: Vec::new(),
            have_listed: false,
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
        }
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
                self.backend = Some(b);
                let mut s = self.shared();
                s.api = api;
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

    fn handle(&mut self, req: Request) {
        match req {
            Request::UpdateSettings(s) => {
                self.settings = s;
                return;
            }
            Request::SetMonitored(key, on) => {
                if on {
                    self.monitored.insert(key);
                } else {
                    self.monitored.remove(&key);
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
        let name = match &req {
            Request::SetDefault(k) | Request::SetVolume(k, _) | Request::SetMute(k, _) => self.name_of(k),
            _ => String::new(),
        };
        let backend = self.backend.as_mut().unwrap();
        let result = match &req {
            Request::SetDefault(k) => {
                backend.set_default(&k.id, k.direction).map(|_| format!("Default {} set to {name}", k.direction))
            }
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
        let native: BTreeSet<DeviceKey> = self.native_cells.keys().cloned().collect();
        let devices = self.devices.clone();
        let monitored = self.monitored.clone();
        let mut s = self.shared();
        s.devices = devices;
        s.levels = levels;
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

        let mut applied = Vec::new();
        for c in self.locker.corrections(&devices, &self.settings) {
            match backend.set_volume(&c.key.id, c.key.direction, c.to) {
                Ok(()) => {
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
                && (d.transport != Transport::Bluetooth || self.settings.monitor_bluetooth_inputs);
            if (auto || self.monitored.contains(&key))
                && let Some(cap) = backend.capture_id(&d.id, d.direction)
            {
                wanted.insert(key, cap);
            }
        }
        self.native_cells = native;
        self.meters.sync(&wanted);
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
        let shared = Arc::new(Mutex::new(mock));
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
}
