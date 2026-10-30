//! LevelDB key layout.
//!
//! Chunk records use `x: i32 LE, z: i32 LE, [dimension: i32 LE], tag: u8, [subchunk: i8]`,
//! where the dimension is omitted for the Overworld and the subchunk index is present only
//! for `SubChunkPrefix`. Other keys are names such as `~local_player`, `actorprefix<uid>`
//! or `digp<x><z>[<dim>]`.

/// Dimension ids used in keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
}

impl Dimension {
    pub fn id(self) -> i32 {
        match self {
            Dimension::Overworld => 0,
            Dimension::Nether => 1,
            Dimension::End => 2,
        }
    }

    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Dimension::Overworld),
            1 => Some(Dimension::Nether),
            2 => Some(Dimension::End),
            _ => None,
        }
    }
}

macro_rules! tags {
    ($($name:ident = $value:expr),* $(,)?) => {
        /// Chunk record tags (the byte after the coordinates).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum ChunkTag {
            $($name,)*
            /// A tag this crate does not know (newer game versions add tags); its record
            /// is kept verbatim.
            Unknown(u8),
        }

        impl ChunkTag {
            pub fn from_byte(b: u8) -> Self {
                match b {
                    $($value => ChunkTag::$name,)*
                    other => ChunkTag::Unknown(other),
                }
            }

            pub fn byte(self) -> u8 {
                match self {
                    $(ChunkTag::$name => $value,)*
                    ChunkTag::Unknown(b) => b,
                }
            }

            pub fn is_known(self) -> bool {
                !matches!(self, ChunkTag::Unknown(_))
            }
        }
    };
}

/// Coordinates beyond this are outside any reachable world (±30 million blocks).
const MAX_CHUNK_COORD: i32 = 30_000_000 / 16 + 1;

tags! {
    Data3D = 43,
    Version = 44,
    Data2D = 45,
    Data2DLegacy = 46,
    SubChunkPrefix = 47,
    LegacyTerrain = 48,
    BlockEntity = 49,
    Entity = 50,
    PendingTicks = 51,
    LegacyBlockExtraData = 52,
    BiomeState = 53,
    FinalizedState = 54,
    ConversionData = 55,
    BorderBlocks = 56,
    HardcodedSpawners = 57,
    RandomTicks = 58,
    Checksums = 59,
    GenerationSeed = 60,
    GeneratedPreCavesAndCliffsBlending = 61,
    BlendingBiomeHeight = 62,
    MetaDataHash = 63,
    BlendingData = 64,
    ActorDigestVersion = 65,
    LegacyVersion = 118,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkKey {
    pub x: i32,
    pub z: i32,
    pub dimension: Dimension,
    pub tag: ChunkTag,
    /// Subchunk Y index, only for [`ChunkTag::SubChunkPrefix`].
    pub subchunk: Option<i8>,
}

impl ChunkKey {
    pub fn encode(&self) -> Vec<u8> {
        let mut k = Vec::with_capacity(14);
        k.extend_from_slice(&self.x.to_le_bytes());
        k.extend_from_slice(&self.z.to_le_bytes());
        if self.dimension != Dimension::Overworld {
            k.extend_from_slice(&self.dimension.id().to_le_bytes());
        }
        k.push(self.tag.byte());
        if let Some(y) = self.subchunk {
            k.push(y as u8);
        }
        k
    }

    fn decode(key: &[u8]) -> Option<Self> {
        let i32_at = |o: usize| i32::from_le_bytes(key[o..o + 4].try_into().unwrap());
        let (dimension, tag_at) = match key.len() {
            9 | 10 => (Dimension::Overworld, 8),
            13 | 14 => (
                Dimension::from_id(i32_at(8)).filter(|d| *d != Dimension::Overworld)?,
                12,
            ),
            _ => return None,
        };
        let tag = ChunkTag::from_byte(key[tag_at]);
        let has_sub = key.len() == tag_at + 2;
        if has_sub != (tag == ChunkTag::SubChunkPrefix) {
            return None;
        }
        let (x, z) = (i32_at(0), i32_at(4));
        if x.abs() > MAX_CHUNK_COORD || z.abs() > MAX_CHUNK_COORD {
            return None;
        }
        // Unknown tags are accepted only when the key cannot be a printable name.
        if !tag.is_known() && key.iter().all(|b| b.is_ascii_graphic()) {
            return None;
        }
        Some(ChunkKey {
            x,
            z,
            dimension,
            tag,
            subchunk: has_sub.then(|| key[tag_at + 1] as i8),
        })
    }
}

/// A classified database key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    Chunk(ChunkKey),
    /// `actorprefix` + 8-byte actor unique id.
    Actor(Vec<u8>),
    /// `digp` + chunk position (+ dimension): list of actor ids in that chunk.
    ActorDigest {
        x: i32,
        z: i32,
        dimension: Dimension,
    },
    /// Any other key, e.g. `~local_player`, `player_<uuid>`, `portals`, `BiomeData`.
    Other(Vec<u8>),
}

