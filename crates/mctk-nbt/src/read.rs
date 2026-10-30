use std::io::Read;

use crate::{Compound, Error, Flavor, List, MAX_DEPTH, NamedTag, Tag, TagType, mutf8};

/// Reads one named root tag (compound or list) from a stream.
pub fn read_root<R: Read>(reader: &mut R, flavor: Flavor) -> Result<NamedTag, Error> {
    let mut r = Reader {
        inner: reader,
        flavor,
    };
    let id = r.u8()?;
    let ty = TagType::from_id(id).ok_or(Error::UnknownTagType(id))?;
    if !matches!(ty, TagType::Compound | TagType::List) {
        return Err(Error::BadRoot(ty));
    }
    let name = r.string()?;
    let tag = r.payload(ty, 0)?;
    Ok(NamedTag { name, tag })
}

/// Reads one named root tag from a byte slice.
pub fn from_bytes(bytes: &[u8], flavor: Flavor) -> Result<NamedTag, Error> {
    read_root(&mut &bytes[..], flavor)
}

struct Reader<'a, R> {
    inner: &'a mut R,
    flavor: Flavor,
}

macro_rules! num {
    ($name:ident, $ty:ty) => {
        fn $name(&mut self) -> Result<$ty, Error> {
            let mut buf = [0u8; std::mem::size_of::<$ty>()];
            self.inner.read_exact(&mut buf)?;
            Ok(match self.flavor {
                Flavor::Java => <$ty>::from_be_bytes(buf),
                Flavor::Bedrock => <$ty>::from_le_bytes(buf),
            })
        }
    };
}

impl<R: Read> Reader<'_, R> {
    fn u8(&mut self) -> Result<u8, Error> {
        let mut b = [0u8; 1];
        self.inner.read_exact(&mut b)?;
        Ok(b[0])
    }

    num!(i16, i16);
    num!(u16, u16);
    num!(i32, i32);
    num!(i64, i64);
    num!(f32, f32);
    num!(f64, f64);

    fn len(&mut self) -> Result<usize, Error> {
        let n = self.i32()?;
        usize::try_from(n).map_err(|_| Error::BadLength(n.into()))
    }

    fn bytes(&mut self, n: usize) -> Result<Vec<u8>, Error> {
        // Read through `take` so a corrupt length cannot force a huge allocation up front.
        let mut buf = Vec::with_capacity(n.min(1 << 20));
        let got = self.inner.by_ref().take(n as u64).read_to_end(&mut buf)?;
        if got != n {
            return Err(Error::Io(std::io::ErrorKind::UnexpectedEof.into()));
        }
        Ok(buf)
    }

    fn string(&mut self) -> Result<String, Error> {
        let n = self.u16()? as usize;
        let raw = self.bytes(n)?;
        match self.flavor {
            Flavor::Java => mutf8::decode(&raw),
            Flavor::Bedrock => String::from_utf8(raw).map_err(|e| Error::BadString(e.to_string())),
        }
    }

    fn payload(&mut self, ty: TagType, depth: usize) -> Result<Tag, Error> {
        if depth > MAX_DEPTH {
            return Err(Error::TooDeep(MAX_DEPTH));
        }
        Ok(match ty {
            TagType::End => return Err(Error::UnknownTagType(0)),
            TagType::Byte => Tag::Byte(self.u8()? as i8),
            TagType::Short => Tag::Short(self.i16()?),
            TagType::Int => Tag::Int(self.i32()?),
            TagType::Long => Tag::Long(self.i64()?),
            TagType::Float => Tag::Float(self.f32()?),
            TagType::Double => Tag::Double(self.f64()?),
            TagType::ByteArray => {
                let n = self.len()?;
                Tag::ByteArray(self.bytes(n)?.into_iter().map(|b| b as i8).collect())
            }
            TagType::String => Tag::String(self.string()?),
            TagType::List => {
                let id = self.u8()?;
                let element = TagType::from_id(id).ok_or(Error::UnknownTagType(id))?;
                let n = self.len()?;
                if element == TagType::End && n > 0 {
                    return Err(Error::EndInList(n));
                }
                let mut items = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    items.push(self.payload(element, depth + 1)?);
                }
                Tag::List(List { element, items })
            }
            TagType::Compound => {
                let mut map = Compound::new();
                loop {
                    let id = self.u8()?;
                    let ty = TagType::from_id(id).ok_or(Error::UnknownTagType(id))?;
                    if ty == TagType::End {
                        break;
                    }
                    let name = self.string()?;
                    let value = self.payload(ty, depth + 1)?;
                    map.insert(name, value);
                }
                Tag::Compound(map)
            }
            TagType::IntArray => {
                let n = self.len()?;
                let raw = self.bytes(n.checked_mul(4).ok_or(Error::BadLength(n as i64))?)?;
                let java = self.flavor == Flavor::Java;
                Tag::IntArray(
                    raw.chunks_exact(4)
                        .map(|c| {
                            let b = [c[0], c[1], c[2], c[3]];
                            if java { i32::from_be_bytes(b) } else { i32::from_le_bytes(b) }
                        })
                        .collect(),
                )
            }
            TagType::LongArray => {
                let n = self.len()?;
                let raw = self.bytes(n.checked_mul(8).ok_or(Error::BadLength(n as i64))?)?;
                let java = self.flavor == Flavor::Java;
                Tag::LongArray(
                    raw.chunks_exact(8)
                        .map(|c| {
                            let b = [c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]];
                            if java { i64::from_be_bytes(b) } else { i64::from_le_bytes(b) }
                        })
                        .collect(),
                )
            }
        })
    }
}
