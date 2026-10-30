//! Reports gaps in the built-in tables themselves (not tied to any world).
//!
//! cargo run --release -p mctk-mapping --example gaps

use std::collections::BTreeMap;

use mctk_mapping::mappings;

fn main() {
    let m = mappings();
    let n = m.java_state_count();
    println!("{n} Java block states");
    // Collect through the public API only: round-trip every Java state.
    let mut to_air: BTreeMap<String, usize> = BTreeMap::new();
    let mut lossy: BTreeMap<String, usize> = BTreeMap::new();
    let mut renamed: BTreeMap<String, String> = BTreeMap::new();
    for js in m.java_states() {
        let Some(b) = m.java_to_bedrock(js) else {
            *to_air.entry(js.name.clone()).or_default() += 1;
            continue;
        };
        if b.name == "minecraft:air" && js.name != "minecraft:air" {
            *to_air.entry(js.name.clone()).or_default() += 1;
        }
        if b.name != js.name {
            renamed.insert(js.name.clone(), b.name.clone());
        }
        let back = m.bedrock_to_java(&b.name, &b.states);
        if back != Some(&m.complete_java(js)) {
            *lossy.entry(js.name.clone()).or_default() += 1;
        }
    }
    println!("\nJava blocks that map to air or nothing:");
    for (k, v) in &to_air {
        println!("  {v:5} {k}");
    }
    println!("\nJava blocks renamed on Bedrock: {}", renamed.len());
    println!("\nBlocks whose states do not all round-trip: {}", lossy.len());
    for (k, v) in lossy.iter().take(40) {
        println!("  {v:5} {k}");
    }
    let mut names: Vec<_> = m.java_biome_names().collect();
    names.sort();
    println!("\nBIOMES {}", names.join(" "));
}


