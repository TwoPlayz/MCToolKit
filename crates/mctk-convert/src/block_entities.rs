//! Block entities between Java (26.x) and Bedrock, for the kinds that are plain data: containers,
//! furnaces, hoppers, campfires, comparators, mob spawners and signs.
//!
//! Each conversion maps one compound to the other; the shapes were read from worlds the two games
//! saved. A kind this module does not know
//! comes back as `None` and is left to the caller.
//!
//! Positions are not touched here (the caller shifts them). A few kinds depend on the block they
//! sit in: a chest's pair (its `type` and facing on Java, `pairx`/`pairz` on Bedrock) and a
//! shulker box's facing.

use mctk_java::BlockState;
use mctk_nbt::{Compound, List, Tag, TagType};

use crate::item_codec::{self, Notes};
use crate::text_codec;

/// Changes to the block a block entity sits in: a new name, and properties to set (`Some`) or
/// remove (`None`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockEdit {
    pub name: Option<String>,
    pub props: Vec<(&'static str, Option<String>)>,
}

impl BlockEdit {
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.props.is_empty()
    }

    pub fn apply(&self, state: &mut BlockState) {
        if let Some(n) = &self.name {
            state.name = n.clone();
        }
        for (k, v) in &self.props {
            match v {
                Some(v) => {
                    state.properties.insert((*k).to_string(), v.clone());
                }
                None => {
                    state.properties.shift_remove(*k);
                }
            }
        }
    }
}

/// The result of converting a Bedrock block entity: the Java entity (`None` when the block carries
/// everything) and the changes to the block.
pub struct ToJava {
    pub nbt: Option<Compound>,
    pub edit: BlockEdit,
}

fn int(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(Tag::as_i64)
}

fn text<'a>(c: &'a Compound, key: &str) -> Option<&'a str> {
    c.get(key).and_then(Tag::as_str)
}

fn compounds(items: Vec<Compound>) -> Tag {
    if items.is_empty() {
        return Tag::List(List::new(TagType::End));
    }
    Tag::List(List::from_tags(TagType::Compound, items.into_iter().map(Tag::Compound).collect()).expect("compounds"))
}

fn items_of<'a>(c: &'a Compound, key: &str) -> Vec<&'a Compound> {
    c.get(key).and_then(Tag::as_list).map(|l| l.iter().filter_map(Tag::as_compound).collect()).unwrap_or_default()
}

/// (Java id, Bedrock id) of every kind handled.
const KINDS: [(&str, &str); 16] = [
    ("minecraft:chest", "Chest"),
    ("minecraft:trapped_chest", "Chest"),
    ("minecraft:barrel", "Barrel"),
    ("minecraft:ender_chest", "EnderChest"),
    ("minecraft:shulker_box", "ShulkerBox"),
    ("minecraft:dispenser", "Dispenser"),
    ("minecraft:dropper", "Dropper"),
    ("minecraft:hopper", "Hopper"),
    ("minecraft:furnace", "Furnace"),
    ("minecraft:blast_furnace", "BlastFurnace"),
    ("minecraft:smoker", "Smoker"),
    ("minecraft:campfire", "Campfire"),
    ("minecraft:comparator", "Comparator"),
    ("minecraft:mob_spawner", "MobSpawner"),
    ("minecraft:sign", "Sign"),
    ("minecraft:hanging_sign", "HangingSign"),
];

/// The Bedrock id of a Java block entity id this module converts.
pub fn bedrock_id(java: &str) -> Option<&'static str> {
    KINDS.iter().find(|(j, _)| *j == java).map(|(_, b)| *b)
}

fn java_id_of(bedrock: &str, block: Option<&BlockState>) -> Option<&'static str> {
    let block_name = block.map(|b| b.name.as_str()).unwrap_or("");
    match bedrock {
        "Chest" if block_name == "minecraft:trapped_chest" => Some("minecraft:trapped_chest"),
        "ShulkerBox" => Some("minecraft:shulker_box"),
        other => KINDS.iter().find(|(_, b)| *b == other).map(|(j, _)| *j),
    }
}

// ---------------------------------------------------------------------------------------------
// Java -> Bedrock

fn bedrock_items(c: &Compound, notes: &mut Notes) -> Tag {
    let mut out = Vec::new();
    for item in items_of(c, "Items") {
        let slot = int(item, "Slot").map(|s| s as u8);
        if let Some(b) = item_codec::java_to_bedrock(item, slot, notes) {
            out.push(b);
        }
    }
    compounds(out)
}

