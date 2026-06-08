//! stage_emit — emit a Fraymakers stage package from a parsed [`StageModel`].
//!
//! Mirrors the converted-character layout but for a `type:"stage"` resource:
//! ```text
//! stages/<id>/
//!   <id>.fraytools
//!   library/
//!     manifest.json (+ .meta)
//!     entities/<id>.entity
//!     scripts/stage/<id>Script.hx (+ .meta)
//!     scripts/stage/<id>StageStats.hx (+ .meta)
//! ```
//! The `.entity` is the normalized FrayTools graph (symbols + keyframes + layers +
//! one `stage` animation) reversed from the public `Fraymakers/stage-template`:
//! COLLISION_BOX death/camera boxes, LINE_SEGMENT floor + soft platforms,
//! ENTRANCE/RESPAWN points, and an IMAGE layer carrying the stage sprite (the SSF2
//! vector art rasterized + composited by the parser, or a geometry placeholder for
//! bitmap-only stages). Parallax backgrounds + hazards are follow-ups.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::stage_parser::{ParallaxMode, Rect, StageArt, StageModel};
use crate::uuid_gen::det_uuid;

/// Emit the FM stage package for `model` under `out_root/<id>/`. Returns the
/// project dir and the `.fraytools` path (for the FrayTools publish step).
pub fn emit_stage(model: &StageModel, out_root: &Path) -> Result<(PathBuf, PathBuf)> {
    let id = &model.id;
    let dir = out_root.join(id);
    let lib = dir.join("library");
    std::fs::create_dir_all(lib.join("entities")).context("mkdir entities")?;
    std::fs::create_dir_all(lib.join("scripts").join("stage")).context("mkdir scripts/stage")?;

    // Stage art — split into depth layers: a parallax background, the main stage at
    // character depth, and a foreground in front of fighters. A stage needs visible
    // content to be playable (the engine sizes the stage + places players from the sprite
    // bounds), so if nothing rasterized we fall back to a geometry placeholder as the
    // stage layer. Each layer is a PNG + `.meta`; the `.png` is gitignored (regenerated).
    let sprites = lib.join("sprites").join("Stage");
    std::fs::create_dir_all(&sprites).context("mkdir sprites/Stage")?;
    let write_layer = |suffix: &str, art: &StageArt| -> Result<ArtRef> {
        let guid = det_uuid(&format!("stage::{id}::{suffix}"));
        std::fs::write(sprites.join(format!("{id}_{suffix}.png")), &art.png)?;
        write_json(&sprites.join(format!("{id}_{suffix}.png.meta")), &json!({
            "export": false, "guid": guid, "id": "", "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
        }))?;
        Ok(ArtRef { guid, x: art.x, y: art.y, w: art.w, h: art.h })
    };
    let stage_fallback;
    let stage_frames: Vec<&StageArt> = if !model.art.stage_frames.is_empty() {
        model.art.stage_frames.iter().collect()
    } else if model.art.is_empty() {
        stage_fallback = render_placeholder(model);
        vec![&stage_fallback]
    } else {
        vec![]
    };
    let stage_refs: Vec<ArtRef> = stage_frames.iter().enumerate()
        .map(|(i, a)| write_layer(&format!("stage{i}"), a))
        .collect::<Result<_>>()?;
    let parallax_refs: Vec<ParallaxRef> = model.art.parallax.iter().enumerate()
        .map(|(i, layer)| write_layer(&format!("parallax{i}"), &layer.art)
            .map(|r| ParallaxRef { art: r, mode: layer.mode, x_pan: layer.x_pan, y_pan: layer.y_pan }))
        .collect::<Result<_>>()?;
    let art = ArtRefs {
        background: model.art.background.as_ref().map(|a| write_layer("bg", a)).transpose()?,
        parallax: parallax_refs,
        stage: stage_refs,
        foreground: model.art.foreground.as_ref().map(|a| write_layer("fg", a)).transpose()?,
    };

    let entity = build_entity(model, &art);
    // FM-side validation: the built entity must carry the layers a Fraymakers stage needs
    // (these are invariants of build_entity; the check guards against regressions).
    let missing = validate_fm_entity(&entity);
    if !missing.is_empty() {
        bail!("emitted {id}.entity is missing required Fraymakers stage layers: {}", missing.join(", "));
    }
    write_json(&lib.join("entities").join(format!("{id}.entity")), &entity)?;

    // hazards (custom game objects) the stage spawns, if any are declared for this stage.
    let hazard_entries = emit_hazards(model, &lib)?;

    write_json(&lib.join("manifest.json"), &build_manifest(model, &hazard_entries))?;
    write_meta(&lib.join("manifest.json.meta"), id, "manifest", "json", None, None)?;

    let scripts = lib.join("scripts").join("stage");
    std::fs::write(scripts.join(format!("{id}Script.hx")), script_hx(id, art.stage.len() > 1, &hazard_spawn_lines(model)))?;
    write_meta(&scripts.join(format!("{id}Script.hx.meta")), id, &format!("{id}Script"), "", Some("STAGE"), None)?;
    std::fs::write(scripts.join(format!("{id}StageStats.hx")), stage_stats_hx(id, &art.parallax, model.scale))?;
    write_meta(&scripts.join(format!("{id}StageStats.hx.meta")), id, &format!("{id}StageStats"), "hscript", None, None)?;

    let fraytools = dir.join(format!("{id}.fraytools"));
    write_json(&fraytools, &build_fraytools())?;

    Ok((dir, fraytools))
}

