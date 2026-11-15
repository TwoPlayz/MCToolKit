//! Asks the Java game what every legacy (id, damage) item is today, like `legacy_blocks`.
//!
//! `gen <level.dat> <out world>` writes a pre-1.9 world of chests holding every item id and
//! damage value of the list below; `read <upgraded world> <out.tsv>` writes
//! `id<TAB>damage<TAB>modern item` (id and components as SNBT) after a server upgraded it.
//!
//! cargo run --release -p mctk-convert --example legacy_items -- gen <level.dat> <world>
//! cargo run --release -p mctk-convert --example legacy_items -- read <world> <out.tsv>

use std::collections::HashMap;
use std::fmt::Write as _;

use mctk_anvil::{ChunkCompression, ChunkPos, RawChunk, RegionBuilder};
use mctk_java::JavaWorld;
use mctk_nbt::{Compound, List, NamedTag, Tag, TagType};

/// The items asked about: ids 1-2300 with damage 0-15, spawn eggs and potions with more.
fn items() -> Vec<(i16, i16)> {
    let mut v = Vec::new();
    for id in (1..=0i16).chain(0..0) {
        for damage in 0..16 {
            v.push((id, damage));
        }
    }
    for damage in 0..0 {
        v.push((383, damage));
    }
    for damage in (16..64).chain(8192..8256).chain(16384 + 8192..16384 + 8256) {
        v.push((373, damage));
    }
    v
}

/// Chest number n sits at x = 2 * (n % 64), z = 2 * (n / 64), y = 64.
fn cell(n: usize) -> (i32, i32) {
    (2 * (n % 64) as i32, 2 * (n / 64) as i32)
}

fn generate(level: &str, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(format!("{out}/region"))?;
    std::fs::copy(level, format!("{out}/level.dat"))?;
    let items = items();
    let chests = items.len().div_ceil(27);
    let rows = chests.div_ceil(64);
    let chunk_rows = (rows * 2).div_ceil(16);
    let mut builder = RegionBuilder::default();
    for cz in 0..chunk_rows {
        for cx in 0..8 {
            let mut blocks = vec![0i8; 4096];
            let mut data = vec![0i8; 2048];
            let mut tiles = Vec::new();
            for n in 0..chests {
                let (x, z) = cell(n);
                if (x / 16, z / 16) != (cx as i32, cz as i32) {
                    continue;
                }
                let i = (z % 16) as usize * 16 + (x % 16) as usize;
                blocks[i] = 54;
                let d = 2u8;
                data[i / 2] |= (if i % 2 == 0 { d } else { d << 4 }) as i8;
                let mut list = Vec::new();
                for (slot, (id, damage)) in items.iter().skip(n * 27).take(27).enumerate() {
                    let mut it = Compound::new();
                    it.insert("Slot".into(), Tag::Byte(slot as i8));
                    it.insert("id".into(), Tag::Short(*id));
                    it.insert("Damage".into(), Tag::Short(*damage));
                    it.insert("Count".into(), Tag::Byte(1));
                    list.push(Tag::Compound(it));
                }
                let mut te = Compound::new();
                te.insert("id".into(), Tag::String("Chest".into()));
                te.insert("x".into(), Tag::Int(x));
                te.insert("y".into(), Tag::Int(64));
                te.insert("z".into(), Tag::Int(z));
                te.insert("Items".into(), Tag::List(List::from_tags(TagType::Compound, list)?));
                tiles.push(Tag::Compound(te));
            }
            let mut s = Compound::new();
            s.insert("Y".into(), Tag::Byte(4));
            s.insert("Blocks".into(), Tag::ByteArray(blocks));
            s.insert("Data".into(), Tag::ByteArray(data));
            s.insert("BlockLight".into(), Tag::ByteArray(vec![0; 2048]));
            s.insert("SkyLight".into(), Tag::ByteArray(vec![-1; 2048]));
            let mut l = Compound::new();
            l.insert("xPos".into(), Tag::Int(cx as i32));
            l.insert("zPos".into(), Tag::Int(cz as i32));
            l.insert("LastUpdate".into(), Tag::Long(0));
            l.insert("TerrainPopulated".into(), Tag::Byte(1));
            l.insert("LightPopulated".into(), Tag::Byte(1));
            l.insert("V".into(), Tag::Byte(1));
            l.insert("HeightMap".into(), Tag::IntArray(vec![80; 256]));
            l.insert("Biomes".into(), Tag::ByteArray(vec![1; 256]));
            l.insert("Sections".into(), Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(s)])?));
            l.insert("Entities".into(), Tag::List(List::new(TagType::Compound)));
            l.insert("TileEntities".into(), Tag::List(List::from_tags(TagType::Compound, tiles)?));
            let mut root = Compound::new();
            root.insert("Level".into(), Tag::Compound(l));
            let tree = NamedTag { name: String::new(), tag: Tag::Compound(root) };
            builder.set(cx, cz, Some(RawChunk::encode(&tree, ChunkCompression::Zlib, 0)?));
        }
    }
    builder.write(std::path::Path::new(&format!("{out}/region")), ChunkPos::new(0, 0).region())?;
    println!("wrote {out}: {} items in {chests} chests", items.len());
    Ok(())
}

