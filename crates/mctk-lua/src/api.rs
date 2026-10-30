//! The Lua script API.
//!
//! Globals:
//! - `BASE_PATH`: the extension's folder.
//! - `Print(...)` (also `print`): appends a line to the run log.
//! - `UTF8ToPercentEncoding(s)`: percent-encodes a string for URLs.
//! - Enums `EDITION` (JAVA 0, BEDROCK 1, XBOX360 2, PS3 3, WIIU 4), `TYPE` (NBT tag ids,
//!   END 0 … LONG_ARRAY 12), `JSON_TYPE` and `EXTENSION_TYPE`.
//!
//! Modules:
//! - `NBT.read(path)` → tag, info; `NBT.write(path, tag, {flavor="java"|"bedrock",
//!   compression="gzip"|"zlib"|"none", header=bool})`; `NBT.compound()`, `NBT.list(TYPE.x)`.
//!   Tags are handles into a document: `t:type()`, `t:get(key|index)`, `t:set(key|index,
//!   value[, TYPE.x])`, `t:push(value[, TYPE.x])`, `t:remove(key|index)`, `t:keys()`,
//!   `t:len()`, `t:value()`, `t:copy()`, `tostring(t)` (SNBT). Lists are 1-based.
//! - `World.open(path)` → world with `edition()`, `name()`, `dimensions()`,
//!   `chunks(dim)` → `{ {x=, z=}, … }`, `block(dim, x, y, z)` → name, properties, and
//!   `level()` → a copy of `level.dat`. Worlds are read-only.
//! - `Database.open(bedrock world)` → `count()`, `keys([prefix])`, `get(key)` (raw bytes).
//! - `Files.read_text(path)`, `Files.write_text(path, text)`, `Files.exists(path)`,
//!   `Files.list(dir)`.
//! - `JSON.decode(text)`, `JSON.encode(value[, pretty])`.
//!
//! Relative paths are resolved against the output folder for writes and against
//! `BASE_PATH` for reads.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use mctk_anvil::ChunkPos;
use mctk_bedrock::{BedrockWorld, BlockStorage, ChunkKey, ChunkTag, SubChunk, SubChunkBody};
use mctk_console::{ConsoleChunk, ConsoleRegion, ConsoleSave};
use mctk_java::{BlockStorage as JavaBlocks, Chunk, JavaWorld};
use mctk_nbt::{Compound, Compression, Flavor, List, NamedTag, Tag, TagType};
use mctk_world::Edition;
use mlua::{
    AnyUserData, ChunkMode, HookTriggers, Lua, LuaOptions, MetaMethod, StdLib, Table, UserData,
    UserDataMethods, Value, VmState,
};

use crate::{Error, RunOptions};

fn rt(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::runtime(msg.into())
}

fn ext<E: std::fmt::Display>(e: E) -> mlua::Error {
    mlua::Error::runtime(e.to_string())
}

/// `EDITION` number of a world.
pub fn edition_number(world: &Path, edition: Edition) -> i64 {
    match edition {
        Edition::Java => 0,
        Edition::Bedrock => 1,
        Edition::Console if world.is_dir() || world.ends_with("GAMEDATA") => 3,
        Edition::Console => 2,
    }
}

// ------------------------------------------------------------------ paths

#[derive(Clone)]
struct Paths {
    base: PathBuf,
    output: Option<PathBuf>,
    read_roots: Vec<PathBuf>,
    write_roots: Vec<PathBuf>,
}

/// Canonical form of a path that may not exist yet: the nearest existing ancestor is
/// canonicalized and the remaining plain components appended.
fn canonical(path: &Path) -> Result<PathBuf, mlua::Error> {
    let mut existing = path.to_path_buf();
    let mut rest = Vec::new();
    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or_else(|| rt("path has no existing parent"))?;
        rest.push(name.to_os_string());
        existing = existing
            .parent()
            .ok_or_else(|| rt("path has no existing parent"))?
            .to_path_buf();
    }
    let mut out = existing.canonicalize().map_err(ext)?;
    for name in rest.iter().rev() {
        out.push(name);
    }
    Ok(out)
}

impl Paths {
    fn check(&self, raw: &str, write: bool) -> Result<PathBuf, mlua::Error> {
        let p = Path::new(raw);
        if p.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(rt(format!("'..' is not allowed in paths: {raw}")));
        }
        let anchor = if write {
            self.output.as_ref()
        } else {
            Some(&self.base)
        };
        let full = if p.is_absolute() {
            p.to_path_buf()
        } else {
            anchor
                .ok_or_else(|| rt("no output folder was given for this run"))?
                .join(p)
        };
        let c = canonical(&full)?;
        let roots = if write {
            &self.write_roots
        } else {
            &self.read_roots
        };
        if roots.iter().any(|r| c.starts_with(r)) {
            Ok(c)
        } else {
            Err(rt(format!(
                "{} is outside the folders this run may {}",
                full.display(),
                if write { "write" } else { "read" }
            )))
        }
    }
}

