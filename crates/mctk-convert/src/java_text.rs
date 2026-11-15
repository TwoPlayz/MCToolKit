//! Text components between the two Java storage styles: JSON strings (1.20.1 and older, e.g.
//! `'{"text":"Hi","color":"red"}'`) and NBT (1.21.5 and later, e.g. `{text:"Hi",color:"red"}`).
//!
//! Plain text collapses to a bare string in NBT. NBT lists cannot mix types, so a list that
//! holds both strings and compounds wraps each string as `{"": "..."}`, as the game does.

use mctk_nbt::{Compound, List, Tag, TagType};
use serde_json::{Map, Value};

/// Style keys that are booleans in JSON and bytes in NBT.
const BOOL_KEYS: [&str; 6] = ["bold", "italic", "underlined", "strikethrough", "obfuscated", "interpreted"];

/// A list tag from `tags`, wrapping bare strings when the element types are mixed.
pub fn mixed_list(tags: Vec<Tag>) -> Tag {
    let Some(first) = tags.first() else {
        return Tag::List(List::new(TagType::Compound));
    };
    let ty = first.tag_type();
    if tags.iter().all(|t| t.tag_type() == ty) {
        return Tag::List(List::from_tags(ty, tags).expect("same type"));
    }
    let wrapped = tags
        .into_iter()
        .map(|t| match t {
            Tag::String(_) => Tag::Compound(Compound::from([(String::new(), t)])),
            other => other,
        })
        .collect();
    Tag::List(List::from_tags(TagType::Compound, wrapped).expect("compounds"))
}

/// The string inside a bare or `{"": ...}`-wrapped string tag.
pub fn unwrap_str(tag: &Tag) -> Option<&str> {
    match tag {
        Tag::String(s) => Some(s),
        Tag::Compound(c) if c.len() == 1 => c.get("").and_then(Tag::as_str),
        _ => None,
    }
}

fn value_to_tag(v: &Value) -> Option<Tag> {
    Some(match v {
        Value::Null => return None,
        Value::Bool(b) => Tag::Byte(*b as i8),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i32::try_from(i).map_or(Tag::Long(i), Tag::Int),
            None => Tag::Double(n.as_f64()?),
        },
        Value::String(s) => Tag::String(s.clone()),
        Value::Array(a) => mixed_list(a.iter().filter_map(value_to_tag).collect()),
        Value::Object(o) => {
            // Click and hover events changed shape in NBT text; they are left out.
            let mut c = Compound::new();
            for (k, v) in o {
                if k == "clickEvent" || k == "hoverEvent" {
                    continue;
                }
                if let Some(t) = value_to_tag(v) {
                    c.insert(k.clone(), t);
                }
            }
            if c.len() == 1 {
                if let Some(Tag::String(s)) = c.get("text") {
                    return Some(Tag::String(s.clone()));
                }
            }
            Tag::Compound(c)
        }
    })
}

/// A JSON text component string as an NBT text component. Text that is not JSON is taken as
/// literal text.
pub fn json_to_nbt(json: &str) -> Tag {
    match serde_json::from_str::<Value>(json) {
        Ok(v) => value_to_tag(&v).unwrap_or_else(|| Tag::String(String::new())),
        Err(_) => Tag::String(json.to_string()),
    }
}

fn tag_to_value(tag: &Tag, key: Option<&str>) -> Value {
    match tag {
        Tag::String(s) => Value::String(s.clone()),
        Tag::Byte(b) if key.is_some_and(|k| BOOL_KEYS.contains(&k)) => Value::Bool(*b != 0),
        Tag::Byte(b) => Value::from(*b),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(v) => Value::from(*v),
        Tag::Double(v) => Value::from(*v),
        Tag::List(l) => Value::Array(l.iter().map(|t| tag_to_value(t, None)).collect()),
        Tag::Compound(c) => {
            if let Some(s) = unwrap_str(tag) {
                return Value::String(s.to_string());
            }
            let mut m = Map::new();
            for (k, v) in c {
                m.insert(k.clone(), tag_to_value(v, Some(k)));
            }
            Value::Object(m)
        }
        Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_) => Value::Null,
    }
}

/// An NBT text component as the JSON string older versions store.
pub fn nbt_to_json(tag: &Tag) -> String {
    let v = match tag {
        // A bare string is literal text; older versions want it as a text object.
        Tag::String(s) => {
            let mut m = Map::new();
            m.insert("text".into(), Value::String(s.clone()));
            Value::Object(m)
        }
        other => match unwrap_str(other) {
            Some(s) => {
                let mut m = Map::new();
                m.insert("text".into(), Value::String(s.to_string()));
                Value::Object(m)
            }
            None => tag_to_value(other, None),
        },
    };
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_collapses_and_styles_stay_compound() {
        assert_eq!(json_to_nbt(r#"{"text":"Hello"}"#), Tag::String("Hello".into()));
        let styled = json_to_nbt(r#"{"text":"Blade","color":"red","italic":false}"#);
        let c = styled.as_compound().unwrap();
        assert_eq!(c.get("color").and_then(Tag::as_str), Some("red"));
        assert_eq!(c.get("italic"), Some(&Tag::Byte(0)));
        assert_eq!(c.get("text").and_then(Tag::as_str), Some("Blade"));
    }

    #[test]
    fn lists_mixing_strings_and_compounds_wrap_the_strings() {
        let list = mixed_list(vec![
            json_to_nbt(r#"{"text":"first line"}"#),
            json_to_nbt(r#"{"text":"second","color":"blue"}"#),
        ]);
        let l = list.as_list().unwrap();
        assert_eq!(l.element_type(), TagType::Compound);
        assert_eq!(unwrap_str(l.get(0).unwrap()), Some("first line"));
    }

    #[test]
    fn round_trips_through_json() {
        for src in [
            r#"{"text":"Hello"}"#,
            r#"{"color":"red","text":"World"}"#,
            r#"{"text":"a","extra":[{"text":"b","bold":true},"c"]}"#,
        ] {
            let nbt = json_to_nbt(src);
            let back = nbt_to_json(&nbt);
            let a: Value = serde_json::from_str(src).unwrap();
            let b: Value = serde_json::from_str(&back).unwrap();
            assert_eq!(a, b, "{src} -> {back}");
        }
    }

    #[test]
    fn non_json_is_literal_text() {
        assert_eq!(json_to_nbt("plain words"), Tag::String("plain words".into()));
    }
}
