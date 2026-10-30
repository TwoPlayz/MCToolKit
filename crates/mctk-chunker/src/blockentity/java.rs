//! Java 26.x block entities (`Java*BlockEntityHandler`).

use mctk_nbt::{Compound, Tag};

use super::*;
use crate::ids::DYES;
use crate::{Notes, child, compound, compound_list, int, list, string, text};

/// Sign colour names ↔ Bedrock ARGB (Chunker's `COLOR_TO_BEDROCK_HEX`: black is plain black).
pub(crate) fn sign_argb(name: &str) -> i32 {
    if name == "black" {
        return 0xFF00_0000u32 as i32;
    }
    let rgb = DYES.iter().find(|(n, _, _)| *n == name).map_or(0, |(_, b, _)| *b);
    (0xFF00_0000 | rgb) as i32
}

pub(crate) fn sign_color_name(argb: i32) -> &'static str {
    let c = argb as u32 & 0xFF_FFFF;
    if c == 0 {
        return "black";
    }
    let dist = |rgb: u32| {
        let d = |s: u32| (((rgb >> s) & 255) as i64 - ((c >> s) & 255) as i64).pow(2);
        d(16) + d(8) + d(0)
    };
    DYES.iter()
        .filter(|(n, _, _)| *n != "black")
        .min_by_key(|(_, b, _)| dist(*b))
        .filter(|(_, b, _)| dist(*b) < dist(0))
        .map_or("black", |(n, _, _)| n)
}

fn read_items(be: &Compound, notes: &mut Notes) -> std::collections::BTreeMap<u8, ItemStack> {
    let mut items = std::collections::BTreeMap::new();
    for (index, tag) in list(be, "Items").enumerate() {
        let Some(c) = tag.as_compound() else { continue };
        let slot = int(c, "Slot").unwrap_or(index as i64) as u8;
        if let Some(item) = crate::item::java::read(c, notes) {
            items.insert(slot, item);
        }
    }
    items
}

fn read_sign_face(face: &Compound) -> SignFace {
    let mut lines: Vec<Text> = Vec::new();
    if let Some(Tag::List(messages)) = face.get("messages") {
        lines.extend(messages.iter().take(4).map(text::from_nbt));
    } else {
        // Before 1.20 each line was its own tag.
        for i in 1..=4 {
            lines.push(face.get(&format!("Text{i}")).map_or_else(|| text::plain(""), text::from_nbt));
        }
    }
    lines.resize(4, text::plain(""));
    let lit = int(face, "has_glowing_text").or_else(|| int(face, "GlowingText")).unwrap_or(0) != 0;
    let color = string(face, "color").or_else(|| string(face, "Color")).unwrap_or("black");
    SignFace {
        lines,
        lit,
        color: sign_argb(color),
    }
}

fn write_sign_face(face: &SignFace) -> Compound {
    let mut lines = face.lines.clone();
    lines.resize(4, text::plain(""));
    let mut c = Compound::new();
    c.insert("messages".into(), text::to_nbt(&serde_json::Value::Array(lines)));
    c.insert("has_glowing_text".into(), Tag::Byte(face.lit as i8));
    c.insert("color".into(), Tag::String(sign_color_name(face.color).into()));
    c
}

