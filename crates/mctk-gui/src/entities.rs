//! What the game draws in code rather than from block models: block entities (chests, shulker
//! boxes, skulls and heads, banners, decorated pots, bells, conduits, end portals), as the game
//! builds them from boxes laid out on an entity texture (`ModelPart.Cube`), placed as its
//! renderers place them.
//!
//! Each piece comes out as quads in the block's space (0 to 1) with its texture and the part of
//! it shown, for the GPU world's mesher.

use std::sync::Arc;

use mctk_nbt::Tag;

/// One face, in the block's space.
pub struct EQuad {
    /// Corners in the order the GPU world draws (clockwise as seen from outside).
    pub pos: [[f32; 3]; 4],
    /// Each corner's place in the texture (0 to 1).
    pub uv: [[f32; 2]; 4],
    pub tex: Arc<str>,
    pub tint: [f32; 3],
    /// How the game shades a face facing this way.
    pub shade: f32,
}

// ---------------------------------------------------------------------------------------------
// Transforms
// ---------------------------------------------------------------------------------------------

/// A 4 x 4 transform (rows), applied to column vectors.
#[derive(Clone, Copy)]
pub struct Mat([[f32; 4]; 4]);

impl Mat {
    pub const ID: Mat = Mat([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);

    pub fn mul(self, b: Mat) -> Mat {
        let mut r = [[0.0; 4]; 4];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..4).map(|k| self.0[i][k] * b.0[k][j]).sum();
            }
        }
        Mat(r)
    }
    pub fn t(self, x: f32, y: f32, z: f32) -> Mat {
        self.mul(Mat([[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, z], [0.0, 0.0, 0.0, 1.0]]))
    }
    pub fn s(self, x: f32, y: f32, z: f32) -> Mat {
        self.mul(Mat([[x, 0.0, 0.0, 0.0], [0.0, y, 0.0, 0.0], [0.0, 0.0, z, 0.0], [0.0, 0.0, 0.0, 1.0]]))
    }
    /// Turned by `a` radians about x, right-handed.
    pub fn rx(self, a: f32) -> Mat {
        let (s, c) = a.sin_cos();
        self.mul(Mat([[1.0, 0.0, 0.0, 0.0], [0.0, c, -s, 0.0], [0.0, s, c, 0.0], [0.0, 0.0, 0.0, 1.0]]))
    }
    pub fn ry(self, a: f32) -> Mat {
        let (s, c) = a.sin_cos();
        self.mul(Mat([[c, 0.0, s, 0.0], [0.0, 1.0, 0.0, 0.0], [-s, 0.0, c, 0.0], [0.0, 0.0, 0.0, 1.0]]))
    }
    pub fn rz(self, a: f32) -> Mat {
        let (s, c) = a.sin_cos();
        self.mul(Mat([[c, -s, 0.0, 0.0], [s, c, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]))
    }
    /// A model part's pose, as the game applies it: moved to its pivot, then turned about z,
    /// y and x (in that order outward).
    pub fn pose(self, pivot: [f32; 3], rot: [f32; 3]) -> Mat {
        self.t(pivot[0], pivot[1], pivot[2]).rz(rot[2]).ry(rot[1]).rx(rot[0])
    }
    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.0;
        [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3])
    }
}

const DEG: f32 = std::f32::consts::PI / 180.0;

// ---------------------------------------------------------------------------------------------
// Boxes on an entity texture
// ---------------------------------------------------------------------------------------------

/// An entity texture: its file and size in pixels.
#[derive(Clone)]
pub struct Tex {
    pub file: Arc<str>,
    pub w: f32,
    pub h: f32,
}

impl Tex {
    pub fn new(file: &str, w: f32, h: f32) -> Tex {
        Tex { file: format!("{file}.png").into(), w, h }
    }
}

/// Builds quads from boxes, as the game's model parts do.
pub struct Builder {
    pub out: Vec<EQuad>,
    pub tint: [f32; 3],
}

impl Builder {
    pub fn new() -> Self {
        Builder { out: Vec::new(), tint: [1.0; 3] }
    }

    /// A box at `o` sized `d` (model pixels) under transform `m`, its faces laid out on `tex`
    /// from (`u`, `v`) as the game lays a box's faces out; grown by `g` all round, mirrored
    /// across x with `mirror`.
    #[allow(clippy::too_many_arguments)]
    pub fn cube(&mut self, m: &Mat, tex: &Tex, u: f32, v: f32, o: [f32; 3], d: [f32; 3], g: f32, mirror: bool) {
        let (mut x0, y0, z0) = (o[0] - g, o[1] - g, o[2] - g);
        let (mut x1, y1, z1) = (o[0] + d[0] + g, o[1] + d[1] + g, o[2] + d[2] + g);
        if mirror {
            std::mem::swap(&mut x0, &mut x1);
        }
        // The game's names for the corners.
        let v7 = [x0, y0, z0];
        let v0 = [x1, y0, z0];
        let v1 = [x1, y1, z0];
        let v2 = [x0, y1, z0];
        let v3 = [x0, y0, z1];
        let v4 = [x1, y0, z1];
        let v5 = [x1, y1, z1];
        let v6 = [x0, y1, z1];
        let (dx, dy, dz) = (d[0], d[1], d[2]);
        let (f4, f5, f6, f7, f8, f9) = (u, u + dz, u + dz + dx, u + dz + dx + dx, u + dz + dx + dz, u + dz + dx + dz + dx);
        let (f10, f11, f12) = (v, v + dz, v + dz + dy);
        let faces: [([[f32; 3]; 4], f32, f32, f32, f32); 6] = [
            ([v4, v3, v7, v0], f5, f10, f6, f11),
            ([v1, v2, v6, v5], f6, f11, f7, f10),
            ([v7, v3, v6, v2], f4, f11, f5, f12),
            ([v0, v7, v2, v1], f5, f11, f6, f12),
            ([v4, v0, v1, v5], f6, f11, f8, f12),
            ([v3, v4, v5, v6], f8, f11, f9, f12),
        ];
        // A flat box (a fin, an ear, a wing): the game draws entities without culling, so each
        // side is seen from behind too.
        let flat = (0..3).any(|k| (d[k] + 2.0 * g).abs() < 1e-6);
        for (verts, u1, va, u2, vb) in faces {
            let uvs = [[u2, va], [u1, va], [u1, vb], [u2, vb]].map(|[a, b]| [a / tex.w, b / tex.h]);
            self.face_sided(m, tex, verts, uvs, mirror, false);
            if flat {
                self.face_sided(m, tex, verts, uvs, mirror, true);
            }
        }
    }

    /// One face given as the game gives it (counter-clockwise from outside, before `mirror`).
    fn face(&mut self, m: &Mat, tex: &Tex, verts: [[f32; 3]; 4], uvs: [[f32; 2]; 4], mirror: bool) {
        self.face_sided(m, tex, verts, uvs, mirror, false);
    }

