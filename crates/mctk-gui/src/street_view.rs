//! The 3D view of the map: zoomed in, the land around the middle of the view drawn from the
//! surface the map already holds (each column's ground, the water over it and the leaves or plants
//! on it) with the resource pack's block pictures. Straight down it is the close-up of the flat map
//! (the "above view"); tilted, the street view.
//!
//! Everything is drawn as one egui mesh, columns furthest from the camera first, so no depth
//! buffer is needed; the block pictures share one texture so the mesh is a single draw. Faces are
//! clipped at a near plane (so nothing close to the eye drops out) and split into smaller pieces
//! where their depth changes a lot (egui maps pictures without perspective, which would warp them).

use std::collections::HashMap;

use eframe::egui::{self, Color32, Pos2, Rect, epaint::Vertex, pos2};

use crate::world::{self, MapImage};

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn lerp3(a: V3, b: V3, t: f32) -> V3 {
    add(a, mul(sub(b, a), t))
}
fn lerp2(a: Pos2, b: Pos2, t: f32) -> Pos2 {
    a + (b - a) * t
}

/// Vertical field of view.
pub const FOV_Y: f32 = 60.0 * std::f32::consts::PI / 180.0;
/// Nothing nearer the eye than this is drawn (faces crossing it are cut there).
const NEAR: f32 = 0.05;
/// The color of a chunk's line along the top of the land (Land Outline): cyan, apart from the
/// chunk grid's yellow, the hovered chunk's dark box, picked chunks' green and the selection's
/// white.
pub const LAND_LINE: Color32 = Color32::from_rgb(40, 220, 235);

/// How far below the target the land drawn is allowed for, looking down: land that low still
/// reaches the screen's edges (nothing of the flat map shows around it).
const BELOW: f32 = 64.0;

/// Where the camera is: it looks at `target` from `dist` blocks away, turned `yaw` from north
/// (clockwise seen from above) and tilted `pitch` down from level (90 degrees: straight down).
#[derive(Clone, Copy, PartialEq)]
pub struct Camera {
    pub target: V3,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub rect: Rect,
    /// The vertical field of view ([`FOV_Y`] as normal, narrower with the lens zoomed in).
    pub fov: f32,
}

impl Camera {
    /// Where the eye is.
    pub fn eye(&self) -> V3 {
        self.basis().0
    }

    /// Eye position, the direction it looks, and the screen's right and up in the world.
    fn basis(&self) -> (V3, V3, V3, V3) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let fwd = [sy * cp, -sp, -cy * cp];
        // Right is level and taken from the yaw alone, so looking straight down still has one.
        let right = [cy, 0.0, sy];
        let up = cross(right, fwd);
        (sub(self.target, mul(fwd, self.dist)), fwd, right, up)
    }

    pub fn focal(&self) -> f32 {
        self.rect.height() * 0.5 / (self.fov * 0.5).tan()
    }

    /// The camera distance that shows the target's level at `zoom` pixels a block when looking
    /// straight down: there the 3D view matches the flat map exactly.
    pub fn dist_for_zoom(rect: Rect, zoom: f32) -> f32 {
        rect.height() * 0.5 / (FOV_Y * 0.5).tan() / zoom
    }

    /// The camera's frame worked out once, for projecting many points.
    fn view(&self) -> View {
        let (eye, fwd, right, up) = self.basis();
        View { eye, fwd, right, up, focal: self.focal(), center: self.rect.center() }
    }

    /// Projects a point onto the screen, with its depth; `None` behind the near plane.
    pub fn project(&self, p: V3) -> Option<(Pos2, f32)> {
        self.view().project(p)
    }

    /// The camera as one matrix for the GPU (columns first, as WGSL reads them): a point of the
    /// world to clip space, depth 0 at `near` to 1 far away.
    pub(crate) fn view_proj(&self) -> [[f32; 4]; 4] {
        let (eye, fwd, right, up) = self.basis();
        let aspect = self.rect.width() / self.rect.height().max(1.0);
        let f = 1.0 / (self.fov * 0.5).tan();
        let (near, far) = (0.1_f32, 5000.0_f32);
        let (a, b) = (far / (far - near), -near * far / (far - near));
        let rows = [
            [right[0] * f / aspect, right[1] * f / aspect, right[2] * f / aspect, -dot(right, eye) * f / aspect],
            [up[0] * f, up[1] * f, up[2] * f, -dot(up, eye) * f],
            [fwd[0] * a, fwd[1] * a, fwd[2] * a, -dot(fwd, eye) * a + b],
            [fwd[0], fwd[1], fwd[2], -dot(fwd, eye)],
        ];
        let mut m = [[0.0; 4]; 4];
        for c in 0..4 {
            for r in 0..4 {
                m[c][r] = rows[r][c];
            }
        }
        m
    }

    /// The ray from the eye through a point on the screen.
    pub(crate) fn ray(&self, s: Pos2) -> (V3, V3) {
        let (eye, fwd, right, up) = self.basis();
        let c = self.rect.center();
        let f = self.focal();
        let d = add(fwd, add(mul(right, (s.x - c.x) / f), mul(up, -(s.y - c.y) / f)));
        let l = dot(d, d).sqrt();
        (eye, mul(d, 1.0 / l))
    }

    /// The level directions forward and right, for moving the target by dragging.
    /// Where the screen's four corners (top-left, top-right, bottom-right, bottom-left) look at
    /// the level `y`, on the ground (x, z): the part of it in view. A corner that looks above
    /// the level (or further than `far` blocks off) is taken `far` blocks out its way.
    pub fn footprint(&self, y: f32, far: f32) -> [[f32; 2]; 4] {
        let (eye, fwd, right, up) = self.basis();
        let f = self.focal();
        let (hw, hh) = (self.rect.width() * 0.5, self.rect.height() * 0.5);
        [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)].map(|(sx, sy)| {
            let d = [0, 1, 2].map(|k| fwd[k] * f + right[k] * sx - up[k] * sy);
            let len = dot(d, d).sqrt().max(1e-6);
            let d = d.map(|v| v / len);
            let level = (d[0] * d[0] + d[2] * d[2]).sqrt().max(1e-4);
            let mut t = far / level;
            if d[1] < -1e-4 && eye[1] > y {
                t = t.min((y - eye[1]) / d[1]);
            }
            [eye[0] + d[0] * t, eye[2] + d[2] * t]
        })
    }

    pub fn ground_axes(&self) -> ([f32; 2], [f32; 2]) {
        let (sy, cy) = self.yaw.sin_cos();
        ([sy, -cy], [cy, sy])
    }

    /// Blocks moved per pixel dragged, near the target.
    pub fn drag_scale(&self) -> f32 {
        self.dist / self.focal()
    }
}

/// A camera's frame (see [`Camera::view`]).
#[derive(Clone, Copy)]
struct View {
    eye: V3,
    fwd: V3,
    right: V3,
    up: V3,
    focal: f32,
    center: Pos2,
}

impl View {
    fn to_view(&self, p: V3) -> V3 {
        let v = sub(p, self.eye);
        [dot(v, self.right), dot(v, self.up), dot(v, self.fwd)]
    }

    fn to_screen(&self, v: V3) -> Pos2 {
        let f = self.focal / v[2];
        pos2(self.center.x + v[0] * f, self.center.y - v[1] * f)
    }

    fn project(&self, p: V3) -> Option<(Pos2, f32)> {
        let v = self.to_view(p);
        (v[2] >= NEAR).then(|| (self.to_screen(v), v[2]))
    }
}

/// Which face of a block a picture is for.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Face {
    Top,
    Side,
    /// The side of a block lower in a cliff (dirt under grass, for example).
    Under,
    /// Something more drawn with the block: a campfire's flames.
    Extra,
}

/// A block picture placed in the shared texture, and the color it is multiplied by.
#[derive(Clone, Copy)]
pub(crate) struct Slot {
    pub uv: Rect,
    pub tint: [f32; 3],
}

impl Slot {
    /// The part of the picture between fractions `u0..u1` across and `v0..v1` down.
    pub(crate) fn part(&self, u0: f32, u1: f32, v0: f32, v1: f32) -> Rect {
        let (a, s) = (self.uv.min, self.uv.size());
        Rect::from_min_max(pos2(a.x + s.x * u0, a.y + s.y * v0), pos2(a.x + s.x * u1, a.y + s.y * v1))
    }
}

pub(crate) const CELL: usize = 16;
pub(crate) const SIZE: usize = 2048;
const PER_ROW: usize = SIZE / CELL;
/// Block and item pictures fill the top half in cells; entity pictures (chests, mobs...) are
/// packed at their own size into the bottom half.
const BLOCK_CELLS: usize = PER_ROW * (SIZE / 2 / CELL);

/// An animated picture (water, lava, fire...): its cell in the texture, every frame, the order
/// they are shown in with how long each stays (in game ticks, twentieths of a second, as the
/// pack's `.mcmeta` gives), and the frame showing now.
struct Anim {
    x: usize,
    y: usize,
    frames: Vec<Vec<u8>>,
    seq: Vec<(usize, u32)>,
    total: u32,
    shown: usize,
}

