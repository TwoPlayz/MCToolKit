//! Smart defaults: the best replacement we can propose for a block state the target version
//! cannot represent exactly. The user can always override a suggestion.

use indexmap::IndexMap;
use mctk_java::BlockState;
use mctk_registry::{BlockCheck, Registry, Renames};

/// What to put in the target world instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Replace(BlockState),
    /// Use this block instead and carry the properties over (rotation, half, ...) wherever the
    /// new block has them.
    Block(String),
    /// Remove the block (air).
    Air,
    /// Write the state unchanged even though the target does not know it.
    Keep,
}

/// Why a suggestion was made, in decreasing order of confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The block exists but some properties or values did not; they were adapted.
    AdaptedState,
    /// The block was renamed between the versions.
    Renamed,
    /// A block of the same shape in a different material (e.g. `pale_oak_stairs` ->
    /// `oak_stairs`).
    SameShape,
    /// Nothing comparable exists.
    NoEquivalent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub action: Action,
    pub reason: Reason,
}

/// Materials tried, in order, when only the shape of a block (its trailing words) can be
/// matched. Stone-like shapes prefer stone, everything else oak.
fn materials_for(suffix: &str) -> &'static [&'static str] {
    match suffix {
        "slab" | "stairs" | "wall" | "bricks" | "brick_slab" | "brick_stairs" | "brick_wall" => {
            &["stone", "cobblestone", "oak"]
        }
        _ => &["oak", "stone"],
    }
}

/// `state` with the properties `target` knows for `name`: unknown properties dropped, values
/// the version does not allow replaced by the default. `None` if `name` does not exist.
pub(crate) fn adapt(state: &BlockState, name: &str, target: &Registry) -> Option<BlockState> {
    let def = target.blocks.get(name)?;
    let mut out = BlockState::new(name);
    for (k, v) in &state.properties {
        if let Some(allowed) = def.properties.get(k) {
            if allowed.iter().any(|a| a == v) {
                out.properties.insert(k.clone(), v.clone());
            }
        }
    }
    Some(out)
}

/// Proposes a replacement for `state`, or `None` when it is already exact in `target`.
pub fn suggest(state: &BlockState, target: &Registry, renames: &Renames) -> Option<Suggestion> {
    let props = || state.properties.iter().map(|(k, v)| (k.as_str(), v.as_str()));
    match target.check_block(&state.name, props()) {
        BlockCheck::Exact => return None,
        BlockCheck::BadState(_) => {
            // A double slab the target has no state for is a full block of the same material.
            if state.properties.get("type").is_some_and(|t| t == "double") {
                if let Some(base) = state.name.strip_suffix("_slab") {
                    for full in [base.to_string(), format!("{base}s"), format!("{base}_planks"), format!("{base}_block")] {
                        if target.has_block(&full) {
                            if let Some(s) = adapt(state, &full, target) {
                                return Some(Suggestion { action: Action::Replace(s), reason: Reason::SameShape });
                            }
                        }
                    }
                }
            }
            return Some(Suggestion {
                action: Action::Replace(adapt(state, &state.name, target)?),
                reason: Reason::AdaptedState,
            });
        }
        BlockCheck::UnknownBlock => {}
    }

    if let Some(other) = renames.counterpart(&state.name) {
        if let Some(s) = adapt(state, other, target) {
            return Some(Suggestion {
                action: Action::Replace(s),
                reason: Reason::Renamed,
            });
        }
    }

    if let Some(s) = same_family(state, target) {
        return Some(Suggestion { action: Action::Replace(s), reason: Reason::SameShape });
    }

    // Same shape, other material: keep the last one or two words and swap the material.
    let bare = state.name.strip_prefix("minecraft:").unwrap_or(&state.name);
    let words: Vec<&str> = bare.split('_').collect();
    for take in [2usize, 1] {
        if words.len() <= take {
            continue;
        }
        let suffix = words[words.len() - take..].join("_");
        for material in materials_for(&suffix) {
            let candidate = format!("minecraft:{material}_{suffix}");
            if let Some(s) = adapt(state, &candidate, target) {
                return Some(Suggestion {
                    action: Action::Replace(s),
                    reason: Reason::SameShape,
                });
            }
        }
    }

    // Coloured solid blocks a version lacks stand in as wool of the same colour.
    for tail in ["_glazed_terracotta", "_concrete_powder", "_terracotta", "_concrete"] {
        if let Some(colour) = bare.strip_suffix(tail) {
            for wool in [format!("minecraft:{colour}_wool"), "minecraft:white_wool".to_string()] {
                if let Some(s) = adapt(state, &wool, target) {
                    return Some(Suggestion { action: Action::Replace(s), reason: Reason::SameShape });
                }
            }
        }
    }

    Some(Suggestion {
        action: Action::Air,
        reason: Reason::NoEquivalent,
    })
}

