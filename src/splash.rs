//! Animated splash screen: LED-segment level meters dancing to an imaginary
//! voice, the app name, and live start-up status. Shown for at least
//! [`MIN_SHOW`], until the first device list arrives, then fades out.

use crate::icon::meter_color;
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Vec2, pos2};
use std::time::{Duration, Instant};

const MIN_SHOW: Duration = Duration::from_millis(1400);
const MAX_SHOW: Duration = Duration::from_millis(4500);
const FADE: Duration = Duration::from_millis(450);
const BARS: usize = 9;
const SEGMENTS: usize = 14;

pub struct Splash {
    started: Instant,
    fade_started: Option<Instant>,
    done: bool,
    /// Keep the splash up indefinitely (for screenshots).
    hold: bool,
}

impl Splash {
    pub fn new() -> Self {
        Splash {
            started: Instant::now(),
            fade_started: None,
            done: std::env::var_os("AUDIO_INTERFACE_NO_SPLASH").is_some(),
            hold: std::env::var_os("AUDIO_INTERFACE_SPLASH_HOLD").is_some(),
        }
    }

    /// A splash that is already finished (e.g. when starting minimized).
    pub fn skipped() -> Self {
        Splash { done: true, ..Self::new() }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Draw the splash over everything. `ready` = start-up work finished.
    pub fn show(&mut self, ctx: &egui::Context, status: &str, ready: bool) {
        if self.done {
            return;
        }
        let elapsed = self.started.elapsed();
        if !self.hold && self.fade_started.is_none() && ((ready && elapsed >= MIN_SHOW) || elapsed >= MAX_SHOW) {
            self.fade_started = Some(Instant::now());
        }
        let alpha = match self.fade_started {
            Some(f) => {
                let t = f.elapsed().as_secs_f32() / FADE.as_secs_f32();
                if t >= 1.0 {
                    self.done = true;
                    return;
                }
                1.0 - t * t
            }
            // Fade in quickly too.
            None => (elapsed.as_secs_f32() / 0.25).min(1.0),
        };
        let screen = ctx.content_rect();
        let time = elapsed.as_secs_f32();

        egui::Area::new(egui::Id::unique("splash"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .interactable(true)
            .show(ctx, |ui| {
                // Swallow clicks while the splash is visible.
                ui.allocate_rect(screen, Sense::click_and_drag());
                let p = ui.painter();
                let a = |c: Color32| c.gamma_multiply(alpha);

                // Background: vertical navy gradient (mesh with vertex colours).
                let mut mesh = egui::Mesh::default();
                let top = a(Color32::from_rgb(0x1f, 0x29, 0x46));
                let bottom = a(Color32::from_rgb(0x0a, 0x0e, 0x1c));
                mesh.colored_vertex(screen.left_top(), top);
                mesh.colored_vertex(screen.right_top(), top);
                mesh.colored_vertex(screen.right_bottom(), bottom);
                mesh.colored_vertex(screen.left_bottom(), bottom);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                p.add(egui::Shape::mesh(mesh));

                // Meter block.
                let size = Vec2::new(
                    (screen.width() * 0.42).clamp(220.0, 420.0),
                    (screen.height() * 0.32).clamp(120.0, 240.0),
                );
                let centre = screen.center() - Vec2::new(0.0, size.y * 0.35);
                let block = Rect::from_center_size(centre, size);
                let slot = block.width() / BARS as f32;
                let bar_w = slot * 0.62;
                let seg_h = block.height() / SEGMENTS as f32;
                for i in 0..BARS {
                    let level = bar_level(i, time);
                    let peak = peak_level(i, time);
                    let x0 = block.left() + slot * i as f32 + (slot - bar_w) / 2.0;
                    for s in 0..SEGMENTS {
                        let frac = (s as f32 + 0.5) / SEGMENTS as f32;
                        let y1 = block.bottom() - seg_h * s as f32;
                        let r = Rect::from_min_max(pos2(x0, y1 - seg_h * 0.78), pos2(x0 + bar_w, y1));
                        let [cr, cg, cb] = meter_color(frac);
                        let lit = frac <= level;
                        let is_peak = (frac - peak).abs() < 0.5 / SEGMENTS as f32 && peak > level;
                        let col = Color32::from_rgb(cr, cg, cb);
                        let col = if lit || is_peak { col } else { col.gamma_multiply(0.10) };
                        p.rect_filled(r, 2.0, a(col));
                    }
                }
                // Reflection of the meters.
                let refl = Rect::from_min_size(
                    pos2(block.left(), block.bottom() + 6.0),
                    Vec2::new(block.width(), seg_h * 3.0),
                );
                for i in 0..BARS {
                    let level = bar_level(i, time);
                    let x0 = refl.left() + slot * i as f32 + (slot - bar_w) / 2.0;
                    let h = refl.height() * level.min(1.0);
                    let [cr, cg, cb] = meter_color(0.2);
                    p.rect_filled(
                        Rect::from_min_size(pos2(x0, refl.top()), Vec2::new(bar_w, h)),
                        2.0,
                        a(Color32::from_rgb(cr, cg, cb).gamma_multiply(0.12)),
                    );
                }

                // Title, tagline, status.
                let mut y = refl.bottom() + 28.0;
                p.text(
                    pos2(screen.center().x, y),
                    egui::Align2::CENTER_TOP,
                    "Audio Interface",
                    FontId::proportional(36.0),
                    a(Color32::WHITE),
                );
                y += 48.0;
                p.text(
                    pos2(screen.center().x, y),
                    egui::Align2::CENTER_TOP,
                    "Every mic, speaker and level, under control",
                    FontId::proportional(15.0),
                    a(Color32::from_gray(170)),
                );
                y += 34.0;
                let dots = ".".repeat(((time * 3.0) as usize % 3) + 1);
                let status_text = if ready { status.to_string() } else { format!("{status}{dots}") };
                p.text(
                    pos2(screen.center().x, y),
                    egui::Align2::CENTER_TOP,
                    status_text,
                    FontId::monospace(12.0),
                    a(Color32::from_gray(130)),
                );
                p.text(
                    screen.right_bottom() - Vec2::new(12.0, 10.0),
                    egui::Align2::RIGHT_BOTTOM,
                    concat!("v", env!("CARGO_PKG_VERSION")),
                    FontId::proportional(11.0),
                    a(Color32::from_gray(100)),
                );
                let _: Pos2 = centre;
            });
        ctx.request_repaint();
    }
}

impl Default for Splash {
    fn default() -> Self {
        Self::new()
    }
}

/// Voice-like level for bar `i` at time `t`: a shared syllable envelope times
/// a per-bar "spectrum" shape, plus flicker. Range 0..=1.
fn bar_level(i: usize, t: f32) -> f32 {
    let centre = (BARS as f32 - 1.0) / 2.0;
    let shape = 1.0 - ((i as f32 - centre) / (centre + 1.0)).powi(2) * 0.75;
    let syllable = ((t * 5.3).sin() * 0.5 + 0.5) * ((t * 1.7 + 1.0).sin() * 0.35 + 0.65);
    let flicker = ((t * 17.0 + i as f32 * 1.9).sin() * 0.5 + 0.5) * 0.25;
    (shape * (0.25 + 0.75 * syllable) + flicker - 0.1).clamp(0.05, 1.0)
}

/// Peak-hold cap: the max of recent levels, falling slowly.
fn peak_level(i: usize, t: f32) -> f32 {
    let mut peak: f32 = 0.0;
    for k in 0..12 {
        let dt = k as f32 * 0.06;
        peak = peak.max(bar_level(i, t - dt) - dt * 0.6);
    }
    peak.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn levels_stay_in_range() {
        for i in 0..super::BARS {
            for k in 0..500 {
                let t = k as f32 * 0.037;
                let l = super::bar_level(i, t);
                let p = super::peak_level(i, t);
                assert!((0.0..=1.0).contains(&l));
                assert!(p >= l - 1e-6, "peak hold is at least the level");
            }
        }
    }
}
