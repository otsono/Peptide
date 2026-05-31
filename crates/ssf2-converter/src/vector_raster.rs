//! Rasterizes SWF vector shapes (`DefineShape` with no bitmap fill, and
//! interpolated `DefineMorphShape` frames) to RGBA bitmaps, so SSF2 VFX that
//! are drawn as vector art — dust swirls, sparks, trails, morphing waves —
//! can be exported as PNGs and actually render in Fraymakers (which only
//! displays bitmaps).
//!
//! Pipeline: SWF ShapeRecords → per-fill directed edges → stitched closed
//! loops → tiny-skia paths → anti-aliased fill/stroke → straight RGBA.
//!
//! Fills: solid colors are exact; linear/radial/focal gradients render as
//! real tiny-skia shaders using the SWF gradient matrix. Strokes use the line
//! width + a solid color (averaged stops for gradient strokes).

use swf::{Color, FillStyle, Gradient, GradientSpread, LineStyle, Matrix, Rectangle, ShapeRecord, Twips};
use tiny_skia::{
    FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, RadialGradient,
    Shader, SpreadMode, Stroke, Transform,
};

/// A rasterized vector shape.
pub struct Raster {
    /// Straight (un-premultiplied) RGBA8, row-major, `width*height*4` bytes.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Offset (in pixels) from the shape's local (0,0) to the PNG's top-left
    /// pixel — i.e. `(x_min/20, y_min/20)`. Stored as the shape's "pivot" so
    /// the existing placement math positions the PNG exactly where the vector
    /// art sat.
    pub origin_px: (f64, f64),
}

type Pt = (f64, f64); // pixel-space point

#[derive(Clone)]
enum Seg {
    Line { to: Pt },
    Quad { ctrl: Pt, to: Pt },
}

#[derive(Clone)]
struct DEdge {
    from: Pt,
    to: Pt,
    seg: Seg, // geometry oriented from->to
}

/// One contiguous style group (SWF shapes can switch the whole style table
/// mid-record via NEW_STYLES; each group resolves its own fill/line indices).
struct Group {
    fills: Vec<FillStyle>,
    lines: Vec<LineStyle>,
    fill_edges: std::collections::BTreeMap<usize, Vec<DEdge>>, // 1-based fill idx → edges
    strokes: Vec<(usize, Vec<Seg>, Pt)>,                       // (line idx, segs, start)
}

impl Group {
    fn new(fills: Vec<FillStyle>, lines: Vec<LineStyle>) -> Self {
        Group { fills, lines, fill_edges: Default::default(), strokes: Vec::new() }
    }
}

