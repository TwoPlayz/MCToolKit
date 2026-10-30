//! Icons: UMT's original artwork where it is free to use (see `assets/icons/NOTICE.md`),
//! and vector drawings of our own for everything else — the app logo and app icons, which
//! stay MCToolKit's own, plus any icon without an image.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui::{
    self, Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, TextureHandle, pos2, vec2,
};

use crate::theme;

macro_rules! images {
    ($($name:literal),* $(,)?) => {
        const IMAGES: &[(&str, &[u8])] = &[
            $(($name, include_bytes!(concat!("../assets/icons/", $name, ".png"))),)*
        ];
    };
}

images![
    "ADD",
    "ADD_h",
    "ARROW_DOWN",
    "ARROW_RIGHT",
    "AUTHOR",
    "CHUNK_LOCATOR",
    "CHUNK_LOCATOR_h",
    "CLOSE",
    "CLOSE_h",
    "CLOSE_ALT",
    "CLOSE_ALT_h",
    "CONVERTER_ARROW",
    "EDITION_CHECK",
    "EDITION_CROSS",
    "EXTENSIONS",
    "EXTENSIONS_h",
    "FILE",
    "FILE_h",
    "FIND",
    "FOLDER",
    "FOLDER_h",
    "INFO",
    "INFO_h",
    "MAX",
    "MAX_h",
    "MIN",
    "MIN_h",
    "PRUNER_BOX",
    "PRUNER_BOX_h",
    "PRUNER_CIRCLE",
    "PRUNER_CIRCLE_h",
    "PRUNER_LAYER_TRASH",
    "PRUNER_LAYER_TRASH_h",
    "PRUNER_OPTIMIZE",
    "PRUNER_OPTIMIZE_h",
    "PRUNER_POLYGON",
    "PRUNER_POLYGON_h",
    "PRUNER_SPLIT",
    "PRUNER_SPLIT_h",
    "PRUNER_TIME",
    "PRUNER_TIME_h",
    "REDO",
    "REFRESH",
    "REFRESH_h",
    "SAVE",
    "SAVE_h",
    "SETTINGS",
    "SETTINGS_h",
    "SLICER",
    "SLICER_h",
    "STEP_COMPLETE",
    "STEP_INCOMPLETE",
    "STEP_IN_PROGRESS",
    "STEP_NEXT",
    "STEP_PREV",
    "TRASH",
    "TRASH_h",
    "UNDO",
    "TAG_BYTE",
    "TAG_SHORT",
    "TAG_INT",
    "TAG_LONG",
    "TAG_FLOAT",
    "TAG_DOUBLE",
    "TAG_STRING",
    "TAG_LIST",
    "TAG_COMPOUND",
    "TAG_BYTE_ARRAY",
    "TAG_INT_ARRAY",
    "TAG_LONG_ARRAY",
    "THUMB_BOX_DELETE",
    "THUMB_BOX_DELETE_INVERSE",
    "THUMB_BOX_KEEP",
    "THUMB_BOX_KEEP_INVERSE",
    "THUMB_CIRCLE_DELETE",
    "THUMB_CIRCLE_DELETE_INVERSE",
    "THUMB_CIRCLE_KEEP",
    "THUMB_CIRCLE_KEEP_INVERSE",
    "THUMB_SPLIT_DELETE",
    "THUMB_SPLIT_KEEP",
    "THUMB_TIME",
    "PLATFORM_JAVA",
    "PLATFORM_JAVA_h",
    "PLATFORM_BEDROCK",
    "PLATFORM_BEDROCK_h",
    "PLATFORM_X360",
    "PLATFORM_X360_h",
    "PLATFORM_PS3",
    "PLATFORM_PS3_h",
    "PLATFORM_WIIU",
    "PLATFORM_WIIU_h",
];

static TEXTURES: OnceLock<HashMap<&'static str, TextureHandle>> = OnceLock::new();

/// Premultiplied source pixels of the bundled images, for resampling.
static SOURCES: OnceLock<HashMap<&'static str, image::RgbaImage>> = OnceLock::new();

/// Bundled images resampled to an exact pixel size, by (name, width, height).
type SizedCache = HashMap<(String, u32, u32), TextureHandle>;
static SIZED: OnceLock<Mutex<SizedCache>> = OnceLock::new();