    /// As [`Builder::face`]; with `back`, the face as seen from behind, a hair behind the face
    /// on the other side of a flat box (so that one shows where both are drawn).
    fn face_sided(&mut self, m: &Mat, tex: &Tex, verts: [[f32; 3]; 4], uvs: [[f32; 2]; 4], mirror: bool, back: bool) {
        let p = verts.map(|c| m.apply(c));
        // Flat (a zero-thickness side): nothing to see.
        let a = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
        let b = [p[3][0] - p[0][0], p[3][1] - p[0][1], p[3][2] - p[0][2]];
        let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len < 1e-9 {
            return;
        }
        let n = n.map(|v| v / len);
        // The game's order is counter-clockwise; the GPU world's clockwise (a mirrored box is
        // already reversed).
        let order = if mirror != back { [0, 1, 2, 3] } else { [3, 2, 1, 0] };
        let n = if mirror { n.map(|v| -v) } else { n };
        // Seen from behind: pushed out along its own front (away from whoever sees its back).
        let (p, n) = if back {
            const HAIR: f32 = 0.002;
            (p.map(|c| [c[0] + n[0] * HAIR, c[1] + n[1] * HAIR, c[2] + n[2] * HAIR]), n.map(|v| -v))
        } else {
            (p, n)
        };
        let shade = (n[0] * n[0] * 0.6 + n[1] * n[1] * ((3.0 + n[1]) / 4.0) + n[2] * n[2] * 0.8).min(1.0);
        self.out.push(EQuad { pos: order.map(|i| p[i]), uv: order.map(|i| uvs[i]), tex: tex.file.clone(), tint: self.tint, shade });
    }

    /// One face given in the GPU world's order (clockwise as seen from outside), with each
    /// corner's place in the texture (0 to 1), under transform `m`.
    pub fn quad(&mut self, m: &Mat, tex: &Arc<str>, corners: [[f32; 3]; 4], uvs: [[f32; 2]; 4]) {
        let p = corners.map(|c| m.apply(c));
        let a = [p[3][0] - p[0][0], p[3][1] - p[0][1], p[3][2] - p[0][2]];
        let b = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
        let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len < 1e-12 {
            return;
        }
        let n = n.map(|v| v / len);
        let shade = (n[0] * n[0] * 0.6 + n[1] * n[1] * ((3.0 + n[1]) / 4.0) + n[2] * n[2] * 0.8).min(1.0);
        self.out.push(EQuad { pos: p, uv: uvs, tex: tex.clone(), tint: self.tint, shade });
    }

    /// A block model's faces (in its block's space, 0 to 1) under transform `m`, tinted faces
    /// in `tint`.
    pub fn model(&mut self, m: &Mat, model: &crate::models::Model, tint: [f32; 3]) {
        let keep = self.tint;
        for q in &model.quads {
            self.tint = if q.tint { tint } else { keep };
            self.quad(m, &q.tex, q.pos, q.uv);
        }
        self.tint = keep;
    }

    /// A flat item picture in the unit cube (facing south, a pixel thick at its middle), pushed
    /// out as the game does: its front and back, and a side wherever a solid pixel meets a
    /// clear one.
    pub fn flat(&mut self, m: &Mat, tex: &Arc<str>) {
        let (z0, z1) = (7.5 / 16.0, 8.5 / 16.0);
        self.quad(m, tex, [[0.0, 1.0, z1], [1.0, 1.0, z1], [1.0, 0.0, z1], [0.0, 0.0, z1]], [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        self.quad(m, tex, [[1.0, 1.0, z0], [0.0, 1.0, z0], [0.0, 0.0, z0], [1.0, 0.0, z0]], [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]]);
        let Some(mask) = crate::models::sprite_mask(tex) else { return };
        let (w, h, solid) = (mask.0, mask.1, &mask.2);
        let at = |i: i32, j: i32| i >= 0 && j >= 0 && (i as usize) < w && (j as usize) < h && solid[j as usize * w + i as usize];
        for j in 0..h as i32 {
            for i in 0..w as i32 {
                if !at(i, j) {
                    continue;
                }
                let (x0, x1) = (i as f32 / w as f32, (i + 1) as f32 / w as f32);
                let (y0, y1) = (1.0 - (j + 1) as f32 / h as f32, 1.0 - j as f32 / h as f32);
                // The side takes the pixel's own color.
                let (u0, u1) = ((i as f32 + 0.3) / w as f32, (i as f32 + 0.7) / w as f32);
                let (v0, v1) = ((j as f32 + 0.3) / h as f32, (j as f32 + 0.7) / h as f32);
                let uv = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
                if !at(i, j - 1) {
                    self.quad(m, tex, [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]], uv);
                }
                if !at(i, j + 1) {
                    self.quad(m, tex, [[x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0]], uv);
                }
                if !at(i - 1, j) {
                    self.quad(m, tex, [[x0, y1, z0], [x0, y1, z1], [x0, y0, z1], [x0, y0, z0]], uv);
                }
                if !at(i + 1, j) {
                    self.quad(m, tex, [[x1, y1, z1], [x1, y1, z0], [x1, y0, z0], [x1, y0, z1]], uv);
                }
            }
        }
    }

    /// An item (by id) in the unit cube under `m`: its block model, or its pictures pushed out.
    /// Whether it is block-shaped (for sizing it as the game does) comes back.
    pub fn item(&mut self, m: &Mat, id: &str, tint: [f32; 3]) -> bool {
        match crate::models::item_model(id).as_deref() {
            Some(crate::models::ItemModel::Model(model)) => {
                self.model(m, model, tint);
                true
            }
            Some(crate::models::ItemModel::Flat(layers)) => {
                for (k, l) in layers.iter().enumerate() {
                    // Each layer a hair in front of the one under it.
                    let lm = m.t(0.5, 0.5, 0.5).s(1.0, 1.0, 1.0 + k as f32 * 0.02).t(-0.5, -0.5, -0.5);
                    self.flat(&lm, l);
                }
                false
            }
            None => false,
        }
    }

    /// A flat rectangle (both sides seen): `corners` clockwise as seen from the front, with the
    /// whole of `uv` (u0, v0, u1, v1 in texture pixels) laid over it.
    pub fn plane(&mut self, m: &Mat, tex: &Tex, corners: [[f32; 3]; 4], uv: [f32; 4]) {
        let uvs = [[uv[0], uv[1]], [uv[2], uv[1]], [uv[2], uv[3]], [uv[0], uv[3]]].map(|[a, b]| [a / tex.w, b / tex.h]);
        // `face` takes the game's counter-clockwise order: given reversed, then mirrored for the back.
        let ccw = [corners[3], corners[2], corners[1], corners[0]];
        let ccw_uv = [uvs[3], uvs[2], uvs[1], uvs[0]];
        self.face(m, tex, ccw, ccw_uv, false);
        self.face(m, tex, ccw, ccw_uv, true);
    }
}

// ---------------------------------------------------------------------------------------------
// Block entities
// ---------------------------------------------------------------------------------------------

/// A block state's property.
fn prop<'a>(props: &'a str, key: &str) -> Option<&'a str> {
    props.split(',').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

