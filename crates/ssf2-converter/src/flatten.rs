//! A frame's display list, flattened.
//!
//! A source frame places any number of pieces at their depths, and the image pass gives each its
//! own picture and placement. A move drawn from many vector pieces (zelda's neutral special) then
//! comes out as dozens of layers whose pieces change places from frame to frame. Here the pieces
//! are sorted into two kinds, by what each does over the frames it is on:
//!
//! - a piece that keeps its depth and its picture, and whose placement moves as the engine's own
//!   tween does (it holds, or goes in a straight line between two keyframes in place, scale,
//!   turn and opacity), keeps its own layer: the entity writes it as holds and tweens.
//! - everything else is flattened: on each frame, every run of such pieces between two kept ones
//!   is drawn into one picture, in depth order, through each piece's own placement. a run of one
//!   keeps its own picture.
//!
//! The layers are then the kept pieces and the runs between them, in depth order, the same layer
//! for the same piece or run on every frame (a frame without one holds nothing on its layer).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::image_extractor::{AnimFrameImages, ExtractedImage, FrameImageEntry, ImageLocalMatrix, ShapeToBitmapMap};

/// How finely a flattened picture is drawn, in its pixels per the source's: the source's own
/// (drawn finer, the pictures came out a third bigger for no gain at the size the game shows them).
const RES: f64 = 1.0;

/// What a piece's picture is and how it sits in the piece's own coordinates.
pub struct Art<'a> {
    pub shape_to_bitmap: &'a ShapeToBitmapMap,
    pub shape_pivot: &'a BTreeMap<u16, (f64, f64)>,
    pub shape_fill_scale: &'a BTreeMap<u16, (f64, f64)>,
}

impl Art<'_> {
    fn bitmap_of(&self, e: &FrameImageEntry) -> u16 {
        self.shape_to_bitmap.get(&e.shape_id).copied().unwrap_or(e.shape_id)
    }

    /// the piece's picture pixel (u, v) in the world: `world = A·(u, v) + o`
    fn transform(&self, e: &FrameImageEntry) -> [f64; 6] {
        let (px, py) = self.shape_pivot.get(&e.shape_id).copied().unwrap_or((0.0, 0.0));
        let (fx, fy) = self.shape_fill_scale.get(&e.shape_id).copied().unwrap_or((1.0, 1.0));
        let (a, b, c, d) = (e.world_a, e.world_b, e.world_c, e.world_d);
        let (ox, oy) = (px * fx, py * fy);
        [a * fx, b * fx, c * fy, d * fy, e.world_tx + a * ox + c * oy, e.world_ty + b * ox + d * oy]
    }

    /// the piece as the entity places it (`Pose`): its pivot is the piece's own registration
    /// point, where the source turns and scales it from, in its picture's pixels
    fn placement(&self, e: &FrameImageEntry) -> Pose {
        let pivot = self.shape_pivot.get(&e.shape_id).copied().unwrap_or((0.0, 0.0));
        let p = crate::fm_placement::image_placement(
            &crate::fm_placement::WorldPlacement {
                tx: e.world_tx, ty: e.world_ty, sx: e.world_sx, sy: e.world_sy, rotation: e.world_rotation,
                a: e.world_a, b: e.world_b, c: e.world_c, d: e.world_d,
            },
            pivot,
            self.shape_fill_scale.get(&e.shape_id).copied().unwrap_or((1.0, 1.0)),
        );
        [p.x, p.y, p.scale_x, p.scale_y, p.rotation, e.alpha, -pivot.0, -pivot.1]
    }
}

/// An IMAGE symbol's placement: x, y, scale x and y, turn, opacity, and its pivot x and y.
pub type Pose = [f64; 8];

