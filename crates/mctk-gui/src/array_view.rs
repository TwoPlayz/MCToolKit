//! How the NBT editor shows byte, int and long arrays: a paged grid (or hex dump) instead of
//! one row per value, range tools (fill, zero, resize, edit as text, copy, import, export),
//! and pictures for arrays whose name and size say what they hold (map colors, light
//! levels, heightmaps and other 16 x 16 planes).

// Arrays open in the hex editor (hex_editor); its View switch shows the pictures here (map colors with
// picture import, light layers, 16 x 16 planes). The old inline hex, grid and tools views are not used.
#![allow(dead_code)]

use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use eframe::egui::{self, Color32, Sense, TextureHandle, Ui, vec2};

use crate::theme;

/// A value an array holds: `i8` (byte arrays), `i32` (int arrays) or `i64` (long arrays).
pub trait Elem: eframe::emath::Numeric {
    /// Values on one page.
    const PAGE: usize;
    /// Most columns in a grid row.
    const COLS: usize;
    /// Width of one value box.
    const CELL: f32;
    /// Size in bytes, which is also how it is exported.
    const WIDTH: usize;
    const LO: i64;
    const HI: i64;
    fn get(self) -> i64;
    fn make(v: i64) -> Self;
    fn push_be(self, out: &mut Vec<u8>);
    fn from_be(b: &[u8]) -> Self;
}

macro_rules! elem {
    ($t:ty, $page:expr, $cols:expr, $cell:expr) => {
        impl Elem for $t {
            const PAGE: usize = $page;
            const COLS: usize = $cols;
            const CELL: f32 = $cell;
            const WIDTH: usize = std::mem::size_of::<$t>();
            const LO: i64 = <$t>::MIN as i64;
            const HI: i64 = <$t>::MAX as i64;
            fn get(self) -> i64 {
                self as i64
            }
            fn make(v: i64) -> Self {
                v as $t
            }
            fn push_be(self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_be_bytes());
            }
            fn from_be(b: &[u8]) -> Self {
                <$t>::from_be_bytes(b.try_into().unwrap_or_default())
            }
        }
    };
}
elem!(i8, 256, 16, 40.0);
elem!(i32, 128, 8, 84.0);
elem!(i64, 64, 4, 150.0);

const CELL_H: f32 = 18.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Grid,
    Hex,
    /// 128 x 128 map color ids drawn as the map.
    Map,
    /// Two 4-bit light levels per byte, shown one 16 x 16 layer at a time.
    Nibbles,
    /// A 16 x 16 plane of values: a heightmap's packed 9-bit heights, or 256 ints.
    Plane,
}

impl View {
    fn label(self) -> &'static str {
        match self {
            View::Grid => "Grid",
            View::Hex => "Hex",
            View::Map => "Map image",
            View::Nibbles => "Light layers",
            View::Plane => "16 x 16",
        }
    }
}

#[derive(Clone)]
#[derive(Default)]
struct State {
    view: Option<View>,
    page: usize,
    goto: usize,
    from: usize,
    /// End of the range the tools use; `None` is the end of the array.
    to: Option<usize>,
    fill: i64,
    new_len: Option<usize>,
    layer: usize,
    /// A picture being imported into a map, while its preview window is open.
    draft: Option<Draft>,
    tools: bool,
    /// Asked for by the buttons in the editor's bar, done on the next draw.
    import: bool,
    clear: bool,
    /// The "edit as text" window, with its text, while it is open.
    text: Option<String>,
}


/// Where and how an array is drawn.
pub struct Panel<'a> {
    /// Identifies the array (its path), so each keeps its own page and view.
    pub key: &'a str,
    /// The tag's name, which decides what the array probably holds.
    pub name: &'a str,
    pub indent: f32,
    pub width: f32,
    pub read_only: bool,
    /// Set while a value box or the text window has keyboard focus.
    pub typing: &'a Cell<bool>,
}

/// What the page has to do after drawing: an undo step, a style refresh, a message.
#[derive(Default)]
pub struct Outcome {
    pub snapshot: bool,
    pub changed: bool,
    pub note: Option<String>,
}

/// How values are packed into the array, for the picture views.
#[derive(Clone, Copy)]
enum Packing {
    /// Two 4-bit values per byte, low nibble first.
    Nibble,
    /// 9-bit heights, seven to a long (the format since 1.16).
    Height9,
    Direct,
}

fn read<E: Elem>(v: &[E], pk: Packing, i: usize) -> i64 {
    match pk {
        Packing::Nibble => {
            let b = v[i / 2].get() as u8;
            (if i % 2 == 0 { b & 15 } else { b >> 4 }) as i64
        }
        Packing::Height9 => ((v[i / 7].get() as u64 >> ((i % 7) * 9)) & 0x1FF) as i64,
        Packing::Direct => v[i].get(),
    }
}

fn write<E: Elem>(v: &mut [E], pk: Packing, i: usize, val: i64) {
    match pk {
        Packing::Nibble => {
            let b = v[i / 2].get() as u8;
            let val = (val as u8) & 15;
            let b = if i % 2 == 0 { (b & 0xF0) | val } else { (b & 0x0F) | (val << 4) };
            v[i / 2] = E::make(b as i8 as i64);
        }
        Packing::Height9 => {
            let shift = (i % 7) * 9;
            let l = (v[i / 7].get() as u64 & !(0x1FFu64 << shift)) | (((val as u64) & 0x1FF) << shift);
            v[i / 7] = E::make(l as i64);
        }
        Packing::Direct => v[i] = E::make(val),
    }
}

fn is_light(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("light") || name == "Data"
}

fn is_heightmap(name: &str) -> bool {
    matches!(
        name,
        "MOTION_BLOCKING" | "MOTION_BLOCKING_NO_LEAVES" | "WORLD_SURFACE" | "OCEAN_FLOOR"
            | "WORLD_SURFACE_WG" | "OCEAN_FLOOR_WG"
    )
}

/// The views that make sense for this array, the first being the plain table: hex for bytes,
/// numbers for the wider types.
fn views<E: Elem>(name: &str, len: usize) -> Vec<View> {
    let mut v = vec![if E::WIDTH == 1 { View::Hex } else { View::Grid }];
    if E::WIDTH == 1 {
        if len == 128 * 128 {
            v.push(View::Map);
        }
        if len == 2048 && is_light(name) {
            v.push(View::Nibbles);
        }
    }
    if (E::WIDTH == 8 && len == 37 && is_heightmap(name)) || (E::WIDTH == 4 && len == 256) {
        v.push(View::Plane);
    }
    v
}

