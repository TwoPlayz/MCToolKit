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

/// Sets which way an entity looks (its yaw in degrees, 0 to 360), keeping its pitch.
pub fn set_yaw(e: &mut Compound, yaw: f32) {
    let yaw = yaw.rem_euclid(360.0);
    let pitch = e.get("Rotation").and_then(Tag::as_list).and_then(|l| l.get(1)).and_then(Tag::as_f64).unwrap_or(0.0) as f32;
    let list = List::from_tags(TagType::Float, vec![Tag::Float(yaw), Tag::Float(pitch)]).expect("floats");
    e.insert("Rotation".into(), Tag::List(list));
}

/// Moves an entity to `pos`, in the number type its edition keeps (Bedrock: floats).
pub fn set_entity_pos(e: &mut Compound, pos: [f64; 3]) {
    let floats = matches!(e.get("Pos"), Some(Tag::List(l)) if l.element_type() == TagType::Float);
    let list = if floats {
        List::from_tags(TagType::Float, pos.iter().map(|v| Tag::Float(*v as f32)).collect())
    } else {
        List::from_tags(TagType::Double, pos.iter().map(|v| Tag::Double(*v)).collect())
    };
    e.insert("Pos".into(), Tag::List(list.expect("numbers")));
}

/// Puts a new mob of a kind (a Java id) on an entity as its passenger (Java worlds).
pub fn add_passenger(e: &mut Compound, java_id: &str, dv: Option<i64>) {
    let id = if java_id.contains(':') { java_id.to_string() } else { format!("minecraft:{java_id}") };
    let mut rider = Compound::new();
    rider.insert("id".into(), Tag::String(id));
    mob_defaults(&mut rider, dv);
    push_passenger(e, rider);
}

/// Gives a new mob the tags the game saves on every one (health, air, fire, hurt and death timers, what it
/// wears and holds, its age or its villager data...), so it is a whole mob and not a bare id.
pub fn mob_defaults(e: &mut Compound, dv: Option<i64>) {
    let id = e.get("id").and_then(Tag::as_str).unwrap_or("").trim_start_matches("minecraft:").to_string();
    // Not every entity lives: the ones that stand about, and the things that are thrown, ride or hang.
    let lifeless = id.ends_with("item_frame") || id.ends_with("minecart") || id.ends_with("boat") || id.ends_with("raft") || id.ends_with("arrow") || matches!(id.as_str(), "item" | "painting" | "armor_stand" | "tnt" | "falling_block" | "experience_orb" | "end_crystal" | "lightning_bolt" | "area_effect_cloud" | "snowball" | "egg" | "ender_pearl");
    let set = |e: &mut Compound, k: &str, v: Tag| {
        if !e.contains_key(k) {
            e.insert(k.into(), v);
        }
    };
    set(e, "Air", Tag::Short(300));
    set(e, "FallDistance", Tag::Float(0.0));
    set(e, "Fire", Tag::Short(-1));
    set(e, "Invulnerable", Tag::Byte(0));
    set(e, "OnGround", Tag::Byte(1));
    set(e, "PortalCooldown", Tag::Int(0));
    if lifeless {
        return;
    }
    let max = crate::mob_edit::max_health(e, &id).unwrap_or(20.0);
    set(e, "Health", Tag::Float(max as f32));
    set(e, "AbsorptionAmount", Tag::Float(0.0));
    set(e, "DeathTime", Tag::Short(0));
    set(e, "HurtTime", Tag::Short(0));
    set(e, "HurtByTimestamp", Tag::Int(0));
    set(e, "FallFlying", Tag::Byte(0));
    set(e, "CanPickUpLoot", Tag::Byte(0));
    set(e, "LeftHanded", Tag::Byte(0));
    set(e, "PersistenceRequired", Tag::Byte(0));
    // What it wears and holds: lists of empty stacks before 1.21.5 (after it, none is none).
    if dv.is_some_and(|v| v > 0 && v < 4325) {
        let empty = |n: usize| Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(Compound::new()); n]).expect("compounds"));
        let chances = |n: usize| Tag::List(List::from_tags(TagType::Float, vec![Tag::Float(0.085); n]).expect("floats"));
        set(e, "ArmorItems", empty(4));
        set(e, "HandItems", empty(2));
        set(e, "ArmorDropChances", chances(4));
        set(e, "HandDropChances", chances(2));
    }
    // Animals and villagers have an age and a love timer.
    if matches!(id.as_str(), "cow" | "mooshroom" | "pig" | "sheep" | "chicken" | "horse" | "donkey" | "mule" | "llama" | "trader_llama" | "cat" | "wolf" | "fox" | "rabbit" | "panda" | "ocelot" | "goat" | "camel" | "sniffer" | "turtle" | "bee" | "strider" | "hoglin" | "axolotl" | "frog" | "villager") {
        set(e, "Age", Tag::Int(0));
        set(e, "ForcedAge", Tag::Int(0));
        set(e, "InLove", Tag::Int(0));
    }
    match id.as_str() {
        "villager" | "zombie_villager" => {
            let mut data = Compound::new();
            data.insert("level".into(), Tag::Int(1));
            data.insert("profession".into(), Tag::String("minecraft:none".into()));
            data.insert("type".into(), Tag::String("minecraft:plains".into()));
            set(e, "VillagerData", Tag::Compound(data));
            set(e, "Xp", Tag::Int(0));
            if id == "villager" {
                set(e, "FoodLevel", Tag::Byte(0));
                set(e, "Willing", Tag::Byte(0));
            }
        }
        "zombie" | "husk" | "drowned" | "zombified_piglin" => {
            set(e, "IsBaby", Tag::Byte(0));
            set(e, "CanBreakDoors", Tag::Byte(0));
            set(e, "InWaterTime", Tag::Int(-1));
            set(e, "DrownedConversionTime", Tag::Int(-1));
        }
        "creeper" => {
            set(e, "Fuse", Tag::Short(30));
            set(e, "ExplosionRadius", Tag::Byte(3));
            set(e, "ignited", Tag::Byte(0));
        }
        "sheep" => {
            set(e, "Color", Tag::Byte(0));
            set(e, "Sheared", Tag::Byte(0));
        }
        "slime" | "magma_cube" => {
            set(e, "Size", Tag::Int(1));
            set(e, "wasOnGround", Tag::Byte(0));
        }
        _ => {}
    }
}

