//! Embeds the built-in extensions (`assets/extensions/**`) as a file list.

use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("assets/extensions")
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.join("assets/extensions");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut code = String::from("pub const FILES: &[(&str, &[u8])] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let rel = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        code.push_str(&format!(
            "    ({rel:?}, include_bytes!({:?})),\n",
            f.display().to_string()
        ));
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("builtin_extensions.rs");
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