fn write_json(path: &Path, v: &Value) -> Result<()> {
    std::fs::write(path, serde_json::to_string_pretty(v)?).with_context(|| format!("write {}", path.display()))
}

// ─────────────────────────────── the .entity ────────────────────────────────

/// Builders accumulate symbols/keyframes/layers; each `add_*` returns the layer
/// `$id` so the animation can list them in order.
struct EntityBuilder<'a> {
    id: &'a str,
    seq: usize,
    symbols: Vec<Value>,
    keyframes: Vec<Value>,
    layers: Vec<Value>,
    /// Layer ids of the `stage` animation, in render order (first = back).
    anim_layers: Vec<String>,
    /// One `(animationName, layerId)` per parallax camera-background layer (`parallax0`,
    /// `parallax1`, …) — each `_cambg` layer scrolls at its own rate, so each is its own.
    parallax_anims: Vec<(String, String)>,
    /// Length of the stage animation in frames (static layers hold for this many).
    frame_len: usize,
    /// SSF2 -> FM art scale (`size_multiplier`): native-resolution art PNGs render at this
    /// scale so they match the scaled-up geometry + fighters.
    scale: f64,
}

impl<'a> EntityBuilder<'a> {
    fn new(id: &'a str) -> Self {
        EntityBuilder { id, seq: 0, symbols: vec![], keyframes: vec![], layers: vec![], anim_layers: vec![], parallax_anims: vec![], frame_len: 1, scale: 1.0 }
    }
    /// A stable per-entity uuid for `role` (e.g. `"layer:Floor"`).
    fn uid(&mut self, role: &str) -> String {
        self.seq += 1;
        det_uuid(&format!("stage::{}::{}::{}", self.id, role, self.seq))
    }