/// Puts a mob (as it is saved) on an entity as its passenger.
fn push_passenger(e: &mut Compound, rider: Compound) {
    let mut all: Vec<Tag> = e.get("Passengers").and_then(Tag::as_list).map(|l| l.iter().cloned().collect()).unwrap_or_default();
    all.push(Tag::Compound(rider));
    e.insert("Passengers".into(), Tag::List(List::from_tags(TagType::Compound, all).unwrap_or_else(|_| List::new(TagType::Compound))));
}

/// Puts the mob standing at `rider` on the one standing at `mount` as its passenger (Java
/// worlds): it leaves the world's list and is saved with its mount. Gives whether it was done.
pub fn mount_existing(ed: &mut Editor, mount: [f64; 3], path: &[usize], rider: [f64; 3]) -> bool {
    if !matches!(ed.base, Base::Java { .. }) {
        return false;
    }
    let cell = |p: [f64; 3]| [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
    let is_at = |p: [f64; 3]| move |c: &Compound| entity_pos(c).is_some_and(|q| (0..3).all(|i| (q[i] - p[i]).abs() < 0.01));
    let (a, b) = (cell(mount), cell(rider));
    let Some(r) = entities_in(ed, (b, b)).into_iter().find(|c| is_at(rider)(c)) else { return false };
    // The mount must be there before the rider leaves the world.
    if !entities_in(ed, (a, a)).iter().any(|c| is_at(mount)(c)) || (path.is_empty() && mount == rider) || !delete_entity_at(ed, rider) {
        return false;
    }
    // (A mount that is itself a rider is reached through the seats from the mob standing in the world.)
    edit_entity_at(ed, mount, &|m| {
        if let Some(t) = rider_at(m, path) {
            push_passenger(t, r.clone());
        }
    })
}

/// The mob at the end of a path of seats from an entity: the rider in seat `path[0]` of it, the
/// rider in seat `path[1]` of that one, and so on (the entity itself for an empty path). Riders
/// can be stacked as deep as the world holds them.
pub fn rider_at<'a>(e: &'a mut Compound, path: &[usize]) -> Option<&'a mut Compound> {
    let Some((first, rest)) = path.split_first() else { return Some(e) };
    let Some(Tag::List(l)) = e.get_mut("Passengers") else { return None };
    rider_at(l.get_mut(*first)?.as_compound_mut()?, rest)
}

/// Takes a rider off the mount standing at `mount` (the one in seat `seat`, or all of them): they
/// stand in the world where the mount is, or, with `delete`, are removed. Gives how many were
/// taken off (Java worlds).
pub fn unmount(ed: &mut Editor, mount: [f64; 3], path: &[usize], seat: Option<usize>, delete: bool) -> usize {
    if !matches!(ed.base, Base::Java { .. }) {
        return 0;
    }
    let off = std::cell::RefCell::new(Vec::<Compound>::new());
    let done = edit_entity_at(ed, mount, &|root| {
        // `path` leads from the entity standing in the world, through the seats, to the mount.
        let Some(m) = rider_at(root, path) else { return };
        let all: Vec<Tag> = m.get("Passengers").and_then(Tag::as_list).map(|l| l.iter().cloned().collect()).unwrap_or_default();
        let (mut stay, mut go) = (Vec::new(), Vec::new());
        for (i, r) in all.into_iter().enumerate() {
            if seat.is_none_or(|s| s == i) { go.push(r) } else { stay.push(r) }
        }
        if stay.is_empty() {
            m.shift_remove("Passengers");
        } else {
            m.insert("Passengers".into(), Tag::List(List::from_tags(TagType::Compound, stay).unwrap_or_else(|_| List::new(TagType::Compound))));
        }
        *off.borrow_mut() = go.into_iter().filter_map(|t| t.as_compound().cloned()).collect();
    });
    if !done {
        return 0;
    }
    let off = off.into_inner();
    let n = off.len();
    if !delete && n > 0 {
        let freed: Vec<Compound> = off
            .into_iter()
            .map(|mut r| {
                // Beside the mount, not inside it: where it can be picked up and moved.
                set_entity_pos(&mut r, [mount[0] + 1.5, mount[1], mount[2]]);
                r
            })
            .collect();
        add_entities(ed, freed);
    }
    n
}

