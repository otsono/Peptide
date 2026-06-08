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
    /// The walkable TOP surface as a polyline in FM coords (left to right), when the terrain
    /// is curved/sloped. `None` for a flat platform (use `rect.top()` as a level line).
    pub profile: Option<Vec<(f64, f64)>>,
    /// `true` if the SSF2 source moves this platform (its instance name carries `moving`).
    /// Emitted as a STATIC platform at its start position: SSF2 moving-platform motion is
    /// bespoke per stage (custom AS3 / timeline animation) and isn't ported yet.
    pub moving: bool,
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
    /// Content id (from `Main.id`, fallback file stem). The emitter may suffix this
    /// (`<id>ssf2`) so it can't shadow a built-in stage; `display_name` stays clean.
    pub id: String,
    /// Human display name for the stage-select screen (override map, else the SSF2 id
    /// title-cased).
    pub display_name: String,
    /// Source series (for the description), if known from the override map.
    pub series: Option<String>,
    /// The stage's original SSF2 soundtrack track ids (`bgm_*`), preserved in the
    /// description. Not playable in FM (the audio isn't shipped), recorded for the author.
    pub ssf2_music: Vec<String>,
    /// FM bgm resource ids the converted stage actually references (override map, else the
    /// configured default). Must be real public FM resources so the match can start.
    pub fm_music: Vec<String>,
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
    /// Grabbable ledge x-positions `(left, right)` from the SSF2 `ledge_mc` instances,
    /// in FM coords. Used as the main floor's left/right endpoints.
    pub ledges: Option<(f64, f64)>,
    /// Rendered stage art, split into depth layers (the painted backdrop, the main
    /// stage at character depth, and a foreground that draws in front of fighters).
    pub art: StageArtSet,
    /// Non-fatal validation notes: required SSF2 stage linkages that were missing from
    /// the source (e.g. no collision floor, no spawn beacons). Surfaced to the user.
    pub warnings: Vec<String>,
    /// SSF2 -> Fraymakers spatial scale (the `size_multiplier` knob, default 1.3). Fraymakers
    /// space is SSF2 space scaled up by this factor (characters are rendered at this scale,
    /// so the stage geometry + art must match it). Geometry coords are already scaled; the
    /// emitter renders the art IMAGE layers at this scale.
    pub scale: f64,
}

/// The stage art split by depth so the emitter can layer it around the characters
/// and parallax-scroll the background.
#[derive(Clone, Debug, Default)]
pub struct StageArtSet {
    /// Fixed painted backdrop (SSF2 `<id>_bg` / `background`), drawn behind everything.
    /// Moves 1:1 with the world (it carries the surface fighters stand on).
    pub background: Option<StageArt>,
    /// Camera-relative parallax layers (the SSF2 `<id>_bg` backdrop + the `_cambg` layers that
    /// `SSF2Stage.getCameraBackgrounds` returns), back-to-front. Each is its OWN plane with its
    /// OWN pan rate (SSF2 auto-derives it from the layer size). Empty for the 109/110 corpus
    /// stages with no camera backgrounds.
    pub parallax: Vec<ParallaxLayer>,
    /// The main stage art (terrain / platforms / props) at character depth. More than
    /// one frame when the source has animated clips (the emitter loops them).
    pub stage_frames: Vec<StageArt>,
    /// Art that draws in front of the fighters (SSF2 `foreground`).
    pub foreground: Option<StageArt>,
}

impl StageArtSet {
    /// `true` if no layer rasterized (e.g. a stage with only bitmap fills we can't
    /// decode) — the emitter then falls back to a geometry placeholder.
    pub fn is_empty(&self) -> bool {
        self.background.is_none() && self.parallax.is_empty()
            && self.stage_frames.is_empty() && self.foreground.is_none()
    }
}

/// How a camera-background layer scrolls (mirrors SSF2 `VcamBGSettings.mode` =
/// Fraymakers `ParallaxMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParallaxMode {
    /// Straight parallax pan at `x_pan`/`y_pan` (`bg = camera * multiplier`). SSF2's `_cambg`
    /// discrete layers use this (the `autoPanMultiplier` feeds the pan).
    Pan,
    /// Anchored to the camera bounds, tiling to fill (SSF2 makes ±width copies). For a
    /// repeating/wrapping backdrop.
    Bounds,
}

/// One camera-background parallax layer: the composited art + its per-layer scroll mode and
/// pan rate (SSF2 auto-derives the rate from the layer's pixel size; see `parallax_pan`).
#[derive(Clone, Debug)]
pub struct ParallaxLayer {
    pub art: StageArt,
    pub mode: ParallaxMode,
    /// Fraction of the camera's movement this layer scrolls by (0 = screen-fixed).
    pub x_pan: f64,
    pub y_pan: f64,
}

/// A composited stage-art image ready to drop in as an IMAGE layer / parallax bg.
#[derive(Clone, Debug)]
pub struct StageArt {
    /// PNG bytes (RGBA).
    pub png: Vec<u8>,
    /// Top-left of the image in FM stage coords.
    pub x: f64,
    pub y: f64,
    /// Pixel dimensions (for parallax `originalBGWidth/Height`).
    pub w: u32,
    pub h: u32,
}

impl StageModel {
    /// The main (solid) floor: the widest non-drop-through platform, if any.
    pub fn main_floor(&self) -> Option<&Platform> {
        self.platforms.iter().filter(|p| !p.drop_through).max_by(|a, b| a.rect.w.total_cmp(&b.rect.w))
    }
}