fn premultiply(mut img: image::RgbaImage) -> image::RgbaImage {
    for p in img.pixels_mut() {
        let a = p[3] as u16;
        for c in 0..3 {
            p[c] = ((p[c] as u16 * a + 127) / 255) as u8;
        }
    }
    img
}

/// Draws `src` (premultiplied) fitted in `r`, resampled with Lanczos to the exact number of
/// screen pixels and snapped to the pixel grid, so downscaled artwork stays sharp the way
/// Qt's smooth scaling shows it. `full` is the full-size texture used when not shrinking.
fn paint_sharp(
    p: &Painter,
    r: Rect,
    key: &str,
    src: &image::RgbaImage,
    full: &TextureHandle,
    tint: Color32,
) {
    let (w, h) = (src.width() as f32, src.height() as f32);
    let scale = (r.width() / w).min(r.height() / h);
    let ppp = p.ctx().pixels_per_point();
    let px = (vec2(w, h) * scale * ppp).round();
    let (pw, ph) = (px.x.max(1.0) as u32, px.y.max(1.0) as u32);
    let size = px / ppp;
    let min = ((r.center() - size / 2.0) * ppp).round() / ppp;
    let fitted = Rect::from_min_size(min, size);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if pw >= src.width() || ph >= src.height() {
        p.image(full.id(), fitted, uv, tint);
        return;
    }
    let cache = SIZED.get_or_init(Default::default);
    let tex = cache
        .lock()
        .expect("sized icon cache")
        .entry((key.to_string(), pw, ph))
        .or_insert_with(|| {
            let small = image::imageops::resize(src, pw, ph, image::imageops::FilterType::Lanczos3);
            let color = egui::ColorImage::from_rgba_premultiplied(
                [pw as usize, ph as usize],
                small.as_raw(),
            );
            p.ctx().load_texture(
                format!("{key}@{pw}x{ph}"),
                color,
                egui::TextureOptions::LINEAR,
            )
        })
        .clone();
    p.image(tex.id(), fitted, uv, tint);
}

/// Linear filtering with mipmaps, so large source images stay smooth when drawn small.
const SMOOTH: egui::TextureOptions = egui::TextureOptions {
    mipmap_mode: Some(egui::TextureFilter::Linear),
    ..egui::TextureOptions::LINEAR
};

/// Images loaded from disk (extension icons, Minecraft textures), by path.
static FILES: OnceLock<Mutex<HashMap<PathBuf, Option<TextureHandle>>>> = OnceLock::new();

/// Draws an image file fitted in `r` (loaded once, then cached); false if it can't be read.
/// Small pixel-art images are drawn with nearest filtering, as UMT shows them.
pub fn file_image(p: &Painter, r: Rect, path: &Path) -> bool {
    let Some(tex) = file_texture(p.ctx(), path) else {
        return false;
    };
    let [w, h] = tex.size();
    if w > 32 {
        let key = path.display().to_string();
        let src = FILE_SOURCES
            .get_or_init(Default::default)
            .lock()
            .expect("file sources")
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                Arc::new(
                    image::open(path)
                        .map(|i| premultiply(i.to_rgba8()))
                        .unwrap_or_default(),
                )
            })
            .clone();
        if src.width() > 0 {
            paint_sharp(p, r, &key, &src, &tex, Color32::WHITE);
            return true;
        }
    }
    let scale = (r.width() / w as f32).min(r.height() / h as f32);
    let fitted = Rect::from_center_size(r.center(), vec2(w as f32, h as f32) * scale);
    p.image(
        tex.id(),
        fitted,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    true
}

static FILE_SOURCES: OnceLock<Mutex<HashMap<PathBuf, Arc<image::RgbaImage>>>> = OnceLock::new();

static PACK: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Sets the resource pack textures folder (`…/assets/minecraft/textures`).
pub fn set_texture_pack(dir: Option<PathBuf>) {
    let mut pack = PACK.lock().expect("pack");
    if *pack != dir {
        *pack = dir;
    }
}

/// A file in the resource pack's textures folder, such as `block/netherrack.png`.
pub fn pack_texture(name: &str) -> Option<PathBuf> {
    let p = PACK.lock().expect("pack").as_ref()?.join(name);
    p.is_file().then_some(p)
}