/// Changes the lingering cloud standing at `at` to `cloud`, keeping its identity and age (Java
/// worlds). Gives whether it was found.
pub fn update_cloud(ed: &mut Editor, at: [f64; 3], cloud: &crate::cloud::Cloud) -> bool {
    if !matches!(ed.base, Base::Java { .. }) {
        return false;
    }
    let (cx, cz) = ((at[0].floor() as i32).div_euclid(16), (at[2].floor() as i32).div_euclid(16));
    let version = ed.java(cx, cz).and_then(|jc| jc.root.as_ref().and_then(|r| r.tag.get("DataVersion")).and_then(Tag::as_i64)).unwrap_or(0);
    let cloud = cloud.clone();
    edit_entity_at(ed, at, &move |e| {
        let mut fresh = cloud.entity(at, version);
        for keep in ["UUID", "Age"] {
            if let Some(v) = e.get(keep) {
                fresh.insert(keep.into(), v.clone());
            }
        }
        *e = fresh;
    })
}

/// Puts a lingering potion cloud at `pos`, written for the world's version (Java worlds only).
/// Gives whether it went in.
pub fn place_cloud(ed: &mut Editor, cloud: &crate::cloud::Cloud, pos: [f64; 3]) -> bool {
    if !matches!(ed.base, Base::Java { .. }) {
        return false;
    }
    let (cx, cz) = ((pos[0].floor() as i32).div_euclid(16), (pos[2].floor() as i32).div_euclid(16));
    let version = ed.java(cx, cz).and_then(|jc| jc.root.as_ref().and_then(|r| r.tag.get("DataVersion")).and_then(Tag::as_i64)).unwrap_or(0);
    add_entities(ed, vec![cloud.entity(pos, version)]) > 0
}

/// The data version of the Java chunk at a place (0 if there is none), to write things for.
fn chunk_version(ed: &mut Editor, pos: [f64; 3]) -> i64 {
    let (cx, cz) = ((pos[0].floor() as i32).div_euclid(16), (pos[2].floor() as i32).div_euclid(16));
    ed.java(cx, cz).and_then(|jc| jc.root.as_ref().and_then(|r| r.tag.get("DataVersion")).and_then(Tag::as_i64)).unwrap_or(0)
}

/// A new entity of a Java id standing at `pos`, looking south, with an identity of its own.
fn new_entity(java_id: &str, pos: [f64; 3]) -> Compound {
    let id = if java_id.contains(':') { java_id.to_string() } else { format!("minecraft:{java_id}") };
    let mut e = Compound::new();
    e.insert("id".into(), Tag::String(id));
    e.insert("Pos".into(), Tag::List(List::from_tags(TagType::Double, pos.iter().map(|v| Tag::Double(*v)).collect()).expect("numbers")));
    e.insert("Motion".into(), Tag::List(List::from_tags(TagType::Double, vec![Tag::Double(0.0); 3]).expect("numbers")));
    e.insert("Rotation".into(), Tag::List(List::from_tags(TagType::Float, vec![Tag::Float(0.0), Tag::Float(0.0)]).expect("floats")));
    let (a, b) = (random_u64(), random_u64());
    e.insert("UUID".into(), Tag::IntArray(vec![(a >> 32) as i32, a as i32, (b >> 32) as i32, b as i32]));
    e
}

/// Puts a new mob of a kind (a Java id) at `pos` (Java worlds). Gives whether it went in.
pub fn place_mob(ed: &mut Editor, java_id: &str, pos: [f64; 3]) -> bool {
    if !matches!(ed.base, Base::Java { .. }) {
        return false;
    }
    let dv = chunk_version(ed, pos);
    let mut e = new_entity(java_id, pos);
    mob_defaults(&mut e, (dv > 0).then_some(dv));
    add_entities(ed, vec![e]) > 0
}

/// Puts a thing that stands on its own (an armor stand, an item frame, a minecart, a boat...) in
/// the block `cell`: on the floor of it, or, for an item frame, against the wall opposite
/// `facing` (the Facing byte: 2 north, 3 south, 4 west, 5 east). Java worlds. Gives whether it went in.
pub fn place_entity(ed: &mut Editor, java_id: &str, cell: [i32; 3], facing: Option<u8>) -> bool {
    if !matches!(ed.base, Base::Java { .. }) {
        return false;
    }
    let centre = |i: usize| cell[i] as f64 + 0.5;
    let mut e;
    if java_id.ends_with("item_frame") {
        let face = facing.unwrap_or(2);
        let (dx, dz) = match face {
            2 => (0.0, -1.0),
            3 => (0.0, 1.0),
            4 => (-1.0, 0.0),
            _ => (1.0, 0.0),
        };
        e = new_entity(java_id, [centre(0) - dx * 0.46875, cell[1] as f64 + 0.5, centre(2) - dz * 0.46875]);
        e.insert("Facing".into(), Tag::Byte(face as i8));
        e.insert("TileX".into(), Tag::Int(cell[0]));
        e.insert("TileY".into(), Tag::Int(cell[1]));
        e.insert("TileZ".into(), Tag::Int(cell[2]));
        e.insert("ItemRotation".into(), Tag::Byte(0));
        e.insert("ItemDropChance".into(), Tag::Float(1.0));
    } else {
        e = new_entity(java_id, [centre(0), cell[1] as f64, centre(2)]);
    }
    add_entities(ed, vec![e]) > 0
}

