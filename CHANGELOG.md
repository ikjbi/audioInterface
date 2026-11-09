# Changelog

Notable changes to this project are recorded here.
The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

## [0.1.3] - 2026-11-09

### Added
- **Check for updates** (Settings → Updates, or `audio-interface update`): finds the newest
  release on GitHub and, with "Update and restart", installs it where the app already is and
  restarts. No admin rights needed. It only contacts GitHub when you press the button.

### Fixed
- **Much lower CPU/GPU use** (reported as loud fans on Windows), from a code review
  (`.code-review/REVIEW_LOG.md`, findings F-0001…F-0012):
  - the window redraws quickly only while a meter is actually moving; in silence it drops to
    2 redraws a second. On Windows every output has a meter, so before this the window redrew
    30 times a second whenever it was focused, even with nothing playing;
  - meters that only feed the display are closed 10 s after the window is hidden or minimized.
    An open mic kept Windows' audio engine and the headset's effects (noise suppression, Sonar)
    running all the time. Recordings keep their meter;
  - Windows: device objects are kept between refreshes, and audio sessions are listed once per
    refresh instead of twice;
  - dragging a slider no longer re-lists all devices about 30 times a second;
  - the session chart draws at most one bar per pixel (an 8-hour recording drew ~29,000 bars
    per frame);
  - smaller savings: the device snapshot is copied only when it changes, the noise floor is
    computed once per frame, the event log lays out only visible rows, only the selected
    recording is kept in memory, and settings are saved at most once a second.
- Hotkeys that couldn't be registered were retried every frame (and their error flickered).
- The spectrum analyzer's fall-off no longer depends on the frame rate.
- The Settings window scrolls when it is taller than the screen.

## [0.1.2] - 2026-11-09

### Fixed
- **Input cards no longer flicker in size** while levels change. The level stats always use four
  lines, and all warnings share one fixed-height status line (full text on hover).
- **"■ Hide level"** on every live meter. Previously only meters you'd started yourself could be
  stopped, so the default input's meter could never be hidden. Press "▶ Show level" to bring it back.

### Added
- **Quiet mics are visible and analysable** (reported with an analog construction headset on a
  laptop headset jack, whose voice sat around -60 dBFS):
  - meters, history and spectrum auto-scale down to 10 dB below each device's own noise floor;
  - speech detection (the live "typical" level and recording analysis) is relative to the noise
    floor instead of a fixed -55 dBFS, so a quiet mic no longer reads as "no speech";
  - a weak-signal warning when your speaking level is below -50 dBFS, with a button to open the
    sound settings (Windows: Recording tab, for Levels and Microphone Boost);
  - recording reports flag a very weak speaking level as a likely reason colleagues can't hear you.

### Changed
- Release files include the version in their name, e.g. `audio-interface-0.1.2-windows-x86_64.zip`.
  The README links to the latest release page instead of fixed file names.

## [0.1.1] - 2026-11-09

### Fixed
- **Much lower CPU and GPU use** (reported as loud fans on Windows). The window now redraws at 30
  fps only while it is focused and meters are moving, 5 fps in the background, 2 fps when nothing
  animates, and not at all when minimized, hidden or covered. It also uses the low-power
  (integrated) GPU and a low-latency surface instead of eframe's default high-performance GPU,
  which woke laptops' discrete GPUs. Measured on a MacBook: about 11% CPU before, 1–3% after.
- **Profiles: the name field closed the menu**, so "Save current" could never be enabled.
  Profiles is now a small window.

### Added
- Recording reports show when apps start or stop using the device, and explain a gain change
  that happens right as an app (e.g. Teams) leaves or joins the mic: "2 s after Microsoft Teams
  stopped using the device: most likely the app restoring the mic level it had before the call".
  Such changes are no longer counted as problems. Past recordings get this when reopened.

### Changed
- Download links point at the latest release (`releases/latest/download/…`).

## [0.1.0-alpha.1] - 2026-11-09

First public pre-release.

### Added
- **In-app help**: a ❓ Help window explaining every feature in plain language, plus "?" buttons next
  to controls (gain lock, meters, record, profiles, auto-record, hotkeys) that open the right topic.
- **Download section** in the README and quick-start steps in `docs/INSTALL.md`, including how to
  get past SmartScreen and Gatekeeper for the unsigned app.
- **Per-app mixer** (Windows, Linux): an "Apps" section on each device with per-application volume
  and mute; CLI `apps`, `app-volume`, `app-mute`. macOS has no public API for it.
- **Loudness (EBU R128)**: momentary (400 ms) and short-term (3 s) LUFS for metered inputs, using
  BS.1770 K-weighting computed in the capture callback (verified against the EBU 1 kHz reference).
- **Single instance**: launching the app again brings the running window to the front.
- **Diagnostics**: "Copy diagnostics" (Settings → Support) and `audio-interface diagnostics`.
- **Settings export/import**: `settings export|import|path`, plus export and open-folder buttons.
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
- Test tones and mic-test playback could be cut off when the device requested a large buffer up
  front (PulseAudio): the stream now lives until the delivered audio has played. Found by Linux CI.
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
