//! Block models as the game draws them, from its own files: a block's state picks models from
//! its `blockstates/<block>.json` (variants, or multipart pieces chosen by conditions), each
//! model's boxes come from `models/block/*.json` (inheriting from parents, textures named by
//! variables), turned as the state says (by 90° steps, with the texture locked to the world
//! where asked) and with each box's own turn. Read from the chosen resource pack where it has
//! its own files, else the latest release's from mcasset.cloud (see `assets`).
//!
//! The result is a list of quads in the block's space (0 to 1), each with its texture file,
//! the part of the texture it shows, whether it takes the block's tint, which side of the
//! block hides it (when a whole block is there) and how the game shades it.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use serde_json::Value;

/// One face of a model, in the block's space.
#[derive(Clone)]
pub struct Quad {
    /// Corners top-left, top-right, bottom-right, bottom-left as seen from outside.
    pub pos: [[f32; 3]; 4],
    /// Each corner's place in its texture (0 to 1 across and down).
    pub uv: [[f32; 2]; 4],
    /// The texture file (`block/stone.png`).
    pub tex: Arc<str>,
    /// Takes the block's tint (grass, leaves, redstone...).
    pub tint: bool,
    /// The side of the block (top, bottom, north, south, west, east) whose neighbor, when it is
    /// a whole block, hides this face.
    pub cull: Option<usize>,
    /// How the game shades a face facing this way (1 straight up, 0.5 down...).
    pub shade: f32,
    /// The side it faces, when it faces straight along an axis, and whether it lies on that
    /// side of the block (so its light is the neighbor's, shaded by the corners).
    pub face: Option<usize>,
    pub on_side: bool,
}

/// A model as placed: its faces.
pub struct Model {
    pub quads: Vec<Quad>,
    /// Shaded in its corners (ambient occlusion), as most blocks are.
    pub ao: bool,
}

/// The models a block state is drawn with: one of `alts` (by weight, picked per position as
/// the game does), and the box around them all.
pub struct Baked {
    pub alts: Vec<(Arc<Model>, u32)>,
    pub total: u32,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    /// It fills its whole block (some element runs from corner to corner).
    pub full: bool,
}

impl Baked {
    /// The model for a block at a position (the same position always gets the same one).
    pub fn pick(&self, x: i32, y: i32, z: i32) -> &Model {
        &self.alts[self.pick_index(x, y, z)].0
    }

    /// Which of `alts` a block at a position gets.
    pub fn pick_index(&self, x: i32, y: i32, z: i32) -> usize {
        if self.alts.len() == 1 || self.total == 0 {
            return 0;
        }
        // The game's position hash, near enough: spread well, the same every time.
        let mut h = (x as i64).wrapping_mul(3129871) ^ (z as i64).wrapping_mul(116129781) ^ (y as i64);
        h = h.wrapping_mul(h.wrapping_mul(42317861).wrapping_add(11));
        let mut k = ((h >> 16) as u64 % self.total as u64) as u32;
        for (i, (_, w)) in self.alts.iter().enumerate() {
            if k < *w {
                return i;
            }
            k -= w;
        }
        0
    }
}

/// The side names as models give them, in the order faces are numbered here.
const SIDES: [&str; 6] = ["up", "down", "north", "south", "west", "east"];
const DIRS: [[f32; 3]; 6] = [[0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]];

// ---------------------------------------------------------------------------------------------
// Reading the files
// ---------------------------------------------------------------------------------------------

