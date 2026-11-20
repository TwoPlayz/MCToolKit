//! One chunk opened to read and set blocks by their Java states, whatever the edition keeps:
//! Java palettes (or numeric ids before 1.13), Bedrock subchunks (with the water layer for
//! waterlogged blocks), console numeric ids. Written back to the edit layer with
//! [`ChunkEdit::store`].

use std::collections::{BTreeMap, BTreeSet};

use mctk_bedrock::{BlockStorage, ChunkTag, SubChunk, SubChunkBody};
use mctk_java::BlockState;
use mctk_nbt::{Compound, List, NamedTag, Tag, TagType};

use super::chunks::{Base, Editor, bedrock_compounds, bedrock_record};
use super::layer::JavaChunk;

enum Kind {
    Java { chunk: Box<mctk_java::Chunk>, entities: Option<Option<NamedTag>> },
    Bedrock { subs: BTreeMap<i8, SubChunk>, dirty: BTreeSet<i8>, old: bool },
    Console { chunk: Box<mctk_console::ConsoleChunk> },
}

pub struct ChunkEdit {
    pub cx: i32,
    pub cz: i32,
    kind: Kind,
    /// The heights blocks can be at (inclusive).
    pub ylo: i32,
    pub yhi: i32,
    changed: bool,
    /// Block entities to take away (where blocks were set) and to add (pasted), as the edition
    /// keeps them, at world positions.
    gone_be: BTreeSet<[i32; 3]>,
    new_be: Vec<Compound>,
    /// Blocks the edition has nothing for (left as they were).
    pub unmapped: usize,
}

/// Bedrock's water, for the second layer of a waterlogged block.
fn bedrock_state(java: &BlockState) -> Option<NamedTag> {
    crate::world::java_bedrock_state(java)
}

fn bedrock_water() -> NamedTag {
    bedrock_state(&BlockState::new("minecraft:water").with("level", "0")).unwrap_or_else(|| {
        let mut c = Compound::new();
        c.insert("name".into(), Tag::String("minecraft:water".into()));
        let mut s = Compound::new();
        s.insert("liquid_depth".into(), Tag::Int(0));
        c.insert("states".into(), Tag::Compound(s));
        NamedTag { name: String::new(), tag: Tag::Compound(c) }
    })
}

fn bedrock_air() -> NamedTag {
    bedrock_state(&BlockState::air()).unwrap_or_else(|| {
        let mut c = Compound::new();
        c.insert("name".into(), Tag::String("minecraft:air".into()));
        c.insert("states".into(), Tag::Compound(Compound::new()));
        NamedTag { name: String::new(), tag: Tag::Compound(c) }
    })
}

