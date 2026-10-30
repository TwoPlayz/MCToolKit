//! Converts a Java world to Bedrock, then checks the actors in the result: every id in a
//! chunk's list has an actor, every actor is listed exactly once, definitions (when there are any) start with
//! the actor's own identifier, and the player record decodes.
//!
//! cargo run --release -p mctk-convert --example verify_actors -- <java world> <out folder>

use std::collections::{BTreeMap, HashMap, HashSet};

use mctk_bedrock::{BedrockWorld, Key};
use mctk_nbt::{Flavor, Tag};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (src, out) = (args.get(1).ok_or("java world")?, args.get(2).ok_or("out")?);
    let report = mctk_convert::java_to_bedrock(std::path::Path::new(src), std::path::Path::new(out))?;
    println!("converted entities: {:?}", report.converted_entities);
    let skipped: BTreeMap<_, _> = report.skipped.iter().filter(|(k, _)| k.starts_with("entities")).collect();
    println!("skipped entities: {skipped:?}");
    for n in &report.notes { println!("note: {n}"); }

    let w = BedrockWorld::open(out)?;
    let mut actors: HashMap<Vec<u8>, mctk_nbt::Compound> = HashMap::new();
    let mut listed: Vec<(i32, i32, Vec<u8>)> = Vec::new();
    let mut digest_versions = 0;
    let mut player = None;
    for (k, v) in &w.db.entries {
        match Key::decode(k) {
            Key::Actor(uid) => {
                let root = mctk_nbt::from_bytes(v, Flavor::Bedrock)?;
                if let Tag::Compound(c) = root.tag { actors.insert(uid, c); }
            }
            Key::ActorDigest { x, z, .. } => listed.push((x, z, v.clone())),
            Key::Chunk(c) if c.tag == mctk_bedrock::ChunkTag::ActorDigestVersion => digest_versions += 1,
            Key::Other(n) if n == b"~local_player" => player = Some(mctk_nbt::from_bytes(v, Flavor::Bedrock)?),
            _ => {}
        }
    }
    let mut seen = HashSet::new();
    let mut bad = 0;
    for (x, z, list) in &listed {
        for uid in list.chunks_exact(8) {
            if !seen.insert(uid.to_vec()) { bad += 1; println!("listed twice: {uid:?}"); }
            let Some(a) = actors.get(uid) else { bad += 1; println!("missing actor {uid:?}"); continue };
            let pos: Vec<f32> = a.get("Pos").and_then(Tag::as_list).unwrap().iter().map(|t| if let Tag::Float(f) = t { *f } else { f32::NAN }).collect();
            let cx = (pos[0].floor() as i32).div_euclid(16);
            let cz = (pos[2].floor() as i32).div_euclid(16);
            if (cx, cz) != (*x, *z) { bad += 1; println!("actor in wrong chunk list: {pos:?} vs {x},{z}"); }
            let ident = a.get("identifier").and_then(Tag::as_str).unwrap_or("");
            let first = a.get("definitions").and_then(Tag::as_list).and_then(|l| l.get(0)).and_then(Tag::as_str).unwrap_or("");
            if a.contains_key("definitions") && first != format!("+{ident}") { bad += 1; println!("definitions mismatch for {ident}: {first}"); }
        }
    }
    let unlisted = actors.keys().filter(|k| !seen.contains(*k)).count();
    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    for a in actors.values() { *by_type.entry(a.get("identifier").and_then(Tag::as_str).unwrap_or("?").into()).or_default() += 1; }
    println!("{} actors in {} chunk lists ({} ActorDigestVersion records), {unlisted} unlisted, {bad} problems", actors.len(), listed.len(), digest_versions);
    println!("{by_type:?}");
    match player {
        Some(p) => println!("player: Pos {:?}, inventory slots {}", p.tag.get("Pos"), p.tag.get("Inventory").and_then(Tag::as_list).map_or(0, |l| l.len())),
        None => println!("no player record"),
    }
    if bad > 0 || unlisted > 0 { return Err("actor check failed".into()); }
    Ok(())
}