/// The view an array opens in: its picture when its name says what it is.
fn default_view(avail: &[View], name: &str) -> View {
    let wanted = if name == "colors" {
        View::Map
    } else if is_light(name) {
        View::Nibbles
    } else if is_heightmap(name) {
        View::Plane
    } else {
        avail[0]
    };
    if avail.contains(&wanted) { wanted } else { avail[0] }
}

/// A picture of an array's contents, offered beside the hex editor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Picture {
    /// 128 x 128 map color ids drawn as the map, with picture import.
    Map,
    /// Two 4-bit light levels per byte, one 16 x 16 layer at a time.
    Light,
    /// A 16 x 16 plane of values (a heightmap, or 256 ints).
    Plane,
}

impl Picture {
    pub fn label(self) -> &'static str {
        match self {
            Picture::Map => "Map",
            Picture::Light => "Light layers",
            Picture::Plane => "16 x 16",
        }
    }
}

/// The pictures that make sense for an array of this name and length.
pub fn pictures<E: Elem>(name: &str, len: usize) -> Vec<Picture> {
    views::<E>(name, len)
        .into_iter()
        .filter_map(|v| match v {
            View::Map => Some(Picture::Map),
            View::Nibbles => Some(Picture::Light),
            View::Plane => Some(Picture::Plane),
            _ => None,
        })
        .collect()
}