impl ChunkEdit {
    /// Opens a chunk as it is now (edited or saved); `None` when there is none.
    pub fn load(ed: &Editor, cx: i32, cz: i32) -> Option<ChunkEdit> {
        let (kind, ylo, yhi) = match &ed.base {
            Base::Java { .. } => {
                let jc = ed.java(cx, cz)?;
                let chunk = mctk_java::Chunk::from_nbt(jc.root?).ok()?;
                // 1.18 (data version 2860) made the world reach from -64 to 319.
                let tall = chunk.data_version.is_some_and(|v| v >= 2860) || chunk.sections.iter().any(|s| s.y < 0);
                let (lo, hi) = if tall { (-64, 319) } else { (0, 255) };
                (Kind::Java { chunk: Box::new(chunk), entities: jc.entities }, lo, hi)
            }
            Base::Bedrock { dim, .. } => {
                let (lo, hi) = match dim {
                    mctk_bedrock::Dimension::Overworld => (-4i32, 19i32),
                    mctk_bedrock::Dimension::Nether => (0, 7),
                    mctk_bedrock::Dimension::End => (0, 15),
                };
                let mut subs = BTreeMap::new();
                let mut old = false;
                for sy in lo..=hi {
                    let key = ed.bedrock_key(cx, cz, ChunkTag::SubChunkPrefix, Some(sy as i8));
                    if let Some(raw) = ed.bedrock(&key) {
                        match SubChunk::parse(&raw) {
                            Ok(s) => {
                                old |= matches!(s.body, SubChunkBody::Legacy(_));
                                subs.insert(sy as i8, s);
                            }
                            Err(_) => old = true,
                        }
                    }
                }
                let exists = !subs.is_empty() || ed.bedrock(&ed.bedrock_key(cx, cz, ChunkTag::Version, None)).is_some() || ed.bedrock(&ed.bedrock_key(cx, cz, ChunkTag::LegacyVersion, None)).is_some();
                if !exists {
                    return None;
                }
                old |= ed.bedrock(&ed.bedrock_key(cx, cz, ChunkTag::LegacyTerrain, None)).is_some();
                (Kind::Bedrock { subs, dirty: BTreeSet::new(), old }, lo * 16, hi * 16 + 15)
            }
            Base::Console { .. } => {
                let chunk = mctk_console::ConsoleChunk::from_nbt(ed.console(cx, cz)?).ok()?;
                let hi = chunk.height as i32 - 1;
                (Kind::Console { chunk: Box::new(chunk) }, 0, hi)
            }
        };
        Some(ChunkEdit { cx, cz, kind, ylo, yhi, changed: false, gone_be: BTreeSet::new(), new_be: Vec::new(), unmapped: 0 })
    }

    /// A Java chunk's data version.
    pub fn data_version(&self) -> Option<i32> {
        match &self.kind {
            Kind::Java { chunk, .. } => chunk.data_version,
            _ => None,
        }
    }

    /// Whether this is a Bedrock chunk in a format from before 1.0 (left as it is).
    pub fn is_old_bedrock(&self) -> bool {
        matches!(self.kind, Kind::Bedrock { old: true, .. })
    }

    /// The block at chunk-local x, z and world y, as a Java state.
    pub fn get(&self, x: usize, y: i32, z: usize) -> BlockState {
        if y < self.ylo || y > self.yhi {
            return BlockState::air();
        }
        match &self.kind {
            Kind::Java { chunk, .. } => {
                if let Some(b) = chunk.block(x, y, z) {
                    return b.clone();
                }
                chunk.legacy_block(x, y, z).map_or_else(BlockState::air, |(id, d)| crate::world::legacy_java_state(id, d))
            }
            Kind::Bedrock { subs, .. } => {
                let Some(SubChunkBody::Palettized(st)) = subs.get(&(y.div_euclid(16) as i8)).map(|s| &s.body) else { return BlockState::air() };
                let i = mctk_bedrock::block_index(x, y.rem_euclid(16) as usize, z);
                let Some(first) = st.first() else { return BlockState::air() };
                let mut s = first.palette.get(first.indices[i] as usize).map_or_else(BlockState::air, crate::world::bedrock_java_state);
                let wet = st.get(1).and_then(|w| w.name(w.indices[i] as usize)).is_some_and(|n| n.ends_with("water"));
                if wet && s.properties.contains_key("waterlogged") {
                    s.properties.insert("waterlogged".into(), "true".into());
                }
                s
            }
            Kind::Console { chunk } => chunk.block(x, y as usize, z).map_or_else(BlockState::air, |(id, d)| crate::world::legacy_java_state(u16::from(id), d)),
        }
    }

