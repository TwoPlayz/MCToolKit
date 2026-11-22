//! Version 12 chunks, against four written by a real converter (`tests/fixtures/v12`).

use mctk_console::{ChunkCodec, ChunkV12, RawConsoleChunk, is_v12, v12_cell};
use mctk_nbt::Tag;

fn fixture(n: usize) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/v12/chunk{n}.bin"))).unwrap()
}

#[test]
fn real_chunks_re_encode_to_the_same_bytes() {
    for n in 1..=4 {
        let bytes = fixture(n);
        assert!(is_v12(&bytes), "chunk {n}");
        let chunk = ChunkV12::parse(&bytes).unwrap_or_else(|e| panic!("chunk {n}: {e}"));
        assert_eq!(chunk.to_bytes().unwrap(), bytes, "chunk {n} does not re-encode byte for byte");
    }
}

#[test]
fn real_chunk_contents() {
    let c = ChunkV12::parse(&fixture(1)).unwrap();
    assert_eq!((c.x, c.z, c.last_update, c.inhabited_time), (-27, -21, 0, 0));
    assert_eq!(c.terrain_populated, 0x7fe);
    // Bedrock (id 7) at the bottom of this column, stone (id 1) above; water (id 9) near the surface.
    assert_eq!(c.blocks[v12_cell(0, 0, 0)], 0x70);
    assert_eq!(c.blocks[v12_cell(0, 1, 0)] >> 4, 1);
    assert_eq!(c.blocks[v12_cell(0, 62, 0)], 0x90);
    assert_eq!(c.blocks[v12_cell(0, 100, 0)], 0, "air above");
    // 43 liquid cells hold flowing water (id 8).
    assert_eq!(c.liquid.iter().filter(|v| **v == 0x80).count(), 43);
    // The tail is the three lists.
    for key in ["TileTicks", "TileEntities", "Entities"] {
        assert!(c.extra.tag.get(key).is_some(), "{key}");
    }
    // Chunk 3 has 18 scheduled ticks.
    let ticks = ChunkV12::parse(&fixture(3)).unwrap();
    assert_eq!(ticks.extra.tag.get("TileTicks").and_then(Tag::as_list).map(|l| l.len()), Some(18));
}

#[test]
fn real_chunks_become_level_trees_and_come_back() {
    for n in 1..=4 {
        let c = ChunkV12::parse(&fixture(n)).unwrap();
        let tree = c.to_level_tree();
        let level = tree.tag.get("Level").unwrap();
        assert_eq!(level.get("xPos").and_then(Tag::as_i64), Some(i64::from(c.x)));
        let Some(Tag::ByteArray(blocks)) = level.get("Blocks") else { panic!("Blocks") };
        assert_eq!(blocks.len(), 65536);
        // Bedrock in the console index layout (x 0, z 0, y 0).
        assert_eq!(blocks[0], 7);
        // Back to Version 12 and out to a record and in again.
        let again = ChunkV12::from_level_tree(&tree).unwrap();
        // The tree holds one-byte ids: a cell with an extended id (above 255; chunk 1 has 43, chunk 2 one, all
        // of them water-related) keeps only its liquid there (water), while `ChunkV12` keeps the block too.
        let mut extended = 0;
        for i in 0..65536 {
            extended += usize::from(c.blocks[i] >> 4 > 255);
            assert_eq!(again.blocks[i], mctk_console::v12_tree_value(c.blocks[i], c.liquid[i]), "chunk {n} cell {i}");
        }
        assert_eq!(extended, [43, 1, 0, 0][n - 1], "chunk {n}");
        // Every extended-id cell of these chunks has water in its liquid layer, which the tree keeps.
        for i in 0..65536 {
            if c.blocks[i] >> 4 > 255 {
                assert_eq!(again.blocks[i] >> 4, 8, "chunk {n} cell {i}: water survives");
            }
        }
        assert_eq!(again.sky_light, c.sky_light);
        assert_eq!(again.block_light, c.block_light);
    }
}

#[test]
fn a_tail_with_same_named_tags_is_written_back_unchanged() {
    // Real entity attributes hold a `Base` double and a `Base` float side by side; a name-keyed compound keeps one.
    let bytes = fixture(1);
    let c = ChunkV12::parse(&bytes).unwrap();
    let tail_len = mctk_nbt::to_bytes(&c.extra, mctk_nbt::Flavor::Java).unwrap().len();
    let mut patched = bytes[..bytes.len() - tail_len].to_vec();
    patched.extend([0x0a, 0, 0, 6, 0, 4]);
    patched.extend(b"Base");
    patched.extend(16f64.to_be_bytes());
    patched.extend([5, 0, 4]);
    patched.extend(b"Base");
    patched.extend(16f32.to_be_bytes());
    patched.push(0);

    let c2 = ChunkV12::parse(&patched).unwrap();
    assert_eq!(c2.to_bytes().unwrap(), patched, "the stored tail is written back as it was");
    // After an edit the tail is serialised from the tree.
    let mut edited = c2.clone();
    edited.extra.tag.as_compound_mut().unwrap().insert("Extra".into(), Tag::Byte(1));
    let out = edited.to_bytes().unwrap();
    assert_ne!(out, patched);
    assert_eq!(ChunkV12::parse(&out).unwrap().extra, edited.extra);
}

#[test]
fn records_carry_version_12_through_every_codec() {
    let tree = ChunkV12::parse(&fixture(2)).unwrap().to_level_tree();
    for codec in [ChunkCodec::XMemLzx, ChunkCodec::Deflate, ChunkCodec::Zlib] {
        let record = RawConsoleChunk::encode_v12(&tree, codec, 5).unwrap();
        assert!(record.v12_layout, "{codec:?}");
        assert_eq!(record.codec(), codec);
        let v12 = record.v12().unwrap().expect("a Version 12 chunk");
        assert_eq!(v12.x, ChunkV12::parse(&fixture(2)).unwrap().x);
        // `nbt()` gives the same tree as converting directly.
        assert_eq!(record.nbt().unwrap(), v12.to_level_tree());
    }
}