/// Draws one picture of the array, with nothing around it.
pub fn show_picture<E: Elem>(ui: &mut Ui, p: &Panel, values: &mut [E], picture: Picture) -> Outcome {
    let id = egui::Id::new(("array-view", p.key));
    let mut st: State = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut out = Outcome::default();
    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
    if values.is_empty() {
        ui.label(egui::RichText::new("Empty array").color(theme::text_dim()));
    } else {
        match picture {
            Picture::Map => map(ui, p, &mut st, values, &mut out),
            Picture::Light => {
                ui.horizontal(|ui| {
                    ui.label("Layer (Y)");
                    ui.add(egui::Slider::new(&mut st.layer, 0..=15));
                    ui.label(egui::RichText::new("columns are X, rows are Z").color(theme::text_dim()));
                });
                let base = st.layer.min(15) * 256;
                plane(ui, p, values, Packing::Nibble, base, 15, &mut out);
            }
            Picture::Plane => {
                let pk = if E::WIDTH == 8 { Packing::Height9 } else { Packing::Direct };
                let max = if E::WIDTH == 8 { 511 } else { E::HI };
                ui.label(egui::RichText::new("columns are X, rows are Z").color(theme::text_dim()));
                plane(ui, p, values, pk, 0, max, &mut out);
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(id, st));
    out
}
pub fn show<E: Elem>(ui: &mut Ui, p: &Panel, values: &mut Vec<E>) -> Outcome {
    let id = egui::Id::new(("array-view", p.key));
    let mut st: State = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut out = Outcome::default();
    let avail = views::<E>(p.name, values.len());
    let view = st
        .view
        .filter(|v| avail.contains(v))
        .unwrap_or_else(|| default_view(&avail, p.name));
    ui.horizontal_top(|ui| {
        ui.add_space(p.indent);
        ui.vertical(|ui| {
            ui.set_max_width((p.width - p.indent - 12.0).max(240.0));
            ui.add_space(2.0);
            body(ui, p, &mut st, view, &avail, values, &mut out);
            ui.add_space(4.0);
        });
    });
    ui.data_mut(|d| d.insert_temp(id, st));
    out
}

fn body<E: Elem>(
    ui: &mut Ui,
    p: &Panel,
    st: &mut State,
    view: View,
    avail: &[View],
    values: &mut Vec<E>,
    out: &mut Outcome,
) {
    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
    // View choice and tools toggle.
    ui.horizontal(|ui| {
        if avail.len() > 1 {
            for v in avail {
                if ui.selectable_label(view == *v, v.label()).clicked() {
                    st.view = Some(*v);
                    st.page = 0;
                }
            }
            ui.separator();
        }
        if ui.selectable_label(st.tools, "Tools").clicked() {
            st.tools = !st.tools;
        }
    });
    if st.tools {
        tools(ui, p, st, values, out);
    }
    if values.is_empty() {
        ui.label(egui::RichText::new("Empty array").color(theme::text_dim()));
        return;
    }
    match view {
        View::Grid | View::Hex => {
            paging::<E>(ui, st, values.len());
            grid(ui, p, st, values, view == View::Hex, out);
        }
        View::Map => map(ui, p, st, values, out),
        View::Nibbles => {
            ui.horizontal(|ui| {
                ui.label("Layer (Y)");
                ui.add(egui::Slider::new(&mut st.layer, 0..=15));
                ui.label(egui::RichText::new("columns are X, rows are Z").color(theme::text_dim()));
            });
            let base = st.layer.min(15) * 256;
            plane(ui, p, values, Packing::Nibble, base, 15, out);
        }
        View::Plane => {
            let pk = if E::WIDTH == 8 { Packing::Height9 } else { Packing::Direct };
            let max = if E::WIDTH == 8 { 511 } else { E::HI };
            ui.label(
                egui::RichText::new("columns are X, rows are Z")
                    .color(theme::text_dim()),
            );
            plane(ui, p, values, pk, 0, max, out);
        }
    }
}

fn paging<E: Elem>(ui: &mut Ui, st: &mut State, len: usize) {
    let pages = len.div_ceil(E::PAGE).max(1);
    st.page = st.page.min(pages - 1);
    let (start, end) = (st.page * E::PAGE, ((st.page + 1) * E::PAGE).min(len));
    ui.horizontal(|ui| {
        if ui.add_enabled(st.page > 0, egui::Button::new("\u{2039}")).clicked() {
            st.page -= 1;
        }
        ui.label(format!("{start}\u{2013}{} of {len}", end.saturating_sub(1)));
        if ui.add_enabled(st.page + 1 < pages, egui::Button::new("\u{203A}")).clicked() {
            st.page += 1;
        }
        ui.separator();
        ui.label("Go to");
        ui.add(egui::DragValue::new(&mut st.goto).range(0..=len.saturating_sub(1)));
        if ui.button("Go").clicked() {
            st.page = st.goto / E::PAGE;
        }
    });
}

/// A value box styled like the editor's other fields.
fn flat(ui: &mut Ui) {
    let s = ui.style_mut();
    s.spacing.item_spacing = vec2(3.0, 2.0);
    s.spacing.interact_size.y = CELL_H;
    s.spacing.button_padding = vec2(4.0, 0.0);
    s.visuals.widgets.inactive.weak_bg_fill = theme::field();
    s.visuals.widgets.inactive.bg_fill = theme::field();
    s.override_font_id = Some(theme::regular(13.0));
}

fn note_edit(resp: &egui::Response, p: &Panel, out: &mut Outcome) {
    if resp.has_focus() {
        p.typing.set(true);
    }
    if resp.drag_started() || resp.gained_focus() {
        out.snapshot = true;
    }
    if resp.changed() {
        out.changed = true;
    }
}

fn grid<E: Elem>(ui: &mut Ui, p: &Panel, st: &State, values: &mut [E], hex: bool, out: &mut Outcome) {
    let cell = if hex { 30.0 } else { E::CELL };
    let label = 58.0;
    let ascii = if hex { 150.0 } else { 0.0 };
    let cols = (((ui.available_width() - label - ascii) / (cell + 3.0)).floor() as usize)
        .clamp(1, E::COLS);
    let len = values.len();
    let end = ((st.page + 1) * E::PAGE).min(len);
    let mut i = st.page * E::PAGE;
    ui.spacing_mut().item_spacing.y = 2.0;
    while i < end {
        let row_end = (i + cols).min(end);
        ui.horizontal(|ui| {
            flat(ui);
            ui.add_sized(
                [label, CELL_H],
                egui::Label::new(
                    egui::RichText::new(i.to_string()).monospace().color(theme::text_muted()),
                ),
            );
            for v in &mut values[i..row_end] {
                let mut dv = egui::DragValue::new(v);
                if hex {
                    dv = dv.hexadecimal(2, false, true);
                }
                let resp = ui
                    .add_enabled_ui(!p.read_only, |ui| ui.add_sized([cell, CELL_H], dv))
                    .inner;
                note_edit(&resp, p, out);
            }
            if hex {
                let text: String = values[i..row_end]
                    .iter()
                    .map(|v| match v.get() as u8 {
                        c @ 0x20..=0x7E => c as char,
                        _ => '.',
                    })
                    .collect();
                ui.label(egui::RichText::new(text).monospace().color(theme::text_dim()));
            }
        });
        i = row_end;
    }
}

/// Map color ids are `base * 4 + shade`; the bases are the game's block colors.
const MAP_BASES: [[u8; 3]; 62] = [
    [0, 0, 0],
    [127, 178, 56],
    [247, 233, 163],
    [199, 199, 199],
    [255, 0, 0],
    [160, 160, 255],
    [167, 167, 167],
    [0, 124, 0],
    [255, 255, 255],
    [164, 168, 184],
    [151, 109, 77],
    [112, 112, 112],
    [64, 64, 255],
    [143, 119, 72],
    [255, 252, 245],
    [216, 127, 51],
    [178, 76, 216],
    [102, 153, 216],
    [229, 229, 51],
    [127, 204, 25],
    [242, 127, 165],
    [76, 76, 76],
    [153, 153, 153],
    [76, 127, 153],
    [127, 63, 178],
    [51, 76, 178],
    [102, 76, 51],
    [102, 127, 51],
    [153, 51, 51],
    [25, 25, 25],
    [250, 238, 77],
    [92, 219, 213],
    [74, 128, 255],
    [0, 217, 58],
    [129, 86, 49],
    [112, 2, 0],
    [209, 177, 161],
    [159, 82, 36],
    [149, 87, 108],
    [112, 108, 138],
    [186, 133, 36],
    [103, 117, 53],
    [160, 77, 78],
    [57, 41, 35],
    [135, 107, 98],
    [87, 92, 92],
    [122, 73, 88],
    [76, 62, 92],
    [76, 50, 35],
    [76, 82, 42],
    [142, 60, 46],
    [37, 22, 16],
    [189, 48, 49],
    [148, 63, 97],
    [92, 25, 29],
    [22, 126, 134],
    [58, 142, 140],
    [86, 44, 62],
    [20, 180, 133],
    [100, 100, 100],
    [216, 175, 147],
    [127, 167, 150],
];

/// The color a map id is drawn with; ids 0 to 3 are transparent, unknown bases are magenta.
fn map_rgba(id: u8) -> [u8; 4] {
    let (base, shade) = ((id >> 2) as usize, id & 3);
    if base == 0 {
        return [0, 0, 0, 0];
    }
    let Some(c) = MAP_BASES.get(base) else {
        return [255, 0, 255, 255];
    };
    let m = [180u32, 220, 255, 135][shade as usize];
    [
        (c[0] as u32 * m / 255) as u8,
        (c[1] as u32 * m / 255) as u8,
        (c[2] as u32 * m / 255) as u8,
        255,
    ]
}

fn map<E: Elem>(ui: &mut Ui, p: &Panel, st: &mut State, values: &mut [E], out: &mut Outcome) {
    let mut draft = st.draft.take();
    let (mut apply, mut cancel, mut other) = (false, false, false);
    // The Import image and Clear map buttons are in the editor's bar (see `map_bar`).
    let clear = std::mem::take(&mut st.clear);
    if std::mem::take(&mut st.import) && draft.is_none() {
        draft = pick_image(p);
    }
    let scale = ((ui.available_width() - 20.0) / 128.0).floor().clamp(2.0, 3.0);
    let size = vec2(128.0 * scale, 128.0 * scale);
    let sense = if draft.is_some() { Sense::click_and_drag() } else { Sense::hover() };
    // The options sit beside the map when there is room for them, otherwise under it.
    let beside = draft.is_some() && ui.available_width() >= size.x + 20.0 + 300.0;
    // A checkerboard behind it shows which pixels are transparent.
    let (rect, resp);
    if beside {
        let row = ui.horizontal_top(|ui| {
            let canvas = ui.allocate_exact_size(size, sense);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                if let Some(d) = draft.as_mut() {
                    draft_controls(ui, d, &mut apply, &mut cancel, &mut other);
                }
            });
            canvas
        });
        (rect, resp) = row.inner;
    } else {
        if let Some(d) = draft.as_mut() {
            draft_controls(ui, d, &mut apply, &mut cancel, &mut other);
        }
        (rect, resp) = ui.allocate_exact_size(size, sense);
    }
    if let Some(d) = draft.as_mut() {
        // Tells the tree not to start a drag-select box from a press on the picture.
        ui.ctx().data_mut(|data| {
            data.get_temp_mut_or_default::<Vec<egui::Rect>>(egui::Id::new(NO_MARQUEE))
                .push(rect.expand(8.0))
        });
        navigate(ui, &resp, rect, scale, d);
        refresh(ui.ctx(), p, d);
    }
    // The picture being imported takes the place of the array's own colors while it is open.
    let previewed = draft.as_ref().and_then(|d| d.result.as_ref());
    let tex = match previewed {
        Some((_, _, t)) => t.clone(),
        None => {
            let hash = {
                let mut h = DefaultHasher::new();
                for v in values.iter() {
                    v.get().hash(&mut h);
                }
                h.finish()
            };
            let tex_id = egui::Id::new(("array-map", p.key));
            let cached: Option<(u64, TextureHandle)> = ui.data(|d| d.get_temp(tex_id));
            match cached {
                Some((h, t)) if h == hash => t,
                _ => {
                    let mut px = Vec::with_capacity(values.len() * 4);
                    for v in values.iter() {
                        px.extend_from_slice(&map_rgba(v.get() as u8));
                    }
                    let image = egui::ColorImage::from_rgba_unmultiplied([128, 128], &px);
                    let t = ui.ctx().load_texture(
                        format!("array-map-{}", p.key),
                        image,
                        egui::TextureOptions::NEAREST,
                    );
                    ui.data_mut(|d| d.insert_temp(tex_id, (hash, t.clone())));
                    t
                }
            }
        }
    };
    let step = 8.0;
    for cy in 0..(size.y / step).ceil() as usize {
        for cx in 0..(size.x / step).ceil() as usize {
            let c = if (cx + cy) % 2 == 0 {
                Color32::from_gray(0x2A)
            } else {
                Color32::from_gray(0x36)
            };
            let r = egui::Rect::from_min_size(
                rect.min + vec2(cx as f32 * step, cy as f32 * step),
                vec2(step, step),
            )
            .intersect(rect);
            ui.painter().rect_filled(r, 0.0, c);
        }
    }
    ui.painter().image(
        tex.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    let at = |pos: egui::Pos2| {
        let q = (pos - rect.min) / scale;
        let (x, z) = (q.x.floor() as i32, q.y.floor() as i32);
        ((0..128).contains(&x) && (0..128).contains(&z)).then(|| (z * 128 + x) as usize)
    };
    if draft.is_none() {
        if let Some(i) = resp.hover_pos().and_then(at) {
            let id = values[i].get() as u8;
            resp.clone().on_hover_text(format!(
                "x {}, z {}: id {id} (color {}, shade {})",
                i % 128,
                i / 128,
                id >> 2,
                id & 3
            ));
        }
    } else if let Some(d) = draft.as_ref() {
        outline(ui, rect, scale, d);
    }
    if clear {
        out.snapshot = true;
        out.changed = true;
        for v in values.iter_mut() {
            *v = E::make(0);
        }
        out.note = Some("Map cleared".into());
    }
    if apply {
        if let Some((_, ids, _)) = draft.as_ref().and_then(|d| d.result.as_ref()) {
            out.snapshot = true;
            out.changed = true;
            for (v, id) in values.iter_mut().zip(ids.iter()) {
                *v = E::make(*id as i8 as i64);
            }
            out.note = Some("Picture imported as map colors".into());
        }
        draft = None;
    } else if cancel {
        draft = None;
    } else if other {
        if let Some(next) = pick_image(p) {
            draft = Some(next);
        }
    }
    st.draft = draft;
}

/// The buttons for a map, drawn in the editor's bar: they ask the map view to import a picture or
/// clear the map. `blank` is whether the map is already empty.
pub fn map_bar(ui: &mut Ui, p: &Panel, blank: bool) {
    let id = egui::Id::new(("array-view", p.key));
    let mut st: State = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    if st.draft.is_none() {
        if crate::widgets::compact_button(ui, "Import image\u{2026}", !p.read_only)
            .on_hover_text("Turn a PNG or JPEG into map colors, previewed right here")
            .clicked()
        {
            st.import = true;
        }
        if crate::widgets::compact_button(ui, "Clear map", !p.read_only && !blank)
            .on_hover_text("Erase the whole map to transparent")
            .clicked()
        {
            st.clear = true;
        }
    }
    ui.data_mut(|d| d.insert_temp(id, st));
}

/// The options for a picture being imported.
fn draft_controls(ui: &mut Ui, d: &mut Draft, apply: &mut bool, cancel: &mut bool, other: &mut bool) {
    ui.label(egui::RichText::new(&d.name).color(theme::text_muted()));
    let a = &mut d.adjust;
    let fit = a.fit;
    egui::Grid::new("array-import-options")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.spacing_mut().slider_width = 130.0;
            ui.label("Fit");
            let labels = Fit::ALL.map(Fit::label);
            let now = Fit::ALL.iter().position(|f| *f == a.fit).unwrap_or(0);
            if let Some(i) = crate::widgets::choice(ui, "array-fit", now, &labels) {
                a.fit = Fit::ALL[i];
            }
            ui.end_row();
            ui.label("Zoom");
            ui.add(
                egui::Slider::new(&mut a.zoom, 0.1..=8.0)
                    .logarithmic(true)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                    .custom_parser(|s| {
                        s.trim().trim_end_matches('%').parse::<f64>().ok().map(|v| v / 100.0)
                    }),
            );
            ui.end_row();
            for (label, v) in [
                ("Brightness", &mut a.brightness),
                ("Contrast", &mut a.contrast),
                ("Saturation", &mut a.saturation),
            ] {
                ui.label(label);
                ui.add(egui::Slider::new(v, -100.0..=100.0).fixed_decimals(0));
                ui.end_row();
            }
            ui.label("");
            ui.checkbox(&mut a.dither, "Dither")
                .on_hover_text("Blend neighboring colors to approximate shades the map lacks");
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut a.smooth, "Smoothing")
                .on_hover_text("Smooth the picture when it is resized; off keeps hard pixel edges");
            ui.end_row();
        });
    if a.fit != fit {
        // A new fit places the picture afresh: its own proportions, centerd, not zoomed.
        a.wx = 1.0;
        a.wy = 1.0;
        a.zoom = 1.0;
        a.ox = 0.0;
        a.oy = 0.0;
    }
    ui.horizontal_wrapped(|ui| {
        *apply = ui.button("Apply").clicked();
        *cancel = ui.button("Cancel").clicked();
        *other = ui.button("Choose another\u{2026}").clicked();
        if ui
            .button("Reset")
            .on_hover_text("Zoom, shape, position and color adjustments")
            .clicked()
        {
            *a = Adjust { fit: a.fit, dither: a.dither, smooth: a.smooth, ..Adjust::default() };
        }
    });
    ui.label(
        egui::RichText::new("Drag to move \u{b7} drag an edge to stretch \u{b7} scroll to zoom")
            .small()
            .color(theme::text_dim()),
    );
}