/// A file at the resource pack's root, such as `pack.png`.
pub fn pack_file(name: &str) -> Option<PathBuf> {
    let textures = PACK.lock().expect("pack").clone()?;
    let p = textures.parent()?.parent()?.parent()?.join(name);
    p.is_file().then_some(p)
}

/// Draws an isometric block from the pack's top and side textures; false if missing.
/// The top is multiplied by `tint` (grass is greyscale in the pack).
pub fn block(p: &Painter, r: Rect, top: &str, side: &str, tint: Color32) -> bool {
    let ctx = p.ctx();
    let (Some(t), Some(s)) = (
        pack_texture(top).and_then(|f| file_texture(ctx, &f)),
        pack_texture(side).and_then(|f| file_texture(ctx, &f)),
    ) else {
        return false;
    };
    // Animated textures are vertical strips; use the first frame.
    let uv_h = |tex: &TextureHandle| {
        let [w, h] = tex.size();
        (w as f32 / h as f32).min(1.0)
    };
    let c = r.center();
    let k = r.width() * 0.46;
    let (n, e, s_, w) = (
        pos2(c.x, c.y - k),
        pos2(c.x + k * 0.87, c.y - k * 0.5),
        pos2(c.x, c.y),
        pos2(c.x - k * 0.87, c.y - k * 0.5),
    );
    let down = vec2(0.0, k);
    let face = |tex: &TextureHandle, q: [Pos2; 4], color: Color32| {
        let v = uv_h(tex);
        let mut mesh = egui::Mesh::with_texture(tex.id());
        let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, v), pos2(0.0, v)];
        for (pt, uv) in q.iter().zip(uvs) {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: *pt,
                uv,
                color,
            });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        p.add(Shape::mesh(mesh));
    };
    let shade = |f: f32| Color32::from_gray((255.0 * f) as u8);
    face(&t, [w, n, e, s_], tint);
    face(&s, [w, s_, s_ + down, w + down], shade(0.8));
    face(&s, [s_, e, e + down, s_ + down], shade(0.6));
    true
}

/// The texture for an image file, loaded once.
pub fn file_texture(ctx: &egui::Context, path: &Path) -> Option<TextureHandle> {
    let cache = FILES.get_or_init(Default::default);
    let mut map = cache.lock().expect("icon cache");
    map.entry(path.to_path_buf())
        .or_insert_with(|| {
            let img = image::open(path).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            let options = if size[0] <= 32 {
                egui::TextureOptions::NEAREST
            } else {
                SMOOTH
            };
            Some(ctx.load_texture(path.display().to_string(), color, options))
        })
        .clone()
}

/// Decodes the bundled images into textures; call once at startup.
pub fn install_textures(ctx: &egui::Context) {
    SOURCES.get_or_init(|| {
        IMAGES
            .iter()
            .filter_map(|(name, bytes)| {
                let img = image::load_from_memory(bytes).ok()?.to_rgba8();
                Some((*name, premultiply(img)))
            })
            .collect()
    });
    TEXTURES.get_or_init(|| {
        IMAGES
            .iter()
            .filter_map(|(name, bytes)| {
                let img = image::load_from_memory(bytes).ok()?.to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                Some((*name, ctx.load_texture(*name, color, SMOOTH)))
            })
            .collect()
    });
}

/// Draws a bundled image fitted (aspect kept) and centred in `r`.
pub fn image(p: &Painter, r: Rect, name: &str, tint: Color32) -> bool {
    let Some(tex) = TEXTURES.get().and_then(|t| t.get(name)) else {
        return false;
    };
    match SOURCES.get().and_then(|s| s.get(name)) {
        Some(src) => paint_sharp(p, r, name, src, tex, tint),
        None => {
            let [w, h] = tex.size();
            let scale = (r.width() / w as f32).min(r.height() / h as f32);
            let fitted = Rect::from_center_size(r.center(), vec2(w as f32, h as f32) * scale);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            p.image(tex.id(), fitted, uv, tint);
        }
    }
    true
}

