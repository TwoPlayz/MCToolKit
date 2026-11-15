//! Prints what a Windows 64 `saveData.ms` holds: regions, chunk range and a block histogram.
use std::collections::BTreeMap;

use mctk_console::{ConsoleChunk, ConsoleRegion, ConsoleSave};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("path to saveData.ms");
    let save = ConsoleSave::open(path)?;
    println!("{:?} version {}/{}", save.platform, save.archive.original_version, save.archive.version);
    // Optional `x z` (overworld block column): print its blocks from the top down.
    if let (Some(x), Some(z)) = (std::env::args().nth(2), std::env::args().nth(3)) {
        let (x, z): (i32, i32) = (x.parse()?, z.parse()?);
        for (dim, _, _, entry) in save.archive.regions() {
            if dim != 0 || entry.data.is_empty() {
                continue;
            }
            let region = ConsoleRegion::parse(entry.data.clone())?;
            let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
            if let Some(raw) = region.raw_chunk(cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize)? {
                let chunk = ConsoleChunk::from_nbt(raw.nbt()?)?;
                if (chunk.x, chunk.z) == (cx, cz) {
                    let (lx, lz) = (x.rem_euclid(16) as usize, z.rem_euclid(16) as usize);
                    let col: Vec<_> = (60..100).filter_map(|y| chunk.block(lx, y, lz).filter(|b| b.0 != 0).map(|b| (y, b))).collect();
                    println!("column {x},{z} y60..100: {col:?}");
                }
            }
        }
        return Ok(());
    }
    for (dim, rx, rz, entry) in save.archive.regions() {
        let region = ConsoleRegion::parse(entry.data.clone())?;
        let mut hist: BTreeMap<(u8, u8), u32> = BTreeMap::new();
        let (mut n, mut xs, mut zs) = (0, (i32::MAX, i32::MIN), (i32::MAX, i32::MIN));
        for (lx, lz) in region.chunks() {
            let raw = region.raw_chunk(lx, lz)?.unwrap();
            let chunk = ConsoleChunk::from_nbt(raw.nbt()?)?;
            n += 1;
            xs = (xs.0.min(chunk.x), xs.1.max(chunk.x));
            zs = (zs.0.min(chunk.z), zs.1.max(chunk.z));
            for x in 0..16 {
                for z in 0..16 {
                    for y in 0..chunk.height {
                        let (id, d) = chunk.block(x, y, z).unwrap();
                        if id != 0 {
                            *hist.entry((id, d)).or_default() += 1;
                        }
                    }
                }
            }
        }
        if n == 0 {
            continue;
        }
        let mut top: Vec<_> = hist.iter().collect();
        top.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
        println!("dim {dim} r.{rx}.{rz}: {n} chunks x {xs:?} z {zs:?}; top blocks {:?}", &top[..top.len().min(12)]);
    }
    Ok(())
}
