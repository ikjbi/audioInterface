// Hide the console window on Windows for release builds, so the GUI starts
// cleanly from Explorer. The CLI subcommands still print when run from a
// terminal, because we re-attach to the parent console below.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::{Context, Result};
use audio_interface::backend::{self, AudioBackend, mock::MockBackend};
use audio_interface::levels::{LevelCell, MeterState, SignalStatus, meter_position};
use audio_interface::meter::MeterEngine;
use audio_interface::model::{Device, Direction, resolve_device};
use audio_interface::service::{BackendFactory, Service};
use audio_interface::settings::Settings;
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::HashMap;
use std::io::Write;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "audio-interface", version, about = "Manage audio devices: defaults, volume, live levels.")]
struct Cli {
    /// Use a simulated set of devices instead of the real ones (for demos/testing).
    #[arg(long, global = true, env = "AUDIO_INTERFACE_MOCK")]
    mock: bool,

    /// Use this settings file instead of the default one in the OS config directory.
    #[arg(long, global = true, env = "AUDIO_INTERFACE_SETTINGS", value_name = "PATH")]
    settings: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open the desktop app (default when no command is given).
    Gui,
    /// List audio devices.
    List {
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
        /// Only list one direction.
        #[arg(long, value_enum)]
        direction: Option<Direction>,
    },
    /// Make a device the system default.
    SetDefault {
        #[arg(value_enum)]
        direction: Direction,
        /// Device ID, exact name, or a unique part of the name.
        device: String,
    },
    /// Print or set a device's volume (0-100).
    Volume {
        #[arg(value_enum)]
        direction: Direction,
        device: String,
        percent: Option<f32>,
    },
    /// Mute, unmute or toggle a device.
    Mute {
        #[arg(value_enum)]
        direction: Direction,
        device: String,
        #[arg(value_enum, default_value_t = MuteAction::Toggle)]
        action: MuteAction,
    },
    /// Show a live level meter in the terminal (defaults to the default input).
    Meter {
        /// Device to meter. Defaults to the default input.
        device: Option<String>,
        #[arg(long, value_enum, default_value_t = Direction::Input)]
        direction: Direction,
        /// Stop after this many seconds (default: run until Ctrl+C).
        #[arg(long)]
        seconds: Option<f32>,
    },
    /// Log a device's levels and gain once per second (CSV) until Ctrl+C or
    /// `--seconds`, then print a drop-analysis report. Use `--auto` to instead
    /// watch for calls and record every input another app starts using.
    Record {
        /// Device to record. Defaults to the default input.
        device: Option<String>,
        #[arg(long, value_enum, default_value_t = Direction::Input)]
        direction: Direction,
        /// Also record the audio (16 kHz mono WAV).
        #[arg(long)]
        audio: bool,
        /// Stop after this many seconds.
        #[arg(long)]
        seconds: Option<f64>,
        /// Folder for the recordings (default: Documents/Audio Interface Recordings).
        #[arg(long, value_name = "DIR")]
        out: Option<std::path::PathBuf>,
        /// Watch mode: auto-record any input that another app starts using.
        #[arg(long, conflicts_with = "device")]
        auto: bool,
        /// With --auto: only for apps whose name contains this (repeatable), e.g. --app teams
        #[arg(long = "app", requires = "auto")]
        apps: Vec<String>,
    },
    /// Analyze a recording (CSV written by `record` or the app) and print the report.
    Analyze {
        file: std::path::PathBuf,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Write the app icon as a PNG (used when packaging).
    #[command(hide = true)]
    ExportIcon {
        path: std::path::PathBuf,
        #[arg(long, default_value_t = 1024)]
        size: u32,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum MuteAction {
    On,
    Off,
    Toggle,
}

fn main() {
    #[cfg(all(windows, not(debug_assertions)))]
    attach_parent_console();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn make_factory(mock: bool) -> BackendFactory {
    if mock {
        // One shared mock instance, so changes survive reconnects.
        let mut demo = MockBackend::demo();
        demo.animate = true;
        let shared = std::sync::Arc::new(std::sync::Mutex::new(Some(demo)));
        Box::new(move || {
            let m = shared.lock().unwrap().take().context("mock backend already in use")?;
            Ok(Box::new(m) as Box<dyn AudioBackend>)
        })
    } else {
        Box::new(backend::create)
    }
}

fn make_backend(mock: bool) -> Result<Box<dyn AudioBackend>> {
    if mock {
        let mut m = MockBackend::demo();
        m.animate = true;
        Ok(Box::new(m))
    } else {
        backend::create()
    }
}

fn run(cli: Cli) -> Result<()> {
    let mock = cli.mock;
    let settings_path = cli.settings.clone().or_else(Settings::default_path);
    match cli.command.unwrap_or(Cmd::Gui) {
        Cmd::Gui => {
            let path = settings_path;
            let settings = path.as_deref().map(Settings::load).unwrap_or_default();
            let factory = make_factory(mock);
            let service = Service::start(settings.clone(), factory);
            audio_interface::gui::run(service, settings, path).map_err(|e| anyhow::anyhow!("GUI error: {e}"))
        }
        Cmd::List { json, direction } => {
            let mut b = make_backend(mock)?;
            let mut devices = b.devices()?;
            if let Some(dir) = direction {
                devices.retain(|d| d.direction == dir);
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&devices)?);
            } else {
                print_table(b.api_name(), &devices);
            }
            Ok(())
        }
        Cmd::SetDefault { direction, device } => {
            let mut b = make_backend(mock)?;
            let devices = b.devices()?;
            let d = resolve_device(&devices, direction, &device)?;
            b.set_default(&d.id, direction)?;
            println!("Default {direction} set to \"{}\"", d.name);
            if d.transport == audio_interface::model::Transport::Bluetooth && direction == Direction::Input {
                println!("note: using a Bluetooth headset mic switches it to hands-free (call-quality) audio");
            }
            Ok(())
        }
        Cmd::Volume { direction, device, percent } => {
            let mut b = make_backend(mock)?;
            let devices = b.devices()?;
            let d = resolve_device(&devices, direction, &device)?.clone();
            match percent {
                Some(p) => {
                    b.set_volume(&d.id, direction, p / 100.0)?;
                    let now = b.devices()?.into_iter().find(|x| x.key() == d.key()).and_then(|x| x.volume);
                    println!("{} volume: {}", d.name, fmt_volume(now));
                }
                None => println!("{} volume: {}", d.name, fmt_volume(d.volume)),
            }
            Ok(())
        }
        Cmd::Mute { direction, device, action } => {
            let mut b = make_backend(mock)?;
            let devices = b.devices()?;
            let d = resolve_device(&devices, direction, &device)?;
            let target = match action {
                MuteAction::On => true,
                MuteAction::Off => false,
                MuteAction::Toggle => !d.muted.unwrap_or(false),
            };
            b.set_mute(&d.id, direction, target)?;
            println!("{} {}", d.name, if target { "muted" } else { "unmuted" });
            Ok(())
        }
        Cmd::Meter { device, direction, seconds } => run_meter(mock, device, direction, seconds),
        Cmd::Record { device, direction, audio, seconds, out, auto, apps } => {
            let mut settings = settings_path.as_deref().map(Settings::load).unwrap_or_default();
            if let Some(out) = out {
                settings.recordings_dir = Some(out);
            }
            run_record(mock, settings, device, direction, audio, seconds, auto, apps)
        }
        Cmd::Analyze { file, json } => {
            let rows =
                audio_interface::analysis::read_csv(&file).with_context(|| format!("read {}", file.display()))?;
            let report = audio_interface::analysis::analyze(&rows);
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                print!("{}", report.to_text(&format!("Audio session report: {name}")));
            }
            Ok(())
        }
        Cmd::ExportIcon { path, size } => {
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ico")) {
                audio_interface::icon::write_ico(&path)
            } else {
                audio_interface::icon::write_png(&path, size)
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_record(
    mock: bool,
    mut settings: Settings,
    device: Option<String>,
    direction: Direction,
    audio: bool,
    seconds: Option<f64>,
    auto: bool,
    apps: Vec<String>,
) -> Result<()> {
    use audio_interface::service::Request;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst)).context("install Ctrl+C handler")?;
    }
    if auto {
        settings.auto_record_calls = true;
        settings.auto_record_apps = apps;
        settings.record_audio = audio;
    }
    let dir = settings.recordings_dir();
    let service = Service::start(settings, make_factory(mock));

    // Wait for the first device list.
    let start = Instant::now();
    while service.snapshot().generation == 0 {
        if let Some(e) = service.snapshot().error.clone() {
            anyhow::bail!("{e}");
        }
        if start.elapsed() > Duration::from_secs(10) {
            anyhow::bail!("timed out waiting for the audio system");
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if auto {
        eprintln!("Watching for calls; recordings go to {}. Ctrl+C to stop.", dir.display());
    } else {
        let devices = service.snapshot().devices.clone();
        let d = match &device {
            Some(q) => resolve_device(&devices, direction, q)?.clone(),
            None => devices
                .iter()
                .find(|d| d.direction == direction && d.is_default)
                .cloned()
                .with_context(|| format!("no default {direction} device"))?,
        };
        service.send(Request::StartRecording { key: d.key(), audio });
        eprintln!("Recording \"{}\" to {}. Ctrl+C to stop.", d.name, dir.display());
    }

    let mut finished_seen = 0;
    let started = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(250));
        {
            let snap = service.snapshot();
            for f in snap.finished.iter().skip(finished_seen) {
                println!();
                print!(
                    "{}",
                    f.report.to_text(&format!("Audio session report: {} ({})", f.info.device_name, f.info.started))
                );
                println!("\nFiles: {}", f.info.csv_path.display());
                for w in &f.info.wav_paths {
                    println!("       {}", w.display());
                }
            }
            finished_seen = snap.finished.len();
            match snap.recordings.first() {
                Some(r) => {
                    let last = r.recent.last();
                    print!(
                        "\r⏺ {}  {}  level {:>6.1} dBFS  gain {:>5}  {}   ",
                        r.info.device_name,
                        audio_interface::analysis::fmt_duration(r.elapsed_s),
                        last.map(|x| x.rms_db).unwrap_or(-96.0),
                        last.and_then(|x| x.gain_pct).map(|g| format!("{g:.0}%")).unwrap_or_else(|| "n/a".into()),
                        last.map(|x| x.status.as_str()).unwrap_or("starting")
                    );
                }
                None if auto => print!("\rWaiting for a call… ({} recorded so far)        ", finished_seen),
                None => {}
            }
            let _ = std::io::stdout().flush();
            if !auto && snap.recordings.is_empty() && finished_seen > 0 {
                return Ok(());
            }
            if let Some(e) = snap.events.back().filter(|e| e.kind == audio_interface::service::EventKind::Error)
                && !auto
                && snap.recordings.is_empty()
                && started.elapsed() > Duration::from_secs(2)
            {
                anyhow::bail!("{}", e.text);
            }
        }
        let timed_out = seconds.is_some_and(|s| started.elapsed().as_secs_f64() >= s);
        if stop.load(Ordering::SeqCst) || timed_out {
            let keys: Vec<_> = service.snapshot().recordings.iter().map(|r| r.info.key.clone()).collect();
            for k in &keys {
                service.send(Request::StopRecording(k.clone()));
            }
            // Wait for the reports.
            let t = Instant::now();
            while service.snapshot().finished.len() < finished_seen + keys.len()
                && t.elapsed() < Duration::from_secs(15)
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            let snap = service.snapshot();
            for f in snap.finished.iter().skip(finished_seen) {
                println!();
                print!(
                    "{}",
                    f.report.to_text(&format!("Audio session report: {} ({})", f.info.device_name, f.info.started))
                );
                println!("\nFiles: {}", f.info.csv_path.display());
                for w in &f.info.wav_paths {
                    println!("       {}", w.display());
                }
            }
            return Ok(());
        }
    }
}

fn fmt_volume(v: Option<f32>) -> String {
    v.map(|v| format!("{:.0}%", v * 100.0)).unwrap_or_else(|| "n/a".into())
}

fn print_table(api: &str, devices: &[Device]) {
    println!("Audio API: {api}\n");
    for dir in Direction::ALL {
        let list: Vec<&Device> = devices.iter().filter(|d| d.direction == dir).collect();
        println!("{} ({}):", if dir == Direction::Input { "INPUTS" } else { "OUTPUTS" }, list.len());
        for d in list {
            let mut flags = Vec::new();
            if d.muted == Some(true) {
                flags.push("muted");
            }
            if d.hands_free {
                flags.push("hands-free");
            }
            println!(
                "  {} {:<40} {:>5}  {:<11} {}",
                if d.is_default { "*" } else { " " },
                truncate(&d.name, 40),
                fmt_volume(d.volume),
                d.transport.label(),
                flags.join(", ")
            );
            println!("      id: {}", d.id);
        }
        println!();
    }
    println!("* = system default");
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "…" }
}

