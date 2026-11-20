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
}
