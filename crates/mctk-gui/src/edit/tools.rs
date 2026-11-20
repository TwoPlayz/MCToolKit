//! The World Explorer's tools, each working on every edition's own chunk data through an
//! [`Editor`]: clearing the blocks of a box, removing the entities in it, deleting chunks.

use mctk_bedrock::{ChunkTag, Key, SubChunk, SubChunkBody};
use mctk_nbt::{Compound, List, NamedTag, Tag, TagType};

use mctk_java::BlockState;

use super::blocks::ChunkEdit;
use super::chunks::{Base, Editor, bedrock_compounds, bedrock_record};
use super::layer::JavaChunk;
use super::transform::Placement;

/// An inclusive box of blocks.
pub type Bounds = ([i32; 3], [i32; 3]);

/// The chunks a box covers, with the part of the box in each (chunk-local x and z, 0–15).
fn chunk_parts(b: Bounds) -> Vec<(i32, i32, [usize; 2], [usize; 2])> {
    let (lo, hi) = b;
    let mut out = Vec::new();
    for cz in lo[2].div_euclid(16)..=hi[2].div_euclid(16) {
        for cx in lo[0].div_euclid(16)..=hi[0].div_euclid(16) {
            let x0 = (lo[0] - cx * 16).clamp(0, 15) as usize;
            let x1 = (hi[0] - cx * 16).clamp(0, 15) as usize;
            let z0 = (lo[2] - cz * 16).clamp(0, 15) as usize;
            let z1 = (hi[2] - cz * 16).clamp(0, 15) as usize;
            out.push((cx, cz, [x0, x1], [z0, z1]));
        }
    }
    out
}

fn inside(b: Bounds, x: i64, y: i64, z: i64) -> bool {
    let (lo, hi) = b;
    (lo[0] as i64..=hi[0] as i64).contains(&x) && (lo[1] as i64..=hi[1] as i64).contains(&y) && (lo[2] as i64..=hi[2] as i64).contains(&z)
}

/// Whether a block entity (or tick) compound sits in the box (`x`, `y`, `z` ints).
fn at_in(c: &Compound, b: Bounds) -> bool {
    let n = |k: &str| c.get(k).and_then(Tag::as_i64);
    matches!((n("x"), n("y"), n("z")), (Some(x), Some(y), Some(z)) if inside(b, x, y, z))
}

/// Whether an entity stands in the box (by the block its `Pos` is in).
fn entity_in(c: &Compound, b: Bounds) -> bool {
    let p = c.get("Pos").and_then(Tag::as_list);
    let v = |i: usize| p.and_then(|l| l.get(i)).and_then(Tag::as_f64).map(|v| v.floor() as i64);
    matches!((v(0), v(1), v(2)), (Some(x), Some(y), Some(z)) if inside(b, x, y, z))
}

/// Keeps the compounds of a list `keep` says to; the number taken out.
fn retain_list(parent: &mut Compound, key: &str, keep: impl Fn(&Compound) -> bool) -> usize {
    let Some(Tag::List(l)) = parent.get(key) else { return 0 };
    let before = l.len();
    let kept: Vec<Tag> = l.iter().filter(|t| t.as_compound().is_none_or(&keep)).cloned().collect();
    let gone = before - kept.len();
    if gone > 0 {
        let ty = if kept.is_empty() { TagType::End } else { TagType::Compound };
        parent.insert(key.into(), Tag::List(List::from_tags(ty, kept).unwrap_or_else(|_| List::new(TagType::End))));
    }
    gone
}

// ---------------------------------------------------------------------------------------------
// Delete Blocks
// ---------------------------------------------------------------------------------------------

/// Makes every block in the box air, and removes the block entities and pending ticks there.
/// Gives how many chunks were changed.
pub fn delete_blocks(ed: &mut Editor, b: Bounds) -> usize {
    let mut changed = 0;
    for (cx, cz, xs, zs) in chunk_parts(b) {
        let done = match &ed.base {
            Base::Java { .. } => java_clear(ed, cx, cz, b, xs, zs),
            Base::Bedrock { .. } => bedrock_clear(ed, cx, cz, b, xs, zs),
            Base::Console { .. } => console_clear(ed, cx, cz, b, xs, zs),
        };
        changed += usize::from(done);
    }
    changed
}

fn java_clear(ed: &mut Editor, cx: i32, cz: i32, b: Bounds, xs: [usize; 2], zs: [usize; 2]) -> bool {
    let Some(jc) = ed.java(cx, cz) else { return false };
    let Some(root) = jc.root.clone() else { return false };
    let Ok(mut chunk) = mctk_java::Chunk::from_nbt(root) else {
        ed.notes.push(format!("Chunk {cx}, {cz} could not be read"));
        return false;
    };
    let legacy = chunk.era == mctk_java::ChunkEra::Legacy;
    let (ylo, yhi) = (b.0[1], b.1[1]);
    let air = mctk_java::BlockState::air();
    for y in ylo..=yhi {
        for z in zs[0]..=zs[1] {
            for x in xs[0]..=xs[1] {
                if legacy {
                    chunk.set_legacy_block(x, y, z, 0, 0);
                } else {
                    chunk.set_block(x, y, z, &air);
                }
            }
        }
    }
    let body = chunk.body_mut();
    for key in ["block_entities", "TileEntities", "block_ticks", "fluid_ticks", "TileTicks", "LiquidTicks"] {
        retain_list(body, key, |c| !at_in(c, b));
    }
    chunk.finish_edit();
    let id = ed.id(cx, cz);
    ed.layer.set_java(id, JavaChunk { root: Some(chunk.to_nbt()), entities: jc.entities });
    true
}

