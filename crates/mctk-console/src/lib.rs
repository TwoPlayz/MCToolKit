//! Minecraft Legacy Console Edition (Xbox 360, PS3, Wii U) saves.
//!
//! A console save is one archive file (`GAMEDATA` on PS3, the decompressed `savegame.dat`
//! on Xbox 360) holding the world's files: `level.dat`, region files (`r.x.z.mcr`,
//! `DIM-1r.x.z.mcr` for the Nether, `DIM1/r.x.z.mcr` for the End), maps and players.
//! Region chunks are NBT, run-length encoded and then compressed per platform: raw
//! deflate on PS3, XMem LZX on Xbox 360.

mod archive;
mod chunk;
mod compressed;
mod lzx_enc;
mod lzx_store;
mod ps3;
mod ps3_crypto;
mod region;
mod wiiu;
mod rle;
mod save;
mod v12;
mod xmem;

pub use archive::{ArchiveEntry, SaveArchive};
pub use chunk::{ConsoleChunk, console_block_index};
pub use lzx_enc::xmem_compress;
pub use lzx_store::xmem_store;
pub use lzxd::WindowSize;
pub use ps3::{
    METADATA_SIZE, METADATA_VERSION, PFD_SIZE, Ps3PfdContents, Ps3Random, Ps3Region, Ps3SaveFolder, build_metadata,
    build_param_pfd, build_param_sfo, build_save_folder, decrypt_metadata, encrypt_metadata, metadata_plaintext,
    metadata_plaintext_v, read_pfd_random, resign_save_folder, save_directory_name, utc_timestamp, verify_param_pfd,
    verify_save_folder, write_save_folder, build_save_folder_unencrypted, unprotect_save_folder, Ps3Ids, apply_ids,
    patch_sfo_ids, read_sfo_ids,
};
pub use region::{ChunkCodec, ConsoleRegion, ConsoleRegionBuilder, RawConsoleChunk};
pub use rle::{rle_decode, rle_encode};
pub use save::{
    ConsoleSave, Platform, compress_ps3_gamedata, compress_windows64, compress_wiiu, compress_xbox360,
    decompress_ps3_gamedata, decompress_windows64, decompress_wiiu, decompress_xbox360,
};
pub use v12::{CELLS as V12_CELLS, ChunkV12, cell as v12_cell, is_v12, tree_value as v12_tree_value, IN_WATER as V12_IN_WATER, IN_WATER_KEY as V12_IN_WATER_KEY, EXTENDED_KEY as V12_EXTENDED_KEY};
pub use wiiu::{EXT_NAME_BYTES, WiiUExt, build_wiiu_ext, parse_wiiu_ext, read_wiiu_ext, wiiu_ext_path, write_wiiu_ext};
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
