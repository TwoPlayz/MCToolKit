//! Java worlds from before Java 26.x: the pieces of Chunker's older-version handling that
//! the converters need to read them.
//!
//! * Before 1.13 blocks are numeric ids with a data value, biomes a column of ids, and block
//!   entities, entities and items use short names (`Chest`, `Pig`) and `Damage` / `tag`.
//! * From 1.13 to 1.20.4 blocks are palettes but items still use `Count` and `tag`.
//!
//! Everything is turned into what the 26.x readers expect (palettes, `components`), and the
//! converters carry on as for a current world. The id tables come from `mctk-mapping`
//! (`legacy`).

use std::collections::HashMap;

use mctk_java::{BlockState, BlockStorage, Chunk, ChunkEra, PalettedContainer};
use mctk_mapping::legacy::legacy;
use mctk_mapping::mappings;
use mctk_nbt::{Compound, List, NamedTag, Tag, TagType};

use crate::java_bedrock::EditionReport;

/// First data version that stores items as `components` (1.20.5).
pub(crate) const DV_COMPONENTS: i32 = 3837;

// ----------------------------------------------------------------------- biomes

/// The biome ids a pre-1.18 chunk keeps beside its sections.
pub(crate) struct Biomes {
    ids: Vec<i32>,
    /// One id per block column (z * 16 + x), else one per 4x4x4 cell over the whole height.
    columns: bool,
    /// Bedrock biome ids rather than Java's pre-1.18 ones.
    bedrock: bool,
}

impl Biomes {
    /// The 256 biome bytes of a Bedrock `Data2D` record (after its 512-byte height map).
    pub(crate) fn from_bedrock_data2d(data: &[u8]) -> Option<Biomes> {
        let ids = data.get(512..768)?;
        Some(Biomes {
            ids: ids.iter().map(|b| *b as i32).collect(),
            columns: true,
            bedrock: true,
        })
    }
}

/// Reads `Level.Biomes`: bytes or ints per column (1.0 - 1.14) or ints per cell (1.15 - 1.17).
pub(crate) fn read_biomes(root: &NamedTag) -> Option<Biomes> {
    let level = root.tag.get("Level")?;
    match level.get("Biomes")? {
        Tag::ByteArray(b) if b.len() == 256 => Some(Biomes {
            ids: b.iter().map(|v| *v as u8 as i32).collect(),
            columns: true,
            bedrock: false,
        }),
        Tag::IntArray(a) if a.len() == 256 => Some(Biomes {
            ids: a.clone(),
            columns: true,
            bedrock: false,
        }),
        Tag::IntArray(a) if a.len() == 1024 => Some(Biomes {
            ids: a.clone(),
            columns: false,
            bedrock: false,
        }),
        _ => None,
    }
}

pub(crate) fn section_biomes(b: &Biomes, sy: i8, report: &mut EditionReport) -> PalettedContainer<String> {
    let mut palette: Vec<String> = Vec::new();
    let mut indices = vec![0u16; 64];
    for cy in 0..4usize {
        for cz in 0..4usize {
            for cx in 0..4usize {
                let at = if b.columns {
                    cz * 4 * 16 + cx * 4
                } else {
                    ((sy as usize * 4 + cy) * 4 + cz) * 4 + cx
                };
                let id = b.ids.get(at).copied().unwrap_or(1);
                // 255 is "not set": the game picks the biome when the chunk is next loaded.
                let found = if b.bedrock {
                    mappings().bedrock_biome_to_java(id)
                } else {
                    legacy().biome(id)
                };
                let name = found.unwrap_or_else(|| {
                    if id != 255 && id != -1 {
                        report.skip(&format!("biome id {id} (shown as plains)"), 1);
                    }
                    "minecraft:plains"
                });
                let slot = match palette.iter().position(|p| p == name) {
                    Some(i) => i,
                    None => {
                        palette.push(name.to_string());
                        palette.len() - 1
                    }
                };
                indices[(cy * 4 + cz) * 4 + cx] = slot as u16;
            }
        }
    }
    PalettedContainer { palette, indices }
}

// ----------------------------------------------------------------------- blocks

const DOORS: [u16; 7] = [64, 71, 193, 194, 195, 196, 197];

