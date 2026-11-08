//! Desktop UI (egui / eframe).
//!
//! The UI is a thin view over [`Service`]: every frame it copies the latest
//! snapshot, drains level cells into meter ballistics, and turns clicks into
//! [`Request`]s. Nothing in here blocks on the OS.

use crate::levels::{MeterState, SignalStatus, meter_position};
use crate::model::{Device, DeviceKey, DeviceRef, Direction, Transport};
use crate::service::{Event, EventKind, Request, Service};
use crate::settings::Settings;
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

pub fn run(service: Service, settings: Settings, settings_path: Option<PathBuf>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Audio Interface")
            .with_inner_size([1000.0, 740.0])
            .with_min_inner_size([440.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Audio Interface",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(service, settings, settings_path)))),
    )
}

/// Data copied out of the service snapshot each frame.
#[derive(Default)]
struct View {
    api: &'static str,
    meter_host: &'static str,
    devices: Vec<Device>,
    error: Option<String>,
    events: Vec<Event>,
    meter_errors: HashMap<DeviceKey, String>,
    monitored: BTreeSet<DeviceKey>,
    native_metered: BTreeSet<DeviceKey>,
}

struct App {
    service: Service,
    settings: Settings,
    settings_path: Option<PathBuf>,
    settings_dirty: bool,
    meters: HashMap<DeviceKey, MeterState>,
    last_frame: Instant,
    /// Slider value being dragged, so refreshes don't make it jump.
    dragging: Option<(DeviceKey, f32)>,
    show_settings: bool,
    show_log: bool,
    view: View,
}

const GREEN: Color32 = Color32::from_rgb(0x3f, 0xb9, 0x50);
const YELLOW: Color32 = Color32::from_rgb(0xe3, 0xb3, 0x41);
const RED: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
const BLUE: Color32 = Color32::from_rgb(0x4c, 0x8d, 0xf6);
const MUTED_TEXT: Color32 = Color32::from_gray(140);

impl App {
    fn new(service: Service, settings: Settings, settings_path: Option<PathBuf>) -> Self {
        App {
            service,
            settings,
            settings_path,
            settings_dirty: false,
            meters: HashMap::new(),
            last_frame: Instant::now(),
            dragging: None,
            show_settings: false,
            show_log: true,
            view: View::default(),
        }
    }

    fn settings_changed(&mut self) {
        self.service.send(Request::UpdateSettings(self.settings.clone()));
        self.settings_dirty = true;
    }

    fn save_settings_if_needed(&mut self) {
        if self.settings_dirty && self.dragging.is_none() {
            self.settings_dirty = false;
            if let Some(path) = &self.settings_path
                && let Err(e) = self.settings.save(path)
            {
                log::error!("saving settings: {e:#}");
            }
        }
    }

    /// Copy the snapshot and advance meter ballistics.
    fn pull(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).min(Duration::from_millis(250));
        self.last_frame = now;

        let snap = self.service.snapshot();
        self.view = View {
            api: snap.api,
            meter_host: snap.meter_host,
            devices: snap.devices.clone(),
            error: snap.error.clone(),
            events: snap.events.iter().rev().take(100).cloned().collect(),
            meter_errors: snap.meter_errors.clone(),
            monitored: snap.monitored.clone(),
            native_metered: snap.native_metered.clone(),
        };
        let readings: Vec<_> = snap.levels.iter().map(|(k, c)| (k.clone(), c.take())).collect();
        drop(snap);

        self.meters.retain(|k, _| readings.iter().any(|(rk, _)| rk == k));
        for (key, reading) in readings {
            self.meters.entry(key).or_insert_with(|| MeterState::new(now)).update(reading, dt, now);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Audio Interface");
            ui.add_space(8.0);
            let api = if self.view.api.is_empty() { "connecting…" } else { self.view.api };
            ui.label(RichText::new(format!("{api} · meters via {}", self.view.meter_host)).color(MUTED_TEXT).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙ Settings").clicked() {
                    self.show_settings = !self.show_settings;
                }
                if ui.button("⟳ Refresh").clicked() {
                    self.service.send(Request::Refresh);
                }
                ui.toggle_value(&mut self.show_log, "📋 Log");
                let mut auto = self.settings.auto_switch;
                if ui
                    .toggle_value(&mut auto, "⇄ Auto-switch")
                    .on_hover_text("Make the most preferred device the default whenever devices connect or disconnect")
                    .changed()
                {
                    self.settings.auto_switch = auto;
                    self.settings_changed();
                }
            });
        });
    }

    fn device_columns(&mut self, ui: &mut egui::Ui) {
        let wide = ui.available_width() > 860.0;
        let devices = self.view.devices.clone();
        let visible = |dir: Direction| -> Vec<Device> {
            devices
                .iter()
                .filter(|d| d.direction == dir)
                .filter(|d| {
                    !(self.settings.hide_virtual && matches!(d.transport, Transport::Virtual | Transport::Aggregate))
                })
                .cloned()
                .collect()
        };
        let inputs = visible(Direction::Input);
        let outputs = visible(Direction::Output);

        if wide {
            ui.columns(2, |cols| {
                self.device_section(&mut cols[0], Direction::Input, &inputs);
                self.device_section(&mut cols[1], Direction::Output, &outputs);
            });
        } else {
            self.device_section(ui, Direction::Input, &inputs);
            ui.add_space(12.0);
            self.device_section(ui, Direction::Output, &outputs);
        }
    }

    fn device_section(&mut self, ui: &mut egui::Ui, dir: Direction, devices: &[Device]) {
        let title = match dir {
            Direction::Input => "🎤 Inputs",
            Direction::Output => "🔊 Outputs",
        };
        ui.label(RichText::new(title).heading());
        if devices.is_empty() {
            ui.label(RichText::new(format!("No {dir} devices found")).color(MUTED_TEXT));
        }
        // Default device first, then preferred order, then by name.
        let mut sorted: Vec<&Device> = devices.iter().collect();
        let pref_rank = |d: &Device| {
            self.settings.preferred(dir).iter().position(|r| r.id == d.id || r.name == d.name).unwrap_or(usize::MAX)
        };
        sorted.sort_by_key(|d| (!d.is_default, pref_rank(d), d.name.to_lowercase()));
        for d in sorted {
            self.device_card(ui, d);
            ui.add_space(6.0);
        }
    }

    fn device_card(&mut self, ui: &mut egui::Ui, d: &Device) {
        let key = d.key();
        let stroke = if d.is_default { Stroke::new(1.5, GREEN) } else { ui.visuals().widgets.noninteractive.bg_stroke };
        egui::Frame::group(ui.style()).stroke(stroke).inner_margin(10.0).show(ui, |ui| {
            ui.set_width(ui.available_width());

            // Header: name, badges, "make default".
            ui.horizontal(|ui| {
                ui.label(RichText::new(&d.name).strong().size(15.0));
                if d.is_default {
                    ui.label(RichText::new("DEFAULT").small().strong().color(GREEN));
                }
                if let Some(rank) = self.settings.preferred(d.direction).iter().position(|r| r.id == d.id || r.name == d.name) {
                    ui.label(RichText::new(format!("★{}", rank + 1)).small().color(YELLOW))
                        .on_hover_text("Position in your preferred-device list");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !d.is_default && ui.button("Make default").clicked() {
                        self.service.send(Request::SetDefault(key.clone()));
                    }
                    ui.label(RichText::new(transport_icon(d.transport)).small().color(MUTED_TEXT));
                });
            });

            // Bluetooth guidance.
            if d.transport == Transport::Bluetooth && d.direction == Direction::Input {
                ui.label(
                    RichText::new("ⓘ Using this mic switches the headset to hands-free (call-quality) audio.")
                        .small()
                        .color(MUTED_TEXT),
                );
            }
            if d.direction == Direction::Output && d.hands_free {
                ui.label(
                    RichText::new("⚠ Hands-free mode: playback quality is reduced while the headset mic is in use.")
                        .small()
                        .color(YELLOW),
                );
            }

            // Volume, mute, lock.
            ui.horizontal(|ui| {
                if let Some(muted) = d.muted {
                    let label = if muted { "🔇" } else { if d.direction == Direction::Input { "🎤" } else { "🔊" } };
                    let btn = egui::Button::new(RichText::new(label).size(15.0)).selected(muted);
                    if ui.add(btn).on_hover_text(if muted { "Unmute" } else { "Mute" }).clicked() {
                        self.service.send(Request::SetMute(key.clone(), !muted));
                    }
                }
                if let Some(vol) = d.volume {
                    let locked = self.settings.gain_lock_for(&key, &d.name).map(|l| l.volume);
                    let mut pct = match &self.dragging {
                        Some((k, v)) if *k == key => *v,
                        _ => vol * 100.0,
                    };
                    let width = (ui.available_width() - 110.0).max(80.0);
                    ui.spacing_mut().slider_width = width;
                    let resp = ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%").fixed_decimals(0));
                    if resp.dragged() || resp.changed() {
                        self.dragging = Some((key.clone(), pct));
                    }
                    if resp.changed() {
                        if locked.is_some() {
                            self.settings.set_gain_lock(DeviceRef::of(d), d.direction, Some(pct / 100.0));
                            self.settings_changed();
                        }
                        self.service.send(Request::SetVolume(key.clone(), pct / 100.0));
                    }
                    if !resp.dragged() && self.dragging.as_ref().is_some_and(|(k, _)| *k == key) && !resp.changed() {
                        self.dragging = None;
                    }
                    let mut is_locked = locked.is_some();
                    let lock_label = if is_locked { "🔒" } else { "🔓" };
                    if ui
                        .toggle_value(&mut is_locked, lock_label)
                        .on_hover_text(match locked {
                            Some(v) => format!("Gain locked at {:.0}%. Click to unlock.", v * 100.0),
                            None => "Lock the gain at its current level. If an app (Teams, Zoom, …) changes it, it is put back.".into(),
                        })
                        .changed()
                    {
                        let v = if is_locked { Some(vol) } else { None };
                        self.settings.set_gain_lock(DeviceRef::of(d), d.direction, v);
                        self.settings_changed();
                    }
                } else {
                    ui.label(RichText::new("No software volume control").small().color(MUTED_TEXT));
                }
            });

            // Meter.
            self.meter_row(ui, d);

            // Preference toggle.
            ui.horizontal(|ui| {
                let is_pref = self.settings.preferred(d.direction).iter().any(|r| r.id == d.id || r.name == d.name);
                let mut p = is_pref;
                if ui
                    .toggle_value(&mut p, if is_pref { "★ Preferred" } else { "☆ Prefer" })
                    .on_hover_text("Preferred devices become the default automatically when they connect (with Auto-switch on). Reorder them in Settings.")
                    .changed()
                {
                    self.settings.toggle_preferred(DeviceRef::of(d), d.direction);
                    self.settings_changed();
                }
            });
        });
    }

    fn meter_row(&mut self, ui: &mut egui::Ui, d: &Device) {
        let key = d.key();
        let native = self.view.native_metered.contains(&key);
        let user_monitored = self.view.monitored.contains(&key);
        let floor = self.settings.meter_floor_db;

        if let Some(m) = self.meters.get(&key) {
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 150.0).max(60.0);
                draw_meter(ui, m, floor, Vec2::new(w, 16.0));
                let peak = if m.peak_hold_db <= crate::levels::FLOOR_DB + 0.5 {
                    "-∞ dB".to_string()
                } else {
                    format!("{:.1} dB", m.peak_hold_db)
                };
                ui.label(RichText::new(peak).monospace());
                if !native {
                    let (txt, color) = match m.status {
                        SignalStatus::Ok => ("●", GREEN),
                        SignalStatus::Starting => ("○", MUTED_TEXT),
                        SignalStatus::Silent => ("●", YELLOW),
                        SignalStatus::Lost => ("●", RED),
                    };
                    ui.label(RichText::new(txt).color(color)).on_hover_text(m.status.label());
                }
                if user_monitored && ui.small_button("■").on_hover_text("Stop metering").clicked() {
                    self.service.send(Request::SetMonitored(key.clone(), false));
                }
            });
            if !native {
                match m.status {
                    SignalStatus::Lost => {
                        ui.label(
                            RichText::new("⚠ Signal lost: no audio is arriving from this device.").small().color(RED),
                        );
                    }
                    SignalStatus::Silent => {
                        ui.label(
                            RichText::new(
                                "⚠ Digital silence: the device is delivering only zeros (muted in hardware?).",
                            )
                            .small()
                            .color(YELLOW),
                        );
                    }
                    _ => {}
                }
            }
            if m.max_db > -0.1 {
                ui.label(
                    RichText::new("⚠ Clipping: the signal hit 0 dBFS. Consider lowering the gain.").small().color(RED),
                );
            }
        } else if let Some(err) = self.view.meter_errors.get(&key) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Meter unavailable: {err}")).small().color(YELLOW));
                if user_monitored && ui.small_button("✕").clicked() {
                    self.service.send(Request::SetMonitored(key.clone(), false));
                }
            });
        } else if user_monitored {
            ui.label(RichText::new("Starting meter…").small().color(MUTED_TEXT));
        } else {
            let can_meter = d.direction == Direction::Input || cfg!(target_os = "linux");
            if can_meter {
                let hint = if d.transport == Transport::Bluetooth && d.direction == Direction::Input {
                    "Show a live level meter. Note: this opens the Bluetooth mic, which switches the headset to hands-free mode."
                } else {
                    "Show a live level meter for this device"
                };
                if ui.small_button("▶ Show level").on_hover_text(hint).clicked() {
                    self.service.send(Request::SetMonitored(key.clone(), true));
                }
            }
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        let mut changed = false;
        egui::Window::new("Settings").open(&mut open).resizable(true).default_width(420.0).show(ctx, |ui| {
            ui.heading("Metering");
            changed |=
                ui.checkbox(&mut self.settings.monitor_default_input, "Always meter the default input").changed();
            changed |= ui
                .checkbox(&mut self.settings.monitor_bluetooth_inputs, "…even when it is a Bluetooth mic")
                .on_hover_text("Opening a Bluetooth mic switches the headset to hands-free mode")
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut self.settings.meter_floor_db, -96.0..=-30.0).text("meter floor (dBFS)"))
                .changed();
            changed |= ui.checkbox(&mut self.settings.hide_virtual, "Hide virtual and aggregate devices").changed();

            ui.separator();
            ui.heading("Preferred devices");
            changed |=
                ui.checkbox(&mut self.settings.auto_switch, "Auto-switch when devices connect or disconnect").changed();
            for dir in Direction::ALL {
                ui.label(RichText::new(format!("{}s (highest priority first)", capitalize(dir.label()))).strong());
                let list = self.settings.preferred(dir).clone();
                if list.is_empty() {
                    ui.label(RichText::new("None. Click ☆ Prefer on a device.").small().color(MUTED_TEXT));
                }
                for (i, r) in list.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let present =
                            self.view.devices.iter().any(|d| d.direction == dir && (d.id == r.id || d.name == r.name));
                        let name = RichText::new(format!("{}. {}", i + 1, r.name));
                        ui.label(if present { name } else { name.color(MUTED_TEXT) }).on_hover_text(if present {
                            "Connected"
                        } else {
                            "Not connected"
                        });
                        if ui.small_button("⬆").clicked() {
                            self.settings.move_preferred(dir, i, -1);
                            changed = true;
                        }
                        if ui.small_button("⬇").clicked() {
                            self.settings.move_preferred(dir, i, 1);
                            changed = true;
                        }
                        if ui.small_button("✕").clicked() {
                            self.settings.toggle_preferred(r.clone(), dir);
                            changed = true;
                        }
                    });
                }
            }

            ui.separator();
            ui.heading("Gain locks");
            let locks = self.settings.gain_locks.clone();
            if locks.is_empty() {
                ui.label(RichText::new("None. Click 🔓 next to a volume slider.").small().color(MUTED_TEXT));
            }
            for l in locks {
                ui.horizontal(|ui| {
                    ui.label(format!("{} ({}) at {:.0}%", l.device.name, l.direction, l.volume * 100.0));
                    if ui.small_button("✕").clicked() {
                        self.settings.set_gain_lock(l.device.clone(), l.direction, None);
                        changed = true;
                    }
                });
            }

            ui.separator();
            if let Some(p) = &self.settings_path {
                ui.label(RichText::new(format!("Settings file: {}", p.display())).small().color(MUTED_TEXT));
            }
        });
        self.show_settings = open;
        if changed {
            self.settings_changed();
        }
    }

    fn log_panel(&self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if self.view.events.is_empty() {
                ui.label(RichText::new("No events yet").color(MUTED_TEXT));
            }
            let now = SystemTime::now();
            for e in &self.view.events {
                let color = match e.kind {
                    EventKind::Info => ui.visuals().text_color(),
                    EventKind::Action => BLUE,
                    EventKind::Warning => YELLOW,
                    EventKind::Error => RED,
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(ago(now, e.at)).monospace().small().color(MUTED_TEXT));
                    ui.label(RichText::new(&e.text).color(color));
                });
            }
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pull();
        let ctx = ui.ctx().clone();

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });

        if self.show_log {
            egui::Panel::bottom("log").resizable(true).default_size(140.0).show(ui, |ui| {
                ui.add_space(4.0);
                ui.label(RichText::new("Event log").strong());
                self.log_panel(ui);
            });
        }

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(err) = &self.view.error {
                egui::Frame::new().fill(RED.gamma_multiply(0.2)).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
                    ui.label(RichText::new(format!("⚠ {err}")).color(RED));
                });
                ui.add_space(8.0);
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                self.device_columns(ui);
            });
        });

        if self.show_settings {
            self.settings_window(&ctx);
        }
        self.save_settings_if_needed();
        ctx.request_repaint_after(Duration::from_millis(33));
    }

    fn on_exit(&mut self) {
        self.dragging = None;
        self.save_settings_if_needed();
    }
}

