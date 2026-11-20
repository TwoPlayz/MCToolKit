//! The NBT tree editor: toolbar (undo/redo, add tags by type, find), rows with type
//! badges, inline value editing, readable hints (game mode, dates, durations), and a
//! context menu for rename, reorder, copy/paste and delete.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use mctk_nbt::{Compound, Compression, Flavor, List, NamedTag, Tag, TagType};

use crate::array_view;
use crate::ctx_menu;
use crate::icons::{self, Icon};
use crate::theme;
use crate::widgets;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// An item's wear and where it is kept: `Damage` (a short in old worlds), `tag.Damage` before
/// 1.20.5, the `minecraft:damage` component after.
/// A Bedrock item stack names itself with `Name`, not `id`.
fn is_bedrock_item(c: &Compound) -> bool {
    matches!(c.get("Name"), Some(Tag::String(_))) && !c.contains_key("id")
}

fn damage_of(c: &Compound) -> Option<(i64, TagPath)> {
    let number = |t: Option<&Tag>| match t {
        Some(Tag::Short(v)) => Some(i64::from(*v)),
        Some(Tag::Int(v)) => Some(i64::from(*v)),
        _ => None,
    };
    if let Some(v) = number(c.get("Damage")) {
        return Some((v, vec![Seg::Key("Damage".into())]));
    }
    if let Some(v) = number(c.get("tag").and_then(|t| t.get("Damage"))) {
        return Some((v, vec![Seg::Key("tag".into()), Seg::Key("Damage".into())]));
    }
    number(c.get("components").and_then(|t| t.get("minecraft:damage")))
        .map(|v| (v, vec![Seg::Key("components".into()), Seg::Key("minecraft:damage".into())]))
}

/// The key that holds the id of an item stack, if this compound is one: `id` beside a count or
/// a slot (Java; a name, or a number in console and old worlds), `Name` beside a count (Bedrock).
fn item_id_key(tag: &Tag) -> Option<&'static str> {
    let c = tag.as_compound()?;
    let is_text = |k: &str| matches!(c.get(k), Some(Tag::String(_)));
    let counted = c.contains_key("Count") || c.contains_key("count");
    let has_id = matches!(c.get("id"), Some(Tag::String(_) | Tag::Short(_) | Tag::Int(_)));
    if has_id && (counted || c.contains_key("Slot")) {
        Some("id")
    } else if is_text("Name") && counted && c.contains_key("Damage") {
        Some("Name")
    } else {
        None
    }
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

/// Colors extensions name: Qt/SVG color names or #rrggbb.
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
    // The tag badges, falling back to drawn ones.
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
        TagType::List => icons::paint(p, r, Icon::List, theme::text()),
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
    // UTC.
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
    Select(TagPath),
    /// Ctrl+click: add the tag to the selection, or take it out.
    SelectMore(TagPath),
    /// Shift+click: everything from the selected tag to this one.
    SelectRange(TagPath),
    Toggle(String),
    Delete(TagPath),
    Move(TagPath, isize),
    StartRename(TagPath, String),
    Add(TagPath, TagType),
    /// Make the item stack at this path another item (its id changes, nothing else).
    SetItem(TagPath, String),
    /// Open the inventory screen on the list at this path.
    OpenInventory(TagPath),
    /// Open the map file (`map_2`) of a map item in the map editor.
    OpenMap(String),
    /// Give an item, mob or container this custom name (empty: none).
    SetName(TagPath, String),
    /// Make an item never wear out (true), or wear out again.
    SetUnbreakable(TagPath, bool),
    /// Change a mob, villager or gateway (see mob_edit).
    EditMob(TagPath, crate::mob_edit::MobEdit),
    /// Open the trades screen on the villager at this path.
    OpenTrades(TagPath),
    /// Change an item stack's attributes or adventure-mode blocks (see item_edit).
    EditItem(TagPath, crate::item_edit::ItemEdit),
    /// Put a new item stack at the end of the list at this path.
    AddStack(TagPath, Compound),
    /// Set tags that exist to new values, as one undo step called by the text.
    SetValues(Vec<(TagPath, Tag)>, String),
    /// Give an enchantment of the item stack a level, or take it off with `None`.
    SetEnchant(TagPath, crate::enchant::EnchantEdit),
    Copy(TagPath),
    Cut(TagPath),
    Paste(TagPath),
    /// Bring a deleted tag back, or return an edited one to what it was.
    Restore(TagPath),
    /// Take back what was undone for this tag.
    RedoAt(TagPath),
    /// A value was edited in place in this tag.
    ChangedAt(TagPath),
    /// Delete the file or record this document was opened from.
    DeleteDocument,
    /// Open this array tag in the hex editor.
    OpenHex(TagPath),
    /// Open this array tag as a picture (the map editor, light layers, ...).
    OpenPicture(TagPath, array_view::Picture),
}

/// What has happened to a tag since the document was opened or last saved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Added: a green bar.
    Fresh,
    /// Existed already and was changed (its value or its name): an orange bar.
    Edited,
    /// Deleted but kept until the save: a red bar, faint and inactive.
    Gone,
}

/// One marked tag. `base` is the name and value Undo brings it back to: what it was before the
/// edit, or what it was when it was added.
#[derive(Clone, PartialEq)]
struct Mark {
    kind: Kind,
    path: TagPath,
    base: Option<(String, Tag)>,
}

/// Something undone that the tag it concerns can take back again: Redo in that tag's menu.
#[derive(Clone)]
enum Undone {
    /// The tag had this value (and name) before it was undone.
    Value { path: TagPath, name: String, tag: Tag },
    /// The tag had been deleted.
    Delete(TagPath),
}

impl Undone {
    fn path(&self) -> &TagPath {
        match self {
            Undone::Value { path, .. } | Undone::Delete(path) => path,
        }
    }
}

/// The paths of tags whose values differ between two trees (compounds and lists compared place by
/// place), as far as `out` has room.
fn diff_values(a: &Tag, b: &Tag, path: &mut TagPath, out: &mut Vec<TagPath>) {
    if out.len() >= 24 {
        return;
    }
    match (a, b) {
        (Tag::Compound(x), Tag::Compound(y)) => {
            for (k, va) in x {
                if let Some(vb) = y.get(k) {
                    path.push(Seg::Key(k.clone()));
                    diff_values(va, vb, path, out);
                    path.pop();
                }
            }
        }
        (Tag::List(x), Tag::List(y)) if x.len() == y.len() => {
            for i in 0..x.len() {
                if let (Some(va), Some(vb)) = (x.get(i), y.get(i)) {
                    path.push(Seg::Index(i));
                    diff_values(va, vb, path, out);
                    path.pop();
                }
            }
        }
        _ => {
            if a != b {
                out.push(path.clone());
            }
        }
    }
}

/// The marks of a document. Kept as paths; the edits that move tags update them.
#[derive(Clone, Default, PartialEq)]
struct Marks {
    list: Vec<Mark>,
}

/// A menu row whose icon is drawn white, like the other icons of the menu.
pub fn white_item(ui: &mut Ui, icon: Icon, text: &str) -> bool {
    // White asks for the highlighted artwork (UNDO_h, REDO_h), which is white like the other icons.
    ctx_menu::painted(ui, text, "", true, |p, r, _| icons::paint(p, r, icon, Color32::WHITE))
}

fn get_ref<'a>(tag: &'a Tag, path: &[Seg]) -> Option<&'a Tag> {
    let Some((first, rest)) = path.split_first() else {
        return Some(tag);
    };
    let child = match (tag, first) {
        (Tag::Compound(c), Seg::Key(k)) => c.get(k)?,
        (Tag::List(l), Seg::Index(i)) => l.get(*i)?,
        _ => return None,
    };
    get_ref(child, rest)
}

/// A tag's name: its key, or its index in a list.
/// The text with its first letter lower case, to follow "Undid: ".
fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The tag named `name` in the same compound as the tag at `p`.
fn key_beside(p: &TagPath, name: &str) -> TagPath {
    let mut c = p[..p.len().saturating_sub(1)].to_vec();
    c.push(Seg::Key(name.to_string()));
    c
}

/// A dimension number as the three the map editor offers: 0 Overworld, -1 Nether, 1 End.
fn dim_index(v: i32) -> usize {
    match v {
        -1 => 1,
        1 => 2,
        _ => 0,
    }
}

/// An item stack: a compound with an `id` (a name, or a number in console and old worlds) or Bedrock\n/// `Name`, and a `count` or `Count`. Gives the id as a name and the count.
fn item_stack(c: &Compound) -> Option<(String, Option<i64>)> {
    let count = c.get("count").or_else(|| c.get("Count"))?.as_i64();
    let damage = match c.get("Damage") {
        Some(Tag::Short(d)) => *d as u16,
        _ => 0,
    };
    let id = match (c.get("id"), c.get("Name")) {
        (Some(Tag::String(s)), _) | (None, Some(Tag::String(s))) => s.clone(),
        // Console and old worlds give the item as a number (a short), with its damage value.
        (Some(Tag::Short(n)), _) => mctk_mapping::legacy::legacy().item(*n as u16, damage)?.to_string(),
        (Some(Tag::Int(n)), _) => mctk_mapping::legacy::legacy().item(*n as u16, damage)?.to_string(),
        _ => return None,
    };
    Some((id, count))
}
/// What a compound that is no item stack stands for, in any edition: a block entity (an `id` and
/// its block's `x`, `y`, `z`), an entity (an `id` with its `Pos` or `UUID`, or Bedrock's
/// `identifier`) or a block state (Java's `Name` with its `Properties`, Bedrock's `name` with its
/// `states`). Gives the item to picture it by and its name.
fn thing_of(c: &Compound) -> Option<(String, String)> {
    let bare = |id: &str| id.trim_start_matches("minecraft:").to_string();
    let is_int = |k: &str| matches!(c.get(k), Some(Tag::Int(_)));
    // A block state.
    if let Some(Tag::String(n)) = c.get("Name") {
        if c.len() == 1 || (c.len() == 2 && c.contains_key("Properties")) {
            return Some((n.clone(), item_name(n)));
        }
    }
    if let (Some(Tag::String(_)), Some(Tag::Compound(_))) = (c.get("name"), c.get("states")) {
        let entry = NamedTag { name: String::new(), tag: Tag::Compound(c.clone()) };
        let java = crate::world::bedrock_java_state(&entry);
        return Some((java.name.clone(), item_name(&java.name)));
    }
    // An entity.
    let entity = c.contains_key("identifier") || (matches!(c.get("id"), Some(Tag::String(_))) && (c.contains_key("Pos") || c.contains_key("UUID")));
    if entity {
        let id = mctk_convert::view::java_entity_id(c)?;
        let n = bare(&id);
        // A dropped item: the item it is.
        if n == "item" {
            if let Some((item, _)) = c.get("Item").and_then(Tag::as_compound).and_then(item_stack) {
                return Some((item.clone(), format!("{} (dropped)", item_name(&item))));
            }
        }
        let icon = match n.as_str() {
            "falling_block" => c
                .get("BlockState")
                .or_else(|| c.get("FallingBlock"))
                .and_then(|b| b.get("Name").or_else(|| b.get("name")))
                .and_then(Tag::as_str)
                .map_or_else(|| "sand".to_string(), str::to_string),
            "experience_orb" => "experience_bottle".into(),
            "player" => "player_head".into(),
            // A mob: its spawn egg (a rabbit is not its meat); a thing with an item of its own
            // (a boat, a minecart, an armor stand): that.
            other if crate::models::item_model(&format!("{other}_spawn_egg")).is_some() => format!("{other}_spawn_egg"),
            other if crate::models::item_model(other).is_some() => other.to_string(),
            other => format!("{other}_spawn_egg"),
        };
        return Some((icon, item_name(&id)));
    }
    // A block entity.
    if matches!(c.get("id"), Some(Tag::String(_))) && is_int("x") && is_int("y") && is_int("z") {
        let id = mctk_convert::view::java_block_entity_id(c)?;
        let icon = match bare(&id).as_str() {
            "mob_spawner" => "spawner".to_string(),
            "sign" => "oak_sign".into(),
            "hanging_sign" => "oak_hanging_sign".into(),
            "banner" => "white_banner".into(),
            "bed" => "red_bed".into(),
            "skull" => "skeleton_skull".into(),
            "end_portal" | "end_gateway" => "end_portal_frame".into(),
            "piston" | "moving_piston" => "piston".into(),
            "brushable_block" => "suspicious_sand".into(),
            other => other.to_string(),
        };
        return Some((icon, item_name(&id)));
    }
    None
}

/// What a compound's menu can change if it is a mob (or an armor stand, an item frame) or an end
/// gateway: `None` for anything else.
fn mob_menu(c: &Compound, block_entity: bool, data_version: Option<i64>) -> Option<MobMenu> {
    use crate::mob_edit as m;
    if block_entity {
        let id = mctk_convert::view::java_block_entity_id(c)?;
        if !id.ends_with("end_gateway") {
            return None;
        }
        return Some(MobMenu { id, gateway: Some((m::gateway_exit(c).unwrap_or([0, 64, 0]), m::gateway_exact(c))), ..Default::default() });
    }
    // A player's file (Java and the console editions keep no id in it): its status effects.
    let player_file = !crate::naming::is_mob(c) && c.contains_key("abilities") && m::health(c).is_some();
    if player_file {
        return Some(MobMenu {
            id: "minecraft:player".into(),
            icon: "player_head".into(),
            effects: Some((m::effects(c), crate::effects::choices(c, crate::effects::Holder::Mob, data_version))),
            data_version,
            ..Default::default()
        });
    }
    if !crate::naming::is_mob(c) {
        return None;
    }
    let id = mctk_convert::view::java_entity_id(c)?;
    let health = m::health(c);
    // What it wears: living things (they have health) and armor stands.
    let wears = health.is_some() || id.ends_with("armor_stand");
    let icon = thing_of(c).map(|(i, _)| i).unwrap_or_else(|| format!("{}_spawn_egg", id.trim_start_matches("minecraft:")));
    Some(MobMenu {
        id: id.clone(),
        icon,
        invulnerable: Some(m::invulnerable(c)),
        no_ai: m::no_ai(c).filter(|_| health.is_some()),
        invisible: m::invisible(c, &id),
        gravity: m::gravity(c),
        equipment: if wears {
            m::equipment(c, data_version)
                .into_iter()
                .map(|(s, item)| {
                    let stack = item.as_ref().and_then(|_| m::slot_stack(c, s, data_version).cloned());
                    (s, item, stack)
                })
                .collect()
        } else {
            Vec::new()
        },
        data_version,
        trades: m::trades_with(&id),
        drops: m::drops(c, data_version).filter(|_| health.is_some() && !id.ends_with(":player")),
        effects: health.map(|_| (m::effects(c), crate::effects::choices(c, crate::effects::Holder::Mob, data_version))),
        shulker_color: m::shulker_color(c, &id),
        gateway: None,
    })
}

/// An item id as a name to read: `minecraft:diamond_sword` is "Diamond Sword".
fn item_name(id: &str) -> String {
    id.rsplit(':')
        .next()
        .unwrap_or(id)
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn seg_name(p: &TagPath) -> String {
    match p.last() {
        Some(Seg::Key(k)) => k.clone(),
        Some(Seg::Index(i)) => i.to_string(),
        None => String::new(),
    }
}

/// An array tag open in the hex editor.
struct HexState {
    path: TagPath,
    ed: crate::hex_editor::HexEditor,
    /// The tree generation the bytes were read at; a change by anything else reloads them.
    seen: u64,
    /// When the bytes last changed (egui time), so a burst of typing is one undo step.
    last_change: f64,
    /// Showing a picture of the array (the map editor, ...) instead of the hex dump.
    picture: Option<array_view::Picture>,
}

/// What undo and redo bring back: the tree and its marks.
#[derive(Clone)]
struct Snap {
    root: NamedTag,
    marks: Marks,
    /// What the edit made after this state was, in a few words, for the Undo and Redo messages.
    note: String,
}

/// One open NBT document.
pub struct NbtTree {
    pub root: NamedTag,
    pub dirty: bool,
    /// How many edits were behind the document when it was opened or last saved; being back at
    /// that point means nothing is changed. `None` once the history no longer reaches it.
    clean: Option<usize>,
    pub read_only: bool,
    /// Set by the page: the root's menu offers deleting the file or record behind this tree.
    pub deletable: bool,
    /// Set by that menu entry; the page takes it and asks to confirm.
    pub delete_requested: bool,
    marks: Marks,
    /// What undo took back, per tag, so each tag's menu can offer Redo.
    undone: Vec<Undone>,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    frame_start: Option<Snap>,
    expanded: HashSet<String>,
    selected: Option<TagPath>,
    /// Every selected tag (the one in `selected` is among them); cut, copy and delete act on all.
    marked: Vec<TagPath>,
    /// The visible rows, top to bottom, as of the last frame.
    row_order: Vec<TagPath>,
    /// A drag-select in progress.
    marquee: Option<Marquee>,
    /// Scroll speed while a drag-select is held past the top or bottom edge.
    autoscroll: f32,
    /// A value box had keyboard focus as of the last frame.
    typing: bool,
    find_focused: bool,
    find: String,
    finding: bool,
    rename: Option<(TagPath, String)>,
    clipboard: Vec<(String, Tag)>,
    /// A message for the editor to show as a toast; it takes it.
    pub status: String,
    /// The live count of search matches, drawn in the corner while a search is typed.
    find_status: String,
    /// Labels and icons from style extensions, by tag path.
    pub styles: HashMap<TagPath, crate::info::TagStyle>,
    /// Bumped on every edit, so styles can be refreshed.
    pub generation: u64,
    /// The array tag open in the hex editor, which then takes the place of the tags.
    hex: Option<HexState>,
    /// The inventory screen open on a list of item stacks, and what it remembers.
    inventory: Option<(TagPath, crate::inventory::State)>,
    /// The second chest of a large chest, when the screen shows both (its `Items` list).
    inventory_partner: Option<TagPath>,
    /// The villager (its path) whose trades screen is open.
    trades: Option<TagPath>,
    /// The large chests' halves (see RowCtx::chests), and the tree generation they were found for.
    chests: (Option<u64>, HashMap<TagPath, (bool, usize)>),
    /// The document read as a Java chunk (for its block states), and the generation it was read at.
    decoded: RefCell<Option<(u64, Option<Arc<mctk_java::Chunk>>)>>,
    /// Set by "Open Map Editor" on a map item: the name of its `map_N` file; the page opens it.
    pub open_map_request: Option<String>,
    /// Describes the edit about to be pushed on the undo stack.
    edit_note: String,
    /// The tag whose value edit has already been announced, so typing in it says so once.
    announced: Option<TagPath>,
}

fn path_key(p: &TagPath) -> String {
    format!("{p:?}")
}

/// A drag-select: where it began (relative to the top of the rows, so it follows scrolling),
/// the selection it adds to, and whether the pointer has moved far enough to count as a drag.
struct Marquee {
    start: Vec2,
    base: Vec<TagPath>,
    active: bool,
}

/// Where a row was drawn this frame, for range and drag selection.
struct RowInfo {
    path: TagPath,
    rect: Rect,
    /// The value box, where a drag edits the value instead of selecting.
    field: Option<Rect>,
}

struct RowCtx<'a> {
    find: &'a str,
    selected: &'a Option<TagPath>,
    marked: &'a HashSet<String>,
    rows: &'a RefCell<Vec<RowInfo>>,
    /// Set when a value box has keyboard focus.
    typing: &'a Cell<bool>,
    expanded: &'a HashSet<String>,
    full_precision: bool,
    read_only: bool,
    width: f32,
    styles: &'a HashMap<TagPath, crate::info::TagStyle>,
    /// Something was cut or copied, so Paste has a tag to insert.
    has_clipboard: bool,
    /// The document is a file or record the root's menu may delete.
    deletable: bool,
    /// What has happened to tags since the document was opened or saved.
    marks: &'a [Mark],
    undone: &'a [Undone],
    /// Counts the rows drawn, so every other one gets a lighter band.
    stripe: &'a Cell<usize>,
    /// How much bigger the rows are drawn than normal (the tree scale).
    k: f32,
    /// The tag whose children are being drawn is a container (a chest ...): its `Items` list can
    /// open as a screen.
    container: &'a Cell<bool>,
    /// The halves of large chests, by the path of each chest: which half it is and where the
    /// other is.
    chests: &'a HashMap<TagPath, (bool, usize)>,
    /// The document's DataVersion (Java), for laying new tags out as its version does.
    data_version: Option<i64>,
}

/// What a row's menu offers beyond the usual, worked out where the tag and its neighbors are known.
#[derive(Default)]
struct Extras {
    /// The list "Edit Inventory" opens.
    inventory: Option<TagPath>,
    /// The `map_N` file of a map item.
    map: Option<String>,
    /// Something that can be given a custom name (an item; or a mob, or a container such as a
    /// chest), and the name it has now.
    name: Option<(bool, String)>,
    /// An item that wears out: whether it is made never to.
    unbreakable: Option<bool>,
    /// A mob (or armor stand, item frame, end gateway): what its menu can change.
    mob: Option<MobMenu>,
    /// An item stack: its attribute modifiers (None where its edition keeps none) and the
    /// blocks it can break and be placed on in adventure mode.
    item: Option<(Option<Vec<crate::item_edit::Modifier>>, Vec<String>, Vec<String>)>,
    /// The document's DataVersion (Java), for the menus that write by version.
    data_version: Option<i64>,
    /// A block entity's menu: a spawner's mob, a container's lock.
    block: Option<BlockMenu>,
}

/// What a block entity's menu can change.
#[derive(Default)]
struct BlockMenu {
    /// A spawner: the mob it spawns now (a Java id), if it has one.
    spawner: Option<Option<String>>,
    /// A container that can be locked: the name of the key that opens it ("" when unlocked).
    lock: Option<String>,
}

/// What a mob's (or gateway's) menu offers, and how things are now; None where it does not apply.
#[derive(Default)]
struct MobMenu {
    /// Its current Java id, and the item to picture it by (its spawn egg).
    id: String,
    icon: String,
    invulnerable: Option<bool>,
    no_ai: Option<bool>,
    invisible: Option<bool>,
    gravity: Option<bool>,
    /// What it wears and holds, slot by slot, and the stack itself.
    equipment: Vec<(crate::mob_edit::Slot, Option<(String, i64)>, Option<Compound>)>,
    /// The document's DataVersion, for the stacks' menus.
    data_version: Option<i64>,
    /// A villager or wandering trader.
    trades: bool,
    /// What it drops and picks up (mobs, outside Bedrock).
    drops: Option<crate::mob_edit::Drops>,
    /// Living things and players: their status effects, and those they can be given.
    effects: Option<(Vec<crate::effects::Effect>, Vec<&'static str>)>,
    /// A shulker: its color (16: its own).
    shulker_color: Option<i64>,
    /// An end gateway: where it sends things, and whether exactly there.
    gateway: Option<([i64; 3], Option<bool>)>,
}

/// The tree's own scale, set from the tag bar; kept by the app for every document.
pub fn tree_scale(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(Id::new("tree-scale"))).unwrap_or(1.0).clamp(0.6, 2.5)
}

