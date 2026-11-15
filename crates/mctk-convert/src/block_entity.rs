//! Block entity (tile entity) conversion between Java and Bedrock.
//!
//! The conversion itself lives in `block_entities` and `block_entities_more`; this module
//! adapts it to the converters: it shifts positions by the chunk offset, supplies the block an
//! entity sits in (double chests, banner colours and skulls depend on it) and counts what could
//! not be converted in the report. Kinds neither module knows are counted by type and left out.

use crate::block_entities::{self, BlockEdit};
use crate::block_entities_more;
use mctk_java::BlockState;
use mctk_nbt::{Compound, Tag};

use crate::java_bedrock::EditionReport;

#[cfg(test)]
fn int(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(Tag::as_i64)
}

/// Plain text of a text component: a string (plain or JSON) or a compound with `text`.
pub(crate) fn plain_text(tag: &Tag) -> String {
    crate::text_codec::to_plain(tag)
}

// ---------------------------------------------------------------------------- items

fn slot_of(item: &Compound) -> Option<u8> {
    item.get("Slot").and_then(Tag::as_i64).map(|v| v as u8)
}

/// Counts what the item codec could not carry over in the conversion report.
pub(crate) fn report_item_notes(notes: crate::item_codec::Notes, report: &mut EditionReport) {
    for (missing, n) in notes.missing {
        report.skip(&format!("items: {missing}"), n);
    }
}

/// Converts one Java item stack to a Bedrock one (`None` for empty slots and items with no
/// equivalent).
pub(crate) fn item_to_bedrock(item: &Compound, report: &mut EditionReport) -> Option<Compound> {
    let mut notes = crate::item_codec::Notes::default();
    let out = crate::item_codec::java_to_bedrock(item, slot_of(item), &mut notes);
    report_item_notes(notes, report);
    out
}

/// Converts one Bedrock item stack to a Java one (`None` for empty slots and unknown items).
pub(crate) fn item_to_java(item: &Compound, report: &mut EditionReport) -> Option<Compound> {
    let mut notes = crate::item_codec::Notes::default();
    let out = crate::item_codec::bedrock_to_java(item, slot_of(item), &mut notes);
    report_item_notes(notes, report);
    out
}
// ------------------------------------------------------------------- entry points

/// Moves a converted entity by a chunk offset.
fn shift_compound(c: &mut Compound, offset: (i32, i32)) {
    for (key, by) in [("x", offset.0), ("z", offset.1)] {
        if let Some(v) = c.get(key).and_then(Tag::as_i64) {
            c.insert(key.into(), Tag::Int(v as i32 + by * 16));
        }
    }
}

/// Converts a Java block entity to its Bedrock form. `offset` is the chunk offset applied
/// to the output and `block` the block the entity sits in. Returns `None` (after counting
/// it in `report`) for unsupported kinds.
pub(crate) fn java_to_bedrock(
    be: &Compound,
    offset: (i32, i32),
    block: Option<&BlockState>,
    report: &mut EditionReport,
) -> Option<Compound> {
    let id = be.get("id").and_then(Tag::as_str)?;
    // Java's moving piston and the like have no Bedrock form and are dropped quietly.
    if block_entities_more::is_java_only(id) {
        return None;
    }
    let mut notes = crate::item_codec::Notes::default();
    let Some(mut out) = block_entities::java_to_bedrock(be, block, &mut notes) else {
        report.skip(&format!("block entities: {id}"), 1);
        return None;
    };
    report_item_notes(notes, report);
    shift_compound(&mut out, offset);
    *report.converted_block_entities.entry(id.to_string()).or_insert(0) += 1;
    Some(out)
}

/// Converts an entity made from a block (a bed's colour, a cauldron, a potted plant) to
/// Bedrock. Its position is in source coordinates; `offset` is the chunk offset.
pub(crate) fn generated_to_bedrock(
    block: &BlockState,
    at: (i32, i32, i32),
    offset: (i32, i32),
    report: &mut EditionReport,
) -> Option<Compound> {
    let mut out = block_entities_more::from_block(block)?;
    out.insert("x".into(), Tag::Int(at.0));
    out.insert("y".into(), Tag::Int(at.1));
    out.insert("z".into(), Tag::Int(at.2));
    shift_compound(&mut out, offset);
    let name = out.get("id").and_then(Tag::as_str).unwrap_or("?").to_string();
    *report.converted_block_entities.entry(format!("{name} (from block)")).or_insert(0) += 1;
    Some(out)
}

