//! Puts some of one world's records for chunk (0, 0) into a copy of another world, to find which
//! record makes a game crash.
//!
//! cargo run --release -p mctk-convert --example bds_mix -- <good world> <mine> <out> <data3d|sub|version|final|blend|all>
use mctk_bedrock::BedrockWorld;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let (good, mine, out, what) = (&a[1], &a[2], &a[3], &a[4]);
    let mut world = BedrockWorld::open(good)?;
    let other = BedrockWorld::open(mine)?;
    let wanted = |k: &[u8]| -> bool {
        if !(k.len() == 9 || k.len() == 10) || k[..8] != [0u8; 8] {
            return false;
        }
        match (what.as_str(), k[8]) {
            ("all", 43 | 44 | 47 | 54 | 64) => true,
            ("data3d", 43) | ("version", 44) | ("sub", 47) | ("final", 54) | ("blend", 64) => true,
            _ => false,
        }
    };
    world.db.entries.retain(|k, _| !wanted(k));
    let mut n = 0;
    for (k, v) in &other.db.entries {
        if wanted(k) {
            world.db.entries.insert(k.clone(), v.clone());
            n += 1;
        }
    }
    // Mixed Data3D records: one half from each world.
    if what == "d3hm" || what == "d3bio" {
        let key = [0u8; 8].iter().copied().chain([43]).collect::<Vec<u8>>();
        let (g, m) = (world.db.entries.get(&key).cloned().unwrap(), other.db.entries.get(&key).cloned().unwrap());
        let mixed: Vec<u8> = if what == "d3hm" { [&m[..512], &g[512..]].concat() } else { [&g[..512], &m[512..]].concat() };
        world.db.entries.insert(key, mixed);
        n += 1;
    }
    // `d3id:<n>`: my Data3D with every biome set to id n (and the good heightmap kept out).
    if let Some(id) = what.strip_prefix("d3id:") {
        let id: i32 = id.parse().unwrap();
        let key = [0u8; 8].iter().copied().chain([43]).collect::<Vec<u8>>();
        let mut d = mctk_bedrock::Data3D::parse(other.db.entries.get(&key).unwrap()).unwrap();
        for b in &mut d.biomes {
            if let mctk_bedrock::BiomeStorage::Palettized { palette, .. } = b {
                palette.iter_mut().for_each(|p| *p = id);
            }
        }
        world.db.entries.insert(key, d.encode());
        n += 1;
    }
    // `d3expand`: my Data3D with every CopyPrevious written out in full.
    if what == "d3expand" {
        let key = [0u8; 8].iter().copied().chain([43]).collect::<Vec<u8>>();
        let mut d = mctk_bedrock::Data3D::parse(other.db.entries.get(&key).unwrap()).unwrap();
        let mut last = None;
        for b in &mut d.biomes {
            match b {
                mctk_bedrock::BiomeStorage::CopyPrevious => *b = last.clone().unwrap(),
                other => last = Some(other.clone()),
            }
        }
        world.db.entries.insert(key, d.encode());
        n += 1;
    }
    println!("copied {n} records");
    world.save_to_folder(out)?;
    Ok(())
}
