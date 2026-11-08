//! Session recorder: logs a device once per second to CSV (levels, OS gain,
//! mute, signal status, Bluetooth mode, apps using it, events) and can also
//! record the audio to a WAV file. On finish it writes a drop-analysis report.
//!
//! Files land in the recordings folder (see [`recordings_dir`]):
//! `2026-11-09_101500_Headset.csv`, `….wav`, `….report.txt`, `….report.json`.

use crate::analysis::{self, Report, Sample};
use crate::levels::{LevelReading, MeterState, SignalStatus, to_db};
use crate::meter::AudioTap;
use crate::model::{Device, DeviceKey};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Sample rate audio recordings are reduced to when the device rate allows
/// (plenty for listening back to speech and keeps files small: ~115 MB/hour).
pub const WAV_TARGET_RATE: u32 = 16_000;

/// Where recordings are stored by default.
pub fn recordings_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|u| u.document_dir().map(|d| d.join("Audio Interface Recordings")))
        .or_else(|| {
            directories::ProjectDirs::from("com", "ikjbi", "AudioInterface").map(|d| d.data_dir().join("recordings"))
        })
        .unwrap_or_else(|| PathBuf::from("recordings"))
}

/// Public description of a recording in progress or finished.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecordingInfo {
    pub key: DeviceKey,
    pub device_name: String,
    pub csv_path: PathBuf,
    pub wav_paths: Vec<PathBuf>,
    /// Local start time, e.g. `2026-11-09 10:15:00`.
    pub started: String,
    /// Started automatically because another app began using the device.
    pub auto: bool,
}

pub struct SessionRecorder {
    pub info: RecordingInfo,
    started: Instant,
    csv: csv::Writer<std::fs::File>,
    rows: Vec<Sample>,
    seconds_written: u64,
    acc_peak: f32,
    acc_energy: f64,
    acc_samples: u64,
    meter: MeterState,
    last_tick: Instant,
    device: Option<Device>,
    events: Vec<String>,
    wav: Option<WavThread>,
}

fn now_local(fmt: &str) -> String {
    jiff::Zoned::now().strftime(fmt).to_string()
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    s.chars().take(60).collect()
}

