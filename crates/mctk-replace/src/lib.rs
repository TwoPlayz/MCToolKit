//! World scanning and replacement resolution.
//!
//! 1. [`Scan`] walks a source world and counts every distinct block state in use.
//! 2. [`Scan::unsupported`] checks those against the target version's
//!    [`Registry`](mctk_registry::Registry) and lists what it cannot represent exactly.
//!
//! Replacement choices and smart defaults build on that list.

mod map;
mod plan;
mod suggest;

pub use map::{MapError, ReplacementMap, Source, parse_state};
pub use plan::{Choice, Plan, PlanGroup, candidates, reason_label};
pub use suggest::{Action, Reason, Suggestion, suggest, suggest_all};

use indexmap::IndexMap;
use mctk_java::{BlockState, BlockStorage, JavaWorld};
use mctk_registry::{BlockCheck, Registry};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Java(#[from] mctk_java::Error),
    #[error(transparent)]
    Anvil(#[from] mctk_anvil::Error),
    #[error("Bedrock world: {0}")]
    Bedrock(String),
}

/// A block state the target version cannot represent exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
    pub state: BlockState,
    pub count: u64,
    /// Never [`BlockCheck::Exact`].
    pub check: BlockCheck,
}

/// Everything a scan found in a source world.
#[derive(Debug, Default)]
pub struct Scan {
    /// Distinct block states and how many blocks use each.
    pub blocks: IndexMap<BlockState, u64>,
    pub chunks: u64,
    /// Chunks stored in a pre-1.13 layout, which are not scanned yet.
    pub legacy_chunks_skipped: u64,
}

impl Scan {
    /// Scans every chunk of every dimension of a Java world.
    pub fn java_world(path: impl AsRef<std::path::Path>) -> Result<Scan, Error> {
        Self::java_world_where(path, |_, _, _| true)
    }

    /// Scans the chunks of a Java world that `keep(dimension, chunk x, chunk z)` accepts (the
    /// dimension is 0, -1 for the Nether or 1 for the End), for a conversion that leaves the rest out.
    pub fn java_world_where(path: impl AsRef<std::path::Path>, keep: impl Fn(i32, i32, i32) -> bool) -> Result<Scan, Error> {
        let world = JavaWorld::open(path)?;
        let mut scan = Scan::default();
        for dim in &world.dimensions {
            let number = match dim.id.as_str() {
                "minecraft:the_nether" => -1,
                "minecraft:the_end" => 1,
                _ => 0,
            };
            for rpos in dim.regions()? {
                let region = mctk_anvil::Region::open(dim.region_dir().join(rpos.file_name()))?;
                for (lx, lz) in region.chunks() {
                    if !keep(number, rpos.x * 32 + lx as i32, rpos.z * 32 + lz as i32) {
                        continue;
                    }
                    let Some(nbt) = region.read_chunk(lx, lz)? else {
                        continue;
                    };
                    let chunk = mctk_java::Chunk::from_nbt_read_only(nbt)?;
                    scan.chunks += 1;
                    for section in &chunk.sections {
                        match &section.blocks {
                            Some(BlockStorage::Palette(c)) => {
                                let mut uses = vec![0u64; c.palette.len()];
                                for &i in &c.indices {
                                    uses[i as usize] += 1;
                                }
                                for (state, n) in c.palette.iter().zip(uses) {
                                    if n > 0 {
                                        *scan.blocks.entry(state.clone()).or_insert(0) += n;
                                    }
                                }
                            }
                            Some(BlockStorage::Legacy { .. }) => scan.legacy_chunks_skipped += 1,
                            None => {}
                        }
                    }
                }
            }
        }
        Ok(scan)
    }

