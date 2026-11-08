//! Spectrum analysis for the live view: Hann-windowed FFT, grouped into
//! logarithmically spaced bands (20 Hz – 20 kHz) like a hardware analyzer.

use rustfft::{FftPlanner, num_complex::Complex};

pub const FFT_SIZE: usize = 4096;
pub const BANDS: usize = 48;
pub const F_MIN: f32 = 20.0;
pub const F_MAX: f32 = 20_000.0;

/// Band centre frequencies (log spaced).
pub fn band_centres() -> Vec<f32> {
    (0..BANDS).map(|i| F_MIN * (F_MAX / F_MIN).powf((i as f32 + 0.5) / BANDS as f32)).collect()
}

pub struct Analyzer {
    planner: FftPlanner<f32>,
    window: Vec<f32>,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE as f32 - 1.0)).cos())
            .collect();
        Analyzer { planner: FftPlanner::new(), window }
    }

    /// Per-band level in dBFS (a full-scale sine reads about 0 dB in its band).
    pub fn bands(&mut self, samples: &[f32], sample_rate: u32) -> Vec<f32> {
        let mut buf: Vec<Complex<f32>> =
            (0..FFT_SIZE).map(|i| Complex::new(samples.get(i).copied().unwrap_or(0.0) * self.window[i], 0.0)).collect();
        self.planner.plan_fft_forward(FFT_SIZE).process(&mut buf);
        // Amplitude normalisation for a Hann window: 2 / (N * 0.5).
        let norm = 4.0 / FFT_SIZE as f32;
        let bin_hz = sample_rate as f32 / FFT_SIZE as f32;
        let nyquist = sample_rate as f32 / 2.0;
        (0..BANDS)
            .map(|b| {
                let lo = F_MIN * (F_MAX / F_MIN).powf(b as f32 / BANDS as f32);
                let hi = F_MIN * (F_MAX / F_MIN).powf((b + 1) as f32 / BANDS as f32);
                if lo >= nyquist {
                    return crate::levels::FLOOR_DB;
                }
                let (i0, i1) = (
                    (lo / bin_hz).floor() as usize,
                    ((hi / bin_hz).ceil() as usize).max((lo / bin_hz).floor() as usize + 1),
                );
                let peak = buf[i0.max(1)..i1.min(FFT_SIZE / 2)].iter().map(|c| c.norm() * norm).fold(0.0f32, f32::max);
                crate::levels::to_db(peak)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_lands_in_the_right_band_at_the_right_level() {
        let rate = 48_000;
        let f = 1000.0;
        let samples: Vec<f32> =
            (0..FFT_SIZE).map(|i| 0.5 * (2.0 * std::f32::consts::PI * f * i as f32 / rate as f32).sin()).collect();
        let bands = Analyzer::new().bands(&samples, rate);
        let centres = band_centres();
        let loudest = bands.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        let expected = centres.iter().enumerate().min_by(|a, b| (a.1 - f).abs().total_cmp(&(b.1 - f).abs())).unwrap().0;
        assert!((loudest as i32 - expected as i32).abs() <= 1, "band {loudest} vs {expected}");
        // 0.5 amplitude = -6 dBFS (within the window's scalloping loss).
        assert!((bands[loudest] - -6.0).abs() < 2.0, "{}", bands[loudest]);
        // Far-away bands are much quieter.
        assert!(bands[2] < -60.0, "{}", bands[2]);
    }
}
