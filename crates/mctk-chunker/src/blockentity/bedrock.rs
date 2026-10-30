//! Bedrock 1.26.x block entities (`Bedrock*BlockEntityHandler`).

use std::collections::BTreeMap;

use mctk_nbt::{Compound, Tag};

use super::java::sign_argb;
use super::*;
use crate::ids::{entity_bedrock_to_java, entity_java_to_bedrock};
use crate::{Notes, child, compound, compound_list, int, list, string, text};

/// Widest a sign line gets before Bedrock wraps it (hanging signs are narrower).
const SIGN_MAX_WIDTH: u32 = 88;
const HANGING_SIGN_MAX_WIDTH: u32 = 60;

fn read_items(be: &Compound, notes: &mut Notes) -> BTreeMap<u8, ItemStack> {
    let mut items = BTreeMap::new();
    for (index, tag) in list(be, "Items").enumerate() {
        let Some(c) = tag.as_compound() else { continue };
        let slot = int(c, "Slot").unwrap_or(index as i64) as u8;
        if let Some(item) = crate::item::bedrock::read(c, notes) {
            items.insert(slot, item);
        }
    }
    items
}

fn read_sign_face(face: &Compound, hanging: bool) -> SignFace {
    let mut lines: Vec<String> = if let Some(t) = string(face, "Text") {
        let raw: Vec<String> = t.split('\n').map(str::to_string).collect();
        // Bedrock wraps long lines itself; split them the way the game would.
        crate::font::split(
            if hanging { HANGING_SIGN_MAX_WIDTH } else { SIGN_MAX_WIDTH },
            4,
            &raw,
        )
    } else {
        (1..=4)
            .map(|i| string(face, &format!("Text{i}")).unwrap_or_default().to_string())
            .collect()
    };
    lines.resize(4, String::new());
    SignFace {
        lines: lines.iter().map(|l| text::plain(l)).collect(),
        lit: int(face, "IgnoreLighting").unwrap_or(0) != 0,
        color: int(face, "SignTextColor").map_or(sign_argb("black"), |v| v as i32),
    }
}

fn write_sign_face(face: &SignFace) -> Compound {
    let mut t = String::new();
    for line in &face.lines {
        t.push_str(&text::to_legacy(line, true));
        t.push('\n');
    }
    let mut c = Compound::new();
    c.insert("Text".into(), Tag::String(t));
    c.insert("IgnoreLighting".into(), Tag::Byte(face.lit as i8));
    c.insert("SignTextColor".into(), Tag::Int(face.color));
    c
}

