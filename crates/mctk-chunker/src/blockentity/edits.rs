//! Block entities that depend on the block they sit in, and the changes they make to it.
//!
//! Chunker does this in `updateBeforeWrite` (Java → Bedrock: fill the entity from the
//! block) and `updateBeforeProcess` (Bedrock → Java: fix the block from the entity):
//! double chests and trapped chests, skull types and rotation, lecterns holding a book and
//! jukeboxes holding a record, and the block a brushable block is made of.

use mctk_java::BlockState;

use super::{BedrockOnly, BlockEntity, Kind, chest};

/// Skull blocks and the Bedrock `SkullType` each has, as (floor block, wall block, type).
/// Copper golem statue poses in Bedrock\u0027s numbering.
const POSES: [&str; 4] = ["standing", "sitting", "running", "star"];

/// Skull blocks and the Bedrock `SkullType` each has, as (floor block, wall block, type).
const SKULLS: [(&str, &str, i8); 7] = [
    ("minecraft:skeleton_skull", "minecraft:skeleton_wall_skull", 0),
    ("minecraft:wither_skeleton_skull", "minecraft:wither_skeleton_wall_skull", 1),
    ("minecraft:zombie_head", "minecraft:zombie_wall_head", 2),
    ("minecraft:player_head", "minecraft:player_wall_head", 3),
    ("minecraft:creeper_head", "minecraft:creeper_wall_head", 4),
    ("minecraft:dragon_head", "minecraft:dragon_wall_head", 5),
    ("minecraft:piglin_head", "minecraft:piglin_wall_head", 6),
];

/// What converting a block entity changes about its block (Bedrock → Java).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlockEdit {
    /// A different block name (a skull's kind comes from the entity on Bedrock).
    pub name: Option<String>,
    /// Properties to set; `None` removes the property.
    pub props: Vec<(&'static str, Option<String>)>,
}

impl BlockEdit {
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.props.is_empty()
    }

    /// Applies the edit to a block state. Defaults are not stripped.
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

fn is_wall(name: &str) -> bool {
    name.contains("_wall_")
}

/// The dye id (Java order) of a coloured block named `minecraft:<dye>_<suffix>`.
fn dye_of(block: &str, suffix: &str) -> Option<u8> {
    let colour = block.strip_prefix("minecraft:")?.strip_suffix(suffix)?;
    crate::ids::DYES.iter().position(|(n, _, _)| *n == colour).map(|i| i as u8)
}

fn dye_name(id: u8) -> &'static str {
    crate::ids::DYES[(id as usize).min(15)].0
}

/// Potted plants whose block name is not `potted_<plant>`.
const POT_EXCEPTIONS: [(&str, &str); 2] = [
    ("minecraft:azalea", "minecraft:potted_azalea_bush"),
    ("minecraft:flowering_azalea", "minecraft:potted_flowering_azalea_bush"),
];

fn potted_block(plant: &str) -> String {
    POT_EXCEPTIONS
        .iter()
        .find(|(p, _)| *p == plant)
        .map_or_else(
            || format!("minecraft:potted_{}", plant.trim_start_matches("minecraft:")),
            |(_, b)| b.to_string(),
        )
}

fn plant_of(potted: &str) -> Option<String> {
    if potted == "minecraft:flower_pot" {
        return None;
    }
    POT_EXCEPTIONS
        .iter()
        .find(|(_, b)| *b == potted)
        .map(|(p, _)| p.to_string())
        .or_else(|| potted.strip_prefix("minecraft:potted_").map(|p| format!("minecraft:{p}")))
}

/// Java → Bedrock: the entity Bedrock needs for a block that has none on Java (a bed's
/// colour, a cauldron, a potted plant).
pub fn from_block(block: &BlockState) -> Option<Kind> {
    if let Some(color) = dye_of(&block.name, "_bed") {
        return Some(Kind::Bed { color });
    }
    match block.name.as_str() {
        "minecraft:cauldron"
        | "minecraft:water_cauldron"
        | "minecraft:lava_cauldron"
        | "minecraft:powder_snow_cauldron" => Some(Kind::Cauldron(Default::default())),
        "minecraft:flower_pot" => Some(Kind::FlowerPot { plant: None }),
        "minecraft:note_block" => Some(Kind::NoteBlock {
            note: block.properties.get("note").and_then(|n| n.parse().ok()).unwrap_or(0),
        }),
        "minecraft:piston" | "minecraft:sticky_piston" => Some(Kind::PistonArm {
            sticky: block.name == "minecraft:sticky_piston",
            extended: block.properties.get("extended").is_some_and(|e| e == "true"),
        }),
        "minecraft:lodestone" => Some(Kind::BedrockOnly(BedrockOnly::Lodestone)),
        "minecraft:spore_blossom" => Some(Kind::BedrockOnly(BedrockOnly::SporeBlossom)),
        n if n.starts_with("minecraft:potted_") => Some(Kind::FlowerPot { plant: plant_of(n) }),
        _ => None,
    }
}

