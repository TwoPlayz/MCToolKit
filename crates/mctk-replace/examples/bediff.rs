//! `cargo run -p mctk-replace --example bediff -- <folder-a> <folder-b> <chunk-x> <chunk-z>`
//! Compares the block entities (`region/` folders) or entities (`entities/` folders) of one
//! chunk in two worlds. Elements are matched by position or UUID; fields that change as the
//! game runs (age, motion, ...) are ignored. Prints every difference by path.

use mctk_anvil::{ChunkPos, Region};
use mctk_nbt::Tag;

const VOLATILE: [&str; 24] = [
    "Age", "Motion", "Pos", "Fire", "Air", "fall_distance", "FallDistance", "TickCount",
    "ticks_since_song_started", "lit_time_remaining", "lit_total_time", "cooking_time_spent",
    "cooking_total_time", "BurnTime", "CookTime", "CookTimeTotal", "RecipesUsed", "Rotation",
    "OnGround", "HurtTime", "current_impulse_context_reset_grace_time", "keepPacked", "LastUpdate",
    "RecordStartTick",
];

fn load(dir: &str, cx: i32, cz: i32) -> Vec<Tag> {
    let pos = ChunkPos { x: cx, z: cz };
    let region = Region::open(std::path::Path::new(dir).join(pos.region().file_name())).expect("region");
    let (lx, lz) = pos.local();
    let nbt = region.read_chunk(lx, lz).expect("read").expect("chunk absent");
    let key = if dir.ends_with("entities") { "Entities" } else { "block_entities" };
    match nbt.tag.get(key) {
        Some(Tag::List(l)) => l.iter().cloned().collect(),
        _ => vec![],
    }
}

fn key_of(t: &Tag) -> String {
    if let Some(Tag::IntArray(u)) = t.get("UUID") {
        return format!("{u:?}");
    }
    format!(
        "{} {:?} {:?} {:?}",
        t.get("id").map(|i| i.to_string()).unwrap_or_default(),
        t.get("x").and_then(Tag::as_i64),
        t.get("y").and_then(Tag::as_i64),
        t.get("z").and_then(Tag::as_i64)
    )
}

fn short(t: &Tag) -> String {
    let s = t.to_string();
    if s.len() > 150 { format!("{}...", &s[..150]) } else { s }
}

fn diff(path: &str, a: &Tag, b: &Tag, out: &mut Vec<String>) {
    match (a, b) {
        (Tag::Compound(x), Tag::Compound(y)) => {
            for (k, va) in x {
                if VOLATILE.contains(&k.as_str()) {
                    continue;
                }
                match y.get(k) {
                    Some(vb) => diff(&format!("{path}.{k}"), va, vb, out),
                    None => out.push(format!("{path}.{k}: only in A = {}", short(va))),
                }
            }
            for (k, vb) in y {
                if !VOLATILE.contains(&k.as_str()) && !x.contains_key(k) {
                    out.push(format!("{path}.{k}: only in B = {}", short(vb)));
                }
            }
        }
        (Tag::List(x), Tag::List(y)) if x.len() == y.len() => {
            for (i, (va, vb)) in x.iter().zip(y.iter()).enumerate() {
                diff(&format!("{path}[{i}]"), va, vb, out);
            }
        }
        _ => {
            if a != b {
                out.push(format!("{path}: A = {}  B = {}", short(a), short(b)));
            }
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (da, db) = (&a[0], &a[1]);
    let (cx, cz) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let (la, lb) = (load(da, cx, cz), load(db, cx, cz));
    let (mut matched, mut total_diffs) = (0, 0);
    for ea in &la {
        let k = key_of(ea);
        let Some(eb) = lb.iter().find(|e| key_of(e) == k) else {
            println!("ONLY IN A: {k}");
            continue;
        };
        matched += 1;
        let mut d = Vec::new();
        diff("", ea, eb, &mut d);
        if !d.is_empty() {
            println!("== {k}");
            for line in &d {
                println!("   {line}");
            }
            total_diffs += d.len();
        }
    }
    for eb in &lb {
        if !la.iter().any(|e| key_of(e) == key_of(eb)) {
            println!("ONLY IN B: {}", key_of(eb));
        }
    }
    println!("{} in A, {} in B, {matched} matched, {total_diffs} differing fields", la.len(), lb.len());
}
