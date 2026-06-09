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
        Ok(ArtRef { guid, x: art.x, y: art.y, w: art.w, h: art.h, hold: art.hold.max(1) as usize })
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
    // each backdrop ELEMENT is its own layer (the SSF2 movieclip model). a static element is one
    // frame (`bgN`); an animated element is its loop (`bgN_0..M`). draw order = list order.
    let bg_refs: Vec<BgLayerRef> = model.art.background.iter().enumerate()
        .map(|(i, layer)| -> Result<BgLayerRef> {
            let frames: Vec<ArtRef> = if layer.frames.len() == 1 {
                vec![write_layer(&format!("bg{i}"), &layer.frames[0])?]
            } else {
                layer.frames.iter().enumerate()
                    .map(|(j, a)| write_layer(&format!("bg{i}_{j}"), a))
                    .collect::<Result<_>>()?
            };
            Ok(BgLayerRef { name: bg_layer_name(&layer.name, i), frames })
        })
        .collect::<Result<_>>()?;
    // declared platforms become MOVING STRUCTURES (like the official stage-template's moving
    // platform): one shared grey `platformSprite` (an IMAGE + a structure LINE_SEGMENT, in the
    // stage entity), and one structure CONTENT per platform that the stage spawns and that moves
    // itself in its own Script (the sink/rise cycle). emit_platform_structures writes the per-
    // platform Stats + the shared Script + the grey sprite, and returns what the manifest + stage
    // Script need.
    let (platform_sprites, structure_contents, structure_spawn_ids) =
        emit_platform_structures(model, &lib, &sprites)?;
    let art = ArtRefs {
        background: bg_refs,
        parallax: parallax_refs,
        stage: stage_refs,
        foreground: model.art.foreground.iter().enumerate()
            .map(|(i, a)| write_layer(&format!("fg{i}"), a)).collect::<Result<_>>()?,
        platform_sprites,
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

    write_json(&lib.join("manifest.json"), &build_manifest(model, &hazard_entries, &structure_contents))?;
    write_meta(&lib.join("manifest.json.meta"), id, "manifest", "json", None, None)?;

    let scripts = lib.join("scripts").join("stage");
    // animated if ANY looping art layer has multiple frames — the background (lava/fire/Bowser on
    // bowserscastle) animates just as much as the stage plane, so checking only the stage plane
    // froze background-animated stages (the Script paused the whole timeline).
    // the foreground glow flickers (its own animated layer), so it also requires a playing timeline.
    let animated = art.background.iter().any(|l| l.frames.len() > 1) || art.stage.len() > 1
        || !art.foreground.is_empty();
    // the stage spawns its moving-platform structures + hazards in its Script.
    let mut spawns = structure_spawn_ids.iter()
        .map(|cid| format!("\t\t\tmatch.createStructure(self.getResource().getContent(\"{cid}\"));\n"))
        .collect::<String>();
    spawns.push_str(&hazard_spawn_lines(model));
    std::fs::write(scripts.join(format!("{id}Script.hx")), script_hx(id, animated, &spawns))?;
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
    /// Length of the stage animation in engine frames (static layers hold for this many).
    frame_len: usize,
    /// SSF2 -> FM art scale (`size_multiplier`): native-resolution art PNGs render at this
    /// scale so they match the scaled-up geometry + fighters.
    scale: f64,
    /// Extra animations beyond `stage`/parallax (e.g. `platformSprite` for moving structures).
    extra_anims: Vec<Value>,
}