/// Reads a Java block entity; `None` when the type is not handled.
pub fn read(be: &Compound, notes: &mut Notes) -> Option<BlockEntity> {
    let id = string(be, "id")?;
    let kind = if let Some(ck) = ContainerKind::from_java(id) {
        let mut c = Container::new(ck);
        c.items = read_items(be, notes);
        if ck.randomizable() {
            c.loot_table = string(be, "LootTable").filter(|t| !t.is_empty()).map(str::to_string);
        }
        if ck.is_furnace() {
            let get = |new: &str, old: &str| {
                int(be, new).or_else(|| int(be, old)).unwrap_or(0) as i16
            };
            c.furnace = Furnace {
                burn_time: get("lit_time_remaining", "BurnTime"),
                cook_time: get("cooking_time_spent", "CookTime"),
                cook_time_total: get("lit_total_time", "CookTimeTotal"),
            };
        }
        if ck == ContainerKind::BrewingStand {
            let short = |k: &str, d: i64| int(be, k).unwrap_or(d) as i16;
            c.brewing = Brewing {
                brew_time: short("BrewTime", 0),
                fuel: short("Fuel", 0),
                fuel_total: short("total_fuel", 20),
            };
        }
        if ck == ContainerKind::Crafter {
            let bits = match be.get("disabled_slots") {
                Some(Tag::IntArray(a)) => a.iter().filter(|i| (0..16).contains(*i)).fold(0u16, |m, i| m | 1 << *i),
                _ => 0,
            };
            c.crafter = Crafter {
                disabled_slots: bits,
                crafting_ticks_remaining: int(be, "crafting_ticks_remaining").unwrap_or(0) as i32,
            };
        }
        Kind::Container(c)
    } else if let Some(ek) = EmptyKind::from_java(id) {
        Kind::Empty(ek)
    } else {
        match id {
            "minecraft:sign" | "minecraft:hanging_sign" => {
                let mut s = Sign::new(id == "minecraft:hanging_sign");
                s.waxed = int(be, "is_waxed").unwrap_or(0) != 0;
                s.allow_op_features = int(be, "allow_op_features").unwrap_or(1) != 0;
                s.front = read_sign_face(compound(be, "front_text").unwrap_or(be));
                if let Some(back) = compound(be, "back_text") {
                    s.back = read_sign_face(back);
                }
                Kind::Sign(s)
            }
            "minecraft:mob_spawner" => {
                let d = Spawner::default();
                let short = |k: &str, default: i16| int(be, k).map_or(default, |v| v as i16);
                let spawn_data = compound(be, "SpawnData");
                let entity = spawn_data.and_then(|d| {
                    string(d, "id").or_else(|| compound(d, "entity").and_then(|e| string(e, "id")))
                });
                Kind::Spawner(Spawner {
                    delay: short("Delay", d.delay),
                    spawn_range: short("SpawnRange", d.spawn_range),
                    max_spawn_delay: short("MaxSpawnDelay", d.max_spawn_delay),
                    min_spawn_delay: short("MinSpawnDelay", d.min_spawn_delay),
                    spawn_count: short("SpawnCount", d.spawn_count),
                    required_player_range: short("RequiredPlayerRange", d.required_player_range),
                    max_nearby_entities: short("MaxNearbyEntities", d.max_nearby_entities),
                    entity: entity.map(str::to_string),
                })
            }
            "minecraft:comparator" => Kind::Comparator {
                output_signal: int(be, "OutputSignal").unwrap_or(0) as i32,
            },
            "minecraft:sculk_sensor" => Kind::SculkSensor { calibrated: false },
            "minecraft:calibrated_sculk_sensor" => Kind::SculkSensor { calibrated: true },
            "minecraft:campfire" => {
                let mut f = Campfire::default();
                let times: Vec<i32> = match be.get("CookingTimes") {
                    Some(Tag::IntArray(a)) => a.clone(),
                    _ => Vec::new(),
                };
                for (index, tag) in list(be, "Items").enumerate() {
                    let Some(c) = tag.as_compound() else { continue };
                    let slot = int(c, "Slot").unwrap_or(index as i64);
                    if !(0..4).contains(&slot) {
                        continue;
                    }
                    if let Some(item) = crate::item::java::read(c, notes) {
                        f.items[slot as usize] = Some(item);
                        f.cooking_times[slot as usize] = times.get(slot as usize).copied().unwrap_or(0);
                    }
                }
                Kind::Campfire(f)
            }
            "minecraft:decorated_pot" => {
                let mut pot = DecoratedPot::default();
                let sherds = if let Some(m) = compound(be, "sherds").or_else(|| compound(be, "shards")) {
                    // 26.3 stores the sides by name.
                    ["back", "left", "right", "front"]
                        .iter()
                        .map(|side| {
                            compound(m, side)
                                .and_then(|s| string(s, "id"))
                                .unwrap_or("minecraft:brick")
                                .to_string()
                        })
                        .collect::<Vec<_>>()
                } else {
                    list(be, "sherds").chain(list(be, "shards")).filter_map(Tag::as_str).map(str::to_string).collect()
                };
                for (slot, s) in pot.sherds.iter_mut().zip(sherds) {
                    *slot = s;
                }
                pot.item = compound(be, "item").and_then(|i| crate::item::java::read(i, notes));
                Kind::DecoratedPot(pot)
            }
            "minecraft:brushable_block" => Kind::Brushable(Brushable {
                brush_direction: int(be, "hit_direction").unwrap_or(6) as i8,
                item: compound(be, "item").and_then(|i| crate::item::java::read(i, notes)),
                loot_table: string(be, "LootTable").filter(|t| !t.is_empty()).map(str::to_string),
                ..Brushable::default()
            }),
            "minecraft:lectern" => Kind::Lectern {
                book: compound(be, "Book").and_then(|b| crate::item::java::read(b, notes)),
                page: int(be, "Page").unwrap_or(0) as i32,
            },
            "minecraft:jukebox" => Kind::Jukebox {
                record: compound(be, "RecordItem").and_then(|r| crate::item::java::read(r, notes)),
            },
            "minecraft:skull" => Kind::Skull(read_skull(be)),
            "minecraft:banner" => {
                let dye = |c: &Compound| -> Option<u8> {
                    match c.get("Color").or_else(|| c.get("color")) {
                        Some(Tag::String(n)) => crate::ids::DYES.iter().position(|(d, _, _)| *d == n.trim_start_matches("minecraft:")).map(|i| i as u8),
                        Some(t) => t.as_i64().map(|v| v as u8),
                        None => None,
                    }
                };
                let mut b = Banner {
                    base: int(be, "Base").map(|v| v as u8),
                    patterns: Vec::new(),
                };
                for p in list(be, "patterns").chain(list(be, "Patterns")).filter_map(Tag::as_compound) {
                    let pattern = string(p, "pattern")
                        .and_then(crate::ids::banner_pattern_from_java)
                        .or_else(|| string(p, "Pattern").and_then(crate::ids::banner_pattern_from_bedrock));
                    if let (Some(c), Some(pat)) = (dye(p), pattern) {
                        b.patterns.push((c, pat));
                    }
                }
                Kind::Banner(b)
            }
            "minecraft:beacon" => {
                let effect = |new: &str, old: &str| -> Option<crate::ids::Effect> {
                    match (string(be, new), int(be, old)) {
                        (Some(n), _) => crate::ids::Effect::from_java(n),
                        (None, Some(id)) if id > 0 => crate::ids::Effect::from_bedrock(id as i32),
                        _ => None,
                    }
                };
                Kind::Beacon {
                    primary: effect("primary_effect", "Primary"),
                    secondary: effect("secondary_effect", "Secondary"),
                }
            }
            "minecraft:command_block" => Kind::CommandBlock(CommandBlock {
                command: string(be, "Command").unwrap_or_default().to_string(),
                auto: int(be, "auto").unwrap_or(0) != 0,
                track_output: int(be, "TrackOutput").unwrap_or(1) != 0,
            }),
            "minecraft:end_gateway" => Kind::EndGateway {
                age: int(be, "Age").unwrap_or(0),
                exit: match be.get("exit_portal") {
                    Some(Tag::IntArray(a)) if a.len() == 3 => Some([a[0], a[1], a[2]]),
                    _ => None,
                },
            },
            "minecraft:chiseled_bookshelf" => {
                let mut books: [Option<ItemStack>; 6] = Default::default();
                for (index, tag) in list(be, "Items").enumerate() {
                    let Some(c) = tag.as_compound() else { continue };
                    let slot = int(c, "Slot").unwrap_or(index as i64);
                    if (0..6).contains(&slot) {
                        books[slot as usize] = crate::item::java::read(c, notes);
                    }
                }
                Kind::ChiseledBookshelf { books }
            }
            "minecraft:shelf" => {
                let mut items: [Option<ItemStack>; 3] = Default::default();
                for (index, tag) in list(be, "Items").enumerate() {
                    let Some(c) = tag.as_compound() else { continue };
                    let slot = int(c, "Slot").unwrap_or(index as i64);
                    if (0..3).contains(&slot) {
                        items[slot as usize] = crate::item::java::read(c, notes);
                    }
                }
                Kind::Shelf { items }
            }
            "minecraft:jigsaw" => {
                let d = Jigsaw::default();
                let text = |k: &str, default: &str| string(be, k).unwrap_or(default).to_string();
                Kind::Jigsaw(Jigsaw {
                    name: text("name", &d.name),
                    target: text("target", &d.target),
                    pool: text("pool", &d.pool),
                    final_state: text("final_state", &d.final_state),
                    joint: text("joint", "rollable"),
                    selection_priority: int(be, "selection_priority").unwrap_or(0) as i32,
                    placement_priority: int(be, "placement_priority").unwrap_or(0) as i32,
                })
            }
            "minecraft:structure_block" => {
                let flag = |k: &str, d: i64| int(be, k).unwrap_or(d) != 0;
                let num = |k: &str| int(be, k).unwrap_or(0) as i32;
                Kind::Structure(Box::new(StructureBlock {
                    name: string(be, "name").unwrap_or_default().to_string(),
                    author: string(be, "author").unwrap_or_default().to_string(),
                    metadata: string(be, "metadata").unwrap_or_default().to_string(),
                    pos: [num("posX"), num("posY"), num("posZ")],
                    size: [num("sizeX"), num("sizeY"), num("sizeZ")],
                    rotation: match string(be, "rotation").unwrap_or("NONE").to_uppercase().as_str() {
                        "CLOCKWISE_90" => 1,
                        "CLOCKWISE_180" => 2,
                        "COUNTERCLOCKWISE_90" => 3,
                        _ => 0,
                    },
                    mirror: match string(be, "mirror").unwrap_or("NONE").to_uppercase().as_str() {
                        "LEFT_RIGHT" => 1,
                        "FRONT_BACK" => 2,
                        _ => 0,
                    },
                    mode: match string(be, "mode").unwrap_or("DATA").to_uppercase().as_str() {
                        "SAVE" => StructureMode::Save,
                        "LOAD" => StructureMode::Load,
                        "CORNER" => StructureMode::Corner,
                        _ => StructureMode::Data,
                    },
                    ignore_entities: flag("ignoreEntities", 1),
                    strict: flag("strict", 0),
                    powered: flag("powered", 0),
                    show_air: flag("showair", 0),
                    show_bounding_box: flag("showboundingbox", 1),
                    integrity: be.get("integrity").and_then(Tag::as_f64).unwrap_or(1.0) as f32,
                    seed: int(be, "seed").unwrap_or(0),
                    ..StructureBlock::default()
                }))
            }
            // Bedrock has no use for Java's moving piston.
            "minecraft:piston" => Kind::Discard,
            "minecraft:copper_golem_statue" => Kind::CopperGolemStatue { pose: 0 },
            _ => return None,
        }
    };
    let mut out = BlockEntity::new(
        int(be, "x").unwrap_or(0) as i32,
        int(be, "y").unwrap_or(0) as i32,
        int(be, "z").unwrap_or(0) as i32,
        kind,
    );
    if matches!(out.kind, Kind::Container(_) | Kind::Banner(_) | Kind::CommandBlock(_)) {
        out.custom_name = be.get("CustomName").map(text::from_nbt);
    }
    Some(out)
}

