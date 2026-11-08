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
    match cli.command.unwrap_or(Cmd::Gui) {
        Cmd::Gui => {
            let path = Settings::default_path();
            let settings = path.as_deref().map(Settings::load).unwrap_or_default();
            let factory: BackendFactory = if mock {
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
            };
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
