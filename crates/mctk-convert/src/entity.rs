//! Entity and player conversion, both ways.
//!
//! Java accepts entity data with missing fields (the game fills in defaults), so a Bedrock
//! actor is turned into the fields both editions share: type, position, motion, rotation,
//! fire and air, custom name, health, baby age, sheep colour, cushion colour and dropped
//! items. Anything Bedrock-specific (definitions, internal components, AI state) is left
//! behind.
//!
//! A Java entity becomes a Bedrock actor in one of two ways. Living mobs the vanilla behaviour
//! data describes get their `definitions` (`+minecraft:rabbit`, `+adult`) and attributes.
//! Every other entity whose type both editions share is written as Chunker writes it: the
//! type, position, motion and rotation only, which the game accepts. Paintings (see
//! `painting_to_actor`) and item frames (see `mctk_chunker::frame`) need their own handling.

use mctk_nbt::{Compound, List, Tag, TagType};

use crate::block_entity::item_to_java;
use crate::java_bedrock::EditionReport;

/// Bedrock entity ids that Java spells differently.
const RENAMED: [(&str, &str); 8] = [
    ("minecraft:villager_v2", "minecraft:villager"),
    ("minecraft:zombie_villager_v2", "minecraft:zombie_villager"),
    ("minecraft:xp_orb", "minecraft:experience_orb"),
    ("minecraft:ender_crystal", "minecraft:end_crystal"),
    ("minecraft:evocation_illager", "minecraft:evoker"),
    ("minecraft:vindication_illager", "minecraft:vindicator"),
    ("minecraft:zombie_pigman", "minecraft:zombified_piglin"),
    ("minecraft:eye_of_ender_signal", "minecraft:eye_of_ender"),
];

/// Bedrock-only or player-bound entities that have no Java counterpart.
const NOT_CONVERTED: [&str; 6] = [
    "minecraft:player",
    "minecraft:npc",
    "minecraft:agent",
    "minecraft:fishing_hook",
    "minecraft:lightning_bolt",
    "minecraft:area_effect_cloud",
];

/// Height of a standing player's eyes above their feet.
const BEDROCK_PLAYER_EYE_HEIGHT: f64 = 1.62;

/// The Java id of a Bedrock entity: Chunker's type table first, then the older Bedrock ids
/// the table does not list, and otherwise the id as it is.
fn java_entity_id(bedrock: &str) -> Option<String> {
    if NOT_CONVERTED.contains(&bedrock) {
        return None;
    }
    Some(
        mctk_chunker::entity::bedrock_to_java_type(bedrock)
            .or_else(|| RENAMED.iter().find(|(b, _)| *b == bedrock).map(|(_, j)| *j))
            .unwrap_or(bedrock)
            .to_string(),
    )
}

/// A Java UUID derived from Bedrock's unique id (Java needs one per entity).
fn uuid_from(uid: i64) -> Tag {
    Tag::IntArray(vec![0x4D43_544Bu32 as i32, (uid >> 32) as i32, uid as i32, 1])
}

/// Converts a Bedrock painting. Bedrock stores where the painting hangs as a position; Java
/// keeps the block it is attached to, so the position is turned back into a block.
fn painting_to_java(actor: &Compound, offset: (i32, i32), report: &mut EditionReport) -> Option<Compound> {
    let motive = actor
        .get("Motif")
        .or_else(|| actor.get("Motive"))
        .and_then(Tag::as_str)
        .and_then(mctk_chunker::entity::motive_from_bedrock);
    let Some(motive) = motive else {
        report.skip("entities: minecraft:painting (unknown picture)", 1);
        return None;
    };
    let facing = mctk_chunker::entity::Facing::from_byte(i64_of(actor, "Direction").unwrap_or(0));
    let p = floats(actor, "Pos", 3);
    let (mut neutral, mut tile) = mctk_chunker::entity::java_position((p[0], p[1], p[2]), motive, facing);
    neutral.0 += offset.0 as f64 * 16.0;
    neutral.2 += offset.1 as f64 * 16.0;
    tile.0 += offset.0 * 16;
    tile.2 += offset.1 * 16;
    let mut out = Compound::new();
    out.insert("id".into(), Tag::String("minecraft:painting".into()));
    out.insert("Pos".into(), double_list(&[neutral.0, neutral.1, neutral.2]));
    out.insert("Motion".into(), double_list(&[0.0, 0.0, 0.0]));
    out.insert("Rotation".into(), float_list(&floats(actor, "Rotation", 2)));
    if let Some(uid) = i64_of(actor, "UniqueID") {
        out.insert("UUID".into(), uuid_from(uid));
    }
    out.insert("variant".into(), Tag::String(motive.0.into()));
    out.insert("facing".into(), Tag::Byte(facing.to_byte()));
    out.insert("block_pos".into(), Tag::IntArray(vec![tile.0, tile.1, tile.2]));
    *report
        .converted_entities
        .entry("minecraft:painting".to_string())
        .or_insert(0) += 1;
    Some(out)
}

fn floats(c: &Compound, key: &str, n: usize) -> Vec<f64> {
    let mut v: Vec<f64> = c
        .get(key)
        .and_then(Tag::as_list)
        .map(|l| {
            l.iter()
                .map(|t| match t {
                    Tag::Float(f) => *f as f64,
                    Tag::Double(d) => *d,
                    other => other.as_i64().unwrap_or(0) as f64,
                })
                .collect()
        })
        .unwrap_or_default();
    v.resize(n, 0.0);
    v
}

fn double_list(v: &[f64]) -> Tag {
    Tag::List(
        List::from_tags(TagType::Double, v.iter().map(|d| Tag::Double(*d)).collect())
            .expect("doubles"),
    )
}

fn float_list(v: &[f64]) -> Tag {
    Tag::List(
        List::from_tags(TagType::Float, v.iter().map(|d| Tag::Float(*d as f32)).collect())
            .expect("floats"),
    )
}

/// Current value of a Bedrock attribute (`Current`, else `Base`).
fn attribute(c: &Compound, name: &str) -> Option<f64> {
    c.get("Attributes")
        .and_then(Tag::as_list)?
        .iter()
        .find(|a| a.get("Name").and_then(Tag::as_str) == Some(name))
        .and_then(|a| a.get("Current").or_else(|| a.get("Base")))
        .and_then(|v| match v {
            Tag::Float(f) => Some(*f as f64),
            Tag::Double(d) => Some(*d),
            other => other.as_i64().map(|i| i as f64),
        })
}

fn i64_of(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(Tag::as_i64)
}

