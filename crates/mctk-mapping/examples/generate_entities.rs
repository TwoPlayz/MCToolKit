//! Regenerates `data/entities.tsv`: the `definitions` a Bedrock actor starts with.
//!
//! A Bedrock actor on disk records which of its entity type's component groups are active
//! (`definitions`, e.g. `+minecraft:rabbit`, `+adult`). The game picks them when the
//! entity spawns, by running the `minecraft:entity_spawned` event of the vanilla behaviour
//! pack. This tool resolves that event for every entity type, the way a freshly spawned
//! adult (and, where the type has one, baby) would come out:
//!
//! - `randomize` takes the option that fits (baby or adult) and otherwise the heaviest;
//! - `filters` that test the environment (biome, light, snow, ...) are not met, and
//!   "has no component" tests are met;
//! - `trigger` follows the named event.
//!
//! Only facts are kept (identifier and component group names), see `data/NOTICE.md`.
//!
//! cargo run -p mctk-mapping --example generate_entities -- <behavior_pack/entities dir> <out entities.tsv>

use serde_json::Value;

/// JSON with `//` comments and trailing commas, as Mojang writes it.
fn parse_lenient(text: &str) -> Result<Value, serde_json::Error> {
    let text = text.trim_start_matches('\u{feff}');
    let mut out = String::with_capacity(text.len());
    let (mut in_str, mut chars) = (false, text.chars().peekable());
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            for n in chars.by_ref() {
                if n == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    // Trailing commas before a closing bracket.
    let bytes: Vec<char> = out.chars().collect();
    let mut cleaned = String::with_capacity(bytes.len());
    let mut in_str = false;
    for (i, &c) in bytes.iter().enumerate() {
        if in_str {
            cleaned.push(c);
            if c == '"' && bytes[i - 1] != '\\' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
        }
        if c == ',' {
            let next = bytes[i + 1..].iter().find(|n| !n.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        cleaned.push(c);
    }
    serde_json::from_str(&cleaned)
}

fn filter_ok(f: &Value) -> bool {
    match f {
        Value::Array(a) => a.iter().all(filter_ok),
        Value::Object(o) => {
            if let Some(a) = o.get("all_of").and_then(Value::as_array) {
                return a.iter().all(filter_ok);
            }
            if let Some(a) = o.get("any_of").and_then(Value::as_array) {
                return a.iter().any(filter_ok);
            }
            let test = o.get("test").and_then(Value::as_str).unwrap_or("");
            let op = o.get("operator").and_then(Value::as_str).unwrap_or("==");
            // "does not have this component/property" holds for a fresh entity.
            matches!(test, "has_component" | "has_property" | "has_tag")
                && matches!(op, "!=" | "not")
        }
        _ => false,
    }
}

struct Ctx<'a> {
    events: &'a serde_json::Map<String, Value>,
    baby: bool,
}

fn resolve(node: &Value, ctx: &Ctx, groups: &mut Vec<String>, depth: usize) {
    if depth > 8 {
        return;
    }
    let Some(o) = node.as_object() else { return };
    if let Some(f) = o.get("filters") {
        if !filter_ok(f) {
            return;
        }
    }
    if let Some(seq) = o.get("sequence").and_then(Value::as_array) {
        for n in seq {
            resolve(n, ctx, groups, depth + 1);
        }
    }
    if let Some(opts) = o.get("randomize").and_then(Value::as_array) {
        // Resolve each option on its own, then pick the one that fits.
        let resolved: Vec<(f64, Vec<String>)> = opts
            .iter()
            .map(|opt| {
                let mut g = groups.clone();
                resolve(opt, ctx, &mut g, depth + 1);
                let weight = opt.get("weight").and_then(Value::as_f64).unwrap_or(1.0);
                (weight, g)
            })
            .collect();
        let is_baby = |g: &Vec<String>| g.iter().any(|n| n.contains("baby"));
        let pick = resolved
            .iter()
            .filter(|(_, g)| is_baby(g) == ctx.baby)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .or_else(|| resolved.iter().max_by(|a, b| a.0.total_cmp(&b.0)));
        if let Some((_, g)) = pick {
            *groups = g.clone();
        }
    }
    if let Some(add) = o.get("add").and_then(|a| a.get("component_groups")) {
        for g in add.as_array().into_iter().flatten().filter_map(Value::as_str) {
            if !groups.iter().any(|x| x == g) {
                groups.push(g.to_string());
            }
        }
    }
    if let Some(rem) = o.get("remove").and_then(|a| a.get("component_groups")) {
        for g in rem.as_array().into_iter().flatten().filter_map(Value::as_str) {
            groups.retain(|x| x != g);
        }
    }
    if let Some(t) = o.get("trigger") {
        let name = t
            .as_str()
            .or_else(|| t.get("event").and_then(Value::as_str));
        if let Some(ev) = name.and_then(|n| ctx.events.get(n)) {
            resolve(ev, ctx, groups, depth + 1);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (dir, out) = (
        args.get(1).ok_or("usage: <entities dir> <out>")?,
        args.get(2).ok_or("usage: <entities dir> <out>")?,
    );
    let mut lines = Vec::new();
    for f in std::fs::read_dir(dir)? {
        let path = f?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let json = match parse_lenient(&std::fs::read_to_string(&path)?) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("skipping {}: {e}", path.display());
                continue;
            }
        };
        let Some(entity) = json.get("minecraft:entity") else {
            continue;
        };
        let Some(id) = entity
            .pointer("/description/identifier")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let empty = serde_json::Map::new();
        let events = entity
            .get("events")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let has_baby_group = entity
            .get("component_groups")
            .and_then(Value::as_object)
            .is_some_and(|g| g.keys().any(|k| k.contains("baby")));
        let run = |baby: bool| {
            let mut groups = Vec::new();
            if let Some(ev) = events.get("minecraft:entity_spawned") {
                resolve(ev, &Ctx { events, baby }, &mut groups, 0);
            }
            groups.join(",")
        };
        let adult = run(false);
        let baby = if has_baby_group { run(true) } else { String::new() };
        lines.push(format!("{id}\t{adult}\t{baby}"));
    }
    lines.sort();
    std::fs::write(out, lines.join("\n") + "\n")?;
    println!("{} entity types", lines.len());
    Ok(())
}