/// What converting a Bedrock block entity gives: the Java entity and the changes to make to
/// the block it sits in (chest halves, skull kind and rotation, lectern and jukebox flags).
pub(crate) struct JavaBlockEntity {
    /// `None` for kinds Java keeps entirely in the block (beds, cauldrons, flower pots).
    pub nbt: Option<Compound>,
    pub edit: BlockEdit,
}

/// Converts a Bedrock block entity to its Java form. `block` is the Java block the entity
/// sits in. Returns `None` (after counting it in `report`) for unsupported kinds.
pub(crate) fn bedrock_to_java(
    be: &Compound,
    offset: (i32, i32),
    block: Option<&BlockState>,
    report: &mut EditionReport,
) -> Option<JavaBlockEntity> {
    let id = be.get("id").and_then(Tag::as_str)?;
    let mut notes = crate::item_codec::Notes::default();
    let Some(mut j) = block_entities::bedrock_to_java(be, block, &mut notes) else {
        // Bedrock-only entities (lodestone, moving block, ...) have no Java form.
        if !block_entities_more::is_bedrock_only(id) {
            report.skip(&format!("block entities: {id}"), 1);
        }
        return None;
    };
    report_item_notes(notes, report);
    if let Some(n) = j.nbt.as_mut() {
        shift_compound(n, offset);
    }
    let java_id = match &j.nbt {
        Some(n) => n.get("id").and_then(Tag::as_str).unwrap_or(id).to_string(),
        // Carried by the block: counted under the Bedrock name.
        None => format!("{id} (applied to block)"),
    };
    *report.converted_block_entities.entry(java_id).or_insert(0) += 1;
    Some(JavaBlockEntity { nbt: j.nbt, edit: j.edit })
}
#[cfg(test)]
mod tests {
    use super::*;
    use mctk_nbt::{List, TagType};

    fn s(v: &str) -> Tag {
        Tag::String(v.into())
    }

