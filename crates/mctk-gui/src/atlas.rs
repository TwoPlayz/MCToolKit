//! The Biome Atlas: the map far out drawn as flat biome colors with soft borders between
//! biomes, in place of the shaded terrain. Columns whose save does not record a biome are drawn
//! by what lies on top (water, sand, snow, trees...) in the same flat style.

use eframe::egui;

use crate::world::{self, MapImage};

/// Cells across and down one atlas tile.
pub const TILE: usize = 96;

/// A biome's color on the atlas.
pub fn biome_color(name: &str) -> [u8; 3] {
    let n = name.trim_start_matches("minecraft:");
    const EXACT: &[(&str, [u8; 3])] = &[
        ("ocean", [26, 66, 150]),
        ("deep_ocean", [17, 44, 112]),
        ("warm_ocean", [36, 120, 178]),
        ("lukewarm_ocean", [30, 92, 168]),
        ("deep_lukewarm_ocean", [22, 70, 138]),
        ("cold_ocean", [38, 64, 132]),
        ("deep_cold_ocean", [28, 48, 110]),
        ("frozen_ocean", [118, 140, 196]),
        ("deep_frozen_ocean", [84, 104, 166]),
        ("river", [52, 108, 210]),
        ("frozen_river", [152, 172, 224]),
        ("beach", [226, 212, 152]),
        ("snowy_beach", [236, 236, 218]),
        ("stony_shore", [132, 132, 126]),
        ("plains", [134, 184, 86]),
        ("sunflower_plains", [166, 194, 82]),
        ("snowy_plains", [236, 242, 246]),
        ("ice_spikes", [184, 216, 240]),
        ("desert", [226, 198, 112]),
        ("forest", [62, 132, 56]),
        ("flower_forest", [104, 152, 62]),
        ("birch_forest", [94, 154, 78]),
        ("old_growth_birch_forest", [82, 142, 72]),
        ("dark_forest", [38, 84, 36]),
        ("pale_garden", [150, 160, 150]),
        ("taiga", [58, 108, 88]),
        ("old_growth_pine_taiga", [74, 94, 68]),
        ("old_growth_spruce_taiga", [66, 88, 62]),
        ("snowy_taiga", [142, 172, 162]),
        ("jungle", [42, 142, 32]),
        ("sparse_jungle", [84, 154, 52]),
        ("bamboo_jungle", [94, 172, 42]),
        ("savanna", [182, 172, 92]),
        ("savanna_plateau", [166, 156, 86]),
        ("windswept_savanna", [192, 168, 112]),
        ("badlands", [204, 102, 52]),
        ("wooded_badlands", [172, 112, 62]),
        ("eroded_badlands", [218, 92, 46]),
        ("swamp", [72, 96, 62]),
        ("mangrove_swamp", [62, 112, 72]),
        ("mushroom_fields", [162, 112, 162]),
        ("windswept_hills", [112, 126, 106]),
        ("windswept_gravelly_hills", [128, 128, 122]),
        ("windswept_forest", [82, 112, 82]),
        ("meadow", [142, 192, 112]),
        ("cherry_grove", [236, 172, 202]),
        ("grove", [192, 212, 202]),
        ("snowy_slopes", [226, 236, 242]),
        ("frozen_peaks", [202, 222, 242]),
        ("jagged_peaks", [232, 236, 246]),
        ("stony_peaks", [152, 146, 140]),
        ("dripstone_caves", [132, 102, 72]),
        ("lush_caves", [92, 162, 62]),
        ("deep_dark", [22, 32, 42]),
        ("nether_wastes", [132, 42, 36]),
        ("crimson_forest", [172, 32, 42]),
        ("warped_forest", [32, 142, 132]),
        ("soul_sand_valley", [92, 76, 62]),
        ("basalt_deltas", [82, 76, 82]),
        ("the_end", [222, 222, 162]),
        ("end_highlands", [202, 202, 142]),
        ("end_midlands", [212, 212, 152]),
        ("small_end_islands", [192, 192, 142]),
        ("end_barrens", [182, 182, 132]),
        ("the_void", [12, 12, 18]),
    ];
    if let Some((_, c)) = EXACT.iter().find(|(k, _)| *k == n) {
        return *c;
    }
    // Older or modded names: the nearest family by the words in them.
    const FAMILY: &[(&str, [u8; 3])] = &[
        ("deep", [17, 44, 112]),
        ("ocean", [26, 66, 150]),
        ("river", [52, 108, 210]),
        ("beach", [226, 212, 152]),
        ("shore", [132, 132, 126]),
        ("frozen", [202, 222, 242]),
        ("snow", [236, 242, 246]),
        ("ice", [184, 216, 240]),
        ("desert", [226, 198, 112]),
        ("mesa", [204, 102, 52]),
        ("badlands", [204, 102, 52]),
        ("jungle", [42, 142, 32]),
        ("swamp", [72, 96, 62]),
        ("mushroom", [162, 112, 162]),
        ("savanna", [182, 172, 92]),
        ("taiga", [58, 108, 88]),
        ("birch", [94, 154, 78]),
        ("roofed", [38, 84, 36]),
        ("dark", [38, 84, 36]),
        ("forest", [62, 132, 56]),
        ("mountain", [112, 126, 106]),
        ("hills", [112, 126, 106]),
        ("extreme", [112, 126, 106]),
        ("plains", [134, 184, 86]),
        ("nether", [132, 42, 36]),
        ("hell", [132, 42, 36]),
        ("end", [222, 222, 162]),
        ("sky", [222, 222, 162]),
    ];
    FAMILY
        .iter()
        .find(|(k, _)| n.contains(k))
        .map(|(_, c)| *c)
        .unwrap_or_else(|| {
            let h = n.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
            [100 + (h % 80) as u8, 110 + (h / 80 % 80) as u8, 90 + (h / 6400 % 70) as u8]
        })
}