fn loot(java: &Compound, out: &mut Compound) {
    if let Some(t) = text(java, "LootTable") {
        let path = t.trim_start_matches("minecraft:");
        out.insert("LootTable".into(), Tag::String(format!("loot_tables/{path}.json")));
        let seed = int(java, "LootTableSeed").unwrap_or(0);
        out.insert("LootTableSeed".into(), Tag::Int(seed as i32));
    }
}

fn name_of(java: &Compound, out: &mut Compound) {
    if let Some(n) = java.get("CustomName") {
        out.insert("CustomName".into(), Tag::String(text_codec::to_legacy(n)));
    }
}

/// How a chest pairs with its neighbour: the offset of the partner on Java's `type` and facing.
fn pair_offset(facing: &str, kind: &str) -> Option<(i32, i32)> {
    let left = match kind {
        "left" => true,
        "right" => false,
        _ => return None,
    };
    let (dx, dz) = match facing {
        "north" => (1, 0),
        "east" => (0, 1),
        "south" => (-1, 0),
        "west" => (0, -1),
        _ => return None,
    };
    Some(if left { (dx, dz) } else { (-dx, -dz) })
}

fn facing_index(facing: &str) -> i8 {
    ["down", "up", "north", "south", "west", "east"].iter().position(|f| *f == facing).unwrap_or(1) as i8
}

fn set_position(from: &Compound, to: &mut Compound) {
    for k in ["x", "y", "z"] {
        if let Some(v) = int(from, k) {
            to.insert(k.into(), Tag::Int(v as i32));
        }
    }
}

fn short(c: &Compound, key: &str) -> i16 {
    int(c, key).unwrap_or(0) as i16
}