    /// Scans every subchunk of a Bedrock world (folder or `.mcworld`). Block states keep Bedrock's
    /// names, and their values are typed text as in the Bedrock registry: `b:1`, `i:3`, `s:top`.
    pub fn bedrock_world(path: impl AsRef<std::path::Path>) -> Result<Scan, Error> {
        use mctk_bedrock::{ChunkTag, Key, SubChunk};
        let world = mctk_bedrock::BedrockWorld::open(path).map_err(|e| Error::Bedrock(e.to_string()))?;
        let mut scan = Scan::default();
        let mut columns = std::collections::HashSet::new();
        for (key, bytes) in &world.db.entries {
            let Key::Chunk(ck) = Key::decode(key) else { continue };
            if ck.tag != ChunkTag::SubChunkPrefix {
                continue;
            }
            columns.insert((ck.dimension.id(), ck.x, ck.z));
            let Ok(sub) = SubChunk::parse(bytes) else { continue };
            let Some(storage) = sub.blocks() else {
                scan.legacy_chunks_skipped += 1;
                continue;
            };
            let mut uses = vec![0u64; storage.palette.len()];
            for &i in &storage.indices {
                uses[i as usize] += 1;
            }
            for (entry, n) in storage.palette.iter().zip(uses) {
                if n == 0 {
                    continue;
                }
                let Some(name) = entry.tag.get("name").and_then(mctk_nbt::Tag::as_str) else { continue };
                let mut state = BlockState::new(name);
                if let Some(mctk_nbt::Tag::Compound(states)) = entry.tag.get("states") {
                    for (k, v) in states {
                        let text = match v {
                            mctk_nbt::Tag::Byte(b) => format!("b:{b}"),
                            mctk_nbt::Tag::Int(i) => format!("i:{i}"),
                            mctk_nbt::Tag::String(s) => format!("s:{s}"),
                            other => format!("?:{other}"),
                        };
                        state.properties.insert(k.clone(), text);
                    }
                }
                *scan.blocks.entry(state).or_insert(0) += n;
            }
        }
        scan.chunks = columns.len() as u64;
        Ok(scan)
    }

    /// Block states the `target` registry cannot represent exactly, most used first.
    pub fn unsupported(&self, target: &Registry) -> Vec<Unsupported> {
        let mut out: Vec<Unsupported> = self
            .blocks
            .iter()
            .filter_map(|(state, &count)| {
                let check = target.check_block(
                    &state.name,
                    state.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                );
                (check != BlockCheck::Exact).then(|| Unsupported {
                    state: state.clone(),
                    count,
                    check,
                })
            })
            .collect();
        out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.state.cmp(&b.state)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mctk_registry::{BlockDef, Edition};

    #[test]
    fn unsupported_lists_unknown_and_bad_states_by_use() {
        let mut target = Registry::new(Edition::Java, "t");
        target.blocks.insert(
            "minecraft:stone".into(),
            BlockDef {
                properties: IndexMap::new(),
                defaults: IndexMap::new(),
            },
        );
        let mut scan = Scan::default();
        scan.blocks.insert(BlockState::new("minecraft:stone"), 50);
        scan.blocks.insert(BlockState::new("minecraft:copper_chest"), 3);
        scan.blocks
            .insert(BlockState::new("minecraft:stone").with("axis", "y"), 7);
        let bad = scan.unsupported(&target);
        assert_eq!(bad.len(), 2);
        assert_eq!(bad[0].count, 7);
        assert_eq!(bad[1].check, BlockCheck::UnknownBlock);
    }

    /// Set `MCTK_TEST_JAVA_WORLD` and `MCTK_TEST_JAVA_REGISTRY` to a generated world and its
    /// version's registry file; every block in a real world must be exact for its own version.
    #[test]
    fn real_world_is_exact_for_its_own_version() {
        let (Ok(world), Ok(reg)) = (
            std::env::var("MCTK_TEST_JAVA_WORLD"),
            std::env::var("MCTK_TEST_JAVA_REGISTRY"),
        ) else {
            return;
        };
        let target = Registry::from_text(&std::fs::read_to_string(reg).unwrap()).unwrap();
        let scan = Scan::java_world(world).unwrap();
        assert!(scan.chunks > 100, "scanned only {} chunks", scan.chunks);
        let bad = scan.unsupported(&target);
        assert!(bad.is_empty(), "{} states not exact, first: {:?}", bad.len(), bad.first());
        eprintln!("{} chunks, {} distinct states", scan.chunks, scan.blocks.len());
    }
}