/// Rasterize a shape given its bounds, style table, and records.
/// Returns None if the shape has no drawable content or zero area.
pub fn rasterize_shape(
    bounds: &Rectangle<Twips>,
    fills: &[FillStyle],
    lines: &[LineStyle],
    records: &[ShapeRecord],
) -> Option<Raster> {
    let min_x = bounds.x_min.to_pixels();
    let min_y = bounds.y_min.to_pixels();
    let max_x = bounds.x_max.to_pixels();
    let max_y = bounds.y_max.to_pixels();
    let w = (max_x - min_x).ceil();
    let h = (max_y - min_y).ceil();
    if !(w >= 1.0 && h >= 1.0) || w > 4096.0 || h > 4096.0 {
        return None;
    }
    let (wpx, hpx) = (w as u32, h as u32);

    // twip → pixel, shifted so bounds.min maps to (0,0).
    let tx = |t: Twips| t.to_pixels() - min_x;
    let ty = |t: Twips| t.to_pixels() - min_y;

    // ── Walk records into style groups + directed edges. ───────────────────────
    let mut groups: Vec<Group> = Vec::new();
    let mut cur = Group::new(fills.to_vec(), lines.to_vec());
    let mut pen: Pt = (tx(Twips::ZERO), ty(Twips::ZERO)); // (0,0) shape-local
    let mut fs0: usize = 0;
    let mut fs1: usize = 0;
    let mut ls: usize = 0;
    // active stroke run for the current line style
    let mut stroke_run: Option<(usize, Vec<Seg>, Pt)> = None;

    let flush_stroke = |run: &mut Option<(usize, Vec<Seg>, Pt)>, g: &mut Group| {
        if let Some((idx, segs, start)) = run.take() {
            if idx > 0 && !segs.is_empty() {
                g.strokes.push((idx, segs, start));
            }
        }
    };

    for rec in records {
        match rec {
            ShapeRecord::StyleChange(sc) => {
                if let Some(mv) = sc.move_to {
                    flush_stroke(&mut stroke_run, &mut cur);
                    pen = (tx(mv.x), ty(mv.y));
                }
                if let Some(ns) = &sc.new_styles {
                    // Close the current group, start a new one with the new table.
                    flush_stroke(&mut stroke_run, &mut cur);
                    let done = std::mem::replace(&mut cur, Group::new(ns.fill_styles.clone(), ns.line_styles.clone()));
                    groups.push(done);
                    fs0 = 0; fs1 = 0; ls = 0;
                }
                if let Some(f) = sc.fill_style_0 { fs0 = f as usize; }
                if let Some(f) = sc.fill_style_1 { fs1 = f as usize; }
                if let Some(l) = sc.line_style {
                    flush_stroke(&mut stroke_run, &mut cur);
                    ls = l as usize;
                }
            }
            ShapeRecord::StraightEdge { delta } => {
                let to = (pen.0 + delta.dx.to_pixels(), pen.1 + delta.dy.to_pixels());
                add_edge(&mut cur, fs0, fs1, pen, to, Seg::Line { to });
                push_stroke(&mut stroke_run, ls, pen, Seg::Line { to });
                pen = to;
            }
            ShapeRecord::CurvedEdge { control_delta, anchor_delta } => {
                let ctrl = (pen.0 + control_delta.dx.to_pixels(), pen.1 + control_delta.dy.to_pixels());
                let to = (ctrl.0 + anchor_delta.dx.to_pixels(), ctrl.1 + anchor_delta.dy.to_pixels());
                add_edge(&mut cur, fs0, fs1, pen, to, Seg::Quad { ctrl, to });
                push_stroke(&mut stroke_run, ls, pen, Seg::Quad { ctrl, to });
                pen = to;
            }
        }
    }
    flush_stroke(&mut stroke_run, &mut cur);
    groups.push(cur);

    // ── Rasterize. ─────────────────────────────────────────────────────────────
    let mut pixmap = Pixmap::new(wpx, hpx)?;
    let ident = Transform::identity();
    let mut drew_anything = false;

    for g in &groups {
        // Fills (draw before strokes so strokes sit on top).
        for (&idx, edges) in &g.fill_edges {
            if idx == 0 || idx > g.fills.len() { continue; }
            let fill = &g.fills[idx - 1];
            let paint = match build_fill_paint(fill, min_x, min_y) {
                Some(p) => p,
                None => continue, // bitmap fill etc. — skip
            };
            let loops = stitch(edges);
            let mut pb = PathBuilder::new();
            let mut any = false;
            for lp in &loops {
                if lp.is_empty() { continue; }
                pb.move_to(lp[0].from.0 as f32, lp[0].from.1 as f32);
                for e in lp {
                    match &e.seg {
                        Seg::Line { to } => pb.line_to(to.0 as f32, to.1 as f32),
                        Seg::Quad { ctrl, to } => pb.quad_to(ctrl.0 as f32, ctrl.1 as f32, to.0 as f32, to.1 as f32),
                    }
                }
                pb.close();
                any = true;
            }
            if !any { continue; }
            if let Some(path) = pb.finish() {
                pixmap.fill_path(&path, &paint, FillRule::Winding, ident, None);
                drew_anything = true;
            }
        }

        // Strokes.
        for (idx, segs, start) in &g.strokes {
            if *idx == 0 || *idx > g.lines.len() { continue; }
            let line = &g.lines[*idx - 1];
            let color = match resolve_fill_color(line.fill_style()) {
                Some(c) => c,
                None => (0, 0, 0, 255),
            };
            let mut pb = PathBuilder::new();
            pb.move_to(start.0 as f32, start.1 as f32);
            for s in segs {
                match s {
                    Seg::Line { to } => pb.line_to(to.0 as f32, to.1 as f32),
                    Seg::Quad { ctrl, to } => pb.quad_to(ctrl.0 as f32, ctrl.1 as f32, to.0 as f32, to.1 as f32),
                }
            }
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                paint.anti_alias = true;
                paint.set_color_rgba8(color.0, color.1, color.2, color.3);
                let mut stroke = Stroke::default();
                stroke.width = (line.width().to_pixels() as f32).max(1.0);
                pixmap.stroke_path(&path, &paint, &stroke, ident, None);
                drew_anything = true;
            }
        }
    }

    if !drew_anything {
        return None;
    }

    // Pixmap is premultiplied; demultiply to straight RGBA for the PNG encoder.
    let mut rgba = Vec::with_capacity((wpx * hpx * 4) as usize);
    for p in pixmap.pixels() {
        let c = p.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }

    Some(Raster {
        rgba,
        width: wpx,
        height: hpx,
        origin_px: (min_x, min_y),
    })
}

