//! Writes `items.tsv` (Java item, Bedrock item, Bedrock data value) without a third-party table.
//!
//! Most items have the same name in both editions. The ones that do not are renames the games
//! made at different times (the 1.13 flattening on the Java side, Bedrock keeping its older
//! names), written here by rule: coloured beds and banners are one Bedrock item with the colour
//! in the data value, every structure map is a filled map, and so on.

use std::path::Path;

use mctk_registry::builtin;

/// Dye colours in the order Bedrock numbers them for beds (and wool, carpet, ...).
const COLORS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];
/// The same colours in the order banners are numbered: the dye order, black first.
const BANNER_COLORS: [&str; 16] = [
    "black", "red", "green", "brown", "blue", "purple", "cyan", "light_gray", "gray", "pink", "lime", "yellow", "light_blue", "magenta", "orange", "white",
];

const RENAMES: &[(&str, &str)] = &[
    ("bricks", "brick_block"),
    ("cobblestone_stairs", "stone_stairs"),
    ("cobweb", "web"),
    ("dead_bush", "deadbush"),
    ("debug_stick", "stick"),
    ("dirt_path", "grass_path"),
    ("end_stone_brick_stairs", "end_brick_stairs"),
    ("end_stone_bricks", "end_bricks"),
    ("flowering_azalea_leaves", "azalea_leaves_flowered"),
    ("frogspawn", "frog_spawn"),
    ("furnace_minecart", "hopper_minecart"),
    ("glow_item_frame", "glow_frame"),
    ("item_frame", "frame"),
    ("jack_o_lantern", "lit_pumpkin"),
    ("knowledge_book", "book"),
    ("light", "light_block"),
    ("light_gray_glazed_terracotta", "silver_glazed_terracotta"),
    ("lily_pad", "waterlily"),
    ("magma_block", "magma"),
    ("map", "empty_map"),
    ("melon", "melon_block"),
    ("nether_brick", "netherbrick"),
    ("nether_bricks", "nether_brick"),
    ("nether_quartz_ore", "quartz_ore"),
    ("note_block", "noteblock"),
    ("oak_button", "wooden_button"),
    ("oak_door", "wooden_door"),
    ("oak_fence_gate", "fence_gate"),
    ("oak_pressure_plate", "wooden_pressure_plate"),
    ("oak_trapdoor", "trapdoor"),
    ("powered_rail", "golden_rail"),
    ("prismarine_brick_stairs", "prismarine_bricks_stairs"),
    ("red_nether_bricks", "red_nether_brick"),
    ("rooted_dirt", "dirt_with_roots"),
    ("shulker_box", "undyed_shulker_box"),
    ("slime_block", "slime"),
    ("small_dripleaf", "small_dripleaf_block"),
    ("snow", "snow_layer"),
    ("snow_block", "snow"),
    ("spawner", "mob_spawner"),
    ("spectral_arrow", "arrow"),
    ("tipped_arrow", "arrow"),
    ("stone_slab", "normal_stone_slab"),
    ("stone_stairs", "normal_stone_stairs"),
    ("stonecutter", "stonecutter_block"),
    ("terracotta", "hardened_clay"),
    ("test_block", "unknown"),
    ("test_instance_block", "unknown"),
    ("waxed_copper_block", "waxed_copper"),
    ("zombified_piglin_spawn_egg", "zombie_pigman_spawn_egg"),
];

fn bedrock_item(java: &str) -> (String, u16) {
    let bare = java.trim_start_matches("minecraft:");
    if let Some(color) = bare.strip_suffix("_bed") {
        if let Some(n) = COLORS.iter().position(|c| *c == color) {
            return ("minecraft:bed".into(), n as u16);
        }
    }
    if let Some(color) = bare.strip_suffix("_banner") {
        if let Some(n) = BANNER_COLORS.iter().position(|c| *c == color) {
            return ("minecraft:banner".into(), n as u16);
        }
    }
    if bare.ends_with("_map") {
        return ("minecraft:filled_map".into(), 0);
    }
    if let Some((_, to)) = RENAMES.iter().find(|(from, _)| *from == bare) {
        return (format!("minecraft:{to}"), 0);
    }
    (java.to_string(), 0)
}

pub fn emit(out: &Path) -> Result<(), String> {
    let java = builtin::java("26.3").ok_or("no Java registry")?;
    let mut items = java.items.clone();
    items.sort();
    let mut text = String::new();
    for item in &items {
        let (name, data) = bedrock_item(item);
        text.push_str(&format!("{item}\t{name}\t{data}\n"));
    }
    std::fs::write(out, text).map_err(|e| e.to_string())?;
    println!("wrote {} ({} items)", out.display(), items.len());
    Ok(())
}