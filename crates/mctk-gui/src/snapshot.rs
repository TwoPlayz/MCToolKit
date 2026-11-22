//! Pictures of the world map taken without a window: the map is run in egui as in the app and
//! the meshes egui produces are drawn in software to PNG files. For looking at the map's zoom
//! stages (Biome Atlas, terrain, top-down blocks, street view) on a machine with no GPU.
//!
//! MCTK_SNAPSHOT_WORLD=<java world> MCTK_SNAPSHOT_OUT=<dir> \
//!     cargo test --release -p mctk-gui snapshot -- --nocapture
//!
//! Skipped when the variables are not set. Textures come from the newest installed Java client
//! jar (extracted to a folder in the temp directory), as the app's own pack does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Event, Pos2, RawInput, Rect, pos2, vec2};

use crate::map_view::MapView;
use crate::world::Edition;

/// The 3D view's usual field of view.
const FOV: f32 = crate::street_view::FOV_Y;
const W: f32 = 1280.0;
const H: f32 = 720.0;

/// Textures egui has asked for, as premultiplied RGBA, and how many partial updates came (an
/// animated block picture changing frame is one).
#[derive(Default)]
pub(crate) struct Textures(HashMap<egui::TextureId, (usize, usize, Vec<Color32>)>, usize);

impl Textures {
    pub(crate) fn apply(&mut self, delta: &egui::TexturesDelta) {
        for (id, d) in &delta.set {
            if d.pos.is_some() {
                self.1 += 1;
            }
            let (w, h, px): (usize, usize, Vec<Color32>) = match &d.image {
                egui::ImageData::Color(c) => (c.size[0], c.size[1], c.pixels.clone()),
                egui::ImageData::Font(f) => (f.size[0], f.size[1], f.srgba_pixels(None).collect()),
            };
            match d.pos {
                None => {
                    self.0.insert(*id, (w, h, px));
                }
                Some([x0, y0]) => {
                    if let Some((tw, _, dst)) = self.0.get_mut(id) {
                        for y in 0..h {
                            for x in 0..w {
                                dst[(y0 + y) * *tw + x0 + x] = px[y * w + x];
                            }
                        }
                    }
                }
            }
        }
        for id in &delta.free {
            self.0.remove(id);
        }
    }
}

/// Draws egui's output into an RGBA picture (blending premultiplied colors as egui does).
pub(crate) fn rasterize(prims: &[egui::ClippedPrimitive], tex: &Textures, w: usize, h: usize) -> Vec<[f32; 4]> {
    let mut img = vec![[0.0f32; 4]; w * h];
    for p in prims {
        let egui::epaint::Primitive::Mesh(mesh) = &p.primitive else { continue };
        let Some((tw, th, tpx)) = tex.0.get(&mesh.texture_id) else { continue };
        let clip = p.clip_rect.intersect(Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32)));
        if clip.width() <= 0.0 || clip.height() <= 0.0 {
            continue;
        }
        for t in mesh.indices.chunks_exact(3) {
            let v = [&mesh.vertices[t[0] as usize], &mesh.vertices[t[1] as usize], &mesh.vertices[t[2] as usize]];
            let (a, b, c) = (v[0].pos, v[1].pos, v[2].pos);
            let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
            if area.abs() < 1e-6 {
                continue;
            }
            let x0 = a.x.min(b.x).min(c.x).max(clip.left()).floor() as i32;
            let x1 = a.x.max(b.x).max(c.x).min(clip.right()).ceil() as i32;
            let y0 = a.y.min(b.y).min(c.y).max(clip.top()).floor() as i32;
            let y1 = a.y.max(b.y).max(c.y).min(clip.bottom()).ceil() as i32;
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    if px < clip.left() || px >= clip.right() || py < clip.top() || py >= clip.bottom() {
                        continue;
                    }
                    let w0 = ((b.x - px) * (c.y - py) - (b.y - py) * (c.x - px)) / area;
                    let w1 = ((c.x - px) * (a.y - py) - (c.y - py) * (a.x - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                        continue;
                    }
                    let u = v[0].uv.x * w0 + v[1].uv.x * w1 + v[2].uv.x * w2;
                    let vv = v[0].uv.y * w0 + v[1].uv.y * w1 + v[2].uv.y * w2;
                    let tx = ((u * *tw as f32) as i64).clamp(0, *tw as i64 - 1) as usize;
                    let ty = ((vv * *th as f32) as i64).clamp(0, *th as i64 - 1) as usize;
                    let s = tpx[ty * tw + tx].to_array();
                    let mut src = [0.0f32; 4];
                    for k in 0..4 {
                        let vc = v[0].color.to_array()[k] as f32 * w0 + v[1].color.to_array()[k] as f32 * w1 + v[2].color.to_array()[k] as f32 * w2;
                        src[k] = s[k] as f32 / 255.0 * vc / 255.0;
                    }
                    let d = &mut img[y as usize * w + x as usize];
                    for k in 0..4 {
                        d[k] = src[k] + d[k] * (1.0 - src[3]);
                    }
                }
            }
        }
    }
    img
}

pub(crate) fn save(img: &[[f32; 4]], w: usize, h: usize, path: &Path) {
    let mut out = image::RgbImage::new(w as u32, h as u32);
    for (i, p) in img.iter().enumerate() {
        out.put_pixel((i % w) as u32, (i / w) as u32, image::Rgb([0, 1, 2].map(|k| (p[k].clamp(0.0, 1.0) * 255.0) as u8)));
    }
    out.save(path).expect("save snapshot");
}

/// The app's map in one egui context, run frame by frame.
struct Harness {
    ctx: egui::Context,
    view: MapView,
    tex: Textures,
    time: f64,
    last: Vec<egui::ClippedPrimitive>,
    /// The keys held (as the window reports them with each frame).
    mods: egui::Modifiers,
}