/// How far round a horizontal facing is from south, as the game's `toYRot` (degrees).
fn facing_yrot(f: &str) -> f32 {
    match f {
        "west" => 90.0,
        "north" => 180.0,
        "east" => 270.0,
        _ => 0.0,
    }
}

/// The colors dyes give (as the game tints banners and the like).
pub fn dye(name: &str) -> [f32; 3] {
    let c: u32 = match name {
        "white" => 0xF9FFFE,
        "orange" => 0xF9801D,
        "magenta" => 0xC74EBD,
        "light_blue" => 0x3AB3DA,
        "yellow" => 0xFED83D,
        "lime" => 0x80C71F,
        "pink" => 0xF38BAA,
        "gray" => 0x474F52,
        "light_gray" => 0x9D9D97,
        "cyan" => 0x169C9C,
        "purple" => 0x8932B8,
        "blue" => 0x3C44AA,
        "brown" => 0x835432,
        "green" => 0x5E7C16,
        "red" => 0xB02E26,
        "black" => 0x1D1D21,
        _ => 0xF9FFFE,
    };
    [(c >> 16) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0]
}

const DYES: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];

/// Whether the game draws a block (by its bare id) in code, as here.
pub fn is_block_entity(block: &str) -> bool {
    matches!(block, "chest" | "trapped_chest" | "ender_chest" | "decorated_pot" | "conduit" | "end_portal" | "end_gateway" | "bell")
        || block.contains("copper_chest")
        || block.ends_with("shulker_box")
        || block.ends_with("_skull")
        || block.ends_with("_head") && !block.ends_with("piston_head")
        || block.ends_with("_banner")
        || block.ends_with("copper_golem_statue")
}

/// The quads for a block drawn in code, by its bare id and state, with its saved data (`extra`)
/// where it has any. `None` for a block drawn otherwise.
pub fn block_entity(block: &str, props: &str, extra: Option<&Tag>) -> Option<Vec<EQuad>> {
    if !is_block_entity(block) {
        return None;
    }
    let mut b = Builder::new();
    let one = Mat::ID.s(1.0 / 16.0, 1.0 / 16.0, 1.0 / 16.0);
    if block == "chest" || block == "trapped_chest" || block == "ender_chest" || block.contains("copper_chest") {
        chest(&mut b, block, props);
    } else if block.ends_with("shulker_box") {
        let color = block.strip_suffix("_shulker_box").filter(|c| DYES.contains(c));
        let tex = Tex::new(&match color {
            Some(c) => format!("entity/shulker/shulker_{c}"),
            None => "entity/shulker/shulker".to_string(),
        }, 64.0, 64.0);
        let half = std::f32::consts::FRAC_PI_2;
        let pi = std::f32::consts::PI;
        let m = Mat::ID.t(0.5, 0.5, 0.5).s(0.9995, 0.9995, 0.9995);
        let m = match prop(props, "facing").unwrap_or("up") {
            "down" => m.rx(pi),
            "north" => m.rx(half).rz(pi),
            "south" => m.rx(half),
            "west" => m.rx(half).rz(half),
            "east" => m.rx(half).rz(-half),
            _ => m,
        };
        let m = m.s(1.0, -1.0, -1.0).t(0.0, -1.0, 0.0).s(1.0 / 16.0, 1.0 / 16.0, 1.0 / 16.0).t(0.0, 24.0, 0.0);
        b.cube(&m, &tex, 0.0, 28.0, [-8.0, -8.0, -8.0], [16.0, 8.0, 16.0], 0.0, false);
        b.cube(&m, &tex, 0.0, 0.0, [-8.0, -16.0, -8.0], [16.0, 12.0, 16.0], 0.0, false);
    } else if block.ends_with("_skull") || block.ends_with("_head") {
        skull(&mut b, block, props);
    } else if block.ends_with("_banner") {
        banner(&mut b, block, props, extra);
    } else if block.ends_with("copper_golem_statue") {
        copper_golem_statue(&mut b, block, props);
    } else if block == "decorated_pot" {
        pot(&mut b, props, extra);
    } else if block == "bell" {
        let tex = Tex::new("entity/bell/bell_body", 32.0, 32.0);
        b.cube(&one, &tex, 0.0, 0.0, [5.0, 6.0, 5.0], [6.0, 7.0, 6.0], 0.0, false);
        b.cube(&one, &tex, 0.0, 13.0, [4.0, 4.0, 4.0], [8.0, 2.0, 8.0], 0.0, false);
    } else if block == "conduit" {
        let tex = Tex::new("entity/conduit/base", 32.0, 16.0);
        b.cube(&one, &tex, 0.0, 0.0, [5.0, 5.0, 5.0], [6.0, 6.0, 6.0], 0.0, false);
    } else if block == "end_portal" || block == "end_gateway" {
        // The starry surface (the game's shader scrolls layers of it; here, its picture).
        let tex = Tex::new("entity/end_portal", 256.0, 256.0);
        let top = if block == "end_portal" { 12.0 } else { 16.0 };
        b.tint = [0.25, 0.3, 0.35];
        b.plane(&one, &tex, [[0.0, top, 0.0], [16.0, top, 0.0], [16.0, top, 16.0], [0.0, top, 16.0]], [0.0, 0.0, 64.0, 64.0]);
        if block == "end_gateway" {
            b.cube(&one, &tex, 0.0, 0.0, [0.0, 0.0, 0.0], [16.0, 16.0, 16.0], 0.0, false);
        }
    }
    Some(b.out)
}

/// A copper golem statue: the copper golem's model (its texture by how weathered the block is;
/// waxed looks the same) facing the way the block does, in one of its four poses.
fn copper_golem_statue(b: &mut Builder, block: &str, props: &str) {
    copper_golem_at(b, block, props, Mat::ID);
}

