//! Rewrites chunk (-15, 11) of a Windows 64 save with a known test pattern (stairs, wool, a
//! chest, sign, furnace, more orientable blocks, a pig and a sheep) so the legacy game and a
//! converter can be compared against known contents.
//!
//! cargo run --release -p mctk-console --example win64_inject -- <saveData.ms>
use mctk_console::{ChunkCodec, ConsoleChunk, ConsoleRegion, ConsoleRegionBuilder, ConsoleSave, RawConsoleChunk};
use mctk_nbt::{Compound, List, Tag, TagType};

fn s(v: &str) -> Tag {
    Tag::String(v.into())
}

fn compound(fields: Vec<(&str, Tag)>) -> Compound {
    let mut c = Compound::new();
    for (k, v) in fields {
        c.insert(k.into(), v);
    }
    c
}

fn list(items: Vec<Compound>) -> Tag {
    Tag::List(List::from_tags(TagType::Compound, items.into_iter().map(Tag::Compound).collect()).unwrap())
}

fn item(slot: i8, id: i16, count: i8, damage: i16) -> Compound {
    compound(vec![
        ("Slot", Tag::Byte(slot)),
        ("id", Tag::Short(id)),
        ("Count", Tag::Byte(count)),
        ("Damage", Tag::Short(damage)),
    ])
}

