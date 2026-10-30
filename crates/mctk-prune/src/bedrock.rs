//! Bedrock Edition: deletes every database record of a removed chunk, including the
//! actors listed in its `digp` record. The world is then saved with a freshly compacted
//! database, which is UMT's "Optimize" step.

use std::collections::BTreeSet;
use std::path::Path;

use mctk_anvil::ChunkPos;
use mctk_bedrock::{BedrockWorld, Key};

use crate::{Error, PruneReport, Selection, applies, check_selection, prepare_output};

/// Prunes a loaded world in memory.
pub fn prune_bedrock(
    world: &mut BedrockWorld,
    selection: &Selection,
    dimension: Option<i32>,
) -> Result<PruneReport, Error> {
    check_selection(selection)?;
    if selection.uses_inhabited_time() {
        return Err(Error::Invalid(
            "the Time layer needs InhabitedTime, which Bedrock worlds do not record".into(),
        ));
    }
    let (mut kept, mut removed) = (BTreeSet::new(), BTreeSet::new());
    let mut doomed = Vec::new();
    for (k, v) in &world.db.entries {
        match Key::decode(k) {
            Key::Chunk(c) if applies(dimension, Some(c.dimension.id())) => {
                let column = (c.dimension.id(), c.x, c.z);
                if selection.keeps(ChunkPos::new(c.x, c.z), None) {
                    kept.insert(column);
                } else {
                    removed.insert(column);
                    doomed.push(k.clone());
                }
            }
            Key::ActorDigest { x, z, dimension: d }
                if applies(dimension, Some(d.id()))
                    && !selection.keeps(ChunkPos::new(x, z), None) =>
            {
                doomed.push(k.clone());
                for uid in v.chunks_exact(8) {
                    doomed.push(Key::Actor(uid.to_vec()).encode());
                }
            }
            _ => {}
        }
    }
    let records = doomed.len();
    for k in doomed {
        world.db.entries.remove(&k);
    }
    Ok(PruneReport {
        kept: kept.len(),
        removed: removed.len(),
        notes: vec![format!("{records} database records deleted")],
    })
}

/// Opens the Bedrock world `src` (folder or `.mcworld`), prunes it and saves the result
/// as a world folder at `out` (empty or missing).
pub fn prune_bedrock_world(
    src: &Path,
    out: &Path,
    selection: &Selection,
    dimension: Option<i32>,
) -> Result<PruneReport, Error> {
    let mut world = BedrockWorld::open(src)?;
    let report = prune_bedrock(&mut world, selection, dimension)?;
    prepare_output(src, out)?;
    world.save_to_folder(out)?;
    Ok(report)
}
