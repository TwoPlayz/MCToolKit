//! `cargo run -p mctk-replace --example status -- <java-world>`
//! Counts chunks per generation `Status` across the overworld.

use mctk_nbt::Tag;
use std::collections::BTreeMap;

fn main() {
    let world = std::env::args().nth(1).expect("usage: status <java-world>");
    let w = mctk_java::JavaWorld::open(&world).expect("open");
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for dim in &w.dimensions {
        for rpos in dim.regions().unwrap() {
            let region = mctk_anvil::Region::open(dim.region_dir().join(rpos.file_name())).unwrap();
            for (lx, lz) in region.chunks() {
                let nbt = region.read_chunk(lx, lz).unwrap().unwrap();
                let status = nbt.tag.get("Status").or_else(|| nbt.tag.get("Level").and_then(|l| l.get("Status")));
                let s = status.and_then(Tag::as_str).unwrap_or("?").to_string();
                *counts.entry(format!("{} {s}", dim.id)).or_insert(0) += 1;
            }
        }
    }
    for (k, v) in counts {
        println!("{v:>6}  {k}");
    }
}
