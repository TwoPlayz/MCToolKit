use mctk_bedrock::BedrockWorld;
use mctk_nbt::{Flavor, Tag};
fn main() {
    let w = BedrockWorld::open(std::env::args().nth(1).unwrap()).unwrap();
    let v = w.db.entries.iter().find(|(k, _)| k.as_slice() == b"~local_player").unwrap().1;
    let root = mctk_nbt::from_bytes(v, Flavor::Bedrock).unwrap();
    for (k, t) in root.tag.as_compound().unwrap() {
        let s = format!("{t:?}");
        println!("{k}: {}", &s[..s.len().min(if k == "Inventory" || k == "EnderChestInventory" { 0 } else if k == "abilities" { 2000 } else { 170 })]);
    }
    if let Some(Tag::List(l)) = root.tag.get("Inventory") { println!("Inventory len {} first {:?}", l.len(), l.get(0)); }
}

