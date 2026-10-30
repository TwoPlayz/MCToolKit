use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use mctk_anvil::Region;
use mctk_nbt::Flavor;

#[derive(Parser)]
#[command(name = "mctoolkit", version, about = "Minecraft world toolkit")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect NBT files such as level.dat
    Nbt {
        #[command(subcommand)]
        action: NbtAction,
    },
    /// Inspect Java Edition region files (r.x.z.mca)
    Region {
        #[command(subcommand)]
        action: RegionAction,
    },
    /// Inspect a Java Edition world folder
    World {
        #[command(subcommand)]
        action: WorldAction,
    },
    /// Bedrock Edition worlds (folders or .mcworld files)
    Bedrock {
        #[command(subcommand)]
        action: BedrockAction,
    },
    /// Legacy Console Edition saves (PS3 save folders / GAMEDATA, Xbox 360 savegame.dat)
    Console {
        #[command(subcommand)]
        action: ConsoleAction,
    },
    /// Write a copy of a world without the selected chunks (or, with --invert, with only
    /// the selected chunks). Layers are block coordinates and combine as a union.
    Prune(PruneArgs),
    /// Convert a world between editions into a new folder
    Convert {
        #[command(subcommand)]
        action: ConvertAction,
    },
    /// Lua extensions
    Ext {
        /// Extensions folder (default: %APPDATA%\MCToolKit\extensions)
        #[arg(long, global = true)]
        dir: Option<PathBuf>,
        #[command(subcommand)]
        action: ExtAction,
    },
}

