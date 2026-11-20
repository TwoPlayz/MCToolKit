//! Display names of items and mobs, and the map an item shows. An item's name is
//! `tag.display.Name` (Bedrock, console and Java before 1.20.5) or the `minecraft:custom_name`
//! component (1.20.5 on); a mob's is `CustomName`.

use mctk_nbt::{Compound, Tag};

/// A name as the version writes it: a JSON text for 1.13 to 1.21.4, plain text elsewhere.
fn text(name: &str, data_version: Option<i64>, plain: bool) -> String {
    let json = !plain && data_version.is_some_and(|v| (1393..4325).contains(&v));
    if json { format!("{{\"text\":\"{name}\"}}") } else { name.to_string() }
}

fn is_bedrock(c: &Compound) -> bool {
    matches!(c.get("Name"), Some(Tag::String(_))) && !c.contains_key("id")
}

/// A mob: something with a position and an id (Java `id`, Bedrock `identifier`).
pub fn is_mob(c: &Compound) -> bool {
    matches!(c.get("Pos"), Some(Tag::List(_))) && (matches!(c.get("id"), Some(Tag::String(_))) || matches!(c.get("identifier"), Some(Tag::String(_))))
}

/// Gives an item a name (Name) where the stack keeps names, for set_item_name to fill in.
fn add_item_name(c: &mut Compound, data_version: Option<i64>) {
    let numeric = matches!(c.get("id"), Some(Tag::Short(_) | Tag::Int(_)));
    let bedrock = is_bedrock(c);
    let components = c.get("components").is_some_and(|t| t.as_compound().is_some())
        || (!numeric && !bedrock && (c.contains_key("count") || data_version.is_some_and(|v| v >= 3837)));
    if components {
        let comps = c.entry("components".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
        if let Some(comps) = comps.as_compound_mut() {
            comps.insert("minecraft:custom_name".into(), Tag::String(text("Name", data_version, false)));
        }
        return;
    }
    let tag = c.entry("tag".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
    let Some(tag) = tag.as_compound_mut() else { return };
    let display = tag.entry("display".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
    if let Some(display) = display.as_compound_mut() {
        display.insert("Name".into(), Tag::String(text("Name", data_version, numeric || bedrock)));
    }
}

/// A saved name as the words it shows: a plain one as it is, a JSON text (`{"text":"Bob"}`,
/// or `"Bob"`) or a text compound (`{text: "Bob"}`, 1.21.5 on) by its text.
fn shown(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => {
            let s = s.trim();
            if s.starts_with('{') || s.starts_with('"') || s.starts_with('[') {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
                    return Some(json_words(&v));
                }
            }
            Some(s.to_string())
        }
        Tag::Compound(c) => {
            let mut out = c.get("text").and_then(Tag::as_str).unwrap_or_default().to_string();
            for e in c.get("extra").and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()) {
                out.push_str(&shown(e).unwrap_or_default());
            }
            Some(out)
        }
        Tag::List(l) => Some(l.iter().filter_map(shown).collect()),
        _ => None,
    }
}

fn json_words(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a.iter().map(json_words).collect(),
        serde_json::Value::Object(o) => {
            let mut out = o.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_string();
            if let Some(serde_json::Value::Array(extra)) = o.get("extra") {
                out.extend(extra.iter().map(json_words));
            }
            out
        }
        _ => String::new(),
    }
}

/// An item's custom name, if it has one.
pub fn item_name(c: &Compound) -> Option<String> {
    c.get("components")
        .and_then(|t| t.get("minecraft:custom_name"))
        .or_else(|| c.get("tag").and_then(|t| t.get("display")).and_then(|d| d.get("Name")))
        .and_then(shown)
}

/// A mob's (or a chest's) custom name, if it has one.
pub fn mob_name(c: &Compound) -> Option<String> {
    c.get("CustomName").and_then(shown)
}

