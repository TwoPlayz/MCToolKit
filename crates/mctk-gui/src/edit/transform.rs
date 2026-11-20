//! Turning and mirroring blocks with what they hold: positions in a box, and the properties of
//! block states that point a way (facing, axis, rotation, connections, stair and rail shapes,
//! door hinges, chest halves).

use mctk_java::BlockState;

/// How a copied box is placed: turned clockwise (seen from above) a number of quarter turns,
/// after being mirrored east to west when `mirror` is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    pub turns: u8,
    pub mirror: bool,
}

const SIDES: [&str; 4] = ["north", "east", "south", "west"];

impl Placement {
    /// The size of a box of `size` once placed.
    pub fn size(self, size: [i32; 3]) -> [i32; 3] {
        if self.turns % 2 == 1 { [size[2], size[1], size[0]] } else { size }
    }

    /// Where a position in a box of `size` ends up in the placed box.
    pub fn pos(self, size: [i32; 3], p: [i32; 3]) -> [i32; 3] {
        let (mut x, y, mut z) = (p[0], p[1], p[2]);
        let (mut sx, mut sz) = (size[0], size[2]);
        if self.mirror {
            x = sx - 1 - x;
        }
        for _ in 0..self.turns % 4 {
            // Clockwise: north (−z) becomes east (+x).
            (x, z) = (sz - 1 - z, x);
            (sx, sz) = (sz, sx);
        }
        [x, y, z]
    }

    /// A side (`north`...) once placed.
    fn side(self, s: &str) -> String {
        let Some(mut i) = SIDES.iter().position(|d| *d == s) else { return s.to_string() };
        if self.mirror && (i == 1 || i == 3) {
            i = 4 - i;
        }
        SIDES[(i + self.turns as usize) % 4].to_string()
    }

    /// A word made of sides joined by `_` (rail shapes `north_east`, `ascending_west`, jigsaw
    /// `north_up`), each side placed.
    fn sides_in(self, s: &str) -> String {
        let parts: Vec<String> = s.split('_').map(|p| self.side(p)).collect();
        // Curved rails are named north/south first.
        if parts.len() == 2 && SIDES.contains(&parts[0].as_str()) && SIDES.contains(&parts[1].as_str()) {
            let (a, b) = (&parts[0], &parts[1]);
            let ns = |p: &str| p == "north" || p == "south";
            if !ns(a) && ns(b) {
                return format!("{b}_{a}");
            }
            if (a == "north" && b == "south") || (a == "east" && b == "west") {
                return format!("{a}_{b}");
            }
            if (a == "south" && b == "north") || (a == "west" && b == "east") {
                return format!("{b}_{a}");
            }
        }
        parts.join("_")
    }

    /// A block state once placed.
    pub fn state(self, s: &BlockState) -> BlockState {
        if self.turns % 4 == 0 && !self.mirror {
            return s.clone();
        }
        let mut out = s.clone();
        let swap_lr = |v: &str| match v {
            "left" => "right".to_string(),
            "right" => "left".to_string(),
            "inner_left" => "inner_right".to_string(),
            "inner_right" => "inner_left".to_string(),
            "outer_left" => "outer_right".to_string(),
            "outer_right" => "outer_left".to_string(),
            o => o.to_string(),
        };
        for (k, v) in &s.properties {
            let new = match k.as_str() {
                "facing" | "horizontal_facing" => self.side(v),
                "axis" if self.turns % 2 == 1 => match v.as_str() {
                    "x" => "z".into(),
                    "z" => "x".into(),
                    o => o.into(),
                },
                "rotation" => {
                    let mut r: i32 = v.parse().unwrap_or(0);
                    if self.mirror {
                        r = (16 - r) % 16;
                    }
                    ((r + 4 * self.turns as i32) % 16).to_string()
                }
                "shape" if v.contains("north") || v.contains("east") || v.contains("south") || v.contains("west") => self.sides_in(v),
                "shape" | "hinge" | "type" if self.mirror => swap_lr(v),
                "orientation" => self.sides_in(v),
                _ => continue,
            };
            out.properties.insert(k.clone(), new);
        }
        // Connections on each side (fences, walls, glass panes, redstone, vines...).
        if SIDES.iter().any(|d| s.properties.contains_key(*d)) {
            for d in SIDES {
                if let Some(v) = s.properties.get(d) {
                    out.properties.insert(self.side(d), v.clone());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_turns_and_two_mirrors_give_back_the_original() {
        let stair = BlockState::new("minecraft:oak_stairs").with("facing", "north").with("shape", "inner_left").with("half", "bottom");
        let fence = BlockState::new("minecraft:oak_fence").with("north", "true").with("east", "false").with("south", "false").with("west", "true");
        let rail = BlockState::new("minecraft:rail").with("shape", "north_east");
        let sign = BlockState::new("minecraft:oak_sign").with("rotation", "3");
        for s in [&stair, &fence, &rail, &sign] {
            let one = Placement { turns: 1, mirror: false };
            let mut t = s.clone();
            for _ in 0..4 {
                t = one.state(&t);
            }
            assert_eq!(&t, s, "four turns");
            let m = Placement { turns: 0, mirror: true };
            assert_eq!(&m.state(&m.state(s)), s, "two mirrors");
        }
        // One turn: north becomes east; a rail's north_east becomes south_east.
        let one = Placement { turns: 1, mirror: false };
        assert_eq!(one.state(&stair).properties["facing"], "east");
        assert_eq!(one.state(&rail).properties["shape"], "south_east");
        assert_eq!(one.state(&fence).properties["north"], "true", "west's connection is now north's");
        assert_eq!(one.state(&sign).properties["rotation"], "7");
        // Mirrored: east and west swap, left and right too.
        let m = Placement { turns: 0, mirror: true };
        let east = BlockState::new("minecraft:oak_stairs").with("facing", "east").with("shape", "inner_left");
        assert_eq!(m.state(&east).properties["facing"], "west");
        assert_eq!(m.state(&east).properties["shape"], "inner_right");
        // Positions: a 3 x 1 x 2 box turned once is 2 x 1 x 3; its corner (0,0,0) goes to (1,0,0).
        assert_eq!(one.size([3, 1, 2]), [2, 1, 3]);
        assert_eq!(one.pos([3, 1, 2], [0, 0, 0]), [1, 0, 0]);
        assert_eq!(one.pos([3, 1, 2], [2, 0, 1]), [0, 0, 2]);
    }
}
