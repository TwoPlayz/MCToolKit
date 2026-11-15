//! Regenerates registry data files from the games' own data.
//!
//! `mctk-gen-java <version> <data-version> <generated-dir> <out.tsv>` where `<generated-dir>` is the
//! `--output` directory of the Java data generator run with `--reports` (and `--server`
//! for biomes).

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("mctk-gen: {e}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args {
        [version, data_version, dir, out] => {
            let dir = Path::new(dir);
            let read = |p: &str| {
                std::fs::read_to_string(dir.join(p)).map_err(|e| format!("{p}: {e}"))
            };
            let blocks = read("reports/blocks.json")?;
            // 1.13 has no registry report; its biomes and entities then come up empty.
            let regs = read("reports/registries.json").unwrap_or_else(|_| "{}".to_string());
            // Biomes are data files from 1.16.2 (and a report folder in 1.16.0 - 1.16.1); before
            // that they are a registry, which the report below carries.
            let mut biomes: Vec<String> = Vec::new();
            for sub in ["data/minecraft/worldgen/biome", "reports/biomes"] {
                if let Ok(rd) = std::fs::read_dir(dir.join(sub)) {
                    biomes = rd
                        .filter_map(|e| e.ok())
                        .filter_map(|e| {
                            let name = e.file_name().to_string_lossy().into_owned();
                            name.strip_suffix(".json").map(|n| format!("minecraft:{n}"))
                        })
                        .collect();
                    break;
                }
            }
            biomes.sort();
            let mut reg = mctk_registry::java_reports::from_reports(version, &blocks, &regs, biomes)
                .map_err(|e| e.to_string())?;
            // From 1.19 the structures are data files; before, the registry read above has them.
            let structure_dir = dir.join("data/minecraft/worldgen/structure");
            if let Ok(rd) = std::fs::read_dir(&structure_dir) {
                let mut names: Vec<String> = rd
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(|n| format!("minecraft:{n}")))
                    .collect();
                names.sort();
                reg.structures = names;
            }
            reg.data_version = Some(data_version.parse().map_err(|_| "bad data version")?);
            std::fs::write(out, reg.to_text()).map_err(|e| format!("{out}: {e}"))?;
            println!(
                "{} blocks, {} items, {} entities, {} biomes",
                reg.blocks.len(),
                reg.items.len(),
                reg.entities.len(),
                reg.biomes.len()
            );
            println!("{} structures", reg.structures.len());
            Ok(())
        }
        _ => Err("usage: mctk-gen-java <version> <data-version> <generated-dir> <out.tsv>".into()),
    }
}
