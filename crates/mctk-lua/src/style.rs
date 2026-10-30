//! NBT editor styling: runs `NBT_EDITOR_STYLE` extensions — the same Lua scripts UMT's
//! NBT editor runs, such as Player Info or Entity Info — and collects the labels, label
//! colours and icons they attach to tags.
//!
//! Script API (as those extensions use it): a script returns a table whose `styler` field
//! has `recursive`, `main(self, root, context)` (called once) and, for recursive stylers,
//! `recursion(self, root, tag, context)` (called for every tag). Tags are tables with `name`, `type`, `value`, `childCount`,
//! `listType`, `parentTag`, `lastFound` and methods `contains(name, TYPE.x[, TYPE.y])`,
//! `child(i)` (0-based), `getSize()`, `getInt(byteOffset)`, `getString()`. `Style:setLabel`,
//! `Style:setLabelColor` and `Style:setIcon` (a path relative to the extensions folder)
//! style a tag. `context` has `edition` (EDITION.JAVA/BEDROCK/CONSOLE) and `type`
//! (FILE_TYPE flags). `JSONValue.new()` parses JSON text, `os.date` formats times, and
//! `Database:find` answers nothing (UMT's online database is not part of MCToolKit).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use mctk_nbt::Tag;
use mlua::{Function, Lua, Table, Value};

use crate::{Error, Extension, RunOptions};

/// What extensions attached to one tag.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagStyle {
    pub label: Option<String>,
    /// A colour name ("lime", "gold", …) or `#rrggbb`.
    pub color: Option<String>,
    /// Image path relative to the extensions folder.
    pub icon: Option<String>,
}

/// Address of a tag below the root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSeg {
    Key(String),
    Index(usize),
}

pub mod file_type {
    pub const LEVEL: u32 = 1;
    pub const PLAYER: u32 = 2;
    pub const CHUNK: u32 = 4;
    pub const MAP: u32 = 8;
    pub const OTHER: u32 = 16;
}

/// `EDITION` values seen by stylers.
pub mod edition {
    pub const JAVA: i64 = 0;
    pub const BEDROCK: i64 = 1;
    pub const CONSOLE: i64 = 2;
}

pub type Styles = HashMap<Vec<PathSeg>, TagStyle>;

const PRELUDE: &str = r#"
FILE_TYPE = { LEVEL = 1, PLAYER = 2, CHUNK = 4, MAP = 8, OTHER = 16 }
EDITION.CONSOLE = 2
BLOCK_FORMAT = { ID_DATA = 0, NAME_DATA = 1, NAME_STATES = 2 }
-- UMT's online database is not available: every lookup is a miss.
Database = { find = function() return { valid = false, name = "" } end }

local Tag = {}
TagMeta = { __index = Tag }
function Tag:contains(name, ty, listTy)
  local c = self.__keys and self.__keys[name]
  if c ~= nil and (ty == nil or c.type == ty) and (listTy == nil or c.listType == listTy) then
    self.lastFound = c
    return true
  end
  return false
end
function Tag:child(i) return self.__children and self.__children[i + 1] end
function Tag:getString() return tostring(self.value) end
function Tag:getSize()
  if self.__arr then return #self.__arr * self.__width end
  return self.childCount or 0
