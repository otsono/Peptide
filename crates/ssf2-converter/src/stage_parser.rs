//! stage_parser — extract a geometry model from an SSF2 stage `.ssf`.
//!
//! SSF2 stages are SWF resources: a root `stageMC` MovieClip holds a `terrain`
//! container (the collision geometry), named boundary clips (`deathBoundary`,
//! `camBoundary`, `smashBallBoundary`), and per-player `pN_Start` / `pN_Spawn`
//! beacon instances. We walk the placement tree accumulating the FULL affine
//! matrix down each branch, then read each instance's WORLD axis-aligned bounding
//! box (the referenced shape's local bounds transformed by the compounded matrix).
//!
//! Output is in **Fraymakers stage coordinates**: SSF2 world coords minus the
//! `stageMC` origin (FrayTools and SSF2 share a y-down pixel space at 1:1 scale —
//! the battlefield floor is ~520px wide in both). The FM stage emitter consumes
//! this model directly. Read-only on the input file.
//!
//! Reuses [`crate::ssf::decompress`] (the DAT-archive aware path; the raw `swf`
//! decompressor fails on the install's DATs).

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// A 2D affine matrix (SWF convention: x' = a·x + c·y + tx, y' = b·x + d·y + ty).
#[derive(Clone, Copy, Debug)]
struct Mat { a: f64, b: f64, c: f64, d: f64, tx: f64, ty: f64 }
impl Mat {
    fn id() -> Mat { Mat { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 } }
    /// `self ∘ other` — apply `other` first, then `self`.
    fn mul(&self, o: &Mat) -> Mat {
        Mat {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            tx: self.a * o.tx + self.c * o.ty + self.tx,
            ty: self.b * o.tx + self.d * o.ty + self.ty,
        }
    }
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }
    /// `+1.0` if this matrix preserves x-orientation, `-1.0` if mirrored (scaleX < 0).
    fn x_sign(&self) -> f64 { if self.a < 0.0 { -1.0 } else { 1.0 } }
}

/// An axis-aligned box in FM stage coordinates (x/y = top-left, w/h = size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }
impl Rect {
    pub fn left(&self) -> f64 { self.x }
    pub fn top(&self) -> f64 { self.y }
    pub fn right(&self) -> f64 { self.x + self.w }
    pub fn bottom(&self) -> f64 { self.y + self.h }
}

/// A collision platform (the main floor or a soft platform).
#[derive(Clone, Debug)]
pub struct Platform {
    /// World AABB in FM coords.
    pub rect: Rect,
    /// `true` for a one-way (drop-through) soft platform; `false` for solid terrain.
    pub drop_through: bool,
}

/// A player beacon (match-start "entrance" or respawn point) in FM coords.
#[derive(Clone, Debug)]
pub struct SpawnPoint {
    /// 0-based player index (p1 -> 0 …).
    pub index: usize,
    pub x: f64,
    pub y: f64,
    /// `true` if the beacon faces left (SSF2 scaleX < 0).
    pub face_left: bool,
}

/// The parsed SSF2 stage geometry, in FM stage coordinates.
#[derive(Clone, Debug)]
pub struct StageModel {
    /// Content id (from `Main.id`, fallback file stem).
    pub id: String,
    /// Collision platforms (floor + soft platforms), top surfaces are `rect.top()`.
    pub platforms: Vec<Platform>,
    /// Blast zone (KO boundary).
    pub death_box: Option<Rect>,
    /// Hard camera bounds.
    pub camera_box: Option<Rect>,
    /// Match-start beacons (SSF2 `pN_Start`), ordered by player index.
    pub entrances: Vec<SpawnPoint>,
    /// Respawn beacons (SSF2 `pN_Spawn`), ordered by player index.
    pub respawns: Vec<SpawnPoint>,
}

impl StageModel {
    /// The main (solid) floor: the widest non-drop-through platform, if any.
    pub fn main_floor(&self) -> Option<&Platform> {
        self.platforms.iter().filter(|p| !p.drop_through).max_by(|a, b| a.rect.w.total_cmp(&b.rect.w))
    }
}

/// One placed shape instance discovered during the tree walk.
struct Instance {
    /// PlaceObject instance name (e.g. `deathBoundary`), if any.
    inst_name: Option<String>,
    /// SWF SymbolClass name of the nearest named sprite ancestor (e.g.
    /// `battlefield_fla.battlefield_TerrainMC_5`).
    sym_name: String,
    /// World AABB in raw SSF2 coords.
    aabb: Rect,
    /// World center (raw SSF2 coords).
    cx: f64,
    cy: f64,
    /// `-1.0` if mirrored along x (facing left).
    x_sign: f64,
}