/// The order and timing of an animated picture's frames, from the `.mcmeta` beside it (every
/// frame in turn, one tick each, when it says nothing).
fn frame_order(png: &std::path::Path, n: usize) -> Vec<(usize, u32)> {
    let meta = std::fs::read_to_string(png.with_extension("png.mcmeta"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let anim = meta.as_ref().map(|m| &m["animation"]);
    let tick = anim.and_then(|a| a["frametime"].as_u64()).unwrap_or(1).max(1) as u32;
    let listed: Option<Vec<(usize, u32)>> = anim.and_then(|a| a["frames"].as_array()).map(|list| {
        list.iter()
            .filter_map(|f| match f.as_u64() {
                Some(i) => Some((i as usize, tick)),
                None => Some((f["index"].as_u64()? as usize, f["time"].as_u64().map_or(tick, |t| t.max(1) as u32))),
            })
            .filter(|(i, _)| *i < n)
            .collect()
    });
    match listed {
        Some(l) if !l.is_empty() => l,
        _ => (0..n).map(|k| (k, tick)).collect(),
    }
}

/// The block pictures, packed into one texture.
pub(crate) struct Atlas {
    /// Leaves drawn solid (their gaps filled), for drawing without a depth buffer; with one (the
    /// GPU world) they keep their gaps, as the game's Fancy leaves.
    pub solid_leaves: bool,
    pixels: Vec<u8>,
    tex: Option<egui::TextureHandle>,
    /// Each block's pictures by map number and face (top, side, under, extra), once worked out.
    slots: Vec<[Option<Option<Slot>>; 4]>,
    /// Pictures already placed, by file: where, and their average color.
    by_file: HashMap<String, Option<(Rect, [f32; 3])>>,
    anims: Vec<Anim>,
    next: usize,
    dirty: bool,
    /// Where the next entity picture goes in the bottom half, and the current row's height.
    ent_x: usize,
    ent_y: usize,
    ent_row: usize,
    /// Parts put in since the GPU last took them, and whether it has yet to take the whole.
    regions: Vec<[usize; 4]>,
    gpu_full: bool,
}

impl Atlas {
    pub(crate) fn new() -> Self {
        // Slot 0 is plain white, for faces drawn in a color alone.
        let mut pixels = vec![0u8; SIZE * SIZE * 4];
        for y in 0..CELL {
            for x in 0..CELL {
                let o = (y * SIZE + x) * 4;
                pixels[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        Atlas { solid_leaves: true, pixels, tex: None, slots: Vec::new(), by_file: HashMap::new(), anims: Vec::new(), next: 1, dirty: true, ent_x: 0, ent_y: SIZE / 2, ent_row: 0, regions: Vec::new(), gpu_full: true }
    }

    pub(crate) fn white() -> Rect {
        let c = CELL as f32 * 0.5 / SIZE as f32;
        Rect::from_center_size(pos2(c, c), egui::vec2(0.5 / SIZE as f32, 0.5 / SIZE as f32))
    }

    /// Puts a picture file of the pack in the texture (once), returning where it went and the
    /// average color of its opaque pixels. An animated picture (a tall strip of square frames)
    /// keeps all its frames, shown in turn by [`Atlas::animate`].
    fn place(&mut self, file: &str, axis_x: bool) -> Option<(Rect, [f32; 3])> {
        let key = format!("{file}{}", if axis_x { "#x" } else { "" });
        if let Some(found) = self.by_file.get(&key) {
            return *found;
        }
        let found = self.load(file, axis_x);
        self.by_file.insert(key, found);
        found
    }

    /// An entity picture (any size) packed into the bottom half at its own size, in rows.
    fn load_entity(&mut self, img: &image::RgbaImage) -> Option<(Rect, [f32; 3])> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        // Kept on a 16-pixel grid, so the smaller copies (mip levels) mix no two pictures much.
        let (gw, gh) = (w.div_ceil(CELL) * CELL, h.div_ceil(CELL) * CELL);
        if gw > SIZE || gh > SIZE / 2 {
            return None;
        }
        if self.ent_x + gw > SIZE {
            self.ent_x = 0;
            self.ent_y += self.ent_row;
            self.ent_row = 0;
        }
        if self.ent_y + gh > SIZE {
            return None;
        }
        let (sx, sy) = (self.ent_x, self.ent_y);
        self.ent_x += gw;
        self.ent_row = self.ent_row.max(gh);
        for (x, y, p) in img.enumerate_pixels() {
            let o = ((sy + y as usize) * SIZE + sx + x as usize) * 4;
            self.pixels[o..o + 4].copy_from_slice(&p.0);
        }
        self.mark(sx, sy, gw, gh);
        let inset = 0.02 / SIZE as f32;
        Some((
            Rect::from_min_max(
                pos2(sx as f32 / SIZE as f32 + inset, sy as f32 / SIZE as f32 + inset),
                pos2((sx + w) as f32 / SIZE as f32 - inset, (sy + h) as f32 / SIZE as f32 - inset),
            ),
            [128.0; 3],
        ))
    }

    fn load(&mut self, file: &str, axis_x: bool) -> Option<(Rect, [f32; 3])> {
        let path = crate::icons::pack_texture(file)?;
        let img = image::open(&path).ok()?.to_rgba8();
        // Pictures not made for a block's 16 x 16 (entities, paintings) keep their own size.
        if !(file.starts_with("block/") || file.starts_with("item/")) {
            return self.load_entity(&img);
        }
        let w = img.width();
        let n = if img.height() > w && img.height() % w == 0 { (img.height() / w) as usize } else { 1 };
        // A flowing liquid's picture is twice a block across; the game shows the middle half of
        // it over a block (shrunk whole, every other pixel dropped, it looked like sand).
        let flow = file.ends_with("_flow.png");
        // A still picture bigger than a block's 16 x 16 (signs are 32 x 32) keeps its own size,
        // packed as the entity pictures are: shrunk into a cell it lost half its detail.
        if n == 1 && !flow && !axis_x && (img.width() as usize > CELL || img.height() as usize > CELL) {
            let (mut sum, mut opaque) = ([0f32; 3], 0f32);
            for p in img.pixels().filter(|p| p[3] > 128) {
                for k in 0..3 {
                    sum[k] += p[k] as f32;
                }
                opaque += 1.0;
            }
            if opaque == 0.0 {
                return None;
            }
            let (r, _) = self.load_entity(&img)?;
            return Some((r, sum.map(|s| s / opaque)));
        }
        let frame = |k: usize| {
            let mut pic = if flow {
                image::imageops::crop_imm(&img, w / 4, k as u32 * w + w / 4, w / 2, w / 2).to_image()
            } else {
                image::imageops::crop_imm(&img, 0, k as u32 * w, w, w.min(img.height())).to_image()
            };
            if axis_x {
                pic = image::imageops::rotate90(&pic);
            }
            if pic.width() as usize != CELL || pic.height() as usize != CELL {
                pic = image::imageops::resize(&pic, CELL as u32, CELL as u32, image::imageops::FilterType::Nearest);
            }
            pic
        };
        let pic = frame(0);
        let (mut sum, mut opaque) = ([0f32; 3], 0f32);
        for p in pic.pixels().filter(|p| p[3] > 128) {
            for k in 0..3 {
                sum[k] += p[k] as f32;
            }
            opaque += 1.0;
        }
        if opaque == 0.0 || self.next >= BLOCK_CELLS {
            return None;
        }
        let avg = sum.map(|s| s / opaque);
        // Leaves are drawn solid, as the game's Fast leaves are: their gaps filled in a darker
        // shade of the leaf, so the land does not show through the trees.
        let solid = self.solid_leaves && file.contains("leaves");
        let fill = |mut pic: image::RgbaImage| {
            if solid {
                for p in pic.pixels_mut().filter(|p| p[3] < 128) {
                    *p = image::Rgba([(avg[0] * 0.55) as u8, (avg[1] * 0.55) as u8, (avg[2] * 0.55) as u8, 255]);
                }
            }
            pic
        };
        let pic = fill(pic);
        let (sx, sy) = (self.next % PER_ROW * CELL, self.next / PER_ROW * CELL);
        self.next += 1;
        for (x, y, p) in pic.enumerate_pixels() {
            let o = ((sy + y as usize) * SIZE + sx + x as usize) * 4;
            self.pixels[o..o + 4].copy_from_slice(&p.0);
        }
        self.mark(sx, sy, CELL, CELL);
        if n > 1 {
            let seq = frame_order(&path, n);
            let total = seq.iter().map(|(_, t)| *t).sum::<u32>().max(1);
            let frames = (0..n).map(|k| fill(frame(k)).into_raw()).collect();
            self.anims.push(Anim { x: sx, y: sy, frames, seq, total, shown: usize::MAX });
        }
        // A sliver in from the edges, so neighboring pictures never bleed in.
        let inset = 0.02 / SIZE as f32;
        let r = Rect::from_min_max(
            pos2(sx as f32 / SIZE as f32 + inset, sy as f32 / SIZE as f32 + inset),
            pos2((sx + CELL) as f32 / SIZE as f32 - inset, (sy + CELL) as f32 / SIZE as f32 - inset),
        );
        Some((r, avg))
    }

    /// A grass block's side as the game draws it: the pack's side picture with its gray fringe
    /// (`grass_block_side_overlay`) laid over it in the biome's grass color `tint`, so the side
    /// matches the top in every biome. Once per color.
    fn grass_side(&mut self, tint: [f32; 3]) -> Option<Rect> {
        let key = format!("grass_side@{:.0},{:.0},{:.0}", tint[0], tint[1], tint[2]);
        if let Some(found) = self.by_file.get(&key) {
            return found.map(|(r, _)| r);
        }
        let found = (|| {
            let open = |f: &str| -> Option<image::RgbaImage> {
                let pic = image::open(crate::icons::pack_texture(f)?).ok()?.to_rgba8();
                let w = pic.width();
                let pic = image::imageops::crop_imm(&pic, 0, 0, w, w.min(pic.height())).to_image();
                Some(image::imageops::resize(&pic, CELL as u32, CELL as u32, image::imageops::FilterType::Nearest))
            };
            // The side without its fringe (dirt), or the pack's own side where it has no plain one.
            let mut pic = open("block/dirt.png").or_else(|| open("block/grass_block_side.png"))?;
            let overlay = open("block/grass_block_side_overlay.png")?;
            for (p, o) in pic.pixels_mut().zip(overlay.pixels()) {
                let a = o[3] as f32 / 255.0;
                for k in 0..3 {
                    let g = o[k] as f32 * tint[k] / 255.0;
                    p[k] = (g * a + p[k] as f32 * (1.0 - a)).clamp(0.0, 255.0) as u8;
                }
            }
            if self.next >= BLOCK_CELLS {
                return None;
            }
            let (sx, sy) = (self.next % PER_ROW * CELL, self.next / PER_ROW * CELL);
            self.next += 1;
            for (x, y, p) in pic.enumerate_pixels() {
                let o = ((sy + y as usize) * SIZE + sx + x as usize) * 4;
                self.pixels[o..o + 4].copy_from_slice(&p.0);
            }
            self.mark(sx, sy, CELL, CELL);
            let inset = 0.02 / SIZE as f32;
            let r = Rect::from_min_max(
                pos2(sx as f32 / SIZE as f32 + inset, sy as f32 / SIZE as f32 + inset),
                pos2((sx + CELL) as f32 / SIZE as f32 - inset, (sy + CELL) as f32 / SIZE as f32 - inset),
            );
            Some((r, [128.0; 3]))
        })();
        self.by_file.insert(key, found);
        found.map(|(r, _)| r)
    }

    /// Notes a part of the picture changed (for both ways of drawing).
    fn mark(&mut self, x: usize, y: usize, w: usize, h: usize) {
        self.dirty = true;
        self.regions.push([x, y, w, h]);
        // (Drawn without the GPU nothing takes them: past a point, the whole goes next time.)
        if self.regions.len() > 4096 {
            self.regions.clear();
            self.gpu_full = true;
        }
    }

    /// The whole picture to go to the GPU again next time (it held another world's).
    pub(crate) fn resend_all(&mut self) {
        self.gpu_full = true;
    }

    /// For drawing elsewhere (the GPU world): the parts of the picture put in since last asked,
    /// as (x, y, width, height, RGBA) — the whole of it the first time. (Sending only what
    /// changed: the whole 2048 x 2048 picture each time a block's picture came in made loading
    /// chunks crawl.)
    pub(crate) fn take_uploads(&mut self) -> Vec<(usize, usize, usize, usize, Vec<u8>)> {
        if self.gpu_full {
            self.gpu_full = false;
            self.regions.clear();
            for a in &mut self.anims {
                a.shown = usize::MAX;
            }
            return vec![(0, 0, SIZE, SIZE, self.pixels.clone())];
        }
        let regions = std::mem::take(&mut self.regions);
        regions
            .into_iter()
            .map(|[x, y, w, h]| {
                let mut px = Vec::with_capacity(w * h * 4);
                for row in y..y + h {
                    px.extend_from_slice(&self.pixels[(row * SIZE + x) * 4..(row * SIZE + x + w) * 4]);
                }
                (x, y, w, h, px)
            })
            .collect()
    }

    /// For drawing elsewhere: the animated cells whose frame changes at `time` (seconds), as
    /// (x, y, 16 x 16 RGBA), and how long until the next change, if anything is animated.
    pub(crate) fn due_frames(&mut self, time: f64) -> (Vec<(usize, usize, Vec<u8>)>, Option<f64>) {
        let mut out = Vec::new();
        let tick = time * 20.0;
        let mut next = f64::MAX;
        for a in &mut self.anims {
            let t = tick % a.total as f64;
            let (mut at, mut pick, mut until) = (0.0, 0, 1.0);
            for (k, (_, d)) in a.seq.iter().enumerate() {
                if t < at + *d as f64 {
                    pick = k;
                    until = at + *d as f64 - t;
                    break;
                }
                at += *d as f64;
            }
            next = next.min(until / 20.0);
            if pick != a.shown {
                a.shown = pick;
                out.push((a.x, a.y, a.frames[a.seq[pick].0].clone()));
            }
        }
        (out, (next < f64::MAX).then_some(next))
    }

    /// Shows each animated picture's frame for `time` (seconds), changing only the cells whose
    /// frame changed. Gives how long until the next change, if anything is animated.
    fn animate(&mut self, time: f64) -> Option<f64> {
        let tex = self.tex.as_mut()?;
        let tick = time * 20.0;
        let mut next = f64::MAX;
        for a in &mut self.anims {
            let t = tick % a.total as f64;
            let (mut at, mut pick, mut until) = (0.0, 0, 1.0);
            for (k, (_, d)) in a.seq.iter().enumerate() {
                if t < at + *d as f64 {
                    pick = k;
                    until = at + *d as f64 - t;
                    break;
                }
                at += *d as f64;
            }
            next = next.min(until / 20.0);
            if pick != a.shown {
                a.shown = pick;
                let img = egui::ColorImage::from_rgba_unmultiplied([CELL, CELL], &a.frames[a.seq[pick].0]);
                tex.set_partial([a.x, a.y], img, egui::TextureOptions::NEAREST);
            }
        }
        (next < f64::MAX).then_some(next)
    }

    /// A picture file of the pack as it is, untinted (for a block drawn differently from what
    /// its name alone says, as a stack of dripstone).
    pub(crate) fn file_slot(&mut self, file: &str) -> Option<Slot> {
        self.place(file, false).map(|(uv, _)| Slot { uv, tint: [1.0; 3] })
    }

    /// The picture for one face of a block, tinted toward the color the map gives the block.
    pub(crate) fn slot(&mut self, id: u16, face: Face) -> Option<Slot> {
        let f = match face {
            Face::Top => 0,
            Face::Side => 1,
            Face::Under => 2,
            Face::Extra => 3,
        };
        if let Some(Some(s)) = self.slots.get(id as usize).map(|s| s[f]) {
            return s;
        }
        let found = (|| {
            let name = world::block_name(id)?;
            let plain = name.split('@').next().unwrap_or(&name).to_string();
            let (base, axis) = plain.split_once('#').map_or((plain.as_str(), None), |(b, a)| (b, Some(a)));
            let leafy = base.ends_with("_leaves") || base == "leaves" || base == "vine";
            let grassy = matches!(base, "short_grass" | "tall_grass" | "fern" | "large_fern" | "sugar_cane" | "lily_pad");
            // Under the top of a cliff: dirt below grass and the like; the block itself otherwise.
            let under = match base {
                "grass_block" | "podzol" | "mycelium" | "dirt_path" | "farmland" | "snow" | "snow_block" | "moss_block" => Some("dirt"),
                _ => None,
            };
            // A grass block's side, its fringe in the biome's grass color like its top.
            if face == Face::Side && base == "grass_block" {
                if let Some(uv) = self.grass_side(world::grass_color(&name)) {
                    return Some(Slot { uv, tint: [1.0; 3] });
                }
            }
            let files: Vec<String> = match (face, base) {
                // Only a campfire has more to draw: its flames.
                (Face::Extra, "campfire" | "soul_campfire") => vec![format!("block/{base}_fire.png")],
                (Face::Extra, _) => return None,
                // Water and lava as the game shows them: still, or flowing.
                (_, "water" | "flowing_water") if flowing(&plain) => vec!["block/water_flow.png".into(), "block/water_still.png".into()],
                (_, "water" | "flowing_water") => vec!["block/water_still.png".into()],
                (_, "lava" | "flowing_lava") if flowing(&plain) => vec!["block/lava_flow.png".into(), "block/lava_still.png".into()],
                (_, "lava" | "flowing_lava") => vec!["block/lava_still.png".into()],
                // A campfire is its logs (lit on top, plain on the sides); the flames are drawn
                // over it.
                (Face::Top, "campfire" | "soul_campfire") => vec!["block/campfire_log_lit.png".into(), "block/campfire_log.png".into()],
                (_, "campfire" | "soul_campfire") => vec!["block/campfire_log.png".into()],
                (Face::Top, _) => world::texture_files(base, axis.is_some()),
                (Face::Under, _) if under.is_some() => vec![format!("block/{}.png", under.unwrap_or("dirt"))],
                (_, _) => {
                    let mut f = vec![format!("block/{base}_side.png"), format!("block/{base}.png")];
                    f.extend(world::texture_files(base, false));
                    f
                }
            };
            let (uv, avg) = files.iter().find_map(|f| self.place(f, face == Face::Top && axis == Some("x")))?;
            // Grass tops, leaves and plants are gray in the pack and colored by the biome; the
            // map's color for the block carries that, so the picture is tinted toward it.
            let tinted = (face == Face::Top && !matches!(base, "campfire" | "soul_campfire" | "lava" | "flowing_lava")) || leafy || grassy;
            let tint = if matches!(base, "water" | "flowing_water") {
                // The pack's water is gray; the game multiplies it by water's blue (#3F76E4),
                // as here (a tint over 1, which the GPU's colors cannot hold, made it gray).
                let _ = avg;
                [63.0 / 255.0, 118.0 / 255.0, 228.0 / 255.0]
            } else if tinted && face != Face::Under {
                let flat = world::surface_color(&name).map(|c| c as f32);
                [0, 1, 2].map(|k| (flat[k] / avg[k].max(1.0)).min(1.6))
            } else {
                [1.0; 3]
            };
            Some(Slot { uv, tint })
        })();
        if self.slots.len() <= id as usize {
            self.slots.resize(id as usize + 1, [None; 4]);
        }
        self.slots[id as usize][f] = Some(found);
        found
    }

    fn texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        if self.dirty || self.tex.is_none() {
            let img = egui::ColorImage::from_rgba_unmultiplied([SIZE, SIZE], &self.pixels);
            match &mut self.tex {
                Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                None => self.tex = Some(ctx.load_texture("street-view-blocks", img, egui::TextureOptions::NEAREST)),
            }
            self.dirty = false;
            // The whole texture went up with first frames: each animation shows its own again.
            for a in &mut self.anims {
                a.shown = usize::MAX;
            }
        }
        self.tex.as_ref().map_or(egui::TextureId::default(), |t| t.id())
    }
}

/// The shape a block is drawn as.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Form {
    /// A full-width block of this height (1 for a whole block, 0.5 for a slab...).
    Cube(f32),
    /// A narrow upright piece standing in the middle (fences, torches, lanterns...): half its
    /// width and its height.
    Post(f32, f32),
    /// Something lying flat on the ground (rails, redstone).
    Decal,
    /// Two crossed pictures (plants, cobwebs).
    Cross,
}

/// How high a water or lava block's surface is (1: the whole block): a source is a little under
/// full, flowing liquid lower the further it has run, falling liquid full. `None` for anything
/// else. Names carry the level after '#' ("water#3"); "flowing_water" (older Bedrock) and a level
/// not known run halfway.
pub fn liquid_height(name: &str) -> Option<f32> {
    let n = name.trim_start_matches("minecraft:");
    let (base, level) = n.split_once('#').map_or((n, None), |(b, l)| (b, Some(l)));
    let level: Option<u32> = match base {
        "water" | "lava" => match level {
            None => Some(0),
            Some(l) => Some(l.parse().unwrap_or(4)),
        },
        "flowing_water" | "flowing_lava" => Some(4),
        _ => None,
    };
    level.map(|k| match k {
        0 => 0.875,
        1..=7 => (0.875 * (8 - k) as f32 / 8.0).max(0.125),
        _ => 1.0,
    })
}

/// Whether a liquid is flowing (drawn with its flowing picture) rather than a still source.
pub(crate) fn flowing(name: &str) -> bool {
    let n = name.trim_start_matches("minecraft:");
    n.starts_with("flowing_") || ((n.starts_with("water#") || n.starts_with("lava#")) && !n.ends_with("#0"))
}

pub(crate) fn form_of(name: &str) -> Form {
    if name.trim_start_matches("minecraft:").starts_with("lava") || name.contains("flowing_lava") {
        if let Some(h) = liquid_height(name) {
            return Form::Cube(h);
        }
    }
    let n = name.trim_start_matches("minecraft:").split(['#', '@']).next().unwrap_or("");
    let px = |p: f32| p / 16.0;
    match n {
        "campfire" | "soul_campfire" => return Form::Cube(px(7.0)),
        "snow" => return Form::Cube(px(2.0)),
        "chest" | "trapped_chest" | "ender_chest" => return Form::Cube(px(14.0)),
        "enchanting_table" => return Form::Cube(px(12.0)),
        "stonecutter" => return Form::Cube(px(9.0)),
        "end_portal_frame" => return Form::Cube(px(13.0)),
        "dirt_path" | "farmland" => return Form::Cube(px(15.0)),
        "cake" => return Form::Cube(px(8.0)),
        "daylight_detector" => return Form::Cube(px(6.0)),
        "repeater" | "comparator" => return Form::Cube(px(2.0)),
        // Pictures of a thing rather than block faces: drawn as two crossed pictures.
        "cobweb" | "pointed_dripstone" | "amethyst_cluster" | "small_amethyst_bud" | "medium_amethyst_bud" | "large_amethyst_bud"
        | "mangrove_propagule" | "sea_pickle" | "turtle_egg" | "brewing_stand" | "frogspawn" | "big_dripleaf_stem" => return Form::Cross,
        "bamboo" | "bell" => return Form::Post(px(2.0), 1.0),
        "redstone_wire" | "tripwire" | "rail" => return Form::Decal,
        "lantern" | "soul_lantern" => return Form::Post(px(3.0), px(9.0)),
        "flower_pot" => return Form::Post(px(3.0), px(6.0)),
        "lever" => return Form::Post(px(2.0), px(8.0)),
        "iron_bars" | "chain" | "end_rod" | "lightning_rod" => return Form::Post(px(1.5), 1.0),
        _ => {}
    }
    if n.ends_with("_carpet") || n.ends_with("_pressure_plate") {
        Form::Cube(px(1.0))
    } else if n.ends_with("_slab") {
        Form::Cube(0.5)
    } else if n.ends_with("_bed") {
        Form::Cube(px(9.0))
    } else if n.ends_with("_trapdoor") {
        Form::Cube(px(3.0))
    } else if n.ends_with("_rail") {
        Form::Decal
    } else if n.ends_with("_wall") {
        Form::Post(px(4.0), 1.0)
    } else if n.ends_with("_fence") || n.ends_with("_pane") || n.ends_with("_fence_gate") {
        Form::Post(px(2.0), 1.0)
    } else if n.contains("torch") {
        Form::Post(px(1.0), px(10.0))
    } else if n.contains("candle") {
        Form::Post(px(1.0), px(6.0))
    } else if n.ends_with("_sign") || n.ends_with("_banner") || n.ends_with("_button") || n.ends_with("_head") || n.ends_with("_skull") {
        Form::Post(px(3.0), px(8.0))
    } else {
        Form::Cube(1.0)
    }
}

/// Builds one mesh, adding faces in the order given.
struct Builder {
    view: View,
    mesh: egui::Mesh,
    clip: Rect,
}

impl Builder {
    /// A four-cornered face (corners in order around it) with a picture, a color and opacity.
    /// The picture's corners go top-left, top-right, bottom-right, bottom-left with the face's.
    fn quad(&mut self, p: [V3; 4], uv: Rect, rgb: [f32; 3], alpha: f32) {
        let v = p.map(|q| self.view.to_view(q));
        let zmax = v.iter().fold(f32::MIN, |m, q| m.max(q[2]));
        if zmax <= NEAR {
            return;
        }
        let zmin = v.iter().fold(f32::MAX, |m, q| m.min(q[2]));
        if zmin > NEAR {
            let s = v.map(|q| self.view.to_screen(q));
            if !Rect::from_points(&s).intersects(self.clip) {
                return;
            }
            // Thinner than a pixel on screen (a block's side seen from nearly above): nothing to
            // see, so not drawn.
            let area = 0.5 * ((s[0].x * s[1].y - s[1].x * s[0].y) + (s[1].x * s[2].y - s[2].x * s[1].y) + (s[2].x * s[3].y - s[3].x * s[2].y) + (s[3].x * s[0].y - s[0].x * s[3].y));
            if area.abs() < 1.0 {
                return;
            }
        }
        let c = Color32::from_rgba_unmultiplied(
            (rgb[0].clamp(0.0, 1.0) * 255.0) as u8,
            (rgb[1].clamp(0.0, 1.0) * 255.0) as u8,
            (rgb[2].clamp(0.0, 1.0) * 255.0) as u8,
            (alpha.clamp(0.0, 1.0) * 255.0) as u8,
        );
        let uvs = [uv.left_top(), uv.right_top(), uv.right_bottom(), uv.left_bottom()];
        self.split(&[(v[0], uvs[0]), (v[1], uvs[1]), (v[2], uvs[2]), (v[3], uvs[3])], c, 0);
    }

    /// Adds a face, split into quarters (again and again near the eye) until the depth across
    /// each piece changes little, so the picture on it does not warp: egui maps pictures without
    /// perspective, which only looks right where the depth is nearly even.
    fn split(&mut self, q: &[(V3, Pos2); 4], c: Color32, depth: u32) {
        let zmin = q.iter().fold(f32::MAX, |m, p| m.min(p.0[2]));
        let zmax = q.iter().fold(f32::MIN, |m, p| m.max(p.0[2]));
        if zmax <= NEAR {
            return;
        }
        // Even enough in depth, or small enough on screen that any warp cannot be seen; a piece
        // reaching behind the eye is split a few times and then cut at the near plane.
        let even = zmin > NEAR && zmax / zmin < 1.12;
        let small = zmin > NEAR && {
            let s = q.map(|p| self.view.to_screen(p.0));
            let r = Rect::from_points(&s);
            r.width().max(r.height()) < 24.0 || !r.intersects(self.clip)
        };
        if even || small || depth >= if zmin > NEAR { 6 } else { 3 } {
            self.poly(q, c);
            return;
        }
        let mid = |a: (V3, Pos2), b: (V3, Pos2)| (lerp3(a.0, b.0, 0.5), lerp2(a.1, b.1, 0.5));
        let (t, r, b, l) = (mid(q[0], q[1]), mid(q[1], q[2]), mid(q[3], q[2]), mid(q[0], q[3]));
        let m = mid(t, b);
        for piece in [[q[0], t, m, l], [t, q[1], r, m], [m, r, q[2], b], [l, m, b, q[3]]] {
            self.split(&piece, c, depth + 1);
        }
    }

    /// A flat polygon in the camera's frame, cut at the near plane and added as a fan.
    fn poly(&mut self, pts: &[(V3, Pos2)], c: Color32) {
        let mut out: [(V3, Pos2); 8] = [([0.0; 3], Pos2::ZERO); 8];
        let mut n = 0;
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            let (ina, inb) = (a.0[2] >= NEAR, b.0[2] >= NEAR);
            if ina && n < 8 {
                out[n] = a;
                n += 1;
            }
            if ina != inb && n < 8 {
                let t = (NEAR - a.0[2]) / (b.0[2] - a.0[2]);
                out[n] = (lerp3(a.0, b.0, t), lerp2(a.1, b.1, t));
                n += 1;
            }
        }
        if n < 3 {
            return;
        }
        let i0 = self.mesh.vertices.len() as u32;
        for (v, uv) in &out[..n] {
            self.mesh.vertices.push(Vertex { pos: self.view.to_screen(*v), uv: *uv, color: c });
        }
        for k in 1..n as u32 - 1 {
            self.mesh.indices.extend_from_slice(&[i0, i0 + k, i0 + k + 1]);
        }
    }

    /// The sides of a box from `lo` to `hi` that face the eye, and its top.
    fn boxed(&mut self, eye: V3, lo: V3, hi: V3, top: Option<Slot>, side: Option<Slot>, fallback: [f32; 3], alpha: f32) {
        let white = Atlas::white();
        let paint = |s: Option<Slot>, shade: f32, part: (f32, f32, f32, f32)| match s {
            Some(s) => (s.part(part.0, part.1, part.2, part.3), s.tint.map(|k| k * shade)),
            None => (white, fallback.map(|k| k * shade)),
        };
        let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
        let (u0, u1) = (0.5 - w * 0.5, 0.5 + w * 0.5);
        let side_part = (u0, u1, 1.0 - h.min(1.0), 1.0);
        let faces: [(bool, [V3; 4], f32); 4] = [
            (eye[2] < lo[2], [[hi[0], hi[1], lo[2]], [lo[0], hi[1], lo[2]], [lo[0], lo[1], lo[2]], [hi[0], lo[1], lo[2]]], SHADE_NS[0]),
            (eye[2] > hi[2], [[lo[0], hi[1], hi[2]], [hi[0], hi[1], hi[2]], [hi[0], lo[1], hi[2]], [lo[0], lo[1], hi[2]]], SHADE_NS[1]),
            (eye[0] < lo[0], [[lo[0], hi[1], lo[2]], [lo[0], hi[1], hi[2]], [lo[0], lo[1], hi[2]], [lo[0], lo[1], lo[2]]], SHADE_EW[0]),
            (eye[0] > hi[0], [[hi[0], hi[1], hi[2]], [hi[0], hi[1], lo[2]], [hi[0], lo[1], lo[2]], [hi[0], lo[1], hi[2]]], SHADE_EW[1]),
        ];
        for (facing, p, shade) in faces {
            if facing {
                let (uv, rgb) = paint(side, shade, side_part);
                self.quad(p, uv, rgb, alpha);
            }
        }
        if eye[1] > hi[1] {
            let (uv, rgb) = paint(top, SHADE_TOP, (u0, u1, u0, u1));
            self.quad([[lo[0], hi[1], lo[2]], [hi[0], hi[1], lo[2]], [hi[0], hi[1], hi[2]], [lo[0], hi[1], hi[2]]], uv, rgb, alpha);
        }
    }
}

/// Light on each face: full on top, less on the sides, the least on those facing away from the
/// north-west sun (as the flat map is lit).
const SHADE_TOP: f32 = 1.0;
const SHADE_NS: [f32; 2] = [0.86, 0.66];
const SHADE_EW: [f32; 2] = [0.76, 0.6];

/// A block found under the pointer: where it is, and the box outlined around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// The box's bottom and top heights, and its sides as fractions of the block: west, east,
    /// north, south (a plant's box is narrower than the block; a button's sits against one side).
    pub y0: f32,
    pub y1: f32,
    pub xz: [f32; 4],
}

