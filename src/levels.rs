//! Level math and meter ballistics. Pure functions + small state machines with
//! no audio I/O, so it is all unit-testable.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Anything quieter than this is shown as "silence" (-inf).
pub const FLOOR_DB: f32 = -96.0;

/// Convert a linear amplitude (0..=1, 1 = full scale) to dBFS.
pub fn to_db(linear: f32) -> f32 {
    if linear <= 0.0 || !linear.is_finite() {
        FLOOR_DB
    } else {
        (20.0 * linear.log10()).max(FLOOR_DB)
    }
}

/// Convert dBFS to linear amplitude.
pub fn from_db(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Peak absolute sample value of a block.
pub fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0f32, |m, s| if s.is_finite() { m.max(s.abs()) } else { m })
}

/// Root-mean-square of a block.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().filter(|s| s.is_finite()).map(|&s| (s as f64) * (s as f64)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// Map dBFS to a 0..=1 position on a meter whose scale starts at `min_db`.
pub fn meter_position(db: f32, min_db: f32) -> f32 {
    ((db - min_db) / -min_db).clamp(0.0, 1.0)
}

/// Lock-free hand-off of levels from the realtime audio callback to the UI.
///
/// The callback calls [`LevelCell::push`] for each block. The reader calls
/// [`LevelCell::take`], which returns the maxima since the previous read. That
/// way short transients are never lost between UI frames. Non-negative f32 values
/// compare the same way as their bit patterns do, so `fetch_max` on the bits
/// works without locks.
#[derive(Debug)]
pub struct LevelCell {
    peak_bits: AtomicU32,
    rms_bits: AtomicU32,
    /// Number of audio blocks received in total (detects stalled streams).
    blocks: AtomicU64,
    /// Number of blocks that contained at least one non-zero sample.
    non_silent_blocks: AtomicU64,
}

impl Default for LevelCell {
    fn default() -> Self {
        LevelCell {
            peak_bits: AtomicU32::new(0),
            rms_bits: AtomicU32::new(0),
            blocks: AtomicU64::new(0),
            non_silent_blocks: AtomicU64::new(0),
        }
    }
}

/// Values read out of a [`LevelCell`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LevelReading {
    pub peak: f32,
    pub rms: f32,
    pub blocks: u64,
    pub non_silent_blocks: u64,
}

impl LevelCell {
    /// Called from the audio thread. Must not block or allocate.
    pub fn push(&self, samples: &[f32]) {
        let p = peak(samples);
        let r = rms(samples);
        self.push_levels(p, r);
    }

    pub fn push_levels(&self, peak: f32, rms: f32) {
        self.peak_bits.fetch_max(peak.max(0.0).to_bits(), Ordering::Relaxed);
        self.rms_bits.fetch_max(rms.max(0.0).to_bits(), Ordering::Relaxed);
        self.blocks.fetch_add(1, Ordering::Relaxed);
        if peak > 0.0 {
            self.non_silent_blocks.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Read and reset the peak/RMS maxima. Block counters are cumulative.
    pub fn take(&self) -> LevelReading {
        LevelReading {
            peak: f32::from_bits(self.peak_bits.swap(0, Ordering::Relaxed)),
            rms: f32::from_bits(self.rms_bits.swap(0, Ordering::Relaxed)),
            blocks: self.blocks.load(Ordering::Relaxed),
            non_silent_blocks: self.non_silent_blocks.load(Ordering::Relaxed),
        }
    }
}

/// Health of a monitored signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalStatus {
    /// Waiting for the stream to deliver its first audio.
    Starting,
    /// Audio is arriving and is not pure digital silence.
    Ok,
    /// Audio is arriving but every sample is exactly zero. The device is
    /// probably muted in hardware or the OS is feeding silence.
    Silent,
    /// No audio is arriving at all (stream stalled, device disconnected).
    Lost,
}

impl SignalStatus {
    pub fn label(self) -> &'static str {
        match self {
            SignalStatus::Starting => "starting…",
            SignalStatus::Ok => "signal OK",
            SignalStatus::Silent => "digital silence",
            SignalStatus::Lost => "signal lost",
        }
    }
}

