//! Opens a Java world, prints its summary, parses every chunk, checks that re-encoding each
//! chunk reproduces its NBT byte for byte, and prints a block census. Read-only.
//!
//! cargo run --release -p mctk-java --example inspect_world -- <world dir>

use std::collections::HashMap;

use mctk_anvil::Region;
use mctk_java::{BlockStorage, JavaWorld};
use mctk_nbt::Flavor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: inspect_world <world dir>")?;
    let world = JavaWorld::open(&path)?;
    let l = &world.level;
    println!(
        "{} | version {:?} | DataVersion {:?} | spawn {:?}",
        l.name, l.version_name, l.data_version, l.spawn
    );

    let mut census: HashMap<String, u64> = HashMap::new();
    for dim in &world.dimensions {
        let regions = dim.regions()?;
        let (mut chunks, mut mismatches, mut styles) = (0, 0, HashMap::new());
        for rpos in &regions {
            let region = Region::open(dim.region_dir().join(rpos.file_name()))?;
            for (lx, lz) in region.chunks().collect::<Vec<_>>() {
                let original = region.read_chunk(lx, lz)?.expect("listed");
                let bytes = mctk_nbt::to_bytes(&original, Flavor::Java)?;
                let chunk = mctk_java::Chunk::from_nbt(original)?;
                assert_eq!(
                    (chunk.x, chunk.z),
                    (rpos.chunk(lx, lz).x, rpos.chunk(lx, lz).z),
                    "chunk position"
                );
                *styles
                    .entry(format!("{:?}/{:?}", chunk.era, chunk.palette_style))
                    .or_insert(0) += 1;
                if mctk_nbt::to_bytes(&chunk.to_nbt(), Flavor::Java)? != bytes {
                    mismatches += 1;
                    if mismatches <= 3 {
                        eprintln!("MISMATCH {} chunk ({}, {})", dim.id, chunk.x, chunk.z);
                    }
                }
                for s in &chunk.sections {
                    if let Some(BlockStorage::Palette(p)) = &s.blocks {
                        for &i in &p.indices {
                            *census
                                .entry(p.palette[i as usize].name.clone())
                                .or_insert(0) += 1;
                        }
                    }
                }
                chunks += 1;
            }
        }
        println!(
            "{:<24} {:>3} regions {:>6} chunks {:>4} mismatches  {:?}",
            dim.id,
            regions.len(),
            chunks,
            mismatches,
            styles
        );
    }

    let mut top: Vec<_> = census.into_iter().collect();
    top.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    println!("top blocks:");
    for (name, n) in top.iter().take(12) {
        println!("  {n:>11}  {name}");
    }

    // Spot check: the column under spawn.
    if let (Some(dim), Some(sp)) = (world.dimension("minecraft:overworld"), &l.spawn) {
        let pos = mctk_anvil::ChunkPos::new(sp.x.div_euclid(16), sp.z.div_euclid(16));
        if let Some(chunk) = world.chunk(&dim.id, pos)? {
            let (x, z) = (sp.x.rem_euclid(16) as usize, sp.z.rem_euclid(16) as usize);
            let column: Vec<String> = (sp.y - 4..=sp.y)
                .rev()
                .filter_map(|y| chunk.block(x, y, z).map(|b| format!("y={y} {b}")))
                .collect();
            println!("spawn column: {}", column.join(" | "));
        }
    }
    Ok(())
}