impl Hit {
    /// A box `inset` in from all four sides of the block.
    pub fn inset(x: i32, y: i32, z: i32, y0: f32, y1: f32, inset: f32) -> Hit {
        Hit { x, y, z, y0, y1, xz: [inset, 1.0 - inset, inset, 1.0 - inset] }
    }

    /// Whether the box is narrower than the block on any side.
    pub fn narrow(&self) -> bool {
        self.xz[0] > 0.0 || self.xz[1] < 1.0 || self.xz[2] > 0.0 || self.xz[3] < 1.0
    }
}

/// The street view's pictures and the last mesh built (kept while the camera stays still).
pub struct Street {
    atlas: Atlas,
    /// Each block's shape and whether it is leaves, by map number (worked out once).
    forms: Vec<Option<Form>>,
    leafy: Vec<Option<bool>>,
    /// Whether each block can be walked through, once worked out (see `passable`).
    passable: Vec<Option<bool>>,
    liquids: Vec<Option<f32>>,
    last: Option<(Camera, std::sync::Arc<egui::Mesh>)>,
}

impl Default for Street {
    fn default() -> Self {
        Street { atlas: Atlas::new(), forms: Vec::new(), leafy: Vec::new(), passable: Vec::new(), liquids: Vec::new(), last: None }
    }
}

