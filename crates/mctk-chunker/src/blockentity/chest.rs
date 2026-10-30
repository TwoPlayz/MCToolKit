//! Double chests and trapped chests, which depend on the block the entity sits in
//! (`BedrockChestBlockEntityHandler.updateBeforeProcess` / `updateBeforeWrite`).
//!
//! Java stores a double chest's halves in the block state (`type` = left / right and the
//! `facing`); Bedrock stores, in the block entity, where the other half is, and marks the
//! right half as the "lead". Bedrock also uses one `Chest` entity for trapped chests, which
//! Java tells apart by the block.

use mctk_java::BlockState;

use super::{BlockEntity, ChestPair, ContainerKind, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Facing {
    North,
    East,
    South,
    West,
}

impl Facing {
    fn parse(s: &str) -> Option<Facing> {
        Some(match s {
            "north" => Facing::North,
            "east" => Facing::East,
            "south" => Facing::South,
            "west" => Facing::West,
            _ => return None,
        })
    }

    fn delta(self) -> (i32, i32) {
        match self {
            Facing::North => (0, -1),
            Facing::East => (1, 0),
            Facing::South => (0, 1),
            Facing::West => (-1, 0),
        }
    }

    fn clockwise(self) -> Facing {
        match self {
            Facing::North => Facing::East,
            Facing::East => Facing::South,
            Facing::South => Facing::West,
            Facing::West => Facing::North,
        }
    }

    fn anticlockwise(self) -> Facing {
        match self {
            Facing::North => Facing::West,
            Facing::West => Facing::South,
            Facing::South => Facing::East,
            Facing::East => Facing::North,
        }
    }
}

/// Before writing Bedrock: derives the pairing from the Java block's `type` and `facing`.
pub fn prepare_for_bedrock(be: &mut BlockEntity, block: &BlockState) {
    let (x, z) = (be.x, be.z);
    let Kind::Container(c) = &mut be.kind else { return };
    if !matches!(c.kind, ContainerKind::Chest | ContainerKind::TrappedChest) {
        return;
    }
    c.chest_pair = None;
    let facing = block.properties.get("facing").and_then(|f| Facing::parse(f));
    let (Some(facing), Some(ty)) = (facing, block.properties.get("type")) else {
        return;
    };
    let target = match ty.as_str() {
        "left" => facing.clockwise(),
        "right" => facing.anticlockwise(),
        _ => return,
    };
    let (dx, dz) = target.delta();
    c.chest_pair = Some(ChestPair {
        x: x + dx,
        z: z + dz,
        lead: ty == "right",
    });
}

/// After reading Bedrock: makes a trapped chest of a `Chest` entity sitting in a trapped
/// chest block, and returns the `type` (`left`, `right`, `single`) the Java block should
/// have. `None` when the entity is not a chest.
pub fn resolve_for_java(be: &mut BlockEntity, block: &BlockState) -> Option<&'static str> {
    let (x, z) = (be.x, be.z);
    let Kind::Container(c) = &mut be.kind else { return None };
    if !matches!(c.kind, ContainerKind::Chest | ContainerKind::TrappedChest) {
        return None;
    }
    if block.name == "minecraft:trapped_chest" {
        c.kind = ContainerKind::TrappedChest;
    }
    let facing = block.properties.get("facing").and_then(|f| Facing::parse(f));
    let ty = match (c.chest_pair, facing) {
        (Some(pair), Some(facing)) => {
            let at = |f: Facing| (x + f.delta().0, z + f.delta().1);
            if at(facing.clockwise()) == (pair.x, pair.z) {
                "left"
            } else if at(facing.anticlockwise()) == (pair.x, pair.z) {
                "right"
            } else {
                // A pair that points nowhere valid is a single chest.
                "single"
            }
        }
        _ => "single",
    };
    c.chest_pair = None;
    Some(ty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockentity::Container;

    fn chest(x: i32, z: i32) -> BlockEntity {
        BlockEntity::new(x, 64, z, Kind::Container(Container::new(ContainerKind::Chest)))
    }

    fn block(name: &str, facing: &str, ty: &str) -> BlockState {
        BlockState::new(name).with("facing", facing).with("type", ty)
    }

    #[test]
    fn a_double_chest_round_trips_through_both_editions() {
        for facing in ["north", "east", "south", "west"] {
            // Left half at (10, 10); its partner lies clockwise of the facing.
            let mut left = chest(10, 10);
            prepare_for_bedrock(&mut left, &block("minecraft:chest", facing, "left"));
            let Kind::Container(lc) = &left.kind else { panic!() };
            let pair = lc.chest_pair.expect("paired");
            assert!(!pair.lead);

            // The partner is the right half, pointing back at the left half.
            let mut right = chest(pair.x, pair.z);
            prepare_for_bedrock(&mut right, &block("minecraft:chest", facing, "right"));
            let Kind::Container(rc) = &right.kind else { panic!() };
            let rpair = rc.chest_pair.expect("paired");
            assert!(rpair.lead);
            assert_eq!((rpair.x, rpair.z), (10, 10), "{facing}");

            // And back to Java.
            let java_left = block("minecraft:chest", facing, "single");
            assert_eq!(resolve_for_java(&mut left, &java_left), Some("left"), "{facing}");
            assert_eq!(resolve_for_java(&mut right, &java_left), Some("right"), "{facing}");
        }
    }

    #[test]
    fn single_chests_and_bad_pairs() {
        let mut c = chest(0, 0);
        prepare_for_bedrock(&mut c, &block("minecraft:chest", "north", "single"));
        let Kind::Container(cc) = &mut c.kind else { panic!() };
        assert!(cc.chest_pair.is_none());
        cc.chest_pair = Some(ChestPair { x: 50, z: 50, lead: false });
        assert_eq!(
            resolve_for_java(&mut c, &block("minecraft:chest", "north", "single")),
            Some("single")
        );
    }

    #[test]
    fn trapped_chests_follow_the_block() {
        let mut c = chest(0, 0);
        resolve_for_java(&mut c, &block("minecraft:trapped_chest", "north", "single"));
        let Kind::Container(cc) = &c.kind else { panic!() };
        assert_eq!(cc.kind, ContainerKind::TrappedChest);
    }
}