/// Fields shared by every Java entity: position, motion, rotation, fire, air, ground flag.
fn common_fields(actor: &Compound, offset: (i32, i32), out: &mut Compound) {
    let mut pos = floats(actor, "Pos", 3);
    pos[0] += offset.0 as f64 * 16.0;
    pos[2] += offset.1 as f64 * 16.0;
    out.insert("Pos".into(), double_list(&pos));
    out.insert("Motion".into(), double_list(&floats(actor, "Motion", 3)));
    out.insert("Rotation".into(), float_list(&floats(actor, "Rotation", 2)));
    if let Some(v) = i64_of(actor, "Fire") {
        out.insert("Fire".into(), Tag::Short(v.clamp(-1, i16::MAX as i64) as i16));
    }
    if let Some(v) = i64_of(actor, "Air") {
        out.insert("Air".into(), Tag::Short(v as i16));
    }
    if let Some(v) = i64_of(actor, "OnGround") {
        out.insert("OnGround".into(), Tag::Byte((v != 0) as i8));
    }
}

/// Converts one Bedrock actor to a Java entity (`None` for kinds Java does not have, after
/// counting them in `report`).
pub(crate) fn actor_to_java(
    actor: &Compound,
    offset: (i32, i32),
    report: &mut EditionReport,
) -> Option<Compound> {
    let ident = actor.get("identifier").and_then(Tag::as_str)?;
    let Some(id) = java_entity_id(ident) else {
        report.skip(&format!("entities: {ident}"), 1);
        return None;
    };
    if ident == "minecraft:painting" {
        return painting_to_java(actor, offset, report);
    }
    let mut out = Compound::new();
    out.insert("id".into(), Tag::String(id.clone()));
    common_fields(actor, offset, &mut out);
    if let Some(uid) = i64_of(actor, "UniqueID") {
        // Java needs a UUID per entity; derive a stable one from Bedrock's unique id.
        out.insert("UUID".into(), uuid_from(uid));
    }
    if let Some(n) = actor.get("CustomName").and_then(Tag::as_str) {
        if !n.is_empty() {
            out.insert("CustomName".into(), Tag::String(n.to_string()));
        }
    }
    if let Some(h) = attribute(actor, "minecraft:health") {
        out.insert("Health".into(), Tag::Float(h as f32));
    }
    let baby = actor
        .get("definitions")
        .and_then(Tag::as_list)
        .is_some_and(|d| d.iter().any(|t| t.as_str() == Some("+baby")));
    if baby {
        out.insert("Age".into(), Tag::Int(-24000));
    }
    match ident {
        "minecraft:sheep" => {
            if let Some(c) = i64_of(actor, "Color") {
                out.insert("Color".into(), Tag::Byte(c as i8));
            }
            if let Some(s) = i64_of(actor, "Sheared") {
                out.insert("Sheared".into(), Tag::Byte((s != 0) as i8));
            }
        }
        "minecraft:cushion" => {
            let variant = i64_of(actor, "Variant").unwrap_or(0).clamp(0, 15);
            let dye = mctk_chunker::ids::DYES[(15 - variant) as usize].0;
            out.insert("color".into(), Tag::String(dye.into()));
        }
        "minecraft:item" => {
            let stack = actor.get("Item").and_then(Tag::as_compound)?;
            let mut item = item_to_java(stack, report)?;
            item.shift_remove("Slot");
            out.insert("Item".into(), Tag::Compound(item));
            out.insert(
                "Age".into(),
                Tag::Short(i64_of(actor, "Age").unwrap_or(0) as i16),
            );
            out.insert(
                "PickupDelay".into(),
                Tag::Short(i64_of(actor, "PickupDelay").unwrap_or(0) as i16),
            );
        }
        _ => {}
    }
    *report
        .converted_entities
        .entry(id.to_string())
        .or_insert(0) += 1;
    Some(out)
}

fn dimension_name(id: i64) -> &'static str {
    match id {
        1 => "minecraft:the_nether",
        2 => "minecraft:the_end",
        _ => "minecraft:overworld",
    }
}

/// Converts the Bedrock local player (`~local_player`) to Java's `Data.Player`.
pub(crate) fn player_to_java(
    player: &Compound,
    offset: (i32, i32),
    report: &mut EditionReport,
) -> Compound {
    let mut out = Compound::new();
    common_fields(player, offset, &mut out);
    // Bedrock stores a player's position at eye level; Java stores the feet.
    if let Some(Tag::List(pos)) = out.get_mut("Pos") {
        if let Some(Tag::Double(y)) = pos.get_mut(1) {
            *y -= BEDROCK_PLAYER_EYE_HEIGHT;
        }
    }
    out.insert(
        "Dimension".into(),
        Tag::String(dimension_name(i64_of(player, "DimensionId").unwrap_or(0)).into()),
    );
    let attr = |n: &str| attribute(player, n);
    if let Some(v) = attr("minecraft:health") {
        out.insert("Health".into(), Tag::Float(v as f32));
    }
    if let Some(v) = attr("minecraft:player.hunger") {
        out.insert("foodLevel".into(), Tag::Int(v as i32));
    }
    if let Some(v) = attr("minecraft:player.saturation") {
        out.insert("foodSaturationLevel".into(), Tag::Float(v as f32));
    }
    if let Some(v) = attr("minecraft:player.exhaustion") {
        out.insert("foodExhaustionLevel".into(), Tag::Float(v as f32));
    }
    if let Some(v) = attr("minecraft:player.level") {
        out.insert("XpLevel".into(), Tag::Int(v as i32));
    }
    if let Some(v) = attr("minecraft:player.experience") {
        out.insert("XpP".into(), Tag::Float(v as f32));
    }
    if let Some(v) = i64_of(player, "SelectedInventorySlot") {
        out.insert("SelectedItemSlot".into(), Tag::Int(v as i32));
    }
    if let Some(v) = i64_of(player, "EnchantmentSeed") {
        out.insert("XpSeed".into(), Tag::Int(v as i32));
    }
    if let Some(a) = player.get("abilities").and_then(Tag::as_compound) {
        let flag = |k: &str| a.get(k).and_then(Tag::as_i64).map(|v| Tag::Byte((v != 0) as i8));
        let mut j = Compound::new();
        for (java_key, bedrock_key) in [
            ("instabuild", "instabuild"),
            ("invulnerable", "invulnerable"),
            ("mayfly", "mayfly"),
            ("flying", "flying"),
            ("mayBuild", "build"),
        ] {
            if let Some(t) = flag(bedrock_key) {
                j.insert(java_key.into(), t);
            }
        }
        for (java_key, bedrock_key) in [("flySpeed", "flySpeed"), ("walkSpeed", "walkSpeed")] {
            if let Some(v) = a.get(bedrock_key).and_then(Tag::as_f64) {
                j.insert(java_key.into(), Tag::Float(v as f32));
            }
        }
        out.insert("abilities".into(), Tag::Compound(j));
    }

    let stacks = |key: &str, report: &mut EditionReport| -> Vec<Option<Compound>> {
        player
            .get(key)
            .and_then(Tag::as_list)
            .map(|l| {
                l.iter()
                    .map(|t| t.as_compound().and_then(|c| item_to_java(c, report)))
                    .collect()
            })
            .unwrap_or_default()
    };
    // Backpack and hotbar: slots 0–35 are numbered the same in both editions.
    let inventory: Vec<Compound> = stacks("Inventory", report).into_iter().flatten().collect();
    let count = inventory.len();
    out.insert(
        "Inventory".into(),
        Tag::List(
            List::from_tags(
                TagType::Compound,
                inventory.into_iter().map(Tag::Compound).collect(),
            )
            .expect("compounds"),
        ),
    );
    // Armor and off hand live in `equipment` (head, chest, legs, feet, offhand).
    let mut equipment = Compound::new();
    for (slot, item) in ["head", "chest", "legs", "feet"]
        .into_iter()
        .zip(stacks("Armor", report))
    {
        if let Some(mut item) = item {
            item.shift_remove("Slot");
            equipment.insert(slot.into(), Tag::Compound(item));
        }
    }
    if let Some(Some(mut item)) = stacks("Offhand", report).into_iter().next() {
        item.shift_remove("Slot");
        equipment.insert("offhand".into(), Tag::Compound(item));
    }
    if !equipment.is_empty() {
        out.insert("equipment".into(), Tag::Compound(equipment));
    }
    report
        .converted_entities
        .insert("player (inventory stacks)".into(), count);
    out
}