/// One placed shape instance discovered during the tree walk.
#[derive(Clone)]
struct Instance {
    /// The placed `DefineShape` id (so its art can be rasterized later).
    shape_id: u16,
    /// PlaceObject instance name (e.g. `deathBoundary`), if any.
    inst_name: Option<String>,
    /// SWF SymbolClass name of the nearest named sprite ancestor (e.g.
    /// `battlefield_fla.battlefield_TerrainMC_5`). Fla-prefixed + auto-numbered, so it's a
    /// weak cross-stage signal; prefer [`Instance::plane`] for art and linkage suffixes for
    /// markers.
    sym_name: String,
    /// The stage-root plane this instance descends from, by INSTANCE name (the stable AS3
    /// slot: `terrain` / `background` / `foreground` / `shadowMask` / `reflectionMask`), or
    /// an extra root instance (`*_cambg` parallax, `stance`, decorations). The robust art
    /// classifier (instance names generalize across stages; symbol names do not).
    plane: Option<String>,
    /// World AABB in raw SSF2 coords.
    aabb: Rect,
    /// World center (raw SSF2 coords).
    cx: f64,
    cy: f64,
    /// `-1.0` if mirrored along x (facing left).
    x_sign: f64,
    /// `true` if this shape descends from a `moving`-named ancestor (an SSF2 moving
    /// platform/foreground). The collision child is usually named `terrainGround_platform`,
    /// so the `moving` signal lives on the parent container and is propagated down.
    moving: bool,
}

/// Parse the SSF2 stage at `path` into a [`StageModel`], rendering its art (read-only).
pub fn parse_stage(path: &Path) -> Result<StageModel> {
    parse_stage_opts(path, true)
}

