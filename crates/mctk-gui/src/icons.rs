//! Icons: most are SVG images (`assets/icons`, drawn by `resvg` at the exact size needed); the
//! app logo, app icons and any icon without an image are drawn from lines.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use eframe::egui::{
    self, Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, TextureHandle, pos2, vec2,
};

use crate::theme;

mod bundled {
    include!(concat!(env!("OUT_DIR"), "/icon_list.rs"));
}

/// The parsed vector icons by file name (`ADD`, `ADD_h`, ...).
static TREES: OnceLock<HashMap<&'static str, resvg::usvg::Tree>> = OnceLock::new();

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
/// Small pixel-art images are drawn with nearest filtering.
pub fn file_image(p: &Painter, r: Rect, path: &Path) -> bool {
    file_image_tinted(p, r, path, Color32::WHITE)
}

/// Draws an image file in gray (its colors as light and dark only), first frame of an
/// animated one.
pub fn gray_image(p: &Painter, r: Rect, path: &Path) -> bool {
    static GRAY: OnceLock<Mutex<HashMap<PathBuf, Option<TextureHandle>>>> = OnceLock::new();
    let tex = GRAY
        .get_or_init(Default::default)
        .lock()
        .expect("gray icons")
        .entry(path.to_path_buf())
        .or_insert_with(|| {
            let img = image::open(path).ok()?.to_rgba8();
            let side = img.width().min(img.height()).max(1);
            let mut img = image::imageops::crop_imm(&img, 0, 0, side, side).to_image();
            for px in img.pixels_mut() {
                let l = (0.3 * f32::from(px[0]) + 0.59 * f32::from(px[1]) + 0.11 * f32::from(px[2])) as u8;
                px[0] = l;
                px[1] = l;
                px[2] = l;
            }
            let color = egui::ColorImage::from_rgba_unmultiplied([side as usize, side as usize], img.as_raw());
            Some(p.ctx().load_texture(format!("gray:{}", path.display()), color, egui::TextureOptions::NEAREST))
        })
        .clone();
    let Some(tex) = tex else { return false };
    let side = r.width().min(r.height());
    p.image(
        tex.id(),
        Rect::from_center_size(r.center(), vec2(side, side)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    true
}

/// Like [`file_image`], multiplied by `tint`.
pub fn file_image_tinted(p: &Painter, r: Rect, path: &Path, tint: Color32) -> bool {
    let Some(tex) = file_texture(p.ctx(), path) else {
        return false;
    };
    let [w, full_h] = tex.size();
    // An animated texture is a strip of frames: the first one is the picture.
    let h = full_h.min(w);
    let top = h as f32 / full_h as f32;
    if w > 32 && full_h <= w {
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
            paint_sharp(p, r, &key, &src, &tex, tint);
            return true;
        }
    }
    let scale = (r.width() / w as f32).min(r.height() / h as f32);
    let fitted = Rect::from_center_size(r.center(), vec2(w as f32, h as f32) * scale);
    p.image(
        tex.id(),
        fitted,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, top)),
        tint,
    );
    true
}

static FILE_SOURCES: OnceLock<Mutex<HashMap<PathBuf, Arc<image::RgbaImage>>>> = OnceLock::new();

/// A file of the game's assets beside the textures (`models/block/stone.json`,
/// `blockstates/...`), the latest release's (see `assets`).
pub fn pack_asset(rel: &str) -> Option<PathBuf> {
    crate::assets::fetch(&format!("assets/minecraft/{rel}"))
}

/// A texture of the game, such as `block/netherrack.png`, the latest release's (see `assets`).
pub fn pack_texture(name: &str) -> Option<PathBuf> {
    crate::assets::fetch(&format!("assets/minecraft/textures/{name}"))
}