/// A column's index in the picture, if the picture has it and something was drawn there.
fn index(img: &MapImage, bx: i32, bz: i32) -> Option<usize> {
    let (ox, oz) = (img.min_x * 16, img.min_z * 16);
    if bx < ox || bz < oz || bx >= ox + img.width as i32 || bz >= oz + img.height as i32 {
        return None;
    }
    let i = (bz - oz) as usize * img.width + (bx - ox) as usize;
    (img.blocks[i] != 0).then_some(i)
}

impl Street {
    /// How far around the target the land is drawn. Looking down, far enough to fill the screen;
    /// tilted, a distance the fog closes.
    pub fn radius(cam: &Camera) -> f32 {
        // At least far enough to see well around (a camera close in, as the free camera can be,
        // still sees into the distance).
        let near = (cam.dist * 2.4).clamp(80.0, 140.0);
        if cam.pitch > 60f32.to_radians() {
            // What the screen shows at the target's level, widened for land well below it
            // (valleys, the sea floor), which shows further out toward the screen's edges.
            let half = cam.rect.size().length() * 0.5 * (cam.dist + BELOW) / cam.focal() + 6.0;
            near.max(half).min(300.0)
        } else {
            near
        }
    }

    fn form(&mut self, id: u16) -> Form {
        let k = id as usize;
        if k >= self.forms.len() {
            self.forms.resize(k + 1, None);
        }
        *self.forms[k].get_or_insert_with(|| world::block_name(id).map_or(Form::Cube(1.0), |n| form_of(&n)))
    }

    /// How high the water in a column comes within its top block (see [`liquid_height`]).
    fn liquid(&mut self, id: u16) -> f32 {
        let k = id as usize;
        if k >= self.liquids.len() {
            self.liquids.resize(k + 1, None);
        }
        *self.liquids[k].get_or_insert_with(|| world::block_name(id).and_then(|n| liquid_height(&n)).unwrap_or(0.875))
    }

    /// Whether a block can be walked through (no collision in the game: grass, flowers, crops,
    /// torches, buttons, signs, rails, redstone, fire...), so it never counts toward the land's
    /// height for the camera.
    fn passable(&mut self, id: u16) -> bool {
        if id == 0 {
            return true;
        }
        let k = id as usize;
        if k >= self.passable.len() {
            self.passable.resize(k + 1, None);
        }
        *self.passable[k].get_or_insert_with(|| {
            let n = world::block_name(id).unwrap_or_default();
            let n = n.split(['#', '@']).next().unwrap_or("");
            // (Potted plants and solid blocks named like plants have collision.)
            let solid_lookalike = n.starts_with("potted_")
                || n == "flower_pot"
                || n.ends_with("_block")
                || n.contains("mangrove_roots")
                || n == "mushroom_stem"
                || n == "dirt_path";
            let plantlike = world::is_plant(n)
                || matches!(n, "snow" | "light" | "structure_void" | "lever" | "tripwire" | "redstone_wire" | "cobweb" | "fire" | "soul_fire" | "sugar_cane")
                || [
                    "torch", "button", "sign", "banner", "pressure_plate", "tripwire_hook", "rail", "vine", "sapling", "kelp", "seagrass", "grass",
                    "fern", "flower", "bush", "glow_lichen", "sculk_vein", "_roots", "sprouts", "wheat", "carrots", "potatoes", "beetroots",
                    "nether_wart", "_crop", "coral", "spore_blossom", "petals", "leaf_litter", "melon_stem", "pumpkin_stem", "dead_bush", "fungus", "mushroom",
                ]
                .iter()
                .any(|p| n.contains(p));
            plantlike && !solid_lookalike
        })
    }

    fn is_leaves(&mut self, id: u16) -> bool {
        if id == 0 {
            return false;
        }
        let k = id as usize;
        if k >= self.leafy.len() {
            self.leafy.resize(k + 1, None);
        }
        *self.leafy[k].get_or_insert_with(|| {
            let n = world::block_name(id).unwrap_or_default();
            let n = n.split(['#', '@']).next().unwrap_or("");
            n.ends_with("_leaves") || n == "leaves"
        })
    }