/// The image for an icon, if UMT's artwork covers it.
fn image_name(icon: Icon) -> Option<&'static str> {
    Some(match icon {
        Icon::Java => "PLATFORM_JAVA",
        Icon::Bedrock => "PLATFORM_BEDROCK",
        Icon::Xbox360 => "PLATFORM_X360",
        Icon::PlayStation3 => "PLATFORM_PS3",
        Icon::WiiU => "PLATFORM_WIIU",
        Icon::Folder => "FOLDER",
        Icon::Refresh => "REFRESH",
        Icon::Plus => "ADD",
        Icon::Close => "CLOSE",
        Icon::Cross => "EDITION_CROSS",
        Icon::Check => "EDITION_CHECK",
        Icon::ChevronLeft => "STEP_PREV",
        Icon::ChevronRight => "ARROW_RIGHT",
        Icon::Save => "SAVE",
        Icon::Gear => "SETTINGS",
        Icon::Extensions => "EXTENSIONS",
        Icon::ChunkGrid => "CHUNK_LOCATOR",
        Icon::Search => "FIND",
        Icon::Trash => "TRASH",
        Icon::Info => "INFO",
        Icon::ShapeBox => "PRUNER_BOX",
        Icon::ShapeCircle => "PRUNER_CIRCLE",
        Icon::ShapePolygon => "PRUNER_POLYGON",
        Icon::ShapeSplit => "PRUNER_SPLIT",
        Icon::Time => "PRUNER_TIME",
        Icon::Optimize => "PRUNER_OPTIMIZE",
        Icon::Undo => "UNDO",
        Icon::Redo => "REDO",
        Icon::Compound => "TAG_COMPOUND",
        Icon::List => "TAG_LIST",
        Icon::File => "FILE",
        Icon::Layers => "SLICER",
        Icon::Minimize => "MIN",
        Icon::Maximize | Icon::Restore => "MAX",
        Icon::User => "AUTHOR",
        Icon::ArrowRight => "CONVERTER_ARROW",
        // Drawn as lines, so they take any colour and can point either way.
        Icon::Logo
        | Icon::AppEditor
        | Icon::AppConverter
        | Icon::AppPruner
        | Icon::ChevronDown
        | Icon::ChevronUp => return None,
    })
}

/// Draws `icon` from UMT's artwork: white asks for the highlighted (`_h`) variant, the dim
/// greys fade it as disabled. Returns false when there is no image for the icon.
fn paint_image(p: &Painter, r: Rect, icon: Icon, c: Color32) -> bool {
    let Some(name) = image_name(icon) else {
        return false;
    };
    let disabled = c == theme::TEXT_DIM || c == theme::CHIP;
    let tint = if disabled {
        Color32::from_white_alpha(70)
    } else {
        Color32::WHITE
    };
    if c == Color32::WHITE && image(p, r, &format!("{name}_h"), tint) {
        return true;
    }
    image(p, r, name, tint)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Logo,
    AppEditor,
    AppConverter,
    AppPruner,
    Java,
    Bedrock,
    Xbox360,
    PlayStation3,
    WiiU,
    Folder,
    Refresh,
    Plus,
    Close,
    Check,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    ChevronUp,
    Save,
    Gear,
    Extensions,
    ChunkGrid,
    Search,
    Trash,
    Info,
    ShapeBox,
    ShapeCircle,
    ShapePolygon,
    ShapeSplit,
    Time,
    Optimize,
    Undo,
    Redo,
    Compound,
    List,
    File,
    Layers,
    Minimize,
    Maximize,
    Restore,
    User,
    ArrowRight,
    /// Red/grey "not supported" cross (extension editions).
    Cross,
}

