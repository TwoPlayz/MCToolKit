//! Mobs as the game builds them: each from boxes laid out on its texture (the game's
//! `LayerDefinition`s), in its resting pose, turned the way it faces, with its variant's
//! texture and the armor it wears. A mob not built here is marked by its spawn egg.

use mctk_nbt::Tag;

use crate::entities::{armor, Builder, Humanoid, Mat, Tex};

const PI: f32 = std::f32::consts::PI;
const HALF: f32 = std::f32::consts::FRAC_PI_2;

/// A variant name saved as `minecraft:cold` (or a number, for the oldest saves).
fn variant(e: &Tag, key: &str) -> Option<String> {
    let v = e.get(key)?;
    v.as_str().map(|s| s.trim_start_matches("minecraft:").to_string())
}

/// Draws a mob (by its id, from its saved data) at `local` facing `yaw`; false for a mob not
/// built here.
pub fn mob(b: &mut Builder, e: &Tag, id: &str, local: &Mat, yaw: f32) -> bool {
    let flag = |k: &str| e.get(k).and_then(Tag::as_i64).unwrap_or(0) != 0;
    let baby = flag("IsBaby") || e.get("Age").and_then(Tag::as_i64).is_some_and(|a| a < 0);
    let scale = match id {
        "husk" => 1.0625,
        "wither_skeleton" => 1.2,
        "cave_spider" => 0.7,
        "giant" => 6.0,
        "ghast" => 4.5,
        "happy_ghast" => 4.0,
        "wither" => 2.0,
        "elder_guardian" => 2.35,
        "ender_dragon" => 1.0,
        _ => 1.0,
    } * if baby { 0.5 } else { 1.0 };
    let scale = if matches!(id, "slime" | "magma_cube") {
        // A slime's size: 1 (small) up, as saved (from 0).
        (e.get("Size").and_then(Tag::as_i64).unwrap_or(0) + 1) as f32 * 0.999
    } else {
        scale
    };
    // As the game places a living thing: turned to face its way, flipped into model space (y
    // down, in sixteenths), feet on the ground.
    let root = local.ry((180.0 - yaw) * PI / 180.0).s(-1.0, -1.0, 1.0).s(scale, scale, scale).t(0.0, -1.501, 0.0).s(1.0 / 16.0, 1.0 / 16.0, 1.0 / 16.0);
    let r = &root;
    // 26.x's babies have models of their own (worked out from their pictures); drawn at full
    // size, their boxes being a baby's.
    if baby {
        let r_full = local.ry((180.0 - yaw) * PI / 180.0).s(-1.0, -1.0, 1.0).t(0.0, -1.501, 0.0).s(1.0 / 16.0, 1.0 / 16.0, 1.0 / 16.0);
        if baby_model(b, e, id, &r_full) {
            return true;
        }
    }
    match id {
        "zombie" | "husk" | "drowned" | "giant" | "zombified_piglin" | "piglin" | "piglin_brute" | "skeleton" | "stray" | "bogged" | "wither_skeleton" | "parched" | "player" => {
            humanoid(b, e, id, r);
        }
        "zombie_villager" | "villager" | "wandering_trader" | "witch" | "pillager" | "vindicator" | "evoker" | "illusioner" => villager(b, e, id, r),
        "creeper" => {
            let t = Tex::new("entity/creeper/creeper", 64.0, 32.0);
            b.cube(&r.pose([0.0, 6.0, 0.0], [0.0; 3]), &t, 0.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], 0.0, false);
            b.cube(&r.pose([0.0, 6.0, 0.0], [0.0; 3]), &t, 16.0, 16.0, [-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], 0.0, false);
            for (x, z) in [(-2.0, 4.0), (2.0, 4.0), (-2.0, -4.0), (2.0, -4.0)] {
                b.cube(&r.pose([x, 18.0, z], [0.0; 3]), &t, 0.0, 16.0, [-2.0, 0.0, -2.0], [4.0, 6.0, 4.0], 0.0, false);
            }
        }
        "pig" => {
            let v = variant(e, "variant").unwrap_or_else(|| "temperate".into());
            let t = Tex::new(&format!("entity/pig/pig_{v}"), 64.0, 64.0);
            let head = r.pose([0.0, 12.0, -6.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-4.0, -4.0, -8.0], [8.0, 8.0, 8.0], 0.0, false);
            b.cube(&head, &t, 16.0, 16.0, [-2.0, 0.0, -9.0], [4.0, 3.0, 1.0], 0.0, false);
            b.cube(&r.pose([0.0, 11.0, 2.0], [HALF, 0.0, 0.0]), &t, 28.0, 8.0, [-5.0, -10.0, -7.0], [10.0, 16.0, 8.0], 0.0, false);
            legs(b, r, &t, 0.0, 16.0, 6.0, [(-3.0, 7.0), (3.0, 7.0), (-3.0, -5.0), (3.0, -5.0)], 18.0, [4.0, 4.0]);
        }
        "cow" | "mooshroom" => {
            let file = if id == "mooshroom" {
                format!("entity/cow/mooshroom_{}", variant(e, "Type").unwrap_or_else(|| "red".into()))
            } else {
                format!("entity/cow/cow_{}", variant(e, "variant").unwrap_or_else(|| "temperate".into()))
            };
            let t = Tex::new(&file, 64.0, 64.0);
            // The cow model since the cow variants (1.21.5): a snout on the head, horns a pixel
            // further forward.
            let head = r.pose([0.0, 4.0, -8.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-4.0, -4.0, -6.0], [8.0, 8.0, 6.0], 0.0, false);
            b.cube(&head, &t, 1.0, 33.0, [-3.0, 1.0, -7.0], [6.0, 3.0, 1.0], 0.0, false);
            b.cube(&head, &t, 22.0, 0.0, [-5.0, -5.0, -5.0], [1.0, 3.0, 1.0], 0.0, false);
            b.cube(&head, &t, 22.0, 0.0, [4.0, -5.0, -5.0], [1.0, 3.0, 1.0], 0.0, false);
            let body = r.pose([0.0, 5.0, 2.0], [HALF, 0.0, 0.0]);
            b.cube(&body, &t, 18.0, 4.0, [-6.0, -10.0, -7.0], [12.0, 18.0, 10.0], 0.0, false);
            b.cube(&body, &t, 52.0, 0.0, [-2.0, 2.0, -8.0], [4.0, 6.0, 1.0], 0.0, false);
            legs(b, r, &t, 0.0, 16.0, 12.0, [(-4.0, 7.0), (4.0, 7.0), (-4.0, -6.0), (4.0, -6.0)], 12.0, [4.0, 4.0]);
            // A grown mooshroom's mushrooms: two on its back, one on its head, placed as the
            // game's mushroom layer places them (in the model's space, in blocks, y down).
            if id == "mooshroom" && !baby {
                let kind = variant(e, "Type").filter(|k| k == "brown").map_or("red", |_| "brown");
                if let Some(model) = crate::models::named_model(&format!("minecraft:block/{kind}_mushroom")) {
                    let space = r.s(16.0, 16.0, 16.0);
                    let block = |m: Mat, turn: f32| m.ry(turn * PI / 180.0).s(-1.0, -1.0, 1.0).t(-0.5, -0.5, -0.5);
                    let back = space.t(0.2, -0.35, 0.5);
                    b.model(&block(back, -48.0), &model, [1.0; 3]);
                    b.model(&block(back.ry(42.0 * PI / 180.0).t(0.1, 0.0, -0.6), -48.0), &model, [1.0; 3]);
                    b.model(&block(space.t(0.0, 4.0 / 16.0, -8.0 / 16.0).t(0.0, -0.7, -0.2), -78.0), &model, [1.0; 3]);
                }
            }
        }
        "bat" => {
            let t = Tex::new("entity/bat/bat", 32.0, 32.0);
            let body = r.pose([0.0, 17.0, 0.0], [0.0; 3]);
            b.cube(&body, &t, 0.0, 7.0, [-1.5, 0.0, -1.0], [3.0, 5.0, 2.0], 0.0, false);
            let head = r.pose([0.0, 17.0, 0.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-2.0, -3.0, -1.0], [4.0, 3.0, 2.0], 0.0, false);
            b.cube(&head.pose([-1.5, -2.0, 0.0], [0.0; 3]), &t, 1.0, 15.0, [-2.5, -4.0, 0.0], [3.0, 5.0, 0.0], 0.0, false);
            b.cube(&head.pose([1.1, -3.0, 0.0], [0.0; 3]), &t, 8.0, 15.0, [-0.1, -3.0, 0.0], [3.0, 5.0, 0.0], 0.0, false);
            // Wings spread, as it flies.
            let rw = body.pose([-1.5, 0.0, 0.0], [0.0, 0.2, 0.0]);
            b.cube(&rw, &t, 12.0, 0.0, [-2.0, -2.0, 0.0], [2.0, 7.0, 0.0], 0.0, false);
            b.cube(&rw.pose([-2.0, 0.0, 0.0], [0.0, 0.3, 0.0]), &t, 16.0, 0.0, [-6.0, -2.0, 0.0], [6.0, 8.0, 0.0], 0.0, false);
            let lw = body.pose([1.5, 0.0, 0.0], [0.0, -0.2, 0.0]);
            b.cube(&lw, &t, 12.0, 7.0, [0.0, -2.0, 0.0], [2.0, 7.0, 0.0], 0.0, false);
            b.cube(&lw.pose([2.0, 0.0, 0.0], [0.0, -0.3, 0.0]), &t, 16.0, 8.0, [0.0, -2.0, 0.0], [6.0, 8.0, 0.0], 0.0, false);
            b.cube(&body.pose([0.0, 5.0, 0.0], [0.0; 3]), &t, 16.0, 16.0, [-1.5, 0.0, 0.0], [3.0, 2.0, 0.0], 0.0, false);
        }
        "turtle" => {
            // (26.x babies have models of their own; until they are built here, a small adult.)
            let t = Tex::new("entity/turtle/turtle", 128.0, 64.0);
            b.cube(&r.pose([0.0, 19.0, -10.0], [0.0; 3]), &t, 3.0, 0.0, [-3.0, -1.0, -3.0], [6.0, 5.0, 6.0], 0.0, false);
            let body = r.pose([0.0, 11.0, -10.0], [HALF, 0.0, 0.0]);
            b.cube(&body, &t, 7.0, 37.0, [-9.5, 3.0, -10.0], [19.0, 20.0, 6.0], 0.0, false);
            b.cube(&body, &t, 31.0, 1.0, [-5.5, 3.0, -13.0], [11.0, 18.0, 3.0], 0.0, false);
            b.cube(&r.pose([-3.5, 22.0, 11.0], [0.0; 3]), &t, 1.0, 23.0, [-2.0, 0.0, 0.0], [4.0, 1.0, 10.0], 0.0, false);
            b.cube(&r.pose([3.5, 22.0, 11.0], [0.0; 3]), &t, 1.0, 12.0, [-2.0, 0.0, 0.0], [4.0, 1.0, 10.0], 0.0, false);
            b.cube(&r.pose([-5.0, 21.0, -4.0], [0.0; 3]), &t, 27.0, 30.0, [-13.0, 0.0, -2.0], [13.0, 1.0, 5.0], 0.0, false);
            b.cube(&r.pose([5.0, 21.0, -4.0], [0.0; 3]), &t, 27.0, 24.0, [0.0, 0.0, -2.0], [13.0, 1.0, 5.0], 0.0, false);
        }
        "dolphin" => {
            let t = Tex::new("entity/dolphin/dolphin", 64.0, 64.0);
            let body = r.pose([0.0, 22.0, -5.0], [0.0; 3]);
            b.cube(&body, &t, 22.0, 0.0, [-4.0, -7.0, 0.0], [8.0, 7.0, 13.0], 0.0, false);
            b.cube(&body.pose([0.0; 3], [PI / 3.0, 0.0, 0.0]), &t, 51.0, 0.0, [-0.5, 0.0, 8.0], [1.0, 4.0, 5.0], 0.0, false);
            b.cube(&body.pose([2.0, -2.0, 4.0], [PI / 3.0, 0.0, 2.0 * PI / 3.0]), &t, 48.0, 20.0, [-0.5, -4.0, 0.0], [1.0, 4.0, 7.0], 0.0, true);
            b.cube(&body.pose([-2.0, -2.0, 4.0], [PI / 3.0, 0.0, -2.0 * PI / 3.0]), &t, 48.0, 20.0, [-0.5, -4.0, 0.0], [1.0, 4.0, 7.0], 0.0, false);
            let tail = body.pose([0.0, -2.5, 11.0], [-0.104_72, 0.0, 0.0]);
            b.cube(&tail, &t, 0.0, 19.0, [-2.0, -2.5, 0.0], [4.0, 5.0, 11.0], 0.0, false);
            b.cube(&tail.pose([0.0, 0.0, 9.0], [0.0; 3]), &t, 19.0, 20.0, [-5.0, -0.5, 0.0], [10.0, 1.0, 6.0], 0.0, false);
            let head = body.pose([0.0, -4.0, -3.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-4.0, -3.0, -3.0], [8.0, 7.0, 6.0], 0.0, false);
            b.cube(&head, &t, 0.0, 13.0, [-1.0, 2.0, -7.0], [2.0, 2.0, 4.0], 0.0, false);
        }
        "cod" => {
            let t = Tex::new("entity/fish/cod", 32.0, 32.0);
            b.cube(&r.pose([0.0, 22.0, 0.0], [0.0; 3]), &t, 0.0, 0.0, [-1.0, -2.0, 0.0], [2.0, 4.0, 7.0], 0.0, false);
            b.cube(&r.pose([0.0, 22.0, 0.0], [0.0; 3]), &t, 11.0, 0.0, [-1.0, -2.0, -3.0], [2.0, 4.0, 3.0], 0.0, false);
            b.cube(&r.pose([0.0, 22.0, -3.0], [0.0; 3]), &t, 0.0, 0.0, [-1.0, -2.0, -1.0], [2.0, 3.0, 1.0], 0.0, false);
            b.cube(&r.pose([-1.0, 23.0, 0.0], [0.0, 0.0, -PI / 4.0]), &t, 22.0, 1.0, [-2.0, 0.0, -1.0], [2.0, 0.0, 2.0], 0.0, false);
            b.cube(&r.pose([1.0, 23.0, 0.0], [0.0, 0.0, PI / 4.0]), &t, 22.0, 4.0, [0.0, 0.0, -1.0], [2.0, 0.0, 2.0], 0.0, false);
            b.cube(&r.pose([0.0, 22.0, 7.0], [0.0; 3]), &t, 22.0, 3.0, [0.0, -2.0, 0.0], [0.0, 4.0, 4.0], 0.0, false);
            b.cube(&r.pose([0.0, 20.0, 0.0], [0.0; 3]), &t, 20.0, -6.0, [0.0, -1.0, -1.0], [0.0, 1.0, 6.0], 0.0, false);
        }
        "salmon" => {
            let t = Tex::new("entity/fish/salmon", 32.0, 32.0);
            let front = r.pose([0.0, 20.0, 0.0], [0.0; 3]);
            b.cube(&front, &t, 0.0, 0.0, [-1.5, -2.5, 0.0], [3.0, 5.0, 8.0], 0.0, false);
            b.cube(&front.pose([0.0, -4.5, 5.0], [0.0; 3]), &t, 2.0, 1.0, [0.0, 0.0, 0.0], [0.0, 2.0, 3.0], 0.0, false);
            let back = r.pose([0.0, 20.0, 8.0], [0.0; 3]);
            b.cube(&back, &t, 0.0, 13.0, [-1.5, -2.5, 0.0], [3.0, 5.0, 8.0], 0.0, false);
            b.cube(&back.pose([0.0, 0.0, 8.0], [0.0; 3]), &t, 20.0, 10.0, [0.0, -2.5, 0.0], [0.0, 5.0, 6.0], 0.0, false);
            b.cube(&back.pose([0.0, -4.5, -1.0], [0.0; 3]), &t, 0.0, 2.0, [0.0, 0.0, 0.0], [0.0, 2.0, 4.0], 0.0, false);
            b.cube(&r.pose([0.0, 20.0, 0.0], [0.0; 3]), &t, 22.0, 0.0, [-1.0, -2.0, -3.0], [2.0, 4.0, 3.0], 0.0, false);
            b.cube(&r.pose([-1.5, 21.5, 0.0], [0.0, 0.0, -PI / 4.0]), &t, -4.0, 0.0, [-2.0, 0.0, 0.0], [2.0, 0.0, 2.0], 0.0, false);
            b.cube(&r.pose([1.5, 21.5, 0.0], [0.0, 0.0, PI / 4.0]), &t, 0.0, 0.0, [0.0, 0.0, 0.0], [2.0, 0.0, 2.0], 0.0, false);
        }
        "pufferfish" => {
            let t = Tex::new("entity/fish/pufferfish", 32.0, 32.0);
            match e.get("PuffState").and_then(Tag::as_i64).unwrap_or(0) {
                0 => {
                    b.cube(&r.pose([0.0, 23.0, 0.0], [0.0; 3]), &t, 0.0, 27.0, [-1.5, -2.0, -1.5], [3.0, 2.0, 3.0], 0.0, false);
                    b.cube(&r.pose([0.0, 20.0, 0.0], [0.0; 3]), &t, 24.0, 6.0, [-1.5, 0.0, -1.5], [1.0, 1.0, 1.0], 0.0, false);
                    b.cube(&r.pose([0.0, 20.0, 0.0], [0.0; 3]), &t, 28.0, 6.0, [0.5, 0.0, -1.5], [1.0, 1.0, 1.0], 0.0, false);
                    b.cube(&r.pose([0.0, 22.0, 1.5], [0.0; 3]), &t, -3.0, 0.0, [-1.5, 0.0, 0.0], [3.0, 0.0, 3.0], 0.0, false);
                    b.cube(&r.pose([-1.5, 22.0, -1.5], [0.0; 3]), &t, 25.0, 0.0, [-1.0, 0.0, 0.0], [1.0, 0.0, 2.0], 0.0, false);
                    b.cube(&r.pose([1.5, 22.0, -1.5], [0.0; 3]), &t, 25.0, 0.0, [0.0, 0.0, 0.0], [1.0, 0.0, 2.0], 0.0, false);
                }
                _ => {
                    // Blown up: the body, its side fins and its spines all round.
                    b.cube(&r.pose([0.0, 22.0, 0.0], [0.0; 3]), &t, 0.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], 0.0, false);
                    b.cube(&r.pose([-4.0, 15.0, -2.0], [0.0; 3]), &t, 24.0, 0.0, [-2.0, 0.0, -1.0], [2.0, 1.0, 2.0], 0.0, false);
                    b.cube(&r.pose([4.0, 15.0, -2.0], [0.0; 3]), &t, 24.0, 3.0, [0.0, 0.0, -1.0], [2.0, 1.0, 2.0], 0.0, false);
                    let q = PI / 4.0;
                    for (pivot, rot, u, v, o, d) in [
                        ([0.0, 14.0, -4.0], [q, 0.0, 0.0], 15.0, 17.0, [-4.0, -1.0, 0.0], [8.0, 1.0, 0.0]),
                        ([0.0, 14.0, 0.0], [0.0; 3], 14.0, 16.0, [-4.0, -1.0, 0.0], [8.0, 1.0, 1.0]),
                        ([0.0, 14.0, 4.0], [-q, 0.0, 0.0], 23.0, 18.0, [-4.0, -1.0, 0.0], [8.0, 1.0, 0.0]),
                        ([-4.0, 22.0, -4.0], [0.0, -q, 0.0], 5.0, 17.0, [-1.0, -8.0, 0.0], [1.0, 8.0, 0.0]),
                        ([4.0, 22.0, -4.0], [0.0, q, 0.0], 1.0, 17.0, [0.0, -8.0, 0.0], [1.0, 8.0, 0.0]),
                        ([0.0, 22.0, -4.0], [-q, 0.0, 0.0], 15.0, 20.0, [-4.0, 0.0, 0.0], [8.0, 1.0, 0.0]),
                        ([0.0, 22.0, 0.0], [0.0; 3], 15.0, 20.0, [-4.0, 0.0, 0.0], [8.0, 1.0, 0.0]),
                        ([0.0, 22.0, 4.0], [q, 0.0, 0.0], 15.0, 20.0, [-4.0, 0.0, 0.0], [8.0, 1.0, 0.0]),
                        ([-4.0, 22.0, 4.0], [0.0, q, 0.0], 9.0, 17.0, [-1.0, -8.0, 0.0], [1.0, 8.0, 0.0]),
                        ([4.0, 22.0, 4.0], [0.0, -q, 0.0], 9.0, 17.0, [0.0, -8.0, 0.0], [1.0, 8.0, 0.0]),
                    ] {
                        b.cube(&r.pose(pivot, rot), &t, u, v, o, d, 0.0, false);
                    }
                }
            }
        }
        "tropical_fish" => {
            // Its shape, pattern and two colors, packed in one number.
            let v = e.get("Variant").and_then(Tag::as_i64).unwrap_or(0);
            let (shape, pattern, base, over) = (v & 0xFF, (v >> 8) & 0xFF, ((v >> 16) & 0xFF) as usize, ((v >> 24) & 0xFF) as usize);
            const DYES: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
            let letter = if shape == 0 { "a" } else { "b" };
            let layers = [(format!("entity/fish/tropical_{letter}"), DYES[base.min(15)]), (format!("entity/fish/tropical_{letter}_pattern_{}", pattern.min(5) + 1), DYES[over.min(15)])];
            for (k, (file, dye)) in layers.iter().enumerate() {
                let t = Tex::new(file, 32.0, 32.0);
                let g = k as f32 * 0.01;
                b.tint = crate::entities::dye(dye);
                if shape == 0 {
                    b.cube(&r.pose([0.0, 22.0, 0.0], [0.0; 3]), &t, 0.0, 0.0, [-1.0, -1.5, -3.0], [2.0, 3.0, 6.0], g, false);
                    b.cube(&r.pose([0.0, 22.0, 3.0], [0.0; 3]), &t, 22.0, -6.0, [0.0, -1.5, 0.0], [0.0, 3.0, 6.0], g, false);
                    b.cube(&r.pose([-1.0, 22.5, 0.0], [0.0, PI / 4.0, 0.0]), &t, 2.0, 16.0, [-2.0, -1.0, 0.0], [2.0, 2.0, 0.0], g, false);
                    b.cube(&r.pose([1.0, 22.5, 0.0], [0.0, -PI / 4.0, 0.0]), &t, 2.0, 12.0, [0.0, -1.0, 0.0], [2.0, 2.0, 0.0], g, false);
                    b.cube(&r.pose([0.0, 20.5, -3.0], [0.0; 3]), &t, 10.0, -5.0, [0.0, -3.0, 0.0], [0.0, 3.0, 6.0], g, false);
                } else {
                    b.cube(&r.pose([0.0, 19.0, 0.0], [0.0; 3]), &t, 0.0, 20.0, [-1.0, -3.0, -3.0], [2.0, 6.0, 6.0], g, false);
                    b.cube(&r.pose([0.0, 19.0, 3.0], [0.0; 3]), &t, 21.0, 16.0, [0.0, -3.0, 0.0], [0.0, 6.0, 5.0], g, false);
                    b.cube(&r.pose([-1.0, 20.0, 0.0], [0.0, PI / 4.0, 0.0]), &t, 2.0, 16.0, [-2.0, 0.0, 0.0], [2.0, 2.0, 0.0], g, false);
                    b.cube(&r.pose([1.0, 20.0, 0.0], [0.0, -PI / 4.0, 0.0]), &t, 2.0, 12.0, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0], g, false);
                    b.cube(&r.pose([0.0, 16.0, -3.0], [0.0; 3]), &t, 20.0, 11.0, [0.0, -4.0, 0.0], [0.0, 4.0, 6.0], g, false);
                    b.cube(&r.pose([0.0, 22.0, -3.0], [0.0; 3]), &t, 20.0, 21.0, [0.0, 0.0, 0.0], [0.0, 4.0, 6.0], g, false);
                }
            }
            b.tint = [1.0; 3];
        }
        "axolotl" => {
            let kind = ["lucy", "wild", "gold", "cyan", "blue"][e.get("Variant").and_then(Tag::as_i64).unwrap_or(0).clamp(0, 4) as usize];
            let t = Tex::new(&format!("entity/axolotl/axolotl_{kind}"), 64.0, 64.0);
            let body = r.pose([0.0, 20.0, 5.0], [0.0; 3]);
            b.cube(&body, &t, 0.0, 11.0, [-4.0, -2.0, -9.0], [8.0, 4.0, 10.0], 0.0, false);
            b.cube(&body, &t, 2.0, 17.0, [0.0, -3.0, -8.0], [0.0, 5.0, 9.0], 0.0, false);
            let head = body.pose([0.0, 0.0, -9.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 1.0, [-4.0, -3.0, -5.0], [8.0, 5.0, 5.0], 0.0, false);
            b.cube(&head.pose([0.0, -3.0, -1.0], [0.0; 3]), &t, 3.0, 37.0, [-4.0, -3.0, 0.0], [8.0, 3.0, 0.0], 0.0, false);
            b.cube(&head.pose([-4.0, 0.0, -1.0], [0.0; 3]), &t, 0.0, 40.0, [-3.0, -5.0, 0.0], [3.0, 7.0, 0.0], 0.0, false);
            b.cube(&head.pose([4.0, 0.0, -1.0], [0.0; 3]), &t, 11.0, 40.0, [0.0, -5.0, 0.0], [3.0, 7.0, 0.0], 0.0, false);
            for (x, z, o) in [(-3.5, -1.0, -1.0), (3.5, -1.0, -2.0), (-3.5, -8.0, -1.0), (3.5, -8.0, -2.0)] {
                b.cube(&body.pose([x, 1.0, z], [0.0; 3]), &t, 2.0, 13.0, [o, 0.0, 0.0], [3.0, 5.0, 0.0], 0.0, false);
            }
            b.cube(&body.pose([0.0, 0.0, 1.0], [0.0; 3]), &t, 2.0, 19.0, [0.0, -3.0, 0.0], [0.0, 5.0, 12.0], 0.0, false);
        }
        "frog" => {
            let kind = variant(e, "variant").unwrap_or_else(|| "temperate".into());
            let t = Tex::new(&format!("entity/frog/frog_{kind}"), 48.0, 48.0);
            let root = r.pose([0.0, 24.0, 0.0], [0.0; 3]);
            let body = root.pose([0.0, -2.0, 4.0], [0.0; 3]);
            b.cube(&body, &t, 3.0, 1.0, [-3.5, -2.0, -8.0], [7.0, 3.0, 9.0], 0.0, false);
            b.cube(&body, &t, 23.0, 22.0, [-3.5, -1.0, -8.0], [7.0, 0.0, 9.0], 0.0, false);
            let head = body.pose([0.0, -2.0, -1.0], [0.0; 3]);
            b.cube(&head, &t, 23.0, 13.0, [-3.5, -1.0, -7.0], [7.0, 0.0, 9.0], 0.0, false);
            b.cube(&head, &t, 0.0, 13.0, [-3.5, -2.0, -7.0], [7.0, 3.0, 9.0], 0.0, false);
            let eyes = head.pose([-0.5, 0.0, 2.0], [0.0; 3]);
            b.cube(&eyes.pose([-1.5, -3.0, -6.5], [0.0; 3]), &t, 0.0, 0.0, [-1.5, -1.0, -1.5], [3.0, 2.0, 3.0], 0.0, false);
            b.cube(&eyes.pose([2.5, -3.0, -6.5], [0.0; 3]), &t, 0.0, 5.0, [-1.5, -1.0, -1.5], [3.0, 2.0, 3.0], 0.0, false);
            let la = body.pose([4.0, -1.0, -6.5], [0.0; 3]);
            b.cube(&la, &t, 0.0, 32.0, [-1.0, 0.0, -1.0], [2.0, 3.0, 3.0], 0.0, false);
            b.cube(&la.pose([0.0, 3.0, -1.0], [0.0; 3]), &t, 18.0, 40.0, [-4.0, 0.01, -4.0], [8.0, 0.0, 8.0], 0.0, false);
            let ra = body.pose([-4.0, -1.0, -6.5], [0.0; 3]);
            b.cube(&ra, &t, 0.0, 38.0, [-1.0, 0.0, -1.0], [2.0, 3.0, 3.0], 0.0, false);
            b.cube(&ra.pose([0.0, 3.0, 0.0], [0.0; 3]), &t, 2.0, 40.0, [-4.0, 0.01, -5.0], [8.0, 0.0, 8.0], 0.0, false);
            let ll = root.pose([3.5, -3.0, 4.0], [0.0; 3]);
            b.cube(&ll, &t, 14.0, 25.0, [-1.0, 0.0, -2.0], [3.0, 3.0, 4.0], 0.0, false);
            b.cube(&ll.pose([2.0, 3.0, 0.0], [0.0; 3]), &t, 2.0, 32.0, [-4.0, 0.01, -4.0], [8.0, 0.0, 8.0], 0.0, false);
            let rl = root.pose([-3.5, -3.0, 4.0], [0.0; 3]);
            b.cube(&rl, &t, 0.0, 25.0, [-2.0, 0.0, -2.0], [3.0, 3.0, 4.0], 0.0, false);
            b.cube(&rl.pose([-2.0, 3.0, 0.0], [0.0; 3]), &t, 18.0, 32.0, [-4.0, 0.01, -4.0], [8.0, 0.0, 8.0], 0.0, false);
        }
        "tadpole" => {
            let t = Tex::new("entity/tadpole/tadpole", 16.0, 16.0);
            b.cube(&r.pose([0.0, 22.0, -3.0], [0.0; 3]), &t, 0.0, 0.0, [-1.5, -1.0, 0.0], [3.0, 2.0, 3.0], 0.0, false);
            b.cube(&r.pose([0.0, 22.0, 0.0], [0.0; 3]), &t, 0.0, 0.0, [0.0, -1.0, 0.0], [0.0, 2.0, 7.0], 0.0, false);
        }
        "ghast" | "happy_ghast" => {
            // (26.x's pictures are twice as fine; laid out as before.)
            let (file, h) = if id == "ghast" { ("entity/ghast/ghast", 32.0) } else { ("entity/ghast/happy_ghast", 64.0) };
            let t = Tex::new(file, 64.0, h);
            b.cube(&r.pose([0.0, 17.6, 0.0], [0.0; 3]), &t, 0.0, 0.0, [-8.0, -8.0, -8.0], [16.0, 16.0, 16.0], 0.0, false);
            for i in 0..9 {
                let fx = ((i % 3) as f32 - (i / 3 % 2) as f32 * 0.5 + 0.25) / 2.0 * 2.0 - 1.0;
                let fz = (i / 3) as f32 / 2.0 * 2.0 - 1.0;
                let len = 8.0 + ((i * 5 + 3) % 7) as f32;
                b.cube(&r.pose([fx * 5.0, 24.6, fz * 5.0], [0.0; 3]), &t, 0.0, 0.0, [-1.0, 0.0, -1.0], [2.0, len, 2.0], 0.0, false);
            }
        }
        "blaze" => {
            let t = Tex::new("entity/blaze/blaze", 64.0, 32.0);
            b.cube(&r.pose([0.0; 3], [0.0; 3]), &t, 0.0, 0.0, [-4.0, -4.0, -4.0], [8.0, 8.0, 8.0], 0.0, false);
            // Its rods in three rings, each lower and narrower.
            for (ring, (y, radius, start)) in [(-2.0f32, 9.0f32, 0.0f32), (2.0, 7.0, PI / 4.0), (11.0, 5.0, 0.47)].into_iter().enumerate() {
                for k in 0..4 {
                    let a = start + k as f32 * HALF;
                    let _ = ring;
                    b.cube(&r.pose([a.cos() * radius, y, a.sin() * radius], [0.0; 3]), &t, 0.0, 16.0, [0.0, 0.0, 0.0], [2.0, 8.0, 2.0], 0.0, false);
                }
            }
        }
        "guardian" | "elder_guardian" => {
            let t = Tex::new(if id == "guardian" { "entity/guardian/guardian" } else { "entity/guardian/guardian_elder" }, 64.0, 64.0);
            let head = r.pose([0.0; 3], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-6.0, 10.0, -8.0], [12.0, 12.0, 16.0], 0.0, false);
            b.cube(&head, &t, 0.0, 28.0, [-8.0, 10.0, -6.0], [2.0, 12.0, 12.0], 0.0, false);
            b.cube(&head, &t, 0.0, 28.0, [6.0, 10.0, -6.0], [2.0, 12.0, 12.0], 0.0, true);
            b.cube(&head, &t, 16.0, 40.0, [-6.0, 8.0, -6.0], [12.0, 2.0, 12.0], 0.0, false);
            b.cube(&head, &t, 16.0, 40.0, [-6.0, 22.0, -6.0], [12.0, 2.0, 12.0], 0.0, false);
            b.cube(&head.pose([0.0, 0.0, -8.25], [0.0; 3]), &t, 8.0, 0.0, [-1.0, 15.0, 0.0], [2.0, 2.0, 1.0], 0.0, false);
            const X: [f32; 12] = [0.0, 0.0, 8.0, -8.0, -8.0, 8.0, 8.0, -8.0, 0.0, 0.0, 8.0, -8.0];
            const Y: [f32; 12] = [-8.0, -8.0, -8.0, -8.0, 0.0, 0.0, 0.0, 0.0, 8.0, 8.0, 8.0, 8.0];
            const Z: [f32; 12] = [8.0, -8.0, 0.0, 0.0, -8.0, -8.0, 8.0, 8.0, 8.0, -8.0, 0.0, 0.0];
            const RX: [f32; 12] = [1.75, 0.25, 0.0, 0.0, 0.5, 0.5, 0.5, 0.5, 1.25, 0.75, 0.0, 0.0];
            const RY: [f32; 12] = [0.0, 0.0, 0.0, 0.0, 0.25, 1.75, 1.25, 0.75, 0.0, 0.0, 0.0, 0.0];
            const RZ: [f32; 12] = [0.0, 0.0, 0.25, 1.75, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.75, 1.25];
            for i in 0..12 {
                b.cube(&head.pose([X[i], 16.0 + Y[i], Z[i]], [RX[i] * PI, RY[i] * PI, RZ[i] * PI]), &t, 0.0, 0.0, [-1.0, -4.5, -1.0], [2.0, 9.0, 2.0], 0.0, false);
            }
            b.cube(&head, &t, 40.0, 0.0, [-2.0, 14.0, 7.0], [4.0, 4.0, 8.0], 0.0, false);
            let t1 = head.pose([-1.5, 0.5, 14.0], [0.0; 3]);
            b.cube(&t1, &t, 0.0, 54.0, [0.0, 14.0, 0.0], [3.0, 3.0, 7.0], 0.0, false);
            let t2 = t1.pose([0.5, 0.5, 6.0], [0.0; 3]);
            b.cube(&t2, &t, 41.0, 32.0, [0.0, 14.0, 0.0], [2.0, 2.0, 6.0], 0.0, false);
            b.cube(&t2, &t, 25.0, 19.0, [1.0, 10.5, 3.0], [1.0, 9.0, 9.0], 0.0, false);
        }
        "phantom" => {
            let t = Tex::new("entity/phantom/phantom", 64.0, 64.0);
            let body = r.pose([0.0, 16.0, 0.0], [-0.1, 0.0, 0.0]);
            b.cube(&body, &t, 0.0, 8.0, [-3.0, -2.0, -8.0], [5.0, 3.0, 9.0], 0.0, false);
            let tail = body.pose([0.0, -2.0, 1.0], [0.0; 3]);
            b.cube(&tail, &t, 3.0, 20.0, [-2.0, 0.0, 0.0], [3.0, 2.0, 6.0], 0.0, false);
            b.cube(&tail.pose([0.0, 0.5, 6.0], [0.0; 3]), &t, 4.0, 29.0, [-1.0, 0.0, 0.0], [1.0, 1.0, 6.0], 0.0, false);
            let lw = body.pose([2.0, -2.0, -8.0], [0.0, 0.0, 0.1]);
            b.cube(&lw, &t, 23.0, 12.0, [0.0, 0.0, 0.0], [6.0, 2.0, 9.0], 0.0, false);
            b.cube(&lw.pose([6.0, 0.0, 0.0], [0.0, 0.0, 0.1]), &t, 16.0, 24.0, [0.0, 0.0, 0.0], [13.0, 1.0, 9.0], 0.0, false);
            let rw = body.pose([-3.0, -2.0, -8.0], [0.0, 0.0, -0.1]);
            b.cube(&rw, &t, 23.0, 12.0, [-6.0, 0.0, 0.0], [6.0, 2.0, 9.0], 0.0, true);
            b.cube(&rw.pose([-6.0, 0.0, 0.0], [0.0, 0.0, -0.1]), &t, 16.0, 24.0, [-13.0, 0.0, 0.0], [13.0, 1.0, 9.0], 0.0, true);
            b.cube(&body.pose([0.0, 1.0, -7.0], [0.2, 0.0, 0.0]), &t, 0.0, 0.0, [-4.0, -2.0, -5.0], [7.0, 3.0, 5.0], 0.0, false);
        }
        "armadillo" => {
            let t = Tex::new("entity/armadillo/armadillo", 64.0, 64.0);
            let body = r.pose([0.0, 21.0, 4.0], [0.0; 3]);
            b.cube(&body, &t, 0.0, 20.0, [-4.0, -7.0, -10.0], [8.0, 8.0, 12.0], 0.3, false);
            b.cube(&body, &t, 0.0, 40.0, [-4.0, -7.0, -10.0], [8.0, 8.0, 12.0], 0.0, false);
            b.cube(&body.pose([0.0, -3.0, 1.0], [0.5061, 0.0, 0.0]), &t, 44.0, 53.0, [-0.5, -0.0865, 0.0933], [1.0, 6.0, 1.0], 0.0, false);
            let head = body.pose([0.0, -2.0, -11.0], [0.0; 3]);
            b.cube(&head.pose([0.0; 3], [-std::f32::consts::FRAC_PI_8, 0.0, 0.0]), &t, 43.0, 15.0, [-1.5, -1.0, -1.0], [3.0, 5.0, 2.0], 0.0, false);
            b.cube(&head.pose([-1.0, -1.0, 0.0], [0.1886, -0.3864, -0.0718]), &t, 43.0, 10.0, [-2.0, -3.0, 0.0], [2.0, 5.0, 0.0], 0.0, false);
            b.cube(&head.pose([1.0, -2.0, 0.0], [0.1886, 0.3864, 0.0718]), &t, 47.0, 10.0, [0.0, -3.0, 0.0], [2.0, 5.0, 0.0], 0.0, false);
            for (x, z, u, v) in [(-2.0, 4.0, 51.0, 31.0), (2.0, 4.0, 42.0, 31.0), (-2.0, -4.0, 51.0, 43.0), (2.0, -4.0, 42.0, 43.0)] {
                b.cube(&r.pose([x, 21.0, z], [0.0; 3]), &t, u, v, [-1.0, 0.0, -1.0], [2.0, 3.0, 2.0], 0.0, false);
            }
        }
        "warden" => {
            let t = Tex::new("entity/warden/warden", 128.0, 128.0);
            let bone = r.pose([0.0, 24.0, 0.0], [0.0; 3]);
            let body = bone.pose([0.0, -21.0, 0.0], [0.0; 3]);
            b.cube(&body, &t, 0.0, 0.0, [-9.0, -13.0, -4.0], [18.0, 21.0, 11.0], 0.0, false);
            b.cube(&body.pose([-7.0, -2.0, -4.0], [0.0; 3]), &t, 90.0, 11.0, [-2.0, -11.0, -0.1], [9.0, 21.0, 0.0], 0.0, false);
            b.cube(&body.pose([7.0, -2.0, -4.0], [0.0; 3]), &t, 90.0, 11.0, [-7.0, -11.0, -0.1], [9.0, 21.0, 0.0], 0.0, true);
            let head = body.pose([0.0, -13.0, 0.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 32.0, [-8.0, -16.0, -5.0], [16.0, 16.0, 10.0], 0.0, false);
            b.cube(&head.pose([-8.0, -12.0, 0.0], [0.0; 3]), &t, 52.0, 32.0, [-16.0, -13.0, 0.0], [16.0, 16.0, 0.0], 0.0, false);
            b.cube(&head.pose([8.0, -12.0, 0.0], [0.0; 3]), &t, 58.0, 0.0, [0.0, -13.0, 0.0], [16.0, 16.0, 0.0], 0.0, false);
            b.cube(&body.pose([-13.0, -13.0, 1.0], [0.0; 3]), &t, 44.0, 50.0, [-4.0, 0.0, -4.0], [8.0, 28.0, 8.0], 0.0, false);
            b.cube(&body.pose([13.0, -13.0, 1.0], [0.0; 3]), &t, 0.0, 58.0, [-4.0, 0.0, -4.0], [8.0, 28.0, 8.0], 0.0, false);
            b.cube(&bone.pose([-5.9, -13.0, 0.0], [0.0; 3]), &t, 76.0, 48.0, [-3.1, 0.0, -3.0], [6.0, 13.0, 6.0], 0.0, false);
            b.cube(&bone.pose([5.9, -13.0, 0.0], [0.0; 3]), &t, 76.0, 76.0, [-2.9, 0.0, -3.0], [6.0, 13.0, 6.0], 0.0, false);
        }
        "breeze" => {
            let t = Tex::new("entity/breeze/breeze", 32.0, 32.0);
            let head = r.pose([0.0, 3.0, 0.0], [0.0; 3]);
            b.cube(&head, &t, 4.0, 24.0, [-5.0, -5.0, -4.2], [10.0, 3.0, 4.0], 0.0, false);
            b.cube(&head, &t, 0.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], 0.0, false);
            let rods = r.pose([0.0, 8.0, 0.0], [0.0; 3]);
            b.cube(&rods.pose([2.5981, -3.0, 1.5], [-2.7489, -std::f32::consts::FRAC_PI_3, PI]), &t, 0.0, 17.0, [-1.0, 0.0, -3.0], [2.0, 8.0, 2.0], 0.0, false);
            b.cube(&rods.pose([-2.5981, -3.0, 1.5], [-2.7489, std::f32::consts::FRAC_PI_3, PI]), &t, 0.0, 17.0, [-1.0, 0.0, -3.0], [2.0, 8.0, 2.0], 0.0, false);
            b.cube(&rods.pose([0.0, -3.0, -3.0], [std::f32::consts::FRAC_PI_8, 0.0, 0.0]), &t, 0.0, 17.0, [-1.0, 0.0, -3.0], [2.0, 8.0, 2.0], 0.0, false);
        }
        "hoglin" | "zoglin" => {
            let t = Tex::new(&format!("entity/hoglin/{id}"), 128.0, 64.0);
            let body = r.pose([0.0, 7.0, 0.0], [0.0; 3]);
            b.cube(&body, &t, 1.0, 1.0, [-8.0, -7.0, -13.0], [16.0, 14.0, 26.0], 0.0, false);
            b.cube(&body.pose([0.0, -14.0, -5.0], [0.0; 3]), &t, 90.0, 33.0, [0.0, 0.0, -9.0], [0.0, 10.0, 19.0], 0.001, false);
            let head = r.pose([0.0, 2.0, -12.0], [0.8727, 0.0, 0.0]);
            b.cube(&head, &t, 61.0, 1.0, [-7.0, -3.0, -19.0], [14.0, 6.0, 19.0], 0.0, false);
            b.cube(&head.pose([-6.0, -2.0, -3.0], [0.0, 0.0, -0.6981]), &t, 1.0, 1.0, [-6.0, -1.0, -2.0], [6.0, 1.0, 4.0], 0.0, false);
            b.cube(&head.pose([6.0, -2.0, -3.0], [0.0, 0.0, 0.6981]), &t, 1.0, 6.0, [0.0, -1.0, -2.0], [6.0, 1.0, 4.0], 0.0, false);
            b.cube(&head.pose([-7.0, 2.0, -12.0], [0.0; 3]), &t, 10.0, 13.0, [-1.0, -11.0, -1.0], [2.0, 11.0, 2.0], 0.0, false);
            b.cube(&head.pose([7.0, 2.0, -12.0], [0.0; 3]), &t, 1.0, 13.0, [-1.0, -11.0, -1.0], [2.0, 11.0, 2.0], 0.0, false);
            b.cube(&r.pose([-4.0, 10.0, -8.5], [0.0; 3]), &t, 66.0, 42.0, [-3.0, 0.0, -3.0], [6.0, 14.0, 6.0], 0.0, false);
            b.cube(&r.pose([4.0, 10.0, -8.5], [0.0; 3]), &t, 41.0, 42.0, [-3.0, 0.0, -3.0], [6.0, 14.0, 6.0], 0.0, false);
            b.cube(&r.pose([-5.0, 13.0, 10.0], [0.0; 3]), &t, 21.0, 45.0, [-2.5, 0.0, -2.5], [5.0, 11.0, 5.0], 0.0, false);
            b.cube(&r.pose([5.0, 13.0, 10.0], [0.0; 3]), &t, 0.0, 45.0, [-2.5, 0.0, -2.5], [5.0, 11.0, 5.0], 0.0, false);
        }
        "strider" => {
            let cold = e.get("Health").is_none();
            let _ = cold;
            let t = Tex::new("entity/strider/strider", 64.0, 128.0);
            let body = r.pose([0.0, 1.0, 0.0], [0.0; 3]);
            b.cube(&body, &t, 0.0, 0.0, [-8.0, -6.0, -8.0], [16.0, 14.0, 16.0], 0.0, false);
            b.cube(&r.pose([-4.0, 8.0, 0.0], [0.0; 3]), &t, 0.0, 32.0, [-2.0, 0.0, -2.0], [4.0, 16.0, 4.0], 0.0, false);
            b.cube(&r.pose([4.0, 8.0, 0.0], [0.0; 3]), &t, 0.0, 55.0, [-2.0, 0.0, -2.0], [4.0, 16.0, 4.0], 0.0, false);
            for (pivot, z, v, left) in [
                ([-8.0, 4.0, -8.0], -1.2217, 65.0, false),
                ([-8.0, -1.0, -8.0], -1.1345, 49.0, false),
                ([-8.0, -5.0, -8.0], -0.8727, 33.0, false),
                ([8.0, -6.0, -8.0], 0.8727, 33.0, true),
                ([8.0, -2.0, -8.0], std::f32::consts::FRAC_PI_3, 49.0, true),
                ([8.0, 3.0, -8.0], 1.2217, 65.0, true),
            ] {
                let o = if left { [0.0, 0.0, 0.0] } else { [-12.0, 0.0, 0.0] };
                b.cube(&body.pose(pivot, [0.0, 0.0, z]), &t, 16.0, v, o, [12.0, 0.0, 16.0], 0.0, !left);
            }
        }
        "wither" => {
            let t = Tex::new("entity/wither/wither", 64.0, 64.0);
            b.cube(&r.pose([0.0; 3], [0.0; 3]), &t, 0.0, 16.0, [-10.0, 3.9, -0.5], [20.0, 3.0, 3.0], 0.0, false);
            let rib = r.pose([-2.0, 6.9, -0.5], [0.2042, 0.0, 0.0]);
            b.cube(&rib, &t, 0.0, 22.0, [0.0, 0.0, 0.0], [3.0, 10.0, 3.0], 0.0, false);
            for y in [1.5, 4.0, 6.5] {
                b.cube(&rib, &t, 24.0, 22.0, [-4.0, y, 0.5], [11.0, 2.0, 2.0], 0.0, false);
            }
            let a: f32 = 0.2042;
            b.cube(&r.pose([-2.0, 6.9 + a.cos() * 10.0, -0.5 + a.sin() * 10.0], [0.83, 0.0, 0.0]), &t, 12.0, 22.0, [0.0, 0.0, 0.0], [3.0, 6.0, 3.0], 0.0, false);
            b.cube(&r.pose([0.0; 3], [0.0; 3]), &t, 0.0, 0.0, [-4.0, -4.0, -4.0], [8.0, 8.0, 8.0], 0.0, false);
            b.cube(&r.pose([-8.0, 4.0, 0.0], [0.0; 3]), &t, 32.0, 0.0, [-4.0, -4.0, -4.0], [6.0, 6.0, 6.0], 0.0, false);
            b.cube(&r.pose([10.0, 4.0, 0.0], [0.0; 3]), &t, 32.0, 0.0, [-4.0, -4.0, -4.0], [6.0, 6.0, 6.0], 0.0, false);
        }
        "vex" => {
            let t = Tex::new("entity/illager/vex", 32.0, 32.0);
            let root = r.pose([0.0, 20.0, 0.0], [0.0; 3]);
            b.cube(&root, &t, 0.0, 0.0, [-2.5, -5.0, -2.5], [5.0, 5.0, 5.0], 0.0, false);
            b.cube(&root, &t, 0.0, 10.0, [-1.5, 0.0, -1.0], [3.0, 4.0, 2.0], 0.0, false);
            b.cube(&root, &t, 0.0, 16.0, [-1.5, 1.0, -1.0], [3.0, 5.0, 2.0], -0.2, false);
            b.cube(&root.pose([-1.75, 0.25, 0.0], [0.0; 3]), &t, 23.0, 0.0, [-1.25, -0.5, -1.0], [2.0, 4.0, 2.0], -0.1, false);
            b.cube(&root.pose([1.75, 0.25, 0.0], [0.0; 3]), &t, 23.0, 6.0, [-0.75, -0.5, -1.0], [2.0, 4.0, 2.0], -0.1, false);
            b.cube(&root.pose([0.5, 1.0, 1.0], [0.0, 0.45, 0.0]), &t, 16.0, 14.0, [0.0, 0.0, 0.0], [0.0, 5.0, 8.0], 0.0, true);
            b.cube(&root.pose([-0.5, 1.0, 1.0], [0.0, -0.45, 0.0]), &t, 16.0, 14.0, [0.0, 0.0, 0.0], [0.0, 5.0, 8.0], 0.0, false);
        }
        "endermite" | "silverfish" => {
            // Segments nose to tail, each its own size, laid end to end.
            let (file, sizes, texs): (&str, &[[f32; 3]], &[[f32; 2]]) = if id == "endermite" {
                ("entity/endermite/endermite", &[[4.0, 3.0, 2.0], [6.0, 4.0, 5.0], [3.0, 3.0, 1.0], [1.0, 2.0, 1.0]], &[[0.0, 0.0], [0.0, 5.0], [0.0, 14.0], [0.0, 18.0]])
            } else {
                (
                    "entity/silverfish/silverfish",
                    &[[3.0, 2.0, 2.0], [4.0, 3.0, 2.0], [6.0, 4.0, 3.0], [3.0, 3.0, 3.0], [2.0, 2.0, 3.0], [2.0, 1.0, 2.0], [1.0, 1.0, 2.0]],
                    &[[0.0, 0.0], [0.0, 4.0], [0.0, 9.0], [0.0, 16.0], [0.0, 22.0], [11.0, 0.0], [13.0, 4.0]],
                )
            };
            let t = Tex::new(file, 64.0, 32.0);
            let mut z = -3.5f32;
            let mut zs = Vec::new();
            for (i, s) in sizes.iter().enumerate() {
                zs.push(z);
                b.cube(&r.pose([0.0, 24.0 - s[1], z], [0.0; 3]), &t, texs[i][0], texs[i][1], [-s[0] * 0.5, 0.0, -s[2] * 0.5], *s, 0.0, false);
                if i + 1 < sizes.len() {
                    z += (s[2] + sizes[i + 1][2]) * 0.5;
                }
            }
            if id == "silverfish" {
                // Its three ridges of scales.
                b.cube(&r.pose([0.0, 16.0, zs[2]], [0.0; 3]), &t, 20.0, 0.0, [-5.0, 0.0, -sizes[2][2] * 0.5], [10.0, 8.0, 3.0], 0.0, false);
                b.cube(&r.pose([0.0, 20.0, zs[4]], [0.0; 3]), &t, 20.0, 11.0, [-3.0, 0.0, -sizes[4][2] * 0.5], [6.0, 4.0, 3.0], 0.0, false);
                b.cube(&r.pose([0.0, 19.0, zs[1]], [0.0; 3]), &t, 20.0, 18.0, [-3.0, 0.0, -sizes[4][2] * 0.5], [6.0, 5.0, 2.0], 0.0, false);
            }
        }
        "shulker" => {
            let color = e.get("Color").and_then(Tag::as_i64).unwrap_or(16);
            const DYES: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
            let file = match DYES.get(color as usize) {
                Some(d) => format!("entity/shulker/shulker_{d}"),
                None => "entity/shulker/shulker".to_string(),
            };
            let t = Tex::new(&file, 64.0, 64.0);
            b.cube(&r.pose([0.0, 24.0, 0.0], [0.0; 3]), &t, 0.0, 0.0, [-8.0, -16.0, -8.0], [16.0, 12.0, 16.0], 0.0, false);
            b.cube(&r.pose([0.0, 24.0, 0.0], [0.0; 3]), &t, 0.0, 28.0, [-8.0, -8.0, -8.0], [16.0, 8.0, 16.0], 0.0, false);
            b.cube(&r.pose([0.0, 12.0, 0.0], [0.0; 3]), &t, 0.0, 52.0, [-3.0, 0.0, -3.0], [6.0, 6.0, 6.0], 0.0, false);
        }
        "sniffer" => {
            let t = Tex::new("entity/sniffer/sniffer", 192.0, 192.0);
            let body = r.pose([0.0, 5.0, 0.0], [0.0; 3]);
            b.cube(&body, &t, 62.0, 68.0, [-12.5, -14.0, -20.0], [25.0, 29.0, 40.0], 0.0, false);
            b.cube(&body, &t, 62.0, 0.0, [-12.5, -14.0, -20.0], [25.0, 24.0, 40.0], 0.5, false);
            for (x, z, u, v) in [(-7.5, -15.0, 32.0, 87.0), (-7.5, 0.0, 32.0, 105.0), (-7.5, 15.0, 32.0, 123.0), (6.5, -15.0, 0.0, 87.0), (6.5, 0.0, 0.0, 105.0), (6.5, 15.0, 0.0, 123.0)] {
                b.cube(&r.pose([x, 10.0, z], [0.0; 3]), &t, u, v, [-3.5, 4.0, -4.0], [7.0, 10.0, 8.0], 0.0, false);
            }
            let head = body.pose([0.0, 6.5, -19.48], [0.0; 3]);
            b.cube(&head, &t, 8.0, 15.0, [-6.5, -7.5, -11.5], [13.0, 18.0, 11.0], 0.0, false);
            b.cube(&head.pose([6.51, -7.5, -4.51], [0.0; 3]), &t, 2.0, 0.0, [0.0, 0.0, -3.0], [1.0, 19.0, 7.0], 0.0, false);
            b.cube(&head.pose([-6.51, -7.5, -4.51], [0.0; 3]), &t, 48.0, 0.0, [-1.0, 0.0, -3.0], [1.0, 19.0, 7.0], 0.0, false);
            b.cube(&head.pose([0.0, -4.5, -11.5], [0.0; 3]), &t, 10.0, 45.0, [-6.5, -2.0, -9.0], [13.0, 2.0, 9.0], 0.0, false);
            b.cube(&head.pose([0.0, 2.5, -12.5], [0.0; 3]), &t, 10.0, 57.0, [-6.5, -7.0, -8.0], [13.0, 12.0, 9.0], 0.0, false);
        }
        "ender_dragon" => {
            let t = Tex::new("entity/enderdragon/dragon", 256.0, 256.0);
            // Laid out straight, at rest: neck before the body, tail behind, wings spread.
            let body = r.pose([0.0, 4.0, 8.0], [0.0; 3]);
            b.cube(&body, &t, 0.0, 0.0, [-12.0, 0.0, -16.0], [24.0, 24.0, 64.0], 0.0, false);
            for z in [-10.0, 10.0, 30.0] {
                b.cube(&body, &t, 220.0, 53.0, [-1.0, -6.0, z], [2.0, 6.0, 12.0], 0.0, false);
            }
            let segment = |b: &mut Builder, at: Mat| {
                b.cube(&at, &t, 192.0, 104.0, [-5.0, -5.0, -5.0], [10.0, 10.0, 10.0], 0.0, false);
                b.cube(&at, &t, 48.0, 0.0, [-1.0, -9.0, -3.0], [2.0, 4.0, 6.0], 0.0, false);
            };
            for i in 0..5 {
                segment(b, r.pose([0.0, 10.0, -13.0 - i as f32 * 10.0], [0.0; 3]));
            }
            let head = r.pose([0.0, 10.0, -58.0], [0.0; 3]);
            b.cube(&head, &t, 176.0, 44.0, [-6.0, -1.0, -24.0], [12.0, 5.0, 16.0], 0.0, false);
            b.cube(&head, &t, 112.0, 30.0, [-8.0, -8.0, -10.0], [16.0, 16.0, 16.0], 0.0, false);
            b.cube(&head, &t, 112.0, 0.0, [-5.0, -3.0, -22.0], [2.0, 2.0, 4.0], 0.0, true);
            b.cube(&head, &t, 112.0, 0.0, [3.0, -3.0, -22.0], [2.0, 2.0, 4.0], 0.0, false);
            b.cube(&head, &t, 0.0, 0.0, [-5.0, -12.0, -4.0], [2.0, 4.0, 6.0], 0.0, true);
            b.cube(&head, &t, 0.0, 0.0, [3.0, -12.0, -4.0], [2.0, 4.0, 6.0], 0.0, false);
            b.cube(&head.pose([0.0, 4.0, -8.0], [0.0; 3]), &t, 176.0, 65.0, [-6.0, 0.0, -16.0], [12.0, 4.0, 16.0], 0.0, false);
            for i in 0..12 {
                segment(b, r.pose([0.0, 10.0, 60.0 + i as f32 * 10.0], [0.0; 3]));
            }
            for side in [-1.0f32, 1.0] {
                let mirror = side > 0.0;
                let wing = r.pose([12.0 * side, 5.0, 2.0], [0.0, 0.0, 0.0]);
                let (bone, skin) = if mirror { ([0.0, -4.0, -4.0], [0.0, 0.0, 2.0]) } else { ([-56.0, -4.0, -4.0], [-56.0, 0.0, 2.0]) };
                b.cube(&wing, &t, 112.0, 88.0, bone, [56.0, 8.0, 8.0], 0.0, mirror);
                b.cube(&wing, &t, -56.0, 88.0, skin, [56.0, 0.0, 56.0], 0.0, mirror);
                let tip = wing.pose([56.0 * side, 0.0, 0.0], [0.0; 3]);
                let (tb, ts) = if mirror { ([0.0, -2.0, -2.0], [0.0, 0.0, 2.0]) } else { ([-56.0, -2.0, -2.0], [-56.0, 0.0, 2.0]) };
                b.cube(&tip, &t, 112.0, 136.0, tb, [56.0, 4.0, 4.0], 0.0, mirror);
                b.cube(&tip, &t, -56.0, 144.0, ts, [56.0, 0.0, 56.0], 0.0, mirror);
                let front = r.pose([12.0 * side, 20.0, 2.0], [0.0; 3]);
                b.cube(&front, &t, 112.0, 104.0, [-4.0, -4.0, -4.0], [8.0, 24.0, 8.0], 0.0, mirror);
                let ft = front.pose([0.0, 20.0, -1.0], [0.0; 3]);
                b.cube(&ft, &t, 226.0, 138.0, [-3.0, -1.0, -3.0], [6.0, 24.0, 6.0], 0.0, mirror);
                b.cube(&ft.pose([0.0, 23.0, 0.0], [0.0; 3]), &t, 144.0, 104.0, [-4.0, 0.0, -12.0], [8.0, 4.0, 16.0], 0.0, mirror);
                let rear = r.pose([16.0 * side, 16.0, 42.0], [0.0; 3]);
                b.cube(&rear, &t, 0.0, 0.0, [-8.0, -4.0, -8.0], [16.0, 32.0, 16.0], 0.0, mirror);
                let rt = rear.pose([0.0, 32.0, -4.0], [0.0; 3]);
                b.cube(&rt, &t, 196.0, 0.0, [-6.0, -2.0, 0.0], [12.0, 32.0, 12.0], 0.0, mirror);
                b.cube(&rt.pose([0.0, 31.0, 4.0], [0.0; 3]), &t, 112.0, 0.0, [-9.0, 0.0, -20.0], [18.0, 6.0, 24.0], 0.0, mirror);
            }
        }
        "minecart" | "chest_minecart" | "furnace_minecart" | "tnt_minecart" | "hopper_minecart" | "spawner_minecart" | "command_block_minecart" => {
            let t = Tex::new("entity/minecart/minecart", 64.0, 32.0);
            // Its floor two pixels over the rail, as the game lifts it.
            b.cube(&r.pose([0.0, 22.0, 0.0], [HALF, 0.0, 0.0]), &t, 0.0, 10.0, [-10.0, -8.0, -1.0], [20.0, 16.0, 2.0], 0.0, false);
            for (pivot, yr) in [([-9.0, 22.0, 0.0], 3.0 * HALF), ([9.0, 22.0, 0.0], HALF)] {
                b.cube(&r.pose(pivot, [0.0, yr, 0.0]), &t, 0.0, 0.0, [-8.0, -9.0, -1.0], [16.0, 8.0, 2.0], 0.0, false);
            }
            for (pivot, yr) in [([0.0, 22.0, -7.0], PI), ([0.0, 22.0, 7.0], 0.0)] {
                b.cube(&r.pose(pivot, [0.0, yr, 0.0]), &t, 0.0, 0.0, [-8.0, -9.0, -1.0], [16.0, 8.0, 2.0], 0.0, false);
            }
            // What it carries, as its block, sitting in it.
            let block = match id {
                "furnace_minecart" => Some("furnace"),
                "tnt_minecart" => Some("tnt"),
                "hopper_minecart" => Some("hopper"),
                "spawner_minecart" => Some("spawner"),
                "command_block_minecart" => Some("command_block"),
                _ => None,
            };
            if let Some(name) = block {
                if let Some(model) = crate::models::named_model(&format!("minecraft:block/{name}")) {
                    // Three-quarters size, a little into the cart (as the game puts it), upright.
                    let m = local.ry((180.0 - yaw) * PI / 180.0).t(-0.375, 0.28, -0.375).s(0.75, 0.75, 0.75);
                    b.model(&m, &model, [1.0; 3]);
                }
            }
        }
        _ if id.ends_with("_boat") || id.ends_with("_raft") || id == "boat" || id == "chest_boat" => {
            let wood = if id == "boat" || id == "chest_boat" {
                ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry", "bamboo", "pale_oak"]
                    .get(e.get("Type").and_then(Tag::as_str).map_or(0, |t| ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry", "bamboo"].iter().position(|w| *w == t).unwrap_or(0)))
                    .copied()
                    .unwrap_or("oak")
                    .to_string()
            } else {
                id.trim_end_matches("_chest_boat").trim_end_matches("_chest_raft").trim_end_matches("_boat").trim_end_matches("_raft").to_string()
            };
            let chest = id.contains("chest");
            let t = Tex::new(&format!("entity/{}/{wood}", if chest { "chest_boat" } else { "boat" }), 128.0, if chest { 128.0 } else { 64.0 });
            // The game turns a boat a quarter round and lifts it into the water.
            let boat = r.t(0.0, 24.0 - 6.0, 0.0).ry(HALF);
            b.cube(&boat.pose([0.0, 3.0, 1.0], [HALF, 0.0, 0.0]), &t, 0.0, 0.0, [-14.0, -9.0, -3.0], [28.0, 16.0, 3.0], 0.0, false);
            b.cube(&boat.pose([-15.0, 4.0, 4.0], [0.0, 3.0 * HALF, 0.0]), &t, 0.0, 19.0, [-13.0, -7.0, -1.0], [18.0, 6.0, 2.0], 0.0, false);
            b.cube(&boat.pose([15.0, 4.0, 0.0], [0.0, HALF, 0.0]), &t, 0.0, 27.0, [-8.0, -7.0, -1.0], [16.0, 6.0, 2.0], 0.0, false);
            b.cube(&boat.pose([0.0, 4.0, -9.0], [0.0, PI, 0.0]), &t, 0.0, 35.0, [-14.0, -7.0, -1.0], [28.0, 6.0, 2.0], 0.0, false);
            b.cube(&boat.pose([0.0, 4.0, 9.0], [0.0; 3]), &t, 0.0, 43.0, [-14.0, -7.0, -1.0], [28.0, 6.0, 2.0], 0.0, false);
            if chest {
                b.cube(&boat.pose([-2.0, -5.0, -6.0], [0.0, -HALF, 0.0]), &t, 0.0, 76.0, [0.0, 0.0, 0.0], [12.0, 8.0, 12.0], 0.0, false);
                b.cube(&boat.pose([-2.0, -9.0, -6.0], [0.0, -HALF, 0.0]), &t, 0.0, 59.0, [0.0, 0.0, 0.0], [12.0, 4.0, 12.0], 0.0, false);
            }
        }
        "copper_golem" => {
            // The statue's model where the golem stands, weathered as it is.
            let age = match e.get("weather_state").and_then(Tag::as_str).unwrap_or("unaffected") {
                "exposed" => "exposed_",
                "weathered" => "weathered_",
                "oxidized" => "oxidized_",
                _ => "",
            };
            crate::entities::copper_golem_at(b, &format!("{age}copper_golem_statue"), "facing=south,copper_golem_pose=standing", local.ry(-yaw * PI / 180.0).t(-0.5, 0.0, -0.5));
        }
        "ravager" => {
            let t = Tex::new("entity/illager/ravager", 128.0, 128.0);
            let neck = r.pose([0.0, -7.0, 5.5], [0.0; 3]);
            b.cube(&neck, &t, 68.0, 73.0, [-5.0, -1.0, -18.0], [10.0, 10.0, 18.0], 0.0, false);
            let head = neck.pose([0.0, 16.0, -17.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-8.0, -20.0, -14.0], [16.0, 20.0, 16.0], 0.0, false);
            b.cube(&head, &t, 0.0, 0.0, [-2.0, -6.0, -18.0], [4.0, 8.0, 4.0], 0.0, false);
            b.cube(&head.pose([-10.0, -14.0, -8.0], [1.0996, 0.0, 0.0]), &t, 74.0, 55.0, [0.0, -14.0, -2.0], [2.0, 14.0, 4.0], 0.0, false);
            b.cube(&head.pose([8.0, -14.0, -8.0], [1.0996, 0.0, 0.0]), &t, 74.0, 55.0, [0.0, -14.0, -2.0], [2.0, 14.0, 4.0], 0.0, true);
            b.cube(&head.pose([0.0, -2.0, 2.0], [0.0; 3]), &t, 0.0, 36.0, [-8.0, 0.0, -16.0], [16.0, 3.0, 16.0], 0.0, false);
            let body = r.pose([0.0, 1.0, 2.0], [HALF, 0.0, 0.0]);
            b.cube(&body, &t, 0.0, 55.0, [-7.0, -10.0, -7.0], [14.0, 16.0, 20.0], 0.0, false);
            b.cube(&body, &t, 0.0, 91.0, [-6.0, 6.0, -7.0], [12.0, 13.0, 18.0], 0.0, false);
            for (x, z, u, mirror) in [(-8.0, 18.0, 96.0, false), (8.0, 18.0, 96.0, true), (-8.0, -5.0, 64.0, false), (8.0, -5.0, 64.0, true)] {
                b.cube(&r.pose([x, -13.0, z], [0.0; 3]), &t, u, 0.0, [-4.0, 0.0, -4.0], [8.0, 37.0, 8.0], 0.0, mirror);
            }
        }
        "creaking" => {
            let t = Tex::new("entity/creaking/creaking", 64.0, 64.0);
            let root = r.pose([0.0, 24.0, 0.0], [0.0; 3]);
            let upper = root.pose([-1.0, -19.0, 0.0], [0.0; 3]);
            let head = upper.pose([-3.0, -11.0, 0.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-3.0, -10.0, -3.0], [6.0, 10.0, 6.0], 0.0, false);
            b.cube(&head, &t, 28.0, 31.0, [-3.0, -13.0, -3.0], [6.0, 3.0, 6.0], 0.0, false);
            b.cube(&head, &t, 12.0, 40.0, [3.0, -13.0, 0.0], [9.0, 14.0, 0.0], 0.0, false);
            b.cube(&head, &t, 34.0, 12.0, [-12.0, -14.0, 0.0], [9.0, 14.0, 0.0], 0.0, false);
            let body = upper.pose([0.0, -7.0, 1.0], [0.0; 3]);
            b.cube(&body, &t, 0.0, 16.0, [0.0, -3.0, -3.0], [6.0, 13.0, 5.0], 0.0, false);
            b.cube(&body, &t, 24.0, 0.0, [-6.0, -4.0, -3.0], [6.0, 7.0, 5.0], 0.0, false);
            let ra = upper.pose([-7.0, -9.5, 1.5], [0.0; 3]);
            b.cube(&ra, &t, 22.0, 13.0, [-2.0, -1.5, -1.5], [3.0, 21.0, 3.0], 0.0, false);
            b.cube(&ra, &t, 46.0, 0.0, [-2.0, 19.5, -1.5], [3.0, 4.0, 3.0], 0.0, false);
            let la = upper.pose([6.0, -9.0, 0.5], [0.0; 3]);
            b.cube(&la, &t, 30.0, 40.0, [0.0, -1.0, -1.5], [3.0, 16.0, 3.0], 0.0, false);
            b.cube(&la, &t, 52.0, 12.0, [0.0, -5.0, -1.5], [3.0, 4.0, 3.0], 0.0, false);
            b.cube(&la, &t, 52.0, 19.0, [0.0, 15.0, -1.5], [3.0, 4.0, 3.0], 0.0, false);
            let ll = root.pose([1.5, -16.0, 0.5], [0.0; 3]);
            b.cube(&ll, &t, 42.0, 40.0, [-1.5, 0.0, -1.5], [3.0, 16.0, 3.0], 0.0, false);
            b.cube(&ll, &t, 45.0, 55.0, [-1.5, 15.7, -4.5], [5.0, 0.0, 9.0], 0.0, false);
            let rl = root.pose([-1.0, -17.5, 0.5], [0.0; 3]);
            b.cube(&rl, &t, 0.0, 34.0, [-3.0, -1.5, -1.5], [3.0, 19.0, 3.0], 0.0, false);
            b.cube(&rl, &t, 45.0, 46.0, [-5.0, 17.2, -4.5], [5.0, 0.0, 9.0], 0.0, false);
            b.cube(&rl, &t, 12.0, 34.0, [-3.0, -4.5, -1.5], [3.0, 3.0, 3.0], 0.0, false);
        }
        "sulfur_cube" => {
            // A cube in a cube, as a slime is: the core, then the shell around it.
            let small = e.get("Size").and_then(Tag::as_i64).is_some_and(|s| s <= 0);
            let tail = if small { "_small" } else { "" };
            let inner = Tex::new(&format!("entity/sulfur_cube/sulfur_cube_inner{tail}"), 128.0, 128.0);
            b.cube(&r.pose([0.0, 24.0, 0.0], [0.0; 3]), &inner, 0.0, 36.0, [-8.0, -17.0, -8.0], [16.0, 16.0, 16.0], 0.0, false);
            let outer = Tex::new(&format!("entity/sulfur_cube/sulfur_cube_outer{tail}"), 128.0, 128.0);
            b.cube(&r.pose([0.0, 24.0, 0.0], [0.0; 3]), &outer, 0.0, 0.0, [-9.0, -18.0, -9.0], [18.0, 18.0, 18.0], 0.0, false);
        }
        "nautilus" | "zombie_nautilus" => {
            // From its picture and the wiki's renders: a shell on a longer lower shell, open at
            // the back (its brown inside), the head looking out of the opening and its
            // tentacles trailing behind (it swims shell first).
            let file = if id == "zombie_nautilus" { "entity/nautilus/zombie_nautilus" } else { "entity/nautilus/nautilus" };
            let t = Tex::new(file, 128.0, 128.0);
            b.cube(&r.pose([0.0, 6.0, -8.0], [0.0; 3]), &t, 0.0, 0.0, [-7.0, 0.0, 0.0], [14.0, 10.0, 16.0], 0.0, false);
            b.cube(&r.pose([0.0, 16.0, -8.0], [0.0; 3]), &t, 0.0, 26.0, [-7.0, 0.0, 0.0], [14.0, 8.0, 20.0], 0.0, false);
            b.cube(&r.pose([0.0, 14.0, 8.0], [0.0; 3]), &t, 14.0, 57.0, [-2.5, 0.0, 0.0], [5.0, 6.0, 5.0], 0.0, false);
            // Its tentacles: a ring of tips round the head, facing back.
            b.cube(&r.pose([0.0, 17.0, 12.5], [0.0; 3]), &t, 58.0, 58.0, [-10.0, -10.0, 0.0], [20.0, 20.0, 0.0], 0.0, false);
        }
        "allay" => {
            let t = Tex::new("entity/allay/allay", 32.0, 32.0);
            let root = r.pose([0.0, 23.5, 0.0], [0.0; 3]);
            b.cube(&root.pose([0.0, -3.99, 0.0], [0.0; 3]), &t, 0.0, 0.0, [-2.5, -5.0, -2.5], [5.0, 5.0, 5.0], 0.0, false);
            let body = root.pose([0.0, -4.0, 0.0], [0.0; 3]);
            b.cube(&body, &t, 0.0, 10.0, [-1.5, 0.0, -1.0], [3.0, 4.0, 2.0], 0.0, false);
            b.cube(&body, &t, 0.0, 16.0, [-1.5, 0.0, -1.0], [3.0, 5.0, 2.0], -0.2, false);
            b.cube(&body.pose([-1.75, 0.5, 0.0], [0.0; 3]), &t, 23.0, 0.0, [-0.75, -0.5, -1.0], [1.0, 4.0, 2.0], -0.01, false);
            b.cube(&body.pose([1.75, 0.5, 0.0], [0.0; 3]), &t, 23.0, 6.0, [-0.25, -0.5, -1.0], [1.0, 4.0, 2.0], -0.01, false);
            // Its wings, spread a little behind it.
            b.cube(&body.pose([-0.5, 0.0, 0.6], [0.0, -0.45, 0.0]), &t, 16.0, 14.0, [0.0, 1.0, 0.0], [0.0, 5.0, 8.0], 0.0, false);
            b.cube(&body.pose([0.5, 0.0, 0.6], [0.0, 0.45, 0.0]), &t, 16.0, 14.0, [0.0, 1.0, 0.0], [0.0, 5.0, 8.0], 0.0, false);
        }
        "camel" | "camel_husk" => {
            let file = if id == "camel_husk" { "entity/camel/camel_husk" } else { "entity/camel/camel" };
            let t = Tex::new(file, 128.0, 128.0);
            let body = r.pose([0.0, 4.0, 9.5], [0.0; 3]);
            b.cube(&body, &t, 0.0, 25.0, [-7.5, -12.0, -23.5], [15.0, 12.0, 27.0], 0.0, false);
            b.cube(&body.pose([0.0, -12.0, -10.0], [0.0; 3]), &t, 74.0, 0.0, [-4.5, -5.0, -5.5], [9.0, 5.0, 11.0], 0.0, false);
            b.cube(&body.pose([0.0, -9.0, 3.5], [0.0; 3]), &t, 122.0, 0.0, [-1.5, 0.0, 0.0], [3.0, 14.0, 0.0], 0.0, false);
            let head = body.pose([0.0, -3.0, -19.5], [0.0; 3]);
            b.cube(&head, &t, 60.0, 24.0, [-3.5, -7.0, -15.0], [7.0, 8.0, 19.0], 0.0, false);
            b.cube(&head, &t, 21.0, 0.0, [-3.5, -21.0, -15.0], [7.0, 14.0, 7.0], 0.0, false);
            b.cube(&head, &t, 50.0, 0.0, [-2.5, -21.0, -21.0], [5.0, 5.0, 6.0], 0.0, false);
            b.cube(&head.pose([3.0, -21.0, -9.5], [0.0; 3]), &t, 45.0, 0.0, [-0.5, 0.5, -1.0], [3.0, 1.0, 2.0], 0.0, false);
            b.cube(&head.pose([-3.0, -21.0, -9.5], [0.0; 3]), &t, 67.0, 0.0, [-2.5, 0.5, -1.0], [3.0, 1.0, 2.0], 0.0, false);
            for (x, z, u, v) in [(4.9, 9.5, 58.0, 16.0), (-4.9, 9.5, 94.0, 16.0), (4.9, -10.5, 0.0, 0.0), (-4.9, -10.5, 0.0, 26.0)] {
                b.cube(&r.pose([x, 1.0, z], [0.0; 3]), &t, u, v, [-2.5, 2.0, -2.5], [5.0, 21.0, 5.0], 0.0, false);
            }
        }
        "rabbit" => {
            // 26.x's rabbit: its picture's boxes (a body, head, ears, tail, front legs and hind
            // feet), set as it crouches.
            let kind = match e.get("RabbitType").and_then(Tag::as_i64) {
                Some(1) => "white",
                Some(2) => "black",
                Some(3) => "white_splotched",
                Some(4) => "gold",
                Some(5) => "salt",
                Some(99) => "caerbannog",
                _ => "brown",
            };
            let named_toast = e.get("CustomName").and_then(Tag::as_str).is_some_and(|n| n.contains("Toast"));
            let file = format!("entity/rabbit/rabbit_{}", if named_toast { "toast" } else { kind });
            let t = Tex::new(&file, 64.0, 64.0);
            let body = r.pose([0.0, 23.0, 5.0], [-0.35, 0.0, 0.0]);
            b.cube(&body, &t, 0.0, 0.0, [-4.0, -6.0, -10.0], [8.0, 6.0, 10.0], 0.0, false);
            let head = r.pose([0.0, 15.5, -2.5], [0.0; 3]);
            b.cube(&head, &t, 0.0, 16.0, [-2.5, -5.0, -5.0], [5.0, 5.0, 5.0], 0.0, false);
            b.cube(&head.pose([-1.0, -5.0, -1.5], [-0.15, 0.0, -0.1]), &t, 26.0, 0.0, [-1.0, -5.0, -0.5], [2.0, 5.0, 1.0], 0.0, false);
            b.cube(&head.pose([1.0, -5.0, -1.5], [-0.15, 0.0, 0.1]), &t, 32.0, 0.0, [-1.0, -5.0, -0.5], [2.0, 5.0, 1.0], 0.0, false);
            b.cube(&r.pose([0.0, 16.5, 5.0], [0.0; 3]), &t, 20.0, 16.0, [-2.0, 0.0, 0.0], [4.0, 4.0, 4.0], 0.0, false);
            b.cube(&r.pose([-2.0, 20.0, -3.0], [0.0; 3]), &t, 36.0, 18.0, [-1.0, 0.0, -1.0], [2.0, 4.0, 2.0], 0.0, false);
            b.cube(&r.pose([2.0, 20.0, -3.0], [0.0; 3]), &t, 44.0, 18.0, [-1.0, 0.0, -1.0], [2.0, 4.0, 2.0], 0.0, false);
            b.cube(&r.pose([-3.0, 23.0, 0.0], [0.0; 3]), &t, 20.0, 24.0, [-2.0, 0.0, 0.0], [4.0, 1.0, 6.0], 0.0, false);
            b.cube(&r.pose([3.0, 23.0, 0.0], [0.0; 3]), &t, 36.0, 24.0, [-2.0, 0.0, 0.0], [4.0, 1.0, 6.0], 0.0, false);
        }
        "sheep" => {
            let t = Tex::new("entity/sheep/sheep", 64.0, 32.0);
            b.cube(&r.pose([0.0, 6.0, -8.0], [0.0; 3]), &t, 0.0, 0.0, [-3.0, -4.0, -6.0], [6.0, 6.0, 8.0], 0.0, false);
            b.cube(&r.pose([0.0, 5.0, 2.0], [HALF, 0.0, 0.0]), &t, 28.0, 8.0, [-4.0, -10.0, -7.0], [8.0, 16.0, 6.0], 0.0, false);
            let feet = [(-3.0, 7.0), (3.0, 7.0), (-3.0, -5.0), (3.0, -5.0)];
            legs(b, r, &t, 0.0, 16.0, 12.0, feet, 12.0, [4.0, 4.0]);
            if !flag("Sheared") {
                let w = Tex::new("entity/sheep/sheep_wool", 64.0, 32.0);
                let color = e.get("Color").and_then(Tag::as_i64).unwrap_or(0).clamp(0, 15) as usize;
                b.tint = crate::entities::dye(["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"][color]);
                b.cube(&r.pose([0.0, 6.0, -8.0], [0.0; 3]), &w, 0.0, 0.0, [-3.0, -4.0, -4.0], [6.0, 6.0, 6.0], 0.6, false);
                b.cube(&r.pose([0.0, 5.0, 2.0], [HALF, 0.0, 0.0]), &w, 28.0, 8.0, [-4.0, -10.0, -7.0], [8.0, 16.0, 6.0], 1.75, false);
                for (x, z) in feet {
                    b.cube(&r.pose([x, 12.0, z], [0.0; 3]), &w, 0.0, 16.0, [-2.0, 0.0, -2.0], [4.0, 6.0, 4.0], 0.5, false);
                }
                b.tint = [1.0; 3];
            }
        }
        "chicken" => {
            let v = variant(e, "variant").unwrap_or_else(|| "temperate".into());
            let t = Tex::new(&format!("entity/chicken/chicken_{v}"), 64.0, 32.0);
            let head = r.pose([0.0, 15.0, -4.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-2.0, -6.0, -2.0], [4.0, 6.0, 3.0], 0.0, false);
            b.cube(&head, &t, 14.0, 0.0, [-2.0, -4.0, -4.0], [4.0, 2.0, 2.0], 0.0, false);
            b.cube(&head, &t, 14.0, 4.0, [-1.0, -2.0, -3.0], [2.0, 2.0, 2.0], 0.0, false);
            b.cube(&r.pose([0.0, 16.0, 0.0], [HALF, 0.0, 0.0]), &t, 0.0, 9.0, [-3.0, -4.0, -3.0], [6.0, 8.0, 6.0], 0.0, false);
            b.cube(&r.pose([-2.0, 19.0, 1.0], [0.0; 3]), &t, 26.0, 0.0, [-1.0, 0.0, -3.0], [3.0, 5.0, 3.0], 0.0, false);
            b.cube(&r.pose([1.0, 19.0, 1.0], [0.0; 3]), &t, 26.0, 0.0, [-1.0, 0.0, -3.0], [3.0, 5.0, 3.0], 0.0, false);
            b.cube(&r.pose([-4.0, 13.0, 0.0], [0.0; 3]), &t, 24.0, 13.0, [0.0, 0.0, -3.0], [1.0, 4.0, 6.0], 0.0, false);
            b.cube(&r.pose([4.0, 13.0, 0.0], [0.0; 3]), &t, 24.0, 13.0, [-1.0, 0.0, -3.0], [1.0, 4.0, 6.0], 0.0, false);
        }
        "spider" | "cave_spider" => {
            let t = Tex::new(if id == "spider" { "entity/spider/spider" } else { "entity/spider/cave_spider" }, 64.0, 32.0);
            b.cube(&r.pose([0.0, 15.0, -3.0], [0.0; 3]), &t, 32.0, 4.0, [-4.0, -4.0, -8.0], [8.0, 8.0, 8.0], 0.0, false);
            b.cube(&r.pose([0.0, 15.0, 0.0], [0.0; 3]), &t, 0.0, 0.0, [-3.0, -3.0, -3.0], [6.0, 6.0, 6.0], 0.0, false);
            b.cube(&r.pose([0.0, 15.0, 9.0], [0.0; 3]), &t, 0.0, 12.0, [-5.0, -4.0, -6.0], [10.0, 8.0, 12.0], 0.0, false);
            // Its legs splayed as it stands (the game's resting angles).
            let (q, m) = (std::f32::consts::FRAC_PI_4, 0.581_194_6);
            for (z, zr, yr) in [(2.0, q, q), (1.0, m, std::f32::consts::FRAC_PI_8), (0.0, m, -std::f32::consts::FRAC_PI_8), (-1.0, q, -q)] {
                b.cube(&r.pose([-4.0, 15.0, z], [0.0, yr, -zr]), &t, 18.0, 0.0, [-15.0, -1.0, -1.0], [16.0, 2.0, 2.0], 0.0, false);
                b.cube(&r.pose([4.0, 15.0, z], [0.0, -yr, zr]), &t, 18.0, 0.0, [-1.0, -1.0, -1.0], [16.0, 2.0, 2.0], 0.0, false);
            }
        }
        "wolf" => {
            let v = variant(e, "variant").unwrap_or_else(|| "pale".into());
            let file = if v == "pale" { "entity/wolf/wolf".to_string() } else { format!("entity/wolf/wolf_{v}") };
            let t = Tex::new(&file, 64.0, 32.0);
            let head = r.pose([-1.0, 13.5, -7.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-2.0, -3.0, -2.0], [6.0, 6.0, 4.0], 0.0, false);
            b.cube(&head, &t, 16.0, 14.0, [-2.0, -5.0, 0.0], [2.0, 2.0, 1.0], 0.0, false);
            b.cube(&head, &t, 16.0, 14.0, [2.0, -5.0, 0.0], [2.0, 2.0, 1.0], 0.0, false);
            b.cube(&head, &t, 0.0, 10.0, [-0.5, -0.001, -5.0], [3.0, 3.0, 4.0], 0.0, false);
            b.cube(&r.pose([0.0, 14.0, 2.0], [HALF, 0.0, 0.0]), &t, 18.0, 14.0, [-3.0, -2.0, -3.0], [6.0, 9.0, 6.0], 0.0, false);
            b.cube(&r.pose([-1.0, 14.0, -3.0], [HALF, 0.0, 0.0]), &t, 21.0, 0.0, [-3.0, -3.0, -3.0], [8.0, 6.0, 7.0], 0.0, false);
            for (x, z) in [(-2.5, 7.0), (0.5, 7.0), (-2.5, -4.0), (0.5, -4.0)] {
                b.cube(&r.pose([x, 16.0, z], [0.0; 3]), &t, 0.0, 18.0, [0.0, 0.0, -1.0], [2.0, 8.0, 2.0], 0.0, false);
            }
            b.cube(&r.pose([-1.0, 12.0, 8.0], [0.63, 0.0, 0.0]), &t, 9.0, 18.0, [0.0, 0.0, -1.0], [2.0, 8.0, 2.0], 0.0, false);
        }
        "iron_golem" => {
            let t = Tex::new("entity/iron_golem/iron_golem", 128.0, 128.0);
            let head = r.pose([0.0, -7.0, -2.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-4.0, -12.0, -5.5], [8.0, 10.0, 8.0], 0.0, false);
            b.cube(&head, &t, 24.0, 0.0, [-1.0, -5.0, -7.5], [2.0, 4.0, 2.0], 0.0, false);
            let body = r.pose([0.0, -7.0, 0.0], [0.0; 3]);
            b.cube(&body, &t, 0.0, 40.0, [-9.0, -2.0, -6.0], [18.0, 12.0, 11.0], 0.0, false);
            b.cube(&body, &t, 0.0, 70.0, [-4.5, 10.0, -3.0], [9.0, 5.0, 6.0], 0.5, false);
            b.cube(&body, &t, 60.0, 21.0, [-13.0, -2.5, -3.0], [4.0, 30.0, 6.0], 0.0, false);
            b.cube(&body, &t, 60.0, 58.0, [9.0, -2.5, -3.0], [4.0, 30.0, 6.0], 0.0, false);
            b.cube(&r.pose([-4.0, 11.0, 0.0], [0.0; 3]), &t, 37.0, 0.0, [-3.5, -3.0, -3.0], [6.0, 16.0, 5.0], 0.0, false);
            b.cube(&r.pose([5.0, 11.0, 0.0], [0.0; 3]), &t, 60.0, 0.0, [-3.5, -3.0, -3.0], [6.0, 16.0, 5.0], 0.0, true);
        }
        "enderman" => {
            let t = Tex::new("entity/enderman/enderman", 64.0, 32.0);
            let head = r.pose([0.0, -14.0, 0.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], 0.0, false);
            b.cube(&head, &t, 0.0, 16.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], -0.5, false);
            b.cube(&head, &t, 32.0, 16.0, [-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], 0.0, false);
            b.cube(&r.pose([-5.0, -12.0, 0.0], [0.0; 3]), &t, 56.0, 0.0, [-1.0, -2.0, -1.0], [2.0, 30.0, 2.0], 0.0, false);
            b.cube(&r.pose([5.0, -12.0, 0.0], [0.0; 3]), &t, 56.0, 0.0, [-1.0, -2.0, -1.0], [2.0, 30.0, 2.0], 0.0, true);
            b.cube(&r.pose([-2.0, -5.0, 0.0], [0.0; 3]), &t, 56.0, 0.0, [-1.0, 0.0, -1.0], [2.0, 30.0, 2.0], 0.0, false);
            b.cube(&r.pose([2.0, -5.0, 0.0], [0.0; 3]), &t, 56.0, 0.0, [-1.0, 0.0, -1.0], [2.0, 30.0, 2.0], 0.0, true);
        }
        "slime" | "magma_cube" => {
            if id == "slime" {
                let t = Tex::new("entity/slime/slime", 64.0, 32.0);
                b.cube(r, &t, 0.0, 16.0, [-3.0, 17.0, -3.0], [6.0, 6.0, 6.0], 0.0, false);
                b.cube(r, &t, 32.0, 0.0, [-3.25, 18.0, -3.5], [2.0, 2.0, 2.0], 0.0, false);
                b.cube(r, &t, 32.0, 4.0, [1.25, 18.0, -3.5], [2.0, 2.0, 2.0], 0.0, false);
                b.cube(r, &t, 32.0, 8.0, [0.0, 21.0, -3.5], [1.0, 1.0, 1.0], 0.0, false);
                b.cube(r, &t, 0.0, 0.0, [-4.0, 16.0, -4.0], [8.0, 8.0, 8.0], 0.0, false);
            } else {
                // A magma cube: its eight slices stacked into a cube, and its core.
                let t = Tex::new("entity/slime/magmacube", 64.0, 32.0);
                for k in 0..8 {
                    let (u, v) = if k < 4 { (0.0, k as f32) } else { (32.0, (k - 4) as f32) };
                    b.cube(r, &t, u, v, [-4.0, 16.0 + k as f32, -4.0], [8.0, 1.0, 8.0], 0.0, false);
                }
                b.cube(r, &t, 0.0, 16.0, [-2.0, 18.0, -2.0], [4.0, 4.0, 4.0], 0.0, false);
            }
        }
        "squid" | "glow_squid" => {
            let t = Tex::new(if id == "squid" { "entity/squid/squid" } else { "entity/squid/glow_squid" }, 64.0, 32.0);
            b.cube(&r.pose([0.0, 8.0, 0.0], [0.0; 3]), &t, 0.0, 0.0, [-6.0, -8.0, -6.0], [12.0, 16.0, 12.0], 0.0, false);
            for k in 0..8 {
                let a = k as f32 * PI * 2.0 / 8.0;
                let (x, z) = (a.cos() * 5.0, a.sin() * 5.0);
                b.cube(&r.pose([x, 15.0, z], [PI / 4.0, PI * 2.0 * -(k as f32) / 8.0 + HALF, 0.0]), &t, 48.0, 0.0, [-1.0, 0.0, -1.0], [2.0, 18.0, 2.0], 0.0, false);
            }
        }
        "cat" | "ocelot" => {
            let file = if id == "ocelot" { "entity/cat/ocelot".to_string() } else { format!("entity/cat/cat_{}", variant(e, "variant").unwrap_or_else(|| "tabby".into())) };
            let t = Tex::new(&file, 64.0, 32.0);
            // Cats are drawn a little smaller than ocelots.
            let r = &if id == "cat" { r.t(0.0, 24.0, 0.0).s(0.8, 0.8, 0.8).t(0.0, -24.0, 0.0) } else { *r };
            let head = r.pose([0.0, 15.0, -9.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-2.5, -2.0, -3.0], [5.0, 4.0, 5.0], 0.0, false);
            b.cube(&head, &t, 0.0, 24.0, [-1.5, 0.0, -4.0], [3.0, 2.0, 2.0], 0.0, false);
            b.cube(&head, &t, 0.0, 10.0, [-2.0, -3.0, 0.0], [1.0, 1.0, 2.0], 0.0, false);
            b.cube(&head, &t, 6.0, 10.0, [1.0, -3.0, 0.0], [1.0, 1.0, 2.0], 0.0, false);
            b.cube(&r.pose([0.0, 12.0, -10.0], [HALF, 0.0, 0.0]), &t, 20.0, 0.0, [-2.0, 3.0, -8.0], [4.0, 16.0, 6.0], 0.0, false);
            b.cube(&r.pose([0.0, 15.0, 8.0], [0.9, 0.0, 0.0]), &t, 0.0, 15.0, [-0.5, 0.0, 0.0], [1.0, 8.0, 1.0], 0.0, false);
            b.cube(&r.pose([0.0, 20.0, 14.0], [1.7, 0.0, 0.0]), &t, 4.0, 15.0, [-0.5, 0.0, 0.0], [1.0, 8.0, 1.0], 0.0, false);
            for x in [1.1, -1.1] {
                b.cube(&r.pose([x, 18.0, 5.0], [0.0; 3]), &t, 8.0, 13.0, [-1.0, 0.0, 1.0], [2.0, 6.0, 2.0], 0.0, false);
            }
            for x in [1.2, -1.2] {
                b.cube(&r.pose([x, 14.1, -5.0], [0.0; 3]), &t, 40.0, 0.0, [-1.0, 0.0, 0.0], [2.0, 10.0, 2.0], 0.0, false);
            }
        }
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" => {
            let file = match id {
                "horse" => {
                    let v = e.get("Variant").and_then(Tag::as_i64).unwrap_or(0);
                    let colors = ["white", "creamy", "chestnut", "brown", "black", "gray", "darkbrown"];
                    format!("entity/horse/horse_{}", colors[(v & 0xFF).clamp(0, 6) as usize])
                }
                "skeleton_horse" => "entity/horse/horse_skeleton".to_string(),
                "zombie_horse" => "entity/horse/horse_zombie".to_string(),
                other => format!("entity/horse/{other}"),
            };
            let s = match id {
                "donkey" => 0.87,
                "mule" => 0.92,
                _ => 1.1,
            };
            let r = &r.t(0.0, 24.0, 0.0).s(s, s, s).t(0.0, -24.0, 0.0);
            let mut layers = vec![Tex::new(&file, 64.0, 64.0)];
            if id == "horse" {
                let marks = (e.get("Variant").and_then(Tag::as_i64).unwrap_or(0) >> 8) & 0xFF;
                if let Some(m) = ["", "white", "whitefield", "whitedots", "blackdots"].get(marks as usize).filter(|m| !m.is_empty()) {
                    layers.push(Tex::new(&format!("entity/horse/horse_markings_{m}"), 64.0, 64.0));
                }
            }
            for (k, t) in layers.iter().enumerate() {
                let g = k as f32 * 0.02;
                b.cube(&r.pose([0.0, 11.0, 5.0], [0.0; 3]), t, 0.0, 32.0, [-5.0, -8.0, -17.0], [10.0, 10.0, 22.0], 0.05 + g, false);
                let neck = r.pose([0.0, 4.0, -12.0], [PI / 6.0, 0.0, 0.0]);
                b.cube(&neck, t, 0.0, 35.0, [-2.05, -6.0, -2.0], [4.0, 12.0, 7.0], g, false);
                b.cube(&neck, t, 0.0, 13.0, [-3.0, -11.0, -2.0], [6.0, 5.0, 7.0], g, false);
                b.cube(&neck, t, 56.0, 36.0, [-1.0, -11.0, 5.01], [2.0, 16.0, 2.0], g, false);
                b.cube(&neck, t, 0.0, 25.0, [-2.0, -11.0, -7.0], [4.0, 5.0, 5.0], g, false);
                let ear = if matches!(id, "donkey" | "mule") { 6.0 } else { 3.0 };
                b.cube(&neck, t, 19.0, 16.0, [0.55, -10.0 - ear, 4.0], [2.0, ear, 1.0], g - 0.001, false);
                b.cube(&neck, t, 19.0, 16.0, [-2.55, -10.0 - ear, 4.0], [2.0, ear, 1.0], g - 0.001, false);
                b.cube(&r.pose([4.0, 14.0, 7.0], [0.0; 3]), t, 48.0, 21.0, [-3.0, -1.01, -1.0], [4.0, 11.0, 4.0], g, true);
                b.cube(&r.pose([-4.0, 14.0, 7.0], [0.0; 3]), t, 48.0, 21.0, [-1.0, -1.01, -1.0], [4.0, 11.0, 4.0], g, false);
                b.cube(&r.pose([4.0, 14.0, -12.0], [0.0; 3]), t, 48.0, 21.0, [-3.0, -1.01, -1.9], [4.0, 11.0, 4.0], g, true);
                b.cube(&r.pose([-4.0, 14.0, -12.0], [0.0; 3]), t, 48.0, 21.0, [-1.0, -1.01, -1.9], [4.0, 11.0, 4.0], g, false);
                b.cube(&r.pose([0.0, 4.0, 11.0], [PI / 6.0, 0.0, 0.0]), t, 42.0, 36.0, [-1.5, 0.0, 0.0], [3.0, 14.0, 4.0], g, false);
            }
        }
        "fox" => {
            let file = if variant(e, "Type").as_deref() == Some("snow") { "entity/fox/fox_snow" } else { "entity/fox/fox" };
            let t = Tex::new(file, 48.0, 32.0);
            let head = r.pose([-1.0, 16.5, -3.0], [0.0; 3]);
            b.cube(&head, &t, 1.0, 5.0, [-3.0, -2.0, -5.0], [8.0, 6.0, 6.0], 0.0, false);
            b.cube(&head, &t, 8.0, 1.0, [-3.0, -4.0, -4.0], [2.0, 2.0, 1.0], 0.0, false);
            b.cube(&head, &t, 15.0, 1.0, [3.0, -4.0, -4.0], [2.0, 2.0, 1.0], 0.0, false);
            b.cube(&head, &t, 6.0, 18.0, [-1.0, 2.01, -8.0], [4.0, 2.0, 3.0], 0.0, false);
            let body = r.pose([0.0, 16.0, -6.0], [HALF, 0.0, 0.0]);
            b.cube(&body, &t, 24.0, 15.0, [-3.0, 3.999, -3.5], [6.0, 11.0, 6.0], 0.0, false);
            b.cube(&body.pose([-4.0, 15.0, -1.0], [-0.052_359_88, 0.0, 0.0]), &t, 30.0, 0.0, [2.0, 0.0, -1.0], [4.0, 9.0, 5.0], 0.0, false);
            for (x, z, u) in [(-5.0, 7.0, 13.0), (-1.0, 7.0, 4.0), (-5.0, 0.0, 13.0), (-1.0, 0.0, 4.0)] {
                b.cube(&r.pose([x, 17.5, z], [0.0; 3]), &t, u, 24.0, [2.0, 0.5, -1.0], [2.0, 6.0, 2.0], 0.001, false);
            }
        }
        "bee" => {
            let angry = e.get("AngerTime").and_then(Tag::as_i64).unwrap_or(0) > 0;
            let nectar = flag("HasNectar");
            let file = match (angry, nectar) {
                (true, true) => "entity/bee/bee_angry_nectar",
                (true, false) => "entity/bee/bee_angry",
                (false, true) => "entity/bee/bee_nectar",
                _ => "entity/bee/bee",
            };
            let t = Tex::new(file, 64.0, 64.0);
            let bone = r.pose([0.0, 19.0, 0.0], [0.0; 3]);
            b.cube(&bone, &t, 0.0, 0.0, [-3.5, -4.0, -5.0], [7.0, 7.0, 10.0], 0.0, false);
            if !flag("HasStung") {
                b.cube(&bone, &t, 26.0, 7.0, [0.0, -1.0, 5.0], [0.0, 1.0, 2.0], 0.0, false);
            }
            let ant = bone.pose([0.0, -2.0, -5.0], [0.0; 3]);
            b.cube(&ant, &t, 2.0, 0.0, [1.5, -2.0, -3.0], [1.0, 2.0, 3.0], 0.0, false);
            b.cube(&ant, &t, 2.0, 3.0, [-2.5, -2.0, -3.0], [1.0, 2.0, 3.0], 0.0, false);
            b.cube(&bone.pose([-1.5, -4.0, -3.0], [0.0, -0.2618, 0.0]), &t, 0.0, 18.0, [-9.0, 0.0, 0.0], [9.0, 0.0, 6.0], 0.0, false);
            b.cube(&bone.pose([1.5, -4.0, -3.0], [0.0, 0.2618, 0.0]), &t, 0.0, 18.0, [0.0, 0.0, 0.0], [9.0, 0.0, 6.0], 0.0, true);
            for (z, v) in [(-2.0, 1.0), (0.0, 3.0), (2.0, 5.0)] {
                b.cube(&bone.pose([1.5, 3.0, z], [0.0; 3]), &t, 26.0, v, [-5.0, 0.0, 0.0], [7.0, 2.0, 0.0], 0.0, false);
            }
        }
        "goat" => {
            let t = Tex::new("entity/goat/goat", 64.0, 64.0);
            let head = r.pose([1.0, 14.0, 0.0], [0.0; 3]);
            b.cube(&head, &t, 2.0, 61.0, [-6.0, -11.0, -10.0], [3.0, 2.0, 1.0], 0.0, false);
            b.cube(&head, &t, 2.0, 61.0, [2.0, -11.0, -10.0], [3.0, 2.0, 1.0], 0.0, true);
            b.cube(&head, &t, 23.0, 52.0, [-0.5, -3.0, -14.0], [0.0, 7.0, 5.0], 0.0, false);
            if !e.get("HasLeftHorn").is_some_and(|h| h.as_i64() == Some(0)) {
                b.cube(&head, &t, 12.0, 55.0, [-0.01, -16.0, -10.0], [2.0, 7.0, 2.0], 0.0, false);
            }
            if !e.get("HasRightHorn").is_some_and(|h| h.as_i64() == Some(0)) {
                b.cube(&head, &t, 12.0, 55.0, [-2.99, -16.0, -10.0], [2.0, 7.0, 2.0], 0.0, false);
            }
            b.cube(&head.pose([0.0, -8.0, -8.0], [0.9599, 0.0, 0.0]), &t, 34.0, 46.0, [-3.0, -4.0, -8.0], [5.0, 7.0, 10.0], 0.0, false);
            let body = r.pose([0.0, 24.0, 0.0], [0.0; 3]);
            b.cube(&body, &t, 1.0, 1.0, [-4.0, -17.0, -7.0], [9.0, 11.0, 16.0], 0.0, false);
            b.cube(&body, &t, 0.0, 28.0, [-5.0, -18.0, -8.0], [11.0, 14.0, 11.0], 0.0, false);
            b.cube(&r.pose([1.0, 14.0, 4.0], [0.0; 3]), &t, 36.0, 29.0, [0.0, 4.0, 0.0], [3.0, 6.0, 3.0], 0.0, false);
            b.cube(&r.pose([-3.0, 14.0, 4.0], [0.0; 3]), &t, 49.0, 29.0, [0.0, 4.0, 0.0], [3.0, 6.0, 3.0], 0.0, false);
            b.cube(&r.pose([1.0, 14.0, -6.0], [0.0; 3]), &t, 49.0, 2.0, [0.0, 0.0, 0.0], [3.0, 10.0, 3.0], 0.0, false);
            b.cube(&r.pose([-3.0, 14.0, -6.0], [0.0; 3]), &t, 35.0, 2.0, [0.0, 0.0, 0.0], [3.0, 10.0, 3.0], 0.0, false);
        }
        "llama" | "trader_llama" => {
            let v = e.get("Variant").and_then(Tag::as_i64).unwrap_or(0).clamp(0, 3) as usize;
            let t = Tex::new(&format!("entity/llama/llama_{}", ["creamy", "white", "brown", "gray"][v]), 128.0, 64.0);
            let head = r.pose([0.0, 7.0, -6.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-2.0, -14.0, -10.0], [4.0, 4.0, 9.0], 0.0, false);
            b.cube(&head, &t, 0.0, 14.0, [-4.0, -16.0, -6.0], [8.0, 18.0, 6.0], 0.0, false);
            b.cube(&head, &t, 17.0, 0.0, [-4.0, -19.0, -4.0], [3.0, 3.0, 2.0], 0.0, false);
            b.cube(&head, &t, 17.0, 0.0, [1.0, -19.0, -4.0], [3.0, 3.0, 2.0], 0.0, false);
            b.cube(&r.pose([0.0, 5.0, 2.0], [HALF, 0.0, 0.0]), &t, 29.0, 0.0, [-6.0, -10.0, -7.0], [12.0, 18.0, 10.0], 0.0, false);
            legs(b, r, &t, 29.0, 29.0, 14.0, [(-3.5, 6.0), (3.5, 6.0), (-3.5, -5.0), (3.5, -5.0)], 10.0, [4.0, 4.0]);
        }
        "polar_bear" => {
            let t = Tex::new("entity/bear/polarbear", 128.0, 64.0);
            let head = r.pose([0.0, 10.0, -16.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-3.5, -3.0, -3.0], [7.0, 7.0, 7.0], 0.0, false);
            b.cube(&head, &t, 0.0, 44.0, [-2.5, 1.0, -6.0], [5.0, 3.0, 3.0], 0.0, false);
            b.cube(&head, &t, 26.0, 0.0, [-4.5, -4.0, -1.0], [2.0, 2.0, 1.0], 0.0, false);
            b.cube(&head, &t, 26.0, 0.0, [2.5, -4.0, -1.0], [2.0, 2.0, 1.0], 0.0, true);
            let body = r.pose([-2.0, 9.0, 12.0], [HALF, 0.0, 0.0]);
            b.cube(&body, &t, 0.0, 19.0, [-5.0, -13.0, -7.0], [14.0, 14.0, 11.0], 0.0, false);
            b.cube(&body, &t, 39.0, 0.0, [-4.0, -25.0, -7.0], [12.0, 12.0, 10.0], 0.0, false);
            for (x, z, v, d) in [(-4.5, 6.0, 22.0, 8.0), (4.5, 6.0, 22.0, 8.0), (-3.5, -8.0, 40.0, 6.0), (3.5, -8.0, 40.0, 6.0)] {
                b.cube(&r.pose([x, 14.0, z], [0.0; 3]), &t, 50.0, v, [-2.0, 0.0, -2.0], [4.0, 10.0, d], 0.0, false);
            }
        }
        "panda" => {
            let gene = variant(e, "MainGene").unwrap_or_else(|| "normal".into());
            let file = if gene == "normal" { "entity/panda/panda".to_string() } else { format!("entity/panda/panda_{gene}") };
            let t = Tex::new(&file, 64.0, 64.0);
            let head = r.pose([0.0, 11.5, -17.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 6.0, [-6.5, -5.0, -4.0], [13.0, 10.0, 9.0], 0.0, false);
            b.cube(&head, &t, 45.0, 16.0, [-3.5, 0.0, -6.0], [7.0, 5.0, 2.0], 0.0, false);
            b.cube(&head, &t, 52.0, 25.0, [3.5, -8.0, -1.0], [5.0, 4.0, 1.0], 0.0, false);
            b.cube(&head, &t, 52.0, 25.0, [-8.5, -8.0, -1.0], [5.0, 4.0, 1.0], 0.0, false);
            b.cube(&r.pose([0.0, 10.0, 0.0], [HALF, 0.0, 0.0]), &t, 0.0, 25.0, [-9.5, -13.0, -6.5], [19.0, 26.0, 13.0], 0.0, false);
            legs(b, r, &t, 40.0, 0.0, 9.0, [(-5.5, 9.0), (5.5, 9.0), (-5.5, -9.0), (5.5, -9.0)], 15.0, [6.0, 6.0]);
        }
        "parrot" => {
            let v = e.get("Variant").and_then(Tag::as_i64).unwrap_or(0).clamp(0, 4) as usize;
            let t = Tex::new(&format!("entity/parrot/parrot_{}", ["red_blue", "blue", "green", "yellow_blue", "grey"][v]), 32.0, 32.0);
            b.cube(&r.pose([0.0, 16.5, -3.0], [0.4937, 0.0, 0.0]), &t, 2.0, 8.0, [-1.5, 0.0, -1.5], [3.0, 6.0, 3.0], 0.0, false);
            b.cube(&r.pose([0.0, 21.07, 1.16], [1.015, 0.0, 0.0]), &t, 22.0, 1.0, [-1.5, -1.0, -1.0], [3.0, 4.0, 1.0], 0.0, false);
            b.cube(&r.pose([1.5, 16.94, -2.76], [-0.6981, -PI, 0.0]), &t, 19.0, 8.0, [-0.5, 0.0, -1.5], [1.0, 5.0, 3.0], 0.0, false);
            b.cube(&r.pose([-1.5, 16.94, -2.76], [-0.6981, -PI, 0.0]), &t, 19.0, 8.0, [-0.5, 0.0, -1.5], [1.0, 5.0, 3.0], 0.0, false);
            let head = r.pose([0.0, 15.69, -2.76], [0.0; 3]);
            b.cube(&head, &t, 2.0, 2.0, [-1.0, -1.5, -1.0], [2.0, 3.0, 2.0], 0.0, false);
            b.cube(&head.pose([0.0, -2.0, -1.0], [0.0; 3]), &t, 10.0, 0.0, [-1.0, -0.5, -2.0], [2.0, 1.0, 4.0], 0.0, false);
            b.cube(&head.pose([0.0, -0.5, -1.5], [0.0; 3]), &t, 11.0, 7.0, [-0.5, -1.0, -0.5], [1.0, 2.0, 1.0], 0.0, false);
            b.cube(&head.pose([0.0, -1.75, -2.45], [0.0; 3]), &t, 16.0, 7.0, [-0.5, 0.0, -0.5], [1.0, 2.0, 1.0], 0.0, false);
            b.cube(&head.pose([0.0, -2.15, 0.15], [-0.2214, 0.0, 0.0]), &t, 2.0, 18.0, [0.0, -4.0, -2.0], [0.0, 5.0, 4.0], 0.0, false);
            b.cube(&r.pose([1.0, 22.0, -1.05], [-0.0299, 0.0, 0.0]), &t, 14.0, 18.0, [-0.5, 0.0, -0.5], [1.0, 2.0, 1.0], 0.0, false);
            b.cube(&r.pose([-1.0, 22.0, -1.05], [-0.0299, 0.0, 0.0]), &t, 14.0, 18.0, [-0.5, 0.0, -0.5], [1.0, 2.0, 1.0], 0.0, false);
        }
        "snow_golem" => {
            let t = Tex::new("entity/snow_golem/snow_golem", 64.0, 64.0);
            let head = r.pose([0.0, 4.0, 0.0], [0.0; 3]);
            b.cube(&head, &t, 0.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], -0.5, false);
            b.cube(&r.pose([0.0, 13.0, 0.0], [0.0; 3]), &t, 0.0, 16.0, [-5.0, -10.0, -5.0], [10.0, 10.0, 10.0], -0.5, false);
            b.cube(&r.pose([0.0, 24.0, 0.0], [0.0; 3]), &t, 0.0, 36.0, [-6.0, -12.0, -6.0], [12.0, 12.0, 12.0], -0.5, false);
            b.cube(&r.pose([5.0, 6.0, 1.0], [0.0, 0.0, 1.0]), &t, 32.0, 0.0, [-1.0, 0.0, -1.0], [12.0, 2.0, 2.0], -0.5, false);
            b.cube(&r.pose([-5.0, 6.0, -1.0], [0.0, PI, -1.0]), &t, 32.0, 0.0, [-1.0, 0.0, -1.0], [12.0, 2.0, 2.0], -0.5, false);
            // Its carved pumpkin, unless sheared off.
            if e.get("Pumpkin").and_then(Tag::as_i64).unwrap_or(1) != 0 {
                let m = head.t(0.0, -4.0, 0.0).s(-10.0, -10.0, 10.0).t(-0.5, -0.5, -0.5);
                b.item(&m, "carved_pumpkin", [1.0; 3]);
            }
        }
        _ => return false,
    }
    // Armor it wears, on humanoid mobs.
    // Armor it wears and what it holds, on humanoid mobs (in any edition's layout).
    if let (Some(parts), Some(c)) = (humanoid_parts(id, r, e.get("MctkMount").is_some()), e.as_compound()) {
        use crate::mob_edit::Slot;
        for (slot, item) in crate::mob_edit::equipment(c, None) {
            let Some((item, _)) = item else { continue };
            let item = item.trim_start_matches("minecraft:").to_string();
            match slot {
                Slot::Head => armor(b, &parts, "head", &item, [0.63, 0.4, 0.25]),
                Slot::Chest => armor(b, &parts, "chest", &item, [0.63, 0.4, 0.25]),
                Slot::Legs => armor(b, &parts, "legs", &item, [0.63, 0.4, 0.25]),
                Slot::Feet => armor(b, &parts, "feet", &item, [0.63, 0.4, 0.25]),
                Slot::MainHand => held(b, &parts.rarm, &item, false),
                Slot::OffHand => held(b, &parts.larm, &item, true),
            }
        }
    }
    true
}

