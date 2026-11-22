//! Xbox 360 chunk payloads end with a 5-byte zero terminator after the last XMem frame, and the
//! chunk record's length includes it. Measured on every chunk (3,564 + 2,588) of a genuine console
//! save and of a converter's output: all had exactly these 5 zero bytes.

use mctk_console::{ChunkCodec, ConsoleRegion, ConsoleRegionBuilder, RawConsoleChunk};
use mctk_nbt::{Compound, NamedTag, Tag};

fn chunk_tree() -> NamedTag {
    let mut level = Compound::new();
    level.insert("xPos".into(), Tag::Int(3));
    level.insert("zPos".into(), Tag::Int(-4));
    // Run-length friendly, like real terrain; the crate detects XMem chunks by a short first frame.
    level.insert("Blocks".into(), Tag::ByteArray((0..40_000u32).map(|i| (i / 3000) as i8).collect()));
    let mut root = Compound::new();
    root.insert("Level".into(), Tag::Compound(level));
    NamedTag { name: String::new(), tag: Tag::Compound(root) }
}

#[test]
fn xmem_chunk_payloads_end_in_the_terminator_and_still_decode() {
    let tree = chunk_tree();
    let chunk = RawConsoleChunk::encode(&tree, ChunkCodec::XMemLzx, 7).unwrap();
    let p = &chunk.payload;
    assert_eq!(&p[p.len() - 5..], &[0u8; 5]);
    assert_eq!(chunk.codec(), ChunkCodec::XMemLzx);
    assert_eq!(chunk.nbt().unwrap(), tree);
}

#[test]
fn the_record_length_in_a_region_includes_the_terminator() {
    let tree = chunk_tree();
    let chunk = RawConsoleChunk::encode(&tree, ChunkCodec::XMemLzx, 7).unwrap();
    let mut b = ConsoleRegionBuilder::default();
    b.set(1, 2, Some(chunk.clone()));
    let region = ConsoleRegion::parse(b.build().unwrap()).unwrap();
    let back = region.raw_chunk(1, 2).unwrap().unwrap();
    assert_eq!(back.payload, chunk.payload, "the stored payload is exactly what was written, terminator included");
    assert_eq!(back.nbt().unwrap(), tree);
}

#[test]
fn a_chunk_whose_first_frame_is_a_full_frame_is_still_recognised_as_xmem() {
    // After run-length encoding, large or noisy chunks exceed one 32 KiB frame, so the stream starts
    // with a plain 2-byte block size, not 0xFF. 4 of 2,592 chunks of a real converter save were
    // misread as deflate before the frame chain was checked.
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let noise: Vec<i8> = (0..120_000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as i8
        })
        .collect();
    let mut level = Compound::new();
    level.insert("Blocks".into(), Tag::ByteArray(noise));
    let mut root = Compound::new();
    root.insert("Level".into(), Tag::Compound(level));
    let tree = NamedTag { name: String::new(), tag: Tag::Compound(root) };

    let chunk = RawConsoleChunk::encode(&tree, ChunkCodec::XMemLzx, 0).unwrap();
    assert_ne!(chunk.payload[0], 0xff, "this chunk must start with a full frame");
    assert_eq!(chunk.codec(), ChunkCodec::XMemLzx);
    assert_eq!(chunk.nbt().unwrap(), tree);
}

#[test]
fn deflate_and_zlib_payloads_are_never_taken_for_xmem() {
    let tree = chunk_tree();
    for codec in [ChunkCodec::Deflate, ChunkCodec::Zlib] {
        let chunk = RawConsoleChunk::encode(&tree, codec, 0).unwrap();
        assert_eq!(chunk.codec(), codec);
    }
}

#[test]
fn other_codecs_are_unchanged() {
    let tree = chunk_tree();
    for codec in [ChunkCodec::Deflate, ChunkCodec::Zlib] {
        let chunk = RawConsoleChunk::encode(&tree, codec, 0).unwrap();
        assert_eq!(chunk.nbt().unwrap(), tree, "{codec:?}");
    }
}