    /// A CONTAINER layer (Characters / effects depth groups). No symbol.
    fn add_container(&mut self, name: &str, container_type: &str) {
        let kf = self.uid(&format!("kf:{name}"));
        self.keyframes.push(json!({ "$id": kf, "length": self.frame_len, "pluginMetadata": {}, "type": "CONTAINER" }));
        let lid = self.uid(&format!("layer:{name}"));
        self.layers.push(json!({
            "$id": lid, "hidden": false, "locked": false, "name": name, "type": "CONTAINER",
            "keyframes": [kf],
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "containerType": container_type } }
        }));
        self.anim_layers.push(lid);
    }

    /// A COLLISION_BOX layer (death / camera box). `rect` in FM coords.
    fn add_collision_box(&mut self, name: &str, box_type: &str, rect: &Rect) {
        let sym = self.uid(&format!("sym:{name}"));
        self.symbols.push(json!({
            "$id": sym, "type": "COLLISION_BOX", "alpha": null, "color": null, "pluginMetadata": {},
            "x": rect.left(), "y": rect.top(), "scaleX": rect.w, "scaleY": rect.h,
            "pivotX": rect.w / 2.0, "pivotY": 0, "rotation": 0
        }));
        let kf = self.uid(&format!("kf:{name}"));
        self.keyframes.push(json!({ "$id": kf, "length": self.frame_len, "pluginMetadata": {}, "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "COLLISION_BOX" }));
        let lid = self.uid(&format!("layer:{name}"));
        self.layers.push(json!({
            "$id": lid, "hidden": false, "locked": false, "name": name, "type": "COLLISION_BOX",
            "defaultAlpha": 0.5, "defaultColor": "0xd1d1d1", "keyframes": [kf],
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "collisionBoxType": box_type } }
        }));
        self.anim_layers.push(lid);
    }


    /// A LINE_SEGMENT layer (walkable surface). `points` is the polyline (>= 2 points; a
    /// flat floor is 2, a curved/sloped terrain traces its surface). `pm` is the per-symbol
    /// FraymakersMetadata (structureType + ledge/dropThrough flags).
    fn add_line_segment(&mut self, name: &str, points: &[(f64, f64)], pm: Value) {
        let flat: Vec<f64> = points.iter().flat_map(|&(x, y)| [x, y]).collect();
        let sym = self.uid(&format!("sym:{name}"));
        self.symbols.push(json!({
            "$id": sym, "type": "LINE_SEGMENT", "alpha": 0.5, "color": "0xeeeeee",
            "points": flat,
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": pm }
        }));
        let kf = self.uid(&format!("kf:{name}"));
        self.keyframes.push(json!({ "$id": kf, "length": self.frame_len, "pluginMetadata": {}, "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "LINE_SEGMENT" }));
        let lid = self.uid(&format!("layer:{name}"));
        self.layers.push(json!({
            "$id": lid, "hidden": false, "locked": false, "name": name, "type": "LINE_SEGMENT",
            "keyframes": [kf],
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "lineSegmentType": "LINE_SEGMENT_STRUCTURE" } }
        }));
        self.anim_layers.push(lid);
    }

    /// An IMAGE layer referencing a sprite by its `.meta` guid (`image_asset`), placed
    /// at `(x, y)` in stage coords. A stage MUST have visible content: the engine sizes
    /// the stage sprite from its image bounds during match setup, and a stage with no
    /// IMAGE never places a player (verified live). So even the geometry MVP emits a
    /// placeholder sprite.
    /// Create an IMAGE layer (in the entity pools) at `img_scale` and return its layer id.
    /// The caller pushes the id into whichever animation it belongs to (stage vs parallax).
    fn make_image(&mut self, name: &str, image_asset: &str, x: f64, y: f64, img_scale: f64) -> String {
        let sym = self.uid(&format!("sym:{name}"));
        self.symbols.push(json!({
            "$id": sym, "type": "IMAGE", "imageAsset": image_asset, "alpha": 1,
            "x": x, "y": y, "scaleX": img_scale, "scaleY": img_scale, "rotation": 0, "pivotX": 0, "pivotY": 0,
            "pluginMetadata": {}
        }));
        let kf = self.uid(&format!("kf:{name}"));
        self.keyframes.push(json!({ "$id": kf, "length": self.frame_len, "pluginMetadata": {}, "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" }));
        let lid = self.uid(&format!("layer:{name}"));
        self.layers.push(json!({
            "$id": lid, "hidden": false, "locked": false, "name": name, "type": "IMAGE",
            "keyframes": [kf], "pluginMetadata": {}
        }));
        lid
    }
    /// Add an IMAGE layer to the `stage` animation at the current depth (rendered at the
    /// stage `scale` so the native-resolution art matches the scaled-up geometry).
    fn add_image(&mut self, name: &str, image_asset: &str, x: f64, y: f64) {
        let lid = self.make_image(name, image_asset, x, y, self.scale);
        self.anim_layers.push(lid);
    }
    /// Add a parallax camera-background layer as its own `parallax{idx}` animation. The IMAGE
    /// symbol stays at scale 1: the camera's ParallaxBG sizes it from
    /// `originalBGWidth × scaleMultiplier` (set in StageStats), so scaling the symbol too
    /// would double it.
    fn add_image_parallax(&mut self, idx: usize, image_asset: &str, x: f64, y: f64) {
        let lid = self.make_image(&format!("Parallax {idx}"), image_asset, x, y, 1.0);
        self.parallax_anims.push((format!("parallax{idx}"), lid));
    }
    /// Add an animated IMAGE layer to the `stage` animation: one keyframe per frame, each
    /// referencing that frame's image, so the layer plays through them (loops).
    fn add_image_frames(&mut self, name: &str, frames: &[(String, f64, f64)]) {
        let mut kfs = Vec::new();
        for (i, (guid, x, y)) in frames.iter().enumerate() {
            let sym = self.uid(&format!("sym:{name}:{i}"));
            self.symbols.push(json!({
                "$id": sym, "type": "IMAGE", "imageAsset": guid, "alpha": 1,
                "x": x, "y": y, "scaleX": self.scale, "scaleY": self.scale, "rotation": 0, "pivotX": 0, "pivotY": 0,
                "pluginMetadata": {}
            }));
            let kf = self.uid(&format!("kf:{name}:{i}"));
            self.keyframes.push(json!({ "$id": kf, "length": 1, "pluginMetadata": {}, "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" }));
            kfs.push(kf);
        }
        let lid = self.uid(&format!("layer:{name}"));
        self.layers.push(json!({
            "$id": lid, "hidden": false, "locked": false, "name": name, "type": "IMAGE",
            "keyframes": kfs, "pluginMetadata": {}
        }));
        self.anim_layers.push(lid);
    }

    /// A POINT layer (entrance / respawn). `point_type` = ENTRANCE_POINT|RESPAWN_POINT.
    fn add_point(&mut self, name: &str, point_type: &str, index: usize, x: f64, y: f64, rotation: i64) {
        let sym = self.uid(&format!("sym:{name}"));
        self.symbols.push(json!({
            "$id": sym, "type": "POINT", "alpha": 1, "color": "0xff0000", "pluginMetadata": {},
            "x": x, "y": y, "rotation": rotation
        }));
        let kf = self.uid(&format!("kf:{name}"));
        self.keyframes.push(json!({ "$id": kf, "length": self.frame_len, "pluginMetadata": {}, "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "POINT" }));
        let lid = self.uid(&format!("layer:{name}"));
        self.layers.push(json!({
            "$id": lid, "hidden": false, "locked": false, "name": name, "type": "POINT",
            "keyframes": [kf],
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "pointType": point_type, "index": index } }
        }));
        self.anim_layers.push(lid);
    }
}

/// A written art layer the entity references: the sprite `.meta` guid + placement + size.
struct ArtRef { guid: String, x: f64, y: f64, w: u32, h: u32 }

/// A parallax camera-background layer: the written sprite + its scroll mode + pan rate.
struct ParallaxRef { art: ArtRef, mode: ParallaxMode, x_pan: f64, y_pan: f64 }

/// The depth layers the entity lays out. `stage` is the frame sequence (1 = static).
struct ArtRefs { background: Option<ArtRef>, parallax: Vec<ParallaxRef>, stage: Vec<ArtRef>, foreground: Option<ArtRef> }

/// Render the floor + soft platforms as filled rectangles on a transparent canvas
/// covering their bounding box (1px = 1 stage unit). Gives the stage visible content
/// (required for play) and shows the playable geometry.
fn render_placeholder(model: &StageModel) -> StageArt {
    use image::{Rgba, RgbaImage};
    // bounding box of all collision geometry, with a small margin.
    let rects: Vec<Rect> = model.platforms.iter().map(|p| p.rect).collect();
    let margin = 8.0;
    let min_x = rects.iter().map(|r| r.left()).fold(f64::MAX, f64::min) - margin;
    let min_y = rects.iter().map(|r| r.top()).fold(f64::MAX, f64::min) - margin;
    let max_x = rects.iter().map(|r| r.right()).fold(f64::MIN, f64::max) + margin;
    let max_y = rects.iter().map(|r| r.bottom()).fold(f64::MIN, f64::max) + margin;
    let w = ((max_x - min_x).ceil() as u32).clamp(1, 4096);
    let h = ((max_y - min_y).ceil() as u32).clamp(1, 4096);
    let mut img = RgbaImage::new(w, h);
    for p in &model.platforms {
        let r = &p.rect;
        let color = if p.drop_through { Rgba([120, 160, 220, 235]) } else { Rgba([90, 100, 120, 255]) };
        let x0 = ((r.left() - min_x).max(0.0)) as u32;
        let y0 = ((r.top() - min_y).max(0.0)) as u32;
        let x1 = ((r.right() - min_x).min(w as f64)) as u32;
        let y1 = ((r.bottom() - min_y).min(h as f64)) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x, y, color);
            }
        }
    }
    let mut png = Vec::new();
    {
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
            .expect("encode placeholder png");
    }
    StageArt { png, x: min_x, y: min_y, w, h }
}