impl Harness {
    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(W, H))),
            time: Some(self.time),
            modifiers: self.mods,
            events,
            ..Default::default()
        };
        let view = &mut self.view;
        let t_run = Instant::now();
        let out = self.ctx.run(input, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(crate::theme::bg())).show(ctx, |ui| {
                let f = view.show_map(ui, true);
                view.show_chrome(ui, &f);
            });
        });
        let run_ms = t_run.elapsed().as_secs_f32() * 1000.0;
        self.tex.apply(&out.textures_delta);
        let t = Instant::now();
        self.last = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        if std::env::var("MCTK_SNAPSHOT_TIMING").is_ok() {
            let verts: usize = self.last.iter().map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                _ => 0,
            }).sum();
            eprintln!("run {run_ms:.1} ms, tessellate {:.1} ms, {verts} vertices", t.elapsed().as_secs_f32() * 1000.0);
        }
    }

    /// Runs frames until the map is drawn and its zoomed-out tiles are built.
    fn settle(&mut self, frames: usize, events: Vec<Event>) {
        self.frame(events);
        for _ in 0..frames {
            self.frame(vec![]);
        }
    }

    fn shot(&self, dir: &Path, name: &str) {
        eprintln!("SHOT {name}");
        let img = rasterize(&self.last, &self.tex, W as usize, H as usize);
        let path = dir.join(format!("{name}.png"));
        save(&img, W as usize, H as usize, &path);
        println!("saved {}", path.display());
    }
}

fn hover(at: Pos2) -> Event {
    Event::PointerMoved(at)
}

fn scroll(dy: f32) -> Event {
    Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, dy), modifiers: egui::Modifiers::NONE }
}

