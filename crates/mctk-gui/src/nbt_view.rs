//! The NBT tree editor: toolbar (undo/redo, add tags by type, find), rows with type
//! badges, inline value editing, readable hints (game mode, dates, durations), and a
//! context menu for rename, reorder, copy/paste and delete.

use std::collections::{HashMap, HashSet};
use std::io::Write;

use eframe::egui::{self, Align2, Color32, Id, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use mctk_nbt::{Compound, Compression, Flavor, List, NamedTag, Tag, TagType};

use crate::icons::{self, Icon};
use crate::theme;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Seg {
    Key(String),
    Index(usize),
}

pub type TagPath = Vec<Seg>;

/// How an NBT blob is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Java(Compression),
    /// Little-endian; `Some(version)` with Bedrock's 8-byte `level.dat` header.
    Bedrock(Option<u32>),
}

pub fn load(bytes: &[u8]) -> Result<(Format, NamedTag), String> {
    if bytes.len() > 9 {
        let len = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        if len == bytes.len() - 8 && bytes[8] == 0x0a {
            let version = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
            let root =
                mctk_nbt::from_bytes(&bytes[8..], Flavor::Bedrock).map_err(|e| e.to_string())?;
            return Ok((Format::Bedrock(Some(version)), root));
        }
    }
    let (compression, raw) = mctk_nbt::decompress(bytes).map_err(|e| e.to_string())?;
    match mctk_nbt::from_bytes(&raw, Flavor::Java) {
        Ok(root) => Ok((Format::Java(compression), root)),
        Err(java_err) => mctk_nbt::from_bytes(&raw, Flavor::Bedrock)
            .map(|root| (Format::Bedrock(None), root))
            .map_err(|_| java_err.to_string()),
    }
}

pub fn encode(format: Format, root: &NamedTag) -> Result<Vec<u8>, String> {
    match format {
        Format::Java(c) => {
            let raw = mctk_nbt::to_bytes(root, Flavor::Java).map_err(|e| e.to_string())?;
            compress(c, raw)
        }
        Format::Bedrock(header) => {
            let body = mctk_nbt::to_bytes(root, Flavor::Bedrock).map_err(|e| e.to_string())?;
            Ok(match header {
                Some(v) => {
                    let mut out = v.to_le_bytes().to_vec();
                    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
                    out.extend_from_slice(&body);
                    out
                }
                None => body,
            })
        }
    }
}

pub fn compress(c: Compression, raw: Vec<u8>) -> Result<Vec<u8>, String> {
    let e = |e: std::io::Error| e.to_string();
    Ok(match c {
        Compression::None => raw,
        Compression::Gzip => {
            let mut z = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            z.write_all(&raw).map_err(e)?;
            z.finish().map_err(e)?
        }
        Compression::Zlib => {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            z.write_all(&raw).map_err(e)?;
            z.finish().map_err(e)?
        }
    })
}

fn get_mut<'a>(tag: &'a mut Tag, path: &[Seg]) -> Option<&'a mut Tag> {
    let Some((first, rest)) = path.split_first() else {
        return Some(tag);
    };
    let child = match (tag, first) {
        (Tag::Compound(c), Seg::Key(k)) => c.get_mut(k)?,
        (Tag::List(l), Seg::Index(i)) => l.get_mut(*i)?,
        _ => return None,
    };
    get_mut(child, rest)
}

fn default_tag(t: TagType) -> Option<Tag> {
    Some(match t {
        TagType::Byte => Tag::Byte(0),
        TagType::Short => Tag::Short(0),
        TagType::Int => Tag::Int(0),
        TagType::Long => Tag::Long(0),
        TagType::Float => Tag::Float(0.0),
        TagType::Double => Tag::Double(0.0),
        TagType::String => Tag::String(String::new()),
        TagType::ByteArray => Tag::ByteArray(Vec::new()),
        TagType::IntArray => Tag::IntArray(Vec::new()),
        TagType::LongArray => Tag::LongArray(Vec::new()),
        TagType::List => Tag::List(List::new(TagType::End)),
        TagType::Compound => Tag::Compound(Compound::new()),
        TagType::End => return None,
    })
}

const ADDABLE: [(TagType, &str); 12] = [
    (TagType::Compound, "Compound"),
    (TagType::List, "List"),
    (TagType::String, "String"),
    (TagType::Byte, "Byte"),
    (TagType::Short, "Short"),
    (TagType::Int, "Int"),
    (TagType::Long, "Long"),
    (TagType::Float, "Float"),
    (TagType::Double, "Double"),
    (TagType::ByteArray, "Byte Array"),
    (TagType::IntArray, "Int Array"),
    (TagType::LongArray, "Long Array"),
];

