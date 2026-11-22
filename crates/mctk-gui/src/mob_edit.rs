//! What the tree's menu changes on mobs and the like, in each edition's own layout: being
//! invulnerable, having no AI, being invisible, health, what a mob wears and holds, a
//! villager's trades, and where an end gateway sends things.
//!
//! | | Java | Bedrock | Console / Java before 1.9 |
//! |---|---|---|---|
//! | invulnerable | `Invulnerable` | `Invulnerable` | `Invulnerable` |
//! | no AI | `NoAI` | (none) | `NoAI` |
//! | health | `Health` (float) | `Attributes`' `minecraft:health` `Current` | `Health` (and `HealF`) |
//! | wearing | `equipment` (1.21.5 on), else `ArmorItems` + `HandItems` | `Armor`, `Mainhand`, `Offhand` | `Equipment` |
//! | trades | `Offers.Recipes`: `buy`, `buyB`, `sell` | `Offers.Recipes`: `buyA`, `buyB`, `sell` | as Java, items by number |
//! | gateway exit | `exit_portal` (1.20.5 on), else `ExitPortal {X, Y, Z}` | `ExitPortal` [x, y, z] | `ExitPortal {X, Y, Z}` |

use mctk_nbt::{Compound, List, Tag, TagType};

fn flag(c: &Compound, k: &str) -> bool {
    c.get(k).and_then(Tag::as_i64).is_some_and(|v| v != 0)
}

/// Sets a byte flag, or (off) takes it away.
fn set_flag(c: &mut Compound, k: &str, on: bool) {
    if on {
        c.insert(k.into(), Tag::Byte(1));
    } else {
        c.shift_remove(k);
    }
}

fn is_bedrock(c: &Compound) -> bool {
    c.contains_key("identifier")
}

/// Java 1.21.5 (data version 4325), where mobs keep what they wear in `equipment`.
const DV_EQUIPMENT: i64 = 4325;
/// Java 1.20.5 (data version 3837), where item stacks became `{id, count}` with components.
const DV_COMPONENTS: i64 = 3837;

// ---------------------------------------------------------------------------------------------
// Item stacks
// ---------------------------------------------------------------------------------------------

/// How a world writes an item stack.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StackForm {
    /// Java 1.20.5 on: `{id, count}` (an int).
    Components,
    /// Java 1.8 to 1.20.4: `{id, Count}` (a byte).
    Counted,
    /// Console editions and old Java: `{id, Count, Damage}`, the id a number.
    Numeric,
    /// Bedrock: `{Name, Count, Damage, WasPickedUp}`.
    Bedrock,
}

impl StackForm {
    /// The form for stacks inside `c` (a mob, a villager...), from the stacks it already has,
    /// else from its edition and version.
    pub fn of(c: &Compound, data_version: Option<i64>) -> StackForm {
        if is_bedrock(c) {
            return StackForm::Bedrock;
        }
        if let Some(found) = first_stack(&Tag::Compound(c.clone())) {
            if matches!(found.get("id"), Some(Tag::Short(_) | Tag::Int(_))) {
                return StackForm::Numeric;
            }
            if found.contains_key("count") {
                return StackForm::Components;
            }
            if found.contains_key("Count") {
                return StackForm::Counted;
            }
        }
        match data_version {
            Some(v) if v >= DV_COMPONENTS => StackForm::Components,
            Some(_) => StackForm::Counted,
            // No version: a console save (ids like `Pig`), unless its id is namespaced.
            None if c.get("id").and_then(Tag::as_str).is_some_and(|i| i.contains(':')) => StackForm::Counted,
            None => StackForm::Numeric,
        }
    }
}

/// The first non-empty item stack anywhere under a tag.
fn first_stack(t: &Tag) -> Option<Compound> {
    match t {
        Tag::Compound(c) => {
            let has_id = matches!(c.get("id"), Some(Tag::String(_) | Tag::Short(_) | Tag::Int(_))) || matches!(c.get("Name"), Some(Tag::String(s)) if !s.is_empty());
            if has_id && (c.contains_key("count") || c.contains_key("Count")) {
                return Some(c.clone());
            }
            c.values().find_map(first_stack)
        }
        Tag::List(l) => l.iter().find_map(first_stack),
        _ => None,
    }
}

/// A stack of `count` of item `id` (Java's `minecraft:...` name) in a form.
pub fn make_stack(form: StackForm, id: &str, count: i64) -> Compound {
    let mut c = Compound::new();
    let id = if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") };
    match form {
        StackForm::Components => {
            c.insert("id".into(), Tag::String(id));
            c.insert("count".into(), Tag::Int(count as i32));
        }
        StackForm::Counted => {
            c.insert("id".into(), Tag::String(id));
            c.insert("Count".into(), Tag::Byte(count as i8));
        }
        StackForm::Numeric => {
            let (n, damage) = mctk_mapping::legacy::legacy().numeric_item(&id).unwrap_or((1, 0));
            c.insert("id".into(), Tag::Short(n as i16));
            c.insert("Count".into(), Tag::Byte(count as i8));
            c.insert("Damage".into(), Tag::Short(damage as i16));
        }
        StackForm::Bedrock => {
            let (name, data) = mctk_mapping::item_mappings().java_to_bedrock(&id).map_or((id.clone(), 0), |(n, d)| (n.to_string(), d));
            c.insert("Name".into(), Tag::String(name));
            c.insert("Count".into(), Tag::Byte(count as i8));
            c.insert("Damage".into(), Tag::Short(data));
            c.insert("WasPickedUp".into(), Tag::Byte(0));
        }
    }
    c
}

/// An empty slot in a form (Bedrock and the lists of older Java keep one in each).
fn empty_stack(form: StackForm) -> Compound {
    match form {
        StackForm::Bedrock => {
            let mut c = Compound::new();
            c.insert("Name".into(), Tag::String(String::new()));
            c.insert("Count".into(), Tag::Byte(0));
            c.insert("Damage".into(), Tag::Short(0));
            c.insert("WasPickedUp".into(), Tag::Byte(0));
            c
        }
        _ => Compound::new(),
    }
}

/// What a stack is (its Java id) and how many; `None` for an empty one.
pub fn stack_of(c: &Compound) -> Option<(String, i64)> {
    let count = c.get("count").or_else(|| c.get("Count")).and_then(Tag::as_i64).unwrap_or(1);
    let damage = c.get("Damage").and_then(Tag::as_i64).unwrap_or(0);
    let id = match (c.get("id"), c.get("Name")) {
        (Some(Tag::String(s)), _) => s.clone(),
        (Some(Tag::Short(n)), _) => mctk_mapping::legacy::legacy().item(*n as u16, damage as u16)?.to_string(),
        (Some(Tag::Int(n)), _) => mctk_mapping::legacy::legacy().item(*n as u16, damage as u16)?.to_string(),
        (None, Some(Tag::String(n))) if !n.is_empty() => {
            mctk_mapping::item_mappings().bedrock_to_java(n, damage as i16).map_or_else(|| n.clone(), str::to_string)
        }
        _ => return None,
    };
    if id.is_empty() || id.ends_with(":air") || count <= 0 {
        return None;
    }
    Some((id, count))
}

/// The stack changed to `count` of the same item, in its own form.
pub fn with_count(c: &Compound, count: i64) -> Compound {
    let mut c = c.clone();
    if c.contains_key("count") {
        c.insert("count".into(), Tag::Int(count as i32));
    } else {
        c.insert("Count".into(), Tag::Byte(count.clamp(0, 127) as i8));
    }
    c
}

// ---------------------------------------------------------------------------------------------
// Flags and health
// ---------------------------------------------------------------------------------------------

pub fn invulnerable(c: &Compound) -> bool {
    flag(c, "Invulnerable")
}

pub fn set_invulnerable(c: &mut Compound, on: bool) {
    set_flag(c, "Invulnerable", on);
}

/// Whether a mob has no AI; `None` where the edition keeps no such thing (Bedrock).
pub fn no_ai(c: &Compound) -> Option<bool> {
    (!is_bedrock(c)).then(|| flag(c, "NoAI"))
}

pub fn set_no_ai(c: &mut Compound, on: bool) {
    set_flag(c, "NoAI", on);
}

/// Whether a thing falls (has no `NoGravity`); `None` where the edition keeps no such thing
/// (Bedrock).
pub fn gravity(c: &Compound) -> Option<bool> {
    (!is_bedrock(c)).then(|| !flag(c, "NoGravity"))
}

/// Makes a thing fall (`NoGravity` taken away) or float.
pub fn set_gravity(c: &mut Compound, on: bool) {
    set_flag(c, "NoGravity", !on);
}

/// Whether a thing is invisible, for the things the game saves it for (armor stands and item
/// frames, in Java and the console editions); `None` for others.
pub fn invisible(c: &Compound, java_id: &str) -> Option<bool> {
    let n = java_id.trim_start_matches("minecraft:");
    (!is_bedrock(c) && matches!(n, "armor_stand" | "item_frame" | "glow_item_frame")).then(|| flag(c, "Invisible"))
}