end
local function element(self, offset, width)
  if not self.__arr then return nil end
  if self.__width == width then return self.__arr[offset // width + 1] end
  -- Big-endian composition from byte arrays.
  local v = 0
  for k = 0, width - 1 do
    local b = self.__arr[offset + k + 1] or 0
    v = (v << 8) | (b & 0xff)
  end
  if width == 4 and v >= 0x80000000 then v = v - 0x100000000 end
  return v
end
function Tag:getInt(offset) return element(self, offset, 4) end
function Tag:getByte(offset) return element(self, offset, 1) end
function Tag:getLong(offset) return element(self, offset, 8) end

JSONValue = {}
JSONValue.__index = JSONValue
function JSONValue.new() return setmetatable({ type = JSON_TYPE.NULL }, JSONValue) end
local function wrapJson(v)
  return setmetatable(v, JSONValue)
end
function JSONValue:parse(text)
  local ok, v = pcall(JSON.decodeTyped, text)
  if ok and v then
    for k, x in pairs(v) do self[k] = x end
  else
    self.type = JSON_TYPE.NULL
  end
  return self
end
function JSONValue:contains(key, ty)
  local m = self.members and self.members[key]
  if m ~= nil and (ty == nil or m.type == ty) then
    self.lastFound = wrapJson(m)
    return true
  end
  return false
end
function JSONValue:child(i)
  local c = self.items and self.items[i + 1]
  return c and wrapJson(c)
end
function JSONValue:getString() return tostring(self.value or "") end
function JSONValue:getInt() return math.tointeger(self.value) or 0 end
function JSONValue:getDouble() return tonumber(self.value) or 0 end
function JSONValue:getBool() return self.value == true end
"#;

fn json_typed(lua: &Lua, v: &serde_json::Value) -> mlua::Result<Table> {
    use serde_json::Value as J;
    let t = lua.create_table()?;
    let (ty, value): (i64, Value) = match v {
        J::Null => (0, Value::Nil),
        J::Bool(b) => (1, Value::Boolean(*b)),
        J::Number(n) => (
            2,
            n.as_i64()
                .map(Value::Integer)
                .unwrap_or(Value::Number(n.as_f64().unwrap_or(0.0))),
        ),
        J::String(s) => (3, Value::String(lua.create_string(s)?)),
        J::Array(a) => {
            let items = lua.create_table()?;
            for (i, x) in a.iter().enumerate() {
                items.set(i + 1, json_typed(lua, x)?)?;
            }
            t.set("items", items)?;
            t.set("childCount", a.len())?;
            (4, Value::Nil)
        }
        J::Object(o) => {
            let members = lua.create_table()?;
            for (k, x) in o {
                members.set(k.as_str(), json_typed(lua, x)?)?;
            }
            t.set("members", members)?;
            t.set("childCount", o.len())?;
            (5, Value::Nil)
        }
    };
    t.set("type", ty)?;
    t.set("value", value)?;
    Ok(t)
}

/// `os.date` for the formats extensions use (`!` = UTC; local time is shown as UTC too).
fn os_date(fmt: &str, t: i64) -> String {
    let fmt = fmt.trim_start_matches('!');
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const DAYS: [&str; 7] = [
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
    ];
    let (h, min, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    let mut out = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('B') => out.push_str(MONTHS[(m - 1) as usize]),
            Some('b') => out.push_str(&MONTHS[(m - 1) as usize][..3]),
            Some('A') => out.push_str(DAYS[days.rem_euclid(7) as usize]),
            Some('a') => out.push_str(&DAYS[days.rem_euclid(7) as usize][..3]),
            Some('d') => out.push_str(&format!("{d:02}")),
            Some('m') => out.push_str(&format!("{m:02}")),
            Some('Y') => out.push_str(&y.to_string()),
            Some('y') => out.push_str(&format!("{:02}", y % 100)),
            Some('H') => out.push_str(&format!("{h:02}")),
            Some('I') => out.push_str(&format!("{h12:02}")),
            Some('M') => out.push_str(&format!("{min:02}")),
            Some('S') => out.push_str(&format!("{s:02}")),
            Some('p') => out.push_str(if h < 12 { "AM" } else { "PM" }),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

struct Builder<'a> {
    lua: &'a Lua,
    meta: Table,
    paths: Vec<Vec<PathSeg>>,
    tables: Vec<Table>,
}

impl Builder<'_> {
    fn build(
        &mut self,
        tag: &Tag,
        name: &str,
        parent: Option<&Table>,
        path: Vec<PathSeg>,
    ) -> mlua::Result<Table> {
        let lua = self.lua;
        let t = lua.create_table()?;
        t.set_metatable(Some(self.meta.clone()));
        t.set("name", name)?;
        t.set("type", tag.tag_type() as u8 as i64)?;
        t.set("__id", self.paths.len())?;
        if let Some(p) = parent {
            t.set("parentTag", p.clone())?;
        }
        self.paths.push(path.clone());
        self.tables.push(t.clone());
        let arr = |values: Vec<i64>, width: i64| -> mlua::Result<()> {
            t.set("__arr", lua.create_sequence_from(values)?)?;
            t.set("__width", width)?;
            Ok(())
        };
        match tag {
            Tag::Byte(v) => t.set("value", *v as i64)?,
            Tag::Short(v) => t.set("value", *v as i64)?,
            Tag::Int(v) => t.set("value", *v as i64)?,
            Tag::Long(v) => t.set("value", *v)?,
            Tag::Float(v) => t.set("value", *v as f64)?,
            Tag::Double(v) => t.set("value", *v)?,
            Tag::String(s) => t.set("value", s.as_str())?,
            Tag::ByteArray(a) => {
                t.set("childCount", a.len())?;
                arr(a.iter().map(|&v| v as i64).collect(), 1)?;
            }
            Tag::IntArray(a) => {
                t.set("childCount", a.len())?;
                arr(a.iter().map(|&v| v as i64).collect(), 4)?;
            }
            Tag::LongArray(a) => {
                t.set("childCount", a.len())?;
                arr(a.clone(), 8)?;
            }
            Tag::List(l) => {
                t.set("childCount", l.len())?;
                t.set("listType", l.element_type() as u8 as i64)?;
                let children = lua.create_table()?;
                for (i, c) in l.iter().enumerate() {
                    let mut p = path.clone();
                    p.push(PathSeg::Index(i));
                    children.set(i + 1, self.build(c, "", Some(&t), p)?)?;
                }
                t.set("__children", children)?;
            }
            Tag::Compound(c) => {
                t.set("childCount", c.len())?;
                let (children, keys) = (lua.create_table()?, lua.create_table()?);
                for (i, (k, v)) in c.iter().enumerate() {
                    let mut p = path.clone();
                    p.push(PathSeg::Key(k.clone()));
                    let child = self.build(v, k, Some(&t), p)?;
                    children.set(i + 1, child.clone())?;
                    keys.set(k.as_str(), child)?;
                }
                t.set("__children", children)?;
                t.set("__keys", keys)?;
            }
        }
        Ok(t)
    }
}

/// Runs every given style extension over `root`; returns the styles and, per failing
/// extension, its error.
pub fn style_tree(
    extensions: &[Extension],
    root: &Tag,
    ed: i64,
    file_type: u32,
) -> (Styles, Vec<String>) {
    let mut styles = Styles::new();
    let mut errors = Vec::new();
    for ext in extensions {
        if let Err(e) = run_styler(ext, root, ed, file_type, &mut styles) {
            errors.push(format!("{}: {e}", ext.manifest.name));
        }
    }
    (styles, errors)
}

fn run_styler(
    ext: &Extension,
    root: &Tag,
    ed: i64,
    file_type: u32,
    styles: &mut Styles,
) -> Result<(), Error> {
    let options = RunOptions {
        time_limit: Duration::from_secs(5),
        ..Default::default()
    };
    let host = crate::api::Host::new(&ext.dir, &options)?;
    let lua = host.lua()?;
    let g = lua.globals();
    lua.load(PRELUDE)
        .set_name("mctoolkit-style-prelude")
        .exec()?;
    let json: Table = g.get("JSON")?;
    json.set(
        "decodeTyped",
        lua.create_function(|lua, text: String| {
            let v: serde_json::Value =
                serde_json::from_str(&text).map_err(mlua::Error::external)?;
            json_typed(lua, &v)
        })?,
    )?;
    let os = lua.create_table()?;
    os.set(
        "date",
        lua.create_function(|_, (fmt, t): (Option<String>, Option<i64>)| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            Ok(os_date(fmt.as_deref().unwrap_or("%c"), t.unwrap_or(now)))
        })?,
    )?;
    os.set(
        "time",
        lua.create_function(|_, ()| {
            Ok(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64))
        })?,
    )?;
    os.set("clock", lua.create_function(|_, ()| Ok(0.0))?)?;
    g.set("os", os)?;

    let collected: Rc<RefCell<HashMap<usize, TagStyle>>> = Rc::default();
    let style = lua.create_table()?;
    for (method, field) in [("setLabel", 0), ("setLabelColor", 1), ("setIcon", 2)] {
        let sink = Rc::clone(&collected);
        style.set(
            method,
            lua.create_function(move |_, (_this, tag, text): (Value, Table, Value)| {
                let id: usize = tag.get("__id")?;
                let text = match text {
                    Value::Nil => None,
                    other => Some(other.to_string()?),
                };
                let mut map = sink.borrow_mut();
                let s = map.entry(id).or_default();
                match field {
                    0 => s.label = text,
                    1 => s.color = text,
                    _ => s.icon = text,
                }
                Ok(())
            })?,
        )?;
    }
    g.set("Style", style)?;

    let meta: Table = g.get("TagMeta")?;
    let mut b = Builder {
        lua,
        meta,
        paths: Vec::new(),
        tables: Vec::new(),
    };
    let root_table = b.build(root, "", None, Vec::new())?;

    let script = std::fs::read_to_string(ext.dir.join(&ext.manifest.entry))?;
    let module: Value = lua
        .load(&script)
        .set_name(ext.manifest.entry.as_str())
        .eval()?;
    let module = match module {
        Value::Table(t) => t,
        _ => {
            return Err(Error::Invalid(
                "the script did not return its extension table".into(),
            ));
        }
    };
    let styler: Table = module.get("styler")?;
    let main: Function = styler.get("main")?;
    let recursive: bool = styler.get::<Option<bool>>("recursive")?.unwrap_or(false);
    let ctx = lua.create_table()?;
    ctx.set("edition", ed)?;
    ctx.set("type", file_type)?;
    // As in UMT: main(root) once, then recursion(root, tag) for every tag.
    main.call::<()>((styler.clone(), root_table.clone(), ctx.clone()))?;
    if recursive {
        if let Some(recursion) = styler.get::<Option<Function>>("recursion")? {
            for t in &b.tables {
                recursion.call::<()>((
                    styler.clone(),
                    root_table.clone(),
                    t.clone(),
                    ctx.clone(),
                ))?;
            }
        }
    }
    for (id, s) in collected.borrow().iter() {
        let entry = styles.entry(b.paths[*id].clone()).or_default();
        if s.label.is_some() {
            entry.label = s.label.clone();
        }
        if s.color.is_some() {
            entry.color = s.color.clone();
        }
        if s.icon.is_some() {
            entry.icon = s.icon.clone();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_date_matches_lua() {
        assert_eq!(
            os_date("!%B %d, %Y, %I:%M%p", 1_793_245_010),
            "October 29, 2026, 03:36AM"
        );
    }
}