/// A Java block entity as a Bedrock one. `None` for kinds not handled here.
pub fn java_to_bedrock(java: &Compound, block: Option<&BlockState>, notes: &mut Notes) -> Option<Compound> {
    let id = text(java, "id")?;
    if let Some(out) = crate::block_entities_more::java_to_bedrock(java, block, notes) {
        return Some(out);
    }
    let bedrock = bedrock_id(id)?;
    let mut out = Compound::new();
    out.insert("id".into(), Tag::String(bedrock.to_string()));
    set_position(java, &mut out);
    out.insert("isMovable".into(), Tag::Byte(1));
    match id {
        "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel" | "minecraft:ender_chest" | "minecraft:shulker_box" | "minecraft:dispenser" | "minecraft:dropper" => {
            out.insert("Findable".into(), Tag::Byte(0));
            if id != "minecraft:ender_chest" {
                out.insert("Items".into(), bedrock_items(java, notes));
                loot(java, &mut out);
                name_of(java, &mut out);
            }
            if id == "minecraft:shulker_box" {
                let facing = block.and_then(|b| b.properties.get("facing")).map(String::as_str).unwrap_or("up");
                out.insert("facing".into(), Tag::Byte(facing_index(facing)));
            }
            if matches!(id, "minecraft:chest" | "minecraft:trapped_chest") {
                if let Some(b) = block {
                    let facing = b.properties.get("facing").map(String::as_str).unwrap_or("north");
                    let kind = b.properties.get("type").map(String::as_str).unwrap_or("single");
                    if let Some((dx, dz)) = pair_offset(facing, kind) {
                        let (x, z) = (int(java, "x").unwrap_or(0) as i32, int(java, "z").unwrap_or(0) as i32);
                        out.insert("pairx".into(), Tag::Int(x + dx));
                        out.insert("pairz".into(), Tag::Int(z + dz));
                        // The server makes whichever half was placed first the lead; any one half of
                        // the pair will do, so the right half leads.
                        out.insert("pairlead".into(), Tag::Byte(u8::from(kind == "right") as i8));
                    }
                }
            }
        }
        "minecraft:hopper" => {
            out.insert("Items".into(), bedrock_items(java, notes));
            out.insert("TransferCooldown".into(), Tag::Int(int(java, "TransferCooldown").unwrap_or(0) as i32));
            name_of(java, &mut out);
        }
        "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => {
            out.insert("Items".into(), bedrock_items(java, notes));
            out.insert("BurnTime".into(), Tag::Short(short(java, "lit_time_remaining")));
            out.insert("BurnDuration".into(), Tag::Short(short(java, "lit_total_time")));
            out.insert("CookTime".into(), Tag::Short(short(java, "cooking_time_spent")));
            out.insert("StoredXPInt".into(), Tag::Int(0));
            name_of(java, &mut out);
        }
        "minecraft:campfire" => {
            let times = |key: &str| -> Vec<i64> {
                match java.get(key) {
                    Some(Tag::IntArray(a)) => a.iter().map(|v| *v as i64).collect(),
                    _ => Vec::new(),
                }
            };
            let cooking = times("CookingTimes");
            for item in items_of(java, "Items") {
                let slot = int(item, "Slot").unwrap_or(0).clamp(0, 3) as usize;
                if let Some(b) = item_codec::java_to_bedrock(item, None, notes) {
                    out.insert(format!("Item{}", slot + 1), Tag::Compound(b));
                    out.insert(format!("ItemTime{}", slot + 1), Tag::Int(cooking.get(slot).copied().unwrap_or(0) as i32));
                }
            }
            for n in 1..=4 {
                out.entry(format!("ItemTime{n}")).or_insert(Tag::Int(0));
            }
        }
        "minecraft:comparator" => {
            out.insert("OutputSignal".into(), Tag::Int(int(java, "OutputSignal").unwrap_or(0) as i32));
        }
        "minecraft:mob_spawner" => {
            let entity = java
                .get("SpawnData")
                .and_then(|d| d.get("entity"))
                .and_then(|e| e.get("id"))
                .and_then(Tag::as_str)
                .and_then(crate::entity::bedrock_entity_id)
                .unwrap_or_default();
            out.insert("EntityIdentifier".into(), Tag::String(entity));
            for (java_key, bedrock_key, default) in [
                ("Delay", "Delay", 20),
                ("MinSpawnDelay", "MinSpawnDelay", 200),
                ("MaxSpawnDelay", "MaxSpawnDelay", 800),
                ("SpawnCount", "SpawnCount", 4),
                ("MaxNearbyEntities", "MaxNearbyEntities", 6),
                ("RequiredPlayerRange", "RequiredPlayerRange", 16),
                ("SpawnRange", "SpawnRange", 4),
            ] {
                out.insert(bedrock_key.into(), Tag::Short(int(java, java_key).unwrap_or(default) as i16));
            }
            out.insert("DisplayEntityWidth".into(), Tag::Float(0.8));
            out.insert("DisplayEntityHeight".into(), Tag::Float(1.8));
            out.insert("DisplayEntityScale".into(), Tag::Float(1.0));
        }
        "minecraft:sign" | "minecraft:hanging_sign" => {
            out.insert("FrontText".into(), Tag::Compound(sign_side(java.get("front_text").and_then(Tag::as_compound))));
            out.insert("BackText".into(), Tag::Compound(sign_side(java.get("back_text").and_then(Tag::as_compound))));
            out.insert("IsWaxed".into(), Tag::Byte(int(java, "is_waxed").unwrap_or(0) as i8));
        }
        _ => return None,
    }
    Some(out)
}

/// Java's sign text colours (the dye `textColor` values) as Bedrock writes them.
const SIGN_COLORS: [(&str, u32); 16] = [
    ("white", 0xFFFFFF),
    ("orange", 0xFF681F),
    ("magenta", 0xFF00FF),
    ("light_blue", 0x9AC0CD),
    ("yellow", 0xFFFF00),
    ("lime", 0xBFFF00),
    ("pink", 0xFF69B4),
    ("gray", 0x808080),
    ("light_gray", 0xD3D3D3),
    ("cyan", 0x00FFFF),
    ("purple", 0xA020F0),
    ("blue", 0x0000FF),
    ("brown", 0x8B4513),
    ("green", 0x00FF00),
    ("red", 0xFF0000),
    ("black", 0x000000),
];