/// Names an item `name` where its edition and version keep names, as they write it; `None` (or
/// nothing typed) takes its name away.
pub fn set_item_name(c: &mut Compound, name: Option<&str>, data_version: Option<i64>) {
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    // Away from both places first, then into the one this stack uses.
    if let Some(comps) = c.get_mut("components").and_then(Tag::as_compound_mut) {
        comps.shift_remove("minecraft:custom_name");
    }
    if let Some(tag) = c.get_mut("tag").and_then(Tag::as_compound_mut) {
        if let Some(display) = tag.get_mut("display").and_then(Tag::as_compound_mut) {
            display.shift_remove("Name");
            if display.is_empty() {
                tag.shift_remove("display");
            }
        }
        if tag.is_empty() {
            c.shift_remove("tag");
        }
    }
    let Some(name) = name else { return };
    add_item_name(c, data_version);
    let numeric = matches!(c.get("id"), Some(Tag::Short(_) | Tag::Int(_)));
    let value = Tag::String(written(name, data_version, numeric || is_bedrock(c)));
    if let Some(slot) = c.get_mut("components").and_then(Tag::as_compound_mut).and_then(|t| t.get_mut("minecraft:custom_name")) {
        *slot = value;
    } else if let Some(slot) = c.get_mut("tag").and_then(|t| t.as_compound_mut()?.get_mut("display")).and_then(|d| d.as_compound_mut()?.get_mut("Name")) {
        *slot = value;
    }
}

/// A name as the version writes it (see `text`), quotes and backslashes in a JSON text escaped.
fn written(name: &str, data_version: Option<i64>, plain: bool) -> String {
    let json = !plain && data_version.is_some_and(|v| (1393..4325).contains(&v));
    if json { text(&escape(name), data_version, false) } else { name.to_string() }
}

/// Names a mob (or a chest) `name`, as its edition and version write it; `None` (or nothing
/// typed) takes its name away.
pub fn set_mob_name(c: &mut Compound, name: Option<&str>, data_version: Option<i64>) {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => {
            let plain = c.contains_key("identifier") || !matches!(c.get("id"), Some(Tag::String(s)) if s.contains(':'));
            c.insert("CustomName".into(), Tag::String(written(n, data_version, plain)));
        }
        None => {
            c.shift_remove("CustomName");
            c.shift_remove("CustomNameVisible");
        }
    }
}

/// Whether an item stack keeps its data in components (Java 1.20.5 on) rather than in `tag`.
fn uses_components(c: &Compound, data_version: Option<i64>) -> bool {
    let numeric = matches!(c.get("id"), Some(Tag::Short(_) | Tag::Int(_)));
    c.get("components").is_some_and(|t| t.as_compound().is_some())
        || (!numeric && !is_bedrock(c) && (c.contains_key("count") || data_version.is_some_and(|v| v >= 3837)))
}

/// Whether an item never wears out: the `minecraft:unbreakable` component (Java 1.20.5 on), or
/// `tag.Unbreakable` set to 1 (older Java, Bedrock and console).
pub fn is_unbreakable(c: &Compound) -> bool {
    c.get("components").is_some_and(|t| t.get("minecraft:unbreakable").is_some())
        || c.get("tag").and_then(|t| t.get("Unbreakable")).and_then(Tag::as_i64).is_some_and(|v| v != 0)
}