/// The flat color of a column with no biome recorded, by what is on top of it.
fn terrain_color(block: &str, water: u8) -> [u8; 3] {
    let n = block.split(['#', '@']).next().unwrap_or(block);
    if n == "water" {
        return if water > 18 { [17, 44, 112] } else if water > 6 { [26, 66, 150] } else { [52, 108, 210] };
    }
    const CLASSES: &[(&str, [u8; 3])] = &[
        ("lava", [214, 96, 24]),
        ("leaves", [62, 132, 56]),
        ("red_sand", [204, 102, 52]),
        ("terracotta", [204, 102, 52]),
        ("sand", [226, 212, 152]),
        ("snow", [236, 242, 246]),
        ("ice", [184, 216, 240]),
        ("grass", [134, 184, 86]),
        ("fern", [134, 184, 86]),
        ("moss", [92, 162, 62]),
        ("podzol", [58, 108, 88]),
        ("mycelium", [162, 112, 162]),
        ("mud", [72, 96, 62]),
        ("gravel", [128, 128, 122]),
        ("netherrack", [132, 42, 36]),
        ("crimson", [172, 32, 42]),
        ("warped", [32, 142, 132]),
        ("soul", [92, 76, 62]),
        ("basalt", [82, 76, 82]),
        ("blackstone", [60, 54, 60]),
        ("end_stone", [222, 222, 162]),
        ("stone", [140, 140, 136]),
        ("deepslate", [96, 96, 100]),
        ("dirt", [134, 104, 74]),
    ];
    CLASSES
        .iter()
        .find(|(k, _)| n.contains(k))
        .map(|(_, c)| *c)
        // Buildings and anything else: a quiet warm gray, so towns read as settled ground.
        .unwrap_or([168, 160, 150])
}

/// Colors already worked out, by atlas key (see [`key`]).
#[derive(Default)]
pub struct Palette {
    colors: std::collections::HashMap<u32, [u8; 3]>,
}

impl Palette {
    fn color(&mut self, k: u32, img: &MapImage, i: usize) -> [u8; 3] {
        *self.colors.entry(k).or_insert_with(|| {
            if k & 0x8000_0000 == 0 {
                world::biome_name(k as u16).map_or([120, 120, 120], |n| biome_color(&n))
            } else {
                let block = world::block_name(img.blocks[i]).unwrap_or_default();
                // The flat color of whatever covers the ground (leaves over grass read as forest).
                let over = world::block_name(img.over[i]).filter(|o| o.ends_with("_leaves") || o == "leaves");
                terrain_color(over.as_deref().unwrap_or(&block), img.water[i])
            }
        })
    }
}

/// What a column counts as on the atlas: its biome, or (top bit set) its terrain class when no
/// biome was recorded. Columns with the same key are drawn the same color.
fn key(img: &MapImage, i: usize) -> u32 {
    let b = img.biome[i];
    if b != 0 {
        return b as u32;
    }
    let over = img.over[i] as u32;
    // Water by three depths; the rest by the block on top (and leaves over it).
    let depth = if img.water[i] > 18 { 2 } else if img.water[i] > 6 { 1 } else { 0 };
    0x8000_0000 | (img.blocks[i] as u32) << 14 | (over & 0xFFF) << 2 | depth
}