fn sign_side(java: Option<&Compound>) -> Compound {
    let mut out = Compound::new();
    let mut lines: Vec<String> = Vec::new();
    if let Some(side) = java {
        lines = side.get("messages").and_then(Tag::as_list).map(|l| l.iter().map(text_codec::to_legacy).collect()).unwrap_or_default();
    }
    while lines.len() < 4 {
        lines.push(String::new());
    }
    let color = java.and_then(|s| text(s, "color")).and_then(|c| SIGN_COLORS.iter().find(|(n, _)| *n == c)).map_or(0, |(_, v)| *v);
    let glowing = java.and_then(|s| int(s, "has_glowing_text")).unwrap_or(0);
    out.insert("FilteredText".into(), Tag::String(String::new()));
    out.insert("HideGlowOutline".into(), Tag::Byte(0));
    out.insert("IgnoreLighting".into(), Tag::Byte(glowing as i8));
    out.insert("PersistFormatting".into(), Tag::Byte(1));
    out.insert("SignTextColor".into(), Tag::Int((color | 0xFF00_0000) as i32));
    // Every line ends with a newline; a sign with no text has an empty string.
    let body = if lines.iter().all(String::is_empty) { String::new() } else { lines.iter().map(|l| format!("{l}\n")).collect() };
    out.insert("Text".into(), Tag::String(body));
    out.insert("TextOwner".into(), Tag::String(String::new()));
    out
}

// ---------------------------------------------------------------------------------------------
// Bedrock -> Java

fn java_items(c: &Compound, notes: &mut Notes) -> Tag {
    let mut out = Vec::new();
    for item in items_of(c, "Items") {
        let slot = int(item, "Slot").map(|s| s as u8);
        if let Some(j) = item_codec::bedrock_to_java(item, slot, notes) {
            out.push(j);
        }
    }
    compounds(out)
}

fn java_loot(bedrock: &Compound, out: &mut Compound) {
    if let Some(t) = text(bedrock, "LootTable").filter(|t| !t.is_empty()) {
        let path = t.trim_start_matches("loot_tables/").trim_end_matches(".json");
        out.insert("LootTable".into(), Tag::String(format!("minecraft:{path}")));
        out.insert("LootTableSeed".into(), Tag::Long(int(bedrock, "LootTableSeed").unwrap_or(0)));
    }
}

/// A Bedrock block entity as a Java one. `block` is the Java block it sits in. `None` for kinds
/// not handled here.
pub fn bedrock_to_java(bedrock: &Compound, block: Option<&BlockState>, notes: &mut Notes) -> Option<ToJava> {
    let id = text(bedrock, "id")?;
    if let Some(j) = crate::block_entities_more::bedrock_to_java(bedrock, block, notes) {
        return Some(j);
    }
    let java_id = java_id_of(id, block)?;
    let mut out = Compound::new();
    out.insert("id".into(), Tag::String(java_id.to_string()));
    set_position(bedrock, &mut out);
    out.insert("keepPacked".into(), Tag::Byte(0));
    let mut edit = BlockEdit::default();
    match id {
        "Chest" | "Barrel" | "EnderChest" | "ShulkerBox" | "Dispenser" | "Dropper" => {
            if id != "EnderChest" {
                out.insert("Items".into(), java_items(bedrock, notes));
                java_loot(bedrock, &mut out);
                if let Some(n) = text(bedrock, "CustomName").filter(|n| !n.is_empty()) {
                    out.insert("CustomName".into(), text_codec::from_legacy(n));
                }
            }
            if id == "Chest" {
                if let (Some(px), Some(pz)) = (int(bedrock, "pairx"), int(bedrock, "pairz")) {
                    let (x, z) = (int(bedrock, "x").unwrap_or(0), int(bedrock, "z").unwrap_or(0));
                    let facing = block.and_then(|b| b.properties.get("facing")).map(String::as_str).unwrap_or("north");
                    let (dx, dz) = ((px - x).signum(), (pz - z).signum());
                    let left = match facing {
                        "north" => dx > 0,
                        "east" => dz > 0,
                        "south" => dx < 0,
                        "west" => dz < 0,
                        _ => false,
                    };
                    if dx != 0 || dz != 0 {
                        edit.props.push(("type", Some(if left { "left" } else { "right" }.to_string())));
                    }
                }
            }
        }
        "Hopper" => {
            out.insert("Items".into(), java_items(bedrock, notes));
            out.insert("TransferCooldown".into(), Tag::Int(int(bedrock, "TransferCooldown").unwrap_or(0) as i32));
        }
        "Furnace" | "BlastFurnace" | "Smoker" => {
            out.insert("Items".into(), java_items(bedrock, notes));
            out.insert("lit_time_remaining".into(), Tag::Int(int(bedrock, "BurnTime").unwrap_or(0) as i32));
            out.insert("lit_total_time".into(), Tag::Int(int(bedrock, "BurnDuration").unwrap_or(0) as i32));
            out.insert("cooking_time_spent".into(), Tag::Int(int(bedrock, "CookTime").unwrap_or(0) as i32));
            out.insert("cooking_total_time".into(), Tag::Int(if id == "Furnace" { 200 } else { 100 }));
            out.insert("RecipesUsed".into(), Tag::Compound(Compound::new()));
        }
        "Campfire" => {
            let mut items = Vec::new();
            let (mut cooking, mut total) = (vec![0i32; 4], vec![0i32; 4]);
            for n in 1..=4usize {
                if let Some(stack) = bedrock.get(&format!("Item{n}")).and_then(Tag::as_compound) {
                    if let Some(mut j) = item_codec::bedrock_to_java(stack, Some((n - 1) as u8), notes) {
                        j.insert("Slot".into(), Tag::Byte((n - 1) as i8));
                        items.push(j);
                        cooking[n - 1] = int(bedrock, &format!("ItemTime{n}")).unwrap_or(0) as i32;
                        total[n - 1] = 600;
                    }
                }
            }
            out.insert("Items".into(), compounds(items));
            out.insert("CookingTimes".into(), Tag::IntArray(cooking));
            out.insert("CookingTotalTimes".into(), Tag::IntArray(total));
        }
        "Comparator" => {
            out.insert("OutputSignal".into(), Tag::Int(int(bedrock, "OutputSignal").unwrap_or(0) as i32));
        }
        "MobSpawner" => {
            let entity = text(bedrock, "EntityIdentifier").and_then(crate::entity::java_entity_id_of).unwrap_or_default();
            if !entity.is_empty() {
                let mut e = Compound::new();
                e.insert("id".into(), Tag::String(entity));
                let mut data = Compound::new();
                data.insert("entity".into(), Tag::Compound(e));
                out.insert("SpawnData".into(), Tag::Compound(data));
            }
            for key in ["Delay", "MinSpawnDelay", "MaxSpawnDelay", "SpawnCount", "MaxNearbyEntities", "RequiredPlayerRange", "SpawnRange"] {
                if let Some(v) = int(bedrock, key) {
                    out.insert(key.into(), Tag::Short(v as i16));
                }
            }
        }
        "Sign" | "HangingSign" => {
            out.insert("front_text".into(), Tag::Compound(java_sign_side(bedrock.get("FrontText").and_then(Tag::as_compound))));
            out.insert("back_text".into(), Tag::Compound(java_sign_side(bedrock.get("BackText").and_then(Tag::as_compound))));
            out.insert("is_waxed".into(), Tag::Byte(int(bedrock, "IsWaxed").unwrap_or(0) as i8));
        }
        _ => return None,
    }
    Some(ToJava { nbt: Some(out), edit })
}

