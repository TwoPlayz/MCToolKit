//! `cargo run -p mctk-replace --example scan -- <java-world> <target-registry.tsv>`
//! Lists the block states the target version cannot represent exactly.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [world, registry] = args.as_slice() else {
        eprintln!("usage: scan <java-world> <target-registry.tsv>");
        std::process::exit(2);
    };
    let target = mctk_registry::Registry::from_text(
        &std::fs::read_to_string(registry).expect("read registry"),
    )
    .expect("parse registry");
    let scan = mctk_replace::Scan::java_world(world).expect("scan world");
    let bad = scan.unsupported(&target);
    println!(
        "{} chunks, {} distinct states; {} not exact in {} {}",
        scan.chunks,
        scan.blocks.len(),
        bad.len(),
        target.edition.as_str(),
        target.version
    );
    let renames = mctk_registry::Renames::java_blocks();
    for u in &bad {
        let s = mctk_replace::suggest(&u.state, &target, &renames).expect("not exact");
        println!("{:>9}  {}  ->  {:?} ({:?})", u.count, u.state, s.action, s.reason);
    }
}
