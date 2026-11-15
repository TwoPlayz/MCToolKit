//! Item frames.
//!
//! Java keeps an item frame as an entity hanging in the block in front of a wall. Bedrock keeps
//! it as a block (`minecraft:frame` / `minecraft:glow_frame`, carrying the facing) plus a block
//! entity (`ItemFrame` / `GlowItemFrame`, carrying the item and its rotation in degrees).

use mctk_nbt::{Compound, List, Tag, TagType};

use crate::item_codec::{self, Notes};

pub const BLOCK: &str = "minecraft:frame";
pub const GLOW_BLOCK: &str = "minecraft:glow_frame";
pub const JAVA_ID: &str = "minecraft:item_frame";
pub const JAVA_GLOW_ID: &str = "minecraft:glow_item_frame";

/// Whether a Bedrock block is an item frame.
pub fn is_frame_block(name: &str) -> bool {
    name == BLOCK || name == GLOW_BLOCK
}

/// Whether a Java entity id is an item frame.
pub fn is_java_frame(id: &str) -> bool {
    id == JAVA_ID || id == JAVA_GLOW_ID
}

/// An item frame in the form both editions can be built from.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// The block the frame occupies (not the wall it hangs on).
    pub tile: (i32, i32, i32),
    /// Which way the frame faces, away from its wall: 0 down, 1 up, 2 north, 3 south, 4 west,
    /// 5 east (Java's `facing` and Bedrock's `facing_direction`).
    pub facing: u8,
    pub glow: bool,
    /// The held item, as a Java item compound.
    pub item: Option<Compound>,
    /// Turns of 45 degrees, 0 to 7.
    pub rotation: u8,
    pub map: bool,
}

fn int(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(Tag::as_i64)
}

fn step(facing: u8) -> (i32, i32, i32) {
    match facing {
        0 => (0, -1, 0),
        1 => (0, 1, 0),
        2 => (0, 0, -1),
        3 => (0, 0, 1),
        4 => (-1, 0, 0),
        _ => (1, 0, 0),
    }
}

fn doubles(v: &[f64]) -> Tag {
    Tag::List(List::from_tags(TagType::Double, v.iter().map(|d| Tag::Double(*d)).collect()).expect("doubles"))
}

fn floats(v: &[f32]) -> Tag {
    Tag::List(List::from_tags(TagType::Float, v.iter().map(|d| Tag::Float(*d)).collect()).expect("floats"))
}

fn position(entity: &Compound) -> Option<[f64; 3]> {
    let list = entity.get("Pos").and_then(Tag::as_list)?;
    let at = |i: usize| list.get(i).and_then(Tag::as_f64);
    Some([at(0)?, at(1)?, at(2)?])
}

fn is_air(item: &Compound) -> bool {
    item.get("id").and_then(Tag::as_str).is_none_or(|id| id == "minecraft:air")
}

impl Frame {
    /// Reads a Java item frame entity; `None` when the entity is something else.
    pub fn from_java(entity: &Compound) -> Option<Frame> {
        let id = entity.get("id").and_then(Tag::as_str)?;
        if !is_java_frame(id) {
            return None;
        }
        let item = entity.get("Item").and_then(Tag::as_compound).filter(|i| !is_air(i)).cloned();
        // 1.21.5 replaced TileX/Y/Z with block_pos.
        let tile = match entity.get("block_pos") {
            Some(Tag::IntArray(a)) if a.len() == 3 => (a[0], a[1], a[2]),
            _ => {
                let pos = position(entity).unwrap_or([0.0; 3]);
                let at = |key: &str, p: f64| int(entity, key).map_or(p.floor() as i32, |v| v as i32);
                (at("TileX", pos[0]), at("TileY", pos[1]), at("TileZ", pos[2]))
            }
        };
        let facing = int(entity, "facing").or_else(|| int(entity, "Facing")).unwrap_or(0).rem_euclid(6) as u8;
        let map = item.as_ref().and_then(|i| i.get("id")).and_then(Tag::as_str) == Some("minecraft:filled_map");
        Some(Frame {
            tile,
            facing,
            glow: id == JAVA_GLOW_ID,
            item,
            rotation: int(entity, "ItemRotation").unwrap_or(0).rem_euclid(8) as u8,
            map,
        })
    }

