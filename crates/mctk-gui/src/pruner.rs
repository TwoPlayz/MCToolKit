//! The World Explorer's Prune tools (once the Pruner page): draw layers on the flat map, see
//! the chunks that would be removed (red hatching), then Prune deletes them as an edit (one
//! step to undo, written on Save).

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use mctk_anvil::ChunkPos;
use mctk_prune::{Comparison, Layer, Selection};

use crate::editor::LEFT_PANEL;
use crate::icons::{self, Icon};
use crate::jobs::Job;
use crate::map_view::MapFrame;
use crate::theme;
use crate::widgets;
use crate::workspace::Workspace;
use crate::world_picker::UiEdition;

/// A layer in chunk units.
#[derive(Debug, Clone, PartialEq)]
enum PLayer {
    Box { x: f64, z: f64, w: f64, h: f64 },
    /// `a` is the angle (radians) of the radius handle, so it can be turned around the circle.
    Circle { x: f64, z: f64, r: f64, a: f64 },
    /// `angle` is the direction of the dividing line (as in the pruner crate); the arrow from the
    /// start points to the side that is selected, a quarter turn on. `len` is how far (in chunks)
    /// the arrow's tip is, so it can be any length and its tip sits on the middle of a chunk.
    Split { x: f64, z: f64, angle: f64, len: f64 },
    Polygon { points: Vec<(f64, f64)> },
    Time { cmp: usize, ticks: f64 },
}

/// Diagonal hatch lines over just the part of `r` where `inside` holds.
fn hatch_where(p: &egui::Painter, r: Rect, s: f32, stroke: Stroke, inside: &dyn Fn(Pos2) -> bool) {
    let c0 = ((r.left() - r.bottom()) / s).floor() as i32;
    let c1 = ((r.right() - r.top()) / s).ceil() as i32;
    for k in c0..=c1 {
        let c = k as f32 * s;
        let (xa, xb) = (r.left().max(r.top() + c), r.right().min(r.bottom() + c));
        if xa >= xb {
            continue;
        }
        let mut start: Option<Pos2> = None;
        let mut last = pos2(xa, xa - c);
        let mut x = xa;
        while x <= xb {
            let q = pos2(x, x - c);
            match (inside(q), start) {
                (true, None) => start = Some(q),
                (false, Some(a)) => {
                    p.line_segment([a, last], stroke);
                    start = None;
                }
                _ => {}
            }
            last = q;
            x += 1.0;
        }
        if let Some(a) = start {
            p.line_segment([a, last], stroke);
        }
    }
}

/// A card's picture of its layer as it really is: the box with its own proportions, the polygon
/// outline, the split's line with its arrow, with the part that gets removed shaded and hatched
/// as on the map. Inverted removes everything around the shape instead.
fn draw_thumb(p: &egui::Painter, rect: Rect, layer: &PLayer, inverted: bool) {
    p.rect_filled(rect, theme::radius(3), theme::deep());
    if let PLayer::Time { .. } = layer {
        if !icons::image(p, rect, "THUMB_TIME", Color32::WHITE) {
            icons::paint(p, rect.shrink(9.0), layer.icon(), theme::RED);
        }
        return;
    }
    let p = p.with_clip_rect(rect);
    let tint = Color32::from_rgba_unmultiplied(170, 40, 40, 80);
    let hatch_line = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(235, 60, 60, 190));
    let edge = Stroke::new(1.5_f32, theme::RED);
    let inner = rect.shrink(6.0);
    let c = inner.center();
    // Where the shape is, in the picture.
    let inside: Box<dyn Fn(Pos2) -> bool>;
    let mut outline: Vec<Pos2> = Vec::new();
    match layer {
        PLayer::Box { w, h, .. } => {
            let s = (inner.width() / (*w as f32).max(0.01)).min(inner.height() / (*h as f32).max(0.01));
            let size = vec2(*w as f32 * s, *h as f32 * s).max(vec2(3.0, 3.0));
            let r = Rect::from_center_size(c, size);
            outline = vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            inside = Box::new(move |q| r.contains(q));
        }
        PLayer::Circle { .. } => {
            let rad = inner.width() / 2.0;
            outline = (0..=48)
                .map(|k| {
                    let a = k as f32 / 48.0 * std::f32::consts::TAU;
                    c + vec2(a.cos(), a.sin()) * rad
                })
                .collect();
            inside = Box::new(move |q| (q - c).length() <= rad);
        }
        PLayer::Split { angle, .. } => {
            let a = angle.to_radians();
            let dir = vec2(a.cos() as f32, a.sin() as f32);
            let n = vec2(-dir.y, dir.x);
            outline = vec![c - dir * 80.0, c + dir * 80.0];
            inside = Box::new(move |q| (q - c).dot(n) > 0.0);
        }
        PLayer::Polygon { points } if points.len() >= 3 => {
            let (mut lo, mut hi) = (points[0], points[0]);
            for q in points {
                lo = (lo.0.min(q.0), lo.1.min(q.1));
                hi = (hi.0.max(q.0), hi.1.max(q.1));
            }
            let s = (inner.width() / ((hi.0 - lo.0) as f32).max(0.01))
                .min(inner.height() / ((hi.1 - lo.1) as f32).max(0.01));
            let mid = ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0);
            let pts: Vec<Pos2> = points
                .iter()
                .map(|q| c + vec2((q.0 - mid.0) as f32 * s, (q.1 - mid.1) as f32 * s))
                .collect();
            outline = pts.clone();
            outline.push(pts[0]);
            inside = Box::new(move |q| {
                let mut hit = false;
                for k in 0..pts.len() {
                    let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
                    if (a.y > q.y) != (b.y > q.y) && q.x < (b.x - a.x) * (q.y - a.y) / (b.y - a.y) + a.x {
                        hit = !hit;
                    }
                }
                hit
            });
        }
        PLayer::Polygon { .. } | PLayer::Time { .. } => {
            icons::paint(&p, rect.shrink(9.0), layer.icon(), theme::RED);
            return;
        }
    }
    // Shade and hatch what is removed, a bit of the way as the map does.
    let removed = move |q: Pos2| inside(q) != inverted;
    let mut y = rect.top();
    while y < rect.bottom() {
        let mut x = rect.left();
        while x < rect.right() {
            if removed(pos2(x + 1.0, y + 1.0)) {
                p.rect_filled(Rect::from_min_size(pos2(x, y), vec2(2.0, 2.0)), 0.0, tint);
            }
            x += 2.0;
        }
        y += 2.0;
    }
    hatch_where(&p, rect, 5.0, hatch_line, &removed);
    if outline.len() > 1 {
        p.add(Shape::line(outline, edge));
    }
}
/// Radius (screen pixels) of the area that grabs a whole shape to move it.
const MOVE_AREA: f32 = 14.0;

/// How big the grab area of a shape that is `extent` pixels across is: smaller for a shape
/// seen from far away, so it does not swallow it.
fn move_area(extent: f32) -> f32 {
    (extent * 0.25).clamp(8.0, MOVE_AREA)
}

const COMPARISONS: [(&str, Comparison); 5] = [
    ("<", Comparison::Less),
    ("≤", Comparison::LessOrEqual),
    ("=", Comparison::Equal),
    ("≥", Comparison::GreaterOrEqual),
    (">", Comparison::Greater),
];

