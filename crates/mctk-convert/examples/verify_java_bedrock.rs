//! Converts Java → Bedrock and Bedrock → Java, reopens each output and compares every
//! block with the source (through the mapping table), checks the Data3D height map layout
//! against the game's own, and measures a Bedrock → Java → Bedrock round trip.
//!
//! cargo run --release -p mctk-convert --example verify_java_bedrock -- <java world> <bedrock world> <scratch dir>

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use mctk_anvil::{ChunkPos, Region};
use mctk_bedrock::{BedrockWorld, ChunkTag, Data3D, Key, SubChunk, SubChunkBody};
use mctk_java::{BlockState, BlockStorage, Chunk, JavaWorld};
use mctk_mapping::{BedrockBlock, mappings};
use mctk_nbt::Tag;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// (dim, x, z, section y) -> storages
type Subchunks = HashMap<(i32, i32, i32, i8), Vec<mctk_bedrock::BlockStorage>>;
/// (dim, x, z) -> Data3D
type Heights = HashMap<(i32, i32, i32), Data3D>;

fn load_subchunks(world: &BedrockWorld) -> Res<(Subchunks, Heights)> {
    let (mut subs, mut d3) = (HashMap::new(), HashMap::new());
    for (k, v) in &world.db.entries {
        if let Key::Chunk(c) = Key::decode(k) {
            let col = (c.dimension.id(), c.x, c.z);
            match c.tag {
                ChunkTag::SubChunkPrefix => {
                    if let SubChunkBody::Palettized(s) = SubChunk::parse(v)?.body {
                        subs.insert((col.0, col.1, col.2, c.subchunk.unwrap_or(0)), s);
                    }
                }
                ChunkTag::Data3D => {
                    d3.insert(col, Data3D::parse(v)?);
                }
                _ => {}
            }
        }
    }
    Ok((subs, d3))
}

fn bedrock_at(
    subs: &Subchunks,
    key: (i32, i32, i32, i8),
    x: usize,
    y: usize,
    z: usize,
) -> (BedrockBlock, bool) {
    let air = BedrockBlock {
        name: "minecraft:air".into(),
        states: Default::default(),
    };
    let Some(s) = subs.get(&key) else {
        return (air, false);
    };
    let i = mctk_bedrock::block_index(x, y, z);
    let b = BedrockBlock::from_nbt(&s[0].palette[s[0].indices[i] as usize]).unwrap_or(air);
    let wet = s.get(1).is_some_and(|w| {
        matches!(
            w.palette[w.indices[i] as usize]
                .tag
                .get("name")
                .and_then(Tag::as_str),
            Some("minecraft:water" | "minecraft:flowing_water")
        )
    });
    (b, wet)
}

