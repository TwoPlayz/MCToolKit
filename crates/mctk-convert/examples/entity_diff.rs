//! Lists the entity ids only one edition has (before any renames).
use std::collections::HashSet;

fn main() {
    let j = mctk_registry::builtin::java(mctk_registry::builtin::java_versions()[0]).expect("java").entities;
    let b = mctk_registry::builtin::bedrock(mctk_registry::builtin::bedrock_versions()[0]).expect("bedrock").entities;
    let (js, bs): (HashSet<_>, HashSet<_>) = (j.iter().collect(), b.iter().collect());
    println!("java only:");
    for e in j.iter().filter(|e| !bs.contains(e)) {
        println!("  {e}");
    }
    println!("bedrock only:");
    for e in b.iter().filter(|e| !js.contains(e)) {
        println!("  {e}");
    }
}