/// The poses an armor stand can be put in: a name and the angles (degrees about x, y, z) of the
/// parts it moves (the rest stay as they are in the game: straight).
pub const STAND_POSES: &[(&str, &[(&str, [f32; 3])])] = &[
    ("Default", &[]),
    ("Walking", &[("LeftArm", [-20.0, 0.0, 0.0]), ("RightArm", [20.0, 0.0, 0.0]), ("LeftLeg", [20.0, 0.0, 0.0]), ("RightLeg", [-20.0, 0.0, 0.0])]),
    ("Pointing", &[("RightArm", [-90.0, 0.0, 0.0]), ("Head", [-8.0, 0.0, 0.0])]),
    ("Salute", &[("RightArm", [-110.0, -30.0, 0.0]), ("Head", [0.0, 0.0, 0.0])]),
    ("Hooray", &[("LeftArm", [-170.0, 0.0, -10.0]), ("RightArm", [-170.0, 0.0, 10.0]), ("Head", [-10.0, 0.0, 0.0])]),
    ("Sitting", &[("LeftLeg", [-85.0, -15.0, 0.0]), ("RightLeg", [-85.0, 15.0, 0.0]), ("LeftArm", [-20.0, 0.0, 0.0]), ("RightArm", [-20.0, 0.0, 0.0])]),
    ("Looking up", &[("Head", [-35.0, 0.0, 0.0])]),
    ("Looking down", &[("Head", [35.0, 0.0, 0.0])]),
    ("Looking left", &[("Head", [0.0, -50.0, 0.0])]),
    ("Looking right", &[("Head", [0.0, 50.0, 0.0])]),
    ("Head tilted", &[("Head", [0.0, 0.0, 15.0])]),
    ("Arms out", &[("LeftArm", [0.0, 0.0, -90.0]), ("RightArm", [0.0, 0.0, 90.0])]),
    ("Arms forward", &[("LeftArm", [-90.0, 0.0, 0.0]), ("RightArm", [-90.0, 0.0, 0.0])]),
    ("Waving", &[("RightArm", [-160.0, 0.0, 25.0]), ("LeftArm", [10.0, 0.0, -10.0]), ("Head", [0.0, 0.0, 8.0])]),
    ("Running", &[("LeftArm", [-60.0, 0.0, 0.0]), ("RightArm", [60.0, 0.0, 0.0]), ("LeftLeg", [60.0, 0.0, 0.0]), ("RightLeg", [-60.0, 0.0, 0.0]), ("Body", [12.0, 0.0, 0.0]), ("Head", [-8.0, 0.0, 0.0])]),
    ("Legs apart", &[("LeftLeg", [0.0, 0.0, -20.0]), ("RightLeg", [0.0, 0.0, 20.0])]),
    ("Kneeling", &[("LeftLeg", [-90.0, 0.0, 0.0]), ("RightLeg", [-20.0, 0.0, 0.0]), ("Head", [10.0, 0.0, 0.0])]),
    ("Bowing", &[("Body", [30.0, 0.0, 0.0]), ("Head", [25.0, 0.0, 0.0]), ("LeftArm", [20.0, 0.0, 0.0]), ("RightArm", [20.0, 0.0, 0.0])]),
    ("Praying", &[("LeftArm", [-60.0, 30.0, 0.0]), ("RightArm", [-60.0, -30.0, 0.0]), ("Head", [20.0, 0.0, 0.0])]),
];

/// The parts of an armor stand a pose sets.
const STAND_PARTS: [&str; 6] = ["Head", "Body", "LeftArm", "RightArm", "LeftLeg", "RightLeg"];

/// The name of the pose an armor stand is in (`None` when its pose is none of the named ones),
/// so the menu can tick it.
pub fn stand_pose_name(e: &Compound) -> Option<&'static str> {
    let pose = e.get("Pose").and_then(Tag::as_compound);
    let angles = |part: &str| -> [f32; 3] {
        let l = pose.and_then(|p| p.get(part)).and_then(Tag::as_list);
        let n = |i: usize| l.and_then(|l| l.get(i)).and_then(Tag::as_f64).unwrap_or(0.0) as f32;
        [n(0), n(1), n(2)]
    };
    STAND_POSES.iter().find(|(_, parts)| {
        STAND_PARTS.iter().all(|part| {
            let want = parts.iter().find(|(p, _)| p == part).map_or([0.0; 3], |(_, a)| *a);
            let have = angles(part);
            (0..3).all(|k| (want[k] - have[k]).abs() < 0.5)
        })
    }).map(|(n, _)| *n)
}

