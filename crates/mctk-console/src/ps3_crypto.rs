//! Small, dependency-free AES-128, SHA-1 and HMAC-SHA-1 for PS3 save signing.
//!
//! These exist so the crate needs no crypto dependency for one file format. They are tested
//! against the FIPS-197 and RFC 2202 vectors below. They are not constant-time and must not be
//! used for anything security sensitive; PS3 save signing is integrity metadata, not secrecy.

// ---------------------------------------------------------------------------------------------
// AES-128
// ---------------------------------------------------------------------------------------------

const fn make_sboxes() -> ([u8; 256], [u8; 256]) {
    let mut sbox = [0u8; 256];
    let mut inv = [0u8; 256];
    let mut p: u8 = 1;
    let mut q: u8 = 1;
    loop {
        // p *= 3 in GF(2^8)
        p = p ^ (p << 1) ^ if p & 0x80 != 0 { 0x1b } else { 0 };
        // q /= 3 in GF(2^8)
        q ^= q << 1;
        q ^= q << 2;
        q ^= q << 4;
        if q & 0x80 != 0 {
            q ^= 0x09;
        }
        let x = q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4);
        let s = x ^ 0x63;
        sbox[p as usize] = s;
        inv[s as usize] = p;
        if p == 1 {
            break;
        }
    }
    sbox[0] = 0x63;
    inv[0x63] = 0;
    (sbox, inv)
}

const SBOXES: ([u8; 256], [u8; 256]) = make_sboxes();
const SBOX: [u8; 256] = SBOXES.0;
const INV_SBOX: [u8; 256] = SBOXES.1;

#[inline]
fn xtime(x: u8) -> u8 {
    (x << 1) ^ if x & 0x80 != 0 { 0x1b } else { 0 }
}

#[inline]
fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    p
}

/// AES-128 with its expanded key.
#[derive(Clone)]
pub(crate) struct Aes128 {
    round_keys: [[u8; 16]; 11],
}

impl Aes128 {
    pub(crate) fn new(key: &[u8; 16]) -> Self {
        let mut w = [[0u8; 4]; 44];
        for (i, word) in w.iter_mut().enumerate().take(4) {
            word.copy_from_slice(&key[4 * i..4 * i + 4]);
        }
        let mut rcon = 1u8;
        for i in 4..44 {
            let mut t = w[i - 1];
            if i % 4 == 0 {
                t = [SBOX[t[1] as usize] ^ rcon, SBOX[t[2] as usize], SBOX[t[3] as usize], SBOX[t[0] as usize]];
                rcon = xtime(rcon);
            }
            for j in 0..4 {
                w[i][j] = w[i - 4][j] ^ t[j];
            }
        }
        let mut round_keys = [[0u8; 16]; 11];
        for (r, rk) in round_keys.iter_mut().enumerate() {
            for c in 0..4 {
                rk[4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
            }
        }
        Aes128 { round_keys }
    }

    pub(crate) fn encrypt_block(&self, block: &[u8; 16]) -> [u8; 16] {
        let mut s = *block;
        add_round_key(&mut s, &self.round_keys[0]);
        for r in 1..=10 {
            for b in s.iter_mut() {
                *b = SBOX[*b as usize];
            }
            shift_rows(&mut s);
            if r < 10 {
                mix_columns(&mut s);
            }
            add_round_key(&mut s, &self.round_keys[r]);
        }
        s
    }

    pub(crate) fn decrypt_block(&self, block: &[u8; 16]) -> [u8; 16] {
        let mut s = *block;
        add_round_key(&mut s, &self.round_keys[10]);
        for r in (0..10).rev() {
            inv_shift_rows(&mut s);
            for b in s.iter_mut() {
                *b = INV_SBOX[*b as usize];
            }
            add_round_key(&mut s, &self.round_keys[r]);
            if r > 0 {
                inv_mix_columns(&mut s);
            }
        }
        s
    }

    /// CBC without padding; `data.len()` must be a multiple of 16.
    pub(crate) fn cbc_encrypt(&self, iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
        assert!(data.len() % 16 == 0, "CBC input must be a multiple of 16 bytes");
        let mut prev = *iv;
        let mut out = Vec::with_capacity(data.len());
        for chunk in data.chunks_exact(16) {
            let mut b = [0u8; 16];
            for i in 0..16 {
                b[i] = chunk[i] ^ prev[i];
            }
            prev = self.encrypt_block(&b);
            out.extend_from_slice(&prev);
        }
        out
    }

    pub(crate) fn cbc_decrypt(&self, iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
        assert!(data.len() % 16 == 0, "CBC input must be a multiple of 16 bytes");
        let mut prev = *iv;
        let mut out = Vec::with_capacity(data.len());
        for chunk in data.chunks_exact(16) {
            let c: [u8; 16] = chunk.try_into().unwrap();
            let d = self.decrypt_block(&c);
            for i in 0..16 {
                out.push(d[i] ^ prev[i]);
            }
            prev = c;
        }
        out
    }
}

fn add_round_key(s: &mut [u8; 16], rk: &[u8; 16]) {
    for i in 0..16 {
        s[i] ^= rk[i];
    }
}

// State is column-major: s[4 * column + row].
fn shift_rows(s: &mut [u8; 16]) {
    let t = *s;
    for c in 0..4 {
        for r in 0..4 {
            s[4 * c + r] = t[4 * ((c + r) % 4) + r];
        }
    }
}

fn inv_shift_rows(s: &mut [u8; 16]) {
    let t = *s;
    for c in 0..4 {
        for r in 0..4 {
            s[4 * ((c + r) % 4) + r] = t[4 * c + r];
        }
    }
}

fn mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let a = [s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]];
        s[4 * c] = xtime(a[0]) ^ (xtime(a[1]) ^ a[1]) ^ a[2] ^ a[3];
        s[4 * c + 1] = a[0] ^ xtime(a[1]) ^ (xtime(a[2]) ^ a[2]) ^ a[3];
        s[4 * c + 2] = a[0] ^ a[1] ^ xtime(a[2]) ^ (xtime(a[3]) ^ a[3]);
        s[4 * c + 3] = (xtime(a[0]) ^ a[0]) ^ a[1] ^ a[2] ^ xtime(a[3]);
    }
}