/// A JSON file of the game's assets (`models/block/stone.json`): the chosen pack's own, else the
/// latest release's (see `assets`).
fn asset_json(rel: &str) -> Option<Value> {
    let p = crate::icons::pack_asset(rel)?;
    std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// Whether the game's files can be had (fetched, or kept from before).
pub fn available() -> bool {
    asset_json("blockstates/stone.json").is_some()
}

static MODEL_FILES: LazyLock<Mutex<HashMap<String, Option<Arc<Value>>>>> = LazyLock::new(Default::default);

/// `minecraft:block/stone` or `block/stone` as `block/stone`.
fn plain_ref(r: &str) -> &str {
    r.strip_prefix("minecraft:").unwrap_or(r)
}

fn model_file(name: &str) -> Option<Arc<Value>> {
    let name = plain_ref(name).to_string();
    if let Some(v) = MODEL_FILES.lock().ok()?.get(&name) {
        return v.clone();
    }
    let v = asset_json(&format!("models/{name}.json")).map(Arc::new);
    MODEL_FILES.lock().ok()?.insert(name, v.clone());
    v
}

/// A model with its parents' parts filled in: its textures (by variable), its boxes, and
/// whether it is shaded in its corners.
struct Resolved {
    textures: HashMap<String, String>,
    elements: Option<Arc<Value>>,
    ao: bool,
}

fn resolve(name: &str) -> Option<Resolved> {
    let mut textures = HashMap::new();
    let mut elements = None;
    let mut ao = None;
    let mut next = Some(name.to_string());
    let mut depth = 0;
    while let Some(n) = next.take() {
        depth += 1;
        if depth > 16 {
            break;
        }
        let Some(m) = model_file(&n) else { break };
        if let Some(t) = m["textures"].as_object() {
            for (k, v) in t {
                // A name, or (newer packs) `{"sprite": name, "force_translucent": ...}`.
                if let Some(s) = v.as_str().or_else(|| v["sprite"].as_str()) {
                    textures.entry(k.clone()).or_insert_with(|| s.to_string());
                }
            }
        }
        if elements.is_none() && m["elements"].is_array() {
            elements = Some(Arc::new(m["elements"].clone()));
        }
        if ao.is_none() {
            ao = m["ambientocclusion"].as_bool();
        }
        next = m["parent"].as_str().map(str::to_string);
    }
    Some(Resolved { textures, elements, ao: ao.unwrap_or(true) })
}

/// A texture reference (`#side`, followed through the variables) as its file.
fn texture_file(r: &str, textures: &HashMap<String, String>) -> Option<Arc<str>> {
    let mut r = r.to_string();
    for _ in 0..10 {
        match r.strip_prefix('#') {
            Some(var) => r = textures.get(var)?.clone(),
            None => return Some(format!("{}.png", plain_ref(&r)).into()),
        }
    }
    None
}

/// Whether a texture is partly see-through (stained glass, ice, honey, slime), so it is drawn
/// blended over what is behind rather than with its clear parts cut out.
pub fn translucent(tex: &str) -> bool {
    static SEEN: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Default::default);
    if let Some(t) = SEEN.lock().ok().and_then(|s| s.get(tex).copied()) {
        return t;
    }
    let t = crate::icons::pack_texture(tex).and_then(|p| image::open(p).ok()).is_some_and(|img| {
        let img = img.to_rgba8();
        let w = img.width();
        // The first frame of an animated picture.
        let frame = image::imageops::crop_imm(&img, 0, 0, w, w.min(img.height())).to_image();
        let n = frame.pixels().count().max(1);
        let half = frame.pixels().filter(|p| p[3] > 10 && p[3] < 245).count();
        half * 20 > n
    });
    if let Ok(mut s) = SEEN.lock() {
        s.insert(tex.to_string(), t);
    }
    t
}

// ---------------------------------------------------------------------------------------------
// Turning
// ---------------------------------------------------------------------------------------------

/// `p` turned by `deg` degrees about `axis` (0 x, 1 y, 2 z) through `o`, right-handed.
fn rotate(p: [f32; 3], axis: usize, deg: f32, o: [f32; 3]) -> [f32; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    let (x, y, z) = (p[0] - o[0], p[1] - o[1], p[2] - o[2]);
    let r = match axis {
        0 => [x, y * c - z * s, y * s + z * c],
        1 => [x * c + z * s, y, -x * s + z * c],
        _ => [x * c - y * s, x * s + y * c, z],
    };
    [r[0] + o[0], r[1] + o[1], r[2] + o[2]]
}

/// The side a direction points to, if it points straight along an axis.
fn side_of(d: [f32; 3]) -> Option<usize> {
    DIRS.iter().position(|v| (v[0] - d[0]).abs() < 0.01 && (v[1] - d[1]).abs() < 0.01 && (v[2] - d[2]).abs() < 0.01)
}

/// Where a point falls in the texture of a face looking toward `side`, as the game lays a
/// texture over a face when the model gives no part of it (and to keep it fixed to the world
/// when a block is turned with `uvlock`).
fn projected(p: [f32; 3], side: usize) -> [f32; 2] {
    match side {
        0 => [p[0], p[2]],
        1 => [p[0], 1.0 - p[2]],
        2 => [1.0 - p[0], 1.0 - p[1]],
        3 => [p[0], 1.0 - p[1]],
        4 => [p[2], 1.0 - p[1]],
        _ => [1.0 - p[2], 1.0 - p[1]],
    }
}

