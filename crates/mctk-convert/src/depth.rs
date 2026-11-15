//! The Depth Fixer: what happens to a 1.18+ overworld, which reaches down to y -64, when it is
//! written for a version whose world starts at y 0 and is 256 blocks tall.

use mctk_java::{BlockState, BlockStorage, Chunk, PalettedContainer, Section, block_index};
use mctk_nbt::{Compound, Tag};

/// How the part of the world below y 0 is dealt with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DepthMode {
    /// Cut everything below y 0 and make y 0 a layer of bedrock.
    #[default]
    ClipBedrock,
    /// Cut everything below y 0.
    Clip,
    /// Move the whole world up by 64 blocks, so the deep layers survive; what then rises above
    /// y 255 is cut.
    Raise,
}

/// How far `Raise` moves the world.
pub const RAISE: i32 = 64;

/// Whether the chunk reaches below y 0 (only the deep overworld does).
pub fn applies(chunk: &Chunk) -> bool {
    chunk.sections.iter().any(|s| s.y < 0)
}

/// Applies `mode` to the chunk's sections; returns how many blocks it moved up.
pub fn fix(chunk: &mut Chunk, mode: DepthMode) -> i32 {
    match mode {
        DepthMode::Clip => 0,
        DepthMode::ClipBedrock => {
            bedrock_floor(chunk);
            0
        }
        DepthMode::Raise => {
            for s in &mut chunk.sections {
                s.y += (RAISE / 16) as i8;
            }
            RAISE
        }
    }
}

fn bedrock_floor(chunk: &mut Chunk) {
    let bedrock = BlockState::new("minecraft:bedrock");
    if !chunk.sections.iter().any(|s| s.y == 0) {
        chunk.sections.push(Section::new(0, Some(BlockStorage::Palette(PalettedContainer::filled(BlockState::air(), 4096))), None));
    }
    let Some(section) = chunk.sections.iter_mut().find(|s| s.y == 0) else { return };
    let Some(BlockStorage::Palette(p)) = &mut section.blocks else { return };
    let id = match p.palette.iter().position(|b| *b == bedrock) {
        Some(i) => i,
        None => {
            p.palette.push(bedrock);
            p.palette.len() - 1
        }
    };
    for z in 0..16 {
        for x in 0..16 {
            p.indices[block_index(x, 0, z)] = id as u16;
        }
    }
}

/// Moves a list of entities (or tile entities) up by `by` blocks.
pub fn raise_entities(list: &mut [Tag], by: i32) {
    for t in list {
        if let Some(c) = t.as_compound_mut() {
            raise(c, by);
        }
    }
}

/// Moves one entity or tile entity up by `by` blocks.
pub fn raise(c: &mut Compound, by: i32) {
    if let Some(Tag::List(pos)) = c.get_mut("Pos") {
        if let Some(Tag::Double(y)) = pos.iter_mut().nth(1) {
            *y += by as f64;
        }
    }
    for key in ["y", "TileY", "tileY"] {
        if let Some(Tag::Int(y)) = c.get_mut(key) {
            *y += by;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mctk_java::PaletteStyle;

    fn deep_chunk() -> Chunk {
        let stone = || Some(BlockStorage::Palette(PalettedContainer::filled(BlockState::new("minecraft:stone"), 4096)));
        Chunk::new_modern(
            0,
            0,
            1,
            PaletteStyle::IdString,
            Compound::new(),
            vec![Section::new(-1, stone(), None), Section::new(0, stone(), None), Section::new(1, stone(), None)],
        )
    }

    #[test]
    fn raise_moves_every_section_up_four() {
        let mut c = deep_chunk();
        assert!(applies(&c));
        assert_eq!(fix(&mut c, DepthMode::Raise), 64);
        let ys: Vec<i8> = c.sections.iter().map(|s| s.y).collect();
        assert_eq!(ys, vec![3, 4, 5]);
    }

    #[test]
    fn bedrock_floor_fills_the_bottom_layer_of_section_zero() {
        let mut c = deep_chunk();
        fix(&mut c, DepthMode::ClipBedrock);
        assert_eq!(c.block(3, 0, 5).map(|b| b.name.as_str()), Some("minecraft:bedrock"));
        assert_eq!(c.block(3, 1, 5).map(|b| b.name.as_str()), Some("minecraft:stone"));
    }

    #[test]
    fn entities_move_with_the_world() {
        let mut e = Compound::from([("Pos".to_string(), Tag::List(mctk_nbt::List::from_tags(mctk_nbt::TagType::Double, vec![Tag::Double(1.0), Tag::Double(70.0), Tag::Double(2.0)]).unwrap()))]);
        raise(&mut e, 64);
        let y = e.get("Pos").and_then(Tag::as_list).and_then(|l| l.iter().nth(1).and_then(Tag::as_f64));
        assert_eq!(y, Some(134.0));
    }
}