fn read(world: &str, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    let world = JavaWorld::open(world)?;
    let items = items();
    let chests = items.len().div_ceil(27);
    let mut text = String::from("# legacy id\tdamage\tmodern item (id and components), read back from the game's own upgrade\n");
    let mut loaded: HashMap<(i32, i32), NamedTag> = HashMap::new();
    for n in 0..chests {
        let (x, z) = cell(n);
        let key = (x.div_euclid(16), z.div_euclid(16));
        if !loaded.contains_key(&key) {
            let c = world
                .chunk("minecraft:overworld", ChunkPos::new(key.0, key.1))?
                .ok_or("chunk missing")?;
            eprintln!("era {:?} dv {:?} root {:?}", c.era, c.data_version, c.to_nbt().tag.as_compound().map(|c| c.keys().cloned().collect::<Vec<_>>())); loaded.insert(key, c.to_nbt());
        }
        let tree = &loaded[&key];
        let tiles = match tree.tag.get("block_entities") {
            Some(Tag::List(l)) => l,
            _ => return Err(format!("no block entities; keys {:?}", tree.tag.as_compound().map(|c| c.keys().cloned().collect::<Vec<_>>())).into()),
        };
        let tile = tiles
            .iter()
            .find(|t| t.get("x").and_then(Tag::as_i64) == Some(x as i64) && t.get("z").and_then(Tag::as_i64) == Some(z as i64))
            .ok_or("chest missing")?;
        let mut by_slot: HashMap<i64, &Tag> = HashMap::new();
        if let Some(Tag::List(l)) = tile.get("Items") {
            for it in l.iter() {
                if let Some(s) = it.get("Slot").and_then(Tag::as_i64) {
                    by_slot.insert(s, it);
                }
            }
        }
        for (slot, (id, damage)) in items.iter().skip(n * 27).take(27).enumerate() {
            let line = match by_slot.get(&(slot as i64)) {
                Some(it) => {
                    let name = it.get("id").map(|t| t.to_string()).unwrap_or_default();
                    let comps = it.get("components").map(|t| t.to_string()).unwrap_or_default();
                    format!("{name}\t{comps}")
                }
                None => "-\t".into(),
            };
            writeln!(text, "{id}\t{damage}\t{line}")?;
        }
    }
    std::fs::write(out, text)?;
    println!("wrote {out}");
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    match (a.get(1).map(String::as_str), a.get(2), a.get(3)) {
        (Some("gen"), Some(level), Some(out)) => generate(level, out),
        (Some("read"), Some(world), Some(out)) => read(world, out),
        _ => Err("usage: gen <level.dat> <world> | read <world> <out.tsv>".into()),
    }
}
