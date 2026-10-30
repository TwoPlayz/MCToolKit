//! Survey of block entities in a Java and a Bedrock world (ids and one example of each).
//!
//! cargo run --release -p mctk-convert --example be_survey -- <java world> <bedrock world>

use std::collections::BTreeMap;

use mctk_anvil::Region;
use mctk_bedrock::{BedrockWorld, ChunkTag, Key};
use mctk_java::JavaWorld;
use mctk_nbt::{Flavor, Tag};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut java: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let world = JavaWorld::open(args.get(1).ok_or("java world")?)?;
    for dim in &world.dimensions {
        for rpos in dim.regions()? {
            let region = Region::open(dim.region_dir().join(rpos.file_name()))?;
            for (lx, lz) in region.chunks().collect::<Vec<_>>() {
                let Some(root) = region.read_chunk(lx, lz)? else { continue };
                let list = root.tag.get("block_entities").and_then(Tag::as_list);
                for t in list.into_iter().flat_map(|l| l.iter()) {
                    let id = t.get("id").and_then(Tag::as_str).unwrap_or("?").to_string();
                    let e = java.entry(id).or_insert((0, String::new()));
                    e.0 += 1;
                    if e.1.is_empty() || (e.1.len() < 300 && format!("{t:?}").len() > e.1.len()) {
                        e.1 = format!("{t:?}");
                    }
                }
            }
        }
    }
    println!("== Java");
    for (id, (n, ex)) in &java {
        println!("{n:6} {id}\n       {}", &ex[..ex.len().min(900)]);
    }
    let bw = BedrockWorld::open(args.get(2).ok_or("bedrock world")?)?;
    let mut bed: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for (k, v) in &bw.db.entries {
        if let Key::Chunk(c) = Key::decode(k) {
            if c.tag == ChunkTag::BlockEntity {
                let mut s = v.as_slice();
                while !s.is_empty() {
                    let Ok(root) = mctk_nbt::read_root(&mut s, Flavor::Bedrock) else { break };
                    let id = root.tag.get("id").and_then(Tag::as_str).unwrap_or("?").to_string();
                    let e = bed.entry(id).or_insert((0, String::new()));
                    e.0 += 1;
                    let d = format!("{:?}", root.tag);
                    if e.1.len() < d.len() && d.len() < 1500 { e.1 = d; }
                }
            }
        }
    }
    println!("== Bedrock");
    for (id, (n, ex)) in &bed {
        println!("{n:6} {id}\n       {}", &ex[..ex.len().min(900)]);
    }
    Ok(())
}
