//! Level math and meter ballistics. Pure functions + small state machines with
//! no audio I/O, so it is all unit-testable.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Anything quieter than this is shown as "silence" (-inf).
pub const FLOOR_DB: f32 = -96.0;

/// Typical speaking level (RMS, dBFS) below which colleagues are likely to hear
/// you quietly. Clear speech into a mic is usually around -40 to -15 dBFS.
pub const WEAK_SPEECH_DB: f32 = -50.0;

/// Threshold above which a moment counts as sound (speech): 12 dB above the
/// device's own noise floor. A fixed threshold fails for quiet analog headsets,
/// whose whole voice can sit below -55 dBFS. Kept within -80..=-45 dBFS.
pub fn speech_gate(noise_floor_db: Option<f32>) -> f32 {
    match noise_floor_db {
        Some(n) => (n + 12.0).clamp(-80.0, -45.0),
        None => -55.0,
    }
}

/// Lowest dB shown on a device's meters: the user's setting, extended down to
/// 10 dB below the device's noise floor (in 6 dB steps) so a quiet mic still
/// moves the meter visibly.
pub fn display_floor(setting_db: f32, noise_floor_db: Option<f32>) -> f32 {
    match noise_floor_db {
        Some(n) => setting_db.min(((n - 10.0) / 6.0).floor() * 6.0).max(FLOOR_DB),
        None => setting_db,
    }
}

/// Convert a linear amplitude (0..=1, 1 = full scale) to dBFS.
pub fn to_db(linear: f32) -> f32 {
    if linear <= 0.0 || !linear.is_finite() { FLOOR_DB } else { (20.0 * linear.log10()).max(FLOOR_DB) }
}

/// Convert dBFS to linear amplitude.
pub fn from_db(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Peak absolute sample value of a block.
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| if s.is_finite() { m.max(s.abs()) } else { m })
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
    /// Per-consumer maxima, so the UI and a recorder can each drain their own
    /// copy without stealing peaks from each other.
    peak_bits: [AtomicU32; CONSUMERS],
    rms_bits: [AtomicU32; CONSUMERS],
    /// Sum of squared samples (f64 bits) and sample count since the last
    /// take, for an exact RMS over the period regardless of block timing.
    energy_bits: [AtomicU64; CONSUMERS],
    samples: [AtomicU64; CONSUMERS],
    /// K-weighted energy (f64 bits) and its sample count, for loudness.
    k_energy_bits: [AtomicU64; CONSUMERS],
    k_samples: [AtomicU64; CONSUMERS],
    /// Number of audio blocks received in total (detects stalled streams).
    blocks: AtomicU64,
    /// Number of blocks that contained at least one non-zero sample.
    non_silent_blocks: AtomicU64,
}

const CONSUMERS: usize = 2;

/// Who is reading a [`LevelCell`]. Each consumer has independent maxima.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consumer {
    Ui = 0,
    Recorder = 1,
}

