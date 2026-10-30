//! Counts the block entities, entities and item stacks in a pre-1.13 Java world, and how
//! the converter handles them.
//!
//! cargo run --release -p mctk-convert --example legacy_survey -- <legacy java world>

use std::collections::BTreeMap;
use std::path::Path;

use mctk_anvil::Region;
use mctk_java::JavaWorld;
use mctk_nbt::Tag;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let src = std::env::args().nth(1).ok_or("usage: <world>")?;
    let java = JavaWorld::open(Path::new(&src))?;
    let mut tiles: BTreeMap<String, usize> = BTreeMap::new();
    let mut ents: BTreeMap<String, usize> = BTreeMap::new();
    let mut items: BTreeMap<String, usize> = BTreeMap::new();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    for id in ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"] {
        let Some(dim) = java.dimension(id) else { continue };
        for rpos in dim.regions()? {
            let region = Region::open(dim.region_dir().join(rpos.file_name()))?;
            for (lx, lz) in region.chunks().collect::<Vec<_>>() {
                let root = region.read_chunk(lx, lz)?.unwrap();
                let Some(level) = root.tag.get("Level") else { continue };
                let mut note_items = |list: Option<&Tag>| {
                    for i in list.and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()) {
                        let name = match i.get("id") {
                            Some(Tag::String(s)) => s.clone(),
                            Some(t) => format!("#{}", t.as_i64().unwrap_or(-1)),
                            None => "?".into(),
                        };
                        *items.entry(name).or_default() += 1;
                        if let Some(Tag::Compound(t)) = i.get("tag") {
                            for k in t.keys() {
                                *tags.entry(k.clone()).or_default() += 1;
                            }
                        }
                    }
                };
                for t in level.get("TileEntities").and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()) {
                    *tiles.entry(t.get("id").and_then(Tag::as_str).unwrap_or("?").into()).or_default() += 1;
                    note_items(t.get("Items"));
                }
                for e in level.get("Entities").and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()) {
                    *ents.entry(e.get("id").and_then(Tag::as_str).unwrap_or("?").into()).or_default() += 1;
                    note_items(e.get("Items"));
                    note_items(e.get("Inventory"));
                }
            }
        }
    }
    println!("tile entities: {tiles:?}");
    println!("entities: {ents:?}");
    println!("items: {items:?}");
    println!("item tags: {tags:?}");
    Ok(())
}
