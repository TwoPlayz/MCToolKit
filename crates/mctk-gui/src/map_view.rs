//! The world map used by the Chunk Locator and the Pruner: rulers, pan and zoom, a
//! chunk/block tooltip, the Locate bar, dimension buttons and the Slicer.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use eframe::egui::{
    self, Align2, Color32, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2, pos2, vec2,
};

use crate::icons::{self, Icon};
use crate::jobs::Job;
use crate::theme;
use crate::widgets;
use crate::world::{self, Edition, MapImage, MapProgress};

const RULER: f32 = 20.0;
const DIMS: [&str; 3] = [
    "minecraft:overworld",
    "minecraft:the_nether",
    "minecraft:the_end",
];

pub struct MapView {
    pub path: PathBuf,
    pub edition: Edition,
    pub dimension: String,
    image: Option<(MapImage, egui::TextureHandle)>,
    /// The picture shrunk by whole numbers (2, 3, 4, 6, 8, ... 128) with each pixel the average of
    /// what it covers, built when first wanted: the one whose pixels are nearest a screen pixel is
    /// drawn when zoomed out, so it stays sharp instead of blurring.
    /// Zoomed-out tiles already built, by shrink and tile position (see `lod_tile`), and the shrink
    /// last drawn (its tiles stand in while the new ones are built).
    lods: std::collections::HashMap<(usize, usize, usize), egui::TextureHandle>,
    last_f: usize,
    /// How much smaller the locate bar, dimension buttons and slicer are drawn (narrow window).
    ui_k: f32,
    /// The pictures of blocks, for drawing them in detail when zoomed in far.
    block_tex: std::collections::HashMap<u16, Option<BlockTex>>,
    job: Option<Job<MapImage>>,
    progress: Option<Arc<MapProgress>>,
    started: Option<Instant>,
    stream_dirty: bool,
    last_upload: Option<Instant>,
    error: Option<String>,
    /// Camera centre in block coordinates and scale in pixels per block.
    pub center: Vec2,
    pub zoom: f32,
    fitted: bool,
    chunk_mode: bool,
    locate: (f64, f64),
    slicer_open: bool,
    /// The Slicer's upper and lower limits; `applied_slicer` is what the map was last drawn with.
    slicer: i32,
    slicer_min: i32,
    /// The handle being dragged: `true` for the upper one.
    slicer_grab: Option<bool>,
    /// The pointer was over the Locate bar, dimension buttons or Slicer last frame, so the map
    /// under them shows no hovered chunk.
    over_controls: bool,
    applied_slicer: Option<(i32, i32)>,
    /// The chunks picked in the Chunk Locator: click one, Ctrl+click to add or remove, or
    /// Shift+drag a box.
    pub selected: Vec<(i32, i32)>,
    /// The Shift+drag box being drawn, in block coordinates (where it began, where it is now).
    marquee: Option<(Vec2, Vec2)>,
    /// Whether a click picks a chunk: the Chunk Locator does, the Pruner does not.
    pub selectable: bool,
    /// Set by a tool that has the pointer on one of its handles: no chunk is outlined under it.
    pub hide_hover: bool,
}

/// Where the map was drawn this frame, for overlays and tools.
pub struct MapFrame {
    pub rect: Rect,
    pub center: Vec2,
    pub zoom: f32,
    pub response: Response,
    /// Block under the pointer.
    pub hover: Option<Vec2>,
}

impl MapFrame {
    pub fn to_screen(&self, b: Vec2) -> Pos2 {
        self.rect.center() + (b - self.center) * self.zoom
    }
    pub fn to_block(&self, p: Pos2) -> Vec2 {
        self.center + (p - self.rect.center()) / self.zoom
    }
}

/// A block's picture (and smaller copies of it) with its average colour.
struct BlockTex {
    /// The picture's side in pixels, and the picture not smoothed (for when it is bigger on screen).
    side: u32,
    nearest: egui::TextureHandle,
    /// The picture and its halves, quarters... down to one pixel, smoothed (for when it is smaller
    /// on screen), biggest first.
    mips: Vec<(u32, egui::TextureHandle)>,
    avg: [f32; 3],
    /// The colour the flat map draws the block in, which the texture is tinted toward.
    flat: [f32; 3],
}

impl BlockTex {
    /// The texture that suits a block drawn `cell` pixels wide.
    fn pick(&self, cell: f32) -> egui::TextureId {
        if cell >= self.side as f32 {
            return self.nearest.id();
        }
        // The smallest copy still at least as big as the block on screen.
        self.mips
            .iter()
            .rev()
            .find(|(s, _)| *s as f32 >= cell)
            .map_or(self.nearest.id(), |(_, t)| t.id())
    }
}

/// The picture for a block on the map (its top, from the resource pack), loaded once. None for
/// blocks the pack has no picture of.
fn block_texture<'a>(
    cache: &'a mut std::collections::HashMap<u16, Option<BlockTex>>,
    ctx: &egui::Context,
    id: u16,
) -> Option<&'a BlockTex> {
    cache
        .entry(id)
        .or_insert_with(|| {
            let name = world::block_name(id)?;
            // "oak_log#x" is a log lying along x: its side seen from above, turned to run that way.
            let (base, axis) = name.split('@').next().unwrap_or(&name).split_once('#').map_or((name.split('@').next().unwrap_or(&name), None), |(b, a)| (b, Some(a)));
            let files: Vec<String> = if base == "water" {
                vec!["block/water_still.png".into()]
            } else {
                world::texture_files(base, axis.is_some())
            };
            let file = files.iter().find_map(|f| crate::icons::pack_texture(f))?;
            let img = image::open(file).ok()?.to_rgba8();
            let w = img.width();
            // An animated texture is a tall strip of frames; the first is the picture.
            let h = img.height().min(w);
            let mut pic = image::imageops::crop_imm(&img, 0, 0, w, h).to_image();
            if axis == Some("x") {
                pic = image::imageops::rotate90(&pic);
            }
            let (mut sum, mut n) = ([0f64; 3], 0f64);
            for p in pic.pixels().filter(|p| p[3] > 128) {
                for k in 0..3 {
                    sum[k] += p[k] as f64;
                }
                n += 1.0;
            }
            let avg = if n > 0.0 { [sum[0] / n, sum[1] / n, sum[2] / n] } else { return None };
            let upload = |im: &image::RgbaImage, opts: egui::TextureOptions, tag: &str| {
                ctx.load_texture(
                    format!("block-{name}-{tag}"),
                    egui::ColorImage::from_rgba_unmultiplied([im.width() as usize, im.height() as usize], im.as_raw()),
                    opts,
                )
            };
            let nearest = upload(&pic, egui::TextureOptions::NEAREST, "n");
            // Each smaller copy averages 2 x 2 of the one before.
            let mut mips = Vec::new();
            let mut cur = pic.clone();
            loop {
                mips.push((cur.width(), upload(&cur, egui::TextureOptions::LINEAR, &format!("m{}", cur.width()))));
                if cur.width() <= 1 {
                    break;
                }
                cur = image::imageops::resize(&cur, (cur.width() / 2).max(1), (cur.height() / 2).max(1), image::imageops::FilterType::Triangle);
            }
            let flat = world::surface_colour(&name).map(|c| c as f32);
            Some(BlockTex {
                side: pic.width(),
                nearest,
                mips,
                avg: [avg[0] as f32, avg[1] as f32, avg[2] as f32],
                flat,
            })
        })
        .as_ref()
}
/// Cells across and down one zoomed-out tile.
const LOD_TILE: usize = 96;

