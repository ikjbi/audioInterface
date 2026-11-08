//! Live level metering via `cpal` capture streams.
//!
//! Each monitored device gets its own capture stream. The realtime callback
//! converts samples to f32 and pushes peak/RMS into a lock-free [`LevelCell`],
//! which the UI drains every frame.
//!
//! Streams are opened on short-lived helper threads. Opening a device can block
//! for a long time, for example while macOS waits for the user to answer the
//! microphone permission prompt, and that must never stall the service loop.
//!
//! Note: opening a Bluetooth headset's microphone switches the headset to its
//! hands-free profile, the same as joining a call would. The UI warns about this
//! and only meters Bluetooth mics on request.

use crate::levels::LevelCell;
use crate::model::DeviceKey;
use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Optional raw-audio tap on a capture stream, used to record audio to disk.
/// The realtime callback downmixes to mono and pushes into a lock-free ring
/// buffer, but only while a recorder is attached. It never blocks: it uses
/// `try_lock` and drops samples if the ring is full.
pub struct AudioTap {
    pub sample_rate: u32,
    pub channels: u16,
    producer: Mutex<Option<rtrb::Producer<f32>>>,
}

impl AudioTap {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        AudioTap { sample_rate, channels: channels.max(1), producer: Mutex::new(None) }
    }

    /// Start receiving mono audio. `seconds` of buffering are allocated.
    pub fn attach(&self, seconds: u32) -> rtrb::Consumer<f32> {
        let (producer, consumer) = rtrb::RingBuffer::new((self.sample_rate * seconds.max(1)) as usize);
        *self.producer.lock().unwrap_or_else(|e| e.into_inner()) = Some(producer);
        consumer
    }

    pub fn detach(&self) {
        *self.producer.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Called from the audio thread.
    pub(crate) fn feed(&self, interleaved: &[f32]) {
        let Ok(mut guard) = self.producer.try_lock() else { return };
        let Some(p) = guard.as_mut() else { return };
        let ch = self.channels as usize;
        for frame in interleaved.chunks_exact(ch) {
            let mono = frame.iter().sum::<f32>() / ch as f32;
            if p.push(mono).is_err() {
                break; // writer fell behind; drop rather than block
            }
        }
    }
}

struct Monitor {
    cell: Arc<LevelCell>,
    window: Arc<crate::levels::SampleWindow>,
    tap: Arc<AudioTap>,
    error: Arc<Mutex<Option<String>>>,
    capture_id: String,
    started: Instant,
    _stream: cpal::Stream,
}

struct Opened {
    key: DeviceKey,
    capture_id: String,
    result: Result<Monitor>,
}

pub struct MeterEngine {
    host_name: &'static str,
    monitors: HashMap<DeviceKey, Monitor>,
    /// Opens in flight: key -> (capture id, started at).
    pending: HashMap<DeviceKey, (String, Instant)>,
    /// Devices we failed to open, with the time and error, so we don't retry
    /// every tick.
    failures: HashMap<DeviceKey, (Instant, String)>,
    tx: Sender<Opened>,
    rx: Receiver<Opened>,
}

/// How long to wait before retrying a device that failed to open.
const RETRY_AFTER: Duration = Duration::from_secs(5);
/// An open taking longer than this is reported as probably blocked on permissions.
const SLOW_OPEN: Duration = Duration::from_secs(4);

