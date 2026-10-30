//! World conversion between Minecraft editions.
//!
//! Implemented: Legacy Console (Xbox 360 / PS3) → Java Edition, and Java ↔ Bedrock.

mod block_entity;
mod console_to_java;
mod entity;
mod java_bedrock;
mod legacy;
mod options;
pub mod pipeline;

pub use console_to_java::{
    ConvertReport, console_chunk_to_anvil, console_to_java, console_to_java_with,
};
pub use java_bedrock::{
    EditionReport, bedrock_to_java, bedrock_to_java_with, java_to_bedrock, java_to_bedrock_with,
};
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