pub fn set_invisible(c: &mut Compound, on: bool) {
    set_flag(c, "Invisible", on);
}

/// A mob's health, if it has one.
pub fn health(c: &Compound) -> Option<f64> {
    if is_bedrock(c) {
        return bedrock_health(c).and_then(|h| h.get("Current")).and_then(Tag::as_f64);
    }
    c.get("HealF").or_else(|| c.get("Health")).and_then(Tag::as_f64)
}

fn bedrock_health(c: &Compound) -> Option<&Compound> {
    c.get("Attributes")?.as_list()?.iter().filter_map(Tag::as_compound).find(|a| a.get("Name").and_then(Tag::as_str) == Some("minecraft:health"))
}

/// The most health a mob can have: its max health attribute in any edition's layout, else the
/// game's usual for its kind (`java_id`), where known.
pub fn max_health(c: &Compound, java_id: &str) -> Option<f64> {
    if is_bedrock(c) {
        if let Some(m) = bedrock_health(c).and_then(|h| h.get("Max")).and_then(Tag::as_f64) {
            return Some(m);
        }
    }
    for key in ["attributes", "Attributes"] {
        let Some(l) = c.get(key).and_then(Tag::as_list) else { continue };
        for a in l.iter().filter_map(Tag::as_compound) {
            let name = a.get("id").or_else(|| a.get("Name")).and_then(Tag::as_str).unwrap_or("");
            let bare = name.trim_start_matches("minecraft:").trim_start_matches("generic.");
            if bare == "max_health" || bare == "maxHealth" {
                if let Some(b) = a.get("base").or_else(|| a.get("Base")).and_then(Tag::as_f64) {
                    return Some(b);
                }
            }
        }
    }
    let n = java_id.trim_start_matches("minecraft:");
    Some(match n {
        "zombie" | "husk" | "drowned" | "zombie_villager" | "zombified_piglin" | "skeleton" | "stray" | "bogged" | "wither_skeleton" | "creeper" | "villager"
        | "wandering_trader" | "blaze" | "piglin" | "allay" | "panda" | "strider" | "phantom" => 20.0,
        "witch" => 26.0,
        "enderman" => 40.0,
        "pillager" | "vindicator" | "evoker" => 24.0,
        "illusioner" | "camel" => 32.0,
        "turtle" | "breeze" => 30.0,
        "parrot" | "bat" => 6.0,
        "frog" | "goat" | "pig" | "sheep" | "mooshroom" | "cow" | "chicken" | "cat" | "ocelot" | "fox" => 10.0,
        "rabbit" => 3.0,
        "spider" => 16.0,
        "cave_spider" => 12.0,
        "wolf" => 8.0,
        "iron_golem" => 100.0,
        "snow_golem" => 4.0,
        "ghast" => 10.0,
        "guardian" => 30.0,
        "elder_guardian" => 80.0,
        "shulker" => 30.0,
        "hoglin" | "zoglin" => 40.0,
        "piglin_brute" => 50.0,
        "ravager" => 100.0,
        "warden" => 500.0,
        "wither" => 300.0,
        "ender_dragon" => 200.0,
        "endermite" | "silverfish" => 8.0,
        "vex" => 14.0,
        "squid" | "glow_squid" | "dolphin" => 10.0,
        "cod" | "salmon" | "tropical_fish" | "pufferfish" | "tadpole" => 3.0,
        "axolotl" => 14.0,
        "bee" => 10.0,
        "polar_bear" => 30.0,
        "llama" | "trader_llama" | "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" => 15.0,
        "sniffer" => 14.0,
        "armadillo" => 12.0,
        "creaking" => 1.0,
        _ => return None,
    })
}

pub use crate::effects::Effect;

/// The status effects on a mob (or player), in any edition's layout.
pub fn effects(c: &Compound) -> Vec<Effect> {
    crate::effects::read(c, crate::effects::Holder::Mob)
}

/// The armor points a mob has: its kind's own (zombies have 2) and what it wears.
pub fn armor_points(c: &Compound, java_id: &str, data_version: Option<i64>) -> i64 {
    let n = java_id.trim_start_matches("minecraft:");
    let own = if matches!(n, "zombie" | "husk" | "drowned" | "zombie_villager" | "zombified_piglin" | "zombie_horse") { 2 } else { 0 };
    let worn: i64 = equipment(c, data_version)
        .into_iter()
        .filter_map(|(slot, item)| {
            let id = item?.0;
            let bare = id.trim_start_matches("minecraft:");
            let (material, piece) = bare.rsplit_once('_')?;
            // Points by material: leather, chainmail, iron, golden, diamond, netherite, copper.
            let by: [i64; 7] = match (slot, piece) {
                (Slot::Head, "helmet") if material == "turtle" => return Some(2),
                (Slot::Head, "helmet") => [1, 2, 2, 2, 3, 3, 2],
                (Slot::Chest, "chestplate") => [3, 5, 6, 5, 8, 8, 4],
                (Slot::Legs, "leggings") => [2, 4, 5, 3, 6, 6, 3],
                (Slot::Feet, "boots") => [1, 1, 2, 1, 3, 3, 1],
                _ => return None,
            };
            let k = ["leather", "chainmail", "iron", "golden", "diamond", "netherite", "copper"].iter().position(|m| *m == material)?;
            Some(by[k])
        })
        .sum();
    own + worn
}

// ---------------------------------------------------------------------------------------------
// What a mob wears and holds
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Head,
    Chest,
    Legs,
    Feet,
    MainHand,
    OffHand,
}

impl Slot {
    pub const ALL: [Slot; 6] = [Slot::Head, Slot::Chest, Slot::Legs, Slot::Feet, Slot::MainHand, Slot::OffHand];

    pub fn name(self) -> &'static str {
        match self {
            Slot::Head => "Head",
            Slot::Chest => "Chest",
            Slot::Legs => "Legs",
            Slot::Feet => "Feet",
            Slot::MainHand => "Main Hand",
            Slot::OffHand => "Off Hand",
        }
    }

    /// The game's picture for the slot while empty.
    pub fn empty_picture(self) -> &'static str {
        match self {
            Slot::Head => "gui/sprites/container/slot/helmet.png",
            Slot::Chest => "gui/sprites/container/slot/chestplate.png",
            Slot::Legs => "gui/sprites/container/slot/leggings.png",
            Slot::Feet => "gui/sprites/container/slot/boots.png",
            Slot::MainHand => "gui/sprites/container/slot/sword.png",
            Slot::OffHand => "gui/sprites/container/slot/shield.png",
        }
    }

    /// Java 1.21.5's key in `equipment`.
    fn key(self) -> &'static str {
        match self {
            Slot::Head => "head",
            Slot::Chest => "chest",
            Slot::Legs => "legs",
            Slot::Feet => "feet",
            Slot::MainHand => "mainhand",
            Slot::OffHand => "offhand",
        }
    }
}

/// Which layout a mob keeps its gear in.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Gear {
    /// `equipment {head, chest, ...}` (Java 1.21.5 on).
    Named,
    /// `ArmorItems` [feet, legs, chest, head] and `HandItems` [main, off] (Java 1.9 to 1.21.4).
    Lists,
    /// `Equipment` [hand, feet, legs, chest, head] (console, Java before 1.9).
    Old,
    /// `Armor` [head, chest, legs, feet], `Mainhand` [one], `Offhand` [one] (Bedrock).
    Bedrock,
}

fn gear(c: &Compound, data_version: Option<i64>) -> Gear {
    if is_bedrock(c) {
        Gear::Bedrock
    } else if c.contains_key("equipment") {
        Gear::Named
    } else if c.contains_key("ArmorItems") || c.contains_key("HandItems") {
        Gear::Lists
    } else if c.contains_key("Equipment") {
        Gear::Old
    } else {
        match data_version {
            Some(v) if v >= DV_EQUIPMENT => Gear::Named,
            Some(_) => Gear::Lists,
            None if StackForm::of(c, None) == StackForm::Numeric => Gear::Old,
            None => Gear::Lists,
        }
    }
}

/// The list and place a slot is kept at (all but `Named`); `None` where the layout has no such
/// slot (the old layout has no off hand).
fn gear_place(g: Gear, s: Slot) -> Option<(&'static str, usize, usize)> {
    // (list, place, length of the list)
    Some(match (g, s) {
        (Gear::Lists, Slot::Feet) => ("ArmorItems", 0, 4),
        (Gear::Lists, Slot::Legs) => ("ArmorItems", 1, 4),
        (Gear::Lists, Slot::Chest) => ("ArmorItems", 2, 4),
        (Gear::Lists, Slot::Head) => ("ArmorItems", 3, 4),
        (Gear::Lists, Slot::MainHand) => ("HandItems", 0, 2),
        (Gear::Lists, Slot::OffHand) => ("HandItems", 1, 2),
        (Gear::Old, Slot::MainHand) => ("Equipment", 0, 5),
        (Gear::Old, Slot::Feet) => ("Equipment", 1, 5),
        (Gear::Old, Slot::Legs) => ("Equipment", 2, 5),
        (Gear::Old, Slot::Chest) => ("Equipment", 3, 5),
        (Gear::Old, Slot::Head) => ("Equipment", 4, 5),
        (Gear::Bedrock, Slot::Head) => ("Armor", 0, 4),
        (Gear::Bedrock, Slot::Chest) => ("Armor", 1, 4),
        (Gear::Bedrock, Slot::Legs) => ("Armor", 2, 4),
        (Gear::Bedrock, Slot::Feet) => ("Armor", 3, 4),
        (Gear::Bedrock, Slot::MainHand) => ("Mainhand", 0, 1),
        (Gear::Bedrock, Slot::OffHand) => ("Offhand", 0, 1),
        _ => return None,
    })
}

