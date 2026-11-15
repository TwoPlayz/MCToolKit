//! `cargo run --release -p mctk-convert --example bedrock_convert -- <java-world> <out> [map.tsv]`
//! Converts a Java world to Bedrock, replacing the blocks the target version cannot hold
//! (smart defaults, or the choices in `map.tsv`), and prints what happened.

use mctk_convert::{BedrockReplacements, ConvertOptions, Progress, java_to_bedrock_replacing};
use mctk_replace::ReplacementMap;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: bedrock_convert <java-world> <out> [map.tsv]");
        std::process::exit(2);
    }
    let map = match args.get(2) {
        Some(p) => ReplacementMap::from_text(&std::fs::read_to_string(p).expect("read map")).expect("parse map"),
        None => ReplacementMap::default(),
    };
    // MCTK_BEDROCK_REGISTRY: a registry file for another Bedrock version.
    let bedrock = match std::env::var("MCTK_BEDROCK_REGISTRY") {
        Ok(p) => mctk_registry::Registry::from_text(&std::fs::read_to_string(p).expect("read registry")).expect("parse registry"),
        Err(_) => mctk_registry::builtin::bedrock("1.26.61.1").expect("built-in Bedrock registry"),
    };
    mctk_convert::set_bedrock_target(&bedrock);
    let replace = BedrockReplacements::new(map, &bedrock);
    let start = std::time::Instant::now();
    let report = java_to_bedrock_replacing(
        args[0].as_ref(),
        args[1].as_ref(),
        &ConvertOptions::default(),
        &Progress::default(),
        Some(&replace),
    )
    .expect("conversion");
    let chunks: usize = report.chunks.values().sum();
    println!("{chunks} chunks in {:.1}s", start.elapsed().as_secs_f32());
    for (k, n) in &report.replaced {
        println!("replaced {n:>8} x {k}");
    }
    for (k, n) in &report.unmapped {
        println!("unmapped (air) {n}x {k}");
    }
    for (k, n) in report.approximated.iter().take(10) {
        println!("approximated {n}x {k}");
    }
    for (k, n) in &report.skipped {
        println!("left out {n}x {k}");
    }
}
