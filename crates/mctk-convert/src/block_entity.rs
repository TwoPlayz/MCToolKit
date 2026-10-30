//! Block entity (tile entity) conversion between Java and Bedrock.
//!
//! The conversion itself lives in `mctk-chunker` (a port of Chunker's block entity model
//! and handlers); this module adapts it to the converters: it shifts positions by the chunk
//! offset, supplies the block a chest sits in (double chests and trapped chests depend on
//! it) and counts what could not be converted in the report.
//!
//! Converted both ways: containers with items and loot tables (chests, barrels, dispensers,
//! droppers, hoppers, shulker boxes, ender chests, furnaces), signs, mob spawners,
//! comparators, campfires and sculk sensors. Vaults, trial spawners, beehives and the like
//! are written as bare entities. Other kinds are counted by type and left out.

use mctk_chunker::blockentity::edits::{self, BlockEdit};
use mctk_chunker::blockentity::{self, BlockEntity, Kind};
use mctk_java::BlockState;
use mctk_nbt::{Compound, Tag};

use crate::java_bedrock::EditionReport;

#[cfg(test)]
fn int(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(Tag::as_i64)
}

/// Plain text of a text component: a string (plain or JSON) or a compound with `text`.
pub(crate) fn plain_text(tag: &Tag) -> String {
    mctk_chunker::text::to_plain(&mctk_chunker::text::from_nbt(tag))
}

// ---------------------------------------------------------------------------- items

/// Counts what the item codec could not carry over in the conversion report.
pub(crate) fn report_notes(notes: mctk_chunker::Notes, what: &str, report: &mut EditionReport) {
    for (missing, n) in notes.missing {
        report.skip(&format!("{what}: {missing}"), n);
    }
}

fn slot_of(item: &Compound) -> Option<u8> {
    item.get("Slot").and_then(Tag::as_i64).map(|v| v as u8)
}

/// Converts one Java item stack to a Bedrock one (`None` for empty slots and items with no
/// equivalent). The work is done by the item codecs in `mctk-chunker`.
pub(crate) fn item_to_bedrock(item: &Compound, report: &mut EditionReport) -> Option<Compound> {
    let mut notes = mctk_chunker::Notes::default();
    let out = mctk_chunker::item::java::read(item, &mut notes)
        .and_then(|s| mctk_chunker::item::bedrock::write(&s, slot_of(item), &mut notes));
    report_notes(notes, "items", report);
    out
}

/// Converts one Bedrock item stack to a Java one (`None` for empty slots and unknown items).
pub(crate) fn item_to_java(item: &Compound, report: &mut EditionReport) -> Option<Compound> {
    let mut notes = mctk_chunker::Notes::default();
    let out = mctk_chunker::item::bedrock::read(item, &mut notes)
        .map(|s| mctk_chunker::item::java::write(&s, slot_of(item)));
    report_notes(notes, "items", report);
    out
}

// ------------------------------------------------------------------- entry points

fn shift(be: &mut BlockEntity, offset: (i32, i32)) {
    be.x += offset.0 * 16;
    be.z += offset.1 * 16;
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
    if id == "DUMMY" {
        return None;
    }
    let mut notes = mctk_chunker::Notes::default();
    let Some(mut neutral) = blockentity::java::read(be, &mut notes) else {
        report.skip(&format!("block entities: {id}"), 1);
        return None;
    };
    // Java's moving piston and the like have no Bedrock form and are dropped quietly.
    if neutral.kind == Kind::Discard {
        return None;
    }
    shift(&mut neutral, offset);
    if let Some(block) = block {
        edits::prepare_for_bedrock(&mut neutral, block);
    }
    let out = blockentity::bedrock::write(&neutral, &mut notes);
    report_notes(notes, "block entities", report);
    *report
        .converted_block_entities
        .entry(id.to_string())
        .or_insert(0) += 1;
    Some(out)
}

/// Converts an entity made from a block (a bed's colour, a cauldron, a potted plant) to
/// Bedrock. Its position is in source coordinates; `offset` is the chunk offset.
pub(crate) fn generated_to_bedrock(
    mut be: BlockEntity,
    offset: (i32, i32),
    report: &mut EditionReport,
) -> Compound {
    shift(&mut be, offset);
    let mut notes = mctk_chunker::Notes::default();
    let out = blockentity::bedrock::write(&be, &mut notes);
    report_notes(notes, "block entities", report);
    let name = out.get("id").and_then(Tag::as_str).unwrap_or("?").to_string();
    *report
        .converted_block_entities
        .entry(format!("{name} (from block)"))
        .or_insert(0) += 1;
    out
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
    let mut notes = mctk_chunker::Notes::default();
    let Some(mut neutral) = blockentity::bedrock::read(be, &mut notes) else {
        report.skip(&format!("block entities: {id}"), 1);
        return None;
    };
    // Bedrock-only entities (lodestone, moving block, ...) have no Java form.
    if neutral.kind == Kind::Discard {
        return None;
    }
    let edit = block.map(|b| edits::resolve_for_java(&mut neutral, b)).unwrap_or_default();
    shift(&mut neutral, offset);
    let nbt = blockentity::java::write(&neutral, &mut notes);
    report_notes(notes, "block entities", report);
    let java_id = match (&neutral.kind, &nbt) {
        (Kind::Container(c), _) => c.kind.java_id().to_string(),
        (_, Some(n)) => n.get("id").and_then(Tag::as_str).unwrap_or(id).to_string(),
        // Carried by the block: counted under the Bedrock name.
        (_, None) => format!("{id} (applied to block)"),
    };
    *report.converted_block_entities.entry(java_id).or_insert(0) += 1;
    Some(JavaBlockEntity { nbt, edit })
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