// ------------------------------------------------------------------ Java → Bedrock

/// Hands out the ids Bedrock gives actors in its database. An id is two big-endian words;
/// the first is 2 as in real worlds, the second counts up (7 bits per byte, so the
/// `StorageKey` stays valid UTF-8). Assigned in chunk order, so output is repeatable.
#[derive(Default)]
pub(crate) struct ActorIds {
    next: u32,
}

impl ActorIds {
    /// (database key, `UniqueID`) of the next actor.
    fn next(&mut self) -> ([u8; 8], i64) {
        self.next += 1;
        let n = self.next;
        let lo = [
            ((n >> 21) & 0x7f) as u8,
            ((n >> 14) & 0x7f) as u8,
            ((n >> 7) & 0x7f) as u8,
            (n & 0x7f) as u8,
        ];
        let key = [0, 0, 0, 2, lo[0], lo[1], lo[2], lo[3]];
        let unique = ((-2i64) << 32) | u32::from_be_bytes(lo) as i64;
        (key, unique)
    }
}

/// Java entities with no Bedrock actor to make: players are not stored as entities, and
/// an area effect cloud is Bedrock-specific.
const JAVA_SKIPPED: [&str; 2] = ["minecraft:player", "minecraft:area_effect_cloud"];

/// Java entities that are nothing without their data (the item, the block), which the shared
/// fields do not carry. Item frames are converted with their chunk instead.
const NEEDS_DATA: [&str; 4] = [
    "minecraft:item",
    "minecraft:falling_block",
    "minecraft:item_frame",
    "minecraft:glow_item_frame",
];

/// The Bedrock id of a Java entity: Chunker's type table, then the older ids it does not
/// list, and then an id Bedrock's behaviour data knows under the same name.
fn bedrock_type(java: &str) -> Option<String> {
    if JAVA_SKIPPED.contains(&java) || NEEDS_DATA.contains(&java) {
        return None;
    }
    mctk_chunker::entity::java_to_bedrock_type(java)
        .or_else(|| RENAMED.iter().find(|(_, j)| *j == java).map(|(b, _)| *b))
        .or_else(|| mctk_mapping::bedrock_entity_definitions(java, false).map(|_| java))
        .map(str::to_string)
}

fn internal_components(key: &[u8; 8]) -> Tag {
    let mut storage = Compound::new();
    storage.insert(
        "StorageKey".into(),
        // Every byte is below 0x80, so the key is also a valid string.
        Tag::String(key.iter().map(|b| *b as char).collect()),
    );
    let mut internal = Compound::new();
    internal.insert("EntityStorageKeyComponent".into(), Tag::Compound(storage));
    Tag::Compound(internal)
}

/// Whether a Java entity holds items the shared fields do not carry (a chest minecart's
/// chest, an armor stand's gear).
fn holds_items(java: &Compound) -> bool {
    ["Items", "equipment", "HandItems", "ArmorItems"]
        .iter()
        .any(|k| match java.get(*k) {
            Some(Tag::List(l)) => l.iter().any(|t| t.as_compound().is_none_or(|c| !c.is_empty())),
            Some(Tag::Compound(c)) => !c.is_empty(),
            _ => false,
        })
}

/// Writes a Java entity as Chunker does: type, position, motion and rotation, which the game
/// completes with the defaults for the type.
fn minimal_actor(
    java: &Compound,
    ident: &str,
    offset: (i32, i32),
    ids: &mut ActorIds,
    report: &mut EditionReport,
) -> Option<Actor> {
    let java_id = java.get("id").and_then(Tag::as_str)?;
    let mut pos = floats(java, "Pos", 3);
    pos[0] += offset.0 as f64 * 16.0;
    pos[2] += offset.1 as f64 * 16.0;
    let chunk = (
        (pos[0].floor() as i32).div_euclid(16),
        (pos[2].floor() as i32).div_euclid(16),
    );
    let (key, unique) = ids.next();
    let mut out = Compound::new();
    out.insert("Motion".into(), float_list(&floats(java, "Motion", 3)));
    out.insert("Pos".into(), float_list(&pos));
    out.insert("Rotation".into(), float_list(&floats(java, "Rotation", 2)));
    out.insert("Tags".into(), empty_list());
    out.insert("UniqueID".into(), Tag::Long(unique));
    out.insert("identifier".into(), Tag::String(ident.to_string()));
    out.insert("internalComponents".into(), internal_components(&key));
    if java_id == "minecraft:cushion" {
        // Bedrock numbers the dyes the other way round.
        let dye = java
            .get("color")
            .and_then(Tag::as_str)
            .and_then(|c| mctk_chunker::ids::DYES.iter().position(|(n, _, _)| *n == c))
            .unwrap_or(0);
        out.insert("Variant".into(), Tag::Int(15 - dye as i32));
    }
    if holds_items(java) {
        report.skip(&format!("entity contents: {java_id}"), 1);
    }
    *report.converted_entities.entry(java_id.to_string()).or_insert(0) += 1;
    Some(Actor { chunk, key, nbt: out })
}