// ------------------------------------------------------------------ NBT handles

#[derive(Debug, Clone, PartialEq)]
enum Seg {
    Key(String),
    Index(usize),
}

#[derive(Clone)]
struct NbtRef {
    doc: Rc<RefCell<Tag>>,
    path: Vec<Seg>,
}

fn walk_mut<'a>(tag: &'a mut Tag, path: &[Seg]) -> Option<&'a mut Tag> {
    let Some((first, rest)) = path.split_first() else {
        return Some(tag);
    };
    let child = match (tag, first) {
        (Tag::Compound(c), Seg::Key(k)) => c.get_mut(k)?,
        (Tag::List(l), Seg::Index(i)) => l.get_mut(*i)?,
        _ => return None,
    };
    walk_mut(child, rest)
}

fn type_from(n: i64) -> Result<TagType, mlua::Error> {
    u8::try_from(n)
        .ok()
        .and_then(TagType::from_id)
        .ok_or_else(|| rt(format!("{n} is not a TYPE value")))
}

/// Converts a Lua value into a tag of the hinted (or existing, or inferred) type.
fn to_tag(v: &Value, hint: Option<TagType>) -> Result<Tag, mlua::Error> {
    let int = |n: i64| -> Result<Tag, mlua::Error> {
        Ok(match hint.unwrap_or(TagType::Int) {
            TagType::Byte => Tag::Byte(n as i8),
            TagType::Short => Tag::Short(n as i16),
            TagType::Int => Tag::Int(n as i32),
            TagType::Long => Tag::Long(n),
            TagType::Float => Tag::Float(n as f32),
            TagType::Double => Tag::Double(n as f64),
            t => return Err(rt(format!("a number cannot be stored as {t:?}"))),
        })
    };
    match v {
        Value::Boolean(b) => int(*b as i64).map(|t| {
            if hint.is_none() {
                Tag::Byte(*b as i8)
            } else {
                t
            }
        }),
        Value::Integer(n) => int(*n),
        Value::Number(f) => Ok(match hint {
            Some(TagType::Float) => Tag::Float(*f as f32),
            None | Some(TagType::Double) => Tag::Double(*f),
            Some(_) => int(*f as i64)?,
        }),
        Value::String(s) => Ok(Tag::String(s.to_str()?.to_string())),
        Value::UserData(ud) => {
            let r = ud.borrow::<NbtRef>()?;
            let doc = r.doc.borrow();
            let mut root = doc.clone();
            Ok(walk_mut(&mut root, &r.path)
                .ok_or_else(|| rt("tag no longer exists"))?
                .clone())
        }
        Value::Table(t) => {
            let nums: Vec<i64> = t
                .clone()
                .sequence_values::<i64>()
                .collect::<Result<_, _>>()?;
            Ok(match hint {
                Some(TagType::ByteArray) => Tag::ByteArray(nums.iter().map(|&n| n as i8).collect()),
                Some(TagType::IntArray) => Tag::IntArray(nums.iter().map(|&n| n as i32).collect()),
                Some(TagType::LongArray) => Tag::LongArray(nums),
                _ => {
                    return Err(rt(
                        "tables become arrays only with TYPE.BYTE_ARRAY, INT_ARRAY or LONG_ARRAY; use NBT.compound() / NBT.list() for containers",
                    ));
                }
            })
        }
        _ => Err(rt("this value cannot be stored in NBT")),
    }
}

impl NbtRef {
    fn new(tag: Tag) -> Self {
        NbtRef {
            doc: Rc::new(RefCell::new(tag)),
            path: Vec::new(),
        }
    }

    fn with<R>(
        &self,
        f: impl FnOnce(&mut Tag) -> Result<R, mlua::Error>,
    ) -> Result<R, mlua::Error> {
        let mut doc = self.doc.borrow_mut();
        let tag = walk_mut(&mut doc, &self.path).ok_or_else(|| rt("tag no longer exists"))?;
        f(tag)
    }

    fn seg(&self, key: &Value, tag: &Tag) -> Result<Seg, mlua::Error> {
        match (tag, key) {
            (Tag::Compound(_), Value::String(s)) => Ok(Seg::Key(s.to_str()?.to_string())),
            (
                Tag::List(_) | Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_),
                Value::Integer(i),
            ) if *i >= 1 => Ok(Seg::Index(*i as usize - 1)),
            _ => Err(rt(
                "use a string key for compounds and a 1-based index for lists and arrays",
            )),
        }
    }

    fn child(&self, seg: Seg) -> NbtRef {
        let mut path = self.path.clone();
        path.push(seg);
        NbtRef {
            doc: Rc::clone(&self.doc),
            path,
        }
    }
}