/// The copper golem's model, as [`copper_golem_statue`] places it in a block, under `base` (the
/// mob uses it where it stands).
pub(crate) fn copper_golem_at(b: &mut Builder, block: &str, props: &str, base: Mat) {
    let age = block.trim_start_matches("waxed_").strip_suffix("copper_golem_statue").unwrap_or("").trim_end_matches('_');
    let tex = Tex::new(&match age {
        "exposed" | "weathered" | "oxidized" => format!("entity/copper_golem/copper_golem_{age}"),
        _ => "entity/copper_golem/copper_golem".to_string(),
    }, 64.0, 64.0);
    let yaw = match prop(props, "facing").unwrap_or("north") {
        "south" => 0.0,
        "west" => 90.0,
        "north" => 180.0,
        _ => 270.0,
    };
    // As the game places an entity's model: its feet (y 24 in model pixels, y down) on the
    // block's floor, turned to face the way the block does.
    let root = base.t(0.5, 0.0, 0.5).ry((180.0 - yaw) * DEG).s(-1.0, -1.0, 1.0).t(0.0, -1.501, 0.0).s(1.0 / 16.0, 1.0 / 16.0, 1.0 / 16.0);
    let half = std::f32::consts::FRAC_PI_2;
    // Each part's turn (x, y, z) by pose: body, right arm, left arm, right leg, left leg; and how
    // far the body sits down (sitting, on its legs stretched out in front).
    let (body, ra, la, rl, ll, sit) = match prop(props, "copper_golem_pose").unwrap_or("standing") {
        "running" => ([0.15, 0.0, 0.0], [-0.9, 0.0, 0.0], [0.9, 0.0, 0.0], [0.7, 0.0, 0.0], [-0.7, 0.0, 0.0], 0.0),
        "star" => ([0.0; 3], [0.0, 0.0, 2.4], [0.0, 0.0, -2.4], [0.0, 0.0, 0.3], [0.0, 0.0, -0.3], 0.0),
        "sitting" => ([0.0; 3], [-0.5, 0.0, 0.1], [-0.5, 0.0, -0.1], [-half, 0.2, 0.0], [-half, -0.2, 0.0], 3.0),
        _ => ([0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], 0.0),
    };
    let body_m = root.pose([0.0, 19.0 + sit, 0.0], body);
    b.cube(&body_m, &tex, 0.0, 15.0, [-4.0, -6.0, -3.0], [8.0, 6.0, 6.0], 0.0, false);
    // The head, with its nose and the antenna on top.
    let head = body_m.pose([0.0, -6.0, 0.0], [0.0; 3]);
    b.cube(&head, &tex, 0.0, 0.0, [-4.0, -5.0, -5.0], [8.0, 5.0, 10.0], 0.0, false);
    b.cube(&head, &tex, 56.0, 0.0, [-1.0, -2.0, -6.0], [2.0, 3.0, 2.0], 0.0, false);
    b.cube(&head, &tex, 37.0, 8.0, [-1.0, -9.0, -1.0], [2.0, 4.0, 2.0], 0.0, false);
    b.cube(&head, &tex, 37.0, 0.0, [-2.0, -13.0, -2.0], [4.0, 4.0, 4.0], 0.0, false);
    b.cube(&body_m.pose([-4.0, -5.0, 0.0], ra), &tex, 36.0, 16.0, [-3.0, -1.0, -2.0], [3.0, 10.0, 4.0], 0.0, false);
    b.cube(&body_m.pose([4.0, -5.0, 0.0], la), &tex, 50.0, 16.0, [0.0, -1.0, -2.0], [3.0, 10.0, 4.0], 0.0, false);
    b.cube(&root.pose([0.0, 19.0 + sit, 0.0], rl), &tex, 0.0, 27.0, [-4.0, 0.0, -2.0], [4.0, 5.0, 4.0], 0.0, false);
    b.cube(&root.pose([0.0, 19.0 + sit, 0.0], ll), &tex, 16.0, 27.0, [0.0, 0.0, -2.0], [4.0, 5.0, 4.0], 0.0, false);
}

fn chest(b: &mut Builder, block: &str, props: &str) {
    let kind = match block {
        "trapped_chest" => "trapped",
        "ender_chest" => "ender",
        _ if block.contains("copper_chest") => {
            let b = block.trim_start_matches("waxed_");
            match b.strip_suffix("_copper_chest") {
                Some(age) => match age {
                    "exposed" => "copper_exposed",
                    "weathered" => "copper_weathered",
                    "oxidized" => "copper_oxidized",
                    _ => "copper",
                },
                None => "copper",
            }
        }
        _ => "normal",
    };
    // Which half of a double chest: the other half is clockwise of its facing for "left",
    // anticlockwise for "right" — in the chest's own frame (facing south), west or east.
    let half = if kind == "ender" { "single" } else { prop(props, "type").unwrap_or("single") };
    let tex = Tex::new(&match half {
        "left" => format!("entity/chest/{kind}_left"),
        "right" => format!("entity/chest/{kind}_right"),
        _ => format!("entity/chest/{kind}"),
    }, 64.0, 64.0);
    let yrot = facing_yrot(prop(props, "facing").unwrap_or("north"));
    let m = Mat::ID.t(0.5, 0.5, 0.5).ry(-yrot * DEG).t(-0.5, -0.5, -0.5).s(1.0 / 16.0, 1.0 / 16.0, 1.0 / 16.0);
    let (x0, w, lock_x, lock_w) = match half {
        // The other half to the west: this half runs to its west side.
        "left" => (0.0, 15.0, 0.0, 1.0),
        "right" => (1.0, 15.0, 15.0, 1.0),
        _ => (1.0, 14.0, 7.0, 2.0),
    };
    b.cube(&m, &tex, 0.0, 19.0, [x0, 0.0, 1.0], [w, 10.0, 14.0], 0.0, false);
    b.cube(&m.t(0.0, 9.0, 1.0), &tex, 0.0, 0.0, [x0, 0.0, 0.0], [w, 5.0, 14.0], 0.0, false);
    // The lock hangs on the lid (the same pivot), standing a pixel out from its front.
    b.cube(&m.t(0.0, 9.0, 1.0), &tex, 0.0, 0.0, [lock_x, -2.0, 14.0], [lock_w, 4.0, 1.0], 0.0, false);
}

fn skull(b: &mut Builder, block: &str, props: &str) {
    let wall = block.contains("_wall_");
    let kind = block.replace("_wall", "");
    let (file, w, h, hat) = match kind.as_str() {
        "skeleton_skull" => ("entity/skeleton/skeleton", 64.0, 32.0, false),
        "wither_skeleton_skull" => ("entity/skeleton/wither_skeleton", 64.0, 32.0, false),
        "zombie_head" => ("entity/zombie/zombie", 64.0, 64.0, true),
        "creeper_head" => ("entity/creeper/creeper", 64.0, 32.0, false),
        "piglin_head" => ("entity/piglin/piglin", 64.0, 64.0, false),
        "dragon_head" => ("entity/enderdragon/dragon", 256.0, 256.0, false),
        _ => ("entity/player/wide/steve", 64.0, 64.0, true),
    };
    let tex = Tex::new(file, w, h);
    let yrot = if wall {
        let d2 = match prop(props, "facing").unwrap_or("north") {
            "south" => 0.0,
            "west" => 1.0,
            "north" => 2.0,
            _ => 3.0,
        };
        90.0 * (2.0 + d2)
    } else {
        prop(props, "rotation").and_then(|r| r.parse::<f32>().ok()).unwrap_or(0.0) * 22.5
    };
    let base = if wall {
        let (sx, sz) = match prop(props, "facing").unwrap_or("north") {
            "south" => (0.0, 1.0),
            "west" => (-1.0, 0.0),
            "north" => (0.0, -1.0),
            _ => (1.0, 0.0),
        };
        Mat::ID.t(0.5 - sx * 0.25, 0.25, 0.5 - sz * 0.25)
    } else {
        Mat::ID.t(0.5, 0.0, 0.5)
    };
    let m = base.s(-1.0, -1.0, 1.0).s(1.0 / 16.0, 1.0 / 16.0, 1.0 / 16.0).ry(yrot * DEG);
    match kind.as_str() {
        "piglin_head" => {
            b.cube(&m, &tex, 0.0, 0.0, [-5.0, -8.0, -4.0], [10.0, 8.0, 8.0], 0.0, false);
            b.cube(&m, &tex, 31.0, 1.0, [-2.0, -4.0, -5.0], [4.0, 4.0, 1.0], 0.0, false);
            b.cube(&m, &tex, 2.0, 4.0, [2.0, -2.0, -5.0], [1.0, 2.0, 1.0], 0.0, false);
            b.cube(&m, &tex, 2.0, 0.0, [-3.0, -2.0, -5.0], [1.0, 2.0, 1.0], 0.0, false);
            // Ears, hanging out at the sides.
            b.cube(&m.pose([4.5, -6.0, 0.0], [0.0, 0.0, -std::f32::consts::FRAC_PI_6]), &tex, 51.0, 6.0, [0.0, 0.0, -2.0], [1.0, 5.0, 4.0], 0.0, false);
            b.cube(&m.pose([-4.5, -6.0, 0.0], [0.0, 0.0, std::f32::consts::FRAC_PI_6]), &tex, 39.0, 6.0, [-1.0, 0.0, -2.0], [1.0, 5.0, 4.0], 0.0, false);
        }
        "dragon_head" => {
            // The dragon's head: its skull and snout (the jaw closed).
            let m = m.t(0.0, -4.0, 2.0).s(0.75, 0.75, 0.75);
            b.cube(&m, &tex, 112.0, 30.0, [-8.0, -8.0, -10.0], [16.0, 16.0, 16.0], 0.0, false);
            b.cube(&m, &tex, 176.0, 44.0, [-6.0, -1.0, -24.0], [12.0, 5.0, 16.0], 0.0, false);
            b.cube(&m, &tex, 176.0, 65.0, [-6.0, 4.0, -24.0], [12.0, 4.0, 16.0], 0.0, false);
        }
        _ => {
            b.cube(&m, &tex, 0.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], 0.0, false);
            if hat {
                b.cube(&m, &tex, 32.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], 0.25, false);
            }
        }
    }
}