/// Bedrock's air, in the record version the subchunk's own palette uses.
fn bedrock_air(like: Option<&NamedTag>) -> NamedTag {
    let mut c = Compound::new();
    c.insert("name".into(), Tag::String("minecraft:air".into()));
    c.insert("states".into(), Tag::Compound(Compound::new()));
    let version = like.and_then(|p| p.tag.get("version")).cloned().unwrap_or(Tag::Int(18_100_737));
    c.insert("version".into(), version);
    NamedTag { name: String::new(), tag: Tag::Compound(c) }
}

fn bedrock_clear(ed: &mut Editor, cx: i32, cz: i32, b: Bounds, xs: [usize; 2], zs: [usize; 2]) -> bool {
    let id = ed.id(cx, cz);
    let mut any = false;
    let mut old_format = false;
    for sy in b.0[1].div_euclid(16)..=b.1[1].div_euclid(16) {
        let key = ed.bedrock_key(cx, cz, ChunkTag::SubChunkPrefix, Some(sy as i8));
        let Some(raw) = ed.bedrock(&key) else { continue };
        let Ok(mut sub) = SubChunk::parse(&raw) else { continue };
        let SubChunkBody::Palettized(storages) = &mut sub.body else {
            old_format = true;
            continue;
        };
        let (y0, y1) = ((b.0[1] - sy * 16).clamp(0, 15) as usize, (b.1[1] - sy * 16).clamp(0, 15) as usize);
        for st in storages.iter_mut() {
            let air = bedrock_air(st.palette.first());
            for y in y0..=y1 {
                for z in zs[0]..=zs[1] {
                    for x in xs[0]..=xs[1] {
                        st.set(mctk_bedrock::block_index(x, y, z), &air);
                    }
                }
            }
            st.compact();
        }
        if let Ok(bytes) = sub.encode() {
            ed.layer.set_bedrock(id.clone(), key, Some(bytes));
            any = true;
        }
    }
    if old_format || (!any && ed.bedrock(&ed.bedrock_key(cx, cz, ChunkTag::LegacyTerrain, None)).is_some()) {
        ed.notes.push(format!("Chunk {cx}, {cz} is in an old Bedrock format and was left as it is"));
    }
    // Block entities and pending ticks in the box go too.
    for tag in [ChunkTag::BlockEntity, ChunkTag::PendingTicks] {
        let key = ed.bedrock_key(cx, cz, tag, None);
        let Some(raw) = ed.bedrock(&key) else { continue };
        let all = bedrock_compounds(&raw);
        let kept: Vec<Compound> = all.iter().filter(|c| !at_in(c, b)).cloned().collect();
        if kept.len() != all.len() {
            ed.layer.set_bedrock(id.clone(), key, bedrock_record(&kept));
            any = true;
        }
    }
    any
}

fn console_clear(ed: &mut Editor, cx: i32, cz: i32, b: Bounds, xs: [usize; 2], zs: [usize; 2]) -> bool {
    let Some(root) = ed.console(cx, cz) else { return false };
    let Ok(mut chunk) = mctk_console::ConsoleChunk::from_nbt(root) else {
        ed.notes.push(format!("Chunk {cx}, {cz} could not be read"));
        return false;
    };
    let top = chunk.height as i32 - 1;
    for y in b.0[1].max(0)..=b.1[1].min(top) {
        for z in zs[0]..=zs[1] {
            for x in xs[0]..=xs[1] {
                let _ = chunk.set_block(x, y as usize, z, 0, 0);
            }
        }
    }
    let mut root = chunk.to_nbt().clone();
    if let Some(level) = root.tag.as_compound_mut().and_then(|c| c.get_mut("Level")).and_then(Tag::as_compound_mut) {
        for key in ["TileEntities", "TileTicks"] {
            retain_list(level, key, |c| !at_in(c, b));
        }
        console_relight(level, chunk.height, xs, zs);
    }
    let id = ed.id(cx, cz);
    ed.layer.set_console(id, Some(root));
    true
}

/// After blocks of a console chunk changed (the game does not light chunks again when it loads
/// them): for the columns changed, the height map again and full sky light above the highest
/// block.
fn console_relight(level: &mut Compound, height: usize, xs: [usize; 2], zs: [usize; 2]) {
    let Some(Tag::ByteArray(blocks)) = level.get("Blocks").cloned() else { return };
    let mut heights = match level.get("HeightMap") {
        Some(Tag::ByteArray(h)) if h.len() == 256 => h.clone(),
        _ => vec![0; 256],
    };
    let Some(Tag::ByteArray(sky)) = level.get_mut("SkyLight") else { return };
    for z in zs[0]..=zs[1] {
        for x in xs[0]..=xs[1] {
            let top = (0..height).rev().find(|&y| blocks[mctk_console::console_block_index(x, y, z)] != 0).map_or(0, |y| y + 1);
            heights[z * 16 + x] = top.min(255) as u8 as i8;
            for y in top..height {
                let i = mctk_console::console_block_index(x, y, z);
                if let Some(byte) = sky.get_mut(i / 2) {
                    let b = *byte as u8;
                    *byte = if i % 2 == 0 { (b & 0xf0) | 0x0f } else { (b & 0x0f) | 0xf0 } as i8;
                }
            }
        }
    }
    level.insert("HeightMap".into(), Tag::ByteArray(heights));
}

