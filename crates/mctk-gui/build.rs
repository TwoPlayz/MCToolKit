//! Lists the vector icons in `assets/icons/` for the app to embed, and embeds the exe's icon
//! on Windows.

use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    // Every `assets/icons/NAME.svg` becomes ("NAME", its text).
    let dir = manifest_dir.join("assets/icons");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("assets/icons")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "svg"))
        .collect();
    files.sort();
    let mut code = String::from("pub const SVGS: &[(&str, &str)] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let name = f.file_stem().unwrap().to_string_lossy();
        code.push_str(&format!("    ({name:?}, include_str!({:?})),\n", f.display().to_string()));
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("icon_list.rs");
    std::fs::write(out, code).unwrap();

    // The exe's icon (Explorer, taskbar, Alt-Tab), from the same mark used for the
    // in-app title bar and the runtime window icon (see `examples/gen_logo.rs`).
    let icon_rc = manifest_dir.join("assets/icon.rc");
    println!("cargo:rerun-if-changed={}", icon_rc.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("assets/logo.ico").display()
    );
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if let Err(e) = embed_resource::compile(&icon_rc, embed_resource::NONE).manifest_optional()
        {
            println!("cargo:warning=could not embed the exe icon: {e}");
        }
    }
}
