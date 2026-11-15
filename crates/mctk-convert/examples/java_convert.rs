//! `cargo run --release -p mctk-convert --example java_convert -- <src> <dst> <target-registry.tsv> [map.tsv]`
//! Converts a Java world to the classic (1.18 - 1.21) layout of the target version and
//! prints what was replaced or could not be carried over.

use mctk_convert::{ConvertOptions, JavaTarget, Progress, java_to_java};
use mctk_replace::ReplacementMap;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: java_convert <src> <dst> <target-registry.tsv> [map.tsv]");
        std::process::exit(2);
    }
    let registry = mctk_registry::Registry::from_text(
        &std::fs::read_to_string(&args[2]).expect("read registry"),
    )
    .expect("parse registry");
    let map = match args.get(3) {
        Some(p) => ReplacementMap::from_text(&std::fs::read_to_string(p).expect("read map"))
            .expect("parse map"),
        None => ReplacementMap::default(),
    };
    let target = JavaTarget::for_version(registry).expect("supported target version");
    let start = std::time::Instant::now();
    let report = java_to_java(
        args[0].as_ref(),
        args[1].as_ref(),
        &target,
        &map,
        &ConvertOptions::default(),
        &Progress::default(),
    )
    .expect("conversion");
    println!("{} chunks in {:.1}s", report.chunks, start.elapsed().as_secs_f32());
    println!("unsupported (pre-1.18) chunks: {}", report.unsupported_chunks);
    for (from, r) in &report.replaced {
        println!("replaced {:>8} x {from} -> {}", r.blocks, r.to);
    }
    for (from, n) in &report.biomes_replaced {
        println!("biome {from}: {n} cells");
    }
    println!("block entities converted: {}, entities converted: {}", report.block_entities_converted, report.entities_converted);
    println!("players: {}, saved-data files copied: {}", report.players, report.saved_data_files);
    for (what, n) in &report.notes.0 {
        println!("not carried over ({n}x): {what}");
    }
}