/// Tells the tree which parts of the screen are not for drag-select boxes.
pub const NO_MARQUEE: &str = "array-no-marquee";

/// The picture's size in map pixels, before it is rounded and cut to the map.
fn extent(img: &image::DynamicImage, a: &Adjust) -> (f32, f32) {
    const N: f32 = 128.0;
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (sx, sy) = match a.fit {
        Fit::Stretch => (N / w, N / h),
        Fit::Fit => {
            let s = (N / w).min(N / h);
            (s, s)
        }
        Fit::Fill => {
            let s = (N / w).max(N / h);
            (s, s)
        }
    };
    (w * sx * a.zoom * a.wx, h * sy * a.zoom * a.wy)
}

/// Where the picture is on screen.
fn picture_rect(d: &Draft, rect: egui::Rect, scale: f32) -> egui::Rect {
    let (w, h) = extent(&d.source, &d.adjust);
    let center = rect.center() + vec2(d.adjust.ox, d.adjust.oy) * scale;
    egui::Rect::from_center_size(center, vec2(w, h) * scale)
}

/// Which edges of the picture a point is on: -1 left or top, 1 right or bottom, 0 neither.
fn edge_at(pic: egui::Rect, pos: egui::Pos2) -> Option<(i8, i8)> {
    const GRAB: f32 = 6.0;
    if !pic.expand(GRAB).contains(pos) {
        return None;
    }
    let side = |p: f32, lo: f32, hi: f32| {
        if (p - lo).abs() <= GRAB {
            -1
        } else if (p - hi).abs() <= GRAB {
            1
        } else {
            0
        }
    };
    let e = (side(pos.x, pic.left(), pic.right()), side(pos.y, pic.top(), pic.bottom()));
    (e != (0, 0)).then_some(e)
}

