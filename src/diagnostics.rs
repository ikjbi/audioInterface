//! Diagnostics report for bug reports: versions, OS, audio APIs, devices,
//! relevant settings and recent events, as plain text.

use crate::model::Device;
use crate::settings::Settings;

pub struct Inputs<'a> {
    pub api: &'a str,
    pub meter_host: &'a str,
    pub devices: &'a [Device],
    pub settings: &'a Settings,
    /// Most recent last.
    pub events: &'a [String],
    pub backend_error: Option<&'a str>,
}

pub fn report(i: &Inputs) -> String {
    let mut s = String::new();
    s += &format!("Audio Interface {}\n", env!("CARGO_PKG_VERSION"));
    s += &format!("OS: {} {} ({})\n", std::env::consts::OS, os_version(), std::env::consts::ARCH);
    s += &format!(
        "Audio API: {} · meters via {}\n",
        if i.api.is_empty() { "not connected" } else { i.api },
        i.meter_host
    );
    if let Some(e) = i.backend_error {
        s += &format!("Backend error: {e}\n");
    }
    s += &format!("\nDevices ({}):\n", i.devices.len());
    for d in i.devices {
        s += &format!(
            "  [{}] {}{}  transport={} vol={} muted={} fmt={} hands_free={} in_use_by={:?}\n      id={}\n",
            d.direction,
            d.name,
            if d.is_default { " (default)" } else { "" },
            d.transport,
            d.volume.map(|v| format!("{:.0}%", v * 100.0)).unwrap_or_else(|| "n/a".into()),
            d.muted.map(|m| m.to_string()).unwrap_or_else(|| "n/a".into()),
            d.format_label().unwrap_or_else(|| "?".into()),
            d.hands_free,
            d.in_use_by,
            d.id,
        );
    }
    let st = i.settings;
    s += "\nSettings:\n";
    s += &format!(
        "  auto_switch={} preferred_in={} preferred_out={} gain_locks={} profiles={}\n",
        st.auto_switch,
        st.preferred_inputs.len(),
        st.preferred_outputs.len(),
        st.gain_locks.len(),
        st.profiles.len()
    );
    s += &format!(
        "  auto_record_calls={} apps={:?} record_audio={} notifications={} theme={:?}\n",
        st.auto_record_calls, st.auto_record_apps, st.record_audio, st.notifications, st.theme
    );
    s += &format!("\nRecent events ({}):\n", i.events.len());
    for e in i.events {
        s += &format!("  {e}\n");
    }
    s
}

fn os_version() -> String {
    let run = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    if cfg!(target_os = "macos") {
        run("sw_vers", &["-productVersion"])
    } else if cfg!(target_os = "linux") {
        std::fs::read_to_string("/etc/os-release").ok().and_then(|t| {
            t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string()))
        })
    } else {
        run("cmd", &["/C", "ver"])
    }
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Direction, test_device};

    #[test]
    fn report_contains_the_essentials() {
        let mut d = test_device("id-1", "Headset", Direction::Input);
        d.is_default = true;
        let devices = vec![d];
        let settings = Settings::default();
        let events = vec!["Connected: Headset (input)".to_string()];
        let r = report(&Inputs {
            api: "CoreAudio",
            meter_host: "CoreAudio",
            devices: &devices,
            settings: &settings,
            events: &events,
            backend_error: None,
        });
        assert!(r.contains(env!("CARGO_PKG_VERSION")));
        assert!(r.contains("Headset (default)"));
        assert!(r.contains("id=id-1"));
        assert!(r.contains("Connected: Headset"));
        assert!(r.contains(std::env::consts::OS));
    }
}