fn num3(v: &Value) -> Option<[f32; 3]> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32])
}

/// A model's faces, turned by `rx` then `ry` degrees (as a block state turns it), the textures
/// kept to the world with `uvlock`.
fn bake_model(name: &str, rx: f32, ry: f32, uvlock: bool) -> Option<Model> {
    let r = resolve(name)?;
    let mut quads = Vec::new();
    let center = [0.5, 0.5, 0.5];
    let turn = |p: [f32; 3]| rotate(rotate(p, 0, -rx, center), 1, -ry, center);
    let turn_dir = |d: [f32; 3]| rotate(rotate(d, 0, -rx, [0.0; 3]), 1, -ry, [0.0; 3]);
    for e in r.elements.as_ref().and_then(|e| e.as_array()).map_or(&[][..], |v| v.as_slice()) {
        let (Some(from), Some(to)) = (num3(&e["from"]), num3(&e["to"])) else { continue };
        let (f, t) = (from.map(|v| v / 16.0), to.map(|v| v / 16.0));
        let shade_on = e["shade"].as_bool().unwrap_or(true);
        // The box's own turn.
        let rot = &e["rotation"];
        let erot = rot["angle"].as_f64().map(|a| {
            let axis = match rot["axis"].as_str() {
                Some("x") => 0,
                Some("z") => 2,
                _ => 1,
            };
            let o = num3(&rot["origin"]).map_or([0.5; 3], |o| o.map(|v| v / 16.0));
            (axis, a as f32, o, rot["rescale"].as_bool().unwrap_or(false))
        });
        let (x0, y0, z0, x1, y1, z1) = (f[0], f[1], f[2], t[0], t[1], t[2]);
        let corners: [[[f32; 3]; 4]; 6] = [
            [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
            [[x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0]],
            [[x1, y1, z0], [x0, y1, z0], [x0, y0, z0], [x1, y0, z0]],
            [[x0, y1, z1], [x1, y1, z1], [x1, y0, z1], [x0, y0, z1]],
            [[x0, y1, z0], [x0, y1, z1], [x0, y0, z1], [x0, y0, z0]],
            [[x1, y1, z1], [x1, y1, z0], [x1, y0, z0], [x1, y0, z1]],
        ];
        let Some(faces) = e["faces"].as_object() else { continue };
        for (k, side) in SIDES.iter().enumerate() {
            let Some(face) = faces.get(*side) else { continue };
            let Some(tex) = face["texture"].as_str().and_then(|t| texture_file(t, &r.textures)) else { continue };
            let mut pos = corners[k];
            // Its part of the texture: as given (in sixteenths), else where the face falls.
            let mut uv: [[f32; 2]; 4] = match face["uv"].as_array().filter(|a| a.len() == 4) {
                Some(a) => {
                    let v: Vec<f32> = a.iter().map(|n| n.as_f64().unwrap_or(0.0) as f32 / 16.0).collect();
                    [[v[0], v[1]], [v[2], v[1]], [v[2], v[3]], [v[0], v[3]]]
                }
                None => pos.map(|p| projected(p, k)),
            };
            // A face's texture turned by 90° steps, clockwise.
            let steps = (face["rotation"].as_i64().unwrap_or(0) / 90).rem_euclid(4) as usize;
            if steps != 0 {
                let was = uv;
                for (i, c) in uv.iter_mut().enumerate() {
                    *c = was[(i + 4 - steps) % 4];
                }
            }
            if let Some((axis, angle, o, rescale)) = erot {
                for p in &mut pos {
                    let mut q = rotate(*p, axis, angle, o);
                    if rescale {
                        let s = 1.0 / angle.to_radians().cos().max(0.1);
                        for a in 0..3 {
                            if a != axis {
                                q[a] = o[a] + (q[a] - o[a]) * s;
                            }
                        }
                    }
                    *p = q;
                }
            }
            let pos = pos.map(turn);
            let n = {
                let (a, b) = ([pos[3][0] - pos[0][0], pos[3][1] - pos[0][1], pos[3][2] - pos[0][2]], [pos[1][0] - pos[0][0], pos[1][1] - pos[0][1], pos[1][2] - pos[0][2]]);
                let c = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
                let l = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt().max(1e-6);
                c.map(|v| v / l)
            };
            let facing = side_of(n);
            if uvlock && (rx != 0.0 || ry != 0.0) {
                if let Some(s) = facing {
                    uv = pos.map(|p| projected(p, s));
                }
            }
            let cull = face["cullface"].as_str().and_then(|c| SIDES.iter().position(|s| *s == c)).and_then(|c| side_of(turn_dir(DIRS[c])));
            let on_side = facing.is_some_and(|s| {
                let (axis, end) = match s {
                    0 => (1, 1.0),
                    1 => (1, 0.0),
                    2 => (2, 0.0),
                    3 => (2, 1.0),
                    4 => (0, 0.0),
                    _ => (0, 1.0),
                };
                pos.iter().all(|p| (p[axis] - end).abs() < 1e-4)
            });
            let shade = if shade_on { (n[0] * n[0] * 0.6 + n[1] * n[1] * ((3.0 + n[1]) / 4.0) + n[2] * n[2] * 0.8).min(1.0) } else { 1.0 };
            quads.push(Quad { pos, uv, tex, tint: face["tintindex"].as_i64().is_some_and(|t| t >= 0), cull, shade, face: facing, on_side });
        }
    }
    Some(Model { quads, ao: r.ao })
}

// ---------------------------------------------------------------------------------------------
// Block states
// ---------------------------------------------------------------------------------------------

/// A block state's properties by name.
fn props_map(props: &str) -> HashMap<&str, &str> {
    props.split(',').filter_map(|p| p.split_once('=')).collect()
}

/// Whether a variant's key (`facing=east,half=bottom`; empty for every state) fits a state.
fn key_fits(key: &str, props: &HashMap<&str, &str>) -> bool {
    key.is_empty() || key == "normal" || key.split(',').filter_map(|p| p.split_once('=')).all(|(k, v)| props.get(k) == Some(&v))
}

/// Whether a multipart piece's condition holds for a state.
fn when_fits(when: &Value, props: &HashMap<&str, &str>) -> bool {
    let Some(obj) = when.as_object() else { return true };
    if let Some(any) = obj.get("OR").and_then(Value::as_array) {
        return any.iter().any(|w| when_fits(w, props));
    }
    if let Some(all) = obj.get("AND").and_then(Value::as_array) {
        return all.iter().all(|w| when_fits(w, props));
    }
    obj.iter().all(|(k, v)| {
        let want = match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let have = props.get(k.as_str()).copied();
        match want.strip_prefix('!') {
            Some(not) => have.is_none_or(|h| !not.split('|').any(|w| w == h)),
            None => have.is_some_and(|h| want.split('|').any(|w| w == h)),
        }
    })
}

/// A variant's models with their weights (a list is picked from by weight).
fn variant_models(v: &Value) -> Vec<(Model, u32)> {
    let one = |v: &Value| -> Option<(Model, u32)> {
        let m = bake_model(v["model"].as_str()?, v["x"].as_f64().unwrap_or(0.0) as f32, v["y"].as_f64().unwrap_or(0.0) as f32, v["uvlock"].as_bool().unwrap_or(false))?;
        Some((m, v["weight"].as_u64().unwrap_or(1).max(1) as u32))
    };
    match v.as_array() {
        Some(list) => list.iter().filter_map(one).collect(),
        None => one(v).into_iter().collect(),
    }
}

static BLOCKSTATES: LazyLock<Mutex<HashMap<String, Option<Arc<Value>>>>> = LazyLock::new(Default::default);
static BAKED: LazyLock<Mutex<HashMap<String, Option<Arc<Baked>>>>> = LazyLock::new(Default::default);

fn blockstate(block: &str) -> Option<Arc<Value>> {
    if let Some(v) = BLOCKSTATES.lock().ok()?.get(block) {
        return v.clone();
    }
    let v = asset_json(&format!("blockstates/{block}.json")).map(Arc::new);
    BLOCKSTATES.lock().ok()?.insert(block.to_string(), v.clone());
    v
}

/// The models a block (by its bare id, `oak_stairs`) in a state (`facing=east,half=bottom,...`)
/// is drawn with; `None` when the game's files have nothing for it (or no variant fits).
pub fn baked(block: &str, props: &str) -> Option<Arc<Baked>> {
    let key = format!("{block}[{props}]");
    if let Some(b) = BAKED.lock().ok()?.get(&key) {
        return b.clone();
    }
    let made = make(block, props).map(Arc::new);
    BAKED.lock().ok()?.insert(key, made.clone());
    made
}

fn make(block: &str, props: &str) -> Option<Baked> {
    let state = blockstate(block)?;
    let p = props_map(props);
    let mut alts: Vec<(Model, u32)> = Vec::new();
    if let Some(vars) = state["variants"].as_object() {
        // The variant whose key fits (the most specific, should several); for a state missing
        // some properties (an old or hand-made save), the first that fits those it has.
        let strict = vars.iter().filter(|(k, _)| key_fits(k, &p)).max_by_key(|(k, _)| k.split(',').count());
        // Of those, the one whose values for the missing properties are the game's usual defaults
        // (upright, facing north, the bottom half, off, closed...).
        let loose = || {
            const DEFAULTS: [(&str, &str); 16] = [
                ("axis", "y"), ("facing", "north"), ("half", "bottom"), ("type", "bottom"), ("shape", "straight"), ("waterlogged", "false"),
                ("powered", "false"), ("open", "false"), ("lit", "false"), ("snowy", "false"), ("age", "0"), ("face", "floor"),
                ("hanging", "false"), ("attachment", "floor"), ("part", "foot"), ("rotation", "0"),
            ];
            vars.iter()
                .filter(|(k, _)| k.split(',').filter_map(|kv| kv.split_once('=')).all(|(k, v)| p.get(k).is_none_or(|have| *have == v)))
                .max_by_key(|(k, _)| {
                    let parts: Vec<_> = k.split(',').filter_map(|kv| kv.split_once('=')).collect();
                    parts.iter().filter(|(k, v)| !p.contains_key(k) && DEFAULTS.contains(&(*k, *v))).count() * 10 + parts.len()
                })
        };
        let (_, v) = strict.or_else(loose)?;
        alts = variant_models(v);
    } else if let Some(parts) = state["multipart"].as_array() {
        // Every piece whose condition holds, all together (a weighted piece: its first).
        let mut quads = Vec::new();
        let mut ao = true;
        for part in parts {
            if !when_fits(&part["when"], &p) {
                continue;
            }
            if let Some((m, _)) = variant_models(&part["apply"]).into_iter().next() {
                ao &= m.ao;
                quads.extend(m.quads);
            }
        }
        alts.push((Model { quads, ao }, 1));
    }
    if alts.is_empty() || alts.iter().all(|(m, _)| m.quads.is_empty()) {
        return None;
    }
    let (mut lo, mut hi) = ([1.0f32; 3], [0.0f32; 3]);
    for q in alts.iter().flat_map(|(m, _)| m.quads.iter()) {
        for c in &q.pos {
            for a in 0..3 {
                lo[a] = lo[a].min(c[a]);
                hi[a] = hi[a].max(c[a]);
            }
        }
    }
    // Whole: every side has a face covering it entirely, hidden by a whole neighbor.
    let full = alts.iter().all(|(m, _)| {
        (0..6).all(|s| m.quads.iter().any(|q| q.cull == Some(s) && q.on_side && {
            let (mut a, mut b) = ([1.0f32; 3], [0.0f32; 3]);
            for c in &q.pos {
                for k in 0..3 {
                    a[k] = a[k].min(c[k]);
                    b[k] = b[k].max(c[k]);
                }
            }
            (0..3).filter(|k| *k != [1, 1, 2, 2, 0, 0][s]).all(|k| a[k] < 1e-4 && b[k] > 1.0 - 1e-4)
        }))
    });
    let total = alts.iter().map(|(_, w)| *w).sum();
    Some(Baked { alts: alts.into_iter().map(|(m, w)| (Arc::new(m), w)).collect(), total, lo, hi, full })
}

// ---------------------------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------------------------

/// How an item is drawn: a block-shaped model (a block item, or an item with its own boxes),
/// or flat pictures (layers, first at the bottom) pushed out a pixel thick, as the game's
/// "generated" items are.
pub enum ItemModel {
    Model(Model),
    Flat(Vec<Arc<str>>),
}

/// A state a block (bare id) can be in, from its blockstate file: its first variant's, or for one
/// built of parts each property's first value its parts are shown for. Empty when it has none.
pub fn default_state(name: &str) -> String {
    let Some(json) = asset_json(&format!("blockstates/{name}.json")) else { return String::new() };
    if let Some(v) = json.get("variants").and_then(|v| v.as_object()) {
        return v.keys().next().cloned().unwrap_or_default();
    }
    let mut props: Vec<(String, String)> = Vec::new();
    let mut add = |when: &Value| {
        for (k, v) in when.as_object().into_iter().flatten() {
            let first = v.as_str().map(|s| s.split('|').next().unwrap_or(s).to_string());
            if let (Some(first), false) = (first, props.iter().any(|(p, _)| p == k)) {
                props.push((k.clone(), first));
            }
        }
    };
    for part in json.get("multipart").and_then(|m| m.as_array()).into_iter().flatten() {
        if let Some(when) = part.get("when") {
            match when.get("OR").or_else(|| when.get("AND")).and_then(|o| o.as_array()) {
                Some(list) => list.iter().for_each(&mut add),
                None => add(when),
            }
        }
    }
    props.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")
}

/// For picturing a block that is no item: its last variant (a crop or stem fully grown), else
/// as [`default_state`].
pub fn shown_state(name: &str) -> String {
    let last = asset_json(&format!("blockstates/{name}.json"))
        .and_then(|j| j.get("variants").and_then(|v| v.as_object()).and_then(|v| v.keys().last().cloned()));
    last.unwrap_or_else(|| default_state(name))
}

/// A model by its name (`minecraft:block/item_frame`), unturned (once worked out).
pub fn named_model(name: &str) -> Option<Arc<Model>> {
    static NAMED: LazyLock<Mutex<HashMap<String, Option<Arc<Model>>>>> = LazyLock::new(Default::default);
    if let Some(m) = NAMED.lock().ok()?.get(name) {
        return m.clone();
    }
    let m = bake_model(name, 0.0, 0.0, false).filter(|m| !m.quads.is_empty()).map(Arc::new);
    NAMED.lock().ok()?.insert(name.to_string(), m.clone());
    m
}

/// The model an item definition (`items/<id>.json`) shows in the plainest case: a plain
/// model, a select's or condition's fallback, a special model's base, a composite's first.
fn definition_model(v: &Value) -> Option<String> {
    match v["type"].as_str().map(|t| t.trim_start_matches("minecraft:")) {
        Some("model") => v["model"].as_str().map(str::to_string),
        Some("special") => v["base"].as_str().map(str::to_string),
        Some("composite") => v["models"].as_array()?.iter().find_map(definition_model),
        Some("condition") => definition_model(&v["on_false"]).or_else(|| definition_model(&v["on_true"])),
        Some("select") => definition_model(&v["fallback"]).or_else(|| v["cases"].as_array()?.iter().find_map(|c| definition_model(&c["model"]))),
        Some("range_dispatch") => definition_model(&v["fallback"]).or_else(|| v["entries"].as_array()?.iter().find_map(|c| definition_model(&c["model"]))),
        _ => None,
    }
}

/// How an item (by id, `diamond_sword`) is drawn, once worked out.
pub fn item_model(id: &str) -> Option<Arc<ItemModel>> {
    static ITEMS: LazyLock<Mutex<HashMap<String, Option<Arc<ItemModel>>>>> = LazyLock::new(Default::default);
    let id = id.trim_start_matches("minecraft:");
    if let Some(m) = ITEMS.lock().ok()?.get(id) {
        return m.clone();
    }
    let made = (|| {
        let name = asset_json(&format!("items/{id}.json"))
            .and_then(|d| definition_model(&d["model"]))
            .unwrap_or_else(|| format!("minecraft:item/{id}"));
        let r = resolve(&name)?;
        if r.elements.is_some() {
            return bake_model(&name, 0.0, 0.0, false).filter(|m| !m.quads.is_empty()).map(ItemModel::Model);
        }
        let layers: Vec<Arc<str>> = (0..5).map_while(|k| r.textures.get(&format!("layer{k}")).and_then(|t| texture_file(t, &r.textures))).collect();
        if !layers.is_empty() {
            return Some(ItemModel::Flat(layers));
        }
        // Drawn in code by the game (chests, shields...): its plain picture, if it has one.
        crate::icons::pack_texture(&format!("item/{id}.png")).map(|_| ItemModel::Flat(vec![format!("item/{id}.png").into()]))
    })()
    .map(Arc::new);
    ITEMS.lock().ok()?.insert(id.to_string(), made.clone());
    made
}

/// Which pixels of a picture are solid (its first frame), and its size, for pushing a flat
/// item out a pixel thick (once each).
pub fn sprite_mask(tex: &str) -> Option<Arc<(usize, usize, Vec<bool>)>> {
    static MASKS: LazyLock<Mutex<HashMap<String, Option<Arc<(usize, usize, Vec<bool>)>>>>> = LazyLock::new(Default::default);
    if let Some(m) = MASKS.lock().ok()?.get(tex) {
        return m.clone();
    }
    let m = crate::icons::pack_texture(tex).and_then(|p| image::open(p).ok()).map(|img| {
        let img = img.to_rgba8();
        let w = img.width() as usize;
        let h = w.min(img.height() as usize);
        let mask = (0..w * h).map(|i| img.get_pixel((i % w) as u32, (i / w) as u32)[3] > 16).collect();
        Arc::new((w, h, mask))
    });
    MASKS.lock().ok()?.insert(tex.to_string(), m.clone());
    m
}

/// A painting's size in blocks and its picture (`painting/alban.png`), by its variant
/// (`minecraft:alban`).
pub fn painting(variant: &str) -> Option<(u32, u32, Arc<str>)> {
    let v = variant.trim_start_matches("minecraft:");
    let p = crate::assets::fetch(&format!("data/minecraft/painting_variant/{v}.json"))?;
    let d: Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    let asset = d["asset_id"].as_str().unwrap_or(v).trim_start_matches("minecraft:").to_string();
    Some((d["width"].as_u64().unwrap_or(1) as u32, d["height"].as_u64().unwrap_or(1) as u32, format!("painting/{asset}.png").into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stairs_and_fences_from_the_game_files() {
        crate::assets::set_root(std::env::temp_dir().join("mctk-snapshot-pack"));
        if !available() {
            eprintln!("game assets cannot be had (offline): skipped");
            return;
        }
        let s = baked("oak_stairs", "facing=east,half=bottom,shape=straight,waterlogged=false").expect("oak stairs");
        let m = s.pick(0, 0, 0);
        println!("oak stairs: {} quads, box {:?}..{:?}, full {}", m.quads.len(), s.lo, s.hi, s.full);
        assert!(m.quads.len() >= 10 && !s.full);
        assert!(m.quads.iter().all(|q| q.tex.as_ref() == "block/oak_planks.png"));
        // The step is on the east half (facing east), on top.
        let top_step = m.quads.iter().filter(|q| q.face == Some(0) && q.pos.iter().all(|p| (p[1] - 1.0).abs() < 1e-4)).flat_map(|q| q.pos).fold(1.0f32, |a, p| a.min(p[0]));
        assert!((top_step - 0.5).abs() < 1e-4, "the step's top should start halfway across (got {top_step})");
        let south = baked("oak_stairs", "facing=south,half=bottom,shape=straight,waterlogged=false").expect("south stairs");
        let z0 = south.pick(0, 0, 0).quads.iter().filter(|q| q.face == Some(0) && q.pos.iter().all(|p| (p[1] - 1.0).abs() < 1e-4)).flat_map(|q| q.pos).fold(1.0f32, |a, p| a.min(p[2]));
        assert!((z0 - 0.5).abs() < 1e-4, "facing south, the step is on the south half (got {z0})");
        let fence = baked("oak_fence", "east=true,north=false,south=false,waterlogged=false,west=false").expect("fence");
        println!("fence: {} quads", fence.pick(0, 0, 0).quads.len());
        assert!(fence.hi[0] > 0.99 && fence.lo[0] > 0.3, "the fence reaches east only");
        let stone = baked("stone", "").expect("stone");
        assert!(stone.full);
        let grass = baked("grass_block", "snowy=false").expect("grass");
        assert!(grass.pick(0, 0, 0).quads.iter().any(|q| q.tint), "grass has tinted faces");
        let torch = baked("wall_torch", "facing=north").expect("wall torch");
        assert!(!torch.full);
        let pane = baked("glass_pane", "east=true,west=false").expect("pane");
        for q in &pane.pick(0, 0, 0).quads {
            println!("pane quad {:?} {:?} cull {:?}", q.tex, q.pos, q.cull);
        }
        assert!(pane.hi[0] > 0.99, "the pane reaches east");
    }
}
