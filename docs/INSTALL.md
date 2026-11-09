# Installing Audio Interface

Audio Interface is a single, self-contained program. **None of the options below need
administrator / root rights**, unless a section says otherwise.

- [Quick start: download and run](#quick-start-download-and-run)
- [Windows](#windows)
  - [Portable zip (recommended, no admin)](#portable-zip-recommended-no-admin)
  - [Build from source without admin](#build-from-source-on-windows-without-admin)
  - [What about WSL?](#what-about-wsl)
- [Linux](#linux)
  - [Portable tar.gz (recommended, no sudo)](#portable-targz-recommended-no-sudo)
  - [Build from source without sudo](#build-from-source-on-linux-without-sudo)
- [macOS](#macos)
- [Corporate / locked-down machines](#corporate--locked-down-machines)

Pre-built downloads are attached to each
[GitHub release](https://github.com/ikjbi/audioInterface/releases) (the links above always point to the newest one).

---

## Quick start: download and run

1. Download the file for your computer:
   - **Windows:** [audio-interface-windows-x86_64.zip](https://github.com/ikjbi/audioInterface/releases/latest/download/audio-interface-windows-x86_64.zip)
   - **macOS:** [audio-interface-macos-universal.zip](https://github.com/ikjbi/audioInterface/releases/latest/download/audio-interface-macos-universal.zip)
   - **Linux:** [audio-interface-linux-x86_64.tar.gz](https://github.com/ikjbi/audioInterface/releases/latest/download/audio-interface-linux-x86_64.tar.gz)
2. Follow the steps for your OS below. The app is not code-signed yet, so the first launch needs
   one extra click:
   - **Windows:** "Windows protected your PC" → **More info** → **Run anyway**.
   - **macOS:** "Audio Interface can't be opened / Apple could not verify…" → click **Done**, open
     **System Settings → Privacy & Security**, scroll down and click **Open Anyway** next to *Audio
     Interface*, then confirm. (On macOS 14 and older you can also right-click the app → **Open**.)
     Terminal alternative: `xattr -dr com.apple.quarantine "/Applications/Audio Interface.app"`.
   - **Linux:** no prompt.
3. Allow microphone access when asked. It's needed for level meters and recordings.

---

## Windows

Windows 10 or 11, 64-bit.

### Portable zip (recommended, no admin)

1. Download `audio-interface-windows-x86_64.zip` from the releases page.
2. Extract it to a folder you own, e.g. `%LOCALAPPDATA%\Programs\AudioInterface`
   (paste that path into the File Explorer address bar). A USB stick works too.
3. Run `audio-interface.exe`.
   - **SmartScreen** may say *"Windows protected your PC"* because the app is not code-signed.
     Click **More info → Run anyway**. This prompt does not need admin rights.
4. Optional: run **`Install shortcuts.cmd`** from the same folder. It adds a Start-menu
   entry and, if you choose, starts the app at login. It only touches your own profile
   (`%APPDATA%\Microsoft\Windows\Start Menu`).

Level meters and recordings need microphone access:
*Settings → Privacy & security → Microphone → "Let desktop apps access your microphone"*.
This is a per-user setting. The app's **Open privacy settings** button takes you there.

**Uninstall:** delete the folder and the shortcuts. Settings are in
`%APPDATA%\ikjbi\AudioInterface`, recordings in `Documents\Audio Interface Recordings`.

### Build from source on Windows without admin

Rust installs per user. The usual Rust setup needs the *Visual Studio Build Tools*, and
installing those needs admin rights. The **GNU toolchain** ships its own linker, so it
doesn't:

```powershell
# 1. Install Rust for your user only (no admin). Download rustup-init.exe from https://rustup.rs,
#    then in PowerShell:
.\rustup-init.exe -y --default-host x86_64-pc-windows-gnu --default-toolchain stable

# 2. Open a NEW terminal, then get the code (or download the ZIP from GitHub, if git isn't available):
git clone https://github.com/ikjbi/audioInterface
cd audioInterface

# 3. Build
cargo build --release
.\target\release\audio-interface.exe
```

If you already have the Visual Studio Build Tools (MSVC), the default toolchain works
as well. The MSVC build also embeds the app icon in the `.exe`.

Copy `target\release\audio-interface.exe` anywhere. It is portable.

### What about WSL?

**WSL is not a workaround for this app.** Linux under WSL cannot see your Windows audio
devices (your headset, USB mic, speakers). With WSLg you only get one virtual
PulseAudio device that forwards to whatever Windows currently has as default. The app
would run, but it could not list your real devices, change the Windows default, adjust
Windows mic gain, or detect Teams using the mic. Use the portable Windows build instead.

---

## Linux

Any modern desktop distribution with PulseAudio or PipeWire (Ubuntu 22.04+, Fedora 36+,
Debian 12+, Arch, …). PipeWire works through its PulseAudio compatibility, which is on
by default.

### Portable tar.gz (recommended, no sudo)

```sh
tar xzf audio-interface-linux-x86_64.tar.gz
cd audio-interface-linux-x86_64
./install-user.sh              # installs to ~/.local/bin and adds a menu entry
./install-user.sh --autostart  # optional: start at login
```

Or just run `./audio-interface` straight from the extracted folder.

`install-user.sh --uninstall` removes it again. Nothing is written outside your home
directory.

Runtime requirements are already present on desktop systems: `libasound.so.2` (ALSA
library), X11 or Wayland, and a running PulseAudio/PipeWire session.

### Build from source on Linux without sudo

```sh
# 1. Rust, per user:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"

# 2. Get the code
git clone https://github.com/ikjbi/audioInterface && cd audioInterface

# 3. Build
cargo build --release
./target/release/audio-interface
```

The build needs the ALSA development files (`alsa.pc` + headers). If `cargo build` fails
with *"The system library `alsa` required by crate `alsa-sys` was not found"* and you can't
`sudo apt install libasound2-dev`, unpack the package into your home directory instead:

```sh
# Debian / Ubuntu
mkdir -p ~/.local/alsa && cd ~/.local/alsa
apt-get download libasound2-dev          # downloading does not need sudo
dpkg -x libasound2-dev_*.deb root
cd -
export PKG_CONFIG_PATH="$HOME/.local/alsa/root/usr/lib/x86_64-linux-gnu/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR=/           # paths inside alsa.pc point at the system
# libasound.so.2 itself is already installed; give the linker the unversioned name:
mkdir -p ~/.local/alsa/lib && ln -sf /usr/lib/x86_64-linux-gnu/libasound.so.2 ~/.local/alsa/lib/libasound.so
export RUSTFLAGS="-L $HOME/.local/alsa/lib"
cargo build --release
```

On Fedora, `dnf download alsa-lib-devel` + `rpm2cpio alsa-lib-devel-*.rpm | cpio -idm` does
the same.

Then install it for your user with
`packaging/linux/install-user.sh` (copy `target/release/audio-interface` and
`assets/icon.png` next to the script first), or just run the binary.

---

## macOS

macOS 12 or newer (macOS 14+ for "which app is using the mic"). Works on Apple Silicon and Intel.

### Download (recommended, no admin)

1. Download [audio-interface-macos-universal.zip](https://github.com/ikjbi/audioInterface/releases/latest/download/audio-interface-macos-universal.zip) and
   double-click it to unzip.
2. Drag **Audio Interface** into **Applications**. Your own `~/Applications` folder works too,
   without admin rights.
3. Open it. The first time, macOS blocks it because it's not notarized: open **System Settings →
   Privacy & Security** and click **Open Anyway** (see Quick start above).
4. Allow microphone access when asked. It's needed for meters and recordings. If you clicked
   "Don't Allow", turn it on under **System Settings → Privacy & Security → Microphone**.

### Build from source

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
git clone https://github.com/ikjbi/audioInterface && cd audioInterface
cargo build --release
scripts/bundle-macos.sh target/release/audio-interface 0.1.0 dist   # makes "Audio Interface.app"
```

Drag `dist/Audio Interface.app` to `~/Applications` (your own Applications folder, no admin).
The bundle is signed ad hoc, so the first launch needs **right-click → Open**. Allow
microphone access when asked. It's needed for meters and recordings.

---

## Corporate / locked-down machines

- **AppLocker / WDAC** policies may block running programs from your profile folder.
  If so, ask IT to allow it or to install it centrally. There is nothing the app can do
  about that.
- **Microphone privacy** may be disabled by policy. The app still lists devices, switches
  defaults and locks gain, but level meters and recordings need mic access.
- Switching the default device and changing volumes are per-user actions on all three
  OSes and do not need admin rights.
