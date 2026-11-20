//! The world map used by the World Explorer and the Pruner: rulers, pan and zoom, a
//! chunk/block tooltip, the Locate bar, dimension buttons and the Slicer.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use eframe::egui::{
    self, Align2, Color32, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2, pos2, vec2,
};

use crate::atlas;
use crate::icons::{self, Icon};
use crate::jobs::Job;
use crate::street_view::{Camera, FOV_Y, Hit, Street};
use crate::theme;
use crate::widgets;
use crate::world::{self, Edition, MapImage, MapProgress};

const RULER: f32 = 20.0;
/// The closest the top-down map goes (pixels a block); zooming in further tilts into the
/// street view.
const TOP_MAX: f32 = 16.0;
/// Zoomed out past this (pixels a block) the map is all Biome Atlas; in from `ATLAS_END` it is
/// all terrain, and in between the one fades into the other.
const ATLAS_FULL: f32 = 0.2;
const ATLAS_END: f32 = 0.45;
/// How much the street view tilts for each unit of scroll (in the map's zoom steps): about
/// eight notches of a mouse wheel from top-down to fully tilted.
const TILT_RATE: f32 = 1.2;
/// The nearest (in blocks) the usual 3D camera comes to what it looks at.
const USUAL_NEAREST: f32 = 8.0;
/// From this zoom in (pixels a block), where the flat map would stop showing the lie of the land,
/// the map is drawn in 3D looking straight down (the "above view"), with cliffs and trees standing.
const ABOVE_FROM: f32 = 4.0;
/// From this zoom in (looking down) the block under the pointer is outlined; further out the
/// blocks are too small to be seen as blocks and only the chunk is.
const OUTLINE_FROM: f32 = 8.0;

/// The time of day the 3D view is lit for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TimeOfDay {
    World,
    /// The sun low in the west (11,500), still daylight: to see the sun.
    Sunset,
    Day,
    Night,
}

const DIMS: [&str; 3] = [
    "minecraft:overworld",
    "minecraft:the_nether",
    "minecraft:the_end",
];

/// What a right-click in the blocks' views was on: an entity (its saved data and where it
/// stands), or a block (where, its Java name and properties).
#[derive(Clone)]
pub enum MenuTarget {
    Entity(Arc<mctk_nbt::Tag>, [f64; 3]),
    Block([i32; 3], String, String),
}

/// Search's Find: what is looked for (0 blocks and mobs, 1 blocks, 2 mobs), how far from the
/// camera, the search running (chunks done of all, and the way to stop it) and what it found.
struct Find {
    /// The item picked to find (a block's id, or a spawn egg's for its mob).
    query: String,
    radius: f32,
    job: Option<Job<Vec<crate::gpu_world::Found>>>,
    done: Arc<std::sync::atomic::AtomicUsize>,
    total: Arc<std::sync::atomic::AtomicUsize>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    results: Vec<crate::gpu_world::Found>,
    note: Option<String>,
    /// Its text box has the keyboard (the panel stays open).
    typing: bool,
    /// For development: search as soon as the panel is drawn.
    start_soon: bool,
}

impl Default for Find {
    fn default() -> Self {
        Find {
            query: String::new(),
            radius: 128.0,
            job: None,
            done: Default::default(),
            total: Default::default(),
            stop: Default::default(),
            results: Vec::new(),
            note: None,
            typing: false,
            start_soon: false,
        }
    }
}

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
    job: Option<Job<MapImage>>,
    progress: Option<Arc<MapProgress>>,
    started: Option<Instant>,
    stream_dirty: bool,
    last_upload: Option<Instant>,
    error: Option<String>,
    /// Camera center in block coordinates and scale in pixels per block.
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
    /// The chunks picked in the World Explorer: click one, Ctrl+click to add or remove, or
    /// Shift+drag a box.
    pub selected: Vec<(i32, i32)>,
    /// The Shift+drag box being drawn, in block coordinates (where it began, where it is now).
    marquee: Option<(Vec2, Vec2)>,
    /// Whether a click picks a chunk: the World Explorer does, the Pruner does not.
    pub selectable: bool,
    /// Whether the flat map and Top-down may be turned (right-drag); the World Explorer's and the
    /// Pruner's stay north-up (only 3D turns).
    pub can_rotate: bool,
    /// Box Select (as MCEdit's select tool): clicks pick blocks, the first click one corner and
    /// the second the other; the box (lowest and highest corner blocks, inclusive) picks the
    /// chunks it covers. `box_anchor` is the first corner while the second is being chosen.
    pub box_mode: bool,
    pub sel_box: Option<([i32; 3], [i32; 3])>,
    box_anchor: Option<[i32; 3]>,
    /// The chunk borders drawn over the map (the game's F3+G).
    pub chunk_grid: bool,
    /// In 3D: cards of what mobs and containers hold by the pointer, and mobs' name tags.
    pub show_details: bool,
    /// The editing tools' unsaved changes (the 3D view reads through them).
    pub edits: std::sync::Arc<std::sync::RwLock<crate::edit::EditLayer>>,
    /// What Copy or Cut took, for Paste.
    pub clipboard: Option<crate::edit::tools::Clipboard>,
    /// The block Fill and Replace put in (with its properties).
    pub block: mctk_java::BlockState,

    /// Placing the clipboard: how it is turned and mirrored, where it would go (its lowest
    /// north-west corner, under the pointer), and how far it was nudged from there.
    pub paste: Option<PasteMode>,
    /// Set when the user placed the clipboard: where, and how; the page runs the paste.
    pub paste_request: Option<([i32; 3], crate::edit::transform::Placement)>,
    /// The paste preview last sent to the 3D view (where, how, which copy).
    ghost_sent: Option<u64>,
    /// Painting with the brush: its size and shape, and whether it erases.
    pub brush: Option<BrushMode>,
    /// Set when the user clicked with the brush: the block painted around; the page paints.
    pub brush_request: Option<([i32; 3], bool)>,
    /// Painting by Alt+drag: the last block painted around (one step to undo for the stroke).
    brush_last: Option<[i32; 3]>,
    /// Where the brush would paint (the block under the pointer).
    brush_at: Option<[i32; 3]>,
    /// The selection box's face being dragged: which axis, which end, where it was, where the
    /// drag began, and how far on screen one block along the axis is.
    box_drag: Option<(usize, bool, i32, Pos2, Vec2)>,
    /// The selection box's faces' handles as last drawn: axis, end, place on screen and one
    /// block's step along the axis on screen.
    handles: Vec<(usize, bool, Pos2, Vec2)>,
    /// The pointer stops at water and lava too (to select them), not only what is in them.
    pub pick_liquids: bool,
    /// Barrier blocks shown by their icon (and selectable), as the game shows them.
    pub show_barriers: bool,
    /// A mob (or item, minecart...) clicked on in 3D: selected instead of blocks.
    pub sel_entity: Option<std::sync::Arc<mctk_nbt::Tag>>,
    /// Box Select's third step: the box's floor is set, its height follows the pointer.
    box_floor: Option<([i32; 3], [i32; 3])>,
    /// Set by the selected entity's Delete: where it stands; the page removes it.
    pub entity_delete_request: Option<[f64; 3]>,
    /// The control folded out (0 search, 1 view options, 2 dimension), and whether a click
    /// pinned it open (else it folds back when the pointer leaves it).
    fold: Option<(u8, bool)>,
    /// Where the folded-out panel was last drawn (the pointer keeps it open while over it).
    fold_panel: Option<Rect>,
    /// Search's Find: blocks and mobs near the camera.
    find: Find,
    /// What the right-click menu is about (set when it opens).
    pub menu_target: Option<MenuTarget>,
    /// Chunk borders in 3D also drawn along the land (besides the box around each chunk).
    pub land_outline: bool,
    /// Set by a tool that has the pointer on one of its handles: no chunk is outlined under it.
    pub hide_hover: bool,
    /// The Biome Atlas tiles built (as `lods`).
    atlas_tiles: std::collections::HashMap<(usize, usize, usize), egui::TextureHandle>,
    last_atlas_f: usize,
    palette: atlas::Palette,
    /// The street view: how far it is tilted from top-down (0, the flat map, to 1), turned from
    /// north, tilted further by hand, how much nearer than its start the camera has come, and
    /// the height it looks at (eased toward the land under the middle).
    street: Street,
    tilt: f32,
    yaw: f32,
    /// The camera's angle above level when the user has turned it by hand (right-drag); `None`
    /// while it follows the scroll (straight down at Top-down, easing to 34° as it comes in).
    /// Kept apart from the tilt (which sets how far off the camera is), so looking up or down
    /// never moves the camera nearer or further.
    pitch_hand: Option<f32>,
    /// The Overview inset: a small north-up map around the camera showing where it is, which way
    /// it faces and the ground it sees (in the 3D views).
    overview: bool,
    /// Whether the chunk under the pointer is outlined in Top-down and 3D (the user can hide it
    /// there; back on the flat map it comes back on).
    chunk_highlight: bool,
    dolly: f32,
    look_y: Option<f32>,
    /// The 3D camera drawn with this frame (the street view, or the above view of the flat map
    /// zoomed in), and the block under the pointer in it.
    camera: Option<Camera>,
    picked: Option<Hit>,
    /// The entity under the pointer in the 3D view (its saved data), shown in a card.
    hover_entity: Option<std::sync::Arc<mctk_nbt::Tag>>,
    /// The extra tilt keeping the eye out of blocks right around it (eased).
    lift: f32,
    /// The 3D views' real blocks on the GPU (when the app draws with wgpu), how many chunks
    /// around the camera they reach, and whether they are drawn evenly lit (Full Bright) rather
    /// than by the sky's and the glowing blocks' light.
    gpu: Option<crate::gpu_world::World>,
    pub render_distance: i32,
    pub full_bright: bool,
    /// The time of day the 3D view is lit for: the world's own (as saved), day or night; and the
    /// world's saved time, once read.
    time_of_day: TimeOfDay,
    world_time: Option<Option<i64>>,
    /// Street view: the free camera (no rising over the land, no keeping out of blocks; the
    /// height looked at stays put, moved with E and Q), whether scrolling zooms the lens rather
    /// than moving the camera, and how far the lens is zoomed in (1: as normal), with the lens
    /// scroll still to be eased in.
    pub noclip: bool,
    /// How much the free camera's lens still has to zoom from the wheel (in zoom steps; out
    /// below 0).
    pending_lens: f32,
    /// How far the view has gone from the flat picture into 3D (0 to 1), eased.
    fade_3d: f32,
    /// How long (seconds) the land under the camera, and the way around its eye, have been
    /// lower than it is at (it steps down only after a moment).
    drop_wait: f32,
    lift_wait: f32,
    lens: f32,
    /// Easing back down to the Top-down view (the Top-down button), more slowly than a scroll.
    homing: bool,
    /// Scroll still to be applied (in zoom steps), eased in over the next frames, and the point
    /// zoomed around.
    pending_zoom: f32,
    zoom_anchor: Option<Pos2>,
    /// Dragging: whether the drag moves the map, how fast it is going (screen pixels a second),
    /// and the glide left after letting go.
    panning: bool,
    drag_vel: Vec2,
    momentum: Vec2,
}

/// Where the map was drawn this frame, for overlays and tools.
pub struct MapFrame {
    pub rect: Rect,
    pub center: Vec2,
    pub zoom: f32,
    pub response: Response,
    /// Block under the pointer.
    pub hover: Option<Vec2>,
    /// The street view is showing: `to_screen` and `to_block` do not apply, and overlays drawn
    /// on the flat map should not be drawn.
    pub street: bool,
    /// How far the map is turned from north-up (as the 3D camera's yaw; 0 in the Pruner).
    pub yaw: f32,
}

/// A block-space offset turned by the map's yaw into screen space (and back with `-yaw`), the
/// same way the 3D camera looking straight down turns it.
fn turn(d: Vec2, yaw: f32) -> Vec2 {
    if yaw == 0.0 {
        return d;
    }
    let (s, c) = yaw.sin_cos();
    vec2(d.x * c + d.y * s, -d.x * s + d.y * c)
}

impl MapFrame {
    pub fn to_screen(&self, b: Vec2) -> Pos2 {
        self.rect.center() + turn(b - self.center, self.yaw) * self.zoom
    }
    pub fn to_block(&self, p: Pos2) -> Vec2 {
        self.center + turn((p - self.rect.center()) / self.zoom, -self.yaw)
    }
    /// The area of the world the map shows (its lowest and highest corners in blocks): with the
    /// map turned, around the whole turned screen.
    pub fn world_bounds(&self) -> (Vec2, Vec2) {
        let r = self.rect;
        let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|p| self.to_block(p));
        let lo = pts.iter().fold(vec2(f32::MAX, f32::MAX), |m, p| m.min(*p));
        let hi = pts.iter().fold(vec2(f32::MIN, f32::MIN), |m, p| m.max(*p));
        (lo, hi)
    }
}

/// Cells across and down one zoomed-out tile.
const LOD_TILE: usize = 96;

/// One tile of the picture shrunk by `f` in both directions (tile `tx`, `ty` of `LOD_TILE` cells
/// square), each pixel standing for a block of columns: their average color and height, lit
/// again from the heights so the lie of the land stays readable (and not left to average out), as
/// a texture that is not smoothed further. Tiles are built a few at a time, nearest the view
/// first, so zooming never stalls.
fn lod_tile(ctx: &egui::Context, img: &MapImage, f: usize, tx: usize, ty: usize) -> egui::TextureHandle {
    let (nw, nh) = (img.width.div_ceil(f), img.height.div_ceil(f));
    let (cx0, cy0) = (tx * LOD_TILE, ty * LOD_TILE);
    let (cx1, cy1) = ((cx0 + LOD_TILE).min(nw), (cy0 + LOD_TILE).min(nh));
    // One cell more above and to the left, for the lighting of the first row and column.
    let (bx0, by0) = (cx0.saturating_sub(1), cy0.saturating_sub(1));
    let gw = cx1 - bx0;
    // Per cell: columns drawn, summed color, summed height, water columns and their summed depth.
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
    // A slope's step between cells grows with the cells' size; softened so the relief deepens only
    // gently going out (and the half-size shrink is not much more contrasted than the picture
    // itself, which looked like losing detail zooming in).
    let soften = 1.0 / (f as f32).sqrt();
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
            let land = world::lit(alb, hy, 0, n, wst, soften);
            // The more of the cell is water, the more it takes the color of water of that depth.
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
/// The height chunk borders rise to: the top of the world (320 in the overworld, 256 elsewhere).
fn world_top(dimension: &str) -> f32 {
    if dimension == "minecraft:overworld" { 320.0 } else { 256.0 }
}

/// The box with corners at two blocks: its lowest and highest corner blocks.
fn span(a: [i32; 3], b: [i32; 3]) -> ([i32; 3], [i32; 3]) {
    ([a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])], [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])])
}

/// "snowy_taiga" as "Snowy Taiga".
fn pretty(name: &str) -> String {
    name.trim_start_matches("minecraft:")
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Draws a picture over a part of the world (`lo` to `hi`, in blocks), turned with the map.
fn draw_turned(p: &egui::Painter, tex: egui::TextureId, frame: &MapFrame, lo: Vec2, hi: Vec2, tint: Color32) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if frame.yaw == 0.0 {
        p.image(tex, Rect::from_two_pos(frame.to_screen(lo), frame.to_screen(hi)), uv, tint);
        return;
    }
    let mut m = egui::Mesh::with_texture(tex);
    let corners = [lo, vec2(hi.x, lo.y), hi, vec2(lo.x, hi.y)];
    let uvs = [uv.left_top(), uv.right_top(), uv.right_bottom(), uv.left_bottom()];
    for k in 0..4 {
        m.vertices.push(egui::epaint::Vertex { pos: frame.to_screen(corners[k]), uv: uvs[k], color: tint });
    }
    m.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    p.add(egui::Shape::mesh(m));
}

