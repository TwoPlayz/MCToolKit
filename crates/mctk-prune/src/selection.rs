//! Selection layers. Shapes use block coordinates and test each chunk's centre block
//! (16·x + 8, 16·z + 8).

use mctk_anvil::ChunkPos;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    Less,
    LessOrEqual,
    Equal,
    GreaterOrEqual,
    Greater,
}

impl Comparison {
    pub fn test(self, value: i64, threshold: i64) -> bool {
        match self {
            Comparison::Less => value < threshold,
            Comparison::LessOrEqual => value <= threshold,
            Comparison::Equal => value == threshold,
            Comparison::GreaterOrEqual => value >= threshold,
            Comparison::Greater => value > threshold,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Layer {
    /// Rectangle with one corner at (x, z), extending `width` along X and `height` along Z
    /// (negative sizes extend the other way).
    Box {
        x: f64,
        z: f64,
        width: f64,
        height: f64,
    },
    Circle {
        x: f64,
        z: f64,
        radius: f64,
    },
    /// Half of the world: the side to the right of a ray from (x, z) heading `angle`
    /// degrees (0° = +X/east, 90° = +Z/south), as seen on a north-up map.
    Split {
        x: f64,
        z: f64,
        angle: f64,
    },
    /// Polygon whose vertices snap to the centre of the chunk they fall in; chunks on
    /// its edges are inside.
    Polygon {
        points: Vec<(f64, f64)>,
    },
    /// Chunks whose InhabitedTime (ticks players spent nearby) passes the comparison.
    Time {
        comparison: Comparison,
        ticks: i64,
    },
}

fn snap(v: f64) -> f64 {
    (v / 16.0).floor() * 16.0 + 8.0
}

fn on_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> bool {
    let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    cross.abs() < 1e-9
        && p.0 >= a.0.min(b.0)
        && p.0 <= a.0.max(b.0)
        && p.1 >= a.1.min(b.1)
        && p.1 <= a.1.max(b.1)
}

fn in_polygon(p: (f64, f64), points: &[(f64, f64)]) -> bool {
    let n = points.len();
    if n == 0 {
        return false;
    }
    if (0..n).any(|i| on_segment(p, points[i], points[(i + 1) % n])) {
        return true;
    }
    // Even-odd ray cast towards +X.
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) * (b.0 - a.0) / (b.1 - a.1) {
            inside = !inside;
        }
    }
    inside
}

impl Layer {
    pub fn uses_inhabited_time(&self) -> bool {
        matches!(self, Layer::Time { .. })
    }

    /// Whether the layer selects the chunk. `inhabited` is the chunk's InhabitedTime,
    /// when known; the Time layer never selects a chunk without one.
    pub fn selects(&self, pos: ChunkPos, inhabited: Option<i64>) -> bool {
        let c = (pos.x as f64 * 16.0 + 8.0, pos.z as f64 * 16.0 + 8.0);
        match self {
            Layer::Box {
                x,
                z,
                width,
                height,
            } => {
                let (x0, x1) = (x.min(x + width), x.max(x + width));
                let (z0, z1) = (z.min(z + height), z.max(z + height));
                c.0 >= x0 && c.0 < x1 && c.1 >= z0 && c.1 < z1
            }
            Layer::Circle { x, z, radius } => (c.0 - x).hypot(c.1 - z) <= *radius,
            Layer::Split { x, z, angle } => {
                let (sin, cos) = angle.to_radians().sin_cos();
                cos * (c.1 - z) - sin * (c.0 - x) >= 0.0
            }
            Layer::Polygon { points } => {
                let snapped: Vec<_> = points.iter().map(|&(x, z)| (snap(x), snap(z))).collect();
                in_polygon(c, &snapped)
            }
            Layer::Time { comparison, ticks } => {
                inhabited.is_some_and(|t| comparison.test(t, *ticks))
            }
        }
    }
}

/// A union of layers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selection {
    pub layers: Vec<Layer>,
    /// Keep only the selected chunks instead of removing them.
    pub inverted: bool,
}

impl Selection {
    pub fn uses_inhabited_time(&self) -> bool {
        self.layers.iter().any(Layer::uses_inhabited_time)
    }

    pub fn selects(&self, pos: ChunkPos, inhabited: Option<i64>) -> bool {
        self.layers.iter().any(|l| l.selects(pos, inhabited))
    }

    /// Whether pruning keeps the chunk: normal mode removes the selection, inverted
    /// mode keeps only the selection (as UMT's Pruner does).
    pub fn keeps(&self, pos: ChunkPos, inhabited: Option<i64>) -> bool {
        self.selects(pos, inhabited) == self.inverted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i32, z: i32) -> ChunkPos {
        ChunkPos::new(x, z)
    }

    #[test]
    fn shapes() {
        let b = Layer::Box {
            x: 0.0,
            z: 0.0,
            width: 32.0,
            height: -16.0,
        };
        assert!(b.selects(p(0, -1), None) && b.selects(p(1, -1), None));
        assert!(!b.selects(p(2, -1), None) && !b.selects(p(0, 0), None));

        let c = Layer::Circle {
            x: 8.0,
            z: 8.0,
            radius: 16.0,
        };
        assert!(c.selects(p(0, 0), None) && c.selects(p(1, 0), None) && c.selects(p(0, -1), None));
        assert!(!c.selects(p(1, 1), None));

        // Ray heading east: the south side (+Z) is selected.
        let s = Layer::Split {
            x: 0.0,
            z: 0.0,
            angle: 0.0,
        };
        assert!(s.selects(p(5, 3), None) && !s.selects(p(5, -3), None));
        // Heading south: the west side is selected.
        let s = Layer::Split {
            x: 0.0,
            z: 0.0,
            angle: 90.0,
        };
        assert!(s.selects(p(-3, 5), None) && !s.selects(p(3, 5), None));
    }

    #[test]
    fn polygon_snaps_and_includes_edges() {
        let tri = Layer::Polygon {
            points: vec![(1.0, 1.0), (70.0, 3.0), (2.0, 75.0)],
        };
        // Snapped to (8,8), (72,8), (8,72): chunks (0..=4, 0) and (0, 0..=4) are edges.
        assert!(
            tri.selects(p(4, 0), None) && tri.selects(p(0, 4), None) && tri.selects(p(1, 1), None)
        );
        assert!(
            !tri.selects(p(3, 3), None)
                && !tri.selects(p(5, 0), None)
                && !tri.selects(p(-1, 0), None)
        );
    }

    #[test]
    fn time_and_inversion() {
        let mut sel = Selection {
            layers: vec![Layer::Time {
                comparison: Comparison::GreaterOrEqual,
                ticks: 1200,
            }],
            inverted: false,
        };
        // Normal mode removes what the layers select.
        assert!(!sel.keeps(p(0, 0), Some(1200)) && sel.keeps(p(0, 0), Some(5)));
        assert!(sel.keeps(p(0, 0), None));
        sel.inverted = true;
        assert!(sel.keeps(p(0, 0), Some(1200)) && !sel.keeps(p(0, 0), None));
    }
}