/// Parse the SSF2 stage at `path`. `render_art` toggles the (relatively expensive)
/// art rasterization: the converter needs it, but a geometry-only pass (e.g. the
/// coverage test, or `--info`) skips decoding + compositing every shape/bitmap.
pub fn parse_stage_opts(path: &Path, render_art_flag: bool) -> Result<StageModel> {
    let raw = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let swf_data = ssf_decompress(&raw, path)?;
    let buf = swf::decompress_swf(&swf_data[..]).context("decompress SWF")?;
    let swf = swf::parse_swf(&buf).context("parse SWF")?;

    let meta = stage_package_metadata(&swf_data);
    let id = meta.as_ref().and_then(|m| m.id.clone()).unwrap_or_else(|| {
        path.file_stem().and_then(|s| s.to_str()).unwrap_or("stage").to_string()
    });
    let ssf2_music = meta.map(|m| m.music).unwrap_or_default();
    // display name + FM music: the override map keyed by the SSF2 id, else title-case +
    // the default bgm. The original SSF2 soundtrack (ssf2_music) is preserved separately.
    let smeta = crate::mappings::stage_metadata();
    let entry = smeta.stages.get(&id);
    let display_name = entry.and_then(|e| e.name.clone()).unwrap_or_else(|| title_case(&id));
    let series = entry.and_then(|e| e.series.clone());
    let fm_music = match entry.map(|e| &e.music) {
        Some(m) if !m.is_empty() => m.clone(),
        _ => vec![smeta.default_music.clone()],
    };

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
    // DefineShape registry, for rasterizing stage art.
    let mut shape_defs: BTreeMap<u16, &swf::Shape> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::DefineShape(s) = tag { shape_defs.insert(s.id, s); }
    }
    // Bitmap registry (id -> (w, h, RGBA)) for shapes whose fill is a bitmap (stage
    // backgrounds are usually bitmaps, which `vector_raster` can't fill). Only decoded
    // when rendering art (decoding every stage's bitmaps is the slow part).
    let mut bitmaps: BTreeMap<u16, (u32, u32, Vec<u8>)> = BTreeMap::new();
    if render_art_flag {
        for tag in &swf.tags {
            match tag {
                swf::Tag::DefineBitsLossless(b) => {
                    if let Ok(rgba) = crate::image_extractor::decode_lossless(b) {
                        bitmaps.insert(b.id, (b.width as u32, b.height as u32, rgba));
                    }
                }
                swf::Tag::DefineBitsJpeg3(j) => {
                    if let Ok((w, h, rgba)) = crate::image_extractor::decode_jpeg3(j) {
                        bitmaps.insert(j.id, (w, h, rgba));
                    }
                }
                _ => {}
            }
        }
    }

    // PEPTIDE_STAGE_DEBUG dumps the SymbolClass linkage table; PEPTIDE_STAGE_TREE (in `walk`)
    // dumps the placement tree with each instance's resolved plane. Both gated, for stage RE.
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        eprintln!("=== SymbolClass linkage table ({} entries) for {} ===", sym_names.len(), id);
        for (cid, n) in &sym_names { eprintln!("  id={cid} linkage={n:?}"); }
    }

    // Collect every placed shape instance (with world AABB) and find the stageMC origin.
    let mut instances: Vec<Instance> = Vec::new();
    let mut origin: Option<(f64, f64)> = None;
    walk(&swf.tags, Mat::id(), &sym_names, &shape_bounds, &sprites, 0, None, None, false, &mut instances, &mut origin);

    let (ox, oy) = origin.unwrap_or((275.0, 200.0)); // SWF stage center fallback
    // Fraymakers space = SSF2 space scaled up by `size_multiplier` (the same knob the
    // character converter scales sprites by, default 1.3), so the stage matches the
    // scaled-up fighters and the art fills the FM camera the way it did in SSF2.
    let scale = crate::mappings::character_stats().scaling.size_multiplier;
    let to_fm = |r: &Rect| Rect {
        x: (r.x - ox) * scale, y: (r.y - oy) * scale, w: r.w * scale, h: r.h * scale,
    };

    // --- platforms: `*platform*` = drop-through soft platform; otherwise any terrain /
    // collision shape (SSF2 stages name these inconsistently: TerrainMC, terrain_mc,
    // ChunkTerrain, CollisonBox [sic], ground) is a solid floor. Check `platform` first so
    // `terrainGround_platform` (both words) is classified drop-through.
    let mut platforms: Vec<Platform> = Vec::new();
    let mut moving_count = 0usize;
    for inst in &instances {
        let sn = inst.sym_name.to_ascii_lowercase();
        // `back`/`fore`ground are ART, not collision — exclude them (they contain the
        // substring "ground", which would otherwise read as a floor).
        let is_art_bg = sn.contains("background") || sn.contains("foreground");
        // SSF2 moving platforms carry `moving` in the linkage (movingplatform_N,
        // tos_movingplatform_6, movingPlatformTerrain_14) — usually on the parent container,
        // not the collision child, so the walk propagates it down. We collide with them at
        // their start position; the motion itself is bespoke per stage and not ported yet.
        let moving = inst.moving;
        if !is_art_bg && sn.contains("platform") {
            if moving { moving_count += 1; }
            platforms.push(Platform { rect: to_fm(&inst.aabb), drop_through: true, profile: None, moving });
        } else if !is_art_bg && ["terrain", "collison", "collision", "ground"].iter().any(|m| sn.contains(m)) {
            // solid terrain can be curved/sloped (e.g. a hilly island), so trace its top
            // surface as a polyline instead of a flat line.
            let profile = shape_defs.get(&inst.shape_id)
                .and_then(|s| floor_profile(s, &inst.aabb, ox, oy, scale));
            if moving { moving_count += 1; }
            platforms.push(Platform { rect: to_fm(&inst.aabb), drop_through: false, profile, moving });
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
            entrances.push(SpawnPoint { index: idx, x: (inst.cx - ox) * scale, y: (inst.cy - oy) * scale, face_left: inst.x_sign < 0.0 });
        } else if let Some(idx) = player_index(&sn, "_spawn_") {
            respawns.push(SpawnPoint { index: idx, x: (inst.cx - ox) * scale, y: (inst.cy - oy) * scale, face_left: inst.x_sign < 0.0 });
        }
    }
    entrances.sort_by_key(|s| s.index);
    respawns.sort_by_key(|s| s.index);

    // --- ledges: the SSF2 `ledge_mc_left` / `ledge_mc_right` beacons mark the floor's
    // grabbable edges (FM grabs ledges at the floor's left/right endpoints).
    let ledge_x = |needle: &str| instances.iter()
        .find(|i| i.sym_name.to_ascii_lowercase().contains(needle))
        .map(|i| (i.cx - ox) * scale);
    let ledges = match (ledge_x("ledge_mc_left"), ledge_x("ledge_mc_right")) {
        (Some(l), Some(r)) => Some((l.min(r), l.max(r))),
        _ => None,
    };

    if platforms.is_empty() {
        bail!("no collision geometry found in {} (not a recognised SSF2 stage?)", path.display());
    }

    // --- art: rasterize the stage's shapes, split into background / stage / foreground.
    // The PNGs stay native-resolution; placement is scaled here and the emitter renders the
    // IMAGE layers at `scale`, matching the geometry.
    let art = if render_art_flag {
        render_art_layers(&swf.tags, &sprites, &sym_names, &shape_defs, &bitmaps, ox, oy, scale)
    } else {
        StageArtSet::default()
    };

    // --- validate: the SSF2 source should carry the standard stage linkages, and the
    // parse should have recovered the playable bits. Non-fatal notes for the user.
    let mut warnings = validate_ssf2_linkages(&sym_names, &id);
    if death_box.is_none() { warnings.push("blast zone not parsed (no deathBoundary)".into()); }
    if camera_box.is_none() { warnings.push("camera bounds not parsed (no camBoundary)".into()); }
    if entrances.is_empty() { warnings.push("no match-start entrances parsed (pN_Start)".into()); }
    if !platforms.iter().any(|p| !p.drop_through) { warnings.push("no solid floor parsed".into()); }
    if moving_count > 0 {
        warnings.push(format!("{moving_count} moving platform(s) emitted as static (SSF2 \
            moving-platform motion is bespoke per stage and not ported yet)"));
    }

    Ok(StageModel { id, display_name, series, ssf2_music, fm_music, platforms, death_box, camera_box, entrances, respawns, ledges, art, warnings, scale })
}

/// Validate that the SSF2 source carries the linkages a playable stage needs. Returns a
/// note for each missing one (non-fatal: the converter fills fallbacks, but a gap usually
/// means a mis-parse or an unusual stage). Keyed off the SymbolClass linkage table.
fn validate_ssf2_linkages(sym_names: &BTreeMap<u16, String>, model_id: &str) -> Vec<String> {
    let all: Vec<String> = sym_names.values().map(|s| s.to_ascii_lowercase()).collect();
    let has = |needles: &[&str]| all.iter().any(|n| needles.iter().any(|m| n.contains(m)));
    let mut w = Vec::new();
    if !has(&["terrainmc", "terrain_mc", "collison", "collision"]) {
        w.push("SSF2 source has no collision-floor linkage (TerrainMC / CollisonBox)".into());
    }
    if !has(&["_start_"]) { w.push("SSF2 source has no pN_Start spawn beacons".into()); }
    if !has(&["boundary"]) { w.push("SSF2 source has no boundary_clip (blast/camera bounds)".into()); }
    if !has(&["_bg", "background"]) {
        w.push(format!("SSF2 source has no backdrop art linkage (`{model_id}_bg`)"));
    }
    w
}