/// The tiles of `tile` cells (each `f` columns square) over the part of the picture from `lo` to
/// `hi` (in its pixels).
fn tiles_in_view(img: &MapImage, lo: Vec2, hi: Vec2, f: usize, tile: usize) -> Vec<(usize, usize)> {
    let span = (tile * f) as f32;
    let (nx, ny) = (img.width.div_ceil(tile * f), img.height.div_ceil(tile * f));
    let range = |a: f32, b: f32, n: usize| ((a / span).floor().max(0.0) as usize)..=((b / span).floor().max(0.0) as usize).min(n.saturating_sub(1));
    let mut v = Vec::new();
    for ty in range(lo.y, hi.y, ny) {
        for tx in range(lo.x, hi.x, nx) {
            v.push((tx, ty));
        }
    }
    v
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
            can_rotate: true,
            box_mode: false,
            sel_box: None,
            box_anchor: None,
            chunk_grid: false,
            show_details: true,
            edits: Default::default(),
            clipboard: None,
            block: mctk_java::BlockState::new("minecraft:stone"),
            paste: None,
            paste_request: None,
            ghost_sent: None,
            brush: None,
            brush_request: None,
            brush_last: None,
            brush_at: None,
            box_drag: None,
            handles: Vec::new(),
            pick_liquids: false,
            show_barriers: true,
            sel_entity: None,
            box_floor: None,
            entity_delete_request: None,
            fold: None,
            fold_panel: None,
            find: Find::default(),
            menu_target: None,
            land_outline: false,
            hide_hover: false,
            atlas_tiles: Default::default(),
            last_atlas_f: 0,
            palette: Default::default(),
            street: Street::default(),
            tilt: 0.0,
            yaw: 0.0,
            pitch_hand: None,
            overview: false,
            chunk_highlight: true,
            dolly: 1.0,
            look_y: None,
            camera: None,
            picked: None,
            hover_entity: None,
            lift: 0.0,
            gpu: None,
            render_distance: crate::gpu_world::RADIUS,
            full_bright: false,
            time_of_day: TimeOfDay::World,
            world_time: None,
            noclip: false,
            pending_lens: 0.0,
            fade_3d: 0.0,
            drop_wait: 0.0,
            lift_wait: 0.0,
            lens: 1.0,
            homing: false,
            pending_zoom: 0.0,
            zoom_anchor: None,
            panning: false,
            drag_vel: Vec2::ZERO,
            momentum: Vec2::ZERO,
        }
    }

    /// For development: puts the camera where a preset says, e.g.
    /// `x=100;z=-40;zoom=0.2;style=biomes` or `x=0;z=0;tilt=0.8;yaw=0.6;dolly=0.7`.
    pub fn dev_preset(&mut self, preset: &str) {
        // (`pitch` is how far the hand turned from the scroll's angle, applied after the tilt.)
        let mut hand = None;
        for part in preset.split(';') {
            let Some((k, v)) = part.split_once('=') else { continue };
            let n = v.trim().parse::<f32>().ok();
            match (k.trim(), n) {
                ("x", Some(n)) => self.center.x = n,
                ("z", Some(n)) => self.center.y = n,
                ("zoom", Some(n)) => self.zoom = n.clamp(0.03, TOP_MAX),
                ("tilt", Some(n)) => self.tilt = n.clamp(0.0, 1.0),
                ("yaw", Some(n)) => self.yaw = n,
                ("pitch", Some(n)) => hand = Some(n),
                ("dolly", Some(n)) => self.dolly = n.clamp(0.05, TOP_MAX / ABOVE_FROM),
                ("lens", Some(n)) => self.lens = n.clamp(1.0, 12.0),
                ("bright", Some(n)) => self.full_bright = n > 0.5,
                ("night", Some(n)) => self.time_of_day = if n > 1.5 { TimeOfDay::Sunset } else if n > 0.5 { TimeOfDay::Night } else { TimeOfDay::Day },
                ("slicer", Some(n)) => self.slicer_open = n > 0.5,
                ("noclip", Some(n)) => self.noclip = n > 0.5,
                ("overview", Some(n)) => self.overview = n > 0.5,
                ("distance", Some(n)) => self.render_distance = (n as i32).clamp(2, 64),
                ("y", Some(n)) => self.look_y = Some(n),
                _ => {}
            }
        }
        self.pitch_hand = hand.map(|n| self.base_pitch() + n);
        self.fitted = true;
        self.pending_zoom = 0.0;
        self.pending_lens = 0.0;
        self.momentum = Vec2::ZERO;
        self.lift = 0.0;
        // Put straight there, without the fade between the flat map and 3D.
        self.fade_3d = if self.tilt > 0.0 || self.zoom >= ABOVE_FROM { 1.0 } else { 0.0 };
    }

    /// The picture is drawn and no render is running.
    #[cfg(test)]
    pub(crate) fn ready(&self) -> bool {
        self.image.is_some() && self.job.is_none()
    }

    /// How far the street view is tilted (0: the flat map), and how near the camera has come.
    #[cfg(test)]
    pub(crate) fn street_state(&self) -> (f32, f32) {
        (self.tilt, self.dolly)
    }

    /// How far the 3D camera drawn last is from what it looks at.
    #[cfg(test)]
    pub(crate) fn camera_dist(&self) -> Option<f32> {
        self.camera.map(|c| c.dist)
    }

    /// The 3D camera's field of view (narrower with the lens zoomed in).
    #[cfg(test)]
    pub(crate) fn camera_fov(&self) -> Option<f32> {
        self.camera.map(|c| c.fov)
    }

    /// Draws the map again (as a Slicer change does).
    #[cfg(test)]
    pub(crate) fn redraw(&mut self) {
        self.invalidate();
    }

    /// The way the map or camera faces.
    #[cfg(test)]
    pub(crate) fn yaw(&self) -> f32 {
        self.yaw
    }

    /// What the 3D camera looks at.
    #[cfg(test)]
    pub(crate) fn camera_target(&self) -> Option<[f32; 3]> {
        self.camera.map(|c| c.target)
    }

    /// Where the 3D camera's eye is.
    #[cfg(test)]
    pub(crate) fn camera_eye(&self) -> Option<[f32; 3]> {
        self.camera.map(|c| c.eye())
    }

    /// The 3D camera's tilt down from level.
    #[cfg(test)]
    pub(crate) fn camera_pitch(&self) -> Option<f32> {
        self.camera.map(|c| c.pitch)
    }

    /// Where a point of the world is on screen in the 3D view drawn last.
    #[cfg(test)]
    pub(crate) fn project(&self, p: [f32; 3]) -> Option<Pos2> {
        self.camera.and_then(|c| c.project(p)).map(|(s, _)| s)
    }

    /// The block under the pointer in the 3D view.
    #[cfg(test)]
    pub(crate) fn picked(&self) -> Option<Hit> {
        self.picked
    }

    /// How bright the sky's light is for the time of day chosen (1 by day).
    /// The time of day the 3D view is for, in ticks (0 sunrise, 6000 noon, 18000 midnight).
    fn day_ticks(&mut self) -> i64 {
        match self.time_of_day {
            TimeOfDay::Day => 6000,
            TimeOfDay::Sunset => 11500,
            TimeOfDay::Night => 18000,
            TimeOfDay::World => {
                let (path, edition) = (self.path.clone(), self.edition);
                self.world_time.get_or_insert_with(|| crate::gpu_world::world_time(&path, edition)).unwrap_or(6000).rem_euclid(24000)
            }
        }
    }

    fn daylight(&mut self) -> f32 {
        match self.time_of_day {
            TimeOfDay::Day => 1.0,
            TimeOfDay::Sunset => crate::gpu_world::daylight(11500),
            TimeOfDay::Night => crate::gpu_world::daylight(18000),
            TimeOfDay::World => {
                let (path, edition) = (self.path.clone(), self.edition);
                let t = *self.world_time.get_or_insert_with(|| crate::gpu_world::world_time(&path, edition));
                crate::gpu_world::daylight(t.unwrap_or(6000))
            }
        }
    }

    /// How far the flat map and the Top-down view are turned (north-up where they may not turn).
    fn map_yaw(&self) -> f32 {
        if self.can_rotate { self.yaw } else { 0.0 }
    }

    /// Back to the flat map, facing north.
    /// (Back at the zoom the 3D view's distance matches: turning into 3D from the Top-down view
    /// zoomed out a little comes back there.)
    fn leave_street(&mut self) {
        if self.tilt > 0.0 {
            self.zoom = (TOP_MAX / self.dolly.max(1.0)).clamp(ABOVE_FROM, TOP_MAX);
        }
        self.tilt = 0.0;
        // The map keeps the way the camera faced (where it may turn).
        if !self.can_rotate {
            self.yaw = 0.0;
        }
        self.pitch_hand = None;
        self.dolly = 1.0;
        self.camera = None;
        self.lift = 0.0;
        self.lens = 1.0;
        // The height looked at follows the land again (the free camera may have flown off).
        self.look_y = None;
        // The free camera is for 3D: back on the map it is off.
        self.noclip = false;
        self.pending_lens = 0.0;
    }

    /// The street view's camera for the current tilt, turn and nearness.
    fn street_camera(&self, rect: Rect, look_y: f32) -> Camera {
        let e = self.tilt * self.tilt * (3.0 - 2.0 * self.tilt);
        let deg = std::f32::consts::PI / 180.0;
        // The free camera may look up as well; the usual camera stays looking down at the land.
        let lowest = if self.noclip { -80.0 } else { 8.0 };
        let pitch = self.pitch_hand.unwrap_or_else(|| self.base_pitch()).clamp(lowest * deg, 90.0 * deg);
        let dist = Camera::dist_for_zoom(rect, TOP_MAX) * (1.0 - 0.5 * e) * self.dolly;
        Camera { target: [self.center.x, look_y, self.center.y], yaw: self.yaw, pitch, dist, rect, fov: FOV_Y / self.lens }
    }

    /// The camera's tilt for the street view's tilt alone (before any turned by hand): straight
    /// down at the start (where it is the above view), about a third of the way above level when
    /// fully tilted.
    fn base_pitch(&self) -> f32 {
        let e = self.tilt * self.tilt * (3.0 - 2.0 * self.tilt);
        (90.0 + (34.0 - 90.0) * e) * std::f32::consts::PI / 180.0
    }

    /// The above view: the flat map zoomed in, drawn in 3D straight down, at the same scale as the
    /// flat map at the height looked at.
    fn above_camera(&self, rect: Rect, look_y: f32) -> Camera {
        Camera {
            target: [self.center.x, look_y, self.center.y],
            yaw: self.map_yaw(),
            pitch: std::f32::consts::FRAC_PI_2,
            dist: Camera::dist_for_zoom(rect, self.zoom),
            rect,
            fov: FOV_Y,
        }
    }

    /// The height to look at: the land under the middle of the view, eased toward so the camera
    /// does not jump at every hill (and never below the column right under the middle).
    fn ease_look_y(&mut self, ctx: &egui::Context) -> f32 {
        // The free camera keeps its height (Space and Ctrl raise and lower it) instead of
        // following the land.
        if self.noclip && self.tilt > 0.0 {
            if let Some(y) = self.look_y {
                return y;
            }
        }
        let Some((img, _)) = &self.image else { return 64.0 };
        let median = self.street.ground_near(img, self.center.x, self.center.y).unwrap_or(64.0);
        let here = self.street.surface_at(img, self.center.x, self.center.y).unwrap_or(median);
        let want = median.max(here);
        // Up quickly (so the camera never sinks into a hill it moves onto); down only once the
        // land has stayed lower a moment, and gently (crossing a gap a block or two wide, it
        // does not dip down and back up).
        let dt = ctx.input(|i| i.stable_dt).clamp(1.0 / 240.0, 0.1);
        let lower = self.look_y.is_some_and(|y| want < y - 0.05);
        self.drop_wait = if lower { self.drop_wait + dt } else { 0.0 };
        if lower && self.drop_wait < 0.5 {
            ctx.request_repaint();
            return self.look_y.unwrap_or(want);
        }
        let y = self.look_y.map_or(want, |y| y + (want - y) * if want > y { 0.2 } else { 0.05 });
        // Settles exactly, so a still camera stays the same and its land is not built again.
        let y = if (want - y).abs() > 0.05 {
            ctx.request_repaint();
            y
        } else {
            want
        };
        self.look_y = Some(y);
        y
    }

    /// The least `dolly` may be: the camera never comes nearer than a few blocks.
    /// (The free camera may come to within a block of what it looks at.)
    fn min_dolly(&self, rect: Rect) -> f32 {
        let nearest = if self.noclip { 1.0 } else { USUAL_NEAREST };
        (nearest / (Camera::dist_for_zoom(rect, TOP_MAX) * 0.5)).min(1.0)
    }

    /// Re-renders (after the world changed on disk).
    pub fn reload(&mut self, path: PathBuf) {
        self.path = path;
        self.invalidate();
    }

    /// After chunks (by chunk x, z) were saved with changes: the 3D view reads them again.
    pub fn chunks_changed(&mut self, chunks: &[(i32, i32)]) {
        if let Some(g) = &mut self.gpu {
            g.reload_chunks(chunks);
        }
    }

    /// Whether the blocks are shown (Top-down zoomed in, or 3D), not the flat map.
    pub fn shows_blocks(&self) -> bool {
        self.camera.is_some()
    }

    /// Where Box Select is in making a box, for the toolbar to say what to do next.
    pub fn box_hint(&self) -> Option<&'static str> {
        let in_3d = self.camera.is_some() && self.tilt > 0.0;
        match (self.box_anchor.is_some(), self.box_floor.is_some()) {
            (true, _) if in_3d => Some("Move to set the width and length, then click"),
            (true, _) => Some("Click the other corner"),
            (_, true) => Some("Move up or down for the height, then click"),
            _ if self.box_mode && self.sel_box.is_none() => Some(if in_3d { "Click the first block of the box" } else { "Click a corner (or Shift+drag an area)" }),
            _ => None,
        }
    }

    /// The selection box changed from outside (its height set in the toolbar, cleared): the
    /// chunks picked follow it, and a box half made is let go.
    pub fn box_changed(&mut self) {
        self.box_anchor = None;
        self.box_floor = None;
        self.sync_box_chunks();
    }

    /// What the pointer is on in the blocks' views (a mob before the block behind it), for the
    /// right-click menu.
    pub fn pointed(&self) -> Option<MenuTarget> {
        if let Some(e) = &self.hover_entity {
            let l = e.get("Pos").and_then(mctk_nbt::Tag::as_list)?;
            let n = |i: usize| l.get(i).and_then(mctk_nbt::Tag::as_f64);
            return Some(MenuTarget::Entity(e.clone(), [n(0)?, n(1)?, n(2)?]));
        }
        let h = self.picked?;
        let id = self.gpu.as_ref()?.block_at(h.x, h.y, h.z)?;
        let full = world::block_name(id)?;
        let name = full.split(['#', '@']).next().unwrap_or(&full).to_string();
        let props = world::block_props(id).map(|p| p.to_string()).unwrap_or_default();
        Some(MenuTarget::Block([h.x, h.y, h.z], name, props))
    }

    /// The block at a place, as the 3D view has it (`None` where it is not loaded).
    pub fn block_name_at(&self, p: [i32; 3]) -> Option<String> {
        let id = self.gpu.as_ref()?.block_at(p[0], p[1], p[2])?;
        Some(world::block_name(id).map(|n| pretty(n.split(['#', '@']).next().unwrap_or(&n))).unwrap_or_else(|| "Air".into()))
    }

    /// Search's Find, under the Locate bar: a name, what to look for, how far from the camera,
    /// then the matches nearest first (a click goes there and selects it). Gives the rect drawn.
    fn find_panel(&mut self, ui: &mut Ui, at: Pos2, width: f32, ks: f32) -> Rect {
        use std::sync::atomic::Ordering;
        let running = self.find.job.is_some();
        if let Some(r) = self.find.job.as_ref().and_then(Job::take) {
            self.find.job = None;
            match r {
                Ok(found) => {
                    self.find.note = Some(match found.len() {
                        0 => "Nothing found in range".into(),
                        n if n >= 500 => "The nearest 500 shown".into(),
                        1 => "1 found".into(),
                        n => format!("{n} found"),
                    });
                    self.find.results = found;
                }
                Err(e) => self.find.note = Some(e),
            }
        } else if running {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        let bg = ui.painter().add(egui::Shape::Noop);
        let pad = 10.0 * ks;
        let inner = Rect::from_min_size(at + vec2(pad, pad), vec2(width - 2.0 * pad, 2000.0));
        let mut start = std::mem::take(&mut self.find.start_soon);
        let mut goto: Option<crate::gpu_world::Found> = None;
        let used = ui
            .scope_builder(UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                ui.style_mut().override_font_id = Some(theme::regular(14.0 * ks));
                ui.spacing_mut().item_spacing = vec2(6.0 * ks, 6.0 * ks);
                ui.spacing_mut().interact_size.y = 24.0 * ks;
                // What to find, from the item list: a block, or a spawn egg for its mob.
                let (r, pick) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0 * ks), Sense::click());
                ui.painter().rect_filled(r, theme::radius(4), if pick.hovered() { theme::button_hover() } else { theme::button() });
                let icon = Rect::from_center_size(pos2(r.left() + 18.0 * ks, r.center().y), Vec2::splat(22.0 * ks));
                let (label, color) = if self.find.query.is_empty() {
                    ("Choose a block or mob to find\u{2026}".to_string(), theme::text_muted())
                } else {
                    let id = self.find.query.as_str();
                    crate::icons::item_icon(ui.painter(), icon, id, Color32::WHITE);
                    let name = crate::item_picker::display_name(id.strip_suffix("_spawn_egg").unwrap_or(id));
                    (name, theme::text())
                };
                ui.painter().text(pos2(r.left() + 36.0 * ks, r.center().y), Align2::LEFT_CENTER, label, theme::regular(14.0 * ks), color);
                let pick = pick.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Pick a block, or a mob's spawn egg");
                let mut picked = None;
                crate::ctx_menu::dropdown(ui, &pick, |ui, _| {
                    picked = crate::item_picker::show(ui, &self.find.query, false);
                });
                // (The panel stays open while its list is.)
                self.find.typing = crate::ctx_menu::is_open_for(ui.ctx(), &pick);
                if let Some(id) = picked {
                    self.find.query = id;
                    start = true;
                }
                ui.horizontal(|ui| {
                    ui.label("Within");
                    ui.add(egui::DragValue::new(&mut self.find.radius).range(16.0..=512.0).speed(2.0).suffix(" blocks"))
                        .on_hover_text("How far from the camera to look (16 to 512 blocks)");
                });
                ui.horizontal(|ui| {
                    if running {
                        let total = self.find.total.load(Ordering::Relaxed).max(1);
                        let done = self.find.done.load(Ordering::Relaxed);
                        if ui.button("Stop").clicked() {
                            self.find.stop.store(true, Ordering::Relaxed);
                        }
                        ui.add(egui::ProgressBar::new(done as f32 / total as f32).desired_width(ui.available_width()).text(format!("{done} of {total} chunks")));
                    } else {
                        let can = !self.find.query.trim().is_empty();
                        if ui.add_enabled(can, egui::Button::new("\u{1F50D} Find")).clicked() {
                            start = true;
                        }
                        if let Some(note) = &self.find.note {
                            ui.label(egui::RichText::new(note).color(theme::text_dim()));
                        }
                        if !self.find.results.is_empty() && ui.small_button("Clear").clicked() {
                            self.find.results.clear();
                            self.find.note = None;
                        }
                    }
                });
                if !self.find.results.is_empty() {
                    egui::ScrollArea::vertical().max_height(220.0 * ks).auto_shrink([false, true]).show(ui, |ui| {
                        for f in &self.find.results {
                            let mut text = egui::text::LayoutJob::default();
                            let font = theme::regular(14.0 * ks);
                            text.append(&f.name, 0.0, egui::TextFormat::simple(font.clone(), theme::text()));
                            let at = format!("   {} {} {}   {:.0} m", f.pos[0].floor(), f.pos[1].floor(), f.pos[2].floor(), f.dist);
                            text.append(&at, 0.0, egui::TextFormat::simple(font, theme::text_muted()));
                            let r = ui
                                .add(egui::Button::new(text).frame(false).wrap_mode(egui::TextWrapMode::Truncate))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text(if f.entity.is_some() { "Go to it and select it" } else { "Go to it and select the block" });
                            if r.clicked() {
                                goto = Some(f.clone());
                            }
                        }
                    });
                }
            })
            .response
            .rect;
        let rect = Rect::from_min_max(at, pos2(at.x + width, used.bottom() + pad));
        ui.painter().set(bg, egui::Shape::rect_filled(rect, theme::radius(5), theme::deep()));
        if start {
            self.start_find(ui.ctx());
        }
        if let Some(f) = goto {
            self.go_to_found(&f);
        }
        rect
    }

    /// Starts Find around the camera (in 3D: the place it looks at; on the flat map: the middle,
    /// counting only the distance across).
    fn start_find(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        if self.find.query.trim().is_empty() {
            return;
        }
        if let Some(_old) = self.find.job.take() {
            self.find.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let center = match self.camera {
            Some(c) => c.target,
            None => [self.center.x, f32::NAN, self.center.y],
        };
        let (done, total, stop) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
        self.find.done = done.clone();
        self.find.total = total.clone();
        self.find.stop = stop.clone();
        self.find.results.clear();
        self.find.note = None;
        let (path, edition, dim, edits) = (self.path.clone(), self.edition, self.dimension.clone(), self.edits.clone());
        // A spawn egg finds its mob; anything else the block (exactly that one).
        let id = self.find.query.clone();
        let (query, mobs) = match id.strip_suffix("_spawn_egg") {
            Some(mob) => (mob.to_string(), true),
            None => (id.clone(), false),
        };
        let query = if query.contains(':') { query } else { format!("minecraft:{query}") };
        let radius = self.find.radius;
        self.find.job = Some(Job::spawn("Finding", ctx, move || {
            crate::gpu_world::search(&path, edition, &dim, &edits, center, radius, &query, !mobs, mobs, &done, &total, &stop)
        }));
    }

    /// Goes to something Find found and selects it (a block as a one-block box; a mob as the
    /// selected mob).
    fn go_to_found(&mut self, f: &crate::gpu_world::Found) {
        let block = f.entity.is_none();
        let half = if block { 0.5 } else { 0.0 };
        self.center = vec2(f.pos[0] as f32 + half, f.pos[2] as f32 + half);
        self.momentum = Vec2::ZERO;
        self.pending_zoom = 0.0;
        if self.noclip {
            self.look_y = Some(f.pos[1] as f32 + half);
        }
        if self.camera.is_none() {
            self.zoom = self.zoom.max(4.0);
        }
        match &f.entity {
            Some(e) => {
                self.sel_entity = Some(e.clone());
                self.sel_box = None;
            }
            None => {
                let p = [f.pos[0] as i32, f.pos[1] as i32, f.pos[2] as i32];
                self.sel_entity = None;
                self.sel_box = Some((p, p));
            }
        }
        self.box_changed();
    }

    /// For development: folds out a control (0 search, 1 view options, 2 dimension), pinned.
    pub fn dev_fold(&mut self, which: u8) {
        self.fold = Some((which, true));
    }

    /// For development: Search folded out, finding `query` (blocks and mobs).
    pub fn dev_find(&mut self, query: &str) {
        self.fold = Some((0, true));
        self.find.query = query.to_string();
        self.find.start_soon = true;
    }

    /// An icon on the view that folds out its panel: hovering opens it, and it folds back once
    /// the pointer leaves the icon and the panel (not while its text box is typed in). Gives
    /// whether it is open. The caller paints the icon on top. (Pinned open only for development.)
    fn fold_button(&mut self, ui: &Ui, which: u8, icon: Rect, tip: &str, ks: f32) -> bool {
        // Only hovered: it just shows its panel, so there is nothing to click.
        let resp = ui.interact(icon, ui.id().with(("fold", which)), Sense::hover()).on_hover_text(tip);
        let mine = |f: Option<(u8, bool)>| f.is_some_and(|(w, _)| w == which);
        if resp.hovered() && !self.fold.is_some_and(|(_, pinned)| pinned) && !mine(self.fold) {
            self.fold = Some((which, false));
            self.fold_panel = None;
        }
        if let Some((w, pinned)) = self.fold {
            if w == which {
                let pointer = ui.input(|i| i.pointer.hover_pos());
                let near = pointer.is_some_and(|p| icon.expand(8.0 * ks).contains(p) || self.fold_panel.is_some_and(|r| r.expand(12.0 * ks).contains(p)));
                let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                // (Not while typing in it: the pointer may wander off.)
                let typing = which == 0 && self.find.typing;
                if (!pinned && !near && pointer.is_some() && !typing) || (pinned && esc) {
                    self.fold = None;
                }
            }
        }
        let open = mine(self.fold);
        ui.painter().rect_filled(icon, theme::radius(5), if open || resp.hovered() { theme::button_hover() } else { theme::deep() });
        if open {
            ui.painter().rect_stroke(icon, theme::radius(5), Stroke::new(1.2_f32 * ks, theme::accent()), StrokeKind::Inside);
        }
        open
    }

    /// The paste preview in 3D: the copy's blocks, see-through, where a click would put them;
    /// sent again only when that changes, and taken away when pasting stops.
    fn sync_ghost(&mut self) {
        use std::hash::{Hash, Hasher};
        // What is shown: the copy being placed, or the brush's shape (filled with its block;
        // red glass where it erases).
        let paste = match (&self.paste, &self.clipboard) {
            (Some(p), Some(c)) => p.at.map(|at| (at, p.place, c)),
            _ => None,
        };
        let brush = match (&self.brush, self.brush_at, paste.is_none()) {
            (Some(b), Some(at), true) => Some((b.clone(), at)),
            _ => None,
        };
        let key = if self.camera.is_none() {
            None
        } else if let Some((at, place, c)) = &paste {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (0u8, at, place.turns, place.mirror, c.blocks.as_ptr() as usize, c.size).hash(&mut h);
            Some(h.finish())
        } else if let Some((b, at)) = &brush {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (1u8, at, b.radius, b.sphere, b.erase, self.block.to_string()).hash(&mut h);
            Some(h.finish())
        } else {
            None
        };
        if key == self.ghost_sent {
            return;
        }
        let Some(g) = &mut self.gpu else { return };
        let chunks = match (key, paste, brush) {
            (None, ..) => Vec::new(),
            (Some(_), Some((at, place, c)), _) => crate::gpu_world::ghost_chunks(c, at, place),
            (Some(_), None, Some((b, at))) => {
                let state = if b.erase { mctk_java::BlockState::new("minecraft:red_stained_glass") } else { self.block.clone() };
                let shape = crate::edit::tools::brush_shape(b.radius, b.sphere, &state);
                let r = b.radius.max(0);
                crate::gpu_world::ghost_chunks(&shape, [at[0] - r, at[1] - r, at[2] - r], Default::default())
            }
            _ => Vec::new(),
        };
        g.set_ghost(chunks);
        self.ghost_sent = key;
    }

    /// After the editing tools (or undo) changed chunks in memory: the 3D view draws them again
    /// as they are now.
    pub fn edits_changed(&mut self, chunks: &[(i32, i32)]) {
        if let Some(g) = &mut self.gpu {
            g.refresh_chunks(chunks);
        }
    }

    /// Drops the picture and stops any render in flight, so the next frame starts a new one.
    fn invalidate(&mut self) {
        if let Some(p) = self.progress.take() {
            p.cancel();
        }
        self.image = None;
        self.lods.clear();
        self.atlas_tiles.clear();
        self.palette = Default::default();
        self.street = Street::default();
        self.gpu = None;
        self.job = None;
        self.error = None;
        self.ghost_sent = None;
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
            let img = MapImage::blank(min_x, min_z, width, height);
            let color = egui::ColorImage::from_rgba_unmultiplied([width, height], &img.rgba);
            let tex = ctx.load_texture("world-map", color, egui::TextureOptions::NEAREST);
            self.image = Some((img, tex));
        }
        let Some((img, tex)) = &mut self.image else {
            return;
        };
        for (cx, cz, p) in &fresh {
            img.put(*cx, *cz, p);
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
                    ui.label(RichText::new(headline).size(16.0).color(theme::text()));
                    ui.add_space(8.0);
                    // Until the chunk count is known, a bar that keeps filling to show activity.
                    let shown = fraction.unwrap_or_else(|| (elapsed * 0.6).fract());
                    widgets::progress_bar(ui, shown, card.x - 40.0);
                    ui.add_space(8.0);
                    ui.label(RichText::new(detail).size(13.0).color(theme::text_muted()));
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
        painter.rect_filled(outer, 0.0, theme::bg());

        // Zoomed in (and once the whole picture is drawn) the map is drawn in 3D: straight down
        // (the above view) or tilted (the street view).
        let wants_3d = self.image.is_some() && self.job.is_none() && (self.tilt > 0.0 || self.zoom >= ABOVE_FROM);
        // Eased between the flat picture and the 3D view (a third of a second), so neither way
        // is a cut: the 3D view stays drawn a moment going out, the picture over it fading in.
        {
            let target = if wants_3d { 1.0 } else { 0.0 };
            let dt = ui.input(|i| i.stable_dt).clamp(1.0 / 240.0, 0.1);
            if self.image.is_none() || self.job.is_some() || self.tilt > 0.0 {
                self.fade_3d = target;
            } else if self.fade_3d != target {
                let step = dt / 0.3;
                self.fade_3d = if target > self.fade_3d { (self.fade_3d + step).min(1.0) } else { (self.fade_3d - step).max(0.0) };
                ctx.request_repaint();
            }
        }
        let three_d = wants_3d || self.fade_3d > 0.0;
        let look = if three_d {
            Some(self.ease_look_y(&ctx))
        } else {
            // Kept while the 3D view waits for the map to be drawn again (the free camera's
            // height with it).
            if self.tilt <= 0.0 {
                self.look_y = None;
            }
            None
        };
        if !three_d {
            self.camera = None;
        }
        // How bright the sky's light is for the time of day chosen.
        let daylight = if three_d { self.daylight() } else { 1.0 };
        let (ticks, secs) = (if three_d { self.day_ticks() } else { 6000 }, ctx.input(|i| i.time));
        // Drawing with wgpu, the 3D views are drawn on the GPU from the real blocks.
        if three_d && crate::gpu_world::ready(&ctx).is_some() {
            let fits = self.gpu.as_ref().is_some_and(|w| w.is_for(&self.path, &self.dimension, self.applied_slicer));
            if !fits {
                self.gpu = Some(crate::gpu_world::World::new(self.path.clone(), self.edition, &self.dimension, self.applied_slicer, self.edits.clone()));
                self.ghost_sent = None;
            }
        }

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
            let street = self.tilt > 0.0 && self.job.is_none();
            if street {
                let mut cam = self.street_camera(rect, look.unwrap_or(64.0));
                // Walls and hills in the way never clip the view: the whole camera rises straight
                // up over them (its angle and distance stay as the user set them; it never pulls
                // in or tilts by itself). Not the free camera, which goes where it is put.
                let want = if self.noclip { 0.0 } else { self.street.clear_lift(img, &cam) };
                // Up out of blocks at once; down again eased, once the way has stayed clear a
                // moment (so passing over a lone block or gap does not bob the camera).
                let dt = ctx.input(|i| i.stable_dt).clamp(1.0 / 240.0, 0.1);
                let lower = want < self.lift - 0.02;
                self.lift_wait = if lower { self.lift_wait + dt } else { 0.0 };
                self.lift = if want > self.lift || (want - self.lift).abs() <= 0.02 {
                    want
                } else if self.lift_wait < 0.5 {
                    self.lift
                } else {
                    self.lift + (want - self.lift) * 0.08
                };
                if self.lift != want {
                    ctx.request_repaint();
                }
                cam.target[1] += self.lift;
                self.camera = Some(cam);
                let clip = painter.with_clip_rect(rect);
                match self.gpu.as_mut() {
                    // On the GPU: the sky, then the chunks around the camera as real blocks.
                    Some(w) => {
                        if cam.pitch < 80f32.to_radians() {
                            crate::street_view::sky(&clip, &cam, daylight, ticks, &self.dimension, secs);
                            // (The clouds drift.)
                            if self.dimension == DIMS[0] {
                                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                            }
                        }
                        w.update(&ctx, (cam.target[0], cam.target[2]), self.render_distance);
                        w.update_lod((cam.target[0], cam.target[2]), self.render_distance);
                        if crate::gpu_world::World::can_paint(&cam) {
                            clip.add(egui::Shape::Callback(w.paint(&cam, ctx.input(|i| i.time), self.full_bright, daylight)));
                        }
                        ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    }
                    None => self.street.draw(&ctx, &clip, img, &cam),
                }
            } else {
            let frame = MapFrame {
                rect,
                center: self.center,
                zoom: self.zoom,
                response: response.clone(),
                hover: None,
                street: false,
                yaw: self.map_yaw(),
            };
            // Far out the Biome Atlas, fading into the shaded terrain closer in.
            let atlas_w = ((ATLAS_END - self.zoom) / (ATLAS_END - ATLAS_FULL)).clamp(0.0, 1.0);
            // Where the picture's first pixel is in the world, and the part of the picture in view
            // (in its pixels; around the whole screen when the map is turned).
            let org = vec2(img.min_x as f32 * 16.0, img.min_z as f32 * 16.0);
            let (wlo, whi) = frame.world_bounds();
            let (plo, phi) = (wlo - org, whi - org);
            let clip = painter.with_clip_rect(rect);
            // The picture itself underneath, for whatever the zoomed-out tiles have not reached yet.
            // (Not under the 3D view once it is fully in: where nothing is below the blocks — the
            // void, a sky island — the map's squares showed through.)
            let under_3d = look.is_some() && self.fade_3d >= 1.0 && self.gpu.is_some();
            if !under_3d {
                draw_turned(&clip, tex.id(), &frame, org, org + vec2(img.width as f32, img.height as f32), Color32::WHITE);
            }
            let zoom = self.zoom;
            // The spot (in picture pixels) tiles are built outward from: under the pointer if it
            // is on the map, else the middle.
            let focus = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)).unwrap_or(rect.center());
            let focus = frame.to_block(focus) - org;
            // A tile's place in the world: `tile` cells across of `f` columns each.
            let tile_at = |tx: usize, ty: usize, f: usize, tile: usize, size: [usize; 2]| {
                let a = org + vec2((tx * tile * f) as f32, (ty * tile * f) as f32);
                (a, a + vec2((size[0] * f) as f32, (size[1] * f) as f32))
            };
            // (Made already while the biomes are fully shown, once the fade into terrain is near:
            // started only as the fade began, the unshrunk picture showed through meanwhile and
            // shimmered.)
            if self.zoom < 1.0 && self.job.is_none() && (atlas_w < 1.0 || self.zoom > ATLAS_FULL * 0.6) {
                // The shrink whose pixels come nearest to (and not under) one screen pixel, shaded
                // for the lie of the land at every size (far out, Auto shows the Biome Atlas).
                const STEPS: [usize; 15] = [2, 3, 4, 5, 6, 8, 10, 12, 16, 24, 32, 48, 64, 128, 256];
                let f = STEPS.iter().copied().find(|f| *f as f32 * self.zoom >= 1.0).unwrap_or(256);
                let prev = self.last_f;
                // Other shrinks are kept (drawn under this one while it is made, so zooming fast
                // never leaves a gap) until there are too many.
                if self.lods.len() > 600 {
                    self.lods.retain(|(ff, _, _), _| *ff == f || *ff == prev);
                }
                let visible = |f: usize| tiles_in_view(img, plo, phi, f, LOD_TILE);
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
                // The previous shrink's tiles (still held) first, then this one's over them; fading
                // into the full picture (underneath) as the zoom nears a block a pixel.
                let fade = ((1.0 - self.zoom) / 0.3).clamp(0.0, 1.0);
                let tint = Color32::from_white_alpha((fade * 255.0) as u8);
                // Every shrink at hand, coarsest first, this one last (on top).
                let mut levels: Vec<usize> = self.lods.keys().map(|k| k.0).filter(|ff| *ff != f).collect();
                levels.sort_unstable_by(|a, b| b.cmp(a));
                levels.dedup();
                levels.push(f);
                // (Once this shrink is all made, it alone: the faded layers would add up otherwise.)
                if left == 0 {
                    levels = vec![f];
                }
                let _ = prev;
                for ff in levels {
                    if ff == 0 {
                        continue;
                    }
                    for (tx, ty) in visible(ff) {
                        if let Some(t) = self.lods.get(&(ff, tx, ty)) {
                            let (a, b) = tile_at(tx, ty, ff, LOD_TILE, t.size());
                            draw_turned(&clip, t.id(), &frame, a, b, tint);
                        }
                    }
                }
                if left == 0 {
                    self.last_f = f;
                }
            }
            // The Biome Atlas over the terrain, fading in as the map is zoomed out.
            if atlas_w > 0.0 && self.job.is_none() {
                const STEPS: [usize; 15] = [1, 2, 3, 4, 5, 6, 8, 10, 12, 16, 24, 32, 48, 64, 128];
                let f = STEPS.iter().copied().find(|f| *f as f32 * zoom >= 1.0).unwrap_or(256);
                let prev = self.last_atlas_f;
                if self.atlas_tiles.len() > 600 {
                    self.atlas_tiles.retain(|(ff, _, _), _| *ff == f || *ff == prev);
                }
                let span = (atlas::TILE * f) as f32;
                let mut missing: Vec<_> = tiles_in_view(img, plo, phi, f, atlas::TILE)
                    .into_iter()
                    .filter(|(tx, ty)| !self.atlas_tiles.contains_key(&(f, *tx, *ty)))
                    .collect();
                missing.sort_by(|a, b| {
                    let d = |t: &(usize, usize)| (vec2((t.0 as f32 + 0.5) * span, (t.1 as f32 + 0.5) * span) - focus).length_sq();
                    d(a).total_cmp(&d(b))
                });
                let started = Instant::now();
                let mut left = missing.len();
                for (tx, ty) in missing {
                    if started.elapsed().as_millis() >= 8 {
                        break;
                    }
                    let t = atlas::tile(&ctx, img, &mut self.palette, f, tx, ty);
                    self.atlas_tiles.insert((f, tx, ty), t);
                    left -= 1;
                }
                if left > 0 {
                    ctx.request_repaint();
                }
                let tint = Color32::from_white_alpha((atlas_w * 255.0) as u8);
                let mut levels: Vec<usize> = self.atlas_tiles.keys().map(|k| k.0).filter(|ff| *ff != f).collect();
                levels.sort_unstable_by(|a, b| b.cmp(a));
                levels.dedup();
                levels.push(f);
                // (Once this shrink is all made, it alone: the faded layers would add up otherwise.)
                if left == 0 {
                    levels = vec![f];
                }
                let _ = prev;
                for ff in levels {
                    if ff == 0 {
                        continue;
                    }
                    for (tx, ty) in tiles_in_view(img, plo, phi, ff, atlas::TILE) {
                        if let Some(t) = self.atlas_tiles.get(&(ff, tx, ty)) {
                            let (a, b) = tile_at(tx, ty, ff, atlas::TILE, t.size());
                            draw_turned(&clip, t.id(), &frame, a, b, tint);
                        }
                    }
                }
                if left == 0 {
                    self.last_atlas_f = f;
                }
            }
            // Zoomed in far enough, the land in 3D seen straight down: cliffs and trees stand,
            // and at the height looked at it lines up with the flat map exactly.
            if let Some(y) = look {
                let cam = self.above_camera(rect, y);
                self.camera = Some(cam);
                match self.gpu.as_mut() {
                    Some(w) => {
                        w.update(&ctx, (cam.target[0], cam.target[2]), self.render_distance);
                        w.update_lod((cam.target[0], cam.target[2]), self.render_distance);
                        if crate::gpu_world::World::can_paint(&cam) {
                            clip.add(egui::Shape::Callback(w.paint(&cam, ctx.input(|i| i.time), self.full_bright, daylight)));
                        }
                        ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    }
                    None => self.street.draw(&ctx, &clip, img, &cam),
                }
                // Crossing into (or out of) the Top-down view, the flat picture fades over it.
                if self.fade_3d < 1.0 {
                    let tint = Color32::from_white_alpha(((1.0 - self.fade_3d) * 255.0) as u8);
                    draw_turned(&clip, tex.id(), &frame, org, org + vec2(img.width as f32, img.height as f32), tint);
                }
            }
            }
        } else if let Some(e) = &self.error {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("Could not render the map: {e}"),
                theme::regular(15.0),
                theme::text_muted(),
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
            painter.rect_filled(r, theme::radius(4), theme::card());
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                format!("Rendering map\u{2026} {pct}%"),
                theme::regular(13.0),
                theme::text_muted(),
            );
        }

        let shift = ui.input(|i| i.modifiers.shift);
        // The street view needs the whole picture (it is not shown while the map is drawing).
        let can_tilt = self.image.is_some() && self.job.is_none();
        // (While the map is drawn again, after the Slicer changed, the 3D camera keeps its place
        // and comes back as it was once the map is ready.)
        let dt = ui.input(|i| i.stable_dt).clamp(1.0 / 240.0, 0.1);
        // Scrolling is gathered and eased in over the next frames, so zooming glides. Zooming in
        // past the closest Top-down view tilts into 3D; in 3D it tilts toward the horizon, then
        // comes nearer, then zooms the lens (and scrolling out undoes these in turn).
        if response.hovered() {
            let raw = ui.input(|i| i.raw_scroll_delta.y);
            // The free camera's wheel zooms its lens only (it stays where it is: WASD moves it).
            if raw != 0.0 && self.noclip && self.tilt > 0.0 && !self.homing {
                self.pending_lens += raw * 0.002;
            } else if raw != 0.0 {
                self.pending_zoom += raw * 0.002;
                self.zoom_anchor = ui.input(|i| i.pointer.hover_pos());
                self.homing = false;
            }
            if ui.input(|i| i.key_pressed(egui::Key::N)) && !ctx.wants_keyboard_input() {
                self.yaw = 0.0;
            }
        }
        if self.pending_zoom != 0.0 {
            // The Top-down button eases home more slowly than a scroll.
            let rate = if self.homing { 11.0 } else { 14.0 };
            let step = if self.pending_zoom.abs() < 0.003 {
                self.pending_zoom
            } else {
                self.pending_zoom * (1.0 - (-dt * rate).exp())
            };
            self.pending_zoom -= step;
            self.zoom_by(step, rect, can_tilt);
            ctx.request_repaint();
        } else {
            self.homing = false;
        }
        // The free camera flies with WASD (along the way it faces), Space (up) and Ctrl (down);
        // Shift goes faster.
        if self.tilt > 0.0 && self.noclip && !ctx.wants_keyboard_input() {
            let (keys, fast) = ui.input(|i| {
                let k = |key| if i.key_down(key) { 1.0 } else { 0.0 };
                ([k(egui::Key::W) - k(egui::Key::S), k(egui::Key::D) - k(egui::Key::A), k(egui::Key::Space) - if i.modifiers.ctrl { 1.0 } else { 0.0 }], i.modifiers.shift)
            });
            // The wheel's lens zoom, eased in like a scroll.
            if self.pending_lens != 0.0 {
                let step = if self.pending_lens.abs() < 0.003 { self.pending_lens } else { self.pending_lens * (1.0 - (-dt * 14.0).exp()) };
                self.pending_lens -= step;
                self.lens = (self.lens * step.exp()).clamp(1.0, 12.0);
                ctx.request_repaint();
            }
            if keys != [0.0; 3] {
                let speed = if fast { 36.0 } else { 12.0 } * dt;
                if let (Some(cam), Some(y)) = (self.camera, self.look_y) {
                    // Forward is the way the camera looks (up and down too), as an editor's
                    // flying camera; right is level; Space and Ctrl go straight up and down.
                    let eye = cam.eye();
                    let d = cam.dist.max(1e-3);
                    let fwd = [(cam.target[0] - eye[0]) / d, (cam.target[1] - eye[1]) / d, (cam.target[2] - eye[2]) / d];
                    let (_, right) = cam.ground_axes();
                    self.center.x += (fwd[0] * keys[0] + right[0] * keys[1]) * speed;
                    self.center.y += (fwd[2] * keys[0] + right[1] * keys[1]) * speed;
                    self.look_y = Some((y + (fwd[1] * keys[0] + keys[2]) * speed).clamp(-80.0, 400.0));
                }
                ctx.request_repaint();
            }
        }
        // Dragging moves the map (and, let go while moving, it glides on and slows). Right-drag
        // (or Shift+drag) turns the camera in 3D; in the Top-down view it starts turning from
        // straight down, into 3D. (The flat map further out stays north-up: right-drag moves it.)
        let street_now = self.tilt > 0.0;
        let orbit_from_above = !street_now && self.camera.is_some() && can_tilt && response.drag_started_by(egui::PointerButton::Secondary);
        if orbit_from_above {
            // Into 3D at the same scale, straight down and facing north.
            self.dolly = (TOP_MAX / self.zoom).clamp(1.0, TOP_MAX / ABOVE_FROM);
            self.tilt = 0.001;
            self.yaw = self.map_yaw();
            self.pitch_hand = None;
            self.momentum = Vec2::ZERO;
        }
        let street_now = self.tilt > 0.0;
        let turns = street_now
            && (response.dragged_by(egui::PointerButton::Secondary)
                || (shift && response.dragged_by(egui::PointerButton::Primary)));
        let boxing = !street_now && primary_pans && self.selectable && self.box_mode && (shift || self.marquee.is_some());
        // On the flat map further out, right-drag turns the map (where it may turn; else it moves it).
        let turns_flat = !street_now && self.camera.is_none() && self.can_rotate && response.dragged_by(egui::PointerButton::Secondary);
        // A face of the selection box dragged by its handle (in Box Select): that side moves,
        // a block at a time, instead of the map.
        let mut grabbing = false;
        if self.box_mode && self.selectable && primary_pans && self.box_anchor.is_none() && self.box_floor.is_none() && self.paste.is_none() && self.brush.is_none() {
            if response.drag_started_by(egui::PointerButton::Primary) {
                if let (Some(p), Some((lo, hi))) = (response.interact_pointer_pos(), self.sel_box) {
                    let near = self.handles.iter().filter(|h| h.2.distance(p) < 14.0).min_by(|a, b| a.2.distance(p).total_cmp(&b.2.distance(p)));
                    if let Some(&(axis, max, _, step)) = near {
                        self.box_drag = Some((axis, max, if max { hi[axis] } else { lo[axis] }, p, step));
                    }
                }
            }
            if let Some((axis, max, start, from, step)) = self.box_drag {
                grabbing = true;
                if let (Some(p), Some((mut lo, mut hi))) = (response.interact_pointer_pos(), self.sel_box) {
                    let blocks = ((p - from).dot(step) / step.length_sq().max(1e-3)).round() as i32;
                    let v = start + blocks;
                    if max {
                        hi[axis] = v.max(lo[axis]);
                    } else {
                        lo[axis] = v.min(hi[axis]);
                    }
                    self.sel_box = Some((lo, hi));
                }
                if !response.dragged() {
                    self.box_drag = None;
                    self.sync_box_chunks();
                }
            }
        }
        // Alt+drag with the brush paints instead of moving the map.
        let painting = self.brush.is_some() && self.selectable && ui.input(|i| i.modifiers.alt);
        let pans = !turns
            && !turns_flat
            && !boxing
            && !grabbing
            && !painting
            && ((primary_pans && response.dragged_by(egui::PointerButton::Primary))
                || response.dragged_by(egui::PointerButton::Middle)
                || (!street_now && response.dragged_by(egui::PointerButton::Secondary)));
        let delta = response.drag_delta();
        if turns_flat {
            self.yaw = (self.yaw - delta.x * 0.006).rem_euclid(std::f32::consts::TAU);
        }
        // The free camera turns where it is (its eye stays put, the point it looks at swings
        // round it), as an editor's camera; the usual camera turns around what it looks at.
        // (Measured from the camera as it is now, after any flying this frame, so turning while
        // flying keeps the flying.)
        let eye_before = match (turns && self.noclip, self.look_y) {
            (true, Some(y)) => Some(self.street_camera(rect, y).eye()),
            _ => None,
        };
        if turns {
            // Looking around: the pointer is out of the way.
            ctx.set_cursor_icon(egui::CursorIcon::None);
            let tau = std::f32::consts::TAU;
            // The usual camera swings round what it looks at (drag right, the land turns right
            // with the pointer); the free camera turns its head (drag right, look right).
            let turn = if self.noclip { delta.x } else { -delta.x };
            self.yaw = (self.yaw + turn * 0.006).rem_euclid(tau);
            // Up and down turns only the camera's angle: how far off it is (the scroll's) stays,
            // so looking down never backs it out and scrolling never has to undo a look. From
            // here on the scroll no longer tilts it (until back at Top-down).
            let deg = std::f32::consts::PI / 180.0;
            let lowest = if self.noclip { -80.0 } else { 8.0 };
            let now = self.pitch_hand.unwrap_or_else(|| self.base_pitch());
            self.pitch_hand = Some((now + delta.y * 0.004).clamp(lowest * deg, 90.0 * deg));
        }
        if let (Some(before), Some(y)) = (eye_before, self.look_y) {
            let after = self.street_camera(rect, y).eye();
            self.center.x += before[0] - after[0];
            self.center.y += before[2] - after[2];
            self.look_y = Some(y + before[1] - after[1]);
        }
        if ui.input(|i| i.pointer.any_pressed()) && response.hovered() {
            self.momentum = Vec2::ZERO;
        }
        if pans {
            self.pan_by(delta);
            self.drag_vel = self.drag_vel * 0.5 + delta / dt * 0.5;
            self.panning = true;
        } else if self.panning {
            // Let go: what speed the drag had carries on. (Not when something else took the
            // drag over, a shape's dot or a box's face: letting that go leaves the map still.)
            self.panning = false;
            if !response.dragged() && self.drag_vel.length() > 120.0 {
                self.momentum = self.drag_vel;
            }
            self.drag_vel = Vec2::ZERO;
        }
        if !pans && self.momentum != Vec2::ZERO {
            self.pan_by(self.momentum * dt);
            self.momentum *= (-dt * 6.0).exp();
            if self.momentum.length() < 8.0 {
                self.momentum = Vec2::ZERO;
            }
            ctx.request_repaint();
        }
        let street = self.tilt > 0.0;
        // The block under the pointer in the 3D views, found along the pointer's ray.
        self.picked = match (self.camera, &self.image, response.hover_pos().filter(|p| rect.contains(*p))) {
            // The block under the pointer: among the real blocks on the GPU, else on the land.
            (Some(cam), Some((img, _)), Some(p)) => match self.gpu.as_mut() {
                Some(w) => w.pick_with(&cam, p, self.pick_liquids),
                None => self.street.pick(img, &cam, p),
            },
            _ => None,
        };
        // A mob (or armor stand, item...) under the pointer, in front of the block it is on.
        self.hover_entity = match (self.camera, self.gpu.as_ref(), response.hover_pos().filter(|p| rect.contains(*p))) {
            (Some(cam), Some(w), Some(p)) => {
                let before = self.picked.map_or(f32::INFINITY, |h| crate::gpu_world::hit_distance(&cam, p, &h));
                w.pick_entity(&cam, p, before)
            }
            _ => None,
        };
        let mut frame = MapFrame {
            rect,
            center: self.center,
            zoom: self.zoom,
            response,
            hover: None,
            street,
            yaw: if street { self.yaw } else { self.map_yaw() },
        };
        frame.hover = if street {
            // Over a block, that block; over empty ground (the void, ungenerated chunks) the
            // point where the pointer's ray meets the height looked at, so the chunk there is
            // still highlighted (none above the horizon).
            // The land under the pointer as the map knows it (beyond the loaded blocks too), as
            // in Top-down; past the map's edge, the level of the sea. (Not the height looked at:
            // the free camera looks at a point beside itself, which put every miss in the
            // camera's own chunk.)
            match self.picked {
                Some(h) => Some(vec2(h.x as f32 + 0.5, h.z as f32 + 0.5)),
                None => match (self.camera, &self.image, frame.response.hover_pos().filter(|p| rect.contains(*p))) {
                    (Some(cam), Some((img, _)), Some(p)) => self.street.pick(img, &cam, p).map(|h| vec2(h.x as f32 + 0.5, h.z as f32 + 0.5)).or_else(|| {
                        let (eye, dir) = cam.ray(p);
                        let y = 63.0;
                        if dir[1] > -1e-4 || eye[1] <= y {
                            return None;
                        }
                        let t = (y - eye[1]) / dir[1];
                        (t < 4000.0).then(|| vec2(eye[0] + dir[0] * t, eye[2] + dir[2] * t))
                    }),
                    _ => None,
                },
            }
        } else {
            frame.response.hover_pos().filter(|p| rect.contains(*p)).map(|p| frame.to_block(p))
        };
        if self.paste.is_some() && primary_pans && self.selectable {
            self.paste_input(ui, &frame);
        } else if self.brush.is_some() && primary_pans && self.selectable {
            // The brush: a click paints around the block under the pointer; Esc stops.
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().wants_keyboard_input() {
                self.brush = None;
            }
            let snap = self.brush.as_ref().is_some_and(|b| b.snap);
            self.brush_at = self.hover_block(&frame).map(|b| if snap { [b[0].div_euclid(16) * 16 + 8, b[1], b[2].div_euclid(16) * 16 + 8] } else { b });
            // A click paints once; Alt+drag paints along the way, one step to undo for the stroke.
            let alt = ui.input(|i| i.modifiers.alt);
            if frame.response.clicked() {
                self.brush_request = self.brush_at.map(|b| (b, false));
            } else if alt && frame.response.dragged_by(egui::PointerButton::Primary) {
                // Along a stroke the brush stays at the height it began at (else it would climb
                // onto what it just painted): where the pointer's ray meets that level.
                if let (Some(last), Some(cam), Some(p)) = (self.brush_last, self.camera, frame.response.hover_pos()) {
                    let (eye, dir) = cam.ray(p);
                    let level = last[1] as f32 + 0.5;
                    if dir[1].abs() > 1e-4 {
                        let t = (level - eye[1]) / dir[1];
                        if t > 0.0 && t < 2000.0 {
                            let (x, z) = ((eye[0] + dir[0] * t).floor() as i32, (eye[2] + dir[2] * t).floor() as i32);
                            self.brush_at = Some(if snap { [x.div_euclid(16) * 16 + 8, last[1], z.div_euclid(16) * 16 + 8] } else { [x, last[1], z] });
                        }
                    }
                }
                if let Some(b) = self.brush_at.filter(|b| Some(*b) != self.brush_last) {
                    self.brush_request = Some((b, self.brush_last.is_some()));
                    self.brush_last = Some(b);
                }
            }
            if !frame.response.dragged() {
                self.brush_last = None;
            }
        } else if self.box_mode && primary_pans && self.selectable {
            self.box_input(ui, &frame, shift);
        } else if street || self.camera.is_some() {
            // With the blocks shown (3D and Top-down) a click selects what it is on: a mob (or
            // item, minecart...), else the block (plants and flowers too); Shift+click adds the
            // block clicked to the box. Esc clears the selection.
            self.marquee = None;
            if self.selectable && ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().wants_keyboard_input() {
                self.sel_entity = None;
                self.sel_box = None;
                self.sync_box_chunks();
            }
            if primary_pans && self.selectable && frame.response.clicked() {
                let shift = ui.input(|i| i.modifiers.shift);
                if let Some(e) = self.hover_entity.clone().filter(|_| !shift) {
                    let at = e.get("Pos").and_then(mctk_nbt::Tag::as_list).map(|l| (0..3).map(|i| l.get(i).and_then(mctk_nbt::Tag::as_f64).unwrap_or(0.0).floor() as i32).collect::<Vec<_>>());
                    self.sel_entity = Some(e);
                    self.sel_box = None;
                    self.selected = at.map(|p| vec![(p[0].div_euclid(16), p[2].div_euclid(16))]).unwrap_or_default();
                } else if let Some(h) = self.picked {
                    let b = [h.x, h.y, h.z];
                    self.sel_entity = None;
                    self.sel_box = match (shift, self.sel_box) {
                        (true, Some((lo, hi))) => Some(([lo[0].min(b[0]), lo[1].min(b[1]), lo[2].min(b[2])], [hi[0].max(b[0]), hi[1].max(b[1]), hi[2].max(b[2])])),
                        _ => Some((b, b)),
                    };
                    self.sync_box_chunks();
                }
            }
        } else {
            // The flat map picks no chunks by clicking: there the Prune shapes pick them, and
            // zoomed in the block or box selected says which chunks Open Chunks opens.
            self.marquee = None;
        }
        frame
    }

    /// The top block of the land in a column on the flat map: the water's surface or the ground.
    fn surface_y(&self, bx: i32, bz: i32) -> Option<i32> {
        let (img, _) = self.image.as_ref()?;
        let (x, z) = (bx - img.min_x * 16, bz - img.min_z * 16);
        if x < 0 || z < 0 || x as usize >= img.width || z as usize >= img.height {
            return None;
        }
        let i = z as usize * img.width + x as usize;
        (img.blocks[i] != 0).then(|| if img.water[i] > 0 { img.heights[i] as i32 } else { img.ground[i] as i32 })
    }

    /// The block under the pointer for Box Select: the one picked in 3D, else the top block of
    /// the land under the pointer on the flat map.
    fn hover_block(&self, frame: &MapFrame) -> Option<[i32; 3]> {
        if let Some(p) = self.picked.filter(|_| self.camera.is_some()) {
            return Some([p.x, p.y, p.z]);
        }
        let h = frame.hover?;
        let (bx, bz) = (h.x.floor() as i32, h.y.floor() as i32);
        Some([bx, self.surface_y(bx, bz)?, bz])
    }

    /// Placing the clipboard: the box follows the block under the pointer (sitting on top of
    /// it); R turns it, M mirrors it, the arrow keys and Page Up/Down nudge it, a click places
    /// it (as often as wanted) and Esc stops.
    fn paste_input(&mut self, ui: &Ui, frame: &MapFrame) {
        let Some(size) = self.clipboard.as_ref().map(|c| c.size) else {
            self.paste = None;
            return;
        };
        let typing = ui.ctx().wants_keyboard_input();
        let keys = |k: egui::Key| !typing && ui.input(|i| i.key_pressed(k));
        if keys(egui::Key::Escape) {
            self.paste = None;
            return;
        }
        let hover = self.hover_block(frame);
        let Some(p) = &mut self.paste else { return };
        if keys(egui::Key::R) {
            p.place.turns = (p.place.turns + 1) % 4;
        }
        if keys(egui::Key::M) {
            p.place.mirror = !p.place.mirror;
        }
        for (k, d) in [(egui::Key::ArrowLeft, [-1, 0, 0]), (egui::Key::ArrowRight, [1, 0, 0]), (egui::Key::ArrowUp, [0, 0, -1]), (egui::Key::ArrowDown, [0, 0, 1]), (egui::Key::PageUp, [0, 1, 0]), (egui::Key::PageDown, [0, -1, 0])] {
            if keys(k) {
                for i in 0..3 {
                    p.nudge[i] += d[i];
                }
            }
        }
        // On top of the block pointed at, centered on it across.
        let placed = p.place.size(size);
        if let Some(h) = hover {
            p.at = Some([h[0] - placed[0] / 2 + p.nudge[0], h[1] + 1 + p.nudge[1], h[2] - placed[2] / 2 + p.nudge[2]]);
        }
        if frame.response.clicked() {
            if let Some(at) = p.at {
                self.paste_request = Some((at, p.place));
            }
        }
    }

    /// The chunks the selection box covers become the chunks picked.
    fn sync_box_chunks(&mut self) {
        self.selected.clear();
        let Some((lo, hi)) = self.sel_box else { return };
        let (c0, c1) = ((lo[0].div_euclid(16), lo[2].div_euclid(16)), (hi[0].div_euclid(16), hi[2].div_euclid(16)));
        if (c1.0 - c0.0 + 1) as i64 * (c1.1 - c0.1 + 1) as i64 > 20_000 {
            return;
        }
        for z in c0.1..=c1.1 {
            for x in c0.0..=c1.0 {
                self.selected.push((x, z));
            }
        }
    }

    /// Clicks and drags in Box Select. A click sets one corner, the next the other (the box
    /// follows the pointer in between); Shift+drag on the flat map boxes an area as tall as the
    /// land in it; Esc clears the box.
    fn box_input(&mut self, ui: &Ui, frame: &MapFrame, shift: bool) {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.sel_box = None;
            self.box_anchor = None;
            self.box_floor = None;
            self.sync_box_chunks();
        }
        let hover = self.hover_block(frame);
        if !frame.street && shift {
            let at = frame.response.interact_pointer_pos().map(|p| frame.to_block(p));
            if frame.response.drag_started_by(egui::PointerButton::Primary) {
                self.marquee = at.map(|a| (a, a));
            }
            if let (Some((a, _)), Some(b)) = (self.marquee, at) {
                if frame.response.dragged() {
                    self.marquee = Some((a, b));
                }
            }
        }
        if frame.response.drag_stopped() {
            if let Some((a, b)) = self.marquee.take() {
                let (x0, x1) = (a.x.min(b.x).floor() as i32, a.x.max(b.x).floor() as i32);
                let (z0, z1) = (a.y.min(b.y).floor() as i32, a.y.max(b.y).floor() as i32);
                // As tall as the land in the area: from its lowest top block to its highest
                // (looked at every few columns over a big area).
                let step = (((x1 - x0 + 1) as i64 * (z1 - z0 + 1) as i64) as f64 / 250_000.0).sqrt().ceil().max(1.0) as usize;
                let (mut lo, mut hi) = (i32::MAX, i32::MIN);
                for z in (z0..=z1).step_by(step) {
                    for x in (x0..=x1).step_by(step) {
                        if let Some(y) = self.surface_y(x, z) {
                            lo = lo.min(y);
                            hi = hi.max(y);
                        }
                    }
                }
                if lo <= hi {
                    self.sel_box = Some(([x0, lo, z0], [x1, hi, z1]));
                    self.box_anchor = None;
                    self.sync_box_chunks();
                }
            }
            return;
        }
        // In 3D, three steps: a click sets the first corner; moving stretches the box across that
        // block's level (width and length) and a click sets them; then the box's height follows
        // the pointer up and down (over air too) and a click sets it.
        if let Some(cam) = self.camera.filter(|_| frame.street) {
            let ray = frame.response.hover_pos().filter(|p| frame.rect.contains(*p)).map(|p| cam.ray(p));
            let clicked = frame.response.clicked();
            if let Some((lo, hi)) = self.box_floor {
                if let Some((eye, dir)) = ray {
                    let mid = [(lo[0] + hi[0] + 1) as f32 / 2.0, (lo[2] + hi[2] + 1) as f32 / 2.0];
                    // Above the floor the height pointed at is the box's top face (pointing at the
                    // floor's own top keeps it one block tall); below, its bottom face.
                    let h = height_on_line(eye, dir, mid);
                    let floor = lo[1];
                    let (y0, y1) = if h >= floor as f32 + 0.5 {
                        (floor, (h.round() as i32 - 1).max(floor))
                    } else {
                        ((h.floor() as i32).min(floor), floor)
                    };
                    self.sel_box = Some(([lo[0], y0.clamp(-64, 319), lo[2]], [hi[0], y1.clamp(-64, 319), hi[2]]));
                }
                if clicked {
                    self.box_floor = None;
                    self.sync_box_chunks();
                }
                return;
            }
            if let Some(a) = self.box_anchor {
                // Where the pointer's ray meets the top of the first block's level.
                let level = a[1] as f32 + 1.0;
                let xz = ray
                    .and_then(|(eye, dir)| {
                        if dir[1].abs() < 1e-4 {
                            return None;
                        }
                        let t = (level - eye[1]) / dir[1];
                        (t > 0.0 && t < 2000.0).then(|| [(eye[0] + dir[0] * t).floor() as i32, (eye[2] + dir[2] * t).floor() as i32])
                    })
                    .or(hover.map(|h| [h[0], h[2]]));
                if let Some(b) = xz {
                    self.sel_box = Some(span(a, [b[0], a[1], b[1]]));
                }
                if clicked {
                    self.box_anchor = None;
                    self.box_floor = self.sel_box;
                }
                return;
            }
            if clicked {
                if let Some(b) = hover {
                    self.box_anchor = Some(b);
                    self.sel_box = Some((b, b));
                    self.sel_entity = None;
                    self.sync_box_chunks();
                }
            }
            return;
        }
        if frame.response.clicked() {
            if let Some(b) = hover {
                match self.box_anchor.take() {
                    Some(a) => self.sel_box = Some(span(a, b)),
                    None => {
                        self.sel_box = Some((b, b));
                        self.box_anchor = Some(b);
                    }
                }
                self.sync_box_chunks();
            }
        } else if let (Some(a), Some(b)) = (self.box_anchor, hover) {
            // Choosing the second corner: the box follows the pointer.
            self.sel_box = Some(span(a, b));
        }
    }

    /// Applies `d` zoom steps (positive: in) around the point zoomed at. On the flat map past the
    /// closest top-down view the rest tilts into the street view; in the street view scrolling in
    /// tilts toward the horizon and then comes nearer, and scrolling out does the reverse and,
    /// back at top-down, turns to face north.
    fn zoom_by(&mut self, d: f32, rect: Rect, can_tilt: bool) {
        if self.tilt <= 0.0 {
            let p = self.zoom_anchor.filter(|p| rect.contains(*p)).unwrap_or(rect.center());
            let yaw = self.map_yaw();
            let before = self.center + turn((p - rect.center()) / self.zoom, -yaw);
            let want = self.zoom * d.exp();
            if want > TOP_MAX && can_tilt {
                // As close as top-down goes: the rest of the scroll tilts (and goes on into the
                // 3D view's own stages).
                let used = (TOP_MAX / self.zoom).ln().max(0.0);
                self.zoom = TOP_MAX;
                self.center = before - turn((p - rect.center()) / self.zoom, -yaw);
                let rest = d - used;
                if rest > 0.0 {
                    self.tilt = 0.001;
                    self.zoom_by(rest - 0.001 / TILT_RATE, rect, can_tilt);
                }
            } else {
                self.zoom = want.clamp(0.03, TOP_MAX);
                self.center = before - turn((p - rect.center()) / self.zoom, -yaw);
            }
            return;
        }
        let tau = std::f32::consts::TAU;
        // What a stage does not use of a step goes on to the next, so easing across a stage's end
        // loses nothing (the Top-down button then always gets all the way back).
        let mut d = d;
        if d > 0.0 {
            // In: tilt, then come nearer, then (as near as the camera may come) zoom the lens.
            let nearest = self.min_dolly(rect);
            if self.tilt < 1.0 {
                let used = d.min((1.0 - self.tilt) / TILT_RATE);
                self.tilt = (self.tilt + used * TILT_RATE).min(1.0);
                d -= used;
            }
            if d > 0.0 && self.dolly > nearest * 1.001 {
                let used = d.min((self.dolly / nearest).ln());
                self.dolly = (self.dolly * (-used).exp()).max(nearest);
                d -= used;
            }
            // (Not while easing to a set place, as the free camera's start: that stops at the
            // nearest and leaves the lens alone.)
            if d > 0.0 && !self.homing {
                self.lens = (self.lens * d.exp()).min(12.0);
            }
        } else if d < 0.0 {
            // Out: the lens back first, then the camera back, then the tilt.
            if self.lens > 1.0 {
                let used = d.max(-self.lens.ln());
                self.lens = (self.lens * used.exp()).max(1.0);
                d -= used;
            }
            if d < 0.0 && self.dolly < 1.0 {
                let used = d.max(self.dolly.ln());
                self.dolly = (self.dolly * (-used).exp()).min(1.0);
                d -= used;
            }
            if d < 0.0 {
                let tilt = self.tilt + d * TILT_RATE;
                if tilt <= 0.0 {
                    self.leave_street();
                } else {
                    // Unwind the turn and the hand's angle with the tilt, so the camera arrives at
                    // Top-down looking straight down (and the flat map comes back facing north).
                    let k = tilt / self.tilt;
                    let down = std::f32::consts::FRAC_PI_2;
                    self.pitch_hand = self.pitch_hand.map(|p| down - (down - p) * k);
                    // (The Pruner's map is north-up: there the turn unwinds too.)
                    if !self.can_rotate {
                        self.yaw = (self.yaw + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
                        self.yaw *= k;
                    }
                    self.tilt = tilt;
                }
            }
        }
    }

    /// Eases back down to the Top-down view: the lens, nearness and tilt are undone in turn, as
    /// scrolling out would, but more slowly.
    pub(crate) fn go_top_down(&mut self) {
        let need = self.lens.max(1.0).ln() + (-self.dolly.min(1.0).ln()).max(0.0) + self.tilt / TILT_RATE;
        self.pending_zoom = -(need + 0.002);
        self.homing = true;
    }

    /// Turns the free camera on (easing into 3D up close) or off (easing back to Top-down), as
    /// its button does; `rect` is the map's.
    pub(crate) fn set_free_camera(&mut self, on: bool, rect: Rect) {
        // Starting from 3D, the free camera starts exactly where the camera was: how far the land
        // brought it in, or the eye raised over a hill, are kept (instead of it dropping to the
        // ground the moment those stop applying).
        if on && !self.noclip && self.tilt > 0.0 {
            if let (Some(cam), Some(y)) = (self.camera, self.look_y) {
                let plain = self.street_camera(rect, y).dist;
                if plain > 1e-3 {
                    self.dolly = (self.dolly * cam.dist / plain).clamp(self.min_dolly(rect), 1.0);
                }
            }
        }
        self.noclip = on;
        self.pending_lens = 0.0;
        self.lift = 0.0;
        if on {
            self.go_near_3d(rect);
        } else {
            self.go_top_down();
        }
    }

    /// Eases into the 3D view as near as the camera comes (the lens left at 1), as a 3D editor's
    /// camera starts: for the free camera.
    fn go_near_3d(&mut self, rect: Rect) {
        self.lens = 1.0;
        let from_top = if self.tilt <= 0.0 { (TOP_MAX / self.zoom.max(0.01)).ln().max(0.0) } else { 0.0 };
        // As near as the usual camera comes (a few blocks off, up in the air), not the free
        // camera's own nearest (a block off, down on the ground).
        let nearest = (USUAL_NEAREST / (Camera::dist_for_zoom(rect, TOP_MAX) * 0.5)).min(1.0);
        let need = from_top + (1.0 - self.tilt) / TILT_RATE + (self.dolly / nearest).ln().max(0.0);
        self.pending_zoom = need;
        self.zoom_anchor = None;
        self.homing = true;
    }

    /// While a tool drags something near the view's edge: scrolls that way (faster the nearer
    /// the edge), so the drag can go on past it.
    pub(crate) fn edge_scroll(&mut self, ctx: &egui::Context, rect: Rect, pointer: Pos2) {
        const EDGE: f32 = 48.0;
        let push = |lo: f32, hi: f32, v: f32| {
            if v < lo + EDGE {
                ((lo + EDGE - v) / EDGE).min(1.5)
            } else if v > hi - EDGE {
                -((v - (hi - EDGE)) / EDGE).min(1.5)
            } else {
                0.0
            }
        };
        let dir = vec2(push(rect.left(), rect.right(), pointer.x), push(rect.top(), rect.bottom(), pointer.y));
        if dir != Vec2::ZERO {
            let dt = ctx.input(|i| i.stable_dt).clamp(1.0 / 240.0, 0.1);
            self.momentum = Vec2::ZERO;
            self.pan_by(dir * 600.0 * dt);
            ctx.request_repaint();
        }
    }

    /// Moves the map by a drag of `delta` screen pixels: the land under the pointer follows it.
    fn pan_by(&mut self, delta: Vec2) {
        match self.camera.filter(|_| self.tilt > 0.0) {
            Some(cam) => {
                let (fwd, right) = cam.ground_axes();
                let s = cam.drag_scale();
                // Further away the land moves faster under the pointer, as it does.
                let k = s / cam.pitch.sin().max(0.35);
                self.center.x += -right[0] * delta.x * s + fwd[0] * delta.y * k;
                self.center.y += -right[1] * delta.x * s + fwd[1] * delta.y * k;
            }
            None => self.center -= turn(delta / self.zoom, -self.map_yaw()),
        }
    }

    /// The Overview inset in the bottom-right corner: the flat map from above (north up) around
    /// what the camera looks at, with the camera, the way it faces and the ground in view.
    /// Clicking it moves the camera there. Returns where it was drawn.
    fn overview_inset(&mut self, ui: &mut Ui, frame: &MapFrame, ks: f32) -> Option<Rect> {
        let cam = self.camera?;
        let (img, tex) = self.image.as_ref()?;
        let size = 220.0 * ks;
        // Left of the compass when it shows.
        let right = frame.rect.right() - if frame.street || frame.yaw != 0.0 { 84.0 * ks } else { 14.0 * ks };
        let r = Rect::from_min_size(pos2(right - size, frame.rect.bottom() - 14.0 * ks - size), Vec2::splat(size));
        let resp = ui.interact(r, ui.id().with("overview-inset"), Sense::click()).on_hover_cursor(egui::CursorIcon::Crosshair);
        let p = ui.painter_at(r);
        p.rect_filled(r, theme::radius(6), theme::deep());

        let eye = cam.eye();
        let (cx, cz) = (cam.target[0], cam.target[2]);
        let foot = cam.footprint(cam.target[1], 600.0);
        // Wide enough for the camera and the near edge of the view, with room around them.
        let reach = |q: [f32; 2]| ((q[0] - cx).powi(2) + (q[1] - cz).powi(2)).sqrt();
        let radius = (reach([eye[0], eye[2]]).max(reach(foot[2])).max(reach(foot[3])) * 1.4).clamp(24.0, 400.0);
        let scale = size / (2.0 * radius);
        let to_px = |x: f32, z: f32| r.center() + vec2(x - cx, z - cz) * scale;

        // The map picture under the part of the world shown (clipped to what was drawn).
        let (ox, oz) = (img.min_x as f32 * 16.0, img.min_z as f32 * 16.0);
        let (w, h) = (img.width as f32, img.height as f32);
        let (x0, x1) = ((cx - radius).max(ox), (cx + radius).min(ox + w));
        let (z0, z1) = ((cz - radius).max(oz), (cz + radius).min(oz + h));
        if x1 > x0 && z1 > z0 {
            let uv = Rect::from_min_max(pos2((x0 - ox) / w, (z0 - oz) / h), pos2((x1 - ox) / w, (z1 - oz) / h));
            p.image(tex.id(), Rect::from_min_max(to_px(x0, z0), to_px(x1, z1)), uv, Color32::WHITE);
        }

        // The ground in view, and the field of view's edges out from the eye.
        let e = to_px(eye[0], eye[2]);
        let pts: Vec<Pos2> = foot.iter().map(|q| to_px(q[0], q[1])).collect();
        p.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_white_alpha(46), Stroke::NONE));
        p.add(egui::Shape::closed_line(pts.clone(), Stroke::new(1.2 * ks, Color32::from_white_alpha(200))));
        let fov_line = Stroke::new(1.2 * ks, Color32::from_rgba_unmultiplied(255, 220, 90, 220));
        p.line_segment([e, pts[0]], fov_line);
        p.line_segment([e, pts[1]], fov_line);
        // What the camera looks at, then the camera with an arrow the way it faces.
        let t = to_px(cx, cz);
        let cross = Stroke::new(1.5 * ks, Color32::WHITE);
        p.line_segment([t - vec2(4.0, 0.0) * ks, t + vec2(4.0, 0.0) * ks], cross);
        p.line_segment([t - vec2(0.0, 4.0) * ks, t + vec2(0.0, 4.0) * ks], cross);
        let (fwd, _) = cam.ground_axes();
        let dir = vec2(fwd[0], fwd[1]);
        let side = vec2(-dir.y, dir.x);
        let tip = e + dir * 12.0 * ks;
        p.add(egui::Shape::convex_polygon(vec![tip, e + side * 5.0 * ks - dir * 3.0 * ks, e - side * 5.0 * ks - dir * 3.0 * ks], theme::accent(), Stroke::new(1.0 * ks, Color32::BLACK)));
        p.circle_filled(e, 3.5 * ks, Color32::WHITE);

        p.text(pos2(r.center().x, r.top() + 4.0 * ks), Align2::CENTER_TOP, "N", theme::regular(11.0 * ks), theme::text());
        ui.painter().rect_stroke(r, theme::radius(6), Stroke::new(1.0 * ks, theme::border()), StrokeKind::Inside);

        // Click to look at that place instead (the camera keeps its angle and distance).
        if resp.clicked() {
            if let Some(at) = resp.interact_pointer_pos() {
                let d = (at - r.center()) / scale;
                self.center = vec2(cx + d.x, cz + d.y);
                self.momentum = Vec2::ZERO;
            }
        }
        Some(r)
    }

    /// Rulers, selection, tooltip and the floating controls, drawn over any overlays.
    pub fn show_chrome(&mut self, ui: &mut Ui, frame: &MapFrame) {
        self.sync_ghost();
        // The chunk highlight can be hidden in Top-down and 3D; leaving for the flat map turns
        // it back on.
        if self.camera.is_none() {
            self.chunk_highlight = true;
        }
        let outer = Rect::from_min_max(frame.rect.min - vec2(RULER, RULER), frame.rect.max);
        let p = ui.painter_at(outer);
        // On the GPU the lines in the world (block outline, chunk walls, the selection) are drawn
        // among the blocks, hidden behind them, instead of over everything.
        let to_gpu = self.gpu.is_some() && self.camera.is_some();
        if to_gpu {
            crate::street_view::gather_lines();
        }
        let inside = p.with_clip_rect(frame.rect);
        let green = Color32::from_rgb(0x1A, 0xFB, 0x2F);
        // In the 3D views (Top-down zoomed in, or tilted) picked chunks are drawn as the game
        // draws chunk borders, rising from the land; on the flat map as squares (turned with the
        // map when it is turned).
        let view3d = match (self.camera, &self.image) {
            (Some(cam), Some(_)) => Some(cam),
            _ => None,
        };
        let top_y = world_top(&self.dimension);
        let square = |x0: f32, z0: f32, x1: f32, z1: f32| -> Vec<Pos2> {
            vec![frame.to_screen(vec2(x0, z0)), frame.to_screen(vec2(x1, z0)), frame.to_screen(vec2(x1, z1)), frame.to_screen(vec2(x0, z1))]
        };
        if self.chunk_grid {
            match (frame.street, view3d, &self.image) {
                // In 3D: an upright line at each chunk corner, rising from the land.
                (true, Some(cam), Some((img, _))) => self.street.chunk_grid(&inside, img, &cam, top_y),
                // On the flat map and the Top-down view: lines along the chunk borders.
                (false, ..) if frame.zoom * 16.0 >= 6.0 => {
                    let stroke = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 220, 60, 150));
                    let (lo, hi) = frame.world_bounds();
                    let mut x = (lo.x / 16.0).floor() * 16.0;
                    while x <= hi.x {
                        inside.line_segment([frame.to_screen(vec2(x, lo.y)), frame.to_screen(vec2(x, hi.y))], stroke);
                        x += 16.0;
                    }
                    let mut z = (lo.y / 16.0).floor() * 16.0;
                    while z <= hi.y {
                        inside.line_segment([frame.to_screen(vec2(lo.x, z)), frame.to_screen(vec2(hi.x, z))], stroke);
                        z += 16.0;
                    }
                }
                _ => {}
            }
        }
        if self.selectable {
            if let (Some(cam), Some((img, _))) = (view3d, &self.image) {
                let stroke = Stroke::new(2.0_f32, green);
                // The first few picked chunks lined every 16 blocks up; the rest by their corners.
                for (k, &(cx, cz)) in self.selected.iter().take(400).enumerate() {
                    let up = if k < 32 { 16 } else { 100_000 };
                    self.street.chunk_walls(&inside, img, &cam, cx, cz, stroke, 16, up, top_y, self.land_outline);
                }
            }
            // Where the clipboard would be pasted: a cyan box.
            if let (Some(p), Some(clip)) = (&self.paste, &self.clipboard) {
                if let Some(at) = p.at {
                    let s = p.place.size(clip.size);
                    let (l3, h3) = ([at[0] as f32, at[1] as f32, at[2] as f32], [(at[0] + s[0]) as f32, (at[1] + s[1]) as f32, (at[2] + s[2]) as f32]);
                    let cyan = Color32::from_rgb(80, 220, 255);
                    match view3d {
                        Some(cam) => Street::selection(&inside, &cam, l3, h3, cyan),
                        None if !frame.street => {
                            let pts = square(l3[0], l3[2], h3[0], h3[2]);
                            inside.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_unmultiplied(80, 220, 255, 40), Stroke::NONE));
                            inside.add(egui::Shape::closed_line(pts, Stroke::new(2.0_f32, cyan)));
                        }
                        None => {}
                    }
                }
            }
            // The mob under the pointer outlined in white, the one selected in yellow: the outline
            // of its model, as the game's glowing (seen through what is in front too).
            // Barriers near the camera, by their icon (as the game shows them while holding one),
            // each where nothing solid is in front of it.
            if let (Some(cam), Some(w)) = (view3d, self.gpu.as_mut()) {
                w.show_barriers = self.show_barriers;
                if self.show_barriers {
                    let f = cam.focal();
                    for b in w.barriers_near(cam.eye(), cam.target, 48.0, 400) {
                        let c = [b[0] as f32 + 0.5, b[1] as f32 + 0.5, b[2] as f32 + 0.5];
                        if let Some((s, depth)) = cam.project(c) {
                            let size = (0.8 * f / depth.max(0.1)).clamp(4.0, 160.0);
                            let r = Rect::from_center_size(s, Vec2::splat(size));
                            if r.intersects(frame.rect) {
                                icons::item_icon(&inside, r, "minecraft:barrier", Color32::WHITE);
                            }
                        }
                    }
                }
            }
            if let Some(cam) = view3d {
                let same =|a: &Option<std::sync::Arc<mctk_nbt::Tag>>, b: &Option<std::sync::Arc<mctk_nbt::Tag>>| matches!((a, b), (Some(a), Some(b)) if std::sync::Arc::ptr_eq(a, b));
                if let Some(e) = self.hover_entity.as_ref().filter(|_| !self.over_controls && !same(&self.hover_entity, &self.sel_entity)) {
                    entity_outline(&inside, &cam, e, frame.rect, Color32::from_rgba_unmultiplied(255, 255, 255, 220));
                }
                if let Some(e) = &self.sel_entity {
                    entity_outline(&inside, &cam, e, frame.rect, Color32::from_rgb(255, 220, 60));
                }
            }
            // The brush: the block it would paint around, and how far it reaches (magenta).
            if let (Some(b), Some(at)) = (&self.brush, self.brush_at) {
                let r = b.radius as f32;
                let (l3, h3) = ([at[0] as f32 - r, at[1] as f32 - r, at[2] as f32 - r], [at[0] as f32 + r + 1.0, at[1] as f32 + r + 1.0, at[2] as f32 + r + 1.0]);
                let magenta = Color32::from_rgb(240, 90, 220);
                match view3d {
                    Some(cam) => Street::selection(&inside, &cam, l3, h3, magenta),
                    None if !frame.street => {
                        inside.add(egui::Shape::closed_line(square(l3[0], l3[2], h3[0], h3[2]), Stroke::new(2.0_f32, magenta)));
                    }
                    None => {}
                }
            }
            // Handles on the selection box's faces, to drag them (in Box Select, once both
            // corners are set): every face in 3D, the four sides on the flat map.
            self.handles.clear();
            if let (true, Some((lo, hi)), None) = (self.box_mode && self.paste.is_none() && self.brush.is_none() && self.box_floor.is_none(), self.sel_box, self.box_anchor) {
                let (l, h) = ([lo[0] as f32, lo[1] as f32, lo[2] as f32], [hi[0] as f32 + 1.0, hi[1] as f32 + 1.0, hi[2] as f32 + 1.0]);
                let mid = [(l[0] + h[0]) / 2.0, (l[1] + h[1]) / 2.0, (l[2] + h[2]) / 2.0];
                for axis in 0..3 {
                    for max in [false, true] {
                        let mut c = mid;
                        c[axis] = if max { h[axis] } else { l[axis] };
                        let mut c1 = c;
                        c1[axis] += 1.0;
                        let at = match view3d {
                            Some(cam) => cam.project(c).zip(cam.project(c1)).map(|((a, _), (b, _))| (a, b - a)),
                            None if !frame.street && axis != 1 => {
                                let (a, b) = (frame.to_screen(vec2(c[0], c[2])), frame.to_screen(vec2(c1[0], c1[2])));
                                Some((a, b - a))
                            }
                            None => None,
                        };
                        if let Some((p, step)) = at.filter(|(p, _)| frame.rect.contains(*p)) {
                            self.handles.push((axis, max, p, step));
                        }
                    }
                }
                let pointer = frame.response.hover_pos();
                let held = self.box_drag.map(|d| (d.0, d.1));
                for &(axis, max, p, _) in &self.handles {
                    let hot = held == Some((axis, max)) || pointer.is_some_and(|q| q.distance(p) < 14.0);
                    let r = Rect::from_center_size(p, vec2(if hot { 12.0 } else { 9.0 }, if hot { 12.0 } else { 9.0 }));
                    inside.rect_filled(r, 2.0, if hot { Color32::from_rgb(80, 220, 255) } else { Color32::WHITE });
                    inside.rect_stroke(r, 2.0, Stroke::new(1.0_f32, Color32::from_black_alpha(160)), StrokeKind::Inside);
                }
                if pointer.is_some_and(|q| self.handles.iter().any(|h| h.2.distance(q) < 14.0)) || self.box_drag.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }
            if let Some((lo, hi)) = self.sel_box {
                let color = Color32::from_rgb(255, 255, 255);
                let (l3, h3) = ([lo[0] as f32, lo[1] as f32, lo[2] as f32], [hi[0] as f32 + 1.0, hi[1] as f32 + 1.0, hi[2] as f32 + 1.0]);
                match view3d {
                    Some(cam) => Street::selection(&inside, &cam, l3, h3, color),
                    None if !frame.street => {
                        let pts = square(l3[0], l3[2], h3[0], h3[2]);
                        inside.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_white_alpha(28), Stroke::NONE));
                        inside.add(egui::Shape::closed_line(pts, Stroke::new(2.0_f32, color)));
                    }
                    None => {}
                }
            }
        }
        if self.selectable && view3d.is_none() && !frame.street {
            for &(cx, cz) in &self.selected {
                let (x0, z0) = (cx as f32 * 16.0, cz as f32 * 16.0);
                let pts = square(x0, z0, x0 + 16.0, z0 + 16.0);
                if !Rect::from_points(&pts).intersects(frame.rect) {
                    continue;
                }
                // As in the original tool: a green tint with a green border split by a black line.
                inside.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_premultiplied(0, 83, 16, 84), Stroke::NONE));
                inside.add(egui::Shape::closed_line(pts.clone(), Stroke::new(3.0_f32, green)));
                inside.add(egui::Shape::closed_line(pts, Stroke::new(1.0_f32, Color32::BLACK)));
            }
        }
        // Chunks deleted by the editing tools and not saved yet: dark, crossed out.
        if view3d.is_none() && !frame.street {
            let gone = self.edits.read().map(|l| l.deleted_in(&self.dimension)).unwrap_or_default();
            let red = Stroke::new(1.5_f32, Color32::from_rgb(230, 80, 70));
            for (cx, cz) in gone {
                let (x0, z0) = (cx as f32 * 16.0, cz as f32 * 16.0);
                let pts = square(x0, z0, x0 + 16.0, z0 + 16.0);
                if !Rect::from_points(&pts).intersects(frame.rect) {
                    continue;
                }
                inside.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_black_alpha(190), Stroke::NONE));
                inside.line_segment([pts[0], pts[2]], red);
                inside.line_segment([pts[1], pts[3]], red);
            }
        }
        if self.selectable && !frame.street {
            if let Some((a, b)) = self.marquee {
                let pts = square(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y));
                inside.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_premultiplied(0, 40, 8, 40), Stroke::NONE));
                inside.add(egui::Shape::closed_line(pts, Stroke::new(1.0_f32, green)));
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
        let ruler_bg = theme::button();
        let tick = Stroke::new(1.0_f32, theme::text_dim());
        let digits = theme::text_muted();
        p.rect_filled(top, 0.0, ruler_bg);
        p.rect_filled(left, 0.0, ruler_bg);
        p.rect_filled(Rect::from_min_max(outer.min, frame.rect.min), 0.0, theme::bg());
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
        // The street view has no flat scale: its rulers stay empty.
        let ticks = !frame.street && frame.yaw == 0.0;
        let mut i = (b0.x / minor).floor() as i64;
        while ticks && i as f32 * minor <= b1.x {
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
        while ticks && i as f32 * minor <= b1.y {
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
        }
        // Named mobs' names over their heads, as the game's name tags (near enough to read).
        if let (Some(cam), Some(w)) = (self.camera.filter(|_| frame.street && self.show_details), self.gpu.as_ref()) {
            let f = cam.focal();
            for (at, name) in w.named_entities() {
                let Some((s, depth)) = cam.project(at) else { continue };
                if depth > 48.0 || !frame.rect.contains(s) {
                    continue;
                }
                let size = (0.3 * f / depth).clamp(9.0, 20.0);
                let galley = inside.layout_no_wrap(name, theme::regular(size), Color32::WHITE);
                let r = Rect::from_center_size(s, galley.size() + vec2(size * 0.4, size * 0.15));
                inside.rect_filled(r, 0.0, Color32::from_black_alpha(64));
                inside.galley(r.center() - galley.size() / 2.0, galley, Color32::WHITE);
            }
        }
        // What the container or mob under the pointer holds, in a card by the pointer.
        // (Over everything on the view, its folded-out panels too, so nothing covers it; and not
        // while the pointer is on those, or a menu is open.)
        let menu_open = crate::ctx_menu::any_open(ui.ctx());
        if !self.over_controls && !self.hide_hover && !menu_open && self.camera.is_some() && self.show_details {
            if let Some(pointer) = frame.response.hover_pos().filter(|p| frame.rect.contains(*p)) {
                if let Some(card) = self.contents_card() {
                    let top = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("explorer-contents-card")));
                    paint_contents(&top.with_clip_rect(frame.rect), frame.rect, pointer, &card);
                }
            }
        }
        if let Some(h) = frame.hover.filter(|_| !self.over_controls && !self.hide_hover) {
            // In the 3D views, the block the pointer is on (side or top), outlined as the game
            // does; its coordinates are the ones shown.
            // Only once the blocks are big enough to be seen as blocks: in the street view, or
            // looking down from `OUTLINE_FROM` in.
            let picked = self.picked.filter(|_| self.camera.is_some() && (frame.street || self.zoom >= OUTLINE_FROM));
            let (bx, bz) = picked.map_or((h.x.floor() as i64, h.y.floor() as i64), |k| (k.x as i64, k.z as i64));
            if let (Some(cam), Some(k)) = (self.camera, picked) {
                Street::outline(&p.with_clip_rect(frame.rect), &cam, &k);
            }
            // The chunk under the pointer's walls: on the GPU drawn inverted over what is behind
            // them (as the game's crosshair), so they show on any ground; else dark.
            let wall = if to_gpu { Stroke::new(2.0_f32, crate::gpu_world::INVERT_LINE) } else { Stroke::new(2.0_f32, Color32::from_rgba_unmultiplied(0x14, 0x18, 0x1C, 220)) };
            // (Not while a box is being made or a face of it dragged: the box is what matters.)
            let show_chunk = self.chunk_highlight && self.box_anchor.is_none() && self.box_floor.is_none() && self.box_drag.is_none();
            let _pointer = if frame.street {
                if let (true, Some(cam), Some((img, _))) = (show_chunk, self.camera, &self.image) {
                    let stroke = wall;
                    // As the game draws the chunk you stand in: walls rising from the land, lined
                    // every two blocks along and four up.
                    self.street.chunk_walls(&p.with_clip_rect(frame.rect), img, &cam, bx.div_euclid(16) as i32, bz.div_euclid(16) as i32, stroke, 2, 4, world_top(&self.dimension), self.land_outline);
                }
                frame.response.hover_pos().unwrap_or(frame.rect.center())
            } else {
                // The chunk under the pointer: on the land in the 3D view, else a square.
                let (cx, cz) = ((h.x / 16.0).floor(), (h.y / 16.0).floor());
                let inside = p.with_clip_rect(frame.rect);
                match (self.camera, &self.image) {
                    (Some(cam), Some((img, _))) => {
                        if show_chunk {
                            self.street.chunk_walls(&inside, img, &cam, cx as i32, cz as i32, wall, 2, 4, world_top(&self.dimension), self.land_outline);
                        }
                    }
                    _ => {
                        // Its four corners as they fall on the screen (the map may be turned), a
                        // light line inside a dark one so it shows on any ground.
                        let corners: Vec<Pos2> = [(0.0, 0.0), (16.0, 0.0), (16.0, 16.0), (0.0, 16.0)].iter().map(|(dx, dz)| frame.to_screen(vec2(cx * 16.0 + dx, cz * 16.0 + dz))).collect();
                        inside.add(egui::Shape::convex_polygon(corners.clone(), Color32::from_white_alpha(28), Stroke::NONE));
                        let mut ring = corners.clone();
                        ring.push(corners[0]);
                        inside.add(egui::Shape::line(ring.clone(), Stroke::new(3.5_f32, Color32::from_black_alpha(200))));
                        inside.add(egui::Shape::line(ring, Stroke::new(1.5_f32, Color32::from_rgb(245, 245, 245))));
                    }
                }
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
                s
            };
            let block = match picked {
                Some(k) => format!("{}, {}, {}", k.x, k.y, k.z),
                None => format!("{bx}, {bz}"),
            };
            let mut text = format!("Chunk:  {}, {}\nPosition:  {block}",bx.div_euclid(16), bz.div_euclid(16));
            if let Some((img, _)) = &self.image {
                if let Some(b) = atlas::biome_at(img, bx as i32, bz as i32) {
                    text.push_str(&format!("\nBiome:  {}", pretty(&b)));
                }
                // The block highlighted itself (read from the real blocks where they are loaded);
                // else the top block of the column.
                let real = picked.and_then(|k| self.gpu.as_ref()?.block_at(k.x, k.y, k.z));
                let id = real.or_else(|| {
                    let (x, z) = (bx as i32 - img.min_x * 16, bz as i32 - img.min_z * 16);
                    (x >= 0 && z >= 0 && (x as usize) < img.width && (z as usize) < img.height).then(|| {
                        let i = z as usize * img.width + x as usize;
                        if img.over[i] != 0 { img.over[i] } else { img.blocks[i] }
                    })
                });
                if let Some(n) = id.filter(|_| picked.is_some()).and_then(world::block_name) {
                    let label = if real.is_some() { "Block" } else { "Top" };
                    text.push_str(&format!("\n{label}:  {}", pretty(n.split(['#', '@']).next().unwrap_or(&n))));
                    // Its block state, a property a line (as the game's F3 screen lists them).
                    if let Some(props) = real.and_then(world::block_props) {
                        for kv in props.split(',').filter(|s| !s.is_empty()) {
                            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                            text.push_str(&format!("\n    {k}:  {v}"));
                        }
                    }
                }
            }
            // Shown in a card in the map's corner (above the stage tag), out of the pointer's way.
            let ks = ((frame.rect.width() / 730.0).clamp(0.62, 1.0) * 50.0).round() / 50.0;
            let galley = p.layout_no_wrap(text, theme::regular(13.0 * ks), theme::text());
            let size = galley.size() + vec2(16.0, 10.0) * ks;
            let r = Rect::from_min_size(pos2(frame.rect.left() + 10.0 * ks, frame.rect.bottom() - 34.0 * ks - size.y), size);
            p.rect_filled(
                r,
                theme::radius(4),
                theme::deep_alpha(235),
            );
            p.galley(r.min + vec2(8.0, 5.0), galley, theme::text());
        }

        // Everything on the controls is drawn this much smaller in a narrow window.
        let ks = ((frame.rect.width() / 730.0).clamp(0.62, 1.0) * 50.0).round() / 50.0;
        self.ui_k = ks;
        // Icons in the top-left corner that fold out: Search (going to a place) and View
        // (the view's options); in 3D, the way back to Top-down beside them.
        let search_icon = Rect::from_min_size(frame.rect.min + vec2(22.0 * ks, 22.0 * ks), Vec2::splat(44.0 * ks));
        let search_open = self.fold_button(ui, 0, search_icon, "Search: go to a place, or find blocks and mobs", ks);
        icons::paint(ui.painter(), Rect::from_center_size(search_icon.center(), Vec2::splat(20.0 * ks)), Icon::Search, theme::text());
        let view_icon = search_icon.translate(vec2(50.0 * ks, 0.0));
        let view_open = self.fold_button(ui, 1, view_icon, "View options", ks);
        icons::paint(ui.painter(), Rect::from_center_size(view_icon.center(), Vec2::splat(20.0 * ks)), Icon::Gear, theme::text());
        let mut corner_icons = vec![search_icon, view_icon];
        if frame.street {
            let r = Rect::from_min_size(view_icon.right_top() + vec2(6.0 * ks, 0.0), vec2(124.0 * ks, 44.0 * ks));
            let resp = ui.interact(r, ui.id().with("street-back"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Back to the Top-down view (or scroll out)");
            ui.memory_mut(|m| m.surrender_focus(resp.id));
            ui.painter().rect_filled(r, theme::radius(5), if resp.hovered() { theme::button_hover() } else { theme::deep() });
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{2B07}  Top-down", theme::regular(14.0 * ks), theme::text());
            if resp.clicked() {
                self.go_top_down();
            }
            corner_icons.push(r);
        }
        // The Locate bar (Search folded out), under the icons.
        let bar = Rect::from_min_size(pos2(search_icon.left(), search_icon.bottom() + 6.0 * ks), vec2(384.0 * ks, 44.0 * ks));
        if search_open {
        self.fold_panel = Some(bar);
        ui.painter().rect_filled(bar, theme::radius(5), theme::deep());
        // Every part is the same height; a labelled part is a lighter cell beside a dark field.
        let cell = theme::cell();
        let field = theme::well();
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
            Stroke::new(1.3_f32 * ks, theme::strong()),
            StrokeKind::Inside,
        );
        ui.painter().text(
            pos2(r.left() + 28.0 * ks + 25.0 * ks, r.center().y),
            Align2::CENTER_CENTER,
            if self.chunk_mode { "Chunk" } else { "Block" },
            theme::regular(14.0 * ks),
            theme::strong(),
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
                theme::strong(),
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
            if resp.hovered() { theme::button_hover() } else { theme::button() },
        );
        ui.painter().rect_stroke(
            r,
            theme::radius(5),
            Stroke::new(1.0_f32 * ks, theme::border()),
            StrokeKind::Inside,
        );
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(r.left() + 20.0 * ks, r.center().y), Vec2::splat(16.0 * ks)),
            Icon::Search,
            theme::strong(),
        );
        ui.painter().text(
            pos2(r.left() + 40.0 * ks, r.center().y),
            Align2::LEFT_CENTER,
            "Locate",
            theme::regular(14.0 * ks),
            theme::strong(),
        );
        if resp.clicked() {
            let k = if self.chunk_mode { 16.0 } else { 1.0 };
            self.center = vec2(
                self.locate.0 as f32 * k + if self.chunk_mode { 8.0 } else { 0.5 },
                self.locate.1 as f32 * k + if self.chunk_mode { 8.0 } else { 0.5 },
            );
            self.zoom = self.zoom.max(2.0);
            self.momentum = Vec2::ZERO;
            self.pending_zoom = 0.0;
            if self.chunk_mode {
                self.selected = vec![(self.locate.0 as i32, self.locate.1 as i32)];
            }
        }
        }
        let bar = if search_open {
            let find = self.find_panel(ui, pos2(bar.left(), bar.bottom() + 6.0 * ks), bar.width(), ks);
            let both = bar.union(find);
            self.fold_panel = Some(both);
            both
        } else {
            Rect::NOTHING
        };
        // Top right: the Slicer, and the dimension shown (an icon that folds out the three).
        let slicer_rect = Rect::from_min_size(
            pos2(frame.rect.right() - 118.0 * ks, frame.rect.top() + 22.0 * ks),
            vec2(96.0 * ks, 44.0 * ks),
        );
        let dims_icon = Rect::from_min_size(pos2(slicer_rect.left() - 50.0 * ks, slicer_rect.top()), Vec2::splat(44.0 * ks));
        let dims_open = self.fold_button(ui, 2, dims_icon, "Dimension: Overworld, Nether or The End", ks);
        let now = DIMS.iter().position(|d| self.dimension == *d).unwrap_or(0);
        dim_tile(ui.painter(), Rect::from_center_size(dims_icon.center(), Vec2::splat(30.0 * ks)), now);
        corner_icons.push(dims_icon);
        let dims_rect = Rect::from_min_size(pos2(dims_icon.right() - 158.0 * ks, dims_icon.bottom() + 6.0 * ks), vec2(158.0 * ks, 44.0 * ks));
        let dims_rect = if dims_open { dims_rect } else { Rect::NOTHING };
        if dims_open {
        self.fold_panel = Some(dims_rect);
        ui.painter()
            .rect_filled(dims_rect, theme::radius(5), theme::deep());
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
                    .rect_filled(r, theme::radius(4), theme::button_hover());
            }
            if selected {
                ui.painter().rect_filled(r, theme::radius(4), theme::card());
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
                self.fold = None;
            }
        }
        }
        // The Slicer: a button that opens a panel with a track and two handles, as in the
        // original tool. Only blocks between the handles are drawn.
        let border = theme::border();
        let open = self.slicer_open;
        // As tall as fits: down to the bottom of the map, or above the compass when it shows.
        let compass_room = if frame.street || frame.yaw != 0.0 { 84.0 * ks } else { 16.0 * ks };
        let panel_h = (frame.rect.bottom() - compass_room - slicer_rect.top()).clamp(170.0 * ks, 552.0 * ks);
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
            ui.painter().rect_filled(panel, theme::radius(6), theme::bg());
            ui.painter().rect_filled(
                header,
                egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 },
                theme::deep(),
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
                if resp.hovered() { theme::button_hover() } else { theme::deep() },
            );
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(header.left_center() + vec2(22.0 * ks, 0.0), Vec2::splat(22.0 * ks)),
            Icon::Layers,
            theme::text(),
        );
        ui.painter().text(
            header.left_center() + vec2(40.0 * ks, 0.0),
            Align2::LEFT_CENTER,
            "Slicer",
            theme::regular(14.0 * ks),
            theme::text(),
        );
        if resp.clicked() {
            self.slicer_open = !self.slicer_open;
        }
        if open {
            self.slicer_panel(ui, panel, header);
        }
        // Under the Locate bar: the map style on the flat map; in the street view a compass
        // (click to face north) and the way back to top-down.
        let mut extra = Vec::new();
        if frame.street || frame.yaw != 0.0 {
            let k = ks;
            let compass = Rect::from_center_size(frame.rect.right_bottom() - vec2(44.0 * k, 44.0 * k), Vec2::splat(52.0 * k));
            let resp = ui
                .interact(compass, ui.id().with("street-compass"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Face north (N)");
            let c = compass.center();
            ui.painter().circle_filled(c, 26.0 * k, if resp.hovered() { theme::button_hover() } else { theme::deep() });
            ui.painter().circle_stroke(c, 26.0 * k, Stroke::new(1.0 * k, theme::border()));
            // North on screen: the needle turns with the camera.
            let (s, co) = self.yaw.sin_cos();
            let dir = vec2(-s, -co);
            let side = vec2(dir.y, -dir.x);
            let tip = c + dir * 18.0 * k;
            let tail = c - dir * 18.0 * k;
            ui.painter().add(egui::Shape::convex_polygon(vec![tip, c + side * 6.0 * k, c - side * 6.0 * k], Color32::from_rgb(220, 70, 60), Stroke::NONE));
            ui.painter().add(egui::Shape::convex_polygon(vec![tail, c - side * 6.0 * k, c + side * 6.0 * k], theme::text_muted(), Stroke::NONE));
            if resp.clicked() {
                self.yaw = 0.0;
            }
            extra.push(compass);
        }
        if frame.street {
            let hint = if self.noclip {
                "WASD to fly \u{b7} Space / Ctrl up and down \u{b7} Right-drag to look \u{b7} Scroll to zoom"
            } else {
                "Drag to move \u{b7} Right-drag to turn \u{b7} Scroll to zoom"
            };
            ui.painter().text(
                pos2(frame.rect.center().x, frame.rect.bottom() - 14.0),
                Align2::CENTER_BOTTOM,
                hint,
                theme::regular(12.0),
                theme::deep_alpha(200),
            );
        }
        // A row of buttons under the Locate bar: back to Top-down (in 3D), the chunk grid, the
        // line along the land (in the 3D views) and the free camera (in 3D).
        // Laid out left to right under the Locate bar, onto a new row when the next does not fit.
        // (All in the View panel, folded out under its icon.)
        let pad = 8.0 * ks;
        let (left, limit) = (view_icon.left() + pad, (frame.rect.right() - 12.0 * ks - pad).min(view_icon.left() + 560.0 * ks));
        let mut cursor = (left, view_icon.bottom() + 6.0 * ks + pad);
        let panel_bg = ui.painter().add(egui::Shape::Noop);
        let extra_from = extra.len();
        let pill = |ui: &mut Ui, r: Rect, key: &str, on: bool, tip: &str| {
            let resp = ui.interact(r, ui.id().with(key), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
            // Never holding the keyboard, so Space (the free camera's up) cannot press it.
            ui.memory_mut(|m| m.surrender_focus(resp.id));
            ui.painter().rect_filled(r, theme::radius(5), if resp.hovered() { theme::button_hover() } else { theme::deep() });
            if on {
                ui.painter().rect_stroke(r, theme::radius(5), Stroke::new(1.2_f32 * ks, theme::accent()), StrokeKind::Inside);
            }
            resp
        };
        let mut place = |w: f32| {
            let w = w * ks;
            if cursor.0 + w > limit && cursor.0 > left {
                cursor = (left, cursor.1 + 38.0 * ks);
            }
            let r = Rect::from_min_size(pos2(cursor.0, cursor.1), vec2(w, 32.0 * ks));
            cursor.0 = r.right() + 6.0 * ks;
            r
        };
        if view_open {
        let r = place(112.0);
        let resp = pill(ui, r, "chunk-grid", self.chunk_grid, "Show the chunk borders (as the game's F3+G)");
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "Chunk Grid", theme::regular(14.0 * ks), if self.chunk_grid { theme::text() } else { theme::text_muted() });
        if resp.clicked() {
            self.chunk_grid = !self.chunk_grid;
        }
        extra.push(r);
        if self.camera.is_some() {
            let r = place(128.0);
            let resp = pill(ui, r, "land-outline", self.land_outline, "Also draw each chunk's edge along the top of the land (in cyan)");
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "Land Outline", theme::regular(14.0 * ks), if self.land_outline { theme::text() } else { theme::text_muted() });
            if resp.clicked() {
                self.land_outline = !self.land_outline;
            }
            extra.push(r);
        }
        // In 3D: the cards of what mobs and containers hold, and mobs' name tags.
        if frame.street && self.gpu.is_some() {
            let r = place(124.0);
            let resp = pill(ui, r, "show-details", self.show_details, "Show what mobs and containers hold when pointed at, and named mobs' name tags");
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "Show Details", theme::regular(14.0 * ks), if self.show_details { theme::text() } else { theme::text_muted() });
            if resp.clicked() {
                self.show_details = !self.show_details;
            }
            extra.push(r);
        }
        // On the GPU: the light (Full Bright to see into dark places) and how far the blocks reach,
        // on a second row.
        if self.gpu.is_some() && self.camera.is_some() {
            let r = place(116.0);
            let resp = pill(ui, r, "full-bright", self.full_bright, "Draw everything evenly lit, to see into caves and other dark places (off: lit by the sky and glowing blocks, as in the game)");
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "Full Bright", theme::regular(14.0 * ks), if self.full_bright { theme::text() } else { theme::text_muted() });
            if resp.clicked() {
                self.full_bright = !self.full_bright;
            }
            extra.push(r);
            let r = place(150.0);
            let resp = pill(ui, r, "time-of-day", false, "The time of day the light is for: the world's own (as saved), day or night");
            let label = match self.time_of_day {
                TimeOfDay::World => format!("Time: World ({})", if self.daylight() < 0.6 { "Night" } else { "Day" }),
                TimeOfDay::Day => "Time: Day".to_string(),
                TimeOfDay::Sunset => "Time: Sunset".to_string(),
                TimeOfDay::Night => "Time: Night".to_string(),
            };
            ui.painter().text(r.center(), Align2::CENTER_CENTER, label, theme::regular(14.0 * ks), theme::text());
            if resp.clicked() {
                self.time_of_day = match self.time_of_day {
                    TimeOfDay::World => TimeOfDay::Day,
                    TimeOfDay::Day => TimeOfDay::Sunset,
                    TimeOfDay::Sunset => TimeOfDay::Night,
                    TimeOfDay::Night => TimeOfDay::World,
                };
            }
            extra.push(r);
            let r = place(128.0);
            let resp = pill(ui, r, "render-distance", false, "How many chunks around the camera are drawn as blocks (click for more; low-detail land goes on beyond)");
            ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("Distance {}", self.render_distance), theme::regular(14.0 * ks), theme::text());
            if resp.clicked() {
                const STEPS: [i32; 3] = [32, 48, 64];
                let next = STEPS.iter().position(|s| *s == self.render_distance).map_or(0, |i| (i + 1) % STEPS.len());
                self.render_distance = STEPS[next];
            }
            extra.push(r);
        }
        // In 3D: the pointer stops at water and lava too, to select them.
        if frame.street && self.gpu.is_some() {
            let r = place(128.0);
            let resp = pill(ui, r, "pick-liquids", self.pick_liquids, "Let the pointer stop at water and lava, to select them (off: it goes through to what is under them)");
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "Water/Lava", theme::regular(14.0 * ks), if self.pick_liquids { theme::text() } else { theme::text_muted() });
            if resp.clicked() {
                self.pick_liquids = !self.pick_liquids;
            }
            extra.push(r);
        }
        // With the blocks shown: barriers (by their icon, as the game shows them), or not.
        if self.gpu.is_some() && self.camera.is_some() {
            let r = place(108.0);
            let resp = pill(ui, r, "barriers", self.show_barriers, "Show barrier blocks by their icon, and let the pointer select them (off: they are passed through as air)");
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "Barriers", theme::regular(14.0 * ks), if self.show_barriers { theme::text() } else { theme::text_muted() });
            if resp.clicked() {
                self.show_barriers = !self.show_barriers;
            }
            extra.push(r);
        }
        if frame.street || self.camera.is_some() {
            let r = place(128.0);
            let resp = pill(ui, r, "noclip", self.noclip, "Free camera, as in a 3D editor: goes into the 3D view up close, flies anywhere (through blocks too) and back to Top-down when turned off.\nWASD to move, Space to go up, Ctrl to go down, right-drag to look.");
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "Free Camera", theme::regular(14.0 * ks), if self.noclip { theme::text() } else { theme::text_muted() });
            if resp.clicked() {
                self.set_free_camera(!self.noclip, frame.rect);
            }
            extra.push(r);
        }
        if self.camera.is_some() {
            let r = place(150.0);
            let resp = pill(ui, r, "chunk-highlight", self.chunk_highlight, "Outline the chunk under the pointer (comes back on when you go back to the flat map)");
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "Chunk Highlight", theme::regular(14.0 * ks), if self.chunk_highlight { theme::text() } else { theme::text_muted() });
            if resp.clicked() {
                self.chunk_highlight = !self.chunk_highlight;
            }
            extra.push(r);
            let r = place(104.0);
            let resp = pill(ui, r, "overview", self.overview, "A small map from above showing where the camera is, which way it faces and the ground in view (click it to move there)");
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "Overview", theme::regular(14.0 * ks), if self.overview { theme::text() } else { theme::text_muted() });
            if resp.clicked() {
                self.overview = !self.overview;
            }
            extra.push(r);
        }
        }
        // The View panel's background, behind its buttons.
        if view_open && extra.len() > extra_from {
            let mut r = extra[extra_from];
            for e in &extra[extra_from..] {
                r = r.union(*e);
            }
            let r = r.expand(pad);
            ui.painter().set(panel_bg, egui::Shape::rect_filled(r, theme::radius(6), theme::deep_alpha(235)));
            self.fold_panel = Some(r);
            extra.push(r);
        }
        if self.camera.is_some() && self.overview {
            if let Some(r) = self.overview_inset(ui, frame, ks) {
                extra.push(r);
            }
        }
        extra.extend(corner_icons);
        // Which stage the map is showing, small in the corner: the Biome Atlas or the shaded
        // Terrain far out, the Top-down 3D view zoomed in, and the tilted 3D view.
        let stage = if frame.street {
            "3D"
        } else if self.camera.is_some() {
            "Top-down"
        } else if frame.zoom < (ATLAS_FULL + ATLAS_END) * 0.5 {
            "Biomes"
        } else {
            "Terrain"
        };
        let tag = ui.painter().layout_no_wrap(stage.to_string(), theme::regular(11.0), theme::text_muted());
        let at = pos2(frame.rect.left() + 8.0, frame.rect.bottom() - 8.0 - tag.size().y);
        let back = Rect::from_min_size(at - vec2(5.0, 2.0), tag.size() + vec2(10.0, 4.0));
        ui.painter().rect_filled(back, theme::radius(3), theme::deep_alpha(190));
        ui.painter().galley(at, tag, theme::text_muted());
        // (What is selected, the brush's and the paste's settings: in the World Explorer's toolbar.)
        let pointer = ui.input(|i| i.pointer.hover_pos());
        self.over_controls = pointer.is_some_and(|p| {
            [bar, dims_rect, panel].iter().chain(extra.iter()).any(|r| r.expand(2.0).contains(p))
        });        if to_gpu {
            let lines = crate::street_view::take_lines();
            if let Some(w) = &self.gpu {
                w.set_lines(lines);
            }
        } else if let Some(w) = &self.gpu {
            w.set_lines(Vec::new());
        }
        // The map's buttons never keep the keyboard (Space, the free camera's up, would press them).
        let base = ui.id();
        ui.memory_mut(|m| {
            for key in ["loc-mode", "loc-go", "slicer", "street-compass"] {
                m.surrender_focus(base.with(key));
            }
            for i in 0..DIMS.len() {
                m.surrender_focus(base.with(("dim", i)));
            }
        });
        // How far the lens is zoomed in, while it is.
        if frame.street && self.lens > 1.01 {
            let text = format!("Zoom {:.1}\u{d7}", self.lens);
            let galley = ui.painter().layout_no_wrap(text, theme::regular(13.0), theme::text());
            let at = pos2(frame.rect.center().x - galley.size().x / 2.0, frame.rect.top() + 14.0);
            let back = Rect::from_min_size(at - vec2(10.0, 5.0), galley.size() + vec2(20.0, 10.0));
            ui.painter().rect_filled(back, theme::radius(5), theme::deep_alpha(210));
            ui.painter().galley(at, galley, theme::text());
        }
        if self.job.is_some() {
            ui.painter().text(
                frame.rect.right_bottom() - vec2(12.0, 10.0),
                Align2::RIGHT_BOTTOM,
                "Rendering…",
                theme::regular(12.0),
                theme::text_muted(),
            );
        }
    }

    /// The open Slicer: upper and lower Y boxes around a track with a handle for each limit.
    fn slicer_panel(&mut self, ui: &mut Ui, panel: Rect, header: Rect) {
        let ks = self.ui_k;
        let cell = theme::cell();
        let handle = Color32::from_rgb(45, 194, 115);
        let mut changed = false;
        let x0 = panel.left() + 9.0 * ks;
        let top_box = Rect::from_min_size(pos2(x0, header.bottom() + 12.0 * ks), vec2(75.0 * ks, 27.0 * ks));
        let bottom_box = Rect::from_min_size(pos2(x0, panel.bottom() - 12.0 * ks - 27.0 * ks), vec2(75.0 * ks, 27.0 * ks));
        let track = Rect::from_min_max(
            pos2(x0, top_box.bottom() + 10.0 * ks),
            pos2(x0 + 47.0 * ks, bottom_box.top() - 10.0 * ks),
        );
        // Pixels along the track for its 384 heights, and the least the handles may come apart
        // (in heights) so they never sit on each other.
        let span = (track.height() - 26.0 * ks).max(1.0 * ks);
        let gap = 1;
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
                theme::deep(),
            );
            ui.painter().text(
                pos2(r.left() + 11.0 * ks, r.center().y),
                Align2::CENTER_CENTER,
                "Y",
                theme::regular(14.0 * ks),
                theme::strong(),
            );
            let (lo, hi) = if upper { ((self.slicer_min + gap).min(320), 320) } else { (-64, (self.slicer - gap).max(-64)) };
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
        let y_of = |v: i32| track.top() + 13.0 * ks + (320 - v) as f32 / 384.0 * span;
        let v_of = |y: f32| {
            (320.0 - (y - track.top() - 13.0 * ks) / span * 384.0)
                .round()
                .clamp(-64.0, 320.0) as i32
        };
        ui.painter().rect_filled(track, theme::radius(6), theme::deep());
        for v in [320, 256, 128, 64, 0, -64] {
            let y = y_of(v);
            if ![320, -64].contains(&v) {
                ui.painter().line_segment(
                    [pos2(track.center().x - 14.0 * ks, y), pos2(track.center().x + 14.0 * ks, y)],
                    Stroke::new(1.0_f32 * ks, theme::text_dim()),
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
            // A handle stops against the other one.
            if upper {
                me.slicer = v.max(me.slicer_min + gap).min(320);
            } else {
                me.slicer_min = v.min(me.slicer - gap).max(-64);
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

// ---------------------------------------------------------------------------------------------
// What a container or mob holds, shown by the pointer in the 3D view
// ---------------------------------------------------------------------------------------------

/// Painting with the brush (see `MapView::brush`).
#[derive(Clone, Debug)]
pub struct BrushMode {
    /// Blocks from the middle to the edge.
    pub radius: i32,
    pub sphere: bool,
    /// Paints air instead of the block chosen.
    pub erase: bool,
    /// Its middle across is the middle of the chunk pointed at (its height, the block's).
    pub snap: bool,
}

impl Default for BrushMode {
    fn default() -> Self {
        BrushMode { radius: 2, sphere: true, erase: false, snap: false }
    }
}

/// The outline of an entity's model on screen, in `color`: its quads filled into a small mask
/// over where it is, then the cells just outside the mask's edge drawn (two cells thick), so
/// it is the outline of the whole figure, not of each part. Falls back to its box for an
/// entity with no model here.
fn entity_outline(p: &egui::Painter, cam: &Camera, e: &mctk_nbt::Tag, bounds: Rect, color: Color32) {
    let mut polys: Vec<[Pos2; 4]> = Vec::new();
    if let Some((quads, _)) = crate::entities::entity(e, [0.0, 0.0, 0.0]) {
        for q in &quads {
            let pts: Option<Vec<Pos2>> = q.pos.iter().map(|c| cam.project(*c).map(|(s, _)| s)).collect();
            if let Some(pts) = pts {
                polys.push([pts[0], pts[1], pts[2], pts[3]]);
            }
        }
    }
    if polys.is_empty() {
        // No model: the box the game gives it.
        let Some(at) = e.get("Pos").and_then(mctk_nbt::Tag::as_list).map(|l| (0..3).map(|i| l.get(i).and_then(mctk_nbt::Tag::as_f64).unwrap_or(0.0) as f32).collect::<Vec<_>>()) else { return };
        let (r, h) = crate::gpu_world::entity_size(e);
        Street::selection(p, cam, [at[0] - r, at[1], at[2] - r], [at[0] + r, at[1] + h, at[2] + r], color);
        return;
    }
    let mut bb = Rect::NOTHING;
    for q in &polys {
        for s in q {
            bb.extend_with(*s);
        }
    }
    let bb = bb.expand(6.0).intersect(bounds);
    if !(bb.width() > 1.0 && bb.height() > 1.0) {
        return;
    }
    // About a pixel and a half a cell, fewer cells for one that fills the screen.
    let cell = (bb.width().max(bb.height()) / 320.0).max(1.5);
    let (w, h) = ((bb.width() / cell).ceil() as usize + 1, (bb.height() / cell).ceil() as usize + 1);
    let mut mask = vec![false; w * h];
    for q in &polys {
        let mut qb = Rect::NOTHING;
        for s in q {
            qb.extend_with(*s);
        }
        let (x0, x1) = (((qb.left() - bb.left()) / cell).floor().max(0.0) as usize, (((qb.right() - bb.left()) / cell).ceil() as usize).min(w - 1));
        let (y0, y1) = (((qb.top() - bb.top()) / cell).floor().max(0.0) as usize, (((qb.bottom() - bb.top()) / cell).ceil() as usize).min(h - 1));
        // Inside a convex quad (either winding): on the same side of every edge.
        let side = |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let c = pos2(bb.left() + (x as f32 + 0.5) * cell, bb.top() + (y as f32 + 0.5) * cell);
                let s: Vec<f32> = (0..4).map(|k| side(q[k], q[(k + 1) % 4], c)).collect();
                if s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0) {
                    mask[y * w + x] = true;
                }
            }
        }
    }
    let inside = |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && mask[y as usize * w + x as usize];
    let mut mesh = egui::Mesh::default();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if inside(x, y) {
                continue;
            }
            let near = (-2..=2).any(|dy: i64| (-2..=2).any(|dx: i64| (dx * dx + dy * dy) <= 5 && inside(x + dx, y + dy)));
            if near {
                let at = pos2(bb.left() + x as f32 * cell, bb.top() + y as f32 * cell);
                mesh.add_colored_rect(Rect::from_min_size(at, Vec2::splat(cell)), color);
            }
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

/// The height on the upright line through (x, z) = `at` nearest the ray from `eye` along `dir`
/// (what height the pointer is at, above or beside a box's floor, over air too).
fn height_on_line(eye: [f32; 3], dir: [f32; 3], at: [f32; 2]) -> f32 {
    // The line: (at.x, s, at.z), up; the ray: eye + t dir.
    let w0 = [at[0] - eye[0], -eye[1], at[1] - eye[2]];
    let b = dir[1];
    let c = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
    let d = w0[1];
    let e = dir[0] * w0[0] + dir[1] * w0[1] + dir[2] * w0[2];
    let denom = c - b * b;
    if denom.abs() < 1e-6 {
        return eye[1];
    }
    (b * e - c * d) / denom
}

#[cfg(test)]
mod outline_tests {
    use super::*;

    /// A zombie seen from the side is outlined as one figure: a ring of cells around it, the
    /// middle of its body left clear.
    #[test]
    fn a_mob_is_outlined_as_one_figure() {
        let mut zombie = mctk_nbt::Compound::new();
        zombie.insert("id".into(), mctk_nbt::Tag::String("minecraft:zombie".into()));
        let pos = mctk_nbt::List::from_tags(mctk_nbt::TagType::Double, vec![mctk_nbt::Tag::Double(0.5), mctk_nbt::Tag::Double(64.0), mctk_nbt::Tag::Double(0.5)]).unwrap();
        zombie.insert("Pos".into(), mctk_nbt::Tag::List(pos));
        let zombie = mctk_nbt::Tag::Compound(zombie);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0));
        let cam = Camera { target: [0.5, 65.0, 0.5], yaw: 0.0, pitch: 0.0, dist: 6.0, rect, fov: crate::street_view::FOV_Y };
        let ctx = egui::Context::default();
        let mut cells = Vec::new();
        let out = ctx.run(egui::RawInput { screen_rect: Some(rect), ..Default::default() }, |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            entity_outline(&p, &cam, &zombie, rect, Color32::WHITE);
        });
        for s in out.shapes {
            if let egui::epaint::Shape::Mesh(m) = s.shape {
                cells.extend(m.vertices.chunks(4).map(|v| v[0].pos));
            }
        }
        assert!(cells.len() > 100, "an outline drawn ({} cells)", cells.len());
        let (top, bottom) = cells.iter().fold((f32::MAX, f32::MIN), |(t, b), p| (t.min(p.y), b.max(p.y)));
        let mid = cam.project([0.5, 65.0, 0.5]).unwrap().0;
        assert!(bottom - top > 150.0, "as tall as the zombie ({top}..{bottom})");
        assert!(!cells.iter().any(|c| c.distance(mid) < 10.0), "the middle of the body is not drawn over");
    }
}

