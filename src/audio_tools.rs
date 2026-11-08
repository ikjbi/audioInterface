//! Hands-on test tools that play sound:
//! * **Speaker test**: a soft 440 Hz tone on the left, right or both channels.
//! * **Mic test**: record a few seconds, then play them back.
//! * **Monitor** ("hear yourself"): route a mic to an output live.
//!
//! Each tool runs on its own thread and is controlled with a [`ToolHandle`].
//! Device IDs are the same strings the backends use (cpal shares them).

use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Channel {
    Left,
    Right,
    Both,
}

/// Progress of a running tool, for the UI.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolState {
    Starting,
    Playing { label: String },
    Recording { seconds_left: f32 },
    Done,
    Failed(String),
}

#[derive(Clone)]
pub struct ToolHandle {
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<ToolState>>,
    /// Monitor volume (f32 bits, 0..=1).
    volume: Arc<AtomicU32>,
}

impl ToolHandle {
    fn new() -> Self {
        ToolHandle {
            stop: Arc::new(AtomicBool::new(false)),
            state: Arc::new(Mutex::new(ToolState::Starting)),
            volume: Arc::new(AtomicU32::new(0.7f32.to_bits())),
        }
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
    pub fn state(&self) -> ToolState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn is_finished(&self) -> bool {
        matches!(self.state(), ToolState::Done | ToolState::Failed(_))
    }
    pub fn set_volume(&self, v: f32) {
        self.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }
    fn set(&self, s: ToolState) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

fn spawn(name: &str, f: impl FnOnce(&ToolHandle) -> Result<()> + Send + 'static) -> ToolHandle {
    let h = ToolHandle::new();
    let h2 = h.clone();
    let _ = std::thread::Builder::new().name(name.into()).spawn(move || match f(&h2) {
        Ok(()) => h2.set(ToolState::Done),
        Err(e) => h2.set(ToolState::Failed(format!("{e:#}"))),
    });
    h
}

fn find(host: &cpal::Host, id: &str) -> Result<cpal::Device> {
    host.device_by_id(&cpal::DeviceId::new(host.id(), id)).ok_or_else(|| anyhow!("device not found: {id}"))
}

/// Linear-interpolating resampler for mono f32.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = ((input.len() as f64) / ratio).floor() as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * ratio;
            let j = pos.floor() as usize;
            let frac = (pos - j as f64) as f32;
            let a = input[j.min(input.len() - 1)];
            let b = input[(j + 1).min(input.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

/// Test-tone samples (mono): 440 Hz sine at -12 dBFS with 20 ms fades.
pub fn tone(sample_rate: u32, seconds: f32) -> Vec<f32> {
    let n = (sample_rate as f32 * seconds) as usize;
    let fade = (sample_rate as f32 * 0.02) as usize;
    (0..n)
        .map(|i| {
            let env = (i.min(n - 1 - i) as f32 / fade as f32).min(1.0);
            0.25 * env * (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / sample_rate as f32).sin()
        })
        .collect()
}

/// Something that yields mono samples for an output stream.
type Source = Box<dyn FnMut() -> Option<f32> + Send>;

/// Play `source` on `device` until it returns None or the tool is stopped.
/// Mono samples go to the selected channel(s).
fn play(
    device: &cpal::Device,
    handle: &ToolHandle,
    channel: Channel,
    make_source: impl FnOnce(u32) -> Source,
) -> Result<()> {
    let supported = device.default_output_config().context("query output format")?;
    let config = supported.config();
    let rate = config.sample_rate;
    let source = Arc::new(Mutex::new(make_source(rate)));
    let finished = Arc::new(AtomicBool::new(false));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_out::<f32>(device, config, channel, source, finished.clone()),
        SampleFormat::I16 => build_out::<i16>(device, config, channel, source, finished.clone()),
        SampleFormat::I32 => build_out::<i32>(device, config, channel, source, finished.clone()),
        SampleFormat::U16 => build_out::<u16>(device, config, channel, source, finished.clone()),
        SampleFormat::F64 => build_out::<f64>(device, config, channel, source, finished.clone()),
        other => Err(anyhow!("unsupported output format {other:?}")),
    }?;
    stream.play()?;
    while !finished.load(Ordering::SeqCst) && !handle.stopped() {
        std::thread::sleep(Duration::from_millis(20));
    }
    // Let the device drain its buffer.
    std::thread::sleep(Duration::from_millis(150));
    Ok(())
}

fn build_out<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channel: Channel,
    source: Arc<Mutex<Source>>,
    finished: Arc<AtomicBool>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let ch = config.channels.max(1) as usize;
    let stream = device.build_output_stream::<T, _, _>(
        config,
        move |out: &mut [T], _| {
            let mut src = source.lock().unwrap_or_else(|e| e.into_inner());
            for frame in out.chunks_mut(ch) {
                let s = match src() {
                    Some(s) => s,
                    None => {
                        finished.store(true, Ordering::SeqCst);
                        0.0
                    }
                };
                for (i, o) in frame.iter_mut().enumerate() {
                    let on = match channel {
                        Channel::Both => true,
                        Channel::Left => i == 0 || ch == 1,
                        Channel::Right => i == 1 || ch == 1,
                    };
                    *o = T::from_sample(if on { s } else { 0.0 });
                }
            }
        },
        |e| log::warn!("output stream error: {e}"),
        None,
    )?;
    Ok(stream)
}

/// Speaker test: 1.5 s tone on the chosen channel(s).
pub fn speaker_test(output_id: String, channel: Channel) -> ToolHandle {
    spawn("speaker-test", move |h| {
        let host = cpal::default_host();
        let dev = find(&host, &output_id)?;
        let label = match channel {
            Channel::Left => "left",
            Channel::Right => "right",
            Channel::Both => "both channels",
        };
        h.set(ToolState::Playing { label: format!("test tone ({label})") });
        play(&dev, h, channel, |rate| {
            let samples = tone(rate, 1.5);
            let mut i = 0;
            Box::new(move || {
                let s = samples.get(i).copied();
                i += 1;
                s
            })
        })
    })
}

/// Capture mono audio from an input until `until` returns true. Returns (samples, rate).
fn capture(device: &cpal::Device, handle: &ToolHandle, seconds: f32) -> Result<(Vec<f32>, u32)> {
    let supported = device.default_input_config().context("query input format")?;
    let config = supported.config();
    let rate = config.sample_rate;
    let ch = config.channels.max(1) as usize;
    let (mut tx, mut rx) = rtrb::RingBuffer::<f32>::new((rate as f32 * (seconds + 2.0)) as usize);
    let mut feed = move |data: &[f32]| {
        for frame in data.chunks_exact(ch) {
            let _ = tx.push(frame.iter().sum::<f32>() / ch as f32);
        }
    };
    let err = |e| log::warn!("capture error: {e}");
    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_input_stream::<f32, _, _>(config, move |d: &[f32], _| feed(d), err, None)?,
        SampleFormat::I16 => {
            let mut buf = Vec::new();
            device.build_input_stream::<i16, _, _>(
                config,
                move |d: &[i16], _| {
                    buf.clear();
                    buf.extend(d.iter().map(|s| *s as f32 / i16::MAX as f32));
                    feed(&buf)
                },
                err,
                None,
            )?
        }
        SampleFormat::I32 => {
            let mut buf = Vec::new();
            device.build_input_stream::<i32, _, _>(
                config,
                move |d: &[i32], _| {
                    buf.clear();
                    buf.extend(d.iter().map(|s| *s as f32 / i32::MAX as f32));
                    feed(&buf)
                },
                err,
                None,
            )?
        }
        other => bail!("unsupported input format {other:?}"),
    };
    stream.play()?;
    let start = Instant::now();
    let mut out = Vec::with_capacity((rate as f32 * seconds) as usize);
    while start.elapsed().as_secs_f32() < seconds && !handle.stopped() {
        h_set_recording(handle, seconds - start.elapsed().as_secs_f32());
        while let Ok(s) = rx.pop() {
            out.push(s);
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    drop(stream);
    while let Ok(s) = rx.pop() {
        out.push(s);
    }
    Ok((out, rate))
}

fn h_set_recording(h: &ToolHandle, left: f32) {
    h.set(ToolState::Recording { seconds_left: left.max(0.0) });
}

/// Mic test: record `seconds`, then play it back on `output_id`.
pub fn mic_test(input_id: String, output_id: String, seconds: f32) -> ToolHandle {
    spawn("mic-test", move |h| {
        let host = cpal::default_host();
        let input = find(&host, &input_id)?;
        let output = find(&host, &output_id)?;
        let (rec, in_rate) = capture(&input, h, seconds)?;
        if h.stopped() {
            return Ok(());
        }
        if rec.is_empty() {
            bail!("no audio was captured (check microphone permission)");
        }
        let peak = crate::levels::peak(&rec);
        h.set(ToolState::Playing { label: format!("playback (recorded peak {:.0} dBFS)", crate::levels::to_db(peak)) });
        play(&output, h, Channel::Both, move |out_rate| {
            let samples = resample(&rec, in_rate, out_rate);
            let mut i = 0;
            Box::new(move || {
                let s = samples.get(i).copied();
                i += 1;
                s
            })
        })
    })
}

/// Live monitoring: route `input_id` to `output_id` until stopped. Use headphones,
/// since speakers will feed back.
pub fn monitor(input_id: String, output_id: String) -> ToolHandle {
    spawn("monitor", move |h| {
        let host = cpal::default_host();
        let input = find(&host, &input_id)?;
        let output = find(&host, &output_id)?;
        let in_cfg = input.default_input_config()?;
        let out_cfg = output.default_output_config()?;
        let (in_rate, out_rate) = (in_cfg.config().sample_rate, out_cfg.config().sample_rate);
        let ch = in_cfg.config().channels.max(1) as usize;
        if in_cfg.sample_format() != SampleFormat::F32 {
            bail!("monitoring needs a float input format (device uses {:?})", in_cfg.sample_format());
        }
        // ~200 ms of buffering; resample on the output side.
        let (mut tx, mut rx) = rtrb::RingBuffer::<f32>::new((in_rate / 5) as usize);
        let in_stream = input.build_input_stream::<f32, _, _>(
            in_cfg.config(),
            move |d: &[f32], _| {
                for frame in d.chunks_exact(ch) {
                    let _ = tx.push(frame.iter().sum::<f32>() / ch as f32);
                }
            },
            |e| log::warn!("monitor input error: {e}"),
            None,
        )?;
        in_stream.play()?;
        h.set(ToolState::Playing { label: "monitoring (use headphones)".into() });
        let vol = h.volume.clone();
        let step = in_rate as f64 / out_rate as f64;
        play(&output, h, Channel::Both, move |_| {
            let (mut pos, mut a, mut b) = (0.0f64, 0.0f32, 0.0f32);
            Box::new(move || {
                // Keep latency low: drop backlog beyond ~60 ms.
                while rx.slots() > (in_rate / 16) as usize {
                    let _ = rx.pop();
                }
                pos += step;
                while pos >= 1.0 {
                    pos -= 1.0;
                    a = b;
                    b = rx.pop().unwrap_or(b * 0.98);
                }
                let s = a + (b - a) * pos as f32;
                Some(s * f32::from_bits(vol.load(Ordering::Relaxed)))
            })
        })?;
        drop(in_stream);
        Ok(())
    })
}

/// Block until a tool finishes (CLI use). Returns its error, if any.
pub fn wait(h: &ToolHandle, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        match h.state() {
            ToolState::Done => return Ok(()),
            ToolState::Failed(e) => bail!("{e}"),
            _ => std::thread::sleep(Duration::from_millis(30)),
        }
    }
    h.stop();
    bail!("timed out")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_is_soft_and_faded() {
        let t = tone(48_000, 0.5);
        assert_eq!(t.len(), 24_000);
        assert_eq!(t[0], 0.0, "starts silent (fade in)");
        assert!(t.last().unwrap().abs() < 1e-3, "ends silent (fade out)");
        let peak = crate::levels::peak(&t);
        assert!((peak - 0.25).abs() < 0.01, "about -12 dBFS, got {peak}");
    }

    #[test]
    fn resampling_keeps_duration_and_shape() {
        let s: Vec<f32> = (0..48_000).map(|i| (i as f32 / 48_000.0 * std::f32::consts::TAU * 100.0).sin()).collect();
        let r = resample(&s, 48_000, 16_000);
        assert_eq!(r.len(), 16_000);
        assert!((r[4000] - s[12_000]).abs() < 1e-3);
        let up = resample(&[0.0, 1.0], 1, 2);
        assert_eq!(up, vec![0.0, 0.5, 1.0, 1.0][..up.len()].to_vec());
        assert_eq!(resample(&[1.0, 2.0], 8, 8), vec![1.0, 2.0]);
    }
}