/// Changes the block entity of the block at `at` (a beacon's powers, a chest's lock...), as the
/// edition keeps it. Gives whether the block has one.
pub fn change_block_entity(ed: &mut Editor, at: [i32; 3], change: &crate::mob_edit::MobEdit) -> bool {
    let dv = chunk_version(ed, [f64::from(at[0]), f64::from(at[1]), f64::from(at[2])]);
    let dv = (dv > 0).then_some(dv);
    let Some(mut chunk) = ChunkEdit::load(ed, at[0].div_euclid(16), at[2].div_euclid(16)) else { return false };
    // A block put down in the World Explorer has no block entity yet: a Java world's gets one made
    // (an empty one) so it can be given its powers or its lock.
    let fresh = || -> Option<Compound> {
        if !matches!(ed.base, Base::Java { .. }) {
            return None;
        }
        let name = chunk.get(at[0].rem_euclid(16) as usize, at[1], at[2].rem_euclid(16) as usize).name;
        let bare = name.trim_start_matches("minecraft:");
        let id = if bare.ends_with("shulker_box") { "shulker_box" } else { bare };
        let mut be = Compound::new();
        be.insert("id".into(), Tag::String(format!("minecraft:{id}")));
        be.insert("x".into(), Tag::Int(at[0]));
        be.insert("y".into(), Tag::Int(at[1]));
        be.insert("z".into(), Tag::Int(at[2]));
        (crate::mob_edit::is_beacon(&be) || crate::mob_edit::lockable(&be)).then_some(be)
    };
    let Some(mut be) = chunk.block_entities_in(ed, at, at).into_iter().next().or_else(fresh) else { return false };
    change.apply(&mut be, dv);
    chunk.replace_block_entity(be);
    chunk.store(ed)
}

/// Changes one thing about an entity (see [`EntityChange`]), the one standing at `at`, or, with
/// `seat`, the rider in that seat of the mount standing there. Gives whether it was found.
pub fn change_entity(ed: &mut Editor, at: [f64; 3], path: &[usize], change: &crate::workspace::EntityChange) -> bool {
    let dv = chunk_version(ed, at);
    let dv = (dv > 0).then_some(dv);
    edit_entity_at(ed, at, &|e| {
        if let Some(c) = rider_at(e, path) {
            apply_change(c, change, dv);
        }
    })
}

/// Makes a [`EntityChange`] to an entity as it is saved.
pub fn apply_change(e: &mut Compound, change: &crate::workspace::EntityChange, dv: Option<i64>) {
    use crate::workspace::EntityChange as C;
    match change {
        C::Yaw(y) => set_yaw(e, *y),
        C::Retype(id) => retype_entity(e, id),
        C::AddRider(id) => add_passenger(e, id, dv),
        C::Mob(m) => m.apply(e, dv),
        C::Equip(slot, item) => crate::mob_edit::set_equipment(e, *slot, item.as_deref().map(|i| (i, 1)), dv),
        C::FrameItem(Some(id)) => {
            let form = crate::mob_edit::StackForm::of(e, dv);
            e.insert("Item".into(), Tag::Compound(crate::mob_edit::make_stack(form, id, 1)));
        }
        C::FrameItem(None) => {
            e.shift_remove("Item");
            e.shift_remove("ItemRotation");
        }
        C::FrameTurn => {
            let r = e.get("ItemRotation").and_then(Tag::as_i64).unwrap_or(0);
            e.insert("ItemRotation".into(), Tag::Byte(((r + 1) % 8) as i8));
        }
        C::Invisible(on) => crate::mob_edit::set_invisible(e, *on),
        C::Flag(key, on) => {
            if *on {
                e.insert(key.clone(), Tag::Byte(1));
            } else {
                e.shift_remove(key.as_str());
            }
        }
        C::StandPose(name) => {
            e.shift_remove("Pose");
            if let Some((_, parts)) = STAND_POSES.iter().find(|(n, _)| n == name) {
                let mut pose = Compound::new();
                for (part, a) in *parts {
                    let list = List::from_tags(TagType::Float, a.iter().map(|v| Tag::Float(*v)).collect()).expect("floats");
                    pose.insert((*part).into(), Tag::List(list));
                }
                if !pose.is_empty() {
                    e.insert("Pose".into(), Tag::Compound(pose));
                }
            }
        }
        C::ClearGear => {
            for slot in crate::mob_edit::Slot::ALL {
                crate::mob_edit::set_equipment(e, slot, None, dv);
            }
        }
    }
}

/// The box a box of blocks becomes when it is turned `turns` quarter turns clockwise about its
/// middle (across: a quarter turn swaps its width and length). Its height and floor stay.
pub fn turned_box(lo: [i32; 3], hi: [i32; 3], turns: u8) -> ([i32; 3], [i32; 3]) {
    let (sx, sy, sz) = (hi[0] - lo[0] + 1, hi[1] - lo[1] + 1, hi[2] - lo[2] + 1);
    let (nx, nz) = if turns % 2 == 1 { (sz, sx) } else { (sx, sz) };
    let origin = [lo[0] + (sx - nx).div_euclid(2), lo[1], lo[2] + (sz - nz).div_euclid(2)];
    (origin, [origin[0] + nx - 1, origin[1] + sy - 1, origin[2] + nz - 1])
}