    fn java_chest() -> Compound {
        let mut item = Compound::new();
        item.insert("id".into(), s("minecraft:diamond_sword"));
        item.insert("count".into(), Tag::Int(1));
        item.insert("Slot".into(), Tag::Byte(13));
        let mut ench = Compound::new();
        ench.insert("minecraft:sharpness".into(), Tag::Int(3));
        let mut comps = Compound::new();
        comps.insert("minecraft:enchantments".into(), Tag::Compound(ench));
        comps.insert("minecraft:damage".into(), Tag::Int(7));
        comps.insert("minecraft:custom_name".into(), s("{\"text\":\"Slayer\"}"));
        item.insert("components".into(), Tag::Compound(comps));
        let mut be = Compound::new();
        be.insert("id".into(), s("minecraft:chest"));
        be.insert("x".into(), Tag::Int(-5));
        be.insert("y".into(), Tag::Int(64));
        be.insert("z".into(), Tag::Int(20));
        be.insert(
            "Items".into(),
            Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(item)]).unwrap()),
        );
        be
    }

    #[test]
    fn chest_round_trips_with_item_details() {
        let mut report = EditionReport::default();
        let b = java_to_bedrock(&java_chest(), (1, 0), None, &mut report).unwrap();
        assert_eq!(b.get("id").and_then(Tag::as_str), Some("Chest"));
        assert_eq!(int(&b, "x"), Some(11));
        let item = b.get("Items").and_then(Tag::as_list).unwrap().get(0).unwrap();
        assert_eq!(item.get("Name").and_then(Tag::as_str), Some("minecraft:diamond_sword"));
        // Durability lives in `tag.Damage`; the top-level `Damage` is the variant data value.
        assert_eq!(item.get("Damage").and_then(Tag::as_i64), Some(0));
        assert_eq!(item.get("tag").and_then(|t| t.get("Damage")).and_then(Tag::as_i64), Some(7));
        assert_eq!(
            item.get("tag").and_then(|t| t.get("display")).and_then(|d| d.get("Name")).and_then(Tag::as_str),
            Some("Slayer")
        );

        let j = bedrock_to_java(&b, (-1, 0), None, &mut report).unwrap().nbt.unwrap();
        assert_eq!(j.get("id").and_then(Tag::as_str), Some("minecraft:chest"));
        assert_eq!(int(&j, "x"), Some(-5));
        let item = j.get("Items").and_then(Tag::as_list).unwrap().get(0).unwrap();
        assert_eq!(item.get("id").and_then(Tag::as_str), Some("minecraft:diamond_sword"));
        let comps = item.get("components").and_then(Tag::as_compound).unwrap();
        assert_eq!(comps.get("minecraft:damage").and_then(Tag::as_i64), Some(7));
        assert_eq!(
            comps["minecraft:enchantments"].get("minecraft:sharpness").and_then(Tag::as_i64),
            Some(3)
        );
        // Counted once per direction (the report is shared by both conversions here).
        assert_eq!(report.converted_block_entities.get("minecraft:chest"), Some(&2));
    }

    #[test]
    fn sign_text_and_colour_survive() {
        let mut face = Compound::new();
        face.insert(
            "messages".into(),
            Tag::List(
                List::from_tags(TagType::String, vec![s("\"Hello\""), s("{\"text\":\"World\"}"), s(""), s("x")])
                    .unwrap(),
            ),
        );
        face.insert("color".into(), s("red"));
        face.insert("has_glowing_text".into(), Tag::Byte(1));
        let mut be = Compound::new();
        be.insert("id".into(), s("minecraft:sign"));
        for (k, v) in [("x", 1), ("y", 2), ("z", 3)] {
            be.insert(k.into(), Tag::Int(v));
        }
        be.insert("front_text".into(), Tag::Compound(face));
        be.insert("is_waxed".into(), Tag::Byte(1));
        let mut report = EditionReport::default();
        let b = java_to_bedrock(&be, (0, 0), None, &mut report).unwrap();
        let ft = b.get("FrontText").unwrap();
        assert_eq!(ft.get("Text").and_then(Tag::as_str), Some("Hello\nWorld\n\nx\n"));
        let j = bedrock_to_java(&b, (0, 0), None, &mut report).unwrap().nbt.unwrap();
        let ft = j.get("front_text").unwrap();
        assert_eq!(ft.get("color").and_then(Tag::as_str), Some("red"));
        assert_eq!(ft.get("has_glowing_text").and_then(Tag::as_i64), Some(1));
        assert_eq!(j.get("is_waxed").and_then(Tag::as_i64), Some(1));
        let msgs = ft.get("messages").and_then(Tag::as_list).unwrap();
        assert_eq!(msgs.get(1).and_then(Tag::as_str), Some("World"));
    }

    #[test]
    fn unsupported_kinds_are_counted() {
        let mut be = Compound::new();
        be.insert("id".into(), s("minecraft:not_a_real_block_entity"));
        for k in ["x", "y", "z"] {
            be.insert(k.into(), Tag::Int(0));
        }
        let mut report = EditionReport::default();
        assert!(java_to_bedrock(&be, (0, 0), None, &mut report).is_none());
        assert_eq!(report.skipped.get("block entities: minecraft:not_a_real_block_entity"), Some(&1));
    }

    #[test]
    fn data_less_kinds_are_kept_as_bare_entities() {
        let mut be = Compound::new();
        be.insert("id".into(), s("minecraft:vault"));
        for k in ["x", "y", "z"] {
            be.insert(k.into(), Tag::Int(0));
        }
        let mut report = EditionReport::default();
        let b = java_to_bedrock(&be, (0, 0), None, &mut report).unwrap();
        assert_eq!(b.get("id").and_then(Tag::as_str), Some("Vault"));
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn double_chests_get_their_pairing_from_the_block() {
        let block = BlockState::new("minecraft:chest").with("facing", "north").with("type", "left");
        let mut report = EditionReport::default();
        let b = java_to_bedrock(&java_chest(), (0, 0), Some(&block), &mut report).unwrap();
        // A left chest facing north pairs clockwise (east), one block along x.
        assert_eq!((int(&b, "pairx"), int(&b, "pairz")), (Some(-4), Some(20)));
        assert_eq!(int(&b, "pairlead"), Some(0));
        let single = BlockState::new("minecraft:chest").with("facing", "north");
        let back = bedrock_to_java(&b, (0, 0), Some(&single), &mut report).unwrap();
        assert_eq!(back.edit.props, vec![("type", Some("left".to_string()))]);
    }

    #[test]
    fn trapped_chest_is_chosen_from_the_block() {
        let mut be = Compound::new();
        be.insert("id".into(), s("Chest"));
        for k in ["x", "y", "z"] {
            be.insert(k.into(), Tag::Int(0));
        }
        let block = BlockState::new("minecraft:trapped_chest").with("facing", "north");
        let j = bedrock_to_java(&be, (0, 0), Some(&block), &mut EditionReport::default()).unwrap();
        assert_eq!(j.nbt.unwrap().get("id").and_then(Tag::as_str), Some("minecraft:trapped_chest"));
    }
}