/// Makes an item never wear out, or wear out again, as its edition and version keep it.
pub fn set_unbreakable(c: &mut Compound, on: bool, data_version: Option<i64>) {
    if uses_components(c, data_version) {
        let comps = c.entry("components".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
        if let Some(comps) = comps.as_compound_mut() {
            if on {
                comps.insert("minecraft:unbreakable".into(), Tag::Compound(Compound::new()));
            } else {
                comps.shift_remove("minecraft:unbreakable");
            }
        }
        if c.get("components").and_then(Tag::as_compound).is_some_and(|t| t.is_empty()) {
            c.shift_remove("components");
        }
        return;
    }
    if on {
        let tag = c.entry("tag".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
        if let Some(tag) = tag.as_compound_mut() {
            tag.insert("Unbreakable".into(), Tag::Byte(1));
        }
    } else if let Some(tag) = c.get_mut("tag").and_then(Tag::as_compound_mut) {
        tag.shift_remove("Unbreakable");
        if tag.is_empty() {
            c.shift_remove("tag");
        }
    }
}

/// A name safe inside a JSON text's quotes.
fn escape(name: &str) -> String {
    name.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The map a filled map shows: the number of its `map_N` file (Java and console), or the
/// `map_<uuid>` key of Bedrock. `name` is the item's id (`minecraft:filled_map`).
pub fn map_file(c: &Compound, name: &str) -> Option<String> {
    if !name.ends_with("filled_map") {
        return None;
    }
    // 1.20.5 on, then 1.13 to 1.20.4, then the damage value of older worlds.
    let number = |t: Option<&Tag>| t.and_then(Tag::as_i64);
    if let Some(n) = number(c.get("components").and_then(|t| t.get("minecraft:map_id"))) {
        return Some(format!("map_{n}"));
    }
    let tag = c.get("tag");
    if let Some(n) = number(tag.and_then(|t| t.get("map"))) {
        return Some(format!("map_{n}"));
    }
    if let Some(n) = number(tag.and_then(|t| t.get("map_uuid"))) {
        return Some(format!("map_{n}"));
    }
    number(c.get("Damage")).map(|n| format!("map_{n}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_go_where_the_stack_keeps_them() {
        // Bedrock and old worlds: tag.display.Name, plain.
        let mut c = Compound::new();
        c.insert("Name".into(), Tag::String("minecraft:apple".into()));
        add_item_name(&mut c, None);
        let name = c.get("tag").and_then(|t| t.get("display")).and_then(|d| d.get("Name"));
        assert_eq!(name, Some(&Tag::String("Name".into())));
        assert_eq!(item_name(&c).as_deref(), Some("Name"));
        // 1.20.4: tag.display.Name as JSON; 1.20.5: a component.
        let mut c = Compound::new();
        c.insert("id".into(), Tag::String("minecraft:apple".into()));
        c.insert("Count".into(), Tag::Byte(1));
        add_item_name(&mut c, Some(3700));
        let name = c.get("tag").and_then(|t| t.get("display")).and_then(|d| d.get("Name"));
        assert_eq!(name, Some(&Tag::String("{\"text\":\"Name\"}".into())));
        let mut c = Compound::new();
        c.insert("id".into(), Tag::String("minecraft:apple".into()));
        c.insert("count".into(), Tag::Int(1));
        add_item_name(&mut c, Some(3900));
        assert!(c.get("components").and_then(|t| t.get("minecraft:custom_name")).is_some());
    }

    #[test]
    fn a_filled_map_knows_its_file() {
        let mut c = Compound::new();
        c.insert("Damage".into(), Tag::Short(2));
        assert_eq!(map_file(&c, "minecraft:filled_map"), Some("map_2".into()));
        assert_eq!(map_file(&c, "minecraft:apple"), None);
        let mut comps = Compound::new();
        comps.insert("minecraft:map_id".into(), Tag::Int(7));
        let mut c = Compound::new();
        c.insert("components".into(), Tag::Compound(comps));
        assert_eq!(map_file(&c, "minecraft:filled_map"), Some("map_7".into()));
    }
    /// A name typed in goes where, and how, each edition and version keeps it; it reads back as
    /// typed; and an empty one takes it away.
    #[test]
    fn custom_names_are_written_as_each_edition_writes_them() {
        let s = |v: &str| Tag::String(v.into());
        let item = |pairs: &[(&str, Tag)]| -> Compound { pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect() };
        let name = "Bob's \"Axe\"";
        // Java 26.x: the component, as plain text.
        let mut c = item(&[("id", s("minecraft:iron_axe")), ("count", Tag::Int(1))]);
        set_item_name(&mut c, Some(name), Some(4500));
        assert_eq!(c.get("components").and_then(|t| t.get("minecraft:custom_name")), Some(&s(name)));
        assert_eq!(item_name(&c).as_deref(), Some(name));
        // Java 1.20.4: tag.display.Name, a JSON text.
        let mut c = item(&[("id", s("minecraft:iron_axe")), ("Count", Tag::Byte(1))]);
        set_item_name(&mut c, Some(name), Some(3700));
        assert_eq!(c.get("tag").and_then(|t| t.get("display")).and_then(|d| d.get("Name")), Some(&s(r#"{"text":"Bob's \"Axe\""}"#)));
        assert_eq!(item_name(&c).as_deref(), Some(name));
        // Bedrock and console: tag.display.Name, plain.
        let mut c = item(&[("Name", s("minecraft:iron_axe")), ("Count", Tag::Byte(1)), ("Damage", Tag::Short(0))]);
        set_item_name(&mut c, Some(name), None);
        assert_eq!(c.get("tag").and_then(|t| t.get("display")).and_then(|d| d.get("Name")), Some(&s(name)));
        let mut c = item(&[("id", Tag::Short(258)), ("Count", Tag::Byte(1)), ("Damage", Tag::Short(0))]);
        set_item_name(&mut c, Some(name), None);
        assert_eq!(item_name(&c).as_deref(), Some(name));
        // Taken away: and nothing left behind.
        set_item_name(&mut c, Some("  "), None);
        assert!(item_name(&c).is_none() && !c.contains_key("tag"));
        // Mobs: JSON for Java 1.13 to 1.21.4, plain for 1.21.5 on, Bedrock and console.
        let mut m = item(&[("id", s("minecraft:pig")), ("Pos", Tag::List(mctk_nbt::List::new(mctk_nbt::TagType::Double)))]);
        set_mob_name(&mut m, Some("Bob"), Some(3953));
        assert_eq!(m.get("CustomName"), Some(&s(r#"{"text":"Bob"}"#)));
        assert_eq!(mob_name(&m).as_deref(), Some("Bob"));
        set_mob_name(&mut m, Some("Bob"), Some(4500));
        assert_eq!(m.get("CustomName"), Some(&s("Bob")));
        let mut b = item(&[("identifier", s("minecraft:pig"))]);
        set_mob_name(&mut b, Some("Bob"), None);
        assert_eq!(b.get("CustomName"), Some(&s("Bob")));
        let mut old = item(&[("id", s("Pig"))]);
        set_mob_name(&mut old, Some("Bob"), None);
        assert_eq!(old.get("CustomName"), Some(&s("Bob")));
        set_mob_name(&mut old, None, None);
        assert!(mob_name(&old).is_none());
    }
    /// Unbreakable is the component on Java 1.20.5 and later, `tag.Unbreakable` (1) before and in
    /// Bedrock and console; turned off, nothing is left behind.
    #[test]
    fn unbreakable_goes_where_each_edition_keeps_it() {
        let s = |v: &str| Tag::String(v.into());
        let stack = |pairs: &[(&str, Tag)]| -> Compound { pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect() };
        // Java 26.x: the component.
        let mut c = stack(&[("id", s("minecraft:diamond_pickaxe")), ("count", Tag::Int(1))]);
        set_unbreakable(&mut c, true, Some(4500));
        assert!(c.get("components").and_then(|t| t.get("minecraft:unbreakable")).is_some() && is_unbreakable(&c));
        set_unbreakable(&mut c, false, Some(4500));
        assert!(!is_unbreakable(&c) && !c.contains_key("components"));
        // Java 1.20.4, Bedrock and console: the byte.
        for mut c in [
            stack(&[("id", s("minecraft:diamond_pickaxe")), ("Count", Tag::Byte(1))]),
            stack(&[("Name", s("minecraft:diamond_pickaxe")), ("Count", Tag::Byte(1)), ("Damage", Tag::Short(0))]),
            stack(&[("id", Tag::Short(278)), ("Count", Tag::Byte(1)), ("Damage", Tag::Short(0))]),
        ] {
            set_unbreakable(&mut c, true, Some(3700));
            assert_eq!(c.get("tag").and_then(|t| t.get("Unbreakable")), Some(&Tag::Byte(1)), "{c:?}");
            assert!(is_unbreakable(&c));
            set_unbreakable(&mut c, false, Some(3700));
            assert!(!is_unbreakable(&c) && !c.contains_key("tag"), "{c:?}");
        }
    }
}