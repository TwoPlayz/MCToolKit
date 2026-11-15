//! Asks the Bedrock server what every legacy (numeric id, data value) block is today.
//!
//! `legacy-gen <template world> <out world>` copies a world the server made and adds, far from
//! its spawn, 256 chunks written in the old pre-palette format: chunk `N` holds block id `N` with
//! each of the data values 0-15, each alone in the air. The server upgrades them when it loads
//! them (a ticking area makes it) and saves the result; `legacy-read <world> <out.tsv>` then
//! reads the states it made: `id<TAB>data<TAB>name<TAB>states` with typed values (`b:1`).
//! Together with the Java server's answer for the same ids (`mctk-convert`'s `legacy_blocks`)
//! this relates the two editions' states through their shared ancestor, with no third-party table.

use std::path::Path;

use mctk_bedrock::{BedrockWorld, ChunkKey, ChunkTag, Dimension, SubChunk, SubChunkBody};
use mctk_nbt::Tag;

/// The chunk that holds block id `n`.
pub fn chunk_of(n: usize) -> (i32, i32) {
    (1000 + (n % 16) as i32, 1000 + (n / 16) as i32)
}

/// Where data value `d` sits in the chunk: x, y, z of its cell.
fn cell(d: usize) -> (usize, usize, usize) {
    ((d % 8) * 2, 2, (d / 8) * 2)
}

fn key(x: i32, z: i32, tag: ChunkTag, sub: Option<i8>) -> Vec<u8> {
    ChunkKey { x, z, dimension: Dimension::Overworld, tag, subchunk: sub }.encode()
}

pub fn generate(template: &Path, out: &Path) -> Result<(), String> {
    let mut world = BedrockWorld::open(template).map_err(|e| e.to_string())?;
    for n in 0..256usize {
        let (cx, cz) = chunk_of(n);
        let mut body = vec![0u8; 10240];
        for d in 0..16usize {
            let (x, y, z) = cell(d);
            let i = x * 256 + z * 16 + y;
            body[i] = n as u8;
            let nibble = &mut body[4096 + i / 2];
            *nibble |= if i % 2 == 0 { d as u8 } else { (d as u8) << 4 };
        }
        // Full sky light so nothing is treated as buried.
        body[4096 + 2048..4096 + 4096].fill(0xff);
        let sub = SubChunk { version: 0, y: None, body: SubChunkBody::Legacy(body) };
        world.db.entries.insert(key(cx, cz, ChunkTag::SubChunkPrefix, Some(0)), sub.encode().map_err(|e| e.to_string())?);
        // Heights and biomes, then the chunk's version (an old one, so the server upgrades it).
        let mut d2 = vec![0u8; 512];
        d2.extend(vec![1u8; 256]);
        world.db.entries.insert(key(cx, cz, ChunkTag::Data2D, None), d2);
        world.db.entries.insert(key(cx, cz, ChunkTag::Version, None), vec![15]);
        world.db.entries.insert(key(cx, cz, ChunkTag::FinalizedState, None), 2i32.to_le_bytes().to_vec());
    }
    world.save_to_folder(out).map_err(|e| e.to_string())
}

pub fn read(world: &Path, out: &Path) -> Result<(), String> {
    let world = BedrockWorld::open(world).map_err(|e| e.to_string())?;
    let mut text = String::from("# legacy id\tdata\tBedrock block\tstates (typed), read back from the server's own upgrade\n");
    let mut missing = 0;
    for n in 0..256usize {
        let (cx, cz) = chunk_of(n);
        let Some(bytes) = world.db.entries.get(&key(cx, cz, ChunkTag::SubChunkPrefix, Some(0))) else {
            missing += 16;
            continue;
        };
        let sub = SubChunk::parse(bytes).map_err(|e| e.to_string())?;
        let Some(storage) = sub.blocks() else {
            missing += 16;
            continue;
        };
        for d in 0..16usize {
            let (x, y, z) = cell(d);
            let slot = storage.indices[mctk_bedrock::block_index(x, y, z)] as usize;
            let entry = &storage.palette[slot];
            let name = entry.tag.get("name").and_then(Tag::as_str).unwrap_or("?");
            let mut states: Vec<String> = match entry.tag.get("states") {
                Some(Tag::Compound(c)) => c
                    .iter()
                    .map(|(k, v)| {
                        let typed = match v {
                            Tag::Byte(b) => format!("b:{b}"),
                            Tag::Int(i) => format!("i:{i}"),
                            Tag::String(s) => format!("s:{s}"),
                            other => format!("?:{other}"),
                        };
                        format!("{k}={typed}")
                    })
                    .collect(),
                _ => Vec::new(),
            };
            states.sort();
            text.push_str(&format!("{n}\t{d}\t{name}\t{}\n", states.join(",")));
        }
    }
    std::fs::write(out, text).map_err(|e| e.to_string())?;
    println!("wrote {} ({} cells missing)", out.display(), missing);
    Ok(())
}
