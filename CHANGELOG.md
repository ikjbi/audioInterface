# Changelog

Notable changes to this project are recorded here.
The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- **Theme**: system, light or dark (Settings → General). Chart colors adapt to the theme.
- **Card profiles on Linux**, e.g. switch a Bluetooth headset between A2DP (high quality) and
  headset/HFP (mic): device menu → Profile, or `card-profile <dir> <device> [profile]`.
- **Left/right balance** for stereo outputs with per-channel volume (Windows, macOS, Linux): slider
  on the device card, `balance <device> <-1..1>` in the CLI.
- **Device format**: sample rate and channel count on every device (GUI, `list`, JSON). On macOS
  the sample rate can be changed (device menu, `sample-rate` command).
- **Spectrum analyzer** (📊 on capture-metered devices): 48 log-spaced bands from 20 Hz to 20 kHz,
  Hann-windowed FFT.
- **Noise floor, SNR and clip counter** under each metered input.
- **Test tools** (right-click a device): speaker test tone (left/right/both), mic test (record 5 s and
  play back), and live mic monitoring with a volume control. CLI: `test-tone`, `mic-test`.
- **Global hotkeys** (configurable, Settings → Global hotkeys): Ctrl+Alt+M mutes/unmutes the mic,
  Ctrl+Alt+O / Ctrl+Alt+I switch to the next output/input (preferred order, hidden skipped), with
  a notification. Linux: X11 sessions.
- **Tray / menu-bar icon** (Windows, macOS): switch input/output, mute the mic, apply profiles, start
  or stop recording the default input, show the window or quit. Closing the window keeps the app in
  the tray (optional).
- **Desktop notifications** (Settings → General) and **"mic gain lowered by another app"**
  detection: a 10+ point drop on an input that you didn't make is logged (also into recordings),
  with the app using the mic, and notified. Also notifies on gain-lock restores, default-device
  disconnects, hands-free switches, and call recordings starting or being saved.
- **Windows communications device**: "CALLS" badge, "Use for calls only" in the device menu and
  `set-default --comms`, to send Teams/Zoom to a headset while media stays on speakers.
- **Rename and hide devices**: right-click (or double-click) a device name; hidden devices can be
  restored in Settings.
- **Start at login** (Settings → General): per user on all OSes (HKCU Run key, LaunchAgent, XDG
  autostart), launching with the new `--minimized` flag.
- **Profiles**: save default devices, volumes, mutes and gain locks as named setups ("Calls",
  "Music") and switch in one click (top-bar menu) or with `audio-interface profile apply <name>`.
- **Session recording**: ⏺ Record on any device logs level (peak and exact RMS), OS gain, mute,
  signal status, Bluetooth hands-free mode, apps using the device, and events once per second to
  CSV, with optional 16 kHz mono WAV audio. Files go to `Documents/Audio Interface Recordings`.
- **Drop analysis** (`src/analysis.rs`): baseline speaking level, 60 s sliding-window drop
  detection, OS gain changes (ramps merged), and attribution of each drop to a lowered OS gain
  (app auto-adjust) or to an acoustic/hardware cause. Also reports signal-lost, digital-silence
  and muted periods and hands-free switches. Writes `.report.txt` and `.report.json`.
- **Which app is using a device** (macOS process objects, Windows audio sessions, PulseAudio
  streams) shown on each device card and in `list --json` (`in_use_by`).
- **Auto-record calls**: record any input another app starts using (optionally only some apps,
  e.g. "teams"), and stop 10 s after the call ends.
- Recordings window: live recording charts, auto-record settings, past sessions with verdict,
  findings and a full-session chart (level, speaking-level trend, OS gain, drops, signal loss).
- CLI: `record [--audio] [--seconds] [--out]`, `record --auto [--app NAME]`, `analyze <csv> [--json]`.
- New app icon (meter bars with a level gradient), embedded in the Windows `.exe`; `assets/icon.png`
  and `assets/icon.ico`.
- Animated splash screen (LED meters with peak hold, live start-up status).
- `docs/INSTALL.md`: no-admin installs for Windows (portable zip, GNU toolchain builds) and Linux
  (portable tar.gz with `install-user.sh`, sudo-free builds), plus why WSL is not an option. Both
  sudo-free build paths are tested in CI.

### Fixed
- Changing the volume keeps the left/right balance (macOS per-channel devices, Linux).
- The CLI no longer panics when its output is piped into `head` and similar (SIGPIPE).
- A meter blocked by OS microphone privacy (`Access is denied` on Windows) now explains why,
  and the GUI offers an "Open privacy settings" button (macOS and Windows).
- Gain lock no longer loops forever on devices that only accept volume in steps. When the
  device settles on a nearby value, that value is accepted and a warning is logged once.

### Added
- Project scaffold: Cargo project, MIT license, build plan (`PLAN.md`).
- Core types (`Device`, `Direction`, `Transport`, `DeviceRef`) and CLI-style device resolution.
- Level math: peak, RMS, dBFS, lock-free `LevelCell`, and `MeterState` ballistics with
  signal-health detection (OK / digital silence / signal lost).
- Bluetooth hands-free (HFP/HSP) detection heuristics.
- `AudioBackend` trait, plus a mock backend for tests.
- macOS CoreAudio backend: list devices, read/set the default input/output, volume, mute,
  transport type, and hands-free detection.
- Windows backend: MMDevice/WASAPI device list, default switching via `IPolicyConfig` (all three
  roles), endpoint volume and mute, Bluetooth/USB/HDMI detection from the endpoint properties, and
  native output peak meters (`IAudioMeterInformation`).
- Linux backend: PulseAudio native protocol over the Unix socket (pure Rust, works with
  PipeWire). Covers sinks and sources, set default, volume, mute, Bluetooth profile detection, and
  output metering through monitor sources. Reconnects when the sound server restarts.
- Meter engine on `cpal`. Capture streams open on helper threads, so a blocked open (for
  example macOS waiting on the microphone permission prompt) can't freeze the app.
- Background service: device polling, event log, gain lock enforcement, and auto-switching on
  connect/disconnect.
- Desktop GUI (egui): input/output cards, make-default, volume, mute, gain lock, preferred
  devices, live meters with peak hold, clip and signal-lost/digital-silence warnings, Bluetooth
  hands-free notices, settings window, and event log.
- CLI: `list [--json]`, `set-default`, `volume`, `mute`, `meter`, plus global `--mock` and
  `--settings <path>`.
- `AUDIO_INTERFACE_SCREENSHOT=<file>` renders the UI to a PNG and exits (docs and visual checks).
- GitHub Actions CI on Linux, Windows and macOS (fmt, clippy, tests, CLI smoke tests), plus a
  PulseAudio integration test with virtual devices and a 440 Hz test-tone source.
- `scripts/check-targets.sh` lints all three OS targets from one machine.
- 30-second level history graph under each metered input, with a "typical level" readout (average
  RMS while there is sound), so slow gain loss is easy to spot.
- App icon drawn in code (window/taskbar icon on every OS). A hidden `export-icon` command
  provides it for packaging.
- Release workflow: tag `v*` to build Linux (`.tar.gz`), Windows (`.zip`) and a universal
  macOS `.app` (with `NSMicrophoneUsageDescription` and an icon, ad-hoc signed).
- `scripts/device-smoke.py`: real-device test that changes volume, mute and defaults through the
  CLI, verifies each change through the OS, and restores everything. CI runs it on Windows
  (with a VB-CABLE virtual sound card) and macOS.
