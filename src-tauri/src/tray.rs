use tauri::image::Image;

const SIZE: u32 = 32;

/// 5x7 pixel font, one bitmask row per line (bit 4 = leftmost pixel).
fn glyph(c: char) -> [u8; 7] {
    match c {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        _ => [0; 7],
    }
}

fn text_width(text: &str, scale: u32) -> u32 {
    if text.is_empty() {
        return 0;
    }
    (text.chars().count() as u32 * 6 - 1) * scale
}

fn draw_text(buf: &mut [u8], text: &str, x0: i32, y0: i32, scale: u32, rgba: [u8; 4]) {
    let mut x = x0;
    for c in text.chars() {
        let g = glyph(c);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..5u32 {
                if bits & (1 << (4 - col)) != 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = x + (col * scale + sx) as i32;
                            let py = y0 + (row as u32 * scale + sy) as i32;
                            if px >= 0 && py >= 0 && (px as u32) < SIZE && (py as u32) < SIZE {
                                let idx = ((py as u32 * SIZE + px as u32) * 4) as usize;
                                buf[idx..idx + 4].copy_from_slice(&rgba);
                            }
                        }
                    }
                }
            }
        }
        x += (6 * scale) as i32;
    }
}

/// Render a "done over total" fraction badge, e.g. 6.2 over /8.
pub fn render_fraction_icon(done: &str, total: &str) -> Image<'static> {
    let mut buf = vec![0u8; (SIZE * SIZE * 4) as usize];

    // rounded dark badge background so white text is visible on any taskbar theme
    for y in 0..SIZE {
        for x in 0..SIZE {
            let corner = |cx: i64, cy: i64| -> bool {
                let dx = x as i64 - cx;
                let dy = y as i64 - cy;
                dx * dx + dy * dy <= 49
            };
            let r = 7i64;
            let inside = if x < 7 && y < 7 {
                corner(r, r)
            } else if x >= SIZE - 7 && y < 7 {
                corner(SIZE as i64 - 1 - r, r)
            } else if x < 7 && y >= SIZE - 7 {
                corner(r, SIZE as i64 - 1 - r)
            } else if x >= SIZE - 7 && y >= SIZE - 7 {
                corner(SIZE as i64 - 1 - r, SIZE as i64 - 1 - r)
            } else {
                true
            };
            if inside {
                let idx = ((y * SIZE + x) * 4) as usize;
                buf[idx..idx + 4].copy_from_slice(&[22, 24, 34, 235]);
            }
        }
    }

    let scale_for = |t: &str| -> u32 {
        let mut s = 2;
        while s > 1 && text_width(t, s) > SIZE - 2 {
            s -= 1;
        }
        s
    };

    let s1 = scale_for(done);
    let s2 = scale_for(total);
    let h1 = 7 * s1;
    let h2 = 7 * s2;
    // layout: top text, divider, bottom text — vertically centered
    let content_h = h1 + 3 + h2;
    let top = ((SIZE.saturating_sub(content_h)) / 2) as i32;

    let x1 = ((SIZE - text_width(done, s1).min(SIZE)) / 2) as i32;
    draw_text(&mut buf, done, x1, top, s1, [255, 255, 255, 255]);

    // divider line
    let dy = top + h1 as i32 + 1;
    if dy >= 0 && (dy as u32) < SIZE {
        for x in 6..SIZE - 6 {
            let idx = ((dy as u32 * SIZE + x) * 4) as usize;
            buf[idx..idx + 4].copy_from_slice(&[140, 150, 180, 255]);
        }
    }

    let x2 = ((SIZE - text_width(total, s2).min(SIZE)) / 2) as i32;
    draw_text(&mut buf, total, x2, dy + 2, s2, [170, 200, 255, 255]);

    Image::new_owned(buf, SIZE, SIZE)
}

/// Format seconds as compact hours: "6", "6.2"
pub fn fmt_hours(secs: u64) -> String {
    let h = secs as f64 / 3600.0;
    let rounded = (h * 10.0).round() / 10.0;
    if (rounded - rounded.trunc()).abs() < 0.05 {
        format!("{}", rounded.trunc() as u64)
    } else {
        format!("{:.1}", rounded)
    }
}

pub fn fmt_target(hours: f64) -> String {
    if (hours - hours.trunc()).abs() < 0.01 {
        format!("{}", hours.trunc() as u64)
    } else {
        format!("{:.1}", hours)
    }
}
