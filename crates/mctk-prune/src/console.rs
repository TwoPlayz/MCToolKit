//! Legacy Console: rebuilds the region files inside the save archive without the removed
//! chunks. Kept chunk records are copied as stored, without recompression.

use std::collections::HashMap;
use std::path::Path;

use mctk_anvil::ChunkPos;
use mctk_console::{ConsoleRegion, ConsoleRegionBuilder, ConsoleSave, Platform, RawConsoleChunk};
use mctk_nbt::Tag;

use crate::{Error, PruneReport, Selection, applies, check_selection, prepare_output};

fn inhabited_time(raw: &RawConsoleChunk) -> Result<Option<i64>, Error> {
    Ok(raw
        .nbt()?
        .tag
        .get("Level")
        .and_then(|l| l.get("InhabitedTime"))
        .and_then(Tag::as_i64))
}

/// Every chunk of a dimension (0, -1 or 1) of a console save, by chunk position, with its
/// InhabitedTime when `read_times` asks for it and the chunk records one. To preview a prune.
pub fn chunk_times_console(src: &Path, dimension: i32, read_times: bool) -> Result<HashMap<(i32, i32), Option<i64>>, Error> {
    let save = ConsoleSave::open(src)?;
    let mut chunks = HashMap::new();
    for (dim, rx, rz, e) in save.archive.regions() {
        if dim != dimension || e.data.is_empty() {
            continue;
        }
        let region = ConsoleRegion::parse(e.data.clone())?;
        for (lx, lz) in region.chunks().collect::<Vec<_>>() {
            let time = if read_times {
                let raw = region.raw_chunk(lx, lz)?.expect("listed chunk");
                inhabited_time(&raw)?
            } else {
                None
            };
            chunks.insert((rx * 32 + lx as i32, rz * 32 + lz as i32), time);
        }
    }
    Ok(chunks)
}

/// Prunes a loaded save in memory.
pub fn prune_console(
    save: &mut ConsoleSave,
    selection: &Selection,
    dimension: Option<i32>,
) -> Result<PruneReport, Error> {
    check_selection(selection)?;
    let regions: Vec<(i32, i32, i32, String)> = save
        .archive
        .regions()
        .into_iter()
        .filter(|(dim, ..)| applies(dimension, Some(*dim)))
        .map(|(dim, x, z, e)| (dim, x, z, e.name.clone()))
        .collect();
    let mut report = PruneReport::default();
    for (_, rx, rz, name) in regions {
        let entry = save
            .archive
            .entries
            .iter_mut()
            .find(|e| e.name == name)
            .expect("listed region");
        if entry.data.is_empty() {
            continue;
        }
        let region = ConsoleRegion::parse(entry.data.clone())?;
        let mut builder = ConsoleRegionBuilder::default();
        let (mut kept, mut removed) = (0, 0);
        for (lx, lz) in region.chunks().collect::<Vec<_>>() {
            let raw = region.raw_chunk(lx, lz)?.expect("listed chunk");
            let pos = ChunkPos::new(rx * 32 + lx as i32, rz * 32 + lz as i32);
            let inhabited = if selection.uses_inhabited_time() {
                inhabited_time(&raw)?
            } else {
                None
            };
            if selection.keeps(pos, inhabited) {
                builder.set(lx, lz, Some(raw));
                kept += 1;
            } else {
                removed += 1;
            }
        }
        // Untouched regions keep their exact bytes; emptied ones become empty entries,
        // as the game itself stores unused regions.
        if removed > 0 {
            entry.data = if kept == 0 {
                Vec::new()
            } else {
                builder.build()?
            };
        }
        report.kept += kept;
        report.removed += removed;
    }
    Ok(report)
}

/// Writes the save to `out`. A PS3 save folder becomes a copy of the folder with the new
/// `GAMEDATA` (its `PARAM.PFD` then needs re-signing before a PS3 accepts it); an
/// Xbox 360 save becomes a new `savegame.dat` file at `out`.
pub fn save_console_copy(save: &ConsoleSave, out: &Path) -> Result<(), Error> {
    let bytes = save.encode()?;
    let folder = save
        .source
        .parent()
        .filter(|p| save.platform == Platform::Ps3 && p.join("PARAM.SFO").is_file());
    match folder {
        Some(src_dir) => {
            prepare_output(src_dir, out)?;
            for entry in std::fs::read_dir(src_dir)? {
                let path = entry?.path();
                if path.is_file() && path != save.source {
                    std::fs::copy(&path, out.join(path.file_name().expect("file")))?;
                }
            }
            std::fs::write(out.join("GAMEDATA"), bytes)?;
        }
        None => {
            if out.exists() {
                return Err(Error::Invalid(format!("{} already exists", out.display())));
            }
            std::fs::write(out, bytes)?;
        }
    }
    Ok(())
}
