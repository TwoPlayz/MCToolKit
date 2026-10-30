//! Legacy Console → Java Edition.
//!
//! Console Edition descends from Java Edition 1.8 and stores chunks with Java's numeric
//! block ids. The conversion therefore writes classic Anvil chunks in the pre-1.9 layout
//! and deliberately omits `DataVersion`: Java treats such chunks as coming from before
//! 1.9 and runs its own complete upgrade chain on them when the world is opened (numeric
//! ids to 1.13 block states, entity id renames, and every later format change). Blocks,
//! data values and light are copied exactly; only the layout changes, from full-height
//! console columns to 16-block sections.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use mctk_anvil::{ChunkCompression, ChunkPos, RawChunk, RegionBuilder, RegionPos};
use mctk_console::{
    ConsoleChunk, ConsoleRegion, ConsoleSave, RawConsoleChunk, console_block_index,
};
use mctk_nbt::{Compound, Flavor, List, NamedTag, Tag, TagType};

use crate::Error;
use crate::options::{ConvertOptions, Progress};
use crate::pipeline::{ChunkStages, Flow, run_pipeline};

/// What a conversion produced.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConvertReport {
    /// Converted chunks per Java dimension folder ("" = Overworld, "DIM-1", "DIM1").
    pub chunks: BTreeMap<String, usize>,
    pub region_files: usize,
    pub copied_files: Vec<String>,
    pub notes: Vec<String>,
}

fn nibble(arr: &[i8], i: usize) -> u8 {
    let b = arr.get(i / 2).copied().unwrap_or(0) as u8;
    if i % 2 == 0 { b & 0x0f } else { b >> 4 }
}

fn set_nibble(arr: &mut [i8], i: usize, v: u8) {
    let b = arr[i / 2] as u8;
    arr[i / 2] = if i % 2 == 0 {
        (b & 0xf0) | (v & 0x0f)
    } else {
        (b & 0x0f) | (v << 4)
    } as i8;
}

fn byte_array<'a>(level: &'a Compound, key: &str) -> Option<&'a [i8]> {
    match level.get(key) {
        Some(Tag::ByteArray(a)) => Some(a),
        _ => None,
    }
}

/// Lists Java expects to hold compounds; console saves write empty ones as byte lists.
fn compound_list(level: &Compound, key: &str) -> Tag {
    match level.get(key) {
        Some(Tag::List(l)) if !l.is_empty() => Tag::List(l.clone()),
        _ => Tag::List(List::new(TagType::Compound)),
    }
}

