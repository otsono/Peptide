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
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::stage_parser::{ParallaxMode, Rect, StageArt, StageModel};
use crate::emit::stage_tables::{BootAt, BootCamBg, BootDraw, BootElement, BootFollow, BootGroup, BootObject,
                                BootStructure, BootWeather, StageTables};
use crate::uuid_gen::det_uuid;

/// Emit the FM stage package for `model` under `out_root/<id>/`. Returns the
/// project dir and the `.fraytools` path (for the FrayTools publish step).

/// The one conversion for a per-frame SPEED, shared by everything that moves.
///
/// A speed is a distance per frame, and a conversion changes both: the world is `size_multiplier`
/// bigger and a frame is half as long. Characters got this from the physics model; stages open-coded
/// it as `scale * 0.5`, which agreed only for as long as nobody edited the frame rates in
/// stats.jsonc. A thwomp falls by the same arithmetic a fighter does.
/// px a frame squared: the velocity scale over the frame ratio again (a stop the source takes
/// per source frame is taken per engine frame here, twice as often)
fn accel_scale() -> f64 { velocity_scale() / 2.0 }

fn velocity_scale() -> f64 {
    crate::mappings::character_stats().scaling.velocity_scale()
}


/// Paint a placement's colour transform INTO the picture.
///
/// The engine takes a per-channel multiply and offset only as a SHADER, and only a drawable takes
/// a shader: a container takes filters, none of which express an offset. So art the stage draws
/// through the engine's own camera-background system has nowhere to carry its colour at runtime,
/// and the only way it keeps the source's colour is in its pixels.
///
/// Bounded by area, because the source slides these colours: a piece drawn through a hundred
/// tints becomes a hundred pictures, which is nothing for a clock ring and gigabytes for a
/// full-width backdrop plate. Over the budget the colour is left on the frame for a shader to
/// carry if the piece has one, and the caller warns if it does not.
const BAKE_BUDGET_PX: u64 = 400_000;

fn bake_tint(png: &[u8], tint: [f64; 6], w: u32, h: u32) -> Option<Vec<u8>> {
    if (w as u64) * (h as u64) > BAKE_BUDGET_PX { return None; }
    let mut img = image::load_from_memory(png).ok()?.to_rgba8();
    for px in img.pixels_mut() {
        for c in 0..3 {
            // exactly SWF's rule, and the shader's: scale, then offset, then clamp
            px[c] = (px[c] as f64 * tint[c] + tint[3 + c]).clamp(0.0, 255.0) as u8;
        }
    }
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&img, w, h, image::ExtendedColorType::Rgba8).ok()?;
    Some(out)
}

/// A plain white plate, the shape of nothing in particular.
///
/// A shadow mask is read for its COVERAGE, not its colour: wherever the mask draws, a shadow is
/// allowed to land. So one opaque tile scaled to each walkable surface says "shadows fall here"
/// without carrying any art of its own.
fn white_plate(w: u32, h: u32) -> Vec<u8> {
    use image::ImageEncoder;
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
    let mut out = Vec::new();
    let _ = image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&img, w, h, image::ExtendedColorType::Rgba8);
    out
}

/// The side of the mask plate, in pixels. It is scaled to each surface, so this only sets how
/// much resolution the edge has; a single flat colour needs almost none.
const MASK_PLATE_PX: u32 = 8;

/// The name the stage's shadow layer goes by, and the name its light casts through. The two have
/// to agree: a light names the layers it lights, so an unnamed layer is lit by nothing.
/// The animation of the stage's own sprite that the shadow mask lives on. The official stage keeps
/// it under exactly this name (and `shadowMaskFront` for one drawn in front), so the engine has an
/// animation to read the mask out of.
const SHADOW_MASK_ANIM: &str = "shadowMask";

/// How far above the play area the light sits: a stage-height again, so the cast stays near
/// vertical across the whole stage instead of raking as a fighter walks to the edge.
fn r_light_height(model: &StageModel) -> f64 {
    model.camera_box.as_ref().map(|r| r.h).unwrap_or(720.0)
}

/// The stage entity's animation that shows the stage's own art.
pub(crate) const STAGE_ANIM: &str = "stage";

pub fn emit_stage(model: &StageModel, out_root: &Path) -> Result<(PathBuf, PathBuf)> {
    let id = &model.id;
    let dir = out_root.join(id);
    let lib = dir.join("library");
    // the library is fully converter-generated: start clean so renamed content (a hazard id
    // derived from its class, an element id from its symbol) doesn't leave a stale generation
    // behind that FrayTools would bundle alongside the new one.
    if lib.exists() { std::fs::remove_dir_all(&lib).context("clean library")?; }
    std::fs::create_dir_all(lib.join("entities")).context("mkdir entities")?;
    std::fs::create_dir_all(lib.join("scripts").join("stage")).context("mkdir scripts/stage")?;

    // Stage art — split into depth layers: a parallax background, the main stage at
    // character depth, and a foreground in front of fighters. A stage needs visible
    // content to be playable (the engine sizes the stage + places players from the sprite
    // bounds), so if nothing rasterized we fall back to a geometry placeholder as the
    // stage layer. Each layer is a PNG + `.meta`; the `.png` is gitignored (regenerated).
    // THE SOUNDS THE STAGE AND ITS OBJECTS PLAY. A class asks for one by name, and the engine has
    // it because the package it came in carries it. Not shipped, the ask names nothing and the
    // whole stage plays silent: no thwomp landing, no lava, no crystal breaking.
    let shipped_sounds: std::collections::BTreeSet<String> = (|| {
        let raw = std::fs::read(&model.source).ok()?;
        let data = crate::ssf::decompress(&raw).ok()?;
        let buf = swf::decompress_swf(&data[..]).ok()?;
        let parsed = swf::parse_swf(&buf).ok()?;
        let audio = lib.join("audio");
        let sounds = crate::sound_extractor::extract_all_sounds(&parsed, &audio, id).ok()?;
        crate::haxe_gen::generate_sound_entries(&audio, id, &sounds).ok()?;
        Some(sounds.iter().map(|s| s.name.clone()).collect())
    })().unwrap_or_default();
    // A SOUND A CLASS ASKS FOR THAT ITS OWN PACKAGE DOES NOT CARRY. The game keeps shared packages
    // loaded, and a sound resolves out of them exactly as a class does, so the ones the stage's
    // objects name are looked for there and brought across -- only those, since a shared package
    // carries every borrowed class's sounds at once.
    let mut shipped_sounds = shipped_sounds;
    {
        let mut want: std::collections::BTreeSet<String> = model.hazards.iter()
            .flat_map(crate::stage_text::sounds_played)
            .filter(|s| !shipped_sounds.contains(s)).collect();
        if !want.is_empty() {
            let audio = lib.join("audio");
            let _ = std::fs::create_dir_all(&audio);
            for pkg in crate::shared_packages::packages() {
                if want.is_empty() { break }
                let Ok(buf) = swf::decompress_swf(&pkg.swf[..]) else { continue };
                let Ok(parsed) = swf::parse_swf(&buf) else { continue };
                let mine: Vec<_> = crate::sound_extractor::parse_sounds(&parsed)
                    .into_iter().filter(|e| want.contains(&e.name)).collect();
                if mine.is_empty() { continue }
                for e in &mine {
                    let safe: String = e.name.chars()
                        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
                    if crate::sound_extractor::convert_to_wav(e, &audio.join(format!("{safe}.wav"))).is_ok() {
                        shipped_sounds.insert(e.name.clone());
                        want.remove(&e.name);
                    }
                }
                let _ = crate::haxe_gen::generate_sound_entries(&audio, id, &mine);
                log::info!("{} sounds came across from the shared '{}' package", mine.len(), pkg.id);
            }
            // and finally the game's own movie, where what its fighters share is kept
            if !want.is_empty() {
                if let Some(movie) = crate::shared_packages::game_movie() {
                    if let Ok(parsed) = swf::decompress_swf(&movie[..]).and_then(|b| {
                        // the borrow has to outlive the parse, so the sounds are taken here
                        let p = swf::parse_swf(&b)?;
                        Ok(crate::sound_extractor::parse_sounds(&p))
                    }) {
                        let mine: Vec<_> = parsed.into_iter().filter(|e| want.contains(&e.name)).collect();
                        for e in &mine {
                            let safe: String = e.name.chars()
                                .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
                            if crate::sound_extractor::convert_to_wav(e, &audio.join(format!("{safe}.wav"))).is_ok() {
                                shipped_sounds.insert(e.name.clone());
                                want.remove(&e.name);
                            }
                        }
                        if !mine.is_empty() {
                            let _ = crate::haxe_gen::generate_sound_entries(&audio, id, &mine);
                            log::info!("{} sounds came across from the game's own movie", mine.len());
                        }
                    }
                }
            }
            if !want.is_empty() {
                log::warn!("sounds the stage's objects ask for that no package carries: {}",
                    want.iter().cloned().collect::<Vec<_>>().join(", "));
            }
        }
    }
    if !shipped_sounds.is_empty() {
        log::info!("{} sounds shipped with the stage", shipped_sounds.len());
    }
    let sprites = lib.join("sprites").join("Stage");
    std::fs::create_dir_all(&sprites).context("mkdir sprites/Stage")?;
    // The same picture is written ONCE, however many times the stage draws it. A source timeline
    // reuses art constantly -- a decoration that reappears each cycle, a backdrop shared between
    // two times of day, a splash every instance of the weather plays -- and each of those reaches
    // here as its own layer with its own suffix. Keyed by the encoded bytes, so it is exactly the
    // pictures that ARE the same, and the repeats become extra references to one image asset
    // rather than extra copies of it.
    let mut png_seen: std::collections::BTreeMap<Vec<u8>, String> = Default::default();
    let over_budget = std::cell::Cell::new(0usize);
    let mut write_layer = |suffix: &str, art: &StageArt| -> Result<ArtRef> {
        // The colour goes into the pixels where it can, and the frame stops carrying it -- so
        // nothing downstream applies it a SECOND time on a shader.
        let id_tint = art.tint.iter().zip(crate::stage_parser::TINT_ID)
            .all(|(a, b)| (a - b).abs() < 1e-4);
        let (png, tint) = if id_tint { (art.png.clone(), art.tint) } else {
            match bake_tint(&art.png, art.tint, art.w, art.h) {
                Some(p) => (p, crate::stage_parser::TINT_ID),
                None => { over_budget.set(over_budget.get() + 1); (art.png.clone(), art.tint) }
            }
        };
        let art = &StageArt { png, tint, ..art.clone() };
        let guid = match png_seen.get(&art.png) {
            Some(g) => g.clone(),
            None => {
                let guid = det_uuid(&format!("stage::{id}::{suffix}"));
                std::fs::write(sprites.join(format!("{id}_{suffix}.png")), &art.png)?;
                write_json(&sprites.join(format!("{id}_{suffix}.png.meta")), &json!({
                    "export": false, "guid": guid, "id": "", "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
                }))?;
                png_seen.insert(art.png.clone(), guid.clone());
                guid
            }
        };
        // a 1x1 fully-transparent cel is the parser's stand-in for a frame the source does not
        // place the object on; it is not art and must not ship as a picture
        let blank = art.w <= 1 && art.h <= 1
            && image::load_from_memory(&art.png).ok()
                .is_none_or(|i| i.to_rgba8().pixels().all(|p| p[3] == 0));
        // nothing to ship for a frame that draws nothing: no picture, no asset, no reference
        if blank {
            return Ok(ArtRef { blank, guid: String::new(), x: art.x, y: art.y, w: art.w, h: art.h,
                hold: art.hold.max(1) as usize, rotation: art.rotation, scale_x: art.scale_x,
                scale_y: art.scale_y, alpha: art.alpha, pivot: art.pivot, tint: art.tint });
        }
        Ok(ArtRef { blank, guid, x: art.x, y: art.y, w: art.w, h: art.h, hold: art.hold.max(1) as usize,
                    rotation: art.rotation, scale_x: art.scale_x, scale_y: art.scale_y, alpha: art.alpha,
                    pivot: art.pivot, tint: art.tint })
    };
    if std::env::var("PEPTIDE_ELEMTINT").is_ok() {
        for layer in model.art.background.iter() {
            let mut seen: std::collections::BTreeSet<String> = Default::default();
            for f in &layer.frames {
                if f.w <= 1 && f.h <= 1 { continue; }
                seen.insert(format!("{:?}", f.tint.map(|v| (v * 1000.0) as i64)));
            }
            eprintln!("ELEMTINT {:?} frames={} distinct_tints={}", layer.name, layer.frames.len(), seen.len());
        }
    }
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
    // the pictures its code draws from (`library_bitmaps`), and a white pixel for what it paints:
    // each an animation of the stage's own sprite (`bmp_<name>`, `px_white`), at the source's size
    let mut picture_refs: Vec<(String, ArtRef)> = Vec::new();
    if !model.library_bitmaps.is_empty() {
        for (name, art) in &model.library_bitmaps {
            let anim = format!("bmp_{}", crate::compile::ident(name));
            picture_refs.push((anim.clone(), write_layer(&anim, art)?));
        }
        let white = {
            use image::ImageEncoder;
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png).write_image(&[255u8, 255, 255, 255], 1, 1, image::ExtendedColorType::Rgba8)?;
            StageArt { png, w: 1, h: 1, ..Default::default() }
        };
        picture_refs.push(("px_white".to_string(), write_layer("px_white", &white)?));
    }
    let mut parallax_refs: Vec<ParallaxRef> = model.art.parallax.iter().enumerate()
        .map(|(i, layer)| write_layer(&format!("parallax{i}"), &layer.art)
            .map(|r| ParallaxRef { tracks: vec![vec![r]], w: layer.art.w, h: layer.art.h,
                                   mode: layer.mode, x_pan: layer.x_pan, y_pan: layer.y_pan,
                                   alpha_keys: Vec::new() }))
        .collect::<Result<_>>()?;
    // each backdrop ELEMENT is its own layer (the SSF2 movieclip model). a static element is one
    // frame (`bgN`); an animated element is its loop (`bgN_0..M`). draw order = list order.
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        for (i, layer) in model.art.background.iter().enumerate() {
            let f0 = &layer.frames[0];
            eprintln!("[bg-emit] {} name={:?} pan={:?} frames={} canvas={}x{} at=({:.1},{:.1}) holds={:?}",
                element_ids(&model.art.background)[i], layer.name, layer.pan, layer.frames.len(), f0.w, f0.h, f0.x, f0.y,
                layer.frames.iter().map(|f| f.hold).collect::<Vec<_>>());
        }
    }
    let elem_ids = element_ids(&model.art.background);
    let mut bg_refs: Vec<BgLayerRef> = Vec::new();
    for (i, layer) in model.art.background.iter().enumerate() {
        bg_refs.extend(build_element_layers(&elem_ids[i], bg_layer_name(&layer.name, i),
                                            layer.clip.clone(), &layer.named_path,
                                            &layer.frames, &layer.segments,
                                            if layer.behind_base { BgPlane::BackgroundBehind } else { BgPlane::Background },
                                            layer.pan, layer.fades, layer.authored_alpha, &layer.presence, layer.starts_hidden, &mut write_layer)?);
    }
    // The moving elements of the stage's OWN plane, on the same footing as the backdrop's: each
    // keeps its own loop instead of being frozen into the painted picture. They belong in FRONT of
    // that picture, because they were painted into it and it still carries what they stand on.
    {
        let ids = element_ids(&model.art.stage_elements);
        for (i, layer) in model.art.stage_elements.iter().enumerate() {
            bg_refs.extend(build_element_layers(&format!("sel_{}", ids[i]),
                                                bg_layer_name(&layer.name, i),
                                                layer.clip.clone(), &layer.named_path,
                                                &layer.frames, &layer.segments, BgPlane::Background,
                                                layer.pan, layer.fades, layer.authored_alpha,
                                                &layer.presence, layer.starts_hidden, &mut write_layer)?);
        }
    }
    // UNIVERSAL: promote every ANIMATED backdrop element (the weather embers, jumping podoboos,
    // lava bubbles/splashes, flickering torches, a spectating Bowser, …) to its own looping VFX
    // the stage spawns, instead of baking it as a layer inside the single `stage` animation where
    // every layer shares one master clock. that shared clock is the architectural bug: a long
    // element loop (a 260-frame Bowser) gets tiled/truncated to the master length and phase-jumps
    // each restart, and Flash's independent nested-movieclip loops + same-layer multiples can't be
    // represented at all. an animated backdrop element is exactly that case; a static (1-frame)
    // element has no loop, so it stays baked (cheaper). this is automatic, not flagged: any
    // animated bg element on any stage is treated this way. `PEPTIDE_BG_INLINE` forces the old
    // all-baked behavior for debugging/diffing.
    let inline_bg = std::env::var("PEPTIDE_BG_INLINE").is_ok();
    // Promotion moves an element out of the stage entity into a background CONTAINER, and the
    // containers draw in front of every baked IMAGE layer. So a baked element SSF2 draws in FRONT
    // of a promoted one has to move as well, or it vanishes behind it -- clocktown's wooden deck
    // (static, drawn over the animated town backdrop) was hidden by it completely. Layers behind
    // the first promoted element keep their place; the ones after it are emitted PAST the
    // containers instead, which restores the source order across the plane.
    // A PIECE THAT RIDES THE CAMERA IS ONE WHETHER OR NOT IT MOVES. Baking a still one into the
    // stage puts it at a place in the WORLD, and a camera background is not in the world: hueco
    // mundo's sky and its moon were parked hundreds of units above the play area, where the camera
    // never looks, so the stage lost its moon and most of its skyline.
    // A piece that ANIMATES becomes a container element; a piece that rides the CAMERA becomes a
    // camera background. Both leave the baked stack, but only the first kind changes what draws
    // over what: the containers draw in front of every baked layer, while a camera background goes
    // behind the lot. Counting a camera background as the first promoted layer pushed every baked
    // piece after it in front of the containers instead, which put bowser's castle's near
    // background over its torches and stopped them showing at all.
    let is_element = |l: &BgLayerRef| !inline_bg && l.frames.len() > 1;
    let is_promoted = |l: &BgLayerRef| is_element(l) || (!inline_bg && l.pan.is_some());
    let first_promoted = bg_refs.iter().position(&is_element);
    let (mut baked_bg, mut baked_front, mut promoted_bg) = (Vec::new(), Vec::new(), Vec::new());
    for (i, layer) in bg_refs.into_iter().enumerate() {
        if is_promoted(&layer) { promoted_bg.push(layer); }
        else if first_promoted.is_some_and(|p| i > p) { baked_front.push(layer); }
        else { baked_bg.push(layer); }
    }
    // an ANIMATED foreground element hits the SAME shared-master-clock bug as an animated
    // background one: baked into the stage entity it gets tiled/truncated to the master length
    // instead of running its own loop. Promotion was background-only, so it never reached the
    // foreground plane. Promote it on the same rule (more than one frame = it animates), routed
    // by its plane into a foreground container so it keeps drawing in front of the fighters.
    let fg_refs: Vec<ArtRef> = model.art.foreground.iter().enumerate()
        .map(|(i, a)| write_layer(&format!("fg{i}"), a)).collect::<Result<_>>()?;
    let promote_fg = !inline_bg && fg_refs.len() > 1;
    if promote_fg {
        promoted_bg.push(BgLayerRef { starts_hidden: false, presence: Vec::new(), named_path: Vec::new(), clip_path: Vec::new(),
            name: "Foreground Art".to_string(), eid: "fg".to_string(), clip: "fg".to_string(),
            frames: fg_refs.clone(), plane: BgPlane::Foreground, segments: Vec::new(), pan: None, fades: false, authored_alpha: model.art.foreground_alpha,
        });
    }
    // PER-ELEMENT foreground: the parser now splits the in-front plane the same way it splits the
    // backdrop, so each prop is promoted on its own instead of being flattened with the rest. The
    // old whole-plane composite above only ever promoted when the FLATTENED image animated, which
    // is a different (and much rarer) condition than "this plane contains something that animates"
    // — a plane of independently-moving props whose composite happens to be static stayed frozen.
    let fg_elem_ids = element_ids(&model.art.foreground_elements);
    if !inline_bg {
        for (i, layer) in model.art.foreground_elements.iter().enumerate() {
            // namespaced: ids come from the symbol name alone, and a plane's element can share a
            // name with a backdrop one — same id means same filenames, and one silently overwrites
            // the other's art.
            let eid = &format!("fge_{}", fg_elem_ids[i]);
            promoted_bg.extend(build_element_layers(eid, bg_layer_name(&layer.name, i),
                                                    layer.clip.clone(), &layer.named_path,
                                                    &layer.frames, &layer.segments,
                                                    BgPlane::Foreground, layer.pan, layer.fades, layer.authored_alpha, &layer.presence, layer.starts_hidden, &mut write_layer)?);
        }
    }
    // CLIPS THE STAGE'S CODE MAKES FROM THE LIBRARY AT RUN TIME: each ships as a vfx entity the
    // script makes on demand (`__libClip`), never spawned at boot; the table says which entities
    // draw for a linkage and where each piece's art sits against the clip's own origin
    let mut lib_entries: Vec<Value> = Vec::new();
    let mut lib_table: Vec<String> = Vec::new();
    // ...and the same by name, for the runtime's stage (which finds the clip's own symbol)
    let mut lib_rows: Vec<(String, String)> = Vec::new();
    // the linkage names the stage's code makes on demand: it adds and removes these at runtime (a
    // stream of butterflies flying across, spawned on a random timer and dropped off-screen), so
    // how many are alive at any instant is RNG-driven and never a fixed count. Recorded in the plan
    // so the cross-engine check verifies the port MAKES them, not that two independent random
    // populations hold the same head-count at one sampled frame.
    let mut on_demand_clips: Vec<String> = Vec::new();
    for (i, layer) in model.art.runtime_clips.iter().enumerate() {
        on_demand_clips.push(layer.name.clone());
        let refs = build_element_layers(&format!("lib_{i}"), layer.name.clone(), layer.clip.clone(), &[],
                                        &layer.frames, &[], BgPlane::Background, None, false, 1.0, &[], false, &mut write_layer)?;
        let mut parts: Vec<String> = Vec::new();
        for r in &refs {
            let eid = format!("{}_{}", model.id, r.eid);
            write_json(&lib.join("entities").join(format!("{eid}.entity")), &bg_element_entity(&eid, r, model.scale))?;
            write_meta(&lib.join("entities").join(format!("{eid}.entity.meta")), &eid, &eid, "", Some("VFX"), None)?;
            lib_entries.push(json!({ "namespace": "custom", "resourceId": model.id, "contentId": eid }));
            let (ax, ay) = bg_element_origin(r);
            let w = r.frames.iter().map(|f| f.x + f.w as f64 * model.scale).fold(f64::MIN, f64::max) - ax;
            parts.push(format!("{{ id: \"{eid}\", ax: {ax:.3}, ay: {ay:.3}, w: {:.3} }}", w.max(0.0)));
        }
        lib_table.push(format!("{{ n: \"{}\", parts: [{}] }}", layer.name.replace('\\', "\\\\").replace('"', "\\\""), parts.join(", ")));
        lib_rows.push((layer.name.clone(), parts.join(", ")));
    }
    // declared platforms become MOVING STRUCTURES (like the official stage-template's moving
    // platform): one shared grey `platformSprite` (an IMAGE + a structure LINE_SEGMENT, in the
    // stage entity), and one structure CONTENT per platform that the stage spawns and that moves
    // itself in its own Script (the sink/rise cycle). emit_platform_structures writes the per-
    // platform Stats + the shared Script + the grey sprite, and returns what the manifest + stage
    // Script need.
    // An element out of a CAMERA BACKGROUND is not spawned and driven by the stage: it becomes a
    // camera-background entry of its own, so the ENGINE plays its animation and scrolls it at the
    // declared pan (`ParallaxMode.PAN`). Doing it from the stage script would re-implement, in
    // hscript, the parallax the engine already has.
    // A camera background can be built ONE OF TWO WAYS. Composited into a single scrolling picture
    // it keeps the engine's own parallax, but every constituent shares one cel track: the composite
    // gains a boundary whenever ANY piece changes, each boundary repaints only part of the picture,
    // and pieces of one backdrop then turn over at different moments. Clocktown's ten pieces -- one
    // of them 1953 frames long -- collapse to fourteen pictures that way, and its clock face turns
    // ahead of the sky behind it.
    //
    // Kept SEPARATE, each piece is its own object on its own timeline, exactly as the source has
    // it, and each can be tinted and faded like any other element. What the engine then stops doing
    // is the scrolling, so this does it: one container for the lot, moved against the camera by the
    // pan the source declared.
    // This is how a camera background is built now. Composited, its pieces share one cel track and
    // one opacity, so none of them can FADE -- and the source's day/night change is a cross-fade,
    // a day copy of the backdrop going out while a night copy comes in. Separate, each piece keeps
    // its own timeline and its own fade, and the whole backdrop turns over together.
    // The ENGINE places and pans a camera background; we do not. Handing each piece to it as its
    // own entry keeps what it is good at -- depth, pan rate, centring against the view, holding the
    // pieces in register as the camera moves -- and still lets a piece fade, because the engine
    // makes a container per entry and a container's opacity is its own.
    //
    // Placing them by hand instead re-implemented all of that, badly: the pieces drifted out of
    // alignment, each got the plate's pan rate instead of its own, the clock turned against the sky
    // behind it, and zooming out showed the gaps between them.
    let separate_cam_bg = std::env::var("PEPTIDE_HANDPLACE_CAMBG").is_ok();
    // A PIECE WHOSE TIMELINE IS A STATE MACHINE IS PLACED BY HAND. The engine plays a camera
    // background's animation on its own and takes no jump; a piece that holds a section and
    // plays a transition when the stage says (castle siege's sky) needs its own script, so it is
    // an object in a panned container, moved against the camera by the rate the source declared.
    // A SECTIONED PIECE NOTHING CAN SEND ANYWHERE (no name over it for the code to reach) plays
    // from its first section on its own: where that section holds one picture, or loops on itself,
    // the piece is the background's own art, drawn by its entry. only a piece with an animation
    // of its own nested in the background is an effect
    // (one whose first section draws nothing and holds there draws nothing all match, and is left out)
    let mut never_drawn = Vec::new();
    for (k, l) in promoted_bg.iter_mut().enumerate().filter(|(_, l)| l.pan.is_some() && l.segments.len() > 1 && l.named_path.is_empty() && !l.starts_hidden) {
        let Some(chain) = segment_chain(l) else { continue };
        let Some((_, start, end, _)) = bg_segment_spans(l).into_iter().next() else { continue };
        if end <= start { continue }
        match chain.first().copied() {
            Some(None) if l.frames[start..end].iter().all(|f| f.blank) => never_drawn.push(k),
            Some(None) if still_frames(&l.frames[start..end]) => { l.frames = vec![l.frames[end - 1].clone()]; l.segments.clear(); }
            Some(Some(0)) => { l.frames = l.frames[start..end].to_vec(); l.segments.clear(); }
            _ => {}
        }
    }
    let promoted_bg: Vec<BgLayerRef> = promoted_bg.into_iter().enumerate()
        .filter(|(k, _)| !never_drawn.contains(k)).map(|(_, l)| l).collect();
    let (cam_bg_elems, mut promoted_bg): (Vec<BgLayerRef>, Vec<BgLayerRef>) =
        promoted_bg.into_iter().partition(|l| l.pan.is_some() && !separate_cam_bg && l.segments.len() <= 1);
    // the segmented pieces of the camera background: an object each, in the container of a
    // camera background entry of their own (blank, sized as the plate) so the engine places and
    // pans them with the rest
    let cam_objs: Vec<usize> = promoted_bg.iter().enumerate()
        .filter(|(_, l)| l.pan.is_some() && !separate_cam_bg).map(|(i, _)| i).collect();
    // Kept separate, the pieces still have to keep their ARRANGEMENT. Their coordinates are where
    // they sit in the world, and a camera background is not placed in the world -- it is placed
    // against the view. So each piece is made relative to the backdrop's own top-left, and the
    // container carries that corner: the pieces keep their positions relative to one another, and
    // the container is what the camera moves.
    // The PLATE, the same one the composite picks: the biggest piece that is solid throughout, not
    // the union of everywhere anything ever goes (a moon crossing the sky drags that to four
    // thousand pixels and centring it puts the whole backdrop off the screen).
    let pan_plate: Option<(f64, f64, u32, u32)> = if !separate_cam_bg { None } else {
        let pick = |fades: bool| promoted_bg.iter()
            .filter(|l| l.pan.is_some() && (fades || !l.fades))
            .flat_map(|l| l.frames.iter())
            .max_by_key(|f| f.w as u64 * f.h as u64)
            .map(|f| (f.x, f.y, f.w, f.h));
        pick(false).or_else(|| pick(true))
    };
    let pan_origin: Option<(f64, f64)> = pan_plate.map(|(x, y, _, _)| (x, y));
    if let Some((px, py)) = pan_origin {
        for l in promoted_bg.iter_mut().filter(|l| l.pan.is_some()) {
            // relative to the plate's corner, at the spacing the composite uses (the arrangement was
            // laid out in the world, at stage scale; the pieces are drawn at that scale already, so
            // the gaps between them must not be scaled a second time).
            for f in &mut l.frames {
                f.x = (f.x - px) / model.scale;
                f.y = (f.y - py) / model.scale;
            }
        }
    }
    // how big the backdrop is, once its pieces are relative to their own corner: the engine centres
    // a camera background on the view, so this is what has to be centred.
    let pan_size: (f64, f64) = pan_plate
        .map(|(_, _, w, h)| (w as f64, h as f64))
        .unwrap_or((0.0, 0.0));
    if !cam_bg_elems.is_empty() || !cam_objs.is_empty() {
        // ONE background holding every object of the source's backdrop, in draw order. Its size is
        // the whole picture's extent, not one object's, and each object keeps its place inside it:
        // the engine centres a background on the view, so anything emitted as its own background
        // would be re-centred and the arrangement lost.
        // The picture is the backdrop PLATE: the biggest object that is solid throughout. Neither
        // the union of everywhere anything ever goes (a moon crossing the sky, a fog sheet drawn
        // far wider than the view) nor simply the biggest object works -- the biggest is usually a
        // full-screen FADE sheet, which is invisible almost always, and anchoring the picture on
        // it leaves the actual backdrop off in a corner of an empty canvas.
        let biggest = |it: &mut dyn Iterator<Item = &ArtRef>| it.max_by_key(|f| f.w as u64 * f.h as u64).cloned();
        let cam_all = || cam_bg_elems.iter().chain(cam_objs.iter().map(|&i| &promoted_bg[i]));
        let plate = biggest(&mut cam_all().filter(|l| !l.fades).flat_map(|l| l.frames.iter()))
            .or_else(|| biggest(&mut cam_all().flat_map(|l| l.frames.iter())));
        let plate = plate.as_ref();
        if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
            for l in &cam_bg_elems {
                let w = l.frames.iter().map(|f| f.w).max().unwrap_or(0);
                let h = l.frames.iter().map(|f| f.h).max().unwrap_or(0);
                eprintln!("[cambg] {:32} fades={} {}x{} frames={}", l.name, l.fades, w, h, l.frames.len());
            }
            eprintln!("[cambg] plate = {:?}", plate.map(|f| (f.w, f.h, f.x, f.y)));
        }
        let (w, h) = plate.map(|f| (f.w, f.h)).unwrap_or((1, 1));
        let (l, t) = plate.map(|f| (f.x, f.y)).unwrap_or((0.0, 0.0));
        // Every object's position, from the picture's own top-left and in the picture's own
        // PIXELS. The track positions are in stage units (they were laid out against the world),
        // while a camera background draws its images unscaled and lets the layer transform do the
        // scaling -- so they have to come back to pixels or the arrangement is off by the stage
        // scale.
        // ONE ENTRY PER PIECE, not one entry for the lot. The engine makes a container per entry
        // and a container's opacity is its own, so pieces kept apart can CROSS-FADE -- a day copy
        // going out while a night copy comes in -- which a single shared container cannot do.
        // Every entry carries the PLATE's size, so the engine centres them all on the same point
        // and the arrangement between the pieces survives being split. Each also keeps its OWN pan
        // rate, which collapsing them onto the first piece's rate used to throw away.
        for layer in cam_bg_elems.iter() {
            let track: Vec<ArtRef> = layer.frames.iter().cloned()
                .map(|mut f| { f.x = (f.x - l) / model.scale; f.y = (f.y - t) / model.scale; f })
                .collect();
            let (x_pan, y_pan) = layer.pan.unwrap_or((0.0, 0.0));
            parallax_refs.push(ParallaxRef {
                tracks: vec![track], w, h, mode: ParallaxMode::Pan, x_pan, y_pan,
                alpha_keys: alpha_keyframes(&layer.frames),
            });
        }
        // A PIECE THAT HOLDS A SECTION AND JUMPS is an object in the container of an entry of
        // its own: the entry carries nothing but the plate's size and the piece's rate, so the
        // engine makes a container placed and panned like the others, and the object sits in it
        // at the piece's place in the picture's own pixels, switched by its script.
        for &i in &cam_objs {
            let layer = &mut promoted_bg[i];
            let (x_pan, y_pan) = layer.pan.take().unwrap_or((0.0, 0.0));
            let blank = ArtRef { guid: String::new(), x: 0.0, y: 0.0, w, h, hold: 1, alpha: 1.0, pivot: (0.0, 0.0),
                blank: true, rotation: 0.0, scale_x: 1.0, scale_y: 1.0, tint: [1.0, 1.0, 1.0, 0.0, 0.0, 0.0] };
            parallax_refs.push(ParallaxRef { tracks: vec![vec![blank]], w, h, mode: ParallaxMode::Pan, x_pan, y_pan, alpha_keys: Vec::new() });
            // in the picture's own pixels from its corner, and then from its CENTRE, which is
            // where the engine anchors the entry (the plate's frames get the same shift when the
            // stage entity is built)
            for f in &mut layer.frames {
                f.x = (f.x - l) / model.scale - (w as f64) / 2.0;
                f.y = (f.y - t) / model.scale - (h as f64) / 2.0;
            }
            layer.plane = BgPlane::CameraEntry(parallax_refs.len() - 1);
        }
    }
    let (platform_sprites, structure_contents, structure_spawn_ids) =
        emit_platform_structures(model, &lib, &sprites)?;
    // the mask picture is written here so the stage entity can carry it as an animation
    let shadow_mask_ref = match &model.art.shadow_mask {
        Some(m) => Some(write_layer("shadowmask", m)?),
        None => None,
    };
    let art = ArtRefs {
        pictures: picture_refs,
        shadow_mask: shadow_mask_ref.clone(),
        background: baked_bg,
        background_front: baked_front,
        parallax: parallax_refs,
        stage: stage_refs,
        // emptied when the foreground was promoted to its own looping VFX above, so it isn't
        // drawn twice (once on the master clock, once on its own).
        foreground: if promote_fg { vec![] } else { fg_refs },
        foreground_is_blend_overlay: model.art.foreground_is_blend_overlay,
        foreground_alpha: model.art.foreground_alpha,
        foreground_occluders: model.art.foreground_occluders.iter().enumerate()
            .map(|(i, a)| write_layer(&format!("fgocc{i}"), a)).collect::<Result<_>>()?,
        platform_sprites,
    };

    // a named surface a sectioned piece carries on its own frames (a phase's terrain sliding in) is
    // that piece's: it leaves the stage entity and goes into the piece's animations
    let carriers = surface_carriers(model, &promoted_bg);
    let carried_names: BTreeSet<String> = carriers.keys().cloned().collect();
    let (entity, plat_segs, carried) = build_entity(model, &art, &carried_names);
    // FM-side validation: the built entity must carry the layers a Fraymakers stage needs
    // (these are invariants of build_entity; the check guards against regressions).
    let mut missing = validate_fm_entity(&entity);
    // a stage whose floors are all placed bodies draws no floor line of its own: the decks are
    // there from the first frame, made by the boot (steel diver)
    if model.hazards.iter().any(|h| h.behavior.self_platform.is_some() || !h.body_faces.is_empty()) {
        missing.retain(|m| !m.contains("floor line"));
    }
    if !missing.is_empty() {
        bail!("emitted {id}.entity is missing required Fraymakers stage layers: {}", missing.join(", "));
    }
    write_json(&lib.join("entities").join(format!("{id}.entity")), &entity)?;

    // hazards (custom game objects) the stage spawns, if any are declared for this stage.
    let hazard_entries = emit_hazards(model, &lib)?;
    // every animated backdrop element becomes its own looping VFX the stage spawns + reparents
    // into a background container. VFX carry no manifest entry (resolved by id via getContent),
    // so this only returns spawn lines.
    // THE PLAN: everything the stage script is written to make, as the script is written. The
    // verify gates read this back against what the running script reports it made, so the
    // question "did the port come up" is answered by the code itself and never by a walk that
    // can miss a container or a join that can pick the wrong thing at one position.
    let mut plan: Vec<Value> = Vec::new();
    // clips the stage makes on demand at runtime: recorded so a cross-engine check treats their
    // head-count as RNG-driven (verify they are made, not that two random populations match)
    for clip in &on_demand_clips {
        plan.push(json!({ "kind": "on_demand", "clip": clip }));
    }
    // the camera backgrounds: animations of the stage entity, where no object of their own shows
    // them; the plan states each one's length so a checker can still count them
    for (i, p) in art.parallax.iter().enumerate() {
        let frames = p.tracks.iter().map(|tr| element_len(tr)).max().unwrap_or(0);
        plan.push(json!({ "kind": "parallax", "id": format!("parallax{i}"), "frames": frames }));
    }
    // what the package declares and this port leaves out, said in the plan so a check against
    // the source reads it as left out rather than missing
    for (class, base, clip) in &model.deferred {
        plan.push(json!({ "kind": "deferred", "class": class, "base": base, "clip": clip }));
    }
    // the stage's own painted art: how long it runs here, and how long the terrain clip's timeline
    // runs in the source; a timeline that draws one picture comes through as one
    // ...and the stills painted into it, by the clip each came from: a plane whose own timeline
    // runs many frames over fixed plates is compared by its children, not its length
    let painted: Vec<Value> = model.art.background.iter().chain(model.art.stage_elements.iter()).chain(model.art.foreground_elements.iter())
        .filter(|l| l.frames.len() <= 2)
        .map(|l| json!({ "clip": l.clip, "frames": l.frames.len() })).collect();
    plan.push(json!({ "kind": "stage", "frames": element_len(&art.stage), "source_frames": model.terrain_frames,
                      "plane_art_moves": model.terrain_art_moves, "painted": painted }));
    set_clashing_handles(model);
    // THE BOOT, as data: everything the stage puts down as the match starts, in the order it puts
    // it down (`DATA.stage.boot`, made by `runtime/stage/boot.hx`)
    let mut boot = StageTables { objects_declared: model.hazards.len(), ..Default::default() };
    let bg_entries = emit_bg_elements(model, &promoted_bg, &lib, &mut plan, &mut boot, &carriers, &carried)?;

    write_json(&lib.join("manifest.json"), &build_manifest(model, &[hazard_entries.clone(), bg_entries.clone(), lib_entries.clone()].concat(), &structure_contents))?;
    write_meta(&lib.join("manifest.json.meta"), id, "manifest", "json", None, None)?;

    let scripts = lib.join("scripts").join("stage");
    // the stage spawns its moving-platform structures + hazards in its Script.
    // A structure the stage's own code drives is filed under the name it drives it by; the rest
    // are simply put down.
    boot.structures = structure_spawn_ids.iter().enumerate()
        .map(|(i, (cid, name))| {
            plan.push(json!({ "kind": "structure", "id": cid, "var": format!("_st{i}") }));
            BootStructure { id: cid.clone(), name: name.clone() }
        })
        .collect();
    // What goes behind the hazards switch is READ OFF THE SOURCE STAGE, not decided here: SSF2
    // stages ask the switch themselves and each one means something different by it, so the port
    // gates exactly what the original gated (`abc_parser::extract_hazard_gate`) and nothing else.
    // A stage that never asks runs everything regardless, and so does its port.
    boot.objects = hazard_boot(model, &mut plan);
    // A camera background that FADES drives the container the engine made for it. The containers
    // come back in the order the entries were declared, so entry i is container i.
    // A piece the source draws faintly is not a piece the source draws. One with a single key has
    // no fade to run, but it still has an OPACITY, and skipping it left the engine's default: fully
    // opaque. Nineteen pieces with fifteen schedules meant four of clocktown's backdrop drawn at
    // full strength whatever the source asked for, which is most of why the port sat brighter than
    // the game all day.
    boot.cam_bg = art.parallax.iter().enumerate()
        .filter(|(_, p)| !p.alpha_keys.is_empty())
        .map(|(i, p)| if p.alpha_keys.len() > 1 {
            BootCamBg::Fade { index: i, len: p.tracks.first().map(|t| element_len(t)).unwrap_or(0), keys: p.alpha_keys.clone() }
        } else {
            // one key: a constant opacity, set once
            BootCamBg::Alpha { index: i, alpha: p.alpha_keys[0].rsplit(',').next().unwrap_or("1.0")
                .trim_end_matches(']').trim().to_string() }
        })
        .collect();
    boot.weather = emit_weather(model, &lib)?;
    for (i, w) in model.weathers.iter().enumerate() {
        plan.push(json!({ "kind": "weather", "id": format!("{}_weather{i}", model.id),
                          "var": format!("m_weather{i}"), "count": w.count.min(WEATHER_MAX_PARTICLES) }));
    }
    // THE STAGE'S SCRIPT: the stage's own class run by the runtime, over the boot above as data,
    // and each object with a class of its own run by that class (`emit::stage`).
    let objects: Vec<(String, String)> = model.hazards.iter().enumerate()
        .filter(|(_, hz)| hz.instance_name.as_deref().is_none_or(str::is_empty))
        .filter_map(|(i, hz)| Some((hz.class_name.clone().filter(|c| !c.is_empty())?, hazard_id(&model.id, i, hz))))
        .fold(Vec::new(), |mut v: Vec<(String, String)>, (c, id)| {
            // one row per class: several placed objects can share one
            if !v.iter().any(|(k, _)| *k == c) { v.push((c, id)); }
            v
        });
    let (frictions, pushes) = surface_rows(model);
    // each object with a class of its own runs that class: the ones the stage's own code
    // makes are left to that code, and a placed body carries the outline its clip draws
    let rows: Vec<crate::emit::stage::ObjectRow> = model.hazards.iter().enumerate().filter_map(|(i, hz)| {
        let class = hz.class_name.clone().filter(|c| !c.is_empty())?;
        let placed_body = !hz.declared_kind.is_empty() && hz.behavior.self_platform.is_some();
        let body = placed_body.then(|| {
            let (bx, by, bw, bh) = hz.behavior.self_platform.unwrap_or_default();
            let s = model.scale;
            crate::emit::stage::Body { bounds: (bx * s, by * s, (bx + bw) * s, (by + bh) * s),
                faces: hz.body_faces.clone(), ledges: hz.ledges }
        });
        Some(crate::emit::stage::ObjectRow {
            index: i, id: hazard_id(&model.id, i, hz), class,
            clips: [hz.linkage_id.clone(), hz.art_clip.clone()].into_iter().flatten().collect(),
            by_code: recycles_itself(hz, model),
            // placed on the stage's timeline (a clip that declares its class): where it was put
            at: (hz.instance_name.is_some() || body.is_some() || !hz.declared_kind.is_empty()).then_some((hz.x, hz.y)),
            // a single picture is drawn centred on the object, which the boot puts at the
            // middle of its box; made by code, the object is where its clip's registration is,
            // and the box sits off that by the box's own place in the clip
            registration: (hz.anims.is_empty() && hz.instance_name.is_none() && body.is_none() && hz.declared_kind.is_empty())
                .then(|| hz.attack_boxes.first().map(|b| (b.x + b.w / 2.0, b.y + b.h / 2.0))).flatten(),
            body,
        })
    }).collect();
    let input = crate::emit::stage::StageInput {
        follow_light: model.stage_script.as_deref().is_some_and(light_follows).then(|| light_place(model)),
        tables: &boot, anim: STAGE_ANIM, pan: pan_size, objects,
        surfaces: &plat_segs, frictions, pushes,
        particles: model.weathers.iter().enumerate().filter(|(_, w)| w.count > 0)
            .map(|(i, w)| (w.linkage.clone(), format!("{}_weather{i}", model.id))).collect(),
        particle_cap: WEATHER_MAX_PARTICLES,
        lib: lib_rows,
        placed_at: placed_at_table(model),
        origin: entity_origin(model),
        rows,
    };
    // (a model made by hand, with no package behind it, has no classes to run: it gets none)
    let (objects, played) = if model.source.as_os_str().is_empty() {
        log::warn!("{id}: no package to read the stage's classes from, so it is written with no script");
        (Vec::new(), Default::default())
    } else {
        let (out, objects, played) = crate::emit::stage::runtime_stage(model, &dir, &input)
            .with_context(|| format!("{id}: the stage's script"))?;
        log::info!("{id}: the stage script runs on the runtime ({} problems, {} unknown names, {} objects on their own classes, see runtime_report.json)",
            out.problems.len(), out.unknown.len(), objects.len());
        (objects, played)
    };
    // an object the stage's own code makes over and over runs its own class, body and all: the
    // pipeline's followers standing in for its body stay parked off the world, with nothing to
    // follow
    let recycled = model.hazards.iter().enumerate()
        .any(|(i, hz)| recycles_itself(hz, model) && hz.behavior.self_platform.is_some() && objects.contains(&hazard_id(&model.id, i, hz)));
    let followers = lib.join("scripts").join("platform").join(format!("{id}thwompdeckScript.hx"));
    if recycled && followers.exists() { write_script(&followers, &platform_script_hx())?; }
    // ...and the boot does not stand it up: it comes and goes as the stage's code makes it, so
    // the plan says so
    for row in plan.iter_mut().filter(|r| r["kind"] == "object") {
        let made_by_code = model.hazards.iter().enumerate().any(|(i, hz)| recycles_itself(hz, model)
            && row["id"].as_str() == Some(hazard_id(&model.id, i, hz).as_str()) && objects.iter().any(|o| row["id"].as_str() == Some(o.as_str())));
        if made_by_code { row["kind"] = json!("spawned"); }
    }
    // WHAT THE STAGE'S OWN CODE DRIVES, said in the plan: a picture the class sends to a frame
    // (a platform's holder shown by its height) or that rides an object keeps the frame the
    // code picks, not the engine's count, and a check reads it as such rather than as a loop
    {
        for row in plan.iter_mut() {
            if row["kind"] != "vfx" { continue; }
            let names: Vec<String> = row["names"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let clip = row["clip"].as_str().unwrap_or("").to_string();
            if row["rides"].is_string() || played.contains(&clip) || names.iter().any(|n| played.contains(n)) {
                row["driven"] = json!(true);
            }
        }
    }
    write_meta(&scripts.join(format!("{id}Script.hx.meta")), id, &format!("{id}Script"), "", Some("STAGE"), None)?;
    // The SHADOW MASK, as the engine takes it: a mask sprite named on a shadow layer. SSF2 keeps
    // the same idea in a plane it never draws, so this is that shape carried across rather than
    // anything invented -- and without it a stage that says where its shadows fall gets none.
    // ONE shadow layer, and the stage gets shadows. The engine builds a shadow layer per entry
    // here and casts into it from the stage's light; with the list empty there is nothing to cast
    // into and a fighter stands on the ground with no shadow, however good the light is. The
    // layer masks itself with the animation written above, so shadows land on ground and nowhere
    // else. The sprite is named the way the engine keys its cache: namespace, resource, content --
    // and a converted stage is always custom content, so the namespace is the custom one.
    // the plate the synthesised mask places once per surface. Written whenever the stage has no
    // mask plane of its own, which is nearly all of them.
    if model.art.shadow_mask.is_none() {
        let guid = mask_plate_guid(id);
        std::fs::write(sprites.join(format!("{id}_shadowmask.png")),
                       white_plate(MASK_PLATE_PX, MASK_PLATE_PX)).context("write shadow mask plate")?;
        write_json(&sprites.join(format!("{id}_shadowmask.png.meta")), &json!({
            "export": false, "guid": guid, "id": "", "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
        }))?;
    }
    let mask_rects = shadow_mask_rects(model);
    let shadow_layers = if model.art.shadow_mask.is_none() && mask_rects.is_empty() {
        // nothing to mask with and nothing to derive one from: an unmasked layer would let
        // shadows fall through open sky, so the stage goes without rather than wrong.
        log::warn!("{id}: no walkable surface to hang a shadow mask on, so this stage gets no shadows");
        String::new()
    } else {
        // COLOUR CARRIES THE OPACITY. The engine takes the shadow's alpha from the top byte of
        // this colour and its ink from the rest, so a layer without one is a black shadow drawn
        // at zero opacity: built, placed under the fighter, and invisible. Black at just under
        // half strength is the shipped look.
        format!("\n\t\t{{ id: \"shadow\", foreground: false, color: 0x7f000000, maskSpriteContent: \"private::{id}.{id}\", maskAnimationId: \"{SHADOW_MASK_ANIM}\" }}\n\t")
    };
    write_script(&scripts.join(format!("{id}StageStats.hx")), &stage_stats_hx(id, &art.parallax, &shadow_layers))?;
    write_meta(&scripts.join(format!("{id}StageStats.hx.meta")), id, &format!("{id}StageStats"), "hscript", None, None)?;

    crate::content_audit::audit_and_log(&lib, id);

    // WHICH SSF2 OBJECT EACH EMITTED ELEMENT CAME FROM. Written beside the project rather than
    // inside the library, so it ships with nothing. Without it the only way to pair the two sides
    // is by guessing from names, and an element named for its symbol id pairs with nothing: a
    // backdrop whose pieces DO animate reads as a backdrop that lost all of them, which is a gap
    // that is not there and hides the ones that are.
    {
        let mut sources: Vec<Value> = Vec::new();
        let mut note = |eid: &str, layers: &[crate::stage_parser::BgLayer], i: usize| {
            if let Some(l) = layers.get(i) {
                // WHERE it is, as well as what it is called. A name pairs the two sides only
                // when both sides use the same one, and an element whose symbol carries no name
                // gets a made-up id that pairs with nothing. Position is the same on both sides
                // whatever either calls it.
                let (x, y) = l.frames.first().map(|f| (f.x, f.y)).unwrap_or((0.0, 0.0));
                sources.push(json!({ "element": eid, "from": l.name, "clip": l.clip,
                                     "frames": l.frames.len(), "x": x.round(), "y": y.round() }));
            }
        };
        for (i, e) in element_ids(&model.art.background).iter().enumerate() {
            note(e, &model.art.background, i);
        }
        for (i, e) in element_ids(&model.art.stage_elements).iter().enumerate() {
            note(&format!("sel_{e}"), &model.art.stage_elements, i);
        }
        for (i, e) in element_ids(&model.art.foreground_elements).iter().enumerate() {
            note(&format!("fge_{e}"), &model.art.foreground_elements, i);
        }
        // AND THE OBJECTS. A hazard is emitted as an entity named for its CLASS, while the source
        // knows it by the clip it draws with -- so a clip ported perfectly well as an object had
        // nothing to pair with and was counted as a lost animation. Emerald cave's wooden panel
        // is one: ten labelled frames, ported as an object with ten states, reported lost.
        let objects: Vec<Value> = model.hazards.iter().map(|hz| json!({
            "entity": format!("{}{}", model.id, hz.label),
            "class":  hz.class_name.clone().unwrap_or_default(),
            "draws":  hz.linkage_id.clone().unwrap_or_default(),
            "label":  hz.label,
        })).collect();
        write_json(&dir.join(format!("{id}.sources.json")),
                   &json!({ "elements": sources, "objects": objects }))?;
    }

    let fraytools = dir.join(format!("{id}.fraytools"));
    write_json(&fraytools, &build_fraytools())?;
    write_json(&dir.join("verify.json"), &verify_plan(id, &lib, &plan, model))?;

    if over_budget.get() > 0 {
        log::warn!("{} tinted stage pictures too large to colour in their pixels keep the tint on the frame",
            over_budget.get());
    }
    Ok((dir, fraytools))
}

