//! Chunk pruning: delete every chunk inside (or, inverted, outside) a selection.
//!
//! A [`Selection`] is a union of layers — Box, Circle, Split, Polygon and Time
//! (InhabitedTime). As in UMT's Pruner, normal mode removes the selected chunks and
//! inverted mode keeps only the selected chunks. Pruning never modifies the source world:
//! every function writes a pruned copy.

mod bedrock;
mod console;
mod java;
mod selection;

use std::path::Path;

pub use bedrock::{prune_bedrock, prune_bedrock_world};
pub use console::{prune_console, save_console_copy};
pub use java::prune_java;
pub use selection::{Comparison, Layer, Selection};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("NBT error: {0}")]
    Nbt(#[from] mctk_nbt::Error),
    #[error("region error: {0}")]
    Region(#[from] mctk_anvil::Error),
    #[error("Java world error: {0}")]
    Java(#[from] mctk_java::Error),
    #[error("Bedrock world error: {0}")]
    Bedrock(#[from] mctk_bedrock::Error),
    #[error("console save error: {0}")]
    Console(#[from] mctk_console::Error),
    #[error("{0}")]
    Invalid(String),
}

/// What a prune removed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PruneReport {
    /// Terrain chunks kept and removed (Bedrock: chunk columns).
    pub kept: usize,
    pub removed: usize,
    pub notes: Vec<String>,
}

/// Dimension filter: `None` prunes every dimension, `Some(id)` only that one
/// (0 Overworld, -1 Nether, 1 End).
fn applies(filter: Option<i32>, dimension: Option<i32>) -> bool {
    filter.is_none() || filter == dimension
}

fn check_selection(selection: &Selection) -> Result<(), Error> {
    if selection.layers.is_empty() {
        return Err(Error::Invalid(
            "the selection has no layers; add at least one".into(),
        ));
    }
    Ok(())
}

/// Refuses an output folder that is non-empty or lies inside the source, then creates it.
fn prepare_output(src: &Path, out: &Path) -> Result<(), Error> {
    let (src_abs, out_abs) = (std::path::absolute(src)?, std::path::absolute(out)?);
    if out_abs.starts_with(&src_abs) {
        return Err(Error::Invalid(format!(
            "{} is inside the source world",
            out.display()
        )));
    }
    if out.exists() && std::fs::read_dir(out)?.next().is_some() {
        return Err(Error::Invalid(format!("{} is not empty", out.display())));
    }
    std::fs::create_dir_all(out)?;
    Ok(())
}