fn build_entity(model: &StageModel, art: &ArtRefs) -> Value {
    let id = &model.id;
    let mut b = EntityBuilder::new(id);
    b.scale = model.scale;
    // the stage animation runs for as many frames as the stage art has (1 = static);
    // static layers hold across all of them.
    b.frame_len = art.stage.len().max(1);

    // ── render order (first = back): the painted backdrop, background depth containers,
    // the stage art (behind fighters), the character containers, the foreground art (in
    // front of fighters), the foreground containers, then the invisible collision / spawns.
    // The backdrop is FIXED, not parallax-scrolled: the SSF2 `<id>_bg` plane includes the
    // surface fighters stand on, so it has to stay aligned with the collision. ──
    if let Some(a) = &art.background { b.add_image("Background Art", &a.guid, a.x, a.y); }
    b.add_container("Background Behind", "BACKGROUND_BEHIND_CONTAINER");
    b.add_container("Background Effects", "BACKGROUND_EFFECTS_CONTAINER");
    b.add_container("Background Shadows", "BACKGROUND_SHADOWS_CONTAINER");
    b.add_container("Background Structures", "BACKGROUND_STRUCTURES_CONTAINER");
    if !art.stage.is_empty() {
        let frames: Vec<(String, f64, f64)> = art.stage.iter().map(|a| (a.guid.clone(), a.x, a.y)).collect();
        b.add_image_frames("Stage Art", &frames);
    }
    b.add_container("Characters Back", "CHARACTERS_BACK_CONTAINER");
    b.add_container("Characters", "CHARACTERS_CONTAINER");
    b.add_container("Characters Front", "CHARACTERS_FRONT_CONTAINER");
    if let Some(a) = &art.foreground { b.add_image("Foreground Art", &a.guid, a.x, a.y); }
    b.add_container("Foreground Structures", "FOREGROUND_STRUCTURES_CONTAINER");
    b.add_container("Foreground Shadows", "FOREGROUND_SHADOWS_CONTAINER");
    b.add_container("Foreground Effects", "FOREGROUND_EFFECTS_CONTAINER");
    b.add_container("Foreground Front", "FOREGROUND_FRONT_CONTAINER");

    // boundaries: blast zone + hard camera bounds. NO camera-anchor box: the engine adds an
    // anchor to the camera's target bounds, so any anchor forces the camera to zoom out to
    // keep it framed. A blast-zone-sized anchor pins the camera at max zoom-out and the art
    // never fills the view. Without one, the camera frames the players within the camera box
    // and sits at the StageStats `minZoomHeight` floor, where the scaled backdrop fills, the
    // same as a FM-native stage.
    if let Some(r) = &model.death_box { b.add_collision_box("Death Box", "DEATH_BOX", r); }
    if let Some(r) = &model.camera_box {
        b.add_collision_box("Camera Box", "CAMERA_BOX", r);
    }

    // collision: main floor (solid, with ledges at the SSF2 ledge positions) + soft
    // platforms (drop-through). Ledges grab at the floor's left/right endpoints.
    let main_floor = model.main_floor().cloned();
    let mut plat_n = 0usize;
    for p in &model.platforms {
        let r = &p.rect;
        let (lx, rx) = model.ledges
            .filter(|_| main_floor.as_ref().map(|m| m.rect == *r).unwrap_or(false))
            .unwrap_or((r.left(), r.right()));
        // a curved/sloped terrain traces its top surface as a polyline; a flat one is the
        // level top line. ledges anchor at the surface endpoints.
        let surface = |default: Vec<(f64, f64)>| -> Vec<(f64, f64)> {
            p.profile.clone().filter(|pf| pf.len() >= 2).unwrap_or(default)
        };
        if !p.drop_through && main_floor.as_ref().map(|m| m.rect == *r).unwrap_or(false) {
            b.add_line_segment("Floor", &surface(vec![(lx, r.top()), (rx, r.top())]), json!({
                "structureType": "FLOOR", "leftLedge": true, "rightLedge": true, "dropThrough": false
            }));
        } else if p.drop_through {
            plat_n += 1;
            // tag moving platforms in the label so an author opening the entity in FrayTools
            // can find the ones meant to move (their motion isn't ported, see StageModel.moving).
            let tag = if p.moving { " (SSF2 moving, static)" } else { "" };
            b.add_line_segment(&format!("Platform {plat_n} Floor{tag}"), &[(r.left(), r.top()), (r.right(), r.top())], json!({
                "structureType": "FLOOR", "leftLedge": false, "rightLedge": false, "dropThrough": true
            }));
        } else {
            plat_n += 1;
            let tag = if p.moving { " (SSF2 moving, static)" } else { "" };
            b.add_line_segment(&format!("Solid {plat_n} Floor{tag}"), &surface(vec![(r.left(), r.top()), (r.right(), r.top())]), json!({
                "structureType": "FLOOR", "leftLedge": false, "rightLedge": false, "dropThrough": false
            }));
        }
    }

    // spawns: entrances (match start) + respawns. Fill 4 of each (the engine expects
    // a full set); if SSF2 declared fewer, fall back to the main floor center.
    let floor_cx = main_floor.as_ref().map(|m| m.rect.left() + m.rect.w / 2.0).unwrap_or(0.0);
    let floor_top = main_floor.as_ref().map(|m| m.rect.top()).unwrap_or(0.0);
    for i in 0..4usize {
        let (x, y, rot) = model.entrances.iter().find(|s| s.index == i)
            .map(|s| (s.x, s.y, if s.face_left { 270 } else { 90 }))
            .unwrap_or((floor_cx + (i as f64 - 1.5) * 60.0, floor_top - 40.0, 90));
        b.add_point(&format!("Entrance {i}"), "ENTRANCE_POINT", i, x, y, rot);
    }
    for i in 0..4usize {
        let (x, y, rot) = model.respawns.iter().find(|s| s.index == i)
            .map(|s| (s.x, s.y, if s.face_left { 270 } else { 90 }))
            .unwrap_or((floor_cx + (i as f64 - 1.5) * 60.0, floor_top - 200.0, 90));
        b.add_point(&format!("Respawn {i}"), "RESPAWN_POINT", i, x, y, rot);
    }

    // each SSF2 camera-background layer -> its own `parallax{i}` animation (each scrolls at its
    // own rate, set in StageStats).
    for (i, p) in art.parallax.iter().enumerate() {
        b.add_image_parallax(i, &p.art.guid, p.art.x, p.art.y);
    }

    let mut animations = vec![json!({
        "$id": b.uid("anim:stage"), "name": "stage", "pluginMetadata": {}, "layers": b.anim_layers
    })];
    for (name, lid) in b.parallax_anims.clone() {
        let aid = b.uid(&format!("anim:{name}"));
        animations.push(json!({ "$id": aid, "name": name, "pluginMetadata": {}, "layers": [lid] }));
    }

    json!({
        "version": 14,
        "id": id,
        "guid": det_uuid(&format!("stage::{id}::entity")),
        "export": true,
        "plugins": ["com.fraymakers.FraymakersMetadata"],
        "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "objectType": "STAGE", "version": "0.4.0" } },
        "symbols": b.symbols,
        "keyframes": b.keyframes,
        "layers": b.layers,
        "animations": animations,
        "tags": [],
        "terrains": [],
        "tilesets": [],
        "paletteMap": { "paletteCollection": null, "paletteMap": null }
    })
}

