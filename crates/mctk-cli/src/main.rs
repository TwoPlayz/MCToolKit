use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use mctk_anvil::Region;
use mctk_nbt::Flavor;

#[derive(Parser)]
#[command(name = "mctk", version, about = "Minecraft world toolkit")]
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
}

#[derive(Subcommand)]
enum ConvertAction {
    /// Java 1.13+ world (best: 26.3) to a Bedrock 1.26.60 world folder
    JavaToBedrock { world: PathBuf, out: PathBuf },
    /// Bedrock world (folder or .mcworld) to a Java 26.3 world folder
    BedrockToJava { world: PathBuf, out: PathBuf },
    /// Legacy Console save to a Java world (upgraded by the game on first open)
    ConsoleToJava { save: PathBuf, out: PathBuf },
    /// Java 26.x world to a console save in a folder: a signed PS3 save folder, an Xbox 360
    /// `savegame.dat`, or a Wii U `<name>.wii` with its `.ext` companion
    JavaToConsole {
        world: PathBuf,
        /// Folder to write into (created if missing)
        out: PathBuf,
        /// ps3, xbox360 or wiiu
        #[arg(long, default_value = "ps3")]
        platform: String,
        /// current (what the original converter writes: version 10 archive, Version 12 chunks, compressed PS3
        /// GAMEDATA, fitted world size) or legacy (version 8, readable by older game versions)
        #[arg(long, default_value = "current")]
        format: String,
        /// With --format current: write the classic 54-chunk world and drop the chunks beyond it, as the
        /// original converter does (default: the smallest size class that holds every chunk)
        #[arg(long)]
        classic_size: bool,
        /// PS3 only: us-digital, us-disc, eu-digital, eu-disc, jp-digital (or a title id)
        #[arg(long, default_value = "us-digital")]
        region: String,
        /// The world's name as the console shows it (default: the world folder's name)
        #[arg(long)]
        name: Option<String>,
        /// PS3 ICON0.png (default: the world's icon.png)
        #[arg(long)]
        icon: Option<PathBuf>,
        /// PS3 THUMB and the Wii U thumbnail, 64x64 (default: the world's icon.png)
        #[arg(long)]
        thumb: Option<PathBuf>,
        /// PS3 only: write the save folder unencrypted (plain METADATA, no PARAM.PFD) for an emulator such as
        /// RPCS3, which cannot read a console's protected files. A real console will not accept it.
        #[arg(long)]
        unencrypted: bool,
    },
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
    /// Write a signed PlayStation 3 save folder (PARAM.SFO, METADATA, PARAM.PFD...) around a PS3 save
    SignPs3 {
        /// A PS3 save folder or its GAMEDATA file
        save: PathBuf,
        /// Folder to create the save folder in (on a USB drive: <drive>/PS3/SAVEDATA)
        out: PathBuf,
        /// us-digital, us-disc, eu-digital, eu-disc or jp-digital (or a title id such as NPUB31419)
        #[arg(long, default_value = "us-digital")]
        region: String,
        /// The world's name as the console shows it
        #[arg(long, default_value = "My Converted World")]
        name: String,
        /// PNG for ICON0.png, the menu icon (genuine saves: 320x176; default: the ICON0 of a save folder given as `save`)
        #[arg(long)]
        icon: Option<PathBuf>,
        /// PNG for THUMB, the world thumbnail METADATA records (genuine: 64x64; default: the THUMB of a
        /// save folder given as `save`, else the icon)
        #[arg(long)]
        thumb: Option<PathBuf>,
        /// yyMMddHHmmss for the folder name (default: now, UTC)
        #[arg(long)]
        timestamp: Option<String>,
        /// Write the folder unencrypted (plain METADATA, no PARAM.PFD) for an emulator such as RPCS3
        #[arg(long)]
        unencrypted: bool,
    },
    /// Make a PlayStation 3 save folder readable by an emulator such as RPCS3, in place: METADATA is decrypted
    /// and PARAM.PFD removed (a real console will not accept the result; keep a copy)
    UnprotectPs3 { folders: Vec<PathBuf> },
    /// Check the signing of PlayStation 3 save folders (PFD hashes, METADATA)
    VerifyPs3 { folders: Vec<PathBuf> },
    /// Re-sign a PlayStation 3 save folder in place after its GAMEDATA changed
    ResignPs3 { folder: PathBuf },
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
            if save.platform == mctk_console::Platform::WiiU {
                if let Some(ext) = mctk_console::read_wiiu_ext(&save.source) {
                    println!("name:     {} (thumbnail {} bytes)", ext.name, ext.thumbnail.len());
                }
            }
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
        Command::Console {
            action:
                ConsoleAction::SignPs3 {
                    save,
                    out,
                    region,
                    name,
                    icon,
                    thumb,
                    timestamp,
                    unencrypted,
                },
        } => {
            use mctk_console::{Ps3Random, Ps3Region};
            let region = match region.to_ascii_lowercase().as_str() {
                "us-digital" | "us" => Ps3Region::UsDigital,
                "us-disc" => Ps3Region::UsDisc,
                "eu-digital" | "eu" => Ps3Region::EuDigital,
                "eu-disc" => Ps3Region::EuDisc,
                "jp-digital" | "jp" => Ps3Region::JpDigital,
                other => Ps3Region::from_title_id(&other.to_ascii_uppercase())
                    .ok_or_else(|| format!("unknown region '{other}'"))?,
            };
            // The save must be a PS3 one (a folder or its GAMEDATA); its file is signed byte for byte.
            let parsed = mctk_console::ConsoleSave::open(&save)?;
            if parsed.platform != mctk_console::Platform::Ps3 {
                return Err(format!(
                    "{} is a {:?} save; sign-ps3 needs a PS3 save (convert it to PS3 first)",
                    save.display(),
                    parsed.platform
                )
                .into());
            }
            let gamedata = std::fs::read(&parsed.source)?;
            // Genuine saves name the icon ICON0.PNG; look for either spelling beside the save.
            let beside = |names: &[&str]| {
                let dir = if save.is_dir() { save.clone() } else { save.parent().map(PathBuf::from).unwrap_or_default() };
                names.iter().find_map(|n| std::fs::read(dir.join(n)).ok())
            };
            let icon_bytes = match &icon {
                Some(p) => std::fs::read(p)?,
                None => beside(&["ICON0.png", "ICON0.PNG"]).ok_or(
                    "no icon: pass --icon <png> (or a save folder that holds an ICON0.png)",
                )?,
            };
            let thumb_bytes = match &thumb {
                Some(p) => std::fs::read(p)?,
                None => beside(&["THUMB"]).unwrap_or_else(|| icon_bytes.clone()),
            };
            let timestamp =
                timestamp.unwrap_or_else(|| mctk_console::utc_timestamp(std::time::SystemTime::now()));
            let folder = if unencrypted {
                mctk_console::build_save_folder_unencrypted(gamedata, icon_bytes, thumb_bytes, region, &name, &timestamp)?
            } else {
                mctk_console::build_save_folder(gamedata, icon_bytes, thumb_bytes, region, &name, &timestamp, &Ps3Random::generate())?
            };
            let dest = mctk_console::write_save_folder(&out, &folder)?;
            println!("wrote {} ({} files, {})", dest.display(), folder.files.len(), if unencrypted { "unencrypted, for an emulator" } else { "signed" });
        }
        Command::Console {
            action: ConsoleAction::UnprotectPs3 { folders },
        } => {
            for f in &folders {
                mctk_console::unprotect_save_folder(f)?;
                println!("unprotected {} (METADATA plain, no PARAM.PFD)", f.display());
            }
        }
        Command::Console {
            action: ConsoleAction::ResignPs3 { folder },
        } => {
            mctk_console::resign_save_folder(&folder)?;
            let n = mctk_console::verify_save_folder(&folder)?;
            println!("re-signed {} (GAMEDATA {n} bytes)", folder.display());
        }
        Command::Console {
            action: ConsoleAction::VerifyPs3 { folders },
        } => {
            let mut failures = 0;
            for f in &folders {
                match mctk_console::verify_save_folder(f) {
                    Ok(n) => println!("OK    {}: signed correctly ({n} bytes GAMEDATA)", f.display()),
                    Err(e) => {
                        failures += 1;
                        println!("FAIL  {}: {e}", f.display());
                    }
                }
            }
            if failures > 0 {
                return Err(format!("{failures} save(s) failed verification").into());
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
            ConvertAction::JavaToConsole {
                world,
                out,
                platform,
                format,
                classic_size,
                region,
                name,
                icon,
                thumb,
                unencrypted,
            } => {
                use mctk_console::{Platform, Ps3Random, Ps3Region};
                let platform = match platform.to_ascii_lowercase().as_str() {
                    "ps3" => Platform::Ps3,
                    "xbox360" | "xbox" | "x360" => Platform::Xbox360,
                    "wiiu" | "wii-u" => Platform::WiiU,
                    other => return Err(format!("unknown platform '{other}' (ps3, xbox360 or wiiu)").into()),
                };
                let console_format = match format.to_ascii_lowercase().as_str() {
                    "current" => mctk_convert::ConsoleFormat::Current,
                    "legacy" => mctk_convert::ConsoleFormat::Legacy,
                    other => return Err(format!("unknown format '{other}' (current or legacy)").into()),
                };
                let options = mctk_convert::ConvertOptions { console_format, classic_world_size: classic_size, ..Default::default() };
                let progress = mctk_convert::Progress::default();
                let map = mctk_replace::ReplacementMap::default();
                let name = name.unwrap_or_else(|| {
                    world.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "World".into())
                });
                std::fs::create_dir_all(&out)?;
                // The world's own icon (a 64x64 PNG) stands in for any image not given.
                let world_icon = std::fs::read(world.join("icon.png")).ok();
                let image = |given: &Option<PathBuf>| -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
                    Ok(match given {
                        Some(p) => Some(std::fs::read(p)?),
                        None => world_icon.clone(),
                    })
                };
                match platform {
                    Platform::Xbox360 => {
                        let file = out.join("savegame.dat");
                        let report = mctk_convert::java_to_console(&world, &file, platform, &map, &options, &progress)?;
                        println!("wrote {} ({} chunks, {} outside the world, {} unfinished left out)", file.display(), report.chunks, report.outside, report.partial);
                    }
                    Platform::WiiU => {
                        let file = out.join(format!("{name}.wii"));
                        let report = mctk_convert::java_to_console(&world, &file, platform, &map, &options, &progress)?;
                        let ext = mctk_console::write_wiiu_ext(&file, &name, &image(&thumb)?.unwrap_or_default())?;
                        println!(
                            "wrote {} and {} ({} chunks, {} outside the world, {} unfinished left out)",
                            file.display(),
                            ext.display(),
                            report.chunks,
                            report.outside,
                            report.partial
                        );
                    }
                    _ => {
                        let region = match region.to_ascii_lowercase().as_str() {
                            "us-digital" | "us" => Ps3Region::UsDigital,
                            "us-disc" => Ps3Region::UsDisc,
                            "eu-digital" | "eu" => Ps3Region::EuDigital,
                            "eu-disc" => Ps3Region::EuDisc,
                            "jp-digital" | "jp" => Ps3Region::JpDigital,
                            other => Ps3Region::from_title_id(&other.to_ascii_uppercase())
                                .ok_or_else(|| format!("unknown region '{other}'"))?,
                        };
                        let icon_png = image(&icon)?.ok_or("no icon: pass --icon <png> (the world has no icon.png)")?;
                        let thumb_png = image(&thumb)?.unwrap_or_else(|| icon_png.clone());
                        // The converter writes the GAMEDATA file; it is then signed into a save folder.
                        let stage = out.join(format!(".{name}.stage"));
                        std::fs::create_dir_all(&stage)?;
                        let gamedata = stage.join("GAMEDATA");
                        let report = mctk_convert::java_to_console(&world, &gamedata, platform, &map, &options, &progress)?;
                        let bytes = std::fs::read(&gamedata)?;
                        std::fs::remove_dir_all(&stage)?;
                        let stamp = mctk_console::utc_timestamp(std::time::SystemTime::now());
                        let folder = if unencrypted {
                            mctk_console::build_save_folder_unencrypted(bytes, icon_png, thumb_png, region, &name, &stamp)?
                        } else {
                            mctk_console::build_save_folder(bytes, icon_png, thumb_png, region, &name, &stamp, &Ps3Random::generate())?
                        };
                        let dest = mctk_console::write_save_folder(&out, &folder)?;
                        println!(
                            "wrote {} ({}; {} chunks, {} outside the world, {} unfinished left out)",
                            dest.display(),
                            if unencrypted { "unencrypted, for an emulator such as RPCS3" } else { "signed" },
                            report.chunks,
                            report.outside,
                            report.partial
                        );
                    }
                }
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
