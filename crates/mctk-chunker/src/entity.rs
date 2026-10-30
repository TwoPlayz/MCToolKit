//! Entity types and paintings. Ported from Chunker's entity type resolvers and painting
//! handlers (MIT, Hive Games).
//!
//! Chunker converts an entity's type, position, motion and rotation, and treats paintings
//! and item frames specially. The type table (`data/entity_types.tsv`) lists, per Chunker
//! entity type, its Java and Bedrock ids for the current versions (Java 26.3, Bedrock
//! 1.26.60); it was produced by evaluating the version conditions in Chunker's
//! `JavaEntityTypeResolver` and `BedrockEntityTypeResolver`.

use std::collections::HashMap;
use std::sync::OnceLock;

struct Types {
    to_bedrock: HashMap<&'static str, &'static str>,
    to_java: HashMap<&'static str, &'static str>,
}

fn types() -> &'static Types {
    static T: OnceLock<Types> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = Types {
            to_bedrock: HashMap::new(),
            to_java: HashMap::new(),
        };
        for line in include_str!("../data/entity_types.tsv").lines() {
            let mut f = line.split('\t');
            let (Some(_), Some(java), Some(bedrock)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            if !java.is_empty() && !bedrock.is_empty() {
                t.to_bedrock.insert(java, bedrock);
                t.to_java.insert(bedrock, java);
            }
        }
        t
    })
}

/// The Bedrock id of a Java entity type; `None` when Bedrock has no such entity.
pub fn java_to_bedrock_type(java: &str) -> Option<&'static str> {
    types().to_bedrock.get(java).copied()
}

/// The Java id of a Bedrock entity type; `None` when Java has no such entity.
pub fn bedrock_to_java_type(bedrock: &str) -> Option<&'static str> {
    types().to_java.get(bedrock).copied()
}

// ------------------------------------------------------------------- paintings

/// A painting motive: (Java name, Bedrock name, width, height in blocks).
pub type Motive = (&'static str, &'static str, i32, i32);

pub const MOTIVES: [Motive; 51] = [
    ("minecraft:kebab", "Kebab", 1, 1),
    ("minecraft:aztec", "Aztec", 1, 1),
    ("minecraft:alban", "Alban", 1, 1),
    ("minecraft:aztec2", "Aztec2", 1, 1),
    ("minecraft:bomb", "Bomb", 1, 1),
    ("minecraft:plant", "Plant", 1, 1),
    ("minecraft:wasteland", "Wasteland", 1, 1),
    ("minecraft:meditative", "meditative", 1, 1),
    ("minecraft:wanderer", "Wanderer", 1, 2),
    ("minecraft:graham", "Graham", 1, 2),
    ("minecraft:prairie_ride", "prairie_ride", 1, 2),
    ("minecraft:sea", "Sea", 2, 1),
    ("minecraft:creebet", "Creebet", 2, 1),
    ("minecraft:sunset", "Sunset", 2, 1),
    ("minecraft:courbet", "Courbet", 2, 1),
    ("minecraft:pool", "Pool", 2, 1),
    ("minecraft:earth", "Earth", 2, 2),
    ("minecraft:fire", "Fire", 2, 2),
    ("minecraft:water", "Water", 2, 2),
    ("minecraft:wind", "Wind", 2, 2),
    ("minecraft:match", "Match", 2, 2),
    ("minecraft:bust", "Bust", 2, 2),
    ("minecraft:stage", "Stage", 2, 2),
    ("minecraft:void", "Void", 2, 2),
    ("minecraft:skull_and_roses", "SkullAndRoses", 2, 2),
    ("minecraft:wither", "Wither", 2, 2),
    ("minecraft:baroque", "baroque", 2, 2),
    ("minecraft:humble", "humble", 2, 2),
    ("minecraft:bouquet", "bouquet", 3, 3),
    ("minecraft:cavebird", "cavebird", 3, 3),
    ("minecraft:cotan", "cotan", 3, 3),
    ("minecraft:endboss", "endboss", 3, 3),
    ("minecraft:fern", "fern", 3, 3),
    ("minecraft:owlemons", "owlemons", 3, 3),
    ("minecraft:sunflowers", "sunflowers", 3, 3),
    ("minecraft:tides", "tides", 3, 3),
    ("minecraft:dennis", "dennis", 3, 3),
    ("minecraft:backyard", "backyard", 3, 4),
    ("minecraft:pond", "pond", 3, 4),
    ("minecraft:fighters", "Fighters", 4, 2),
    ("minecraft:changing", "changing", 4, 2),
    ("minecraft:finding", "finding", 4, 2),
    ("minecraft:lowmist", "lowmist", 4, 2),
    ("minecraft:passage", "passage", 4, 2),
    ("minecraft:skeleton", "Skeleton", 4, 3),
    ("minecraft:donkey_kong", "DonkeyKong", 4, 3),
    ("minecraft:pointer", "Pointer", 4, 4),
    ("minecraft:pigscene", "Pigscene", 4, 4),
    ("minecraft:burning_skull", "BurningSkull", 4, 4),
    ("minecraft:orb", "orb", 4, 4),
    ("minecraft:unpacked", "unpacked", 4, 4),
];