/// Turns a box of blocks (and the mobs in it, and everything they face) `turns` quarter turns
/// clockwise about its middle, in place: what was in it is cleared and put back turned.
pub fn turn_box(ed: &mut Editor, b: Bounds, turns: u8) -> Done {
    let turns = turns % 4;
    if turns == 0 {
        return Done::default();
    }
    let clip = copy(ed, b);
    let (origin, _) = turned_box(b.0, b.1, turns);
    delete_blocks(ed, b);
    delete_entities(ed, b);
    let place = Placement { turns, mirror: false };
    let done = paste(ed, &clip, origin, place, true);
    paste_entities(ed, &clip, origin, place);
    done
}

/// Sets a property (facing, half, open...) of every block in the box that has it, keeping the rest
/// of each block and what it holds. A block that has no such property, or not that value, is left.
pub fn set_box_props(ed: &mut Editor, b: Bounds, props: &[(String, String)]) -> Done {
    set_where(ed, b, |now, _| {
        let known = crate::edit::states::properties(&now.name);
        let mut new = now.clone();
        let mut changed = false;
        for (k, v) in props {
            if known.iter().any(|(pk, values)| pk == k && values.contains(v)) {
                new.properties.insert(k.clone(), v.clone());
                changed = true;
            }
        }
        changed.then_some(new)
    })
}