/// The state for a numeric block. A door's halves each hold half of its state, so `other` is
/// the data value of the half above (for a lower half) or below (for an upper one).
fn resolve_block(id: u16, data: u8, other: u8) -> Option<BlockState> {
    let l = legacy();
    let state = l.block(id, data)?.clone();
    if !DOORS.contains(&id) {
        return Some(state);
    }
    let mut out = state;
    if data & 8 != 0 {
        // Upper half: hinge and powered are its own; facing and open come from the lower half.
        if let Some(lower) = l.block(id, other & 7) {
            for k in ["facing", "open"] {
                if let Some(v) = lower.properties.get(k) {
                    out.properties.insert(k.into(), v.clone());
                }
            }
        }
    } else if let Some(upper) = l.block(id, other | 8) {
        for k in ["hinge", "powered"] {
            if let Some(v) = upper.properties.get(k) {
                out.properties.insert(k.into(), v.clone());
            }
        }
    }
    Some(out)
}

/// Block names older versions spelled differently from the current game.
const RENAMED_BLOCKS: [(&str, &str); 3] = [
    ("minecraft:grass_path", "minecraft:dirt_path"),
    ("minecraft:grass", "minecraft:short_grass"),
    ("minecraft:chain", "minecraft:iron_chain"),
];

/// Renames blocks a palette-era chunk spells the old way (only where the old name is not a
/// block the game has now).
fn rename_old_blocks(state: &mut BlockState) {
    let m = mappings();
    if m.is_known_java(state) {
        return;
    }
    if let Some((_, new)) = RENAMED_BLOCKS.iter().find(|(old, _)| *old == state.name) {
        let mut renamed = state.clone();
        renamed.name = (*new).to_string();
        if m.is_known_java(&renamed) {
            *state = renamed;
        }
    }
}

/// Turns a chunk written by an older game into a palette chunk: numeric blocks become states,
/// old block names are renamed, and biomes are filled in from the chunk's biome ids.
pub(crate) fn modernize_chunk(chunk: &mut Chunk, biomes: Option<&Biomes>, report: &mut EditionReport) {
    if chunk.era == ChunkEra::Legacy {
        let raw: HashMap<i8, (Vec<u16>, Vec<u8>)> = chunk
            .sections
            .iter()
            .filter_map(|s| match &s.blocks {
                Some(BlockStorage::Legacy { ids, data }) => Some((s.y, (ids.clone(), data.clone()))),
                _ => None,
            })
            .collect();
        // The raw data value at a position, for a door's other half.
        let data_at = |sy: i8, x: usize, y: i32, z: usize, id: u16| -> u8 {
            let (sy, ly) = (sy as i32 + y.div_euclid(16), y.rem_euclid(16) as usize);
            raw.get(&(sy as i8))
                .map(|(ids, data)| {
                    let i = mctk_java::block_index(x, ly, z);
                    if ids[i] == id { data[i] } else { 0 }
                })
                .unwrap_or(0)
        };
        for section in &mut chunk.sections {
            let Some(BlockStorage::Legacy { ids, data }) = &section.blocks else {
                continue;
            };
            let mut palette: Vec<BlockState> = Vec::new();
            let mut slot_of: HashMap<(u16, u8, u8), u16> = HashMap::new();
            let mut indices = vec![0u16; 4096];
            for y in 0..16usize {
                for z in 0..16usize {
                    for x in 0..16usize {
                        let i = mctk_java::block_index(x, y, z);
                        let (id, d) = (ids[i], data[i]);
                        let other = if DOORS.contains(&id) {
                            let dy = if d & 8 != 0 { -1 } else { 1 };
                            data_at(section.y, x, y as i32 + dy, z, id)
                        } else {
                            0
                        };
                        let slot = *slot_of.entry((id, d, other)).or_insert_with(|| {
                            let state = resolve_block(id, d, other).unwrap_or_else(|| {
                                *report
                                    .unmapped
                                    .entry(format!("legacy block {id}:{d}"))
                                    .or_insert(0) += 1;
                                BlockState::air()
                            });
                            match palette.iter().position(|p| *p == state) {
                                Some(p) => p as u16,
                                None => {
                                    palette.push(state);
                                    (palette.len() - 1) as u16
                                }
                            }
                        });
                        indices[i] = slot;
                    }
                }
            }
            section.blocks = Some(BlockStorage::Palette(PalettedContainer { palette, indices }));
        }
    } else {
        for section in &mut chunk.sections {
            if let Some(BlockStorage::Palette(p)) = &mut section.blocks {
                p.palette.iter_mut().for_each(rename_old_blocks);
            }
        }
    }
    if let Some(b) = biomes {
        for section in &mut chunk.sections {
            if section.biomes.is_none() {
                section.biomes = Some(section_biomes(b, section.y, report));
            }
        }
    }
}

