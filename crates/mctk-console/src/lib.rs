//! Minecraft Legacy Console Edition (Xbox 360, PS3, Wii U) saves.
//!
//! A console save is one archive file (`GAMEDATA` on PS3, the decompressed `savegame.dat`
//! on Xbox 360) holding the world's files: `level.dat`, region files (`r.x.z.mcr`,
//! `DIM-1r.x.z.mcr` for the Nether, `DIM1/r.x.z.mcr` for the End), maps and players.
//! Region chunks are NBT, run-length encoded and then compressed per platform: raw
//! deflate on PS3, XMem LZX on Xbox 360.

mod archive;
mod chunk;
mod lzx_store;
mod region;
mod rle;
mod save;
mod xmem;

pub use archive::{ArchiveEntry, SaveArchive};
pub use chunk::{ConsoleChunk, console_block_index};
pub use lzx_store::xmem_store;
pub use lzxd::WindowSize;
pub use region::{ChunkCodec, ConsoleRegion, ConsoleRegionBuilder, RawConsoleChunk};
pub use rle::{rle_decode, rle_encode};
pub use save::{ConsoleSave, Platform, compress_xbox360, decompress_xbox360};
pub use xmem::{Xbox360SaveHeader, xmem_decompress, xmem_decompress_all};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("NBT error: {0}")]
    Nbt(#[from] mctk_nbt::Error),
    #[error("malformed {0}")]
    Malformed(String),
}

pub(crate) fn bad(msg: impl Into<String>) -> Error {
    Error::Malformed(msg.into())
}