fn java_id(kind: &Kind) -> Option<&'static str> {
    Some(match kind {
        Kind::Container(c) => c.kind.java_id(),
        Kind::Sign(s) if s.hanging => "minecraft:hanging_sign",
        Kind::Sign(_) => "minecraft:sign",
        Kind::Spawner(_) => "minecraft:mob_spawner",
        Kind::Comparator { .. } => "minecraft:comparator",
        Kind::SculkSensor { calibrated: false } => "minecraft:sculk_sensor",
        Kind::SculkSensor { calibrated: true } => "minecraft:calibrated_sculk_sensor",
        Kind::Campfire(_) => "minecraft:campfire",
        Kind::DecoratedPot(_) => "minecraft:decorated_pot",
        Kind::Brushable(_) => "minecraft:brushable_block",
        Kind::Lectern { .. } => "minecraft:lectern",
        Kind::Jukebox { .. } => "minecraft:jukebox",
        Kind::Skull(_) => "minecraft:skull",
        Kind::Banner(_) => "minecraft:banner",
        Kind::Beacon { .. } => "minecraft:beacon",
        Kind::CommandBlock(_) => "minecraft:command_block",
        Kind::EndGateway { .. } => "minecraft:end_gateway",
        Kind::ChiseledBookshelf { .. } => "minecraft:chiseled_bookshelf",
        Kind::Shelf { .. } => "minecraft:shelf",
        Kind::Jigsaw(_) => "minecraft:jigsaw",
        Kind::Structure(_) => "minecraft:structure_block",
        Kind::CopperGolemStatue { .. } => "minecraft:copper_golem_statue",
        Kind::Empty(e) => e.java_id(),
        // These live in the block on Java, or have no Java form.
        Kind::ItemFrame(_)
        | Kind::Bed { .. }
        | Kind::Cauldron(_)
        | Kind::FlowerPot { .. }
        | Kind::NoteBlock { .. }
        | Kind::PistonArm { .. }
        | Kind::BedrockOnly(_)
        | Kind::Discard => return None,
    })
}