/// Depth layer an art instance belongs to (back-to-front). `Backdrop` is the document-root
/// `<id>_bg` scene (the far sky), which composites WITH `Parallax` (the `_cambg` layers) into
/// one camera-relative background when parallax is present, so the rays/sun draw in front of
/// the sky instead of being occluded by it.
#[derive(Clone, Copy, PartialEq)]
enum ArtKind { Backdrop, Parallax, Background, Stage, Foreground }

/// Classify a *visual* art instance by its stage PLANE — the instance name of the stage-root
/// child it descends from (the stable AS3 slot: `terrain`/`background`/`foreground`/mask, or
/// an extra root instance). Instance names generalize across stages; fla-prefixed symbol
/// names do not. `*_cambg` = a camera-background parallax layer (SSF2
/// `SSF2Stage.getCameraBackgrounds`); `foreground`/`*_fg` = in front of fighters;
/// `background` = the fixed backdrop; decorations (clouds, lensflare, …) sit at stage depth.
/// Collision (`terrain`) and masks never reach here (filtered by [`is_non_art_plane`]).
fn art_kind(inst: &Instance) -> ArtKind {
    let plane = inst.plane.as_deref().unwrap_or("").to_ascii_lowercase();
    if plane.contains("cambg") {
        ArtKind::Parallax
    } else if plane == "foreground" || plane.contains("_fg") {
        ArtKind::Foreground
    } else if plane == "backdrop" {
        ArtKind::Backdrop
    } else if plane == "background" {
        ArtKind::Background
    } else {
        ArtKind::Stage
    }
}

/// Planes that are not visual art: collision (`terrain`), the shadow/reflection masks, and
/// spawn-pose beacons (`stance`). The plane is already normalized by [`plane_tag`].
fn is_non_art_plane(plane: Option<&str>) -> bool {
    matches!(plane, Some("terrain") | Some("mask") | Some("stance"))
}

/// The normalized stage plane an instance establishes for its subtree, from its INSTANCE
/// name (the AS3 slot vocabulary + the `_cambg` parallax / `_fg` foreground conventions),
/// or `None` to inherit the parent's plane. The one symbol-name exception: the document-
/// root backdrop container carries no instance name but the explicit linkage `<id>_bg`, so
/// an unnamed `*_bg` instance tags `background`. Instance/linkage signals generalize across
/// stages; fla-prefixed timeline symbol names do not.
fn plane_tag(inst_name: Option<&str>, sym: &str) -> Option<&'static str> {
    let n = inst_name.unwrap_or("");
    let nl = n.to_ascii_lowercase();
    if nl.contains("cambg") { Some("cambg") }
    else if nl == "foreground" || nl.contains("_fg") { Some("foreground") }
    else if nl == "background" { Some("background") }
    else if nl == "terrain" { Some("terrain") }
    else if nl.contains("mask") { Some("mask") }
    else if nl == "stance" { Some("stance") }
    // the document-root backdrop container (`<id>_bg`, and `*_bg` sky props inside it) carries
    // no instance name. it sits BEHIND the stageMC `background` plane and holds the `_cambg`
    // parallax layers, so it gets its own `backdrop` tag (composited with the parallax when
    // present, otherwise drawn as the farthest fixed layer).
    else if n.is_empty() && sym.to_ascii_lowercase().ends_with("_bg") { Some("backdrop") }
    else { None }
}

// ── SSF2 stage linkage vocabulary ──
// A shipped SSF2 stage carries a consistent set of named symbols (linkage ids). Art
// classification and validation key off these names. Two families are NOT art:
//  - COLLISION masks: `*TerrainMC*`, `terrainGround_platform*`, `CollisonBox*` [sic],
//    `ledge_mc_*`. SSF2 renders them invisibly; Peptide maps them to FM collision boxes /
//    line segments. Drawing them as art double-draws a flat silhouette over the backdrop.
//  - SCAFFOLDING: `pN_Start`/`pN_Spawn` beacons, the death/camera `boundary_clip`,
//    off-screen `warningbounds_*`, the `itemGen_mc`, `shadowMask`/`reflectionMask`, the
//    `light_source_mc`, the `smashball` spawn. All non-visual.

/// SSF2 collision-geometry linkage (invisible in SSF2; becomes FM collision, never art).
fn is_collision_linkage(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    l.contains("terrainmc") || l.contains("terrain_mc") || l.contains("platform")
        || l.contains("collison") || l.contains("collision") || l.contains("ledge_mc")
}

/// SSF2 non-visual scaffolding linkage (spawn/boundary/warning/itemgen/mask/light/smashball).
fn is_scaffold_linkage(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    ["_start_", "_spawn_", "boundary", "warningbounds", "warning", "itemgen",
     "shadowmask", "reflectionmask", "light_source", "smashball"]
        .iter().any(|m| l.contains(m))
}

/// `true` if a placed instance is collision geometry or scaffolding (not stage art).
fn is_non_art(label: &str) -> bool {
    is_collision_linkage(label) || is_scaffold_linkage(label)
}

/// Cap on stage-animation frames (SSF2 clips can be hundreds of frames; we loop a short
/// slice). Background/foreground are single-frame.
const ANIM_FRAME_CAP: usize = 12;

/// One placed child in a sprite's timeline frame: `(character id, local matrix, name)`.
type PlacedChild = (u16, Mat, Option<String>);