impl Default for MeterEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MeterEngine {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        MeterEngine {
            host_name: cpal::default_host().id().name(),
            monitors: HashMap::new(),
            pending: HashMap::new(),
            failures: HashMap::new(),
            tx,
            rx,
        }
    }

    pub fn host_name(&self) -> &'static str {
        self.host_name
    }

    pub fn cell(&self, key: &DeviceKey) -> Option<Arc<LevelCell>> {
        self.monitors.get(key).map(|m| m.cell.clone())
    }

    /// Recent samples of a running capture stream (for the spectrum view).
    pub fn window(&self, key: &DeviceKey) -> Option<Arc<crate::levels::SampleWindow>> {
        self.monitors.get(key).map(|m| m.window.clone())
    }

    /// Raw-audio tap of a running capture stream (for recording audio).
    pub fn tap(&self, key: &DeviceKey) -> Option<Arc<AudioTap>> {
        self.monitors.get(key).map(|m| m.tap.clone())
    }

    pub fn error(&self, key: &DeviceKey) -> Option<String> {
        if let Some(m) = self.monitors.get(key) {
            return m.error.lock().ok().and_then(|e| e.clone());
        }
        if let Some((_, since)) = self.pending.get(key)
            && since.elapsed() > SLOW_OPEN
        {
            return Some(slow_open_hint().to_string());
        }
        self.failures.get(key).map(|(_, e)| e.clone())
    }

    pub fn is_monitoring(&self, key: &DeviceKey) -> bool {
        self.monitors.contains_key(key)
    }

    pub fn is_pending(&self, key: &DeviceKey) -> bool {
        self.pending.contains_key(key)
    }

    /// Make the set of running streams match `wanted` (key -> cpal capture id):
    /// start the missing ones and stop the rest. Streams that reported an error,
    /// e.g. because the device was unplugged, are restarted after a delay.
    pub fn sync(&mut self, wanted: &HashMap<DeviceKey, String>) {
        self.collect_opened(wanted);
        self.monitors.retain(|k, m| wanted.get(k) == Some(&m.capture_id));
        self.failures.retain(|k, _| wanted.contains_key(k));

        // Restart streams that died.
        let now = Instant::now();
        let dead: Vec<DeviceKey> = self
            .monitors
            .iter()
            .filter(|(_, m)| {
                m.error.lock().map(|e| e.is_some()).unwrap_or(false) && now.duration_since(m.started) > RETRY_AFTER
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in dead {
            log::info!("restarting meter for {}", k.id);
            self.monitors.remove(&k);
        }

        for (key, capture_id) in wanted {
            if self.monitors.contains_key(key) || self.pending.contains_key(key) {
                continue;
            }
            if let Some((at, _)) = self.failures.get(key)
                && now.duration_since(*at) < RETRY_AFTER
            {
                continue;
            }
            self.spawn_open(key.clone(), capture_id.clone());
        }
    }

    /// Wait (up to `timeout`) until `key` is running or has failed. Used by
    /// the CLI. Returns `Ok(())` once the stream is running.
    pub fn wait_ready(
        &mut self,
        key: &DeviceKey,
        wanted: &HashMap<DeviceKey, String>,
        timeout: Duration,
    ) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            self.collect_opened(wanted);
            if self.monitors.contains_key(key) {
                return Ok(());
            }
            if let Some((_, e)) = self.failures.get(key) {
                bail!("{e}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        bail!("{}", slow_open_hint())
    }

    pub fn stop_all(&mut self) {
        self.monitors.clear();
    }

    fn spawn_open(&mut self, key: DeviceKey, capture_id: String) {
        self.pending.insert(key.clone(), (capture_id.clone(), Instant::now()));
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new().name("meter-open".into()).spawn(move || {
            let result = open(&capture_id);
            let _ = tx.send(Opened { key, capture_id, result });
        });
        if let Err(e) = spawned {
            log::error!("cannot spawn meter thread: {e}");
        }
    }

    fn collect_opened(&mut self, wanted: &HashMap<DeviceKey, String>) {
        while let Ok(o) = self.rx.try_recv() {
            self.pending.remove(&o.key);
            if wanted.get(&o.key) != Some(&o.capture_id) {
                continue; // no longer wanted; dropping the stream stops it
            }
            match o.result {
                Ok(m) => {
                    self.failures.remove(&o.key);
                    self.monitors.insert(o.key, m);
                }
                Err(e) => {
                    log::warn!("cannot meter {}: {e:#}", o.capture_id);
                    self.failures.insert(o.key, (Instant::now(), explain_open_error(&format!("{e:#}"))));
                }
            }
        }
    }
}

/// Add a hint to errors with a known cause.
fn explain_open_error(err: &str) -> String {
    let denied = err.contains("Access is denied") || err.contains("0x80070005") || err.contains("-2147024891");
    if denied {
        format!(
            "{err}. The OS is blocking microphone access. Check Settings → Privacy & security → Microphone \
             (\"Let desktop apps access your microphone\")"
        )
    } else {
        err.to_string()
    }
}

fn slow_open_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "the device is taking long to open. Make sure this app is allowed under System Settings → Privacy & Security → Microphone"
    } else if cfg!(windows) {
        "the device is taking long to open. Check Settings → Privacy & security → Microphone"
    } else {
        "the device is taking long to open"
    }
}

