//! Our own Java to Bedrock block mapping, built from the two games.
//!
//! Both editions grew from the 1.12 numeric blocks, and each game can say what it makes of every
//! old (id, data) block: the Java server's upgrade (`mctk-convert`'s `legacy_blocks`, in
//! `mctk-registry/data/legacy/blocks.tsv`) and the Bedrock server's (`legacy-gen`/`legacy-read`,
//! `mctk-registry/data/bedrock/legacy_blocks.tsv`). Joining them on (id, data) pairs a Java state
//! with a Bedrock state for every block of that era, with no third-party table. (Bedrock numbers
//! a few blocks differently from Java; those are lined up by name first.)
//!
//! Newer blocks have no old id. For them the pairs show how the editions spell things: which
//! Java property value goes with which Bedrock one for blocks of the same shape (all stairs, all
//! doors, all logs), and that is applied to the newer blocks of that shape. What the pairs cannot
//! show (what Java works out from neighbours and Bedrock stores, such as stair corners and wall
//! connections) is a few rules at the end of this file.

use std::collections::{BTreeMap, HashMap};

use mctk_registry::{Registry, builtin};

/// A state's properties by name.
pub type Props = BTreeMap<String, String>;
/// A Java state: block name and properties.
pub type JState = (String, Props);
/// A Bedrock state: block name and typed properties (`b:1`, `i:3`, `s:top`).
pub type BState = (String, Props);

