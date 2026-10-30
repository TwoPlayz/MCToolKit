//! CRC-32C (Castagnoli) with LevelDB's masking.

const POLY: u32 = 0x82f6_3b78;

const TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

pub fn crc32c(parts: &[&[u8]]) -> u32 {
    let mut crc = !0u32;
    for part in parts {
        for &b in *part {
            crc = TABLE[((crc ^ b as u32) & 0xff) as usize] ^ (crc >> 8);
        }
    }
    !crc
}

const MASK_DELTA: u32 = 0xa282_ead8;

pub fn mask(crc: u32) -> u32 {
    crc.rotate_right(15).wrapping_add(MASK_DELTA)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // Test vectors from RFC 3720 (iSCSI), also used by LevelDB's crc32c tests.
        assert_eq!(crc32c(&[&[0u8; 32]]), 0x8a91_36aa);
        assert_eq!(crc32c(&[&[0xffu8; 32]]), 0x62a8_ab43);
        let ascending: Vec<u8> = (0..32).collect();
        assert_eq!(crc32c(&[&ascending]), 0x46dd_794e);
        assert_eq!(crc32c(&[b"123456789"]), 0xe306_9283);
    }

    #[test]
    fn split_input_matches_whole() {
        assert_eq!(crc32c(&[b"1234", b"56789"]), crc32c(&[b"123456789"]));
    }
}
