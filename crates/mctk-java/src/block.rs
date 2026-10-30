use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use indexmap::IndexMap;
use mctk_nbt::{Compound, Tag};

/// A block state: namespaced id plus string properties, e.g. `minecraft:oak_log[axis=y]`.
///
/// Properties keep the order they were read in, so chunks round-trip byte for byte, but
/// equality, hashing and ordering ignore property order.
#[derive(Debug, Clone)]
pub struct BlockState {
    pub name: String,
    pub properties: IndexMap<String, String>,
}

impl PartialEq for BlockState {
    fn eq(&self, other: &Self) -> bool {
        // IndexMap equality is order-insensitive.
        self.name == other.name && self.properties == other.properties
    }
}

impl Eq for BlockState {}

impl Hash for BlockState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.sorted_properties().hash(state);
    }
}

impl PartialOrd for BlockState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BlockState {
    fn cmp(&self, other: &Self) -> Ordering {
        self.name
            .cmp(&other.name)
            .then_with(|| self.sorted_properties().cmp(&other.sorted_properties()))
    }
}

impl BlockState {
    pub fn new(name: impl Into<String>) -> Self {
        BlockState {
            name: name.into(),
            properties: IndexMap::new(),
        }
    }

    fn sorted_properties(&self) -> Vec<(&String, &String)> {
        let mut v: Vec<_> = self.properties.iter().collect();
        v.sort();
        v
    }

    pub fn air() -> Self {
        BlockState::new("minecraft:air")
    }

    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.insert(key.into(), value.into());
        self
    }

    pub fn is_air(&self) -> bool {
        matches!(
            self.name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        )
    }

    /// Parses any palette entry form:
    /// - `"minecraft:stone"` (26.x string entry)
    /// - `{"": "minecraft:stone"}` (26.x entry wrapped inside a mixed list)
    /// - `{id: "...", properties: {...}}` (26.x)
    /// - `{Name: "...", Properties: {...}}` (1.13 – 1.21)
    pub fn from_palette_entry(tag: &Tag) -> Option<Self> {
        match tag {
            Tag::String(s) => Some(BlockState::new(s.clone())),
            Tag::Compound(c) => {
                if let Some(inner) = c.get("") {
                    return BlockState::from_palette_entry(inner);
                }
                let name = c.get("id").or_else(|| c.get("Name"))?.as_str()?.to_string();
                let mut properties = IndexMap::new();
                if let Some(props) = c
                    .get("properties")
                    .or_else(|| c.get("Properties"))
                    .and_then(Tag::as_compound)
                {
                    for (k, v) in props {
                        properties.insert(k.clone(), v.as_str()?.to_string());
                    }
                }
                Some(BlockState { name, properties })
            }
            _ => None,
        }
    }

    /// Classic `{Name, Properties}` compound used by 1.13 – 1.21.
    pub fn to_named_compound(&self) -> Tag {
        let mut c = Compound::new();
        c.insert("Name".into(), Tag::String(self.name.clone()));
        if !self.properties.is_empty() {
            c.insert("Properties".into(), Tag::Compound(self.props_compound()));
        }
        Tag::Compound(c)
    }

    /// 26.x entry: a bare string when there are no properties, otherwise `{id, properties}`.
    /// Mixed lists must wrap the bare strings; see [`crate::chunk`] for the list encoding.
    pub fn to_id_entry(&self) -> Tag {
        if self.properties.is_empty() {
            return Tag::String(self.name.clone());
        }
        let mut c = Compound::new();
        c.insert("id".into(), Tag::String(self.name.clone()));
        c.insert("properties".into(), Tag::Compound(self.props_compound()));
        Tag::Compound(c)
    }

    fn props_compound(&self) -> Compound {
        self.properties
            .iter()
            .map(|(k, v)| (k.clone(), Tag::String(v.clone())))
            .collect()
    }
}

impl fmt::Display for BlockState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        if !self.properties.is_empty() {
            f.write_str("[")?;
            for (i, (k, v)) in self.properties.iter().enumerate() {
                if i > 0 {
                    f.write_str(",")?;
                }
                write!(f, "{k}={v}")?;
            }
            f.write_str("]")?;
        }
        Ok(())
    }
}
