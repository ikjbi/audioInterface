# Audio Interface: Build Plan

A cross-platform (Windows / macOS / Linux) desktop app written in Rust. It sits
between your audio devices and the OS sound settings. You can:

- see every input and output device and which one is the system default
- set the default input and output device with one click
- watch live level meters, for example to check that a Bluetooth headset mic is working
- watch and **lock** input gain, so apps like Teams or Zoom can't quietly lower it
- set up auto-switch rules ("when my headset connects, make it the default")

Status key: `[x]` done · `[~]` partially done / see notes · `[ ]` not started

---

## Phase 0: Project setup
- [x] Install the Rust toolchain
- [x] Cargo project, `.gitignore`, MIT license, README
- [x] This plan file and a CHANGELOG
- [x] Public GitHub repo `ikjbi/audioInterface`, first push

## Phase 1: Core abstractions (platform independent)
- [x] `AudioBackend` trait: list devices, set default, get/set volume and mute
- [x] Shared types: `Device`, `Direction`, `Transport` (Bluetooth / USB / built-in / virtual)
- [x] Level math (peak, RMS, dBFS) with unit tests
- [x] Bluetooth "hands-free mode" heuristics with unit tests

## Phase 2: Platform backends
- [x] macOS: CoreAudio (devices, default get/set, volume, mute, transport type)
- [x] Windows: WASAPI / MMDevice (devices, `IPolicyConfig` default set for all roles, endpoint volume, mute, output peak meter) (verified in CI on a Windows runner with a VB-CABLE virtual sound card)
- [x] Linux: PulseAudio / PipeWire-Pulse (sinks, sources, default get/set, volume, mute, bus type) (verified in CI on a real PulseAudio server)

