//! `cargo run --release -p mctk-prune --example optimize -- <Bedrock world> <out folder>`
//! Writes the world again with a compacted database and says how the size changed.
fn size(p: &std::path::Path) -> u64 {
    match std::fs::metadata(p) {
        Ok(m) if m.is_file() => m.len(),
        Ok(_) => std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| size(&e.path())).sum(),
        Err(_) => 0,
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (src, out) = (std::path::Path::new(&a[1]), std::path::Path::new(&a[2]));
    let _ = std::fs::remove_dir_all(out);
    let chunks = mctk_prune::chunk_times_bedrock(src, 0).expect("chunks");
    println!("{} overworld chunks", chunks.len());
    let records = mctk_prune::optimize_bedrock_world(src, out).expect("optimize");
    println!("{records} records; {} bytes before, {} after", size(src), size(out));
    let again = mctk_prune::chunk_times_bedrock(out, 0).expect("chunks of the result");
    assert_eq!(chunks.len(), again.len(), "the optimized world has the same chunks");
    println!("the optimized world has the same {} chunks", again.len());
}
