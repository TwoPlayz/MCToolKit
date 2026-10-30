//! Prints the first lines where a chunk's re-encoded SNBT differs from the original.
//!
//! cargo run -p mctk-java --example diff_chunk -- <r.x.z.mca> <local x> <local z>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let region = mctk_anvil::Region::open(&args[1])?;
    let original = region
        .read_chunk(args[2].parse()?, args[3].parse()?)?
        .ok_or("no chunk")?;
    let again = mctk_java::Chunk::from_nbt(original.clone())?.to_nbt();
    let (a, b) = (format!("{original:#}"), format!("{again:#}"));
    let (a, b): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let mut shown = 0;
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or("<end>"),
            b.get(i).copied().unwrap_or("<end>"),
        );
        if x != y {
            println!(
                "line {i}\n  original: {}\n  re-coded: {}",
                &x[..x.len().min(140)],
                &y[..y.len().min(140)]
            );
            shown += 1;
            if shown == 6 {
                break;
            }
        }
    }
    println!("{} vs {} lines", a.len(), b.len());
    Ok(())
}
