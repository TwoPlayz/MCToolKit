//! How the Bedrock version being written lays its chunks out. Versions before 1.18 start the
//! overworld at y 0 (sixteen subchunks) and keep heights and biomes in `Data2D`; versions before
//! 1.18.30 keep a chunk's entities inside it rather than as separate actors.

use std::sync::Mutex;

/// The parts of the chunk format that differ between Bedrock versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// The overworld is 256 blocks tall, y 0 to 255, with `Data2D` records (before 1.18).
    pub legacy_height: bool,
    /// The chunk version byte the game of that version writes.
    pub chunk_version: u8,
    /// Entities are a record of their chunk, not actors with a digest (before 1.18.30).
    pub entities_in_chunk: bool,
    /// Chunks carry a `BlendingData` record (1.18 on).
    pub blending: bool,
}

impl Layout {
    /// The newest version's layout.
    pub const NEWEST: Layout = Layout { legacy_height: false, chunk_version: 42, entities_in_chunk: false, blending: true };

    /// The layout of the version a registry describes, from its facts (see `bedrock-finish`).
    pub fn of(registry: &mctk_registry::Registry) -> Layout {
        let meta = &registry.meta;
        let client: Vec<i32> = meta
            .get("client_version")
            .map(|v| v.split(',').filter_map(|n| n.parse().ok()).collect())
            .unwrap_or_default();
        let at_least = |major: i32, minor: i32, patch: i32| {
            client.len() >= 3 && (client[0], client[1], client[2]) >= (major, minor, patch)
        };
        if client.len() < 3 {
            return Layout::NEWEST;
        }
        let legacy_height = match meta.get("height_record").map(String::as_str) {
            Some("data2d") => true,
            Some(_) => false,
            None => !at_least(1, 18, 0),
        };
        Layout {
            legacy_height,
            chunk_version: meta
                .get("chunk_version")
                .and_then(|v| v.parse().ok())
                .unwrap_or(if legacy_height { 25 } else { Layout::NEWEST.chunk_version }),
            entities_in_chunk: !at_least(1, 18, 30),
            blending: !legacy_height,
        }
    }
}

static LAYOUT: Mutex<Layout> = Mutex::new(Layout::NEWEST);

pub fn set(layout: Layout) {
    *LAYOUT.lock().expect("layout") = layout;
}

pub fn get() -> Layout {
    *LAYOUT.lock().expect("layout")
}

/// A pre-1.18 `Data2D` record: the 16x16 height map (z * 16 + x, 16-bit) and a biome id per column.
pub fn data2d(heightmap: &[i16], biomes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(768);
    for h in heightmap {
        out.extend_from_slice(&h.to_le_bytes());
    }
    out.extend_from_slice(biomes);
    out
}

/// The biome id a version before 1.18 knows for a newer one (biomes added in 1.18 and later fold
/// into the closest older biome; the rest are unchanged).
pub fn legacy_biome(id: i32) -> u8 {
    match id {
        // Up to the nether biomes of 1.16 (basalt deltas, 181) the ids are the old ones.
        0..=181 => id as u8,
        // jagged and frozen peaks, snowy slopes, grove, stony peaks, mangrove swamp; the caves,
        // meadow, deep dark, cherry grove and the rest become plains
        182 | 183 => 13,
        184 => 12,
        185 => 30,
        189 => 3,
        191 => 6,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_biomes_fold_into_old_ones() {
        assert_eq!(legacy_biome(1), 1);
        assert_eq!(legacy_biome(179), 179);
        assert_eq!(legacy_biome(184), 12);
        assert_eq!(legacy_biome(191), 6);
        assert_eq!(legacy_biome(187), 1);
    }

    #[test]
    fn data2d_is_height_map_then_biomes() {
        let d = data2d(&[7; 256], &[4; 256]);
        assert_eq!(d.len(), 768);
        assert_eq!(&d[..2], &7i16.to_le_bytes());
        assert_eq!(d[512], 4);
    }
}