/// One tile of the picture shrunk by `f` in both directions (tile `tx`, `ty` of `LOD_TILE` cells
/// square), each pixel standing for a block of columns: their average colour and height, lit
/// again from the heights so the lie of the land stays readable (and not left to average out), as
/// a texture that is not smoothed further. From 16 (a chunk) up the cells are a flat mosaic with
/// no relief. Tiles are built a few at a time, nearest the view first, so zooming never stalls.
fn lod_tile(ctx: &egui::Context, img: &MapImage, f: usize, tx: usize, ty: usize) -> egui::TextureHandle {
    let (nw, nh) = (img.width.div_ceil(f), img.height.div_ceil(f));
    let (cx0, cy0) = (tx * LOD_TILE, ty * LOD_TILE);
    let (cx1, cy1) = ((cx0 + LOD_TILE).min(nw), (cy0 + LOD_TILE).min(nh));
    // One cell more above and to the left, for the lighting of the first row and column.
    let (bx0, by0) = (cx0.saturating_sub(1), cy0.saturating_sub(1));
    let gw = cx1 - bx0;
    // Per cell: columns drawn, summed colour, summed height, water columns and their summed depth.
    type Cell = ([u32; 3], u32, u64, u32, u32);
    let mut cells: Vec<Cell> = vec![Default::default(); gw * (cy1 - by0)];
    for sy in by0 * f..(cy1 * f).min(img.height) {
        for sx in bx0 * f..(cx1 * f).min(img.width) {
            let i = sy * img.width + sx;
            if img.blocks[i] == 0 {
                continue;
            }
            let c = &mut cells[(sy / f - by0) * gw + sx / f - bx0];
            for k in 0..3 {
                c.0[k] += img.albedo[i * 3 + k] as u32;
            }
            c.1 += 1;
            c.2 = c.2.wrapping_add((img.heights[i] as i64 + 4096) as u64);
            if img.water[i] > 0 {
                c.3 += 1;
                c.4 += img.water[i] as u32;
            }
        }
    }
    let avg_y = |c: &Cell| (c.2 / c.1 as u64) as i32 - 4096;
    let soften = 2.0 / (f as f32).sqrt();
    let (w, h) = (cx1 - cx0, cy1 - cy0);
    let mut out = vec![0u8; w * h * 4];
    for y in cy0..cy1 {
        for x in cx0..cx1 {
            let c = &cells[(y - by0) * gw + (x - bx0)];
            if c.1 == 0 {
                continue;
            }
            let alb = [0, 1, 2].map(|k| (c.0[k] / c.1) as u8);
            let hy = avg_y(c);
            let n = (y > 0 && cells[(y - 1 - by0) * gw + (x - bx0)].1 > 0).then(|| avg_y(&cells[(y - 1 - by0) * gw + (x - bx0)]));
            let wst = (x > 0 && cells[(y - by0) * gw + (x - 1 - bx0)].1 > 0).then(|| avg_y(&cells[(y - by0) * gw + (x - 1 - bx0)]));
            // From a chunk upward each cell is one flat colour, a mosaic of chunks as in UMT.
            let land = if f >= 16 { alb } else { world::lit(alb, hy, 0, n, wst, soften) };
            // The more of the cell is water, the more it takes the colour of water of that depth.
            let share = c.3 as f32 / c.1 as f32;
            let wet = if c.3 > 0 { world::lit(alb, hy, (c.4 / c.3) as i32, None, None, 1.0) } else { land };
            let o = ((y - cy0) * w + (x - cx0)) * 4;
            for k in 0..3 {
                out[o + k] = (land[k] as f32 + (wet[k] as f32 - land[k] as f32) * share) as u8;
            }
            out[o + 3] = 255;
        }
    }
    ctx.load_texture(
        format!("world-map-lod{f}-{tx}-{ty}"),
        egui::ColorImage::from_rgba_unmultiplied([w, h], &out),
        egui::TextureOptions::NEAREST,
    )
}
pub(crate) fn dim_tile(p: &egui::Painter, r: Rect, dim: usize) {
    let (top, side, tint) = match dim {
        0 => (
            "block/grass_block_top.png",
            "block/grass_block_side.png",
            Color32::from_rgb(0x91, 0xBD, 0x59),
        ),
        1 => (
            "block/netherrack.png",
            "block/netherrack.png",
            Color32::WHITE,
        ),
        _ => ("block/end_stone.png", "block/end_stone.png", Color32::WHITE),
    };
    if icons::block(p, r, top, side, tint) {
        return;
    }
    let (top, side, dots): (Color32, Color32, Color32) = match dim {
        0 => (
            Color32::from_rgb(0x5E, 0x9D, 0x3A),
            Color32::from_rgb(0x86, 0x60, 0x43),
            Color32::from_rgb(0x6D, 0x4A, 0x30),
        ),
        1 => (
            Color32::from_rgb(0x7A, 0x2A, 0x2A),
            Color32::from_rgb(0x6A, 0x22, 0x24),
            Color32::from_rgb(0x9A, 0x3A, 0x3A),
        ),
        _ => (
            Color32::from_rgb(0xE6, 0xE2, 0xA8),
            Color32::from_rgb(0xD7, 0xD3, 0x95),
            Color32::from_rgb(0xBE, 0xB9, 0x7C),
        ),
    };
    // A small isometric block of our own drawing.
    let c = r.center();
    let s = r.width() * 0.42;
    let t = [
        pos2(c.x, c.y - s * 0.9),
        pos2(c.x + s, c.y - s * 0.4),
        pos2(c.x, c.y + s * 0.1),
        pos2(c.x - s, c.y - s * 0.4),
    ];
    let l = [
        pos2(c.x - s, c.y - s * 0.4),
        pos2(c.x, c.y + s * 0.1),
        pos2(c.x, c.y + s),
        pos2(c.x - s, c.y + s * 0.5),
    ];
    let rr = [
        pos2(c.x, c.y + s * 0.1),
        pos2(c.x + s, c.y - s * 0.4),
        pos2(c.x + s, c.y + s * 0.5),
        pos2(c.x, c.y + s),
    ];
    p.add(egui::Shape::convex_polygon(t.to_vec(), top, Stroke::NONE));
    p.add(egui::Shape::convex_polygon(l.to_vec(), side, Stroke::NONE));
    p.add(egui::Shape::convex_polygon(
        rr.to_vec(),
        side.gamma_multiply(0.8),
        Stroke::NONE,
    ));
    for (dx, dy) in [(-0.4, 0.4), (0.3, 0.55), (-0.15, 0.75), (0.55, 0.2)] {
        p.circle_filled(pos2(c.x + dx * s, c.y + dy * s), s * 0.07, dots);
    }
}