fn find_device(host: &cpal::Host, capture_id: &str) -> Result<cpal::Device> {
    let id = cpal::DeviceId::new(host.id(), capture_id);
    if let Some(d) = host.device_by_id(&id) {
        return Ok(d);
    }
    // Fallback: scan input devices and compare IDs.
    for d in host.input_devices().context("enumerate capture devices")? {
        if d.id().map(|i| i.id() == capture_id).unwrap_or(false) {
            return Ok(d);
        }
    }
    bail!("capture device not found")
}

fn open(capture_id: &str) -> Result<Monitor> {
    let host = cpal::default_host();
    let device = find_device(&host, capture_id)?;
    let supported = device.default_input_config().context("query input format")?;
    let config = supported.config();
    let cell = Arc::new(LevelCell::default());
    let error = Arc::new(Mutex::new(None));
    let tap = Arc::new(AudioTap::new(config.sample_rate, config.channels));
    let window = Arc::new(crate::levels::SampleWindow::new(8192, config.sample_rate));

    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, &cell, &tap, &window, &error),
        SampleFormat::F64 => build::<f64>(&device, config, &cell, &tap, &window, &error),
        SampleFormat::I8 => build::<i8>(&device, config, &cell, &tap, &window, &error),
        SampleFormat::I16 => build::<i16>(&device, config, &cell, &tap, &window, &error),
        SampleFormat::I24 => build::<cpal::I24>(&device, config, &cell, &tap, &window, &error),
        SampleFormat::I32 => build::<i32>(&device, config, &cell, &tap, &window, &error),
        SampleFormat::U8 => build::<u8>(&device, config, &cell, &tap, &window, &error),
        SampleFormat::U16 => build::<u16>(&device, config, &cell, &tap, &window, &error),
        SampleFormat::U32 => build::<u32>(&device, config, &cell, &tap, &window, &error),
        other => Err(anyhow!("unsupported sample format {other:?}")),
    }?;
    stream.play().context("start capture stream")?;
    Ok(Monitor {
        cell,
        window,
        tap,
        error,
        capture_id: capture_id.to_string(),
        started: Instant::now(),
        _stream: stream,
    })
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    cell: &Arc<LevelCell>,
    tap: &Arc<AudioTap>,
    window: &Arc<crate::levels::SampleWindow>,
    error: &Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let cell = cell.clone();
    let tap = tap.clone();
    let window = window.clone();
    let channels = config.channels as usize;
    let err = error.clone();
    let mut buf: Vec<f32> = Vec::with_capacity(16 * 1024);
    let stream = device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _info| {
            buf.clear();
            buf.extend(data.iter().map(|s| s.to_sample::<f32>()));
            cell.push(&buf);
            tap.feed(&buf);
            window.push_interleaved(&buf, channels);
        },
        move |e| {
            log::warn!("capture stream error: {e}");
            if let Ok(mut slot) = err.lock() {
                *slot = Some(e.to_string());
            }
        },
        None,
    )?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    #[test]
    fn tap_downmixes_and_only_feeds_when_attached() {
        let tap = super::AudioTap::new(4, 2);
        tap.feed(&[1.0, 0.0]); // not attached: dropped
        let mut rx = tap.attach(2); // 8 slots
        tap.feed(&[1.0, 0.0, 0.5, 0.5, -1.0, -1.0]);
        let got: Vec<f32> = std::iter::from_fn(|| rx.pop().ok()).collect();
        assert_eq!(got, vec![0.5, 0.5, -1.0]);
        tap.feed(&[0.0; 40]); // overflow: drops, never blocks
        assert_eq!(std::iter::from_fn(|| rx.pop().ok()).count(), 8);
        tap.detach();
        tap.feed(&[1.0, 1.0]);
        assert!(rx.pop().is_err());
    }

    #[test]
    fn access_denied_gets_privacy_hint() {
        let e =
            super::explain_open_error("Failed to initialize audio client: Access is denied. (os error -2147024891)");
        assert!(e.contains("Microphone"), "{e}");
        assert_eq!(super::explain_open_error("device not found"), "device not found");
    }
}
