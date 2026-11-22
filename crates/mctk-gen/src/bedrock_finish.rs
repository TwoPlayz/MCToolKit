//! Finishes the registry of an older Bedrock version: the blocks come from probing that version's
//! server; the biomes are its definition files; the entity names are the newest version's, kept
//! where the name still appears in the server's executable, and its items and block entities are
//! the newest version's; and the facts the world files need (the block
//! state version, network and storage versions, the game version) are read from what the server
//! itself wrote.

use std::path::Path;

use mctk_bedrock::BedrockWorld;
use mctk_nbt::Tag;
use mctk_registry::Registry;

pub fn finish(registry: &Path, world: &Path, exe_ids: &Path, block_version: i64, out: &Path, server: &Path) -> Result<(), String> {
    let mut reg = Registry::from_text(&std::fs::read_to_string(registry).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let latest = mctk_registry::builtin::bedrock(mctk_registry::builtin::bedrock_versions()[0]).ok_or("no newest registry")?;
    let present: std::collections::HashSet<String> = std::fs::read_to_string(exe_ids)
        .map_err(|e| e.to_string())?
        .lines()
        .map(str::to_string)
        .collect();
    reg.entities = latest.entities.iter().filter(|n| present.contains(*n)).cloned().collect();
    // Items are built into the server and cannot be listed; the newest version's are kept.
    reg.items = latest.items.clone();
    reg.block_entities = latest.block_entities.clone();
    // Biomes are definition files of the server, one per biome.
    // Newer servers no longer ship them as files; the newest version's list stands in.
    let mut biomes: Vec<String> = std::fs::read_dir(server.join("definitions/biomes"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".biome.json").map(|n| format!("minecraft:{n}")))
                .collect()
        })
        .unwrap_or_default();
    biomes.sort();
    reg.biomes = if biomes.is_empty() { latest.biomes.clone() } else { biomes };
    let w = BedrockWorld::open(world).map_err(|e| e.to_string())?;
    let root = &w.level.root.tag;
    let int = |k: &str| root.get(k).and_then(Tag::as_i64);
    reg.meta.insert("block_version".into(), block_version.to_string());
    if let Some(n) = int("NetworkVersion") {
        reg.meta.insert("network_version".into(), n.to_string());
    }
    reg.meta.insert("storage_version".into(), w.level.storage_version.to_string());
    if let Some(Tag::List(l)) = root.get("lastOpenedWithVersion") {
        let v: Vec<String> = l.iter().filter_map(|t| t.as_i64()).map(|n| n.to_string()).collect();
        reg.meta.insert("client_version".into(), v.join(","));
    }
    // The chunk version byte and the height/biome record the server itself writes (versions before
    // 1.18 keep `Data2D`, newer ones `Data3D`).
    let mut versions: std::collections::BTreeMap<u8, usize> = Default::default();
    let (mut data2d, mut data3d) = (0usize, 0usize);
    for key in w.db.entries.keys() {
        if let mctk_bedrock::Key::Chunk(c) = mctk_bedrock::Key::decode(key) {
            match c.tag {
                mctk_bedrock::ChunkTag::Version | mctk_bedrock::ChunkTag::LegacyVersion => {
                    if let Some(&b) = w.db.entries.get(key).and_then(|v| v.first()) {
                        *versions.entry(b).or_insert(0) += 1;
                    }
                }
                mctk_bedrock::ChunkTag::Data2D => data2d += 1,
                mctk_bedrock::ChunkTag::Data3D => data3d += 1,
                _ => {}
            }
        }
    }
    if let Some((v, _)) = versions.iter().max_by_key(|(_, n)| **n) {
        reg.meta.insert("chunk_version".into(), v.to_string());
    }
    reg.meta.insert("height_record".into(), if data3d >= data2d { "data3d" } else { "data2d" }.into());
    std::fs::write(out, reg.to_text()).map_err(|e| e.to_string())?;
    println!(
        "{}: {} blocks, {} items, {} entities, {} biomes; {:?}",
        reg.version,
        reg.blocks.len(),
        reg.items.len(),
        reg.entities.len(),
        reg.biomes.len(),
        reg.meta
    );
    Ok(())
}
