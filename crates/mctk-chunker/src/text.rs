//! Text components (item names, lore, sign lines, book pages). Ported from Chunker's
//! `JsonTextUtil` (MIT, Hive Games).
//!
//! The intermediate form is the JSON text component. Java 26.x stores components as NBT
//! (a string, a compound or a list); Bedrock stores plain strings with `§` formatting codes.

use mctk_nbt::{Compound, List, Tag, TagType};
use serde_json::{Map, Value};

pub type Text = Value;

const SECTION: char = '§';

/// Formatting codes for the legacy `§` format.
const CODES: [(&str, char); 22] = [
    ("black", '0'),
    ("dark_blue", '1'),
    ("dark_green", '2'),
    ("dark_aqua", '3'),
    ("dark_red", '4'),
    ("dark_purple", '5'),
    ("gold", '6'),
    ("gray", '7'),
    ("dark_gray", '8'),
    ("blue", '9'),
    ("green", 'a'),
    ("aqua", 'b'),
    ("red", 'c'),
    ("light_purple", 'd'),
    ("yellow", 'e'),
    ("white", 'f'),
    ("obfuscated", 'k'),
    ("bold", 'l'),
    ("strikethrough", 'm'),
    ("underline", 'n'),
    ("italic", 'o'),
    ("reset", 'r'),
];

/// Style flags stored as bytes in NBT and as booleans in JSON.
const FLAGS: [&str; 6] = [
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
    "interpret",
];

/// Children that are themselves text components.
const NESTED: [&str; 4] = ["extra", "separator", "fallback", "with"];

/// A text component holding just `text`.
pub fn plain(text: &str) -> Text {
    let mut m = Map::new();
    m.insert("text".into(), Value::String(text.into()));
    Value::Object(m)
}

/// Parses a JSON component string; anything that is not JSON is taken as plain text.
pub fn from_json(s: &str) -> Text {
    let t = s.trim_start();
    if t.starts_with(['{', '[', '"']) {
        match serde_json::from_str::<Value>(t) {
            Ok(Value::String(inner)) => return plain(&inner),
            Ok(Value::Null) | Err(_) => {}
            Ok(v) => return v,
        }
    }
    plain(s)
}

fn nbt_to_json(tag: &Tag) -> Value {
    match tag {
        Tag::String(s) => Value::String(s.clone()),
        Tag::Byte(v) => Value::from(*v),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(v) => Value::from(*v as f64),
        Tag::Double(v) => Value::from(*v),
        Tag::List(l) => Value::Array(l.iter().map(nbt_to_json).collect()),
        Tag::Compound(c) => Value::Object(
            c.iter()
                .map(|(k, v)| (k.clone(), nbt_to_json(v)))
                .collect(),
        ),
        Tag::IntArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
        Tag::ByteArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
        Tag::LongArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
    }
}

/// Reads a Java NBT text component.
pub fn from_nbt(tag: &Tag) -> Text {
    match tag {
        Tag::String(s) => from_json(s),
        Tag::List(l) => Value::Array(l.iter().map(from_nbt).collect()),
        Tag::Compound(c) => {
            // Lists of mixed types wrap each element as {"": value}.
            if let Some(inner) = c.get("") {
                return from_nbt(inner);
            }
            let mut m = Map::new();
            for (k, v) in c {
                let value = if FLAGS.contains(&k.as_str()) {
                    Value::Bool(v.as_i64().unwrap_or(0) != 0)
                } else if NESTED.contains(&k.as_str()) {
                    from_nbt(v)
                } else {
                    nbt_to_json(v)
                };
                m.insert(k.clone(), value);
            }
            Value::Object(m)
        }
        other => plain(&nbt_to_json(other).to_string()),
    }
}

fn json_to_nbt(v: &Value) -> Tag {
    match v {
        Value::String(s) => Tag::String(s.clone()),
        Value::Bool(b) => Tag::Byte(*b as i8),
        Value::Number(n) => match n.as_i64() {
            Some(i) if i32::try_from(i).is_ok() => Tag::Int(i as i32),
            Some(i) => Tag::Long(i),
            None => Tag::Double(n.as_f64().unwrap_or(0.0)),
        },
        Value::Array(a) => list_of(a.iter().map(json_to_nbt).collect()),
        Value::Object(o) => Tag::Compound(
            o.iter()
                .map(|(k, v)| (k.clone(), json_to_nbt(v)))
                .collect(),
        ),
        Value::Null => Tag::String(String::new()),
    }
}

