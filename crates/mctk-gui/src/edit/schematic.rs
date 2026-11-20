//! Schematic files, read into the clipboard and written from it:
//!
//! | file | made by | blocks | block entities |
//! |---|---|---|---|
//! | `.schem` | WorldEdit, Litematica's export (Sponge v2 and v3) | palette of state strings, varint indices | Java |
//! | `.nbt` | Java's structure blocks | palette of `{Name, Properties}`, one entry per block | Java |
//! | `.mcstructure` | Bedrock's structure blocks | Bedrock palette, two layers (the second: water) | Bedrock |
//! | `.schematic` | MCEdit, old WorldEdit | numeric ids and data values | numeric-era (console) |
//!
//! Block entities are kept as the file holds them, and pasted into the kind of world that keeps
//! them so (a `.schem` chest's contents into a Java world).

use std::collections::HashMap;

use mctk_java::BlockState;
use mctk_nbt::{Compound, Flavor, List, NamedTag, Tag, TagType};

use super::tools::Clipboard;

/// The schematic formats, by file extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Sponge,
    Structure,
    McStructure,
    Legacy,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Sponge, Format::Structure, Format::McStructure, Format::Legacy];

    pub fn from_path(p: &std::path::Path) -> Option<Format> {
        match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "schem" => Some(Format::Sponge),
            "nbt" => Some(Format::Structure),
            "mcstructure" => Some(Format::McStructure),
            "schematic" => Some(Format::Legacy),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Sponge => "schem",
            Format::Structure => "nbt",
            Format::McStructure => "mcstructure",
            Format::Legacy => "schematic",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Sponge => "WorldEdit schematic",
            Format::Structure => "Java structure",
            Format::McStructure => "Bedrock structure",
            Format::Legacy => "MCEdit schematic",
        }
    }
}

/// The data version written into Java files when the copy came from no Java world (1.21).
const DEFAULT_DATA_VERSION: i32 = 3953;

fn gzip(root: &NamedTag) -> Result<Vec<u8>, String> {
    let raw = mctk_nbt::to_bytes(root, Flavor::Java).map_err(|e| e.to_string())?;
    crate::nbt_view::compress(mctk_nbt::Compression::Gzip, raw)
}

fn java_root(bytes: &[u8]) -> Result<NamedTag, String> {
    let (_, raw) = mctk_nbt::decompress(bytes).map_err(|e| e.to_string())?;
    mctk_nbt::from_bytes(&raw, Flavor::Java).map_err(|e| e.to_string())
}

fn int(c: &Compound, k: &str) -> i32 {
    c.get(k).and_then(Tag::as_i64).unwrap_or(0) as i32
}

fn ints(t: Option<&Tag>) -> Vec<i32> {
    match t {
        Some(Tag::IntArray(v)) => v.clone(),
        Some(Tag::List(l)) => l.iter().filter_map(Tag::as_i64).map(|v| v as i32).collect(),
        _ => Vec::new(),
    }
}

fn doubles(t: Option<&Tag>) -> Option<[f64; 3]> {
    let l = t.and_then(Tag::as_list)?;
    let v = |i: usize| l.get(i).and_then(Tag::as_f64);
    Some([v(0)?, v(1)?, v(2)?])
}

fn double_list(v: [f64; 3]) -> Tag {
    Tag::List(List::from_tags(TagType::Double, v.iter().map(|x| Tag::Double(*x)).collect()).expect("doubles"))
}

/// The entities a file holds, each `(position in the box, entity)`, from a list of compounds;
/// `split` gives each one's position and the entity itself.
fn entities_from(list: Option<&Tag>, split: impl Fn(&Compound) -> Option<([f64; 3], Compound)>) -> Vec<([f64; 3], Compound)> {
    list.and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()).filter_map(Tag::as_compound).filter_map(split).collect()
}

fn int_list(v: &[i32]) -> Tag {
    Tag::List(List::from_tags(TagType::Int, v.iter().map(|x| Tag::Int(*x)).collect()).expect("ints"))
}

fn compounds(items: Vec<Compound>) -> Tag {
    Tag::List(List::from_tags(TagType::Compound, items.into_iter().map(Tag::Compound).collect()).unwrap_or_else(|_| List::new(TagType::Compound)))
}