/// where a tween puts a picture's corner part of the way (`t`) from one pose to the next: it turns
/// about its pivot by the eased turn, and the pivot moves in a straight line between where it is
/// on the two poses. the packer expands a tween by the same steps (`fra_pack::tweened_position`),
/// as FrayTools previews it; worked in y-up space, angles counter-clockwise
pub fn tweened_corner(a: &Pose, b: &Pose, t: f64) -> (f64, f64) {
    let lerp = |p: f64, q: f64| p + (q - p) * t;
    let pivot_at = |o: (f64, f64), p: (f64, f64), deg: f64| -> (f64, f64) {
        if deg % 360.0 == 0.0 { return (o.0 + p.0, o.1 + p.1) }
        let len = (p.0 * p.0 + p.1 * p.1).sqrt();
        let ang = p.1.atan2(p.0) + deg.to_radians();
        (o.0 + len * ang.cos(), o.1 + len * ang.sin())
    };
    let (sx, sy, rot) = (lerp(a[2], b[2]), lerp(a[3], b[3]), lerp(a[4], b[4]));
    let pivot = (lerp(a[6] * sx, b[6] * sx), -lerp(a[7] * sy, b[7] * sy));
    let origin = (a[0], -a[1]);
    let moved = if a[4] == rot { origin } else {
        let abs = pivot_at(origin, pivot, -a[4]);
        let len = (pivot.0 * pivot.0 + pivot.1 * pivot.1).sqrt();
        let back = pivot.1.atan2(pivot.0) - std::f64::consts::PI - rot.to_radians();
        (abs.0 + len * back.cos(), abs.1 + len * back.sin())
    };
    let own = |p: &Pose| { let q = pivot_at((p[0], -p[1]), (p[6] * p[2], -p[7] * p[3]), -p[4]); (q.0, -q.1) };
    let (p1, p2) = (own(a), own(b));
    (moved.0 + lerp(p1.0, p2.0) - p1.0, -moved.1 + lerp(p1.1, p2.1) - p1.1)
}

/// whether `mid` is where a linear tween from `a` to `b` puts it at `t` (to the placements' own
/// rounding)
pub fn on_line(a: &Pose, b: &Pose, mid: &Pose, t: f64) -> bool {
    const TOL: [f64; 8] = [0.06, 0.06, 0.006, 0.006, 0.2, 0.02, 0.01, 0.01];
    if !(2..8).all(|i| (a[i] + (b[i] - a[i]) * t - mid[i]).abs() <= TOL[i]) { return false }
    let (x, y) = tweened_corner(a, b, t);
    (x - mid[0]).abs() <= TOL[0] && (y - mid[1]).abs() <= TOL[1]
}

/// how many keyframes a piece's placements take as holds and straight tweens, about whichever
/// pivot fits them better
pub fn keyframes_needed(p: &[Pose]) -> usize { keyframes_at(p).min(keyframes_at(&at_corner(p))) }

/// the same poses with the pivot at the picture's corner: a tween then moves the corner in a
/// straight line (a scale about the registration point fits this way, a turn about it the other)
pub fn at_corner(p: &[Pose]) -> Vec<Pose> { p.iter().map(|q| { let mut q = *q; q[6] = 0.0; q[7] = 0.0; q }).collect() }

/// how many keyframes the poses take as they are (their own pivots)
pub fn keyframes_at(p: &[Pose]) -> usize {
    let n = p.len();
    let (mut i, mut k) = (0usize, 0usize);
    while i < n {
        k += 1;
        if i == n - 1 { break }
        // a hold: the same placement on and on
        let mut j = i;
        while j + 1 < n && on_line(&p[i], &p[i], &p[j + 1], 0.0) { j += 1; }
        if j > i { i = j + 1; continue }
        // a tween: the furthest keyframe every frame on the way lies on the line to
        let mut best = i + 1;
        for end in (i + 2)..n {
            let span = (end - i) as f64;
            if ((i + 1)..end).all(|m| on_line(&p[i], &p[end], &p[m], (m - i) as f64 / span)) { best = end } else { break }
        }
        i = best;
    }
    k
}

/// One piece over the frames it stays on (same depth, same picture): where it is on each.
struct Track { frames: Vec<(u16, usize)> }

