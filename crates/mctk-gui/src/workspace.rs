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

/// A number no other tab has had, so a tab keeps its identity when tabs are reordered.
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
    pub show_locator: bool,
    /// The working copy differs from the original (e.g. after pruning).
    pub world_changed: bool,
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
            show_locator: false,
            world_changed: false,
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
        self.world_changed || !self.gone.is_empty() || self.docs.iter().any(|d| d.tree.dirty)
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
        if self.show_locator {
            return None;
        }
        self.active.and_then(|i| self.docs.get(i))
    }

    /// Whether the Undo button has anything to take back: the open tab's tag edits or a file
    /// change on the left.
    pub fn can_undo_any(&self) -> bool {
        self.history_doc().is_some_and(|d| d.tree.can_undo()) || !self.ops.is_empty()
    }

    pub fn can_redo_any(&self) -> bool {
        self.history_doc().is_some_and(|d| d.tree.can_redo()) || !self.ops_redo.is_empty()
    }

    /// Takes back whichever happened last, a tag edit in the open tab or a file change. Returns
    /// what was undone, to tell the user.
    pub fn undo_any(&mut self) -> Result<String, String> {
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
        if let Some(i) = self.active.filter(|_| !self.show_locator && tag.is_some()) {
            if let Some(d) = self.docs.get_mut(i) {
                d.tree.undo();
                self.last_undone_file = false;
                return Ok(std::mem::take(&mut d.tree.status));
            }
        }
        Ok(String::new())
    }

    pub fn redo_any(&mut self) -> Result<String, String> {
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

    // -------------------------------------------------------------- documents

    fn load(&self, source: &DocSource) -> Result<(String, Option<Format>, NamedTag, bool), String> {
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

    /// Opens (or focuses) a document tab.
    pub fn open_doc(&mut self, source: DocSource) -> Result<(), String> {
        // Something marked deleted is only waiting to go; it is not opened.
        if self.is_gone(&source) {
            return Ok(());
        }
        if let Some(i) = self.docs.iter().position(|d| d.source == source) {
            self.active = Some(i);
            self.show_locator = false;
            return Ok(());
        }
        let (title, format, root, read_only) = self.load(&source)?;
        let mut tree = NbtTree::new(root);
        tree.read_only = read_only;
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
        let mut console_changed = std::mem::take(&mut self.console_dirty);
        let changed_world = self.world_changed || bedrock_db_changed || console_changed;
        let dirty: Vec<usize> = (0..self.docs.len())
            .filter(|&i| only.is_none_or(|o| o == i))
            .filter(|&i| self.docs[i].tree.dirty && !self.docs[i].tree.read_only)
            .collect();
        for i in dirty {
            // Deleted tags go now, and what was new counts as saved.
            self.docs[i].tree.purge_deleted();
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
        if only.is_none() {
            self.fresh_files.clear();
            self.edited_files.clear();
            self.rename_log.clear();
            self.rename_redo.clear();
            self.ops.clear();
            self.ops_redo.clear();
        }
        let written = written + deleted;
        Ok(if written == 0 && !changed_world {
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
