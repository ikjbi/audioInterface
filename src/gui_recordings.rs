//! Recordings window: active recordings, auto-record settings, and past
//! sessions with their drop-analysis report and a full-session chart.

use crate::analysis::{self, Finding, PeriodKind, Report, Sample};
use crate::recorder::list_recordings;
use crate::service::{ActiveRecording, FinishedRecording, Request, Service};
use crate::settings::Settings;
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2, pos2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const RED: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
const GREEN: Color32 = Color32::from_rgb(0x3f, 0xb9, 0x50);
const BLUE: Color32 = Color32::from_rgb(0x4c, 0x8d, 0xf6);
const YELLOW: Color32 = Color32::from_rgb(0xe3, 0xb3, 0x41);
const MUTED: Color32 = Color32::from_gray(140);

type Loaded = Result<(Vec<Sample>, Report), String>;

#[derive(Default)]
pub struct RecordingsPanel {
    pub open: bool,
    pub(crate) selected: Option<PathBuf>,
    pub(crate) cache: HashMap<PathBuf, Loaded>,
    pub(crate) files: Vec<PathBuf>,
    pub(crate) files_listed: Option<Instant>,
    pub(crate) known_finished: usize,
    pub(crate) apps_text: Option<String>,
}

impl RecordingsPanel {
    /// Draw the window. Returns true if settings were changed.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        service: &Service,
        active: &[ActiveRecording],
        finished: &[FinishedRecording],
    ) -> bool {
        let dir = settings.recordings_dir();
        // Re-list the folder now and then, and right after a recording finishes.
        if finished.len() != self.known_finished
            || self.files_listed.is_none_or(|t| t.elapsed() > Duration::from_secs(5))
        {
            if finished.len() != self.known_finished
                && let Some(f) = finished.last()
            {
                // Show the newest report automatically.
                self.selected = Some(f.info.csv_path.clone());
                self.cache.remove(&f.info.csv_path);
            }
            self.known_finished = finished.len();
            self.files = list_recordings(&dir);
            self.files_listed = Some(Instant::now());
            if self.selected.is_none() {
                self.selected = self.files.first().cloned();
            }
        }

        let mut changed = false;
        let mut open = self.open;
        egui::Window::new("⏺ Recordings").open(&mut open).default_size([760.0, 620.0]).resizable(true).show(
            ctx,
            |ui| {
                changed |= self.settings_section(ui, settings, &dir);
                ui.separator();
                self.active_section(ui, service, active);
                ui.separator();
                self.history_section(ui);
            },
        );
        self.open = open;
        changed
    }

    fn settings_section(&mut self, ui: &mut egui::Ui, settings: &mut Settings, dir: &Path) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            changed |= ui
                .checkbox(&mut settings.auto_record_calls, "Auto-record calls")
                .on_hover_text(
                    "Start recording an input automatically when another app (Teams, Zoom, …) starts using it, \
                     and stop when the call ends.",
                )
                .changed();
            ui.label("only for apps containing:");
            let text = self.apps_text.get_or_insert_with(|| settings.auto_record_apps.join(", "));
            let resp =
                ui.add(egui::TextEdit::singleline(text).hint_text("any app (e.g. teams, zoom)").desired_width(170.0));
            if resp.changed() {
                settings.auto_record_apps =
                    text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                changed = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            changed |= ui
                .checkbox(&mut settings.record_audio, "Include audio (16 kHz mono WAV)")
                .on_hover_text(
                    "Also save the sound itself, so you can listen to the moment it dropped. About 115 MB per hour.",
                )
                .changed();
            ui.label(RichText::new(format!("Folder: {}", dir.display())).small().color(MUTED));
            if ui.small_button("Open folder").clicked() {
                let _ = std::fs::create_dir_all(dir);
                open_path(dir);
            }
        });
        changed
    }

    fn active_section(&mut self, ui: &mut egui::Ui, service: &Service, active: &[ActiveRecording]) {
        if active.is_empty() {
            ui.label(
                RichText::new("Not recording. Press ⏺ Record on a device, or turn on auto-record above.").color(MUTED),
            );
            return;
        }
        for r in active {
            ui.horizontal(|ui| {
                ui.label(RichText::new("⏺").color(RED));
                ui.label(RichText::new(&r.info.device_name).strong());
                ui.label(analysis::fmt_duration(r.elapsed_s));
                if r.info.auto {
                    ui.label(RichText::new("auto").small().color(MUTED));
                }
                if r.audio {
                    ui.label(RichText::new("+ audio").small().color(MUTED));
                }
                if let Some(last) = r.recent.last() {
                    ui.label(
                        RichText::new(format!(
                            "{:.1} dBFS · gain {}",
                            last.rms_db,
                            last.gain_pct.map(|g| format!("{g:.0}%")).unwrap_or_else(|| "n/a".into())
                        ))
                        .monospace()
                        .small(),
                    );
                }
                if ui.button("■ Stop").clicked() {
                    service.send(Request::StopRecording(r.info.key.clone()));
                }
            });
            session_chart(ui, &r.recent, None, 110.0);
        }
    }

    fn history_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Past sessions").strong());
            egui::ComboBox::from_id_salt("rec-select")
                .width(380.0)
                .selected_text(self.selected.as_deref().map(file_label).unwrap_or_else(|| "choose a recording…".into()))
                .show_ui(ui, |ui| {
                    for f in &self.files {
                        let label = file_label(f);
                        ui.selectable_value(&mut self.selected, Some(f.clone()), label);
                    }
                });
            if self.files.is_empty() {
                ui.label(RichText::new("none yet").color(MUTED));
            }
        });
        let Some(path) = self.selected.clone() else { return };
        let loaded = self.cache.entry(path.clone()).or_insert_with(|| {
            analysis::read_csv(&path)
                .map(|rows| {
                    let report = analysis::analyze(&rows);
                    (rows, report)
                })
                .map_err(|e| format!("{e:#}"))
        });
        match loaded {
            Err(e) => {
                ui.label(RichText::new(format!("Could not read {}: {e}", path.display())).color(RED));
            }
            Ok((rows, report)) => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Duration {}", analysis::fmt_duration(report.duration_s)));
                    if let Some(b) = report.baseline_db {
                        ui.label(format!("· baseline {b:.0} dBFS"));
                    }
                    if ui.small_button("Open CSV").clicked() {
                        open_path(&path);
                    }
                    let wav = path.with_extension("wav");
                    if wav.exists() && ui.small_button("▶ Play audio").clicked() {
                        open_path(&wav);
                    }
                    let txt = PathBuf::from(format!("{}.report.txt", path.with_extension("").display()));
                    if txt.exists() && ui.small_button("Report file").clicked() {
                        open_path(&txt);
                    }
                });
                egui::Frame::new().fill(ui.visuals().faint_bg_color).inner_margin(8.0).corner_radius(4.0).show(
                    ui,
                    |ui| {
                        ui.label(&report.verdict);
                    },
                );
                session_chart(ui, rows, Some(report), 190.0);
                legend(ui);
                ui.label(RichText::new("Findings").strong());
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    if report.findings.is_empty() {
                        ui.label(RichText::new("No issues found.").color(GREEN));
                    }
                    for f in &report.findings {
                        let color = match f {
                            Finding::LevelDrop { .. } => RED,
                            Finding::GainChange { from, to, .. } if to < from => YELLOW,
                            Finding::Period { what: PeriodKind::SignalLost, .. } => RED,
                            Finding::Period { .. } | Finding::HandsFree { .. } => YELLOW,
                            _ => ui.visuals().text_color(),
                        };
                        ui.label(RichText::new(f.to_string()).color(color));
                    }
                });
            }
        }
    }
}