/// What a mob wears and holds, slot by slot (the slots its layout has), as Java ids and counts.
pub fn equipment(c: &Compound, data_version: Option<i64>) -> Vec<(Slot, Option<(String, i64)>)> {
    let g = gear(c, data_version);
    Slot::ALL
        .iter()
        .filter_map(|&s| {
            let stack = if g == Gear::Named {
                c.get("equipment").and_then(|e| e.get(s.key())).and_then(Tag::as_compound)
            } else {
                let (list, place, _) = gear_place(g, s)?;
                c.get(list).and_then(Tag::as_list).and_then(|l| l.get(place)).and_then(Tag::as_compound)
            };
            Some((s, stack.and_then(stack_of)))
        })
        .collect()
}

/// The stack in a slot (empty or not), as it is kept.
pub fn slot_stack(c: &Compound, slot: Slot, data_version: Option<i64>) -> Option<&Compound> {
    let g = gear(c, data_version);
    if g == Gear::Named {
        return c.get("equipment").and_then(|e| e.get(slot.key())).and_then(Tag::as_compound);
    }
    let (list, place, _) = gear_place(g, slot)?;
    c.get(list).and_then(Tag::as_list).and_then(|l| l.get(place)).and_then(Tag::as_compound)
}

fn slot_stack_mut(c: &mut Compound, slot: Slot, data_version: Option<i64>) -> Option<&mut Compound> {
    let g = gear(c, data_version);
    if g == Gear::Named {
        return c.get_mut("equipment").and_then(Tag::as_compound_mut).and_then(|e| e.get_mut(slot.key())).and_then(Tag::as_compound_mut);
    }
    let (list, place, _) = gear_place(g, slot)?;
    match c.get_mut(list) {
        Some(Tag::List(l)) => l.get_mut(place).and_then(Tag::as_compound_mut),
        _ => None,
    }
}