pub fn motive_from_java(name: &str) -> Option<&'static Motive> {
    let full;
    let name = if name.contains(':') {
        name
    } else {
        full = format!("minecraft:{name}");
        &full
    };
    MOTIVES.iter().find(|m| m.0 == name)
}

pub fn motive_from_bedrock(name: &str) -> Option<&'static Motive> {
    MOTIVES.iter().find(|m| m.1 == name)
}

/// Which way a painting faces (Java's and Bedrock's `facing` / `Direction` byte: 0 south,
/// 1 west, 2 north, 3 east).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    South,
    West,
    North,
    East,
}

impl Facing {
    pub fn from_byte(b: i64) -> Facing {
        match b.rem_euclid(4) {
            0 => Facing::South,
            1 => Facing::West,
            2 => Facing::North,
            _ => Facing::East,
        }
    }

    pub fn to_byte(self) -> i8 {
        match self {
            Facing::South => 0,
            Facing::West => 1,
            Facing::North => 2,
            Facing::East => 3,
        }
    }
}

/// Offset from the painting's block position to its tile, in whole blocks.
fn tile_offset(m: &Motive, facing: Facing) -> (i32, i32, i32) {
    let (w, h) = (m.2, m.3);
    (
        if facing == Facing::South && w % 2 == 0 { -1 } else { 0 },
        if h % 2 == 0 { -1 } else { 0 },
        if facing == Facing::West && w % 2 == 0 { -1 } else { 0 },
    )
}

/// Offset between Bedrock's painting position and the neutral position.
fn position_offset(m: &Motive, facing: Facing) -> (f64, f64, f64) {
    let (w, h) = (m.2, m.3);
    let y = if h % 2 == 0 { 0.0 } else { -0.5 };
    let along = if w % 2 == 0 { 0.0 } else { -0.5 };
    match facing {
        Facing::North => (along, y, -0.96875),
        Facing::East => (-0.03125, y, along),
        Facing::South => (along, y, -0.03125),
        Facing::West => (-0.96875, y, along),
    }
}

/// Java → Bedrock: the position of a painting from its block position (`block_pos`).
pub fn bedrock_position(tile: (i32, i32, i32), m: &Motive, facing: Facing) -> (f64, f64, f64) {
    let (to, po) = (tile_offset(m, facing), position_offset(m, facing));
    (
        (tile.0 - to.0) as f64 - po.0,
        (tile.1 - to.1) as f64 - po.1,
        (tile.2 - to.2) as f64 - po.2,
    )
}

/// Bedrock → Java: the neutral position and block position of a painting from the
/// position Bedrock stores.
pub fn java_position(
    pos: (f64, f64, f64),
    m: &Motive,
    facing: Facing,
) -> ((f64, f64, f64), (i32, i32, i32)) {
    let (to, po) = (tile_offset(m, facing), position_offset(m, facing));
    let neutral = (pos.0 + po.0, pos.1 + po.1, pos.2 + po.2);
    let tile = (
        (neutral.0 + to.0 as f64).floor() as i32,
        (neutral.1 + to.1 as f64).floor() as i32,
        (neutral.2 + to.2 as f64).floor() as i32,
    );
    (neutral, tile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_table_has_the_real_renames() {
        assert_eq!(java_to_bedrock_type("minecraft:end_crystal"), Some("minecraft:ender_crystal"));
        assert_eq!(bedrock_to_java_type("minecraft:evocation_illager"), Some("minecraft:evoker"));
        assert_eq!(java_to_bedrock_type("minecraft:oak_boat"), Some("minecraft:boat"));
        assert_eq!(java_to_bedrock_type("minecraft:cow"), Some("minecraft:cow"));
        assert_eq!(java_to_bedrock_type("minecraft:giant"), None);
        assert_eq!(bedrock_to_java_type("minecraft:moving_block"), None);
    }

    #[test]
    fn every_motive_has_both_names() {
        assert_eq!(MOTIVES.len(), 51);
        for m in &MOTIVES {
            assert_eq!(motive_from_java(m.0), Some(m));
            assert_eq!(motive_from_bedrock(m.1), Some(m));
        }
        assert_eq!(motive_from_java("kebab").unwrap().1, "Kebab");
    }

    #[test]
    fn paintings_round_trip_their_block_position() {
        for m in &MOTIVES {
            for facing in [Facing::South, Facing::West, Facing::North, Facing::East] {
                let tile = (-13, 70, 41);
                let pos = bedrock_position(tile, m, facing);
                let (_, back) = java_position(pos, m, facing);
                assert_eq!(back, tile, "{} {facing:?}", m.0);
            }
        }
        // A one-block painting facing south sits in the middle of its block, against the
        // south face.
        let kebab = motive_from_java("minecraft:kebab").unwrap();
        assert_eq!(bedrock_position((10, 64, 20), kebab, Facing::South), (10.5, 64.5, 20.03125));
    }

    #[test]
    fn facing_bytes() {
        for b in 0..4 {
            assert_eq!(Facing::from_byte(b).to_byte() as i64, b);
        }
        assert_eq!(Facing::from_byte(0), Facing::South);
        assert_eq!(Facing::from_byte(6), Facing::North);
    }
}