/// Converts one console chunk to a pre-1.9 Anvil chunk (no `DataVersion`).
pub fn console_chunk_to_anvil(chunk: &ConsoleChunk) -> Result<NamedTag, Error> {
    let level = chunk
        .to_nbt()
        .tag
        .get("Level")
        .and_then(Tag::as_compound)
        .ok_or_else(|| Error::Invalid("chunk without Level".into()))?;
    let blocks =
        byte_array(level, "Blocks").ok_or_else(|| Error::Invalid("chunk without Blocks".into()))?;
    let data = byte_array(level, "Data");
    let sky = byte_array(level, "SkyLight");
    let block_light = byte_array(level, "BlockLight");

    let mut sections = Vec::new();
    let mut heightmap = vec![0i32; 256];
    for sy in 0..chunk.height / 16 {
        let mut s_blocks = vec![0i8; 4096];
        let mut s_data = vec![0i8; 2048];
        let mut s_sky = vec![0i8; 2048];
        let mut s_light = vec![0i8; 2048];
        let mut any = false;
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let src = console_block_index(x, sy * 16 + y, z);
                    let dst = (y * 16 + z) * 16 + x;
                    let id = blocks[src];
                    s_blocks[dst] = id;
                    if id != 0 {
                        any = true;
                        let h = &mut heightmap[z * 16 + x];
                        *h = (*h).max((sy * 16 + y + 1) as i32);
                    }
                    if let Some(d) = data {
                        set_nibble(&mut s_data, dst, nibble(d, src));
                    }
                    if let Some(l) = sky {
                        set_nibble(&mut s_sky, dst, nibble(l, src));
                    }
                    if let Some(l) = block_light {
                        set_nibble(&mut s_light, dst, nibble(l, src));
                    }
                }
            }
        }
        // Empty sections are omitted, as Java does for all-air sections.
        if !any {
            continue;
        }
        let mut s = Compound::new();
        s.insert("Y".into(), Tag::Byte(sy as i8));
        s.insert("Blocks".into(), Tag::ByteArray(s_blocks));
        s.insert("Data".into(), Tag::ByteArray(s_data));
        s.insert("BlockLight".into(), Tag::ByteArray(s_light));
        s.insert("SkyLight".into(), Tag::ByteArray(s_sky));
        sections.push(Tag::Compound(s));
    }

    let mut out = Compound::new();
    out.insert("xPos".into(), Tag::Int(chunk.x));
    out.insert("zPos".into(), Tag::Int(chunk.z));
    out.insert(
        "LastUpdate".into(),
        Tag::Long(level.get("LastUpdate").and_then(Tag::as_i64).unwrap_or(0)),
    );
    out.insert(
        "InhabitedTime".into(),
        Tag::Long(
            level
                .get("InhabitedTime")
                .and_then(Tag::as_i64)
                .unwrap_or(0),
        ),
    );
    out.insert("TerrainPopulated".into(), Tag::Byte(1));
    out.insert("LightPopulated".into(), Tag::Byte(1));
    out.insert("V".into(), Tag::Byte(1));
    if let Some(Tag::ByteArray(b)) = level.get("Biomes") {
        if b.len() == 256 {
            out.insert("Biomes".into(), Tag::ByteArray(b.clone()));
        }
    }
    out.insert("HeightMap".into(), Tag::IntArray(heightmap));
    out.insert(
        "Sections".into(),
        Tag::List(List::from_tags(TagType::Compound, sections).expect("compounds")),
    );
    out.insert("Entities".into(), compound_list(level, "Entities"));
    out.insert("TileEntities".into(), compound_list(level, "TileEntities"));
    if let Some(Tag::List(l)) = level.get("TileTicks") {
        if !l.is_empty() {
            out.insert("TileTicks".into(), Tag::List(l.clone()));
        }
    }
    let mut root = Compound::new();
    root.insert("Level".into(), Tag::Compound(out));
    Ok(NamedTag {
        name: String::new(),
        tag: Tag::Compound(root),
    })
}

fn dimension_folder(console_dim: i32) -> &'static str {
    match console_dim {
        -1 => "DIM-1",
        1 => "DIM1",
        _ => "",
    }
}

fn write_gzip_nbt(path: &Path, tag: &NamedTag) -> Result<(), Error> {
    let raw = mctk_nbt::to_bytes(tag, Flavor::Java)?;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&raw)?;
    std::fs::write(path, e.finish()?)?;
    Ok(())
}

/// Builds the Java `level.dat` from the console one. Console `level.dat` is Java-style
/// NBT already; version markers are left as the console wrote them (pre-1.9) so the game
/// upgrades the world, and the host player is attached when `Data.Player` is missing.
fn convert_level_dat(save: &ConsoleSave, report: &mut ConvertReport) -> Result<NamedTag, Error> {
    let entry = save
        .archive
        .get("level.dat")
        .ok_or_else(|| Error::Invalid("save has no level.dat".into()))?;
    let (_, raw) = mctk_nbt::decompress(&entry.data)?;
    let mut root = mctk_nbt::from_bytes(&raw, Flavor::Java)?;
    let data = root
        .tag
        .as_compound_mut()
        .and_then(|c| c.get_mut("Data"))
        .and_then(Tag::as_compound_mut)
        .ok_or_else(|| Error::Invalid("level.dat without Data".into()))?;
    data.shift_remove("DataVersion");
    data.shift_remove("Version");
    // Anvil storage version.
    data.insert("version".into(), Tag::Int(19133));
    if !data.contains_key("Player") {
        let player = save.archive.entries.iter().find(|e| {
            let name = e.name.rsplit('/').next().unwrap_or(&e.name);
            name.ends_with(".dat") && (e.name.starts_with("players/") || name.starts_with("P_"))
        });
        if let Some(p) = player {
            let (_, raw) = mctk_nbt::decompress(&p.data)?;
            if let Ok(tag) = mctk_nbt::from_bytes(&raw, Flavor::Java) {
                if let Tag::Compound(c) = tag.tag {
                    data.insert("Player".into(), Tag::Compound(c));
                    report
                        .notes
                        .push(format!("used {} as the single-player data", p.name));
                }
            }
        }
    }
    Ok(root)
}

/// Per-chunk work of the console → Java conversion, for [`run_pipeline`].
struct ConsoleStages<'a> {
    work: &'a [(i32, ConsoleRegion)],
    /// (index into `work`, local x, local z) of every chunk to convert.
    jobs: &'a [(usize, usize, usize)],
    options: &'a ConvertOptions,
    progress: &'a Progress,
    timestamp: u32,
}