fn type_name(t: TagType) -> &'static str {
    match t {
        TagType::Byte => "byte",
        TagType::Short => "short",
        TagType::Int => "int",
        TagType::Long => "long",
        TagType::Float => "float",
        TagType::Double => "double",
        TagType::String => "string",
        TagType::ByteArray => "byte array",
        TagType::IntArray => "int array",
        TagType::LongArray => "long array",
        TagType::List => "list",
        TagType::Compound => "compound",
        TagType::End => "end",
    }
}

/// Colours extensions name: Qt/SVG colour names or #rrggbb.
pub fn named_color(name: &str) -> Option<Color32> {
    let n = name.trim().to_ascii_lowercase();
    if let Some(hex) = n.strip_prefix('#') {
        let v = u32::from_str_radix(hex, 16).ok()?;
        return (hex.len() == 6)
            .then(|| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
    }
    Some(match n.as_str() {
        "lime" => Color32::from_rgb(0x00, 0xFF, 0x00),
        "green" => Color32::from_rgb(0x00, 0x80, 0x00),
        "gold" => Color32::from_rgb(0xFF, 0xD7, 0x00),
        "yellow" => Color32::from_rgb(0xFF, 0xFF, 0x00),
        "orange" => Color32::from_rgb(0xFF, 0xA5, 0x00),
        "red" => Color32::from_rgb(0xFF, 0x00, 0x00),
        "darkred" => Color32::from_rgb(0x8B, 0x00, 0x00),
        "cyan" | "aqua" => Color32::from_rgb(0x00, 0xFF, 0xFF),
        "blue" => Color32::from_rgb(0x00, 0x00, 0xFF),
        "lightblue" => Color32::from_rgb(0xAD, 0xD8, 0xE6),
        "magenta" | "fuchsia" => Color32::from_rgb(0xFF, 0x00, 0xFF),
        "purple" => Color32::from_rgb(0x80, 0x00, 0x80),
        "pink" => Color32::from_rgb(0xFF, 0xC0, 0xCB),
        "white" => Color32::WHITE,
        "black" => Color32::BLACK,
        "gray" | "grey" => Color32::from_rgb(0x80, 0x80, 0x80),
        "lightgray" | "lightgrey" | "silver" => Color32::from_rgb(0xC0, 0xC0, 0xC0),
        "darkgray" | "darkgrey" => Color32::from_rgb(0xA9, 0xA9, 0xA9),
        "brown" => Color32::from_rgb(0xA5, 0x2A, 0x2A),
        _ => return None,
    })
}

/// Display order of tag types within a compound.
fn type_rank(t: TagType) -> u8 {
    match t {
        TagType::Compound => 0,
        TagType::List => 1,
        TagType::Byte => 2,
        TagType::Short => 3,
        TagType::Int => 4,
        TagType::Long => 5,
        TagType::Float => 6,
        TagType::Double => 7,
        TagType::String => 8,
        TagType::ByteArray => 9,
        TagType::IntArray => 10,
        TagType::LongArray => 11,
        TagType::End => 12,
    }
}

/// Paints the icon or letter badge for a tag type.
fn type_icon(p: &egui::Painter, r: Rect, t: TagType) {
    // UMT's tag badges, falling back to drawn ones.
    let image = match t {
        TagType::Compound => "TAG_COMPOUND",
        TagType::List => "TAG_LIST",
        TagType::String => "TAG_STRING",
        TagType::Byte => "TAG_BYTE",
        TagType::Short => "TAG_SHORT",
        TagType::Int => "TAG_INT",
        TagType::Long => "TAG_LONG",
        TagType::Float => "TAG_FLOAT",
        TagType::Double => "TAG_DOUBLE",
        TagType::ByteArray => "TAG_BYTE_ARRAY",
        TagType::IntArray => "TAG_INT_ARRAY",
        TagType::LongArray => "TAG_LONG_ARRAY",
        TagType::End => return,
    };
    if icons::image(p, r, image, Color32::WHITE) {
        return;
    }
    match t {
        TagType::Compound => icons::paint(p, r, Icon::Compound, theme::FOLDER),
        TagType::List => icons::paint(p, r, Icon::List, theme::TEXT),
        TagType::String => icons::badge(p, r, "Aa", Color32::TRANSPARENT, false),
        TagType::Byte => icons::badge(p, r, "B", theme::BADGE_BYTE, false),
        TagType::Short => icons::badge(p, r, "S", theme::BADGE_SHORT, false),
        TagType::Int => icons::badge(p, r, "I", theme::BADGE_INT, false),
        TagType::Long => icons::badge(p, r, "L", theme::BADGE_LONG, false),
        TagType::Float => icons::badge(p, r, "F", theme::BADGE_FLOAT, false),
        TagType::Double => icons::badge(p, r, "D", theme::BADGE_DOUBLE, false),
        TagType::ByteArray => icons::badge(p, r, "B", theme::BADGE_BYTE, true),
        TagType::IntArray => icons::badge(p, r, "I", theme::BADGE_INT, true),
        TagType::LongArray => icons::badge(p, r, "L", theme::BADGE_LONG, true),
        TagType::End => {}
    }
}

/// Days since 1970-01-01 → (year, month, day) (Howard Hinnant's civil_from_days).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

fn date(ms: i64) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let secs = ms.div_euclid(1000);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    let (h, min) = (t / 3600, t % 3600 / 60);
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    // UTC, as UMT shows it.
    format!(
        "{} {d:02}, {y}, {h12:02}:{min:02}{ampm}",
        MONTHS[(m - 1) as usize]
    )
}

