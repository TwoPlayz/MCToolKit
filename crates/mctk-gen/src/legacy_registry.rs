//! The registries of Java 1.8 to 1.12, built from the legacy tables the converter already carries
//! (the Java game's own data fixers) and the 1.13 block definitions. Those versions have no data
//! generator, so what each one has is read from what was added when:
//!
//! * blocks by numeric id: 1.8 up to 197, 1.9 to 212, 1.10 to 217, 1.11 to 234, 1.12 to 255;
//! * items likewise: up to 431, 448, 448, 452, 453 (and the music discs);
//! * entities and block entities by the version that introduced them.

use std::path::Path;

use mctk_mapping::legacy::legacy;
use mctk_registry::{Edition, Registry};

/// (version, DataVersion (0 before 1.9), era, highest block id, highest item id)
const VERSIONS: &[(&str, i32, u8, u16, u16)] = &[
    ("1.8", 0, 8, 197, 431), ("1.8.1", 0, 8, 197, 431), ("1.8.2", 0, 8, 197, 431),
    ("1.8.3", 0, 8, 197, 431), ("1.8.4", 0, 8, 197, 431), ("1.8.5", 0, 8, 197, 431),
    ("1.8.6", 0, 8, 197, 431), ("1.8.7", 0, 8, 197, 431), ("1.8.8", 0, 8, 197, 431),
    ("1.8.9", 0, 8, 197, 431),
    ("1.9", 169, 9, 212, 448), ("1.9.1", 175, 9, 212, 448), ("1.9.2", 176, 9, 212, 448),
    ("1.9.3", 183, 9, 212, 448), ("1.9.4", 184, 9, 212, 448),
    ("1.10", 510, 10, 217, 448), ("1.10.1", 511, 10, 217, 448), ("1.10.2", 512, 10, 217, 448),
    ("1.11", 819, 11, 234, 452), ("1.11.1", 921, 11, 234, 452), ("1.11.2", 922, 11, 234, 452),
    ("1.12", 1139, 12, 255, 453), ("1.12.1", 1241, 12, 255, 453), ("1.12.2", 1343, 12, 255, 453),
];

/// Entities by the era that introduced them, by their pre-1.11 names.
const ENTITIES: &[(u8, &[&str])] = &[
    (8, &[
        "Item", "XPOrb", "LeashKnot", "Painting", "ItemFrame", "ArmorStand", "EnderCrystal", "Arrow",
        "Snowball", "Fireball", "SmallFireball", "ThrownEnderpearl", "EyeOfEnderSignal",
        "ThrownPotion", "ThrownExpBottle", "FireworksRocketEntity", "PrimedTnt", "FallingSand",
        "MinecartCommandBlock", "Boat", "MinecartRideable", "MinecartChest", "MinecartFurnace",
        "MinecartTNT", "MinecartHopper", "MinecartSpawner", "Creeper", "Skeleton", "Spider", "Giant",
        "Zombie", "Slime", "Ghast", "PigZombie", "Enderman", "CaveSpider", "Silverfish", "Blaze",
        "LavaSlime", "EnderDragon", "WitherBoss", "Bat", "Witch", "Endermite", "Guardian", "Pig",
        "Sheep", "Cow", "Chicken", "Squid", "Wolf", "MushroomCow", "SnowMan", "Ozelot",
        "VillagerGolem", "EntityHorse", "Rabbit", "Villager", "WitherSkull", "ThrownEgg",
        // Kept as one entity with a type before 1.11 (and 1.10 for the stray and husk).
        "WitherSkeleton", "Stray", "Husk", "ZombieVillager", "Donkey", "Mule", "SkeletonHorse",
        "ZombieHorse", "ElderGuardian",
    ]),
    (9, &["ShulkerBullet", "Shulker", "AreaEffectCloud", "SpectralArrow", "DragonFireball"]),
    (10, &["PolarBear", "Stray", "Husk"]),
    (11, &[
        "ElderGuardian", "WitherSkeleton", "ZombieVillager", "SkeletonHorse", "ZombieHorse", "Donkey",
        "Mule", "Llama", "LlamaSpit", "Vex", "Evoker", "Vindicator", "EvocationFangs",
    ]),
    (12, &["Parrot", "Illusioner"]),
];

/// Block entities by the era that introduced them (current ids).
const BLOCK_ENTITIES: &[(u8, &[&str])] = &[
    (8, &[
        "furnace", "chest", "trapped_chest", "ender_chest", "jukebox", "dispenser", "dropper", "sign",
        "mob_spawner", "note_block", "piston", "brewing_stand", "enchanting_table", "end_portal",
        "command_block", "beacon", "skull", "daylight_detector", "hopper", "comparator", "flower_pot",
        "banner",
    ]),
    (9, &["end_gateway"]),
    (10, &["structure_block"]),
    (11, &["shulker_box"]),
    (12, &["bed"]),
];

