//! Survey of entities and players in a Java and a Bedrock world.
//!
//! cargo run --release -p mctk-convert --example entity_survey -- <java world> <bedrock world>

use std::collections::BTreeMap;

use mctk_anvil::Region;
use mctk_bedrock::{BedrockWorld, Key};
use mctk_java::JavaWorld;
use mctk_nbt::{Flavor, Tag};

fn show(t: &Tag, max: usize) -> String {
    let s = format!("{t:?}");
    s[..s.len().min(max)].to_string()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let world = JavaWorld::open(args.get(1).ok_or("java world")?)?;
    println!("== Java level.dat Player present: {}", world.level.root.tag.get("Data").and_then(|d| d.get("Player")).is_some());
    if let Some(p) = world.level.root.tag.get("Data").and_then(|d| d.get("Player")) {
        println!("   {}", show(p, 1500));
    }
    let mut java: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for dim in &world.dimensions {
        let Ok(dir) = std::fs::read_dir(dim.entities_dir()) else { continue };
        for f in dir.flatten() {
            let Ok(region) = Region::open(f.path()) else { continue };
            for (lx, lz) in region.chunks().collect::<Vec<_>>() {
                let Some(root) = region.read_chunk(lx, lz)? else { continue };
                for e in root.tag.get("Entities").and_then(Tag::as_list).map(|l| l.iter()).into_iter().flatten() {
                    let id = e.get("id").and_then(Tag::as_str).unwrap_or("?").to_string();
                    let ent = java.entry(id).or_insert((0, String::new()));
                    ent.0 += 1;
                    if ent.1.is_empty() { ent.1 = show(e, 1100); }
                }
            }
        }
    }
    println!("== Java entities");
    for (id, (n, ex)) in &java { println!("{n:5} {id}\n      {ex}"); }

    let bw = BedrockWorld::open(args.get(2).ok_or("bedrock world")?)?;
    println!("== Bedrock keys");
    let mut other: BTreeMap<String, usize> = BTreeMap::new();
    for (k, v) in &bw.db.entries {
        match Key::decode(k) {
            Key::Actor(uid) => {
                println!("actor key uid: {uid:02x?}");
                if let Ok(root) = mctk_nbt::from_bytes(v, Flavor::Bedrock) {
                    println!("actor: {}", show(&root.tag, 6000));
                }
            }
            Key::Other(name) => {
                let n = String::from_utf8_lossy(&name).to_string();
                let n = if n.len() > 40 { n[..40].to_string() } else { n };
                *other.entry(n.clone()).or_default() += 1;
                if n == "~local_player" {
                    if let Ok(root) = mctk_nbt::from_bytes(v, Flavor::Bedrock) {
                        println!("~local_player: {}", show(&root.tag, 9000));
                    }
                }
            }
            _ => {}
        }
    }
    println!("other keys: {other:?}");
    Ok(())
}