fn duration(ticks: i64) -> String {
    let s = ticks / 20;
    match s {
        0..=59 => format!("{s} seconds"),
        60..=3599 => format!("{} minutes", s / 60),
        3600..=86_399 => format!("{:.1} hours", s as f64 / 3600.0),
        _ => format!("{:.1} days", s as f64 / 86_400.0),
    }
}

/// A readable gloss for well-known tags, shown after the value.
fn hint(key: &str, tag: &Tag) -> Option<(String, bool)> {
    let n = tag.as_i64()?;
    match key {
        "GameType" | "playerGameType" | "ForceGameType" => {
            let mode = ["Survival", "Creative", "Adventure", "Spectator"].get(n as usize)?;
            Some((mode.to_string(), true))
        }
        "Difficulty" => Some((
            ["Peaceful", "Easy", "Normal", "Hard"]
                .get(n as usize)?
                .to_string(),
            true,
        )),
        "LastPlayed" if n > 100_000_000_000 => Some((date(n), false)),
        "LastPlayed" if n > 1_000_000_000 => Some((date(n * 1000), false)),
        "Time" | "DayTime" | "InhabitedTime" | "currentTick" => Some((duration(n), false)),
        _ => None,
    }
}

enum Action {
    Snapshot,
    /// A value was edited in place.
    Changed,
    Select(TagPath),
    Toggle(String),
    Delete(TagPath),
    Move(TagPath, isize),
    StartRename(TagPath, String),
    Add(TagPath, TagType),
    Copy(TagPath),
    Paste(TagPath),
}

/// One open NBT document.
pub struct NbtTree {
    pub root: NamedTag,
    pub dirty: bool,
    pub read_only: bool,
    undo: Vec<NamedTag>,
    redo: Vec<NamedTag>,
    frame_start: Option<NamedTag>,
    expanded: HashSet<String>,
    selected: Option<TagPath>,
    find: String,
    finding: bool,
    rename: Option<(TagPath, String)>,
    clipboard: Option<(String, Tag)>,
    status: String,
    /// Labels and icons from style extensions, by tag path.
    pub styles: HashMap<TagPath, mctk_lua::style::TagStyle>,
    /// Bumped on every edit, so styles can be refreshed.
    pub generation: u64,
}

fn path_key(p: &TagPath) -> String {
    format!("{p:?}")
}

struct RowCtx<'a> {
    find: &'a str,
    selected: &'a Option<TagPath>,
    expanded: &'a HashSet<String>,
    full_precision: bool,
    read_only: bool,
    width: f32,
    styles: &'a HashMap<TagPath, mctk_lua::style::TagStyle>,
    extensions_dir: &'a std::path::Path,
}

const ROW: f32 = 18.0;
const INDENT: f32 = 18.0;

impl NbtTree {
    pub fn new(root: NamedTag) -> Self {
        let mut expanded = HashSet::new();
        expanded.insert(path_key(&Vec::new()));
        // Open the first level, as UMT does for level.dat's Data compound.
        if let Some(c) = root.tag.as_compound() {
            if c.len() == 1 {
                if let Some(k) = c.keys().next() {
                    expanded.insert(path_key(&vec![Seg::Key(k.clone())]));
                }
            }
        }
        NbtTree {
            root,
            dirty: false,
            read_only: false,
            undo: Vec::new(),
            redo: Vec::new(),
            frame_start: None,
            expanded,
            selected: None,
            find: String::new(),
            finding: false,
            rename: None,
            clipboard: None,
            status: String::new(),
            styles: HashMap::new(),
            generation: 0,
        }
    }

