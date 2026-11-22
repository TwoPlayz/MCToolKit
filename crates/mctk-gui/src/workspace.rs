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

use eframe::egui::{Rect, Vec2};

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
    /// A Bedrock chunk's NBT records gathered in one tree (block entities, pending ticks,
    /// entities), each saved back to where it came from.
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

/// One change to an entity (see `EditTool::Change`).
#[derive(Clone, Debug, PartialEq)]
pub enum EntityChange {
    /// Which way it faces, in degrees.
    Yaw(f32),
    /// Another kind of mob (a Java id).
    Retype(String),
    /// Puts a new mob (a Java id) on it, to ride it (a rider carrying a rider).
    AddRider(String),
    /// Any change the mob menus make to a mob's own data (a villager's job...).
    Mob(crate::mob_edit::MobEdit),
    /// Puts an item (a Java id) on a slot of an armor stand or mob, or empties it.
    #[cfg_attr(not(test), allow(dead_code))]
    Equip(crate::mob_edit::Slot, Option<String>),
    /// The item in an item frame (or none).
    FrameItem(Option<String>),
    /// Turns the item in a frame an eighth of a turn.
    FrameTurn,
    /// Whether it is invisible.
    Invisible(bool),
    /// Empties every slot of an armor stand.
    #[cfg_attr(not(test), allow(dead_code))]
    ClearGear,
    /// A byte flag of the entity on or off (an armor stand's `Small`, `ShowArms`, `NoBasePlate`; a
    /// frame's `Fixed`).
    Flag(String, bool),
    /// An armor stand put in a pose (one of `tools::STAND_POSES`, by name).
    StandPose(String),
}

/// One of the World Explorer's editing tools.
#[derive(Clone, Debug, PartialEq)]
pub enum EditTool {
    /// Makes every block in the selected box air.
    DeleteBlocks,
    /// Removes the entities standing in the selected box.
    DeleteEntities,
    /// Deletes the selected chunks.
    DeleteChunks,
    /// Fills the box with a block, or a palette of blocks mixed evenly.
    Fill(Vec<mctk_java::BlockState>),
    /// In the box, makes the blocks named `from` the block `to` (keeping the properties both
    /// have when `keep` is set).
    Replace { from: Vec<String>, to: Vec<mctk_java::BlockState>, keep: bool },
    /// Clears the box after it was copied.
    Cut,
    /// Places the copied box with its lowest north-west corner at `origin`.
    Paste { origin: [i32; 3], place: crate::edit::transform::Placement, with_air: bool },
    /// Paints a ball (or cube) of a block (air erases) around a block.
    /// `stroke`: going on with the brush stroke before (dragged), one step to undo with it.
    /// `palette`: the blocks mixed evenly (one is a single block); `only`: what it may paint over.
    Brush { center: [i32; 3], radius: i32, sphere: bool, palette: Vec<mctk_java::BlockState>, only: crate::edit::tools::BrushOnly, stroke: bool },
    /// Removes the one entity (mob, item...) standing at a place (the one selected).
    DeleteEntity([f64; 3]),
    /// Makes one block another (air removes it).
    SetBlock([i32; 3], mctk_java::BlockState),
    /// Sets properties of one block (waterlogged...), keeping what it holds.
    BlockProps([i32; 3], Vec<(String, String)>),
    /// Paints a biome (a Java id) on the 4 x 4 x 4 block cells from `lo` to `hi` (inclusive, in cells). `stroke`: going on with the
    /// strokes before (dragged), one step to undo.
    BiomeBrush { lo: [i32; 3], hi: [i32; 3], biome: String, stroke: bool },
    /// Changes what a block holds in its block entity: a container's lock, a beacon's powers.
    BlockEntity([i32; 3], crate::mob_edit::MobEdit),
    /// Makes the entity standing at a place another kind (a Java id).
    RetypeEntity([f64; 3], String),
    /// Moves the entity standing at a place to another.
    MoveEntity([f64; 3], [f64; 3]),
    /// Moves one block, with what it holds, to another place (air is left behind).
    MoveBlock([i32; 3], [i32; 3]),
    /// Turns the entity standing at a place to face a direction (yaw in degrees). `stroke`: going
    /// on with a turn begun before (dragged), one step to undo with it.
    SetYaw { at: [f64; 3], yaw: f32, stroke: bool },
    /// Puts a new mob (a Java id) on the entity standing at a place, to ride it.
    Ride([f64; 3], String),
    /// Turns a box of blocks `turns` quarter turns clockwise about its middle, in place. `stroke`:
    /// going on with a turn begun before (dragged), one step to undo.
    TurnBox { from: ([i32; 3], [i32; 3]), turns: u8, stroke: bool },
    /// Sets properties of every block in a box that has them (facing...), keeping the rest.
    BoxProps { bounds: ([i32; 3], [i32; 3]), props: Vec<(String, String)> },
    /// Moves a box (what was copied from it): it is cleared, and placed at `origin` as Paste would.
    MoveBox { from: ([i32; 3], [i32; 3]), origin: [i32; 3], place: crate::edit::transform::Placement, with_air: bool },
    /// Puts a new mob (a Java id) at a place (Java worlds).
    PlaceMob { at: [f64; 3], id: String },
    /// Puts a lingering potion cloud at a place (Java worlds).
    PlaceCloud { at: [f64; 3], cloud: crate::cloud::Cloud },
    /// Changes the cloud standing at a place to this one. `stroke`: going on with changes made
    /// before to it (one step to undo).
    UpdateCloud { at: [f64; 3], cloud: crate::cloud::Cloud, stroke: bool },
    /// Puts the mob standing at the second place on the one at the first, to ride it (Java).
    MountMob { mount: [f64; 3], path: Vec<usize>, rider: [f64; 3] },
    /// Puts a thing that stands on its own (armor stand, item frame, minecart...) in a block.
    PlaceEntity { cell: [i32; 3], id: String, facing: Option<u8> },
    /// Changes the entity standing at `at` (or the rider in `seat` of the mount there).
    Change { at: [f64; 3], path: Vec<usize>, change: EntityChange },
    /// Takes the rider in `seat` (or every one) off the mount at `mount`; with `delete`, removes them.
    Unmount { mount: [f64; 3], path: Vec<usize>, seat: Option<usize>, delete: bool },
}

impl EditTool {
    pub fn label(&self) -> &'static str {
        match self {
            EditTool::DeleteBlocks => "Delete Blocks",
            EditTool::DeleteEntities => "Delete Entities",
            EditTool::DeleteChunks => "Delete Chunks",
            EditTool::Fill(_) => "Fill",
            EditTool::Replace { .. } => "Replace",
            EditTool::Cut => "Cut",
            EditTool::Paste { .. } => "Paste",
            EditTool::Brush { .. } => "Brush",
            EditTool::DeleteEntity(_) => "Delete Entity",
            EditTool::SetBlock(..) => "Change Block",
            EditTool::BlockProps(..) => "Change Block",
            EditTool::BlockEntity(..) => "Change Block",
            EditTool::BiomeBrush { .. } => "Paint Biome",
            EditTool::RetypeEntity(..) => "Change Entity",
            EditTool::MoveEntity(..) => "Move Entity",
            EditTool::MoveBlock(..) => "Move Block",
            EditTool::SetYaw { .. } => "Turn Entity",
            EditTool::Ride(..) => "Add Rider",
            EditTool::MoveBox { .. } => "Move",
            EditTool::TurnBox { .. } => "Turn",
            EditTool::BoxProps { .. } => "Change Blocks",
            EditTool::PlaceMob { .. } => "Place Mob",
            EditTool::PlaceCloud { .. } => "Place Potion Cloud",
            EditTool::UpdateCloud { .. } => "Change Potion Cloud",
            EditTool::MountMob { .. } => "Add Rider",
            EditTool::PlaceEntity { .. } => "Place",
            EditTool::Change { .. } => "Change Entity",
            EditTool::Unmount { delete: false, .. } => "Unmount",
            EditTool::Unmount { .. } => "Delete Rider",
        }
    }
}

/// The size of a file, or of everything in a folder.
fn dir_size(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => m.len(),
        Ok(_) => std::fs::read_dir(path).into_iter().flatten().flatten().map(|e| dir_size(&e.path())).sum(),
        Err(_) => 0,
    }
}

/// "12.3 MB", "640 KB".
fn human_size(bytes: u64) -> String {
    if bytes >= 1 << 20 { format!("{:.1} MB", bytes as f64 / (1 << 20) as f64) } else { format!("{} KB", bytes.div_ceil(1024)) }
}

/// The chunks a box of blocks covers.
fn box_chunks(lo: [i32; 3], hi: [i32; 3]) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for cz in lo[2].div_euclid(16)..=hi[2].div_euclid(16) {
        for cx in lo[0].div_euclid(16)..=hi[0].div_euclid(16) {
            v.push((cx, cz));
        }
    }
    v
}

/// The console dimension number for a dimension name (0, -1 the Nether, 1 the End).
fn console_dim(dim: &str) -> i32 {
    match dim {
        "minecraft:the_nether" => -1,
        "minecraft:the_end" => 1,
        _ => 0,
    }
}

/// A region file's position from its name (`r.-1.2.mca`).
fn region_pos(file: &Path) -> Option<mctk_anvil::RegionPos> {
    let name = file.file_name()?.to_str()?;
    let mut parts = name.strip_prefix("r.")?.strip_suffix(".mca")?.split('.');
    let x = parts.next()?.parse().ok()?;
    let z = parts.next()?.parse().ok()?;
    Some(mctk_anvil::RegionPos { x, z })
}

/// A number no other tab has had, so a tab keeps its identity when tabs are reordered.
/// The entities region file beside a Java chunk region file (`<dimension>/entities/r.x.z.mca` for
/// `<dimension>/region/r.x.z.mca`); `None` for a region file not in a `region` folder.
fn entities_file(region: &Path) -> Option<PathBuf> {
    let dir = region.parent()?;
    let name = region.file_name()?;
    (dir.file_name()? == "region").then(|| dir.parent().unwrap_or(Path::new(".")).join("entities").join(name))
}