fn blank(depth: u16) -> FrameImageEntry {
    FrameImageEntry {
        depth, shape_id: u16::MAX, symbol_name: String::new(), alpha: 1.0,
        local_matrix: ImageLocalMatrix::from_abcd(1.0, 0.0, 0.0, 1.0, 0.0, 0.0),
        world_tx: 0.0, world_ty: 0.0, world_sx: 1.0, world_sy: 1.0, world_rotation: 0.0,
        world_a: 1.0, world_b: 0.0, world_c: 0.0, world_d: 1.0, anim_origin_x: 0.0, anim_origin_y: 0.0,
        look: Default::default(),
    }
}

fn is_blank(e: &FrameImageEntry) -> bool { e.shape_id == u16::MAX && e.symbol_name.is_empty() }

/// a stable hash, for the flattened pictures' names (the same run, the same name, every build)
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// Flattens every animation's frames; returns how many pictures it drew.
pub fn flatten(
    anim_images: &mut BTreeMap<String, AnimFrameImages>,
    images: &mut BTreeMap<u16, ExtractedImage>,
    art: &Art,
    char_dir: &Path,
    char_name: &str,
) -> anyhow::Result<usize> {
    let sprites = char_dir.join("library/sprites");
    std::fs::create_dir_all(&sprites)?;
    let mut next_id: u16 = images.keys().copied().filter(|k| *k != u16::MAX).max().unwrap_or(0).max(59_999) + 1;
    let mut drawn: HashMap<String, (u16, String, f64, f64)> = HashMap::new();
    let mut decoded: HashMap<u16, Option<image::RgbaImage>> = HashMap::new();
    let mut count = 0usize;
    let styled = anim_images.values().flat_map(|a| a.frames.values().flatten()).filter(|e| !e.look.is_plain()).count();
    if styled > 0 { log::info!("{styled} placement(s) drawn with their colour, blend or filters"); }

    for anim in anim_images.values_mut() {
        let total = anim.frames.keys().max().map(|m| *m as u32 + 1).unwrap_or(0) as u16;
        if total == 0 { continue }
        // the pieces, followed from frame to frame by depth and picture
        let mut tracks: Vec<Track> = Vec::new();
        let mut open: HashMap<(u16, u16), usize> = HashMap::new();
        let mut track_of: BTreeMap<(u16, usize), usize> = BTreeMap::new();
        for f in 0..total {
            let mut still: HashMap<(u16, u16), usize> = HashMap::new();
            if let Some(entries) = anim.frames.get(&f) {
                for (i, e) in entries.iter().enumerate() {
                    let key = (e.depth, e.shape_id);
                    let t = match open.get(&key) {
                        Some(t) => *t,
                        None => { tracks.push(Track { frames: Vec::new() }); tracks.len() - 1 }
                    };
                    tracks[t].frames.push((f, i));
                    track_of.insert((f, i), t);
                    still.insert(key, t);
                }
            }
            open = still;
        }
        // which keep their own layer: on two frames or more, and moving as tweens do
        let mut kept: Vec<bool> = tracks.iter().map(|t| {
            if t.frames.len() < 2 { return false }
            // a colour, a blend or a filter is drawn into the picture: no tween carries it
            if t.frames.iter().any(|(f, i)| !anim.frames[f][*i].look.is_plain()) { return false }
            let p: Vec<Pose> = t.frames.iter().map(|(f, i)| art.placement(&anim.frames[f][*i])).collect();
            keyframes_needed(&p) * 2 <= p.len()
        }).collect();
        // each frame as its units, bottom to top: runs of the rest and kept pieces in turn (an empty
        // run where two kept pieces meet). the layers are the units' places: the first run, the
        // first kept piece, the second run and so on, so a frame takes as many layers as it has
        // units and a layer holds whatever is at its place. a kept piece holds its place all the
        // way (its tweens are on one layer): one that does not is flattened with the rest, until
        // every kept piece does
        let units = |kept: &[bool]| -> BTreeMap<u16, Vec<Vec<usize>>> {
            let mut out = BTreeMap::new();
            for f in 0..total {
                let Some(entries) = anim.frames.get(&f) else { continue };
                let mut list: Vec<Vec<usize>> = vec![Vec::new()];
                for (i, e) in entries.iter().enumerate() {
                    if is_blank(e) { continue }
                    if kept[track_of[&(f, i)]] {
                        list.push(vec![i]);
                        list.push(Vec::new());
                    } else {
                        list.last_mut().unwrap().push(i);
                    }
                }
                out.insert(f, list);
            }
            out
        };
        let mut list = units(&kept);
        loop {
            let mut place: HashMap<usize, usize> = HashMap::new();
            let mut moved: Vec<usize> = Vec::new();
            for (f, l) in &list {
                for (slot, run) in l.iter().enumerate() {
                    if slot % 2 == 0 { continue }
                    let t = track_of[&(*f, run[0])];
                    if *place.entry(t).or_insert(slot) != slot { moved.push(t); }
                }
            }
            if moved.is_empty() { break }
            for t in moved { kept[t] = false; }
            list = units(&kept);
        }
        let lanes = list.values().map(Vec::len).max().unwrap_or(1);
        let mut frames: BTreeMap<u16, Vec<FrameImageEntry>> = BTreeMap::new();
        for (f, units) in &list {
            let entries = &anim.frames[f];
            let base = entries.first().map(|e| e.depth).unwrap_or(0);
            let mut row: Vec<FrameImageEntry> = (0..lanes).map(|_| blank(base)).collect();
            for (slot, run) in units.iter().enumerate() {
                if run.is_empty() { continue }
                if run.len() == 1 && entries[run[0]].look.is_plain() { row[slot] = entries[run[0]].clone(); continue }
                // a run of pieces: one picture, drawn once for every frame that places them alike
                let pieces: Vec<&FrameImageEntry> = run.iter().map(|i| &entries[*i]).collect();
                // keyed by where its pieces are from the first one's whole pixel: a run that only
                // slides by whole pixels is the same picture, placed that much further on
                let first = art.transform(pieces[0]);
                let (ox, oy) = (first[4].floor(), first[5].floor());
                let key: String = pieces.iter().map(|e| {
                    let mut m = art.transform(e);
                    m[4] -= ox; m[5] -= oy;
                    format!("{}:{}:{}:{:?}|", art.bitmap_of(e), m.iter().map(|v| format!("{:x}", v.to_bits())).collect::<Vec<_>>().join(","), e.alpha.to_bits(), e.look)
                }).collect();
                let hit = match drawn.get(&key) {
                    Some(h) => Some(h.clone()),
                    None => {
                        let h = draw_run(&pieces, art, images, &mut decoded, char_dir, &sprites, char_name, &key, next_id)?;
                        if let Some((id, sym, w, hgt, ..)) = &h {
                            images.insert(*id, ExtractedImage {
                                bitmap_id: *id, symbol_name: sym.clone(), width: *w, height: *hgt,
                                png_path: format!("library/sprites/{sym}.png"), is_vector: true,
                            });
                            next_id = next_id.checked_add(1).ok_or_else(|| anyhow::anyhow!("{char_name}: more flattened pictures than picture ids"))?;
                            count += 1;
                        }
                        let h = h.map(|(id, sym, _, _, x, y)| (id, sym, x - ox, y - oy));
                        if let Some(h) = &h { drawn.insert(key.clone(), h.clone()); }
                        h
                    }
                };
                let Some((id, sym, x, y)) = hit else { row[slot] = pieces[0].clone(); continue };
                let (x, y) = (x + ox, y + oy);
                let k = 1.0 / RES;
                let mut e = pieces[0].clone();
                e.shape_id = id;
                e.symbol_name = sym;
                e.alpha = 1.0;
                e.world_tx = x; e.world_ty = y;
                e.world_a = k; e.world_b = 0.0; e.world_c = 0.0; e.world_d = k;
                e.world_sx = k; e.world_sy = k; e.world_rotation = 0.0;
                e.local_matrix = ImageLocalMatrix::from_abcd(k, 0.0, 0.0, k, x, y);
                row[slot] = e;
            }
            frames.insert(*f, row);
        }
        // a place empty on every frame (a run between two kept pieces that never has anything in
        // it) takes no layer
        let used: Vec<usize> = (0..lanes).filter(|l| frames.values().any(|r| !is_blank(&r[*l]))).collect();
        for r in frames.values_mut() {
            let keep: Vec<FrameImageEntry> = used.iter().map(|l| r[*l].clone()).collect();
            *r = keep;
        }
        anim.max_depth_slots = used.len().max(1);
        anim.frames = frames;
    }
    Ok(count)
}

