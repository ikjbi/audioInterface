//! Desktop integration: global hotkeys (all OSes; X11 on Linux) and a system
//! tray / menu-bar icon (Windows and macOS: on Linux the tray would need GTK
//! development headers, which breaks the sudo-free build).
//!
//! Must be created on the main thread (inside eframe's app creation). Event
//! handlers wake the UI with `request_repaint`, and the app drains
//! [`Desktop::poll`] in `App::logic`, which also runs while the window is hidden.

use crate::model::{Device, DeviceKey, Direction};
use crate::settings::Settings;
use eframe::egui;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    ToggleMicMute,
    NextOutput,
    NextInput,
    ShowWindow,
    Quit,
    SetDefault(DeviceKey),
    ApplyProfile(String),
    ToggleRecordDefaultInput,
}

/// Parse a hotkey string, with a friendly error.
pub fn parse_hotkey(s: &str) -> Result<HotKey, String> {
    HotKey::from_str(s.trim()).map_err(|e| format!("{e}"))
}

pub struct Desktop {
    ctx: egui::Context,
    hotkeys: Option<GlobalHotKeyManager>,
    registered: Vec<(HotKey, Command)>,
    hotkey_config: (String, String, String),
    pub hotkey_errors: Vec<String>,
    #[cfg(not(target_os = "linux"))]
    tray: Option<tray::Tray>,
}

impl Desktop {
    pub fn new(ctx: &egui::Context, settings: &Settings, with_tray: bool) -> Self {
        let hotkeys = match GlobalHotKeyManager::new() {
            Ok(m) => Some(m),
            Err(e) => {
                log::warn!("global hotkeys unavailable: {e}");
                None
            }
        };
        let wake = ctx.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |_e: GlobalHotKeyEvent| wake.request_repaint()));
        let mut d = Desktop {
            ctx: ctx.clone(),
            hotkeys,
            registered: Vec::new(),
            hotkey_config: Default::default(),
            hotkey_errors: Vec::new(),
            #[cfg(not(target_os = "linux"))]
            tray: None,
        };
        d.update_hotkeys(settings);
        #[cfg(not(target_os = "linux"))]
        if with_tray {
            d.tray = tray::Tray::new(ctx);
        }
        #[cfg(target_os = "linux")]
        let _ = with_tray;
        d
    }

    pub fn has_tray(&self) -> bool {
        #[cfg(not(target_os = "linux"))]
        return self.tray.is_some();
        #[cfg(target_os = "linux")]
        false
    }

    /// (Re-)register hotkeys if the settings changed.
    pub fn update_hotkeys(&mut self, settings: &Settings) {
        let config =
            (settings.hotkey_mute_mic.clone(), settings.hotkey_next_output.clone(), settings.hotkey_next_input.clone());
        if config == self.hotkey_config && !self.registered.is_empty() {
            return;
        }
        self.hotkey_config = config.clone();
        self.hotkey_errors.clear();
        let Some(mgr) = &self.hotkeys else { return };
        for (hk, _) in self.registered.drain(..) {
            let _ = mgr.unregister(hk);
        }
        for (text, cmd) in
            [(&config.0, Command::ToggleMicMute), (&config.1, Command::NextOutput), (&config.2, Command::NextInput)]
        {
            if text.trim().is_empty() {
                continue;
            }
            match parse_hotkey(text) {
                Ok(hk) => match mgr.register(hk) {
                    Ok(()) => self.registered.push((hk, cmd)),
                    Err(e) => self.hotkey_errors.push(format!("{text}: {e}")),
                },
                Err(e) => self.hotkey_errors.push(format!("{text}: {e}")),
            }
        }
    }

    /// Commands triggered since the last call.
    pub fn poll(&mut self) -> Vec<Command> {
        let mut out = Vec::new();
        while let Ok(e) = GlobalHotKeyEvent::receiver().try_recv() {
            if e.state == HotKeyState::Pressed
                && let Some((_, cmd)) = self.registered.iter().find(|(hk, _)| hk.id() == e.id)
            {
                out.push(cmd.clone());
            }
        }
        #[cfg(not(target_os = "linux"))]
        if let Some(t) = &mut self.tray {
            out.extend(t.poll());
        }
        if !out.is_empty() {
            self.ctx.request_repaint();
        }
        out
    }

    /// Refresh the tray menu (cheap if nothing changed).
    pub fn update_tray(&mut self, state: &TrayState) {
        #[cfg(not(target_os = "linux"))]
        if let Some(t) = &mut self.tray {
            t.update(state);
        }
        #[cfg(target_os = "linux")]
        let _ = state;
    }
}

/// What the tray menu shows.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TrayState {
    pub inputs: Vec<(DeviceKey, String, bool)>,
    pub outputs: Vec<(DeviceKey, String, bool)>,
    pub mic_muted: Option<bool>,
    pub recording: bool,
    pub profiles: Vec<String>,
}