    /// The top of a column's solid ground (under any water). A post or the like stands on the
    /// ground beneath it: the map knows only the top block, so that is taken from the solid
    /// blocks beside it (a torch on a fence reaches down to the floor the fence stands on).
    fn ground(&mut self, img: &MapImage, i: usize) -> f32 {
        let gy = img.ground[i] as f32;
        if img.water[i] > 0 {
            return gy + 1.0;
        }
        // Something walked through (grass, a flower, a torch): the ground it stands on.
        if self.passable(img.blocks[i]) {
            return gy;
        }
        if let Form::Cube(h) = self.form(img.blocks[i]) {
            return gy + h;
        }
        let (bx, bz) = ((i % img.width) as i32 + img.min_x * 16, (i / img.width) as i32 + img.min_z * 16);
        let mut floor: Option<f32> = None;
        for (dx, dz) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
            let Some(j) = index(img, bx + dx, bz + dz) else { continue };
            if img.water[j] > 0 {
                continue;
            }
            if let Form::Cube(h) = self.form(img.blocks[j]) {
                let g = img.ground[j] as f32 + h;
                if g <= gy + 0.01 {
                    floor = Some(floor.map_or(g, |f: f32| f.min(g)));
                }
            }
        }
        floor.unwrap_or(gy)
    }

    /// What a column holds, as heights: its solid top (water's surface included, a post's top
    /// included) and the canopy of leaves over it, if any (bottom, top).
    fn solid(&mut self, img: &MapImage, i: usize) -> (f32, Option<(f32, f32)>) {
        let gy = img.ground[i] as f32;
        let top = if img.water[i] > 0 {
            img.heights[i] as f32 + self.liquid(img.blocks[i])
        } else if self.passable(img.blocks[i]) {
            gy
        } else {
            match self.form(img.blocks[i]) {
                Form::Cube(h) => gy + h,
                Form::Post(_, h) => gy + h,
                Form::Decal => gy + 0.06,
                Form::Cross => gy,
            }
        };
        let canopy = self.is_leaves(img.over[i]).then(|| {
            let t = img.top[i] as f32 + 1.0;
            ((t - 2.0).max(top), t)
        });
        (top, canopy)
    }

    /// Draws the sky (when it can be seen) and the land.
    pub fn draw(&mut self, ctx: &egui::Context, painter: &egui::Painter, img: &MapImage, cam: &Camera) {
        if cam.pitch < 80f32.to_radians() {
            overworld_sky(painter, cam, 1.0);
        }
        let mesh = match &self.last {
            Some((c, m)) if c == cam => m.clone(),
            _ => {
                let m = std::sync::Arc::new(self.build(ctx, img, cam));
                self.last = Some((*cam, m.clone()));
                m
            }
        };
        painter.add(egui::Shape::Mesh(mesh));
        // Water, lava, fire and the like move as in the game.
        if let Some(wait) = self.atlas.animate(ctx.input(|i| i.time)) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.clamp(0.01, 1.0)));
        }
    }

    fn build(&mut self, ctx: &egui::Context, img: &MapImage, cam: &Camera) -> egui::Mesh {
        let r = Self::radius(cam);
        // Fog only when the view reaches toward the horizon.
        let fogged = cam.pitch < 72f32.to_radians();
        let eye = cam.eye();
        let (ox, oz) = (img.min_x * 16, img.min_z * 16);
        let t = cam.target;
        // Looking straight down, just the ground the screen shows (and some room for what stands
        // up into view from beside it); otherwise everything within reach.
        let (hx, hz) = if cam.pitch > 85f32.to_radians() {
            // As far out as the screen's edges reach on the lowest land in view (found by a
            // sparse look over the widest area it could be, at most `BELOW` under the target).
            let extent = |below: f32| {
                let k = (cam.dist + below) / cam.focal();
                let (sy, cy) = cam.yaw.sin_cos();
                let (w, h) = (cam.rect.width() * 0.5 * k, cam.rect.height() * 0.5 * k);
                ((w * cy).abs() + (h * sy).abs() + 4.0, (w * sy).abs() + (h * cy).abs() + 4.0)
            };
            let (wx, wz) = extent(BELOW);
            let mut low = t[1];
            let mut sz = -wz;
            while sz <= wz {
                let mut sx = -wx;
                while sx <= wx {
                    if let Some(j) = index(img, (t[0] + sx) as i32, (t[2] + sz) as i32) {
                        low = low.min(img.ground[j] as f32);
                    }
                    sx += 6.0;
                }
                sz += 6.0;
            }
            let (ex, ez) = extent((t[1] - low + 3.0).clamp(4.0, BELOW));
            (ex.min(r), ez.min(r))
        } else {
            (r, r)
        };
        let x0 = ((t[0] - hx).floor() as i32).max(ox);
        let x1 = ((t[0] + hx).ceil() as i32).min(ox + img.width as i32);
        let z0 = ((t[2] - hz).floor() as i32).max(oz);
        let z1 = ((t[2] + hz).ceil() as i32).min(oz + img.height as i32);
        let view = cam.view();
        // Each column's ground top, worked out once (sides compare with the neighbors'); NaN
        // where there is no column. One column more all round, for the neighbors of the edge.
        let gw = (x1 - x0 + 2).max(0) as usize;
        let gh = (z1 - z0 + 2).max(0) as usize;
        let mut grid = vec![f32::NAN; gw * gh];
        for bz in z0 - 1..z1 + 1 {
            for bx in x0 - 1..x1 + 1 {
                if let Some(j) = index(img, bx, bz) {
                    let g = self.ground(img, j);
                    if let Some(cell) = grid.get_mut((bz - z0 + 1) as usize * gw + (bx - x0 + 1) as usize) {
                        *cell = g;
                    }
                }
            }
        }
        let ground_at = |bx: i32, bz: i32| -> Option<f32> {
            let (gx, gz) = (bx - x0 + 1, bz - z0 + 1);
            if gx < 0 || gz < 0 || gx as usize >= gw || gz as usize >= gh {
                return None;
            }
            let g = grid[gz as usize * gw + gx as usize];
            (!g.is_nan()).then_some(g)
        };
        // Columns in reach, furthest from the eye first.
        let mut cols: Vec<(f32, i32, i32, usize)> = Vec::new();
        let clip = cam.rect.expand(cam.rect.width() * 0.1);
        for bz in z0..z1 {
            for bx in x0..x1 {
                let Some(i) = index(img, bx, bz) else { continue };
                let (dx, dz) = (bx as f32 + 0.5 - t[0], bz as f32 + 0.5 - t[2]);
                if dx * dx + dz * dz > r * r {
                    continue;
                }
                // Skip columns well off screen: the span from the column's top to far below it
                // is tested as a whole (a column near the eye can cross the screen with both ends
                // off it). One end behind the eye: kept, its faces are cut one by one.
                let top = img.top[i].max(img.ground[i]) as f32 + 1.0;
                let (cx, cz) = (bx as f32 + 0.5, bz as f32 + 0.5);
                let seen = match (view.project([cx, top, cz]), view.project([cx, top - 24.0, cz])) {
                    (Some((a, za)), Some((b, zb))) => {
                        let half = view.focal / za.min(zb) * 0.75;
                        Rect::from_two_pos(a, b).expand(half + 8.0).intersects(clip)
                    }
                    // Wholly behind the eye (with room for the column's width): not seen.
                    (None, None) => {
                        let d = view.to_view([cx, top, cz])[2].max(view.to_view([cx, top - 24.0, cz])[2]);
                        d > -1.5
                    }
                    _ => true,
                };
                if !seen {
                    continue;
                }
                let (ex, ez) = (cx - eye[0], cz - eye[2]);
                cols.push((ex * ex + ez * ez, bx, bz, i));
            }
        }
        cols.sort_by(|a, b| b.0.total_cmp(&a.0));
        let white = Atlas::white();
        let mut b = Builder { view: cam.view(), mesh: egui::Mesh::default(), clip: cam.rect };
        for &(_, bx, bz, i) in &cols {
            let (dx, dz) = (bx as f32 + 0.5 - t[0], bz as f32 + 0.5 - t[2]);
            let d = (dx * dx + dz * dz).sqrt();
            // Fades into the sky toward the edge of what is drawn.
            let fog = if fogged { 1.0 - ((d - r * 0.72) / (r * 0.28)).clamp(0.0, 1.0) } else { 1.0 };
            if fog <= 0.01 {
                continue;
            }
            let (x, z) = (bx as f32, bz as f32);
            let id = img.blocks[i];
            let water = img.water[i] > 0;
            let form = if water { Form::Cube(1.0) } else { self.form(id) };
            let gy = img.ground[i] as f32;
            let gtop = ground_at(bx, bz).unwrap_or(gy + 1.0);
            let albedo = [0, 1, 2].map(|k| img.albedo[i * 3 + k] as f32 / 255.0);
            // Under water a plain floor in the color of what lies there; a post or the like
            // stands on ground taken from a neighbor (the map only knows the top block).
            let (top_slot, side_slot, under_slot) = match (water, form) {
                (true, _) => (None, None, None),
                (false, Form::Cube(_)) => (self.atlas.slot(id, Face::Top), self.atlas.slot(id, Face::Side), self.atlas.slot(id, Face::Under)),
                (false, _) => {
                    let near = [(0, -1), (0, 1), (-1, 0), (1, 0)].iter().find_map(|(nx, nz)| {
                        let j = index(img, bx + nx, bz + nz)?;
                        (img.water[j] == 0 && self.form(img.blocks[j]) == Form::Cube(1.0)).then_some(img.blocks[j])
                    });
                    let s = near.and_then(|n| self.atlas.slot(n, Face::Top));
                    (s, near.and_then(|n| self.atlas.slot(n, Face::Side)), near.and_then(|n| self.atlas.slot(n, Face::Under)))
                }
            };
            let paint = |s: Option<Slot>, shade: f32| -> (Rect, [f32; 3]) {
                match s {
                    Some(s) => (s.uv, s.tint.map(|c| c * shade)),
                    None => (white, albedo.map(|c| c * shade * if water { 0.75 } else { 1.0 })),
                }
            };
            // The top block's own height in the column (a slab is half a block).
            let own = match form {
                Form::Cube(h) => h,
                _ => 0.0,
            };
            // Sides where the neighbor is lower and the side faces the eye.
            let sides: [(i32, i32, bool, [V3; 2], f32); 4] = [
                (0, -1, eye[2] < z, [[x + 1.0, 0.0, z], [x, 0.0, z]], SHADE_NS[0]),
                (0, 1, eye[2] > z + 1.0, [[x, 0.0, z + 1.0], [x + 1.0, 0.0, z + 1.0]], SHADE_NS[1]),
                (-1, 0, eye[0] < x, [[x, 0.0, z], [x, 0.0, z + 1.0]], SHADE_EW[0]),
                (1, 0, eye[0] > x + 1.0, [[x + 1.0, 0.0, z + 1.0], [x + 1.0, 0.0, z]], SHADE_EW[1]),
            ];
            for (nx, nz, facing, [a, c], shade) in sides {
                if !facing {
                    continue;
                }
                let low = ground_at(bx + nx, bz + nz).unwrap_or(gtop - 3.0);
                // A side is seen only from the air beside it: from below the neighbor's ground
                // (under the land, as the free camera can be) it is inside solid blocks.
                if low >= gtop || eye[1] <= low {
                    continue;
                }
                // The top block's side first (as tall as the block is), then a picture a block
                // of what is under it; very tall cliffs end in one plain face.
                let mut y = gtop;
                let mut layer = 0;
                while y > low + 1e-3 {
                    let (y1, slot, part) = if layer == 0 && own > 0.0 {
                        ((gtop - own).max(low), Some(side_slot), 1.0 - own)
                    } else if layer >= 20 {
                        (low, None, 0.0)
                    } else {
                        ((y - 1.0).max(low), Some(under_slot), 0.0)
                    };
                    let (uv, rgb) = match slot {
                        Some(s) => paint(s, shade),
                        None => (white, albedo.map(|c| c * shade * 0.7)),
                    };
                    // The picture's rows for this part of the side (the lower part of a slab's
                    // side, the top part of a cut-off block).
                    let v0 = part;
                    let v1 = (v0 + (y - y1)).min(1.0);
                    let uv = Rect::from_min_max(pos2(uv.min.x, uv.min.y + uv.height() * v0), pos2(uv.max.x, uv.min.y + uv.height() * v1));
                    b.quad([[a[0], y, a[2]], [c[0], y, c[2]], [c[0], y1, c[2]], [a[0], y1, a[2]]], uv, rgb, fog);
                    y = y1;
                    layer += 1;
                }
            }
            // The top, only seen from above (from beneath, as the free camera can be, a block's
            // top is hidden by the block).
            if eye[1] > gtop {
                let (uv, rgb) = paint(top_slot, SHADE_TOP);
                b.quad([[x, gtop, z], [x + 1.0, gtop, z], [x + 1.0, gtop, z + 1.0], [x, gtop, z + 1.0]], uv, rgb, fog);
            }
            if !water {
                if let Some(fire) = self.atlas.slot(id, Face::Extra) {
                    if eye[1] > gtop {
                        cross_quads(&mut b, x, z, gtop, gtop + 1.0, fire, fog);
                    }
                }
            }
            // What stands on the ground when the top block is not a whole-width block.
            match (water, form) {
                (false, Form::Post(w, h)) => {
                    let (s_top, s_side) = (self.atlas.slot(id, Face::Top), self.atlas.slot(id, Face::Side));
                    if eye[1] > gtop {
                        b.boxed(eye, [x + 0.5 - w, gtop, z + 0.5 - w], [x + 0.5 + w, gy + h, z + 0.5 + w], s_top, s_side, albedo, fog);
                    }
                }
                (false, Form::Decal) => {
                    if let Some(s) = self.atlas.slot(id, Face::Top) {
                        let y = gtop + 0.03;
                        b.quad([[x, y, z], [x + 1.0, y, z], [x + 1.0, y, z + 1.0], [x, y, z + 1.0]], s.uv, s.tint, fog);
                    }
                }
                (false, Form::Cross) => {
                    if let Some(s) = self.atlas.slot(id, Face::Side) {
                        if eye[1] > gtop {
                            cross_quads(&mut b, x, z, gtop, gtop + 1.0, s, fog);
                        }
                    }
                }
                _ => {}
            }
            if water {
                // The surface, see-through enough to show the floor in the shallows.
                let surface = img.heights[i] as f32 + self.liquid(img.blocks[i]);
                let depth = (img.water[i] as f32 / 12.0).clamp(0.0, 1.0);
                let (uv, rgb) = match self.atlas.slot(id, Face::Top) {
                    Some(s) => (s.uv, s.tint),
                    None => (white, [0.25, 0.45, 0.9]),
                };
                b.quad(
                    [[x, surface, z], [x + 1.0, surface, z], [x + 1.0, surface, z + 1.0], [x, surface, z + 1.0]],
                    uv,
                    rgb,
                    fog * (0.55 + 0.4 * depth),
                );
            }
            let over = img.over[i];
            if over == 0 {
                continue;
            }
            if self.is_leaves(over) {
                // A canopy two blocks deep at the height the leaves were found.
                let top = img.top[i] as f32 + 1.0;
                let bottom = (top - 2.0).max(gtop);
                let Some(ls) = self.atlas.slot(over, Face::Top) else { continue };
                for (nx, nz, facing, [a, c], shade) in sides {
                    let next = index(img, bx + nx, bz + nz);
                    let covered = next.is_some_and(|j| self.is_leaves(img.over[j]) && img.top[j] >= img.top[i]);
                    if !facing || covered {
                        continue;
                    }
                    let uv = ls.part(0.0, 1.0, 0.0, ((top - bottom) / 2.0).min(1.0));
                    b.quad([[a[0], top, a[2]], [c[0], top, c[2]], [c[0], bottom, c[2]], [a[0], bottom, a[2]]], uv, ls.tint.map(|k| k * shade), fog);
                }
                if eye[1] > top {
                    b.quad([[x, top, z], [x + 1.0, top, z], [x + 1.0, top, z + 1.0], [x, top, z + 1.0]], ls.uv, ls.tint, fog);
                }
                // Seen from beneath, the underside of the canopy.
                if eye[1] < bottom {
                    b.quad([[x, bottom, z], [x + 1.0, bottom, z], [x + 1.0, bottom, z + 1.0], [x, bottom, z + 1.0]], ls.uv, ls.tint.map(|k| k * 0.55), fog);
                }
            } else if let Some(ps) = self.atlas.slot(over, Face::Side) {
                // A plant is seen only from above the ground it stands on.
                if eye[1] > gtop {
                    cross_quads(&mut b, x, z, gtop, gtop + 1.0, ps, fog);
                }
            }
        }
        // Uploaded after the faces are made, as making them puts new block pictures in it.
        b.mesh.texture_id = self.atlas.texture(ctx);
        b.mesh
    }

    /// The block under a point on the screen: the ray from the eye is walked column by column
    /// (as a grid traversal) until it meets a column's ground, a post or a canopy, side or top.
    pub fn pick(&mut self, img: &MapImage, cam: &Camera, at: Pos2) -> Option<Hit> {
        let (eye, dir) = cam.ray(at);
        let (dx, dz) = (dir[0], dir[2]);
        let (mut cx, mut cz) = (eye[0].floor() as i32, eye[2].floor() as i32);
        let first = |d: f32, e: f32, c: i32| {
            if d.abs() < 1e-9 {
                f32::INFINITY
            } else if d > 0.0 {
                (c as f32 + 1.0 - e) / d
            } else {
                (c as f32 - e) / d
            }
        };
        let (mut tmx, mut tmz) = (first(dx, eye[0], cx), first(dz, eye[2], cz));
        let (tdx, tdz) = (if dx.abs() < 1e-9 { f32::INFINITY } else { 1.0 / dx.abs() }, if dz.abs() < 1e-9 { f32::INFINITY } else { 1.0 / dz.abs() });
        let (sx, sz) = (if dx > 0.0 { 1 } else { -1 }, if dz > 0.0 { 1 } else { -1 });
        let reach = Self::radius(cam) * 2.0 + cam.dist + 64.0;
        let mut t0 = 0.0f32;
        for _ in 0..4096 {
            let t1 = tmx.min(tmz).min(reach);
            if let Some(i) = index(img, cx, cz) {
                let (top, canopy) = self.solid(img, i);
                let (y0, y1) = (eye[1] + dir[1] * t0, eye[1] + dir[1] * t1);
                let column = |y: f32, lo: f32, hi: f32| {
                    let by = y.clamp(lo, hi - 1e-3).floor();
                    Hit::inset(cx, by as i32, cz, by.max(lo), (by + 1.0).min(hi), 0.0)
                };
                // A flower, grass or the like standing on the ground: the block above the ground,
                // outlined narrower as the game does.
                let plant = (img.over[i] != 0 && img.water[i] == 0 && !self.is_leaves(img.over[i])).then_some((top, top + 0.8));
                if let Some((lo, hi)) = plant {
                    let (a, b) = (y0.min(y1), y0.max(y1));
                    if y0 > top && a <= hi && b >= lo {
                        return Some(Hit::inset(cx, lo.floor() as i32, cz, lo, hi, 0.2));
                    }
                }
                // The ground entered from the side (below its top); then leaves over it, met
                // before the ground beneath them; then the ground's top crossed.
                if y0 <= top {
                    return Some(column(y0, f32::MIN, top));
                }
                if let Some((lo, hi)) = canopy {
                    let (a, b) = (y0.min(y1), y0.max(y1));
                    if a <= hi && b >= lo {
                        let y = if (lo..=hi).contains(&y0) { y0 } else if y0 > hi { hi - 1e-3 } else { lo };
                        return Some(column(y, lo, hi));
                    }
                }
                if y1 <= top {
                    let by = (top - 1e-3).floor();
                    return Some(Hit::inset(cx, by as i32, cz, by, top, 0.0));
                }
            }
            if t1 >= reach {
                break;
            }
            if tmx < tmz {
                cx += sx;
                t0 = tmx;
                tmx += tdx;
            } else {
                cz += sz;
                t0 = tmz;
                tmz += tdz;
            }
        }
        None
    }

    /// How far the whole camera (eye and what it looks at together, so its angle and distance stay)
    /// must rise for nothing to be in the way: the line from what it looks at to the eye passing
    /// over the land, and the eye clear of whatever is right around it (leaves too). 0 when clear.
    pub fn clear_lift(&mut self, img: &MapImage, cam: &Camera) -> f32 {
        let eye = cam.eye();
        let back = mul(sub(eye, cam.target), 1.0 / cam.dist.max(1e-3));
        // The line is drawn from just above the land at the target (the height looked at eases
        // and can lag inside a hill while moving; that is not land in the way).
        let base = self.highest_near(img, cam.target[0], cam.target[2], 1)
            .map_or(0.0, |h| (h + 0.5 - cam.target[1]).max(0.0));
        let from = [cam.target[0], cam.target[1] + base, cam.target[2]];
        let mut need = 0.0f32;
        let mut s = 1.5;
        while s < cam.dist {
            let p = add(from, mul(back, s));
            for (ex, ez) in [(0.0, 0.0), (0.4, 0.4), (-0.4, 0.4), (0.4, -0.4), (-0.4, -0.4)] {
                let Some(i) = index(img, (p[0] + ex).floor() as i32, (p[2] + ez).floor() as i32) else { continue };
                let (top, _) = self.solid(img, i);
                need = need.max(top + 0.6 - p[1]);
            }
            s += 0.25;
        }
        // The eye itself, out of blocks and leaves around it.
        if let Some(h) = self.highest_near(img, eye[0], eye[2], 1) {
            need = need.max(h + 0.8 - eye[1]);
        }
        need.max(0.0)
    }

    /// The height of the land at a point: the column's ground (or water) top.
    pub fn surface_at(&mut self, img: &MapImage, x: f32, z: f32) -> Option<f32> {
        let i = index(img, x.floor() as i32, z.floor() as i32)?;
        Some(if img.water[i] > 0 { img.heights[i] as f32 + self.liquid(img.blocks[i]) } else { self.ground(img, i) })
    }

    /// The height of the land around a point (the median of the ground near it, so one tree or
    /// hole does not throw the camera about).
    /// (Each column at its block's real height: redstone, a carpet, rails, fire or flowers raise
    /// it only as much as they are tall, not a whole block.)
    pub fn ground_near(&mut self, img: &MapImage, x: f32, z: f32) -> Option<f32> {
        let mut ys = Vec::new();
        for dz in -3..=3 {
            for dx in -3..=3 {
                let Some(i) = index(img, x.floor() as i32 + dx * 2, z.floor() as i32 + dz * 2) else { continue };
                ys.push(if img.water[i] > 0 { img.heights[i] as f32 + self.liquid(img.blocks[i]) } else { self.ground(img, i) });
            }
        }
        if ys.is_empty() {
            return None;
        }
        ys.sort_by(|a, b| a.total_cmp(b));
        Some(ys[ys.len() / 2])
    }

    /// The highest thing (ground, water or leaves) within `reach` columns of a point, each at its
    /// real height (thin things like redstone and carpets, and plants, barely count).
    pub fn highest_near(&mut self, img: &MapImage, x: f32, z: f32, reach: i32) -> Option<f32> {
        let mut best: Option<f32> = None;
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let Some(i) = index(img, x.floor() as i32 + dx, z.floor() as i32 + dz) else { continue };
                let (top, canopy) = self.solid(img, i);
                let top = canopy.map_or(top, |(_, t)| t.max(top));
                best = Some(best.map_or(top, |b: f32| b.max(top)));
            }
        }
        best
    }

    /// Outlines a picked block as the game does: the edges of its box that can be seen.
    pub fn outline(painter: &egui::Painter, cam: &Camera, hit: &Hit) {
        let lo = [hit.x as f32 + hit.xz[0], hit.y0, hit.z as f32 + hit.xz[2]];
        let hi = [hit.x as f32 + hit.xz[1], hit.y1, hit.z as f32 + hit.xz[3]];
        let stroke = egui::Stroke::new(2.0_f32, Color32::from_rgba_unmultiplied(10, 10, 10, 210));
        box_lines(painter, cam, lo, hi, stroke, None, None);
    }

    /// A selection box from `lo` to `hi` (block corners): faces that face the eye tinted, the
    /// edges in front drawn strongly and the ones behind faintly (as MCEdit shows it).
    pub fn selection(painter: &egui::Painter, cam: &Camera, lo: V3, hi: V3, color: Color32) {
        let strong = egui::Stroke::new(2.0_f32, color);
        let faint = egui::Stroke::new(1.0_f32, color.gamma_multiply(0.45));
        box_lines(painter, cam, lo, hi, strong, Some(faint), Some(color.gamma_multiply(0.16)));
    }

    /// The top of the land in a column, for drawing lines on it (the chunk grid and highlight):
    /// the ground, under any water or lava (they count as air, as in the game's F3+G).
    fn land_top(&mut self, img: &MapImage, bx: i32, bz: i32) -> Option<f32> {
        let i = index(img, bx, bz)?;
        Some(self.ground(img, i))
    }

    /// A chunk's border drawn on the land, stepping up and down with it (as the game's chunk
    /// border lines do), so it reads in 3D from any angle.
    pub fn chunk_outline(&mut self, painter: &egui::Painter, img: &MapImage, cam: &Camera, cx: i32, cz: i32, stroke: egui::Stroke) {
        let view = cam.view();
        let (x0, z0) = (cx * 16, cz * 16);
        // Each side as the columns along it inside the chunk, and where the line runs.
        for side in 0..4 {
            let mut prev: Option<(V3, f32)> = None;
            for k in 0..16 {
                let (bx, bz, a, b) = match side {
                    0 => (x0 + k, z0, [(x0 + k) as f32, (z0) as f32], [(x0 + k + 1) as f32, z0 as f32]),
                    1 => (x0 + k, z0 + 15, [(x0 + k) as f32, (z0 + 16) as f32], [(x0 + k + 1) as f32, (z0 + 16) as f32]),
                    2 => (x0, z0 + k, [x0 as f32, (z0 + k) as f32], [x0 as f32, (z0 + k + 1) as f32]),
                    _ => (x0 + 15, z0 + k, [(x0 + 16) as f32, (z0 + k) as f32], [(x0 + 16) as f32, (z0 + k + 1) as f32]),
                };
                let Some(h) = self.land_top(img, bx, bz) else {
                    prev = None;
                    continue;
                };
                let (p, q) = ([a[0], h + 0.02, a[1]], [b[0], h + 0.02, b[1]]);
                // A step where the land changes height between columns.
                if let Some((last, lh)) = prev {
                    if (lh - h).abs() > 1e-3 {
                        segment(painter, &view, last, p, stroke);
                    }
                }
                segment(painter, &view, p, q, stroke);
                prev = Some((q, h));
            }
        }
    }

    /// How high chunk walls are drawn: to 	op, but looking down not past just below the eye
    /// (above it they would be behind the camera).
    fn wall_top(cam: &Camera, top: f32) -> f32 {
        if cam.pitch > 70f32.to_radians() { top.min(cam.eye()[1] - 2.0) } else { top }
    }

    /// The top of the land where a chunk's edge runs between two columns (the higher of the
    /// two), if either is known.
    fn edge_ground(&mut self, img: &MapImage, a: (i32, i32), b: (i32, i32)) -> Option<f32> {
        match (self.land_top(img, a.0, a.1), self.land_top(img, b.0, b.1)) {
            (Some(p), Some(q)) => Some(p.max(q)),
            (p, q) => p.or(q),
        }
    }

    /// A chunk drawn as the game draws the chunk you stand in (F3+G): its four walls rising from
    /// the land to `top`, lined every `across` blocks along and every `up` blocks up. Nothing is
    /// drawn below the land, so the lines never show through the ground. With `land`, also the
    /// line along the top of the land (in its own color, see [`LAND_LINE`]).
    #[allow(clippy::too_many_arguments)]
    pub fn chunk_walls(&mut self, painter: &egui::Painter, img: &MapImage, cam: &Camera, cx: i32, cz: i32, stroke: egui::Stroke, across: i32, up: i32, top: f32, land: bool) {
        let view = cam.view();
        let top = Self::wall_top(cam, top);
        let (x0, z0) = (cx * 16, cz * 16);
        // Each side: where its line starts, the way it runs, and (from a point on the line) the
        // column just inside the chunk and the one just outside.
        type Side = ((i32, i32), (i32, i32), (i32, i32), (i32, i32));
        let sides: [Side; 4] = [
            ((x0, z0), (1, 0), (0, 0), (0, -1)),
            ((x0, z0 + 16), (1, 0), (0, -1), (0, 0)),
            ((x0, z0), (0, 1), (0, 0), (-1, 0)),
            ((x0 + 16, z0), (0, 1), (-1, 0), (0, 0)),
        ];
        // No line is drawn twice (each corner post by one side only): the same line twice would
        // be inverted twice where lines are drawn inverted.
        for (side, (start, dir, inside, outside)) in sides.into_iter().enumerate() {
            // The land along the side, block by block.
            let ground: Vec<Option<f32>> = (0..16)
                .map(|k| {
                    let (x, z) = (start.0 + dir.0 * k, start.1 + dir.1 * k);
                    self.edge_ground(img, (x + inside.0, z + inside.1), (x + outside.0, z + outside.1))
                })
                .collect();
            let low = ground.iter().flatten().fold(f32::MAX, |m, g| m.min(*g));
            if low == f32::MAX {
                continue;
            }
            // Upright lines: from the land at that point up to the top. The corners are the
            // first two sides' to draw.
            for k in (0..=16i32).step_by(across.max(1) as usize) {
                let g = [k.checked_sub(1).and_then(|j| ground.get(j as usize).copied().flatten()), ground.get(k as usize).copied().flatten()];
                let Some(g) = g.iter().flatten().fold(None, |m: Option<f32>, v| Some(m.map_or(*v, |m| m.max(*v)))) else { continue };
                if side >= 2 && (k == 0 || k == 16) {
                    continue;
                }
                let (x, z) = ((start.0 + dir.0 * k) as f32, (start.1 + dir.1 * k) as f32);
                segment(painter, &view, [x, g, z], [x, top, z], stroke);
            }
            // Level lines every `up` blocks where they are above the land, each run of blocks one
            // unbroken line (drawn inverted, pieces overlapping at their ends cancelled out and
            // left gaps).
            let mut y = (low / up as f32).ceil() * up as f32;
            while y <= top {
                let mut run: Option<i32> = None;
                for k in 0..=16 {
                    let open = k < 16 && ground[k as usize].is_some_and(|g| y >= g);
                    match (open, run) {
                        (true, None) => run = Some(k),
                        (false, Some(from)) => {
                            let at = |k: i32| [(start.0 + dir.0 * k) as f32, y, (start.1 + dir.1 * k) as f32];
                            segment(painter, &view, at(from), at(k), stroke);
                            run = None;
                        }
                        _ => {}
                    }
                }
                y += up as f32;
            }
        }
        if land {
            self.chunk_outline(painter, img, cam, cx, cz, egui::Stroke::new(1.5_f32, LAND_LINE));
        }
    }

    /// Chunk borders around the camera in 3D, as the game marks the chunks around the one you
    /// stand in: an upright line at each chunk corner, rising from the land to `top`.
    pub fn chunk_grid(&mut self, painter: &egui::Painter, img: &MapImage, cam: &Camera, top: f32) {
        let top = Self::wall_top(cam, top);
        let r = Self::radius(cam).min(160.0);
        let t = cam.target;
        let (c0x, c1x) = (((t[0] - r) / 16.0).floor() as i32, ((t[0] + r) / 16.0).ceil() as i32);
        let (c0z, c1z) = (((t[2] - r) / 16.0).floor() as i32, ((t[2] + r) / 16.0).ceil() as i32);
        let stroke = egui::Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 220, 60, 170));
        let view = cam.view();
        for cz in c0z..=c1z {
            for cx in c0x..=c1x {
                let (x, z) = (cx * 16, cz * 16);
                // The highest of the four columns meeting at the corner.
                let g = [(x - 1, z - 1), (x, z - 1), (x - 1, z), (x, z)]
                    .iter()
                    .filter_map(|(a, b)| self.land_top(img, *a, *b))
                    .fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v))));
                if let Some(g) = g {
                    segment(painter, &view, [x as f32, g, z as f32], [x as f32, top, z as f32], stroke);
                }
            }
        }
    }
}