#[derive(Default)]
struct ConsoleSlot {
    raw: Option<RawConsoleChunk>,
    chunk: Option<ConsoleChunk>,
    converted: Option<(ChunkPos, RawChunk)>,
}

impl ChunkStages for ConsoleStages<'_> {
    type Slot = ConsoleSlot;
    type Output = (ChunkPos, RawChunk);

    fn load(&self, index: usize, slot: &mut ConsoleSlot) -> Result<Flow, Error> {
        let (r, lx, lz) = self.jobs[index];
        let (dim, region) = &self.work[r];
        self.progress.set_dimension(match dim {
            -1 => "minecraft:the_nether",
            1 => "minecraft:the_end",
            _ => "minecraft:overworld",
        });
        slot.raw = region.raw_chunk(lx, lz)?;
        Ok(if slot.raw.is_some() {
            Flow::Continue
        } else {
            Flow::Invalid
        })
    }

    fn parse(&self, _: usize, slot: &mut ConsoleSlot) -> Result<Flow, Error> {
        let raw = slot.raw.take().expect("loaded");
        // A chunk that does not decode is counted as invalid and left out.
        slot.chunk = raw
            .nbt()
            .ok()
            .and_then(|n| ConsoleChunk::from_nbt(n).ok());
        Ok(if slot.chunk.is_some() {
            Flow::Continue
        } else {
            Flow::Invalid
        })
    }

    fn convert(&self, _: usize, slot: &mut ConsoleSlot) -> Result<Flow, Error> {
        let chunk = slot.chunk.take().expect("parsed");
        let offset = self.options.offset;
        let mut anvil = console_chunk_to_anvil(&chunk)?;
        let pos = ChunkPos::new(chunk.x + offset.0, chunk.z + offset.1);
        if offset != (0, 0) {
            if let Some(l) = anvil
                .tag
                .as_compound_mut()
                .and_then(|c| c.get_mut("Level"))
                .and_then(Tag::as_compound_mut)
            {
                l.insert("xPos".into(), Tag::Int(pos.x));
                l.insert("zPos".into(), Tag::Int(pos.z));
            }
        }
        let raw = RawChunk::encode(&anvil, ChunkCompression::Zlib, self.timestamp)?;
        self.progress.chunk(pos.x, pos.z);
        slot.converted = Some((pos, raw));
        Ok(Flow::Continue)
    }

    fn clean(&self, _: usize, slot: &mut ConsoleSlot) -> Option<(ChunkPos, RawChunk)> {
        std::mem::take(slot).converted
    }
}

/// Converts a console save into a new Java world folder at `out` (must be empty or missing).
pub fn console_to_java(save: &ConsoleSave, out: &Path) -> Result<ConvertReport, Error> {
    console_to_java_with(save, out, &ConvertOptions::default(), &Progress::default())
}

