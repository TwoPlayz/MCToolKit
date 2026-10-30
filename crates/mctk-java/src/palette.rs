//! Paletted containers: a palette of distinct values plus packed indices into it.
//!
//! Since 1.16 indices never straddle two longs (`64 / bits` entries per long, the top
//! bits unused). Before 1.16 the indices form one continuous bit stream that may cross
//! long boundaries ("spanning" packing).

use crate::Error;

/// Bits per index for a palette of `len` entries, never below `min_bits`.
/// A single-entry palette needs no index data at all (returns 0).
pub fn bits_for(len: usize, min_bits: u32) -> u32 {
    if len <= 1 {
        return 0;
    }
    let needed = usize::BITS - (len - 1).leading_zeros();
    needed.max(min_bits)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PalettedContainer<T> {
    pub palette: Vec<T>,
    /// One palette index per position (4096 blocks or 64 biome cells per section).
    pub indices: Vec<u16>,
}

impl<T: Clone + PartialEq> PalettedContainer<T> {
    /// A container where every position holds `value`.
    pub fn filled(value: T, size: usize) -> Self {
        PalettedContainer {
            palette: vec![value],
            indices: vec![0; size],
        }
    }

    /// Unpacks `data` for a palette of `palette.len()` entries.
    pub fn decode(
        palette: Vec<T>,
        data: &[i64],
        size: usize,
        min_bits: u32,
        spanning: bool,
    ) -> Result<Self, Error> {
        let bits = bits_for(palette.len(), min_bits);
        if palette.is_empty() {
            return Err(Error::BadChunk("empty palette".into()));
        }
        if bits == 0 || data.is_empty() {
            if palette.len() > 1 && data.is_empty() {
                return Err(Error::BadChunk("multi-entry palette without data".into()));
            }
            return Ok(PalettedContainer {
                palette,
                indices: vec![0; size],
            });
        }
        let expected = packed_len(size, bits, spanning);
        // Some writers pick a wider width than the minimum; accept any width that fits.
        let bits = if data.len() == expected {
            bits
        } else {
            infer_bits(data.len(), size, spanning).ok_or_else(|| {
                Error::BadChunk(format!("{} longs cannot hold {size} indices", data.len()))
            })?
        };
        let mask = (1u64 << bits) - 1;
        let mut indices = Vec::with_capacity(size);
        for i in 0..size {
            let v = if spanning {
                let bit = i * bits as usize;
                let (word, shift) = (bit / 64, bit % 64);
                let mut v = (data[word] as u64) >> shift;
                if shift + bits as usize > 64 {
                    v |= (data[word + 1] as u64) << (64 - shift);
                }
                v & mask
            } else {
                let per_long = 64 / bits as usize;
                ((data[i / per_long] as u64) >> ((i % per_long) * bits as usize)) & mask
            };
            if v as usize >= palette.len() {
                return Err(Error::BadChunk(format!(
                    "index {v} outside palette of {}",
                    palette.len()
                )));
            }
            indices.push(v as u16);
        }
        Ok(PalettedContainer { palette, indices })
    }

    /// Packs the indices with the minimal width for the palette.
    pub fn encode(&self, min_bits: u32, spanning: bool) -> Vec<i64> {
        let bits = bits_for(self.palette.len(), min_bits);
        if bits == 0 {
            return Vec::new();
        }
        let mut data = vec![0u64; packed_len(self.indices.len(), bits, spanning)];
        for (i, &v) in self.indices.iter().enumerate() {
            let v = v as u64;
            if spanning {
                let bit = i * bits as usize;
                let (word, shift) = (bit / 64, bit % 64);
                data[word] |= v << shift;
                if shift + bits as usize > 64 {
                    data[word + 1] |= v >> (64 - shift);
                }
            } else {
                let per_long = 64 / bits as usize;
                data[i / per_long] |= v << ((i % per_long) * bits as usize);
            }
        }
        data.into_iter().map(|w| w as i64).collect()
    }

    pub fn get(&self, index: usize) -> &T {
        &self.palette[self.indices[index] as usize]
    }

    /// Sets a position, extending the palette when the value is new.
    pub fn set(&mut self, index: usize, value: T) {
        let slot = match self.palette.iter().position(|p| *p == value) {
            Some(i) => i,
            None => {
                self.palette.push(value);
                self.palette.len() - 1
            }
        };
        self.indices[index] = slot as u16;
    }

    /// Drops palette entries no position uses, keeping first-use order.
    pub fn compact(&mut self) {
        let mut remap = vec![u16::MAX; self.palette.len()];
        let mut palette = Vec::new();
        for idx in &mut self.indices {
            let old = *idx as usize;
            if remap[old] == u16::MAX {
                remap[old] = palette.len() as u16;
                palette.push(self.palette[old].clone());
            }
            *idx = remap[old];
        }
        self.palette = palette;
    }
}

fn packed_len(size: usize, bits: u32, spanning: bool) -> usize {
    if spanning {
        (size * bits as usize).div_ceil(64)
    } else {
        size.div_ceil(64 / bits as usize)
    }
}

fn infer_bits(longs: usize, size: usize, spanning: bool) -> Option<u32> {
    (1..=16).find(|&b| packed_len(size, b, spanning) == longs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_widths() {
        assert_eq!(bits_for(1, 4), 0);
        assert_eq!(bits_for(2, 4), 4);
        assert_eq!(bits_for(16, 4), 4);
        assert_eq!(bits_for(17, 4), 5);
        assert_eq!(bits_for(2, 1), 1);
        assert_eq!(bits_for(3, 1), 2);
        assert_eq!(bits_for(64, 1), 6);
    }

    fn sample(n: usize) -> PalettedContainer<u32> {
        let palette: Vec<u32> = (0..n as u32).collect();
        let indices = (0..4096).map(|i| ((i * 7919) % n) as u16).collect();
        PalettedContainer { palette, indices }
    }

    #[test]
    fn round_trips_both_packings() {
        for n in [2, 5, 16, 17, 33, 300] {
            for spanning in [false, true] {
                let c = sample(n);
                let data = c.encode(4, spanning);
                let back =
                    PalettedContainer::decode(c.palette.clone(), &data, 4096, 4, spanning).unwrap();
                assert_eq!(back, c, "n={n} spanning={spanning}");
            }
        }
    }

    #[test]
    fn non_spanning_leaves_top_bits_unused() {
        // 5 bits -> 12 entries per long, 342 longs for 4096 entries.
        assert_eq!(sample(17).encode(4, false).len(), 342);
        // Spanning packs 4096 * 5 bits into 320 longs.
        assert_eq!(sample(17).encode(4, true).len(), 320);
    }

    #[test]
    fn single_entry_palette_needs_no_data() {
        let c = PalettedContainer::filled("air", 4096);
        assert!(c.encode(4, false).is_empty());
        assert_eq!(
            PalettedContainer::decode(vec!["air"], &[], 4096, 4, false).unwrap(),
            c
        );
    }

    #[test]
    fn rejects_out_of_range_indices() {
        let data = vec![-1i64; 256];
        assert!(PalettedContainer::decode(vec![0u32, 1, 2], &data, 4096, 4, false).is_err());
    }

    #[test]
    fn set_and_compact() {
        let mut c = PalettedContainer::filled("stone", 8);
        c.set(3, "dirt");
        c.set(3, "stone");
        assert_eq!(c.palette.len(), 2);
        c.compact();
        assert_eq!(c.palette, vec!["stone"]);
    }
}