/// Legacy beds are all one block; the colour is in the block entity. Renames the bed blocks
/// to their colour.
pub(crate) fn colour_beds(chunk: &mut Chunk, block_entities: &[Compound]) {
    for be in block_entities {
        if be.get("id").and_then(Tag::as_str) != Some("minecraft:bed") {
            continue;
        }
        let at = |k: &str| be.get(k).and_then(Tag::as_i64);
        let (Some(x), Some(y), Some(z), Some(color)) = (at("x"), at("y"), at("z"), at("color")) else {
            continue;
        };
        let Some(dye) = mctk_chunker::ids::DYES.get(color.clamp(0, 15) as usize).map(|d| d.0) else {
            continue;
        };
        let Some(section) = chunk.sections.iter_mut().find(|s| s.y as i64 == y.div_euclid(16)) else {
            continue;
        };
        let Some(BlockStorage::Palette(p)) = &mut section.blocks else {
            continue;
        };
        let i = mctk_java::block_index(x.rem_euclid(16) as usize, y.rem_euclid(16) as usize, z.rem_euclid(16) as usize);
        let mut state = p.get(i).clone();
        if state.name.ends_with("_bed") {
            state.name = format!("minecraft:{dye}_bed");
            p.set(i, state);
        }
    }
}

/// The state for a numeric block id and data value of a pre-1.2.13 Bedrock chunk. Bedrock gave
/// ids below 198 (except 95) the same names as Java 1.12, but it stored data values its own
/// way (a hopper's bit, a door's bits), so the id goes by name through the table of Bedrock's
/// own conversion of old blocks. Where that table has no entry Java's reading is used.
fn bedrock_legacy_block(id: u16, data: u8) -> Option<BlockState> {
    if id >= 198 || id == 95 {
        // Bedrock numbered its newer blocks differently from Java from here on.
        return None;
    }
    let name = legacy().block_name(id)?;
    if let Some(b) = mctk_mapping::legacy::legacy_bedrock_block(name, data as u16) {
        if let Some(j) = mappings().bedrock_to_java(&b.name, &b.states) {
            return Some(j.clone());
        }
    }
    legacy().block(id, data).cloned()
}

/// A pre-1.2.13 Bedrock block storage as a palette: `ids` and `data` are the numeric blocks and
/// their data values (4096 bytes and 2048 nibbles), indexed as Bedrock indexes subchunks.
/// Blocks with ids Bedrock numbered differently from Java (198 and up) become air and are
/// counted in the report.
pub(crate) fn bedrock_legacy_section(
    get: impl Fn(usize, usize, usize) -> (u16, u8),
    report: &mut EditionReport,
) -> PalettedContainer<BlockState> {
    let mut palette: Vec<BlockState> = Vec::new();
    let mut slot_of: HashMap<(u16, u8), u16> = HashMap::new();
    let mut indices = vec![0u16; 4096];
    for y in 0..16usize {
        for z in 0..16usize {
            for x in 0..16usize {
                let (id, d) = get(x, y, z);
                let slot = *slot_of.entry((id, d)).or_insert_with(|| {
                    let state = bedrock_legacy_block(id, d).unwrap_or_else(|| {
                        *report
                            .unmapped
                            .entry(format!("legacy Bedrock block {id}:{d}"))
                            .or_insert(0) += 1;
                        BlockState::air()
                    });
                    match palette.iter().position(|p| *p == state) {
                        Some(p) => p as u16,
                        None => {
                            palette.push(state);
                            (palette.len() - 1) as u16
                        }
                    }
                });
                indices[mctk_java::block_index(x, y, z)] = slot;
            }
        }
    }
    *report
        .approximated
        .entry("pre-1.2.13 Bedrock chunks (blocks 198 and up are not read)".to_string())
        .or_insert(0) += 1;
    PalettedContainer { palette, indices }
}

fn nibble_of(data: &[u8], i: usize) -> u8 {
    let b = data.get(i / 2).copied().unwrap_or(0);
    if i % 2 == 0 { b & 0x0f } else { b >> 4 }
}

/// A legacy subchunk body: 4096 block ids, then 2048 data nibbles.
pub(crate) fn bedrock_legacy_subchunk(body: &[u8], report: &mut EditionReport) -> PalettedContainer<BlockState> {
    bedrock_legacy_section(
        |x, y, z| {
            let i = (x * 16 + z) * 16 + y;
            (
                body.get(i).copied().unwrap_or(0) as u16,
                nibble_of(body.get(4096..).unwrap_or(&[]), i),
            )
        },
        report,
    )
}

/// The sections (0..8) of a `LegacyTerrain` record: a whole 16x128x16 column with ids, then data
/// nibbles, in x, z, y order.
pub(crate) fn bedrock_legacy_terrain(
    body: &[u8],
    report: &mut EditionReport,
) -> Vec<(i8, PalettedContainer<BlockState>)> {
    (0..8i8)
        .map(|sy| {
            let section = bedrock_legacy_section(
                |x, y, z| {
                    let i = x * 2048 + z * 128 + sy as usize * 16 + y;
                    (
                        body.get(i).copied().unwrap_or(0) as u16,
                        nibble_of(body.get(32768..).unwrap_or(&[]), i),
                    )
                },
                report,
            );
            (sy, section)
        })
        .collect()
}