/// Block names the 1.13 definitions spell the old way (the legacy tables use the current names).
const RENAMED: [(&str, &str); 3] = [
    ("minecraft:grass", "minecraft:short_grass"),
    ("minecraft:grass_path", "minecraft:dirt_path"),
    ("minecraft:chain", "minecraft:iron_chain"),
];

fn current_name(name: &str) -> String {
    let bare = name.trim_start_matches("minecraft:");
    // Colours and kinds the old versions keep in block entities fold into the one block they had.
    let folded = if bare.ends_with("_wall_banner") {
        "white_wall_banner"
    } else if bare.ends_with("_banner") {
        "white_banner"
    } else if bare.ends_with("_bed") {
        "red_bed"
    } else if bare.ends_with("_wall_skull") || bare.ends_with("_wall_head") {
        "skeleton_wall_skull"
    } else if bare.ends_with("_skull") || bare.ends_with("_head") {
        "skeleton_skull"
    } else {
        bare
    };
    let full = format!("minecraft:{folded}");
    RENAMED.iter().find(|(old, _)| *old == full).map_or(full.clone(), |(_, new)| (*new).to_string())
}

pub fn emit(out: &Path) -> Result<(), String> {
    let base = mctk_registry::builtin::java("1.13.2").ok_or("no 1.13.2 registry")?;
    let biomes: Vec<String> = base
        .biomes
        .iter()
        .filter(|b| !b.contains("bamboo"))
        .cloned()
        .collect();
    // Which block names the legacy tables have, with the lowest id of each.
    let mut lowest: std::collections::HashMap<String, u16> = std::collections::HashMap::new();
    for id in 0..256u16 {
        for data in 0..16u8 {
            if legacy().has_exact_block(id, data) {
                if let Some(s) = legacy().block(id, data) {
                    lowest.entry(s.name.clone()).or_insert(id);
                }
            }
        }
    }
    let items_text = mctk_registry::builtin::legacy_items_text();
    let mut item_names: Vec<(u16, String)> = Vec::new();
    for line in items_text.lines().filter(|l| !l.starts_with('#')) {
        let mut f = line.split('\t');
        let (Some(id), Some(_), Some(name)) = (f.next(), f.next(), f.next()) else { continue };
        if let Ok(id) = id.parse::<u16>() {
            item_names.push((id, name.trim_matches('"').to_string()));
        }
    }
    for &(version, dv, era, max_block, max_item) in VERSIONS {
        let mut r = Registry::new(Edition::Java, version);
        r.data_version = Some(dv);
        for (name, def) in &base.blocks {
            // Skulls (144) and flower pots (140) are not in the tables: the game keeps what they hold in block
            // entities, which the tables could not read back.
            let bare = name.trim_start_matches("minecraft:");
            let special = ((bare.ends_with("_skull") || bare.ends_with("_head")) && max_block >= 144)
                || ((bare == "flower_pot" || bare.starts_with("potted_")) && max_block >= 140);
            if special || lowest.get(&current_name(name)).is_some_and(|id| *id <= max_block) {
                r.blocks.insert(name.clone(), def.clone());
            }
        }
        let mut items: Vec<String> = Vec::new();
        for (id, name) in &item_names {
            if (*id <= max_item || (2256..=2267).contains(id)) && !items.contains(name) {
                items.push(name.clone());
                // The names 1.13 to 1.20.2 used for the same items.
                for (old, new) in RENAMED {
                    if name == new {
                        items.push(old.to_string());
                    }
                }
            }
        }
        r.items = items;
        for (from, names) in ENTITIES {
            if *from <= era {
                for n in *names {
                    if let Some(id) = legacy().entity(n) {
                        r.entities.push(id.to_string());
                    }
                }
            }
        }
        // Boats and minecarts are single entities with a type in 1.8 to 1.12; the current game
        // names them by kind.
        for extra in ["minecraft:boat", "minecraft:spruce_boat", "minecraft:birch_boat", "minecraft:jungle_boat", "minecraft:acacia_boat", "minecraft:dark_oak_boat"] {
            if !r.entities.iter().any(|e| e == extra) && r.entities.iter().any(|e| e == "minecraft:oak_boat") {
                r.entities.push(extra.to_string());
            }
        }
        for (from, names) in BLOCK_ENTITIES {
            if *from <= era {
                r.block_entities.extend(names.iter().map(|n| format!("minecraft:{n}")));
            }
        }
        r.biomes = biomes.clone();
        let file = out.join(format!("{version}.tsv"));
        std::fs::write(&file, r.to_text()).map_err(|e| format!("{}: {e}", file.display()))?;
        println!("{version}: {} blocks, {} items, {} entities, {} block entities", r.blocks.len(), r.items.len(), r.entities.len(), r.block_entities.len());
    }
    Ok(())
}