impl<'a> EntityBuilder<'a> {
    fn new(id: &'a str) -> Self {
        EntityBuilder { id, seq: 0, symbols: vec![], keyframes: vec![], layers: vec![], anim_layers: vec![], parallax_anims: vec![], frame_len: 1, scale: 1.0, extra_anims: vec![] }
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
        self.make_image_alpha(name, image_asset, x, y, img_scale, 1.0)
    }
    /// `make_image` with an explicit symbol alpha (for a semi-transparent overlay).
    fn make_image_alpha(&mut self, name: &str, image_asset: &str, x: f64, y: f64, img_scale: f64, alpha: f64) -> String {
        let sym = self.uid(&format!("sym:{name}"));
        self.symbols.push(json!({
            "$id": sym, "type": "IMAGE", "imageAsset": image_asset, "alpha": alpha,
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
    /// Build a `platformSprite` animation (an IMAGE + a structure LINE_SEGMENT, in LOCAL coords
    /// centered on the object origin) that a moving Structure references by animationId. The grey
    /// PNG is native size `w x h`; the standable line is the top edge. Stored in `extra_anims`.
    fn add_platform_animation(&mut self, idx: usize, grey_guid: &str, w: f64, _h: f64) {
        let img = self.make_image(&format!("platformImage{idx}"), grey_guid, -w / 2.0, 0.0, 1.0);
        // a solid floor line across the top, with grabbable ledges at both ends.
        let pm = json!({ "structureType": "FLOOR", "leftLedge": true, "rightLedge": true, "dropThrough": false });
        let sym = self.uid("sym:platformLine");
        self.symbols.push(json!({
            "$id": sym, "type": "LINE_SEGMENT", "alpha": 0.5, "color": "0xeeeeee",
            "points": [-w / 2.0, 0.0, w / 2.0, 0.0],
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": pm }
        }));
        let kf = self.uid("kf:platformLine");
        self.keyframes.push(json!({ "$id": kf, "length": 1, "pluginMetadata": {}, "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "LINE_SEGMENT" }));
        let line = self.uid("layer:platformLine");
        self.layers.push(json!({
            "$id": line, "hidden": false, "locked": false, "name": "Line Segment Layer", "type": "LINE_SEGMENT",
            "keyframes": [kf], "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "lineSegmentType": "LINE_SEGMENT_STRUCTURE" } }
        }));
        let aid = self.uid(&format!("anim:platformSprite{idx}"));
        self.extra_anims.push(json!({
            "$id": aid, "name": format!("platformSprite{idx}"), "pluginMetadata": {}, "layers": [img, line]
        }));
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
    fn add_image_frames(&mut self, name: &str, frames: &[(String, f64, f64, usize)]) {
        self.add_image_frames_alpha(name, frames, 1.0);
    }
    /// `add_image_frames` with an explicit symbol alpha (a semi-transparent animated overlay).
    fn add_image_frames_alpha(&mut self, name: &str, frames: &[(String, f64, f64, usize)], alpha: f64) {
        let mut kfs = Vec::new();
        for (i, (guid, x, y, hold)) in frames.iter().enumerate() {
            let sym = self.uid(&format!("sym:{name}:{i}"));
            self.symbols.push(json!({
                "$id": sym, "type": "IMAGE", "imageAsset": guid, "alpha": alpha,
                "x": x, "y": y, "scaleX": self.scale, "scaleY": self.scale, "rotation": 0, "pivotX": 0, "pivotY": 0,
                "pluginMetadata": {}
            }));
            let kf = self.uid(&format!("kf:{name}:{i}"));
            // per-frame hold = the run length of identical source frames (RLE), so held frames
            // read as pauses and the loop runs at the SSF2 pace.
            self.keyframes.push(json!({ "$id": kf, "length": (*hold).max(1), "pluginMetadata": {}, "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" }));
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
struct ArtRef { guid: String, x: f64, y: f64, w: u32, h: u32, hold: usize }

/// A parallax camera-background layer: the written sprite + its scroll mode + pan rate.
struct ParallaxRef { art: ArtRef, mode: ParallaxMode, x_pan: f64, y_pan: f64 }

/// One backdrop element as its own layer: a display name + its frame sequence (1 = static).
struct BgLayerRef { name: String, frames: Vec<ArtRef> }

/// A readable FM layer name for a backdrop element from its SSF2 symbol id. Strips the
/// `_bg` suffix and any `<stage>_` prefix, title-cases the rest; falls back to a numbered
/// "Background Art" for the unnamed root backdrop.
fn bg_layer_name(sym: &str, idx: usize) -> String {
    let s = sym.trim().trim_end_matches("_bg").trim_end_matches("_BG");
    // drop a leading `<stageid>_` style prefix (everything up to the last meaningful token group).
    let cleaned: String = s.split(['_', '.']).filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() { Some(f) => f.to_uppercase().collect::<String>() + c.as_str(), None => String::new() }
        })
        .collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        if idx == 0 { "Background Art".to_string() } else { format!("Background Art {idx}") }
    } else {
        cleaned
    }
}

/// The depth layers the entity lays out. `stage` is the frame sequence (1 = static);
/// `background` is the ordered per-element backdrop layers (each 1 = static).
struct ArtRefs { background: Vec<BgLayerRef>, parallax: Vec<ParallaxRef>, stage: Vec<ArtRef>, foreground: Vec<ArtRef>, platform_sprites: Vec<(String, f64, f64)> }

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
    StageArt { png, x: min_x, y: min_y, w, h, hold: 1 }
}

fn build_entity(model: &StageModel, art: &ArtRefs) -> Value {
    let id = &model.id;
    let mut b = EntityBuilder::new(id);
    b.scale = model.scale;
    // the stage animation loops for the SUM of the per-frame holds of the richest animated layer
    // (each frame carries its own hold = the run length of identical source frames at FM's
    // 60fps), so the loop matches the SSF2 duration. a static layer holds for the whole loop.
    let layer_len = |refs: &[ArtRef]| refs.iter().map(|a| a.hold).sum::<usize>();
    let bg_max_len = art.background.iter().map(|l| layer_len(&l.frames)).max().unwrap_or(0);
    b.frame_len = bg_max_len.max(layer_len(&art.stage)).max(1);

    // ── render order (first = back): the painted backdrop, background depth containers,
    // the stage art (behind fighters), the character containers, the foreground art (in
    // front of fighters), the foreground containers, then the invisible collision / spawns.
    // The backdrop is FIXED, not parallax-scrolled: the SSF2 `<id>_bg` plane includes the
    // surface fighters stand on, so it has to stay aligned with the collision. ──
    // each backdrop element is its own IMAGE layer/animation (SSF2 movieclip model), in
    // back-to-front list order. a one-frame element is a static image; a multi-frame one loops.
    for layer in &art.background {
        match layer.frames.as_slice() {
            [] => {}
            [a] => b.add_image(&layer.name, &a.guid, a.x, a.y),
            frames => b.add_image_frames(&layer.name, &frames.iter().map(|a| (a.guid.clone(), a.x, a.y, a.hold)).collect::<Vec<_>>()),
        }
    }
    b.add_container("Background Behind", "BACKGROUND_BEHIND_CONTAINER");
    b.add_container("Background Effects", "BACKGROUND_EFFECTS_CONTAINER");
    b.add_container("Background Shadows", "BACKGROUND_SHADOWS_CONTAINER");
    b.add_container("Background Structures", "BACKGROUND_STRUCTURES_CONTAINER");
    if !art.stage.is_empty() {
        let frames: Vec<(String, f64, f64, usize)> = art.stage.iter().map(|a| (a.guid.clone(), a.x, a.y, a.hold)).collect();
        b.add_image_frames("Stage Art", &frames);
    }
    // moving platforms reference the shared `platformSprite` animation (grey surface + a structure
    // line segment); the stage spawns them as Structures that move themselves. Add that animation.
    for (i, (grey, w, h)) in art.platform_sprites.iter().enumerate() {
        b.add_platform_animation(i, grey, *w, *h);
    }
    b.add_container("Characters Back", "CHARACTERS_BACK_CONTAINER");
    b.add_container("Characters", "CHARACTERS_CONTAINER");
    b.add_container("Characters Front", "CHARACTERS_FRONT_CONTAINER");
    // the foreground (bowserscastle's lava-glow sheet + lightmask, classified foreground by the
    // AS3 plane map) draws IN FRONT of fighters as a semi-transparent overlay at ~0.5 alpha — the
    // REAL clip art, one keyframe per real frame (no synthesized flicker).
    if !art.foreground.is_empty() {
        b.add_image_frames_alpha("Foreground Art",
            &art.foreground.iter().map(|a| (a.guid.clone(), a.x, a.y, a.hold)).collect::<Vec<_>>(), 0.5);
    }
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
        // visible platforms are spawned as moving Structures (their collision comes from the
        // structure's own line segment), so they get NO static collision here.
        if p.visible { continue; }
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
    animations.extend(b.extra_anims.clone());

    // SSF2 30fps -> Fraymakers 60fps: double every keyframe length, exactly like the character
    // port (entity_gen::double_keyframe_lengths). every layer (bg clips, foreground, platforms,
    // containers, points, line segments) was built in SSF2 frame units, so this doubling keeps
    // them all in lockstep at the right real-time speed.
    crate::entity_gen::double_keyframe_lengths(&mut b.keyframes);

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

use crate::stage_parser::{Hazard, Platform};

/// Emit the declared platforms as MOVING STRUCTURES (the FM idiom for a moving/sinking platform,
/// per the official stage-template): one shared grey `platformSprite` PNG (referenced by the
/// `platformSprite` animation in the stage entity), a shared sink/rise Script, and one structure
/// CONTENT (with its own Stats giving startX/startY) per platform. Returns the shared sprite
/// `(guid, w, h)` for the entity animation, the manifest structure-content entries, and the
/// content ids the stage Script spawns with `match.createStructure`.
#[allow(clippy::type_complexity)]
fn emit_platform_structures(model: &StageModel, lib: &Path, sprites: &Path)
    -> Result<(Vec<(String, f64, f64)>, Vec<Value>, Vec<String>)>
{
    let vis: Vec<&Platform> = model.platforms.iter().filter(|p| p.visible).collect();
    if vis.is_empty() { return Ok((Vec::new(), Vec::new(), Vec::new())); }
    let id = &model.id;
    let scripts = lib.join("scripts").join("platform");
    std::fs::create_dir_all(&scripts).context("mkdir scripts/platform")?;
    // shared sink/rise structure Script. HALF_W gates which platform a Thwomp landed on; the
    // platforms sit far apart (columns ~330px), so a generous-but-sub-spacing half-width assigns
    // the Thwomp to exactly one without bleeding into its neighbour.
    let script_id = format!("{id}platformScript");
    std::fs::write(scripts.join(format!("{script_id}.hx")), platform_script_hx(150.0))?;
    write_meta(&scripts.join(format!("{script_id}.hx.meta")), id, &script_id, "hscript", Some("LINE_SEGMENT_STRUCTURE"), None)?;
    // per-platform: a sprite sized to THIS platform (the SSF2 standing platforms are different
    // widths), its own `platformSprite{i}` animation, Stats (startX/startY), a structure content
    // entry, and the spawn id. the fill is an opaque shaded grey stone block matching SSF2's
    // lava-lit terrain rendering (see the per-pixel shading below).
    let mut sprite_dims = Vec::new();
    let mut contents = Vec::new();
    let mut spawn_ids = Vec::new();
    for (i, p) in vis.iter().enumerate() {
        let (pw, ph) = (p.rect.w.round().max(8.0) as u32, p.rect.h.round().max(10.0) as u32);
        // SSF2 renders its standing platforms as opaque shaded grey stone blocks (lava-lit), not
        // a flat translucent fill. Match that: an OPAQUE block with a top-lit vertical shade
        // (lighter standable lip -> darker base, grey ~182..96 sampled from SSF2) plus a faint
        // mortar lattice (stone courses + offset joints) so it reads as terrain, not a placeholder.
        let lip = (ph / 8).max(3);
        let course = 18u32; // px between horizontal stone courses
        let joint = 30u32;  // px between vertical joints (offset every other course)
        let mut img = image::RgbaImage::new(pw, ph);
        for (x, y, px) in img.enumerate_pixels_mut() {
            if y < lip { *px = image::Rgba([200, 200, 204, 255]); continue; } // lit standable lip
            let yf = y as f32 / ph.max(1) as f32;            // 0 top .. 1 bottom
            let mut v = (182.0 - 86.0 * yf) as i32;          // top-lit vertical shade
            let row = y / course;
            let off = (row % 2) * (joint / 2);               // brick-offset alternate courses
            if (y % course) < 2 || ((x + off) % joint) < 2 { v -= 34; } // darker mortar
            let v = v.clamp(60, 220) as u8;
            *px = image::Rgba([v, v, v, 255]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .context("encode platform png")?;
        let guid = det_uuid(&format!("stage::{id}::platformSprite{i}"));
        std::fs::write(sprites.join(format!("{id}_platformSprite{i}.png")), &png)?;
        write_json(&sprites.join(format!("{id}_platformSprite{i}.png.meta")), &json!({
            "export": false, "guid": guid, "id": "", "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
        }))?;
        sprite_dims.push((guid, pw as f64, ph as f64));

        let cid = format!("{id}platform{i}");
        let stats_id = format!("{cid}Stats");
        let (sx, sy) = (p.rect.x + p.rect.w / 2.0, p.rect.y);
        std::fs::write(scripts.join(format!("{stats_id}.hx")), platform_stats_hx(id, sx, sy, i))?;
        write_meta(&scripts.join(format!("{stats_id}.hx.meta")), id, &stats_id, "hscript", None, None)?;
        contents.push(json!({ "id": cid, "type": "structure", "objectStatsId": stats_id, "scriptId": script_id }));
        spawn_ids.push(cid);
    }
    Ok((sprite_dims, contents, spawn_ids))
}

/// Stats for one moving platform: which sprite + animation gives its geometry, and where it spawns.
fn platform_stats_hx(stage_id: &str, start_x: f64, start_y: f64, idx: usize) -> String {
    format!(
        "// Moving-platform stats (sink/rise) for {stage_id}\n{{\n\
         \tspriteContent: self.getResource().getContent(\"{stage_id}\"),\n\
         \tanimationId: \"platformSprite{idx}\",\n\tstartX: {start_x:.1},\n\tstartY: {start_y:.1}\n}}\n")
}

/// A STATIC standing-platform structure. SSF2's standing platforms don't sink (the Thwomp drops
/// with its own self-platform); the structure just holds its spawn position and provides the floor
/// line segment from its `platformSprite{i}` animation.
fn platform_script_hx(_half_w: f64) -> String {
    "// Static standing platform (its floor line segment comes from the platformSprite animation).\n\
     function initialize() {}\nfunction update() {}\n\
     function onTeardown() {}\nfunction onKill() {}\nfunction onStale() {}\n\
     function afterPushState() {}\nfunction afterPopState() {}\nfunction afterFlushStates() {}\n".to_string()
}

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
    // a `thwomp`-motion hazard cycles through the stage's platform columns, falling onto each so
    // the platform under it sinks (its standable top y is the thwomp's land height).
    let cols: Vec<(f64, f64)> = model.platforms.iter().filter(|p| p.visible)
        .map(|p| (p.rect.x + p.rect.w / 2.0, p.rect.y)).collect();

    for (i, hz) in model.hazards.iter().enumerate() {
        let hid = hazard_id(&model.id, i);
        // sprite: the real rasterized SSF2 hazard art if we recovered it, else a translucent red
        // hitbox-volume placeholder (w x h). real art renders at the stage scale, centered on the
        // hazard; the placeholder fills the hitbox 1:1.
        let (png, img_x, img_y, img_scale) = if let Some(art) = &hz.art {
            let (aw, ah) = (art.w as f64 * model.scale, art.h as f64 * model.scale);
            (art.png.clone(), -aw / 2.0, -ah / 2.0, model.scale)
        } else {
            let (w, h) = (hz.w.max(8.0) as u32, hz.h.max(8.0) as u32);
            let mut img = image::RgbaImage::new(w, h);
            for px in img.pixels_mut() { *px = image::Rgba([220, 40, 40, 130]); }
            let mut p = Vec::new();
            image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut p), image::ImageFormat::Png)
                .context("encode hazard png")?;
            (p, -hz.w / 2.0, -hz.h / 2.0, 1.0)
        };
        let sprite_guid = det_uuid(&format!("hazard::{hid}::sprite"));
        std::fs::write(sprites.join(format!("{hid}.png")), &png)?;
        write_json(&sprites.join(format!("{hid}.png.meta")), &json!({
            "export": false, "guid": sprite_guid, "id": "", "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
        }))?;

        write_json(&lib.join("entities").join(format!("{hid}.entity")), &hazard_entity(&hid, hz, &sprite_guid, img_x, img_y, img_scale))?;
        write_meta(&lib.join("entities").join(format!("{hid}.entity.meta")), &hid, &hid, "", Some("CUSTOM_GAME_OBJECT"), None)?;

        let script = if hz.motion == "thwomp" && !cols.is_empty() {
            thwomp_script_hx(&cols)
        } else {
            hazard_script_hx(hz)
        };
        let files = [
            ("Script", script),
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
fn hazard_entity(hid: &str, hz: &Hazard, sprite_guid: &str, img_x: f64, img_y: f64, img_scale: f64) -> Value {
    let g = |s: &str| det_uuid(&format!("hazard::{hid}::{s}"));
    let (hw, hh) = (hz.w / 2.0, hz.h / 2.0);
    let img_sym = |s: &str| json!({ "$id": g(&format!("imgsym{s}")), "type": "IMAGE", "imageAsset": sprite_guid, "x": img_x, "y": img_y, "pivotX": 0.0, "pivotY": 0.0, "scaleX": img_scale, "scaleY": img_scale, "rotation": 0.0, "alpha": 1.0, "pluginMetadata": {} });
    let img_kf = |s: &str| json!({ "$id": g(&format!("imgkf{s}")), "symbol": g(&format!("imgsym{s}")), "length": 1, "tweened": false, "tweenType": "LINEAR", "type": "IMAGE", "pluginMetadata": {} });
    let img_layer = |s: &str| json!({ "$id": g(&format!("imglayer{s}")), "name": "art", "type": "IMAGE", "hidden": false, "locked": false, "keyframes": [g(&format!("imgkf{s}"))], "pluginMetadata": {} });
    let mut e = json!({
        "export": true, "guid": g("entity"), "id": hid, "version": 5,
        "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "objectType": "CUSTOM_GAME_OBJECT", "version": "0.4.0" } },
        "plugins": ["com.fraymakers.FraymakersTypes", "com.fraymakers.FraymakersMetadata"],
        "tags": [], "paletteMap": {}, "tilesets": [], "terrains": [],
        "symbols": [
            img_sym("A"), img_sym("B"),
            { "$id": g("boxsym"), "type": "COLLISION_BOX", "x": -hw, "y": -hh, "pivotX": hw, "pivotY": hh, "scaleX": hz.w, "scaleY": hz.h, "rotation": 0.0, "alpha": 0.5, "color": "0xff0000", "pluginMetadata": {} }
        ],
        "keyframes": [
            img_kf("A"), img_kf("B"),
            { "$id": g("boxkf"), "symbol": g("boxsym"), "length": 1, "tweened": false, "tweenType": "LINEAR", "type": "COLLISION_BOX", "pluginMetadata": {} }
        ],
        "layers": [
            img_layer("A"), img_layer("B"),
            { "$id": g("boxlayer"), "name": "hitbox0", "type": "COLLISION_BOX", "hidden": false, "locked": false, "defaultAlpha": 0.5, "defaultColor": "0xff0000", "keyframes": [g("boxkf")],
              "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "collisionBoxType": "HIT_BOX", "index": 0 } } }
        ],
        // gameObjectIdle = sprite + the HIT_BOX (damages); gameObjectInactive = sprite only (safe,
        // for the pulse off-phase). The Script's local state machine plays one or the other.
        "animations": [
            { "$id": g("anim"), "name": "gameObjectIdle", "layers": [g("imglayerA"), g("boxlayer")], "pluginMetadata": {} },
            { "$id": g("anim2"), "name": "gameObjectInactive", "layers": [g("imglayerB")], "pluginMetadata": {} }
        ]
    });
    // SSF2 30fps -> FM 60fps doubling, same as the stage entity + the character port.
    if let Some(kfs) = e.get_mut("keyframes").and_then(|k| k.as_array_mut()) {
        crate::entity_gen::double_keyframe_lengths(kfs);
    }
    e
}

/// The Thwomp (converted from SSF2 `Thwomp` + `bowserscastle::update`): it cycles through the
/// stage's platform columns, accelerating down onto each (a gravity slam), holding, then rising
/// and moving to the next column. Its native HIT_BOX damages on contact, and the platform it lands
/// on detects it (a custom game object on its surface) and sinks, exactly as SSF2's thwomp calls
/// `platform.sink()`. `cols` are the platform `(x-center, standable-top-y)` pairs; the Thwomp
/// falls onto each in turn, landing at THAT platform's top (they sit at different heights).
fn thwomp_script_hx(cols: &[(f64, f64)]) -> String {
    let cols_lit = cols.iter().map(|(x, _)| format!("{x:.1}")).collect::<Vec<_>>().join(", ");
    let land_lit = cols.iter().map(|(_, y)| format!("{y:.1}")).collect::<Vec<_>>().join(", ");
    // the rest height is well above the highest platform top (smallest y).
    let top_y = cols.iter().map(|(_, y)| *y).fold(f64::MAX, f64::min) - 340.0;
    format!(
        "// Thwomp (converted from SSF2). Falls onto a platform column -> that platform sinks; then\n\
         // rises and moves to the next column. Native HIT_BOX (HitboxStats) damages on contact.\n\n\
         function _prepLocalState(animation:String, ?index:Int=Math.NaN):Int {{\n\
         \tif (!__hasInitLocalStateMachine) {{ Common.initLocalStateMachine(); __hasInitLocalStateMachine = true; }}\n\
         \tif (index != Math.NaN) {{ index = __localStatePrepIndex++; }}\n\
         \tCommon.registerLocalState(index, animation);\n\treturn index;\n}}\n\
         var __hasInitLocalStateMachine = false;\nvar __localStatePrepIndex = -1;\n\
         var LState = {{\n\tUNINITIALIZED: _prepLocalState(\"#n/a\", -1),\n\tACTIVE: _prepLocalState(\"gameObjectIdle\"),\n\tINACTIVE: _prepLocalState(\"gameObjectInactive\")\n}};\n\n\
         var COLUMNS = [{cols_lit}];\nvar LAND_YS = [{land_lit}];\nvar TOP_Y = {top_y:.1};\n\
         var m_col = 0;\nvar m_phase = 0;\nvar m_fallV = 0.0;\nvar m_timer = 0;\nvar m_cool = 0;\nvar m_init = false;\n\n\
         function initialize() {{\n\tself.setState(PState.ACTIVE);\n\tCommon.toLocalState(LState.ACTIVE);\n}}\n\n\
         function update() {{\n\
         \tif (!m_init) {{ m_init = true; self.setX(COLUMNS[m_col]); self.setY(TOP_Y); }}\n\
         \tvar landY = LAND_YS[m_col];\n\
         \t// keep the native hitbox live so it damages fighters it falls through.\n\
         \tif (m_cool > 0) {{ m_cool = m_cool - 1; }} else {{ self.reactivateHitboxes(); m_cool = 18; }}\n\
         \tif (m_phase == 0) {{\n\
         \t\tm_fallV = m_fallV + 0.9;\n\t\tself.setY(self.getY() + m_fallV);\n\
         \t\tif (self.getY() >= landY) {{ self.setY(landY); m_phase = 1; m_timer = 0; }}\n\
         \t}} else if (m_phase == 1) {{\n\
         \t\tm_timer = m_timer + 1;\n\t\tif (m_timer >= 80) {{ m_phase = 2; }}\n\
         \t}} else {{\n\
         \t\tself.setY(self.getY() - 6.0);\n\
         \t\tif (self.getY() <= TOP_Y) {{ self.setY(TOP_Y); m_phase = 0; m_fallV = 0.0; m_col = (m_col + 1) % COLUMNS.length; self.setX(COLUMNS[m_col]); }}\n\
         \t}}\n\
         }}\n")
}

fn hazard_script_hx(hz: &Hazard) -> String {
    // A stage hazard is a custom game object with NO fighter owner (null owner), which makes it
    // neutral — a null hitbox owner passes the engine's team-hit validation, so it damages everyone.
    // Damage/knockback come from the NATIVE HIT_BOX (HitboxStats), not a script overlap test. The
    // local state machine plays the animations: ACTIVE (gameObjectIdle, carries the HIT_BOX) and
    // INACTIVE (gameObjectInactive, safe) for the on/off pulse. Two stats are load-bearing for the
    // native hitbox to connect: GameObjectStats.stateTransitionMapOverrides (PState.ACTIVE ->
    // gameObjectIdle, so an animation actually plays and the collision detector's currentAnimation
    // resolves against the HitboxStats map) and spriteContent as a real content ref. The hitbox is
    // re-armed on a cadence so a lingering fighter keeps taking damage. `motion` moves the entity.
    let tau = "6.2831853";
    let motion = match hz.motion.as_str() {
        "oscillateX" => format!("\tself.setX(m_baseX + {r} * Math.sin(m_frame * {tau} / {p}));\n", r = hz.range, p = hz.period),
        "oscillateY" => format!("\tself.setY(m_baseY + {r} * Math.sin(m_frame * {tau} / {p}));\n", r = hz.range, p = hz.period),
        "circle" => format!("\tself.setX(m_baseX + {r} * Math.cos(m_frame * {tau} / {p}));\n\tself.setY(m_baseY + {r} * Math.sin(m_frame * {tau} / {p}));\n", r = hz.range, p = hz.period),
        // thwomp: hover at the top, accelerate DOWN (quadratic, a gravity slam), rest at the
        // bottom, then ease back up. SMOOTH per-frame Y (no teleport) so it doesn't flicker.
        "fall" => format!(
            "\tvar _t = m_frame % {p};\n\
             \tif (_t < {p} * 0.5) {{ self.setY(m_baseY); }}\n\
             \telse if (_t < {p} * 0.62) {{ var _f = (_t - {p} * 0.5) / ({p} * 0.12); self.setY(m_baseY + {r} * _f * _f); }}\n\
             \telse if (_t < {p} * 0.78) {{ self.setY(m_baseY + {r}); }}\n\
             \telse {{ var _f = (_t - {p} * 0.78) / ({p} * 0.22); self.setY(m_baseY + {r} * (1.0 - _f)); }}\n",
            r = hz.range, p = hz.period),
        _ => String::new(),
    };
    // pulse: toggle the active (hitbox) and inactive (no hitbox) states on the duty cycle.
    let pulse = if hz.interval > 0 {
        format!(
            "\tvar on = (m_frame % {iv}) < {ac};\n\
             \tif (on && Common.inLocalState(LState.INACTIVE)) {{ Common.toLocalState(LState.ACTIVE); }}\n\
             \telse if (!on && Common.inLocalState(LState.ACTIVE)) {{ Common.toLocalState(LState.INACTIVE); }}\n",
            iv = hz.interval, ac = hz.active)
    } else {
        String::new()
    };
    format!(
        "// Stage hazard (custom game object) — converted from SSF2.\n\
         // Local state machine (clean multi-animation on a non-character entity) + the native\n\
         // hitbox (HitboxStats). null owner is fine for damage. `motion` = the SSF2 movement.\n\n\
         function _prepLocalState(animation:String, ?index:Int=Math.NaN):Int {{\n\
         \tif (!__hasInitLocalStateMachine) {{ Common.initLocalStateMachine(); __hasInitLocalStateMachine = true; }}\n\
         \tif (index != Math.NaN) {{ index = __localStatePrepIndex++; }}\n\
         \tCommon.registerLocalState(index, animation);\n\
         \treturn index;\n\
         }}\n\
         var __hasInitLocalStateMachine = false;\n\
         var __localStatePrepIndex = -1;\n\
         var LState = {{\n\
         \tUNINITIALIZED: _prepLocalState(\"#n/a\", -1),\n\
         \tACTIVE: _prepLocalState(\"gameObjectIdle\"),\n\
         \tINACTIVE: _prepLocalState(\"gameObjectInactive\")\n\
         }};\n\n\
         var REHIT = {rehit};\n\
         var m_frame = 0;\nvar m_baseX = 0.0;\nvar m_baseY = 0.0;\nvar m_init = false;\nvar m_cooldown = 0;\n\n\
         function initialize() {{\n\tself.setState(PState.ACTIVE);\n\tCommon.toLocalState(LState.ACTIVE);\n}}\n\n\
         function update() {{\n\
         \tif (!m_init) {{ m_baseX = self.getX(); m_baseY = self.getY(); m_init = true; }}\n\
         \tm_frame = m_frame + 1;\n\
{motion}{pulse}\
         \t// re-arm the native HIT_BOX so a fighter standing in the hazard keeps taking hits\n\
         \t// (a hitbox hits each target once per attack id; reactivateHitboxes issues a fresh one).\n\
         \tif (Common.inLocalState(LState.ACTIVE)) {{\n\
         \t\tif (m_cooldown > 0) {{ m_cooldown = m_cooldown - 1; }}\n\
         \t\telse {{ self.reactivateHitboxes(); m_cooldown = REHIT; }}\n\
         \t}}\n\
         }}\n",
        rehit = hz.rehit())
}

fn hazard_gameobject_stats_hx(hid: &str) -> String {
    // stateTransitionMapOverrides maps each PState to the animation the engine plays while in it.
    // Without it a custom game object in PState.ACTIVE plays NO animation, so the collision-box
    // detector's currentAnimation stays null and the native HIT_BOX never resolves against the
    // HitboxStats map (keyed by animation name) — the hit is silently dropped. ACTIVE plays the
    // damaging frame; the local state machine swaps in gameObjectInactive for the off-pulse.
    format!(
        "// GameObjectStats for {hid}\n{{\n\tspriteContent: self.getResource().getContent(\"{hid}\"),\n\tinitialState: PState.ACTIVE,\n\
         \tstateTransitionMapOverrides: [\n\t\tPState.ACTIVE => {{ animation: \"gameObjectIdle\" }}\n\t],\n\
         \tbaseScaleX: 1,\n\tbaseScaleY: 1,\n\tweight: 100,\n\tgravity: 0,\n\tfriction: 0\n}}\n")
}

fn hazard_hitbox_stats_hx(hz: &Hazard) -> String {
    format!(
        "// HitboxStats for the stage hazard. damage/knockback/angle from mappings/stage/metadata.jsonc.\n\
         {{\n\tgameObjectIdle: {{\n\t\thitbox0: {{ damage: {}, angle: {}, baseKnockback: {}, knockbackGrowth: 40, \
         hitstop: 6, hitstun: 24, reversibleAngle: true, directionalInfluence: true, reflectable: false }}\n\t}},\n\
         \tgameObjectInactive: {{}}\n}}\n",
        hz.damage, hz.angle, hz.knockback)
}

fn hazard_animation_stats_hx() -> String {
    "// AnimationStats for the stage hazard.\n{\n\tgameObjectIdle: { endType: AnimationEndType.NONE },\n\tgameObjectInactive: { endType: AnimationEndType.NONE }\n}\n".to_string()
}

/// hscript the stage Script runs to spawn its hazards (createCustomGameObject + position).
fn hazard_spawn_lines(model: &StageModel) -> String {
    let mut out = String::new();
    for (i, hz) in model.hazards.iter().enumerate() {
        let hid = hazard_id(&model.id, i);
        // owned by a character (a GameObject) so the hitbox registers; setX/setY positions it.
        out.push_str(&format!(
            "\t\t\tvar _hz{i} = match.createCustomGameObject(self.getResource().getContent(\"{hid}\"), owner);\n\
             \t\t\tif (_hz{i} != null) {{ _hz{i}.setX({:.1}); _hz{i}.setY({:.1}); }}\n",
            hz.x, hz.y));
    }
    out
}

fn build_manifest(model: &StageModel, hazard_entries: &[Value], structure_entries: &[Value]) -> Value {
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
    content.extend(structure_entries.iter().cloned());
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
    // hazards spawn DEFERRED in update() once the match is live (one-shot via a flag). owner is
    // NULL: a stage hazard belongs to no fighter, so it damages everyone (a null hitbox owner
    // passes the engine's team-hit validation), and createCustomGameObject accepts a null owner.
    let (haz_var, haz_body) = if hazard_spawns.is_empty() {
        // no hazards: keep update() a clean empty body (byte-stable with hazardless stages).
        (String::new(), String::new())
    } else {
        ("var m_hazardsSpawned = false;\n".to_string(),
         format!("\tif (!m_hazardsSpawned) {{\n\
                  \t\tvar chars = match.getCharacters();\n\
                  \t\tif (chars.length > 0) {{\n\
                  \t\t\tm_hazardsSpawned = true;\n\
                  \t\t\tvar owner = null;\n\
{hazard_spawns}\
                  \t\t}}\n\
                  \t}}\n"))
    };
    let update_fn = if haz_body.is_empty() {
        "function update() {}\n".to_string()
    } else {
        format!("function update() {{\n{haz_body}}}\n")
    };
    format!(
        "// API Script for {id} (converted from SSF2)\n\n\
{haz_var}\
function initialize() {{\n\
{init}\n\
}}\n\
{update_fn}\
function onTeardown() {{}}\n\
function onKill() {{}}\n\
function onStale() {{}}\n\
function afterPushState() {{}}\n\
function afterPopState() {{}}\n\
function afterFlushStates() {{}}\n"
    )
}

#[cfg(test)]
mod hazard_tests {
    use super::*;

    fn demo_hazard() -> Hazard {
        Hazard {
            x: 0.0, y: 150.0, w: 700.0, h: 160.0,
            damage: 10.0, knockback: 0.0, angle: 45.0,
            interval: 0, active: 20, motion: "static".into(),
            range: 0.0, period: 120, rehit: 30, label: "TestHazard".into(), art: None,
        }
    }

    // The native custom-game-object hitbox only connects when the engine is actually playing the
    // hitbox animation, so the collision detector's currentAnimation resolves against the
    // HitboxStats map. These three pieces are each individually load-bearing and fail SILENTLY
    // (0 damage, no error) if dropped — so assert them.
    #[test]
    fn hazard_gameobject_stats_drive_the_hitbox_animation() {
        let s = hazard_gameobject_stats_hx("demohazard0");
        // PState.ACTIVE must map to the animation that carries the HIT_BOX, or nothing plays and
        // the collision detector's currentAnimation stays null -> every hit is silently dropped.
        assert!(s.contains("stateTransitionMapOverrides"), "missing state->animation map: {s}");
        assert!(s.contains("PState.ACTIVE => { animation: \"gameObjectIdle\" }"), "wrong/absent ACTIVE animation: {s}");
        // spriteContent must be a real content ref (a bare string id does not load the sprite).
        assert!(s.contains("spriteContent: self.getResource().getContent(\"demohazard0\")"), "spriteContent not a content ref: {s}");
    }

    #[test]
    fn hazard_script_uses_native_hitbox_with_rearm() {
        let s = hazard_script_hx(&demo_hazard());
        // a hitbox hits each target once per attack id; re-arm so a lingering fighter keeps taking
        // hits (continuous SSF2-style hazard damage).
        assert!(s.contains("reactivateHitboxes()"), "no native-hitbox re-arm: {s}");
        // the local state machine plays the hitbox animation.
        assert!(s.contains("Common.toLocalState(LState.ACTIVE)"), "no local-state activation: {s}");
        // no script-overlap damage fallback — damage comes from the native HIT_BOX.
        assert!(!s.contains("addDamage"), "script-overlap damage should be gone: {s}");
    }

    #[test]
    fn hazard_spawns_with_null_owner() {
        // a null hitbox owner is neutral and passes the engine's team-hit validation (hits
        // everyone), and createCustomGameObject accepts a null owner.
        let spawn = "\t\t\tvar _hz0 = match.createCustomGameObject(self.getResource().getContent(\"demohazard0\"), owner);\n";
        let s = script_hx("demo", false, spawn);
        assert!(s.contains("var owner = null;"), "hazard owner should be null: {s}");
        assert!(s.contains("createCustomGameObject"), "no spawn call: {s}");
    }
}