impl PLayer {
    fn title(&self) -> &'static str {
        match self {
            PLayer::Box { .. } => "BOX",
            PLayer::Circle { .. } => "CIRCLE",
            PLayer::Split { .. } => "SPLIT",
            PLayer::Polygon { .. } => "POLYGON",
            PLayer::Time { .. } => "TIME",
        }
    }
    fn icon(&self) -> Icon {
        match self {
            PLayer::Box { .. } => Icon::ShapeBox,
            PLayer::Circle { .. } => Icon::ShapeCircle,
            PLayer::Split { .. } => Icon::ShapeSplit,
            PLayer::Polygon { .. } => Icon::ShapePolygon,
            PLayer::Time { .. } => Icon::Time,
        }
    }
    fn to_layer(&self) -> Option<Layer> {
        let b = |v: f64| v * 16.0;
        Some(match self {
            PLayer::Box { x, z, w, h } => Layer::Box {
                x: b(*x),
                z: b(*z),
                width: b(*w),
                height: b(*h),
            },
            PLayer::Circle { x, z, r, .. } => Layer::Circle {
                x: b(*x),
                z: b(*z),
                radius: b(*r),
            },
            PLayer::Split { x, z, angle, .. } => Layer::Split {
                x: b(*x),
                z: b(*z),
                angle: *angle,
            },
            PLayer::Polygon { points } if points.len() >= 3 => Layer::Polygon {
                points: points.iter().map(|(x, z)| (b(*x), b(*z))).collect(),
            },
            PLayer::Polygon { .. } => return None,
            PLayer::Time { cmp, ticks } => Layer::Time {
                comparison: COMPARISONS[*cmp].1,
                ticks: *ticks as i64,
            },
        })
    }
    /// Handle positions (chunk units) for dragging.
    fn handles(&self) -> Vec<(f64, f64)> {
        match self {
            PLayer::Box { x, z, w, h } => vec![
                // The corner dots sit in the middle of the corner chunks.
                (x + 0.5, z + 0.5),
                (x + w - 0.5, z + 0.5),
                (x + w - 0.5, z + h - 0.5),
                (x + 0.5, z + h - 0.5),
                (x + w / 2.0, z + h / 2.0),
            ],
            PLayer::Circle { x, z, r, a } => vec![(*x, *z), (x + r * a.cos(), z + r * a.sin())],
            PLayer::Split { x, z, angle, len } => {
                let a = (angle + 90.0).to_radians();
                vec![(*x, *z), (x + len * a.cos(), z + len * a.sin())]
            }
            PLayer::Polygon { points } => {
                let mut v = points.clone();
                v.extend(self.bounds_center());
                v
            }
            PLayer::Time { .. } => Vec::new(),
        }
    }
    /// For a box, the middles of the chunks around its center: one chunk, two or four, depending
    /// on whether the center is on a chunk edge or corner. The move dot stretches over them.
    fn move_span(&self) -> Option<((f64, f64), (f64, f64))> {
        let PLayer::Box { x, z, w, h } = self else { return None };
        let span = |lo: f64, size: f64| {
            let c = lo + size / 2.0;
            if (c - c.floor() - 0.5).abs() < 1e-6 { (c, c) } else { (c.round() - 0.5, c.round() + 0.5) }
        };
        let (x0, x1) = span(*x, *w);
        let (z0, z1) = span(*z, *h);
        Some(((x0, z0), (x1, z1)))
    }
    /// The point the shape's resize dots are seen from: its middle.
    fn anchor(&self) -> (f64, f64) {
        match self {
            PLayer::Box { x, z, w, h } => (x + w / 2.0, z + h / 2.0),
            PLayer::Circle { x, z, .. } | PLayer::Split { x, z, .. } => (*x, *z),
            PLayer::Polygon { .. } => self.bounds_center().unwrap_or((0.0, 0.0)),
            PLayer::Time { .. } => (0.0, 0.0),
        }
    }
    /// The handle under a screen point: a reshaping dot first, else the grab area that moves it.
    fn pick_handle(&self, q: Pos2, to_screen: &dyn Fn((f64, f64)) -> Pos2) -> Option<usize> {
        let hs = self.handles();
        // The grab area is bigger than the 10 px dot.
        let near = |hi: usize, r: f32| to_screen(hs[hi]).distance(q) < r;
        let near_move = |hi: usize| match self.move_span() {
            Some((lo, hi_)) => {
                let r = Rect::from_two_pos(to_screen(lo), to_screen(hi_)).expand(5.0);
                let (dx, dy) = ((r.left() - q.x).max(q.x - r.right()).max(0.0), (r.top() - q.y).max(q.y - r.bottom()).max(0.0));
                dx.hypot(dy) < 8.0
            }
            None => near(hi, self.move_radius(to_screen)),
        };
        (0..hs.len())
            .find(|hi| !self.is_move_handle(*hi) && near(*hi, 13.0))
            .or_else(|| (0..hs.len()).find(|hi| self.is_move_handle(*hi) && near_move(*hi)))
    }
    /// The radius of the area that moves the shape, on screen.
    fn move_radius(&self, to_screen: &dyn Fn((f64, f64)) -> Pos2) -> f32 {
        let per_chunk = to_screen((1.0, 0.0)).x - to_screen((0.0, 0.0)).x;
        let chunks = match self {
            PLayer::Box { w, h, .. } => w.min(*h),
            PLayer::Circle { r, .. } => r * 2.0,
            PLayer::Split { len, .. } => *len,
            PLayer::Polygon { .. } => {
                let b = self.handles();
                let xs = b.iter().map(|p| p.0);
                let zs = b.iter().map(|p| p.1);
                let span = |v: &mut dyn Iterator<Item = f64>| {
                    let (lo, hi) = v.fold((f64::MAX, f64::MIN), |(l, h), x| (l.min(x), h.max(x)));
                    hi - lo
                };
                span(&mut xs.clone()).min(span(&mut zs.clone()))
            }
            PLayer::Time { .. } => 0.0,
        };
        move_area(chunks as f32 * per_chunk.abs())
    }
    /// Whether handle `i` moves the whole shape (a round dot) rather than reshaping it.
    fn is_move_handle(&self, i: usize) -> bool {
        match self {
            PLayer::Box { .. } => i == 4,
            PLayer::Circle { .. } | PLayer::Split { .. } => i == 0,
            PLayer::Polygon { points } => i == points.len(),
            PLayer::Time { .. } => false,
        }
    }
    /// The middle of a polygon's bounding box.
    fn bounds_center(&self) -> Option<(f64, f64)> {
        let PLayer::Polygon { points } = self else { return None };
        let first = points.first()?;
        let (mut lo, mut hi) = (*first, *first);
        for p in points {
            lo = (lo.0.min(p.0), lo.1.min(p.1));
            hi = (hi.0.max(p.0), hi.1.max(p.1));
        }
        Some(((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0))
    }
    /// The outline in chunk units: closed shapes end where they began; a split is a long line.
    fn outline(&self) -> Vec<(f64, f64)> {
        match self {
            PLayer::Box { x, z, w, h } => {
                // Through the corner dots (the middles of the corner chunks).
                let (x0, z0, x1, z1) = (x + 0.5, z + 0.5, x + w - 0.5, z + h - 0.5);
                vec![(x0, z0), (x1, z0), (x1, z1), (x0, z1), (x0, z0)]
            }
            PLayer::Circle { x, z, r, .. } => (0..=96)
                .map(|k| {
                    let a = k as f64 / 96.0 * std::f64::consts::TAU;
                    (x + r * a.cos(), z + r * a.sin())
                })
                .collect(),
            PLayer::Split { x, z, angle, .. } => {
                // The dividing line, long enough to leave the view; see `arrow` for the other part.
                let a = angle.to_radians();
                vec![
                    (x - 4000.0 * a.cos(), z - 4000.0 * a.sin()),
                    (x + 4000.0 * a.cos(), z + 4000.0 * a.sin()),
                ]
            }
            PLayer::Polygon { points } => {
                let mut v = points.clone();
                if let Some(f) = v.first().copied() {
                    v.push(f);
                }
                v
            }
            PLayer::Time { .. } => Vec::new(),
        }
    }
    /// A split's arrow, from its start to its tip.
    fn arrow(&self) -> Option<((f64, f64), (f64, f64))> {
        matches!(self, PLayer::Split { .. }).then(|| {
            let h = self.handles();
            (h[0], h[1])
        })
    }
    /// Whether a point (chunk units) is inside the area this layer selects.
    fn contains(&self, (px, pz): (f64, f64)) -> bool {
        match self {
            PLayer::Box { x, z, w, h } => px > *x && px < x + w && pz > *z && pz < z + h,
            PLayer::Circle { x, z, r, .. } => (px - x).hypot(pz - z) < *r,
            PLayer::Split { x, z, angle, .. } => {
                let a = angle.to_radians();
                a.cos() * (pz - z) - a.sin() * (px - x) > 0.0
            }
            PLayer::Polygon { points } => {
                let mut inside = false;
                let mut j = points.len().wrapping_sub(1);
                for (k, a) in points.iter().enumerate() {
                    let b = points[j];
                    if (a.1 > pz) != (b.1 > pz) && px < (b.0 - a.0) * (pz - a.1) / (b.1 - a.1) + a.0 {
                        inside = !inside;
                    }
                    j = k;
                }
                inside
            }
            PLayer::Time { .. } => false,
        }
    }
    /// Whether a point is inside the shape as its outline is drawn. A box's outline runs through
    /// the middles of its corner chunks, so that is where other shapes' outlines meet it: they are
    /// hidden inside that line, not inside the chunks' outer edge.
    fn outline_contains(&self, (px, pz): (f64, f64)) -> bool {
        match self {
            PLayer::Box { x, z, w, h } => px > x + 0.5 && px < x + w - 0.5 && pz > z + 0.5 && pz < z + h - 0.5,
            other => other.contains((px, pz)),
        }
    }
    fn drag_handle(&mut self, i: usize, to: (f64, f64)) {
        match self {
            PLayer::Box { x, z, w, h } => {
                if i == 4 {
                    // The middle area moves the box a whole number of chunks.
                    *x += (to.0 - (*x + *w / 2.0)).round();
                    *z += (to.1 - (*z + *h / 2.0)).round();
                    return;
                }
                // The opposite corner chunk stays; the box covers it and the chunk under the dot.
                let (ax, az) = match i {
                    0 => (*x + *w - 0.5, *z + *h - 0.5),
                    1 => (*x + 0.5, *z + *h - 0.5),
                    2 => (*x + 0.5, *z + 0.5),
                    _ => (*x + *w - 0.5, *z + 0.5),
                };
                let (tx, tz) = center(to);
                *x = ax.min(tx) - 0.5;
                *z = az.min(tz) - 0.5;
                *w = (ax - tx).abs() + 1.0;
                *h = (az - tz).abs() + 1.0;
            }
            PLayer::Circle { x, z, r, a } => {
                if i == 0 {
                    (*x, *z) = center(to);
                } else {
                    // The handle sits on the middle of a chunk and can go all the way round.
                    let at = center(to);
                    *r = circle_radius((*x, *z), at);
                    *a = (at.1 - *z).atan2(at.0 - *x);
                }
            }
            PLayer::Split { x, z, angle, len } => {
                if i == 0 {
                    (*x, *z) = center(to);
                } else {
                    let at = center(to);
                    *len = circle_radius((*x, *z), at);
                    *angle = ((at.1 - *z).atan2(at.0 - *x).to_degrees() - 90.0).rem_euclid(360.0);
                }
            }
            PLayer::Polygon { points } => {
                if i == points.len() {
                    if let Some((cx, cz)) = self_bounds(points) {
                        let (dx, dz) = ((to.0 - cx).round(), (to.1 - cz).round());
                        for p in points.iter_mut() {
                            *p = (p.0 + dx, p.1 + dz);
                        }
                    }
                } else if let Some(p) = points.get_mut(i) {
                    *p = center(to);
                }
            }
            PLayer::Time { .. } => {}
        }
    }
}