/// Reads a Bedrock block entity; `None` when the type is not handled.
pub fn read(be: &Compound, notes: &mut Notes) -> Option<BlockEntity> {
    let id = string(be, "id")?;
    let kind = if let Some(ck) = ContainerKind::from_bedrock(id) {
        let mut c = Container::new(ck);
        c.items = read_items(be, notes);
        if ck == ContainerKind::BrewingStand {
            // Bedrock numbers the ingredient and the bottles differently.
            c.items = std::mem::take(&mut c.items)
                .into_iter()
                .map(|(slot, item)| (brewing_slot(slot, true), item))
                .collect();
        }
        if ck.randomizable() {
            c.loot_table = string(be, "LootTable")
                .filter(|t| !t.is_empty())
                .map(loot_from_bedrock);
        }
        if ck.is_furnace() {
            let short = |k: &str| int(be, k).unwrap_or(0) as i16;
            c.furnace = Furnace {
                burn_time: short("BurnTime"),
                cook_time: short("CookTime"),
                // Bedrock's "burn duration" is Java's total lit time.
                cook_time_total: short("BurnDuration"),
            };
        }
        if ck == ContainerKind::Chest && (be.contains_key("pairx") || be.contains_key("pairz")) {
            c.chest_pair = Some(ChestPair {
                x: int(be, "pairx").or_else(|| int(be, "x")).unwrap_or(0) as i32,
                z: int(be, "pairz").or_else(|| int(be, "z")).unwrap_or(0) as i32,
                lead: int(be, "pairlead").unwrap_or(0) != 0,
            });
        }
        if ck == ContainerKind::BrewingStand {
            let short = |k: &str| int(be, k).unwrap_or(0) as i16;
            c.brewing = Brewing {
                brew_time: short("CookTime"),
                fuel: short("FuelAmount"),
                fuel_total: short("FuelTotal"),
            };
        }
        if ck == ContainerKind::Crafter {
            c.crafter = Crafter {
                disabled_slots: int(be, "disabled_slots").unwrap_or(0) as u16,
                crafting_ticks_remaining: int(be, "crafting_ticks_remaining").unwrap_or(0) as i32,
            };
        }
        Kind::Container(c)
    } else if let Some(ek) = EmptyKind::from_bedrock(id) {
        Kind::Empty(ek)
    } else {
        match id {
            "Sign" | "HangingSign" => {
                let hanging = id == "HangingSign";
                let mut s = Sign::new(hanging);
                s.waxed = int(be, "IsWaxed").unwrap_or(0) != 0;
                s.front = read_sign_face(compound(be, "FrontText").unwrap_or(be), hanging);
                if let Some(back) = compound(be, "BackText") {
                    s.back = read_sign_face(back, hanging);
                }
                Kind::Sign(s)
            }
            "MobSpawner" => {
                let d = Spawner::default();
                let short = |k: &str, default: i16| int(be, k).map_or(default, |v| v as i16);
                let entity = string(be, "EntityIdentifier")
                    .filter(|e| !e.is_empty() && *e != "minecraft:")
                    .map(|e| entity_bedrock_to_java(e).to_string());
                Kind::Spawner(Spawner {
                    delay: short("Delay", d.delay),
                    spawn_range: short("SpawnRange", d.spawn_range),
                    max_spawn_delay: short("MaxSpawnDelay", d.max_spawn_delay),
                    min_spawn_delay: short("MinSpawnDelay", d.min_spawn_delay),
                    spawn_count: short("SpawnCount", d.spawn_count),
                    required_player_range: short("RequiredPlayerRange", d.required_player_range),
                    max_nearby_entities: short("MaxNearbyEntities", d.max_nearby_entities),
                    entity,
                })
            }
            "Comparator" => Kind::Comparator {
                output_signal: int(be, "OutputSignal").unwrap_or(0) as i32,
            },
            "SculkSensor" => Kind::SculkSensor { calibrated: false },
            "CalibratedSculkSensor" => Kind::SculkSensor { calibrated: true },
            "Campfire" => {
                let mut f = Campfire::default();
                for i in 1..=4 {
                    if let Some(c) = compound(be, &format!("Item{i}")) {
                        f.items[i - 1] = crate::item::bedrock::read(c, notes);
                    }
                    f.cooking_times[i - 1] = int(be, &format!("ItemTime{i}")).unwrap_or(0) as i32;
                }
                Kind::Campfire(f)
            }
            "DecoratedPot" => {
                let mut pot = DecoratedPot::default();
                let names: Vec<String> = list(be, "sherds").chain(list(be, "shards")).filter_map(Tag::as_str).map(str::to_string).collect();
                for (slot, n) in pot.sherds.iter_mut().zip(names) {
                    if let Some(j) = mctk_mapping::item_mappings().bedrock_to_java(&n, 0) {
                        *slot = j.to_string();
                    }
                }
                pot.item = compound(be, "item").and_then(|i| crate::item::bedrock::read(i, notes));
                Kind::DecoratedPot(pot)
            }
            "BrushableBlock" => Kind::Brushable(Brushable {
                brush_direction: int(be, "brush_direction").unwrap_or(0) as i8,
                brush_count: int(be, "brush_count").unwrap_or(0) as i32,
                item: compound(be, "item").and_then(|i| crate::item::bedrock::read(i, notes)),
                loot_table: string(be, "LootTable").filter(|t| !t.is_empty()).map(loot_from_bedrock),
                block: string(be, "type").map(str::to_string),
            }),
            "ItemFrame" | "GlowItemFrame" => {
                let rotation = match be.get("ItemRotation") {
                    Some(Tag::Float(r)) => (((*r + 360.0) % 360.0) / 45.0).round() as i64,
                    Some(t) => t.as_i64().unwrap_or(0),
                    None => 0,
                };
                Kind::ItemFrame(ItemFrame {
                    glow: id == "GlowItemFrame",
                    item: compound(be, "Item")
                        .and_then(|i| crate::item::bedrock::read(i, notes))
                        .filter(|i| !i.is_air()),
                    rotation: rotation.rem_euclid(8) as u8,
                })
            }
            "Lectern" => Kind::Lectern {
                // The neutral form assumes two open pages.
                page: int(be, "page").unwrap_or(0) as i32 * 2,
                book: compound(be, "book").and_then(|b| crate::item::bedrock::read(b, notes)),
            },
            "Jukebox" => Kind::Jukebox {
                record: compound(be, "RecordItem").and_then(|r| crate::item::bedrock::read(r, notes)),
            },
            "Skull" => {
                let rotation = match be.get("Rotation") {
                    Some(Tag::Float(r)) => (((*r + 360.0) % 360.0) / 22.5).round() as i8,
                    _ => int(be, "Rot").unwrap_or(0) as i8,
                };
                Kind::Skull(Skull {
                    skull_type: int(be, "SkullType").unwrap_or(0) as i8,
                    rotation,
                    moving_mouth: int(be, "DoingAnimation").unwrap_or(0) != 0,
                    ..Skull::default()
                })
            }
            "Banner" => {
                let rev = |v: i64| (15 - v).clamp(0, 15) as u8;
                let mut b = Banner {
                    base: int(be, "Base").map(rev),
                    patterns: Vec::new(),
                };
                for p in list(be, "Patterns").filter_map(Tag::as_compound) {
                    let pattern = string(p, "Pattern").and_then(crate::ids::banner_pattern_from_bedrock);
                    if let (Some(c), Some(pat)) = (int(p, "Color"), pattern) {
                        b.patterns.push((rev(c), pat));
                    }
                }
                Kind::Banner(b)
            }
            "Bed" => Kind::Bed {
                color: int(be, "color").unwrap_or(0).clamp(0, 15) as u8,
            },
            "Cauldron" => Kind::Cauldron(Cauldron {
                potion: int(be, "PotionId").and_then(|id| crate::ids::Potion::from_bedrock(id as i16)),
                bottle: if int(be, "IsSplash").unwrap_or(0) != 0 {
                    1
                } else {
                    int(be, "PotionType").unwrap_or(-1) as i16
                },
                custom_color: int(be, "CustomColor").map(|v| v as i32),
            }),
            "FlowerPot" => Kind::FlowerPot {
                plant: compound(be, "PlantBlock").and_then(|p| {
                    let name = string(p, "name")?;
                    let states = compound(p, "states").cloned().unwrap_or_default();
                    mctk_mapping::mappings()
                        .bedrock_to_java(name, &states)
                        .map(|j| j.name.clone())
                }),
            },
            "Beacon" => {
                let effect = |k: &str| {
                    int(be, k)
                        .filter(|id| *id > 0)
                        .and_then(|id| crate::ids::Effect::from_bedrock(id as i32))
                };
                Kind::Beacon { primary: effect("primary"), secondary: effect("secondary") }
            }
            "CommandBlock" => Kind::CommandBlock(CommandBlock {
                command: string(be, "Command").unwrap_or_default().to_string(),
                auto: int(be, "auto").unwrap_or(0) != 0,
                track_output: int(be, "TrackOutput").unwrap_or(1) != 0,
            }),
            "EndGateway" => {
                let exit: Vec<i32> = list(be, "ExitPortal").filter_map(Tag::as_i64).map(|v| v as i32).collect();
                Kind::EndGateway {
                    age: int(be, "Age").unwrap_or(0),
                    exit: (exit.len() == 3).then(|| [exit[0], exit[1], exit[2]]),
                }
            }
            "ChiseledBookshelf" => {
                let mut books: [Option<ItemStack>; 6] = Default::default();
                for (i, tag) in list(be, "Items").take(6).enumerate() {
                    books[i] = tag.as_compound().and_then(|c| crate::item::bedrock::read(c, notes));
                }
                Kind::ChiseledBookshelf { books }
            }
            "Shelf" => {
                let mut items: [Option<ItemStack>; 3] = Default::default();
                for (i, tag) in list(be, "Items").take(3).enumerate() {
                    items[i] = tag.as_compound().and_then(|c| crate::item::bedrock::read(c, notes));
                }
                Kind::Shelf { items }
            }
            "JigsawBlock" => {
                let d = Jigsaw::default();
                let text = |k: &str, default: &str| string(be, k).unwrap_or(default).to_string();
                Kind::Jigsaw(Jigsaw {
                    name: text("name", &d.name),
                    target: text("target", &d.target),
                    pool: text("target_pool", &d.pool),
                    final_state: text("final_state", &d.final_state),
                    joint: text("joint", "rollable"),
                    selection_priority: int(be, "selection_priority").unwrap_or(0) as i32,
                    placement_priority: int(be, "placement_priority").unwrap_or(0) as i32,
                })
            }
            "StructureBlock" => {
                let flag = |k: &str, d: i64| int(be, k).unwrap_or(d) != 0;
                let num = |k: &str| int(be, k).unwrap_or(0) as i32;
                Kind::Structure(Box::new(StructureBlock {
                    name: string(be, "structureName").unwrap_or_default().to_string(),
                    metadata: string(be, "dataField").unwrap_or_default().to_string(),
                    pos: [num("xStructureOffset"), num("yStructureOffset"), num("zStructureOffset")],
                    size: [num("xStructureSize"), num("yStructureSize"), num("zStructureSize")],
                    mode: match int(be, "data").unwrap_or(0) {
                        1 => StructureMode::Save,
                        2 => StructureMode::Load,
                        3 => StructureMode::Corner,
                        4 => StructureMode::Invalid,
                        5 => StructureMode::Export,
                        _ => StructureMode::Data,
                    },
                    rotation: int(be, "rotation").unwrap_or(0).clamp(0, 3) as u8,
                    mirror: int(be, "mirror").unwrap_or(0).clamp(0, 3) as u8,
                    ignore_entities: flag("ignoreEntities", 0),
                    include_players: flag("includePlayers", 0),
                    remove_blocks: flag("removeBlocks", 0),
                    powered: flag("isPowered", 0),
                    show_bounding_box: flag("showBoundingBox", 1),
                    integrity: be.get("integrity").and_then(Tag::as_f64).unwrap_or(100.0) as f32 / 100.0,
                    seed: int(be, "seed").unwrap_or(0),
                    redstone_save_mode: num("redstoneSaveMode"),
                    last_touched_player: int(be, "lastTouchedPlayerId").unwrap_or(0),
                    animation_mode: int(be, "animationMode").unwrap_or(0) as i8,
                    animation_seconds: be.get("animationSeconds").and_then(Tag::as_f64).unwrap_or(0.0) as f32,
                    ..StructureBlock::default()
                }))
            }
            "CopperGolemStatue" => Kind::CopperGolemStatue {
                pose: int(be, "Pose").unwrap_or(0).clamp(0, 3) as u8,
            },
            "Music" => Kind::NoteBlock { note: int(be, "note").unwrap_or(0).clamp(0, 24) as u8 },
            "PistonArm" => Kind::PistonArm {
                sticky: int(be, "Sticky").unwrap_or(0) != 0,
                extended: int(be, "State").unwrap_or(0) == 2,
            },
            // Bedrock only, no data; Java has no equivalent.
            "Lodestone" | "SporeBlossom" | "MovingBlock" => Kind::Discard,
            _ => return None,
        }
    };
    let mut out = BlockEntity::new(
        int(be, "x").unwrap_or(0) as i32,
        int(be, "y").unwrap_or(0) as i32,
        int(be, "z").unwrap_or(0) as i32,
        kind,
    );
    out.movable = int(be, "isMovable").unwrap_or(0) != 0;
    out.custom_name = string(be, "CustomName").filter(|n| !n.is_empty()).map(text::plain);
    Some(out)
}