/// Horizontal bar meter: green / yellow / red zones, RMS shading, peak-hold tick.
fn draw_meter(ui: &mut egui::Ui, m: &MeterState, floor_db: f32, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    let bg = ui.visuals().extreme_bg_color;
    painter.rect_filled(rect, 3.0, bg);

    let x_at = |db: f32| rect.left() + rect.width() * meter_position(db, floor_db);
    let level_x = x_at(m.level_db);
    let zones = [(floor_db, -18.0, GREEN), (-18.0, -6.0, YELLOW), (-6.0, 0.0, RED)];
    for (lo, hi, color) in zones {
        let x0 = x_at(lo.max(floor_db));
        let x1 = x_at(hi).min(level_x);
        if x1 > x0 {
            let r = egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom()));
            painter.rect_filled(r, 0.0, color);
        }
    }
    // RMS: darker band along the bottom third.
    let rms_x = x_at(m.rms_db);
    if rms_x > rect.left() {
        let r = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - rect.height() / 3.0),
            egui::pos2(rms_x, rect.bottom()),
        );
        painter.rect_filled(r, 0.0, Color32::from_black_alpha(90));
    }
    // Peak hold.
    let px = x_at(m.peak_hold_db);
    if px > rect.left() + 1.0 {
        painter.line_segment(
            [egui::pos2(px, rect.top()), egui::pos2(px, rect.bottom())],
            Stroke::new(2.0, Color32::WHITE),
        );
    }
    // Scale ticks every 6 dB.
    let mut db = 0.0;
    while db > floor_db {
        let x = x_at(db);
        painter.line_segment(
            [egui::pos2(x, rect.bottom() - 3.0), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_white_alpha(60)),
        );
        db -= 6.0;
    }
    if m.clipped {
        let r = egui::Rect::from_min_size(egui::pos2(rect.right() - 6.0, rect.top()), Vec2::new(6.0, rect.height()));
        painter.rect_filled(r, 0.0, RED);
    }
}

fn transport_icon(t: Transport) -> String {
    let icon = match t {
        Transport::Bluetooth => "ᛒ",
        Transport::Usb => "⭘",
        Transport::BuiltIn => "💻",
        Transport::Hdmi | Transport::DisplayPort => "🖵",
        Transport::Virtual | Transport::Aggregate => "◇",
        Transport::Network => "📡",
        _ => "•",
    };
    format!("{icon} {}", t.label())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn ago(now: SystemTime, at: SystemTime) -> String {
    let s = now.duration_since(at).unwrap_or_default().as_secs();
    match s {
        0..=4 => "now".into(),
        5..=59 => format!("{s}s ago"),
        60..=3599 => format!("{}m ago", s / 60),
        _ => format!("{}h ago", s / 3600),
    }
}