impl MapView {
    pub fn new(path: PathBuf, edition: Edition) -> Self {
        MapView {
            path,
            edition,
            dimension: DIMS[0].to_string(),
            image: None,
            lods: Default::default(),
            last_f: 0,
            ui_k: 1.0,
            block_tex: Default::default(),
            job: None,
            progress: None,
            started: None,
            stream_dirty: false,
            last_upload: None,
            error: None,
            center: Vec2::ZERO,
            zoom: 1.0,
            fitted: false,
            chunk_mode: false,
            locate: (0.0, 0.0),
            slicer_open: false,
            slicer: 320,
            slicer_min: -64,
            slicer_grab: None,
            over_controls: false,
            applied_slicer: None,
            selected: Vec::new(),
            marquee: None,
            selectable: true,
            hide_hover: false,
        }
    }

    /// Re-renders (after the world changed on disk).
    pub fn reload(&mut self, path: PathBuf) {
        self.path = path;
        self.invalidate();
    }

    /// Drops the picture and stops any render in flight, so the next frame starts a new one.
    fn invalidate(&mut self) {
        if let Some(p) = self.progress.take() {
            p.cancel();
        }
        self.image = None;
        self.lods.clear();
        self.job = None;
        self.error = None;
    }

    /// While a render runs, paints finished chunks into a provisional picture (uploaded at
    /// most ten times a second) so the map fills in instead of appearing all at once.
    fn stream(&mut self, ctx: &egui::Context) {
        let Some(p) = self.progress.clone() else {
            return;
        };
        let Some((min_x, min_z, width, height)) = *p.layout.lock().expect("map layout") else {
            return;
        };
        let fresh = std::mem::take(&mut *p.fresh.lock().expect("map tiles"));
        if self.image.is_none() {
            let img = MapImage {
                min_x,
                min_z,
                width,
                height,
                rgba: vec![0; width * height * 4],
                blocks: vec![0; width * height],
                over: vec![0; width * height],
                albedo: vec![0; width * height * 3],
                heights: vec![0; width * height],
                water: vec![0; width * height],
            };
            let color = egui::ColorImage::from_rgba_unmultiplied([width, height], &img.rgba);
            let tex = ctx.load_texture("world-map", color, egui::TextureOptions::NEAREST);
            self.image = Some((img, tex));
        }
        let Some((img, tex)) = &mut self.image else {
            return;
        };
        for (cx, cz, p) in &fresh {
            let (gx, gz) = ((cx - img.min_x) as usize * 16, (cz - img.min_z) as usize * 16);
            for row in 0..16 {
                let dst = ((gz + row) * img.width + gx) * 4;
                img.rgba[dst..dst + 64].copy_from_slice(&p.px[row * 64..row * 64 + 64]);
                let b = (gz + row) * img.width + gx;
                img.blocks[b..b + 16].copy_from_slice(&p.base[row * 16..row * 16 + 16]);
                img.over[b..b + 16].copy_from_slice(&p.over[row * 16..row * 16 + 16]);
                img.albedo[b * 3..(b + 16) * 3].copy_from_slice(&p.albedo[row * 48..row * 48 + 48]);
                img.heights[b..b + 16].copy_from_slice(&p.y[row * 16..row * 16 + 16]);
                img.water[b..b + 16].copy_from_slice(&p.water[row * 16..row * 16 + 16]);
            }
        }
        self.stream_dirty |= !fresh.is_empty();
        if self.stream_dirty && self.last_upload.is_none_or(|t| t.elapsed().as_millis() >= 100) {
            let color = egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.rgba);
            tex.set(color, egui::TextureOptions::NEAREST);
            self.stream_dirty = false;
            self.last_upload = Some(Instant::now());
        }
    }

    /// "Rendering Map" card with a progress bar, drawn while the picture is being made.
    fn loading_card(&self, ui: &mut Ui, area: Rect) {
        let card = vec2(420.0f32.min(area.width() - 24.0).max(200.0), 120.0);
        let rect = Rect::from_center_size(area.center(), card);
        let (fraction, done, total) = self.progress.as_ref().map_or((None, 0, 0), |p| {
            (
                p.fraction(),
                p.done.load(Ordering::Relaxed),
                p.total.load(Ordering::Relaxed),
            )
        });
        let elapsed = self.started.map_or(0.0, |s| s.elapsed().as_secs_f32());
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            widgets::card(ui, 19, |ui| {
                ui.set_width(card.x - 40.0);
                ui.vertical_centered(|ui| {
                    let (headline, detail) = match fraction {
                        Some(_) => (
                            "Rendering Map".to_string(),
                            format!("{done} of {total} chunks \u{b7} {elapsed:.0}s"),
                        ),
                        None => (
                            "Reading World".to_string(),
                            format!("Scanning chunks \u{b7} {elapsed:.0}s"),
                        ),
                    };
                    ui.label(RichText::new(headline).size(16.0).color(theme::TEXT));
                    ui.add_space(8.0);
                    // Until the chunk count is known, a bar that keeps filling to show activity.
                    let shown = fraction.unwrap_or_else(|| (elapsed * 0.6).fract());
                    widgets::progress_bar(ui, shown, card.x - 40.0);
                    ui.add_space(8.0);
                    ui.label(RichText::new(detail).size(13.0).color(theme::TEXT_MUTED));
                });
            });
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }

    /// Draws the map and handles pan/zoom. `primary_pans` is false while a drawing tool
    /// owns the left button (the right or middle button still pans).
    pub fn show_map(&mut self, ui: &mut Ui, primary_pans: bool) -> MapFrame {
        let ctx = ui.ctx().clone();
        if let Some(r) = self.job.as_ref().and_then(Job::take) {
            self.job = None;
            match r {
                Ok(img) => {
                    let color = egui::ColorImage::from_rgba_unmultiplied(
                        [img.width, img.height],
                        &img.rgba,
                    );
                    let tex = ctx.load_texture("world-map", color, egui::TextureOptions::NEAREST);
                    self.image = Some((img, tex));
                    self.lods.clear();
                    self.error = None;
                }
                Err(e) => {
                    self.image = None;
                    self.error = Some(e);
                }
            }
        }
        if self.job.is_some() {
            self.stream(&ctx);
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.image.is_none() && self.job.is_none() && self.error.is_none() {
            let (p, e, d, s) = (
                self.path.clone(),
                self.edition,
                self.dimension.clone(),
                self.applied_slicer,
            );
            let progress = Arc::new(MapProgress::default());
            let shared = Arc::clone(&progress);
            self.job = Some(Job::spawn("Rendering map", &ctx, move || {
                let (min_y, max_y) = s.map_or((i32::MIN, None), |(lo, hi)| (lo, Some(hi)));
                world::render_map_with(&p, e, &d, max_y, min_y, &shared)
            }));
            self.progress = Some(progress);
            self.started = Some(Instant::now());
        }

        let outer = ui.available_rect_before_wrap();
        let rect = Rect::from_min_max(outer.min + vec2(RULER, RULER), outer.max);
        let response = ui.allocate_rect(outer, Sense::click_and_drag());
        let painter = ui.painter_at(outer);
        painter.rect_filled(outer, 0.0, theme::BG);

        if let Some((img, tex)) = &self.image {
            if !self.fitted {
                let (w, h) = (img.width as f32, img.height as f32);
                self.zoom = (rect.width() / w).min(rect.height() / h).clamp(0.05, 8.0);
                self.center = vec2(
                    img.min_x as f32 * 16.0 + w / 2.0,
                    img.min_z as f32 * 16.0 + h / 2.0,
                );
                self.fitted = true;
            }
            let frame = MapFrame {
                rect,
                center: self.center,
                zoom: self.zoom,
                response: response.clone(),
                hover: None,
            };
            let min = frame.to_screen(vec2(img.min_x as f32 * 16.0, img.min_z as f32 * 16.0));
            let size = vec2(img.width as f32, img.height as f32) * self.zoom;
            // Zoomed out, a smaller smoothed copy is drawn; zoomed in, the blocks themselves.
            let whole = Rect::from_min_size(min, size);
            let uv_all = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            let clip = painter.with_clip_rect(rect);
            // The picture itself underneath, for whatever the zoomed-out tiles have not reached yet.
            clip.image(tex.id(), whole, uv_all, Color32::WHITE);
            if self.zoom < 1.0 && self.job.is_none() {
                // The shrink whose pixels come nearest to (and not under) one screen pixel. Far
                // enough out, a flat mosaic of chunks (then of 2, 4, 8... chunks) as in UMT.
                const STEPS: [usize; 8] = [2, 3, 4, 5, 6, 8, 10, 12];
                const MOSAIC: [usize; 5] = [16, 32, 64, 128, 256];
                let steps: &[usize] = if self.zoom < 0.3 { &MOSAIC } else { &STEPS };
                let f = steps.iter().copied().find(|f| *f as f32 * self.zoom >= 1.0).unwrap_or(256);
                let prev = self.last_f;
                self.lods.retain(|(ff, _, _), _| *ff == f || *ff == prev);
                // The tiles in view at a shrink, and the spot (in picture pixels) the view works
                // outward from: under the pointer if it is on the map, else the middle.
                let zoom = self.zoom;
                let focus = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)).unwrap_or(rect.center());
                let focus = (focus - min) / zoom;
                let visible = |f: usize| {
                    let span = (LOD_TILE * f) as f32;
                    let (nx, ny) = (img.width.div_ceil(LOD_TILE * f), img.height.div_ceil(LOD_TILE * f));
                    let lo = (rect.min - min) / zoom;
                    let hi = (rect.max - min) / zoom;
                    let range = |a: f32, b: f32, n: usize| (((a / span).floor().max(0.0) as usize)..=((b / span).floor().max(0.0) as usize).min(n.saturating_sub(1)));
                    let mut v = Vec::new();
                    for ty in range(lo.y, hi.y, ny) {
                        for tx in range(lo.x, hi.x, nx) {
                            v.push((tx, ty));
                        }
                    }
                    v
                };
                // Build the missing tiles at this shrink, the ones nearest the focus first, within
                // a few milliseconds a frame.
                let mut missing: Vec<_> = visible(f).into_iter().filter(|(tx, ty)| !self.lods.contains_key(&(f, *tx, *ty))).collect();
                let span = (LOD_TILE * f) as f32;
                missing.sort_by(|a, b| {
                    let d = |t: &(usize, usize)| {
                        let c = vec2((t.0 as f32 + 0.5) * span, (t.1 as f32 + 0.5) * span);
                        (c - focus).length_sq()
                    };
                    d(a).total_cmp(&d(b))
                });
                let started = Instant::now();
                let mut left = missing.len();
                for (tx, ty) in missing {
                    if started.elapsed().as_millis() >= 6 {
                        break;
                    }
                    self.lods.insert((f, tx, ty), lod_tile(&ctx, img, f, tx, ty));
                    left -= 1;
                }
                if left > 0 {
                    ctx.request_repaint();
                }
                // The previous shrink's tiles (still held) first, then this one's over them.
                for ff in [prev, f] {
                    if ff == 0 {
                        continue;
                    }
                    let span = (LOD_TILE * ff) as f32 * zoom;
                    for (tx, ty) in visible(ff) {
                        if let Some(t) = self.lods.get(&(ff, tx, ty)) {
                            let (w, h) = (t.size()[0] as f32, t.size()[1] as f32);
                            let at = min + vec2(tx as f32, ty as f32) * span;
                            let r = Rect::from_min_size(at, vec2(w, h) * (ff as f32 * zoom));
                            clip.image(t.id(), r, uv_all, Color32::WHITE);
                        }
                    }
                }
                if left == 0 {
                    self.last_f = f;
                }
            }            // Zoomed in far enough, the blocks themselves are drawn (their pictures, lit as the flat
            // map is), on whole screen pixels so they do not crawl as the view moves.
            if self.zoom >= 4.0 {
                let (b0, b1) = (frame.to_block(rect.min), frame.to_block(rect.max));
                let (ox, oz) = (img.min_x * 16, img.min_z * 16);
                let x0 = (b0.x.floor() as i32).max(ox);
                let x1 = (b1.x.ceil() as i32).min(ox + img.width as i32);
                let z0 = (b0.y.floor() as i32).max(oz);
                let z1 = (b1.y.ceil() as i32).min(oz + img.height as i32);
                if (x1 - x0).max(0) as i64 * (z1 - z0).max(0) as i64 <= 40_000 {
                    let clip = painter.with_clip_rect(rect);
                    let zoom = self.zoom;
                    let snap = |bx: i32, bz: i32| {
                        let a = frame.to_screen(vec2(bx as f32, bz as f32));
                        let b = frame.to_screen(vec2(bx as f32 + 1.0, bz as f32 + 1.0));
                        Rect::from_min_max(pos2(a.x.round(), a.y.round()), pos2(b.x.round(), b.y.round()))
                    };
                    for bz in z0..z1 {
                        for bx in x0..x1 {
                            let idx = (bz - oz) as usize * img.width + (bx - ox) as usize;
                            let id = img.blocks[idx];
                            if id == 0 {
                                continue;
                            }
                            let cell = snap(bx, bz);
                            let lit = &img.rgba[idx * 4..idx * 4 + 3];
                            // Zoomed in the blocks are not shaded for height, only tinted (and water by its depth).
                            let light = 1.0;
                            let water = img.water[idx] > 0;
                            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                            // The ground, then a plant or leaves over it.
                            for layer in [id, img.over[idx]] {
                                if layer == 0 {
                                    continue;
                                }
                                let Some(bt) = block_texture(&mut self.block_tex, &ctx, layer) else { continue };
                                // Water takes the colour its depth gives; the rest the flat map''s, lit.
                                let t = |k: usize| {
                                    let r = if water { lit[k] as f32 / bt.avg[k].max(1.0) } else { bt.flat[k] / bt.avg[k].max(1.0) * light };
                                    (r.min(1.0) * 255.0) as u8
                                };
                                clip.image(bt.pick(zoom), cell, uv, Color32::from_rgb(t(0), t(1), t(2)));
                            }
                        }
                    }
                }
            }        } else if let Some(e) = &self.error {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("Could not render the map: {e}"),
                theme::regular(15.0),
                theme::TEXT_MUTED,
            );
        } else {
            self.loading_card(ui, rect);
        }

        // A small progress note while the picture is still filling in.
        if self.job.is_some() && self.image.is_some() {
            let pct = self
                .progress
                .as_ref()
                .and_then(|p| p.fraction())
                .map_or(0, |f| (f * 100.0) as u32);
            let r = Rect::from_center_size(
                pos2(rect.center().x, rect.top() + 22.0),
                vec2(170.0, 26.0),
            );
            painter.rect_filled(r, theme::radius(4), theme::CARD);
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                format!("Rendering map\u{2026} {pct}%"),
                theme::regular(13.0),
                theme::TEXT_MUTED,
            );
        }

        // Zoom around the pointer and pan.
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
                    let before = self.center + (p - rect.center()) / self.zoom;
                    self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.03, 24.0);
                    self.center = before - (p - rect.center()) / self.zoom;
                }
            }
        }
        // Shift+drag draws a box over chunks instead of panning, in the Chunk Locator.
        let shift = ui.input(|i| i.modifiers.shift);
        let boxing = primary_pans && self.selectable && (shift || self.marquee.is_some());
        let pans = (primary_pans && !boxing && response.dragged_by(egui::PointerButton::Primary))
            || response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle);
        if pans {
            self.center -= response.drag_delta() / self.zoom;
        }
        let mut frame = MapFrame {
            rect,
            center: self.center,
            zoom: self.zoom,
            response,
            hover: None,
        };
        frame.hover = frame
            .response
            .hover_pos()
            .filter(|p| rect.contains(*p))
            .map(|p| frame.to_block(p));
        if primary_pans && self.selectable {
            let ctrl = ui.input(|i| i.modifiers.command);
            let at = frame.response.interact_pointer_pos().map(|p| frame.to_block(p));
            let chunk_at = |b: Vec2| ((b.x / 16.0).floor() as i32, (b.y / 16.0).floor() as i32);
            if shift && frame.response.drag_started_by(egui::PointerButton::Primary) {
                self.marquee = at.map(|a| (a, a));
            }
            if let (Some((a, _)), Some(b)) = (self.marquee, at) {
                if frame.response.dragged() {
                    self.marquee = Some((a, b));
                }
            }
            if frame.response.drag_stopped() {
                if let Some((a, b)) = self.marquee.take() {
                    let (lo, hi) = (chunk_at(a.min(b)), chunk_at(a.max(b)));
                    // Ctrl keeps what was already picked; otherwise the box replaces it.
                    if !ctrl {
                        self.selected.clear();
                    }
                    let count = (hi.0 - lo.0 + 1) as i64 * (hi.1 - lo.1 + 1) as i64;
                    if count <= 20_000 {
                        let mut have: std::collections::HashSet<_> = self.selected.iter().copied().collect();
                        for z in lo.1..=hi.1 {
                            for x in lo.0..=hi.0 {
                                if have.insert((x, z)) {
                                    self.selected.push((x, z));
                                }
                            }
                        }
                    }
                }
            } else if frame.response.clicked() {
                if let Some(c) = frame.hover.map(chunk_at) {
                    match (ctrl, self.selected.iter().position(|s| *s == c)) {
                        (true, Some(i)) => {
                            self.selected.remove(i);
                        }
                        (true, None) => self.selected.push(c),
                        (false, _) => self.selected = vec![c],
                    }
                }
            }
        }
        frame
    }

    /// Rulers, selection, tooltip and the floating controls, drawn over any overlays.
    pub fn show_chrome(&mut self, ui: &mut Ui, frame: &MapFrame) {
        let outer = Rect::from_min_max(frame.rect.min - vec2(RULER, RULER), frame.rect.max);
        let p = ui.painter_at(outer);
        if self.selectable {
            let inside = p.with_clip_rect(frame.rect);
            let green = Color32::from_rgb(0x1A, 0xFB, 0x2F);
            for &(cx, cz) in &self.selected {
                let a = frame.to_screen(vec2(cx as f32 * 16.0, cz as f32 * 16.0));
                let b = frame.to_screen(vec2(cx as f32 * 16.0 + 16.0, cz as f32 * 16.0 + 16.0));
                // As in the original tool: a green tint with a green border split by a black line.
                let chunk = Rect::from_min_max(a, b);
                if !chunk.intersects(frame.rect) {
                    continue;
                }
                inside.rect_filled(chunk, 0.0, Color32::from_rgba_premultiplied(0, 83, 16, 84));
                for (rect, colour) in [
                    (chunk.expand(1.0), green),
                    (chunk, Color32::BLACK),
                    (chunk.shrink(1.0), green),
                ] {
                    inside.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, colour), StrokeKind::Middle);
                }
            }
            if let Some((a, b)) = self.marquee {
                let r = Rect::from_two_pos(frame.to_screen(a), frame.to_screen(b));
                inside.rect_filled(r, 0.0, Color32::from_rgba_premultiplied(0, 40, 8, 40));
                inside.rect_stroke(r, 0.0, Stroke::new(1.0_f32, green), StrokeKind::Middle);
            }
        }
        // Rulers as in the original tool: a tick every eighth of a labelled step (16 blocks at
        // the usual zoom); the labelled ticks run the ruler's full width with the number beside.
        let top = Rect::from_min_max(
            pos2(frame.rect.left(), outer.top()),
            pos2(outer.right(), frame.rect.top()),
        );
        let left = Rect::from_min_max(
            pos2(outer.left(), frame.rect.top()),
            pos2(frame.rect.left(), outer.bottom()),
        );
        let ruler_bg = Color32::from_rgb(35, 40, 48);
        let tick = Stroke::new(1.0_f32, Color32::from_rgb(71, 80, 92));
        let digits = Color32::from_rgb(196, 202, 212);
        p.rect_filled(top, 0.0, ruler_bg);
        p.rect_filled(left, 0.0, ruler_bg);
        p.rect_filled(Rect::from_min_max(outer.min, frame.rect.min), 0.0, theme::BG);
        p.line_segment([top.left_bottom(), top.right_bottom()], tick);
        p.line_segment([left.right_top(), left.right_bottom()], tick);
        let step = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192]
            .into_iter()
            .find(|s| *s as f32 * frame.zoom >= 96.0)
            .unwrap_or(16384) as f32;
        let mut minor = step / 8.0;
        while minor * frame.zoom < 6.0 {
            minor *= 2.0;
        }
        let b0 = frame.to_block(frame.rect.min);
        let b1 = frame.to_block(frame.rect.max);
        let short = RULER * 0.65;
        let mut i = (b0.x / minor).floor() as i64;
        while i as f32 * minor <= b1.x {
            let x = i as f32 * minor;
            let sx = frame.to_screen(vec2(x, 0.0)).x;
            if sx >= frame.rect.left() && sx <= frame.rect.right() {
                let major = (x / step).fract().abs() < 1e-4;
                let from = if major { top.top() } else { top.bottom() - short };
                p.line_segment([pos2(sx, from), pos2(sx, top.bottom())], tick);
                if major {
                    // The number sits on a patch that hides the ticks behind it.
                    let galley =
                        p.layout_no_wrap(format!("{}", x as i64), theme::regular(12.0), digits);
                    let at = pos2(sx + 4.0, top.center().y - 1.0 - galley.size().y / 2.0);
                    p.rect_filled(
                        Rect::from_min_max(
                            pos2(sx + 2.0, top.top() + 1.0),
                            pos2(at.x + galley.size().x + 3.0, top.bottom() - 4.0),
                        ),
                        0.0,
                        ruler_bg,
                    );
                    p.galley(at, galley, digits);
                }
            }
            i += 1;
        }
        let mut i = (b0.y / minor).floor() as i64;
        while i as f32 * minor <= b1.y {
            let z = i as f32 * minor;
            let sy = frame.to_screen(vec2(0.0, z)).y;
            if sy >= frame.rect.top() && sy <= frame.rect.bottom() {
                let major = (z / step).fract().abs() < 1e-4;
                let from = if major { left.left() } else { left.right() - short };
                p.line_segment([pos2(from, sy), pos2(left.right(), sy)], tick);
                if major {
                    // Read upward, ending just before the tick.
                    let galley =
                        p.layout_no_wrap(format!("{}", z as i64), theme::regular(12.0), digits);
                    let at = pos2(left.left() + (left.width() - galley.size().y) / 2.0, sy - 4.0);
                    // Clipped to the ruler, so a number near the top never runs onto the corner.
                    let p = p.with_clip_rect(left);
                    p.rect_filled(
                        Rect::from_min_max(
                            pos2(left.left() + 1.0, at.y - galley.size().x - 3.0),
                            pos2(left.right() - 4.0, sy - 2.0),
                        ),
                        0.0,
                        ruler_bg,
                    );
                    p.add(
                        egui::epaint::TextShape::new(at, galley, digits)
                            .with_angle(-std::f32::consts::FRAC_PI_2),
                    );
                }
            }
            i += 1;
        }        if let Some(h) = frame.hover.filter(|_| !self.over_controls && !self.hide_hover) {
            // The chunk under the pointer.
            let (cx, cz) = ((h.x / 16.0).floor(), (h.y / 16.0).floor());
            let a = frame.to_screen(vec2(cx * 16.0, cz * 16.0));
            let b = frame.to_screen(vec2(cx * 16.0 + 16.0, cz * 16.0 + 16.0));
            let chunk = Rect::from_min_max(a, b);
            let inside = p.with_clip_rect(frame.rect);
            inside.rect_filled(chunk, 0.0, Color32::from_rgba_premultiplied(34, 34, 34, 34));
            inside.rect_stroke(
                chunk,
                0.0,
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0x14, 0x18, 0x1C, 200)),
                StrokeKind::Middle,
            );
            // Ruler markers at the pointer.
            let s = frame.to_screen(h);
            p.line_segment(
                [pos2(s.x, top.top()), pos2(s.x, top.bottom())],
                Stroke::new(1.5_f32, theme::accent()),
            );
            p.line_segment(
                [pos2(left.left(), s.y), pos2(left.right(), s.y)],
                Stroke::new(1.5_f32, theme::accent()),
            );
            let (bx, bz) = (h.x.floor() as i64, h.y.floor() as i64);
            let text = format!(
                "Chunk:  {}, {}\nBlock:  {}, {}",
                bx.div_euclid(16),
                bz.div_euclid(16),
                bx,
                bz
            );
            let galley = p.layout_no_wrap(text, theme::regular(13.0), theme::TEXT);
            let r = Rect::from_min_size(s + vec2(16.0, 12.0), galley.size() + vec2(16.0, 10.0));
            p.rect_filled(
                r,
                theme::radius(4),
                Color32::from_rgba_unmultiplied(0x16, 0x18, 0x1E, 235),
            );
            p.galley(r.min + vec2(8.0, 5.0), galley, theme::TEXT);
        }

        // Everything on the controls is drawn this much smaller in a narrow window.
        let ks = ((frame.rect.width() / 730.0).clamp(0.62, 1.0) * 50.0).round() / 50.0;
        self.ui_k = ks;
        // Locate bar.
        let bar = Rect::from_min_size(frame.rect.min + vec2(22.0 * ks, 22.0 * ks), vec2(384.0 * ks, 44.0 * ks));
        ui.painter().rect_filled(bar, theme::radius(5), theme::DEEP);
        // Every part is the same height; a labelled part is a lighter cell beside a dark field.
        let cell = Color32::from_rgb(43, 48, 59);
        let field = Color32::from_rgb(12, 14, 18);
        let h = 28.0 * ks;
        let y = bar.top() + (bar.height() - h) / 2.0;
        let mut x = bar.left() + 8.0 * ks;
        let split = |ui: &Ui, r: Rect, cell_w: f32| {
            let r5 = 5;
            ui.painter().rect_filled(
                Rect::from_min_size(r.min, vec2(cell_w, h)),
                egui::CornerRadius { nw: r5, sw: r5, ne: 0, se: 0 },
                cell,
            );
            ui.painter().rect_filled(
                Rect::from_min_max(pos2(r.left() + cell_w, r.top()), r.max),
                egui::CornerRadius { nw: 0, sw: 0, ne: r5, se: r5 },
                field,
            );
        };
        // Block / Chunk: a small square for blocks, a big one for chunks.
        let r = Rect::from_min_size(pos2(x, y), vec2(78.0 * ks, h));
        split(ui, r, 28.0 * ks);
        let resp = ui
            .interact(r, ui.id().with("loc-mode"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let side = if self.chunk_mode { 18.0 * ks } else { 8.0 * ks };
        ui.painter().rect_stroke(
            Rect::from_center_size(pos2(r.left() + 14.0 * ks, r.center().y), Vec2::splat(side)),
            // A small block would be a circle at the chunk's corner radius.
            theme::radius(if self.chunk_mode { 3 } else { 1 }),
            Stroke::new(1.3_f32 * ks, Color32::WHITE),
            StrokeKind::Inside,
        );
        ui.painter().text(
            pos2(r.left() + 28.0 * ks + 25.0 * ks, r.center().y),
            Align2::CENTER_CENTER,
            if self.chunk_mode { "Chunk" } else { "Block" },
            theme::regular(14.0 * ks),
            Color32::WHITE,
        );
        if resp.clicked() {
            self.chunk_mode = !self.chunk_mode;
        }
        x = r.right() + 6.0 * ks;
        for (axis, label) in [(0, "X"), (1, "Z")] {
            let r = Rect::from_min_size(pos2(x, y), vec2(84.0 * ks, h));
            split(ui, r, 22.0 * ks);
            ui.painter().text(
                pos2(r.left() + 11.0 * ks, r.center().y),
                Align2::CENTER_CENTER,
                label,
                theme::regular(14.0 * ks),
                Color32::WHITE,
            );
            let value = Rect::from_min_max(pos2(r.left() + 22.0 * ks + 4.0 * ks, r.top()), pos2(r.right() - 6.0 * ks, r.bottom()));
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(value)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    let s = ui.style_mut();
                    for w in [
                        &mut s.visuals.widgets.inactive,
                        &mut s.visuals.widgets.hovered,
                        &mut s.visuals.widgets.active,
                    ] {
                        w.weak_bg_fill = Color32::TRANSPARENT;
                        w.bg_fill = Color32::TRANSPARENT;
                        w.bg_stroke = Stroke::NONE;
                    }
                    s.spacing.interact_size = vec2(1.0 * ks, h);
                    s.override_font_id = Some(theme::regular(14.0 * ks));
                    let v = if axis == 0 { &mut self.locate.0 } else { &mut self.locate.1 };
                    ui.add(egui::DragValue::new(v).speed(1.0 * ks));
                },
            );
            x = r.right() + 6.0 * ks;
        }
        let r = Rect::from_min_max(pos2(x, y), pos2(bar.right() - 8.0 * ks, y + h));
        let resp = ui
            .interact(r, ui.id().with("loc-go"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        ui.painter().rect_filled(
            r,
            theme::radius(5),
            if resp.hovered() { Color32::from_rgb(52, 58, 70) } else { Color32::from_rgb(35, 40, 48) },
        );
        ui.painter().rect_stroke(
            r,
            theme::radius(5),
            Stroke::new(1.0_f32 * ks, Color32::from_rgb(44, 50, 58)),
            StrokeKind::Inside,
        );
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(r.left() + 20.0 * ks, r.center().y), Vec2::splat(16.0 * ks)),
            Icon::Search,
            Color32::WHITE,
        );
        ui.painter().text(
            pos2(r.left() + 40.0 * ks, r.center().y),
            Align2::LEFT_CENTER,
            "Locate",
            theme::regular(14.0 * ks),
            Color32::WHITE,
        );
        if resp.clicked() {
            let k = if self.chunk_mode { 16.0 } else { 1.0 };
            self.center = vec2(
                self.locate.0 as f32 * k + if self.chunk_mode { 8.0 } else { 0.5 },
                self.locate.1 as f32 * k + if self.chunk_mode { 8.0 } else { 0.5 },
            );
            self.zoom = self.zoom.max(2.0);
            if self.chunk_mode {
                self.selected = vec![(self.locate.0 as i32, self.locate.1 as i32)];
            }
        }
        // Dimension buttons and Slicer.
        let mut slicer_rect = Rect::from_min_size(
            pos2(frame.rect.right() - 118.0 * ks, frame.rect.top() + 22.0 * ks),
            vec2(96.0 * ks, 44.0 * ks),
        );
        let mut dims_rect = Rect::from_min_size(
            pos2(slicer_rect.left() - 162.0 * ks, slicer_rect.top()),
            vec2(150.0 * ks, 44.0 * ks),
        );
        // Too narrow for one row (small window or a large scale factor): move the
        // dimension buttons and Slicer below the Locate bar instead of over it.
        if dims_rect.left() < bar.right() + 12.0 * ks {
            slicer_rect = slicer_rect.translate(vec2(0.0, 54.0 * ks));
            dims_rect = dims_rect.translate(vec2(0.0, 54.0 * ks));
        }
        ui.painter()
            .rect_filled(dims_rect, theme::radius(5), theme::DEEP);
        for (i, d) in DIMS.iter().enumerate() {
            let r = Rect::from_min_size(
                dims_rect.min + vec2(4.0 * ks + i as f32 * 48.0 * ks, 4.0 * ks),
                vec2(46.0 * ks, 36.0 * ks),
            );
            let resp = ui
                .interact(r, ui.id().with(("dim", i)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(match i {
                    0 => "Overworld",
                    1 => "Nether",
                    _ => "The End",
                });
            let selected = self.dimension == *d;
            if resp.hovered() && !selected {
                ui.painter()
                    .rect_filled(r, theme::radius(4), theme::BUTTON_HOVER);
            }
            if selected {
                ui.painter().rect_filled(r, theme::radius(4), theme::CARD);
                ui.painter().rect_stroke(
                    r,
                    theme::radius(4),
                    Stroke::new(1.2_f32 * ks, theme::accent()),
                    StrokeKind::Inside,
                );
            }
            dim_tile(
                ui.painter(),
                Rect::from_center_size(r.center(), Vec2::splat(30.0 * ks)),
                i,
            );
            if resp.clicked() && !selected {
                self.dimension = d.to_string();
                self.invalidate();
                self.error = None;
                self.fitted = false;
                self.selected.clear();
            }
        }
        // The Slicer: a button that opens a panel with a track and two handles, as in the
        // original tool. Only blocks between the handles are drawn.
        let border = Color32::from_rgb(44, 50, 58);
        let open = self.slicer_open;
        let panel_h = (frame.rect.height() - 32.0 * ks).clamp(300.0 * ks, 552.0 * ks);
        let panel = Rect::from_min_size(
            slicer_rect.min,
            vec2(
                slicer_rect.width(),
                if open { panel_h } else { slicer_rect.height() },
            ),
        );
        let header = Rect::from_min_size(panel.min, slicer_rect.size());
        let resp = ui
            .interact(header, ui.id().with("slicer"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if open {
            ui.painter().rect_filled(panel, theme::radius(6), theme::BG);
            ui.painter().rect_filled(
                header,
                egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 },
                theme::DEEP,
            );
            ui.painter()
                .line_segment([header.left_bottom(), header.right_bottom()], Stroke::new(1.0_f32 * ks, border));
            ui.painter().rect_stroke(
                panel,
                theme::radius(6),
                Stroke::new(1.0_f32 * ks, border),
                StrokeKind::Inside,
            );
        } else {
            ui.painter().rect_filled(
                header,
                theme::radius(5),
                if resp.hovered() { theme::BUTTON_HOVER } else { theme::DEEP },
            );
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(header.left_center() + vec2(22.0 * ks, 0.0), Vec2::splat(22.0 * ks)),
            Icon::Layers,
            theme::TEXT,
        );
        ui.painter().text(
            header.left_center() + vec2(40.0 * ks, 0.0),
            Align2::LEFT_CENTER,
            "Slicer",
            theme::regular(14.0 * ks),
            theme::TEXT,
        );
        if resp.clicked() {
            self.slicer_open = !self.slicer_open;
        }
        if open {
            self.slicer_panel(ui, panel, header);
        }
        let pointer = ui.input(|i| i.pointer.hover_pos());
        self.over_controls = pointer
            .is_some_and(|p| [bar, dims_rect, panel].iter().any(|r| r.expand(2.0).contains(p)));        if self.job.is_some() {
            ui.painter().text(
                frame.rect.right_bottom() - vec2(12.0, 10.0),
                Align2::RIGHT_BOTTOM,
                "Rendering…",
                theme::regular(12.0),
                theme::TEXT_MUTED,
            );
        }
    }

    /// The open Slicer: upper and lower Y boxes around a track with a handle for each limit.
    fn slicer_panel(&mut self, ui: &mut Ui, panel: Rect, header: Rect) {
        let ks = self.ui_k;
        let cell = Color32::from_rgb(43, 48, 59);
        let handle = Color32::from_rgb(45, 194, 115);
        let mut changed = false;
        let x0 = panel.left() + 9.0 * ks;
        let top_box = Rect::from_min_size(pos2(x0, header.bottom() + 12.0 * ks), vec2(75.0 * ks, 27.0 * ks));
        let bottom_box = Rect::from_min_size(pos2(x0, panel.bottom() - 12.0 * ks - 27.0 * ks), vec2(75.0 * ks, 27.0 * ks));
        let track = Rect::from_min_max(
            pos2(x0, top_box.bottom() + 10.0 * ks),
            pos2(x0 + 47.0 * ks, bottom_box.top() - 10.0 * ks),
        );
        // The two Y boxes: a lighter cell holding the letter, a dark field with the number.
        for (r, upper) in [(top_box, true), (bottom_box, false)] {
            ui.painter().rect_filled(
                Rect::from_min_size(r.min, vec2(22.0 * ks, r.height())),
                egui::CornerRadius { nw: 5, sw: 5, ne: 0, se: 0 },
                cell,
            );
            ui.painter().rect_filled(
                Rect::from_min_max(pos2(r.left() + 22.0 * ks, r.top()), r.max),
                egui::CornerRadius { nw: 0, sw: 0, ne: 5, se: 5 },
                theme::DEEP,
            );
            ui.painter().text(
                pos2(r.left() + 11.0 * ks, r.center().y),
                Align2::CENTER_CENTER,
                "Y",
                theme::regular(14.0 * ks),
                Color32::WHITE,
            );
            let (lo, hi) = if upper { (self.slicer_min, 320) } else { (-64, self.slicer) };
            let value = Rect::from_min_max(pos2(r.left() + 26.0 * ks, r.top()), pos2(r.right() - 6.0 * ks, r.bottom()));
            let slot = if upper { &mut self.slicer } else { &mut self.slicer_min };
            let resp = ui
                .scope_builder(
                    UiBuilder::new()
                        .max_rect(value)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                    |ui| {
                        let s = ui.style_mut();
                        for w in [
                            &mut s.visuals.widgets.inactive,
                            &mut s.visuals.widgets.hovered,
                            &mut s.visuals.widgets.active,
                        ] {
                            w.weak_bg_fill = Color32::TRANSPARENT;
                            w.bg_fill = Color32::TRANSPARENT;
                            w.bg_stroke = Stroke::NONE;
                        }
                        s.spacing.interact_size = vec2(1.0 * ks, r.height());
                        s.override_font_id = Some(theme::regular(14.0 * ks));
                        ui.add(egui::DragValue::new(slot).range(lo..=hi).speed(1.0 * ks))
                    },
                )
                .inner;
            if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                changed = true;
            }
        }
        // The track, its ticks and the two handles.
        let span = (track.height() - 26.0 * ks).max(1.0 * ks);
        let y_of = |v: i32| track.top() + 13.0 * ks + (320 - v) as f32 / 384.0 * span;
        let v_of = |y: f32| {
            (320.0 - (y - track.top() - 13.0 * ks) / span * 384.0)
                .round()
                .clamp(-64.0, 320.0) as i32
        };
        ui.painter().rect_filled(track, theme::radius(6), theme::DEEP);
        for v in [320, 256, 128, 64, 0, -64] {
            let y = y_of(v);
            if ![320, -64].contains(&v) {
                ui.painter().line_segment(
                    [pos2(track.center().x - 14.0 * ks, y), pos2(track.center().x + 14.0 * ks, y)],
                    Stroke::new(1.0_f32 * ks, Color32::from_rgb(86, 97, 111)),
                );
            }
            ui.painter().text(
                pos2(track.right() + 10.0 * ks, y),
                Align2::LEFT_CENTER,
                v.to_string(),
                theme::regular(12.0 * ks),
                Color32::from_rgb(130, 138, 150),
            );
        }
        for v in [self.slicer, self.slicer_min] {
            ui.painter().rect_filled(
                Rect::from_center_size(pos2(track.center().x, y_of(v)), vec2(35.0 * ks, 8.0 * ks)),
                theme::radius(4),
                handle,
            );
        }
        let resp = ui.interact(track, ui.id().with("slicer-track"), Sense::click_and_drag());
        // The handle nearer the pointer is the one that moves.
        let nearer = |me: &Self, y: f32| (y - y_of(me.slicer)).abs() <= (y - y_of(me.slicer_min)).abs();
        let set = |me: &mut Self, upper: bool, y: f32| {
            let v = v_of(y);
            if upper {
                me.slicer = v.max(me.slicer_min);
            } else {
                me.slicer_min = v.min(me.slicer);
            }
        };
        if resp.drag_started() {
            if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
                self.slicer_grab = Some(nearer(self, p.y));
            }
        }
        if resp.dragged() {
            if let (Some(upper), Some(p)) = (self.slicer_grab, resp.interact_pointer_pos()) {
                set(self, upper, p.y);
            }
        }
        if resp.drag_stopped() {
            self.slicer_grab = None;
            changed = true;
        } else if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let upper = nearer(self, p.y);
                set(self, upper, p.y);
                changed = true;
            }
        }
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if changed {
            self.applied_slicer = (self.slicer_min > -64 || self.slicer < 320)
                .then_some((self.slicer_min, self.slicer));
            self.invalidate();
            self.error = None;
        }
    }}