fn run_meter(mock: bool, device: Option<String>, direction: Direction, seconds: Option<f32>) -> Result<()> {
    let mut b = make_backend(mock)?;
    let devices = b.devices()?;
    let d = match &device {
        Some(q) => resolve_device(&devices, direction, q)?.clone(),
        None => devices
            .iter()
            .find(|d| d.direction == direction && d.is_default)
            .cloned()
            .with_context(|| format!("no default {direction} device"))?,
    };
    let key = d.key();
    let native = b.has_native_meter(direction);
    let mut engine = MeterEngine::new();
    let native_cell = LevelCell::default();
    if !native {
        let cap = b
            .capture_id(&d.id, direction)
            .with_context(|| format!("metering {direction} devices is not supported on this platform"))?;
        let wanted = HashMap::from([(key.clone(), cap)]);
        engine.sync(&wanted);
        engine.wait_ready(&key, &wanted, Duration::from_secs(10)).with_context(|| format!("cannot open {}", d.name))?;
    }
    eprintln!("Metering \"{}\" ({direction}). Ctrl+C to stop.", d.name);
    let start = Instant::now();
    let mut last = start;
    let mut state = MeterState::new(start);
    let floor = -60.0;
    let width = 40usize;
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let now = Instant::now();
        let reading = if native {
            if let Some(p) = b.native_peak(&d.id, direction) {
                native_cell.push_levels(p, p);
            }
            native_cell.take()
        } else {
            engine.cell(&key).map(|c| c.take()).unwrap_or_default()
        };
        state.update(reading, now - last, now);
        last = now;
        let filled = (meter_position(state.level_db, floor) * width as f32).round() as usize;
        let hold = (meter_position(state.peak_hold_db, floor) * width as f32).round() as usize;
        let bar: String = (0..width)
            .map(|i| {
                if i < filled {
                    '█'
                } else if i + 1 == hold.max(1) && hold > filled {
                    '|'
                } else {
                    '·'
                }
            })
            .collect();
        let status = if native { "" } else { state.status.label() };
        print!(
            "\r[{bar}] {:>6.1} dBFS peak {:>6.1} {:<16}{}",
            state.level_db,
            state.peak_hold_db,
            status,
            if state.clipped { " CLIP" } else { "     " }
        );
        let _ = std::io::stdout().flush();
        if let Some(s) = seconds
            && start.elapsed().as_secs_f32() >= s
        {
            println!();
            // Machine-readable summary (used by the CI smoke tests).
            println!(
                "summary: device=\"{}\" max_db={:.1} status={:?} blocks={}",
                d.name, state.max_db, state.status, reading.blocks
            );
            if !native && state.status == SignalStatus::Lost {
                anyhow::bail!("no audio received from {}", d.name);
            }
            return Ok(());
        }
    }
}

/// With `windows_subsystem = "windows"` we have no console. If launched from
/// a terminal, attach to it so CLI output is visible.
#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console() {
    unsafe extern "system" {
        fn AttachConsole(pid: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