// ---------------------------------------------------------------------------------------------
// Delete Entities
// ---------------------------------------------------------------------------------------------

/// Removes the entities (mobs, items, minecarts, frames...) standing in the box. Gives how many.
pub fn delete_entities(ed: &mut Editor, b: Bounds) -> usize {
    let mut gone = 0;
    for (cx, cz, ..) in chunk_parts(b) {
        gone += match &ed.base {
            Base::Java { .. } => java_entities_out(ed, cx, cz, &|c| entity_in(c, b)),
            Base::Bedrock { .. } => bedrock_entities_out(ed, cx, cz, &|c| entity_in(c, b)),
            Base::Console { .. } => console_entities_out(ed, cx, cz, &|c| entity_in(c, b)),
        };
    }
    gone
}

/// Removes the one entity standing at pos (as the 3D view showed it: matched by where it is).
/// Gives whether it was found.
pub fn delete_entity_at(ed: &mut Editor, pos: [f64; 3]) -> bool {
    let (cx, cz) = ((pos[0].floor() as i32).div_euclid(16), (pos[2].floor() as i32).div_euclid(16));
    let near = |c: &Compound| entity_pos(c).is_some_and(|p| (0..3).all(|i| (p[i] - pos[i]).abs() < 0.01));
    let gone = match &ed.base {
        Base::Java { .. } => java_entities_out(ed, cx, cz, &near),
        Base::Bedrock { .. } => bedrock_entities_out(ed, cx, cz, &near),
        Base::Console { .. } => console_entities_out(ed, cx, cz, &near),
    };
    gone > 0
}

/// Changes the one entity standing at `pos` (matched as [`delete_entity_at`] does) with
/// `change`: it is taken out and put back changed. Gives whether it was found.
pub fn edit_entity_at(ed: &mut Editor, pos: [f64; 3], change: &dyn Fn(&mut Compound)) -> bool {
    let at = [pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32];
    let near = |c: &Compound| entity_pos(c).is_some_and(|p| (0..3).all(|i| (p[i] - pos[i]).abs() < 0.01));
    let Some(mut e) = entities_in(ed, (at, at)).into_iter().find(|c| near(c)) else { return false };
    if !delete_entity_at(ed, pos) {
        return false;
    }
    change(&mut e);
    add_entities(ed, vec![e]) > 0
}

/// Turns an entity a quarter turn clockwise (seen from above).
pub fn turn_entity(e: &mut Compound) {
    if let Some(Tag::List(l)) = e.get_mut("Rotation") {
        if let Some(Tag::Float(yaw)) = l.get_mut(0) {
            *yaw = (*yaw + 90.0).rem_euclid(360.0);
        }
    }
}

/// Makes an entity another kind (a Java id), keeping where it is and what else it has.
pub fn retype_entity(e: &mut Compound, java_id: &str) {
    let id = if java_id.contains(':') { java_id.to_string() } else { format!("minecraft:{java_id}") };
    if e.contains_key("identifier") {
        e.insert("identifier".into(), Tag::String(id));
    } else {
        e.insert("id".into(), Tag::String(id));
    }
}

/// Turns one block a quarter turn (its facing, axis...), keeping what it holds.
pub fn turn_block(ed: &mut Editor, at: [i32; 3]) -> Done {
    let clip = copy(ed, (at, at));
    paste(ed, &clip, at, Placement { turns: 1, mirror: false }, true)
}

/// Sets properties of one block (`waterlogged=true`...), keeping what it holds.
pub fn set_block_props(ed: &mut Editor, at: [i32; 3], props: &[(String, String)]) -> Done {
    let mut clip = copy(ed, (at, at));
    let slot = clip.blocks.first().copied().unwrap_or(0) as usize;
    if let Some(s) = clip.palette.get_mut(slot) {
        for (k, v) in props {
            s.properties.insert(k.clone(), v.clone());
        }
    }
    paste(ed, &clip, at, Placement::default(), true)
}

fn java_entities_out(ed: &mut Editor, cx: i32, cz: i32, hit: &dyn Fn(&Compound) -> bool) -> usize {
    let Some(mut jc) = ed.java(cx, cz) else { return 0 };
    let id = ed.id(cx, cz);
    // Before 1.17, in the chunk itself.
    let mut gone = 0;
    if let Some(level) = jc.root.as_mut().and_then(|r| r.tag.as_compound_mut()).and_then(|c| c.get_mut("Level")).and_then(Tag::as_compound_mut) {
        gone = retain_list(level, "Entities", |c| !hit(c));
    }
    if gone == 0 {
        if let Some(mut record) = ed.java_entities(cx, cz, &jc) {
            if let Some(c) = record.tag.as_compound_mut() {
                gone = retain_list(c, "Entities", |c| !hit(c));
            }
            if gone > 0 {
                jc.entities = Some(Some(record));
            }
        }
    }
    if gone > 0 {
        ed.layer.set_java(id, jc);
    }
    gone
}