/// Converts a Java painting. Java keeps the block it hangs on; Bedrock stores a position
/// that depends on the picture's size and the way it faces.
fn painting_to_actor(
    java: &Compound,
    offset: (i32, i32),
    ids: &mut ActorIds,
    report: &mut EditionReport,
) -> Option<Actor> {
    let motive = java
        .get("variant")
        .or_else(|| java.get("Motive"))
        .and_then(Tag::as_str)
        .and_then(|n| {
            mctk_chunker::entity::motive_from_java(n).or_else(|| mctk_chunker::entity::motive_from_bedrock(n))
        });
    let Some(motive) = motive else {
        report.skip("entities: minecraft:painting (unknown picture)", 1);
        return None;
    };
    let facing = mctk_chunker::entity::Facing::from_byte(
        i64_of(java, "facing").or_else(|| i64_of(java, "Facing")).unwrap_or(0),
    );
    let pos = floats(java, "Pos", 3);
    let mut tile = match java.get("block_pos") {
        Some(Tag::IntArray(a)) if a.len() == 3 => (a[0], a[1], a[2]),
        _ => (
            i64_of(java, "TileX").map_or(pos[0].floor() as i32, |v| v as i32),
            i64_of(java, "TileY").map_or(pos[1].floor() as i32, |v| v as i32),
            i64_of(java, "TileZ").map_or(pos[2].floor() as i32, |v| v as i32),
        ),
    };
    tile.0 += offset.0 * 16;
    tile.2 += offset.1 * 16;
    let p = mctk_chunker::entity::bedrock_position(tile, motive, facing);
    let chunk = (
        (p.0.floor() as i32).div_euclid(16),
        (p.2.floor() as i32).div_euclid(16),
    );
    let (key, unique) = ids.next();
    let mut out = Compound::new();
    out.insert("Direction".into(), Tag::Byte(facing.to_byte()));
    out.insert("Motif".into(), Tag::String(motive.1.into()));
    out.insert("Motion".into(), float_list(&[0.0, 0.0, 0.0]));
    out.insert("Pos".into(), float_list(&[p.0, p.1, p.2]));
    out.insert("Rotation".into(), float_list(&floats(java, "Rotation", 2)));
    out.insert("Tags".into(), empty_list());
    out.insert("UniqueID".into(), Tag::Long(unique));
    out.insert("identifier".into(), Tag::String("minecraft:painting".into()));
    let mut storage = Compound::new();
    storage.insert(
        "StorageKey".into(),
        Tag::String(key.iter().map(|b| *b as char).collect()),
    );
    let mut internal = Compound::new();
    internal.insert("EntityStorageKeyComponent".into(), Tag::Compound(storage));
    out.insert("internalComponents".into(), Tag::Compound(internal));
    *report
        .converted_entities
        .entry("minecraft:painting".to_string())
        .or_insert(0) += 1;
    Some(Actor { chunk, key, nbt: out })
}

fn str_list(items: &[String]) -> Tag {
    Tag::List(
        List::from_tags(
            TagType::String,
            items.iter().cloned().map(Tag::String).collect(),
        )
        .expect("strings"),
    )
}

/// An empty list, typed "End" as the games write it.
fn empty_list() -> Tag {
    Tag::List(List::new(TagType::End))
}

fn empty_compounds(n: usize) -> Tag {
    Tag::List(
        List::from_tags(TagType::Compound, vec![Tag::Compound(Compound::new()); n])
            .expect("compounds"),
    )
}

fn attribute_entry(name: &str, base: f32, current: Option<f32>, min: f32, max: f32) -> Tag {
    let mut a = Compound::new();
    a.insert("Base".into(), Tag::Float(base));
    if let Some(c) = current {
        a.insert("Current".into(), Tag::Float(c));
    }
    a.insert("DefaultMax".into(), Tag::Float(max));
    a.insert("DefaultMin".into(), Tag::Float(min));
    a.insert("Name".into(), Tag::String(name.into()));
    Tag::Compound(a)
}

/// Java's own maximum health of an entity, when it records one.
fn java_max_health(java: &Compound) -> Option<f64> {
    java.get("attributes")
        .and_then(Tag::as_list)?
        .iter()
        .find(|a| a.get("id").and_then(Tag::as_str) == Some("minecraft:max_health"))
        .and_then(|a| a.get("base"))
        .and_then(Tag::as_f64)
}

/// A converted Java entity: the Bedrock actor and the chunk it belongs to.
pub(crate) struct Actor {
    pub chunk: (i32, i32),
    pub key: [u8; 8],
    pub nbt: Compound,
}

/// Converts one Java entity to a Bedrock actor; entities with no Bedrock form are counted in
/// `report`.
pub(crate) fn entity_to_actor(
    java: &Compound,
    offset: (i32, i32),
    ids: &mut ActorIds,
    report: &mut EditionReport,
) -> Option<Actor> {
    let java_id = java.get("id").and_then(Tag::as_str)?;
    if java_id == "minecraft:painting" {
        return painting_to_actor(java, offset, ids, report);
    }
    let Some(ident) = bedrock_type(java_id) else {
        report.skip(&format!("entities: {java_id}"), 1);
        return None;
    };
    let baby = java.get("Age").and_then(Tag::as_i64).is_some_and(|a| a < 0);
    let definitions = java
        .contains_key("Health")
        .then(|| mctk_mapping::bedrock_entity_definitions(&ident, baby))
        .flatten();
    let Some(definitions) = definitions else {
        return minimal_actor(java, &ident, offset, ids, report);
    };

    let mut pos = floats(java, "Pos", 3);
    pos[0] += offset.0 as f64 * 16.0;
    pos[2] += offset.1 as f64 * 16.0;
    let chunk = (
        (pos[0].floor() as i32).div_euclid(16),
        (pos[2].floor() as i32).div_euclid(16),
    );
    let (key, unique) = ids.next();

    let mut out = Compound::new();
    {
        out.insert("Air".into(), Tag::Short(i64_of(java, "Air").unwrap_or(300) as i16));
        out.insert("Armor".into(), empty_compounds(5));
        let health = java.get("Health").and_then(Tag::as_f64).unwrap_or(20.0) as f32;
        let max = java_max_health(java).map_or(health, |m| m as f32).max(health);
        out.insert(
            "Attributes".into(),
            Tag::List(
                List::from_tags(
                    TagType::Compound,
                    vec![attribute_entry(
                        "minecraft:health",
                        max,
                        (health != max).then_some(health),
                        0.0,
                        max,
                    )],
                )
                .expect("compounds"),
            ),
        );
        out.insert("LeasherID".into(), Tag::Long(-1));
        out.insert("OwnerNew".into(), Tag::Long(-1));
        out.insert("TargetID".into(), Tag::Long(-1));
        out.insert("Mainhand".into(), empty_compounds(1));
        out.insert("Offhand".into(), empty_compounds(1));
        out.insert("canPickupItems".into(), Tag::Byte(1));
        out.insert("hasSetCanPickupItems".into(), Tag::Byte(1));
    }
    let motion = floats(java, "Motion", 3);
    if motion.iter().any(|m| *m != 0.0) {
        out.insert("Motion".into(), float_list(&motion));
    }
    if let Some(n) = java.get("CustomName") {
        let name = crate::block_entity::plain_text(n);
        if !name.is_empty() {
            out.insert("CustomName".into(), Tag::String(name));
            if i64_of(java, "CustomNameVisible").unwrap_or(0) != 0 {
                out.insert("CustomNameVisible".into(), Tag::Byte(1));
            }
        }
    }
    if let Some(fire) = i64_of(java, "Fire").filter(|f| *f > 0) {
        out.insert("Fire".into(), Tag::Short(fire as i16));
    }
    if i64_of(java, "OnGround").unwrap_or(0) != 0 {
        out.insert("OnGround".into(), Tag::Byte(1));
    }
    out.insert("Pos".into(), float_list(&pos));
    out.insert("Rotation".into(), float_list(&floats(java, "Rotation", 2)));
    out.insert("Tags".into(), empty_list());
    out.insert("UniqueID".into(), Tag::Long(unique));
    out.insert("definitions".into(), str_list(&definitions));
    out.insert("expDropEnabled".into(), Tag::Byte(1));
    out.insert("identifier".into(), Tag::String(ident.clone()));
    out.insert("internalComponents".into(), internal_components(&key));

    match java_id {
        "minecraft:sheep" => {
            out.insert("Color".into(), Tag::Byte(i64_of(java, "Color").unwrap_or(0) as i8));
            out.insert(
                "Sheared".into(),
                Tag::Byte((i64_of(java, "Sheared").unwrap_or(0) != 0) as i8),
            );
        }
        _ => {}
    }
    *report.converted_entities.entry(java_id.to_string()).or_insert(0) += 1;
    Some(Actor { chunk, key, nbt: out })
}

