//! Java's modified UTF-8: NUL is encoded as `C0 80` and supplementary characters as
//! CESU-8 surrogate pairs (two three-byte sequences) instead of four-byte sequences.

use crate::Error;

pub fn encode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for unit in s.encode_utf16() {
        match unit {
            0x0001..=0x007f => out.push(unit as u8),
            0x0000 | 0x0080..=0x07ff => {
                out.push(0xc0 | (unit >> 6) as u8);
                out.push(0x80 | (unit & 0x3f) as u8);
            }
            _ => {
                out.push(0xe0 | (unit >> 12) as u8);
                out.push(0x80 | ((unit >> 6) & 0x3f) as u8);
                out.push(0x80 | (unit & 0x3f) as u8);
            }
        }
    }
    out
}

pub fn decode(bytes: &[u8]) -> Result<String, Error> {
    // Fast path: pure ASCII without NUL is identical in every encoding.
    if bytes.iter().all(|&b| b != 0 && b < 0x80) {
        return Ok(String::from_utf8(bytes.to_vec()).expect("ascii"));
    }
    let bad = |i: usize| Error::BadString(format!("invalid modified UTF-8 at byte {i}"));
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b & 0x80 == 0 {
            units.push(b as u16);
            i += 1;
        } else if b & 0xe0 == 0xc0 {
            let b2 = *bytes.get(i + 1).ok_or_else(|| bad(i))?;
            if b2 & 0xc0 != 0x80 {
                return Err(bad(i));
            }
            units.push((((b & 0x1f) as u16) << 6) | (b2 & 0x3f) as u16);
            i += 2;
        } else if b & 0xf0 == 0xe0 {
            let (b2, b3) = match (bytes.get(i + 1), bytes.get(i + 2)) {
                (Some(&x), Some(&y)) if x & 0xc0 == 0x80 && y & 0xc0 == 0x80 => (x, y),
                _ => return Err(bad(i)),
            };
            units.push(
                (((b & 0x0f) as u16) << 12) | (((b2 & 0x3f) as u16) << 6) | (b3 & 0x3f) as u16,
            );
            i += 3;
        } else {
            return Err(bad(i));
        }
    }
    String::from_utf16(&units).map_err(|e| Error::BadString(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_round_trips() {
        assert_eq!(encode("minecraft:stone"), b"minecraft:stone");
        assert_eq!(decode(b"minecraft:stone").unwrap(), "minecraft:stone");
    }

    #[test]
    fn nul_uses_two_bytes() {
        assert_eq!(encode("a\0b"), [b'a', 0xc0, 0x80, b'b']);
        assert_eq!(decode(&[b'a', 0xc0, 0x80, b'b']).unwrap(), "a\0b");
    }

    #[test]
    fn supplementary_characters_use_surrogate_pairs() {
        let s = "\u{1F600}";
        let encoded = encode(s);
        assert_eq!(encoded.len(), 6);
        assert_eq!(decode(&encoded).unwrap(), s);
    }

    #[test]
    fn two_and_three_byte_characters() {
        let s = "é€";
        assert_eq!(encode(s), s.as_bytes());
        assert_eq!(decode(s.as_bytes()).unwrap(), s);
    }

    #[test]
    fn truncated_input_is_rejected() {
        assert!(decode(&[0xe2, 0x82]).is_err());
    }
}
