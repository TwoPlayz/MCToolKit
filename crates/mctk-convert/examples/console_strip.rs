//! Copies a Windows 64 console save without some records, to find what makes the game fail to
//! load it.
//!
//! cargo run --release -p mctk-convert --example console_strip -- <in.ms> <out.ms> <entities|tiles|both|level>
//! `level` keeps the chunks but takes level.dat from the save given as the fifth argument.
use mctk_console::{ChunkCodec, ConsoleChunk, ConsoleRegion, ConsoleRegionBuilder, ConsoleSave, RawConsoleChunk};
use mctk_nbt::{List, Tag, TagType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let (input, output, mode) = (&a[1], &a[2], a[3].as_str());
    let mut save = ConsoleSave::open(input)?;
    if mode == "set" {
        // Key=b:1 / Key=i:35 / Key=l:5 entries to level.dat.
        let entry = save.archive.entries.iter_mut().find(|e| e.name == "level.dat").unwrap();
        let (_, raw) = mctk_nbt::decompress(&entry.data)?;
        let mut root = mctk_nbt::from_bytes(&raw, mctk_nbt::Flavor::Java)?;
        let data = root.tag.as_compound_mut().and_then(|c| c.get_mut("Data")).and_then(Tag::as_compound_mut).unwrap();
        for arg in &a[4..] {
            let (key, rest) = arg.split_once('=').unwrap();
            let (kind, value) = rest.split_once(':').unwrap();
            let tag = match kind {
                "b" => Tag::Byte(value.parse()?),
                "i" => Tag::Int(value.parse()?),
                _ => Tag::Long(value.parse()?),
            };
            data.insert(key.into(), tag);
        }
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&mctk_nbt::to_bytes(&root, mctk_nbt::Flavor::Java)?)?;
        entry.data = e.finish()?;
    } else if mode == "level" {
        let other = ConsoleSave::open(&a[4])?;
        let level = other.archive.get("level.dat").unwrap().data.clone();
        save.archive.entries.iter_mut().find(|e| e.name == "level.dat").unwrap().data = level;
    } else {
        for entry in save.archive.entries.iter_mut().filter(|e| e.name.ends_with(".mcr") && !e.data.is_empty()) {
            let region = ConsoleRegion::parse(entry.data.clone())?;
            let mut builder = ConsoleRegionBuilder::default();
            builder.little = true;
            for (lx, lz) in region.chunks() {
                let raw = region.raw_chunk(lx, lz)?.unwrap();
                let mut tree = raw.nbt()?;
                let _ = ConsoleChunk::from_nbt(tree.clone())?;
                let level = tree.tag.as_compound_mut().unwrap().get_mut("Level").and_then(Tag::as_compound_mut).unwrap();
                if matches!(mode, "entities" | "both") {
                    level.insert("Entities".into(), Tag::List(List::new(TagType::Compound)));
                }
                if matches!(mode, "tiles" | "both") {
                    level.insert("TileEntities".into(), Tag::List(List::new(TagType::Compound)));
                }
                builder.set(lx, lz, Some(RawConsoleChunk::encode_compressed(&tree, 8, ChunkCodec::Zlib, raw.timestamp)?));
            }
            entry.data = builder.build()?;
        }
    }
    std::fs::write(output, save.encode()?)?;
    Ok(())
}
