//! "Check for updates": find the newest GitHub release and install it over
//! the running copy, without admin rights.
//!
//! Only runs when the user asks (button in Settings, or `audio-interface update`).
//! It talks to api.github.com and GitHub's download servers, nothing else.
//!
//! Installing replaces the program where it already is:
//! * Windows: a running `.exe` can't be overwritten but can be renamed, so the
//!   old one becomes `audio-interface.old.exe` (deleted on the next start).
//! * macOS: the whole `Audio Interface.app` bundle is swapped.
//! * Linux: the binary is swapped (Linux allows replacing a running file).
//!
//! Everything is downloaded into a staging folder next to the installed copy,
//! so the final swap is a same-folder rename. If the folder isn't writable
//! (e.g. installed by IT under Program Files), the error says so.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::time::Duration;

const LATEST_URL: &str = "https://api.github.com/repos/ikjbi/audioInterface/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/ikjbi/audioInterface/releases/latest";
const STAGING: &str = ".audio-interface-update";
/// Testing aid: pretend to be this version (e.g. `0.0.1`) so an update is offered.
const PRETEND_ENV: &str = "AUDIO_INTERFACE_PRETEND_VERSION";

/// A newer release with a download for this OS.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub page_url: String,
    pub asset_name: String,
    pub asset_url: String,
    pub size: u64,
}

/// The version of this program.
pub fn current_version() -> String {
    std::env::var(PRETEND_ENV).unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
}

/// End of the release file name for this OS, as made by the release workflow.
fn asset_suffix() -> Option<&'static str> {
    if cfg!(windows) {
        Some("-windows-x86_64.zip")
    } else if cfg!(target_os = "macos") {
        Some("-macos-universal.zip")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("-linux-x86_64.tar.gz")
    } else {
        None
    }
}

/// `(major, minor, patch, is_final)` from "v1.2.3" or "1.2.3-alpha.1".
fn parse_version(v: &str) -> Option<(u64, u64, u64, bool)> {
    let v = v.trim().trim_start_matches('v');
    let (core, pre) = match v.split_once('-') {
        Some((c, _)) => (c, true),
        None => (v, false),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0), !pre))
}

/// Whether `latest` is newer than `current`. A final release beats a
/// pre-release of the same number (0.2.0 > 0.2.0-alpha.1).
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// Pick the release and this OS's download from the GitHub API response.
fn parse_latest(json: &str, suffix: &str) -> Result<Release> {
    let v: serde_json::Value = serde_json::from_str(json).context("unexpected reply from GitHub")?;
    let tag = v["tag_name"].as_str().context("release has no tag")?;
    let asset = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(suffix)))
        .with_context(|| format!("release {tag} has no download for this computer (*{suffix})"))?;
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        page_url: v["html_url"].as_str().unwrap_or(RELEASES_PAGE).to_string(),
        asset_name: asset["name"].as_str().unwrap_or_default().to_string(),
        asset_url: asset["browser_download_url"].as_str().context("download has no URL")?.to_string(),
        size: asset["size"].as_u64().unwrap_or(0),
    })
}

#[cfg(not(target_os = "linux"))]
mod http {
    use super::*;
    use std::io::Read;