#[test]
fn snapshot_map_stages() {
    let (Ok(world), Ok(out)) = (std::env::var("MCTK_SNAPSHOT_WORLD"), std::env::var("MCTK_SNAPSHOT_OUT")) else {
        eprintln!("MCTK_SNAPSHOT_WORLD / MCTK_SNAPSHOT_OUT not set; skipped");
        return;
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("out dir");
    // The block pictures: the latest release's, fetched from mcasset.cloud and kept.
    let pack = std::env::temp_dir().join("mctk-snapshot-pack");
    crate::resource_pack::ensure(&pack);

    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    crate::icons::install_textures(&ctx);
    let mut h = Harness { ctx, view: MapView::new(PathBuf::from(&world), Edition::Java), tex: Textures::default(), time: 0.0, last: Vec::new(), mods: egui::Modifiers::NONE };
    // A deep-ocean coast seen from above at the widest 3D zoom: no flat map at the edges.
    let coast = "x=430;z=-60;zoom=4;tilt=0;style=auto";
    let mid = pos2(W / 2.0 + 10.0, H / 2.0 + 10.0);
    // Render the world (in the background, as the app does).
    let started = Instant::now();
    while !h.view.ready() {
        h.frame(vec![]);
        assert!(started.elapsed() < Duration::from_secs(300), "map did not finish rendering");
        std::thread::sleep(Duration::from_millis(30));
    }
    println!("map rendered in {:.1}s", started.elapsed().as_secs_f32());
    h.settle(40, vec![]);
    h.shot(&out, "00_fit");

    // Where the scenes are taken from: x, z, as set by MCTK_SNAPSHOT_AT ("x,z"), else the middle.
    let at = std::env::var("MCTK_SNAPSHOT_AT").ok();
    let (x, z) = at
        .as_deref()
        .and_then(|s| s.split_once(','))
        .and_then(|(a, b)| Some((a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?)))
        .unwrap_or((0.0, 0.0));
    let scenes: &[(&str, String)] = &[
        ("01_atlas_far", "x=0;z=0;zoom=0.12;style=auto".to_string()),
        ("02_atlas_blend", "x=0;z=0;zoom=0.32;style=auto".to_string()),
        ("05_terrain_mid", format!("x={x};z={z};zoom=2;style=auto")),
        ("06_above_z4", format!("x={x};z={z};zoom=4;style=auto")),
        ("06b_above_z8", format!("x={x};z={z};zoom=8;style=auto")),
        ("06c_above_max", format!("x={x};z={z};zoom=16;style=auto")),
        ("06d_above_coast", coast.to_string()),
        ("07_street_tilt30", format!("x={x};z={z};zoom=16;tilt=0.3;yaw=0;dolly=1")),
        ("08_street_full", format!("x={x};z={z};zoom=16;tilt=1;yaw=0;dolly=1")),
        ("09_street_close_turned", format!("x={x};z={z};zoom=16;tilt=1;yaw=0.8;dolly=0.45")),
        ("10_street_horizon", format!("x={x};z={z};zoom=16;tilt=1;yaw=2.2;dolly=0.6;pitch=-0.35")),
        // Low and close, turned toward the slope: the camera must stay out of the hill.
        ("14_low_into_slope", format!("x={x};z={z};zoom=16;tilt=1;yaw=4.0;dolly=0.25;pitch=-0.45")),
        // The camp the test world has at 8, 8: blocks that are not whole cubes.
        ("15_camp", "x=8.5;z=8.5;zoom=16;tilt=1;yaw=0.6;dolly=0.55;pitch=0".to_string()),
        ("16_camp_close", "x=8.5;z=8.5;zoom=16;tilt=1;yaw=2.6;dolly=0.25;pitch=-0.3".to_string()),
        ("17_camp_above", "x=8.5;z=8.5;zoom=12;style=auto".to_string()),
        // Water running from a source across the camp floor, lower as it goes.
        ("27_flowing_water", "x=5.5;z=12.5;zoom=16;tilt=1;yaw=3.3;dolly=0.4;pitch=0".to_string()),
        // The Overview inset: the camera, its facing and the ground in view, from above.
        ("30_overview", format!("x={x};z={z};zoom=16;tilt=1;yaw=0.8;dolly=0.6;pitch=-0.2;overview=1")),
        ("30b_overview_top_down", format!("x={x};z={z};zoom=8;tilt=0;overview=1")),
    ];
    for (name, preset) in scenes {
        h.view.dev_preset(preset);
        // Street scenes start from top-down, so a scene before does not leave its tilt behind.
        if !preset.contains("tilt") {
            h.view.dev_preset("tilt=0;yaw=0;pitch=0;dolly=1");
        }
        let t = Instant::now();
        h.settle(60, vec![hover(pos2(W - 200.0, H - 160.0))]);
        println!("{name}: {:.0} ms a frame", t.elapsed().as_secs_f32() * 1000.0 / 61.0);
        h.shot(&out, name);
    }
    h.view.dev_preset("overview=0");

    // Tilted, with the pointer over the void past the drawn chunks: the chunk there is still
    // highlighted (the pointer's ray meets the height looked at).
    h.view.dev_preset("x=8.5;z=8.5;zoom=16;tilt=1;yaw=0;dolly=1;pitch=-0.3;distance=3");
    h.settle(60, vec![hover(pos2(W * 0.5, H * 0.29))]);
    println!("pointer over the void: block picked {:?}", h.view.picked().map(|k| (k.x, k.y, k.z)));
    h.shot(&out, "31_void_hover");
    h.view.dev_preset("distance=12");

    // How long a street view frame takes while the camera moves (each frame builds the land
    // again): at the start of the tilt, where the most columns are in view, and fully tilted.
    for (label, tilt) in [("tilt 0.3", 0.3), ("tilt 1.0", 1.0)] {
        h.view.dev_preset(&format!("x={x};z={z};zoom=16;tilt={tilt};yaw=0;pitch=0;dolly=1"));
        h.settle(3, vec![]);
        let t = Instant::now();
        for k in 0..30 {
            h.view.dev_preset(&format!("yaw={}", k as f32 * 0.02));
            h.frame(vec![]);
        }
        println!("street view while moving ({label}): {:.1} ms a frame", t.elapsed().as_secs_f32() * 1000.0 / 30.0);
    }
    // The above view while panning, at the zoom where it starts (the most columns on screen).
    h.view.dev_preset(&format!("x={x};z={z};zoom=4;tilt=0;yaw=0;pitch=0;dolly=1"));
    h.settle(3, vec![]);
    let t = Instant::now();
    for k in 0..30 {
        h.view.dev_preset(&format!("x={}", x + k as f32 * 0.7));
        h.frame(vec![]);
    }
    println!("above view while moving (zoom 4): {:.1} ms a frame", t.elapsed().as_secs_f32() * 1000.0 / 30.0);
    let t = Instant::now();
    for _ in 0..30 {
        h.frame(vec![]);
    }
    println!("above view still (zoom 4): {:.1} ms a frame", t.elapsed().as_secs_f32() * 1000.0 / 30.0);

    // Moving about in the street view over hills: land in the way lifts the camera over it, but
    // never brings it in or tips its angle.
    h.view.dev_preset(&format!("x={x};z={z};zoom=16;tilt=1;yaw=0.7;pitch=0;dolly=0.6"));
    h.settle(10, vec![hover(mid)]);
    let mut prev = h.view.camera_dist().unwrap_or(0.0);
    let mut worst: f32 = 1.0;
    let pitch0 = h.view.camera_pitch().unwrap_or(0.0);
    let mut tipped: f32 = 0.0;
    h.frame(vec![Event::PointerButton { pos: mid, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }]);
    let mut at = mid;
    for k in 0..120 {
        // Wander: forward, then sideways, then back.
        let step = match k / 40 { 0 => vec2(0.0, 6.0), 1 => vec2(6.0, 0.0), _ => vec2(0.0, -6.0) };
        at += step;
        h.frame(vec![hover(at)]);
        let d = h.view.camera_dist().unwrap_or(prev);
        if prev > 0.0 {
            worst = worst.min(d / prev);
        }
        prev = d;
        tipped = tipped.max((h.view.camera_pitch().unwrap_or(pitch0) - pitch0).abs());
    }
    println!("street view camera: tipped at most {:.1} degrees while moving", tipped.to_degrees());
    assert!(tipped.to_degrees() < 1.0, "moving over high ground should not tip the camera's angle");
    h.frame(vec![Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }]);
    println!("street view camera: smallest frame-to-frame change {worst:.3}");
    assert!(worst > 0.999, "the camera came in while moving (to {:.0}% of its distance in one frame)", worst * 100.0);
    h.shot(&out, "18_after_wander");

    // Animated pictures change frame by themselves: water at the camp.
    let before = h.tex.1;
    h.view.dev_preset("x=10.5;z=9.5;zoom=16;tilt=1;yaw=0.3;pitch=0;dolly=0.4");
    for _ in 0..40 {
        h.frame(vec![]);
    }
    let changes = h.tex.1 - before;
    println!("animated frame updates in 40 frames: {changes}");
    assert!(changes > 0, "animated block pictures should change frame over time");
    h.shot(&out, "19_camp_water");

    let click = |at: Pos2| {
        vec![
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ]
    };
    // A flower can be pointed at: the poppy at the camp (7, 10), outlined narrower than a block.
    h.view.dev_preset("x=7.5;z=10.5;zoom=16;tilt=0;yaw=0;pitch=0;dolly=1");
    h.settle(20, vec![]);
    // Aim at the middle of the poppy's column at each height in turn (the floor's height is not
    // known here) until it is the poppy that is picked.
    let mut flower = None;
    for y in (60..90).rev() {
        let Some(at) = h.view.project([7.5, y as f32 + 0.4, 10.5]) else { continue };
        h.frame(vec![hover(at)]);
        if std::env::var("MCTK_SNAPSHOT_TIMING").is_ok() {
            eprintln!("aim y {y} at {at:?}: picked {:?}", h.view.picked());
        }
        if let Some(hit) = h.view.picked().filter(|k| k.x == 7 && k.z == 10 && k.narrow()) {
            flower = Some(hit);
            break;
        }
    }
    println!("flower picked: {flower:?}");
    assert!(flower.is_some(), "pointing at a flower should pick it");
    h.shot(&out, "25_flower_pick");

    // The chunk grid on the land in the street view, at the camp.
    h.view.chunk_grid = true;
    h.view.dev_preset("x=8.5;z=8.5;zoom=16;tilt=1;yaw=0.6;pitch=0;dolly=0.8");
    h.settle(20, vec![hover(pos2(W - 200.0, H - 160.0))]);
    h.shot(&out, "20_grid_street");
    h.view.chunk_grid = false;

    // Box Select: click one corner and then the other in the street view.
    h.view.box_mode = true;
    h.settle(5, vec![hover(pos2(500.0, 330.0))]);
    h.frame(click(pos2(500.0, 330.0)));
    h.settle(3, vec![hover(pos2(820.0, 470.0))]);
    h.frame(click(pos2(820.0, 470.0)));
    h.settle(5, vec![hover(pos2(W - 200.0, H - 160.0))]);
    let sel = h.view.sel_box;
    println!("box selected: {sel:?}");
    let (lo, hi) = sel.expect("two clicks in Box Select should make a box");
    assert!(hi[0] > lo[0] || hi[2] > lo[2], "the box should span the two corners clicked");
    h.shot(&out, "21_box_street");
    // The same box seen from above.
    h.view.dev_preset("x=8.5;z=8.5;zoom=10;tilt=0;yaw=0;pitch=0;dolly=1");
    h.settle(20, vec![hover(pos2(W - 200.0, H - 160.0))]);
    h.shot(&out, "22_box_above");
    h.view.box_mode = false;

    // Scrolling in at full tilt brings the camera nearer, and once it is as near as it may come,
    // zooms the lens.
    h.view.dev_preset("x=8.5;z=8.5;zoom=16;tilt=1;yaw=0.6;pitch=0;dolly=0.8");
    h.settle(5, vec![hover(mid)]);
    let dist_before = h.view.camera_dist().unwrap_or(0.0);
    for _ in 0..40 {
        h.frame(vec![hover(mid), scroll(80.0)]);
    }
    h.settle(30, vec![hover(mid)]);
    let fov = h.view.camera_fov().unwrap_or(1.0);
    println!("lens: field of view {:.1} degrees (camera distance {:.1} -> {:.1})", fov.to_degrees(), dist_before, h.view.camera_dist().unwrap_or(0.0));
    assert!(fov.to_degrees() < 40.0, "scrolling in past the nearest the camera goes should zoom the lens");
    h.shot(&out, "23_lens");
    // And scrolling out undoes it all, back to the Top-down view.
    for _ in 0..120 {
        h.frame(vec![hover(mid), scroll(-80.0)]);
    }
    h.settle(40, vec![hover(mid)]);
    assert!((h.view.camera_fov().unwrap_or(FOV) - FOV).abs() < 1e-3, "scrolling out should undo the lens");

    // Right-drag in the Top-down view turns it into 3D from where it is; the Top-down button
    // eases back to the same zoom.
    h.view.dev_preset("x=8.5;z=8.5;zoom=8;tilt=0;yaw=0;pitch=0;dolly=1");
    h.settle(10, vec![hover(mid)]);
    let right = |pressed: bool, at: Pos2| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
    h.frame(vec![right(true, mid)]);
    let mut at = mid;
    for _ in 0..20 {
        at += vec2(8.0, -6.0);
        h.frame(vec![hover(at)]);
    }
    h.frame(vec![right(false, at)]);
    h.settle(10, vec![hover(mid)]);
    let (tilt, _) = h.view.street_state();
    let pitch = h.view.camera_pitch().unwrap_or(std::f32::consts::FRAC_PI_2);
    println!("right-drag in Top-down: tilt {tilt:.3}, pitch {:.0} degrees", pitch.to_degrees());
    assert!(tilt > 0.0 && pitch.to_degrees() < 80.0, "right-drag in the Top-down view should turn it into 3D");
    h.shot(&out, "26_orbit_from_top_down");
    h.view.go_top_down();
    for _ in 0..120 {
        h.frame(vec![hover(mid)]);
    }
    let (tilt, _) = h.view.street_state();
    println!("after Top-down: tilt {tilt:.3}, zoom {:.2}", h.view.zoom);
    assert_eq!(tilt, 0.0, "the Top-down button should come back to the Top-down view");
    assert!((h.view.zoom - 8.0).abs() < 0.5, "and to the zoom it was turned from");

    // From deep in 3D (lens zoomed, camera near) one press of Top-down gets all the way back,
    // and quickly.
    h.view.dev_preset("x=8.5;z=8.5;zoom=16;tilt=1;yaw=1;pitch=0;dolly=0.2;lens=4");
    h.settle(5, vec![hover(mid)]);
    h.view.go_top_down();
    let mut frames = 0;
    while h.view.street_state().0 > 0.0 && frames < 200 {
        h.frame(vec![hover(mid)]);
        frames += 1;
    }
    println!("Top-down from deep 3D: back in {frames} frames (tilt {:.3})", h.view.street_state().0);
    assert_eq!(h.view.street_state().0, 0.0, "one press of Top-down should get back from lens zoom and nearness");
    assert!(frames < 90, "and within about a second and a half");

    // The free camera, as a 3D editor's: turned on from Top-down it eases into 3D as near as the
    // usual camera comes (up off the ground, the lens left alone); turned off it eases back to
    // Top-down.
    let map_rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(W, H));
    h.view.set_free_camera(true, map_rect);
    h.settle(90, vec![hover(mid)]);
    let (tilt, _) = h.view.street_state();
    let near = h.view.camera_dist().unwrap_or(99.0);
    let fov = h.view.camera_fov().unwrap_or(0.0);
    let up = h.view.camera_eye().zip(h.view.camera_target()).map_or(0.0, |(e, t)| e[1] - t[1]);
    println!("free camera on: tilt {tilt:.2}, {near:.1} blocks off, eye {up:.1} up, fov {fov:.3}");
    assert!(tilt >= 0.999, "turning the free camera on should go into the 3D view");
    assert!((6.0..10.0).contains(&near), "as near as the usual camera comes");
    assert!(up > 3.0, "with the eye up off the ground");
    assert!((fov - FOV).abs() < 1e-3, "without zooming the lens");
    // The wheel zooms its lens (the camera stays where it is).
    let (eye0, state0) = (h.view.camera_eye().unwrap_or([0.0; 3]), h.view.street_state());
    for _ in 0..5 {
        h.frame(vec![hover(mid), scroll(80.0)]);
    }
    h.settle(40, vec![hover(mid)]);
    let eye1 = h.view.camera_eye().unwrap_or([0.0; 3]);
    let moved = ((0..3).map(|k| (eye1[k] - eye0[k]).powi(2)).sum::<f32>()).sqrt();
    let fov1 = h.view.camera_fov().unwrap_or(FOV);
    println!("free camera wheel: moved {moved:.2} blocks, fov {:.3} -> {fov1:.3}, state {:?} -> {:?}", FOV, state0, h.view.street_state());
    assert!(fov1 < FOV * 0.8, "the wheel should zoom the free camera's lens");
    assert!(moved < 0.1, "without moving it");
    assert_eq!(state0, h.view.street_state(), "or tilting it");
    h.view.set_free_camera(false, map_rect);
    let mut frames = 0;
    while h.view.street_state().0 > 0.0 && frames < 200 {
        h.frame(vec![hover(mid)]);
        frames += 1;
    }
    println!("free camera off: back to Top-down in {frames} frames");
    assert_eq!(h.view.street_state().0, 0.0, "turning the free camera off should go back to Top-down");
    assert!(!h.view.noclip);

    // The free camera right up to the blocks, its lens zoomed by the wheel.
    h.view.dev_preset("x=8.5;z=8.5;zoom=16;tilt=1;yaw=2.6;pitch=-0.2;dolly=0.1;noclip=1");
    h.settle(5, vec![hover(mid)]);
    for _ in 0..10 {
        h.frame(vec![hover(mid), scroll(80.0)]);
    }
    h.settle(30, vec![hover(mid)]);
    h.shot(&out, "24_noclip_close");
    // It flies with W (forward) and Space (up).
    let key = |key: egui::Key, pressed: bool| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
    let (c0, y0) = (h.view.center, h.view.camera_target().map_or(0.0, |t| t[1]));
    h.frame(vec![key(egui::Key::W, true), key(egui::Key::Space, true)]);
    for _ in 0..30 {
        h.frame(vec![]);
    }
    h.frame(vec![key(egui::Key::W, false), key(egui::Key::Space, false)]);
    let moved = (h.view.center - c0).length();
    let rose = h.view.camera_target().map_or(0.0, |t| t[1]) - y0;
    println!("free camera: W moved {moved:.1} blocks, Space rose {rose:.1}");
    assert!(moved > 3.0 && rose > 3.0, "WASD and Space should fly the free camera");
    // It can look up (dragging the right button up), and sees far.
    let right_btn = |pressed: bool, at: Pos2| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
    h.view.dev_preset("x=8.5;z=8.5;zoom=16;tilt=1;yaw=2.6;pitch=0;dolly=0.3");
    h.settle(5, vec![hover(mid)]);
    let eye_before = h.view.camera_eye().unwrap_or([0.0; 3]);
    h.frame(vec![right_btn(true, mid)]);
    let mut at = mid;
    for _ in 0..30 {
        at += vec2(0.0, -20.0);
        h.frame(vec![hover(at)]);
    }
    h.frame(vec![right_btn(false, at)]);
    h.settle(5, vec![hover(mid)]);
    let pitch = h.view.camera_pitch().unwrap_or(1.0);
    println!("free camera looking up: pitch {:.0} degrees", pitch.to_degrees());
    assert!(pitch < 0.0, "the free camera should be able to look up");
    // It turned where it was: the eye stayed put.
    let eye_after = h.view.camera_eye().unwrap_or([0.0; 3]);
    let moved = ((0..3).map(|k| (eye_after[k] - eye_before[k]).powi(2)).sum::<f32>()).sqrt();
    println!("free camera eye moved {moved:.2} blocks while turning");
    assert!(moved < 0.1, "the free camera should turn in place (its eye moved {moved:.2} blocks)");
    h.shot(&out, "29_freecam_look_up");
    // Drawing the map again (as a Slicer change does) keeps the 3D camera where it was.
    let before = (h.view.street_state(), h.view.camera_target());
    h.view.redraw();
    let started = Instant::now();
    while !h.view.ready() && started.elapsed() < Duration::from_secs(60) {
        h.frame(vec![]);
        std::thread::sleep(Duration::from_millis(20));
    }
    h.settle(5, vec![]);
    let after = (h.view.street_state(), h.view.camera_target());
    println!("redraw: before {before:?} after {after:?}");
    assert_eq!(before.0, after.0, "the 3D view should stay as it was through a redraw");
    assert!(before.1.zip(after.1).is_some_and(|(a, b)| (a[1] - b[1]).abs() < 0.01), "the free camera keeps its height");
    h.view.noclip = false;

    // In 3D, looking down (right-drag down) only turns the camera: it does not back it out
    // (the tilt and nearness stay as they are), so scrolling in afterwards has nothing to undo.
    h.view.dev_preset("x=8.5;z=8.5;zoom=16;tilt=1;yaw=1;pitch=0;dolly=0.5");
    h.settle(5, vec![hover(mid)]);
    let (state0, pitch0) = (h.view.street_state(), h.view.camera_pitch().unwrap_or(0.0));
    h.frame(vec![right_btn(true, mid)]);
    let mut at = mid;
    for _ in 0..15 {
        at += vec2(0.0, 12.0);
        h.frame(vec![hover(at)]);
    }
    h.frame(vec![right_btn(false, at)]);
    let (state1, pitch1) = (h.view.street_state(), h.view.camera_pitch().unwrap_or(0.0));
    println!("look down in 3D: {state0:?} -> {state1:?}, pitch {:.0} -> {:.0} degrees", pitch0.to_degrees(), pitch1.to_degrees());
    assert_eq!(state0, state1, "looking down should not change the tilt or nearness");
    assert!(pitch1 > pitch0 + 0.2, "but should look further down");

    // The same partway into 3D (where looking down used to undo the tilt and back the camera
    // out): the distance stays, and scrolling in afterwards keeps the angle looked at.
    h.view.dev_preset("x=8.5;z=8.5;zoom=16;tilt=0.4;yaw=1;dolly=1");
    h.settle(5, vec![hover(mid)]);
    let (state0, pitch0) = (h.view.street_state(), h.view.camera_pitch().unwrap_or(0.0));
    h.frame(vec![right_btn(true, mid)]);
    let mut at = mid;
    for _ in 0..10 {
        at += vec2(0.0, 12.0);
        h.frame(vec![hover(at)]);
    }
    h.frame(vec![right_btn(false, at)]);
    let (state1, pitch1) = (h.view.street_state(), h.view.camera_pitch().unwrap_or(0.0));
    println!("look down part-tilted: {state0:?} -> {state1:?}, pitch {:.0} -> {:.0} degrees", pitch0.to_degrees(), pitch1.to_degrees());
    assert_eq!(state0, state1, "looking down part-tilted should not change the tilt or nearness");
    assert!(pitch1 > pitch0 + 0.1, "but should look further down");
    for _ in 0..4 {
        h.frame(vec![hover(mid), scroll(60.0)]);
    }
    h.settle(30, vec![hover(mid)]);
    let (state2, pitch2) = (h.view.street_state(), h.view.camera_pitch().unwrap_or(0.0));
    println!("then scrolled in: {state2:?}, pitch {:.0} degrees", pitch2.to_degrees());
    assert!(state2.0 > state1.0 || state2.1 < state1.1, "scrolling in should come nearer");
    assert!((pitch2 - pitch1).abs() < 0.01, "without undoing the angle looked at");

    // Right-drag turns the flat map (further out than the Top-down view).
    h.view.dev_preset("x=0;z=0;zoom=1;tilt=0;yaw=0;pitch=0;dolly=1");
    h.settle(10, vec![hover(mid)]);
    h.frame(vec![right_btn(true, mid)]);
    let mut at = mid;
    for _ in 0..20 {
        at += vec2(12.0, 0.0);
        h.frame(vec![hover(at)]);
    }
    h.frame(vec![right_btn(false, at)]);
    h.settle(20, vec![hover(pos2(W - 200.0, H - 160.0))]);
    println!("flat map turned to {:.0} degrees", h.view.yaw().to_degrees());
    assert!(h.view.yaw().abs() > 0.3, "right-drag should turn the flat map");
    h.shot(&out, "28_turned_flat_map");
    h.view.dev_preset("yaw=0");

    // A quick drag let go of keeps the map gliding for a moment.
    h.view.dev_preset(&format!("x={x};z={z};zoom=2;tilt=0;yaw=0;pitch=0;dolly=1"));
    h.settle(3, vec![hover(mid)]);
    let press = |pressed: bool, at: Pos2| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    h.frame(vec![press(true, mid)]);
    let mut at = mid;
    for _ in 0..10 {
        at += vec2(-25.0, 0.0);
        h.frame(vec![hover(at)]);
    }
    h.frame(vec![press(false, at)]);
    let released = h.view.center;
    for _ in 0..20 {
        h.frame(vec![]);
    }
    let glide = h.view.center.x - released.x;
    println!("glide after letting go: {glide:.1} blocks");
    assert!(glide > 5.0, "a quick drag let go of should glide on (moved {glide:.1} blocks)");

    // Scrolling in at the closest top-down view tilts into the street view; scrolling out comes
    // back to the flat map.
    h.view.dev_preset(&format!("x={x};z={z};zoom=16;tilt=0;yaw=0;pitch=0;dolly=1;style=auto"));
    h.settle(3, vec![hover(mid)]);
    // About a dozen notches of a mouse wheel.
    for _ in 0..12 {
        h.frame(vec![hover(mid), scroll(60.0)]);
    }
    h.settle(30, vec![hover(mid)]);
    let (tilt, _) = h.view.street_state();
    println!("tilt after scrolling in: {tilt:.2}");
    assert!(tilt > 0.5, "scrolling in at the closest top-down view should tilt into the street view");
    h.shot(&out, "11_scrolled_in");
    // Turning with the right button, then a hover for the tooltip.
    h.frame(vec![Event::PointerButton { pos: mid, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE }]);
    for k in 1..=20 {
        h.frame(vec![hover(mid + vec2(k as f32 * 12.0, 0.0))]);
    }
    h.frame(vec![Event::PointerButton { pos: mid + vec2(240.0, 0.0), button: egui::PointerButton::Secondary, pressed: false, modifiers: egui::Modifiers::NONE }]);
    let probe = std::env::var("MCTK_SNAPSHOT_PROBE")
        .ok()
        .and_then(|s| s.split_once(',').and_then(|(a, b)| Some(pos2(a.trim().parse().ok()?, b.trim().parse().ok()?))))
        .unwrap_or(pos2(W / 2.0, H * 0.7));
    h.settle(5, vec![hover(probe)]);
    h.shot(&out, "12_turned_hover");
    for _ in 0..80 {
        h.frame(vec![hover(mid), scroll(-60.0)]);
    }
    h.settle(40, vec![hover(mid)]);
    let (tilt, _) = h.view.street_state();
    println!("tilt after scrolling out: {tilt:.2}");
    assert_eq!(tilt, 0.0, "scrolling out should come back to the flat map");
    h.shot(&out, "13_scrolled_out");
}

