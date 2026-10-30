//! Edition detection and chunk listing for Java, Bedrock and Legacy Console worlds.

use std::collections::BTreeMap;
use std::path::Path;

use mctk_anvil::{ChunkPos, Region};
use mctk_bedrock::{BedrockWorld, ChunkTag, Key};
use mctk_console::{ConsoleRegion, ConsoleSave};
use mctk_java::JavaWorld;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edition {
    Java,
    Bedrock,
    Console,
}

impl Edition {
    pub fn label(self) -> &'static str {
        match self {
            Edition::Java => "Java Edition",
            Edition::Bedrock => "Bedrock Edition",
            Edition::Console => "Legacy Console Edition",
        }
    }
}

/// Guesses the edition of a world folder or file.
pub fn detect(path: &Path) -> Option<Edition> {
    if path.is_dir() {
        if path.join("db").is_dir() {
            return Some(Edition::Bedrock);
        }
        if path.join("GAMEDATA").is_file() {
            return Some(Edition::Console);
        }
        if path.join("level.dat").is_file() {
            return Some(Edition::Java);
        }
        return None;
    }
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".mcworld") {
        Some(Edition::Bedrock)
    } else if name == "gamedata" || name.ends_with(".dat") || name.ends_with(".bin") {
        Some(Edition::Console)
    } else {
        None
    }
}

/// Chunk positions per dimension name.
pub type ChunkList = BTreeMap<String, Vec<ChunkPos>>;

pub fn list_chunks(path: &Path, edition: Edition) -> Result<ChunkList, String> {
    let mut out = ChunkList::new();
    match edition {
        Edition::Java => {
            let world = JavaWorld::open(path).map_err(|e| e.to_string())?;
            for dim in &world.dimensions {
                let list = out.entry(dim.id.clone()).or_default();
                for rpos in dim.regions().map_err(|e| e.to_string())? {
                    let region = Region::open(dim.region_dir().join(rpos.file_name()))
                        .map_err(|e| e.to_string())?;
                    list.extend(region.chunks().map(|(lx, lz)| rpos.chunk(lx, lz)));
                }
            }
        }
        Edition::Bedrock => {
            let world = BedrockWorld::open(path).map_err(|e| e.to_string())?;
            let mut seen = std::collections::BTreeSet::new();
            for k in world.db.entries.keys() {
                if let Key::Chunk(c) = Key::decode(k) {
                    if c.tag == ChunkTag::SubChunkPrefix {
                        seen.insert((c.dimension.id(), c.x, c.z));
                    }
                }
            }
            for (d, x, z) in seen {
                let name = [
                    "minecraft:overworld",
                    "minecraft:the_nether",
                    "minecraft:the_end",
                ][d as usize];
                out.entry(name.to_string())
                    .or_default()
                    .push(ChunkPos::new(x, z));
            }
        }
        Edition::Console => {
            let save = ConsoleSave::open(path).map_err(|e| e.to_string())?;
            for (dim, rx, rz, e) in save.archive.regions() {
                if e.data.is_empty() {
                    continue;
                }
                let region = ConsoleRegion::parse(e.data.clone()).map_err(|e| e.to_string())?;
                let name = match dim {
                    -1 => "minecraft:the_nether",
                    1 => "minecraft:the_end",
                    _ => "minecraft:overworld",
                };
                out.entry(name.to_string()).or_default().extend(
                    region
                        .chunks()
                        .map(|(lx, lz)| ChunkPos::new(rx * 32 + lx as i32, rz * 32 + lz as i32)),
                );
            }
        }
    }
    Ok(out)
}