fn scalar(lua: &Lua, tag: &Tag) -> Result<Option<Value>, mlua::Error> {
    Ok(Some(match tag {
        Tag::Byte(v) => Value::Integer(*v as i64),
        Tag::Short(v) => Value::Integer(*v as i64),
        Tag::Int(v) => Value::Integer(*v as i64),
        Tag::Long(v) => Value::Integer(*v),
        Tag::Float(v) => Value::Number(*v as f64),
        Tag::Double(v) => Value::Number(*v),
        Tag::String(s) => Value::String(lua.create_string(s)?),
        _ => return Ok(None),
    }))
}

impl UserData for NbtRef {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("type", |_, this, ()| {
            this.with(|t| Ok(t.tag_type() as u8 as i64))
        });
        methods.add_method("len", |_, this, ()| {
            this.with(|t| {
                Ok(match t {
                    Tag::Compound(c) => c.len(),
                    Tag::List(l) => l.len(),
                    Tag::ByteArray(a) => a.len(),
                    Tag::IntArray(a) => a.len(),
                    Tag::LongArray(a) => a.len(),
                    Tag::String(s) => s.len(),
                    _ => 0,
                })
            })
        });
        methods.add_method("keys", |lua, this, ()| {
            let keys = this.with(|t| match t {
                Tag::Compound(c) => Ok(c.keys().cloned().collect::<Vec<_>>()),
                _ => Err(rt("keys() needs a compound")),
            })?;
            lua.create_sequence_from(keys)
        });
        methods.add_method("get", |lua, this, key: Value| {
            let (seg, child) = this.with(|t| {
                let seg = this.seg(&key, t)?;
                let child = match (&*t, &seg) {
                    (Tag::Compound(c), Seg::Key(k)) => c.get(k).cloned(),
                    (Tag::List(l), Seg::Index(i)) => l.get(*i).cloned(),
                    (Tag::ByteArray(a), Seg::Index(i)) => a.get(*i).map(|v| Tag::Long(*v as i64)),
                    (Tag::IntArray(a), Seg::Index(i)) => a.get(*i).map(|v| Tag::Long(*v as i64)),
                    (Tag::LongArray(a), Seg::Index(i)) => a.get(*i).map(|v| Tag::Long(*v)),
                    _ => None,
                };
                Ok((seg, child))
            })?;
            let Some(child) = child else {
                return Ok(Value::Nil);
            };
            match scalar(lua, &child)? {
                Some(v) => Ok(v),
                None => Ok(Value::UserData(lua.create_userdata(this.child(seg))?)),
            }
        });
        methods.add_method(
            "set",
            |_, this, (key, value, ty): (Value, Value, Option<i64>)| {
                let hint = ty.map(type_from).transpose()?;
                this.with(|t| {
                    let seg = this.seg(&key, t)?;
                    match (t, seg) {
                        (Tag::Compound(c), Seg::Key(k)) => {
                            let hint = hint.or_else(|| c.get(&k).map(Tag::tag_type));
                            let new = to_tag(&value, hint)?;
                            c.insert(k, new);
                        }
                        (Tag::List(l), Seg::Index(i)) => {
                            let new = to_tag(&value, hint.or(Some(l.element_type())))?;
                            if new.tag_type() != l.element_type() {
                                return Err(rt("value type does not match the list"));
                            }
                            *l.get_mut(i)
                                .ok_or_else(|| rt("index outside the list; use push"))? = new;
                        }
                        (Tag::ByteArray(a), Seg::Index(i)) => {
                            *a.get_mut(i).ok_or_else(|| rt("index outside the array"))? =
                                to_tag(&value, Some(TagType::Long))
                                    .map(|t| t.as_i64().unwrap_or(0) as i8)?;
                        }
                        (Tag::IntArray(a), Seg::Index(i)) => {
                            *a.get_mut(i).ok_or_else(|| rt("index outside the array"))? =
                                to_tag(&value, Some(TagType::Long))
                                    .map(|t| t.as_i64().unwrap_or(0) as i32)?;
                        }
                        (Tag::LongArray(a), Seg::Index(i)) => {
                            *a.get_mut(i).ok_or_else(|| rt("index outside the array"))? =
                                to_tag(&value, Some(TagType::Long))
                                    .map(|t| t.as_i64().unwrap_or(0))?;
                        }
                        _ => return Err(rt("cannot set here")),
                    }
                    Ok(())
                })
            },
        );
        methods.add_method("push", |_, this, (value, ty): (Value, Option<i64>)| {
            let hint = ty.map(type_from).transpose()?;
            this.with(|t| match t {
                Tag::List(l) => {
                    let hint = hint.or(match l.element_type() {
                        TagType::End => None,
                        e => Some(e),
                    });
                    l.push(to_tag(&value, hint)?).map_err(ext)
                }
                _ => Err(rt("push() needs a list")),
            })
        });
        methods.add_method("remove", |_, this, key: Value| {
            this.with(|t| {
                let seg = this.seg(&key, t)?;
                match (t, seg) {
                    (Tag::Compound(c), Seg::Key(k)) => {
                        c.shift_remove(&k);
                    }
                    (Tag::List(l), Seg::Index(i)) => {
                        l.remove(i);
                    }
                    _ => return Err(rt("remove() needs a compound key or list index")),
                }
                Ok(())
            })
        });
        methods.add_method("value", |lua, this, ()| {
            let tag = this.with(|t| Ok(t.clone()))?;
            if let Some(v) = scalar(lua, &tag)? {
                return Ok(v);
            }
            let seq = |v: Vec<i64>| lua.create_sequence_from(v).map(Value::Table);
            match tag {
                Tag::ByteArray(a) => seq(a.into_iter().map(i64::from).collect()),
                Tag::IntArray(a) => seq(a.into_iter().map(i64::from).collect()),
                Tag::LongArray(a) => seq(a),
                _ => Ok(Value::Nil),
            }
        });
        methods.add_method("copy", |_, this, ()| {
            this.with(|t| Ok(NbtRef::new(t.clone())))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            this.with(|t| Ok(format!("{t}")))
        });
    }
}

