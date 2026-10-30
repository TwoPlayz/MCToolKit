//! Probes a console save archive: checks the archive round trip and decodes every chunk
//! (codec + RLE + NBT), printing one chunk's layout. Read-only.
//!
//! cargo run --release -p mctk-console --example probe -- <GAMEDATA or decompressed archive>

use std::collections::BTreeMap;

use mctk_console::{ConsoleRegion, SaveArchive};
use mctk_nbt::{Flavor, Tag};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: probe <archive>")?;
    let bytes = std::fs::read(&path)?;
    let archive = SaveArchive::parse(&bytes)?;
    println!(
        "archive v{}/{}: {} files, re-encode {}",
        archive.original_version,
        archive.version,
        archive.entries.len(),
        if archive.encode()? == bytes {
            "identical"
        } else {
            "DIFFERENT"
        }
    );

    let (mut ok, mut failed, mut nbt_bad) = (0, 0, 0);
    let mut codecs = BTreeMap::new();
    let mut shown = false;
    for (dim, rx, rz, entry) in archive.regions() {
        let region = ConsoleRegion::parse(entry.data.clone())?;
        for (lx, lz) in region.chunks().collect::<Vec<_>>() {
            let raw = region.raw_chunk(lx, lz)?.expect("listed");
            *codecs.entry(format!("{:?}", raw.codec())).or_insert(0) += 1;
            match raw.decompress() {
                Ok(chunk) => {
                    let tag = mctk_nbt::from_bytes(&chunk, Flavor::Java)?;
                    if mctk_nbt::to_bytes(&tag, Flavor::Java)? != chunk {
                        nbt_bad += 1;
                    }
                    ok += 1;
                    if !shown {
                        shown = true;
                        println!(
                            "dim {dim} region ({rx},{rz}) chunk ({lx},{lz}): {} bytes",
                            chunk.len()
                        );
                        if let Some(level) = tag.tag.get("Level").and_then(Tag::as_compound) {
                            for (k, v) in level {
                                let desc = match v {
                                    Tag::ByteArray(a) => format!("ByteArray[{}]", a.len()),
                                    Tag::IntArray(a) => format!("IntArray[{}]", a.len()),
                                    Tag::List(l) => {
                                        format!("List<{:?}>[{}]", l.element_type(), l.len())
                                    }
                                    other => format!("{other}"),
                                };
                                println!("    {k}: {}", &desc[..desc.len().min(90)]);
                            }
                        }
                    }
                }
                Err(e) => {
                    failed += 1;
                    if failed <= 3 {
                        println!("FAILED dim {dim} region ({rx},{rz}) chunk ({lx},{lz}): {e}");
                    }
                }
            }
        }
    }
    println!(
        "codecs {codecs:?}; chunks decoded: {ok}, failed: {failed}, NBT round-trip mismatches: {nbt_bad}"
    );
    Ok(())
}
