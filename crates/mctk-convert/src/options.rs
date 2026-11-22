//! Conversion options and live progress, shared by every converter.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Which generation of console save a conversion to PS3, Xbox 360 or Wii U writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConsoleFormat {
    /// The older layout, readable by older game versions: archive version 8, the older compact chunk
    /// layout, an uncompressed PS3 `GAMEDATA`, and a world of 320 chunks.
    #[default]
    Legacy,
    /// What the original converter writes for the current game versions: archive version 10, chunks in the
    /// Version 12 layout (record bit 30 set), a compressed PS3 `GAMEDATA`, a `level.dat` with `DataVersion`
    /// 922, and the smallest world size class (54, 64, 192 or 320 chunks) that holds the converted chunks.
    Current,
}

/// What to convert and how to adjust the output (the Converter's "Conversion Options").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertOptions {
    pub overworld: bool,
    pub nether: bool,
    pub the_end: bool,
    /// Leave out chunks the game had not finished generating.
    pub skip_partial_chunks: bool,
    /// Convert chests, signs and the other tile entities; off leaves them empty.
    pub tile_entities: bool,
    /// Convert mobs, items on the ground and the other entities; off leaves them out.
    pub entities: bool,
    /// Keep scheduled block and fluid ticks (Java to Java; other targets recompute them).
    pub ticks: bool,
    /// Convert chunks written by versions older than the world's own (stale chunks the game never reloaded).
    pub older_chunks: bool,
    /// What to do with the part of a deep overworld below y 0 when the target starts at y 0.
    pub depth: crate::depth::DepthMode,
    /// Output game mode (0 survival, 1 creative, 2 adventure); `None` keeps the source's.
    pub game_mode: Option<i32>,
    /// Output difficulty (0 peaceful … 3 hard); `None` keeps the source's.
    pub difficulty: Option<i32>,
    /// Chunk offset added to every output chunk (moves the world's centre).
    pub offset: (i32, i32),
    /// The generation of console save written when the target is a console (ignored otherwise).
    pub console_format: ConsoleFormat,
    /// With [`ConsoleFormat::Current`]: write the classic 54-chunk world (Nether 18) and leave out the
    /// chunks beyond it, as the original converter does, instead of the smallest class that holds them all.
    pub classic_world_size: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        ConvertOptions {
            overworld: true,
            nether: true,
            the_end: true,
            skip_partial_chunks: true,
            tile_entities: true,
            entities: true,
            ticks: true,
            older_chunks: true,
            depth: crate::depth::DepthMode::default(),
            game_mode: None,
            difficulty: None,
            offset: (0, 0),
            console_format: ConsoleFormat::default(),
            classic_world_size: false,
        }
    }
}

impl ConvertOptions {
    /// Whether a dimension (Java numbering: 0 Overworld, -1 Nether, 1 End) is converted.
    pub fn includes(&self, dimension: i32) -> bool {
        match dimension {
            -1 => self.nether,
            1 => self.the_end,
            _ => self.overworld,
        }
    }
}

/// Live progress of a running conversion; shared with the UI, which may also cancel it.
#[derive(Debug, Default)]
pub struct Progress {
    pub chunks_done: AtomicUsize,
    pub chunks_total: AtomicUsize,
    pub regions_done: AtomicUsize,
    pub regions_total: AtomicUsize,
    pub invalid_chunks: AtomicUsize,
    pub dimension: Mutex<String>,
    /// Output chunk positions in the order they were written (for a coverage preview).
    pub converted: Mutex<Vec<(i32, i32)>>,
    /// Where in converted the dimension being converted starts, so a preview can show just it.
    pub dimension_start: AtomicUsize,
    cancel: AtomicBool,
}

impl Progress {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub(crate) fn set_dimension(&self, name: &str) {
        let start = self.converted.lock().expect("progress lock").len();
        self.dimension_start.store(start, Ordering::Relaxed);
        *self.dimension.lock().expect("progress lock") = name.to_string();
    }

    pub(crate) fn chunk(&self, x: i32, z: i32) {
        self.chunks_done.fetch_add(1, Ordering::Relaxed);
        self.converted.lock().expect("progress lock").push((x, z));
    }
}

/// Whether a chunk's generation status means it is complete.
pub(crate) fn is_complete(status: Option<&str>) -> bool {
    match status {
        None => true,
        Some(s) => matches!(
            s.trim_start_matches("minecraft:"),
            "full" | "postprocessed" | "fullchunk"
        ),
    }
}
