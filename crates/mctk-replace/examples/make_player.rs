//! `cargo run -p mctk-replace --example make_player -- <out.dat>`
//! Writes a realistic 1.20.1 player file (DataVersion 3465) to test player conversion with: full
//! survival stats, armor, offhand, an enchanted and named hotbar, ender chest, effects, a spawn
//! point and recipes. Used as input to the real game's upgrader for ground truth.

use mctk_nbt::{Compound, Flavor, List, NamedTag, Tag, TagType};

fn s(v: &str) -> Tag {
    Tag::String(v.into())
}
fn c(fields: Vec<(&str, Tag)>) -> Tag {
    Tag::Compound(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect::<Compound>())
}
fn list(ty: TagType, items: Vec<Tag>) -> Tag {
    Tag::List(List::from_tags(ty, items).unwrap())
}
fn item(slot: i8, id: &str, count: i8, tag: Option<Tag>) -> Tag {
    let mut f = vec![("Slot", Tag::Byte(slot)), ("id", s(id)), ("Count", Tag::Byte(count))];
    if let Some(t) = tag {
        f.push(("tag", t));
    }
    c(f)
}
fn doubles(v: &[f64]) -> Tag {
    list(TagType::Double, v.iter().map(|d| Tag::Double(*d)).collect())
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: make_player <out.dat>");
    let sword = c(vec![
        ("Damage", Tag::Int(7)),
        ("Enchantments", list(TagType::Compound, vec![c(vec![("id", s("minecraft:sharpness")), ("lvl", Tag::Short(4))]), c(vec![("id", s("minecraft:sweeping")), ("lvl", Tag::Short(3))])])),
        ("display", c(vec![("Name", s(r#"{"text":"Steve's Blade","color":"gold","italic":false}"#))])),
    ]);
    let inventory = list(
        TagType::Compound,
        vec![
            item(0, "minecraft:diamond_sword", 1, Some(sword)),
            item(1, "minecraft:cooked_beef", 37, None),
            item(2, "minecraft:torch", 64, None),
            item(3, "minecraft:potion", 1, Some(c(vec![("Potion", s("minecraft:strong_healing"))]))),
            item(9, "minecraft:oak_log", 12, None),
            item(10, "minecraft:enchanted_golden_apple", 2, None),
            item(35, "minecraft:written_book", 1, Some(c(vec![("title", s("Notes")), ("author", s("Steve")), ("pages", list(TagType::String, vec![s(r#"{"text":"day 1"}"#)]))]))),
            item(100, "minecraft:iron_boots", 1, Some(c(vec![("Damage", Tag::Int(20))]))),
            item(101, "minecraft:iron_leggings", 1, None),
            item(102, "minecraft:diamond_chestplate", 1, Some(c(vec![("Enchantments", list(TagType::Compound, vec![c(vec![("id", s("minecraft:protection")), ("lvl", Tag::Short(3))])]))]))),
            item(103, "minecraft:turtle_helmet", 1, None),
            item(-106, "minecraft:shield", 1, Some(c(vec![("Damage", Tag::Int(5))]))),
        ],
    );
    let ender = list(TagType::Compound, vec![item(0, "minecraft:diamond", 9, None), item(13, "minecraft:netherite_ingot", 3, None)]);
    let effects = list(
        TagType::Compound,
        vec![c(vec![("Id", Tag::Byte(1)), ("Amplifier", Tag::Byte(1)), ("Duration", Tag::Int(4000)), ("Ambient", Tag::Byte(0)), ("ShowParticles", Tag::Byte(1)), ("ShowIcon", Tag::Byte(1))])],
    );
    let player = c(vec![
        ("DataVersion", Tag::Int(3465)),
        ("UUID", Tag::IntArray(vec![-1996495974, 974998821, -1499058141, 1483921170])),
        ("Pos", doubles(&[3.5, 70.0, 12.5])),
        ("Motion", doubles(&[0.0, -0.0784, 0.0])),
        ("Rotation", list(TagType::Float, vec![Tag::Float(-49.0), Tag::Float(20.0)])),
        ("Dimension", s("minecraft:overworld")),
        ("Health", Tag::Float(14.0)),
        ("HurtTime", Tag::Short(0)),
        ("HurtByTimestamp", Tag::Int(0)),
        ("DeathTime", Tag::Short(0)),
        ("AbsorptionAmount", Tag::Float(2.0)),
        ("foodLevel", Tag::Int(11)),
        ("foodSaturationLevel", Tag::Float(2.5)),
        ("foodExhaustionLevel", Tag::Float(1.25)),
        ("foodTickTimer", Tag::Int(30)),
        ("XpLevel", Tag::Int(23)),
        ("XpP", Tag::Float(0.4)),
        ("XpTotal", Tag::Int(540)),
        ("XpSeed", Tag::Int(123456)),
        ("Score", Tag::Int(540)),
        ("Air", Tag::Short(300)),
        ("Fire", Tag::Short(-20)),
        ("FallDistance", Tag::Float(0.0)),
        ("OnGround", Tag::Byte(1)),
        ("FallFlying", Tag::Byte(0)),
        ("Invulnerable", Tag::Byte(0)),
        ("PortalCooldown", Tag::Int(0)),
        ("SleepTimer", Tag::Short(0)),
        ("playerGameType", Tag::Int(0)),
        ("previousPlayerGameType", Tag::Int(1)),
        ("seenCredits", Tag::Byte(1)),
        ("SelectedItemSlot", Tag::Int(2)),
        ("SpawnX", Tag::Int(-48)),
        ("SpawnY", Tag::Int(121)),
        ("SpawnZ", Tag::Int(-16)),
        ("SpawnForced", Tag::Byte(1)),
        ("SpawnAngle", Tag::Float(90.0)),
        ("SpawnDimension", s("minecraft:overworld")),
        (
            "abilities",
            c(vec![
                ("invulnerable", Tag::Byte(0)),
                ("mayfly", Tag::Byte(0)),
                ("instabuild", Tag::Byte(0)),
                ("walkSpeed", Tag::Float(0.1)),
                ("mayBuild", Tag::Byte(1)),
                ("flying", Tag::Byte(0)),
                ("flySpeed", Tag::Float(0.05)),
            ]),
        ),
        (
            "recipeBook",
            c(vec![
                ("recipes", list(TagType::String, vec![s("minecraft:crafting_table"), s("minecraft:stick")])),
                ("toBeDisplayed", list(TagType::String, vec![s("minecraft:stick")])),
                ("isFilteringCraftable", Tag::Byte(0)),
                ("isGuiOpen", Tag::Byte(1)),
                ("isFurnaceFilteringCraftable", Tag::Byte(0)),
                ("isFurnaceGuiOpen", Tag::Byte(0)),
                ("isBlastingFurnaceFilteringCraftable", Tag::Byte(0)),
                ("isBlastingFurnaceGuiOpen", Tag::Byte(0)),
                ("isSmokerFilteringCraftable", Tag::Byte(0)),
                ("isSmokerGuiOpen", Tag::Byte(0)),
            ]),
        ),
        ("Inventory", inventory),
        ("EnderItems", ender),
        ("ActiveEffects", effects),
        ("LastDeathLocation", c(vec![("dimension", s("minecraft:overworld")), ("pos", Tag::IntArray(vec![10, 64, -20]))])),
        (
            "Attributes",
            list(TagType::Compound, vec![c(vec![("Name", s("minecraft:generic.movement_speed")), ("Base", Tag::Double(0.1))]), c(vec![("Name", s("minecraft:generic.max_health")), ("Base", Tag::Double(20.0))])]),
        ),
        ("Brain", c(vec![("memories", c(vec![]))])),
    ]);
    let root = NamedTag { name: String::new(), tag: player };
    let raw = mctk_nbt::to_bytes(&root, Flavor::Java).unwrap();
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&raw).unwrap();
    std::fs::write(&out, e.finish().unwrap()).unwrap();
    println!("wrote {out}");
}