/// A sheet of item icons as the NBT editor draws them (blocks from their models, chests, heads
/// and the like from the boxes the game draws, flat items from their pictures), for looking at.
#[test]
fn snapshot_item_icons() {
    let Ok(out) = std::env::var("MCTK_SNAPSHOT_OUT") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("out dir");
    let ids = [
        "grass_block", "furnace", "oak_stairs", "crafting_table", "oak_leaves", "glass", "chest", "ender_chest",
        "copper_chest", "player_head", "creeper_head", "skeleton_skull", "decorated_pot", "copper_golem_statue", "shield", "pig_spawn_egg",
        "villager_spawn_egg", "diamond_sword", "potion", "oak_fence", "hopper", "anvil", "bell", "lantern",
        "trapped_chest", "pink_candle_cake", "piston_head", "pitcher_crop", "potatoes", "powder_snow", "pumpkin_stem", "fire",
    ];
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let mut tex = Textures::default();
    let (cell, cols) = (64.0, 8);
    let (w, h) = (cols as f32 * cell, (ids.len() as f32 / cols as f32).ceil() * cell);
    let mut last = Vec::new();
    for _ in 0..3 {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w, h))), ..Default::default() };
        let o = ctx.run(input, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::from_gray(139))).show(ctx, |ui| {
                for (i, id) in ids.iter().enumerate() {
                    let r = Rect::from_min_size(pos2((i % cols) as f32 * cell, (i / cols) as f32 * cell), vec2(cell, cell)).shrink(8.0);
                    assert!(crate::icons::item_icon(ui.painter(), r, id, Color32::WHITE), "{id} drawn");
                }
            });
        });
        tex.apply(&o.textures_delta);
        last = ctx.tessellate(o.shapes, o.pixels_per_point);
    }
    let img = rasterize(&last, &tex, w as usize, h as usize);
    save(&img, w as usize, h as usize, &out.join("icons.png"));
}

