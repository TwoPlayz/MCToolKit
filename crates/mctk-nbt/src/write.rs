use std::io::Write;

use crate::{Error, Flavor, NamedTag, Tag, TagType, mutf8};

/// Writes one named root tag.
pub fn write_root<W: Write>(writer: &mut W, root: &NamedTag, flavor: Flavor) -> Result<(), Error> {
    let ty = root.tag.tag_type();
    if !matches!(ty, TagType::Compound | TagType::List) {
        return Err(Error::BadRoot(ty));
    }
    let mut w = Writer {
        inner: writer,
        flavor,
    };
    w.inner.write_all(&[ty as u8])?;
    w.string(&root.name)?;
    w.payload(&root.tag)
}

/// Serializes one named root tag to bytes.
pub fn to_bytes(root: &NamedTag, flavor: Flavor) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    write_root(&mut out, root, flavor)?;
    Ok(out)
}

struct Writer<'a, W> {
    inner: &'a mut W,
    flavor: Flavor,
}

macro_rules! num {
    ($name:ident, $ty:ty) => {
        fn $name(&mut self, v: $ty) -> Result<(), Error> {
            let bytes = match self.flavor {
                Flavor::Java => v.to_be_bytes(),
                Flavor::Bedrock => v.to_le_bytes(),
            };
            self.inner.write_all(&bytes)?;
            Ok(())
        }
    };
}

impl<W: Write> Writer<'_, W> {
    num!(i16, i16);
    num!(u16, u16);
    num!(i32, i32);
    num!(i64, i64);
    num!(f32, f32);
    num!(f64, f64);

    fn len(&mut self, n: usize) -> Result<(), Error> {
        let n = i32::try_from(n).map_err(|_| Error::BadLength(n as i64))?;
        self.i32(n)
    }

    fn string(&mut self, s: &str) -> Result<(), Error> {
        let encoded = match self.flavor {
            Flavor::Java => mutf8::encode(s),
            Flavor::Bedrock => s.as_bytes().to_vec(),
        };
        let n = u16::try_from(encoded.len()).map_err(|_| Error::BadLength(encoded.len() as i64))?;
        self.u16(n)?;
        self.inner.write_all(&encoded)?;
        Ok(())
    }

    fn payload(&mut self, tag: &Tag) -> Result<(), Error> {
        match tag {
            Tag::Byte(v) => self.inner.write_all(&[*v as u8])?,
            Tag::Short(v) => self.i16(*v)?,
            Tag::Int(v) => self.i32(*v)?,
            Tag::Long(v) => self.i64(*v)?,
            Tag::Float(v) => self.f32(*v)?,
            Tag::Double(v) => self.f64(*v)?,
            Tag::ByteArray(v) => {
                self.len(v.len())?;
                let bytes: Vec<u8> = v.iter().map(|&b| b as u8).collect();
                self.inner.write_all(&bytes)?;
            }
            Tag::String(s) => self.string(s)?,
            Tag::List(list) => {
                let element = if list.is_empty() {
                    list.element_type()
                } else {
                    list.items[0].tag_type()
                };
                self.inner.write_all(&[element as u8])?;
                self.len(list.len())?;
                for item in list.iter() {
                    self.payload(item)?;
                }
            }
            Tag::Compound(map) => {
                for (name, value) in map {
                    self.inner.write_all(&[value.tag_type() as u8])?;
                    self.string(name)?;
                    self.payload(value)?;
                }
                self.inner.write_all(&[TagType::End as u8])?;
            }
            Tag::IntArray(v) => {
                self.len(v.len())?;
                for &x in v {
                    self.i32(x)?;
                }
            }
            Tag::LongArray(v) => {
                self.len(v.len())?;
                for &x in v {
                    self.i64(x)?;
                }
            }
        }
        Ok(())
    }
}
