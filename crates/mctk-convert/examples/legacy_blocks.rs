//! Asks the Java game what every legacy (id, data) block is today.
//!
//! `gen <level.dat of a pre-1.9 world> <out world>` writes a pre-1.9 world holding every block
//! id 0-255 with every data value, each alone in the air. Let a real server upgrade it
//! (`java -jar server.jar --forceUpgrade`), then
//! `read <upgraded world> <out.tsv>` writes `id<TAB>data<TAB>state` for each block.
//!
//! cargo run --release -p mctk-convert --example legacy_blocks -- gen <level.dat> <world>
//! cargo run --release -p mctk-convert --example legacy_blocks -- read <world> <out.tsv>

use std::fmt::Write as _;

use mctk_anvil::{ChunkCompression, ChunkPos, RawChunk, RegionBuilder};
use mctk_java::JavaWorld;
use mctk_nbt::{Compound, List, NamedTag, Tag, TagType};

/// Block (id, data) number n sits at x = 2 * (n % 64), z = 2 * (n / 64), y = 64.
fn cell(n: usize) -> (i32, i32) {
    (2 * (n % 64) as i32, 2 * (n / 64) as i32)
}

fn generate(level: &str, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(format!("{out}/region"))?;
    std::fs::copy(level, format!("{out}/level.dat"))?;
    let mut builder = RegionBuilder::default();
    for cz in 0..8 {
        for cx in 0..8 {
            let mut blocks = vec![0i8; 4096];
            let mut data = vec![0i8; 2048];
            // Section 4 holds y 64-79; the cell is on its bottom layer.
            for n in 0..4096usize {
                let (x, z) = cell(n);
                if (x / 16, z / 16) != (cx, cz) {
                    continue;
                }
                let (lx, lz) = ((x % 16) as usize, (z % 16) as usize);
                let i = lz * 16 + lx;
                blocks[i] = (n / 16) as i8;
                let d = (n % 16) as u8;
                data[i / 2] |= (if i % 2 == 0 { d } else { d << 4 }) as i8;
            }
            let mut s = Compound::new();
            s.insert("Y".into(), Tag::Byte(4));
            s.insert("Blocks".into(), Tag::ByteArray(blocks));
            s.insert("Data".into(), Tag::ByteArray(data));
            s.insert("BlockLight".into(), Tag::ByteArray(vec![0; 2048]));
            s.insert("SkyLight".into(), Tag::ByteArray(vec![-1; 2048]));
            let mut l = Compound::new();
            l.insert("xPos".into(), Tag::Int(cx));
            l.insert("zPos".into(), Tag::Int(cz));
            l.insert("LastUpdate".into(), Tag::Long(0));
            l.insert("TerrainPopulated".into(), Tag::Byte(1));
            l.insert("LightPopulated".into(), Tag::Byte(1));
            l.insert("V".into(), Tag::Byte(1));
            l.insert("HeightMap".into(), Tag::IntArray(vec![80; 256]));
            // Chunk k (0-15) holds biome ids 16k..16k+15, one per 4x4 column group.
            let k = (cz * 8 + cx) as usize;
            let biomes: Vec<i8> = (0..256usize)
                .map(|i| {
                    let (x, z) = (i % 16, i / 16);
                    let id = k * 16 + (z / 4) * 4 + x / 4;
                    if k < 16 { id as i8 } else { 1 }
                })
                .collect();
            l.insert("Biomes".into(), Tag::ByteArray(biomes));
            l.insert(
                "Sections".into(),
                Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(s)])?),
            );
            l.insert("Entities".into(), Tag::List(List::new(TagType::Compound)));
            l.insert("TileEntities".into(), Tag::List(List::new(TagType::Compound)));
            let mut root = Compound::new();
            root.insert("Level".into(), Tag::Compound(l));
            let tree = NamedTag { name: String::new(), tag: Tag::Compound(root) };
            let raw = RawChunk::encode(&tree, ChunkCompression::Zlib, 0)?;
            builder.set(cx as usize, cz as usize, Some(raw));
        }
    }
    builder.write(std::path::Path::new(&format!("{out}/region")), ChunkPos::new(0, 0).region())?;
    println!("wrote {out}");
    Ok(())
}

fn read(world: &str, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    let world = JavaWorld::open(world)?;
    let mut text = String::from("# legacy id\tdata\tJava block state (read back from the game's own upgrade)\n");
    let mut chunks = std::collections::HashMap::new();
    for n in 0..4096usize {
        let (x, z) = cell(n);
        let key = (x.div_euclid(16), z.div_euclid(16));
        if !chunks.contains_key(&key) {
            let c = world
                .chunk("minecraft:overworld", ChunkPos::new(key.0, key.1))?
                .ok_or("chunk missing")?;
            chunks.insert(key, c);
        }
        let chunk = &chunks[&key];
        let state = chunk
            .block(x.rem_euclid(16) as usize, 64, z.rem_euclid(16) as usize)
            .ok_or("block missing")?;
        let props: Vec<String> = state.properties.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let state = if props.is_empty() { state.name.clone() } else { format!("{}[{}]", state.name, props.join(",")) };
        writeln!(text, "{}\t{}\t{}", n / 16, n % 16, state)?;
    }
    std::fs::write(out, text)?;
    // The biome of every legacy id, as the game stored it in the upgraded chunks.
    let mut biomes = String::from("# legacy biome id\tJava biome\n");
    for k in 0..16usize {
        let (cx, cz) = ((k % 8) as i32, (k / 8) as i32);
        let chunk = world
            .chunk("minecraft:overworld", ChunkPos::new(cx, cz))?
            .ok_or("biome chunk missing")?;
        let section = chunk.section(4).ok_or("no section")?;
        let b = section.biomes.as_ref().ok_or("no biomes")?;
        for j in 0..16usize {
            let (xx, zz) = (j % 4, j / 4);
            writeln!(biomes, "{}\t{}", k * 16 + j, b.get(zz * 4 + xx))?;
        }
    }
    std::fs::write(format!("{out}.biomes"), biomes)?;
    println!("wrote {out}");
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    match (a.get(1).map(String::as_str), a.get(2), a.get(3)) {
        (Some("gen"), Some(level), Some(out)) => generate(level, out),
        (Some("read"), Some(world), Some(out)) => read(world, out),
        _ => Err("usage: gen <level.dat> <world> | read <world> <out.tsv>".into()),
    }
}
