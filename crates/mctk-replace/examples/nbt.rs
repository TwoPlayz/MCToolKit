//! `cargo run -p mctk-replace --example nbt -- <file.dat> [Key.Path]`
//! Prints a (gzip) Java NBT file as SNBT, optionally only the tag at a dotted path.

use mctk_nbt::{Flavor, Tag};

fn main() {
    let mut args = std::env::args().skip(1);
    let file = args.next().expect("usage: nbt <file> [path]");
    let (_, root) = mctk_nbt::read_file(&file, Flavor::Java).expect("read nbt");
    let mut tag: &Tag = &root.tag;
    if let Some(path) = args.next() {
        for part in path.split('.') {
            tag = tag.get(part).unwrap_or_else(|| panic!("no {part}"));
        }
    }
    println!("{tag}");
}
