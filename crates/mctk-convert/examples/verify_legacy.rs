//! Converts a pre-1.13 Java world (for example one made by `console-to-java`) to Bedrock and
//! checks every block against the numeric id / data value in the source, through the legacy
//! table and the Java -> Bedrock mapping. Doors and beds are skipped: their state is
//! combined from neighbours / block entities.
//!
//! cargo run --release -p mctk-convert --example verify_legacy -- <legacy java world> <out folder>

use std::collections::HashMap;
use std::path::Path;

use mctk_anvil::Region;
use mctk_bedrock::{BedrockWorld, ChunkTag, Key, SubChunk, SubChunkBody};
use mctk_java::{BlockStorage, Chunk, ChunkEra, JavaWorld};
use mctk_mapping::legacy::legacy;
use mctk_mapping::{BedrockBlock, mappings};
use mctk_nbt::Tag;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let (src, out) = (
        Path::new(args.get(1).ok_or("usage: <legacy java world> <out>")?),
        Path::new(args.get(2).ok_or("usage: <legacy java world> <out>")?),
    );
    let report = mctk_convert::java_to_bedrock(src, out)?;
    println!(
        "chunks {:?}; unmapped {:?}; skipped {:?}",
        report.chunks, report.unmapped, report.skipped
    );
    let world = BedrockWorld::open(out)?;
    let mut subs: HashMap<(i32, i32, i32, i8), Vec<mctk_bedrock::BlockStorage>> = HashMap::new();
    for (k, v) in &world.db.entries {
        if let Key::Chunk(c) = Key::decode(k) {
            if c.tag == ChunkTag::SubChunkPrefix {
                if let SubChunkBody::Palettized(s) = SubChunk::parse(v)?.body {
                    subs.insert((c.dimension.id(), c.x, c.z, c.subchunk.unwrap_or(0)), s);
                }
            }
        }
    }
    let m = mappings();
    let java = JavaWorld::open(src)?;
    let (mut blocks, mut wrong, mut legacy_chunks) = (0u64, 0u64, 0u64);
    let mut by_kind: HashMap<String, u64> = HashMap::new();
    for (id, bdim) in [("minecraft:overworld", 0), ("minecraft:the_nether", 1), ("minecraft:the_end", 2)] {
        let Some(dim) = java.dimension(id) else { continue };
        for rpos in dim.regions()? {
            let region = Region::open(dim.region_dir().join(rpos.file_name()))?;
            for (lx, lz) in region.chunks().collect::<Vec<_>>() {
                let root = region.read_chunk(lx, lz)?.unwrap();
                let chunk = Chunk::from_nbt(root)?;
                if chunk.era != ChunkEra::Legacy {
                    continue;
                }
                legacy_chunks += 1;
                for s in &chunk.sections {
                    let Some(BlockStorage::Legacy { ids, data }) = &s.blocks else { continue };
                    for y in 0..16 {
                        for z in 0..16 {
                            for x in 0..16 {
                                let i = mctk_java::block_index(x, y, z);
                                let (bid, d) = (ids[i], data[i]);
                                if [26, 64, 71, 193, 194, 195, 196, 197].contains(&bid) {
                                    continue;
                                }
                                let Some(state) = legacy().block(bid, d) else { continue };
                                let expect = m.java_to_bedrock(state).cloned().unwrap();
                                let got = match subs.get(&(bdim, chunk.x, chunk.z, s.y)) {
                                    Some(st) => {
                                        let b = mctk_bedrock::block_index(x, y, z);
                                        BedrockBlock::from_nbt(&st[0].palette[st[0].indices[b] as usize]).unwrap()
                                    }
                                    None => BedrockBlock {
                                        name: "minecraft:air".into(),
                                        states: Default::default(),
                                    },
                                };
                                blocks += 1;
                                // Sections of only air are not written.
                                let all_air = ids.iter().all(|&b| b == 0);
                                if !(all_air && got.name == "minecraft:air") && got != expect {
                                    wrong += 1;
                                    *by_kind.entry(format!("{bid}:{d} {} -> {}", expect.name, got.name)).or_insert(0) += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    println!("{legacy_chunks} legacy chunks, checked {blocks} blocks: {wrong} wrong");
    let mut top: Vec<_> = by_kind.into_iter().collect();
    top.sort_by_key(|x| std::cmp::Reverse(x.1));
    for (k, n) in top.iter().take(10) {
        println!("  {n:8} x {k}");
    }
    let _ = Tag::Byte(0);
    if wrong > 0 {
        return Err("legacy mismatch".into());
    }
    Ok(())
}
