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
- [ ] Install the Rust toolchain
- [ ] Cargo project, `.gitignore`, MIT license, README
- [ ] This plan file and a CHANGELOG
- [ ] Public GitHub repo `ikjbi/audioInterface`, first push

## Phase 1: Core abstractions (platform independent)
- [ ] `AudioBackend` trait: list devices, set default, get/set volume and mute
- [ ] Shared types: `Device`, `Direction`, `Transport` (Bluetooth / USB / built-in / virtual)
- [ ] Level math (peak, RMS, dBFS) with unit tests
- [ ] Bluetooth "hands-free mode" heuristics with unit tests

## Phase 2: Platform backends
- [ ] macOS: CoreAudio (devices, default get/set, volume, mute, transport type)
- [ ] Windows: WASAPI / MMDevice (devices, `IPolicyConfig` default set for all roles, endpoint volume, mute, output peak meter)
- [ ] Linux: PulseAudio / PipeWire-Pulse (sinks, sources, default get/set, volume, mute, bus type)

## Phase 3: Live level metering
- [ ] Metering engine: background capture thread per monitored input, lock-free level hand-off
- [ ] macOS + Windows capture via `cpal`
- [ ] Linux capture via PulseAudio (record stream from the specific source)
- [ ] Windows output meters via `IAudioMeterInformation`

## Phase 4: CLI (scriptable, also used for headless CI smoke tests)
- [ ] `audio-interface list` (table or `--json`)
- [ ] `audio-interface set-default <input|output> <name-or-id>`
- [ ] `audio-interface volume <input|output> <name-or-id> <0-100>`
- [ ] `audio-interface meter <name-or-id>` (terminal level meter)

## Phase 5: GUI (egui / eframe)
- [ ] Device list split into Inputs / Outputs, with the default marked
- [ ] "Make default" button for each device
- [ ] Volume slider + mute toggle for each device
- [ ] Live level meter with peak hold, plus a dB readout for inputs
- [ ] Bluetooth hands-free warning badge
- [ ] Auto-refresh when devices appear or disappear

## Phase 6: Smart features
- [ ] Persistent settings (JSON in the OS config dir)
- [ ] Gain lock: put the input volume back if something else changes it
- [ ] Auto-switch rules: preferred input/output priority lists
- [ ] "Signal lost" alert when a monitored mic goes silent

## Phase 7: CI / cross-platform verification
- [ ] GitHub Actions: build + clippy + tests on ubuntu, windows, macos
- [ ] Linux smoke test against a real PulseAudio server with a null sink (list, set-default, volume)
- [ ] Windows and macOS smoke test of the CLI (headless runners)
- [ ] Release workflow: tagged builds upload binaries for all 3 OSes

## Phase 8: Polish (stretch)
- [ ] System tray / menu bar icon
- [ ] App icon and bundling (`.app`, `.msi`/`.exe`, AppImage/deb)
- [ ] Event-driven device notifications instead of polling

---

## Progress log
