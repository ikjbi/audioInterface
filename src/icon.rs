//! The app icon, drawn in code: level-meter bars on a rounded dark tile.
//! Drawing it here means no binary assets for the window/taskbar icon on any
//! OS. `audio-interface export-icon` writes it as a PNG for bundling.

/// Render the icon as straight (non-premultiplied) RGBA8, `size`×`size`.
pub fn rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let mut px = vec![0u8; (size * size * 4) as usize];
    let radius = s * 0.22;
    let margin = s * 0.04;
    // (height fraction, color) for each bar, left to right.
    let bars: [(f32, [u8; 3]); 5] = [
        (0.35, [0x3f, 0xb9, 0x50]),
        (0.55, [0x3f, 0xb9, 0x50]),
        (0.80, [0xe3, 0xb3, 0x41]),
        (0.62, [0x3f, 0xb9, 0x50]),
        (0.42, [0x3f, 0xb9, 0x50]),
    ];
    let inner_l = s * 0.20;
    let inner_r = s * 0.80;
    let bottom = s * 0.78;
    let max_h = s * 0.56;
    let slot = (inner_r - inner_l) / bars.len() as f32;
    let bar_w = slot * 0.62;

    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Coverage of the rounded tile (with 1px anti-aliasing).
            let tile = rounded_rect_coverage(fx, fy, margin, margin, s - margin, s - margin, radius);
            if tile <= 0.0 {
                continue;
            }
            // Vertical gradient background.
            let t = fy / s;
            let mut c = [lerp(0x2a, 0x16, t), lerp(0x30, 0x1a, t), lerp(0x3c, 0x22, t)];
            for (i, (h, color)) in bars.iter().enumerate() {
                let x0 = inner_l + slot * i as f32 + (slot - bar_w) / 2.0;
                let top = bottom - max_h * h;
                let cov = rounded_rect_coverage(fx, fy, x0, top, x0 + bar_w, bottom, bar_w * 0.25);
                if cov > 0.0 {
                    for k in 0..3 {
                        c[k] = c[k] * (1.0 - cov) + color[k] as f32 * cov;
                    }
                }
            }
            let i = (y * size + x) as usize * 4;
            px[i] = c[0].round() as u8;
            px[i + 1] = c[1].round() as u8;
            px[i + 2] = c[2].round() as u8;
            px[i + 3] = (tile * 255.0).round() as u8;
        }
    }
    px
}

fn lerp(a: u8, b: u8, t: f32) -> f32 {
    a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0)
}

/// Approximate anti-aliased coverage of a pixel center by a rounded rectangle.
fn rounded_rect_coverage(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r;
    (0.5 - d).clamp(0.0, 1.0)
}

pub fn egui_icon() -> eframe::egui::IconData {
    let size = 256;
    eframe::egui::IconData { rgba: rgba(size), width: size, height: size }
}

pub fn write_png(path: &std::path::Path, size: u32) -> anyhow::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, size, size);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgba(size))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_has_transparent_corners_and_opaque_center() {
        let size = 64;
        let px = super::rgba(size);
        assert_eq!(px.len(), (size * size * 4) as usize);
        assert_eq!(px[3], 0, "top-left corner is transparent");
        let c = ((size / 2 * size + size / 2) * 4) as usize;
        assert_eq!(px[c + 3], 255, "center is opaque");
    }
}