/// Moves one block (with what it holds) from one place to another, leaving air behind.
pub fn move_block(ed: &mut Editor, from: [i32; 3], to: [i32; 3]) -> Done {
    let clip = copy(ed, (from, from));
    if from != to {
        delete_blocks(ed, (from, from));
    }
    paste(ed, &clip, to, Placement::default(), true)
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
fn set_where(ed: &mut Editor, b: Bounds, pick: impl Fn(&BlockState, [i32; 3]) -> Option<BlockState>) -> Done {
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
                    if let Some(new) = pick(&now, [cx * 16 + x as i32, y, cz * 16 + z as i32]) {
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

/// The biome (a Java id) at a block, in whatever edition: the cell it is in.
pub fn biome_at(ed: &Editor, p: [i32; 3]) -> Option<String> {
    let c = ChunkEdit::load(ed, p[0].div_euclid(16), p[2].div_euclid(16))?;
    c.biome_cell(p[0].rem_euclid(16) as usize / 4, p[1].div_euclid(4), p[2].rem_euclid(16) as usize / 4)
}

/// The biome in each column of a chunk (row by row) at the given heights (one per column): what the flat map's Biomes view shows.
pub fn biome_columns(ed: &Editor, cx: i32, cz: i32, heights: &[i32]) -> Vec<Option<String>> {
    let Some(c) = ChunkEdit::load(ed, cx, cz) else { return Vec::new() };
    heights.iter().enumerate().map(|(i, y)| c.biome_cell((i % 16) / 4, y.div_euclid(4), (i / 16) / 4)).collect()
}

/// The biome cells (inclusive, in cells: x, height from y 0, z) a biome brush of `size` cells covers when it is on the cell holding the
/// block `at`; the lowest cell of an even size is the one the block is in. `full`: every height, whatever the height size.
pub fn biome_box(at: [i32; 3], size: [i32; 3], full: bool) -> ([i32; 3], [i32; 3]) {
    let mut lo = [0; 3];
    let mut hi = [0; 3];
    for a in 0..3 {
        let n = size[a].max(1);
        lo[a] = at[a].div_euclid(4) - (n - 1) / 2;
        hi[a] = lo[a] + n - 1;
    }
    if full {
        // The world's own limits are at most y -64 to 319.
        lo[1] = -16;
        hi[1] = 79;
    }
    (lo, hi)
}

/// Paints a biome on the cells from `lo` to `hi` (inclusive, see `biome_box`), `Done.blocks` the cells set.
pub fn biome_brush(ed: &mut Editor, lo: [i32; 3], hi: [i32; 3], biome: &str) -> Done {
    // The cells, by the chunk they are in (a cell's x and z in cells, its height in cells from y 0).
    let mut by_chunk: std::collections::BTreeMap<(i32, i32), Vec<(usize, i32, usize)>> = std::collections::BTreeMap::new();
    for cy in lo[1]..=hi[1] {
        for cz in lo[2]..=hi[2] {
            for cx in lo[0]..=hi[0] {
                by_chunk.entry((cx.div_euclid(4), cz.div_euclid(4))).or_default().push((cx.rem_euclid(4) as usize, cy, cz.rem_euclid(4) as usize));
            }
        }
    }
    let mut done = Done::default();
    for ((cx, cz), cells) in by_chunk {
        let Some(mut c) = ChunkEdit::load(ed, cx, cz) else { continue };
        for (x4, y4, z4) in cells {
            if c.set_biome_cell(x4, y4, z4, biome) {
                done.blocks += 1;
            }
        }
        done.chunks += usize::from(c.store(ed));
    }
    done
}/// Fills the box with blocks: one, or a palette mixed evenly (the same mix for the same place).
pub fn fill(ed: &mut Editor, b: Bounds, palette: &[BlockState]) -> Done {
    if palette.is_empty() {
        return Done::default();
    }
    set_where(ed, b, |_, p| Some(palette[pick_index(palette.len(), p[0], p[1], p[2])].clone()))
}

/// In the box, makes every block named as one of `from` (any state) the block `to`. Properties
/// both have (facing, half, waterlogged...) are kept when `keep_props` is set, so stairs
/// replaced by other stairs still face the same way. `to` is one block or a palette, mixed evenly.
pub fn replace(ed: &mut Editor, b: Bounds, from: &[String], to: &[BlockState], keep_props: bool) -> Done {
    let full = |n: &str| if n.contains(':') { n.to_string() } else { format!("minecraft:{n}") };
    let from: Vec<String> = from.iter().map(|n| full(n)).collect();
    if to.is_empty() {
        return Done::default();
    }
    set_where(ed, b, |now, p| {
        if !from.contains(&now.name) {
            return None;
        }
        let to = &to[pick_index(to.len(), p[0], p[1], p[2])];
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
        let s = [BlockState::new("minecraft:glass")];
        assert_eq!(brush_shape(3, true, &s, false).solid(), 179);
        assert_eq!(brush_shape(1, false, &s, false).solid(), 27);
        assert_eq!(brush_shape(0, true, &s, false).solid(), 1);
        // Painting terrain below: only the half at and below the middle.
        assert_eq!(brush_shape(1, false, &s, true).solid(), 18);
    }

    /// A palette is mixed evenly across the shape, the same blocks each time, every block of it
    /// used; one block is the old single-block paint.
    #[test]
    fn a_palette_is_mixed_across_the_shape() {
        let p = [BlockState::new("minecraft:stone"), BlockState::new("minecraft:dirt"), BlockState::new("minecraft:gravel")];
        let a = brush_shape(5, true, &p, false);
        let b = brush_shape(5, true, &p, false);
        assert_eq!(a.blocks, b.blocks, "the same mix each time");
        for k in 1..=3u32 {
            let n = a.blocks.iter().filter(|v| **v == k).count();
            let total = a.solid();
            assert!(n > total / 6, "block {k} used about a third of the time ({n} of {total})");
        }
        let one = brush_shape(2, true, &p[..1], false);
        assert!(one.blocks.iter().all(|v| *v <= 1), "a single block fills with just that block");
        assert_eq!((0..100).map(|i| pick_index(1, i, i, i)).sum::<usize>(), 0);
    }

    /// Land is not air, water or lava.
    #[test]
    fn terrain_is_not_air_or_liquid() {
        assert!(is_terrain(&BlockState::new("minecraft:grass_block")));
        assert!(!is_terrain(&BlockState::new("minecraft:air")));
        assert!(!is_terrain(&BlockState::new("minecraft:water")));
        assert!(!is_terrain(&BlockState::new("minecraft:lava")));
    }

    /// A box turns about its middle: a quarter turn swaps its width and length, a half turn leaves it.
    #[test]
    fn a_box_turns_about_its_middle() {
        assert_eq!(turned_box([10, 0, 10], [13, 1, 11], 1), ([11, 0, 9], [12, 1, 12]));
        assert_eq!(turned_box([0, 0, 0], [2, 2, 2], 1), ([0, 0, 0], [2, 2, 2]));
        assert_eq!(turned_box([5, 3, 5], [8, 4, 6], 2), ([5, 3, 5], [8, 4, 6]));
        // Four quarter turns bring it home.
        let mut b = ([3, 0, 3], [8, 2, 4]);
        for _ in 0..4 {
            b = turned_box(b.0, b.1, 1);
        }
        assert_eq!(b.1[1] - b.0[1], 2);
        assert_eq!((b.1[0] - b.0[0], b.1[2] - b.0[2]), (5, 1));
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
pub fn brush_shape(radius: i32, sphere: bool, palette: &[BlockState], below: bool) -> Clipboard {
    let r = radius.max(0);
    let n = 2 * r + 1;
    let reach = (r as f64 + 0.5).powi(2);
    let mut blocks = vec![BlockState::air()];
    blocks.extend(palette.iter().cloned());
    let mut clip = Clipboard { size: [n, n, n], palette: blocks, blocks: vec![0; (n * n * n) as usize], ..Default::default() };
    for y in 0..n {
        for z in 0..n {
            for x in 0..n {
                let d = ((x - r).pow(2) + (y - r).pow(2) + (z - r).pow(2)) as f64;
                // (Only the half at and below the middle when painting terrain below.)
                if (!sphere || d <= reach) && (!below || y <= r) && !palette.is_empty() {
                    let i = clip.index([x, y, z]);
                    // Which of the palette goes here, as the real paint will pick it (by place).
                    clip.blocks[i] = 1 + pick_index(palette.len(), x, y, z) as u32;
                }
            }
        }
    }
    clip
}

/// What the brush may paint over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BrushOnly {
    /// Every block in the shape (air too): the brush builds.
    #[default]
    Anywhere,
    /// Only blocks already there (not air, water or lava): the terrain is changed, nothing added.
    Terrain,
    /// Only that terrain at or below the block pointed at.
    TerrainBelow,
}

/// Whether a block is land: something that is there, not air, water or lava.
fn is_terrain(s: &BlockState) -> bool {
    let n = s.name.rsplit(':').next().unwrap_or(&s.name);
    !matches!(n, "air" | "cave_air" | "void_air" | "water" | "flowing_water" | "lava" | "flowing_lava" | "bubble_column")
}

/// Which of `count` blocks goes at a place: an even mix, the same each time for the same place.
pub fn pick_index(count: usize, x: i32, y: i32, z: i32) -> usize {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA6B) ^ (z as u32).wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    h as usize % count.max(1)
}

/// Paints a ball (or cube) `radius` from `center` with a mix of blocks (air to erase), over what
/// `only` allows.
pub fn brush(ed: &mut Editor, center: [i32; 3], radius: i32, sphere: bool, palette: &[BlockState], only: BrushOnly) -> Done {
    let mut done = Done::default();
    if palette.is_empty() {
        return done;
    }
    let r = radius.max(0);
    let b = ([center[0] - r, center[1] - r, center[2] - r], [center[0] + r, center[1] + r, center[2] + r]);
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
                    let here = c.get(x, y, z);
                    let allowed = match only {
                        BrushOnly::Anywhere => true,
                        BrushOnly::Terrain => is_terrain(&here),
                        BrushOnly::TerrainBelow => y <= center[1] && is_terrain(&here),
                    };
                    if !allowed {
                        continue;
                    }
                    let state = &palette[pick_index(palette.len(), wx, y, wz)];
                    if here != *state && c.set(x, y, z, state) {
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

#[cfg(test)]
mod change_tests {
    use super::*;
    use crate::workspace::EntityChange as C;

    fn stand() -> Compound {
        new_entity("minecraft:armor_stand", [1.5, 64.0, 2.5])
    }

    /// An armor stand is posed, flagged and dressed in the layout of each version; a frame holds
    /// an item in the stack form of its version, turns it and lets it go.
    #[test]
    fn armor_stands_and_frames_change_as_each_version_writes_them() {
        let mut e = stand();
        apply_change(&mut e, &C::StandPose("Walking".into()), Some(3900));
        let arm = e.get("Pose").and_then(|p| p.get("RightArm")).and_then(Tag::as_list).expect("a pose");
        assert_eq!(arm.get(0).and_then(Tag::as_f64), Some(20.0));
        apply_change(&mut e, &C::StandPose("Default".into()), Some(3900));
        assert!(e.get("Pose").is_none(), "the default pose has none");
        apply_change(&mut e, &C::Flag("ShowArms".into(), true), None);
        assert_eq!(e.get("ShowArms").and_then(Tag::as_i64), Some(1));
        apply_change(&mut e, &C::Flag("ShowArms".into(), false), None);
        assert!(e.get("ShowArms").is_none());
        // Gear: named slots from 1.21.5 (3837+ stacks with `count`), lists before.
        let mut modern = stand();
        apply_change(&mut modern, &C::Equip(crate::mob_edit::Slot::Head, Some("minecraft:iron_helmet".into())), Some(4400));
        assert!(modern.get("equipment").and_then(|g| g.get("head")).and_then(|h| h.get("count")).is_some(), "{modern:?}");
        let mut old = stand();
        apply_change(&mut old, &C::Equip(crate::mob_edit::Slot::Head, Some("minecraft:iron_helmet".into())), Some(3000));
        assert!(old.get("ArmorItems").is_some(), "{old:?}");
        apply_change(&mut old, &C::ClearGear, Some(3000));
        assert!(crate::mob_edit::equipment(&old, Some(3000)).iter().all(|(_, i)| i.is_none()));
        // A frame.
        let mut frame = new_entity("minecraft:item_frame", [0.5, 64.5, 0.5]);
        apply_change(&mut frame, &C::FrameItem(Some("minecraft:diamond".into())), Some(4300));
        assert!(frame.get("Item").and_then(|i| i.get("count")).is_some(), "1.20.5+ counts in `count`");
        let mut frame_old = new_entity("minecraft:item_frame", [0.5, 64.5, 0.5]);
        apply_change(&mut frame_old, &C::FrameItem(Some("minecraft:diamond".into())), Some(3000));
        assert!(frame_old.get("Item").and_then(|i| i.get("Count")).is_some(), "before it, a byte `Count`");
        for _ in 0..9 {
            apply_change(&mut frame, &C::FrameTurn, Some(4300));
        }
        assert_eq!(frame.get("ItemRotation").and_then(Tag::as_i64), Some(1), "eight turns come round");
        apply_change(&mut frame, &C::FrameItem(None), Some(4300));
        assert!(frame.get("Item").is_none() && frame.get("ItemRotation").is_none());
    }
}
