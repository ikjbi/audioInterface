//! The app icon, drawn in code: seven level-meter bars, coloured green →
//! yellow → red by height like a real meter, on a deep navy rounded tile.
//! No binary assets are needed for the window/taskbar icon on any OS.
//! `audio-interface export-icon` writes PNG/ICO files for packaging.

/// Bar heights (fraction of the maximum) for the static icon.
pub const BARS: [f32; 7] = [0.30, 0.52, 0.76, 1.0, 0.70, 0.48, 0.28];

pub const GREEN: [u8; 3] = [0x3f, 0xd1, 0x6b];
pub const YELLOW: [u8; 3] = [0xf2, 0xc1, 0x3d];
pub const RED: [u8; 3] = [0xff, 0x55, 0x5a];

/// Meter colour at a height fraction (0 = bottom, 1 = top).
pub fn meter_color(frac: f32) -> [u8; 3] {
    let lerp = |a: [u8; 3], b: [u8; 3], t: f32| -> [u8; 3] {
        std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t.clamp(0.0, 1.0)).round() as u8)
    };
    if frac < 0.55 {
        GREEN
    } else if frac < 0.80 {
        lerp(GREEN, YELLOW, (frac - 0.55) / 0.25)
    } else {
        lerp(YELLOW, RED, (frac - 0.80) / 0.20)
    }
}

/// Render the icon as straight (non-premultiplied) RGBA8, `size`×`size`.
pub fn rgba(size: u32) -> Vec<u8> {
    render(size, &BARS)
}

/// Render with custom bar heights (each 0..=1).
pub fn render(size: u32, bars: &[f32]) -> Vec<u8> {
    let s = size as f32;
    let mut px = vec![0u8; (size * size * 4) as usize];
    let margin = s * 0.03;
    let radius = s * 0.225;
    let inner_l = s * 0.17;
    let inner_r = s * 0.83;
    let bottom = s * 0.78;
    let max_h = s * 0.56;
    let n = bars.len().max(1) as f32;
    let slot = (inner_r - inner_l) / n;
    let bar_w = slot * 0.64;

    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let tile = rounded_rect_coverage(fx, fy, margin, margin, s - margin, s - margin, radius);
            if tile <= 0.0 {
                continue;
            }
            // Navy gradient with a soft highlight in the top-left.
            let t = fy / s;
            let mut c = [lerp(0x22, 0x0c, t), lerp(0x2c, 0x11, t), lerp(0x4a, 0x24, t)];
            let hx = (fx - s * 0.3) / s;
            let hy = (fy - s * 0.2) / s;
            let glow = (1.0 - (hx * hx + hy * hy).sqrt() * 2.2).max(0.0) * 22.0;
            for v in &mut c {
                *v = (*v + glow).min(255.0);
            }
            // Faint baseline under the bars.
            let base =
                rounded_rect_coverage(fx, fy, inner_l, bottom + s * 0.045, inner_r, bottom + s * 0.06, s * 0.008);
            if base > 0.0 {
                for v in &mut c {
                    *v = *v * (1.0 - base * 0.35) + 255.0 * base * 0.35;
                }
            }
            for (i, h) in bars.iter().enumerate() {
                let x0 = inner_l + slot * i as f32 + (slot - bar_w) / 2.0;
                let top = bottom - max_h * h.clamp(0.02, 1.0);
                let cov = rounded_rect_coverage(fx, fy, x0, top, x0 + bar_w, bottom, bar_w * 0.32);
                if cov > 0.0 {
                    let col = meter_color((bottom - fy) / max_h);
                    for k in 0..3 {
                        c[k] = c[k] * (1.0 - cov) + col[k] as f32 * cov;
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
    let r = r.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r;
    (0.5 - d).clamp(0.0, 1.0)
}

pub fn egui_icon() -> eframe::egui::IconData {
    let size = 256;
    eframe::egui::IconData { rgba: rgba(size), width: size, height: size }
}

fn png_bytes(size: u32) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, size, size);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&rgba(size))?;
    }
    Ok(out)
}

pub fn write_png(path: &std::path::Path, size: u32) -> anyhow::Result<()> {
    std::fs::write(path, png_bytes(size)?)?;
    Ok(())
}

/// Windows `.ico` with PNG-compressed images (16–256 px), supported since Vista.
pub fn write_ico(path: &std::path::Path) -> anyhow::Result<()> {
    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| png_bytes(s)).collect::<anyhow::Result<_>>()?;
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    std::fs::write(path, out)?;
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

    #[test]
    fn meter_colors_go_green_yellow_red() {
        assert_eq!(super::meter_color(0.1), super::GREEN);
        assert_eq!(super::meter_color(0.8), super::YELLOW);
        assert_eq!(super::meter_color(1.0), super::RED);
    }

    #[test]
    fn ico_header_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("i.ico");
        super::write_ico(&p).unwrap();
        let b = std::fs::read(&p).unwrap();
        assert_eq!(&b[..4], &[0, 0, 1, 0]);
        assert_eq!(u16::from_le_bytes([b[4], b[5]]), 7);
        // First image is a PNG at the declared offset.
        let off = u32::from_le_bytes([b[18], b[19], b[20], b[21]]) as usize;
        assert_eq!(&b[off..off + 4], b"\x89PNG");
    }
}