/// Whether [`from_block`] can make an entity for a block of this name (a cheap check for
/// scanning palettes).
pub fn generates_entity(name: &str) -> bool {
    name.ends_with("_bed")
        || name.ends_with("cauldron")
        || name == "minecraft:flower_pot"
        || name.starts_with("minecraft:potted_")
        || matches!(
            name,
            "minecraft:note_block"
                | "minecraft:piston"
                | "minecraft:sticky_piston"
                | "minecraft:lodestone"
                | "minecraft:spore_blossom"
        )
}

/// Java → Bedrock: fills what Bedrock keeps in the entity from the block (with default
/// properties filled in).
pub fn prepare_for_bedrock(be: &mut BlockEntity, block: &BlockState) {
    chest::prepare_for_bedrock(be, block);
    match &mut be.kind {
        Kind::Brushable(b) => b.block = Some(block.name.clone()),
        Kind::CopperGolemStatue { pose } => {
            *pose = block
                .properties
                .get("copper_golem_pose")
                .and_then(|p| POSES.iter().position(|n| n == p))
                .unwrap_or(0) as u8;
        }
        Kind::Banner(b) => {
            b.base = dye_of(&block.name, "_wall_banner").or_else(|| dye_of(&block.name, "_banner"));
        }
        Kind::Skull(s) => {
            s.skull_type = SKULLS
                .iter()
                .find(|(f, w, _)| *f == block.name || *w == block.name)
                .map_or(0, |(_, _, t)| *t);
            if let Some(r) = block.properties.get("rotation").and_then(|r| r.parse::<i8>().ok()) {
                s.rotation = r;
            }
            s.moving_mouth = block.properties.get("powered").is_some_and(|p| p == "true");
        }
        _ => {}
    }
}