impl SessionRecorder {
    /// Start recording `device`. Pass `tap` to also record audio.
    pub fn start(dir: &Path, device: &Device, tap: Option<Arc<AudioTap>>, auto: bool) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let stem = format!("{}_{}", now_local("%Y-%m-%d_%H%M%S"), sanitize(&device.name));
        let csv_path = dir.join(format!("{stem}.csv"));
        let csv = csv::Writer::from_path(&csv_path).with_context(|| format!("create {}", csv_path.display()))?;
        let now = Instant::now();
        let mut rec = SessionRecorder {
            info: RecordingInfo {
                key: device.key(),
                device_name: device.name.clone(),
                csv_path,
                wav_paths: Vec::new(),
                started: now_local("%Y-%m-%d %H:%M:%S"),
                auto,
            },
            started: now,
            csv,
            rows: Vec::new(),
            seconds_written: 0,
            acc_peak: 0.0,
            acc_energy: 0.0,
            acc_samples: 0,
            meter: MeterState::new(now),
            last_tick: now,
            device: Some(device.clone()),
            events: vec![format!("Recording started ({})", if auto { "automatic" } else { "manual" })],
            wav: None,
        };
        if let Some(tap) = tap {
            rec.attach_audio(tap)?;
        }
        Ok(rec)
    }

    pub fn records_audio(&self) -> bool {
        self.wav.is_some()
    }

    /// Start (or restart, after the capture stream was re-opened) the WAV writer.
    pub fn attach_audio(&mut self, tap: Arc<AudioTap>) -> Result<()> {
        if let Some(w) = self.wav.take() {
            w.finish();
        }
        let part = self.info.wav_paths.len() + 1;
        let stem = self.info.csv_path.with_extension("");
        let path = if part == 1 {
            stem.with_extension("wav")
        } else {
            PathBuf::from(format!("{}_part{part}.wav", stem.display()))
        };
        self.wav = Some(WavThread::start(&path, tap)?);
        self.info.wav_paths.push(path);
        Ok(())
    }

    /// The tap currently recorded from, so the caller can detect stream restarts.
    pub fn audio_tap(&self) -> Option<&Arc<AudioTap>> {
        self.wav.as_ref().map(|w| &w.tap)
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn rows(&self) -> &[Sample] {
        &self.rows
    }

    pub fn update_device(&mut self, device: Option<&Device>) {
        self.device = device.cloned();
    }

    pub fn event(&mut self, text: impl Into<String>) {
        self.events.push(text.into());
    }

    /// Feed one level reading (called every service tick, ~30 ms).
    pub fn tick(&mut self, reading: LevelReading, now: Instant) -> Result<()> {
        let dt = now.duration_since(self.last_tick);
        self.last_tick = now;
        self.meter.update(reading, dt, now);
        if reading.peak > 0.0 || reading.rms > 0.0 {
            self.acc_peak = self.acc_peak.max(reading.peak);
        }
        self.acc_energy += reading.energy;
        self.acc_samples += reading.samples;
        while now.duration_since(self.started).as_secs() > self.seconds_written {
            self.write_row()?;
        }
        Ok(())
    }

    fn write_row(&mut self) -> Result<()> {
        // Exact RMS over the samples that arrived during this second. Audio
        // arrives in blocks (up to ~200 ms on PulseAudio), so a second can
        // be empty. Then repeat the previous value instead of logging silence.
        let rms = if self.acc_samples > 0 {
            (self.acc_energy / self.acc_samples as f64).sqrt() as f32
        } else {
            self.rows.last().map(|r| crate::levels::from_db(r.rms_db)).unwrap_or(0.0)
        };
        if self.acc_samples == 0
            && let Some(last) = self.rows.last()
        {
            self.acc_peak = self.acc_peak.max(crate::levels::from_db(last.peak_db));
        }
        let d = self.device.as_ref();
        let status = if d.is_none() {
            "lost"
        } else {
            match self.meter.status {
                SignalStatus::Ok => "ok",
                SignalStatus::Silent => "silent",
                SignalStatus::Lost => "lost",
                SignalStatus::Starting => "starting",
            }
        };
        let row = Sample {
            t: self.seconds_written as f64,
            time: now_local("%H:%M:%S"),
            peak_db: to_db(self.acc_peak),
            rms_db: to_db(rms),
            gain_pct: d.and_then(|d| d.volume).map(|v| (v * 1000.0).round() / 10.0),
            muted: d.and_then(|d| d.muted),
            status: status.to_string(),
            hands_free: d.is_some_and(|d| d.hands_free),
            in_use_by: d.map(|d| d.in_use_by.join(";")).unwrap_or_default(),
            event: std::mem::take(&mut self.events).join("; "),
        };
        self.csv.serialize(&row)?;
        // Flush every 10 s so a crash loses little.
        if self.seconds_written.is_multiple_of(10) {
            self.csv.flush()?;
        }
        self.rows.push(row);
        self.seconds_written += 1;
        self.acc_peak = 0.0;
        self.acc_energy = 0.0;
        self.acc_samples = 0;
        Ok(())
    }

    /// Stop, flush everything and write the analysis report.
    pub fn finish(mut self) -> Result<(RecordingInfo, Report, PathBuf)> {
        self.events.push("Recording stopped".into());
        self.write_row()?;
        self.csv.flush()?;
        if let Some(w) = self.wav.take() {
            w.finish();
        }
        let report = analysis::analyze(&self.rows);
        let base = self.info.csv_path.with_extension("");
        let txt = PathBuf::from(format!("{}.report.txt", base.display()));
        let title = format!("Audio session report: {} ({})", self.info.device_name, self.info.started);
        std::fs::write(&txt, report.to_text(&title))?;
        std::fs::write(
            PathBuf::from(format!("{}.report.json", base.display())),
            serde_json::to_string_pretty(&report)?,
        )?;
        Ok((self.info, report, txt))
    }
}

/// Drains an [`AudioTap`] into a 16-bit mono WAV file on its own thread.
struct WavThread {
    tap: Arc<AudioTap>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<()>>>,
}

impl WavThread {
    fn start(path: &Path, tap: Arc<AudioTap>) -> Result<Self> {
        let rate = tap.sample_rate;
        // Reduce to 16 kHz by averaging when the rate is a multiple of it
        // (48k -> /3, 32k -> /2). Otherwise keep the native rate.
        let factor =
            if rate > WAV_TARGET_RATE && rate.is_multiple_of(WAV_TARGET_RATE) { rate / WAV_TARGET_RATE } else { 1 };
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: rate / factor,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).with_context(|| format!("create {}", path.display()))?;
        let mut rx = tap.attach(10);
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = std::thread::Builder::new().name("wav-writer".into()).spawn(move || -> Result<()> {
            let (mut acc, mut n) = (0.0f32, 0u32);
            loop {
                let stopping = stop2.load(Ordering::SeqCst);
                while let Ok(x) = rx.pop() {
                    acc += x;
                    n += 1;
                    if n == factor {
                        let v = (acc / n as f32).clamp(-1.0, 1.0);
                        writer.write_sample((v * i16::MAX as f32) as i16)?;
                        acc = 0.0;
                        n = 0;
                    }
                }
                if stopping {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            writer.finalize()?;
            Ok(())
        })?;
        Ok(WavThread { tap, stop, handle: Some(handle) })
    }

    fn finish(mut self) {
        self.tap.detach();
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            match h.join() {
                Ok(Err(e)) => log::error!("WAV writer: {e:#}"),
                Err(_) => log::error!("WAV writer panicked"),
                Ok(Ok(())) => {}
            }
        }
    }
}

