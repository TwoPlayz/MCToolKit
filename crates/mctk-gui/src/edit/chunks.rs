//! Reading chunks to change them, through the edit layer (what was changed already) and else
//! from the world, in each edition's own form.

use std::path::PathBuf;

use mctk_anvil::{ChunkPos, Region};
use mctk_bedrock::{ChunkKey, ChunkTag, Dimension, Key};
use mctk_nbt::{Flavor, NamedTag, Tag};

use super::layer::{ChunkId, EditLayer, JavaChunk};

/// Where a world's chunks are read from.
pub enum Base<'a> {
    Java { region_dir: PathBuf, entities_dir: PathBuf },
    Bedrock { world: &'a mctk_bedrock::BedrockWorld, dim: Dimension },
    /// `dim` is the console dimension number (0, -1 the Nether, 1 the End).
    Console { save: &'a mctk_console::ConsoleSave, dim: i32 },
}

/// A tool's access to a world: the edit layer it writes to and the world under it.
pub struct Editor<'a> {
    pub layer: &'a mut EditLayer,
    pub base: Base<'a>,
    /// The dimension being edited (`minecraft:overworld`...).
    pub dim: String,
    /// Things the tool could not do, for the user.
    pub notes: Vec<String>,
}

impl Editor<'_> {
    pub fn id(&self, cx: i32, cz: i32) -> ChunkId {
        (self.dim.clone(), cx, cz)
    }

    // --- Java ------------------------------------------------------------------------------

    fn region_chunk(dir: &std::path::Path, cx: i32, cz: i32) -> Option<NamedTag> {
        let pos = ChunkPos::new(cx, cz);
        let (lx, lz) = pos.local();
        Region::open(dir.join(pos.region().file_name())).ok()?.read_chunk(lx, lz).ok().flatten()
    }

    /// A Java chunk as it is now (changed or as saved); `None` when there is none.
    pub fn java(&self, cx: i32, cz: i32) -> Option<JavaChunk> {
        if let Some(j) = self.layer.java.get(&self.id(cx, cz)) {
            return j.root.is_some().then(|| j.clone());
        }
        let Base::Java { region_dir, .. } = &self.base else { return None };
        Some(JavaChunk { root: Some(Self::region_chunk(region_dir, cx, cz)?), entities: None })
    }

    /// A Java chunk's entities' record (1.17 and later), as it is now.
    pub fn java_entities(&self, cx: i32, cz: i32, chunk: &JavaChunk) -> Option<NamedTag> {
        if let Some(e) = &chunk.entities {
            return e.clone();
        }
        let Base::Java { entities_dir, .. } = &self.base else { return None };
        Self::region_chunk(entities_dir, cx, cz)
    }

    // --- Console ---------------------------------------------------------------------------

    /// A console chunk as it is now; `None` when there is none.
    pub fn console(&self, cx: i32, cz: i32) -> Option<NamedTag> {
        if let Some(c) = self.layer.console.get(&self.id(cx, cz)) {
            return c.clone();
        }
        let Base::Console { save, dim } = &self.base else { return None };
        let (rx, rz) = (cx.div_euclid(32), cz.div_euclid(32));
        let entry = save.archive.regions().into_iter().find(|(d, x, z, _)| d == dim && *x == rx && *z == rz)?.3;
        let r = mctk_console::ConsoleRegion::parse(entry.data.clone()).ok()?;
        r.raw_chunk(cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize).ok()??.nbt().ok()
    }

    // --- Bedrock ---------------------------------------------------------------------------

    pub fn bedrock_dim(&self) -> Option<Dimension> {
        match &self.base {
            Base::Bedrock { dim, .. } => Some(*dim),
            _ => None,
        }
    }

    pub fn bedrock_key(&self, cx: i32, cz: i32, tag: ChunkTag, subchunk: Option<i8>) -> Vec<u8> {
        let dimension = self.bedrock_dim().unwrap_or(Dimension::Overworld);
        ChunkKey { x: cx, z: cz, dimension, tag, subchunk }.encode()
    }

    /// A Bedrock record as it is now.
    pub fn bedrock(&self, key: &[u8]) -> Option<Vec<u8>> {
        if let Some(v) = self.layer.bedrock.get(key) {
            return v.clone();
        }
        let Base::Bedrock { world, .. } = &self.base else { return None };
        world.db.get(key).map(<[u8]>::to_vec)
    }

    /// Every record of a Bedrock chunk that exists now: its own (by its coordinates) and its
    /// entities' (listed in its `digp` record).
    pub fn bedrock_chunk_keys(&self, cx: i32, cz: i32) -> Vec<Vec<u8>> {
        let Base::Bedrock { world, dim } = &self.base else { return Vec::new() };
        let mut prefix = Vec::with_capacity(12);
        prefix.extend_from_slice(&cx.to_le_bytes());
        prefix.extend_from_slice(&cz.to_le_bytes());
        if *dim != Dimension::Overworld {
            prefix.extend_from_slice(&dim.id().to_le_bytes());
        }
        let ours = |k: &[u8]| k.starts_with(&prefix) && (k.len() == prefix.len() + 1 || k.len() == prefix.len() + 2);
        let mut keys: Vec<Vec<u8>> = world.db.entries.range(prefix.clone()..).take_while(|(k, _)| k.starts_with(&prefix)).map(|(k, _)| k.clone()).filter(|k| ours(k)).collect();
        keys.extend(self.layer.bedrock.keys().filter(|k| ours(k)).cloned());
        let digp = Key::ActorDigest { x: cx, z: cz, dimension: *dim }.encode();
        if let Some(list) = self.bedrock(&digp) {
            keys.push(digp);
            keys.extend(list.chunks_exact(8).map(|uid| Key::Actor(uid.to_vec()).encode()));
        }
        keys.sort();
        keys.dedup();
        keys.retain(|k| self.bedrock(k).is_some());
        keys
    }
}

/// The NBT compounds stored one after another in a Bedrock record.
pub fn bedrock_compounds(raw: &[u8]) -> Vec<mctk_nbt::Compound> {
    let mut out = Vec::new();
    let mut s = raw;
    while !s.is_empty() {
        let Ok(root) = mctk_nbt::read_root(&mut s, Flavor::Bedrock) else { break };
        if let Tag::Compound(c) = root.tag {
            out.push(c);
        }
    }
    out
}

/// Compounds written one after another, as a Bedrock record holds them; `None` for none.
pub fn bedrock_record(items: &[mctk_nbt::Compound]) -> Option<Vec<u8>> {
    if items.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for c in items {
        out.extend(mctk_nbt::to_bytes(&NamedTag { name: String::new(), tag: Tag::Compound(c.clone()) }, Flavor::Bedrock).ok()?);
    }
    Some(out)
}