// ------------------------------------------------------------------------- items

/// Pre-1.13 enchantment ids.
const ENCHANTMENTS: [(i64, &str); 30] = [
    (0, "protection"),
    (1, "fire_protection"),
    (2, "feather_falling"),
    (3, "blast_protection"),
    (4, "projectile_protection"),
    (5, "respiration"),
    (6, "aqua_affinity"),
    (7, "thorns"),
    (8, "depth_strider"),
    (9, "frost_walker"),
    (10, "binding_curse"),
    (16, "sharpness"),
    (17, "smite"),
    (18, "bane_of_arthropods"),
    (19, "knockback"),
    (20, "fire_aspect"),
    (21, "looting"),
    (22, "sweeping_edge"),
    (32, "efficiency"),
    (33, "silk_touch"),
    (34, "unbreaking"),
    (35, "fortune"),
    (48, "power"),
    (49, "punch"),
    (50, "flame"),
    (51, "infinity"),
    (61, "luck_of_the_sea"),
    (62, "lure"),
    (70, "mending"),
    (71, "vanishing_curse"),
];

fn int_of(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(Tag::as_i64)
}

fn string_list(items: Vec<Tag>) -> Tag {
    if items.is_empty() {
        return Tag::List(List::new(TagType::End));
    }
    Tag::List(List::from_tags(TagType::String, items).expect("strings"))
}

fn pages(tag: &Compound, key: &str, wrap: bool) -> Tag {
    let list = tag.get(key).and_then(Tag::as_list);
    let items: Vec<Tag> = list
        .into_iter()
        .flat_map(List::iter)
        .filter_map(Tag::as_str)
        .map(|p| {
            if wrap {
                let mut c = Compound::new();
                c.insert("raw".into(), Tag::String(p.into()));
                Tag::Compound(c)
            } else {
                Tag::String(p.into())
            }
        })
        .collect();
    if items.is_empty() {
        return Tag::List(List::new(TagType::End));
    }
    let ty = if wrap { TagType::Compound } else { TagType::String };
    Tag::List(List::from_tags(ty, items).expect("pages"))
}

/// Enchantments from either spelling: `{id: 16s, lvl: 5s}` or `{id: "minecraft:sharpness", lvl: 5s}`.
fn enchantments(list: Option<&Tag>) -> Compound {
    let mut out = Compound::new();
    for e in list.and_then(Tag::as_list).into_iter().flat_map(List::iter) {
        let Some(e) = e.as_compound() else { continue };
        let name = match e.get("id") {
            Some(Tag::String(s)) => Some(s.trim_start_matches("minecraft:").to_string()),
            Some(t) => t
                .as_i64()
                .and_then(|n| ENCHANTMENTS.iter().find(|(i, _)| *i == n))
                .map(|(_, n)| (*n).to_string()),
            None => None,
        };
        if let Some(name) = name {
            out.insert(
                format!("minecraft:{name}"),
                Tag::Int(int_of(e, "lvl").unwrap_or(1) as i32),
            );
        }
    }
    out
}

