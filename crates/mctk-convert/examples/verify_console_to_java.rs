//! Converts a console save to a Java world in a new folder, reopens the result with the
//! Java reader and checks every block (id, data) of every chunk against the source.
//!
//! cargo run --release -p mctk-convert --example verify_console_to_java -- <save> <out folder>

use std::collections::HashMap;

use mctk_console::{ConsoleChunk, ConsoleRegion, ConsoleSave};
use mctk_java::{BlockStorage, JavaWorld};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (src, out) = (
        args.get(1).ok_or("usage: <save> <out>")?,
        args.get(2).ok_or("usage: <save> <out>")?,
    );
    let save = ConsoleSave::open(src)?;
    let report = mctk_convert::console_to_java(&save, std::path::Path::new(out))?;
    println!(
        "converted: {:?}, {} region files, copied {:?}, notes {:?}",
        report.chunks, report.region_files, report.copied_files, report.notes
    );

    let world = JavaWorld::open(out)?;
    println!(
        "Java world: {:?}, data version {:?}, spawn {:?}",
        world.level.name, world.level.data_version, world.level.spawn
    );
    let dim_ids: HashMap<i32, &str> = [
        (0, "minecraft:overworld"),
        (-1, "minecraft:the_nether"),
        (1, "minecraft:the_end"),
    ]
    .into();

    let (mut checked_chunks, mut blocks, mut mismatches, mut missing) = (0, 0u64, 0u64, 0);
    for (dim, _, _, entry) in save.archive.regions() {
        if entry.data.is_empty() {
            continue;
        }
        let region = ConsoleRegion::parse(entry.data.clone())?;
        for (lx, lz) in region.chunks().collect::<Vec<_>>() {
            let source = ConsoleChunk::from_nbt(region.raw_chunk(lx, lz)?.expect("listed").nbt()?)?;
            let Some(java) =
                world.chunk(dim_ids[&dim], mctk_anvil::ChunkPos::new(source.x, source.z))?
            else {
                missing += 1;
                continue;
            };
            checked_chunks += 1;
            for y in 0..source.height {
                let section = java.section((y / 16) as i8);
                for z in 0..16 {
                    for x in 0..16 {
                        let (id, data) = source.block(x, y, z).expect("in range");
                        let got = match section.and_then(|s| s.blocks.as_ref()) {
                            Some(BlockStorage::Legacy { ids, data: d }) => {
                                let i = ((y % 16) * 16 + z) * 16 + x;
                                (ids[i] as u8, d[i])
                            }
                            _ => (0, 0),
                        };
                        blocks += 1;
                        // Omitted (all-air) sections read as air; their data values are irrelevant.
                        if got.0 != id || (id != 0 && got.1 != data) {
                            mismatches += 1;
                        }
                    }
                }
            }
        }
    }
    println!(
        "verified {checked_chunks} chunks, {blocks} blocks: {mismatches} mismatches, {missing} chunks missing"
    );
    if mismatches > 0 || missing > 0 {
        return Err("verification failed".into());
    }
    Ok(())
}