/// A diamond sword with a custom name, lore and Sharpness III.
fn named_sword() -> Compound {
    let mut sword = item(0, 276, 1, 7);
    let lore = Tag::List(List::from_tags(TagType::String, vec![s("Line one"), s("Line two")]).unwrap());
    let display = compound(vec![("Name", s("Named Sword")), ("Lore", lore)]);
    let ench = list(vec![compound(vec![("id", Tag::Short(16)), ("lvl", Tag::Short(3))])]);
    sword.insert("tag".into(), Tag::Compound(compound(vec![("display", Tag::Compound(display)), ("ench", ench)])));
    sword
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("path to saveData.ms");
    let mut save = ConsoleSave::open(&path)?;
    let (cx, cz): (i32, i32) = (-15, 11);
    let (bx, bz) = (cx * 16, cz * 16);
    let index = save
        .archive
        .entries
        .iter()
        .position(|e| e.name == "r.-1.0.mcr")
        .expect("overworld region");
    let region = ConsoleRegion::parse(save.archive.entries[index].data.clone())?;
    let (lx, lz) = (cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize);
    let raw = region.raw_chunk(lx, lz)?.expect("chunk exists");
    let mut chunk = ConsoleChunk::from_nbt(raw.nbt()?)?;
    assert_eq!((chunk.x, chunk.z), (cx, cz));

    // Ground level is y=75 here; the pattern sits on y=76 (and 77 for the upper halves).
    let y = 76;
    let mut put = |x: usize, y: usize, z: usize, id: u8, data: u8| chunk.set_block(x, y, z, id, data).unwrap();
    for d in 0..4 {
        put(1 + d as usize, y, 2, 53, d); // oak stairs, four facings
        put(1 + d as usize, y, 3, 53, d + 4); // upside down
        put(1 + d as usize, y, 4, 67, d); // cobblestone stairs
    }
    for d in 0..16 {
        put(d as usize, y, 7, 35, d); // wool, every colour
    }
    for d in 0..6 {
        put(d as usize, y, 9, 44, d); // slabs
        put(d as usize, y + 1, 9, 17, 0); // log above
    }
    put(8, y, 9, 17, 4); // log along x
    put(9, y, 9, 17, 8); // log along z
    put(5, y, 11, 54, 2); // chest facing north
    put(7, y, 11, 61, 4); // furnace facing west
    put(9, y, 11, 63, 8); // standing sign, rotation 8
    put(11, y, 11, 68, 3); // wall sign facing south
    // Blocks that need something to hang on, each with its supporting stone around it.
    for x in 0..16 {
        put(x, y, 15, 1, 0); // stone floor
    }
    for (x, yy, z) in [(2, 77, 15), (7, 77, 15), (10, 77, 14), (11, 77, 15), (13, 77, 15), (12, 77, 14)] {
        put(x, yy, z, 1, 0);
    }
    put(0, 77, 15, 50, 5); // torch standing
    put(3, 77, 15, 50, 1); // torch on the west block, so it points east
    put(5, 77, 15, 69, 5); // lever on the floor
    put(8, 77, 15, 69, 1); // lever on the west block
    put(10, 77, 15, 65, 3); // ladder on the block to the north
    put(12, 77, 15, 96, 5); // open trapdoor
    put(14, 77, 15, 55, 5); // redstone wire
    // Redstone: dust, a lever, a repeater, dust again.
    for (x, id, data) in [(0, 55, 0), (1, 55, 3), (2, 55, 0), (3, 69, 5), (4, 55, 7), (5, 93, 1), (6, 55, 0)] {
        put(x, y, 14, id, data);
    }
    put(13, y, 11, 54, 3); // a second chest, for potions
    put(5, y, 13, 64, 0); // door, lower
    put(5, y + 1, 13, 64, 8); // door, upper
    put(7, y, 13, 26, 1); // bed foot
    put(6, y, 13, 26, 9); // bed head, one block west of the foot
    put(9, y, 13, 33, 1); // piston facing up
    put(10, y, 13, 29, 2); // sticky piston facing north
    put(12, y, 13, 23, 3); // dispenser
    put(14, y, 13, 25, 0); // note block

    let mut tree = chunk.to_nbt().clone();
    {
        let level = tree.tag.as_compound_mut().unwrap().get_mut("Level").unwrap().as_compound_mut().unwrap();
        // Heights of the columns that gained blocks.
        if let Some(Tag::ByteArray(h)) = level.get_mut("HeightMap") {
            for v in h.iter_mut() {
                *v = (*v).max(78);
            }
        }
        let te = vec![
            compound(vec![
                ("id", s("Chest")),
                ("x", Tag::Int(bx + 5)),
                ("y", Tag::Int(y as i32)),
                ("z", Tag::Int(bz + 11)),
                (
                    "Items",
                    list(vec![named_sword(), item(1, 264, 7, 0), item(13, 35, 12, 14), item(26, 322, 3, 1)]),
                ),
            ]),
            compound(vec![
                ("id", s("Furnace")),
                ("x", Tag::Int(bx + 7)),
                ("y", Tag::Int(y as i32)),
                ("z", Tag::Int(bz + 11)),
                ("BurnTime", Tag::Short(200)),
                ("CookTime", Tag::Short(50)),
                ("Items", list(vec![item(0, 15, 5, 0), item(1, 263, 8, 0)])),
            ]),
            compound(vec![
                ("id", s("Sign")),
                ("x", Tag::Int(bx + 9)),
                ("y", Tag::Int(y as i32)),
                ("z", Tag::Int(bz + 11)),
                ("Text1", s("Hello")),
                ("Text2", s("Converter")),
                ("Text3", s("")),
                ("Text4", s("")),
            ]),
        ];
        let mut te = te;
        te.push(compound(vec![
            ("id", s("Chest")),
            ("x", Tag::Int(bx + 13)),
            ("y", Tag::Int(y as i32)),
            ("z", Tag::Int(bz + 11)),
            (
                "Items",
                list(
                    [0i16, 16, 8193, 8226, 8258, 8229, 8196, 16384 + 8192 + 9, 16384 + 8193, 8201]
                        .iter()
                        .enumerate()
                        .map(|(i, d)| item(i as i8, 373, 1, *d))
                        .collect(),
                ),
            ),
        ]));
        level.insert("TileEntities".into(), list(te));
        let pos = |x: f64, z: f64| {
            Tag::List(
                List::from_tags(TagType::Double, vec![Tag::Double(bx as f64 + x), Tag::Double(76.0), Tag::Double(bz as f64 + z)])
                    .unwrap(),
            )
        };
        let zero = |n: usize| Tag::List(List::from_tags(TagType::Double, vec![Tag::Double(0.0); n]).unwrap());
        let rot = Tag::List(List::from_tags(TagType::Float, vec![Tag::Float(90.0), Tag::Float(0.0)]).unwrap());
        let mob = |id: &str, p: Tag, health: i16, extra: Vec<(&str, Tag)>| {
            let mut fields = vec![
                ("id", s(id)),
                ("Pos", p),
                ("Motion", zero(3)),
                ("Rotation", rot.clone()),
                ("Health", Tag::Short(health)),
                ("Fire", Tag::Short(-1)),
                ("Air", Tag::Short(300)),
                ("OnGround", Tag::Byte(1)),
                ("Dimension", Tag::Int(0)),
                ("Age", Tag::Int(0)),
                ("CustomName", s("Test")),
            ];
            fields.extend(extra);
            compound(fields)
        };
        let trade = |buy: i16, count: i8, sell: i16| {
            compound(vec![
                ("buy", Tag::Compound(item(0, buy, count, 0))),
                ("sell", Tag::Compound(item(0, sell, 1, 0))),
                ("uses", Tag::Int(1)),
                ("maxUses", Tag::Int(7)),
                ("rewardExp", Tag::Byte(1)),
            ])
        };
        let offers = compound(vec![("Recipes", list(vec![trade(297, 3, 388), trade(388, 1, 264)]))]);
        let entities = vec![
            mob("Villager", pos(9.5, 5.5), 20, vec![("Profession", Tag::Int(1)), ("Offers", Tag::Compound(offers))]),
            mob("Pig", pos(3.5, 5.5), 7, vec![("Saddle", Tag::Byte(1))]),
            mob("Sheep", pos(6.5, 5.5), 6, vec![("Color", Tag::Byte(5)), ("Sheared", Tag::Byte(0))]),
        ];
        level.insert("Entities".into(), list(entities));
    }
    let new = RawConsoleChunk::encode_compressed(&tree, 8, ChunkCodec::Zlib, raw.timestamp)?;
    // Decoding what was written must give the same tree back.
    assert_eq!(new.nbt()?, tree, "encode/decode round trip");

    let mut builder = ConsoleRegionBuilder::default();
    builder.little = true;
    for (x, z) in region.chunks() {
        builder.set(x, z, if (x, z) == (lx, lz) { Some(new.clone()) } else { region.raw_chunk(x, z)? });
    }
    save.archive.entries[index].data = builder.build()?;
    let bytes = save.encode()?;
    std::fs::write(&path, bytes)?;
    println!("wrote {path}");
    Ok(())
}
