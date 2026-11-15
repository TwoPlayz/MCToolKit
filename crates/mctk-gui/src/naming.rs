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

pub fn has_mob_name(c: &Compound) -> bool {
    c.contains_key("CustomName")
}

pub fn has_item_name(c: &Compound) -> bool {
    c.get("tag").and_then(|t| t.get("display")).is_some_and(|d| d.get("Name").is_some())
        || c.get("components").is_some_and(|t| t.get("minecraft:custom_name").is_some())
}

/// Gives a mob a name to rename.
pub fn add_mob_name(c: &mut Compound, data_version: Option<i64>) {
    let plain = c.contains_key("identifier");
    c.insert("CustomName".into(), Tag::String(text("Name", data_version, plain)));
}

/// Gives an item a name to rename, where the stack keeps names.
pub fn add_item_name(c: &mut Compound, data_version: Option<i64>) {
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
        assert!(has_item_name(&c));
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
}
