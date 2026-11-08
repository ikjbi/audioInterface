//! "Start at login", per user, without admin rights:
//! * Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
//! * macOS: `~/Library/LaunchAgents/com.ikjbi.AudioInterface.plist`
//! * Linux: `~/.config/autostart/com.ikjbi.AudioInterface.desktop` (XDG autostart)
//!
//! The app is started with `--minimized`.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const LABEL: &str = "com.ikjbi.AudioInterface";

fn exe() -> Result<PathBuf> {
    std::env::current_exe().context("locate the running executable")
}

/// launchd agent that starts the app at login.
pub fn macos_plist(exe: &Path) -> String {
    let exe = xml_escape(&exe.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string><string>--minimized</string></array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#
    )
}

/// XDG autostart entry.
pub fn linux_desktop(exe: &Path) -> String {
    let exe = exe.display().to_string().replace('"', "\\\"");
    format!(
        "[Desktop Entry]\nType=Application\nName=Audio Interface\nExec=\"{exe}\" --minimized\nIcon=audio-interface\n\
         X-GNOME-Autostart-enabled=true\nTerminal=false\n"
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn entry_path() -> Option<PathBuf> {
    let home = directories::BaseDirs::new()?;
    #[cfg(target_os = "macos")]
    return Some(home.home_dir().join("Library/LaunchAgents").join(format!("{LABEL}.plist")));
    #[cfg(target_os = "linux")]
    return Some(home.config_dir().join("autostart").join(format!("{LABEL}.desktop")));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn is_enabled() -> bool {
    entry_path().is_some_and(|p| p.exists())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn set_enabled(on: bool) -> Result<()> {
    let path = entry_path().context("no home directory")?;
    if !on {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    #[cfg(target_os = "macos")]
    let content = macos_plist(&exe()?);
    #[cfg(target_os = "linux")]
    let content = linux_desktop(&exe()?);
    std::fs::write(&path, content).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
fn reg(args: &[&str]) -> Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Ok(std::process::Command::new("reg").args(args).creation_flags(CREATE_NO_WINDOW).output()?)
}

#[cfg(windows)]
pub fn is_enabled() -> bool {
    reg(&["query", RUN_KEY, "/v", "AudioInterface"]).is_ok_and(|o| o.status.success())
}

#[cfg(windows)]
pub fn set_enabled(on: bool) -> Result<()> {
    let out = if on {
        let cmd = format!("\"{}\" --minimized", exe()?.display());
        reg(&["add", RUN_KEY, "/v", "AudioInterface", "/t", "REG_SZ", "/d", &cmd, "/f"])?
    } else {
        if !is_enabled() {
            return Ok(());
        }
        reg(&["delete", RUN_KEY, "/v", "AudioInterface", "/f"])?
    };
    anyhow::ensure!(out.status.success(), "reg.exe failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn is_enabled() -> bool {
    false
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn set_enabled(_on: bool) -> Result<()> {
    anyhow::bail!("start at login is not supported on this OS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_reference_the_executable() {
        let exe = Path::new("/Applications/Audio Interface.app/Contents/MacOS/audio-interface");
        let plist = macos_plist(exe);
        assert!(plist.contains("<string>/Applications/Audio Interface.app/Contents/MacOS/audio-interface</string>"));
        assert!(plist.contains("<string>--minimized</string>"));
        assert!(plist.contains(LABEL));
        let d = linux_desktop(Path::new("/home/me/.local/bin/audio-interface"));
        assert!(d.contains("Exec=\"/home/me/.local/bin/audio-interface\" --minimized"));
        assert!(macos_plist(Path::new("/a&b")).contains("/a&amp;b"));
    }
}
