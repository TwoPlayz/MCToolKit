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
    Bedrock {
        subs: BTreeMap<i8, SubChunk>,
        dirty: BTreeSet<i8>,
        old: bool,
        /// The biomes by 4 x 4 x 4 cells (1.18 and later: Data3D), or per column (before: Data2D); iomes_dirty once one was set.
        data3d: Option<mctk_bedrock::Data3D>,
        data2d: Option<Vec<u8>>,
        biomes_dirty: bool,
    },
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

/// Sets one 4 x 4 x 4 cell of a Bedrock subchunk's biomes (`sec` counts subchunks from the lowest; `y4` is the cell's height
/// in it, 0 to 3). A subchunk that said "same as below" is written out first, and so are the ones above it that were
/// copies of it, so only the cell changes.
fn bedrock_set_cell(d: &mut mctk_bedrock::Data3D, sec: usize, x4: usize, y4: usize, z4: usize, id: i32) -> bool {
    use mctk_bedrock::BiomeStorage;
    if sec >= d.biomes.len() {
        return false;
    }
    let own = match &d.biomes[sec] {
        BiomeStorage::CopyPrevious => (0..sec).rev().find_map(|j| matches!(d.biomes[j], BiomeStorage::Palettized { .. }).then(|| d.biomes[j].clone())),
        s => Some(s.clone()),
    };
    let own = own.unwrap_or(BiomeStorage::Palettized { palette: vec![id], indices: vec![0; 4096], bits: 0, flag: true });
    for j in sec + 1..d.biomes.len() {
        if d.biomes[j] != BiomeStorage::CopyPrevious {
            break;
        }
        d.biomes[j] = own.clone();
    }
    let BiomeStorage::Palettized { mut palette, mut indices, bits, flag } = own else { return false };
    let at = match palette.iter().position(|p| *p == id) {
        Some(i) => i,
        None => {
            palette.push(id);
            palette.len() - 1
        }
    };
    for dy in 0..4 {
        for dz in 0..4 {
            for dx in 0..4 {
                indices[mctk_bedrock::block_index(x4 * 4 + dx, y4 * 4 + dy, z4 * 4 + dz)] = at as u16;
            }
        }
    }
    d.biomes[sec] = BiomeStorage::Palettized { palette, indices, bits, flag };
    true
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
                let data3d = ed.bedrock(&ed.bedrock_key(cx, cz, ChunkTag::Data3D, None)).and_then(|r| mctk_bedrock::Data3D::parse(&r).ok());
                let data2d = ed.bedrock(&ed.bedrock_key(cx, cz, ChunkTag::Data2D, None)).filter(|r| r.len() >= 768);
                (Kind::Bedrock { subs, dirty: BTreeSet::new(), old, data3d, data2d, biomes_dirty: false }, lo * 16, hi * 16 + 15)
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
            Kind::Bedrock { subs, dirty, old, .. } => {
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

    /// The biome (a Java id) of one cell: see `set_biome_cell` for what `x4`, `y4` and `z4` are. `None` where the chunk has none there.
    pub fn biome_cell(&self, x4: usize, y4: i32, z4: usize) -> Option<String> {
        match &self.kind {
            Kind::Java { chunk, .. } => {
                if chunk.era == mctk_java::ChunkEra::Modern {
                    let b = chunk.sections.iter().find(|s| i32::from(s.y) == y4.div_euclid(4))?.biomes.as_ref()?;
                    return Some(b.get((y4.rem_euclid(4) as usize * 4 + z4) * 4 + x4).clone());
                }
                let root = chunk.to_nbt();
                let level = root.tag.get("Level")?;
                let id = match level.get("Biomes")? {
                    Tag::IntArray(a) if a.len() == 1024 => *a.get((usize::try_from(y4).ok()? * 4 + z4) * 4 + x4)?,
                    Tag::IntArray(a) if a.len() == 256 => *a.get((z4 * 4) * 16 + x4 * 4)?,
                    Tag::ByteArray(a) if a.len() == 256 => i32::from(*a.get((z4 * 4) * 16 + x4 * 4)? as u8),
                    _ => return None,
                };
                mctk_mapping::legacy::legacy().biome(id).map(str::to_string)
            }
            Kind::Bedrock { data3d, data2d, .. } => {
                let id = if let Some(d) = data3d {
                    let sec = usize::try_from(y4.div_euclid(4) - self.ylo.div_euclid(16)).ok()?;
                    let mut at = sec;
                    loop {
                        match d.biomes.get(at)? {
                            mctk_bedrock::BiomeStorage::Palettized { palette, indices, .. } => {
                                let i = indices[mctk_bedrock::block_index(x4 * 4, y4.rem_euclid(4) as usize * 4, z4 * 4)] as usize;
                                break *palette.get(i)?;
                            }
                            mctk_bedrock::BiomeStorage::CopyPrevious if at > 0 => at -= 1,
                            mctk_bedrock::BiomeStorage::CopyPrevious => return None,
                        }
                    }
                } else {
                    i32::from(*data2d.as_ref()?.get(512 + (z4 * 4) * 16 + x4 * 4)?)
                };
                mctk_mapping::mappings().bedrock_biome_to_java(id).map(str::to_string)
            }
            Kind::Console { chunk } => chunk.biome(x4 * 4, z4 * 4).map(mctk_convert::console_biome_name),
        }
    }

    /// Sets the biome of one 4 x 4 x 4 block cell: `x4` and `z4` are the cell's place in the chunk (0 to 3), `y4` its height in
    /// cells from y 0 (a cell is the blocks from `y4 * 4` to `y4 * 4 + 3`). Every edition keeps biomes its own way: in cells of
    /// 4 x 4 x 4 (Java 1.18 and later, Bedrock 1.18 and later), 4 x 4 x 4 over 256 blocks (Java 1.15 to 1.17), or per column
    /// (older Java, older Bedrock, the console games: then the cell's columns, at every height). False where there is nothing to set.
    pub fn set_biome_cell(&mut self, x4: usize, y4: i32, z4: usize, biome: &str) -> bool {
        let ylo = self.ylo;
        let done = match &mut self.kind {
            Kind::Java { chunk, .. } => {
                if chunk.era == mctk_java::ChunkEra::Modern {
                    let sy = y4.div_euclid(4);
                    let Some(b) = chunk.sections.iter_mut().find(|s| i32::from(s.y) == sy).and_then(|s| s.biomes.as_mut()) else { return false };
                    let i = (y4.rem_euclid(4) as usize * 4 + z4) * 4 + x4;
                    if b.get(i).as_str() != biome {
                        b.set(i, biome.to_string());
                    }
                    true
                } else {
                    // Before 1.18: a list of numbers in `Level`: 1024 (1.15 on: 4 x 4 x 4 cells up to 256), or 256 (a number a column).
                    let id = mctk_convert::old_java_biome_id(biome);
                    let level = chunk.body_mut();
                    match level.get_mut("Biomes") {
                        Some(Tag::IntArray(a)) if a.len() == 1024 => {
                            if !(0..64).contains(&y4) {
                                return false;
                            }
                            a[(y4 as usize * 4 + z4) * 4 + x4] = id;
                            true
                        }
                        Some(Tag::IntArray(a)) if a.len() == 256 => {
                            for dz in 0..4 {
                                for dx in 0..4 {
                                    a[(z4 * 4 + dz) * 16 + x4 * 4 + dx] = id;
                                }
                            }
                            true
                        }
                        Some(Tag::ByteArray(a)) if a.len() == 256 => {
                            for dz in 0..4 {
                                for dx in 0..4 {
                                    a[(z4 * 4 + dz) * 16 + x4 * 4 + dx] = id as i8;
                                }
                            }
                            true
                        }
                        _ => false,
                    }
                }
            }
            Kind::Bedrock { data3d, data2d, biomes_dirty, .. } => {
                let Some(id) = mctk_mapping::mappings().java_biome_to_bedrock(biome) else { return false };
                if let Some(d) = data3d {
                    let sec = y4.div_euclid(4) - ylo.div_euclid(16);
                    let Ok(sec) = usize::try_from(sec) else { return false };
                    if !bedrock_set_cell(d, sec, x4, y4.rem_euclid(4) as usize, z4, id) {
                        return false;
                    }
                } else if let Some(r) = data2d {
                    // Before 1.18: 512 bytes of heights, then a biome number for each of the 256 columns.
                    for dz in 0..4 {
                        for dx in 0..4 {
                            r[512 + (z4 * 4 + dz) * 16 + x4 * 4 + dx] = id as u8;
                        }
                    }
                } else {
                    return false;
                }
                *biomes_dirty = true;
                true
            }
            Kind::Console { chunk } => {
                let Some(id) = mctk_convert::console_biome_id(biome) else { return false };
                let mut any = false;
                for dz in 0..4 {
                    for dx in 0..4 {
                        any |= chunk.set_biome(x4 * 4 + dx, z4 * 4 + dz, id);
                    }
                }
                any
            }
        };
        if done {
            self.changed = true;
        }
        done
    }
    /// Puts `be` (as the edition keeps it, its `x`, `y`, `z` set) in place of the block entity at its position.
    pub fn replace_block_entity(&mut self, be: Compound) {
        let n = |k: &str| be.get(k).and_then(Tag::as_i64).map(|v| v as i32);
        if let (Some(x), Some(y), Some(z)) = (n("x"), n("y"), n("z")) {
            self.gone_be.insert([x, y, z]);
        }
        self.new_be.push(be);
        self.changed = true;
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
            Kind::Bedrock { mut subs, dirty, data3d, data2d, biomes_dirty, .. } => {
                if biomes_dirty {
                    if let Some(d) = &data3d {
                        ed.layer.set_bedrock(id.clone(), ed.bedrock_key(self.cx, self.cz, ChunkTag::Data3D, None), Some(d.encode()));
                    } else if let Some(r) = &data2d {
                        ed.layer.set_bedrock(id.clone(), ed.bedrock_key(self.cx, self.cz, ChunkTag::Data2D, None), Some(r.clone()));
                    }
                }
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

#[cfg(test)]
mod biome_tests {
    use super::*;
    use mctk_bedrock::{BiomeStorage, Data3D};

    fn plains(n: usize) -> BiomeStorage {
        BiomeStorage::Palettized { palette: vec![1; 1].into_iter().take(n).collect(), indices: vec![0; 4096], bits: 0, flag: true }
    }

    /// A Bedrock cell is set without touching the cells round it, and a subchunk that was "the same as below" does not drag
    /// the ones above it along.
    #[test]
    fn a_bedrock_cell_is_painted_alone_and_copies_above_keep_their_biome() {
        let mut d = Data3D { heightmap: vec![0; 256], biomes: vec![plains(1), BiomeStorage::CopyPrevious, BiomeStorage::CopyPrevious] };
        assert!(bedrock_set_cell(&mut d, 0, 1, 2, 3, 12));
        let at = |d: &Data3D, sec: usize, x4: usize, y4: usize, z4: usize| match &d.biomes[sec] {
            BiomeStorage::Palettized { palette, indices, .. } => palette[indices[mctk_bedrock::block_index(x4 * 4 + 1, y4 * 4 + 2, z4 * 4 + 3)] as usize],
            BiomeStorage::CopyPrevious => -1,
        };
        assert_eq!(at(&d, 0, 1, 2, 3), 12, "the cell");
        assert_eq!(at(&d, 0, 0, 2, 3), 1, "its neighbour");
        assert_eq!(at(&d, 1, 1, 2, 3), 1, "the copy above kept the old biome");
        // Encodes and reads back.
        assert_eq!(Data3D::parse(&d.encode()).unwrap().biomes.len(), 3);
        // A subchunk that says "same as below" is written out when one of its cells is set.
        assert!(bedrock_set_cell(&mut d, 2, 0, 0, 0, 7));
        assert_eq!(at(&d, 2, 0, 0, 0), 7);
        assert_eq!(at(&d, 2, 1, 0, 0), 1);
        assert!(!bedrock_set_cell(&mut d, 9, 0, 0, 0, 7), "no such subchunk");
    }
}