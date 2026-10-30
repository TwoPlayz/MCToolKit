//! Checks the tables against real worlds: every block state in a Java world must map to
//! Bedrock exactly, every state in a Bedrock world must map to Java exactly, and the
//! Bedrock palette `version` numbers are reported.
//!
//! cargo run --release -p mctk-mapping --example coverage -- <java world> <bedrock world>

use std::collections::{BTreeMap, BTreeSet};

use mctk_anvil::Region;
use mctk_bedrock::{BedrockWorld, ChunkTag, Key, SubChunk, SubChunkBody};
use mctk_java::{BlockStorage, JavaWorld};
use mctk_mapping::{BedrockBlock, mappings};
use mctk_nbt::Tag;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let m = mappings();

    let java = JavaWorld::open(&args[1])?;
    let (mut states, mut biomes) = (BTreeSet::new(), BTreeSet::new());
    for dim in &java.dimensions {
        for pos in dim.regions()? {
            let region = Region::open(dim.region_dir().join(pos.file_name()))?;
            for (lx, lz) in region.chunks().collect::<Vec<_>>() {
                let chunk = mctk_java::Chunk::from_nbt(region.read_chunk(lx, lz)?.unwrap())?;
                for s in &chunk.sections {
                    if let Some(BlockStorage::Palette(p)) = &s.blocks {
                        states.extend(p.palette.iter().cloned());
                    }
                    if let Some(b) = &s.biomes {
                        biomes.extend(b.palette.iter().cloned());
                    }
                }
            }
        }
    }
    let missing: Vec<_> = states.iter().filter(|s| !m.is_known_java(s)).collect();
    let missing_biomes: Vec<_> = biomes
        .iter()
        .filter(|b| m.java_biome_to_bedrock(b).is_none())
        .collect();
    println!(
        "Java: {} distinct states, {} not in the table {:?}; {} biomes, unmapped {:?}",
        states.len(),
        missing.len(),
        missing
            .iter()
            .take(10)
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        biomes.len(),
        missing_biomes
    );

    let bedrock = BedrockWorld::open(&args[2])?;
    let mut seen = BTreeMap::new();
    let mut versions = BTreeMap::new();
    let mut biome_ids = BTreeSet::new();
    for (k, v) in &bedrock.db.entries {
        let Key::Chunk(c) = Key::decode(k) else {
            continue;
        };
        match c.tag {
            ChunkTag::SubChunkPrefix => {
                if let SubChunkBody::Palettized(storages) = SubChunk::parse(v)?.body {
                    for s in &storages {
                        for p in &s.palette {
                            if let Some(Tag::Int(ver)) = p.tag.get("version") {
                                *versions.entry(*ver).or_insert(0) += 1;
                            }
                            let b = BedrockBlock::from_nbt(p).ok_or("bad palette entry")?;
                            let known = m.is_known_bedrock(&b.name, &b.states);
                            seen.insert(format!("{} {:?}", b.name, b.states), known);
                        }
                    }
                }
            }
            ChunkTag::Data3D => {
                for b in mctk_bedrock::Data3D::parse(v)?.biomes {
                    if let mctk_bedrock::BiomeStorage::Palettized { palette, .. } = b {
                        biome_ids.extend(palette);
                    }
                }
            }
            _ => {}
        }
    }
    let unknown: Vec<_> = seen.iter().filter(|(_, k)| !**k).map(|(s, _)| s).collect();
    let unmapped_biomes: Vec<_> = biome_ids
        .iter()
        .filter(|&&id| m.bedrock_biome_to_java(id).is_none())
        .collect();
    println!(
        "Bedrock: {} distinct states, {} not in the table {:?}; palette versions {:?}; biome ids {:?}, unmapped {:?}",
        seen.len(),
        unknown.len(),
        unknown.iter().take(10).collect::<Vec<_>>(),
        versions,
        biome_ids,
        unmapped_biomes
    );
    Ok(())
}
