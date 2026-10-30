//! Checks double-chest pairing against a real Bedrock world: converts it to Java and back
//! and compares every chest's pair data (`pairx`, `pairz`, `pairlead`) with the original.
//!
//! cargo run --release -p mctk-convert --example verify_chests -- <bedrock world> <scratch dir>

use std::collections::BTreeMap;
use std::path::Path;

use mctk_bedrock::{BedrockWorld, ChunkTag, Key};
use mctk_nbt::{Flavor, Tag};

type Pairs = BTreeMap<(i64, i64, i64), (Option<i64>, Option<i64>, Option<i64>)>;

fn chests(world: &BedrockWorld) -> Pairs {
    let mut out = Pairs::new();
    for (k, v) in &world.db.entries {
        if let Key::Chunk(c) = Key::decode(k) {
            if c.tag != ChunkTag::BlockEntity { continue; }
            let mut s = v.as_slice();
            while !s.is_empty() {
                let Ok(root) = mctk_nbt::read_root(&mut s, Flavor::Bedrock) else { break };
                if root.tag.get("id").and_then(Tag::as_str) != Some("Chest") { continue; }
                let g = |k: &str| root.tag.get(k).and_then(Tag::as_i64);
                out.insert((g("x").unwrap(), g("y").unwrap(), g("z").unwrap()), (g("pairx"), g("pairz"), g("pairlead")));
            }
        }
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (src, scratch) = (args.get(1).ok_or("bedrock world")?, args.get(2).ok_or("scratch")?);
    let java = Path::new(scratch).join("java");
    let back = Path::new(scratch).join("bedrock");
    mctk_convert::bedrock_to_java(Path::new(src), &java)?;
    mctk_convert::java_to_bedrock(&java, &back)?;
    let (before, after) = (chests(&BedrockWorld::open(src)?), chests(&BedrockWorld::open(&back)?));
    let paired = before.values().filter(|(x, _, _)| x.is_some()).count();
    let (mut same, mut missing, mut different) = (0, 0, 0);
    for (pos, orig) in &before {
        match after.get(pos) {
            None => missing += 1,
            Some(now) if now == orig => same += 1,
            Some(now) => {
                // A single chest may or may not carry pair fields; only compare real pairs.
                if orig.0.is_some() || now.0.is_some() {
                    different += 1;
                    println!("differs at {pos:?}: {orig:?} -> {now:?}");
                } else { same += 1 }
            }
        }
    }
    println!("{} chests, {paired} in double chests: {same} identical, {different} different, {missing} not in the round trip", before.len());
    if different > 0 { return Err("pairing differs".into()); }
    Ok(())
}
