//! `cargo run --release -p mctk-convert --example peek -- <world> [limit]`
//! Prints one example of every tile entity and entity id found in a pre-1.18 world's chunks.
use std::collections::BTreeSet;

use mctk_anvil::Region;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = std::path::Path::new(&args[0]).join("region");
    let mut seen = BTreeSet::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let Ok(region) = Region::open(entry.path()) else { continue };
        for (lx, lz) in region.chunks() {
            let Ok(Some(raw)) = region.raw_chunk(lx, lz) else { continue };
            let Ok(nbt) = raw.nbt() else { continue };
            let Some(level) = nbt.tag.get("Level") else { continue };
            for key in ["TileEntities", "Entities"] {
                let Some(list) = level.get(key).and_then(|t| t.as_list()) else { continue };
                for t in list.iter() {
                    let id = t.get("id").and_then(|i| i.as_str()).unwrap_or("?").to_string();
                    if seen.insert(format!("{key}:{id}")) {
                        println!("--- {key} {id}\n{t}");
                    }
                }
            }
        }
    }
}
