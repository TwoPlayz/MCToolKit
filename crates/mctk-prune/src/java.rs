//! Java Edition: rewrites the region, entity and POI files of the pruned dimensions and
//! copies everything else unchanged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mctk_anvil::{ChunkPos, Region, RegionBuilder, RegionPos};
use mctk_java::{Dimension, JavaWorld};
use mctk_nbt::{NamedTag, Tag};

use crate::{Error, PruneReport, Selection, applies, check_selection, prepare_output};

fn vanilla_dimension(id: &str) -> Option<i32> {
    match id {
        "minecraft:overworld" => Some(0),
        "minecraft:the_nether" => Some(-1),
        "minecraft:the_end" => Some(1),
        _ => None,
    }
}

/// InhabitedTime sits at the root since 1.18 and under `Level` before.
fn inhabited_time(root: &NamedTag) -> Option<i64> {
    root.tag
        .get("InhabitedTime")
        .or_else(|| root.tag.get("Level")?.get("InhabitedTime"))
        .and_then(Tag::as_i64)
}

fn is_region_file(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        RegionPos::from_file_name(n).is_some() || (n.starts_with("c.") && n.ends_with(".mcc"))
    })
}

fn copy_tree(src: &Path, out: &Path, skip: &dyn Fn(&Path) -> bool) -> Result<(), Error> {
    for entry in std::fs::read_dir(src)? {
        let path = entry?.path();
        let target = out.join(path.file_name().expect("directory entry"));
        if path.is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_tree(&path, &target, skip)?;
        } else if !skip(&path) {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// Rewrites every region file of `src_dir` into `out_dir` with only the chunks `keep`
/// accepts; regions left empty are not written.
fn prune_folder(
    src_dir: &Path,
    out_dir: &Path,
    mut keep: impl FnMut(ChunkPos, &Region, usize, usize) -> Result<bool, Error>,
) -> Result<(usize, usize), Error> {
    let (mut kept, mut removed) = (0, 0);
    if !src_dir.is_dir() {
        return Ok((kept, removed));
    }
    for entry in std::fs::read_dir(src_dir)? {
        let path = entry?.path();
        let Some(pos) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(RegionPos::from_file_name)
        else {
            continue;
        };
        let region = Region::open(&path)?;
        let mut builder = RegionBuilder::new();
        let mut any = false;
        for (lx, lz) in region.chunks().collect::<Vec<_>>() {
            if keep(pos.chunk(lx, lz), &region, lx, lz)? {
                builder.set(lx, lz, region.raw_chunk(lx, lz)?);
                any = true;
                kept += 1;
            } else {
                removed += 1;
            }
        }
        if any {
            std::fs::create_dir_all(out_dir)?;
            builder.write(out_dir, pos)?;
        }
    }
    Ok((kept, removed))
}

/// Whether any terrain chunk stores InhabitedTime; without it a Time layer would select
/// nothing and inverted mode would delete every chunk.
fn records_inhabited_time(dims: &[&Dimension]) -> Result<bool, Error> {
    for d in dims {
        for pos in d.regions()? {
            let region = Region::open(d.region_dir().join(pos.file_name()))?;
            for (lx, lz) in region.chunks().collect::<Vec<_>>() {
                if region
                    .read_chunk(lx, lz)?
                    .as_ref()
                    .and_then(inhabited_time)
                    .is_some()
                {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Writes a pruned copy of the Java world `src` to `out` (empty or missing).
/// `dimension` limits pruning to one vanilla dimension; others are copied unchanged.
pub fn prune_java(
    src: &Path,
    out: &Path,
    selection: &Selection,
    dimension: Option<i32>,
) -> Result<PruneReport, Error> {
    check_selection(selection)?;
    let world = JavaWorld::open(src)?;
    let dims: Vec<_> = world
        .dimensions
        .iter()
        .filter(|d| applies(dimension, vanilla_dimension(&d.id)))
        .collect();
    if selection.uses_inhabited_time() && !records_inhabited_time(&dims)? {
        return Err(Error::Invalid(
            "this world does not record InhabitedTime (saved before 1.6); the Time layer cannot be used"
                .into(),
        ));
    }
    prepare_output(src, out)?;
    let rewritten: Vec<PathBuf> = dims
        .iter()
        .flat_map(|d| [d.region_dir(), d.entities_dir(), d.poi_dir()])
        .collect();
    copy_tree(src, out, &|path| {
        is_region_file(path)
            && path
                .parent()
                .is_some_and(|p| rewritten.iter().any(|r| r == p))
    })?;

    let mut report = PruneReport::default();
    for d in dims {
        let rel = d.root.strip_prefix(src).unwrap_or(Path::new(""));
        let out_root = out.join(rel);
        // Entity and POI chunks follow the terrain chunk's decision.
        let mut decisions: HashMap<ChunkPos, bool> = HashMap::new();
        let (kept, removed) = prune_folder(
            &d.region_dir(),
            &out_root.join("region"),
            |pos, region, lx, lz| {
                let inhabited = if selection.uses_inhabited_time() {
                    region.read_chunk(lx, lz)?.as_ref().and_then(inhabited_time)
                } else {
                    None
                };
                let keep = selection.keeps(pos, inhabited);
                decisions.insert(pos, keep);
                Ok(keep)
            },
        )?;
        report.kept += kept;
        report.removed += removed;
        for folder in ["entities", "poi"] {
            prune_folder(
                &d.root.join(folder),
                &out_root.join(folder),
                |pos, _, _, _| {
                    Ok(decisions
                        .get(&pos)
                        .copied()
                        .unwrap_or_else(|| selection.keeps(pos, None)))
                },
            )?;
        }
    }
    Ok(report)
}