    fn agent() -> ureq::Agent {
        use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
        // The OS's own TLS and certificate store (which matters behind corporate proxies).
        ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build(),
            )
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(600)))
            .user_agent(concat!("audio-interface/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    }

    pub fn get_text(url: &str, headers: &[(&str, String)]) -> Result<String> {
        let mut req = agent().get(url);
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        let mut resp = req.call().map_err(|e| anyhow::anyhow!("could not reach GitHub: {e}"))?;
        resp.body_mut().read_to_string().context("reading the reply from GitHub")
    }

    pub fn download(url: &str, to: &Path, total: u64, progress: &dyn Fn(u64, u64)) -> Result<u64> {
        let mut resp = agent().get(url).call().map_err(|e| anyhow::anyhow!("download failed: {e}"))?;
        let mut reader = resp.body_mut().with_config().limit(1 << 30).reader();
        let mut file = std::fs::File::create(to).with_context(|| format!("create {}", to.display()))?;
        let mut buf = vec![0u8; 64 * 1024];
        let mut done = 0u64;
        loop {
            let n = reader.read(&mut buf).context("download interrupted")?;
            if n == 0 {
                return Ok(done);
            }
            std::io::Write::write_all(&mut file, &buf[..n]).context("writing the download")?;
            done += n as u64;
            progress(done, total);
        }
    }
}

/// Linux: curl or wget (one of them is on every desktop distribution). They use
/// the system's certificates and proxy settings, and need nothing at build time.
#[cfg(target_os = "linux")]
mod http {
    use super::*;
    use std::process::{Command, Stdio};

    fn tool() -> Result<&'static str> {
        ["curl", "wget"]
            .into_iter()
            .find(|t| Command::new(t).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok())
            .context("checking for updates needs curl or wget")
    }

    fn command(url: &str, out: &Path, headers: &[(&str, String)]) -> Result<Command> {
        let agent = concat!("audio-interface/", env!("CARGO_PKG_VERSION"));
        let tool = tool()?;
        let mut cmd = Command::new(tool);
        if tool == "curl" {
            cmd.args(["-fsSL", "--connect-timeout", "15", "-A", agent, "-o"]).arg(out);
            for (k, v) in headers {
                cmd.arg("-H").arg(format!("{k}: {v}"));
            }
        } else {
            cmd.args(["-q", "--timeout=30", "-U", agent, "-O"]).arg(out);
            for (k, v) in headers {
                cmd.arg(format!("--header={k}: {v}"));
            }
        }
        cmd.arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        Ok(cmd)
    }

    fn finish(child: std::process::Child, what: &str) -> Result<()> {
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!("{what} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    pub fn get_text(url: &str, headers: &[(&str, String)]) -> Result<String> {
        let tmp = std::env::temp_dir().join(format!("audio-interface-update-{}.json", std::process::id()));
        let child = command(url, &tmp, headers)?.spawn().context("start curl/wget")?;
        let r = finish(child, "could not reach GitHub").and_then(|_| Ok(std::fs::read_to_string(&tmp)?));
        let _ = std::fs::remove_file(&tmp);
        r
    }

    pub fn download(url: &str, to: &Path, total: u64, progress: &dyn Fn(u64, u64)) -> Result<u64> {
        let mut child = command(url, to, &[])?.spawn().context("start curl/wget")?;
        while child.try_wait()?.is_none() {
            progress(std::fs::metadata(to).map(|m| m.len()).unwrap_or(0), total);
            std::thread::sleep(Duration::from_millis(200));
        }
        finish(child, "download")?;
        let done = std::fs::metadata(to)?.len();
        progress(done, total);
        Ok(done)
    }
}

/// Ask GitHub for the latest release. `Ok(None)` means this is the newest version.
pub fn check() -> Result<Option<Release>> {
    let suffix = asset_suffix().context("there are no ready-made downloads for this OS / processor")?;
    let mut headers = vec![("Accept", "application/vnd.github+json".to_string())];
    // Only for CI, where many jobs share an IP address and its rate limit.
    if let Ok(token) = std::env::var("AUDIO_INTERFACE_GITHUB_TOKEN")
        && !token.is_empty()
    {
        headers.push(("Authorization", format!("Bearer {token}")));
    }
    let body = http::get_text(LATEST_URL, &headers)?;
    let release = parse_latest(&body, suffix)?;
    Ok(is_newer(&release.version, &current_version()).then_some(release))
}

/// What gets replaced: the `.exe` / binary, or on macOS the `.app` bundle.
fn install_target() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("cannot find the running program")?;
    let exe = exe.canonicalize().map(plain_path).unwrap_or(exe);
    let dir = exe.parent().context("program has no folder")?;
    if dir.ends_with("target/debug") || dir.ends_with("target/release") || dir.ends_with("target\\release") {
        bail!("this is a development build ({}); update it with git and cargo instead", exe.display());
    }
    if cfg!(target_os = "macos") {
        return exe
            .ancestors()
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .map(Path::to_path_buf)
            .context("updating needs the app bundle (Audio Interface.app); download the new version from the release page instead");
    }
    Ok(exe)
}

/// Windows: `canonicalize` gives `\\?\C:\...`; use the ordinary `C:\...` form
/// (it is what shortcuts point to, and what gets started again).
fn plain_path(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => p,
    }
}

/// Result of [`install`]: where the new version is, to restart it.
#[derive(Debug)]
pub struct Installed {
    pub version: String,
    pub path: PathBuf,
}

/// Download `release` and put it in place of the running copy.
/// `progress(done, total)` is called while downloading.
pub fn install(release: &Release, progress: &dyn Fn(u64, u64)) -> Result<Installed> {
    let target = install_target()?;
    let parent = target.parent().context("install folder")?.to_path_buf();
    let staging = parent.join(STAGING);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).with_context(|| {
        format!("cannot write to {} (no permission?); download the new version from the release page", parent.display())
    })?;
    let result = (|| {
        let archive = staging.join(&release.asset_name);
        download(release, &archive, progress)?;
        let new = unpack(&archive, &staging)?;
        swap(&new, &target)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result?;
    Ok(Installed { version: release.version.clone(), path: target })
}

fn download(release: &Release, to: &Path, progress: &dyn Fn(u64, u64)) -> Result<()> {
    let done = http::download(&release.asset_url, to, release.size, progress)?;
    if release.size > 0 && done != release.size {
        bail!("download incomplete ({done} of {} bytes)", release.size);
    }
    Ok(())
}

/// Extract the archive into `dir` and return the new program (or bundle).
fn unpack(archive: &Path, dir: &Path) -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?).context("open the downloaded zip")?;
        let mut entry = zip.by_name("audio-interface.exe").context("the zip has no audio-interface.exe")?;
        let out = dir.join("audio-interface.exe");
        std::io::copy(&mut entry, &mut std::fs::File::create(&out)?).context("unzip audio-interface.exe")?;
        Ok(out)
    }
    #[cfg(target_os = "macos")]
    {
        run_tool(std::process::Command::new("ditto").arg("-x").arg("-k").arg(archive).arg(dir))?;
        let app = dir.join("Audio Interface.app");
        if !app.join("Contents/MacOS/audio-interface").exists() {
            bail!("the download doesn't contain Audio Interface.app");
        }
        Ok(app)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        run_tool(std::process::Command::new("tar").arg("-xzf").arg(archive).arg("-C").arg(dir))?;
        std::fs::read_dir(dir)?
            .flatten()
            .map(|e| e.path().join("audio-interface"))
            .find(|p| p.is_file())
            .context("the download doesn't contain the audio-interface program")
    }
}

