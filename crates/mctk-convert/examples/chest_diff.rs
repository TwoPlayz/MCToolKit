//! `cargo run --release -p mctk-convert --example chest_diff -- <world a> <world b>`
//! Compares the stacks of the chests in two worlds slot by slot (ignoring the order of keys) and
//! prints the ones that differ. Used to check a conversion against what the game itself wrote.
use std::collections::BTreeMap;

use mctk_anvil::Region;
use mctk_nbt::Tag;

fn stacks(world: &str) -> BTreeMap<(i64, i64, i64), Tag> {
    let mut out = BTreeMap::new();
    let dir = std::path::Path::new(world).join("region");
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let Ok(region) = Region::open(entry.path()) else { continue };
        for (lx, lz) in region.chunks() {
            let Ok(Some(raw)) = region.raw_chunk(lx, lz) else { continue };
            let Ok(nbt) = raw.nbt() else { continue };
            let root = nbt.tag.get("Level").unwrap_or(&nbt.tag);
            let Some(list) = root
                .get("block_entities")
                .or_else(|| root.get("TileEntities"))
                .and_then(|t| t.as_list())
            else {
                continue;
            };
            for be in list.iter() {
                if be.get("id").and_then(|i| i.as_str()) != Some("minecraft:chest") {
                    continue;
                }
                let z = be.get("z").and_then(|t| t.as_i64()).unwrap_or(0);
                for s in be.get("Items").and_then(|t| t.as_list()).into_iter().flat_map(|l| l.iter()) {
                    let slot = s.get("Slot").and_then(|t| t.as_i64()).unwrap_or(0);
                    let mut s = s.clone();
                    if let Tag::Compound(c) = &mut s {
                        c.shift_remove("Slot");
                    }
                    out.insert((z, slot, 0), s);
                }
            }
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (a, b) = (stacks(&args[0]), stacks(&args[1]));
    let mut bad = 0;
    for (k, x) in &a {
        match b.get(k) {
            Some(y) if x == y => {}
            Some(y) => {
                bad += 1;
                println!("--- chest z={} slot {}\n  a: {x}\n  b: {y}", k.0, k.1);
            }
            None => {
                bad += 1;
                println!("--- chest z={} slot {} only in a\n  a: {x}", k.0, k.1);
            }
        }
    }
    for (k, y) in &b {
        if !a.contains_key(k) {
            bad += 1;
            println!("--- chest z={} slot {} only in b\n  b: {y}", k.0, k.1);
        }
    }
    println!("{} stacks in a, {} in b, {bad} differ", a.len(), b.len());
}
