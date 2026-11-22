//! The conversions a viewer needs to show a Bedrock or console world as Java would, one chunk
//! at a time and without writing anything: the same code the world converters use, with
//! their reports thrown away.

use mctk_console::ConsoleChunk;
use mctk_java::{BlockState, PalettedContainer};
use mctk_nbt::{Compound, NamedTag, Tag};

use crate::java_bedrock::EditionReport;

/// A console chunk as a current Java chunk (block states worked out from their neighbours,
/// block entities and entities upgraded), with its entities. `dimension` is the console
/// dimension number (0, -1 the Nether, 1 the End).
pub fn console_chunk(chunk: &ConsoleChunk, dimension: i32) -> Option<(NamedTag, Vec<Tag>)> {
    static REG: std::sync::OnceLock<Option<mctk_registry::Registry>> = std::sync::OnceLock::new();
    let reg = REG.get_or_init(|| mctk_registry::builtin::java(mctk_registry::builtin::java_versions()[0])).as_ref()?;
    let dv = reg.data_version?;
    let mut notes = crate::java_items::Notes::default();
    let c = crate::legacy_up::chunk(chunk, dimension, dv, reg, &mut notes).ok()?;
    Some((c.chunk, c.entities))
}

/// What a Bedrock block entity is in Java: its data (`None` for kinds Java keeps in the block,
/// such as beds and flower pots) and the change it makes to its block (`block` is the Java
/// block it sits in): a bed's colour, a chest's half, a skull's kind, a pot's plant.
pub struct BlockEntity {
    pub nbt: Option<Compound>,
    pub rename: Option<String>,
    pub props: Vec<(&'static str, Option<String>)>,
}

impl BlockEntity {
    /// The block it sits in, changed as the block entity says.
    pub fn apply(&self, state: &mut BlockState) {
        if let Some(n) = &self.rename {
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

/// A Bedrock block entity in Java form (`None` for kinds Java does not have).
pub fn bedrock_block_entity(be: &Compound, block: Option<&BlockState>) -> Option<BlockEntity> {
    let mut report = EditionReport::default();
    let j = crate::block_entity::bedrock_to_java(be, (0, 0), block, &mut report)?;
    Some(BlockEntity { nbt: j.nbt, rename: j.edit.name, props: j.edit.props })
}

/// A Bedrock actor (mob, item, painting, boat...) as a Java entity.
pub fn bedrock_actor(actor: &Compound) -> Option<Compound> {
    let mut report = EditionReport::default();
    crate::entity::actor_to_java(actor, (0, 0), &mut report)
}

/// Whether a Bedrock block is an item frame (Java's frames are entities, not blocks).
pub fn is_frame_block(name: &str) -> bool {
    crate::frames::is_frame_block(name)
}

/// The Java item frame entity for a Bedrock frame block at `tile` (world block position),
/// with the block entity holding its item when there is one.
pub fn bedrock_frame(name: &str, states: &Compound, tile: (i32, i32, i32), held: Option<&Compound>) -> Compound {
    let mut notes = crate::item_codec::Notes::default();
    let frame = crate::frames::Frame::from_bedrock(name, states, tile, held, &mut notes);
    frame.to_java([0x4D43_544Bu32 as i32, tile.0, tile.1, tile.2])
}

/// The current Java id (`minecraft:ender_chest`) of a block entity of any edition, from its saved
/// data: Java's own, old Java's and the console editions' (`Chest`, `Trap`, `RecordPlayer`) or
/// Bedrock's (`EnderChest`, `Music`).
pub fn java_block_entity_id(be: &Compound) -> Option<String> {
    let id = be.get("id").and_then(Tag::as_str)?;
    if id.contains(':') {
        return Some(id.to_string());
    }
    // `Cauldron` was the brewing stand before 1.11 (it brews); in Bedrock it is the cauldron.
    if id == "Cauldron" {
        return Some(if be.contains_key("BrewTime") { "minecraft:brewing_stand" } else { "minecraft:cauldron" }.into());
    }
    if let Some(old) = crate::legacy_up::block_entity_name(id) {
        return Some(old);
    }
    let bedrock = match id {
        "EnchantTable" => "enchanting_table",
        "Music" => "note_block",
        "PistonArm" => "piston",
        "MovingBlock" => "moving_piston",
        "NetherReactor" => "nether_reactor_core",
        other => return Some(format!("minecraft:{}", snake(other))),
    };
    Some(format!("minecraft:{bedrock}"))
}

/// The current Java id (`minecraft:zombified_piglin`) of an entity of any edition, from its saved
/// data: Java's own `id`, old Java's and the console editions' (`Pig`, `EntityHorse`), or
/// Bedrock's `identifier` (`minecraft:zombie_pigman`).
pub fn java_entity_id(e: &Compound) -> Option<String> {
    if let Some(b) = e.get("identifier").and_then(Tag::as_str) {
        return crate::entity::java_entity_id_of(b).or_else(|| Some(b.to_string()));
    }
    let id = e.get("id").and_then(Tag::as_str)?;
    if id.contains(':') {
        // the current console games use 1.12's names (`minecraft:zombie_pigman`)
        return Some(crate::legacy_up::entity_id(id).unwrap_or_else(|| id.to_string()));
    }
    crate::legacy_up::entity_id(id)
        .or_else(|| mctk_mapping::legacy::legacy().entity(id).map(str::to_string))
        .or_else(|| Some(format!("minecraft:{}", snake(id))))
}

/// `EnderChest` as `ender_chest`.
fn snake(camel: &str) -> String {
    let mut out = String::new();
    for (i, c) in camel.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A Bedrock subchunk from before palettes (ids and data values) as Java block states.
pub fn bedrock_legacy_subchunk(body: &[u8]) -> PalettedContainer<BlockState> {
    crate::legacy::bedrock_legacy_subchunk(body, &mut EditionReport::default())
}

/// A Bedrock chunk from before subchunks (`LegacyTerrain`, 128 high) as Java sections.
pub fn bedrock_legacy_terrain(body: &[u8]) -> Vec<(i8, PalettedContainer<BlockState>)> {
    crate::legacy::bedrock_legacy_terrain(body, &mut EditionReport::default())
}