/// An item in a humanoid's hand (`arm` its arm's pose), as the game's item-in-hand layer holds
/// it: at the end of the arm, turned by the item model's third-person hold (a tool's, a flat
/// item's or a block's).
pub(crate) fn held(b: &mut Builder, arm: &Mat, item: &str, left: bool) {
    let deg = PI / 180.0;
    let side = if left { -1.0 } else { 1.0 };
    let hand = arm.rx(-90.0 * deg).ry(180.0 * deg).t(side, 2.0, -10.0);
    let tool = item.ends_with("_sword") || item.ends_with("_axe") || item.ends_with("_pickaxe") || item.ends_with("_shovel") || item.ends_with("_hoe") || item.ends_with("_spear")
        || matches!(item, "stick" | "bone" | "blaze_rod" | "breeze_rod" | "fishing_rod" | "carrot_on_a_stick" | "warped_fungus_on_a_stick" | "mace" | "trident" | "debug_stick");
    let block = matches!(crate::models::item_model(item).as_deref(), Some(crate::models::ItemModel::Model(_)));
    // (rotation x, y, z in degrees; translation in pixels; scale), for the right hand; the left
    // hand's mirrors it.
    let (rot, tr, s): ([f32; 3], [f32; 3], f32) = if block {
        ([75.0, 45.0, 0.0], [0.0, 2.5, 0.0], 0.375)
    } else if tool {
        ([0.0, -90.0, 55.0], [0.0, 4.0, 0.5], 0.85)
    } else {
        ([0.0, 0.0, 0.0], [0.0, 3.0, 1.0], 0.55)
    };
    let (rot, tr) = if left { ([rot[0], -rot[1], -rot[2]], [-tr[0], tr[1], tr[2]]) } else { (rot, tr) };
    let m = hand.t(tr[0], tr[1], tr[2]).rx(rot[0] * deg).ry(rot[1] * deg).rz(rot[2] * deg).s(s * 16.0, s * 16.0, s * 16.0).t(-0.5, -0.5, -0.5);
    b.item(&m, item, crate::world::block_tint(item, ""));
}