/// Converts an item from any older format to the 26.x one: `{id, count, components}`. `old_ids`
/// says whether string ids are pre-1.13 names. Returns `None` for empty stacks and items the
/// tables do not know.
pub(crate) fn upgrade_item(item: &Compound, old_ids: bool, report: &mut EditionReport) -> Option<Compound> {
    // Already current.
    if item.contains_key("components") && item.contains_key("count") {
        return Some(item.clone());
    }
    let l = legacy();
    let damage_top = int_of(item, "Damage");
    let tag = item.get("tag").and_then(Tag::as_compound);
    let damage = damage_top
        .or_else(|| tag.and_then(|t| int_of(t, "Damage")))
        .unwrap_or(0);
    let numeric = match item.get("id")? {
        Tag::String(s) if old_ids => l.numeric_id(s),
        Tag::String(_) => None,
        other => other.as_i64().map(|n| n as u16),
    };
    let (id, exact) = match (numeric, item.get("id")) {
        (Some(n), _) => {
            let name = l.item(n, damage.max(0) as u16)?;
            (name.to_string(), l.item(n, 0) != Some(name) || damage == 0)
        }
        (None, Some(Tag::String(s))) => (s.clone(), false),
        _ => return None,
    };
    if id == "minecraft:air" {
        return None;
    }
    let count = int_of(item, "Count").or_else(|| int_of(item, "count")).unwrap_or(1);
    let mut out = Compound::new();
    if let Some(slot) = item.get("Slot") {
        out.insert("Slot".into(), slot.clone());
    }
    out.insert("id".into(), Tag::String(id.clone()));
    out.insert("count".into(), Tag::Int(count as i32));
    let mut c = Compound::new();
    // Tools keep their wear in the damage value (before 1.13, where an unknown value is wear).
    if damage > 0 && (!numeric.is_some() || !exact) {
        c.insert("minecraft:damage".into(), Tag::Int(damage as i32));
    }
    if let Some(tag) = tag {
        let mut handled = vec!["Damage"];
        let ench = tag.get("Enchantments").or_else(|| tag.get("ench"));
        if ench.is_some() {
            handled.extend(["Enchantments", "ench"]);
            let e = enchantments(ench);
            if !e.is_empty() {
                c.insert("minecraft:enchantments".into(), Tag::Compound(e));
            }
        }
        if let Some(stored) = tag.get("StoredEnchantments") {
            handled.push("StoredEnchantments");
            c.insert("minecraft:stored_enchantments".into(), Tag::Compound(enchantments(Some(stored))));
        }
        if let Some(display) = tag.get("display").and_then(Tag::as_compound) {
            handled.push("display");
            if let Some(name) = display.get("Name") {
                c.insert("minecraft:custom_name".into(), name.clone());
            }
            if let Some(Tag::List(lore)) = display.get("Lore") {
                let lines: Vec<Tag> = lore.iter().cloned().collect();
                c.insert("minecraft:lore".into(), string_list(lines));
            }
            if let Some(color) = int_of(display, "color") {
                c.insert("minecraft:dyed_color".into(), Tag::Int(color as i32));
            }
        }
        if id == "minecraft:written_book" {
            handled.extend(["title", "author", "pages", "generation", "resolved"]);
            let mut b = Compound::new();
            let mut title = Compound::new();
            title.insert("raw".into(), Tag::String(tag.get("title").and_then(Tag::as_str).unwrap_or("").into()));
            b.insert("title".into(), Tag::Compound(title));
            b.insert("author".into(), Tag::String(tag.get("author").and_then(Tag::as_str).unwrap_or("").into()));
            b.insert("pages".into(), pages(tag, "pages", true));
            c.insert("minecraft:written_book_content".into(), Tag::Compound(b));
        } else if id == "minecraft:writable_book" {
            handled.push("pages");
            let mut b = Compound::new();
            b.insert("pages".into(), pages(tag, "pages", true));
            c.insert("minecraft:writable_book_content".into(), Tag::Compound(b));
        }
        if let Some(Tag::String(p)) = tag.get("Potion") {
            handled.push("Potion");
            let mut pc = Compound::new();
            pc.insert("potion".into(), Tag::String(p.clone()));
            c.insert("minecraft:potion_contents".into(), Tag::Compound(pc));
        }
        // Shulker boxes (and other block items) carry their contents in `BlockEntityTag`.
        if let Some(bet) = tag.get("BlockEntityTag").and_then(Tag::as_compound) {
            handled.push("BlockEntityTag");
            let contents: Vec<Tag> = bet
                .get("Items")
                .and_then(Tag::as_list)
                .into_iter()
                .flat_map(List::iter)
                .filter_map(Tag::as_compound)
                .filter_map(|i| {
                    let slot = int_of(i, "Slot").unwrap_or(0) as i32;
                    let mut inner = upgrade_item(i, old_ids, report)?;
                    inner.shift_remove("Slot");
                    let mut entry = Compound::new();
                    entry.insert("slot".into(), Tag::Int(slot));
                    entry.insert("item".into(), Tag::Compound(inner));
                    Some(Tag::Compound(entry))
                })
                .collect();
            if !contents.is_empty() {
                c.insert(
                    "minecraft:container".into(),
                    Tag::List(List::from_tags(TagType::Compound, contents).expect("compounds")),
                );
            }
        }
        // Spawn eggs name their entity in `EntityTag`.
        if let Some(entity) = tag.get("EntityTag").and_then(Tag::as_compound) {
            handled.push("EntityTag");
            if entity.contains_key("id") {
                c.insert("minecraft:entity_data".into(), Tag::Compound(entity.clone()));
            }
        }
        handled.extend(["Unbreakable", "RepairCost", "HideFlags"]);
        for key in tag.keys().filter(|k| !handled.contains(&k.as_str())) {
            report.skip(&format!("old item tag {key}"), 1);
        }
    }
    if !c.is_empty() {
        out.insert("components".into(), Tag::Compound(c));
    }
    Some(out)
}

