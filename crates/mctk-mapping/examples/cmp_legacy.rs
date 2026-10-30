//! Cross-checks the two legacy tables: for every old Bedrock `(name, val)` whose name is also a
//! pre-1.13 Java string id, the Java block it converts to must equal what the Java legacy
//! table gives for the same numeric id and data value.
//!
//! cargo run -p mctk-mapping --example cmp_legacy

use std::collections::BTreeMap;

use mctk_mapping::legacy::{legacy, legacy_bedrock_block};
use mctk_mapping::mappings;

const BEDROCK_BLOCKS: &str = include_str!("../data/legacy_bedrock_blocks.tsv");

fn main() {
    let m = mappings();
    let l = legacy();
    let (mut same, mut diff, mut skipped) = (0, 0, 0);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for line in BEDROCK_BLOCKS.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let (name, val): (&str, u8) = (f[0], f[1].parse().unwrap());
        let Some(id) = l.numeric_id(name).filter(|i| *i < 256) else {
            skipped += 1;
            continue;
        };
        if !l.has_exact_block(id, val) {
            skipped += 1;
            continue;
        }
        let want = m.complete_java(l.block(id, val).unwrap());
        let b = legacy_bedrock_block(name, val as u16).unwrap();
        let got = m.bedrock_to_java(&b.name, &b.states).map(|j| m.complete_java(j));
        if got.as_ref() == Some(&want) {
            same += 1;
        } else {
            diff += 1;
            // Doors, fences and the like differ only by connections the game recomputes.
            if ["minecraft:trapdoor", "minecraft:lever", "minecraft:hopper", "minecraft:acacia_door", "minecraft:leaves"].contains(&name) && std::env::var("SHOW").is_ok() {
                println!("{name}:{val}\n   bedrock route {:?}\n   java route    {:?}", got.as_ref().map(|g| g.to_string()), want.to_string());
            }
            let key = format!("{name} -> {} vs {}", got.map_or("none".to_string(), |g| g.name), want.name);
            *kinds.entry(key).or_default() += 1;
        }
    }
    println!("same {same}, different {diff}, not comparable {skipped}");
    let mut v: Vec<_> = kinds.into_iter().collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    for (k, n) in v.iter().take(40) {
        println!("  {n:4} x {k}");
    }
}