/// Puts an item on (or, `None`, takes it off) a mob's slot, in its layout and its stacks' form.
/// What the stack there had besides its item and count (enchantments, a name...) is kept.
pub fn set_equipment(c: &mut Compound, slot: Slot, item: Option<(&str, i64)>, data_version: Option<i64>) {
    let g = gear(c, data_version);
    let form = StackForm::of(c, data_version);
    let before = slot_stack(c, slot, data_version).filter(|s| stack_of(s).is_some()).cloned();
    let stack = item.map(|(id, n)| {
        let mut s = make_stack(form, id, n.max(1));
        for (k, v) in before.iter().flatten() {
            if !s.contains_key(k) && !matches!(k.as_str(), "id" | "Name" | "Count" | "count" | "Damage" | "Block") {
                s.insert(k.clone(), v.clone());
            }
        }
        s
    });
    if g == Gear::Named {
        let e = c.entry("equipment".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
        if let Some(e) = e.as_compound_mut() {
            match stack {
                Some(s) => {
                    e.insert(slot.key().into(), Tag::Compound(s));
                }
                None => {
                    e.shift_remove(slot.key());
                }
            }
        }
        if c.get("equipment").and_then(Tag::as_compound).is_some_and(|e| e.is_empty()) {
            c.shift_remove("equipment");
        }
        return;
    }
    let Some((list, place, len)) = gear_place(g, slot) else { return };
    // The list with every place filled (an empty stack where nothing is), as the game keeps it.
    let mut items: Vec<Tag> = c.get(list).and_then(Tag::as_list).map(|l| l.iter().cloned().collect()).unwrap_or_default();
    while items.len() < len {
        items.push(Tag::Compound(empty_stack(form)));
    }
    items[place] = Tag::Compound(stack.unwrap_or_else(|| empty_stack(form)));
    c.insert(list.into(), Tag::List(List::from_tags(TagType::Compound, items).expect("compounds")));
}

// ---------------------------------------------------------------------------------------------
// Villager trades
// ---------------------------------------------------------------------------------------------

/// One trade as the trades screen shows it: what is paid (one or two stacks) and what is got,
/// and how often it has been and can be made.
#[derive(Clone, Debug, PartialEq)]
pub struct Trade {
    /// Buy, second buy, sell: (Java id, count), `None` where empty.
    pub items: [Option<(String, i64)>; 3],
    pub uses: i64,
    pub max_uses: i64,
    /// The stacks as saved (for their menus: enchantments, names...).
    pub stacks: [Option<Compound>; 3],
    /// The most a stack's count can be in this edition: 99 where it is an int (Java 1.20.5 on,
    /// as the game reads it), 127 where it is a byte.
    pub max_count: i64,
}

/// A change to a villager's trades.
#[derive(Clone, Debug, PartialEq)]
pub enum TradeEdit {
    /// Put an item in trade `n`'s place `which` (0 buy, 1 second buy, 2 sell), or (`None`, the
    /// second buy only) empty it.
    Item { n: usize, which: usize, id: Option<String> },
    Count { n: usize, which: usize, count: i64 },
    Uses { n: usize, uses: i64 },
    MaxUses { n: usize, max: i64 },
    Delete(usize),
    /// Anything else a stack's menu changes (enchantments, name, attributes...).
    Stack { n: usize, which: usize, edit: crate::item_edit::StackEdit },
    /// Move trade `n` up (or down) the list.
    Move { n: usize, up: bool },
    Add,
}

/// The keys of a recipe's three stacks in an edition.
fn trade_keys(bedrock: bool) -> [&'static str; 3] {
    if bedrock { ["buyA", "buyB", "sell"] } else { ["buy", "buyB", "sell"] }
}

/// Whether a mob trades (a villager or a wandering trader).
pub fn trades_with(java_id: &str) -> bool {
    matches!(java_id.trim_start_matches("minecraft:"), "villager" | "wandering_trader")
}

fn recipes(c: &Compound) -> Vec<Compound> {
    c.get("Offers").and_then(|o| o.get("Recipes")).and_then(Tag::as_list).map(|l| l.iter().filter_map(Tag::as_compound).cloned().collect()).unwrap_or_default()
}

/// A villager's trades.
pub fn trades(c: &Compound, data_version: Option<i64>) -> Vec<Trade> {
    let keys = trade_keys(is_bedrock(c));
    let max_count = if StackForm::of(c, data_version) == StackForm::Components { 99 } else { 127 };
    recipes(c)
        .iter()
        .map(|r| Trade {
            items: keys.map(|k| r.get(k).and_then(Tag::as_compound).and_then(stack_of)),
            uses: r.get("uses").and_then(Tag::as_i64).unwrap_or(0),
            max_uses: r.get("maxUses").and_then(Tag::as_i64).unwrap_or(12),
            stacks: keys.map(|k| r.get(k).and_then(Tag::as_compound).filter(|s| stack_of(s).is_some()).cloned()),
            max_count,
        })
        .collect()
}

/// A new trade: one emerald for one emerald (to be changed), laid out as the villager's others
/// are (or as its edition lays a fresh one out).
fn new_recipe(c: &Compound, data_version: Option<i64>, template: Option<&Compound>) -> Compound {
    let bedrock = is_bedrock(c);
    let form = StackForm::of(c, data_version);
    let keys = trade_keys(bedrock);
    let mut r = match template {
        Some(t) => t.clone(),
        None => {
            let mut r = Compound::new();
            if bedrock {
                for (k, v) in [("buyCountA", Tag::Int(1)), ("buyCountB", Tag::Int(0)), ("tier", Tag::Int(0)), ("traderExp", Tag::Int(1)), ("demand", Tag::Int(0))] {
                    r.insert(k.into(), v);
                }
                r.insert("priceMultiplierA".into(), Tag::Float(0.05));
                r.insert("priceMultiplierB".into(), Tag::Float(0.0));
                r.insert("rewardExp".into(), Tag::Byte(1));
            } else {
                r.insert("rewardExp".into(), Tag::Byte(1));
                if data_version.is_some() {
                    for (k, v) in [("xp", Tag::Int(1)), ("specialPrice", Tag::Int(0)), ("demand", Tag::Int(0))] {
                        r.insert(k.into(), v);
                    }
                    r.insert("priceMultiplier".into(), Tag::Float(0.05));
                }
            }
            r
        }
    };
    r.insert(keys[0].into(), Tag::Compound(make_stack(form, "minecraft:emerald", 1)));
    r.shift_remove(keys[1]);
    r.insert(keys[2].into(), Tag::Compound(make_stack(form, "minecraft:emerald", 1)));
    r.insert("uses".into(), Tag::Int(0));
    r.insert("maxUses".into(), Tag::Int(template.and_then(|t| t.get("maxUses")).and_then(Tag::as_i64).unwrap_or(12) as i32));
    if bedrock {
        r.insert("buyCountA".into(), Tag::Int(1));
        r.insert("buyCountB".into(), Tag::Int(0));
    }
    r
}

/// Applies a change to a villager's trades.
pub fn edit_trades(c: &mut Compound, edit: TradeEdit, data_version: Option<i64>) {
    let bedrock = is_bedrock(c);
    let keys = trade_keys(bedrock);
    let form = StackForm::of(c, data_version);
    let mut list = recipes(c);
    match edit {
        TradeEdit::Item { n, which, id } => {
            let Some(r) = list.get_mut(n) else { return };
            match id {
                Some(id) => {
                    // The count it had, if there was one.
                    let count = r.get(keys[which]).and_then(Tag::as_compound).and_then(stack_of).map_or(1, |(_, n)| n);
                    r.insert(keys[which].into(), Tag::Compound(make_stack(form, &id, count)));
                }
                None if which == 1 => {
                    r.shift_remove(keys[1]);
                }
                None => {}
            }
        }
        TradeEdit::Count { n, which, count } => {
            let Some(r) = list.get_mut(n) else { return };
            let Some(s) = r.get(keys[which]).and_then(Tag::as_compound).cloned() else { return };
            let most = if s.contains_key("count") { 99 } else { 127 };
            r.insert(keys[which].into(), Tag::Compound(with_count(&s, count.clamp(1, most))));
        }
        TradeEdit::Stack { n, which, edit } => {
            let Some(r) = list.get_mut(n) else { return };
            let Some(mut s) = r.get(keys[which]).and_then(Tag::as_compound).cloned() else { return };
            edit.apply(&mut s, data_version);
            r.insert(keys[which].into(), Tag::Compound(s));
        }
        TradeEdit::Uses { n, uses } => {
            if let Some(r) = list.get_mut(n) {
                r.insert("uses".into(), Tag::Int(uses.max(0) as i32));
            }
        }
        TradeEdit::MaxUses { n, max } => {
            if let Some(r) = list.get_mut(n) {
                r.insert("maxUses".into(), Tag::Int(max.max(1) as i32));
            }
        }
        TradeEdit::Delete(n) => {
            if n < list.len() {
                list.remove(n);
            }
        }
        TradeEdit::Move { n, up } => {
            let to = if up { n.checked_sub(1) } else { Some(n + 1) };
            if let Some(to) = to.filter(|t| *t < list.len() && n < list.len()) {
                list.swap(n, to);
            }
        }
        TradeEdit::Add => {
            let r = new_recipe(c, data_version, list.first());
            list.push(r);
        }
    }
    // Bedrock keeps how many of each is paid beside the stacks, too.
    if bedrock {
        for r in &mut list {
            for (k, count_key) in [("buyA", "buyCountA"), ("buyB", "buyCountB")] {
                let n = r.get(k).and_then(Tag::as_compound).and_then(stack_of).map_or(0, |(_, n)| n);
                r.insert(count_key.into(), Tag::Int(n as i32));
            }
        }
    }
    let offers = c.entry("Offers".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
    if let Some(o) = offers.as_compound_mut() {
        let tags: Vec<Tag> = list.into_iter().map(Tag::Compound).collect();
        let l = if tags.is_empty() { List::new(TagType::End) } else { List::from_tags(TagType::Compound, tags).expect("compounds") };
        o.insert("Recipes".into(), Tag::List(l));
    }
}

// ---------------------------------------------------------------------------------------------
// End gateways
// ---------------------------------------------------------------------------------------------

/// Where an end gateway sends things, if it says.
pub fn gateway_exit(c: &Compound) -> Option<[i64; 3]> {
    match c.get("exit_portal").or_else(|| c.get("ExitPortal"))? {
        Tag::IntArray(a) if a.len() == 3 => Some([a[0] as i64, a[1] as i64, a[2] as i64]),
        Tag::List(l) if l.len() == 3 => Some([0, 1, 2].map(|i| l.get(i).and_then(Tag::as_i64).unwrap_or(0))),
        Tag::Compound(p) => Some(["X", "Y", "Z"].map(|k| p.get(k).and_then(Tag::as_i64).unwrap_or(0))),
        _ => None,
    }
}

/// Sets where an end gateway sends things, in the form it (or its edition) keeps it in.
pub fn set_gateway_exit(c: &mut Compound, at: [i64; 3], data_version: Option<i64>) {
    let [x, y, z] = at.map(|v| v as i32);
    let bedrock = c.get("id").and_then(Tag::as_str) == Some("EndGateway");
    let form = match (c.get("exit_portal"), c.get("ExitPortal")) {
        (Some(_), _) => 0,
        (None, Some(Tag::List(_))) => 1,
        (None, Some(_)) => 2,
        (None, None) if bedrock => 1,
        (None, None) if data_version.is_some_and(|v| v >= DV_COMPONENTS) => 0,
        (None, None) => 2,
    };
    match form {
        0 => {
            c.insert("exit_portal".into(), Tag::IntArray(vec![x, y, z]));
        }
        1 => {
            let l = List::from_tags(TagType::Int, vec![Tag::Int(x), Tag::Int(y), Tag::Int(z)]).expect("ints");
            c.insert("ExitPortal".into(), Tag::List(l));
        }
        _ => {
            let p: Compound = [("X", x), ("Y", y), ("Z", z)].into_iter().map(|(k, v)| (k.to_string(), Tag::Int(v))).collect();
            c.insert("ExitPortal".into(), Tag::Compound(p));
        }
    }
}

/// Whether an end gateway sends things to exactly its exit; `None` where the edition keeps no
/// such thing (Bedrock).
pub fn gateway_exact(c: &Compound) -> Option<bool> {
    (c.get("id").and_then(Tag::as_str) != Some("EndGateway")).then(|| flag(c, "ExactTeleport"))
}

pub fn set_gateway_exact(c: &mut Compound, on: bool) {
    set_flag(c, "ExactTeleport", on);
}

// ---------------------------------------------------------------------------------------------
// What a mob drops
// ---------------------------------------------------------------------------------------------

/// The game's chance of a mob dropping what a slot holds, where none is saved.
pub const DEFAULT_DROP: f64 = 0.085;

/// The list and place a slot's drop chance is kept at, by layout (as [`gear_place`]).
fn drop_place(g: Gear, s: Slot) -> Option<(&'static str, usize, usize)> {
    let (list, place, len) = gear_place(g, s)?;
    Some(match list {
        "ArmorItems" => ("ArmorDropChances", place, len),
        "HandItems" => ("HandDropChances", place, len),
        "Equipment" => ("DropChances", place, len),
        _ => return None,
    })
}

/// What a mob does with things when it dies and finds: each slot's chance of dropping what it
/// holds (0 to 1; above 1, always and undamaged), its loot table (`None`: the game's own for
/// it), and whether it picks things up. `None` where the edition keeps none of it (Bedrock).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drops {
    pub chances: Vec<(Slot, f64)>,
    pub loot_table: Option<String>,
    /// Java only (the console editions keep no loot table).
    pub has_loot_table: bool,
    pub pick_up: bool,
}

pub fn drops(c: &Compound, data_version: Option<i64>) -> Option<Drops> {
    if is_bedrock(c) {
        return None;
    }
    let g = gear(c, data_version);
    let chances = Slot::ALL
        .iter()
        .filter_map(|&s| {
            let v = if g == Gear::Named {
                Some(c.get("drop_chances").and_then(|d| d.get(s.key())).and_then(Tag::as_f64).unwrap_or(DEFAULT_DROP))
            } else {
                let (list, place, _) = drop_place(g, s)?;
                Some(c.get(list).and_then(Tag::as_list).and_then(|l| l.get(place)).and_then(Tag::as_f64).unwrap_or(DEFAULT_DROP))
            };
            v.map(|v| (s, v))
        })
        .collect();
    let java = c.get("id").and_then(Tag::as_str).is_some_and(|i| i.contains(':'));
    Some(Drops {
        chances,
        loot_table: c.get("DeathLootTable").and_then(Tag::as_str).map(str::to_string),
        has_loot_table: java,
        pick_up: flag(c, "CanPickUpLoot"),
    })
}

/// Sets a slot's chance of dropping what it holds, in the mob's layout.
pub fn set_drop_chance(c: &mut Compound, slot: Slot, chance: f64, data_version: Option<i64>) {
    let g = gear(c, data_version);
    let chance = chance as f32;
    if g == Gear::Named {
        let d = c.entry("drop_chances".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
        if let Some(d) = d.as_compound_mut() {
            if (chance as f64 - DEFAULT_DROP).abs() < 1e-6 {
                d.shift_remove(slot.key());
            } else {
                d.insert(slot.key().into(), Tag::Float(chance));
            }
        }
        if c.get("drop_chances").and_then(Tag::as_compound).is_some_and(|d| d.is_empty()) {
            c.shift_remove("drop_chances");
        }
        return;
    }
    let Some((list, place, len)) = drop_place(g, slot) else { return };
    let mut values: Vec<Tag> = c.get(list).and_then(Tag::as_list).map(|l| l.iter().cloned().collect()).unwrap_or_default();
    while values.len() < len {
        values.push(Tag::Float(DEFAULT_DROP as f32));
    }
    values[place] = Tag::Float(chance);
    c.insert(list.into(), Tag::List(List::from_tags(TagType::Float, values).expect("floats")));
}

/// Gives a mob a loot table of its own (`None`, or nothing typed: the game's own for it).
pub fn set_loot_table(c: &mut Compound, table: Option<&str>) {
    match table.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => {
            let t = if t.contains(':') { t.to_string() } else { format!("minecraft:{t}") };
            c.insert("DeathLootTable".into(), Tag::String(t));
        }
        None => {
            c.shift_remove("DeathLootTable");
            c.shift_remove("DeathLootTableSeed");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Spawners, locks, shulkers' colors
// ---------------------------------------------------------------------------------------------

/// Java 1.18 (data version 2860), where a spawner's mob is `SpawnData.entity`.
const DV_SPAWN_ENTITY: i64 = 2860;
/// Java 1.21.2 (data version 4080), where a lock is an item predicate (`lock`), and 1.21.5
/// (4325), where names in it are text as NBT rather than JSON.
const DV_LOCK_PREDICATE: i64 = 4080;
const DV_TEXT_NBT: i64 = 4325;

fn raw_id(c: &Compound) -> &str {
    c.get("id").and_then(Tag::as_str).unwrap_or("")
}

/// A Bedrock block entity (they all have `isMovable`; Bedrock spawners name their mob in
/// `EntityIdentifier`).
fn bedrock_block(c: &Compound) -> bool {
    c.contains_key("isMovable") || c.contains_key("EntityIdentifier")
}

/// A modern id from an old name (`Zombie`, `PigZombie`) or a modern one.
fn modern_entity(name: &str) -> String {
    if name.contains(':') {
        return name.to_string();
    }
    mctk_mapping::legacy::legacy().entity(name).map_or_else(|| format!("minecraft:{}", name.to_lowercase()), str::to_string)
}

/// Whether a block entity is a mob spawner, in any edition.
pub fn is_spawner(c: &Compound) -> bool {
    matches!(raw_id(c).trim_start_matches("minecraft:"), "mob_spawner" | "MobSpawner")
}

/// The mob a spawner spawns (a Java id), `None` when it has none. Only for spawners.
pub fn spawner_mob(c: &Compound) -> Option<String> {
    if let Some(t) = c.get("EntityIdentifier").and_then(Tag::as_str).filter(|t| !t.is_empty()) {
        let java = match t {
            "minecraft:zombie_pigman" => "minecraft:zombified_piglin",
            "minecraft:evocation_illager" => "minecraft:evoker",
            other => other,
        };
        return Some(java.to_string());
    }
    let data = c.get("SpawnData").and_then(Tag::as_compound);
    if let Some(id) = data.and_then(|d| d.get("entity")).and_then(|e| e.get("id")).and_then(Tag::as_str) {
        return Some(modern_entity(id));
    }
    if let Some(id) = data.and_then(|d| d.get("id")).and_then(Tag::as_str) {
        return Some(modern_entity(id));
    }
    c.get("EntityId").and_then(Tag::as_str).filter(|t| !t.is_empty()).map(modern_entity)
}

/// Makes a spawner spawn another mob (a Java id), in its edition's layout. The list of mobs it
/// would pick from instead (`SpawnPotentials`) is emptied, so it spawns this one.
pub fn set_spawner_mob(c: &mut Compound, java_id: &str, data_version: Option<i64>) {
    let id = if java_id.contains(':') { java_id.to_string() } else { format!("minecraft:{java_id}") };
    if bedrock_block(c) {
        let bedrock = match id.as_str() {
            "minecraft:zombified_piglin" => "minecraft:zombie_pigman",
            "minecraft:evoker" => "minecraft:evocation_illager",
            other => other,
        };
        c.insert("EntityIdentifier".into(), Tag::String(bedrock.to_string()));
        return;
    }
    // Console and Java before 1.9: the old name, in `EntityId`.
    if c.contains_key("EntityId") {
        let old = mctk_mapping::legacy::legacy().old_entity_name(&id).map_or_else(|| id.trim_start_matches("minecraft:").to_string(), str::to_string);
        c.insert("EntityId".into(), Tag::String(old.clone()));
        if let Some(Tag::Compound(d)) = c.get_mut("SpawnData") {
            d.insert("id".into(), Tag::String(old));
        }
    } else {
        let entity_form = c.get("SpawnData").and_then(|d| d.get("entity")).is_some() || data_version.is_none_or(|v| v >= DV_SPAWN_ENTITY);
        let mut data = Compound::new();
        if entity_form {
            let mut e = Compound::new();
            e.insert("id".into(), Tag::String(id));
            data.insert("entity".into(), Tag::Compound(e));
        } else {
            data.insert("id".into(), Tag::String(id));
        }
        c.insert("SpawnData".into(), Tag::Compound(data));
    }
    if c.contains_key("SpawnPotentials") {
        c.insert("SpawnPotentials".into(), Tag::List(List::new(TagType::Compound)));
    }
}

/// Whether a block entity is a container the game lets a key lock (Java and the console
/// editions; Bedrock has no locks).
pub fn lockable(c: &Compound) -> bool {
    if bedrock_block(c) {
        return false;
    }
    let n = raw_id(c).trim_start_matches("minecraft:").to_ascii_lowercase();
    n.ends_with("chest")
        || n.ends_with("shulker_box")
        || matches!(n.as_str(), "barrel" | "furnace" | "smoker" | "blast_furnace" | "hopper" | "dispenser" | "dropper" | "trap" | "brewing_stand" | "cauldron" | "beacon" | "crafter")
}

/// The name of the key that opens a locked container ("" when it is not locked).
pub fn lock(c: &Compound) -> String {
    if let Some(name) = c.get("Lock").and_then(Tag::as_str) {
        return name.to_string();
    }
    let Some(n) = c.get("lock").and_then(|l| l.get("components")).and_then(|t| t.get("minecraft:custom_name")) else {
        return String::new();
    };
    match n {
        // JSON text ("\"Key\"", or {"text": "Key"}) before 1.21.5, plain text after.
        Tag::String(s) => serde_json::from_str::<serde_json::Value>(s).ok().and_then(|v| match v {
            serde_json::Value::String(t) => Some(t),
            serde_json::Value::Object(o) => o.get("text").and_then(|t| t.as_str()).map(str::to_string),
            _ => None,
        }).unwrap_or_else(|| s.clone()),
        Tag::Compound(t) => t.get("text").and_then(Tag::as_str).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// Locks a container so only an item named `key` opens it (`""`: unlocks it).
pub fn set_lock(c: &mut Compound, key: &str, data_version: Option<i64>) {
    c.shift_remove("Lock");
    c.shift_remove("lock");
    if key.is_empty() {
        return;
    }
    let predicate = c.contains_key("components") || data_version.is_some_and(|v| v >= DV_LOCK_PREDICATE);
    if !predicate {
        c.insert("Lock".into(), Tag::String(key.to_string()));
        return;
    }
    let name = if data_version.is_none_or(|v| v >= DV_TEXT_NBT) { key.to_string() } else { serde_json::Value::String(key.to_string()).to_string() };
    let mut comps = Compound::new();
    comps.insert("minecraft:custom_name".into(), Tag::String(name));
    let mut l = Compound::new();
    l.insert("components".into(), Tag::Compound(comps));
    c.insert("lock".into(), Tag::Compound(l));
}

// ---------------------------------------------------------------------------------------------
// Beacons
// ---------------------------------------------------------------------------------------------

/// Java 1.20.2 (data version 3578), where a beacon keeps its powers as effect names
/// (`primary_effect`, `secondary_effect`) and not as numbers (`Primary`, `Secondary`).
const DV_BEACON_NAMES: i64 = 3578;

/// An effect's number in the games before 1.20.2 (only the first 33 effects have one).
fn effect_number(name: &str) -> Option<i64> {
    crate::effects::cloud_effect_names().iter().position(|n| *n == name).filter(|i| *i < 33).map(|i| i as i64 + 1)
}

/// The effect a number stands for in those games.
fn effect_of_number(n: i64) -> Option<&'static str> {
    usize::try_from(n - 1).ok().filter(|i| *i < 33).and_then(|i| crate::effects::cloud_effect_names().get(i).copied())
}

/// Whether a block entity is a beacon (Java and the console editions).
pub fn is_beacon(c: &Compound) -> bool {
    !bedrock_block(c) && raw_id(c).trim_start_matches("minecraft:").eq_ignore_ascii_case("beacon")
}

/// A beacon's primary and secondary power, as effect names (`speed`), and how many levels its
/// pyramid had when it was saved. The game works the levels out again from the blocks below the
/// beacon, so they only say what it was.
pub fn beacon_powers(c: &Compound) -> (Option<String>, Option<String>) {
    let power = |named: &str, numbered: &str| -> Option<String> {
        if let Some(n) = c.get(named).and_then(Tag::as_str) {
            return Some(n.trim_start_matches("minecraft:").to_string());
        }
        let n = c.get(numbered).and_then(Tag::as_i64).filter(|n| *n > 0)?;
        effect_of_number(n).map(str::to_string)
    };
    (power("primary_effect", "Primary"), power("secondary_effect", "Secondary"))
}

/// How many levels the pyramid under a beacon had when it was saved (0 to 4).
pub fn beacon_levels(c: &Compound) -> i64 {
    c.get("Levels").and_then(Tag::as_i64).unwrap_or(0)
}

/// What a beacon at `levels` gives, as the game works it out: how far (blocks), how long (seconds)
/// each time it is renewed, and the first power's level (II when the second power repeats it).
pub fn beacon_reach(levels: i64, primary: Option<&str>, secondary: Option<&str>) -> (i64, i64, i64) {
    let level = if levels >= 4 && primary.is_some() && primary == secondary { 2 } else { 1 };
    (10 + levels * 10, 9 + levels * 2, level)
}

/// Sets a beacon's primary (or `secondary`) power to an effect name (`None`: no power), in the form
/// the world keeps it in.
pub fn set_beacon_power(c: &mut Compound, secondary: bool, effect: Option<&str>, data_version: Option<i64>) {
    let (named, numbered) = if secondary { ("secondary_effect", "Secondary") } else { ("primary_effect", "Primary") };
    c.shift_remove(named);
    c.shift_remove(numbered);
    let Some(effect) = effect.map(|e| e.trim_start_matches("minecraft:")) else { return };
    let names = match data_version {
        Some(v) => v >= DV_BEACON_NAMES,
        None => !c.contains_key("Primary") && !c.contains_key("Secondary"),
    };
    if names {
        c.insert(named.into(), Tag::String(format!("minecraft:{effect}")));
    } else if let Some(n) = effect_number(effect) {
        c.insert(numbered.into(), Tag::Int(n as i32));
    }
}

/// The jobs a villager can have, each with the block it works at (for its picture).
pub const PROFESSIONS: [(&str, &str); 15] = [
    ("none", "barrier"),
    ("armorer", "blast_furnace"),
    ("butcher", "smoker"),
    ("cartographer", "cartography_table"),
    ("cleric", "brewing_stand"),
    ("farmer", "composter"),
    ("fisherman", "barrel"),
    ("fletcher", "fletching_table"),
    ("leatherworker", "cauldron"),
    ("librarian", "lectern"),
    ("mason", "stonecutter"),
    ("nitwit", "bell"),
    ("shepherd", "loom"),
    ("toolsmith", "smithing_table"),
    ("weaponsmith", "grindstone"),
];

/// A villager's (or zombie villager's) job, by name (`farmer`; `none` for none); `None` for
/// anything that has no villager data.
pub fn profession(c: &Compound) -> Option<String> {
    let data = c.get("VillagerData").and_then(Tag::as_compound)?;
    Some(data.get("profession").and_then(Tag::as_str).map_or("none", |p| p.trim_start_matches("minecraft:")).to_string())
}

/// Gives a villager a job (`none`: takes it away). Its level and trades are kept.
pub fn set_profession(c: &mut Compound, job: &str) {
    let Some(Tag::Compound(data)) = c.get_mut("VillagerData") else { return };
    data.insert("profession".into(), Tag::String(format!("minecraft:{job}")));
    if data.get("level").and_then(Tag::as_i64).is_none_or(|l| l < 1) {
        data.insert("level".into(), Tag::Int(1));
    }
}

/// A shulker's color (0 white ... 15 black, 16 its own purple), for shulkers only.
pub fn shulker_color(c: &Compound, java_id: &str) -> Option<i64> {
    java_id.ends_with(":shulker").then(|| c.get("Color").and_then(Tag::as_i64).unwrap_or(16))
}

/// The game's 16 colors, by number.
pub const COLORS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// One change from the menu, applied to the compound it was made on.
#[derive(Clone, Debug, PartialEq)]
pub enum MobEdit {
    Invulnerable(bool),
    NoAi(bool),
    Invisible(bool),
    Gravity(bool),
    /// Put `count` of an item in a slot, or (`None`) empty it.
    Equip(Slot, Option<(String, i64)>),
    GatewayExit([i64; 3]),
    GatewayExact(bool),
    Trades(TradeEdit),
    /// A slot's chance of dropping what it holds (0 to 1; above 1, always).
    DropChance(Slot, f64),
    /// Its own loot table, or (`None`) the game's.
    LootTable(Option<String>),
    PickUp(bool),
    /// A change to the stack in a slot (its enchantments, attributes, name...).
    EquipStack(Slot, crate::item_edit::StackEdit),
    /// A change to its status effects.
    Effect(crate::effects::EffectEdit),
    /// A spawner's mob (a Java id).
    SpawnerMob(String),
    /// A container's lock: the name of the key that opens it ("" unlocks it).
    Lock(String),
    /// A shulker's color (16: its own).
    ShulkerColor(i64),
    /// A beacon's primary (or secondary) power: an effect name, or (`None`) none.
    BeaconPower { secondary: bool, effect: Option<String> },
    /// A villager's job (the name `none` is no job).
    Profession(String),
}

impl MobEdit {
    /// What the edit is called in the history.
    pub fn note(&self) -> String {
        match self {
            MobEdit::Invulnerable(on) => if *on { "Made it invulnerable" } else { "Made it vulnerable" }.into(),
            MobEdit::NoAi(on) => if *on { "Took its AI away" } else { "Gave it its AI back" }.into(),
            MobEdit::Invisible(on) => if *on { "Made it invisible" } else { "Made it visible" }.into(),
            MobEdit::Gravity(on) => if *on { "Gave it gravity" } else { "Took its gravity away" }.into(),
            MobEdit::Equip(s, Some((id, _))) => format!("Put {} on its {}", crate::item_picker::display_name(id), s.name().to_lowercase()),
            MobEdit::Equip(s, None) => format!("Took off what its {} had", s.name().to_lowercase()),
            MobEdit::GatewayExit([x, y, z]) => format!("Sent the gateway to {x}, {y}, {z}"),
            MobEdit::GatewayExact(on) => if *on { "Made the gateway exact" } else { "Made the gateway inexact" }.into(),
            MobEdit::Trades(_) => "Changed a trade".into(),
            MobEdit::DropChance(s, v) => format!("Set its {} drop chance to {}%", s.name().to_lowercase(), (v.min(1.0) * 1000.0).round() / 10.0),
            MobEdit::LootTable(Some(t)) => format!("Gave it the loot table {t}"),
            MobEdit::LootTable(None) => "Gave it its own loot back".into(),
            MobEdit::PickUp(on) => if *on { "Let it pick up loot" } else { "Stopped it picking up loot" }.into(),
            MobEdit::EquipStack(_, e) => e.note(),
            MobEdit::Effect(e) => e.note(),
            MobEdit::SpawnerMob(id) => format!("Made the spawner spawn {}", crate::item_picker::display_name(id)),
            MobEdit::Lock(k) if k.is_empty() => "Unlocked it".into(),
            MobEdit::Lock(k) => format!("Locked it with the key \u{201c}{k}\u{201d}"),
            MobEdit::ShulkerColor(16) => "Gave the shulker its own color".into(),
            MobEdit::ShulkerColor(c) => format!("Made the shulker {}", crate::item_picker::display_name(COLORS.get(*c as usize).copied().unwrap_or("purple"))),
            MobEdit::BeaconPower { secondary, effect: Some(e) } => {
                format!("Set the beacon's {} power to {}", if *secondary { "secondary" } else { "primary" }, crate::item_picker::display_name(e))
            }
            MobEdit::Profession(p) if p == "none" => "Took the villager's job away".into(),
            MobEdit::Profession(p) => format!("Made the villager a {}", crate::item_picker::display_name(p)),
            MobEdit::BeaconPower { secondary, effect: None } => format!("Took the beacon's {} power away", if *secondary { "secondary" } else { "primary" }),
        }
    }

    pub fn apply(&self, c: &mut Compound, data_version: Option<i64>) {
        match self {
            MobEdit::Invulnerable(on) => set_invulnerable(c, *on),
            MobEdit::NoAi(on) => set_no_ai(c, *on),
            MobEdit::Invisible(on) => set_invisible(c, *on),
            MobEdit::Gravity(on) => set_gravity(c, *on),
            MobEdit::Equip(s, item) => set_equipment(c, *s, item.as_ref().map(|(id, n)| (id.as_str(), *n)), data_version),
            MobEdit::GatewayExit(at) => set_gateway_exit(c, *at, data_version),
            MobEdit::GatewayExact(on) => set_gateway_exact(c, *on),
            MobEdit::Trades(e) => edit_trades(c, e.clone(), data_version),
            MobEdit::DropChance(s, v) => set_drop_chance(c, *s, *v, data_version),
            MobEdit::LootTable(t) => set_loot_table(c, t.as_deref()),
            MobEdit::PickUp(on) => set_flag(c, "CanPickUpLoot", *on),
            MobEdit::EquipStack(s, e) => {
                if let Some(stack) = slot_stack_mut(c, *s, data_version) {
                    e.apply(stack, data_version);
                }
            }
            MobEdit::Effect(e) => e.apply(c, crate::effects::Holder::Mob, data_version),
            MobEdit::SpawnerMob(id) => set_spawner_mob(c, id, data_version),
            MobEdit::Lock(k) => set_lock(c, k, data_version),
            MobEdit::ShulkerColor(v) => {
                c.insert("Color".into(), Tag::Byte(*v as i8));
            }
            MobEdit::BeaconPower { secondary, effect } => set_beacon_power(c, *secondary, effect.as_deref(), data_version),
            MobEdit::Profession(p) => set_profession(c, p),
        }
    }
}

#[cfg(test)]
/// Sets a mob's health where its edition keeps it.
pub fn set_health(c: &mut Compound, value: f64) {
    if is_bedrock(c) {
        if let Some(Tag::List(l)) = c.get_mut("Attributes") {
            for a in l.iter_mut() {
                if let Some(a) = a.as_compound_mut() {
                    if a.get("Name").and_then(Tag::as_str) == Some("minecraft:health") {
                        a.insert("Current".into(), Tag::Float(value as f32));
                    }
                }
            }
        }
        return;
    }
    // Before 1.9 health was a short, with the exact value in `HealF` beside it.
    if c.contains_key("HealF") {
        c.insert("HealF".into(), Tag::Float(value as f32));
        c.insert("Health".into(), Tag::Short(value.round() as i16));
    } else if matches!(c.get("Health"), Some(Tag::Short(_))) {
        c.insert("Health".into(), Tag::Short(value.round() as i16));
    } else {
        c.insert("Health".into(), Tag::Float(value as f32));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Tag {
        Tag::String(v.into())
    }
    fn of(pairs: &[(&str, Tag)]) -> Compound {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }
    fn mob_java() -> Compound {
        of(&[("id", s("minecraft:zombie")), ("Health", Tag::Float(20.0)), ("Pos", Tag::List(List::new(TagType::Double)))])
    }

    #[test]
    fn spawners_spawn_the_mob_picked_in_each_layout() {
        // Java 1.21: `SpawnData.entity`, and the list it would pick from instead emptied.
        let mut c = of(&[("id", s("minecraft:mob_spawner")), ("x", Tag::Int(0)), ("y", Tag::Int(0)), ("z", Tag::Int(0))]);
        c.insert("SpawnPotentials".into(), Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(Compound::new())]).unwrap()));
        assert!(is_spawner(&c));
        set_spawner_mob(&mut c, "minecraft:blaze", Some(4000));
        assert_eq!(spawner_mob(&c).as_deref(), Some("minecraft:blaze"));
        assert_eq!(c.get("SpawnPotentials").and_then(Tag::as_list).map(List::len), Some(0));
        // Console: the old name.
        let mut c = of(&[("id", s("MobSpawner")), ("EntityId", s("Pig"))]);
        assert_eq!(spawner_mob(&c).as_deref(), Some("minecraft:pig"));
        set_spawner_mob(&mut c, "minecraft:zombified_piglin", None);
        assert_eq!(c.get("EntityId"), Some(&s("PigZombie")));
        // Bedrock: `EntityIdentifier`.
        let mut b = of(&[("id", s("MobSpawner")), ("isMovable", Tag::Byte(1)), ("EntityIdentifier", s("minecraft:pig"))]);
        set_spawner_mob(&mut b, "minecraft:cave_spider", None);
        assert_eq!(b.get("EntityIdentifier"), Some(&s("minecraft:cave_spider")));
    }

    #[test]
    fn containers_lock_with_a_key_name_by_version() {
        let mut c = of(&[("id", s("minecraft:chest")), ("x", Tag::Int(0)), ("y", Tag::Int(0)), ("z", Tag::Int(0))]);
        assert!(lockable(&c));
        set_lock(&mut c, "Gold Key", Some(3700));
        assert_eq!(c.get("Lock"), Some(&s("Gold Key")));
        assert_eq!(lock(&c), "Gold Key");
        // 1.21.2 to 1.21.4: an item predicate, the name as JSON; 1.21.5 on, as text.
        set_lock(&mut c, "Gold Key", Some(4100));
        assert!(!c.contains_key("Lock"));
        assert_eq!(lock(&c), "Gold Key");
        set_lock(&mut c, "Gold Key", Some(4400));
        assert_eq!(c.get("lock").and_then(|l| l.get("components")).and_then(|t| t.get("minecraft:custom_name")), Some(&s("Gold Key")));
        set_lock(&mut c, "", Some(4400));
        assert!(!c.contains_key("lock") && lock(&c).is_empty());
        // Bedrock has no locks.
        assert!(!lockable(&of(&[("id", s("Chest")), ("isMovable", Tag::Byte(1))])));
    }

    #[test]
    fn a_beacons_powers_are_read_and_set_in_the_form_of_its_version() {
        let mut modern = of(&[("id", s("minecraft:beacon")), ("Levels", Tag::Int(4)), ("primary_effect", s("minecraft:speed"))]);
        assert!(is_beacon(&modern));
        assert_eq!(beacon_powers(&modern), (Some("speed".into()), None));
        set_beacon_power(&mut modern, true, Some("minecraft:speed"), Some(4400));
        assert_eq!(beacon_powers(&modern), (Some("speed".into()), Some("speed".into())));
        assert_eq!(beacon_reach(beacon_levels(&modern), Some("speed"), Some("speed")), (50, 17, 2));
        set_beacon_power(&mut modern, false, None, Some(4400));
        assert_eq!(beacon_powers(&modern).0, None);
        // Before 1.20.2 and in the console games the powers are numbers.
        let mut old = of(&[("id", s("Beacon")), ("Levels", Tag::Int(1)), ("Primary", Tag::Int(11))]);
        assert_eq!(beacon_powers(&old).0.as_deref(), Some("resistance"));
        set_beacon_power(&mut old, true, Some("regeneration"), Some(3465));
        assert_eq!(old.get("Secondary"), Some(&Tag::Int(10)));
        set_beacon_power(&mut old, false, Some("haste"), None);
        assert_eq!(old.get("Primary"), Some(&Tag::Int(3)));
        assert!(!old.contains_key("primary_effect"));
        assert!(lockable(&old));
    }

    #[test]
    fn a_shulker_box_changes_color_and_keeps_what_it_holds() {
        let mut c = of(&[("id", s("minecraft:shulker_box")), ("count", Tag::Int(1))]);
        let mut comps = Compound::new();
        comps.insert("minecraft:container".into(), Tag::List(List::new(TagType::Compound)));
        c.insert("components".into(), Tag::Compound(comps.clone()));
        crate::item_edit::ItemEdit::Recolor("minecraft:red_shulker_box".into()).apply(&mut c, Some(4000));
        assert_eq!(c.get("id"), Some(&s("minecraft:red_shulker_box")));
        assert_eq!(c.get("components"), Some(&Tag::Compound(comps)));
    }

    #[test]
    fn flags_toggle_and_leave_nothing_when_off() {
        let mut c = mob_java();
        set_invulnerable(&mut c, true);
        set_no_ai(&mut c, true);
        assert!(invulnerable(&c) && no_ai(&c) == Some(true));
        set_invulnerable(&mut c, false);
        set_no_ai(&mut c, false);
        assert!(!c.contains_key("Invulnerable") && !c.contains_key("NoAI"));
        // Gravity: on unless `NoGravity` says otherwise.
        assert_eq!(gravity(&c), Some(true));
        set_gravity(&mut c, false);
        assert_eq!((gravity(&c), c.get("NoGravity")), (Some(false), Some(&Tag::Byte(1))));
        set_gravity(&mut c, true);
        assert!(!c.contains_key("NoGravity"));
        // Bedrock keeps no such thing as no AI; invisibility only on stands and frames.
        assert_eq!(no_ai(&of(&[("identifier", s("minecraft:zombie"))])), None);
        assert_eq!(invisible(&c, "minecraft:zombie"), None);
        assert_eq!(invisible(&of(&[("id", s("minecraft:armor_stand"))]), "minecraft:armor_stand"), Some(false));
    }

    #[test]
    fn health_goes_where_each_edition_keeps_it() {
        let mut c = mob_java();
        set_health(&mut c, 7.5);
        assert_eq!(c.get("Health"), Some(&Tag::Float(7.5)));
        // Bedrock: the attribute's current value.
        let attr = of(&[("Name", s("minecraft:health")), ("Current", Tag::Float(20.0)), ("Max", Tag::Float(20.0))]);
        let mut b = of(&[("identifier", s("minecraft:cow")), ("Attributes", Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(attr)]).unwrap()))]);
        set_health(&mut b, 3.0);
        assert_eq!(health(&b), Some(3.0));
        // Old: the short and its exact value.
        let mut o = of(&[("id", s("Zombie")), ("Health", Tag::Short(20)), ("HealF", Tag::Float(20.0))]);
        set_health(&mut o, 4.5);
        assert_eq!((o.get("Health"), o.get("HealF")), (Some(&Tag::Short(5)), Some(&Tag::Float(4.5))));
    }

    #[test]
    fn gear_goes_in_each_layout() {
        // Java 26.x: `equipment`, by name.
        let mut c = mob_java();
        set_equipment(&mut c, Slot::Head, Some(("minecraft:diamond_helmet", 1)), Some(4500));
        assert_eq!(c.get("equipment").and_then(|e| e.get("head")).and_then(|h| h.get("id")), Some(&s("minecraft:diamond_helmet")));
        assert_eq!(equipment(&c, Some(4500))[0], (Slot::Head, Some(("minecraft:diamond_helmet".into(), 1))));
        set_equipment(&mut c, Slot::Head, None, Some(4500));
        assert!(!c.contains_key("equipment"));
        // Java 1.20.4: the lists, every place filled, `Count` a byte.
        let mut c = mob_java();
        set_equipment(&mut c, Slot::MainHand, Some(("minecraft:iron_sword", 1)), Some(3700));
        let hands = c.get("HandItems").and_then(Tag::as_list).unwrap();
        assert_eq!(hands.len(), 2);
        assert_eq!(hands.get(0).and_then(|h| h.get("Count")), Some(&Tag::Byte(1)));
        assert_eq!(equipment(&c, Some(3700)).iter().find(|(s, _)| *s == Slot::MainHand).unwrap().1.as_ref().map(|i| i.0.as_str()), Some("minecraft:iron_sword"));
        // Console: `Equipment`, ids by number, no off hand.
        let mut o = of(&[("id", s("Zombie")), ("Health", Tag::Short(20))]);
        set_equipment(&mut o, Slot::Feet, Some(("minecraft:iron_boots", 1)), None);
        let eq = o.get("Equipment").and_then(Tag::as_list).unwrap();
        assert_eq!(eq.len(), 5);
        assert!(matches!(eq.get(1).and_then(|f| f.get("id")), Some(Tag::Short(_))));
        assert!(equipment(&o, None).iter().all(|(s, _)| *s != Slot::OffHand));
        // Bedrock: `Armor` head first, its names, empty slots kept.
        let mut b = of(&[("identifier", s("minecraft:zombie"))]);
        set_equipment(&mut b, Slot::Chest, Some(("minecraft:diamond_chestplate", 1)), None);
        let armor = b.get("Armor").and_then(Tag::as_list).unwrap();
        assert_eq!(armor.len(), 4);
        assert_eq!(armor.get(1).and_then(|c| c.get("Name")), Some(&s("minecraft:diamond_chestplate")));
        assert_eq!(armor.get(0).and_then(|c| c.get("Name")), Some(&s("")));
    }

    #[test]
    fn trades_are_added_changed_moved_and_deleted() {
        // Java: a fresh trade, then made into 3 paper for 1 emerald, with a second payment.
        let mut v = of(&[("id", s("minecraft:villager")), ("Pos", Tag::List(List::new(TagType::Double)))]);
        edit_trades(&mut v, TradeEdit::Add, Some(4500));
        edit_trades(&mut v, TradeEdit::Item { n: 0, which: 0, id: Some("minecraft:paper".into()) }, Some(4500));
        edit_trades(&mut v, TradeEdit::Count { n: 0, which: 0, count: 3 }, Some(4500));
        edit_trades(&mut v, TradeEdit::Item { n: 0, which: 1, id: Some("minecraft:stick".into()) }, Some(4500));
        edit_trades(&mut v, TradeEdit::MaxUses { n: 0, max: 7 }, Some(4500));
        let t = &trades(&v, Some(4500))[0];
        assert_eq!(t.items[0], Some(("minecraft:paper".into(), 3)));
        assert_eq!(t.items[1], Some(("minecraft:stick".into(), 1)));
        assert_eq!(t.items[2], Some(("minecraft:emerald".into(), 1)));
        assert_eq!(t.max_uses, 7);
        // A second, moved up; the first deleted.
        edit_trades(&mut v, TradeEdit::Add, Some(4500));
        edit_trades(&mut v, TradeEdit::Item { n: 1, which: 2, id: Some("minecraft:bread".into()) }, Some(4500));
        edit_trades(&mut v, TradeEdit::Move { n: 1, up: true }, Some(4500));
        assert_eq!(trades(&v, Some(4500))[0].items[2].as_ref().map(|s| s.0.as_str()), Some("minecraft:bread"));
        edit_trades(&mut v, TradeEdit::Delete(1), Some(4500));
        assert_eq!(trades(&v, Some(4500)).len(), 1);
        edit_trades(&mut v, TradeEdit::Item { n: 0, which: 1, id: None }, Some(4500));
        assert_eq!(trades(&v, Some(4500))[0].items[1], None);
        // Bedrock: `buyA`, and the counts beside the stacks kept the same.
        let mut b = of(&[("identifier", s("minecraft:villager_v2"))]);
        edit_trades(&mut b, TradeEdit::Add, None);
        edit_trades(&mut b, TradeEdit::Count { n: 0, which: 0, count: 5 }, None);
        let r = b.get("Offers").and_then(|o| o.get("Recipes")).and_then(Tag::as_list).and_then(|l| l.get(0)).cloned().unwrap();
        assert_eq!(r.get("buyA").and_then(|s| s.get("Count")), Some(&Tag::Byte(5)));
        assert_eq!(r.get("buyCountA"), Some(&Tag::Int(5)));
    }

    #[test]
    fn a_gateway_exit_keeps_its_form() {
        let mut j = of(&[("id", s("minecraft:end_gateway"))]);
        set_gateway_exit(&mut j, [10, 70, -5], Some(4500));
        assert_eq!(j.get("exit_portal"), Some(&Tag::IntArray(vec![10, 70, -5])));
        let mut old = of(&[("id", s("minecraft:end_gateway"))]);
        set_gateway_exit(&mut old, [1, 2, 3], Some(1343));
        assert_eq!(gateway_exit(&old), Some([1, 2, 3]));
        assert!(matches!(old.get("ExitPortal"), Some(Tag::Compound(_))));
        let mut b = of(&[("id", s("EndGateway"))]);
        set_gateway_exit(&mut b, [4, 5, 6], None);
        assert!(matches!(b.get("ExitPortal"), Some(Tag::List(_))));
        assert_eq!((gateway_exit(&b), gateway_exact(&b)), (Some([4, 5, 6]), None));
        set_gateway_exact(&mut j, true);
        assert_eq!(gateway_exact(&j), Some(true));
    }
    /// A trade's stacks take counts past a stack's 64 (to what the edition saves: 99 for Java's
    /// counts as ints, 127 for a byte), and are enchanted and named like any stack.
    #[test]
    fn trade_stacks_take_big_counts_enchantments_and_names() {
        use crate::item_edit::StackEdit;
        let mut v = of(&[("id", s("minecraft:villager")), ("Pos", Tag::List(List::new(TagType::Double)))]);
        edit_trades(&mut v, TradeEdit::Add, Some(4500));
        edit_trades(&mut v, TradeEdit::Count { n: 0, which: 0, count: 96 }, Some(4500));
        edit_trades(&mut v, TradeEdit::Count { n: 0, which: 2, count: 500 }, Some(4500));
        let t = &trades(&v, Some(4500))[0];
        assert_eq!((t.items[0].as_ref().map(|s| s.1), t.items[2].as_ref().map(|s| s.1), t.max_count), (Some(96), Some(99), 99));
        edit_trades(&mut v, TradeEdit::Item { n: 0, which: 2, id: Some("minecraft:diamond_sword".into()) }, Some(4500));
        edit_trades(&mut v, TradeEdit::Stack { n: 0, which: 2, edit: StackEdit::Enchant(crate::enchant::EnchantEdit::Add("minecraft:sharpness".into())) }, Some(4500));
        edit_trades(&mut v, TradeEdit::Stack { n: 0, which: 2, edit: StackEdit::Enchant(crate::enchant::EnchantEdit::Level(0, 5)) }, Some(4500));
        edit_trades(&mut v, TradeEdit::Stack { n: 0, which: 2, edit: StackEdit::Name("Bob's Blade".into()) }, Some(4500));
        let sold = trades(&v, Some(4500))[0].stacks[2].clone().unwrap();
        assert_eq!(crate::enchant::read(&sold), vec![("minecraft:sharpness".to_string(), 5)]);
        assert_eq!(crate::naming::item_name(&sold).as_deref(), Some("Bob's Blade"));
        // Bedrock: a byte, so up to 127.
        let mut b = of(&[("identifier", s("minecraft:villager_v2"))]);
        edit_trades(&mut b, TradeEdit::Add, None);
        edit_trades(&mut b, TradeEdit::Count { n: 0, which: 0, count: 120 }, None);
        let t = &trades(&b, None)[0];
        assert_eq!((t.items[0].as_ref().map(|s| s.1), t.max_count), (Some(120), 127));
    }
}
