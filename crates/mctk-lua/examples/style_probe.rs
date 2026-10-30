//! Runs NBT_EDITOR_STYLE extensions over an NBT file and prints the labels they produce.
//!
//! cargo run -p mctk-lua --example style_probe -- <extensions dir> <nbt file | console save> [entry]

use std::path::Path;

use mctk_lua::style::{self, PathSeg};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let dir = Path::new(&args[1]);
    let exts: Vec<_> = mctk_lua::list(dir)
        .into_iter()
        .flatten()
        .filter(|e| {
            e.manifest
                .extension_types
                .iter()
                .any(|t| t == "NBT_EDITOR_STYLE")
        })
        .collect();
    println!("{} style extensions", exts.len());
    let (root, ed, ft) = if let Some(entry) = args.get(3) {
        let save = mctk_console::ConsoleSave::open(&args[2])?;
        let e = save.archive.get(entry).ok_or("no such entry")?;
        let (_, raw) = mctk_nbt::decompress(&e.data)?;
        (
            mctk_nbt::from_bytes(&raw, mctk_nbt::Flavor::Java)?,
            style::edition::CONSOLE,
            style::file_type::PLAYER,
        )
    } else {
        let (_, r) = mctk_nbt::read_file(&args[2], mctk_nbt::Flavor::Java)?;
        (r, style::edition::JAVA, style::file_type::LEVEL)
    };
    let (styles, errors) = style::style_tree(&exts, &root.tag, ed, ft);
    for e in &errors {
        println!("ERROR {e}");
    }
    let mut rows: Vec<_> = styles.iter().collect();
    rows.sort_by_key(|(p, _)| format!("{p:?}"));
    for (path, s) in rows {
        let name: Vec<String> = path
            .iter()
            .map(|p| match p {
                PathSeg::Key(k) => k.clone(),
                PathSeg::Index(i) => i.to_string(),
            })
            .collect();
        println!(
            "{:40} {:?} {:?} {:?}",
            name.join("/"),
            s.label,
            s.color,
            s.icon
        );
    }
    Ok(())
}