fn upgrade_items(list: &Tag, old_ids: bool, report: &mut EditionReport) -> Tag {
    let items: Vec<Tag> = list
        .as_list()
        .into_iter()
        .flat_map(List::iter)
        .filter_map(Tag::as_compound)
        .filter_map(|i| upgrade_item(i, old_ids, report))
        .map(Tag::Compound)
        .collect();
    if items.is_empty() {
        return Tag::List(List::new(TagType::End));
    }
    Tag::List(List::from_tags(TagType::Compound, items).expect("compounds"))
}

// ---------------------------------------------------------------- block entities

/// Pre-1.11 block entity ids.
const OLD_BLOCK_ENTITIES: [(&str, &str); 25] = [
    ("Airportal", "minecraft:end_portal"),
    ("Banner", "minecraft:banner"),
    ("Beacon", "minecraft:beacon"),
    ("Bed", "minecraft:bed"),
    ("Cauldron", "minecraft:brewing_stand"),
    ("Chest", "minecraft:chest"),
    ("Comparator", "minecraft:comparator"),
    ("Control", "minecraft:command_block"),
    ("DLDetector", "minecraft:daylight_detector"),
    ("Dropper", "minecraft:dropper"),
    ("EnchantTable", "minecraft:enchanting_table"),
    ("EndGateway", "minecraft:end_gateway"),
    ("EnderChest", "minecraft:ender_chest"),
    ("FlowerPot", "minecraft:flower_pot"),
    ("Furnace", "minecraft:furnace"),
    ("Hopper", "minecraft:hopper"),
    ("MobSpawner", "minecraft:mob_spawner"),
    ("Music", "minecraft:note_block"),
    ("Piston", "minecraft:piston"),
    ("RecordPlayer", "minecraft:jukebox"),
    ("Sign", "minecraft:sign"),
    ("Skull", "minecraft:skull"),
    ("Structure", "minecraft:structure_block"),
    ("Trap", "minecraft:dispenser"),
    ("Shulker", "minecraft:shulker_box"),
];

/// Upgrades a block entity: old ids are renamed, items converted, and a spawner's `EntityId`
/// becomes `SpawnData`. `old` is true for pre-1.13 chunks.
pub(crate) fn upgrade_block_entity(be: &Compound, old: bool, report: &mut EditionReport) -> Compound {
    let mut out = be.clone();
    if let Some(id) = be.get("id").and_then(Tag::as_str) {
        if let Some((_, new)) = OLD_BLOCK_ENTITIES.iter().find(|(o, _)| *o == id) {
            out.insert("id".into(), Tag::String((*new).into()));
        }
    }
    if let Some(items) = be.get("Items") {
        out.insert("Items".into(), upgrade_items(items, old, report));
    }
    for key in ["RecordItem", "Item"] {
        if let Some(item) = be.get(key).and_then(Tag::as_compound) {
            match upgrade_item(item, old, report) {
                Some(i) => out.insert(key.into(), Tag::Compound(i)),
                None => out.shift_remove(key),
            };
        }
    }
    if let Some(entity) = be.get("EntityId").and_then(Tag::as_str) {
        let modern = legacy().entity(entity).unwrap_or(entity);
        let mut data = Compound::new();
        data.insert("id".into(), Tag::String(modern.into()));
        out.insert("SpawnData".into(), Tag::Compound(data));
    }
    out
}

// ---------------------------------------------------------------------- entities

