//! Block entities: the edition-neutral model and the Java / Bedrock codecs. Ported from
//! Chunker's `intermediate/column/blockentity` model and its `*BlockEntityHandler` classes
//! (MIT, Hive Games).
//!
//! Chunker chains handlers from the base class down: every entity has a position, a
//! container adds items, a "randomizable" container adds a loot table, and the specific
//! kind adds its own fields. Here that is [`BlockEntity`] (position, name) around a
//! [`Kind`]. Kinds that carry no data Chunker can convert are written as bare entities
//! (just the id and position) and the game fills in the defaults.

pub mod bedrock;
pub mod chest;
pub mod edits;
pub mod java;

use std::collections::BTreeMap;

use crate::item::ItemStack;
use crate::text::Text;

#[derive(Debug, Clone, PartialEq)]
pub struct BlockEntity {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Bedrock `isMovable` (can be moved by pistons).
    pub movable: bool,
    pub custom_name: Option<Text>,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Container(Container),
    Sign(Sign),
    Spawner(Spawner),
    Comparator { output_signal: i32 },
    SculkSensor { calibrated: bool },
    Campfire(Campfire),
    DecoratedPot(DecoratedPot),
    Brushable(Brushable),
    Lectern { book: Option<ItemStack>, page: i32 },
    /// Bedrock keeps an item frame's item in a block entity; Java has an entity instead
    /// (see [`crate::frame`]).
    ItemFrame(ItemFrame),
    Jukebox { record: Option<ItemStack> },
    Skull(Skull),
    Banner(Banner),
    /// Bedrock keeps a bed's colour in the entity; Java has it in the block.
    Bed { color: u8 },
    Cauldron(Cauldron),
    /// Bedrock keeps the potted plant in the entity; Java has a block per plant.
    FlowerPot { plant: Option<String> },
    Beacon { primary: Option<crate::ids::Effect>, secondary: Option<crate::ids::Effect> },
    CommandBlock(CommandBlock),
    EndGateway { age: i64, exit: Option<[i32; 3]> },
    ChiseledBookshelf { books: [Option<ItemStack>; 6] },
    Shelf { items: [Option<ItemStack>; 3] },
    Jigsaw(Jigsaw),
    Structure(Box<StructureBlock>),
    /// Pose 0..=3 (standing, sitting, running, star); Java keeps it in the block.
    CopperGolemStatue { pose: u8 },
    /// Bedrock note block entity (the note is a block property on Java).
    NoteBlock { note: u8 },
    /// Bedrock piston arm entity (Java has only the block).
    PistonArm { sticky: bool, extended: bool },
    /// Bedrock-only entities without data, made from their blocks.
    BedrockOnly(BedrockOnly),
    /// Entities that are dropped without a trace (Java's moving piston, Bedrock's moving
    /// block).
    Discard,
    /// Kinds without converted data.
    Empty(EmptyKind),
}

