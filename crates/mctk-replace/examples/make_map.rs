//! `cargo run -p mctk-replace --example make_map -- <world-dir>`
//! Writes a 1.20.1 map (`data/map_0.dat`, a recognisable colour pattern) and the map counter
//! (`data/idcounts.dat`) into a classic world, to test the migration of saved data with.

use mctk_nbt::{Compound, Flavor, List, NamedTag, Tag, TagType};

fn write(path: &std::path::Path, data: Compound) {
    use std::io::Write;
    let root = NamedTag {
        name: String::new(),
        tag: Tag::Compound(Compound::from([("data".to_string(), Tag::Compound(data)), ("DataVersion".to_string(), Tag::Int(3465))])),
    };
    let raw = mctk_nbt::to_bytes(&root, Flavor::Java).unwrap();
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&raw).unwrap();
    std::fs::write(path, e.finish().unwrap()).unwrap();
}

fn main() {
    let world = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: make_map <world-dir>"));
    let data = world.join("data");
    std::fs::create_dir_all(&data).unwrap();
    // 128 x 128 colour indices: a diagonal gradient.
    let colors: Vec<i8> = (0..16384).map(|i| (((i % 128) + (i / 128)) % 60 + 4) as i8).collect();
    let banner = Tag::Compound(Compound::from([
        ("Color".to_string(), s("white")),
        ("Name".to_string(), s("{\"text\":\"Home\"}")),
        ("Pos".to_string(), Tag::Compound(Compound::from([("X".to_string(), Tag::Int(10)), ("Y".to_string(), Tag::Int(64)), ("Z".to_string(), Tag::Int(-5))]))),
    ]));
    write(
        &data.join("map_0.dat"),
        Compound::from([
            ("scale".to_string(), Tag::Byte(1)),
            ("dimension".to_string(), s("minecraft:overworld")),
            ("trackingPosition".to_string(), Tag::Byte(1)),
            ("unlimitedTracking".to_string(), Tag::Byte(0)),
            ("locked".to_string(), Tag::Byte(1)),
            ("xCenter".to_string(), Tag::Int(64)),
            ("zCenter".to_string(), Tag::Int(-64)),
            ("colors".to_string(), Tag::ByteArray(colors)),
            ("banners".to_string(), Tag::List(List::from_tags(TagType::Compound, vec![banner]).unwrap())),
            ("frames".to_string(), Tag::List(List::new(TagType::Compound))),
        ]),
    );
    write(&data.join("idcounts.dat"), Compound::from([("map".to_string(), Tag::Int(0))]));
    println!("wrote map_0.dat and idcounts.dat to {}", data.display());
}

fn s(v: &str) -> Tag {
    Tag::String(v.into())
}