/// The Fraymakers stage layers a playable `.entity` must carry. Returns the names of any
/// that are missing (empty = valid). Mirrors the required-linkage check on the SSF2 side:
/// a stage needs a Characters container, a walkable floor, spawn/respawn points, and a
/// visible IMAGE (the engine sizes the stage + places fighters from the sprite bounds).
fn validate_fm_entity(entity: &Value) -> Vec<String> {
    let empty = vec![];
    let layers = entity.get("layers").and_then(|l| l.as_array()).unwrap_or(&empty);
    let meta = |l: &Value, key: &str| -> Option<String> {
        l.pointer(&format!("/pluginMetadata/com.fraymakers.FraymakersMetadata/{key}"))
            .and_then(|v| v.as_str()).map(str::to_string)
    };
    let has_container = |ct: &str| layers.iter().any(|l| meta(l, "containerType").as_deref() == Some(ct));
    let has_point = |pt: &str| layers.iter().any(|l| meta(l, "pointType").as_deref() == Some(pt));
    let has_type = |ty: &str| layers.iter().any(|l| l.get("type").and_then(|v| v.as_str()) == Some(ty));

    let mut missing = Vec::new();
    if !has_container("CHARACTERS_CONTAINER") { missing.push("Characters container".to_string()); }
    if !has_type("LINE_SEGMENT") { missing.push("a floor line segment".to_string()); }
    if !has_point("ENTRANCE_POINT") { missing.push("an entrance point".to_string()); }
    if !has_point("RESPAWN_POINT") { missing.push("a respawn point".to_string()); }
    if !has_type("IMAGE") { missing.push("a visible IMAGE layer".to_string()); }
    missing
}

// ───────────────────────── manifest / scripts / project ─────────────────────

