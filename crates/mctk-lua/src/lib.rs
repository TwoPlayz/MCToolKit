//! Lua extension host.
//!
//! An extension is a folder with a `manifest.json` (`name`, `description`, `link`, `commit`,
//! `author`, `version`, `editions`, `extension_types`, `tags`, optional `entry`) and a Lua
//! entry script (default `main.lua`). Extensions live in `<app dir>/extensions/<id>/`.
//!
//! Scripts run in a sandbox: Lua's `io`, `os`, `package`, `debug`, `dofile` and `loadfile`
//! are absent, files are reached only through the `NBT` and `World` APIs, reads are limited
//! to the extension folder, the chosen world and the output folder, writes to the output
//! folder only, and every run has a time and memory limit. See [`api`] for the script API.

pub mod api;
pub mod style;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("manifest: {0}")]
    Manifest(String),
    #[error("Lua: {0}")]
    Lua(#[from] mlua::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("{0}")]
    Invalid(String),
}

/// `manifest.json`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub description: String,
    pub link: String,
    pub commit: String,
    pub author: String,
    pub version: String,
    /// `JAVA`, `BEDROCK`, `CONSOLE`; empty means any.
    pub editions: Vec<String>,
    pub extension_types: Vec<String>,
    pub tags: Vec<String>,
    /// Entry script relative to the extension folder.
    pub entry: String,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let v: Value = serde_json::from_str(text).map_err(|e| Error::Manifest(e.to_string()))?;
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let list = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let m = Manifest {
            name: s("name"),
            description: s("description"),
            link: s("link"),
            commit: s("commit"),
            author: s("author"),
            version: s("version"),
            editions: list("editions"),
            extension_types: list("extension_types"),
            tags: list("tags"),
            entry: v
                .get("entry")
                .and_then(Value::as_str)
                .unwrap_or("main.lua")
                .to_string(),
        };
        if m.name.is_empty() {
            return Err(Error::Manifest("missing \"name\"".into()));
        }
        let entry = Path::new(&m.entry);
        if entry.is_absolute()
            || entry
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(Error::Manifest(
                "\"entry\" must be a plain relative path".into(),
            ));
        }
        Ok(m)
    }

    /// Whether the extension declares support for an edition (`JAVA`, `BEDROCK`, `CONSOLE`).
    pub fn supports(&self, edition: &str) -> bool {
        self.editions.is_empty()
            || self
                .editions
                .iter()
                .any(|e| e.eq_ignore_ascii_case(edition))
    }
}

/// An installed extension.
#[derive(Debug, Clone)]
pub struct Extension {
    pub id: String,
    pub dir: PathBuf,
    pub manifest: Manifest,
}

impl Extension {
    /// Loads an extension from its folder without installing it.
    pub fn load(dir: &Path) -> Result<Self, Error> {
        let manifest = Manifest::parse(&std::fs::read_to_string(dir.join("manifest.json"))?)?;
        Ok(Extension {
            id: sanitize_id(&manifest.name),
            dir: dir.to_path_buf(),
            manifest,
        })
    }
}

/// `%APPDATA%\MCToolKit\extensions` on Windows, `~/.config/MCToolKit/extensions` elsewhere
/// (matches the GUI's `Settings::extensions_dir()` default).
pub fn default_extensions_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("MCToolKit").join("extensions")
}

/// An installed folder whose manifest could not be read.
pub type Broken = (String, Error);

/// Installed extensions, sorted by id; folders with a broken manifest are reported as errors.
pub fn list(extensions_dir: &Path) -> Vec<Result<Extension, Broken>> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(extensions_dir) else {
        return out;
    };
    for e in entries.flatten() {
        let dir = e.path();
        let id = e.file_name().to_string_lossy().to_string();
        if !dir.is_dir() || id.starts_with('.') {
            continue;
        }
        out.push(match Extension::load(&dir) {
            Ok(ext) => Ok(Extension { id, ..ext }),
            Err(err) => Err((id, err)),
        });
    }
    let key = |r: &Result<Extension, Broken>| match r {
        Ok(e) => e.id.clone(),
        Err((id, _)) => id.clone(),
    };
    out.sort_by_key(key);
    out
}