/// Upgrades an entity: its id (`Pig`, `EntityHorse`) and any items it carries.
pub(crate) fn upgrade_entity(e: &Compound, old: bool, report: &mut EditionReport) -> Compound {
    let mut out = e.clone();
    if let Some(id) = e.get("id").and_then(Tag::as_str) {
        if !id.starts_with("minecraft:") {
            if let Some(modern) = legacy().entity(id) {
                out.insert("id".into(), Tag::String(modern.into()));
            }
        }
    }
    for key in ["Items", "Inventory"] {
        if let Some(items) = e.get(key) {
            out.insert(key.into(), upgrade_items(items, old, report));
        }
    }
    if let Some(item) = e.get("Item").and_then(Tag::as_compound) {
        match upgrade_item(item, old, report) {
            Some(i) => out.insert("Item".into(), Tag::Compound(i)),
            None => out.shift_remove("Item"),
        };
    }
    if old {
        let id = out.get("id").and_then(Tag::as_str).unwrap_or_default();
        let facing = e.get("Facing").and_then(Tag::as_i64).or_else(|| e.get("Direction").and_then(Tag::as_i64));
        match (id, facing) {
            // Item frames' horizontal facing (south, west, north, east) became a 3D one.
            ("minecraft:item_frame", Some(f)) => {
                out.insert("Facing".into(), Tag::Byte([3, 4, 2, 5][f.rem_euclid(4) as usize]));
            }
            ("minecraft:painting", _) => {
                if let Some(m) = e
                    .get("Motive")
                    .and_then(Tag::as_str)
                    .and_then(mctk_chunker::entity::motive_from_bedrock)
                {
                    out.insert("variant".into(), Tag::String(m.0.into()));
                }
                if let Some(f) = facing {
                    out.insert("facing".into(), Tag::Byte(f.rem_euclid(4) as i8));
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Tag {
        Tag::String(v.into())
    }

    #[test]
    fn old_items_become_components() {
        let mut ench = Compound::new();
        ench.insert("id".into(), Tag::Short(16));
        ench.insert("lvl".into(), Tag::Short(5));
        let mut display = Compound::new();
        display.insert("Name".into(), s("Slayer"));
        let mut tag = Compound::new();
        tag.insert("ench".into(), Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(ench)]).unwrap()));
        tag.insert("display".into(), Tag::Compound(display));
        let mut item = Compound::new();
        item.insert("id".into(), Tag::Short(276));
        item.insert("Damage".into(), Tag::Short(12));
        item.insert("Count".into(), Tag::Byte(1));
        item.insert("Slot".into(), Tag::Byte(4));
        item.insert("tag".into(), Tag::Compound(tag));
        let mut report = EditionReport::default();
        let up = upgrade_item(&item, true, &mut report).unwrap();
        assert_eq!(up.get("id").and_then(Tag::as_str), Some("minecraft:diamond_sword"));
        assert_eq!(up.get("count"), Some(&Tag::Int(1)));
        assert_eq!(up.get("Slot"), Some(&Tag::Byte(4)));
        let c = up.get("components").and_then(Tag::as_compound).unwrap();
        assert_eq!(c.get("minecraft:damage"), Some(&Tag::Int(12)));
        assert_eq!(c.get("minecraft:custom_name").and_then(Tag::as_str), Some("Slayer"));
        let e = c.get("minecraft:enchantments").and_then(Tag::as_compound).unwrap();
        assert_eq!(e.get("minecraft:sharpness"), Some(&Tag::Int(5)));
        // The upgraded item reads with the current codec.
        let read = mctk_chunker::item::java::read(&up, &mut mctk_chunker::Notes::default()).unwrap();
        assert_eq!(read.props.damage, Some(12));
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn variants_use_the_damage_value_not_wear() {
        let mut item = Compound::new();
        item.insert("id".into(), s("minecraft:wool"));
        item.insert("Damage".into(), Tag::Short(5));
        item.insert("Count".into(), Tag::Byte(3));
        let up = upgrade_item(&item, true, &mut EditionReport::default()).unwrap();
        assert_eq!(up.get("id").and_then(Tag::as_str), Some("minecraft:lime_wool"));
        assert!(up.get("components").is_none());
    }

    #[test]
    fn shulker_contents_and_spawn_eggs_survive() {
        let mut inner = Compound::new();
        inner.insert("id".into(), s("minecraft:diamond"));
        inner.insert("Count".into(), Tag::Byte(5));
        inner.insert("Slot".into(), Tag::Byte(2));
        let mut bet = Compound::new();
        bet.insert("Items".into(), Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(inner)]).unwrap()));
        let mut tag = Compound::new();
        tag.insert("BlockEntityTag".into(), Tag::Compound(bet));
        let mut shulker = Compound::new();
        shulker.insert("id".into(), s("minecraft:shulker_box"));
        shulker.insert("Count".into(), Tag::Byte(1));
        shulker.insert("tag".into(), Tag::Compound(tag));
        let mut report = EditionReport::default();
        let up = upgrade_item(&shulker, false, &mut report).unwrap();
        let read = mctk_chunker::item::java::read(&up, &mut mctk_chunker::Notes::default()).unwrap();
        let held = read.props.container.as_ref().expect("contents");
        let (slot, stack) = &held[0];
        assert_eq!((*slot, stack.id.as_str(), stack.count), (2, "minecraft:diamond", 5));

        let mut entity = Compound::new();
        entity.insert("id".into(), s("minecraft:creeper"));
        let mut tag = Compound::new();
        tag.insert("EntityTag".into(), Tag::Compound(entity));
        let mut egg = Compound::new();
        egg.insert("id".into(), s("minecraft:creeper_spawn_egg"));
        egg.insert("Count".into(), Tag::Byte(1));
        egg.insert("tag".into(), Tag::Compound(tag));
        let up = upgrade_item(&egg, false, &mut report).unwrap();
        let read = mctk_chunker::item::java::read(&up, &mut mctk_chunker::Notes::default()).unwrap();
        assert_eq!(read.props.spawn_egg.as_deref(), Some("minecraft:creeper"));
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    }

    #[test]
    fn current_items_are_left_alone() {
        let mut item = Compound::new();
        item.insert("id".into(), s("minecraft:diamond"));
        item.insert("count".into(), Tag::Int(2));
        item.insert("components".into(), Tag::Compound(Compound::new()));
        assert_eq!(upgrade_item(&item, false, &mut EditionReport::default()), Some(item));
    }

    #[test]
    fn flattened_items_keep_their_names() {
        let mut tag = Compound::new();
        tag.insert("Damage".into(), Tag::Int(30));
        let mut item = Compound::new();
        item.insert("id".into(), s("minecraft:iron_pickaxe"));
        item.insert("Count".into(), Tag::Byte(1));
        item.insert("tag".into(), Tag::Compound(tag));
        let up = upgrade_item(&item, false, &mut EditionReport::default()).unwrap();
        assert_eq!(up.get("id").and_then(Tag::as_str), Some("minecraft:iron_pickaxe"));
        let c = up.get("components").and_then(Tag::as_compound).unwrap();
        assert_eq!(c.get("minecraft:damage"), Some(&Tag::Int(30)));
    }

    #[test]
    fn old_block_entities_and_entities_are_renamed() {
        let mut spawner = Compound::new();
        spawner.insert("id".into(), s("MobSpawner"));
        spawner.insert("EntityId".into(), s("Pig"));
        let mut report = EditionReport::default();
        let up = upgrade_block_entity(&spawner, true, &mut report);
        assert_eq!(up.get("id").and_then(Tag::as_str), Some("minecraft:mob_spawner"));
        let data = up.get("SpawnData").and_then(Tag::as_compound).unwrap();
        assert_eq!(data.get("id").and_then(Tag::as_str), Some("minecraft:pig"));

        let mut cow = Compound::new();
        cow.insert("id".into(), s("EntityHorse"));
        let up = upgrade_entity(&cow, true, &mut report);
        assert_eq!(up.get("id").and_then(Tag::as_str), Some("minecraft:horse"));
    }

    #[test]
    fn doors_combine_their_halves() {
        // Lower half facing south, open; upper half hinged right, unpowered.
        let lower = resolve_block(64, 1 | 4, 8 | 1).unwrap();
        let upper = resolve_block(64, 8 | 1, 1 | 4).unwrap();
        assert_eq!(lower.properties.get("half").map(String::as_str), Some("lower"));
        assert_eq!(upper.properties.get("half").map(String::as_str), Some("upper"));
        for k in ["facing", "open"] {
            assert_eq!(lower.properties.get(k), upper.properties.get(k), "{k}");
        }
        assert_eq!(upper.properties.get("open").map(String::as_str), Some("true"));
        assert_eq!(lower.properties.get("hinge"), upper.properties.get("hinge"));
    }

    #[test]
    fn biomes_follow_their_ids() {
        let b = Biomes {
            ids: (0..256).map(|i| if i % 16 < 8 { 4 } else { 2 }).collect(),
            columns: true,
            bedrock: false,
        };
        let c = section_biomes(&b, 0, &mut EditionReport::default());
        assert_eq!(c.palette[c.indices[0] as usize], "minecraft:forest");
        assert_eq!(c.palette[c.indices[3] as usize], "minecraft:desert");
    }

    #[test]
    fn old_block_names_are_renamed_only_when_needed() {
        let mut path = BlockState::new("minecraft:grass_path");
        rename_old_blocks(&mut path);
        assert_eq!(path.name, "minecraft:dirt_path");
        let mut stone = BlockState::new("minecraft:stone");
        rename_old_blocks(&mut stone);
        assert_eq!(stone.name, "minecraft:stone");
    }

    #[test]
    fn old_bedrock_numeric_blocks_follow_bedrocks_own_data_values() {
        // Bedrock's hopper bit 8 means "enabled"; Java's data bit 8 means "disabled".
        let pe = bedrock_legacy_block(154, 8).unwrap();
        let java = legacy().block(154, 8).unwrap();
        assert_eq!(pe.properties.get("enabled").map(String::as_str), Some("true"));
        assert_eq!(java.properties.get("enabled").map(String::as_str), Some("false"));
        assert_eq!(bedrock_legacy_block(35, 5).unwrap().name, "minecraft:lime_wool");
        // Ids Bedrock numbered differently are not guessed at.
        assert!(bedrock_legacy_block(241, 0).is_none());
        assert!(bedrock_legacy_block(95, 0).is_none());
    }
}