/// `minecraft:oak_stairs[facing=north,half=bottom]` as a state.
fn parse_state(s: &str) -> BlockState {
    let (name, props) = match s.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']')),
        None => (s, ""),
    };
    let mut b = BlockState::new(if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") });
    for kv in props.split(',').filter(|kv| !kv.is_empty()) {
        if let Some((k, v)) = kv.split_once('=') {
            b.properties.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    b
}

fn state_string(b: &BlockState) -> String {
    if b.properties.is_empty() {
        return b.name.clone();
    }
    let props: Vec<String> = b.properties.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{}[{}]", b.name, props.join(","))
}

/// An empty clipboard of a size (all air).
fn empty(size: [i32; 3], edition: &str) -> Clipboard {
    let n = (size[0].max(0) * size[1].max(0) * size[2].max(0)) as usize;
    Clipboard { size, palette: vec![BlockState::air()], blocks: vec![0; n], block_entities: Vec::new(), edition: edition.into(), data_version: None, entities: Vec::new(), origin: [0, 0, 0] }
}

fn index(size: [i32; 3], x: i32, y: i32, z: i32) -> usize {
    ((y * size[2] + z) * size[0] + x) as usize
}

/// Reads a schematic file into a clipboard.
pub fn read(bytes: &[u8], format: Format) -> Result<Clipboard, String> {
    match format {
        Format::Sponge => read_sponge(&java_root(bytes)?),
        Format::Structure => read_structure(&java_root(bytes)?),
        Format::McStructure => read_mcstructure(&mctk_nbt::from_bytes(bytes, Flavor::Bedrock).map_err(|e| e.to_string())?),
        Format::Legacy => read_legacy(&java_root(bytes)?),
    }
}

/// Writes a clipboard as a schematic file.
pub fn write(clip: &Clipboard, format: Format) -> Result<Vec<u8>, String> {
    match format {
        Format::Sponge => gzip(&write_sponge(clip)),
        Format::Structure => gzip(&write_structure(clip)),
        Format::McStructure => mctk_nbt::to_bytes(&write_mcstructure(clip), Flavor::Bedrock).map_err(|e| e.to_string()),
        Format::Legacy => gzip(&write_legacy(clip)),
    }
}

// ---------------------------------------------------------------------------------------------
// Sponge (.schem)
// ---------------------------------------------------------------------------------------------

fn varints(data: &[i8]) -> Vec<u32> {
    let mut out = Vec::new();
    let (mut v, mut shift) = (0u32, 0);
    for &b in data {
        let b = b as u8;
        v |= u32::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            out.push(v);
            v = 0;
            shift = 0;
        } else {
            shift += 7;
        }
    }
    out
}

fn put_varint(out: &mut Vec<i8>, mut v: u32) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b as i8);
            return;
        }
        out.push((b | 0x80) as i8);
    }
}

fn read_sponge(root: &NamedTag) -> Result<Clipboard, String> {
    let top = root.tag.as_compound().ok_or("not a schematic")?;
    // v3 wraps everything in `Schematic`; v2 is the root itself.
    let s = top.get("Schematic").and_then(Tag::as_compound).unwrap_or(top);
    let size = [int(s, "Width"), int(s, "Height"), int(s, "Length")];
    let blocks = s.get("Blocks").and_then(Tag::as_compound);
    let palette = blocks.and_then(|b| b.get("Palette")).or_else(|| s.get("Palette")).and_then(Tag::as_compound).ok_or("no palette")?;
    let data = match blocks.and_then(|b| b.get("Data")).or_else(|| s.get("BlockData")) {
        Some(Tag::ByteArray(d)) => d.clone(),
        _ => return Err("no block data".into()),
    };
    let mut clip = empty(size, "java");
    clip.data_version = s.get("DataVersion").and_then(Tag::as_i64).map(|v| v as i32);
    let mut by_number: HashMap<u32, u32> = HashMap::new();
    for (k, v) in palette {
        let state = parse_state(k);
        let slot = if state.is_air() { 0 } else {
            clip.palette.push(state);
            (clip.palette.len() - 1) as u32
        };
        by_number.insert(v.as_i64().unwrap_or(0) as u32, slot);
    }
    for (i, n) in varints(&data).into_iter().enumerate().take(clip.blocks.len()) {
        clip.blocks[i] = by_number.get(&n).copied().unwrap_or(0);
    }
    let bes = blocks.and_then(|b| b.get("BlockEntities")).or_else(|| s.get("BlockEntities")).or_else(|| s.get("TileEntities")).and_then(Tag::as_list);
    for be in bes.into_iter().flat_map(|l| l.iter()).filter_map(Tag::as_compound) {
        let pos = ints(be.get("Pos"));
        if pos.len() != 3 {
            continue;
        }
        // v3 keeps the data apart (`Data`); v2 beside `Pos` and `Id`.
        let mut c = be.get("Data").and_then(Tag::as_compound).cloned().unwrap_or_else(|| be.clone());
        c.shift_remove("Pos");
        if let Some(id) = be.get("Id").and_then(Tag::as_str) {
            c.shift_remove("Id");
            c.insert("id".into(), Tag::String(id.to_string()));
        }
        clip.block_entities.push(([pos[0], pos[1], pos[2]], c));
    }
    // Entities: `{Pos, Id, Data}` (v3), or with their data beside `Pos` and `Id` (v2).
    clip.entities = entities_from(s.get("Entities"), |e| {
        let pos = doubles(e.get("Pos"))?;
        let mut c = e.get("Data").and_then(Tag::as_compound).cloned().unwrap_or_else(|| e.clone());
        c.shift_remove("Data");
        if let Some(id) = e.get("Id").and_then(Tag::as_str) {
            c.shift_remove("Id");
            c.insert("id".into(), Tag::String(id.to_string()));
        }
        Some((pos, c))
    });
    Ok(clip)
}

