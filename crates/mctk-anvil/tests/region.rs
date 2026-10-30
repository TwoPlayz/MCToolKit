use mctk_anvil::{ChunkCompression, ChunkPos, RawChunk, Region, RegionBuilder, RegionPos};
use mctk_nbt::{Compound, NamedTag, Tag};

fn chunk_nbt(x: i32, z: i32, filler: usize) -> NamedTag {
    let mut c = Compound::new();
    c.insert("xPos".into(), Tag::Int(x));
    c.insert("zPos".into(), Tag::Int(z));
    c.insert("Status".into(), Tag::String("minecraft:full".into()));
    // Incompressible-ish payload to control size.
    let noise: Vec<i8> = (0..filler)
        .map(|i| ((i * 2654435761usize) >> 13) as i8)
        .collect();
    c.insert("noise".into(), Tag::ByteArray(noise));
    NamedTag {
        name: String::new(),
        tag: Tag::Compound(c),
    }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mctk-anvil-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn coordinates_use_floor_division() {
    assert_eq!(ChunkPos::new(0, 0).region(), RegionPos { x: 0, z: 0 });
    assert_eq!(ChunkPos::new(31, 32).region(), RegionPos { x: 0, z: 1 });
    assert_eq!(ChunkPos::new(-1, -32).region(), RegionPos { x: -1, z: -1 });
    assert_eq!(ChunkPos::new(-33, 5).region(), RegionPos { x: -2, z: 0 });
    assert_eq!(ChunkPos::new(-1, -33).local(), (31, 31));
    assert_eq!(
        RegionPos { x: -2, z: 0 }.chunk(31, 5),
        ChunkPos::new(-33, 5)
    );
}

#[test]
fn file_names_round_trip() {
    let pos = RegionPos { x: -3, z: 12 };
    assert_eq!(pos.file_name(), "r.-3.12.mca");
    assert_eq!(RegionPos::from_file_name("r.-3.12.mca"), Some(pos));
    assert_eq!(RegionPos::from_file_name("r.1.2.3.mca"), None);
    assert_eq!(RegionPos::from_file_name("c.1.2.mcc"), None);
}

#[test]
fn writes_and_reads_chunks() {
    let dir = temp_dir("rw");
    let pos = RegionPos { x: -1, z: 2 };
    let mut b = RegionBuilder::new();
    let compressions = [
        ChunkCompression::Zlib,
        ChunkCompression::Gzip,
        ChunkCompression::None,
    ];
    for (i, &(lx, lz)) in [(0, 0), (31, 0), (5, 17)].iter().enumerate() {
        let c = pos.chunk(lx, lz);
        b.set(
            lx,
            lz,
            Some(
                RawChunk::encode(
                    &chunk_nbt(c.x, c.z, 9000),
                    compressions[i],
                    1_700_000_000 + i as u32,
                )
                .unwrap(),
            ),
        );
    }
    let path = b.write(&dir, pos).unwrap();

    let region = Region::open(&path).unwrap();
    assert_eq!(region.position(), Some(pos));
    let present: Vec<_> = region.chunks().collect();
    assert_eq!(present, vec![(0, 0), (31, 0), (5, 17)]);
    for (i, &(lx, lz)) in present.iter().enumerate() {
        let raw = region.raw_chunk(lx, lz).unwrap().unwrap();
        assert_eq!(raw.compression, compressions[i]);
        assert_eq!(raw.timestamp, 1_700_000_000 + i as u32);
        let c = pos.chunk(lx, lz);
        assert_eq!(raw.nbt().unwrap(), chunk_nbt(c.x, c.z, 9000));
    }
    assert!(region.read_chunk(1, 1).unwrap().is_none());
    assert_eq!(std::fs::metadata(&path).unwrap().len() % 4096, 0);
}

#[test]
fn oversized_chunks_go_to_mcc_files() {
    let dir = temp_dir("mcc");
    let pos = RegionPos { x: 0, z: 0 };
    let big = RawChunk::encode(&chunk_nbt(3, 4, 1_200_000), ChunkCompression::None, 7).unwrap();
    assert!(big.data.len() > 255 * 4096);
    let mut b = RegionBuilder::new();
    b.set(3, 4, Some(big.clone()));
    let path = b.write(&dir, pos).unwrap();

    assert!(dir.join("c.3.4.mcc").exists());
    let region = Region::open(&path).unwrap();
    assert_eq!(region.raw_chunk(3, 4).unwrap().unwrap(), big);
}

#[test]
fn rebuilding_an_existing_region_preserves_it() {
    let pos = RegionPos { x: 0, z: 0 };
    let mut b = RegionBuilder::new();
    b.set(
        2,
        9,
        Some(RawChunk::encode(&chunk_nbt(2, 9, 100), ChunkCompression::Zlib, 1).unwrap()),
    );
    let bytes = b.build(pos, None).unwrap();
    let rebuilt = RegionBuilder::from_region(&Region::from_bytes(bytes.clone()).unwrap())
        .unwrap()
        .build(pos, None)
        .unwrap();
    assert_eq!(rebuilt, bytes);
}

#[test]
fn rejects_corrupt_locations() {
    let mut bytes = vec![0u8; 8192];
    // Chunk (0,0) claims sector 50, far past the end of the file.
    bytes[0..4].copy_from_slice(&((50u32 << 8) | 1).to_be_bytes());
    let region = Region::from_bytes(bytes).unwrap();
    assert!(region.raw_chunk(0, 0).is_err());
    assert!(Region::from_bytes(vec![0u8; 100]).is_err());
    assert_eq!(Region::from_bytes(Vec::new()).unwrap().chunks().count(), 0);
}

#[test]
fn lz4_is_reported_as_unsupported() {
    let raw = RawChunk {
        compression: ChunkCompression::Lz4,
        data: vec![1, 2, 3],
        timestamp: 0,
    };
    assert!(raw.decompress().is_err());
}

#[test]
fn default_builder_has_every_slot() {
    let mut b = RegionBuilder::default();
    b.set(31, 31, None);
    assert!(b.get(31, 31).is_none());
}