impl Default for LevelCell {
    fn default() -> Self {
        LevelCell {
            peak_bits: std::array::from_fn(|_| AtomicU32::new(0)),
            rms_bits: std::array::from_fn(|_| AtomicU32::new(0)),
            energy_bits: std::array::from_fn(|_| AtomicU64::new(0)),
            samples: std::array::from_fn(|_| AtomicU64::new(0)),
            k_energy_bits: std::array::from_fn(|_| AtomicU64::new(0)),
            k_samples: std::array::from_fn(|_| AtomicU64::new(0)),
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
    /// Sum of squared samples since the previous take (this consumer).
    pub energy: f64,
    /// Number of samples since the previous take (this consumer).
    pub samples: u64,
    /// K-weighted energy and sample count since the previous take (loudness).
    pub k_energy: f64,
    pub k_samples: u64,
}

impl LevelReading {
    /// Exact RMS over all samples since the previous take, if any arrived.
    pub fn mean_rms(&self) -> Option<f32> {
        (self.samples > 0).then(|| (self.energy / self.samples as f64).sqrt() as f32)
    }
}

impl LevelCell {
    /// Called from the audio thread. Must not block or allocate.
    pub fn push(&self, samples: &[f32]) {
        let p = peak(samples);
        let energy: f64 = samples.iter().filter(|s| s.is_finite()).map(|&s| (s as f64) * (s as f64)).sum();
        let r = if samples.is_empty() { 0.0 } else { (energy / samples.len() as f64).sqrt() as f32 };
        self.add_energy(energy, samples.len() as u64);
        self.push_peak_rms(p, r);
    }

    /// For sources that only report levels (native OS peak meters): counts as
    /// one sample with power `rms²`.
    pub fn push_levels(&self, peak: f32, rms: f32) {
        self.add_energy((rms as f64) * (rms as f64), 1);
        self.push_peak_rms(peak, rms);
    }

    /// Audio thread: add K-weighted energy for loudness measurement.
    pub fn add_k_energy(&self, energy: f64, n: u64) {
        for i in 0..CONSUMERS {
            add_f64(&self.k_energy_bits[i], energy);
            self.k_samples[i].fetch_add(n, Ordering::Relaxed);
        }
    }

    fn add_energy(&self, energy: f64, n: u64) {
        for i in 0..CONSUMERS {
            add_f64(&self.energy_bits[i], energy);
            self.samples[i].fetch_add(n, Ordering::Relaxed);
        }
    }

    fn push_peak_rms(&self, peak: f32, rms: f32) {
        let (p, r) = (peak.max(0.0).to_bits(), rms.max(0.0).to_bits());
        for i in 0..CONSUMERS {
            self.peak_bits[i].fetch_max(p, Ordering::Relaxed);
            self.rms_bits[i].fetch_max(r, Ordering::Relaxed);
        }
        self.blocks.fetch_add(1, Ordering::Relaxed);
        if peak > 0.0 {
            self.non_silent_blocks.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Read and reset the UI's peak/RMS maxima. Block counters are cumulative.
    pub fn take(&self) -> LevelReading {
        self.take_for(Consumer::Ui)
    }

    /// Read and reset one consumer's maxima.
    pub fn take_for(&self, consumer: Consumer) -> LevelReading {
        let i = consumer as usize;
        LevelReading {
            peak: f32::from_bits(self.peak_bits[i].swap(0, Ordering::Relaxed)),
            rms: f32::from_bits(self.rms_bits[i].swap(0, Ordering::Relaxed)),
            blocks: self.blocks.load(Ordering::Relaxed),
            non_silent_blocks: self.non_silent_blocks.load(Ordering::Relaxed),
            energy: f64::from_bits(self.energy_bits[i].swap(0, Ordering::Relaxed)),
            samples: self.samples[i].swap(0, Ordering::Relaxed),
            k_energy: f64::from_bits(self.k_energy_bits[i].swap(0, Ordering::Relaxed)),
            k_samples: self.k_samples[i].swap(0, Ordering::Relaxed),
        }
    }
}

/// ITU-R BS.1770 / EBU R128 K-weighting: a high-shelf ("head") filter followed
/// by a high-pass ("RLB") filter. Coefficients are derived for any sample rate
/// (same formulas as libebur128).
#[derive(Clone, Debug)]
pub struct KWeighting {
    shelf: Biquad,
    highpass: Biquad,
}

#[derive(Clone, Debug, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

impl KWeighting {
    pub fn new(sample_rate: u32) -> Self {
        let fs = sample_rate.max(1) as f64;
        // Stage 1: high shelf.
        let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
        let k = (std::f64::consts::PI * f0 / fs).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.4996667741545416);
        let a0 = 1.0 + k / q + k * k;
        let shelf = Biquad {
            b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            z: [0.0; 2],
        };
        // Stage 2: high-pass (RLB).
        let (f0, q) = (38.13547087602444, 0.5003270373238773);
        let k = (std::f64::consts::PI * f0 / fs).tan();
        let a0 = 1.0 + k / q + k * k;
        let highpass =
            Biquad { b: [1.0, -2.0, 1.0], a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0], z: [0.0; 2] };
        KWeighting { shelf, highpass }
    }

    /// Sum of squared K-weighted samples for a block (mono).
    pub fn block_energy(&mut self, mono: impl Iterator<Item = f32>) -> (f64, u64) {
        let mut e = 0.0;
        let mut n = 0;
        for x in mono {
            let y = self.highpass.process(self.shelf.process(x as f64));
            e += y * y;
            n += 1;
        }
        (e, n)
    }
}

/// Loudness in LUFS from a mean square of K-weighted samples (one channel).
pub fn lufs(mean_square: f64) -> f32 {
    if mean_square <= 0.0 { FLOOR_DB } else { (-0.691 + 10.0 * mean_square.log10()) as f32 }
}

/// Sliding-window loudness: momentary (400 ms) and short-term (3 s) LUFS.
#[derive(Clone, Debug, Default)]
pub struct Loudness {
    /// (time, K-weighted energy, samples) per UI frame.
    frames: std::collections::VecDeque<(Instant, f64, u64)>,
}

impl Loudness {
    pub fn push(&mut self, now: Instant, energy: f64, samples: u64) {
        if samples > 0 {
            self.frames.push_back((now, energy, samples));
        }
        while self.frames.front().is_some_and(|f| now.duration_since(f.0) > Duration::from_secs(3)) {
            self.frames.pop_front();
        }
    }

    fn window(&self, now: Instant, span: Duration) -> Option<f32> {
        let (e, n) = self
            .frames
            .iter()
            .filter(|f| now.duration_since(f.0) <= span)
            .fold((0.0, 0u64), |(e, n), f| (e + f.1, n + f.2));
        (n > 0).then(|| lufs(e / n as f64))
    }

    pub fn momentary(&self, now: Instant) -> Option<f32> {
        self.window(now, Duration::from_millis(400))
    }

    pub fn short_term(&self, now: Instant) -> Option<f32> {
        self.window(now, Duration::from_secs(3))
    }
}

/// The most recent mono samples of a stream, readable without locks (for the
/// spectrum analyzer). The audio thread overwrites a ring of atomics.
#[derive(Debug)]
pub struct SampleWindow {
    data: Vec<AtomicU32>,
    pos: std::sync::atomic::AtomicUsize,
    pub sample_rate: u32,
}

impl SampleWindow {
    pub fn new(len: usize, sample_rate: u32) -> Self {
        SampleWindow {
            data: (0..len.max(1)).map(|_| AtomicU32::new(0)).collect(),
            pos: std::sync::atomic::AtomicUsize::new(0),
            sample_rate,
        }
    }

    /// Audio thread: append interleaved samples (downmixed to mono).
    pub fn push_interleaved(&self, interleaved: &[f32], channels: usize) {
        let ch = channels.max(1);
        let mut pos = self.pos.load(Ordering::Relaxed);
        for frame in interleaved.chunks_exact(ch) {
            let mono = frame.iter().sum::<f32>() / ch as f32;
            self.data[pos % self.data.len()].store(mono.to_bits(), Ordering::Relaxed);
            pos = pos.wrapping_add(1);
        }
        self.pos.store(pos, Ordering::Release);
    }

    /// The latest `n` samples, oldest first.
    pub fn latest(&self, n: usize) -> Vec<f32> {
        let len = self.data.len();
        let n = n.min(len);
        let end = self.pos.load(Ordering::Acquire);
        (0..n).map(|i| f32::from_bits(self.data[(end.wrapping_sub(n) + i) % len].load(Ordering::Relaxed))).collect()
    }
}

/// Lock-free f64 add (compare-exchange loop; uncontended in practice).
fn add_f64(cell: &AtomicU64, v: f64) {
    let mut cur = cell.load(Ordering::Relaxed);
    loop {
        let new = (f64::from_bits(cur) + v).to_bits();
        match cell.compare_exchange_weak(cur, new, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => cur = actual,
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
    /// Number of separate clipping events since start / reset.
    pub clip_count: u32,
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
            clip_count: 0,
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
            if !self.clipped {
                self.clip_count += 1;
            }
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
            if now.duration_since(self.started) > LOST_AFTER * 2 { SignalStatus::Lost } else { SignalStatus::Starting }
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
        self.clip_count = 0;
    }
}

/// Rolling history of levels for the "is my gain drifting?" graph: fixed-length
/// buckets (e.g. 200 ms each) holding the loudest peak and RMS in that bucket.
#[derive(Clone, Debug)]
pub struct LevelHistory {
    bucket: Duration,
    capacity: usize,
    /// (peak dB, rms dB) per bucket, oldest first.
    buckets: std::collections::VecDeque<(f32, f32)>,
    current: Option<(f32, f32)>,
    current_started: Option<Instant>,
}

impl LevelHistory {
    pub fn new(bucket: Duration, capacity: usize) -> Self {
        LevelHistory {
            bucket,
            capacity,
            buckets: std::collections::VecDeque::with_capacity(capacity),
            current: None,
            current_started: None,
        }
    }

    /// 30 seconds at 5 buckets per second.
    pub fn thirty_seconds() -> Self {
        Self::new(Duration::from_millis(200), 150)
    }

    pub fn push(&mut self, peak_db: f32, rms_db: f32, now: Instant) {
        let started = *self.current_started.get_or_insert(now);
        if now.duration_since(started) >= self.bucket {
            if let Some(c) = self.current.take() {
                self.buckets.push_back(c);
                while self.buckets.len() > self.capacity {
                    self.buckets.pop_front();
                }
            }
            self.current_started = Some(now);
        }
        let c = self.current.get_or_insert((FLOOR_DB, FLOOR_DB));
        c.0 = c.0.max(peak_db);
        c.1 = c.1.max(rms_db);
    }

    /// Completed buckets, oldest first.
    pub fn buckets(&self) -> impl ExactSizeIterator<Item = &(f32, f32)> {
        self.buckets.iter()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Background noise level: the 10th percentile of the RMS buckets
    /// (the quietest moments, between words). `None` until enough data.
    pub fn noise_floor(&self) -> Option<f32> {
        let mut v: Vec<f32> = self.buckets.iter().map(|b| b.1).filter(|&r| r > FLOOR_DB).collect();
        if v.len() < 10 {
            return None;
        }
        v.sort_by(|a, b| a.total_cmp(b));
        Some(v[v.len() / 10])
    }

    /// Signal-to-noise ratio estimate: typical speaking level minus noise floor.
    pub fn snr(&self, gate_db: f32) -> Option<f32> {
        Some(self.typical_level(gate_db)? - self.noise_floor()?)
    }

    /// Average RMS (dB) over the buckets that contain sound above `gate_db`,
    /// i.e. a "typical speaking level" that ignores pauses. `None` if
    /// there's too little sound to judge.
    pub fn typical_level(&self, gate_db: f32) -> Option<f32> {
        let active: Vec<f32> = self.buckets.iter().map(|b| b.1).filter(|&r| r > gate_db).collect();
        if active.len() < 5 { None } else { Some(active.iter().sum::<f32>() / active.len() as f32) }
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
    fn gates_and_floors_follow_the_noise() {
        // Quiet analog headset: noise at -80, so speech from -68 counts.
        assert_eq!(speech_gate(Some(-80.0)), -68.0);
        // Very clean USB mic: clamp at -80.
        assert_eq!(speech_gate(Some(-100.0)), -80.0);
        // Noisy room: clamp at -45 so not everything counts as speech.
        assert_eq!(speech_gate(Some(-40.0)), -45.0);
        assert_eq!(speech_gate(None), -55.0);
        assert_eq!(display_floor(-60.0, Some(-80.0)), -90.0);
        assert_eq!(display_floor(-60.0, Some(-50.0)), -60.0, "never narrower than the setting");
        assert_eq!(display_floor(-60.0, Some(-120.0)), FLOOR_DB);
        assert_eq!(display_floor(-60.0, None), -60.0);
    }

    #[test]
    fn peak_and_rms() {
        assert_eq!(peak(&[]), 0.0);
        assert_eq!(peak(&[0.1, -0.7, 0.3, f32::NAN]), 0.7);
        assert_eq!(rms(&[]), 0.0);
        // A full-scale square wave has RMS 1.
        assert!((rms(&[1.0, -1.0, 1.0, -1.0]) - 1.0).abs() < 1e-6);
        // A sine wave has RMS 1/sqrt(2).
        let sine: Vec<f32> =
            (0..48_000).map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / 48_000.0).sin()).collect();
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
    fn k_weighting_matches_the_ebu_reference() {
        // EBU Tech 3341: a 0 dBFS 1 kHz sine on one channel reads -3.01 LUFS.
        for rate in [44_100u32, 48_000] {
            let mut k = KWeighting::new(rate);
            let sine = (0..rate * 2).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate as f32).sin());
            let (e, n) = k.block_energy(sine);
            let l = lufs(e / n as f64);
            assert!((l - -3.01).abs() < 0.05, "{rate} Hz: {l}");
        }
        // -20 dB quieter reads 20 LU lower; 100 Hz is attenuated slightly, 20 Hz strongly.
        let rate = 48_000;
        let level = |f: f32, amp: f32| {
            let mut k = KWeighting::new(rate);
            let s = (0..rate * 2).map(|i| amp * (2.0 * std::f32::consts::PI * f * i as f32 / rate as f32).sin());
            let (e, n) = k.block_energy(s);
            lufs(e / n as f64)
        };
        assert!((level(1000.0, 0.1) - -23.01).abs() < 0.05);
        assert!(level(100.0, 1.0) < -3.0 && level(100.0, 1.0) > -5.0);
        assert!(level(20.0, 1.0) < -10.0);
    }

    #[test]
    fn loudness_windows() {
        let t0 = Instant::now();
        let mut l = Loudness::default();
        // About 3 s of mean square 0.5 (≈ -3.7 LUFS), then the last ~430 ms silent.
        for i in 0..90u64 {
            let quiet = i >= 76; // the last 13 frames (429 ms) are silent
            l.push(t0 + Duration::from_millis(i * 33), if quiet { 0.0 } else { 0.5 * 1600.0 }, 1600);
        }
        let now = t0 + Duration::from_millis(89 * 33);
        assert_eq!(l.momentary(now), Some(FLOOR_DB));
        let s = l.short_term(now).unwrap();
        assert!(s > -5.0 && s < -3.5, "{s}");
    }

    #[test]
    fn sample_window_keeps_latest_in_order() {
        let w = SampleWindow::new(4, 48_000);
        w.push_interleaved(&[1.0, 1.0, 2.0, 2.0, 3.0, 3.0], 2);
        assert_eq!(w.latest(2), vec![2.0, 3.0]);
        w.push_interleaved(&[4.0, 5.0, 6.0], 1);
        assert_eq!(w.latest(10), vec![3.0, 4.0, 5.0, 6.0], "wraps, capped at capacity");
    }

    #[test]
    fn level_cell_consumers_are_independent() {
        let cell = LevelCell::default();
        cell.push(&[0.5]);
        assert_eq!(cell.take().peak, 0.5);
        assert_eq!(cell.take_for(Consumer::Recorder).peak, 0.5, "UI read must not steal the recorder's peak");
        assert_eq!(cell.take_for(Consumer::Recorder).peak, 0.0);
    }

    #[test]
    fn mean_rms_is_exact_over_uneven_blocks() {
        let cell = LevelCell::default();
        // One loud block of 3 samples and one quiet block of 1 sample.
        cell.push(&[1.0, -1.0, 1.0]);
        cell.push(&[0.0]);
        let r = cell.take_for(Consumer::Recorder);
        assert_eq!(r.samples, 4);
        assert!((r.mean_rms().unwrap() - (3.0f32 / 4.0).sqrt()).abs() < 1e-6);
        assert_eq!(cell.take_for(Consumer::Recorder).mean_rms(), None, "nothing new");
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
        LevelReading { peak, rms: peak / 2.0, blocks, non_silent_blocks: non_silent, ..Default::default() }
    }

    #[test]
    fn meter_attack_release_and_hold() {
        let t0 = Instant::now();
        let mut m = MeterState::new(t0);
        let frame = Duration::from_millis(100);
        m.update(reading(1.0, 1, 1), frame, t0 + frame);
        assert!(m.level_db.abs() < 1e-3, "instant attack");
        assert!(m.clipped);
        assert_eq!(m.clip_count, 1);
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
    fn history_buckets_and_typical_level() {
        let t0 = Instant::now();
        let mut h = LevelHistory::new(Duration::from_millis(100), 4);
        // 10 frames per bucket, 6 buckets' worth of input -> capacity caps at 4.
        for i in 0..60u64 {
            let loud = (i / 10) % 2 == 0;
            let db = if loud { -20.0 } else { -70.0 };
            h.push(db + 1.0, db, t0 + Duration::from_millis(i * 10));
        }
        let b: Vec<_> = h.buckets().copied().collect();
        assert_eq!(b.len(), 4);
        assert_eq!(b[0], (-69.0, -70.0), "oldest kept bucket is a quiet one");
        assert_eq!(b[1], (-19.0, -20.0));
        assert_eq!(h.typical_level(-50.0), None, "too few active buckets");

        let mut h = LevelHistory::new(Duration::from_millis(100), 100);
        for i in 0..200u64 {
            let db = if (i / 10) % 2 == 0 { -24.0 } else { -80.0 };
            h.push(db, db, t0 + Duration::from_millis(i * 10));
        }
        assert_eq!(h.typical_level(-50.0), Some(-24.0), "pauses are ignored");
        assert_eq!(h.noise_floor(), Some(-80.0));
        assert_eq!(h.snr(-50.0), Some(56.0));
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