#[cfg(not(windows))]
fn run_tool(cmd: &mut std::process::Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("run {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!("{:?} failed: {}", cmd.get_program(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Name the replaced copy is moved to.
fn old_path(target: &Path) -> PathBuf {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match name.rsplit_once('.') {
        // audio-interface.exe -> audio-interface.old.exe, X.app -> X.old.app
        Some((stem, ext)) => target.with_file_name(format!("{stem}.old.{ext}")),
        None => target.with_file_name(format!("{name}.old")),
    }
}

/// Move `target` aside and `new` into its place; put the old one back on failure.
fn swap(new: &Path, target: &Path) -> Result<()> {
    let old = old_path(target);
    remove_any(&old);
    std::fs::rename(target, &old).with_context(|| format!("cannot replace {} (no permission?)", target.display()))?;
    if let Err(e) = std::fs::rename(new, target) {
        let _ = std::fs::rename(&old, target);
        return Err(e).with_context(|| format!("cannot put the new version at {}", target.display()));
    }
    // A running .exe can't be deleted on Windows; it's removed on the next start.
    remove_any(&old);
    Ok(())
}

fn remove_any(p: &Path) {
    let _ = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
}

/// Remove what an earlier update left behind (call at start-up).
pub fn cleanup() {
    if let Ok(target) = install_target() {
        remove_any(&old_path(&target));
        if let Some(parent) = target.parent() {
            remove_any(&parent.join(STAGING));
        }
    }
}

/// Start the installed new version (with `--after-update`, so it waits for this
/// copy to exit instead of handing over to it as a second instance).
pub fn relaunch(installed: &Installed) -> Result<()> {
    if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-n").arg(&installed.path).args(["--args", "--after-update"]).spawn()
    } else {
        std::process::Command::new(&installed.path).arg("--after-update").spawn()
    }
    .context("start the new version")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert!(is_newer("v0.1.3", "0.1.2"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.2.0", "0.2.0-alpha.1"));
        assert!(!is_newer("0.2.0-alpha.1", "0.2.0"));
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.1", "0.1.2"));
        assert!(!is_newer("garbage", "0.1.2"));
    }

    #[test]
    fn picks_this_os_download() {
        let json = r#"{"tag_name":"v0.1.3","html_url":"https://example/rel","assets":[
            {"name":"audio-interface-0.1.3-linux-x86_64.tar.gz","browser_download_url":"https://example/l","size":10},
            {"name":"audio-interface-0.1.3-windows-x86_64.zip","browser_download_url":"https://example/w","size":20}]}"#;
        let r = parse_latest(json, "-windows-x86_64.zip").unwrap();
        assert_eq!(r.version, "0.1.3");
        assert_eq!(r.asset_url, "https://example/w");
        assert_eq!(r.size, 20);
        assert!(parse_latest(json, "-macos-universal.zip").is_err());
    }

    #[test]
    fn verbatim_windows_paths_become_plain() {
        assert_eq!(
            plain_path(PathBuf::from(r"\\?\D:\a\audio-interface.exe")),
            PathBuf::from(r"D:\a\audio-interface.exe")
        );
        assert_eq!(plain_path(PathBuf::from(r"\\?\UNC\server\x.exe")), PathBuf::from(r"\\?\UNC\server\x.exe"));
        assert_eq!(plain_path(PathBuf::from("/usr/bin/x")), PathBuf::from("/usr/bin/x"));
    }

    #[test]
    fn old_copy_names() {
        assert_eq!(old_path(Path::new("/x/audio-interface.exe")), Path::new("/x/audio-interface.old.exe"));
        assert_eq!(old_path(Path::new("/A/Audio Interface.app")), Path::new("/A/Audio Interface.old.app"));
        assert_eq!(old_path(Path::new("/bin/audio-interface")), Path::new("/bin/audio-interface.old"));
    }

    #[test]
    fn swap_replaces_and_keeps_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("audio-interface");
        let new = dir.path().join("new");
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        swap(&new, &target).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(!old_path(&target).exists());
        assert!(!new.exists());
    }
}