/// Every mob the world view draws, each from its model, in a labelled sheet (for looking at
/// models against their pictures; with MCTK_SNAPSHOT_OUT set). `MCTK_MOBS` picks which (by id,
/// comma-separated).
#[test]
fn snapshot_mob_sheet() {
    let Ok(out) = std::env::var("MCTK_SNAPSHOT_OUT") else { return };
    let ids: Vec<String> = std::env::var("MCTK_MOBS").map(|s| s.split(',').map(str::to_string).collect()).unwrap_or_else(|_| {
        "villager,allay,camel,rabbit,bat,turtle,dolphin,cod,salmon,pufferfish,tropical_fish,axolotl,frog,tadpole,ghast,blaze,guardian,phantom,sniffer,armadillo,warden,breeze,hoglin,zoglin,strider,wither,vex,endermite,silverfish,shulker,ender_dragon"
            .split(',').map(str::to_string).collect()
    });
    let cell = 160usize;
    let cols = 6usize;
    let rows = ids.len().div_ceil(cols);
    let mut sheet = image::RgbaImage::from_pixel((cols * cell) as u32, (rows * (cell + 16)) as u32, image::Rgba([90, 96, 104, 255]));
    for (i, id) in ids.iter().enumerate() {
        // `baby_pig`: a baby.
        // `armed_zombie`: wearing iron armor, a sword in hand and a shield in the other.
        let (id, armed) = match id.strip_prefix("armed_") {
            Some(rest) => (&rest.to_string(), true),
            None => (id, false),
        };
        let (id, baby) = match id.strip_prefix("baby_") {
            Some(rest) => (&rest.to_string(), true),
            None => (id, false),
        };
        let mut c = mctk_nbt::Compound::new();
        c.insert("id".into(), mctk_nbt::Tag::String(format!("minecraft:{id}")));
        if armed {
            let mut eq = mctk_nbt::Compound::new();
            for (slot, item) in [("head", "iron_helmet"), ("chest", "iron_chestplate"), ("legs", "iron_leggings"), ("feet", "iron_boots"), ("mainhand", "iron_sword"), ("offhand", "torch")] {
                let mut s = mctk_nbt::Compound::new();
                s.insert("id".into(), mctk_nbt::Tag::String(format!("minecraft:{item}")));
                s.insert("count".into(), mctk_nbt::Tag::Int(1));
                eq.insert(slot.into(), mctk_nbt::Tag::Compound(s));
            }
            c.insert("equipment".into(), mctk_nbt::Tag::Compound(eq));
        }
        if baby {
            c.insert("Age".into(), mctk_nbt::Tag::Int(-24000));
            c.insert("IsBaby".into(), mctk_nbt::Tag::Byte(1));
        }
        let tag = mctk_nbt::Tag::Compound(c);
        let mut b = crate::entities::Builder::new();
        // Turned to face the camera (three-quarters on).
        let yaw: f32 = std::env::var("MCTK_MOB_YAW").ok().and_then(|v| v.parse().ok()).unwrap_or(200.0);
        let built = crate::mobs::mob(&mut b, &tag, id, &crate::entities::Mat::ID, yaw);
        if !built {
            crate::mobs::marker(&mut b, id, &crate::entities::Mat::ID, 0.0);
        }
        let (x0, y0) = ((i % cols) * cell, (i / cols) * (cell + 16));
        if let Some(img) = crate::icons::render_quads(&b.out, cell) {
            for (k, px) in img.pixels.iter().enumerate() {
                let (x, y) = (k % cell, k / cell);
                let [r, g, bl, a] = px.to_array();
                if a > 0 {
                    let d = sheet.get_pixel_mut((x0 + x) as u32, (y0 + y) as u32);
                    let inv = 255 - a as u32;
                    *d = image::Rgba([(r as u32 + d[0] as u32 * inv / 255) as u8, (g as u32 + d[1] as u32 * inv / 255) as u8, (bl as u32 + d[2] as u32 * inv / 255) as u8, 255]);
                }
            }
        }
        // A strip under it: green when modelled, red when only its egg.
        let color = if built { image::Rgba([60, 170, 80, 255]) } else { image::Rgba([190, 60, 60, 255]) };
        for y in y0 + cell..y0 + cell + 16 {
            for x in x0..x0 + cell {
                sheet.put_pixel(x as u32, y as u32, color);
            }
        }
        println!("MOB {i}: {id} {}", if built { "model" } else { "EGG" });
    }
    sheet.save(std::path::Path::new(&out).join("mobs.png")).expect("save");
}