/// Four legs from (`u`, `v`), `h` tall and `w` across, hung from height `y` at `at` (x, z).
#[allow(clippy::too_many_arguments)]
fn legs(b: &mut Builder, r: &Mat, t: &Tex, u: f32, v: f32, h: f32, at: [(f32, f32); 4], y: f32, w: [f32; 2]) {
    for (k, (x, z)) in at.into_iter().enumerate() {
        // The left legs are the right ones mirrored.
        b.cube(&r.pose([x, y, z], [0.0; 3]), t, u, v, [-w[0] / 2.0, 0.0, -w[1] / 2.0], [w[0], h, w[1]], 0.0, k % 2 == 1);
    }
}

/// Where a humanoid mob's parts are, for its armor.
fn humanoid_parts(id: &str, r: &Mat, riding: bool) -> Option<Humanoid> {
    let arms_out = matches!(id, "zombie" | "husk" | "drowned" | "giant" | "zombified_piglin" | "zombie_villager");
    let arm = if arms_out { [-HALF + 0.1, 0.0, 0.0] } else { [0.0; 3] };
    // A rider sits: its legs forward and turned out a little, its arms a little forward.
    let arm = if riding { [arm[0] - 0.6283, 0.0, 0.0] } else { arm };
    let (rleg, lleg) = if riding { ([-1.4137, 0.3142, 0.0], [-1.4137, -0.3142, 0.0]) } else { ([0.0; 3], [0.0; 3]) };
    match id {
        "zombie" | "husk" | "drowned" | "giant" | "zombified_piglin" | "piglin" | "piglin_brute" | "skeleton" | "stray" | "bogged" | "wither_skeleton" | "parched" | "player" | "zombie_villager" => Some(Humanoid {
            head: r.pose([0.0; 3], [0.0; 3]),
            body: r.pose([0.0; 3], [0.0; 3]),
            rarm: r.pose([-5.0, 2.0, 0.0], arm),
            larm: r.pose([5.0, 2.0, 0.0], arm),
            rleg: r.pose([-1.9, 12.0, 0.0], rleg),
            lleg: r.pose([1.9, 12.0, 0.0], lleg),
        }),
        _ => None,
    }
}