/// A list of tags; mixed element types are wrapped as {"": value}, as Java writes them.
fn list_of(tags: Vec<Tag>) -> Tag {
    let Some(first) = tags.first().map(Tag::tag_type) else {
        return Tag::List(List::new(TagType::End));
    };
    if tags.iter().all(|t| t.tag_type() == first) {
        return Tag::List(List::from_tags(first, tags).expect("same type"));
    }
    let wrapped = tags
        .into_iter()
        .map(|t| match t {
            Tag::Compound(c) => Tag::Compound(c),
            other => {
                let mut c = Compound::new();
                c.insert(String::new(), other);
                Tag::Compound(c)
            }
        })
        .collect();
    Tag::List(List::from_tags(TagType::Compound, wrapped).expect("compounds"))
}

/// Writes a Java NBT text component; a component holding only text becomes a string.
pub fn to_nbt(text: &Text) -> Tag {
    match text {
        Value::Object(o) if o.len() == 1 && o.get("text").is_some_and(Value::is_string) => {
            Tag::String(o["text"].as_str().unwrap_or_default().to_string())
        }
        Value::Object(o) => {
            let mut c = Compound::new();
            for (k, v) in o {
                let tag = if FLAGS.contains(&k.as_str()) {
                    Tag::Byte(v.as_bool().unwrap_or(false) as i8)
                } else if NESTED.contains(&k.as_str()) {
                    to_nbt(v)
                } else {
                    json_to_nbt(v)
                };
                c.insert(k.clone(), tag);
            }
            Tag::Compound(c)
        }
        Value::Array(a) => list_of(a.iter().map(to_nbt).collect()),
        other => json_to_nbt(other),
    }
}

/// Renders a component in the legacy `§` format. Bedrock has no underline or strikethrough.
pub fn to_legacy(text: &Text, bedrock: bool) -> String {
    match text {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => a.iter().map(|t| to_legacy(t, bedrock)).collect(),
        Value::Object(o) => {
            let mut out = String::new();
            let code = |name: &str| CODES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c);
            if let Some(c) = o.get("color").and_then(Value::as_str).and_then(code) {
                out.push(SECTION);
                out.push(c);
            }
            let flag = |k: &str| o.get(k).and_then(Value::as_bool).unwrap_or(false);
            let mut push = |on: bool, name: &str| {
                if on {
                    out.push(SECTION);
                    out.push(code(name).expect("known code"));
                }
            };
            push(flag("bold"), "bold");
            push(flag("italic"), "italic");
            push(flag("underlined") && !bedrock, "underline");
            push(flag("strikethrough") && !bedrock, "strikethrough");
            push(flag("obfuscated"), "obfuscated");
            for key in ["text", "translate", "extra"] {
                if let Some(v) = o.get(key) {
                    out.push_str(&to_legacy(v, bedrock));
                }
            }
            out
        }
    }
}

/// The text without any formatting.
pub fn to_plain(text: &Text) -> String {
    let legacy = to_legacy(text, false);
    let mut out = String::with_capacity(legacy.len());
    let mut chars = legacy.chars();
    while let Some(c) = chars.next() {
        if c == SECTION {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nbt_round_trip_and_legacy() {
        let mut c = Compound::new();
        c.insert("text".into(), Tag::String("Hi".into()));
        c.insert("color".into(), Tag::String("red".into()));
        c.insert("bold".into(), Tag::Byte(1));
        let t = from_nbt(&Tag::Compound(c.clone()));
        assert_eq!(t["bold"], Value::Bool(true));
        assert_eq!(to_nbt(&t), Tag::Compound(c));
        assert_eq!(to_legacy(&t, true), "§c§lHi");
        assert_eq!(to_plain(&t), "Hi");
    }

    #[test]
    fn strings_json_and_plain() {
        assert_eq!(from_nbt(&Tag::String("\"x\"".into())), plain("x"));
        assert_eq!(from_nbt(&Tag::String("plain".into())), plain("plain"));
        assert_eq!(to_nbt(&plain("x")), Tag::String("x".into()));
        let j = from_json("{\"text\":\"a\",\"extra\":[\"b\"]}");
        assert_eq!(to_plain(&j), "ab");
    }
}