fn resize_cursor(e: (i8, i8)) -> egui::CursorIcon {
    match e {
        (_, 0) => egui::CursorIcon::ResizeHorizontal,
        (0, _) => egui::CursorIcon::ResizeVertical,
        (a, b) if a == b => egui::CursorIcon::ResizeNwSe,
        _ => egui::CursorIcon::ResizeNeSw,
    }
}

/// The picture's border, so its edges can be found.
fn outline(ui: &Ui, rect: egui::Rect, scale: f32, d: &Draft) {
    let pic = picture_rect(d, rect, scale);
    ui.painter().with_clip_rect(rect).rect_stroke(
        pic,
        0.0,
        egui::Stroke::new(1.0_f32, Color32::from_white_alpha(150)),
        egui::StrokeKind::Inside,
    );
}

/// Dragging moves the picture, dragging an edge stretches it, and the wheel zooms about the pointer.
fn navigate(ui: &mut Ui, resp: &egui::Response, rect: egui::Rect, scale: f32, d: &mut Draft) {
    let pic = picture_rect(d, rect, scale);
    if resp.drag_started() {
        d.grab = ui
            .input(|i| i.pointer.press_origin())
            .and_then(|p| edge_at(pic, p));
    }
    if !resp.dragged() {
        d.grab = None;
    }
    let hover = resp.hover_pos().and_then(|p| edge_at(pic, p));
    if resp.dragged() {
        let delta = resp.drag_delta() / scale;
        match d.grab {
            Some((gx, gy)) => {
                let (w, h) = extent(&d.source, &d.adjust);
                let a = &mut d.adjust;
                for (g, size, factor, offset, step) in [
                    (gx, w, &mut a.wx, &mut a.ox, delta.x),
                    (gy, h, &mut a.wy, &mut a.oy, delta.y),
                ] {
                    if g == 0 {
                        continue;
                    }
                    let target = (size + g as f32 * step).max(4.0);
                    let next = (*factor * target / size).clamp(0.02, 50.0);
                    let grown = size * next / *factor - size;
                    *factor = next;
                    // The edge opposite the one held stays where it is.
                    *offset += g as f32 * grown / 2.0;
                }
                ui.ctx().set_cursor_icon(resize_cursor((gx, gy)));
            }
            None => {
                d.adjust.ox += delta.x;
                d.adjust.oy += delta.y;
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
        }
    } else if let Some(e) = hover {
        ui.ctx().set_cursor_icon(resize_cursor(e));
    } else if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let a = &mut d.adjust;
    if !resp.hovered() {
        return;
    }
    let wheel = ui.input(|i| i.smooth_scroll_delta.y);
    if wheel == 0.0 {
        return;
    }
    let zoom = (a.zoom * (wheel * 0.0025).exp()).clamp(0.1, 8.0);
    if let Some(pos) = resp.hover_pos() {
        // Keeps the part of the picture under the pointer where it is.
        let rel = (pos - rect.center()) / scale;
        let ratio = zoom / a.zoom;
        a.ox = rel.x - (rel.x - a.ox) * ratio;
        a.oy = rel.y - (rel.y - a.oy) * ratio;
    }
    a.zoom = zoom;
    // So the page behind does not scroll as well.
    ui.input_mut(|i| {
        i.smooth_scroll_delta = egui::Vec2::ZERO;
        i.raw_scroll_delta = egui::Vec2::ZERO;
    });
}

/// How the picture is placed and adjusted before it is turned into map colors.
#[derive(Clone, Copy, PartialEq)]
struct Adjust {
    fit: Fit,
    dither: bool,
    /// Smooth resizing (else hard pixel edges).
    smooth: bool,
    /// Magnification on top of what `fit` gives.
    zoom: f32,
    /// Where the picture's middle is, in map pixels from the map's middle.
    ox: f32,
    oy: f32,
    /// Stretch of the width and height set by dragging the picture's edges.
    wx: f32,
    wy: f32,
    /// -100 to 100 each.
    brightness: f32,
    contrast: f32,
    saturation: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Adjust {
            fit: Fit::Fit,
            dither: true,
            smooth: true,
            zoom: 1.0,
            ox: 0.0,
            oy: 0.0,
            wx: 1.0,
            wy: 1.0,
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
        }
    }
}