/// The middle of the bounding box of some points.
fn self_bounds(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    PLayer::Polygon { points: points.to_vec() }.bounds_center()
}

/// The middle of the chunk a point (chunk units) is in: where shapes snap to.
fn center(c: (f64, f64)) -> (f64, f64) {
    (c.0.floor() + 0.5, c.1.floor() + 0.5)
}

/// A circle's radius from its center to a point (the middle of a chunk), at least one chunk.
fn circle_radius(from: (f64, f64), to: (f64, f64)) -> f64 {
    (to.0 - from.0).hypot(to.1 - from.1).max(1.0)
}

/// The part of the segment `a`-`b` inside `rect`, if any.
fn clip_segment(a: Pos2, b: Pos2, rect: Rect) -> Option<(Pos2, Pos2)> {
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    let d = b - a;
    for (p, q) in [
        (-d.x, a.x - rect.left()),
        (d.x, rect.right() - a.x),
        (-d.y, a.y - rect.top()),
        (d.y, rect.bottom() - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    (t0 < t1).then(|| (a + d * t0, a + d * t1))
}

/// The parts of a layer's outline (screen points) that are in view and not inside any of
/// `others`, so overlapping shapes show as one joined outline. The flag is true when the whole
/// outline came through untouched.
fn visible_runs(
    pts: &[Pos2],
    others: &[&PLayer],
    to_chunk: &dyn Fn(Pos2) -> (f64, f64),
    view: Rect,
) -> (Vec<Vec<Pos2>>, bool) {
    let mut runs: Vec<Vec<Pos2>> = Vec::new();
    let mut run: Vec<Pos2> = Vec::new();
    let mut whole = true;
    let flush = |run: &mut Vec<Pos2>, runs: &mut Vec<Vec<Pos2>>| {
        if run.len() > 1 {
            runs.push(std::mem::take(run));
        }
        run.clear();
    };
    for w in pts.windows(2) {
        let Some((a, b)) = clip_segment(w[0], w[1], view) else {
            whole = false;
            flush(&mut run, &mut runs);
            continue;
        };
        if a.distance(w[0]) > 0.5 || b.distance(w[1]) > 0.5 {
            whole = false;
        }
        if run.last().is_some_and(|l| l.distance(a) > 0.5) {
            flush(&mut run, &mut runs);
        }
        let n = ((a.distance(b) / 3.0).ceil() as usize).clamp(1, 4000);
        for k in 0..n {
            let s0 = a.lerp(b, k as f32 / n as f32);
            let s1 = a.lerp(b, (k + 1) as f32 / n as f32);
            let mid = to_chunk(s0.lerp(s1, 0.5));
            if others.iter().any(|o| o.outline_contains(mid)) {
                whole = false;
                flush(&mut run, &mut runs);
            } else {
                if run.is_empty() {
                    run.push(s0);
                }
                run.push(s1);
            }
        }
    }
    flush(&mut run, &mut runs);
    (runs, whole)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Box,
    Circle,
    Polygon,
    Split,
}

/// The World Explorer's Prune tools: shapes drawn on the flat map pick the chunks to delete
/// (or, inverted, the ones to keep), with the chunks of the world shown read to know which
/// exist (and their InhabitedTime, for a Time layer).
#[derive(Default)]
pub struct PruneTools {
    tool: Option<Tool>,
    layers: Vec<PLayer>,
    selected: Option<usize>,
    inverted: bool,
    drawing: Option<((f64, f64), (f64, f64))>,
    polygon: Vec<(f64, f64)>,
    dragging: Option<(usize, usize)>,
    /// The chunks last shaded as removed, with what they were worked out for, so a still
    /// picture is not recomputed every frame.
    shade: Option<(ShadeKey, std::rc::Rc<Vec<(i32, i32)>>)>,
    /// The chunks of the dimension shown, by position, with their InhabitedTime once a Time
    /// layer needs it: what the list was read for (world, dimension, with times), the reading
    /// in progress, and the result. Only chunks that exist can be removed.
    chunks_for: Option<(std::path::PathBuf, String, bool)>,
    chunks_job: Option<Job<std::collections::HashMap<(i32, i32), Option<i64>>>>,
    chunks: std::rc::Rc<std::collections::HashMap<(i32, i32), Option<i64>>>,
    chunks_loaded: bool,
    /// The picture of where the shapes are (see `overlay`), with what it was drawn for.
    mask: Option<(MaskKey, egui::TextureHandle)>,
    /// A message for the page (a chunk list that could not be read...).
    status: String,
}

type ShadeKey = (Vec<PLayer>, bool, (i32, i32, i32, i32));
/// What the shape mask was drawn for: the shapes and the view (as bits).
type MaskKey = (Vec<PLayer>, [u32; 8]);

fn selection(layers: &[PLayer], inverted: bool) -> Selection {
    Selection {
        layers: layers.iter().filter_map(PLayer::to_layer).collect(),
        inverted,
    }
}

/// Diagonal hatch lines (direction (1, 1)) clipped to `r`, spaced `s` px in screen space.
fn hatch(p: &egui::Painter, r: Rect, s: f32, stroke: Stroke) {
    let (x0, x1, y0, y1) = (r.left(), r.right(), r.top(), r.bottom());
    let c0 = ((x0 - y1) / s).floor() as i64;
    let c1 = ((x1 - y0) / s).ceil() as i64;
    for k in c0..=c1 {
        let c = k as f32 * s; // line: x - y = c
        let xa = x0.max(y0 + c);
        let xb = x1.min(y1 + c);
        if xa < xb {
            p.line_segment([pos2(xa, xa - c), pos2(xb, xb - c)], stroke);
        }
    }
}

impl PruneTools {
    /// The shape tools, Normal or Inverted, the layers' cards and the Prune button at the
    /// panel's foot. Gives whether Prune was pressed.
    pub(crate) fn tool_panel(&mut self, ui: &mut Ui, panel: Rect, bedrock: bool) -> bool {
        let tools: [(Option<Tool>, Icon, &str); 5] = [
            (Some(Tool::Box), Icon::ShapeBox, "Box"),
            (Some(Tool::Circle), Icon::ShapeCircle, "Circle"),
            (Some(Tool::Polygon), Icon::ShapePolygon, "Polygon"),
            (Some(Tool::Split), Icon::ShapeSplit, "Split"),
            (None, Icon::Time, "Time"),
        ];
        // Sized to the room, so the layers' cards below still show in a small window: the five
        // in one row, their names under them only when the panel is tall (and on hover).
        let tall = panel.height() >= 760.0;
        let (bh, icon_size, named) = if tall { (54.0, 22.0, true) } else { (38.0, 22.0, false) };
        let per_row = tools.len();
        let w = (LEFT_PANEL - 12.0 - 4.0 * (per_row as f32 - 1.0)) / per_row as f32;
        for row in tools.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (tool, icon, label) in row {
                    let time = *label == "Time";
                    // Only Java and the consoles record InhabitedTime; Bedrock has none to go by.
                    let enabled = !(time && bedrock);
                    let active = tool.is_some() && self.tool == *tool;
                    let (r, resp) = ui.allocate_exact_size(
                        vec2(w, bh),
                        if enabled {
                            Sense::click()
                        } else {
                            Sense::hover()
                        },
                    );
                    ui.painter().rect_filled(
                        r,
                        theme::radius(4),
                        if resp.hovered() && enabled {
                            theme::button_hover()
                        } else {
                            theme::tile()
                        },
                    );
                    if active {
                        ui.painter().rect_stroke(
                            r,
                            theme::radius(4),
                            Stroke::new(1.2_f32, theme::accent()),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let c = if enabled {
                        theme::text()
                    } else {
                        theme::text_dim()
                    };
                    let lift = if named { bh * 0.14 } else { 0.0 };
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(r.center() - vec2(0.0, lift), Vec2::splat(icon_size)),
                        *icon,
                        c,
                    );
                    if named {
                        ui.painter().text(
                            r.center() + vec2(0.0, bh * 0.3),
                            Align2::CENTER_CENTER,
                            *label,
                            theme::regular(12.0),
                            c,
                        );
                    }
                    let resp = if time {
                        resp.on_hover_text(if bedrock {
                            "Bedrock worlds record no InhabitedTime, so there is nothing to prune by time"
                        } else {
                            "Prune the chunks by the time players spent near them (InhabitedTime)"
                        })
                    } else {
                        // (The name, when the button has no room to show it.)
                        let tip = match tool {
                            Some(Tool::Box) => "Box: drag on the map to draw one",
                            Some(Tool::Circle) => "Circle: drag from the middle on the map",
                            Some(Tool::Polygon) => "Polygon: click the corners on the map; right-click places the last",
                            Some(Tool::Split) => "Split: drag a line on the map; the side to its right is picked",
                            None => label,
                        };
                        resp.on_hover_text(tip)
                    };
                    if resp.clicked() && enabled {
                        match tool {
                            Some(t) => {
                                self.tool = if self.tool == Some(*t) {
                                    None
                                } else {
                                    Some(*t)
                                };
                                self.polygon.clear();
                            }
                            None if *label == "Time" => {
                                // Equal to 0 ticks, which picks the chunks nobody
                                // has spent time near.
                                self.layers.push(PLayer::Time {
                                    cmp: 2,
                                    ticks: 0.0,
                                });
                                self.selected = Some(self.layers.len() - 1);
                            }
                            None => {}
                        }
                    }
                }
            });
        }
        ui.add_space(6.0);
        let mut mode = self.inverted as usize;
        if widgets::segmented(
            ui,
            &mut mode,
            &["Normal", "Inverted"],
            LEFT_PANEL - 12.0,
            true,
        ) {
            self.inverted = mode == 1;
        }
        ui.add_space(8.0);
        let prune_h = if tall { 46.0 } else { 36.0 };
        let list_h = (ui.available_height() - prune_h - 24.0).max(0.0);
        egui::ScrollArea::vertical()
            .max_height(list_h)
            .auto_shrink(false)
            .show(ui, |ui| {
                let mut remove = None;
                for (i, l) in self.layers.iter_mut().enumerate() {
                    let sel = self.selected == Some(i);
                    // The card is a click target for selecting it. It is made first (from where the
                    // card was last frame) so the buttons inside, made later, win the clicks.
                    let rect_key = ui.id().with(("layer-rect", i));
                    let last_rect: Option<Rect> = ui.data(|d| d.get_temp(rect_key));
                    let picked = last_rect
                        .map(|r| ui.interact(r, ui.id().with(("layer", i)), Sense::click()))
                        .is_some_and(|r| r.clicked());
                    let card_w = ui.available_width() - 16.0;
                    let frame = egui::Frame::new()
                        .fill(theme::field())
                        .inner_margin(egui::Margin::same(8));
                    let resp = frame.show(ui, |ui| {
                        ui.set_width(card_w);
                        let top = ui.max_rect().left_top() - vec2(8.0, 8.0);
                        if sel {
                            ui.painter().line_segment(
                                [top, top + vec2(card_w + 16.0, 0.0)],
                                Stroke::new(1.5_f32, theme::accent()),
                            );
                        }
                        // Thumbnail, name and the remove button on one line.
                        ui.horizontal(|ui| {
                            let (pv, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                            draw_thumb(ui.painter(), pv, &*l, self.inverted);
                            ui.label(egui::RichText::new(l.title()).size(12.5).color(if sel {
                                theme::accent()
                            } else {
                                theme::text_muted()
                            }));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if widgets::square_icon_button(ui, Icon::Trash, "Remove layer")
                                        .clicked()
                                    {
                                        remove = Some(i);
                                    }
                                },
                            );
                        });
                        ui.add_space(6.0);
                        // The fields use the card's whole width, two to a line.
                        let total = card_w;
                        let half = (total - 6.0) / 2.0;
                        let pair = |ui: &mut Ui, a: (&str, &mut f64), b: (&str, &mut f64)| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                widgets::labeled_field(ui, a.0, a.1, half, 0.1);
                                widgets::labeled_field(ui, b.0, b.1, half, 0.1);
                            });
                        };
                        ui.spacing_mut().item_spacing.y = 6.0;
                        match l {
                            PLayer::Box { x, z, w, h } => {
                                pair(ui, ("X", x), ("Z", z));
                                pair(ui, ("W", w), ("H", h));
                            }
                            PLayer::Circle { x, z, r, .. } => {
                                pair(ui, ("X", x), ("Z", z));
                                widgets::labeled_field(ui, "Radius", r, total, 0.1);
                            }
                            PLayer::Split { x, z, angle, .. } => {
                                pair(ui, ("X", x), ("Z", z));
                                widgets::labeled_field(ui, "Angle", angle, total, 1.0);
                            }
                            PLayer::Polygon { points } => {
                                ui.label(
                                    egui::RichText::new(format!("{} points", points.len()))
                                        .color(theme::text()),
                                );

                            }
                            PLayer::Time { cmp, ticks } => {
                                ui.label(
                                    egui::RichText::new("Select if InhabitedTime is:")
                                        .italics()
                                        .size(12.5)
                                        .color(theme::text()),
                                );
                                ui.horizontal(|ui| {
                                    // Greater than, equal to, less than.
                                    const ORDER: [usize; 3] = [4, 2, 0];
                                    let mut at = ORDER.iter().position(|c| c == cmp).unwrap_or(1);
                                    if widgets::segmented(ui, &mut at, &[">", "=", "<"], 96.0, true) {
                                        *cmp = ORDER[at];
                                    }
                                    ui.add(
                                        egui::DragValue::new(ticks)
                                            .speed(20.0)
                                            .range(0.0..=f64::MAX),
                                    );
                                });
                            }
                        }
                    });
                    ui.data_mut(|d| d.insert_temp(rect_key, resp.response.rect));
                    if picked {
                        self.selected = Some(i);
                    }                    ui.add_space(4.0);
                }
                if let Some(i) = remove {
                    self.layers.remove(i);
                    self.selected = None;
                }
            });
        // The Prune button at the panel's foot.
        let r = Rect::from_min_size(
            pos2(panel.left() + 6.0, panel.bottom() - prune_h - 8.0),
            vec2(LEFT_PANEL - 12.0, prune_h),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        let enabled = !self.layers.is_empty();
        let resp = widgets::primary(&mut child, "Prune", r.size(), enabled).on_hover_text("Delete the chunks shaded red (Ctrl+Z undoes; Save writes it)");
        resp.clicked() && enabled
    }

    /// The chunks the layers pick (those that exist), once the world's chunks are read: what
    /// Prune removes.
    pub(crate) fn picked_chunks(&self, bedrock: bool) -> Option<Vec<(i32, i32)>> {
        if !self.chunks_loaded {
            return None;
        }
        let mut layers = self.layers.clone();
        if bedrock {
            layers.retain(|l| !matches!(l, PLayer::Time { .. }));
        }
        let sel = selection(&layers, self.inverted);
        // (Inverted with nothing drawn would be every chunk: a shape first.)
        if sel.layers.is_empty() {
            return Some(Vec::new());
        }
        let mut v: Vec<(i32, i32)> = self.chunks.iter().filter(|(k, t)| !sel.keeps(ChunkPos::new(k.0, k.1), **t)).map(|(k, _)| *k).collect();
        v.sort_unstable();
        Some(v)
    }

    /// After a prune in the World Explorer: the shapes are cleared (the chunks are gone).
    pub(crate) fn clear_layers(&mut self) {
        self.layers.clear();
        self.selected = None;
        self.tool = None;
        self.polygon.clear();
        self.drawing = None;
        self.dragging = None;
        self.shade = None;
    }

    /// Puts the shape tool down (and drops a shape half drawn).
    pub(crate) fn drop_tool(&mut self) {
        self.tool = None;
        self.polygon.clear();
        self.drawing = None;
        self.dragging = None;
    }

    /// Whether a shape tool is chosen (the map's clicks are its).
    pub(crate) fn drawing_tool(&self) -> bool {
        self.tool.is_some()
    }

    /// For development: a box layer (in chunks), chosen.
    pub(crate) fn dev_box(&mut self, x: f64, z: f64, w: f64, h: f64) {
        self.layers.push(PLayer::Box { x, z, w, h });
        self.selected = Some(self.layers.len() - 1);
    }

    /// Whether there is anything to draw on the map: shapes, or one being made.
    pub(crate) fn has_shapes(&self) -> bool {
        !self.layers.is_empty() || self.tool.is_some() || !self.polygon.is_empty()
    }

    /// The messages it left (a chunk list that could not be read...), taken.
    pub(crate) fn take_status(&mut self) -> String {
        std::mem::take(&mut self.status)
    }
    /// Reads, in the background, which chunks the dimension shown has (and their InhabitedTime
    /// when a Time layer needs it).
    pub(crate) fn read_chunks(&mut self, ctx: &egui::Context, ws: &Workspace) {
        if let Some(r) = self.chunks_job.as_ref().and_then(Job::take) {
            self.chunks_job = None;
            self.shade = None;
            match r {
                Ok(c) => {
                    self.chunks = std::rc::Rc::new(c);
                    self.chunks_loaded = true;
                }
                Err(e) => self.status = format!("Could not read the world's chunks: {e}"),
            }
        }
        let (path, edition) = (ws.path.clone(), ws.edition);
        let dim = match ws.map.dimension.as_str() {
            "minecraft:the_nether" => -1,
            "minecraft:the_end" => 1,
            _ => 0,
        };
        let with_times = edition != UiEdition::Bedrock && self.layers.iter().any(|l| matches!(l, PLayer::Time { .. }));
        let key = (path.clone(), ws.map.dimension.clone(), with_times);
        if self.chunks_for.as_ref() == Some(&key) {
            return;
        }
        self.chunks_for = Some(key);
        self.chunks_loaded = false;
        self.shade = None;
        self.chunks_job = Some(Job::spawn_notifying("Reading chunks", ctx, move || {
            match edition {
                UiEdition::Java => mctk_prune::chunk_times_java(&path, dim, with_times),
                // Bedrock numbers its dimensions Overworld 0, Nether 1, End 2.
                UiEdition::Bedrock => mctk_prune::chunk_times_bedrock(&path, if dim == 0 { 0 } else if dim == -1 { 1 } else { 2 }),
                _ => mctk_prune::chunk_times_console(&path, dim, with_times),
            }
            .map_err(|e| e.to_string())
        }));
    }

    /// Whether a drag on the map moves it (not while a shape tool is chosen or a shape's dot is
    /// being dragged).
    pub(crate) fn map_pans(&self) -> bool {
        self.tool.is_none() && self.dragging.is_none()
    }

    /// The shapes drawn on the flat map (the chunks they pick shaded), their dots, and the tool
    /// chosen drawing a new one.
    pub(crate) fn overlay(&mut self, ui: &mut Ui, ws: &mut Workspace, frame: &MapFrame) {
        let p = ui.painter_at(frame.rect);
        // The shape being drawn is shaded too, so the prune area shows while placing it.
        let mut shown = self.layers.clone();
        // Bedrock worlds record no InhabitedTime, so a Time layer picks nothing there.
        if ws.edition == UiEdition::Bedrock {
            shown.retain(|l| !matches!(l, PLayer::Time { .. }));
        }
        shown.extend(self.preview_layer(frame.hover));
        let sel = selection(&shown, self.inverted);
        let shapes: Vec<PLayer> = shown.iter().filter(|l| !matches!(l, PLayer::Time { .. })).cloned().collect();
        let shapes_only = selection(&shapes, self.inverted);
        // Shade removed chunks.
        let b0 = frame.to_block(frame.rect.min);
        let b1 = frame.to_block(frame.rect.max);
        let (cx0, cz0) = ((b0.x / 16.0).floor() as i32, (b0.y / 16.0).floor() as i32);
        let (cx1, cz1) = ((b1.x / 16.0).ceil() as i32, (b1.y / 16.0).ceil() as i32);
        let cells = ((cx1 - cx0) as i64) * ((cz1 - cz0) as i64);
        // (Inverted with no shapes removes everything, so everything is shaded.)
        if (!sel.layers.is_empty() || self.inverted) && cells < 120_000 {
            let tint = Color32::from_rgba_unmultiplied(170, 40, 40, 80);
            let line = Stroke::new(1.3_f32, Color32::from_rgba_unmultiplied(235, 60, 60, 190));
            let key: ShadeKey = (shown.clone(), self.inverted, (cx0, cz0, cx1, cz1));
            // Removing only touches chunks that exist, so in the normal mode only those are
            // shaded (the inverted mode removes everything outside the shapes, so there it is
            // everywhere). Until the chunks are known nothing is shaded in the normal mode.
            let removed = match &self.shade {
                _ if !self.inverted && !self.chunks_loaded => std::rc::Rc::new(Vec::new()),
                Some((k, v)) if *k == key => v.clone(),
                _ => {
                    let mut v = Vec::new();
                    for cz in cz0..cz1 {
                        for cx in cx0..cx1 {
                            let known = self.chunks.get(&(cx, cz));
                            if !self.inverted && known.is_none() {
                                continue;
                            }
                            // A Time layer selects by what was read from the chunk, so it never
                            // selects a place with no chunk.
                            let keeps = match known {
                                Some(time) => sel.keeps(ChunkPos::new(cx, cz), *time),
                                None => shapes_only.keeps(ChunkPos::new(cx, cz), None),
                            };
                            if !keeps {
                                v.push((cx, cz));
                            }
                        }
                    }
                    let v = std::rc::Rc::new(v);
                    self.shade = Some((key, v.clone()));
                    v
                }
            };
            for &(cx, cz) in removed.iter() {
                let a = frame.to_screen(vec2(cx as f32 * 16.0, cz as f32 * 16.0));
                let b = frame.to_screen(vec2(cx as f32 * 16.0 + 16.0, cz as f32 * 16.0 + 16.0));
                let r = Rect::from_min_max(a, b);
                let under_shape = !self.inverted && shapes_only.selects(ChunkPos::new(cx, cz), None);
                if !under_shape {
                    p.rect_filled(r, 0.0, tint);
                }
                if cells < 30_000 {
                    hatch(&p, r, 16.0, line);
                }
            }
        }
        let to_screen = |c: (f64, f64)| frame.to_screen(vec2(c.0 as f32 * 16.0, c.1 as f32 * 16.0));        // The shapes themselves are tinted too, exactly: one picture of where any shape is, so
        // overlapping shapes are one area and the tint does not pile up into a solid color. (The
        // chunks above are what will go, a whole chunk at a time; this is the shape that picked
        // them.)
        if !self.inverted && !shapes.is_empty() {
            let key: MaskKey = (
                shapes.clone(),
                [frame.rect.min.x, frame.rect.min.y, frame.rect.max.x, frame.rect.max.y, b0.x, b0.y, b1.x, b1.y].map(f32::to_bits),
            );
            let fresh = !matches!(&self.mask, Some((k, _)) if *k == key);
            if fresh {
                const STEP: f32 = 3.0;
                let (w, h) = ((frame.rect.width() / STEP).ceil() as usize, (frame.rect.height() / STEP).ceil() as usize);
                let tint = Color32::from_rgba_unmultiplied(170, 40, 40, 80);
                let mut pixels = Vec::with_capacity(w * h);
                for j in 0..h {
                    for i in 0..w {
                        let at = frame.rect.min + vec2((i as f32 + 0.5) * STEP, (j as f32 + 0.5) * STEP);
                        let block = frame.to_block(at);
                        let chunk = (block.x as f64 / 16.0, block.y as f64 / 16.0);
                        pixels.push(if shapes.iter().any(|l| l.contains(chunk)) { tint } else { Color32::TRANSPARENT });
                    }
                }
                let image = egui::ColorImage { size: [w.max(1), h.max(1)], pixels };
                let texture = ui.ctx().load_texture("prune-shapes", image, egui::TextureOptions::LINEAR);
                self.mask = Some((key, texture));
            }
            if let Some((_, texture)) = &self.mask {
                p.image(
                    texture.id(),
                    frame.rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }        // The chosen layer's outline marches (a dashed white line moving over a black
        // one); the others are plain red lines, joined where shapes overlap.
        let time = ui.input(|i| i.time);
        ui.ctx().request_repaint();
        let view = frame.rect.expand(40.0);
        let ants = |p: &egui::Painter, pts: &[Pos2], closed: bool| {
            let pts: Vec<Pos2> = if pts.len() == 2 {
                match clip_segment(pts[0], pts[1], view) {
                    Some((a, b)) => vec![a, b],
                    None => return,
                }
            } else {
                pts.to_vec()
            };
            p.add(Shape::line(pts.clone(), Stroke::new(2.4_f32, Color32::BLACK)));
            let len: f32 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
            // A closed outline gets a dash length that divides it exactly, so the pattern
            // meets itself at the start with no visible seam.
            let (dash, offset) = if closed && len > 20.0 {
                let period = len / (len / 20.0).round().max(1.0);
                (period / 2.0, (time * 24.0) as f32 % period)
            } else {
                (10.0, (time * 24.0 % 20.0) as f32)
            };
            p.extend(Shape::dashed_line_with_offset(
                &pts,
                Stroke::new(2.4_f32, Color32::WHITE),
                &[dash],
                &[dash],
                offset,
            ));
        };
        let to_chunk = |q: Pos2| {
            let b = frame.to_block(q);
            ((b.x / 16.0) as f64, (b.y / 16.0) as f64)
        };
        // Before the shapes are drawn, so one just made shows its dots at once.
        self.interact(frame);
        // A dot dragged (or a shape drawn) near the edge scrolls the map along.
        if self.dragging.is_some() || self.drawing.is_some() {
            if let Some(q) = frame.response.interact_pointer_pos() {
                ws.map.edge_scroll(ui.ctx(), frame.rect, q);
            }
        }
        let pointer = frame.response.hover_pos();
        // The arrow of a split: a shaft and head, each over a black one for contrast.
        let head = |p: &egui::Painter, from: Pos2, tip: Pos2, white: Color32, width: f32| {
            let d = (tip - from).normalized();
            if !d.x.is_finite() {
                return;
            }
            let side = vec2(-d.y, d.x);
            for (w, c) in [(width + 1.8, Color32::BLACK), (width, white)] {
                let back = tip - d * 14.0;
                p.add(Shape::line(vec![from, tip], Stroke::new(w, c)));
                p.add(Shape::line(
                    vec![back + side * 8.0, tip, back - side * 8.0],
                    Stroke::new(w, c),
                ));
            }
        };
        for (i, l) in self.layers.iter().enumerate() {
            let outline = l.outline();
            let pts: Vec<Pos2> = outline.iter().map(|c| to_screen(*c)).collect();
            let closed = !matches!(l, PLayer::Split { .. });
            if pts.len() < 2 {
                continue;
            }
            if let Some((tail, tip)) = l.arrow() {
                // The dividing line, and the arrow to the side that is selected.
                let (tail, tip) = (to_screen(tail), to_screen(tip));
                if self.selected == Some(i) {
                    ants(&p, &pts, false);
                    // Ends just short of the dot at the tip, so the head is not covered.
                    let dir = (tip - tail).normalized();
                    let stop = if dir.x.is_finite() { tip - dir * 9.0 } else { tip };
                    if (stop - tail).dot(dir) > 4.0 {
                        head(&p, tail, stop, Color32::WHITE, 2.2);
                    }
                } else {
                    p.add(Shape::line(pts.clone(), Stroke::new(2.0_f32, theme::RED)));
                }
            } else if self.selected == Some(i) {
                // The chosen shape keeps its whole outline, marching, over the others.
                ants(&p, &pts, closed);
            } else {
                // The others are plain red lines, joined where shapes overlap.
                let others: Vec<&PLayer> = self
                    .layers
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, o)| o)
                    .collect();
                let (runs, _) = visible_runs(&pts, &others, &to_chunk, view);
                for run in runs {
                    p.add(Shape::line(run, Stroke::new(2.0_f32, theme::RED)));
                }
            }
            if self.selected == Some(i) {
                // Round dots move the whole shape, squares reshape it; one under the pointer
                // (or being dragged) turns red and the cursor becomes the move cursor.
                let under = if self.dragging.is_none() {
                    pointer.and_then(|q| l.pick_handle(q, &to_screen))
                } else {
                    None
                };
                for (hi, h) in l.handles().into_iter().enumerate() {
                    let s = to_screen(h);
                    let dragged = self.dragging == Some((i, hi));
                    let hot = dragged || under == Some(hi);
                    let moves = l.is_move_handle(hi);
                    if hot {
                        let cursor = if moves {
                            egui::CursorIcon::Move
                        } else {
                            // The resize cursor along the line from the shape's middle to the dot.
                            let from = to_screen(l.anchor());
                            let deg = (s.y - from.y).atan2(s.x - from.x).to_degrees().rem_euclid(180.0);
                            match deg {
                                d if !(22.5..157.5).contains(&d) => egui::CursorIcon::ResizeHorizontal,
                                d if d < 67.5 => egui::CursorIcon::ResizeNwSe,
                                d if d < 112.5 => egui::CursorIcon::ResizeVertical,
                                _ => egui::CursorIcon::ResizeNeSw,
                            }
                        };
                        ui.ctx().set_cursor_icon(cursor);
                    }
                    let fill = if hot { theme::RED } else { Color32::WHITE };
                    // Every dot is the same small square on screen (whatever the zoom or turn);
                    // the area that grabs the move dot is bigger than what is drawn.
                    let sq = Rect::from_center_size(s, Vec2::splat(10.0));
                    // The move dot is see-through until it is used.
                    let faint = moves && !hot;
                    p.rect_filled(sq, 0.0, if faint { Color32::from_white_alpha(40) } else { fill });
                    p.rect_stroke(
                        sq,
                        0.0,
                        Stroke::new(1.0_f32, if faint { Color32::from_black_alpha(70) } else { Color32::BLACK }),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        }
        // Shape being drawn.
        if let Some((a, b)) = self.drawing {
            let preview = match self.tool {
                Some(Tool::Box) => {
                    // The whole chunks it will cover, as it will be made.
                    let (x0, x1) = (a.0.min(b.0).floor(), a.0.max(b.0).ceil());
                    let (z0, z1) = (a.1.min(b.1).floor(), a.1.max(b.1).ceil());
                    Some(vec![
                        to_screen((x0, z0)),
                        to_screen((x1, z0)),
                        to_screen((x1, z1)),
                        to_screen((x0, z1)),
                        to_screen((x0, z0)),
                    ])
                }
                Some(Tool::Circle) => {
                    let r = circle_radius(a, b);
                    Some(
                        (0..=64)
                            .map(|k| {
                                let t = k as f64 / 64.0 * std::f64::consts::TAU;
                                to_screen((a.0 + r * t.cos(), a.1 + r * t.sin()))
                            })
                            .collect(),
                    )
                }
                Some(Tool::Split) => {
                    let l = PLayer::Split {
                        x: a.0,
                        z: a.1,
                        angle: ((b.1 - a.1).atan2(b.0 - a.0).to_degrees() - 90.0).rem_euclid(360.0),
                        len: circle_radius(a, b),
                    };
                    let line: Vec<Pos2> = l.outline().iter().map(|c| to_screen(*c)).collect();
                    ants(&p, &line, false);
                    let (from, tip) = (to_screen(a), to_screen(b));
                    let dir = (tip - from).normalized();
                    if dir.x.is_finite() && from.distance(tip) > 14.0 {
                        head(&p, from, tip - dir * 9.0, Color32::WHITE, 2.2);
                    }
                    None
                }
                _ => None,
            };
            if let Some(pts) = preview {
                ants(&p, &pts, !matches!(self.tool, Some(Tool::Split)));
            }
        }
        if !self.polygon.is_empty() {
            // Always drawn closed: the corners so far, the pointer, then back to the first.
            let mut pts: Vec<Pos2> = self.polygon.iter().map(|c| to_screen(*c)).collect();
            if let Some(h) = frame.hover {
                pts.push(to_screen(center((h.x as f64 / 16.0, h.y as f64 / 16.0))));
            }
            let corners = pts.clone();
            pts.push(pts[0]);
            ants(&p, &pts, true);
            for q in &corners {
                p.circle_filled(*q, 4.0, Color32::WHITE);
            }
        }
        if let Some(tool) = self.tool {
            let hint = match tool {
                Tool::Box => "Drag to draw a box",
                Tool::Circle => "Drag from the center to draw a circle",
                Tool::Split => "Drag to draw the split line; the right side is selected",
                Tool::Polygon => "Click to add corners; right-click to place the last corner",
            };
            p.text(
                frame.rect.center_bottom() - vec2(0.0, 40.0),
                Align2::CENTER_CENTER,
                hint,
                theme::regular(13.0),
                Color32::WHITE,
            );
        }
        // No chunk is outlined under the pointer while it is on a dot or area of the chosen shape.
        // (While one is being dragged the chunk under the pointer is outlined again.)
        let on_handle = self.dragging.is_none()
            && self.selected.and_then(|i| self.layers.get(i)).is_some_and(|l| {
                frame
                    .response
                    .hover_pos()
                    .is_some_and(|q| l.pick_handle(q, &to_screen).is_some())
            });
        ws.map.hide_hover = on_handle;
    }

    /// The layer the shape being drawn would become, for shading while placing it.
    fn preview_layer(&self, hover: Option<Vec2>) -> Option<PLayer> {
        if !self.polygon.is_empty() {
            let mut points = self.polygon.clone();
            if let Some(h) = hover {
                points.push(center((h.x as f64 / 16.0, h.y as f64 / 16.0)));
            }
            return (points.len() >= 3).then_some(PLayer::Polygon { points });
        }
        let (a, c) = self.drawing?;
        Some(match self.tool? {
            Tool::Box => {
                let (x0, x1) = (a.0.min(c.0).floor(), a.0.max(c.0).ceil());
                let (z0, z1) = (a.1.min(c.1).floor(), a.1.max(c.1).ceil());
                PLayer::Box {
                    x: x0,
                    z: z0,
                    w: x1 - x0,
                    h: z1 - z0,
                }
            }
            Tool::Circle => PLayer::Circle {
                x: a.0,
                z: a.1,
                r: circle_radius(a, c),
                a: (c.1 - a.1).atan2(c.0 - a.0),
            },
            Tool::Split => PLayer::Split {
                x: a.0,
                z: a.1,
                angle: ((c.1 - a.1).atan2(c.0 - a.0).to_degrees() - 90.0).rem_euclid(360.0), len: circle_radius(a, c),
            },
            Tool::Polygon => return None,
        })
    }

    fn interact(&mut self, frame: &MapFrame) {
        let resp = &frame.response;
        let chunk = |b: Vec2| ((b.x / 16.0) as f64, (b.y / 16.0) as f64);
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        // Handle dragging for the selected layer.
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let (Some(i), Some(pos)) = (self.selected, pointer) {
                if let Some(l) = self.layers.get(i) {
                    let to_screen =
                        |c: (f64, f64)| frame.to_screen(vec2(c.0 as f32 * 16.0, c.1 as f32 * 16.0));
                    if let Some(hi) = l.pick_handle(pos, &to_screen) {
                        self.dragging = Some((i, hi));
                    }
                }
            }
        }
        if let Some((i, hi)) = self.dragging {
            if let Some(pos) = pointer {
                let c = chunk(frame.to_block(pos));
                if let Some(l) = self.layers.get_mut(i) {
                    l.drag_handle(hi, c);
                }
            }
            if resp.drag_stopped() || !resp.dragged() {
                self.dragging = None;
            }
            return;
        }
        let Some(tool) = self.tool else { return };
        let Some(pos) = pointer else { return };
        // New shapes snap to the middle of the chunk under the pointer.
        let c = center(chunk(frame.to_block(pos)));
        match tool {
            // Left-click adds a corner, right-click places the last one and closes
            // the polygon back to the first corner.
            Tool::Polygon => {
                if resp.clicked_by(egui::PointerButton::Primary) {
                    self.polygon.push(c);
                } else if resp.clicked_by(egui::PointerButton::Secondary) {
                    self.polygon.push(c);
                    if self.polygon.len() >= 3 {
                        self.layers.push(PLayer::Polygon {
                            points: std::mem::take(&mut self.polygon),
                        });
                        self.selected = Some(self.layers.len() - 1);
                        self.tool = None;
                    } else {
                        self.polygon.pop();
                    }
                }
            }
            _ => {
                if resp.drag_started_by(egui::PointerButton::Primary) {
                    self.drawing = Some((c, c));
                }
                if let Some((a, _)) = self.drawing {
                    self.drawing = Some((a, c));
                    if resp.drag_stopped() {
                        self.drawing = None;
                        let layer = match tool {
                            Tool::Box => {
                                let (x0, x1) = (a.0.min(c.0).floor(), a.0.max(c.0).ceil());
                                let (z0, z1) = (a.1.min(c.1).floor(), a.1.max(c.1).ceil());
                                PLayer::Box {
                                    x: x0,
                                    z: z0,
                                    w: x1 - x0,
                                    h: z1 - z0,
                                }
                            }
                            Tool::Circle => PLayer::Circle {
                                x: a.0,
                                z: a.1,
                                r: circle_radius(a, c),
                                a: (c.1 - a.1).atan2(c.0 - a.0),
                            },
                            _ => PLayer::Split {
                                x: a.0,
                                z: a.1,
                                angle: ((c.1 - a.1).atan2(c.0 - a.0).to_degrees() - 90.0).rem_euclid(360.0), len: circle_radius(a, c),
                            },
                        };
                        self.layers.push(layer);
                        self.selected = Some(self.layers.len() - 1);
                        self.tool = None;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::path::Path;

    /// In the World Explorer: a box drawn picks the chunks under it (inverted, all the others),
    /// and Prune deletes them as one step that Undo brings back.
    #[test]
    fn explorer_prune_picks_the_shapes_chunks_and_undoes() {
        let Ok(src) = std::env::var("MCTK_SNAPSHOT_WORLD") else { return };
        let tmp = std::env::temp_dir().join(format!("mctk-explorer-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for d in ["region", "entities"] {
            std::fs::create_dir_all(tmp.join(d)).unwrap();
            for f in std::fs::read_dir(Path::new(&src).join(d)).unwrap().flatten() {
                std::fs::copy(f.path(), tmp.join(d).join(f.file_name())).unwrap();
            }
        }
        std::fs::copy(Path::new(&src).join("level.dat"), tmp.join("level.dat")).unwrap();
        let mut ws = Workspace::open(&tmp, UiEdition::Java, &Settings::default(), false, false).expect("opens");
        ws.show_explorer = true;
        let mut page = PruneTools::default();
        assert_eq!(page.picked_chunks(false), None, "nothing until the chunks are read");
        page.chunks = std::rc::Rc::new(mctk_prune::chunk_times_java(&tmp, 0, false).expect("chunks"));
        page.chunks_loaded = true;
        assert_eq!(page.picked_chunks(false), Some(Vec::new()), "nothing drawn, nothing picked");
        page.inverted = true;
        assert_eq!(page.picked_chunks(false), Some(Vec::new()), "inverted with nothing drawn picks nothing either");
        page.inverted = false;
        page.layers.push(PLayer::Box { x: 0.0, z: 0.0, w: 2.0, h: 2.0 });
        let picked = page.picked_chunks(false).unwrap();
        assert!(!picked.is_empty() && picked.iter().all(|&(x, z)| (0..=2).contains(&x) && (0..=2).contains(&z)), "under the box: {picked:?}");
        page.inverted = true;
        let others = page.picked_chunks(false).unwrap();
        assert_eq!(picked.len() + others.len(), page.chunks.len(), "inverted: every other chunk");
        assert!(others.iter().all(|c| !picked.contains(c)));
        page.inverted = false;

        let read = |ws: &Workspace, c: (i32, i32)| crate::gpu_world::read_box(&ws.path, UiEdition::Java.world(),&ws.map.edits.read().unwrap(), [c.0 * 16, 0, c.1 * 16], [c.0 * 16 + 15, 0, c.1 * 16 + 15]).2;
        assert_eq!(read(&ws, picked[0]), 1);
        ws.map.selected = picked.clone();
        ws.run_edit(crate::workspace::EditTool::DeleteChunks).expect("prunes");
        assert!(picked.iter().all(|&c| read(&ws, c) == 0), "all gone at once");
        assert_eq!(read(&ws, others[0]), 1, "the others stay");
        ws.undo_any().expect("undoes");
        assert!(picked.iter().all(|&c| read(&ws, c) == 1), "one step brings them all back");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn outlines_that_meet_a_box_meet_the_line_it_is_drawn_with() {
        let b = PLayer::Box { x: 0.0, z: 0.0, w: 4.0, h: 4.0 };
        // The outline runs through the middles of the corner chunks, half a chunk in.
        assert!(!b.outline_contains((0.25, 2.0)) && b.outline_contains((0.75, 2.0)));
        assert!(b.contains((0.25, 2.0)));
    }

    #[test]
    fn layers_convert_from_chunk_units() {
        let c = PLayer::Circle {
            x: -5.5,
            z: 0.5,
            r: 8.0,
            a: 0.0,
        };
        assert_eq!(
            c.to_layer(),
            Some(Layer::Circle {
                x: -88.0,
                z: 8.0,
                radius: 128.0
            })
        );
        let mut b = PLayer::Box {
            x: 0.0,
            z: 0.0,
            w: 4.0,
            h: 2.0,
        };
        // Corner dots sit in corner chunks: the box covers the opposite corner chunk (0, 0) and
        // the chunk (6, -2) under the dot.
        b.drag_handle(2, (6.4, -1.2));
        assert_eq!(
            b,
            PLayer::Box {
                x: 0.0,
                z: -2.0,
                w: 7.0,
                h: 3.0
            }
        );
        assert!(
            PLayer::Polygon {
                points: vec![(0.0, 0.0), (1.0, 1.0)]
            }
            .to_layer()
            .is_none()
        );
    }
}