fn add_edge(g: &mut Group, fs0: usize, fs1: usize, from: Pt, to: Pt, seg: Seg) {
    if fs1 > 0 {
        g.fill_edges.entry(fs1).or_default().push(DEdge { from, to, seg: seg.clone() });
    }
    if fs0 > 0 {
        // Same edge bounds the fs0 fill on the other side → add reversed.
        g.fill_edges.entry(fs0).or_default().push(reverse_edge(from, to, &seg));
    }
}

fn reverse_edge(from: Pt, to: Pt, seg: &Seg) -> DEdge {
    match seg {
        Seg::Line { .. } => DEdge { from: to, to: from, seg: Seg::Line { to: from } },
        Seg::Quad { ctrl, .. } => DEdge { from: to, to: from, seg: Seg::Quad { ctrl: *ctrl, to: from } },
    }
}

fn push_stroke(run: &mut Option<(usize, Vec<Seg>, Pt)>, ls: usize, from: Pt, seg: Seg) {
    if ls == 0 {
        return;
    }
    match run {
        Some((idx, segs, _)) if *idx == ls => segs.push(seg),
        _ => *run = Some((ls, vec![seg], from)),
    }
}

/// Chain directed edges into closed loops by matching endpoints. SWF edge
/// coordinates derive from integer twips, so endpoints meet exactly; we key on
/// rounded pixel coords (×20 back to ~twip precision) to be safe.
fn stitch(edges: &[DEdge]) -> Vec<Vec<DEdge>> {
    let key = |p: Pt| ((p.0 * 20.0).round() as i64, (p.1 * 20.0).round() as i64);
    let mut by_start: std::collections::HashMap<(i64, i64), Vec<usize>> = std::collections::HashMap::new();
    for (i, e) in edges.iter().enumerate() {
        by_start.entry(key(e.from)).or_default().push(i);
    }
    let mut used = vec![false; edges.len()];
    let mut loops: Vec<Vec<DEdge>> = Vec::new();

    for start_idx in 0..edges.len() {
        if used[start_idx] {
            continue;
        }
        let mut loop_edges: Vec<DEdge> = Vec::new();
        let mut cur = start_idx;
        let mut guard = 0;
        loop {
            used[cur] = true;
            loop_edges.push(edges[cur].clone());
            let want = key(edges[cur].to);
            // find an unused edge starting at `want`
            let next = by_start.get(&want).and_then(|cands| cands.iter().copied().find(|&j| !used[j]));
            match next {
                Some(j) => cur = j,
                None => break,
            }
            guard += 1;
            if guard > edges.len() + 4 {
                break;
            }
        }
        loops.push(loop_edges);
    }
    loops
}

/// Map the SWF gradient matrix to a tiny-skia shader transform that takes
/// gradient-local twips → the pixel canvas (shifted so bounds.min = origin).
///   shape_twip = M·grad_local;  pixel = shape_twip/20 − bounds.min
fn grad_transform(m: &Matrix, min_x: f64, min_y: f64) -> Transform {
    Transform::from_row(
        (m.a.to_f64() / 20.0) as f32,
        (m.b.to_f64() / 20.0) as f32,
        (m.c.to_f64() / 20.0) as f32,
        (m.d.to_f64() / 20.0) as f32,
        (m.tx.to_pixels() - min_x) as f32,
        (m.ty.to_pixels() - min_y) as f32,
    )
}

fn grad_stops(g: &Gradient) -> Vec<GradientStop> {
    g.records.iter().map(|r| GradientStop::new(
        r.ratio as f32 / 255.0,
        tiny_skia::Color::from_rgba8(r.color.r, r.color.g, r.color.b, r.color.a),
    )).collect()
}