/// Parse the SSF2 stage at `path` into a [`StageModel`] (read-only).
pub fn parse_stage(path: &Path) -> Result<StageModel> {
    let raw = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let swf_data = ssf_decompress(&raw, path)?;
    let buf = swf::decompress_swf(&swf_data[..]).context("decompress SWF")?;
    let swf = swf::parse_swf(&buf).context("parse SWF")?;

    let id = stage_id(&swf_data).unwrap_or_else(|| {
        path.file_stem().and_then(|s| s.to_str()).unwrap_or("stage").to_string()
    });

    // SymbolClass id -> name; DefineShape bounds; DefineSprite tag lists.
    let mut sym_names: BTreeMap<u16, String> = BTreeMap::new();
    let mut shape_bounds: BTreeMap<u16, (f64, f64, f64, f64)> = BTreeMap::new();
    let mut sprites: BTreeMap<u16, &Vec<swf::Tag>> = BTreeMap::new();
    for tag in &swf.tags {
        match tag {
            swf::Tag::SymbolClass(links) => {
                for link in links {
                    let n = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                    if !n.is_empty() { sym_names.insert(link.id, n); }
                }
            }
            swf::Tag::DefineShape(s) => {
                let b = &s.shape_bounds;
                shape_bounds.insert(s.id, (
                    b.x_min.get() as f64 / 20.0, b.y_min.get() as f64 / 20.0,
                    b.x_max.get() as f64 / 20.0, b.y_max.get() as f64 / 20.0,
                ));
            }
            swf::Tag::DefineSprite(s) => { sprites.insert(s.id, &s.tags); }
            _ => {}
        }
    }

    // Collect every placed shape instance (with world AABB) and find the stageMC origin.
    let mut instances: Vec<Instance> = Vec::new();
    let mut origin: Option<(f64, f64)> = None;
    walk(&swf.tags, Mat::id(), &sym_names, &shape_bounds, &sprites, 0, None, &mut instances, &mut origin);

    let (ox, oy) = origin.unwrap_or((275.0, 200.0)); // SWF stage center fallback
    let to_fm = |r: &Rect| Rect { x: r.x - ox, y: r.y - oy, w: r.w, h: r.h };

    // --- platforms: TerrainMC = solid floor; *platform* = drop-through soft platform.
    let mut platforms: Vec<Platform> = Vec::new();
    for inst in &instances {
        let sn = inst.sym_name.to_ascii_lowercase();
        if sn.contains("terrainmc") {
            platforms.push(Platform { rect: to_fm(&inst.aabb), drop_through: false });
        } else if sn.contains("platform") {
            platforms.push(Platform { rect: to_fm(&inst.aabb), drop_through: true });
        }
    }

    // --- boundaries: identified by the carried name (the boundary clip is placed with
    // a PlaceObject name like `deathBoundary`, which the walk carries down to its shape).
    let mut death_box = None;
    let mut camera_box = None;
    for inst in &instances {
        let label = inst.inst_name.as_deref().unwrap_or(&inst.sym_name).to_ascii_lowercase();
        if label.contains("deathboundary") { death_box = Some(to_fm(&inst.aabb)); }
        else if label.contains("camboundary") { camera_box = Some(to_fm(&inst.aabb)); }
    }

    // --- spawns: pN_Start -> entrances, pN_Spawn -> respawns (by symbol-class name).
    let mut entrances: Vec<SpawnPoint> = Vec::new();
    let mut respawns: Vec<SpawnPoint> = Vec::new();
    for inst in &instances {
        let sn = inst.sym_name.to_ascii_lowercase();
        if let Some(idx) = player_index(&sn, "_start_") {
            entrances.push(SpawnPoint { index: idx, x: inst.cx - ox, y: inst.cy - oy, face_left: inst.x_sign < 0.0 });
        } else if let Some(idx) = player_index(&sn, "_spawn_") {
            respawns.push(SpawnPoint { index: idx, x: inst.cx - ox, y: inst.cy - oy, face_left: inst.x_sign < 0.0 });
        }
    }
    entrances.sort_by_key(|s| s.index);
    respawns.sort_by_key(|s| s.index);

    if platforms.is_empty() {
        bail!("no collision geometry found in {} (not a recognised SSF2 stage?)", path.display());
    }
    Ok(StageModel { id, platforms, death_box, camera_box, entrances, respawns })
}

