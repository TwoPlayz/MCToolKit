use mctk_nbt::{
    Compound, Compression, Error, Flavor, List, NamedTag, Tag, TagType, decompress, from_bytes,
    to_bytes,
};

/// `hello_world.nbt` from the original NBT specification: a compound named "hello world"
/// holding the string tag `name = "Bananrama"`.
const HELLO_WORLD: &[u8] = &[
    0x0a, 0x00, 0x0b, b'h', b'e', b'l', b'l', b'o', b' ', b'w', b'o', b'r', b'l', b'd', 0x08, 0x00,
    0x04, b'n', b'a', b'm', b'e', 0x00, 0x09, b'B', b'a', b'n', b'a', b'n', b'r', b'a', b'm', b'a',
    0x00,
];

fn sample() -> NamedTag {
    let mut inner = Compound::new();
    inner.insert("x".into(), Tag::Int(-3));
    let mut list = List::new(TagType::Compound);
    list.push(Tag::Compound(inner)).unwrap();

    let mut root = Compound::new();
    root.insert("byte".into(), Tag::Byte(-1));
    root.insert("short".into(), Tag::Short(300));
    root.insert("int".into(), Tag::Int(i32::MIN));
    root.insert("long".into(), Tag::Long(i64::MAX));
    root.insert("float".into(), Tag::Float(1.5));
    root.insert("double".into(), Tag::Double(-0.25));
    root.insert("bytes".into(), Tag::ByteArray(vec![1, -2, 3]));
    root.insert("text".into(), Tag::String("é \u{1F600} \0".into()));
    root.insert("list".into(), Tag::List(list));
    root.insert("empty".into(), Tag::List(List::new(TagType::String)));
    root.insert("ints".into(), Tag::IntArray(vec![1, 2, 3]));
    root.insert("longs".into(), Tag::LongArray(vec![-1, 0, 1]));
    NamedTag {
        name: "root".into(),
        tag: Tag::Compound(root),
    }
}

#[test]
fn reads_hello_world() {
    let root = from_bytes(HELLO_WORLD, Flavor::Java).unwrap();
    assert_eq!(root.name, "hello world");
    assert_eq!(
        root.tag.get("name").and_then(Tag::as_str),
        Some("Bananrama")
    );
}

#[test]
fn writes_hello_world_byte_for_byte() {
    let root = from_bytes(HELLO_WORLD, Flavor::Java).unwrap();
    assert_eq!(to_bytes(&root, Flavor::Java).unwrap(), HELLO_WORLD);
}

#[test]
fn round_trips_every_type_in_both_flavors() {
    for flavor in [Flavor::Java, Flavor::Bedrock] {
        let original = sample();
        let bytes = to_bytes(&original, flavor).unwrap();
        let back = from_bytes(&bytes, flavor).unwrap();
        assert_eq!(back, original, "{flavor:?}");
        assert_eq!(
            to_bytes(&back, flavor).unwrap(),
            bytes,
            "{flavor:?} re-encode"
        );
    }
}

#[test]
fn flavors_differ_in_byte_order() {
    let root = sample();
    assert_ne!(
        to_bytes(&root, Flavor::Java).unwrap(),
        to_bytes(&root, Flavor::Bedrock).unwrap()
    );
}

#[test]
fn preserves_compound_key_order() {
    let back = from_bytes(&to_bytes(&sample(), Flavor::Java).unwrap(), Flavor::Java).unwrap();
    let keys: Vec<&str> = back
        .tag
        .as_compound()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "byte", "short", "int", "long", "float", "double", "bytes", "text", "list", "empty",
            "ints", "longs"
        ]
    );
}

#[test]
fn keeps_element_type_of_empty_lists() {
    let back = from_bytes(&to_bytes(&sample(), Flavor::Java).unwrap(), Flavor::Java).unwrap();
    assert_eq!(
        back.tag
            .get("empty")
            .and_then(Tag::as_list)
            .unwrap()
            .element_type(),
        TagType::String
    );
}

#[test]
fn rejects_mixed_lists() {
    let mut list = List::new(TagType::Int);
    assert!(matches!(
        list.push(Tag::Byte(1)),
        Err(Error::ListTypeMismatch { .. })
    ));
}

#[test]
fn rejects_truncated_input() {
    assert!(from_bytes(&HELLO_WORLD[..HELLO_WORLD.len() - 3], Flavor::Java).is_err());
}

#[test]
fn rejects_negative_array_length() {
    // compound "" { bytes: ByteArray(len = -1) }
    let data = [0x0a, 0, 0, 0x07, 0, 1, b'b', 0xff, 0xff, 0xff, 0xff, 0];
    assert!(matches!(
        from_bytes(&data, Flavor::Java),
        Err(Error::BadLength(-1))
    ));
}

#[test]
fn rejects_excessive_nesting() {
    // 600 nested lists of lists.
    let mut data = vec![0x09, 0, 0];
    for _ in 0..600 {
        data.extend_from_slice(&[0x09, 0, 0, 0, 1]);
    }
    data.extend_from_slice(&[0x00, 0, 0, 0, 0]);
    assert!(matches!(
        from_bytes(&data, Flavor::Java),
        Err(Error::TooDeep(_))
    ));
}

#[test]
fn detects_and_decompresses_gzip_and_zlib() {
    use std::io::Write;
    let raw = to_bytes(&sample(), Flavor::Java).unwrap();

    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw).unwrap();
    let gz = gz.finish().unwrap();
    assert_eq!(decompress(&gz).unwrap(), (Compression::Gzip, raw.clone()));

    let mut zl = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    zl.write_all(&raw).unwrap();
    let zl = zl.finish().unwrap();
    assert_eq!(decompress(&zl).unwrap(), (Compression::Zlib, raw.clone()));

    assert_eq!(decompress(&raw).unwrap(), (Compression::None, raw));
}

#[test]
fn snbt_output() {
    let root = from_bytes(HELLO_WORLD, Flavor::Java).unwrap();
    assert_eq!(root.to_string(), "\"hello world\": {name:\"Bananrama\"}");
    let mut c = Compound::new();
    c.insert("arr".into(), Tag::LongArray(vec![1, -2]));
    c.insert("f".into(), Tag::Float(2.0));
    assert_eq!(Tag::Compound(c).to_string(), "{arr:[L;1L,-2L],f:2.0f}");
}
