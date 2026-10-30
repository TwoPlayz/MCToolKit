//! Renders the app's box logo mark on a blue badge at high resolution with tiny-skia, for
//! producing the bundled window/taskbar/exe icon (matches `icons.rs`'s `Icon::Logo`).
//!
//! `cargo run -p mctk-gui --example gen_logo -- <out dir>` writes `logo-<size>.png` for a
//! few sizes plus the master `logo.png` (1024px).

use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Badge blue (`theme::BLUE`).
const BLUE: (u8, u8, u8) = (0x3D, 0x7F, 0xE0);

fn draw(size: u32) -> Pixmap {
    let mut pm = Pixmap::new(size, size).expect("pixmap");
    let s = size as f32;

    // Rounded-square badge.
    let mut badge = Paint::default();
    badge.set_color_rgba8(BLUE.0, BLUE.1, BLUE.2, 255);
    badge.anti_alias = true;
    let r = s * 0.16;
    let rect = tiny_skia::Rect::from_xywh(0.0, 0.0, s, s).unwrap();
    let path = rounded_rect(rect, r);
    pm.fill_path(
        &path,
        &badge,
        FillRule::Winding,
        Transform::identity(),
        None,
    );

    // The box mark: a hexagon outline (an open box viewed in pseudo-3D) with a chevron
    // for the opened lid and a vertical line for the front edge. Same relative stroke
    // width (7.5% of the icon size) as the in-app `Pen::w()` uses for this icon.
    let mut white = Paint::default();
    white.set_color_rgba8(255, 255, 255, 255);
    white.anti_alias = true;
    let stroke = Stroke {
        width: s * 0.075,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    };

    let at = |x: f32, y: f32| (x * s, y * s);

    let mut hex = PathBuilder::new();
    let pts = [
        (0.5, 0.2),
        (0.78, 0.36),
        (0.78, 0.68),
        (0.5, 0.84),
        (0.22, 0.68),
        (0.22, 0.36),
    ];
    let (x0, y0) = at(pts[0].0, pts[0].1);
    hex.move_to(x0, y0);
    for &(x, y) in &pts[1..] {
        let (px, py) = at(x, y);
        hex.line_to(px, py);
    }
    hex.close();
    pm.stroke_path(
        &hex.finish().unwrap(),
        &white,
        &stroke,
        Transform::identity(),
        None,
    );

    let mut chevron = PathBuilder::new();
    let (cx0, cy0) = at(0.22, 0.36);
    let (cx1, cy1) = at(0.5, 0.52);
    let (cx2, cy2) = at(0.78, 0.36);
    chevron.move_to(cx0, cy0);
    chevron.line_to(cx1, cy1);
    chevron.line_to(cx2, cy2);
    pm.stroke_path(
        &chevron.finish().unwrap(),
        &white,
        &stroke,
        Transform::identity(),
        None,
    );

    let mut edge = PathBuilder::new();
    let (ex0, ey0) = at(0.5, 0.52);
    let (ex1, ey1) = at(0.5, 0.84);
    edge.move_to(ex0, ey0);
    edge.line_to(ex1, ey1);
    pm.stroke_path(
        &edge.finish().unwrap(),
        &white,
        &stroke,
        Transform::identity(),
        None,
    );

    pm
}

fn rounded_rect(rect: tiny_skia::Rect, radius: f32) -> tiny_skia::Path {
    let rr = tiny_skia::Rect::from_xywh(
        rect.x() + radius,
        rect.y(),
        rect.width() - 2.0 * radius,
        rect.height(),
    );
    let _ = rr;
    // tiny-skia has no built-in rounded-rect path builder; approximate corners with
    // short bezier-free arcs via quad curves, good enough at the sizes we render.
    let mut pb = PathBuilder::new();
    let (x, y, w, h) = (rect.x(), rect.y(), rect.width(), rect.height());
    let r = radius;
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish().unwrap()
}

/// Packs same-order PNG byte buffers into a Windows .ico (each entry's image data is a
/// full PNG, which Vista+ accepts at any size — simpler and sharper than raw DIB entries).
fn write_ico(sizes_and_pngs: &[(u32, Vec<u8>)], path: &str) {
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&(sizes_and_pngs.len() as u16).to_le_bytes());
    let header_len = 6 + 16 * sizes_and_pngs.len();
    let mut offset = header_len as u32;
    for (size, png) in sizes_and_pngs {
        let side = if *size >= 256 { 0u8 } else { *size as u8 };
        out.push(side); // width
        out.push(side); // height
        out.push(0); // color count
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in sizes_and_pngs {
        out.extend_from_slice(png);
    }
    std::fs::write(path, out).unwrap();
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    std::fs::create_dir_all(&out).unwrap();
    let sizes = [256, 128, 64, 48, 32, 16];
    let mut pngs = Vec::new();
    for size in sizes {
        let pm = draw(size);
        pngs.push((size, pm.encode_png().unwrap()));
    }
    std::fs::write(format!("{out}/logo.png"), &pngs[0].1).unwrap();
    println!("wrote {out}/logo.png ({}x{})", pngs[0].0, pngs[0].0);
    write_ico(&pngs, &format!("{out}/logo.ico"));
    println!("wrote {out}/logo.ico ({} sizes)", pngs.len());
}
