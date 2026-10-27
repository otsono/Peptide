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

    /// the piece as the entity places it: x, y, scale x and y, turn, opacity
    fn placement(&self, e: &FrameImageEntry) -> [f64; 6] {
        let p = crate::fm_placement::image_placement(
            &crate::fm_placement::WorldPlacement {
                tx: e.world_tx, ty: e.world_ty, sx: e.world_sx, sy: e.world_sy, rotation: e.world_rotation,
                a: e.world_a, b: e.world_b, c: e.world_c, d: e.world_d,
            },
            self.shape_pivot.get(&e.shape_id).copied().unwrap_or((0.0, 0.0)),
            self.shape_fill_scale.get(&e.shape_id).copied().unwrap_or((1.0, 1.0)),
        );
        [p.x, p.y, p.scale_x, p.scale_y, p.rotation, e.alpha]
    }
}

/// whether `mid` is where a straight tween from `a` to `b` puts it at `t` (to the placements'
/// own rounding)
pub fn on_line(a: &[f64; 6], b: &[f64; 6], mid: &[f64; 6], t: f64) -> bool {
    const TOL: [f64; 6] = [0.06, 0.06, 0.006, 0.006, 0.2, 0.02];
    (0..6).all(|i| (a[i] + (b[i] - a[i]) * t - mid[i]).abs() <= TOL[i])
}

/// how many keyframes a piece's placements take as holds and straight tweens
pub fn keyframes_needed(p: &[[f64; 6]]) -> usize {
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
    let mut decoded: HashMap<u16, Option<tiny_skia::Pixmap>> = HashMap::new();
    let mut count = 0usize;

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
            let p: Vec<[f64; 6]> = t.frames.iter().map(|(f, i)| art.placement(&anim.frames[f][*i])).collect();
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
                if run.len() == 1 { row[slot] = entries[run[0]].clone(); continue }
                // a run of pieces: one picture, drawn once for every frame that places them alike
                let pieces: Vec<&FrameImageEntry> = run.iter().map(|i| &entries[*i]).collect();
                let key: String = pieces.iter().map(|e| {
                    let m = art.transform(e);
                    format!("{}:{}:{}|", art.bitmap_of(e), m.iter().map(|v| format!("{:x}", v.to_bits())).collect::<Vec<_>>().join(","), e.alpha.to_bits())
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
                        let h = h.map(|(id, sym, _, _, x, y)| (id, sym, x, y));
                        if let Some(h) = &h { drawn.insert(key.clone(), h.clone()); }
                        h
                    }
                };
                let Some((id, sym, x, y)) = hit else { row[slot] = pieces[0].clone(); continue };
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
    decoded: &mut HashMap<u16, Option<tiny_skia::Pixmap>>,
    char_dir: &Path,
    sprites: &Path,
    char_name: &str,
    key: &str,
    id: u16,
) -> anyhow::Result<Option<(u16, String, u32, u32, f64, f64)>> {
    let mut placed: Vec<([f64; 6], u16, f32)> = Vec::new();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for e in pieces {
        let bmp = art.bitmap_of(e);
        let Some(img) = images.get(&bmp) else { continue };
        let pix = decoded.entry(bmp).or_insert_with(|| {
            let rgba = image::open(char_dir.join(&img.png_path)).ok()?.to_rgba8();
            let (w, h) = rgba.dimensions();
            let mut data = rgba.into_raw();
            for px in data.chunks_exact_mut(4) {
                let a = px[3] as u32;
                for c in &mut px[..3] { *c = ((*c as u32 * a + 127) / 255) as u8; }
            }
            tiny_skia::Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h)?)
        });
        let Some(pix) = pix else { continue };
        let m = art.transform(e);
        let (w, h) = (pix.width() as f64, pix.height() as f64);
        for (u, v) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
            let (x, y) = (m[0] * u + m[2] * v + m[4], m[1] * u + m[3] * v + m[5]);
            x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y);
        }
        placed.push((m, bmp, e.alpha as f32));
    }
    if placed.is_empty() { return Ok(None) }
    let (x0, y0) = (x0.floor(), y0.floor());
    let (w, h) = (((x1 - x0) * RES).ceil().max(1.0), ((y1 - y0) * RES).ceil().max(1.0));
    if w > 4096.0 || h > 4096.0 { return Ok(None) }
    let Some(mut canvas) = tiny_skia::Pixmap::new(w as u32, h as u32) else { return Ok(None) };
    for (m, bmp, alpha) in &placed {
        let Some(Some(pix)) = decoded.get(bmp) else { continue };
        let r = RES as f32;
        let t = tiny_skia::Transform::from_row(
            m[0] as f32 * r, m[1] as f32 * r, m[2] as f32 * r, m[3] as f32 * r,
            ((m[4] - x0) * RES) as f32, ((m[5] - y0) * RES) as f32);
        let paint = tiny_skia::PixmapPaint { opacity: alpha.clamp(0.0, 1.0), blend_mode: tiny_skia::BlendMode::SourceOver, quality: tiny_skia::FilterQuality::Bicubic };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, r: f64) -> [f64; 6] { [x, y, 1.0, 1.0, r, 1.0] }

    #[test]
    fn a_hold_and_a_straight_move_take_a_keyframe_each() {
        let mut t = vec![p(0.0, 0.0, 0.0); 4];
        t.extend((0..=6).map(|i| p(i as f64 * 3.0, 0.0, i as f64 * 10.0)));
        assert_eq!(keyframes_needed(&t), 3);
    }

    #[test]
    fn jitter_takes_a_keyframe_a_frame() {
        let t: Vec<[f64; 6]> = (0..8).map(|i| p(if i % 2 == 0 { 0.0 } else { 5.0 }, i as f64, 0.0)).collect();
        assert!(keyframes_needed(&t) * 2 > t.len());
    }
}