// ------------------------------------------------------------------ worlds

enum WorldKind {
    Java(JavaWorld),
    Bedrock(BedrockWorld),
    Console(ConsoleSave),
}

type BedrockCache = HashMap<(i32, i32, i32, i8), Option<Rc<Vec<BlockStorage>>>>;
type JavaCache = HashMap<(String, i32, i32), Option<Rc<Chunk>>>;
type ConsoleCache = HashMap<(i32, i32, i32), Option<Rc<ConsoleChunk>>>;

struct WorldRef {
    path: PathBuf,
    edition: Edition,
    kind: WorldKind,
    java_cache: RefCell<JavaCache>,
    bedrock_cache: RefCell<BedrockCache>,
    console_cache: RefCell<ConsoleCache>,
}

const CACHE_LIMIT: usize = 1024;

/// Block name and its properties (Java) or states (Bedrock).
type BlockInfo = (String, Vec<(String, Tag)>);

fn bedrock_dim(name: &str) -> Option<mctk_bedrock::Dimension> {
    match name {
        "minecraft:overworld" => Some(mctk_bedrock::Dimension::Overworld),
        "minecraft:the_nether" => Some(mctk_bedrock::Dimension::Nether),
        "minecraft:the_end" => Some(mctk_bedrock::Dimension::End),
        _ => None,
    }
}

