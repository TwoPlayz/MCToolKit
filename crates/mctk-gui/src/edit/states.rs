//! What block states there are: each block's properties and the values each can take (from the
//! game's own table of states), and its usual state.

use std::collections::HashMap;
use std::sync::OnceLock;

use mctk_java::BlockState;

/// Each block's properties, in the game's order, with their values.
fn table() -> &'static HashMap<String, Vec<(String, Vec<String>)>> {
    static T: OnceLock<HashMap<String, Vec<(String, Vec<String>)>>> = OnceLock::new();
    T.get_or_init(|| {
        let mut out: HashMap<String, Vec<(String, Vec<String>)>> = HashMap::new();
        for s in mctk_mapping::mappings().java_states() {
            let props = out.entry(s.name.clone()).or_default();
            for (k, v) in &s.properties {
                match props.iter_mut().find(|(pk, _)| pk == k) {
                    Some((_, vals)) => {
                        if !vals.contains(v) {
                            vals.push(v.clone());
                        }
                    }
                    None => props.push((k.clone(), vec![v.clone()])),
                }
            }
        }
        out
    })
}

/// A block's properties and the values each can take (none for a block without any).
pub fn properties(name: &str) -> &'static [(String, Vec<String>)] {
    let full = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
    table().get(&full).map_or(&[], Vec::as_slice)
}

/// A block's properties as the block really is, in the game's order: the value the map recorded
/// (`props` as `key=value` pairs joined by commas, maybe none: a block saved without them is in
/// its usual state) and, for any not recorded, the game's default.
pub fn full_properties(name: &str, props: &str) -> Vec<(String, String)> {
    let usual = default_state(name);
    properties(name)
        .iter()
        .map(|(k, _)| {
            let recorded = props.split(',').find_map(|kv| kv.strip_prefix(k.as_str())?.strip_prefix('=')).map(str::to_string);
            (k.clone(), recorded.or_else(|| usual.properties.get(k).cloned()).unwrap_or_default())
        })
        .collect()
}

/// A block in its usual state (every property at the game's default).
pub fn default_state(name: &str) -> BlockState {
    let full = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
    mctk_mapping::mappings().complete_java(&BlockState::new(full))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stairs_have_their_properties_and_a_usual_state() {
        let p = properties("oak_stairs");
        let facing = p.iter().find(|(k, _)| k == "facing").expect("facing");
        assert_eq!(facing.1.len(), 4);
        assert!(p.iter().any(|(k, _)| k == "waterlogged"));
        let s = default_state("minecraft:oak_stairs");
        assert!(s.properties.contains_key("facing") && s.properties.contains_key("half"));
        assert!(properties("stone").is_empty());
    }

    /// A block saved with no properties is its usual state; what was recorded wins.
    #[test]
    fn a_block_with_no_recorded_state_is_in_its_usual_state() {
        let none = full_properties("minecraft:barrel", "");
        assert!(none.iter().any(|(k, v)| k == "facing" && !v.is_empty()), "{none:?}");
        assert!(none.iter().any(|(k, v)| k == "open" && v == "false"), "{none:?}");
        let set = full_properties("minecraft:barrel", "facing=up");
        assert_eq!(set.iter().find(|(k, _)| k == "facing").map(|(_, v)| v.as_str()), Some("up"));
        assert!(full_properties("minecraft:stone", "").is_empty());
    }
}