/// Build a sprite/root timeline: the placed-child state snapshotted at each `ShowFrame`
/// (Flash semantics — Place/Replace set a depth, Modify updates its matrix, Remove clears
/// it). At least one frame.
fn build_frames(tags: &[swf::Tag]) -> Vec<Vec<PlacedChild>> {
    let mut depth: std::collections::BTreeMap<u16, PlacedChild> = std::collections::BTreeMap::new();
    let mat_of = |po: &swf::PlaceObject| po.matrix.as_ref().map(|m| Mat {
        a: m.a.to_f64(), b: m.b.to_f64(), c: m.c.to_f64(), d: m.d.to_f64(),
        tx: m.tx.get() as f64 / 20.0, ty: m.ty.get() as f64 / 20.0,
    });
    let name_of = |po: &swf::PlaceObject| po.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
    let mut frames: Vec<Vec<PlacedChild>> = Vec::new();
    for tag in tags {
        match tag {
            swf::Tag::PlaceObject(po) => match &po.action {
                swf::PlaceObjectAction::Place(id) | swf::PlaceObjectAction::Replace(id) => {
                    depth.insert(po.depth, (*id, mat_of(po).unwrap_or(Mat::id()), name_of(po)));
                }
                swf::PlaceObjectAction::Modify => {
                    if let Some(e) = depth.get_mut(&po.depth) {
                        if let Some(m) = mat_of(po) { e.1 = m; }
                        if let Some(n) = name_of(po) { e.2 = Some(n); }
                    }
                }
            },
            swf::Tag::RemoveObject(r) => { depth.remove(&r.depth); }
            swf::Tag::ShowFrame => { frames.push(depth.values().cloned().collect()); }
            _ => {}
        }
    }
    if frames.is_empty() { frames.push(depth.values().cloned().collect()); }
    frames
}

/// Walk the placement tree at a fixed global frame (every animated sprite shows its
/// `global_frame % len` frame — Flash advances all clips together), collecting the placed
/// shape instances with world AABBs. Mirrors [`walk`] but frame-aware.
#[allow(clippy::too_many_arguments)]
fn walk_frame(
    children: &[PlacedChild], parent: Mat, global_frame: usize, carried_sym: Option<&str>,
    plane: Option<&str>,
    sym_names: &BTreeMap<u16, String>, shape_defs: &BTreeMap<u16, &swf::Shape>,
    sprite_frames: &BTreeMap<u16, Vec<Vec<PlacedChild>>>, out: &mut Vec<Instance>, rec: usize,
) {
    if rec > 8 { return; }
    for (id, local, name) in children {
        let world = parent.mul(local);
        let sym = sym_names.get(id).cloned().unwrap_or_default();
        // an instance establishes a plane for its subtree by instance name (or the `_bg`
        // linkage for the unnamed root backdrop); otherwise it inherits the parent's plane.
        let my_plane = plane_tag(name.as_deref(), &sym).or(plane);
        if let Some(s) = shape_defs.get(id) {
            let b = &s.shape_bounds;
            let (x0, y0, x1, y1) = (b.x_min.get() as f64/20.0, b.y_min.get() as f64/20.0, b.x_max.get() as f64/20.0, b.y_max.get() as f64/20.0);
            let cs = [world.apply(x0, y0), world.apply(x1, y0), world.apply(x1, y1), world.apply(x0, y1)];
            let xmn = cs.iter().map(|c| c.0).fold(f64::MAX, f64::min);
            let xmx = cs.iter().map(|c| c.0).fold(f64::MIN, f64::max);
            let ymn = cs.iter().map(|c| c.1).fold(f64::MAX, f64::min);
            let ymx = cs.iter().map(|c| c.1).fold(f64::MIN, f64::max);
            out.push(Instance {
                shape_id: *id, inst_name: name.clone(), sym_name: carried_sym.unwrap_or("").to_string(),
                plane: my_plane.map(str::to_string),
                aabb: Rect { x: xmn, y: ymn, w: xmx - xmn, h: ymx - ymn },
                cx: world.tx, cy: world.ty, x_sign: world.x_sign(),
                moving: false, // art-path instances classify by plane, not collision
            });
        }
        if let Some(frames) = sprite_frames.get(id) {
            let next = name.as_deref().or(if sym.is_empty() { carried_sym } else { Some(&sym) }).map(|s| s.to_string());
            let f = &frames[global_frame % frames.len()];
            walk_frame(f, world, global_frame, next.as_deref(), my_plane, sym_names, shape_defs, sprite_frames, out, rec + 1);
        }
    }
}