impl WorldRef {
    /// Block name and properties at absolute coordinates, `None` where nothing is stored.
    fn block(&self, dim: &str, x: i32, y: i32, z: i32) -> Result<Option<BlockInfo>, mlua::Error> {
        let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
        let (lx, lz) = (x.rem_euclid(16) as usize, z.rem_euclid(16) as usize);
        match &self.kind {
            WorldKind::Java(w) => {
                let mut cache = self.java_cache.borrow_mut();
                if cache.len() > CACHE_LIMIT {
                    cache.clear();
                }
                let key = (dim.to_string(), cx, cz);
                if !cache.contains_key(&key) {
                    let chunk = w
                        .chunk(dim, ChunkPos::new(cx, cz))
                        .map_err(ext)?
                        .map(Rc::new);
                    cache.insert(key.clone(), chunk);
                }
                let Some(chunk) = cache[&key].clone() else {
                    return Ok(None);
                };
                let Some(section) = chunk.section(y.div_euclid(16) as i8) else {
                    return Ok(None);
                };
                let i = mctk_java::block_index(lx, y.rem_euclid(16) as usize, lz);
                Ok(match &section.blocks {
                    Some(JavaBlocks::Palette(p)) => {
                        let b = p.get(i);
                        let props = b
                            .properties
                            .iter()
                            .map(|(k, v)| (k.clone(), Tag::String(v.clone())))
                            .collect();
                        Some((b.name.clone(), props))
                    }
                    Some(JavaBlocks::Legacy { ids, data }) => Some((
                        format!("legacy:{}", ids[i]),
                        vec![("data".into(), Tag::Byte(data[i] as i8))],
                    )),
                    None => None,
                })
            }
            WorldKind::Bedrock(w) => {
                let d = bedrock_dim(dim).ok_or_else(|| rt(format!("unknown dimension {dim}")))?;
                let sy = y.div_euclid(16) as i8;
                let mut cache = self.bedrock_cache.borrow_mut();
                if cache.len() > CACHE_LIMIT {
                    cache.clear();
                }
                let key = (d.id(), cx, cz, sy);
                if let Entry::Vacant(slot) = cache.entry(key) {
                    let k = ChunkKey {
                        x: cx,
                        z: cz,
                        dimension: d,
                        tag: ChunkTag::SubChunkPrefix,
                        subchunk: Some(sy),
                    }
                    .encode();
                    let storages = match w.db.get(&k) {
                        Some(raw) => match SubChunk::parse(raw).map_err(ext)?.body {
                            SubChunkBody::Palettized(s) => Some(Rc::new(s)),
                            SubChunkBody::Legacy(_) => None,
                        },
                        None => None,
                    };
                    slot.insert(storages);
                }
                let Some(storages) = cache[&key].clone() else {
                    return Ok(None);
                };
                let Some(s) = storages.first() else {
                    return Ok(None);
                };
                let state = &s.palette[s.indices
                    [mctk_bedrock::block_index(lx, y.rem_euclid(16) as usize, lz)]
                    as usize];
                let name = state
                    .tag
                    .get("name")
                    .and_then(Tag::as_str)
                    .unwrap_or("minecraft:air")
                    .to_string();
                let props = state
                    .tag
                    .get("states")
                    .and_then(Tag::as_compound)
                    .map(|c| c.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();
                Ok(Some((name, props)))
            }
            WorldKind::Console(save) => {
                let want = match dim {
                    "minecraft:the_nether" => -1,
                    "minecraft:the_end" => 1,
                    _ => 0,
                };
                let mut cache = self.console_cache.borrow_mut();
                if cache.len() > CACHE_LIMIT {
                    cache.clear();
                }
                let key = (want, cx, cz);
                if let Entry::Vacant(slot) = cache.entry(key) {
                    let (rx, rz) = (cx.div_euclid(32), cz.div_euclid(32));
                    let mut found = None;
                    for (d, x, z, e) in save.archive.regions() {
                        if d == want && x == rx && z == rz && !e.data.is_empty() {
                            let region = ConsoleRegion::parse(e.data.clone()).map_err(ext)?;
                            if let Some(raw) = region
                                .raw_chunk(cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize)
                                .map_err(ext)?
                            {
                                found = Some(Rc::new(
                                    ConsoleChunk::from_nbt(raw.nbt().map_err(ext)?).map_err(ext)?,
                                ));
                            }
                        }
                    }
                    slot.insert(found);
                }
                let Some(chunk) = cache[&key].clone() else {
                    return Ok(None);
                };
                Ok(chunk.block(lx, y.max(0) as usize, lz).map(|(id, data)| {
                    (
                        format!("legacy:{id}"),
                        vec![("data".to_string(), Tag::Byte(data as i8))],
                    )
                }))
            }
        }
    }
}

impl UserData for WorldRef {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("edition", |_, this, ()| {
            Ok(edition_number(&this.path, this.edition))
        });
        methods.add_method("name", |_, this, ()| {
            Ok(match &this.kind {
                WorldKind::Java(w) => w.level.name.clone(),
                WorldKind::Bedrock(w) => w
                    .level_name_txt
                    .clone()
                    .or_else(|| w.level.level_name().map(str::to_string))
                    .unwrap_or_default(),
                WorldKind::Console(_) => this
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
            })
        });
        methods.add_method("dimensions", |lua, this, ()| {
            let list = mctk_world::list_chunks(&this.path, this.edition).map_err(rt)?;
            lua.create_sequence_from(list.into_keys())
        });
        methods.add_method("chunks", |lua, this, dim: String| {
            let list = mctk_world::list_chunks(&this.path, this.edition).map_err(rt)?;
            let t = lua.create_table()?;
            for (i, c) in list.get(&dim).into_iter().flatten().enumerate() {
                let e = lua.create_table()?;
                e.set("x", c.x)?;
                e.set("z", c.z)?;
                t.set(i + 1, e)?;
            }
            Ok(t)
        });
        methods.add_method(
            "block",
            |lua, this, (dim, x, y, z): (String, i32, i32, i32)| {
                let Some((name, props)) = this.block(&dim, x, y, z)? else {
                    return Ok((Value::Nil, Value::Nil));
                };
                let t = lua.create_table()?;
                for (k, v) in props {
                    if let Some(v) = scalar(lua, &v)? {
                        t.set(k, v)?;
                    }
                }
                Ok((Value::String(lua.create_string(&name)?), Value::Table(t)))
            },
        );
        methods.add_method("level", |_, this, ()| {
            let root = match &this.kind {
                WorldKind::Java(w) => w.level.root.tag.clone(),
                WorldKind::Bedrock(w) => w.level.root.tag.clone(),
                WorldKind::Console(s) => {
                    let e = s
                        .archive
                        .get("level.dat")
                        .ok_or_else(|| rt("save has no level.dat"))?;
                    let (_, raw) = mctk_nbt::decompress(&e.data).map_err(ext)?;
                    mctk_nbt::from_bytes(&raw, Flavor::Java).map_err(ext)?.tag
                }
            };
            Ok(NbtRef::new(root))
        });
    }
}

