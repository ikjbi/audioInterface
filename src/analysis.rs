//! Session analysis: did the mic level or gain drop during the recording,
//! and why?
//!
//! Input is the one-row-per-second log written by the recorder. People
//! pause, listen and talk at different volumes, so we never compare single
//! seconds. Instead we compute a **speaking level** per sliding window (the
//! median RMS of the seconds where there is sound) and compare it to a
//! **baseline** taken from the start of the session.
//!
//! Each drop is then attributed:
//! * the OS gain was lowered (typically an app's "automatically adjust
//!   microphone" feature, e.g. Teams), or
//! * the gain was unchanged, so the cause is acoustic or hardware (mic moved,
//!   headset slipping, Bluetooth link degrading, battery saving, ...).
//!
//! It also reports gain changes, muting, signal loss, digital silence and
//! Bluetooth hands-free switches, all with timestamps.

use serde::{Deserialize, Serialize};
use std::fmt;

/// One logged second.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Seconds since the recording started.
    pub t: f64,
    /// Local wall-clock time, `HH:MM:SS`.
    pub time: String,
    /// Loudest peak in this second, dBFS.
    pub peak_db: f32,
    /// RMS level, dBFS (average over the second).
    pub rms_db: f32,
    /// OS input gain / volume in percent, if the device has one.
    pub gain_pct: Option<f32>,
    pub muted: Option<bool>,
    /// `ok`, `silent`, `lost` or `starting`.
    pub status: String,
    pub hands_free: bool,
    /// Other apps using the device, `;`-separated.
    pub in_use_by: String,
    /// Notable things that happened in this second (`;`-separated).
    pub event: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Finding {
    /// The OS gain/volume setting changed.
    GainChange {
        t: f64,
        time: String,
        from: f32,
        to: f32,
        /// What happened around it, e.g. "2 s after Microsoft Teams stopped using the device".
        #[serde(default)]
        context: Option<AppContext>,
    },
    /// An app started or stopped using the device (from the `in_use_by` column).
    AppUse { t: f64, time: String, app: String, started: bool },
    /// Speaking level fell well below the session's baseline.
    LevelDrop {
        start: f64,
        end: f64,
        start_time: String,
        end_time: String,
        baseline_db: f32,
        lowest_db: f32,
        /// A gain decrease happened shortly before or during the drop.
        gain_lowered: bool,
    },
    /// Contiguous period with a signal problem.
    Period { start: f64, end: f64, start_time: String, what: PeriodKind },
    /// Bluetooth headset switched into or out of hands-free mode.
    HandsFree { t: f64, time: String, on: bool },
    /// Free-text event from the log (disconnects, gain-lock restores, ...).
    Event { t: f64, time: String, text: String },
}

/// An app start/stop near a gain change: likely the cause.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppContext {
    pub app: String,
    /// True if the app started using the device, false if it stopped.
    pub started: bool,
    /// Seconds from the app change to the gain change (negative: the gain changed first).
    pub seconds_after: f64,
}