impl Adjust {
    /// Changes whenever the result would.
    fn key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        (self.fit as u8, self.dither, self.smooth).hash(&mut h);
        for v in [
            self.zoom,
            self.ox,
            self.oy,
            self.wx,
            self.wy,
            self.brightness,
            self.contrast,
            self.saturation,
        ] {
            v.to_bits().hash(&mut h);
        }
        h.finish()
    }

    fn colors(&self, rgba: &mut [u8]) {
        if self.brightness == 0.0 && self.contrast == 0.0 && self.saturation == 0.0 {
            return;
        }
        let c = self.contrast.clamp(-99.0, 99.0);
        let factor = (259.0 * (c + 255.0)) / (255.0 * (259.0 - c));
        let sat = 1.0 + self.saturation / 100.0;
        for px in rgba.chunks_exact_mut(4) {
            let rgb = [px[0] as f32, px[1] as f32, px[2] as f32];
            let gray = 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2];
            for (o, v) in px.iter_mut().zip(rgb) {
                let v = gray + (v - gray) * sat;
                let v = factor * (v - 128.0) + 128.0 + self.brightness * 2.55;
                *o = v.clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// A picture being imported: the source and the last result, kept until an option changes it.
#[derive(Clone)]
struct Draft {
    name: String,
    source: std::sync::Arc<image::DynamicImage>,
    adjust: Adjust,
    /// The edge being dragged, if the drag began on one.
    grab: Option<(i8, i8)>,
    result: Option<(u64, std::sync::Arc<Vec<u8>>, TextureHandle)>,
}

/// How an imported picture is made to fit the square map.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum Fit {
    /// Squashed to the full map.
    Stretch,
    /// Whole picture, keeping its proportions; the rest of the map stays transparent.
    Fit,
    /// Fills the map, cutting off what overflows.
    Fill,
}

impl Fit {
    const ALL: [Fit; 3] = [Fit::Fit, Fit::Fill, Fit::Stretch];

    fn label(self) -> &'static str {
        match self {
            Fit::Stretch => "Stretch",
            Fit::Fit => "Fit (keep proportions)",
            Fit::Fill => "Fill (crop)",
        }
    }
}

/// Every id a map can draw (colors 1 up, four shades each) with its color.
fn map_palette() -> Vec<(u8, [f32; 3])> {
    (4u8..(MAP_BASES.len() as u8 * 4))
        .map(|id| {
            let c = map_rgba(id);
            (id, [c[0] as f32, c[1] as f32, c[2] as f32])
        })
        .collect()
}

/// The map id closest to a color (green counts most, as the eye is most sensitive to it).
fn nearest(palette: &[(u8, [f32; 3])], c: [f32; 3]) -> (u8, [f32; 3]) {
    let mut best = (0u8, [0.0; 3]);
    let mut best_d = f32::MAX;
    for &(id, p) in palette {
        let d = 2.0 * (c[0] - p[0]).powi(2) + 4.0 * (c[1] - p[1]).powi(2) + 3.0 * (c[2] - p[2]).powi(2);
        if d < best_d {
            best_d = d;
            best = (id, p);
        }
    }
    best
}

/// Map ids for a 128 x 128 RGBA picture; mostly transparent pixels become id 0. With `dither`
/// the color error of each pixel is passed to its neighbors (Floyd-Steinberg).
fn quantize(rgba: &[u8], dither: bool) -> Vec<u8> {
    const N: usize = 128;
    let palette = map_palette();
    let mut work: Vec<[f32; 3]> = rgba
        .chunks_exact(4)
        .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
        .collect();
    let mut ids = vec![0u8; N * N];
    for y in 0..N {
        for x in 0..N {
            let i = y * N + x;
            if rgba[i * 4 + 3] < 128 {
                continue;
            }
            let c = work[i].map(|v| v.clamp(0.0, 255.0));
            let (id, got) = nearest(&palette, c);
            ids[i] = id;
            if dither {
                let err = [c[0] - got[0], c[1] - got[1], c[2] - got[2]];
                let mut spread = |dx: isize, dy: usize, k: f32| {
                    let (nx, ny) = (x as isize + dx, y + dy);
                    if (0..N as isize).contains(&nx) && ny < N {
                        let j = ny * N + nx as usize;
                        for (w, e) in work[j].iter_mut().zip(err) {
                            *w += e * k;
                        }
                    }
                };
                spread(1, 0, 7.0 / 16.0);
                spread(-1, 1, 3.0 / 16.0);
                spread(0, 1, 5.0 / 16.0);
                spread(1, 1, 1.0 / 16.0);
            }
        }
    }
    ids
}

/// The picture placed on a 128 x 128 transparent canvas as RGBA bytes: sized by the fit mode and
/// zoom, centerd, then moved by the offset. Anything off the canvas is cut away.
fn place(img: &image::DynamicImage, a: &Adjust) -> Vec<u8> {
    use image::imageops::FilterType::{CatmullRom, Nearest};
    const N: f32 = 128.0;
    let (w, h) = extent(img, a);
    let tw = w.round().clamp(1.0, 4096.0) as u32;
    let th = h.round().clamp(1.0, 4096.0) as u32;
    let scaled = img.resize_exact(tw, th, if a.smooth { CatmullRom } else { Nearest }).to_rgba8();
    let mut canvas = image::RgbaImage::new(128, 128);
    let x = (N / 2.0 - tw as f32 / 2.0 + a.ox).round() as i64;
    let y = (N / 2.0 - th as f32 / 2.0 + a.oy).round() as i64;
    image::imageops::replace(&mut canvas, &scaled, x, y);
    canvas.into_raw()
}