/// [`console_to_java`] with options and live progress.
pub fn console_to_java_with(
    save: &ConsoleSave,
    out: &Path,
    options: &ConvertOptions,
    progress: &Progress,
) -> Result<ConvertReport, Error> {
    if out.exists() && std::fs::read_dir(out)?.next().is_some() {
        return Err(Error::Invalid(format!("{} is not empty", out.display())));
    }
    std::fs::create_dir_all(out)?;
    let mut report = ConvertReport::default();

    let mut level = convert_level_dat(save, &mut report)?;
    if let Some(data) = level
        .tag
        .as_compound_mut()
        .and_then(|c| c.get_mut("Data"))
        .and_then(Tag::as_compound_mut)
    {
        if let Some(g) = options.game_mode {
            data.insert("GameType".into(), Tag::Int(g));
        }
        if let Some(d) = options.difficulty {
            data.insert("Difficulty".into(), Tag::Byte(d as i8));
        }
        for key in ["SpawnX", "SpawnZ"] {
            let shift = if key == "SpawnX" {
                options.offset.0
            } else {
                options.offset.1
            } * 16;
            if let Some(v) = data.get(key).and_then(Tag::as_i64) {
                data.insert(key.into(), Tag::Int(v as i32 + shift));
            }
        }
    }
    write_gzip_nbt(&out.join("level.dat"), &level)?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    let mut regions: HashMap<(&'static str, RegionPos), RegionBuilder> = HashMap::new();
    let mut work = Vec::new();
    for (dim, _, _, entry) in save.archive.regions() {
        if entry.data.is_empty() || !options.includes(dim) {
            continue;
        }
        let region = ConsoleRegion::parse(entry.data.clone())?;
        progress
            .chunks_total
            .fetch_add(region.chunks().count(), Ordering::Relaxed);
        work.push((dim, region));
    }
    progress.regions_total.store(work.len(), Ordering::Relaxed);
    // Every chunk of every selected region goes through the staged pipeline; the Java
    // dimension name shown in the progress is that of the most recently started region.
    let jobs: Vec<(usize, usize, usize)> = work
        .iter()
        .enumerate()
        .flat_map(|(r, (_, region))| region.chunks().map(move |(lx, lz)| (r, lx, lz)))
        .collect();
    let stages = ConsoleStages {
        work: &work,
        jobs: &jobs,
        options,
        progress,
        timestamp,
    };
    let finished = run_pipeline(&stages, jobs.len(), 0, progress)?;
    for (&(r, _, _), done) in jobs.iter().zip(finished) {
        let Some((pos, raw)) = done else { continue };
        let folder = dimension_folder(work[r].0);
        let (clx, clz) = pos.local();
        regions
            .entry((folder, pos.region()))
            .or_default()
            .set(clx, clz, Some(raw));
        *report.chunks.entry(folder.to_string()).or_insert(0) += 1;
    }
    progress.regions_done.store(work.len(), Ordering::Relaxed);
    for ((folder, pos), builder) in &regions {
        let dir = out.join(folder).join("region");
        std::fs::create_dir_all(&dir)?;
        builder.write(&dir, *pos)?;
        report.region_files += 1;
    }

    for e in &save.archive.entries {
        if let Some(rest) = e.name.strip_prefix("data/") {
            // Archive names are relative paths; refuse anything that could escape `out`.
            let rel = Path::new(rest);
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err(Error::Invalid(format!(
                    "unsafe file name in save: {}",
                    e.name
                )));
            }
            let path = out.join("data").join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, &e.data)?;
            report.copied_files.push(e.name.clone());
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console_chunk(height: usize) -> ConsoleChunk {
        let mut level = Compound::new();
        level.insert("xPos".into(), Tag::Int(-3));
        level.insert("zPos".into(), Tag::Int(7));
        level.insert("Blocks".into(), Tag::ByteArray(vec![0; height * 256]));
        level.insert("Data".into(), Tag::ByteArray(vec![0; height * 128]));
        level.insert("SkyLight".into(), Tag::ByteArray(vec![0; height * 128]));
        level.insert("BlockLight".into(), Tag::ByteArray(vec![0; height * 128]));
        let mut root = Compound::new();
        root.insert("Level".into(), Tag::Compound(level));
        ConsoleChunk::from_nbt(NamedTag {
            name: String::new(),
            tag: Tag::Compound(root),
        })
        .unwrap()
    }

    #[test]
    fn relayout_into_sections() {
        let mut chunk = console_chunk(256);
        chunk.set_block(1, 0, 2, 7, 0).unwrap();
        chunk.set_block(15, 200, 3, 35, 14).unwrap();
        let out = console_chunk_to_anvil(&chunk).unwrap();
        let level = out.tag.get("Level").unwrap();
        assert_eq!(level.get("xPos"), Some(&Tag::Int(-3)));
        assert!(level.get("DataVersion").is_none() && out.tag.get("DataVersion").is_none());
        let Some(Tag::List(sections)) = level.get("Sections") else {
            panic!("no sections")
        };
        // Only the two non-empty sections are written.
        assert_eq!(sections.len(), 2);
        let s12 = sections
            .iter()
            .find(|s| s.get("Y") == Some(&Tag::Byte(12)))
            .expect("section 12");
        let Some(Tag::ByteArray(blocks)) = s12.get("Blocks") else {
            panic!()
        };
        let Some(Tag::ByteArray(data)) = s12.get("Data") else {
            panic!()
        };
        let i = (8 * 16 + 3) * 16 + 15; // y=200 -> local y 8
        assert_eq!(blocks[i], 35);
        assert_eq!(nibble(data, i), 14);
        let Some(Tag::IntArray(hm)) = level.get("HeightMap") else {
            panic!()
        };
        assert_eq!(hm[3 * 16 + 15], 201);
        assert_eq!(hm[2 * 16 + 1], 1);
        assert_eq!(hm[0], 0);
    }
}