fn bedrock_id(kind: &Kind) -> &'static str {
    match kind {
        Kind::Container(c) => c.kind.bedrock_id(),
        Kind::Sign(s) if s.hanging => "HangingSign",
        Kind::Sign(_) => "Sign",
        Kind::Spawner(_) => "MobSpawner",
        Kind::Comparator { .. } => "Comparator",
        Kind::SculkSensor { calibrated: false } => "SculkSensor",
        Kind::SculkSensor { calibrated: true } => "CalibratedSculkSensor",
        Kind::Campfire(_) => "Campfire",
        Kind::DecoratedPot(_) => "DecoratedPot",
        Kind::Brushable(_) => "BrushableBlock",
        Kind::Lectern { .. } => "Lectern",
        Kind::ItemFrame(f) if f.glow => "GlowItemFrame",
        Kind::ItemFrame(_) => "ItemFrame",
        Kind::Jukebox { .. } => "Jukebox",
        Kind::Skull(_) => "Skull",
        Kind::Banner(_) => "Banner",
        Kind::Bed { .. } => "Bed",
        Kind::Cauldron(_) => "Cauldron",
        Kind::FlowerPot { .. } => "FlowerPot",
        Kind::Beacon { .. } => "Beacon",
        Kind::CommandBlock(_) => "CommandBlock",
        Kind::EndGateway { .. } => "EndGateway",
        Kind::ChiseledBookshelf { .. } => "ChiseledBookshelf",
        Kind::Shelf { .. } => "Shelf",
        Kind::Jigsaw(_) => "JigsawBlock",
        Kind::Structure(_) => "StructureBlock",
        Kind::CopperGolemStatue { .. } => "CopperGolemStatue",
        Kind::NoteBlock { .. } => "Music",
        Kind::PistonArm { .. } => "PistonArm",
        Kind::BedrockOnly(BedrockOnly::Lodestone) => "Lodestone",
        Kind::BedrockOnly(BedrockOnly::SporeBlossom) => "SporeBlossom",
        Kind::Discard => "Discard",
        Kind::Empty(e) => e.bedrock_id(),
    }
}