/// A banner's pattern layers from its saved data: (texture name, dye), base first.
fn banner_layers(base: &str, extra: Option<&Tag>) -> Vec<(String, String)> {
    let mut out = vec![("base".to_string(), base.to_string())];
    let Some(e) = extra else { return out };
    // 1.20.5 and later: `patterns: [{pattern: "minecraft:stripe_top", color: "red"}]`.
    if let Some(list) = e.get("patterns").and_then(Tag::as_list) {
        for p in list.iter() {
            let pattern = p.get("pattern").and_then(Tag::as_str).or_else(|| p.get("pattern").and_then(|t| t.get("asset_id")).and_then(Tag::as_str));
            let color = p.get("color").and_then(Tag::as_str);
            if let (Some(pat), Some(c)) = (pattern, color) {
                out.push((pat.rsplit(':').next().unwrap_or(pat).rsplit('/').next().unwrap_or(pat).to_string(), c.to_string()));
            }
        }
    } else if let Some(list) = e.get("Patterns").and_then(Tag::as_list) {
        // Before: `Patterns: [{Pattern: "ts", Color: 14}]`, short codes.
        for p in list.iter() {
            let code = p.get("Pattern").and_then(Tag::as_str).unwrap_or("");
            let color = p.get("Color").and_then(Tag::as_i64).unwrap_or(0).clamp(0, 15) as usize;
            let name = match code {
                "bs" => "stripe_bottom", "ts" => "stripe_top", "ls" => "stripe_left", "rs" => "stripe_right", "cs" => "stripe_center",
                "ms" => "stripe_middle", "drs" => "stripe_downright", "dls" => "stripe_downleft", "ss" => "small_stripes", "cr" => "cross",
                "sc" => "straight_cross", "bt" => "triangle_bottom", "tt" => "triangle_top", "bts" => "triangles_bottom", "tts" => "triangles_top",
                "ld" => "diagonal_left", "rd" => "diagonal_up_right", "lud" => "diagonal_up_left", "rud" => "diagonal_right", "mc" => "circle",
                "mr" => "rhombus", "vh" => "half_vertical", "hh" => "half_horizontal", "vhr" => "half_vertical_right", "hhb" => "half_horizontal_bottom",
                "bo" => "border", "cbo" => "curly_border", "gra" => "gradient", "gru" => "gradient_up", "bri" => "bricks", "glb" => "globe",
                "cre" => "creeper", "sku" => "skull", "flo" => "flower", "moj" => "mojang", "pig" => "piglin",
                "bl" => "square_bottom_left", "br" => "square_bottom_right", "tl" => "square_top_left", "tr" => "square_top_right",
                _ => continue,
            };
            out.push((name.to_string(), DYES[color].to_string()));
        }
    }
    out
}

fn banner(b: &mut Builder, block: &str, props: &str, extra: Option<&Tag>) {
    let wall = block.ends_with("_wall_banner");
    let color = block.strip_suffix("_wall_banner").or_else(|| block.strip_suffix("_banner")).unwrap_or("white");
    let m = if wall {
        let yrot = facing_yrot(prop(props, "facing").unwrap_or("north"));
        Mat::ID.t(0.5, -0.16666667, 0.5).ry(-yrot * DEG).t(0.0, -0.3125, -0.4375)
    } else {
        let rot = prop(props, "rotation").and_then(|r| r.parse::<f32>().ok()).unwrap_or(0.0);
        Mat::ID.t(0.5, 0.5, 0.5).ry(-rot * 22.5 * DEG)
    };
    let m = m.s(0.6666667, -0.6666667, -0.6666667).s(1.0 / 16.0, 1.0 / 16.0, 1.0 / 16.0);
    let wood = Tex::new("entity/banner/banner_base", 64.0, 64.0);
    if !wall {
        b.cube(&m, &wood, 44.0, 0.0, [-1.0, -30.0, -1.0], [2.0, 42.0, 2.0], 0.0, false);
    }
    b.cube(&m, &wood, 0.0, 42.0, [-10.0, -32.0, -1.0], [20.0, 2.0, 2.0], 0.0, false);
    // The cloth: each pattern a layer over the last, in its dye's color.
    for (i, (pattern, color)) in banner_layers(color, extra).iter().enumerate() {
        let tex = Tex::new(&format!("entity/banner/{pattern}"), 64.0, 64.0);
        b.tint = dye(color);
        b.cube(&m.t(0.0, -32.0, 0.0), &tex, 0.0, 0.0, [-10.0, 0.0, -2.0], [20.0, 40.0, 1.0], i as f32 * 0.01, false);
    }
    b.tint = [1.0; 3];
}

