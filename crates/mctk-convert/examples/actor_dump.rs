//! Prints every actor of a Bedrock world: identifier and (with a filter) the whole tag.
//! cargo run --release -p mctk-convert --example actor_dump -- <bedrock world> [identifier-substring]
use mctk_bedrock::{BedrockWorld, Key};
use mctk_nbt::{Flavor, Tag};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let bw = BedrockWorld::open(args.next().ok_or("world")?)?;
    let filter = args.next();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for (k, v) in &bw.db.entries {
        if !matches!(Key::decode(k), Key::Actor(_)) {
            continue;
        }
        let Ok(root) = mctk_nbt::from_bytes(v, Flavor::Bedrock) else { continue };
        let id = root.tag.get("identifier").and_then(Tag::as_str).unwrap_or("?").to_string();
        *counts.entry(id.clone()).or_default() += 1;
        if filter.as_deref() == Some("enchants") {
            for key in ["Mainhand", "Armor"] {
                for item in root.tag.get(key).and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()) {
                    let name = item.get("Name").and_then(Tag::as_str).unwrap_or("");
                    let ench = item.get("tag").and_then(|t| t.get("ench")).and_then(Tag::as_list);
                    for e in ench.into_iter().flat_map(|l| l.iter()) {
                        println!("ENCH {name} {} {:?}", e.get("id").and_then(Tag::as_i64).unwrap_or(-1), root.tag.get("Pos").map(|p| p.to_string()));
                    }
                }
            }
        } else if filter.as_deref().is_some_and(|f| id.contains(f)) {
            println!("{:?}", root.tag);
        }
    }
    for (id, n) in counts {
        println!("{n:>5} {id}");
    }
    Ok(())
}


