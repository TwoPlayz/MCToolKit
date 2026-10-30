//! Decodes every chunk of every region file under a directory and checks that re-encoding
//! the NBT reproduces the decompressed bytes exactly. Read-only.
//!
//! cargo run --release -p mctk-anvil --example roundtrip_world -- <world dir>

use std::path::{Path, PathBuf};

use mctk_anvil::Region;
use mctk_nbt::Flavor;

fn region_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            region_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "mca") {
            out.push(path);
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: roundtrip_world <dir>")?,
    );
    let mut files = Vec::new();
    region_files(&dir, &mut files)?;
    files.sort();
    let (mut chunks, mut failures) = (0usize, 0usize);
    for file in &files {
        let region = Region::open(file)?;
        let mut n = 0;
        for (lx, lz) in region.chunks().collect::<Vec<_>>() {
            let raw = region.raw_chunk(lx, lz)?.expect("listed");
            let bytes = raw.decompress()?;
            let tag = mctk_nbt::from_bytes(&bytes, Flavor::Java)?;
            if mctk_nbt::to_bytes(&tag, Flavor::Java)? != bytes {
                failures += 1;
                eprintln!("MISMATCH {} ({lx}, {lz})", file.display());
            }
            n += 1;
        }
        chunks += n;
        println!(
            "{:>5} chunks  {}",
            n,
            file.strip_prefix(&dir).unwrap_or(file).display()
        );
    }
    println!(
        "{} files, {chunks} chunks, {failures} mismatches",
        files.len()
    );
    if failures > 0 {
        return Err("round-trip mismatches".into());
    }
    Ok(())
}