    /// Builds a frame from a Bedrock frame block (`name`, `states`) and, when there is one, the
    /// block entity next to it.
    pub fn from_bedrock(name: &str, states: &Compound, tile: (i32, i32, i32), entity: Option<&Compound>, notes: &mut Notes) -> Frame {
        let item = entity
            .and_then(|e| e.get("Item"))
            .and_then(Tag::as_compound)
            .and_then(|i| item_codec::bedrock_to_java(i, None, notes));
        let rotation = match entity.and_then(|e| e.get("ItemRotation")) {
            Some(Tag::Float(r)) => (((*r + 360.0) % 360.0) / 45.0).round() as i64,
            Some(t) => t.as_i64().unwrap_or(0),
            None => 0,
        };
        let glow = name == GLOW_BLOCK || entity.and_then(|e| e.get("id")).and_then(Tag::as_str) == Some("GlowItemFrame");
        Frame {
            tile,
            // Legacy values 6 and 7 fall back to down.
            facing: match int(states, "facing_direction").unwrap_or(0) {
                v @ 0..=5 => v as u8,
                _ => 0,
            },
            glow,
            item,
            rotation: rotation.rem_euclid(8) as u8,
            map: int(states, "item_frame_map_bit").unwrap_or(0) != 0,
        }
    }

    /// The Bedrock block the frame becomes: its name and block states.
    pub fn bedrock_block(&self) -> (&'static str, Compound) {
        let mut states = Compound::new();
        states.insert("facing_direction".into(), Tag::Int(self.facing as i32));
        states.insert("item_frame_map_bit".into(), Tag::Byte(self.map as i8));
        states.insert("item_frame_photo_bit".into(), Tag::Byte(0));
        (if self.glow { GLOW_BLOCK } else { BLOCK }, states)
    }

    /// The Bedrock block entity that holds the item.
    pub fn block_entity(&self, notes: &mut Notes) -> Compound {
        let mut be = Compound::new();
        be.insert("id".into(), Tag::String(if self.glow { "GlowItemFrame" } else { "ItemFrame" }.into()));
        be.insert("x".into(), Tag::Int(self.tile.0));
        be.insert("y".into(), Tag::Int(self.tile.1));
        be.insert("z".into(), Tag::Int(self.tile.2));
        be.insert("isMovable".into(), Tag::Byte(1));
        be.insert("ItemDropChance".into(), Tag::Float(1.0));
        if let Some(item) = self.item.as_ref().and_then(|i| item_codec::java_to_bedrock(i, None, notes)) {
            be.insert("Item".into(), Tag::Compound(item));
        }
        be.insert("ItemRotation".into(), Tag::Float(self.rotation as f32 * 45.0));
        be
    }

