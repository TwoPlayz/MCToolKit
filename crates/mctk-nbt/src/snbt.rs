//! Stringified NBT output (Minecraft's text form, as used by `/data get`).

use std::fmt::{self, Display, Write};

use crate::{NamedTag, Tag};

impl Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_tag(f, self, if f.alternate() { Some(0) } else { None })
    }
}

impl Display for NamedTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.name.is_empty() {
            write!(f, "{}: ", quote_key(&self.name))?;
        }
        write_tag(f, &self.tag, if f.alternate() { Some(0) } else { None })
    }
}

fn quote_key(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.+".contains(c));
    if bare { key.to_string() } else { quote(key) }
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn newline(f: &mut fmt::Formatter<'_>, indent: Option<usize>) -> fmt::Result {
    if let Some(n) = indent {
        f.write_char('\n')?;
        for _ in 0..n {
            f.write_str("  ")?;
        }
    }
    Ok(())
}

fn write_array<T: Display>(
    f: &mut fmt::Formatter<'_>,
    prefix: &str,
    suffix: &str,
    items: &[T],
) -> fmt::Result {
    write!(f, "[{prefix};")?;
    for (i, v) in items.iter().enumerate() {
        if i > 0 {
            f.write_char(',')?;
        }
        write!(f, "{v}{suffix}")?;
    }
    f.write_char(']')
}

fn write_tag(f: &mut fmt::Formatter<'_>, tag: &Tag, indent: Option<usize>) -> fmt::Result {
    let inner = indent.map(|n| n + 1);
    match tag {
        Tag::Byte(v) => write!(f, "{v}b"),
        Tag::Short(v) => write!(f, "{v}s"),
        Tag::Int(v) => write!(f, "{v}"),
        Tag::Long(v) => write!(f, "{v}L"),
        Tag::Float(v) => write!(f, "{v:?}f"),
        Tag::Double(v) => write!(f, "{v:?}d"),
        Tag::String(s) => f.write_str(&quote(s)),
        Tag::ByteArray(v) => write_array(f, "B", "b", v),
        Tag::IntArray(v) => write_array(f, "I", "", v),
        Tag::LongArray(v) => write_array(f, "L", "L", v),
        Tag::List(list) => {
            f.write_char('[')?;
            for (i, item) in list.iter().enumerate() {
                if i > 0 {
                    f.write_char(',')?;
                }
                newline(f, inner)?;
                write_tag(f, item, inner)?;
            }
            if !list.is_empty() {
                newline(f, indent)?;
            }
            f.write_char(']')
        }
        Tag::Compound(map) => {
            f.write_char('{')?;
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    f.write_char(',')?;
                }
                newline(f, inner)?;
                write!(f, "{}:", quote_key(k))?;
                if indent.is_some() {
                    f.write_char(' ')?;
                }
                write_tag(f, v, inner)?;
            }
            if !map.is_empty() {
                newline(f, indent)?;
            }
            f.write_char('}')
        }
    }
}