fn pot(b: &mut Builder, props: &str, extra: Option<&Tag>) {
    let yrot = facing_yrot(prop(props, "facing").unwrap_or("north"));
    let m = Mat::ID.t(0.5, 0.0, 0.5).ry(-yrot * DEG).t(-0.5, 0.0, -0.5).s(1.0 / 16.0, 1.0 / 16.0, 1.0 / 16.0);
    let base = Tex::new("entity/decorated_pot/decorated_pot_base", 32.0, 32.0);
    // Neck and rim.
    b.cube(&m, &base, 0.0, 0.0, [4.0, 17.0, 4.0], [8.0, 3.0, 8.0], 0.0, false);
    b.cube(&m, &base, 0.0, 5.0, [5.0, 16.0, 5.0], [6.0, 1.0, 6.0], 0.0, false);
    // Top and bottom.
    b.plane(&m, &base, [[1.0, 16.0, 1.0], [15.0, 16.0, 1.0], [15.0, 16.0, 15.0], [1.0, 16.0, 15.0]], [0.0, 13.0, 14.0, 27.0]);
    b.plane(&m, &base, [[1.0, 0.0, 15.0], [15.0, 0.0, 15.0], [15.0, 0.0, 1.0], [1.0, 0.0, 1.0]], [0.0, 13.0, 14.0, 27.0]);
    // The four sides: each the plain side, or the pattern of the sherd set in it (saved as
    // back, left, right, front).
    let sherds: Vec<String> = extra
        .and_then(|e| e.get("sherds").and_then(Tag::as_list))
        .map(|l| l.iter().filter_map(Tag::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let side_tex = |k: usize| -> Tex {
        let pattern = sherds.get(k).map(|s| s.rsplit(':').next().unwrap_or(s).to_string()).and_then(|s| s.strip_suffix("_pottery_sherd").map(|p| format!("entity/decorated_pot/{p}_pottery_pattern")));
        Tex::new(&pattern.unwrap_or_else(|| "entity/decorated_pot/decorated_pot_side".to_string()), 16.0, 16.0)
    };
    // (Corners clockwise as seen from outside: top-left, top-right, bottom-right, bottom-left.)
    let sides = [
        (0, [[15.0, 16.0, 1.0], [1.0, 16.0, 1.0], [1.0, 0.0, 1.0], [15.0, 0.0, 1.0]]),
        (1, [[1.0, 16.0, 1.0], [1.0, 16.0, 15.0], [1.0, 0.0, 15.0], [1.0, 0.0, 1.0]]),
        (2, [[15.0, 16.0, 15.0], [15.0, 16.0, 1.0], [15.0, 0.0, 1.0], [15.0, 0.0, 15.0]]),
        (3, [[1.0, 16.0, 15.0], [15.0, 16.0, 15.0], [15.0, 0.0, 15.0], [1.0, 0.0, 15.0]]),
    ];
    for (k, c) in sides {
        let tex = side_tex(k);
        b.plane(&m, &tex, c, [1.0, 0.0, 15.0, 16.0]);
    }
}

// ---------------------------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------------------------

/// A number list in an entity's data (`Pos`, `Rotation`, a pose angle).
fn nums(t: Option<&Tag>) -> Vec<f32> {
    t.and_then(Tag::as_list).map(|l| l.iter().filter_map(|v| v.as_f64().map(|f| f as f32)).collect()).unwrap_or_default()
}

/// A block position saved as `TileX/TileY/TileZ` (older) or `block_pos: [x, y, z]`.
fn block_pos(e: &Tag) -> Option<[i32; 3]> {
    if let Some(Tag::IntArray(a)) = e.get("block_pos") {
        if a.len() == 3 {
            return Some([a[0], a[1], a[2]]);
        }
    }
    let n = |k: &str| e.get(k).and_then(Tag::as_i64).map(|v| v as i32);
    Some([n("TileX")?, n("TileY")?, n("TileZ")?])
}

/// An item stack's id (`minecraft:diamond_sword` as `diamond_sword`), if it holds anything.
fn stack_id(s: Option<&Tag>) -> Option<String> {
    let id = s?.get("id")?.as_str()?.trim_start_matches("minecraft:").to_string();
    (id != "air").then_some(id)
}

/// A leather piece's dye (its `dyed_color`, as a number or `{rgb: n}`, or the old
/// `tag.display.color`), else leather's own brown.
fn dyed(s: Option<&Tag>) -> [f32; 3] {
    let c = s.and_then(|s| {
        let comp = s.get("components").and_then(|c| c.get("minecraft:dyed_color"));
        comp.and_then(|d| d.as_i64().or_else(|| d.get("rgb").and_then(Tag::as_i64))).or_else(|| s.get("tag")?.get("display")?.get("color")?.as_i64())
    });
    let c = c.unwrap_or(0xA06540) as u32;
    [((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0]
}

/// The quads for an entity (its saved data), in the space of its chunk's corner `origin`
/// (world x, 0, z), and the world spot whose light it takes. `None` for an entity not drawn
/// here.
pub fn entity(e: &Tag, origin: [f32; 3]) -> Option<(Vec<EQuad>, [f32; 3])> {
    let id = e.get("id")?.as_str()?.trim_start_matches("minecraft:");
    let pos = nums(e.get("Pos"));
    let p = [*pos.first()?, *pos.get(1)?, *pos.get(2)?];
    let local = Mat::ID.t(p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]);
    let yaw = nums(e.get("Rotation")).first().copied().unwrap_or(0.0);
    let mut b = Builder::new();
    let light = match id {
        "item_frame" | "glow_item_frame" => frame(&mut b, e, id, origin)?,
        "armor_stand" => {
            armor_stand(&mut b, e, &local, yaw);
            p
        }
        "item" => {
            dropped(&mut b, e, &local, p);
            p
        }
        "painting" => painting(&mut b, e, origin)?,
        _ => {
            // A mob: built as the game builds it, else (any other living thing) marked by its
            // spawn egg. Projectiles, orbs and the like are left out.
            if !crate::mobs::mob(&mut b, e, id, &local, yaw) {
                e.get("Health")?;
                crate::mobs::marker(&mut b, id, &local, yaw);
            }
            [p[0], p[1] + 0.5, p[2]]
        }
    };
    Some((b.out, light))
}

/// An item frame on the face of its block, with what it holds.
fn frame(b: &mut Builder, e: &Tag, id: &str, origin: [f32; 3]) -> Option<[f32; 3]> {
    let tile = block_pos(e)?;
    let half = std::f32::consts::FRAC_PI_2;
    let at = Mat::ID.t(tile[0] as f32 - origin[0], tile[1] as f32 - origin[1], tile[2] as f32 - origin[2]).t(0.5, 0.5, 0.5);
    // The frame's back (in its model, the south side) against the block it hangs on.
    let turned = match e.get("Facing").and_then(Tag::as_i64).unwrap_or(2) {
        0 => at.rx(-half),
        1 => at.rx(half),
        3 => at.ry(std::f32::consts::PI),
        4 => at.ry(half),
        5 => at.ry(-half),
        _ => at,
    };
    let m = turned.t(-0.5, -0.5, -0.5);
    let item = stack_id(e.get("Item"));
    let glow = id == "glow_item_frame";
    let invisible = e.get("Invisible").and_then(Tag::as_i64).unwrap_or(0) != 0;
    if !invisible {
        let map = item.as_deref() == Some("filled_map");
        let name = format!("minecraft:block/{}{}", if glow { "glow_item_frame" } else { "item_frame" }, if map { "_map" } else { "" });
        if let Some(model) = crate::models::named_model(&name) {
            b.model(&m, &model, [1.0; 3]);
        }
    }
    if let Some(item) = item {
        let rot = e.get("ItemRotation").and_then(Tag::as_i64).unwrap_or(0) as f32;
        let block_like = matches!(crate::models::item_model(&item).as_deref(), Some(crate::models::ItemModel::Model(_)));
        // Flat items half a block across, just off the back; block items smaller, sat on it.
        let (s, z) = if block_like { (0.3125, 15.0 / 16.0 - 0.3125 / 2.0) } else { (0.5, 15.0 / 16.0 - 1.0 / 32.0) };
        let im = m.t(0.5, 0.5, z).rz(-rot * 45.0 * DEG).ry(std::f32::consts::PI).s(s, s, s).t(-0.5, -0.5, -0.5);
        b.item(&im, &item, crate::world::block_tint(&item, ""));
    }
    Some([tile[0] as f32 + 0.5, tile[1] as f32 + 0.5, tile[2] as f32 + 0.5])
}

/// An armor stand in its pose, with the armor it wears.
fn armor_stand(b: &mut Builder, e: &Tag, local: &Mat, yaw: f32) {
    let flag = |k: &str| e.get(k).and_then(Tag::as_i64).unwrap_or(0) != 0;
    let small = flag("Small");
    let k = if small { 0.5 } else { 1.0 };
    // As the game places a living entity: turned to face its way, flipped into model space
    // (y down, sixteenths), its feet on the ground.
    let root = local.ry((180.0 - yaw) * DEG).s(-1.0, -1.0, 1.0).s(k, k, k).t(0.0, -1.501, 0.0).s(1.0 / 16.0, 1.0 / 16.0, 1.0 / 16.0);
    let pose = e.get("Pose");
    let angle = |part: &str, default: [f32; 3]| -> [f32; 3] {
        let v = nums(pose.and_then(|p| p.get(part)));
        if v.len() == 3 { [v[0] * DEG, v[1] * DEG, v[2] * DEG] } else { default.map(|d| d * DEG) }
    };
    let (head, body) = (angle("Head", [0.0; 3]), angle("Body", [0.0; 3]));
    let (larm, rarm) = (angle("LeftArm", [-10.0, 0.0, -10.0]), angle("RightArm", [-15.0, 0.0, 10.0]));
    let (lleg, rleg) = (angle("LeftLeg", [-1.0, 0.0, -1.0]), angle("RightLeg", [1.0, 0.0, 1.0]));
    if !flag("Invisible") {
        let tex = Tex::new("entity/armorstand/armorstand", 64.0, 64.0);
        b.cube(&root.pose([0.0, 1.0, 0.0], head), &tex, 0.0, 0.0, [-1.0, -7.0, -1.0], [2.0, 7.0, 2.0], 0.0, false);
        let bm = root.pose([0.0; 3], body);
        b.cube(&bm, &tex, 0.0, 26.0, [-6.0, 0.0, -1.5], [12.0, 3.0, 3.0], 0.0, false);
        b.cube(&bm, &tex, 16.0, 0.0, [-3.0, 3.0, -1.0], [2.0, 7.0, 2.0], 0.0, false);
        b.cube(&bm, &tex, 48.0, 16.0, [1.0, 3.0, -1.0], [2.0, 7.0, 2.0], 0.0, false);
        b.cube(&bm, &tex, 0.0, 48.0, [-4.0, 10.0, -1.0], [8.0, 2.0, 2.0], 0.0, false);
        if flag("ShowArms") {
            b.cube(&root.pose([-5.0, 2.0, 0.0], rarm), &tex, 24.0, 0.0, [-2.0, -2.0, -1.0], [2.0, 12.0, 2.0], 0.0, false);
            b.cube(&root.pose([5.0, 2.0, 0.0], larm), &tex, 32.0, 16.0, [0.0, -2.0, -1.0], [2.0, 12.0, 2.0], 0.0, true);
        }
        b.cube(&root.pose([-1.9, 12.0, 0.0], rleg), &tex, 8.0, 0.0, [-1.0, 0.0, -1.0], [2.0, 11.0, 2.0], 0.0, false);
        b.cube(&root.pose([1.9, 12.0, 0.0], lleg), &tex, 40.0, 16.0, [-1.0, 0.0, -1.0], [2.0, 11.0, 2.0], 0.0, true);
        if !flag("NoBasePlate") {
            b.cube(&root.pose([0.0, 12.0, 0.0], [0.0; 3]), &tex, 0.0, 32.0, [-6.0, 11.0, -6.0], [12.0, 1.0, 12.0], 0.0, false);
        }
    }
    // Its armor: `equipment: {head, chest, legs, feet}` (newer) or `ArmorItems: [feet, legs,
    // chest, head]`.
    let slot = |name: &str, index: usize| -> Option<&Tag> {
        e.get("equipment").and_then(|q| q.get(name)).or_else(|| e.get("ArmorItems")?.as_list()?.get(index))
    };
    let parts = Humanoid { head: root.pose([0.0, 1.0, 0.0], head), body: root.pose([0.0; 3], body), rarm: root.pose([-5.0, 2.0, 0.0], rarm), larm: root.pose([5.0, 2.0, 0.0], larm), rleg: root.pose([-1.9, 12.0, 0.0], rleg), lleg: root.pose([1.9, 12.0, 0.0], lleg) };
    for (name, index) in [("head", 3), ("chest", 2), ("legs", 1), ("feet", 0)] {
        let s = slot(name, index);
        if let Some(item) = stack_id(s) {
            armor(b, &parts, name, &item, dyed(s));
        }
    }
    // What it holds.
    if let Some(c) = e.as_compound() {
        for (slot, item) in crate::mob_edit::equipment(c, None) {
            let Some((item, _)) = item else { continue };
            let item = item.trim_start_matches("minecraft:");
            match slot {
                crate::mob_edit::Slot::MainHand => crate::mobs::held(b, &parts.rarm, item, false),
                crate::mob_edit::Slot::OffHand => crate::mobs::held(b, &parts.larm, item, true),
                _ => {}
            }
        }
    }
}

/// Where a humanoid's parts are (each part's own pose).
pub struct Humanoid {
    pub head: Mat,
    pub body: Mat,
    pub rarm: Mat,
    pub larm: Mat,
    pub rleg: Mat,
    pub lleg: Mat,
}

/// A piece of armor on a humanoid's parts, as the game's armor layer draws it.
pub fn armor(b: &mut Builder, parts: &Humanoid, slot: &str, item: &str, dye: [f32; 3]) {
    let material = item
        .trim_end_matches("_helmet")
        .trim_end_matches("_chestplate")
        .trim_end_matches("_leggings")
        .trim_end_matches("_boots");
    let material = match material {
        "golden" => "gold",
        "turtle" => "turtle_scute",
        "carved_pumpkin" | "skeleton_skull" | "player_head" | "zombie_head" | "creeper_head" => {
            // A block or head worn on the head: the block, over the head.
            let m = parts.head.t(0.0, -4.0, 0.0).s(-10.0, -10.0, 10.0).t(-0.5, -0.5, -0.5);
            b.item(&m, item, [1.0; 3]);
            return;
        }
        m => m,
    };
    if material == item {
        // Not armor: a block worn on the head sits over it, as the game shows it.
        if slot == "head" && matches!(crate::models::item_model(item).as_deref(), Some(crate::models::ItemModel::Model(_))) {
            let m = parts.head.t(0.0, -4.0, 0.0).s(-10.0, -10.0, 10.0).t(-0.5, -0.5, -0.5);
            b.item(&m, item, [1.0; 3]);
        }
        return;
    }
    let legs = slot == "legs";
    let folder = if legs { "humanoid_leggings" } else { "humanoid" };
    let g = if legs { 0.5 } else { 1.0 };
    let mut layers = vec![(format!("entity/equipment/{folder}/{material}"), if material == "leather" { dye } else { [1.0; 3] })];
    if material == "leather" {
        layers.push((format!("entity/equipment/{folder}/leather_overlay"), [1.0; 3]));
    }
    for (file, tint) in layers {
        let tex = Tex::new(&file, 64.0, 32.0);
        b.tint = tint;
        match slot {
            "head" => {
                b.cube(&parts.head, &tex, 0.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], g, false);
                b.cube(&parts.head, &tex, 32.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], g + 0.5, false);
            }
            "chest" => {
                b.cube(&parts.body, &tex, 16.0, 16.0, [-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], g, false);
                b.cube(&parts.rarm, &tex, 40.0, 16.0, [-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], g, false);
                b.cube(&parts.larm, &tex, 40.0, 16.0, [-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], g, true);
            }
            "legs" => {
                b.cube(&parts.body, &tex, 16.0, 16.0, [-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], g, false);
                b.cube(&parts.rleg, &tex, 0.0, 16.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], g, false);
                b.cube(&parts.lleg, &tex, 0.0, 16.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], g, true);
            }
            _ => {
                b.cube(&parts.rleg, &tex, 0.0, 16.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], g, false);
                b.cube(&parts.lleg, &tex, 0.0, 16.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], g, true);
            }
        }
    }
    b.tint = [1.0; 3];
}