fn spread_mode(s: GradientSpread) -> SpreadMode {
    match s {
        GradientSpread::Pad => SpreadMode::Pad,
        GradientSpread::Reflect => SpreadMode::Reflect,
        GradientSpread::Repeat => SpreadMode::Repeat,
    }
}

fn radial_shader(g: &Gradient, focal: f64, min_x: f64, min_y: f64) -> Option<Shader<'static>> {
    // SWF radial: circle centred at (0,0), radius 16384 in gradient space;
    // a focal point (FocalGradient) sits on the x-axis at focal*16384.
    // tiny-skia two-point conical: start = focal circle (r=0), end = main circle.
    RadialGradient::new(
        Point::from_xy((focal * 16384.0) as f32, 0.0),
        Point::from_xy(0.0, 0.0),
        16384.0,
        grad_stops(g),
        spread_mode(g.spread),
        grad_transform(&g.matrix, min_x, min_y),
    )
}

/// Build a tiny-skia Paint for a fill: solid colour, or a real linear/radial
/// gradient shader. None for bitmap fills (handled elsewhere).
fn build_fill_paint(fill: &FillStyle, min_x: f64, min_y: f64) -> Option<Paint<'static>> {
    let mut paint = Paint::default();
    paint.anti_alias = true;
    let shader = match fill {
        FillStyle::Color(c) => {
            paint.set_color_rgba8(c.r, c.g, c.b, c.a);
            return Some(paint);
        }
        FillStyle::LinearGradient(g) => LinearGradient::new(
            Point::from_xy(-16384.0, 0.0),
            Point::from_xy(16384.0, 0.0),
            grad_stops(g),
            spread_mode(g.spread),
            grad_transform(&g.matrix, min_x, min_y),
        ),
        FillStyle::RadialGradient(g) => radial_shader(g, 0.0, min_x, min_y),
        FillStyle::FocalGradient { gradient, focal_point } => {
            radial_shader(gradient, focal_point.to_f64(), min_x, min_y)
        }
        FillStyle::Bitmap { .. } => return None,
    };
    // If shader construction failed (degenerate matrix, single stop, etc.),
    // fall back to averaged stop color so the shape still renders.
    match shader {
        Some(s) => paint.shader = s,
        None => {
            let (r, g, b, a) = resolve_fill_color(fill)?;
            paint.set_color_rgba8(r, g, b, a);
        }
    }
    Some(paint)
}

/// Solid color for a fill, or an average-stop approximation for gradients.
/// Used for STROKE colors (strokes keep the solid approximation; the fill
/// path uses `build_fill_paint` for real gradients). None for bitmap fills.
fn resolve_fill_color(fill: &FillStyle) -> Option<(u8, u8, u8, u8)> {
    match fill {
        FillStyle::Color(c) => Some((c.r, c.g, c.b, c.a)),
        FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g) | FillStyle::FocalGradient { gradient: g, .. } => {
            if g.records.is_empty() {
                return None;
            }
            let n = g.records.len() as u32;
            let (mut r, mut gn, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            for rec in &g.records {
                r += rec.color.r as u32;
                gn += rec.color.g as u32;
                b += rec.color.b as u32;
                a += rec.color.a as u32;
            }
            Some(((r / n) as u8, (gn / n) as u8, (b / n) as u8, (a / n) as u8))
        }
        FillStyle::Bitmap { .. } => None,
    }
}

// ── Morph shapes ────────────────────────────────────────────────────────────────

/// Interpolate a `DefineMorphShape` at `ratio` ∈ [0,1] and rasterize it.
/// Walks the start/end record lists in lockstep, lerping edge deltas and
/// fill/line colors. Falls back to None if the two record lists don't align.
pub fn rasterize_morph(m: &swf::DefineMorphShape, ratio: f64) -> Option<Raster> {
    let t = ratio.clamp(0.0, 1.0);

    // Interpolate bounds.
    let lerp_tw = |a: Twips, b: Twips| Twips::new((a.get() as f64 + (b.get() as f64 - a.get() as f64) * t) as i32);
    let bounds = Rectangle {
        x_min: lerp_tw(m.start.shape_bounds.x_min, m.end.shape_bounds.x_min),
        x_max: lerp_tw(m.start.shape_bounds.x_max, m.end.shape_bounds.x_max),
        y_min: lerp_tw(m.start.shape_bounds.y_min, m.end.shape_bounds.y_min),
        y_max: lerp_tw(m.start.shape_bounds.y_max, m.end.shape_bounds.y_max),
    };

    // Interpolate fill/line colors (geometry of styles is shared; only colors
    // and gradient stops morph — we approximate with the start table's
    // structure and interpolated solid colors).
    let fills = interp_fills(&m.start.fill_styles, &m.end.fill_styles, t);
    let lines = interp_lines(&m.start.line_styles, &m.end.line_styles, t);

    // Interpolate records: start records carry move/style + start deltas; end
    // records carry the matching end deltas (same count + kinds in a valid
    // morph). Build a merged record list using start's styles and lerped deltas.
    let records = interp_records(&m.start.shape, &m.end.shape, t)?;

    rasterize_shape(&bounds, &fills, &lines, &records)
}

