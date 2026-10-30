//! NBT (Named Binary Tag) codec.
//!
//! Java Edition and Legacy Console Edition store NBT big-endian with strings in Java's
//! modified UTF-8; Bedrock Edition stores it little-endian with plain UTF-8 strings.
//! [`Flavor`] selects between the two. Compound key order is preserved so files round-trip.

mod compression;
mod mutf8;
mod read;
mod snbt;
mod write;

use indexmap::IndexMap;

pub use compression::{Compression, decompress, read_file};
pub use read::{from_bytes, read_root};
pub use write::{to_bytes, write_root};

/// Byte order and string encoding of a serialized NBT stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// Java / Legacy Console: big-endian, modified UTF-8 strings.
    Java,
    /// Bedrock: little-endian, UTF-8 strings.
    Bedrock,
}

/// NBT tag type ids as stored on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TagType {
    End = 0,
    Byte = 1,
    Short = 2,
    Int = 3,
    Long = 4,
    Float = 5,
    Double = 6,
    ByteArray = 7,
    String = 8,
    List = 9,
    Compound = 10,
    IntArray = 11,
    LongArray = 12,
}

impl TagType {
    pub fn from_id(id: u8) -> Option<Self> {
        use TagType::*;
        Some(match id {
            0 => End,
            1 => Byte,
            2 => Short,
            3 => Int,
            4 => Long,
            5 => Float,
            6 => Double,
            7 => ByteArray,
            8 => String,
            9 => List,
            10 => Compound,
            11 => IntArray,
            12 => LongArray,
            _ => return None,
        })
    }
}

/// An ordered map of named tags.
pub type Compound = IndexMap<String, Tag>;

/// A list of tags that all share one element type.
///
/// The element type is kept even for empty lists, because files distinguish an empty
/// list of compounds from an empty list of strings.
#[derive(Debug, Clone, PartialEq)]
pub struct List {
    element: TagType,
    items: Vec<Tag>,
}

impl List {
    pub fn new(element: TagType) -> Self {
        List {
            element,
            items: Vec::new(),
        }
    }

    /// Builds a list from tags, which must all have the same type.
    pub fn from_tags(element: TagType, items: Vec<Tag>) -> Result<Self, Error> {
        if let Some(bad) = items.iter().find(|t| t.tag_type() != element) {
            return Err(Error::ListTypeMismatch {
                expected: element,
                found: bad.tag_type(),
            });
        }
        Ok(List { element, items })
    }

    pub fn element_type(&self) -> TagType {
        self.element
    }

    pub fn push(&mut self, tag: Tag) -> Result<(), Error> {
        if self.items.is_empty() && self.element == TagType::End {
            self.element = tag.tag_type();
        }
        if tag.tag_type() != self.element {
            return Err(Error::ListTypeMismatch {
                expected: self.element,
                found: tag.tag_type(),
            });
        }
        self.items.push(tag);
        Ok(())
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Tag> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Tag> {
        self.items.get(index)
    }

    /// Mutable access to an element. Editors may change its value but must keep its
    /// type, which the list's element type promises.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Tag> {
        self.items.get_mut(index)
    }

    /// Mutable iteration; the same type rule as [`List::get_mut`] applies.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Tag> {
        self.items.iter_mut()
    }

    pub fn remove(&mut self, index: usize) -> Option<Tag> {
        (index < self.items.len()).then(|| self.items.remove(index))
    }

    pub fn swap(&mut self, a: usize, b: usize) {
        if a < self.items.len() && b < self.items.len() {
            self.items.swap(a, b);
        }
    }
}

/// A single NBT value.
#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(List),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn tag_type(&self) -> TagType {
        match self {
            Tag::Byte(_) => TagType::Byte,
            Tag::Short(_) => TagType::Short,
            Tag::Int(_) => TagType::Int,
            Tag::Long(_) => TagType::Long,
            Tag::Float(_) => TagType::Float,
            Tag::Double(_) => TagType::Double,
            Tag::ByteArray(_) => TagType::ByteArray,
            Tag::String(_) => TagType::String,
            Tag::List(_) => TagType::List,
            Tag::Compound(_) => TagType::Compound,
            Tag::IntArray(_) => TagType::IntArray,
            Tag::LongArray(_) => TagType::LongArray,
        }
    }

    pub fn as_compound(&self) -> Option<&Compound> {
        match self {
            Tag::Compound(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_compound_mut(&mut self) -> Option<&mut Compound> {
        match self {
            Tag::Compound(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&List> {
        match self {
            Tag::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    /// Any integer tag widened to `i64`.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Tag::Byte(v) => Some(v.into()),
            Tag::Short(v) => Some(v.into()),
            Tag::Int(v) => Some(v.into()),
            Tag::Long(v) => Some(v),
            _ => None,
        }
    }

    /// The value of any numeric tag (integers included) as a double.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Tag::Float(v) => Some(v.into()),
            Tag::Double(v) => Some(v),
            _ => self.as_i64().map(|v| v as f64),
        }
    }

    /// Looks up a child by name when this tag is a compound.
    pub fn get(&self, name: &str) -> Option<&Tag> {
        self.as_compound()?.get(name)
    }
}

/// A root tag together with its name (usually empty).
#[derive(Debug, Clone, PartialEq)]
pub struct NamedTag {
    pub name: String,
    pub tag: Tag,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unknown tag type id {0}")]
    UnknownTagType(u8),
    #[error("root tag must be a compound or list, found {0:?}")]
    BadRoot(TagType),
    #[error("negative or oversized length {0}")]
    BadLength(i64),
    #[error("nesting deeper than {0} levels")]
    TooDeep(usize),
    #[error("invalid string data: {0}")]
    BadString(String),
    #[error("list expects {expected:?} elements, found {found:?}")]
    ListTypeMismatch { expected: TagType, found: TagType },
    #[error("unexpected TAG_End inside a list of {0} elements")]
    EndInList(usize),
}

/// Maximum nesting depth accepted when reading, matching the limit Minecraft enforces.
pub const MAX_DEPTH: usize = 512;