/// Asks for a picture to import; large ones are shrunk first, since the map is only 128 x 128.
fn pick_image(p: &Panel) -> Option<Draft> {
    let path = rfd::FileDialog::new()
        .set_title("Import image")
        .add_filter("Pictures", &["png", "jpg", "jpeg"])
        .pick_file()?;
    let img = match image::open(&path) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Could not read the picture for {}: {e}", p.name);
            return None;
        }
    };
    let img = if img.width().max(img.height()) > 512 {
        img.resize(512, 512, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    Some(Draft {
        name: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
        source: std::sync::Arc::new(img),
        adjust: Adjust::default(),
        grab: None,
        result: None,
    })
}

/// Brings the draft's result up to date with its options.
fn refresh(ctx: &egui::Context, p: &Panel, d: &mut Draft) {
    let key = d.adjust.key();
    if d.result.as_ref().is_some_and(|(k, _, _)| *k == key) {
        return;
    }
    let mut rgba = place(&d.source, &d.adjust);
    d.adjust.colors(&mut rgba);
    let ids = quantize(&rgba, d.adjust.dither);
    let mut px = Vec::with_capacity(ids.len() * 4);
    for id in &ids {
        px.extend_from_slice(&map_rgba(*id));
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([128, 128], &px);
    let tex = match d.result.take() {
        Some((_, _, mut t)) => {
            t.set(image, egui::TextureOptions::NEAREST);
            t
        }
        None => ctx.load_texture(
            format!("array-import-{}", p.key),
            image,
            egui::TextureOptions::NEAREST,
        ),
    };
    d.result = Some((key, std::sync::Arc::new(ids), tex));
    ctx.request_repaint();
}

/// A 16 x 16 block of editable values starting at `base`, shaded from dark (lowest) to light.
fn plane<E: Elem>(
    ui: &mut Ui,
    p: &Panel,
    values: &mut [E],
    pk: Packing,
    base: usize,
    max: i64,
    out: &mut Outcome,
) {
    let count = match pk {
        Packing::Nibble => values.len() * 2,
        Packing::Height9 => values.len() * 7,
        Packing::Direct => values.len(),
    };
    let top = (0..256)
        .filter(|k| base + k < count)
        .map(|k| read(values, pk, base + k))
        .max()
        .unwrap_or(1)
        .max(1);
    let cell = if max > 99 { 40.0 } else { 34.0 };
    ui.spacing_mut().item_spacing.y = 2.0;
    for z in 0..16 {
        ui.horizontal(|ui| {
            flat(ui);
            for x in 0..16 {
                let i = base + z * 16 + x;
                if i >= count {
                    break;
                }
                let mut val = read(values, pk, i);
                let t = (val.max(0) as f32 / top as f32).clamp(0.0, 1.0);
                let fill = Color32::from_rgb(
                    (28.0 + 60.0 * t) as u8,
                    (40.0 + 120.0 * t) as u8,
                    (56.0 + 50.0 * t) as u8,
                );
                let resp = ui
                    .scope(|ui| {
                        ui.style_mut().visuals.widgets.inactive.weak_bg_fill = fill;
                        ui.style_mut().visuals.widgets.inactive.bg_fill = fill;
                        ui.add_enabled_ui(!p.read_only, |ui| {
                            ui.add_sized(
                                [cell, CELL_H],
                                egui::DragValue::new(&mut val).range(0..=max),
                            )
                        })
                        .inner
                    })
                    .inner;
                note_edit(&resp, p, out);
                if resp.changed() {
                    write(values, pk, i, val);
                }
            }
        });
    }
}

/// The range the tools work on, clamped to the array.
fn range(st: &State, len: usize) -> (usize, usize) {
    let to = st.to.unwrap_or(len).min(len);
    (st.from.min(to), to)
}

fn text_of<E: Elem>(values: &[E]) -> String {
    let mut s = String::with_capacity(values.len() * 4);
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            s.push(if i % 16 == 0 { '\n' } else { ' ' });
        }
        s.push_str(&v.get().to_string());
    }
    s
}

fn parse_numbers<E: Elem>(text: &str) -> Result<Vec<E>, String> {
    let mut out = Vec::new();
    for tok in text.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()) {
        let n = match tok.strip_prefix("0x").or_else(|| tok.strip_prefix("0X")) {
            Some(h) => i64::from_str_radix(h, 16),
            None => tok.parse::<i64>(),
        }
        .map_err(|_| format!("Could not read \"{tok}\" as a number"))?;
        if !(E::LO..=E::HI).contains(&n) {
            return Err(format!("{n} does not fit ({} to {})", E::LO, E::HI));
        }
        out.push(E::make(n));
    }
    Ok(out)
}

fn tools<E: Elem>(ui: &mut Ui, p: &Panel, st: &mut State, values: &mut Vec<E>, out: &mut Outcome) {
    let len = values.len();
    st.from = st.from.min(len);
    let mut to = st.to.unwrap_or(len).min(len);
    let editable = !p.read_only;
    egui::Frame::new()
        .fill(theme::field())
        .corner_radius(theme::radius(4))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Range");
                ui.add(egui::DragValue::new(&mut st.from).range(0..=to));
                ui.label("to");
                ui.add(egui::DragValue::new(&mut to).range(st.from..=len));
                st.to = (to != len).then_some(to);
                if ui.small_button("All").clicked() {
                    st.from = 0;
                    st.to = None;
                }
            });
            let (from, to) = range(st, len);
            ui.horizontal_wrapped(|ui| {
                ui.label("Value");
                ui.add(egui::DragValue::new(&mut st.fill).range(E::LO..=E::HI));
                if ui.add_enabled(editable, egui::Button::new("Fill range")).clicked() {
                    out.snapshot = true;
                    out.changed = true;
                    values[from..to].fill(E::make(st.fill));
                }
                if ui.add_enabled(editable, egui::Button::new("Zero all")).clicked() {
                    out.snapshot = true;
                    out.changed = true;
                    values.fill(E::make(0));
                }
            });
            ui.horizontal_wrapped(|ui| {
                let mut n = st.new_len.unwrap_or(len);
                ui.label("Length");
                ui.add(egui::DragValue::new(&mut n).range(0..=u32::MAX as usize));
                st.new_len = (n != len).then_some(n);
                if ui
                    .add_enabled(editable && n != len, egui::Button::new("Resize"))
                    .on_hover_text("New values are zero; a shorter array drops the end")
                    .clicked()
                {
                    out.snapshot = true;
                    out.changed = true;
                    values.resize(n, E::make(0));
                    st.new_len = None;
                    st.to = None;
                    st.page = 0;
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Edit as text\u{2026}").clicked() {
                    st.text = Some(text_of(&values[from..to]));
                }
                if ui.button("Copy").on_hover_text("The range as numbers").clicked() {
                    ui.ctx().copy_text(text_of(&values[from..to]));
                    out.note = Some(format!("Copied {} values", to - from));
                }
                if ui
                    .add_enabled(editable, egui::Button::new("Import\u{2026}"))
                    .on_hover_text(format!(
                        "Replace the array with a file of {}-byte big-endian values",
                        E::WIDTH
                    ))
                    .clicked()
                {
                    import(p, values, st, out);
                }
                if ui
                    .button("Export\u{2026}")
                    .on_hover_text(format!(
                        "Save the array as {}-byte big-endian values",
                        E::WIDTH
                    ))
                    .clicked()
                {
                    export(p, values, out);
                }
            });
        });
    text_window(ui, p, st, values, out);
}

fn import<E: Elem>(p: &Panel, values: &mut Vec<E>, st: &mut State, out: &mut Outcome) {
    let Some(path) = rfd::FileDialog::new().set_title("Import array").pick_file() else {
        return;
    };
    match std::fs::read(&path) {
        Err(e) => out.note = Some(format!("Could not read the file: {e}")),
        Ok(b) if b.len() % E::WIDTH != 0 => {
            out.note = Some(format!(
                "The file is {} bytes, not a whole number of {}-byte values",
                b.len(),
                E::WIDTH
            ));
        }
        Ok(b) => {
            out.snapshot = true;
            out.changed = true;
            *values = b.chunks_exact(E::WIDTH).map(E::from_be).collect();
            out.note = Some(format!("Imported {} values into {}", values.len(), p.name));
            st.page = 0;
            st.to = None;
            st.new_len = None;
        }
    }
}