fn parse_java(text: &str) -> JState {
    let (name, props) = match text.split_once('[') {
        Some((n, p)) => (n, p.trim_end_matches(']')),
        None => (text, ""),
    };
    let props = props
        .split(',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect();
    (name.to_string(), props)
}

fn complete_java(reg: &Registry, mut s: JState) -> JState {
    if let Some(def) = reg.blocks.get(&s.0) {
        for (k, v) in &def.defaults {
            s.1.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    s
}

fn parse_bedrock(name: &str, states: &str) -> BState {
    let props = states
        .split(',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect();
    (name.to_string(), props)
}

/// A name with what only one edition adds taken off, to tell whether two blocks are the same one.
fn norm(name: &str) -> String {
    let mut s = name.trim_start_matches("minecraft:").to_string();
    for prefix in ["lit_", "unlit_", "powered_", "unpowered_", "flowing_", "double_"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
        }
    }
    for (from, to) in [("_double_slab", "_slab"), ("_inverted", ""), ("_wall_torch", "_torch")] {
        if let Some(rest) = s.strip_suffix(from) {
            s = format!("{rest}{to}");
        }
    }
    s.replace('_', "")
}

/// Java legacy states by (id, data) and Bedrock ones, as the two games made them.
struct Legacy {
    java: HashMap<(u8, u8), JState>,
    bedrock: HashMap<(u8, u8), BState>,
}

fn load_legacy(java: &Registry, bedrock: &Registry) -> Legacy {
    let mut l = Legacy { java: HashMap::new(), bedrock: HashMap::new() };
    for line in builtin::legacy_blocks_text().lines().filter(|l| !l.starts_with('#')) {
        let mut f = line.split('\t');
        if let (Some(Ok(id)), Some(Ok(data)), Some(state)) = (f.next().map(str::parse::<u8>), f.next().map(str::parse::<u8>), f.next()) {
            l.java.insert((id, data), complete_java(java, parse_java(state)));
        }
    }
    for line in builtin::bedrock_legacy_blocks_text().lines().filter(|l| !l.starts_with('#')) {
        let mut f = line.split('\t');
        let (Some(Ok(id)), Some(Ok(data)), Some(name)) = (f.next().map(str::parse::<u8>), f.next().map(str::parse::<u8>), f.next()) else {
            continue;
        };
        if bedrock.blocks.contains_key(name) {
            l.bedrock.insert((id, data), parse_bedrock(name, f.next().unwrap_or("")));
        }
    }
    l
}

/// For each Java legacy id, the Bedrock id that holds the same block. Bedrock numbers most blocks
/// like Java, but swaps or moves some (dropper and activator rail, the wooden slabs, observer,
/// glazed terracotta); those are found by the names the two games give the ids.
fn align(l: &Legacy) -> HashMap<u8, u8> {
    let norms = |m: &HashMap<(u8, u8), JState>| -> HashMap<(u8, u8), String> { m.iter().map(|(k, v)| (*k, norm(&v.0))).collect() };
    let jn = norms(&l.java);
    let bn: HashMap<(u8, u8), String> = l.bedrock.iter().map(|(k, v)| (*k, norm(&v.0))).collect();
    // (names alike after taking off what one edition adds, names exactly alike), as fractions of
    // the Java id's data values that hold a block.
    let score = |j: u8, b: u8| -> (f32, f32) {
        let any = (0..16u8).filter(|d| jn.get(&(j, *d)).is_some_and(|n| n != "air")).count().max(1) as f32;
        let (mut alike, mut exact) = (0.0, 0.0);
        for d in 0..16u8 {
            if let (Some(x), Some(y)) = (jn.get(&(j, d)), bn.get(&(b, d))) {
                if x == y && x != "air" {
                    alike += 1.0;
                    if l.java[&(j, d)].0 == l.bedrock[&(b, d)].0 {
                        exact += 1.0;
                    }
                }
            }
        }
        (alike / any, exact / any)
    };
    let mut out = HashMap::new();
    for j in 0..=255u8 {
        let base = score(j, j);
        let mut pick = j;
        // Another id only when the same id is clearly not the block and another clearly is.
        if base.0 < 0.2 {
            let mut best = (0.5f32, 0.0f32);
            for b in 0..=255u8 {
                let s = score(j, b);
                if s.0 >= best.0 && (s.0, s.1) > best {
                    best = s;
                    pick = b;
                }
            }
        }
        out.insert(j, pick);
    }
    out
}

/// The legacy join: for every (id, data) the Java state and the Bedrock state the games made.
fn legacy_pairs(l: &Legacy) -> Vec<(JState, BState, bool)> {
    let aligned = align(l);
    let mut out = Vec::new();
    for (&(id, data), j) in &l.java {
        if j.0 == "minecraft:air" {
            continue;
        }
        let Some(b) = l.bedrock.get(&(aligned[&id], data)) else { continue };
        // The same block under another name or a different id is fine; a clear mismatch is not.
        out.push((j.clone(), b.clone(), aligned[&id] == id));
    }
    out
}

/// Which Bedrock values come with a Java property value, as counts over pairs.
#[derive(Default, Clone)]
struct Assoc {
    total: HashMap<(String, String), usize>,
    seen: HashMap<(String, String), HashMap<(String, String), usize>>,
}

impl Assoc {
    fn add(&mut self, j: &JState, b: &BState) {
        for (k, v) in &j.1 {
            let key = (k.clone(), v.clone());
            *self.total.entry(key.clone()).or_default() += 1;
            let slot = self.seen.entry(key).or_default();
            *slot.entry(("@name".into(), b.0.clone())).or_default() += 1;
            for (q, w) in &b.1 {
                *slot.entry((q.clone(), w.clone())).or_default() += 1;
            }
        }
    }

    /// The Bedrock values a Java value always comes with and that change with the Java property.
    fn assignments(&self, k: &str, v: &str) -> Vec<(String, String)> {
        let key = (k.to_string(), v.to_string());
        let (Some(total), Some(seen)) = (self.total.get(&key), self.seen.get(&key)) else { return Vec::new() };
        let mut out = Vec::new();
        for ((q, w), n) in seen {
            if n != total {
                continue;
            }
            // Only a Bedrock property that takes another value for another value of this one.
            let varies = self
                .seen
                .iter()
                .any(|((k2, v2), s2)| k2 == k && v2 != v && s2.keys().any(|(q2, w2)| q2 == q && w2 != w) && !s2.contains_key(&(q.clone(), w.clone())));
            if varies {
                out.push((q.clone(), w.clone()));
            }
        }
        out.sort();
        out
    }
}

/// Java values to a Bedrock property's values across all blocks: `(java prop, value, bedrock prop)`
/// to how often each Bedrock value came with it.
#[derive(Default)]
struct Global(HashMap<(String, String, String), HashMap<String, usize>>);

impl Global {
    fn add(&mut self, j: &JState, b: &BState) {
        for (k, v) in &j.1 {
            for (q, w) in &b.1 {
                *self.0.entry((k.clone(), v.clone(), q.clone())).or_default().entry(w.clone()).or_default() += 1;
            }
        }
    }
}

/// A block's shape: its properties with their values, which blocks of one family share.
fn signature(def: &mctk_registry::BlockDef) -> String {
    def.properties.iter().map(|(k, vs)| format!("{k}={}", vs.join("|"))).collect::<Vec<_>>().join(";")
}

fn bedrock_shape(def: &mctk_registry::BlockDef) -> String {
    def.properties.keys().cloned().collect::<Vec<_>>().join(",")
}

/// The Bedrock block a Java block without legacy pairs is, when its name differs.
fn renamed(name: &str, bedrock: &Registry) -> Option<String> {
    let bare = name.trim_start_matches("minecraft:");
    let has = |n: &str| bedrock.blocks.contains_key(n);
    let candidates: Vec<String> = {
        let mut c = Vec::new();
        if let Some(stem) = bare.strip_suffix("_wall_torch") {
            c.push(format!("minecraft:{stem}_torch"));
        }
        if let Some(stem) = bare.strip_suffix("_wall_sign") {
            c.push(format!("minecraft:{stem}_wall_sign"));
        }
        if bare == "oak_sign" {
            c.push("minecraft:standing_sign".into());
        } else if let Some(stem) = bare.strip_suffix("_sign").filter(|s| !s.ends_with("wall")) {
            c.push(format!("minecraft:{stem}_standing_sign"));
        }
        if bare == "oak_wall_sign" {
            c.push("minecraft:wall_sign".into());
        }
        if let Some(stem) = bare.strip_suffix("_wall_hanging_sign") {
            c.push(format!("minecraft:{stem}_hanging_sign"));
        }
        if bare.ends_with("_banner") {
            c.push(if bare.ends_with("_wall_banner") { "minecraft:wall_banner".into() } else { "minecraft:standing_banner".into() });
        }
        if bare.ends_with("_bed") {
            c.push("minecraft:bed".into());
        }
        if let Some(stem) = bare.strip_suffix("_wall_head") {
            c.push(format!("minecraft:{stem}_head"));
        }
        if let Some(stem) = bare.strip_suffix("_wall_skull") {
            c.push(format!("minecraft:{stem}_skull"));
        }
        if let Some(stem) = bare.strip_suffix("_sign").or_else(|| bare.strip_suffix("_wall_sign")).filter(|s| s.starts_with("dark_oak") && !s.ends_with("hanging")) {
            let wall = bare.ends_with("_wall_sign");
            c.push(format!("minecraft:darkoak_{}_sign", if wall { "wall" } else { "standing" }));
            let _ = stem;
        }
        if bare.starts_with("potted_") {
            c.push("minecraft:flower_pot".into());
        }
        for (from, to) in [
            ("cave_air", "air"),
            ("void_air", "air"),
            ("moving_piston", "moving_block"),
            ("rooted_dirt", "dirt_with_roots"),
            ("small_dripleaf", "small_dripleaf_block"),
            ("stonecutter", "stonecutter_block"),
            ("waxed_copper_block", "waxed_copper"),
            ("flowering_azalea_leaves", "azalea_leaves_flowered"),
            ("frogspawn", "frog_spawn"),
            ("stone_slab", "normal_stone_slab"),
            ("end_stone_brick_stairs", "end_brick_stairs"),
            ("prismarine_brick_stairs", "prismarine_bricks_stairs"),
            ("kelp_plant", "kelp"),
            ("cave_vines_plant", "cave_vines"),
            ("twisting_vines_plant", "twisting_vines"),
            ("weeping_vines_plant", "weeping_vines"),
            ("attached_melon_stem", "melon_stem"),
            ("attached_pumpkin_stem", "pumpkin_stem"),
            ("tall_seagrass", "seagrass"),
            ("big_dripleaf_stem", "big_dripleaf"),
            ("lava_cauldron", "cauldron"),
            ("powder_snow_cauldron", "cauldron"),
            ("light", "light_block_15"),
            ("shulker_box", "undyed_shulker_box"),
            ("light_gray_glazed_terracotta", "silver_glazed_terracotta"),
            ("snow_block", "snow"),
            ("wall_torch", "torch"),
            ("water_cauldron", "cauldron"),
            ("beetroots", "beetroot"),
            ("cobblestone_stairs", "stone_stairs"),
            ("stone_stairs", "normal_stone_stairs"),
            ("dirt_path", "grass_path"),
            ("piston_head", "piston_arm_collision"),
        ] {
            if bare == from {
                c.push(format!("minecraft:{to}"));
            }
        }
        c
    };
    candidates.into_iter().find(|n| has(n))
}

/// Blocks whose Bedrock name depends on the state: double slabs, powered repeaters and
/// comparators, lights, cave vines with and without berries.
fn special_name(j: &JState, bedrock: &Registry) -> Option<String> {
    let has = |n: &str| bedrock.blocks.contains_key(n);
    let bare = j.0.trim_start_matches("minecraft:");
    if java(j, "type") == Some("double") && bare.ends_with("_slab") {
        // The plain stone slab is Bedrock's "normal" one (its single slab is renamed too).
        if bare == "stone_slab" && has("minecraft:normal_stone_double_slab") {
            return Some("minecraft:normal_stone_double_slab".into());
        }
        let candidates = [
            format!("minecraft:{}_double_slab", bare.trim_end_matches("_slab")),
            format!("minecraft:{}", bare.replace("_cut_copper_slab", "_double_cut_copper_slab")).replace("minecraft:cut_copper_slab", "minecraft:double_cut_copper_slab"),
        ];
        // A double slab the game has no block for keeps the name it would have had, so it is
        // reported as missing instead of being squeezed into the single slab.
        let first = candidates[0].clone();
        let own = format!("minecraft:{bare}");
        return Some(candidates.into_iter().find(|n| *n != own && has(n)).unwrap_or(first));
    }
    let lit = java(j, "lit") == Some("true");
    let fixed = match bare {
        "dirt_path" => Some("minecraft:grass_path"),
        "powered_rail" => Some("minecraft:golden_rail"),
        "water" if java(j, "level") == Some("0") => Some("minecraft:water"),
        "lava" if java(j, "level") == Some("0") => Some("minecraft:lava"),
        "snow" => Some("minecraft:snow_layer"),
        "moving_piston" => Some("minecraft:moving_block"),
        "redstone_lamp" if lit => Some("minecraft:lit_redstone_lamp"),
        "redstone_ore" if lit => Some("minecraft:lit_redstone_ore"),
        "deepslate_redstone_ore" if lit => Some("minecraft:lit_deepslate_redstone_ore"),
        "redstone_torch" | "redstone_wall_torch" => Some(if lit { "minecraft:redstone_torch" } else { "minecraft:unlit_redstone_torch" }),
        "piston_head" => Some(if java(j, "type") == Some("sticky") { "minecraft:sticky_piston_arm_collision" } else { "minecraft:piston_arm_collision" }),
        "stone_slab" if java(j, "type") == Some("double") => Some("minecraft:normal_stone_double_slab"),
        _ => None,
    };
    if let Some(n) = fixed.filter(|n| has(n)) {
        return Some(n.to_string());
    }
    match bare {
        "comparator" | "repeater" => {
            let powered = java(j, "powered") == Some("true");
            let n = format!("minecraft:{}_{bare}", if powered { "powered" } else { "unpowered" });
            has(&n).then_some(n)
        }
        "light" => {
            let n = format!("minecraft:light_block_{}", java(j, "level").unwrap_or("15"));
            has(&n).then_some(n)
        }
        "cave_vines" | "cave_vines_plant" => {
            let berries = java(j, "berries") == Some("true");
            let n = match (bare, berries) {
                ("cave_vines", true) => "minecraft:cave_vines_head_with_berries",
                ("cave_vines_plant", true) => "minecraft:cave_vines_body_with_berries",
                _ => "minecraft:cave_vines",
            };
            has(n).then(|| n.to_string())
        }
        _ => None,
    }
}

/// Bedrock name of a state of a block that has no legacy pairs.
fn name_for(j: &JState, bedrock: &Registry) -> Option<String> {
    if let Some(n) = special_name(j, bedrock) {
        return Some(n);
    }
    if let Some(n) = renamed(&j.0, bedrock) {
        return Some(n);
    }
    if bedrock.blocks.contains_key(&j.0) {
        // A lit block is a block of its own in Bedrock when the game has one.
        let lit = j.1.get("lit").map(String::as_str) == Some("true");
        let bare = j.0.trim_start_matches("minecraft:");
        if lit && bedrock.blocks.contains_key(&format!("minecraft:lit_{bare}")) {
            return Some(format!("minecraft:lit_{bare}"));
        }
        return Some(j.0.clone());
    }
    None
}

struct Learned {
    direct: HashMap<JState, BState>,
    pairs_name: HashMap<String, String>,
    block: HashMap<String, Assoc>,
    class: HashMap<String, Assoc>,
    global: Global,
}

fn learn(java: &Registry, bedrock: &Registry, pairs: &[(JState, BState)]) -> Learned {
    let mut counts: HashMap<JState, HashMap<BState, usize>> = HashMap::new();
    let mut by_block: HashMap<String, HashMap<String, usize>> = HashMap::new();
    let mut l = Learned { direct: HashMap::new(), pairs_name: HashMap::new(), block: HashMap::new(), class: HashMap::new(), global: Global::default() };
    for (j, b) in pairs {
        *counts.entry(j.clone()).or_default().entry(b.clone()).or_default() += 1;
        *by_block.entry(j.0.clone()).or_default().entry(b.0.clone()).or_default() += 1;
        l.block.entry(j.0.clone()).or_default().add(j, b);
        l.global.add(j, b);
    }
    for (j, bs) in counts {
        if let Some((b, _)) = bs.into_iter().max_by_key(|(_, n)| *n) {
            l.direct.insert(j, b);
        }
    }
    for (jname, bs) in &by_block {
        if let Some((b, _)) = bs.iter().max_by_key(|(_, n)| **n) {
            l.pairs_name.insert(jname.clone(), b.clone());
        }
    }
    // Blocks of one shape (same Java properties, same Bedrock property names) share what they teach.
    for (j, b) in pairs {
        let (Some(jdef), Some(bdef)) = (java.blocks.get(&j.0), bedrock.blocks.get(&b.0)) else { continue };
        l.class.entry(format!("{}||{}", signature(jdef), bedrock_shape(bdef))).or_default().add(j, b);
    }
    l
}

/// Everything a state of a Java block turns into, before the checks.
fn derive(j: &JState, java: &Registry, bedrock: &Registry, l: &Learned) -> BState {
    let jdef = &java.blocks[&j.0];
    let has_pairs = l.pairs_name.contains_key(&j.0);
    // The block.
    let special = special_name(j, bedrock);
    let mut name = if let Some(n) = special.clone() {
        n
    } else if has_pairs {
        l.pairs_name[&j.0].clone()
    } else {
        name_for(j, bedrock).unwrap_or_else(|| j.0.clone())
    };
    if has_pairs && special.is_none() {
        if let Some(a) = l.block.get(&j.0) {
            for (k, v) in &j.1 {
                if let Some((_, n)) = a.assignments(k, v).into_iter().find(|(q, _)| q == "@name") {
                    name = n;
                }
            }
        }
    }
    let Some(bdef) = bedrock.blocks.get(&name) else { return (name, Props::new()) };
    let mut states: Props = bdef.defaults.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let shape = format!("{}||{}", signature(jdef), bedrock_shape(bdef));
    let assoc: Option<&Assoc> = if has_pairs { l.block.get(&j.0) } else { l.class.get(&shape) };
    // What the pairs teach, except where the two games read the old data bits differently: the
    // directions and the door and bed halves, which the conventions below decide.
    let mut learned_q: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (k, v) in &j.1 {
        if let Some(a) = assoc {
            for (q, w) in a.assignments(k, v) {
                if q != "@name" && !conventional(&q) && bdef.properties.get(&q).is_some_and(|vs| vs.contains(&w)) {
                    states.insert(q.clone(), w);
                    learned_q.insert(q);
                }
            }
        }
    }
    for q in bdef.properties.keys() {
        if learned_q.contains(q) {
            continue;
        }
        if let Some(w) = convention(q, &name, j) {
            if bdef.properties[q].contains(&w) {
                states.insert(q.clone(), w);
            }
        }
    }
    (name, states)
}

/// Bedrock properties whose values come from `convention`, not from the legacy pairs: the
/// direction tables and the bits the two games keep in different halves of the old data (a door's
/// hinge and powered bits are in its upper half in Java, a bed's head piece in a data bit of its
/// own).
fn conventional(q: &str) -> bool {
    matches!(
        q,
        "minecraft:cardinal_direction"
            | "weirdo_direction"
            | "direction"
            | "facing_direction"
            | "minecraft:facing_direction"
            | "ground_sign_direction"
            | "torch_facing_direction"
            | "upside_down_bit"
            | "minecraft:vertical_half"
            | "upper_block_bit"
            | "door_hinge_bit"
            | "open_bit"
            | "powered_bit"
            | "lit"
            | "attached_bit"
            | "in_wall_bit"
            | "hanging"
            | "candles"
            | "redstone_signal"
            | "redstone_north"
            | "redstone_east"
            | "redstone_south"
            | "redstone_west"
            | "minecraft:corner"
            | "powered_shelf_type"
            | "head_piece_bit"
            | "occupied_bit"
            | "update_bit"
            | "persistent_bit"
            | "button_pressed_bit"
            | "coral_direction"
            | "rail_data_bit"
            | "toggle_bit"
            | "conditional_bit"
            | "triggered_bit"
            | "crafting"
            | "extinguished"
            | "disarmed_bit"
            | "ominous"
            | "attachment"
            | "sculk_sensor_phase"
            | "growth"
            | "age_bit"
            | "huge_mushroom_bits"
            | "pillar_axis"
            | "natural"
            | "big_dripleaf_head"
            | "big_dripleaf_tilt"
            | "dripstone_thickness"
    ) || q.starts_with("minecraft:connection_")
        || q.starts_with("wall_connection_type_")
        || q.starts_with("pale_moss_carpet_side_")
        || q == "wall_post_bit"
}

fn bit(b: bool) -> String {
    format!("b:{}", u8::from(b))
}

fn java<'a>(j: &'a JState, k: &str) -> Option<&'a str> {
    j.1.get(k).map(String::as_str)
}

fn is_true(j: &JState, k: &str) -> Option<bool> {
    java(j, k).map(|v| v == "true")
}

/// The Bedrock value of property `q` for a Java state, by how the two editions spell it. The
/// direction tables are Bedrock's own numbering of its blocks: stairs count east, west, south,
/// north; trapdoors the same; beds, beehives and the like south, west, north, east; the six-way
/// `facing_direction` down, up, north, south, west, east.
fn convention(q: &str, bname: &str, j: &JState) -> Option<String> {
    let facing = java(j, "facing");
    let trapdoor = bname.ends_with("trapdoor");
    let hanging_sign = bname.ends_with("hanging_sign");
    let ceiling_hanging = hanging_sign && !j.0.contains("wall");
    let rotation_facing = |r: i32| -> i32 {
        // Bedrock's facing of a ceiling sign turned `r` sixteenths from south: south 3, west 4, north 2, east 5.
        match ((r + 2).rem_euclid(16)) / 4 {
            0 => 3,
            1 => 4,
            2 => 2,
            _ => 5,
        }
    };
    match q {
        "minecraft:cardinal_direction" => {
            let f = facing.filter(|f| matches!(*f, "north" | "south" | "east" | "west"))?;
            // A door's own turn: Java's east is Bedrock's south, and so on round.
            let f = if bname.ends_with("_door") {
                match f {
                    "east" => "south",
                    "north" => "east",
                    "south" => "west",
                    _ => "north",
                }
            } else {
                f
            };
            Some(format!("s:{f}"))
        }
        "weirdo_direction" => facing.and_then(|f| ["east", "west", "south", "north"].iter().position(|d| *d == f)).map(|n| format!("i:{n}")),
        "direction" if bname.ends_with("bell") => {
            facing.and_then(|f| ["north", "east", "south", "west"].iter().position(|d| *d == f)).map(|n| format!("i:{n}"))
        }
        "direction" => {
            let order: [&str; 4] = if trapdoor { ["east", "west", "south", "north"] } else { ["south", "west", "north", "east"] };
            facing.and_then(|f| order.iter().position(|d| *d == f)).map(|n| format!("i:{n}"))
        }
        "facing_direction" | "minecraft:facing_direction" => {
            if ceiling_hanging {
                return java(j, "rotation").and_then(|r| r.parse::<i32>().ok()).map(|r| format!("i:{}", rotation_facing(r)));
            }
            if (bname.ends_with("_head") || bname.ends_with("_skull"))
                && java(j, "rotation").is_some() {
                    return Some("i:1".into());
                }
            if bname.ends_with("end_rod") {
                return facing.and_then(|f| ["down", "up", "south", "north", "east", "west"].iter().position(|d| *d == f)).map(|n| format!("i:{n}"));
            }
            if java(j, "face").is_some() && facing.is_some() {
                let f = match java(j, "face")? {
                    "ceiling" => "down",
                    "floor" => "up",
                    _ => facing?,
                };
                return ["down", "up", "north", "south", "west", "east"].iter().position(|d| *d == f).map(|n| format!("i:{n}"));
            }
            facing.and_then(|f| ["down", "up", "north", "south", "west", "east"].iter().position(|d| *d == f)).map(|n| format!("i:{n}"))
        }
        "ground_sign_direction" if hanging_sign && facing.is_some() => {
            facing.and_then(|f| ["south", "west", "north", "east"].iter().position(|d| *d == f)).map(|n| format!("i:{}", n * 4))
        }
        "ground_sign_direction" => java(j, "rotation").map(|r| format!("i:{r}")),
        "hanging" if hanging_sign => Some(bit(ceiling_hanging)),
        "hanging" => java(j, "vertical_direction").or(java(j, "hanging")).map(|v| bit(v == "down" || v == "true")),
        "attached_bit" if hanging_sign && !ceiling_hanging => Some(bit(true)),
        "upside_down_bit" => java(j, "half").map(|h| bit(h == "top")),
        "minecraft:vertical_half" => java(j, "type").or(java(j, "half")).filter(|v| matches!(*v, "top" | "bottom")).map(|v| format!("s:{v}")),
        "upper_block_bit" if bname.contains("pale_moss_carpet") => is_true(j, "bottom").map(|b| bit(!b)),
        "upper_block_bit" => java(j, "half").map(|h| bit(h == "upper")),
        "door_hinge_bit" => java(j, "hinge").map(|h| bit(h == "right")),
        "open_bit" if bname.ends_with("lever") => is_true(j, "powered").map(bit),
        "open_bit" => is_true(j, "open").map(bit),
        "powered_bit" | "toggle_bit" | "rail_data_bit" | "button_pressed_bit" => is_true(j, "powered").map(bit),
        "lit" => is_true(j, "lit").map(bit),
        "extinguished" => is_true(j, "lit").map(|l| bit(!l)),
        "attached_bit" => is_true(j, "attached").map(bit),
        "in_wall_bit" => is_true(j, "in_wall").map(bit),
        "persistent_bit" => is_true(j, "persistent").map(bit),
        "disarmed_bit" => is_true(j, "disarmed").map(bit),
        "triggered_bit" => is_true(j, "triggered").map(bit),
        "crafting" => is_true(j, "crafting").map(bit),
        "ominous" => is_true(j, "ominous").map(bit),
        "conditional_bit" => is_true(j, "conditional").map(bit),
        "head_piece_bit" => java(j, "part").map(|p| bit(p == "head")),
        "occupied_bit" => is_true(j, "occupied").map(bit),
        "pillar_axis" => java(j, "axis").map(|a| format!("s:{a}")),
        "powered_shelf_type" => java(j, "side_chain")
            .and_then(|s| ["unconnected", "right", "center", "left"].iter().position(|d| *d == s))
            .map(|n| format!("i:{n}")),
        "sculk_sensor_phase" => java(j, "sculk_sensor_phase")
            .and_then(|s| ["inactive", "active", "cooldown"].iter().position(|d| *d == s))
            .map(|n| format!("i:{n}")),
        "coral_direction" => facing.and_then(|f| ["west", "east", "north", "south"].iter().position(|d| *d == f)).map(|n| format!("i:{n}")),
        "attachment" => java(j, "attachment").map(|a| {
            format!(
                "s:{}",
                match a {
                    "floor" => "standing",
                    "ceiling" => "hanging",
                    "single_wall" => "side",
                    _ => "multiple",
                }
            )
        }),
        "growth" if java(j, "segment_amount").is_some() => java(j, "segment_amount").and_then(|a| a.parse::<i32>().ok()).map(|a| format!("i:{}", a - 1)),
        "growth" => java(j, "age").and_then(|a| a.parse::<i32>().ok()).map(|a| {
            // Beetroots and nether wart stop at 3, Bedrock's growth runs to 7.
            let max = if bname.contains("beetroot") { 3 } else { 7 };
            format!("i:{}", (a * 7 + max / 2) / max)
        }),
        "age_bit" => java(j, "age").or(java(j, "stage")).map(|a| bit(a != "0")),
        "natural" => is_true(j, "natural").map(bit),
        "big_dripleaf_head" => Some(bit(!bname.contains("stem"))),
        "big_dripleaf_tilt" => java(j, "tilt").map(|t| format!("s:{}", match t { "partial" => "partial_tilt", "full" => "full_tilt", o => o })),
        "dripstone_thickness" => java(j, "thickness").map(|t| format!("s:{}", if t == "tip_merge" { "merge" } else { t })),
        "huge_mushroom_bits" => {
            let on = |k: &str| java(j, k) == Some("true");
            let (u, d, n, s, e, w) = (on("up"), on("down"), on("north"), on("south"), on("east"), on("west"));
            let n = if bname.contains("stem") {
                15
            } else if u && d && n && s && e && w {
                14
            } else if !u && !d && n && s && e && w {
                10
            } else if u {
                match (n, s, e, w) {
                    (true, false, false, true) => 1,
                    (true, false, false, false) => 2,
                    (true, false, true, false) => 3,
                    (false, false, false, true) => 4,
                    (false, false, true, false) => 6,
                    (false, true, false, true) => 7,
                    (false, true, false, false) => 8,
                    (false, true, true, false) => 9,
                    _ => 5,
                }
            } else {
                0
            };
            Some(format!("i:{n}"))
        }
        "candles" => java(j, "candles").and_then(|c| c.parse::<i32>().ok()).map(|c| format!("i:{}", c - 1)),
        "redstone_signal" => java(j, "power").or(java(j, "powered").map(|p| if p == "true" { "15" } else { "0" })).map(|p| format!("i:{p}")),
        "redstone_north" | "redstone_east" | "redstone_south" | "redstone_west" => {
            java(j, q.trim_start_matches("redstone_")).map(|v| format!("s:{v}"))
        }
        "minecraft:corner" => java(j, "shape").map(|s| format!("s:{}", if s == "straight" { "none" } else { s })),
        "wall_post_bit" => is_true(j, "up").map(bit),
        _ if q.starts_with("pale_moss_carpet_side_") => {
            java(j, q.trim_start_matches("pale_moss_carpet_side_")).map(|v| format!("s:{}", if v == "low" { "short" } else { v }))
        }
        _ if q.starts_with("minecraft:connection_") => is_true(j, q.trim_start_matches("minecraft:connection_")).map(bit),
        _ if q.starts_with("wall_connection_type_") => {
            java(j, q.trim_start_matches("wall_connection_type_")).map(|v| format!("s:{}", if v == "low" { "short" } else { v }))
        }
        _ => None,
    }
}/// Every Java state of every block, in registry order (last property varying fastest).
fn java_states(java: &Registry) -> Vec<JState> {
    let mut out = Vec::new();
    for (name, def) in &java.blocks {
        let props: Vec<(&String, &Vec<String>)> = def.properties.iter().collect();
        let mut idx = vec![0usize; props.len()];
        loop {
            out.push((name.clone(), props.iter().zip(&idx).map(|((k, vs), i)| ((*k).clone(), vs[*i].clone())).collect()));
            let mut p = props.len();
            loop {
                if p == 0 {
                    break;
                }
                p -= 1;
                idx[p] += 1;
                if idx[p] < props[p].1.len() {
                    break;
                }
                idx[p] = 0;
                if p == 0 {
                    p = usize::MAX;
                    break;
                }
            }
            if props.is_empty() || p == usize::MAX {
                break;
            }
        }
    }
    out
}

/// Makes the table. Returns the Java states with their Bedrock state.
pub fn table() -> Result<Vec<(JState, BState)>, String> {
    let java = builtin::java("26.3").ok_or("no Java registry")?;
    let bedrock = builtin::bedrock("1.26.61.1").ok_or("no Bedrock registry")?;
    let legacy = load_legacy(&java, &bedrock);
    let java_names: std::collections::HashSet<&String> = java.blocks.keys().collect();
    // A pair is the same block when the names agree (after taking off what one edition adds) or,
    // when they differ, when neither edition's own name for it explains the other (a Java block
    // with no Bedrock block of that name, paired with a Bedrock block no Java block is named for).
    let pairs: Vec<(JState, BState)> = legacy_pairs(&legacy)
        .into_iter()
        .filter(|(j, b, _same_id)| {
            if b.0 == "minecraft:air" {
                return j.0.ends_with("air");
            }
            norm(&j.0) == norm(&b.0) || (!bedrock.blocks.contains_key(&j.0) && !java_names.contains(&b.0))
        })
        .map(|(j, b, _)| (j, b))
        .collect();
    let learned = learn(&java, &bedrock, &pairs);
    Ok(java_states(&java)
        .into_iter()
        .map(|j| {
            let b = derive(&j, &java, &bedrock, &learned);
            (j, b)
        })
        .collect())
}

/// `blockmap-report`: how well the table fits the Bedrock registry.
pub fn report() -> Result<(), String> {
    let bedrock = builtin::bedrock("1.26.61.1").ok_or("no Bedrock registry")?;
    let rows = table()?;
    let (mut exact, mut bad_state, mut unknown) = (0usize, BTreeMap::<String, usize>::new(), BTreeMap::<String, usize>::new());
    for (j, b) in &rows {
        match bedrock.check_block(&b.0, b.1.iter().map(|(k, v)| (k.as_str(), v.as_str()))) {
            mctk_registry::BlockCheck::Exact => exact += 1,
            mctk_registry::BlockCheck::BadState(p) => *bad_state.entry(format!("{} -> {} {:?}", j.0, b.0, p.first())).or_default() += 1,
            mctk_registry::BlockCheck::UnknownBlock => *unknown.entry(j.0.clone()).or_default() += 1,
        }
    }
    println!("{} Java states: {exact} map to exact Bedrock states, {} have a bad state, {} have no Bedrock block", rows.len(), bad_state.values().sum::<usize>(), unknown.values().sum::<usize>());
    for (k, n) in bad_state.iter().take(40) {
        println!("  bad {n:>5}  {k}");
    }
    for (k, n) in unknown.iter().take(60) {
        println!("  none {n:>5}  {k}");
    }
    Ok(())
}
fn typed(v: &str) -> mctk_nbt::Tag {
    match v.split_once(':') {
        Some(("b", n)) => mctk_nbt::Tag::Byte(n.parse().unwrap_or(0)),
        Some(("i", n)) => mctk_nbt::Tag::Int(n.parse().unwrap_or(0)),
        Some((_, s)) => mctk_nbt::Tag::String(s.to_string()),
        None => mctk_nbt::Tag::String(v.to_string()),
    }
}

/// `blockmap-compare [block-prefix]`: checks the table against the previous (third-party derived)
/// one, which is a way to find mistakes, not a source. Differences are listed by Bedrock property.
pub fn compare(prefix: Option<&str>) -> Result<(), String> {
    let rows = table()?;
    let old = mctk_mapping::mappings();
    let (mut same, mut different, mut names) = (0usize, 0usize, BTreeMap::<String, usize>::new());
    let mut by_prop: BTreeMap<(String, String, String, String), (usize, String)> = BTreeMap::new();
    for (j, b) in &rows {
        let mut state = mctk_java::BlockState::new(j.0.clone());
        for (k, v) in &j.1 {
            state.properties.insert(k.clone(), v.clone());
        }
        let Some(o) = old.java_to_bedrock(&state) else { continue };
        let mine: mctk_nbt::Compound = b.1.iter().map(|(k, v)| (k.clone(), typed(v))).collect();
        if o.name == b.0 && o.states == mine {
            same += 1;
            continue;
        }
        different += 1;
        if o.name != b.0 {
            *names.entry(format!("{} -> mine {} / old {}", j.0, b.0, o.name)).or_default() += 1;
            continue;
        }
        if prefix.is_some_and(|p| !j.0.contains(p)) {
            continue;
        }
        let text = |t: Option<&mctk_nbt::Tag>| t.map_or("-".to_string(), |t| match t {
            mctk_nbt::Tag::Byte(n) => format!("b:{n}"),
            mctk_nbt::Tag::Int(n) => format!("i:{n}"),
            mctk_nbt::Tag::String(s) => format!("s:{s}"),
            other => other.to_string(),
        });
        let keys: std::collections::BTreeSet<&String> = mine.keys().chain(o.states.keys()).collect();
        for q in keys {
            let (m, ov) = (text(mine.get(q)), text(o.states.get(q)));
            if m != ov {
                let family = b.0.trim_start_matches("minecraft:").to_string();
                let e = by_prop.entry((q.clone(), m.clone(), ov.clone(), family)).or_insert((0, String::new()));
                e.0 += 1;
                if e.1.is_empty() {
                    e.1 = format!("{} {:?}", j.0, j.1);
                }
            }
        }
    }
    println!("{same} states agree with the old table, {different} differ");
    println!("-- different Bedrock block ({} kinds)", names.len());
    for (k, n) in names.iter().take(40) {
        println!("{n:>5} {k}");
    }
    // Group by property and the pair of values, summing over blocks; print the biggest.
    let mut grouped: BTreeMap<(String, String, String), (usize, usize, String)> = BTreeMap::new();
    for ((q, m, o, family), (n, ex)) in &by_prop {
        let e = grouped.entry((q.clone(), m.clone(), o.clone())).or_insert((0, 0, String::new()));
        e.0 += n;
        e.1 += 1;
        if e.2.is_empty() {
            e.2 = format!("[{family}] {ex}");
        }
    }
    let mut v: Vec<_> = grouped.into_iter().collect();
    v.sort_by_key(|(_, (n, _, _))| std::cmp::Reverse(*n));
    println!("-- different property values");
    for ((q, m, o), (n, kinds, ex)) in v.iter().take(70) {
        println!("{n:>6} ({kinds:>3} blocks) {q}: mine {m} old {o}   e.g. {}", ex.chars().take(200).collect::<String>());
    }
    Ok(())
}

/// Writes `mappings.nbt.gz`: every Java state with the Bedrock state this tool derived for it, and
/// the biome ids. `r` is how many of a Java state's properties differ from the block's default,
/// which makes the plainest Java state win when several map to one Bedrock state.
pub fn emit(out: &std::path::Path) -> Result<(), String> {
    use std::io::Write as _;
    use mctk_nbt::{Compound, Flavor, List, NamedTag, Tag, TagType};
    let java = builtin::java("26.3").ok_or("no Java registry")?;
    let rows = table()?;
    let mut blocks = Vec::with_capacity(rows.len());
    for (j, b) in &rows {
        let mut e = Compound::new();
        e.insert("j".into(), Tag::String(j.0.clone()));
        if !j.1.is_empty() {
            e.insert("p".into(), Tag::Compound(j.1.iter().map(|(k, v)| (k.clone(), Tag::String(v.clone()))).collect()));
        }
        let defaults = &java.blocks[&j.0].defaults;
        let rank = j.1.iter().filter(|(k, v)| defaults.get(*k) != Some(*v)).count();
        if rank > 0 {
            e.insert("r".into(), Tag::Byte(rank as i8));
        }
        if b.0 != j.0 {
            e.insert("b".into(), Tag::String(b.0.clone()));
        }
        if !b.1.is_empty() {
            e.insert("s".into(), Tag::Compound(b.1.iter().map(|(k, v)| (k.clone(), typed(v))).collect()));
        }
        blocks.push(Tag::Compound(e));
    }
    // Biomes: the ids the Bedrock server saved come first (the names both editions share), then
    // the legacy ids, which are the same numbers in both editions.
    let mut ids: BTreeMap<String, i32> = BTreeMap::new();
    for (name, id) in builtin::bedrock_biome_ids("1.26.61.1") {
        ids.entry(name.to_string()).or_insert(id);
    }
    for line in builtin::legacy_biomes_text().lines().filter(|l| !l.starts_with('#')) {
        if let Some((id, name)) = line.split_once('\t') {
            if let Ok(id) = id.trim().parse::<i32>() {
                ids.entry(name.trim().to_string()).or_insert(id);
            }
        }
    }
    let mut biomes = Vec::new();
    let mut missing = Vec::new();
    for name in &java.biomes {
        match ids.get(name) {
            Some(id) => {
                let mut e = Compound::new();
                e.insert("j".into(), Tag::String(name.clone()));
                e.insert("b".into(), Tag::Int(*id));
                biomes.push(Tag::Compound(e));
            }
            None => missing.push(name.clone()),
        }
    }
    println!("{} biomes with an id, {} without: {:?}", biomes.len(), missing.len(), missing);
    let mut root = Compound::new();
    root.insert("java_version".into(), Tag::String("26.3".into()));
    root.insert("java_data_version".into(), Tag::Int(5023));
    root.insert("bedrock_version".into(), Tag::String("1.26.61".into()));
    root.insert("blocks".into(), Tag::List(List::from_tags(TagType::Compound, blocks).map_err(|e| e.to_string())?));
    root.insert("biomes".into(), Tag::List(List::from_tags(TagType::Compound, biomes).map_err(|e| e.to_string())?));
    let bytes = mctk_nbt::to_bytes(&NamedTag { name: String::new(), tag: Tag::Compound(root) }, Flavor::Java).map_err(|e| e.to_string())?;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(&bytes).map_err(|e| e.to_string())?;
    let gz = gz.finish().map_err(|e| e.to_string())?;
    std::fs::write(out, &gz).map_err(|e| e.to_string())?;
    println!("wrote {} ({} bytes)", out.display(), gz.len());
    Ok(())
}