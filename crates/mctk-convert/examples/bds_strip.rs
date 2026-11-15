//! Copies a Bedrock world without some kinds of records, to find which kind makes a game crash.
//!
//! cargo run --release -p mctk-convert --example bds_strip -- <world> <out> <be|entities|both>
use mctk_bedrock::BedrockWorld;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let (src, out, mode) = (a.get(1).ok_or("world")?, a.get(2).ok_or("out")?, a.get(3).ok_or("mode")?);
    let mut world = BedrockWorld::open(src)?;
    let before = world.db.entries.len();
    // `only:<x>:<z>` keeps the records of that one overworld chunk.
    if let Some(rest) = mode.strip_prefix("only:") {
        let mut it = rest.split(':').map(|v| v.parse::<i32>().unwrap());
        let (cx, cz) = (it.next().unwrap(), it.next().unwrap());
        let mut prefix = cx.to_le_bytes().to_vec();
        prefix.extend(cz.to_le_bytes());
        // Every chunk record of any dimension goes except this one overworld chunk.
        world.db.entries.retain(|k, _| !matches!(k.len(), 9 | 10 | 13 | 14) || (matches!(k.len(), 9 | 10) && k.starts_with(&prefix)));
        println!("{} records, {} after", before, world.db.entries.len());
        world.save_to_folder(out)?;
        return Ok(());
    }
    world.db.entries.retain(|k, _| {
        let chunk_tag = if matches!(k.len(), 9 | 13) { Some(k[k.len() - 1]) } else { None };
        let be = chunk_tag == Some(49);
        let entity = chunk_tag == Some(50) || k.starts_with(b"actorprefix") || k.starts_with(b"digp");
        !(((mode == "be" || mode == "both") && be) || ((mode == "entities" || mode == "both") && entity))
    });
    println!("{} records, {} after", before, world.db.entries.len());
    world.save_to_folder(out)?;
    Ok(())
}