/// Zombies, skeletons, piglins and the like: the humanoid body with each one's texture,
/// limbs and extras.
fn humanoid(b: &mut Builder, e: &Tag, id: &str, r: &Mat) {
    let _ = e;
    let (file, h, skinny, wide_left) = match id {
        "zombie" | "giant" => ("entity/zombie/zombie", 64.0, false, false),
        "husk" => ("entity/zombie/husk", 64.0, false, false),
        "drowned" => ("entity/zombie/drowned", 64.0, false, true),
        "zombified_piglin" => ("entity/piglin/zombified_piglin", 64.0, false, true),
        "piglin" => ("entity/piglin/piglin", 64.0, false, true),
        "piglin_brute" => ("entity/piglin/piglin_brute", 64.0, false, true),
        "skeleton" => ("entity/skeleton/skeleton", 32.0, true, false),
        "stray" => ("entity/skeleton/stray", 32.0, true, false),
        "bogged" => ("entity/skeleton/bogged", 32.0, true, false),
        "wither_skeleton" => ("entity/skeleton/wither_skeleton", 32.0, true, false),
        "parched" => ("entity/skeleton/parched", 32.0, true, false),
        _ => ("entity/player/wide/steve", 64.0, false, true),
    };
    let t = Tex::new(file, 64.0, h);
    let parts = humanoid_parts(id, r, e.get("MctkMount").is_some()).expect("a humanoid");
    let piglin = id.contains("piglin");
    if piglin {
        b.cube(&parts.head, &t, 0.0, 0.0, [-5.0, -8.0, -4.0], [10.0, 8.0, 8.0], 0.0, false);
        b.cube(&parts.head, &t, 31.0, 1.0, [-2.0, -4.0, -5.0], [4.0, 4.0, 1.0], 0.0, false);
        b.cube(&parts.head, &t, 2.0, 4.0, [2.0, -2.0, -5.0], [1.0, 2.0, 1.0], 0.0, false);
        b.cube(&parts.head, &t, 2.0, 0.0, [-3.0, -2.0, -5.0], [1.0, 2.0, 1.0], 0.0, false);
        b.cube(&parts.head.pose([4.5, -6.0, 0.0], [0.0, 0.0, -PI / 6.0]), &t, 51.0, 6.0, [0.0, 0.0, -2.0], [1.0, 5.0, 4.0], 0.0, false);
        b.cube(&parts.head.pose([-4.5, -6.0, 0.0], [0.0, 0.0, PI / 6.0]), &t, 39.0, 6.0, [-1.0, 0.0, -2.0], [1.0, 5.0, 4.0], 0.0, false);
    } else {
        b.cube(&parts.head, &t, 0.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], 0.0, false);
        b.cube(&parts.head, &t, 32.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], 0.5, false);
    }
    b.cube(&parts.body, &t, 16.0, 16.0, [-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], 0.0, false);
    if skinny {
        b.cube(&parts.rarm, &t, 40.0, 16.0, [-1.0, -2.0, -1.0], [2.0, 12.0, 2.0], 0.0, false);
        b.cube(&parts.larm, &t, 40.0, 16.0, [-1.0, -2.0, -1.0], [2.0, 12.0, 2.0], 0.0, true);
        b.cube(&parts.rleg, &t, 0.0, 16.0, [-1.0, 0.0, -1.0], [2.0, 12.0, 2.0], 0.0, false);
        b.cube(&parts.lleg, &t, 0.0, 16.0, [-1.0, 0.0, -1.0], [2.0, 12.0, 2.0], 0.0, true);
    } else {
        b.cube(&parts.rarm, &t, 40.0, 16.0, [-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], 0.0, false);
        b.cube(&parts.rleg, &t, 0.0, 16.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], 0.0, false);
        if wide_left {
            // The player layout: the left limbs have their own place on the texture.
            b.cube(&parts.larm, &t, 32.0, 48.0, [-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], 0.0, false);
            b.cube(&parts.lleg, &t, 16.0, 48.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], 0.0, false);
            // And an outer layer over each part.
            b.cube(&parts.body, &t, 16.0, 32.0, [-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], 0.25, false);
            b.cube(&parts.rarm, &t, 40.0, 32.0, [-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], 0.25, false);
            b.cube(&parts.larm, &t, 48.0, 48.0, [-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], 0.25, false);
            b.cube(&parts.rleg, &t, 0.0, 32.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], 0.25, false);
            b.cube(&parts.lleg, &t, 0.0, 48.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], 0.25, false);
        } else {
            b.cube(&parts.larm, &t, 40.0, 16.0, [-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], 0.0, true);
            b.cube(&parts.lleg, &t, 0.0, 16.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], 0.0, true);
        }
    }
    // The outer layers of drowned, strays and bogged: the same body a little bigger.
    let outer = match id {
        "drowned" => Some(("entity/zombie/drowned_outer_layer", 64.0)),
        "stray" => Some(("entity/skeleton/stray_overlay", 32.0)),
        "bogged" => Some(("entity/skeleton/bogged_overlay", 32.0)),
        _ => None,
    };
    if let Some((file, h)) = outer {
        let o = Tex::new(file, 64.0, h);
        b.cube(&parts.head, &o, 0.0, 0.0, [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], 0.25, false);
        b.cube(&parts.body, &o, 16.0, 16.0, [-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], 0.25, false);
        b.cube(&parts.rarm, &o, 40.0, 16.0, [-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], 0.25, false);
        b.cube(&parts.larm, &o, 40.0, 16.0, [-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], 0.25, true);
        b.cube(&parts.rleg, &o, 0.0, 16.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], 0.25, false);
        b.cube(&parts.lleg, &o, 0.0, 16.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], 0.25, true);
    }
}