/// Materials and colours preferred, in order, when several blocks of the target share a shape.
const PREFERRED: &[&str] = &["oak", "red", "white", "stone", "cobblestone", "spruce", "birch", "dark_oak", "black"];

const WOODS: &[&str] = &["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry", "pale_oak", "bamboo", "crimson", "warped"];

/// One-word shapes (the last word of a block name) that are safe to match across materials.
const SHAPES: &[&str] = &[
    "stairs", "slab", "wall", "fence", "door", "trapdoor", "button", "log", "wood", "planks", "leaves", "sapling",
    "bed", "banner", "carpet", "wool", "candle", "concrete", "terracotta", "glass", "pane", "bricks", "sign",
    "head", "skull", "coral", "fan", "lantern", "torch", "boat", "plate", "gate",
];

/// A block of the target that shares the end of `state`'s name, nearest first (a coloured bed
/// the version lacks becomes the red bed, a waxed oxidised copper stair its plain cut-copper
/// stair), with the properties the new block has carried over.
fn same_family(state: &BlockState, target: &Registry) -> Option<BlockState> {
    let bare = state.name.strip_prefix("minecraft:").unwrap_or(&state.name);
    let words: Vec<&str> = bare.split('_').collect();
    for take in (1..words.len()).rev() {
        let suffix = words[words.len() - take..].join("_");
        if take == 1 && !SHAPES.contains(&suffix.as_str()) {
            continue;
        }
        let tail = format!("_{suffix}");
        // Wooden things stay wooden; stone-like shapes prefer stone.
        let wooden = WOODS.iter().any(|w| bare.starts_with(&format!("{w}_")));
        let first: &[&str] = if wooden { &["oak", "spruce", "birch"] } else { materials_for(&suffix) };
        let order: Vec<&str> = first.iter().chain(PREFERRED).copied().collect();
        let mut best: Option<((usize, usize), &String)> = None;
        for name in target.blocks.keys() {
            let nb = name.strip_prefix("minecraft:").unwrap_or(name);
            let rank = if nb == suffix {
                0
            } else if let Some(material) = nb.strip_suffix(&tail) {
                1 + order.iter().position(|p| *p == material).unwrap_or(order.len())
            } else {
                continue;
            };
            let key = (rank, nb.len());
            if best.is_none_or(|(b, n)| key < b || (key == b && name < n)) {
                best = Some((key, name));
            }
        }
        if let Some((_, name)) = best {
            if let Some(s) = adapt(state, name, target) {
                return Some(s);
            }
        }
    }
    None
}

