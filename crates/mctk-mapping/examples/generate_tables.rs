//! Regenerates `data/mappings.nbt.gz` from public sources (see `data/NOTICE.md`):
//!
//! - GeyserMC `Blocks.java` (MIT): Java blocks in registry order.
//! - misode/mcmeta `<version>-summary/blocks/data.json`: each block's property values and
//!   default state, from Mojang's data generator.
//! - GeyserMC mappings `blocks.nbt` and `biomes.json` (MIT): the Bedrock block state for
//!   every Java state id, and Bedrock biome ids.
//!
//! Java state ids are rebuilt the way the game assigns them: blocks in registry order, and
//! within a block every combination of its properties (sorted by name, values in declared
//! order, last property varying fastest).
//!
//! cargo run -p mctk-mapping --example generate_tables -- <Blocks.java> <blocks data.json> <blocks.nbt> <biomes.json> <out.nbt.gz>

use std::io::Write;

use mctk_nbt::{Compound, Flavor, List, NamedTag, Tag, TagType};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn registry_order(blocks_java: &str) -> Vec<String> {
    blocks_java
        .lines()
        .filter(|l| l.contains("= register(new"))
        .filter_map(|l| {
            let start = l.find("(\"")? + 2;
            let end = start + l[start..].find('"')?;
            Some(format!("minecraft:{}", &l[start..end]))
        })
        .collect()
}

struct JavaState {
    name: String,
    properties: Vec<(String, String)>,
    /// Properties differing from the block's default state.
    rank: u8,
}

fn enumerate(name: &str, summary: &serde_json::Value) -> Res<Vec<JavaState>> {
    let short = name.trim_start_matches("minecraft:");
    let entry = summary
        .get(short)
        .ok_or_else(|| format!("{name} missing from the block summary"))?;
    let props = entry[0].as_object().ok_or("bad summary entry")?;
    let defaults = entry[1].as_object().ok_or("bad summary entry")?;
    let mut keys: Vec<&String> = props.keys().collect();
    keys.sort();
    let mut states: Vec<Vec<(String, String)>> = vec![Vec::new()];
    for key in keys {
        let values = props[key].as_array().ok_or("bad property values")?;
        states = states
            .into_iter()
            .flat_map(|s| {
                values.iter().map(move |v| {
                    let mut s = s.clone();
                    s.push((key.clone(), v.as_str().unwrap_or_default().to_string()));
                    s
                })
            })
            .collect();
    }
    Ok(states
        .into_iter()
        .map(|properties| {
            let rank = properties
                .iter()
                .filter(|(k, v)| defaults.get(k).and_then(|d| d.as_str()) != Some(v))
                .count() as u8;
            JavaState {
                name: name.to_string(),
                properties,
                rank,
            }
        })
        .collect())
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        return Err(
            "usage: <Blocks.java> <blocks data.json> <blocks.nbt> <biomes.json> <out.nbt.gz>"
                .into(),
        );
    }
    let order = registry_order(&std::fs::read_to_string(&args[1])?);
    let summary: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&args[2])?)?;
    let (_, geyser) = mctk_nbt::read_file(&args[3], Flavor::Java)?;
    let biomes: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&args[4])?)?;

    let mut java = Vec::new();
    for name in &order {
        java.extend(enumerate(name, &summary)?);
    }
    let bedrock = geyser
        .tag
        .get("bedrock_mappings")
        .and_then(Tag::as_list)
        .ok_or("blocks.nbt without bedrock_mappings")?;
    println!(
        "{} Java blocks, {} Java states, {} Bedrock mappings",
        order.len(),
        java.len(),
        bedrock.len()
    );
    if java.len() != bedrock.len() {
        return Err("state counts differ: the sources are for different versions".into());
    }

    // Alignment check: logs carry `axis` in Java and `pillar_axis` in Bedrock.
    let (mut checked, mut wrong) = (0, 0);
    let mut out = Vec::with_capacity(java.len());
    for (j, b) in java.iter().zip(bedrock.iter()) {
        let b = b.as_compound().ok_or("bad Bedrock mapping")?;
        let b_name = match b.get("bedrock_identifier").and_then(Tag::as_str) {
            Some(n) if n.contains(':') => n.to_string(),
            Some(n) => format!("minecraft:{n}"),
            None => j.name.clone(),
        };
        let states = b
            .get("state")
            .and_then(Tag::as_compound)
            .cloned()
            .unwrap_or_default();
        if let Some((_, axis)) = j.properties.iter().find(|(k, _)| k == "axis") {
            if let Some(Tag::String(p)) = states.get("pillar_axis") {
                checked += 1;
                wrong += (p != axis) as usize;
            }
        }
        let mut e = Compound::new();
        e.insert("j".into(), Tag::String(j.name.clone()));
        if !j.properties.is_empty() {
            let p: Compound = j
                .properties
                .iter()
                .map(|(k, v)| (k.clone(), Tag::String(v.clone())))
                .collect();
            e.insert("p".into(), Tag::Compound(p));
        }
        if j.rank > 0 {
            e.insert("r".into(), Tag::Byte(j.rank as i8));
        }
        if b_name != j.name {
            e.insert("b".into(), Tag::String(b_name));
        }
        if !states.is_empty() {
            e.insert("s".into(), Tag::Compound(states));
        }
        out.push(Tag::Compound(e));
    }
    println!("alignment check: {checked} axis/pillar_axis pairs, {wrong} disagree");
    if checked == 0 || wrong > 0 {
        return Err("Java state order does not line up with the Bedrock mappings".into());
    }

    let mut biome_list = Vec::new();
    for (name, v) in biomes.as_object().ok_or("bad biomes.json")? {
        let id = v["bedrock_id"].as_i64().ok_or("biome without bedrock_id")?;
        let mut e = Compound::new();
        e.insert("j".into(), Tag::String(name.clone()));
        e.insert("b".into(), Tag::Int(id as i32));
        biome_list.push(Tag::Compound(e));
    }
    println!("{} biomes", biome_list.len());

    let mut root = Compound::new();
    root.insert("java_version".into(), Tag::String("26.3".into()));
    root.insert("java_data_version".into(), Tag::Int(5023));
    root.insert("bedrock_version".into(), Tag::String("1.26.60".into()));
    root.insert(
        "blocks".into(),
        Tag::List(List::from_tags(TagType::Compound, out)?),
    );
    root.insert(
        "biomes".into(),
        Tag::List(List::from_tags(TagType::Compound, biome_list)?),
    );
    let bytes = mctk_nbt::to_bytes(
        &NamedTag {
            name: String::new(),
            tag: Tag::Compound(root),
        },
        Flavor::Java,
    )?;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(&bytes)?;
    let gz = gz.finish()?;
    std::fs::write(&args[5], &gz)?;
    println!("wrote {} ({} bytes)", args[5], gz.len());
    Ok(())
}
