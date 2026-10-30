//! Loads a Bedrock world and summarises its database keys; dumps sample record bytes.
//!
//! cargo run --release -p mctk-bedrock --example explore -- <world folder or .mcworld>

use std::collections::BTreeMap;

use mctk_bedrock::{BedrockWorld, ChunkTag, Key};

fn hex(b: &[u8], n: usize) -> String {
    b.iter()
        .take(n)
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: explore <world>")?;
    let world = BedrockWorld::open(&path)?;
    println!(
        "name: {:?} / {:?}  storage version {}  last opened {:?}",
        world.level.level_name(),
        world.level_name_txt,
        world.level.storage_version,
        world.level.last_opened_version()
    );
    println!(
        "db: {} keys, last sequence {}",
        world.db.len(),
        world.db.last_sequence
    );

    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    let mut others = Vec::new();
    let mut versions: BTreeMap<String, usize> = BTreeMap::new();
    for (k, v) in &world.db.entries {
        match Key::decode(k) {
            Key::Chunk(c) => {
                *tags
                    .entry(format!("{:?}/{:?}", c.dimension, c.tag))
                    .or_insert(0) += 1;
                if c.tag == ChunkTag::SubChunkPrefix {
                    *versions
                        .entry(format!(
                            "subchunk v{} storages {}",
                            v[0],
                            v.get(1).copied().unwrap_or(0)
                        ))
                        .or_insert(0) += 1;
                }
                if c.tag == ChunkTag::Version {
                    *versions
                        .entry(format!("chunk version {}", v[0]))
                        .or_insert(0) += 1;
                }
            }
            Key::Actor(_) => *tags.entry("actorprefix".into()).or_insert(0) += 1,
            Key::ActorDigest { .. } => *tags.entry("digp".into()).or_insert(0) += 1,
            Key::Other(name) => others.push((String::from_utf8_lossy(&name).into_owned(), v.len())),
        }
    }
    for (t, n) in &tags {
        println!("  {n:>6}  {t}");
    }
    for (t, n) in &versions {
        println!("  {n:>6}  {t}");
    }
    println!("named keys: {others:?}");

    for (k, v) in &world.db.entries {
        if let Key::Chunk(c) = Key::decode(k) {
            if matches!(c.tag, ChunkTag::SubChunkPrefix | ChunkTag::Data3D) {
                println!("{:?} ({} bytes): {}", c, v.len(), hex(v, 48));
            }
            if c.x == 0 && c.z == 0 && c.tag == ChunkTag::Data3D {
                break;
            }
        }
    }
    Ok(())
}