/// For working out models from pictures: each separate patch of a texture read as a box laid out
/// as the game lays a box out (its top and bottom faces in a row, its four sides in the row under
/// them), giving where it starts (u, v) and its size (x, y, z); a patch with no such shape is
/// given as a flat rectangle. `MCTK_TEXTURES` names the pictures (under `textures/entity/`).
#[test]
fn texture_boxes() {
    let Ok(names) = std::env::var("MCTK_TEXTURES") else { return };
    for name in names.split(',') {
        let Some(path) = crate::assets::fetch(&format!("assets/minecraft/textures/entity/{name}.png")) else {
            println!("BOX {name}: not found");
            continue;
        };
        let img = image::open(&path).expect("a picture").to_rgba8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let solid = |x: usize, y: usize| img.get_pixel(x as u32, y as u32)[3] > 0;
        let mut seen = vec![false; w * h];
        let mut out = Vec::new();
        for y0 in 0..h {
            for x0 in 0..w {
                if seen[y0 * w + x0] || !solid(x0, y0) {
                    continue;
                }
                // The patch, and the bounds of each of its rows.
                let mut stack = vec![(x0, y0)];
                seen[y0 * w + x0] = true;
                let mut rows: std::collections::BTreeMap<usize, (usize, usize)> = Default::default();
                let mut n = 0;
                while let Some((x, y)) = stack.pop() {
                    n += 1;
                    let r = rows.entry(y).or_insert((x, x));
                    r.0 = r.0.min(x);
                    r.1 = r.1.max(x);
                    for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                            let (nx, ny) = (nx as usize, ny as usize);
                            if !seen[ny * w + nx] && solid(nx, ny) {
                                seen[ny * w + nx] = true;
                                stack.push((nx, ny));
                            }
                        }
                    }
                }
                if n < 3 {
                    continue;
                }
                let (v, last) = (*rows.keys().next().unwrap(), *rows.keys().last().unwrap());
                let u = rows.values().map(|r| r.0).min().unwrap();
                let right = rows.values().map(|r| r.1).max().unwrap() + 1;
                // Where the rows widen to the full width: the sides start there.
                let side = rows.iter().find(|(_, r)| r.0 == u).map(|(y, _)| *y).unwrap_or(v);
                let top_left = rows[&v].0;
                let (bw, bh) = (right - u, last + 1 - v);
                let dz = side - v;
                let text = if dz > 0 && top_left == u + dz && bw >= 2 * dz {
                    let dx = bw / 2 - dz;
                    format!("box uv ({u},{v}) size x {dx} y {} z {dz}", bh - dz)
                } else {
                    format!("flat ({u},{v}) {bw}x{bh}")
                };
                out.push(text);
            }
        }
        println!("BOX {name} ({w}x{h}): {}", out.join(" | "));
    }
}

/// The pointer's card in the 3D view (what a mob or container holds), for looking at.
#[test]
fn snapshot_contents_card() {
    let Ok(out) = std::env::var("MCTK_SNAPSHOT_OUT") else { return };
    let out = PathBuf::from(out);
    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let mut tex = Textures::default();
    let (w, h) = (300.0, 400.0);
    let mut last = Vec::new();
    for _ in 0..3 {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w, h))), ..Default::default() };
        let o = ctx.run(input, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::from_rgb(90, 120, 150))).show(ctx, |ui| {
                crate::map_view::sample_cards(ui.painter(), ui.max_rect());
            });
        });
        tex.apply(&o.textures_delta);
        last = ctx.tessellate(o.shapes, o.pixels_per_point);
    }
    let img = rasterize(&last, &tex, w as usize, h as usize);
    save(&img, w as usize, h as usize, &out.join("card.png"));
}