#[derive(Subcommand)]
enum ExtAction {
    /// List installed extensions
    List,
    /// Install an extension from a folder or a .zip
    Install { source: PathBuf },
    /// Remove an installed extension
    Remove { id: String },
    /// Run an installed extension (by id) or an extension folder
    Run {
        extension: String,
        /// World the extension may read
        #[arg(long)]
        world: Option<PathBuf>,
        /// Folder the extension may write into
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ConvertAction {
    /// Java 1.13+ world (best: 26.3) to a Bedrock 1.26.60 world folder
    JavaToBedrock { world: PathBuf, out: PathBuf },
    /// Bedrock world (folder or .mcworld) to a Java 26.3 world folder
    BedrockToJava { world: PathBuf, out: PathBuf },
    /// Legacy Console save to a Java world (upgraded by the game on first open)
    ConsoleToJava { save: PathBuf, out: PathBuf },
}

fn print_edition_report(report: &mctk_convert::EditionReport, out: &std::path::Path) {
    for (dim, n) in &report.chunks {
        println!("{dim}: {n} chunks");
    }
    if !report.approximated.is_empty() {
        println!(
            "{} block states had no exact match and used the closest state",
            report.approximated.len()
        );
    }
    for (state, n) in &report.unmapped {
        println!("unmapped (written as air): {state} ({n} palettes)");
    }
    for (what, n) in &report.skipped {
        println!("not converted: {n} {what}");
    }
    for note in &report.notes {
        println!("note: {note}");
    }
    println!("wrote {}", out.display());
}

#[derive(clap::Args)]
struct PruneArgs {
    #[arg(value_enum)]
    edition: PruneEdition,
    /// Source world: Java folder, Bedrock folder or .mcworld, PS3 save folder or Xbox 360 savegame.dat
    world: PathBuf,
    /// New folder (Java, Bedrock, PS3) or file (Xbox 360) for the pruned copy
    out: PathBuf,
    /// Box layer: X,Z,WIDTH,HEIGHT
    #[arg(long = "box", value_name = "X,Z,W,H", allow_hyphen_values = true)]
    boxes: Vec<String>,
    /// Circle layer: X,Z,RADIUS
    #[arg(long = "circle", value_name = "X,Z,R", allow_hyphen_values = true)]
    circles: Vec<String>,
    /// Split layer: X,Z,ANGLE (keeps the right side of a ray heading ANGLE degrees, 0 = east)
    #[arg(long = "split", value_name = "X,Z,ANGLE", allow_hyphen_values = true)]
    splits: Vec<String>,
    /// Polygon layer: "X,Z;X,Z;X,Z..."
    #[arg(long = "polygon", value_name = "POINTS", allow_hyphen_values = true)]
    polygons: Vec<String>,
    /// Time layer on InhabitedTime ticks (Java, console): e.g. ">=1200", "<20"
    #[arg(long, value_name = "CMP+TICKS")]
    time: Option<String>,
    /// Keep only the selected chunks instead of removing them
    #[arg(long)]
    invert: bool,
    /// Only prune this dimension (0 Overworld, -1 Nether, 1 End); others are copied unchanged
    #[arg(long, allow_hyphen_values = true)]
    dimension: Option<i32>,
}

#[derive(Clone, Copy, ValueEnum)]
enum PruneEdition {
    Java,
    Bedrock,
    Console,
}

fn numbers(spec: &str, n: usize, what: &str) -> Result<Vec<f64>, String> {
    let v: Vec<f64> = spec
        .split(',')
        .map(|s| s.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("bad {what} '{spec}'"))?;
    if v.len() != n {
        return Err(format!(
            "{what} needs {n} comma-separated numbers, got '{spec}'"
        ));
    }
    Ok(v)
}

fn selection(args: &PruneArgs) -> Result<mctk_prune::Selection, String> {
    use mctk_prune::{Comparison, Layer};
    let mut layers = Vec::new();
    for s in &args.boxes {
        let v = numbers(s, 4, "box")?;
        layers.push(Layer::Box {
            x: v[0],
            z: v[1],
            width: v[2],
            height: v[3],
        });
    }
    for s in &args.circles {
        let v = numbers(s, 3, "circle")?;
        layers.push(Layer::Circle {
            x: v[0],
            z: v[1],
            radius: v[2],
        });
    }
    for s in &args.splits {
        let v = numbers(s, 3, "split")?;
        layers.push(Layer::Split {
            x: v[0],
            z: v[1],
            angle: v[2],
        });
    }
    for s in &args.polygons {
        let points = s
            .split(';')
            .map(|p| numbers(p, 2, "polygon point").map(|v| (v[0], v[1])))
            .collect::<Result<Vec<_>, _>>()?;
        if points.len() < 3 {
            return Err(format!("polygon needs at least 3 points, got '{s}'"));
        }
        layers.push(Layer::Polygon { points });
    }
    if let Some(t) = &args.time {
        let t = t.trim();
        let (comparison, rest) = [
            ("<=", Comparison::LessOrEqual),
            (">=", Comparison::GreaterOrEqual),
            ("<", Comparison::Less),
            (">", Comparison::Greater),
            ("=", Comparison::Equal),
        ]
        .into_iter()
        .find_map(|(p, c)| t.strip_prefix(p).map(|r| (c, r)))
        .ok_or_else(|| format!("time needs a comparison (<, <=, =, >=, >), got '{t}'"))?;
        let ticks = rest
            .trim()
            .parse()
            .map_err(|_| format!("bad tick count in '{t}'"))?;
        layers.push(Layer::Time { comparison, ticks });
    }
    Ok(mctk_prune::Selection {
        layers,
        inverted: args.invert,
    })
}

#[derive(Subcommand)]
enum ConsoleAction {
    /// Print the save's platform, files and chunk counts
    Info { save: PathBuf },
    /// Write every file in the save archive into a new folder
    Extract { save: PathBuf, out: PathBuf },
    /// Convert the save into a Java Edition world in a new folder
    ToJava { save: PathBuf, out: PathBuf },
}

#[derive(Subcommand)]
enum BedrockAction {
    /// Print the world's name, version and database summary
    Info { world: PathBuf },
    /// Write a copy of the world with a freshly compacted database into a new folder
    Rewrite { world: PathBuf, out: PathBuf },
}

#[derive(Subcommand)]
enum WorldAction {
    /// Print the world's name, version, spawn and dimensions
    Info { dir: PathBuf },
    /// Print the block at absolute coordinates
    #[command(allow_negative_numbers = true)]
    Block {
        dir: PathBuf,
        x: i32,
        y: i32,
        z: i32,
        #[arg(long, default_value = "minecraft:overworld")]
        dimension: String,
    },
}

#[derive(Subcommand)]
enum NbtAction {
    /// Print a file as indented SNBT
    Dump {
        file: PathBuf,
        /// Byte order of the file
        #[arg(long, value_enum, default_value_t = Edition::Java)]
        edition: Edition,
    },
}

#[derive(Subcommand)]
enum RegionAction {
    /// List the chunks stored in a region file
    List { file: PathBuf },
    /// Print one chunk as SNBT, by local coordinates (0-31)
    Dump {
        file: PathBuf,
        #[arg(value_parser = clap::value_parser!(u8).range(0..32))]
        x: u8,
        #[arg(value_parser = clap::value_parser!(u8).range(0..32))]
        z: u8,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Edition {
    Java,
    Bedrock,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Nbt {
            action: NbtAction::Dump { file, edition },
        } => {
            let flavor = match edition {
                Edition::Java => Flavor::Java,
                Edition::Bedrock => Flavor::Bedrock,
            };
            let (compression, root) = mctk_nbt::read_file(&file, flavor)?;
            eprintln!("# {} ({compression:?})", file.display());
            println!("{root:#}");
        }
        Command::Region {
            action: RegionAction::List { file },
        } => {
            let region = Region::open(&file)?;
            let mut count = 0;
            for (lx, lz) in region.chunks() {
                let raw = region.raw_chunk(lx, lz)?.expect("listed chunk exists");
                let abs = region.position().map(|p| p.chunk(lx, lz));
                let abs = abs
                    .map(|c| format!(" chunk ({}, {})", c.x, c.z))
                    .unwrap_or_default();
                println!(
                    "({lx:2}, {lz:2}){abs}  {:?}  {} bytes  t={}",
                    raw.compression,
                    raw.data.len(),
                    raw.timestamp
                );
                count += 1;
            }
            eprintln!("# {count} chunks");
        }
        Command::Region {
            action: RegionAction::Dump { file, x, z },
        } => {
            let region = Region::open(&file)?;
            match region.read_chunk(x.into(), z.into())? {
                Some(root) => println!("{root:#}"),
                None => return Err(format!("no chunk at ({x}, {z})").into()),
            }
        }
        Command::World {
            action: WorldAction::Info { dir },
        } => {
            let world = mctk_java::JavaWorld::open(&dir)?;
            let l = &world.level;
            println!("name:         {}", l.name);
            println!(
                "version:      {}",
                l.version_name.as_deref().unwrap_or("unknown")
            );
            println!(
                "data version: {}",
                l.data_version.map_or("unknown".into(), |v| v.to_string())
            );
            if let Some(s) = &l.spawn {
                println!(
                    "spawn:        {} {} {} {}",
                    s.x,
                    s.y,
                    s.z,
                    s.dimension.as_deref().unwrap_or("")
                );
            }
            for d in &world.dimensions {
                println!("dimension:    {:<24} {} regions", d.id, d.regions()?.len());
            }
        }
        Command::World {
            action:
                WorldAction::Block {
                    dir,
                    x,
                    y,
                    z,
                    dimension,
                },
        } => {
            let world = mctk_java::JavaWorld::open(&dir)?;
            let pos = mctk_anvil::ChunkPos::new(x.div_euclid(16), z.div_euclid(16));
            let chunk = world.chunk(&dimension, pos)?.ok_or("chunk not generated")?;
            match chunk.block(x.rem_euclid(16) as usize, y, z.rem_euclid(16) as usize) {
                Some(b) => println!("{b}"),
                None => return Err("no block data at that position".into()),
            }
        }
        Command::Bedrock {
            action: BedrockAction::Info { world },
        } => {
            let world = mctk_bedrock::BedrockWorld::open(&world)?;
            let mut chunks = std::collections::BTreeSet::new();
            let (mut subchunks, mut actors, mut named) = (0, 0, 0);
            for key in world.db.entries.keys() {
                match mctk_bedrock::Key::decode(key) {
                    mctk_bedrock::Key::Chunk(c) => {
                        chunks.insert((c.dimension, c.x, c.z));
                        if c.tag == mctk_bedrock::ChunkTag::SubChunkPrefix {
                            subchunks += 1;
                        }
                    }
                    mctk_bedrock::Key::Actor(_) => actors += 1,
                    _ => named += 1,
                }
            }
            println!("name:      {}", world.level.level_name().unwrap_or("?"));
            if let Some(v) = world.level.last_opened_version() {
                let v: Vec<String> = v.iter().map(i64::to_string).collect();
                println!("version:   {}", v.join("."));
            }
            println!("keys:      {}", world.db.len());
            println!("chunks:    {} ({subchunks} subchunks)", chunks.len());
            println!("actors:    {actors}");
            println!("other:     {named} keys");
        }
        Command::Bedrock {
            action: BedrockAction::Rewrite { world, out },
        } => {
            let w = mctk_bedrock::BedrockWorld::open(&world)?;
            w.save_to_folder(&out)?;
            println!(
                "wrote {} ({} keys) to {}",
                w.level.level_name().unwrap_or("world"),
                w.db.len(),
                out.display()
            );
        }
        Command::Console {
            action: ConsoleAction::Info { save },
        } => {
            let save = mctk_console::ConsoleSave::open(&save)?;
            println!("platform: {:?}", save.platform);
            println!(
                "archive:  version {} ({} files)",
                save.archive.version,
                save.archive.entries.len()
            );
            for (dim, rx, rz, entry) in save.archive.regions() {
                let chunks = mctk_console::ConsoleRegion::parse(entry.data.clone())?
                    .chunks()
                    .count();
                let dim = match dim {
                    -1 => "nether",
                    1 => "end",
                    _ => "overworld",
                };
                println!(
                    "  {:<22} {dim:<9} region ({rx}, {rz}) {chunks} chunks",
                    entry.name
                );
            }
            for e in save
                .archive
                .entries
                .iter()
                .filter(|e| !e.name.ends_with(".mcr"))
            {
                println!("  {:<34} {} bytes", e.name, e.data.len());
            }
        }
        Command::Console {
            action: ConsoleAction::Extract { save, out },
        } => {
            let save = mctk_console::ConsoleSave::open(&save)?;
            if out.exists() && std::fs::read_dir(&out)?.next().is_some() {
                return Err(format!("{} is not empty", out.display()).into());
            }
            for e in &save.archive.entries {
                // Archive names are relative paths; refuse anything that could escape `out`.
                let rel = std::path::Path::new(&e.name);
                if rel.is_absolute()
                    || rel
                        .components()
                        .any(|c| !matches!(c, std::path::Component::Normal(_)))
                {
                    return Err(format!("unsafe file name in archive: {}", e.name).into());
                }
                let path = out.join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, &e.data)?;
            }
            println!(
                "extracted {} files to {}",
                save.archive.entries.len(),
                out.display()
            );
        }
        Command::Console {
            action: ConsoleAction::ToJava { save, out },
        } => {
            let save = mctk_console::ConsoleSave::open(&save)?;
            let report = mctk_convert::console_to_java(&save, &out)?;
            for (folder, n) in &report.chunks {
                let name = if folder.is_empty() {
                    "overworld"
                } else {
                    folder
                };
                println!("{name}: {n} chunks");
            }
            println!(
                "{} region files, {} data files copied",
                report.region_files,
                report.copied_files.len()
            );
            for note in &report.notes {
                println!("note: {note}");
            }
            println!(
                "wrote {} (pre-1.9 format: open it in Java Edition to upgrade)",
                out.display()
            );
        }
        Command::Ext { dir, action } => {
            let dir = dir.unwrap_or_else(mctk_lua::default_extensions_dir);
            match action {
                ExtAction::List => {
                    let list = mctk_lua::list(&dir);
                    if list.is_empty() {
                        println!("no extensions in {}", dir.display());
                    }
                    for e in list {
                        match e {
                            Ok(e) => println!(
                                "{:20} {} {} — {}",
                                e.id, e.manifest.name, e.manifest.version, e.manifest.description
                            ),
                            Err((id, err)) => println!("{id:20} (broken: {err})"),
                        }
                    }
                }
                ExtAction::Install { source } => {
                    let e = if source.is_dir() {
                        mctk_lua::install_folder(&source, &dir)?
                    } else {
                        mctk_lua::install_zip(&source, &dir)?
                    };
                    println!(
                        "installed {} as {} in {}",
                        e.manifest.name,
                        e.id,
                        e.dir.display()
                    );
                }
                ExtAction::Remove { id } => {
                    mctk_lua::remove(&dir, &id)?;
                    println!("removed {id}");
                }
                ExtAction::Run {
                    extension,
                    world,
                    output,
                } => {
                    let path = PathBuf::from(&extension);
                    let ext = if path.join("manifest.json").is_file() {
                        mctk_lua::Extension::load(&path)?
                    } else {
                        mctk_lua::Extension::load(&dir.join(&extension))?
                    };
                    let out = mctk_lua::run(
                        &ext,
                        &mctk_lua::RunOptions {
                            world,
                            output,
                            ..Default::default()
                        },
                    )?;
                    for line in &out.log {
                        println!("{line}");
                    }
                    if let Some(r) = out.result {
                        println!("=> {r}");
                    }
                }
            }
        }
        Command::Convert { action } => match action {
            ConvertAction::JavaToBedrock { world, out } => {
                print_edition_report(&mctk_convert::java_to_bedrock(&world, &out)?, &out)
            }
            ConvertAction::BedrockToJava { world, out } => {
                print_edition_report(&mctk_convert::bedrock_to_java(&world, &out)?, &out)
            }
            ConvertAction::ConsoleToJava { save, out } => {
                let save = mctk_console::ConsoleSave::open(&save)?;
                let report = mctk_convert::console_to_java(&save, &out)?;
                for (folder, n) in &report.chunks {
                    let name = if folder.is_empty() {
                        "overworld"
                    } else {
                        folder
                    };
                    println!("{name}: {n} chunks");
                }
                println!(
                    "wrote {} (pre-1.9 format: open it in Java Edition to upgrade)",
                    out.display()
                );
            }
        },
        Command::Prune(args) => {
            let sel = selection(&args)?;
            let report = match args.edition {
                PruneEdition::Java => {
                    mctk_prune::prune_java(&args.world, &args.out, &sel, args.dimension)?
                }
                PruneEdition::Bedrock => {
                    mctk_prune::prune_bedrock_world(&args.world, &args.out, &sel, args.dimension)?
                }
                PruneEdition::Console => {
                    let mut save = mctk_console::ConsoleSave::open(&args.world)?;
                    let report = mctk_prune::prune_console(&mut save, &sel, args.dimension)?;
                    mctk_prune::save_console_copy(&save, &args.out)?;
                    report
                }
            };
            println!(
                "kept {} chunks, removed {}; wrote {}",
                report.kept,
                report.removed,
                args.out.display()
            );
            for note in &report.notes {
                println!("note: {note}");
            }
        }
    }
    Ok(())
}