fn java_to_bedrock(src: &Path, out: &Path) -> Res<()> {
    let report = mctk_convert::java_to_bedrock(src, out)?;
    println!(
        "Java → Bedrock: {:?}; approximated {}, unmapped {:?}, skipped {:?}",
        report.chunks,
        report.approximated.len(),
        report.unmapped,
        report.skipped
    );
    let m = mappings();
    let out_world = BedrockWorld::open(out)?;
    let (subs, _) = load_subchunks(&out_world)?;
    let java = JavaWorld::open(src)?;
    let dims = [
        ("minecraft:overworld", 0, -4..20),
        ("minecraft:the_nether", 1, 0..8),
        ("minecraft:the_end", 2, 0..16),
    ];
    let (mut blocks, mut wrong, mut wet_wrong) = (0u64, 0u64, 0u64);
    for (id, bdim, range) in dims {
        let Some(dim) = java.dimension(id) else {
            continue;
        };
        for rpos in dim.regions()? {
            let region = Region::open(dim.region_dir().join(rpos.file_name()))?;
            for (lx, lz) in region.chunks().collect::<Vec<_>>() {
                let root = region.read_chunk(lx, lz)?.unwrap();
                // The converter leaves out chunks the game had not finished generating.
                let status = root
                    .tag
                    .get("Status")
                    .or_else(|| root.tag.get("Level").and_then(|l| l.get("Status")))
                    .and_then(Tag::as_str)
                    .map(str::to_string);
                if let Some(s) = status {
                    if !matches!(
                        s.trim_start_matches("minecraft:"),
                        "full" | "postprocessed" | "fullchunk"
                    ) {
                        continue;
                    }
                }
                let chunk = Chunk::from_nbt(root)?;
                for s in &chunk.sections {
                    let Some(BlockStorage::Palette(p)) = &s.blocks else {
                        continue;
                    };
                    if !range.contains(&s.y) {
                        continue;
                    }
                    for y in 0..16 {
                        for z in 0..16 {
                            for x in 0..16 {
                                let j = m.complete_java(p.get(mctk_java::block_index(x, y, z)));
                                let expect = m.java_to_bedrock(&j).cloned().unwrap();
                                let (got, wet) =
                                    bedrock_at(&subs, (bdim, chunk.x, chunk.z, s.y), x, y, z);
                                blocks += 1;
                                let all_air = p.palette.iter().all(BlockState::is_air);
                                if !(all_air && got.name == "minecraft:air") && got != expect {
                                    wrong += 1;
                                }
                                let want_wet =
                                    j.properties.get("waterlogged").is_some_and(|v| v == "true")
                                        || [
                                            "minecraft:kelp",
                                            "minecraft:kelp_plant",
                                            "minecraft:seagrass",
                                            "minecraft:tall_seagrass",
                                            "minecraft:bubble_column",
                                        ]
                                        .contains(&j.name.as_str());
                                if want_wet != wet {
                                    wet_wrong += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    println!("  checked {blocks} blocks: {wrong} wrong block, {wet_wrong} wrong water layer");
    if wrong + wet_wrong > 0 {
        return Err("Java → Bedrock mismatch".into());
    }
    Ok(())
}

/// Compares stored Data3D height maps with ones computed from the blocks.
fn heightmap_layout(world: &BedrockWorld) -> Res<()> {
    let (subs, d3) = load_subchunks(world)?;
    let (mut zx, mut xz, mut total) = (0, 0, 0);
    for ((dim, cx, cz), d) in &d3 {
        if *dim != 0 {
            continue;
        }
        for x in 0..16 {
            for z in 0..16 {
                let mut top = 0i16;
                'col: for sy in (-4..20).rev() {
                    for y in (0..16).rev() {
                        if bedrock_at(&subs, (0, *cx, *cz, sy), x, y, z).0.name != "minecraft:air" {
                            top = (sy as i32 * 16 + y as i32 + 1 + 64) as i16;
                            break 'col;
                        }
                    }
                }
                total += 1;
                zx += (d.heightmap[z * 16 + x] == top) as usize;
                xz += (d.heightmap[x * 16 + z] == top) as usize;
            }
        }
    }
    println!(
        "height map layout on the game's own chunks: z*16+x matches {zx}/{total}, x*16+z matches {xz}/{total}"
    );
    Ok(())
}

fn bedrock_to_java(src: &Path, out: &Path) -> Res<()> {
    let report = mctk_convert::bedrock_to_java(src, out)?;
    println!(
        "Bedrock → Java: {:?}; approximated {:?}, unmapped {:?}, skipped {:?}",
        report.chunks,
        report.approximated.keys().take(6).collect::<Vec<_>>(),
        report.unmapped,
        report.skipped
    );
    let m = mappings();
    let world = BedrockWorld::open(src)?;
    heightmap_layout(&world)?;
    let (subs, _) = load_subchunks(&world)?;
    let java = JavaWorld::open(out)?;
    let dim_ids = [
        (0, "minecraft:overworld"),
        (1, "minecraft:the_nether"),
        (2, "minecraft:the_end"),
    ];
    let (mut blocks, mut wrong, mut rewritten) = (0u64, 0u64, 0u64);
    for (dim, cx, cz, sy) in subs.keys() {
        let id = dim_ids.iter().find(|d| d.0 == *dim).unwrap().1;
        let chunk = java
            .chunk(id, ChunkPos::new(*cx, *cz))?
            .ok_or("chunk missing in output")?;
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let (b, wet) = bedrock_at(&subs, (*dim, *cx, *cz, *sy), x, y, z);
                    let mut expect = m
                        .bedrock_to_java(&b.name, &b.states)
                        .cloned()
                        .unwrap_or_else(BlockState::air);
                    if wet {
                        if expect.is_air() {
                            expect = BlockState::new("minecraft:water").with("level", "0");
                        } else if expect.properties.contains_key("waterlogged") {
                            expect
                                .properties
                                .insert("waterlogged".into(), "true".into());
                        }
                    }
                    let got = m.complete_java(
                        chunk
                            .block(x, *sy as i32 * 16 + y as i32, z)
                            .ok_or("no block")?,
                    );
                    blocks += 1;
                    if got != m.complete_java(&expect) {
                        // A Bedrock flower pot becomes the Java block of its potted plant
                        // (the plant lives in the block entity), which the table cannot know.
                        if b.name == "minecraft:flower_pot"
                            && (got.name == "minecraft:flower_pot"
                                || got.name.starts_with("minecraft:potted_"))
                        {
                            rewritten += 1;
                        } else {
                            wrong += 1;
                        }
                    }
                }
            }
        }
    }
    println!(
        "  checked {blocks} blocks: {wrong} wrong ({rewritten} flower pots rewritten from their block entity)"
    );
    if wrong > 0 {
        return Err("Bedrock → Java mismatch".into());
    }
    Ok(())
}

/// Bedrock → Java → Bedrock: how many blocks come back identical.
fn round_trip(original: &Path, back: &Path) -> Res<()> {
    let (a, _) = load_subchunks(&BedrockWorld::open(original)?)?;
    let (b, _) = load_subchunks(&BedrockWorld::open(back)?)?;
    let (mut same, mut total) = (0u64, 0u64);
    let mut diffs: BTreeMap<String, u64> = BTreeMap::new();
    for key in a.keys() {
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let (p, pw) = bedrock_at(&a, *key, x, y, z);
                    let (q, qw) = bedrock_at(&b, *key, x, y, z);
                    total += 1;
                    if p == q && pw == qw {
                        same += 1;
                    } else {
                        *diffs
                            .entry(format!(
                                "{} {:?} -> {} {:?}",
                                p.name, p.states, q.name, q.states
                            ))
                            .or_insert(0) += 1;
                    }
                }
            }
        }
    }
    println!("round trip Bedrock → Java → Bedrock: {same}/{total} blocks identical");
    let mut top: Vec<_> = diffs.into_iter().collect();
    top.sort_by_key(|x| std::cmp::Reverse(x.1));
    for (d, n) in top.iter().take(8) {
        println!("  {n:7} x {d}");
    }
    Ok(())
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let (java, bedrock, scratch) = (
        Path::new(&args[1]),
        Path::new(&args[2]),
        Path::new(&args[3]),
    );
    java_to_bedrock(java, &scratch.join("bedrock-from-java"))?;
    bedrock_to_java(bedrock, &scratch.join("java-from-bedrock"))?;
    mctk_convert::java_to_bedrock(
        &scratch.join("java-from-bedrock"),
        &scratch.join("bedrock-round-trip"),
    )?;
    round_trip(bedrock, &scratch.join("bedrock-round-trip"))
}
