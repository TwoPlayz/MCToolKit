use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use mctk_leveldb::Db;
use mctk_nbt::{Flavor, NamedTag, Tag};

use crate::Error;

/// Bedrock `level.dat`: an 8-byte header (storage version, payload length, both u32 LE)
/// followed by little-endian NBT.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelDat {
    pub storage_version: u32,
    pub root: NamedTag,
}

impl LevelDat {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 8 {
            return Err(Error::Malformed("level.dat shorter than its header".into()));
        }
        let storage_version = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let len = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let body = bytes
            .get(8..8 + len)
            .ok_or_else(|| Error::Malformed("level.dat length exceeds file".into()))?;
        Ok(LevelDat {
            storage_version,
            root: mctk_nbt::from_bytes(body, Flavor::Bedrock)?,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let body = mctk_nbt::to_bytes(&self.root, Flavor::Bedrock)?;
        let mut out = Vec::with_capacity(body.len() + 8);
        out.extend_from_slice(&self.storage_version.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        Ok(out)
    }

    pub fn level_name(&self) -> Option<&str> {
        self.root.tag.get("LevelName").and_then(Tag::as_str)
    }

    /// `lastOpenedWithVersion`, e.g. `[1, 21, 50, 7, 0]`.
    pub fn last_opened_version(&self) -> Option<Vec<i64>> {
        let list = self.root.tag.get("lastOpenedWithVersion")?.as_list()?;
        list.iter().map(Tag::as_i64).collect()
    }
}

/// A Bedrock world loaded into memory.
#[derive(Debug)]
pub struct BedrockWorld {
    pub source: PathBuf,
    pub level: LevelDat,
    pub level_name_txt: Option<String>,
    pub db: Db,
    /// Every other file in the world (icon, pack lists, ...), by relative path.
    pub other_files: HashMap<String, Vec<u8>>,
}

impl BedrockWorld {
    /// Opens a world folder or a `.mcworld` archive.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let files = if path.is_dir() {
            read_folder(path)?
        } else {
            read_zip(path)?
        };
        BedrockWorld::from_files(path.to_path_buf(), files)
    }

    fn from_files(source: PathBuf, mut files: HashMap<String, Vec<u8>>) -> Result<Self, Error> {
        // Some archives nest the world inside one top-level folder.
        if !files.contains_key("level.dat") {
            if let Some(prefix) = files.keys().find_map(|k| {
                k.strip_suffix("level.dat")
                    .filter(|p| p.ends_with('/'))
                    .map(str::to_string)
            }) {
                files = files
                    .into_iter()
                    .filter_map(|(k, v)| Some((k.strip_prefix(&prefix)?.to_string(), v)))
                    .collect();
            }
        }
        let level = LevelDat::parse(
            files
                .get("level.dat")
                .ok_or_else(|| Error::NotAWorld("no level.dat".into()))?,
        )?;
        let mut db_files = HashMap::new();
        let mut other_files = HashMap::new();
        for (name, bytes) in files {
            match name.strip_prefix("db/") {
                Some(f) => {
                    db_files.insert(f.to_string(), bytes);
                }
                None => {
                    other_files.insert(name, bytes);
                }
            }
        }
        if db_files.is_empty() {
            return Err(Error::NotAWorld("no db/ folder".into()));
        }
        let db = Db::from_files(&db_files)?;
        let level_name_txt = other_files
            .get("levelname.txt")
            .map(|b| String::from_utf8_lossy(b).trim().to_string());
        Ok(BedrockWorld {
            source,
            level,
            level_name_txt,
            db,
            other_files,
        })
    }

    /// Writes the world as a folder: `level.dat`, the other files, and a freshly compacted
    /// `db/`. The target must not exist yet or be empty; the source is never modified.
    pub fn save_to_folder(&self, dir: impl AsRef<Path>) -> Result<(), Error> {
        let dir = dir.as_ref();
        if dir.exists() && std::fs::read_dir(dir)?.next().is_some() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} is not empty", dir.display()),
            )));
        }
        std::fs::create_dir_all(dir)?;
        for (name, bytes) in &self.other_files {
            if name == "level.dat" {
                continue;
            }
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, bytes)?;
        }
        std::fs::write(dir.join("level.dat"), self.level.encode()?)?;
        mctk_leveldb::write_db(
            dir.join("db"),
            &self.db.entries,
            mctk_leveldb::WriteOptions::default(),
        )?;
        Ok(())
    }
}

fn read_folder(root: &Path) -> Result<HashMap<String, Vec<u8>>, Error> {
    let mut files = HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(rel, std::fs::read(&path)?);
            }
        }
    }
    Ok(files)
}

fn read_zip(path: &Path) -> Result<HashMap<String, Vec<u8>>, Error> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let mut files = HashMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        // enclosed_name rejects absolute paths and `..` components.
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let name = name.to_string_lossy().replace('\\', "/");
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes)?;
        files.insert(name, bytes);
    }
    Ok(files)
}