struct Pen<'a> {
    p: &'a Painter,
    r: Rect,
    c: Color32,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.r.min + vec2(x * self.r.width(), y * self.r.height())
    }
    fn w(&self) -> f32 {
        (self.r.width() * 0.075).clamp(1.2, 4.0)
    }
    fn stroke(&self) -> Stroke {
        Stroke::new(self.w(), self.c)
    }
    fn line(&self, pts: &[(f32, f32)]) {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.p.add(Shape::line(pts, self.stroke()));
    }
    fn closed(&self, pts: &[(f32, f32)]) {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.p.add(Shape::closed_line(pts, self.stroke()));
    }
    fn fill(&self, pts: &[(f32, f32)], c: Color32) {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(self.at(x0, y0), self.at(x1, y1))
    }
    fn rect_stroke(&self, x0: f32, y0: f32, x1: f32, y1: f32, round: u8) {
        self.p.rect_stroke(
            self.rect(x0, y0, x1, y1),
            theme::radius(round),
            self.stroke(),
            eframe::egui::StrokeKind::Middle,
        );
    }
    fn rect_fill(&self, x0: f32, y0: f32, x1: f32, y1: f32, round: u8, c: Color32) {
        self.p
            .rect_filled(self.rect(x0, y0, x1, y1), theme::radius(round), c);
    }
    fn circle(&self, x: f32, y: f32, r: f32) {
        self.p
            .circle_stroke(self.at(x, y), r * self.r.width(), self.stroke());
    }
    fn dot(&self, x: f32, y: f32, r: f32, c: Color32) {
        self.p.circle_filled(self.at(x, y), r * self.r.width(), c);
    }
    fn arc(&self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
        (0..=24)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / 24.0;
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    }
    /// Arrow head at `tip` pointing along angle `a`.
    fn head(&self, tip: (f32, f32), a: f32) {
        let s = 0.16;
        let l = (tip.0 - s * (a - 0.5).cos(), tip.1 - s * (a - 0.5).sin());
        let r = (tip.0 - s * (a + 0.5).cos(), tip.1 - s * (a + 0.5).sin());
        self.line(&[l, tip, r]);
    }
    /// Diagonal hatching clipped to a circle of radius `r` around (cx, cy).
    fn hatch_circle(&self, cx: f32, cy: f32, r: f32) {
        for k in -3..=3 {
            let d = k as f32 * r / 3.2;
            // Line x - y = d (rotated 45°): parametrise along the direction (1, 1)/√2.
            let half = (r * r - d * d / 2.0).max(0.0).sqrt();
            let (mx, my) = (cx + d / 2.0, cy - d / 2.0);
            let t = half / 2f32.sqrt();
            self.line(&[(mx - t, my - t), (mx + t, my + t)]);
        }
    }
}