fn file_label(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_default()
}

fn legend(ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        let item = |ui: &mut egui::Ui, c: Color32, t: &str| {
            let (r, _) = ui.allocate_exact_size(Vec2::new(12.0, 8.0), Sense::hover());
            ui.painter().rect_filled(r, 1.0, c);
            ui.label(RichText::new(t).small().color(MUTED));
        };
        item(ui, GREEN.gamma_multiply(0.5), "level per second (RMS)");
        item(ui, GREEN, "speaking level (60 s)");
        item(ui, BLUE, "OS gain (0–100 %)");
        item(ui, RED.gamma_multiply(0.3), "detected drop");
        item(ui, Color32::from_gray(90), "signal lost / muted");
    });
}

/// Chart of a whole session: per-second RMS bars, the speaking-level trend,
/// OS gain (blue, right axis 0–100 %), drop regions and problem periods.
pub fn session_chart(ui: &mut egui::Ui, rows: &[Sample], report: Option<&Report>, height: f32) {
    let width = ui.available_width().max(200.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    if rows.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "waiting for data…",
            egui::FontId::proportional(13.0),
            MUTED,
        );
        return;
    }
    let (floor, top) = (-70.0f32, 0.0f32);
    let t0 = rows[0].t;
    let t1 = rows.last().unwrap().t.max(t0 + 1.0);
    let x_at = |t: f64| rect.left() + ((t - t0) / (t1 - t0)) as f32 * rect.width();
    let y_db = |db: f32| rect.bottom() - ((db.clamp(floor, top) - floor) / (top - floor)) * rect.height();
    let y_gain = |g: f32| rect.bottom() - (g.clamp(0.0, 100.0) / 100.0) * rect.height();

    // Grid at -60/-40/-20 dB.
    for db in [-60.0, -40.0, -20.0] {
        let y = y_db(db);
        painter.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(1.0, Color32::from_white_alpha(18)),
        );
        painter.text(
            pos2(rect.left() + 3.0, y - 1.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{db:.0} dB"),
            egui::FontId::proportional(9.0),
            MUTED,
        );
    }

    if let Some(rep) = report {
        for f in &rep.findings {
            match f {
                Finding::LevelDrop { start, end, .. } => {
                    let r = egui::Rect::from_x_y_ranges(x_at(*start)..=x_at(*end), rect.y_range());
                    painter.rect_filled(r, 0.0, RED.gamma_multiply(0.18));
                }
                Finding::Period { start, end, .. } => {
                    let r =
                        egui::Rect::from_x_y_ranges(x_at(*start)..=x_at(*end).max(x_at(*start) + 1.0), rect.y_range());
                    painter.rect_filled(r, 0.0, Color32::from_gray(90).gamma_multiply(0.5));
                }
                Finding::GainChange { t, .. } => {
                    let x = x_at(*t);
                    painter.line_segment(
                        [pos2(x, rect.top()), pos2(x, rect.bottom())],
                        Stroke::new(1.0, YELLOW.gamma_multiply(0.7)),
                    );
                }
                _ => {}
            }
        }
    }

    // Per-second RMS bars.
    let bar_w = (rect.width() / rows.len() as f32).max(1.0);
    for r in rows {
        if r.rms_db <= floor {
            continue;
        }
        let x = x_at(r.t);
        let bar = egui::Rect::from_min_max(pos2(x, y_db(r.rms_db)), pos2(x + bar_w, rect.bottom()));
        painter.rect_filled(bar, 0.0, GREEN.gamma_multiply(0.45));
    }

    // Speaking-level trend.
    let trend: Vec<egui::Pos2> = match report {
        Some(rep) => rep.speaking_level.iter().map(|(t, db)| pos2(x_at(*t), y_db(*db))).collect(),
        None => analysis::speaking_level(
            rows,
            &analysis::Params { window_s: 20, step_s: 2, min_active_s: 4, ..Default::default() },
        )
        .iter()
        .map(|(t, db)| pos2(x_at(*t), y_db(*db)))
        .collect(),
    };
    if trend.len() >= 2 {
        painter.add(egui::Shape::line(trend, Stroke::new(2.5, GREEN)));
    }
    if let Some(b) = report.and_then(|r| r.baseline_db) {
        let y = y_db(b);
        painter
            .line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, GREEN.gamma_multiply(0.5)));
    }

    // OS gain as a step line.
    let mut gain_pts = Vec::new();
    for r in rows {
        if let Some(g) = r.gain_pct {
            let x = x_at(r.t);
            if let Some(&last) = gain_pts.last() {
                let last: egui::Pos2 = last;
                gain_pts.push(pos2(x, last.y));
            }
            gain_pts.push(pos2(x, y_gain(g)));
        }
    }
    if gain_pts.len() >= 2 {
        painter.add(egui::Shape::line(gain_pts, Stroke::new(1.5, BLUE)));
    }

    // Hover readout.
    if let Some(p) = resp.hover_pos() {
        let t = t0 + ((p.x - rect.left()) / rect.width()) as f64 * (t1 - t0);
        if let Some(r) = rows.iter().min_by(|a, b| (a.t - t).abs().total_cmp(&(b.t - t).abs())) {
            painter.line_segment(
                [pos2(p.x, rect.top()), pos2(p.x, rect.bottom())],
                Stroke::new(1.0, Color32::from_white_alpha(80)),
            );
            let text = format!(
                "{}  level {:.1} dBFS  gain {}{}",
                r.time,
                r.rms_db,
                r.gain_pct.map(|g| format!("{g:.0}%")).unwrap_or_else(|| "n/a".into()),
                if r.status != "ok" { format!("  ({})", r.status) } else { String::new() }
            );
            let anchor = if p.x > rect.center().x { egui::Align2::RIGHT_TOP } else { egui::Align2::LEFT_TOP };
            painter.text(
                pos2(p.x + if p.x > rect.center().x { -6.0 } else { 6.0 }, rect.top() + 4.0),
                anchor,
                text,
                egui::FontId::monospace(11.0),
                Color32::WHITE,
            );
        }
    }
    // Time labels.
    painter.text(
        pos2(rect.left() + 3.0, rect.bottom() - 2.0),
        egui::Align2::LEFT_BOTTOM,
        &rows[0].time,
        egui::FontId::proportional(9.0),
        MUTED,
    );
    painter.text(
        pos2(rect.right() - 3.0, rect.bottom() - 2.0),
        egui::Align2::RIGHT_BOTTOM,
        &rows.last().unwrap().time,
        egui::FontId::proportional(9.0),
        MUTED,
    );
}

/// Open a file or folder with the OS default handler.
pub fn open_path(path: &Path) {
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(path).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("explorer").arg(path).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path).spawn()
    };
    if let Err(e) = result {
        log::warn!("could not open {}: {e}", path.display());
    }
}