/// Rasterize the stage's art into background / stage / foreground layers. The stage layer
/// is frame-aware: if the source has animated clips it renders multiple frames (the
/// emitter loops them). Background/foreground are single-frame (frame 0).
#[allow(clippy::too_many_arguments)]
fn render_art_layers(
    root_tags: &[swf::Tag],
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    sym_names: &BTreeMap<u16, String>,
    shape_defs: &BTreeMap<u16, &swf::Shape>,
    bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>,
    ox: f64, oy: f64, scale: f64,
) -> StageArtSet {
    // per-sprite + root frame timelines.
    let mut sprite_frames: BTreeMap<u16, Vec<Vec<PlacedChild>>> = BTreeMap::new();
    for (id, tags) in sprites { sprite_frames.insert(*id, build_frames(tags)); }
    let root_frames = build_frames(root_tags);

    // full animation length = longest timeline. We sample ANIM_FRAME_CAP frames spread
    // EVENLY across it (a 645-frame clip's first 12 frames are usually a static slice, so
    // sampling end-to-end captures the motion); the emitter loops the samples.
    let full_len = sprite_frames.values().map(|f| f.len())
        .chain(std::iter::once(root_frames.len())).max().unwrap_or(1);
    let n_samples = full_len.min(ANIM_FRAME_CAP);
    let sample_frame = |i: usize| -> usize { i * full_len / n_samples };

    // instances at a given global frame, classified + composited per layer.
    let frame_instances = |g: usize| -> Vec<Instance> {
        let root = &root_frames[g % root_frames.len()];
        let mut out = Vec::new();
        walk_frame(root, Mat::id(), g, None, None, sym_names, shape_defs, &sprite_frames, &mut out, 0);
        // exclude non-art PLANES (terrain/masks/spawns, by instance name) and any stray
        // collision/scaffolding markers (by linkage suffix) that slipped into an art plane.
        out.retain(|i| shape_defs.contains_key(&i.shape_id)
            && !is_non_art_plane(i.plane.as_deref())
            && !is_non_art(i.inst_name.as_deref().unwrap_or("")) && !is_non_art(&i.sym_name));
        out
    };
    let composite = |insts: &[Instance], kinds: &[ArtKind]| -> Option<StageArt> {
        let group: Vec<&Instance> = insts.iter().filter(|i| kinds.contains(&art_kind(i))).collect();
        // PNG stays native-resolution; only the placement is scaled (the emitter renders the
        // IMAGE layer at `scale`), so the art and geometry share one scale with no upscaling.
        composite_layer(&group, shape_defs, bitmaps, ox, oy).map(|mut a| {
            a.x *= scale; a.y *= scale; a
        })
    };

    // sample every frame's instance set + the composited image.
    let sampled: Vec<(Vec<Instance>, Option<StageArt>)> = (0..n_samples).map(|i| {
        let insts = frame_instances(sample_frame(i));
        let img = composite(&insts, &[ArtKind::Stage]);
        (insts, img)
    }).collect();
    let stage_counts: Vec<usize> = sampled.iter()
        .map(|(insts, _)| insts.iter().filter(|x| art_kind(x) == ArtKind::Stage).count()).collect();
    let max_count = stage_counts.iter().copied().max().unwrap_or(0);

    // background / foreground from the richest frame (an animated stage's frame 0 can be
    // empty; pick the frame with the most content for the static layers).
    let base_idx = stage_counts.iter().enumerate().max_by_key(|(_, c)| **c).map(|(i, _)| i).unwrap_or(0);
    let base_insts = &sampled[base_idx].0;
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        eprintln!("=== art instances (richest frame {base_idx}, {} insts) ===", base_insts.len());
        for i in base_insts {
            let k = match art_kind(i) { ArtKind::Backdrop => "BKDRP", ArtKind::Parallax => "PARLX", ArtKind::Background => "BG", ArtKind::Stage => "STAGE", ArtKind::Foreground => "FG" };
            eprintln!("  [{k:5}] plane={:?} sym={:?} aabb=({:.0},{:.0} {:.0}x{:.0})",
                i.plane, i.sym_name, i.aabb.x, i.aabb.y, i.aabb.w, i.aabb.h);
        }
    }
    let foreground = composite(base_insts, &[ArtKind::Foreground]);
    // when the backdrop carries `_cambg` parallax layers, each backdrop/cambg LAYER (grouped
    // by symbol) becomes its own camera-relative plane with its own auto-derived pan rate (so
    // the sky, sun rays, trees, ... scroll at different rates, reading as depth — and the rays
    // draw in front of the sky, not occluded by it). The stageMC `background` plane (the
    // island fighters stand near) stays a fixed near-layer in front. Without parallax, backdrop
    // + background fold into one fixed bg.
    let has_parallax = base_insts.iter().any(|i| art_kind(i) == ArtKind::Parallax);
    let (background, parallax) = if has_parallax {
        let mut order: Vec<&str> = Vec::new();
        let mut groups: BTreeMap<&str, Vec<&Instance>> = BTreeMap::new();
        for i in base_insts.iter().filter(|i| matches!(art_kind(i), ArtKind::Backdrop | ArtKind::Parallax)) {
            let key = i.sym_name.as_str();
            if !groups.contains_key(key) { order.push(key); }
            groups.entry(key).or_default().push(i);
        }
        // SSF2 `_cambg` layers are discrete elements that PAN (the autoPanMultiplier feeds the
        // pan); BOUNDS is for a tiling/wrapping backdrop, which the corpus has none of. Default
        // PAN; `PEPTIDE_PARALLAX_BOUNDS` forces BOUNDS to exercise that path.
        let mode = if std::env::var("PEPTIDE_PARALLAX_BOUNDS").is_ok() { ParallaxMode::Bounds } else { ParallaxMode::Pan };
        let layers: Vec<ParallaxLayer> = order.iter().filter_map(|key| {
            composite_layer(&groups[*key], shape_defs, bitmaps, ox, oy).map(|mut art| {
                art.x *= scale; art.y *= scale;
                let (x_pan, y_pan) = parallax_pan(art.w, art.h);
                ParallaxLayer { art, mode, x_pan, y_pan }
            })
        }).collect();
        (composite(base_insts, &[ArtKind::Background]), layers)
    } else {
        (composite(base_insts, &[ArtKind::Backdrop, ArtKind::Background]), Vec::new())
    };

    // Emit a multi-frame stage animation only when the samples form a CLEAN animation:
    // every frame well-populated (no blinking on/off) and at least two distinct images.
    // Otherwise (static, or a chaotic/long SSF2 timeline) use the single richest frame.
    let imgs: Vec<&StageArt> = sampled.iter().filter_map(|(_, img)| img.as_ref()).collect();
    let clean = max_count > 0
        && stage_counts.iter().all(|&c| c * 5 >= max_count * 4)   // >=80% of max in every frame
        && imgs.len() == n_samples
        && imgs.windows(2).any(|w| w[0].png != w[1].png);          // some motion
    let stage_frames: Vec<StageArt> = if clean {
        imgs.into_iter().cloned().collect()
    } else {
        sampled[base_idx].1.clone().into_iter().collect()
    };

    StageArtSet { background, parallax, stage_frames, foreground }
}