fn bedrock_entities_out(ed: &mut Editor, cx: i32, cz: i32, hit: &dyn Fn(&Compound) -> bool) -> usize {
    let Some(dim) = ed.bedrock_dim() else { return 0 };
    let id = ed.id(cx, cz);
    let mut gone = 0;
    // Each entity in a record of its own, listed by the chunk (1.18.30 on).
    let digp = Key::ActorDigest { x: cx, z: cz, dimension: dim }.encode();
    if let Some(list) = ed.bedrock(&digp) {
        let mut kept: Vec<u8> = Vec::new();
        for uid in list.chunks_exact(8) {
            let key = Key::Actor(uid.to_vec()).encode();
            let here = ed.bedrock(&key).and_then(|raw| bedrock_compounds(&raw).into_iter().next()).is_some_and(|c| hit(&c));
            if here {
                ed.layer.set_bedrock(id.clone(), key, None);
                gone += 1;
            } else {
                kept.extend_from_slice(uid);
            }
        }
        if gone > 0 {
            ed.layer.set_bedrock(id.clone(), digp, (!kept.is_empty()).then_some(kept));
        }
    }
    // Before: all in one record of the chunk.
    let key = ed.bedrock_key(cx, cz, ChunkTag::Entity, None);
    if let Some(raw) = ed.bedrock(&key) {
        let all = bedrock_compounds(&raw);
        let kept: Vec<Compound> = all.iter().filter(|c| !hit(c)).cloned().collect();
        if kept.len() != all.len() {
            gone += all.len() - kept.len();
            ed.layer.set_bedrock(id, key, bedrock_record(&kept));
        }
    }
    gone
}

fn console_entities_out(ed: &mut Editor, cx: i32, cz: i32, hit: &dyn Fn(&Compound) -> bool) -> usize {
    let Some(mut root) = ed.console(cx, cz) else { return 0 };
    let Some(level) = root.tag.as_compound_mut().and_then(|c| c.get_mut("Level")).and_then(Tag::as_compound_mut) else { return 0 };
    let gone = retain_list(level, "Entities", |c| !hit(c));
    if gone > 0 {
        let id = ed.id(cx, cz);
        ed.layer.set_console(id, Some(root));
    }
    gone
}

// ---------------------------------------------------------------------------------------------
// Fill, Replace
// ---------------------------------------------------------------------------------------------

/// What a tool did: the chunks it changed, blocks set, and blocks the edition has nothing for.
#[derive(Default, Debug)]
pub struct Done {
    pub chunks: usize,
    pub blocks: usize,
    pub unmapped: usize,
}

/// Sets every block in the box that `pick` chooses (given its state now) to what it gives.
fn set_where(ed: &mut Editor, b: Bounds, pick: impl Fn(&BlockState) -> Option<BlockState>) -> Done {
    let mut done = Done::default();
    for (cx, cz, xs, zs) in chunk_parts(b) {
        let Some(mut c) = ChunkEdit::load(ed, cx, cz) else { continue };
        if c.is_old_bedrock() {
            ed.notes.push(format!("Chunk {cx}, {cz} is in an old Bedrock format and was left as it is"));
            continue;
        }
        for y in b.0[1].max(c.ylo)..=b.1[1].min(c.yhi) {
            for z in zs[0]..=zs[1] {
                for x in xs[0]..=xs[1] {
                    let now = c.get(x, y, z);
                    if let Some(new) = pick(&now) {
                        if new != now && c.set(x, y, z, &new) {
                            done.blocks += 1;
                        }
                    }
                }
            }
        }
        done.unmapped += c.unmapped;
        done.chunks += usize::from(c.store(ed));
    }
    done
}

/// Fills the box with a block.
pub fn fill(ed: &mut Editor, b: Bounds, state: &BlockState) -> Done {
    set_where(ed, b, |_| Some(state.clone()))
}

/// In the box, makes every block named as one of `from` (any state) the block `to`. Properties
/// both have (facing, half, waterlogged...) are kept when `keep_props` is set, so stairs
/// replaced by other stairs still face the same way.
pub fn replace(ed: &mut Editor, b: Bounds, from: &[String], to: &BlockState, keep_props: bool) -> Done {
    let full = |n: &str| if n.contains(':') { n.to_string() } else { format!("minecraft:{n}") };
    let from: Vec<String> = from.iter().map(|n| full(n)).collect();
    set_where(ed, b, |now| {
        if !from.contains(&now.name) {
            return None;
        }
        let mut new = to.clone();
        if keep_props {
            let complete = mctk_mapping::mappings().complete_java(to);
            for (k, v) in &now.properties {
                if complete.properties.contains_key(k) {
                    new.properties.insert(k.clone(), v.clone());
                }
            }
        }
        Some(new)
    })
}

// ---------------------------------------------------------------------------------------------
// Copy, Paste
// ---------------------------------------------------------------------------------------------

/// Blocks copied from a box: a palette of states and, per block, its place in it (index
/// `(y * size z + z) * size x + x`), with the block entities there as the edition keeps them
/// (by their place in the box).
#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    pub size: [i32; 3],
    pub palette: Vec<BlockState>,
    pub blocks: Vec<u32>,
    pub block_entities: Vec<([i32; 3], Compound)>,
    /// The kind of world the block entities are from (they are pasted only into the same kind).
    pub edition: String,
    /// The Java data version of the world copied from (for the files written from it).
    pub data_version: Option<i32>,
    /// The entities in the box (by their place in it, in blocks), as the edition keeps them.
    pub entities: Vec<([f64; 3], Compound)>,
    /// Where the box was in its world (its lowest north-west corner), for what the entities
    /// hold of their own place (the block a frame hangs on).
    pub origin: [i32; 3],
}