thread_local! {
    /// While set, lines in the world are gathered here (to be drawn by the GPU, hidden behind
    /// blocks) instead of painted over everything.
    static LINES: std::cell::RefCell<Option<Vec<(V3, V3, Color32)>>> = const { std::cell::RefCell::new(None) };
}

/// Gathers the lines drawn in the world from now on (see [`take_lines`]) instead of painting them.
pub(crate) fn gather_lines() {
    LINES.with(|l| *l.borrow_mut() = Some(Vec::new()));
}

/// The lines gathered since [`gather_lines`], and painting them again from now on.
pub(crate) fn take_lines() -> Vec<(V3, V3, Color32)> {
    LINES.with(|l| l.borrow_mut().take().unwrap_or_default())
}

/// A line between two points, cut at the near plane (or gathered for the GPU).
fn segment(painter: &egui::Painter, view: &View, a: V3, b: V3, stroke: egui::Stroke) {
    if LINES.with(|l| l.borrow_mut().as_mut().map(|v| v.push((a, b, stroke.color))).is_some()) {
        return;
    }
    let (va, vb) = (view.to_view(a), view.to_view(b));
    if va[2] < NEAR && vb[2] < NEAR {
        return;
    }
    let cut = |p: V3, q: V3| if p[2] >= NEAR { p } else { lerp3(p, q, (NEAR - p[2]) / (q[2] - p[2])) };
    painter.line_segment([view.to_screen(cut(va, vb)), view.to_screen(cut(vb, va))], stroke);
}