    /// Sets the block at chunk-local x, z and world y (its block entity, if it had one, goes).
    /// False where the edition has no such block or the chunk cannot be changed.
    pub fn set(&mut self, x: usize, y: i32, z: usize, state: &BlockState) -> bool {
        if y < self.ylo || y > self.yhi {
            return false;
        }
        let done = match &mut self.kind {
            Kind::Java { chunk, .. } => {
                if chunk.era == mctk_java::ChunkEra::Legacy {
                    match mctk_mapping::legacy::legacy().numeric_state(state).filter(|_| !state.is_air()).or(state.is_air().then_some((0, 0))) {
                        Some((id, d)) => chunk.set_legacy_block(x, y, z, id, d),
                        None => false,
                    }
                } else {
                    chunk.set_block(x, y, z, state)
                }
            }
            Kind::Bedrock { subs, dirty, old } => {
                if *old {
                    return false;
                }
                let Some(b) = bedrock_state(state) else {
                    self.unmapped += 1;
                    return false;
                };
                let sy = y.div_euclid(16) as i8;
                if state.is_air() && !subs.contains_key(&sy) {
                    return true;
                }
                let sub = subs.entry(sy).or_insert_with(|| SubChunk::filled(sy, bedrock_air()));
                let SubChunkBody::Palettized(st) = &mut sub.body else { return false };
                let i = mctk_bedrock::block_index(x, y.rem_euclid(16) as usize, z);
                st[0].set(i, &b);
                let wet = state.properties.get("waterlogged").is_some_and(|v| v == "true") || matches!(state.name.as_str(), "minecraft:seagrass" | "minecraft:tall_seagrass" | "minecraft:kelp" | "minecraft:kelp_plant" | "minecraft:bubble_column");
                if wet {
                    if st.len() < 2 {
                        st.push(BlockStorage::filled(bedrock_air()));
                    }
                    st[1].set(i, &bedrock_water());
                } else if st.len() >= 2 {
                    st[1].set(i, &bedrock_air());
                }
                if st.len() > 1 && sub.version == 1 {
                    sub.version = 8;
                }
                dirty.insert(sy);
                true
            }
            Kind::Console { chunk } => {
                let numeric = if state.is_air() { Some((0, 0)) } else { mctk_mapping::legacy::legacy().numeric_state(state) };
                match numeric.filter(|(id, _)| *id <= 255) {
                    Some((id, d)) => chunk.set_block(x, y as usize, z, id as u8, d).is_ok(),
                    None => {
                        self.unmapped += 1;
                        false
                    }
                }
            }
        };
        if done {
            self.changed = true;
            self.gone_be.insert([self.cx * 16 + x as i32, y, self.cz * 16 + z as i32]);
        }
        done
    }

    /// The block entities in a box (world positions, inclusive), as the edition keeps them.
    pub fn block_entities_in(&self, ed: &Editor, lo: [i32; 3], hi: [i32; 3]) -> Vec<Compound> {
        let inside = |c: &Compound| {
            let n = |k: &str| c.get(k).and_then(Tag::as_i64).map(|v| v as i32);
            matches!((n("x"), n("y"), n("z")), (Some(x), Some(y), Some(z)) if (lo[0]..=hi[0]).contains(&x) && (lo[1]..=hi[1]).contains(&y) && (lo[2]..=hi[2]).contains(&z))
        };
        let list: Vec<Compound> = match &self.kind {
            Kind::Java { chunk, .. } => {
                let root = chunk.to_nbt();
                let top = root.tag.as_compound().cloned().unwrap_or_default();
                let body = top.get("Level").and_then(Tag::as_compound).cloned().unwrap_or(top);
                ["block_entities", "TileEntities"].iter().filter_map(|k| body.get(*k).and_then(Tag::as_list)).flat_map(|l| l.iter().filter_map(Tag::as_compound).cloned()).collect()
            }
            Kind::Bedrock { .. } => ed.bedrock(&ed.bedrock_key(self.cx, self.cz, ChunkTag::BlockEntity, None)).map(|r| bedrock_compounds(&r)).unwrap_or_default(),
            Kind::Console { chunk } => chunk.to_nbt().tag.get("Level").and_then(|l| l.get("TileEntities")).and_then(Tag::as_list).map(|l| l.iter().filter_map(Tag::as_compound).cloned().collect()).unwrap_or_default(),
        };
        list.into_iter().filter(|c| inside(c)).collect()
    }