/// Meter ballistics for display: fast attack, smooth release, peak hold, and
/// signal-health tracking. `update` is called once per UI frame.
#[derive(Clone, Debug)]
pub struct MeterState {
    /// Displayed level, dBFS (decays smoothly).
    pub level_db: f32,
    /// Displayed RMS, dBFS.
    pub rms_db: f32,
    /// Held peak, dBFS.
    pub peak_hold_db: f32,
    /// Highest peak seen since reset, dBFS.
    pub max_db: f32,
    /// True if the signal reached full scale recently.
    pub clipped: bool,
    pub status: SignalStatus,
    peak_hold_at: Instant,
    clipped_at: Instant,
    last_blocks: u64,
    last_blocks_change: Instant,
    last_non_silent: u64,
    last_non_silent_change: Instant,
    started: Instant,
}

/// dB per second the displayed level falls by.
const RELEASE_DB_PER_S: f32 = 24.0;
const PEAK_HOLD: Duration = Duration::from_millis(1500);
const CLIP_HOLD: Duration = Duration::from_secs(2);
const CLIP_THRESHOLD: f32 = 0.999;
/// How long without any audio blocks before we call the signal lost.
pub const LOST_AFTER: Duration = Duration::from_millis(1500);
/// How long of pure digital silence before we flag it.
pub const SILENT_AFTER: Duration = Duration::from_secs(3);

impl MeterState {
    pub fn new(now: Instant) -> Self {
        MeterState {
            level_db: FLOOR_DB,
            rms_db: FLOOR_DB,
            peak_hold_db: FLOOR_DB,
            max_db: FLOOR_DB,
            clipped: false,
            status: SignalStatus::Starting,
            peak_hold_at: now,
            clipped_at: now,
            last_blocks: 0,
            last_blocks_change: now,
            last_non_silent: 0,
            last_non_silent_change: now,
            started: now,
        }
    }

    pub fn update(&mut self, reading: LevelReading, dt: Duration, now: Instant) {
        let peak_db = to_db(reading.peak);
        let rms_db = to_db(reading.rms);
        let fall = RELEASE_DB_PER_S * dt.as_secs_f32();

        self.level_db = peak_db.max(self.level_db - fall).max(FLOOR_DB);
        self.rms_db = rms_db.max(self.rms_db - fall).max(FLOOR_DB);

        if peak_db >= self.peak_hold_db || now.duration_since(self.peak_hold_at) > PEAK_HOLD {
            self.peak_hold_db = peak_db.max(self.level_db);
            self.peak_hold_at = now;
        }
        self.max_db = self.max_db.max(peak_db);

        if reading.peak >= CLIP_THRESHOLD {
            self.clipped = true;
            self.clipped_at = now;
        } else if now.duration_since(self.clipped_at) > CLIP_HOLD {
            self.clipped = false;
        }

        if reading.blocks != self.last_blocks {
            self.last_blocks = reading.blocks;
            self.last_blocks_change = now;
        }
        if reading.non_silent_blocks != self.last_non_silent {
            self.last_non_silent = reading.non_silent_blocks;
            self.last_non_silent_change = now;
        }

        self.status = if reading.blocks == 0 {
            if now.duration_since(self.started) > LOST_AFTER * 2 {
                SignalStatus::Lost
            } else {
                SignalStatus::Starting
            }
        } else if now.duration_since(self.last_blocks_change) > LOST_AFTER {
            SignalStatus::Lost
        } else if now.duration_since(self.last_non_silent_change) > SILENT_AFTER {
            SignalStatus::Silent
        } else {
            SignalStatus::Ok
        };
    }

    pub fn reset_max(&mut self) {
        self.max_db = FLOOR_DB;
    }
}