fn write_sponge(clip: &Clipboard) -> NamedTag {
    let mut palette = Compound::new();
    for (i, s) in clip.palette.iter().enumerate() {
        palette.insert(state_string(s), Tag::Int(i as i32));
    }
    let mut data = Vec::new();
    for &b in &clip.blocks {
        put_varint(&mut data, b);
    }
    let bes: Vec<Compound> = if clip.edition == "java" {
        clip.block_entities
            .iter()
            .map(|(p, be)| {
                let mut inner = be.clone();
                let id = inner.shift_remove("id").unwrap_or(Tag::String(String::new()));
                for k in ["x", "y", "z"] {
                    inner.shift_remove(k);
                }
                let mut c = Compound::new();
                c.insert("Pos".into(), Tag::IntArray(p.to_vec()));
                c.insert("Id".into(), id);
                c.insert("Data".into(), Tag::Compound(inner));
                c
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut blocks = Compound::new();
    blocks.insert("Palette".into(), Tag::Compound(palette));
    blocks.insert("Data".into(), Tag::ByteArray(data));
    blocks.insert("BlockEntities".into(), compounds(bes));
    let mut s = Compound::new();
    s.insert("Version".into(), Tag::Int(3));
    s.insert("DataVersion".into(), Tag::Int(clip.data_version.unwrap_or(DEFAULT_DATA_VERSION)));
    s.insert("Width".into(), Tag::Short(clip.size[0] as i16));
    s.insert("Height".into(), Tag::Short(clip.size[1] as i16));
    s.insert("Length".into(), Tag::Short(clip.size[2] as i16));
    s.insert("Offset".into(), Tag::IntArray(vec![0, 0, 0]));
    s.insert("Blocks".into(), Tag::Compound(blocks));
    if clip.edition == "java" && !clip.entities.is_empty() {
        let list = clip
            .entities
            .iter()
            .map(|(p, e)| {
                let mut data = e.clone();
                let id = data.get("id").cloned().unwrap_or(Tag::String(String::new()));
                data.insert("Pos".into(), double_list(*p));
                let mut c = Compound::new();
                c.insert("Pos".into(), double_list(*p));
                c.insert("Id".into(), id);
                c.insert("Data".into(), Tag::Compound(data));
                c
            })
            .collect();
        s.insert("Entities".into(), compounds(list));
    }
    let mut top = Compound::new();
    top.insert("Schematic".into(), Tag::Compound(s));
    NamedTag { name: String::new(), tag: Tag::Compound(top) }
}

// ---------------------------------------------------------------------------------------------
// Java structure (.nbt)
// ---------------------------------------------------------------------------------------------

fn read_structure(root: &NamedTag) -> Result<Clipboard, String> {
    let s = root.tag.as_compound().ok_or("not a structure")?;
    let size = ints(s.get("size"));
    if size.len() != 3 {
        return Err("no size".into());
    }
    let mut clip = empty([size[0], size[1], size[2]], "java");
    clip.data_version = s.get("DataVersion").and_then(Tag::as_i64).map(|v| v as i32);
    // Several palettes (shipwrecks...): the first.
    let palette = s.get("palette").or_else(|| s.get("palettes").and_then(Tag::as_list).and_then(|l| l.get(0))).and_then(Tag::as_list).ok_or("no palette")?;
    let slots: Vec<u32> = palette
        .iter()
        .map(|t| {
            let st = BlockState::from_palette_entry(t).unwrap_or_else(BlockState::air);
            // Structure void is "leave what is there": air, as the clipboard keeps it.
            if st.is_air() || st.name == "minecraft:structure_void" {
                return 0;
            }
            clip.palette.push(st);
            (clip.palette.len() - 1) as u32
        })
        .collect();
    for b in s.get("blocks").and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()).filter_map(Tag::as_compound) {
        let p = ints(b.get("pos"));
        if p.len() != 3 || p.iter().zip(clip.size).any(|(v, s)| *v < 0 || *v >= s) {
            continue;
        }
        let slot = slots.get(int(b, "state") as usize).copied().unwrap_or(0);
        let i = index(clip.size, p[0], p[1], p[2]);
        clip.blocks[i] = slot;
        if let Some(nbt) = b.get("nbt").and_then(Tag::as_compound) {
            clip.block_entities.push(([p[0], p[1], p[2]], nbt.clone()));
        }
    }
    clip.entities = entities_from(s.get("entities"), |e| Some((doubles(e.get("pos"))?, e.get("nbt").and_then(Tag::as_compound)?.clone())));
    Ok(clip)
}

fn write_structure(clip: &Clipboard) -> NamedTag {
    let palette: Vec<Tag> = clip.palette.iter().map(BlockState::to_named_compound).collect();
    let bes: HashMap<[i32; 3], &Compound> = if clip.edition == "java" { clip.block_entities.iter().map(|(p, c)| (*p, c)).collect() } else { HashMap::new() };
    let mut blocks = Vec::new();
    for y in 0..clip.size[1] {
        for z in 0..clip.size[2] {
            for x in 0..clip.size[0] {
                let mut b = Compound::new();
                b.insert("pos".into(), int_list(&[x, y, z]));
                b.insert("state".into(), Tag::Int(clip.blocks[index(clip.size, x, y, z)] as i32));
                if let Some(be) = bes.get(&[x, y, z]) {
                    let mut nbt = (*be).clone();
                    for k in ["x", "y", "z"] {
                        nbt.shift_remove(k);
                    }
                    b.insert("nbt".into(), Tag::Compound(nbt));
                }
                blocks.push(b);
            }
        }
    }
    let mut s = Compound::new();
    s.insert("DataVersion".into(), Tag::Int(clip.data_version.unwrap_or(DEFAULT_DATA_VERSION)));
    s.insert("size".into(), int_list(&clip.size));
    s.insert("palette".into(), Tag::List(List::from_tags(TagType::Compound, palette).expect("compounds")));
    s.insert("blocks".into(), compounds(blocks));
    let entities = if clip.edition == "java" {
        clip.entities
            .iter()
            .map(|(p, e)| {
                let mut c = Compound::new();
                c.insert("pos".into(), double_list(*p));
                c.insert("blockPos".into(), int_list(&[p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32]));
                c.insert("nbt".into(), Tag::Compound(e.clone()));
                c
            })
            .collect()
    } else {
        Vec::new()
    };
    s.insert("entities".into(), compounds(entities));
    NamedTag { name: String::new(), tag: Tag::Compound(s) }
}

// ---------------------------------------------------------------------------------------------
// Bedrock structure (.mcstructure)
// ---------------------------------------------------------------------------------------------

/// A Bedrock structure's index of a position: z fastest, then y, then x.
fn mc_index(size: [i32; 3], x: i32, y: i32, z: i32) -> usize {
    ((x * size[1] + y) * size[2] + z) as usize
}

fn read_mcstructure(root: &NamedTag) -> Result<Clipboard, String> {
    let s = root.tag.as_compound().ok_or("not a structure")?;
    let size = ints(s.get("size"));
    if size.len() != 3 {
        return Err("no size".into());
    }
    let size = [size[0], size[1], size[2]];
    let mut clip = empty(size, "bedrock");
    let st = s.get("structure").and_then(Tag::as_compound).ok_or("no structure")?;
    let layers = st.get("block_indices").and_then(Tag::as_list).ok_or("no blocks")?;
    let layer = |k: usize| ints(layers.get(k));
    let (main, water) = (layer(0), layer(1));
    let pal = st.get("palette").and_then(|p| p.get("default")).and_then(Tag::as_compound).ok_or("no palette")?;
    let entries: Vec<NamedTag> = pal.get("block_palette").and_then(Tag::as_list).map(|l| l.iter().map(|t| NamedTag { name: String::new(), tag: t.clone() }).collect()).unwrap_or_default();
    let states: Vec<BlockState> = entries.iter().map(crate::world::bedrock_java_state).collect();
    let wet: Vec<bool> = entries.iter().map(|e| e.tag.get("name").and_then(Tag::as_str).is_some_and(|n| n.ends_with("water"))).collect();
    let mut slots: HashMap<(i32, bool), u32> = HashMap::new();
    for x in 0..size[0] {
        for y in 0..size[1] {
            for z in 0..size[2] {
                let mi = mc_index(size, x, y, z);
                let k = main.get(mi).copied().unwrap_or(-1);
                // -1 is "leave what is there": air, as the clipboard keeps it.
                if k < 0 {
                    continue;
                }
                let soaked = water.get(mi).is_some_and(|w| *w >= 0 && wet.get(*w as usize).copied().unwrap_or(false));
                let slot = *slots.entry((k, soaked)).or_insert_with(|| {
                    let mut s = states.get(k as usize).cloned().unwrap_or_else(BlockState::air);
                    if s.is_air() {
                        return 0;
                    }
                    if soaked && s.properties.contains_key("waterlogged") {
                        s.properties.insert("waterlogged".into(), "true".into());
                    }
                    clip.palette.push(s);
                    (clip.palette.len() - 1) as u32
                });
                clip.blocks[index(size, x, y, z)] = slot;
            }
        }
    }
    for (k, v) in pal.get("block_position_data").and_then(Tag::as_compound).into_iter().flatten() {
        let Some(be) = v.get("block_entity_data").and_then(Tag::as_compound) else { continue };
        let Ok(i) = k.parse::<i32>() else { continue };
        let (yz, z) = (i / size[2], i % size[2]);
        let (x, y) = (yz / size[1], yz % size[1]);
        clip.block_entities.push(([x, y, z], be.clone()));
    }
    // Entities keep their place in the world the structure was saved from.
    let o = ints(s.get("structure_world_origin"));
    let o = if o.len() == 3 { [o[0] as f64, o[1] as f64, o[2] as f64] } else { [0.0; 3] };
    clip.entities = entities_from(st.get("entities"), |e| {
        let p = doubles(e.get("Pos"))?;
        Some(([p[0] - o[0], p[1] - o[1], p[2] - o[2]], e.clone()))
    });
    Ok(clip)
}

fn write_mcstructure(clip: &Clipboard) -> NamedTag {
    let size = clip.size;
    let n = (size[0] * size[1] * size[2]) as usize;
    let mut palette: Vec<Tag> = Vec::new();
    let mut by_state: HashMap<String, i32> = HashMap::new();
    let mut slot_of = |nbt: NamedTag| -> i32 {
        let key = format!("{:?}", nbt.tag);
        *by_state.entry(key).or_insert_with(|| {
            palette.push(nbt.tag);
            palette.len() as i32 - 1
        })
    };
    let to_bedrock = crate::world::java_bedrock_state;
    let mut main = vec![0i32; n];
    let mut water = vec![-1i32; n];
    let mapped: Vec<Option<NamedTag>> = clip.palette.iter().map(to_bedrock).collect();
    let water_state = to_bedrock(&BlockState::new("minecraft:water").with("level", "0"));
    let air = to_bedrock(&BlockState::air());
    for y in 0..size[1] {
        for z in 0..size[2] {
            for x in 0..size[0] {
                let k = clip.blocks[index(size, x, y, z)] as usize;
                let mi = mc_index(size, x, y, z);
                main[mi] = match mapped[k].clone().or_else(|| air.clone()) {
                    Some(t) => slot_of(t),
                    None => -1,
                };
                if clip.palette[k].properties.get("waterlogged").is_some_and(|v| v == "true") {
                    if let Some(w) = water_state.clone() {
                        water[mi] = slot_of(w);
                    }
                }
            }
        }
    }
    let mut positions = Compound::new();
    if clip.edition == "bedrock" {
        for (p, be) in &clip.block_entities {
            let mut c = Compound::new();
            c.insert("block_entity_data".into(), Tag::Compound(be.clone()));
            positions.insert(mc_index(size, p[0], p[1], p[2]).to_string(), Tag::Compound(c));
        }
    }
    let mut default = Compound::new();
    default.insert("block_palette".into(), Tag::List(List::from_tags(TagType::Compound, palette).unwrap_or_else(|_| List::new(TagType::Compound))));
    default.insert("block_position_data".into(), Tag::Compound(positions));
    let mut pal = Compound::new();
    pal.insert("default".into(), Tag::Compound(default));
    let indices = List::from_tags(TagType::List, vec![int_list(&main), int_list(&water)]).expect("lists");
    let mut st = Compound::new();
    st.insert("block_indices".into(), Tag::List(indices));
    let entities = if clip.edition == "bedrock" {
        clip.entities
            .iter()
            .map(|(p, e)| {
                let mut c = e.clone();
                c.insert("Pos".into(), Tag::List(List::from_tags(TagType::Float, p.iter().map(|v| Tag::Float(*v as f32)).collect()).expect("floats")));
                c
            })
            .collect()
    } else {
        Vec::new()
    };
    st.insert("entities".into(), compounds(entities));
    st.insert("palette".into(), Tag::Compound(pal));
    let mut s = Compound::new();
    s.insert("format_version".into(), Tag::Int(1));
    s.insert("size".into(), int_list(&size));
    s.insert("structure".into(), Tag::Compound(st));
    s.insert("structure_world_origin".into(), int_list(&[0, 0, 0]));
    NamedTag { name: String::new(), tag: Tag::Compound(s) }
}

// ---------------------------------------------------------------------------------------------
// MCEdit (.schematic)
// ---------------------------------------------------------------------------------------------

fn read_legacy(root: &NamedTag) -> Result<Clipboard, String> {
    let s = root.tag.as_compound().ok_or("not a schematic")?;
    let size = [int(s, "Width"), int(s, "Height"), int(s, "Length")];
    let blocks = match s.get("Blocks") {
        Some(Tag::ByteArray(b)) => b,
        _ => return Err("no blocks".into()),
    };
    let data = match s.get("Data") {
        Some(Tag::ByteArray(d)) => d.as_slice(),
        _ => &[],
    };
    let add = match s.get("AddBlocks") {
        Some(Tag::ByteArray(a)) => Some(a.as_slice()),
        _ => None,
    };
    let mut clip = empty(size, "console");
    let mut slots: HashMap<(u16, u8), u32> = HashMap::new();
    for (i, b) in blocks.iter().enumerate().take(clip.blocks.len()) {
        // AddBlocks holds the high bits, two blocks to a byte (the first in the high nibble).
        let high = add.map_or(0, |a| {
            let byte = a.get(i / 2).copied().unwrap_or(0) as u8;
            u16::from(if i % 2 == 0 { byte >> 4 } else { byte & 0x0f })
        });
        let id = u16::from(*b as u8) | (high << 8);
        let d = data.get(i).copied().unwrap_or(0) as u8 & 0x0f;
        if id == 0 {
            continue;
        }
        let slot = *slots.entry((id, d)).or_insert_with(|| {
            clip.palette.push(crate::world::legacy_java_state(id, d));
            (clip.palette.len() - 1) as u32
        });
        clip.blocks[i] = slot;
    }
    for be in s.get("TileEntities").and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()).filter_map(Tag::as_compound) {
        clip.block_entities.push(([int(be, "x"), int(be, "y"), int(be, "z")], be.clone()));
    }
    clip.entities = entities_from(s.get("Entities"), |e| Some((doubles(e.get("Pos"))?, e.clone())));
    Ok(clip)
}

fn write_legacy(clip: &Clipboard) -> NamedTag {
    let n = clip.blocks.len();
    let numbers: Vec<(u16, u8)> = clip.palette.iter().map(|s| if s.is_air() { (0, 0) } else { mctk_mapping::legacy::legacy().numeric_state(s).unwrap_or((0, 0)) }).collect();
    let mut blocks = vec![0i8; n];
    let mut data = vec![0i8; n];
    let mut add = vec![0i8; n.div_ceil(2)];
    let mut any_add = false;
    for (i, &k) in clip.blocks.iter().enumerate() {
        let (id, d) = numbers[k as usize];
        blocks[i] = id as u8 as i8;
        data[i] = d as i8;
        if id > 255 {
            any_add = true;
            let h = ((id >> 8) & 0x0f) as u8;
            let b = add[i / 2] as u8;
            add[i / 2] = if i % 2 == 0 { (b & 0x0f) | (h << 4) } else { (b & 0xf0) | h } as i8;
        }
    }
    let bes: Vec<Compound> = if clip.edition == "console" {
        clip.block_entities
            .iter()
            .map(|(p, be)| {
                let mut c = be.clone();
                c.insert("x".into(), Tag::Int(p[0]));
                c.insert("y".into(), Tag::Int(p[1]));
                c.insert("z".into(), Tag::Int(p[2]));
                c
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut s = Compound::new();
    s.insert("Width".into(), Tag::Short(clip.size[0] as i16));
    s.insert("Height".into(), Tag::Short(clip.size[1] as i16));
    s.insert("Length".into(), Tag::Short(clip.size[2] as i16));
    s.insert("Materials".into(), Tag::String("Alpha".into()));
    s.insert("Blocks".into(), Tag::ByteArray(blocks));
    s.insert("Data".into(), Tag::ByteArray(data));
    if any_add {
        s.insert("AddBlocks".into(), Tag::ByteArray(add));
    }
    let entities = if clip.edition == "console" {
        clip.entities
            .iter()
            .map(|(p, e)| {
                let mut c = e.clone();
                c.insert("Pos".into(), double_list(*p));
                c
            })
            .collect()
    } else {
        Vec::new()
    };
    s.insert("Entities".into(), compounds(entities));
    s.insert("TileEntities".into(), compounds(bes));
    NamedTag { name: "Schematic".into(), tag: Tag::Compound(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Clipboard {
        let mut c = empty([3, 2, 4], "java");
        c.palette.push(BlockState::new("minecraft:stone"));
        c.palette.push(BlockState::new("minecraft:oak_stairs").with("facing", "east").with("half", "bottom").with("shape", "straight").with("waterlogged", "false"));
        c.palette.push(BlockState::new("minecraft:chest").with("facing", "north").with("type", "single").with("waterlogged", "false"));
        for (i, b) in c.blocks.iter_mut().enumerate() {
            *b = (i % 4) as u32;
        }
        let mut chest = Compound::new();
        chest.insert("id".into(), Tag::String("minecraft:chest".into()));
        chest.insert("CustomName".into(), Tag::String("\"Loot\"".into()));
        c.block_entities.push(([2, 0, 0], chest));
        c
    }

    /// Each format written and read back gives the same blocks (as each edition names them);
    /// the Java ones keep the chest's contents too.
    #[test]
    fn every_format_reads_back_what_it_wrote() {
        let clip = sample();
        for f in Format::ALL {
            let bytes = write(&clip, f).unwrap();
            let back = read(&bytes, f).unwrap_or_else(|e| panic!("{f:?}: {e}"));
            assert_eq!(back.size, clip.size, "{f:?}: size");
            for y in 0..clip.size[1] {
                for z in 0..clip.size[2] {
                    for x in 0..clip.size[0] {
                        let (a, b) = (clip.get([x, y, z]), back.get([x, y, z]));
                        assert_eq!(a.name, b.name, "{f:?}: block at {x} {y} {z}");
                        if a.name.ends_with("stairs") {
                            assert_eq!(a.properties.get("facing"), b.properties.get("facing"), "{f:?}: stair facing");
                        }
                    }
                }
            }
            if matches!(f, Format::Sponge | Format::Structure) {
                assert_eq!(back.block_entities.len(), 1, "{f:?}: the chest's data");
                assert_eq!(back.block_entities[0].0, [2, 0, 0]);
                assert_eq!(back.block_entities[0].1.get("CustomName"), Some(&Tag::String("\"Loot\"".into())));
            }
        }
    }

    #[test]
    fn block_state_strings_round_trip() {
        let s = parse_state("minecraft:oak_stairs[facing=north,half=top]");
        assert_eq!(s.properties["half"], "top");
        assert_eq!(parse_state(&state_string(&s)), s);
        assert_eq!(parse_state("stone").name, "minecraft:stone");
    }
}