impl AppContext {
    fn describe(&self) -> String {
        let what = if self.started { "started" } else { "stopped" };
        let s = self.seconds_after.abs().round() as i64;
        if s == 0 {
            format!("as {} {what} using the device", self.app)
        } else if self.seconds_after > 0.0 {
            format!("{s} s after {} {what} using the device", self.app)
        } else {
            format!("{s} s before {} {what} using the device", self.app)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodKind {
    Muted,
    SignalLost,
    DigitalSilence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub duration_s: f64,
    pub seconds_logged: usize,
    /// Speaking level at the start of the session (dBFS RMS).
    pub baseline_db: Option<f32>,
    /// Speaking level over time: (window centre seconds, dBFS).
    pub speaking_level: Vec<(f64, f32)>,
    pub findings: Vec<Finding>,
    /// One-paragraph plain-language conclusion.
    pub verdict: String,
}

/// Tuning knobs (defaults suit speech in calls).
#[derive(Clone, Debug)]
pub struct Params {
    /// Seconds whose RMS is above this are "sound" (dBFS). Ignored when
    /// `auto_gate` is set.
    pub gate_db: f32,
    /// Derive the gate from the session's own noise floor (see
    /// [`crate::levels::speech_gate`]), so quiet mics are analysed too.
    pub auto_gate: bool,
    /// Sliding window length, seconds.
    pub window_s: usize,
    /// Step between windows, seconds.
    pub step_s: usize,
    /// A window needs this many sound seconds to count as "speaking".
    pub min_active_s: usize,
    /// How many of the first valid windows form the baseline.
    pub baseline_windows: usize,
    /// Drop threshold below baseline, dB.
    pub drop_db: f32,
    /// Ignore drops shorter than this, seconds.
    pub min_drop_s: f64,
    /// Gain changes smaller than this (percentage points) are ignored.
    pub min_gain_change_pct: f32,
    /// Ignore problem periods shorter than this, seconds.
    pub min_period_s: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            gate_db: -55.0,
            auto_gate: true,
            window_s: 60,
            step_s: 10,
            min_active_s: 8,
            baseline_windows: 3,
            drop_db: 6.0,
            min_drop_s: 30.0,
            min_gain_change_pct: 2.0,
            min_period_s: 3.0,
        }
    }
}

fn median(v: &mut [f32]) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
}

/// Noise floor of a session: the 10th percentile of per-second RMS while the
/// signal is OK and not muted.
pub fn noise_floor(samples: &[Sample]) -> Option<f32> {
    let mut v: Vec<f32> = samples
        .iter()
        .filter(|s| s.status == "ok" && s.muted != Some(true) && s.rms_db > crate::levels::FLOOR_DB)
        .map(|s| s.rms_db)
        .collect();
    if v.len() < 10 {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[v.len() / 10])
}

/// Params with the gate resolved for this session.
fn effective(samples: &[Sample], p: &Params) -> Params {
    let mut p = p.clone();
    if p.auto_gate {
        p.gate_db = crate::levels::speech_gate(noise_floor(samples));
    }
    p
}

fn is_sound(s: &Sample, p: &Params) -> bool {
    s.rms_db > p.gate_db && s.status == "ok" && s.muted != Some(true)
}

/// Speaking level per sliding window: (centre t, median RMS of sound seconds).
pub fn speaking_level(samples: &[Sample], p: &Params) -> Vec<(f64, f32)> {
    let p = &effective(samples, p);
    let mut out = Vec::new();
    if samples.is_empty() {
        return out;
    }
    let mut start = 0;
    while start < samples.len() {
        let end = (start + p.window_s).min(samples.len());
        let mut active: Vec<f32> = samples[start..end].iter().filter(|s| is_sound(s, p)).map(|s| s.rms_db).collect();
        if active.len() >= p.min_active_s {
            let centre = (samples[start].t + samples[end - 1].t) / 2.0;
            out.push((centre, median(&mut active).unwrap()));
        }
        if end == samples.len() {
            break;
        }
        start += p.step_s;
    }
    out
}

pub fn analyze(samples: &[Sample]) -> Report {
    analyze_with(samples, &Params::default())
}