/// One atlas tile of the picture shrunk by `f` (tile `tx`, `ty`): each cell takes the color of
/// the biome most of its columns are in, and cells on a border between biomes are darkened a
/// little (less the further out, as the borders would otherwise crowd the picture).
pub fn tile(ctx: &egui::Context, img: &MapImage, palette: &mut Palette, f: usize, tx: usize, ty: usize) -> egui::TextureHandle {
    let (nw, nh) = (img.width.div_ceil(f), img.height.div_ceil(f));
    let (cx0, cy0) = (tx * TILE, ty * TILE);
    let (cx1, cy1) = ((cx0 + TILE).min(nw), (cy0 + TILE).min(nh));
    // A ring of cells around the tile, so its border cells know their neighbors.
    let (bx0, by0) = (cx0.saturating_sub(1), cy0.saturating_sub(1));
    let (bx1, by1) = ((cx1 + 1).min(nw), (cy1 + 1).min(nh));
    let gw = bx1 - bx0;
    // Far out a cell covers thousands of columns: a grid of samples is enough to find its biome.
    let step = (f / 12).max(1);
    let mut cells: Vec<Option<(u32, usize)>> = vec![None; gw * (by1 - by0)];
    let mut tally: Vec<(u32, u32, usize)> = Vec::new();
    for cy in by0..by1 {
        for cx in bx0..bx1 {
            tally.clear();
            let (sx0, sy0) = (cx * f, cy * f);
            let (sx1, sy1) = ((sx0 + f).min(img.width), (sy0 + f).min(img.height));
            let mut sy = sy0 + (step / 2).min(sy1 - sy0 - 1);
            while sy < sy1 {
                let mut sx = sx0 + (step / 2).min(sx1 - sx0 - 1);
                while sx < sx1 {
                    let i = sy * img.width + sx;
                    if img.blocks[i] != 0 {
                        let k = key(img, i);
                        match tally.iter_mut().find(|t| t.0 == k) {
                            Some(t) => t.1 += 1,
                            None => tally.push((k, 1, i)),
                        }
                    }
                    sx += step;
                }
                sy += step;
            }
            cells[(cy - by0) * gw + (cx - bx0)] = tally.iter().max_by_key(|t| t.1).map(|t| (t.0, t.2));
        }
    }
    // Borders show plainly up close and fade out as each cell covers more ground.
    let edge = match f {
        0..=2 => 0.72,
        3..=4 => 0.78,
        5..=8 => 0.85,
        9..=16 => 0.92,
        _ => 1.0,
    };
    let (w, h) = (cx1 - cx0, cy1 - cy0);
    let mut out = vec![0u8; w * h * 4];
    let at = |cx: usize, cy: usize| cells[(cy - by0) * gw + (cx - bx0)].map(|c| c.0);
    for cy in cy0..cy1 {
        for cx in cx0..cx1 {
            let Some((k, sample)) = cells[(cy - by0) * gw + (cx - bx0)] else { continue };
            let mut rgb = palette.color(k, img, sample);
            let border = [(cx > bx0, cx.wrapping_sub(1), cy), (cx + 1 < bx1, cx + 1, cy), (cy > by0, cx, cy.wrapping_sub(1)), (cy + 1 < by1, cx, cy + 1)]
                .iter()
                .any(|(ok, x, y)| *ok && at(*x, *y).is_some_and(|o| o != k));
            if border && edge < 1.0 {
                rgb = rgb.map(|c| (c as f32 * edge) as u8);
            }
            let o = ((cy - cy0) * w + (cx - cx0)) * 4;
            out[o..o + 3].copy_from_slice(&rgb);
            out[o + 3] = 255;
        }
    }
    ctx.load_texture(
        format!("world-atlas{f}-{tx}-{ty}"),
        egui::ColorImage::from_rgba_unmultiplied([w, h], &out),
        egui::TextureOptions::NEAREST,
    )
}

/// The name of the biome under a block, for the tooltip.
pub fn biome_at(img: &MapImage, bx: i32, bz: i32) -> Option<String> {
    let (x, z) = (bx - img.min_x * 16, bz - img.min_z * 16);
    if x < 0 || z < 0 || x as usize >= img.width || z as usize >= img.height {
        return None;
    }
    world::biome_name(img.biome[z as usize * img.width + x as usize])
}