fn java_dimension_id(name: &str) -> i32 {
    match name {
        "minecraft:the_nether" => 1,
        "minecraft:the_end" => 2,
        _ => 0,
    }
}

fn abilities(game_mode: i32) -> Tag {
    let creative = game_mode == 1;
    let adventure = game_mode == 2;
    let flag = |b: bool| Tag::Byte(b as i8);
    let mut a = Compound::new();
    a.insert("attackmobs".into(), flag(!adventure));
    a.insert("attackplayers".into(), flag(!adventure));
    a.insert("build".into(), flag(!adventure));
    a.insert("doorsandswitches".into(), flag(true));
    a.insert("flySpeed".into(), Tag::Float(0.05));
    a.insert("flying".into(), flag(false));
    a.insert("instabuild".into(), flag(creative));
    a.insert("invulnerable".into(), flag(creative));
    a.insert("lightning".into(), flag(false));
    a.insert("mayfly".into(), flag(creative));
    a.insert("mine".into(), flag(!adventure));
    a.insert("op".into(), flag(false));
    a.insert("opencontainers".into(), flag(true));
    a.insert("teleport".into(), flag(false));
    a.insert("verticalFlySpeed".into(), Tag::Float(1.0));
    a.insert("walkSpeed".into(), Tag::Float(0.1));
    Tag::Compound(a)
}

/// Bedrock abilities from Java's `abilities` compound; what Java does not record (attack
/// and container permissions, teleport) follows the game mode.
fn abilities_from_java(java: &Compound, game_mode: i32) -> Tag {
    let Tag::Compound(mut a) = abilities(game_mode) else {
        unreachable!("abilities builds a compound")
    };
    for (bedrock, java_key) in [
        ("instabuild", "instabuild"),
        ("invulnerable", "invulnerable"),
        ("mayfly", "mayfly"),
        ("flying", "flying"),
        ("build", "mayBuild"),
    ] {
        if let Some(v) = i64_of(java, java_key) {
            a.insert(bedrock.into(), Tag::Byte((v != 0) as i8));
        }
    }
    if let Some(v) = java.get("flySpeed").and_then(Tag::as_f64) {
        a.insert("flySpeed".into(), Tag::Float(v as f32));
    }
    if let Some(v) = java.get("walkSpeed").and_then(Tag::as_f64) {
        a.insert("walkSpeed".into(), Tag::Float(v as f32));
    }
    Tag::Compound(a)
}

