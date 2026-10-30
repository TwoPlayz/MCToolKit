//! Read-only validation of a Bedrock world: parses and re-encodes every subchunk, Data3D
//! record and level.dat, rewrites the whole database in memory and reloads it, and prints
//! a block census.
//!
//! cargo run --release -p mctk-bedrock --example verify_world -- <world folder or .mcworld>

use std::collections::{BTreeMap, HashMap};

use mctk_bedrock::{BedrockWorld, ChunkTag, Data3D, Key, SubChunk, SubChunkBody};
use mctk_leveldb::{Db, WriteOptions, build_db_files};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: verify_world <world>")?;
    let world = BedrockWorld::open(&path)?;
    println!(
        "{:?}, last opened with {:?}, {} keys",
        world.level.level_name(),
        world.level.last_opened_version(),
        world.db.len()
    );

    // level.dat
    let original_level = world.other_files.get("level.dat").ok_or("level.dat")?;
    println!(
        "level.dat round trip: {}",
        if &world.level.encode()? == original_level {
            "identical"
        } else {
            "DIFFERENT"
        }
    );

    let (mut subchunks, mut sub_bad, mut data3d, mut d3_bad, mut layers) =
        (0, 0, 0, 0, BTreeMap::new());
    let mut unknown_tags: BTreeMap<u8, usize> = BTreeMap::new();
    let mut census: HashMap<String, u64> = HashMap::new();
    for (k, v) in &world.db.entries {
        let Key::Chunk(key) = Key::decode(k) else {
            continue;
        };
        match key.tag {
            ChunkTag::SubChunkPrefix => {
                subchunks += 1;
                let sub = SubChunk::parse(v)?;
                if sub.y != key.subchunk && sub.y.is_some() {
                    eprintln!("Y mismatch in {key:?}");
                }
                if &sub.encode()? != v {
                    sub_bad += 1;
                    if sub_bad <= 3 {
                        eprintln!("SUBCHUNK MISMATCH {key:?}");
                    }
                }
                if let SubChunkBody::Palettized(storages) = &sub.body {
                    *layers.entry(storages.len()).or_insert(0) += 1;
                    if let Some(blocks) = storages.first() {
                        for &i in &blocks.indices {
                            *census
                                .entry(blocks.name(i as usize).unwrap_or("?").to_string())
                                .or_insert(0) += 1;
                        }
                    }
                }
            }
            ChunkTag::Data3D => {
                data3d += 1;
                if &Data3D::parse(v)?.encode() != v {
                    d3_bad += 1;
                }
            }
            ChunkTag::Unknown(b) => *unknown_tags.entry(b).or_insert(0) += 1,
            _ => {}
        }
    }
    println!("subchunks: {subchunks}, re-encode mismatches {sub_bad}, storage layers {layers:?}");
    println!("Data3D: {data3d}, re-encode mismatches {d3_bad}");
    println!("unknown chunk tags kept verbatim: {unknown_tags:?}");

    // Whole-database rewrite.
    let files: HashMap<String, Vec<u8>> =
        build_db_files(&world.db.entries, WriteOptions::default())?
            .into_iter()
            .collect();
    let size: usize = files.values().map(Vec::len).sum();
    let reloaded = Db::from_files(&files)?;
    println!(
        "database rewrite: {} files, {size} bytes, contents {}",
        files.len(),
        if reloaded.entries == world.db.entries {
            "identical"
        } else {
            "DIFFERENT"
        }
    );

    let mut top: Vec<_> = census.into_iter().collect();
    top.sort_by_key(|e| std::cmp::Reverse(e.1));
    println!("top blocks:");
    for (name, n) in top.iter().take(10) {
        println!("  {n:>9}  {name}");
    }
    if sub_bad + d3_bad > 0 || reloaded.entries != world.db.entries {
        return Err("validation failed".into());
    }
    Ok(())
}
