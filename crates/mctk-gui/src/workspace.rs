//! An opened world: its working copy, open NBT documents, and saving back with backups.
//!
//! Saving never loses the original: a copied workspace is written back after the
//! original is zipped to `<name>_MCTBackup.zip`; an in-place Java world backs up each
//! file as `<file>_MCTBackup` before first overwriting it; console saves keep
//! `<file>_MCTBackup`.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use mctk_anvil::{ChunkCompression, ChunkPos, RawChunk, Region, RegionBuilder};
use mctk_bedrock::{BedrockWorld, ChunkKey, ChunkTag, Key};
use mctk_console::{ConsoleRegion, ConsoleRegionBuilder, ConsoleSave, RawConsoleChunk};
use mctk_nbt::{Compound, Compression, Flavor, List, NamedTag, Tag, TagType};

use crate::map_view::MapView;
use crate::nbt_view::{self, Format, NbtTree};
use crate::settings::Settings;
use crate::world_picker::UiEdition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocSource {
    File(PathBuf),
    JavaChunk {
        region: PathBuf,
        x: i32,
        z: i32,
    },
    BedrockKey(Vec<u8>),
    /// Read-only view of a Bedrock chunk's NBT records.
    BedrockChunk {
        dim: i32,
        x: i32,
        z: i32,
    },
    ConsoleEntry(String),
    ConsoleChunk {
        entry: String,
        x: i32,
        z: i32,
    },
}

pub struct Doc {
    pub title: String,
    pub source: DocSource,
    format: Option<Format>,
    pub tree: NbtTree,
    /// Tree generation the styles were computed for.
    pub styled: Option<u64>,
    /// Last generation seen, and when it changed (styles wait for edits to settle).
    pub seen: (u64, f64),
}

impl Doc {
    /// `FILE_TYPE` flags for style extensions.
    pub fn file_type(&self) -> u32 {
        use mctk_lua::style::file_type::*;
        let name = |s: &str| {
            let base = s
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(s)
                .to_ascii_lowercase();
            if base.starts_with("level.dat") {
                LEVEL
            } else if base.starts_with("map_") {
                MAP
            } else if base.starts_with("p_")
                || base.starts_with("player_")
                || base == "~local_player"
            {
                PLAYER
            } else {
                OTHER
            }
        };
        match &self.source {
            DocSource::File(p) => {
                let parent = p
                    .parent()
                    .and_then(|d| d.file_name())
                    .map(|n| n.to_string_lossy().to_ascii_lowercase());
                match parent.as_deref() {
                    Some("playerdata") => PLAYER,
                    Some("data")
                        if p.parent()
                            .and_then(|d| d.parent())
                            .is_some_and(|d| d.ends_with("players")) =>
                    {
                        PLAYER
                    }
                    _ => name(&p.to_string_lossy()),
                }
            }
            DocSource::BedrockKey(k) => name(&String::from_utf8_lossy(k)),
            DocSource::ConsoleEntry(e) if e.starts_with("players/") => PLAYER,
            DocSource::ConsoleEntry(e) => name(e),
            DocSource::JavaChunk { .. }
            | DocSource::BedrockChunk { .. }
            | DocSource::ConsoleChunk { .. } => CHUNK,
        }
    }
}

pub struct Workspace {
    pub name: String,
    pub edition: UiEdition,
    /// Where the world came from.
    pub original: PathBuf,
    /// The folder or file being worked on (a copy when `copied`).
    pub path: PathBuf,
    pub copied: bool,
    pub bedrock: Option<BedrockWorld>,
    pub console: Option<ConsoleSave>,
    pub docs: Vec<Doc>,
    pub active: Option<usize>,
    pub map: MapView,
    pub show_locator: bool,
    /// The working copy differs from the original (e.g. after pruning).
    pub world_changed: bool,
    pub expanded: HashSet<String>,
    backed_up: HashSet<PathBuf>,
}

pub fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let (from, to) = (e.path(), dst.join(e.file_name()));
        if e.file_type()?.is_dir() {
            copy_dir(&from, &to)?;
        } else if e.file_type()?.is_file() {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn zip_dir(src: &Path, zip_path: &Path) -> std::io::Result<()> {
    let mut z = zip::ZipWriter::new(std::fs::File::create(zip_path)?);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut stack = vec![src.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir)? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p
                    .strip_prefix(src)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                z.start_file(rel, opts).map_err(std::io::Error::other)?;
                z.write_all(&std::fs::read(&p)?)?;
            }
        }
    }
    z.finish().map_err(std::io::Error::other)?;
    Ok(())
}

