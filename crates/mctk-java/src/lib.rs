//! Java Edition world model.
//!
//! Reads worlds from every chunk-format era:
//! - **legacy** (before 1.13): numeric block ids in `Level.Sections[].Blocks/Add/Data`
//! - **flattened** (1.13 – 1.17): `Level.Sections[].Palette` + `BlockStates`
//! - **modern** (1.18 and later): root-level `sections[].block_states` / `biomes`,
//!   including the 26.x palette form (`"minecraft:stone"` strings or `{id, properties}`)
//!
//! Fields the model does not interpret are kept, so a chunk can be written back unchanged.

mod block;
mod chunk;
mod level;
mod palette;
mod world;

pub use block::BlockState;
pub use chunk::{BlockStorage, Chunk, ChunkEra, PaletteStyle, Section, block_index};
pub use level::{LevelInfo, Spawn};
pub use palette::{PalettedContainer, bits_for};
pub use world::{Dimension, JavaWorld};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("NBT error: {0}")]
    Nbt(#[from] mctk_nbt::Error),
    #[error("region error: {0}")]
    Region(#[from] mctk_anvil::Error),
    #[error("not a Java world: {0}")]
    NotAWorld(String),
    #[error("malformed chunk: {0}")]
    BadChunk(String),
}