/// The texture of an item or block by its id (`minecraft:diamond_sword` or `diamond_sword`):
/// its item picture, else its block picture. Looked up once per name.
pub fn item_texture(id: &str) -> Option<PathBuf> {
    static CACHE: LazyLock<Mutex<HashMap<String, Option<PathBuf>>>> = LazyLock::new(Default::default);
    let name = id.rsplit(':').next().unwrap_or(id).to_ascii_lowercase();
    if let Some(found) = CACHE.lock().expect("item textures").get(&name) {
        return found.clone();
    }
    let found = find_item_texture(&name);
    CACHE.lock().expect("item textures").insert(name, found.clone());
    found
}

/// Draws an item (by id) as the game's inventory shows it: its pictures for a flat item; a
/// block, or a thing the game draws in code (chests, heads), seen from above at an angle; a
/// block that is no item (a crop, a candle cake) by its block's model; else the picture its
/// name suggests. Multiplied by `tint` (enchanted items are purple). False when there is
/// nothing to draw.
pub fn item_icon(p: &Painter, r: Rect, id: &str, tint: Color32) -> bool {
    let model = crate::models::item_model(id);
    if let Some(crate::models::ItemModel::Flat(layers)) = model.as_deref() {
        let mut drawn = false;
        for (k, layer) in layers.iter().enumerate() {
            if let Some(f) = pack_texture(layer) {
                // Dyed layers (leather, potions) as the plain undyed color the game shows.
                let color = if k > 0 && layer_tinted(id, k) { Color32::from_rgb(0xA0, 0x65, 0x40) } else { tint };
                drawn |= file_image_tinted(p, r, &f, color);
            }
        }
        if drawn {
            return true;
        }
    }
    let bare = id.trim_start_matches("minecraft:");
    // The shield: the face of its board, cut from its picture.
    if bare == "shield" {
        if let Some(tex) = pack_texture("entity/shield/shield_base_nopattern.png").and_then(|f| file_texture(p.ctx(), &f)) {
            let side = r.height().min(r.width());
            let face = Rect::from_center_size(r.center(), vec2(side * 12.0 / 22.0, side));
            p.image(tex.id(), face, Rect::from_min_max(pos2(1.0 / 64.0, 1.0 / 64.0), pos2(13.0 / 64.0, 23.0 / 64.0)), tint);
            return true;
        }
    }
    if let Some(tex) = solid_icon(p.ctx(), bare, model.as_deref(), false) {
        let side = r.width().min(r.height());
        p.image(tex.id(), Rect::from_center_size(r.center(), vec2(side, side)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        return true;
    }
    item_texture(id).is_some_and(|f| file_image_tinted(p, r, &f, tint))
}

/// The picture of an item drawn solid (a block model, a thing drawn in code, or the block of a
/// name that is no item), made once and kept; `None` when it is none of those.
fn solid_icon(ctx: &egui::Context, bare: &str, model: Option<&crate::models::ItemModel>, gray: bool) -> Option<TextureHandle> {
    static ICONS: LazyLock<Mutex<HashMap<(String, bool), Option<TextureHandle>>>> = LazyLock::new(Default::default);
    if let Some(t) = ICONS.lock().ok()?.get(&(bare.to_string(), gray)) {
        return t.clone();
    }
    let leaf = crate::world::block_tint(&format!("{bare}@plains"), "");
    let from_model = |m: &crate::models::Model| -> Vec<IconQuad> { m.quads.iter().map(|q| (q.pos, q.uv, q.tex.clone(), if q.tint { leaf } else { [1.0; 3] })).collect() };
    let (quads, fit): (Vec<IconQuad>, bool) = match model {
        Some(crate::models::ItemModel::Model(m)) if !m.quads.is_empty() => (from_model(m), false),
        Some(crate::models::ItemModel::Flat(_)) => (Vec::new(), false),
        // Drawn in code by the game (chests, heads, the pot, statues): the same boxes the world
        // view builds, turned with their front to the viewer (heads and statues count their
        // turn from the other side to chests).
        _ if crate::entities::is_block_entity(bare) => {
            let front = if bare.ends_with("_head") || bare.ends_with("_skull") || bare.ends_with("copper_golem_statue") {
                "facing=south,type=single,rotation=0"
            } else {
                "facing=north,type=single,rotation=8"
            };
            let quads = crate::entities::block_entity(bare, front, None).unwrap_or_default();
            (quads.into_iter().map(|q| (q.pos, q.uv, q.tex, q.tint)).collect(), true)
        }
        // A block that is no item: its block, in its first state.
        _ => match crate::models::baked(bare, &crate::models::shown_state(bare)) {
            Some(b) => (from_model(b.pick(0, 0, 0)), false),
            None => (Vec::new(), false),
        },
    };
    let made = if quads.is_empty() { None } else { render_icon(&quads, fit) }
        .map(|mut img| {
            if gray {
                for px in &mut img.pixels {
                    let [r, g, b, a] = px.to_array();
                    let l = (0.3 * r as f32 + 0.59 * g as f32 + 0.11 * b as f32) as u8;
                    *px = Color32::from_rgba_premultiplied(l, l, l, a);
                }
            }
            ctx.load_texture(format!("item-icon:{bare}:{gray}"), img, egui::TextureOptions::LINEAR)
        });
    ICONS.lock().ok()?.insert((bare.to_string(), gray), made.clone());
    made
}

/// Like [`item_icon`], in gray (light and dark only), as the menus' own icons are.
pub fn item_icon_gray(p: &Painter, r: Rect, id: &str) -> bool {
    let model = crate::models::item_model(id);
    if let Some(crate::models::ItemModel::Flat(layers)) = model.as_deref() {
        if layers.first().and_then(|l| pack_texture(l)).is_some_and(|f| gray_image(p, r, &f)) {
            return true;
        }
    }
    let bare = id.trim_start_matches("minecraft:");
    if let Some(tex) = solid_icon(p.ctx(), bare, model.as_deref(), true) {
        let side = r.width().min(r.height());
        p.image(tex.id(), Rect::from_center_size(r.center(), vec2(side, side)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        return true;
    }
    item_texture(id).is_some_and(|f| gray_image(p, r, &f))
}

/// A face to draw in an icon: its corners (clockwise from outside, in the block's space), each
/// corner's place in its picture, the picture and its color.
type IconQuad = ([[f32; 3]; 4], [[f32; 2]; 4], Arc<str>, [f32; 3]);

/// Whether an item's layer `k` takes a dye color (left at a plain one here).
fn layer_tinted(id: &str, k: usize) -> bool {
    let n = id.trim_start_matches("minecraft:");
    k == 1 && (n.starts_with("leather_") || n.contains("potion") || n == "tipped_arrow" || n.ends_with("firework_star"))
}

/// A picture of the pack as pixels (for drawing icons), read once.
fn pixels(tex: &str) -> Option<Arc<image::RgbaImage>> {
    static PIXELS: LazyLock<Mutex<HashMap<String, Option<Arc<image::RgbaImage>>>>> = LazyLock::new(Default::default);
    if let Some(p) = PIXELS.lock().ok()?.get(tex) {
        return p.clone();
    }
    let img = pack_texture(tex).and_then(|f| image::open(f).ok()).map(|i| Arc::new(i.to_rgba8()));
    PIXELS.lock().ok()?.insert(tex.to_string(), img.clone());
    img
}

/// Size of a made icon, in pixels; drawn at twice that and averaged down, for smooth edges.
const ICON: usize = 64;

/// Faces drawn as the inventory draws a block: turned 225 degrees about up and tipped 30
/// degrees towards the viewer, its top lit fully and its sides darker, each pixel the nearest
/// face's (so a chest's lock stands in front of its lid). At the game's size for a block, or
/// (`fit`, for the smaller things drawn in code) filling the picture.
fn render_icon(quads: &[IconQuad], fit: bool) -> Option<egui::ColorImage> {
    render_icon_sized(quads, fit, ICON)
}

/// What a mob's (or anything's) faces look like from the inventory's angle, filling a picture
/// `px` across (for looking at models in tests).
#[cfg(test)]
pub(crate) fn render_quads(quads: &[crate::entities::EQuad], px: usize) -> Option<egui::ColorImage> {
    let q: Vec<IconQuad> = quads.iter().map(|q| (q.pos, q.uv, q.tex.clone(), q.tint)).collect();
    render_icon_sized(&q, true, px)
}

fn render_icon_sized(quads: &[IconQuad], fit: bool, icon: usize) -> Option<egui::ColorImage> {
    let n = icon * 2;
    let (sy, cy) = (225f32.to_radians().sin(), 225f32.to_radians().cos());
    let (sx, cx) = (30f32.to_radians().sin(), 30f32.to_radians().cos());
    // Into the view: about y, then x; x right, y up, z towards the viewer.
    let view = |q: [f32; 3]| {
        let (x, y, z) = (q[0] - 0.5, q[1] - 0.5, q[2] - 0.5);
        let (x1, z1) = (x * cy + z * sy, -x * sy + z * cy);
        let (y2, z2) = (y * cx - z1 * sx, y * sx + z1 * cx);
        [x1, y2, z2]
    };
    let side = n as f32;
    // Sized as the game sizes a block (5/8 of the slot across its corners), or to fill it.
    let (mut s, mut c) = (side * 0.625 * 1.05, [side / 2.0, side / 2.0]);
    if fit {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for q in quads {
            for corner in q.0 {
                let v = view(corner);
                lo = [lo[0].min(v[0]), lo[1].min(v[1])];
                hi = [hi[0].max(v[0]), hi[1].max(v[1])];
            }
        }
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-3);
        s = side * 0.92 / extent;
        c = [side / 2.0 - (lo[0] + hi[0]) / 2.0 * s, side / 2.0 + (lo[1] + hi[1]) / 2.0 * s];
    }
    let mut color = vec![[0.0f32; 4]; n * n];
    let mut depth = vec![f32::MIN; n * n];
    let mut any = false;
    for (pos, uvs, tex_file, color3) in quads {
        let v = pos.map(view);
        // Seen from behind: hidden.
        let (a, b) = ([v[1][0] - v[0][0], v[1][1] - v[0][1]], [v[3][0] - v[0][0], v[3][1] - v[0][1]]);
        if a[0] * b[1] - a[1] * b[0] > 0.0 {
            continue;
        }
        let Some(img) = pixels(tex_file) else { continue };
        let (tw, th) = (img.width() as f32, img.height() as f32);
        // An animated picture is a strip of frames: the first one.
        let frame = (tw / th).min(1.0);
        // Lit by which way it faces once turned: up full, the two sides seen less.
        let normal = {
            let (e, f) = ([v[1][0] - v[0][0], v[1][1] - v[0][1], v[1][2] - v[0][2]], [v[3][0] - v[0][0], v[3][1] - v[0][1], v[3][2] - v[0][2]]);
            let m = [e[1] * f[2] - e[2] * f[1], e[2] * f[0] - e[0] * f[2], e[0] * f[1] - e[1] * f[0]];
            let l = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt().max(1e-6);
            [m[0] / l, m[1] / l, m[2] / l]
        };
        let light = if normal[1].abs() > 0.7 { 1.0 } else if normal[0] < 0.0 { 0.8 } else { 0.6 };
        let screen = v.map(|p| [c[0] + p[0] * s, c[1] - p[1] * s, p[2]]);
        for tri in [[0, 1, 2], [0, 2, 3]] {
            let [p0, p1, p2] = tri.map(|i| screen[i]);
            let [t0, t1, t2] = tri.map(|i| uvs[i]);
            let area = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p1[1] - p0[1]) * (p2[0] - p0[0]);
            if area.abs() < 1e-6 {
                continue;
            }
            let x0 = p0[0].min(p1[0]).min(p2[0]).floor().max(0.0) as usize;
            let x1 = (p0[0].max(p1[0]).max(p2[0]).ceil() as usize).min(n);
            let y0 = p0[1].min(p1[1]).min(p2[1]).floor().max(0.0) as usize;
            let y1 = (p0[1].max(p1[1]).max(p2[1]).ceil() as usize).min(n);
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((p1[0] - px) * (p2[1] - py) - (p1[1] - py) * (p2[0] - px)) / area;
                    let w1 = ((p2[0] - px) * (p0[1] - py) - (p2[1] - py) * (p0[0] - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                        continue;
                    }
                    let z = p0[2] * w0 + p1[2] * w1 + p2[2] * w2;
                    let i = y * n + x;
                    if z <= depth[i] {
                        continue;
                    }
                    let u = t0[0] * w0 + t1[0] * w1 + t2[0] * w2;
                    let vv = (t0[1] * w0 + t1[1] * w1 + t2[1] * w2) * frame;
                    let tx = ((u * tw) as i64).clamp(0, tw as i64 - 1) as u32;
                    let ty = ((vv * th) as i64).clamp(0, th as i64 - 1) as u32;
                    let px = img.get_pixel(tx, ty).0;
                    let alpha = px[3] as f32 / 255.0;
                    if alpha < 0.1 {
                        continue;
                    }
                    let rgb = [0, 1, 2].map(|k| px[k] as f32 / 255.0 * light * color3[k]);
                    let d = &mut color[i];
                    if alpha >= 0.5 {
                        // Solid: it is what is seen here.
                        *d = [rgb[0], rgb[1], rgb[2], 1.0];
                        depth[i] = z;
                    } else {
                        // See-through (glass, ice): over what is behind it.
                        for k in 0..3 {
                            d[k] = rgb[k] * alpha + d[k] * (1.0 - alpha);
                        }
                        d[3] = alpha + d[3] * (1.0 - alpha);
                    }
                    any = true;
                }
            }
        }
    }
    if !any {
        return None;
    }
    // Averaged down to the icon's size, as premultiplied color.
    let mut out = Vec::with_capacity(icon * icon);
    for y in 0..icon {
        for x in 0..icon {
            let mut sum = [0.0f32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = color[(y * 2 + dy) * n + x * 2 + dx];
                for k in 0..3 {
                    sum[k] += p[k] * p[3];
                }
                sum[3] += p[3];
            }
            out.push(Color32::from_rgba_premultiplied(
                (sum[0] / 4.0 * 255.0) as u8,
                (sum[1] / 4.0 * 255.0) as u8,
                (sum[2] / 4.0 * 255.0) as u8,
                (sum[3] / 4.0 * 255.0) as u8,
            ));
        }
    }
    Some(egui::ColorImage { size: [icon, icon], pixels: out })
}

/// What a texture of the pack is called for items and blocks that have none of their own name:
/// a stair or slab is drawn as the block it is made of, a block by one of its faces, a bed or
/// banner by its color of wool.
fn find_item_texture(name: &str) -> Option<PathBuf> {
    // Items that look like another one.
    let alias = match name {
        "petrified_oak_slab" => "oak_planks",
        "smooth_quartz" | "smooth_quartz_slab" | "smooth_quartz_stairs" => "quartz_block_bottom",
        "smooth_sandstone" | "smooth_sandstone_slab" | "smooth_sandstone_stairs" => "sandstone_top",
        "smooth_red_sandstone" | "smooth_red_sandstone_slab" | "smooth_red_sandstone_stairs" => "red_sandstone_top",
        "heavy_weighted_pressure_plate" => "iron_block",
        "light_weighted_pressure_plate" => "gold_block",
        "enchanted_golden_apple" => "golden_apple",
        "debug_stick" => "stick",
        "magma_block" => "magma",
        "snow_block" => "snow",
        "chipped_anvil" => "chipped_anvil_top",
        "damaged_anvil" => "damaged_anvil_top",
        "sticky_piston" => "piston_top_sticky",
        "tipped_arrow" => "arrow",
        // Blocks with no look of their own in an inventory: what they are made of or in.
        "bubble_column" => "water",
        "end_portal" | "end_gateway" => "end_portal_frame",
        "moving_piston" => "piston",
        n if n.ends_with("_map") => "filled_map",
        n => n,
    };
    let mut bases = vec![alias.to_string()];
    let mut base = alias;
    for prefix in ["waxed_", "infested_"] {
        if let Some(rest) = base.strip_prefix(prefix) {
            base = rest;
            bases.push(base.to_string());
        }
    }
    for suffix in [
        "_stairs", "_slab", "_wall", "_fence_gate", "_fence", "_button", "_pressure_plate", "_shelf", "_pane",
    ] {
        if let Some(rest) = base.strip_suffix(suffix) {
            // A stained glass pane is the glass; the rest are made of a block named by the rest.
            bases.push(if suffix == "_pane" { rest.to_string() } else { rest.to_string() });
            base = rest;
            break;
        }
    }
    if let Some(rest) = base.strip_suffix("_wood") {
        bases.push(format!("{rest}_log"));
    }
    if let Some(rest) = base.strip_suffix("_hyphae") {
        bases.push(format!("{rest}_stem"));
    }
    for suffix in ["_bed", "_banner", "_carpet", "_bundle", "_harness", "_cushion"] {
        if let Some(color) = base.strip_suffix(suffix) {
            bases.push(format!("{color}_wool"));
        }
    }
    if let Some(rest) = base.strip_suffix("_block") {
        bases.push(rest.to_string());
    }
    // The block under a name, with the faces and endings the pack uses.
    const VARIANTS: [&str; 18] = [
        "", "_planks", "s", "_block", "_side", "_front", "_top", "_block_side", "_00", "_standby", "_side1",
        "_side0", "_south", "_front_off", "_side_inactive", "_bottom", "_still", "_0",
    ];
    for base in &bases {
        for variant in VARIANTS {
            for folder in ["item", "block"] {
                if let Some(found) = pack_texture(&format!("{folder}/{base}{variant}.png")) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// A file at MCToolKit's own resource pack's root, such as `pack.png`.
pub fn pack_file(name: &str) -> Option<PathBuf> {
    let p = crate::assets::root()?.join(name);
    p.is_file().then_some(p).or_else(|| crate::assets::fetch(name))
}

/// Draws an isometric block from the pack's top and side textures; false if missing.
/// The top is multiplied by `tint` (grass is grayscale in the pack).
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

/// Parses the bundled vector icons; call once at startup.
pub fn install_textures(_ctx: &egui::Context) {
    TREES.get_or_init(|| {
        let opt = resvg::usvg::Options::default();
        bundled::SVGS
            .iter()
            .filter_map(|(name, svg)| Some((*name, resvg::usvg::Tree::from_str(svg, &opt).ok()?)))
            .collect()
    });
}

/// Draws a bundled vector icon fitted (aspect kept) and centered in `r`. It is rendered at the
/// exact number of screen pixels it covers (and snapped to the pixel grid), so it stays sharp at
/// any size or zoom; each size is rendered once.
pub fn image(p: &Painter, r: Rect, name: &str, tint: Color32) -> bool {
    let Some(tree) = TREES.get().and_then(|t| t.get(name)) else {
        return false;
    };
    let (w, h) = (tree.size().width(), tree.size().height());
    let scale = (r.width() / w).min(r.height() / h);
    let ppp = p.ctx().pixels_per_point();
    let px = (vec2(w, h) * scale * ppp).round();
    let (pw, ph) = (px.x.clamp(1.0, 2048.0) as u32, px.y.clamp(1.0, 2048.0) as u32);
    let size = px / ppp;
    let min = ((r.center() - size / 2.0) * ppp).round() / ppp;
    let fitted = Rect::from_min_size(min, size);
    let cache = SIZED.get_or_init(Default::default);
    let tex = cache
        .lock()
        .expect("sized icon cache")
        .entry((name.to_string(), pw, ph))
        .or_insert_with(|| {
            let mut pixmap = resvg::tiny_skia::Pixmap::new(pw, ph).expect("icon size is at least 1x1");
            let to_size = resvg::tiny_skia::Transform::from_scale(pw as f32 / w, ph as f32 / h);
            resvg::render(tree, to_size, &mut pixmap.as_mut());
            // tiny-skia's pixels are premultiplied RGBA.
            let color = egui::ColorImage::from_rgba_premultiplied([pw as usize, ph as usize], pixmap.data());
            p.ctx().load_texture(format!("{name}@{pw}x{ph}"), color, egui::TextureOptions::LINEAR)
        })
        .clone();
    p.image(tex.id(), fitted, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    true
}

/// The image for an icon, if there is an image for it.
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
        // Drawn as lines, so they take any color and can point either way.
        Icon::Logo
        | Icon::MobRemove
        | Icon::AppEditor
        | Icon::AppConverter
        | Icon::ChevronDown
        | Icon::ChevronUp
        | Icon::ArrowUp
        | Icon::ArrowDown
        | Icon::Copy
        | Icon::Cut
        | Icon::Paste
        | Icon::Level
        | Icon::Rename => return None,
    })
}

/// Whether a bundled picture is plain: only grays and white (a glyph to color as wanted).
fn is_plain(name: &str) -> bool {
    static PLAIN: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Default::default);
    if let Some(v) = PLAIN.lock().expect("plain icons").get(name) {
        return *v;
    }
    // Rendered small once, and its pixels looked at.
    let v = TREES.get().and_then(|t| t.get(name)).is_some_and(|tree| {
        let (w, h) = (tree.size().width(), tree.size().height());
        let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(32, 32) else { return false };
        resvg::render(tree, resvg::tiny_skia::Transform::from_scale(32.0 / w, 32.0 / h), &mut pixmap.as_mut());
        pixmap.data().chunks_exact(4).all(|p| {
            let (r, g, b, a) = (p[0], p[1], p[2], p[3]);
            a < 16 || (r.abs_diff(g) < 24 && g.abs_diff(b) < 24 && r.abs_diff(b) < 24)
        })
    });
    PLAIN.lock().expect("plain icons").insert(name.to_string(), v);
    v
}

/// Draws `icon` from its image: white asks for the highlighted (`_h`) variant, the dim
/// grays fade it as disabled. Returns false when there is no image for the icon.
fn paint_image(p: &Painter, r: Rect, icon: Icon, c: Color32) -> bool {
    let Some(name) = image_name(icon) else {
        return false;
    };
    let disabled = c == theme::text_dim() || c == theme::chip();
    // The artwork is light glyphs for a dark page; in the light theme a plain (gray and white)
    // picture takes the color asked for instead (colored ones, as the editions', keep theirs).
    let tint = if theme::is_light() && is_plain(name) {
        if disabled { c.gamma_multiply(0.55) } else { c }
    } else if disabled {
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
    ArrowUp,
    ArrowDown,
    Copy,
    Cut,
    Paste,
    Rename,
    Save,
    Gear,
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
    /// A cube with a small gear: the world's `level.dat`.
    Level,
    Layers,
    Minimize,
    Maximize,
    Restore,
    User,
    ArrowRight,
    /// Red/gray "not supported" cross (extension editions).
    Cross,
    /// A figure with a cross: removing mobs and other entities.
    MobRemove,
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
    let gray = theme::text_muted();
    let bg = theme::bg();
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
            let frame = Pen { p, r, c: gray };
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
            let g = Pen { p, r, c: gray };
            g.line(&[(0.78, 0.58), (0.78, 0.78), (0.22, 0.78), (0.22, 0.58)]);
            g.head((0.22, 0.58), -PI / 2.0);
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
        Icon::ArrowUp => {
            pen.line(&[(0.5, 0.84), (0.5, 0.18)]);
            pen.line(&[(0.26, 0.42), (0.5, 0.18), (0.74, 0.42)]);
        }
        Icon::ArrowDown => {
            pen.line(&[(0.5, 0.16), (0.5, 0.82)]);
            pen.line(&[(0.26, 0.58), (0.5, 0.82), (0.74, 0.58)]);
        }
        Icon::Copy => {
            // The sheet behind is open where the front sheet covers it.
            pen.line(&[(0.34, 0.3), (0.34, 0.14), (0.84, 0.14), (0.84, 0.64), (0.7, 0.64)]);
            pen.rect_stroke(0.16, 0.34, 0.66, 0.86, 2);
        }
        Icon::Cut => {
            pen.circle(0.3, 0.74, 0.12);
            pen.circle(0.7, 0.74, 0.12);
            pen.line(&[(0.36, 0.64), (0.7, 0.14)]);
            pen.line(&[(0.64, 0.64), (0.3, 0.14)]);
        }
        Icon::Paste => {
            pen.rect_stroke(0.2, 0.22, 0.8, 0.88, 2);
            pen.rect_fill(0.38, 0.12, 0.62, 0.3, 2, c);
            pen.line(&[(0.34, 0.5), (0.66, 0.5)]);
            pen.line(&[(0.34, 0.68), (0.66, 0.68)]);
        }
        Icon::Rename => {
            pen.closed(&[(0.62, 0.2), (0.8, 0.38), (0.36, 0.82), (0.16, 0.84), (0.18, 0.64)]);
            pen.line(&[(0.5, 0.32), (0.68, 0.5)]);
        }
        Icon::Save => {
            pen.rect_stroke(0.16, 0.16, 0.84, 0.84, 2);
            pen.rect_fill(0.3, 0.16, 0.66, 0.38, 0, c);
            pen.rect_stroke(0.3, 0.56, 0.7, 0.84, 0);
        }
        Icon::Level => {
            // The box mark's cube, as an outline, with the settings gear at its corner.
            pen.closed(&[
                (0.45, 0.06),
                (0.78, 0.25),
                (0.78, 0.63),
                (0.45, 0.82),
                (0.12, 0.63),
                (0.12, 0.25),
            ]);
            pen.line(&[(0.12, 0.25), (0.45, 0.44), (0.78, 0.25)]);
            pen.line(&[(0.45, 0.44), (0.45, 0.82)]);
            let gear = pen.at(0.74, 0.74);
            p.circle_filled(gear, r.width() * 0.24, bg);
            paint(p, Rect::from_center_size(gear, r.size() * 0.52), Icon::Gear, Color32::WHITE);
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
        Icon::MobRemove => {
            pen.dot(0.38, 0.3, 0.14, c);
            pen.fill(&[(0.1, 0.82), (0.17, 0.56), (0.59, 0.56), (0.62, 0.66), (0.52, 0.82)], c);
            pen.line(&[(0.66, 0.6), (0.92, 0.86)]);
            pen.line(&[(0.92, 0.6), (0.66, 0.86)]);
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
            theme::text(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn render(tree: &resvg::usvg::Tree, side: u32) -> resvg::tiny_skia::Pixmap {
        let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side).unwrap();
        let s = side as f32 / tree.size().width().max(tree.size().height());
        resvg::render(tree, resvg::tiny_skia::Transform::from_scale(s, s), &mut pixmap.as_mut());
        pixmap
    }

    /// Every bundled icon parses and draws something, so a broken SVG fails here and not on screen.
    #[test]
    fn every_bundled_icon_parses_and_draws() {
        let opt = resvg::usvg::Options::default();
        assert!(bundled::SVGS.len() > 90);
        for (name, svg) in bundled::SVGS {
            let tree = resvg::usvg::Tree::from_str(svg, &opt).unwrap_or_else(|e| panic!("{name}: {e}"));
            let pixmap = render(&tree, 64);
            assert!(pixmap.pixels().iter().any(|p| p.alpha() > 0), "{name} draws nothing");
        }
    }

    /// The names the app asks for all exist.
    #[test]
    fn the_icons_the_app_uses_exist() {
        let has = |n: &str| bundled::SVGS.iter().any(|(name, _)| *name == n);
        for n in [
            "POS", "ROT", "TAG_BYTE", "TAG_COMPOUND", "TAG_LONG_ARRAY", "THUMB_TIME", "PLATFORM_JAVA_h", "FOLDER_h",
        ] {
            assert!(has(n), "{n}");
        }
    }
}