/// Villagers, wandering traders, witches, zombie villagers and illagers: the villager body
/// (arms folded, robe), with each one's texture and, for villagers, their biome's clothes and
/// their job's.
fn villager(b: &mut Builder, e: &Tag, id: &str, r: &Mat) {
    let (file, h) = match id {
        "wandering_trader" => ("entity/wandering_trader/wandering_trader".to_string(), 64.0),
        "witch" => ("entity/witch/witch".to_string(), 128.0),
        "zombie_villager" => ("entity/zombie_villager/zombie_villager".to_string(), 64.0),
        "villager" => ("entity/villager/villager".to_string(), 64.0),
        other => (format!("entity/illager/{other}"), 64.0),
    };
    let mut layers = vec![Tex::new(&file, 64.0, h)];
    let mut brim = false;
    // A villager's clothes: its biome's, then its job's.
    if matches!(id, "villager" | "zombie_villager") {
        let data = e.get("VillagerData");
        let kind = data.and_then(|d| d.get("type")).and_then(Tag::as_str).map(|s| s.trim_start_matches("minecraft:")).unwrap_or("plains");
        let job = data.and_then(|d| d.get("profession")).and_then(Tag::as_str).map(|s| s.trim_start_matches("minecraft:")).unwrap_or("none");
        let folder = if id == "villager" { "villager" } else { "zombie_villager" };
        layers.push(Tex::new(&format!("entity/{folder}/type/{kind}"), 64.0, 64.0));
        if job != "none" {
            layers.push(Tex::new(&format!("entity/{folder}/profession/{job}"), 64.0, 64.0));
            // Only the jobs with a wide hat have its brim (a flat 16 x 16 plate, which would otherwise
            // show in the outline of every villager).
            brim = matches!(job, "farmer" | "fisherman" | "shepherd");
        }
    }
    let zombie = id == "zombie_villager";
    // A rider sits: its legs forward and turned out a little.
    let riding = e.get("MctkMount").is_some();
    let (rleg, lleg) = if riding { ([-1.4137, 0.3142, 0.0], [-1.4137, -0.3142, 0.0]) } else { ([0.0; 3], [0.0; 3]) };
    // Villagers (and the trader) have a hat over the head and a brim, where the jobs' and
    // biomes' hats are painted (a farmer's straw hat).
    let hatted = matches!(id, "villager" | "zombie_villager" | "wandering_trader");
    for (k, t) in layers.iter().enumerate() {
        // Each layer over the one before by a tenth of a pixel: the game draws them on the same
        // boxes, which here would fight for the same depth and flicker.
        let g = k as f32 * 0.1;
        let head = r.pose([0.0; 3], [0.0; 3]);
        b.cube(&head, t, 0.0, 0.0, [-4.0, -10.0, -4.0], [8.0, 10.0, 8.0], g, false);
        if hatted {
            b.cube(&head, t, 32.0, 0.0, [-4.0, -10.0, -4.0], [8.0, 10.0, 8.0], 0.51 + g, false);
            // (The brim belongs to the job's layer, the last one.)
            if (brim && k + 1 == layers.len()) || id == "wandering_trader" {
                b.cube(&head.pose([0.0; 3], [-HALF, 0.0, 0.0]), t, 30.0, 47.0, [-8.0, -8.0, -6.0], [16.0, 16.0, 1.0], g, false);
            }
        }
        b.cube(&head.pose([0.0, -2.0, 0.0], [0.0; 3]), t, 24.0, 0.0, [-1.0, -1.0, -6.0], [2.0, 4.0, 2.0], g, false);
        b.cube(&head, t, 16.0, 20.0, [-4.0, 0.0, -3.0], [8.0, 12.0, 6.0], g, false);
        b.cube(&head, t, 0.0, 38.0, [-4.0, 0.0, -3.0], [8.0, 20.0, 6.0], 0.5 + g, false);
        if zombie {
            // Zombie villagers hold their arms out.
            let arm = [-HALF + 0.1, 0.0, 0.0];
            b.cube(&r.pose([-5.0, 2.0, 0.0], arm), t, 44.0, 22.0, [-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], g, false);
            b.cube(&r.pose([5.0, 2.0, 0.0], arm), t, 44.0, 22.0, [-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], g, true);
        } else {
            let arms = r.pose([0.0, 3.0, -1.0], [-0.75, 0.0, 0.0]);
            b.cube(&arms, t, 44.0, 22.0, [-8.0, -2.0, -2.0], [4.0, 8.0, 4.0], g, false);
            b.cube(&arms, t, 44.0, 22.0, [4.0, -2.0, -2.0], [4.0, 8.0, 4.0], g, true);
            b.cube(&arms, t, 40.0, 38.0, [-4.0, 2.0, -2.0], [8.0, 4.0, 4.0], g, false);
        }
        b.cube(&r.pose([-2.0, 12.0, 0.0], rleg), t, 0.0, 22.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], g, false);
        b.cube(&r.pose([2.0, 12.0, 0.0], lleg), t, 0.0, 22.0, [-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], g, true);
    }
    if id == "witch" {
        // Her hat: a brim and three narrowing tiers.
        let t = &layers[0];
        let hat = r.pose([-5.0, -10.03125, -5.0], [0.0; 3]);
        b.cube(&hat, t, 0.0, 64.0, [0.0, 0.0, 0.0], [10.0, 2.0, 10.0], 0.0, false);
        let h2 = hat.pose([1.75, -4.0, 2.0], [-0.05, 0.0, 0.026]);
        b.cube(&h2, t, 0.0, 76.0, [0.0, 0.0, 0.0], [7.0, 4.0, 7.0], 0.0, false);
        let h3 = h2.pose([1.75, -4.0, 2.0], [-0.1, 0.0, 0.05]);
        b.cube(&h3, t, 0.0, 87.0, [0.0, 0.0, 0.0], [4.0, 4.0, 4.0], 0.0, false);
        let h4 = h3.pose([1.75, -2.0, 2.0], [-0.2, 0.0, 0.1]);
        b.cube(&h4, t, 0.0, 95.0, [0.0, 0.0, 0.0], [1.0, 2.0, 1.0], 0.25, false);
    }
}

