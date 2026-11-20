//! World conversion between Minecraft editions.
//!
//! Implemented: Legacy Console (Xbox 360 / PS3) → Java Edition, and Java ↔ Bedrock.

mod bedrock_view;
pub mod bedrock_layout;
pub mod depth;
mod block_entities;
mod block_entities_more;
mod frames;
mod block_entity;
mod console_out;
mod console_to_java;
mod entity;
mod item_codec;
mod java_bedrock;
mod java_entities;
mod java_era;
mod java_items;
mod java_java;
mod java_level;
mod java_numeric;
mod java_old;
mod java_player;
mod java_saved;
mod java_text;
mod legacy;
mod legacy_tables;
mod legacy_up;
mod options;
mod paintings;
mod text_codec;
pub mod pipeline;
pub mod view;

pub use console_to_java::{
    ConvertReport, console_chunk_to_anvil, console_to_java, console_to_java_with,
};
pub use java_bedrock::{
    BedrockReplacements, EditionReport, JavaReplacements, bedrock_to_java, bedrock_to_java_replacing,
    bedrock_to_java_with, java_to_bedrock, java_to_bedrock_replacing, java_to_bedrock_with,
    set_bedrock_target,
};
pub use bedrock_view::{bedrock_view, java_view};
pub use console_out::{ConsoleReport, java_to_console, legacy_registry};
pub use legacy_up::{LegacyReport, console_to_modern};
pub use java_java::{JavaReport, JavaTarget, ReplacedBlock, java_to_java};
pub use java_level::{LevelData, RuleValue};
pub use options::{ConvertOptions, Progress};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("NBT error: {0}")]
    Nbt(#[from] mctk_nbt::Error),
    #[error("region error: {0}")]
    Region(#[from] mctk_anvil::Error),
    #[error("console save error: {0}")]
    Console(#[from] mctk_console::Error),
    #[error("Java world error: {0}")]
    Java(#[from] mctk_java::Error),
    #[error("Bedrock world error: {0}")]
    Bedrock(#[from] mctk_bedrock::Error),
    #[error("LevelDB error: {0}")]
    LevelDb(#[from] mctk_leveldb::Error),
    #[error("{0}")]
    Invalid(String),
}