/// The plan the verify gates hold a running stage to: what its script makes, the scripts the
/// package must carry, and how much art it ships. Written by the converter from the same values
/// it emitted, so there is exactly one statement of what the port is supposed to be.
fn verify_plan(id: &str, lib: &Path, made: &[Value], model: &StageModel) -> Value {
    let mut scripts: Vec<String> = Vec::new();
    let mut images = 0usize;
    let mut bytes = 0u64;
    fn walk(dir: &Path, scripts: &mut Vec<String>, images: &mut usize, bytes: &mut u64) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { walk(&p, scripts, images, bytes); continue; }
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".hx.meta") {
                if let Ok(v) = std::fs::read_to_string(&p).map(|s| serde_json::from_str::<Value>(&s)) {
                    if let Some(sid) = v.ok().and_then(|v| v["id"].as_str().map(str::to_string)) { scripts.push(sid); }
                }
            } else if name.ends_with(".png") {
                *images += 1;
                *bytes += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    walk(lib, &mut scripts, &mut images, &mut bytes);
    scripts.sort();
    json!({
        "id": id,
        "made": made,
        "scripts": scripts,
        "assets": { "images": images, "bytes": bytes },
        "hazards": model.hazards.len(),
        "weathers": model.weathers.len(),
        "platforms": model.platforms.len(),
        // every clip class the package declares, by local name: what a checker pairs a source
        // row by, and how it tells the game's own drop-ins (an item) from the stage's clips
        "symbols": model.symbol_names.iter().map(|n| n.rsplit('.').next().unwrap_or(n).to_string()).collect::<std::collections::BTreeSet<_>>(),
        // the port's coordinate model, so a gate can put the source's numbers beside the port's:
        // fm = (source - origin) * scale. `origin` is the one the objects' scripts do their sums
        // in; `fighter_origin` is the stage-world space a fighter's position is read in, which is
        // the terrain clip's registration (the source lays its world out from the terrain), and
        // the objects' one only where no terrain states it.
        "scale": model.scale,
        "origin": [entity_origin(model).0, entity_origin(model).1],
        "fighter_origin": model.terrain_off.map(|(x, y)| json!([-x, -y]))
            .unwrap_or_else(|| json!([entity_origin(model).0, entity_origin(model).1])),
        "art_origin": [model.origin.0, model.origin.1],
        "entrances": model.entrances.iter().map(|e| json!({ "index": e.index, "x": e.x, "y": e.y })).collect::<Vec<_>>(),
    })
}