impl Clipboard {
    fn index(&self, p: [i32; 3]) -> usize {
        ((p[1] * self.size[2] + p[2]) * self.size[0] + p[0]) as usize
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get(&self, p: [i32; 3]) -> &BlockState {
        &self.palette[self.blocks[self.index(p)] as usize]
    }

    /// How many blocks are not air.
    pub fn solid(&self) -> usize {
        let air: Vec<bool> = self.palette.iter().map(BlockState::is_air).collect();
        self.blocks.iter().filter(|i| !air[**i as usize]).count()
    }
}

fn edition_of(ed: &Editor) -> &'static str {
    match ed.base {
        Base::Java { .. } => "java",
        Base::Bedrock { .. } => "bedrock",
        Base::Console { .. } => "console",
    }
}

/// Copies the blocks (and block entities) in a box.
pub fn copy(ed: &Editor, b: Bounds) -> Clipboard {
    let (lo, hi) = b;
    let size = [hi[0] - lo[0] + 1, hi[1] - lo[1] + 1, hi[2] - lo[2] + 1];
    let mut clip = Clipboard { size, palette: vec![BlockState::air()], blocks: vec![0; (size[0] * size[1] * size[2]) as usize], block_entities: Vec::new(), edition: edition_of(ed).into(), data_version: None, entities: Vec::new(), origin: lo };
    for e in entities_in(ed, b) {
        if let Some(p) = entity_pos(&e) {
            clip.entities.push(([p[0] - lo[0] as f64, p[1] - lo[1] as f64, p[2] - lo[2] as f64], e));
        }
    }
    let mut slots: std::collections::HashMap<BlockState, u32> = std::collections::HashMap::from([(BlockState::air(), 0)]);
    for (cx, cz, xs, zs) in chunk_parts(b) {
        let Some(c) = ChunkEdit::load(ed, cx, cz) else { continue };
        clip.data_version = clip.data_version.or(c.data_version());
        for y in lo[1]..=hi[1] {
            for z in zs[0]..=zs[1] {
                for x in xs[0]..=xs[1] {
                    let s = c.get(x, y, z);
                    let slot = *slots.entry(s.clone()).or_insert_with(|| {
                        clip.palette.push(s);
                        (clip.palette.len() - 1) as u32
                    });
                    let p = [cx * 16 + x as i32 - lo[0], y - lo[1], cz * 16 + z as i32 - lo[2]];
                    let i = clip.index(p);
                    clip.blocks[i] = slot;
                }
            }
        }
        for be in c.block_entities_in(ed, lo, hi) {
            let n = |k: &str| be.get(k).and_then(Tag::as_i64).unwrap_or(0) as i32;
            clip.block_entities.push(([n("x") - lo[0], n("y") - lo[1], n("z") - lo[2]], be));
        }
    }
    clip
}

/// Pastes a clipboard with its lowest north-west corner at `origin`, turned and mirrored as
/// `place` says; air in it is pasted too only when `with_air` is set (else what is there
/// stays).
pub fn paste(ed: &mut Editor, clip: &Clipboard, origin: [i32; 3], place: Placement, with_air: bool) -> Done {
    let mut done = Done::default();
    let mut chunks: std::collections::BTreeMap<(i32, i32), Option<ChunkEdit>> = Default::default();
    let states: Vec<BlockState> = clip.palette.iter().map(|s| place.state(s)).collect();
    let same_kind = clip.edition == edition_of(ed);
    for y in 0..clip.size[1] {
        for z in 0..clip.size[2] {
            for x in 0..clip.size[0] {
                let slot = clip.blocks[clip.index([x, y, z])] as usize;
                if !with_air && clip.palette[slot].is_air() {
                    continue;
                }
                let t = place.pos(clip.size, [x, y, z]);
                let (wx, wy, wz) = (origin[0] + t[0], origin[1] + t[1], origin[2] + t[2]);
                let key = (wx.div_euclid(16), wz.div_euclid(16));
                let c = chunks.entry(key).or_insert_with(|| ChunkEdit::load(ed, key.0, key.1).filter(|c| !c.is_old_bedrock()));
                let Some(c) = c else { continue };
                if c.set(wx.rem_euclid(16) as usize, wy, wz.rem_euclid(16) as usize, &states[slot]) {
                    done.blocks += 1;
                }
            }
        }
    }
    // Block entities (chest contents, sign text...), moved to where their blocks went.
    if same_kind {
        for (p, be) in &clip.block_entities {
            let t = place.pos(clip.size, *p);
            let (wx, wy, wz) = (origin[0] + t[0], origin[1] + t[1], origin[2] + t[2]);
            let Some(Some(c)) = chunks.get_mut(&(wx.div_euclid(16), wz.div_euclid(16))) else { continue };
            let mut be = be.clone();
            be.insert("x".into(), Tag::Int(wx));
            be.insert("y".into(), Tag::Int(wy));
            be.insert("z".into(), Tag::Int(wz));
            c.add_block_entity(be);
        }
    }
    for (_, c) in chunks {
        if let Some(c) = c {
            done.unmapped += c.unmapped;
            done.chunks += usize::from(c.store(ed));
        }
    }
    done
}

// ---------------------------------------------------------------------------------------------
// Entities copied and pasted
// ---------------------------------------------------------------------------------------------