fn export<E: Elem>(p: &Panel, values: &[E], out: &mut Outcome) {
    let name = if p.name.is_empty() { "array" } else { p.name };
    let Some(path) = rfd::FileDialog::new()
        .set_title("Export array")
        .set_file_name(format!("{name}.bin"))
        .save_file()
    else {
        return;
    };
    let mut bytes = Vec::with_capacity(values.len() * E::WIDTH);
    for v in values {
        v.push_be(&mut bytes);
    }
    out.note = Some(match std::fs::write(&path, &bytes) {
        Ok(()) => format!("Exported {} values", values.len()),
        Err(e) => format!("Could not write the file: {e}"),
    });
}

/// The range as editable text; Apply swaps the range for what the text says.
fn text_window<E: Elem>(
    ui: &mut Ui,
    p: &Panel,
    st: &mut State,
    values: &mut Vec<E>,
    out: &mut Outcome,
) {
    let Some(mut text) = st.text.take() else {
        return;
    };
    p.typing.set(true);
    let (from, to) = range(st, values.len());
    let (mut open, mut apply, mut cancel) = (true, false, false);
    egui::Window::new(format!("{} as text", p.name))
        .id(egui::Id::new(("array-text", p.key)))
        .open(&mut open)
        .default_width(560.0)
        .collapsible(false)
        .show(ui.ctx(), |ui| {
            ui.label(
                egui::RichText::new(format!(
                    "Values {from} to {to}. Numbers separated by spaces, commas or new lines; 0x for hex."
                ))
                .color(theme::text_muted()),
            );
            egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .font(egui::FontId::monospace(13.0))
                        .desired_width(f32::INFINITY)
                        .desired_rows(14),
                );
            });
            ui.horizontal(|ui| {
                apply = ui
                    .add_enabled(!p.read_only, egui::Button::new("Apply"))
                    .clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
    if apply {
        match parse_numbers::<E>(&text) {
            Ok(new) => {
                out.snapshot = true;
                out.changed = true;
                let n = new.len();
                values.splice(from..to, new);
                out.note = Some(format!("{n} values written"));
                st.to = None;
                st.new_len = None;
                return;
            }
            Err(e) => {
                out.note = Some(e);
            }
        }
    }
    if open && !cancel && !apply {
        st.text = Some(text);
    } else if apply {
        // Left open so the mistake can be fixed.
        st.text = Some(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nibbles_pack_low_first() {
        let mut v = vec![0i8; 2];
        write(&mut v, Packing::Nibble, 0, 5);
        write(&mut v, Packing::Nibble, 1, 12);
        write(&mut v, Packing::Nibble, 2, 15);
        assert_eq!(v[0] as u8, 0xC5);
        assert_eq!(read(&v, Packing::Nibble, 1), 12);
        assert_eq!(read(&v, Packing::Nibble, 2), 15);
        assert_eq!(read(&v, Packing::Nibble, 3), 0);
    }

    #[test]
    fn heights_pack_seven_to_a_long() {
        let mut v = vec![0i64; 37];
        for i in 0..256 {
            write(&mut v, Packing::Height9, i, (i as i64 * 2) % 400);
        }
        for i in 0..256 {
            assert_eq!(read(&v, Packing::Height9, i), (i as i64 * 2) % 400);
        }
        // The top bit of each long is unused.
        assert!(v.iter().all(|l| *l >= 0));
    }

    #[test]
    fn text_round_trips() {
        let v: Vec<i32> = vec![1, -2, 300, 0];
        assert_eq!(parse_numbers::<i32>(&text_of(&v)).unwrap(), v);
        assert_eq!(parse_numbers::<i8>("0x7f -128").unwrap(), vec![127, -128]);
        assert!(parse_numbers::<i8>("200").is_err());
        assert!(parse_numbers::<i8>("x").is_err());
    }

    #[test]
    fn map_ids_split_into_color_and_shade() {
        assert_eq!(map_rgba(0), [0, 0, 0, 0]);
        // Grass, brightest shade.
        assert_eq!(map_rgba(4 + 2), [127, 178, 56, 255]);
        // Darkest shade of grass.
        assert_eq!(map_rgba(4 + 3), [67, 94, 29, 255]);
        assert_eq!(map_rgba(250), [255, 0, 255, 255]);
    }

    #[test]
    fn pictures_become_map_ids() {
        let mut rgba = vec![0u8; 128 * 128 * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            // Left half grass green, right half see-through.
            if i % 128 < 64 {
                px.copy_from_slice(&[127, 178, 56, 255]);
            }
        }
        for dither in [false, true] {
            let ids = quantize(&rgba, dither);
            assert_eq!(ids[0], 4 + 2, "grass at its brightest shade");
            assert_eq!(ids[127], 0, "transparent stays transparent");
            assert_eq!(ids.len(), 128 * 128);
        }
        // Pure white is snow, whose brightest shade is exactly white.
        let white = vec![255u8; 128 * 128 * 4];
        assert_eq!(quantize(&white, false)[0], 8 * 4 + 2);
    }

    #[test]
    fn pictures_zoom_and_move_on_the_map() {
        let red = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            64,
            32,
            image::Rgba([255, 0, 0, 255]),
        ));
        let alpha = |px: &[u8], x: usize, y: usize| px[(y * 128 + x) * 4 + 3];
        // Fit: 2:1 picture across the width, bars above and below.
        let mut a = Adjust::default();
        let px = place(&red, &a);
        assert_eq!(alpha(&px, 64, 64), 255);
        assert_eq!(alpha(&px, 64, 10), 0);
        // Zoomed out it is smaller; zoomed in it fills the map.
        a.zoom = 0.5;
        assert_eq!(alpha(&place(&red, &a), 10, 64), 0);
        a.zoom = 4.0;
        assert_eq!(alpha(&place(&red, &a), 10, 10), 255);
        // Moving right leaves the left edge empty.
        a.zoom = 1.0;
        a.ox = 40.0;
        assert_eq!(alpha(&place(&red, &a), 10, 64), 0);
        assert_eq!(alpha(&place(&red, &a), 120, 64), 255);
        // Fill covers the whole map; stretch too.
        for fit in [Fit::Fill, Fit::Stretch] {
            let a = Adjust { fit, ..Adjust::default() };
            assert_eq!(alpha(&place(&red, &a), 3, 3), 255);
        }
    }

    #[test]
    fn views_follow_name_and_size() {
        assert!(views::<i8>("colors", 16384).contains(&View::Map));
        assert!(views::<i8>("SkyLight", 2048).contains(&View::Nibbles));
        assert!(views::<i64>("MOTION_BLOCKING", 37).contains(&View::Plane));
        assert_eq!(views::<i8>("other", 10), vec![View::Hex]);
        assert_eq!(default_view(&views::<i8>("colors", 16384), "colors"), View::Map);
    }
}
