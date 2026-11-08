# Changelog

Notable changes to this project are recorded here.
The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Fixed
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
