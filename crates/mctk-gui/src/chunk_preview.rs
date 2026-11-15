//! The little map of converted chunks on the conversion page.
//!
//! The picture is worked out as plain rectangles so it can be drawn by egui and, in the test,
//! by a few lines of code into an image to look at. It is a square window onto the world, a
//! power of two chunks across (8, 16, 32...) and placed on a grid of half its own size, so it
//! can sit across the world's centre. The window is the smallest one that holds what has been
//! converted so far: it stays where it is while chunks fill in, and only changes (to the next
//! size up, or half a window over) when conversion reaches beyond it.

use eframe::egui::{Color32, Pos2, Rect, Vec2, pos2};

/// How many of the newest chunks glow.
const GLOW: usize = 40;
/// The smallest window, in chunks.
const SMALLEST: i32 = 8;

pub struct Colors {
    pub frame: Color32,
    pub chunk: Color32,
    pub fresh: Color32,
}

/// The window (its first chunk on each axis and its width in chunks) that holds the box
/// `(x0, z0)` to `(x1, z1)`.
fn window(x0: i32, x1: i32, z0: i32, z1: i32) -> (i32, i32, i32) {
    let mut width = SMALLEST;
    loop {
        let half = width / 2;
        let (ox, oz) = (x0.div_euclid(half) * half, z0.div_euclid(half) * half);
        if x1 < ox + width && z1 < oz + width || width >= 1 << 24 {
            return (ox, oz, width);
        }
        width *= 2;
    }
}