#[cfg(test)]
mod height_tests {
    /// Looking at an upright line from the side: the height pointed at is where the ray
    /// passes it, above the eye, below it, or level.
    #[test]
    fn the_height_pointed_at_on_an_upright_line() {
        let eye = [0.0, 70.0, -10.0];
        for want in [60.0f32, 70.0, 95.0] {
            let d = [0.0, want - 70.0, 10.0];
            let n = (d[1] * d[1] + d[2] * d[2]).sqrt();
            let got = super::height_on_line(eye, [0.0, d[1] / n, d[2] / n], [0.0, 0.0]);
            assert!((got - want).abs() < 1e-3, "{got} vs {want}");
        }
    }
}

/// Placing the clipboard (see `MapView::paste`).
#[derive(Clone, Debug, Default)]
pub struct PasteMode {
    pub place: crate::edit::transform::Placement,
    /// How far the arrow keys moved it from under the pointer.
    pub nudge: [i32; 3],
    /// Its lowest north-west corner, once the pointer was over the land.
    pub at: Option<[i32; 3]>,
    /// Air in the clipboard is pasted too (else what is there stays).
    pub with_air: bool,
}

/// One slot of the card: what is in it, and the game's picture for it while empty.
struct CardSlot {
    item: Option<(String, i64)>,
    empty: Option<&'static str>,
}

