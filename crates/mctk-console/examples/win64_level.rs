//! Sets generator fields in a Windows 64 save's level.dat, to test how worlds made as flat or
//! large-biomes worlds convert.
//!
//! cargo run --release -p mctk-console --example win64_level -- <saveData.ms> <generatorName> [generatorOptions]
use mctk_console::ConsoleSave;
use mctk_nbt::{Flavor, Tag};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let (path, name) = (a.get(1).ok_or("save")?, a.get(2).ok_or("generatorName")?);
    let mut save = ConsoleSave::open(path)?;
    let entry = save.archive.entries.iter_mut().find(|e| e.name == "level.dat").ok_or("no level.dat")?;
    let (_, raw) = mctk_nbt::decompress(&entry.data)?;
    let mut root = mctk_nbt::from_bytes(&raw, Flavor::Java)?;
    let data = root.tag.as_compound_mut().and_then(|c| c.get_mut("Data")).and_then(Tag::as_compound_mut).ok_or("no Data")?;
    data.insert("generatorName".into(), Tag::String(name.clone()));
    if let Some(o) = a.get(3) {
        data.insert("generatorOptions".into(), Tag::String(o.clone()));
    }
    let raw = mctk_nbt::to_bytes(&root, Flavor::Java)?;
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&raw)?;
    entry.data = e.finish()?;
    std::fs::write(path, save.encode()?)?;
    println!("generator set to {name}");
    Ok(())
}