/// The edges of a box (those that can be seen in `front`, the others in `back` if given) and its
/// faces toward the eye filled with `fill`.
fn box_lines(painter: &egui::Painter, cam: &Camera, lo: V3, hi: V3, front: egui::Stroke, back: Option<egui::Stroke>, fill: Option<Color32>) {
    let g = 0.004;
    let (lo, hi) = ([lo[0] - g, lo[1] - g, lo[2] - g], [hi[0] + g, hi[1] + g, hi[2] + g]);
    let view = cam.view();
    let eye = view.eye;
    // Which of the six faces (-x, +x, -y, +y, -z, +z) face the eye.
    let faces = [eye[0] < lo[0], eye[0] > hi[0], eye[1] < lo[1], eye[1] > hi[1], eye[2] < lo[2], eye[2] > hi[2]];
    let corner = |m: usize| [if m & 1 == 0 { lo[0] } else { hi[0] }, if m & 2 == 0 { lo[1] } else { hi[1] }, if m & 4 == 0 { lo[2] } else { hi[2] }];
    // (Lines handed to the GPU are drawn among the blocks; the tint is left out there.)
    if let Some(fill) = fill.filter(|_| LINES.with(|l| l.borrow().is_none())) {
        // Each face as its four corners (by the bits of the two axes it spans).
        for axis in 0..3usize {
            for side in 0..2usize {
                if !faces[axis * 2 + side] {
                    continue;
                }
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                let base = side << axis;
                let ring = [base, base | 1 << u, base | 1 << u | 1 << v, base | 1 << v];
                let pts: Option<Vec<Pos2>> = ring.iter().map(|m| view.project(corner(*m)).map(|(s, _)| s)).collect();
                if let Some(pts) = pts {
                    painter.add(egui::Shape::convex_polygon(pts, fill, egui::Stroke::NONE));
                }
            }
        }
    }
    for a in 0..8usize {
        for axis in 0..3usize {
            let bit = 1 << axis;
            if a & bit != 0 {
                continue;
            }
            // The two faces this edge borders (it runs along `axis`).
            let (k1, k2) = ((axis + 1) % 3, (axis + 2) % 3);
            let face = |k: usize| faces[k * 2 + ((a >> k) & 1)];
            let stroke = if face(k1) || face(k2) {
                front
            } else {
                match back {
                    Some(s) => s,
                    None => continue,
                }
            };
            segment(painter, &view, corner(a), corner(a | bit), stroke);
        }
    }
}

/// Two crossed pictures standing on the ground between `y0` and `y1`.
fn cross_quads(b: &mut Builder, x: f32, z: f32, y0: f32, y1: f32, s: Slot, fog: f32) {
    for (a, c) in [([x, z], [x + 1.0, z + 1.0]), ([x + 1.0, z], [x, z + 1.0])] {
        b.quad([[a[0], y1, a[1]], [c[0], y1, c[1]], [c[0], y0, c[1]], [a[0], y0, a[1]]], s.uv, s.tint.map(|k| k * 0.9), fog);
    }
}