/// Writes a block entity as Java 26.x NBT (including `id`, `x`, `y`, `z`). `None` for kinds
/// Java keeps entirely in the block (beds, cauldrons, flower pots).
pub fn write(be: &BlockEntity, notes: &mut Notes) -> Option<Compound> {
    let mut out = Compound::new();
    out.insert("id".into(), Tag::String(java_id(&be.kind)?.into()));
    out.insert("x".into(), Tag::Int(be.x));
    out.insert("y".into(), Tag::Int(be.y));
    out.insert("z".into(), Tag::Int(be.z));
    match &be.kind {
        Kind::Container(c) => {
            let items = c
                .items
                .iter()
                .filter(|(_, i)| !i.is_air())
                .map(|(slot, i)| crate::item::java::write(i, Some(*slot)))
                .collect();
            out.insert("Items".into(), compound_list(items));
            if let Some(n) = &be.custom_name {
                out.insert("CustomName".into(), text::to_nbt(n));
            }
            if let Some(t) = c.loot_table.as_deref().filter(|_| c.kind.randomizable()) {
                match loot_to_java(t) {
                    Some(t) => {
                        out.insert("LootTable".into(), Tag::String(t));
                    }
                    None => notes.miss(format!("loot table {t}")),
                }
            }
            if c.kind.is_furnace() {
                out.insert("lit_time_remaining".into(), Tag::Int(c.furnace.burn_time as i32));
                out.insert("cooking_time_spent".into(), Tag::Int(c.furnace.cook_time as i32));
                out.insert("lit_total_time".into(), Tag::Int(c.furnace.cook_time_total as i32));
            }
            if c.kind == ContainerKind::BrewingStand {
                out.insert("BrewTime".into(), Tag::Int(c.brewing.brew_time as i32));
                out.insert("Fuel".into(), Tag::Int(c.brewing.fuel as i32));
                out.insert("total_fuel".into(), Tag::Int(c.brewing.fuel_total as i32));
            }
            if c.kind == ContainerKind::Crafter {
                let slots: Vec<i32> = (0..16).filter(|i| c.crafter.disabled_slots & (1 << i) != 0).collect();
                out.insert("disabled_slots".into(), Tag::IntArray(slots));
                out.insert(
                    "crafting_ticks_remaining".into(),
                    Tag::Int(c.crafter.crafting_ticks_remaining),
                );
            }
        }
        Kind::Sign(s) => {
            out.insert("is_waxed".into(), Tag::Byte(s.waxed as i8));
            out.insert("allow_op_features".into(), Tag::Byte(s.allow_op_features as i8));
            out.insert("front_text".into(), Tag::Compound(write_sign_face(&s.front)));
            out.insert("back_text".into(), Tag::Compound(write_sign_face(&s.back)));
        }
        Kind::Spawner(s) => {
            for (k, v) in [
                ("Delay", s.delay),
                ("SpawnRange", s.spawn_range),
                ("MaxSpawnDelay", s.max_spawn_delay),
                ("MinSpawnDelay", s.min_spawn_delay),
                ("SpawnCount", s.spawn_count),
                ("RequiredPlayerRange", s.required_player_range),
                ("MaxNearbyEntities", s.max_nearby_entities),
            ] {
                out.insert(k.into(), Tag::Short(v));
            }
            if let Some(e) = &s.entity {
                child(child(&mut out, "SpawnData"), "entity").insert("id".into(), Tag::String(e.clone()));
            }
        }
        Kind::Comparator { output_signal } => {
            out.insert("OutputSignal".into(), Tag::Int(*output_signal));
        }
        Kind::SculkSensor { .. } => {
            let mut selector = Compound::new();
            selector.insert("tick".into(), Tag::Long(-1));
            let mut listener = Compound::new();
            listener.insert("event_delay".into(), Tag::Int(0));
            listener.insert("selector".into(), Tag::Compound(selector));
            out.insert("listener".into(), Tag::Compound(listener));
            out.insert("last_vibration_frequency".into(), Tag::Int(0));
        }
        Kind::Campfire(f) => {
            let mut times = vec![0i32; 4];
            let mut items = Vec::new();
            for (i, item) in f.items.iter().enumerate() {
                let Some(item) = item.as_ref().filter(|i| !i.is_air()) else { continue };
                items.push(crate::item::java::write(item, Some(i as u8)));
                times[i] = f.cooking_times[i];
            }
            out.insert("Items".into(), compound_list(items));
            out.insert("CookingTimes".into(), Tag::IntArray(times));
        }
        Kind::DecoratedPot(p) => {
            let mut sherds = Compound::new();
            for (side, id) in ["back", "left", "right", "front"].iter().zip(&p.sherds) {
                let mut s = Compound::new();
                s.insert("id".into(), Tag::String(id.clone()));
                sherds.insert((*side).into(), Tag::Compound(s));
            }
            out.insert("sherds".into(), Tag::Compound(sherds));
            if let Some(item) = p.item.as_ref().filter(|i| !i.is_air()) {
                out.insert("item".into(), Tag::Compound(crate::item::java::write(item, None)));
            }
        }
        Kind::Brushable(b) => {
            if b.brush_direction != 6 {
                out.insert("hit_direction".into(), Tag::Byte(b.brush_direction));
            }
            if let Some(item) = b.item.as_ref().filter(|i| !i.is_air()) {
                out.insert("item".into(), Tag::Compound(crate::item::java::write(item, None)));
            }
            if let Some(t) = b.loot_table.as_deref().filter(|t| !t.is_empty()) {
                out.insert("LootTable".into(), Tag::String(t.into()));
            }
        }
        Kind::Lectern { book, page } => {
            out.insert("Page".into(), Tag::Int(*page));
            if let Some(b) = book.as_ref().filter(|b| !b.is_air()) {
                out.insert("Book".into(), Tag::Compound(crate::item::java::write(b, None)));
            }
        }
        Kind::Jukebox { record } => {
            if let Some(r) = record.as_ref().filter(|r| !r.is_air()) {
                out.insert("RecordItem".into(), Tag::Compound(crate::item::java::write(r, None)));
            }
        }
        Kind::Skull(s) => write_skull(s, &mut out),
        Kind::Banner(b) => {
            if let Some(base) = b.base {
                out.insert("Base".into(), Tag::Int(base as i32));
            }
            let patterns = b
                .patterns
                .iter()
                .map(|(dye, pat)| {
                    let mut p = Compound::new();
                    p.insert("color".into(), Tag::String(crate::ids::DYES[(*dye as usize).min(15)].0.into()));
                    p.insert(
                        "pattern".into(),
                        Tag::String(format!("minecraft:{}", crate::ids::BANNER_PATTERNS[*pat].0)),
                    );
                    p
                })
                .collect();
            out.insert("patterns".into(), compound_list(patterns));
            if let Some(n) = &be.custom_name {
                out.insert("CustomName".into(), text::to_nbt(n));
            }
        }
        Kind::Beacon { primary, secondary } => {
            for (key, e) in [("primary_effect", primary), ("secondary_effect", secondary)] {
                if let Some(name) = e.and_then(crate::ids::Effect::java) {
                    out.insert(key.into(), Tag::String(name.into()));
                }
            }
            if let Some(n) = &be.custom_name {
                out.insert("CustomName".into(), text::to_nbt(n));
            }
        }
        Kind::CommandBlock(c) => {
            out.insert("Command".into(), Tag::String(c.command.clone()));
            out.insert("auto".into(), Tag::Byte(c.auto as i8));
            out.insert("TrackOutput".into(), Tag::Byte(c.track_output as i8));
            if let Some(n) = &be.custom_name {
                out.insert("CustomName".into(), text::to_nbt(n));
            }
        }
        Kind::EndGateway { age, exit } => {
            out.insert("Age".into(), Tag::Long(*age));
            if let Some(e) = exit {
                out.insert("exit_portal".into(), Tag::IntArray(e.to_vec()));
            }
        }
        Kind::ChiseledBookshelf { books } => {
            let items = books
                .iter()
                .enumerate()
                .filter_map(|(i, b)| Some(crate::item::java::write(b.as_ref().filter(|b| !b.is_air())?, Some(i as u8))))
                .collect();
            out.insert("Items".into(), compound_list(items));
        }
        Kind::Shelf { items } => {
            let items = items
                .iter()
                .enumerate()
                .filter_map(|(i, b)| Some(crate::item::java::write(b.as_ref().filter(|b| !b.is_air())?, Some(i as u8))))
                .collect();
            out.insert("Items".into(), compound_list(items));
        }
        Kind::Jigsaw(g) => {
            for (k, v) in [
                ("name", &g.name),
                ("target", &g.target),
                ("pool", &g.pool),
                ("final_state", &g.final_state),
                ("joint", &g.joint),
            ] {
                out.insert(k.into(), Tag::String(v.clone()));
            }
            out.insert("selection_priority".into(), Tag::Int(g.selection_priority));
            out.insert("placement_priority".into(), Tag::Int(g.placement_priority));
        }
        Kind::Structure(s) => {
            let flag = |b: bool| Tag::Byte(b as i8);
            out.insert("name".into(), Tag::String(s.name.clone()));
            out.insert("author".into(), Tag::String(s.author.clone()));
            out.insert("metadata".into(), Tag::String(s.metadata.clone()));
            for (k, v) in [("posX", s.pos[0]), ("posY", s.pos[1]), ("posZ", s.pos[2]), ("sizeX", s.size[0]), ("sizeY", s.size[1]), ("sizeZ", s.size[2])] {
                out.insert(k.into(), Tag::Int(v));
            }
            out.insert(
                "rotation".into(),
                Tag::String(["NONE", "CLOCKWISE_90", "CLOCKWISE_180", "COUNTERCLOCKWISE_90"][s.rotation.min(3) as usize].into()),
            );
            // Java has no "both" mirror.
            out.insert(
                "mirror".into(),
                Tag::String(["NONE", "LEFT_RIGHT", "FRONT_BACK", "NONE"][s.mirror.min(3) as usize].into()),
            );
            out.insert(
                "mode".into(),
                Tag::String(
                    match s.mode {
                        StructureMode::Data => "DATA",
                        StructureMode::Load => "LOAD",
                        StructureMode::Corner => "CORNER",
                        StructureMode::Save | StructureMode::Invalid | StructureMode::Export => "SAVE",
                    }
                    .into(),
                ),
            );
            out.insert("ignoreEntities".into(), flag(s.ignore_entities));
            out.insert("strict".into(), flag(s.strict));
            out.insert("powered".into(), flag(s.powered));
            out.insert("showair".into(), flag(s.show_air));
            out.insert("showboundingbox".into(), flag(s.show_bounding_box));
            out.insert("integrity".into(), Tag::Float(s.integrity));
            out.insert("seed".into(), Tag::Long(s.seed));
        }
        Kind::Empty(_) | Kind::CopperGolemStatue { .. } => {}
        Kind::ItemFrame(_)
        | Kind::Bed { .. }
        | Kind::Cauldron(_)
        | Kind::FlowerPot { .. }
        | Kind::NoteBlock { .. }
        | Kind::PistonArm { .. }
        | Kind::BedrockOnly(_)
        | Kind::Discard => {}
    }
    Some(out)
}