struct DbRef(BedrockWorld);

impl UserData for DbRef {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("count", |_, this, ()| Ok(this.0.db.entries.len()));
        methods.add_method("keys", |lua, this, prefix: Option<mlua::String>| {
            let prefix = prefix.map(|p| p.as_bytes().to_vec()).unwrap_or_default();
            let t = lua.create_table()?;
            for (i, k) in this
                .0
                .db
                .entries
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .enumerate()
            {
                t.set(i + 1, lua.create_string(k)?)?;
            }
            Ok(t)
        });
        methods.add_method("get", |lua, this, key: mlua::String| {
            match this.0.db.get(&key.as_bytes()) {
                Some(v) => Ok(Value::String(lua.create_string(v)?)),
                None => Ok(Value::Nil),
            }
        });
    }
}

// ------------------------------------------------------------------ JSON

fn json_to_lua(lua: &Lua, v: &serde_json::Value) -> Result<Value, mlua::Error> {
    use serde_json::Value as J;
    Ok(match v {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Boolean(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        J::String(s) => Value::String(lua.create_string(s)?),
        J::Array(a) => {
            let t = lua.create_table()?;
            for (i, x) in a.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
        J::Object(o) => {
            let t = lua.create_table()?;
            for (k, x) in o {
                t.set(k.as_str(), json_to_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
    })
}

fn lua_to_json(v: &Value, depth: usize) -> Result<serde_json::Value, mlua::Error> {
    use serde_json::Value as J;
    if depth > 128 {
        return Err(rt("value nested too deeply for JSON"));
    }
    Ok(match v {
        Value::Nil => J::Null,
        Value::Boolean(b) => J::Bool(*b),
        Value::Integer(i) => J::from(*i),
        Value::Number(f) => serde_json::Number::from_f64(*f).map_or(J::Null, J::Number),
        Value::String(s) => J::String(s.to_str()?.to_string()),
        Value::Table(t) => {
            let len = t.raw_len();
            let pairs = t.clone().pairs::<Value, Value>().count();
            if len > 0 && len == pairs {
                J::Array(
                    (1..=len)
                        .map(|i| lua_to_json(&t.raw_get(i)?, depth + 1))
                        .collect::<Result<_, _>>()?,
                )
            } else {
                let mut o = serde_json::Map::new();
                for pair in t.clone().pairs::<Value, Value>() {
                    let (k, x) = pair?;
                    let key = match k {
                        Value::String(s) => s.to_str()?.to_string(),
                        Value::Integer(i) => i.to_string(),
                        _ => return Err(rt("JSON object keys must be strings or integers")),
                    };
                    o.insert(key, lua_to_json(&x, depth + 1)?);
                }
                J::Object(o)
            }
        }
        _ => return Err(rt("this value cannot be written as JSON")),
    })
}

// ------------------------------------------------------------------ host

pub struct Host {
    lua: Lua,
    log: Rc<RefCell<Vec<String>>>,
}

fn enum_table(lua: &Lua, entries: &[(&str, i64)]) -> Result<Table, mlua::Error> {
    let t = lua.create_table()?;
    for (k, v) in entries {
        t.set(*k, *v)?;
    }
    Ok(t)
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

impl Host {
    pub fn new(base: &Path, options: &RunOptions) -> Result<Self, Error> {
        let canon = |p: &Path| canonical(p).map_err(|e| Error::Invalid(e.to_string()));
        let base_c = canon(base)?;
        let output_c = options.output.as_deref().map(canon).transpose()?;
        let mut read_roots = vec![base_c.clone()];
        if let Some(w) = &options.world {
            read_roots.push(canon(w)?);
        }
        read_roots.extend(output_c.clone());
        let paths = Paths {
            base: base_c.clone(),
            output: output_c.clone(),
            read_roots,
            write_roots: output_c.into_iter().collect(),
        };

        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE,
            LuaOptions::default(),
        )?;
        lua.set_memory_limit(options.memory_limit)?;
        let deadline = Instant::now() + options.time_limit;
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(10_000),
            move |_, _| {
                if Instant::now() > deadline {
                    Err(rt("the extension ran past its time limit"))
                } else {
                    Ok(VmState::Continue)
                }
            },
        );
        let g = lua.globals();
        // No file access outside the APIs, and no precompiled chunks.
        g.set("dofile", Value::Nil)?;
        g.set("loadfile", Value::Nil)?;
        lua.load("local raw = load; load = function(chunk, name, _, env) return raw(chunk, name, 't', env) end")
            .set_mode(ChunkMode::Text)
            .exec()?;

        let log = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&log);
        let print = lua.create_function(move |_, args: mlua::MultiValue| {
            let parts: Vec<String> = args
                .iter()
                .map(|v| v.to_string())
                .collect::<Result<_, _>>()?;
            sink.borrow_mut().push(parts.join("\t"));
            Ok(())
        })?;
        g.set("Print", print.clone())?;
        g.set("print", print)?;
        g.set("BASE_PATH", base_c.display().to_string())?;
        g.set(
            "UTF8ToPercentEncoding",
            lua.create_function(|_, s: String| Ok(percent_encode(&s)))?,
        )?;
        g.set(
            "EDITION",
            enum_table(
                &lua,
                &[
                    ("JAVA", 0),
                    ("BEDROCK", 1),
                    ("XBOX360", 2),
                    ("PS3", 3),
                    ("WIIU", 4),
                ],
            )?,
        )?;
        g.set(
            "TYPE",
            enum_table(
                &lua,
                &[
                    ("END", 0),
                    ("BYTE", 1),
                    ("SHORT", 2),
                    ("INT", 3),
                    ("LONG", 4),
                    ("FLOAT", 5),
                    ("DOUBLE", 6),
                    ("BYTE_ARRAY", 7),
                    ("STRING", 8),
                    ("LIST", 9),
                    ("COMPOUND", 10),
                    ("INT_ARRAY", 11),
                    ("LONG_ARRAY", 12),
                ],
            )?,
        )?;
        g.set(
            "JSON_TYPE",
            enum_table(
                &lua,
                &[
                    ("NULL", 0),
                    ("BOOL", 1),
                    ("NUMBER", 2),
                    ("STRING", 3),
                    ("ARRAY", 4),
                    ("OBJECT", 5),
                ],
            )?,
        )?;
        let ext_types = lua.create_table()?;
        for t in ["NBT_EDITOR_STYLE", "WORLD_TOOL"] {
            ext_types.set(t, t)?;
        }
        g.set("EXTENSION_TYPE", ext_types)?;

        // NBT
        let nbt = lua.create_table()?;
        let p = paths.clone();
        nbt.set(
            "read",
            lua.create_function(move |lua, path: String| {
                let path = p.check(&path, false)?;
                let bytes = std::fs::read(&path).map_err(ext)?;
                let info = lua.create_table()?;
                // Bedrock level.dat: 8-byte header, then little-endian NBT.
                if bytes.len() > 9
                    && u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize
                        == bytes.len() - 8
                    && bytes[8] == 0x0a
                {
                    let root = mctk_nbt::from_bytes(&bytes[8..], Flavor::Bedrock).map_err(ext)?;
                    info.set("flavor", "bedrock")?;
                    info.set("header", true)?;
                    return Ok((NbtRef::new(root.tag), info));
                }
                let (c, raw) = mctk_nbt::decompress(&bytes).map_err(ext)?;
                let (root, flavor) = match mctk_nbt::from_bytes(&raw, Flavor::Java) {
                    Ok(r) => (r, "java"),
                    Err(_) => (
                        mctk_nbt::from_bytes(&raw, Flavor::Bedrock).map_err(ext)?,
                        "bedrock",
                    ),
                };
                info.set("flavor", flavor)?;
                info.set("compression", format!("{c:?}").to_lowercase())?;
                info.set("name", root.name.clone())?;
                Ok((NbtRef::new(root.tag), info))
            })?,
        )?;
        let p = paths.clone();
        nbt.set(
            "write",
            lua.create_function(
                move |_, (path, tag, opts): (String, AnyUserData, Option<Table>)| {
                    let path = p.check(&path, true)?;
                    let r = tag.borrow::<NbtRef>()?;
                    let value = r.with(|t| Ok(t.clone()))?;
                    let get = |k: &str| -> Result<Option<String>, mlua::Error> {
                        opts.as_ref()
                            .map(|o| o.get::<Option<String>>(k))
                            .transpose()
                            .map(Option::flatten)
                    };
                    let flavor = match get("flavor")?.as_deref() {
                        Some("bedrock") => Flavor::Bedrock,
                        _ => Flavor::Java,
                    };
                    let header = opts
                        .as_ref()
                        .map(|o| o.get::<Option<bool>>("header"))
                        .transpose()?
                        .flatten()
                        .unwrap_or(false);
                    let compression = match get("compression")?.as_deref() {
                        Some("none") => Compression::None,
                        Some("zlib") => Compression::Zlib,
                        Some("gzip") => Compression::Gzip,
                        None if flavor == Flavor::Bedrock => Compression::None,
                        _ => Compression::Gzip,
                    };
                    let root = NamedTag {
                        name: get("name")?.unwrap_or_default(),
                        tag: value,
                    };
                    let raw = mctk_nbt::to_bytes(&root, flavor).map_err(ext)?;
                    let bytes = if header {
                        let mut out = 10u32.to_le_bytes().to_vec();
                        out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
                        out.extend_from_slice(&raw);
                        out
                    } else {
                        match compression {
                            Compression::None => raw,
                            Compression::Gzip => {
                                let mut e = flate2::write::GzEncoder::new(
                                    Vec::new(),
                                    flate2::Compression::default(),
                                );
                                e.write_all(&raw).map_err(ext)?;
                                e.finish().map_err(ext)?
                            }
                            Compression::Zlib => {
                                let mut e = flate2::write::ZlibEncoder::new(
                                    Vec::new(),
                                    flate2::Compression::default(),
                                );
                                e.write_all(&raw).map_err(ext)?;
                                e.finish().map_err(ext)?
                            }
                        }
                    };
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).map_err(ext)?;
                    }
                    std::fs::write(&path, bytes).map_err(ext)
                },
            )?,
        )?;
        nbt.set(
            "compound",
            lua.create_function(|_, ()| Ok(NbtRef::new(Tag::Compound(Compound::new()))))?,
        )?;
        nbt.set(
            "list",
            lua.create_function(|_, ty: Option<i64>| {
                Ok(NbtRef::new(Tag::List(List::new(
                    ty.map(type_from).transpose()?.unwrap_or(TagType::End),
                ))))
            })?,
        )?;
        g.set("NBT", nbt)?;

        // World
        let world = lua.create_table()?;
        let p = paths.clone();
        world.set(
            "open",
            lua.create_function(move |_, path: String| {
                let path = p.check(&path, false)?;
                let edition =
                    mctk_world::detect(&path).ok_or_else(|| rt("not a Minecraft world"))?;
                let kind = match edition {
                    Edition::Java => WorldKind::Java(JavaWorld::open(&path).map_err(ext)?),
                    Edition::Bedrock => WorldKind::Bedrock(BedrockWorld::open(&path).map_err(ext)?),
                    Edition::Console => WorldKind::Console(ConsoleSave::open(&path).map_err(ext)?),
                };
                Ok(WorldRef {
                    path,
                    edition,
                    kind,
                    java_cache: Default::default(),
                    bedrock_cache: Default::default(),
                    console_cache: Default::default(),
                })
            })?,
        )?;
        g.set("World", world)?;

        // Database
        let database = lua.create_table()?;
        let p = paths.clone();
        database.set(
            "open",
            lua.create_function(move |_, path: String| {
                let path = p.check(&path, false)?;
                Ok(DbRef(BedrockWorld::open(&path).map_err(ext)?))
            })?,
        )?;
        g.set("Database", database)?;

        // Files (text only, same folder rules)
        let files = lua.create_table()?;
        let p = paths.clone();
        files.set(
            "read_text",
            lua.create_function(move |_, path: String| {
                std::fs::read_to_string(p.check(&path, false)?).map_err(ext)
            })?,
        )?;
        let p = paths.clone();
        files.set(
            "write_text",
            lua.create_function(move |_, (path, text): (String, mlua::String)| {
                let path = p.check(&path, true)?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(ext)?;
                }
                std::fs::write(path, text.as_bytes()).map_err(ext)
            })?,
        )?;
        let p = paths.clone();
        files.set(
            "exists",
            lua.create_function(move |_, path: String| {
                Ok(p.check(&path, false).is_ok_and(|c| c.exists()))
            })?,
        )?;
        let p = paths.clone();
        files.set(
            "list",
            lua.create_function(move |lua, path: String| {
                let dir = p.check(&path, false)?;
                let mut names: Vec<String> = std::fs::read_dir(dir)
                    .map_err(ext)?
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect();
                names.sort();
                lua.create_sequence_from(names)
            })?,
        )?;
        g.set("Files", files)?;

        // JSON
        let json = lua.create_table()?;
        json.set(
            "decode",
            lua.create_function(|lua, text: String| {
                let v: serde_json::Value = serde_json::from_str(&text).map_err(ext)?;
                json_to_lua(lua, &v)
            })?,
        )?;
        json.set(
            "encode",
            lua.create_function(|_, (v, pretty): (Value, Option<bool>)| {
                let j = lua_to_json(&v, 0)?;
                if pretty.unwrap_or(false) {
                    serde_json::to_string_pretty(&j).map_err(ext)
                } else {
                    serde_json::to_string(&j).map_err(ext)
                }
            })?,
        )?;
        g.set("JSON", json)?;
        drop(g);
        Ok(Host { lua, log })
    }

    pub fn lua(&self) -> Result<&Lua, Error> {
        Ok(&self.lua)
    }

    pub fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut self.log.borrow_mut())
    }
}