/// Composite a set of art instances (back-to-front = walk order) into one RGBA image
/// spanning their union bounds. `None` if nothing rasterized.
fn composite_layer(
    art: &[&Instance],
    shape_defs: &BTreeMap<u16, &swf::Shape>,
    bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>,
    ox: f64, oy: f64,
) -> Option<StageArt> {
    use image::{imageops, RgbaImage};

    // The bitmap id a shape fills with, if any (stage backgrounds are bitmap-filled
    // rects that `vector_raster` skips; we blit the bitmap into the shape's AABB).
    let shape_bitmap = |sid: u16| -> Option<u16> {
        let shape = shape_defs.get(&sid)?;
        shape.styles.fill_styles.iter().find_map(|f| match f {
            swf::FillStyle::Bitmap { id, .. } if bitmaps.contains_key(id) => Some(*id),
            _ => None,
        })
    };

    if art.is_empty() { return None; }

    // union bounds (raw world coords) -> canvas. cap to keep the PNG sane.
    let min_x = art.iter().map(|i| i.aabb.left()).fold(f64::MAX, f64::min);
    let min_y = art.iter().map(|i| i.aabb.top()).fold(f64::MAX, f64::min);
    let max_x = art.iter().map(|i| i.aabb.right()).fold(f64::MIN, f64::max);
    let max_y = art.iter().map(|i| i.aabb.bottom()).fold(f64::MIN, f64::max);
    let cw = ((max_x - min_x).ceil() as u32).clamp(1, 4096);
    let ch = ((max_y - min_y).ceil() as u32).clamp(1, 4096);
    let mut canvas = RgbaImage::new(cw, ch);

    let tw_th = |i: &Instance| (
        (i.aabb.w.round() as u32).clamp(1, 4096),
        (i.aabb.h.round() as u32).clamp(1, 4096),
    );
    let mut drew = false;
    for inst in art {
        let (tw, th) = tw_th(inst);
        // The placed tile at the AABB size: a bitmap fill clipped to the shape OUTLINE, else
        // the vector rasterization.
        let scaled: Option<RgbaImage> = if let Some(bid) = shape_bitmap(inst.shape_id) {
            let (w, h, rgba) = bitmaps.get(&bid).unwrap();
            let Some(bmp) = RgbaImage::from_raw(*w, *h, rgba.clone()) else { continue };
            let mut bmp = imageops::resize(&bmp, tw, th, imageops::FilterType::Triangle);
            // clip the bitmap to the shape's path (not its rectangular AABB): rasterize the
            // path with the bitmap fill swapped for solid white, and use that as an alpha
            // mask. Without this an irregular bitmap-filled shape (e.g. a tree canopy) renders
            // as a hard rectangle.
            let shape = shape_defs.get(&inst.shape_id).unwrap();
            let mask_fills: Vec<swf::FillStyle> = shape.styles.fill_styles.iter().map(|f| match f {
                swf::FillStyle::Bitmap { .. } => swf::FillStyle::Color(swf::Color { r: 255, g: 255, b: 255, a: 255 }),
                other => other.clone(),
            }).collect();
            if let Some(mask) = crate::vector_raster::rasterize_shape(
                &shape.shape_bounds, &mask_fills, &shape.styles.line_styles, &shape.shape,
            ).and_then(|r| RgbaImage::from_raw(r.width, r.height, r.rgba)) {
                let mask = imageops::resize(&mask, tw, th, imageops::FilterType::Triangle);
                for (p, m) in bmp.pixels_mut().zip(mask.pixels()) {
                    p[3] = ((p[3] as u32 * m[3] as u32) / 255) as u8;
                }
            }
            Some(bmp)
        } else {
            let shape = shape_defs.get(&inst.shape_id).unwrap();
            crate::vector_raster::rasterize_shape(
                &shape.shape_bounds, &shape.styles.fill_styles, &shape.styles.line_styles, &shape.shape,
            ).and_then(|r| RgbaImage::from_raw(r.width, r.height, r.rgba))
                .map(|t| imageops::resize(&t, tw, th, imageops::FilterType::Triangle))
        };
        let Some(scaled) = scaled else { continue };
        if scaled.width() == 0 || scaled.height() == 0 { continue; }
        let ox_px = (inst.aabb.left() - min_x).round() as i64;
        let oy_px = (inst.aabb.top() - min_y).round() as i64;
        imageops::overlay(&mut canvas, &scaled, ox_px, oy_px);
        drew = true;
    }
    if !drew { return None; }

    let mut png = Vec::new();
    {
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(canvas.as_raw(), cw, ch, image::ExtendedColorType::Rgba8).ok()?;
    }
    Some(StageArt { png, x: min_x - ox, y: min_y - oy, w: cw, h: ch })
}

/// Trace a solid terrain shape's walkable TOP surface as a polyline in FM coords (left to
/// right). Rasterizes the shape and scans each column for the topmost opaque pixel, then maps
/// to world via the placed AABB and simplifies. `None` if the shape won't rasterize, is
/// essentially flat (so the level `rect.top()` line suffices), or maps to < 2 points. Maps
/// through the AABB, so it's exact for translated/scaled terrain; rotated terrain would skew
/// (rare for stages).
fn floor_profile(shape: &swf::Shape, aabb: &Rect, ox: f64, oy: f64, scale: f64) -> Option<Vec<(f64, f64)>> {
    use image::{imageops, RgbaImage};
    let raster = crate::vector_raster::rasterize_shape(
        &shape.shape_bounds, &shape.styles.fill_styles, &shape.styles.line_styles, &shape.shape,
    )?;
    let mask = RgbaImage::from_raw(raster.width, raster.height, raster.rgba)?;
    let w = (aabb.w.round() as u32).clamp(1, 4096);
    let h = (aabb.h.round() as u32).clamp(1, 4096);
    if w < 2 || h < 2 { return None; }
    let mask = imageops::resize(&mask, w, h, imageops::FilterType::Triangle);
    let step = (w / 96).max(1); // ~96 columns sampled across the span
    let mut pts: Vec<(f64, f64)> = Vec::new();
    let mut cx = 0u32;
    loop {
        let col = cx.min(w - 1);
        if let Some(ty) = (0..h).find(|&y| mask.get_pixel(col, y)[3] > 40) {
            pts.push((
                (aabb.left() + col as f64 - ox) * scale,
                (aabb.top() + ty as f64 - oy) * scale,
            ));
        }
        if col == w - 1 { break; }
        cx += step;
    }
    if pts.len() < 2 { return None; }
    let simplified = rdp_simplify(&pts, 2.0);
    // a (near-)flat profile adds nothing over the level floor line.
    let ys: Vec<f64> = simplified.iter().map(|p| p.1).collect();
    let span = ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min);
    if span < 6.0 { return None; }
    Some(simplified)
}