impl TrayState {
    pub fn build(devices: &[Device], settings: &Settings, recording: bool) -> Self {
        let list = |dir: Direction| -> Vec<(DeviceKey, String, bool)> {
            settings
                .cycle_order(devices, dir)
                .into_iter()
                .map(|d| (d.key(), settings.display_name(d).to_string(), d.is_default))
                .collect()
        };
        TrayState {
            inputs: list(Direction::Input),
            outputs: list(Direction::Output),
            mic_muted: devices.iter().find(|d| d.direction == Direction::Input && d.is_default).and_then(|d| d.muted),
            recording,
            profiles: settings.profiles.iter().map(|p| p.name.clone()).collect(),
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod tray {
    use super::{Command, TrayState};
    use eframe::egui;
    use std::collections::HashMap;
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

    pub struct Tray {
        icon: TrayIcon,
        actions: HashMap<MenuId, Command>,
        shown: Option<TrayState>,
    }

    impl Tray {
        pub fn new(ctx: &egui::Context) -> Option<Self> {
            let size = 64;
            let icon = Icon::from_rgba(crate::icon::rgba(size), size, size).ok()?;
            let wake = ctx.clone();
            MenuEvent::set_event_handler(Some(move |_e: MenuEvent| wake.request_repaint()));
            let wake = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |_e: TrayIconEvent| wake.request_repaint()));
            match TrayIconBuilder::new()
                .with_tooltip("Audio Interface")
                .with_icon(icon)
                .with_menu(Box::new(Menu::new()))
                .build()
            {
                Ok(icon) => Some(Tray { icon, actions: HashMap::new(), shown: None }),
                Err(e) => {
                    log::warn!("tray icon unavailable: {e}");
                    None
                }
            }
        }

        pub fn poll(&mut self) -> Vec<Command> {
            let mut out = Vec::new();
            while let Ok(e) = MenuEvent::receiver().try_recv() {
                if let Some(c) = self.actions.get(&e.id) {
                    out.push(c.clone());
                }
            }
            // Drain icon clicks (the menu handles everything; nothing else to do).
            while TrayIconEvent::receiver().try_recv().is_ok() {}
            out
        }

        pub fn update(&mut self, state: &TrayState) {
            if self.shown.as_ref() == Some(state) {
                return;
            }
            let mut actions = HashMap::new();
            let menu = Menu::new();
            let show = MenuItem::new("Show Audio Interface", true, None);
            actions.insert(show.id().clone(), Command::ShowWindow);
            let _ = menu.append(&show);
            let _ = menu.append(&PredefinedMenuItem::separator());

            let mute_text = match state.mic_muted {
                Some(true) => "Unmute microphone",
                _ => "Mute microphone",
            };
            let mute = CheckMenuItem::new(mute_text, state.mic_muted.is_some(), state.mic_muted == Some(true), None);
            actions.insert(mute.id().clone(), Command::ToggleMicMute);
            let _ = menu.append(&mute);

            for (title, list) in [("Output", &state.outputs), ("Input", &state.inputs)] {
                let sub = Submenu::new(title, !list.is_empty());
                for (key, name, is_default) in list {
                    let item = CheckMenuItem::new(name, true, *is_default, None);
                    actions.insert(item.id().clone(), Command::SetDefault(key.clone()));
                    let _ = sub.append(&item);
                }
                let _ = menu.append(&sub);
            }
            if !state.profiles.is_empty() {
                let sub = Submenu::new("Profiles", true);
                for p in &state.profiles {
                    let item = MenuItem::new(p, true, None);
                    actions.insert(item.id().clone(), Command::ApplyProfile(p.clone()));
                    let _ = sub.append(&item);
                }
                let _ = menu.append(&sub);
            }
            let rec =
                MenuItem::new(if state.recording { "⏹ Stop recording" } else { "⏺ Record default input" }, true, None);
            actions.insert(rec.id().clone(), Command::ToggleRecordDefaultInput);
            let _ = menu.append(&rec);
            let _ = menu.append(&PredefinedMenuItem::separator());
            let quit = MenuItem::new("Quit", true, None);
            actions.insert(quit.id().clone(), Command::Quit);
            let _ = menu.append(&quit);

            self.icon.set_menu(Some(Box::new(menu)));
            let tip = match state.mic_muted {
                Some(true) => "Audio Interface: mic muted",
                _ if state.recording => "Audio Interface: recording",
                _ => "Audio Interface",
            };
            let _ = self.icon.set_tooltip(Some(tip));
            self.actions = actions;
            self.shown = Some(state.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hotkeys_parse() {
        let s = Settings::default();
        for k in [&s.hotkey_mute_mic, &s.hotkey_next_output, &s.hotkey_next_input] {
            parse_hotkey(k).unwrap_or_else(|e| panic!("{k}: {e}"));
        }
        assert!(parse_hotkey("Ctrl+Nope+X").is_err());
    }

    #[test]
    fn tray_state_lists_devices_in_cycle_order() {
        use crate::model::test_device;
        let mut mic = test_device("m", "Mic", Direction::Input);
        mic.is_default = true;
        mic.muted = Some(true);
        let devices = vec![mic, test_device("s", "Speakers", Direction::Output)];
        let st = TrayState::build(&devices, &Settings::default(), false);
        assert_eq!(st.mic_muted, Some(true));
        assert_eq!(st.inputs.len(), 1);
        assert_eq!(st.outputs[0].1, "Speakers");
    }
}