/// A number no entity has: 64 random bits (from the standard library's randomly seeded hasher).
fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(t.as_nanos());
    }
    h.finish()
}

fn entity_pos(c: &Compound) -> Option<[f64; 3]> {
    let p = c.get("Pos").and_then(Tag::as_list)?;
    let v = |i: usize| p.get(i).and_then(Tag::as_f64);
    Some([v(0)?, v(1)?, v(2)?])
}

/// The entities standing in the box, as the edition keeps them.
pub fn entities_in(ed: &Editor, b: Bounds) -> Vec<Compound> {
    let mut out = Vec::new();
    for (cx, cz, ..) in chunk_parts(b) {
        let list: Vec<Compound> = match &ed.base {
            Base::Java { .. } => {
                let Some(jc) = ed.java(cx, cz) else { continue };
                let own = jc.root.as_ref().and_then(|r| r.tag.get("Level")).and_then(|l| l.get("Entities")).and_then(Tag::as_list).map(|l| l.iter().filter_map(Tag::as_compound).cloned().collect::<Vec<_>>()).unwrap_or_default();
                if own.is_empty() {
                    ed.java_entities(cx, cz, &jc).and_then(|r| r.tag.get("Entities").and_then(Tag::as_list).map(|l| l.iter().filter_map(Tag::as_compound).cloned().collect())).unwrap_or_default()
                } else {
                    own
                }
            }
            Base::Bedrock { dim, .. } => {
                let mut v = Vec::new();
                if let Some(list) = ed.bedrock(&Key::ActorDigest { x: cx, z: cz, dimension: *dim }.encode()) {
                    for uid in list.chunks_exact(8) {
                        if let Some(raw) = ed.bedrock(&Key::Actor(uid.to_vec()).encode()) {
                            v.extend(bedrock_compounds(&raw));
                        }
                    }
                }
                if let Some(raw) = ed.bedrock(&ed.bedrock_key(cx, cz, ChunkTag::Entity, None)) {
                    v.extend(bedrock_compounds(&raw));
                }
                v
            }
            Base::Console { .. } => ed.console(cx, cz).and_then(|r| r.tag.get("Level").and_then(|l| l.get("Entities")).and_then(Tag::as_list).map(|l| l.iter().filter_map(Tag::as_compound).cloned().collect())).unwrap_or_default(),
        };
        out.extend(list.into_iter().filter(|c| entity_in(c, b)));
    }
    out
}

/// An entity moved to `pos`, turned and mirrored as `place` says, with a new identity (an
/// entity's id must be its own).
fn placed_entity(e: &Compound, pos: [f64; 3], place: Placement, origin: [i32; 3], clip_size: [i32; 3], rel_block: impl Fn([i32; 3]) -> [i32; 3]) -> Compound {
    let mut e = e.clone();
    // In the number type the edition keeps it in (Bedrock: floats; Java, console: doubles).
    let floats = matches!(e.get("Pos"), Some(Tag::List(l)) if l.element_type() == TagType::Float);
    let pos_tag = if floats {
        List::from_tags(TagType::Float, pos.iter().map(|v| Tag::Float(*v as f32)).collect())
    } else {
        List::from_tags(TagType::Double, pos.iter().map(|v| Tag::Double(*v)).collect())
    };
    e.insert("Pos".into(), Tag::List(pos_tag.expect("numbers")));
    // Which way it looks: a quarter turn clockwise adds 90 to the yaw; mirroring negates it.
    if let Some(Tag::List(r)) = e.get("Rotation") {
        let mut v: Vec<f32> = r.iter().filter_map(Tag::as_f64).map(|v| v as f32).collect();
        if let Some(yaw) = v.first_mut() {
            if place.mirror {
                *yaw = -*yaw;
            }
            *yaw = (*yaw + 90.0 * place.turns as f32).rem_euclid(360.0);
        }
        e.insert("Rotation".into(), Tag::List(List::from_tags(TagType::Float, v.into_iter().map(Tag::Float).collect()).expect("floats")));
    }
    // Frames and paintings hang on a block: that block moves with them, and the side they face.
    if let (Some(x), Some(y), Some(z)) = (e.get("TileX").and_then(Tag::as_i64), e.get("TileY").and_then(Tag::as_i64), e.get("TileZ").and_then(Tag::as_i64)) {
        let t = rel_block([x as i32, y as i32, z as i32]);
        let p = place.pos(clip_size, t);
        e.insert("TileX".into(), Tag::Int(origin[0] + p[0]));
        e.insert("TileY".into(), Tag::Int(origin[1] + p[1]));
        e.insert("TileZ".into(), Tag::Int(origin[2] + p[2]));
    }
    let side3 = ["down", "up", "north", "south", "west", "east"];
    if let Some(f) = e.get("Facing").and_then(Tag::as_i64).filter(|f| (2..6).contains(f)) {
        if let Some(s) = side3.get(f as usize) {
            let turned = place.state(&BlockState::new("x").with("facing", *s));
            if let Some(n) = side3.iter().position(|d| Some(*d) == turned.properties.get("facing").map(String::as_str)) {
                e.insert("Facing".into(), Tag::Byte(n as i8));
            }
        }
    }
    // A new identity, in the edition's own form.
    if e.contains_key("UUID") && matches!(e.get("UUID"), Some(Tag::IntArray(_))) {
        let (a, b) = (random_u64(), random_u64());
        e.insert("UUID".into(), Tag::IntArray(vec![(a >> 32) as i32, a as i32, (b >> 32) as i32, b as i32]));
    }
    if e.contains_key("UUIDMost") {
        e.insert("UUIDMost".into(), Tag::Long(random_u64() as i64));
        e.insert("UUIDLeast".into(), Tag::Long(random_u64() as i64));
    }
    if e.contains_key("UniqueID") {
        e.insert("UniqueID".into(), Tag::Long((random_u64() >> 2) as i64));
    }
    e
}

