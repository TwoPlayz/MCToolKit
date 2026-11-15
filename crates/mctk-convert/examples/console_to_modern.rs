//! Converts a console save to a modern (26.x) Java world, or compares two Java worlds block by
//! block (to check this converter against the game's own upgrade of the same save).
//!
//! cargo run --release -p mctk-convert --example console_to_modern -- convert <save> <out world>
//! cargo run --release -p mctk-convert --example console_to_modern -- compare <mine> <games>

use std::collections::BTreeMap;

use mctk_anvil::ChunkPos;
use mctk_console::ConsoleSave;
use mctk_convert::{ConvertOptions, Progress, console_to_modern};
use mctk_java::JavaWorld;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    match (a.get(1).map(String::as_str), a.get(2), a.get(3)) {
        (Some("convert"), Some(save), Some(out)) => {
            let save = ConsoleSave::open(save)?;
            let reg = mctk_registry::builtin::java("26.3").unwrap();
            let report = console_to_modern(&save, std::path::Path::new(out), &reg, &ConvertOptions::default(), &Progress::default())?;
            println!("{} chunks, {} entities, {} players, {} maps; notes {:?}", report.chunks, report.entities, report.players, report.maps, report.notes);
        }
        (Some("bedrock"), Some(save), Some(out)) => {
            // Through a temporary Java world, as the converter does.
            let save = ConsoleSave::open(save)?;
            let reg = mctk_registry::builtin::java("26.3").unwrap();
            let stage = std::path::PathBuf::from(format!("{out}.java-stage"));
            let _ = std::fs::remove_dir_all(&stage);
            console_to_modern(&save, &stage, &reg, &ConvertOptions::default(), &Progress::default())?;
            let pass = ConvertOptions { offset: (0, 0), ..ConvertOptions::default() };
            let r = mctk_convert::java_to_bedrock_with(&stage, std::path::Path::new(out), &pass, &Progress::default())?;
            let _ = std::fs::remove_dir_all(&stage);
            println!("{:?} replaced {:?} unmapped {:?} skipped {:?}", r.chunks, r.replaced, r.unmapped, r.skipped);
        }
        (Some("console"), Some(world), Some(out)) => {
            // A modern Java world to a Windows 64 console save (the test platform).
            let r = mctk_convert::java_to_console(
                std::path::Path::new(world),
                std::path::Path::new(out),
                match a.get(4).map(String::as_str) {
                    Some("ps3") => mctk_console::Platform::Ps3,
                    Some("x360") => mctk_console::Platform::Xbox360,
                    _ => mctk_console::Platform::Windows64,
                },
                &mctk_replace::ReplacementMap::default(),
                &ConvertOptions::default(),
                &Progress::default(),
            )?;
            println!("{} chunks, {} entities, {} block entities, {} outside; replaced {:?}; notes {:?}", r.chunks, r.entities, r.block_entities, r.outside, r.replaced.iter().collect::<Vec<_>>(), r.notes);
        }
        (Some("compare"), Some(mine), Some(games)) => {
            let (mine, games) = (JavaWorld::open(mine)?, JavaWorld::open(games)?);
            let mut diffs: BTreeMap<String, u64> = BTreeMap::new();
            let (mut chunks, mut blocks, mut different) = (0u64, 0u64, 0u64);
            for dim in &mine.dimensions {
                for rpos in dim.regions()? {
                    let region = mctk_anvil::Region::open(dim.region_dir().join(rpos.file_name()))?;
                    for (lx, lz) in region.chunks() {
                        let pos = rpos.chunk(lx, lz);
                        let (Some(m), Some(g)) = (mine.chunk(&dim.id, ChunkPos::new(pos.x, pos.z))?, games.chunk(&dim.id, ChunkPos::new(pos.x, pos.z))?) else {
                            *diffs.entry("chunk missing in the game's world".into()).or_default() += 1;
                            continue;
                        };
                        chunks += 1;
                        for y in 0..256 {
                            for z in 0..16 {
                                for x in 0..16 {
                                    blocks += 1;
                                    let (bm, bg) = (m.block(x, y, z), g.block(x, y, z));
                                    let eq = match (bm, bg) {
                                        (Some(a), Some(b)) => a.name == b.name && {
                                            // The game leaves defaults out; compare what both say.
                                            b.properties.iter().all(|(k, v)| a.properties.get(k).is_none_or(|x| x == v))
                                        },
                                        (None, None) => true,
                                        (Some(a), None) | (None, Some(a)) => a.name == "minecraft:air",
                                    };
                                    if !eq {
                                        different += 1;
                                        let key = if bg.is_none() { format!("game has no section: {} y={}", dim.id, y / 16) } else { format!("{} <> {}", bm.map_or("-".into(), |b| b.to_string()), bg.map_or("-".into(), |b| b.to_string())) };
                                        if different < 6 { println!("at {} {} {} (chunk {} {}): {key}", pos.x * 16 + x as i32, y, pos.z * 16 + z as i32, pos.x, pos.z); }
                                        *diffs.entry(key).or_default() += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            println!("{chunks} chunks, {blocks} blocks, {different} different");
            let mut v: Vec<_> = diffs.into_iter().collect();
            v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            for (k, n) in v.iter().take(40) {
                println!("{n:>8}  {k}");
            }
        }
        _ => return Err("usage: convert <save> <out> | compare <mine> <games>".into()),
    }
    Ok(())
}
