//! A Java-side view of what a Bedrock version can hold.
//!
//! Java to Bedrock converts block states through the mapping tables. A state is *supported*
//! when its Bedrock counterpart is exact in the game-built Bedrock registry. The view is a
//! registry in Java's block ids and properties holding only supported states, so the same scan,
//! smart defaults and replacement choices that work for any Java target work for Bedrock too.
//!
//! The view keeps, per Java block, the property values that occur in supported states. A block
//! with some unsupported states (a slab type Bedrock has no id for) loses those values, so a
//! state using one is reported and replaced.

use indexmap::IndexMap;
use mctk_java::BlockState;
use mctk_mapping::mappings;
use mctk_nbt::Tag;
use mctk_registry::{BlockCheck, BlockDef, Edition, Registry};

/// The Java blocks and states of the mapping whose Bedrock form is exact in `bedrock`.
pub fn bedrock_view(bedrock: &Registry) -> Registry {
    let m = mappings();
    let mut reg = Registry::new(Edition::Java, format!("Bedrock {}", bedrock.version));
    let mut values: IndexMap<String, IndexMap<String, Vec<String>>> = IndexMap::new();
    for java in m.java_states() {
        let Some(b) = m.java_to_bedrock(java) else { continue };
        let props: Vec<(String, String)> = b
            .states
            .iter()
            .map(|(k, v)| {
                let text = match v {
                    Tag::Byte(n) => format!("b:{n}"),
                    Tag::Int(n) => format!("i:{n}"),
                    Tag::String(s) => format!("s:{s}"),
                    other => format!("?:{other}"),
                };
                (k.clone(), text)
            })
            .collect();
        if bedrock.check_block(&b.name, props.iter().map(|(k, v)| (k.as_str(), v.as_str()))) != BlockCheck::Exact {
            continue;
        }
        let entry = values.entry(java.name.clone()).or_default();
        for (k, v) in &java.properties {
            let list = entry.entry(k.clone()).or_default();
            if !list.contains(v) {
                list.push(v.clone());
            }
        }
    }
    for (name, props) in values {
        // The block's own default state, where Bedrock can hold it; otherwise the first value
        // that it can.
        let default = m.complete_java(&BlockState::new(name.clone()));
        let defaults = props
            .iter()
            .map(|(k, vs)| {
                let d = default.properties.get(k).filter(|d| vs.contains(d)).unwrap_or(&vs[0]);
                (k.clone(), d.clone())
            })
            .collect();
        reg.blocks.insert(name, BlockDef { properties: props, defaults });
    }
    reg
}

/// The Bedrock-side counterpart: the Bedrock blocks and states of the mapping that convert
/// exactly into `java`, in Bedrock's ids and states (values typed as `b:`/`i:`/`s:`). Bedrock to
/// Java uses it to find the blocks a Java version cannot hold.
pub fn java_view(java: &Registry, bedrock: &Registry) -> Registry {
    let m = mappings();
    let mut reg = Registry::new(Edition::Bedrock, format!("Java {}", java.version));
    // Typed text (`b:1`, `i:3`, `s:top`) back to the tag the mapping looks up.
    let tag_of = |v: &str| -> Option<Tag> {
        let (kind, text) = v.split_once(':')?;
        Some(match kind {
            "b" => Tag::Byte(text.parse().ok()?),
            "i" => Tag::Int(text.parse().ok()?),
            _ => Tag::String(text.to_string()),
        })
    };
    // Whether the mapping turns this Bedrock state into a Java state of `java`'s version.
    let converts = |name: &str, state: &IndexMap<String, String>| -> bool {
        let states: mctk_nbt::Compound = state.iter().filter_map(|(k, v)| Some((k.clone(), tag_of(v)?))).collect();
        match m.bedrock_to_java(name, &states) {
            Some(j) => java.check_block(&j.name, j.properties.iter().map(|(k, v)| (k.as_str(), v.as_str()))) == BlockCheck::Exact,
            None => false,
        }
    };
    for (name, def) in &bedrock.blocks {
        // A block is available when its default state converts; each state value when the block
        // still converts with just that value changed (Bedrock's states are independent).
        if !converts(name, &def.defaults) {
            continue;
        }
        let mut props: IndexMap<String, Vec<String>> = IndexMap::new();
        for (key, values) in &def.properties {
            let ok: Vec<String> = values
                .iter()
                .filter(|v| {
                    let mut s = def.defaults.clone();
                    s.insert(key.clone(), (*v).clone());
                    converts(name, &s)
                })
                .cloned()
                .collect();
            props.insert(key.clone(), if ok.is_empty() { vec![def.defaults[key].clone()] } else { ok });
        }
        reg.blocks.insert(name.clone(), BlockDef { properties: props, defaults: def.defaults.clone() });
    }
    // Item frames are blocks in Bedrock and entities in Java; the converter turns them into
    // entities itself, and a block caught mid-move becomes air, so none of them needs a choice.
    for (name, def) in &bedrock.blocks {
        if crate::frames::is_frame_block(name) || name == "minecraft:moving_block" {
            reg.blocks.insert(name.clone(), def.clone());
        }
    }
    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The biome ids the real Bedrock server wrote must be the ones the mapping uses: each turns
    /// into a Java biome and back to the same id, and is a biome the game knows by name.
    #[test]
    fn mapping_biome_ids_agree_with_the_game() {
        let bedrock = mctk_registry::builtin::bedrock("1.26.61.1").unwrap();
        let verified = mctk_registry::builtin::bedrock_biome_ids("1.26.61.1");
        assert!(verified.len() >= 30, "{}", verified.len());
        let m = mappings();
        for (name, id) in verified {
            assert!(bedrock.biomes.iter().any(|b| b == name), "{name} is not a Bedrock biome");
            let java = m.bedrock_biome_to_java(id).unwrap_or_else(|| panic!("{name} ({id}) has no Java biome"));
            assert_eq!(m.java_biome_to_bedrock(java), Some(id), "{name} -> {java}");
        }
    }

    #[test]
    fn view_holds_what_the_game_can_and_drops_what_it_cannot() {
        let bedrock = mctk_registry::builtin::bedrock("1.26.61.1").unwrap();
        let view = bedrock_view(&bedrock);
        assert!(view.blocks.len() > 1000, "{}", view.blocks.len());
        // Ordinary blocks are fully supported, with Java's property names.
        assert!(view.has_block("minecraft:stone") && view.has_block("minecraft:oak_stairs"));
        assert_eq!(
            view.check_block("minecraft:oak_stairs", [("facing", "north"), ("half", "top"), ("shape", "straight"), ("waterlogged", "false")]),
            BlockCheck::Exact
        );
        // The game has no id for a double wool slab, so that slab type is not available.
        let slab = &view.blocks["minecraft:black_wool_slab"];
        assert!(slab.properties["type"].contains(&"bottom".to_string()));
        assert!(!slab.properties["type"].contains(&"double".to_string()), "{:?}", slab.properties["type"]);
        // A block that has no Bedrock id at all is not in the view.
        assert!(!view.has_block("minecraft:moving_piston") || view.blocks["minecraft:moving_piston"].properties.is_empty());
    }
}
