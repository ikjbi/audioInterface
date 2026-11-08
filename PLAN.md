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
- [~] Windows: WASAPI / MMDevice (devices, `IPolicyConfig` default set for all roles, endpoint volume, mute, output peak meter) (written and type-checked; runtime verified by CI only (no Windows machine available))
- [x] Linux: PulseAudio / PipeWire-Pulse (sinks, sources, default get/set, volume, mute, bus type) (verified in CI on a real PulseAudio server)

## Phase 3: Live level metering
- [x] Metering engine: background capture thread per monitored input, lock-free level hand-off
- [~] macOS + Windows capture via `cpal` (macOS capture blocked locally on the mic permission prompt (see notes))
- [x] Linux capture via PulseAudio (record stream from the specific source) (via cpal's PulseAudio host; CI measures a 0.5-amplitude tone at exactly -6.0 dBFS)
- [~] Windows output meters via `IAudioMeterInformation` (implemented, not verified on real hardware)

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
- [~] Windows and macOS smoke test of the CLI (headless runners) (mock backend only for now)
- [x] Release workflow: tagged builds upload binaries for all 3 OSes

## Phase 8: Polish (stretch)
- [x] 30 s level history graph + typical-level readout (gain drift at a glance)
- [ ] System tray / menu bar icon
- [~] App icon and bundling (`.app`, `.msi`/`.exe`, AppImage/deb) (icon on all OSes + macOS `.app` done; Windows/Linux ship as plain binaries)
- [ ] Event-driven device notifications instead of polling

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