pub fn analyze_with(samples: &[Sample], p: &Params) -> Report {
    let p = &effective(samples, p);
    let mut findings = Vec::new();
    let duration_s = samples.last().map(|s| s.t).unwrap_or(0.0);

    // Gain changes, merging rapid steps (an app ramping the gain) into one finding.
    let mut i = 1;
    while i < samples.len() {
        let (Some(a), Some(_)) = (samples[i - 1].gain_pct, samples[i].gain_pct) else {
            i += 1;
            continue;
        };
        if (samples[i].gain_pct.unwrap() - a).abs() >= 0.5 {
            let start = i;
            let mut j = i;
            while j + 1 < samples.len()
                && samples[j + 1].t - samples[j].t <= 5.0
                && samples[j + 1].gain_pct.zip(samples[j].gain_pct).is_some_and(|(x, y)| (x - y).abs() >= 0.5)
            {
                j += 1;
            }
            let to = samples[j].gain_pct.unwrap();
            if (to - a).abs() >= p.min_gain_change_pct {
                findings.push(Finding::GainChange {
                    t: samples[start].t,
                    time: samples[start].time.clone(),
                    from: a,
                    to,
                    context: None,
                });
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }

    // Speaking level and drops relative to the baseline.
    let levels = speaking_level(samples, p);
    let mut first: Vec<f32> = levels.iter().take(p.baseline_windows).map(|l| l.1).collect();
    let baseline_db = median(&mut first);
    if let Some(base) = baseline_db {
        let mut k = 0;
        while k < levels.len() {
            if levels[k].1 < base - p.drop_db {
                let s = k;
                while k + 1 < levels.len() && levels[k + 1].1 < base - p.drop_db {
                    k += 1;
                }
                // Window centres -> approximate time span of the drop.
                let half = p.window_s as f64 / 2.0;
                let start = (levels[s].0 - half).max(0.0);
                let end = (levels[k].0 + half).min(duration_s);
                let lowest = levels[s..=k].iter().map(|l| l.1).fold(f32::INFINITY, f32::min);
                if levels[k].0 - levels[s].0 + p.step_s as f64 >= p.min_drop_s {
                    let gain_lowered = findings.iter().any(|f| {
                        matches!(f, Finding::GainChange { t, from, to, .. }
                            if *to < *from && *t >= start - p.window_s as f64 && *t <= end)
                    });
                    findings.push(Finding::LevelDrop {
                        start,
                        end,
                        start_time: time_at(samples, start),
                        end_time: time_at(samples, end),
                        baseline_db: base,
                        lowest_db: lowest,
                        gain_lowered,
                    });
                }
            }
            k += 1;
        }
    }

    // Problem periods.
    type Pred = fn(&Sample) -> bool;
    let kinds: [(PeriodKind, Pred); 3] = [
        (PeriodKind::Muted, |s| s.muted == Some(true)),
        (PeriodKind::SignalLost, |s| s.status == "lost"),
        (PeriodKind::DigitalSilence, |s| s.status == "silent"),
    ];
    for (kind, pred) in kinds {
        let mut k = 0;
        while k < samples.len() {
            if pred(&samples[k]) {
                let s = k;
                while k + 1 < samples.len() && pred(&samples[k + 1]) {
                    k += 1;
                }
                if samples[k].t - samples[s].t + 1.0 >= p.min_period_s {
                    findings.push(Finding::Period {
                        start: samples[s].t,
                        end: samples[k].t + 1.0,
                        start_time: samples[s].time.clone(),
                        what: kind,
                    });
                }
            }
            k += 1;
        }
    }

    // Hands-free switches and logged events.
    for w in samples.windows(2) {
        if w[0].hands_free != w[1].hands_free {
            findings.push(Finding::HandsFree { t: w[1].t, time: w[1].time.clone(), on: w[1].hands_free });
        }
    }
    // Apps starting / stopping to use the device (e.g. a Teams call starting or ending).
    let apps_of = |s: &Sample| -> std::collections::BTreeSet<String> {
        s.in_use_by.split(';').map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).collect()
    };
    let mut app_changes = Vec::new();
    for w in samples.windows(2) {
        let (before, after) = (apps_of(&w[0]), apps_of(&w[1]));
        for app in after.difference(&before) {
            app_changes.push((w[1].t, w[1].time.clone(), app.clone(), true));
        }
        for app in before.difference(&after) {
            app_changes.push((w[1].t, w[1].time.clone(), app.clone(), false));
        }
    }
    // Explain gain changes by a nearby app start/stop (within 15 s).
    for f in findings.iter_mut() {
        if let Finding::GainChange { t, context, .. } = f {
            *context = app_changes
                .iter()
                .filter(|c| (*t - c.0).abs() <= 15.0)
                .min_by(|a, b| (*t - a.0).abs().total_cmp(&(*t - b.0).abs()))
                .map(|c| AppContext { app: c.2.clone(), started: c.3, seconds_after: *t - c.0 });
        }
    }
    for (t, time, app, started) in app_changes {
        findings.push(Finding::AppUse { t, time, app, started });
    }

    for s in samples {
        let markers = ["Recording started", "Recording stopped"];
        for e in
            s.event.split(';').map(str::trim).filter(|e| !e.is_empty() && !markers.iter().any(|m| e.starts_with(m)))
        {
            findings.push(Finding::Event { t: s.t, time: s.time.clone(), text: e.to_string() });
        }
    }

    findings.sort_by(|a, b| finding_t(a).total_cmp(&finding_t(b)));
    let verdict = verdict(&findings, baseline_db, duration_s, levels.is_empty());
    Report { duration_s, seconds_logged: samples.len(), baseline_db, speaking_level: levels, findings, verdict }
}

fn time_at(samples: &[Sample], t: f64) -> String {
    samples
        .iter()
        .min_by(|a, b| (a.t - t).abs().total_cmp(&(b.t - t).abs()))
        .map(|s| s.time.clone())
        .unwrap_or_default()
}

pub fn finding_t(f: &Finding) -> f64 {
    match f {
        Finding::GainChange { t, .. }
        | Finding::HandsFree { t, .. }
        | Finding::Event { t, .. }
        | Finding::AppUse { t, .. } => *t,
        Finding::LevelDrop { start, .. } | Finding::Period { start, .. } => *start,
    }
}

fn verdict(findings: &[Finding], baseline: Option<f32>, duration: f64, no_speech: bool) -> String {
    let drops: Vec<&Finding> = findings.iter().filter(|f| matches!(f, Finding::LevelDrop { .. })).collect();
    // Gain changes explained by an app starting/stopping (call start/end) are reported
    // separately and not counted as problems.
    let gain_downs = findings
        .iter()
        .filter(|f| matches!(f, Finding::GainChange { from, to, context: None, .. } if to < from))
        .count();
    let explained: Vec<String> = findings
        .iter()
        .filter_map(|f| match f {
            Finding::GainChange { time, from, to, context: Some(c), .. } => {
                let why = if c.started {
                    "the app setting the mic level as the call started"
                } else {
                    "the app restoring the mic level it had before the call, as the call ended"
                };
                Some(format!(
                    "The gain change at {time} ({from:.0}% → {to:.0}%) happened {}: most likely {why}, not a problem during the call.",
                    c.describe()
                ))
            }
            _ => None,
        })
        .collect();
    let lost = findings.iter().filter(|f| matches!(f, Finding::Period { what: PeriodKind::SignalLost, .. })).count();
    let mut parts = Vec::new();
    if duration < 120.0 {
        parts.push("The recording is short, so drift analysis is limited.".to_string());
    }
    if no_speech {
        parts.push("Not enough speech was detected to measure your speaking level.".to_string());
    } else if let Some(b) = baseline {
        parts.push(format!("Your speaking level at the start was about {b:.0} dBFS."));
    }
    if let Some(b) = baseline
        && !no_speech
        && b < crate::levels::WEAK_SPEECH_DB
    {
        parts.push(format!(
            "Your voice reaches the computer very weakly (about {b:.0} dBFS; clear speech is usually -40 to -15). \
             Colleagues may hear you quietly, or noise suppression may cut you off. Raise the input gain, turn on \
             Microphone Boost (Windows: Sound settings → the mic → Properties → Levels), or move the mic closer \
             to your mouth."
        ));
    }
    if drops.is_empty() {
        if !no_speech {
            parts.push("It did not drop significantly during the session.".into());
        }
    } else {
        for d in &drops {
            if let Finding::LevelDrop { start_time, end_time, baseline_db, lowest_db, gain_lowered, .. } = d {
                let cause = if *gain_lowered {
                    "The OS input gain was lowered around then. This is usually an app's \"automatically adjust \
                     microphone\" setting (in Teams: Settings → Devices). Lock the gain to prevent it."
                } else {
                    "The OS gain did not change, so the cause is acoustic or hardware: mic position, headset \
                     fit, Bluetooth link quality or a device power-saving feature."
                };
                parts.push(format!(
                    "Level dropped by {:.0} dB between {start_time} and {end_time}. {cause}",
                    baseline_db - lowest_db
                ));
            }
        }
    }
    if gain_downs > 0 && drops.iter().all(|d| !matches!(d, Finding::LevelDrop { gain_lowered: true, .. })) {
        parts.push(format!("The OS gain was lowered {gain_downs} time(s) during the session (see the findings)."));
    }
    parts.extend(explained);
    if lost > 0 {
        parts.push(format!("The signal was lost {lost} time(s) (device disconnected or stopped delivering audio)."));
    }
    parts.join(" ")
}

impl fmt::Display for PeriodKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PeriodKind::Muted => "muted",
            PeriodKind::SignalLost => "signal lost",
            PeriodKind::DigitalSilence => "digital silence (only zeros)",
        })
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Finding::GainChange { time, from, to, context, .. } => {
                let dir = if to < from { "lowered" } else { "raised" };
                write!(f, "{time}  OS gain {dir} from {from:.0}% to {to:.0}%")?;
                if let Some(c) = context {
                    write!(f, " ({})", c.describe())?;
                }
                Ok(())
            }
            Finding::AppUse { time, app, started, .. } => {
                write!(f, "{time}  {app} {} using the device", if *started { "started" } else { "stopped" })
            }
            Finding::LevelDrop { start_time, end_time, baseline_db, lowest_db, gain_lowered, .. } => write!(
                f,
                "{start_time}–{end_time}  speaking level dropped {:.0} dB (from {baseline_db:.0} to {lowest_db:.0} dBFS){}",
                baseline_db - lowest_db,
                if *gain_lowered { ", OS gain was lowered" } else { ", gain unchanged" }
            ),
            Finding::Period { start, end, start_time, what } => {
                write!(f, "{start_time}  {what} for {:.0} s", end - start)
            }
            Finding::HandsFree { time, on, .. } => {
                write!(f, "{time}  Bluetooth {} hands-free mode", if *on { "switched to" } else { "left" })
            }
            Finding::Event { time, text, .. } => write!(f, "{time}  {text}"),
        }
    }
}