/// A marker for a mob not built here: its spawn egg, floating where it stands.
pub fn marker(b: &mut Builder, id: &str, local: &Mat, yaw: f32) {
    let m = local.t(0.0, 0.6, 0.0).ry(-yaw * PI / 180.0).s(0.6, 0.6, 0.6).t(-0.5, -0.5, -0.5);
    b.item(&m, &format!("{id}_spawn_egg"), [1.0; 3]);
}

/// A box of a baby's model: where on its picture (u, v), where in the model (its corner `o`
/// and size `d`, model pixels, y down, feet at 24) under the part pose `pose`.
struct Part {
    uv: [f32; 2],
    o: [f32; 3],
    d: [f32; 3],
    pivot: [f32; 3],
    rot: [f32; 3],
}

const fn part(uv: [f32; 2], o: [f32; 3], d: [f32; 3]) -> Part {
    Part { uv, o, d, pivot: [0.0; 3], rot: [0.0; 3] }
}

fn parts(b: &mut Builder, r: &Mat, t: &Tex, list: &[Part], grow: f32) {
    for p in list {
        b.cube(&r.pose(p.pivot, p.rot), t, p.uv[0], p.uv[1], p.o, p.d, grow, false);
    }
}

/// Four legs of `d` (x, y, z) standing at x = ±`x` and z = `front`/`back` (their middles), from
/// the picture's places `uvs` (front right, front left, back right, back left).
fn baby_legs(x: f32, front: f32, back: f32, d: [f32; 3], uvs: [[f32; 2]; 4]) -> [Part; 4] {
    let at = |cx: f32, cz: f32, uv: [f32; 2]| part(uv, [cx - d[0] / 2.0, 24.0 - d[1], cz - d[2] / 2.0], d);
    [at(-x, front, uvs[0]), at(x, front, uvs[1]), at(-x, back, uvs[2]), at(x, back, uvs[3])]
}