    /// Adds a block entity (as the edition keeps it, its `x`, `y`, `z` set).
    pub fn add_block_entity(&mut self, be: Compound) {
        self.new_be.push(be);
        self.changed = true;
    }

    /// Writes the chunk into the edit layer, if anything changed.
    pub fn store(mut self, ed: &mut Editor) -> bool {
        if !self.changed {
            return false;
        }
        let gone = std::mem::take(&mut self.gone_be);
        let added = std::mem::take(&mut self.new_be);
        let keep = |c: &Compound| {
            let n = |k: &str| c.get(k).and_then(Tag::as_i64).map(|v| v as i32);
            match (n("x"), n("y"), n("z")) {
                (Some(x), Some(y), Some(z)) => !gone.contains(&[x, y, z]),
                _ => true,
            }
        };
        let fix_list = |parent: &mut Compound, key: &str| {
            let mut items: Vec<Tag> = parent.get(key).and_then(Tag::as_list).map(|l| l.iter().filter(|t| t.as_compound().is_none_or(keep)).cloned().collect()).unwrap_or_default();
            items.extend(added.iter().cloned().map(Tag::Compound));
            if !items.is_empty() || parent.contains_key(key) {
                parent.insert(key.into(), Tag::List(List::from_tags(TagType::Compound, items).unwrap_or_else(|_| List::new(TagType::Compound))));
            }
        };
        let id = ed.id(self.cx, self.cz);
        match self.kind {
            Kind::Java { mut chunk, entities } => {
                let modern = chunk.era == mctk_java::ChunkEra::Modern;
                fix_list(chunk.body_mut(), if modern { "block_entities" } else { "TileEntities" });
                chunk.finish_edit();
                ed.layer.set_java(id, JavaChunk { root: Some(chunk.to_nbt()), entities });
            }
            Kind::Bedrock { mut subs, dirty, .. } => {
                for sy in dirty {
                    let Some(sub) = subs.get_mut(&sy) else { continue };
                    if let SubChunkBody::Palettized(st) = &mut sub.body {
                        for s in st.iter_mut() {
                            s.compact();
                        }
                        // A water layer with no water left goes.
                        if st.len() > 1 && st[1].palette.iter().all(|p| p.tag.get("name").and_then(Tag::as_str) == Some("minecraft:air")) {
                            st.truncate(1);
                        }
                    }
                    if let Ok(bytes) = sub.encode() {
                        let key = ed.bedrock_key(self.cx, self.cz, ChunkTag::SubChunkPrefix, Some(sy));
                        ed.layer.set_bedrock(id.clone(), key, Some(bytes));
                    }
                }
                let key = ed.bedrock_key(self.cx, self.cz, ChunkTag::BlockEntity, None);
                let before = ed.bedrock(&key).map(|r| bedrock_compounds(&r)).unwrap_or_default();
                let mut list: Vec<Compound> = before.iter().filter(|c| keep(c)).cloned().collect();
                list.extend(added);
                if list != before {
                    ed.layer.set_bedrock(id, key, bedrock_record(&list));
                }
            }
            Kind::Console { chunk } => {
                let height = chunk.height;
                let mut root = chunk.to_nbt().clone();
                if let Some(level) = root.tag.as_compound_mut().and_then(|c| c.get_mut("Level")).and_then(Tag::as_compound_mut) {
                    fix_list(level, "TileEntities");
                    // The height map again (the light is worked out on saving).
                    if let Some(Tag::ByteArray(blocks)) = level.get("Blocks").cloned() {
                        let mut h = vec![0i8; 256];
                        for z in 0..16 {
                            for x in 0..16 {
                                let top = (0..height).rev().find(|&y| blocks[mctk_console::console_block_index(x, y, z)] != 0).map_or(0, |y| y + 1);
                                h[z * 16 + x] = top.min(255) as u8 as i8;
                            }
                        }
                        level.insert("HeightMap".into(), Tag::ByteArray(h));
                    }
                }
                ed.layer.set_console(id, Some(root));
            }
        }
        true
    }
}