impl BlockEntity {
    pub fn new(x: i32, y: i32, z: i32, kind: Kind) -> Self {
        BlockEntity {
            x,
            y,
            z,
            movable: false,
            custom_name: None,
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemFrame {
    pub glow: bool,
    pub item: Option<ItemStack>,
    /// Rotation of the item in eighths of a turn, 0..=7.
    pub rotation: u8,
}

// ---------------------------------------------------------------- containers

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerKind {
    Chest,
    TrappedChest,
    Barrel,
    Dispenser,
    Dropper,
    ShulkerBox,
    Hopper,
    EnderChest,
    Furnace,
    BlastFurnace,
    Smoker,
    BrewingStand,
    Crafter,
}

impl ContainerKind {
    pub const ALL: [ContainerKind; 13] = [
        ContainerKind::Chest,
        ContainerKind::TrappedChest,
        ContainerKind::Barrel,
        ContainerKind::Dispenser,
        ContainerKind::Dropper,
        ContainerKind::ShulkerBox,
        ContainerKind::Hopper,
        ContainerKind::EnderChest,
        ContainerKind::Furnace,
        ContainerKind::BlastFurnace,
        ContainerKind::Smoker,
        ContainerKind::BrewingStand,
        ContainerKind::Crafter,
    ];

    pub fn java_id(self) -> &'static str {
        match self {
            ContainerKind::Chest => "minecraft:chest",
            ContainerKind::TrappedChest => "minecraft:trapped_chest",
            ContainerKind::Barrel => "minecraft:barrel",
            ContainerKind::Dispenser => "minecraft:dispenser",
            ContainerKind::Dropper => "minecraft:dropper",
            ContainerKind::ShulkerBox => "minecraft:shulker_box",
            ContainerKind::Hopper => "minecraft:hopper",
            ContainerKind::EnderChest => "minecraft:ender_chest",
            ContainerKind::Furnace => "minecraft:furnace",
            ContainerKind::BlastFurnace => "minecraft:blast_furnace",
            ContainerKind::Smoker => "minecraft:smoker",
            ContainerKind::BrewingStand => "minecraft:brewing_stand",
            ContainerKind::Crafter => "minecraft:crafter",
        }
    }

    /// Bedrock has one `Chest` entity for chests and trapped chests.
    pub fn bedrock_id(self) -> &'static str {
        match self {
            ContainerKind::Chest | ContainerKind::TrappedChest => "Chest",
            ContainerKind::Barrel => "Barrel",
            ContainerKind::Dispenser => "Dispenser",
            ContainerKind::Dropper => "Dropper",
            ContainerKind::ShulkerBox => "ShulkerBox",
            ContainerKind::Hopper => "Hopper",
            ContainerKind::EnderChest => "EnderChest",
            ContainerKind::Furnace => "Furnace",
            ContainerKind::BlastFurnace => "BlastFurnace",
            ContainerKind::Smoker => "Smoker",
            ContainerKind::BrewingStand => "BrewingStand",
            ContainerKind::Crafter => "Crafter",
        }
    }

    /// Whether the container can hold a loot table (`RandomizableContainerBlockEntity`).
    pub fn randomizable(self) -> bool {
        matches!(
            self,
            ContainerKind::Chest
                | ContainerKind::TrappedChest
                | ContainerKind::Barrel
                | ContainerKind::Dispenser
                | ContainerKind::Dropper
                | ContainerKind::ShulkerBox
                | ContainerKind::Crafter
        )
    }

    pub fn is_furnace(self) -> bool {
        matches!(
            self,
            ContainerKind::Furnace | ContainerKind::BlastFurnace | ContainerKind::Smoker
        )
    }

    pub fn from_java(id: &str) -> Option<ContainerKind> {
        Self::ALL.iter().copied().find(|k| k.java_id() == id)
    }

    /// `Chest` is read as a plain chest; whether it is trapped follows from the block.
    pub fn from_bedrock(id: &str) -> Option<ContainerKind> {
        Self::ALL
            .iter()
            .copied()
            .find(|k| k.bedrock_id() == id && *k != ContainerKind::TrappedChest)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    pub kind: ContainerKind,
    /// Items by slot.
    pub items: BTreeMap<u8, ItemStack>,
    /// Loot table in Java spelling (`minecraft:chests/...`).
    pub loot_table: Option<String>,
    pub furnace: Furnace,
    pub brewing: Brewing,
    /// Crafter: bitmask of disabled slots and ticks left of the current craft.
    pub crafter: Crafter,
    /// Where a Bedrock chest says its other half is (Bedrock only; Java derives it from the
    /// block's `type`).
    pub chest_pair: Option<ChestPair>,
}

impl Container {
    pub fn new(kind: ContainerKind) -> Self {
        Container {
            kind,
            items: BTreeMap::new(),
            loot_table: None,
            furnace: Furnace::default(),
            brewing: Brewing::default(),
            crafter: Crafter::default(),
            chest_pair: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Furnace {
    pub burn_time: i16,
    pub cook_time: i16,
    pub cook_time_total: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brewing {
    pub brew_time: i16,
    pub fuel: i16,
    pub fuel_total: i16,
}

impl Default for Brewing {
    fn default() -> Self {
        Brewing { brew_time: 0, fuel: 0, fuel_total: 20 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Crafter {
    pub disabled_slots: u16,
    pub crafting_ticks_remaining: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChestPair {
    pub x: i32,
    pub z: i32,
    /// Bedrock marks one chest of a double chest as the lead (the right half).
    pub lead: bool,
}

// --------------------------------------------------------------------- signs

#[derive(Debug, Clone, PartialEq)]
pub struct Sign {
    pub hanging: bool,
    pub waxed: bool,
    pub allow_op_features: bool,
    pub front: SignFace,
    pub back: SignFace,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignFace {
    pub lines: Vec<Text>,
    /// Glowing text.
    pub lit: bool,
    /// Text colour as Bedrock's ARGB.
    pub color: i32,
}

impl Default for SignFace {
    fn default() -> Self {
        SignFace {
            lines: Vec::new(),
            lit: false,
            color: 0xFF00_0000u32 as i32,
        }
    }
}

impl Sign {
    pub fn new(hanging: bool) -> Self {
        Sign {
            hanging,
            waxed: false,
            allow_op_features: true,
            front: SignFace::default(),
            back: SignFace::default(),
        }
    }
}

// ------------------------------------------------------------------- spawners

#[derive(Debug, Clone, PartialEq)]
pub struct Spawner {
    pub delay: i16,
    pub spawn_range: i16,
    pub max_spawn_delay: i16,
    pub min_spawn_delay: i16,
    pub spawn_count: i16,
    pub required_player_range: i16,
    pub max_nearby_entities: i16,
    /// Java entity id of what it spawns.
    pub entity: Option<String>,
}

impl Default for Spawner {
    fn default() -> Self {
        Spawner {
            delay: 20,
            spawn_range: 4,
            max_spawn_delay: 800,
            min_spawn_delay: 200,
            spawn_count: 4,
            required_player_range: 16,
            max_nearby_entities: 6,
            entity: None,
        }
    }
}

// ------------------------------------------------------------------ campfires

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Campfire {
    pub items: [Option<ItemStack>; 4],
    pub cooking_times: [i32; 4],
    /// Java uses `minecraft:soul_campfire` for the soul variant; Bedrock `Campfire` for both.
    pub soul: bool,
}

// ----------------------------------------------------- pots, brushables, skulls

#[derive(Debug, Clone, PartialEq)]
pub struct DecoratedPot {
    /// Java item names of the back, left, right and front sherds (bricks when plain).
    pub sherds: [String; 4],
    pub item: Option<ItemStack>,
}

impl Default for DecoratedPot {
    fn default() -> Self {
        DecoratedPot {
            sherds: std::array::from_fn(|_| "minecraft:brick".to_string()),
            item: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Brushable {
    /// Java `hit_direction` / Bedrock `brush_direction` (Java's "none" is 6).
    pub brush_direction: i8,
    pub brush_count: i32,
    pub item: Option<ItemStack>,
    pub loot_table: Option<String>,
    /// Bedrock `type`: the block's name. Set from the block when writing Bedrock.
    pub block: Option<String>,
}

impl Default for Brushable {
    fn default() -> Self {
        Brushable {
            brush_direction: 6,
            brush_count: 0,
            item: None,
            loot_table: None,
            block: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skull {
    pub owner_id: Option<String>,
    pub owner_name: Option<String>,
    pub texture: Option<String>,
    pub texture_signature: Option<String>,
    /// Bedrock `SkullType`, `Rotation` (in 22.5° steps) and mouth animation; these come
    /// from the block when writing Bedrock.
    pub skull_type: i8,
    pub rotation: i8,
    pub moving_mouth: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Banner {
    /// Base colour as a dye id (Java order). Java only uses it for shields; Bedrock takes it
    /// from the block.
    pub base: Option<u8>,
    /// (dye id, index into [`crate::ids::BANNER_PATTERNS`]).
    pub patterns: Vec<(u8, usize)>,
}

/// A Bedrock cauldron's potion state (Java cauldrons hold only water, lava or snow, which
/// live in the block).
#[derive(Debug, Clone, PartialEq)]
pub struct Cauldron {
    pub potion: Option<crate::ids::Potion>,
    /// -1 unknown, 0 regular, 1 splash, 2 lingering.
    pub bottle: i16,
    pub custom_color: Option<i32>,
}

impl Default for Cauldron {
    fn default() -> Self {
        Cauldron {
            potion: None,
            bottle: -1,
            custom_color: None,
        }
    }
}

// ----------------------------------------------- command, jigsaw, structure

#[derive(Debug, Clone, PartialEq)]
pub struct CommandBlock {
    pub command: String,
    pub auto: bool,
    pub track_output: bool,
}

impl Default for CommandBlock {
    fn default() -> Self {
        CommandBlock { command: String::new(), auto: false, track_output: true }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Jigsaw {
    pub name: String,
    pub target: String,
    pub pool: String,
    pub final_state: String,
    /// "rollable" or "aligned".
    pub joint: String,
    pub selection_priority: i32,
    pub placement_priority: i32,
}

impl Default for Jigsaw {
    fn default() -> Self {
        Jigsaw {
            name: "minecraft:empty".into(),
            target: "minecraft:empty".into(),
            pool: "minecraft:empty".into(),
            final_state: "minecraft:air".into(),
            joint: "rollable".into(),
            selection_priority: 0,
            placement_priority: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureMode {
    Data,
    Save,
    Load,
    Corner,
    /// Bedrock only.
    Invalid,
    /// Bedrock only.
    Export,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructureBlock {
    pub name: String,
    pub author: String,
    pub metadata: String,
    pub pos: [i32; 3],
    pub size: [i32; 3],
    /// 0 none, 1 clockwise 90, 2 clockwise 180, 3 counterclockwise 90.
    pub rotation: u8,
    /// 0 none, 1 left-right, 2 front-back, 3 both (Bedrock only).
    pub mirror: u8,
    pub mode: StructureMode,
    pub ignore_entities: bool,
    pub include_players: bool,
    pub remove_blocks: bool,
    pub strict: bool,
    pub powered: bool,
    pub show_air: bool,
    pub show_bounding_box: bool,
    pub integrity: f32,
    pub seed: i64,
    pub last_touched_player: i64,
    pub animation_mode: i8,
    pub animation_seconds: f32,
    pub redstone_save_mode: i32,
}

impl Default for StructureBlock {
    fn default() -> Self {
        StructureBlock {
            name: String::new(),
            author: String::new(),
            metadata: String::new(),
            pos: [0; 3],
            size: [0; 3],
            rotation: 0,
            mirror: 0,
            mode: StructureMode::Data,
            ignore_entities: false,
            include_players: false,
            remove_blocks: false,
            strict: false,
            powered: false,
            show_air: false,
            show_bounding_box: true,
            integrity: 1.0,
            seed: 0,
            last_touched_player: 0,
            animation_mode: 0,
            animation_seconds: 0.0,
            redstone_save_mode: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BedrockOnly {
    Lodestone,
    SporeBlossom,
}

// ---------------------------------------------------------------- empty kinds

/// Block entities Chunker converts without any data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmptyKind {
    EndPortal,
    DaylightDetector,
    Conduit,
    Bell,
    Beehive,
    SculkCatalyst,
    SculkShrieker,
    TrialSpawner,
    Vault,
    CreakingHeart,
    PotentSulfur,
    EnchantmentTable,
}

impl EmptyKind {
    pub const ALL: [EmptyKind; 12] = [
        EmptyKind::EndPortal,
        EmptyKind::DaylightDetector,
        EmptyKind::Conduit,
        EmptyKind::Bell,
        EmptyKind::Beehive,
        EmptyKind::SculkCatalyst,
        EmptyKind::SculkShrieker,
        EmptyKind::TrialSpawner,
        EmptyKind::Vault,
        EmptyKind::CreakingHeart,
        EmptyKind::PotentSulfur,
        EmptyKind::EnchantmentTable,
    ];

    pub fn java_id(self) -> &'static str {
        match self {
            EmptyKind::EndPortal => "minecraft:end_portal",
            EmptyKind::DaylightDetector => "minecraft:daylight_detector",
            EmptyKind::Conduit => "minecraft:conduit",
            EmptyKind::Bell => "minecraft:bell",
            EmptyKind::Beehive => "minecraft:beehive",
            EmptyKind::SculkCatalyst => "minecraft:sculk_catalyst",
            EmptyKind::SculkShrieker => "minecraft:sculk_shrieker",
            EmptyKind::TrialSpawner => "minecraft:trial_spawner",
            EmptyKind::Vault => "minecraft:vault",
            EmptyKind::CreakingHeart => "minecraft:creaking_heart",
            EmptyKind::PotentSulfur => "minecraft:potent_sulfur",
            EmptyKind::EnchantmentTable => "minecraft:enchanting_table",
        }
    }

    pub fn bedrock_id(self) -> &'static str {
        match self {
            EmptyKind::EndPortal => "EndPortal",
            EmptyKind::DaylightDetector => "DaylightDetector",
            EmptyKind::Conduit => "Conduit",
            EmptyKind::Bell => "Bell",
            EmptyKind::Beehive => "Beehive",
            EmptyKind::SculkCatalyst => "SculkCatalyst",
            EmptyKind::SculkShrieker => "SculkShrieker",
            EmptyKind::TrialSpawner => "TrialSpawner",
            EmptyKind::Vault => "Vault",
            EmptyKind::CreakingHeart => "CreakingHeart",
            EmptyKind::PotentSulfur => "PotentSulfurBlock",
            EmptyKind::EnchantmentTable => "EnchantTable",
        }
    }

    pub fn from_java(id: &str) -> Option<EmptyKind> {
        Self::ALL.iter().copied().find(|k| k.java_id() == id)
    }

    pub fn from_bedrock(id: &str) -> Option<EmptyKind> {
        Self::ALL.iter().copied().find(|k| k.bedrock_id() == id)
    }
}

// ---------------------------------------------------------------- loot tables

/// Loot tables Bedrock named differently, as (Bedrock name, Java name).
const LOOT_RENAMES: [(&str, &str); 5] = [
    ("minecraft:chests/buriedtreasure", "minecraft:chests/buried_treasure"),
    ("minecraft:chests/dispenser_trap", "minecraft:chests/jungle_temple_dispenser"),
    ("minecraft:chests/shipwreck", "minecraft:chests/shipwreck_map"),
    ("minecraft:chests/shipwrecksupply", "minecraft:chests/shipwreck_supply"),
    ("minecraft:chests/shipwrecktreasure", "minecraft:chests/shipwreck_treasure"),
];

/// Loot tables only Bedrock has (they cannot be written to Java).
const BEDROCK_ONLY_TABLES: [&str; 2] = [
    "minecraft:chests/monster_room",
    "minecraft:chests/village_two_room_house",
];
/// Loot tables only Java has (they cannot be written to Bedrock).
const JAVA_ONLY_TABLES: [&str; 1] = ["minecraft:chests/village/village_fisher"];

/// A Bedrock loot table path as the neutral (Java) name.
pub fn loot_from_bedrock(path: &str) -> String {
    let name = path.replace("loot_tables/", "minecraft:").replace(".json", "");
    LOOT_RENAMES
        .iter()
        .find(|(b, _)| *b == name)
        .map_or(name, |(_, j)| j.to_string())
}

/// The Bedrock path for a neutral loot table (`None` when Bedrock has no such table).
pub fn loot_to_bedrock(name: &str) -> Option<String> {
    if JAVA_ONLY_TABLES.contains(&name) {
        return None;
    }
    let name = LOOT_RENAMES
        .iter()
        .find(|(_, j)| *j == name)
        .map_or(name, |(b, _)| b);
    let path = name.strip_prefix("minecraft:").unwrap_or(name);
    Some(format!("loot_tables/{path}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Notes, text};
    use mctk_nbt::{Compound, Tag};

    fn base(id: &str) -> Compound {
        let mut c = Compound::new();
        c.insert("id".into(), Tag::String(id.into()));
        for (k, v) in [("x", 5), ("y", 70), ("z", -9)] {
            c.insert(k.into(), Tag::Int(v));
        }
        c
    }

    /// Java → neutral → Bedrock → neutral → Java.
    fn through_bedrock(java: &Compound) -> (BlockEntity, Compound, Notes) {
        let mut notes = Notes::default();
        let neutral = java::read(java, &mut notes).expect("java reads");
        let bedrock_nbt = bedrock::write(&neutral, &mut notes);
        let back = bedrock::read(&bedrock_nbt, &mut notes).expect("bedrock reads");
        let java_again = java::write(&back, &mut notes).unwrap();
        (back, java_again, notes)
    }

    #[test]
    fn container_with_items_loot_table_and_name() {
        let mut j = base("minecraft:barrel");
        let mut item = Compound::new();
        item.insert("Slot".into(), Tag::Byte(4));
        item.insert("id".into(), Tag::String("minecraft:water_bucket".into()));
        item.insert("count".into(), Tag::Int(1));
        j.insert(
            "Items".into(),
            Tag::List(mctk_nbt::List::from_tags(mctk_nbt::TagType::Compound, vec![Tag::Compound(item)]).unwrap()),
        );
        j.insert("LootTable".into(), Tag::String("minecraft:chests/trial_chambers/corridor".into()));
        j.insert("CustomName".into(), Tag::String("Stash".into()));
        let (neutral, again, notes) = through_bedrock(&j);
        assert!(notes.missing.is_empty(), "{:?}", notes.missing);
        let Kind::Container(c) = &neutral.kind else { panic!() };
        assert_eq!(c.items[&4].id, "minecraft:water_bucket");
        assert_eq!(c.loot_table.as_deref(), Some("minecraft:chests/trial_chambers/corridor"));
        assert_eq!(again.get("LootTable").and_then(Tag::as_str), Some("minecraft:chests/trial_chambers/corridor"));
        assert_eq!(again.get("CustomName").and_then(Tag::as_str), Some("Stash"));
        assert_eq!((again.get("x").and_then(Tag::as_i64), again.get("z").and_then(Tag::as_i64)), (Some(5), Some(-9)));
    }

    #[test]
    fn loot_tables_follow_the_editions() {
        assert_eq!(loot_from_bedrock("loot_tables/chests/buriedtreasure.json"), "minecraft:chests/buried_treasure");
        assert_eq!(
            loot_to_bedrock("minecraft:chests/buried_treasure").as_deref(),
            Some("loot_tables/chests/buriedtreasure.json")
        );
        assert_eq!(loot_to_bedrock("minecraft:chests/village/village_fisher"), None);
        assert_eq!(loot_to_java("minecraft:chests/monster_room"), None);
    }

    #[test]
    fn furnace_times_use_each_editions_names() {
        let mut j = base("minecraft:furnace");
        j.insert("lit_time_remaining".into(), Tag::Int(30));
        j.insert("cooking_time_spent".into(), Tag::Int(12));
        j.insert("lit_total_time".into(), Tag::Int(200));
        let mut notes = Notes::default();
        let neutral = java::read(&j, &mut notes).unwrap();
        let b = bedrock::write(&neutral, &mut notes);
        assert_eq!(b.get("BurnTime"), Some(&Tag::Short(30)));
        assert_eq!(b.get("CookTime"), Some(&Tag::Short(12)));
        assert_eq!(b.get("BurnDuration"), Some(&Tag::Short(200)));
        let (_, again, _) = through_bedrock(&j);
        assert_eq!(again.get("lit_total_time"), Some(&Tag::Int(200)));
    }

    #[test]
    fn sign_text_colour_glow_and_wax() {
        let mut face = Compound::new();
        face.insert(
            "messages".into(),
            text::to_nbt(&serde_json::json!(["Hello", {"text": "World"}, "", "x"])),
        );
        face.insert("color".into(), Tag::String("red".into()));
        face.insert("has_glowing_text".into(), Tag::Byte(1));
        let mut j = base("minecraft:sign");
        j.insert("front_text".into(), Tag::Compound(face));
        j.insert("is_waxed".into(), Tag::Byte(1));
        let (neutral, again, _) = through_bedrock(&j);
        let Kind::Sign(s) = &neutral.kind else { panic!() };
        assert!(s.waxed && s.front.lit);
        assert_eq!(text::to_plain(&s.front.lines[1]), "World");
        let front = again.get("front_text").unwrap();
        assert_eq!(front.get("color").and_then(Tag::as_str), Some("red"));
        assert_eq!(front.get("has_glowing_text").and_then(Tag::as_i64), Some(1));
        assert_eq!(again.get("is_waxed").and_then(Tag::as_i64), Some(1));
    }

    #[test]
    fn long_bedrock_sign_text_is_wrapped_into_lines() {
        let mut face = Compound::new();
        face.insert("Text".into(), Tag::String("The quick brown fox jumps over the lazy dog".into()));
        let mut b = base("Sign");
        b.insert("FrontText".into(), Tag::Compound(face));
        let mut notes = Notes::default();
        let neutral = bedrock::read(&b, &mut notes).unwrap();
        let Kind::Sign(s) = &neutral.kind else { panic!() };
        let lines: Vec<String> = s.front.lines.iter().map(text::to_plain).collect();
        assert!(lines[0].len() < 43 && !lines[1].is_empty(), "{lines:?}");
        assert_eq!(lines.join(" ").trim(), "The quick brown fox jumps over the lazy dog");
    }

    #[test]
    fn spawner_entity_ids_are_translated() {
        let mut data = Compound::new();
        let mut entity = Compound::new();
        entity.insert("id".into(), Tag::String("minecraft:villager".into()));
        data.insert("entity".into(), Tag::Compound(entity));
        let mut j = base("minecraft:mob_spawner");
        j.insert("SpawnData".into(), Tag::Compound(data));
        j.insert("SpawnCount".into(), Tag::Short(9));
        let mut notes = Notes::default();
        let neutral = java::read(&j, &mut notes).unwrap();
        let b = bedrock::write(&neutral, &mut notes);
        assert_eq!(b.get("EntityIdentifier").and_then(Tag::as_str), Some("minecraft:villager_v2"));
        let (_, again, _) = through_bedrock(&j);
        assert_eq!(again.get("SpawnCount"), Some(&Tag::Short(9)));
        assert_eq!(
            again.get("SpawnData").and_then(|d| d.get("entity")).and_then(|e| e.get("id")).and_then(Tag::as_str),
            Some("minecraft:villager")
        );
    }

    #[test]
    fn campfire_and_data_less_kinds() {
        let mut item = Compound::new();
        item.insert("Slot".into(), Tag::Byte(2));
        item.insert("id".into(), Tag::String("minecraft:beef".into()));
        item.insert("count".into(), Tag::Int(1));
        let mut j = base("minecraft:campfire");
        j.insert(
            "Items".into(),
            Tag::List(mctk_nbt::List::from_tags(mctk_nbt::TagType::Compound, vec![Tag::Compound(item)]).unwrap()),
        );
        j.insert("CookingTimes".into(), Tag::IntArray(vec![0, 0, 150, 0]));
        let (neutral, again, _) = through_bedrock(&j);
        let Kind::Campfire(f) = &neutral.kind else { panic!() };
        assert_eq!(f.items[2].as_ref().unwrap().id, "minecraft:beef");
        assert_eq!(f.cooking_times[2], 150);
        assert_eq!(again.get("CookingTimes"), Some(&Tag::IntArray(vec![0, 0, 150, 0])));

        // Vaults, trial spawners and the like become bare entities.
        for id in ["minecraft:vault", "minecraft:trial_spawner", "minecraft:sculk_shrieker", "minecraft:beehive"] {
            let (neutral, again, notes) = through_bedrock(&base(id));
            assert!(matches!(neutral.kind, Kind::Empty(_)), "{id}");
            assert_eq!(again.get("id").and_then(Tag::as_str), Some(id));
            assert!(notes.missing.is_empty());
        }
        let (n, _, _) = through_bedrock(&base("minecraft:sculk_sensor"));
        assert!(matches!(n.kind, Kind::SculkSensor { calibrated: false }));
    }

    fn item(id: &str) -> Compound {
        let mut c = Compound::new();
        c.insert("id".into(), Tag::String(id.into()));
        c.insert("count".into(), Tag::Int(1));
        c
    }

    #[test]
    fn decorated_pot_sherds_and_contents() {
        let mut sherds = Compound::new();
        for (side, id) in [
            ("back", "minecraft:scrape_pottery_sherd"),
            ("left", "minecraft:brick"),
            ("right", "minecraft:angler_pottery_sherd"),
            ("front", "minecraft:brick"),
        ] {
            let mut s = Compound::new();
            s.insert("id".into(), Tag::String(id.into()));
            sherds.insert(side.into(), Tag::Compound(s));
        }
        let mut j = base("minecraft:decorated_pot");
        j.insert("sherds".into(), Tag::Compound(sherds));
        j.insert("item".into(), Tag::Compound(item("minecraft:emerald")));
        let mut notes = Notes::default();
        let neutral = java::read(&j, &mut notes).unwrap();
        let b = bedrock::write(&neutral, &mut notes);
        let names: Vec<&str> = b.get("sherds").and_then(Tag::as_list).unwrap().iter().filter_map(Tag::as_str).collect();
        assert_eq!(names, ["minecraft:scrape_pottery_sherd", "minecraft:brick", "minecraft:angler_pottery_sherd", "minecraft:brick"]);
        let (back, again, _) = through_bedrock(&j);
        let Kind::DecoratedPot(p) = &back.kind else { panic!() };
        assert_eq!(p.sherds[2], "minecraft:angler_pottery_sherd");
        assert_eq!(p.item.as_ref().unwrap().id, "minecraft:emerald");
        assert_eq!(
            again.get("sherds").and_then(|s| s.get("right")).and_then(|r| r.get("id")).and_then(Tag::as_str),
            Some("minecraft:angler_pottery_sherd")
        );
    }

    #[test]
    fn brushable_block_keeps_loot_and_takes_its_type_from_the_block() {
        let mut j = base("minecraft:brushable_block");
        j.insert("LootTable".into(), Tag::String("minecraft:archaeology/trail_ruins_common".into()));
        let mut notes = Notes::default();
        let mut neutral = java::read(&j, &mut notes).unwrap();
        edits::prepare_for_bedrock(&mut neutral, &mctk_java::BlockState::new("minecraft:suspicious_gravel"));
        let b = bedrock::write(&neutral, &mut notes);
        assert_eq!(b.get("type").and_then(Tag::as_str), Some("minecraft:suspicious_gravel"));
        assert_eq!(b.get("LootTable").and_then(Tag::as_str), Some("loot_tables/archaeology/trail_ruins_common.json"));
        let back = bedrock::read(&b, &mut notes).unwrap();
        let again = java::write(&back, &mut notes).unwrap();
        assert_eq!(again.get("LootTable").and_then(Tag::as_str), Some("minecraft:archaeology/trail_ruins_common"));
        assert!(again.get("hit_direction").is_none(), "Java's default direction is not written");
    }

    #[test]
    fn lectern_pages_halve_and_jukebox_keeps_its_record() {
        let mut j = base("minecraft:lectern");
        let mut book = item("minecraft:writable_book");
        let mut comps = Compound::new();
        let mut content = Compound::new();
        content.insert("pages".into(), crate::compound_list(vec![{
            let mut p = Compound::new();
            p.insert("raw".into(), Tag::String("page one".into()));
            p
        }]));
        comps.insert("minecraft:writable_book_content".into(), Tag::Compound(content));
        book.insert("components".into(), Tag::Compound(comps));
        j.insert("Book".into(), Tag::Compound(book));
        j.insert("Page".into(), Tag::Int(6));
        let mut notes = Notes::default();
        let neutral = java::read(&j, &mut notes).unwrap();
        let b = bedrock::write(&neutral, &mut notes);
        assert_eq!(b.get("page"), Some(&Tag::Int(3)));
        assert_eq!(b.get("hasBook"), Some(&Tag::Byte(1)));
        assert_eq!(b.get("totalPages"), Some(&Tag::Int(1)));
        let back = bedrock::read(&b, &mut notes).unwrap();
        assert_eq!(java::write(&back, &mut notes).unwrap().get("Page"), Some(&Tag::Int(6)));

        let mut jk = base("minecraft:jukebox");
        jk.insert("RecordItem".into(), Tag::Compound(item("minecraft:music_disc_cat")));
        let (back, _, _) = through_bedrock(&jk);
        let Kind::Jukebox { record } = &back.kind else { panic!() };
        assert_eq!(record.as_ref().unwrap().id, "minecraft:music_disc_cat");
    }

    #[test]
    fn skull_profile_survives_a_java_round_trip() {
        let mut entry = Compound::new();
        entry.insert("name".into(), Tag::String("textures".into()));
        entry.insert("value".into(), Tag::String("dGV4dHVyZQ==".into()));
        let mut profile = Compound::new();
        profile.insert("id".into(), Tag::IntArray(vec![0x1234_5678, -2, 3, 4]));
        profile.insert("name".into(), Tag::String("Steve".into()));
        profile.insert("properties".into(), crate::compound_list(vec![entry]));
        let mut j = base("minecraft:skull");
        j.insert("profile".into(), Tag::Compound(profile));
        let mut notes = Notes::default();
        let neutral = java::read(&j, &mut notes).unwrap();
        let again = java::write(&neutral, &mut notes).unwrap();
        let p = again.get("profile").unwrap();
        assert_eq!(p.get("id"), Some(&Tag::IntArray(vec![0x1234_5678, -2, 3, 4])));
        assert_eq!(p.get("name").and_then(Tag::as_str), Some("Steve"));
        let tex = p.get("properties").and_then(Tag::as_list).unwrap().get(0).unwrap();
        assert_eq!(tex.get("value").and_then(Tag::as_str), Some("dGV4dHVyZQ=="));
        // Bedrock keeps only what the block says.
        let b = bedrock::write(&neutral, &mut notes);
        assert!(b.get("profile").is_none());
        assert_eq!(b.get("SkullType"), Some(&Tag::Byte(0)));
    }

    fn items_list(items: Vec<(i8, &str)>) -> Tag {
        let tags = items
            .into_iter()
            .map(|(slot, id)| {
                let mut c = item(id);
                c.insert("Slot".into(), Tag::Byte(slot));
                c
            })
            .collect();
        crate::compound_list(tags)
    }

    #[test]
    fn brewing_stand_slots_are_renumbered_for_bedrock() {
        let mut j = base("minecraft:brewing_stand");
        // Java: bottles 0..2, ingredient 3, fuel 4.
        j.insert("Items".into(), items_list(vec![(0, "minecraft:potion"), (3, "minecraft:nether_wart"), (4, "minecraft:blaze_powder")]));
        j.insert("BrewTime".into(), Tag::Int(250));
        j.insert("Fuel".into(), Tag::Int(7));
        let mut notes = Notes::default();
        let neutral = java::read(&j, &mut notes).unwrap();
        let b = bedrock::write(&neutral, &mut notes);
        let slots: Vec<(i64, String)> = b
            .get("Items").and_then(Tag::as_list).unwrap().iter()
            .map(|i| (i.get("Slot").and_then(Tag::as_i64).unwrap(), i.get("Name").and_then(Tag::as_str).unwrap().to_string()))
            .collect();
        // Bedrock: ingredient 0, bottles 1..3, fuel 4.
        assert!(slots.contains(&(0, "minecraft:nether_wart".into())), "{slots:?}");
        assert!(slots.contains(&(1, "minecraft:potion".into())), "{slots:?}");
        assert_eq!(b.get("CookTime"), Some(&Tag::Short(250)));
        let (_, again, _) = through_bedrock(&j);
        let back: Vec<i64> = again.get("Items").and_then(Tag::as_list).unwrap().iter().filter_map(|i| i.get("Slot").and_then(Tag::as_i64)).collect();
        assert_eq!(back, vec![0, 3, 4]);
        assert_eq!(again.get("BrewTime"), Some(&Tag::Int(250)));
    }

    #[test]
    fn simple_data_kinds_round_trip() {
        // Beacon.
        let mut j = base("minecraft:beacon");
        j.insert("primary_effect".into(), Tag::String("minecraft:haste".into()));
        let (n, again, _) = through_bedrock(&j);
        assert!(matches!(n.kind, Kind::Beacon { primary: Some(_), secondary: None }));
        assert_eq!(again.get("primary_effect").and_then(Tag::as_str), Some("minecraft:haste"));
        // Command block.
        let mut j = base("minecraft:command_block");
        j.insert("Command".into(), Tag::String("say hi".into()));
        j.insert("auto".into(), Tag::Byte(1));
        let (_, again, _) = through_bedrock(&j);
        assert_eq!(again.get("Command").and_then(Tag::as_str), Some("say hi"));
        assert_eq!(again.get("auto"), Some(&Tag::Byte(1)));
        // End gateway: Java long age, Bedrock int; exit as array vs list.
        let mut j = base("minecraft:end_gateway");
        j.insert("Age".into(), Tag::Long(99));
        j.insert("exit_portal".into(), Tag::IntArray(vec![1, 2, 3]));
        let (_, again, _) = through_bedrock(&j);
        assert_eq!(again.get("Age"), Some(&Tag::Long(99)));
        assert_eq!(again.get("exit_portal"), Some(&Tag::IntArray(vec![1, 2, 3])));
        // Jigsaw: Java's `pool` is Bedrock's `target_pool`.
        let mut j = base("minecraft:jigsaw");
        j.insert("pool".into(), Tag::String("minecraft:village/plains/houses".into()));
        let mut notes = Notes::default();
        let b = bedrock::write(&java::read(&j, &mut notes).unwrap(), &mut notes);
        assert_eq!(b.get("target_pool").and_then(Tag::as_str), Some("minecraft:village/plains/houses"));
        // Crafter: Java's disabled-slot list is Bedrock's bitmask.
        let mut j = base("minecraft:crafter");
        j.insert("disabled_slots".into(), Tag::IntArray(vec![0, 4, 8]));
        let b = bedrock::write(&java::read(&j, &mut notes).unwrap(), &mut notes);
        assert_eq!(b.get("disabled_slots"), Some(&Tag::Short(0b1_0001_0001)));
        let (_, again, _) = through_bedrock(&j);
        assert_eq!(again.get("disabled_slots"), Some(&Tag::IntArray(vec![0, 4, 8])));
    }

    #[test]
    fn bookshelves_and_shelves_list_every_slot_on_bedrock() {
        let mut j = base("minecraft:chiseled_bookshelf");
        j.insert("Items".into(), items_list(vec![(2, "minecraft:book")]));
        let mut notes = Notes::default();
        let b = bedrock::write(&java::read(&j, &mut notes).unwrap(), &mut notes);
        assert_eq!(b.get("Items").and_then(Tag::as_list).unwrap().len(), 6);
        let (_, again, _) = through_bedrock(&j);
        let slots: Vec<i64> = again.get("Items").and_then(Tag::as_list).unwrap().iter().filter_map(|i| i.get("Slot").and_then(Tag::as_i64)).collect();
        assert_eq!(slots, vec![2]);
        let mut j = base("minecraft:shelf");
        j.insert("Items".into(), items_list(vec![(1, "minecraft:stick")]));
        let b = bedrock::write(&java::read(&j, &mut notes).unwrap(), &mut notes);
        assert_eq!(b.get("Items").and_then(Tag::as_list).unwrap().len(), 3);
    }

    #[test]
    fn structure_block_fields_convert() {
        let mut j = base("minecraft:structure_block");
        j.insert("name".into(), Tag::String("my:house".into()));
        j.insert("mode".into(), Tag::String("LOAD".into()));
        j.insert("rotation".into(), Tag::String("COUNTERCLOCKWISE_90".into()));
        j.insert("integrity".into(), Tag::Float(0.5));
        for (k, v) in [("sizeX", 4), ("sizeY", 5), ("sizeZ", 6)] {
            j.insert(k.into(), Tag::Int(v));
        }
        let mut notes = Notes::default();
        let b = bedrock::write(&java::read(&j, &mut notes).unwrap(), &mut notes);
        assert_eq!(b.get("structureName").and_then(Tag::as_str), Some("my:house"));
        assert_eq!(b.get("data"), Some(&Tag::Int(2)));
        assert_eq!(b.get("rotation"), Some(&Tag::Byte(3)));
        assert_eq!(b.get("integrity"), Some(&Tag::Float(50.0)));
        assert_eq!(b.get("yStructureSize"), Some(&Tag::Int(5)));
        let (_, again, _) = through_bedrock(&j);
        assert_eq!(again.get("mode").and_then(Tag::as_str), Some("LOAD"));
        assert_eq!(again.get("rotation").and_then(Tag::as_str), Some("COUNTERCLOCKWISE_90"));
    }

    #[test]
    fn bedrock_only_and_dropped_entities() {
        let mut notes = Notes::default();
        for id in ["Lodestone", "SporeBlossom", "MovingBlock"] {
            let n = bedrock::read(&base(id), &mut notes).unwrap();
            assert_eq!(n.kind, Kind::Discard, "{id}");
        }
        assert_eq!(java::read(&base("minecraft:piston"), &mut notes).unwrap().kind, Kind::Discard);
        // Generated Bedrock-only entities write their id.
        let be = BlockEntity::new(0, 0, 0, Kind::BedrockOnly(BedrockOnly::Lodestone));
        assert_eq!(bedrock::write(&be, &mut notes).get("id").and_then(Tag::as_str), Some("Lodestone"));
        assert!(java::write(&be, &mut notes).is_none());
    }

    #[test]
    fn unknown_types_are_not_handled() {
        assert!(java::read(&base("minecraft:definitely_not_real"), &mut Notes::default()).is_none());
        assert!(bedrock::read(&base("DefinitelyNotReal"), &mut Notes::default()).is_none());
    }
}

/// The loot table name to write to Java 26.x (`None` when Java has no such table).
pub fn loot_to_java(name: &str) -> Option<String> {
    if BEDROCK_ONLY_TABLES.contains(&name) {
        return None;
    }
    // Chunker maps this table's pre-1.14 name to the current one.
    Some(match name {
        "minecraft:chests/village_blacksmith" => "minecraft:chests/village/village_weaponsmith".into(),
        other => other.to_string(),
    })
}


