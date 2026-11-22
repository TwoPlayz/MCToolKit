//! MCToolKit's own resource pack folder (`<working dir>/resource_packs/default`): the game's
//! assets for the latest release, fetched from mcasset.cloud as they are wanted and kept here
//! (see `assets`), and a `pack.png` as the image of worlds without one.
//!
//! The textures are Mojang's artwork, so MCToolKit does not bundle them.

use std::path::{Path, PathBuf};

/// `<working dir>/resource_packs/default`.
pub fn default_dir(working_dir: &Path) -> PathBuf {
    working_dir.join("resource_packs").join("default")
}

/// Makes `pack` the folder the latest assets are kept in and checks they are current (on this
/// thread: call it from a background one). The quick part of [`ensure`], which the splash
/// screen waits for.
pub fn prepare(pack: &Path) {
    crate::assets::set_root(pack.to_path_buf());
    crate::assets::prepare();
}

/// All of it, in the background: [`prepare`], then the files the 3D view draws with, and
/// `pack.png` if it is missing.
pub fn ensure(pack: &Path) {
    prepare(pack);
    fetch_rest(pack);
}

/// The files fetched ahead of use (see [`ensure`]), after [`prepare`].
pub fn fetch_rest(pack: &Path) {
    // What the 3D view draws every block and item with, ahead of first use (not in tests,
    // which fetch only what they draw).
    #[cfg(not(test))]
    crate::assets::prefetch(&[
        "assets/minecraft/blockstates",
        "assets/minecraft/models/block",
        "assets/minecraft/models/item",
        "assets/minecraft/items",
        "assets/minecraft/textures/block",
        "assets/minecraft/textures/item",
    ]);
    let png = pack.join("pack.png");
    if !png.is_file() {
        // The game's own pack image, where one is to be had.
        let found = crate::assets::fetch("pack.png");
        if let Some(bytes) = found.and_then(|f| std::fs::read(f).ok()) {
            let _ = std::fs::create_dir_all(pack);
            let _ = std::fs::write(png, bytes);
        }
    }
}
