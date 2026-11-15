//! `cargo run --release -p mctk-convert --example java_from_bedrock -- <bedrock-world> <out> [map.tsv]`
//! Converts a Bedrock world to Java 26.3, replacing the blocks Java cannot hold (smart defaults,
//! or the choices in `map.tsv`). Prints what the scan found and what the conversion did.

use mctk_convert::{ConvertOptions, JavaReplacements, Progress, bedrock_to_java_replacing, java_view};
use mctk_replace::{Plan, ReplacementMap, Scan};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: java_from_bedrock <bedrock-world> <out> [map.tsv]");
        std::process::exit(2);
    }
    let java = mctk_registry::builtin::java("26.3").expect("Java registry");
    let bedrock = mctk_registry::builtin::bedrock("1.26.61.1").expect("Bedrock registry");
    let scan = Scan::bedrock_world(&args[0]).expect("scan");
    let view = java_view(&java, &bedrock);
    let plan = Plan::build(&scan, &view, &mctk_registry::Renames::default());
    println!("scan: {} columns, {} distinct states, {} kinds of blocks need a decision", scan.chunks, scan.blocks.len(), plan.groups.len());
    for g in plan.groups.iter().take(12) {
        println!("  {:>8} x {:<40} -> {}", g.total(), g.name, g.suggestion_text());
    }
    let map = match args.get(2) {
        Some(p) => ReplacementMap::from_text(&std::fs::read_to_string(p).expect("read map")).expect("parse map"),
        None => ReplacementMap::default(),
    };
    let replace = JavaReplacements::new(map, &java, &bedrock);
    let start = std::time::Instant::now();
    let report = bedrock_to_java_replacing(args[0].as_ref(), args[1].as_ref(), &ConvertOptions::default(), &Progress::default(), Some(&replace))
        .expect("conversion");
    println!("{} chunks in {:.1}s", report.chunks.values().sum::<usize>(), start.elapsed().as_secs_f32());
    let total: usize = report.replaced.values().sum();
    println!("{} blocks replaced in {} kinds", total, report.replaced.len());
    for (k, n) in report.replaced.iter().take(8) {
        println!("  replaced {n:>8} x {k}");
    }
    for (k, n) in report.unmapped.iter().take(8) {
        println!("  unmapped (air) {n}x {k}");
    }
    for (k, n) in &report.skipped {
        println!("  left out {n}x {k}");
    }
}