pub fn paint(p: &Painter, r: Rect, icon: Icon, c: Color32) {
    if paint_image(p, r, icon, c) {
        return;
    }
    let pen = Pen { p, r, c };
    let grey = theme::TEXT_MUTED;
    let bg = theme::BG;
    match icon {
        Icon::Logo => {
            // The original box mark. See `examples/gen_logo.rs` for the matching
            // high-resolution render used for the window/taskbar/exe icon.
            pen.rect_fill(0.05, 0.05, 0.95, 0.95, 4, theme::BLUE);
            let w = Pen {
                p,
                r,
                c: Color32::WHITE,
            };
            w.closed(&[
                (0.5, 0.2),
                (0.78, 0.36),
                (0.78, 0.68),
                (0.5, 0.84),
                (0.22, 0.68),
                (0.22, 0.36),
            ]);
            w.line(&[(0.22, 0.36), (0.5, 0.52), (0.78, 0.36)]);
            w.line(&[(0.5, 0.52), (0.5, 0.84)]);
        }
        Icon::AppEditor => {
            let frame = Pen { p, r, c: grey };
            frame.rect_stroke(0.14, 0.12, 0.86, 0.88, 3);
            pen.line(&[(0.36, 0.28), (0.36, 0.72)]);
            for y in [0.3, 0.5, 0.7] {
                pen.line(&[(0.36, y), (0.6, y)]);
                pen.rect_fill(0.6, y - 0.06, 0.72, y + 0.06, 1, c);
            }
        }
        Icon::AppConverter => {
            // A square loop (Minecraft is blocky), two brackets flowing clockwise, each
            // ending in an arrowhead. The brackets stop clear of each other's arrowhead
            // (which has some width behind its tip), leaving a visible gap on each side.
            pen.line(&[(0.22, 0.42), (0.22, 0.22), (0.78, 0.22), (0.78, 0.42)]);
            pen.head((0.78, 0.42), PI / 2.0);
            let g = Pen { p, r, c: grey };
            g.line(&[(0.78, 0.58), (0.78, 0.78), (0.22, 0.78), (0.22, 0.58)]);
            g.head((0.22, 0.58), -PI / 2.0);
        }
        Icon::AppPruner => {
            for row in 0..3 {
                for col in 0..3 {
                    let (x, y) = (0.14 + col as f32 * 0.25, 0.14 + row as f32 * 0.25);
                    if col == 2 || (row == 0 && col == 1) {
                        pen.rect_fill(x, y, x + 0.2, y + 0.2, 1, c);
                    } else {
                        Pen { p, r, c: grey }.rect_stroke(x, y, x + 0.2, y + 0.2, 1);
                    }
                }
            }
        }
        Icon::Java => {
            pen.rect_fill(0.08, 0.3, 0.92, 0.72, 3, c);
            for (row, n) in [(0.4, 7), (0.51, 7)] {
                for i in 0..n {
                    let x = 0.15 + i as f32 * 0.105;
                    pen.rect_fill(x, row, x + 0.06, row + 0.06, 1, bg);
                }
            }
            pen.rect_fill(0.28, 0.61, 0.72, 0.66, 1, bg);
        }
        Icon::Bedrock => {
            pen.rect_stroke(0.08, 0.2, 0.66, 0.62, 2);
            pen.line(&[(0.37, 0.62), (0.37, 0.74)]);
            pen.line(&[(0.24, 0.76), (0.5, 0.76)]);
            pen.rect_fill(0.6, 0.38, 0.92, 0.88, 3, c);
            pen.rect_fill(0.64, 0.44, 0.88, 0.78, 1, bg);
        }
        Icon::Xbox360 | Icon::PlayStation3 => {
            pen.rect_fill(0.12, 0.3, 0.88, 0.6, 8, c);
            pen.dot(0.27, 0.6, 0.15, c);
            pen.dot(0.73, 0.6, 0.15, c);
            // D-pad and face buttons.
            pen.rect_fill(0.22, 0.42, 0.36, 0.47, 1, bg);
            pen.rect_fill(0.265, 0.375, 0.315, 0.515, 1, bg);
            let face = if icon == Icon::Xbox360 {
                [(0.7, 0.38), (0.64, 0.445), (0.76, 0.445), (0.7, 0.51)]
            } else {
                [(0.66, 0.4), (0.74, 0.4), (0.66, 0.49), (0.74, 0.49)]
            };
            for (x, y) in face {
                pen.dot(x, y, 0.032, bg);
            }
        }
        Icon::WiiU => {
            pen.rect_fill(0.06, 0.28, 0.94, 0.74, 10, c);
            pen.rect_fill(0.28, 0.34, 0.72, 0.68, 2, bg);
            pen.dot(0.16, 0.42, 0.04, bg);
            pen.dot(0.84, 0.42, 0.04, bg);
        }
        Icon::Folder => {
            pen.fill(
                &[
                    (0.08, 0.22),
                    (0.4, 0.22),
                    (0.48, 0.32),
                    (0.92, 0.32),
                    (0.92, 0.82),
                    (0.08, 0.82),
                ],
                c,
            );
            pen.rect_fill(0.08, 0.4, 0.92, 0.42, 0, bg);
        }
        Icon::Refresh => {
            let a = pen.arc(0.5, 0.5, 0.3, -PI * 0.35, PI * 1.45);
            pen.line(&a);
            pen.head(*a.last().unwrap(), PI * 1.45 + PI / 2.0);
        }
        Icon::Plus => {
            pen.line(&[(0.5, 0.18), (0.5, 0.82)]);
            pen.line(&[(0.18, 0.5), (0.82, 0.5)]);
        }
        Icon::Close | Icon::Cross => {
            pen.line(&[(0.24, 0.24), (0.76, 0.76)]);
            pen.line(&[(0.76, 0.24), (0.24, 0.76)]);
        }
        Icon::Check => pen.line(&[(0.2, 0.52), (0.42, 0.72), (0.8, 0.3)]),
        Icon::ChevronLeft => pen.line(&[(0.62, 0.2), (0.34, 0.5), (0.62, 0.8)]),
        Icon::ChevronRight => pen.line(&[(0.38, 0.2), (0.66, 0.5), (0.38, 0.8)]),
        Icon::ChevronDown => pen.line(&[(0.22, 0.38), (0.5, 0.64), (0.78, 0.38)]),
        Icon::ChevronUp => pen.line(&[(0.22, 0.62), (0.5, 0.36), (0.78, 0.62)]),
        Icon::Save => {
            pen.rect_stroke(0.16, 0.16, 0.84, 0.84, 2);
            pen.rect_fill(0.3, 0.16, 0.66, 0.38, 0, c);
            pen.rect_stroke(0.3, 0.56, 0.7, 0.84, 0);
        }
        Icon::Gear => {
            pen.circle(0.5, 0.5, 0.2);
            for i in 0..8 {
                let a = i as f32 * TAU / 8.0;
                pen.line(&[
                    (0.5 + 0.26 * a.cos(), 0.5 + 0.26 * a.sin()),
                    (0.5 + 0.38 * a.cos(), 0.5 + 0.38 * a.sin()),
                ]);
            }
            pen.dot(0.5, 0.5, 0.07, c);
        }
        Icon::Extensions => {
            pen.rect_stroke(0.14, 0.14, 0.44, 0.44, 1);
            pen.rect_stroke(0.14, 0.56, 0.44, 0.86, 1);
            pen.rect_stroke(0.56, 0.56, 0.86, 0.86, 1);
            pen.line(&[(0.71, 0.12), (0.71, 0.46)]);
            pen.line(&[(0.54, 0.29), (0.88, 0.29)]);
        }
        Icon::ChunkGrid => {
            pen.rect_stroke(0.14, 0.14, 0.44, 0.44, 2);
            pen.rect_stroke(0.56, 0.14, 0.86, 0.44, 2);
            pen.rect_stroke(0.14, 0.56, 0.44, 0.86, 2);
            pen.rect_fill(0.56, 0.56, 0.86, 0.86, 2, c);
        }
        Icon::Search => {
            pen.circle(0.43, 0.43, 0.24);
            pen.line(&[(0.61, 0.61), (0.84, 0.84)]);
        }
        Icon::Trash => {
            pen.line(&[(0.18, 0.26), (0.82, 0.26)]);
            pen.line(&[(0.4, 0.26), (0.4, 0.16), (0.6, 0.16), (0.6, 0.26)]);
            pen.closed(&[(0.26, 0.3), (0.74, 0.3), (0.7, 0.86), (0.3, 0.86)]);
            pen.line(&[(0.43, 0.42), (0.43, 0.74)]);
            pen.line(&[(0.57, 0.42), (0.57, 0.74)]);
        }
        Icon::Info => {
            pen.circle(0.5, 0.5, 0.36);
            pen.dot(0.5, 0.32, 0.05, c);
            pen.line(&[(0.5, 0.44), (0.5, 0.72)]);
        }
        Icon::ShapeBox => {
            pen.rect_stroke(0.14, 0.14, 0.86, 0.86, 1);
            for k in 1..5 {
                let t = k as f32 * 0.18;
                pen.line(&[(0.14, 0.14 + t), (0.14 + t, 0.14)]);
                pen.line(&[(0.14 + t, 0.86), (0.86, 0.14 + t)]);
            }
        }
        Icon::ShapeCircle => {
            pen.circle(0.5, 0.5, 0.36);
            pen.hatch_circle(0.5, 0.5, 0.36);
        }
        Icon::ShapePolygon => {
            pen.closed(&[
                (0.12, 0.3),
                (0.62, 0.14),
                (0.88, 0.5),
                (0.7, 0.86),
                (0.22, 0.7),
            ]);
            pen.line(&[(0.3, 0.42), (0.6, 0.3)]);
            pen.line(&[(0.32, 0.62), (0.74, 0.52)]);
        }
        Icon::ShapeSplit => {
            pen.line(&[(0.62, 0.1), (0.38, 0.9)]);
            for k in 0..4 {
                let y = 0.2 + k as f32 * 0.2;
                pen.line(&[(0.2, y + 0.12), (0.5 - (y - 0.1) * 0.3, y)]);
            }
        }
        Icon::Time => {
            pen.circle(0.5, 0.56, 0.32);
            pen.line(&[(0.5, 0.56), (0.5, 0.38)]);
            pen.line(&[(0.42, 0.14), (0.58, 0.14)]);
            pen.line(&[(0.5, 0.14), (0.5, 0.24)]);
        }
        Icon::Optimize => {
            let a = pen.arc(0.5, 0.62, 0.36, PI, TAU);
            pen.line(&a);
            pen.line(&[(0.5, 0.62), (0.68, 0.42)]);
            pen.dot(0.5, 0.62, 0.05, c);
        }
        Icon::Undo | Icon::Redo => {
            let flip = |x: f32| if icon == Icon::Undo { x } else { 1.0 - x };
            let a: Vec<(f32, f32)> = pen
                .arc(0.5, 0.58, 0.26, PI * 1.05, PI * 2.2)
                .into_iter()
                .map(|(x, y)| (flip(x), y))
                .collect();
            pen.line(&a);
            let tip = (flip(0.24), 0.56);
            pen.line(&[(flip(0.24), 0.34), tip, (flip(0.46), 0.56)]);
        }
        Icon::Compound => {
            pen.fill(
                &[
                    (0.1, 0.24),
                    (0.38, 0.24),
                    (0.46, 0.34),
                    (0.9, 0.34),
                    (0.9, 0.8),
                    (0.1, 0.8),
                ],
                theme::FOLDER,
            );
        }
        Icon::List => {
            for y in [0.28, 0.5, 0.72] {
                pen.dot(0.2, y, 0.06, c);
                pen.line(&[(0.36, y), (0.84, y)]);
            }
        }
        Icon::File => {
            pen.closed(&[
                (0.22, 0.12),
                (0.6, 0.12),
                (0.8, 0.32),
                (0.8, 0.88),
                (0.22, 0.88),
            ]);
            pen.line(&[(0.6, 0.12), (0.6, 0.32), (0.8, 0.32)]);
        }
        Icon::Layers => {
            pen.closed(&[(0.5, 0.18), (0.86, 0.36), (0.5, 0.54), (0.14, 0.36)]);
            pen.line(&[(0.14, 0.52), (0.5, 0.7), (0.86, 0.52)]);
            pen.line(&[(0.14, 0.66), (0.5, 0.84), (0.86, 0.66)]);
        }
        Icon::Minimize => pen.line(&[(0.3, 0.62), (0.7, 0.62)]),
        Icon::Maximize => pen.rect_stroke(0.3, 0.3, 0.7, 0.7, 0),
        Icon::Restore => {
            pen.rect_stroke(0.28, 0.38, 0.62, 0.72, 0);
            pen.line(&[
                (0.38, 0.38),
                (0.38, 0.28),
                (0.72, 0.28),
                (0.72, 0.62),
                (0.62, 0.62),
            ]);
        }
        Icon::User => {
            pen.dot(0.5, 0.36, 0.16, c);
            pen.fill(&[(0.2, 0.86), (0.28, 0.62), (0.72, 0.62), (0.8, 0.86)], c);
        }
        Icon::ArrowRight => {
            pen.line(&[(0.1, 0.5), (0.86, 0.5)]);
            pen.line(&[(0.6, 0.22), (0.88, 0.5), (0.6, 0.78)]);
        }
    }
}

/// Text badge used for NBT tag types ("B", "I", "Aa", …).
pub fn badge(p: &Painter, r: Rect, text: &str, fill: Color32, bracketed: bool) {
    if bracketed {
        p.rect_stroke(
            r.shrink(1.0),
            theme::radius(2),
            Stroke::new(1.2_f32, fill),
            eframe::egui::StrokeKind::Middle,
        );
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            text,
            theme::bold(9.5),
            fill,
        );
    } else if fill == Color32::TRANSPARENT {
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            text,
            theme::bold(11.0),
            theme::TEXT,
        );
    } else {
        p.rect_filled(r.shrink(1.0), theme::radius(2), fill);
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            text,
            theme::bold(9.5),
            Color32::WHITE,
        );
    }
}

/// The point on a hexagon (pointy top) of radius `r` around `c`.
pub fn hexagon(c: Pos2, r: f32) -> Vec<Pos2> {
    (0..6)
        .map(|i| {
            let a = PI / 6.0 + i as f32 * PI / 3.0;
            pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}
