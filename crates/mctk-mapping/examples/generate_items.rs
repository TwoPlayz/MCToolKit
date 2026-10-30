//! Regenerates `data/items.tsv` from GeyserMC's `items.json` (MIT, see `data/NOTICE.md`):
//! one line per Java item, `java name <TAB> Bedrock identifier <TAB> Bedrock data value`.
//!
//! cargo run -p mctk-mapping --example generate_items -- <items.json> <out items.tsv>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (src, out) = (
        args.get(1).ok_or("usage: <items.json> <out>")?,
        args.get(2).ok_or("usage: <items.json> <out>")?,
    );
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(src)?)?;
    let items = json.as_object().ok_or("items.json is not an object")?;
    let mut lines = Vec::new();
    for (java, v) in items {
        let bedrock = v
            .get("bedrock_identifier")
            .and_then(|b| b.as_str())
            .ok_or_else(|| format!("{java} has no bedrock_identifier"))?;
        let data = v.get("bedrock_data").and_then(|d| d.as_i64()).unwrap_or(0);
        lines.push(format!("{java}\t{bedrock}\t{data}"));
    }
    lines.sort();
    std::fs::write(out, lines.join("\n") + "\n")?;
    println!("{} items", lines.len());
    Ok(())
}