/// Write a JSON file, and if it is an entity, write it compacted.
///
/// A stage emits entities from a dozen places (the stage itself, each hazard, each backdrop
/// element, each segment), so the compaction hangs off the write rather than off any one
/// builder -- see [`crate::entity_compact`]. It only removes redundancy, so what the stage
/// draws is unchanged.
fn write_json(path: &Path, v: &Value) -> Result<()> {
    let text = if path.extension().and_then(|e| e.to_str()) == Some("entity") {
        let mut entity = v.clone();
        let st = crate::entity_compact::compact(&mut entity);
        if st.saved_anything() {
            log::debug!("compacted {}: symbols {} -> {}, keyframes {} -> {} ({} blanked)",
                path.file_name().unwrap_or_default().to_string_lossy(),
                st.symbols_before, st.symbols_after, st.keyframes_before, st.keyframes_after,
                st.blanked);
        }
        serde_json::to_string_pretty(&entity)?
    } else {
        serde_json::to_string_pretty(v)?
    };
    std::fs::write(path, text).with_context(|| format!("write {}", path.display()))
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
    parallax_anims: Vec<(String, Vec<String>)>,
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


    /// The SHADOW MASK, as an animation of the stage's own sprite.
    ///
    /// The engine takes a mask as a sprite plus the animation to read it from, and the official
    /// stage keeps it as an animation of the STAGE (`shadowMask`, and `shadowMaskFront` for the
    /// one in front) rather than a sprite of its own. Handing over a separate entity means naming
    /// an animation the stage does not have, so there is nothing to mask with and no shadow lands.
    fn add_shadow_mask_anim(&mut self, name: &str, guid: &str, x: f64, y: f64) {
        let sym = self.uid(&format!("sym:{name}"));
        self.symbols.push(json!({
            "$id": sym, "type": "IMAGE", "imageAsset": guid, "alpha": 1.0,
            "x": x, "y": y, "scaleX": self.scale, "scaleY": self.scale,
            "rotation": 0, "pivotX": 0, "pivotY": 0, "pluginMetadata": {}
        }));
        let kf = self.uid(&format!("kf:{name}"));
        self.keyframes.push(json!({ "$id": kf, "length": 1, "pluginMetadata": {}, "symbol": sym,
            "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" }));
        let lid = self.uid(&format!("layer:{name}"));
        self.layers.push(json!({
            "$id": lid, "hidden": false, "locked": false, "name": "Shadow Mask", "type": "IMAGE",
            "keyframes": [kf], "pluginMetadata": {}
        }));
        let aid = self.uid(&format!("anim:{name}"));
        self.extra_anims.push(json!({
            "$id": aid, "name": name, "pluginMetadata": {}, "layers": [lid]
        }));
    }

    /// The shadow mask SYNTHESISED from the stage's own walkable surfaces.
    ///
    /// Most SSF2 stages carry no mask plane, because SSF2 draws its shadows as art. The mask is
    /// still derivable from the stage rather than invented: a shadow lands on ground, and the
    /// ground is exactly the collision the converter already worked out. So each walkable surface
    /// contributes one plate covering its top face, and the union of them is where shadows are
    /// allowed to fall.
    fn add_shadow_mask_rects(&mut self, name: &str, guid: &str, rects: &[(f64, f64, f64, f64)]) {
        let mut lids = Vec::new();
        for (i, (x, y, w, h)) in rects.iter().enumerate() {
            let sym = self.uid(&format!("sym:{name}:{i}"));
            self.symbols.push(json!({
                "$id": sym, "type": "IMAGE", "imageAsset": guid, "alpha": 1.0,
                "x": x, "y": y,
                "scaleX": w / MASK_PLATE_PX as f64, "scaleY": h / MASK_PLATE_PX as f64,
                "rotation": 0, "pivotX": 0, "pivotY": 0, "pluginMetadata": {}
            }));
            let kf = self.uid(&format!("kf:{name}:{i}"));
            self.keyframes.push(json!({ "$id": kf, "length": 1, "pluginMetadata": {}, "symbol": sym,
                "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" }));
            let lid = self.uid(&format!("layer:{name}:{i}"));
            self.layers.push(json!({
                "$id": lid, "hidden": false, "locked": false,
                "name": format!("Shadow Mask {i}"), "type": "IMAGE",
                "keyframes": [kf], "pluginMetadata": {}
            }));
            lids.push(lid);
        }
        if lids.is_empty() { return; }
        let aid = self.uid(&format!("anim:{name}"));
        self.extra_anims.push(json!({
            "$id": aid, "name": name, "pluginMetadata": {}, "layers": lids
        }));
    }

    /// The stage's LIGHT: a point the engine casts each fighter's shadow away from.
    ///
    /// Fraymakers works out a shadow from a light source and the shadow layers it names; with no
    /// light there is nothing to cast from, so a stage that declares where its shadows fall still
    /// gets none. SSF2 has no light to carry across -- it draws shadows as art -- so this is the
    /// stage's own geometry standing in for one: overhead, centred on the play area, which is
    /// where a light has to be for shadows to fall under fighters rather than off to one side.
    fn add_shadow_light(&mut self, x: f64, y: f64, layer_ids: &[String], fade: (f64, f64)) {
        // the SAME symbol a working point carries. A point built without its alpha, colour and
        // rotation comes out of the build as `[x, y, null, null]` where the engine wants four
        // numbers -- the entrance points that do work read `[x, y, 1, 90]` -- so the light was
        // being written as a point the engine cannot read.
        let sym = self.uid("sym:shadowlight");
        self.symbols.push(json!({
            "$id": sym, "type": "POINT", "alpha": 1, "color": "0xff0000", "pluginMetadata": {},
            "x": x, "y": y, "rotation": 0
        }));
        let kf = self.uid("kf:shadowlight");
        self.keyframes.push(json!({ "$id": kf, "length": self.frame_len, "pluginMetadata": {},
            "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "POINT" }));
        let lid = self.uid("layer:shadowlight");
        self.layers.push(json!({
            "$id": lid, "hidden": false, "locked": false, "name": "Shadow Light 0", "type": "POINT",
            "keyframes": [kf],
            // EXACTLY the fields the official stage's light carries, and no others. The engine
            // reads this metadata POSITIONALLY (it is a virtual object, so the field order is the
            // field SET), which means an extra key of our own shifts every read past it and the
            // light stops being read as a light. The defaults it does not spell out are the ones
            // it wants; an empty layer list means every shadow layer on the stage.
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": {
                "index": 0,
                "pointType": "SHADOW_LIGHT_POINT",
                // The FADE RADII, and a shadow does not exist without them. The engine kills a
                // shadow outright when the light's end radius squared is under its distance
                // squared -- and the default end radius is -1, so 1 < any distance and every
                // shadow is forced to nothing. Left unwritten they are not "no fade", they are
                // "no shadow, anywhere". These come off the stage's own boxes: full strength out
                // to the far corner of the camera, fading to nothing by the blast zone.
                "shadowFadeStartRadius": fade.0,
                "shadowFadeEndRadius": fade.1,
                "shadowLayerIds": layer_ids
            }}
        }));
        self.anim_layers.push(lid);
    }

    /// A LINE_SEGMENT layer (walkable surface). `points` is the polyline (>= 2 points; a
    /// flat floor is 2, a curved/sloped terrain traces its surface). `pm` is the per-symbol
    /// FraymakersMetadata (structureType + ledge/dropThrough flags).
    /// Emit a terrain surface polyline split into TYPED runs: a segment steeper than
    /// `WALL_STEEPNESS_DEG` from horizontal is a wall (the side a fighter is blocked from
    /// depends on travel direction: surfaces trace left to right with the solid below, so a
    /// descending steep segment is the landmass's right face = RIGHT_WALL, an ascending one
    /// its left face = LEFT_WALL); everything else is floor. Without the split the whole
    /// profile is FLOOR and fighters slide up steep side faces instead of being blocked.
    /// Ledge grabs attach only to the outer ends of the first/last floor run.
    fn add_typed_surface(&mut self, name: &str, points: &[(f64, f64)], ledges: (bool, bool), drop_through: bool) {
        const WALL_STEEPNESS_DEG: f64 = 75.0;
        let steep = WALL_STEEPNESS_DEG.to_radians().tan();
        // classify each segment, then merge consecutive same-type segments into runs.
        let mut runs: Vec<(&'static str, Vec<(f64, f64)>)> = Vec::new();
        for w in points.windows(2) {
            let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let ty = if dy.abs() > dx.abs() * steep {
                if dy > 0.0 { "RIGHT_WALL" } else { "LEFT_WALL" }
            } else {
                "FLOOR"
            };
            match runs.last_mut() {
                Some((t, run)) if *t == ty => run.push(w[1]),
                _ => runs.push((ty, vec![w[0], w[1]])),
            }
        }
        let floor_idx: Vec<usize> = runs.iter().enumerate()
            .filter(|(_, (t, _))| *t == "FLOOR").map(|(i, _)| i).collect();
        for (i, (ty, run)) in runs.iter().enumerate() {
            let left_ledge = ledges.0 && floor_idx.first() == Some(&i);
            let right_ledge = ledges.1 && floor_idx.last() == Some(&i);
            let label = if runs.len() == 1 { name.to_string() } else { format!("{name} {} {}", ty.to_lowercase().replace('_', " "), i) };
            self.add_line_segment(&label, run, json!({
                "structureType": ty, "leftLedge": left_ledge, "rightLedge": right_ledge,
                "dropThrough": drop_through
            }));
        }
    }

    /// A solid body: the whole outline, not just the surface you land on.
    ///
    /// A FLOOR only stops a fighter coming down onto it, so terrain emitted as its top line alone
    /// is walked on from above and passed straight through from below or the side. SSF2 terrain is
    /// a solid body -- you cannot rise through the walkways of a stage, only through the soft
    /// platform that is authored to be passed. So the top is the floor (carrying the profile of a
    /// sloped surface and its ledges), the sides are walls, and the underside is its own CEILING
    /// structure, which is the only thing that stops a jump from below.
    fn add_solid_body(&mut self, name: &str, top: &[(f64, f64)], bottom: f64, ledges: (bool, bool)) {
        if top.len() < 2 { return; }
        self.add_typed_surface(name, top, ledges, false);
        let (lx, rx) = (top[0].0, top[top.len() - 1].0);
        let (ly, ry) = (top[0].1, top[top.len() - 1].1);
        // a body with no thickness has no underside to hit
        if bottom - ly.max(ry) < 1.0 { return; }
        for (label, ty, pts) in [
            (format!("{name} right wall"), "RIGHT_WALL", vec![(rx, ry), (rx, bottom)]),
            (format!("{name} ceiling"), "CEILING", vec![(rx, bottom), (lx, bottom)]),
            (format!("{name} left wall"), "LEFT_WALL", vec![(lx, bottom), (lx, ly)]),
        ] {
            self.add_line_segment(&label, &pts, json!({
                "structureType": ty, "leftLedge": false, "rightLedge": false, "dropThrough": false
            }));
        }
    }

    /// The line segments laid down since layer `from`, taken back out of the entity: each one's
    /// name, points and metadata.
    fn take_layers(&mut self, from: usize) -> SurfaceSegs {
        let taken: Vec<Value> = self.layers.drain(from..).collect();
        let mut out = Vec::new();
        for l in &taken {
            let lid = l["$id"].as_str().unwrap_or_default().to_string();
            self.anim_layers.retain(|x| *x != lid);
            for k in l["keyframes"].as_array().into_iter().flatten() {
                let Some(kf) = self.keyframes.iter().position(|x| x["$id"] == *k) else { continue };
                let kf = self.keyframes.remove(kf);
                let Some(si) = self.symbols.iter().position(|x| x["$id"] == kf["symbol"]) else { continue };
                let sym = self.symbols.remove(si);
                let flat: Vec<f64> = sym["points"].as_array().into_iter().flatten().filter_map(|v| v.as_f64()).collect();
                let pts = flat.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0], c[1])).collect();
                out.push((l["name"].as_str().unwrap_or_default().to_string(), pts,
                    sym["pluginMetadata"]["com.fraymakers.FraymakersMetadata"].clone()));
            }
        }
        out
    }

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

    /// Build a `platformSprite` animation whose collision is SOLID ON ALL SIDES: one line-segment
    /// structure layer carrying a CLOSED clockwise rect outline (floor -> right wall -> ceiling ->
    /// left wall; segment winding sets each face's collision side, the same way the official
    /// content builds solid fences from wall/ceiling segments). Local coords: top edge at y=0
    /// (same convention as the floor-line variant, so a follower script's offsets are unchanged),
    /// spanning x -w/2..w/2 and down to y=h. No grabbable ledges — it's a solid body, not a deck.
    fn add_solid_structure_animation(&mut self, idx: usize, sprite_guid: &str, w: f64, h: f64) {
        let img = self.make_image(&format!("platformImage{idx}"), sprite_guid, -w / 2.0, 0.0, 1.0);
        let pm = json!({ "structureType": "FLOOR", "leftLedge": false, "rightLedge": false, "dropThrough": false });
        let sym = self.uid("sym:platformSolid");
        self.symbols.push(json!({
            "$id": sym, "type": "LINE_SEGMENT", "alpha": 0.5, "color": "0xeeeeee",
            "points": [
                -w / 2.0, 0.0,  w / 2.0, 0.0,   // floor (left -> right)
                 w / 2.0, h,                    // right wall (down)
                -w / 2.0, h,                    // ceiling (right -> left)
                -w / 2.0, 0.0                   // left wall (up, closing the loop)
            ],
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": pm }
        }));
        let kf = self.uid("kf:platformSolid");
        self.keyframes.push(json!({ "$id": kf, "length": 1, "pluginMetadata": {}, "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "LINE_SEGMENT" }));
        let line = self.uid("layer:platformSolid");
        self.layers.push(json!({
            "$id": line, "hidden": false, "locked": false, "name": "Solid Outline Layer", "type": "LINE_SEGMENT",
            "keyframes": [kf], "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "lineSegmentType": "LINE_SEGMENT_STRUCTURE" } }
        }));
        let aid = self.uid(&format!("anim:platformSprite{idx}"));
        self.extra_anims.push(json!({
            "$id": aid, "name": format!("platformSprite{idx}"), "pluginMetadata": {}, "layers": [img, line]
        }));
    }

    /// Build a `platformSprite` animation whose collision is a CEILING segment along the bottom
    /// edge of the box (y=h): a FLOOR-typed structure never blocks an upward mover (one-way
    /// platform semantics), so a solid body's underside must be its own CEILING-typed structure —
    /// the official content ships separate Ceiling structures for its solid fences the same way.
    fn add_ceiling_structure_animation(&mut self, idx: usize, sprite_guid: &str, w: f64, h: f64) {
        let img = self.make_image(&format!("platformImage{idx}"), sprite_guid, -w / 2.0, 0.0, 1.0);
        let pm = json!({ "structureType": "CEILING", "leftLedge": false, "rightLedge": false, "dropThrough": false });
        let sym = self.uid("sym:platformCeiling");
        self.symbols.push(json!({
            "$id": sym, "type": "LINE_SEGMENT", "alpha": 0.5, "color": "0xeeeeee",
            "points": [w / 2.0, h, -w / 2.0, h],
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": pm }
        }));
        let kf = self.uid("kf:platformCeiling");
        self.keyframes.push(json!({ "$id": kf, "length": 1, "pluginMetadata": {}, "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "LINE_SEGMENT" }));
        let line = self.uid("layer:platformCeiling");
        self.layers.push(json!({
            "$id": line, "hidden": false, "locked": false, "name": "Ceiling Layer", "type": "LINE_SEGMENT",
            "keyframes": [kf], "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "lineSegmentType": "LINE_SEGMENT_STRUCTURE" } }
        }));
        let aid = self.uid(&format!("anim:platformSprite{idx}"));
        self.extra_anims.push(json!({
            "$id": aid, "name": format!("platformSprite{idx}"), "pluginMetadata": {}, "layers": [img, line]
        }));
    }

    /// Add an IMAGE layer to the `stage` animation at the current depth (rendered at the
    /// stage `scale` so the native-resolution art matches the scaled-up geometry).
    /// The stage animation's frame labels: the terrain clip's sections, at twice their source
    /// frame, so the stage's own code can jump its painted art the way the source jumps its planes.
    fn add_label_layer(&mut self, labels: &[(String, u16)]) {
        let mut marks: Vec<(usize, String)> = labels.iter()
            .map(|(l, f)| ((*f as usize) * 2, sanitize_anim(l))).filter(|(f, _)| *f < self.frame_len).collect();
        marks.sort_by_key(|(f, _)| *f);
        marks.dedup_by_key(|(f, _)| *f);
        if marks.is_empty() { return; }
        let mut kfs: Vec<String> = Vec::new();
        let mut cursor = 0usize;
        for (i, (f, name)) in marks.iter().enumerate() {
            if *f > cursor {
                let kf = self.uid(&format!("kf:label-gap:{i}"));
                self.keyframes.push(json!({ "$id": kf, "type": "LABEL", "length": f - cursor, "name": "", "pluginMetadata": {} }));
                kfs.push(kf);
            }
            let end = marks.get(i + 1).map(|(n, _)| *n).unwrap_or(self.frame_len);
            let kf = self.uid(&format!("kf:label:{name}"));
            self.keyframes.push(json!({ "$id": kf, "type": "LABEL", "length": end - f, "name": name, "pluginMetadata": {} }));
            kfs.push(kf);
            cursor = end;
        }
        let lid = self.uid("layer:labels");
        self.layers.push(json!({ "$id": lid, "hidden": false, "locked": false, "name": "Labels", "type": "LABEL",
                                 "keyframes": kfs, "pluginMetadata": {} }));
        self.anim_layers.push(lid);
    }
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
        self.parallax_anims.push((format!("parallax{idx}"), vec![lid]));
    }
    /// An ANIMATED camera background: the same per-frame layer an animated stage layer gets, but
    /// registered as its own `parallax{idx}` animation. It has to live in the STAGE entity like
    /// the still ones do -- a background entry pointed at a separate entity does not draw.
    /// A camera background ported as a TIMELINE: each keyframe carries that frame's own image,
    /// position, rotation and alpha, so the source's fades and turns survive as transforms instead
    /// of being baked into a picture per frame.
    ///
    /// `cx`/`cy` centre the layer on its own origin (the engine anchors a background at the centre
    /// of the view), and each frame's own offset rides on top of that.
    fn add_image_tracks_parallax(&mut self, idx: usize, cx: f64, cy: f64, tracks: &[Vec<ArtRef>]) {
        // ONE camera background holding one layer per source object. The engine centres a
        // background layer on the view, so emitting each object as its own background would pile
        // them all on the same point and lose the arrangement -- the town, its clock and its moon
        // sit at different places in one picture, which is exactly what a single animation with
        // several layers expresses.
        let mut lids = Vec::new();
        for (t, frames) in tracks.iter().enumerate() {
            let lid = self.parallax_track_layer(&format!("Parallax {idx}.{t}"), cx, cy, frames);
            lids.push(lid);
        }
        self.parallax_anims.push((format!("parallax{idx}"), lids));
    }

    /// One object's track as an IMAGE layer: a keyframe per change carrying that frame's image,
    /// position, rotation and alpha. Returns the layer id (the caller decides what it belongs to).
    fn parallax_track_layer(&mut self, name: &str, cx: f64, cy: f64, frames: &[ArtRef]) -> String {
        let name = name.to_string();
        let target = self.frame_len.max(1);
        let mut kfs = Vec::new();
        let mut filled = 0usize;
        let mut sym_of: std::collections::HashMap<usize, String> = Default::default();
        'tile: loop {
            for (i, a) in frames.iter().enumerate() {
                let mut len = a.hold.max(1);
                if filled + len > target { len = target - filled; }
                if len == 0 { break 'tile; }
                // one symbol per distinct FRAME of the track, made the first time it is laid down
                let sym = if a.blank { String::new() } else { sym_of.entry(i).or_insert_with(|| {
                    self.seq += 1;
                    let sym = det_uuid(&format!("stage::{}::sym:{name}:{i}::{}", self.id, self.seq));
                    self.symbols.push(json!({
                        "$id": sym, "type": "IMAGE", "imageAsset": a.guid, "alpha": a.alpha,
                        "x": cx + a.x, "y": cy + a.y, "scaleX": a.scale_x, "scaleY": a.scale_y,
                        "rotation": a.rotation, "pivotX": a.pivot.0, "pivotY": a.pivot.1,
                        "pluginMetadata": {}
                    }));
                    sym
                }).clone() };
                self.seq += 1;
                let kf = det_uuid(&format!("stage::{}::kf:{name}:{filled}:{i}::{}", self.id, self.seq));
                // a frame the source does not place the object on carries no symbol at all
                let mut kfj = json!({ "$id": kf, "length": len, "pluginMetadata": {},
                    "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" });
                if !a.blank { kfj["symbol"] = json!(sym); }
                self.keyframes.push(kfj);
                kfs.push(kf);
                filled += len;
                if filled >= target { break 'tile; }
            }
            if frames.is_empty() { break; }
        }
        let lid = self.uid(&format!("layer:{name}"));
        self.layers.push(json!({
            "$id": lid, "hidden": false, "locked": false, "name": name, "type": "IMAGE",
            "keyframes": kfs, "pluginMetadata": {}
        }));
        lid
    }

    /// Add an animated IMAGE layer to the `stage` animation: one keyframe per frame, each
    /// referencing that frame's image, so the layer plays through them (loops).
    fn add_image_frames(&mut self, name: &str, frames: &[(String, f64, f64, usize)]) {
        self.add_image_frames_alpha(name, frames, 1.0);
    }
    /// `add_image_frames` with an explicit symbol alpha (a semi-transparent animated overlay).
    /// Every layer must SPAN the whole stage animation (`frame_len`): a layer whose keyframes end
    /// early goes blank/frozen for the rest of each loop (the "tint flashing" / dead-layer bug).
    /// A single frame becomes one static full-length keyframe; a shorter cycle TILES (repeats,
    /// truncating the last repeat) so it keeps playing for the whole loop like an SSF2 movieclip.
    fn add_image_frames_alpha(&mut self, name: &str, frames: &[(String, f64, f64, usize)], alpha: f64) {
        self.add_image_frames_scaled(name, frames, alpha, self.scale);
    }
    fn add_image_frames_scaled(&mut self, name: &str, frames: &[(String, f64, f64, usize)], alpha: f64, sym_scale: f64) {
        let mut syms = Vec::new();
        for (i, (guid, x, y, _)) in frames.iter().enumerate() {
            let sym = self.uid(&format!("sym:{name}:{i}"));
            self.symbols.push(json!({
                "$id": sym, "type": "IMAGE", "imageAsset": guid, "alpha": alpha,
                "x": x, "y": y, "scaleX": sym_scale, "scaleY": sym_scale, "rotation": 0, "pivotX": 0, "pivotY": 0,
                "pluginMetadata": {}
            }));
            syms.push(sym);
        }
        let cycle: usize = frames.iter().map(|(_, _, _, h)| (*h).max(1)).sum();
        let target = self.frame_len.max(1);
        let mut kfs = Vec::new();
        if frames.len() == 1 {
            // static: one keyframe spanning the whole animation.
            let kf = self.uid(&format!("kf:{name}:0"));
            self.keyframes.push(json!({ "$id": kf, "length": target, "pluginMetadata": {}, "symbol": syms[0], "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" }));
            kfs.push(kf);
        } else {
            // animated: per-frame hold = the run length of identical source frames (RLE), so held
            // frames read as pauses and the loop runs at the SSF2 pace. Tile cycles to fill.
            let mut filled = 0usize;
            let mut rep = 0usize;
            'tile: while filled < target {
                for (i, (_, _, _, hold)) in frames.iter().enumerate() {
                    let mut len = (*hold).max(1);
                    if filled + len > target { len = target - filled; }
                    if len == 0 { break 'tile; }
                    let kf = self.uid(&format!("kf:{name}:{rep}:{i}"));
                    self.keyframes.push(json!({ "$id": kf, "length": len, "pluginMetadata": {}, "symbol": syms[i], "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" }));
                    kfs.push(kf);
                    filled += len;
                    if filled >= target { break 'tile; }
                }
                rep += 1;
                // safety: a degenerate zero-length cycle can't fill anything.
                if cycle == 0 || rep > target { break; }
            }
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
#[derive(Clone)]
struct ArtRef { guid: String, x: f64, y: f64, w: u32, h: u32, hold: usize, alpha: f64, pivot: (f64, f64),
    /// The source does not place the object on this frame. Emitted as a keyframe with NO symbol,
    /// which is how "nothing is here" is expressed; it used to ship a 1x1 transparent picture as
    /// a stand-in, which is a sprite the source does not have and reads as art that draws nothing.
    blank: bool,
    /// Orientation for this frame. Non-zero when the element is a few cels the source MOVES or
    /// TURNS: the art is emitted once per cel and oriented here, rather than baked per angle.
    rotation: f64, scale_x: f64, scale_y: f64,
    /// The frame's colour transform (rgb multiply + offset), driven onto a shader at runtime rather
    /// than baked, so a day-to-night slide stays a slide instead of a step per cel.
    tint: [f64; 6] }

/// The frames on which a layer's opacity CHANGES, as `[frame, alpha]` pairs. A backdrop that
/// holds one opacity throughout yields a single pair (or none); one that cross-fades yields the
/// whole ramp, which is what a fade has to be driven from.
/// How long the element's own animation is, in emitted frames. Its opacity schedule is played
/// against this rather than against the stage's clock: the element's VFX loops here, so a schedule
/// keyed to the stage would slide against the very animation it is meant to be gating.
fn element_len(frames: &[ArtRef]) -> usize {
    frames.iter().map(|f| f.hold.max(1)).sum::<usize>() * crate::entity_gen::FRAME_RATIO as usize
}

fn alpha_keyframes(frames: &[ArtRef]) -> Vec<String> {
    // An alpha track says how OPAQUE something is while it is drawn. It does not say WHETHER it is
    // drawn: an element the source puts down on some frames and not others is handled by presence,
    // and writing its gaps here as alpha 0 turns it into a strobe. Clocktown's rain did exactly
    // that -- 1690 alternations between 1 and 0 on one element -- and the backdrop flickered white
    // through the day change.
    //
    // The two are told apart by what is in the sequence, not by what the element is: a FADE passes
    // through the middle, presence never does. With no intermediate value there is nothing to fade,
    // so there is no track to emit.
    // Only the frames where the element is actually DRAWN have an opacity worth carrying. A frame
    // it sits out is baked into its own animation as a blank, so its absence is already played --
    // writing that down again here as alpha 0 gives the element a second clock. Its animation then
    // runs on its own loop while its opacity runs on the stage's, the two slide apart, and the
    // element is gated on at moments its animation is not at (clocktown's rain splashes never
    // finished a splash, and every one of them was gated by the same stage clock so they clustered).
    let drawn = |f: &ArtRef| f.w > 1 || f.h > 1;
    // IN THE EMITTED CLOCK. The source runs at 30fps and every emitted keyframe is doubled to sit
    // on the engine's 60, so a schedule written in source frames finishes twice over while the art
    // it is fading goes round once, and each key lands at half the frame it belongs to. That is
    // opacity permanently out of step with the picture: the clock face fading while its own cel is
    // somewhere else, and the day's white flash asked for while the backdrop is not at it.
    const RATIO: usize = crate::entity_gen::FRAME_RATIO as usize;
    let mut keys: Vec<String> = Vec::new();
    let (mut at, mut prev) = (0usize, None::<f64>);
    for f in frames {
        if drawn(f) && prev.is_none_or(|p: f64| (p - f.alpha).abs() > 1e-3) {
            keys.push(format!("[{at}, {:.4}]", f.alpha));
            prev = Some(f.alpha);
        }
        at += f.hold.max(1) * RATIO;
    }
    keys
}

/// A parallax camera-background layer: the written sprite + its scroll mode + pan rate.
/// One camera-background entry: the content + animation the engine scrolls, and how.
///
/// Both kinds of layer end up here. A still backdrop is an IMAGE layer of the stage entity
/// (`parallax{i}`); an ANIMATED one is its own element entity, referenced by its content id and
/// its `active` animation, so the engine plays it AND scrolls it. Driving an animated layer from
/// the stage script instead would be a second, parallel implementation of what the engine's own
/// parallax already does.
struct ParallaxRef {
    /// The background's objects, back to front, each as its own track of keyframes. A still
    /// backdrop is one track of one frame. All of them live in the STAGE entity as ONE
    /// `parallax{i}` animation: a background entry pointed at a separate entity does not draw,
    /// and one entry per object would centre each of them on the same point.
    tracks: Vec<Vec<ArtRef>>,
    w: u32,
    h: u32,
    mode: ParallaxMode,
    x_pan: f64,
    y_pan: f64,
    /// When this entry's opacity CHANGES over the source timeline: `[frame, alpha]` pairs. The
    /// engine gives one container per background entry, and a container's alpha is settable, so a
    /// backdrop that cross-fades keeps its fade by driving its own container.
    alpha_keys: Vec<String>,
}

/// One backdrop element as its own layer: a display name + its frame sequence (1 = static).
struct BgLayerRef {
    name: String, eid: String, frames: Vec<ArtRef>, plane: BgPlane,
    /// Which CLIP INSTANCE this is part of (see `BgLayer::clip`). A stage's display tree is named
    /// clips, and one clip is one object -- so parts of the same clip, whether the parser split
    /// them by animating depth or the plate/moving split did, are LAYERS of one entity.
    clip: String,
    /// Every instance name over the element, outermost first (see `BgLayer::named_path`).
    named_path: Vec<String>,
    /// ...with the clips nobody named that have a class of their own (`#<id>`) among them: what a
    /// native stage's elements table files the piece under, so that clip's code reaches it
    clip_path: Vec<String>,
    /// The element's label-delimited segments as `(label, frame count, jump)`, when its SSF2 clip
    /// branches between them instead of looping. Frame counts index into `frames`, which is the
    /// segments concatenated in timeline order. Empty for an ordinary looping element.
    segments: Vec<(String, usize, Option<crate::abc_parser::LabelJump>)>,
    /// `true` when the source animates this object's opacity (see `BgLayer::fades`).
    fades: bool,
    /// The opacity the source authored for this element (see `BgLayer::authored_alpha`).
    authored_alpha: f64,
    /// When the stage's timeline shows this object (see `BgLayer::presence`).
    presence: Vec<(usize, usize)>,
    /// Set when the element came out of a CAMERA BACKGROUND: the fraction of the camera's movement
    /// it scrolls by. The stage drives it from the live camera each frame, which is how an animated
    /// camera-background object stays animated AND stays in the parallax.
    pan: Option<(f64, f64)>,
    /// the clip starts hidden, drawn once the stage plays a label on it
    starts_hidden: bool,
}

/// The source plane a promoted element came from. Decides which stage container it's
/// reparented into and at what alpha, so a promoted element lands at the same depth the
/// baked version occupied in the entity's layer stack (background art draws behind the
/// stage art; foreground art draws in front of the fighters, semi-transparent).
#[derive(Clone, Copy, PartialEq, Debug)]
enum BgPlane { BackgroundBehind, Background, Foreground,
    /// inside the engine's container for camera background entry `n`: a piece of a backdrop
    /// whose timeline is a state machine, placed and panned by the engine like the plate
    /// beside it, and switched by its own script
    CameraEntry(usize) }

impl BgPlane {
    /// The Stage container API call that puts an element at this plane's depth.
    fn container(self) -> &'static str {
        match self {
            // IN FRONT of the baked `Stage Art` layer. The elements pulled out of the near
            // background are things that sit ON the stage -- rain splashing on its boards, fire in
            // its braziers -- and the baked layer they were pulled out of still carries the ground
            // they land on. Parented behind it they are drawn and then covered by it.
            // BEHIND the baked `Stage Art` layer, for an element the source draws under it: a
            // brazier's fire belongs behind the basket, and parented in front it covers the
            // lattice that should be silhouetted against it.
            BgPlane::BackgroundBehind => "getBackgroundBehindContainer",
            BgPlane::Background => "getBackgroundContainer",
            // the lowest of the four foreground containers, i.e. directly above
            // `Characters Front` — where the baked `Foreground Art` layer sat.
            BgPlane::Foreground => "getForegroundStructuresContainer",
            // never a looping piece: only a segmented one is routed here, and it takes boot_at
            BgPlane::CameraEntry(_) => "getBackgroundBehindContainer",
        }
    }
    /// The source's name for the plane a piece in this container came from.
    fn source_plane(self) -> &'static str {
        match self {
            BgPlane::BackgroundBehind | BgPlane::CameraEntry(_) => "background",
            BgPlane::Background => "midground",
            BgPlane::Foreground => "foreground",
        }
    }
    /// Matches the alpha the baked path used for this plane (add_image_frames_alpha).
    fn alpha(self) -> f64 {
        match self { BgPlane::Background | BgPlane::BackgroundBehind | BgPlane::CameraEntry(_) => 1.0, BgPlane::Foreground => 0.5 }
    }
    /// where the boot parents a looping element's container (the stage container `container`
    /// names, by the boot tables' name for it)
    fn boot_plane(self) -> &'static str {
        match self {
            BgPlane::BackgroundBehind | BgPlane::CameraEntry(_) => "behind",
            BgPlane::Background => "background",
            BgPlane::Foreground => "foreground",
        }
    }
    /// where the boot parents a segmented element's view root
    fn boot_at(self) -> BootAt {
        match self {
            BgPlane::CameraEntry(n) => BootAt::Camera(n),
            other => BootAt::Plane(other.boot_plane()),
        }
    }
}

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
struct ArtRefs { background: Vec<BgLayerRef>,
    /// the pictures its code draws from, and the white pixel it paints with, by animation name
    pictures: Vec<(String, ArtRef)>,
    /// Baked background layers that sit IN FRONT of a promoted (container-hosted) element, so they
    /// are laid out after the background containers instead of before them.
    background_front: Vec<BgLayerRef>,
    parallax: Vec<ParallaxRef>, stage: Vec<ArtRef>, foreground: Vec<ArtRef>, foreground_occluders: Vec<ArtRef>, platform_sprites: Vec<(String, f64, f64, u8)>,
    /// see `StageArtSet::foreground_is_blend_overlay`
    foreground_is_blend_overlay: bool,
    /// see `StageArtSet::foreground_alpha`
    foreground_alpha: f64,
    /// The shadow mask picture, to go on the stage's own `shadowMask` animation.
    shadow_mask: Option<ArtRef> }

/// Render the floor + soft platforms as filled rectangles on a transparent canvas
/// covering their bounding box (1px = 1 stage unit). Gives the stage visible content
/// (required for play) and shows the playable geometry.
/// Draw a stand-in for a stage that converted with no art of its own: its collision surfaces, as
/// coloured bands, so the stage is something you can see and stand on rather than an invisible
/// plane. Solid ground and drop-through platforms are drawn in different colours.
///
/// The one rule this has to keep is that the picture sits exactly on the collision it was drawn
/// from. Everything below is in service of that, and each part of it has been got wrong once.
fn render_placeholder(model: &StageModel) -> StageArt {
    use image::{Rgba, RgbaImage};

    // Nothing to draw from. A stage with neither art nor collision is degenerate, but it must not
    // produce a bogus placement: the bounds below would come back inverted (from the empty folds)
    // and land an image at 1e308, which is not a number any consumer of this recovers from.
    if model.platforms.is_empty() {
        let img = RgbaImage::new(1, 1);
        let mut png = Vec::new();
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), 1, 1, image::ExtendedColorType::Rgba8)
            .expect("encode placeholder png");
        return StageArt { png, x: 0.0, y: 0.0, w: 1, h: 1, hold: 1, rotation: 0.0, scale_x: 1.0, scale_y: 1.0, alpha: 1.0 , pivot: (0.0, 0.0), tint: crate::stage_parser::TINT_ID };
    }

    // bounding box of all collision geometry, with a small margin.
    let rects: Vec<Rect> = model.platforms.iter().map(|p| p.rect).collect();
    let margin = 8.0;
    let min_x = rects.iter().map(|r| r.left()).fold(f64::MAX, f64::min) - margin;
    let min_y = rects.iter().map(|r| r.top()).fold(f64::MAX, f64::min) - margin;
    let max_x = rects.iter().map(|r| r.right()).fold(f64::MIN, f64::max) + margin;
    let max_y = rects.iter().map(|r| r.bottom()).fold(f64::MIN, f64::max) + margin;

    // Drawn in SOURCE units, not FM units. Every stage image is placed at the stage scale, so a
    // raster already measured in FM units gets that scale applied a SECOND time: the art comes out
    // `scale` too wide and slides off its own collision, which reads as collision covering only
    // part of the shape and a floor line that does not sit on the floor. The collision geometry is
    // in FM units, so divide by the scale here and let the one placement put it back.
    let s = if model.scale > 0.0 { model.scale } else { 1.0 };
    // Generous, because a raster that does not cover its own bounds is a raster drawn in the wrong
    // place: the position and scale say it spans the whole stage, so anything cropped off the far
    // edge is simply missing rather than merely smaller.
    let w = (((max_x - min_x) / s).ceil() as u32).clamp(1, PLACEHOLDER_MAX_PX);
    let h = (((max_y - min_y) / s).ceil() as u32).clamp(1, PLACEHOLDER_MAX_PX);
    let mut img = RgbaImage::new(w, h);
    for p in &model.platforms {
        let r = &p.rect;
        let color = if p.drop_through { Rgba([120, 160, 220, 235]) } else { Rgba([90, 100, 120, 255]) };
        // Rounded OUTWARD, and never empty. A surface thinner than one pixel once divided by the
        // scale would otherwise round to a zero-height span and vanish -- which is how a stage ends
        // up with collision a fighter stands on and nothing drawn under their feet.
        let x0 = (((r.left() - min_x) / s).floor().max(0.0) as u32).min(w - 1);
        let y0 = (((r.top() - min_y) / s).floor().max(0.0) as u32).min(h - 1);
        let x1 = (((r.right() - min_x) / s).ceil().max(0.0) as u32).clamp(x0 + 1, w);
        let y1 = (((r.bottom() - min_y) / s).ceil().max(0.0) as u32).clamp(y0 + 1, h);
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
    // Position stays in FM units: a placement's x/y are already parent-space coordinates and only
    // the image's PIXELS are scaled. Size is what had to change, not where it sits.
    StageArt { png, x: min_x, y: min_y, w, h, hold: 1, rotation: 0.0, scale_x: 1.0, scale_y: 1.0, alpha: 1.0 , pivot: (0.0, 0.0), tint: crate::stage_parser::TINT_ID }
}

/// How large a placeholder raster may get. Big enough for every stage in the corpus at source
/// resolution; the clamp exists so a corrupt model cannot ask for a terabyte of pixels.
const PLACEHOLDER_MAX_PX: u32 = 8192;

/// The image the synthesised mask plates all share. One asset, placed once per surface.
fn mask_plate_guid(id: &str) -> String { det_uuid(&format!("stage::{id}::shadowmask")) }

/// Where shadows are allowed to land: the top face of every surface a fighter can stand on.
///
/// Returned as (x, y, w, h) plates in stage space. A shadow falls on ground, and the ground is
/// collision the converter has already worked out, so this reads the stage rather than inventing
/// a region. Surfaces the stage spawns as moving structures are left out: those carry their own
/// art and move, so a plate pinned to where they started would mask the wrong place.
fn shadow_mask_rects(model: &StageModel) -> Vec<(f64, f64, f64, f64)> {
    // deep enough that a shadow cast down a slope still lands inside the plate, and the underside
    // of a body is covered rather than cut off at its lip.
    const PLATE_DEPTH: f64 = 64.0;
    model.platforms.iter().filter(|p| !p.visible && !platform_is_driven(p, model)).map(|p| {
        let r = &p.rect;
        (r.left(), r.top(), r.w.max(1.0), (r.h).max(PLATE_DEPTH))
    }).collect()
}

/// A surface's segments as the stage entity would carry them: (layer name, points, metadata).
type SurfaceSegs = Vec<(String, Vec<(f64, f64)>, Value)>;

fn build_entity(model: &StageModel, art: &ArtRefs, carried_names: &BTreeSet<String>)
    -> (Value, Vec<(String, Vec<String>)>, BTreeMap<String, SurfaceSegs>) {
    let mut carried: BTreeMap<String, SurfaceSegs> = BTreeMap::new();
    // the layers each NAMED surface came out as, so the stage's own code can reach it by name
    let mut plat_segs: Vec<(String, Vec<String>)> = Vec::new();
    let id = &model.id;
    let mut b = EntityBuilder::new(id);
    b.scale = model.scale;
    // the stage animation loops for the SUM of the per-frame holds of the richest animated layer
    // (each frame carries its own hold = the run length of identical source frames at FM's
    // 60fps), so the loop matches the SSF2 duration. a static layer holds for the whole loop.
    let layer_len = |refs: &[ArtRef]| refs.iter().map(|a| a.hold).sum::<usize>();
    let bg_max_len = art.background.iter().chain(&art.background_front).map(|l| layer_len(&l.frames)).max().unwrap_or(0);
    // the camera backgrounds count too: a stage's longest timeline is often the backdrop's own
    // (clocktown's day/night cycle), and leaving it out cuts that cycle to whatever the nearer
    // layers happened to need.
    let px_max_len = art.parallax.iter().flat_map(|p| p.tracks.iter()).map(|t| layer_len(t)).max().unwrap_or(0);
    b.frame_len = bg_max_len.max(px_max_len).max(layer_len(&art.stage)).max(1);
    if !model.terrain_labels.is_empty() { b.add_label_layer(&model.terrain_labels); }

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
    b.add_container("Background Structures", "BACKGROUND_STRUCTURES_CONTAINER");
    // baked art the source draws over a promoted element (see the split in `emit_stage`)
    for layer in &art.background_front {
        match layer.frames.as_slice() {
            [] => {}
            [a] => b.add_image(&layer.name, &a.guid, a.x, a.y),
            frames => b.add_image_frames(&layer.name, &frames.iter().map(|a| (a.guid.clone(), a.x, a.y, a.hold)).collect::<Vec<_>>()),
        }
    }
    if !art.stage.is_empty() {
        let frames: Vec<(String, f64, f64, usize)> = art.stage.iter().map(|a| (a.guid.clone(), a.x, a.y, a.hold)).collect();
        b.add_image_frames("Stage Art", &frames);
    }
    // ...and the SHADOWS go over the ground, not under it.
    //
    // A shadow is drawn into this container, so anything emitted after it covers the shadow up.
    // The ground a fighter stands on was emitted after it -- the whole point of a shadow is to
    // land on that ground, and it was landing behind it. The official stage puts nothing between
    // this container and the fighters, which is the same statement.
    b.add_container("Background Shadows", "BACKGROUND_SHADOWS_CONTAINER");
    b.add_container("Background Effects", "BACKGROUND_EFFECTS_CONTAINER");
    // moving platforms reference the shared `platformSprite` animation (grey surface + a structure
    // line segment); the stage spawns them as Structures that move themselves. Add that animation.
    for (i, (grey, w, h, kind)) in art.platform_sprites.iter().enumerate() {
        if *kind == 1 { b.add_solid_structure_animation(i, grey, *w, *h); continue; }
        if *kind == 2 { b.add_ceiling_structure_animation(i, grey, *w, *h); continue; }
        b.add_platform_animation(i, grey, *w, *h);
    }
    b.add_container("Characters Back", "CHARACTERS_BACK_CONTAINER");
    b.add_container("Characters", "CHARACTERS_CONTAINER");
    b.add_container("Characters Front", "CHARACTERS_FRONT_CONTAINER");
    // opaque foreground occluders (a standable structure's near face SSF2 split off — bowscastle's
    // bridge parapet) draw IN FRONT of fighters at FULL alpha, so a fighter on the deck is occluded
    // from the front. Behind the semi-transparent tint below (the red light reddens the parapet too).
    if !art.foreground_occluders.is_empty() {
        b.add_image_frames("Foreground Occluder",
            &art.foreground_occluders.iter().map(|a| (a.guid.clone(), a.x, a.y, a.hold)).collect::<Vec<_>>());
    }
    // the foreground (bowserscastle's lava-glow sheet + lightmask, classified foreground by the
    // AS3 plane map) draws IN FRONT of fighters — the REAL clip art, one keyframe per real frame
    // (no synthesized flicker), at the opacity the SOURCE authored. SSF2 makes a plane translucent
    // by setting alpha on a parent movieclip, so that value is read off the placement chain per
    // stage; it is not a fixed convention and must not be hardcoded.
    // a SOLVED blend overlay ships at FULL layer alpha: its per-pixel alpha already IS the
    // blend, so any layer alpha on top of it scales the tint down (measured: the overlay
    // reached the fighters at a fraction of the intended strength). Authored art keeps the
    // source's own plane opacity.
    let fg_alpha = if art.foreground_is_blend_overlay { 1.0 } else { art.foreground_alpha };
    let add_foreground_art = |b: &mut EntityBuilder| {
        if art.foreground.is_empty() { return; }
        b.add_image_frames_alpha("Foreground Art",
            &art.foreground.iter().map(|a| (a.guid.clone(), a.x, a.y, a.hold)).collect::<Vec<_>>(), fg_alpha);
    };
    // A solved blend overlay is the source's TOPMOST plane: it is the light the stage is lit by,
    // and everything the source draws is underneath it, the animated foreground elements included.
    // Those elements are hosted in the engine's own foreground containers, so the overlay has to go
    // ABOVE those containers or it tints the art it was solved against and nothing else. Authored
    // foreground art is not a light and keeps its place under them.
    if !art.foreground_is_blend_overlay { add_foreground_art(&mut b); }
    b.add_container("Foreground Structures", "FOREGROUND_STRUCTURES_CONTAINER");
    b.add_container("Foreground Shadows", "FOREGROUND_SHADOWS_CONTAINER");
    b.add_container("Foreground Effects", "FOREGROUND_EFFECTS_CONTAINER");
    b.add_container("Foreground Front", "FOREGROUND_FRONT_CONTAINER");
    if art.foreground_is_blend_overlay { add_foreground_art(&mut b); }

    // boundaries: blast zone + hard camera bounds. NO camera-anchor box: the engine adds an
    // anchor to the camera's target bounds, so any anchor forces the camera to zoom out to
    // keep it framed. A blast-zone-sized anchor pins the camera at max zoom-out and the art
    // never fills the view. Without one, the camera frames the players within the camera box
    // and sits at the StageStats `minZoomHeight` floor, where the scaled backdrop fills, the
    // same as a FM-native stage.
    // EVERY STAGE NEEDS A BLAST ZONE, whether its source states one or not. The engine works out
    // where items may fall from the death box, and with none there it reads a rectangle off
    // nothing and the match dies on the way in -- DAT61's source declares no deathBoundary, and
    // it took the engine with it before a single frame ran.
    //
    // Where the source is silent the camera box answers, because the two are related and the
    // corpus says how: across the stages that declare BOTH, the blast zone is the camera box
    // outset by the same amount every time -- about 1.39x its width and 1.49x its height, around
    // the same middle. So a stage that states only where it looks still gets a blast zone the
    // size its neighbours would have given it.
    let derived_death_box = model.death_box.or_else(|| {
        model.camera_box.as_ref().map(|c| {
            let (cx, cy) = (c.left() + c.w / 2.0, c.top() + c.h / 2.0);
            let (w, h) = (c.w * 1.39, c.h * 1.49);
            crate::stage_parser::Rect { x: cx - w / 2.0, y: cy - h / 2.0, w, h }
        })
    });
    if let Some(r) = &derived_death_box { b.add_collision_box("Death Box", "DEATH_BOX", r); }
    if let Some(r) = &model.camera_box {
        b.add_collision_box("Camera Box", "CAMERA_BOX", r);
    }
    // Overhead and centred on the play area. A light off to one side throws every shadow sideways;
    // directly above is the only placement that reads as ordinary daylight, and it is the one the
    // stage's own geometry can give without inventing a position.
    // EVERY stage gets one. The light is what the engine casts from; a mask only says where the
    // shadow is allowed to land. Gating the light on the mask meant a stage that draws its own
    // shadows in SSF2 (so carries no mask plane) came across with nothing to cast from, and its
    // fighters stood on the ground with no shadow at all.
    {
        let (lx, ly) = light_place(model);
        // (a light its own code keeps over the fighters: no engine light, which cannot move; the
        // stage draws the fighters' shadows from where its code puts it, `followLight`)
        let follows = model.stage_script.as_deref().is_some_and(light_follows);
        // how far the light still lights: everything the camera can frame at full strength, out
        // to the blast zone before it is gone. Both measured from the light itself, since that is
        // what the engine compares against.
        let corner = |r: Option<&Rect>| r.map(|r| {
            let dx = (r.left() - lx).abs().max((r.right() - lx).abs());
            let dy = (r.top() - ly).abs().max((r.bottom() - ly).abs());
            (dx * dx + dy * dy).sqrt()
        });
        let fade_start = corner(model.camera_box.as_ref()).unwrap_or(1200.0);
        let fade_end = corner(model.death_box.as_ref()).unwrap_or(fade_start * 1.5).max(fade_start);
        if !follows { b.add_shadow_light(lx, ly, &[], (fade_start, fade_end)); }
        // the source's own mask plane where it has one; otherwise the one the stage's collision
        // implies. Either way the engine gets an animation under the name it reads the mask from.
        match &art.shadow_mask {
            Some(m) => b.add_shadow_mask_anim(SHADOW_MASK_ANIM, &m.guid, m.x, m.y),
            None => b.add_shadow_mask_rects(SHADOW_MASK_ANIM, &mask_plate_guid(&model.id),
                                            &shadow_mask_rects(model)),
        }
    }

    // collision: main floor (solid, with ledges at the SSF2 ledge positions) + soft
    // platforms (drop-through). Ledges grab at the floor's left/right endpoints.
    let main_floor = model.main_floor().cloned();
    let mut plat_n = 0usize;
    for p in &model.platforms {
        // visible platforms are spawned as moving Structures (their collision comes from the
        // structure's own line segment), so they get NO static collision here.
        // a platform spawned as a Structure carries its own collision, so it gets none here --
        // but ONLY the ones that actually become one. A platform the stage's code merely NAMES
        // was being skipped here as well, and nothing emitted a structure for it, so a stage
        // whose code names all of its platforms came out with no ground at all: three of them
        // in the corpus, refused by the engine for having no floor. Naming is not driving, and
        // a platform nothing else carries keeps its own collision where the source put it.
        if p.visible { continue; }
        if platform_is_driven(p, model) && model.platforms.iter().any(|q| q.visible) { continue; }
        let __layers_before = b.layers.len();
        let r = &p.rect;
        let (lx, rx) = model.ledges
            .filter(|_| main_floor.as_ref().map(|m| m.rect == *r).unwrap_or(false))
            .unwrap_or((r.left(), r.right()));
        // a curved/sloped terrain traces its top surface as a polyline; a flat one is the
        // level top line. ledges anchor at the surface endpoints.
        let surface = |default: Vec<(f64, f64)>| -> Vec<(f64, f64)> {
            p.profile.clone().filter(|pf| pf.len() >= 2).unwrap_or(default)
        };
        let is_main = main_floor.as_ref().map(|m| m.rect == *r).unwrap_or(false);
        if !p.drop_through && !p.outline.is_empty() {
            // the solid's own outline, run by run. The ledges go on the floor runs whose ends are
            // the source's ledge points; a main floor without declared ledges grabs at the ends
            // of its widest floor run.
            plat_n += 1;
            let name = if is_main { "Floor".to_string() } else { format!("Solid {plat_n}") };
            let floors: Vec<usize> = p.outline.iter().enumerate().filter(|(_, (t, _))| *t == "FLOOR").map(|(i, _)| i).collect();
            let widest = floors.iter().copied().max_by(|&a, &b| {
                let w = |i: usize| { let pts = &p.outline[i].1; (pts[pts.len() - 1].0 - pts[0].0).abs() };
                w(a).total_cmp(&w(b))
            });
            let declared = model.ledges.filter(|_| is_main);
            for (i, (ty, run)) in p.outline.iter().enumerate() {
                let (first, last) = (run[0], run[run.len() - 1]);
                let (left_ledge, right_ledge) = if *ty != "FLOOR" || !is_main { (false, false) } else {
                    match declared {
                        Some((lx, rx)) => ((first.0 - lx).abs() < 6.0, (last.0 - rx).abs() < 6.0),
                        None => (widest == Some(i), widest == Some(i)),
                    }
                };
                b.add_line_segment(&format!("{name} {} {i}", ty.to_lowercase().replace('_', " ")), run, json!({
                    "structureType": ty, "leftLedge": left_ledge, "rightLedge": right_ledge, "dropThrough": false
                }));
            }
        } else if !p.drop_through && is_main {
            b.add_solid_body("Floor", &surface(vec![(lx, r.top()), (rx, r.top())]), r.bottom(), (true, true));
        } else if p.drop_through || p.is_platform {
            // a platform is a surface along its plank, dropped through or not as it declares:
            // never a block (a no-drop plank that was boxed closed lunar core's pit)
            plat_n += 1;
            b.add_line_segment(&format!("Platform {plat_n} Floor"), &surface(vec![(r.left(), r.top()), (r.right(), r.top())]), json!({
                "structureType": "FLOOR", "leftLedge": p.ledges.0, "rightLedge": p.ledges.1, "dropThrough": p.drop_through
            }));
        } else {
            plat_n += 1;
            b.add_solid_body(&format!("Solid {plat_n}"), &surface(vec![(r.left(), r.top()), (r.right(), r.top())]), r.bottom(), p.ledges);
        }
        // a surface a piece carries leaves with what it was made of
        if let Some(n) = p.name.as_ref().filter(|n| carried_names.contains(*n)) {
            let segs = b.take_layers(__layers_before);
            // still a surface the stage's code reaches by name: the boot hands it the structures
            let names: Vec<String> = segs.iter().map(|(l, ..)| l.clone()).collect();
            match plat_segs.iter_mut().find(|(k, _)| k == n) {
                Some((_, v)) => v.extend(names),
                None => plat_segs.push((n.clone(), names)),
            }
            carried.entry(n.clone()).or_default().extend(segs);
            continue;
        }
        if let Some(n) = &p.name {
            let made: Vec<String> = b.layers[__layers_before..].iter()
                .filter_map(|l| l["name"].as_str().map(str::to_string)).collect();
            if !made.is_empty() {
                match plat_segs.iter_mut().find(|(k, _)| k == n) {
                    Some((_, v)) => v.extend(made),
                    None => plat_segs.push((n.clone(), made)),
                }
            }
        }
    }
    // A STAGE WITH NO GROUND AT ALL. Every platform above can be skipped -- one drawn as art
    // carries its own collision, one the stage drives is a structure of its own -- and a stage
    // whose ground is a HAZARD skips all of them, so the stage itself declares nothing. The
    // engine does not survive that: it works out where items may fall from what the stage
    // declares, and with nothing there it reads a rectangle off nothing and takes the match with
    // it. DAT61's ground is one of its objects, and it crashed on the way in.
    //
    // The floor is known either way -- the model works out where it is to place the entrances --
    // so where nothing else declared one, it is declared here.
    if plat_n == 0 {
        if let Some(m) = main_floor.as_ref() {
            let r = &m.rect;
            b.add_solid_body("Solid 1 (the stage's own ground, declared by nothing else)",
                             &[(r.left(), r.top()), (r.right(), r.top())], r.bottom(),
                             (true, true));
        }
    }

    // spawns: entrances (match start) + respawns. Fill 4 of each (the engine expects
    // a full set); if SSF2 declared fewer, fall back to the main floor center.
    let floor_cx = main_floor.as_ref().map(|m| m.rect.left() + m.rect.w / 2.0).unwrap_or(0.0);
    let floor_top = main_floor.as_ref().map(|m| m.rect.top()).unwrap_or(0.0);
    // a source that places one start puts every player there
    for i in 0..4usize {
        let (x, y, rot) = model.entrances.iter().find(|s| s.index == i)
            .or_else(|| model.entrances.first())
            .map(|s| (s.x, s.y, if s.face_left { 270 } else { 90 }))
            .unwrap_or((floor_cx + (i as f64 - 1.5) * 60.0, floor_top - 40.0, 90));
        b.add_point(&format!("Entrance {i}"), "ENTRANCE_POINT", i, x, y, rot);
    }
    // a respawn the source did not place is the matching entrance (a fighter comes back where
    // it came in), and only a stage with neither falls back to the floor's centre; the engine
    // starts a fighter at its respawn point, so this is where the match begins
    for i in 0..4usize {
        let (x, y, rot) = model.respawns.iter().find(|s| s.index == i)
            .or_else(|| model.entrances.iter().find(|s| s.index == i))
            .or_else(|| model.entrances.first())
            .map(|s| (s.x, s.y, if s.face_left { 270 } else { 90 }))
            .unwrap_or((floor_cx + (i as f64 - 1.5) * 60.0, floor_top - 200.0, 90));
        b.add_point(&format!("Respawn {i}"), "RESPAWN_POINT", i, x, y, rot);
    }

    // each SSF2 camera-background layer -> its own `parallax{i}` animation (each scrolls at its
    // own rate, set in StageStats).
    for (i, p) in art.parallax.iter().enumerate() {
        // The engine anchors a camera background at the CENTRE of the view, so the art is placed
        // centred on the background's own origin rather than at its world position on the stage.
        let (cx, cy) = (-(p.w as f64) / 2.0, -(p.h as f64) / 2.0);
        match p.tracks.as_slice() {
            [] => {}
            // WITH ITS OWN PLACE IN THE BACKDROP. Every piece is measured from the backdrop's
            // corner, and dropping that offset stacks them all on the corner: hueco mundo's near
            // dunes covered its sky and its moon, which are the pieces behind them.
            [one] if one.len() == 1 =>
                b.add_image_parallax(i, &one[0].guid, cx + one[0].x, cy + one[0].y),
            tracks => b.add_image_tracks_parallax(i, cx, cy, tracks),
        }
    }

    for (anim, r) in &art.pictures {
        let lid = b.make_image(anim, &r.guid, 0.0, 0.0, 1.0);
        b.parallax_anims.push((anim.to_string(), vec![lid]));
    }
    let mut animations = vec![json!({
        "$id": b.uid("anim:stage"), "name": STAGE_ANIM, "pluginMetadata": {}, "layers": b.anim_layers
    })];
    for (name, lids) in b.parallax_anims.clone() {
        let aid = b.uid(&format!("anim:{name}"));
        animations.push(json!({ "$id": aid, "name": name, "pluginMetadata": {}, "layers": lids }));
    }
    animations.extend(b.extra_anims.clone());

    // SSF2 30fps -> Fraymakers 60fps: double every keyframe length, exactly like the character
    // port (entity_gen::double_keyframe_lengths). every layer (bg clips, foreground, platforms,
    // containers, points, line segments) was built in SSF2 frame units, so this doubling keeps
    // them all in lockstep at the right real-time speed.
    crate::entity_gen::double_keyframe_lengths(&mut b.keyframes);

    (json!({
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
    }), plat_segs, carried)
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

use crate::stage_parser::{ClipAnim, Hazard, Platform};

/// Emit the declared platforms as MOVING STRUCTURES (the FM idiom for a moving/sinking platform,
/// per the official stage-template): one shared grey `platformSprite` PNG (referenced by the
/// `platformSprite` animation in the stage entity), a shared sink/rise Script, and one structure
/// CONTENT (with its own Stats giving startX/startY) per platform. Returns the shared sprite
/// `(guid, w, h)` for the entity animation, the manifest structure-content entries, and the
/// content ids the stage Script spawns with `match.createStructure`.
#[allow(clippy::type_complexity)]
/// Does the stage's own code reach this platform by name? A stage that keeps a table of layouts
/// and makes each platform solid or passable as its tower rises (butter building) needs the
/// platform to BE something it can reach; one baked into the stage's own collision is not.
fn platform_is_driven(p: &Platform, model: &StageModel) -> bool {
    p.name.as_deref().is_some_and(|name| crate::stage_text::StageText::of(model).names(name))
}

fn emit_platform_structures(model: &StageModel, lib: &Path, sprites: &Path)
    -> Result<(Vec<(String, f64, f64, u8)>, Vec<Value>, Vec<(String, Option<String>)>)>
{
    let vis: Vec<&Platform> = model.platforms.iter().filter(|p| p.visible).collect();
    if vis.is_empty() { return Ok((Vec::new(), Vec::new(), Vec::new())); }
    let id = &model.id;
    let scripts = lib.join("scripts").join("platform");
    std::fs::create_dir_all(&scripts).context("mkdir scripts/platform")?;
    let script_id = format!("{id}platformScript");
    write_script(&scripts.join(format!("{script_id}.hx")), &platform_script_hx())?;
    write_meta(&scripts.join(format!("{script_id}.hx.meta")), id, &script_id, "hscript", Some("LINE_SEGMENT_STRUCTURE"), None)?;
    // per-platform: a sprite sized to THIS platform (the SSF2 standing platforms are different
    // widths), its own `platformSprite{i}` animation, Stats (startX/startY), a structure content
    // entry, and the spawn id. The sprite is TRANSPARENT (collision-only): the visible standable
    // surface is the engine-added bg layer the stage renders (e.g. bowserscastle's brick bridge),
    // so the structure just carries the floor line segment at the platform's position.
    let mut sprite_dims = Vec::new();
    let mut contents = Vec::new();
    let mut spawn_ids = Vec::new();
    for (i, p) in vis.iter().enumerate() {
        let (pw, ph) = (p.rect.w.round().max(8.0) as u32, p.rect.h.round().max(10.0) as u32);
        let img: image::RgbaImage = image::RgbaImage::new(pw, ph); // transparent collision footprint
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .context("encode platform png")?;
        let guid = det_uuid(&format!("stage::{id}::platformSprite{i}"));
        std::fs::write(sprites.join(format!("{id}_platformSprite{i}.png")), &png)?;
        write_json(&sprites.join(format!("{id}_platformSprite{i}.png.meta")), &json!({
            "export": false, "guid": guid, "id": "", "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
        }))?;
        sprite_dims.push((guid, pw as f64, ph as f64, 0));

        let cid = format!("{id}platform{i}");
        let stats_id = format!("{cid}Stats");
        let (sx, sy) = (p.rect.x + p.rect.w / 2.0, p.rect.y);
        write_script(&scripts.join(format!("{stats_id}.hx")), &platform_stats_hx(id, sx, sy, i))?;
        write_meta(&scripts.join(format!("{stats_id}.hx.meta")), id, &stats_id, "hscript", None, None)?;
        contents.push(json!({ "id": cid, "type": "structure", "objectStatsId": stats_id, "scriptId": script_id }));
        spawn_ids.push((cid, p.name.clone()));
    }
    // a hazard that declares createSelfPlatform (the Thwomp) gets a DECK structure riding it: the
    // SSF2 self-platform is a SOLID box (floor + walls + ceiling — fighters can't pass through the
    // body), so it emits as a RECT_STRUCTURE sized to the whole declared box, riding the
    // falling/landed/rising body (riding it up is real SSF2 gameplay). the deck finds the thwomp
    // custom game object by matching its x against the spawn columns, engages only once the thwomp
    // has dropped below its spawn hover (SSF2 keeps fallthrough on during the entrance), and parks
    // off-world between cycles.
    // (the decompiler-reconstruction path creates these at the class's own createSelfPlatform
    // call site instead, so the pre-spawned follower pair is template-path only.)
    let decomp_selfplatform = std::env::var("PEPTIDE_DECOMP_HAZARD").is_ok();
    if let Some(hz) = model.hazards.iter()
        .find(|h| h.behavior.self_platform.is_some() && recycles_itself(h, model) && !decomp_selfplatform) {
        let (bx, by, bw, bh) = hz.behavior.self_platform.unwrap();
        let s = model.scale;
        let deck_w = (bw * s).round().max(8.0);
        let deck_h = (bh * s).round().max(8.0);
        let off_x = (bx + bw / 2.0) * s; // box centre relative to the body origin
        let off_y = by * s;              // solid top edge (the rect extends deck_h below it)
        let cols_x: Vec<f64> = if !model.sink_columns.is_empty() { model.sink_columns.clone() }
            else { vis.iter().map(|p| p.rect.x + p.rect.w / 2.0).collect() };
        let spawn_y = model.death_box.as_ref().map(|b| b.y + 60.0)
            .unwrap_or_else(|| vis.iter().map(|p| p.rect.y).fold(f64::MAX, f64::min) - 520.0);
        let di = sprite_dims.len();
        let img: image::RgbaImage = image::RgbaImage::new(deck_w as u32, deck_h as u32); // transparent, collision-only
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .context("encode deck png")?;
        let guid = det_uuid(&format!("stage::{id}::platformSprite{di}"));
        std::fs::write(sprites.join(format!("{id}_platformSprite{di}.png")), &png)?;
        write_json(&sprites.join(format!("{id}_platformSprite{di}.png.meta")), &json!({
            "export": false, "guid": guid, "id": "", "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
        }))?;
        sprite_dims.push((guid.clone(), deck_w, deck_h, 1));
        let cid = format!("{id}thwompdeck");
        let stats_id = format!("{cid}Stats");
        write_script(&scripts.join(format!("{stats_id}.hx")), &platform_stats_hx(id, -2000.0, -3000.0, di))?;
        write_meta(&scripts.join(format!("{stats_id}.hx.meta")), id, &stats_id, "hscript", None, None)?;
        let deck_script_id = format!("{id}thwompdeckScript");
        write_script(&scripts.join(format!("{deck_script_id}.hx")), &thwomp_deck_script_hx(&cols_x, off_x, off_y, spawn_y + 80.0))?;
        write_meta(&scripts.join(format!("{deck_script_id}.hx.meta")), id, &deck_script_id, "hscript", Some("LINE_SEGMENT_STRUCTURE"), None)?;
        contents.push(json!({ "id": cid, "type": "structure", "objectStatsId": stats_id, "scriptId": deck_script_id }));
        spawn_ids.push((cid, None));
        // the body's UNDERSIDE: a FLOOR-typed structure never blocks an upward mover, so the
        // ceiling face is its own CEILING-typed follower structure (same script, same offsets;
        // its animation puts the segment at the box's bottom edge).
        let ci = sprite_dims.len();
        sprite_dims.push((guid.clone(), deck_w, deck_h, 2));
        let ceil_cid = format!("{id}thwompceiling");
        let ceil_stats_id = format!("{ceil_cid}Stats");
        write_script(&scripts.join(format!("{ceil_stats_id}.hx")), &platform_stats_hx(id, -2000.0, -3000.0, ci))?;
        write_meta(&scripts.join(format!("{ceil_stats_id}.hx.meta")), id, &ceil_stats_id, "hscript", None, None)?;
        contents.push(json!({ "id": ceil_cid, "type": "structure", "objectStatsId": ceil_stats_id, "scriptId": deck_script_id }));
        spawn_ids.push((ceil_cid, None));
    }
    Ok((sprite_dims, contents, spawn_ids))
}

/// The Thwomp's rideable deck — 1:1 from SSF2 `createSelfPlatform`: a line-segment structure that
/// follows the thwomp custom game object (matched by column x), standable once the thwomp drops
/// below its spawn hover, parked off-world between cycles.
fn thwomp_deck_script_hx(cols_x: &[f64], off_x: f64, off_y: f64, engage_y: f64) -> String {
    let cols_lit = cols_x.iter().map(|x| format!("{x:.1}")).collect::<Vec<_>>().join(", ");
    format!(
        "// Thwomp deck — the SSF2 createSelfPlatform box riding the thwomp (standable in\n\
         // fall/land/rise; parked off-world while the thwomp waits at its spawn point).\n\
         var COLS_X = [{cols_lit}];\nvar OFF_X = {off_x:.1};\nvar OFF_Y = {off_y:.1};\nvar ENGAGE_Y = {engage_y:.1};\n\
         var m_excluded = self.makeBool(false);\n\n\
         function findThwomp() {{\n\
         \tvar objs = match.getCustomGameObjects();\n\
         \tfor (i in 0...objs.length) {{\n\
         \t\tvar o = objs[i];\n\
         \t\tfor (j in 0...COLS_X.length) {{ if (Math.abs(o.getX() - COLS_X[j]) < 2) {{ return o; }} }}\n\
         \t}}\n\treturn null;\n}}\n\n\
         function update() {{\n\
         \tvar t = findThwomp();\n\
         \t// SSF2 never grounds an enemy on its OWN self-platform: with an ECB the falling thwomp\n\
         \t// would land on this follower mid-air, so exclude it from the structure's collision.\n\
         \tif (t != null && !m_excluded.get()) {{ self.addToBlacklist(t); m_excluded.set(true); }}\n\
         \tif (t != null && t.getY() > ENGAGE_Y) {{ self.setX(t.getX() + OFF_X); self.setY(t.getY() + OFF_Y); }}\n\
         \t// parking carries any remaining rider off-world -> KO, the SSF2 outcome for riding\n\
         \t// the thwomp to the top (it crosses the top blast bound there).\n\
         \telse {{ self.setX(-2000); self.setY(-3000); }}\n}}\n\
         function initialize() {{}}\nfunction onTeardown() {{}}\nfunction onKill() {{}}\nfunction onStale() {{}}\n\
         function afterPushState() {{}}\nfunction afterPopState() {{}}\nfunction afterFlushStates() {{}}\n")
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
/// The script a declared platform structure carries.
///
/// It stands where the stage put it. A stage whose platforms MOVE says so in a class the tree
/// hands the platform (see `classAPI` in stage_parser) and that class is ported and runs; a
/// converter-side motion template fits only the one thing it was written for, and bowser's castle
/// is the worked example of why: its package ships a sinking-platform class, its stage never hands
/// that class to anything, and its thwomp's six columns miss its one platform entirely. Ported
/// from the template, the bridge sank in the port and never sinks in the source.
fn platform_script_hx() -> String {
    "// A platform stands where the stage put it. Motion, when a stage has it, is in the class the\n\
     // tree hands the platform, and that class is ported and runs on its own.\n\
     function initialize() {}\nfunction update() {}\nfunction onTeardown() {}\nfunction onKill() {}\n\
     function onStale() {}\nfunction afterPushState() {}\nfunction afterPopState() {}\n\
     function afterFlushStates() {}\n".to_string()
}


/// File/content-safe element id from the SSF2 source name (instance or linkage), unique across
/// the stage: `bowsers_bubbles_bg` stays readable instead of `bg37`; repeated placements of one
/// symbol (16 torch embers) get an ordinal suffix. Empty names fall back to `bg{idx}`.
fn element_ids(layers: &[crate::stage_parser::BgLayer]) -> Vec<String> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let base: Vec<String> = layers.iter().enumerate().map(|(i, l)| {
        let b: String = l.name.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
            .collect::<String>().trim_matches('_').to_string();
        let b = if b.is_empty() { format!("bg{i}") } else { b };
        *counts.entry(b.clone()).or_default() += 1;
        b
    }).collect();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    base.into_iter().map(|b| {
        if counts[&b] == 1 { return b; }
        let n = seen.entry(b.clone()).or_default();
        *n += 1;
        format!("{b}{n}")
    }).collect()
}

/// Content id for hazard `idx` of stage `id` — the SSF2 class name when the hazard came from a
/// `spawnEnemy(<Class>)` reference (`bowserscastlessf2BowsersCastleLava`), else `hazard{idx}`.
fn hazard_id(stage_id: &str, idx: usize, hz: &Hazard) -> String {
    let clean = |n: &str| n.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>();
    match hz.class_name.as_deref().map(clean) {
        // A PLACED INSTANCE IS ITS OWN OBJECT. One class placed seven times is seven objects
        // with seven clips and seven bodies (palace of twilight's three platforms and four
        // pillars share a class), so each placement gets its own content under its own name.
        // An object the code spawns by class keeps the class's name, which is what the spawn
        // asks the library for.
        Some(c) if !c.is_empty() => match hz.instance_name.as_deref().map(clean) {
            Some(n) if !n.is_empty() && !hz.shared_name => format!("{stage_id}{c}{n}"),
            Some(n) if !n.is_empty() => format!("{stage_id}{c}{n}{idx}"),
            // the game's own target is only ever placed (nothing spawns it by class), each its own
            _ if c == "PeptideTarget" => format!("{stage_id}{c}{idx}"),
            _ => format!("{stage_id}{c}"),
        },
        _ => format!("{stage_id}hazard{idx}"),
    }
}


/// The pictures an object draws, written once each however often it draws them.
///
/// A source object states its art per animation, and the same cel turns up in many of them: a
/// block that looks the same appearing, waiting and vanishing, a fish with one body across every
/// phase it swims through. Emitted per animation those become separate files with separate ids,
/// and the package carries one picture many times over. Keyed on the encoded bytes, the repeats
/// become extra references to a single image asset.
///
/// This is not only size. The publish step imports assets one at a time, so a package with
/// thousands of them takes longer to build than the export is willing to wait, and a stage that
/// converts perfectly never reaches the engine at all.
#[derive(Default)]
pub(crate) struct PngPool(std::collections::BTreeMap<Vec<u8>, String>);

impl PngPool {
    /// The asset id for these bytes: the one already written if they have been, else a new file
    /// named `name` whose id comes from `seed`.
    fn put(&mut self, dir: &Path, name: &str, seed: &str, png: &[u8]) -> Result<String> {
        if let Some(g) = self.0.get(png) { return Ok(g.clone()); }
        let guid = det_uuid(seed);
        std::fs::write(dir.join(format!("{name}.png")), png)?;
        write_json(&dir.join(format!("{name}.png.meta")), &json!({
            "export": false, "guid": guid, "id": "", "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
        }))?;
        self.0.insert(png.to_vec(), guid.clone());
        Ok(guid)
    }
}

/// What an object's script holds until the runtime writes its own over it (`emit::stage`).
const RUNTIME_WRITES: &str = "// the runtime writes this object's script (emit::stage)\n";

fn emit_hazards(model: &StageModel, lib: &Path) -> Result<Vec<Value>> {
    let mut entries = Vec::new();
    if model.hazards.is_empty() { return Ok(entries); }
    let mut pool = PngPool::default();
    let sprites = lib.join("sprites").join("Hazard");
    let scripts = lib.join("scripts").join("hazard");
    std::fs::create_dir_all(&sprites).context("mkdir sprites/Hazard")?;
    std::fs::create_dir_all(&scripts).context("mkdir scripts/hazard")?;
    // a `thwomp`-motion hazard cycles target columns: the stage's AS3-declared sink columns
    // (1:1 from its update disasm, already FM x) when present, each landing on the platform
    // under it; else fall back to the visible platforms' centers.
    let land_y_at = |x: f64| -> f64 {
        model.platforms.iter()
            .filter(|p| x >= p.rect.left() && x <= p.rect.right())
            .map(|p| p.rect.y).fold(f64::INFINITY, f64::min)
    };
    let cols: Vec<(f64, f64)> = if !model.sink_columns.is_empty() {
        model.sink_columns.iter().map(|&x| (x, land_y_at(x))).filter(|(_, y)| y.is_finite()).collect()
    } else {
        model.platforms.iter().filter(|p| p.visible)
            .map(|p| (p.rect.x + p.rect.w / 2.0, p.rect.y)).collect()
    };

    // a single-art object's entity carries one animation, so every clip its class names is that one
    let play_anim_re = crate::re!(r#"self\.playAnimation\("[^"]*"\)"#);
    for (i, hz) in model.hazards.iter().enumerate() {
        let hid = hazard_id(&model.id, i, hz);
        // THE PIECE IT DRAWS IN FRONT, when the class named one that is not its own art. It is
        // drawn, not collided with, so it ships as a vfx and the stage keeps it on the object.
        if !hz.fg_anims.is_empty() {
            let fid = format!("{hid}Fg");
            let mut cels: Vec<ArtRef> = Vec::new();
            for (fi, fr) in hz.fg_anims[0].frames.iter().enumerate() {
                let guid = pool.put(&sprites, &format!("{fid}_{fi}"),
                    &format!("hazard::{fid}::{fi}"), &fr.png)?;
                cels.push(ArtRef { guid, x: fr.x * model.scale, y: fr.y * model.scale,
                    w: fr.w, h: fr.h, hold: 1, alpha: 1.0, pivot: (0.0, 0.0), blank: false,
                    rotation: 0.0, scale_x: 1.0, scale_y: 1.0, tint: [1.0, 1.0, 1.0, 0.0, 0.0, 0.0] });
            }
            let phases = vec![("active".to_string(), cels)];
            write_json(&lib.join("entities").join(format!("{fid}.entity")),
                &bg_element_entity_phased(&fid, &phases, model.scale, (0.0, 0.0)))?;
            write_meta(&lib.join("entities").join(format!("{fid}.entity.meta")), &fid, &fid, "", Some("VFX"), None)?;
            entries.push(json!({ "namespace": "custom", "resourceId": model.id, "contentId": fid }));
        }
        // a hazard whose SSF2 clip has labelled sub-animations (e.g. a Thwomp's entrance/fall/idle)
        // becomes a MULTI-animation custom game object: one FM animation per label, the local state
        // machine switching between them, the HIT_BOX riding only the active (damaging) ones. This
        // is universal — any multi-animation hazard goes down this path; the single-art path below
        // handles a static/looping hazard with no labelled phases (lava, a spike, …).
        if !hz.anims.is_empty() {
            emit_multi_anim_hazard(&hid, hz, model, &sprites, &scripts, lib, &mut entries, &mut pool)?;
            continue;
        }
        // sprite: the real rasterized SSF2 hazard art if we recovered it, else a translucent red
        // hitbox-volume placeholder (w x h). real art renders at the stage scale, centered on the
        // hazard; the placeholder fills the hitbox 1:1.
        let (png, img_x, img_y, img_scale) = if let Some(art) = &hz.art {
            let (aw, ah) = (art.w as f64 * model.scale, art.h as f64 * model.scale);
            (art.png.clone(), -aw / 2.0, -ah / 2.0, model.scale)
        } else {
            // no own art: a STATIC region hazard's visual IS the stage background (the lava lake is
            // painted bg art; the hazard is just the damage volume) — emit a fully transparent
            // sprite. A MOVING hazard with no art keeps the translucent red debug volume so the
            // missing art is visible rather than an invisible killer.
            let (w, h) = (hz.w.max(8.0) as u32, hz.h.max(8.0) as u32);
            let img = image::RgbaImage::new(w, h);
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

        // an object with a class of its own runs that class: its script is the runtime's
        // (`emit::stage`), written over this one; one without is a template of what it declares
        let script = if hz.class_name.as_deref().is_some_and(|c| !c.is_empty()) { RUNTIME_WRITES.to_string() } else {
            let script = if recycles_itself(hz, model) && !cols.is_empty() {
                thwomp_script_hx(&cols)
            } else if let Some((speed, label)) = &hz.bounce {
                // ...standing on the body the clip declares, which is what a fighter lands on (a
                // clip with no labelled art of its own carries the body all the same)
                let (decls, helpers) = hz.behavior.self_platform
                    .map(|b| self_platform_code(b, model.scale, &hz.platform_shapes, &hz.body_faces)).unwrap_or_default();
                format!("{decls}{helpers}{}", bounce_script_hx(hz, *speed, label))
            } else {
                hazard_script_hx(hz)
            };
            // this entity carries ONE animation, so every clip the class names is that animation
            let script = play_anim_re.replace_all(&script, "self.playAnimation(\"gameObjectIdle\")").into_owned();
            let script = with_sync_ledges(script, hz.ledges);
            let script = with_constructor(script);
            let script = with_declared_body(script);
            let script = if hz.no_knockback { with_launch_armor(script) } else { script };
            let script = if hz.survives_bounds { with_bounds_survival(script) } else { script };
            let script = with_no_hitstop(script);
            let script = with_body_release(script);
            if hz.damage > 0.0 { with_hitbox_rearm(script, hz.rehit) } else { script }
        };
        let files = [
            ("Script", script),
            ("GameObjectStats", hazard_gameobject_stats_hx(&hid)),
            ("HitboxStats", hazard_hitbox_stats_hx(hz)),
            ("AnimationStats", hazard_animation_stats_hx()),
        ];
        for (kind, body) in files {
            let fname = format!("{hid}{kind}");
            write_script(&scripts.join(format!("{fname}.hx")), &body)?;
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
    entries.extend(emit_hazard_projectiles(model, lib)?);
    Ok(entries)
}

/// THE PROJECTILES A STAGE'S OBJECTS FIRE. A class declares them in its own stats, each by the
/// linkage its art is registered under, and fires them by name; the port ships each as the
/// same projectile a fighter's would be, under the stage's id, so the ask
/// (`createProjectile(getContent("<name>Projectile"))`) finds it. Birdo spat nothing until now.
fn emit_hazard_projectiles(model: &StageModel, lib: &Path) -> Result<Vec<Value>> {
    let mut data: std::collections::BTreeMap<String, crate::abc_parser::ProjectileData> = std::collections::BTreeMap::new();
    for hz in &model.hazards {
        for (name, pd) in &hz.projectiles { data.entry(name.clone()).or_insert_with(|| pd.clone()); }
    }
    if data.is_empty() { return Ok(Vec::new()); }
    let linkages: Vec<String> = data.values().filter_map(|pd| pd.names.get("linkage_id").cloned()).collect();
    let raw = std::fs::read(&model.source).context("read the stage package")?;
    let bytes = crate::ssf::decompress(&raw).context("unwrap the stage package")?;
    let buf = swf::decompress_swf(&bytes[..]).context("decompress the stage swf")?;
    let parsed = swf::parse_swf(&buf).context("parse the stage swf")?;
    let found = crate::image_extractor::discover_projectiles_by_linkage(&parsed, &linkages);
    for l in &linkages {
        if !found.iter().any(|p| p.name.eq_ignore_ascii_case(l)) {
            log::warn!("{}: a class declares a projectile by linkage '{l}' and the package carries no clip of that name; it fires nothing", model.id);
        }
    }
    if found.is_empty() { return Ok(Vec::new()); }
    // the projectile's own pictures, and nothing else in the package
    let only = crate::image_extractor::ids_under_sprites(&parsed, &found.iter().map(|p| p.sprite_id).collect::<Vec<_>>());
    let stage_dir = lib.parent().unwrap_or(lib);
    let img_result = crate::image_extractor::extract_images_from_swf_only(
        &parsed, stage_dir, &model.id, &std::collections::BTreeMap::new(), &std::collections::BTreeMap::new(), Some(&only), &[])?;
    // each picture's guid sidecar, the way the character path writes them: the entity names
    // the picture by that guid, and the pack refuses an entity whose picture has none
    for (png_rel, guid) in &crate::entity_gen::get_image_meta_guids(&img_result) {
        let meta = stage_dir.join(format!("{png_rel}.meta"));
        if let Some(parent) = meta.parent() { std::fs::create_dir_all(parent)?; }
        std::fs::write(&meta, crate::entity_gen::generate_meta(guid))?;
    }
    let empty: std::collections::BTreeSet<String> = Default::default();
    let ctx = crate::projectile_gen::ProjectileGenCtx {
        kept_by_summoner: &empty,
        owner_vars: &empty,
        owner_fns: &empty,
        owner_getters: &Default::default(),
        owner_anims: &Default::default(),
        owner_id: &model.id,
        entities_dir: &lib.join("entities"),
        scripts_root: &lib.join("scripts"),
        parsed_swf: &parsed,
        img_result: &img_result,
        projectile_data: &data,
        palette_collection_guid: None,
        palette_base_map_id: None,
        runtime_runs_frames: false,
    };
    crate::projectile_gen::generate_projectiles(&ctx, &found)?;
    let mut entries = Vec::new();
    for p in &found {
        // the declaration's own name for it is what the class fires; the clip's linkage is how
        // the art was found. the content id folds the fired name.
        let fired = data.iter().find(|(_, pd)| pd.names.get("linkage_id").is_some_and(|l| l.eq_ignore_ascii_case(&p.name)))
            .map(|(n, _)| n.clone()).unwrap_or_else(|| p.name.clone());
        let entity_id = p.name.replace('_', "");
        entries.push(json!({
            "id": crate::projectile_gen::projectile_content_id(&fired),
            "type": "projectile",
            "objectStatsId": format!("{entity_id}ProjectileStats"),
            "animationStatsId": format!("{entity_id}ProjectileAnimationStats"),
            "hitboxStatsId": format!("{entity_id}ProjectileHitboxStats"),
            "scriptId": format!("{entity_id}ProjectileScript"),
        }));
        log::info!("{}: projectile '{}' shipped as {} (clip '{}')", model.id, fired, crate::projectile_gen::projectile_content_id(&fired), p.name);
    }
    Ok(entries)
}

/// The CUSTOM_GAME_OBJECT entity: one `gameObjectIdle` animation with an IMAGE + a HIT_BOX
/// (index 0), sized to the hazard. Mirrors a projectile entity (the proven hitbox carrier).
/// Does the object's own declaration make it strike? A body that declares no damage and no
/// knockback is scenery a fighter stands on, and a hit box on it, even one that deals nothing,
/// still puts every fighter that lands on it into hitstun.
fn hazard_hits(hz: &Hazard) -> bool {
    hz.damage > 0.0 || hz.knockback > 0.0 || hz.kb_growth > 0.0
}

/// ...and whether it strikes by touch: not when its clip carries only a body box, the shape it is
/// hit on (konoha's lanterns declare damage and hurt nobody who touches them)
fn hazard_touch_hits(hz: &Hazard) -> bool {
    hazard_hits(hz) && !hz.body_box
}

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
            { "$id": g("anim"), "name": "gameObjectIdle", "layers": if hazard_touch_hits(hz) { json!([g("imglayerA"), g("boxlayer")]) } else { json!([g("imglayerA")]) }, "pluginMetadata": {} },
            { "$id": g("anim2"), "name": "gameObjectInactive", "layers": [g("imglayerB")], "pluginMetadata": {} }
        ]
    });
    // the body its clip declares (`hitBox`) is the shape a fighter hits it on (konoha's lanterns
    // swing when struck): a hurt box of its size, on the animation it idles in
    if hz.body_box {
        let hurt = json!({ "$id": g("hurtlayer"), "name": "hurtbox", "type": "COLLISION_BOX", "hidden": false, "locked": false, "defaultAlpha": 0.5, "defaultColor": "0x00ff00", "keyframes": [g("hurtkf")],
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "collisionBoxType": "HURT_BOX", "index": 0 } } });
        e["symbols"].as_array_mut().unwrap().push(json!({ "$id": g("hurtsym"), "type": "COLLISION_BOX", "x": -hw, "y": -hh, "pivotX": hw, "pivotY": hh, "scaleX": hz.w, "scaleY": hz.h, "rotation": 0.0, "alpha": 0.5, "color": "0x00ff00", "pluginMetadata": {} }));
        e["keyframes"].as_array_mut().unwrap().push(json!({ "$id": g("hurtkf"), "symbol": g("hurtsym"), "length": 1, "tweened": false, "tweenType": "LINEAR", "type": "COLLISION_BOX", "pluginMetadata": {} }));
        e["layers"].as_array_mut().unwrap().push(hurt);
        e["animations"][0]["layers"].as_array_mut().unwrap().push(json!(g("hurtlayer")));
    }
    // SSF2 30fps -> FM 60fps doubling, same as the stage entity + the character port.
    if let Some(kfs) = e.get_mut("keyframes").and_then(|k| k.as_array_mut()) {
        crate::entity_gen::double_keyframe_lengths(kfs);
    }
    e
}

/// One labelled FM animation of a multi-animation hazard CGO: the FM animation name (the SSF2
/// frame label), its per-frame sprite guids (in order), the shared image placement + canvas size
/// (scaled into FM world units), and whether it carries the HIT_BOX (a damaging phase).
struct HzAnim {
    name: String,
    frame_guids: Vec<String>,
    /// IMAGE placement (frame canvas top-left, already × stage scale).
    x: f64,
    y: f64,
    /// Canvas size (× stage scale) — the HIT_BOX volume for an active phase.
    w: f64,
    h: f64,
    /// Stage scale applied to the IMAGE symbol (native-res PNG → scaled geometry).
    scale: f64,
    /// `true` = a damaging phase (carries the HIT_BOX); `false` = a safe phase (sprite only).
    active: bool,
    /// `true` = the SWF sub-clip stop()s (Flash timeline class with a stopping frame script), so
    /// the FM animation plays once and HOLDS (endType NONE); `false` = the clip genuinely loops.
    hold: bool,
    /// `true` = the source SAYS it loops (its own frame script replays the label it sits on), which
    /// outranks anything the driving class would otherwise imply.
    loops: bool,
}

/// A valid FM animation/identifier name from an SSF2 frame label (alphanumerics only, lower-cased
/// first char-run kept; falls back to `anim` if it sanitizes to empty).
fn sanitize_anim(label: &str) -> String {
    let s: String = label.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if s.is_empty() { return "anim".to_string(); }
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) { format!("a{s}") } else { s }
}

/// The animation a section is played as: its label, unless the script language keeps that word
/// for itself (waiting room's first section is `default`), which no stats key can be
fn anim_key(phase: &str) -> String {
    if crate::compile::ident(phase) != phase { format!("{phase}_") } else { phase.to_string() }
}

/// Emit a multi-animation hazard CGO: write each labelled animation's frames as sprite PNGs, build
/// a CUSTOM_GAME_OBJECT entity with one FM animation per label (HIT_BOX on the damaging ones), and
/// a local-state-machine Script that switches animations by phase. Universal across stages; the
/// Thwomp is the first user (entrance/fall/idle).
#[allow(clippy::too_many_arguments)]
fn emit_multi_anim_hazard(
    hid: &str, hz: &Hazard, model: &StageModel, sprites: &Path, scripts: &Path, lib: &Path,
    entries: &mut Vec<Value>, pool: &mut PngPool,
) -> Result<()> {
    let scale = model.scale;
    let mut hzanims: Vec<HzAnim> = Vec::new();
    for anim in &hz.anims {
        let fm_name = sanitize_anim(&anim.label);
        // WHEN IT HURTS IS THE CLASS'S OWN WORD. A class states its boxes under the animation they
        // belong to, so the thwomp hits on its way down and at no other time; a lava sheet names one
        // animation and has only that one, so it hurts throughout. Where the class names none, the
        // hit rides whatever the object draws, which is the only reading left.
        let active = hz.attack_anims.is_empty()
            || hz.attack_anims.iter().any(|l| sanitize_anim(l) == fm_name);
        let mut guids = Vec::new();
        for (fi, fr) in anim.frames.iter().enumerate() {
            let base = format!("{hid}_{fm_name}_{fi}");
            let guid = pool.put(sprites, &base, &format!("hazard::{hid}::{fm_name}::{fi}"), &fr.png)?;
            guids.push(guid);
        }
        // frames of one label share a fixed union canvas, so frame 0 carries the placement + size.
        // a label that draws nothing has no frame 0 and no art layer: it is a state that shows
        // nothing, and it hits nobody.
        let (x, y, w, h, active) = match anim.frames.first() {
            Some(f0) => (f0.x * scale, f0.y * scale, f0.w as f64 * scale, f0.h as f64 * scale, active),
            None => (0.0, 0.0, 0.0, 0.0, false),
        };
        hzanims.push(HzAnim {
            name: fm_name, frame_guids: guids,
            x, y, w, h,
            scale, active, hold: anim.stop_frame.is_some(), loops: anim.loops,
        });
    }
    if hzanims.is_empty() { return Ok(()); }
    // An object whose class declares NO attack is not missing its numbers -- it is saying it does
    // not hit anyone. Bomb factory's bricks are scenery you stand on and break; only the explosion
    // hurts. Given hitboxes anyway they were zero-damage volumes that still collided with every
    // fighter that touched them, on every animation.
    // (the boxes recovered from the clip do not argue with this: SSF2 uses those shapes for its own
    // collision, and a class with no getAttackStats deals no damage whatever shapes it carries.)
    if (hz.damage <= 0.0 && hz.knockback <= 0.0) || !hazard_touch_hits(hz) {
        for a in &mut hzanims { a.active = false; }
    }

    // the resting animation a class-less object's template plays
    let idle_name = hzanims.iter().find(|a| a.name.contains("idle") || a.name.contains("rest"))
        .or_else(|| hzanims.iter().find(|a| !a.active))
        .map(|a| a.name.clone()).unwrap_or_else(|| hzanims[0].name.clone());

    // The hit volume = the clip's REAL SSF2 attackBox shapes (clip-local, FM-scaled), recovered the
    // same way the lava's is — the Thwomp's `fall` carries a left + right pair at its slam face. Top-
    // left rects for the COLLISION_BOX emit. If a hazard has no recoverable box, the emitter falls
    // back to the art canvas. This is NOT a guessed/own-stats size: it's the engine attackBox tied
    // to the animation (a HIT_BOX layer that rides the damaging frames).
    let mut hit_boxes: Vec<(f64, f64, f64, f64)> = hz.attack_boxes.iter()
        .map(|r| (r.x, r.y, r.w, r.h)).collect();
    // a static hazard (lava) has no CollisionBox in its art clip — SSF2 checks it by region, not a
    // shape — so attack_boxes is empty and the art-canvas fallback would shrink the hit volume to one
    // small lava tile. its damage covers the whole detected lava AABB (hz.w x hz.h), centred on the
    // CGO origin. COLLISION_BOX (x,y) is the box's un-rotated TOP-LEFT (the pivot only matters for
    // rotation — the convention the character path verified against framy), so centring on the CGO
    // = minus half the size; (0,0) would hang the whole volume below-right of the hazard.
    // ...and only for an object that declares a hit at all: a body nobody is hurt by has no box
    // (a clip with only a body box of its own is hit on it, and hurts no one by touch)
    if hit_boxes.is_empty() && hazard_touch_hits(hz) && hz.w > 0.0 && hz.h > 0.0 {
        hit_boxes.push((-hz.w / 2.0, -hz.h / 2.0, hz.w, hz.h));
    }
    let dual = hit_boxes.len() >= 2;
    write_json(&lib.join("entities").join(format!("{hid}.entity")), &hazard_entity_multi(hid, &hzanims, &hit_boxes, expects_to_be_hit(hz) || hz.body_box))?;
    write_meta(&lib.join("entities").join(format!("{hid}.entity.meta")), hid, hid, "", Some("CUSTOM_GAME_OBJECT"), None)?;

    // an object with a class of its own runs that class: its script is the runtime's
    // (`emit::stage`), written over this one; one without is a template of what it declares
    let script = if hz.class_name.as_deref().is_some_and(|c| !c.is_empty()) { RUNTIME_WRITES.to_string() } else {
        let script = if let Some((speed, label)) = &hz.bounce {
            // ...standing on the body the clip declares, which is what a fighter lands on
            let (decls, helpers) = hz.behavior.self_platform
                .map(|b| self_platform_code(b, scale, &hz.platform_shapes, &hz.body_faces)).unwrap_or_default();
            format!("{decls}{helpers}{}", bounce_multi_script_hx(&hzanims, &idle_name, *speed, label))
        } else {
            hazard_anim_loop_script_hx(hz, &hzanims, &idle_name)
        };
        let script = with_clip_scripts(script, &hz.anims, &stage_own_methods(model));
        let script = with_sync_ledges(script, hz.ledges);
        let script = with_constructor(script);
        let script = with_declared_body(script);
        let script = if hz.no_knockback { with_launch_armor(script) } else { script };
        let script = if hz.survives_bounds { with_bounds_survival(script) } else { script };
        let script = with_no_hitstop(script);
        with_body_release(script)
    };
    // the engine's grounding box: none, its own code places it
    let ecb: Option<(f64, f64)> = None;
    let files = [
        ("Script", script),
        ("GameObjectStats", hazard_gameobject_stats_multi(hid, &idle_name, ecb)),
        ("HitboxStats", hazard_hitbox_stats_multi(hz, &hzanims, dual)),
        ("AnimationStats", hazard_animation_stats_multi(&hzanims, true)),
    ];
    for (kind, body) in files {
        let fname = format!("{hid}{kind}");
        write_script(&scripts.join(format!("{fname}.hx")), &body)?;
        write_meta(&scripts.join(format!("{fname}.hx.meta")), hid, &fname,
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
    Ok(())
}

/// The multi-animation CGO entity: one IMAGE layer per labelled animation (a keyframe per source
/// frame, 30→60fps doubled), and HIT_BOX layer(s) riding the damaging animations. Each FM animation
/// references its image layer (+ its hitbox layers when active). `hit_boxes` are the clip's real
/// SSF2 attackBox rects (clip-local FM top-left x/y, w, h) — one HIT_BOX layer per box (the thwomp's
/// left+right pair → index 0/1, which the stats angle apart). Empty falls back to the full art
/// canvas as a single hit volume.
/// Does this object expect to be attacked? The class says so by listening for its own hurt event:
/// a bomb factory brick breaks when a fighter hits it, and a thwomp never listens at all.
fn expects_to_be_hit(hz: &Hazard) -> bool { crate::stage_text::listens_for_hits(hz) }

fn hazard_entity_multi(hid: &str, anims: &[HzAnim], hit_boxes: &[(f64, f64, f64, f64)],
                       hurtable: bool) -> Value {
    let g = |s: &str| det_uuid(&format!("hazard::{hid}::{s}"));
    let mut symbols = Vec::new();
    let mut keyframes = Vec::new();
    let mut layers = Vec::new();
    let mut animations = Vec::new();
    for (ai, a) in anims.iter().enumerate() {
        let mut kf_ids = Vec::new();
        for (fi, guid) in a.frame_guids.iter().enumerate() {
            let symid = g(&format!("imgsym{ai}_{fi}"));
            symbols.push(json!({ "$id": symid, "type": "IMAGE", "imageAsset": guid, "x": a.x, "y": a.y, "pivotX": 0.0, "pivotY": 0.0, "scaleX": a.scale, "scaleY": a.scale, "rotation": 0.0, "alpha": 1.0, "pluginMetadata": {} }));
            let kfid = g(&format!("imgkf{ai}_{fi}"));
            keyframes.push(json!({ "$id": kfid, "symbol": symid, "length": 1, "tweened": false, "tweenType": "LINEAR", "type": "IMAGE", "pluginMetadata": {} }));
            kf_ids.push(kfid);
        }
        let img_layer = g(&format!("imglayer{ai}"));
        layers.push(json!({ "$id": img_layer, "name": "art", "type": "IMAGE", "hidden": false, "locked": false, "keyframes": kf_ids, "pluginMetadata": {} }));
        let mut anim_layers = vec![img_layer];
        if a.active {
            let span = a.frame_guids.len().max(1) as u64;
            // one HIT_BOX per recovered SSF2 attackBox (the thwomp's left/right pair = index 0/1);
            // no recovered box -> the art canvas as a single fallback hit volume.
            let boxes: Vec<(f64, f64, f64, f64)> = if hit_boxes.is_empty() {
                vec![(a.x, a.y, a.w, a.h)]
            } else {
                hit_boxes.to_vec()
            };
            for (bi, (bx, by, bw, bh)) in boxes.iter().enumerate() {
                let (hw, hh) = (bw / 2.0, bh / 2.0);
                let boxsym = g(&format!("boxsym{ai}_{bi}"));
                symbols.push(json!({ "$id": boxsym, "type": "COLLISION_BOX", "x": bx, "y": by, "pivotX": hw, "pivotY": hh, "scaleX": bw, "scaleY": bh, "rotation": 0.0, "alpha": 0.5, "color": "0xff0000", "pluginMetadata": {} }));
                let boxkf = g(&format!("boxkf{ai}_{bi}"));
                keyframes.push(json!({ "$id": boxkf, "symbol": boxsym, "length": span, "tweened": false, "tweenType": "LINEAR", "type": "COLLISION_BOX", "pluginMetadata": {} }));
                let boxlayer = g(&format!("boxlayer{ai}_{bi}"));
                layers.push(json!({ "$id": boxlayer, "name": format!("hitbox{bi}"), "type": "COLLISION_BOX", "hidden": false, "locked": false, "defaultAlpha": 0.5, "defaultColor": "0xff0000", "keyframes": [boxkf],
                    "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "collisionBoxType": "HIT_BOX", "index": bi } } }));
                anim_layers.push(boxlayer);
            }
        }
        // A HURT box, on every animation, when the class expects to be hit: an SSF2 object that
        // listens for its own hurt event is one a fighter is meant to attack, and with nothing to
        // strike the listener never fires -- bomb factory's bricks could be hit all day and never
        // break. Sized to the art, which is the body the source presents.
        if hurtable {
            let (hw, hh) = (a.w / 2.0, a.h / 2.0);
            let span = a.frame_guids.len().max(1) as u64;
            let sym = g(&format!("hurtsym{ai}"));
            symbols.push(json!({ "$id": sym, "type": "COLLISION_BOX", "x": a.x, "y": a.y, "pivotX": hw, "pivotY": hh, "scaleX": a.w, "scaleY": a.h, "rotation": 0.0, "alpha": 0.5, "color": "0x00ff00", "pluginMetadata": {} }));
            let kf = g(&format!("hurtkf{ai}"));
            keyframes.push(json!({ "$id": kf, "symbol": sym, "length": span, "tweened": false, "tweenType": "LINEAR", "type": "COLLISION_BOX", "pluginMetadata": {} }));
            let layer = g(&format!("hurtlayer{ai}"));
            layers.push(json!({ "$id": layer, "name": "hurtbox", "type": "COLLISION_BOX", "hidden": false, "locked": false, "defaultAlpha": 0.5, "defaultColor": "0x00ff00", "keyframes": [kf],
                "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "collisionBoxType": "HURT_BOX", "index": 0 } } }));
            anim_layers.push(layer);
        }
        animations.push(json!({ "$id": g(&format!("anim{ai}")), "name": a.name.clone(), "layers": anim_layers, "pluginMetadata": {} }));
    }
    let mut e = json!({
        "export": true, "guid": g("entity"), "id": hid, "version": 5,
        "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "objectType": "CUSTOM_GAME_OBJECT", "version": "0.4.0" } },
        "plugins": ["com.fraymakers.FraymakersTypes", "com.fraymakers.FraymakersMetadata"],
        "tags": [], "paletteMap": {}, "tilesets": [], "terrains": [],
        "symbols": symbols, "keyframes": keyframes, "layers": layers, "animations": animations
    });
    if let Some(kfs) = e.get_mut("keyframes").and_then(|k| k.as_array_mut()) {
        crate::entity_gen::double_keyframe_lengths(kfs);
    }
    e
}

/// Build the `LState` registration block + `_prepLocalState` helper for a hazard whose animations
/// are `anims` (registers each FM animation name as a local state, upper-cased; plus the standard
/// UNINITIALIZED sentinel). Shared by the thwomp + the generic animated-hazard scripts.
fn local_state_block(anims: &[HzAnim]) -> String {
    let mut regs = vec!["\tUNINITIALIZED: _prepLocalState(\"#n/a\", -1)".to_string()];
    for a in anims {
        regs.push(format!("\t{}: _prepLocalState(\"{}\")", a.name.to_ascii_uppercase(), a.name));
    }
    format!(
        "function _prepLocalState(animation:String, ?index:Int=Math.NaN):Int {{\n\
         \tif (!__hasInitLocalStateMachine) {{ Common.initLocalStateMachine(); __hasInitLocalStateMachine = true; }}\n\
         \tif (index != Math.NaN) {{ index = __localStatePrepIndex++; }}\n\
         \tCommon.registerLocalState(index, animation);\n\treturn index;\n}}\n\
         var __hasInitLocalStateMachine = false;\nvar __localStatePrepIndex = -1;\n\
         var LState = {{\n{}\n}};\n",
        regs.join(",\n"))
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
    // rest off-screen above the stage (well above the highest platform top = smallest y), so the
    // Thwomp only appears as it drops in.
    let top_y = cols.iter().map(|(_, y)| *y).fold(f64::MAX, f64::min) - 520.0;
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
         // SSF2: the stage spawns a Thwomp every 300 frames @30fps (=600 @60fps) at a random column;\n\
         // it falls (gravity), lands (the platform sinks), waits, then rises off. We model one\n\
         // persistent Thwomp on that cadence: rest -> fall random column -> land -> wait -> rise.\n\
         var REST = 600;\nvar LAND_WAIT = 110;\nvar GRAV = 1.4;\nvar RISE = 8.0;\n\
         // persistent state (a plain var resets every frame on a custom game object).\n\
         var m_phase = self.makeInt(3);\nvar m_col = self.makeInt(0);\nvar m_fallV = self.makeFloat(0.0);\n\
         var m_timer = self.makeInt(0);\nvar m_cool = self.makeInt(0);\nvar m_init = self.makeBool(false);\n\n\
         function initialize() {{\n\tself.setState(PState.ACTIVE);\n\tCommon.toLocalState(LState.ACTIVE);\n}}\n\n\
         function update() {{\n\
         \tif (!m_init.get()) {{ m_init.set(true); m_col.set(Random.getInt(0, COLUMNS.length - 1)); self.setX(COLUMNS[m_col.get()]); self.setY(TOP_Y); }}\n\
         \t// keep the native hitbox live so it damages fighters it falls through.\n\
         \tif (m_cool.get() > 0) {{ m_cool.dec(); }} else {{ self.reactivateHitboxes(); m_cool.set(18); }}\n\
         \tvar p = m_phase.get();\n\
         \tif (p == 0) {{ // fall\n\
         \t\tm_fallV.set(m_fallV.get() + GRAV);\n\t\tself.setY(self.getY() + m_fallV.get());\n\
         \t\tif (self.getY() >= LAND_YS[m_col.get()]) {{ self.setY(LAND_YS[m_col.get()]); m_phase.set(1); m_timer.set(0); }}\n\
         \t}} else if (p == 1) {{ // landed (its platform sinks); wait\n\
         \t\tm_timer.set(m_timer.get() + 1);\n\t\tif (m_timer.get() >= LAND_WAIT) {{ m_phase.set(2); }}\n\
         \t}} else if (p == 2) {{ // rise back to the top\n\
         \t\tself.setY(self.getY() - RISE);\n\
         \t\tif (self.getY() <= TOP_Y) {{ self.setY(TOP_Y); m_phase.set(3); m_timer.set(0); m_fallV.set(0.0); }}\n\
         \t}} else {{ // rest above the stage, then drop on a fresh random column\n\
         \t\tm_timer.set(m_timer.get() + 1);\n\
         \t\tif (m_timer.get() >= REST) {{ m_col.set(Random.getInt(0, COLUMNS.length - 1)); self.setX(COLUMNS[m_col.get()]); m_phase.set(0); m_timer.set(0); }}\n\
         \t}}\n\
         }}\n")
}












/// Asking the answer of a question nothing answers.
///
/// `_v2.getCurrentPlatform().getBounceSpeed()` asks which platform a fighter is standing on, which
/// is not a thing this engine is asked -- so the first call answers nothing and the second lands
/// on it. Judged on the FIRST name only: what it is asked for afterwards does not matter once
/// there is nothing to ask.
/// Reaching into nothing, or calling it.
///
/// Where a cast or a call had no counterpart it was answered with nothing, and the source then
/// indexes that answer by name -- `null["spirit" + i]` -- or calls it: `null("transition3")`.
/// There is no reaching into nothing and no asking it to do anything, so the line cannot do
/// anything and the ask takes the frame instead.
/// A slot nothing ever fills.
///
/// The stage declares a name for a piece of its backdrop and reaches into it -- `globox.eye1.eye`
/// -- and the port has no handle to give it, so the declaration stands at null and nothing ever
/// puts anything in it. Every other use is struck already; the read that is left answers nothing
/// and takes the frame. Judged on the whole script: a slot that IS filled somewhere is not this.
/// A class named as a value, where there are no classes.
///
/// The source says which kind of thing to make by naming the class: `classAPI: SSF2Projectile`.
/// There are no classes here and nothing declares that name, so the read finds nothing and the
/// frame stops. A name is all that crosses, so the name is what it answers with.
///
/// Only a name said BARE -- not followed by a dot or a bracket -- which is what keeps the things
/// this side does have out of it: those are always said with a member after them.






/// The items of a list written out, split where the commas between them are.
///
/// Not at every comma: an item can be worked out by a call of its own, and
/// `[-12, -Random.getInt(0, 12), 12]` split at every comma comes apart in the middle of the call
/// -- which leaves two half-statements that parse nowhere, and the engine drops the script whole.
/// Saffron city's poke hazards lost every object that way.
pub(crate) fn top_level_items(list: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in list.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                items.push(list[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    items.push(list[start..].trim().to_string());
    items
}













/// Returns (declarations, helper functions) for the script to carry.
fn self_platform_code(box_: (f64, f64, f64, f64), scale: f64,
                      shapes: &[(String, (f64, f64, f64, f64))],
                      faces: &[(&'static str, Vec<(f64, f64)>)]) -> (String, String) {
    let (bx, by, bw, bh) = box_;
    let (l, t, r, b) = (bx * scale, by * scale, (bx + bw) * scale, (by + bh) * scale);
    // THE SHAPE IN FORCE, kept where the object keeps the rest of its state. A class that names one
    // clip for its body never changes these; one that names a clip per form it takes sets them as
    // it takes that form, and everything below builds, pools and rebuilds from what they say.
    // The faces built are kept as one list of structure indices; a body is as many faces as its
    // outline has, floors first, so the first face is always one a fighter stands on.
    let decls = format!(
        "var _sp_d = self.makeObject(null);\nvar _sp_form = self.makeInt(0);\n\
         var __spLedgeL = false;\nvar __spLedgeR = false;\n\
         var _sp_l = self.makeFloat({l:.1});\nvar _sp_t = self.makeFloat({t:.1});\n\
         var _sp_r = self.makeFloat({r:.1});\nvar _sp_b = self.makeFloat({b:.1});\n");
    // the bodies the class names, each the size the clip it named was drawn
    let mut shape_fn = String::new();
    if !shapes.is_empty() {
        shape_fn.push_str("function __spShape(n) {\n");
        for (name, (sx, sy, sw, sh)) in shapes {
            shape_fn.push_str(&format!(
                "\tif (n == \"{name}\") {{ return [{:.1}, {:.1}, {:.1}, {:.1}]; }}\n",
                sx * scale, sy * scale, (sx + sw) * scale, (sy + sh) * scale));
        }
        shape_fn.push_str("\treturn null;\n}\n\n\
            // the body it gives itself, shaped like the clip it named for this form\n\
            function __createSelfPlatformShaped(n) {\n\
            \tvar s = __spShape(n);\n\
            \tif (s == null) { return; }\n\
            \t__releaseSelfPlatform();\n\
            \t_sp_form.set(1);\n\
            \t_sp_l.set(s[0]); _sp_t.set(s[1]); _sp_r.set(s[2]); _sp_b.set(s[3]);\n\
            \t__createSelfPlatform();\n\
            }\n\n");
    }
    // the outline the clip declares, floors first, as the runs the engine takes
    let mut runs: Vec<&(&'static str, Vec<(f64, f64)>)> = faces.iter().filter(|(_, pts)| pts.len() >= 2).collect();
    runs.sort_by_key(|(ty, _)| if *ty == "FLOOR" { 0 } else { 1 });
    let baked: String = runs.iter().map(|(ty, pts)| {
        let flat: Vec<String> = pts.iter().flat_map(|(x, y)| [format!("{x:.1}"), format!("{y:.1}")]).collect();
        format!("[StructureType.{ty}, [{}]]", flat.join(", "))
    }).collect::<Vec<_>>().join(", ");
    let runs_fn = if baked.is_empty() {
        "function __spRuns() {\n\treturn __spBoxRuns();\n}\n\n".to_string()
    } else {
        format!("// the outline the source collides with, as the clip drew it\n\
                 function __spRuns() {{\n\
                 \tif (_sp_form.get() != 0) {{ return __spBoxRuns(); }}\n\
                 \treturn [{baked}];\n\
                 }}\n\n")
    };
    (
        decls,
        format!(
            "SHAPE_FN// the four faces of the box in force\n\
             function __spBoxRuns() {{\n\
             \tvar l = _sp_l.get(); var t = _sp_t.get(); var r = _sp_r.get(); var b = _sp_b.get();\n\
             \treturn [[StructureType.FLOOR, [l, t, r, t]], [StructureType.RIGHT_WALL, [r, t, r, b]], [StructureType.CEILING, [r, b, l, b]], [StructureType.LEFT_WALL, [l, b, l, t]]];\n\
             }}\n\n\
             {runs_fn}\
             // A body's shape, as the key the pool files it under: two objects share a body\n\
             // only when they stand on the same outline.\n\
             function __spKey() {{\n\
             \tif (_sp_form.get() == 0) {{ return \"own\"; }}\n\
             \treturn \"\" + _sp_l.get() + \",\" + _sp_t.get() + \",\" + _sp_r.get() + \",\" + _sp_b.get();\n\
             }}\n\n\
             // how many faces the body has\n\
             function __spCount() {{\n\
             \tvar d = _sp_d.get();\n\
             \treturn d == null ? 0 : d.length;\n\
             }}\n\n\
             // One face, or null. The list is shared by everything on the stage, so an index into\n\
             // it is only good while it is still in range, and reaching past the end stops the\n\
             // script.\n\
             function __face(k) {{\n\
             \tvar d = _sp_d.get();\n\
             \tif (d == null || k >= d.length) {{ return null; }}\n\
             \tvar i = d[k];\n\
             \tif (i < 0) {{ return null; }}\n\
             \tvar all = match.getStructures();\n\
             \tif (all == null || i >= all.length) {{ return null; }}\n\
             \treturn all[i];\n\
             }}\n\n\
             // the face fighters stand on, which is what the rest of the port asks for by name\n\
             function __deck() {{\n\
             \treturn __face(0);\n\
             }}\n\n\
             // whether a fighter stands on one of this body's faces\n\
             function __standsOnMe(o) {{\n\
             \tif (o == null) {{ return false; }}\n\
             \tvar f = o.getCurrentFloor();\n\
             \tif (f == null) {{ return false; }}\n\
             \tvar k = 0;\n\
             \twhile (k < __spCount()) {{\n\
             \t\tif (__face(k) == f) {{ return true; }}\n\
             \t\tk = k + 1;\n\
             \t}}\n\
             \treturn false;\n\
             }}\n\n\
             // A BODY THIS STAGE HAS ALREADY BUILT. The engine's structure list has no remove, so an\n\
             // object that makes one and goes leaves it behind, and bomb factory builds a block\n\
             // every second and a half for the whole match. A body of the same shape whose owner\n\
             // has finished with it is handed on instead; only a stage that has never had one builds.\n\
             function __bodyPool() {{\n\
             \tvar st = self.exports.__stage;\n\
             \tif (st == null) {{ return null; }}\n\
             \tif (st.exports.__spareBodies == null) {{ st.exports.__spareBodies = []; }}\n\
             \treturn st.exports.__spareBodies;\n\
             }}\n\n\
             function __releaseSelfPlatform() {{\n\
             \tif (__spCount() == 0) {{ return; }}\n\
             \t__removeSelfPlatform();\n\
             \tvar p = __bodyPool();\n\
             \tif (p != null) {{\n\
             \t\tp.push({{ k: __spKey(), d: _sp_d.get() }});\n\
             \t}}\n\
             \t_sp_d.set(null);\n\
             }}\n\n\
             // one face of the body, built where nobody can reach it and moved onto the object after\n\
             function __buildFace(pts, ty) {{\n\
             \tmatch.createLineSegmentStructure(pts, new StructureStats({{ startX: -2000, startY: -3000, structureType: ty, leftLedge: (ty == StructureType.FLOOR && __spLedgeL), rightLedge: (ty == StructureType.FLOOR && __spLedgeR) }}));\n\
             \tvar all = match.getStructures();\n\
             \tif (all == null || all.length == 0) {{ return false; }}\n\
             \tvar d = _sp_d.get();\n\
             \tif (d == null) {{ d = []; }}\n\
             \td.push(all.length - 1);\n\
             \t_sp_d.set(d);\n\
             \tvar f = __face(d.length - 1);\n\
             \tif (f == null) {{ d.pop(); _sp_d.set(d); return false; }}\n\
             \t// the body never grounds on its own platform (SSF2 self-platform semantics)\n\
             \tf.addToBlacklist(self);\n\
             \treturn true;\n\
             }}\n\n\
             function __createSelfPlatform() {{\n\
             \t// scenery is not destroyed at the bounds: an object that makes itself a platform is\n\
             \t// part of the stage, and the source parks such things wherever it likes (a ship that\n\
             \t// waits above the top of the stage for a minute before it comes down). the engine\n\
             \t// destroys a game object outside the death bounds unless told otherwise.\n\
             \tself.updateGameObjectStats({{ deathBoundsDestroy: false }});\n\
             \tif (__spCount() > 0) {{ return; }}\n\
             \tvar p = __bodyPool();\n\
             \tif (p != null) {{\n\
             \t\tvar j = 0;\n\
             \t\twhile (j < p.length) {{\n\
             \t\t\tif (p[j].k == __spKey()) {{\n\
             \t\t\t\t_sp_d.set(p[j].d);\n\
             \t\t\t\tp.splice(j, 1);\n\
             \t\t\t\tvar q = 0;\n\
             \t\t\t\twhile (q < __spCount()) {{\n\
             \t\t\t\t\tvar df = __face(q);\n\
             \t\t\t\t\tif (df != null) {{ df.addToBlacklist(self); }}\n\
             \t\t\t\t\tq = q + 1;\n\
             \t\t\t\t}}\n\
             \t\t\t\t__moveSelfPlatform();\n\
             \t\t\t\treturn;\n\
             \t\t\t}}\n\
             \t\t\tj = j + 1;\n\
             \t\t}}\n\
             \t}}\n\
             \tvar runs = __spRuns();\n\
             \tvar n = 0;\n\
             \twhile (n < runs.length) {{\n\
             \t\tif (!__buildFace(runs[n][1], runs[n][0])) {{ return; }}\n\
             \t\tn = n + 1;\n\
             \t}}\n\
             \t// and it stands where the body stands, from the moment it exists\n\
             \t__moveSelfPlatform();\n\
             }}\n\n\
             function __moveSelfPlatform() {{\n\
             \tvar k = 0;\n\
             \twhile (k < __spCount()) {{\n\
             \t\tvar d = __face(k);\n\
             \t\tif (d != null) {{ d.setX(self.getX()); d.setY(self.getY()); }}\n\
             \t\tk = k + 1;\n\
             \t}}\n\
             }}\n\n\
             function __removeSelfPlatform() {{\n\
             \t// parked off-world: the structure list has no remove, and a box nobody can reach\n\
             \t// is the same as one that is gone.\n\
             \tvar k = 0;\n\
             \twhile (k < __spCount()) {{\n\
             \t\tvar d = __face(k);\n\
             \t\tif (d != null) {{ d.setX(-2000); d.setY(-3000); }}\n\
             \t\tk = k + 1;\n\
             \t}}\n\
             }}\n\n\
             // whether the body is off, kept here: the engine's face has no getter, and the class\n\
             // asks its own body (`currentPlatform.getFallthrough()`)\n\
             var __spOff = false;\n\
             function __selfPlatformFallthrough() {{ return __spOff; }}\n\
             function __selfPlatformDisabled(b:Bool) {{\n\
             \t__spOff = b;\n\
             \tvar k = 0;\n\
             \twhile (k < __spCount()) {{\n\
             \t\tvar d = __face(k);\n\
             \t\tif (d != null) {{ d.updateStructureStats({{ disabled: b }}); }}\n\
             \t\tk = k + 1;\n\
             \t}}\n\
             }}\n\
             // what the stage's own timeline asks of this object's body (a plane flips its\n\
             // sections' fallthrough at every phase label)\n\
             self.exports.__setDisabled = function(b) {{ __selfPlatformDisabled(b); }};\n\n").replace("SHAPE_FN", &shape_fn),
    )
}




fn stage_own_methods(model: &StageModel) -> Vec<String> { crate::stage_text::StageText::of(model).functions() }


/// `self.exports.X[expr]` where the script assigns `self.exports.X = { k1: .., k2: .. }` becomes
/// `__get_X(expr)`, a getter that switches over the literal's own keys. Only the keys the class
/// wrote exist, so the getter is the whole object, and a key it never wrote reads as nothing.
















/// Expand single-line statement blocks (`if (x) { a; b; }`, `} else { c; }`) into multi-line form
/// so every emitted script reads like hand-written code. Only a `{` preceded by `)`/`else`/`{`
/// with a top-level `;` inside is a statement block — object literals (`{ gravity: 0 }`) stay
/// inline. Applied to every script the stage emitter writes.
fn expand_blocks(code: &str) -> String {
    fn expand_line(line: &str) -> Option<Vec<String>> {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") { return None; }
        let indent: String = line.chars().take_while(|c| *c == '\t').collect();
        let chars: Vec<char> = line.chars().collect();
        for i in 0..chars.len() {
            if chars[i] != '{' { continue; }
            let before: String = chars[..i].iter().collect();
            let b = before.trim_end();
            if !(b.ends_with(')') || b.ends_with("else") || b.ends_with('{')) { continue; }
            // matching close on this line?
            let (mut d, mut j) = (1i32, i + 1);
            while j < chars.len() {
                match chars[j] { '{' => d += 1, '}' => d -= 1, _ => {} }
                if d == 0 { break; }
                j += 1;
            }
            if j >= chars.len() { continue; }
            let inner: String = chars[i + 1..j].iter().collect();
            if inner.trim().is_empty() { continue; }
            // statements = a `;` at top level inside the block
            let mut dd = 0i32;
            let mut has_stmt = false;
            for c in inner.chars() {
                match c { '{' | '(' => dd += 1, '}' | ')' => dd -= 1, ';' if dd == 0 => has_stmt = true, _ => {} }
            }
            if !has_stmt { continue; }
            let mut stmts: Vec<String> = Vec::new();
            let mut cur = String::new();
            dd = 0;
            for c in inner.chars() {
                match c {
                    '{' | '(' => { dd += 1; cur.push(c); }
                    '}' | ')' => { dd -= 1; cur.push(c); }
                    ';' if dd == 0 => { stmts.push(cur.trim().to_string()); cur.clear(); }
                    _ => cur.push(c),
                }
            }
            if !cur.trim().is_empty() { stmts.push(cur.trim().to_string()); }
            let head: String = chars[..=i].iter().collect();
            let tail: String = chars[j + 1..].iter().collect::<String>().trim_start().to_string();
            let mut out = vec![head.trim_end().to_string()];
            for st in stmts.into_iter().filter(|st| !st.is_empty()) {
                out.push(format!("{indent}\t{st};"));
            }
            if tail.is_empty() { out.push(format!("{indent}}}")); }
            else { out.push(format!("{indent}}} {tail}")); }
            return Some(out);
        }
        None
    }
    let mut lines: Vec<String> = code.lines().map(str::to_string).collect();
    loop {
        let mut changed = false;
        let mut next: Vec<String> = Vec::with_capacity(lines.len());
        for line in &lines {
            match expand_line(line) {
                Some(ex) => { next.extend(ex); changed = true; }
                None => next.push(line.clone()),
            }
        }
        lines = next;
        if !changed { break; }
    }
    lines.join("\n")
}

/// Write an emitted hscript file, expanded to one statement per line.
fn write_script(path: &std::path::Path, body: &str) -> std::io::Result<()> {
    std::fs::write(path, expand_blocks(body))
}



/// A generic animated-hazard script (non-thwomp multi-animation hazards): registers every labelled
/// animation, plays the resting `idle` one, and keeps the native HIT_BOX re-armed so a lingering
/// fighter keeps taking hits. Universal fallback for any future multi-animation stage hazard.
/// A platform that declares a bounce (`bounce_speed`, `bounce_frame`): the game's platform layer
/// sends a fighter that lands on it up at the declared speed and plays the declared label on the
/// clip; this object does the same with the numbers the clip states, and comes back to its rest
/// label when the bounce has played (the clip's own frame script stops it there).
fn bounce_multi_script_hx(anims: &[HzAnim], idle: &str, speed: f64, label: &str) -> String {
    let idle_s = idle.to_ascii_uppercase();
    let bounce_s = anims.iter().find(|a| a.name == sanitize_anim(label)).map(|a| a.name.to_ascii_uppercase()).unwrap_or(idle_s.clone());
    let up = -speed * velocity_scale();
    format!(
        "// a platform that declares a bounce (`bounce_speed`, `bounce_frame`): a fighter landing on it\n\
         // is sent up at the declared speed and the clip plays its bounce label, as the source's\n\
         // platform layer does with those declarations; the rest label is the clip's own\n\n\
         {states}\n\
         var __onMe = [];\n\
         // what the clip declares, published for an object that asks its floor (a goomba bounces\n\
         // off a spring by the spring's own speed)\n\
         self.exports.getBounceSpeed = {speed:.4};\n\
         self.exports.bounceSpeed = {speed:.4};\n\
         function initialize() {{\n\tself.setState(PState.ACTIVE);\n\tCommon.toLocalState(LState.{idle_s});\n}}\n\n\
         function update() {{\n\
         \tif (Common.inLocalState(LState.{bounce_s}) && self.getCurrentFrame() >= self.getTotalFrames()) {{ Common.toLocalState(LState.{idle_s}); }}\n\
         \tvar ps = match.getCharacters();\n\
         \tvar i = 0;\n\
         \twhile (i < ps.length) {{\n\
         \t\tvar p = ps[i];\n\
         \t\tvar on = __standsOnMe(p);\n\
         \t\twhile (__onMe.length <= i) {{ __onMe.push(false); }}\n\
         \t\tif (on && !__onMe[i]) {{\n\
         \t\t\tp.setYSpeed({up:.4});\n\
         \t\t\tCommon.toLocalState(LState.{bounce_s});\n\
         \t\t}}\n\
         \t\t__onMe[i] = on;\n\
         \t\ti = i + 1;\n\
         \t}}\n\
         }}\n",
        states = local_state_block(anims))
}

fn hazard_anim_loop_script_hx(hz: &Hazard, anims: &[HzAnim], idle: &str) -> String {
    let idle_s = idle.to_ascii_uppercase();
    format!(
        "// Animated stage hazard (custom game object) — converted from SSF2. Local state machine\n\
         // across the labelled animations; native HIT_BOX (HitboxStats) on the active ones.\n\
         // Cross-frame state via self.make* (a plain var re-inits every frame on a game object).\n\n\
         {states}\n\
         var m_cooldown = self.makeFrameTimer({rehit});\n\n\
         function initialize() {{\n\tself.setState(PState.ACTIVE);\n\tCommon.toLocalState(LState.{idle_s});\n}}\n\n\
         function update() {{\n\
         \tm_cooldown.tick();\n\
         \tif (m_cooldown.completed) {{ self.reactivateHitboxes(); m_cooldown.reset(); }}\n\
         }}\n",
        states = local_state_block(anims), rehit = hz.rehit())
}

/// GameObjectStats for a multi-animation hazard: PState.ACTIVE plays the resting `idle` animation
/// (the local state machine swaps in the others), so the engine always has a current animation for
/// the collision detector to resolve the native HIT_BOX against.
fn hazard_gameobject_stats_multi(hid: &str, idle: &str, ecb: Option<(f64, f64)>) -> String {
    // a MOVING hazard needs an ECB for the engine to ground it on floors (without one a falling
    // game object passes straight through stage terrain — isOnFloor() never fires). sized from
    // the hazard's own SSF2 collision box: head = body height above the origin, hips = half-width.
    let ecb_block = ecb.map(|(head, hipw)| format!(
        "\tfloorFootPosition: 0,\n\tfloorHeadPosition: {head:.0},\n\tfloorHipWidth: {hipw:.0},\n\
         \taerialFootPosition: 0,\n\taerialHeadPosition: {head:.0},\n\taerialHipWidth: {hipw:.0},\n"))
        .unwrap_or_default();
    format!(
        "// GameObjectStats for {hid}\n{{\n\tspriteContent: self.getResource().getContent(\"{hid}\"),\n\tinitialState: PState.ACTIVE,\n\
         \tstateTransitionMapOverrides: [\n\t\tPState.ACTIVE => {{ animation: \"{idle}\" }}\n\t],\n\
         \tbaseScaleX: 1,\n\tbaseScaleY: 1,\n\tweight: 100,\n\tgravity: 0,\n\tfriction: 0,\n\t// AND IN THE AIR. A hazard that travels is airborne its whole life, and the engine bleeds\n\t// a flying object's horizontal speed away by the aerial figure, which is not zero unless it\n\t// is said to be. SSF2's enemies declare no friction at all and keep whatever speed they were\n\t// given: clocktown's meteors are aimed across the town and arrived falling straight down,\n\t// their sideways speed gone within a dozen frames of being set.\n\taerialFriction: 0,\n\timmovable: true,  // a stage hazard shoves fighters, never the reverse (windboxes included)\n{ecb_block}}}\n")
}

/// HitboxStats for a multi-animation hazard: entries on each ACTIVE (damaging) animation, empty on
/// the safe ones. Keyed by FM animation name, matching the entity's HIT_BOX layers. `dual` splits
/// the hit into left/right half boxes with mirrored away-from-center angles (the SSF2 thwomp's two
/// attackBoxes: direction 135 on the left half, 45 on the right).
fn hazard_hitbox_stats_multi(hz: &Hazard, anims: &[HzAnim], dual: bool) -> String {
    // per-box launch directions from the class's getAttackStats (the Thwomp's 135 left / 45 right),
    // falling back to the away-from-center default only if the source declared none.
    let dir = |i: usize, fallback: f64| hz.hitbox_dirs.get(i).copied().unwrap_or(fallback);
    let mut entries = Vec::new();
    for a in anims {
        if a.active {
            if dual {
                entries.push(format!(
                    "\t{n}: {{\n\
                     \t\thitbox0: {{ damage: {d}, angle: {a0}, baseKnockback: {kb}, knockbackGrowth: {g}, \
                     {t0}reversibleAngle: false, directionalInfluence: true, reflectable: false }},\n\
                     \t\thitbox1: {{ damage: {d}, angle: {a1}, baseKnockback: {kb}, knockbackGrowth: {g}, \
                     {t1}reversibleAngle: false, directionalInfluence: true, reflectable: false }}\n\t}}",
                    n = a.name, d = hz.damage, kb = kb_fm("baseKnockback", hz.knockback),
                    g = kb_fm("knockbackGrowth", hz.kb_growth), a0 = dir(0, 135.0), a1 = dir(1, 45.0),
                    t0 = hazard_hit_timing(hz, 0), t1 = hazard_hit_timing(hz, 1)));
            } else {
                entries.push(format!(
                    "\t{}: {{\n\t\thitbox0: {{ damage: {}, angle: {}, baseKnockback: {}, knockbackGrowth: {}, \
                     {}reversibleAngle: true, directionalInfluence: true, reflectable: false }}\n\t}}",
                    a.name, hz.damage, hz.angle,
                    kb_fm("baseKnockback", hz.knockback), kb_fm("knockbackGrowth", hz.kb_growth),
                    hazard_hit_timing(hz, 0)));
            }
        } else {
            entries.push(format!("\t{}: {{}}", a.name));
        }
    }
    format!(
        "// HitboxStats for the stage hazard — SSF2 attack stats (power -> baseKnockback,\n\
         // kbConstant -> knockbackGrowth, direction -> angle), knockback in Fraymakers units.\n{{\n{}\n}}\n",
        entries.join(",\n"))
}

/// AnimationStats for a multi-animation hazard: cyclic phases (idle face, fall speed-lines) LOOP
/// like the SSF2 movieclips; a one-shot entrance/intro plays once and holds its last frame (the
/// script's local state machine does the switching).
/// `class_driven`: the object runs its own class's state machine and leaves each state by testing
/// whether the clip has finished. SSF2 clips carry no stop() of their own there -- its engine holds
/// them at the end of a `forceAttack` -- so read as loops they restart under the very test that is
/// watching for the end, and a broken block plays its break over and over.
fn hazard_animation_stats_multi(anims: &[HzAnim], class_driven: bool) -> String {
    let entries: Vec<String> = anims.iter()
        .map(|a| {
            // hold-vs-loop is the SWF's own declaration: a sub-clip with a stop() frame script
            // holds (NONE), one without loops — read from the timeline class, not guessed. The
            // entrance/intro keyword stays only as a fallback for clips with no script data.
            //
            // A class that drives its own animations holds each at its end, because the engine
            // does that at the end of the call that played it and the class watches for it. That
            // only means anything when there is somewhere to go NEXT: an object with a single
            // animation is never waiting for it to finish, and holding its last frame just stops
            // it dead. Clocktown's meteor is one of those -- it plays one clip for its whole life,
            // and held, it fell as a still picture.
            // a clip that says it loops loops, whatever is driving the object
            let one_shot = !a.loops && (a.hold
                || (class_driven && anims.len() > 1)
                || a.name.contains("entrance") || a.name.contains("intro"));
            let end = if one_shot { "NONE" } else { "LOOP" };
            format!("\t{}: {{ endType: AnimationEndType.{end} }}", a.name)
        }).collect();
    format!("// AnimationStats for the stage hazard.\n{{\n{}\n}}\n", entries.join(",\n"))
}

/// A platform that declares a bounce: the game's platform layer sends a fighter that lands on it
/// up at the declared speed and plays the declared label on the clip, and this object does the
/// same with the numbers the clip states. Its rest animation is the clip's `idle` label (else its
/// first), which the bounce label's own frame script returns to.
fn bounce_script_hx(hz: &Hazard, speed: f64, label: &str) -> String {
    let idle = hz.anims.iter().find(|a| a.label == "idle").or(hz.anims.first()).map(|a| a.label.clone()).unwrap_or_else(|| "idle".to_string());
    // the declared speed in this engine's units (the script has no speed helper of its own)
    let up = -speed * velocity_scale();
    format!(
        "// a platform that declares a bounce (`bounce_speed`, `bounce_frame`): a fighter landing on it\n\
         // is sent up at the declared speed and the clip plays its bounce label, as the source's\n\
         // platform layer does with those declarations\n\
         var __onMe = [];\n\
         function initialize() {{\n\
         \tself.playAnimation(\"{idle}\");\n\
         }}\n\
         function update() {{\n\
         \tvar ps = match.getCharacters();\n\
         \tvar i = 0;\n\
         \twhile (i < ps.length) {{\n\
         \t\tvar p = ps[i];\n\
         \t\tvar on = __standsOnMe(p);\n\
         \t\twhile (__onMe.length <= i) {{ __onMe.push(false); }}\n\
         \t\tif (on && !__onMe[i]) {{\n\
         \t\t\tp.setYSpeed({up:.4});\n\
         \t\t\tself.playAnimation(\"{label}\");\n\
         \t\t}}\n\
         \t\t__onMe[i] = on;\n\
         \t\ti = i + 1;\n\
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
    // motion reads persistent state through `.get()` (m_frame/m_baseX/m_baseY are `self.make*`).
    // NO synthesized motion. An object moves because its own translated code moves it; a sine
    // wave or a hand-drawn fall curve here would be the converter making up how something behaves,
    // which is right for nothing that was not measured against it.
    let motion = String::new();
    // pulse: toggle the active (hitbox) and inactive (no hitbox) states on the duty cycle.
    let pulse = if hz.interval > 0 {
        format!(
            "\tvar on = (m_frame.get() % {iv}) < {ac};\n\
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
         // persistent state (a plain var resets every frame on a custom game object).\n\
         var m_frame = self.makeInt(0);\nvar m_baseX = self.makeFloat(0.0);\nvar m_baseY = self.makeFloat(0.0);\n\
         var m_init = self.makeBool(false);\nvar m_cooldown = self.makeInt(0);\n\n\
         function initialize() {{\n\tself.setState(PState.ACTIVE);\n\tCommon.toLocalState(LState.ACTIVE);\n}}\n\n\
         function update() {{\n\
         \tif (!m_init.get()) {{ m_baseX.set(self.getX()); m_baseY.set(self.getY()); m_init.set(true); }}\n\
         \tm_frame.set(m_frame.get() + 1);\n\
{motion}{pulse}\
         \t// re-arm the native HIT_BOX so a fighter standing in the hazard keeps taking hits\n\
         \t// (a hitbox hits each target once per attack id; reactivateHitboxes issues a fresh one).\n\
         \tif (Common.inLocalState(LState.ACTIVE)) {{\n\
         \t\tif (m_cooldown.get() > 0) {{ m_cooldown.dec(); }}\n\
         \t\telse {{ self.reactivateHitboxes(); m_cooldown.set(REHIT); }}\n\
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
         \tbaseScaleX: 1,\n\tbaseScaleY: 1,\n\tweight: 100,\n\tgravity: 0,\n\tfriction: 0,\n\t// AND IN THE AIR. A hazard that travels is airborne its whole life, and the engine bleeds\n\t// a flying object's horizontal speed away by the aerial figure, which is not zero unless it\n\t// is said to be. SSF2's enemies declare no friction at all and keep whatever speed they were\n\t// given: clocktown's meteors are aimed across the town and arrived falling straight down,\n\t// their sideways speed gone within a dozen frames of being set.\n\taerialFriction: 0,\n\timmovable: true,  // a stage hazard shoves fighters, never the reverse (windboxes included)\n}}\n")
}

/// A hazard's knockback in Fraymakers units. A hazard hits with the same engine a fighter does,
/// so it converts with the same measured factors (`mappings/character/hitbox_stats.jsonc`).
fn kb_fm(fm_field: &str, v: f64) -> String {
    crate::haxe_gen::fmt_num(v * crate::mappings::character_hitbox_stats().scale_for(fm_field))
}

/// The TIMING of a hit, as the class states it.
///
/// How long the game freezes on a hit and how long the victim is held are the attack's own numbers,
/// converted the same way a fighter's are. The port used to write a fixed pair for every hazard on
/// every stage, which is a hit the source never described: bowser's castle's lava declares that its
/// hold is the engine's to work out, and got a flat twenty-four frames instead. What the class does
/// not state is not emitted, so Fraymakers works it out exactly as SSF2 does.
fn hazard_hit_timing(hz: &Hazard, box_idx: usize) -> String {
    let m = crate::mappings::character_hitbox_stats();
    let Some(h) = hz.attack_stats.get(box_idx).or_else(|| hz.attack_stats.first()) else { return String::new() };
    let mut out = String::new();
    for fm_field in ["hitstop", "selfHitstop", "hitstun"] {
        let Some(v) = m.keys_for(fm_field).iter().find_map(|k| h.get(k.as_str()).copied()) else { continue };
        // a negative duration is the source saying "the engine decides", which it does by being
        // told nothing
        if v < 0.0 { continue }
        let v = (v + m.plus_frames_for(fm_field)) * m.scale_for(fm_field)
            * if m.is_frame(fm_field) { 2.0 } else { 1.0 };
        out.push_str(&format!("{fm_field}: {}, ", crate::haxe_gen::fmt_num(v)));
    }
    out
}

fn hazard_hitbox_stats_hx(hz: &Hazard) -> String {
    if !hazard_hits(hz) {
        return "// HitboxStats for the stage object: it declares no hit, so it has no hit box.\n\
                {\n\tgameObjectIdle: {},\n\tgameObjectInactive: {}\n}\n".to_string();
    }
    format!(
        "// HitboxStats for the stage hazard — SSF2 attack stats (power -> baseKnockback,\n\
         // kbConstant -> knockbackGrowth, direction -> angle).\n\
         {{\n\tgameObjectIdle: {{\n\t\thitbox0: {{ damage: {}, angle: {}, baseKnockback: {}, knockbackGrowth: {}, \
         {}reversibleAngle: true, directionalInfluence: true, reflectable: false }}\n\t}},\n\
         \tgameObjectInactive: {{}}\n}}\n",
        hz.damage, hz.angle, kb_fm("baseKnockback", hz.knockback), kb_fm("knockbackGrowth", hz.kb_growth),
        hazard_hit_timing(hz, 0))
}

fn hazard_animation_stats_hx() -> String {
    "// AnimationStats for the stage hazard.\n{\n\tgameObjectIdle: { endType: AnimationEndType.NONE },\n\tgameObjectInactive: { endType: AnimationEndType.NONE }\n}\n".to_string()
}

// ── particle weather -> N looping VFX the stage drifts every frame ───────────
//
// Weather is neither backdrop art nor a hazard: SSF2 builds it in CODE, so it is never placed on a
// timeline for the art walk to find, and it damages nobody so the hazard path ignores it. A stage
// converts with every layer correct and the air empty.
//
// One VFX ENTITY holds the particle; the stage spawns `count` copies and moves each itself.
// Rise, drift and alpha are rolled PER PARTICLE from the ranges the source rolls them from, so the
// field looks scattered rather than like one animation played N times.

/// The particle entity: one still frame, looping forever, reparented like any backdrop VFX.
fn weather_entity(eid: &str, guid: &str, w: u32, h: u32, scale: f64) -> Value {
    let g = |s: &str| det_uuid(&format!("weather::{eid}::{s}"));
    json!({
        "export": true, "guid": g("entity"), "id": eid, "version": 5,
        "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "objectType": "VFX", "version": "0.1.0" } },
        "plugins": ["com.fraymakers.FraymakersTypes", "com.fraymakers.FraymakersMetadata"],
        "tags": [], "paletteMap": {}, "tilesets": [], "terrains": [],
        "symbols": [ json!({
            "$id": g("sym"), "type": "IMAGE", "imageAsset": guid,
            // drawn around its own centre, so a particle's position is where it LOOKS like it is
            "x": -(w as f64) * scale / 2.0, "y": -(h as f64) * scale / 2.0,
            "pivotX": 0.0, "pivotY": 0.0, "scaleX": scale, "scaleY": scale,
            "rotation": 0.0, "alpha": 1.0, "pluginMetadata": {}
        }) ],
        "keyframes": [ json!({ "$id": g("kf"), "symbol": g("sym"), "length": 1,
            "tweened": false, "tweenType": "LINEAR", "type": "IMAGE", "pluginMetadata": {} }) ],
        "layers": [ json!({ "$id": g("layer"), "name": "art", "type": "IMAGE", "hidden": false,
            "locked": false, "keyframes": [g("kf")], "pluginMetadata": {} }) ],
        "animations": [ json!({ "$id": g("anim"), "name": "active", "layers": [g("layer")],
            "pluginMetadata": {} }) ]
    })
}

/// Write the particle art + entity. Returns the field's boot table row and its per-frame code.
/// Ceiling on live weather particles. The source count is scaled up to keep the authored DENSITY
/// over Fraymakers' larger view, and this stops a wide pull-out (or a misread source) turning that
/// into a swarm.
const WEATHER_MAX_PARTICLES: u32 = 200;

/// Every field the stage runs, each with its own particles and its own strength.
fn emit_weather(model: &StageModel, lib: &Path) -> Result<Vec<BootWeather>> {
    let mut boot = Vec::new();
    for (i, w) in model.weathers.iter().enumerate() {
        boot.extend(emit_one_weather(model, lib, w, i)?);
    }
    Ok(boot)
}

fn emit_one_weather(model: &StageModel, lib: &Path, w: &crate::stage_parser::Weather, idx: usize) -> Result<Option<BootWeather>> {
    // A count of zero means the class's own code did not yield one (the value is neither passed at
    // the construction nor declared as a default). Emitting anyway gives a loop over nothing: the
    // stage carries a weather entity and renders no weather, which looks like it works. Say so
    // instead, so it reads as a known gap rather than a silent one.
    if w.count == 0 {
        log::warn!("stage weather: found the particle ({}x{}) but not how many, so no weather is emitted", w.art.w, w.art.h);
        return Ok(None);
    }
    let eid = format!("{}_weather{}", model.id, idx);
    let sprites = lib.join("sprites").join("Stage");
    std::fs::create_dir_all(&sprites).ok();
    let png_name = format!("{eid}.png");
    std::fs::write(sprites.join(&png_name), &w.art.png).context("write weather particle")?;
    let guid = det_uuid(&format!("weather::{eid}::image"));
    write_json(&sprites.join(format!("{png_name}.meta")), &json!({
        "export": false, "guid": guid, "id": "", "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
    }))?;
    if w.frames.len() > 1 {
        // the particle plays its clip's own frames, around the centre of its first one, so a
        // particle's position stays where it looks like it is
        let mut pool = PngPool::default();
        let (cx, cy) = (w.frames[0].x + w.frames[0].w as f64 / 2.0, w.frames[0].y + w.frames[0].h as f64 / 2.0);
        let mut cels: Vec<ArtRef> = Vec::new();
        for (fi, fr) in w.frames.iter().enumerate() {
            let g = pool.put(&sprites, &format!("{eid}_{fi}"), &format!("weather::{eid}::{fi}"), &fr.png)?;
            cels.push(ArtRef { guid: g, x: (fr.x - cx) * model.scale, y: (fr.y - cy) * model.scale,
                w: fr.w, h: fr.h, hold: fr.hold.max(1) as usize, alpha: fr.alpha, pivot: (0.0, 0.0), blank: false,
                rotation: fr.rotation, scale_x: fr.scale_x, scale_y: fr.scale_y, tint: fr.tint });
        }
        let phases = vec![("active".to_string(), cels)];
        write_json(&lib.join("entities").join(format!("{eid}.entity")),
                   &bg_element_entity_phased(&eid, &phases, model.scale, (0.0, 0.0)))?;
    } else {
        write_json(&lib.join("entities").join(format!("{eid}.entity")),
                   &weather_entity(&eid, &guid, w.art.w, w.art.h, model.scale))?;
    }

    // SSF2 scatters over about 1.5x the camera area and wraps at those edges, which keeps
    // particles arriving from off-screen instead of appearing at the frame edge.
    let hw = (w.width * 0.75).round() as i64;
    let hh = (w.height * 0.75).round() as i64;
    let count = w.count.min(WEATHER_MAX_PARTICLES);   // a stage asking for thousands is a stage we misread
    let vs = velocity_scale();
    // The drifts are the SOURCE's, read off the weather class (`abc_parser::weather_motion`):
    // rain falls fast and straight, embers rise and wander, and which one this stage authored is
    // not something to pick. Per-frame speeds, so they convert like every other one. A class this
    // pass could not read falls back to the drift bowserscastle's embers use, and says so.
    let motion = w.motion.unwrap_or_else(|| {
        log::warn!("stage weather: could not read the particle motion out of the weather class,                     so the particles drift instead of doing what the source does");
        crate::abc_parser::WeatherMotion {
            dy: crate::abc_parser::Drift { base: -1.0, rand: -2.0 },
            dx: crate::abc_parser::Drift { base: -1.5, rand: 4.2 },
            scale: None,
        }
    });
    // `base + rand * U(0,1)` per axis, in FM units per frame. Drawn as integer hundredths
    // because that is the randomness hscript offers.
    let draw = |d: crate::abc_parser::Drift| BootDraw { lo: d.base * vs, span: (d.rand * vs * 100.0).round() as i64 };
    // The SIZE the source gives each particle, read off the weather class the same way its drift
    // is. SSF2 draws every raindrop from one clip and then scales it per drop, so a field of them
    // has depth in it; emitted all at one size they are a hundred copies of one drop, and the
    // whole field reads as moving together.
    let scale = motion.scale.map(|d| BootDraw { lo: d.base, span: (d.rand * 100.0).round() as i64 });
    // Both the scatter and the COUNT come off the live camera, not off the source numbers.
    // SSF2's field is one SSF2 screenful; Fraymakers shows a good deal more world than that, so
    // reusing the source's area scatters the particles into a band in the middle, and reusing the
    // source's count spreads that many across a bigger screen and reads as thinner weather than
    // the original. Scaling the count by the area ratio keeps the DENSITY the source authored,
    // which is what actually looks the same. Capped so a big pull-out cannot spawn a swarm.
    // The source area is what the count's DENSITY is relative to. A stage whose class names its
    // area fields something this pass does not recognise reports none, and scaling would then be
    // dividing by a number we do not have -- so the source count is used as-is instead.
    // Every drop is drawn solid: the source fades the whole FIELD from the stage class
    // (`m_rain.setAlpha` on its own timer), never one drop.
    let src_area = (hw.max(0) * hh.max(0)) as f64;
    let boot = BootWeather {
        field: idx, id: eid.clone(), hw, hh, count, cap: WEATHER_MAX_PARTICLES,
        area: (src_area > 0.0).then_some(src_area),
        dy: draw(motion.dy), dx: draw(motion.dx), scale,
    };
    Ok(Some(boot))
}

// ── animated backdrop element -> independent-loop VFX, reparented into a background container ──
// the architectural fix for Flash's two timeline features the single baked `stage` animation
// can't represent: (1) a nested movieclip whose loop is INDEPENDENT of the parent (a 260-frame
// element plays all 260 regardless of the master length), and (2) multiple distinct objects on
// one layer+frame. each promoted element becomes its own VFX (objectType "VFX", like the character
// port's effects) whose `active` animation is just that element's frames on LOOP (its own clock),
// and the stage reparents its sprite into a background CONTAINER for depth:
//   var v = match.createVfx(new VfxStats({ spriteContent, animation: "active", loop: true,
//                                          timeout: -1, relativeWith: false }));
//   self.getBackgroundBehindContainer().addChild(v.getSprite());
// depth control is the Stage container API (getBackground*Container etc., each returns a Container)
// + Vfx.getSprite() (a DisplayObject) + Container.addChild — the same reparent the CGO version used,
// just getSprite() instead of getViewRootContainer(). a plain VfxLayer drew at the wrong depth, so
// the explicit container reparent is authoritative. VFX carry no scripts/stats/manifest entry.

/// One backdrop-element VFX entity: the element's frames as a single `active` animation on LOOP
/// (looping driven by VfxStats.loop at spawn). objectType "VFX" so createVfx's spriteContent
/// accepts it. references the bg sprite GUIDs already written by `write_layer` (no PNG re-write);
/// keyframe lengths are the per-frame holds (already FM 60fps frames, like the baked bg path).
/// The element's own origin: the top-left it is placed at on the stage.
///
/// Every frame of a static element carries the SAME stage coordinate, and an animated one
/// carries near-identical coords per frame, so this is the element's position rather than a
/// per-frame accident. Hoisting it out of the art is what gives the vfx a real position (see
/// `bg_element_entity`).
fn bg_element_origin(layer: &BgLayerRef) -> (f64, f64) {
    // The element's placement, lifted out of the art and carried on the VFX ITSELF
    // (`VfxStats.x/y`), which works because the stage reparents the vfx's VIEW ROOT CONTAINER
    // rather than its sprite (see `emit_bg_elements`).
    //
    // Reparenting `getSprite()` is what made an earlier attempt at this render wrong: it lifts
    // the sprite OUT of the entity's view hierarchy, so the entity's position stops moving it
    // and every element collapses onto the layer container's origin -- measured live at
    // absolute (-163,-277) for all of them, which on bowserscastle put the podoboos up in the
    // chandeliers instead of in the lava. Moving the view ROOT keeps the entity intact, so its
    // position means what it says and the element stays one coherent, movable object.
    let x = layer.frames.iter().map(|f| f.x).fold(f64::MAX, f64::min);
    let y = layer.frames.iter().map(|f| f.y).fold(f64::MAX, f64::min);
    if x.is_finite() && y.is_finite() { (x, y) } else { (0.0, 0.0) }
}



/// The same element, written as one animation per phase.
fn bg_element_entity_phased(eid: &str, phases: &[(String, Vec<ArtRef>)], scale: f64, origin: (f64, f64)) -> Value {
    let (ox, oy) = origin;
    let g = |s: &str| det_uuid(&format!("bgelem::{eid}::{s}"));
    let (mut symbols, mut keyframes, mut layers, mut animations) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (name, cels) in phases {
        let mut kf_ids = Vec::new();
        let mut kfs: Vec<Value> = Vec::new();
        for (j, f) in cels.iter().enumerate() {
            if f.blank {
                // no symbol: the object is not placed on these frames
                let kid = g(&format!("{name}kfblank{j}"));
                kfs.push(json!({ "$id": kid, "length": f.hold.max(1), "tweened": false,
                    "tweenType": "LINEAR", "type": "IMAGE", "pluginMetadata": {} }));
                kf_ids.push(kid);
                continue;
            }
            let sid = g(&format!("{name}sym{j}"));
            symbols.push(json!({
                "$id": sid, "type": "IMAGE", "imageAsset": f.guid,
                "x": f.x - ox, "y": f.y - oy, "pivotX": 0.0, "pivotY": 0.0,
                "scaleX": scale * f.scale_x, "scaleY": scale * f.scale_y, "rotation": f.rotation,
                "alpha": f.alpha, "pluginMetadata": {}
            }));
            let kid = g(&format!("{name}kf{j}"));
            kfs.push(json!({ "$id": kid, "symbol": sid, "length": f.hold.max(1),
                "tweened": false, "tweenType": "LINEAR", "type": "IMAGE", "pluginMetadata": {} }));
            kf_ids.push(kid);
        }
        crate::entity_gen::double_keyframe_lengths(&mut kfs);
        keyframes.extend(kfs);
        let lid = g(&format!("{name}layer"));
        layers.push(json!({ "$id": lid, "name": "art", "type": "IMAGE", "hidden": false,
            "locked": false, "keyframes": kf_ids, "pluginMetadata": {} }));
        animations.push(json!({ "$id": g(&format!("{name}anim")), "name": name,
            "layers": [lid], "pluginMetadata": {} }));
    }
    json!({
        "export": true, "guid": g("entity"), "id": eid, "version": 5,
        "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "objectType": "VFX", "version": "0.1.0" } },
        "plugins": ["com.fraymakers.FraymakersTypes", "com.fraymakers.FraymakersMetadata"],
        "tags": [], "paletteMap": {}, "tilesets": [], "terrains": [],
        "symbols": symbols, "keyframes": keyframes, "layers": layers, "animations": animations
    })
}

fn bg_element_entity(eid: &str, layer: &BgLayerRef, scale: f64) -> Value {
    let origin = bg_element_origin(layer);
    bg_element_entity_at(eid, layer, scale, origin)
}

fn bg_element_entity_at(eid: &str, layer: &BgLayerRef, scale: f64, origin: (f64, f64)) -> Value {
    // Art is element-LOCAL: the stage offset is hoisted onto the vfx itself (VfxStats x/y)
    // instead of being baked into every symbol. Baking it in meant every element sat at the
    // world origin with its picture drawn far away — so it had no position to read, none to
    // move, and nothing to tell two elements apart by. Per-frame offsets are preserved as
    // deltas from this origin, so an element whose frames crop differently still animates.
    let (ox, oy) = origin;
    let g = |s: &str| det_uuid(&format!("bgelem::{eid}::{s}"));
    let symbols: Vec<Value> = layer.frames.iter().enumerate().filter(|(_, f)| !f.blank).map(|(j, f)| json!({
        "$id": g(&format!("sym{j}")), "type": "IMAGE", "imageAsset": f.guid,
        "x": f.x - ox, "y": f.y - oy, "pivotX": 0.0, "pivotY": 0.0,
        // the frame's own orientation composes with the stage scale: an element emitted as cels
        // plus placements carries its turn here instead of in its pixels
        "scaleX": scale * f.scale_x, "scaleY": scale * f.scale_y, "rotation": f.rotation,
        // the frame's OWN opacity: a tint sheet whose strength follows the time of day, or a
        // glow that pulses, is animated by alpha in the source, and a fixed value for the whole
        // plane paints it over the stage at full strength for the whole match
        "alpha": f.alpha, "pluginMetadata": {}
    })).collect();
    let mut keyframes: Vec<Value> = layer.frames.iter().enumerate().map(|(j, f)| {
        let mut kf = json!({ "$id": g(&format!("kf{j}")), "length": f.hold.max(1),
            "tweened": false, "tweenType": "LINEAR", "type": "IMAGE", "pluginMetadata": {} });
        if !f.blank { kf["symbol"] = json!(g(&format!("sym{j}"))); }
        kf
    }).collect();
    // 30fps SSF2 source -> 60fps engine: double every keyframe length, exactly like the main stage
    // entity (build_entity) and the hazards. Without this the promoted VFX backdrop elements (the
    // torches, embers) animate at half the speed of the rest of the stage.
    crate::entity_gen::double_keyframe_lengths(&mut keyframes);
    let kf_ids: Vec<String> = (0..layer.frames.len()).map(|j| g(&format!("kf{j}"))).collect();
    json!({
        "export": true, "guid": g("entity"), "id": eid, "version": 5,
        "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "objectType": "VFX", "version": "0.1.0" } },
        "plugins": ["com.fraymakers.FraymakersTypes", "com.fraymakers.FraymakersMetadata"],
        "tags": [], "paletteMap": {}, "tilesets": [], "terrains": [],
        "symbols": symbols,
        "keyframes": keyframes,
        "layers": [ { "$id": g("layer"), "name": "art", "type": "IMAGE", "hidden": false, "locked": false, "keyframes": kf_ids, "pluginMetadata": {} } ],
        "animations": [ { "$id": g("anim"), "name": "active", "layers": [g("layer")], "pluginMetadata": {} } ]
    })
}

/// Per-segment `(fm_animation_name, fm_frame_length)` for a segmented element, in timeline order.
/// Lengths are FM (60fps) frames, so they match what the emitted keyframes actually play.
fn bg_segment_spans(layer: &BgLayerRef) -> Vec<(String, usize, usize, usize)> {
    let mut out = Vec::new();
    let mut off = 0usize;
    for (label, n, _) in &layer.segments {
        let end = (off + n).min(layer.frames.len());
        // the emitted keyframes are doubled 30->60fps, so the animation runs twice the source holds
        let fm_len: usize = layer.frames[off..end].iter().map(|f| f.hold.max(1) * 2).sum();
        out.push((sanitize_anim(label), off, end, fm_len));
        off = end;
    }
    out
}

/// The CUSTOM_GAME_OBJECT entity for a label-segmented backdrop element: one FM animation per
/// reachable segment, each carrying only that segment's frames. A VFX can hold exactly one
/// animation and no script, so an element whose SSF2 timeline BRANCHES between labels cannot be
/// one — it needs an object that can switch what it plays.
/// The one transparent picture every stage ships: what a frame that draws nothing points at.
/// A symbol with NO picture is not something the engine survives (it took the process down
/// with a segmentation fault the moment a camera-background piece of galaxy tours reached a
/// frame the source does not draw it on), so such a frame shows this instead.
fn blank_sprite_guid() -> String { det_uuid("stage::blank::1x1") }

fn write_blank_sprite(lib: &Path) -> Result<()> {
    let sprites = lib.join("sprites").join("Stage");
    std::fs::create_dir_all(&sprites).ok();
    let png = crate::stage_parser::blank_png_bytes();
    std::fs::write(sprites.join("_blank.png"), png)?;
    write_json(&sprites.join("_blank.png.meta"), &json!({
        "export": false, "guid": blank_sprite_guid(), "id": "", "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
    }))
}

fn bg_segment_entity(eid: &str, layer: &BgLayerRef, scale: f64) -> Value {
    let alpha = layer.plane.alpha();
    let (ox, oy) = bg_element_origin(layer);
    let g = |s: &str| det_uuid(&format!("bgseg::{eid}::{s}"));
    let symbols: Vec<Value> = layer.frames.iter().enumerate().map(|(j, f)| json!({
        "$id": g(&format!("sym{j}")), "type": "IMAGE", "imageAsset": if f.blank { blank_sprite_guid() } else { f.guid.clone() },
        "x": f.x - ox, "y": f.y - oy, "pivotX": 0.0, "pivotY": 0.0,
        "scaleX": scale, "scaleY": scale, "rotation": 0.0, "alpha": alpha, "pluginMetadata": {}
    })).collect();
    let mut keyframes: Vec<Value> = Vec::new();
    let mut layers: Vec<Value> = Vec::new();
    let mut animations: Vec<Value> = Vec::new();
    for (name, start, end, _) in bg_segment_spans(layer) {
        let mut kfs: Vec<Value> = (start..end).map(|j| json!({
            "$id": g(&format!("kf{j}")), "symbol": g(&format!("sym{j}")),
            "length": layer.frames[j].hold.max(1),
            "tweened": false, "tweenType": "LINEAR", "type": "IMAGE", "pluginMetadata": {}
        })).collect();
        crate::entity_gen::double_keyframe_lengths(&mut kfs);
        keyframes.extend(kfs);
        let lid = g(&format!("layer_{name}"));
        layers.push(json!({ "$id": lid, "name": format!("art_{name}"), "type": "IMAGE",
            "hidden": false, "locked": false,
            "keyframes": (start..end).map(|j| g(&format!("kf{j}"))).collect::<Vec<_>>(),
            "pluginMetadata": {} }));
        animations.push(json!({ "$id": g(&format!("anim_{name}")), "name": anim_key(&name),
            "layers": [lid], "pluginMetadata": {} }));
    }
    json!({
        "export": true, "guid": g("entity"), "id": eid, "version": 5,
        "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "objectType": "CUSTOM_GAME_OBJECT", "version": "0.4.0" } },
        "plugins": ["com.fraymakers.FraymakersTypes", "com.fraymakers.FraymakersMetadata"],
        "tags": [], "paletteMap": {}, "tilesets": [], "terrains": [],
        "symbols": symbols, "keyframes": keyframes, "layers": layers, "animations": animations
    })
}

/// The state machine that replays a segmented clip's own branching, so the element shows what SSF2
/// shows and in the same proportions.
///
/// Each segment's animation LOOPS on its own, so the script only has to act at the end of a lap:
/// staying in the same segment is doing nothing, and a branch is one `toLocalState`. That keeps the
/// common case seamless (no restart hitch on the segment that just repeats) and means the frame
/// counter is the only state that has to survive between frames.
/// Which section follows each of a sectioned element's sections: `Some(j)` plays section j next
/// (itself when it loops), `None` holds its last frame until the stage moves it on. None for an
/// element one of whose sections rolls for where it goes.
fn segment_chain(layer: &BgLayerRef) -> Option<Vec<Option<usize>>> {
    use crate::abc_parser::LabelJump;
    let spans = bg_segment_spans(layer);
    let idx_of = |lbl: &str| spans.iter().position(|(n, ..)| *n == sanitize_anim(lbl));
    (0..spans.len()).map(|i| {
        let fall = if i + 1 < spans.len() { i + 1 } else { 0 };
        match layer.segments.get(i).and_then(|(_, _, j)| j.clone()) {
            Some(LabelJump::Always(t)) | Some(LabelJump::Hold(t)) => Some(Some(idx_of(&t).unwrap_or(fall))),
            Some(LabelJump::Chance { .. }) => None,
            Some(LabelJump::Stop) => Some(None),
            Some(LabelJump::ToFrame(_)) | None => Some(Some(fall)),
        }
    }).collect()
}

/// The script of a sectioned element whose sections chain in its animation stats: it only sets
/// the element up, and the stage plays its sections on it.
fn bg_segment_native_script_hx(eid: &str, layer: &BgLayerRef) -> String {
    format!(
        "// {eid}: a backdrop element whose SSF2 clip branches between frame labels. One animation\n\
         // per label; the animation stats chain them, and the stage plays a label on it.\n\n\
         function initialize() {{\n\tself.setState(PState.ACTIVE);\n\
         \t// scenery can sit past the blast zone (a bird crossing the sky, a far hill); it stays\n\
         \tself.updateGameObjectStats({{ deathBoundsDestroy: false }});\n{}}}\n\n\
         function update() {{}}\n",
        if layer.starts_hidden { "\t// hidden by its own first frame until the stage plays a label on it\n\tself.setVisible(false);\n" } else { "" })
}

fn bg_segment_script_hx(eid: &str, layer: &BgLayerRef) -> String {
    use crate::abc_parser::LabelJump;
    let spans = bg_segment_spans(layer);
    let idx_of = |lbl: &str| spans.iter().position(|(n, ..)| *n == sanitize_anim(lbl));
    let lens: Vec<String> = spans.iter().map(|(_, _, _, l)| l.to_string()).collect();
    // one branch arm per segment, unrolled: which segment follows this one, and how often
    let mut arms = String::new();
    for (i, (_, _, _, _)) in spans.iter().enumerate() {
        let fall = if i + 1 < spans.len() { i + 1 } else { 0 };
        let body = match layer.segments.get(i).and_then(|(_, _, j)| j.clone()) {
            // a move that halts on the target's first frame lands the same way when that frame
            // stops itself, which is how a held section is written; otherwise it plays on from there
            Some(LabelJump::Always(t)) | Some(LabelJump::Hold(t)) => format!("return {};", idx_of(&t).unwrap_or(fall)),
            // the SSF2 frame script rolls the engine RNG here; a roll that misses simply lets the
            // timeline run on into the next segment, which is what the fallthrough is
            Some(LabelJump::Chance { pct, target }) => format!(
                "if (Random.getInt(0, 99) < {pct}) {{ return {}; }}\n\t\treturn {fall};",
                idx_of(&target).unwrap_or(fall)),
            // the playhead halts: the segment holds its last frame until the stage asks by name
            Some(LabelJump::Stop) => "return -1;".to_string(),
            Some(LabelJump::ToFrame(_)) | None => format!("return {fall};"),
        };
        arms.push_str(&format!("\tif (i == {i}) {{\n\t\t{body}\n\t}}\n"));
    }
    let labels: Vec<String> = spans.iter().map(|(n, ..)| format!("\"{n}\"")).collect();
    let states: Vec<String> = spans.iter()
        .map(|(n, ..)| format!("\t{}: _prepLocalState(\"{}\")", n.to_ascii_uppercase(), anim_key(n))).collect();
    format!(
        "// {eid}: a backdrop element whose SSF2 clip BRANCHES between frame labels rather than\n\
         // looping one timeline. One FM animation per label; this replays the branching.\n\n\
         function _prepLocalState(animation:String, ?index:Int=Math.NaN):Int {{\n\
         \tif (!__hasInitLocalStateMachine) {{ Common.initLocalStateMachine(); __hasInitLocalStateMachine = true; }}\n\
         \tif (index != Math.NaN) {{ index = __localStatePrepIndex++; }}\n\
         \tCommon.registerLocalState(index, animation);\n\treturn index;\n}}\n\
         var __hasInitLocalStateMachine = false;\nvar __localStatePrepIndex = -1;\n\
         var LState = {{\n\tUNINITIALIZED: _prepLocalState(\"#n/a\", -1),\n{}\n}};\n\n\
         // segment lengths in engine frames (the source's 30fps holds, doubled)\n\
         var LENS = [{}];\n\
         var STATES = [{}];\n\
         var LABELS = [{}];\n\
         // the labels as the source spells them, for a class that reads the clip's currentLabel\n\
         var RAW = [{}];\n\
         // persistent: a plain var re-initialises every frame on a custom game object.\n\
         var m_seg = self.makeInt(0);\nvar m_t = self.makeInt(0);\nvar m_init = self.makeBool(false);\n\n\
         // where a segment starts on the source's own frame count, and the clip's clock as the\n\
         // source's code reads it (currentFrame / currentLabel / totalFrames), left in the exports\n\
         function _segStart(i:Int):Int {{\n\tvar s = 0;\n\tfor (k in 0...i) {{ s += LENS[k]; }}\n\treturn Std.int(s / 2);\n}}\n\
         function _publish(seg:Int, t:Int) {{\n\tself.exports.__label = RAW[seg];\n\tself.exports.__frame = _segStart(seg) + Std.int(t / 2) + 1;\n}}\n\n\
         // which segment follows `i` — the clip's own frame scripts, unrolled. -1: the playhead\n\
         // stops there, and only the stage moves it on.\n\
         function _next(i:Int):Int {{\n{}\treturn 0;\n}}\n\n\
         function _seg(l:String):Int {{\n\tfor (i in 0...LABELS.length) {{ if (LABELS[i] == l) {{ return i; }} }}\n\treturn -1;\n}}\n\n\
         function initialize() {{\n\tself.setState(PState.ACTIVE);\n\
         \t// scenery can sit past the blast zone (a bird crossing the sky, a far hill); it stays\n\
         \tself.updateGameObjectStats({{ deathBoundsDestroy: false }});\n\
         \tself.exports.__total = _segStart(LENS.length);\n\t_publish(0, 0);\n\
         HIDDEN_INIT}}\n\n\
         function update() {{\n\
         \tif (!m_init.get()) {{ m_init.set(true); Common.toLocalState(STATES[0]); }}\n\
         \t// the stage plays this clip by label (the source's gotoAndPlay on the plane): the ask\n\
         \t// is left in the exports and taken up here, in the script that owns the state.\n\
         \tvar go = self.exports.__go;\n\
         \tif (go != null) {{\n\
         \t\tself.exports.__go = null;\n\
         \t\t// a clip hidden by its own first frame is shown by the label the stage plays on it\n\
         \t\tself.setVisible(true);\n\
         \t\t// by name, or by frame number (the source's gotoAndPlay(3)): the segment holding that frame\n\
         \t\tvar k = -1;\n\tvar t0 = 0;\n\
         \t\tvar gs = Std.string(go);\n\
         \t\tvar isNum = gs.length > 0;\n\
         \t\tfor (ci in 0...gs.length) {{ var kc = gs.charCodeAt(ci); if (kc < 48 || kc > 57) {{ isNum = false; }} }}\n\
         \t\tvar gn = 0;\n\
         \t\tif (isNum) {{ for (ci in 0...gs.length) {{ gn = gn * 10 + (gs.charCodeAt(ci) - 48); }} }}\n\
         \t\tif (isNum) {{\n\
         \t\t\tfor (i in 0...LENS.length) {{ if (gn > _segStart(i) && gn <= _segStart(i + 1)) {{ k = i; t0 = (gn - _segStart(i) - 1) * 2; }} }}\n\
         \t\t}} else {{ k = _seg(gs); }}\n\
         \t\tif (k >= 0) {{ m_seg.set(k); m_t.set(t0); Common.toLocalState(STATES[k]); _publish(k, t0); return; }}\n\
         \t}}\n\
         \tvar seg = m_seg.get();\n\
         \tvar t = m_t.get() + 1;\n\
         \tif (t >= LENS[seg]) {{\n\
         \t\tvar nxt = _next(seg);\n\
         \t\tif (nxt < 0) {{\n\
         \t\t\t// stopped: the animation holds its last frame, and the clock stays put\n\
         \t\t\tt = LENS[seg];\n\
         \t\t}} else {{\n\
         \t\t\tt = 0;\n\
         \t\t\t// staying put needs no call: the animation already loops itself, and restarting it\n\
         \t\t\t// would visibly re-seat the element every lap.\n\
         \t\t\tif (nxt != seg) {{ m_seg.set(nxt); Common.toLocalState(STATES[nxt]); }}\n\
         \t\t}}\n\
         \t}}\n\tm_t.set(t);\n\t_publish(m_seg.get(), t);\n}}\n",
        states.join(",\n"), lens.join(", "),
        spans.iter().map(|(n, ..)| format!("LState.{}", n.to_ascii_uppercase())).collect::<Vec<_>>().join(", "),
        labels.join(", "),
        layer.segments.iter().map(|(l, ..)| format!("\"{}\"", l.replace('\\', "\\\\").replace('"', "\\\""))).collect::<Vec<_>>().join(", "),
        arms).replace("HIDDEN_INIT", if layer.starts_hidden {
            "\t// hidden by its own first frame until the stage plays a label on it\n\tself.setVisible(false);\n"
        } else { "" })
}

/// Emit each promoted backdrop element as a looping VFX content (a `.entity` of objectType "VFX",
/// no scripts/stats/manifest entry — resolved by id via getContent, like the character port's
/// effects) and return the stage-Script spawn lines: createVfx (loop forever, absolute position)
/// then reparent its sprite into the stage background container for depth.



/// One source element as the layer(s) that represent it: normally one, but a mostly-still element
/// becomes a plate plus the part that moves (see `split_static_plate`).
fn build_element_layers(
    eid: &str,
    name: String,
    clip: String,
    named_path: &[String],
    frames: &[crate::stage_parser::StageArt],
    segments: &[crate::stage_parser::BgSegment],
    plane: BgPlane,
    pan: Option<(f64, f64)>,
    fades: bool,
    authored_alpha: f64,
    presence: &[(usize, usize)],
    starts_hidden: bool,
    write_layer: &mut impl FnMut(&str, &crate::stage_parser::StageArt) -> Result<ArtRef>,
) -> Result<Vec<BgLayerRef>> {
    let segs: Vec<(String, usize, Option<crate::abc_parser::LabelJump>)> =
        segments.iter().map(|s| (s.label.clone(), s.frames.len(), s.next.clone())).collect();
    if segs.is_empty() {
        if let Some((plate, moving)) = split_static_plate(frames) {
            let plate_id = format!("{eid}_plate");
            let plate_ref = write_layer(&plate_id, &plate)?;
            let moving_refs = write_element_frames(eid, &moving, false, write_layer)?;
            return Ok(vec![
                // the still part and what moves over it are one clip, kept apart so the plate is
                // rasterised once -- as LAYERS of that clip, not as two objects
                BgLayerRef { starts_hidden, presence: presence.to_vec(), named_path: named_path.iter().filter(|n| !n.starts_with('#')).cloned().collect(), clip_path: named_path.to_vec(), name: format!("{name} (plate)"), eid: plate_id, clip: clip.clone(),
                             frames: vec![plate_ref],
                             plane, segments: Vec::new(), pan, fades, authored_alpha },
                BgLayerRef { starts_hidden, presence: presence.to_vec(), named_path: named_path.iter().filter(|n| !n.starts_with('#')).cloned().collect(), clip_path: named_path.to_vec(), name, eid: eid.to_string(), clip, frames: moving_refs, plane, segments: Vec::new(), pan, fades, authored_alpha },
            ]);
        }
    }
    let refs = write_element_frames(eid, frames, !segs.is_empty(), write_layer)?;
    Ok(vec![BgLayerRef { starts_hidden, presence: presence.to_vec(), named_path: named_path.iter().filter(|n| !n.starts_with('#')).cloned().collect(), clip_path: named_path.to_vec(), name, eid: eid.to_string(), clip, frames: refs, plane, segments: segs, pan, fades, authored_alpha }])
}

/// Split a mostly-static animated element into a still PLATE plus a small moving piece.
///
/// A backdrop element is rasterised whole on every frame, so an element that is a large static
/// picture with one small thing moving in it costs the full canvas per frame. Clocktown's town
/// backdrop is 623 distinct 739x437 frames -- 160MB, and FrayTools could not publish it at all --
/// while only a ~259x258 patch of it ever changes.
///
/// So: frame 0 is emitted once as a full-canvas plate, and the animation is emitted cropped to the
/// region that actually moves, offset to sit back over the plate. Nothing is lost -- the crop is
/// the union of every frame's difference from the plate.
///
/// Returns None when the element is small, short, or genuinely changes across most of its canvas.
fn split_static_plate(frames: &[crate::stage_parser::StageArt])
    -> Option<(crate::stage_parser::StageArt, Vec<crate::stage_parser::StageArt>)>
{
    const MIN_FRAMES: usize = 8;
    const MAX_MOVING_FRACTION: f64 = 0.6;
    if frames.len() < MIN_FRAMES { return None; }
    let plate = &frames[0];
    let base = image::load_from_memory(&plate.png).ok()?.to_rgba8();
    let (cw, ch) = (base.width(), base.height());
    if cw * ch == 0 { return None; }

    // Sample first: decoding every frame to find out the answer is no is the expensive way to
    // learn it, and most elements that reach here are the whole-canvas kind (clocktown's town
    // backdrop differs from its own first frame across all of it by the end).
    let step = (frames.len() / 8).max(1);
    let (mut sx0, mut sy0, mut sx1, mut sy1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for f in frames.iter().step_by(step) {
        let im = image::load_from_memory(&f.png).ok()?.to_rgba8();
        if im.width() != cw || im.height() != ch { return None; }
        for y in 0..ch {
            for x in 0..cw {
                if im.get_pixel(x, y) != base.get_pixel(x, y) {
                    sx0 = sx0.min(x); sy0 = sy0.min(y); sx1 = sx1.max(x); sy1 = sy1.max(y);
                }
            }
        }
    }
    if sx0 == u32::MAX { return None; }
    let sampled = ((sx1 - sx0 + 1) as f64 * (sy1 - sy0 + 1) as f64) / (cw as f64 * ch as f64);
    if sampled > MAX_MOVING_FRACTION { return None; }

    // union of every frame's difference from the plate
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for f in &frames[1..] {
        let im = image::load_from_memory(&f.png).ok()?.to_rgba8();
        if im.width() != cw || im.height() != ch { return None; }   // frames crop differently already
        for y in 0..ch {
            for x in 0..cw {
                if im.get_pixel(x, y) != base.get_pixel(x, y) {
                    x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y);
                }
            }
        }
    }
    if x0 == u32::MAX { return None; }                              // nothing moves at all
    let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
    if (bw as f64 * bh as f64) / (cw as f64 * ch as f64) > MAX_MOVING_FRACTION { return None; }

    let encode = |img: &image::RgbaImage| -> Option<Vec<u8>> {
        use image::ImageEncoder;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8).ok()?;
        Some(png)
    };
    let mut moving = Vec::with_capacity(frames.len());
    for f in frames {
        let im = image::load_from_memory(&f.png).ok()?.to_rgba8();
        let crop = image::imageops::crop_imm(&im, x0, y0, bw, bh).to_image();
        // the crop is cut in the frame's own pixels, which a shipped-smaller frame draws scaled
        moving.push(crate::stage_parser::StageArt {
            png: encode(&crop)?,
            x: f.x + x0 as f64 * f.scale_x, y: f.y + y0 as f64 * f.scale_y,
            w: bw, h: bh, hold: f.hold, rotation: 0.0, scale_x: f.scale_x, scale_y: f.scale_y , alpha: 1.0 , pivot: (0.0, 0.0), tint: crate::stage_parser::TINT_ID });
    }
    log::info!("backdrop element: {}x{} canvas with a {bw}x{bh} moving region — emitting a still plate plus the crop",
               cw, ch);
    Some((plate.clone(), moving))
}

/// Write one backdrop element's frames, collapsing what the source repeats.
///
/// SSF2 authors a small looping element and the stage timeline it sits on runs far longer, so the
/// art walk sees the element's cycle over and over. Clocktown arrives with 1363 frames of bottom
/// fire drawn from 8 distinct images, and 621 frames of fire light drawn from 2 -- rasterised
/// frame by frame that stage emitted 263MB of sprites, enough that publishing it timed out.
///
/// Two reductions, both lossless:
///   - a sequence that is one cycle repeated is emitted as ONE cycle (the element loops anyway).
///     the tail does not have to be a whole cycle -- the source stops wherever its timeline ends.
///   - an image that appears on more than one frame is WRITTEN once and referenced again, which
///     covers a sequence that reuses a handful of images without repeating in a fixed period.
fn write_element_frames(
    eid: &str,
    frames: &[crate::stage_parser::StageArt],
    segmented: bool,
    mut write_layer: impl FnMut(&str, &crate::stage_parser::StageArt) -> Result<ArtRef>,
) -> Result<Vec<ArtRef>> {
    // Segmented elements keep every frame: their segment lengths index into this list.
    let src = if segmented { frames } else { &frames[..element_cycle_len(frames)] };
    if src.len() < frames.len() {
        log::info!("backdrop element '{eid}': {} frames are repeats of a {}-frame cycle; emitting one",
                   frames.len(), src.len());
    }
    if src.len() == 1 {
        return Ok(vec![write_layer(eid, &src[0])?]);
    }
    let mut out: Vec<ArtRef> = Vec::with_capacity(src.len());
    let mut seen: Vec<(usize, ArtRef)> = Vec::new();   // index of first use -> its ref
    let mut written = 0usize;
    for (j, a) in src.iter().enumerate() {
        // by the PICTURE alone: where it sits, how it is turned and how solid it is all ride on
        // the keyframe, so the same cel moved across the screen is one image, not one per step.
        // ...the same picture AND the same colour. Matching on the picture alone folds a piece the
        // source recolours over the day into one asset and keeps the first colour, which is the
        // whole day-to-night slide collapsed to whichever tint happened to come first.
        if let Some((_, prev)) = seen.iter().find(|(i, _)| src[*i].png == a.png
            && src[*i].tint.iter().zip(a.tint).all(|(x, y)| (x - y).abs() < 1e-4)) {
            // same picture as an earlier frame: reuse the asset, keep this frame's own hold
            out.push(ArtRef { hold: a.hold.max(1) as usize, rotation: a.rotation, scale_x: a.scale_x,
                              scale_y: a.scale_y, x: a.x, y: a.y, alpha: a.alpha, pivot: a.pivot,
                              tint: a.tint, w: a.w, h: a.h, ..prev.clone() });
            continue;
        }
        let r = write_layer(&format!("{eid}_{j}"), a)?;
        seen.push((j, r.clone()));
        out.push(r);
        written += 1;
    }
    if written < src.len() {
        log::info!("backdrop element '{eid}': {} frames drawn from {written} images", src.len());
    }
    Ok(out)
}

/// Two frames that draw the SAME thing -- picture, place, pose, opacity and colour.
///
/// Everything the source animates has to be in here. Comparing the picture and the place alone
/// calls a piece that is turning, fading, or being recoloured "unchanged", so the cycle finder
/// below reads a whole day as repeating on the first frame and truncates the element to it: the
/// day-to-night slide, the turn and the fade all deleted, leaving a still.
fn same_art(a: &crate::stage_parser::StageArt, b: &crate::stage_parser::StageArt) -> bool {
    a.png == b.png && a.x == b.x && a.y == b.y && a.w == b.w && a.h == b.h
        && a.rotation == b.rotation && a.scale_x == b.scale_x && a.scale_y == b.scale_y
        && a.alpha == b.alpha
        && a.tint.iter().zip(b.tint).all(|(x, y)| (x - y).abs() < 1e-4)
}

/// The smallest prefix that reproduces the whole sequence by repeating, or its full length.
fn element_cycle_len(frames: &[crate::stage_parser::StageArt]) -> usize {
    let n = frames.len();
    if n < 2 { return n; }
    (1..=n / 2)
        .find(|&p| (p..n).all(|i| same_art(&frames[i], &frames[i % p]) && frames[i].hold == frames[i % p].hold))
        .unwrap_or(n)
}








/// A piece's width in this stage's px: the span of its frames' art.
fn piece_width(layer: &BgLayerRef) -> f64 {
    let (ax, _) = bg_element_origin(layer);
    let right = layer.frames.iter().map(|f| f.x + f.w as f64).fold(f64::MIN, f64::max);
    if right.is_finite() { (right - ax).max(0.0) } else { 0.0 }
}

/// Is a piece INSIDE the placement named `n`: the name sits in its path with a name below it
/// (`movingplatL ▸ sprite`), rather than being the piece's own name in some plane.

fn rides_under(named_path: &[String], n: &str) -> bool {
    named_path.iter().position(|p| p == n).is_some_and(|i| i + 1 < named_path.len())
}

/// A piece belongs to its clip's group and to the group of EVERY instance name over it: the
/// stage's code reaches the same piece as `movingplatLsprite` and as `movingplatLsprite.sprite`,
/// and two platforms' pictures both called `sprite` inside are told apart by the name outside.
fn bind_part(clip_parts: &mut Vec<(String, Vec<(String, Vec<String>)>)>, handle: &str, named_path: &[String], part: (String, Vec<String>)) {
    let mut names: Vec<String> = vec![handle.to_string()];
    for n in named_path {
        if let Some(h) = clip_handle(n) {
            if !names.contains(&h) { names.push(h); }
        }
    }
    for h in names {
        match clip_parts.iter_mut().find(|(c, _)| *c == h) {
            Some((_, v)) => v.push(part.clone()),
            None => clip_parts.push((h, vec![part.clone()])),
        }
    }
}

// A CLIP NAMED LIKE ONE OF THE STAGE'S OWN FIELDS. The source keeps the two apart (a field on
// the stage, a child in a plane); this script has one scope, and pokemon colosseum's helper
// object `stadiumScreen` overwrote the clip group of the same name, so the helper asked itself
// for an animation. A clashing clip's handle gets a name of its own (`<name>__clip`) wherever a
// handle is made or reached; the string keys the code asks by stay the source's.
thread_local! {
    static CLASHING_HANDLES: std::cell::RefCell<std::collections::BTreeSet<String>> = std::cell::RefCell::new(Default::default());
}
const CLIP_ALIAS: &str = "__clip";
fn set_clashing_handles(model: &StageModel) {
    let fields = crate::stage_text::StageText::of(model).fields();
    CLASHING_HANDLES.with(|c| *c.borrow_mut() = fields);
}
fn handle_alias(name: &str) -> String {
    if CLASHING_HANDLES.with(|c| c.borrow().contains(name)) { format!("{name}{CLIP_ALIAS}") } else { name.to_string() }
}
/// the source's name behind a handle (an alias stripped)
fn handle_source_name(handle: &str) -> &str {
    handle.strip_suffix(CLIP_ALIAS).unwrap_or(handle)
}

fn clip_handle(clip: &str) -> Option<String> {
    let ok = !clip.is_empty()
        && clip.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && clip.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then(|| handle_alias(clip))
}

/// The day is ONE clock, so a phase boundary is one moment -- but each element's loop is detected
/// from its own cels, and two elements whose art settles a frame or two apart report the change a
/// frame or two apart. Emitted as they come, the pieces of one backdrop turn over at slightly
/// different times: clocktown's clock face changed its colours ahead of the town behind it.
///
/// Boundaries within a few frames of each other are the SAME boundary. Each cluster collapses to
/// the value most elements reported (ties to the earliest), so the majority keeps exactly what it
/// detected and only the stragglers move.
/// The day is ONE clock, so a phase boundary is one moment -- but each element's loop is detected
/// from its own cels, and two elements whose art settles a frame or two apart report the change a
/// frame or two apart. Emitted as they come, the pieces of one backdrop turn over at slightly
/// different times: clocktown's clock face changed its colours ahead of the town behind it.
///
/// Boundaries within a few frames of each other are the SAME boundary. Each cluster collapses to
/// the value most elements reported (ties to the earliest), so the majority keeps exactly what it
/// detected and only the stragglers move.


/// Every animated backdrop element, as the boot tables list it (`tables.elements`, the tint
/// tables and the clip groups). Returns the lines that give a group its children by the source's
/// names (a field set by name is the script's own to write), the manifest entries, and each
/// clip handle's pieces.
fn emit_bg_elements(model: &StageModel, promoted: &[BgLayerRef], lib: &Path, plan: &mut Vec<Value>, tables: &mut StageTables,
    carriers: &BTreeMap<String, usize>, carried: &BTreeMap<String, SurfaceSegs>)
    -> Result<Vec<Value>> {
    write_blank_sprite(lib)?;
    let mut entries: Vec<Value> = Vec::new();
    // the named surfaces whose collision already rides a picture (one picture each)
    let mut followed: std::collections::BTreeSet<String> = Default::default();
    // ONE VFX PER MOVIECLIP, which is what the source has: a nested clip carries its own
    // timeline and loops on its own period, and the engine gives that for free to a VFX with
    // `loop: true`. Merging a clip's children into layers of one animation puts them all on one
    // clock -- a 28-frame rain splash then shows once in a 7440-frame cycle instead of the two
    // hundred and sixty times SSF2 plays it.
    //
    // The named clip is still one thing, but at the HANDLE: see `clip_groups` below, where the
    // name its plane knows it by is bound to all the objects that came out of it.
    // each piece, and what it answers to by name -- a group passes a label down only to the
    // pieces that carry it
    let mut clip_parts: Vec<(String, Vec<(String, Vec<String>)>)> = Vec::new();
    // each piece's width in this stage's px: what a flip of the group turns the piece about
    let mut part_w: std::collections::BTreeMap<String, f64> = Default::default();
    // ...and the plane each piece draws in: what a clip the code puts INTO the group is put in
    let mut part_plane: std::collections::BTreeMap<String, &'static str> = Default::default();
    // One table per DISTINCT colour schedule. A tint is authored on a PARENT clip, so every child
    // that came out of it slides through the same hundred-odd colours; writing that table per
    // object repeats one schedule a hundred and eighty times, which would be most of the script.
    let mut tint_tables: std::collections::HashMap<String, usize> = Default::default();
    let longest_element = promoted.iter().map(|l| element_len(&l.frames)).max().unwrap_or(0);
    for (i, layer) in promoted.iter().enumerate() {
        let handle = clip_handle(&layer.clip);
        let eid = format!("{}_{}", model.id, layer.eid);
        // an element whose clip branches between labels can't be a VFX: a VFX holds one animation
        // and carries no script, so there is nothing to switch and nothing to switch it.
        if layer.segments.len() > 1 {
            // the piece that carries a surface is that surface's structures, its art on the first
            match layer.named_path.last().filter(|n| carriers.get(*n) == Some(&i)).and_then(|n| carried.get(n).map(|g| (n, g))) {
                Some((n, segs)) => emit_bg_carrier(model, layer, &eid, i, lib, tables, &mut entries, plan, n, segs)?,
                // a piece in a camera background is effects in that background's container
                None if matches!(layer.plane, BgPlane::CameraEntry(_)) && segment_chain(layer).is_some() =>
                    emit_bg_sectioned_vfx(layer, &eid, i, lib, tables, plan)?,
                None => emit_bg_segmented(model, layer, &eid, i, lib, tables, &mut entries, plan, &mut followed)?,
            }
            // it belongs to its clip's group like any other piece: the group is how the ported
            // class reaches it, and this is the piece that actually carries the clip's labels
            if let Some(h) = &handle {
                let anims: Vec<String> = bg_segment_spans(layer).into_iter().map(|(n, ..)| n).collect();
                part_w.insert(format!("_bg{i}"), piece_width(layer));
                part_plane.insert(format!("_bg{i}"), layer.plane.source_plane());
                bind_part(&mut clip_parts, h, &layer.named_path, (format!("_bg{i}"), anims));
            }
            continue;
        }
        write_json(&lib.join("entities").join(format!("{eid}.entity")), &bg_element_entity(&eid, layer, model.scale))?;
        write_meta(&lib.join("entities").join(format!("{eid}.entity.meta")), &eid, &eid, "", Some("VFX"), None)?;
        // createVfx loops the element forever at its baked-in position (relativeWith:false ->
        // absolute), then reparent its sprite into the background container so it draws in front
        // of the static background art and behind the fighters. no owner / no VfxLayer: the
        // container reparent is the authoritative depth control.
        let (ox, oy) = bg_element_origin(layer);
        // at the opacity the SOURCE authored for its plane. A promoted element used to spawn fully
        // opaque whatever the source said, so clocktown's lighting sheet -- a screen-sized overlay
        // its own placements set to zero -- was drawn over the whole stage at full strength.
        // on the reparented CONTAINER, which is what actually draws: a VFX's own setAlpha does not
        // reach the view once it has been moved into a stage container.
        // The element's OWN opacity, not its plane's. The baked path halves a foreground SHEET
        // because that is what the sheet is drawn at; an element pulled out of that plane carries
        // its own alpha per frame and halving it again makes the port dimmer than the source.
        // Measured on clocktown's firelight: the source draws it at full alpha on the frames it
        // lights it (7% of the day), so a half applied on top renders it at half the source's.
        let a = layer.authored_alpha;
        // The source CROSS-FADES: a day copy of the backdrop fades out while a night copy fades in,
        // a step per frame across hundreds of frames. Baked to one number, a piece cannot fade at
        // all -- it holds its painted colours and only changes when a CEL changes, which is a hard
        // step, and pieces that swap cels at different frames turn over at different times. That is
        // what made clocktown's clock face turn ahead of the sky behind it.
        // ONE builder for both kinds of layer, so an element cannot get a fade schedule here that
        // the parallax path would have rejected. It tells a fade from mere presence; see
        // `alpha_keyframes`.
        // An element that LOOPS does not carry a day-length opacity schedule. Its animation is its
        // own short cycle -- clocktown's rain splash is 14 frames, its torch embers 23, read live
        // off the source -- while a schedule built over the whole sampled window is on the stage's
        // clock. The element then plays its lap while the schedule walks the day, and the two are
        // only ever briefly in agreement. The source does not do this either: the splash has no
        // schedule of its own, and what makes it darker at night is the lighting sheet over it.
        // A backdrop ELEMENT carries no opacity schedule. Its animation is its own loop -- the rain
        // splash 14 frames, the torch embers 23, read live off the source -- while a schedule built
        // over the sampled window runs on the stage's day-long clock, so the element plays its lap
        // while the schedule walks the day and the two agree only in passing. That is what left the
        // splashes never finishing and every one of them gated by the same clock.
        //
        // The source does not give them one either. What darkens them at night is the lighting
        // sheet drawn over the whole backdrop, and the day/night cross-fade lives on the camera
        // backgrounds, which keep their schedules (see the `__cbg` fades).
        let alpha = (a < 1.0).then_some(a);
        // A PIECE THE STAGE ONLY SHOWS AT ITS OWN MOMENT starts hidden, because its clip's own
        // first frame says so. Crateria's acid warning sign is one: drawn from the start here, it
        // stood over the stage for the whole match while the source only lights it as the acid
        // comes. The stage's own code turns it on (`setVisible(true)`) when the moment arrives.
        let hidden = {
            // the element carries the clip's name with the walk's own spelling, so both sides are
            // read down to their letters before they are compared
            let plain = |t: &str| t.chars().filter(|c| c.is_ascii_alphanumeric())
                .flat_map(|c| c.to_lowercase()).collect::<String>();
            let base = plain(layer.name.split(|c: char| c.is_ascii_digit()).next().unwrap_or(""));
            !base.is_empty() && model.hidden_clips.iter().any(|h| plain(h) == base)
        };
        // bind it to the name its plane knows it by, so the ported class has an object to reach
        if let Some(h) = &handle {
            // an unsegmented piece plays one thing, under the one name it was given
            part_w.insert(format!("_bg{i}"), piece_width(layer));
            part_plane.insert(format!("_bg{i}"), layer.plane.source_plane());
            bind_part(&mut clip_parts, h, &layer.named_path, (format!("_bg{i}"), vec!["active".to_string()]));
        }
        // WHEN the stage has it on screen. The object loops its own frames forever inside its
        // container; the container is what the stage's timeline puts up and takes away, which is
        // exactly what the source does with the clip.
        if std::env::var("PEPTIDE_DBG_P").is_ok() {
            eprintln!("EMIT {} runs={}", layer.eid, layer.presence.len());
        }
        // The frame's COLOUR, driven live. SSF2 slides a whole stage from day to night with a
        // colour transform on the placement, and the art is baked, so without this the piece holds
        // its painted colours and only changes when a CEL does -- a hard step where the source
        // slides. The engine's colour shader takes the same per-channel multiply and offset the
        // source stores, so the transform is carried rather than approximated, and rgb only: alpha
        // is already on the keyframe and applying it twice would double-darken every fade.
        let tint = {
            let mut keys: Vec<(usize, [f64; 6])> = Vec::new();
            let (mut at, mut prev) = (0usize, None::<[f64; 6]>);
            for f in &layer.frames {
                // A frame the source does not DRAW carries no colour of its own, and reading one
                // off it snaps the shader back to neutral for that frame -- so a piece that blinks
                // through its own animation writes a schedule that flips between its real colour
                // and none, dozens of times a second, instead of the slide the source authored.
                // The blank is invisible anyway: hold the last colour across it.
                if f.w <= 1 && f.h <= 1 {
                    at += f.hold.max(1) * crate::entity_gen::FRAME_RATIO as usize;
                    continue;
                }
                if prev.is_none_or(|p| p.iter().zip(f.tint).any(|(a, b)| (a - b).abs() > 1e-4)) {
                    keys.push((at, f.tint));
                    prev = Some(f.tint);
                }
                // IN THE EMITTED CLOCK, the same as the opacity schedule and its own length: the
                // source runs at 30fps and every emitted keyframe is doubled onto the engine's 60,
                // so a colour written in SOURCE frames lands on half the frame it belongs to. The
                // schedule then runs the day twice over -- through every colour by midday, then
                // pinned on the last one -- while the art it is colouring goes round once. That is
                // the clock face wearing a colour the rest of the backdrop left hours ago.
                at += f.hold.max(1) * crate::entity_gen::FRAME_RATIO as usize;
            }
            // a piece the source never recolours needs no shader at all
            let ever = layer.frames.iter().any(|f|
                f.tint.iter().zip(crate::stage_parser::TINT_ID).any(|(a, b)| (a - b).abs() > 1e-4));
            if !ever || keys.is_empty() { None } else {
                // one table per distinct schedule, told apart at the precision it is written at
                let body = keys.iter().map(|(at, t)| format!("[{at}, {:.4}, {:.4}, {:.4}, {:.1}, {:.1}, {:.1}]",
                    t[0], t[1], t[2], t[3], t[4], t[5])).collect::<Vec<_>>().join(", ");
                let n = tint_tables.len();
                let id = *tint_tables.entry(body).or_insert_with(|| { tables.tints.push(keys); n });
                Some((id, element_len(&layer.frames)))
            }
        };
        // A PIECE PLACED INSIDE AN OBJECT'S OWN CLIP IS THAT OBJECT'S OWN PICTURE (a platform's
        // holder, drawn at the frame the platform's class picks by its height): it rides the
        // object, and the object reaches it as its own view (`getMC().sprite.gotoAndStop(n)` ->
        // `__ownArt`). a clip that merely SHARES the object's name in another plane (planet
        // namek's cliff art, which its class shows and plays by label, never moves) is not inside
        // it: the object's name has to have a name below it in the path
        let rider = model.hazards.iter().find(|hz| hz.instance_name.as_deref()
            .is_some_and(|n| rides_under(&layer.named_path, n)));
        // the object by its own handle: two placements can share a name (flat zone's platforms
        // are both placed as `ground`), the handle is one each
        let ride = rider.map(|hz| (model.hazards.iter().position(|h| std::ptr::eq(h, hz)).unwrap_or(0), ox - hz.x, oy - hz.y, still_picture(layer)));
        // and which source clip it is, and whether it keeps the stage's whole timeline under a
        // schedule (a clip the stage puts down over and over) rather than looping on its own:
        // what a checker pairs it to its source by, and how it reads the length it finds
        // and whether the stage's timeline MOVES it: a placement whose transform changes from
        // frame to frame is the source's own motion of a clip that loops inside, so its length is
        // the stage's, whatever the clip inside is
        // an element recoloured over the stage's day keeps the stage's whole timeline in its
        // frames (its own loop never repeats under a changing tint), which is a schedule too
        let tinted = layer.frames.windows(2).any(|w| w[0].tint != w[1].tint)
            // ...or whose frames never came round within the stage's whole span (its pictures
            // change with the day): the longest element of all keeps that timeline
            || (element_len(&layer.frames) >= 600 && element_len(&layer.frames) == longest_element);
        let moving = layer.frames.windows(2).any(|w| w[0].x != w[1].x || w[0].y != w[1].y
            || w[0].rotation != w[1].rotation || w[0].scale_x != w[1].scale_x || w[0].scale_y != w[1].scale_y);
        plan.push(json!({ "kind": "vfx", "id": eid, "var": format!("_bg{i}"), "x": ox, "y": oy,
                          "container": layer.plane.container(), "frames": element_len(&layer.frames),
                          "clip": layer.clip, "names": layer.named_path, "rides": rider.and_then(|hz| hz.instance_name.clone()),
                          "scheduled": !layer.presence.is_empty() || tinted, "moving": moving }));
        tables.elements.push(BootElement {
            index: i, id: eid, segmented: false, x: ox, y: oy,
            at: match layer.pan {
                // a piece of the camera background rides the pan container for its rate
                Some((px, py)) => BootAt::Pan(px, py),
                None => BootAt::Plane(layer.plane.boot_plane()),
            },
            shown: layer.presence.clone(), tint, alpha, hidden, ride,
            follow: plat_follow(model, layer, &mut followed),
            path: layer.clip_path.clone(), labels: vec!["active".to_string()], sections: None, carry: None, chain: None,
        });
    }
    // The named clip, bound to everything that came out of it. One clip is one thing to the stage
    // class -- it says `clocktown_rainsplash_fg.setVisible(false)` -- while remaining the several
    // independently-looping objects the source actually has.
    for (clip, parts) in &clip_parts {
        // a name the tree also placed an OBJECT under (saffron city's platforms carry their own
        // class) is that object, whose mailbox takes the asks; the group is its art. a bare
        // region under that name (a target test's counters have boxes) is not an object
        let widths = parts.iter().map(|(n, _)| part_w.get(n).copied().unwrap_or(0.0)).collect();
        let plane = parts.first().and_then(|(n, _)| part_plane.get(n).copied()).unwrap_or("background");
        // the group goes by the SOURCE's name for the clip (an alias is the script's), which is
        // the name the tree recorded its box under: saffron city's platform art answered its
        // class's moves from 0 while its box sat under the clip's own name
        tables.groups.push(BootGroup {
            name: clip.clone(), source: handle_source_name(clip).to_string(),
            parts: parts.iter().map(|(n, ls)| (piece_index(n), ls.clone())).collect(),
            widths, plane: plane.to_string(),
        });
    }
    Ok(entries)
}

/// The element index a piece's handle names (`_bg3` is element 3).
fn piece_index(var: &str) -> usize {
    var.strip_prefix("_bg").and_then(|n| n.parse().ok()).expect("a piece is named by its element index")
}

/// One segmented backdrop element: a CUSTOM_GAME_OBJECT (entity + state-machine Script + stats),
/// spawned and reparented into the SAME layer container the VFX form uses, so switching vehicles
/// doesn't change where it draws.
#[allow(clippy::too_many_arguments)]
/// A SURFACE WHOSE PLANE MOVES IT BY LABEL rides its own picture. The source shows one terrain
/// section per phase and slides the next one in on its timeline; the port's collision is built once
/// at the boot placement, so the section's structures are moved every frame by however far the
/// timeline has moved its picture from that placement (mushroom kingdom 3's castle floor rises 500 px
/// into play at its label). The offsets are the picture's own per-source-frame placements, so they
/// follow the tween the source drew, not just its ends.
/// Whether a piece draws one picture, in one place, the same way on every frame: art that never
/// changes, which needs nothing to play it.
fn still_picture(layer: &BgLayerRef) -> bool { still_frames(&layer.frames) }

fn still_frames(frames: &[ArtRef]) -> bool {
    let Some(f0) = frames.first() else { return false };
    frames.iter().all(|f| !f.blank && f.guid == f0.guid && f.x == f0.x && f.y == f0.y && f.w == f0.w && f.h == f0.h
        && f.rotation == f0.rotation && f.scale_x == f0.scale_x && f.scale_y == f0.scale_y
        && f.alpha == f0.alpha && f.tint == f0.tint)
}

fn plat_follow(model: &StageModel, layer: &BgLayerRef, done: &mut std::collections::BTreeSet<String>) -> Option<BootFollow> {
    let name = layer.named_path.last()?;
    if done.contains(name) { return None }
    if !model.platforms.iter().any(|p| p.name.as_deref() == Some(name.as_str()) && !p.placements.is_empty()) { return None }
    let f0 = layer.frames.first()?;
    done.insert(name.clone());
    // one entry per label the piece plays as an animation, its offsets one per SOURCE frame (a
    // held picture repeated for its hold), which is what the piece's own frame counter counts
    let expand = |frames: &[ArtRef]| -> Vec<(f64, f64)> {
        frames.iter().flat_map(|f| std::iter::repeat_n((f.x - f0.x, f.y - f0.y), f.hold.max(1))).collect()
    };
    let mut labels: Vec<(String, Vec<(f64, f64)>)> = Vec::new();
    if layer.segments.is_empty() {
        labels.push(("active".to_string(), expand(&layer.frames)));
    } else {
        let mut off = 0usize;
        for (label, n, _) in &layer.segments {
            let end = (off + n).min(layer.frames.len());
            labels.push((label.replace('"', ""), expand(&layer.frames[off..end])));
            off = end;
        }
    }
    Some(BootFollow { name: name.clone(), labels })
}

/// A sectioned piece in a camera background, as effects in that background's container (the
/// engine places and pans it with the background): one animation per section, and a one-frame
/// `_hold` of its last frame for a section the source stops on. The stage plays a section as an
/// effect and moves on to the next on an engine timer at the section's end.
fn emit_bg_sectioned_vfx(layer: &BgLayerRef, eid: &str, i: usize, lib: &Path,
    tables: &mut StageTables, plan: &mut Vec<Value>) -> Result<()> {
    let spans = bg_segment_spans(layer);
    let chain = segment_chain(layer).unwrap_or_default();
    let origin = bg_element_origin(layer);
    let mut phases: Vec<(String, Vec<ArtRef>)> = Vec::new();
    for (a, (n, start, end, _)) in spans.iter().enumerate() {
        phases.push((anim_key(n), layer.frames[*start..*end].to_vec()));
        if chain.get(a).copied().flatten().is_none() && end > start {
            let mut last = layer.frames[end - 1].clone();
            last.hold = 1;
            phases.push((format!("{}_hold", anim_key(n)), vec![last]));
        }
    }
    write_json(&lib.join("entities").join(format!("{eid}.entity")), &bg_element_entity_phased(eid, &phases, 1.0, origin))?;
    write_meta(&lib.join("entities").join(format!("{eid}.entity.meta")), eid, eid, "", Some("VFX"), None)?;
    let (ox, oy) = origin;
    plan.push(json!({ "kind": "object", "id": eid, "var": format!("_bg{i}"), "x": ox, "y": oy,
                      "clip": layer.clip, "names": layer.named_path,
                      "phases": spans.iter().map(|(n, ..)| n.clone()).collect::<Vec<_>>(),
                      "phase_lens": spans.iter().map(|(.., l)| *l).collect::<Vec<_>>() }));
    tables.elements.push(BootElement {
        index: i, id: eid.to_string(), segmented: true, x: ox, y: oy,
        at: match layer.pan { Some((px, py)) => BootAt::Pan(px, py), None => layer.plane.boot_at() },
        shown: Vec::new(), tint: None, alpha: None, hidden: layer.starts_hidden, ride: None, follow: None,
        path: layer.clip_path.clone(), labels: spans.iter().map(|(n, ..)| n.clone()).collect(),
        sections: Some((spans.iter().map(|(n, ..)| anim_key(n)).collect(), spans.iter().map(|(.., l)| *l).collect())),
        carry: None,
        chain: Some(chain.iter().map(|n| n.map(|j| j as i64).unwrap_or(-1)).collect()),
    });
    Ok(())
}

/// The named surfaces sectioned pieces carry: each by the first sectioned piece placed under its
/// name whose sections chain the same way every time (its index among the promoted pieces).
fn surface_carriers(model: &StageModel, promoted: &[BgLayerRef]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for (i, l) in promoted.iter().enumerate() {
        if l.segments.len() <= 1 || segment_chain(l).is_none() { continue }
        let Some(n) = l.named_path.last() else { continue };
        if out.contains_key(n) { continue }
        if !model.platforms.iter().any(|p| p.name.as_deref() == Some(n.as_str()) && !p.placements.is_empty()) { continue }
        out.insert(n.clone(), i);
    }
    out
}

/// A sectioned piece that carries a named surface on its own frames, as the surface's structures:
/// one per segment (an engine structure holds one). An engine structure collides by the animation
/// it was made with, so each has ONE animation holding every section in turn, a label at each
/// section's start: the stage plays a section by its label, and the structure's own script holds
/// it or moves on at the section's end. Each segment is keyed to the piece's frames where it stands
/// in the piece, moved as far as the piece's picture has moved from its first frame; the first
/// structure carries the piece's art.
#[allow(clippy::too_many_arguments)]
fn emit_bg_carrier(
    model: &StageModel, layer: &BgLayerRef, eid: &str, i: usize, lib: &Path,
    tables: &mut StageTables, entries: &mut Vec<Value>, plan: &mut Vec<Value>,
    surface: &str, segs: &SurfaceSegs,
) -> Result<()> {
    let scripts = lib.join("scripts").join("entities");
    std::fs::create_dir_all(&scripts)?;
    let scale = if matches!(layer.plane, BgPlane::CameraEntry(_)) { 1.0 } else { model.scale };
    let alpha = layer.plane.alpha();
    let (ox, oy) = bg_element_origin(layer);
    let spans = bg_segment_spans(layer);
    let chain = segment_chain(layer).unwrap_or_default();
    let g = |s: &str| det_uuid(&format!("bgcarry::{eid}::{s}"));
    let (mut symbols, mut keyframes, mut layers, mut animations) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let frames = &layer.frames[..spans.last().map(|(_, _, e, _)| *e).unwrap_or(0)];
    let f0 = &layer.frames[0];
    // a layer of one keyframe per source frame, its length the frame's hold on the engine's clock
    let keyed = |tag: &str, ty: &str, sym: &dyn Fn(&ArtRef) -> Value, keyframes: &mut Vec<Value>, symbols: &mut Vec<Value>| -> Vec<Value> {
        let mut kfs: Vec<Value> = frames.iter().enumerate().map(|(j, f)| {
            let sid = g(&format!("sym_{tag}_{j}"));
            let mut sv = sym(f);
            sv["$id"] = json!(sid);
            symbols.push(sv);
            json!({ "$id": g(&format!("kf_{tag}_{j}")), "symbol": sid, "length": f.hold.max(1),
                "tweened": false, "tweenType": "LINEAR", "type": ty, "pluginMetadata": {} })
        }).collect();
        crate::entity_gen::double_keyframe_lengths(&mut kfs);
        let ids = kfs.iter().map(|k| k["$id"].clone()).collect();
        keyframes.extend(kfs);
        ids
    };
    // the sections' labels, where each starts
    let label_ids: Vec<Value> = spans.iter().map(|(n, _, _, len)| {
        let id = g(&format!("label_{n}"));
        keyframes.push(json!({ "$id": id, "type": "LABEL", "length": len, "name": anim_key(n), "pluginMetadata": {} }));
        json!(id)
    }).collect();
    let art = keyed("art", "IMAGE", &|f: &ArtRef| json!({ "type": "IMAGE",
        "imageAsset": if f.blank { blank_sprite_guid() } else { f.guid.clone() },
        "x": f.x - ox, "y": f.y - oy, "pivotX": 0.0, "pivotY": 0.0,
        "scaleX": scale, "scaleY": scale, "rotation": 0.0, "alpha": alpha, "pluginMetadata": {} }), &mut keyframes, &mut symbols);
    let mut ids = Vec::new();
    for (k, (name, pts, pm)) in segs.iter().enumerate() {
        let seg = keyed(&format!("seg{k}"), "LINE_SEGMENT", &|f: &ArtRef| {
            let (dx, dy) = (f.x - f0.x, f.y - f0.y);
            json!({ "type": "LINE_SEGMENT", "alpha": 0.5, "color": "0xeeeeee",
                "points": pts.iter().flat_map(|&(x, y)| [x - ox + dx, y - oy + dy]).collect::<Vec<f64>>(),
                "pluginMetadata": { "com.fraymakers.FraymakersMetadata": pm } })
        }, &mut keyframes, &mut symbols);
        let mut own = Vec::new();
        if k == 0 {
            let lid = g("layer_art");
            layers.push(json!({ "$id": lid, "name": "art", "type": "IMAGE", "hidden": false, "locked": false, "keyframes": art.clone(), "pluginMetadata": {} }));
            own.push(lid);
        }
        let sl = g(&format!("layer_seg{k}"));
        layers.push(json!({ "$id": sl, "name": name, "type": "LINE_SEGMENT", "hidden": false, "locked": false, "keyframes": seg,
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "lineSegmentType": "LINE_SEGMENT_STRUCTURE" } } }));
        own.push(sl);
        let ll = g(&format!("layer_labels{k}"));
        layers.push(json!({ "$id": ll, "name": "Labels", "type": "LABEL", "hidden": false, "locked": false, "keyframes": label_ids.clone(), "pluginMetadata": {} }));
        own.push(ll);
        let anim = format!("carry{k}");
        animations.push(json!({ "$id": g(&format!("anim{k}")), "name": anim, "layers": own, "pluginMetadata": {} }));
        let cid = if k == 0 { eid.to_string() } else { format!("{eid}_s{k}") };
        // where each section starts and ends on the engine's frames, and which follows it
        let (mut starts, mut ends, mut at) = (Vec::new(), Vec::new(), 1usize);
        for (.., len) in &spans { starts.push(at); ends.push(at + len - 1); at += len; }
        let next: Vec<String> = (0..spans.len()).map(|a| match chain.get(a).copied().flatten() {
            Some(j) => j.to_string(), None => "-1".into() }).collect();
        let list = |v: &[usize]| v.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
        let hide = if k == 0 && layer.starts_hidden { "\t// hidden by its own first frame until the stage plays a label on it\n\tself.setVisible(false);\n" } else { "" };
        let files = [
            ("Stats", format!("// {cid}: a structure the backdrop piece {eid} carries.\n{{\n\tspriteContent: self.getResource().getContent(\"{eid}\"),\n\tanimationId: \"{anim}\",\n\tstartX: {ox},\n\tstartY: {oy}\n}}\n"), None),
            ("Script", format!("// {cid}: a structure the backdrop piece {eid} carries (the surface `{surface}`). Its one\n\
                // animation holds every section in turn; the stage plays a section by its label, and at a\n\
                // section's end this holds it or moves on, as the source's clip does.\n\n\
                var STARTS = [{}];\nvar ENDS = [{}];\nvar NEXT = [{}];\n\n\
                function initialize() {{\n{hide}}}\n\n\
                function update() {{\n\tvar f = self.getCurrentFrame();\n\
                \tfor (i in 0...ENDS.length) if (f == ENDS[i]) {{\n\
                \t\tif (NEXT[i] < 0) self.pause();\n\
                \t\telse if (NEXT[i] != i + 1) self.playFrame(STARTS[NEXT[i]]);\n\
                \t\treturn;\n\t}}\n}}\n\n\
                function onTeardown() {{}}\n", list(&starts), list(&ends), next.join(", ")), Some("LINE_SEGMENT_STRUCTURE")),
        ];
        for (kind, body, ot) in files {
            let fname = format!("{cid}{kind}");
            write_script(&scripts.join(format!("{fname}.hx")), &body)?;
            write_meta(&scripts.join(format!("{fname}.hx.meta")), eid, &fname, "hscript", ot, None)?;
        }
        entries.push(json!({ "id": cid, "type": "structure", "objectStatsId": format!("{cid}Stats"), "scriptId": format!("{cid}Script") }));
        plan.push(json!({ "kind": "structure", "id": cid, "x": ox, "y": oy, "clip": layer.clip, "names": layer.named_path }));
        ids.push(cid);
    }
    let entity = json!({
        "export": true, "guid": g("entity"), "id": eid, "version": 5,
        "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "objectType": "CUSTOM_GAME_OBJECT", "version": "0.4.0" } },
        "plugins": ["com.fraymakers.FraymakersTypes", "com.fraymakers.FraymakersMetadata"],
        "tags": [], "paletteMap": {}, "tilesets": [], "terrains": [],
        "symbols": symbols, "keyframes": keyframes, "layers": layers, "animations": animations
    });
    write_json(&lib.join("entities").join(format!("{eid}.entity")), &entity)?;
    write_meta(&lib.join("entities").join(format!("{eid}.entity.meta")), eid, eid, "", Some("CUSTOM_GAME_OBJECT"), None)?;
    tables.elements.push(BootElement {
        index: i, id: eid.to_string(), segmented: true, x: ox, y: oy,
        at: match layer.pan { Some((px, py)) => BootAt::Pan(px, py), None => layer.plane.boot_at() },
        shown: Vec::new(), tint: None, alpha: None, hidden: false, ride: None, follow: None,
        path: layer.clip_path.clone(), labels: spans.iter().map(|(n, ..)| n.clone()).collect(),
        sections: Some((spans.iter().map(|(n, ..)| anim_key(n)).collect(), spans.iter().map(|(.., l)| *l).collect())),
        carry: Some((surface.to_string(), ids)), chain: None,
    });
    Ok(())
}

fn emit_bg_segmented(
    model: &StageModel, layer: &BgLayerRef, eid: &str, i: usize, lib: &Path,
    tables: &mut StageTables, entries: &mut Vec<Value>, plan: &mut Vec<Value>,
    followed: &mut std::collections::BTreeSet<String>,
) -> Result<()> {
    let scripts = lib.join("scripts").join("entities");
    std::fs::create_dir_all(&scripts)?;
    // a camera background draws its pictures unscaled and lets the entry's transform size the
    // lot, so a piece inside one is drawn at 1 too
    let scale = if matches!(layer.plane, BgPlane::CameraEntry(_)) { 1.0 } else { model.scale };
    write_json(&lib.join("entities").join(format!("{eid}.entity")), &bg_segment_entity(eid, layer, scale))?;
    write_meta(&lib.join("entities").join(format!("{eid}.entity.meta")), eid, eid, "", Some("CUSTOM_GAME_OBJECT"), None)?;

    let spans = bg_segment_spans(layer);
    // a piece placed inside a placed object's own clip rides that object (see the vfx path)
    let rider = model.hazards.iter().find(|hz| hz.instance_name.as_deref()
        .is_some_and(|n| rides_under(&layer.named_path, n)));
    // the plan names the object, its source clip and the sections it plays, where it stands
    {
        let (ox, oy) = bg_element_origin(layer);
        plan.push(json!({ "kind": "object", "id": eid, "var": format!("_bg{i}"), "x": ox, "y": oy,
                          "clip": layer.clip, "names": layer.named_path, "rides": rider.and_then(|hz| hz.instance_name.clone()),
                          "phases": spans.iter().map(|(n, ..)| n.clone()).collect::<Vec<_>>(),
                      "phase_lens": spans.iter().map(|(.., l)| *l).collect::<Vec<_>>() }));
    }
    // a segment that loops plays over until the script moves on; one the source stops on holds
    // its last frame, which is the frame the source shows while it waits
    // sections that follow one another the same way every time chain in the animation stats, and
    // the engine plays them with nothing of ours running; a section that rolls for where it goes
    // keeps the script that rolls
    let chain = segment_chain(layer);
    let anim_stats = spans.iter().enumerate()
        .map(|(i, (n, ..))| match chain.as_ref().map(|c| c[i]) {
            Some(None) => format!("\t{}: {{ endType: AnimationEndType.NONE }}", anim_key(n)),
            Some(Some(j)) if j != i => format!("\t{}: {{ endType: AnimationEndType.AUTO, nextAnimation: \"{}\" }}", anim_key(n), anim_key(&spans[j].0)),
            Some(Some(_)) => format!("\t{}: {{ endType: AnimationEndType.LOOP }}", anim_key(n)),
            None => {
                let held = matches!(layer.segments.get(i).map(|(_, _, j)| j), Some(Some(crate::abc_parser::LabelJump::Stop)));
                format!("\t{}: {{ endType: AnimationEndType.{} }}", anim_key(n), if held { "NONE" } else { "LOOP" })
            }
        })
        .collect::<Vec<_>>().join(",\n");
    let files = [
        ("Script", if chain.is_some() { bg_segment_native_script_hx(eid, layer) } else { bg_segment_script_hx(eid, layer) }),
        ("GameObjectStats", format!(
            "// GameObjectStats for the segmented backdrop element {eid}.\n{{\n\
             \tspriteContent: self.getResource().getContent(\"{eid}\"),\n\tinitialState: PState.ACTIVE,\n\
             \tstateTransitionMapOverrides: [\n\t\tPState.ACTIVE => {{ animation: \"{}\" }}\n\t],\n\
             \tbaseScaleX: 1,\n\tbaseScaleY: 1,\n\tweight: 100,\n\tgravity: 0,\n\tfriction: 0,\n\
             \t// scenery: it is never pushed, and it never pushes anything.\n\timmovable: true,\n}}\n",
            spans.first().map(|(n, ..)| anim_key(n)).unwrap_or_default())),
        ("AnimationStats", format!(
            "// AnimationStats for the segmented backdrop element {eid}. A segment loops until the\n\
             // Script moves on at a lap boundary; one the source stops on holds its last frame.\n{{\n{anim_stats}\n}}\n")),
    ];
    for (kind, body) in files {
        let fname = format!("{eid}{kind}");
        write_script(&scripts.join(format!("{fname}.hx")), &body)?;
        write_meta(&scripts.join(format!("{fname}.hx.meta")), eid, &fname,
            if kind == "Script" { "" } else { "hscript" },
            if kind == "Script" { Some("CUSTOM_GAME_OBJECT") } else { None }, None)?;
    }
    entries.push(json!({
        "type": "customGameObject", "id": eid,
        "scriptId": format!("{eid}Script"),
        "objectStatsId": format!("{eid}GameObjectStats"),
        "animationStatsId": format!("{eid}AnimationStats"),
        "name": layer.name.clone(),
    }));
    let (ox, oy) = bg_element_origin(layer);
    // A PIECE OF THE CAMERA BACKGROUND RIDES THE CAMERA, whichever way it had to be emitted. This
    // one is an object rather than a looping vfx because its clip branches between labels, and
    // parented into the world it sits on the patch of world it was placed on -- which for a
    // screen-sized backdrop is a picture the view mostly misses, and crystal smash's whole
    // backdrop went black behind it.
    let ride = rider.map(|hz| (model.hazards.iter().position(|h| std::ptr::eq(h, hz)).unwrap_or(0), ox - hz.x, oy - hz.y, still_picture(layer)));
    // the container a camera-background piece draws in is the engine's container for its entry,
    // and the engine hands back only as many as the stage declares -- an index past the end is
    // null, and addChild on it threw and unloaded the whole stage script (kingdom1 places into
    // [13]..[18]). the boot skips a piece whose container the engine did not make, so the rest
    // of the stage still runs.
    tables.elements.push(BootElement {
        index: i, id: eid.to_string(), segmented: true, x: ox, y: oy,
        at: match layer.pan {
            Some((px, py)) => BootAt::Pan(px, py),
            None => layer.plane.boot_at(),
        },
        shown: Vec::new(), tint: None, alpha: None, hidden: false, ride,
        follow: plat_follow(model, layer, followed),
        path: layer.clip_path.clone(), labels: spans.iter().map(|(n, ..)| n.clone()).collect(),
        sections: chain.as_ref().map(|_| (spans.iter().map(|(n, ..)| anim_key(n)).collect(),
            spans.iter().map(|(.., l)| *l).collect())),
        carry: None, chain: None,
    });
    Ok(())
}














/// Does the class take itself off the stage? An object that ends its own life cannot be a template:
/// the template plays an animation and waits to be told, so the object simply never leaves. Bomb
/// factory's explosion destroys itself when its clip finishes -- templated, every blast left one
/// behind, and they piled up for the whole match.
fn class_ends_its_own_life(hz: &Hazard) -> bool { crate::stage_text::ends_its_own_life(hz) }



/// A hazard the stage's OWN code makes over and over, so this port stands one up and lets it
/// re-run the cycle instead of leaving a new one behind every time.
///
/// The question is answered by reading the stage class, not by what the object is called: does its
/// update spawn this class on a repeat, rather than placing it once at boot. A stage that builds
/// its object once in `initialize` is not recycling anything; one that builds it inside `update`
/// is doing it on a cadence, and that is the cycle a single object can stand in for.
fn recycles_itself(hz: &Hazard, model: &StageModel) -> bool {
    let Some(class) = hz.class_name.as_deref() else { return false };
    // A class that ends its own life is no cycle for one object to stand in for: the stage's
    // spawn is the cycle, and each one it makes goes when its own code says so. Only an object
    // that would otherwise pile up (nothing in its code ends it) is stood up once and recycled.
    !class_ends_its_own_life(hz) && crate::stage_text::StageText::of(model).makes_and_lets_go(class)
}

fn entity_origin(model: &StageModel) -> (f64, f64) {
    let text = crate::stage_text::StageText::of(model);
    let (mut dx, mut dy) = (Vec::new(), Vec::new());
    for hz in &model.hazards {
        let Some(class) = hz.class_name.as_deref() else { continue };
        let (x, y) = text.places(class);
        if let Some(x) = x { dx.push(hz.x / model.scale - x); }
        if let Some(y) = y { dy.push(hz.y / model.scale - y); }
    }
    // A stage states a spawn in TERRAIN-local coordinates, so where the source has a terrain
    // clip, its registration IS the offset between the two spaces, and the stated places only
    // agree with it (or, for an object the parser could not place, say nothing true: saffron
    // city's pokemon spawner has no art and sat at 0, which put every object's sums 700 px
    // off the terrain). The median of the stated places is for a stage with no terrain clip,
    // and the art's own origin is for one that states nothing at all.
    if let Some((tx, ty)) = model.terrain_off {
        return (-tx, -ty);
    }
    let (fx, fy) = model.origin;
    let median = |mut v: Vec<f64>, fallback: f64| {
        if v.is_empty() { return fallback; }
        v.sort_by(|a, b| a.partial_cmp(b).expect("finite offsets"));
        -v[v.len() / 2]
    };
    (median(dx, fx), median(dy, fy))
}

fn hazard_boot(model: &StageModel, plan: &mut Vec<Value>) -> Vec<BootObject> {
    let mut out: Vec<BootObject> = Vec::new();
    // A hazard the stage's OWN code puts down is not also put down here. The stage class spawns
    // its Tingle and its flaming rocks on its own schedule and in its own places; spawning them
    // again at boot leaves a second copy standing wherever the hazard was declared, which for
    // clocktown is off the left edge of the ground, for the whole match.
    // ...and the same for an object one of the stage's OWN OBJECTS makes: planet namek's house
    // spawns its chunk, and a chunk the boot placed as well was one the source never had, dying
    // at once where it was put.
    let text = crate::stage_text::StageText::of(model);
    let self_spawned: std::collections::BTreeSet<String> = model.hazards.iter().enumerate()
        .filter_map(|(i, hz)| {
            let c = hz.class_name.as_deref()?;
            // ...unless this port emits it as one object that recycles: then the boot spawn is the
            // only one, and the stage's own is the duplicate that gets dropped instead.
            let hid = hazard_id(&model.id, i, hz);
            // the ask reads the same whether the class is still named as SSF2 wrote it or has
            // already been lowered onto the content this port ships it as
            (!recycles_itself(hz, model) && text.makes(c, &hid)).then_some(hid)
        })
        .collect();
    for (i, hz) in model.hazards.iter().enumerate() {
        let hid = hazard_id(&model.id, i, hz);
        // the clips the source draws this object with, by the names its own tree gives them, so
        // the comparison pairs them with this object whatever the port named it
        // ...by their local names: the source's tree calls a clip by the last part of its class
        let clips: Vec<&str> = [hz.linkage_id.as_deref(), hz.art_clip.as_deref(), hz.instance_name.as_deref(), hz.class_name.as_deref()]
            .into_iter().flatten().map(|n| n.rsplit('.').next().unwrap_or(n)).collect();
        if self_spawned.contains(&hid) {
            log::info!("{hid}: the stage's own code spawns this, so it is not spawned at boot too");
            plan.push(json!({ "kind": "spawned", "id": hid, "clips": clips,
                "phases": hz.anims.iter().map(|a| a.label.clone()).collect::<Vec<_>>() }));
            continue;
        }
        // owned by a character (a GameObject) so the hitbox registers; setX/setY positions it.
        // ...with the clip it wears and the sections it plays, so the comparison pairs the source's
        // clip with this object's state machine rather than hunting for an element of its length
        // (central highway's floating platforms are the stance the stage hands the objects)
        plan.push(json!({ "kind": "object", "id": hid, "var": format!("_hz{i}"), "x": hz.x, "y": hz.y,
            "clip": hz.art_clip, "clips": clips, "phases": hz.anims.iter().map(|a| a.label.clone()).collect::<Vec<_>>(),
            "placements": hz.placements.iter().map(|(l, x, y)| json!({ "label": l, "x": x, "y": y })).collect::<Vec<_>>() }));
        // AHEAD OF THE FIGHTERS, when the class asks to be. Its drawing is moved into the
        // foreground container, which is the authoritative depth control here, and it is the stage
        // that does the moving because the stage is what holds the containers.
        // ...with the separate piece it named, if it named one: drawn in the same plane and kept
        // on the object, since the object is what moves
        let fg = (hz.draws_in_front && !hz.fg_anims.is_empty()).then(|| format!("{}Fg", hazard_id(&model.id, i, hz)));
        // An object the tree PLACED is one the stage's own code addresses by the name it was
        // placed under (`getPlatform("left_plat")`), so it is filed under that name as it is made.
        let is_gated = hz.class_name.as_deref()
            .is_some_and(|c| model.hazard_gate.gated_classes.iter().any(|g| g == c));
        // A PLACED BODY IS THERE BEFORE THE FIGHTERS ARE. A collision clip the tree places is
        // part of the stage from its first frame, and the engine puts a fighter down on whatever
        // floor is under its entrance when the match starts: made a frame later, the cliff a
        // fighter starts on is not there yet, and it starts on the ground beneath instead.
        let placed_body = !hz.declared_kind.is_empty() && hz.behavior.self_platform.is_some();
        let mut deck = None;
        if placed_body {
            // ...and the engine's start placement reads only the stage's own collision, so a
            // fighter whose entrance is over a placed body is put down on the ground BENEATH it,
            // inside the body. It starts on the body, where the source starts it.
            let (bx, by, bw, bh) = hz.behavior.self_platform.unwrap();
            let s = model.scale;
            let (l, t, r, b) = (bx * s, by * s, (bx + bw) * s, (by + bh) * s);
            // ...but only where an ENTRANCE sits over the deck: an entrance ON the deck or inside
            // the body (the engine's placement drops such a fighter through to the ground
            // beneath); an entrance below the body (jungle island's barrels hang over the start
            // points) is left on its ground
            let entrance_over_deck = model.entrances.iter()
                .any(|e| e.x >= hz.x + l && e.x <= hz.x + r && e.y >= hz.y + t - 4.0 && e.y <= hz.y + b + 4.0);
            if entrance_over_deck { deck = Some((l, t, r)); }
        }
        out.push(BootObject {
            index: i, id: hid, x: hz.x, y: hz.y, front: hz.draws_in_front, fg,
            name: hz.instance_name.clone(), gated: is_gated && !placed_body, placed: placed_body, deck,
        });
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
/// What a camera background's `scaleMultiplier` has to be for the layer to frame the way the
/// source frames it.
///
/// A camera background is drawn in SCREEN space, not world space, so the stage's world scale is
/// the wrong number for it: what matters is the screen it was drawn for. SSF2 draws its backdrops
/// on a 640x360 screen and Fraymakers' is 1280x720, so the layer belongs at 2x -- and the engine
/// halves whatever it is given before using it as the transform (measured live: a layer emitted at
/// the stage scale came out twice the size the source draws it). 2 x 0.5 = 1.
/// The stage's own scale is NOT part of this. That scale carries SSF2's world into Fraymakers',
/// and a camera background is not in the world: it is tethered to the camera and sized against the
/// SCREEN, so it wants the ratio of the two screens and nothing else. Multiplied by the stage scale
/// where the stage's light is: its own where it sits overhead of the play area, else one the
/// stage's geometry stands in for (this engine's units)
fn light_place(model: &StageModel) -> (f64, f64) {
    // Height is measured from the GROUND the shadows land on, not from the top of the camera
    // box. The two differ by however far the stage sits from the origin, and a converted stage
    // sits wherever SSF2 drew it -- clocktown's floor is ~830 down, so measuring from the
    // camera box put the light 1557 above the floor instead of a stage-height, and the engine
    // projected every shadow into a ~900px vertical smear.
    let cx = match &model.camera_box {
        Some(r) => r.left() + r.w / 2.0,
        None => 0.0,
    };
    // the ground shadows land on, and separately the top of the play area -- a declared light
    // has to be above the latter to be a light rather than a lamp in the scene.
    let ground = model.main_floor().map(|f| f.rect.top())
        .or_else(|| model.camera_box.as_ref().map(|r| r.top()))
        .unwrap_or(0.0);
    let top = model.camera_box.as_ref().map(|r| r.top()).unwrap_or(ground);
    // the stage's own light, but only where it is actually OVERHEAD of the play area. A
    // symbol that calls itself a light source is as often a lamp standing in the scene, and
    // one of those sits beside the fighters rather than above them: taken literally it rakes
    // every shadow sideways across the whole stage. Above the camera box is the test that
    // tells a light apart from a lit prop, and a prop falls back to the synthesised light.
    let (lx, ly) = match model.art.light_source {
        Some(p) if p.1 < top => p,
        _ => (cx, ground - r_light_height(model)),
    };
    (lx, ly)
}

/// whether a stage's code moves its light along with the fighters (writes its light's `x`)
pub fn light_follows(script: &str) -> bool {
    script.match_indices("getLightSourceMC()").any(|(i, m)| script[i + m.len()..].trim_start().starts_with(".x"))
}

/// the camera every converted stage declares (its StageStats `camera`): the view at its closest, as
/// the source's screen is. what the source's code asks of its camera by name (`getCameraParameter`)
pub const CAMERA_STATS: [(&str, f64); 3] = [("minZoomHeight", 360.0), ("initialHeight", 360.0), ("initialWidth", 640.0)];

/// as well, fourside's moon came out a quarter again the size it is against its own skyline.
const CAMERA_BG_SCALE: f64 = 1.0;

fn stage_stats_hx(id: &str, parallax: &[ParallaxRef], shadow_layers: &str) -> String {
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
            ParallaxMode::Bounds => ("BOUNDS", "true", "true", p.w, p.h),
        };
        let anim = format!("parallax{i}");
        format!(
        "\n\t\t\t{{\n\
\t\t\t\tspriteContent: self.getResource().getContent(\"{content}\"),\n\
\t\t\t\tanimationId: \"{anim}\",\n\
\t\t\t\tmode: ParallaxMode.{mode},\n\
\t\t\t\toriginalBGWidth: {w},\n\
\t\t\t\toriginalBGHeight: {h},\n\
\t\t\t\thorizontalScroll: {hscroll},\n\
\t\t\t\tverticalScroll: {vscroll},\n\
\t\t\t\tloopWidth: {loop_w},\n\
\t\t\t\tloopHeight: {loop_h},\n\
\t\t\t\txPanMultiplier: {xp},\n\
\t\t\t\tyPanMultiplier: {yp},\n\
\t\t\t\tscaleMultiplier: {bg_scale},\n\
\t\t\t\tforeground: false,\n\
\t\t\t\tdepth: {depth}\n\
\t\t\t}}",
        w = p.w, h = p.h, xp = p.x_pan, yp = p.y_pan, depth = 2000 - (i as i64) * 10,
        content = id, anim = anim, bg_scale = CAMERA_BG_SCALE)
    }).collect();
    let backgrounds = if entries.is_empty() { String::new() } else { format!("{}\n\t\t", entries.join(",")) };
    let camera = |n: &str| CAMERA_STATS.iter().find(|(k, _)| *k == n).map(|(_, v)| *v).unwrap_or(0.0);
    let (min_zoom, init_h, init_w) = (camera("minZoomHeight"), camera("initialHeight"), camera("initialWidth"));
    format!(
        "// Stats for {id} (converted from SSF2)\n\n\
{{\n\
\tspriteContent: self.getResource().getContent(\"{id}\"),\n\
\tanimationId: \"stage\",\n\
\
\tshadowLayers: [{shadow_layers}],\n\
\tcamera: {{\n\
\t\tstartX: 0,\n\
\t\tstartY: 0,\n\
\t\tzoomX: 0,\n\
\t\tzoomY: 0,\n\
\t\tcamEaseRate: 1 / 11,\n\
\t\tcamZoomRate: 1 / 15,\n\
\t\tminZoomHeight: {min_zoom},\n\
\t\tinitialHeight: {init_h},\n\
\t\tinitialWidth: {init_w},\n\
\t\tbackgrounds: [{backgrounds}]\n\
\t}}\n\
}}\n"
    )
}

/// Stage Script.hx — pause a static stage on frame 1; let an animated stage's timeline
/// play (the SSF2 animated clips loop). The parallax background is camera-scrolled by
/// StageStats, so no manual scroll is needed.
/// The placed objects' positions at each plane label, as the label player's table.
/// The pieces that declare their own ground friction and the ones that declare a sideways push, as
/// rows of the box a fighter stands in and the numbers it gets there (this engine's units).
fn surface_rows(model: &StageModel) -> (Vec<String>, Vec<String>) {
    let frictions = model.platforms.iter().filter_map(|p| p.friction.map(|f| (p, f)))
        .map(|(p, (d, a))| format!("{{ x0: {:.1}, x1: {:.1}, y0: {:.1}, y1: {:.1}, f: {:.5}, a: {:.5} }}",
            p.rect.x, p.rect.x + p.rect.w, p.rect.y - 4.0, p.rect.y + p.rect.h + 4.0, d * accel_scale(), a * accel_scale())).collect();
    let pushes = model.platforms.iter().filter_map(|p| p.x_influence.map(|v| (p, v)))
        .map(|(p, v)| format!("{{ x0: {:.1}, x1: {:.1}, y0: {:.1}, y1: {:.1}, dx: {:.4} }}",
            p.rect.x, p.rect.x + p.rect.w, p.rect.y - 4.0, p.rect.y + p.rect.h + 4.0, v * velocity_scale())).collect();
    (frictions, pushes)
}

fn placed_at_table(model: &StageModel) -> String {
    model.hazards.iter()
        .filter_map(|hz| hz.instance_name.as_deref().map(|n| (n, hz)))
        .flat_map(|(n, hz)| hz.placements.iter().map(move |(l, x, y)|
            format!("{{ n: \"{}\", l: \"{}\", x: {x:.2}, y: {y:.2} }}", n.replace('"', "\\\""), l.replace('"', "\\\""))))
        .collect::<Vec<_>>().join(", ")
}


#[cfg(test)]
mod lowering_tests {
    use super::*;

    #[test]
    fn a_section_named_by_a_reserved_word_plays_under_that_word_with_an_underscore() {
        // waiting room's first section is `default`: no stats key can be that word
        assert_eq!(anim_key("default"), "default_");
        assert_eq!(anim_key("christmas"), "christmas");
    }

    #[test]
    fn placements_of_one_class_sharing_a_name_are_each_their_own_content() {
        let mut a = hazard_tests::demo();
        a.class_name = Some("PeptideTarget".into());
        a.instance_name = Some("stance".into());
        let mut b = a.clone();
        assert_eq!(hazard_id("s", 3, &a), "sPeptideTargetstance");
        a.shared_name = true;
        b.shared_name = true;
        assert_ne!(hazard_id("s", 3, &a), hazard_id("s", 4, &b));
    }

    #[test]
    fn synced_ledges_flag_the_body_by_side() {
        let script = "var __spLedgeL = false;\nvar __spLedgeR = false;\n".to_string();
        let out = with_sync_ledges(script.clone(), (true, false));
        assert!(out.contains("var __spLedgeL = true;") && out.contains("var __spLedgeR = false;"), "{out}");
        let out = with_sync_ledges(script, (false, true));
        assert!(out.contains("var __spLedgeL = false;") && out.contains("var __spLedgeR = true;"), "{out}");
    }
}

#[cfg(test)]
mod hazard_tests {
    use super::*;

    pub(super) fn demo() -> Hazard { demo_hazard() }

    fn demo_hazard() -> Hazard {
        Hazard {
            x: 0.0, y: 150.0, w: 700.0, h: 160.0,
            damage: 10.0, knockback: 0.0, angle: 45.0,
            interval: 0, active: 20,
            range: 0.0, period: 120, rehit: 30, kb_growth: 40.0, label: "TestHazard".into(), art: None, anims: vec![], attack_boxes: vec![], body_box: false, hitbox_dirs: vec![], anim_labels: vec![], faller: None, behavior: crate::abc_parser::EnemyBehavior::default(), reconstructed_script: None, class_name: None, max_y_speed: None, instance_name: None, declared_kind: String::new(), art_clip: None,
            external_pkg: None, linkage_id: None, fg_anims: vec![], draws_in_front: false,
            platform_shapes: vec![], body_faces: vec![], child_rects: vec![], placements: vec![], no_knockback: false, survives_bounds: false, projectiles: Default::default(), attack_stats: vec![], attack_anims: Default::default(), bounce: None, ledges: (false, false), dynamic: None, fg_name: None, shared_name: false,
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

    // A plain `var counter = 0` in a custom-game-object script RE-INITIALIZES every frame, so any
    // cross-frame accumulator silently never advances (the thwomp froze at spawn, dealt 0 damage).
    // Cross-frame state MUST be `self.make*` + .get()/.set(). Assert no naked mutable counter leaks
    // back into the hazard/thwomp scripts — this failure is invisible at convert time and only shows
    // as a dead hazard in-engine.
    #[test]
    fn hazard_scripts_use_persistent_state_not_plain_vars() {
        let cols = [(438.0, 660.0), (770.0, 616.0), (1100.0, 660.0)];
        let mut hz = demo_hazard();
        hz.range = 40.0;
        for (name, s) in [
            ("hazard_script", hazard_script_hx(&hz)),
            ("thwomp_single", thwomp_script_hx(&cols)),
        ] {
            assert!(s.contains("self.makeInt(") || s.contains("self.makeFloat(") || s.contains("self.makeBool("),
                "{name}: no persistent state (self.make*) — counters reset every frame: {s}");
            // the bug shape: a top-level mutable counter as a plain numeric var.
            for bad in ["var m_phase = 3;", "var m_timer = 0;", "var m_frame = 0;", "var m_cooldown = 0;", "var m_cool = 0;"] {
                assert!(!s.contains(bad), "{name}: plain-var counter `{bad}` re-inits every frame: {s}");
            }
        }
    }

    #[test]
    fn hazard_spawns_with_null_owner() {
        // a null hitbox owner is neutral and passes the engine's team-hit validation (hits
        // everyone), and createCustomGameObject accepts a null owner.
        let boot = crate::emit::stage_tables::boot_runtime().expect("the runtime's stage boot");
        assert!(boot.contains("createCustomGameObject(self.getResource().getContent(o.id), owner)"),
                "the boot makes an object with the owner it is handed: {boot}");
    }

    /// The boot as calls: placed bodies apart from the rest, the hazards switch around what the
    /// source gates, each element by its kind, and the numbers at the precision the boot has always
    /// had them.
    #[test]
    fn boot_calls() {
        let mut t = StageTables { objects_declared: 2, ..Default::default() };
        t.structures.push(BootStructure { id: "demoplatform0".into(), name: Some("plat".into()) });
        t.objects.push(BootObject { index: 1, id: "demohazard1".into(), x: 12.345, y: -3.0, front: false, fg: None,
            name: None, gated: true, placed: false, deck: None });
        t.objects.push(BootObject { index: 0, id: "demobody0".into(), x: 1.0, y: 2.0, front: false, fg: None,
            name: Some("body".into()), gated: false, placed: true, deck: Some((-5.0, -10.0, 5.0)) });
        t.elements.push(BootElement { index: 0, id: "demo_el".into(), segmented: false, x: 728.0, y: -60.19000000000002,
            at: BootAt::Pan(0.25, 1.0), shown: vec![(0, 10)], tint: None, alpha: Some(0.5), hidden: true, ride: None, follow: None,
            path: Vec::new(), labels: Vec::new(), sections: None, carry: None, chain: None });
        let placed = t.placed_calls("", "null");
        assert!(placed.contains("StageBoot.object({ i: 0, id: \"demobody0\", x: 1.0, y: 2.0"), "{placed}");
        assert!(!placed.contains("demohazard1"), "{placed}");
        let spawn = t.spawn_calls("", "null", "chars", "hazardsOn", |_, made| made);
        assert!(spawn.contains("StageBoot.structure(\"demoplatform0\", \"plat\");"), "{spawn}");
        assert!(spawn.contains("StageBoot.deck({ i: 0, deck: { l: -5.0, t: -10.0, r: 5.0 } }, chars);"), "{spawn}");
        assert!(spawn.contains("if (hazardsOn) {\n\tStageBoot.object({ i: 1, id: \"demohazard1\", x: 12.3, y: -3.0"), "object places at one decimal, behind the switch: {spawn}");
        assert!(spawn.contains("StageBoot.effect({ i: 0, id: \"demo_el\", seg: false, x: 728, y: -60.19000000000002"), "an element's place is written as it always was: {spawn}");
        assert!(spawn.contains("at: { pan: true, px: 0.2500, py: 1.0000 }"), "{spawn}");
        assert!(spawn.contains("alpha: 0.500"), "{spawn}");
        assert!(t.spawns_at_start() && t.has_placed() && !t.is_empty() && t.has_pan() && t.has_shown() && !t.has_tint());
    }}

/// The art fallback, reachable from the stage tests. Exposed rather than duplicated so the tests
/// pin the code that actually ships.
pub fn render_placeholder_for_test(model: &StageModel) -> StageArt { render_placeholder(model) }

/// Give a PLACED collision object its body, and keep it under the object.
///
/// A clip the stage declares as collision is not a thing that decides to become solid: the game
/// builds its class over that geometry, so the body exists from the moment the object does and
/// follows it wherever its own code moves it. A class that makes its own body (bomb factory's
/// blocks call for one when they appear) already says when, and is left alone.
fn with_body_release(script: String) -> String {
    if !script.contains("function __releaseSelfPlatform()") { return script }
    if script.contains("__releaseSelfPlatform();") { return script }
    if script.contains("function onTeardown() {}") {
        script.replace("function onTeardown() {}", "function onTeardown() {\n\t__releaseSelfPlatform();\n}")
    } else if let Some(at) = script.find("function onTeardown() {") {
        let mut s = script.clone();
        s.insert_str(at + "function onTeardown() {".len(), "\n\t__releaseSelfPlatform();");
        s
    } else {
        format!("{script}\nfunction onTeardown() {{\n\t__releaseSelfPlatform();\n}}\n")
    }
}

/// A class runs its constructor before anything asks it to do something.
fn with_constructor(script: String) -> String {
    if !script.contains("function __built()") { return script }
    if script.contains("__built();") { return script }
    // an object whose setup waits for its placement is built there, first
    if script.contains("\t__builtYet.set(true);\n") {
        return script.replacen("\t__builtYet.set(true);\n", "\t__builtYet.set(true);\n\t__built();\n", 1);
    }
    if script.contains("function initialize() {") {
        script.replacen("function initialize() {", "function initialize() {\n\t__built();", 1)
    } else {
        format!("{script}\nfunction initialize() {{\n\t__built();\n}}\n")
    }
}

/// The class says it survives the death bounds, and the engine spells that the other way round
/// on the object's stats. The source starts such a thing outside the bounds on purpose (kingdom2's
/// birdo walks in from past the edge); with the flag dropped, the engine destroyed it on its
/// first frame, every time the stage made one.
fn with_bounds_survival(script: String) -> String {
    const LINE: &str = "\tself.updateGameObjectStats({ deathBoundsDestroy: false }); // SSF2 surviveDeathBounds: true";
    if script.contains("deathBoundsDestroy: false") { return script }
    if script.contains("function initialize() {") {
        script.replacen("function initialize() {", &format!("function initialize() {{\n{LINE}"), 1)
    } else {
        format!("{script}\nfunction initialize() {{\n{LINE}\n}}\n")
    }
}

/// A hit does not send this flying, because the class says so.
///
/// SSF2 objects declare whether a hit can launch them, and a stage's furniture says no: bomb
/// factory's blocks are hit by everything a fighter throws and stay in their grid, and its bombs
/// travel the path they were given. Fraymakers has no such stat, but it has the same behaviour as
/// a body status -- damage lands, the launch does not -- so the declaration is arranged rather
/// than dropped.
fn with_launch_armor(script: String) -> String {
    const LINE: &str = "\tself.applyGlobalBodyStatus(BodyStatus.LAUNCH_ARMOR, -1); // SSF2 canReceiveKnockback: false";
    if script.contains("LAUNCH_ARMOR") { return script }
    if script.contains("function initialize() {") {
        script.replacen("function initialize() {", &format!("function initialize() {{\n{LINE}"), 1)
    } else {
        format!("{script}\nfunction initialize() {{\n{LINE}\n}}\n")
    }
}

/// It is hit, but it does not stop for the hit.
///
/// This is a difference between the two engines rather than anything a class declares. An SSF2
/// stage object is hurt down its OWN path: it takes the damage, runs its own reaction, and keeps
/// running -- there is no hit-freeze on it, which is why its classes drive themselves off their
/// clip's own frame and expect that frame to keep advancing. A Fraymakers game object shares the
/// fighters' hit reaction, freeze included, so a bomb factory block spent its hits standing still:
/// its fuse and its break both wait on frames that were not running, and it took several more hits
/// before anything happened. The freeze is let go each frame, for every ported stage object.
fn with_no_hitstop(script: String) -> String {
    const LINE: &str = "\tif (self.getHitstop() > 0) { self.startHitstop(0); } // SSF2 bypassCollisionTesting";
    if script.contains("startHitstop(0)") { return script }
    if script.contains("function update() {") {
        script.replacen("function update() {", &format!("function update() {{\n{LINE}"), 1)
    } else {
        format!("{script}\nfunction update() {{\n{LINE}\n}}\n")
    }
}

/// The ledges a body carries: a beacon synced to the object's key stands at that edge, so the
/// floor face built for the body grabs there.
fn with_sync_ledges(script: String, ledges: (bool, bool)) -> String {
    let script = if ledges.0 { script.replacen("var __spLedgeL = false;", "var __spLedgeL = true;", 1) } else { script };
    if ledges.1 { script.replacen("var __spLedgeR = false;", "var __spLedgeR = true;", 1) } else { script }
}



/// WHAT AN OBJECT'S OWN TIMELINE ASKS OF THE STAGE. The clip an object draws with carries frame
/// scripts of its own, and some reach past the object: phase 8's acid waterfall has the stage
/// remove it on its last frame and shows the foreground's waterfall clip on the way. The object
/// keeps those scripts as a table by animation and frame, fires each once as its own animation
/// reaches the frame, and what it asks of the stage crosses as a value on its exports
/// (`__stageAsk`), which the stage drains every update: a call on the stage's own method, a
/// named clip shown or hidden, a named clip played by label. The object's own calls
/// (`self.self.m()`) go to its own script when it has the method.
fn with_clip_scripts(script: String, anims: &[ClipAnim], stage_methods: &[String]) -> String {
    let scripts: &[(String, u32, String)] = anims.first().map(|a| a.clip_scripts.as_slice()).unwrap_or(&[]);
    // AN ANIMATION THE CLIP RUNS ON FROM. A label with no stop and no jump at its end runs into
    // the next label in the source (the waterfall's Idle plays through into its loop); the
    // engine's default is to loop the animation, so the run-on is stated here and played when
    // the animation reaches its last frame.
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        for a in anims { eprintln!("[clip-anim] {} loops={} stop={:?} picks={} scripts={}", a.label, a.loops, a.stop_frame, a.pick_between.len(), a.clip_scripts.len()); }
    }
    // ...read off the clip's own scripts: a label whose scripts neither stop nor send playback
    // anywhere runs into the label after it
    let own_hold = crate::re!(r#"(?m)^\s*(?:self\.(?:self\.)?)?(?:stop\(\)|gotoAndStop\([^)]*\)|gotoAndPlay\([^)]*\)|stancePlayFrame\([^)]*\)|forceAttack\([^)]*\))\s*;\s*$"#);
    let holds = |label: &str| scripts.iter().filter(|(l, _, _)| l == label)
        .any(|(_, _, code)| own_hold.is_match(code));
    let run_on: Vec<String> = anims.windows(2)
        .filter(|w| !holds(&w[0].label))
        .map(|w| format!("{{ l: \"{}\", n: \"{}\" }}", sanitize_anim(&w[0].label), sanitize_anim(&w[1].label)))
        .collect();
    if scripts.is_empty() && run_on.is_empty() { return script }
    let stage_call = crate::re!(r"^\s*SSF2API\.getStage\(\)\.(\w+)\((.*)\);\s*$");
    let named_vis = crate::re!(r"^\s*SSF2API\.getStage\(\)\.get\w+\(\)\.(\w+)\.visible\s*=\s*([^;]+);\s*$");
    let named_play = crate::re!(r#"^\s*SSF2API\.getStage\(\)\.get\w+\(\)\.(\w+)\.(?:gotoAndPlay|gotoAndStop|playFrameLabel)\("([^"]+)"\);\s*$"#);
    let own_call = crate::re!(r"^\s*self\.self\.(\w+)\((.*)\);\s*$");
    // the object's own state, kept on its exports by its ported class; a timer it hoisted
    let own_set = crate::re!(r"^\s*self\.self\.(\w+)\s*=\s*([^;]+);\s*$");
    let own_timer = crate::re!(r"^\s*self\.self\.(\w+)\.(\w+)\((.*)\);\s*$");
    let own_play = crate::re!(r#"^\s*self\.self\.(?:stancePlayFrame|forceAttack)\("([^"]+)"\);\s*$"#);
    let own = crate::re!(r"^\s*(?:self\.)?(?:stop\(\)|gotoAndStop\([^)]*\)|gotoAndPlay\([^)]*\))\s*;\s*$");
    let structural = crate::re!(r"^\s*(?:\}\s*else\s*)?(?:if\s*\((.*)\)\s*\{|\}\s*else\s*\{|\})\s*$");
    let answer = |cond: &str| -> String {
        let c = cond.replace("SSF2API.isHazardsOn()", if script.contains("m_hazardsOn") { "m_hazardsOn" } else { "true" })
            .replace("SSF2API.isReady()", "true").replace("__truthy(parent)", "true").replace("__truthy(true)", "true")
            .replace("__truthy(self.self)", "true");
        if c.contains("SSF2API.") || c.contains("self.") { "false".to_string() } else { c }
    };
    let mut entries: Vec<String> = Vec::new();
    for (label, k, code) in scripts {
        let mut lines: Vec<String> = Vec::new();
        let mut ported = false;
        for line in code.lines().skip_while(|l| !l.starts_with("function ")).skip(1).take_while(|l| !l.starts_with('}')) {
            let t = line.trim();
            if t.is_empty() || t == "return;" || own.is_match(t) { continue; }
            if let Some(c) = named_vis.captures(t) {
                lines.push(format!("\t\t__askStage(\"__setVisibleNamed\", [\"{}\", {}]);", &c[1], c[2].trim())); ported = true;
            } else if let Some(c) = named_play.captures(t) {
                lines.push(format!("\t\t__askStage(\"__playNamed\", [\"{}\", \"{}\"]);", &c[1], &c[2])); ported = true;
            } else if let Some(c) = stage_call.captures(t) {
                if stage_methods.iter().any(|m| m == &c[1]) {
                    let args = c[2].trim();
                    lines.push(format!("\t\t__askStage(\"{}\", [{}]);", &c[1], args)); ported = true;
                } else {
                    lines.push(format!("\t\t// [clip frame script: the stage has no such method] {t}"));
                }
            } else if let Some(c) = own_play.captures(t) {
                lines.push(format!("\t\tself.playAnimation(\"{}\");", sanitize_anim(&c[1]))); ported = true;
            } else if let Some(c) = own_timer.captures(t).filter(|c| script.contains(&format!("var _t_{} ", &c[1]))) {
                lines.push(format!("\t\t_t_{}.{}({});", &c[1], &c[2], c[3].trim())); ported = true;
            } else if let Some(c) = own_call.captures(t) {
                match &c[1] {
                    m if script.contains(&format!("function {m}(")) => { lines.push(format!("\t\t{m}({});", c[2].trim())); ported = true; }
                    "destroy" | "kill" => { lines.push("\t\tself.kill();".to_string()); ported = true; }
                    "addToCamera" => { lines.push("\t\tmatch.getCamera().addTarget(self);".to_string()); ported = true; }
                    "removeFromCamera" => { lines.push("\t\tmatch.getCamera().deleteTarget(self);".to_string()); ported = true; }
                    _ => lines.push(format!("\t\t// [clip frame script] {t}")),
                }
            } else if let Some(c) = own_set.captures(t) {
                lines.push(format!("\t\tself.exports.{} = {};", &c[1], c[2].trim().replace("self.self.", "self.exports."))); ported = true;
            } else if let Some(c) = structural.captures(t) {
                let l = match c.get(1) { Some(cond) => t.replacen(cond.as_str(), &answer(cond.as_str()), 1), None => t.to_string() };
                lines.push(format!("\t\t{l}"));
            } else {
                lines.push(format!("\t\t// [clip frame script] {t}"));
            }
        }
        if !ported { continue; }
        entries.push(format!("{{ l: \"{}\", k: {k}, run: function() {{\n{}\n\t}} }}", sanitize_anim(label), lines.join("\n")));
    }
    if entries.is_empty() && run_on.is_empty() { return script }
    let block = format!("\n// what this object's own timeline asks as it plays: each frame script once, when its\n\
// animation reaches the frame; what it asks of the stage crosses on the exports\n\
var __clipScripts = [{}];\n\
// the label each animation runs on into when the clip neither stops nor jumps at its end\n\
var __clipNext = [{}];\n\
var __clipLast = self.makeString(\"\");\n\
function __askStage(m, a) {{\n\
\t// straight onto the stage's own queue through the handle the spawn handed over, so an ask\n\
\t// made on the object's last frame outlives it; else on this object's exports\n\
\tvar st = self.exports.__stage;\n\
\tif (st != null && st.exports != null) {{\n\
\t\tif (st.exports.__stageAsks == null) {{ st.exports.__stageAsks = []; }}\n\
\t\tst.exports.__stageAsks.push({{ m: m, a: a }});\n\
\t\treturn;\n\
\t}}\n\
\tif (self.exports.__stageAsk == null) {{ self.exports.__stageAsk = []; }}\n\
\tself.exports.__stageAsk.push({{ m: m, a: a }});\n\
}}\n\
function __clipScriptsUpdate() {{\n\
\tvar l = self.getAnimation();\n\
\tvar k = Std.int((self.getCurrentFrame() - 1) / 2);\n\
\t// the run-on first: the last frame shares its stamp with the one before it\n\
\tif (self.getCurrentFrame() >= self.getTotalFrames()) {{\n\
\t\tvar j = 0;\n\
\t\twhile (j < __clipNext.length) {{\n\
\t\t\tif (__clipNext[j].l == l) {{ self.playAnimation(__clipNext[j].n); }}\n\
\t\t\tj = j + 1;\n\
\t\t}}\n\
\t}}\n\
\tvar stamp = l + \":\" + k;\n\
\tif (__clipLast.get() == stamp) {{ return; }}\n\
\t__clipLast.set(stamp);\n\
\tvar i = 0;\n\
\twhile (i < __clipScripts.length) {{\n\
\t\tvar e = __clipScripts[i];\n\
\t\tif (e.l == l && e.k == k) {{ e.run(); }}\n\
\t\ti = i + 1;\n\
\t}}\n\
}}\n", entries.join(", "), run_on.join(", "));
    let script = format!("{script}{block}");
    if script.contains("function update() {") {
        script.replacen("function update() {", "function update() {\n\t__clipScriptsUpdate();", 1)
    } else {
        format!("{script}\nfunction update() {{\n\t__clipScriptsUpdate();\n}}\n")
    }
}


fn with_declared_body(script: String) -> String {
    if !script.contains("function __createSelfPlatform()") { return script }
    // TWO SEPARATE QUESTIONS, and answering only the first left every body behind. A class that
    // makes its own body already has the call to make it -- but it still needs the body kept ON it
    // every frame, and the early return skipped that as well, so bomb factory's blocks left their
    // bodies wherever they were standing when they were made and a shaken or moved block walked
    // out of its own floor.
    let script = if script.contains("__createSelfPlatform();") {
        script
    } else if script.contains("function initialize() {") {
        script.replacen("function initialize() {", "function initialize() {\n\t__createSelfPlatform();", 1)
    } else {
        format!("{script}\nfunction initialize() {{\n\t__createSelfPlatform();\n}}\n")
    };
    if script.contains("__moveSelfPlatform();\n") && script.contains("function update() {\n\t__moveSelfPlatform();") {
        return script
    }
    if script.contains("function update() {") {
        script.replacen("function update() {", "function update() {\n\t__moveSelfPlatform();", 1)
    } else {
        format!("{script}\nfunction update() {{\n\t__moveSelfPlatform();\n}}\n")
    }
}

/// Keep a lingering hit alive, the way the source engine does it for free.
///
/// SSF2 gives an attack a refresh rate: stand in the lava and it burns you again every so often.
/// Fraymakers hits each target once per attack id, so the same standing fighter is burned once and
/// never again. That difference is between the two ENGINES, not between the two objects, so it is
/// answered here rather than in the object's ported code -- which is why porting the lava class
/// verbatim (its whole body is `forceAttack("idle")`) still has to come through this.
fn with_hitbox_rearm(script: String, rehit: u32) -> String {
    if script.contains("reactivateHitboxes()") { return script }
    let period = rehit.max(1);
    let decl = format!(
        "// the source attack's refresh rate, in this engine's frames: a fighter who stays in the\n\
         // hazard is hit again on the same cadence SSF2 hits them on.\n\
         var __rehit = self.makeInt(0);\n\
         function __rearmHitbox() {{\n\
         \tif (__rehit.get() > 0) {{ __rehit.dec(); return; }}\n\
         \tself.reactivateHitboxes();\n\
         \t__rehit.set({period});\n\
         }}\n\n");
    let script = format!("{decl}{script}");
    if script.contains("function update() {") {
        script.replacen("function update() {", "function update() {\n\t__rearmHitbox();", 1)
    } else {
        format!("{script}\nfunction update() {{\n\t__rearmHitbox();\n}}\n")
    }
}

