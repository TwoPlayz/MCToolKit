//! Bedrock Edition world model.
//!
//! A world is a folder (or a `.mcworld` zip of one) holding `level.dat` (little-endian NBT
//! behind an 8-byte header), `levelname.txt`, `world_icon.jpeg` and a LevelDB database in
//! `db/`. Chunk data lives under binary keys `x, z, [dimension], tag, [subchunk]`.

mod key;
mod subchunk;
mod world;

pub use key::{ChunkKey, ChunkTag, Dimension, Key};
pub use subchunk::{
    BiomeStorage, BlockStorage, Data3D, SubChunk, SubChunkBody, block_index, block_state_name,
};
pub use world::{BedrockWorld, LevelDat};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("NBT error: {0}")]
    Nbt(#[from] mctk_nbt::Error),
    #[error("LevelDB error: {0}")]
    Db(#[from] mctk_leveldb::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("not a Bedrock world: {0}")]
    NotAWorld(String),
    #[error("malformed {0}")]
    Malformed(String),
}
