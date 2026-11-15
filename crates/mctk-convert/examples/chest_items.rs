//! `cargo run --release -p mctk-convert --example chest_items -- <world>`
//! Prints the stacks of every chest block entity in the world's overworld chunks as SNBT, one
//! per line, in slot order. Used to see how the game itself upgrades items between versions.
use mctk_anvil::Region;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = std::path::Path::new(&args[0]).join("region");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    files.sort_by_key(|e| e.file_name());
    for entry in files {
        let Ok(region) = Region::open(entry.path()) else { continue };
        for (lx, lz) in region.chunks() {
            let Ok(Some(raw)) = region.raw_chunk(lx, lz) else { continue };
            let Ok(nbt) = raw.nbt() else { continue };
            // 1.18+ keeps block entities at the top; older chunks under "Level".
            let root = nbt.tag.get("Level").unwrap_or(&nbt.tag);
            let Some(list) = root
                .get("block_entities")
                .or_else(|| root.get("TileEntities"))
                .and_then(|t| t.as_list())
            else {
                continue;
            };
            for be in list.iter() {
                if be.get("id").and_then(|i| i.as_str()) != Some("minecraft:chest") {
                    continue;
                }
                let Some(items) = be.get("Items").and_then(|t| t.as_list()) else { continue };
                let mut stacks: Vec<_> = items.iter().collect();
                stacks.sort_by_key(|s| s.get("Slot").and_then(|t| t.as_i64()).unwrap_or(0));
                for s in stacks {
                    println!("{s}");
                }
            }
        }
    }
}