fn uuid_string(ints: &[i32]) -> Option<String> {
    let [a, b, c, d] = ints else { return None };
    let (hi, lo) = (((*a as u64) << 32) | (*b as u32 as u64), ((*c as u64) << 32) | (*d as u32 as u64));
    let h = format!("{hi:016x}{lo:016x}");
    Some(format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

fn uuid_ints(s: &str) -> Option<Vec<i32>> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let part = |i: usize| u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).ok().map(|v| v as i32);
    Some(vec![part(0)?, part(1)?, part(2)?, part(3)?])
}

fn read_skull(be: &Compound) -> Skull {
    let mut s = Skull::default();
    s.owner_name = string(be, "ExtraType").map(str::to_string);
    let profile = compound(be, "profile")
        .or_else(|| compound(be, "Owner"))
        .or_else(|| compound(be, "SkullOwner"));
    if let Some(p) = profile {
        s.owner_id = match p.get("id").or_else(|| p.get("Id")) {
            Some(Tag::String(id)) => Some(id.clone()),
            Some(Tag::IntArray(a)) => uuid_string(a),
            _ => None,
        };
        if let Some(n) = string(p, "name").or_else(|| string(p, "Name")) {
            s.owner_name = Some(n.to_string());
        }
        let first = match p.get("properties") {
            Some(Tag::List(l)) => l.get(0).and_then(Tag::as_compound),
            _ => compound(p, "Properties")
                .and_then(|pr| pr.get("textures"))
                .and_then(Tag::as_list)
                .and_then(|l| l.get(0))
                .and_then(Tag::as_compound),
        };
        if let Some(t) = first {
            s.texture = string(t, "value").or_else(|| string(t, "Value")).map(str::to_string);
            s.texture_signature =
                string(t, "signature").or_else(|| string(t, "Signature")).map(str::to_string);
        }
    }
    s
}

fn write_skull(s: &Skull, out: &mut Compound) {
    if s.owner_id.is_none() && s.owner_name.is_none() && s.texture.is_none() && s.texture_signature.is_none() {
        return;
    }
    let mut profile = Compound::new();
    if let Some(ints) = s.owner_id.as_deref().and_then(uuid_ints) {
        profile.insert("id".into(), Tag::IntArray(ints));
    }
    if let Some(n) = &s.owner_name {
        profile.insert("name".into(), Tag::String(n.clone()));
    }
    if s.texture.is_some() || s.texture_signature.is_some() {
        let mut entry = Compound::new();
        entry.insert("name".into(), Tag::String("textures".into()));
        if let Some(v) = &s.texture {
            entry.insert("value".into(), Tag::String(v.clone()));
        }
        if let Some(v) = &s.texture_signature {
            entry.insert("signature".into(), Tag::String(v.clone()));
        }
        profile.insert("properties".into(), compound_list(vec![entry]));
    }
    out.insert("profile".into(), Tag::Compound(profile));
}