/// Draws a run of pieces into one picture: its id, name, size, and where its corner is in the
/// world. None when nothing of it can be drawn.
#[allow(clippy::too_many_arguments)]
fn draw_run(
    pieces: &[&FrameImageEntry],
    art: &Art,
    images: &BTreeMap<u16, ExtractedImage>,
    decoded: &mut HashMap<u16, Option<image::RgbaImage>>,
    char_dir: &Path,
    sprites: &Path,
    char_name: &str,
    key: &str,
    id: u16,
) -> anyhow::Result<Option<(u16, String, u32, u32, f64, f64)>> {
    // each piece's picture as it looks placed: coloured, filtered, and where its corner went
    let mut placed: Vec<([f64; 6], tiny_skia::Pixmap, f32, tiny_skia::BlendMode)> = Vec::new();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for e in pieces {
        let bmp = art.bitmap_of(e);
        let Some(img) = images.get(&bmp) else { continue };
        let rgba = decoded.entry(bmp).or_insert_with(|| image::open(char_dir.join(&img.png_path)).ok().map(|i| i.to_rgba8()));
        let Some(rgba) = rgba else { continue };
        let mut m = art.transform(e);
        let mut rgba = rgba.clone();
        if let Some(t) = e.look.tint {
            // the source's rule: scale, then offset, then clamp
            for px in rgba.pixels_mut() {
                for c in 0..3 { px[c] = (px[c] as f64 * t[c] + t[3 + c]).clamp(0.0, 255.0) as u8; }
            }
        }
        if !e.look.filters.is_empty() {
            let (sx, sy) = ((m[0] * m[0] + m[1] * m[1]).sqrt(), (m[2] * m[2] + m[3] * m[3]).sqrt());
            let (out, dx, dy) = crate::swf_filter::apply(&rgba, &e.look.filters, sx, sy);
            // the filter grew the picture: its corner moved by (dx, dy) of its own pixels
            m[4] += m[0] * dx as f64 + m[2] * dy as f64;
            m[5] += m[1] * dx as f64 + m[3] * dy as f64;
            rgba = out;
        }
        let (w, h) = rgba.dimensions();
        let mut data = rgba.into_raw();
        for px in data.chunks_exact_mut(4) {
            let a = px[3] as u32;
            for c in &mut px[..3] { *c = ((*c as u32 * a + 127) / 255) as u8; }
        }
        let Some(pix) = tiny_skia::IntSize::from_wh(w, h).and_then(|s| tiny_skia::Pixmap::from_vec(data, s)) else { continue };
        let (w, h) = (w as f64, h as f64);
        for (u, v) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
            let (x, y) = (m[0] * u + m[2] * v + m[4], m[1] * u + m[3] * v + m[5]);
            x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y);
        }
        placed.push((m, pix, e.alpha as f32, blend_of(e.look.blend)));
    }
    if placed.is_empty() { return Ok(None) }
    let (x0, y0) = (x0.floor(), y0.floor());
    let (w, h) = (((x1 - x0) * RES).ceil().max(1.0), ((y1 - y0) * RES).ceil().max(1.0));
    if w > 4096.0 || h > 4096.0 { return Ok(None) }
    let Some(mut canvas) = tiny_skia::Pixmap::new(w as u32, h as u32) else { return Ok(None) };
    for (m, pix, alpha, blend) in &placed {
        let r = RES as f32;
        let t = tiny_skia::Transform::from_row(
            m[0] as f32 * r, m[1] as f32 * r, m[2] as f32 * r, m[3] as f32 * r,
            ((m[4] - x0) * RES) as f32, ((m[5] - y0) * RES) as f32);
        let paint = tiny_skia::PixmapPaint { opacity: alpha.clamp(0.0, 1.0), blend_mode: *blend, quality: tiny_skia::FilterQuality::Bicubic };
        canvas.draw_pixmap(0, 0, pix.as_ref(), &paint, t, None);
    }
    // back to straight alpha for the picture on disk
    let mut data = canvas.take();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 { for c in &mut px[..3] { *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8; } }
    }
    let sym = format!("{}_flat_{:016x}", crate::image_extractor::sanitize_name(char_name), fnv(key));
    image::save_buffer(sprites.join(format!("{sym}.png")), &data, w as u32, h as u32, image::ExtendedColorType::Rgba8)?;
    Ok(Some((id, sym, w as u32, h as u32, x0, y0)))
}

