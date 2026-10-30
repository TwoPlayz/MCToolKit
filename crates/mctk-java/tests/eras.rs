use mctk_java::{BlockState, BlockStorage, Chunk, ChunkEra, PaletteStyle, PalettedContainer};
use mctk_nbt::{Compound, Flavor, List, NamedTag, Tag, TagType};

fn compound(entries: Vec<(&str, Tag)>) -> Compound {
    entries
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

fn list(ty: TagType, items: Vec<Tag>) -> Tag {
    Tag::List(List::from_tags(ty, items).unwrap())
}

fn root(c: Compound) -> NamedTag {
    NamedTag {
        name: String::new(),
        tag: Tag::Compound(c),
    }
}

fn round_trip(chunk: &Chunk) -> Chunk {
    let bytes = mctk_nbt::to_bytes(&chunk.to_nbt(), Flavor::Java).unwrap();
    Chunk::from_nbt(mctk_nbt::from_bytes(&bytes, Flavor::Java).unwrap()).unwrap()
}

/// A section's palette container: stone everywhere, a log at (1, 2, 3).
fn sample_container() -> PalettedContainer<BlockState> {
    let mut p = PalettedContainer::filled(BlockState::new("minecraft:stone"), 4096);
    p.set(
        (2 * 16 + 3) * 16 + 1,
        BlockState::new("minecraft:oak_log").with("axis", "y"),
    );
    p
}

#[test]
fn legacy_numeric_chunk() {
    let mut blocks = vec![1i8; 4096]; // stone
    blocks[0] = 17; // log
    let mut data = vec![0i8; 2048];
    data[0] = 0x02; // block 0 data = 2 (birch)
    let mut add = vec![0i8; 2048];
    add[0] = 0x10; // block 1 gets id + 256
    let section = compound(vec![
        ("Y", Tag::Byte(4)),
        ("Blocks", Tag::ByteArray(blocks)),
        ("Add", Tag::ByteArray(add)),
        ("Data", Tag::ByteArray(data)),
        ("SkyLight", Tag::ByteArray(vec![-1; 2048])),
    ]);
    let level = compound(vec![
        ("xPos", Tag::Int(-2)),
        ("zPos", Tag::Int(7)),
        (
            "Sections",
            list(TagType::Compound, vec![Tag::Compound(section)]),
        ),
    ]);
    let chunk = Chunk::from_nbt(root(compound(vec![("Level", Tag::Compound(level))]))).unwrap();

    assert_eq!(chunk.era, ChunkEra::Legacy);
    assert_eq!((chunk.x, chunk.z), (-2, 7));
    let Some(BlockStorage::Legacy { ids, data }) = &chunk.section(4).unwrap().blocks else {
        panic!("legacy storage")
    };
    assert_eq!((ids[0], data[0]), (17, 2));
    assert_eq!(ids[1], 1 + 256);
    assert_eq!(ids[2], 1);
    assert_eq!(round_trip(&chunk), chunk);
}

#[test]
fn flattened_chunk_with_spanning_indices() {
    let container = sample_container();
    let palette: Vec<Tag> = container
        .palette
        .iter()
        .map(BlockState::to_named_compound)
        .collect();
    let section = compound(vec![
        ("Y", Tag::Byte(0)),
        ("Palette", list(TagType::Compound, palette)),
        ("BlockStates", Tag::LongArray(container.encode(4, true))),
    ]);
    let level = compound(vec![
        ("xPos", Tag::Int(0)),
        ("zPos", Tag::Int(0)),
        (
            "Sections",
            list(TagType::Compound, vec![Tag::Compound(section)]),
        ),
    ]);
    // DataVersion 1976 = 1.14.4, before the no-spanning change.
    let chunk = Chunk::from_nbt(root(compound(vec![
        ("DataVersion", Tag::Int(1976)),
        ("Level", Tag::Compound(level)),
    ])))
    .unwrap();

    assert_eq!(chunk.era, ChunkEra::Flattened { spanning: true });
    assert_eq!(
        chunk.block(1, 2, 3).unwrap().to_string(),
        "minecraft:oak_log[axis=y]"
    );
    assert_eq!(chunk.block(0, 0, 0).unwrap().name, "minecraft:stone");
    assert_eq!(round_trip(&chunk), chunk);
}

#[test]
fn modern_name_properties_chunk() {
    let container = sample_container();
    let palette: Vec<Tag> = container
        .palette
        .iter()
        .map(BlockState::to_named_compound)
        .collect();
    let block_states = compound(vec![
        ("palette", list(TagType::Compound, palette)),
        ("data", Tag::LongArray(container.encode(4, false))),
    ]);
    let biomes = compound(vec![(
        "palette",
        list(
            TagType::String,
            vec![Tag::String("minecraft:plains".into())],
        ),
    )]);
    let section = compound(vec![
        ("Y", Tag::Byte(-1)),
        ("block_states", Tag::Compound(block_states)),
        ("biomes", Tag::Compound(biomes)),
    ]);
    let top = compound(vec![
        ("DataVersion", Tag::Int(3953)), // 1.21
        ("xPos", Tag::Int(5)),
        ("zPos", Tag::Int(-5)),
        (
            "sections",
            list(TagType::Compound, vec![Tag::Compound(section)]),
        ),
    ]);
    let chunk = Chunk::from_nbt(root(top)).unwrap();

    assert_eq!(
        (chunk.era, chunk.palette_style),
        (ChunkEra::Modern, PaletteStyle::NameProperties)
    );
    assert_eq!(
        chunk.block(1, -14, 3).unwrap().to_string(),
        "minecraft:oak_log[axis=y]"
    );
    assert_eq!(
        chunk.section(-1).unwrap().biomes.as_ref().unwrap().get(0),
        "minecraft:plains"
    );
    assert_eq!(round_trip(&chunk), chunk);
}

#[test]
fn id_string_palette_wraps_bare_strings_in_mixed_lists() {
    let mut chunk = {
        let block_states = compound(vec![(
            "palette",
            list(TagType::String, vec![Tag::String("minecraft:stone".into())]),
        )]);
        let section = compound(vec![
            ("Y", Tag::Byte(0)),
            ("block_states", Tag::Compound(block_states)),
        ]);
        let top = compound(vec![
            ("DataVersion", Tag::Int(5023)),
            ("xPos", Tag::Int(0)),
            ("zPos", Tag::Int(0)),
            (
                "sections",
                list(TagType::Compound, vec![Tag::Compound(section)]),
            ),
        ]);
        Chunk::from_nbt(root(top)).unwrap()
    };
    assert_eq!(chunk.palette_style, PaletteStyle::IdString);
    let Some(BlockStorage::Palette(p)) = &mut chunk.sections[0].blocks else {
        panic!()
    };
    p.set(0, BlockState::new("minecraft:oak_log").with("axis", "x"));

    let nbt = chunk.to_nbt();
    let palette = nbt
        .tag
        .get("sections")
        .and_then(Tag::as_list)
        .unwrap()
        .get(0)
        .unwrap()
        .get("block_states")
        .unwrap()
        .get("palette")
        .and_then(Tag::as_list)
        .unwrap();
    assert_eq!(palette.element_type(), TagType::Compound);
    assert_eq!(
        palette.get(0).unwrap().get("").and_then(Tag::as_str),
        Some("minecraft:stone")
    );
    assert_eq!(
        palette.get(1).unwrap().get("id").and_then(Tag::as_str),
        Some("minecraft:oak_log")
    );
    assert_eq!(
        round_trip(&chunk).block(0, 0, 0).unwrap().to_string(),
        "minecraft:oak_log[axis=x]"
    );
}

#[test]
fn block_state_equality_ignores_property_order() {
    let a = BlockState::new("minecraft:sculk_vein")
        .with("east", "true")
        .with("up", "false");
    let b = BlockState::new("minecraft:sculk_vein")
        .with("up", "false")
        .with("east", "true");
    assert_eq!(a, b);
    use std::collections::HashSet;
    assert_eq!(HashSet::from([a, b]).len(), 1);
}
