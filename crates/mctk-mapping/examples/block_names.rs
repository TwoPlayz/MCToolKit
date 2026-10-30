//! Prints, for every Java block name, the Bedrock block names its states map to
//! (for comparing with other converters).
use std::collections::{BTreeMap, BTreeSet};

fn main() {
    let m = mctk_mapping::mappings();
    let mut names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for s in m.java_states() {
        let full = m.complete_java(s);
        if let Some(b) = m.java_to_bedrock(&full) {
            names.entry(s.name.clone()).or_default().insert(b.name.clone());
        }
    }
    for (java, bedrock) in names {
        println!("{java}\t{}", bedrock.into_iter().collect::<Vec<_>>().join(","));
    }
}