fn set_tree_scale(ctx: &egui::Context, v: f32) {
    ctx.data_mut(|d| d.insert_temp(Id::new("tree-scale"), (v * 100.0).round() / 100.0));
}



const ROW: f32 = 18.0;
/// The bar on a tag that existed already and was changed.
pub const EDITED: Color32 = Color32::from_rgb(0xE8, 0x93, 0x2E);
const INDENT: f32 = 18.0;

impl NbtTree {
    pub fn new(root: NamedTag) -> Self {
        let mut expanded = HashSet::new();
        expanded.insert(path_key(&Vec::new()));
        // Open the first level, for level.dat's Data compound.
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
            clean: Some(0),
            read_only: false,
            deletable: false,
            delete_requested: false,
            marks: Marks::default(),
            undone: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            frame_start: None,
            expanded,
            selected: None,
            marked: Vec::new(),
            row_order: Vec::new(),
            marquee: None,
            autoscroll: 0.0,
            typing: false,
            find_focused: false,
            find: String::new(),
            finding: false,
            rename: None,
            clipboard: Vec::new(),
            status: String::new(),
            find_status: String::new(),
            styles: HashMap::new(),
            generation: 0,
            hex: None,
            inventory: None,
            inventory_partner: None,
            chests: (None, HashMap::new()),
            trades: None,
            decoded: RefCell::new(None),
            open_map_request: None,
            edit_note: String::new(),
            announced: None,
        }
    }

    fn snap(&self) -> Snap {
        Snap {
            root: self.root.clone(),
            marks: self.marks.clone(),
            note: String::new(),
        }
    }

    fn restore(&mut self, s: Snap) -> Snap {
        let now = self.snap();
        self.root = s.root;
        self.marks = s.marks;
        now
    }

    /// Sets what the edit being made is called, and says so.
    fn announce(&mut self, note: String) {
        self.status = note.clone();
        self.edit_note = note;
    }

    fn push_undo(&mut self, before: Snap) {
        let mut before = before;
        before.note = std::mem::take(&mut self.edit_note);
        self.undo.push(before);
        if self.undo.len() > 100 {
            self.undo.remove(0);
            self.clean = None;
        }
        self.redo.clear();
        self.dirty = true;
        self.generation += 1;
    }

    /// Steps back one edit.
    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            let note = prev.note.clone();
            let mut now = self.restore(prev);
            now.note = note.clone();
            self.record_undone(&now);
            self.redo.push(now);
            self.after_history();
            self.status = if note.is_empty() {
                "Undid the last change".into()
            } else {
                format!("Undid: {}", lower_first(&note))
            };
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.read_only && !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.read_only && !self.redo.is_empty()
    }

    /// Remembers what an undo took back, tag by tag, for each tag's Redo.
    fn record_undone(&mut self, was: &Snap) {
        self.undone.clear();
        let mut changed = Vec::new();
        diff_values(&was.root.tag, &self.root.tag, &mut Vec::new(), &mut changed);
        for path in changed {
            if let Some(tag) = get_ref(&was.root.tag, &path) {
                self.undone.push(Undone::Value {
                    name: seg_name(&path),
                    tag: tag.clone(),
                    path,
                });
            }
        }
        for m in &was.marks.list {
            let still = self
                .marks
                .list
                .iter()
                .any(|x| x.kind == Kind::Gone && x.path == m.path);
            if m.kind == Kind::Gone && !still {
                self.undone.push(Undone::Delete(m.path.clone()));
            }
        }
    }

    /// Re-applies the edit that was undone.
    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            let note = next.note.clone();
            let mut now = self.restore(next);
            now.note = note.clone();
            self.undo.push(now);
            self.undone.clear();
            self.after_history();
            self.status = if note.is_empty() {
                "Redid the change".into()
            } else {
                format!("Redid: {}", lower_first(&note))
            };
        }
    }

    /// The document was written out: it counts as unchanged from this point of its history.
    pub fn mark_saved(&mut self) {
        self.dirty = false;
        self.clean = Some(self.undo.len());
    }

    /// A document that has never been written (a new file): changed until it is saved.
    pub fn mark_unsaved(&mut self) {
        self.dirty = true;
        self.clean = None;
    }

    fn after_history(&mut self) {
        self.marked.clear();
        self.selected = None;
        // Undoing back to the saved point leaves nothing changed.
        self.dirty = Some(self.undo.len()) != self.clean;
        self.generation += 1;
    }

    /// A value box or the find box has keyboard focus, so Ctrl+Z and the like belong to it.
    pub fn is_typing(&self) -> bool {
        self.typing()
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
        ui.painter().rect_filled(bar, 0.0, theme::bg());
        ui.painter().line_segment(
            [bar.left_bottom(), bar.right_bottom()],
            Stroke::new(1.0_f32, theme::border()),
        );
        let y = bar.center().y;
        // The tree's scale: a box like the tag values, dragged left and right or typed in.
        let fr = Rect::from_min_size(pos2(bar.left() + 8.0, y - 11.0), vec2(56.0, 22.0));
        let reset = Rect::from_min_size(fr.right_top(), vec2(24.0, 22.0));
        ui.painter().rect_filled(fr.union(reset), theme::radius(3), theme::field());
        let mut pct = (tree_scale(ui.ctx()) * 100.0).round() as i32;
        {
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(fr)
                    .layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)),
            );
            child.style_mut().visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            child.style_mut().visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
            child.style_mut().visuals.extreme_bg_color = Color32::TRANSPARENT;
            child.style_mut().spacing.button_padding = vec2(4.0, 0.0);
            child.style_mut().override_font_id = Some(theme::regular(13.0));
            let r = child
                .add_sized(fr.size(), egui::DragValue::new(&mut pct).range(60..=250).suffix("%").speed(1.0))
                .on_hover_text("Tree scale (drag, or Ctrl + wheel over the tree)");
            if r.has_focus() {
                self.typing = true;
            }
            if r.changed() {
                set_tree_scale(ui.ctx(), pct as f32 / 100.0);
            }
        }
        // Reset: attached to the box, lit only while the scale is not 100%.
        {
            let off = pct != 100;
            let resp = ui.interact(reset, Id::new("nbt-scale-reset"), if off { Sense::click() } else { Sense::hover() });
            if resp.hovered() && off {
                ui.painter().rect_filled(reset, theme::radius(3), theme::wash(28));
            }
            icons::paint(
                ui.painter(),
                Rect::from_center_size(reset.center(), Vec2::splat(14.0)),
                Icon::Undo,
                if !off {
                    theme::chip()
                } else if resp.hovered() {
                    theme::strong()
                } else {
                    theme::text_muted()
                },
            );
            if resp.on_hover_text("Reset the tree scale to 100%").clicked() && off {
                set_tree_scale(ui.ctx(), 1.0);
            }
        }
        // A divider between the scale and the tags.
        let dx = reset.right() + 8.0;
        ui.painter().line_segment(
            [pos2(dx, bar.top() + 8.0), pos2(dx, bar.bottom() - 8.0)],
            Stroke::new(1.0_f32, theme::border()),
        );
        let mut x = dx + 8.0;
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
                    .rect_filled(r, theme::radius(3), theme::button_hover());
            }
            let c = if enabled { theme::text() } else { theme::chip() };
            paint(ui.painter(), r.shrink(5.0), c);
            let clicked = resp.clicked();
            resp.on_hover_text(tip);
            clicked
        };
        // (Undo and Redo are buttons in the left panel, under Close and Save.)
        let editable = !self.read_only;
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
                    // Faded: the types that can't be added to the selection.
                    let mut faded = p.clone();
                    faded.multiply_opacity(0.3);
                    type_icon(&faded, r, t);
                }
            };
            if button(ui, &mut x, name, enabled, &paint, &format!("Add {name}")) {
                if let Some((path, _)) = target.clone() {
                    self.frame_start = Some(self.snap());
                    self.apply(Action::Add(path, t));
                }
            }
        }
        let sr = Rect::from_center_size(pos2(bar.right() - 20.0, y), Vec2::splat(24.0));
        let tags_end = x;
        let resp = ui.interact(sr, Id::new("nbt-find"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(sr, theme::radius(4), theme::wash(28));
        }
        icons::paint(
            ui.painter(),
            sr.shrink(4.0),
            Icon::Search,
            if resp.hovered() || self.finding {
                theme::strong()
            } else {
                theme::text_muted()
            },
        );
        if resp.clicked() {
            self.finding = !self.finding;
            if !self.finding {
                self.find.clear();
            }
        }
        if self.finding {
            // Beside the tags when there is room; else just under the bar, over the tree and never
            // over the tags.
            let room = sr.left() - 6.0 - (tags_end + 4.0);
            let edit = if room >= 140.0 {
                let left = (sr.left() - 6.0 - 200.0).max(tags_end + 4.0);
                let fr = Rect::from_min_max(pos2(left, y - 13.0), pos2(sr.left() - 6.0, y + 13.0));
                ui.put(
                    fr,
                    egui::TextEdit::singleline(&mut self.find)
                        .hint_text("Find tag or value")
                        .background_color(theme::deep()),
                )
            } else {
                let at = pos2(bar.right() - 8.0 - 220.0, bar.bottom() + 4.0);
                egui::Area::new(Id::new("nbt-find-float"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(at)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::new()
                            .fill(theme::deep())
                            .corner_radius(4)
                            .stroke(Stroke::new(1.0_f32, theme::border()))
                            .inner_margin(2.0)
                            .show(ui, |ui| {
                                ui.add_sized(
                                    vec2(216.0, 22.0),
                                    egui::TextEdit::singleline(&mut self.find)
                                        .hint_text("Find tag or value")
                                        .background_color(theme::deep()),
                                )
                            })
                            .inner
                    })
                    .inner
            };            self.find_focused = edit.has_focus();
        } else {
            self.find_focused = false;
        }
    }

    /// The selected container (or the root) that new tags are added to.
    /// Where the toolbar's add buttons insert, and the only type allowed there (
    /// updateActionStates): a selected compound takes any type, a list only its element
    /// type (any while it has none), and other tags nothing.
    /// Whether every item of the list at `p` is deleted (so it is as good as empty).
    fn list_all_gone(&self, p: &TagPath) -> bool {
        match get_ref(&self.root.tag, p) {
            Some(Tag::List(l)) => (0..l.len()).all(|i| {
                let mut c = p.clone();
                c.push(Seg::Index(i));
                self.under_gone(&c)
            }),
            _ => false,
        }
    }

    fn add_target(&mut self) -> Option<(TagPath, Option<TagType>)> {
        let path = self.selected.clone()?;
        // A list whose items are all deleted takes any type again.
        let emptied = self.list_all_gone(&path);
        match get_mut(&mut self.root.tag, &path)? {
            Tag::Compound(_) => Some((path, None)),
            Tag::List(l) => {
                let only = (l.element_type() != TagType::End && !emptied).then(|| l.element_type());
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
        list_len: usize,
        actions: &mut Vec<Action>,
        hits: &mut usize,
    ) {
        let depth = path.len() as f32;
        let k = cx.k;
        let rh = ROW * k;
        let (rect, resp) = ui.allocate_exact_size(vec2(cx.width, rh), Sense::click());
        let key = path_key(&path);
        let in_marked = cx.marked.contains(&key);
        let selected = in_marked || cx.selected.as_ref() == Some(&path);
        let several = in_marked && cx.marked.len() > 1;
        cx.rows.borrow_mut().push(RowInfo {
            path: path.clone(),
            rect,
            field: None,
        });
        let found = !cx.find.is_empty() && {
            let f = cx.find.to_lowercase();
            label.to_lowercase().contains(&f)
                || matches!(tag, Tag::String(s) if s.to_lowercase().contains(&f))
        };
        *hits += found as usize;
        let p = ui.painter().clone();
        let band = cx.stripe.get();
        cx.stripe.set(band + 1);
        let gone_root = cx
            .marks
            .iter()
            .find(|m| m.kind == Kind::Gone && path.starts_with(m.path.as_slice()))
            .map(|m| m.path.clone());
        let is_gone = gone_root.is_some();
        let redoable = cx.undone.iter().any(|u| *u.path() == path);
        let own = cx.marks.iter().find(|m| m.path == path);
        let is_fresh = own.is_some_and(|m| m.kind == Kind::Fresh);
        let is_edited = own.is_some_and(|m| m.kind == Kind::Edited);
        // Undo is offered once there is something to undo: an edit, or a change to what was added.
        let undoable = own.is_some_and(|m| match (&m.kind, &m.base) {
            (Kind::Edited, _) => true,
            (Kind::Fresh, Some((name, base))) => {
                base != &*tag
                    || (matches!(path.last(), Some(Seg::Key(_))) && name.as_str() != label)
            }
            _ => false,
        });
        if band % 2 == 1 {
            p.rect_filled(rect, 0.0, Color32::from_rgba_premultiplied(7, 7, 7, 7));
        }
        if selected {
            p.rect_filled(rect, 0.0, theme::accent_wash(0.26));
        } else if found {
            p.rect_filled(rect, 0.0, Color32::from_rgb(0x4A, 0x40, 0x1C));
        } else if resp.hovered() {
            // A faint lift in the page accent; the selected row is stronger.
            p.rect_filled(rect, 0.0, theme::accent_wash(0.14));
        }
        // A bar at the left edge: red for a deleted tag, green for a new one, orange for one that
        // was changed.
        if is_fresh || is_edited || is_gone {
            p.rect_filled(
                Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
                0.0,
                if is_gone {
                    theme::RED
                } else if is_fresh {
                    theme::EDITOR_ACCENT
                } else {
                    EDITED
                },
            );
        }
        let mut x = rect.left() + 6.0 * k + depth * INDENT * k;
        // Arrays open in the hex editor instead of unfolding.
        let container = matches!(tag, Tag::Compound(_) | Tag::List(_));
        let open = cx.expanded.contains(&key);
        // The arrow is only there when there is something to open.
        let has_children = match &*tag {
            Tag::Compound(c) => !c.is_empty(),
            Tag::List(l) => !l.is_empty(),
            _ => false,
        };
        if container && has_children {
            let cr = Rect::from_center_size(pos2(x + 5.0 * k, rect.center().y), Vec2::splat(9.0 * k));
            icons::paint(
                &p,
                cr,
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                theme::text_muted(),
            );
            if resp.clicked()
                && ui
                    .input(|i| i.pointer.interact_pos())
                    .is_some_and(|pos| pos.x < x + 14.0 * k)
            {
                actions.push(Action::Toggle(key.clone()));
            }
        }
        x += 14.0 * k;
        type_icon(
            &p,
            Rect::from_min_size(pos2(x, rect.top() + 2.0 * k), Vec2::splat(14.0 * k)),
            tag.tag_type(),
        );
        x += 20.0 * k;
        // A tag in a list has no name of its own, only its place; one in a compound is named.
        if in_compound {
            let name_galley = p.layout_no_wrap(label.to_string(), theme::regular(13.0 * k), theme::text());
            let name_w = name_galley.size().x;
            p.galley(
                pos2(x, rect.center().y - name_galley.size().y / 2.0),
                name_galley,
                theme::text(),
            );
            x += name_w + 6.0 * k;
        }
        if resp.clicked() {
            let (command, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            actions.push(if command {
                Action::SelectMore(path.clone())
            } else if shift {
                Action::SelectRange(path.clone())
            } else {
                Action::Select(path.clone())
            });
        } else if resp.secondary_clicked() && !in_marked {
            // Right-clicking inside the selection keeps it, so the menu acts on all of it.
            actions.push(Action::Select(path.clone()));
        }
        if resp.double_clicked() {
            if in_compound && !is_gone && !cx.read_only {
                // Double-click renames; a folder opens and closes with its arrow.
                actions.push(Action::StartRename(path.clone(), label.to_string()));
            } else if container {
                actions.push(Action::Toggle(key.clone()));
            }
        }
        let count = |n: usize, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
        let value_rect =
            |w: f32| Rect::from_min_size(pos2(x, rect.top() + 1.0 * k), vec2(w, rh - 2.0 * k));
        let counted = match &*tag {
            Tag::Compound(c) => Some(count(c.len(), "tag")),
            Tag::List(l) => Some(count(l.len(), "tag")),
            Tag::ByteArray(a) => Some(count(a.len(), "byte")),
            Tag::IntArray(a) => Some(count(a.len(), "int")),
            Tag::LongArray(a) => Some(count(a.len(), "long")),
            _ => None,
        };
        // An item stack is shown by what it holds: its picture, its name and how many.
        let item = match &*tag {
            Tag::Compound(c) => item_stack(c),
            _ => None,
        };
        // Otherwise a block entity, entity or block state: its picture and name.
        let thing = match &*tag {
            Tag::Compound(c) if item.is_none() => thing_of(c),
            _ => None,
        };
        // Where an extension's icon and label (or our own hint) go.
        let mut after;
        let mut gloss = None;
        match &mut *tag {
            _ if counted.is_some() => {
                let g = p.layout_no_wrap(
                    counted.unwrap_or_default(),
                    theme::regular(12.0 * k),
                    theme::text_dim(),
                );
                let hint_w = g.size().x;
                p.galley(
                    pos2(x, rect.center().y - g.size().y / 2.0),
                    g,
                    theme::text_dim(),
                );
                after = x + hint_w + 8.0 * k;
                // An item stack: its picture, name and how many, after the count of tags.
                if let Some((id, n)) = &item {
                    let ir = Rect::from_min_size(pos2(after, rect.center().y - 8.0 * k), Vec2::splat(16.0 * k));
                    // Enchanted: tinted purple.
                    let enchanted = matches!(&*tag, Tag::Compound(c) if !crate::enchant::read(c).is_empty());
                    let tint = if enchanted { crate::enchant::PURPLE } else { Color32::WHITE };
                    // Bedrock keeps a stack with no name and no count in every empty slot: a gray
                    // grass block, and "Empty".
                    let empty = matches!(&*tag, Tag::Compound(c) if is_bedrock_item(c)) && (id.is_empty() || *n == Some(0));
                    let drawn = if empty {
                        icons::item_texture("grass_block").is_some_and(|f| icons::gray_image(&p, ir, &f))
                    } else {
                        icons::item_icon(&p, ir, id, tint)
                    };
                    if !drawn {
                        icons::paint(&p, ir, Icon::File, theme::text_dim());
                    }
                    let ink = if enchanted {
                        Color32::from_rgb(0xCF, 0xA8, 0xFF)
                    } else if empty {
                        theme::text_dim()
                    } else {
                        theme::text()
                    };
                    after += 22.0 * k;
                    let shown_name = if empty { "Empty".to_string() } else { item_name(id) };
                    let name = p.layout_no_wrap(shown_name, theme::regular(13.0 * k), ink);
                    let w = name.size().x;
                    p.galley(pos2(after, rect.center().y - name.size().y / 2.0), name, ink);
                    after += w + 6.0 * k;
                    // Its custom name, in italics.
                    if let Some(custom) = tag.as_compound().and_then(crate::naming::item_name).filter(|n| !n.trim().is_empty()) {
                        let g = p.layout_no_wrap(format!("\u{201c}{custom}\u{201d}"), theme::italic(12.5 * k), theme::text_muted());
                        let w = g.size().x;
                        p.galley(pos2(after, rect.center().y - g.size().y / 2.0), g, theme::text_muted());
                        after += w + 6.0 * k;
                    }
                    if !empty && n.is_some_and(|n| n != 1) {
                        let c = p.layout_no_wrap(
                            format!("\u{d7}{}", n.unwrap_or_default()),
                            theme::regular(12.5 * k),
                            theme::text_muted(),
                        );
                        let w = c.size().x;
                        p.galley(pos2(after, rect.center().y - c.size().y / 2.0), c, theme::text_muted());
                        after += w + 8.0 * k;
                    }
                }
                if let Some((icon, name)) = &thing {
                    let ir = Rect::from_min_size(pos2(after, rect.center().y - 8.0 * k), Vec2::splat(16.0 * k));
                    if !icons::item_icon(&p, ir, icon, Color32::WHITE) {
                        icons::paint(&p, ir, Icon::File, theme::text_dim());
                    }
                    after += 22.0 * k;
                    // Half of a large chest: named so, with which half and where the other is,
                    // and a dot in a color of the pair's own.
                    let pair = cx.chests.get(&path).copied();
                    let shown = if pair.is_some() { format!("Large {name}") } else { name.clone() };
                    let g = p.layout_no_wrap(shown, theme::regular(13.0 * k), theme::text());
                    let w = g.size().x;
                    p.galley(pos2(after, rect.center().y - g.size().y / 2.0), g, theme::text());
                    after += w + 8.0 * k;
                    // Its custom name (a named mob, a named chest), in italics.
                    if let Some(custom) = tag.as_compound().and_then(crate::naming::mob_name).filter(|n| !n.trim().is_empty()) {
                        let g = p.layout_no_wrap(format!("\u{201c}{custom}\u{201d}"), theme::italic(12.5 * k), theme::text_muted());
                        let w = g.size().x;
                        p.galley(pos2(after, rect.center().y - g.size().y / 2.0), g, theme::text_muted());
                        after += w + 8.0 * k;
                    }
                    if let (Some((right, other)), Some(Seg::Index(me))) = (pair, path.last()) {
                        let hue = ((*me).min(other) as f32 * 0.618_034).fract();
                        let dot = Color32::from(egui::ecolor::Hsva::new(hue, 0.55, 0.95, 1.0));
                        p.circle_filled(pos2(after + 4.0 * k, rect.center().y), 4.0 * k, dot);
                        after += 12.0 * k;
                        let note = format!("{} half", if right { "right" } else { "left" });
                        let g = p.layout_no_wrap(note, theme::regular(12.0 * k), theme::text_dim());
                        let w = g.size().x;
                        p.galley(pos2(after, rect.center().y - g.size().y / 2.0), g, theme::text_dim());
                        after += w + 8.0 * k;
                    }
                }
                if matches!(&*tag, Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_)) && !is_gone {
                    // "Hex Edit..." is on offer on the selected or hovered row.
                    // (The row itself stops being hovered once the pointer is on the button.)
                    let over_row = ui.input(|i| i.pointer.hover_pos()).is_some_and(|q| rect.contains(q));
                    if selected || over_row {
                        let font = theme::regular(12.0 * k);
                        let w = p.layout_no_wrap("Hex Edit...".into(), font.clone(), theme::text()).size().x + 16.0 * k;
                        let br = Rect::from_min_size(pos2(after, rect.top() + 2.0 * k), vec2(w, rh - 4.0 * k));
                        let bresp = ui.interact(br, ui.id().with(("hex-open", &key)), Sense::click());
                        p.rect_filled(
                            br,
                            theme::radius(3),
                            if bresp.hovered() { theme::button_hover() } else { theme::button() },
                        );
                        p.text(br.center(), Align2::CENTER_CENTER, "Hex Edit...", font, theme::text());
                        if bresp.clicked() {
                            actions.push(Action::OpenHex(path.clone()));
                        }
                    }
                    after += 90.0 * k;
                }
            }
            scalar => {
                gloss = hint(label, scalar);
                let text_len = match &*scalar {
                    Tag::String(s) => (s.chars().count().clamp(4, 60) as f32 * 7.2 + 14.0) * k,
                    Tag::Long(v) => (v.to_string().len() as f32 * 7.6 + 16.0) * k,
                    Tag::Float(_) | Tag::Double(_) => {
                        (if cx.full_precision { 150.0 } else { 70.0 }) * k
                    }
                    other => other
                        .as_i64()
                        .map_or(40.0 * k, |v| (v.to_string().len() as f32 * 7.6 + 18.0) * k),
                };
                let r = value_rect(text_len.max(36.0 * k));
                if let Some(info) = cx.rows.borrow_mut().last_mut() {
                    info.field = Some(r);
                }
                p.rect_filled(r, theme::radius(2), theme::field());
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
                child.style_mut().visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                child.style_mut().visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
                child.style_mut().visuals.extreme_bg_color = Color32::TRANSPARENT;
                child.style_mut().spacing.interact_size.y = rh - 2.0 * k;
                child.style_mut().spacing.button_padding = vec2(4.0 * k, 0.0);
                child.style_mut().override_font_id = Some(theme::regular(13.0 * k));
                let decimals = if cx.full_precision { 17 } else { 4 };
                // While a number box is being typed in, only characters a number can have get in.
                let vid = Id::new(("value-box", &key));
                let floating = matches!(scalar, Tag::Float(_) | Tag::Double(_));
                if !matches!(scalar, Tag::String(_))
                    && ui.data(|d| d.get_temp::<Id>(vid)).is_some_and(|id| ui.memory(|m| m.has_focus(id)))
                {
                    let ok = move |c: char| c.is_ascii_digit() || c == '-' || c == '+' || (floating && matches!(c, '.' | 'e' | 'E'));
                    ui.input_mut(|i| {
                        i.events.retain_mut(|e| match e {
                            egui::Event::Text(s) | egui::Event::Paste(s) => {
                                s.retain(ok);
                                !s.is_empty()
                            }
                            _ => true,
                        })
                    });
                }
                let resp = child
                    .add_enabled_ui(!cx.read_only && !is_gone, |ui| match scalar {
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
                ui.data_mut(|d| d.insert_temp(vid, resp.id));
                if resp.has_focus() {
                    cx.typing.set(true);
                }
                if resp.drag_started() || resp.gained_focus() {
                    actions.push(Action::Snapshot);
                }
                if resp.changed() {
                    actions.push(Action::ChangedAt(path.clone()));
                }
                after = r.right() + 10.0 * k;
            }
        }
        // The notes from info (a picture and a label), else our own hint.
        let style = cx.styles.get(&path);
        if let Some(icon) = style.and_then(|s| s.icon) {
            let ir = Rect::from_min_size(pos2(after, rect.top() + 1.0 * k), Vec2::splat(rh - 2.0 * k));
            let drawn = match icon {
                crate::info::NoteIcon::Pack(names) => names
                    .iter()
                    .find_map(|n| icons::pack_texture(n))
                    .is_some_and(|f| icons::file_image(&p, ir, &f)),
                crate::info::NoteIcon::Own(i) => {
                    icons::paint(&p, ir, i, theme::text_muted());
                    true
                }
                crate::info::NoteIcon::Image(name) => icons::image(&p, ir, name, Color32::WHITE),
            };
            if drawn {
                after += rh + 2.0 * k;
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
                theme::italic(12.5 * k),
                color.unwrap_or(Color32::from_rgb(0x5E, 0x68, 0x75)),
            );
        }
        // Deleted: the row fades, and the only thing left to do with it is bring it back.
        if is_gone {
            p.rect_filled(rect, 0.0, Color32::from_rgba_premultiplied(34, 14, 16, 150));
            // The bar stays bright above the fade.
            p.rect_filled(
                Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
                0.0,
                theme::RED,
            );
        }
        if !cx.read_only {
            if let Some(root) = gone_root {
                ctx_menu::context(ui, &resp, |ui, _| {
                    if white_item(ui, Icon::Undo, "Undo") {
                        actions.push(Action::Restore(root.clone()));
                    }
                });
            } else {
                let extras = {
                    let mut e = Extras { data_version: cx.data_version, ..Default::default() };
                    match &*tag {
                        Tag::Compound(c) => {
                            let kind_ok = c.get("id").and_then(Tag::as_str).is_some_and(|i| crate::inventory::Kind::of_container(i).is_some());
                            if kind_ok && matches!(c.get("Items"), Some(Tag::List(_))) {
                                let mut p = path.clone();
                                p.push(Seg::Key("Items".into()));
                                e.inventory = Some(p);
                            }
                            if let Some((id, n)) = &item {
                                let empty = is_bedrock_item(c) && (id.is_empty() || *n == Some(0));
                                e.map = crate::naming::map_file(c, id);
                                if !empty && item_id_key(tag).is_some() {
                                    e.name = Some((true, crate::naming::item_name(c).unwrap_or_default()));
                                    use crate::item_edit::{Rule, modifiers, rule_blocks};
                                    e.item = Some((modifiers(c, cx.data_version), rule_blocks(c, Rule::Break, cx.data_version), rule_blocks(c, Rule::Place, cx.data_version)));
                                    if crate::inventory::durability(id).is_some() {
                                        e.unbreakable = Some(crate::naming::is_unbreakable(c));
                                    }
                                }
                            }
                            let block_entity = c.contains_key("x") && c.contains_key("y") && c.contains_key("z");
                            if item.is_none() {
                                e.mob = mob_menu(c, block_entity, cx.data_version);
                            }
                            // A spawner's mob; a container's lock.
                            if block_entity {
                                let spawner = crate::mob_edit::is_spawner(c).then(|| crate::mob_edit::spawner_mob(c));
                                let lock = crate::mob_edit::lockable(c).then(|| crate::mob_edit::lock(c));
                                if spawner.is_some() || lock.is_some() {
                                    e.block = Some(BlockMenu { spawner, lock });
                                }
                            }
                            if e.name.is_none() && (crate::naming::is_mob(c) || (kind_ok && block_entity)) {
                                e.name = Some((false, crate::naming::mob_name(c).unwrap_or_default()));
                            }
                        }
                        Tag::List(l) => {
                            let named = matches!(label, "Inventory" | "EnderItems" | "EnderChestInventory") || (label == "Items" && cx.container.get());
                            if named && matches!(l.element_type(), TagType::Compound | TagType::End) {
                                e.inventory = Some(path.clone());
                            }
                        }
                        _ => {}
                    }
                    e
                };
                ctx_menu::context(ui, &resp, |ui, mcx| {
                    Self::menu(
                        ui,
                        mcx,
                        &path,
                        label,
                        tag,
                        in_compound,
                        list_len,
                        cx.has_clipboard,
                        cx.deletable,
                        several,
                        undoable,
                        redoable,
                        &extras,
                        actions,
                    )
                });
            }
        }
        if container && open {
            match tag {
                Tag::Compound(c) => {
                    let is_container = c.get("id").and_then(Tag::as_str).is_some_and(|i| crate::inventory::Kind::of_container(i).is_some());
                    let outer = cx.container.replace(is_container);
                    // Shown by type, then name; the stored order is unchanged.
                    let mut keys: Vec<String> = c.keys().cloned().collect();
                    keys.sort_by_cached_key(|k| (type_rank(c[k].tag_type()), k.to_lowercase()));
                    for k in keys {
                        let mut child = path.clone();
                        child.push(Seg::Key(k.clone()));
                        if let Some(v) = c.get_mut(&k) {
                            Self::row(ui, cx, &k, v, child, true, 0, actions, hits);
                        }
                    }
                    cx.container.set(outer);
                }
                Tag::List(l) => {
                    let len = l.len();
                    for (i, v) in l.iter_mut().enumerate() {
                        let mut child = path.clone();
                        child.push(Seg::Index(i));
                        Self::row(ui, cx, &i.to_string(), v, child, false, len, actions, hits);
                    }
                }
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn menu(
        ui: &mut Ui,
        cx: &mut ctx_menu::Cx,
        path: &TagPath,
        label: &str,
        tag: &Tag,
        in_compound: bool,
        list_len: usize,
        has_clipboard: bool,
        deletable: bool,
        several: bool,
        undoable: bool,
        redoable: bool,
        extras: &Extras,
        actions: &mut Vec<Action>,
    ) {
        let root = path.is_empty();
        if several && !root {
            // Several tags selected: only what applies to all of them.
            let mut item = |ui: &mut Ui, icon: Icon, text: &str, hint: &str, a: Action| {
                if ctx_menu::item(ui, icon, text, hint, true) {
                    actions.push(a);
                }
            };
            item(ui, Icon::Cut, "Cut", &ctx_menu::shortcut("X"), Action::Cut(path.clone()));
            item(ui, Icon::Copy, "Copy", &ctx_menu::shortcut("C"), Action::Copy(path.clone()));
            ctx_menu::separator(ui);
            item(ui, Icon::Trash, "Delete", "Del", Action::Delete(path.clone()));
            return;
        }
        if undoable && white_item(ui, Icon::Undo, "Undo") {
            actions.push(Action::Restore(path.clone()));
        }
        if redoable && white_item(ui, Icon::Redo, "Redo") {
            actions.push(Action::RedoAt(path.clone()));
        }
        if matches!(tag, Tag::Compound(_) | Tag::List(_)) {
            let picked = ctx_menu::submenu(ui, cx, Icon::Plus, "Add Tag", |ui| {
                let mut picked = None;
                for (t, name) in ADDABLE {
                    if let Tag::List(l) = tag {
                        if l.element_type() != TagType::End && l.element_type() != t {
                            continue;
                        }
                    }
                    if ctx_menu::painted(ui, name, "", true, |p, r, _| type_icon(p, r, t)) {
                        picked = Some(t);
                    }
                }
                picked
            });
            if let Some(t) = picked {
                actions.push(Action::Add(path.clone(), t));
            }
            ctx_menu::separator(ui);
        }
        // An inventory list opens as the game's inventory screen.
        if let Some(target) = &extras.inventory {
            if ctx_menu::painted(ui, "Edit Inventory", "", true, ctx_menu::item_icon("chest")) {
                actions.push(Action::OpenInventory(target.clone()));
            }
            ctx_menu::separator(ui);
        }
        // An item stack: what it is and has, together under `Item ▸`.
        if let Some(key) = item_id_key(tag) {
            let current = tag.as_compound().and_then(item_stack).map(|(id, _)| id).unwrap_or_default();
            // Console and old worlds name an item by number: only items that have one are offered.
            let numeric = !matches!(tag.get(key), Some(Tag::String(_)));
            let icon = current.clone();
            let chosen = ctx_menu::submenu_painted(ui, cx, move |p, r, ink| {
                if !icons::item_icon_gray(p, r, &icon) {
                    icons::paint(p, r, Icon::File, ink);
                }
            }, "Item", 260.0, 380.0, |ui| {
                let old = ctx_menu::set_row_width(ui.ctx(), 260.0);
                let mut inner = ctx_menu::Cx::default();
                let mut out: Vec<Action> = Vec::new();
                // The item, and how many (at the top of the list, up to what the stack's count
                // holds: an int 99, as the game reads it; a byte 127).
                if let Some(c) = tag.as_compound() {
                    let (count_key, count) = match (c.get("count"), c.get("Count")) {
                        (Some(t), _) => ("count", t.as_i64().unwrap_or(1)),
                        (None, Some(t)) => ("Count", t.as_i64().unwrap_or(1)),
                        _ => ("count", 1),
                    };
                    let most = if count_key == "count" { 99 } else { 127 };
                    let picked = ctx_menu::submenu_wide(ui, &mut inner, Icon::Refresh, "Change Item", crate::item_picker::WIDTH, crate::item_picker::HEIGHT + 30.0, |ui| {
                        crate::item_picker::show_counted(ui, &current, numeric, count, most)
                    });
                    // A potion's kind and effects; a shulker box's color.
                    if let Some(e) = crate::item_edit::special_rows(ui, &mut inner, &path_key(path), &current, c, extras.data_version) {
                        out.push(Action::EditItem(path.clone(), e));
                    }
                    if let Some((id, n)) = picked {
                        if let Some(id) = id {
                            out.push(Action::SetItem(path.clone(), id));
                        }
                        if n != count {
                            let mut p = path.clone();
                            p.push(Seg::Key(count_key.into()));
                            let value = match c.get(count_key) {
                                Some(Tag::Byte(_)) => Tag::Byte(n as i8),
                                Some(Tag::Short(_)) => Tag::Short(n as i16),
                                _ => Tag::Int(n as i32),
                            };
                            out.push(Action::SetValues(vec![(p, value)], format!("Set the count to {n}")));
                        }
                    }
                    if let Some(e) = crate::enchant::menu(ui, &mut inner, &crate::enchant::read(c)) {
                        out.push(Action::SetEnchant(path.clone(), e));
                    }
                }
                if let Some(on) = extras.unbreakable {
                    if ctx_menu::toggle(ui, "Unbreakable", on) {
                        out.push(Action::SetUnbreakable(path.clone(), !on));
                    }
                }
                if let Some((mods, breaks, places)) = &extras.item {
                    if let Some(e) = crate::item_edit::menu_rows(ui, &mut inner, &path_key(path), mods.as_deref(), breaks, places) {
                        out.push(Action::EditItem(path.clone(), e));
                    }
                }
                if let Some((true, current)) = &extras.name {
                    if let Some(n) = crate::item_edit::name_menu(ui, &mut inner, current) {
                        out.push(Action::SetName(path.clone(), n));
                    }
                }
                if let Some(map) = &extras.map {
                    if ctx_menu::painted(ui, "Open Map Editor", "", true, ctx_menu::item_icon("filled_map")) {
                        out.push(Action::OpenMap(map.clone()));
                    }
                }
                ctx_menu::set_row_width(ui.ctx(), old);
                (!out.is_empty()).then_some(out)
            });
            actions.extend(chosen.into_iter().flatten());
            ctx_menu::separator(ui);
        }
        // A mob (or armor stand, item frame): what it is and has, under its name; an end
        // gateway: where it sends things.
        if let Some(m) = &extras.mob {
            use crate::mob_edit::MobEdit;
            let gateway = m.gateway.is_some();
            let title = if gateway { "End Gateway".to_string() } else { item_name(&m.id) };
            let egg = if gateway { "end_portal_frame".to_string() } else { m.icon.clone() };
            let chosen = ctx_menu::submenu_painted(ui, cx, move |p, r, ink| {
                if !icons::item_icon_gray(p, r, &egg) {
                    icons::paint(p, r, Icon::File, ink);
                }
            }, &title, 260.0, 320.0, |ui| {
                let old = ctx_menu::set_row_width(ui.ctx(), 260.0);
                let mut inner = ctx_menu::Cx::default();
                let mut out: Vec<Action> = Vec::new();
                let edit = |out: &mut Vec<Action>, e: MobEdit| out.push(Action::EditMob(path.clone(), e));
                if m.trades && ctx_menu::painted(ui, "Edit Trades", "", true, ctx_menu::item_icon("emerald")) {
                    out.push(Action::OpenTrades(path.clone()));
                }
                if let Some(on) = m.invulnerable {
                    if ctx_menu::toggle(ui, "Invulnerable", on) {
                        edit(&mut out, MobEdit::Invulnerable(!on));
                    }
                }
                if let Some(on) = m.no_ai {
                    if ctx_menu::toggle(ui, "No AI", on) {
                        edit(&mut out, MobEdit::NoAi(!on));
                    }
                }
                if let Some(on) = m.invisible {
                    if ctx_menu::toggle(ui, "Invisible", on) {
                        edit(&mut out, MobEdit::Invisible(!on));
                    }
                }
                // Shown as the tag is: ticked when `NoGravity` is set (it floats).
                if let Some(falls) = m.gravity {
                    if ctx_menu::toggle(ui, "No Gravity", !falls) {
                        edit(&mut out, MobEdit::Gravity(!falls));
                    }
                }
                if !m.equipment.is_empty() {
                    let rows = m.equipment.len() as f32;
                    let picked = ctx_menu::submenu_painted(ui, &mut inner, ctx_menu::item_icon("iron_chestplate"), "Equipment", 280.0, 20.0 + rows * 32.0, |ui| {
                        let old = ctx_menu::set_row_width(ui.ctx(), 280.0);
                        let mut slots = ctx_menu::Cx::default();
                        let mut out = None;
                        for (slot, item, stack) in &m.equipment {
                            let label = match item {
                                Some((id, 1)) => format!("{}: {}", slot.name(), crate::item_picker::display_name(id)),
                                Some((id, n)) => format!("{}: {} \u{d7}{n}", slot.name(), crate::item_picker::display_name(id)),
                                None => format!("{}: nothing", slot.name()),
                            };
                            // In gray, as the menus' icons are: the item, or the game's picture of
                            // the slot while empty.
                            let (icon_id, empty) = (item.as_ref().map(|i| i.0.clone()), slot.empty_picture());
                            let chosen = ctx_menu::submenu_painted(
                                ui,
                                &mut slots,
                                move |p, r, ink| {
                                    let drawn = match &icon_id {
                                        Some(id) => icons::item_icon_gray(p, r, id),
                                        None => icons::pack_texture(empty).is_some_and(|f| icons::gray_image(p, r, &f)),
                                    };
                                    if !drawn {
                                        p.rect_stroke(r.shrink(2.0), theme::radius(2), egui::Stroke::new(1.0_f32, ink), egui::StrokeKind::Inside);
                                    }
                                },
                                &label,
                                260.0,
                                if stack.is_some() { 220.0 } else { 40.0 },
                                |ui| {
                                    let old = ctx_menu::set_row_width(ui.ctx(), 260.0);
                                    let mut rows = ctx_menu::Cx::default();
                                    let mut got = None;
                                    // The item, and how many at the top of the list (the count on
                                    // its own too).
                                    let (now_id, now_n) = item.clone().unwrap_or_default();
                                    let (icon, text) = if item.is_some() { (Icon::Refresh, "Change Item") } else { (Icon::Plus, "Add Item") };
                                    let picked = ctx_menu::submenu_wide(ui, &mut rows, icon, text, crate::item_picker::WIDTH, crate::item_picker::HEIGHT + 30.0, |ui| {
                                        crate::item_picker::show_counted(ui, &now_id, false, now_n.max(1), 99)
                                    });
                                    match picked {
                                        Some((Some(id), n)) => got = Some(MobEdit::Equip(*slot, Some((id, n)))),
                                        Some((None, n)) if !now_id.is_empty() => got = Some(MobEdit::Equip(*slot, Some((now_id.clone(), n)))),
                                        _ => {}
                                    }
                                    if item.is_some() {
                                        if ctx_menu::item(ui, Icon::Trash, "Remove", "", true) {
                                            got = Some(MobEdit::Equip(*slot, None));
                                        }
                                        ctx_menu::separator(ui);
                                    }
                                    // What the stack has: enchantments, attributes (and the slots
                                    // they work in), unbreakable, its name.
                                    if let (Some(stack), Some((id, _))) = (stack, item) {
                                        let key = format!("{}-{}", path_key(path), slot.name());
                                        if let Some(e) = crate::item_edit::stack_menu(ui, &mut rows, &key, id, stack, m.data_version) {
                                            got = Some(MobEdit::EquipStack(*slot, e));
                                        }
                                    }
                                    ctx_menu::set_row_width(ui.ctx(), old);
                                    got
                                },
                            );
                            if chosen.is_some() {
                                out = chosen;
                            }
                        }
                        ctx_menu::set_row_width(ui.ctx(), old);
                        out
                    });
                    if let Some(e) = picked {
                        edit(&mut out, e);
                    }
                }
                // What it drops when it dies (each slot's chance, its loot table) and whether it
                // picks things up.
                if let Some(d) = &m.drops {
                    let rows = d.chances.len() as f32 + if d.has_loot_table { 1.0 } else { 0.0 } + 1.0;
                    let picked = ctx_menu::submenu_painted(ui, &mut inner, ctx_menu::item_icon("bone"), "Drops", 300.0, 20.0 + rows * 32.0, |ui| {
                        let old = ctx_menu::set_row_width(ui.ctx(), 300.0);
                        let mut tables = ctx_menu::Cx::default();
                        let mut got: Option<MobEdit> = None;
                        for (slot, chance) in &d.chances {
                            // As a percentage: 100 is always (saved above 1, so it drops as it was).
                            let key = egui::Id::new(("drop-chance", path_key(path), *slot as u8));
                            let now = (chance.min(1.0) * 100.0 * 100.0).round() / 100.0;
                            let mut v: f64 = ui.ctx().data(|dd| dd.get_temp(key)).unwrap_or(now);
                            // Pictured by what the slot holds (else the slot's own picture), in gray.
                            let held = m.equipment.iter().find(|(s, ..)| s == slot).and_then(|(_, i, _)| i.as_ref().map(|i| i.0.clone()));
                            let empty = slot.empty_picture();
                            let icon = move |p: &egui::Painter, r: Rect, ink: Color32| {
                                let drawn = match &held {
                                    Some(id) => icons::item_icon_gray(p, r, id),
                                    None => icons::pack_texture(empty).is_some_and(|f| icons::gray_image(p, r, &f)),
                                };
                                if !drawn {
                                    icons::paint(p, r, Icon::Rename, ink);
                                }
                            };
                            let (changed, done) = ctx_menu::decimal_row_painted(ui, icon, &format!("{} %", slot.name()), &mut v, 0.0..=100.0);
                            if changed {
                                ui.ctx().data_mut(|dd| dd.insert_temp(key, v));
                            }
                            if done {
                                ui.ctx().data_mut(|dd| dd.remove_temp::<f64>(key));
                                if (v - now).abs() > 1e-9 {
                                    got = Some(MobEdit::DropChance(*slot, if v >= 100.0 { 2.0 } else { v / 100.0 }));
                                }
                            }
                        }
                        if d.has_loot_table {
                            // Picked from the list of tables, the mob's own at the top.
                            let own = format!("minecraft:entities/{}", m.id.trim_start_matches("minecraft:"));
                            let label = match d.loot_table.as_deref() {
                                Some(t) => format!("Loot Table: {}", crate::item_picker::loot_tables().iter().find(|x| x.id == t).map_or_else(|| t.trim_start_matches("minecraft:").to_string(), |x| x.name.clone())),
                                None => "Loot Table: Default".to_string(),
                            };
                            let chosen = ctx_menu::submenu_painted(ui, &mut tables, ctx_menu::item_icon("chest"), &label, crate::item_picker::WIDTH, crate::item_picker::HEIGHT, |ui| {
                                crate::item_picker::show_loot_tables(ui, d.loot_table.as_deref(), &own)
                            });
                            if let Some(t) = chosen {
                                got = Some(MobEdit::LootTable(t));
                            }
                        }
                        if ctx_menu::toggle(ui, "Can Pick Up Loot", d.pick_up) {
                            got = Some(MobEdit::PickUp(!d.pick_up));
                        }
                        ctx_menu::set_row_width(ui.ctx(), old);
                        got
                    });
                    if let Some(e) = picked {
                        edit(&mut out, e);
                    }
                }
                // Its status effects: each one's level and time, and adding more.
                if let Some((effects, choices)) = &m.effects {
                    if let Some(e) = crate::effects::menu(ui, &mut inner, &path_key(path), "Status Effects", effects, choices) {
                        edit(&mut out, MobEdit::Effect(e));
                    }
                }
                // A shulker's color.
                if let Some(now) = m.shulker_color {
                    let shown = if now == 16 { "shulker_box".to_string() } else { format!("{}_shulker_box", crate::mob_edit::COLORS.get(now as usize).copied().unwrap_or("purple")) };
                    let picked = ctx_menu::submenu_painted(ui, &mut inner, ctx_menu::item_icon(&shown), "Color", 220.0, 20.0 + 17.0 * 26.0, |ui| {
                        let old = ctx_menu::set_row_width(ui.ctx(), 220.0);
                        let mut got = None;
                        for (k, (cid, name)) in crate::item_edit::shulker_colors().into_iter().enumerate() {
                            // Undyed first in the list; 16 as the shulker keeps it.
                            let v = if k == 0 { 16 } else { k as i64 - 1 };
                            let current = v == now;
                            let clicked = ctx_menu::painted(ui, &name, if current { "\u{2713}" } else { "" }, true, move |p, r, ink| {
                                if !icons::item_icon(p, r, &cid, Color32::WHITE) {
                                    icons::paint(p, r, Icon::File, ink);
                                }
                            });
                            if clicked && !current {
                                got = Some(v);
                            }
                        }
                        ctx_menu::set_row_width(ui.ctx(), old);
                        got
                    });
                    if let Some(v) = picked {
                        edit(&mut out, MobEdit::ShulkerColor(v));
                    }
                }
                if let Some((at, exact)) = m.gateway {
                    for (k, axis) in ["Exit X", "Exit Y", "Exit Z"].iter().enumerate() {
                        let key = egui::Id::new(("gateway-exit", path_key(path), k));
                        let mut v: i64 = ui.ctx().data(|d| d.get_temp(key)).unwrap_or(at[k]);
                        let (changed, done) = ctx_menu::number_row(ui, Icon::Rename, axis, &mut v, -30_000_000..=30_000_000, "");
                        if changed {
                            ui.ctx().data_mut(|d| d.insert_temp(key, v));
                        }
                        if done {
                            ui.ctx().data_mut(|d| d.remove_temp::<i64>(key));
                            if v != at[k] {
                                let mut to = at;
                                to[k] = v;
                                edit(&mut out, MobEdit::GatewayExit(to));
                            }
                        }
                    }
                    if let Some(on) = exact {
                        if ctx_menu::toggle(ui, "Exact Teleport", on) {
                            edit(&mut out, MobEdit::GatewayExact(!on));
                        }
                    }
                }
                if let Some((false, current)) = &extras.name {
                    if let Some(n) = crate::item_edit::name_menu(ui, &mut inner, current) {
                        out.push(Action::SetName(path.clone(), n));
                    }
                }
                ctx_menu::set_row_width(ui.ctx(), old);
                (!out.is_empty()).then_some(out)
            });
            actions.extend(chosen.into_iter().flatten());
            ctx_menu::separator(ui);
        } else if extras.name.as_ref().is_some_and(|(item, _)| !item) || extras.block.is_some() {
            use crate::mob_edit::MobEdit;
            // A spawner: the mob it spawns, from the list of mobs.
            if let Some(spawner) = extras.block.as_ref().and_then(|b| b.spawner.as_ref()) {
                let label = format!("Spawns: {}", spawner.as_deref().map_or_else(|| "Nothing".to_string(), item_name));
                let egg = spawner.as_deref().map_or_else(|| "spawner".to_string(), |id| format!("{}_spawn_egg", id.trim_start_matches("minecraft:")));
                let picked = ctx_menu::submenu_painted(ui, cx, ctx_menu::item_icon(&egg), &label, crate::item_picker::WIDTH, crate::item_picker::HEIGHT, |ui| {
                    crate::item_picker::show_mobs(ui, spawner.as_deref())
                });
                if let Some(id) = picked {
                    actions.push(Action::EditMob(path.clone(), MobEdit::SpawnerMob(id)));
                }
            }
            // A container: its lock (the name of the key that opens it) and its name.
            if let Some(key) = extras.block.as_ref().and_then(|b| b.lock.as_ref()) {
                let label = if key.is_empty() { "Lock".to_string() } else { format!("Lock: {key}") };
                let picked = ctx_menu::submenu_painted(ui, cx, ctx_menu::item_icon("tripwire_hook"), &label, 300.0, 80.0, |ui| {
                    let old = ctx_menu::set_row_width(ui.ctx(), 300.0);
                    let mut got = None;
                    if let Some(k) = ctx_menu::text_row(ui, Icon::Rename, "Key Name", key, "Name of the key item") {
                        if k.trim() != key {
                            got = Some(k.trim().to_string());
                        }
                    }
                    if !key.is_empty() && ctx_menu::item(ui, Icon::Trash, "Unlock", "", true) {
                        got = Some(String::new());
                    }
                    ctx_menu::set_row_width(ui.ctx(), old);
                    got
                });
                if let Some(k) = picked {
                    actions.push(Action::EditMob(path.clone(), MobEdit::Lock(k)));
                }
            }
            if let Some((false, current)) = &extras.name {
                if let Some(n) = crate::item_edit::name_menu(ui, cx, current) {
                    actions.push(Action::SetName(path.clone(), n));
                }
            }
            ctx_menu::separator(ui);
        }
        // Arrays open in the hex editor, and as a picture where there is one.
        if matches!(tag, Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_)) {
            if ctx_menu::item(ui, Icon::File, "Hex Edit...", "", true) {
                actions.push(Action::OpenHex(path.clone()));
            }
            ctx_menu::separator(ui);
        }
        let mut item = |ui: &mut Ui, icon: Icon, text: &str, hint: &str, on: bool, a: Action| {
            if ctx_menu::item(ui, icon, text, hint, on) {
                actions.push(a);
            }
        };
        if !root {
            item(
                ui,
                Icon::Cut,
                "Cut",
                &ctx_menu::shortcut("X"),
                true,
                Action::Cut(path.clone()),
            );
        }
        item(
            ui,
            Icon::Copy,
            "Copy",
            &ctx_menu::shortcut("C"),
            true,
            Action::Copy(path.clone()),
        );
        if matches!(tag, Tag::Compound(_) | Tag::List(_)) {
            item(
                ui,
                Icon::Paste,
                "Paste",
                &ctx_menu::shortcut("V"),
                has_clipboard,
                Action::Paste(path.clone()),
            );
        }
        if !root {
            let can_move = matches!(path.last(), Some(Seg::Index(i)) if *i > 0 || i + 1 < list_len);
            if in_compound || can_move {
                ctx_menu::separator(ui);
            }
            if in_compound {
                item(
                    ui,
                    Icon::Rename,
                    "Rename",
                    "Double-click",
                    true,
                    Action::StartRename(path.clone(), label.to_string()),
                );
            }
            // Only list entries have an order that shows (compounds list by type and name).
            if let Some(Seg::Index(i)) = path.last() {
                if *i > 0 {
                    item(ui, Icon::ArrowUp, "Move Up", "", true, Action::Move(path.clone(), -1));
                }
                if i + 1 < list_len {
                    item(ui, Icon::ArrowDown, "Move Down", "", true, Action::Move(path.clone(), 1));
                }
            }
            ctx_menu::separator(ui);
            item(
                ui,
                Icon::Trash,
                "Delete",
                "Del",
                true,
                Action::Delete(path.clone()),
            );
        } else if deletable {
            // The top tag is the whole file (or database record): delete that.
            ctx_menu::separator(ui);
            item(ui, Icon::Trash, "Delete File", "", true, Action::DeleteDocument);
        }
    }

    /// What an action on `p` applies to: the whole selection when `p` is in it, else just `p`.
    /// Tags inside another selected tag are left out, and the rest are in display order.
    fn targets(&self, p: &TagPath) -> Vec<TagPath> {
        if !self.marked.contains(p) {
            return vec![p.clone()];
        }
        let inside = |q: &TagPath, o: &TagPath| q.len() > o.len() && q[..o.len()] == o[..];
        let mut out: Vec<TagPath> = self
            .marked
            .iter()
            .filter(|q| !self.marked.iter().any(|o| inside(q, o)))
            .cloned()
            .collect();
        let order: HashMap<&TagPath, usize> =
            self.row_order.iter().enumerate().map(|(i, p)| (p, i)).collect();
        out.sort_by_key(|q| order.get(q).copied().unwrap_or(usize::MAX));
        out
    }

    /// Puts copies of the tags on the editor's clipboard.
    fn copy_tags(&mut self, paths: &[TagPath]) -> bool {
        let mut taken = Vec::new();
        for p in paths {
            let Some(t) = get_mut(&mut self.root.tag, p) else {
                continue;
            };
            let name = match p.last() {
                Some(Seg::Key(k)) => k.clone(),
                _ => "copy".into(),
            };
            taken.push((name, t.clone()));
        }
        if taken.is_empty() {
            return false;
        }
        self.status = if taken.len() == 1 {
            "Copied".into()
        } else {
            format!("Copied {} tags", taken.len())
        };
        self.clipboard = taken;
        true
    }

    /// Whether the tag is deleted (or inside a deleted tag), so it is only waiting to go.
    fn under_gone(&self, p: &TagPath) -> bool {
        self.marks
            .list
            .iter()
            .any(|m| m.kind == Kind::Gone && p.starts_with(m.path.as_slice()))
    }

    fn is_fresh(&self, p: &TagPath) -> bool {
        self.marks.list.iter().any(|m| m.kind == Kind::Fresh && m.path == *p)
    }

    /// Takes the tag out of its compound or list.
    fn remove_at(&mut self, p: &TagPath) -> bool {
        let parent = p[..p.len() - 1].to_vec();
        match (get_mut(&mut self.root.tag, &parent), p.last()) {
            (Some(Tag::Compound(c)), Some(Seg::Key(k))) => c.shift_remove(k).is_some(),
            (Some(Tag::List(l)), Some(Seg::Index(i))) if *i < l.len() => {
                l.remove(*i);
                true
            }
            _ => false,
        }
    }

    /// Marks the tags deleted: they stay, red and faint, until the document is saved. A tag added
    /// since the last save was never kept, so that one really goes. The root cannot be deleted.
    /// `paths` is in display order, so going backwards takes higher list indices first.
    fn delete_tags(&mut self, paths: &[TagPath]) -> bool {
        let mut any = false;
        for p in paths.iter().rev().filter(|p| !p.is_empty()) {
            if self.under_gone(p) {
                continue;
            }
            if self.is_fresh(p) {
                if self.remove_at(p) {
                    self.marks.list.retain(|m| !m.path.starts_with(p.as_slice()));
                    self.undone.retain(|u| !u.path().starts_with(p.as_slice()));
                    if let Some(Seg::Index(i)) = p.last() {
                        self.close_gap(&p[..p.len() - 1].to_vec(), *i);
                    }
                    any = true;
                }
            } else {
                // Tags already deleted inside this one are part of it now.
                self.marks
                    .list
                    .retain(|m| !(m.kind == Kind::Gone && m.path.starts_with(p.as_slice())));
                self.marks.list.push(Mark {
                    kind: Kind::Gone,
                    path: p.clone(),
                    base: None,
                });
                any = true;
            }
        }
        if any {
            self.selected = None;
            self.marked.clear();
        }
        any
    }

    /// Removes the deleted tags for good (when the document is saved) and forgets what was new or
    /// changed.
    pub fn purge_deleted(&mut self) {
        let mut gone: Vec<TagPath> = self
            .marks
            .list
            .iter()
            .filter(|m| m.kind == Kind::Gone)
            .map(|m| m.path.clone())
            .collect();
        // Highest list indices first, so the others keep their places.
        gone.sort();
        gone.reverse();
        for p in gone.iter().filter(|p| !p.is_empty()) {
            self.remove_at(p);
        }
        self.marks.list.clear();
        self.undone.clear();
        if !gone.is_empty() {
            self.selected = None;
            self.marked.clear();
            self.generation += 1;
        }
    }

    /// Marks on the later items of `parent` move up one place after item `removed` is taken out.
    fn close_gap(&mut self, parent: &TagPath, removed: usize) {
        let depth = parent.len();
        for m in self.marks.list.iter_mut() {
            let q = &mut m.path;
            if q.len() > depth && q[..depth] == parent[..] {
                if let Seg::Index(i) = &mut q[depth] {
                    if *i > removed {
                        *i -= 1;
                    }
                }
            }
        }
    }

    /// Marks follow two list items that traded places.
    fn swap_marks(&mut self, parent: &TagPath, a: usize, b: usize) {
        let depth = parent.len();
        for m in self.marks.list.iter_mut() {
            let q = &mut m.path;
            if q.len() > depth && q[..depth] == parent[..] {
                if let Seg::Index(i) = &mut q[depth] {
                    if *i == a {
                        *i = b;
                    } else if *i == b {
                        *i = a;
                    }
                }
            }
        }
    }

    /// Marks follow a tag (and what is inside it) that was renamed to `key`.
    fn rename_marks(&mut self, tag: &TagPath, key: &str) {
        let depth = tag.len();
        for m in self.marks.list.iter_mut() {
            let q = &mut m.path;
            if q.len() >= depth && q[..depth] == tag[..] {
                q[depth - 1] = Seg::Key(key.to_string());
            }
        }
    }

    /// A tag that existed when the document was opened was changed (`before` is the document as
    /// this frame began): it gets an edited mark, which goes again once it is back as it was.
    fn note_edit(&mut self, p: &TagPath, before: &Snap) {
        // A fresh edit of the tag ends what it could redo.
        self.undone.retain(|u| u.path() != p);
        if self.under_gone(p) {
            return;
        }
        if self.marks.list.iter().any(|m| m.path == *p) {
            self.drop_if_reverted(p);
            return;
        }
        let (Some(was), Some(now)) = (get_ref(&before.root.tag, p), get_ref(&self.root.tag, p)) else {
            return;
        };
        if was != now {
            self.marks.list.push(Mark {
                kind: Kind::Edited,
                path: p.clone(),
                base: Some((seg_name(p), was.clone())),
            });
        }
    }

    /// Forgets an edited mark whose tag is exactly what it was.
    fn drop_if_reverted(&mut self, p: &TagPath) {
        let Some(now) = get_ref(&self.root.tag, p) else { return };
        let name = seg_name(p);
        self.marks.list.retain(|m| {
            !(m.kind == Kind::Edited
                && m.path == *p
                && m.base.as_ref().is_some_and(|(n, t)| *n == name && t == now))
        });
    }

    /// Gives the tag in a compound a new key, keeping its place; the marks follow. Returns the
    /// tag's new path, or `None` when the name is taken or the tag is not in a compound.
    fn rename_key(&mut self, p: &TagPath, new: &str) -> Option<TagPath> {
        let Some(Seg::Key(old)) = p.last() else { return None };
        let parent = p[..p.len() - 1].to_vec();
        let Some(Tag::Compound(c)) = get_mut(&mut self.root.tag, &parent) else {
            return None;
        };
        if c.contains_key(new) {
            return None;
        }
        let old = old.clone();
        let rebuilt: Compound = std::mem::take(c)
            .into_iter()
            .map(|(k, v)| if k == old { (new.to_string(), v) } else { (k, v) })
            .collect();
        *c = rebuilt;
        self.rename_marks(p, new);
        let mut at = parent;
        at.push(Seg::Key(new.to_string()));
        Some(at)
    }

    /// Puts an edited tag (or a new one that was changed since it was added) back to its base
    /// value and name. False when there is nothing to put back. What it was is kept, so the tag
    /// can offer Redo.
    fn revert(&mut self, p: &TagPath) -> bool {
        let Some(m) = self.marks.list.iter().find(|m| m.path == *p).cloned() else {
            return false;
        };
        let Some((name, base)) = m.base else { return false };
        let before = get_ref(&self.root.tag, p).cloned();
        let was_name = seg_name(p);
        if let Some(t) = get_mut(&mut self.root.tag, p) {
            *t = base;
        }
        let mut at = p.clone();
        if matches!(p.last(), Some(Seg::Key(_))) && m.kind != Kind::Gone && was_name != name {
            match self.rename_key(p, &name) {
                Some(np) => at = np,
                None => self.status = "That name is already used".into(),
            }
        }
        if m.kind == Kind::Edited {
            self.marks.list.retain(|x| !(x.kind == Kind::Edited && x.path == at));
        }
        self.undone.retain(|u| *u.path() != at);
        if let Some(tag) = before {
            self.undone.push(Undone::Value { path: at, name: was_name, tag });
        }
        true
    }

    /// Takes back what was undone for the tag at `p`.
    fn redo_tag(&mut self, p: &TagPath, before: &Snap) -> bool {
        let Some(i) = self.undone.iter().position(|u| u.path() == p) else {
            return false;
        };
        match self.undone.remove(i) {
            Undone::Delete(path) => {
                self.delete_tags(&[path]);
            }
            Undone::Value { path, name, tag } => {
                let was = get_ref(&self.root.tag, &path).cloned();
                if let Some(t) = get_mut(&mut self.root.tag, &path) {
                    *t = tag;
                }
                let mut at = path.clone();
                let old_name = seg_name(&path);
                if matches!(path.last(), Some(Seg::Key(_))) && name != old_name {
                    if let Some(np) = self.rename_key(&path, &name) {
                        at = np;
                    }
                }
                let has_mark = self.marks.list.iter().any(|m| m.path == at);
                if !has_mark && !self.under_gone(&at) {
                    if let Some(was) = was.or_else(|| get_ref(&before.root.tag, &path).cloned()) {
                        self.marks.list.push(Mark {
                            kind: Kind::Edited,
                            path: at,
                            base: Some((old_name, was)),
                        });
                    }
                }
            }
        }
        true
    }
    /// Drag-select: the pointer drawing a box over the rows, as in a file manager.
    fn drag_select(&mut self, ui: &Ui, area: Rect, top: Pos2, rows: &[RowInfo]) {
        let (pressed, down, pos, command) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
                i.modifiers.command,
            )
        });
        // Parts of the screen that handle their own dragging, such as an image being placed.
        let claimed: Vec<Rect> = ui
            .ctx()
            .data_mut(|d| d.remove_temp(egui::Id::new(array_view::NO_MARQUEE)))
            .unwrap_or_default();
        // Leave the scrollbar on the right alone.
        let inside = Rect::from_min_max(area.min, pos2(area.right() - 14.0, area.bottom()));
        if pressed {
            self.marquee = None;
            if let Some(p) = pos.filter(|p| inside.contains(*p) && ui.rect_contains_pointer(inside))
            {
                if !rows.iter().any(|r| r.field.is_some_and(|f| f.contains(p)))
                    && !claimed.iter().any(|r| r.contains(p))
                {
                    self.marquee = Some(Marquee {
                        start: p - top,
                        base: if command { self.marked.clone() } else { Vec::new() },
                        active: false,
                    });
                }
            }
        }
        self.autoscroll = 0.0;
        if !down {
            self.marquee = None;
            return;
        }
        let (Some(m), Some(cur)) = (self.marquee.as_mut(), pos) else {
            return;
        };
        let start = top + m.start;
        if !m.active && ui.ctx().dragged_id().is_none() && start.distance(cur) > 5.0 {
            m.active = true;
        }
        if !m.active {
            return;
        }
        let rect = Rect::from_two_pos(start, cur);
        let mut marked = m.base.clone();
        for r in rows.iter().filter(|r| r.rect.intersects(rect)) {
            if !marked.contains(&r.path) {
                marked.push(r.path.clone());
            }
        }
        let first = rows.iter().find(|r| r.rect.intersects(rect));
        self.selected = first.map(|r| r.path.clone()).or(self.selected.take());
        self.marked = marked;
        let edge = 24.0;
        self.autoscroll = if cur.y < area.top() + edge {
            (area.top() + edge - cur.y).min(60.0) * 0.4
        } else if cur.y > area.bottom() - edge {
            -((cur.y - (area.bottom() - edge)).min(60.0)) * 0.4
        } else {
            0.0
        };
        ui.painter().with_clip_rect(area).rect(
            rect,
            0.0,
            theme::accent_alpha(40),
            Stroke::new(1.0_f32, theme::accent()),
            egui::StrokeKind::Inside,
        );
        ui.ctx().request_repaint();
    }

    /// Ctrl+A: selects what is inside the selected compound or list, and not the compound itself.
    /// With a plain tag selected that is its siblings; with nothing selected, the whole document.
    fn select_inside(&mut self) {
        let sel = self.selected.clone().unwrap_or_default();
        let container = matches!(
            get_mut(&mut self.root.tag, &sel),
            Some(Tag::Compound(_) | Tag::List(_))
        );
        let parent = if container {
            sel
        } else {
            sel[..sel.len().saturating_sub(1)].to_vec()
        };
        let mut paths: Vec<TagPath> = self
            .row_order
            .iter()
            .filter(|q| q.len() > parent.len() && q[..parent.len()] == parent[..])
            .cloned()
            .collect();
        if paths.is_empty() {
            // Collapsed: open it and take its direct children.
            self.expanded.insert(path_key(&parent));
            let child = |seg: Seg| {
                let mut c = parent.clone();
                c.push(seg);
                c
            };
            paths = match get_mut(&mut self.root.tag, &parent) {
                Some(Tag::Compound(c)) => {
                    let mut keys: Vec<&String> = c.keys().collect();
                    keys.sort_by_cached_key(|k| (type_rank(c[*k].tag_type()), k.to_lowercase()));
                    keys.into_iter().map(|k| child(Seg::Key(k.clone()))).collect()
                }
                Some(Tag::List(l)) => (0..l.len()).map(|i| child(Seg::Index(i))).collect(),
                _ => Vec::new(),
            };
        }
        if !paths.is_empty() {
            self.selected = paths.first().cloned();
            self.marked = paths;
        }
    }

    /// A value box or the find box is being typed in, so keys belong to it. (egui's own
    /// `wants_keyboard_input` is also true after a plain click on a row, which would block the
    /// shortcuts exactly when a tag has just been selected.)
    fn typing(&self) -> bool {
        self.typing || self.find_focused
    }

    /// Ctrl+C / X / V / A and Delete on the selected tag.
    fn shortcuts(&mut self, ui: &Ui) {
        if self.rename.is_some() || self.typing() || !ui.rect_contains_pointer(ui.max_rect()) {
            return;
        }
        let (mut copy, mut cut, mut paste, mut all) = (false, false, false, false);
        ui.input(|i| {
            for e in &i.events {
                match e {
                    egui::Event::Copy => copy = true,
                    egui::Event::Cut => cut = true,
                    egui::Event::Paste(_) => paste = true,
                    egui::Event::Key { key, pressed: true, modifiers, .. } => match key {
                        egui::Key::C if modifiers.command => copy = true,
                        egui::Key::X if modifiers.command => cut = true,
                        egui::Key::V if modifiers.command => paste = true,
                        egui::Key::A if modifiers.command => all = true,
                        _ => {}
                    },
                    _ => {}
                }
            }
        });
        let selected = self.selected.clone().filter(|p| !p.is_empty());
        let whole = self.selected.clone().unwrap_or_default();
        self.frame_start = Some(self.snap());
        if all {
            self.select_inside();
        }
        if copy {
            self.apply(Action::Copy(whole.clone()));
        }
        if !self.read_only {
            if cut {
                if let Some(p) = selected.clone() {
                    self.apply(Action::Cut(p));
                }
            }
            if paste && !self.clipboard.is_empty() {
                // Into the selected compound or list, else into the one holding the selection.
                let target = match get_mut(&mut self.root.tag, &whole) {
                    Some(Tag::Compound(_) | Tag::List(_)) => whole.clone(),
                    _ => whole[..whole.len().saturating_sub(1)].to_vec(),
                };
                self.apply(Action::Paste(target));
            }
        }
    }

    fn apply(&mut self, action: Action) {
        let Some(before) = self.frame_start.clone() else {
            return;
        };
        let parent_of = |p: &TagPath| p[..p.len().saturating_sub(1)].to_vec();
        // Nothing but Restore applies to what is marked deleted.
        let blocked = match &action {
            Action::Delete(p)
            | Action::Cut(p)
            | Action::Move(p, _)
            | Action::StartRename(p, _)
            | Action::Add(p, _)
            | Action::Paste(p) => self.under_gone(p),
            _ => false,
        };
        if blocked {
            return;
        }
        let mut swapped = None;
        let mut added: Vec<Mark> = Vec::new();
        let mut name_it = None;
        match action {
            Action::Restore(p) => {
                if self.under_gone(&p) {
                    // A deleted tag comes back.
                    self.marks.list.retain(|m| !(m.kind == Kind::Gone && m.path == p));
                    self.announce(format!("Restored '{}'", seg_name(&p)));
                } else if !self.revert(&p) {
                    return;
                } else {
                    self.announce(format!("Undid the change to '{}'", seg_name(&p)));
                }
            }
            Action::ChangedAt(p) => {
                self.generation += 1;
                self.note_edit(&p, &before);
                // The value edit began with a Snapshot; name it, and say so once per tag.
                let note = format!("Edited '{}'", seg_name(&p));
                if let Some(s) = self.undo.last_mut() {
                    if s.note.is_empty() {
                        s.note = note.clone();
                    }
                }
                if self.announced.as_ref() != Some(&p) {
                    self.status = note;
                    self.announced = Some(p);
                }
                return;
            }
            Action::RedoAt(p) => {
                if !self.redo_tag(&p, &before) {
                    return;
                }
                self.announce(format!("Redid the change to '{}'", seg_name(&p)));
            }
            Action::Snapshot => {
                self.announced = None;
                self.push_undo(before);
                return;
            }
            Action::DeleteDocument => {
                self.delete_requested = true;
                return;
            }
            Action::OpenHex(p) => {
                if let Some((bytes, width)) = self.array_bytes(&p) {
                    self.hex = Some(HexState {
                        path: p,
                        ed: crate::hex_editor::HexEditor::new(bytes, width),
                        seen: self.generation,
                        last_change: 0.0,
                        picture: None,
                    });
                }
                return;
            }
            Action::OpenPicture(p, pic) => {
                if let Some((bytes, width)) = self.array_bytes(&p) {
                    self.hex = Some(HexState {
                        path: p,
                        ed: crate::hex_editor::HexEditor::new(bytes, width),
                        seen: self.generation,
                        last_change: 0.0,
                        picture: Some(pic),
                    });
                }
                return;
            }
            Action::Select(p) => {
                self.marked = vec![p.clone()];
                self.selected = Some(p);
                return;
            }
            Action::SelectMore(p) => {
                if let Some(i) = self.marked.iter().position(|m| *m == p) {
                    self.marked.remove(i);
                    self.selected = self.marked.last().cloned();
                } else {
                    // Starting from a lone selected tag keeps it in the set.
                    if self.marked.is_empty() {
                        self.marked.extend(self.selected.clone());
                    }
                    self.marked.push(p.clone());
                    self.selected = Some(p);
                }
                return;
            }
            Action::SelectRange(p) => {
                let at = |q: &TagPath| self.row_order.iter().position(|r| r == q);
                let from = self.selected.as_ref().and_then(at).or(at(&p));
                if let (Some(a), Some(b)) = (from, at(&p)) {
                    let (lo, hi) = (a.min(b), a.max(b));
                    self.marked = self.row_order[lo..=hi].to_vec();
                    // The anchor stays where it was, so the range can be stretched again.
                    if self.selected.is_none() {
                        self.selected = Some(p);
                    }
                }
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
                let paths = self.targets(&p);
                self.copy_tags(&paths);
                return;
            }
            Action::Cut(p) => {
                let paths = self.targets(&p);
                if !self.copy_tags(&paths) || !self.delete_tags(&paths) {
                    return;
                }
                self.announce(if paths.len() == 1 {
                    "Cut".into()
                } else {
                    format!("Cut {} tags", paths.len())
                });
            }
            Action::Delete(p) => {
                let paths = self.targets(&p);
                if !self.delete_tags(&paths) {
                    return;
                }
                self.announce(match paths.as_slice() {
                    [one] => format!("Deleted '{}'", seg_name(one)),
                    many => format!("Deleted {} tags", many.len()),
                });
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
                    self.edit_note = format!("Moved '{}'", seg_name(&p));
                }
                (Some(Tag::List(l)), Some(Seg::Index(i))) => {
                    let j = *i as isize + delta;
                    if j < 0 || j as usize >= l.len() {
                        return;
                    }
                    l.swap(*i, j as usize);
                    swapped = Some((parent_of(&p), *i, j as usize));
                    let mut np = parent_of(&p);
                    np.push(Seg::Index(j as usize));
                    self.selected = Some(np);
                }
                _ => return,
            },
            Action::OpenInventory(p) => {
                // A chest next to another that it makes a large chest with shows both.
                let (first, partner) = match self.chest_pair(&p) {
                    Some((a, b)) => (a, Some(b)),
                    None => (p, None),
                };
                self.inventory_partner = partner;
                self.inventory = Some((first, crate::inventory::State::default()));
                return;
            }
            Action::OpenMap(name) => {
                self.open_map_request = Some(name);
                return;
            }
            Action::EditMob(path, edit) => {
                let Some(mut c) = get_ref(&self.root.tag, &path).and_then(Tag::as_compound).cloned() else { return };
                let version = self.root.tag.get("DataVersion").and_then(Tag::as_i64);
                edit.apply(&mut c, version);
                if get_ref(&self.root.tag, &path) == Some(&Tag::Compound(c.clone())) {
                    return;
                }
                return self.apply(Action::SetValues(vec![(path, Tag::Compound(c))], edit.note()));
            }
            Action::EditItem(path, edit) => {
                let Some(mut c) = get_ref(&self.root.tag, &path).and_then(Tag::as_compound).cloned() else { return };
                let version = self.root.tag.get("DataVersion").and_then(Tag::as_i64);
                edit.apply(&mut c, version);
                if get_ref(&self.root.tag, &path) == Some(&Tag::Compound(c.clone())) {
                    return;
                }
                return self.apply(Action::SetValues(vec![(path, Tag::Compound(c))], edit.note()));
            }
            Action::OpenTrades(path) => {
                self.trades = Some(path);
                return;
            }
            Action::SetUnbreakable(path, on) => {
                let Some(mut c) = get_ref(&self.root.tag, &path).and_then(Tag::as_compound).cloned() else { return };
                let version = self.root.tag.get("DataVersion").and_then(Tag::as_i64);
                crate::naming::set_unbreakable(&mut c, on, version);
                let note = if on { "Made it unbreakable" } else { "Made it breakable" };
                return self.apply(Action::SetValues(vec![(path, Tag::Compound(c))], note.into()));
            }
            Action::SetName(path, name) => {
                let Some(mut c) = get_ref(&self.root.tag, &path).and_then(Tag::as_compound).cloned() else { return };
                let version = self.root.tag.get("DataVersion").and_then(Tag::as_i64);
                let name = Some(name.trim()).filter(|n| !n.is_empty());
                // An item stack keeps its name its own way; a mob or a chest in `CustomName`.
                if item_stack(&c).is_some() {
                    crate::naming::set_item_name(&mut c, name, version);
                } else {
                    crate::naming::set_mob_name(&mut c, name, version);
                }
                if get_ref(&self.root.tag, &path) == Some(&Tag::Compound(c.clone())) {
                    return;
                }
                let note = match name {
                    Some(n) => format!("Named it \u{201c}{n}\u{201d}"),
                    None => "Took its name away".to_string(),
                };
                return self.apply(Action::SetValues(vec![(path, Tag::Compound(c))], note));
            }
            Action::SetEnchant(path, edit) => {
                let Some(mut c) = get_ref(&self.root.tag, &path).and_then(Tag::as_compound).cloned() else { return };
                let version = self.root.tag.get("DataVersion").and_then(Tag::as_i64);
                let note = edit.note(&crate::enchant::read(&c));
                crate::enchant::apply(&mut c, &edit, version);
                if get_ref(&self.root.tag, &path) == Some(&Tag::Compound(c.clone())) {
                    return;
                }
                return self.apply(Action::SetValues(vec![(path, Tag::Compound(c))], note));
            }
            Action::SetValues(changes, note) => {
                let differs = changes.iter().any(|(p, t)| !self.under_gone(p) && get_ref(&self.root.tag, p).is_some_and(|c| c != t));
                if !differs {
                    return;
                }
                self.announced = None;
                self.edit_note = note.clone();
                self.push_undo(before.clone());
                for (p, t) in &changes {
                    if self.under_gone(p) {
                        continue;
                    }
                    if let Some(x) = get_mut(&mut self.root.tag, p) {
                        *x = t.clone();
                    }
                    self.note_edit(p, &before);
                }
                self.status = note;
                return;
            }
            Action::AddStack(p, stack) => {
                let base = Tag::Compound(stack.clone());
                match get_mut(&mut self.root.tag, &p) {
                    Some(Tag::List(l)) => {
                        if l.push(Tag::Compound(stack)).is_err() {
                            self.status = "That does not fit this list".into();
                            return;
                        }
                        let mut child = p.clone();
                        child.push(Seg::Index(l.len() - 1));
                        added.push(Mark {
                            kind: Kind::Fresh,
                            path: child,
                            base: Some(((l.len() - 1).to_string(), base)),
                        });
                    }
                    _ => return,
                }
                self.announce("Added an item".into());
            }
            Action::SetItem(p, id) => {
                let Some(key) = get_ref(&self.root.tag, &p).and_then(item_id_key) else { return };
                let mut at = p.clone();
                at.push(Seg::Key(key.to_string()));
                if self.under_gone(&at) {
                    return;
                }
                // What the id tag becomes: the name, or the number (and damage) of that item.
                let (new_id, new_damage) = match get_ref(&self.root.tag, &at) {
                    Some(Tag::String(s)) if *s != id => (Tag::String(id.clone()), None),
                    Some(Tag::Short(_)) | Some(Tag::Int(_)) => {
                        let Some((n, damage)) = mctk_mapping::legacy::legacy().numeric_item(&id) else {
                            self.status = "That item has no number in this kind of world".into();
                            return;
                        };
                        let new = match get_ref(&self.root.tag, &at) {
                            Some(Tag::Int(_)) => Tag::Int(i32::from(n)),
                            _ => Tag::Short(n as i16),
                        };
                        if get_ref(&self.root.tag, &at) == Some(&new) && damage == 0 {
                            return;
                        }
                        (new, Some(damage as i16))
                    }
                    _ => return,
                };
                // An edit of one value, as typing into it is: one undo step, and the tag is marked.
                self.announced = None;
                self.edit_note = format!("Changed the item to '{id}'");
                self.push_undo(before.clone());
                if let Some(t) = get_mut(&mut self.root.tag, &at) {
                    *t = new_id;
                }
                // The damage value is part of what the number means (dye colors, ...); a tool's
                // wear starts again.
                if let Some(damage) = new_damage {
                    let mut dmg = p.clone();
                    dmg.push(Seg::Key("Damage".into()));
                    if let Some(t) = get_mut(&mut self.root.tag, &dmg) {
                        *t = Tag::Short(damage);
                        self.note_edit(&dmg, &before);
                    } else if damage != 0 {
                        if let Some(Tag::Compound(c)) = get_mut(&mut self.root.tag, &p) {
                            c.insert("Damage".into(), Tag::Short(damage));
                        }
                    }
                }
                self.note_edit(&at, &before);
                self.status = format!("Changed the item to '{id}'");
                return;
            }
            Action::Add(p, t) => {
                let Some(new) = default_tag(t) else { return };
                let as_added = new.clone();
                // Every item deleted: the list forgets its type, so the new tag can be any.
                let emptied = self.list_all_gone(&p);
                if emptied {
                    if let Some(Tag::List(l)) = get_ref(&self.root.tag, &p) {
                        if l.element_type() != t {
                            self.marks.list.retain(|m| m.path.len() <= p.len() || !m.path.starts_with(p.as_slice()));
                            if let Some(Tag::List(l)) = get_mut(&mut self.root.tag, &p) {
                                *l = List::new(TagType::End);
                            }
                        }
                    }
                }
                match get_mut(&mut self.root.tag, &p) {
                    Some(Tag::Compound(c)) => {
                        let base = format!("New {}", type_name(t));
                        let mut key = base.clone();
                        let mut n = 1;
                        while c.contains_key(&key) {
                            n += 1;
                            key = format!("{base} {n}");
                        }
                        c.insert(key.clone(), new);
                        let mut child = p.clone();
                        child.push(Seg::Key(key.clone()));
                        added.push(Mark {
                            kind: Kind::Fresh,
                            path: child.clone(),
                            base: Some((key.clone(), as_added)),
                        });
                        // The new tag is named straight away.
                        name_it = Some((child, key));
                    }
                    Some(Tag::List(l)) => {
                        if l.push(new).is_err() {
                            self.status = "That type does not fit this list".into();
                            return;
                        }
                        let mut child = p.clone();
                        child.push(Seg::Index(l.len() - 1));
                        added.push(Mark {
                            kind: Kind::Fresh,
                            path: child,
                            base: Some((l.len().saturating_sub(1).to_string(), as_added)),
                        });
                    }
                    _ => return,
                }
                self.expanded.insert(path_key(&p));
                self.announce(format!("Added a {} tag", type_name(t).to_lowercase()));
            }
            Action::Paste(p) => {
                if self.clipboard.is_empty() {
                    return;
                }
                let items = self.clipboard.clone();
                let mut skipped = 0;
                match get_mut(&mut self.root.tag, &p) {
                    Some(Tag::Compound(c)) => {
                        for (name, tag) in items {
                            let mut key = name.clone();
                            let mut n = 1;
                            while c.contains_key(&key) {
                                n += 1;
                                key = format!("{name} {n}");
                            }
                            let base = tag.clone();
                            c.insert(key.clone(), tag);
                            let mut child = p.clone();
                            child.push(Seg::Key(key.clone()));
                            added.push(Mark {
                                kind: Kind::Fresh,
                                path: child,
                                base: Some((key, base)),
                            });
                        }
                    }
                    Some(Tag::List(l)) => {
                        for (_, tag) in items {
                            let base = tag.clone();
                            let failed = l.push(tag).is_err();
                            skipped += failed as usize;
                            if !failed {
                                let mut child = p.clone();
                                child.push(Seg::Index(l.len() - 1));
                                added.push(Mark {
                                    kind: Kind::Fresh,
                                    path: child,
                                    base: Some(((l.len() - 1).to_string(), base)),
                                });
                            }
                        }
                        if skipped == self.clipboard.len() {
                            self.status = "The copied tags do not fit this list".into();
                            return;
                        }
                    }
                    _ => return,
                }
                if skipped > 0 {
                    self.status = format!("{skipped} tags did not fit this list");
                } else {
                    let n = added.len();
                    self.announce(if n == 1 { "Pasted a tag".into() } else { format!("Pasted {n} tags") });
                }
            }
        }
        if let Some((parent, a, b)) = swapped {
            self.swap_marks(&parent, a, b);
        }
        self.marks.list.extend(added);
        if let Some(rename) = name_it {
            self.rename = Some(rename);
        }
        // Paths shift after an edit, so a multi-selection does not outlive it.
        self.marked = self.selected.iter().cloned().collect();
        self.push_undo(before);
    }

    fn finish_rename(&mut self, path: TagPath, new_name: String) {
        let before = self.snap();
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
            let had = self.marks.list.iter().any(|m| m.path == path);
            let was = get_ref(&before.root.tag, &path).cloned();
            self.rename_marks(&path, &new_name);
            let mut now = path[..path.len() - 1].to_vec();
            now.push(Seg::Key(new_name.clone()));
            if had {
                self.drop_if_reverted(&now);
            } else if let (Some(was), false) = (was, self.under_gone(&now)) {
                if let Some(Seg::Key(old)) = path.last() {
                    self.marks.list.push(Mark {
                        kind: Kind::Edited,
                        path: now,
                        base: Some((old.clone(), was)),
                    });
                }
            }
            self.announce(format!("Renamed '{}' to '{}'", old, new_name));
            self.push_undo(before);
        }
    }

    /// Which dimension a map shows (0 Overworld, 1 Nether, 2 The End), if it says.
    fn map_dimension(&self, p: &TagPath) -> Option<usize> {
        match get_ref(&self.root.tag, &key_beside(p, "dimension"))? {
            Tag::String(s) => Some(match s.as_str() {
                "minecraft:the_nether" => 1,
                "minecraft:the_end" => 2,
                _ => 0,
            }),
            Tag::Byte(v) => Some(dim_index(*v as i32)),
            Tag::Int(v) => Some(dim_index(*v)),
            _ => None,
        }
    }

    fn set_map_dimension(&mut self, p: &TagPath, i: usize) {
        let at = key_beside(p, "dimension");
        self.frame_start = Some(self.snap());
        self.apply(Action::Snapshot);
        let names = ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"];
        let ids = [0, -1, 1];
        match get_mut(&mut self.root.tag, &at) {
            Some(Tag::String(s)) => *s = names[i].to_string(),
            Some(Tag::Byte(v)) => *v = ids[i] as i8,
            Some(Tag::Int(v)) => *v = ids[i],
            _ => {}
        }
        self.apply(Action::ChangedAt(at));
    }

    /// Opens the map picture editor on this document's map colors; false when it has none.
    /// For a player's file or record: its inventory opened as the inventory screen. False when it
    /// has none.
    /// Changes the document's root (a player's file) as its menu would: one step to undo.
    pub fn edit_root(&mut self, e: crate::mob_edit::MobEdit) {
        self.frame_start = Some(self.snap());
        self.apply(Action::EditMob(Vec::new(), e));
    }

    pub fn open_player_inventory(&mut self) -> bool {
        if !matches!(self.root.tag.get("Inventory"), Some(Tag::List(_))) {
            return false;
        }
        self.apply(Action::OpenInventory(vec![Seg::Key("Inventory".into())]));
        true
    }

    pub fn open_map_editor(&mut self) -> bool {
        fn find(t: &Tag, at: &mut TagPath) -> bool {
            let Tag::Compound(c) = t else { return false };
            for (k, v) in c {
                at.push(Seg::Key(k.clone()));
                if k == "colors" && matches!(v, Tag::ByteArray(a) if a.len() == 128 * 128) {
                    return true;
                }
                if find(v, at) {
                    return true;
                }
                at.pop();
            }
            false
        }
        let mut at = TagPath::new();
        if !find(&self.root.tag, &mut at) {
            return false;
        }
        self.apply_open(Action::OpenPicture(at, array_view::Picture::Map));
        true
    }

    fn apply_open(&mut self, a: Action) {
        self.frame_start = Some(self.snap());
        self.apply(a);
    }

    /// The bytes of an array tag, big-endian as they are stored, and the size of one value.
    fn array_bytes(&self, p: &TagPath) -> Option<(Vec<u8>, usize)> {
        match get_ref(&self.root.tag, p)? {
            Tag::ByteArray(a) => Some((a.iter().map(|b| *b as u8).collect(), 1)),
            Tag::IntArray(a) => Some((a.iter().flat_map(|v| v.to_be_bytes()).collect(), 4)),
            Tag::LongArray(a) => Some((a.iter().flat_map(|v| v.to_be_bytes()).collect(), 8)),
            _ => None,
        }
    }

    /// One of the array's pictures, in the area under the hex editor's toolbar.
    fn picture_ui(&mut self, ui: &mut Ui, area: Rect, p: &TagPath, name: &str, pic: array_view::Picture, locked: bool) {
        let key = path_key(p);
        let typing = Cell::new(false);
        let panel = array_view::Panel {
            key: &key,
            name,
            indent: 12.0,
            width: area.width(),
            read_only: locked,
            typing: &typing,
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
        let mut out = array_view::Outcome::default();
        egui::ScrollArea::vertical().auto_shrink(false).id_salt("array-picture").show(&mut child, |ui| {
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.add_space(12.0);
                ui.vertical(|ui| match get_mut(&mut self.root.tag, p) {
                    Some(Tag::ByteArray(a)) => out = array_view::show_picture(ui, &panel, a, pic),
                    Some(Tag::IntArray(a)) => out = array_view::show_picture(ui, &panel, a, pic),
                    Some(Tag::LongArray(a)) => out = array_view::show_picture(ui, &panel, a, pic),
                    _ => {}
                });
            });
        });
        // Edits to the array are not the hex editor's, so its bytes are read again afterwards.
        let before = self.snap();
        self.frame_start = Some(before);
        if out.snapshot {
            self.apply(Action::Snapshot);
        }
        if out.changed {
            self.apply(Action::ChangedAt(p.clone()));
        }
        if let Some(note) = out.note {
            self.status = note;
        }
    }

    /// Writes edited bytes back into the array tag.
    fn set_array_bytes(&mut self, p: &TagPath, bytes: &[u8]) {
        match get_mut(&mut self.root.tag, p) {
            Some(Tag::ByteArray(a)) => *a = bytes.iter().map(|b| *b as i8).collect(),
            Some(Tag::IntArray(a)) => {
                *a = bytes.chunks_exact(4).map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect()
            }
            Some(Tag::LongArray(a)) => {
                *a = bytes
                    .chunks_exact(8)
                    .map(|c| i64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
                    .collect()
            }
            _ => {}
        }
    }

    /// The hex editor, in place of the toolbar and the tags.
    /// The stacks of an inventory list as the screen shows them (deleted ones are not there).
    fn inventory_slots(&self, list: &TagPath) -> Vec<crate::inventory::SlotInfo> {
        let mut out: Vec<crate::inventory::SlotInfo> = Vec::new();
        for (which, lp) in self.inventory_lists(list) {
          let Some(Tag::List(l)) = get_ref(&self.root.tag, &lp) else { continue };
          for (i, t) in l.iter().enumerate() {
            // Which list it is in goes in the high bits of the index (see `stack_at`).
            let index = (usize::from(which) << 16) | i;
            let mut at = lp.clone();
            at.push(Seg::Index(i));
            let Some(c) = t.as_compound() else { continue };
            if self.under_gone(&at) {
                continue;
            }
            let Some((id, count)) = item_stack(c) else { continue };
            // Bedrock keeps a stack with no name and no count in every empty slot.
            if is_bedrock_item(c) && (id.is_empty() || count == Some(0)) {
                continue;
            }
            let Some(slot) = Self::slot_of_elem(which, i, c) else { continue };
            if out.iter().any(|s| s.slot == slot) {
                continue;
            }
            let count_tag = c.get("count").or_else(|| c.get("Count"));
            let number = match c.get("id") {
                Some(Tag::Short(n)) => Some(i64::from(*n)),
                Some(Tag::Int(n)) => Some(i64::from(*n)),
                _ => None,
            };
            let durability = crate::inventory::durability(&id);
            out.push(crate::inventory::SlotInfo {
                slot,
                index,
                damage: durability.and(damage_of(c).map(|(d, _)| d)),
                durability,
                enchants: crate::enchant::read(c),
                unbreakable: crate::naming::is_unbreakable(c),
                stack: c.clone(),
                id,
                number,
                count: count.unwrap_or(1),
                max_count: if matches!(count_tag, Some(Tag::Byte(_))) { 127 } else { 1000 },
            });
          }
        }
        out
    }

    /// A Bedrock player keeps its armor and offhand in lists beside `Inventory`.
    fn side_path(list: &TagPath, which: u8, partner: Option<&TagPath>) -> TagPath {
        if which == 0 {
            return list.clone();
        }
        if which == 3 {
            return partner.cloned().unwrap_or_else(|| list.clone());
        }
        let mut p = list.clone();
        p.pop();
        p.push(Seg::Key(if which == 1 { "Armor" } else { "Offhand" }.to_string()));
        p
    }

    /// The lists the inventory screen shows: this one, and for Bedrock's `Inventory` the armor
    /// (1) and offhand (2) lists too.
    fn inventory_lists(&self, list: &TagPath) -> Vec<(u8, TagPath)> {
        let mut out = vec![(0u8, list.clone())];
        if matches!(list.last(), Some(Seg::Key(k)) if k == "Inventory") {
            for which in [1u8, 2] {
                let p = Self::side_path(list, which, None);
                if matches!(get_ref(&self.root.tag, &p), Some(Tag::List(_))) {
                    out.push((which, p));
                }
            }
        }
        if let Some(p) = &self.inventory_partner {
            out.push((3, p.clone()));
        }
        out
    }

    /// The slot a stack in list `which` at place `i` is in: armor and offhand by place.
    fn slot_of_elem(which: u8, i: usize, c: &Compound) -> Option<i8> {
        match which {
            1 => (i < 4).then(|| 103 - i as i8),
            2 => Some(-106),
            // The second chest of a large chest: its slots come after the first's 27.
            3 => c.get("Slot").and_then(Tag::as_i64).map(|s| s as i8 + 27),
            _ => c.get("Slot").and_then(Tag::as_i64).map(|s| s as i8),
        }
    }

    /// What the inventory screen for this list looks like: the player's, the ender chest's, or
    /// the container (chest, hopper ...) whose `Items` it is.
    fn container_kind(&self, list: &TagPath) -> crate::inventory::Kind {
        use crate::inventory::Kind;
        match list.last() {
            Some(Seg::Key(k)) if k == "EnderItems" || k == "EnderChestInventory" => Kind::Ender,
            Some(Seg::Key(k)) if k == "Items" => {
                let id = get_ref(&self.root.tag, &list[..list.len() - 1]).and_then(|t| t.get("id")).and_then(Tag::as_str).unwrap_or("");
                Kind::of_container(id).unwrap_or(Kind::Chest)
            }
            _ => Kind::Player,
        }
    }

    /// The type of the chest block at a place in this chunk (`single`, `left`, `right`), read
    /// from the chunk's own block states by the Java chunk reader (every era's sections and
    /// packing, as the world view reads them); the chunk read once per edit.
    fn chest_type_at(&self, x: i64, y: i64, z: i64) -> Option<String> {
        let chunk = {
            let mut cache = self.decoded.borrow_mut();
            if cache.as_ref().map(|(g, _)| *g) != Some(self.generation) {
                let read = self.root.tag.get("sections").or_else(|| self.root.tag.get("Level")).is_some().then(|| mctk_java::Chunk::from_nbt_read_only(self.root.clone()).ok().map(Arc::new)).flatten();
                *cache = Some((self.generation, read));
            }
            cache.as_ref().and_then(|(_, c)| c.clone())
        };
        if let Some(chunk) = chunk {
            let s = chunk.sections.iter().find(|s| i64::from(s.y) == y >> 4)?;
            if let Some(mctk_java::BlockStorage::Palette(p)) = &s.blocks {
                let b = p.get(mctk_java::block_index((x & 15) as usize, (y & 15) as usize, (z & 15) as usize));
                return Some(b.properties.get("type").cloned().unwrap_or_else(|| "single".into()));
            }
        }
        self.chest_type_raw(x, y, z)
    }

    /// As [`Self::chest_type_at`], read straight from the tags (for a chunk the reader takes not).
    fn chest_type_raw(&self, x: i64, y: i64, z: i64) -> Option<String> {
        let root = self.root.tag.as_compound()?;
        let sections = root.get("sections").or_else(|| root.get("Level").and_then(|l| l.get("Sections")))?.as_list()?;
        for s in sections.iter().filter_map(Tag::as_compound) {
            if s.get("Y").and_then(Tag::as_i64) != Some(y >> 4) {
                continue;
            }
            let (palette, data) = match s.get("block_states") {
                Some(b) => (b.get("palette")?, b.get("data")),
                None => (s.get("Palette")?, s.get("BlockStates")),
            };
            let palette = palette.as_list()?;
            let index = ((y & 15) << 8 | (z & 15) << 4 | (x & 15)) as usize;
            // Before 1.16 (data version 2526) an entry could run on into the next long.
            let spanning = root.get("DataVersion").and_then(Tag::as_i64).is_some_and(|v| v < 2526);
            let entry = match data {
                Some(Tag::LongArray(longs)) if palette.len() > 1 && !longs.is_empty() => {
                    // How wide an entry is, from how many longs hold the 4096 (as the game reads
                    // it): the width the palette's size gives when that fits, else what the
                    // length says.
                    let packed = |b: usize| if spanning { (4096 * b).div_ceil(64) } else { 4096usize.div_ceil(64 / b) };
                    let from_palette = (usize::BITS - (palette.len() - 1).leading_zeros()).max(4) as usize;
                    let bits = if packed(from_palette) == longs.len() { from_palette } else { (1..=16).find(|b| packed(*b) == longs.len())? };
                    let mask = (1u64 << bits) - 1;
                    if spanning {
                        let bit = index * bits;
                        let (word, off) = (bit / 64, bit % 64);
                        let mut v = (*longs.get(word)? as u64) >> off;
                        if off + bits > 64 {
                            v |= (*longs.get(word + 1)? as u64) << (64 - off);
                        }
                        (v & mask) as usize
                    } else {
                        let per_long = 64 / bits;
                        let word = *longs.get(index / per_long)? as u64;
                        ((word >> ((index % per_long) * bits)) & mask) as usize
                    }
                }
                _ => 0,
            };
            let state = palette.get(entry)?.as_compound()?;
            return Some(state.get("Properties").and_then(|p| p.get("type")).and_then(Tag::as_str).unwrap_or("single").to_string());
        }
        None
    }

    /// Every chest in the document's block entity lists that is half of a large chest, by its
    /// path: whether it is the right half (whose slots come first) and the place in the list of
    /// the other half.
    pub(crate) fn find_large_chests(&self) -> HashMap<TagPath, (bool, usize)> {
        let mut out = HashMap::new();
        let lists: [TagPath; 3] = [
            vec![Seg::Key("block_entities".into())],
            vec![Seg::Key("BlockEntities".into())],
            vec![Seg::Key("Level".into()), Seg::Key("TileEntities".into())],
        ];
        for list in lists {
            let Some(Tag::List(l)) = get_ref(&self.root.tag, &list) else { continue };
            for i in 0..l.len() {
                let mut chest = list.clone();
                chest.push(Seg::Index(i));
                if out.contains_key(&chest) {
                    continue;
                }
                let mut items = chest.clone();
                items.push(Seg::Key("Items".into()));
                let Some((first, second)) = self.chest_pair(&items) else { continue };
                let place = |p: &TagPath| match p.get(p.len().wrapping_sub(2)) {
                    Some(Seg::Index(k)) => Some(*k),
                    _ => None,
                };
                let (Some(a), Some(b)) = (place(&first), place(&second)) else { continue };
                let at = |k: usize| {
                    let mut p = list.clone();
                    p.push(Seg::Index(k));
                    p
                };
                out.insert(at(a), (true, b));
                out.insert(at(b), (false, a));
            }
        }
        out
    }

    /// For the `Items` of a chest that is half of a large chest: this large chest's two `Items`
    /// lists, the one whose slots come first (the right half) and then the other.
    fn chest_pair(&self, items: &TagPath) -> Option<(TagPath, TagPath)> {
        if !matches!(items.last(), Some(Seg::Key(k)) if k == "Items") || items.len() < 3 {
            return None;
        }
        let me_path = &items[..items.len() - 1];
        let Some(Seg::Index(me_index)) = me_path.last().cloned() else { return None };
        let list_path = &me_path[..me_path.len() - 1];
        let me = get_ref(&self.root.tag, me_path)?.as_compound()?;
        if crate::inventory::Kind::of_container(me.get("id")?.as_str()?) != Some(crate::inventory::Kind::Chest) || !me.get("id")?.as_str()?.to_ascii_lowercase().contains("chest") {
            return None;
        }
        let Some(Tag::List(siblings)) = get_ref(&self.root.tag, list_path) else { return None };
        let num = |c: &Compound, k: &str| c.get(k).and_then(Tag::as_i64);
        let make = |i: usize| {
            let mut p = list_path.to_vec();
            p.push(Seg::Index(i));
            p.push(Seg::Key("Items".into()));
            p
        };
        // Bedrock says which chest is the other half, and which leads.
        if let (Some(px), Some(pz)) = (num(me, "pairx"), num(me, "pairz")) {
            let other = siblings.iter().enumerate().find(|(i, t)| {
                *i != me_index && t.as_compound().is_some_and(|c| num(c, "x") == Some(px) && num(c, "z") == Some(pz) && num(c, "y") == num(me, "y"))
            })?;
            let lead = num(me, "pairlead").unwrap_or(0) != 0;
            return Some(if lead { (make(me_index), make(other.0)) } else { (make(other.0), make(me_index)) });
        }
        // Java: the neighbor in the same row of blocks, and the chunk's block states say which half.
        let (x, y, z) = (num(me, "x")?, num(me, "y")?, num(me, "z")?);
        let mine = self.chest_type_at(x, y, z)?;
        if mine != "left" && mine != "right" {
            return None;
        }
        // The chest beside it that is the other half (not just any chest beside it: a lone
        // chest, or another large chest, can stand there too).
        let want = if mine == "left" { "right" } else { "left" };
        let other = siblings.iter().enumerate().find(|(i, t)| {
            *i != me_index
                && t.as_compound().is_some_and(|c| {
                    let near = (num(c, "x").zip(num(c, "z"))).is_some_and(|(cx, cz)| (cx - x).abs() + (cz - z).abs() == 1);
                    near && num(c, "y") == Some(y)
                        && c.get("id") == me.get("id")
                        && num(c, "x").zip(num(c, "z")).and_then(|(cx, cz)| self.chest_type_at(cx, y, cz)).as_deref() == Some(want)
                })
        })?;
        // The right half is the first.
        Some(if mine == "right" { (make(me_index), make(other.0)) } else { (make(other.0), make(me_index)) })
    }

    /// A Bedrock list with a stack for an empty slot in it.
    fn has_empties(&self, list: &TagPath) -> bool {
        matches!(get_ref(&self.root.tag, list), Some(Tag::List(l)) if l.iter().filter_map(Tag::as_compound).any(|c| is_bedrock_item(c) && matches!(c.get("Name"), Some(Tag::String(s)) if s.is_empty())))
    }

    fn is_bedrock_list(&self, list: &TagPath) -> bool {
        matches!(get_ref(&self.root.tag, list), Some(Tag::List(l)) if l.iter().filter_map(Tag::as_compound).any(is_bedrock_item))
    }

    /// Where the compound for a slot is, empty or not (Bedrock has one for every slot).
    fn bedrock_slot_path(&self, list: &TagPath, slot: i8) -> Option<TagPath> {
        for (which, lp) in self.inventory_lists(list) {
            let Some(Tag::List(l)) = get_ref(&self.root.tag, &lp) else { continue };
            for (i, t) in l.iter().enumerate() {
                if t.as_compound().is_some_and(|c| Self::slot_of_elem(which, i, c) == Some(slot)) {
                    let mut p = lp.clone();
                    p.push(Seg::Index(i));
                    return Some(p);
                }
            }
        }
        None
    }

    /// What a Bedrock slot holds when it holds nothing (keeping its `Slot`).
    fn bedrock_empty(old: &Compound) -> Compound {
        let mut c = Compound::new();
        c.insert("Name".into(), Tag::String(String::new()));
        c.insert("Count".into(), Tag::Byte(0));
        c.insert("Damage".into(), Tag::Short(0));
        c.insert("WasPickedUp".into(), Tag::Byte(0));
        if let Some(s) = old.get("Slot") {
            c.insert("Slot".into(), s.clone());
        }
        c
    }

    /// Trades what two Bedrock slots hold; each keeps its own `Slot`.
    fn bedrock_swap(&mut self, list: &TagPath, a: i8, b: i8) {
        let (Some(pa), Some(pb)) = (self.bedrock_slot_path(list, a), self.bedrock_slot_path(list, b)) else { return };
        let (Some(ca), Some(cb)) = (
            get_ref(&self.root.tag, &pa).and_then(Tag::as_compound).cloned(),
            get_ref(&self.root.tag, &pb).and_then(Tag::as_compound).cloned(),
        ) else {
            return;
        };
        let keep = |mut content: Compound, own: &Compound| {
            match own.get("Slot") {
                Some(s) => {
                    content.insert("Slot".into(), s.clone());
                }
                None => {
                    content.shift_remove("Slot");
                }
            }
            Tag::Compound(content)
        };
        let (na, nb) = (keep(cb.clone(), &ca), keep(ca.clone(), &cb));
        self.apply(Action::SetValues(vec![(pa, na), (pb, nb)], "Moved an item".into()));
    }

    /// Draws the trades screen, if one is open, and applies what the user does in it.
    fn trades_screen(&mut self, ui: &mut Ui) {
        let Some(path) = self.trades.clone() else { return };
        let Some(c) = get_ref(&self.root.tag, &path).and_then(Tag::as_compound) else {
            self.trades = None;
            return;
        };
        let version = self.root.tag.get("DataVersion").and_then(Tag::as_i64);
        let list = crate::mob_edit::trades(c, version);
        let who = mctk_convert::view::java_entity_id(c).map_or_else(|| "Villager".to_string(), |id| item_name(&id));
        let title = match crate::naming::mob_name(c) {
            Some(n) if !n.is_empty() => format!("Trades \u{2014} {n} ({who})"),
            _ => format!("Trades \u{2014} {who}"),
        };
        let (edits, open) = crate::trades::show(ui, &list, &title, version);
        self.frame_start = Some(self.snap());
        for e in edits {
            self.apply(Action::EditMob(path.clone(), crate::mob_edit::MobEdit::Trades(e)));
        }
        if !open {
            self.trades = None;
        }
    }

    /// Draws the inventory screen, if one is open, and applies what the user does in it.
    fn inventory_screen(&mut self, ui: &mut Ui) {
        let Some((list, mut state)) = self.inventory.take() else { return };
        let slots = self.inventory_slots(&list);
        let name = match list.last() {
            Some(Seg::Key(k)) => k.clone(),
            _ => String::new(),
        };
        let kind = if self.inventory_partner.is_some() {
            crate::inventory::Kind::DoubleChest
        } else {
            self.container_kind(&list)
        };
        let numeric = matches!(get_ref(&self.root.tag, &list), Some(Tag::List(l)) if l.iter().any(|t| matches!(t.get("id"), Some(Tag::Short(_) | Tag::Int(_)))));
        let _ = name;
        let (edits, open) = crate::inventory::show(ui, &mut state, &slots, kind.title(), kind, numeric, self.root.tag.get("DataVersion").and_then(Tag::as_i64));
        for edit in edits {
            self.frame_start = Some(self.snap());
            self.inventory_edit(&list, &slots, edit, &mut state);
        }
        if open {
            self.inventory = Some((list, state));
        } else {
            self.inventory_partner = None;
        }
    }

    /// Does one thing the inventory screen asked for.
    fn inventory_edit(&mut self, list: &TagPath, slots: &[crate::inventory::SlotInfo], edit: crate::inventory::Edit, state: &mut crate::inventory::State) {
        use crate::inventory::Edit;
        // The high bits of an index say which list (see `inventory_slots`).
        let partner = self.inventory_partner.clone();
        let stack_at = |index: usize| {
            let mut p = Self::side_path(list, (index >> 16) as u8, partner.as_ref());
            p.push(Seg::Index(index & 0xFFFF));
            p
        };
        // Bedrock's player has a stack (an empty one) in every slot; a chest has only the full slots.
        let bedrock = self.is_bedrock_list(list) && self.has_empties(list);
        let double = partner.is_some();
        let half = |slot: i8| if double && slot >= 27 { 3u8 } else { 0 };
        let local = |slot: i8| i64::from(if double && slot >= 27 { slot - 27 } else { slot });
        let target = |h: u8| if h == 3 { partner.clone().unwrap_or_else(|| list.clone()) } else { list.clone() };
        let beside = |index: usize, key: &str| {
            let mut p = stack_at(index);
            p.push(Seg::Key(key.to_string()));
            p
        };
        // A number written as the tag it replaces is written: a byte stays a byte.
        let same_type = |p: &TagPath, v: i64, root: &Tag| match get_ref(root, p) {
            Some(Tag::Byte(_)) => Tag::Byte(v as i8),
            Some(Tag::Short(_)) => Tag::Short(v as i16),
            Some(Tag::Long(_)) => Tag::Long(v),
            _ => Tag::Int(v as i32),
        };
        let slot_of = |slot: i8| slots.iter().find(|s| s.slot == slot);
        match edit {
            Edit::Move { from, to } if bedrock => self.bedrock_swap(list, from, to),
            Edit::Swap(a, b) if bedrock => self.bedrock_swap(list, a, b),
            Edit::Delete(index) if bedrock => {
                let path = stack_at(index);
                let Some(old) = get_ref(&self.root.tag, &path).and_then(Tag::as_compound).cloned() else { return };
                self.apply(Action::SetValues(vec![(path, Tag::Compound(Self::bedrock_empty(&old)))], "Removed an item".into()));
            }
            Edit::Add { slot, id, count } if bedrock && self.bedrock_slot_path(list, slot).is_some() => {
                let Some(path) = self.bedrock_slot_path(list, slot) else { return };
                let Some(mut c) = get_ref(&self.root.tag, &path).and_then(Tag::as_compound).cloned() else { return };
                c.insert("Name".into(), Tag::String(id));
                c.insert("Count".into(), Tag::Byte(count.clamp(1, 127) as i8));
                c.insert("Damage".into(), Tag::Short(0));
                c.entry("WasPickedUp".to_string()).or_insert(Tag::Byte(0));
                self.apply(Action::SetValues(vec![(path, Tag::Compound(c))], "Added an item".into()));
                state.selected = Some(slot);
            }
            Edit::Move { from, to } => {
                let Some(s) = slot_of(from) else { return };
                let p = beside(s.index, "Slot");
                let v = same_type(&p, local(to), &self.root.tag);
                if half(from) == half(to) {
                    self.apply(Action::SetValues(vec![(p, v)], "Moved an item".into()));
                } else {
                    // Into the other chest: the stack goes to that list.
                    let Some(mut c) = get_ref(&self.root.tag, &stack_at(s.index)).and_then(Tag::as_compound).cloned() else { return };
                    c.insert("Slot".into(), v);
                    self.apply(Action::Delete(stack_at(s.index)));
                    self.apply(Action::AddStack(target(half(to)), c));
                }
            }
            Edit::Swap(a, b) => {
                let (Some(x), Some(y)) = (slot_of(a), slot_of(b)) else { return };
                let (px, py) = (beside(x.index, "Slot"), beside(y.index, "Slot"));
                let (vx, vy) = (same_type(&px, local(b), &self.root.tag), same_type(&py, local(a), &self.root.tag));
                if half(a) == half(b) {
                    self.apply(Action::SetValues(vec![(px, vx), (py, vy)], "Swapped two items".into()));
                } else {
                    let (Some(mut cx), Some(mut cy)) = (
                        get_ref(&self.root.tag, &stack_at(x.index)).and_then(Tag::as_compound).cloned(),
                        get_ref(&self.root.tag, &stack_at(y.index)).and_then(Tag::as_compound).cloned(),
                    ) else {
                        return;
                    };
                    cx.insert("Slot".into(), vx);
                    cy.insert("Slot".into(), vy);
                    self.apply(Action::Delete(stack_at(x.index)));
                    self.apply(Action::Delete(stack_at(y.index)));
                    self.apply(Action::AddStack(target(half(b)), cx));
                    self.apply(Action::AddStack(target(half(a)), cy));
                }
            }
            Edit::Delete(index) => self.apply(Action::Delete(stack_at(index))),
            Edit::Change { index, id } => self.apply(Action::SetItem(stack_at(index), id)),
            Edit::SetCount { index, count } => {
                let c = get_ref(&self.root.tag, &stack_at(index)).and_then(Tag::as_compound);
                let key = if c.is_some_and(|c| c.contains_key("count")) { "count" } else { "Count" };
                let p = beside(index, key);
                let v = same_type(&p, count, &self.root.tag);
                self.apply(Action::SetValues(vec![(p, v)], format!("Set the count to {count}")));
            }
            Edit::Stack { index, edit } => {
                let p = stack_at(index);
                let Some(mut c) = get_ref(&self.root.tag, &p).and_then(Tag::as_compound).cloned() else { return };
                let version = self.root.tag.get("DataVersion").and_then(Tag::as_i64);
                edit.apply(&mut c, version);
                self.apply(Action::SetValues(vec![(p, Tag::Compound(c))], edit.note()));
            }
            Edit::SetDamage { index, damage } => {
                let Some((_, path)) = get_ref(&self.root.tag, &stack_at(index)).and_then(Tag::as_compound).and_then(damage_of) else { return };
                let mut p = stack_at(index);
                p.extend(path);
                let v = same_type(&p, damage, &self.root.tag);
                self.apply(Action::SetValues(vec![(p, v)], "Changed the durability".into()));
            }
            Edit::Enchant { index, id, level } => {
                let path = stack_at(index);
                let Some(mut c) = get_ref(&self.root.tag, &path).and_then(Tag::as_compound).cloned() else { return };
                let version = self.root.tag.get("DataVersion").and_then(Tag::as_i64);
                crate::enchant::set(&mut c, &id, level, version);
                let note = match level {
                    Some(l) => format!("Gave {} {}", crate::enchant::name(&id), crate::enchant::level_text(l)),
                    None => format!("Removed {}", crate::enchant::name(&id)),
                };
                self.apply(Action::SetValues(vec![(path, Tag::Compound(c))], note));
            }
            Edit::Add { slot, id, count } => {
                let into = target(half(slot));
                let Some(stack) = self.new_stack(&into, local(slot) as i8, &id) else { return };
                // As many as asked, in the stack's own count.
                let stack = if count > 1 { crate::mob_edit::with_count(&stack, count) } else { stack };
                self.apply(Action::AddStack(into, stack));
                state.selected = Some(slot);
            }
        }
    }

    /// A new item stack for the list, written like the stacks already in it (or like a world of
    /// this version when there are none): a number for an id in console and old worlds, `count`
    /// as an int from 1.20.5 on, `Count` as a byte before.
    fn new_stack(&mut self, list: &TagPath, slot: i8, id: &str) -> Option<Compound> {
        let Some(Tag::List(l)) = get_ref(&self.root.tag, list) else { return None };
        let existing = l.iter().filter_map(Tag::as_compound);
        if existing.clone().any(is_bedrock_item) {
            let mut c = Compound::new();
            c.insert("Name".into(), Tag::String(id.to_string()));
            c.insert("Count".into(), Tag::Byte(1));
            c.insert("Damage".into(), Tag::Short(0));
            c.insert("WasPickedUp".into(), Tag::Byte(0));
            c.insert("Slot".into(), Tag::Byte(slot));
            return Some(c);
        }
        let numeric = existing.clone().find_map(|c| match c.get("id") {
            Some(Tag::Short(_)) => Some(false),
            Some(Tag::Int(_)) => Some(true),
            _ => None,
        });
        let modern = existing.clone().any(|c| c.contains_key("count"))
            || (l.is_empty() && self.root.tag.get("DataVersion").and_then(Tag::as_i64).is_some_and(|v| v >= 3837));
        let mut c = Compound::new();
        match numeric {
            Some(wide) => {
                let Some((n, damage)) = mctk_mapping::legacy::legacy().numeric_item(id) else {
                    self.status = "That item has no number in this kind of world".into();
                    return None;
                };
                c.insert("id".into(), if wide { Tag::Int(i32::from(n)) } else { Tag::Short(n as i16) });
                c.insert("Damage".into(), Tag::Short(damage as i16));
            }
            None => {
                c.insert("id".into(), Tag::String(id.to_string()));
            }
        }
        if modern {
            c.insert("count".into(), Tag::Int(1));
        } else {
            c.insert("Count".into(), Tag::Byte(1));
        }
        c.insert("Slot".into(), Tag::Byte(slot));
        Some(c)
    }

    fn hex_ui(&mut self, ui: &mut Ui) {
        let Some(mut h) = self.hex.take() else { return };
        // Undo, Redo or anything else that changed the tag since: read its bytes again.
        if h.seen != self.generation {
            match self.array_bytes(&h.path) {
                Some((b, _)) => h.ed.set_bytes(b),
                None => return,
            }
            h.seen = self.generation;
        }
        let name = seg_name(&h.path);
        let locked = self.read_only || self.under_gone(&h.path);
        // A picture of the array (opened from its menu) has a bar of its own and no hex dump.
        let out = if let Some(pic) = h.picture {
            let full = ui.max_rect();
            // The map's bar is taller: it holds the dimension buttons.
            let bar_h = if pic == array_view::Picture::Map { 56.0 } else { 40.0 };
            let bar = Rect::from_min_size(full.min, vec2(full.width(), bar_h));
            ui.painter().rect_filled(bar, 0.0, theme::bg());
            ui.painter().line_segment(
                [bar.left_bottom(), bar.right_bottom()],
                Stroke::new(1.0_f32, theme::border()),
            );
            let mut right = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(bar.shrink2(vec2(8.0, 5.0)))
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            let mut left = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(bar.shrink2(vec2(10.0, 5.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            left.spacing_mut().item_spacing.x = 8.0;
            let mut out = crate::hex_editor::Outcome::default();
            if widgets::square_icon_button(&mut right, Icon::Close, "Back to the tags").clicked() {
                out.close = true;
            }
            right.spacing_mut().item_spacing.x = 8.0;
            // A map says which dimension it shows; pick it as in the chunk locator.
            if let Some(cur) = self.map_dimension(&h.path).filter(|_| pic == array_view::Picture::Map) {
                if let Some(i) = widgets::dimension_buttons(&mut right, cur, !locked) {
                    self.set_map_dimension(&h.path, i);
                }
            }
            if pic == array_view::Picture::Map {
                let key = path_key(&h.path);
                let typing = Cell::new(false);
                let panel = array_view::Panel {
                    key: &key,
                    name: &name,
                    indent: 12.0,
                    width: full.width(),
                    read_only: locked,
                    typing: &typing,
                };
                let blank = matches!(get_ref(&self.root.tag, &h.path), Some(Tag::ByteArray(a)) if a.iter().all(|b| *b == 0));
                array_view::map_bar(&mut left, &panel, blank);
            }
            let body = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
            self.picture_ui(ui, body, &h.path, &name, pic, locked);
            out
        } else {
            h.ed.ui(ui, &name, locked, &["Hex"], &mut 0)
        };
        if let Some(note) = out.note {
            self.status = note;
        }
        if out.changed {
            let now = ui.input(|i| i.time);
            let before = self.snap();
            self.frame_start = Some(before.clone());
            // A burst of typing is one step to undo.
            if now - h.last_change > 1.0 {
                self.edit_note = format!("Edited '{name}'");
                self.apply(Action::Snapshot);
                self.status = format!("Edited '{name}'");
            }
            h.last_change = now;
            self.set_array_bytes(&h.path, h.ed.bytes());
            self.frame_start = Some(before);
            self.apply(Action::ChangedAt(h.path.clone()));
            h.seen = self.generation;
        }
        if !out.close {
            self.hex = Some(h);
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, full_precision: bool) {
        if self.hex.is_some() {
            self.hex_ui(ui);
            return;
        }
        if self.inventory.is_some() {
            self.inventory_screen(ui);
            return;
        }
        if self.trades.is_some() {
            self.trades_screen(ui);
            return;
        }
        self.toolbar(ui);
        if let Some((path, mut name)) = self.rename.take() {
            let (mut keep, mut done) = (true, false);
            egui::Window::new("Rename Tag")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    let field = Id::new("rename-field");
                    let mut edit = egui::TextEdit::singleline(&mut name).id(field).show(ui);
                    edit.response.request_focus();
                    // The old name starts out selected, so typing replaces it.
                    let seen = Id::new("rename-seen");
                    let key = path_key(&path);
                    if ui.data(|d| d.get_temp::<String>(seen)).as_deref() != Some(key.as_str()) {
                        ui.data_mut(|d| d.insert_temp(seen, key));
                        let len = name.chars().count();
                        edit.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(len),
                        )));
                        edit.state.store(ui.ctx(), field);
                        ui.ctx().request_repaint();
                    }
                    ui.horizontal(|ui| {
                        done = ui.button("Rename").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Enter));
                        keep = !ui.button("Cancel").clicked();
                    });
                });
            if done || !keep {
                ui.data_mut(|d| d.remove::<String>(Id::new("rename-seen")));
            }
            if done {
                self.finish_rename(path, name);
            } else if keep {
                self.rename = Some((path, name));
            }
        }
        if !self.read_only
            && ui.input(|i| i.key_pressed(egui::Key::Delete))
            && self.rename.is_none()
            && !self.typing()
        {
            if let Some(p) = self.selected.clone().filter(|p| !p.is_empty()) {
                self.frame_start = Some(self.snap());
                self.apply(Action::Delete(p));
            }
        }
        self.shortcuts(ui);
        // Ctrl and the wheel over the tree scale it.
        let wheel = ui.input(|i| {
            let over = i.pointer.hover_pos().is_some_and(|p| ui.max_rect().contains(p));
            (over && i.modifiers.command).then_some(i.raw_scroll_delta.y + i.smooth_scroll_delta.y)
        });
        if let Some(d) = wheel.filter(|d| *d != 0.0) {
            set_tree_scale(ui.ctx(), (tree_scale(ui.ctx()) + d.signum() * 0.05).clamp(0.6, 2.5));
        }
        self.frame_start = Some(self.snap());
        let mut actions = Vec::new();
        let mut hits = 0;
        let find = self.find.clone();
        let selected = self.selected.clone();
        let marked: HashSet<String> = self.marked.iter().map(path_key).collect();
        let rows = RefCell::new(Vec::new());
        let top = Cell::new(Pos2::ZERO);
        let typing = Cell::new(false);
        let scroll_by = self.autoscroll;
        let expanded = self.expanded.clone();
        let read_only = self.read_only;
        let has_clipboard = !self.clipboard.is_empty();
        let marks = self.marks.list.clone();
        let undone = self.undone.clone();
        let stripe = Cell::new(0);
        let container = Cell::new(false);
        if self.chests.0 != Some(self.generation) {
            self.chests = (Some(self.generation), self.find_large_chests());
        }
        let chests = std::mem::take(&mut self.chests.1);
        let out = egui::ScrollArea::vertical()
            .auto_shrink(false)
            // Dragging selects rows; the wheel and scrollbar still scroll.
            .drag_to_scroll(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                top.set(ui.max_rect().min);
                if scroll_by != 0.0 {
                    ui.scroll_with_delta(vec2(0.0, scroll_by));
                }
                let cx = RowCtx {
                    find: &find,
                    selected: &selected,
                    marked: &marked,
                    rows: &rows,
                    typing: &typing,
                    expanded: &expanded,
                    full_precision,
                    read_only,
                    width: ui.available_width(),
                    styles: &self.styles,
                    has_clipboard,
                    deletable: self.deletable,
                    marks: &marks,
                    undone: &undone,
                    stripe: &stripe,
                    k: tree_scale(ui.ctx()) / ui.ctx().zoom_factor(),
                    container: &container,
                    chests: &chests,
                    data_version: self.root.tag.get("DataVersion").and_then(Tag::as_i64),
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
                    0,
                    &mut actions,
                    &mut hits,
                );
            });
        let rows = rows.into_inner();
        self.typing = typing.get();
        self.row_order = rows.iter().map(|r| r.path.clone()).collect();
        self.chests.1 = chests;
        self.drag_select(ui, out.inner_rect, top.get(), &rows);
        self.find_status = if find.is_empty() { String::new() } else { format!("{hits} matches") };
        for a in actions {
            self.apply(a);
        }
        if !self.find_status.is_empty() {
            let r = ui.max_rect();
            ui.painter().text(
                r.right_bottom() - vec2(12.0, 10.0),
                Align2::RIGHT_BOTTOM,
                &self.find_status,
                theme::regular(12.0),
                theme::text_muted(),
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
        t.frame_start = Some(t.snap());
        t.apply(Action::Move(vec![Seg::Key("b".into())], -1));
        assert_eq!(
            t.root.tag.as_compound().unwrap().keys().next().unwrap(),
            "b"
        );
        t.frame_start = Some(t.snap());
        t.apply(Action::Delete(vec![Seg::Key("b".into()), Seg::Index(0)]));
        // Deleted tags stay, marked, until the document is saved.
        assert_eq!(t.root.tag.get("b").unwrap().as_list().unwrap().len(), 2);
        assert_eq!(t.marks.list.iter().filter(|m| m.kind == Kind::Gone).count(), 1);
        t.purge_deleted();
        assert_eq!(t.root.tag.get("b").unwrap().as_list().unwrap().len(), 1);
        t.finish_rename(vec![Seg::Key("a".into())], "renamed".into());
        assert!(t.root.tag.get("renamed").is_some());
        assert_eq!(t.undo.len(), 3);
        assert!(t.dirty);
    }

    #[test]
    fn cuts_and_deletes_a_selection() {
        let list = List::from_tags(
            TagType::Int,
            vec![Tag::Int(1), Tag::Int(2), Tag::Int(3), Tag::Int(4)],
        )
        .unwrap();
        let mut c = Compound::new();
        c.insert("a".into(), Tag::Int(1));
        c.insert("b".into(), Tag::Int(2));
        c.insert("l".into(), Tag::List(list));
        let mut t = NbtTree::new(NamedTag {
            name: String::new(),
            tag: Tag::Compound(c),
        });
        let key = |k: &str| vec![Seg::Key(k.into())];
        let item = |i: usize| vec![Seg::Key("l".into()), Seg::Index(i)];
        t.row_order = vec![vec![], key("l"), item(0), item(1), item(2), item(3), key("a"), key("b")];
        t.marked = vec![item(0), item(2), key("a")];
        t.selected = Some(item(0));
        t.frame_start = Some(t.snap());
        t.apply(Action::Cut(item(0)));
        assert_eq!(t.marks.list.iter().filter(|m| m.kind == Kind::Gone).count(), 3);
        assert_eq!(t.clipboard.len(), 3);
        assert!(t.marked.is_empty());
        t.purge_deleted();
        let l = t.root.tag.get("l").unwrap().as_list().unwrap();
        assert_eq!(l.len(), 2);
        assert!(t.root.tag.get("a").is_none());
        // A tag outside the selection is acted on alone.
        t.marked = vec![key("b")];
        t.frame_start = Some(t.snap());
        t.apply(Action::Delete(item(0)));
        t.purge_deleted();
        assert!(t.root.tag.get("b").is_some());
        assert_eq!(t.root.tag.get("l").unwrap().as_list().unwrap().len(), 1);
    }

    #[test]
    fn an_item_stack_can_be_changed_to_another_item_and_undone() {
        let stack = |id: &str, count: (&str, Tag)| {
            let mut c = Compound::new();
            c.insert("id".into(), Tag::String(id.into()));
            c.insert(count.0.to_string(), count.1);
            c
        };
        let mut root = Compound::new();
        root.insert("item".into(), Tag::Compound(stack("minecraft:stone", ("Count", Tag::Byte(1)))));
        root.insert("not an item".into(), Tag::Compound(stack("minecraft:stone", ("other", Tag::Byte(1)))));
        let mut t = NbtTree::new(NamedTag { name: String::new(), tag: Tag::Compound(root) });
        let item = vec![Seg::Key("item".into())];
        assert_eq!(item_id_key(get_ref(&t.root.tag, &item).unwrap()), Some("id"));
        assert_eq!(item_id_key(get_ref(&t.root.tag, &[Seg::Key("not an item".into())]).unwrap()), None);

        t.frame_start = Some(t.snap());
        t.apply(Action::SetItem(item.clone(), "minecraft:diamond".into()));
        let id = |t: &NbtTree| get_ref(&t.root.tag, &item).and_then(|c| c.get("id")).and_then(Tag::as_str).map(str::to_string);
        assert_eq!(id(&t).as_deref(), Some("minecraft:diamond"));
        // The count stayed; the edit is one undo step.
        assert_eq!(get_ref(&t.root.tag, &item).and_then(|c| c.get("Count")), Some(&Tag::Byte(1)));
        t.undo();
        assert_eq!(id(&t).as_deref(), Some("minecraft:stone"));
        // The same item again changes nothing.
        t.frame_start = Some(t.snap());
        t.apply(Action::SetItem(item.clone(), "minecraft:stone".into()));
        assert!(t.undo.is_empty());
    }

    #[test]
    fn an_item_with_a_number_for_an_id_gets_the_number_of_the_new_item() {
        // As in the console player file: Count, Slot and Damage, and the id an int (a filled map).
        let mut c = Compound::new();
        c.insert("Count".into(), Tag::Byte(1));
        c.insert("Slot".into(), Tag::Byte(9));
        c.insert("Damage".into(), Tag::Short(3));
        c.insert("id".into(), Tag::Int(358));
        let mut root = Compound::new();
        root.insert("stack".into(), Tag::Compound(c));
        let mut t = NbtTree::new(NamedTag { name: String::new(), tag: Tag::Compound(root) });
        let stack = vec![Seg::Key("stack".into())];
        assert_eq!(item_id_key(get_ref(&t.root.tag, &stack).unwrap()), Some("id"));
        let (number, damage) = mctk_mapping::legacy::legacy().numeric_item("minecraft:diamond").expect("diamond has a number");
        t.frame_start = Some(t.snap());
        t.apply(Action::SetItem(stack.clone(), "minecraft:diamond".into()));
        let c = get_ref(&t.root.tag, &stack).unwrap();
        assert_eq!(c.get("id"), Some(&Tag::Int(i32::from(number))));
        assert_eq!(c.get("Damage"), Some(&Tag::Short(damage as i16)));
        assert_eq!(c.get("Slot"), Some(&Tag::Byte(9)));
        t.undo();
        let c = get_ref(&t.root.tag, &stack).unwrap();
        assert_eq!((c.get("id"), c.get("Damage")), (Some(&Tag::Int(358)), Some(&Tag::Short(3))));
    }

    /// A player-like root: an inventory holding a worn sword in slot 0 and 5 apples in slot 9.
    fn inventory_tree() -> NbtTree {
        let stack = |id: &str, count: i8, slot: i8, damage: Option<i16>| {
            let mut c = Compound::new();
            c.insert("id".into(), Tag::String(id.into()));
            c.insert("Count".into(), Tag::Byte(count));
            c.insert("Slot".into(), Tag::Byte(slot));
            if let Some(d) = damage {
                c.insert("Damage".into(), Tag::Short(d));
            }
            Tag::Compound(c)
        };
        let list = List::from_tags(
            TagType::Compound,
            vec![stack("minecraft:diamond_sword", 1, 0, Some(500)), stack("minecraft:apple", 5, 9, None)],
        )
        .unwrap();
        let mut root = Compound::new();
        root.insert("Inventory".into(), Tag::List(list));
        NbtTree::new(NamedTag { name: String::new(), tag: Tag::Compound(root) })
    }

    #[test]
    fn the_inventory_screen_edits_stacks_and_every_edit_undoes() {
        use crate::inventory::Edit;
        let mut t = inventory_tree();
        let list = vec![Seg::Key("Inventory".into())];
        let slot_of = |t: &NbtTree, i: usize| {
            get_ref(&t.root.tag, &[Seg::Key("Inventory".into()), Seg::Index(i), Seg::Key("Slot".into())]).and_then(Tag::as_i64)
        };
        let slots = t.inventory_slots(&list);
        assert_eq!(slots.len(), 2);
        assert_eq!((slots[0].slot, slots[0].damage, slots[0].durability), (0, Some(500), Some(1561)));
        assert_eq!((slots[1].count, slots[1].damage), (5, None));
        let mut state = crate::inventory::State::default();
        let mut run = |t: &mut NbtTree, e: Edit| {
            let slots = t.inventory_slots(&list);
            t.inventory_edit(&list, &slots, e, &mut state);
        };

        // Moving into an empty slot, swapping two stacks.
        t.frame_start = Some(t.snap());
        run(&mut t, Edit::Move { from: 0, to: 4 });
        assert_eq!((slot_of(&t, 0), slot_of(&t, 1)), (Some(4), Some(9)));
        t.frame_start = Some(t.snap());
        run(&mut t, Edit::Swap(4, 9));
        assert_eq!((slot_of(&t, 0), slot_of(&t, 1)), (Some(9), Some(4)));
        t.undo();
        assert_eq!((slot_of(&t, 0), slot_of(&t, 1)), (Some(4), Some(9)));
        t.undo();
        assert_eq!((slot_of(&t, 0), slot_of(&t, 1)), (Some(0), Some(9)));

        // The count and the wear keep the type of the tag they change.
        t.frame_start = Some(t.snap());
        run(&mut t, Edit::SetCount { index: 1, count: 64 });
        t.frame_start = Some(t.snap());
        run(&mut t, Edit::SetDamage { index: 0, damage: 20 });
        let at = |t: &NbtTree, i: usize, k: &str| get_ref(&t.root.tag, &[Seg::Key("Inventory".into()), Seg::Index(i), Seg::Key(k.into())]).cloned();
        assert_eq!(at(&t, 1, "Count"), Some(Tag::Byte(64)));
        assert_eq!(at(&t, 0, "Damage"), Some(Tag::Short(20)));

        // A new item goes in the slot, written like the others; a deleted one leaves its slot empty.
        t.frame_start = Some(t.snap());
        run(&mut t, Edit::Add { slot: 12, id: "minecraft:stick".into(), count: 1 });
        let slots = t.inventory_slots(&list);
        let added = slots.iter().find(|s| s.slot == 12).expect("the new stack");
        assert_eq!((added.id.as_str(), added.count), ("minecraft:stick", 1));
        t.frame_start = Some(t.snap());
        run(&mut t, Edit::Delete(1));
        assert!(t.inventory_slots(&list).iter().all(|s| s.slot != 9));
        t.frame_start = Some(t.snap());
        run(&mut t, Edit::Change { index: 0, id: "minecraft:iron_sword".into() });
        assert_eq!(at(&t, 0, "id"), Some(Tag::String("minecraft:iron_sword".into())));
    }

    #[test]
    fn a_console_inventory_gets_numbered_stacks() {
        use crate::inventory::Edit;
        // As the player file of a console world: the id a number, the count a byte.
        let mut c = Compound::new();
        c.insert("id".into(), Tag::Int(358));
        c.insert("Count".into(), Tag::Byte(1));
        c.insert("Slot".into(), Tag::Byte(9));
        c.insert("Damage".into(), Tag::Short(0));
        let list = List::from_tags(TagType::Compound, vec![Tag::Compound(c)]).unwrap();
        let mut root = Compound::new();
        root.insert("Inventory".into(), Tag::List(list));
        let mut t = NbtTree::new(NamedTag { name: String::new(), tag: Tag::Compound(root) });
        let path = vec![Seg::Key("Inventory".into())];
        let slots = t.inventory_slots(&path);
        assert_eq!(slots[0].number, Some(358));
        let mut state = crate::inventory::State::default();
        t.frame_start = Some(t.snap());
        t.inventory_edit(&path, &slots, Edit::Add { slot: 3, id: "minecraft:diamond".into(), count: 1 }, &mut state);
        let slots = t.inventory_slots(&path);
        let diamond = slots.iter().find(|s| s.slot == 3).expect("added");
        assert_eq!(diamond.number, Some(264));
    }

    #[test]
    fn a_bedrock_inventory_has_a_stack_in_every_slot_and_armor_beside_it() {
        use crate::inventory::Edit;
        let empty = |slot: Option<i8>| {
            let mut c = NbtTree::bedrock_empty(&Compound::new());
            if let Some(s) = slot {
                c.insert("Slot".into(), Tag::Byte(s));
            }
            Tag::Compound(c)
        };
        let mut sword = NbtTree::bedrock_empty(&Compound::new());
        sword.insert("Name".into(), Tag::String("minecraft:diamond_sword".into()));
        sword.insert("Count".into(), Tag::Byte(1));
        sword.insert("Slot".into(), Tag::Byte(0));
        let mut inventory = vec![Tag::Compound(sword)];
        inventory.extend((1..4).map(|s| empty(Some(s))));
        let mut root = Compound::new();
        root.insert("Inventory".into(), Tag::List(List::from_tags(TagType::Compound, inventory).unwrap()));
        root.insert("Armor".into(), Tag::List(List::from_tags(TagType::Compound, (0..4).map(|_| empty(None)).collect()).unwrap()));
        root.insert("Offhand".into(), Tag::List(List::from_tags(TagType::Compound, vec![empty(None)]).unwrap()));
        let mut t = NbtTree::new(NamedTag { name: String::new(), tag: Tag::Compound(root) });
        let path = vec![Seg::Key("Inventory".into())];
        let mut state = crate::inventory::State::default();
        let mut run = |t: &mut NbtTree, e: Edit| {
            let slots = t.inventory_slots(&path);
            t.frame_start = Some(t.snap());
            t.inventory_edit(&path, &slots, e, &mut state);
        };
        // Only the sword shows; the empty slots are not stacks.
        let slots = t.inventory_slots(&path);
        assert_eq!(slots.len(), 1);
        // A sword on the head: armor is a list of its own, the slot's compound is filled in.
        run(&mut t, Edit::Move { from: 0, to: 103 });
        let slots = t.inventory_slots(&path);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].slot, 103);
        let armor = get_ref(&t.root.tag, &[Seg::Key("Armor".into()), Seg::Index(0)]).and_then(Tag::as_compound).expect("head");
        assert_eq!(armor.get("Name"), Some(&Tag::String("minecraft:diamond_sword".into())));
        // The slot it left is an empty stack again, with its Slot.
        let left = get_ref(&t.root.tag, &[Seg::Key("Inventory".into()), Seg::Index(0)]).and_then(Tag::as_compound).expect("slot 0");
        assert_eq!((left.get("Name"), left.get("Slot")), (Some(&Tag::String(String::new())), Some(&Tag::Byte(0))));
        // Enchanting writes Bedrock''s numbers; removing the item empties the slot, not the list.
        run(&mut t, Edit::Enchant { index: 1 << 16, id: "minecraft:sharpness".into(), level: Some(3) });
        let slots = t.inventory_slots(&path);
        assert_eq!(slots[0].enchants, vec![("minecraft:sharpness".to_string(), 3)]);
        run(&mut t, Edit::Add { slot: 2, id: "minecraft:apple".into(), count: 1 });
        assert_eq!(t.inventory_slots(&path).len(), 2);
        run(&mut t, Edit::Delete(1 << 16));
        assert_eq!(t.inventory_slots(&path).len(), 1);
        let Some(Tag::List(l)) = get_ref(&t.root.tag, &[Seg::Key("Armor".into())]) else { panic!() };
        assert_eq!(l.len(), 4);
    }
    /// In a chunk as the game saves it (block states packed in a section, chests' halves in their
    /// `type`), either half of a large chest opens the large chest; a lone chest opens alone.
    /// A villager's menu (trades, flags, health, equipment) and its trades screen, drawn for
    /// looking at (with MCTK_SNAPSHOT_WORLD and MCTK_SNAPSHOT_OUT set); trades added on the
    /// screen land in its `Offers`.
    #[test]
    fn a_villager_menu_and_trades_screen() {
        let (Ok(world), Ok(out)) = (std::env::var("MCTK_SNAPSHOT_WORLD"), std::env::var("MCTK_SNAPSHOT_OUT")) else { return };
        let r = mctk_anvil::Region::open(std::path::Path::new(&world).join("entities").join("r.0.0.mca")).unwrap();
        let mut t = NbtTree::new(r.read_chunk(2, 0).unwrap().unwrap());
        let ents = t.root.tag.get("Entities").and_then(Tag::as_list).unwrap();
        let villager = ents.iter().position(|e| e.get("id").and_then(Tag::as_str) == Some("minecraft:villager")).expect("a villager");
        let at = vec![Seg::Key("Entities".into()), Seg::Index(villager)];
        t.frame_start = Some(t.snap());
        for e in [crate::mob_edit::TradeEdit::Add, crate::mob_edit::TradeEdit::Add] {
            t.apply(Action::EditMob(at.clone(), crate::mob_edit::MobEdit::Trades(e)));
        }
        t.apply(Action::EditMob(at.clone(), crate::mob_edit::MobEdit::Trades(crate::mob_edit::TradeEdit::Item { n: 1, which: 2, id: Some("minecraft:bread".into()) })));
        let made = crate::mob_edit::trades(get_ref(&t.root.tag, &at).and_then(Tag::as_compound).unwrap(), None);
        assert_eq!(made.len(), 2);
        t.expanded.insert(path_key(&vec![Seg::Key("Entities".into())]));
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut tex = crate::snapshot::Textures::default();
        let mut render = |t: &mut NbtTree, name: &str, click: Option<egui::Pos2>, hover: &[egui::Pos2]| {
            let (w, h) = (760.0, 760.0);
            let mut last = Vec::new();
            for frame in 0..(6 + hover.len() * 3) {
                let mut events = Vec::new();
                // The menu left open by the last picture is closed first.
                if frame == 0 {
                    events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
                }
                if let (Some(at), 2) = (click, frame) {
                    events.push(egui::Event::PointerMoved(at));
                    for pressed in [true, false] {
                        events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
                    }
                }
                // Then a click on rows of the menus, one after another, opening each sub-menu.
                if frame >= 4 {
                    let k = (frame - 4) / 3;
                    if let Some(at) = hover.get(k.min(hover.len().saturating_sub(1))) {
                        events.push(egui::Event::PointerMoved(*at));
                        if (frame - 4) % 3 == 0 && k < hover.len() {
                            for pressed in [true, false] {
                                events.push(egui::Event::PointerButton { pos: *at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
                            }
                        }
                    }
                }
                let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))), events, ..Default::default() };
                let o = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme::bg())).show(ctx, |ui| t.ui(ui, false));
                });
                tex.apply(&o.textures_delta);
                last = ctx.tessellate(o.shapes, o.pixels_per_point);
            }
            let img = crate::snapshot::rasterize(&last, &tex, w as usize, h as usize);
            crate::snapshot::save(&img, w as usize, h as usize, &std::path::Path::new(&out).join(name));
        };
        // Its row: below the root and the `Entities` row, one row (18) per entity before it.
        render(&mut t, "villager_menu.png", Some(egui::pos2(200.0, 87.0 + 18.0 * villager as f32)), &[]);
        let row = 87.0 + 18.0 * villager as f32;
        render(&mut t, "villager_group.png", Some(egui::pos2(200.0, row)), &[egui::pos2(290.0, row + 92.0)]);
        render(&mut t, "villager_equipment.png", Some(egui::pos2(200.0, row)), &[egui::pos2(290.0, row + 92.0), egui::pos2(480.0, row + 153.0)]);
        // A slot's own menu: its item, removing it, and what the stack has (kept when the item
        // changes).
        t.apply(Action::EditMob(at.clone(), crate::mob_edit::MobEdit::Equip(crate::mob_edit::Slot::Head, Some(("minecraft:diamond_helmet".into(), 1)))));
        let ench = crate::enchant::EnchantEdit::Add("minecraft:protection".into());
        t.apply(Action::EditMob(at.clone(), crate::mob_edit::MobEdit::EquipStack(crate::mob_edit::Slot::Head, crate::item_edit::StackEdit::Enchant(ench))));
        t.apply(Action::EditMob(at.clone(), crate::mob_edit::MobEdit::Equip(crate::mob_edit::Slot::Head, Some(("minecraft:iron_helmet".into(), 1)))));
        let mob = get_ref(&t.root.tag, &at).and_then(Tag::as_compound).unwrap();
        let head_stack = crate::mob_edit::slot_stack(mob, crate::mob_edit::Slot::Head, None).unwrap();
        assert_eq!(crate::enchant::read(head_stack).len(), 1, "{head_stack:?}");
        let head = (row + 153.0 - 6.0).min(760.0 - 20.0 - 6.0 * 32.0 - 16.0 - 8.0) + 6.0 + 13.0;
        render(&mut t, "villager_slot.png", Some(egui::pos2(200.0, row)), &[egui::pos2(290.0, row + 92.0), egui::pos2(480.0, row + 153.0), egui::pos2(250.0, head)]);
        render(&mut t, "villager_drops.png", Some(egui::pos2(200.0, row)), &[egui::pos2(290.0, row + 92.0), egui::pos2(480.0, row + 185.0)]);
        render(&mut t, "villager_loot.png", Some(egui::pos2(200.0, row)), &[egui::pos2(290.0, row + 92.0), egui::pos2(480.0, row + 185.0), egui::pos2(200.0, row + 229.0)]);
        // Its status effects, and one effect's own menu.
        t.frame_start = Some(t.snap());
        t.apply(Action::EditMob(at.clone(), crate::mob_edit::MobEdit::Effect(crate::effects::EffectEdit::Add("speed".into()))));
        t.apply(Action::EditMob(at.clone(), crate::mob_edit::MobEdit::Effect(crate::effects::EffectEdit::Amplifier(0, 1))));
        render(&mut t, "villager_effects.png", Some(egui::pos2(200.0, row)), &[egui::pos2(290.0, row + 92.0), egui::pos2(480.0, row + 217.0)]);
        render(&mut t, "villager_effect.png", Some(egui::pos2(200.0, row)), &[egui::pos2(290.0, row + 92.0), egui::pos2(480.0, row + 217.0), egui::pos2(200.0, row + 218.0)]);
        t.apply(Action::OpenTrades(at.clone()));
        render(&mut t, "villager_trades.png", None, &[]);
        // A sold stack's menu: its count, enchantments, name and the rest.
        render(&mut t, "villager_trade_menu.png", Some(egui::pos2(284.0, 134.0)), &[]);
    }
    #[test]
    fn a_large_chest_in_a_saved_chunk_opens_as_one() {
        let Ok(world) = std::env::var("MCTK_SNAPSHOT_WORLD") else { return };
        let r = mctk_anvil::Region::open(std::path::Path::new(&world).join("region").join("r.0.0.mca")).unwrap();
        let mut t = NbtTree::new(r.read_chunk(1, 0).unwrap().unwrap());
        let at = |t: &NbtTree, x: i64| {
            let l = t.root.tag.get("block_entities").and_then(Tag::as_list).unwrap();
            l.iter().position(|e| e.get("id").and_then(Tag::as_str) == Some("minecraft:chest") && e.get("x").and_then(Tag::as_i64) == Some(x)).expect("the chest")
        };
        let items = |i: usize| vec![Seg::Key("block_entities".into()), Seg::Index(i), Seg::Key("Items".into())];
        let (lone, right, left) = (at(&t, 18), at(&t, 19), at(&t, 20));
        for half in [right, left] {
            t.frame_start = Some(t.snap());
            t.apply(Action::OpenInventory(items(half)));
            let list = t.inventory.as_ref().map(|(l, _)| l.clone()).expect("open");
            assert_eq!((list, t.inventory_partner.clone()), (items(right), Some(items(left))), "the large chest, its right half first");
            t.inventory = None;
            t.inventory_partner = None;
        }
        t.apply(Action::OpenInventory(items(lone)));
        assert_eq!(t.inventory_partner, None, "a lone chest alone");
        // Marked in the tree: each half knows which it is and where the other is.
        let chest = |i: usize| vec![Seg::Key("block_entities".into()), Seg::Index(i)];
        let pairs = t.find_large_chests();
        assert_eq!((pairs.get(&chest(right)), pairs.get(&chest(left)), pairs.get(&chest(lone))), (Some(&(true, left)), Some(&(false, right)), None));
        // Drawn, for looking at (with MCTK_SNAPSHOT_OUT set).
        if let Ok(out) = std::env::var("MCTK_SNAPSHOT_OUT") {
            t.expanded.insert(path_key(&vec![Seg::Key("block_entities".into())]));
            let ctx = egui::Context::default();
            theme::install(&ctx);
            let mut tex = crate::snapshot::Textures::default();
            let mut render = |t: &mut NbtTree, name: &str, click: Option<egui::Pos2>, hover: Option<egui::Pos2>| {
                let (w, h) = (640.0, 460.0);
                let mut last = Vec::new();
                for frame in 0..9 {
                    let mut events = Vec::new();
                    if let (Some(at), 2) = (click, frame) {
                        events.push(egui::Event::PointerMoved(at));
                        for pressed in [true, false] {
                            events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
                        }
                    }
                    // Then over a row of the menu, to open its sub-menu.
                    if let (Some(at), true) = (hover, frame >= 4) {
                        events.push(egui::Event::PointerMoved(at));
                    }
                    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))), events, ..Default::default() };
                    let o = ctx.run(input, |ctx| {
                        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme::bg())).show(ctx, |ui| t.ui(ui, false));
                    });
                    tex.apply(&o.textures_delta);
                    last = ctx.tessellate(o.shapes, o.pixels_per_point);
                }
                let img = crate::snapshot::rasterize(&last, &tex, w as usize, h as usize);
                crate::snapshot::save(&img, w as usize, h as usize, &std::path::Path::new(&out).join(name));
            };
            t.inventory = None;
            t.inventory_partner = None;
            render(&mut t, "tree_chests.png", None, None);
            // The lone chest's menu: its name, typed straight in.
            render(&mut t, "tree_chest_menu.png", Some(egui::pos2(160.0, 123.0)), None);
            // The right half's diamonds: an attribute and a block it can break, then its menu.
            let stack = { let mut p = items(right); p.push(Seg::Index(0)); p };
            t.apply(Action::EditItem(stack.clone(), crate::item_edit::ItemEdit::Attribute(crate::item_edit::AttrEdit::Add("attack_damage".into()))));
            t.apply(Action::EditItem(stack.clone(), crate::item_edit::ItemEdit::Rule(crate::item_edit::Rule::Break, "minecraft:stone".into(), true)));
            t.apply(Action::SetName(stack.clone(), "Shiny".into()));
            t.expanded.insert(path_key(&chest(right)));
            t.expanded.insert(path_key(&items(right)));
            render(&mut t, "item_menu.png", Some(egui::pos2(110.0, 177.0)), None);
            render(&mut t, "item_menu_open.png", Some(egui::pos2(110.0, 177.0)), Some(egui::pos2(200.0, 276.0)));
            render(&mut t, "item_change.png", Some(egui::pos2(110.0, 177.0)), Some(egui::pos2(200.0, 276.0)));
            t.apply(Action::OpenInventory(items(left)));
            render(&mut t, "large_chest.png", None, None);
        }
    }
    #[test]
    fn two_chests_side_by_side_open_as_one_large_chest() {
        use crate::inventory::Edit;
        let prop = |ty: &str| {
            let mut p = Compound::new();
            p.insert("type".into(), Tag::String(ty.into()));
            p
        };
        let state = |name: &str, props: Option<Compound>| {
            let mut s = Compound::new();
            s.insert("Name".into(), Tag::String(name.into()));
            if let Some(p) = props {
                s.insert("Properties".into(), Tag::Compound(p));
            }
            Tag::Compound(s)
        };
        let palette = List::from_tags(TagType::Compound, vec![state("minecraft:air", None), state("minecraft:chest", Some(prop("right"))), state("minecraft:chest", Some(prop("left")))]).unwrap();
        let mut states = Compound::new();
        states.insert("palette".into(), Tag::List(palette));
        // Blocks 0 and 1 of the section: the right chest, then the left one (4 bits each, 16
        // to a long, the 4096 in 256 longs).
        let mut data = vec![0i64; 256];
        data[0] = 0x21;
        states.insert("data".into(), Tag::LongArray(data));
        let mut section = Compound::new();
        section.insert("Y".into(), Tag::Byte(0));
        section.insert("block_states".into(), Tag::Compound(states));
        let chest = |x: i32, item: &str| {
            let mut stack = Compound::new();
            stack.insert("id".into(), Tag::String(item.into()));
            stack.insert("count".into(), Tag::Int(1));
            stack.insert("Slot".into(), Tag::Byte(3));
            let mut c = Compound::new();
            c.insert("id".into(), Tag::String("minecraft:chest".into()));
            c.insert("x".into(), Tag::Int(x));
            c.insert("y".into(), Tag::Int(0));
            c.insert("z".into(), Tag::Int(0));
            c.insert("Items".into(), Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(stack)]).unwrap()));
            Tag::Compound(c)
        };
        let mut root = Compound::new();
        root.insert("DataVersion".into(), Tag::Int(4325));
        root.insert("sections".into(), Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(section)]).unwrap()));
        // The left chest is listed first, to show that the right one still leads.
        root.insert("block_entities".into(), Tag::List(List::from_tags(TagType::Compound, vec![chest(1, "minecraft:stick"), chest(0, "minecraft:apple")]).unwrap()));
        let mut t = NbtTree::new(NamedTag { name: String::new(), tag: Tag::Compound(root) });
        let items = |i: usize| vec![Seg::Key("block_entities".into()), Seg::Index(i), Seg::Key("Items".into())];
        t.frame_start = Some(t.snap());
        t.apply(Action::OpenInventory(items(0)));
        let list = t.inventory.as_ref().map(|(l, _)| l.clone()).expect("open");
        assert_eq!(list, items(1), "the right chest (x=0) leads");
        assert_eq!(t.inventory_partner, Some(items(0)));
        let slots = t.inventory_slots(&list);
        let by = |slot: i8| slots.iter().find(|s| s.slot == slot).map(|s| s.id.clone());
        assert_eq!(by(3), Some("minecraft:apple".into()));
        assert_eq!(by(30), Some("minecraft:stick".into()));
        // Carry the apple into the left chest''s empty slot 31.
        let mut state = crate::inventory::State::default();
        t.frame_start = Some(t.snap());
        t.inventory_edit(&list, &slots, Edit::Move { from: 3, to: 31 }, &mut state);
        let slots = t.inventory_slots(&list);
        assert!(slots.iter().all(|s| s.slot != 3));
        assert_eq!(slots.iter().find(|s| s.slot == 31).map(|s| s.id.as_str()), Some("minecraft:apple"));
    }
    #[test]
    fn edits_are_marked_and_undone() {
        let mut c = Compound::new();
        c.insert("a".into(), Tag::Int(1));
        let mut t = NbtTree::new(NamedTag { name: String::new(), tag: Tag::Compound(c) });
        let a = vec![Seg::Key("a".into())];
        // An edit made in place marks the tag, and the mark remembers the old value.
        t.frame_start = Some(t.snap());
        *get_mut(&mut t.root.tag, &a).unwrap() = Tag::Int(5);
        t.apply(Action::ChangedAt(a.clone()));
        assert!(matches!(t.marks.list.as_slice(), [m] if m.kind == Kind::Edited));
        // Putting the old value back clears it again.
        t.frame_start = Some(t.snap());
        t.apply(Action::Restore(a.clone()));
        assert_eq!(t.root.tag.get("a"), Some(&Tag::Int(1)));
        assert!(t.marks.list.is_empty());
        // A tag that was only just added has nothing to undo until it is changed.
        t.frame_start = Some(t.snap());
        t.apply(Action::Add(vec![], TagType::Byte));
        let added = vec![Seg::Key("New byte".into())];
        assert!(t.is_fresh(&added));
        t.finish_rename(added, "named".into());
        let named = vec![Seg::Key("named".into())];
        assert!(t.is_fresh(&named));
        t.frame_start = Some(t.snap());
        t.apply(Action::Restore(named));
        assert!(t.root.tag.get("New byte").is_some() && t.root.tag.get("named").is_none());
        assert!(t.is_fresh(&vec![Seg::Key("New byte".into())]));
    }
    #[test]
    fn a_tag_can_redo_what_was_undone() {
        let mut c = Compound::new();
        c.insert("a".into(), Tag::Int(1));
        let mut t = NbtTree::new(NamedTag { name: String::new(), tag: Tag::Compound(c) });
        let a = vec![Seg::Key("a".into())];
        // Edit in place, undo it from the tag's menu, then redo it from there.
        t.frame_start = Some(t.snap());
        *get_mut(&mut t.root.tag, &a).unwrap() = Tag::Int(5);
        t.apply(Action::ChangedAt(a.clone()));
        t.frame_start = Some(t.snap());
        t.apply(Action::Restore(a.clone()));
        assert_eq!(t.root.tag.get("a"), Some(&Tag::Int(1)));
        t.frame_start = Some(t.snap());
        t.apply(Action::RedoAt(a.clone()));
        assert_eq!(t.root.tag.get("a"), Some(&Tag::Int(5)));
        assert!(t.marks.list.iter().any(|m| m.kind == Kind::Edited && m.path == a));
        // The toolbar's undo leaves a Redo on the tags it changed.
        t.frame_start = Some(t.snap());
        t.apply(Action::Snapshot);
        *get_mut(&mut t.root.tag, &a).unwrap() = Tag::Int(9);
        t.undo();
        assert_eq!(t.root.tag.get("a"), Some(&Tag::Int(5)));
        assert!(t.undone.iter().any(|u| *u.path() == a));
    }
    #[test]
    fn undoing_everything_leaves_nothing_changed() {
        let mut c = Compound::new();
        c.insert("a".into(), Tag::Int(1));
        let mut t = NbtTree::new(NamedTag { name: String::new(), tag: Tag::Compound(c) });
        let a = vec![Seg::Key("a".into())];
        t.frame_start = Some(t.snap());
        t.apply(Action::Snapshot);
        *get_mut(&mut t.root.tag, &a).unwrap() = Tag::Int(2);
        assert!(t.dirty);
        t.undo();
        assert!(!t.dirty, "back at the opened state");
        t.redo();
        assert!(t.dirty);
        // After a save, undoing past it is a change again, and coming back is not.
        t.mark_saved();
        assert!(!t.dirty);
        t.undo();
        assert!(t.dirty);
        t.redo();
        assert!(!t.dirty);
    }
    #[test]
    fn hints_read_naturally() {
        assert_eq!(
            hint("GameType", &Tag::Int(1)),
            Some(("Creative".into(), true))
        );
        assert_eq!(
            hint("Time", &Tag::Long(823)),
            Some(("41 seconds".into(), false))
        );
        // The value shown as "October 29, 2026, 03:36AM" for the sample world.
        assert_eq!(date(1_793_245_010_889), "October 29, 2026, 03:36AM");
    }
}
