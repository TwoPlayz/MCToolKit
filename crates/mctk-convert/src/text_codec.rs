//! Text components (Java) and `§` formatted strings (Bedrock).
//!
//! A Java component is a plain string, a JSON string, a compound (`text`, `color`, `bold`, ...,
//! `extra`) or a list of them. Bedrock keeps one string with `§` codes for colours and styles.

use mctk_nbt::{Compound, List, Tag};
use serde_json::Value;

const COLORS: [(&str, char); 16] = [
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
];

/// (field, code) for the styles with a `§` code.
const STYLES: [(&str, char); 5] = [("obfuscated", 'k'), ("bold", 'l'), ("strikethrough", 'm'), ("underlined", 'n'), ("italic", 'o')];

#[derive(Clone, Default)]
struct Style {
    color: Option<String>,
    flags: [Option<bool>; 5],
}

fn from_value(v: &Value, inherited: &Style, out: &mut String) {
    match v {
        Value::String(s) => emit(s, inherited, out),
        Value::Array(a) => a.iter().for_each(|e| from_value(e, inherited, out)),
        Value::Object(o) => {
            let mut style = inherited.clone();
            if let Some(c) = o.get("color").and_then(Value::as_str) {
                style.color = Some(c.to_string());
            }
            for (i, (name, _)) in STYLES.iter().enumerate() {
                if let Some(b) = o.get(*name).and_then(Value::as_bool) {
                    style.flags[i] = Some(b);
                }
            }
            if let Some(t) = o.get("text").and_then(Value::as_str) {
                emit(t, &style, out);
            }
            if let Some(extra) = o.get("extra").and_then(Value::as_array) {
                extra.iter().for_each(|e| from_value(e, &style, out));
            }
        }
        _ => {}
    }
}

fn emit(text: &str, style: &Style, out: &mut String) {
    if text.is_empty() {
        return;
    }
    if let Some(code) = style.color.as_deref().and_then(|c| COLORS.iter().find(|(n, _)| *n == c)) {
        out.push('\u{a7}');
        out.push(code.1);
    }
    for (i, (_, code)) in STYLES.iter().enumerate() {
        if style.flags[i] == Some(true) {
            out.push('\u{a7}');
            out.push(*code);
        }
    }
    out.push_str(text);
}

/// An NBT text component as JSON data.
fn tag_to_value(tag: &Tag) -> Value {
    match tag {
        Tag::String(s) => {
            let t = s.trim_start();
            if t.starts_with('{') || t.starts_with('[') || t.starts_with('"') {
                serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone()))
            } else {
                Value::String(s.clone())
            }
        }
        Tag::Compound(c) => {
            let mut o = serde_json::Map::new();
            for (k, v) in c {
                let v = match v {
                    Tag::Byte(b) if STYLES.iter().any(|(n, _)| n == k) => Value::Bool(*b != 0),
                    other => tag_to_value(other),
                };
                o.insert(k.clone(), v);
            }
            Value::Object(o)
        }
        Tag::List(l) => Value::Array(l.iter().map(tag_to_value).collect()),
        _ => Value::Null,
    }
}

/// A component as a `§`-formatted string.
pub fn to_legacy(tag: &Tag) -> String {
    let mut out = String::new();
    from_value(&tag_to_value(tag), &Style::default(), &mut out);
    out
}

/// A component as plain text, with no formatting.
pub fn to_plain(tag: &Tag) -> String {
    strip(&to_legacy(tag))
}

fn strip(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{a7}' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// A `§`-formatted string as a component: the string itself when it has no formatting, otherwise
/// a compound with one `extra` entry per run of text.
pub fn from_legacy(text: &str) -> Tag {
    if !text.contains('\u{a7}') {
        return Tag::String(text.to_string());
    }
    let mut runs: Vec<Compound> = Vec::new();
    let (mut color, mut flags, mut current) = (None::<&str>, [false; 5], String::new());
    let flush = |color: Option<&str>, flags: &[bool; 5], current: &mut String, runs: &mut Vec<Compound>| {
        if current.is_empty() {
            return;
        }
        let mut c = Compound::new();
        c.insert("text".into(), Tag::String(std::mem::take(current)));
        if let Some(col) = color {
            c.insert("color".into(), Tag::String(col.to_string()));
        }
        for (i, (name, _)) in STYLES.iter().enumerate() {
            if flags[i] {
                c.insert((*name).into(), Tag::Byte(1));
            }
        }
        runs.push(c);
    };
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\u{a7}' {
            current.push(ch);
            continue;
        }
        let Some(code) = chars.next() else { break };
        flush(color, &flags, &mut current, &mut runs);
        if let Some((name, _)) = COLORS.iter().find(|(_, c)| *c == code) {
            // A colour code resets the styles, as in the game.
            color = Some(name);
            flags = [false; 5];
        } else if let Some(i) = STYLES.iter().position(|(_, c)| *c == code) {
            flags[i] = true;
        } else if code == 'r' {
            color = None;
            flags = [false; 5];
        }
    }
    flush(color, &flags, &mut current, &mut runs);
    let mut root = Compound::new();
    root.insert("text".into(), Tag::String(String::new()));
    let extra: Vec<Tag> = runs.into_iter().map(Tag::Compound).collect();
    if !extra.is_empty() {
        root.insert("extra".into(), Tag::List(List::from_tags(mctk_nbt::TagType::Compound, extra).expect("compounds")));
    }
    Tag::Compound(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_json_strings() {
        assert_eq!(to_legacy(&Tag::String("Hi".into())), "Hi");
        assert_eq!(to_plain(&Tag::String("{\"text\":\"a\",\"extra\":[{\"text\":\"b\",\"color\":\"red\"}]}".into())), "ab");
        assert_eq!(to_legacy(&Tag::String("{\"text\":\"x\",\"color\":\"gold\",\"bold\":true}".into())), "\u{a7}6\u{a7}lx");
    }

    #[test]
    fn formatted_strings_become_components_and_back() {
        let tag = from_legacy("\u{a7}cRed \u{a7}lbold");
        assert_eq!(to_legacy(&tag), "\u{a7}cRed \u{a7}c\u{a7}lbold");
        assert_eq!(from_legacy("none"), Tag::String("none".into()));
    }
}