/// Decompress an `.ssf` to SWF bytes via the DAT-archive aware path.
fn ssf_decompress(raw: &[u8], path: &Path) -> Result<Vec<u8>> {
    crate::ssf::decompress(raw).with_context(|| format!("decompress {}", path.display()))
}

/// Read `Main.id` from any ABC block (the content id the engine knows the stage by).
fn stage_id(swf_data: &[u8]) -> Option<String> {
    let swf = crate::swf_parser::parse(swf_data).ok()?;
    for abc_bytes in &swf.abc_blocks {
        if let Ok(abc) = crate::abc_parser::parse(abc_bytes) {
            if let Some(md) = crate::abc_parser::extract_main_package_metadata(&abc) {
                if let Some(id) = md.id { return Some(id); }
            }
        }
    }
    None
}

/// Extract a 0-based player index from a symbol name like
/// `battlefield_fla.p1_Start_9` given the marker `"_start_"` / `"_spawn_"`.
/// Looks for `pN` immediately before the marker.
fn player_index(sym_lower: &str, marker: &str) -> Option<usize> {
    let pos = sym_lower.find(marker)?;
    // walk back over the `pN` token preceding the marker
    let prefix = &sym_lower[..pos];
    let digits: String = prefix.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect();
    if digits.is_empty() { return None; }
    // the char before the digits must be 'p'
    let p_idx = prefix.len() - digits.len();
    if prefix.as_bytes().get(p_idx.wrapping_sub(1)) != Some(&b'p') { return None; }
    digits.parse::<usize>().ok().and_then(|n| n.checked_sub(1))
}

/// Recurse the placement tree, recording each placed shape's world AABB and the
/// `stageMC` origin (the root instance whose name is `stageMC` or whose symbol
/// class starts with `stage_`).
#[allow(clippy::too_many_arguments)]
fn walk<'a>(
    tags: &'a [swf::Tag], parent: Mat,
    sym_names: &BTreeMap<u16, String>,
    shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
    sprites: &BTreeMap<u16, &'a Vec<swf::Tag>>,
    rec: usize, carried_sym: Option<&str>,
    out: &mut Vec<Instance>, origin: &mut Option<(f64, f64)>,
) {
    if rec > 8 { return; }
    for tag in tags {
        let swf::Tag::PlaceObject(po) = tag else { continue };
        let id = match &po.action {
            swf::PlaceObjectAction::Place(id) | swf::PlaceObjectAction::Replace(id) => *id,
            swf::PlaceObjectAction::Modify => continue,
        };
        let local = po.matrix.as_ref().map(|m| Mat {
            a: m.a.to_f64(), b: m.b.to_f64(), c: m.c.to_f64(), d: m.d.to_f64(),
            tx: m.tx.get() as f64 / 20.0, ty: m.ty.get() as f64 / 20.0,
        }).unwrap_or(Mat::id());
        let world = parent.mul(&local);
        let inst_name = po.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
        let sym = sym_names.get(&id).cloned().unwrap_or_default();

        // record the stage origin from the root stageMC instance
        if origin.is_none()
            && (inst_name.as_deref() == Some("stageMC") || sym.to_ascii_lowercase().starts_with("stage_"))
        {
            *origin = Some((world.tx, world.ty));
        }

        if let Some((x0, y0, x1, y1)) = shape_bounds.get(&id) {
            let corners = [world.apply(*x0, *y0), world.apply(*x1, *y0), world.apply(*x1, *y1), world.apply(*x0, *y1)];
            let xmn = corners.iter().map(|c| c.0).fold(f64::MAX, f64::min);
            let xmx = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max);
            let ymn = corners.iter().map(|c| c.1).fold(f64::MAX, f64::min);
            let ymx = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max);
            out.push(Instance {
                inst_name: inst_name.clone(),
                sym_name: carried_sym.unwrap_or("").to_string(),
                aabb: Rect { x: xmn, y: ymn, w: xmx - xmn, h: ymx - ymn },
                cx: world.tx, cy: world.ty,
                x_sign: world.x_sign(),
            });
        }
        if let Some(child) = sprites.get(&id) {
            // carry the most specific identity: instance name wins, else symbol class,
            // else inherit the parent's carried symbol.
            let next = inst_name.as_deref().or(if sym.is_empty() { carried_sym } else { Some(&sym) });
            let next = next.map(|s| s.to_string());
            walk(child, world, sym_names, shape_bounds, sprites, rec + 1, next.as_deref(), out, origin);
        }
    }
}