/// a placement's blend mode as the drawing's: what it mixes with is what is under it in its run
fn blend_of(b: Option<swf::BlendMode>) -> tiny_skia::BlendMode {
    use tiny_skia::BlendMode as T;
    match b {
        Some(swf::BlendMode::Multiply) => T::Multiply,
        Some(swf::BlendMode::Screen) => T::Screen,
        Some(swf::BlendMode::Lighten) => T::Lighten,
        Some(swf::BlendMode::Darken) => T::Darken,
        Some(swf::BlendMode::Difference) => T::Difference,
        Some(swf::BlendMode::Add) => T::Plus,
        Some(swf::BlendMode::Overlay) => T::Overlay,
        Some(swf::BlendMode::HardLight) => T::HardLight,
        Some(swf::BlendMode::Alpha) => T::DestinationIn,
        Some(swf::BlendMode::Erase) => T::DestinationOut,
        _ => T::SourceOver,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, r: f64) -> Pose { [x, y, 1.0, 1.0, r, 1.0, 0.0, 0.0] }

    #[test]
    fn a_hold_and_a_straight_move_take_a_keyframe_each() {
        let mut t = vec![p(0.0, 0.0, 0.0); 4];
        t.extend((0..=6).map(|i| p(i as f64 * 3.0, 0.0, i as f64 * 10.0)));
        assert_eq!(keyframes_needed(&t), 3);
    }

    #[test]
    fn a_turn_about_its_registration_point_is_one_tween() {
        // a 20x10 picture whose registration point is its centre, turned 90 degrees about it in
        // four steps: its corner swings round the centre
        let pose = |deg: f64| -> Pose {
            let r = deg.to_radians();
            let (cx, cy) = (100.0, 50.0);
            // corner = centre + R(deg)·(-10, -5), y-down, clockwise-positive
            let (dx, dy) = (-10.0, -5.0);
            [cx + r.cos() * dx - r.sin() * dy, cy + r.sin() * dx + r.cos() * dy, 1.0, 1.0, deg, 1.0, 10.0, 5.0]
        };
        let t: Vec<Pose> = (0..=4).map(|i| pose(i as f64 * 22.5)).collect();
        assert_eq!(keyframes_needed(&t), 2, "one tween and the keyframe it reaches");
    }

    #[test]
    fn the_tween_turns_about_the_pivot_as_the_packer_does() {
        // the packer's own case (fra_pack: a_turning_image_keeps_its_pivot_where_it_is)
        let a: Pose = [0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 10.0, 0.0];
        let b: Pose = [10.0, -10.0, 1.0, 1.0, 90.0, 1.0, 10.0, 0.0];
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let (x, y) = tweened_corner(&a, &b, t);
            assert!((((x - 10.0).powi(2) + y.powi(2)).sqrt() - 10.0).abs() < 1e-9, "at {t}: ({x}, {y})");
        }
        assert_eq!(tweened_corner(&a, &b, 1.0), (10.0, -10.0));
    }

    #[test]
    fn jitter_takes_a_keyframe_a_frame() {
        let t: Vec<Pose> = (0..8).map(|i| p(if i % 2 == 0 { 0.0 } else { 5.0 }, i as f64, 0.0)).collect();
        assert!(keyframes_needed(&t) * 2 > t.len());
    }
}