    fn push_undo(&mut self, before: NamedTag) {
        self.undo.push(before);
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
        self.generation += 1;
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
        ui.painter().rect_filled(bar, 0.0, theme::BG);
        ui.painter().line_segment(
            [bar.left_bottom(), bar.right_bottom()],
            Stroke::new(1.0_f32, theme::BORDER),
        );
        let mut x = bar.left() + 8.0;
        let y = bar.center().y;
        let button = |ui: &mut Ui,
                      x: &mut f32,
                      id: &str,
                      enabled: bool,
                      paint: &dyn Fn(&egui::Painter, Rect, Color32),
                      tip: &str|
         -> bool {
            let r = Rect::from_center_size(pos2(*x + 12.0, y), Vec2::splat(24.0));
            *x += 26.0;
            let resp = ui.interact(
                r,
                Id::new(("nbt-tool", id)),
                if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            if resp.hovered() && enabled {
                ui.painter()
                    .rect_filled(r, theme::radius(3), theme::BUTTON_HOVER);
            }
            let c = if enabled {
                theme::TEXT_MUTED
            } else {
                theme::CHIP
            };
            paint(ui.painter(), r.shrink(5.0), c);
            let clicked = resp.clicked();
            resp.on_hover_text(tip);
            clicked
        };
        let editable = !self.read_only;
        if button(
            ui,
            &mut x,
            "undo",
            editable && !self.undo.is_empty(),
            &|p, r, c| icons::paint(p, r, Icon::Undo, c),
            "Undo",
        ) {
            if let (Some(prev), cur) = (self.undo.pop(), self.root.clone()) {
                self.redo.push(cur);
                self.root = prev;
                self.dirty = true;
            }
        }
        if button(
            ui,
            &mut x,
            "redo",
            editable && !self.redo.is_empty(),
            &|p, r, c| icons::paint(p, r, Icon::Redo, c),
            "Redo",
        ) {
            if let (Some(next), cur) = (self.redo.pop(), self.root.clone()) {
                self.undo.push(cur);
                self.root = next;
                self.dirty = true;
            }
        }
        ui.painter().line_segment(
            [pos2(x + 4.0, y - 10.0), pos2(x + 4.0, y + 10.0)],
            Stroke::new(1.0_f32, theme::BORDER),
        );
        x += 10.0;
        let target = self.add_target();
        for (t, name) in ADDABLE {
            let enabled = editable
                && target
                    .as_ref()
                    .is_some_and(|(_, only)| only.is_none_or(|o| o == t));
            let paint = move |p: &egui::Painter, r: Rect, c: Color32| {
                let _ = c;
                if enabled {
                    type_icon(p, r, t);
                } else {
                    // Faded, as UMT shows the types that can't be added to the selection.
                    let mut faded = p.clone();
                    faded.multiply_opacity(0.3);
                    type_icon(&faded, r, t);
                }
            };
            if button(ui, &mut x, name, enabled, &paint, &format!("Add {name}")) {
                if let Some((path, _)) = target.clone() {
                    self.frame_start = Some(self.root.clone());
                    self.apply(Action::Add(path, t));
                }
            }
        }
        let sr = Rect::from_center_size(pos2(bar.right() - 20.0, y), Vec2::splat(24.0));
        let resp = ui.interact(sr, Id::new("nbt-find"), Sense::click());
        icons::paint(
            ui.painter(),
            sr.shrink(4.0),
            Icon::Search,
            if self.finding {
                theme::TEXT
            } else {
                theme::TEXT_MUTED
            },
        );
        if resp.clicked() {
            self.finding = !self.finding;
            if !self.finding {
                self.find.clear();
            }
        }
        if self.finding {
            let fr = Rect::from_min_size(pos2(sr.left() - 210.0, y - 13.0), vec2(200.0, 26.0));
            ui.put(
                fr,
                egui::TextEdit::singleline(&mut self.find)
                    .hint_text("Find tag or value")
                    .background_color(theme::DEEP),
            );
        }
    }

    /// The selected container (or the root) that new tags are added to.
    /// Where the toolbar's add buttons insert, and the only type allowed there (as UMT's
    /// updateActionStates): a selected compound takes any type, a list only its element
    /// type (any while it has none), and other tags nothing.
    fn add_target(&mut self) -> Option<(TagPath, Option<TagType>)> {
        let path = self.selected.clone()?;
        match get_mut(&mut self.root.tag, &path)? {
            Tag::Compound(_) => Some((path, None)),
            Tag::List(l) => {
                let only = (l.element_type() != TagType::End).then(|| l.element_type());
                Some((path, only))
            }
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        ui: &mut Ui,
        cx: &RowCtx,
        label: &str,
        tag: &mut Tag,
        path: TagPath,
        in_compound: bool,
        actions: &mut Vec<Action>,
        hits: &mut usize,
    ) {
        let depth = path.len() as f32;
        let (rect, resp) = ui.allocate_exact_size(vec2(cx.width, ROW), Sense::click());
        let key = path_key(&path);
        let selected = cx.selected.as_ref() == Some(&path);
        let found = !cx.find.is_empty() && {
            let f = cx.find.to_lowercase();
            label.to_lowercase().contains(&f)
                || matches!(tag, Tag::String(s) if s.to_lowercase().contains(&f))
        };
        *hits += found as usize;
        let p = ui.painter().clone();
        if selected {
            p.rect_filled(rect, 0.0, Color32::from_rgb(0x26, 0x3A, 0x55));
        } else if found {
            p.rect_filled(rect, 0.0, Color32::from_rgb(0x4A, 0x40, 0x1C));
        } else if resp.hovered() {
            p.rect_filled(rect, 0.0, theme::CARD);
        }
        let mut x = rect.left() + 6.0 + depth * INDENT;
        let container = matches!(
            tag,
            Tag::Compound(_)
                | Tag::List(_)
                | Tag::ByteArray(_)
                | Tag::IntArray(_)
                | Tag::LongArray(_)
        );
        let open = cx.expanded.contains(&key);
        if container {
            let cr = Rect::from_center_size(pos2(x + 5.0, rect.center().y), Vec2::splat(9.0));
            icons::paint(
                &p,
                cr,
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                theme::TEXT_MUTED,
            );
            if resp.clicked()
                && ui
                    .input(|i| i.pointer.interact_pos())
                    .is_some_and(|pos| pos.x < x + 14.0)
            {
                actions.push(Action::Toggle(key.clone()));
            }
        }
        x += 14.0;
        type_icon(
            &p,
            Rect::from_min_size(pos2(x, rect.top() + 2.0), Vec2::splat(14.0)),
            tag.tag_type(),
        );
        x += 20.0;
        let name_galley = p.layout_no_wrap(label.to_string(), theme::regular(13.0), theme::TEXT);
        let name_w = name_galley.size().x;
        p.galley(
            pos2(x, rect.center().y - name_galley.size().y / 2.0),
            name_galley,
            theme::TEXT,
        );
        x += name_w + 6.0;
        if resp.clicked() {
            actions.push(Action::Select(path.clone()));
        }
        if resp.double_clicked() && container {
            actions.push(Action::Toggle(key.clone()));
        }
        let count = |n: usize, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
        let value_rect =
            |w: f32| Rect::from_min_size(pos2(x, rect.top() + 1.0), vec2(w, ROW - 2.0));
        let counted = match &*tag {
            Tag::Compound(c) => Some(count(c.len(), "tag")),
            Tag::List(l) => Some(count(l.len(), "tag")),
            Tag::ByteArray(a) => Some(count(a.len(), "byte")),
            Tag::IntArray(a) => Some(count(a.len(), "int")),
            Tag::LongArray(a) => Some(count(a.len(), "long")),
            _ => None,
        };
        // Where an extension's icon and label (or our own hint) go.
        let mut after;
        let mut gloss = None;
        match &mut *tag {
            _ if counted.is_some() => {
                let g = p.layout_no_wrap(
                    counted.unwrap_or_default(),
                    theme::regular(12.0),
                    theme::TEXT_DIM,
                );
                after = x + g.size().x + 8.0;
                p.galley(
                    pos2(x, rect.center().y - g.size().y / 2.0),
                    g,
                    theme::TEXT_DIM,
                );
            }
            scalar => {
                gloss = hint(label, scalar);
                let text_len = match &*scalar {
                    Tag::String(s) => s.chars().count().clamp(4, 60) as f32 * 7.2 + 14.0,
                    Tag::Long(v) => v.to_string().len() as f32 * 7.6 + 16.0,
                    Tag::Float(_) | Tag::Double(_) => {
                        if cx.full_precision {
                            150.0
                        } else {
                            70.0
                        }
                    }
                    other => other
                        .as_i64()
                        .map_or(40.0, |v| v.to_string().len() as f32 * 7.6 + 18.0),
                };
                let r = value_rect(text_len.max(36.0));
                p.rect_filled(r, theme::radius(2), theme::FIELD);
                // Nudged up so the text sits centred in its box.
                let mut child =
                    ui.new_child(egui::UiBuilder::new().max_rect(r.translate(vec2(0.0, -2.0))));
                child.style_mut().visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                child.style_mut().visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
                child.style_mut().visuals.extreme_bg_color = Color32::TRANSPARENT;
                child.style_mut().spacing.interact_size.y = ROW - 2.0;
                child.style_mut().spacing.button_padding = vec2(4.0, 0.0);
                child.style_mut().override_font_id = Some(theme::regular(13.0));
                let decimals = if cx.full_precision { 17 } else { 4 };
                let resp = child
                    .add_enabled_ui(!cx.read_only, |ui| match scalar {
                        Tag::Byte(v) => ui.add_sized(r.size(), egui::DragValue::new(v)),
                        Tag::Short(v) => ui.add_sized(r.size(), egui::DragValue::new(v)),
                        Tag::Int(v) => ui.add_sized(r.size(), egui::DragValue::new(v)),
                        Tag::Long(v) => ui.add_sized(r.size(), egui::DragValue::new(v)),
                        Tag::Float(v) => ui.add_sized(
                            r.size(),
                            egui::DragValue::new(v).speed(0.1).max_decimals(decimals),
                        ),
                        Tag::Double(v) => ui.add_sized(
                            r.size(),
                            egui::DragValue::new(v).speed(0.1).max_decimals(decimals),
                        ),
                        Tag::String(s) => {
                            ui.add_sized(r.size(), egui::TextEdit::singleline(s).frame(false))
                        }
                        _ => ui.label(""),
                    })
                    .inner;
                if resp.drag_started() || resp.gained_focus() {
                    actions.push(Action::Snapshot);
                }
                if resp.changed() {
                    actions.push(Action::Changed);
                }
                after = r.right() + 10.0;
            }
        }
        // Extension styling (UMT's NBT_EDITOR_STYLE scripts), else our own hint.
        let style = cx.styles.get(&path);
        if let Some(icon) = style.and_then(|s| s.icon.as_ref()) {
            let ir = Rect::from_min_size(pos2(after, rect.top() + 1.0), Vec2::splat(ROW - 2.0));
            if icons::file_image(&p, ir, &cx.extensions_dir.join(icon)) {
                after += ROW + 2.0;
            }
        }
        let label_text = style
            .and_then(|s| {
                s.label
                    .clone()
                    .map(|l| (l, s.color.as_deref().and_then(named_color)))
            })
            .or_else(|| gloss.map(|(t, emphasis)| (t, emphasis.then_some(theme::YELLOW))));
        if let Some((text, color)) = label_text {
            p.text(
                pos2(after, rect.center().y),
                Align2::LEFT_CENTER,
                text,
                theme::italic(12.5),
                color.unwrap_or(Color32::from_rgb(0x5E, 0x68, 0x75)),
            );
        }
        if !cx.read_only {
            resp.context_menu(|ui| Self::menu(ui, &path, label, tag, in_compound, actions));
        }
        if container && open {
            match tag {
                Tag::Compound(c) => {
                    // Shown as UMT does: by type, then name; the stored order is unchanged.
                    let mut keys: Vec<String> = c.keys().cloned().collect();
                    keys.sort_by_cached_key(|k| (type_rank(c[k].tag_type()), k.to_lowercase()));
                    for k in keys {
                        let mut child = path.clone();
                        child.push(Seg::Key(k.clone()));
                        if let Some(v) = c.get_mut(&k) {
                            Self::row(ui, cx, &k, v, child, true, actions, hits);
                        }
                    }
                }
                Tag::List(l) => {
                    for (i, v) in l.iter_mut().enumerate() {
                        let mut child = path.clone();
                        child.push(Seg::Index(i));
                        Self::row(ui, cx, &i.to_string(), v, child, false, actions, hits);
                    }
                }
                Tag::ByteArray(a) => Self::array_rows(ui, cx, depth + 1.0, a, actions),
                Tag::IntArray(a) => Self::array_rows(ui, cx, depth + 1.0, a, actions),
                Tag::LongArray(a) => Self::array_rows(ui, cx, depth + 1.0, a, actions),
                _ => {}
            }
        }
    }

    fn array_rows<T: eframe::emath::Numeric>(
        ui: &mut Ui,
        cx: &RowCtx,
        depth: f32,
        values: &mut [T],
        actions: &mut Vec<Action>,
    ) {
        const SHOWN: usize = 512;
        for (i, v) in values.iter_mut().take(SHOWN).enumerate() {
            let (rect, _) = ui.allocate_exact_size(vec2(cx.width, ROW), Sense::hover());
            let x = rect.left() + 6.0 + depth * INDENT + 34.0;
            ui.painter().text(
                pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                i.to_string(),
                theme::regular(12.5),
                theme::TEXT_MUTED,
            );
            let r = Rect::from_min_size(pos2(x + 40.0, rect.top() + 1.0), vec2(150.0, ROW - 2.0));
            ui.painter().rect_filled(r, theme::radius(2), theme::FIELD);
            // Nudged up so the text sits centred in its box.
            let mut child =
                ui.new_child(egui::UiBuilder::new().max_rect(r.translate(vec2(0.0, -2.0))));
            child.style_mut().visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            child.style_mut().override_font_id = Some(theme::regular(13.0));
            let resp = child.add_enabled(!cx.read_only, egui::DragValue::new(v));
            if resp.drag_started() || resp.gained_focus() {
                actions.push(Action::Snapshot);
            }
        }
        if values.len() > SHOWN {
            let (rect, _) = ui.allocate_exact_size(vec2(cx.width, ROW), Sense::hover());
            ui.painter().text(
                pos2(rect.left() + 40.0 + depth * INDENT, rect.center().y),
                Align2::LEFT_CENTER,
                format!("… {} more", values.len() - SHOWN),
                theme::regular(12.0),
                theme::TEXT_DIM,
            );
        }
    }

    fn menu(
        ui: &mut Ui,
        path: &TagPath,
        label: &str,
        tag: &Tag,
        in_compound: bool,
        actions: &mut Vec<Action>,
    ) {
        let root = path.is_empty();
        fn item(ui: &mut Ui, text: &str, a: Action, actions: &mut Vec<Action>) {
            if ui.button(text).clicked() {
                actions.push(a);
                ui.close_menu();
            }
        }
        if matches!(tag, Tag::Compound(_) | Tag::List(_)) {
            ui.menu_button("Add Tag", |ui| {
                for (t, name) in ADDABLE {
                    if let Tag::List(l) = tag {
                        if l.element_type() != TagType::End && l.element_type() != t {
                            continue;
                        }
                    }
                    if ui.button(name).clicked() {
                        actions.push(Action::Add(path.clone(), t));
                        ui.close_menu();
                    }
                }
            });
            item(ui, "Paste", Action::Paste(path.clone()), actions);
        }
        item(ui, "Copy", Action::Copy(path.clone()), actions);
        if !root {
            if in_compound {
                item(
                    ui,
                    "Rename",
                    Action::StartRename(path.clone(), label.to_string()),
                    actions,
                );
            }
            item(ui, "Move Up", Action::Move(path.clone(), -1), actions);
            item(ui, "Move Down", Action::Move(path.clone(), 1), actions);
            ui.separator();
            item(ui, "Delete", Action::Delete(path.clone()), actions);
        }
    }

    fn apply(&mut self, action: Action) {
        let Some(before) = self.frame_start.clone() else {
            return;
        };
        let parent_of = |p: &TagPath| p[..p.len().saturating_sub(1)].to_vec();
        match action {
            Action::Snapshot => {
                self.push_undo(before);
                return;
            }
            Action::Changed => {
                self.generation += 1;
                return;
            }
            Action::Select(p) => {
                self.selected = Some(p);
                return;
            }
            Action::Toggle(k) => {
                if !self.expanded.remove(&k) {
                    self.expanded.insert(k);
                }
                return;
            }
            Action::StartRename(p, n) => {
                self.rename = Some((p, n));
                return;
            }
            Action::Copy(p) => {
                if let Some(t) = get_mut(&mut self.root.tag, &p) {
                    let name = match p.last() {
                        Some(Seg::Key(k)) => k.clone(),
                        _ => "copy".into(),
                    };
                    self.clipboard = Some((name, t.clone()));
                    self.status = "Copied".into();
                }
                return;
            }
            Action::Delete(p) => {
                match (get_mut(&mut self.root.tag, &parent_of(&p)), p.last()) {
                    (Some(Tag::Compound(c)), Some(Seg::Key(k))) => {
                        c.shift_remove(k);
                    }
                    (Some(Tag::List(l)), Some(Seg::Index(i))) => {
                        l.remove(*i);
                    }
                    _ => return,
                }
                self.selected = None;
            }
            Action::Move(p, delta) => match (get_mut(&mut self.root.tag, &parent_of(&p)), p.last())
            {
                (Some(Tag::Compound(c)), Some(Seg::Key(k))) => {
                    let Some(i) = c.get_index_of(k) else { return };
                    let j = i as isize + delta;
                    if j < 0 || j as usize >= c.len() {
                        return;
                    }
                    c.swap_indices(i, j as usize);
                }
                (Some(Tag::List(l)), Some(Seg::Index(i))) => {
                    let j = *i as isize + delta;
                    if j < 0 || j as usize >= l.len() {
                        return;
                    }
                    l.swap(*i, j as usize);
                    let mut np = parent_of(&p);
                    np.push(Seg::Index(j as usize));
                    self.selected = Some(np);
                }
                _ => return,
            },
            Action::Add(p, t) => {
                let Some(new) = default_tag(t) else { return };
                match get_mut(&mut self.root.tag, &p) {
                    Some(Tag::Compound(c)) => {
                        let base = format!("New {}", type_name(t));
                        let mut key = base.clone();
                        let mut n = 1;
                        while c.contains_key(&key) {
                            n += 1;
                            key = format!("{base} {n}");
                        }
                        c.insert(key, new);
                    }
                    Some(Tag::List(l)) => {
                        if l.push(new).is_err() {
                            self.status = "That type does not fit this list".into();
                            return;
                        }
                    }
                    _ => return,
                }
                self.expanded.insert(path_key(&p));
            }
            Action::Paste(p) => {
                let Some((name, tag)) = self.clipboard.clone() else {
                    return;
                };
                match get_mut(&mut self.root.tag, &p) {
                    Some(Tag::Compound(c)) => {
                        let mut key = name.clone();
                        let mut n = 1;
                        while c.contains_key(&key) {
                            n += 1;
                            key = format!("{name} {n}");
                        }
                        c.insert(key, tag);
                    }
                    Some(Tag::List(l)) => {
                        if l.push(tag).is_err() {
                            self.status = "The copied tag does not fit this list".into();
                            return;
                        }
                    }
                    _ => return,
                }
            }
        }
        self.push_undo(before);
    }

    fn finish_rename(&mut self, path: TagPath, new_name: String) {
        let before = self.root.clone();
        let parent = get_mut(&mut self.root.tag, &path[..path.len() - 1]);
        if let (Some(Tag::Compound(c)), Some(Seg::Key(old))) = (parent, path.last()) {
            if new_name.is_empty() || (c.contains_key(&new_name) && &new_name != old) {
                self.status = "That name is empty or already used".into();
                return;
            }
            let rebuilt: Compound = std::mem::take(c)
                .into_iter()
                .map(|(k, v)| {
                    if &k == old {
                        (new_name.clone(), v)
                    } else {
                        (k, v)
                    }
                })
                .collect();
            *c = rebuilt;
            self.push_undo(before);
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, full_precision: bool, extensions_dir: &std::path::Path) {
        self.toolbar(ui);
        if let Some((path, mut name)) = self.rename.take() {
            let (mut keep, mut done) = (true, false);
            egui::Window::new("Rename Tag")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    let r = ui.text_edit_singleline(&mut name);
                    r.request_focus();
                    ui.horizontal(|ui| {
                        done = ui.button("Rename").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Enter));
                        keep = !ui.button("Cancel").clicked();
                    });
                });
            if done {
                self.finish_rename(path, name);
            } else if keep {
                self.rename = Some((path, name));
            }
        }
        if !self.read_only
            && ui.input(|i| i.key_pressed(egui::Key::Delete))
            && self.rename.is_none()
            && !ui.ctx().wants_keyboard_input()
        {
            if let Some(p) = self.selected.clone().filter(|p| !p.is_empty()) {
                self.frame_start = Some(self.root.clone());
                self.apply(Action::Delete(p));
            }
        }
        self.frame_start = Some(self.root.clone());
        let mut actions = Vec::new();
        let mut hits = 0;
        let find = self.find.clone();
        let selected = self.selected.clone();
        let expanded = self.expanded.clone();
        let read_only = self.read_only;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let cx = RowCtx {
                    find: &find,
                    selected: &selected,
                    expanded: &expanded,
                    full_precision,
                    read_only,
                    width: ui.available_width(),
                    styles: &self.styles,
                    extensions_dir,
                };
                let label = if self.root.name.is_empty() {
                    String::new()
                } else {
                    self.root.name.clone()
                };
                Self::row(
                    ui,
                    &cx,
                    &label,
                    &mut self.root.tag,
                    Vec::new(),
                    false,
                    &mut actions,
                    &mut hits,
                );
            });
        if !find.is_empty() {
            self.status = format!("{hits} matches");
        }
        for a in actions {
            self.apply(a);
        }
        if !self.status.is_empty() {
            let r = ui.max_rect();
            ui.painter().text(
                r.right_bottom() - vec2(12.0, 10.0),
                Align2::RIGHT_BOTTOM,
                &self.status,
                theme::regular(12.0),
                theme::TEXT_MUTED,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_round_trip() {
        let mut c = Compound::new();
        c.insert("LevelName".into(), Tag::String("x".into()));
        let root = NamedTag {
            name: String::new(),
            tag: Tag::Compound(c),
        };
        for f in [
            Format::Java(Compression::Gzip),
            Format::Java(Compression::None),
            Format::Bedrock(Some(10)),
            Format::Bedrock(None),
        ] {
            let (_, back) = load(&encode(f, &root).unwrap()).unwrap();
            assert_eq!(back, root, "{f:?}");
        }
    }

    #[test]
    fn edits_and_undo() {
        let mut c = Compound::new();
        c.insert("a".into(), Tag::Int(1));
        c.insert(
            "b".into(),
            Tag::List(List::from_tags(TagType::Int, vec![Tag::Int(5), Tag::Int(6)]).unwrap()),
        );
        let mut t = NbtTree::new(NamedTag {
            name: String::new(),
            tag: Tag::Compound(c),
        });
        t.frame_start = Some(t.root.clone());
        t.apply(Action::Move(vec![Seg::Key("b".into())], -1));
        assert_eq!(
            t.root.tag.as_compound().unwrap().keys().next().unwrap(),
            "b"
        );
        t.frame_start = Some(t.root.clone());
        t.apply(Action::Delete(vec![Seg::Key("b".into()), Seg::Index(0)]));
        assert_eq!(t.root.tag.get("b").unwrap().as_list().unwrap().len(), 1);
        t.finish_rename(vec![Seg::Key("a".into())], "renamed".into());
        assert!(t.root.tag.get("renamed").is_some());
        assert_eq!(t.undo.len(), 3);
        assert!(t.dirty);
    }

    #[test]
    fn hints_read_like_umt() {
        assert_eq!(
            hint("GameType", &Tag::Int(1)),
            Some(("Creative".into(), true))
        );
        assert_eq!(
            hint("Time", &Tag::Long(823)),
            Some(("41 seconds".into(), false))
        );
        // The value UMT shows as "October 29, 2026, 03:36AM" for the sample world.
        assert_eq!(date(1_793_245_010_889), "October 29, 2026, 03:36AM");
    }
}
