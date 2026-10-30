use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mctk_anvil::{ChunkPos, Region, RegionPos};
use mctk_nbt::Flavor;

use crate::{Chunk, Error, LevelInfo};

/// One dimension's storage folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dimension {
    /// Namespaced id, e.g. `minecraft:overworld`.
    pub id: String,
    pub root: PathBuf,
}

impl Dimension {
    pub fn region_dir(&self) -> PathBuf {
        self.root.join("region")
    }
    pub fn entities_dir(&self) -> PathBuf {
        self.root.join("entities")
    }
    pub fn poi_dir(&self) -> PathBuf {
        self.root.join("poi")
    }

    /// Regions present in the dimension's `region/` folder.
    pub fn regions(&self) -> Result<Vec<RegionPos>, Error> {
        let mut out = Vec::new();
        let dir = self.region_dir();
        if !dir.is_dir() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(dir)? {
            if let Some(pos) = entry?
                .file_name()
                .to_str()
                .and_then(RegionPos::from_file_name)
            {
                out.push(pos);
            }
        }
        out.sort();
        Ok(out)
    }
}

/// A Java world folder.
#[derive(Debug)]
pub struct JavaWorld {
    pub path: PathBuf,
    pub level: LevelInfo,
    pub dimensions: Vec<Dimension>,
    cache: Mutex<HashMap<(String, RegionPos), Option<Region>>>,
}

impl JavaWorld {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        let level_dat = path.join("level.dat");
        if !level_dat.is_file() {
            return Err(Error::NotAWorld(format!(
                "{} has no level.dat",
                path.display()
            )));
        }
        let (_, root) = mctk_nbt::read_file(&level_dat, Flavor::Java)?;
        let level = LevelInfo::from_nbt(root);
        let dimensions = discover_dimensions(&path)?;
        Ok(JavaWorld {
            path,
            level,
            dimensions,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn dimension(&self, id: &str) -> Option<&Dimension> {
        self.dimensions.iter().find(|d| d.id == id)
    }

    /// Reads and parses one chunk, or `None` when it was never generated.
    pub fn chunk(&self, dimension: &str, pos: ChunkPos) -> Result<Option<Chunk>, Error> {
        let dim = self
            .dimension(dimension)
            .ok_or_else(|| Error::NotAWorld(format!("no dimension {dimension}")))?;
        let rpos = pos.region();
        let mut cache = self.cache.lock().expect("cache lock");
        let key = (dim.id.clone(), rpos);
        if !cache.contains_key(&key) {
            let file = dim.region_dir().join(rpos.file_name());
            let region = if file.is_file() {
                Some(Region::open(file)?)
            } else {
                None
            };
            cache.insert(key.clone(), region);
        }
        let Some(region) = cache.get(&key).and_then(Option::as_ref) else {
            return Ok(None);
        };
        let (lx, lz) = pos.local();
        match region.read_chunk(lx, lz)? {
            Some(root) => Ok(Some(Chunk::from_nbt(root)?)),
            None => Ok(None),
        }
    }
}

/// Finds dimensions in both layouts: legacy (`region/`, `DIM-1/`, `DIM1/`) and the
/// `dimensions/<namespace>/<name>/` folders used for custom dimensions and, since 26.x,
/// for the vanilla ones as well.
fn discover_dimensions(world: &Path) -> Result<Vec<Dimension>, Error> {
    let mut dims = Vec::new();
    for (id, rel) in [
        ("minecraft:overworld", ""),
        ("minecraft:the_nether", "DIM-1"),
        ("minecraft:the_end", "DIM1"),
    ] {
        let root = world.join(rel);
        if root.join("region").is_dir() {
            dims.push(Dimension {
                id: id.into(),
                root,
            });
        }
    }
    let ns_root = world.join("dimensions");
    if ns_root.is_dir() {
        for ns in std::fs::read_dir(&ns_root)? {
            let ns = ns?;
            if !ns.path().is_dir() {
                continue;
            }
            for name in std::fs::read_dir(ns.path())? {
                let name = name?;
                let root = name.path();
                if !root.is_dir() {
                    continue;
                }
                let id = format!(
                    "{}:{}",
                    ns.file_name().to_string_lossy(),
                    name.file_name().to_string_lossy()
                );
                // Keep the folder that actually holds chunks when both layouts exist.
                match dims.iter_mut().find(|d| d.id == id) {
                    Some(existing)
                        if !existing.region_dir().is_dir() || root.join("region").is_dir() =>
                    {
                        existing.root = root
                    }
                    Some(_) => {}
                    None => dims.push(Dimension { id, root }),
                }
            }
        }
    }
    dims.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(dims)
}