impl Key {
    pub fn decode(key: &[u8]) -> Key {
        if let Some(uid) = key.strip_prefix(b"actorprefix") {
            return Key::Actor(uid.to_vec());
        }
        if let Some(rest) = key.strip_prefix(b"digp") {
            let i32_at = |o: usize| i32::from_le_bytes(rest[o..o + 4].try_into().unwrap());
            let dim = match rest.len() {
                8 => Some(Dimension::Overworld),
                12 => Dimension::from_id(i32_at(8)),
                _ => None,
            };
            if let Some(dimension) = dim {
                return Key::ActorDigest {
                    x: i32_at(0),
                    z: i32_at(4),
                    dimension,
                };
            }
        }
        match ChunkKey::decode(key) {
            Some(c) => Key::Chunk(c),
            None => Key::Other(key.to_vec()),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        match self {
            Key::Chunk(c) => c.encode(),
            Key::Actor(uid) => [b"actorprefix".as_slice(), uid].concat(),
            Key::ActorDigest { x, z, dimension } => {
                let mut k = b"digp".to_vec();
                k.extend_from_slice(&x.to_le_bytes());
                k.extend_from_slice(&z.to_le_bytes());
                if *dimension != Dimension::Overworld {
                    k.extend_from_slice(&dimension.id().to_le_bytes());
                }
                k
            }
            Key::Other(k) => k.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_keys_round_trip() {
        for key in [
            ChunkKey {
                x: 3,
                z: -7,
                dimension: Dimension::Overworld,
                tag: ChunkTag::Version,
                subchunk: None,
            },
            ChunkKey {
                x: -1,
                z: 0,
                dimension: Dimension::Nether,
                tag: ChunkTag::SubChunkPrefix,
                subchunk: Some(-4),
            },
            ChunkKey {
                x: 0,
                z: 9,
                dimension: Dimension::End,
                tag: ChunkTag::Data3D,
                subchunk: None,
            },
        ] {
            let bytes = key.encode();
            assert_eq!(Key::decode(&bytes), Key::Chunk(key));
        }
    }

    #[test]
    fn named_keys_are_not_chunks() {
        // "~local_player" is 13 bytes, like an Overworld-less chunk key; its "dimension"
        // bytes are invalid, so it must stay a named key.
        assert!(matches!(Key::decode(b"~local_player"), Key::Other(_)));
        assert!(matches!(Key::decode(b"portals"), Key::Other(_)));
        assert!(matches!(
            Key::decode(b"actorprefix\x01\0\0\0\0\0\0\0"),
            Key::Actor(_)
        ));
        let digest = Key::ActorDigest {
            x: 1,
            z: 2,
            dimension: Dimension::End,
        };
        assert_eq!(Key::decode(&digest.encode()), digest);
    }
}