fn inv_mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let a = [s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]];
        s[4 * c] = gmul(a[0], 14) ^ gmul(a[1], 11) ^ gmul(a[2], 13) ^ gmul(a[3], 9);
        s[4 * c + 1] = gmul(a[0], 9) ^ gmul(a[1], 14) ^ gmul(a[2], 11) ^ gmul(a[3], 13);
        s[4 * c + 2] = gmul(a[0], 13) ^ gmul(a[1], 9) ^ gmul(a[2], 14) ^ gmul(a[3], 11);
        s[4 * c + 3] = gmul(a[0], 11) ^ gmul(a[1], 13) ^ gmul(a[2], 9) ^ gmul(a[3], 14);
    }
}

// ---------------------------------------------------------------------------------------------
// SHA-1 and HMAC-SHA-1
// ---------------------------------------------------------------------------------------------

pub(crate) fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[4 * i..4 * i + 4].try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// HMAC-SHA-1 with any key length, including empty.
pub(crate) fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..20].copy_from_slice(&sha1(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(64 + data.len());
    inner.extend(k.iter().map(|b| b ^ 0x36));
    inner.extend_from_slice(data);
    let ih = sha1(&inner);
    let mut outer = Vec::with_capacity(84);
    outer.extend(k.iter().map(|b| b ^ 0x5c));
    outer.extend_from_slice(&ih);
    sha1(&outer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn aes128_fips197_vector() {
        let key: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let plain: [u8; 16] = hex("00112233445566778899aabbccddeeff").try_into().unwrap();
        let cipher: [u8; 16] = hex("69c4e0d86a7b0430d8cdb78070b4c55a").try_into().unwrap();
        let aes = Aes128::new(&key);
        assert_eq!(aes.encrypt_block(&plain), cipher);
        assert_eq!(aes.decrypt_block(&cipher), plain);
    }

    #[test]
    fn aes128_cbc_roundtrip() {
        let aes = Aes128::new(&[7u8; 16]);
        let iv = [9u8; 16];
        let data: Vec<u8> = (0..64u8).collect();
        let enc = aes.cbc_encrypt(&iv, &data);
        assert_ne!(enc, data);
        assert_eq!(aes.cbc_decrypt(&iv, &enc), data);
    }

    #[test]
    fn sha1_vectors() {
        assert_eq!(sha1(b"abc").to_vec(), hex("a9993e364706816aba3e25717850c26c9cd0d89d"));
        assert_eq!(sha1(b"").to_vec(), hex("da39a3ee5e6b4b0d3255bfef95601890afd80709"));
        let long = sha1(&vec![b'a'; 1_000_000]);
        assert_eq!(long.to_vec(), hex("34aa973cd4c4daa4f61eeb2bdbad27316534016f"));
    }

    #[test]
    fn hmac_sha1_rfc2202() {
        // Test cases 1, 2 and 6 (long key) from RFC 2202.
        assert_eq!(
            hmac_sha1(&[0x0b; 20], b"Hi There").to_vec(),
            hex("b617318655057264e28bc0b6fb378c8ef146be00")
        );
        assert_eq!(
            hmac_sha1(b"Jefe", b"what do ya want for nothing?").to_vec(),
            hex("effcdf6ae5eb2fa2d27416d5f184df9c259a7c79")
        );
        assert_eq!(
            hmac_sha1(&[0xaa; 80], b"Test Using Larger Than Block-Size Key - Hash Key First").to_vec(),
            hex("aa4ae5e15272d00e95705637ce8a3b55ed402112")
        );
    }
}
