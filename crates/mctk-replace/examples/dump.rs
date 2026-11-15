//! `cargo run -p mctk-replace --example dump -- <java-world>`
//! Prints the level.dat keys and the structure of one populated chunk, for comparing the
//! storage of two versions.

use mctk_nbt::{Flavor, Tag};

fn short(t: &Tag) -> String {
    let s = t.to_string();
    if s.len() > 110 { format!("{}...", &s[..110]) } else { s }
}

fn keys(label: &str, c: &mctk_nbt::Compound) {
    println!("{label}:");
    for (k, v) in c {
        println!("  {k} = {}", short(v));
    }
}

fn main() {
    let world = std::env::args().nth(1).expect("usage: dump <java-world>");
    let (_, level) = mctk_nbt::read_file(std::path::Path::new(&world).join("level.dat"), Flavor::Java)
        .expect("level.dat");
    let data = level.tag.as_compound().and_then(|c| c.get("Data")).and_then(Tag::as_compound);
    keys("level.dat Data", data.expect("Data"));

    let w = mctk_java::JavaWorld::open(&world).expect("open");
    let dim = w.dimension("minecraft:overworld").expect("overworld");
    for rpos in dim.regions().unwrap() {
        let region = mctk_anvil::Region::open(dim.region_dir().join(rpos.file_name())).unwrap();
        for (lx, lz) in region.chunks() {
            let nbt = region.read_chunk(lx, lz).unwrap().unwrap();
            let top = nbt.tag.as_compound().unwrap().clone();
            let body = top.get("Level").and_then(Tag::as_compound).unwrap_or(&top);
            let sections = body.get("sections").or_else(|| body.get("Sections"));
            let Some(Tag::List(l)) = sections else { continue };
            let status = top.get("Status").or_else(|| body.get("Status")).and_then(Tag::as_str);
            if l.len() < 8 || status != Some("minecraft:full") {
                continue;
            }
            keys("chunk root", &top);
            if let Some(s) = l.iter().find_map(|s| {
                let c = s.as_compound()?;
                (c.get("block_states").is_some() || c.get("Palette").is_some()).then_some(c)
            }) {
                keys("one section", s);
            }
            if let Some(Tag::List(be)) = body.get("block_entities").or_else(|| body.get("TileEntities")) {
                for e in be.iter().take(3) {
                    println!("block entity: {}", short(e));
                }
            }
            return;
        }
    }
}