/// A card of what something holds: its name (and what it is, when named), and rows of slots
/// laid out `cols` across.
struct Card {
    title: String,
    named: bool,
    kind: Option<String>,
    /// A mob's health (and the most it can have), armor points and status effects.
    stats: Option<CardStats>,
    /// Lines of text under the title: what a spawner spawns, what a potion does.
    lines: Vec<String>,
    grids: Vec<(usize, Vec<CardSlot>)>,
}

struct CardStats {
    health: f64,
    max: Option<f64>,
    armor: i64,
    effects: Vec<crate::mob_edit::Effect>,
}

/// An effect's level and time as the game's inventory shows them: "Speed II", "1:30".
fn effect_text(e: &crate::mob_edit::Effect) -> (String, String) {
    (crate::effects::effect_name(e), crate::effects::time_text(e.duration))
}

/// The slots of an item list: by each stack's `Slot` where it has one (from `first`), else in
/// order; `size` slots at least.
fn list_slots(list: Option<&mctk_nbt::Tag>, size: usize, first: i64) -> Vec<CardSlot> {
    let mut placed: Vec<(usize, (String, i64))> = Vec::new();
    for (k, t) in list.and_then(mctk_nbt::Tag::as_list).into_iter().flat_map(|l| l.iter()).enumerate() {
        let Some(c) = t.as_compound() else { continue };
        let Some(stack) = crate::mob_edit::stack_of(c) else { continue };
        let at = c.get("Slot").and_then(mctk_nbt::Tag::as_i64).map_or(k as i64, |s| s - first);
        if at >= 0 {
            placed.push((at as usize, stack));
        }
    }
    let n = placed.iter().map(|(at, _)| at + 1).max().unwrap_or(0).max(size);
    let mut out: Vec<CardSlot> = (0..n).map(|_| CardSlot { item: None, empty: None }).collect();
    for (at, s) in placed {
        out[at].item = Some(s);
    }
    out
}