## Phase 3: Live level metering
- [x] Metering engine: background capture thread per monitored input, lock-free level hand-off
- [x] macOS + Windows capture via `cpal` (verified on CI runners: CoreAudio and WASAPI streams deliver audio; locally the Mac needs mic permission granted)
- [x] Linux capture via PulseAudio (record stream from the specific source) (via cpal's PulseAudio host; CI measures a 0.5-amplitude tone at exactly -6.0 dBFS)
- [x] Windows output meters via `IAudioMeterInformation` (verified on a Windows runner)

## Phase 4: CLI (scriptable, also used for headless CI smoke tests)
- [x] `audio-interface list` (table or `--json`)
- [x] `audio-interface set-default <input|output> <name-or-id>`
- [x] `audio-interface volume <input|output> <name-or-id> <0-100>`
- [x] `audio-interface meter <name-or-id>` (terminal level meter)

## Phase 5: GUI (egui / eframe)
- [x] Device list split into Inputs / Outputs, with the default marked
- [x] "Make default" button for each device
- [x] Volume slider + mute toggle for each device
- [x] Live level meter with peak hold, plus a dB readout for inputs
- [x] Bluetooth hands-free warning badge
- [x] Auto-refresh when devices appear or disappear

## Phase 6: Smart features
- [x] Persistent settings (JSON in the OS config dir)
- [x] Gain lock: put the input volume back if something else changes it
- [x] Auto-switch rules: preferred input/output priority lists
- [x] "Signal lost" alert when a monitored mic goes silent

## Phase 7: CI / cross-platform verification
- [x] GitHub Actions: build + clippy + tests on ubuntu, windows, macos
- [x] Linux smoke test against a real PulseAudio server with a null sink (list, set-default, volume)
- [x] Windows and macOS smoke test of the CLI (real devices: VB-CABLE on Windows, Background Music on macOS)
- [~] Release workflow: tagged builds upload binaries for all 3 OSes (written, not run yet: no tag pushed, publishing a release is left for the owner to decide)

## Phase 8: Polish (stretch)
- [x] 30 s level history graph + typical-level readout (gain drift at a glance)
- [ ] System tray / menu bar icon
- [x] App icon and bundling (new icon embedded in the Windows .exe; macOS .app; portable Windows zip and Linux tar.gz with user-level install scripts)
- [x] Animated splash screen
- [x] No-admin install guide (docs/INSTALL.md), with both sudo-free build paths tested in CI
- [ ] Event-driven device notifications instead of polling

## Phase 9: Session recording and drop analysis
Goal: find out if levels or gain drop halfway through a Teams call, and why.
- [x] Multi-consumer level cells (UI and recorder each see every peak)
- [x] Session recorder: CSV, one row per second (peak, RMS, OS gain, mute, signal status, hands-free, in-use apps, events)
- [x] Optional audio recording (WAV, 16 kHz mono) to listen back to the moment it dropped
- [x] Drop analysis: baseline speaking level, sliding-window drops, OS gain changes (e.g. Teams auto-adjust), signal loss and silence, with timestamps
- [x] CLI: `record` and `analyze`
- [x] GUI: record button, live recording indicator, recordings browser, report view with full-session chart

## Phase 10: Which app is using the mic + auto-record calls
- [x] Detect which other apps are using each device (macOS process objects, Windows audio sessions, PulseAudio source outputs)
- [x] Show "in use by Teams" in the GUI and CLI
- [x] Auto-record: start recording when another app starts using a chosen device (a call starts), stop when the call ends
- [~] CI: Linux test records a test tone, analyzes it, and detects a `parecord` client as "in use" (in-use + auto-record passed; level assertion re-running after the RMS fix)

## Phase 11: Professional feature set
Benchmarked against SoundSource (macOS), EarTrumpet / SoundSwitch / Voicemeeter (Windows) and
pavucontrol (Linux). Ordered by value. Platform limits are noted where an OS has no API.

**Everyday control**
- [x] 11.1 Profiles / presets: save defaults + volumes + gain locks as named profiles ("Calls", "Music") and switch in one click
- [ ] 11.2 Global hotkeys: toggle mic mute anywhere (push-to-mute), cycle output device
- [ ] 11.3 System tray / menu-bar icon: quick device switch, mic mute, recording status
- [ ] 11.4 Desktop notifications: device lost, gain changed by an app, signal lost, call recording saved
- [ ] 11.5 Start at login (per user: Windows HKCU Run, macOS LaunchAgent, Linux autostart)
- [ ] 11.6 Separate "communications" default (Windows has a separate default for calls)
- [ ] 11.7 Custom device names and hiding devices
- [ ] 11.8 Per-app volume and mute mixer (Windows sessions, PulseAudio streams; macOS has no public API, so it's shown read-only)

**Diagnostics & testing**
- [ ] 11.9 Mic test: record 5 s and play it back on the default output
- [ ] 11.10 Speaker test: left/right test tones to identify channels
- [ ] 11.11 Live mic monitoring ("hear yourself") with a volume control
- [ ] 11.12 Spectrum analyzer view for inputs
- [ ] 11.13 Noise floor, SNR estimate, clip counter per metered input
- [ ] 11.14 Device format info: sample rate, channels (and set the sample rate on macOS)
- [ ] 11.15 Channel balance (L/R) for outputs
- [ ] 11.16 Bluetooth profile switch on Linux (A2DP ↔ HFP)
- [ ] 11.17 Loudness meter (EBU R128 momentary/short-term LUFS)

**App quality**
- [ ] 11.18 Single instance (a second launch focuses the first)
- [ ] 11.19 Light/dark/system theme
- [ ] 11.20 Diagnostics bundle: copy system/device info + recent log for bug reports
- [ ] 11.21 Settings import/export

---

## Progress log

### 2026-11-08 (night)
- **22:35** Installed Rust 1.99 via rustup. Scaffolded the project. Created the public repo
  https://github.com/ikjbi/audioInterface.
- **22:50** Core types, level math and Bluetooth heuristics, with unit tests. CoreAudio backend
  tested on real hardware (MacBook Pro + AirPods Pro): list, set default, volume and mute
  all work. Every setting was restored after the test.
- **23:00** Windows (WASAPI + IPolicyConfig) and Linux (PulseAudio protocol) backends
  type-check and pass clippy from macOS via cross-target checks (`scripts/check-targets.sh`).
- **23:05** Metering engine, service thread, GUI and CLI. Found that opening the mic
  **blocks on macOS's microphone permission prompt**. Moved stream opening onto helper
  threads, so the app never freezes and shows a "check microphone permission" hint instead.
- **23:10** GUI verified visually in mock mode (`docs/screenshot.png`). Auto-switch and gain
  lock work end to end. CI pipeline added.
- **23:25** First CI run is green on all three OSes. The Linux PulseAudio integration test
  passes: defaults, volume, mute, and meters reading test tones at exactly -6.0 dBFS.
- **23:35** `scripts/device-smoke.py` passes on the real Mac (3 inputs, 2 outputs), and device
  state afterwards is identical to before. Added a real-device CI job for Windows (VB-CABLE)
  and macOS. Added the release workflow, macOS `.app` bundling, app icon and level-history graph.
- **23:55** Windows backend verified on a real Windows runner: device list, volume
  (37% read back as 0.370), mute and `IPolicyConfig` default switching all pass, and state is
  restored. WASAPI capture failed with `E_ACCESSDENIED` because the runner has microphone
  privacy turned off. The app now explains this error and links to the privacy settings, and
  CI grants access through the registry before the meter test. Fixed a gain-lock loop on
  devices with stepped volume.
- **00:15** All six CI jobs are green. Windows WASAPI capture works once mic consent is granted
  (303 blocks, digital silence correctly detected), and the native output meter works. macOS runner
  capture works (286 blocks). Every backend and capture path is verified on all three OSes.

### 2026-11-09
- **00:05** Added "which app is using the device" detection on all three OSes. On macOS,
  `afplay` was detected playing to the speakers (the output was muted during the test).
- **00:10** Session recorder, WAV tap and drop analysis, with unit tests. CLI `record`,
  `record --auto` and `analyze`. Recordings window with the session chart.
- **00:15** Linux CI found that recorded RMS was diluted by service ticks with no new audio
  block. Fixed with exact per-sample energy accumulation. New icon, Windows exe icon, splash
  screen, and the no-admin install guide with CI jobs that prove the sudo-free build paths.
