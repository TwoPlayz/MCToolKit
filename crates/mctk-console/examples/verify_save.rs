//! Read-only validation of a console save: decodes every chunk, rewrites every region with
//! this crate's encoders, rebuilds the platform file in memory, reopens it and checks that
//! every chunk's NBT and every non-region file is unchanged.
//!
//! cargo run --release -p mctk-console --example verify_save -- <PS3 save folder | GAMEDATA | savegame.dat>

use std::collections::BTreeMap;

use mctk_console::{
    ConsoleChunk, ConsoleRegion, ConsoleRegionBuilder, ConsoleSave, RawConsoleChunk, SaveArchive,
};
use mctk_nbt::NamedTag;

/// Chunk NBT keyed by (region file name, local x, local z).
type ChunkMap = BTreeMap<(String, usize, usize), NamedTag>;

fn chunks_of(archive: &SaveArchive) -> Result<ChunkMap, Box<dyn std::error::Error>> {
    let mut out = BTreeMap::new();
    for (_, _, _, entry) in archive.regions() {
        let region = ConsoleRegion::parse(entry.data.clone())?;
        for (lx, lz) in region.chunks().collect::<Vec<_>>() {
            out.insert(
                (entry.name.clone(), lx, lz),
                region.raw_chunk(lx, lz)?.expect("listed").nbt()?,
            );
        }
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: verify_save <save>")?;
    let save = ConsoleSave::open(&path)?;
    let original = chunks_of(&save.archive)?;
    println!(
        "{:?} save, archive v{}, {} files, {} chunks",
        save.platform,
        save.archive.version,
        save.archive.entries.len(),
        original.len()
    );

    // Typed access sanity check: tallest column of the first overworld chunk.
    if let Some(((name, lx, lz), nbt)) = original.iter().find(|((n, _, _), _)| n.starts_with("r."))
    {
        let chunk = ConsoleChunk::from_nbt(nbt.clone())?;
        let top = chunk.top_block(8, 8);
        println!(
            "{name} chunk ({lx},{lz}) = world chunk ({}, {}), height {}, top block at column (8,8): {top:?}",
            chunk.x, chunk.z, chunk.height
        );
    }

    // Unmodified round trip of the platform file.
    let platform_bytes = std::fs::read(&save.source)?;
    let reencoded = save.encode()?;
    println!(
        "platform file re-encode: {}",
        if reencoded == platform_bytes {
            "identical"
        } else {
            "differs (expected for Xbox 360: stored LZX blocks instead of compressed ones)"
        }
    );

    // Full rewrite: every region rebuilt from re-encoded chunks.
    let mut rewritten = save.clone();
    let codec = save.platform.chunk_codec();
    for entry in &mut rewritten.archive.entries {
        let is_region = entry.name.ends_with(".mcr");
        if !is_region || entry.data.is_empty() {
            continue;
        }
        let region = ConsoleRegion::parse(entry.data.clone())?;
        let mut builder = ConsoleRegionBuilder::default();
        for (lx, lz) in region.chunks().collect::<Vec<_>>() {
            let raw = region.raw_chunk(lx, lz)?.expect("listed");
            builder.set(
                lx,
                lz,
                Some(RawConsoleChunk::encode(&raw.nbt()?, codec, raw.timestamp)?),
            );
        }
        entry.data = builder.build()?;
    }
    let bytes = rewritten.encode()?;
    let dir = std::env::temp_dir().join(format!("mctk-console-verify-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(save.source.file_name().ok_or("file name")?);
    std::fs::write(&file, &bytes)?;
    let reopened = ConsoleSave::open(&file)?;
    let after = chunks_of(&reopened.archive)?;
    let others_equal = save
        .archive
        .entries
        .iter()
        .filter(|e| !e.name.ends_with(".mcr"))
        .all(|e| {
            reopened
                .archive
                .get(&e.name)
                .is_some_and(|r| r.data == e.data)
        });
    println!(
        "full rewrite: {} bytes (original {}), chunks {} ({} of {}), other files {}",
        bytes.len(),
        platform_bytes.len(),
        if after == original {
            "identical"
        } else {
            "DIFFERENT"
        },
        after.len(),
        original.len(),
        if others_equal {
            "identical"
        } else {
            "DIFFERENT"
        }
    );
    std::fs::remove_dir_all(&dir)?;
    if after != original || !others_equal {
        return Err("validation failed".into());
    }
    Ok(())
}