/// Adds entities (as the edition keeps them) to the chunks their positions are in. Gives how
/// many went in.
pub fn add_entities(ed: &mut Editor, list: Vec<Compound>) -> usize {
    let mut by_chunk: std::collections::BTreeMap<(i32, i32), Vec<Compound>> = Default::default();
    for e in list {
        let Some(p) = entity_pos(&e) else { continue };
        by_chunk.entry(((p[0].floor() as i32).div_euclid(16), (p[2].floor() as i32).div_euclid(16))).or_default().push(e);
    }
    let mut added = 0;
    for ((cx, cz), items) in by_chunk {
        let id = ed.id(cx, cz);
        let n = items.len();
        match &ed.base {
            Base::Java { .. } => {
                let Some(mut jc) = ed.java(cx, cz) else { continue };
                // Before 1.17, in the chunk itself; after, in the entities' record.
                let version = jc.root.as_ref().and_then(|r| r.tag.get("DataVersion")).and_then(Tag::as_i64).unwrap_or(0);
                if version < 2724 {
                    let Some(level) = jc.root.as_mut().and_then(|r| r.tag.as_compound_mut()).and_then(|c| c.get_mut("Level")).and_then(Tag::as_compound_mut) else { continue };
                    push_list(level, "Entities", items);
                } else {
                    let mut record = ed.java_entities(cx, cz, &jc).unwrap_or_else(|| {
                        let mut c = Compound::new();
                        c.insert("DataVersion".into(), Tag::Int(version as i32));
                        c.insert("Position".into(), Tag::IntArray(vec![cx, cz]));
                        NamedTag { name: String::new(), tag: Tag::Compound(c) }
                    });
                    if let Some(c) = record.tag.as_compound_mut() {
                        push_list(c, "Entities", items);
                    }
                    jc.entities = Some(Some(record));
                }
                ed.layer.set_java(id, jc);
            }
            Base::Bedrock { dim, .. } => {
                let dim = *dim;
                let old_key = ed.bedrock_key(cx, cz, ChunkTag::Entity, None);
                if let Some(raw) = ed.bedrock(&old_key) {
                    // A chunk from before 1.18.30 keeps them all in one record.
                    let mut all = bedrock_compounds(&raw);
                    all.extend(items);
                    ed.layer.set_bedrock(id, old_key, bedrock_record(&all));
                } else {
                    let digp = Key::ActorDigest { x: cx, z: cz, dimension: dim }.encode();
                    let mut list = ed.bedrock(&digp).unwrap_or_default();
                    for e in items {
                        let uid = e.get("UniqueID").and_then(Tag::as_i64).unwrap_or(random_u64() as i64).to_le_bytes().to_vec();
                        ed.layer.set_bedrock(id.clone(), Key::Actor(uid.clone()).encode(), bedrock_record(&[e]));
                        list.extend_from_slice(&uid);
                    }
                    ed.layer.set_bedrock(id, digp, Some(list));
                }
            }
            Base::Console { .. } => {
                let Some(mut root) = ed.console(cx, cz) else { continue };
                let Some(level) = root.tag.as_compound_mut().and_then(|c| c.get_mut("Level")).and_then(Tag::as_compound_mut) else { continue };
                push_list(level, "Entities", items);
                ed.layer.set_console(id, Some(root));
            }
        }
        added += n;
    }
    added
}

fn push_list(parent: &mut Compound, key: &str, items: Vec<Compound>) {
    let mut all: Vec<Tag> = parent.get(key).and_then(Tag::as_list).map(|l| l.iter().cloned().collect()).unwrap_or_default();
    all.extend(items.into_iter().map(Tag::Compound));
    parent.insert(key.into(), Tag::List(List::from_tags(TagType::Compound, all).unwrap_or_else(|_| List::new(TagType::Compound))));
}

/// The clipboard's entities placed with it (at `origin`, turned and mirrored), each with a new
/// identity; only into the kind of world they came from.
pub fn paste_entities(ed: &mut Editor, clip: &Clipboard, origin: [i32; 3], place: Placement) -> usize {
    if clip.edition != edition_of(ed) || clip.entities.is_empty() {
        return 0;
    }
    let (sx, sz) = (clip.size[0] as f64, clip.size[2] as f64);
    let items: Vec<Compound> = clip
        .entities
        .iter()
        .map(|(p, e)| {
            let (mut x, y, mut z) = (p[0], p[1], p[2]);
            let (mut w, mut d) = (sx, sz);
            if place.mirror {
                x = w - x;
            }
            for _ in 0..place.turns % 4 {
                (x, z) = (d - z, x);
                (w, d) = (d, w);
            }
            let pos = [origin[0] as f64 + x, origin[1] as f64 + y, origin[2] as f64 + z];
            placed_entity(e, pos, place, origin, clip.size, |t| [t[0] - clip.origin[0], t[1] - clip.origin[1], t[2] - clip.origin[2]])
        })
        .collect();
    add_entities(ed, items)
}

