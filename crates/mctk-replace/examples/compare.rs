//! `cargo run --release -p mctk-replace --example compare -- <world-a> <world-b>`
//! Compares every block of the fully generated overworld chunks of A with the same chunks
//! in B (e.g. our converted world against the copy the game re-saved after loading it).

use mctk_anvil::ChunkPos;
use mctk_java::{BlockState, BlockStorage, JavaWorld};
use std::collections::BTreeMap;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [a, b] = args.as_slice() else { panic!("usage: compare <a> <b>") };
    let (wa, wb) = (JavaWorld::open(a).unwrap(), JavaWorld::open(b).unwrap());
    let dim = "minecraft:overworld";
    let (mut chunks, mut missing, mut blocks, mut differ) = (0u64, 0u64, 0u64, 0u64);
    let mut examples: BTreeMap<(String, String), u64> = BTreeMap::new();
    for rpos in wa.dimension(dim).unwrap().regions().unwrap() {
        for lz in 0..32 {
            for lx in 0..32 {
                let pos = rpos.chunk(lx, lz);
                let Ok(Some(ca)) = wa.chunk(dim, ChunkPos { x: pos.x, z: pos.z }) else { continue };
                // Only chunks whose eight neighbours also exist in A: terrain features of a
                // newly generated neighbour can spill into the border chunks.
                let interior = (-1..=1).all(|dz| {
                    (-1..=1).all(|dx| {
                        matches!(wa.chunk(dim, ChunkPos { x: pos.x + dx, z: pos.z + dz }), Ok(Some(_)))
                    })
                });
                if !interior {
                    continue;
                }
                let cb = match wb.chunk(dim, ChunkPos { x: pos.x, z: pos.z }) {
                    Ok(Some(c)) => c,
                    _ => {
                        missing += 1;
                        continue;
                    }
                };
                chunks += 1;
                for sa in &ca.sections {
                    let Some(BlockStorage::Palette(pa)) = &sa.blocks else { continue };
                    let Some(sb) = cb.section(sa.y) else { continue };
                    let Some(BlockStorage::Palette(pb)) = &sb.blocks else { continue };
                    for i in 0..4096 {
                        blocks += 1;
                        let (x, y): (&BlockState, &BlockState) = (pa.get(i), pb.get(i));
                        if x != y {
                            differ += 1;
                            *examples.entry((x.to_string(), y.to_string())).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }
    println!("{chunks} chunks compared ({missing} of A missing in B), {blocks} blocks, {differ} differ");
    let mut ex: Vec<_> = examples.into_iter().collect();
    ex.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for ((x, y), n) in ex.into_iter().take(12) {
        println!("{n:>8}  {x}  ->  {y}");
    }
}