fn sanitize_id(name: &str) -> String {
    let id: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let id = id.trim_matches('-').to_string();
    if id.is_empty() {
        "extension".into()
    } else {
        id
    }
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let (from, to) = (e.path(), dst.join(e.file_name()));
        if e.file_type()?.is_symlink() {
            continue;
        }
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Installs an extension folder (copied) under `extensions_dir`; replaces an existing
/// installation with the same id.
pub fn install_folder(src: &Path, extensions_dir: &Path) -> Result<Extension, Error> {
    let ext = Extension::load(src)?;
    if !src.join(&ext.manifest.entry).is_file() {
        return Err(Error::Manifest(format!(
            "entry script {} is missing",
            ext.manifest.entry
        )));
    }
    let dir = extensions_dir.join(&ext.id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    copy_dir(src, &dir)?;
    Ok(Extension { dir, ..ext })
}

/// Installs from a zip, such as a GitHub "Download ZIP" archive whose files sit inside one
/// top-level folder. Entries that would escape the target folder are refused.
pub fn install_zip(zip_path: &Path, extensions_dir: &Path) -> Result<Extension, Error> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    let staging = extensions_dir.join(format!(".staging-{}", std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let result = (|| {
        for i in 0..archive.len() {
            let mut f = archive.by_index(i)?;
            let Some(rel) = f.enclosed_name() else {
                return Err(Error::Invalid(format!("unsafe path in zip: {}", f.name())));
            };
            let target = staging.join(rel);
            if f.is_dir() {
                std::fs::create_dir_all(&target)?;
            } else {
                if let Some(p) = target.parent() {
                    std::fs::create_dir_all(p)?;
                }
                std::io::copy(&mut f, &mut std::fs::File::create(&target)?)?;
            }
        }
        // The manifest is at the top or inside a single top-level folder.
        let root = if staging.join("manifest.json").is_file() {
            staging.clone()
        } else {
            let dirs: Vec<PathBuf> = std::fs::read_dir(&staging)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join("manifest.json").is_file())
                .collect();
            match dirs.as_slice() {
                [one] => one.clone(),
                _ => return Err(Error::Manifest("no manifest.json in the zip".into())),
            }
        };
        install_folder(&root, extensions_dir)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// Removes an installed extension.
pub fn remove(extensions_dir: &Path, id: &str) -> Result<(), Error> {
    if sanitize_id(id) != id {
        return Err(Error::Invalid(format!("not an extension id: {id}")));
    }
    let dir = extensions_dir.join(id);
    if !dir.join("manifest.json").is_file() {
        return Err(Error::Invalid(format!("{id} is not installed")));
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

/// What a run may touch.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// World the script may read (folder or file).
    pub world: Option<PathBuf>,
    /// Folder the script may write into.
    pub output: Option<PathBuf>,
    pub time_limit: Duration,
    pub memory_limit: usize,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            world: None,
            output: None,
            time_limit: Duration::from_secs(120),
            memory_limit: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunOutput {
    /// Lines printed with `Print` / `print`.
    pub log: Vec<String>,
    /// What `run(ctx)` returned, as text.
    pub result: Option<String>,
}

/// Runs an extension: executes its entry script, then calls its global `run(ctx)` if it
/// defines one, with `ctx = { world = path, output = path, edition = EDITION.* }`.
pub fn run(ext: &Extension, options: &RunOptions) -> Result<RunOutput, Error> {
    let edition = options.world.as_deref().and_then(mctk_world::detect);
    if let Some(e) = edition {
        let name = match e {
            mctk_world::Edition::Java => "JAVA",
            mctk_world::Edition::Bedrock => "BEDROCK",
            mctk_world::Edition::Console => "CONSOLE",
        };
        if !ext.manifest.supports(name) {
            return Err(Error::Invalid(format!(
                "{} does not support {name} worlds",
                ext.manifest.name
            )));
        }
    }
    if let Some(out) = &options.output {
        std::fs::create_dir_all(out)?;
    }
    let host = api::Host::new(&ext.dir, options)?;
    let lua = host.lua()?;
    let script = std::fs::read_to_string(ext.dir.join(&ext.manifest.entry))?;
    let outcome = (|| -> Result<Option<String>, Error> {
        lua.load(&script)
            .set_name(ext.manifest.entry.as_str())
            .exec()?;
        Ok(match lua.globals().get::<mlua::Value>("run")? {
            mlua::Value::Function(f) => {
                let ctx = lua.create_table()?;
                if let Some(w) = &options.world {
                    ctx.set("world", w.display().to_string())?;
                }
                if let Some(o) = &options.output {
                    ctx.set("output", o.display().to_string())?;
                }
                if let (Some(e), Some(w)) = (edition, &options.world) {
                    ctx.set("edition", api::edition_number(w, e))?;
                }
                match f.call::<mlua::Value>(ctx)? {
                    mlua::Value::Nil => None,
                    other => Some(other.to_string()?),
                }
            }
            _ => None,
        })
    })();
    let log = host.take_log();
    match outcome {
        Ok(result) => Ok(RunOutput { log, result }),
        Err(e) if log.is_empty() => Err(e),
        Err(e) => Err(Error::Invalid(format!(
            "{e}\nlog before the error:\n{}",
            log.join("\n")
        ))),
    }
}