fn java_sign_side(bedrock: Option<&Compound>) -> Compound {
    let mut out = Compound::new();
    let body = bedrock.and_then(|s| text(s, "Text")).unwrap_or("");
    let mut lines: Vec<Tag> = body.strip_suffix('\n').unwrap_or(body).split('\n').take(4).map(text_codec::from_legacy).collect();
    while lines.len() < 4 {
        lines.push(Tag::String(String::new()));
    }
    let tag_type = lines.iter().map(Tag::tag_type).find(|t| *t != TagType::String).unwrap_or(TagType::String);
    // A list holds one type: when any line is styled they all become compounds.
    let lines: Vec<Tag> = if tag_type == TagType::String {
        lines
    } else {
        lines
            .into_iter()
            .map(|l| match l {
                Tag::String(s) => {
                    let mut c = Compound::new();
                    c.insert("text".into(), Tag::String(s));
                    Tag::Compound(c)
                }
                other => other,
            })
            .collect()
    };
    out.insert("messages".into(), Tag::List(List::from_tags(lines[0].tag_type(), lines).expect("uniform")));
    let rgb = bedrock.and_then(|s| int(s, "SignTextColor")).map_or(0, |c| c as u32 & 0x00FF_FFFF);
    let color = SIGN_COLORS.iter().find(|(_, v)| *v == rgb).map_or("black", |(n, _)| *n);
    out.insert("color".into(), Tag::String(color.to_string()));
    out.insert("has_glowing_text".into(), Tag::Byte(bedrock.and_then(|s| int(s, "IgnoreLighting")).unwrap_or(0) as i8));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Tag {
        Tag::String(v.to_string())
    }

    fn chest(x: i32, z: i32) -> Compound {
        let mut c = Compound::new();
        c.insert("id".into(), s("minecraft:chest"));
        for (k, v) in [("x", x), ("y", -60), ("z", z)] {
            c.insert(k.into(), Tag::Int(v));
        }
        let mut item = Compound::new();
        item.insert("Slot".into(), Tag::Byte(13));
        item.insert("id".into(), s("minecraft:golden_apple"));
        item.insert("count".into(), Tag::Int(1));
        c.insert("Items".into(), compounds(vec![item]));
        c
    }

    fn state(facing: &str, kind: &str) -> BlockState {
        BlockState::new("minecraft:chest").with("facing", facing).with("type", kind)
    }

    #[test]
    fn double_chests_pair_the_way_the_bedrock_server_pairs_them() {
        // Observed on the server: chests facing north at x 2 and 3 point at each other.
        let mut notes = Notes::default();
        let left = java_to_bedrock(&chest(2, 2), Some(&state("north", "left")), &mut notes).unwrap();
        assert_eq!((left.get("pairx"), left.get("pairz"), left.get("pairlead")), (Some(&Tag::Int(3)), Some(&Tag::Int(2)), Some(&Tag::Byte(0))));
        let right = java_to_bedrock(&chest(3, 2), Some(&state("north", "right")), &mut notes).unwrap();
        assert_eq!((right.get("pairx"), right.get("pairlead")), (Some(&Tag::Int(2)), Some(&Tag::Byte(1))));
        // Facing south the half at the lower x is the right one, and still leads.
        let south = java_to_bedrock(&chest(10, 2), Some(&state("south", "right")), &mut notes).unwrap();
        assert_eq!((south.get("pairx"), south.get("pairlead")), (Some(&Tag::Int(11)), Some(&Tag::Byte(1))));
        // Back to Java: the type comes from where the partner is.
        let java = bedrock_to_java(&left, Some(&state("north", "single")), &mut notes).unwrap();
        assert_eq!(java.edit.props, vec![("type", Some("left".to_string()))]);
        let java = bedrock_to_java(&south, Some(&state("south", "single")), &mut notes).unwrap();
        assert_eq!(java.edit.props, vec![("type", Some("right".to_string()))]);
        let held = java.nbt.unwrap();
        assert_eq!(held.get("Items").and_then(Tag::as_list).unwrap().get(0).unwrap().get("id"), Some(&s("minecraft:golden_apple")));
    }

    #[test]
    fn signs_keep_their_lines_and_colour() {
        let mut side = Compound::new();
        let msgs = vec![s("Hello"), s(""), s("\u{a7}cRed"), s("")];
        side.insert("messages".into(), Tag::List(List::from_tags(TagType::String, msgs).unwrap()));
        side.insert("color".into(), s("red"));
        let mut sign = Compound::new();
        sign.insert("id".into(), s("minecraft:sign"));
        sign.insert("front_text".into(), Tag::Compound(side));
        let mut notes = Notes::default();
        let b = java_to_bedrock(&sign, None, &mut notes).unwrap();
        let front = b.get("FrontText").and_then(Tag::as_compound).unwrap();
        assert_eq!(front.get("Text"), Some(&s("Hello\n\n\u{a7}cRed\n\n")));
        assert_eq!(front.get("SignTextColor"), Some(&Tag::Int(0xFFFF0000u32 as i32)));
        let j = bedrock_to_java(&b, None, &mut notes).unwrap().nbt.unwrap();
        let back = j.get("front_text").and_then(Tag::as_compound).unwrap();
        assert_eq!(back.get("color"), Some(&s("red")));
        assert_eq!(back.get("messages").and_then(Tag::as_list).unwrap().len(), 4);
    }

    #[test]
    fn furnaces_comparators_and_campfires() {
        let mut furnace = Compound::new();
        furnace.insert("id".into(), s("minecraft:furnace"));
        furnace.insert("lit_time_remaining".into(), Tag::Int(120));
        furnace.insert("cooking_time_spent".into(), Tag::Int(30));
        let mut notes = Notes::default();
        let b = java_to_bedrock(&furnace, None, &mut notes).unwrap();
        assert_eq!(b.get("BurnTime"), Some(&Tag::Short(120)));
        let j = bedrock_to_java(&b, None, &mut notes).unwrap().nbt.unwrap();
        assert_eq!(j.get("cooking_time_spent"), Some(&Tag::Int(30)));
        assert!(java_to_bedrock(&{
            let mut c = Compound::new();
            c.insert("id".into(), s("minecraft:not_a_block_entity"));
            c
        }, None, &mut notes).is_none());
    }
}