/// A Java chunk's entities, kept apart since 1.17 (in `entities/`): their list, empty when none
/// are saved (so some can be added). `None` for chunks from before, which hold their own.
fn chunk_entities(region: &Path, x: i32, z: i32, chunk: &NamedTag) -> Option<Tag> {
    // 1.17 (data version 2724) moved them out.
    let version = chunk.tag.get("DataVersion").and_then(Tag::as_i64).unwrap_or(0);
    if version < 2724 {
        return None;
    }
    let file = entities_file(region)?;
    let (lx, lz) = ChunkPos::new(x, z).local();
    let stored = Region::open(&file).ok().and_then(|r| r.read_chunk(lx, lz).ok().flatten());
    Some(match stored.as_ref().and_then(|s| s.tag.get("Entities")) {
        Some(list @ Tag::List(_)) => list.clone(),
        _ => Tag::List(List::new(TagType::Compound)),
    })
}

/// A short name for a file or record, for messages.
pub fn source_name(source: &DocSource) -> String {
    match source {
        DocSource::File(p) => p.file_name().unwrap_or_default().to_string_lossy().to_string(),
        DocSource::ConsoleEntry(n) => n.rsplit(['/', '\\']).next().unwrap_or(n).to_string(),
        DocSource::BedrockKey(k) => String::from_utf8_lossy(k).to_string(),
        _ => "the item".to_string(),
    }
}

fn next_uid() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct Doc {
    pub uid: u64,
    pub title: String,
    pub source: DocSource,
    format: Option<Format>,
    pub tree: NbtTree,
    /// Tree generation the styles were computed for.
    pub styled: Option<u64>,
    /// Last generation seen, and when it changed (styles wait for edits to settle).
    pub seen: (u64, f64),
    /// The tree generation the history clock last saw, and when (on `Workspace::op_clock`) the
    /// tree last changed, so Undo can tell what happened most recently.
    clock_seen: u64,
    stamp: u64,
}

/// A change made to the files on the left, kept so the side panel's Undo and Redo can take it back.
#[derive(Clone, PartialEq)]
pub enum FileOp {
    Delete(DocSource),
    Rename(PathBuf, PathBuf),
}