/// Bedrock → Java: fixes the Java block from the entity and returns the changes to make.
pub fn resolve_for_java(be: &mut BlockEntity, block: &BlockState) -> BlockEdit {
    let mut edit = BlockEdit::default();
    if let Some(ty) = chest::resolve_for_java(be, block) {
        edit.props.push(("type", Some(ty.to_string())));
    }
    // Banners and beds: Bedrock says the colour, Java's block name carries it.
    if let Kind::Banner(b) = &mut be.kind {
        let wall = block.name.contains("_wall_banner");
        if let (Some(base), true) = (b.base.take(), block.name.ends_with("banner")) {
            edit.name = Some(format!(
                "minecraft:{}_{}",
                dye_name(base),
                if wall { "wall_banner" } else { "banner" }
            ));
        }
    }
    match &be.kind {
        Kind::Bed { color } if block.name.ends_with("_bed") => {
            edit.name = Some(format!("minecraft:{}_bed", dye_name(*color)));
        }
        Kind::NoteBlock { note } if block.name == "minecraft:note_block" => {
            edit.props.push(("note", Some(note.to_string())));
        }
        Kind::PistonArm { extended, .. }
            if matches!(block.name.as_str(), "minecraft:piston" | "minecraft:sticky_piston") =>
        {
            edit.props.push(("extended", Some(extended.to_string())));
        }
        Kind::CopperGolemStatue { pose } if block.name.contains("copper_golem_statue") => {
            edit.props.push(("copper_golem_pose", Some(POSES[(*pose as usize).min(3)].to_string())));
        }
        Kind::FlowerPot { plant }
            if block.name == "minecraft:flower_pot" || block.name.starts_with("minecraft:potted_") =>
        {
            edit.name = Some(match plant {
                Some(p) => potted_block(p),
                None => "minecraft:flower_pot".to_string(),
            });
        }
        Kind::Lectern { book, .. } if block.name == "minecraft:lectern" => {
            let has = book.as_ref().is_some_and(|b| !b.is_air());
            edit.props.push(("has_book", Some(has.to_string())));
        }
        Kind::Jukebox { record } if block.name == "minecraft:jukebox" => {
            let has = record.as_ref().is_some_and(|r| !r.is_air());
            edit.props.push(("has_record", Some(has.to_string())));
        }
        Kind::Skull(s) if SKULLS.iter().any(|(f, w, _)| *f == block.name || *w == block.name) => {
            let wall = is_wall(&block.name);
            if let Some((floor, wall_name, _)) = SKULLS.iter().find(|(_, _, t)| *t == s.skull_type) {
                edit.name = Some(if wall { *wall_name } else { *floor }.to_string());
            }
            if !wall {
                edit.props
                    .push(("rotation", Some(s.rotation.rem_euclid(16).to_string())));
            }
            edit.props.push(("powered", Some(s.moving_mouth.to_string())));
        }
        _ => {}
    }
    edit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockentity::Skull;

    #[test]
    fn skull_block_and_entity_agree_both_ways() {
        let block = BlockState::new("minecraft:wither_skeleton_skull")
            .with("rotation", "5")
            .with("powered", "true");
        let mut be = BlockEntity::new(0, 0, 0, Kind::Skull(Skull::default()));
        prepare_for_bedrock(&mut be, &block);
        let Kind::Skull(s) = &be.kind else { panic!() };
        assert_eq!((s.skull_type, s.rotation, s.moving_mouth), (1, 5, true));

        // Bedrock says it is a zombie head; the Java block follows.
        let Kind::Skull(s) = &mut be.kind else { panic!() };
        s.skull_type = 2;
        let mut state = block.clone();
        let edit = resolve_for_java(&mut be, &state);
        edit.apply(&mut state);
        assert_eq!(state.name, "minecraft:zombie_head");
        assert_eq!(state.properties.get("rotation").map(String::as_str), Some("5"));
    }

    #[test]
    fn wall_skulls_keep_their_wall_form_and_have_no_rotation() {
        let mut be = BlockEntity::new(0, 0, 0, Kind::Skull(Skull { skull_type: 3, ..Skull::default() }));
        let block = BlockState::new("minecraft:skeleton_wall_skull").with("facing", "east");
        let edit = resolve_for_java(&mut be, &block);
        assert_eq!(edit.name.as_deref(), Some("minecraft:player_wall_head"));
        assert!(edit.props.iter().all(|(k, _)| *k != "rotation"));
    }

    #[test]
    fn banner_colour_patterns_and_block_name() {
        use crate::{Notes, blockentity::{bedrock, java}};
        use mctk_nbt::{Compound, Tag};
        let mut p = Compound::new();
        p.insert("color".into(), Tag::String("blue".into()));
        p.insert("pattern".into(), Tag::String("minecraft:creeper".into()));
        let mut j = Compound::new();
        for (k, v) in [("x", 1), ("y", 2), ("z", 3)] {
            j.insert(k.into(), Tag::Int(v));
        }
        j.insert("id".into(), Tag::String("minecraft:banner".into()));
        j.insert("patterns".into(), crate::compound_list(vec![p]));
        let mut notes = Notes::default();
        let mut be = java::read(&j, &mut notes).unwrap();
        prepare_for_bedrock(&mut be, &BlockState::new("minecraft:red_wall_banner").with("facing", "north"));
        let b = bedrock::write(&be, &mut notes);
        // Bedrock numbers dyes in reverse: red is 14 in Java, 1 in Bedrock; blue 11 → 4.
        assert_eq!(b.get("Base"), Some(&Tag::Int(1)));
        let pat = b.get("Patterns").and_then(Tag::as_list).unwrap().get(0).unwrap();
        assert_eq!(pat.get("Pattern").and_then(Tag::as_str), Some("cre"));
        assert_eq!(pat.get("Color"), Some(&Tag::Int(4)));

        // Coming back, the base colour becomes the Java block.
        let mut back = bedrock::read(&b, &mut notes).unwrap();
        let block = BlockState::new("minecraft:white_wall_banner").with("facing", "north");
        let edit = resolve_for_java(&mut back, &block);
        assert_eq!(edit.name.as_deref(), Some("minecraft:red_wall_banner"));
        let out = java::write(&back, &mut notes).unwrap();
        assert!(out.get("Base").is_none());
        assert_eq!(
            out.get("patterns").and_then(Tag::as_list).unwrap().get(0).unwrap().get("color").and_then(Tag::as_str),
            Some("blue")
        );
    }

    #[test]
    fn beds_cauldrons_and_flower_pots_are_made_from_blocks() {
        assert_eq!(from_block(&BlockState::new("minecraft:green_bed")), Some(Kind::Bed { color: 13 }));
        assert!(matches!(from_block(&BlockState::new("minecraft:water_cauldron")), Some(Kind::Cauldron(_))));
        assert_eq!(
            from_block(&BlockState::new("minecraft:potted_azalea_bush")),
            Some(Kind::FlowerPot { plant: Some("minecraft:azalea".into()) })
        );
        assert_eq!(
            from_block(&BlockState::new("minecraft:potted_cactus")),
            Some(Kind::FlowerPot { plant: Some("minecraft:cactus".into()) })
        );
        assert_eq!(from_block(&BlockState::new("minecraft:stone")), None);
        assert!(generates_entity("minecraft:potted_fern") && !generates_entity("minecraft:chest"));

        // A Bedrock bed or pot rewrites the Java block it sits in.
        let mut bed = BlockEntity::new(0, 0, 0, Kind::Bed { color: 3 });
        let e = resolve_for_java(&mut bed, &BlockState::new("minecraft:red_bed").with("part", "head"));
        assert_eq!(e.name.as_deref(), Some("minecraft:light_blue_bed"));
        let mut pot = BlockEntity::new(0, 0, 0, Kind::FlowerPot { plant: Some("minecraft:flowering_azalea".into()) });
        let e = resolve_for_java(&mut pot, &BlockState::new("minecraft:flower_pot"));
        assert_eq!(e.name.as_deref(), Some("minecraft:potted_flowering_azalea_bush"));
    }

    #[test]
    fn flower_pot_plant_goes_through_bedrock_block_names() {
        use crate::{Notes, blockentity::bedrock};
        let be = BlockEntity::new(0, 0, 0, Kind::FlowerPot { plant: Some("minecraft:cactus".into()) });
        let mut notes = Notes::default();
        let b = bedrock::write(&be, &mut notes);
        assert_eq!(
            b.get("PlantBlock").and_then(|p| p.get("name")).and_then(mctk_nbt::Tag::as_str),
            Some("minecraft:cactus")
        );
        let back = bedrock::read(&b, &mut notes).unwrap();
        assert_eq!(back.kind, Kind::FlowerPot { plant: Some("minecraft:cactus".into()) });
    }

    #[test]
    fn note_blocks_pistons_and_statues_follow_their_blocks() {
        assert_eq!(
            from_block(&BlockState::new("minecraft:note_block").with("note", "12")),
            Some(Kind::NoteBlock { note: 12 })
        );
        assert_eq!(
            from_block(&BlockState::new("minecraft:sticky_piston").with("extended", "true")),
            Some(Kind::PistonArm { sticky: true, extended: true })
        );
        assert!(matches!(
            from_block(&BlockState::new("minecraft:spore_blossom")),
            Some(Kind::BedrockOnly(BedrockOnly::SporeBlossom))
        ));

        let mut note = BlockEntity::new(0, 0, 0, Kind::NoteBlock { note: 7 });
        let e = resolve_for_java(&mut note, &BlockState::new("minecraft:note_block"));
        assert_eq!(e.props, vec![("note", Some("7".to_string()))]);
        let mut piston = BlockEntity::new(0, 0, 0, Kind::PistonArm { sticky: false, extended: true });
        let e = resolve_for_java(&mut piston, &BlockState::new("minecraft:piston"));
        assert_eq!(e.props, vec![("extended", Some("true".to_string()))]);

        // A statue's pose goes both ways through the block.
        let statue_block = BlockState::new("minecraft:copper_golem_statue").with("copper_golem_pose", "running");
        let mut statue = BlockEntity::new(0, 0, 0, Kind::CopperGolemStatue { pose: 0 });
        prepare_for_bedrock(&mut statue, &statue_block);
        assert_eq!(statue.kind, Kind::CopperGolemStatue { pose: 2 });
        let e = resolve_for_java(&mut statue, &BlockState::new("minecraft:copper_golem_statue"));
        assert_eq!(e.props, vec![("copper_golem_pose", Some("running".to_string()))]);
    }

    #[test]
    fn lecterns_and_jukeboxes_set_their_block_flags() {
        let mut lectern = BlockEntity::new(0, 0, 0, Kind::Lectern { book: None, page: 0 });
        let edit = resolve_for_java(&mut lectern, &BlockState::new("minecraft:lectern"));
        assert_eq!(edit.props, vec![("has_book", Some("false".to_string()))]);
        let mut jukebox = BlockEntity::new(
            0,
            0,
            0,
            Kind::Jukebox { record: Some(crate::item::ItemStack::new("minecraft:music_disc_cat", 1)) },
        );
        let edit = resolve_for_java(&mut jukebox, &BlockState::new("minecraft:jukebox"));
        assert_eq!(edit.props, vec![("has_record", Some("true".to_string()))]);
    }
}