/// How many slots a container block has, and how many across (by its block entity's id).
fn container_shape(id: &str) -> (usize, usize) {
    let n = id.trim_start_matches("minecraft:");
    match n {
        "hopper" => (5, 5),
        "dispenser" | "dropper" | "crafter" => (9, 3),
        "furnace" | "smoker" | "blast_furnace" => (3, 3),
        "brewing_stand" => (5, 5),
        "chiseled_bookshelf" => (6, 3),
        "campfire" | "soul_campfire" => (4, 4),
        "decorated_pot" | "jukebox" | "lectern" => (1, 1),
        _ => (27, 9),
    }
}

impl MapView {
    /// What the mob or container under the pointer holds, if anything.
    fn contents_card(&self) -> Option<Card> {
        let w = self.gpu.as_ref()?;
        if let Some(e) = &self.hover_entity {
            let c = e.as_compound()?;
            let id = mctk_convert::view::java_entity_id(c).unwrap_or_default();
            let name = crate::naming::mob_name(c);
            let mut grids = Vec::new();
            // What it wears and holds (living things and armor stands).
            if c.contains_key("Health") || id.ends_with("armor_stand") {
                let gear: Vec<CardSlot> = crate::mob_edit::equipment(c, None).into_iter().map(|(s, item)| CardSlot { item, empty: Some(s.empty_picture()) }).collect();
                if !gear.is_empty() {
                    grids.push((gear.len(), gear));
                }
            }
            // Its own inventory (villagers, piglins, allays) and a chest it carries (donkeys,
            // minecarts with chests: from slot 2 for the animals, past their saddle and armor).
            if c.contains_key("Inventory") {
                grids.push((9, list_slots(c.get("Inventory"), 0, 0)));
            }
            if c.contains_key("Items") {
                let first = if id.ends_with("minecart") || id.ends_with("boat") || id.ends_with("raft") { 0 } else { 2 };
                grids.push((9, list_slots(c.get("Items"), if first == 0 { 27 } else { 0 }, first)));
            }
            // An item frame's or a dropped item's one stack (and what it does, for a potion).
            let mut lines = Vec::new();
            if let Some(item) = c.get("Item").and_then(mctk_nbt::Tag::as_compound) {
                grids.push((1, vec![CardSlot { item: crate::mob_edit::stack_of(item), empty: None }]));
                lines = crate::effects::potion_lines(item);
            }
            grids.retain(|(_, g)| !g.is_empty());
            // A living thing's health, armor and status effects.
            let stats = crate::mob_edit::health(c).map(|health| CardStats {
                health,
                max: crate::mob_edit::max_health(c, &id),
                armor: crate::mob_edit::armor_points(c, &id, None),
                effects: crate::mob_edit::effects(c),
            });
            if grids.is_empty() && name.is_none() && stats.is_none() {
                return None;
            }
            return Some(Card { title: name.clone().unwrap_or_else(|| pretty(&id)), named: name.is_some(), kind: name.map(|_| pretty(&id)), stats, lines, grids });
        }
        let h = self.picked?;
        let be = w.block_entity_at(h.x, h.y, h.z)?;
        let c = be.as_compound()?;
        // A spawner: the mob it spawns (and the others it may pick from), how many at a time.
        if crate::mob_edit::is_spawner(c) {
            let mut lines = vec![match crate::mob_edit::spawner_mob(c) {
                Some(m) => format!("Spawns {}", pretty(&m)),
                None => "Spawns nothing".to_string(),
            }];
            let others: Vec<String> = c
                .get("SpawnPotentials")
                .and_then(mctk_nbt::Tag::as_list)
                .into_iter()
                .flat_map(|l| l.iter())
                .filter_map(|p| {
                    let p = p.as_compound()?;
                    let data = p.get("data").or_else(|| p.get("Entity")).and_then(mctk_nbt::Tag::as_compound)?;
                    let e = data.get("entity").and_then(mctk_nbt::Tag::as_compound).unwrap_or(data);
                    e.get("id").and_then(mctk_nbt::Tag::as_str).map(pretty)
                })
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            if others.len() > 1 {
                lines.push(format!("Or: {}", others.join(", ")));
            }
            let n = |k: &str| c.get(k).and_then(mctk_nbt::Tag::as_i64);
            if let (Some(count), Some(range)) = (n("SpawnCount"), n("RequiredPlayerRange")) {
                lines.push(format!("Up to {count} at a time, with a player within {range} blocks"));
            }
            return Some(Card { title: "Monster Spawner".into(), named: false, kind: None, stats: None, lines, grids: Vec::new() });
        }
        if !c.contains_key("Items") {
            return None;
        }
        let id = mctk_convert::view::java_block_entity_id(c).unwrap_or_default();
        let (size, cols) = container_shape(&id);
        let mut slots = list_slots(c.get("Items"), size, 0);
        let mut title_from = c.clone();
        let mut double = false;
        // A large chest: both halves, the right half's first (as the game puts them).
        if let Some(block) = w.block_at(h.x, h.y, h.z) {
            let props = world::block_props(block).unwrap_or_default();
            let prop = |k: &str| props.split(',').find_map(|kv| kv.strip_prefix(k)?.strip_prefix('=')).unwrap_or("").to_string();
            let half = prop("type");
            if half == "left" || half == "right" {
                // The other half is clockwise of the facing for the left half, else anticlockwise.
                let (dx, dz) = match (prop("facing").as_str(), half == "left") {
                    ("north", true) | ("south", false) => (1, 0),
                    ("south", true) | ("north", false) => (-1, 0),
                    ("east", true) | ("west", false) => (0, 1),
                    _ => (0, -1),
                };
                if let Some(other) = w.block_entity_at(h.x + dx, h.y, h.z + dz) {
                    if let Some(oc) = other.as_compound() {
                        let theirs = list_slots(oc.get("Items"), 27, 0);
                        slots = if half == "right" { slots.into_iter().chain(theirs).collect() } else { theirs.into_iter().chain(slots).collect() };
                        if crate::naming::mob_name(c).is_none() {
                            title_from = oc.clone();
                        }
                        double = true;
                    }
                }
            }
        }
        let name = crate::naming::mob_name(&title_from);
        let kind = if double { "Large Chest".to_string() } else { pretty(&id) };
        Some(Card { title: name.clone().unwrap_or_else(|| kind.clone()), named: name.is_some(), kind: name.map(|_| kind), stats: None, lines: Vec::new(), grids: vec![(cols, slots)] })
    }
}