/// An item lying on the ground: its block model small, or its picture standing up; a few
/// copies for a stack, as the game shows them.
fn dropped(b: &mut Builder, e: &Tag, local: &Mat, p: [f32; 3]) {
    let s = e.get("Item");
    let Some(item) = stack_id(s) else { return };
    let count = s.and_then(|s| s.get("count").or_else(|| s.get("Count"))).and_then(Tag::as_i64).unwrap_or(1);
    let copies = match count {
        ..=1 => 1,
        2..=16 => 2,
        17..=32 => 3,
        33..=48 => 4,
        _ => 5,
    };
    // A turn of its own (the game spins it; here it holds still at an angle from where it is).
    let spin = ((p[0] * 12.9898 + p[2] * 78.233).sin() * 43758.547).fract() * std::f32::consts::TAU;
    let block_like = matches!(crate::models::item_model(&item).as_deref(), Some(crate::models::ItemModel::Model(_)));
    let tint = crate::world::block_tint(&item, "");
    for c in 0..copies {
        let jitter = |k: u32| ((c as f32 * 7.31 + k as f32 * 3.17 + p[0]).sin() * 0.06) * (c > 0) as i32 as f32;
        let m = local.t(jitter(1), 0.0, jitter(2)).ry(spin);
        let im = if block_like {
            m.t(0.0, 0.1 + 0.125 + c as f32 * 0.03, 0.0).s(0.25, 0.25, 0.25).t(-0.5, -0.5, -0.5)
        } else {
            m.t(0.0, 0.1 + 0.25, c as f32 * 0.04).s(0.5, 0.5, 0.5).t(-0.5, -0.5, -0.5)
        };
        b.item(&im, &item, tint);
    }
}