#[cfg(test)]
mod entity_tests {
    use super::*;

    /// The brush's preview is the shape it paints: a ball of 3 is the 179 blocks painted (as
    /// the world tests count), a cube of 1 is 27.
    #[test]
    fn the_brush_preview_is_its_shape() {
        let s = BlockState::new("minecraft:glass");
        assert_eq!(brush_shape(3, true, &s).solid(), 179);
        assert_eq!(brush_shape(1, false, &s).solid(), 27);
        assert_eq!(brush_shape(0, true, &s).solid(), 1);
    }

    /// A pasted entity is where it was put, looks the way it was turned, and has a new id in its
    /// edition's form.
    #[test]
    fn a_placed_entity_moves_turns_and_gets_a_new_id() {
        let mut e = Compound::new();
        e.insert("id".into(), Tag::String("minecraft:pig".into()));
        e.insert("UUID".into(), Tag::IntArray(vec![1, 2, 3, 4]));
        e.insert("Rotation".into(), Tag::List(List::from_tags(TagType::Float, vec![Tag::Float(180.0), Tag::Float(0.0)]).unwrap()));
        e.insert("Pos".into(), Tag::List(List::from_tags(TagType::Double, vec![Tag::Double(0.0); 3]).unwrap()));
        let one = Placement { turns: 1, mirror: false };
        let p = placed_entity(&e, [10.5, 64.0, -3.5], one, [0, 0, 0], [1, 1, 1], |t| t);
        assert_eq!(entity_pos(&p), Some([10.5, 64.0, -3.5]));
        assert_ne!(p.get("UUID"), e.get("UUID"));
        // Facing north (180) turned a quarter clockwise: east (270).
        let yaw = p.get("Rotation").and_then(Tag::as_list).and_then(|l| l.get(0)).and_then(Tag::as_f64);
        assert_eq!(yaw, Some(270.0));
    }
}

// ---------------------------------------------------------------------------------------------
// Brush
// ---------------------------------------------------------------------------------------------

/// The brush's shape as a clipboard (for showing it see-through): a ball (or cube) of a block,
/// `2 * radius + 1` across, its middle block in the middle.
pub fn brush_shape(radius: i32, sphere: bool, state: &BlockState) -> Clipboard {
    let r = radius.max(0);
    let n = 2 * r + 1;
    let reach = (r as f64 + 0.5).powi(2);
    let mut clip = Clipboard { size: [n, n, n], palette: vec![BlockState::air(), state.clone()], blocks: vec![0; (n * n * n) as usize], ..Default::default() };
    for y in 0..n {
        for z in 0..n {
            for x in 0..n {
                let d = ((x - r).pow(2) + (y - r).pow(2) + (z - r).pow(2)) as f64;
                if !sphere || d <= reach {
                    let i = clip.index([x, y, z]);
                    clip.blocks[i] = 1;
                }
            }
        }
    }
    clip
}

/// Paints a ball (or cube) of blocks `radius` from `center` with a block (air to erase).
pub fn brush(ed: &mut Editor, center: [i32; 3], radius: i32, sphere: bool, state: &BlockState) -> Done {
    let r = radius.max(0);
    let b = ([center[0] - r, center[1] - r, center[2] - r], [center[0] + r, center[1] + r, center[2] + r]);
    let mut done = Done::default();
    let reach = (r as f64 + 0.5).powi(2);
    for (cx, cz, xs, zs) in chunk_parts(b) {
        let Some(mut c) = ChunkEdit::load(ed, cx, cz) else { continue };
        if c.is_old_bedrock() {
            continue;
        }
        for y in b.0[1].max(c.ylo)..=b.1[1].min(c.yhi) {
            for z in zs[0]..=zs[1] {
                for x in xs[0]..=xs[1] {
                    let (wx, wz) = (cx * 16 + x as i32, cz * 16 + z as i32);
                    let d = ((wx - center[0]).pow(2) + (y - center[1]).pow(2) + (wz - center[2]).pow(2)) as f64;
                    if sphere && d > reach {
                        continue;
                    }
                    if c.get(x, y, z) != *state && c.set(x, y, z, state) {
                        done.blocks += 1;
                    }
                }
            }
        }
        done.unmapped += c.unmapped;
        done.chunks += usize::from(c.store(ed));
    }
    done
}

// ---------------------------------------------------------------------------------------------
// Delete Chunks
// ---------------------------------------------------------------------------------------------

/// Deletes whole chunks (the game makes them anew when someone comes near). Gives how many.
pub fn delete_chunks(ed: &mut Editor, chunks: &[(i32, i32)]) -> usize {
    let mut gone = 0;
    for &(cx, cz) in chunks {
        let id = ed.id(cx, cz);
        match &ed.base {
            Base::Java { .. } => {
                if ed.java(cx, cz).is_some() {
                    ed.layer.set_java(id, JavaChunk { root: None, entities: Some(None) });
                    gone += 1;
                }
            }
            Base::Bedrock { .. } => {
                let keys = ed.bedrock_chunk_keys(cx, cz);
                if !keys.is_empty() {
                    for k in keys {
                        ed.layer.set_bedrock(id.clone(), k, None);
                    }
                    gone += 1;
                }
            }
            Base::Console { .. } => {
                if ed.console(cx, cz).is_some() {
                    ed.layer.set_console(id, None);
                    gone += 1;
                }
            }
        }
    }
    gone
}
