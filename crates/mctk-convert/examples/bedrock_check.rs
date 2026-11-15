//! Summarises the chunk records of a converted Bedrock world: subchunk counts, biome storages and ids.
//!
//! cargo run --release -p mctk-convert --example bedrock_check -- <world>
use std::collections::{BTreeMap, BTreeSet};

use mctk_bedrock::{BedrockWorld, Data3D};
use mctk_bedrock::BiomeStorage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let world = BedrockWorld::open(std::env::args().nth(1).ok_or("world")?)?;
    let mut storages: BTreeMap<usize, usize> = BTreeMap::new();
    let mut sub: BTreeMap<usize, usize> = BTreeMap::new();
    let mut ids = BTreeSet::new();
    let mut tags: BTreeMap<u8, usize> = BTreeMap::new();
    let mut per_chunk: BTreeMap<(Vec<u8>, ), usize> = BTreeMap::new();
    let mut other: BTreeMap<String, usize> = BTreeMap::new();
    let registry = mctk_registry::builtin::bedrock("1.26.61.1").unwrap();
    let mut bad: BTreeMap<String, usize> = BTreeMap::new();
    for (k, _) in &world.db.entries {
        if !matches!(k.len(), 9 | 10 | 13 | 14) {
            let name: String = k.iter().take(12).map(|b| if b.is_ascii_graphic() { *b as char } else { '.' }).collect();
            *other.entry(format!("{name} len {}", k.len())).or_default() += 1;
        }
    }
    println!("non-chunk records {other:?}");
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for (k, v) in &world.db.entries {
        if k.starts_with(b"actorprefix") {
            if let Ok(n) = mctk_nbt::from_bytes(v, mctk_nbt::Flavor::Bedrock) {
                let id = n.tag.get("identifier").and_then(mctk_nbt::Tag::as_str).unwrap_or("?").to_string();
                *kinds.entry(id).or_default() += 1;
            }
        }
    }
    println!("actors {kinds:?}");
    if let Some((k, v)) = world.db.entries.iter().find(|(k, _)| k.starts_with(b"actorprefix")) {
        println!("ACTOR KEY {k:02x?}");
        if let Ok(n) = mctk_nbt::from_bytes(v, mctk_nbt::Flavor::Bedrock) {
            println!("ACTOR {}", n.tag);
        }
    }
    for (k, v) in world.db.entries.iter().filter(|(k, _)| k.starts_with(b"digp")) {
        let x = i32::from_le_bytes(k[4..8].try_into().unwrap());
        let z = i32::from_le_bytes(k[8..12].try_into().unwrap());
        if (x, z) == (-15, 11) {
            println!("DIGP -15,11 = {v:02x?}");
        }
    }
    for (_, v) in world.db.entries.iter().filter(|(k, _)| k.starts_with(b"actorprefix")).take(400) {
        if let Ok(n) = mctk_nbt::from_bytes(v, mctk_nbt::Flavor::Bedrock) {
            if n.tag.get("identifier").and_then(mctk_nbt::Tag::as_str) == Some("minecraft:pig") {
                println!("PIG {}", n.tag.get("Pos").map(|t| t.to_string()).unwrap_or_default());
            }
        }
    }
    for (k, v) in world.db.entries.iter().filter(|(k, _)| k.starts_with(b"actorprefix")) {
        if let Ok(n) = mctk_nbt::from_bytes(v, mctk_nbt::Flavor::Bedrock) {
            if let Some(mctk_nbt::Tag::List(p)) = n.tag.get("Pos") {
                let p: Vec<f64> = p.iter().filter_map(mctk_nbt::Tag::as_f64).collect();
                if p.len() == 3 && (-241.0..-224.0).contains(&p[0]) && (175.0..192.0).contains(&p[2]) {
                    println!("NEARX {k:02x?} {}", n.tag.get("identifier").map(|t| t.to_string()).unwrap_or_default());
                }
            }
        }
    }
    if let Some((k, v)) = world.db.entries.iter().find(|(k, _)| k.starts_with(b"digp")) {
        println!("DIGP KEY {k:02x?} = {v:02x?}");
    }
    if world.db.entries.len() < 50 {
        for (k, v) in &world.db.entries {
            println!("{:02x?} ({} bytes)", k, v.len());
        }
    }
    for (k, v) in &world.db.entries {
        if !matches!(k.len(), 9 | 10 | 13 | 14) {
            continue;
        }
        let dim_off = if k.len() >= 13 { 4 } else { 0 };
        let tag = k[8 + dim_off];
        *tags.entry(tag).or_default() += 1;
        if tag == 43 {
            let d = Data3D::parse(v)?;
            if world.db.entries.len() < 400 {
                println!("heightmap {:?}", &d.heightmap[..16]);
                for (i, b) in d.biomes.iter().enumerate() {
                    match b { BiomeStorage::Palettized { palette, bits, flag, indices } => println!("biome storage {i}: bits {bits} flag {flag} palette {palette:?} distinct idx {}", indices.iter().collect::<std::collections::BTreeSet<_>>().len()), other => println!("biome storage {i}: {other:?}") }
                }
            }
            *storages.entry(d.biomes.len()).or_default() += 1;
            for b in &d.biomes {
                if let BiomeStorage::Palettized { palette, .. } = b {
                    ids.extend(palette.iter().copied());
                }
            }
        }
        if tag == 47 {
            if let Ok(sub) = mctk_bedrock::SubChunk::parse(v) {
                if let mctk_bedrock::SubChunkBody::Palettized(stores) = &sub.body {
                    for st in stores {
                        for entry in &st.palette {
                            let name = entry.tag.get("name").and_then(mctk_nbt::Tag::as_str).unwrap_or("?").to_string();
                            let props: Vec<(String, String)> = match entry.tag.get("states") {
                                Some(mctk_nbt::Tag::Compound(c)) => c
                                    .iter()
                                    .map(|(k, v)| {
                                        let text = match v {
                                            mctk_nbt::Tag::Byte(n) => format!("b:{n}"),
                                            mctk_nbt::Tag::Int(n) => format!("i:{n}"),
                                            mctk_nbt::Tag::String(s) => format!("s:{s}"),
                                            other => format!("?:{other}"),
                                        };
                                        (k.clone(), text)
                                    })
                                    .collect(),
                                _ => Vec::new(),
                            };
                            let r = registry.check_block(&name, props.iter().map(|(k, v)| (k.as_str(), v.as_str())));
                            if r != mctk_registry::BlockCheck::Exact {
                                *bad.entry(format!("{name} {props:?} -> {r:?}")).or_default() += 1;
                            }
                        }
                    }
                }
            }
            *per_chunk.entry((k[..8 + dim_off].to_vec(),)).or_default() += 1;
        }
    }
    for n in per_chunk.values() {
        *sub.entry(*n).or_default() += 1;
    }
    println!("block states the game does not know: {}", bad.len());
    for (k, n) in bad.iter().take(30) {
        println!("  {n:>6}  {k}");
    }
    println!("record tags {tags:?}");
    println!("biome storages per chunk (count -> chunks) {storages:?}");
    println!("subchunks per chunk (count -> chunks) {sub:?}");
    println!("biome ids {ids:?}");
    Ok(())
}
