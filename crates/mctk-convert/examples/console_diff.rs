//! Compares two console saves: the archive's file list, level.dat, and the chunk records' shape.
//!
//! cargo run --release -p mctk-convert --example console_diff -- <save a> <save b> <chunk x> <chunk z>
use mctk_console::{ConsoleChunk, ConsoleRegion, ConsoleSave};
use mctk_nbt::Tag;

fn describe(t: &Tag) -> String {
    match t {
        Tag::ByteArray(a) => format!("ByteArray[{}]", a.len()),
        Tag::List(l) => format!("List[{}]", l.len()),
        Tag::Compound(c) => format!("Compound{{{}}}", c.keys().cloned().collect::<Vec<_>>().join(",")),
        other => format!("{other}"),
    }
}

fn level(save: &ConsoleSave) -> mctk_nbt::Compound {
    let e = save.archive.get("level.dat").unwrap();
    let (_, raw) = mctk_nbt::decompress(&e.data).unwrap();
    let root = mctk_nbt::from_bytes(&raw, mctk_nbt::Flavor::Java).unwrap();
    root.tag.get("Data").and_then(Tag::as_compound).cloned().unwrap()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let (x, y) = (ConsoleSave::open(&a[1])?, ConsoleSave::open(&a[2])?);
    let names = |s: &ConsoleSave| s.archive.entries.iter().map(|e| format!("{}({})", e.name, e.data.len())).collect::<Vec<_>>();
    println!("A files: {:?}\nB files: {:?}", names(&x), names(&y));
    println!("versions {}/{} vs {}/{}", x.archive.original_version, x.archive.version, y.archive.original_version, y.archive.version);
    for (n, s) in [("A", &x), ("B", &y)] {
        let d = &s.archive.get("level.dat").unwrap().data;
        println!("{n} level.dat head {:02x?} len {}", &d[..16.min(d.len())], d.len());
    }
    let (la, lb) = (level(&x), level(&y));
    for (k, v) in &la {
        match lb.get(k) {
            None => println!("level only in A: {k} = {v}"),
            Some(w) if w != v => println!("level differs {k}: A {v} B {w}"),
            _ => {}
        }
    }
    for (k, v) in &lb {
        if !la.contains_key(k) {
            println!("level only in B: {k} = {v}");
        }
    }
    let (cx, cz): (i32, i32) = (a[3].parse()?, a[4].parse()?);
    for (label, save) in [("A", &x), ("B", &y)] {
        for (_, _, _, entry) in save.archive.regions() {
            let region = ConsoleRegion::parse(entry.data.clone())?;
            if let Some(raw) = region.raw_chunk(cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize)? {
                let tree = raw.nbt()?;
                let c = ConsoleChunk::from_nbt(tree.clone())?;
                if (c.x, c.z) != (cx, cz) {
                    continue;
                }
                println!("{label} chunk {cx},{cz}: rle {} decompressed {} payload {}", raw.rle, raw.decompressed_len, raw.payload.len());
                for (k, v) in tree.tag.get("Level").and_then(Tag::as_compound).unwrap() {
                    println!("   {k}: {}", describe(v).chars().take(160).collect::<String>());
                }
            }
        }
    }
    Ok(())
}
