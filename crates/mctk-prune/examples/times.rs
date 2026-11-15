//! `cargo run --release -p mctk-prune --example times -- <console save or Java world> <java|console> [dimension]`
//! Prints how the chunks' InhabitedTime is spread, as the pruner's Time layer sees it.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dim: i32 = a.get(3).and_then(|d| d.parse().ok()).unwrap_or(0);
    let path = std::path::Path::new(&a[1]);
    let times = match a[2].as_str() {
        "java" => mctk_prune::chunk_times_java(path, dim, true),
        _ => mctk_prune::chunk_times_console(path, dim, true),
    }
    .expect("read the times");
    let zero = times.values().filter(|t| **t == Some(0)).count();
    let max = times.values().flatten().max().copied().unwrap_or(0);
    println!("{} chunks, {} record InhabitedTime; {zero} are exactly 0; the most is {max} ticks", times.len(), times.values().flatten().count());
}

