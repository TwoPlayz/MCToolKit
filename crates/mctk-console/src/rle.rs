//! The run-length encoding applied to console chunk data before compression.
//!
//! `0xFF` is the escape byte: `FF n` with `n < 3` stands for `n + 1` literal `0xFF` bytes;
//! `FF n v` stands for `n + 1` copies of `v`. Every other byte is a literal.

use crate::{Error, bad};

pub fn rle_decode(input: &[u8], expected_len: Option<usize>) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(expected_len.unwrap_or(input.len() * 4));
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        i += 1;
        if b != 0xff {
            out.push(b);
            continue;
        }
        let n = *input.get(i).ok_or_else(|| bad("truncated RLE escape"))? as usize;
        i += 1;
        if n < 3 {
            out.extend(std::iter::repeat_n(0xff, n + 1));
        } else {
            let v = *input.get(i).ok_or_else(|| bad("truncated RLE run"))?;
            i += 1;
            out.extend(std::iter::repeat_n(v, n + 1));
        }
    }
    if let Some(len) = expected_len {
        if out.len() != len {
            return Err(bad(format!(
                "RLE produced {} bytes, expected {len}",
                out.len()
            )));
        }
    }
    Ok(out)
}

pub fn rle_encode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 2);
    let mut i = 0;
    while i < input.len() {
        let v = input[i];
        let mut run = 1;
        while i + run < input.len() && input[i + run] == v && run < 256 {
            run += 1;
        }
        if v == 0xff && run <= 3 {
            out.extend_from_slice(&[0xff, (run - 1) as u8]);
        } else if run >= 4 || v == 0xff {
            out.extend_from_slice(&[0xff, (run - 1) as u8, v]);
        } else {
            out.extend(std::iter::repeat_n(v, run));
        }
        i += run;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_escapes() {
        assert_eq!(
            rle_decode(&[1, 0xff, 0, 2], None).unwrap(),
            vec![1, 0xff, 2]
        );
        assert_eq!(rle_decode(&[0xff, 2], None).unwrap(), vec![0xff; 3]);
        assert_eq!(rle_decode(&[0xff, 9, 7], None).unwrap(), vec![7; 10]);
        assert!(rle_decode(&[0xff], None).is_err());
    }

    #[test]
    fn encode_decode_round_trip() {
        let mut data = vec![0u8; 5000];
        data.extend([
            0xff, 0xff, 1, 2, 3, 0xff, 0xff, 0xff, 0xff, 0xff, 4, 4, 4, 4,
        ]);
        data.extend((0..=255u8).cycle().take(3000));
        assert_eq!(
            rle_decode(&rle_encode(&data), Some(data.len())).unwrap(),
            data
        );
    }
}