/// Suggestions for every state in `states` that needs one, keyed by the state.
pub fn suggest_all<'a>(
    states: impl IntoIterator<Item = &'a BlockState>,
    target: &Registry,
    renames: &Renames,
) -> IndexMap<BlockState, Suggestion> {
    states
        .into_iter()
        .filter_map(|s| suggest(s, target, renames).map(|g| (s.clone(), g)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mctk_registry::BlockDef;

    fn def(props: &[(&str, &[&str])]) -> BlockDef {
        BlockDef {
            properties: props
                .iter()
                .map(|(k, vs)| (k.to_string(), vs.iter().map(|v| v.to_string()).collect()))
                .collect(),
            defaults: props
                .iter()
                .map(|(k, vs)| (k.to_string(), vs[0].to_string()))
                .collect(),
        }
    }

    fn target() -> Registry {
        let mut t = Registry::new(mctk_registry::Edition::Java, "t");
        let facing: &[&str] = &["north", "south"];
        for (name, d) in [
            ("minecraft:grass", def(&[])),
            ("minecraft:oak_stairs", def(&[("facing", facing)])),
            ("minecraft:stone_slab", def(&[])),
            ("minecraft:stone_brick_slab", def(&[])),
            ("minecraft:oak_planks", def(&[])),
            ("minecraft:stone", def(&[])),
        ] {
            t.blocks.insert(name.into(), d);
        }
        t
    }

    fn s(name: &str) -> BlockState {
        BlockState::new(name)
    }

    #[test]
    fn missing_colour_becomes_the_nearest_block_of_the_family_not_air() {
        let mut t = target();
        t.blocks.insert("minecraft:red_bed".into(), def(&[("facing", &["north", "south"]), ("part", &["head", "foot"])]));
        t.blocks.insert("minecraft:cut_copper_stairs".into(), def(&[("facing", &["north", "south"])]));
        let bed = s("minecraft:cyan_bed").with("facing", "south").with("part", "head").with("occupied", "false");
        let g = suggest(&bed, &t, &Renames::default()).unwrap();
        assert_eq!(g.action, Action::Replace(s("minecraft:red_bed").with("facing", "south").with("part", "head")));
        let g = suggest(&s("minecraft:waxed_oxidized_cut_copper_stairs").with("facing", "north"), &t, &Renames::default()).unwrap();
        assert_eq!(g.action, Action::Replace(s("minecraft:cut_copper_stairs").with("facing", "north")));
    }

    #[test]
    fn exact_needs_no_suggestion() {
        assert_eq!(suggest(&s("minecraft:stone"), &target(), &Renames::java_blocks()), None);
    }

    #[test]
    fn renamed_block_is_followed() {
        let g = suggest(&s("minecraft:short_grass"), &target(), &Renames::java_blocks()).unwrap();
        assert_eq!(g.reason, Reason::Renamed);
        assert_eq!(g.action, Action::Replace(s("minecraft:grass")));
    }

    #[test]
    fn state_is_adapted_keeping_valid_properties() {
        let st = s("minecraft:oak_stairs")
            .with("facing", "south")
            .with("half", "top")
            .with("shape", "straight");
        let g = suggest(&st, &target(), &Renames::default()).unwrap();
        assert_eq!(g.reason, Reason::AdaptedState);
        assert_eq!(
            g.action,
            Action::Replace(s("minecraft:oak_stairs").with("facing", "south"))
        );
    }

    #[test]
    fn same_shape_keeps_rotation_and_swaps_material() {
        let st = s("minecraft:pale_oak_stairs").with("facing", "north").with("half", "top");
        let g = suggest(&st, &target(), &Renames::default()).unwrap();
        assert_eq!(g.reason, Reason::SameShape);
        assert_eq!(
            g.action,
            Action::Replace(s("minecraft:oak_stairs").with("facing", "north"))
        );
        // Stone-like shapes prefer stone; two trailing words are tried first.
        let g = suggest(&s("minecraft:cinnabar_brick_slab"), &target(), &Renames::default()).unwrap();
        assert_eq!(g.action, Action::Replace(s("minecraft:stone_brick_slab")));
        let g = suggest(&s("minecraft:poplar_planks"), &target(), &Renames::default()).unwrap();
        assert_eq!(g.action, Action::Replace(s("minecraft:oak_planks")));
    }

    #[test]
    fn unsupported_double_slab_becomes_the_full_block() {
        let mut t = Registry::new(mctk_registry::Edition::Java, "t");
        let types: &[&str] = &["top", "bottom"];
        for (n, d) in [
            ("minecraft:black_wool_slab", def(&[("type", types)])),
            ("minecraft:black_wool", def(&[])),
            ("minecraft:oak_slab", def(&[("type", types)])),
            ("minecraft:oak_planks", def(&[])),
            ("minecraft:brick_slab", def(&[("type", types)])),
            ("minecraft:bricks", def(&[])),
        ] {
            t.blocks.insert(n.into(), d);
        }
        let none = Renames::default();
        for (slab, full) in [("black_wool", "black_wool"), ("oak", "oak_planks"), ("brick", "bricks")] {
            let st = s(&format!("minecraft:{slab}_slab")).with("type", "double");
            let g = suggest(&st, &t, &none).unwrap();
            assert_eq!(g.action, Action::Replace(s(&format!("minecraft:{full}"))), "{slab}");
        }
        // A half slab keeps being a slab.
        let g = suggest(&s("minecraft:oak_slab").with("type", "bottom"), &t, &none);
        assert!(g.is_none());
    }

    #[test]
    fn nothing_comparable_becomes_air() {
        let g = suggest(&s("minecraft:leaf_litter"), &target(), &Renames::default()).unwrap();
        assert_eq!(g.reason, Reason::NoEquivalent);
        assert_eq!(g.action, Action::Air);
    }
}