/// A baby from 26.x's own baby models: false for one that has none (drawn as a small adult).
fn baby_model(b: &mut Builder, e: &Tag, id: &str, r: &Mat) -> bool {
    match id {
        "pig" => {
            let v = variant(e, "variant").unwrap_or_else(|| "temperate".into());
            let t = Tex::new(&format!("entity/pig/pig_{v}_baby"), 32.0, 32.0);
            parts(b, r, &t, &[
                part([0.0, 0.0], [-3.5, 16.0, -4.5], [7.0, 6.0, 9.0]),
                part([0.0, 15.0], [-3.5, 13.0, -9.5], [7.0, 6.0, 6.0]),
                part([6.0, 27.0], [-1.5, 16.0, -10.5], [3.0, 2.0, 1.0]),
            ], 0.0);
            parts(b, r, &t, &baby_legs(2.0, -3.0, 3.0, [2.0, 2.0, 2.0], [[0.0, 0.0], [23.0, 0.0], [0.0, 4.0], [23.0, 4.0]]), 0.0);
        }
        "cow" | "mooshroom" => {
            let file = if id == "mooshroom" {
                format!("entity/cow/mooshroom_{}_baby", variant(e, "Type").unwrap_or_else(|| "red".into()))
            } else {
                format!("entity/cow/cow_{}_baby", variant(e, "variant").unwrap_or_else(|| "temperate".into()))
            };
            let t = Tex::new(&file, 64.0, 64.0);
            parts(b, r, &t, &[
                part([0.0, 0.0], [-4.0, 12.0, -6.0], [8.0, 6.0, 12.0]),
                part([0.0, 18.0], [-3.0, 8.0, -10.0], [6.0, 6.0, 5.0]),
                part([12.0, 29.0], [-2.0, 11.0, -11.0], [4.0, 2.0, 1.0]),
                part([4.0, 29.0], [-3.5, 6.0, -8.0], [1.0, 2.0, 1.0]),
                part([8.0, 29.0], [2.5, 6.0, -8.0], [1.0, 2.0, 1.0]),
            ], 0.0);
            parts(b, r, &t, &baby_legs(2.5, -4.5, 4.5, [3.0, 6.0, 3.0], [[22.0, 18.0], [34.0, 18.0], [22.0, 27.0], [34.0, 27.0]]), 0.0);
        }
        "sheep" => {
            let t = Tex::new("entity/sheep/sheep_baby", 64.0, 32.0);
            let body = [
                part([0.0, 0.0], [-2.5, 9.0, -8.5], [5.0, 5.0, 5.0]),
                part([0.0, 10.0], [-3.0, 13.0, -4.5], [6.0, 5.0, 9.0]),
            ];
            parts(b, r, &t, &body, 0.0);
            let legs = [
                part([24.0, 5.0], [-3.0, 19.0, -4.0], [2.0, 5.0, 2.0]),
                part([24.0, 12.0], [1.0, 19.0, -4.0], [2.0, 5.0, 2.0]),
                part([0.0, 24.0], [-3.0, 20.0, 2.0], [2.0, 4.0, 2.0]),
                part([8.0, 24.0], [1.0, 20.0, 2.0], [2.0, 4.0, 2.0]),
            ];
            parts(b, r, &t, &legs, 0.0);
            if e.get("Sheared").and_then(Tag::as_i64).unwrap_or(0) == 0 {
                let w = Tex::new("entity/sheep/sheep_wool_baby", 64.0, 32.0);
                let color = e.get("Color").and_then(Tag::as_i64).unwrap_or(0).clamp(0, 15) as usize;
                b.tint = crate::entities::dye(["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"][color]);
                parts(b, r, &w, &body[1..], 0.75);
                b.tint = [1.0; 3];
            }
        }
        "chicken" => {
            let v = variant(e, "variant").unwrap_or_else(|| "temperate".into());
            let t = Tex::new(&format!("entity/chicken/chicken_{v}_baby"), 16.0, 16.0);
            parts(b, r, &t, &[
                part([0.0, 0.0], [-2.0, 19.0, -2.0], [4.0, 4.0, 4.0]),
                part([5.0, 8.0], [-0.5, 21.0, -3.0], [1.0, 1.0, 1.0]),
                part([10.0, 8.0], [-2.5, 23.0, -1.0], [2.0, 1.0, 1.0]),
                part([10.0, 8.0], [0.5, 23.0, -1.0], [2.0, 1.0, 1.0]),
            ], 0.0);
        }
        "wolf" => {
            let v = variant(e, "variant").unwrap_or_else(|| "pale".into());
            let file = if v == "pale" { "entity/wolf/wolf_baby".to_string() } else { format!("entity/wolf/wolf_{v}_baby") };
            let t = Tex::new(&file, 32.0, 32.0);
            parts(b, r, &t, &[
                part([0.0, 0.0], [-3.0, 14.0, -4.0], [6.0, 4.0, 8.0]),
                part([0.0, 12.0], [-3.0, 10.0, -8.0], [6.0, 5.0, 5.0]),
                part([16.0, 12.0], [-1.5, 13.0, -10.0], [3.0, 2.0, 2.0]),
                Part { uv: [20.0, 16.0], o: [-1.5, 0.0, 0.0], d: [3.0, 6.0, 2.0], pivot: [0.0, 15.0, 4.0], rot: [0.9, 0.0, 0.0] },
            ], 0.0);
            parts(b, r, &t, &[
                part([0.0, 0.0], [-3.0, 18.0, -4.0], [2.0, 6.0, 2.0]),
                part([20.0, 0.0], [1.0, 18.0, -4.0], [2.0, 6.0, 2.0]),
                part([0.0, 22.0], [-3.0, 20.0, 2.0], [2.0, 4.0, 2.0]),
                part([8.0, 22.0], [1.0, 20.0, 2.0], [2.0, 4.0, 2.0]),
            ], 0.0);
        }
        "cat" | "ocelot" => {
            let file = if id == "ocelot" {
                "entity/cat/ocelot_baby".to_string()
            } else {
                format!("entity/cat/cat_{}_baby", variant(e, "variant").unwrap_or_else(|| "tabby".into()))
            };
            let t = Tex::new(&file, 32.0, 32.0);
            parts(b, r, &t, &[
                part([0.0, 8.0], [-2.0, 19.0, -3.5], [4.0, 3.0, 7.0]),
                part([0.0, 0.0], [-2.5, 16.0, -7.0], [5.0, 4.0, 4.0]),
                part([18.0, 3.0], [-1.5, 18.0, -8.0], [3.0, 2.0, 1.0]),
                part([18.0, 0.0], [-2.5, 15.0, -5.0], [1.0, 1.0, 2.0]),
                part([24.0, 0.0], [1.5, 15.0, -5.0], [1.0, 1.0, 2.0]),
                Part { uv: [0.0, 18.0], o: [-0.5, 0.0, 0.0], d: [1.0, 1.0, 5.0], pivot: [0.0, 19.5, 3.5], rot: [-0.5, 0.0, 0.0] },
            ], 0.0);
            parts(b, r, &t, &baby_legs(1.5, -2.5, 2.5, [1.0, 2.0, 1.0], [[12.0, 20.0], [16.0, 20.0], [12.0, 24.0], [16.0, 24.0]]), 0.0);
        }
        "fox" => {
            let snow = variant(e, "Type").is_some_and(|t| t == "snow");
            let t = Tex::new(if snow { "entity/fox/fox_snow_baby" } else { "entity/fox/fox_baby" }, 32.0, 32.0);
            parts(b, r, &t, &[
                part([0.0, 10.0], [-2.5, 18.0, -3.0], [5.0, 4.0, 6.0]),
                part([0.0, 0.0], [-3.0, 14.0, -7.5], [6.0, 5.0, 5.0]),
                part([18.0, 20.0], [-1.0, 17.0, -9.5], [2.0, 2.0, 2.0]),
                part([22.0, 8.0], [-2.5, 12.0, -5.5], [1.0, 2.0, 1.0]),
                part([22.0, 11.0], [1.5, 12.0, -5.5], [1.0, 2.0, 1.0]),
                Part { uv: [0.0, 20.0], o: [-1.0, 0.0, 0.0], d: [2.0, 3.0, 6.0], pivot: [0.0, 18.5, 3.0], rot: [-0.4, 0.0, 0.0] },
            ], 0.0);
            parts(b, r, &t, &baby_legs(1.5, -2.0, 2.0, [2.0, 2.0, 2.0], [[22.0, 0.0], [22.0, 0.0], [22.0, 4.0], [22.0, 4.0]]), 0.0);
        }
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" => {
            let file = match id {
                "donkey" => "entity/horse/donkey_baby".to_string(),
                "mule" => "entity/horse/mule_baby".to_string(),
                "skeleton_horse" => "entity/horse/horse_skeleton_baby".to_string(),
                "zombie_horse" => "entity/horse/horse_zombie_baby".to_string(),
                _ => {
                    let coat = ["white", "creamy", "chestnut", "brown", "black", "gray", "darkbrown"][(e.get("Variant").and_then(Tag::as_i64).unwrap_or(0) & 0xFF).clamp(0, 6) as usize];
                    format!("entity/horse/horse_{coat}_baby")
                }
            };
            let t = Tex::new(&file, 64.0, 64.0);
            parts(b, r, &t, &[
                part([0.0, 13.0], [-4.0, 8.0, -7.0], [8.0, 7.0, 14.0]),
                Part { uv: [30.0, 0.0], o: [-2.0, -7.0, -2.0], d: [4.0, 7.0, 4.0], pivot: [0.0, 10.0, -6.0], rot: [0.35, 0.0, 0.0] },
                Part { uv: [0.0, 0.0], o: [-3.0, -3.0, -9.0], d: [6.0, 3.0, 9.0], pivot: [0.0, 3.5, -6.5], rot: [0.35, 0.0, 0.0] },
                Part { uv: [32.0, 34.0], o: [-1.5, 0.0, 0.0], d: [3.0, 8.0, 3.0], pivot: [0.0, 9.0, 6.5], rot: [0.5, 0.0, 0.0] },
            ], 0.0);
            parts(b, r, &t, &baby_legs(2.5, -5.0, 5.0, [3.0, 9.0, 3.0], [[0.0, 34.0], [12.0, 34.0], [0.0, 46.0], [12.0, 46.0]]), 0.0);
        }
        "goat" => {
            let t = Tex::new("entity/goat/goat_baby", 64.0, 64.0);
            parts(b, r, &t, &[
                part([0.0, 10.0], [-3.0, 14.0, -4.5], [6.0, 5.0, 9.0]),
                part([0.0, 24.0], [-2.5, 17.0, -4.0], [5.0, 3.0, 8.0]),
                part([0.0, 0.0], [-2.0, 10.0, -9.5], [4.0, 4.0, 6.0]),
                part([20.0, 0.0], [-1.5, 8.0, -6.5], [1.0, 2.0, 1.0]),
                part([24.0, 0.0], [0.5, 8.0, -6.5], [1.0, 2.0, 1.0]),
            ], 0.0);
            parts(b, r, &t, &baby_legs(2.0, -3.0, 3.0, [2.0, 5.0, 2.0], [[21.0, 5.0], [29.0, 5.0], [21.0, 12.0], [29.0, 12.0]]), 0.0);
        }
        "llama" | "trader_llama" => {
            let coat = ["creamy", "white", "brown", "gray"][e.get("Variant").and_then(Tag::as_i64).unwrap_or(0).clamp(0, 3) as usize];
            let t = Tex::new(&format!("entity/llama/llama_{coat}_baby"), 64.0, 64.0);
            parts(b, r, &t, &[
                part([0.0, 15.0], [-4.0, 10.0, -6.5], [8.0, 6.0, 13.0]),
                part([0.0, 0.0], [-3.0, 1.0, -8.0], [6.0, 10.0, 4.0]),
                part([0.0, 15.0], [-1.5, 3.0, -11.0], [3.0, 3.0, 3.0]),
                part([20.0, 0.0], [-2.5, -2.0, -6.5], [1.0, 3.0, 1.0]),
                part([20.0, 4.0], [1.5, -2.0, -6.5], [1.0, 3.0, 1.0]),
            ], 0.0);
            parts(b, r, &t, &baby_legs(2.5, -4.5, 4.5, [3.0, 8.0, 3.0], [[0.0, 34.0], [12.0, 34.0], [0.0, 45.0], [12.0, 45.0]]), 0.0);
        }
        "panda" => {
            let gene = e.get("MainGene").and_then(Tag::as_str).unwrap_or("normal");
            let file = match gene {
                "lazy" | "playful" | "aggressive" | "weak" | "worried" | "brown" => format!("entity/panda/{gene}_panda_baby"),
                _ => "entity/panda/panda_baby".to_string(),
            };
            let t = Tex::new(&file, 64.0, 64.0);
            parts(b, r, &t, &[
                part([0.0, 10.0], [-4.5, 14.0, -5.5], [9.0, 7.0, 11.0]),
                part([0.0, 0.0], [-3.5, 10.0, -10.0], [7.0, 5.0, 5.0]),
                part([24.0, 6.0], [-1.5, 12.5, -12.0], [3.0, 2.0, 2.0]),
                part([24.0, 0.0], [-4.5, 8.0, -8.0], [3.0, 3.0, 1.0]),
                part([33.0, 0.0], [1.5, 8.0, -8.0], [3.0, 3.0, 1.0]),
            ], 0.0);
            parts(b, r, &t, &baby_legs(3.0, -3.5, 3.5, [3.0, 3.0, 3.0], [[0.0, 28.0], [12.0, 28.0], [0.0, 34.0], [12.0, 34.0]]), 0.0);
        }
        "polar_bear" => {
            let t = Tex::new("entity/bear/polarbear_baby", 64.0, 64.0);
            parts(b, r, &t, &[
                part([0.0, 9.0], [-4.0, 14.0, -6.0], [8.0, 7.0, 12.0]),
                part([0.0, 0.0], [-3.0, 12.0, -10.0], [6.0, 5.0, 4.0]),
                part([20.0, 2.0], [-2.0, 14.0, -12.0], [4.0, 3.0, 2.0]),
            ], 0.0);
            parts(b, r, &t, &baby_legs(2.5, -4.0, 4.0, [3.0, 3.0, 3.0], [[0.0, 28.0], [12.0, 28.0], [0.0, 34.0], [12.0, 34.0]]), 0.0);
        }
        "rabbit" => {
            let kind = match e.get("RabbitType").and_then(Tag::as_i64) {
                Some(1) => "white",
                Some(2) => "black",
                Some(3) => "white_splotched",
                Some(4) => "gold",
                Some(5) => "salt",
                Some(99) => "caerbannog",
                _ => "brown",
            };
            let t = Tex::new(&format!("entity/rabbit/rabbit_{kind}_baby"), 32.0, 32.0);
            parts(b, r, &t, &[
                part([0.0, 8.0], [-2.0, 19.0, -2.0], [4.0, 3.0, 6.0]),
                part([0.0, 0.0], [-2.5, 16.0, -5.0], [5.0, 4.0, 4.0]),
                part([18.0, 0.0], [-2.0, 12.0, -3.0], [2.0, 4.0, 1.0]),
                part([24.0, 0.0], [0.0, 12.0, -3.0], [2.0, 4.0, 1.0]),
                part([14.0, 8.0], [-1.5, 18.0, 4.0], [3.0, 3.0, 1.0]),
                part([0.0, 17.0], [-2.5, 22.0, 1.0], [2.0, 2.0, 2.0]),
                part([10.0, 17.0], [0.5, 22.0, 1.0], [2.0, 2.0, 2.0]),
            ], 0.0);
        }
        "bee" => {
            let angry = e.get("AngerTime").and_then(Tag::as_i64).unwrap_or(0) > 0;
            let nectar = e.get("HasNectar").and_then(Tag::as_i64).unwrap_or(0) != 0;
            let file = format!("entity/bee/bee{}{}_baby", if angry { "_angry" } else { "" }, if nectar { "_nectar" } else { "" });
            let t = Tex::new(&file, 32.0, 32.0);
            parts(b, r, &t, &[
                part([0.0, 0.0], [-2.0, 17.0, -2.5], [4.0, 4.0, 5.0]),
                part([13.0, 0.0], [-1.5, 15.0, -3.5], [1.0, 2.0, 0.0]),
                part([13.0, 0.0], [0.5, 15.0, -3.5], [1.0, 2.0, 0.0]),
                part([0.0, 9.0], [-3.5, 16.9, -1.0], [3.0, 0.0, 3.0]),
                part([6.0, 9.0], [0.5, 16.9, -1.0], [3.0, 0.0, 3.0]),
            ], 0.0);
        }
        "turtle" => {
            let t = Tex::new("entity/turtle/turtle_baby", 16.0, 16.0);
            parts(b, r, &t, &[
                part([0.0, 0.0], [-2.0, 21.0, -2.0], [4.0, 2.0, 4.0]),
                part([0.0, 6.0], [-1.5, 20.5, -4.5], [3.0, 3.0, 3.0]),
                part([0.0, 0.0], [-4.0, 23.0, -2.0], [2.0, 1.0, 1.0]),
                part([0.0, 0.0], [2.0, 23.0, -2.0], [2.0, 1.0, 1.0]),
                part([0.0, 0.0], [-3.0, 23.0, 1.0], [1.0, 1.0, 1.0]),
                part([0.0, 0.0], [2.0, 23.0, 1.0], [1.0, 1.0, 1.0]),
            ], 0.0);
        }
        "dolphin" => {
            let t = Tex::new("entity/dolphin/dolphin_baby", 64.0, 64.0);
            parts(b, r, &t, &[
                part([20.0, 0.0], [-3.0, 17.0, -4.0], [6.0, 5.0, 8.0]),
                part([0.0, 0.0], [-3.0, 17.0, -8.0], [6.0, 5.0, 4.0]),
                part([0.0, 9.0], [-1.0, 20.0, -10.0], [2.0, 2.0, 2.0]),
                part([0.0, 13.0], [-2.0, 18.0, 4.0], [4.0, 3.0, 7.0]),
                part([22.0, 13.0], [-4.0, 19.0, 10.0], [8.0, 1.0, 4.0]),
                part([42.0, 0.0], [-0.5, 14.0, -2.0], [1.0, 3.0, 4.0]),
            ], 0.0);
        }
        "armadillo" => {
            let t = Tex::new("entity/armadillo/armadillo_baby", 64.0, 64.0);
            parts(b, r, &t, &[
                part([0.0, 11.0], [-2.5, 18.0, -3.0], [5.0, 4.0, 6.0]),
                part([0.0, 0.0], [-2.5, 17.5, -3.5], [5.0, 3.0, 7.0]),
                part([22.0, 17.0], [-1.5, 18.0, -5.5], [3.0, 3.0, 2.0]),
            ], 0.0);
            parts(b, r, &t, &baby_legs(1.5, -2.0, 2.0, [1.0, 2.0, 1.0], [[24.0, 9.0], [24.0, 9.0], [24.0, 13.0], [24.0, 13.0]]), 0.0);
        }
        "axolotl" => {
            let kind = ["lucy", "wild", "gold", "cyan", "blue"][e.get("Variant").and_then(Tag::as_i64).unwrap_or(0).clamp(0, 4) as usize];
            let t = Tex::new(&format!("entity/axolotl/axolotl_{kind}_baby"), 32.0, 32.0);
            parts(b, r, &t, &[
                part([0.0, 0.0], [-2.0, 21.0, -3.0], [4.0, 2.0, 6.0]),
                part([0.0, 8.0], [-3.0, 20.0, -7.0], [6.0, 3.0, 4.0]),
                part([0.0, 15.0], [0.0, 20.5, 3.0], [0.0, 3.0, 6.0]),
            ], 0.0);
            parts(b, r, &t, &baby_legs(2.0, -2.0, 2.0, [1.0, 1.0, 1.0], [[20.0, 4.0], [20.0, 4.0], [20.0, 9.0], [20.0, 9.0]]), 0.0);
        }
        "camel" => {
            let t = Tex::new("entity/camel/camel_baby", 64.0, 64.0);
            parts(b, r, &t, &[
                part([0.0, 14.0], [-4.5, 3.0, -8.0], [9.0, 8.0, 16.0]),
                part([20.0, 0.0], [-2.5, -2.0, -3.5], [5.0, 5.0, 7.0]),
                part([0.0, 0.0], [-2.5, -7.0, -12.0], [5.0, 8.0, 5.0]),
                part([0.0, 14.0], [-1.5, -4.0, -15.0], [3.0, 3.0, 3.0]),
            ], 0.0);
            parts(b, r, &t, &baby_legs(3.0, -5.5, 5.5, [3.0, 13.0, 3.0], [[0.0, 38.0], [12.0, 38.0], [36.0, 14.0], [48.0, 14.0]]), 0.0);
        }
        "hoglin" | "zoglin" => {
            let t = Tex::new(&format!("entity/hoglin/{id}_baby"), 64.0, 64.0);
            parts(b, r, &t, &[
                part([0.0, 15.0], [-4.0, 10.0, -7.0], [8.0, 8.0, 14.0]),
                part([44.0, 23.0], [0.0, 5.0, -5.0], [0.0, 5.0, 8.0]),
                Part { uv: [0.0, 0.0], o: [-5.0, -2.0, -12.0], d: [10.0, 4.0, 12.0], pivot: [0.0, 12.0, -6.0], rot: [0.8727, 0.0, 0.0] },
                Part { uv: [32.0, 0.0], o: [-4.0, -0.5, -2.0], d: [4.0, 1.0, 4.0], pivot: [-5.0, 10.0, -7.0], rot: [0.0, 0.0, -0.6] },
                Part { uv: [32.0, 6.0], o: [0.0, -0.5, -2.0], d: [4.0, 1.0, 4.0], pivot: [5.0, 10.0, -7.0], rot: [0.0, 0.0, 0.6] },
            ], 0.0);
            parts(b, r, &t, &baby_legs(2.5, -4.5, 4.5, [3.0, 6.0, 3.0], [[0.0, 38.0], [12.0, 38.0], [0.0, 47.0], [12.0, 47.0]]), 0.0);
        }
        "happy_ghast" => {
            // The ghastling: the adult's boxes on its own picture, at its own size.
            let t = Tex::new("entity/ghast/happy_ghast_baby", 64.0, 64.0);
            b.cube(&r.pose([0.0, 7.0, 0.0], [0.0; 3]), &t, 0.0, 0.0, [-8.0, -8.0, -8.0], [16.0, 16.0, 16.0], 0.0, false);
            for i in 0..9 {
                let fx = ((i % 3) as f32 - (i / 3 % 2) as f32 * 0.5 + 0.25) / 2.0 * 2.0 - 1.0;
                let fz = (i / 3) as f32 / 2.0 * 2.0 - 1.0;
                let len = 4.0 + ((i * 5 + 3) % 4) as f32;
                b.cube(&r.pose([fx * 5.0, 15.0, fz * 5.0], [0.0; 3]), &t, 0.0, 0.0, [-1.0, 0.0, -1.0], [2.0, len, 2.0], 0.0, false);
            }
        }
        "nautilus" => {
            let t = Tex::new("entity/nautilus/nautilus_baby", 64.0, 64.0);
            parts(b, r, &t, &[
                part([0.0, 0.0], [-3.5, 15.0, -3.5], [7.0, 5.0, 7.0]),
                part([0.0, 12.0], [-3.5, 20.0, -3.5], [7.0, 4.0, 10.0]),
                part([2.0, 26.0], [-2.5, 17.0, 3.5], [5.0, 4.0, 5.0]),
                part([24.0, 25.0], [-5.0, 14.0, 8.6], [10.0, 10.0, 0.0]),
            ], 0.0);
        }
        "squid" | "glow_squid" => {
            let t = Tex::new(&format!("entity/squid/{id}_baby"), 32.0, 32.0);
            parts(b, r, &t, &[part([0.0, 0.0], [-4.0, 4.0, -4.0], [8.0, 10.0, 8.0])], 0.0);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0;
                b.cube(&r.pose([a.cos() * 3.0, 14.0, a.sin() * 3.0], [0.0, -a + HALF, 0.0]), &t, 0.0, 18.0, [-1.0, 0.0, -1.0], [2.0, 6.0, 2.0], 0.0, false);
            }
        }
        "zombie" | "husk" | "drowned" => {
            let t = Tex::new(&format!("entity/zombie/{id}_baby"), 64.0, 64.0);
            humanoid_baby(b, r, &t);
        }
        "piglin" | "zombified_piglin" => {
            let t = Tex::new(&format!("entity/piglin/{id}_baby"), 64.0, 64.0);
            parts(b, r, &t, &[
                part([0.0, 0.0], [-4.5, 3.0, -3.5], [9.0, 6.0, 7.0]),
                part([21.0, 30.0], [-1.5, 6.0, -4.5], [3.0, 2.0, 1.0]),
                Part { uv: [0.0, 22.0], o: [-1.0, 0.0, -2.0], d: [1.0, 5.0, 4.0], pivot: [-4.5, 4.0, 0.0], rot: [0.0, 0.0, 0.5] },
                Part { uv: [0.0, 22.0], o: [0.0, 0.0, -2.0], d: [1.0, 5.0, 4.0], pivot: [4.5, 4.0, 0.0], rot: [0.0, 0.0, -0.5] },
                part([2.0, 13.0], [-2.5, 9.0, -1.5], [5.0, 5.0, 3.0]),
                part([16.0, 17.0], [-4.5, 9.0, -1.0], [2.0, 4.0, 2.0]),
                part([28.0, 16.0], [2.5, 9.0, -1.0], [2.0, 4.0, 2.0]),
                part([10.0, 24.0], [-3.0, 14.0, -1.5], [3.0, 3.0, 3.0]),
                part([22.0, 24.0], [0.0, 14.0, -1.5], [3.0, 3.0, 3.0]),
            ], 0.0);
        }
        "villager" | "zombie_villager" => {
            let folder = if id == "villager" { "villager" } else { "zombie_villager" };
            let mut layers = vec![Tex::new(&format!("entity/{folder}/{folder}_baby"), 64.0, 64.0)];
            if let Some(kind) = e.get("VillagerData").and_then(|d| d.get("type")).and_then(Tag::as_str) {
                layers.push(Tex::new(&format!("entity/{folder}/baby/type/{}", kind.trim_start_matches("minecraft:")), 64.0, 64.0));
            }
            for (k, t) in layers.iter().enumerate() {
                let g = k as f32 * 0.1;
                parts(b, r, t, &[
                    part([0.0, 0.0], [-4.0, 1.0, -3.5], [8.0, 8.0, 7.0]),
                    part([24.0, 0.0], [-1.0, 6.0, -4.5], [2.0, 2.0, 1.0]),
                    part([0.0, 16.0], [-2.5, 9.0, -1.5], [5.0, 4.0, 3.0]),
                    part([16.0, 16.0], [-2.5, 10.0, -3.0], [5.0, 3.0, 2.0]),
                    part([0.0, 23.0], [-2.5, 13.0, -1.0], [2.0, 3.0, 2.0]),
                    part([8.0, 23.0], [0.5, 13.0, -1.0], [2.0, 3.0, 2.0]),
                ], g);
            }
        }
        "strider" => {
            let cold = e.get("Health").and_then(Tag::as_f64).is_some_and(|_| false);
            let t = Tex::new(if cold { "entity/strider/strider_cold_baby" } else { "entity/strider/strider_baby" }, 32.0, 32.0);
            parts(b, r, &t, &[
                part([0.0, 0.0], [-3.5, 13.0, -4.0], [7.0, 7.0, 8.0]),
                part([0.0, 24.0], [-3.0, 20.0, -1.0], [2.0, 4.0, 2.0]),
                part([8.0, 24.0], [1.0, 20.0, -1.0], [2.0, 4.0, 2.0]),
            ], 0.0);
        }
        _ => return false,
    }
    true
}