impl Report {
    /// Plain-text report (CLI output and the `.report.txt` file).
    pub fn to_text(&self, title: &str) -> String {
        let mut s = format!("{title}\n{}\n\n", "=".repeat(title.chars().count()));
        s += &format!(
            "Duration: {}   Seconds logged: {}   Baseline speaking level: {}\n\n",
            fmt_duration(self.duration_s),
            self.seconds_logged,
            self.baseline_db.map(|b| format!("{b:.1} dBFS")).unwrap_or_else(|| "n/a".into())
        );
        s += "Verdict\n-------\n";
        s += &self.verdict;
        s += "\n\nFindings\n--------\n";
        if self.findings.is_empty() {
            s += "(none)\n";
        }
        for f in &self.findings {
            s += &format!("{f}\n");
        }
        s
    }
}

pub fn fmt_duration(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 {
        format!("{}h {:02}m {:02}s", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

/// Read a recorder CSV.
pub fn read_csv(path: &std::path::Path) -> anyhow::Result<Vec<Sample>> {
    let mut r = csv::Reader::from_path(path)?;
    let mut out = Vec::new();
    for row in r.deserialize() {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech-like test data: talking for 7 of every 10 s at `level`, pauses at -70.
    fn call(minutes: usize, level_at: impl Fn(usize) -> f32, gain_at: impl Fn(usize) -> f32) -> Vec<Sample> {
        (0..minutes * 60)
            .map(|i| {
                let talking = i % 10 < 7;
                let rms = if talking { level_at(i) } else { -70.0 };
                Sample {
                    t: i as f64,
                    time: format!("{:02}:{:02}:{:02}", 10 + i / 3600, (i / 60) % 60, i % 60),
                    peak_db: rms + 12.0,
                    rms_db: rms,
                    gain_pct: Some(gain_at(i)),
                    muted: Some(false),
                    status: "ok".into(),
                    hands_free: false,
                    in_use_by: "Microsoft Teams".into(),
                    event: String::new(),
                }
            })
            .collect()
    }

    #[test]
    fn steady_call_has_no_drops() {
        let r = analyze(&call(20, |_| -24.0, |_| 80.0));
        assert!((r.baseline_db.unwrap() - -24.0).abs() < 0.01);
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        assert!(r.verdict.contains("did not drop"), "{}", r.verdict);
    }

    #[test]
    fn gain_lowered_by_app_is_detected_and_blamed() {
        // At minute 10 an app lowers the gain from 80% to 40%; level falls 10 dB.
        let r = analyze(&call(20, |i| if i < 600 { -24.0 } else { -34.0 }, |i| if i < 600 { 80.0 } else { 40.0 }));
        let gain = r.findings.iter().find(|f| matches!(f, Finding::GainChange { .. })).unwrap();
        assert!(matches!(gain, Finding::GainChange { t, from, to, .. } if *t == 600.0 && *from == 80.0 && *to == 40.0));
        let drop = r.findings.iter().find(|f| matches!(f, Finding::LevelDrop { .. })).unwrap();
        match drop {
            Finding::LevelDrop { start, gain_lowered, lowest_db, .. } => {
                assert!(*gain_lowered);
                assert!((*lowest_db - -34.0).abs() < 0.01);
                assert!((540.0..=620.0).contains(start), "drop starts near minute 10, got {start}");
            }
            _ => unreachable!(),
        }
        assert!(r.verdict.contains("gain was lowered"), "{}", r.verdict);
        assert!(r.to_text("t").contains("OS gain lowered from 80% to 40%"));
    }

    #[test]
    fn gain_change_at_call_end_is_attributed_to_the_app() {
        // The situation from a real Teams call: gain steady at 100% while Teams uses the
        // mic, Teams lets go at 85 s, gain drops to 34% two seconds later.
        let mut s = call(2, |_| -60.0, |i| if i < 87 { 100.0 } else { 34.0 });
        for (i, x) in s.iter_mut().enumerate() {
            x.in_use_by = if i < 85 { "Microsoft Teams".into() } else { String::new() };
        }
        let r = analyze(&s);
        let g = r.findings.iter().find(|f| matches!(f, Finding::GainChange { .. })).unwrap();
        match g {
            Finding::GainChange { context: Some(c), .. } => {
                assert_eq!(c.app, "Microsoft Teams");
                assert!(!c.started);
                assert_eq!(c.seconds_after, 2.0);
            }
            other => panic!("no context: {other:?}"),
        }
        assert!(g.to_string().contains("2 s after Microsoft Teams stopped using the device"), "{g}");
        assert!(r.findings.iter().any(|f| matches!(f, Finding::AppUse { started: false, .. })));
        assert!(r.verdict.contains("restoring the mic level it had before the call"), "{}", r.verdict);
        assert!(!r.verdict.contains("lowered 1 time"), "explained changes are not counted: {}", r.verdict);
    }

    #[test]
    fn quiet_analog_headset_is_still_analysed_and_flagged_weak() {
        // Speech around -60 dBFS over a -80 noise floor: invisible to a fixed -55 gate.
        let mut s = call(5, |_| -60.0, |_| 34.0);
        for x in s.iter_mut() {
            if x.rms_db < -65.0 {
                x.rms_db = -80.0;
            }
        }
        let r = analyze(&s);
        assert!((r.baseline_db.unwrap() - -60.0).abs() < 0.5, "{:?}", r.baseline_db);
        assert!(r.verdict.contains("very weakly"), "{}", r.verdict);
        // A normal-level call is not flagged.
        assert!(!analyze(&call(5, |_| -30.0, |_| 80.0)).verdict.contains("very weakly"));
    }

    #[test]
    fn drop_without_gain_change_is_acoustic() {
        let r = analyze(&call(20, |i| if (480..900).contains(&i) { -33.0 } else { -24.0 }, |_| 75.0));
        let drop = r.findings.iter().find(|f| matches!(f, Finding::LevelDrop { .. })).unwrap();
        assert!(matches!(drop, Finding::LevelDrop { gain_lowered: false, .. }));
        assert!(r.verdict.contains("did not change"), "{}", r.verdict);
        assert!(!r.findings.iter().any(|f| matches!(f, Finding::GainChange { .. })));
    }

    #[test]
    fn small_dips_and_short_drops_are_ignored() {
        // 3 dB dip: below the threshold.
        assert!(
            analyze(&call(15, |i| if i > 400 { -27.0 } else { -24.0 }, |_| 80.0))
                .findings
                .iter()
                .all(|f| !matches!(f, Finding::LevelDrop { .. }))
        );
        // Gain wiggle of 1 percentage point: ignored.
        assert!(analyze(&call(5, |_| -24.0, |i| if i % 100 < 50 { 80.0 } else { 81.0 })).findings.is_empty());
    }

    #[test]
    fn gain_ramp_is_one_finding() {
        // Gain ramps down 80 -> 50 over 6 consecutive seconds.
        let r = analyze(&call(
            5,
            |_| -24.0,
            |i| {
                if i < 100 {
                    80.0
                } else if i < 106 {
                    80.0 - (i - 99) as f32 * 5.0
                } else {
                    50.0
                }
            },
        ));
        let gains: Vec<_> = r.findings.iter().filter(|f| matches!(f, Finding::GainChange { .. })).collect();
        assert_eq!(gains.len(), 1, "{gains:?}");
        assert!(matches!(gains[0], Finding::GainChange { from, to, .. } if *from == 80.0 && *to == 50.0));
    }

    #[test]
    fn periods_handsfree_and_events() {
        let mut s = call(5, |_| -24.0, |_| 80.0);
        for x in &mut s[100..110] {
            x.status = "lost".into();
        }
        for x in &mut s[200..202] {
            x.status = "silent".into(); // too short (< 3 s)
        }
        for x in &mut s[150..160] {
            x.muted = Some(true);
        }
        for x in &mut s[250..] {
            x.hands_free = true;
        }
        s[120].event = "Disconnected: Headset (input); Connected: Headset (input)".into();
        let r = analyze(&s);
        let periods: Vec<_> = r
            .findings
            .iter()
            .filter_map(|f| match f {
                Finding::Period { what, start, end, .. } => Some((*what, *start, *end)),
                _ => None,
            })
            .collect();
        assert_eq!(periods, vec![(PeriodKind::SignalLost, 100.0, 110.0), (PeriodKind::Muted, 150.0, 160.0)]);
        assert!(r.findings.iter().any(|f| matches!(f, Finding::HandsFree { on: true, t, .. } if *t == 250.0)));
        assert_eq!(r.findings.iter().filter(|f| matches!(f, Finding::Event { .. })).count(), 2);
        assert!(r.verdict.contains("lost 1 time"), "{}", r.verdict);
        // Sorted by time.
        let ts: Vec<f64> = r.findings.iter().map(finding_t).collect();
        assert!(ts.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn silence_only_session() {
        let r = analyze(&call(3, |_| -80.0, |_| 50.0));
        assert!(r.baseline_db.is_none());
        assert!(r.verdict.contains("Not enough speech"), "{}", r.verdict);
        assert_eq!(analyze(&[]).seconds_logged, 0);
    }

    #[test]
    fn csv_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.csv");
        let rows = call(2, |_| -20.0, |_| 70.0);
        let mut w = csv::Writer::from_path(&path).unwrap();
        for r in &rows {
            w.serialize(r).unwrap();
        }
        w.flush().unwrap();
        assert_eq!(read_csv(&path).unwrap(), rows);
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(75.0), "1m 15s");
        assert_eq!(fmt_duration(3725.0), "1h 02m 05s");
    }
}