/// The sky of a dimension as the game draws it. The Overworld's: deep blue overhead to pale
/// at the horizon, darkening toward night as `daylight` (1 by day) falls, with the sun and
/// moon where `ticks` (the time of day) puts them, stars at night and clouds drifting at
/// height 192 (`secs` moves them); nothing below the horizon (the void). The Nether's and the
/// End's are each one sky, whatever the time.
pub(crate) fn sky(painter: &egui::Painter, cam: &Camera, daylight: f32, ticks: i64, dimension: &str, secs: f64) {
    let r = cam.rect;
    match dimension {
        // The Nether's fog color.
        "minecraft:the_nether" => {
            painter.rect_filled(r, 0.0, Color32::from_rgb(0x33, 0x08, 0x08));
            return;
        }
        // The End's sky: its dark speckled texture, tiled.
        "minecraft:the_end" => {
            painter.rect_filled(r, 0.0, Color32::from_rgb(0x12, 0x0E, 0x18));
            if let Some(tex) = crate::icons::pack_texture("environment/end_sky.png").and_then(|f| crate::icons::file_texture(painter.ctx(), &f)) {
                let tile = 256.0;
                let mut y = r.top();
                while y < r.bottom() {
                    let mut x = r.left();
                    while x < r.right() {
                        painter.image(tex.id(), Rect::from_min_size(pos2(x, y), egui::vec2(tile, tile)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::from_gray(60));
                        x += tile;
                    }
                    y += tile;
                }
            }
            return;
        }
        _ => {}
    }
    overworld_sky(painter, cam, daylight);
    let day = ((daylight - 4.0 / 15.0) / (11.0 / 15.0)).clamp(0.0, 1.0);
    let (eye, ..) = cam.basis();
    let f = cam.focal();
    let far = |d: V3| cam.project([eye[0] + d[0] * 1.0e4, eye[1] + d[1] * 1.0e4, eye[2] + d[2] * 1.0e4]).map(|(p, _)| p);
    // The sky turns about the north-south line: the sun rises in the east (time 0), is overhead
    // at noon (6000) and sets in the west; the moon is opposite it; the stars turn with it.
    let a = ticks.rem_euclid(24000) as f32 / 24000.0 * std::f32::consts::TAU;
    let turn = |d: V3| [d[0] * a.cos() - d[1] * a.sin(), d[0] * a.sin() + d[1] * a.cos(), d[2]];
    let night = 1.0 - day;
    if night > 0.02 {
        let mut m = egui::Mesh::default();
        for (d, size, glow) in stars() {
            let d = turn(*d);
            if d[1] < 0.0 {
                continue;
            }
            if let Some(p) = far(d).filter(|p| r.contains(*p)) {
                let c = Color32::from_white_alpha((night * glow * 255.0) as u8);
                m.add_colored_rect(Rect::from_center_size(p, egui::vec2(*size, *size)), c);
            }
        }
        painter.add(egui::Shape::mesh(m));
    }
    let sun = [a.cos(), a.sin(), 0.0];
    // The picture is the first of `files` the pack has (the game moved them to `celestial/`),
    // each with the part of it to draw.
    let disc = |files: &[(&str, Rect)], d: V3, size: f32| {
        if d[1] < -0.08 {
            return;
        }
        let Some(p) = far(d) else { return };
        let Some((tex, uv)) = files.iter().find_map(|(file, uv)| additive_texture(painter.ctx(), file).map(|t| (t, *uv))) else { return };
        painter.with_clip_rect(r).image(tex.id(), Rect::from_center_size(p, egui::vec2(size, size)), uv, Color32::WHITE);
    };
    let whole = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    // The clouds first: the sun and moon glow through them (added on top, as the game blends
    // them), rather than being lost behind them.
    clouds(painter, cam, day, secs);
    // As big as the game's: the sun 30 across at 100 away, the moon 20 (full).
    disc(&[("environment/celestial/sun.png", whole), ("environment/sun.png", whole)], sun, 0.3 * f);
    disc(
        &[("environment/celestial/moon/full_moon.png", whole), ("environment/moon_phases.png", Rect::from_min_max(pos2(0.0, 0.0), pos2(0.25, 0.5)))],
        [-sun[0], -sun[1], 0.0],
        0.2 * f,
    );
}

/// One of the game's sky pictures (a glow on black: the sun, the moon) made to be added to
/// what is under it, as the game blends them: its black adds nothing, its glow lightens. (Each
/// pixel's color kept with no coverage, which egui's premultiplied blending adds.) Once each.
fn additive_texture(ctx: &egui::Context, file: &str) -> Option<egui::TextureHandle> {
    static MADE: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Option<egui::TextureHandle>>>> = std::sync::LazyLock::new(Default::default);
    if let Some(t) = MADE.lock().ok()?.get(file) {
        return t.clone();
    }
    let made = crate::icons::pack_texture(file).and_then(|path| image::open(path).ok()).map(|img| {
        let img = img.to_rgba8();
        let pixels = img.pixels().map(|p| {
            let k = p[3] as f32 / 255.0;
            Color32::from_rgba_premultiplied((p[0] as f32 * k) as u8, (p[1] as f32 * k) as u8, (p[2] as f32 * k) as u8, 0)
        });
        let color = egui::ColorImage { size: [img.width() as usize, img.height() as usize], pixels: pixels.collect() };
        ctx.load_texture(format!("sky:{file}"), color, egui::TextureOptions::NEAREST)
    });
    MADE.lock().ok()?.insert(file.to_string(), made.clone());
    made
}

/// The stars: places on the sky (as directions), their sizes on screen and brightness, the
/// same every time.
fn stars() -> &'static [(V3, f32, f32)] {
    static STARS: std::sync::OnceLock<Vec<(V3, f32, f32)>> = std::sync::OnceLock::new();
    STARS.get_or_init(|| {
        let mut seed: u64 = 10842;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f32) / (1u64 << 31) as f32
        };
        let mut out = Vec::new();
        while out.len() < 1500 {
            let d = [next() * 2.0 - 1.0, next() * 2.0 - 1.0, next() * 2.0 - 1.0];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if !(0.01..=1.0).contains(&l) {
                continue;
            }
            out.push(([d[0] / l, d[1] / l, d[2] / l], 1.0 + next() * 1.5, 0.4 + next() * 0.6));
        }
        out
    })
}

/// The clouds as the game's flat ones: its cloud picture laid over the sky at height 192, each
/// of its pixels 12 blocks across, drifting east; fading out far off, gray at night.
fn clouds(painter: &egui::Painter, cam: &Camera, day: f32, secs: f64) {
    static PICTURE: std::sync::OnceLock<Option<image::RgbaImage>> = std::sync::OnceLock::new();
    let Some(img) = PICTURE.get_or_init(|| crate::icons::pack_texture("environment/clouds.png").and_then(|f| image::open(f).ok()).map(|i| i.to_rgba8())) else { return };
    let (w, h) = (img.width() as i64, img.height() as i64);
    if w == 0 || h == 0 {
        return;
    }
    const CELL: f32 = 12.0;
    const HEIGHT: f32 = 192.0;
    const REACH: i64 = 26;
    let (eye, ..) = cam.basis();
    let drift = (secs * 0.6) as f32;
    let (ex, ez) = (((eye[0] - drift) / CELL).floor() as i64, (eye[2] / CELL).floor() as i64);
    let shade = (60.0 + 195.0 * day) as u8;
    let mut m = egui::Mesh::default();
    for dz in -REACH..=REACH {
        for dx in -REACH..=REACH {
            let fade = 1.0 - ((dx * dx + dz * dz) as f32).sqrt() / REACH as f32;
            if fade <= 0.0 {
                continue;
            }
            let (u, v) = (ex + dx, ez + dz);
            if img.get_pixel(u.rem_euclid(w) as u32, v.rem_euclid(h) as u32)[3] < 128 {
                continue;
            }
            let (x0, z0) = (u as f32 * CELL + drift, v as f32 * CELL);
            let corners = [[x0, HEIGHT, z0], [x0 + CELL, HEIGHT, z0], [x0 + CELL, HEIGHT, z0 + CELL], [x0, HEIGHT, z0 + CELL]];
            let pts: Option<Vec<Pos2>> = corners.iter().map(|c| cam.project(*c).map(|(p, _)| p)).collect();
            let Some(pts) = pts else { continue };
            let c = Color32::from_rgba_unmultiplied(shade, shade, shade, (200.0 * fade) as u8);
            let i = m.vertices.len() as u32;
            for p in pts {
                m.colored_vertex(p, c);
            }
            m.add_triangle(i, i + 1, i + 2);
            m.add_triangle(i, i + 2, i + 3);
        }
    }
    painter.with_clip_rect(cam.rect).add(egui::Shape::mesh(m));
}

/// The Overworld's sky color: deep blue overhead to pale at the horizon, darker at night.
fn overworld_sky(painter: &egui::Painter, cam: &Camera, daylight: f32) {
    let r = cam.rect;
    // Where the horizon sits: the projection of a point far ahead at eye level.
    let (eye, fwd, ..) = cam.basis();
    let ahead = [eye[0] + fwd[0] * 1.0e4, eye[1], eye[2] + fwd[2] * 1.0e4];
    let level = (fwd[0] * fwd[0] + fwd[2] * fwd[2]).sqrt();
    let horizon = if level < 0.01 { r.top() - 1.0 } else { cam.project(ahead).map_or(r.top() - 1.0, |(p, _)| p.y) };
    let day = ((daylight - 4.0 / 15.0) / (11.0 / 15.0)).clamp(0.0, 1.0);
    let mix = |d: [u8; 3], n: [u8; 3]| Color32::from_rgb(
        (n[0] as f32 + (d[0] as f32 - n[0] as f32) * day) as u8,
        (n[1] as f32 + (d[1] as f32 - n[1] as f32) * day) as u8,
        (n[2] as f32 + (d[2] as f32 - n[2] as f32) * day) as u8,
    );
    let top = mix([74, 128, 214], [8, 12, 30]);
    let pale = mix([178, 208, 238], [26, 34, 64]);
    // (Not `clamp`, which panics should the rect ever be inside out.)
    let h = horizon.max(r.top()).min(r.bottom());
    let mut m = egui::Mesh::default();
    let band = |m: &mut egui::Mesh, y0: f32, y1: f32, c0: Color32, c1: Color32| {
        if y1 <= y0 {
            return;
        }
        let i = m.vertices.len() as u32;
        m.colored_vertex(pos2(r.left(), y0), c0);
        m.colored_vertex(pos2(r.right(), y0), c0);
        m.colored_vertex(pos2(r.right(), y1), c1);
        m.colored_vertex(pos2(r.left(), y1), c1);
        m.add_triangle(i, i + 1, i + 2);
        m.add_triangle(i, i + 2, i + 3);
    };
    // Only above the horizon: below it, where there is no world (ungenerated chunks, past the
    // edge), is the void, drawn as the flat map draws it, so empty space looks the same at
    // every stage of the view (as the game shows the void below the horizon).
    band(&mut m, r.top(), h, top, pale);
    // Below the horizon, past the land drawn, the game shows the haze of its horizon going to
    // a deeper blue, not a hole.
    let low = mix([92, 120, 170], [10, 14, 30]);
    band(&mut m, h, r.bottom(), pale, low);
    painter.add(egui::Shape::mesh(m));
}

#[cfg(test)]
mod sky_tests {
    use super::*;

    /// The sun is drawn (the game's picture, added on) when the camera faces it, low in the
    /// east in the morning; at night the moon is, opposite; and nothing of either when they are
    /// below the horizon behind the camera.
    #[test]
    fn the_sun_and_moon_are_drawn_where_they_are() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let ctx = egui::Context::default();
        let drawn = |ticks: i64, yaw: f32| {
            let cam = Camera { target: [0.0, 80.0, 0.0], yaw, pitch: -0.2, dist: 0.5, rect, fov: FOV_Y };
            let out = ctx.run(egui::RawInput { screen_rect: Some(rect), ..Default::default() }, |ctx| {
                let p = ctx.layer_painter(egui::LayerId::background());
                sky(&p, &cam, 1.0, ticks, "minecraft:overworld", 0.0);
            });
            out.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Mesh(m) if m.texture_id != egui::TextureId::default()))
        };
        let yaws: Vec<f32> = (0..16).map(|k| k as f32 / 16.0 * std::f32::consts::TAU).collect();
        // Morning: the sun a little above the east.
        let sun_seen: Vec<f32> = yaws.iter().copied().filter(|y| drawn(1000, *y)).collect();
        assert!(!sun_seen.is_empty(), "the sun is drawn when faced");
        assert!(sun_seen.len() < yaws.len(), "and not whichever way the camera faces");
        // Evening's opposite, the moon at night: seen some way too.
        assert!(yaws.iter().any(|y| drawn(13000, *y)), "the moon is drawn when faced");
    }
}