/// Convert interleaved samples of any cpal sample type to f32 in place into
/// `out`, reusing its allocation.
pub fn to_f32<T: Copy>(input: &[T], out: &mut Vec<f32>, conv: impl Fn(T) -> f32) {
    out.clear();
    out.extend(input.iter().map(|&s| conv(s)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_conversions() {
        assert!((to_db(1.0) - 0.0).abs() < 1e-6);
        assert!((to_db(0.5) - -6.0206).abs() < 1e-3);
        assert_eq!(to_db(0.0), FLOOR_DB);
        assert_eq!(to_db(f32::NAN), FLOOR_DB);
        assert_eq!(to_db(1e-12), FLOOR_DB);
        assert!((from_db(-6.0206) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn peak_and_rms() {
        assert_eq!(peak(&[]), 0.0);
        assert_eq!(peak(&[0.1, -0.7, 0.3, f32::NAN]), 0.7);
        assert_eq!(rms(&[]), 0.0);
        // A full-scale square wave has RMS 1.
        assert!((rms(&[1.0, -1.0, 1.0, -1.0]) - 1.0).abs() < 1e-6);
        // A sine wave has RMS 1/sqrt(2).
        let sine: Vec<f32> = (0..48_000)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / 48_000.0).sin())
            .collect();
        assert!((rms(&sine) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
    }

    #[test]
    fn meter_position_maps_scale() {
        assert_eq!(meter_position(0.0, -60.0), 1.0);
        assert_eq!(meter_position(-60.0, -60.0), 0.0);
        assert_eq!(meter_position(-30.0, -60.0), 0.5);
        assert_eq!(meter_position(-90.0, -60.0), 0.0);
        assert_eq!(meter_position(6.0, -60.0), 1.0);
    }

    #[test]
    fn level_cell_keeps_max_between_reads() {
        let cell = LevelCell::default();
        cell.push(&[0.2, -0.1]);
        cell.push(&[0.6, 0.0]);
        cell.push(&[0.3]);
        let r = cell.take();
        assert_eq!(r.peak, 0.6);
        assert_eq!(r.blocks, 3);
        assert_eq!(r.non_silent_blocks, 3);
        let r2 = cell.take();
        assert_eq!(r2.peak, 0.0, "take() resets the maxima");
        assert_eq!(r2.blocks, 3, "block counters are cumulative");
        cell.push(&[0.0, 0.0]);
        let r3 = cell.take();
        assert_eq!(r3.blocks, 4);
        assert_eq!(r3.non_silent_blocks, 3);
    }

    fn reading(peak: f32, blocks: u64, non_silent: u64) -> LevelReading {
        LevelReading { peak, rms: peak / 2.0, blocks, non_silent_blocks: non_silent }
    }

    #[test]
    fn meter_attack_release_and_hold() {
        let t0 = Instant::now();
        let mut m = MeterState::new(t0);
        let frame = Duration::from_millis(100);
        m.update(reading(1.0, 1, 1), frame, t0 + frame);
        assert!(m.level_db.abs() < 1e-3, "instant attack");
        assert!(m.clipped);
        // Silence afterwards: level falls at RELEASE_DB_PER_S, peak is held.
        m.update(reading(0.0, 2, 1), frame, t0 + frame * 2);
        assert!((m.level_db - -2.4).abs() < 1e-3, "{}", m.level_db);
        assert!(m.peak_hold_db.abs() < 1e-3, "peak held");
        assert!(m.max_db.abs() < 1e-3);
        // After the hold time the peak indicator drops back to the current level.
        m.update(reading(0.0, 3, 1), frame, t0 + Duration::from_secs(3));
        assert!(m.peak_hold_db < -1.0);
        assert!(!m.clipped, "clip indicator releases");
        m.reset_max();
        assert_eq!(m.max_db, FLOOR_DB);
    }

    #[test]
    fn signal_status_transitions() {
        let t0 = Instant::now();
        let mut m = MeterState::new(t0);
        let dt = Duration::from_millis(50);
        m.update(reading(0.0, 0, 0), dt, t0 + dt);
        assert_eq!(m.status, SignalStatus::Starting);
        m.update(reading(0.0, 0, 0), dt, t0 + Duration::from_secs(4));
        assert_eq!(m.status, SignalStatus::Lost, "never started");

        let mut m = MeterState::new(t0);
        m.update(reading(0.1, 5, 5), dt, t0 + dt);
        assert_eq!(m.status, SignalStatus::Ok);
        // Blocks keep coming but are all zero -> digital silence.
        m.update(reading(0.0, 50, 5), dt, t0 + Duration::from_secs(4));
        assert_eq!(m.status, SignalStatus::Silent);
        // Blocks stop arriving -> lost.
        m.update(reading(0.0, 50, 5), dt, t0 + Duration::from_secs(6));
        assert_eq!(m.status, SignalStatus::Lost);
        // Audio comes back.
        m.update(reading(0.2, 51, 6), dt, t0 + Duration::from_secs(7));
        assert_eq!(m.status, SignalStatus::Ok);
    }
}