/// The 26.x baby zombie's boxes (zombies, husks, drowned): a big head on a small body.
fn humanoid_baby(b: &mut Builder, r: &Mat, t: &Tex) {
    parts(b, r, t, &[
        part([3.0, 3.0], [-3.0, 5.0, -3.0], [6.0, 6.0, 6.0]),
        part([16.0, 16.0], [-2.5, 11.0, -1.0], [5.0, 5.0, 2.0]),
        Part { uv: [28.0, 16.0], o: [-2.0, 0.0, -1.0], d: [2.0, 5.0, 2.0], pivot: [-2.5, 11.5, 0.0], rot: [-HALF, 0.0, 0.0] },
        Part { uv: [36.0, 16.0], o: [0.0, 0.0, -1.0], d: [2.0, 5.0, 2.0], pivot: [2.5, 11.5, 0.0], rot: [-HALF, 0.0, 0.0] },
        part([0.0, 16.0], [-2.5, 16.0, -1.0], [2.0, 4.0, 2.0]),
        part([8.0, 16.0], [0.5, 16.0, -1.0], [2.0, 4.0, 2.0]),
    ], 0.0);
}

#[cfg(test)]
mod tests {
    use mctk_nbt::{Compound, List, Tag, TagType};

    fn mob(id: &str, riding: bool) -> Tag {
        let nums = |v: &[f64]| Tag::List(List::from_tags(TagType::Double, v.iter().map(|n| Tag::Double(*n)).collect()).expect("numbers"));
        let mut c = Compound::new();
        c.insert("id".into(), Tag::String(format!("minecraft:{id}")));
        c.insert("Pos".into(), nums(&[0.0, 0.0, 0.0]));
        if riding {
            c.insert("MctkMount".into(), nums(&[0.0, 0.0, 0.0]));
        }
        Tag::Compound(c)
    }

    /// The lowest point of the model: a sitting rider's legs are held out, so it is higher.
    fn reach(t: &Tag) -> f32 {
        let quads = crate::entities::entity(t, [0.0; 3]).expect("a model").0;
        quads.iter().flat_map(|q| q.pos.iter()).map(|p| p[1]).fold(f32::MAX, f32::min)
    }

    #[test]
    fn riders_sit_with_their_legs_forward() {
        // (A villager's long robe hangs to the ground over its legs, so it is not judged by its lowest point.)
        for id in ["zombie", "skeleton", "piglin"] {
            let (standing, sitting) = (reach(&mob(id, false)), reach(&mob(id, true)));
            assert!(sitting > standing + 0.3, "{id}: {standing} {sitting}");
        }
    }
}