fn backup_name(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn bedrock_nbt(bytes: &[u8]) -> Option<NamedTag> {
    mctk_nbt::from_bytes(bytes, Flavor::Bedrock).ok()
}

impl Workspace {
    /// Opens a world for the NBT Editor, copying it first when the settings ask for it.
    pub fn open(
        path: &Path,
        edition: UiEdition,
        settings: &Settings,
        copy_java: bool,
        copy_bedrock: bool,
    ) -> Result<Self, String> {
        let we = edition.world();
        let mut ws = Workspace {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            edition,
            original: path.to_path_buf(),
            path: path.to_path_buf(),
            copied: false,
            bedrock: None,
            console: None,
            docs: Vec::new(),
            active: None,
            map: MapView::new(path.to_path_buf(), we),
            show_locator: false,
            world_changed: false,
            expanded: HashSet::new(),
            backed_up: HashSet::new(),
        };
        match edition {
            UiEdition::Java => {
                let w = mctk_java::JavaWorld::open(path).map_err(|e| e.to_string())?;
                ws.name = w.level.name.clone();
                if copy_java {
                    let dst = settings
                        .new_workspace("EDITOR", &ws.name)
                        .map_err(|e| e.to_string())?;
                    copy_dir(path, &dst).map_err(|e| e.to_string())?;
                    ws.path = dst;
                    ws.copied = true;
                }
            }
            UiEdition::Bedrock => {
                let w = BedrockWorld::open(path).map_err(|e| e.to_string())?;
                ws.name = w
                    .level_name_txt
                    .clone()
                    .or_else(|| w.level.level_name().map(str::to_string))
                    .unwrap_or(ws.name);
                if copy_bedrock || path.is_file() {
                    let dst = settings
                        .new_workspace("EDITOR", &ws.name)
                        .map_err(|e| e.to_string())?;
                    w.save_to_folder(&dst).map_err(|e| e.to_string())?;
                    ws.path = dst;
                    ws.copied = true;
                    ws.bedrock = Some(BedrockWorld::open(&ws.path).map_err(|e| e.to_string())?);
                } else {
                    ws.bedrock = Some(w);
                }
            }
            _ => {
                ws.console = Some(ConsoleSave::open(path).map_err(|e| e.to_string())?);
            }
        }
        ws.map = MapView::new(ws.path.clone(), we);
        Ok(ws)
    }

    /// A pseudo-workspace holding one loose NBT file (More → Open/New NBT File).
    pub fn for_file(path: &Path, name: &str) -> Workspace {
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut expanded = HashSet::new();
        expanded.insert("single".to_string());
        Workspace {
            name: name.to_string(),
            edition: UiEdition::Java,
            original: dir.clone(),
            path: dir.clone(),
            copied: false,
            bedrock: None,
            console: None,
            docs: Vec::new(),
            active: None,
            map: MapView::new(dir, crate::world::Edition::Java),
            show_locator: false,
            world_changed: false,
            expanded,
            backed_up: HashSet::new(),
        }
    }

    pub fn is_single_file(&self) -> bool {
        self.expanded.contains("single")
    }

    /// Adds a document for a file that does not exist yet ("New NBT File").
    pub fn push_new_doc(&mut self, path: PathBuf, tree: NbtTree) {
        self.docs.push(Doc {
            title: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            source: DocSource::File(path),
            format: Some(Format::Java(Compression::Gzip)),
            tree,
            styled: None,
            seen: (0, 0.0),
        });
        self.active = Some(self.docs.len() - 1);
    }

    /// `EDITION` value for style extensions.
    pub fn style_edition(&self) -> i64 {
        match self.edition {
            UiEdition::Java => mctk_lua::style::edition::JAVA,
            UiEdition::Bedrock => mctk_lua::style::edition::BEDROCK,
            _ => mctk_lua::style::edition::CONSOLE,
        }
    }

    pub fn dirty(&self) -> bool {
        self.world_changed || self.docs.iter().any(|d| d.tree.dirty)
    }

    // -------------------------------------------------------------- sources

    pub fn level_source(&self) -> DocSource {
        match self.edition {
            UiEdition::Java | UiEdition::Bedrock => DocSource::File(self.path.join("level.dat")),
            _ => DocSource::ConsoleEntry("level.dat".into()),
        }
    }

    /// (label, source) for player data.
    pub fn players(&self) -> Vec<(String, DocSource)> {
        let mut out = Vec::new();
        match self.edition {
            UiEdition::Java => {
                for dir in ["players/data", "playerdata"] {
                    if let Ok(rd) = std::fs::read_dir(self.path.join(dir)) {
                        for e in rd.flatten() {
                            let p = e.path();
                            if p.extension().is_some_and(|x| x == "dat") {
                                out.push((
                                    p.file_stem()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string(),
                                    DocSource::File(p),
                                ));
                            }
                        }
                    }
                }
            }
            UiEdition::Bedrock => {
                if let Some(w) = &self.bedrock {
                    for k in w.db.entries.keys() {
                        if k == b"~local_player" || k.starts_with(b"player_") {
                            out.push((
                                String::from_utf8_lossy(k).to_string(),
                                DocSource::BedrockKey(k.clone()),
                            ));
                        }
                    }
                }
            }
            _ => {
                if let Some(s) = &self.console {
                    for e in &s.archive.entries {
                        let base = e.name.rsplit('/').next().unwrap_or(&e.name);
                        if e.name.ends_with(".dat")
                            && (e.name.starts_with("players/") || base.starts_with("P_"))
                        {
                            out.push((base.to_string(), DocSource::ConsoleEntry(e.name.clone())));
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn maps(&self) -> Vec<(String, DocSource)> {
        let mut out = Vec::new();
        match self.edition {
            UiEdition::Java => {
                let mut stack = vec![self.path.join("data")];
                while let Some(d) = stack.pop() {
                    for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                        let p = e.path();
                        let n = e.file_name().to_string_lossy().to_string();
                        if p.is_dir() {
                            stack.push(p);
                        } else if n.starts_with("map_") && n.ends_with(".dat") {
                            out.push((n.trim_end_matches(".dat").to_string(), DocSource::File(p)));
                        }
                    }
                }
            }
            UiEdition::Bedrock => {
                if let Some(w) = &self.bedrock {
                    for k in w.db.entries.keys().filter(|k| k.starts_with(b"map_")) {
                        out.push((
                            String::from_utf8_lossy(k).to_string(),
                            DocSource::BedrockKey(k.clone()),
                        ));
                    }
                }
            }
            _ => {
                if let Some(s) = &self.console {
                    for e in s
                        .archive
                        .entries
                        .iter()
                        .filter(|e| e.name.starts_with("data/map_"))
                    {
                        out.push((
                            e.name.trim_start_matches("data/").to_string(),
                            DocSource::ConsoleEntry(e.name.clone()),
                        ));
                    }
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The source for the chunk at (x, z) in the map's current dimension.
    pub fn chunk_source(&self, x: i32, z: i32) -> Option<DocSource> {
        let dim = self.map.dimension.as_str();
        match self.edition {
            UiEdition::Java => {
                let w = mctk_java::JavaWorld::open(&self.path).ok()?;
                let d = w.dimension(dim)?;
                let region = d
                    .region_dir()
                    .join(ChunkPos::new(x, z).region().file_name());
                region
                    .is_file()
                    .then_some(DocSource::JavaChunk { region, x, z })
            }
            UiEdition::Bedrock => {
                let id = match dim {
                    "minecraft:the_nether" => 1,
                    "minecraft:the_end" => 2,
                    _ => 0,
                };
                Some(DocSource::BedrockChunk { dim: id, x, z })
            }
            _ => {
                let s = self.console.as_ref()?;
                let want = match dim {
                    "minecraft:the_nether" => -1,
                    "minecraft:the_end" => 1,
                    _ => 0,
                };
                let entry = s
                    .archive
                    .regions()
                    .into_iter()
                    .find(|(d, rx, rz, _)| {
                        *d == want && *rx == x.div_euclid(32) && *rz == z.div_euclid(32)
                    })?
                    .3
                    .name
                    .clone();
                Some(DocSource::ConsoleChunk { entry, x, z })
            }
        }
    }

    // -------------------------------------------------------------- documents

    fn load(&self, source: &DocSource) -> Result<(String, Option<Format>, NamedTag, bool), String> {
        match source {
            DocSource::File(p) => {
                let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
                let (f, root) =
                    nbt_view::load(&bytes).map_err(|e| format!("not an NBT file ({e})"))?;
                Ok((
                    p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    Some(f),
                    root,
                    false,
                ))
            }
            DocSource::JavaChunk { region, x, z } => {
                let r = Region::open(region).map_err(|e| e.to_string())?;
                let (lx, lz) = ChunkPos::new(*x, *z).local();
                let root = r
                    .read_chunk(lx, lz)
                    .map_err(|e| e.to_string())?
                    .ok_or("that chunk is not stored")?;
                Ok((format!("Chunk {x}, {z}"), None, root, false))
            }
            DocSource::BedrockKey(k) => {
                let w = self.bedrock.as_ref().ok_or("world not loaded")?;
                let v = w.db.get(k).ok_or("record not found")?;
                let root = bedrock_nbt(v).ok_or("record is not NBT")?;
                Ok((
                    String::from_utf8_lossy(k).to_string(),
                    Some(Format::Bedrock(None)),
                    root,
                    false,
                ))
            }
            DocSource::BedrockChunk { dim, x, z } => {
                let w = self.bedrock.as_ref().ok_or("world not loaded")?;
                let d = mctk_bedrock::Dimension::from_id(*dim).ok_or("bad dimension")?;
                let mut root = Compound::new();
                let mut push = |name: &str, bytes: Option<&[u8]>| {
                    let mut list = List::new(TagType::Compound);
                    let mut s = bytes.unwrap_or_default();
                    while !s.is_empty() {
                        match mctk_nbt::read_root(&mut s, Flavor::Bedrock) {
                            Ok(t) => {
                                let _ = list.push(t.tag);
                            }
                            Err(_) => break,
                        }
                    }
                    root.insert(name.into(), Tag::List(list));
                };
                let key = |tag| {
                    ChunkKey {
                        x: *x,
                        z: *z,
                        dimension: d,
                        tag,
                        subchunk: None,
                    }
                    .encode()
                };
                push("BlockEntities", w.db.get(&key(ChunkTag::BlockEntity)));
                push("PendingTicks", w.db.get(&key(ChunkTag::PendingTicks)));
                let digp = Key::ActorDigest {
                    x: *x,
                    z: *z,
                    dimension: d,
                }
                .encode();
                let mut actors = List::new(TagType::Compound);
                for uid in w.db.get(&digp).unwrap_or_default().chunks_exact(8) {
                    if let Some(t) =
                        w.db.get(&Key::Actor(uid.to_vec()).encode())
                            .and_then(bedrock_nbt)
                    {
                        let _ = actors.push(t.tag);
                    }
                }
                root.insert("Entities".into(), Tag::List(actors));
                Ok((
                    format!("Chunk {x}, {z}"),
                    None,
                    NamedTag {
                        name: String::new(),
                        tag: Tag::Compound(root),
                    },
                    true,
                ))
            }
            DocSource::ConsoleEntry(name) => {
                let s = self.console.as_ref().ok_or("save not loaded")?;
                let e = s.archive.get(name).ok_or("file not in the save")?;
                let (c, raw) = mctk_nbt::decompress(&e.data).map_err(|e| e.to_string())?;
                let root = mctk_nbt::from_bytes(&raw, Flavor::Java)
                    .map_err(|e| format!("not an NBT file ({e})"))?;
                Ok((
                    name.rsplit('/').next().unwrap_or(name).to_string(),
                    Some(Format::Java(c)),
                    root,
                    false,
                ))
            }
            DocSource::ConsoleChunk { entry, x, z } => {
                let s = self.console.as_ref().ok_or("save not loaded")?;
                let e = s.archive.get(entry).ok_or("region not in the save")?;
                let r = ConsoleRegion::parse(e.data.clone()).map_err(|e| e.to_string())?;
                let raw = r
                    .raw_chunk(x.rem_euclid(32) as usize, z.rem_euclid(32) as usize)
                    .map_err(|e| e.to_string())?
                    .ok_or("that chunk is not stored")?;
                Ok((
                    format!("Chunk {x}, {z}"),
                    None,
                    raw.nbt().map_err(|e| e.to_string())?,
                    false,
                ))
            }
        }
    }

    /// Opens (or focuses) a document tab.
    pub fn open_doc(&mut self, source: DocSource) -> Result<(), String> {
        if let Some(i) = self.docs.iter().position(|d| d.source == source) {
            self.active = Some(i);
            self.show_locator = false;
            return Ok(());
        }
        let (title, format, root, read_only) = self.load(&source)?;
        let mut tree = NbtTree::new(root);
        tree.read_only = read_only;
        self.docs.push(Doc {
            title,
            source,
            format,
            tree,
            styled: None,
            seen: (0, 0.0),
        });
        self.active = Some(self.docs.len() - 1);
        self.show_locator = false;
        Ok(())
    }

    pub fn close_doc(&mut self, i: usize) {
        self.docs.remove(i);
        self.active = match self.active {
            Some(a) if a > i => Some(a - 1),
            Some(a) if a == i => (!self.docs.is_empty()).then(|| i.min(self.docs.len() - 1)),
            other => other,
        };
    }

    fn backup_file(&mut self, path: &Path) -> std::io::Result<()> {
        if !self.copied && path.is_file() && self.backed_up.insert(path.to_path_buf()) {
            std::fs::copy(path, backup_name(path, "_MCTBackup"))?;
        }
        Ok(())
    }

    /// Writes every changed document, then the world (back to the original if copied).
    pub fn save(&mut self) -> Result<String, String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as u32);
        let mut written = 0;
        let mut bedrock_db_changed = false;
        let mut console_changed = false;
        let dirty: Vec<usize> = (0..self.docs.len())
            .filter(|&i| self.docs[i].tree.dirty && !self.docs[i].tree.read_only)
            .collect();
        for i in dirty {
            let (source, format, root) = {
                let d = &self.docs[i];
                (d.source.clone(), d.format, d.tree.root.clone())
            };
            match source {
                DocSource::File(p) => {
                    let bytes =
                        nbt_view::encode(format.unwrap_or(Format::Java(Compression::Gzip)), &root)?;
                    self.backup_file(&p).map_err(|e| e.to_string())?;
                    std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
                }
                DocSource::JavaChunk { region, x, z } => {
                    self.backup_file(&region).map_err(|e| e.to_string())?;
                    let r = Region::open(&region).map_err(|e| e.to_string())?;
                    let mut b = RegionBuilder::from_region(&r).map_err(|e| e.to_string())?;
                    let pos = ChunkPos::new(x, z);
                    let (lx, lz) = pos.local();
                    b.set(
                        lx,
                        lz,
                        Some(
                            RawChunk::encode(&root, ChunkCompression::Zlib, now)
                                .map_err(|e| e.to_string())?,
                        ),
                    );
                    b.write(region.parent().unwrap_or(Path::new(".")), pos.region())
                        .map_err(|e| e.to_string())?;
                }
                DocSource::BedrockKey(k) => {
                    let bytes =
                        mctk_nbt::to_bytes(&root, Flavor::Bedrock).map_err(|e| e.to_string())?;
                    self.bedrock
                        .as_mut()
                        .ok_or("world not loaded")?
                        .db
                        .entries
                        .insert(k, bytes);
                    bedrock_db_changed = true;
                }
                DocSource::BedrockChunk { .. } => {}
                DocSource::ConsoleEntry(name) => {
                    let Some(Format::Java(c)) = format else {
                        continue;
                    };
                    let raw = mctk_nbt::to_bytes(&root, Flavor::Java).map_err(|e| e.to_string())?;
                    let data = nbt_view::compress(c, raw)?;
                    let s = self.console.as_mut().ok_or("save not loaded")?;
                    if let Some(e) = s.archive.entries.iter_mut().find(|e| e.name == name) {
                        e.data = data;
                    }
                    console_changed = true;
                }
                DocSource::ConsoleChunk { entry, x, z } => {
                    let s = self.console.as_mut().ok_or("save not loaded")?;
                    let codec = s.platform.chunk_codec();
                    let e = s
                        .archive
                        .entries
                        .iter_mut()
                        .find(|e| e.name == entry)
                        .ok_or("region missing")?;
                    let r = ConsoleRegion::parse(e.data.clone()).map_err(|e| e.to_string())?;
                    let mut b = ConsoleRegionBuilder::default();
                    for (lx, lz) in r.chunks().collect::<Vec<_>>() {
                        b.set(lx, lz, r.raw_chunk(lx, lz).map_err(|e| e.to_string())?);
                    }
                    let (lx, lz) = (x.rem_euclid(32) as usize, z.rem_euclid(32) as usize);
                    b.set(
                        lx,
                        lz,
                        Some(
                            RawConsoleChunk::encode(&root, codec, now)
                                .map_err(|e| e.to_string())?,
                        ),
                    );
                    e.data = b.build().map_err(|e| e.to_string())?;
                    console_changed = true;
                }
            }
            self.docs[i].tree.dirty = false;
            written += 1;
        }
        if bedrock_db_changed {
            let w = self.bedrock.as_ref().ok_or("world not loaded")?;
            let tmp = self.path.join("db_mctk_new");
            let _ = std::fs::remove_dir_all(&tmp);
            mctk_leveldb::write_db(&tmp, &w.db.entries, mctk_leveldb::WriteOptions::default())
                .map_err(|e| e.to_string())?;
            let old = self.path.join("db");
            if !self.copied {
                zip_dir(&old, &backup_name(&self.path, "_db_MCTBackup.zip"))
                    .map_err(|e| e.to_string())?;
            }
            std::fs::remove_dir_all(&old).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &old).map_err(|e| e.to_string())?;
        }
        if console_changed {
            let s = self.console.as_ref().ok_or("save not loaded")?;
            let bytes = s.encode().map_err(|e| e.to_string())?;
            let target = s.source.clone();
            if !target
                .with_file_name(format!(
                    "{}_MCTBackup",
                    target.file_name().unwrap_or_default().to_string_lossy()
                ))
                .exists()
            {
                std::fs::copy(&target, backup_name(&target, "_MCTBackup"))
                    .map_err(|e| e.to_string())?;
            }
            std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
        }
        if self.copied && (written > 0 || self.world_changed) {
            self.sync_back()?;
        }
        self.world_changed = false;
        Ok(if written == 0 {
            "Nothing to save".into()
        } else {
            format!("Saved {written} file(s)")
        })
    }

    /// Writes the working copy over the original, keeping a backup of the original.
    pub fn sync_back(&mut self) -> Result<(), String> {
        let e = |e: std::io::Error| e.to_string();
        if self.original.is_dir() {
            zip_dir(
                &self.original,
                &backup_name(&self.original, "_MCTBackup.zip"),
            )
            .map_err(e)?;
            std::fs::remove_dir_all(&self.original).map_err(e)?;
            copy_dir(&self.path, &self.original).map_err(e)?;
        } else if self.edition == UiEdition::Bedrock {
            // .mcworld: re-pack.
            std::fs::copy(&self.original, backup_name(&self.original, "_MCTBackup")).map_err(e)?;
            zip_dir(&self.path, &self.original).map_err(e)?;
        } else {
            std::fs::copy(&self.original, backup_name(&self.original, "_MCTBackup")).map_err(e)?;
            std::fs::copy(&self.path, &self.original).map_err(e)?;
        }
        Ok(())
    }
}

/// Region files list their chunks; everything else is a plain file.
pub fn region_chunks(path: &Path) -> Vec<(i32, i32)> {
    let Ok(r) = Region::open(path) else {
        return Vec::new();
    };
    let Some(pos) = r.position() else {
        return Vec::new();
    };
    r.chunks()
        .map(|(lx, lz)| {
            let c = pos.chunk(lx, lz);
            (c.x, c.z)
        })
        .collect()
}

/// Chunks stored in a console region entry, as absolute chunk coordinates.
pub fn console_region_chunks(save: &ConsoleSave, entry: &str) -> Vec<(i32, i32)> {
    let Some((_, rx, rz, e)) = save
        .archive
        .regions()
        .into_iter()
        .find(|r| r.3.name == entry)
    else {
        return Vec::new();
    };
    let Ok(r) = ConsoleRegion::parse(e.data.clone()) else {
        return Vec::new();
    };
    r.chunks()
        .map(|(lx, lz)| (rx * 32 + lx as i32, rz * 32 + lz as i32))
        .collect()
}