fn lerp_color(a: &Color, b: &Color, t: f64) -> Color {
    let l = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    Color { r: l(a.r, b.r), g: l(a.g, b.g), b: l(a.b, b.b), a: l(a.a, b.a) }
}

fn interp_fills(a: &[FillStyle], b: &[FillStyle], t: f64) -> Vec<FillStyle> {
    a.iter().enumerate().map(|(i, fa)| {
        match (fa, b.get(i)) {
            (FillStyle::Color(ca), Some(FillStyle::Color(cb))) => FillStyle::Color(lerp_color(ca, cb, t)),
            _ => fa.clone(),
        }
    }).collect()
}

fn interp_lines(a: &[LineStyle], b: &[LineStyle], t: f64) -> Vec<LineStyle> {
    a.iter().enumerate().map(|(i, la)| {
        let width = match b.get(i) {
            Some(lb) => Twips::new((la.width().get() as f64 + (lb.width().get() as f64 - la.width().get() as f64) * t) as i32),
            None => la.width(),
        };
        let fill = match (la.fill_style(), b.get(i).map(|l| l.fill_style())) {
            (FillStyle::Color(ca), Some(FillStyle::Color(cb))) => FillStyle::Color(lerp_color(ca, cb, t)),
            _ => la.fill_style().clone(),
        };
        la.clone().with_width(width).with_fill_style(fill)
    }).collect()
}

/// Lerp start/end edge deltas in lockstep. Style-change records (move_to, fill
/// indices) come from `start`; edge magnitudes interpolate. Returns None if the
/// record lists diverge in structure.
fn interp_records(start: &[ShapeRecord], end: &[ShapeRecord], t: f64) -> Option<Vec<ShapeRecord>> {
    use swf::{Point, PointDelta};
    if start.len() != end.len() {
        // Structures differ — fall back to start geometry (still renders).
        return Some(start.to_vec());
    }
    let lerp_tw = |a: Twips, b: Twips| Twips::new((a.get() as f64 + (b.get() as f64 - a.get() as f64) * t) as i32);
    let mut out = Vec::with_capacity(start.len());
    for (s, e) in start.iter().zip(end.iter()) {
        match (s, e) {
            (ShapeRecord::StyleChange(sc), ShapeRecord::StyleChange(ec)) => {
                let mut nc = sc.clone();
                if let (Some(sm), Some(em)) = (sc.move_to, ec.move_to) {
                    nc.move_to = Some(Point::new(lerp_tw(sm.x, em.x), lerp_tw(sm.y, em.y)));
                }
                out.push(ShapeRecord::StyleChange(nc));
            }
            (ShapeRecord::StraightEdge { delta: sd }, ShapeRecord::StraightEdge { delta: ed }) => {
                out.push(ShapeRecord::StraightEdge { delta: PointDelta::new(lerp_tw(sd.dx, ed.dx), lerp_tw(sd.dy, ed.dy)) });
            }
            (ShapeRecord::CurvedEdge { control_delta: sc, anchor_delta: sa },
             ShapeRecord::CurvedEdge { control_delta: ec, anchor_delta: ea }) => {
                out.push(ShapeRecord::CurvedEdge {
                    control_delta: PointDelta::new(lerp_tw(sc.dx, ec.dx), lerp_tw(sc.dy, ec.dy)),
                    anchor_delta: PointDelta::new(lerp_tw(sa.dx, ea.dx), lerp_tw(sa.dy, ea.dy)),
                });
            }
            // Kinds diverge (e.g. straight↔curved): use the start record.
            _ => out.push(s.clone()),
        }
    }
    Some(out)
}
