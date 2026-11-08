# Changelog

Notable changes to this project are recorded here.
The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- Project scaffold: Cargo project, MIT license, build plan (`PLAN.md`).
- Core types (`Device`, `Direction`, `Transport`, `DeviceRef`) and CLI-style device resolution.
- Level math: peak, RMS, dBFS, lock-free `LevelCell`, and `MeterState` ballistics with
  signal-health detection (OK / digital silence / signal lost).
- Bluetooth hands-free (HFP/HSP) detection heuristics.
- `AudioBackend` trait, plus a mock backend for tests.
- macOS CoreAudio backend: list devices, read/set the default input/output, volume, mute,
  transport type, and hands-free detection.