/// A painting on the wall: its picture at its size, the back and edges in the game's back.
fn painting(b: &mut Builder, e: &Tag, origin: [f32; 3]) -> Option<[f32; 3]> {
    let tile = block_pos(e)?;
    let variant = e.get("variant").or_else(|| e.get("Motive")).and_then(Tag::as_str).unwrap_or("kebab").to_ascii_lowercase();
    let (w, h, tex) = crate::models::painting(&variant)?;
    // Which way it faces (south, west, north, east), and to its viewer's right.
    let facing = e.get("facing").or_else(|| e.get("Facing")).and_then(Tag::as_i64).unwrap_or(0);
    let (n, r): ([f32; 3], [f32; 3]) = match facing {
        1 => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        2 => ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0]),
        3 => ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        _ => ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
    };
    let (wf, hf) = (w as f32, h as f32);
    // Its middle: against the wall behind its block, shifted half a block right and up for an
    // even size (as the game hangs it).
    let off_r = if w % 2 == 0 { 0.5 } else { 0.0 };
    let off_u = if h % 2 == 0 { 0.5 } else { 0.0 };
    let c = [0, 1, 2].map(|k| tile[k] as f32 + 0.5 - origin[k] - n[k] * 0.46875 + r[k] * off_r + if k == 1 { off_u } else { 0.0 });
    let at = |dr: f32, du: f32, dn: f32| [c[0] + r[0] * dr + n[0] * dn, c[1] + du + n[1] * dn, c[2] + r[2] * dr + n[2] * dn];
    let t = 1.0 / 32.0;
    let (l, rt, top, bot) = (-wf / 2.0, wf / 2.0, hf / 2.0, -hf / 2.0);
    b.quad(&Mat::ID, &tex, [at(l, top, t), at(rt, top, t), at(rt, bot, t), at(l, bot, t)], [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
    let back: Arc<str> = "painting/back.png".into();
    b.quad(&Mat::ID, &back, [at(rt, top, -t), at(l, top, -t), at(l, bot, -t), at(rt, bot, -t)], [[0.0, 0.0], [wf, 0.0], [wf, hf], [0.0, hf]].map(|[u, v]| [u.min(1.0), v.min(1.0)]));
    let edge = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0 / 16.0], [0.0, 1.0 / 16.0]];
    b.quad(&Mat::ID, &back, [at(l, top, -t), at(rt, top, -t), at(rt, top, t), at(l, top, t)], edge);
    b.quad(&Mat::ID, &back, [at(l, bot, t), at(rt, bot, t), at(rt, bot, -t), at(l, bot, -t)], edge);
    b.quad(&Mat::ID, &back, [at(l, top, -t), at(l, top, t), at(l, bot, t), at(l, bot, -t)], edge);
    b.quad(&Mat::ID, &back, [at(rt, top, t), at(rt, top, -t), at(rt, bot, -t), at(rt, bot, t)], edge);
    Some([tile[0] as f32 + 0.5, tile[1] as f32 + 0.5, tile[2] as f32 + 0.5])
}
