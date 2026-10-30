use mctk_bedrock::{BedrockWorld, ChunkTag, Key};
use std::collections::BTreeMap;
fn main() {
    let w = BedrockWorld::open(std::env::args().nth(1).unwrap()).unwrap();
    let mut vals: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
    let mut digp: Vec<(i32, i32, usize)> = Vec::new();
    for (k, v) in &w.db.entries {
        match Key::decode(k) {
            Key::Chunk(c) if c.tag == ChunkTag::ActorDigestVersion => *vals.entry(v.clone()).or_default() += 1,
            Key::ActorDigest { x, z, .. } => digp.push((x, z, v.len())),
            _ => {}
        }
    }
    println!("ActorDigestVersion values: {vals:?}");
    println!("digp: {digp:?}");
}
