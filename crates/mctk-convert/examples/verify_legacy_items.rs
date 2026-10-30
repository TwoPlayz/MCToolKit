//! Converts a pre-1.13 Java world to Bedrock and compares every container's items with the
//! source (numeric id + damage -> Bedrock name and count), and counts the block entities
//! and entities in the result.
//!
//! cargo run --release -p mctk-convert --example verify_legacy_items -- <legacy java world> <out folder>

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use mctk_anvil::Region;
use mctk_bedrock::{BedrockWorld, ChunkTag, Key};
use mctk_java::JavaWorld;
use mctk_mapping::{item_mappings, legacy::legacy};
use mctk_nbt::{Compound, Flavor, Tag};

type Res<T> = Result<T, Box<dyn std::error::Error>>;
type Stacks = BTreeMap<(String, i64), usize>;

fn items_of(list: Option<&Tag>, java: bool, bad: &mut Vec<String>) -> Stacks {
    let mut out = Stacks::new();
    for i in list.and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()) {
        let Some(i) = i.as_compound() else { continue };
        if java {
            let id = i.get("id").and_then(Tag::as_i64).unwrap_or(0) as u16;
            let damage = i.get("Damage").and_then(Tag::as_i64).unwrap_or(0) as u16;
            let count = i.get("Count").and_then(Tag::as_i64).unwrap_or(1);
            let Some(name) = legacy().item(id, damage) else {
                bad.push(format!("legacy item {id}:{damage} unknown"));
                continue;
            };
            match item_mappings().java_to_bedrock(name) {
                Some((b, _)) => *out.entry((b.to_string(), count)).or_default() += 1,
                None => bad.push(format!("{name} has no Bedrock item")),
            }
        } else {
            let name = i.get("Name").and_then(Tag::as_str).unwrap_or("?").to_string();
            let count = i.get("Count").and_then(Tag::as_i64).unwrap_or(0);
            *out.entry((name, count)).or_default() += 1;
        }
    }
    out
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let (src, out) = (
        Path::new(args.get(1).ok_or("usage: <legacy java world> <out>")?),
        Path::new(args.get(2).ok_or("usage: <legacy java world> <out>")?),
    );
    let report = mctk_convert::java_to_bedrock(src, out)?;
    println!(
        "block entities {:?}\nentities {:?}\nskipped {:?}",
        report.converted_block_entities, report.converted_entities, report.skipped
    );

    let world = BedrockWorld::open(out)?;
    let mut bedrock: HashMap<(i32, i32, i32, i32), Compound> = HashMap::new();
    let mut ids: BTreeMap<String, usize> = BTreeMap::new();
    for (k, v) in &world.db.entries {
        let Key::Chunk(c) = Key::decode(k) else { continue };
        if c.tag != ChunkTag::BlockEntity {
            continue;
        }
        let mut s = v.as_slice();
        while let Ok(root) = mctk_nbt::read_root(&mut s, Flavor::Bedrock) {
            if let Tag::Compound(be) = root.tag {
                *ids.entry(be.get("id").and_then(Tag::as_str).unwrap_or("?").into()).or_default() += 1;
                let g = |k: &str| be.get(k).and_then(Tag::as_i64).unwrap_or(0) as i32;
                bedrock.insert((c.dimension.id(), g("x"), g("y"), g("z")), be);
            }
        }
    }
    println!("Bedrock block entities: {ids:?}");

    let java = JavaWorld::open(src)?;
    let (mut containers, mut bad_containers, mut stacks) = (0, 0, 0usize);
    let mut problems: Vec<String> = Vec::new();
    let mut spawners = BTreeMap::new();
    for (id, bdim) in [("minecraft:overworld", 0), ("minecraft:the_nether", 1), ("minecraft:the_end", 2)] {
        let Some(dim) = java.dimension(id) else { continue };
        for rpos in dim.regions()? {
            let region = Region::open(dim.region_dir().join(rpos.file_name()))?;
            for (lx, lz) in region.chunks().collect::<Vec<_>>() {
                let root = region.read_chunk(lx, lz)?.unwrap();
                let Some(level) = root.tag.get("Level") else { continue };
                for t in level.get("TileEntities").and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()) {
                    let at = |k: &str| t.get(k).and_then(Tag::as_i64).unwrap_or(0) as i32;
                    let key = (bdim, at("x"), at("y"), at("z"));
                    let Some(b) = bedrock.get(&key) else {
                        problems.push(format!("{} at {key:?} missing in Bedrock", t.get("id").and_then(Tag::as_str).unwrap_or("?")));
                        continue;
                    };
                    if t.get("Items").is_some() {
                        containers += 1;
                        let mut bad = Vec::new();
                        let want = items_of(t.get("Items"), true, &mut bad);
                        let got = items_of(b.get("Items"), false, &mut Vec::new());
                        stacks += want.values().sum::<usize>();
                        problems.extend(bad);
                        if want != got {
                            bad_containers += 1;
                            problems.push(format!("items differ at {key:?}: {want:?} vs {got:?}"));
                        }
                    }
                    if t.get("id").and_then(Tag::as_str) == Some("MobSpawner") {
                        let e = t.get("EntityId").and_then(Tag::as_str).unwrap_or("?").to_string();
                        let got = b.get("EntityIdentifier").and_then(Tag::as_str).unwrap_or("-").to_string();
                        *spawners.entry((e, got)).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    println!("{containers} containers, {stacks} stacks checked: {bad_containers} containers differ");
    println!("spawners (legacy entity, Bedrock entity): {spawners:?}");
    for p in problems.iter().take(15) {
        println!("  {p}");
    }
    if bad_containers > 0 || !problems.is_empty() {
        return Err("legacy block entity mismatch".into());
    }
    Ok(())
}
