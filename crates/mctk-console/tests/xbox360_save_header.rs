//! The header word of an Xbox 360 `savegame.dat` and the terminator behind the XMem frames.
//!
//! Two real files were measured (frame bytes counted by walking the 32 KiB frames):
//! * a converter's output: 9,258,551 frame bytes, word 9,258,564, no padding after the terminator;
//! * a genuine console save: 9,429,715 frame bytes, word 9,429,728, 284 bytes of padding after it.
//!
//! Both satisfy `word = frames + 13` (8 header bytes + frames + 5-byte zero terminator).

use mctk_console::{Xbox360SaveHeader, compress_xbox360, decompress_xbox360};

#[test]
fn header_word_matches_both_measured_files() {
    let converter = Xbox360SaveHeader::for_stream(9_258_551, 13_477_318);
    assert_eq!(converter.compressed_len, 9_258_564);
    assert_eq!(converter.reserved, 0);
    assert_eq!(converter.decompressed_len, 13_477_318);
    assert_eq!(converter.end_offset(), 9_258_568); // the converter's file size

    let console = Xbox360SaveHeader::for_stream(9_429_715, 52_256_226);
    assert_eq!(console.compressed_len, 9_429_728);
    assert_eq!(console.end_offset(), 9_429_732); // the console's file is 9,430,016: padded after this
}

fn sample(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 131 % 251) as u8).collect()
}

#[test]
fn written_file_is_exactly_the_recorded_size_and_ends_in_the_terminator() {
    for len in [1usize, 100, 0x8000, 0x8001, 3 * 0x8000 + 77] {
        let archive = sample(len);
        let file = compress_xbox360(&archive);
        let header = Xbox360SaveHeader::parse(&file).unwrap();
        assert_eq!(header.end_offset(), file.len(), "len {len}: no padding is written");
        assert_eq!(&file[file.len() - 5..], &[0u8; 5], "len {len}: zero terminator");
        assert_eq!(header.decompressed_len as usize, len);
        assert_eq!(decompress_xbox360(&file).unwrap(), archive, "len {len}");
    }
}

#[test]
fn padding_after_the_recorded_size_is_ignored() {
    // A genuine console save is padded after its terminator; so was the 289 bytes seen in the sample.
    let archive = sample(70_000);
    let mut file = compress_xbox360(&archive);
    file.extend((0..284u32).map(|i| if i < 5 { 0 } else { 0xff }));
    assert_eq!(decompress_xbox360(&file).unwrap(), archive);
}

#[test]
fn a_file_shorter_than_its_recorded_size_is_rejected() {
    let file = compress_xbox360(&sample(70_000));
    assert!(decompress_xbox360(&file[..file.len() - 1]).is_err());
    assert!(decompress_xbox360(&file[..8]).is_err());
}
