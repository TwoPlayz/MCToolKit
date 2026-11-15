fn main() {
    let w = mctk_bedrock::BedrockWorld::open(std::env::args().nth(1).unwrap()).unwrap();
    let mut other = std::collections::BTreeMap::new();
    for (k, v) in &w.db.entries {
        if let mctk_bedrock::Key::Other(name) = mctk_bedrock::Key::decode(k) {
            other.insert(String::from_utf8_lossy(&name).to_string(), v.len());
        }
    }
    for (k, n) in other { println!("{k}  {n} bytes"); }
}