/// The rectangles of a `size` x `size` picture of `chunks` (in the order they were written),
/// relative to its top-left corner, back to front: the frame first. `running` makes the newest
/// chunks glow.
pub fn cells(chunks: &[(i32, i32)], size: f32, running: bool, colors: &Colors) -> Vec<(Rect, Color32)> {
    let mut out = vec![(Rect::from_min_size(Pos2::ZERO, Vec2::splat(size)), colors.frame)];
    if chunks.is_empty() {
        return out;
    }
    let (mut x0, mut x1, mut z0, mut z1) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for &(x, z) in chunks {
        x0 = x0.min(x);
        x1 = x1.max(x);
        z0 = z0.min(z);
        z1 = z1.max(z);
    }
    let (ax, az, width) = window(x0, x1, z0, z1);
    let scale = size / width as f32;
    // A chunk covers whole pixels, however the scale divides.
    let edge = |v: i32, origin: i32| ((v - origin) as f32 * scale).round();
    let clip = Rect::from_min_size(Pos2::ZERO, Vec2::splat(size));
    // Chunks are drawn a pixel short of their cell once the cells are big enough, so they read
    // as separate squares.
    let gap = if scale >= 6.0 { 1.0 } else { 0.0 };
    let newest = chunks.len().saturating_sub(GLOW);
    for (i, &(x, z)) in chunks.iter().enumerate() {
        let colour = if running && i >= newest {
            let age = (chunks.len() - i) as f32 / GLOW as f32;
            colors.fresh.lerp_to_gamma(colors.chunk, age.clamp(0.0, 1.0))
        } else {
            // A slight difference from chunk to chunk, so a finished region reads as a mosaic
            // and not one flat colour.
            let h = ((x.wrapping_mul(73_856_093) ^ z.wrapping_mul(19_349_663)) & 0xff) as f32 / 255.0;
            colors.chunk.gamma_multiply(0.9 + 0.1 * h)
        };
        let (left, top) = (edge(x, ax), edge(z, az));
        let right = (edge(x + 1, ax) - gap).max(left + 1.0);
        let bottom = (edge(z + 1, az) - gap).max(top + 1.0);
        let rect = Rect::from_min_max(pos2(left, top), pos2(right, bottom)).intersect(clip);
        if rect.is_positive() {
            out.push((rect, colour));
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    const COLORS: Colors = Colors {
        frame: Color32::from_rgb(0x0F, 0x11, 0x16),
        chunk: Color32::from_rgb(0x3B, 0x82, 0xE6),
        fresh: Color32::from_rgb(0xE6, 0x9A, 0x2E),
    };

    /// Chunks of a console-sized world as a converter writes them: region by region, row by row.
    fn world(extent: i32) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        let half = extent / 2;
        for rz in -1..=0 {
            for rx in -1..=0 {
                for z in 0..32 {
                    for x in 0..32 {
                        let (cx, cz) = (rx * 32 + x, rz * 32 + z);
                        let (dx, dz) = (cx as f32 + 0.5, cz as f32 + 0.5);
                        // A rounded island, not a square.
                        if cx >= -half && cx < half && cz >= -half && cz < half && dx * dx + dz * dz < (half * half) as f32 * 1.15 {
                            out.push((cx, cz));
                        }
                    }
                }
            }
        }
        out
    }

    fn draw(shapes: &[(Rect, Color32)], size: usize, into: &mut image::RgbaImage, at: (u32, u32)) {
        for (rect, c) in shapes {
            let (x0, x1) = (rect.left().round() as i64, rect.right().round().max(rect.left().round() + 1.0) as i64);
            let (y0, y1) = (rect.top().round() as i64, rect.bottom().round().max(rect.top().round() + 1.0) as i64);
            for y in y0.max(0)..y1.min(size as i64) {
                for x in x0.max(0)..x1.min(size as i64) {
                    let px = into.get_pixel_mut(at.0 + x as u32, at.1 + y as u32);
                    let a = c.a() as f32 / 255.0;
                    for k in 0..3 {
                        px.0[k] = (px.0[k] as f32 * (1.0 - a) + c.to_array()[k] as f32 * a) as u8;
                    }
                }
            }
        }
    }

    /// The area the chunks cover, as a share of the frame.
    fn filled(shapes: &[(Rect, Color32)], size: f32) -> f32 {
        let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for (r, _) in shapes.iter().skip(1) {
            x0 = x0.min(r.left());
            x1 = x1.max(r.right());
            y0 = y0.min(r.top());
            y1 = y1.max(r.bottom());
        }
        ((x1 - x0).max(y1 - y0)) / size
    }

    /// Writes strips of the picture at several points of a conversion to
    /// `$MCTK_PREVIEW_PNG`, to look at: a console-sized world and a small one.
    #[test]
    fn preview_pictures() {
        let size = 96usize;
        let big = world(54);
        let small: Vec<(i32, i32)> = (-5..5).flat_map(|z| (-5..5).map(move |x| (x, z))).collect();
        let steps = [0.03, 0.15, 0.4, 0.7, 1.0];
        let scale = 3u32;
        let pad = 8u32;
        let width = (size as u32 + pad) * steps.len() as u32 * 2 + pad * 2;
        let mut img = image::RgbaImage::from_pixel(width, size as u32 + 2 * pad, image::Rgba([0x1B, 0x1E, 0x24, 255]));
        for (row, all) in [&big, &small].into_iter().enumerate() {
            for (n, f) in steps.iter().enumerate() {
                let upto = ((all.len() as f32 * f) as usize).max(1);
                let shapes = cells(&all[..upto], size as f32, *f < 1.0, &COLORS);
                assert!(shapes.iter().all(|(r, _)| r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= size as f32 && r.max.y <= size as f32));
                let at = pad + (row * steps.len() + n) as u32 * (size as u32 + pad) + row as u32 * pad;
                draw(&shapes, size, &mut img, (at, pad));
            }
        }
        let img = image::imageops::resize(&img, img.width() * scale, img.height() * scale, image::imageops::FilterType::Nearest);
        if let Ok(path) = std::env::var("MCTK_PREVIEW_PNG") {
            img.save(path).expect("write the preview");
        }
    }

    #[test]
    fn a_finished_world_fills_the_frame_whatever_its_size() {
        let size = 96.0;
        let small: Vec<(i32, i32)> = (-5..5).flat_map(|z| (-5..5).map(move |x| (x, z))).collect();
        assert!(filled(&cells(&small, size, false, &COLORS), size) > 0.55);
        assert!(filled(&cells(&world(54), size, false, &COLORS), size) > 0.75);
        // One chunk is a square, not a speck.
        assert!(filled(&cells(&[(40, -7)], size, false, &COLORS), size) >= 0.1);
    }

    #[test]
    fn the_window_stays_put_while_chunks_fill_it() {
        let all = world(54);
        let mut last: Option<(f32, Vec<Rect>)> = None;
        let mut changes = 0;
        for upto in (1..=all.len()).step_by(3) {
            let shapes = cells(&all[..upto], 96.0, false, &COLORS);
            assert!(shapes.iter().all(|(r, _)| r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= 96.0 && r.max.y <= 96.0));
            // The first chunk written is the first rectangle after the frame; it only moves when
            // the window does.
            let first = shapes[1].0;
            if let Some((_, prev)) = &last {
                if prev[0] != first {
                    changes += 1;
                }
            }
            last = Some((0.0, vec![first]));
        }
        // From 8 chunks across to 64 is at most a few doublings, not a change on every frame.
        assert!(changes <= 8, "{changes}");
    }
}