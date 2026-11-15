//! `cargo run -p mctk-replace --example bents -- <region-folder> <chunk-x> <chunk-z> [key]`
//! Prints each element of a chunk's block entity list (`region/` folders, key
//! `block_entities`) or entity list (`entities/` folders, key `Entities`), one per line.

use mctk_anvil::{ChunkPos, Region};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (dir, cx, cz) = (&a[0], a[1].parse::<i32>().unwrap(), a[2].parse::<i32>().unwrap());
    let pos = ChunkPos { x: cx, z: cz };
    let file = std::path::Path::new(dir).join(pos.region().file_name());
    let region = Region::open(file).expect("region");
    let (lx, lz) = pos.local();
    let nbt = region.read_chunk(lx, lz).expect("read").expect("chunk absent");
    let key = a.get(3).map(String::as_str).unwrap_or(if dir.ends_with("entities") { "Entities" } else { "block_entities" });
    println!("DataVersion = {:?}", nbt.tag.get("DataVersion").map(|t| t.to_string()));
    let Some(mctk_nbt::Tag::List(list)) = nbt.tag.get(key) else {
        println!("no list {key}; keys: {:?}", nbt.tag.as_compound().unwrap().keys().collect::<Vec<_>>());
        return;
    };
    for e in list.iter() {
        println!("{e}\n");
    }
}