/// Paints a [`Card`] beside the pointer, kept inside `bounds`.
fn paint_contents(p: &egui::Painter, bounds: Rect, pointer: Pos2, card: &Card) {
    const SLOT: f32 = 22.0;
    const PAD: f32 = 8.0;
    let title_font = if card.named { theme::italic(13.5) } else { theme::regular(13.5) };
    let title = p.layout_no_wrap(card.title.clone(), title_font, theme::text());
    let kind = card.kind.as_ref().map(|k| p.layout_no_wrap(k.clone(), theme::regular(11.5), theme::text_muted()));
    const LINE: f32 = 18.0;
    let grids_w = card.grids.iter().map(|(cols, s)| cols.min(&s.len()).max(&1) * SLOT as usize).max().unwrap_or(0) as f32;
    let grids_h: f32 = card.grids.iter().map(|(cols, s)| s.len().div_ceil((*cols).max(1)) as f32 * SLOT + 6.0).sum();
    // The kind sits beside the title, on its line.
    let head_h = title.size().y + 6.0;
    // Health and armor on one line, then a line per effect: its picture, name and time left.
    let small = theme::regular(12.0);
    let num = |v: f64| if (v - v.round()).abs() < 0.05 { format!("{}", v.round()) } else { format!("{v:.1}") };
    let health_text = card.stats.as_ref().map(|s| match s.max {
        Some(m) => format!("{} / {}", num(s.health), num(m)),
        None => num(s.health),
    });
    let effect_texts: Vec<(String, String)> = card.stats.iter().flat_map(|s| s.effects.iter().map(effect_text)).collect();
    let text_w = |t: &str| p.layout_no_wrap(t.to_string(), small.clone(), theme::text()).size().x;
    let stats_w = card.stats.as_ref().map_or(0.0, |s| {
        let line = 16.0 + text_w(health_text.as_deref().unwrap_or("")) + if s.armor > 0 { 14.0 + 16.0 + text_w(&s.armor.to_string()) } else { 0.0 };
        effect_texts.iter().map(|(n, t)| 20.0 + text_w(n) + 14.0 + text_w(t)).fold(line, f32::max)
    });
    let stats_h = card.stats.as_ref().map_or(0.0, |_| LINE * (1 + effect_texts.len()) as f32 + 6.0);
    let lines_w = card.lines.iter().map(|l| text_w(l)).fold(0.0, f32::max);
    let lines_h = if card.lines.is_empty() { 0.0 } else { LINE * card.lines.len() as f32 + 6.0 };
    let w = (title.size().x + kind.as_ref().map_or(0.0, |k| k.size().x + 8.0)).max(grids_w).max(stats_w).max(lines_w) + PAD * 2.0;
    let h = head_h + lines_h + stats_h + grids_h + PAD * 2.0 - 6.0;
    let mut at = pointer + vec2(18.0, 18.0);
    if at.x + w > bounds.right() - 4.0 {
        at.x = pointer.x - 18.0 - w;
    }
    if at.y + h > bounds.bottom() - 4.0 {
        at.y = pointer.y - 18.0 - h;
    }
    let r = Rect::from_min_size(at, vec2(w, h));
    p.rect_filled(r, theme::radius(5), theme::deep_alpha(240));
    p.rect_stroke(r, theme::radius(5), Stroke::new(1.0_f32, theme::wash(28)), egui::StrokeKind::Inside);
    let mut y = r.top() + PAD;
    let title_h = title.size().y;
    let title_w = title.size().x;
    p.galley(pos2(r.left() + PAD, y), title, theme::text());
    if let Some(k) = kind {
        p.galley(pos2(r.left() + PAD + title_w + 8.0, y + title_h - k.size().y), k, theme::text_muted());
    }
    y += head_h;
    for l in &card.lines {
        p.text(pos2(r.left() + PAD, y + LINE / 2.0), Align2::LEFT_CENTER, l, small.clone(), theme::text());
        y += LINE;
    }
    if !card.lines.is_empty() {
        y += 6.0;
    }
    if let Some(s) = &card.stats {
        let left = r.left() + PAD;
        let mid = |y: f32| y + LINE / 2.0;
        // The game's own heart and armor pictures (a plain mark where the pack has none).
        let picture = |file: &str, at: Rect, fallback: Color32| {
            if !icons::pack_texture(file).is_some_and(|f| icons::file_image(p, at, &f)) {
                p.circle_filled(at.center(), at.width() * 0.4, fallback);
            }
        };
        picture("gui/sprites/hud/heart/full.png", Rect::from_center_size(pos2(left + 6.0, mid(y)), vec2(12.0, 12.0)), theme::RED);
        let ht = health_text.clone().unwrap_or_default();
        let hw = text_w(&ht);
        p.text(pos2(left + 16.0, mid(y)), Align2::LEFT_CENTER, &ht, small.clone(), theme::text());
        if s.armor > 0 {
            let ax = left + 16.0 + hw + 14.0;
            picture("gui/sprites/hud/armor_full.png", Rect::from_center_size(pos2(ax + 6.0, mid(y)), vec2(12.0, 12.0)), theme::text_muted());
            p.text(pos2(ax + 16.0, mid(y)), Align2::LEFT_CENTER, s.armor.to_string(), small.clone(), theme::text());
        }
        y += LINE;
        for (e, (name, time)) in s.effects.iter().zip(&effect_texts) {
            let icon = Rect::from_center_size(pos2(left + 8.0, mid(y)), vec2(16.0, 16.0));
            if !crate::effects::icon_file(&e.id).is_some_and(|f| icons::file_image(p, icon, &f)) {
                p.circle_filled(icon.center(), 5.0, theme::text_muted());
            }
            p.text(pos2(left + 20.0, mid(y)), Align2::LEFT_CENTER, name, small.clone(), theme::text());
            p.text(pos2(r.right() - PAD, mid(y)), Align2::RIGHT_CENTER, time, small.clone(), theme::text_muted());
            y += LINE;
        }
        y += 6.0;
    }
    for (cols, slots) in &card.grids {
        let cols = (*cols).max(1);
        for (i, s) in slots.iter().enumerate() {
            let cell = Rect::from_min_size(pos2(r.left() + PAD + (i % cols) as f32 * SLOT, y + (i / cols) as f32 * SLOT), vec2(SLOT, SLOT)).shrink(1.0);
            p.rect_filled(cell, theme::radius(2), theme::tile());
            let icon = cell.shrink(2.0);
            match &s.item {
                Some((id, count)) => {
                    if !icons::item_icon(p, icon, id, Color32::WHITE) {
                        p.rect_stroke(icon, theme::radius(2), Stroke::new(1.0_f32, theme::text_muted()), egui::StrokeKind::Inside);
                    }
                    if *count > 1 {
                        let t = count.to_string();
                        let at = cell.right_bottom() + vec2(0.0, 1.0);
                        p.text(at + vec2(1.0, 1.0), Align2::RIGHT_BOTTOM, &t, theme::regular(11.0), Color32::BLACK);
                        p.text(at, Align2::RIGHT_BOTTOM, &t, theme::regular(11.0), Color32::WHITE);
                    }
                }
                None => {
                    if let Some(f) = s.empty.and_then(icons::pack_texture) {
                        icons::file_image(p, icon, &f);
                    }
                }
            }
        }
        y += slots.len().div_ceil(cols) as f32 * SLOT + 6.0;
    }
}
/// Paints sample cards (a mob's gear, a large chest) for looking at in a snapshot.
#[cfg(test)]
pub(crate) fn sample_cards(p: &egui::Painter, bounds: Rect) {
    let s = |id: &str, n: i64| CardSlot { item: Some((format!("minecraft:{id}"), n)), empty: None };
    let gear: Vec<CardSlot> = crate::mob_edit::Slot::ALL
        .iter()
        .map(|sl| CardSlot { item: matches!(sl, crate::mob_edit::Slot::Chest | crate::mob_edit::Slot::MainHand).then(|| (if *sl == crate::mob_edit::Slot::Chest { "minecraft:iron_chestplate" } else { "minecraft:iron_sword" }.to_string(), 1)), empty: Some(sl.empty_picture()) })
        .collect();
    let stats = CardStats {
        health: 15.0,
        max: Some(20.0),
        armor: 8,
        effects: vec![
            crate::mob_edit::Effect { id: "speed".into(), amplifier: 1, duration: 1800 },
            crate::mob_edit::Effect { id: "fire_resistance".into(), amplifier: 0, duration: -1 },
        ],
    };
    paint_contents(p, bounds, bounds.min + vec2(4.0, 4.0), &Card { title: "Bob".into(), named: true, kind: Some("Zombie".into()), stats: Some(stats), lines: Vec::new(), grids: vec![(6, gear)] });
    let mut chest: Vec<CardSlot> = (0..54).map(|_| CardSlot { item: None, empty: None }).collect();
    chest[0] = s("diamond", 12);
    chest[5] = s("oak_log", 64);
    chest[30] = s("golden_apple", 1);
    paint_contents(p, bounds, bounds.min + vec2(4.0, 170.0), &Card { title: "Large Chest".into(), named: false, kind: None, stats: None, lines: Vec::new(), grids: vec![(9, chest)] });
}