impl Doc {
    /// `FILE_TYPE` flags for style extensions.
    pub fn file_type(&self) -> u32 {
        use crate::info::file_type::*;
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
    /// Where the camera's biome was last read (cell and edits generation), see `update_sky_biome`.
    sky_key: Option<([i32; 3], u64)>,
    pub show_explorer: bool,
    /// The working copy differs from the original (e.g. after pruning).
    pub world_changed: bool,
    /// A Bedrock world's database is written again, compacted, on the next save (giving back
    /// the space the game leaves behind when it deletes records), even with nothing else changed.
    pub optimize_on_save: bool,
    pub expanded: HashSet<String>,
    /// The file or folder picked in the side panel (the target of its shortcuts).
    pub selected_file: Option<PathBuf>,
    /// A file or folder cut (`true`) or copied (`false`) in the side panel.
    pub file_clip: Option<(Vec<PathBuf>, bool)>,
    /// Every file or folder picked in the side panel (the one in `selected_file` is among them).
    pub selected_files: Vec<PathBuf>,
    /// The side panel's rows as drawn this frame and last frame, for range and drag selection.
    pub file_rows: Vec<(PathBuf, Rect)>,
    pub file_rows_prev: Vec<(PathBuf, Rect)>,
    /// A drag-select over the side panel in progress.
    pub file_marquee: Option<FileMarquee>,
    /// The file being renamed, and the name typed so far.
    pub renaming_file: Option<(PathBuf, String)>,
    /// Files renamed since the last save (the open ones with unsaved edits count too).
    pub edited_files: Vec<PathBuf>,
    /// Renames since the last save as (old, new), and the ones taken back (to redo).
    pub rename_log: Vec<(PathBuf, PathBuf)>,
    pub rename_redo: Vec<(PathBuf, PathBuf)>,
    /// File changes (deletes, renames) in the order made, each with its place on `op_clock`, and
    /// the ones undone, for the Undo and Redo buttons.
    pub ops: Vec<(u64, FileOp)>,
    pub ops_redo: Vec<(u64, FileOp)>,
    op_clock: u64,
    /// The last Undo took back a file change (so Redo brings that back first).
    last_undone_file: bool,
    /// Waiting for the user to confirm deleting this.
    pub pending_delete: Vec<DocSource>,
    /// Files and records deleted in the side panel but kept (red, faint) until the world is saved.
    pub gone: Vec<DocSource>,
    /// Files pasted since the last save, marked green in the side panel.
    pub fresh_files: Vec<PathBuf>,
    /// A tab with unsaved edits the user asked to close (its `uid`); waiting for their answer.
    pub pending_close: Option<u64>,
    /// How far the tab strip is scrolled, and the tab and tab count it last made room for.
    pub tab_scroll: f32,
    pub tab_seen: (Option<u64>, usize),
    /// A Bedrock database record or console archive entry picked in the side panel.
    pub selected_entry: Option<DocSource>,
    /// The rows that are not files on disk (console entries, Bedrock records) picked in the side
    /// panel, and where each was drawn this and last frame (for range and drag selection).
    pub selected_entries: Vec<DocSource>,
    pub entry_rows: Vec<(DocSource, Rect)>,
    pub entry_rows_prev: Vec<(DocSource, Rect)>,
    /// Deletions made together are one step to undo.
    batch_open: bool,
    /// Records or entries were removed and the database or archive must be rewritten on save.
    db_dirty: bool,
    console_dirty: bool,
    /// What the last file operation did, for the status line.
    pub file_note: String,
    backed_up: HashSet<PathBuf>,
    /// A file or record read for its right-click menu (a player's effects), and what it holds.
    pub peeked: Option<(DocSource, Option<mctk_nbt::Compound>)>,
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

fn file_label(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().to_string()
}

fn copy_any(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        copy_dir(src, dst)
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// `name` in `dir`, or "name - Copy", "name - Copy (2)", … when that is taken.
fn unused_name(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let as_path = Path::new(name);
    let stem = as_path.file_stem().unwrap_or(name).to_string_lossy().to_string();
    let ext = as_path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 1.. {
        let tag = if n == 1 { " - Copy".to_string() } else { format!(" - Copy ({n})") };
        let candidate = dir.join(format!("{stem}{tag}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
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
            sky_key: None,
            show_explorer: false,
            world_changed: false,
            optimize_on_save: false,
            expanded: HashSet::from(["level".to_string()]),
            selected_file: None,
            file_clip: None,
            selected_files: Vec::new(),
            file_rows: Vec::new(),
            file_rows_prev: Vec::new(),
            file_marquee: None,
            renaming_file: None,
            edited_files: Vec::new(),
            rename_log: Vec::new(),
            rename_redo: Vec::new(),
            ops: Vec::new(),
            ops_redo: Vec::new(),
            op_clock: 0,
            last_undone_file: false,
            pending_delete: Vec::new(),
            gone: Vec::new(),
            fresh_files: Vec::new(),
            pending_close: None,
            tab_scroll: 0.0,
            tab_seen: (None, 0),
            selected_entry: None,
            selected_entries: Vec::new(),
            entry_rows: Vec::new(),
            entry_rows_prev: Vec::new(),
            batch_open: false,
            db_dirty: false,
            console_dirty: false,
            file_note: String::new(),
            peeked: None,
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
            sky_key: None,
            show_explorer: false,
            world_changed: false,
            optimize_on_save: false,
            expanded,
            selected_file: None,
            file_clip: None,
            selected_files: Vec::new(),
            file_rows: Vec::new(),
            file_rows_prev: Vec::new(),
            file_marquee: None,
            renaming_file: None,
            edited_files: Vec::new(),
            rename_log: Vec::new(),
            rename_redo: Vec::new(),
            ops: Vec::new(),
            ops_redo: Vec::new(),
            op_clock: 0,
            last_undone_file: false,
            pending_delete: Vec::new(),
            gone: Vec::new(),
            fresh_files: Vec::new(),
            pending_close: None,
            tab_scroll: 0.0,
            tab_seen: (None, 0),
            selected_entry: None,
            selected_entries: Vec::new(),
            entry_rows: Vec::new(),
            entry_rows_prev: Vec::new(),
            batch_open: false,
            db_dirty: false,
            console_dirty: false,
            file_note: String::new(),
            peeked: None,
            backed_up: HashSet::new(),
        }
    }

    pub fn is_single_file(&self) -> bool {
        self.expanded.contains("single")
    }

    /// Adds a document for a file that does not exist yet ("New NBT File").
    pub fn push_new_doc(&mut self, path: PathBuf, tree: NbtTree) {
        self.docs.push(Doc {
            uid: next_uid(),
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
            clock_seen: 0,
            stamp: 0,
        });
        self.active = Some(self.docs.len() - 1);
    }

    /// `EDITION` value for style extensions.
    pub fn style_edition(&self) -> i64 {
        match self.edition {
            UiEdition::Java => crate::info::edition::JAVA,
            UiEdition::Bedrock => crate::info::edition::BEDROCK,
            _ => crate::info::edition::CONSOLE,
        }
    }

    pub fn dirty(&self) -> bool {
        self.world_changed || !self.gone.is_empty() || self.docs.iter().any(|d| d.tree.dirty) || self.has_edits() || self.optimize_on_save
    }

    /// Whether the World Explorer's tools changed anything not saved yet.
    pub fn has_edits(&self) -> bool {
        self.map.edits.read().is_ok_and(|l| l.is_dirty())
    }

    /// Whether a file or record is marked deleted (or sits inside a deleted folder).
    pub fn is_gone(&self, source: &DocSource) -> bool {
        self.gone.iter().any(|g| match (g, source) {
            (DocSource::File(folder), DocSource::File(p)) => p.starts_with(folder),
            _ => g == source,
        })
    }

    /// Marks a file or record deleted; it goes when the world is saved.
    pub fn mark_deleted(&mut self, source: DocSource) {
        if self.is_gone(&source) {
            return;
        }
        if let DocSource::File(folder) = &source {
            self.gone.retain(|g| !matches!(g, DocSource::File(p) if p.starts_with(folder)));
        }
        self.gone.push(source.clone());
        self.record(FileOp::Delete(source));
    }

    /// Changes made between `begin_batch` and `end_batch` are one step for Undo and Redo.
    pub fn begin_batch(&mut self) {
        self.op_clock += 1;
        self.batch_open = true;
    }

    pub fn end_batch(&mut self) {
        self.batch_open = false;
    }

    /// Remembers a file change for the Undo button; a new change ends any Redo.
    fn record(&mut self, op: FileOp) {
        if !self.batch_open {
            self.op_clock += 1;
        }
        self.ops.push((self.op_clock, op));
        self.ops_redo.clear();
    }

    /// The deleted entry that covers `source` (itself, or the folder it is in).
    pub fn gone_root(&self, source: &DocSource) -> Option<DocSource> {
        self.gone
            .iter()
            .find(|g| match (g, source) {
                (DocSource::File(folder), DocSource::File(p)) => p.starts_with(folder),
                _ => *g == source,
            })
            .cloned()
    }

    pub fn restore_deleted(&mut self, source: &DocSource) {
        self.gone.retain(|g| g != source);
        self.ops.retain(|(_, op)| *op != FileOp::Delete(source.clone()));
    }

    /// Whether a file is changed: renamed since the last save, or open with unsaved edits.
    pub fn is_edited(&self, path: &Path) -> bool {
        self.edited_files.iter().any(|f| f == path)
            || self
                .docs
                .iter()
                .any(|d| d.tree.dirty && matches!(&d.source, DocSource::File(p) if p == path))
    }

    /// Renames a file or folder in place. Open tabs, the selection and the open folders follow.
    pub fn rename_file(&mut self, path: &Path, new_name: &str) -> Result<String, String> {
        let new_name = new_name.trim();
        if new_name.is_empty() || new_name.contains(['/', '\\']) {
            return Err("That name is empty or not allowed".into());
        }
        let Some(parent) = path.parent() else {
            return Err("That cannot be renamed".into());
        };
        let target = parent.join(new_name);
        if target == path {
            return Ok(String::new());
        }
        if target.exists() {
            return Err("That name is already used".into());
        }
        self.move_to(path, &target)?;
        self.record(FileOp::Rename(path.to_path_buf(), target.clone()));
        self.rename_log.push((path.to_path_buf(), target));
        self.rename_redo.clear();
        Ok(format!("Renamed to {new_name}"))
    }

    /// Moves the file or folder and carries everything that points at it along.
    fn move_to(&mut self, path: &Path, target: &Path) -> Result<(), String> {
        let new_name = target.file_name().unwrap_or_default().to_string_lossy().to_string();
        std::fs::rename(path, target).map_err(|e| e.to_string())?;
        let target = target.to_path_buf();
        let moved = |p: &Path| p.strip_prefix(path).ok().map(|rest| target.join(rest));
        for d in &mut self.docs {
            if let DocSource::File(p) = &d.source {
                if let Some(np) = moved(p) {
                    if np == target {
                        d.title = new_name.to_string();
                    }
                    d.source = DocSource::File(np);
                }
            }
        }
        for list in [&mut self.selected_files, &mut self.fresh_files, &mut self.edited_files] {
            for p in list.iter_mut() {
                if let Some(np) = moved(p) {
                    *p = np;
                }
            }
        }
        if let Some(np) = self.selected_file.as_deref().and_then(moved) {
            self.selected_file = Some(np);
        }
        // Folders that were open stay open under their new name.
        let (old_key, new_key) = (path.display().to_string(), target.display().to_string());
        let reopened: Vec<String> = self
            .expanded
            .iter()
            .filter(|k| k.starts_with(&old_key))
            .cloned()
            .collect();
        for k in reopened {
            self.expanded.remove(&k);
            self.expanded.insert(format!("{new_key}{}", &k[old_key.len()..]));
        }
        if !self.is_fresh(&target) {
            self.edited_files.push(target.clone());
        }
        self.world_changed = true;
        Ok(())
    }

    /// What the side panel's Undo and Redo do for a file: the open tab's own history first, then
    /// a rename.
    pub fn file_can_undo(&self, path: &Path) -> bool {
        self.doc_at(path).is_some_and(|d| d.tree.can_undo())
            || self.rename_log.iter().any(|(_, new)| new == path)
    }

    pub fn file_can_redo(&self, path: &Path) -> bool {
        self.doc_at(path).is_some_and(|d| d.tree.can_redo())
            || self.rename_redo.iter().any(|(old, _)| old == path)
    }

    fn doc_at(&self, path: &Path) -> Option<&Doc> {
        self.docs
            .iter()
            .find(|d| matches!(&d.source, DocSource::File(p) if p == path))
    }

    pub fn file_undo(&mut self, path: &Path) -> Result<(), String> {
        if let Some(d) = self
            .docs
            .iter_mut()
            .find(|d| matches!(&d.source, DocSource::File(p) if p == path) && d.tree.can_undo())
        {
            d.tree.undo();
            return Ok(());
        }
        if let Some(i) = self.rename_log.iter().rposition(|(_, new)| new == path) {
            let (old, new) = self.rename_log.remove(i);
            self.move_to(&new, &old)?;
            self.edited_files.retain(|f| *f != old);
            self.ops.retain(|(_, op)| *op != FileOp::Rename(old.clone(), new.clone()));
            self.rename_redo.push((old, new));
        }
        Ok(())
    }

    pub fn file_redo(&mut self, path: &Path) -> Result<(), String> {
        if let Some(d) = self
            .docs
            .iter_mut()
            .find(|d| matches!(&d.source, DocSource::File(p) if p == path) && d.tree.can_redo())
        {
            d.tree.redo();
            return Ok(());
        }
        if let Some(i) = self.rename_redo.iter().rposition(|(old, _)| old == path) {
            let (old, new) = self.rename_redo.remove(i);
            self.move_to(&old, &new)?;
            self.rename_log.push((old, new));
        }
        Ok(())
    }

    /// Notes edits the open tabs have had since the last call, so Undo can order them against
    /// file changes. Call once a frame.
    pub fn sync_clock(&mut self) {
        for d in &mut self.docs {
            if d.tree.generation != d.clock_seen {
                d.clock_seen = d.tree.generation;
                self.op_clock += 1;
                d.stamp = self.op_clock;
            }
        }
    }

    fn history_doc(&self) -> Option<&Doc> {
        if self.show_explorer {
            return None;
        }
        self.active.and_then(|i| self.docs.get(i))
    }

    /// Whether the Undo button has anything to take back: the open tab's tag edits or a file
    /// change on the left.
    pub fn can_undo_any(&self) -> bool {
        self.edit_history(false) || self.history_doc().is_some_and(|d| d.tree.can_undo()) || !self.ops.is_empty()
    }

    pub fn can_redo_any(&self) -> bool {
        self.edit_history(true) || self.history_doc().is_some_and(|d| d.tree.can_redo()) || !self.ops_redo.is_empty()
    }

    /// In the World Explorer: whether its tools have a step to undo (or redo).
    fn edit_history(&self, redo: bool) -> bool {
        self.show_explorer && self.map.edits.read().is_ok_and(|l| if redo { l.can_redo() } else { l.can_undo() })
    }

    /// Takes back whichever happened last, a tag edit in the open tab or a file change (in the
    /// World Explorer, its tools' last step). Returns what was undone, to tell the user.
    pub fn undo_any(&mut self) -> Result<String, String> {
        if self.edit_history(false) {
            return Ok(self.undo_edit(false).unwrap_or_default());
        }
        let tag = self.history_doc().filter(|d| d.tree.can_undo()).map(|d| d.stamp);
        let file = self.ops.last().map(|(s, _)| *s);
        if file.is_some() && file > tag {
            let stamp = file.unwrap_or_default();
            self.last_undone_file = true;
            let mut notes = Vec::new();
            while self.ops.last().is_some_and(|(s, _)| *s == stamp) {
                let (stamp, op) = self.ops.pop().expect("an op");
                notes.push(match &op {
                    FileOp::Delete(source) => {
                        self.gone.retain(|g| g != source);
                        format!("Restored {}", source_name(source))
                    }
                    FileOp::Rename(old, new) => {
                        self.rename_log.retain(|(o, n)| !(o == old && n == new));
                        self.move_to(new, old)?;
                        self.edited_files.retain(|f| f != old);
                        self.rename_redo.push((old.clone(), new.clone()));
                        format!("Renamed back to {}", old.file_name().unwrap_or_default().to_string_lossy())
                    }
                });
                self.ops_redo.push((stamp, op));
            }
            return Ok(match notes.len() {
                1 => notes.remove(0),
                n => format!("Restored {n} items"),
            });
        }
        if let Some(i) = self.active.filter(|_| !self.show_explorer && tag.is_some()) {
            if let Some(d) = self.docs.get_mut(i) {
                d.tree.undo();
                self.last_undone_file = false;
                return Ok(std::mem::take(&mut d.tree.status));
            }
        }
        Ok(String::new())
    }

    pub fn redo_any(&mut self) -> Result<String, String> {
        if self.edit_history(true) {
            return Ok(self.undo_edit(true).unwrap_or_default());
        }
        let tag = self.history_doc().is_some_and(|d| d.tree.can_redo());
        let file = !self.ops_redo.is_empty();
        if file && (!tag || self.last_undone_file) {
            let stamp = self.ops_redo.last().map(|(s, _)| *s).unwrap_or_default();
            let mut notes = Vec::new();
            while self.ops_redo.last().is_some_and(|(s, _)| *s == stamp) {
                let (stamp, op) = self.ops_redo.pop().expect("an op");
                notes.push(match &op {
                    FileOp::Delete(source) => {
                        self.gone.push(source.clone());
                        format!("Deleted {} again", source_name(source))
                    }
                    FileOp::Rename(old, new) => {
                        self.rename_redo.retain(|(o, n)| !(o == old && n == new));
                        self.move_to(old, new)?;
                        self.rename_log.push((old.clone(), new.clone()));
                        format!("Renamed to {}", new.file_name().unwrap_or_default().to_string_lossy())
                    }
                });
                self.ops.push((stamp, op));
            }
            return Ok(match notes.len() {
                1 => notes.remove(0),
                n => format!("Deleted {n} items again"),
            });
        }
        if tag {
            if let Some(i) = self.active {
                if let Some(d) = self.docs.get_mut(i) {
                    d.tree.redo();
                    return Ok(std::mem::take(&mut d.tree.status));
                }
            }
        }
        Ok(String::new())
    }

    /// Whether a file was pasted since the last save (or is inside a folder that was).
    pub fn is_fresh(&self, path: &Path) -> bool {
        self.fresh_files.iter().any(|f| path.starts_with(f))
    }

    /// Carries out the deletions that were waiting for a save; tabs open on them close.
    fn apply_deletions(&mut self) -> Result<(), String> {
        for source in std::mem::take(&mut self.gone) {
            while let Some(i) = self.docs.iter().position(|d| match (&d.source, &source) {
                (DocSource::File(p), DocSource::File(folder)) => p.starts_with(folder),
                (a, b) => a == b,
            }) {
                self.close_doc(i);
            }
            self.delete_source(&source)?;
        }
        Ok(())
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

    /// Writes a Bedrock chunk's tags back to the records they were gathered from: its block
    /// entities and pending ticks to the chunk's own records, and each entity to its record
    /// (found by its `UniqueID` among those the chunk lists), the chunk's list made to match.
    /// An entity with no record yet goes in the chunk's old entity record when it has one, else
    /// gets a record of its own. One no longer there has its record removed.
    fn save_bedrock_chunk(&mut self, dim: i32, x: i32, z: i32, root: &NamedTag) -> Result<(), String> {
        let w = self.bedrock.as_mut().ok_or("world not loaded")?;
        let d = mctk_bedrock::Dimension::from_id(dim).ok_or("bad dimension")?;
        let key = |tag| ChunkKey { x, z, dimension: d, tag, subchunk: None }.encode();
        let list = |name: &str| -> Vec<Compound> {
            root.tag.get(name).and_then(Tag::as_list).map(|l| l.iter().filter_map(Tag::as_compound).cloned().collect()).unwrap_or_default()
        };
        let bytes = |c: &Compound| mctk_nbt::to_bytes(&NamedTag { name: String::new(), tag: Tag::Compound(c.clone()) }, Flavor::Bedrock).map_err(|e| e.to_string());
        let joined = |items: &[Compound]| -> Result<Vec<u8>, String> {
            let mut out = Vec::new();
            for c in items {
                out.extend(bytes(c)?);
            }
            Ok(out)
        };
        for (name, tag) in [("BlockEntities", ChunkTag::BlockEntity), ("PendingTicks", ChunkTag::PendingTicks)] {
            let items = list(name);
            if items.is_empty() {
                w.db.entries.remove(&key(tag));
            } else {
                w.db.entries.insert(key(tag), joined(&items)?);
            }
        }
        // The entities' own records, by the `UniqueID` each holds.
        let digp = Key::ActorDigest { x, z, dimension: d }.encode();
        let listed: Vec<Vec<u8>> = w.db.get(&digp).unwrap_or_default().chunks_exact(8).map(<[u8]>::to_vec).collect();
        let mut by_id: std::collections::HashMap<i64, Vec<u8>> = std::collections::HashMap::new();
        for uid in &listed {
            let id = w.db.get(&Key::Actor(uid.clone()).encode()).and_then(bedrock_nbt).and_then(|t| t.tag.get("UniqueID").and_then(Tag::as_i64));
            if let Some(id) = id {
                by_id.insert(id, uid.clone());
            }
        }
        let had_old = w.db.get(&key(ChunkTag::Entity)).is_some();
        let (mut kept, mut old) = (Vec::new(), Vec::new());
        for a in list("Entities") {
            let id = a.get("UniqueID").and_then(Tag::as_i64);
            let uid = match id.and_then(|i| by_id.get(&i).cloned()) {
                Some(uid) => uid,
                None if had_old => {
                    old.push(a);
                    continue;
                }
                // New here: a record of its own, named by its id.
                None => id.unwrap_or_else(|| kept.len() as i64 + 1).to_le_bytes().to_vec(),
            };
            w.db.entries.insert(Key::Actor(uid.clone()).encode(), bytes(&a)?);
            kept.push(uid);
        }
        for uid in listed.iter().filter(|u| !kept.contains(u)) {
            w.db.entries.remove(&Key::Actor(uid.clone()).encode());
        }
        if kept.is_empty() {
            w.db.entries.remove(&digp);
        } else {
            w.db.entries.insert(digp, kept.concat());
        }
        if had_old {
            if old.is_empty() {
                w.db.entries.remove(&key(ChunkTag::Entity));
            } else {
                w.db.entries.insert(key(ChunkTag::Entity), joined(&old)?);
            }
        }
        Ok(())
    }

    /// Before saving a Java chunk shown with its entities (see `chunk_entities`): the chunk
    /// without them, its `Entities` written to the entities file they came from (one made for it
    /// when there was none and it now has some).
    fn split_entities(&mut self, region: &Path, x: i32, z: i32, mut root: NamedTag, now: u32) -> Result<NamedTag, String> {
        let Some(file) = entities_file(region) else { return Ok(root) };
        let Some(Tag::List(list)) = root.tag.as_compound_mut().and_then(|c| c.shift_remove("Entities")) else { return Ok(root) };
        let pos = ChunkPos::new(x, z);
        let (lx, lz) = pos.local();
        let old = Region::open(&file).ok();
        let stored = old.as_ref().and_then(|r| r.read_chunk(lx, lz).ok().flatten());
        if stored.is_none() && list.is_empty() {
            return Ok(root);
        }
        let mut record = match stored {
            Some(NamedTag { tag: Tag::Compound(c), .. }) => c,
            _ => {
                let mut c = Compound::new();
                let version = root.tag.get("DataVersion").and_then(Tag::as_i64).unwrap_or(0) as i32;
                c.insert("DataVersion".into(), Tag::Int(version));
                c.insert("Position".into(), Tag::IntArray(vec![x, z]));
                c
            }
        };
        record.insert("Entities".into(), Tag::List(list));
        let record = NamedTag { name: String::new(), tag: Tag::Compound(record) };
        let mut b = match &old {
            Some(r) => {
                self.backup_file(&file).map_err(|e| e.to_string())?;
                RegionBuilder::from_region(r).map_err(|e| e.to_string())?
            }
            None => RegionBuilder::default(),
        };
        b.set(lx, lz, Some(RawChunk::encode(&record, ChunkCompression::Zlib, now).map_err(|e| e.to_string())?));
        let dir = file.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        b.write(dir, pos.region()).map_err(|e| e.to_string())?;
        Ok(root)
    }

    // -------------------------------------------------------------- documents

    fn load(&self, source: &DocSource) -> Result<(String, Option<Format>, NamedTag, bool), String> {
        // A chunk the World Explorer's tools changed opens once those changes are saved (the
        // tab would show it as it was).
        match source {
            DocSource::File(p) => {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
                let (f, root) = nbt_view::load(&bytes).map_err(|e| not_nbt(&name, &e.to_string()))?;
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
                let mut root = r.read_chunk(lx, lz).map_err(|e| e.to_string())?.ok_or("that chunk is not stored")?;
                // The same kind of file holds a chunk's entities (in `entities/`).
                let entities = region.parent().and_then(|d| d.file_name()).is_some_and(|n| n == "entities");
                let title = if entities { format!("Entities {x}, {z}") } else { format!("Chunk {x}, {z}") };
                // Since 1.17 a chunk's mobs, items and other entities are kept apart, in
                // `entities/`: shown here among the chunk's tags, as `Entities`, and put back
                // there on saving (see `split_entities`).
                if !entities {
                    if let Some(list) = chunk_entities(region, *x, *z, &root) {
                        if let Some(c) = root.tag.as_compound_mut() {
                            c.insert("Entities".into(), list);
                        }
                    }
                }
                Ok((title, None, root, false))
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
                let root = self.bedrock_chunk_root(*dim, *x, *z, false)?;
                // Editable: saving puts each part back (see `save_bedrock_chunk`).
                Ok((format!("Chunk {x}, {z}"), None, root, false))
            }
            DocSource::ConsoleEntry(name) => {
                let s = self.console.as_ref().ok_or("save not loaded")?;
                let e = s.archive.get(name).ok_or("file not in the save")?;
                let (c, raw) = mctk_nbt::decompress(&e.data).map_err(|e| e.to_string())?;
                let root = mctk_nbt::from_bytes(&raw, Flavor::Java)
                    .map_err(|e| not_nbt(name, &e.to_string()))?;
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

    /// A Bedrock chunk's records (block entities, ticks, entities) as one document root: as the database has them,
    /// or (`layered`) as the World Explorer's unsaved edits leave them.
    fn bedrock_chunk_root(&self, dim: i32, x: i32, z: i32, layered: bool) -> Result<NamedTag, String> {
        let w = self.bedrock.as_ref().ok_or("world not loaded")?;
        let d = mctk_bedrock::Dimension::from_id(dim).ok_or("bad dimension")?;
        let layer = if layered { self.map.edits.read().ok() } else { None };
        let get = |k: &Vec<u8>| -> Option<Vec<u8>> {
            if let Some(l) = &layer {
                if let Some(v) = l.bedrock.get(k) {
                    return v.clone();
                }
            }
            w.db.get(k).map(<[u8]>::to_vec)
        };
        let mut root = Compound::new();
        let mut push = |name: &str, bytes: Option<Vec<u8>>| {
            let mut list = List::new(TagType::Compound);
            let owned = bytes.unwrap_or_default();
            let mut s: &[u8] = &owned;
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
        let key = |tag| ChunkKey { x, z, dimension: d, tag, subchunk: None }.encode();
        push("BlockEntities", get(&key(ChunkTag::BlockEntity)));
        push("PendingTicks", get(&key(ChunkTag::PendingTicks)));
        let digp = Key::ActorDigest { x, z, dimension: d }.encode();
        let mut actors = List::new(TagType::Compound);
        for uid in get(&digp).unwrap_or_default().chunks_exact(8) {
            if let Some(t) = get(&Key::Actor(uid.to_vec()).encode()).as_deref().and_then(bedrock_nbt) {
                let _ = actors.push(t.tag);
            }
        }
        // Before 1.18.30 a chunk kept its entities in a record of its own.
        let owned = get(&key(ChunkTag::Entity)).unwrap_or_default();
        let mut s: &[u8] = &owned;
        while !s.is_empty() {
            match mctk_nbt::read_root(&mut s, Flavor::Bedrock) {
                Ok(t) => {
                    let _ = actors.push(t.tag);
                }
                Err(_) => break,
            }
        }
        root.insert("Entities".into(), Tag::List(actors));
        Ok(NamedTag { name: String::new(), tag: Tag::Compound(root) })
    }

    /// What a chunk tab would show with the World Explorer's unsaved changes in it (`None` when it has none
    /// there), in any edition: opened over the saved chunk, so each change shows as one in the tree.
    fn layered_root(&self, source: &DocSource) -> Option<NamedTag> {
        let id = self.tab_chunk(source)?;
        match source {
            DocSource::JavaChunk { region, x, z } => {
                let jc = self.layer_java(source)?;
                let mut root = jc.root.clone()?;
                // The World Explorer's own record of the entities, if it changed them (they are among the
                // chunk's tags here).
                let entities = region.parent().and_then(|d| d.file_name()).is_some_and(|n| n == "entities");
                if !entities {
                    let own = jc.entities.clone().map(|rec| rec.and_then(|r| r.tag.get("Entities").cloned()));
                    let list = match own {
                        Some(list) => list,
                        None => chunk_entities(region, *x, *z, &root),
                    };
                    if let (Some(list), Some(c)) = (list, root.tag.as_compound_mut()) {
                        c.insert("Entities".into(), list);
                    }
                }
                Some(root)
            }
            DocSource::ConsoleChunk { .. } => self.map.edits.read().ok()?.console.get(&id)?.clone(),
            DocSource::BedrockChunk { dim, x, z } => {
                if !self.map.edits.read().ok()?.has_chunk(&id) {
                    return None;
                }
                self.bedrock_chunk_root(*dim, *x, *z, true).ok()
            }
            _ => None,
        }
    }
    /// Opens (or focuses) a document tab.
    /// What a file or record holds at its root (as open, if it is), read once for a menu and
    /// kept until `peeked` is cleared.
    pub fn peek_root(&mut self, source: &DocSource) -> Option<mctk_nbt::Compound> {
        if let Some((s, c)) = &self.peeked {
            if s == source {
                return c.clone();
            }
        }
        let c = match self.docs.iter().find(|d| d.source == *source) {
            Some(d) => d.tree.root.tag.as_compound().cloned(),
            None => self.load(source).ok().and_then(|(_, _, root, _)| root.tag.as_compound().cloned()),
        };
        self.peeked = Some((source.clone(), c.clone()));
        c
    }

    pub fn open_doc(&mut self, source: DocSource) -> Result<(), String> {
        // Something marked deleted is only waiting to go; it is not opened.
        if self.is_gone(&source) {
            return Ok(());
        }
        if let Some(i) = self.docs.iter().position(|d| d.source == source) {
            self.active = Some(i);
            self.show_explorer = false;
            return Ok(());
        }
        let layered = self.layered_root(&source);
        let (title, format, root, read_only) = self.load(&source)?;
        let mut tree = NbtTree::new(root);
        tree.read_only = read_only;
        // A chunk with the World Explorer's unsaved changes opens over the saved chunk, and the changes are
        // made in the tree as edits (each marked): saving the tab writes them, and the explorer lets go of
        // its copy then.
        if let Some(new) = layered {
            tree.adopt(new);
        }
        self.docs.push(Doc {
            uid: next_uid(),
            title,
            source,
            format,
            tree,
            styled: None,
            seen: (0, 0.0),
            clock_seen: 0,
            stamp: 0,
        });
        self.active = Some(self.docs.len() - 1);
        self.show_explorer = false;
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

    /// Closes the tabs open on `path` (or inside it); fails if one has unsaved edits.
    fn close_docs_under(&mut self, path: &Path) -> Result<(), String> {
        self.close_docs_where(|d| matches!(&d.source, DocSource::File(p) if p.starts_with(path)))
    }

    fn close_docs_where(&mut self, under: impl Fn(&Doc) -> bool) -> Result<(), String> {
        if let Some(d) = self.docs.iter().find(|d| under(d) && d.tree.dirty) {
            return Err(format!("Save or close {} first (it has unsaved changes)", d.title));
        }
        while let Some(i) = self.docs.iter().position(&under) {
            self.close_doc(i);
        }
        Ok(())
    }

    /// Deletes what a side-panel row stands for: a file or folder on disk, a Bedrock database
    /// record or a console archive entry (the last two are written when the world is saved).
    pub fn delete_source(&mut self, source: &DocSource) -> Result<String, String> {
        match source {
            DocSource::File(p) => self.delete_file(p),
            DocSource::BedrockKey(k) => {
                self.close_docs_where(|d| d.source == *source)?;
                let w = self.bedrock.as_mut().ok_or("world not loaded")?;
                w.db.entries.remove(k);
                self.db_dirty = true;
                self.world_changed = true;
                self.selected_entry = None;
                self.selected_entries.clear();
                Ok(format!("Deleted {}", String::from_utf8_lossy(k)))
            }
            DocSource::ConsoleEntry(name) => {
                self.close_docs_where(|d| match &d.source {
                    DocSource::ConsoleEntry(n) => n == name,
                    DocSource::ConsoleChunk { entry, .. } => entry == name,
                    _ => false,
                })?;
                let s = self.console.as_mut().ok_or("save not loaded")?;
                s.archive.entries.retain(|e| e.name != *name);
                self.console_dirty = true;
                self.world_changed = true;
                self.selected_entry = None;
                self.selected_entries.clear();
                Ok(format!("Deleted {name}"))
            }
            _ => Err("That cannot be deleted".into()),
        }
    }

    /// Cut or copied: remembers the files and folders for a later paste.
    pub fn clip_files(&mut self, paths: &[PathBuf], cut: bool) {
        self.file_clip = Some((paths.to_vec(), cut));
        let what = match paths {
            [one] => file_label(one),
            many => format!("{} items", many.len()),
        };
        self.file_note = format!("{} {what}", if cut { "Cut" } else { "Copied" });
    }

    /// Pastes the clipped files and folders into `dir` (a cut moves them, a copy duplicates them).
    pub fn paste_file(&mut self, dir: &Path) -> Result<String, String> {
        let Some((sources, cut)) = self.file_clip.clone() else {
            return Err("Nothing to paste".into());
        };
        let mut notes = Vec::new();
        for src in &sources {
            notes.push(self.paste_one(src, cut, dir)?);
        }
        if cut {
            self.file_clip = None;
        }
        Ok(if notes.len() == 1 {
            notes.remove(0)
        } else {
            format!("{} {} items", if cut { "Moved" } else { "Pasted" }, notes.len())
        })
    }

    fn paste_one(&mut self, src: &Path, cut: bool, dir: &Path) -> Result<String, String> {
        let src = src.to_path_buf();
        let name = src.file_name().ok_or("That cannot be pasted")?.to_os_string();
        if dir.starts_with(&src) {
            return Err("A folder cannot be pasted into itself".into());
        }
        if cut && src.parent() == Some(dir) {
            return Ok("Already there".into());
        }
        let dest = unused_name(dir, &name);
        if cut {
            self.close_docs_under(&src)?;
            if std::fs::rename(&src, &dest).is_err() {
                copy_any(&src, &dest).map_err(|e| e.to_string())?;
                remove_any(&src).map_err(|e| e.to_string())?;
            }
        } else {
            copy_any(&src, &dest).map_err(|e| e.to_string())?;
        }
        self.world_changed = true;
        self.fresh_files.push(dest.clone());
        self.selected_file = Some(dest.clone());
        self.selected_files = vec![dest.clone()];
        Ok(format!("{} {}", if cut { "Moved" } else { "Pasted" }, file_label(&dest)))
    }

    /// Deletes a file or folder from the world on disk.
    pub fn delete_file(&mut self, path: &Path) -> Result<String, String> {
        self.close_docs_under(path)?;
        remove_any(path).map_err(|e| e.to_string())?;
        if self.file_clip.as_ref().is_some_and(|(p, _)| p.iter().any(|f| f.starts_with(path))) {
            self.file_clip = None;
        }
        if self.selected_file.as_deref().is_some_and(|p| p.starts_with(path)) {
            self.selected_file = None;
        }
        self.world_changed = true;
        Ok(format!("Deleted {}", file_label(path)))
    }

    // -----------------------------------------------------------------------------------------
    // The World Explorer's editing tools
    // -----------------------------------------------------------------------------------------

    /// The world under the edit layer, for the map's dimension `dim`.
    pub(crate) fn edit_base(&self, dim: &str) -> Result<crate::edit::chunks::Base<'_>, String> {
        use crate::edit::chunks::Base;
        Ok(match self.edition {
            UiEdition::Java => {
                let w = mctk_java::JavaWorld::open(&self.path).map_err(|e| e.to_string())?;
                let d = w.dimension(dim).ok_or("no such dimension")?;
                Base::Java { region_dir: d.region_dir(), entities_dir: d.entities_dir() }
            }
            UiEdition::Bedrock => {
                let world = self.bedrock.as_ref().ok_or("world not loaded")?;
                let d = match dim {
                    "minecraft:the_nether" => mctk_bedrock::Dimension::Nether,
                    "minecraft:the_end" => mctk_bedrock::Dimension::End,
                    _ => mctk_bedrock::Dimension::Overworld,
                };
                Base::Bedrock { world, dim: d }
            }
            _ => {
                let save = self.console.as_ref().ok_or("save not loaded")?;
                Base::Console { save, dim: console_dim(dim) }
            }
        })
    }

    /// What the World Explorer's edit layer holds of the Java chunk a tab would show (changed and not saved).
    fn layer_java(&self, source: &DocSource) -> Option<crate::edit::layer::JavaChunk> {
        let id = self.tab_chunk(source)?;
        self.map.edits.read().ok()?.java.get(&id).cloned()
    }

    /// After chunks changed: the flat map's Biomes view takes the biomes they have now (at each column's ground).
    fn refresh_map_biomes(&mut self, chunks: &[(i32, i32)]) {
        if chunks.is_empty() || chunks.len() > 96 {
            return;
        }
        let Ok(base) = self.edit_base(&self.map.dimension) else { return };
        let edits = self.map.edits.clone();
        let Ok(mut layer) = edits.write() else { return };
        let ed = crate::edit::chunks::Editor { layer: &mut layer, base, dim: self.map.dimension.clone(), notes: Vec::new() };
        let mut found = Vec::new();
        for &(cx, cz) in chunks {
            if let Some(h) = self.map.chunk_surface(cx, cz) {
                found.push((cx, cz, crate::edit::tools::biome_columns(&ed, cx, cz, &h)));
            }
        }
        drop(layer);
        for (cx, cz, names) in found {
            self.map.put_chunk_biomes(cx, cz, &names);
        }
    }

    /// The biome the 3D camera is in, kept for the sky (read again when it moves to another cell or the world changes).
    pub fn update_sky_biome(&mut self) {
        let Some(eye) = self.map.eye_block() else {
            self.map.sky_biome = None;
            return;
        };
        let generation = self.map.edits.read().map_or(0, |l| l.generation);
        let key = ([eye[0].div_euclid(4), eye[1].div_euclid(4), eye[2].div_euclid(4)], generation);
        if self.sky_key == Some(key) {
            return;
        }
        self.sky_key = Some(key);
        self.map.sky_biome = self.biome_at(eye);
    }

    /// The biome (a Java id) at a block as the world is now, with the explorer's unsaved changes (any edition).
    pub fn biome_at(&self, p: [i32; 3]) -> Option<String> {
        let base = self.edit_base(&self.map.dimension).ok()?;
        let edits = self.map.edits.clone();
        let mut layer = edits.write().ok()?;
        let ed = crate::edit::chunks::Editor { layer: &mut layer, base, dim: self.map.dimension.clone(), notes: Vec::new() };
        crate::edit::tools::biome_at(&ed, p)
    }

    /// The chunk a chunk tab shows (dimension, x, z).
    fn tab_chunk(&self, source: &DocSource) -> Option<crate::edit::layer::ChunkId> {
        match source {
            DocSource::JavaChunk { region, x, z } => {
                let p = region.to_string_lossy();
                let dim = if p.contains("DIM-1") {
                    "minecraft:the_nether"
                } else if p.contains("DIM1") {
                    "minecraft:the_end"
                } else {
                    "minecraft:overworld"
                };
                Some((dim.to_string(), *x, *z))
            }
            DocSource::BedrockChunk { dim, x, z } => Some((["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"].get(*dim as usize).copied().unwrap_or("minecraft:overworld").to_string(), *x, *z)),
            DocSource::ConsoleChunk { x, z, .. } => Some((self.map.dimension.clone(), *x, *z)),
            _ => None,
        }
    }

    /// Before a tool changes chunks: a tab with unsaved changes open on one of them stops it;
    /// tabs without changes close (they would show the chunk as it was).
    fn release_tabs(&mut self, chunks: &[(i32, i32)]) -> Result<(), String> {
        let dim = self.map.dimension.clone();
        let hit = |ws: &Workspace, d: &Doc| ws.tab_chunk(&d.source).is_some_and(|(dd, x, z)| dd == dim && chunks.contains(&(x, z)));
        // (A tab holding only the explorer's own changes is let go: it is opened again from them.)
        if let Some(d) = self.docs.iter().find(|d| hit(self, d) && d.tree.dirty && !d.tree.only_adopted()) {
            return Err(format!("Save or close the tab \u{201c}{}\u{201d} first (it has unsaved changes)", d.title));
        }
        while let Some(i) = self.docs.iter().position(|d| hit(self, d)) {
            self.close_doc(i);
        }
        Ok(())
    }

    /// Runs one of the World Explorer's tools on the box (or, for chunks, the chunks) selected:
    /// one step to undo, shown at once in 3D, written when the world is saved.
    pub fn run_edit(&mut self, tool: EditTool) -> Result<String, String> {
        use crate::edit::tools;
        let sel = self.map.sel_box;
        let chunks: Vec<(i32, i32)> = match (&tool, sel) {
            (EditTool::DeleteChunks, _) => self.map.selected.clone(),
            (EditTool::DeleteEntity(p) | EditTool::RetypeEntity(p, _) | EditTool::SetYaw { at: p, .. } | EditTool::Ride(p, _) | EditTool::Unmount { mount: p, .. } | EditTool::Change { at: p, .. } | EditTool::PlaceCloud { at: p, .. } | EditTool::UpdateCloud { at: p, .. } | EditTool::PlaceMob { at: p, .. }, _) => box_chunks([p[0].floor() as i32, 0, p[2].floor() as i32], [p[0].floor() as i32, 0, p[2].floor() as i32]),
            (EditTool::PlaceEntity { cell, .. }, _) => box_chunks([cell[0], 0, cell[2]], [cell[0], 0, cell[2]]),
            (EditTool::MoveBox { from, origin, place, .. }, _) => {
                let clip = self.map.clipboard.as_ref().ok_or("Copy something first")?;
                let size = place.size(clip.size);
                let mut v = box_chunks(from.0, from.1);
                v.extend(box_chunks(*origin, [origin[0] + size[0] - 1, origin[1] + size[1] - 1, origin[2] + size[2] - 1]));
                v.sort();
                v.dedup();
                v
            }
            (EditTool::TurnBox { from, turns, .. }, _) => {
                let (lo, hi) = crate::edit::tools::turned_box(from.0, from.1, *turns);
                let mut v = box_chunks(from.0, from.1);
                v.extend(box_chunks(lo, hi));
                v.sort();
                v.dedup();
                v
            }
            (EditTool::BoxProps { bounds, .. }, _) => box_chunks(bounds.0, bounds.1),
            (EditTool::MountMob { mount: a, rider: b, .. }, _) => {
                let cell = |p: &[f64; 3]| [p[0].floor() as i32, 0, p[2].floor() as i32];
                let mut v = box_chunks(cell(a), cell(a));
                v.extend(box_chunks(cell(b), cell(b)));
                v.sort();
                v.dedup();
                v
            }
            (EditTool::MoveEntity(a, b), _) => {
                let mut v = box_chunks([a[0].floor() as i32, 0, a[2].floor() as i32], [a[0].floor() as i32, 0, a[2].floor() as i32]);
                v.extend(box_chunks([b[0].floor() as i32, 0, b[2].floor() as i32], [b[0].floor() as i32, 0, b[2].floor() as i32]));
                v.sort();
                v.dedup();
                v
            }
            (EditTool::MoveBlock(a, b), _) => {
                let mut v = box_chunks(*a, *a);
                v.extend(box_chunks(*b, *b));
                v.sort();
                v.dedup();
                v
            }
            (EditTool::SetBlock(p, _) | EditTool::BlockProps(p, _) | EditTool::BlockEntity(p, _), _) => box_chunks(*p, *p),
            (EditTool::BiomeBrush { lo, hi, .. }, _) => box_chunks([lo[0] * 4, lo[1] * 4, lo[2] * 4], [hi[0] * 4 + 3, hi[1] * 4 + 3, hi[2] * 4 + 3]),
            (EditTool::Brush { center: c, radius: r, .. }, _) => box_chunks([c[0] - r, c[1] - r, c[2] - r], [c[0] + r, c[1] + r, c[2] + r]),
            (EditTool::Paste { origin, place, .. }, _) => {
                let clip = self.map.clipboard.as_ref().ok_or("Copy something first")?;
                let size = place.size(clip.size);
                box_chunks(*origin, [origin[0] + size[0] - 1, origin[1] + size[1] - 1, origin[2] + size[2] - 1])
            }
            (_, Some((lo, hi))) => box_chunks(lo, hi),
            (_, None) => return Err("Select a box first (Box Select)".into()),
        };
        if chunks.is_empty() {
            return Err("Select some chunks first".into());
        }
        let plural = |n: usize, w: &str| match (w, n) {
            ("entity", 1) => "1 entity".to_string(),
            ("entity", n) => format!("{n} entities"),
            (w, 1) => format!("1 {w}"),
            (w, n) => format!("{n} {w}s"),
        };
        let blocks = |d: &tools::Done| {
            let mut m = if d.blocks == 0 { "nothing to change".to_string() } else { format!("{} in {}", plural(d.blocks, "block"), plural(d.chunks, "chunk")) };
            if d.unmapped > 0 {
                m.push_str(&format!(" ({} this kind of world has no such block for)", plural(d.unmapped, "block")));
            }
            m
        };
        let clip = self.map.clipboard.clone();
        let resume = matches!(tool, EditTool::Brush { stroke: true, .. } | EditTool::BiomeBrush { stroke: true, .. } | EditTool::SetYaw { stroke: true, .. } | EditTool::UpdateCloud { stroke: true, .. } | EditTool::TurnBox { stroke: true, .. });
        let (msg, notes) = self.edit_step(tool.label(), &chunks, resume, |ed| match &tool {
            EditTool::DeleteBlocks => {
                let n = tools::delete_blocks(ed, sel.expect("checked"));
                if n == 0 { "Nothing there to change".to_string() } else { format!("Cleared the box ({})", plural(n, "chunk")) }
            }
            EditTool::DeleteEntities => format!("Removed {}", plural(tools::delete_entities(ed, sel.expect("checked")), "entity")),
            EditTool::DeleteChunks => format!("Deleted {}", plural(tools::delete_chunks(ed, &chunks), "chunk")),
            EditTool::Fill(palette) => format!("Filled: {}", blocks(&tools::fill(ed, sel.expect("checked"), palette))),
            EditTool::Replace { from, to, keep } => format!("Replaced: {}", blocks(&tools::replace(ed, sel.expect("checked"), from, to, *keep))),
            EditTool::Cut => {
                let n = tools::delete_blocks(ed, sel.expect("checked"));
                let e = tools::delete_entities(ed, sel.expect("checked"));
                format!("Cut the box ({}, {})", plural(n, "chunk"), plural(e, "entity"))
            }
            EditTool::Paste { origin, place, with_air } => match &clip {
                Some(c) => {
                    let done = tools::paste(ed, c, *origin, *place, *with_air);
                    let e = tools::paste_entities(ed, c, *origin, *place);
                    let mut m = format!("Pasted: {}", blocks(&done));
                    if e > 0 {
                        m.push_str(&format!(" and {}", plural(e, "entity")));
                    }
                    m
                }
                None => "Copy something first".into(),
            },
            EditTool::DeleteEntity(p) => {
                if tools::delete_entity_at(ed, *p) { "Removed it".to_string() } else { "It was not found there".to_string() }
            }
            EditTool::Brush { center, radius, sphere, palette, only, .. } => format!("Painted: {}", blocks(&tools::brush(ed, *center, *radius, *sphere, palette, *only))),
            EditTool::SetBlock(p, state) => {
                tools::fill(ed, (*p, *p), std::slice::from_ref(state));
                if state.name.ends_with(":air") { "Removed the block".into() } else { format!("Made it {}", crate::item_picker::display_name(&state.name)) }
            }
            EditTool::BlockProps(p, props) => {
                tools::set_block_props(ed, *p, props);
                props.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", ")
            }
            EditTool::BiomeBrush { lo, hi, biome, .. } => {
                let done = tools::biome_brush(ed, *lo, *hi, biome);
                if done.blocks == 0 {
                    "This world keeps no such biome there".to_string()
                } else {
                    format!("Painted {} {}", plural(done.blocks, "cell"), crate::item_picker::display_name(biome))
                }
            }
            EditTool::BlockEntity(p, change) => {
                if tools::change_block_entity(ed, *p, change) { change.note() } else { "That block has nothing to change".to_string() }
            }
            EditTool::RetypeEntity(p, id) => {
                let id = id.clone();
                if tools::edit_entity_at(ed, *p, &move |e| tools::retype_entity(e, &id)) { "Changed it".into() } else { "It was not found there".into() }
            }
            EditTool::MoveEntity(from, to) => {
                let to = *to;
                if tools::edit_entity_at(ed, *from, &move |e| tools::set_entity_pos(e, to)) { "Moved it".into() } else { "It was not found there".into() }
            }
            EditTool::MoveBlock(from, to) => {
                tools::move_block(ed, *from, *to);
                "Moved the block".into()
            }
            EditTool::SetYaw { at, yaw, .. } => {
                let yaw = *yaw;
                if tools::edit_entity_at(ed, *at, &move |e| tools::set_yaw(e, yaw)) { "Turned it".into() } else { "It was not found there".into() }
            }
            EditTool::TurnBox { from, turns, .. } => {
                tools::turn_box(ed, *from, *turns);
                "Turned the blocks".into()
            }
            EditTool::BoxProps { bounds, props } => format!("Changed: {}", blocks(&tools::set_box_props(ed, *bounds, props))),
            EditTool::MoveBox { from, origin, place, with_air } => match &clip {
                Some(c) => {
                    // What was copied is taken out of where it was, then put where it goes.
                    tools::delete_blocks(ed, *from);
                    tools::delete_entities(ed, *from);
                    let done = tools::paste(ed, c, *origin, *place, *with_air);
                    let e = tools::paste_entities(ed, c, *origin, *place);
                    let mut m = format!("Moved: {}", blocks(&done));
                    if e > 0 {
                        m.push_str(&format!(" and {}", plural(e, "entity")));
                    }
                    m
                }
                None => "Copy something first".into(),
            },
            EditTool::PlaceMob { at, id } => {
                if tools::place_mob(ed, id, *at) { format!("Placed a {}", crate::item_picker::display_name(id)) } else { "Mobs are placed in Java worlds".into() }
            }
            EditTool::UpdateCloud { at, cloud, .. } => {
                if tools::update_cloud(ed, *at, cloud) { "Changed the potion cloud".into() } else { "The cloud was not found there".into() }
            }
            EditTool::PlaceEntity { cell, id, facing } => {
                if tools::place_entity(ed, id, *cell, *facing) { format!("Placed {}", crate::item_picker::display_name(id)) } else { "These are placed in Java worlds".into() }
            }
            EditTool::Change { at, path, change } => {
                if tools::change_entity(ed, *at, path, change) { "Changed it".into() } else { "It was not found there".into() }
            }
            EditTool::Unmount { mount, path, seat, delete } => {
                let n = tools::unmount(ed, *mount, path, *seat, *delete);
                match (n, *delete) {
                    (0, _) => "No rider was found there".into(),
                    (_, true) => "Removed the rider".into(),
                    _ => "Took it off its mount".into(),
                }
            }
            EditTool::MountMob { mount, path, rider } => {
                if tools::mount_existing(ed, *mount, path, *rider) { "Put it on as a rider".into() } else { "Riders are set in Java worlds, on mobs that are there".into() }
            }
            EditTool::PlaceCloud { at, cloud } => {
                if tools::place_cloud(ed, cloud, *at) { "Placed a potion cloud".into() } else { "Potion clouds are placed in Java worlds".into() }
            }
            EditTool::Ride(p, id) => {
                let id = id.clone();
                if tools::edit_entity_at(ed, *p, &move |e| tools::add_passenger(e, &id, None)) { "Put a rider on it".into() } else { "It was not found there".into() }
            }
        })?;
        Ok(match notes.first() {
            Some(first) => format!("{msg}. {first}"),
            None => msg,
        })
    }

    /// One step of the editing tools over `chunks`: open tabs on them dealt with, `run` given
    /// the world to change through the edit layer, the step kept to undo and the 3D view told.
    /// Gives what `run` said and what could not be done.
    fn edit_step(&mut self, label: &str, chunks: &[(i32, i32)], resume: bool, run: impl FnOnce(&mut crate::edit::chunks::Editor) -> String) -> Result<(String, Vec<String>), String> {
        self.release_tabs(chunks)?;
        let dim = self.map.dimension.clone();
        let edits = self.map.edits.clone();
        let mut layer = edits.write().map_err(|_| "the edits are locked")?;
        let base = self.edit_base(&dim)?;
        let mut ed = crate::edit::chunks::Editor { layer: &mut layer, base, dim: dim.clone(), notes: Vec::new() };
        if resume {
            ed.layer.resume(label);
        } else {
            ed.layer.begin(label);
        }
        let msg = run(&mut ed);
        let notes = std::mem::take(&mut ed.notes);
        let changed = ed.layer.commit();
        drop(layer);
        let here: Vec<(i32, i32)> = changed.iter().filter(|(d, ..)| *d == dim).map(|(_, x, z)| (*x, *z)).collect();
        self.map.edits_changed(&here);
        self.refresh_map_biomes(&here);
        Ok((msg, notes))
    }

    /// Copies the box selected (blocks and block entities) for Paste; with `cut`, the box is
    /// then cleared (one step to undo).
    pub fn copy_box(&mut self, cut: bool) -> Result<String, String> {
        let (lo, hi) = self.map.sel_box.ok_or("Select a box first (Box Select)")?;
        let dim = self.map.dimension.clone();
        let clip = {
            let edits = self.map.edits.clone();
            let mut layer = edits.write().map_err(|_| "the edits are locked")?;
            let base = self.edit_base(&dim)?;
            let ed = crate::edit::chunks::Editor { layer: &mut layer, base, dim, notes: Vec::new() };
            crate::edit::tools::copy(&ed, (lo, hi))
        };
        let (size, solid) = (clip.size, clip.solid());
        self.map.clipboard = Some(clip);
        let what = format!("{} \u{d7} {} \u{d7} {} ({solid} blocks)", size[0], size[1], size[2]);
        if cut {
            self.run_edit(EditTool::Cut)?;
            return Ok(format!("Cut {what}: Paste to place it"));
        }
        Ok(format!("Copied {what}: Paste to place it"))
    }

    /// Reads a schematic file (`.schem`, `.nbt`, `.mcstructure`, `.schematic`) as the copy to
    /// paste, and starts placing it.
    pub fn import_schematic(&mut self, path: &Path) -> Result<String, String> {
        use crate::edit::schematic::{Format, read};
        let format = Format::from_path(path).ok_or("Not a schematic file (.schem, .nbt, .mcstructure or .schematic)")?;
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let clip = read(&bytes, format).map_err(|e| format!("Could not read the {}: {e}", format.name()))?;
        let (size, solid) = (clip.size, clip.solid());
        self.map.clipboard = Some(clip);
        self.map.paste = Some(Default::default());
        Ok(format!("{} \u{d7} {} \u{d7} {} ({solid} blocks) from {}: click where it goes", size[0], size[1], size[2], path.file_name().unwrap_or_default().to_string_lossy()))
    }

    /// Writes the box selected as a schematic file (its format by the file's extension).
    pub fn export_selection(&mut self, path: &Path) -> Result<String, String> {
        use crate::edit::schematic::{Format, write};
        let format = Format::from_path(path).ok_or("Pick a schematic file type (.schem, .nbt, .mcstructure or .schematic)")?;
        let b = self.map.sel_box.ok_or("Select a box first (Box Select)")?;
        let dim = self.map.dimension.clone();
        let clip = {
            let edits = self.map.edits.clone();
            let mut layer = edits.write().map_err(|_| "the edits are locked")?;
            let base = self.edit_base(&dim)?;
            let ed = crate::edit::chunks::Editor { layer: &mut layer, base, dim, notes: Vec::new() };
            crate::edit::tools::copy(&ed, b)
        };
        let bytes = write(&clip, format)?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())?;
        Ok(format!("Saved the box as a {} ({})", format.name(), path.file_name().unwrap_or_default().to_string_lossy()))
    }

    /// Takes back the editing tools' last step (in the World Explorer).
    fn undo_edit(&mut self, redo: bool) -> Option<String> {
        let edits = self.map.edits.clone();
        let mut layer = edits.write().ok()?;
        let (label, chunks) = if redo { layer.redo()? } else { layer.undo()? };
        drop(layer);
        let dim = self.map.dimension.clone();
        let here: Vec<(i32, i32)> = chunks.iter().filter(|(d, ..)| *d == dim).map(|(_, x, z)| (*x, *z)).collect();
        self.map.edits_changed(&here);
        self.refresh_map_biomes(&here);
        Some(if redo { format!("Redid {label}") } else { format!("Undid {label}") })
    }

    /// Writes the editing tools' changes into the world (files for Java, the database for
    /// Bedrock, the save's regions for console). Gives the chunks written (in the map's
    /// dimension), and whether the database or the console save must be written out.
    fn save_edits(&mut self, now: u32) -> Result<(Vec<(i32, i32)>, bool, bool, bool), String> {
        let edits = self.map.edits.clone();
        let layer = edits.read().map_err(|_| "the edits are locked")?;
        if !layer.is_dirty() {
            return Ok((Vec::new(), false, false, false));
        }
        let all = layer.chunks();
        let deleted = all.iter().any(|c| layer.is_deleted(c)) || layer.bedrock.values().any(Option::is_none);
        let here: Vec<(i32, i32)> = all.iter().filter(|(d, ..)| *d == self.map.dimension).map(|(_, x, z)| (*x, *z)).collect();
        let (mut db, mut console) = (false, false);
        // Java: each region file (and entities file) once, with all its chunks.
        if !layer.java.is_empty() {
            let w = mctk_java::JavaWorld::open(&self.path).map_err(|e| e.to_string())?;
            let mut files: std::collections::BTreeMap<PathBuf, Vec<(usize, usize, Option<NamedTag>)>> = Default::default();
            for ((dim, x, z), jc) in &layer.java {
                let d = w.dimension(dim).ok_or("no such dimension")?;
                let pos = ChunkPos::new(*x, *z);
                let (lx, lz) = pos.local();
                let name = pos.region().file_name();
                files.entry(d.region_dir().join(&name)).or_default().push((lx, lz, jc.root.clone()));
                if let Some(e) = &jc.entities {
                    files.entry(d.entities_dir().join(&name)).or_default().push((lx, lz, e.clone()));
                }
            }
            for (file, chunks) in files {
                let old = Region::open(&file).ok();
                let mut b = match &old {
                    Some(r) => {
                        self.backup_file(&file).map_err(|e| e.to_string())?;
                        RegionBuilder::from_region(r).map_err(|e| e.to_string())?
                    }
                    None if chunks.iter().all(|c| c.2.is_none()) => continue,
                    None => RegionBuilder::default(),
                };
                for (lx, lz, root) in chunks {
                    let raw = match root {
                        Some(r) => Some(RawChunk::encode(&r, ChunkCompression::Zlib, now).map_err(|e| e.to_string())?),
                        None => None,
                    };
                    b.set(lx, lz, raw);
                }
                let dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let pos = region_pos(&file).ok_or("bad region file name")?;
                b.write(&dir, pos).map_err(|e| e.to_string())?;
            }
        }
        // Bedrock: into the database, written out with the rest.
        if !layer.bedrock.is_empty() {
            let w = self.bedrock.as_mut().ok_or("world not loaded")?;
            for (k, v) in &layer.bedrock {
                match v {
                    Some(v) => {
                        w.db.entries.insert(k.clone(), v.clone());
                    }
                    None => {
                        w.db.entries.remove(k);
                    }
                }
            }
            db = true;
        }
        // Console: each region entry once, with all its chunks.
        if !layer.console.is_empty() {
            // The console games keep light as saved: the chunks changed (and those beside them)
            // lit again here, so they look right when the world is loaded.
            let mut roots: std::collections::BTreeMap<crate::edit::layer::ChunkId, Option<NamedTag>> = layer.console.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let dims: std::collections::BTreeSet<String> = roots.keys().map(|k| k.0.clone()).collect();
            for dim in dims {
                let edited: Vec<(i32, i32)> = roots.iter().filter(|(k, v)| k.0 == dim && v.is_some()).map(|(k, _)| (k.1, k.2)).collect();
                for ((x, z), root) in crate::gpu_world::console_relight(&self.path, &dim, &layer, &edited) {
                    roots.insert((dim.clone(), x, z), Some(root));
                }
            }
            let s = self.console.as_mut().ok_or("save not loaded")?;
            let codec = s.platform.chunk_codec();
            let mut by_entry: std::collections::BTreeMap<String, Vec<(usize, usize, Option<NamedTag>)>> = Default::default();
            for ((dim, x, z), root) in &roots {
                let want = console_dim(dim);
                let (rx, rz) = (x.div_euclid(32), z.div_euclid(32));
                let Some(entry) = s.archive.regions().into_iter().find(|(d, a, b, _)| *d == want && *a == rx && *b == rz).map(|e| e.3.name.clone()) else { continue };
                by_entry.entry(entry).or_default().push((x.rem_euclid(32) as usize, z.rem_euclid(32) as usize, root.clone()));
            }
            for (entry, chunks) in by_entry {
                let e = s.archive.entries.iter_mut().find(|e| e.name == entry).ok_or("region missing")?;
                let r = ConsoleRegion::parse(e.data.clone()).map_err(|e| e.to_string())?;
                let mut b = ConsoleRegionBuilder::default();
                for (lx, lz) in r.chunks().collect::<Vec<_>>() {
                    b.set(lx, lz, r.raw_chunk(lx, lz).map_err(|e| e.to_string())?);
                }
                for (lx, lz, root) in chunks {
                    let raw = match root {
                        Some(r) => Some(RawConsoleChunk::encode(&r, codec, now).map_err(|e| e.to_string())?),
                        None => None,
                    };
                    b.set(lx, lz, raw);
                }
                e.data = b.build().map_err(|e| e.to_string())?;
            }
            console = true;
        }
        Ok((here, db, console, deleted))
    }

    fn backup_file(&mut self, path: &Path) -> std::io::Result<()> {
        if !self.copied && path.is_file() && self.backed_up.insert(path.to_path_buf()) {
            std::fs::copy(path, backup_name(path, "_MCTBackup"))?;
        }
        Ok(())
    }

    /// Writes every changed document, then the world (back to the original if copied).
    pub fn save(&mut self) -> Result<String, String> {
        self.save_docs(None)
    }

    /// Writes just the tab at `doc` (and, for a working copy, the copy back to the original).
    pub fn save_doc(&mut self, doc: usize) -> Result<String, String> {
        self.save_docs(Some(doc))
    }

    fn save_docs(&mut self, only: Option<usize>) -> Result<String, String> {
        // Files marked deleted go first, so the flags below see what they changed.
        let deleted = self.gone.len();
        if only.is_none() {
            self.apply_deletions()?;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as u32);
        let mut written = 0;
        let mut bedrock_db_changed = std::mem::take(&mut self.db_dirty);
        // Optimize: the database written again whatever changed (writing it is compacting it).
        let optimize = only.is_none() && self.bedrock.is_some() && std::mem::take(&mut self.optimize_on_save);
        let db_before = if optimize { dir_size(&self.path.join("db")) } else { 0 };
        bedrock_db_changed |= optimize;
        // (A working copy is written back to the original with it.)
        self.world_changed |= optimize;
        let mut console_changed = std::mem::take(&mut self.console_dirty);
        // The World Explorer's edits (with the whole world only).
        let (edited, edit_db, edit_console, edit_deleted) = if only.is_none() { self.save_edits(now)? } else { Default::default() };
        let had_edits = !edited.is_empty() || edit_db || edit_console;
        bedrock_db_changed |= edit_db;
        console_changed |= edit_console;
        let changed_world = self.world_changed || bedrock_db_changed || console_changed || had_edits;
        let dirty: Vec<usize> = (0..self.docs.len())
            .filter(|&i| only.is_none_or(|o| o == i))
            .filter(|&i| self.docs[i].tree.dirty && !self.docs[i].tree.read_only)
            .collect();
        // The chunks written, for the World Explorer's 3D view to read again.
        let mut chunks: Vec<(i32, i32)> = edited;
        if had_edits {
            self.world_changed = true;
        }
        for i in dirty {
            match &self.docs[i].source {
                DocSource::JavaChunk { x, z, .. } | DocSource::BedrockChunk { x, z, .. } | DocSource::ConsoleChunk { x, z, .. } => chunks.push((*x, *z)),
                _ => {}
            }
            // Deleted tags go now, and what was new counts as saved.
            self.docs[i].tree.purge_deleted();
            let (source, format, root) = {
                let d = &self.docs[i];
                (d.source.clone(), d.format, d.tree.root.clone())
            };
            let layer_id = self.tab_chunk(&source);
            match source {
                DocSource::File(p) => {
                    let bytes =
                        nbt_view::encode(format.unwrap_or(Format::Java(Compression::Gzip)), &root)?;
                    self.backup_file(&p).map_err(|e| e.to_string())?;
                    std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
                    // A Bedrock world's name is kept twice, in level.dat and in levelname.txt (the
                    // one the game's list and a conversion read): a changed name is written to both.
                    if self.edition == UiEdition::Bedrock && p.file_name().is_some_and(|n| n == "level.dat") {
                        let txt = p.with_file_name("levelname.txt");
                        if let (true, Some(name)) = (txt.is_file(), root.tag.get("LevelName").and_then(Tag::as_str)) {
                            if std::fs::read_to_string(&txt).is_ok_and(|old| old != name) {
                                std::fs::write(&txt, name).map_err(|e| e.to_string())?;
                            }
                        }
                    }
                }
                DocSource::JavaChunk { region, x, z } => {
                    // The entities shown among the chunk's tags go back to their own file.
                    let root = self.split_entities(&region, x, z, root, now)?;
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
                DocSource::BedrockChunk { dim, x, z } => {
                    self.save_bedrock_chunk(dim, x, z, &root)?;
                    bedrock_db_changed = true;
                }
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
            if let Some(id) = layer_id {
                if let Ok(mut layer) = self.map.edits.write() {
                    layer.forget(&id);
                }
            }
            self.docs[i].tree.mark_saved();
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
        if had_edits {
            if let Ok(mut l) = self.map.edits.write() {
                l.clear();
            }
        }
        if edit_deleted {
            // Chunks gone: the flat map is drawn again too.
            self.map.reload(self.path.clone());
        } else if !chunks.is_empty() {
            self.map.chunks_changed(&chunks);
        }
        if only.is_none() {
            self.fresh_files.clear();
            self.edited_files.clear();
            self.rename_log.clear();
            self.rename_redo.clear();
            self.ops.clear();
            self.ops_redo.clear();
        }
        let written = written + deleted;
        let mut msg = if written == 0 && !changed_world && !optimize {
            "Nothing to save".to_string()
        } else {
            format!("Saved {written} file(s)")
        };
        if optimize {
            let after = dir_size(&self.path.join("db"));
            let sizes = format!("{} down to {}", human_size(db_before), human_size(after));
            msg = if written == 0 && !had_edits { format!("Optimized the database ({sizes})") } else { format!("{msg}; the database optimized ({sizes})") };
        }
        Ok(msg)
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

/// A file that will not open as NBT, such as the console's binary map-id table.
fn not_nbt(_name: &str, _why: &str) -> String {
    "Invalid NBT file".to_string()
}

/// A drag-select over the side panel's rows: where it began (relative to the top of the rows, so
/// it follows scrolling), the selection it adds to, and whether it has moved far enough to count.
pub struct FileMarquee {
    pub start: Vec2,
    pub base: Vec<PathBuf>,
    pub base_entries: Vec<DocSource>,
    pub active: bool,
}