// ───────────────────────── stage hazards (custom game objects) ──────────────
// Each declared hazard (mappings/stage/metadata.jsonc) becomes a Fraymakers CUSTOM_GAME_OBJECT:
// an entity with a sprite + a HIT_BOX, a HitboxStats giving the damage, a GameObjectStats, and a
// Script that keeps it active. The stage Script spawns each via match.createCustomGameObject.
// SSF2 hazards are bespoke per stage, so this is the framework + an author-editable hazard, not
// an auto-port of the original behavior.

use crate::stage_parser::Hazard;

/// camelCase content id for hazard `idx` of stage `id` (e.g. `battlefieldssf2hazard0`).
fn hazard_id(stage_id: &str, idx: usize) -> String { format!("{stage_id}hazard{idx}") }

/// Emit every hazard custom game object (entity + Script/GameObjectStats/HitboxStats/
/// AnimationStats + sprite) under the library. Returns the manifest content entries.
fn emit_hazards(model: &StageModel, lib: &Path) -> Result<Vec<Value>> {
    let mut entries = Vec::new();
    if model.hazards.is_empty() { return Ok(entries); }
    let sprites = lib.join("sprites").join("Hazard");
    let scripts = lib.join("scripts").join("hazard");
    std::fs::create_dir_all(&sprites).context("mkdir sprites/Hazard")?;
    std::fs::create_dir_all(&scripts).context("mkdir scripts/hazard")?;

    for (i, hz) in model.hazards.iter().enumerate() {
        let hid = hazard_id(&model.id, i);
        // a translucent red hitbox-volume sprite (w x h).
        let (w, h) = (hz.w.max(8.0) as u32, hz.h.max(8.0) as u32);
        let mut img = image::RgbaImage::new(w, h);
        for px in img.pixels_mut() { *px = image::Rgba([220, 40, 40, 130]); }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .context("encode hazard png")?;
        let sprite_guid = det_uuid(&format!("hazard::{hid}::sprite"));
        std::fs::write(sprites.join(format!("{hid}.png")), &png)?;
        write_json(&sprites.join(format!("{hid}.png.meta")), &json!({
            "export": false, "guid": sprite_guid, "id": "", "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
        }))?;

        write_json(&lib.join("entities").join(format!("{hid}.entity")), &hazard_entity(&hid, hz, &sprite_guid))?;
        write_meta(&lib.join("entities").join(format!("{hid}.entity.meta")), &hid, &hid, "", Some("CUSTOM_GAME_OBJECT"), None)?;

        let files = [
            ("Script", hazard_script_hx(hz)),
            ("GameObjectStats", hazard_gameobject_stats_hx(&hid)),
            ("HitboxStats", hazard_hitbox_stats_hx(hz)),
            ("AnimationStats", hazard_animation_stats_hx()),
        ];
        for (kind, body) in files {
            let fname = format!("{hid}{kind}");
            std::fs::write(scripts.join(format!("{fname}.hx")), body)?;
            write_meta(&scripts.join(format!("{fname}.hx.meta")), &hid, &fname,
                if kind == "Script" { "" } else { "hscript" },
                if kind == "Script" { Some("CUSTOM_GAME_OBJECT") } else { None }, None)?;
        }
        entries.push(json!({
            "type": "customGameObject", "id": hid,
            "scriptId": format!("{hid}Script"),
            "objectStatsId": format!("{hid}GameObjectStats"),
            "hitboxStatsId": format!("{hid}HitboxStats"),
            "animationStatsId": format!("{hid}AnimationStats"),
            "name": hz.label.clone(),
        }));
    }
    Ok(entries)
}

