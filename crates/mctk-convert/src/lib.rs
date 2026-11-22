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

/// The number Java before 1.18 saved a biome as (plains for one it had no number for): newer biomes count as the closest old one.
pub fn old_java_biome_id(name: &str) -> i32 {
    java_old::biome_id(&java_old::old_biome_name(name))
}

/// The Java biome a console game's biome number stands for.
pub fn console_biome_name(id: u8) -> String {
    legacy_tables::LegacyTables::get().biome(id).to_string()
}

/// The number the console games save a biome as.
pub fn console_biome_id(name: &str) -> Option<u8> {
    let t = legacy_tables::LegacyTables::get();
    t.console_biome(name).or_else(|| t.console_biome(&java_old::old_biome_name(name)))
}
pub use console_out::{ConsoleReport, classic_keeps, console_supports, current_item_text, java_to_console, legacy_registry, legacy_registry_in};
pub use legacy_up::{LegacyReport, console_to_modern};
pub use java_java::{JavaReport, JavaTarget, ReplacedBlock, java_to_java};
pub use java_level::{LevelData, RuleValue};
pub use options::{ConsoleFormat, ConvertOptions, Progress};

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