/// Ramer-Douglas-Peucker polyline simplification (perpendicular-distance epsilon in px).
fn rdp_simplify(pts: &[(f64, f64)], eps: f64) -> Vec<(f64, f64)> {
    if pts.len() < 3 { return pts.to_vec(); }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-9);
    let (mut max_d, mut idx) = (0.0_f64, 0);
    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let d = ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len;
        if d > max_d { max_d = d; idx = i; }
    }
    if max_d > eps {
        let mut left = rdp_simplify(&pts[..=idx], eps);
        let right = rdp_simplify(&pts[idx..], eps);
        left.pop();
        left.extend(right);
        left
    } else {
        vec![a, b]
    }
}

/// SSF2's auto pan multiplier for a camera-background layer of `w x h` native px, vs the
/// 640x360 game view (the SSF2 `Vcam` formula): `(size - view) / (2 * size)`, clamped to
/// `[0, 0.5]` (a layer <= the view is screen-fixed; a very wide layer approaches 0.5).
fn parallax_pan(w: u32, h: u32) -> (f64, f64) {
    let x = ((w as f64 - 640.0) / (2.0 * w.max(1) as f64)).clamp(0.0, 0.5);
    let y = ((h as f64 - 360.0) / (2.0 * h.max(1) as f64)).clamp(0.0, 0.5);
    (x, y)
}

/// Decompress an `.ssf` to SWF bytes via the DAT-archive aware path.
fn ssf_decompress(raw: &[u8], path: &Path) -> Result<Vec<u8>> {
    crate::ssf::decompress(raw).with_context(|| format!("decompress {}", path.display()))
}

/// Read the stage DAT's `Main` package metadata (`id`, `guid`, `music`) from the first
/// ABC block that carries an `id` (the content id the engine knows the stage by).
fn stage_package_metadata(swf_data: &[u8]) -> Option<crate::abc_parser::MainPackageMetadata> {
    let swf = crate::swf_parser::parse(swf_data).ok()?;
    for abc_bytes in &swf.abc_blocks {
        if let Ok(abc) = crate::abc_parser::parse(abc_bytes) {
            if let Some(md) = crate::abc_parser::extract_main_package_metadata(&abc) {
                if md.id.is_some() { return Some(md); }
            }
        }
    }
    None
}

/// Title-case an SSF2 lowercase-concatenated id for a display name: capitalize the first
/// letter and any after an underscore/space ("battlefield" -> "Battlefield"). Multi-word
/// ids that aren't underscore-separated stay one word; add an override in the metadata map.
fn title_case(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    let mut cap = true;
    for c in id.chars() {
        if c == '_' || c == '-' { out.push(' '); cap = true; }
        else if cap { out.extend(c.to_uppercase()); cap = false; }
        else { out.push(c); }
    }
    out
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
    rec: usize, carried_sym: Option<&str>, plane: Option<&str>, moving_anc: bool,
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
        let my_plane = plane_tag(inst_name.as_deref(), &sym).or(plane);
        // sticky once any ancestor (or this node) is `moving`-named — the collision child
        // is named separately, so the signal must flow down the subtree.
        let here_moving = moving_anc
            || inst_name.as_deref().is_some_and(|n| n.to_ascii_lowercase().contains("moving"))
            || sym.to_ascii_lowercase().contains("moving");

        if std::env::var("PEPTIDE_STAGE_TREE").is_ok() {
            let kind = if sprites.contains_key(&id) { "MC" } else if shape_bounds.contains_key(&id) { "shape" } else { "?" };
            eprintln!("{}d{} {kind} inst={:?} sym={:?} plane={:?} @({:.0},{:.0})",
                "  ".repeat(rec), po.depth, inst_name.as_deref().unwrap_or(""), sym, my_plane.unwrap_or(""), world.tx, world.ty);
        }

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
                shape_id: id,
                inst_name: inst_name.clone(),
                sym_name: carried_sym.unwrap_or("").to_string(),
                plane: my_plane.map(str::to_string),
                aabb: Rect { x: xmn, y: ymn, w: xmx - xmn, h: ymx - ymn },
                cx: world.tx, cy: world.ty,
                x_sign: world.x_sign(),
                moving: here_moving,
            });
        }
        if let Some(child) = sprites.get(&id) {
            // carry the most specific identity: instance name wins, else symbol class,
            // else inherit the parent's carried symbol.
            let next = inst_name.as_deref().or(if sym.is_empty() { carried_sym } else { Some(&sym) });
            let next = next.map(|s| s.to_string());
            walk(child, world, sym_names, shape_bounds, sprites, rec + 1, next.as_deref(), my_plane, here_moving, out, origin);
        }
    }
}