/// Finished recordings in `dir`, newest first.
pub fn list_recordings(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == "csv")).collect()
        })
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Direction, test_device};

    #[test]
    fn records_rows_events_and_report() {
        let dir = tempfile::tempdir().unwrap();
        let mut dev = test_device("mic", "Test Mic: USB/1", Direction::Input);
        dev.in_use_by = vec!["Microsoft Teams".into()];
        let mut rec = SessionRecorder::start(dir.path(), &dev, None, true).unwrap();
        assert!(rec.info.csv_path.file_name().unwrap().to_string_lossy().ends_with("_Test-Mic-USB-1.csv"));

        // Simulate 3.5 s of ticks at 0.1 amplitude, with a gain change midway.
        let t0 = rec.started;
        for i in 1..=35u64 {
            let blocks = i;
            if i == 20 {
                dev.volume = Some(0.25);
                rec.update_device(Some(&dev));
                rec.event("Gain changed by test");
            }
            // 100 samples per tick at RMS 0.05.
            let r = LevelReading {
                peak: 0.1,
                rms: 0.05,
                blocks,
                non_silent_blocks: blocks,
                energy: 100.0 * 0.0025,
                samples: 100,
                ..Default::default()
            };
            rec.tick(r, t0 + Duration::from_millis(i * 100)).unwrap();
        }
        assert_eq!(rec.rows().len(), 3);
        let r0 = &rec.rows()[0];
        assert!((r0.peak_db - -20.0).abs() < 0.1, "{}", r0.peak_db);
        assert!((r0.rms_db - -26.02).abs() < 0.1, "{}", r0.rms_db);
        assert_eq!(r0.gain_pct, Some(50.0));
        assert_eq!(r0.in_use_by, "Microsoft Teams");
        assert!(r0.event.contains("Recording started (automatic)"));
        assert_eq!(rec.rows()[2].gain_pct, Some(25.0));
        assert!(rec.rows()[1].event.contains("Gain changed by test"));

        let (info, report, txt) = rec.finish().unwrap();
        assert_eq!(report.seconds_logged, 4);
        assert!(txt.exists());
        assert!(PathBuf::from(format!("{}.report.json", info.csv_path.with_extension("").display())).exists());
        let back = analysis::read_csv(&info.csv_path).unwrap();
        assert_eq!(back.len(), 4);
        assert!(back[3].event.contains("Recording stopped"));
        assert!(
            report
                .findings
                .iter()
                .any(|f| matches!(f, analysis::Finding::GainChange { from, to, .. } if *from == 50.0 && *to == 25.0))
        );
        assert_eq!(list_recordings(dir.path()), vec![info.csv_path]);
    }

    #[test]
    fn records_audio_to_16k_wav() {
        let dir = tempfile::tempdir().unwrap();
        let dev = test_device("mic", "Mic", Direction::Input);
        let tap = Arc::new(AudioTap::new(48_000, 2));
        let rec = SessionRecorder::start(dir.path(), &dev, Some(tap.clone()), false).unwrap();
        assert!(rec.records_audio());
        // 1 s of a 440 Hz sine at 0.5 amplitude, stereo, fed in 10 ms blocks.
        let frames: Vec<f32> = (0..48_000)
            .flat_map(|i| {
                let v = 0.5 * (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / 48_000.0).sin();
                [v, v]
            })
            .collect();
        for block in frames.chunks(960) {
            tap.feed(block);
            std::thread::sleep(Duration::from_millis(1));
        }
        let (info, _, _) = rec.finish().unwrap();
        let r = hound::WavReader::open(&info.wav_paths[0]).unwrap();
        assert_eq!(r.spec().sample_rate, 16_000);
        assert_eq!(r.spec().channels, 1);
        let samples: Vec<i16> = r.into_samples::<i16>().map(|s| s.unwrap()).collect();
        assert!((15_900..=16_000).contains(&samples.len()), "{}", samples.len());
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap() as f32 / i16::MAX as f32;
        assert!((0.45..=0.5).contains(&peak), "peak {peak}");
    }

    #[test]
    fn device_disappearing_is_logged_as_lost() {
        let dir = tempfile::tempdir().unwrap();
        let dev = test_device("mic", "Mic", Direction::Input);
        let mut rec = SessionRecorder::start(dir.path(), &dev, None, false).unwrap();
        rec.update_device(None);
        let t0 = rec.started;
        rec.tick(LevelReading::default(), t0 + Duration::from_millis(1100)).unwrap();
        assert_eq!(rec.rows()[0].status, "lost");
        assert_eq!(rec.rows()[0].gain_pct, None);
    }
}