/// An air item as a campfire writes an empty slot.
fn air() -> Compound {
    let mut c = Compound::new();
    c.insert("Count".into(), Tag::Byte(0));
    c.insert("Damage".into(), Tag::Short(0));
    c.insert("Name".into(), Tag::String("minecraft:air".into()));
    c
}

/// Writes a block entity as Bedrock NBT (including `id`, `x`, `y`, `z`, `isMovable`).
pub fn write(be: &BlockEntity, notes: &mut Notes) -> Compound {
    let mut out = Compound::new();
    out.insert("id".into(), Tag::String(bedrock_id(&be.kind).into()));
    out.insert("x".into(), Tag::Int(be.x));
    out.insert("y".into(), Tag::Int(be.y));
    out.insert("z".into(), Tag::Int(be.z));
    out.insert("isMovable".into(), Tag::Byte(be.movable as i8));
    // Banners keep their name in the item, not the entity.
    if let (Some(n), false) = (&be.custom_name, matches!(be.kind, Kind::Banner(_))) {
        out.insert("CustomName".into(), Tag::String(text::to_legacy(n, true)));
    }
    match &be.kind {
        Kind::Container(c) => {
            let mut items = Vec::new();
            for (slot, item) in c.items.iter().filter(|(_, i)| !i.is_air()) {
                let slot = if c.kind == ContainerKind::BrewingStand { brewing_slot(*slot, false) } else { *slot };
                if let Some(i) = crate::item::bedrock::write(item, Some(slot), notes) {
                    items.push(i);
                }
            }
            out.insert("Items".into(), compound_list(items));
            if let Some(t) = c.loot_table.as_deref().filter(|_| c.kind.randomizable()) {
                match loot_to_bedrock(t) {
                    Some(p) => {
                        out.insert("LootTable".into(), Tag::String(p));
                    }
                    None => notes.miss(format!("loot table {t}")),
                }
            }
            if c.kind.is_furnace() {
                out.insert("BurnTime".into(), Tag::Short(c.furnace.burn_time));
                out.insert("CookTime".into(), Tag::Short(c.furnace.cook_time));
                out.insert("BurnDuration".into(), Tag::Short(c.furnace.cook_time_total));
            }
            if c.kind == ContainerKind::BrewingStand {
                out.insert("FuelAmount".into(), Tag::Short(c.brewing.fuel));
                out.insert("FuelTotal".into(), Tag::Short(c.brewing.fuel_total));
                out.insert("CookTime".into(), Tag::Short(c.brewing.brew_time));
            }
            if c.kind == ContainerKind::Crafter {
                out.insert("disabled_slots".into(), Tag::Short(c.crafter.disabled_slots as i16));
                out.insert(
                    "crafting_ticks_remaining".into(),
                    Tag::Int(c.crafter.crafting_ticks_remaining),
                );
            }
            if let Some(p) = c.chest_pair {
                out.insert("pairlead".into(), Tag::Byte(p.lead as i8));
                out.insert("pairx".into(), Tag::Int(p.x));
                out.insert("pairz".into(), Tag::Int(p.z));
            }
        }
        Kind::Sign(s) => {
            out.insert("IsWaxed".into(), Tag::Byte(s.waxed as i8));
            out.insert("FrontText".into(), Tag::Compound(write_sign_face(&s.front)));
            out.insert("BackText".into(), Tag::Compound(write_sign_face(&s.back)));
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
                out.insert(
                    "EntityIdentifier".into(),
                    Tag::String(entity_java_to_bedrock(e).to_string()),
                );
            }
        }
        Kind::Comparator { output_signal } => {
            out.insert("OutputSignal".into(), Tag::Int(*output_signal));
        }
        Kind::SculkSensor { .. } => {
            child(&mut out, "VibrationListener").insert("selector".into(), Tag::Compound(Compound::new()));
        }
        Kind::Campfire(f) => {
            for i in 0..4 {
                let item = f.items[i].as_ref().filter(|i| !i.is_air());
                let tag = match item {
                    Some(item) => crate::item::bedrock::write(item, None, notes).unwrap_or_else(air),
                    None => air(),
                };
                out.insert(format!("Item{}", i + 1), Tag::Compound(tag));
                out.insert(format!("ItemTime{}", i + 1), Tag::Int(f.cooking_times[i]));
            }
        }
        Kind::DecoratedPot(p) => {
            let names: Vec<String> = p
                .sherds
                .iter()
                .map(|j| {
                    mctk_mapping::item_mappings()
                        .java_to_bedrock(j)
                        .map_or_else(|| j.clone(), |(n, _)| n.to_string())
                })
                .collect();
            out.insert("sherds".into(), crate::string_list(names));
            if let Some(item) = p.item.as_ref().filter(|i| !i.is_air()) {
                if let Some(i) = crate::item::bedrock::write(item, None, notes) {
                    out.insert("item".into(), Tag::Compound(i));
                }
            }
        }
        Kind::Brushable(b) => {
            out.insert(
                "type".into(),
                Tag::String(b.block.clone().unwrap_or_else(|| "minecraft:suspicious_sand".into())),
            );
            out.insert("brush_direction".into(), Tag::Byte(b.brush_direction));
            out.insert("brush_count".into(), Tag::Int(b.brush_count));
            if let Some(item) = b.item.as_ref().filter(|i| !i.is_air()) {
                if let Some(i) = crate::item::bedrock::write(item, None, notes) {
                    out.insert("item".into(), Tag::Compound(i));
                }
            }
            if let Some(t) = b.loot_table.as_deref().filter(|t| !t.is_empty()) {
                match loot_to_bedrock(t) {
                    Some(p) => {
                        out.insert("LootTable".into(), Tag::String(p));
                    }
                    None => notes.miss(format!("loot table {t}")),
                }
            }
        }
        Kind::Lectern { book, page } => {
            out.insert("page".into(), Tag::Int(page / 2));
            let written = book
                .as_ref()
                .filter(|b| !b.is_air())
                .and_then(|b| crate::item::bedrock::write(b, None, notes).map(|w| (b, w)));
            match written {
                Some((b, tag)) => {
                    let pages = b.props.book_pages.as_ref().map_or(0, Vec::len);
                    out.insert("hasBook".into(), Tag::Byte(1));
                    out.insert("book".into(), Tag::Compound(tag));
                    out.insert("totalPages".into(), Tag::Int(pages as i32));
                }
                None => {
                    out.insert("hasBook".into(), Tag::Byte(0));
                }
            }
        }
        Kind::ItemFrame(f) => {
            if let Some(i) = f.item.as_ref().filter(|i| !i.is_air()) {
                if let Some(written) = crate::item::bedrock::write(i, None, notes) {
                    out.insert("Item".into(), Tag::Compound(written));
                }
            }
            out.insert("ItemRotation".into(), Tag::Float(f.rotation as f32 * 45.0));
        }
        Kind::Jukebox { record } => {
            if let Some(r) = record.as_ref().filter(|r| !r.is_air()) {
                if let Some(i) = crate::item::bedrock::write(r, None, notes) {
                    out.insert("RecordItem".into(), Tag::Compound(i));
                }
            }
        }
        Kind::Skull(s) => {
            out.insert("SkullType".into(), Tag::Byte(s.skull_type));
            out.insert("Rotation".into(), Tag::Float(s.rotation as f32 * 22.5));
            out.insert("DoingAnimation".into(), Tag::Byte(s.moving_mouth as i8));
        }
        Kind::Banner(b) => {
            let rev = |d: u8| Tag::Int(15 - d.min(15) as i32);
            if let Some(base) = b.base {
                out.insert("Base".into(), rev(base));
            }
            let patterns = b
                .patterns
                .iter()
                .map(|(dye, pat)| {
                    let mut p = Compound::new();
                    p.insert("Color".into(), rev(*dye));
                    p.insert("Pattern".into(), Tag::String(crate::ids::BANNER_PATTERNS[*pat].1.into()));
                    p
                })
                .collect();
            out.insert("Patterns".into(), compound_list(patterns));
        }
        Kind::Bed { color } => {
            out.insert("color".into(), Tag::Byte(*color as i8));
        }
        Kind::Cauldron(c) => {
            out.insert("IsSplash".into(), Tag::Byte((c.bottle == 1) as i8));
            out.insert("PotionType".into(), Tag::Short(c.bottle));
            out.insert(
                "PotionId".into(),
                Tag::Short(c.potion.and_then(crate::ids::Potion::bedrock).unwrap_or(-1)),
            );
            if let Some(color) = c.custom_color {
                out.insert("CustomColor".into(), Tag::Int(color));
            }
        }
        Kind::FlowerPot { plant } => {
            let spec = plant.as_deref().and_then(|name| {
                mctk_mapping::mappings().java_to_bedrock(&mctk_java::BlockState::new(name))
            });
            if let Some(block) = spec {
                out.insert("PlantBlock".into(), block.to_nbt().tag);
            }
        }
        Kind::Beacon { primary, secondary } => {
            let id = |e: &Option<crate::ids::Effect>| {
                Tag::Int(e.and_then(crate::ids::Effect::bedrock).unwrap_or(0))
            };
            out.insert("primary".into(), id(primary));
            out.insert("secondary".into(), id(secondary));
        }
        Kind::CommandBlock(c) => {
            out.insert("Command".into(), Tag::String(c.command.clone()));
            out.insert("auto".into(), Tag::Byte(c.auto as i8));
            out.insert("TrackOutput".into(), Tag::Byte(c.track_output as i8));
        }
        Kind::EndGateway { age, exit } => {
            out.insert("Age".into(), Tag::Int(*age as i32));
            if let Some(e) = exit {
                let l = mctk_nbt::List::from_tags(
                    mctk_nbt::TagType::Int,
                    e.iter().map(|v| Tag::Int(*v)).collect(),
                )
                .expect("ints");
                out.insert("ExitPortal".into(), Tag::List(l));
            }
        }
        Kind::ChiseledBookshelf { books } => {
            // Every one of the six slots is listed; empty ones as air.
            let items = books
                .iter()
                .map(|b| {
                    b.as_ref()
                        .filter(|b| !b.is_air())
                        .and_then(|b| crate::item::bedrock::write(b, None, notes))
                        .unwrap_or_else(air)
                })
                .collect();
            out.insert("Items".into(), compound_list(items));
        }
        Kind::Shelf { items } => {
            let items = items
                .iter()
                .map(|b| {
                    b.as_ref()
                        .filter(|b| !b.is_air())
                        .and_then(|b| crate::item::bedrock::write(b, None, notes))
                        .unwrap_or_else(air)
                })
                .collect();
            out.insert("Items".into(), compound_list(items));
        }
        Kind::Jigsaw(g) => {
            for (k, v) in [
                ("name", &g.name),
                ("target", &g.target),
                ("target_pool", &g.pool),
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
            out.insert("structureName".into(), Tag::String(s.name.clone()));
            out.insert("dataField".into(), Tag::String(s.metadata.clone()));
            for (k, v) in [
                ("xStructureOffset", s.pos[0]),
                ("yStructureOffset", s.pos[1]),
                ("zStructureOffset", s.pos[2]),
                ("xStructureSize", s.size[0]),
                ("yStructureSize", s.size[1]),
                ("zStructureSize", s.size[2]),
            ] {
                out.insert(k.into(), Tag::Int(v));
            }
            let mode = match s.mode {
                StructureMode::Data => 0,
                StructureMode::Save => 1,
                StructureMode::Load => 2,
                StructureMode::Corner => 3,
                StructureMode::Invalid => 4,
                StructureMode::Export => 5,
            };
            out.insert("data".into(), Tag::Int(mode));
            out.insert("rotation".into(), Tag::Byte(s.rotation as i8));
            out.insert("mirror".into(), Tag::Byte(s.mirror as i8));
            out.insert("ignoreEntities".into(), flag(s.ignore_entities));
            out.insert("includePlayers".into(), flag(s.include_players));
            out.insert("removeBlocks".into(), flag(s.remove_blocks));
            out.insert("isPowered".into(), flag(s.powered));
            out.insert("showBoundingBox".into(), flag(s.show_bounding_box));
            out.insert("integrity".into(), Tag::Float(s.integrity * 100.0));
            out.insert("seed".into(), Tag::Long(s.seed));
            out.insert("redstoneSaveMode".into(), Tag::Int(s.redstone_save_mode));
            out.insert("lastTouchedPlayerId".into(), Tag::Long(s.last_touched_player));
            out.insert("animationMode".into(), Tag::Byte(s.animation_mode));
            out.insert("animationSeconds".into(), Tag::Float(s.animation_seconds));
        }
        Kind::CopperGolemStatue { pose } => {
            out.insert("Pose".into(), Tag::Int(*pose as i32));
        }
        Kind::NoteBlock { note } => {
            out.insert("note".into(), Tag::Byte(*note as i8));
        }
        Kind::PistonArm { sticky, extended } => {
            let (progress, state) = if *extended { (1.0f32, 2i8) } else { (0.0, 0) };
            out.insert("isMovable".into(), Tag::Byte(1));
            out.insert("LastProgress".into(), Tag::Float(progress));
            out.insert("NewState".into(), Tag::Byte(state));
            out.insert("Progress".into(), Tag::Float(progress));
            out.insert("State".into(), Tag::Byte(state));
            out.insert("Sticky".into(), Tag::Byte(*sticky as i8));
        }
        Kind::BedrockOnly(_) | Kind::Discard => {}
        Kind::Empty(_) => {}
    }
    out
}




/// Brewing stands number their slots differently: Java has the three bottles in 0–2, the
/// ingredient in 3 and fuel in 4; Bedrock has the ingredient in 0 and the bottles in 1–3.
fn brewing_slot(slot: u8, from_bedrock: bool) -> u8 {
    match (from_bedrock, slot) {
        (true, 0) => 3,
        (true, 1..=3) => slot - 1,
        (false, 3) => 0,
        (false, 0..=2) => slot + 1,
        _ => slot,
    }
}