/// Converts Java's `Data.Player` to Bedrock's `~local_player` record. `game_mode` is the
/// world's game mode (0 survival, 1 creative, 2 adventure, 3 spectator).
pub(crate) fn player_to_bedrock(
    java: &Compound,
    game_mode: i32,
    offset: (i32, i32),
    report: &mut EditionReport,
) -> Compound {
    let f = |k: &str, d: f64| java.get(k).and_then(Tag::as_f64).unwrap_or(d) as f32;
    let mut pos = floats(java, "Pos", 3);
    pos[0] += offset.0 as f64 * 16.0;
    pos[1] += BEDROCK_PLAYER_EYE_HEIGHT;
    pos[2] += offset.1 as f64 * 16.0;

    // Bedrock lists every one of the 36 backpack and hotbar slots, empty ones included.
    let mut slots: Vec<Option<Compound>> = vec![None; 36];
    let mut armor: [Option<Compound>; 4] = [None, None, None, None];
    let mut offhand = None;
    let mut converted = 0;
    for stack in java.get("Inventory").and_then(Tag::as_list).map(List::iter).into_iter().flatten() {
        let Some(stack) = stack.as_compound() else { continue };
        let slot = i64_of(stack, "Slot").unwrap_or(-1);
        let Some(mut item) = crate::block_entity::item_to_bedrock(stack, report) else {
            continue;
        };
        converted += 1;
        match slot {
            0..=35 => slots[slot as usize] = Some(item),
            // Pre-1.21.5 worlds keep armor and the off hand in the inventory.
            100..=103 => {
                item.shift_remove("Slot");
                armor[(103 - slot) as usize] = Some(item);
            }
            -106 => {
                item.shift_remove("Slot");
                offhand = Some(item);
            }
            _ => report.skip("inventory stacks in unknown slots", 1),
        }
    }
    if let Some(eq) = java.get("equipment").and_then(Tag::as_compound) {
        for (i, name) in ["head", "chest", "legs", "feet"].into_iter().enumerate() {
            if let Some(stack) = eq.get(name).and_then(Tag::as_compound) {
                if let Some(mut item) = crate::block_entity::item_to_bedrock(stack, report) {
                    item.shift_remove("Slot");
                    armor[i] = Some(item);
                    converted += 1;
                }
            }
        }
        if let Some(stack) = eq.get("offhand").and_then(Tag::as_compound) {
            if let Some(mut item) = crate::block_entity::item_to_bedrock(stack, report) {
                item.shift_remove("Slot");
                offhand = Some(item);
                converted += 1;
            }
        }
    }
    let fill = |slots: Vec<Option<Compound>>| -> Vec<Tag> {
        slots
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                Tag::Compound(s.unwrap_or_else(|| {
                    let mut e = Compound::new();
                    e.insert("Count".into(), Tag::Byte(0));
                    e.insert("Damage".into(), Tag::Short(0));
                    e.insert("Name".into(), Tag::String(String::new()));
                    e.insert("Slot".into(), Tag::Byte(i as i8));
                    e.insert("WasPickedUp".into(), Tag::Byte(0));
                    e
                }))
            })
            .collect()
    };
    let inventory = fill(slots);
    // The ender chest has 27 slots, listed the same way.
    let mut ender: Vec<Option<Compound>> = vec![None; 27];
    for stack in java.get("EnderItems").and_then(Tag::as_list).map(List::iter).into_iter().flatten() {
        let Some(stack) = stack.as_compound() else { continue };
        let slot = i64_of(stack, "Slot").unwrap_or(-1);
        if let (0..=26, Some(item)) = (slot, crate::block_entity::item_to_bedrock(stack, report)) {
            ender[slot as usize] = Some(item);
            converted += 1;
        }
    }
    let ender = fill(ender);
    let compound_list = |items: Vec<Tag>| {
        Tag::List(List::from_tags(TagType::Compound, items).expect("compounds"))
    };
    let mut armor_tags: Vec<Tag> = armor
        .into_iter()
        .map(|a| Tag::Compound(a.unwrap_or_default()))
        .collect();
    armor_tags.push(Tag::Compound(Compound::new()));

    let health = f("Health", 20.0);
    let attributes = vec![
        attribute_entry("minecraft:player.hunger", 20.0, Some(f("foodLevel", 20.0)), 0.0, 20.0),
        attribute_entry("minecraft:player.saturation", 5.0, Some(f("foodSaturationLevel", 5.0)), 0.0, 20.0),
        attribute_entry("minecraft:player.exhaustion", 0.0, Some(f("foodExhaustionLevel", 0.0)), 0.0, 20.0),
        attribute_entry("minecraft:player.level", 0.0, Some(f("XpLevel", 0.0)), 0.0, 24791.0),
        attribute_entry("minecraft:player.experience", 0.0, Some(f("XpP", 0.0)), 0.0, 1.0),
        attribute_entry("minecraft:health", 20.0, Some(health), 0.0, 20.0),
        attribute_entry("minecraft:movement", 0.1, None, 0.0, f32::MAX),
        attribute_entry("minecraft:attack_damage", 1.0, None, 1.0, 1.0),
    ];

    let mut out = Compound::new();
    out.insert("Air".into(), Tag::Short(i64_of(java, "Air").unwrap_or(300) as i16));
    out.insert("Armor".into(), compound_list(armor_tags));
    out.insert("Attributes".into(), compound_list(attributes));
    out.insert("DimensionId".into(), Tag::Int(java_dimension_id(
        java.get("Dimension").and_then(Tag::as_str).unwrap_or("minecraft:overworld"),
    )));
    out.insert("Inventory".into(), compound_list(inventory));
    out.insert("EnderChestInventory".into(), compound_list(ender));
    // Interface and bookkeeping fields at the values a new player has.
    for (key, value) in [
        ("BlastFurnaceLayout", 0),
        ("CraftingLayout", 0),
        ("FurnaceLayout", 0),
        ("InventoryLayout", 0),
        ("SmokerLayout", 0),
        ("LeftBlastFurnaceTabSelection", 4),
        ("LeftFurnaceTabSelection", 4),
        ("LeftInventoryTabSelection", 5),
        ("LeftSmokerTabSelection", 4),
        ("RightInventoryTabSelection", 2),
        ("SelectedContainerId", 0),
        ("TimeSinceRest", 0),
        ("TradeExperience", 0),
        ("TradeTier", 0),
        ("WardenThreatDecreaseTimer", 0),
        ("WardenThreatLevel", 0),
        ("WardenThreatLevelIncreaseCooldown", 0),
        // No bed or respawn point set.
        ("SpawnBlockPositionX", i32::MIN),
        ("SpawnBlockPositionY", i32::MIN),
        ("SpawnBlockPositionZ", i32::MIN),
        ("SpawnX", i32::MIN),
        ("SpawnY", i32::MIN),
        ("SpawnZ", i32::MIN),
        ("SpawnDimension", 3),
    ] {
        out.insert(key.into(), Tag::Int(value));
    }
    for key in [
        "BlastFurnaceFiltering",
        "FurnaceFiltering",
        "InventoryFiltering",
        "SmokerFiltering",
        "HasSeenCredits",
        "Sitting",
        "Sleeping",
        "Sneaking",
    ] {
        out.insert(key.into(), Tag::Byte(0));
    }
    if let Some(seed) = i64_of(java, "XpSeed") {
        out.insert("EnchantmentSeed".into(), Tag::Int(seed as i32));
    }
    out.insert("SleepTimer".into(), Tag::Short(0));
    for key in ["PlayerUIItems", "SlotDropChances", "fogCommandStack"] {
        out.insert(key.into(), empty_list());
    }
    out.insert("LeasherID".into(), Tag::Long(-1));
    out.insert("Mainhand".into(), empty_compounds(1));
    out.insert(
        "Offhand".into(),
        compound_list(vec![Tag::Compound(offhand.unwrap_or_default())]),
    );
    out.insert("OnGround".into(), Tag::Byte(i64_of(java, "OnGround").unwrap_or(1) as i8));
    out.insert("OwnerNew".into(), Tag::Long(-1));
    // 5 = "follow the world's game mode".
    out.insert("PlayerGameMode".into(), Tag::Int(5));
    out.insert("PlayerLevel".into(), Tag::Int(i64_of(java, "XpLevel").unwrap_or(0) as i32));
    out.insert("PlayerLevelProgress".into(), Tag::Float(f("XpP", 0.0)));
    out.insert("Pos".into(), float_list(&pos));
    out.insert("Rotation".into(), float_list(&floats(java, "Rotation", 2)));
    out.insert(
        "SelectedInventorySlot".into(),
        Tag::Int(i64_of(java, "SelectedItemSlot").unwrap_or(0) as i32),
    );
    out.insert("Tags".into(), empty_list());
    out.insert("TargetID".into(), Tag::Long(-1));
    out.insert("UniqueID".into(), Tag::Long(-4294967295));
    out.insert(
        "abilities".into(),
        match java.get("abilities").and_then(Tag::as_compound) {
            Some(a) => abilities_from_java(a, game_mode),
            None => abilities(game_mode),
        },
    );
    out.insert("canPickupItems".into(), Tag::Byte(1));
    out.insert("definitions".into(), str_list(&["+minecraft:player".to_string()]));
    out.insert("expDropEnabled".into(), Tag::Byte(1));
    out.insert("format_version".into(), Tag::String("1.12.0".into()));
    out.insert("hasSetCanPickupItems".into(), Tag::Byte(1));
    out.insert("identifier".into(), Tag::String("minecraft:player".into()));
    let mut storage = Compound::new();
    storage.insert(
        "StorageKey".into(),
        Tag::String("\0\0\0\u{1}\0\0\0\u{1}".into()),
    );
    let mut internal = Compound::new();
    internal.insert("EntityStorageKeyComponent".into(), Tag::Compound(storage));
    out.insert("internalComponents".into(), Tag::Compound(internal));
    report
        .converted_entities
        .insert("player (inventory stacks)".into(), converted);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rabbit() -> Compound {
        let mut a = Compound::new();
        a.insert("identifier".into(), Tag::String("minecraft:rabbit".into()));
        a.insert("UniqueID".into(), Tag::Long(-8589934571));
        a.insert("Pos".into(), float_list(&[66.5, 64.0, -240.5]));
        a.insert("Rotation".into(), float_list(&[-89.0, 0.0]));
        a.insert(
            "definitions".into(),
            Tag::List(
                List::from_tags(
                    TagType::String,
                    vec![Tag::String("+minecraft:rabbit".into()), Tag::String("+baby".into())],
                )
                .unwrap(),
            ),
        );
        let mut health = Compound::new();
        health.insert("Name".into(), Tag::String("minecraft:health".into()));
        health.insert("Base".into(), Tag::Float(3.0));
        a.insert(
            "Attributes".into(),
            Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(health)]).unwrap()),
        );
        a
    }

    #[test]
    fn rabbit_keeps_position_health_and_age() {
        let mut report = EditionReport::default();
        let j = actor_to_java(&rabbit(), (1, -1), &mut report).unwrap();
        assert_eq!(j.get("id").and_then(Tag::as_str), Some("minecraft:rabbit"));
        let pos = j.get("Pos").and_then(Tag::as_list).unwrap();
        assert_eq!(pos.get(0), Some(&Tag::Double(82.5)));
        assert_eq!(pos.get(2), Some(&Tag::Double(-256.5)));
        assert_eq!(j.get("Health"), Some(&Tag::Float(3.0)));
        assert_eq!(j.get("Age"), Some(&Tag::Int(-24000)));
        assert!(j.get("UUID").is_some());
        assert_eq!(report.converted_entities.get("minecraft:rabbit"), Some(&1));
    }

    fn java_cow(age: i32) -> Compound {
        let mut c = Compound::new();
        c.insert("id".into(), Tag::String("minecraft:cow".into()));
        c.insert("Pos".into(), double_list(&[-17.5, 70.0, 40.25]));
        c.insert("Rotation".into(), float_list(&[90.0, 0.0]));
        c.insert("Motion".into(), double_list(&[0.0, 0.0, 0.0]));
        c.insert("Health".into(), Tag::Float(7.0));
        c.insert("Age".into(), Tag::Int(age));
        c.insert("OnGround".into(), Tag::Byte(1));
        c
    }

    #[test]
    fn java_cow_becomes_an_actor_with_definitions_and_ids() {
        let mut report = EditionReport::default();
        let mut ids = ActorIds::default();
        let a = entity_to_actor(&java_cow(0), (0, 0), &mut ids, &mut report).unwrap();
        assert_eq!(a.chunk, (-2, 2));
        let defs: Vec<_> = a
            .nbt
            .get("definitions")
            .and_then(Tag::as_list)
            .unwrap()
            .iter()
            .filter_map(Tag::as_str)
            .collect();
        assert_eq!(defs, ["+minecraft:cow", "+minecraft:cow_adult"]);
        // Same relation as in real worlds: key = (2, n), UniqueID = (-2 << 32) | n.
        assert_eq!(&a.key[..4], &[0, 0, 0, 2]);
        let n = u32::from_be_bytes(a.key[4..].try_into().unwrap()) as i64;
        assert_eq!(a.nbt.get("UniqueID"), Some(&Tag::Long(((-2i64) << 32) | n)));
        let storage = a.nbt["internalComponents"]
            .get("EntityStorageKeyComponent")
            .and_then(|c| c.get("StorageKey"))
            .and_then(Tag::as_str)
            .unwrap();
        assert_eq!(storage.as_bytes(), a.key);

        let baby = entity_to_actor(&java_cow(-20000), (0, 0), &mut ids, &mut report).unwrap();
        let defs = baby.nbt.get("definitions").and_then(Tag::as_list).unwrap();
        assert_eq!(defs.get(1).and_then(Tag::as_str), Some("+minecraft:cow_baby"));
        assert_ne!(a.key, baby.key);
    }

    #[test]
    fn actor_ids_stay_valid_strings_for_many_actors() {
        let mut ids = ActorIds::default();
        for _ in 0..70_000 {
            let (key, _) = ids.next();
            assert!(key.iter().all(|b| *b < 0x80));
        }
    }

    #[test]
    fn entities_without_their_data_are_counted_not_converted() {
        for id in ["minecraft:item", "minecraft:falling_block", "minecraft:not_a_real_entity"] {
            let mut c = Compound::new();
            c.insert("id".into(), Tag::String(id.into()));
            c.insert("Pos".into(), double_list(&[0.0, 0.0, 0.0]));
            let mut report = EditionReport::default();
            assert!(entity_to_actor(&c, (0, 0), &mut ActorIds::default(), &mut report).is_none());
            assert_eq!(report.skipped.get(&format!("entities: {id}")), Some(&1), "{id}");
        }
    }

    #[test]
    fn other_entities_are_written_as_chunker_writes_them() {
        let mut c = Compound::new();
        c.insert("id".into(), Tag::String("minecraft:chest_minecart".into()));
        c.insert("Pos".into(), double_list(&[17.5, 64.0, -3.0]));
        c.insert("Motion".into(), double_list(&[0.0, -0.1, 0.0]));
        c.insert("Rotation".into(), float_list(&[90.0, 0.0]));
        let mut chest = Compound::new();
        chest.insert("id".into(), Tag::String("minecraft:diamond".into()));
        c.insert(
            "Items".into(),
            Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(chest)]).unwrap()),
        );
        let mut report = EditionReport::default();
        let a = entity_to_actor(&c, (1, 0), &mut ActorIds::default(), &mut report).expect("an actor");
        assert_eq!(a.nbt.get("identifier").and_then(Tag::as_str), Some("minecraft:chest_minecart"));
        // Moved a chunk east; (33.5, -3.0) lies in chunk (2, -1).
        assert_eq!(a.chunk, (2, -1));
        assert_eq!(floats(&a.nbt, "Pos", 3), vec![33.5, 64.0, -3.0]);
        assert_eq!(floats(&a.nbt, "Rotation", 2), vec![90.0, 0.0]);
        assert!(a.nbt.get("definitions").is_none());
        assert!(a.nbt.get("internalComponents").is_some());
        // The chest's contents are not carried over, and that is reported.
        assert_eq!(report.skipped.get("entity contents: minecraft:chest_minecart"), Some(&1));
        assert_eq!(report.converted_entities.get("minecraft:chest_minecart"), Some(&1));
    }

    #[test]
    fn cushions_keep_their_colour_both_ways() {
        let mut j = Compound::new();
        j.insert("id".into(), Tag::String("minecraft:cushion".into()));
        j.insert("Pos".into(), double_list(&[1.5, 70.0, 2.5]));
        j.insert("color".into(), Tag::String("lime".into()));
        let mut report = EditionReport::default();
        let a = entity_to_actor(&j, (0, 0), &mut ActorIds::default(), &mut report).expect("an actor");
        // Lime is dye 5, which Bedrock numbers 15 - 5.
        assert_eq!(a.nbt.get("Variant"), Some(&Tag::Int(10)));
        let back = actor_to_java(&a.nbt, (0, 0), &mut report).expect("a cushion");
        assert_eq!(back.get("id").and_then(Tag::as_str), Some("minecraft:cushion"));
        assert_eq!(back.get("color").and_then(Tag::as_str), Some("lime"));
    }

    #[test]
    fn player_survives_a_round_trip() {
        let mut java = Compound::new();
        java.insert("Pos".into(), double_list(&[10.5, 64.0, -3.25]));
        java.insert("Rotation".into(), float_list(&[12.0, -5.0]));
        java.insert("Health".into(), Tag::Float(14.0));
        java.insert("foodLevel".into(), Tag::Int(17));
        java.insert("XpLevel".into(), Tag::Int(9));
        java.insert("SelectedItemSlot".into(), Tag::Int(3));
        let mut stack = Compound::new();
        stack.insert("Slot".into(), Tag::Byte(3));
        stack.insert("id".into(), Tag::String("minecraft:diamond_pickaxe".into()));
        stack.insert("count".into(), Tag::Int(1));
        java.insert(
            "Inventory".into(),
            Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(stack)]).unwrap()),
        );
        let mut report = EditionReport::default();
        let b = player_to_bedrock(&java, 0, (0, 0), &mut report);
        let inv = b.get("Inventory").and_then(Tag::as_list).unwrap();
        assert_eq!(inv.len(), 36);
        assert_eq!(
            inv.get(3).and_then(|s| s.get("Name")).and_then(Tag::as_str),
            Some("minecraft:diamond_pickaxe")
        );
        assert_eq!(inv.get(4).and_then(|s| s.get("Name")).and_then(Tag::as_str), Some(""));

        let back = player_to_java(&b, (0, 0), &mut report);
        let pos = back.get("Pos").and_then(Tag::as_list).unwrap();
        assert!(matches!(pos.get(1), Some(Tag::Double(y)) if (y - 64.0).abs() < 1e-3));
        assert_eq!(back.get("Health"), Some(&Tag::Float(14.0)));
        assert_eq!(back.get("foodLevel"), Some(&Tag::Int(17)));
        assert_eq!(back.get("XpLevel"), Some(&Tag::Int(9)));
        assert_eq!(back.get("SelectedItemSlot"), Some(&Tag::Int(3)));
        let stack = back.get("Inventory").and_then(Tag::as_list).unwrap().get(0).unwrap();
        assert_eq!(stack.get("id").and_then(Tag::as_str), Some("minecraft:diamond_pickaxe"));
    }

    #[test]
    fn player_position_moves_from_eyes_to_feet() {
        let mut p = Compound::new();
        p.insert("Pos".into(), float_list(&[10.0, 65.62, 5.0]));
        let j = player_to_java(&p, (0, 0), &mut EditionReport::default());
        let y = match j.get("Pos").and_then(Tag::as_list).and_then(|l| l.get(1)) {
            Some(Tag::Double(y)) => *y,
            other => panic!("unexpected {other:?}"),
        };
        assert!((y - 64.0).abs() < 1e-4, "{y}");
    }

    #[test]
    fn paintings_keep_picture_facing_and_block_through_both_editions() {
        use mctk_chunker::entity::{Facing, MOTIVES, bedrock_position, motive_from_java};
        for motive in &MOTIVES {
            for facing in [Facing::South, Facing::West, Facing::North, Facing::East] {
                let mut java = Compound::new();
                java.insert("id".into(), Tag::String("minecraft:painting".into()));
                java.insert("variant".into(), Tag::String(motive.0.into()));
                java.insert("facing".into(), Tag::Byte(facing.to_byte()));
                java.insert("block_pos".into(), Tag::IntArray(vec![-20, 70, 33]));
                java.insert("Pos".into(), double_list(&[-19.5, 70.5, 32.97]));
                let mut report = EditionReport::default();
                let a = entity_to_actor(&java, (1, 0), &mut ActorIds::default(), &mut report).unwrap();
                assert_eq!(a.nbt.get("Motif").and_then(Tag::as_str), Some(motive.1));
                assert_eq!(a.nbt.get("Direction"), Some(&Tag::Byte(facing.to_byte())));
                // The chunk offset moved it one chunk east.
                let expect = bedrock_position((-4, 70, 33), motive_from_java(motive.0).unwrap(), facing);
                let pos = a.nbt.get("Pos").and_then(Tag::as_list).unwrap();
                assert!(matches!(pos.get(0), Some(Tag::Float(x)) if (*x as f64 - expect.0).abs() < 1e-4));

                let back = actor_to_java(&a.nbt, (-1, 0), &mut report).unwrap();
                assert_eq!(back.get("variant").and_then(Tag::as_str), Some(motive.0));
                assert_eq!(back.get("block_pos"), Some(&Tag::IntArray(vec![-20, 70, 33])), "{} {facing:?}", motive.0);
                assert_eq!(back.get("facing"), Some(&Tag::Byte(facing.to_byte())));
            }
        }
    }

    #[test]
    fn unknown_paintings_are_counted() {
        let mut java = Compound::new();
        java.insert("id".into(), Tag::String("minecraft:painting".into()));
        java.insert("variant".into(), Tag::String("minecraft:not_a_picture".into()));
        let mut report = EditionReport::default();
        assert!(entity_to_actor(&java, (0, 0), &mut ActorIds::default(), &mut report).is_none());
        assert_eq!(report.skipped.get("entities: minecraft:painting (unknown picture)"), Some(&1));
    }

    #[test]
    fn unconvertible_kinds_are_counted() {
        let mut a = Compound::new();
        a.insert("identifier".into(), Tag::String("minecraft:npc".into()));
        let mut report = EditionReport::default();
        assert!(actor_to_java(&a, (0, 0), &mut report).is_none());
        assert_eq!(report.skipped.get("entities: minecraft:npc"), Some(&1));
    }

    #[test]
    fn renamed_ids() {
        assert_eq!(java_entity_id("minecraft:villager_v2").as_deref(), Some("minecraft:villager"));
        assert_eq!(java_entity_id("minecraft:cow").as_deref(), Some("minecraft:cow"));
        assert_eq!(java_entity_id("minecraft:ender_crystal").as_deref(), Some("minecraft:end_crystal"));
    }
}

