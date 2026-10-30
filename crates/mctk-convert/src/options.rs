//! Conversion options and live progress, shared by every converter.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// What to convert and how to adjust the output (the Converter's "Conversion Options").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertOptions {
    pub overworld: bool,
    pub nether: bool,
    pub the_end: bool,
    /// Leave out chunks the game had not finished generating.
    pub skip_partial_chunks: bool,
    /// Output game mode (0 survival, 1 creative, 2 adventure); `None` keeps the source's.
    pub game_mode: Option<i32>,
    /// Output difficulty (0 peaceful … 3 hard); `None` keeps the source's.
    pub difficulty: Option<i32>,
    /// Chunk offset added to every output chunk (moves the world's centre).
    pub offset: (i32, i32),
}

impl Default for ConvertOptions {
    fn default() -> Self {
        ConvertOptions {
            overworld: true,
            nether: true,
            the_end: true,
            skip_partial_chunks: true,
            game_mode: None,
            difficulty: None,
            offset: (0, 0),
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