    /// The Java entity. `uuid` is made up by the caller (Bedrock frames have none).
    pub fn to_java(&self, uuid: [i32; 4]) -> Compound {
        let s = step(self.facing);
        // The entity sits against its wall: the block's centre moved 0.03125 in from the face.
        let pos = [
            self.tile.0 as f64 + 0.5 - s.0 as f64 * 0.46875,
            self.tile.1 as f64 + 0.5 - s.1 as f64 * 0.46875,
            self.tile.2 as f64 + 0.5 - s.2 as f64 * 0.46875,
        ];
        let yaw = match self.facing {
            2 => 180.0,
            4 => 90.0,
            5 => 270.0,
            _ => 0.0,
        };
        let pitch = match self.facing {
            0 => 90.0,
            1 => -90.0,
            _ => 0.0,
        };
        let mut out = Compound::new();
        out.insert("id".into(), Tag::String(if self.glow { JAVA_GLOW_ID } else { JAVA_ID }.into()));
        out.insert("Pos".into(), doubles(&pos));
        out.insert("Motion".into(), doubles(&[0.0, 0.0, 0.0]));
        out.insert("Rotation".into(), floats(&[yaw, pitch]));
        out.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
        out.insert("block_pos".into(), Tag::IntArray(vec![self.tile.0, self.tile.1, self.tile.2]));
        out.insert("facing".into(), Tag::Byte(self.facing as i8));
        out.insert("ItemRotation".into(), Tag::Byte(self.rotation as i8));
        out.insert("ItemDropChance".into(), Tag::Float(1.0));
        if let Some(item) = &self.item {
            out.insert("Item".into(), Tag::Compound(item.clone()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn java_frame(id: &str, facing: i8) -> Compound {
        let mut item = Compound::new();
        item.insert("id".into(), Tag::String("minecraft:diamond".into()));
        item.insert("count".into(), Tag::Int(3));
        let mut e = Compound::new();
        e.insert("id".into(), Tag::String(id.into()));
        e.insert("block_pos".into(), Tag::IntArray(vec![-17, 70, 40]));
        e.insert("facing".into(), Tag::Byte(facing));
        e.insert("ItemRotation".into(), Tag::Byte(5));
        e.insert("Item".into(), Tag::Compound(item));
        e
    }

    #[test]
    fn only_frames_are_read() {
        let mut cow = Compound::new();
        cow.insert("id".into(), Tag::String("minecraft:cow".into()));
        assert!(Frame::from_java(&cow).is_none());
        assert!(Frame::from_java(&java_frame(JAVA_ID, 3)).is_some());
    }

    #[test]
    fn a_java_frame_becomes_a_block_and_an_entity() {
        let f = Frame::from_java(&java_frame(JAVA_GLOW_ID, 5)).unwrap();
        assert_eq!(f.tile, (-17, 70, 40));
        let (name, states) = f.bedrock_block();
        assert_eq!(name, GLOW_BLOCK);
        assert_eq!(int(&states, "facing_direction"), Some(5));
        let be = f.block_entity(&mut Notes::default());
        assert_eq!(be.get("id").and_then(Tag::as_str), Some("GlowItemFrame"));
        assert_eq!(int(&be, "x"), Some(-17));
        assert_eq!(be.get("ItemRotation"), Some(&Tag::Float(225.0)));
        assert!(be.get("Item").is_some());
    }

    #[test]
    fn bedrock_to_java_and_back_keeps_the_frame() {
        let mut notes = Notes::default();
        let original = Frame::from_java(&java_frame(JAVA_ID, 2)).unwrap();
        let (name, states) = original.bedrock_block();
        let be = original.block_entity(&mut notes);
        let back = Frame::from_bedrock(name, &states, original.tile, Some(&be), &mut notes);
        assert_eq!(back.facing, 2);
        assert_eq!(back.rotation, 5);
        assert_eq!(
            back.item.as_ref().and_then(|i| i.get("id")).and_then(Tag::as_str),
            Some("minecraft:diamond")
        );
        let entity = back.to_java([1, 2, 3, 4]);
        let again = Frame::from_java(&entity).unwrap();
        assert_eq!((again.tile, again.facing, again.rotation), (original.tile, 2, 5));
        // Facing north, so the frame sits against the wall on its south side.
        assert_eq!(position(&entity).unwrap(), [-16.5, 70.5, 40.5 + 0.46875]);
    }

    #[test]
    fn empty_frames_and_odd_states() {
        let mut states = Compound::new();
        states.insert("facing_direction".into(), Tag::Int(7));
        states.insert("item_frame_map_bit".into(), Tag::Byte(1));
        let f = Frame::from_bedrock(GLOW_BLOCK, &states, (0, 0, 0), None, &mut Notes::default());
        assert_eq!(f.facing, 0);
        assert!(f.map && f.glow && f.item.is_none());
        let e = f.to_java([0; 4]);
        assert_eq!(e.get("id").and_then(Tag::as_str), Some(JAVA_GLOW_ID));
        assert!(e.get("Item").is_none());
        assert_eq!(e.get("Rotation"), Some(&floats(&[0.0, 90.0])));
    }
}