/// The CUSTOM_GAME_OBJECT entity: one `gameObjectIdle` animation with an IMAGE + a HIT_BOX
/// (index 0), sized to the hazard. Mirrors a projectile entity (the proven hitbox carrier).
fn hazard_entity(hid: &str, hz: &Hazard, sprite_guid: &str) -> Value {
    let g = |s: &str| det_uuid(&format!("hazard::{hid}::{s}"));
    let (img_sym, box_sym) = (g("imgsym"), g("boxsym"));
    let (img_layer, box_layer) = (g("imglayer"), g("boxlayer"));
    let (img_kf, box_kf) = (g("imgkf"), g("boxkf"));
    let (hw, hh) = (hz.w / 2.0, hz.h / 2.0);
    json!({
        "export": true, "guid": g("entity"), "id": hid, "version": 5,
        "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "objectType": "CUSTOM_GAME_OBJECT", "version": "0.4.0" } },
        "plugins": ["com.fraymakers.FraymakersTypes", "com.fraymakers.FraymakersMetadata"],
        "tags": [], "paletteMap": {}, "tilesets": [], "terrains": [],
        "symbols": [
            { "$id": img_sym, "type": "IMAGE", "imageAsset": sprite_guid, "x": -hw, "y": -hh, "pivotX": 0.0, "pivotY": 0.0, "scaleX": 1.0, "scaleY": 1.0, "rotation": 0.0, "alpha": 1.0, "pluginMetadata": {} },
            { "$id": box_sym, "type": "COLLISION_BOX", "x": -hw, "y": -hh, "pivotX": hw, "pivotY": hh, "scaleX": hz.w, "scaleY": hz.h, "rotation": 0.0, "alpha": 0.5, "color": "0xff0000", "pluginMetadata": {} }
        ],
        "keyframes": [
            { "$id": img_kf, "symbol": img_sym, "length": 1, "tweened": false, "pluginMetadata": {} },
            { "$id": box_kf, "symbol": box_sym, "length": 1, "tweened": false, "pluginMetadata": {} }
        ],
        "layers": [
            { "$id": img_layer, "name": "art", "type": "IMAGE", "hidden": false, "locked": false, "keyframes": [img_kf], "pluginMetadata": {} },
            { "$id": box_layer, "name": "hitbox0", "type": "COLLISION_BOX", "hidden": false, "locked": false, "defaultAlpha": 0.5, "defaultColor": "0xff0000", "keyframes": [box_kf],
              "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "collisionBoxType": "HIT_BOX", "index": 0 } } }
        ],
        "animations": [
            { "$id": g("anim"), "name": "gameObjectIdle", "layers": [img_layer, box_layer], "pluginMetadata": {} }
        ]
    })
}

fn hazard_script_hx(hz: &Hazard) -> String {
    // The hitbox fires automatically while `gameObjectIdle` plays (the HIT_BOX layer + the
    // HitboxStats entry). For a pulsing hazard, stop/restart the animation on the duty cycle.
    let pulse = if hz.interval > 0 {
        format!(
            "\tvar t = self.getAnimationFrame();\n\
             \tif (t % {interval} == 0) {{ self.playAnimation(\"gameObjectIdle\"); }}\n\
             \telse if (t % {interval} == {active}) {{ self.stopAnimation(); }}\n",
            interval = hz.interval, active = hz.active)
    } else {
        String::new()
    };
    format!(
        "// Stage hazard (custom game object) — converted from SSF2 (bespoke behavior not ported).\n\
         // Edit this to give the hazard real movement/behavior; the damage comes from HitboxStats.\n\n\
         function initialize() {{\n\tself.playAnimation(\"gameObjectIdle\");\n}}\n\n\
         function update() {{\n{pulse}}}\n")
}

fn hazard_gameobject_stats_hx(hid: &str) -> String {
    format!(
        "// GameObjectStats for {hid}\n{{\n\tspriteContent: \"{hid}\",\n\tinitialState: 0,\n\
         \tbaseScaleX: 1,\n\tbaseScaleY: 1,\n\tweight: 100,\n\tgravity: 0,\n\tfriction: 0\n}}\n")
}

fn hazard_hitbox_stats_hx(hz: &Hazard) -> String {
    format!(
        "// HitboxStats for the stage hazard. damage/knockback/angle from mappings/stage/metadata.jsonc.\n\
         {{\n\tgameObjectIdle: {{\n\t\thitbox0: {{ damage: {}, angle: {}, baseKnockback: {}, knockbackGrowth: 30, hitstun: 1.0 }}\n\t}}\n}}\n",
        hz.damage, hz.angle, hz.knockback)
}

fn hazard_animation_stats_hx() -> String {
    "// AnimationStats for the stage hazard.\n{\n\tgameObjectIdle: { endType: AnimationEndType.NONE }\n}\n".to_string()
}

/// hscript the stage Script runs to spawn its hazards (createCustomGameObject + position).
fn hazard_spawn_lines(model: &StageModel) -> String {
    let mut out = String::new();
    for (i, hz) in model.hazards.iter().enumerate() {
        let hid = hazard_id(&model.id, i);
        // owner is null: the spawner must be a GameObjectApi and a stage's `self` is a StageApi
        // (cast fails); a hazard belongs to no fighter, so null owner is correct.
        out.push_str(&format!(
            "\tvar _hz{i} = match.createCustomGameObject(self.getResource().getContent(\"{hid}\"), null);\n\
             \tif (_hz{i} != null) {{ _hz{i}.setX({:.1}); _hz{i}.setY({:.1}); }}\n",
            hz.x, hz.y));
    }
    out
}

fn build_manifest(model: &StageModel, hazard_entries: &[Value]) -> Value {
    let id = &model.id;
    // a match needs at least one music track to start, and it must be a real public FM bgm
    // (the SSF2 soundtrack isn't shipped with the engine). `fm_music` is the override-map
    // pick or the configured default; the original SSF2 track ids go in the description.
    let music: Vec<Value> = model.fm_music.iter().map(|bgm| json!({
        "namespace": "public", "resourceId": bgm, "contentId": bgm
    })).collect();
    let mut description = format!("{} — converted from Super Smash Flash 2", model.display_name);
    if let Some(series) = &model.series { description.push_str(&format!(" ({series})")); }
    if !model.ssf2_music.is_empty() {
        description.push_str(&format!(". Original SSF2 soundtrack: {}", model.ssf2_music.join(", ")));
    }
    let mut content = vec![json!({
        "id": id,
        "name": model.display_name,
        "description": description,
        "type": "stage",
        "objectStatsId": format!("{id}StageStats"),
        "scriptId": format!("{id}Script"),
        "music": music,
        "metadata": {
            "ui": { "entityId": id, "render": { "animation": "stage" } }
        }
    })];
    content.extend(hazard_entries.iter().cloned());
    json!({ "resourceId": id, "content": content })
}

fn build_fraytools() -> Value {
    json!({
        "frame_rate": 60,
        "snapToPixel": true,
        "paletteShaderMode": "RG_MAP",
        "plugins": [
            "com.fraymakers.ContentExporter",
            "com.fraymakers.FraymakersTypes",
            "com.fraymakers.FraymakersMetadata"
        ],
        "pluginMetadata": {},
        "publishFolders": [ { "id": "build0", "path": "./build" } ],
        "version": 12
    })
}

fn write_meta(path: &Path, _stage_id: &str, id: &str, language: &str, object_type: Option<&str>, _unused: Option<()>) -> Result<()> {
    let mut pm = json!({});
    if let Some(ot) = object_type {
        pm = json!({ "com.fraymakers.FraymakersMetadata": { "objectType": ot, "version": "0.4.0" } });
    }
    let plugins: Vec<&str> = if object_type.is_some() { vec!["com.fraymakers.FraymakersMetadata"] } else { vec![] };
    let v = json!({
        "export": true,
        "guid": det_uuid(&format!("stage::meta::{id}")),
        "id": id,
        "language": language,
        "pluginMetadata": pm,
        "plugins": plugins,
        "tags": [],
        "version": 1
    });
    write_json(path, &v)
}

/// StageStats: stage sprite + camera. The fixed SSF2 backdrop lives as an IMAGE layer in the
/// `stage` animation (it carries the surface fighters stand on, so it moves 1:1 with the
/// world). Each SSF2 camera-background layer is emitted as its own `parallax{i}` animation +
/// a `camera.backgrounds` entry that pans at the layer's own SSF2-derived `xPanMultiplier`.
fn stage_stats_hx(id: &str, parallax: &[ParallaxRef], scale: f64) -> String {
    // back-to-front: layer 0 is the farthest (highest depth). ParallaxBG sizes the layer as
    // `originalBGWidth × scaleMultiplier` (native png × the stage scale; the IMAGE symbol stays
    // at scale 1). PAN mode pans the layer at `xPanMultiplier` of the camera offset, matching
    // how SSF2's Vcam scrolls each camera background.
    let entries: Vec<String> = parallax.iter().enumerate().map(|(i, p)| {
        // PAN: straight parallax pan at the per-layer multiplier (the SSF2 `_cambg` behavior).
        // BOUNDS: anchored to the camera bounds + tiling to fill (loopWidth/Height = the tile
        // size), for a repeating backdrop.
        let (mode, hscroll, vscroll, loop_w, loop_h) = match p.mode {
            ParallaxMode::Pan => ("PAN", "false", "false", 0, 0),
            ParallaxMode::Bounds => ("BOUNDS", "true", "true", p.art.w, p.art.h),
        };
        format!(
        "\n\t\t\t{{\n\
\t\t\t\tspriteContent: self.getResource().getContent(\"{id}\"),\n\
\t\t\t\tanimationId: \"parallax{i}\",\n\
\t\t\t\tmode: ParallaxMode.{mode},\n\
\t\t\t\toriginalBGWidth: {w},\n\
\t\t\t\toriginalBGHeight: {h},\n\
\t\t\t\thorizontalScroll: {hscroll},\n\
\t\t\t\tverticalScroll: {vscroll},\n\
\t\t\t\tloopWidth: {loop_w},\n\
\t\t\t\tloopHeight: {loop_h},\n\
\t\t\t\txPanMultiplier: {xp},\n\
\t\t\t\tyPanMultiplier: {yp},\n\
\t\t\t\tscaleMultiplier: {scale},\n\
\t\t\t\tforeground: false,\n\
\t\t\t\tdepth: {depth}\n\
\t\t\t}}",
        w = p.art.w, h = p.art.h, xp = p.x_pan, yp = p.y_pan, depth = 2000 - (i as i64) * 10)
    }).collect();
    let backgrounds = if entries.is_empty() { String::new() } else { format!("{}\n\t\t", entries.join(",")) };
    format!(
        "// Stats for {id} (converted from SSF2)\n\n\
{{\n\
\tspriteContent: self.getResource().getContent(\"{id}\"),\n\
\tanimationId: \"stage\",\n\
\tambientColor: 0xffffffff,\n\
\tshadowLayers: [],\n\
\tcamera: {{\n\
\t\tstartX: 0,\n\
\t\tstartY: 0,\n\
\t\tzoomX: 0,\n\
\t\tzoomY: 0,\n\
\t\tcamEaseRate: 1 / 11,\n\
\t\tcamZoomRate: 1 / 15,\n\
\t\tminZoomHeight: 360,\n\
\t\tinitialHeight: 360,\n\
\t\tinitialWidth: 640,\n\
\t\tbackgrounds: [{backgrounds}]\n\
\t}}\n\
}}\n"
    )
}

/// Stage Script.hx — pause a static stage on frame 1; let an animated stage's timeline
/// play (the SSF2 animated clips loop). The parallax background is camera-scrolled by
/// StageStats, so no manual scroll is needed.
fn script_hx(id: &str, animated: bool, hazard_spawns: &str) -> String {
    let init = if animated { "\t// animated stage clips play + loop on the timeline" } else { "\tself.pause();" };
    let hazards = if hazard_spawns.is_empty() { String::new() }
        else { format!("\t// spawn the stage's hazards (custom game objects)\n{hazard_spawns}") };
    format!(
        "// API Script for {id} (converted from SSF2)\n\n\
function initialize() {{\n\
{init}\n\
{hazards}\
}}\n\
function update() {{}}\n\
function onTeardown() {{}}\n\
function onKill() {{}}\n\
function onStale() {{}}\n\
function afterPushState() {{}}\n\
function afterPopState() {{}}\n\
function afterFlushStates() {{}}\n"
    )
}
