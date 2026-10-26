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
    // the stage's own scripts reach the stage their own way (`self.exports.__stage`)
    let _in_stage = crate::api_mappings::StageScope::enter();
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
        let want_re = crate::re!(r#"AudioClip\.play\(\s*"([^"]+)"\s*\)"#);
        let mut want: std::collections::BTreeSet<String> = Default::default();
        for hz in &model.hazards {
            let Some(src) = hz.reconstructed_script.as_deref() else { continue };
            for c in want_re.captures_iter(src) {
                if !shipped_sounds.contains(&c[1]) { want.insert(c[1].to_string()); }
            }
        }
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
        promoted_bg.push(BgLayerRef { starts_hidden: false, presence: Vec::new(), named_path: Vec::new(),
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
    let (hazard_entries, object_methods, object_values, stage_chains) = emit_hazards(model, &lib, &shipped_sounds)?;
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
    let (group_fields, bg_entries, clip_parts) = emit_bg_elements(model, &promoted_bg, &lib, &mut plan, &mut boot, &carriers, &carried)?;
    let clip_handles: Vec<String> = clip_parts.iter().map(|(c, _)| c.clone()).collect();

    write_json(&lib.join("manifest.json"), &build_manifest(model, &[hazard_entries.clone(), bg_entries.clone(), lib_entries.clone()].concat(), &structure_contents))?;
    write_meta(&lib.join("manifest.json.meta"), id, "manifest", "json", None, None)?;

    let scripts = lib.join("scripts").join("stage");
    // animated if ANY looping art layer has multiple frames — the background (lava/fire/Bowser on
    // bowserscastle) animates just as much as the stage plane, so checking only the stage plane
    // froze background-animated stages (the Script paused the whole timeline).
    // the foreground glow flickers (its own animated layer), so it also requires a playing timeline.
    let animated = art.background.iter().chain(&art.background_front).any(|l| l.frames.len() > 1) || art.stage.len() > 1
        || !art.foreground.is_empty();
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
    let (weather_boot, weather_frame) = emit_weather(model, &lib)?;
    boot.weather = weather_boot;
    for (i, w) in model.weathers.iter().enumerate() {
        plan.push(json!({ "kind": "weather", "id": format!("{}_weather{i}", model.id),
                          "var": format!("m_weather{i}"), "count": w.count.min(WEATHER_MAX_PARTICLES) }));
    }
    // where the stage script asks for the boot: the placed bodies in its initialize, the rest once
    // the match is live, and each weather field's particles once there is a camera. A named clip
    // and a weather field are the script's own variables, so it binds those itself.
    let boot_spawns = if boot.has_placed() { "\t\t\tStageBoot.placed(owner);\n".to_string() } else { String::new() };
    let spawns = if !boot.spawns_at_start() { String::new() } else {
        let mut s = String::from("\t\t\tStageBoot.spawn(owner, chars, m_hazardsOn);\n");
        for (k, g) in boot.groups.iter().enumerate() {
            s.push_str(&format!("\t\t\t{} = StageBoot.group({k});\n", g.name));
        }
        s.push_str(&group_fields);
        if !boot.cam_bg.is_empty() { s.push_str("\t\t\tStageBoot.camBg();\n"); }
        s
    };
    let weather_spawn: String = boot.weather.iter().enumerate()
        .map(|(k, w)| format!("\t\t\tm_weather{f} = StageBoot.weather({k}, m_weatherAlpha{f});\n", f = w.field))
        .collect();
    // The stage class's OWN code, alongside the generated script rather than instead of it.
    //
    // This is the thing that should eventually BE the stage script: SSF2 puts the cadence there --
    // when a thwomp drops and onto which column, when the clock turns over, when it rains. The
    // generated setup below is a conversion of the stage's DATA -- collision, backgrounds, weather,
    // the objects it declares -- and this is a conversion of its CODE. Both, composed: the setup
    // gives Fraymakers what the source never had to build, and the ported class then runs.
    //
    // Its spawns convert too: SSF2 hands the engine a class, Fraymakers takes content, and the
    // package ships each of those classes as content.
    // ...but NOT by loading the class wholesale, which was tried and fails for a reason worth
    // stating: much of a stage class manipulates SSF2's DISPLAY TREE
    // (`getBackground().clocktown_rainsplash_bg`, `m_rain.update()`), and this package has already
    // converted those clips into art, VFX and its own weather. That code is not missing here, it is
    // REDUNDANT -- and run as-is it reaches for a tree Fraymakers does not have and dies on null.
    //
    // So it is written beside the script for reading until the display-tree half can be told from
    // the behaviour half. What transfers is the behaviour: the spawn cadence, the day counting, the
    // timers. What does not is everything that arranges pictures.
    let ported = model.stage_script.as_ref().map(|src| {
        // the id the object was emitted under: the class name with only its letters and digits
        // (phase 8's SR388_AcidWaterfall ships as SR388AcidWaterfall, and asked for by its raw
        // name the spawn made nothing)
        let converted = crate::api_mappings::rewrite_spawn_enemy_calls(
            src, &|class: &str| format!("{id}{}", class.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>()));
        // A JUMP THE STAGE MAKES ON ITS MIDGROUND OR ITS MASK is a jump of the whole stage to a
        // section: castle siege sends its midground to `transition1` and every plane follows.
        // Here the planes are groups and the painted art is the stage's own animation, so the
        // jump fans out to all of them by name.
        let converted = converted
            .replace("self.getMidground().playFrameLabel(", "__planesPlayFrameLabel(")
            .replace("self.getShadowMaskMC().playFrameLabel(", "__planesPlayFrameLabel(")
            // a whole plane sent to a label is every group in it sent there, which is the same
            // fan-out (a group without the label ignores it)
            .replace("self.getBackground().playFrameLabel(", "__planesPlayFrameLabel(")
            .replace("self.getForeground().playFrameLabel(", "__planesPlayFrameLabel(")
            // the midground's clock is the stage's own timeline
            .replace("__currentFrame(self.getMidground())", "__currentFrame(self)")
            .replace("__currentLabel(self.getMidground())", "__currentLabel(self)")
            .replace("__totalFrames(self.getMidground())", "__totalFrames(self)");
        // the hazards this port emits as self-recycling objects: the stage boot stands each one up
        // once, so the stage's own spawn for it is a duplicate (see drop_recycled_spawns).
        let recycled: Vec<String> = model.hazards.iter().enumerate()
            .filter(|(_, hz)| recycles_itself(hz, model))
            .map(|(i, hz)| hazard_id(&model.id, i, hz))
            .collect();
        // the duplicate spawns go FIRST: dropping one takes the lines that placed it with it, and
        // those have to still be sitting where the dropper can see them.
        if let Ok(w) = std::env::var("PEPTIDE_TRACE_LINE") {
            for l in converted.lines().filter(|l| l.contains(&w)) { eprintln!("[trace converted] {l}"); }
            for l in named_asks(&converted).lines().filter(|l| l.contains(&w)) { eprintln!("[trace named_asks] {l}"); }
        }
        // the named asks come back BEFORE the runnable pass: that pass strikes a read of a slot
        // whose write is still a struck line, and the write of a placed object was one
        let converted = string_concat_is_plus(&named_asks(&converted));
        let converted = inline_class_statics(&converted, &class_statics(model));
        // the planes' clips are reached as the objects they became BEFORE a chain below one is
        // read, so `getForeground().giygasTextBox.giygasSpeech` is a child of a handle
        let converted = plane_reach_to_handles(&converted, &clip_handles);
        let converted = a_chain_of_clip_names(&converted, &clip_parts);
        let prepass = drop_recycled_spawns(&stage_runnable(&converted, &clip_handles,
            &model.weathers.iter().map(|w| w.field.clone()).collect::<Vec<_>>(),
            &model.hazards.iter().filter_map(|h| h.class_name.clone()).collect::<Vec<_>>(), &model.cam_backgrounds,
            &plat_segs.iter().map(|(n, _)| n.clone())
                .chain(model.hazards.iter().filter_map(|h| h.instance_name.clone()))
                // ...and every region the tree placed under a name (emerald cave's ledge marks)
                .chain(model.named_boxes.keys().cloned()).collect::<Vec<_>>()), &recycled);
        if let Ok(path) = std::env::var("PEPTIDE_DUMP_PREPASS") { let _ = std::fs::write(&path, &prepass); }
        let body = calls_reaching_through_nothing(&reads_of_a_local_nothing_writes(&prepass));
        if let Ok(w) = std::env::var("PEPTIDE_TRACE_LINE") {
            for l in body.lines().filter(|l| l.contains(&w) || l.contains("setPath")) { eprintln!("[trace after-chain] {l}"); }
        }
        // the objects the tree placed, under the names the stage's code asks for them by
        let placed_by_name: std::collections::BTreeMap<String, String> = model.hazards.iter().enumerate()
            .filter_map(|(i, hz)| hz.instance_name.clone().map(|n| (n, hazard_id(&model.id, i, hz))))
            .collect();
        // A CLIP AN OBJECT DRAWS WITH IS THAT OBJECT. The stage still reaches for the clip by name
        // to show and hide it -- SSF2's own way of saying which of its platforms is up -- and that
        // reach lands on the object the clip was handed to.
        let body = model.hazards.iter().filter_map(|hz| Some((hz.art_clip.as_deref()?, hz.instance_name.as_deref()?)))
            .fold(body, |acc, (clip, place)| {
                regex::Regex::new(&format!(r"[\w.()]*\.{}\.setVisible\(([^;]+)\)", regex::escape(clip)))
                    .map(|re| re.replace_all(&acc, format!("__objectVisible(\"{place}\", ${{1}})").as_str()).into_owned())
                    .unwrap_or(acc)
            });
        // THE SOURCE SPAWNING ONE OF ITS OWN CLASSES, by handing the class itself over. This
        // engine has no such call and no such value -- a class is not a thing you can pass here --
        // so the name went out bare and read as an unknown variable, which took the frame with it
        // and left the slot empty for everything after. The package emits the same object under a
        // content id of its own, so the spawn becomes that: the slot then holds what the source
        // meant it to hold, and the code that moves it afterwards works.
        let body = {
            let mut out = body;
            for (i, hz) in model.hazards.iter().enumerate() {
                let Some(class) = hz.class_name.as_deref() else { continue };
                let re = crate::dyn_re(&format!(
                    r"[\w.]*\bspawn(?:Platform|Enemy)\(\s*{}\s*(?:,[^)]*)?\)", regex::escape(class)));
                // the id the object was emitted under, cleaned the same way (a class named with
                // an underscore, phase 8's SR388_AcidWaterfall, asked for content that was not there)
                let made = format!(
                    "match.createCustomGameObject(self.getResource().getContent({:?}), null)",
                    hazard_id(&model.id, i, hz));
                out = re.replace_all(&out, made.as_str()).into_owned();
            }
            out
        };
        let body = route_object_calls(&body, &object_methods, &object_values, &placed_by_name);
        let body = route_floor_calls(&body);
        let body = if body.contains("__floor") && !body.contains("function __floorOwner(") { format!("{FLOOR_HELPERS}{body}") } else { body };
        source_units_at_the_boundary(&body, &clip_handles)
    }).unwrap_or_default();
    if !ported.trim().is_empty() {
        write_script(&scripts.join(format!("{id}StageSource.hx.txt")), &format!(
            "// {id}: the SSF2 stage class, decompiled, translated and made runnable.\n             // Reference, not loaded -- see stage_emit for what still stands in the way.\n\n{ported}"))?;
    }

    let stage_script = {
        let body = script_hx(id, animated, &spawns, &boot_spawns, &plat_segs, (&weather_spawn, &weather_frame), model.weathers.len(), &ported,
                             &clip_handles, entity_origin(model), model.scale,
                             pan_size, &model.named_boxes, &model.named_regs, &model.terrain_labels, &placed_at_table(model));
        // the answers objects asked the stage for, worked out here where the api can be reached and
        // published as plain values, refreshed each frame.
        let body = if stage_chains.is_empty() { body } else {
            let publish: String = stage_chains.iter()
                .map(|c| format!("\tself.exports.{} = self.{c};\n", chain_key(c)))
                .collect();
            let decls: String = stage_chains.iter()
                .map(|c| format!("self.exports.{} = null;\n", chain_key(c)))
                .collect();
            format!("{decls}{}\n// what an object asked the stage for: worked out here, where the api\n                     // can be reached, and handed over as a value.\nfunction __publishStageValues() {{\n{publish}}}\n",
                    body.replace("function update() {", "function update() {\n\t__publishStageValues();"))
        };
        let body = format!("{}{}",
            (if command_mailbox(&body, false).is_empty() { body.clone() } else { body.replace("function update() {", "function update() {\n\t__runCommand();") }),
            command_mailbox(&body, false));
        // ONLY CALL WHAT WAS WRITTEN. Both of these are put in by the passes above and only when
        // the stage needs them -- a stage with no object to ask and nothing to follow gets
        // neither -- so calling them regardless names a function that is not there, and the whole
        // of update stops on its first line. The training grid has neither.
        // the library-clip helpers cover more than made clips: a Flash `addChild` of a bitmap the
        // port never builds still lowers to `__addTo`, so the helpers ship whenever the body
        // names one of them, not only when the stage makes a library clip (krazoapalace adds a
        // wind bitmap that has no FM analog, and __addTo on the null handle is a safe no-op).
        let uses_lib_helper = ["__addTo(", "__addToOf(", "__removeFrom(", "__libSettle("]
            .iter().any(|h| body.contains(h));
        let body = if lib_table.is_empty() && !uses_lib_helper { body }
                   else { format!("{body}\n{}", lib_clip_helpers(&lib_table)) };
        // WHAT A NAMED CLIP'S OWN TIMELINE DOES AS IT PLAYS. A plane that shows one section per phase
        // writes each section's `fallthrough` from its frame scripts at every label; the port runs
        // the same script when the clip's pieces reach that label and frame, on the surfaces and
        // objects under those names.
        let plane_scripts = plane_frame_scripts(model, &body);
        let body = if plane_scripts.is_empty() { body } else { format!("{body}\n{plane_scripts}") };
        // WHAT AN OBJECT'S OWN TIMELINE ASKS OF THE STAGE (see with_clip_scripts): drained here
        // a member read through a factory's field that holds a clip, once the field's value has
        // settled (the chain it was filled from only takes its final shape late)
        let body = a_child_the_group_lacks(&body, &clip_parts);
        let asks = stage_ask_block(model, &body);
        let body = if asks.is_empty() { body } else { format!("{body}\n{asks}") };
        // A PIECE THAT DECLARES ITS OWN GROUND FRICTION (`decel_friction`, `accel_friction`: race to
        // the finish's ice at 0.01 where a fighter's own stop is about 1.0): whoever stands in the
        // piece's box slows and speeds up by the piece's numbers, and gets its own back on leaving.
        // the source's stop takes decel_rate off the speed a frame, the engine takes `friction`
        // off a frame, so the numbers cross by the acceleration scale.
        let (frictions, pushes) = surface_rows(model);
        let body = if frictions.is_empty() { body } else {
            format!("{body}\n// pieces that declare their own ground friction: a fighter standing in one slows by it\n\
var __frictions = [{}];\n\
var __frictionHeld = [];\n\
function __frictionUpdate() {{\n\
\tvar ps = match.getCharacters();\n\
\tvar i = 0;\n\
\twhile (i < ps.length) {{\n\
\t\tvar p = ps[i];\n\
\t\twhile (__frictionHeld.length <= i) {{ __frictionHeld.push(null); }}\n\
\t\tvar on = null;\n\
\t\tif (p.isOnFloor()) {{\n\
\t\t\tvar x = p.getX(); var y = p.getY();\n\
\t\t\tvar k = 0;\n\
\t\t\twhile (k < __frictions.length) {{\n\
\t\t\t\tvar b = __frictions[k];\n\
\t\t\t\tif (x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1) {{ on = b; }}\n\
\t\t\t\tk = k + 1;\n\
\t\t\t}}\n\
\t\t}}\n\
\t\tif (on != null && __frictionHeld[i] == null) {{\n\
\t\t\t__frictionHeld[i] = {{ f: p.getCharacterStat(\"friction\"), a: p.getCharacterStat(\"walkSpeedAcceleration\") }};\n\
\t\t\tp.updateCharacterStats({{ friction: on.f, walkSpeedAcceleration: on.a }});\n\
\t\t}} else if (on == null && __frictionHeld[i] != null) {{\n\
\t\t\tp.updateCharacterStats({{ friction: __frictionHeld[i].f, walkSpeedAcceleration: __frictionHeld[i].a }});\n\
\t\t\t__frictionHeld[i] = null;\n\
\t\t}}\n\
\t\ti = i + 1;\n\
\t}}\n\
}}\n", frictions.join(", "))
        };
        // A PIECE THAT DECLARES A SIDEWAYS PUSH (`x_influence`, desk's lamp slopes): the game moves a
        // fighter standing on it by that much a frame, and so does this stage, on a fighter whose
        // feet are in the piece's box while it stands
        let body = if pushes.is_empty() { body } else {
            format!("{body}\n// pieces that declare a sideways push (`x_influence`): a fighter standing in one is moved by it\n\
var __pushes = [{}];\n\
function __pushUpdate() {{\n\
\tvar ps = match.getCharacters();\n\
\tvar i = 0;\n\
\twhile (i < ps.length) {{\n\
\t\tvar p = ps[i];\n\
\t\tif (p.isOnFloor()) {{\n\
\t\t\tvar x = p.getX(); var y = p.getY();\n\
\t\t\tvar k = 0;\n\
\t\t\twhile (k < __pushes.length) {{\n\
\t\t\t\tvar b = __pushes[k];\n\
\t\t\t\tif (x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1) {{ p.setX(x + b.dx); }}\n\
\t\t\t\tk = k + 1;\n\
\t\t\t}}\n\
\t\t}}\n\
\t\ti = i + 1;\n\
\t}}\n\
}}\n", pushes.join(", "))
        };
        let body = {
            let mut head = String::new();
            if body.contains("function __deliverAsks(") { head.push_str("\n\t__deliverAsks();"); }
            if body.contains("function __followUpdate(") { head.push_str("\n\t__followUpdate();"); }
            if body.contains("__platFollow.push(") || boot.has_follow() { head.push_str("\n\t__platFollowUpdate();"); }
            if body.contains("function __planeScriptsUpdate(") { head.push_str("\n\t__planeScriptsUpdate();"); }
            if body.contains("function __drainStageAsks(") { head.push_str("\n\t__drainStageAsks();"); }
            if body.contains("function __libSettle(") { head.push_str("\n\t__libSettle();"); }
            if body.contains("function __pushUpdate(") { head.push_str("\n\t__pushUpdate();"); }
            if body.contains("function __frictionUpdate(") { head.push_str("\n\t__frictionUpdate();"); }
            let body = body.replace("function update() {", &format!("function update() {{{head}"));
            // ...and the clock the schedules run on is written with those helpers, so a stage that
            // gets none of them cannot count on it either. The training grid has no object to ask
            // and nothing to follow, and counted anyway.
            if body.contains("var __clockFrames") { body }
            else { body.replace("\t__clockFrames = __clockFrames + 1;\n", "") }
        };
        // the verify counters and the made-list the gates read are infrastructure the gate needs
        // whatever the stage holds -- it asks the running script for `__verifyFrames` and reads
        // `__verifyDump()`. a bare grid (the training grid) has no hazards and no weather, so it
        // reached none of the blocks that declare them, yet its update counts and onTeardown
        // walks m_owned. declare the whole set once, at the top, when nothing else did.
        let body = if body.contains("var __verifyFrames") { body } else {
            format!(
                "var __verifyFrames = 0;\nvar __clockFrames = 0;\nvar __live = false;\n                 var m_owned = [];\nvar __made = [];\nvar __verifyBuilt = true;\n                 function __verifyDump() {{\n                 \tvar s = \"\"; var i = 0;\n                 \twhile (i < __made.length) {{\n                 \t\tvar m = __made[i]; var o = m.o; var line = m.id + \"|\" + m.k;\n                 \t\tif (o == null) {{ line = line + \"|null\"; }}\n                 \t\telse if (o.isDisposed()) {{ line = line + \"|gone\"; }}\n                 \t\telse {{ line = line + \"|\" + o.getX() + \"|\" + o.getY();\n                 \t\t\tif (m.k != \"s\") {{ line = line + \"|\" + o.getCurrentFrame() + \"|\" + o.getTotalFrames(); }} }}\n                 \t\ts = s + line + \";\"; i = i + 1;\n                 \t}}\n\treturn s;\n}}\n{body}")
        };
        // A NAME THE PACKAGE GIVES ITS OWN ART, HANDED AROUND AS A VALUE. The source passes the
        // class of the thing to make -- the splash, the raindrop -- and there are no classes here,
        // so the read finds nothing and the frame stops. A name is all it is, and the package
        // states which names are its own, so that is what it answers with.
        let body = {
            let mut out = String::with_capacity(body.len());
            let declared: std::collections::BTreeSet<String> =
                crate::re!(r"(?m)^\s*(?:var\s+|function\s+)([A-Za-z_]\w*)")
                    .captures_iter(&body).map(|c| c[1].to_string()).collect();
            for line in body.lines() {
                let t = line.trim();
                let mut done = line.to_string();
                if !t.starts_with("//") {
                    for n in &model.symbol_names {
                        if n.is_empty() || !done.contains(n.as_str()) { continue }
                        // ...unless the script binds that name itself, in which case the bare
                        // mention is a real binding rather than a class reference left dangling.
                        // A package is free to name its art whatever a variable is called.
                        if declared.contains(n.as_str()) { continue }
                        done = bare_field_read(&done, n, &format!("{n:?}"));
                    }
                }
                out.push_str(&done);
                out.push('\n');
            }
            out
        };
        let body = asking_until_nothing_more_is_lacking(&building_what_nothing_can_build(
            &calls_reaching_through_nothing(&reads_of_a_local_nothing_writes(&{
                if let Ok(w) = std::env::var("PEPTIDE_TRACE_LINE") {
                    for l in body.lines().filter(|l| l.contains(&w) || l.contains("setPath")) { eprintln!("[trace before-second] {l}"); }
                }
                body.clone()
            }))));
        // WHAT THE PORT ALREADY DID, ASKED ABOUT AFTERWARDS. The stage guards its own placing of
        // the weather on whether the weather made a container -- and the port makes and places
        // the weather itself, so the field the guard reads holds nothing by the time it runs and
        // the ask takes the frame. The statements inside were already answered elsewhere; the
        // test is the last part still asking, so it is told what the port knows: no.
        let body = {
            let mut out = String::with_capacity(body.len());
            let fields: Vec<&str> = model.weathers.iter().map(|w| w.field.as_str()).collect();
            for line in body.lines() {
                let t = line.trim();
                let reads_one = fields.iter().any(|f| {
                    t.contains(&format!("({f}.")) || t.contains(&format!(" {f}."))
                        || t.starts_with(&format!("{f}."))
                });
                let opener = t.ends_with('{')
                    && ["if (", "while (", "} else if ("].iter().any(|k| t.starts_with(k));
                if reads_one && opener {
                    if let (Some(a), Some(b)) = (t.find('('), t.rfind(')')) {
                        if a < b {
                            let indent = &line[..line.len() - line.trim_start().len()];
                            out.push_str(&format!("{indent}{}false{} // [ported elsewhere] {}\n",
                                &t[..=a], &t[b..], &t[a + 1..b]));
                            continue;
                        }
                    }
                }
                out.push_str(line);
                out.push('\n');
            }
            out
        };
        let body = format!("{body}{}", exports_publication(&body));
        // the boot's tables and the runtime that makes what they list, after every pass above:
        // they are data and the runtime's own code, not the stage's to rewrite
        if boot.is_empty() { body } else {
            format!("{body}\n// ---- the stage's boot: what it puts down as the match starts\n{}\n// ---- runtime: runtime/stage/boot.hx\n{}",
                    boot.literal(), crate::emit::stage_tables::boot_runtime()?)
        }
    };
    write_script(&scripts.join(format!("{id}Script.hx")), &stage_script)?;
    // THE RUNTIME'S STAGE SCRIPT (PEPTIDE_STAGE_TEXT keeps the pipeline's instead): the stage's own
    // class run by the runtime, over the boot above as data. It replaces only the script: the plan
    // below is still worked out from what the stage pipeline wrote, so the verify gates hold both
    // to the same plan. a stage it cannot write keeps the pipeline's script.
    if std::env::var_os("PEPTIDE_STAGE_TEXT").is_none() {
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
        match crate::emit::stage::runtime_stage(model, &dir, &input) {
            Ok((out, objects)) => {
                log::info!("{id}: the stage script runs on the runtime ({} problems, {} unknown names, {} objects on their own classes, see runtime_report.json)",
                    out.problems.len(), out.unknown.len(), objects.len());
                // an object the stage's own code makes over and over runs its own class now, body
                // and all: the pipeline's followers standing in for its body stay parked off the
                // world, with nothing to follow
                let recycled = model.hazards.iter().enumerate()
                    .any(|(i, hz)| recycles_itself(hz, model) && hz.behavior.self_platform.is_some() && objects.contains(&hazard_id(&model.id, i, hz)));
                let followers = lib.join("scripts").join("platform").join(format!("{id}thwompdeckScript.hx"));
                if recycled && followers.exists() { write_script(&followers, &platform_script_hx())?; }
                // ...and the boot no longer stands it up: it comes and goes as the stage's code
                // makes it, so the plan says so
                for row in plan.iter_mut().filter(|r| r["kind"] == "object") {
                    let made_by_code = model.hazards.iter().enumerate().any(|(i, hz)| recycles_itself(hz, model)
                        && row["id"].as_str() == Some(hazard_id(&model.id, i, hz).as_str()) && objects.iter().any(|o| row["id"].as_str() == Some(o.as_str())));
                    if made_by_code { row["kind"] = json!("spawned"); }
                }
            }
            Err(e) => {
                log::warn!("{id}: the stage script stays the stage pipeline's: {e}");
                write_json(&dir.join("runtime_report.json"), &json!({ "problems": [e.to_string()] }))?;
            }
        }
    }
    // WHAT THE STAGE'S OWN CODE DRIVES, said in the plan: a picture the class sends to a frame
    // (a platform's holder shown by its height) or that rides an object keeps the frame the
    // code picks, not the engine's count, and a check reads it as such rather than as a loop
    {
        let fanout = crate::re!(r"(?s)function __planesPlayFrameLabel\(l\) \{.*?\n\}\n");
        let own = fanout.replace(&stage_script, "");
        let played: std::collections::BTreeSet<String> = crate::re!(r"\b([A-Za-z_]\w*)\.(?:playFrameLabel|playFrame)\(")
            .captures_iter(&own).map(|c| c[1].to_string()).collect();
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
        b.add_shadow_light(lx, ly, &[], (fade_start, fade_end));
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
    let (Some(name), Some(script)) = (p.name.as_deref(), model.stage_script.as_deref())
        else { return false };
    script.contains(&format!("\"{name}\""))
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

/// ONE DECLARATION PER NAME. A field the class writes and a slot it declares can both come out
/// as a script-level `var` (dream land's `i`), and the second would start it over; the first
/// stays, carrying the stated value where one of them has it.
fn dedupe_top_level_vars(script: &str) -> String {
    let decl = crate::re!(r"^var ([A-Za-z_]\w*) = (.*);$");
    let mut best: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for line in script.lines() {
        if let Some(c) = decl.captures(line) {
            let e = best.entry(c[1].to_string()).or_insert_with(|| c[2].to_string());
            // a type's zero (the slot declared and nothing more) yields to a value the source
            // states: pokemon colosseum's `homeType` is "normal", not the [] its slot started as
            let bare = |v: &str| matches!(v, "null" | "[]" | "{}" | "0" | "0.0" | "\"\"" | "false");
            if bare(e) && !bare(&c[2]) { *e = c[2].to_string(); }
        }
    }
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    let mut out = String::with_capacity(script.len());
    for line in script.lines() {
        if let Some(c) = decl.captures(line) {
            let name = c[1].to_string();
            if !seen.insert(name.clone()) { continue; }
            out.push_str(&format!("var {name} = {};\n", best[&name]));
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
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

/// Emit every hazard custom game object (entity + Script/GameObjectStats/HitboxStats/
/// AnimationStats + sprite) under the library. Returns the manifest content entries.
/// The methods each emitted object publishes, keyed by content id -- what the stage is able to
/// call on it (see `route_object_calls`).
type ObjectMethods = std::collections::BTreeMap<String, Vec<String>>;

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

fn emit_hazards(model: &StageModel, lib: &Path, sounds: &std::collections::BTreeSet<String>)
    -> Result<(Vec<Value>, ObjectMethods, ObjectMethods, Vec<String>)> {
    let mut entries = Vec::new();
    let mut published: ObjectMethods = Default::default();
    let mut values: ObjectMethods = Default::default();
    let mut chains: Vec<String> = Vec::new();
    if model.hazards.is_empty() { return Ok((entries, published, values, chains)); }
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
            emit_multi_anim_hazard(&hid, hz, model, &sprites, &scripts, lib, &cols, &mut entries,
                                   &mut published, &mut values, &mut chains, sounds, &mut pool)?;
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

    // An object the stage re-spawns on a cadence gets the column cycle synthesized around its own
    // code (see recycles_itself); everything else just runs its own.
    let is_thwomp = recycles_itself(hz, model);
        // An object runs its own code here too. A class whose art carries no labelled phases is
        // still a class: central highway's platforms fade themselves out and drop fighters through,
        // and a template that only knows how to sit still cannot do any of it.
        let script = if let Some(rs) = hz.reconstructed_script.as_ref() {
            format!("// {} reconstructed from its SSF2 update()/initialize() via the character decompiler,\n\
                     // then made FM-CGO-runnable: field state -> named local states, FrameTimers -> the engine's\n\
                     // makeFrameTimer, physics -> a scripted kinematics integrator (30fps units converted).\n\n{}",
                    hz.label,
                    cgo_runnable(&inline_class_statics(rs, &class_statics(model)), &[], model.scale, None, &model.id, hz.behavior.self_platform, &hz.platform_shapes, &hz.body_faces, &hz.child_rects, sounds,
                                 &stage_own_methods(model), entity_origin(model), &[]))
        } else if is_thwomp && !cols.is_empty() {
            thwomp_script_hx(&cols)
        } else if let Some((speed, label)) = &hz.bounce {
            // ...standing on the body the clip declares, which is what a fighter lands on (a clip
            // with no labelled art of its own carries the body all the same)
            let (decls, helpers) = hz.behavior.self_platform
                .map(|b| self_platform_code(b, model.scale, &hz.platform_shapes, &hz.body_faces)).unwrap_or_default();
            format!("{decls}{helpers}{}", bounce_script_hx(hz, *speed, label))
        } else {
            hazard_script_hx(hz)
        };
        // this entity carries ONE animation, so every clip the class names is that animation
        let script = play_anim_re.replace_all(&script, "self.playAnimation(\"gameObjectIdle\")").into_owned();
        let script = with_clip_scripts(script, &hz.anims, &stage_own_methods(model));
        let script = with_sync_ledges(script, hz.ledges);
    let script = with_constructor(script);
        let script = with_declared_body(script);
        let script = if hz.no_knockback { with_launch_armor(script) } else { script };
        let script = if hz.survives_bounds { with_bounds_survival(script) } else { script };
        let script = with_no_hitstop(script);
        let script = with_body_release(script);
        let script = if hz.damage > 0.0 { with_hitbox_rearm(script, hz.rehit) } else { script };
        let script = vars_through_channel(&script);
        let script = format!("{}{}", (if getter_refresh(&script).is_empty() { script.clone() } else { script.replace("function update() {", "function update() {\n\t__refreshExports();") }), getter_refresh(&script));
        let script = format!("{}{}", (if command_mailbox(&script, hz.instance_name.is_some()).is_empty() { script.clone() } else { script.replace("function update() {", &(if hz.instance_name.is_some() { format!("function update() {{\n\t{GATE}\n\t__runCommand();") } else { "function update() {\n\t__runCommand();".to_string() })) }), command_mailbox(&script, hz.instance_name.is_some()));
        let script = format!("{script}{}", exports_publication(&script));
        let script = end_of_life_cleanup(&script);
        published.insert(hid.clone(), published_methods(&script));
        values.insert(hid.clone(), published_values(&script));
        // the same substitution the object script gets, so the chains read the same on both sides
        chains.extend(hz.reconstructed_script.as_deref()
            .map(|r| stage_value_chains(&r.replace("match.getStage()", "self.exports.__stage")))
            .unwrap_or_default());
        let files = [
            ("Script", script),
            ("GameObjectStats", hazard_gameobject_stats_hx(&hid)),
            ("HitboxStats", hazard_hitbox_stats_hx(hz)),
            ("AnimationStats", hazard_animation_stats_hx()),
        ];
        for (kind, body) in files {
            let fname = format!("{hid}{kind}");
            // last, because the routing that sends an object's own fields through `exports` runs
            // between here and where the body was built
            let body = if kind == "Script" { art_calls_on_the_object(&body) } else { body };
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
    chains.sort(); chains.dedup();
    Ok((entries, published, values, chains))
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
    cols: &[(f64, f64)], entries: &mut Vec<Value>, published: &mut ObjectMethods,
    values: &mut ObjectMethods, chains: &mut Vec<String>,
    sounds: &std::collections::BTreeSet<String>, pool: &mut PngPool,
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

    // the resting/default animation (idle), the entrance (played at spawn), and the damaging
    // falling animation (fall), resolved by keyword so the state machine and stats line up
    // regardless of the source's exact labels.
    let fall_name = hzanims.iter().find(|a| a.active)
        .map(|a| a.name.clone()).unwrap_or_else(|| hzanims[0].name.clone());
    let idle_name = hzanims.iter().find(|a| a.name.contains("idle") || a.name.contains("rest"))
        .or_else(|| hzanims.iter().find(|a| !a.active))
        .map(|a| a.name.clone()).unwrap_or_else(|| hzanims[0].name.clone());
    let entrance_name = hzanims.iter().find(|a| a.name.contains("entrance") || a.name.contains("intro"))
        .map(|a| a.name.clone()).unwrap_or_else(|| idle_name.clone());

    let is_thwomp = recycles_itself(hz, model);
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

    // EXPERIMENTAL (PEPTIDE_DECOMP_HAZARD): emit the hazard class's update()/initialize()
    // reconstructed through the character/projectile decompile→translate pipeline, instead of the
    // hand-written template. Not yet runnable (needs the field-state + FrameTimer pass) — gated so
    // it never regresses the working template; used to iterate the reconstruction toward 1:1.
    // The class's own code, rather than a template, WHEN THE STAGE ASKS FOR IT: if the stage's own
    // code calls a method on this object that only the class defines -- bomb factory's stage builds
    // a block and immediately tells it `setHome()`, `setCell()`, `startBomb()` -- then a template
    // cannot answer, however well it moves. Where the stage only makes the object and places it
    // (bowser's castle does exactly that), the template stands, because it is the verified one.
    // An object runs ITS OWN code. That is the whole port: the class says what it does, and a
    // template is a converter-side guess that only ever fits the one thing it was written for.
    // Kept as a fallback only for a class the decompiler could not reconstruct at all, which is a
    // stated gap rather than a category.
    let decomp = true;
    let _ = (stage_calls_own_methods(hz, model), class_ends_its_own_life(hz));
    let script = if let (true, Some(rs)) = (decomp, hz.reconstructed_script.as_ref()) {
        // the entity's real animation names (what registerLocalState/playAnimation need) — the
        // full clip set, not just the forceAttack subset (the fall clip has no forceAttack).
        let entity_anims: Vec<String> = hz.anims.iter().map(|a| sanitize_anim(&a.label)).collect();
        // the segments the source draws between, if its clip cuts itself up that way
        let pick_between: Vec<String> = hz.anims.iter()
            .flat_map(|a| a.pick_between.iter())
            .map(|l| sanitize_anim(l))
            .fold(Vec::new(), |mut acc, n| { if !acc.contains(&n) { acc.push(n); } acc });
        // a column-falling hazard gets the synthesized stage spawn cycle (the part of its
        // lifecycle that lives in the STAGE class, not the enemy class) — same data the template
        // uses: the spawn machine's period + columns, the spawn height, the fall-speed cap, the
        // entrance clip's frame-script bob, and the camera-target flag.
        let wrap = (recycles_itself(hz, model) && !cols.is_empty()).then(|| {
            let fc = hz.faller.clone().unwrap_or_default();
            ReconWrap {
                cols: cols.to_vec(),
                spawn_y: model.death_box.as_ref().map(|b| b.y + 60.0)
                    .unwrap_or_else(|| cols.iter().map(|(_, y)| *y).fold(f64::MAX, f64::min) - 520.0),
                spawn_period: fc.spawn_period.unwrap_or(600.0) * 2.0,
                terminal: hz.max_y_speed.or(hz.behavior.fall_gravity).unwrap_or(30.0) * velocity_scale(),
                bob: hz.anims.iter().find(|a| sanitize_anim(&a.label) == entrance_name)
                    .map(|a| a.frame_velocities.clone()).unwrap_or_default(),
                camera: hz.behavior.camera_target,
                entrance_anim: entrance_name.clone(),
                self_platform: hz.behavior.self_platform,
            }
        });
        format!("// {} reconstructed from its SSF2 update()/initialize() via the character decompiler,\n\
                 // then made FM-CGO-runnable: field state -> named local states, FrameTimers -> the engine's\n\
                 // makeFrameTimer, physics -> a scripted kinematics integrator (30fps units converted), and\n\
                 // the STAGE class's spawn cycle synthesized around the enemy class's own state machine.\n\n{}",
                hz.label, cgo_runnable(&inline_class_statics(rs, &class_statics(model)), &entity_anims, scale, wrap.as_ref(), &model.id, hz.behavior.self_platform, &hz.platform_shapes, &hz.body_faces, &hz.child_rects, sounds,
                             &stage_own_methods(model), entity_origin(model), &pick_between))
    } else if is_thwomp && !cols.is_empty() {
        // hover/spawn height: SSF2 spawns the thwomp AT getDeathBounds().y with
        // surviveDeathBounds=true. FM kills a game object outside the blast zone, so park
        // just INSIDE the top bound — still above the camera ceiling, so visually identical.
        let spawn_y = model.death_box.as_ref().map(|b| b.y + 60.0)
            .unwrap_or_else(|| cols.iter().map(|(_, y)| *y).fold(f64::MAX, f64::min) - 520.0);
        // The thwomp's motion values come from stepping ITS OWN update()/initialize() (not a
        // hardcoded template): `getCamera().shake(N)` (the slam shake), `updateEnemyStats({gravity})`
        // (the capped fall speed), and `setYSpeed(-N)` (the rise). 30fps→60fps + the size scale =
        // ×scale/2 for a per-frame velocity; the shake amplitude is a screen effect, just ×scale.
        // `match.getCamera().shake` is the FM camera API (RE'd MatchApi::getCamera → CameraApi::shake;
        // the bare `cameraApi` binding is compiler-injected, absent in interpreted hscript). Per-kind
        // defaults only apply if the class declared no value.
        let b = &hz.behavior;
        let shake_amp = b.shake.unwrap_or(13.0) * scale;
        let fall_v = b.fall_gravity.unwrap_or(30.0) * velocity_scale();
        let rise_v = b.rise_yspeed.map(f64::abs).unwrap_or(6.0) * velocity_scale();
        // the landing dust poof: the global-vfx shape from the mappings' global_vfx_map, at the
        // scale the hazard's own attachEffect call declared (× stage scale, a screen effect).
        let dust_scale = b.dust.as_ref().map(|(_, sx, _)| *sx).unwrap_or(2.0) * scale;
        // the cycle cadence, stepped from the classes' own FrameTimers (enemy: entrance delay +
        // landed wait, in declaration order; stage: the spawn machine's two phases), x2 for 60fps.
        let fc = hz.faller.clone().unwrap_or_default();
        let spawn_period = fc.spawn_period.unwrap_or(600.0) * 2.0;
        let entrance_t = fc.entrance_delay.unwrap_or(60.0) * 2.0;
        let land_wait = fc.land_wait.unwrap_or(90.0) * 2.0;
        // the entrance bob: the entrance sub-clip's frame scripts drive a setYSpeed timeline
        // (descend into view, hover, rise) — converted 30->60fps (frame x2, speed x scale/2) into
        // a timer-keyed velocity ladder the ENTRANCE branch integrates.
        let entrance_bob: String = hz.anims.iter()
            .find(|a| sanitize_anim(&a.label) == entrance_name)
            .map(|a| a.frame_velocities.iter()
                .map(|(f, v)| format!("\t\tif (m_timer.get() == {}) {{ m_vy.set({:.2}); }}\n",
                    (f.saturating_sub(1)) * 2, v * velocity_scale()))
                .collect())
            .unwrap_or_default();
        let camera = hz.behavior.camera_target;
        thwomp_multi_script_hx(cols, spawn_y, shake_amp, fall_v, rise_v, dust_scale, spawn_period, entrance_t, land_wait, &entrance_bob, camera, &entrance_name, &idle_name, &fall_name)
    } else if let Some((speed, label)) = &hz.bounce {
        // ...standing on the body the clip declares, which is what a fighter lands on
        let (decls, helpers) = hz.behavior.self_platform
            .map(|b| self_platform_code(b, scale, &hz.platform_shapes, &hz.body_faces)).unwrap_or_default();
        format!("{decls}{helpers}{}", bounce_multi_script_hx(&hzanims, &idle_name, *speed, label))
    } else {
        hazard_anim_loop_script_hx(hz, &hzanims, &idle_name)
    };
    // ECB for the engine's terrain grounding, only for a hazard that moves: head = body height
    // above the origin, hips = half-width, from the class's own createSelfPlatform box (its real
    // SSF2 collision volume) or the detected body as fallback.
    let ecb: Option<(f64, f64)> = None;
    let script = with_clip_scripts(script, &hz.anims, &stage_own_methods(model));
        let script = with_sync_ledges(script, hz.ledges);
    let script = with_constructor(script);
    let script = with_declared_body(script);
    let script = if hz.no_knockback { with_launch_armor(script) } else { script };
    let script = if hz.survives_bounds { with_bounds_survival(script) } else { script };
    let script = with_no_hitstop(script);
    let script = with_body_release(script);
    let script = vars_through_channel(&script);
    let script = format!("{}{}", (if getter_refresh(&script).is_empty() { script.clone() } else { script.replace("function update() {", "function update() {\n\t__refreshExports();") }), getter_refresh(&script));
    let script = format!("{}{}", (if command_mailbox(&script, hz.instance_name.is_some() && script.contains("function __landPlace()")).is_empty() { script.clone() } else { script.replace("function update() {", &(if hz.instance_name.is_some() { format!("function update() {{\n\t{GATE}\n\t__runCommand();") } else { "function update() {\n\t__runCommand();".to_string() })) }), command_mailbox(&script, hz.instance_name.is_some() && script.contains("function __landPlace()")));
    let script = format!("{script}{}", exports_publication(&script));
    let script = end_of_life_cleanup(&script);
    published.insert(hid.to_string(), published_methods(&script));
    values.insert(hid.to_string(), published_values(&script));
    // the same substitution the object script gets, so the chains read the same on both sides
        chains.extend(hz.reconstructed_script.as_deref()
            .map(|r| stage_value_chains(&r.replace("match.getStage()", "self.exports.__stage")))
            .unwrap_or_default());
    let files = [
        ("Script", script),
        ("GameObjectStats", hazard_gameobject_stats_multi(hid, &idle_name, ecb)),
        ("HitboxStats", hazard_hitbox_stats_multi(hz, &hzanims, dual)),
        ("AnimationStats", hazard_animation_stats_multi(&hzanims, decomp)),
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
fn expects_to_be_hit(hz: &Hazard) -> bool {
    hz.reconstructed_script.as_deref()
        .is_some_and(|s| s.contains("HIT_RECEIVED") || s.contains("ENEMY_HURT"))
}

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

/// The multi-animation Thwomp script — 1:1 from the SSF2 disasm (bowserscastle::update +
/// Thwomp::initialize/update), frame-doubled (×2 frames) and velocity-converted (×0.65 =
/// size 1.3 × half-rate):
///   stage:  spawnTimer 300f -> spawn at (random column, deathBounds top) -> waitTimer 300f
///           -> repeat  ⇒ one spawn every 600 SSF2 f = 1200 FM frames (recycled CGO).
///   thwomp: `entrance` anim, 60f hover (gravity 0)  ⇒ 120 FM frames at the spawn point;
///           then gravity 30 with max_ySpeed 30 ⇒ constant 30 px/f fall = 19.5 FM px/frame;
///           land -> `idle` anim + the column platform sinks, wait 90f ⇒ 180 FM;
///           rise at YSpeed -6 ⇒ 3.9 FM px/frame and despawn (here: hide + restart cycle).
/// Make a decompiled SSF2Enemy script runnable as an FM custom game object without erroring. The
/// decompiler gives the real `update()` state machine, but it references SSF2 instance fields
/// (`self.m_action`, FrameTimers) and SSF2-only calls an FM CGO can't run. This pass:
///   - detects the field driving the update() switch (the int `m_X` compared against the most integer
///     literals) and lowers it onto the engine's local-state subsystem: `initLocalStateMachine` +
///     `registerLocalState(value, anim)` per state, `m_X = N` -> `toLocalState(N)`, `m_X == N` ->
///     `inLocalState(N)`. the field name isn't assumed (SSF2 authors name it freely),
///   - models every other `self.m_X` field as FM persistent state (`self.makeInt`), a `FrameTimer(N)`
///     as a persistent frame counter (`.tick()`/`.completed`/`.reset()` -> counter ops),
///   - drops SSF2-only object-stat flags (bypassCollisionTesting/surviveDeathBounds) from
///     `updateGameObjectStats`,
///   - comments out any line that has no safe FM equivalent (a field bound to an `[SSF2-only]` call
///     like `createSelfPlatform`, plus `setCamBoxSize`/`addTarget`/`isOnFloor`/`AudioClip.play`/the
///     `createVfx` dust, which need verified resources) — conservatively, so it never throws.
/// `anims` are the entity's real animation names (the `forceAttack` labels) — a state is registered
/// against the label nearest its assignment, so `toLocalState` plays a clip that exists.
/// Returns the rewritten initialize()+update() body plus the persistent-state declarations.
/// The stage-side context the reconstruction can't get from the enemy class alone: SSF2 splits a
/// spawned hazard's lifecycle between the enemy class (the state machine `cgo_runnable` rebuilds)
/// and the STAGE class (the spawn machine: when/where it appears, when it's culled). These are the
/// same stepped values the template path uses.
struct ReconWrap {
    /// (x-center, landing-surface top y) per spawn column, FM coords.
    cols: Vec<(f64, f64)>,
    /// Park/spawn height (just inside the top blast bound).
    spawn_y: f64,
    /// Spawn-to-spawn period, FM frames (the stage spawn machine's full cycle).
    spawn_period: f64,
    /// Fall-speed cap (the class's getOwnStats maxYSpeed), FM px/frame.
    terminal: f64,
    /// The entrance clip's frame-script setYSpeed timeline, raw SSF2 (frame, speed).
    bob: Vec<(u32, f64)>,
    /// The class called addToCamera() — mirror with camera add/deleteTarget.
    camera: bool,
    /// FM name of the entrance animation (locates the entrance state for the bob gate).
    entrance_anim: String,
    /// The class's own createSelfPlatform box (raw SSF2 coords, relative to the body origin):
    /// lowered to runtime structures created at the call site.
    self_platform: Option<(f64, f64, f64, f64)>,
}

/// THE ENGINE IS THE ONLY PLACE THE UNITS CHANGE. A stage's code was written in the source's
/// units: every literal, every table, every step it adds to a place is in them. So the code
/// keeps them, all of it, and the conversion sits at the engine boundary and nowhere else --
/// a place READ off the engine comes in through `__isx`, a place WRITTEN to it goes out
/// through `__sx`, and nothing between ever sees the engine's numbers. Judging each value
/// instead (a literal converts, a read does not, a bare variable is whatever it was assigned)
/// left the two mixed: central highway stepped an engine read by a source literal, and palace
/// of twilight handed engine reads to an object that counted them as source numbers.
///
/// The groups and regions the port keeps for the source's clips are the engine's side of the
/// line too: a field read or written on one is a read or write of the engine's place.
fn source_units_at_the_boundary(code: &str, handles: &[String]) -> String {
    let mut out = code.to_string();
    // a clip group's place is asked and set through its getters, so the same rule reaches it
    for h in handles {
        let field_write = crate::dyn_re(&format!(r"(?m)^(\s*){}\.(x|y)\s*=\s*([^;=][^;]*);", regex::escape(h)));
        out = field_write.replace_all(&out, |c: &regex::Captures| {
            format!("{}{}.set{}({});", &c[1], h, c[2].to_ascii_uppercase(), &c[3])
        }).into_owned();
        let field_read = crate::dyn_re(&format!(r"\b{}\.(x|y)\b", regex::escape(h)));
        out = replace_reads(&out, &field_read, |c| format!("{}.get{}()", h, c[1].to_ascii_uppercase()));
    }
    // a region's rectangle: its place through the getters, its size in the source's units
    let region = r#"(__objectBoundary\("[^"()]*"\))"#;
    out = crate::dyn_re(&format!(r"(?m)^(\s*){region}\.(x|y)\s*=\s*([^;=][^;]*);"))
        .replace_all(&out, |c: &regex::Captures| format!("{}{}.set{}({});", &c[1], &c[2], c[3].to_ascii_uppercase(), &c[4])).into_owned();
    out = replace_reads(&out, &crate::dyn_re(&format!(r"{region}\.(x|y)\b")),
        |c| format!("{}.get{}()", &c[1], c[2].to_ascii_uppercase()));
    out = crate::dyn_re(&format!(r"(?m)^(\s*){region}\.(width|height)\s*=\s*([^;=][^;]*);"))
        .replace_all(&out, |c: &regex::Captures| {
            let short = if &c[3] == "width" { "w" } else { "h" };
            format!("{}{}.{short} = __sd({}); {}.{} = {}.{short};", &c[1], &c[2], &c[4], &c[2], &c[3], &c[2])
        }).into_owned();
    out = replace_reads(&out, &crate::dyn_re(&format!(r"{region}\.(width|height)\b")), |c| {
        let short = if &c[2] == "width" { "w" } else { "h" };
        format!("__srcLen({}.{short})", &c[1])
    });
    boundary_reads_and_writes(&out, &|_recv: &str| false)
}

/// Every engine place read and written in `code`, converted at the boundary: `R.getX()` becomes
/// `__isx(R.getX())` and `R.setX(arg)` becomes `R.setX(__sx(arg))`, the receiver walked back
/// over names, dots, index brackets and call parentheses, the argument taken with its
/// parentheses balanced (a regex allowed two levels and let a three-deep sum through raw:
/// planet namek's platform eased itself to a place in the source's numbers, 640 px down).
/// `skip` names the receivers left alone (a projectile the port placed, the camera).
fn boundary_reads_and_writes(code: &str, skip: &dyn Fn(&str) -> bool) -> String {
    let bytes = code.as_bytes();
    let re = regex::Regex::new(r"\.get([XY])\(\)").expect("regex");
    let mut res = String::with_capacity(code.len() + 64);
    let mut cursor = 0usize;
    for m in re.captures_iter(code) {
        let whole = m.get(0).unwrap();
        if whole.start() < cursor { continue; }
        let axis = m[1].chars().next().unwrap();
        let rs = receiver_start(bytes, whole.start());
        if rs == whole.start() || rs < cursor { continue; }
        let recv = &code[rs..whole.start()];
        let before = &code[..rs];
        let wrapped = ["__isx(", "__isy(", "__sx(", "__sy("].iter().any(|w| before.ends_with(w));
        res.push_str(&code[cursor..rs]);
        if wrapped || skip(recv) {
            res.push_str(&code[rs..whole.end()]);
        } else {
            let f = if axis == 'X' { "__isx" } else { "__isy" };
            res.push_str(&format!("{f}({})", &code[rs..whole.end()]));
        }
        cursor = whole.end();
    }
    res.push_str(&code[cursor..]);
    let out = res;
    let next_write = |s: &str| -> Option<usize> {
        match (s.find(".setX("), s.find(".setY(")) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(usize::MAX).min(b.unwrap_or(usize::MAX))),
        }
    };
    let mut res = String::with_capacity(out.len() + 64);
    let mut rest: &str = &out;
    while let Some(at) = next_write(rest) {
        let axis = &rest[at + 4..at + 5];
        let open = at + 6;
        let (mut depth, mut end) = (1usize, open);
        for (i, ch) in rest[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => { depth -= 1; if depth == 0 { end = open + i; break; } }
                _ => {}
            }
        }
        if depth != 0 { break; }
        let recv_start = receiver_start(rest.as_bytes(), at);
        let recv = &rest[recv_start..at];
        let arg = &rest[open..end];
        let t = arg.trim_start();
        let done = t.starts_with("__sx(") || t.starts_with("__sy(") || t.is_empty() || skip(recv);
        res.push_str(&rest[..open]);
        if done {
            res.push_str(arg);
        } else {
            let f = if axis == "X" { "__sx" } else { "__sy" };
            res.push_str(&format!("{f}({arg})"));
        }
        res.push(')');
        rest = &rest[end + 1..];
    }
    res.push_str(rest);
    res
}

/// a match that is READ, not assigned: the text after it is not `= value`
fn replace_reads(code: &str, re: &regex::Regex, f: impl Fn(&regex::Captures) -> String) -> String {
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0;
    for c in re.captures_iter(code) {
        let m = c.get(0).unwrap();
        let after = code[m.end()..].trim_start();
        let assigned = after.starts_with('=') && !after.starts_with("==");
        out.push_str(&code[cursor..m.start()]);
        if assigned { out.push_str(m.as_str()); } else { out.push_str(&f(&c)); }
        cursor = m.end();
    }
    out.push_str(&code[cursor..]);
    out
}

/// A CLASS'S STEP RUNS AT THE SOURCE'S RATE, AND A PLACE IT WRITES LANDS WHEN THE STEP IS DONE.
///
/// The source steps its objects thirty times a second and this engine calls `update` sixty
/// times, so a class that carries its own motion (`position.y += speed`, jungle island's barrels)
/// moved twice as fast, and every count it keeps ran double. The step now runs every other
/// engine frame, so a frame in the class's own code is a frame of the source's; its timers are
/// made with the source's counts (see `per_engine_frame`).
///
/// And a place it writes for itself lands at the end of that step. Measured live in the source:
/// a class that sets its place and then reads it back in the same step reads the old place
/// (the barrels set themselves to a path's start, then take their position from where they read
/// themselves, which is where the tree put them; the start never shows). This engine applies a
/// write at once, so the write is held and applied when the step's code is done -- the last
/// write of the step wins, as it does there.
fn the_step_runs_at_the_source_rate(body: &str) -> String {
    let lands = body.contains("self.setX(") || body.contains("self.setY(");
    let mut out = if lands { body.replace("self.setX(", "__placeX(").replace("self.setY(", "__placeY(") } else { body.to_string() };
    let mut wrappers = String::from(
        "\nvar __stepOdd = self.makeBool(false);\n\
         var __wx = null;\nvar __wy = null;\n\
         function __placeX(v) { __wx = v; }\n\
         function __placeY(v) { __wy = v; }\n\
         function __landPlace() {\n\
         \tif (__wx != null) { self.setX(__wx); __wx = null; }\n\
         \tif (__wy != null) { self.setY(__wy); __wy = null; }\n\
         }\n");
    // THE CLASS IS BUILT WHERE THE TREE PUT IT. The source constructs an object at its placement,
    // so a constructor that reads its own place (crateria's platform keeps its start as the point
    // it eases back to) reads the right one. This engine runs initialize inside the create call,
    // before the stage's setX/setY have landed, and the read came back as the origin: the
    // platform sat at (0, 0) for the whole match. So the class's own setup runs once, on the
    // object's first update, when the placement is in.
    let has_init = out.contains("function initialize() {");
    if has_init { out = out.replacen("function initialize() {", "function __stepInit() {", 1); }
    wrappers.push_str(&format!(
        "var __builtYet = self.makeBool(false);\n\
         function __buildOnce() {{\n\
         \tif (__builtYet.get()) {{ return; }}\n\
         \t__builtYet.set(true);\n\
         {}\
         \t__landPlace();\n\
         }}\n\
         function initialize() {{\n}}\n", if has_init { "\t__stepInit();\n" } else { "" }));
    if out.contains("function update() {") {
        out = out.replacen("function update() {", "function __stepUpdate() {", 1);
        wrappers.push_str("function update() {\n\t__buildOnce();\n\t__stepOdd.set(!__stepOdd.get());\n\tif (!__stepOdd.get()) { return; }\n\t__stepUpdate();\n\t__landPlace();\n}\n");
    }
    format!("{out}{wrappers}")
}

/// where the receiver of a `.getX()` at `dot` begins: back over a name, a dot, an index or a
/// call, string literals inside them included. Returns `dot` itself when nothing precedes it.
fn receiver_start(bytes: &[u8], dot: usize) -> usize {
    let mut i = dot;
    loop {
        if i == 0 { return i; }
        let c = bytes[i - 1];
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' {
            i -= 1;
            continue;
        }
        if c == b')' || c == b']' {
            let (open, close) = if c == b')' { (b'(', b')') } else { (b'[', b']') };
            let mut depth = 0usize;
            let mut j = i;
            let mut in_str: Option<u8> = None;
            while j > 0 {
                j -= 1;
                let d = bytes[j];
                if let Some(q) = in_str {
                    if d == q && !(j > 0 && bytes[j - 1] == b'\\') { in_str = None; }
                    continue;
                }
                if d == b'"' || d == b'\'' { in_str = Some(d); continue; }
                if d == close { depth += 1; }
                if d == open {
                    depth -= 1;
                    if depth == 0 { break; }
                }
            }
            if depth != 0 { return dot; }
            i = j;
            continue;
        }
        break;
    }
    // a receiver starts with a name or with a bracketed group of its own (`(a ? b : c)[j]`),
    // never with a dot
    while i < dot && !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' || bytes[i] == b'(' || bytes[i] == b'[') { i += 1; }
    i
}

/// a member access with nothing after the dot
static NO_MEMBER: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"\.\s*[=;)]").expect("regex"));

fn stage_runnable(raw: &str, clip_handles: &[String], weather_field_names: &[String],
                  class_names: &[String], cam_backgrounds: &[String], plat_names: &[String]) -> String {
    static FIELD: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let field = FIELD.get_or_init(|| regex::Regex::new(r"self\.(m_[A-Za-z0-9_]+)").unwrap());
    static TIMER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let timer = TIMER.get_or_init(||
        // the duration can have brackets of its own -- `new FrameTimer((30 * 60))` -- and stopping
        // at the first close takes half of it, which then goes into the emitted line one bracket
        // short and the whole script stops parsing
        regex::Regex::new(r"(m_[A-Za-z0-9_]+)\s*=\s*new FrameTimer\(([^()]*(?:\([^()]*\)[^()]*)*)\)").expect("regex"));

    // every field the class touches, in a stable order
    let mut fields: Vec<String> = field.captures_iter(raw)
        .map(|c| c[1].to_string()).collect();
    // ...and the ones the CLASS declares, which is the only complete answer: an author who does not
    // prefix a member `m_` still has a member, and central highway keeps four lists of collision
    // boundaries under plain names. Bound to nothing, the first read of one stops the stage's
    // whole script on its first frame.
    static DECLARED: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let declared = DECLARED.get_or_init(|| regex::Regex::new(r"(?m)^// \[class-fields\] (.*)$").unwrap());
    // ...each with the value its declared type starts it at, when it has one (`m_action=0`)
    let mut typed_starts: std::collections::BTreeMap<String, String> = Default::default();
    if let Some(c) = declared.captures(raw) {
        for w in c[1].split_whitespace() {
            match w.split_once('=') {
                Some((n, v)) => { fields.push(n.to_string()); typed_starts.insert(n.to_string(), v.to_string()); }
                None => fields.push(w.to_string()),
            }
        }
    }
    // ...and whatever else the class writes to itself. Butter building keeps its whole layout table
    // in one, and read as the engine's own the stage stopped at the first look at it.
    static OWN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let own = OWN.get_or_init(|| regex::Regex::new(r"(?m)^\s*self\.(\w+)\s*=[^=]").unwrap());
    fields.extend(own.captures_iter(raw).map(|c| c[1].to_string()));
    fields.sort(); fields.dedup();
    // a name that is not a plain hscript identifier is not a var the script can declare: a
    // private or namespace-qualified member whose resolved name carries punctuation, or a
    // garbled trait read, emitted as `var <junk> = null` stops the WHOLE script parsing
    // (kingdom1's class carried four such names). drop them; nothing in the body names them.
    fields.retain(|f| {
        let mut cs = f.chars();
        cs.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && f.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    });

    // the fields the source builds its WEATHER in: those become this package's weather, not null
    static WEATHER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let weather_re = WEATHER.get_or_init(||
        regex::Regex::new(r"self\.(m_[A-Za-z0-9_]+)\s*=\s*new \w*Weather\(").expect("regex"));
    let weather_fields: std::collections::BTreeSet<String> = weather_re.captures_iter(raw)
        .map(|c| c[1].to_string()).collect();

    // a field initialised from a FrameTimer is one, and gets the engine's -- with the source's
    // duration in SSF2 frames doubled, like every other ported timing
    let mut timers: std::collections::BTreeMap<String, String> = Default::default();
    for c in timer.captures_iter(raw) {
        timers.insert(c[1].to_string(), c[2].trim().to_string());
    }

    // a line the translation struck out, with the symbol it was struck for
    let struck_for = crate::re!(r"^(\s*)// \[SSF2-only: (\w+)\] (.*)$");
    // The display tree is gone -- its clips were converted into objects -- so a reach through it
    // becomes the object that took that clip's place. This is why the emission is grouped by clip:
    // `getBackground().clocktown_rainsplash_bg` only has something to resolve to if the clip is
    // one object rather than the thirteen it used to be.
    let mut raw = raw.to_string();
    for h in clip_handles {
        for accessor in ["self.getBackground()", "self.getForeground()", "self.getMidground()",
                         "match.getStage().getBackground()", "match.getStage().getForeground()", "match.getStage().getMidground()"] {
            raw = regex::Regex::new(&format!(r"{}\.{}\b", regex::escape(accessor), regex::escape(h)))
                .map(|re| re.replace_all(&raw, regex::NoExpand(h)).into_owned()).unwrap_or(raw);
        }
        // a clip under the named clip (`movingplatLsprite.sprite.gotoAndStop(n)`, the picture a
        // platform's holder shows by height) is a piece of the group, and the ask goes to the group
        raw = regex::Regex::new(&format!(r"\b{}\.[A-Za-z_]\w*\.(playFrameLabel|playFrame|gotoAndStop|gotoAndPlay|setVisible|setAlpha|stop|play)\(", regex::escape(h)))
            .map(|re| re.replace_all(&raw, format!("{h}.$1(").as_str()).into_owned()).unwrap_or(raw);
        // a camera background is reached through its own clip
        raw = regex::Regex::new(&format!(r"[A-Za-z_][\w.()\[\]]*getCameraBackgrounds\(\)\[\d+\]\.mc\.{}", regex::escape(h)))
            .map(|re| re.replace_all(&raw, h.as_str()).into_owned()).unwrap_or(raw);
    }
    // A NAMED SURFACE reached through the midground (`getMidground().barrierL`) is the handle the
    // script keeps for that surface: it answers the asks the source makes of a wall or a floor
    // (its place, its ignore list, whether it is on)
    for n in plat_names {
        // a name the stage binds as a clip handle of its own (a target test's counters) is that
        // handle everywhere, never a surface
        if clip_handles.iter().any(|h| h == n) { continue; }
        for accessor in ["self.getMidground()", "match.getStage().getMidground()"] {
            raw = regex::Regex::new(&format!(r"{}\.{}\b", regex::escape(accessor), regex::escape(n)))
                .map(|re| re.replace_all(&raw, format!("__objectNamed(\"{n}\")").as_str()).into_owned()).unwrap_or(raw);
        }
        // ...and the same surface reached by its bare name (the stage keeps it as a field of its
        // own: `if (barrierL != null)`), which is the same handle. a declaration, a key, a call and
        // a string are left as they are
        raw = regex::Regex::new(&format!(r"(^|[^\w.\x22])({})([^\w:(\x22]|$)", regex::escape(n)))
            .map(|re| re.replace_all(&raw, |c: &regex::Captures| format!("{}__objectNamed(\"{}\"){}", &c[1], &c[2], &c[3])).into_owned()).unwrap_or(raw);
        // ...but not one being assigned: that is the class's own field of that name
        raw = regex::Regex::new(&format!(r#"__objectNamed\("{}"\)(\s*=[^=])"#, regex::escape(n)))
            .map(|re| re.replace_all(&raw, format!("{n}$1").as_str()).into_owned()).unwrap_or(raw);
    }
    // A CHILD REACHED BY A COMPUTED NAME (`getBackground()["spirit" + n]`) is looked up among the
    // clips this port made objects of, by that name, at the time
    if let Ok(re) = regex::Regex::new(r"(?:self\.|match\.getStage\(\)\.)?get(?:Background|Foreground|Midground)\(\)\[([^\]]+)\]") {
        raw = re.replace_all(&raw, "__groupNamed($1)").into_owned();
    }
    // What is left of the display tree after its clips became objects is the tree ITSELF -- the
    // roots the clips used to hang off. Those have no counterpart at all, so they read as nothing
    // rather than as a call into an engine that does not have them.
    // A PLANE ASKED FOR AS A WHOLE (`it.stadiumMidground = self.getMidground()`) is its children
    // by name: the object the port keeps for each, under the name the source reaches it by
    // ...whatever the stage is called by (a helper keeps it as `it.stadium`): only the stage has
    // planes, so the ask names the stage whatever hangs before it
    raw = crate::re!(r"\b[A-Za-z_][\w.]*\.get(?:Midground|Background|Foreground)\(\)([^.\w]|$)")
        .replace_all(&raw, "__plane()$1").into_owned();
    // A CLIP THE CODE MAKES FROM THE LIBRARY (`getMCByLinkageName("x")`) is a vfx this port
    // makes on demand; putting it in a plane (`getBackground().container.addChild(mc)`) is the
    // plane's container taking its picture, and taking it out ends it. what shapes a body
    // (`createSelfPlatformWithMC`) is read by the object pass and left alone here
    raw = crate::re!(r"(?:self\.|match\.getStage\(\)\.)?get(Background|Foreground|Midground)\(\)(?:\.\w+)*\.addChild\(")
        .replace_all(&raw, |c: &regex::Captures| format!("__addTo(\"{}\", ", c[1].to_ascii_lowercase())).into_owned();
    raw = crate::re!(r"(?:self\.|match\.getStage\(\)\.)?get(?:Background|Foreground|Midground)\(\)(?:\.\w+)*\.removeChild\(")
        .replace_all(&raw, "__removeFrom(").into_owned();
    raw = crate::re!(r"(?:SSF2API|match)\.getMCByLinkageName\(")
        .replace_all(&raw, "__libClip(").into_owned();
    raw = camera_points(&raw);
    for root in ["self.getBackground()", "self.getForeground()", "self.getMidground()",
                 "match.getStage().getBackground()", "match.getStage().getForeground()", "match.getStage().getMidground()"] {
        // ...taking the CHILD with it. A root that reads as nothing still leaves the clip it was
        // reached through hanging off it -- `null.clocktown_rainsplash_bg` -- which is a read on
        // nothing every time the line runs. The whole reach is the thing with no counterpart, not
        // just its first half. (Same shape as the camera-background reach below.)
        if let Ok(re) = regex::Regex::new(&format!(r"{}\.[A-Za-z_]\w*", regex::escape(root))) {
            raw = re.replace_all(&raw, "null").into_owned();
        }
        raw = raw.replace(root, "null");
    }
    // A line that DRIVES something the port does not have drives nothing at all. Once the reach
    // has read as nothing, what is left is `null.setVisible(false)` -- a call on nothing, which
    // throws every time it runs. The statement is the thing with no counterpart, so it is marked
    // as one rather than left to fail.
    {
        let mut out = String::with_capacity(raw.len());
        let null_read = crate::re!(r"\bnull\.[A-Za-z_]\w*");
        for line in raw.lines() {
            let t = line.trim_start();
            // ...including a call whose NAME is what the port does not have (`null("rainStart")`):
            // calling nothing is fatal, and the statement is the thing with no counterpart.
            // Inside an expression it is not the statement that is missing, only the value, so
            // there it reads as nothing and the rest of the line stands: butter building builds a
            // record of its layout from four clip lookups, three of which this port does have.
            if !t.starts_with("null(") && t.contains("null(") {
                let fixed = crate::api_mappings::drop_calls_on_nothing(line);
                // A LINE STRUCK OUT FOR A CALL IT NO LONGER MAKES is a line that runs. The strike
                // named the symbol it was made for; with that symbol gone from the line, what is
                // left is code this port can run, and leaving it commented throws away everything
                // else the statement did.
                let fixed = match struck_for.captures(&fixed) {
                    Some(c) if !c[3].contains(&c[2]) => format!("{}{}", &c[1], &c[3]),
                    _ => fixed,
                };
                out.push_str(&fixed);
                out.push('\n');
                continue;
            }
            // ...and a READ off nothing is the same reach (`{ x: null.x }` threw on the field)
            let reads_nothing = null_read.is_match(t)
                && t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum::<i32>() == 0;
            if (t.starts_with("null.") && t.contains('(')) || t.starts_with("null(") || reads_nothing {
                let indent = &line[..line.len() - t.len()];
                out.push_str(&format!("{indent}// [no counterpart: the clip this drove is not in this stage] {t}\n"));
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        raw = out;
    }
    // In a stage script the stage IS `self`. Reaching for it through the match returns nothing
    // (measured live: `match.getStage()` is null in this context), so everything the class asks the
    // stage -- its blast bounds, its ledges -- was a call on null.
    raw = raw.replace("match.getStage()", "self");
    // A camera background reached by index is the group of pieces its clip became, when the
    // clip became any: castle siege plays its sky's transition through the first one.
    for (i, bg) in cam_backgrounds.iter().enumerate() {
        let Some(h) = clip_handle(bg).filter(|h| clip_handles.contains(h)) else { continue };
        raw = regex::Regex::new(&format!(r"[A-Za-z_][\w.()\[\]]*getCameraBackgrounds\(\)\[{i}\]\.mc\."))
            .map(|re| re.replace_all(&raw, format!("{h}.").as_str()).into_owned()).unwrap_or(raw);
    }
    // a made clip put INTO a clip the port keeps (sky pillar's stars into its backdrop) is placed
    // from that clip's origin, in its plane; taken out, it ends
    for h in clip_handles {
        raw = regex::Regex::new(&format!(r"\b{}\.addChild\(", regex::escape(h)))
            .map(|re| re.replace_all(&raw, format!("__addToOf({h}, ").as_str()).into_owned()).unwrap_or(raw);
        raw = regex::Regex::new(&format!(r"\b{}\.removeChild\(", regex::escape(h)))
            .map(|re| re.replace_all(&raw, "__removeFrom(").into_owned()).unwrap_or(raw);
    }
    // a line struck for a call it no longer makes (`addChild`, now the plane taking a picture)
    // is a line that runs
    raw = raw.lines().map(|line| match struck_for.captures(line) {
        Some(c) if !c[3].contains(&c[2]) => format!("{}{}", &c[1], &c[3]),
        _ => line.to_string(),
    }).collect::<Vec<_>>().join("\n") + if raw.ends_with('\n') { "\n" } else { "" };
    // A camera-background child the port could not give a name of its own reads as nothing rather
    // than as a reach into a tree that is gone. Each one left here is an object the stage's class
    // wants to drive and cannot -- clocktown's moon-impact flashes are two of them -- so they are
    // logged as a gap rather than passed over in silence.
    if let Some(re) = Some(crate::re!(r"[A-Za-z_][\w.()\[\]]*getCameraBackgrounds\(\)\[\d+\]\.mc\.([A-Za-z_]\w*)")) {
        // ...except a child named only in a list handed to the quality switch: at full quality
        // that list is never read (the switch only hides pieces when a setting is off), so a
        // name in it drives nothing and is no gap (hyrule castle's windmills, yoshi's story's
        // clouds, clock town's moon-impact flashes all sit in those lists and nowhere else)
        let quality_args: std::collections::BTreeSet<String> = crate::re!(r"SSF2StageQuality\.init\(([^)]*)\)")
            .captures_iter(&raw)
            .flat_map(|c| c[1].split(',').map(|a| a.trim().to_string()).collect::<Vec<_>>())
            .collect();
        let list_re = crate::re!(r"^\s*([A-Za-z_]\w*)\s*=\s*\[");
        for line in raw.lines() {
            let quality_list = list_re.captures(line).is_some_and(|c| quality_args.contains(&c[1]));
            if quality_list { continue; }
            for c in re.captures_iter(line) {
                log::warn!("stage class drives '{}', which this stage file does not contain: the class \
                            names it, but no placement of it exists here, so there is nothing to port \
                            and nothing to drive", &c[1]);
            }
        }
        raw = re.replace_all(&raw, "null").into_owned();
    }
    // "does this stage have camera backgrounds?" -- after the port it does: they are its parallax
    // layers. The accessor is SSF2's and returns nothing here, so the QUESTION is answered rather
    // than the call left to fail. Clocktown gates its whole storm behind this one, so a stage that
    // asks and gets null never starts raining.
    if let Ok(re) = regex::Regex::new(r"[A-Za-z_][\w.()]*getCameraBackgrounds\(\)\.length\s*>\s*0") {
        raw = re.replace_all(&raw, "true").into_owned();
    }
    // the WHOLE reach, index and member included: leaving the tail behind indexes nothing
    // (`null[0].mc`), which is a crash rather than the absence it is meant to express
    if let Ok(re) = regex::Regex::new(
        r"[A-Za-z_][\w.()]*getCameraBackgrounds\(\)(\[[^\]]*\])?(\.mc)?(\.[A-Za-z_]\w*)?") {
        raw = re.replace_all(&raw, "null").into_owned();
    }

    // A list of SPAWNED objects that gets emptied has to destroy what it held. In SSF2 dropping
    // the last reference is enough -- the objects are cleaned up for it -- but here the engine
    // keeps every object it was asked to make, so clearing the list only loses track of them:
    // they go on living, drawing and updating for the rest of the match. clocktown lets go of a
    // batch of flaming rocks that way, and the longer a match runs the more of them are adrift.
    if let Ok(re) = regex::Regex::new(r"(?m)^([ \t]*)(?:self\.)?(\w+)\.splice\(0,\s*(?:self\.)?(\w+)\.length\)\s*;") {
        let mut out = String::with_capacity(raw.len());
        let mut last = 0;
        for c in re.captures_iter(&raw) {
            let m = c.get(0).unwrap();
            out.push_str(&raw[last..m.start()]);
            let (pad, list) = (&c[1], &c[2]);
            // UNLESS THE SOURCE STILL EXPECTS THEM TO BE ALIVE. An object the code asks about, or
            // asks to do something, is not one it is throwing away: clocktown tells its meteors to
            // turn round and fly back up and then lets go of the list, and each meteor ends itself
            // when it leaves the death bounds. Destroying them as the list is cleared kills them on
            // the very frame they were told to rewind, so the rewind never happens.
            let elem = format!("{}[", list);
            let consulted = raw.lines().any(|l| l.contains(&elem)
                && (l.contains("inState(") || l.contains("__ask(") || l.contains(".exports.")));
            if c[2] == c[3] && !consulted {
                out.push_str(&format!(
                    "{pad}for (__o in {list}) {{ if (__o != null) {{ __o.destroy(); }} }}\n{pad}{}",
                    m.as_str().trim_start()));
            } else {
                out.push_str(m.as_str());
            }
            last = m.end();
        }
        out.push_str(&raw[last..]);
        raw = out;
    }

    // An AS3 typed vector (`new Vector.<Thing>()`) comes back through the decompiler as a
    // construction on the element's class. It is an empty list and nothing more.
    if let Ok(re) = regex::Regex::new(r"(?m)^(\s*)Vector\.(\w+)\s*=\s*\w+\.new\(\)\s*;") {
        raw = re.replace_all(&raw, "$1$2 = [];").into_owned();
    }
    // The item system is deferred, so a spawn through it cannot stand -- and neither can what the
    // lines after it do to the thing it would have returned.
    if let Ok(re) = regex::Regex::new(r"(?m)^(\s*)(.*?(\w+)\s*=\s*[^=]*\bspawnItem\(.*)$") {
        let mut dead: Vec<String> = Vec::new();
        for c in re.captures_iter(&raw.clone()) { dead.push(c[3].to_string()); }
        raw = re.replace_all(&raw, "$1// [deferred: no item system] $2").into_owned();
        for v in dead {
            if let Ok(use_re) = regex::Regex::new(&format!(r"(?m)^(\s*)({}\.\w+\(.*)$", regex::escape(&v))) {
                raw = use_re.replace_all(&raw, "$1// [deferred: no item system] $2").into_owned();
            }
        }
    }
    if let Ok(re) = regex::Regex::new(r"(?m)^(\s*)(.*\bspawnItem\(.*)$") {
        raw = re.replace_all(&raw, "$1// [deferred: no item system] $2").into_owned();
    }

    // AS3 tests an object by its truthiness (`if (obj)`, `!obj`); hscript does not, and reading a
    // null that way runs on into the call after it. Every field the class holds an OBJECT in --
    // one it never assigns a number or a bool to -- gets the comparison spelled out.
    let obj_fields: Vec<String> = fields.iter().filter(|f| {
        let assign = crate::dyn_re(&format!(r"\b{}\s*=\s*([^=;]+)", regex::escape(f)));
        let vals: Vec<String> = assign.captures_iter(&raw).map(|c| c[1].trim().to_string()).collect();
        vals.iter().all(|v| !(v == "true" || v == "false" || v.parse::<f64>().is_ok()))
    }).cloned().collect();
    for f in &obj_fields {
        // `!field` where the next thing is not a call: the AS3 "is it there" test
        let not = crate::dyn_re(&format!(r"!\s*(self\.)?{}\b", regex::escape(f)));
        raw = not.replace_all(&raw, format!("{f} == null").as_str()).into_owned();
    }
    // A FIELD REACHED BY A COMPUTED NAME (`this["block" + i]`): the source's class is a dynamic
    // object, this script's fields are its own vars, so the name goes through a lookup over them
    // (`__field`, `__setField`, defined below over every field the class has). Read as written,
    // the bracket indexed the stage's handle as a list and stopped the script.
    let computed = !fields.is_empty() && raw.contains("self[");
    let raw = if computed {
        let set_re = regex::Regex::new(r"(?m)^(\s*)self\[([^\]]+)\]\s*=\s*([^=][^;]*);").expect("computed set regex");
        let r = set_re.replace_all(&raw, "${1}__setField($2, $3);").into_owned();
        let get_re = regex::Regex::new(r"self\[([^\]]+)\]").expect("computed get regex");
        get_re.replace_all(&r, "__field($1)").into_owned()
    } else { raw };
    let raw = if raw.contains("\"]") || raw.contains("\")]") { string_indexed_members(&raw) } else { raw };
    let raw = raw.as_str();
    // every field this class declares, so a read of one is a read of this script's own var. The
    // `m_` habit is not the rule -- the declaration is.
    let own_field = regex::Regex::new(&format!(r"self\.({})\b",
        fields.iter().map(|f| regex::escape(f)).collect::<Vec<_>>().join("|")))
        .unwrap_or_else(|_| field.clone());

    let mut out = String::new();
    // the fields that start with NOTHING IN THEM: a clip the source reaches through the display
    // tree has no counterpart here, so the stage holds null for it and every call on it would stop
    // the script. The stage drives what it HAS.
    let mut unbound: std::collections::BTreeSet<String> = Default::default();
    // AS3's own rule for reading a value as a test, spelled out: a non-null object and a non-zero
    // number are true. hscript reads only a bool and calls everything else false, so without this
    // every condition the source writes as `if (thing)` is dead.
    out.push_str("function __truthy(v) { return !(v == null || v == false || v == 0); }\n\n");
    if raw.contains("__fieldOf(") || raw.contains("__setFieldOf(") { out.push_str(FIELD_OF_HELPERS); out.push('\n'); }
    out.push_str("// the stage class's own state. a stage script keeps its module scope between\n                  // frames, so these are plain vars.\n");
    for f in &fields {
        match timers.get(f) {
            Some(dur) => out.push_str(&format!(
                "var {f} = self.makeFrameTimer(({dur}) * {});\n", crate::entity_gen::FRAME_RATIO)),
            // a typed field starts at its type's zero, as the source's does
            None if typed_starts.contains_key(f) => out.push_str(&format!("var {f} = {};\n", typed_starts[f])),
            // SSF2's weather object, mapped onto the weather this package already ports. The class
            // drives it -- fading it in and out as the day turns -- so binding it to nothing is what
            // left a stage raining for the whole match. Only the field this port BUILT is bound: a
            // stage with two weathers holds the other at nothing until its own event, and bound to
            // the same field that would stop the one that is falling.
            // each field the port BUILT gets the object that drives its own particles; a weather
            // field the port did not build still answers, so the stage's code runs either way
            None if weather_fields.contains(f) => out.push_str(&match weather_field_names.iter().position(|n| n == f) {
                Some(i) => format!("var {f} = __weatherObject{i}();\n"),
                None => format!("var {f} = __weatherObjectOther();\n"),
            }),
            // a field the class pushes into is a LIST, and starts as one: SSF2 gives it its
            // array in the class's own constructor, which is not part of what a port reads.
            None if raw.contains(&format!("{f}.push("))
                || raw.contains(&format!("{f}.length"))
                || raw.contains(&format!("{f}[")) => out.push_str(&format!("var {f} = [];\n")),
            None => { out.push_str(&format!("var {f} = null;\n")); unbound.insert(f.clone()); }
        }
    }
    out.push('\n');
    if computed {
        out.push_str("// a field reached by a computed name, looked up over the class's own\n");
        out.push_str("function __field(n) {\n");
        for f in &fields { out.push_str(&format!("\tif (n == \"{f}\") {{ return {f}; }}\n")); }
        out.push_str("\treturn null;\n}\n");
        out.push_str("function __setField(n, v) {\n");
        for f in &fields { out.push_str(&format!("\tif (n == \"{f}\") {{ {f} = v; return; }}\n")); }
        out.push_str("}\n\n");
    }

    // A CALL DOES NOT HAVE TO FIT ON ONE LINE, and striking one out by commenting the line it
    // starts on leaves the rest of it standing: `SSF2StageQuality.init(` commented, and a bare
    // `null);` on the next line, which closes a paren nothing opened. The engine drops a script
    // it cannot parse WHOLE, so one stray line silently costs the stage its entire script --
    // twenty of them across the corpus. So the lines are walked with a cursor rather than an
    // iterator, and striking a line that leaves a call open strikes the rest of the call too.
    let src_lines: Vec<&str> = raw.lines().collect();
    let mut li = 0usize;
    let open_parens = |s: &str| -> i32 {
        let code = s.split("//").next().unwrap_or("");
        code.matches('(').count() as i32 - code.matches(')').count() as i32
    };
    while li < src_lines.len() {
        let line = src_lines[li];
        li += 1;
        // whatever this line was struck for, the rest of its call goes with it
        let strike_tail = |out: &mut String, first: &str, li: &mut usize| {
            let mut depth = open_parens(first);
            while depth > 0 && *li < src_lines.len() {
                let l = src_lines[*li];
                depth += open_parens(l);
                out.push_str(&format!("{}// {}\n",
                    &l[..l.len() - l.trim_start().len()], l.trim()));
                *li += 1;
            }
        };
        let t = line.trim();
        // the api objects Fraymakers has no counterpart for, and this package covers another way
        // EVERY weather class, not just the rain one. This package builds a stage's weather from
        // the source's own particle and motion (see the weather spawn above), so the class the
        // stage constructs is redundant here -- and left in, it names a linkage this port does not
        // have, which throws and takes the REST of the stage's setup down with it. Bowser's Castle
        // lost its lava that way: the spawn sits three lines below the ember weather it builds.
        // a member access with no member -- `obj. = v`, `obj.;` -- parses nowhere, and one of them
        // fails the WHOLE script, so the stage gets none of its code rather than most of it. Say so
        // and carry on; the line is worth a look, the other ninety are worth running.
        // ...judged on the code alone: a message the stage shows (devil's machine's whispers) can
        // carry any characters inside its quotes, and none of them is a member access
        if NO_MEMBER.is_match(&crate::api_mappings::code_only(t)) {
            log::warn!("a line came out with a member access and no member, so it is commented out \
                        rather than failing the whole stage script: {t}");
            out.push_str(&format!("{}// [came out unparseable] {t}\n",
                &line[..line.len() - line.trim_start().len()]));
            continue;
        }
        // ...however the construction is spelled by the time it gets here. A class the stage
        // builds for itself is turned into a factory earlier, so `new EmberWeather(` arrives as
        // `EmberWeather__new(` -- and this test, looking only for the word `new`, stopped seeing
        // it. Bowser's Castle then ran a line that had been deliberately dead for good reason.
        // ...but only where one is BUILT, never where the building is defined. Commenting the
        // `function` line leaves its body standing at the top level, which then runs on load --
        // and the body of a weather class is full of things this engine has no word for.
        // WHAT THE STRUCK CONSTRUCTION WAS GOING TO FILL. The weather is ported natively, so the
        // stage's own building of it is struck -- but the slot it would have filled is still
        // there, holding whatever it was initialised with, and the stage goes on calling the
        // weather's methods on it. An empty list has no `update`, and the ask takes the frame.
        // So the calls go with the construction: same thing, ported the same way, elsewhere.
        let weather_slot = |line: &str| -> Option<String> {
            let t = line.trim().trim_start_matches("// [ported elsewhere] ");
            if !(t.contains("Weather(") || t.contains("Weather__new(")) { return None }
            let (lhs, _) = t.split_once('=')?;
            Some(lhs.trim().trim_start_matches("self.").trim().to_string())
        };
        let weather_slots: std::collections::BTreeSet<String> =
            raw.lines().filter_map(weather_slot).collect();
        let names_a_weather_slot = weather_slots.iter().any(|n| {
            t.starts_with(&format!("{n}.")) || t.starts_with(&format!("self.{n}."))
                || t.contains(&format!(" {n}.")) || t.contains(&format!("({n}."))
        });
        let indent = &line[..line.len() - line.trim_start().len()];
        if names_a_weather_slot && t.ends_with(");") {
            out.push_str(&format!("{indent}// [ported elsewhere, with what it was called on] {t}\n"));
            continue;
        }
        // ...and where the ask is the TEST rather than the statement, the test is what goes. The
        // slot holds what it was initialised with, so asking it anything answers nothing, and a
        // block whose entry depends on that answer cannot be entered. Striking the line instead
        // would take its `{` and orphan the `}` below it, which costs the whole script.
        if names_a_weather_slot && t.ends_with('{') {
            if let (Some(a), Some(b)) = (t.find('('), t.rfind(')')) {
                if a < b && ["if (", "while (", "} else if ("].iter().any(|k| t.starts_with(k)) {
                    out.push_str(&format!("{indent}{}false{} // [ported elsewhere] {}\n",
                        &t[..=a], &t[b..], &t[a + 1..b]));
                    continue;
                }
            }
        }
        let weather_ctor = !t.starts_with("function ")
            && ((t.contains("new ") && t.contains("Weather("))
                || t.contains("Weather__new("));
        if weather_ctor {
            out.push_str(&format!("{}// [ported elsewhere] {t}\n",
                &line[..line.len() - line.trim_start().len()]));
            strike_tail(&mut out, line, &mut li);
            continue;
        }
        // The quality gate turns pieces OFF, never on: it walks the groups it is handed and hides
        // each one whose setting is not "enabled". Everything runs at full quality here, so it has
        // nothing to hide and the call carries no effect worth reproducing -- which is a different
        // thing from being ported elsewhere, and worth saying, because a stage that reads dim or
        // over-lit is a reasonable place to come looking.
        if t.contains("SSF2StageQuality") {
            out.push_str(&format!("{}// [no-op at full quality: this only HIDES pieces, and only \
when a setting is off] {t}\n",
                &line[..line.len() - line.trim_start().len()]));
            strike_tail(&mut out, line, &mut li);
            continue;
        }
        // the timer construction is hoisted into the preamble above
        if timer.is_match(t) && t.contains("new FrameTimer") { continue; }
        // the declaration list has been read; it is not code
        if t.starts_with("// [class-fields]") { continue; }
        // the class's fields are this script's own vars, whatever they are called
        out.push_str(&own_field.replace_all(line, "$1"));
        out.push('\n');
    }

    // The class's own methods call each other through `self`; here they are plain functions in one
    // script. And its lifecycle entry points are renamed, because the emitted script has its own
    // `initialize`/`update` doing the Fraymakers-side setup -- structures, backgrounds, weather --
    // that the source never had to do. The two are composed rather than one replacing the other:
    // the setup is a conversion of the stage's DATA, this is a conversion of its CODE.
    let mut fns: Vec<String> = crate::re!(r"(?m)^function\s+([A-Za-z_]\w*)\s*\(").captures_iter(&out).map(|c| c[1].to_string()).collect();
    fns.sort(); fns.dedup();
    let mut done = out;
    for f in &fns {
        done = done.replace(&format!("self.{f}("), &format!("{f}("));
    }
    done = done.replace("function initialize()", "function __source_initialize()")
               .replace("function update()", "function __source_update()");
    // A CALL ON A FIELD THAT HOLDS NOTHING stops the whole script, and a stage keeps fields for
    // clips this port has no counterpart for. So the stage drives what it has: the call runs when
    // there is something to run it on, and is skipped when there is not. Crateria's caution sign
    // is one of these, and every frame of its acid cycle died on it.
    let mut guarded = String::with_capacity(done.len());
    let call = crate::re!(r"^(\s*)([A-Za-z_]\w*)((?:\.\w+)*\.\w+\(.*\);)$");
    // A call THROUGH a list the stage filled from lookups: what it found is in there, what this
    // port does not have is not, so the walk asks whatever is actually at that place.
    // one level of nesting inside an index, which is what the sources actually write
    const IDX: &str = r"\[(?:[^\[\]]|\[[^\[\]]*\])*\]";
    let through = regex::Regex::new(
        // AN INDEX CAN HOLD BRACKETS OF ITS OWN. `plats[plats.indexOf(chars[i].where())]` has one
        // inside the other, and a pattern that stops at the first close cut the expression in
        // half -- the guard was then written around a fragment, which does not parse, and the
        // engine dropped the whole script.
        &format!(r"^(\s*)([A-Za-z_]\w*(?:{IDX}|\.\w+)*{IDX}(?:\.\w+)*)\.(\w+\(.*\);)$"))
        .expect("indexed call regex");
    // a member READ through a list of things the port may not have
    let read_through = crate::dyn_re(&format!(r"([A-Za-z_]\w*(?:{IDX}|\.\w+)*{IDX}(?:\.\w+)*)\.(\w+)([^(\w]|$)"));
    // ...and a value written INTO one is the same: a stage that scrolls moves everything it holds,
    // including what it holds nothing of.
    let write = crate::re!(r"^(\s*)([A-Za-z_]\w*)((?:\.\w+)+\s*=\s*[^;]+;)$");
    // ...and a write THROUGH a list of them: the same statement, reached by index.
    let write_through = crate::re!(r"^(\s*)([A-Za-z_]\w*(?:\[[^\]]*\]|\.\w+)*\[[^\]]*\](?:\.\w+)*)\.(\w+\s*=\s*[^;]+;)$");
    for line in done.lines() {
        // A READ of something the stage may not have, inside a test: the test asks for it and the
        // read stops the script. So the ask is spelled out -- nothing reads as nothing, and the
        // test decides what it would have decided if the thing were simply not there.
        let line = if line.trim_start().starts_with("if (") || line.trim_start().starts_with("while (") {
            read_through.replace_all(line, |c: &regex::Captures| {
                let root = c[1].split(['.', '[']).next().unwrap_or("");
                if fields.iter().any(|f| f == root) {
                    // whatever followed the read is put back: it is the rest of the test
                    format!("({} == null ? 0 : {}.{}){}", &c[1], &c[1], &c[2], &c[3])
                } else { c[0].to_string() }
            }).into_owned()
        } else { line.to_string() };
        let line = line.as_str();
        let root_field = |c: &regex::Captures| {
            let root = c[2].split(['.', '[']).next().unwrap_or("");
            fields.iter().any(|f| f == root)
        };
        match call.captures(line).or_else(|| write.captures(line)) {
            Some(c) if unbound.contains(&c[2]) => {
                guarded.push_str(&format!("{}if ({} != null) {{ {}{} }}\n", &c[1], &c[2], &c[2], &c[3]));
            }
            _ => match through.captures(line).or_else(|| write_through.captures(line)) {
                Some(c) if root_field(&c) => {
                    guarded.push_str(&format!("{}if ({} != null) {{ {}.{} }}\n", &c[1], &c[2], &c[2], &c[3]));
                }
                _ => { guarded.push_str(line); guarded.push('\n'); }
            },
        }
    }
    // AN AS3 CAST IS NOT A CALL. `MovieClip(getChildByName("x"))` is the source naming a type,
    // and the lookup inside it has no counterpart here, so it is answered with nothing -- which
    // leaves `MovieClip(null)`, read as a call to a method of that name. There is none, so it
    // throws and takes the rest of the handler with it. This has to run HERE rather than in the
    // rename table, because the cast only becomes a cast of nothing after that table has run.
    let mut out = strike_orphan_closers(&guarded);
    for ty in ["MovieClip", "Sprite", "DisplayObject", "DisplayObjectContainer"] {
        out = out.replace(&format!("self.{ty}(null)"), "null")
                 .replace(&format!("{ty}(null)"), "null");
    }
    // ...and a cast to one of the stage's OWN classes is the same thing wearing a name only this
    // stage knows. The class graph says what those names are, so there is nothing to guess and no
    // list to keep: `EmeraldCaveWoodenPanel(x)` is x, whatever the class happens to be called.
    for class in class_names {
        out = unwrap_cast_to(&out, class);
    }
    // NOTE: a spawned object's own methods are NOT routed here. `route_object_calls` already does
    // it, and does it properly -- it queues the ask so the object runs its own method in its OWN
    // scope, which is the only place the engine's helpers are in reach (see the note below about
    // exports). Rewriting the call to `.exports.M(` here instead reached the method but ran it in
    // the CALLER's scope, where a `toLocalState` inside it is an unknown variable -- and worse, it
    // hid the call from the router that would have done it right.
    strike_reads_of_a_struck_temp(&out)
}

/// Strike a read of a temp whose ASSIGNMENT was struck.
///
/// Where a call has no counterpart here its line is commented out -- but when that call was
/// what filled a temp, the temp keeps whatever it had, which is nothing, and the next line that
/// reaches into it throws. Emerald cave asks where a fighter is in the midground's own
/// coordinates, which this engine has no equivalent for, and then asks the cover art whether it
/// covers that point: the ask is struck, the point is nothing, and reading `.x` off it takes the
/// stage's update down every frame. A line built on a value that was never worked out cannot
/// run, so it is struck with the line it depended on.
/// A temp the function it sits in never writes.
///
/// Same rule as a temp whose write was struck, one step earlier: there was never a write to
/// strike. It reaches here when the source's own index is lost rebuilding the control flow, and
/// the read that is left names a temp the engine has never heard of -- so the frame stops there,
/// and everything the function had left to do stops with it. A test that cannot be worked out
/// cannot pass, and a statement built on one cannot do anything, which is what is said here.
/// A call reaching through a name the engine does not have.
///
/// `getViewRootContainer().parent.addChildAt(...)` walks back up the display list to put an
/// object at a chosen depth. The container is real and `addChildAt` is real, but `parent` is not
/// a name this engine's container has -- so the middle of the chain answers with nothing and the
/// call lands on it. Only the MIDDLE is judged, and only where it is a receiver for a further
/// call: reading a name off an engine handle is ordinary (`getRectangle().x`), and it is being
/// asked to DO something that makes it a chain the port cannot follow.
/// Building something nothing here can build.
///
/// By the time a script is written, every class the stage builds for itself has become a factory
/// in the same file, and the source language's own containers have become the shapes they are.
/// What is left named by `new` is a thing this engine has never had -- a bitmap, a display
/// object -- and the ask answers with nothing, so the statement goes, and the reads waiting on it
/// fall to the rule for a value that was never worked out.
///
/// The exceptions are the engine types the PORT builds, which are its own vocabulary rather than
/// anything read out of the source.
/// Asking a ported class for a name it does not have.
///
/// A class the stage builds for itself becomes a factory in the same file, and the port WROTE
/// that factory, so it knows exactly which names the thing ends up with. `clouds1Base.width` is
/// a display object's size, which came from art rather than from anything the class set -- the
/// structure has no such name, the read answers nothing, and the arithmetic around it takes the
/// frame. Nothing is guessed here: the factory is right there to be read.
/// The lacking pass, run until it finds nothing more: a field filled only from a name the
/// factory lacks (`it.stadiumScreen = grandstand.stadiumScreen`) is struck on the first run,
/// and only then reads as a field the factory never gives, so the reads through IT go on the
/// second (pokemon colosseum's screen setup).
fn asking_until_nothing_more_is_lacking(src: &str) -> String {
    let mut cur = src.to_string();
    for _ in 0..4 {
        let next = asking_a_ported_class_for_what_it_lacks(&cur);
        if next == cur { break; }
        cur = next;
    }
    cur
}

fn asking_a_ported_class_for_what_it_lacks(src: &str) -> String {
    use regex::Regex;
    use std::collections::{BTreeMap, BTreeSet};
    static MADE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static SETS: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    if !src.contains("__new(") && !src.contains("__clipGroup(") { return src.to_string() }
    let made = MADE.get_or_init(|| {
        regex::Regex::new(r"(?m)^\s*(?:var\s+)?([A-Za-z_]\w*)\s*=\s*(__clipGroup|[A-Za-z_]\w*__new)\(").expect("regex")
    });
    // what each factory leaves on the thing it builds
    // a field the factory only ever gives NOTHING (`it.k1HUDwrapper_b = null;`, an art child it
    // cannot build) is a name it lacks: a reach through it lands on null and throws
    let sets = SETS.get_or_init(|| Regex::new(r"(?m)^\s*it\.([A-Za-z_]\w*)\s*=\s*(?:(null)\s*;|[^;])").expect("sets regex"));
    let mut has: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for m in regex::Regex::new(r"(?ms)^function ([A-Za-z_]\w*__new)\([^)]*\) \{(.*?)^\}").expect("regex")
        .captures_iter(src)
    {
        let names: BTreeSet<&str> = sets.captures_iter(m.get(2).unwrap().as_str())
            .filter(|c| c.get(2).is_none())
            .map(|c| c.get(1).unwrap().as_str()).collect();
        has.insert(m.get(1).unwrap().as_str(), names);
    }
    // ...and the clip group the port builds for itself is the same kind of thing: a structure
    // with the names it was given. Fairy glade reaches into one for `globox.eye1.eye`, which is
    // art children -- there are none here, and the read takes the frame.
    if let Some(m) = crate::re!(r"(?ms)^function __clipGroup\([^)]*\) \{(.*?)^\}").captures(src)
    {
        let names: BTreeSet<&str> = crate::re!(r"(?m)^\s*g\.([A-Za-z_]\w*)\s*=")
            .captures_iter(m.get(1).unwrap().as_str())
            .map(|c| c.get(1).unwrap().as_str()).collect();
        if !names.is_empty() { has.insert("__clipGroup", names); }
    }
    if has.is_empty() { return src.to_string() }
    // which slot holds which ported class
    // a slot the stage ALSO fills from a surface by name (`goodegg_ground = __objectNamed(..)`)
    // is asked what a surface answers, not what the clip group under the same name does
    let by_name = crate::re!(r"\b([A-Za-z_]\w*)\s*=\s*__objectNamed\(");
    let handles: std::collections::BTreeSet<&str> = by_name.captures_iter(src).map(|c| c.get(1).unwrap().as_str()).collect();
    let of_class: BTreeMap<&str, &str> = made.captures_iter(src)
        .filter_map(|c| {
            let (slot, class) = (c.get(1).unwrap().as_str(), c.get(2).unwrap().as_str());
            (has.contains_key(class) && !handles.contains(slot)).then_some((slot, class))
        })
        .collect();
    // (no slot holding a class still leaves the factories' own reads to judge, below)
    let read = crate::re!(r"\b([A-Za-z_]\w*)\.([A-Za-z_]\w*)\b");
    // A HANDLE NEVER GIVEN IS NOTHING EVERYWHERE AFTER. A variable filled from a name the clip
    // lacks (`textBox = box.speech`) stays null, and every later write through it stops the
    // script; when no other line assigns it, those lines go the way of the one that filled it.
    let assign = crate::re!(r"^([A-Za-z_]\w*)\s*=\s*[^=]");
    let lacking_line = |t: &str| !t.starts_with("//") && read.captures_iter(t).any(|c| {
        of_class.get(&c[1]).and_then(|k| has.get(k))
            .is_some_and(|names| !names.contains(&c[2]))
    });
    // ...and INSIDE the factory, a member read through a field it never gives (`it.stadiumScreen
    // .mainBox`, the screen being an art child the port has no clip for) lands on null the same
    // way, whichever method of the class makes the read (pokemon colosseum's screen setup)
    let factory_spans: Vec<(usize, usize, &str)> = regex::Regex::new(r"(?ms)^function ([A-Za-z_]\w*__new)\([^)]*\) \{(.*?)^\}").expect("regex")
        .captures_iter(src)
        .map(|m| (m.get(0).unwrap().start(), m.get(0).unwrap().end(), m.get(1).unwrap().as_str()))
        .collect();
    let member_of_it = crate::re!(r"\bit\.([A-Za-z_]\w*)\s*[.\[]");
    let lacking_inside = |at: usize, t: &str| -> bool {
        if t.starts_with("//") { return false; }
        factory_spans.iter().find(|(a, b, _)| at >= *a && at < *b)
            .and_then(|(_, _, class)| has.get(class))
            .is_some_and(|names| member_of_it.captures_iter(t).any(|c| !names.contains(&c[1])))
    };
    // A WRITE TO WHERE A GROUP IS moves it: `hole.x = __sx(hole.x + 900)` is the source moving
    // its clip, and the group moves every piece by the same amount
    let src: &str = &{
        let moved = crate::re!(r"(?m)^(\s*)([A-Za-z_]\w*)\.(x|y)\s*=\s*([^;]+);\s*$");
        moved.replace_all(src, |c: &regex::Captures| {
            if of_class.get(&c[2]) == Some(&"__clipGroup") {
                format!("{}{}.set{}({});", &c[1], &c[2], c[3].to_ascii_uppercase(), &c[4])
            } else { c[0].to_string() }
        }).into_owned()
    };
    let dead: std::collections::BTreeSet<String> = src.lines().map(str::trim)
        .filter(|t| lacking_line(t))
        .filter_map(|t| assign.captures(t).map(|c| c[1].to_string()))
        .filter(|v| !src.lines().map(str::trim).any(|t| !lacking_line(t) && !t.starts_with("//")
            && assign.captures(t).is_some_and(|c| &c[1] == v.as_str())))
        .collect();
    let mut out = String::with_capacity(src.len());
    let mut at = 0usize;
    for line in src.lines() {
        let here = at;
        at += line.len() + 1;
        let t = line.trim();
        let lacks = lacking_line(t) || lacking_inside(here, t)
            || (!t.starts_with("//") && !t.starts_with("var ") && read.captures_iter(t).any(|c| dead.contains(&c[1])));
        if lacks {
            let indent = &line[..line.len() - line.trim_start().len()];
            let opener = t.ends_with('{')
                && ["if (", "while (", "} else if ("].iter().any(|k| t.starts_with(k));
            if opener {
                if let (Some(a), Some(b)) = (t.find('('), t.rfind(')')) {
                    if a < b {
                        out.push_str(&format!("{indent}{}false{} // [no such name on it] {}\n",
                            &t[..=a], &t[b..], &t[a + 1..b]));
                        continue;
                    }
                }
            }
            let net: i32 = t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum();
            if net == 0 {
                out.push_str(&format!("{indent}// [no such name on it] {t}\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn building_what_nothing_can_build(src: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    const PORT_BUILDS: [&str; 3] = ["VfxStats", "RgbaColorShader", "StructureStats"];
    let re = RE.get_or_init(|| Regex::new(r"\bnew ([A-Za-z_]\w*)\s*\(").expect("new regex"));
    if !src.contains("new ") { return src.to_string() }
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let t = line.trim();
        let unbuildable = !t.starts_with("//") && re.captures_iter(t).any(|c| {
            let name = &c[1];
            !PORT_BUILDS.contains(&name) && !src.contains(&format!("function {name}__new("))
        });
        let net: i32 = t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum();
        if unbuildable && net == 0 {
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push_str(&format!("{indent}// [nothing here can build one] {t}\n"));
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
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

fn a_class_named_as_a_value(src: &str) -> String {
    use regex::Regex;
    use std::collections::BTreeSet;
    static NAME: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static DECL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    // EVERY declaration on the line, not just the one that starts it. The port writes a row of
    // them at once -- `var l = ...; var t = ...; var r = ...;` -- and reading only the first left
    // the rest looking like names nothing declares, so bowsers castle's thwomp built its faces
    // out of nothing.
    let decl = DECL.get_or_init(|| {
        regex::Regex::new(r"\b(?:var|function)\s+([A-Za-z_]\w*)").expect("regex")
    });
    let mut declared: BTreeSet<&str> = decl.captures_iter(src)
        .map(|c| c.get(1).unwrap().as_str()).collect();
    // ...and what a loop names as it walks something
    for c in crate::re!(r"\bfor\s*\(\s*([A-Za-z_]\w*)\s+in\b")
        .captures_iter(src)
    {
        declared.insert(c.get(1).unwrap().as_str());
    }
    // A PARAMETER IS DECLARED BY BEING ONE. It is not a `var`, and read as undeclared every
    // function's own arguments would be answered with nothing the moment they were passed on.
    // every function's, named or not: the port writes plenty of anonymous ones, and their
    // arguments are as declared as any other
    for c in crate::re!(r"\bfunction\s*\w*\s*\(([^)]*)\)").captures_iter(src) {
        for p in c.get(1).unwrap().as_str().split(',') {
            let p = p.trim().trim_start_matches('?').split(':').next().unwrap_or("").trim();
            if !p.is_empty() { declared.insert(p); }
        }
    }
    // ...and a name that is not a class is a thing the source drew. Where the port has one it
    // declares a handle for it; where it has none there is nothing to give, and saying so is
    // better than a name the engine has never heard of. Saturn valley lists thirteen pieces of
    // its backdrop and the port found one.
    let name = NAME.get_or_init(|| {
        // the delimiter AFTER is checked by hand rather than matched: matching it would eat the
        // comma the next item needs, so `[a, b, c]` would only ever answer for every other one
        regex::Regex::new(r"([:,(]\s*)([A-Za-z_]\w*)").expect("regex")
    });
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        if line.trim_start().starts_with("//") || line.trim_start().starts_with("function ") {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        const THE_LANGUAGE: [&str; 12] = ["null", "true", "false", "this", "self", "super",
            "new", "function", "var", "return", "in", "cast"];
        let done = name.replace_all(line, |c: &regex::Captures| {
            let n = &c[2];
            if declared.contains(n) || crate::api_mappings::the_engine_answers_to(n)
                || THE_LANGUAGE.contains(&n)
            {
                return c[0].to_string();
            }
            // it has to BE the whole item: what follows is the end of it, not more of it
            let after = line[c.get(0).unwrap().end()..].trim_start();
            if !after.starts_with([',', ')', ']', '}', ';']) {
                return c[0].to_string();
            }
            // A TYPE IS NOT A VALUE. `function f(b:Bool)` wears the same shape as a key and its
            // value, and saying `b:"Bool"` there is a signature that means nothing. A value said
            // after a colon belongs to a structure, so there is a `{` open before it.
            let at = c.get(0).unwrap().start();
            if c[1].starts_with(':') && !line[..at].contains('{') {
                return c[0].to_string();
            }
            // a class is named the way classes are named, and a name is all that crosses of one;
            // anything else is a thing the port does not have, which is nothing
            if n.starts_with(|c: char| c.is_uppercase()) {
                format!("{}{:?}", &c[1], n)
            } else {
                format!("{}null /* [nothing here is called {n}] */", &c[1])
            }
        }).into_owned();
        out.push_str(&done);
        out.push('\n');
    }
    out
}

/// A loop whose body was struck out entirely.
///
/// Striking a statement takes its work away; striking EVERY statement in a loop takes away the
/// step that ends it, and what is left spins forever. The engine stops answering -- no error, no
/// crash, just a stage that never comes up, which is the most expensive way for a port to be
/// wrong. Butter building walks its segments that way, and every statement in the walk depends on
/// a blast zone the stage cannot reach.
///
/// A loop with nothing left to do cannot do anything, however many times it goes round. Said as a
/// test that cannot pass, so the braces stay where they are.
fn a_loop_with_nothing_left_to_do(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::with_capacity(src.len());
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        let opener = t.ends_with('{')
            && (t.starts_with("while (") || t.starts_with("for ("))
            && !t.contains("(false)");
        if opener {
            let mut depth = 1i32;
            let mut j = i + 1;
            let mut live = 0usize;
            while j < lines.len() && depth > 0 {
                let b = lines[j].trim();
                if !(b.starts_with("//") || b.is_empty()) {
                    let code = b.split("//").next().unwrap_or("").trim();
                    depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
                    if depth > 0 && !code.is_empty() && code != "}" { live += 1 }
                }
                j += 1;
            }
            if live == 0 {
                if let (Some(a), Some(b)) = (t.find('('), t.rfind(')')) {
                    if a < b {
                        let indent = &line[..line.len() - line.trim_start().len()];
                        out.push_str(&format!("{indent}{}false{} // [nothing left to do in here] {}\n",
                            &t[..=a], &t[b..], &t[a + 1..b]));
                        continue;
                    }
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn reads_of_a_slot_nothing_fills(src: &str) -> String {
    use regex::Regex;
    use std::collections::BTreeSet;
    static DECL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let decl = DECL.get_or_init(|| {
        regex::Regex::new(r"(?m)^\s*var ([A-Za-z_]\w*) = null;").expect("regex")
    });
    let names: Vec<String> = decl.captures_iter(src).map(|c| c[1].to_string()).collect();
    if names.is_empty() { return src.to_string() }
    let live: String = src.lines()
        .filter(|l| !l.trim_start().starts_with("//") && !l.trim_start().starts_with("var "))
        .collect::<Vec<_>>().join("\n");
    // GIVING IT NOTHING IS NOT FILLING IT. `_v7 = null;` is how the port says a thing had no
    // counterpart, and a name read off that is as much nothing as if it had never been assigned
    // at all -- world tournament reads its blimp out of one.
    let empty: BTreeSet<&str> = names.iter()
        .filter(|n| {
            // an assignment, not a comparison: what fills it may itself compare (the clip
            // binding asks `__objectNamed(n) != null` before it picks)
            let re = Regex::new(&format!(r"\b{}\s*=([^=;][^;]*);", regex::escape(n)));
            let filled = re.map(|re| re.captures_iter(&live)
                .any(|c| c[1].trim() != "null" && !c[1].trim().is_empty()))
                .unwrap_or(true);
            !filled
        })
        .map(|n| n.as_str()).collect();
    if empty.is_empty() { return src.to_string() }
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let t = line.trim();
        let indent = &line[..line.len() - line.trim_start().len()];
        let reads = !t.starts_with("//") && !line.starts_with("var ") && empty.iter().any(|n| {
            Regex::new(&format!(r"\b{}\.", regex::escape(n)))
                .map(|re| re.is_match(t)).unwrap_or(false)
        });
        if reads {
            let net: i32 = t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum();
            let opener = t.ends_with('{')
                && ["if (", "while (", "} else if ("].iter().any(|k| t.starts_with(k));
            if opener {
                if let (Some(a), Some(b)) = (t.find('('), t.rfind(')')) {
                    if a < b {
                        out.push_str(&format!("{indent}{}false{} // [nothing ever fills it] {}\n",
                            &t[..=a], &t[b..], &t[a + 1..b]));
                        continue;
                    }
                }
            }
            if net == 0 {
                out.push_str(&format!("{indent}// [nothing ever fills it] {t}\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Asking a placed object for something the engine has no name for.
///
/// `__objectNamed("jankWall")` hands back the engine's own handle for a thing the stage placed,
/// so it answers to the engine's api and to nothing else. The source asks it to ignore an object
/// list, which is SSF2's own idea, and the ask takes the frame.
/// A write to a setter runs the setter, from outside the class as much as from inside it.
///
/// A ported class names its setters on its first line, and a slot holding one of its objects
/// that is written by that name is calling it: `m_spaceshipActivateTimer.duration = 30 * 80`
/// sets the timer's own duration field through the setter's body, which is what the source
/// did. Run LAST: the write reaches its final spelling only after the passes that fold the
/// decompiler's temporaries away, and matched earlier it is not there to match.
fn a_write_to_a_setter_runs_it(src: &str) -> String {
    use std::collections::{BTreeMap, BTreeSet};
    if !src.contains("// setters: ") { return src.to_string() }
    let setters_of: BTreeMap<&str, BTreeSet<&str>> = crate::re!(r"(?m)^function ([A-Za-z_]\w*__new)\([^)]*\) \{\n\s*var it = \{\};\n\s*// setters: ([\w,]+)").captures_iter(src)
        .map(|c| (c.get(1).unwrap().as_str(), c.get(2).unwrap().as_str().split(',').collect()))
        .collect();
    if setters_of.is_empty() { return src.to_string() }
    let made = crate::re!(r"(?m)^\s*(?:var\s+)?([A-Za-z_]\w*)\s*=\s*([A-Za-z_]\w*__new)\(");
    let of_class: Vec<(&str, &str)> = made.captures_iter(src)
        .filter_map(|c| {
            let (slot, class) = (c.get(1).unwrap().as_str(), c.get(2).unwrap().as_str());
            setters_of.contains_key(class).then_some((slot, class))
        })
        .collect();
    let mut s = src.to_string();
    for (slot, class) in of_class {
        for n in &setters_of[class] {
            // wherever on the line it sits: a guarded write is still on one line with its
            // guard here (`if (t != null) { t.duration = 30 * 80; }`), so no line anchor
            let re = crate::dyn_re(&format!(r"\b(?:self\.)?{}\.{}\s*=\s*([^=][^;]*);",
                regex::escape(slot), regex::escape(n)));
            s = re.replace_all(&s, format!("{slot}.__set_{n}(${{1}});")).to_string();
        }
    }
    s
}

fn asking_a_placed_object_for_nothing(src: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    if !src.contains("__objectNamed(") { return src.to_string() }
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"__objectNamed\([^()]*\)\.(\w+)\(").expect("regex")
    });
    static LOOKUP: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let lookup = LOOKUP.get_or_init(|| {
        regex::Regex::new(r#"__objectNamed\("[^"]*"\)"#).expect("regex")
    });
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let t = line.trim();
        let indent = &line[..line.len() - line.trim_start().len()];
        let unanswered = !t.starts_with("//") && re.captures_iter(t)
            .any(|c| !crate::api_mappings::the_engine_answers_to(&c[1]) && !the_port_answers_to(&c[1]));
        if unanswered {
            let name = re.captures_iter(t)
                .find(|c| !crate::api_mappings::the_engine_answers_to(&c[1]) && !the_port_answers_to(&c[1]))
                .map(|c| c[1].to_string()).unwrap_or_default();
            let net: i32 = t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum();
            let opener = t.ends_with('{')
                && ["if (", "while (", "} else if ("].iter().any(|k| t.starts_with(k));
            if opener {
                if let (Some(a), Some(b)) = (t.find('('), t.rfind(')')) {
                    if a < b {
                        out.push_str(&format!("{indent}{}false{} // [nothing placed answers to {name}] {}\n",
                            &t[..=a], &t[b..], &t[a + 1..b]));
                        continue;
                    }
                }
            }
            if net == 0 {
                out.push_str(&format!("{indent}// [nothing placed answers to {name}] {t}\n"));
                continue;
            }
        }
        // ...AND THE OBJECT MAY NOT BE THERE AT ALL. The ask is one the engine answers, but the
        // stage names something this package did not place -- warioware turns terrain on and off
        // by a name with nothing behind it -- so the ask lands on nothing all the same. Every
        // other reach for a placed object is guarded; this one is now too.
        let asks_a_name = !t.starts_with("//") && re.is_match(t);
        let net: i32 = t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum();
        if asks_a_name && net == 0 && t.ends_with(';') {
            if let Some(m) = lookup.find(t) {
                out.push_str(&format!("{indent}if ({} != null) {{ {t} }}\n", m.as_str()));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

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

/// A pair of coordinates handed straight to a call.
///
/// The engine takes its own point and nothing else, and it will only make one while the object is
/// still being set up -- so a pair written out at the call itself, `getAngleBetween({x: .., y:
/// ..}, p)`, cannot be made there. One point per place in the code is made at setup instead and
/// filled in as the line runs: the value is used and done with before the line ends, so the same
/// point serves that place every time without ever being two things at once.
fn a_pair_written_at_the_call(src: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    if !src.contains("{ x: ") { return src.to_string() }
    // a PAIR: two names and nothing else. A rectangle copied carries its width and height too,
    // and reading the y up to the closing brace swallowed them (yoshi's island's goonie path).
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"([(,]\s*)\{ x: ((?:[^{}(),]|\((?:[^()]|\([^()]*\))*\))+?), y: ((?:[^{}(),]|\((?:[^()]|\([^()]*\))*\))+?) \}").expect("regex")
    });
    // ONLY WHERE THE ENGINE IS THE ONE ASKING. A pair handed to an engine call is used and done
    // with before the line ends, so one point can serve that place every time. A pair handed to
    // anything else may be KEPT -- bowsers castle pushes one per parallax rate into a list -- and
    // then every one of them would be the same point, which is a different stage entirely.
    let enclosing_call = |line: &str, at: usize| -> Option<String> {
        let b = line.as_bytes();
        let mut depth = 0i32;
        let mut i = at;
        while i > 0 {
            i -= 1;
            match b[i] {
                b')' => depth += 1,
                b'(' if depth > 0 => depth -= 1,
                b'(' => {
                    let name_end = i;
                    let mut j = name_end;
                    while j > 0 && (b[j - 1].is_ascii_alphanumeric() || b[j - 1] == b'_') { j -= 1 }
                    return Some(line[j..name_end].to_string());
                }
                _ => {}
            }
        }
        None
    };
    let mut n = 0usize;
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let done = re.replace_all(line, |c: &regex::Captures| {
            // from the pair's own brace: from the match's start, which for a first argument is
            // the call's own paren, the scan walked out to the call around it, and the engine's
            // angle helper read as the object's own bounce
            let at = c.get(0).unwrap().start() + c[1].len();
            // ...and not by a list keeping it: a pair pushed is kept, and one point serving
            // every push would make the whole path one place
            let asked_by_the_engine = enclosing_call(line, at)
                .is_some_and(|name| crate::api_mappings::the_engine_answers_to(&name)
                    && !matches!(name.as_str(), "push" | "unshift" | "insert" | "concat" | "splice" | "set" | "add"));
            if !asked_by_the_engine { return c[0].to_string() }
            n += 1;
            format!("{}__atPoint(__pt{n}, {}, {})", &c[1], &c[2], &c[3])
        }).into_owned();
        out.push_str(&done);
        out.push('\n');
    }
    let out = out;
    if n == 0 { return src.to_string() }
    let mut head = String::new();
    for i in 1..=n { head.push_str(&format!("var __pt{i} = self.makePoint(0, 0);\n")); }
    head.push_str("function __atPoint(p, x, y) { p.x = x; p.y = y; return p; }\n");
    format!("{head}{out}")
}

fn reaching_into_nothing(src: &str) -> String {
    if !src.contains("null[") && !src.contains("null(") { return src.to_string() }
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let t = line.trim();
        let indent = &line[..line.len() - line.trim_start().len()];
        // ...and no calling it either. Where the thing being CALLED had no counterpart it was
        // answered with nothing, and the call stayed: `null("transition3")` asks nothing to do
        // something, which takes the frame the same way reaching into it does.
        if !t.starts_with("//") && (t.contains("null[") || t.contains("null(")) {
            let net: i32 = t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum();
            let opener = t.ends_with('{')
                && ["if (", "while (", "} else if ("].iter().any(|k| t.starts_with(k));
            if opener {
                if let (Some(a), Some(b)) = (t.find('('), t.rfind(')')) {
                    if a < b {
                        out.push_str(&format!("{indent}{}false{} // [nothing to reach into] {}\n",
                            &t[..=a], &t[b..], &t[a + 1..b]));
                        continue;
                    }
                }
            }
            if net == 0 {
                out.push_str(&format!("{indent}// [nothing to reach into] {t}\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// What the port's OWN handles answer to: a named surface (`__platHandle`), a boundary region
/// (`__objectBoundary`) and a clip group (`__clipGroup`) each carry these, so a call on one is
/// not a call on nothing even though the engine has no such name.
fn the_port_answers_to(name: &str) -> bool {
    matches!(name, "addIgnoreObject" | "removeIgnoreObject" | "clearIgnoreObject" | "setIgnoreObjectListInversed"
        | "updateStructureStats" | "setVisible" | "setAlpha" | "getViewRootContainer" | "playFrameLabel"
        | "getX" | "getY" | "setX" | "setY")
}

fn asking_the_answer_of_nothing(src: &str) -> String {
    use regex::Regex;
    use std::collections::BTreeSet;
    static DEF: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let def = DEF.get_or_init(|| Regex::new(r"(?m)^function (\w+)\s*\(").expect("def regex"));
    let mine: BTreeSet<&str> = def.captures_iter(src).map(|c| c.get(1).unwrap().as_str()).collect();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"\.(\w+)\((?:[^()]|\([^()]*\))*\)\.(\w+)\(").expect("regex")
    });
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let t = line.trim();
        let indent = &line[..line.len() - line.trim_start().len()];
        let broken = !t.starts_with("//") && re.captures_iter(t).any(|c| {
            !mine.contains(&c[1]) && !crate::api_mappings::the_engine_answers_to(&c[1]) && !the_port_answers_to(&c[1])
        });
        if broken {
            let net: i32 = t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum();
            let opener = t.ends_with('{')
                && ["if (", "while (", "} else if ("].iter().any(|k| t.starts_with(k));
            if opener {
                if let (Some(a), Some(b)) = (t.find('('), t.rfind(')')) {
                    if a < b {
                        out.push_str(&format!("{indent}{}false{} // [nothing to ask] {}\n",
                            &t[..=a], &t[b..], &t[a + 1..b]));
                        continue;
                    }
                }
            }
            if net == 0 {
                out.push_str(&format!("{indent}// [asks the answer of a question nothing answers] {t}\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn calls_reaching_through_nothing(src: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        // the third part may be a call or a plain read: what makes the middle broken is that
        // something is asked of it at all, not what kind of asking it is
        regex::Regex::new(r"\.(\w+)\(\)\.(\w+)\.(\w+)").expect("regex")
    });
    if !src.contains(")(") && !src.contains("().") { return src.to_string() }
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let t = line.trim();
        let broken = !t.starts_with("//") && re.captures_iter(t).any(|c| {
            crate::api_mappings::the_engine_answers_to(&c[1])
                && !crate::api_mappings::the_engine_answers_to(&c[2])
        });
        if broken {
            let indent = &line[..line.len() - line.trim_start().len()];
            // A LINE THAT OPENS AND CLOSES ITS OWN BLOCK TAKES ALL OF IT WITH IT. The guard around
            // the call is still on the same line at this point, so a rule that only strikes lines
            // without braces would leave every guarded one standing; and one that struck any line
            // with braces would orphan a `}` further down. Net zero says the line is whole.
            let net: i32 = t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum();
            if net == 0 {
                out.push_str(&format!("{indent}// [reaches through a name the engine does not have] {t}\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn reads_of_a_local_nothing_writes(src: &str) -> String {
    use regex::Regex;
    use std::collections::BTreeSet;
    static ANY: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static SET: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let any = ANY.get_or_init(|| Regex::new(r"_v\d+").expect("temp regex"));
    // GIVING IT NOTHING IS NOT WRITING TO IT. `_v7 = null;` is how the port says a thing had no
    // counterpart, and a name read off that is as much nothing as if it had never been written --
    // world tournament reads its blimp out of one and then reaches three names deep into it.
    let set = SET.get_or_init(|| {
        regex::Regex::new(r"(?:var\s+)?(_v\d+)\s*=\s*([^=;]*);").expect("regex")
    });
    if !src.contains("_v") { return src.to_string() }
    let lines: Vec<&str> = src.lines().collect();
    // the functions, so a temp is judged where it lives rather than across the whole file
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut open: Option<usize> = None;
    let mut depth = 0i32;
    for (i, l) in lines.iter().enumerate() {
        if open.is_none() && l.starts_with("function ") { open = Some(i); depth = 0; }
        if open.is_some() {
            let live = if l.trim_start().starts_with("//") { "" } else { l.split("//").next().unwrap_or("") };
            depth += live.matches('{').count() as i32 - live.matches('}').count() as i32;
            if depth <= 0 && i > open.unwrap() { spans.push((open.unwrap(), i)); open = None; }
        }
    }
    let mut dead_in: Vec<BTreeSet<String>> = Vec::new();
    for &(a, b) in &spans {
        let live: String = lines[a..=b].iter()
            .filter(|l| !l.trim_start().starts_with("//"))
            .copied().collect::<Vec<_>>().join("\n");
        let written: BTreeSet<&str> = set.captures_iter(&live)
            .filter(|c| c[2].trim() != "null" && !c[2].trim().is_empty())
            .map(|c| c.get(1).unwrap().as_str()).collect();
        if let Ok(w) = std::env::var("PEPTIDE_TRACE_LINE") {
            if live.contains(&w) {
                eprintln!("[trace span] {a}..{b} head={:?} written={:?}", lines[a].chars().take(60).collect::<String>(), written);
            }
        }
        dead_in.push(any.find_iter(&live).map(|m| m.as_str())
            .filter(|v| !written.contains(v)).map(str::to_string).collect());
    }
    if dead_in.iter().all(|d| d.is_empty()) { return src.to_string() }
    let mut out = String::with_capacity(src.len());
    for (i, line) in lines.iter().enumerate() {
        let dead = spans.iter().position(|&(a, b)| i >= a && i <= b)
            .map(|k| &dead_in[k]).filter(|d| !d.is_empty());
        let t = line.trim();
        // a line that DECLARES the temp is not a line that reads it: `var _v5 = null;` is where
        // the port says it has nothing, and striking that says nothing about anything
        // a line that GIVES the temp something is not a line that reads it, whether it declares it
        // first or not: what is looked for is the name on the right of the equals.
        let reads = dead.is_some_and(|d| !t.starts_with("//") && d.iter().any(|v| {
            let rhs = match t.split_once('=') {
                Some((lhs, rhs)) if lhs.trim().trim_start_matches("var ").trim() == v => rhs,
                _ => t,
            };
            any.find_iter(rhs).any(|m| m.as_str() == v)
        }));
        if reads {
            let indent = &line[..line.len() - line.trim_start().len()];
            let opener = t.ends_with('{')
                && ["if (", "while (", "} else if (", "for ("].iter().any(|k| t.starts_with(k));
            if opener {
                if let (Some(a), Some(b)) = (t.find('('), t.rfind(')')) {
                    if a < b {
                        out.push_str(&format!("{indent}{}false{} // [never worked out] {}\n",
                            &t[..=a], &t[b..], &t[a + 1..b]));
                        continue;
                    }
                }
            }
            // net zero says the line opens and closes whatever it opens, so striking it takes all
            // of it: a pair of coordinates written out in an argument is a brace that closes.
            let net: i32 = t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum();
            if net == 0 {
                out.push_str(&format!("{indent}// [reads a value that was never worked out] {t}\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn strike_reads_of_a_struck_temp(src: &str) -> String {
    use std::collections::BTreeSet;
    let assign = crate::re!(r"^\s*//.*?(_v\d+) = ");
    let refill = crate::re!(r"^\s*(_v\d+) = ");
    let mut dead: BTreeSet<String> = BTreeSet::new();
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        // a fresh assignment brings it back to life
        if let Some(c) = refill.captures(line) { dead.remove(&c[1]); }
        if let Some(c) = assign.captures(line) { dead.insert(c[1].to_string()); }
        let reads_dead = !line.trim_start().starts_with("//")
            && dead.iter().any(|v| line.contains(&format!("{v}.")));
        if reads_dead {
            let indent = &line[..line.len() - line.trim_start().len()];
            // A LINE THAT OPENS A BLOCK KEEPS ITS BRACES. Commenting out `if (cond) {` takes the
            // `{` with it and leaves the matching `}` an orphan, which costs the whole script.
            // The condition is what could not be worked out, so that is what goes: a test on a
            // value nothing ever set cannot pass, and `false` says so while the shape stays put.
            let t = line.trim();
            let opener = t.ends_with('{')
                && ["if (", "while (", "} else if (", "for ("].iter().any(|k| t.starts_with(k));
            if opener {
                if let (Some(a), Some(b)) = (t.find('('), t.rfind(')')) {
                    if a < b {
                        out.push_str(&format!("{indent}{}false{} // [never worked out] {}\n",
                            &t[..=a], &t[b..], &t[a + 1..b]));
                        continue;
                    }
                }
            }
            if t.contains('{') || t.contains('}') {
                out.push_str(line);
                out.push('\n');
                continue;
            }
            out.push_str(&format!("{indent}// [reads a value that was never worked out] {t}\n"));
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}



/// `self.Type(expr)` -> `expr`, for one type name. The argument can hold nested calls and
/// brackets, so its end is counted rather than searched for.
fn unwrap_cast_to(src: &str, ty: &str) -> String {
    let needle = format!("self.{ty}(");
    if !src.contains(&needle) { return src.to_string() }
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(at) = rest.find(&needle) {
        out.push_str(&rest[..at]);
        let from = at + needle.len();
        let mut depth = 1i32;
        let mut end = None;
        for (i, c) in rest[from..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => { depth -= 1; if depth == 0 { end = Some(from + i); break } }
                _ => {}
            }
        }
        let Some(close) = end else { out.push_str(&rest[at..]); return out };
        out.push_str(&rest[from..close]);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Comment out a line that only CLOSES what nothing opened.
///
/// A call reconstructed over several lines can leave its tail behind when its head is rewritten
/// or struck: a bare `null);` sitting where a call used to be. Such a line cannot be part of any
/// valid script -- it closes a bracket that was never opened -- and the engine does not skip the
/// line, it drops the WHOLE script it is in, so one of them costs the stage every object its
/// script was going to spawn. Twenty stages carried one.
fn strike_orphan_closers(src: &str) -> String {
    fn code_of(l: &str) -> &str { l.split("//").next().unwrap_or("") }
    let mut depth = 0i32;
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let code = code_of(line);
        let opens = code.matches('(').count() as i32 - code.matches(')').count() as i32;
        // A LINE THAT IS NOTHING BUT A CLOSE is garbage whatever the running count says. Counting
        // brackets from the top of the file is thrown off by one inside a string, and these files
        // are long enough for that to happen -- twenty-two of them kept a bare `null);` that the
        // count said was fine. `null);` and `);` are not statements in any language: they are the
        // tail of a call whose head was rewritten, and nothing else.
        let bare = code.trim();
        let only_close = !bare.is_empty()
            && bare.trim_end_matches(';').trim_end_matches(')').trim_end().is_empty()
            || bare.trim_end_matches(';').trim_end_matches(')').trim() == "null";
        if only_close && !code.contains('(') {
            out.push_str(&format!("{}// [closes nothing: the call it ended was rewritten] {}\n",
                &line[..line.len() - line.trim_start().len()], bare));
            continue;
        }
        if depth + opens < 0 && code.trim_start().starts_with(|c: char| c == ')' || c.is_alphanumeric())
            && code.trim_end().ends_with(");")
            && !code.contains('(')
        {
            out.push_str(&format!("{}// [closes nothing: the call it ended was rewritten] {}\n",
                &line[..line.len() - line.trim_start().len()], line.trim()));
            continue;
        }
        depth = (depth + opens).max(0);
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// SSF2's `createSelfPlatform`: the object declares a solid box of itself, and standing on it is
/// how a stage's terrain-bearing objects work -- bomb factory's blocks ARE the floor. Fraymakers
/// has no per-object platform, so the box becomes a real structure made at the class's own call
/// site, blacklisted against its maker (an SSF2 self-platform never grounds the body that owns
/// it). hscript can't hold a handle across frames, so the structure's index is what is kept.
///
/// A class that ENDS ITS OWN LIFE releases what it reserved on the way out. The source does both
/// in one step, so the release cannot be missed. Here the release is a queued ask and the destroy
/// takes effect at once, so an object that leaves the match any OTHER way never releases, and
/// whatever it held stays held for the rest of the match.
///
/// So the statements that run immediately before the destroy are hoisted into one function, called
/// from the destroy site AND from `onTeardown`, guarded to run exactly once. Only applied when every
/// destroy in the class is preceded by the SAME statements: that is what makes them the class's
/// cleanup rather than something particular to one branch.
fn end_of_life_cleanup(script: &str) -> String {
    let lines: Vec<&str> = script.lines().collect();
    let is_destroy = |t: &str| t == "self.kill();" || t == "self.destroy();";
    let is_ask = |t: &str| t.starts_with("self.exports.__stage.exports.__q.push(") && t.ends_with(");");
    // each destroy, with the contiguous run of asks directly above it
    let mut runs: Vec<(usize, usize)> = Vec::new(); // [start, end) of the ask run
    for (i, l) in lines.iter().enumerate() {
        if !is_destroy(l.trim()) { continue; }
        let mut start = i;
        while start > 0 && is_ask(lines[start - 1].trim()) { start -= 1; }
        if start == i { return script.to_string(); } // a destroy with no cleanup: nothing to hoist
        runs.push((start, i));
    }
    if runs.is_empty() { return script.to_string(); }
    let body: Vec<String> = lines[runs[0].0..runs[0].1].iter().map(|l| l.trim().to_string()).collect();
    for (a, b) in &runs {
        let other: Vec<String> = lines[*a..*b].iter().map(|l| l.trim().to_string()).collect();
        if other != body { return script.to_string(); } // branches differ: not one cleanup
    }
    if script.contains("function __endOfLife(") { return script.to_string(); }
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        if let Some((_, b)) = runs.iter().find(|(a, _)| *a == i).copied() {
            let indent: String = lines[b].chars().take_while(|c| *c == '\t' || *c == ' ').collect();
            out.push(format!("{indent}__endOfLife();"));
            i = b;
            continue;
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    let mut s = out.join("\n");
    if !s.ends_with('\n') { s.push('\n'); }
    s.push_str("\nvar _s_eol = self.makeInt(0);\n");
    s.push_str("// what this object gives back when it goes, however it goes, and only once.\n");
    s.push_str("function __endOfLife() {\n\tif (_s_eol.get() == 1) { return; }\n\t_s_eol.set(1);\n");
    for b in &body { s.push_str(&format!("\t{b}\n")); }
    s.push_str("}\n");
    // the release runs on TEARDOWN: the object is on its way out whatever took it there, so this
    // is the one place that sees every ending, including the ones its own code never reaches.
    if s.contains("function onTeardown() {}") {
        s = s.replace("function onTeardown() {}", "function onTeardown() {\n\t__endOfLife();\n}");
    } else if let Some(at) = s.find("function onTeardown() {") {
        let ins = at + "function onTeardown() {".len();
        s.insert_str(ins, "\n\t__endOfLife();");
    } else {
        s.push_str("\nfunction onTeardown() {\n\t__endOfLife();\n}\n");
    }
    s
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

/// The stage-script table of a named clip's frame scripts and the runtime that fires each one as
/// the clip's pieces reach its frame: `self.<child>.fallthrough = v` on a child goes to that child's
/// surface (its structures' `disabled` stat) or object (its body); a hold or jump is the pieces'
/// own; anything else the script does is kept as a comment.
fn plane_frame_scripts(model: &StageModel, body: &str) -> String {
    let fall = crate::re!(r"^\s*self\.(\w+)\.fallthrough\s*=\s*([^;]+);\s*$");
    let vis = crate::re!(r"^\s*self\.(\w+)\.visible\s*=\s*([^;]+);\s*$");
    // the stage's own method, called from its plane: the stage script defines it by that name
    let stage_call = crate::re!(r"^\s*SSF2API\.getStage\(\)\.(\w+)\((.*)\);\s*$");
    let own = crate::re!(r"^\s*(?:self\.)?(?:stop\(\)|gotoAndStop\([^)]*\)|gotoAndPlay\([^)]*\))\s*;\s*$");
    let structural = crate::re!(r"^\s*(?:\}\s*else\s*)?(?:if\s*\((.*)\)\s*\{|\}\s*else\s*\{|\})\s*$");
    // what a condition on the plane asks of the game, answered by what the stage script holds
    let answer = |cond: &str| -> String {
        let c = cond.replace("SSF2API.isHazardsOn()", "m_hazardsOn")
            .replace("SSF2API.isReady()", "true")
            .replace("__truthy(parent)", "true").replace("__truthy(true)", "true");
        if c.contains("SSF2API.") || c.contains("self.") { "false".to_string() } else { c }
    };
    let has_visible_helper = body.contains("function __objectVisible(");
    let mut entries: Vec<String> = Vec::new();
    for (name, label, k, code) in &model.clip_frame_scripts {
        let mut lines: Vec<String> = Vec::new();
        let mut ported = false;
        for line in code.lines().skip_while(|l| !l.starts_with("function ")).skip(1).take_while(|l| !l.starts_with('}')) {
            let t = line.trim();
            if t.is_empty() || t == "return;" || own.is_match(t) { continue; }
            if let Some(c) = fall.captures(t) {
                lines.push(format!("\t\t__setFallthrough(\"{}\", {});", &c[1], c[2].trim()));
                ported = true;
            } else if let Some(c) = vis.captures(t) {
                lines.push(format!("\t\t__setVisibleNamed(\"{}\", {});", &c[1], c[2].trim()));
                ported = true;
            } else if let Some(c) = stage_call.captures(t) {
                if body.contains(&format!("function {}(", &c[1])) {
                    lines.push(format!("\t\t{}({});", &c[1], c[2].trim()));
                    ported = true;
                } else {
                    lines.push(format!("\t\t// [plane frame script: the stage script has no such method] {t}"));
                }
            } else if let Some(c) = structural.captures(t) {
                // the block's shape is kept whatever is inside it; a test the stage cannot answer
                // is false, so its block is dead rather than broken
                let s = match c.get(1) {
                    Some(cond) => t.replacen(cond.as_str(), &answer(cond.as_str()), 1),
                    None => t.to_string(),
                };
                lines.push(format!("\t\t{s}"));
            } else {
                lines.push(format!("\t\t// [plane frame script] {t}"));
            }
        }
        if !ported { continue; }
        entries.push(format!("{{ p: \"{name}\", l: \"{}\", k: {k}, run: function() {{\n{}\n\t}} }}", label.replace('"', ""), lines.join("\n")));
    }
    if entries.is_empty() { return String::new() }
    let visible_helper = format!("function __setVisibleNamed(n, v) {{\n\
\tvar g = __groupNamed(n);\n\
\tif (g != null && g.setVisible != null) {{ g.setVisible(v); return; }}\n\
{}}}\n", if has_visible_helper { "\t__objectVisible(n, v);\n" } else { "" });
    format!("// what a named clip's own timeline does as it plays: each frame script runs once when the\n\
// clip's pieces reach its label and frame\n\
var __planeScripts = [{}];\n\
var __planeLast = [];\n\
{visible_helper}\
// a write to an object whose script has not run yet (the plane's first frame fires before the\n\
// objects it places have published their bodies) waits for it\n\
var __fallLater = [];\n\
function __setFallthrough(n, v) {{\n\
\tvar o = __objectNamed(n);\n\
\t// a surface's handle answers for its structures; an object answers for its body once its\n\
\t// script has run, and one that runs without a body has nothing to flip. an object under\n\
\t// the name that carries no body of its own (a section's picture, bound under the section's\n\
\t// name) leaves the write to the structures filed under that name: pokemon colosseum's\n\
\t// steel and bug grounds stayed solid through every section while their pictures took it\n\
\tif (o != null && o.getX != null && !(o.exports != null && o.exports.__deck != null && o.parts == null)) {{\n\
\t\tif (o.exports == null) {{ __fallLater.push({{ n: n, v: v }}); return; }}\n\
\t\tif (o.exports.__setDisabled != null) {{ o.exports.__setDisabled(v); return; }}\n\
\t}}\n\
\t__platStats(n, {{ disabled: v }});\n\
}}\n\
function __planeScriptsUpdate() {{\n\
\tif (__fallLater.length > 0) {{\n\
\t\tvar w = __fallLater; __fallLater = [];\n\
\t\tvar wi = 0;\n\
\t\twhile (wi < w.length) {{ __setFallthrough(w[wi].n, w[wi].v); wi = wi + 1; }}\n\
\t}}\n\
\tvar i = 0;\n\
\twhile (i < __planeScripts.length) {{\n\
\t\tvar e = __planeScripts[i];\n\
\t\tvar g = __groupNamed(e.p);\n\
\t\tvar p = (g != null && g.parts != null) ? __firstPart(g) : null;\n\
\t\tif (p != null) {{\n\
\t\t\tvar l = (p.exports != null && p.exports.__label != null) ? p.exports.__label : p.getAnimation();\n\
\t\t\tvar k = Std.int((p.getCurrentFrame() - 1) / 2);\n\
\t\t\tvar stamp = l + \":\" + k;\n\
\t\t\tvar j = 0; var seen = null;\n\
\t\t\twhile (j < __planeLast.length) {{ if (__planeLast[j].p == e.p) {{ seen = __planeLast[j]; }} j = j + 1; }}\n\
\t\t\tif (seen == null) {{ seen = {{ p: e.p, s: \"\", ran: [] }}; __planeLast.push(seen); }}\n\
\t\t\tif (seen.s != stamp) {{ seen.s = stamp; seen.ran = []; }}\n\
\t\t\tif (e.l == l && e.k == k && seen.ran.indexOf(i) < 0) {{ seen.ran.push(i); e.run(); }}\n\
\t\t}}\n\
\t\ti = i + 1;\n\
\t}}\n\
}}\n", entries.join(", "))
}

fn stage_value_chains(script: &str) -> Vec<String> {
    crate::re!(r"self\.exports\.__stage\.((?:\w+\([^()]*\))(?:\.(?:\w+\([^()]*\)|\w+))*)")
        .captures_iter(script)
        // Something the stage KNOWS, never something it DOES. A chain ending in a call is an action;
        // so is one whose last name is followed by `(` in the source, which reads as a property here
        // but is a method being called with arguments. Publishing either would perform it, once a
        // frame, forever.
        .filter(|c| {
            let chain = &c[1];
            let after = &script[c.get(1).expect("chain").end()..];
            !chain.ends_with(')') && !after.starts_with('(')
        })
        .map(|c| c[1].to_string())
        .collect()
}

/// A safe identifier for one such chain, so both sides agree on the name without sharing anything
/// but the text of the chain itself.
fn chain_key(chain: &str) -> String {
    format!("__v_{}", chain.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>())
}

fn stage_own_methods(model: &StageModel) -> Vec<String> {
    crate::re!(r"(?m)^function\s+(\w+)\s*\(")
        .captures_iter(model.stage_script.as_deref().unwrap_or(""))
        .map(|c| c[1].to_string())
        .collect()
}


/// `self.exports.X[expr]` where the script assigns `self.exports.X = { k1: .., k2: .. }` becomes
/// `__get_X(expr)`, a getter that switches over the literal's own keys. Only the keys the class
/// wrote exist, so the getter is the whole object, and a key it never wrote reads as nothing.

/// A STRING GROWS BY +: a local that starts as a string literal and is then `.concat(x)`-ed is
/// a string (kingdom1 builds its font map that way); this engine's concat is arrays only.
fn string_concat_is_plus(body: &str) -> String {
    let str_locals: std::collections::BTreeSet<String> = crate::re!(r#"(?m)^\s*(?:var\s+)?([A-Za-z_]\w*)\s*=\s*"[^"\n]*"\s*;"#)
        .captures_iter(body).map(|c| c[1].to_string()).collect();
    let mut out = body.to_string();
    for v in str_locals {
        let re = crate::dyn_re(&format!(r"\b{}\.concat\(((?:[^()]|\([^()]*\))*)\)", regex::escape(&v)));
        out = re.replace_all(&out, format!("({v} + ($1))").as_str()).into_owned();
    }
    out
}

fn index_a_known_literal(body: &str) -> String {
    let assign = crate::re!(r"(?m)self\.exports\.([A-Za-z_]\w*)\s*=\s*\{");
    let mut out = body.to_string();
    let mut getters = String::new();
    let names: Vec<String> = assign.captures_iter(body).map(|c| c[1].to_string()).collect();
    for name in names {
        if !out.contains(&format!("self.exports.{name}[")) { continue }
        // the literal's top-level keys, read by walking its braces
        let Some(m) = crate::re!(&format!(r"self\.exports\.{}\s*=\s*\{{", regex::escape(&name))).find(body) else { continue };
        let rest = &body[m.end()..];
        let (mut depth, mut i, mut keys, mut at_key) = (1i32, 0usize, Vec::<String>::new(), true);
        let b = rest.as_bytes();
        while i < b.len() && depth > 0 {
            let c = b[i] as char;
            match c {
                '{' | '[' | '(' => { depth += 1; at_key = false; }
                '}' | ']' | ')' => { depth -= 1; if depth == 1 { at_key = false; } }
                ',' if depth == 1 => { at_key = true; }
                '"' => { i += 1; while i < b.len() && b[i] != b'"' { if b[i] == b'\\' { i += 1; } i += 1; } }
                _ if depth == 1 && at_key && (c.is_ascii_alphabetic() || c == '_') => {
                    let start = i;
                    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
                    let k = rest[start..i].to_string();
                    // only a key (followed by ':'), not a value
                    let mut j = i; while j < b.len() && (b[j] as char).is_whitespace() { j += 1; }
                    if j < b.len() && b[j] == b':' { keys.push(k); }
                    at_key = false;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        if keys.is_empty() { continue }
        let arms: String = keys.iter().map(|k| format!("\tif (k == \"{k}\") {{ return self.exports.{name}.{k}; }}\n")).collect();
        getters.push_str(&format!("function __get_{name}(k) {{\n{arms}\treturn null;\n}}\n"));
        let idx = crate::re!(&format!(r"self\.exports\.{}\[((?:[^\[\]]|\[[^\[\]]*\])+)\]", regex::escape(&name)));
        out = idx.replace_all(&out, format!("__get_{name}($1)").as_str()).into_owned();
    }
    if getters.is_empty() { out } else { format!("{getters}{out}") }
}

/// `self.getViewRootContainer().<child>.<ask>(args)` -> `__ownArt("<ask>", [args])`, the args
/// taken to their balanced close (a frame picked by height nests four deep).
fn own_art_asks(src: &str) -> String {
    let head = crate::re!(r"self\.getViewRootContainer\(\)\.[A-Za-z_]\w*\.(playFrameLabel|playFrame|gotoAndStop|gotoAndPlay|setVisible|setAlpha)\(");
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    while let Some(m) = head.find_at(src, at) {
        let ask = head.captures(&src[m.start()..]).map(|c| c[1].to_string()).unwrap_or_default();
        let mut depth = 1usize;
        let mut end = None;
        for (k, ch) in src[m.end()..].char_indices() {
            match ch { '(' => depth += 1, ')' => { depth -= 1; if depth == 0 { end = Some(m.end() + k); break; } } _ => {} }
        }
        let Some(close) = end else { break };
        out.push_str(&src[at..m.start()]);
        out.push_str(&format!("__ownArt(\"{ask}\", [{}])", &src[m.end()..close]));
        at = close + 1;
    }
    out.push_str(&src[at..]);
    out
}

/// The pieces the stage placed under an object's own name, handed over as `exports.__art`: the
/// object's asks of its clip's child go to them. A frame number holds that frame (the source's
/// gotoAndStop), at this engine's two frames per source frame.
const OWN_ART_HELPER: &str = "// the object's own pictures, handed over by the stage (pieces placed under its name)\n\
function __ownArt(m, a) {\n\
\tvar arts = self.exports.__art;\n\
\tif (arts == null) { return; }\n\
\tfor (i in 0...arts.length) {\n\
\t\tvar g = arts[i];\n\
\t\tif (g == null) { continue; }\n\
\t\tif (m == \"playFrameLabel\" || m == \"playFrame\" || m == \"gotoAndStop\" || m == \"gotoAndPlay\") {\n\
\t\t\tvar s = Std.string(a[0]); var num = s.length > 0;\n\
\t\t\tfor (ci in 0...s.length) { var k = s.charCodeAt(ci); if ((k < 48 || k > 57) && !(ci == 0 && k == 45)) { num = false; } }\n\
\t\t\t// a frame off the timeline's start is its first frame, as the source clamps it\n\
\t\t\tif (num && g.playFrame != null) { var fno = Std.int(a[0]); if (fno < 1) { fno = 1; } g.playFrame(fno * 2 - 1); if (m != \"gotoAndPlay\" && g.pause != null) { g.pause(); } }\n\
\t\t\telse if (g.playFrameLabel != null) { g.playFrameLabel(a[0]); }\n\
\t\t} else if (m == \"setVisible\" && g.setVisible != null) { g.setVisible(a[0]); }\n\
\t\telse if (m == \"setAlpha\" && g.setAlpha != null) { g.setAlpha(a[0]); }\n\
\t}\n\
}\n";

fn cgo_runnable(raw: &str, anims: &[String], scale: f64, wrap: Option<&ReconWrap>, stage_id: &str,
                self_platform: Option<(f64, f64, f64, f64)>,
                sp_shapes: &[(String, (f64, f64, f64, f64))],
                body_faces: &[(&'static str, Vec<(f64, f64)>)],
                child_rects: &[(String, (f64, f64, f64, f64))],
                sounds: &std::collections::BTreeSet<String>, stage_methods: &[String],
                origin: (f64, f64), pick: &[String]) -> String {
    use regex::Regex;
    use std::collections::{BTreeMap, BTreeSet};
    // A RECTANGLE READ OFF THE OBJECT'S OWN CLIP. The source asks the display tree for a named
    // child's box; the child is a fact of the clip, measured at conversion, and here it is that
    // box at wherever the object is now. The ask that guards it (`getMC().hitBox ? ... : ...`)
    // is answered too: the child is there.
    let raw = &{
        let mut r = raw.to_string();
        // THE OBJECT'S OWN PICTURE, asked through its clip's child (`getMC().sprite.gotoAndStop(n)`):
        // the stage hands the pieces placed under this object's name over as `exports.__art`,
        // and the ask goes to them
        r = own_art_asks(&r);
        r = camera_points(&r);
        // the class asks its own body whether it is off: the port keeps that flag
        if self_platform.is_some() || !sp_shapes.is_empty() {
            r = crate::re!(r"\b[\w.]+\.getFallthrough\(\)").replace_all(&r, "__selfPlatformFallthrough()").into_owned();
        }
        // A CAST IS THE THING ITSELF. `MovieClip(x)` narrows a type and does nothing to the value;
        // read as a call it was an ask nothing answered, and what it wrapped went with it.
        let cast = Regex::new(r"\bself\.(?:MovieClip|Sprite|DisplayObject|DisplayObjectContainer|Bitmap|Shape|Object)\(((?:[^()]|\([^()]*(?:\([^()]*\)[^()]*)*\))*)\)")
            .expect("cast regex");
        r = cast.replace_all(&r, "$1").into_owned();
        for (name, (x, y, w, h)) in child_rects {
            // through a helper, so the position read inside it is not converted by the pass that
            // wraps a position beside a literal: the box is in the engine's own units already
            let _ = scale;
            let lit = format!("__childRect({x:.2}, {y:.2}, {w:.2}, {h:.2})");
            let call = crate::dyn_re(&format!(
                r"(?:self\.MovieClip\()?self\.(?:getMC|getViewRootContainer)\(\)\.{}\)?\.(?:getRect|getBounds)\((?:[^()]|\([^()]*\))*\)",
                regex::escape(name)));
            r = call.replace_all(&r, lit.as_str()).into_owned();
            // the bare read (a truthiness test), not one followed by a member or a call
            let there = crate::dyn_re(&format!(r"self\.(?:getMC|getViewRootContainer)\(\)\.{}(\s*[^\s.(\[\w]|$)", regex::escape(name)));
            r = there.replace_all(&r, "true$1").into_owned();
        }
        r
    };
    // The stage handle an SSF2 object asks the engine for. Fraymakers has no such ask, so the
    // stage hands itself over when it makes the object (see `route_object_calls`) and the object
    // reads it back off its own channel -- persisting it would only capture the null it held
    // before the stage got to it, so every use reads it fresh.
    let raw = &raw.replace("match.getStage()", "self.exports.__stage");
    // an object spawns objects too (a bomb block makes the explosion that clears its row), and its
    // spawn needs the same treatment the stage's got: a real content id, and the stage handle
    // passed along so what it makes can talk back the same way its maker does.
    let raw = &crate::api_mappings::rewrite_spawn_enemy_calls(raw, &|class: &str| format!("{stage_id}{}", class.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>()));
    // "DO I HAVE A BODY". A class keeps what it got back when it gave itself one and tests that
    // handle before touching it. The port keeps its body somewhere else, so the test read a field
    // that is not there and the whole guarded block went out -- taking the toggle inside it with
    // it. The question has an exact answer here, so it is asked of the body the port does keep.
    let raw = &{
        let held = crate::re!(r"self\.(\w+)\s*=\s*self\.createSelfPlatform\w*\(");
        match held.captures(raw).map(|c| c[1].to_string()) {
            Some(f) if self_platform.is_some() => raw
                .replace(&format!("__truthy(self.{f})"), "(__deck() != null)")
                .replace(&format!("self.{f} != null"), "(__deck() != null)")
                .replace(&format!("self.{f} == null"), "(__deck() == null)"),
            _ => raw.clone(),
        }
    };
    let raw = &{
        let made = crate::re!(r#"(?m)^(\s*)(\w+)\s*=\s*match\.createCustomGameObject\(.*getContent\("[^"]+"\)"#);
        let mut o = String::new();
        for line in raw.lines() {
            o.push_str(line);
            o.push('\n');
            if let Some(c) = made.captures(line) {
                o.push_str(&format!("{}if ({v} != null) {{ {v}.exports.__stage = self.exports.__stage; }}\n",
                                    &c[1], v = &c[2]));
            }
        }
        o
    };
    // The class keeps that handle in a field, and the field-to-persistent-slot pass would try to
    // store an object in an int. It isn't state: it is the one stage, so every read goes straight
    // back to the channel and the slot disappears.
    // A field the class fills with a small fixed LIST cannot live in one of the engine's scalar
    // slots -- the store is an int and the value is an array, and the engine says so. Each element
    // gets its own slot instead, and the reads that index it follow. (Only a flat literal of
    // scalars: anything else is left alone rather than guessed at.)
    let list_field = crate::re!(r"(?m)^[ \t]*self\.(m_\w+)\s*=\s*\[([^\[\]]+)\];");
    let raw = &{
        let mut out = raw.clone();
        let found: Vec<(String, Vec<String>)> = list_field.captures_iter(&out)
            .map(|c| (c[1].to_string(), top_level_items(&c[2])))
            .collect();
        for (field, elems) in found {
            // `self.f = [a, b];` -> one assignment per element
            let sets: String = elems.iter().enumerate()
                .map(|(i, e)| format!("self.{field}__{i} = {e};"))
                .collect::<Vec<_>>().join("\n\t");
            out = list_field.replace(&out, sets.as_str()).into_owned();
            // and `self.f[N]` -> the element's own field
            // AN INDEX CAN BE WRITTEN AS A WHOLE NUMBER OR AS A NUMBER THAT HAPPENS TO BE ONE.
            // The source's own index comes back as `0.0000` as often as `0`, and a rewrite that
            // only knew the first left `self.f[0.0000]` behind -- which then read as the field
            // `__0` with `.0000` after it, and the line stopped being a line.
            for i in 0..elems.len() {
                let re = crate::dyn_re(&format!(
                    r"self\.{}\[\s*{i}(?:\.0+)?\s*\]", regex::escape(&field)));
                out = re.replace_all(&out, format!("self.{field}__{i}").as_str()).into_owned();
            }
            // ...and where the index was written as a number that happens to be whole, the part
            // after the point is left sitting against the field's name. It is the same element.
            let trail = crate::dyn_re(&format!(r"({}__\d+)\.0+\b", regex::escape(&field)));
            out = trail.replace_all(&out, "${1}").into_owned();
        }
        out
    };
    // THE OBJECT'S ART IS THE OBJECT. SSF2 hands an object the clip it draws with (`setStance`) and
    // the class then animates, fades and hides that clip; in Fraymakers the object HAS its art, and
    // reaching for a separate clip finds nothing at all. So a field holding the object's own clip
    // is the object's own view, and every ask of it lands there. The field is named by the source,
    // not guessed: it is the one the class's `setStance` writes.
    let raw = &{
        let setter = crate::re!(r"function setStance\(arg0\)\s*\{\s*self\.(\w+)\s*=\s*arg0;");
        match setter.captures(raw).map(|c| c[1].to_string()) {
            None => raw.clone(),
            Some(f) => {
                let mut out = raw.clone();
                for (pat, rep) in [
                    (format!(r#"self\.{f}\.(?:playFrameLabel|gotoAndStop|gotoAndPlay)\("([^"]+)"\)"#),
                     "self.playAnimation(\"$1\")".to_string()),
                    (format!(r#"self\.{f}\.setAlpha\(([^;]+)\)"#),
                     "self.getViewRootContainer().alpha = $1".to_string()),
                    (format!(r#"self\.{f}\.alpha\s*=\s*([^;]+)"#),
                     "self.getViewRootContainer().alpha = $1".to_string()),
                    (format!(r#"self\.{f}\.setVisible\(([^;]+)\)"#),
                     "self.getViewRootContainer().visible = $1".to_string()),
                    (format!(r#"self\.{f}\.alpha"#),
                     "self.getViewRootContainer().alpha".to_string()),
                    (format!(r#"(?m)^[ \t]*(?://[^\n]*)?self\.{f}\.nextFrame\(\);\n"#),
                     String::new()),
                    (format!(r#"(?m)^[ \t]*self\.{f}\s*=\s*arg0;\n"#),
                     "\t// the object's art is its own: SSF2 handed it a clip, and there is no clip to hand\n".to_string()),
                ] {
                    if let Ok(re) = regex::Regex::new(&pat) {
                        out = re.replace_all(&out, rep.as_str()).into_owned();
                    }
                }
                out
            }
        }
    };
    let stage_field = crate::re!(r"(?m)^[ \t]*self\.(m_\w+)\s*=\s*self\.exports\.__stage;\n");
    let raw = &match stage_field.captures(raw).map(|c| c[1].to_string()) {
        None => raw.clone(),
        Some(field) => stage_field.replace_all(raw, "")
            .replace(&format!("self.{field}"), "self.exports.__stage"),
    };
    // an api chain on the stage handle: read the answer the stage publishes instead of trying to
    // call through a handle that cannot be called through.
    let raw = &{
        let mut out = raw.clone();
        for chain in stage_value_chains(&out) {
            out = out.replace(&format!("self.exports.__stage.{chain}"),
                              &format!("self.exports.__stage.exports.{}", chain_key(&chain)));
        }
        out
    };
    // A field whose value comes from the stage is not kept in a slot: the object's initialize runs
    // the moment it is made, BEFORE whoever made it hands the stage over, so the store would take
    // a null. Every use reads the published value instead, and by update time it is there.
    let fed_by_stage = crate::re!(r"(?m)^[ \t]*self\.(m_\w+)\s*=\s*(self\.exports\.__stage\.exports\.\w+);\n");
    let raw = &{
        let pairs: Vec<(String, String)> = fed_by_stage.captures_iter(raw)
            .map(|c| (c[1].to_string(), c[2].to_string())).collect();
        let mut out = fed_by_stage.replace_all(raw, "").into_owned();
        for (field, read) in pairs {
            out = out.replace(&format!("self.{field}"), &read);
        }
        out
    };
    // and what it calls on the stage are the STAGE CLASS's own methods, which reach it the same
    // way its own do: through the channel, never off the api.
    // and what it asks OF THE STAGE goes the same way as what the stage asks of it: a call cannot
    // carry the stage's scope any more than the stage's could carry this one's. Bomb factory's block
    // tells the stage which cell it just freed -- unasked, no cell ever cleared, so the stage read a
    // full grid, and a full grid makes every new block a bomb. The bricks simply stopped coming.
    let raw = &crate::re!(r"self\.exports\.__stage\.(\w+)\(([^()]*)\)")
        .replace_all(raw, |c: &regex::Captures| {
            // ...but only what the STAGE CLASS itself defines. `getDeathBounds()` is the engine's,
            // asked of the stage handle and answering right now; queued, it returns a queue length
            // and the code that wanted a rectangle gets a number.
            if !stage_methods.iter().any(|m| m == &c[1]) { return c[0].to_string(); }
            let args = c[2].trim();
            format!("self.exports.__stage.exports.__q.push({{ n: \"{}\", a: [{args}] }})", &c[1])
        }).into_owned();
    // FrameTimer fields -> duration expr; dead fields (bound to an [SSF2-only] call); int fields.
    let timer_re = crate::re!(r"self\.(\w+)\s*=\s*new FrameTimer\(([^;]+)\)");
    let dead_re = crate::re!(r"//\s*\[SSF2-only:[^]]*\]\s*self\.(\w+)\s*=");
    let field_re = crate::re!(r"self\.(m_\w+)");

    let mut timers: BTreeMap<String, String> = BTreeMap::new();
    let mut dead: BTreeSet<String> = BTreeSet::new();
    for c in timer_re.captures_iter(raw) { timers.insert(c[1].to_string(), c[2].trim().to_string()); }
    for c in dead_re.captures_iter(raw) { dead.insert(c[1].to_string()); }
    // A field is only dead if NOTHING still writes it. One unportable read commented out the line
    // it shared with a live assignment, and the field went dead on that alone: bomb factory's block
    // lost its whole state machine (and with it every method the stage calls on a block) because
    // one of six `m_action = N` writes sat behind a clip-label check.
    let live_write = crate::re!(r"(?m)^\s*self\.(\w+)\s*=");
    for c in live_write.captures_iter(raw) { dead.remove(&c[1]); }
    // every other m_X (read or written, not a timer, not dead) becomes an int persistent slot.
    let mut ints: BTreeSet<String> = BTreeSet::new();
    for c in field_re.captures_iter(raw) {
        let f = c[1].to_string();
        if !timers.contains_key(&f) && !dead.contains(&f) { ints.insert(f); }
    }
    // A FIELD IS WHATEVER THE CLASS WRITES TO ITSELF, not whatever matches a naming habit. SSF2
    // authors usually prefix members `m_`, and reading only those left every other one addressed to
    // the engine's own object -- central highway's pillars set `self.point1` on the api and the
    // script stopped there. The class's own assignments say which names are its.
    let own_write = crate::re!(r"(?m)^\s*(?://[^\n]*?)?self\.(\w+)\s*=[^=]");
    for c in own_write.captures_iter(raw) {
        let f = c[1].to_string();
        if !timers.contains_key(&f) && !dead.contains(&f) { ints.insert(f); }
    }
    // ...including one it only ever writes THROUGH: a list is filled by index, and read that way
    // it is still the class's own field.
    let own_index = crate::re!(r"self\.(\w+)\[");
    for c in own_index.captures_iter(raw) {
        let f = c[1].to_string();
        if !timers.contains_key(&f) && !dead.contains(&f) { ints.insert(f); }
    }
    // ...and what it is asked to HOLD says where it lives. A number belongs in a persistent slot,
    // which is what the engine rolls back; anything else (a point, another object) does not fit in
    // one, so it goes on the object's own channel, which holds a value of any kind.
    let objs: BTreeSet<String> = ints.iter()
        .filter(|f| !timers.contains_key(f.as_str()))
        .filter(|f| {
            let new_re = crate::dyn_re(&format!(r"self\.{}\s*=\s*new\s+(\w+)", regex::escape(f)));
            let assigned_object = new_re.captures_iter(raw).any(|c| &c[1] != "FrameTimer");
            let used_as_receiver = raw.contains(&format!("self.{f}."));
            let indexed = raw.contains(&format!("self.{f}["));
            assigned_object || used_as_receiver || indexed
        })
        .cloned().collect();
    for f in &objs { ints.remove(f); }

    // STATE-MACHINE DETECTION: the int field compared against the most integer literals is the
    // update() switch discriminant (SSF2's `m_action`-style state var). its name isn't fixed.
    let cmp_re = crate::re!(r"self\.(m_\w+)\s*(?:==|!=)\s*(-?\d+)");
    let mut cmp_counts: BTreeMap<String, usize> = BTreeMap::new();
    for c in cmp_re.captures_iter(raw) {
        if ints.contains(&c[1]) { *cmp_counts.entry(c[1].to_string()).or_default() += 1; }
    }
    let state_field = cmp_counts.into_iter().filter(|(_, n)| *n >= 1).max_by_key(|(_, n)| *n).map(|(f, _)| f);
    // the state values + the animation each plays. a value is registered against the forceAttack label
    // textually nearest its `= N` (initialize sets the entrance state next to forceAttack("entrance"),
    // a handler plays its anim next to its transition), constrained to a real entity animation.
    let mut state_regs: BTreeMap<i64, String> = BTreeMap::new();
    // the states entered without starting a clip: registered under the clip that carries on
    // (the engine ignores a move to a value it was never given, whichever way it is asked), and
    // entered through the value-only setter, so the clip playing is left alone.
    let mut carried_states: BTreeSet<i64> = BTreeSet::new();
    if let Some(st) = &state_field {
        ints.remove(st); // no makeInt slot — it lives in the local-state machine now.
        let raw_lines: Vec<&str> = raw.lines().collect();
        let label_re = crate::re!(r#"forceAttack\("([^"]+)"\)"#);
        let asg_re = crate::dyn_re(&format!(r"self\.{st}\s*=\s*(-?\d+)"));
        let val_re = crate::dyn_re(&format!(r"self\.{st}\s*(?:==|!=|=)\s*(-?\d+)"));
        // a state plays the forceAttack label nearest its `= N` (constrained to a real entity anim).
        let fn_re = crate::re!(r"^\s*function\s+\w+");
        let mut labeled: BTreeMap<i64, String> = BTreeMap::new();
        for (i, line) in raw_lines.iter().enumerate() {
            let Some(c) = asg_re.captures(line) else { continue };
            let Ok(v) = c[1].parse::<i64>() else { continue };
            // the METHOD this assignment sits in, not three lines either side of it: a class plays
            // its clip and sets its state in the same method, but not necessarily next to each
            // other -- bomb factory's explode() plays "explode" and sets the state eight lines
            // later, past a spawn and its setup. Missed, that state inherited the 282-frame bomb
            // clip, so a dying block sat on its cell for five seconds, the grid filled, and every
            // new block came out a bomb.
            let lo = raw_lines[..=i].iter().rposition(|l| fn_re.is_match(l)).unwrap_or(0);
            let hi = raw_lines.iter().enumerate().skip(i + 1)
                .find(|(_, l)| fn_re.is_match(l)).map(|(j, _)| j)
                .unwrap_or(raw_lines.len());
            let mut best: Option<(usize, String)> = None;
            // ...and at the SAME nesting as the assignment. A clip named in a neighbouring branch
            // is that branch's, however few lines away it sits.
            let indent_of = |l: &str| l.len() - l.trim_start().len();
            let want = indent_of(raw_lines[i]);
            for (j, near) in raw_lines.iter().enumerate().take(hi).skip(lo) {
                if indent_of(near) != want { continue; }
                if let Some(lc) = label_re.captures(near) {
                    let lbl = lc[1].to_string();
                    if anims.iter().any(|a| a.eq_ignore_ascii_case(&lbl)) {
                        let d = i.abs_diff(j);
                        if best.as_ref().map(|(bd, _)| d < *bd).unwrap_or(true) { best = Some((d, lbl)); }
                    }
                }
            }
            // Take the method's label only when it can only mean this state: either it sits right
            // beside the assignment, or the method names exactly one clip. `update()` names several
            // -- one per branch -- and the nearest by line is then just the nearest, not the right
            // one: bomb factory's armed bomb took "idle" from another branch and detonated two
            // frames after arming. Anything else carries the clip already playing.
            let distinct: std::collections::BTreeSet<String> = raw_lines[lo..hi].iter()
                .filter_map(|l| label_re.captures(l).map(|c| c[1].to_string()))
                .filter(|l| anims.iter().any(|a| a.eq_ignore_ascii_case(l)))
                .collect();
            if let Some((d, lbl)) = best {
                if d <= 4 || distinct.len() == 1 { labeled.insert(v, lbl); }
            }
        }
        // every value set or tested must be registered (toLocalState warns on an unregistered value).
        let mut all_vals: BTreeSet<i64> = BTreeSet::new();
        for c in val_re.captures_iter(raw) { if let Ok(v) = c[1].parse::<i64>() { all_vals.insert(v); } }
        // states SSF2 animates implicitly (no adjacent forceAttack, e.g. the gravity-on fall) get the
        // entity anims no labeled state claimed, handed out in state-value order; idle is the last resort.
        let claimed: BTreeSet<String> = labeled.values().map(|s| s.to_ascii_lowercase()).collect();
        // SEGMENTS OF ONE LABEL ARE ONE ANIMATION, not several to hand out. The source cuts its
        // clip into `loop_1`..`loop_4` and plays whichever it drew -- the same tumble entered at
        // four phases, not four things the object can be doing. Handed out per state, the object
        // wears a different one for falling, waiting and rising, and nothing is left for the label
        // its class actually names.
        let mut spare: Vec<String> = if pick.is_empty() {
            anims.iter().filter(|a| !claimed.contains(&a.to_ascii_lowercase())).cloned().collect()
        } else { Vec::new() };
        let default_anim = pick.first().cloned()
            .or_else(|| anims.iter().find(|a| a.eq_ignore_ascii_case("idle")).cloned())
            // WHEN THE CLIP GAVE NO ANIMATIONS, the entity was built from the template and carries
            // the template's names -- so "idle" names nothing it has, and every state entry asks
            // the engine for an animation that is not there. Name what the entity actually got.
            .or_else(|| anims.first().cloned())
            .unwrap_or_else(|| "gameObjectIdle".to_string());
        // A state whose assignment sits next to no forceAttack is one SSF2 never re-animates: it
        // CARRIES ON with the clip already playing. Bomb factory's block arms a bomb (state 3,
        // "bomb"), then goes to state 4 when the fuse clip ends and keeps standing there as the
        // same lit bomb -- handed the next unclaimed clip instead, it turned into an explosion the
        // moment it armed. So an unlabelled state inherits the nearest labelled one below it, and
        // only a state with nothing below it at all has to be guessed.
        // ...and it carries on by naming NOTHING, not by naming the same clip again. Registering
        // the clip a second time makes the engine start it over on the way in, so bomb factory's
        // bomb played its fuse twice and only blew up at the end of the second one -- which is also
        // why it took so long to go off. A state that names no animation leaves what is playing.
        let mut carried: Option<String> = None;
        for v in all_vals {
            let (anim, guessed) = match labeled.get(&v) {
                Some(a) => { carried = Some(a.clone()); (a.clone(), false) }
                None if carried.is_some() => { carried_states.insert(v); (carried.clone().unwrap_or_default(), false) }
                None if !spare.is_empty() => (spare.remove(0), true),
                None => (default_anim.clone(), true),
            };
            if guessed {
                // nothing labelled came before it either, so this one really is a guess. flag it so a
                // wrong-looking hazard animation is traceable to here, not assumed correct.
                eprintln!("  warning: hazard state {v} has no labelled animation and none before it; \
                           guessed \"{anim}\" (verify the clip is right)");
            }
            state_regs.insert(v, anim);
        }
    }
    // state read/compare/assign -> local-state API (compiled once; the field name is fixed here).
    let state_lower = state_field.as_ref().map(|st| (
        crate::dyn_re(&format!(r"self\.{st}\s*==\s*(-?\d+)")),
        crate::dyn_re(&format!(r"self\.{st}\s*!=\s*(-?\d+)")),
        crate::dyn_re(&format!(r"self\.{st}\s*=\s*(-?\d+)")),
        st.clone(),
    ));

    // isOnFloor/setYVelocity/getCamera/createVfx are REAL FM Entity APIs (fraymakers-api-docs
    // Entity.md) and pass through 1:1 — only their VALUES get unit-converted below. isOnGround
    // normalizes to the FM spelling. setCamBoxSize has no FM port (the camera frames its targets
    // itself), setFallthrough is ported by the deck-structure follower, and sounds lack resources.
    let force_re = crate::re!(r#"\[SSF2-only: forceAttack\]\s*self\.forceAttack\("([^"]+)"\)"#);
    // the same ask with a label the class WORKS OUT rather than writes down ("idle" + its form).
    // The engine takes a name at runtime either way, so a computed one plays just as a literal does.
    let force_expr_re = crate::re!(r#"\[SSF2-only: forceAttack\]\s*self\.forceAttack\((.+)\);?\s*$"#);
    let risky_expr: &[(&str, &str)] = &[("self.isOnGround()", "self.isOnFloor()")];
    // A sound the package ships is a sound the engine can find, so the ask goes through. One it
    // does not ship names nothing, and asking for it is worse than silence.
    let play_re = crate::re!(r#"AudioClip\.play\(\s*"([^"]+)"\s*\)"#);
    let risky = ["setCamBoxSize", "setFallthrough"];
    // why each of those is left alone, said where the line is
    let risky_why: &[(&str, &str)] = &[
        ("setCamBoxSize", "SSF2 gives an object its own box for the camera to keep in frame; Fraymakers takes the object as a target and sizes the shot itself, with no box to hand it"),
    ];
    // physics intents -> the scripted kinematics integrator (the wrapper appends the step). units:
    // a 30fps per-frame velocity halves (x scale x 0.5); a per-frame^2 acceleration quarters.
    let grav_re = crate::re!(r"^(\s*)self\.updateGameObjectStats\(\{ gravity: (-?\d+(?:\.\d+)?) \}\);");
    let shake_re = crate::re!(r"\.shake\((-?\d+(?:\.\d+)?)\)");
    // the number inside a setXSpeed/setYSpeed call, wherever in the argument it sits
    // BOUNDED TO THE CALL IT IS IN. `[^)]*` crosses newlines, so a `setXSpeed(` anywhere above
    // let this reach down the script and scale the first number it met -- which was the `1` in an
    // object key called `path1Spawn`, turning the key into `path0.6500Spawn` and the whole script
    // into something the engine cannot parse. Four stages lost everything their scripts do, to a
    // pattern that did not stop at the end of a line.
    // THE NUMBER HAS TO BE AN ARGUMENT, not any digit that happens to be in the line. A lazy run
    // up to the first digit finds the `0` in `arg0` and scales it, which leaves `arg0.0000` -- a
    // field of a name, on a line that then parses nowhere and takes the whole script with it. So
    // what comes before it is a bracket or a comma, and what follows is a bracket or a comma:
    // that is what an argument looks like, whether it sits in the call itself or in a helper
    // inside it. A number being multiplied by something is not one.
    let vfx_scale_re = crate::re!(r"(scaleX|scaleY): (-?\d+(?:\.\d+)?)");
    // The object's own solidity, however it is spelled: an SSF2Platform asks itself
    // (`self.setFallthrough`), and a stage's platform field is asked through the field.
    let fallthrough_re = Regex::new(
        // Both spellings of the same toggle: the source's own, and the one it has already been
        // lowered to by the time this runs. Read only as the first, an object whose fallthrough
        // call came through the general mapping was left with a write to a field that is not
        // there, struck out, and its body stayed solid for the whole match -- the thwomp's deck
        // never opened during its drop.
        r"^(\s*)(?://\s*\[[A-Za-z0-9-]+(?::\s*\w+)?\]\s*)?self\.(?:m_\w+\.)?(?:setFallthrough\((true|false)\)|updateStructureStats\(\{\s*disabled:\s*(true|false)\s*\}\));").unwrap();

    // A frame index the class compares against is the SOURCE's frame, counted at 30 per second;
    // the engine answers at 60. Left alone, a bomb that should arm at frame 13 of its fuse arms at
    // frame 6 or 7 -- half the fuse -- so bomb factory detonated its whole grid at once.
    let raw = &crate::re!(r"(self\.getCurrentFrame\(\)\s*(?:>=|>|==|<=|<)\s*)(\d+)")
        .replace_all(raw, |c: &regex::Captures| {
            let n: u64 = c[2].parse().expect("digits");
            format!("{}{}", &c[1], n * crate::entity_gen::FRAME_RATIO)
        }).into_owned();
    // A FIELD THE CLASS MOVES ITSELF BY is a speed: so much per frame at the source's thirty a
    // second. This engine runs at sixty, so what goes into that field is half as much each time --
    // the size of the world is applied later, by the pass that puts source coordinates into this
    // stage's. Crateria's platform keeps its whole route this way and drifted twice as fast.
    let raw = &{
        use std::collections::BTreeSet;
        let step = crate::re!(r"self\.set[XY]\(([^;]*)\)");
        let field = crate::re!(r"self\.(\w+)");
        let mut speeds: BTreeSet<String> = BTreeSet::new();
        // ...and whether the step it feeds is measured from the object's OWN position. When it is,
        // the pass that puts source coordinates into this stage's applies the size of the world to
        // the step as well, so only the frame rate is left to convert here; when the class carries
        // its position in a field of its own, that pass never sees it and both apply.
        let mut from_own_position: BTreeSet<String> = BTreeSet::new();
        for c in step.captures_iter(raw) {
            let arg = &c[1];
            let own = arg.contains("self.getX()") || arg.contains("self.getY()");
            for f in field.captures_iter(arg) {
                speeds.insert(f[1].to_string());
                if own { from_own_position.insert(f[1].to_string()); }
            }
        }
        // ...and a limit the class clamps that speed to is stated in the same units
        let mut clamps: BTreeSet<String> = BTreeSet::new();
        for f in &speeds {
            let re = crate::dyn_re(&format!(r"Math\.(?:min|max)\(\s*-?self\.(\w+),\s*self\.{}\b", regex::escape(f)));
            for c in re.captures_iter(raw) { clamps.insert(c[1].to_string()); }
        }
        // a field that only ever holds a POSITION is not a speed, however it is spelled: the give-away
        // is being read for its own x/y rather than added to one.
        speeds.retain(|f| !raw.contains(&format!("self.{f}.x")) && !raw.contains(&format!("self.{f}.y")));
        // ...and a field the class never ACCUMULATES is an offset, not a speed: a speed is so much
        // per frame, which the class adds to the object's own position or to a position it keeps
        // (crateria's `position.x = position.x + xSpeed`); a table of stops added to a rest position
        // read once (dracula's castle's `pos[0] + platformStops[..]`) is so many source pixels,
        // which the position boundary converts, and scaling it here moved the platforms short
        let accumulated: BTreeSet<String> = {
            let mut acc = BTreeSet::new();
            let sum = crate::re!(r"self\.(\w+)((?:\.[xy])?)\s*=\s*self\.(\w+)((?:\.[xy])?)\s*[+-]\s*self\.(\w+)\b");
            for c in sum.captures_iter(raw) {
                if c[1] == c[3] && c[2] == c[4] { acc.insert(c[5].to_string()); }
            }
            let short = crate::re!(r"self\.\w+(?:\.[xy])?\s*[+-]=\s*self\.(\w+)\b");
            for c in short.captures_iter(raw) { acc.insert(c[1].to_string()); }
            acc
        };
        speeds.retain(|f| from_own_position.contains(f) || accumulated.contains(f));
        let num = crate::re!(r"-?\d+(?:\.\d+)?");
        let convert = |text: &str, factor: f64| -> String {
            num.replace_all(text, |c: &regex::Captures| {
                let m = c.get(0).expect("match");
                // A NUMBER INSIDE A NAME IS NOT A NUMBER. `path1Spawn` is one name, and scaling
                // the `1` in it made `path0.6500Spawn` -- which is not a name at all, so the
                // engine could not parse the script and dropped every object the stage makes.
                // Four stages went that way. What comes BEFORE decides it: a letter, a digit or
                // an underscore means this is part of a name and none of our business.
                let in_name = text[..m.start()].chars().next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_');
                if in_name { return m.as_str().to_string() }
                let v: f64 = m.as_str().parse().unwrap_or(0.0);
                let h = v * factor;
                if h == h.trunc() { format!("{h}") } else { format!("{h:.4}") }
            }).into_owned()
        };
        let mut o = raw.clone();
        for f in speeds.iter().chain(clamps.iter()) {
            let factor = if from_own_position.contains(f) {
                1.0 / crate::entity_gen::FRAME_RATIO as f64
            } else {
                velocity_scale()
            };
            let halve = |t: &str| convert(t, factor);
            // what is PUT IN the field
            let set = crate::dyn_re(&format!(r"(?m)^(\s*)self\.{}\s*=\s*([^;]+);", regex::escape(f)));
            o = set.replace_all(&o, |c: &regex::Captures| {
                format!("{}self.{f} = {};", &c[1], halve(&c[2]))
            }).into_owned();
            // and what it is measured against
            let cmp = crate::dyn_re(&format!(r"self\.{}\s*(<=|>=|<|>)\s*(-?\d+(?:\.\d+)?)", regex::escape(f)));
            o = cmp.replace_all(&o, |c: &regex::Captures| {
                format!("self.{f} {} {}", &c[1], halve(&c[2]))
            }).into_owned();
        }
        o
    };
    // A METHOD THE CLASS HANDS AROUND rather than calls: crateria's platform keeps its route as a
    // list of timers, each carrying the move to run when it fires. Here those are this script's own
    // functions, so the reference is to one of them.
    let raw = &{
        let mut o = raw.clone();
        for m in crate::re!(r"(?m)^function (\w+)\(").captures_iter(raw) {
            let name = m[1].to_string();
            let needle = format!("self.{name}");
            let mut acc = String::with_capacity(o.len());
            let mut rest = o.as_str();
            while let Some(at) = rest.find(&needle) {
                acc.push_str(&rest[..at]);
                let after = &rest[at + needle.len()..];
                // a CALL keeps its receiver dropped elsewhere; only a bare reference is rewritten
                let is_call = after.trim_start().starts_with('(');
                let is_longer_name = after.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_');
                if is_call || is_longer_name { acc.push_str(&needle); } else { acc.push_str(&name); }
                rest = after;
            }
            acc.push_str(rest);
            o = acc;
        }
        o
    };
    // a struck line handing a clip over as the shape of a body, when this class named shapes
    let sp_shape_re = crate::re!(r#"createSelfPlatformWithMC\(\s*(?:[\w.]+\.)?getMCByLinkageName\(\s*"([^"]+)"\s*\)"#);
    // ...or from a name the class puts together as it runs (`"terrain_" + self.vehicleName`)
    let sp_shape_expr_re = crate::re!(r#"(?:(self\.\w+)\s*=\s*)?(?:self\.)?createSelfPlatformWithMC\(\s*(?:[\w.]+\.)?getMCByLinkageName\(\s*("[^"]+"\s*\+\s*[^()]+)\)\s*\)"#);
    let mut out: Vec<String> = Vec::new();
    // How much of a struck line's block is still open. A line that cannot be ported is commented
    // out, and when that line OPENS a block the block goes with it: leaving the body and its
    // closing brace behind is a brace the script cannot account for, and the whole file stops
    // parsing at the next `else`.
    let mut struck_depth: i32 = 0;
    let braces = |t: &str| -> i32 {
        t.chars().filter(|c| *c == '{').count() as i32 - t.chars().filter(|c| *c == '}').count() as i32
    };
    for line in raw.lines() {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];
        if struck_depth > 0 {
            struck_depth += braces(trimmed);
            out.push(if trimmed.is_empty() { line.to_string() }
                     else { format!("{indent}// [needs-port] {trimmed}") });
            continue;
        }
        // the class's field list has been read by the passes above; it is not code
        if trimmed.starts_with("// [class-fields]") { continue }
        // already-commented decompiler gaps pass through untouched — except the ground-detach,
        // which has a 1:1 FM equivalent (Entity.unattachFromFloor).
        if trimmed.starts_with("//") {
            if trimmed.contains("[SSF2-only: unnattachFromGround]") {
                out.push(format!("{indent}self.unattachFromFloor(); // SSF2 unnattachFromGround"));
            } else if let Some(c) = force_re.captures(trimmed) {
                // SSF2's forceAttack on an OBJECT plays that clip -- it is the class naming one of
                // its own labels, not a fighter's buffered input. It plays the clip and nothing
                // else: the object's state is its own field, written by its own assignments (which
                // are the state moves here), and a clip is not a state. Birdo walks in its state 0
                // and only LEAVES in its state 3, both drawn with "walk"; routed through the state
                // machine, the first "walk" jumped it into the leaving state and it died at once.
                let label = c[1].to_string();
                match state_regs.iter().find(|(_, a)| a.eq_ignore_ascii_case(&label)) {
                    Some(_) => out.push(format!("{indent}self.playAnimation(\"{label}\"); // SSF2 forceAttack")),
                    // the label names a clip the source cut into segments and draws between
                    None if !pick.is_empty() => {
                        let list = pick.iter().map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(", ");
                        out.push(format!("{indent}// SSF2 forceAttack(\"{label}\"), whose clip draws which of                                           its own segments to play"));
                        out.push(format!("{indent}self.playAnimation([{list}][Random.getInt(0, {})]);",
                            pick.len() - 1));
                    }
                    // A clip can only draw what it HAS. SSF2 ignores a class asking its clip for a
                    // label the clip does not carry, so asking here -- where it is an error that
                    // stops the script -- would be less faithful, not more.
                    None if !anims.is_empty() && !anims.iter().any(|a| a.eq_ignore_ascii_case(&label)) => {
                        out.push(format!("{indent}// the clip has no \"{label}\" to draw, and the source's own engine drew nothing for it either"));
                    }
                    None => out.push(format!("{indent}self.playAnimation(\"{label}\"); // SSF2 forceAttack")),
                }
            } else if let Some(c) = force_expr_re.captures(trimmed) {
                // reached only when the label is not a plain literal: that arm is above. the label
                // is worked out from the object's own fields, so those are read the way the rest of
                // its code reads them.
                let mut e = c[1].to_string();
                for f in &objs { e = e.replace(&format!("self.{f}"), &format!("self.exports.{f}")); }
                for f in &ints { e = wrap_whole_field(&e, f, &format!("_s_{f}.get()")); }
                out.push(format!("{indent}self.playAnimation({e}); // SSF2 forceAttack"));
            } else if let Some(c) = sp_shape_re.captures(trimmed) {
                // the class hands a clip over as the outline of the body it stands on; the port
                // measured that clip, so the body it builds is the size the source drew
                out.push(format!("{indent}__createSelfPlatformShaped(\"{}\"); // SSF2 createSelfPlatformWithMC", &c[1]));
            } else if let Some(c) = sp_shape_expr_re.captures(trimmed) {
                // ...by a name it puts together as it runs, out of the shapes measured for every
                // clip under that name's prefix; the body it keeps a handle to is its deck
                let mut e = c[2].to_string();
                for f in &objs { e = e.replace(&format!("self.{f}"), &format!("self.exports.{f}")); }
                for f in &ints { e = wrap_whole_field(&e, f, &format!("_s_{f}.get()")); }
                let keep = c.get(1).map(|m| {
                    let mut l = m.as_str().to_string();
                    for f in &objs { l = l.replace(&format!("self.{f}"), &format!("self.exports.{f}")); }
                    format!(" {l} = __deck();")
                }).unwrap_or_default();
                out.push(format!("{indent}__createSelfPlatformShaped({e});{keep} // SSF2 createSelfPlatformWithMC"));
            } else if trimmed.contains("[SSF2-only: addChild]") && trimmed.contains("getWeatherMC()") {
                // the weather itself is ported (see the line above it, which builds it); the port
                // puts it in its layer as it makes it, so the source's own placing has already
                // happened by the time this line would run
                out.push(format!("{indent}// the weather is put in its layer as the port makes it"));
            } else if trimmed.contains("[SSF2-only: setForeground]") {
                // the ask is carried out where the containers are: see the boot (runtime/stage/boot.hx)
                out.push(format!("{indent}// this object is drawn ahead of the fighters, which the stage does as it makes it"));
            } else if trimmed.contains("[SSF2-only: createSelfPlatform]") && self_platform.is_some() {
                out.push(format!("{indent}__createSelfPlatform(); // SSF2 createSelfPlatform"));
            } else if trimmed.contains("[SSF2-only: removeSelfPlatform]") && self_platform.is_some() {
                out.push(format!("{indent}__removeSelfPlatform(); // SSF2 removeSelfPlatform"));
            } else {
                out.push(line.to_string());
            }
            continue;
        }
        // FrameTimer construction: the counter IS the state (starts at 0); drop the `new`.
        if timer_re.is_match(line) { out.push(format!("{indent}// FrameTimer construction -> the module-scope makeFrameTimer")); continue; }
        // the self-platform's fallthrough toggle lowers natively: SSF2 fallthrough(true) means
        // riders pass through entirely, which is FM's `disabled` stat (dropThrough would only
        // make it a soft platform).
        if self_platform.is_some() {
            if let Some(c) = fallthrough_re.captures(line) {
                let v = c.get(2).or_else(|| c.get(3)).map(|m| m.as_str()).unwrap_or("false");
                out.push(format!("{}__selfPlatformDisabled({v}); // SSF2 setFallthrough({v})", &c[1]));
                continue;
            }
        }
        // a line touching a dead field or a risky unmapped call: neutralize.
        // A SOUND IS ASKED FOR THROUGH THE PACKAGE THAT CARRIES IT. Named on its own, the engine
        // looks for it among everything loaded and finds nothing, so the ask is silent -- the name
        // the package files it under is not the name the class calls it. Handed the package's own
        // answer for that name, it plays. (Confirmed in-engine: the bare name gives back nothing,
        // the resolved one gives back a clip.)
        let unshipped_sound = play_re.captures(line).is_some_and(|c| !sounds.contains(&c[1]));
        if dead.iter().any(|f| line.contains(&format!("self.{f}")))
            || unshipped_sound
            || risky.iter().any(|r| line.contains(r)) {
            if let Some((_, w)) = risky_why.iter().find(|(r, _)| line.contains(r)) {
                out.push(format!("{indent}// {w}"));
            }
            out.push(format!("{indent}// [needs-port] {trimmed}"));
            struck_depth += braces(trimmed).max(0);
            continue;
        }
        let mut s = line.to_string();
        s = play_re.replace_all(&s, "AudioClip.play(self.getResource().getContent(\"$1\"))").to_string();
        // risky query in a condition -> safe literal (keeps braces balanced).
        for (call, lit) in risky_expr { s = s.replace(call, lit); }
        // the state field -> local-state machine (before the generic int rewrite; it's not in `ints`).
        // the local-state methods live on EntityScriptCommon (the script's own base class), so they're
        // called BARE — `self.` resolves the api object, which doesn't carry them ("Invalid function null").
        if let Some((eq, ne, asg, st)) = &state_lower {
            // A STATE THAT CARRIES ON WITH THE CLIP PLAYING IS ENTERED WITHOUT STARTING ONE. a state
            // left out of the registry to keep the clip going was a state the object could never
            // reach (the engine stays put on a value it was never given): bowser's castle's thwomp
            // fell onto the floor still in its entrance state and never saw the landing.
            s = asg.replace_all(&s, |c: &regex::Captures| {
                let quiet = c[1].parse::<i64>().is_ok_and(|v| carried_states.contains(&v));
                if quiet { format!("setLocalState({})", &c[1]) } else { format!("toLocalState({})", &c[1]) }
            }).to_string();
            s = eq.replace_all(&s, "inLocalState($1)").to_string();
            s = ne.replace_all(&s, "!inLocalState($1)").to_string();
            s = s.replace(&format!("self.{st}"), "getLocalState()");
        }
        // timer ops lower in a post-flatten pass (the frames-in-state form needs the flat ladder
        // to PROVE per timer that all its accesses live in one state handler). without the
        // wrapper there's no frames-in-state counter, so lower onto per-slot counters here.
        // a FrameTimer duration is 30fps frames; the counters tick at 60fps, so x2.
        if wrap.is_none() {
            for f in timers.keys() {
                s = s.replace(&format!("self.{f}."), &format!("_t_{f}."));
                // ...and the timer handed over as a value: a class that keeps a "current"
                // timer among several assigns one to a field and reads through the field
                s = wrap_whole_field(&s, f, &format!("_t_{f}"));
            }
        }
        // a field holding something other than a number lives on the object's own channel.
        for f in &objs {
            s = s.replace(&format!("self.{f}"), &format!("self.exports.{f}"));
        }
        // int field write/read -> persistent get/set.
        // A WRITE, NOT A COMPARISON. `self.x = v` becomes a persistent set; `self.x == v` must
        // not, and matched on its first `=` it did: sector z's arwing asked `if (m_decision ==
        // "shoot")` and was emitted `if (_s_m_decision.set("shoot"))`, which decides nothing and
        // sets the field every frame. Every ported class that compares a persistent field to a
        // value was doing this.
        for f in &ints {
            let wre = crate::dyn_re(&format!(r"self\.{f}\s*=\s*([^=\s])"));
            s = wre.replace_all(&s, format!("_s_{f}.__SET__ ${{1}}")).to_string();
            s = wrap_whole_field(&s, f, &format!("_s_{f}.get()"));
            s = bare_field_read(&s, f, &format!("_s_{f}.get()"));
        }
        // updateGameObjectStats: the flag the source has and the engine spells the other way
        // round. A class that says it survives the death bounds is one the source parks outside
        // them on purpose (fourside's spaceship waits above the top of the stage for a minute
        // before it comes down), and with the flag dropped the engine destroyed it on its first
        // frame: the stage then asked a gone object to arrive, and nothing did. The projectile
        // side already carried this as `deathBoundsDestroy`, inverted; the object side does now.
        s = s.replace(", bypassCollisionTesting: false", "").replace(", bypassCollisionTesting: true", "")
             .replace(", surviveDeathBounds: false", ", deathBoundsDestroy: true")
             .replace(", surviveDeathBounds: true", ", deathBoundsDestroy: false");
        // SSF2 physics calls stay NATIVE (the engine integrates gravity/velocity and grounds the
        // object, so isOnFloor() is the real engine test) — only the VALUES convert from 30fps:
        // acceleration x scale/4, velocity x scale/2; the class's maxYSpeed caps as terminalVelocity.
        if let Some(c) = grav_re.captures(&s) {
            let g: f64 = c[2].parse().unwrap_or(0.0);
            let term = wrap.filter(|_| g > 0.0)
                .map(|w| format!(", terminalVelocity: {:.2}", w.terminal)).unwrap_or_default();
            s = format!("{}self.updateGameObjectStats({{ gravity: {:.2}{} }}); // SSF2 gravity {} @30fps",
                &c[1], g * scale * 0.25, term, g);
        }
        // a speed the class states or reads crossed the boundary in translation (`__sv`/`__isv`,
        // the same helpers a fighter's script carries), so nothing here scales a literal
        // screen-space args (camera shake amplitude, the dust vfx scale) x the stage scale.
        if let Some(c) = shake_re.captures(&s) {
            let a: f64 = c[1].parse().unwrap_or(0.0);
            s = shake_re.replace(&s, format!(".shake({:.1})", a * scale).as_str()).to_string();
        }
        if s.contains("createVfx") {
            s = vfx_scale_re.replace_all(&s, |c: &regex::Captures| {
                format!("{}: {:.1}", &c[1], c[2].parse::<f64>().unwrap_or(1.0) * scale)
            }).to_string();
        }
        out.push(s);
    }
    let mut body = format!(
        "// AS3's own rule for reading a value as a test: a non-null object and a non-zero number\n\
         // are true. hscript reads only a bool, so without this every `if (thing)` the source\n\
         // wrote is dead.\nfunction __truthy(v) {{ return !(v == null || v == false || v == 0); }}\n\n{}",
        out.join("\n"));
    // The class's own methods call each other through `self`; here they are plain functions in one
    // script, and `self` is the api, which has never heard of them.
    let own: Vec<String> = crate::re!(r"(?m)^function\s+([A-Za-z_]\w*)\s*\(")
        .captures_iter(&body).map(|c| c[1].to_string()).collect();
    for f in &own {
        body = body.replace(&format!("self.{f}("), &format!("{f}("));
        // and a method handed over as a CALLBACK is the same name without the call -- to a timer,
        // to an event listener, wherever it is passed rather than invoked.
        body = body.replace(&format!("self.{f})"), &format!("{f})"));
        body = body.replace(&format!("self.{f},"), &format!("{f},"));
    }
    // finalize the write marker into a .set(...) wrapping the rest of the statement.
    // A clip can only draw what it HAS, wherever the ask came from -- a label the class names on
    // the clip it was handed reaches here as a play too. SSF2 ignores an ask for a label its clip
    // does not carry; this engine stops the script over it, so the ask is left as a note.
    if !anims.is_empty() {
        let play_re = crate::re!(r#"(?m)^(\s*)self\.playAnimation\("([^"]+)"\);"#);
        body = play_re.replace_all(&body, |c: &regex::Captures| {
            if anims.iter().any(|a| a.eq_ignore_ascii_case(&c[2])) { c[0].to_string() }
            else { format!("{}// the clip has no \"{}\" to draw, and the source's own engine drew nothing for it either", &c[1], &c[2]) }
        }).into_owned();
    }
    // A TIMER IS A TIMER WHEREVER IT IS KEPT. A class that holds a whole route of them in a list
    // (crateria's platform) builds each one in place, so the construction converts here the same
    // way a field's does: the engine's own timer, with the source's frames doubled.
    // the duration can be worked out rather than written down: `(30 * 6.75) - 2` has brackets of
    // its own, and an argument list with no brackets in it does not match one -- so sand ocean's
    // seventh timer stayed a name nothing here builds, and the entry it was made for was never
    // made at all, leaving the line after it writing into nothing.
    let ctor_re = crate::re!(r"new FrameTimer\(((?:[^()]|\([^()]*\))*)\)");
    // ...doubled only where the step runs every engine frame (the spawn wrapper): a class's own
    // step runs at the source's rate (see the_step_runs_at_the_source_rate), so its frames are its own
    let per_engine_frame = if wrap.is_some() { " * 2" } else { "" };
    body = ctor_re.replace_all(&body, format!("self.makeFrameTimer(($1){per_engine_frame})").as_str()).to_string();
    // ...and the noise a typed list declaration leaves behind is not code.
    let vec_re = crate::re!(r"(?m)^[ \t]*Vector\.\w+\s*=\s*[\w.]+\.new\(\);\n");
    body = vec_re.replace_all(&body, "").to_string();
    // ...but a timer can only be MADE while the object is still being set up. A class that builds
    // its timers in a method (crateria's platform builds a whole route of them) asks the engine for
    // one after that moment, and the engine refuses. The making is lifted out to where the object
    // is still open; the method keeps the timer it was given, in the order it asked for them.
    let mut hoisted = String::new();
    {
        // WHERE THE CALL ENDS, counted rather than guessed. Reading to the last `)` before the
        // `;` assumes the making is the whole statement, and a class that collects its timers
        // (`m_timers.push(new FrameTimer(4.5))`) makes one INSIDE another call -- so the outer
        // call's own closing paren was taken as part of the timer's arguments and carried into
        // the hoisted line, which then had one `)` too many and would not parse. The engine
        // drops a script it cannot parse whole, so the object it belonged to did nothing at all.
        let take_call = |l: &str, n: &mut usize, hoisted: &mut String| -> String {
            const OPEN: &str = "self.makeFrameTimer(";
            let mut out = String::with_capacity(l.len());
            let mut rest = l;
            while let Some(at) = rest.find(OPEN) {
                out.push_str(&rest[..at]);
                let args_from = at + OPEN.len();
                let mut depth = 1i32;
                let mut end = None;
                for (i, c) in rest[args_from..].char_indices() {
                    match c {
                        '(' => depth += 1,
                        ')' => { depth -= 1; if depth == 0 { end = Some(args_from + i); break } }
                        _ => {}
                    }
                }
                let Some(close) = end else { out.push_str(&rest[at..]); return out };
                *n += 1;
                hoisted.push_str(&format!(
                    "var _tm{n} = self.makeFrameTimer({});\n", &rest[args_from..close]));
                out.push_str(&format!("_tm{n}"));
                rest = &rest[close + 1..];
            }
            out.push_str(rest);
            out
        };
        let mut n = 0usize;
        let mut out = String::with_capacity(body.len());
        let mut in_fn = 0i32;
        for line in body.lines() {
            let mut l = line.to_string();
            if in_fn > 0 && l.contains("self.makeFrameTimer(") {
                l = take_call(&l, &mut n, &mut hoisted);
            }
            in_fn += l.matches('{').count() as i32 - l.matches('}').count() as i32;
            if in_fn < 0 { in_fn = 0 }
            out.push_str(&l);
            out.push('\n');
        }
        body = out;
    }
    // A TIMER THE CLASS HANGS SOMETHING ON. SSF2 lets a timer carry a payload (crateria's platform
    // hangs the move to run on each timer of its route); the engine's timer carries nothing, so the
    // pair is kept together instead -- the timer under `t`, whatever was hung on it beside it.
    {
        let with_data = crate::re!(r"(?m)^\s*([\w.]+\[[^\]]+\])\.data\s*=");
        let carriers: std::collections::BTreeSet<String> = with_data.captures_iter(&body)
            .map(|c| {
                // the LIST, not the element: every element of it is a pair from here on
                let e = &c[1];
                e[..e.find('[').unwrap_or(e.len())].to_string()
            })
            .collect();
        for list in &carriers {
            // what is put IN the list becomes the pair
            let put = crate::dyn_re(&format!(r"(?m)^(\s*){}(\[[^\]]+\])\s*=\s*([^;]+);",
                                          regex::escape(list)));
            body = put.replace_all(&body, |c: &regex::Captures| {
                let v = c[3].trim();
                if v.starts_with('{') { c[0].to_string() }   // already a pair
                else { format!("{}{list}{} = {{ t: {v}, data: null }};", &c[1], &c[2]) }
            }).into_owned();
            // and every ask of the timer goes through the half that is one
            for m in ["tick()", "reset()", "finish()", "completed", "duration"] {
                let ask = crate::dyn_re(&format!(r"{}(\[[^\]]+\])\.{}",
                                              regex::escape(list), regex::escape(m)));
                body = ask.replace_all(&body, format!("{list}${{1}}.t.{m}").as_str()).into_owned();
            }
        }
    }
    // A FLAG THAT FLIPS. AS3 writes `x = !x` and reads `if (x)`; the slot holding it is a number,
    // and `!` on a number is not a flip -- it reads as the same thing every frame, so a pillar that
    // should shake left and right walked steadily off the stage instead.
    // A CLASS THAT MOVES ITSELF BY STEPPING ITS OWN POSITION is stating a speed: so much per frame,
    // at the source's thirty a second. This engine runs at sixty, so the step is half as far each
    // time -- the size of the world is not applied here, because the step stays in the source's own
    // coordinates and the pass that puts those into this stage's applies it. Left alone, central
    // highway's pillar descended at twice the rate it should.
    let step_re = crate::re!(r"self\.set([XY])\(self\.get([XY])\(\)\s*([+-])\s*(\d+(?:\.\d+)?)\)");
    let body = step_re.replace_all(&body, |c: &regex::Captures| {
        let n: f64 = c[4].parse().unwrap_or(0.0);
        format!("self.set{}(self.get{}() {} {:.4})", &c[1], &c[2], &c[3],
                n / crate::entity_gen::FRAME_RATIO as f64)
    }).into_owned();
    let mut body = body;
    let flip_re = crate::re!(r"_s_(\w+)\.__SET__ !\s*_s_(\w+)\.get\(\);");
    body = flip_re.replace_all(&body, "_s_$1.__SET__ (_s_$2.get() == 0 ? 1 : 0);").to_string();
    let not_re = crate::re!(r"!\s*_s_(\w+)\.get\(\)");
    body = not_re.replace_all(&body, "(_s_$1.get() == 0)").to_string();
    // the placeholder stands for the `=`, but the source does not always leave it out -- absorb
    // one if it is there, or the write comes out as `set(= 0)`
    let set_re = crate::re!(r"_s_(\w+)\.__SET__\s*=?\s*(.*);");
    body = set_re.replace_all(&body, "_s_$1.set($2);").to_string();
    // ...AND WHERE THE ASSIGNMENT IS NOT A STATEMENT. The source writes to a field and uses what
    // it wrote in the same breath -- `if ((m_x = t.elapsedFrames) && ...)` -- so there is no `;`
    // to match on, and the placeholder went out as written: a field called `__SET__` on a store
    // that has no such field, refused on the spot. Bounded by the brackets it sits in.
    let set_in_expr =
        crate::re!(r"_s_(\w+)\.__SET__\s*=?\s*([^;()]*(?:\([^()]*\)[^;()]*)*)");
    body = set_in_expr.replace_all(&body, "_s_$1.set($2)").to_string();
    // A timer's length is stated in the source's frames, and this engine counts twice as many, so a
    // timer RESET to a new length converts exactly like the one it was built with.
    let dur_re = crate::re!(r"(?m)^(\s*)(_t_\w+)\.duration\s*=\s*([^;]+);");
    body = dur_re.replace_all(&body, format!("$1$2.duration = ($3){per_engine_frame};").as_str()).to_string();

    // with the spawn wrapper, the decompiled lifecycle becomes inner machinery: the wrapper owns
    // the engine entry points and re-runs __hazardInit on every spawn-cycle engage. (renaming
    // BEFORE the state registration below routes the registrations to the one-time module-scope
    // guard instead of re-registering per engage.)
    if wrap.is_some() {
        body = body.replace("function initialize() {", "function __hazardInit() {")
                   .replace("function update() {", "function __hazardUpdate() {");
    }

    // register the local-state machine + each state's animation at the top of initialize() — before the
    // decompiled body's first toLocalState, and once (initialize runs once per spawn). a hazard always
    // has initialize(); if a future projectile path lacks one, guard a module-scope init instead.
    // (with the wrapper, registration moves into the SYNTHESIZED initialize below — a make* slot
    // dereferenced at module scope null-accesses on the first eval, before the CGO api is live.)
    if !state_regs.is_empty() && wrap.is_none() {
        // The Fraymakers-side setup the source never had to do: an object has to be put into an
        // active state or the engine never updates it -- it stands wherever it was made, never
        // animating, never running its own code, and never hearing anything asked of it. The
        // template has always done this; a class running its own code needs it just the same.
        let mut reg = String::from("\n\tself.setState(PState.ACTIVE);\n\tinitLocalStateMachine();");
        // a state is registered with an animation only when the entity has one of that name.
        // named with something it does not carry -- an empty string, or a label the entity was
        // not built from -- entering the state asks the engine to play an animation that is not
        // there, which it refuses once per entry. a state that carries on with the clip playing
        // is registered under that clip and entered through the value-only setter, since the
        // engine ignores a move to a value it was never given, whichever way it is asked.
        // A STATE IS REGISTERED WITH WHAT THE SOURCE SAYS, even when this package could not build
        // an animation of that name. Registering only the ones it could build leaves an object
        // with NO states, and then the code that moves between them has nothing to move to and
        // stops on its first move -- which is worse than the engine noting an animation it does
        // not have and carrying on. Fourside's spaceship lost its whole state machine that way.
        for (v, anim) in state_regs.iter().filter(|(_, a)| !a.is_empty()) {
            reg.push_str(&format!("\n\tregisterLocalState({v}, \"{anim}\");"));
        }
        let init_re = crate::re!(r"function initialize\(\) \{");
        if init_re.is_match(&body) {
            body = init_re.replace(&body, format!("function initialize() {{{reg}").as_str()).to_string();
        } else {
            body = format!("var _sm_init = self.makeBool(false);\nif (!_sm_init.get()) {{\n\t_sm_init.set(true);{reg}\n}}\n\n{body}");
        }
    }

    // persistent-state declarations at MODULE scope, exactly like the verified template: the engine's
    // makeInt(default) returns a call-order-keyed persistent store, so the script re-running each frame
    // just rebinds the same var to the same store (default applied once). functions close over these by
    // bare name — never re-call makeInt inside a function (that would shift the call-order keying).
    // A PORTED CLASS COUNTS IN THE UNITS IT WAS WRITTEN IN.
    //
    // The source works things out from where it is, against numbers calibrated to ITS coordinates:
    // clocktown's meteor aims itself with `10 - (x + 35) / 32.25`, which is a position in the town
    // measured in the town's own units. The engine answers `getX()` in the STAGE's units, which
    // are the source's shifted by the stage origin and scaled -- so the same arithmetic reads a
    // different place, and the meteor aimed away from the town instead of across it.
    //
    // A LITERAL is the tell. Every number in the source's own code is in the source's units, so a
    // coordinate combined with one is source-space arithmetic and the read converts back. A
    // coordinate handed straight to something else, or compared against a value the ENGINE gave
    // (a death bound, a column the converter placed), is not arithmetic in those units and is left
    // alone. Where a converted read is written back out, the write converts forward again, so a
    // round trip cancels and a move by N source units moves N source units.
    let has_literal = crate::re!(r"[-+*/(]\s*[0-9]+(?:\.[0-9]+)?|[0-9]+(?:\.[0-9]+)?\s*[-+*/]");
    let mut needs_coord_helpers = false;
    // A PLACE THE CLASS STATES OUTRIGHT (`self.setX(241)`) is a place in the source's stage
    // space, the same space the stage script's own literals are in, and goes through the same
    // conversion (emerald cave's panels sat at the source's numbers, half a stage off)
    // AN OBJECT'S OWN CODE LIVES IN THE SOURCE'S UNITS, all of it: its tables of spawn points,
    // the position it steps along a path, the place it remembers off a read. So every place
    // READ off the engine comes in through the conversion and every place WRITTEN to it goes
    // out through it, and the code between never sees the engine's units. Half measures
    // (converting a literal here, a table there) sent jungle island's barrels a third of the
    // way up the stage and palace of twilight's platforms off their own start points.
    {
        let before = body.clone();
        body = boundary_reads_and_writes(&body, &|recv: &str| recv.contains("_proj") || recv.contains("getCamera"));
        if body != before { needs_coord_helpers = true; }
    }
    let _ = &has_literal;
    if body.contains("__childRect(") || body.contains("__boundsRect(") || body.contains("__rectsMeet(") {
        needs_coord_helpers = true;
    }

    // A FIELD READ BY A NAME WORKED OUT AT RUNTIME (`ALL_PATHS["path" + n]`) off an object the
    // class wrote as a literal: this engine's hscript cannot index a plain object, so the read
    // becomes a getter over the keys the literal has (jungle island's barrel paths).
    // a string index the known-literal pass left (a table it could not read) goes to a lookup
    let mut body = string_indexed_members(&string_concat_is_plus(&index_a_known_literal(&body)));
    // AN ANIMATION PLAYED BY A NAME WORKED OUT AT RUNTIME (`playAnimation(arg0)`, the type the
    // stage handed over) is spelled the way the entity's animations are: the source's labels
    // keep their underscores, the entity's names do not (emerald cave's ecave_wood_left)
    if crate::re!(r#"self\.playAnimation\(\s*[^\s"()]"#).is_match(&body) {
        body = crate::re!(r#"self\.playAnimation\(\s*([^"()][^()]*)\)"#)
            .replace_all(&body, "self.playAnimation(__anim($1))").into_owned();
    }
    let mut preamble = String::new();
    if body.contains("__anim(") {
        preamble.push_str(
            "function __anim(l) { var s = \"\"; var t = Std.string(l); for (i in 0...t.length) { var k = t.charCodeAt(i); if ((k >= 48 && k <= 57) || (k >= 65 && k <= 90) || (k >= 97 && k <= 122)) { s += t.charAt(i); } } return s; }\n");
    }
    // the fields that turned out to hold a point: their reads and writes are the point's own
    let mut point_fields: Vec<String> = Vec::new();
    if needs_coord_helpers {
        preamble.push_str(&format!(
            "// the source's own units: the stage's, less the origin it was laid out from\n\
             function __isx(v) {{ return v / {scale:.6} + ({:.4}); }}\n\
             function __isy(v) {{ return v / {scale:.6} + ({:.4}); }}\n\
             function __sx(v) {{ return (v - ({:.4})) * {scale:.6}; }}\n\
             function __sy(v) {{ return (v - ({:.4})) * {scale:.6}; }}\n\
             function __sd(v) {{ return v * {scale:.6}; }}\n\
             // a pair handed to the engine as a point, in this stage's px\n\
             function __pt(p) {{ return Point.create(__sx(p.x), __sy(p.y)); }}\n",
            origin.0, origin.1, origin.0, origin.1));
    }
    // A FIELD THE CLASS INDEXES OR PUSHES INTO IS A LIST, and starts as one. SSF2 builds it in the
    // class's own declaration, which is not part of what a port reads, so an unstarted list is a
    // null the first write lands on -- crateria's platform keeps its whole route in one.
    // ...unless the class itself asks whether the field is still empty (`pos == null`, dracula's
    // castle's platform reading its rest position once): then null is what it starts as, and
    // the class is what fills it
    for f in &objs {
        let tested = raw.contains(&format!("self.{f} == null")) || raw.contains(&format!("self.{f} != null"));
        if !tested && (raw.contains(&format!("self.{f}[")) || raw.contains(&format!("self.{f}.push("))) {
            preamble.push_str(&format!(
                "if (self.exports.{f} == null) {{ self.exports.{f} = []; }}\n"));
        }
    }
    let decimal = crate::re!(r"[0-9]\.[0-9]");
    static ARG_ONLY: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^\s*arg\d+\s*\)?\s*$").expect("regex"));
    for f in &ints {
        // A FIELD IS WHAT IT IS ASKED TO HOLD. An integer store rounds whatever is put in it, and
        // a source field that carries a fraction then reads back as something else: clocktown's
        // meteor works out where to aim from where it spawned (`10 - (x + 35) / 32.25`), and
        // rounded to a whole number that lands on ZERO for a meteor near the middle. Zero is also
        // the value the source tests to decide whether it has aimed yet, so the meteor re-aims
        // every frame with a fresh random and drifts nowhere: it falls straight down instead of
        // across. Read the assignments and give the field a store that can hold them.
        let set_re = crate::dyn_re(&format!(r"_s_{}\.(?:__SET__|set\()([^;]*)", regex::escape(f)));
        let fractional = set_re.captures_iter(&body)
            .any(|c| c[1].contains('/') || decimal.is_match(&c[1]));
        // ...and a field asked to hold TEXT is not a number at all. Emerald cave's panel keeps
        // which side of the frame it is -- "ecave_wood_left" -- in one, tests it, and builds its
        // broken form by adding "_break" to it. A number store refuses the very first assignment,
        // which is the empty string, and the object stops there: it never learns which panel it
        // is and never plays anything. What the source puts in a field says what the field is.
        // WHAT IS PUT IN, NOT WHAT IS MENTIONED ANYWHERE IN THE LINE. A quote inside a nested
        // call's arguments is not the field being given text: peach's castle's bonzai bill is
        // given the handle of a sound it started -- `AudioClip.play(getContent("..."))` -- and
        // reading the quote in there as text gave it a text store, which refuses the handle.
        let value_of = |v: &str| v.trim().trim_start_matches('(').trim().to_string();
        let is_text = |v: &str| {
            let v = value_of(v);
            v.starts_with('"') || v.contains("+ \"") || v.contains("\" +")
        };
        let is_number = |v: &str| {
            let v = value_of(v);
            !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()
                || matches!(c, '-' | '+' | '.' | ' ' | '(' | ')' | '*' | '/'))
        };
        let values: Vec<String> = set_re.captures_iter(&body).map(|c| c[1].to_string()).collect();
        let textual = values.iter().any(|v| is_text(v))
            || body.contains(&format!("_s_{f}.get() + \""))
            || body.contains(&format!("\" + _s_{f}.get()"));
        // ...and anything that is neither a number nor text is a thing only a store that holds
        // anything can hold. Where the source does not say plainly, it does not say.
        let plainly = !values.is_empty()
            && values.iter().all(|v| is_text(v) || is_number(v));
        // A FIELD FILLED FROM AN ARGUMENT IS WHATEVER THE CALLER HANDS IT, and that is not
        // knowable from the assignment alone -- so a number store is the one choice that can be
        // WRONG in a way that stops the object dead: it refuses the value outright. Emerald
        // cave's panel is handed its boundary that way. A store that holds anything holds a
        // number too, so where the source does not say, it does not say.
        let from_arg = set_re.captures_iter(&body).any(|c| ARG_ONLY.is_match(&c[1]));
        // ...and a field the source puts BOTH in is neither. lunar core's laser keeps which of
        // three kinds it is, sets it from a random number, and elsewhere sets it from a name --
        // a text store refuses the number outright and the laser never starts. Where the source
        // says two things, the store that holds anything is the only one that holds both.
        let also_untexted = set_re.captures_iter(&body)
            .any(|c| !c[1].contains('"') && !c[1].trim().trim_matches(|c| c == ')' || c == '(').is_empty());
        // A FIELD THAT HOLDS A PAIR OF COORDINATES HOLDS THE ENGINE'S OWN POINT. The engine makes
        // points, and reads and writes both halves of one, but it will only MAKE one while the
        // object is still being set up -- which is exactly when a field is declared. So a field
        // the source only ever puts a pair of coordinates in is declared as one, and then it is
        // a point everywhere: the stage's own code reads the halves as before, and the engine
        // takes it where it wants a point (a camera target, the angle between two of them),
        // which it would not do for a plain pair of names.
        let point_shaped = |v: &str| {
            let v = v.trim().trim_start_matches('(').trim();
            v.starts_with("{ x: ") && v.contains(", y: ")
        };
        let a_point = !values.is_empty() && values.iter().all(|v| point_shaped(v));
        // a store that holds anything, whether because the source says two things about the field
        // or because what it says is not plainly one kind
        let holds_anything = !a_point && (!plainly || (textual && also_untexted));
        let make = if a_point { "makePoint(0, 0)".to_string() }
                   else if holds_anything { "makeObject(null)".to_string() }
                   else if textual { "makeString(\"\")".to_string() }
                   else if from_arg { "makeObject(null)".to_string() }
                   else if fractional { "makeFloat(0.0)".to_string() }
                   else { "makeInt(0)".to_string() };
        preamble.push_str(&format!("var _s_{f} = self.{make};\n"));
        if a_point { point_fields.push(f.clone()); }
    }
    // ...and a point is not a store, so it is not read and written like one. `set` copies the two
    // halves into the point that is already there, and `get` is the point itself.
    for f in &point_fields {
        let set = crate::dyn_re(&format!(
            r"_s_{}\.(?:__SET__|set\()\s*\{{ x: ((?:[^{{}}]|\{{[^{{}}]*\}})*?), y: ((?:[^{{}}]|\{{[^{{}}]*\}})*?) \}}\s*\)?;",
            regex::escape(f)));
        body = set.replace_all(&body, format!("_s_{f}.x = ${{1}}; _s_{f}.y = ${{2}};").as_str()).into_owned();
        body = body.replace(&format!("_s_{f}.get()"), &format!("_s_{f}"));
    }
    // FrameTimers lower 1:1 onto the engine's own makeFrameTimer (the tick/completed/reset
    // vocabulary is identical; a duration is 30fps frames, so x2). The timer counts UP to its
    // length, exactly like SSF2's.
    for (f, dur) in &timers {
        preamble.push_str(&format!("var _t_{f} = self.makeFrameTimer(({dur}){per_engine_frame});\n"));
    }
    // the timers lifted out of the class's own methods (see above), made while the object still can
    preamble.push_str(&hoisted);
    // WHERE IT STARTED. An object that swings or circles works from the point it was put down --
    // SSF2 asks the object itself for it, and this engine has no such ask, so the call landed on
    // nothing and the object stopped on its first frame of movement. It is the object's own
    // placement, which it can simply remember: read once, before anything has moved it.
    // the source asks the object for it, and here it is the script's own function, which is
    // reached by name and not through the handle
    // A RECTANGLE ASKED WHETHER IT MEETS ANOTHER, and a fighter asked for its rectangle. The
    // rectangles here are plain values, so the ask is a function of both; a fighter's box is the
    // one the engine keeps for it, from its hips to its head and feet.
    let meets = crate::re!(r"([\w.\[\]]+)\.intersects\(");
    let body = meets.replace_all(&body, "__rectsMeet($1, ").into_owned();
    let bounds = crate::re!(r"([\w.\[\]]+)\.getBoundsRect\(\)");
    let body = bounds.replace_all(&body, "__boundsRect($1)").into_owned();
    let has = crate::re!(r"([\w.\[\]]+)\.containsPoint\(");
    let body = has.replace_all(&body, "__rectHasPoint($1, ").into_owned();
    // AN OBJECT AN OBJECT MAKES. The house on planet namek spawns its chunk and tells it what
    // it is (`setType`): a call on the handle the engine gave back, which has no such name. The
    // stage routes a call on what it spawned through the ask channel; so does an object now,
    // with the same outbox, delivered at the top of its update.
    // ...and a call on THE FLOOR AN OBJECT STANDS ON: the source's floor is the object that put
    // it there (bowser's castle's thwomp lands and tells its floor to sink), this engine's is a
    // face handle with no such name, so the call goes through the ask channel to the object
    // whose deck the face is (`__floorOwner`), and to nobody when no object owns it
    let body = {
        let made = crate::re!(r#"(?m)^\s*(?:var\s+)?(\w+)\s*=\s*match\.createCustomGameObject\("#);
        let stood = crate::re!(r#"(?m)^\s*(?:var\s+)?(\w+)\s*=\s*(?:[\w.\[\]]+(?:\(\))?\.)*getCurrentFloor\(\);"#);
        let spawned: BTreeSet<String> = made.captures_iter(&body).map(|c| c[1].to_string()).collect();
        let floors: BTreeSet<String> = stood.captures_iter(&body).map(|c| c[1].to_string())
            .filter(|v| !spawned.contains(v)).collect();
        if spawned.is_empty() && floors.is_empty() { body } else {
            let mut out = body.clone();
            for v in spawned.iter().chain(floors.iter()) {
                let owner = if floors.contains(v) { format!("__floorOwner({v})") } else { v.clone() };
                let call = crate::dyn_re(&format!(r"\b{}\.(\w+)\(((?:[^()]|\((?:[^()]|\([^()]*\))*\))*)\)", regex::escape(v)));
                out = call.replace_all(&out, |c: &regex::Captures| {
                    let name = &c[1];
                    if crate::api_mappings::the_engine_answers_to(name) || name == "exports" {
                        return c[0].to_string();
                    }
                    let args = c[2].trim();
                    format!("__ask({owner}, \"{name}\", [{args}])")
                }).into_owned();
            }
            out
        }
    };
    let body = route_floor_calls(&body);
    // a place written onto something the script holds (`m_backgroundSprite.x = ..`): a plain
    // point takes the field, an object of this engine takes its setter, in its pixels (saffron
    // city's platforms carry their picture as an object and place it every frame)
    let body = crate::re!(r"(?m)^(\s*)((?:self\.exports\.\w+|[A-Za-z_]\w*))\.([xy]) = ([^;]+);")
        .replace_all(&body, |c: &regex::Captures| {
            if &c[2] == "self" || &c[2] == "g" || &c[2] == "h" || &c[2] == "b" || &c[2] == "p" || &c[2] == "it" { return c[0].to_string(); }
            format!("{}__put{}({}, {});", &c[1], c[3].to_ascii_uppercase(), &c[2], &c[4])
        }).into_owned();
    // an object taking a piece out of its own clip (sky sanctuary's falling platform drops its
    // ground child once it has fallen) is taking its floor away: the same platform here is a
    // structure, and one turned off is one nobody stands on
    let body = crate::re!(r"self\.getViewRootContainer\(\)\.removeChild\(self\.getViewRootContainer\(\)\.\w+\);")
        .replace_all(&body, "{ var __d = __deck(); if (__d != null) { __d.updateStructureStats({ disabled: true }); } }").into_owned();
    if body.contains("__floor") && !preamble.contains("function __floorOwner(") {
        preamble.push_str(FLOOR_HELPERS);
    }
    if body.contains("__ownArt(") && !preamble.contains("function __ownArt(") {
        preamble.push_str(OWN_ART_HELPER);
    }
    if body.contains("__putX(") || body.contains("__putY(") {
        preamble.push_str(
            "// a place written onto what the script holds: a point takes the field, an object its setter\n\
             function __putX(o, v) { if (o == null) { return; } if (o.setX != null) { o.setX(__sx(v)); } else { o.x = v; } }\n\
             function __putY(o, v) { if (o == null) { return; } if (o.setY != null) { o.setY(__sy(v)); } else { o.y = v; } }\n");
    }
    if body.contains("__ask(") && !preamble.contains("function __ask(") {
        preamble.push_str(
            "var __outbox = [];\n\
             function __ask(o, n, a) {\n\
             \tif (o == null) { return; }\n\
             \tif (o.exports != null) { if (o.exports.__q == null) { o.exports.__q = []; } o.exports.__q.push({ n: n, a: a }); return; }\n\
             \t__outbox.push({ o: o, n: n, a: a });\n\
             }\n\
             function __deliverAsks() {\n\
             \tvar i = 0;\n\
             \twhile (i < __outbox.length) {\n\
             \t\tvar e = __outbox[i];\n\
             \t\tif (e.o == null || e.o.exports == null) { i = i + 1; }\n\
             \t\telse {\n\
             \t\t\tif (e.o.exports.__q == null) { e.o.exports.__q = []; }\n\
             \t\t\te.o.exports.__q.push({ n: e.n, a: e.a });\n\
             \t\t\ti = i + 1;\n\
             \t\t}\n\
             \t}\n\
             \t__outbox = [];\n\
             }\n");
    }
    // ...delivered before the object's own update reads anything back
    let body = if body.contains("__ask(") && body.contains("function update() {") {
        body.replacen("function update() {", "function update() {\n\t__deliverAsks();", 1)
    } else { body };
    // WHETHER HAZARDS ARE ON, asked by an object of its own accord: the same read the stage
    // makes off the match settings. Undefined here, the ask read as off and a bumper killed
    // itself on its first frame.
    if body.contains("m_hazardsOn") && !preamble.contains("var m_hazardsOn") {
        preamble.push_str(
            "var m_hazardsOn = (match.getMatchSettingsConfig() == null || match.getMatchSettingsConfig().hazards);\n");
    }
    // AN ID FOR EACH SWING. The source stamps its attack record so a fighter is hit once per
    // swing; the record here carries the same count, kept on the object.
    if body.contains("__nextUid(") {
        preamble.push_str(
            "var __uid = self.makeInt(0);\n\
             function __nextUid() { __uid.set(__uid.get() + 1); return __uid.get(); }\n");
    }
    // A FOR-EACH LOOP walks a list by index: the index moves to the next entry (1-based, 0 when
    // there is none) and the body reads the entry under it.
    // the clock of a clip, as the source's code reads it (see the stage script's helpers)
    if body.contains("__currentFrame(") || body.contains("__currentLabel(") || body.contains("__totalFrames(") {
        preamble.push_str(
            "function __srcFrameOf(o) { return Std.int((o.getCurrentFrame() - 1) / 2) + 1; }\n\
             function __firstPart(o) { for (i in 0...o.parts.length) { if (o.parts[i] != null && o.parts[i].exports != null) { return o.parts[i]; } } return null; }\n\
             function __currentFrame(o) { if (o == null) { return 0; } if (o.parts != null) { var p = __firstPart(o); return (p != null && p.exports.__frame != null) ? p.exports.__frame : 0; } if (o.exports != null && o.exports.__frame != null) { return o.exports.__frame; } return __srcFrameOf(o); }\n\
             function __currentLabel(o) { if (o == null) { return \"\"; } if (o.parts != null) { var p = __firstPart(o); return (p != null && p.exports.__label != null) ? p.exports.__label : \"\"; } if (o.exports != null && o.exports.__label != null) { return o.exports.__label; } return o.getAnimation(); }\n\
             function __totalFrames(o) { if (o == null) { return 0; } if (o.parts != null) { var p = __firstPart(o); return (p != null && p.exports.__total != null) ? p.exports.__total : 0; } if (o.exports != null && o.exports.__total != null) { return o.exports.__total; } return Std.int(o.getTotalFrames() / 2); }\n");
    }
    // the object's own clip length, and a sound it keeps to stop later: the
    // fighter's and the projectile's scripts carry these, an object's has to as well (bomb
    // factory's blocks and bowser's castle's thwomp asked for both and stopped there)
    // an object reads its own frame on this engine's clock (`self.getCurrentFrame()`), so its length
    // is read on the same one: halved against that read, a block's explosion ended half way
    if body.contains("__srcTotalFrames(") && !preamble.contains("function __srcTotalFrames") {
        preamble.push_str("function __srcTotalFrames() { return self.getTotalFrames(); }\n");
    }
    if (body.contains("__sound(") || body.contains("__stopSound(")) && !preamble.contains("function __sound(") {
        preamble.push_str(&crate::api_mappings::sound_handle_helpers());
    }
    // a thing taken off the display list is ended
    if body.contains("__endOther(") && !preamble.contains("function __endOther") {
        preamble.push_str("function __endOther(o) { if (o == null) { return; } if (o.exports != null && o.exports.__linkageId != null) { o.exports.__askEnd = true; } else if (o.kill != null) { o.kill(); } }\n");
    }
    // a timer taken off by what the script was handed: a stage object's timers are the engine's own
    if body.contains("__rmTimer(") && !preamble.contains("function __rmTimer") {
        preamble.push_str("function __rmTimer(x) { return self.removeTimer(x); }\n");
    }
    // who a fighter is, by the id its stats state (published by every script this converter makes)
    if body.contains("__statsNameOf(") && !preamble.contains("function __statsNameOf") {
        preamble.push_str(crate::api_mappings::STATS_NAME_OF);
    }
    // a thing turned to face the other way, by the way it faces
    if body.contains("__flip(") && !preamble.contains("function __flip") {
        preamble.push_str("function __flip(o) { if (o == null) { return; } if (o.isFacingRight()) { o.faceLeft(); } else { o.faceRight(); } }\n");
    }
    // an effect put on a list, which a stage object keeps as it is
    if body.contains("__addFx(") && !preamble.contains("function __addFx") {
        preamble.push_str("function __addFx(v) { return v; }\n");
    }
    if body.contains("__iter") {
        preamble.push_str(
            "function __iterNext(o, i) { if (o == null || o.length == null) { return 0; } return i < o.length ? i + 1 : 0; }\n\
             function __iterValue(o, i) { return (o == null || i < 1) ? null : o[i - 1]; }\n\
             function __iterName(o, i) { return i - 1; }\n");
    }

    // a speed the object states or reads is the source's per-frame one, crossing at the boundary
    preamble.push_str(&crate::api_mappings::speed_boundary_helpers_for(crate::api_mappings::SpeedUnits::Scaled));
    // a child's box and a fighter's bounds answer in the units the code does its sums in
    if body.contains("__replaceAll(") {
        preamble.push_str(
            "function __replaceAll(s, p, r) { if (s == null || p == null || p == \"\") { return s; } var out = \"\"; var t = Std.string(s); var i = 0; while (i < t.length) { if (t.substr(i, p.length) == p) { out = out + r; i = i + p.length; } else { out = out + t.charAt(i); i = i + 1; } } return out; }\n");
    }
    if body.contains("__childRect(") {
        preamble.push_str(
            "function __childRect(x, y, w, h) {\n\
             \treturn { x: __isx(self.getX()) + x, y: __isy(self.getY()) + y, width: w, height: h };\n\
             }\n");
    }
    if body.contains("__rectsMeet(") || body.contains("__boundsRect(") || body.contains("__rectHasPoint(") {
        preamble.push_str(
            "function __rectsMeet(a, b) {\n\
             \tif (a == null || b == null) { return false; }\n\
             \treturn a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;\n\
             }\n\
             function __rectHasPoint(a, p) {\n\
             \tif (a == null || p == null) { return false; }\n\
             \treturn p.x >= a.x && p.x < a.x + a.width && p.y >= a.y && p.y < a.y + a.height;\n\
             }\n\
             function __boundsRect(o) {\n\
             \tif (o == null) { return { x: 0, y: 0, width: 0, height: 0 }; }\n\
             \tvar l = __isx(o.getEcbLeftHipX()); var r = __isx(o.getEcbRightHipX()); var t = __isy(o.getEcbHeadY()); var b = __isy(o.getEcbFootY());\n\
             \treturn { x: l, y: t, width: r - l, height: b - t };\n\
             }\n");
    }
    let body = body.replace("self.getStartPosition(", "getStartPosition(");
    if body.contains("getStartPosition(") {
        // the point is read in the units the body does its sums in: a class eases itself back to
        // its start (`y += (start.y - y) / 8`), and a start kept in the port's units while the
        // reads are in the source's parks the object a whole origin away
        let (px, py) = if needs_coord_helpers { ("__isx(self.getX())", "__isy(self.getY())") } else { ("self.getX()", "self.getY()") };
        preamble.push_str(&format!(
            "var _startPt = self.makeObject(null);\n\
             function getStartPosition() {{\n\
             \tif (_startPt.get() == null) {{ _startPt.set({{ x: {px}, y: {py} }}); }}\n\
             \treturn _startPt.get();\n\
             }}\n"));
    }
    let mut body = flatten_state_ladder(&body);
    // The solid body an SSF2 object makes of itself is any object's to make, not only a faller's:
    // bomb factory's blocks ARE the floor, and without this they are pictures a fighter falls
    // through. Built from the box the class declares, wherever that object came from.
    let standalone_sp = match (wrap, self_platform) {
        (None, Some(box_)) => self_platform_code(box_, scale, sp_shapes, body_faces),
        _ => (String::new(), String::new()),
    };
    if body.contains("__fieldOf(") || body.contains("__setFieldOf(") { preamble.push_str(FIELD_OF_HELPERS); }
    // an object with no body of its own has nobody standing on it
    if self_platform.is_none() && body.contains("__standsOnMe(") {
        preamble.push_str("function __standsOnMe(o) {\n\treturn false;\n}\n");
    }
    // AN ANIMATION WORKED OUT AT THE TIME. A class that has one animation per kind of itself asks
    // for the one it is -- `playAnimation("attack" + m_type)` -- and the engine refuses a name the
    // entity has not got. Where the object came out with none that even START that way, no value
    // of the field can name one, so the ask cannot be carried out and saying so is better than
    // asking every frame. Where some do, it is left alone: the field decides which.
    let built_anim = crate::re!(r#"self\.playAnimation\("([A-Za-z_]\w*)" \+ "#);
    body = {
        let mut out = String::with_capacity(raw.len());
        for line in body.lines() {
            let t = line.trim();
            let missing = !t.starts_with("//") && built_anim.captures(t).is_some_and(|c| {
                !anims.iter().any(|a| a.starts_with(&c[1]))
            });
            let net: i32 = t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum();
            if missing && net == 0 {
                let indent = &line[..line.len() - line.trim_start().len()];
                out.push_str(&format!("{indent}// [no animation of this object is named that] {t}\n"));
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        // the body keeps the ending it had: adding one leaves a blank line that shows up as a
        // difference in every script this pass so much as looks at
        if !body.ends_with('\n') { while out.ends_with('\n') { out.pop(); } }
        out
    };
    let Some(w) = wrap else {
        let body = the_step_runs_at_the_source_rate(&body);
        return guard_stage_reads(&format!("{}{}{preamble}\n{body}", standalone_sp.0, standalone_sp.1))
    };

    // wrap path: the same 1:1 FrameTimer lowering (the engage below resets each timer, the
    // fresh-instance semantics SSF2 gets by constructing new timers per spawn).
    for f in timers.keys() {
        body = body.replace(&format!("self.{f}."), &format!("_t_{f}."));
        body = wrap_whole_field(&body, f, &format!("_t_{f}"));
    }

    // the update() switch values become named states: SSF2 inlines the constants, so the name is
    // derived from the animation each state registers (the forceAttack label its handler plays).
    let mut state_consts: std::collections::BTreeMap<i64, String> = std::collections::BTreeMap::new();
    {
        let mut used: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for (v, anim) in &state_regs {
            let base: String = anim.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
            let n = used.entry(base.clone()).or_default();
            *n += 1;
            let name = if *n == 1 { format!("ST_{base}") } else { format!("ST_{base}_{n}") };
            state_consts.insert(*v, name);
        }
        let st_re = crate::re!(r"(toLocalState|setLocalState|inLocalState)\((-?\d+)\)");
        body = st_re.replace_all(&body, |c: &regex::Captures| {
            let v: i64 = c[2].parse().unwrap();
            match state_consts.get(&v) {
                Some(n) => format!("{}({})", &c[1], n),
                None => c[0].to_string(),
            }
        }).to_string();
    }
    let cname = |v: &i64| state_consts.get(v).cloned().unwrap_or_else(|| v.to_string());
    let mut const_decls = String::new();
    for (v, name) in &state_consts { const_decls.push_str(&format!("var {name} = {v};\n")); }

    // ---- the synthesized stage spawn cycle + kinematics integrator ----
    // SSF2 splits this hazard across two classes: the enemy class (the state machine above) and
    // the stage class (spawn cadence/columns, the death-bounds cull). The wrapper rebuilds the
    // stage half around the decompiled enemy half, plus the parts the SSF2 ENGINE did implicitly:
    // gravity/yspeed integration with the maxYSpeed cap, landing on the column surface, and the
    // entrance clip's frame-script setYSpeed timeline (a Graphic-pinned clip never self-runs).
    let cols_lit = w.cols.iter().map(|(x, _)| format!("{x:.1}")).collect::<Vec<_>>().join(", ");
    let land_lit = w.cols.iter().map(|(_, y)| format!("{y:.1}")).collect::<Vec<_>>().join(", ");
    // every class timer restarts on engage (a fresh SSF2 instance constructs fresh timers).
    let timer_resets: String = timers.keys()
        .map(|f| format!("\t\t\t_t_{f}.reset();\n")).collect();
    // state registration runs once, behind the wrapper's first-update guard. NOT module scope
    // (make* slots aren't dereferenceable on the first eval, before the CGO api is live) and NOT
    // initialize() (the engine never calls it on a stage custom game object — the template's
    // m_init-in-update guard is the proven pattern).
    let mut state_reg = String::new();
    if !state_regs.is_empty() {
        state_reg.push_str("var __hasInitLocalStateMachine = false;\n\
                            if (!__hasInitLocalStateMachine) {\n\
                            \tinitLocalStateMachine();\n\
                            \t__hasInitLocalStateMachine = true;\n\
                            }\n");
        // only a state whose animation the entity actually carries -- see above
        for (v, anim) in state_regs.iter().filter(|(_, a)| !a.is_empty()) {
            state_reg.push_str(&format!("registerLocalState({}, \"{anim}\");\n", cname(v)));
        }
    }
    // the class's own addToCamera (inside __hazardInit) fires per engage; the wrapper only needs
    // the matching deleteTarget at the recycle (SSF2's death-bounds cull dropped the target).
    let cam_del = if w.camera { "\t\tmatch.getCamera().deleteTarget(self);\n" } else { "" };
    // SSF2 createSelfPlatform lowering: the body's solid box as runtime structures created at
    // the class's own call site. hscript can't hold object handles across frames (make* slots
    // are scalars), so the structures' indices in match.getStructures() are stashed instead.
    let (sp_decls, sp_helpers, sp_follow, sp_park) = match w.self_platform {
        Some(box_) => {
            let (d, h) = self_platform_code(box_, scale, sp_shapes, body_faces);
            (
                d,
                h,
                // every face of the body rides it, through the helpers that know where they are
                "\t// the self-platform rides the body (SSF2 moves it with the enemy)\n\
                 \t__moveSelfPlatform();\n".to_string(),
                "\t\t// park the self-platform off-world (carrying a rider out = KO, the SSF2 outcome)\n\
                 \t\t__removeSelfPlatform();\n".to_string(),
            )
        }
        None => (String::new(), String::new(), String::new(), String::new()),
    };
    // the bob plays during the entrance state — locate it by its registered animation. its keys
    // ride the frames-in-state counter (the entrance clip starts when the state does).
    let bob_block = state_regs.iter()
        .find(|(_, a)| a.eq_ignore_ascii_case(&w.entrance_anim))
        .filter(|_| !w.bob.is_empty())
        .map(|(v, _)| {
            let ladder: String = w.bob.iter()
                .map(|(f, sp)| format!("\t\tif (_w_state_t.get() == {}) {{ self.setYVelocity({:.2}); }}\n",
                    (f.saturating_sub(1)) * 2, sp * velocity_scale()))
                .collect();
            format!("\t// entrance bob: the entrance sub-clip's frame scripts (setYSpeed timeline, 30->60fps)\n\
                     \tif (inLocalState({})) {{\n{ladder}\t}}\n", cname(v))
        })
        .unwrap_or_default();
    // the bob ladder is the one consumer of a frames-in-state clock (its keys are frame indices,
    // which a FrameTimer's completed flag can't address) — only emit the tracker when it's used.
    let (bob_decls, bob_tracker) = if bob_block.is_empty() { ("", "") } else { (
        "var _w_prev = self.makeInt(-99);\nvar _w_state_t = self.makeInt(0);\n",
        "\t// frames-in-state clock for the entrance bob ladder\n\
         \tif (getLocalState() != _w_prev.get()) {\n\
         \t\t_w_prev.set(getLocalState());\n\
         \t\t_w_state_t.set(0);\n\
         \t} else {\n\
         \t\t_w_state_t.inc();\n\
         \t}\n",
    ) };
    let script = format!(
        "{const_decls}\
         // the local-state machine inits + registers at MODULE scope on every eval, the\n\
         // proven template idiom.\n\
         {state_reg}\
         {preamble}\n\
         // stage spawn machine constants (stepped from the stage + enemy classes)\n\
         var COLUMNS = [{cols_lit}];\nvar LAND_YS = [{land_lit}];\n\
         var SPAWN_Y = {spawn_y:.1};\nvar SPAWN_PERIOD = {spawn_period:.0};\n\
         var _w_init = self.makeBool(false);\n\
         var _w_active = self.makeBool(false);\nvar _w_col = self.makeInt(0);\n\
         // the stage's spawn machine (one full spawn-to-spawn period) + the rehit cadence\n\
         var _w_clock = self.makeFrameTimer(SPAWN_PERIOD);\nvar _w_cool = self.makeFrameTimer(60);\n\
         {bob_decls}{sp_decls}\n\
         {sp_helpers}\
         {body}\n\n\
         function update() {{\n\
         \t// one-time setup on the first update: the stage script positions this object right\n\
         \t// after createCustomGameObject (initialize() runs inside the create call, so a park\n\
         \t// set there would be overridden), and make* slots aren't live at module scope.\n\
         \tif (!_w_init.get()) {{\n\
         \t\t_w_init.set(true);\n\
         \t\tself.setState(PState.ACTIVE);\n\
         \t\tself.setX(COLUMNS[0]);\n\
         \t\tself.setY(SPAWN_Y);\n\
         \t\t// SSF2's first spawn lands half a period in: pre-advance the spawn clock.\n\
         \t\tfor (i in 0...{half_period:.0}) {{ _w_clock.tick(); }}\n\
         \t}}\n\
         \t// spawn-to-spawn clock (the stage spawn machine runs regardless of the enemy's phase)\n\
         \t_w_clock.tick();\n\
         \tif (!_w_active.get()) {{\n\
         \t\tif (_w_clock.completed) {{\n\
         \t\t\t_w_clock.reset();\n\
         \t\t\t_w_col.set(Random.getInt(0, COLUMNS.length - 1));\n\
         \t\t\tself.setX(COLUMNS[_w_col.get()]);\n\
         \t\t\tself.setY(SPAWN_Y);\n\
         \t\t\tself.setYVelocity(0);\n\
         \t\t\tself.updateGameObjectStats({{ gravity: 0 }});\n\
         {timer_resets}\
         \t\t\t__hazardInit();\n\
         \t\t\t_w_active.set(true);\n\
         \t\t}}\n\
         \t\treturn;\n\
         \t}}\n\
         {bob_tracker}\
         \t_w_cool.tick();\n\
         \tif (_w_cool.completed) {{ self.reactivateHitboxes(); _w_cool.reset(); }}\n\
         {bob_block}\
         \t__hazardUpdate();\n\
         {sp_follow}\
         \t// SSF2 culls it past the death bounds (surviveDeathBounds=false); recycle for the next spawn\n\
         \tif (self.getYVelocity() < 0 && self.getY() <= SPAWN_Y) {{\n\
         \t\tself.setY(SPAWN_Y);\n\
         \t\tself.setYVelocity(0);\n\
         {cam_del}\
         {sp_park}\
         \t\t_w_active.set(false);\n\
         \t}}\n\
         }}\n",
        spawn_y = w.spawn_y, spawn_period = w.spawn_period,
        half_period = w.spawn_period / 2.0);
    // the local-state subsystem is reached through the live-verified `Common.` binding (the bare
    // names are unproven in a CGO scope; one pass over the assembled script keeps the lowering,
    // the flattener, and the wrapper all working in bare-name terms).
    let mut script = script;
    for f in ["initLocalStateMachine(", "registerLocalState(", "toLocalState(", "setLocalState(", "inLocalState(", "getLocalState("] {
        script = script.replace(f, &format!("Common.{f}"));
    }
    script
}

/// A call an object makes on its OWN ART, made on the object instead.
///
/// SSF2 hands a thing the clip it draws with and the thing then talks to that clip -- shows it,
/// hides it, moves it. Here there is no clip to talk to: the object draws itself, and what it
/// publishes as its `stance` is whatever the stage handed over, which answers to nothing. This
/// engine has the same thing to say, on the object.
/// A script's OWN function, called by name rather than through the handle.
///
/// The source says `this.fade()` and means the method it defines a few lines up. Here the handle
/// is the engine's api object, which has no such method, so the call is refused and everything
/// after it in that frame stops. What the script defines, the script calls.
fn own_functions_called_by_name(script: &str) -> String {
    use regex::Regex;
    static DEF: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let def = DEF.get_or_init(|| Regex::new(r"(?m)^function (\w+)\s*\(").expect("def regex"));
    let mut out = script.to_string();
    for c in def.captures_iter(script) {
        let n = &c[1];
        // a name this engine answers to as well stays with the engine: those calls were always
        // the engine's, and the script defining a helper of the same name does not change that
        if crate::api_mappings::the_engine_answers_to(n) { continue }
        out = out.replace(&format!("self.{n}("), &format!("{n}("));
        // ...AND HANDED OVER BY NAME, not only called. A listener is given the function itself --
        // `addEventListener(TICK_END, self.checkPanelCollisions)` -- and through `self` that is
        // nothing, so the engine crashes the moment the event fires. Emerald cave registers one
        // on every tick, so it died on its first. The function is the script's own either way.
        let needle = format!("self.{n}");
        let mut rebuilt = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(at) = rest.find(&needle) {
            let after = &rest[at + needle.len()..];
            // the whole name, and not a call: a call was already dealt with above
            let whole = !after.starts_with(|c: char| c.is_alphanumeric() || c == '_');
            rebuilt.push_str(&rest[..at]);
            rebuilt.push_str(if whole && !after.trim_start().starts_with('(') { n } else { &needle });
            rest = after;
        }
        rebuilt.push_str(rest);
        out = rebuilt;
    }
    out
}

/// Strike a call on the object that NOTHING can answer.
///
/// The source's own base class gives its objects methods this engine has no counterpart for --
/// a bumper asks itself to `fade()`. The script does not define it and the engine does not have
/// it, so the call is refused and every line after it in that frame stops running. Said out loud
/// as a comment instead, so what is missing is visible and the rest of the frame goes on.
/// Declare a field the script READS and never declares.
///
/// A field whose only assignment came from something with no counterpart here loses the
/// assignment, and the name then stands for nothing: reading it is an unknown variable, which
/// takes the frame with it. Declared empty, the read is of nothing instead -- the same nothing
/// the source would have had if its own setup had not run.
fn declare_fields_only_read(script: &str) -> String {
    use regex::Regex;
    use std::collections::BTreeSet;
    static READ: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static DECL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let read = READ.get_or_init(|| Regex::new(r"\b(m_[A-Za-z0-9_]+)\b").expect("read regex"));
    let decl = DECL.get_or_init(|| Regex::new(r"(?m)^\s*var\s+(m_[A-Za-z0-9_]+)\b").expect("decl regex"));
    let declared: BTreeSet<&str> = decl.captures_iter(script).map(|c| c.get(1).unwrap().as_str()).collect();
    let mut missing: Vec<&str> = Vec::new();
    for c in read.captures_iter(script) {
        let n = c.get(1).unwrap().as_str();
        // a field reached through something else belongs to that something, not to this script
        if declared.contains(n) || missing.contains(&n) { continue }
        if script.contains(&format!(".{n}")) && !script.contains(&format!("\n\t{n}"))
            && !script.contains(&format!(" {n} ")) { continue }
        missing.push(n);
    }
    if missing.is_empty() { return script.to_string() }
    // ...and a line that reaches INTO one of them cannot work either: nothing ever put anything
    // there, so asking it for a field or a method is asking nothing. Declared for the plain
    // reads, struck for the reaches, so neither takes the frame down.
    let mut body = String::new();
    for line in script.lines() {
        let t = line.trim();
        let reaches = missing.iter().any(|n| t.contains(&format!("{n}.")));
        if reaches && t.ends_with(';') && !t.contains('{') && !t.starts_with("//") {
            body.push_str(&format!("{}// [nothing ever sets this, so nothing can be read off it] {}\n",
                &line[..line.len() - line.trim_start().len()], t));
            continue;
        }
        body.push_str(line);
        body.push('\n');
    }
    let mut head = String::new();
    for n in &missing {
        head.push_str(&format!("var {n} = null; // [only ever read here: whatever set it has no counterpart]\n"));
    }
    format!("{head}{body}")
}

/// A call nothing can answer evaluates to NOTHING, wherever it sits.
///
/// Striking the whole line only works when the call IS the line; most of them sit inside a test
/// or a sum, and there the line has to stay. So the call itself becomes nothing: the shape of the
/// code is kept, the throw is not. `if (self.isFading())` then reads false, a movement asked of
/// nothing does nothing, and the frame goes on -- which is the difference between a stage that
/// does not do one thing and a stage that stops doing everything.
/// Replace `self.<field>` with its wrapper, but only where `<field>` is the WHOLE name.
///
/// A plain text replace matches the front of a longer name too: a class with a field `open` and
/// a method `openChest` had the method turned into `_s_open.get()Chest()`, which is not a name at
/// all, and the engine then dropped the whole script. The character port learned this the same
/// way. The name has to end where the field ends.
/// A class reads its own field bare as often as it says `self` first.
///
/// The `self.` form is rewritten to the store that holds it; the bare form was left alone, so
/// sand ocean's platform tested `spawnPlatforms` -- a name nothing declares -- and the frame
/// stopped there. Not where it is being CALLED, which is a method of that name and a different
/// thing entirely, and not where it is already part of a longer name.
fn bare_field_read(src: &str, field: &str, with: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(at) = rest.find(field) {
        let before = &rest[..at];
        let after = &rest[at + field.len()..];
        // ...and not where it is already being said as text. A name inside quotes is a name the
        // line already carries, not one being read.
        let quoted = before.ends_with('"') || after.starts_with('"');
        let starts = !quoted && before.chars().next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.'));
        let ends = after.chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let called = after.trim_start().starts_with('(');
        out.push_str(before);
        if starts && ends && !called { out.push_str(with) } else { out.push_str(field) }
        rest = after;
    }
    out.push_str(rest);
    out
}

fn wrap_whole_field(src: &str, field: &str, with: &str) -> String {
    let needle = format!("self.{field}");
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(at) = rest.find(&needle) {
        let after = &rest[at + needle.len()..];
        let whole = !after.starts_with(|c: char| c.is_alphanumeric() || c == '_');
        out.push_str(&rest[..at]);
        if whole { out.push_str(with) } else { out.push_str(&needle) }
        rest = &rest[at + needle.len()..];
    }
    out.push_str(rest);
    out
}

/// A statement whose whole job is a question the match cannot be asked.
///
/// `match.getEnemies()` has no counterpart here, so it comes back as nothing and the loop over
/// its length throws, every frame. Replacing the call with `null` only moves the throw to the
/// next line, so the STATEMENT goes -- and the reads that were waiting on it then fall to the
/// same cascade that handles any other value that was never worked out.
/// Somewhere to keep what the source hangs off a timer.
///
/// The source parks a handler on the timer itself and reads it back when the timer completes:
/// `t.data = {fn: moveUP, next: spawnChunk}`. A timer here is an engine handle, not an open
/// object, so the write is refused and every completed timer takes the frame with it. The bag
/// is the port's to keep, so it keeps it alongside -- looked up BY the timer, which is what the
/// source was really doing, so a handler set on one timer and read through another name (the
/// `currentTimer` the source swings between them) still finds it.
fn timer_handlers_kept_alongside(script: &str) -> String {
    use regex::Regex;
    use std::collections::BTreeSet;
    if !script.contains(".data") { return script.to_string() }
    static DECL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static ASSIGN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let decl = DECL.get_or_init(|| {
        regex::Regex::new(r"var (_t_(\w+)) = [\w.]*makeFrameTimer\(").expect("regex")
    });
    // the names a timer is known by: the variable it was made into, and the field it stands for
    let mut names: BTreeSet<String> = BTreeSet::new();
    for c in decl.captures_iter(script) {
        names.insert(c[1].to_string());
        names.insert(c[2].to_string());
    }
    if names.is_empty() { return script.to_string() }
    // ...and any name the script hands one of those to, which is how the source points a single
    // `currentTimer` at whichever timer is running.
    let assign = ASSIGN.get_or_init(|| {
        regex::Regex::new(r"([\w.]+)\s*=\s*[\w.]*?(\w+)\s*;").expect("regex")
    });
    for _ in 0..2 {
        let found: Vec<String> = assign.captures_iter(script)
            .filter(|c| names.contains(&c[2]))
            .filter_map(|c| c[1].rsplit('.').next().map(str::to_string))
            .collect();
        for f in found { names.insert(f); }
    }
    let tail_is_timer = |recv: &str| {
        recv.rsplit(['.', ' ', '(', ')', '!']).next().is_some_and(|n| names.contains(n))
    };
    let mut out = String::with_capacity(script.len() + 512);
    for line in script.lines() {
        let mut rebuilt = String::with_capacity(line.len() + 32);
        let mut rest = line;
        while let Some(at) = rest.find(".data") {
            let recv_end = at;
            let recv_start = rest[..recv_end].rfind(|c: char| {
                !(c.is_alphanumeric() || c == '_' || c == '.' || c == '[' || c == ']')
            }).map_or(0, |i| i + 1);
            let recv = &rest[recv_start..recv_end];
            if recv.is_empty() || !tail_is_timer(recv) {
                rebuilt.push_str(&rest[..at + ".data".len()]);
                rest = &rest[at + ".data".len()..];
                continue;
            }
            let after = &rest[at + ".data".len()..];
            rebuilt.push_str(&rest[..recv_start]);
            // a write parks the bag; anything else is a read of it
            if let Some(v) = after.strip_prefix(" = ").and_then(|v| v.strip_suffix(';')) {
                rebuilt.push_str(&format!("__setBag({recv}, {v});"));
                rest = "";
                break;
            }
            rebuilt.push_str(&format!("__bagOf({recv})"));
            rest = after;
        }
        rebuilt.push_str(rest);
        out.push_str(&rebuilt);
        out.push('\n');
    }
    if !out.contains("__bagOf(") && !out.contains("__setBag(") { return script.to_string() }
    format!("var __bags = [];
function __bagOf(t) {{
\tvar i = 0;
\twhile (i < __bags.length) {{
\t\tif (__bags[i].t == t) {{ return __bags[i].d; }}
\t\ti = i + 1;
\t}}
\treturn null;
}}
function __setBag(t, d) {{
\tvar i = 0;
\twhile (i < __bags.length) {{
\t\tif (__bags[i].t == t) {{ __bags[i].d = d; return; }}
\t\ti = i + 1;
\t}}
\t__bags.push({{ t: t, d: d }});
}}
{out}")
}

fn strike_unanswerable_match_calls(script: &str) -> String {
    use regex::Regex;
    use std::collections::BTreeSet;
    static DEF: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static ASK: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let def = DEF.get_or_init(|| Regex::new(r"(?m)^function (\w+)\s*\(").expect("def regex"));
    let mine: BTreeSet<&str> = def.captures_iter(script).map(|c| c.get(1).unwrap().as_str()).collect();
    // ANYWHERE IN THE STATEMENT, not only as the whole of it. `match.getPlayer(1)` answers with
    // nothing, and the source goes on to hang a listener off that nothing -- so the statement is
    // undoable whether the ask is the whole of it or the start of a chain through it.
    let ask = ASK.get_or_init(|| {
        regex::Regex::new(r"(?:self\.)?match\.(\w+)\(").expect("regex")
    });
    let mut out = String::with_capacity(script.len());
    for line in script.lines() {
        let t = line.trim();
        let indent = &line[..line.len() - line.trim_start().len()];
        let unanswered = !t.starts_with("//") && ask.captures_iter(t).any(|c| {
            !mine.contains(&c[1]) && !crate::api_mappings::the_engine_answers_to(&c[1])
        });
        if unanswered {
            let name = ask.captures_iter(t)
                .find(|c| !mine.contains(&c[1]) && !crate::api_mappings::the_engine_answers_to(&c[1]))
                .map(|c| c[1].to_string()).unwrap_or_default();
            let net: i32 = t.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum();
            let opener = t.ends_with('{')
                && ["if (", "while (", "} else if ("].iter().any(|k| t.starts_with(k));
            if opener {
                if let (Some(a), Some(b)) = (t.find('('), t.rfind(')')) {
                    if a < b {
                        out.push_str(&format!("{indent}{}false{} // [nothing answers to {name}] {}\n",
                            &t[..=a], &t[b..], &t[a + 1..b]));
                        continue;
                    }
                }
            }
            if net == 0 {
                out.push_str(&format!("{indent}// [nothing here answers to {name}] {t}\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn unanswerable_calls_are_nothing(script: &str) -> String {
    use regex::Regex;
    use std::collections::BTreeSet;
    static DEF: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let def = DEF.get_or_init(|| Regex::new(r"(?m)^function (\w+)\s*\(").expect("def regex"));
    let mine: BTreeSet<&str> = def.captures_iter(script).map(|c| c.get(1).unwrap().as_str()).collect();
    const OPEN: &str = "self.";
    let mut out = String::with_capacity(script.len());
    for line in script.lines() {
        if line.trim_start().starts_with("//") { out.push_str(line); out.push('\n'); continue }
        let mut rest = line;
        let mut done = String::new();
        'scan: while let Some(at) = rest.find(OPEN) {
            // a name, then a bracket: anything else is not a call
            let after = &rest[at + OPEN.len()..];
            let nlen = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').count();
            if nlen == 0 || !after[nlen..].starts_with('(') {
                done.push_str(&rest[..at + OPEN.len()]);
                rest = &rest[at + OPEN.len()..];
                continue;
            }
            let name = &after[..nlen];
            if mine.contains(name) || crate::api_mappings::the_engine_answers_to(name) {
                done.push_str(&rest[..at + OPEN.len() + nlen]);
                rest = &rest[at + OPEN.len() + nlen..];
                continue;
            }
            let from = at + OPEN.len() + nlen + 1;
            let mut depth = 1i32;
            let mut close = None;
            for (i, ch) in rest[from..].char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => { depth -= 1; if depth == 0 { close = Some(from + i); break } }
                    _ => {}
                }
            }
            let Some(close) = close else { break 'scan };
            // AND WHATEVER IS READ OFF IT. A field of nothing is nothing too, so the chain that
            // follows goes with the call -- otherwise the line still reaches into the nothing
            // this just put there.
            let mut tail = close + 1;
            while rest[tail..].starts_with('.') {
                let n = rest[tail + 1..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').count();
                if n == 0 { break }
                tail += 1 + n;
                if rest[tail..].starts_with('(') {
                    let mut d = 1i32;
                    let f = tail + 1;
                    let mut e = None;
                    for (i, ch) in rest[f..].char_indices() {
                        match ch { '(' => d += 1, ')' => { d -= 1; if d == 0 { e = Some(f + i); break } }, _ => {} }
                    }
                    let Some(e) = e else { break 'scan };
                    tail = e + 1;
                }
            }
            // a span holding an unmatched brace cannot be lifted out without changing the shape
            let span = &rest[at..tail];
            if span.matches('{').count() != span.matches('}').count() { break 'scan }
            done.push_str(&rest[..at]);
            done.push_str(&format!("null /* [nothing here answers to {name}] */"));
            rest = &rest[tail..];
        }
        done.push_str(rest);
        out.push_str(&done);
        out.push('\n');
    }
    // a class fetched by name that nothing here answers to (rest area's bitmap data for a tint)
    // is nothing, and what is built from it is nothing too
    let nothing_named = crate::re!(r"(?m)^\s*(\w+) = null /\* \[nothing here answers");
    let nothings: Vec<String> = nothing_named.captures_iter(&out).map(|c| c[1].to_string()).collect();
    for v in nothings {
        out = regex::Regex::new(&format!(r"\b{}\.new\((?:[^()]|\([^()]*\))*\)", regex::escape(&v)))
            .map(|re| re.replace_all(&out, "null").into_owned()).unwrap_or(out);
    }
    out
}

fn strike_calls_nothing_can_answer(script: &str) -> String {
    use regex::Regex;
    use std::collections::BTreeSet;
    static DEF: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static CALL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let def = DEF.get_or_init(|| Regex::new(r"(?m)^function (\w+)\s*\(").expect("def regex"));
    let call = CALL.get_or_init(|| Regex::new(r"^self\.(\w+)\(").expect("call regex"));
    let defined: BTreeSet<&str> = def.captures_iter(script).map(|c| c.get(1).unwrap().as_str()).collect();
    let mut out = String::with_capacity(script.len());
    for line in script.lines() {
        let t = line.trim();
        let unanswerable = call.captures(t).map(|c| {
            let n = c.get(1).unwrap().as_str();
            !defined.contains(n) && !crate::api_mappings::the_engine_answers_to(n)
        }).unwrap_or(false);
        // only a whole statement, so nothing around it is disturbed
        if unanswerable && t.ends_with(");") && !t.contains('{') {
            out.push_str(&format!("{}// [nothing here answers to this] {}\n",
                &line[..line.len() - line.trim_start().len()], t));
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// `x is SomeClass` -- the source asking what something IS.
///
/// AS3 has the test and hscript does not, so the words go out as written and the script will not
/// parse: the engine then drops it whole and the stage makes nothing. The port has no types to
/// ask about either -- what comes back from a spawn is the engine's handle whatever class the
/// source built it from -- so the question cannot be answered, only removed. It is removed as
/// TRUE rather than false: the source uses it to sift a mixed list, and the tests beside it
/// still sift. False would switch the whole branch off, which is a bigger change than the port
/// is entitled to make.
fn no_type_test_here(script: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(
        // the thing being tested can be a call, an index, or a chain of both
        r"[A-Za-z_][\w.\[\]]*(?:\([^()]*\))?(?:[\w.\[\]]|\([^()]*\))*\s+is\s+([A-Z]\w*)")
        .expect("is regex"));
    re.replace_all(script, "true /* [no type test here: was `is $1`] */").into_owned()
}

fn art_calls_on_the_object(script: &str) -> String {
    let mut out = script.to_string();
    // ...and asking is the same as telling: a class that turns its art reads the angle back to
    // add to it, and yoshi's island's windmill does both on the same line. the container has
    // neither, so both go to the object.
    for m in ["setVisible", "setAlpha", "setScaleX", "setScaleY", "setX", "setY", "setRotation",
              "getRotation", "getAlpha", "getScaleX", "getScaleY"] {
        for recv in ["self.exports.stance.", "self.stance.",
                     "self.getViewRootContainer().sprite.", "self.getViewRootContainer().",
                     "self.exports.stance.get()."] {
            out = out.replace(&format!("{recv}{m}("), &format!("self.{m}("));
        }
    }
    // ...and the art is as often held in a name for a few lines as reached for on the spot. the
    // windmill takes its container once and then turns it, so the name it took it under carries
    // the same asks, and they land in the same place.
    let held = crate::re!(r"(?m)^\s*(?:var\s+)?(\w+)\s*=\s*self\.getViewRootContainer\(\);");
    let names: Vec<String> = held.captures_iter(&out).map(|c| c[1].to_string()).collect();
    for n in names {
        for m in ["setVisible", "setAlpha", "setScaleX", "setScaleY", "setX", "setY",
                  "setRotation", "getRotation", "getAlpha", "getScaleX", "getScaleY"] {
            out = out.replace(&format!("{n}.{m}("), &format!("self.{m}("));
        }
    }
    out
}

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
    // THE REFERENCE DUMP IS THE SOURCE, TRANSLATED, not the script that runs. It has none of the
    // declarations the port writes around it, so a rule that answers for names nothing declares
    // would report every one of them -- which is wrong about the script that actually loads.
    let reference_only = path.extension().is_some_and(|e| e == "txt");
    let named_value_pass = |c: &str| -> String {
        if reference_only { c.to_string() } else { a_class_named_as_a_value(c) }
    };
    // Last of all, and only if the braces do not already balance: a closing brace struck out from
    // under a block that is still live. Left open, the script runs off the end of the file and the
    // engine drops the whole thing.
    // both directions: a struck opener leaves a live close orphaned, a struck close leaves a live
    // block open. the first is what the character port already had; stage scripts never ran it.
    let body = &crate::api_mappings::restore_struck_closers(
        &crate::api_mappings::balance_commented_blocks(body));
    // Every script goes out through here, which is the one place certainly AFTER all the rewrites
    // -- including the one that sends an object's own fields through `exports`, which is what
    // makes a call on its own art look the way it does by the time it matters.
    // the type test goes FIRST, while the thing being tested is still written the way the source
    // wrote it: the passes below turn an unanswerable call into `null /* ... */`, and `is` after
    // one of those no longer looks like a test of anything.
    let body = &declare_fields_only_read(&unanswerable_calls_are_nothing(
        &strike_calls_nothing_can_answer(&own_functions_called_by_name(
            &art_calls_on_the_object(&no_type_test_here(body))))));
    // ...and then the same one step further. Striking `_v1 = m_hitBoxRect.clone()` because nothing
    // ever set that field leaves `_v1.offset(...)` on the next line still asking a null for a
    // method, every frame. The stage script has always carried the read one step past the write;
    // an object's script needs it for the same reason, and did not have it.
    let body = &strike_reads_of_a_struck_temp(&reads_of_a_local_nothing_writes(
        &strike_unanswerable_match_calls(&timer_handlers_kept_alongside(
            &calls_reaching_through_nothing(&asking_the_answer_of_nothing(
                &reaching_into_nothing(
                    &named_value_pass(&building_what_nothing_can_build(body)))))))));
    // A SLOT IS ONLY EMPTY ONCE EVERYTHING THAT EMPTIES IT HAS RUN. Pac maze fills its edge-wrap
    // from a boundary ask nothing answers, so that line goes -- and only then is the slot one
    // nothing fills. Asked before those strikes, it still looked filled and the read stood.
    let body = &a_pair_written_at_the_call(&a_loop_with_nothing_left_to_do(
        &reads_of_a_slot_nothing_fills(&asking_a_placed_object_for_nothing(body))));
    // AN ELEMENT'S NAME IS NOT A NUMBER. A list written out one field per element names them
    // `f__0`, `f__1`, and where the source's index came through as a number that happens to be
    // whole the part after the point is left sitting against the name -- `f__0.0000.get()`, which
    // reads as a field of a field and stops the line. It is the same element either way.
    let body = &crate::re!(r"(__\d+)\.0+\b")
        .replace_all(body, "${1}").into_owned();
    // A CALL ON NOTHING, LEFT STANDING. A receiver the port could not answer is written as
    // the literal null, and a call through it is a statement that throws on its first frame
    // and every frame after: galaxy tours asked a platform clip it does not have to play a
    // label, and reported it every frame it ran. What the source did is kept in the note.
    let body = &crate::re!(r"(?m)^(\s*)null\.(\w+\([^\n]*\);)\s*$")
        .replace_all(body, "${1}// [no counterpart: the clip this drove is not in this stage] null.${2}").into_owned();
        let body = &a_write_to_a_setter_runs_it(body);
    std::fs::write(path, expand_blocks(body))
}

/// Flatten the decompiler's nested-NEGATION state ladder into the canonical substate shape:
/// `if (!inLocalState(A)) { <deeper ladder> } else { <A handler> }` chains (the natural CFG of a
/// decompiled switch) become the flat banner form
/// `if (inLocalState(A)) { <A> } else if (inLocalState(B)) { <B> } ...`.
/// The deepest rung's fallthrough tail (the all-states-missed `return;`) is dropped. Operates on
/// lines; brace depth skips comment lines (the [needs-port] markers carry unbalanced braces).
fn flatten_state_ladder(code: &str) -> String {
    let open_re = crate::re!(r"^\t*if \(!inLocalState\((-?\d+)\)\) \{$");
    let lines: Vec<String> = code.lines().map(|l| l.to_string()).collect();
    let Some(start) = lines.iter().position(|l| open_re.is_match(l)) else { return code.to_string() };
    // char-wise rung scan from the `if (!inLocalState(N)) {` line: the if-block's close brace can
    // share a line with the else opener (`} else {`), so depth is tracked per character. Returns
    // (if-close line, else-close line if an else block follows).
    let scan_rung = |open: usize| -> Option<(usize, Option<usize>)> {
        let mut depth = 0i32;
        let mut if_close: Option<usize> = None;
        for (j, l) in lines.iter().enumerate().skip(open) {
            if l.trim_start().starts_with("//") { continue; }
            for ch in l.chars() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            match if_close {
                                None => {
                                    if !l.contains("else") { return Some((j, None)); }
                                    if_close = Some(j); // the trailing `{` re-opens to depth 1
                                }
                                Some(ic) => return Some((ic, Some(j))),
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    };
    // walk the ladder: rung at `i` yields (state, handler line range), recursing into the if-block.
    let mut pairs: Vec<(String, Vec<String>)> = Vec::new();
    let mut i = start;
    let mut outer_end = start; // line AFTER the outermost rung (set on the first iteration)
    let mut first = true;
    while let Some(c) = open_re.captures(&lines[i]) {
        let state = c[1].to_string();
        let Some((if_close, else_close)) = scan_rung(i) else { return code.to_string() };
        let (handler, after): (Vec<String>, usize) = match else_close {
            Some(ec) => (lines[if_close + 1..ec].to_vec(), ec + 1),
            None => (Vec::new(), if_close + 1),
        };
        if first { outer_end = after; first = false; }
        pairs.push((state, handler));
        // descend into the if-block: the next rung is its first non-empty line.
        let inner_first = (i + 1..if_close).find(|&j| !lines[j].trim().is_empty());
        match inner_first {
            Some(j) if open_re.is_match(&lines[j]) => { i = j; }
            _ => break,
        }
    }
    if pairs.len() < 2 { return code.to_string(); }
    let indent = lines[start].len() - lines[start].trim_start().len();
    let ind = "\t".repeat(indent);
    let mut flat = vec![format!("{ind}// --------- SUBSTATE SYSTEM ----------")];
    for (k, (state, handler)) in pairs.iter().enumerate() {
        let kw = if k == 0 { format!("{ind}if") } else { format!("{ind}}} else if") };
        flat.push(format!("{kw} (inLocalState({state})) {{"));
        // re-indent the handler to one level under the branch.
        let min_tabs = handler.iter().filter(|l| !l.trim().is_empty())
            .map(|l| l.len() - l.trim_start_matches('\t').len()).min().unwrap_or(0);
        for l in handler {
            if l.trim().is_empty() { flat.push(String::new()); continue; }
            let stripped = if l.len() >= min_tabs { &l[min_tabs..] } else { l.trim_start() };
            flat.push(format!("{ind}\t{stripped}"));
        }
    }
    flat.push(format!("{ind}}}"));
    let mut out: Vec<String> = lines[..start].to_vec();
    out.extend(flat);
    out.extend(lines[outer_end..].iter().cloned());
    out.join("\n")
}

/// Cross-frame state via `self.make*` (a plain `var` re-inits every frame on a game object).
#[allow(clippy::too_many_arguments)]
fn thwomp_multi_script_hx(cols: &[(f64, f64)], spawn_y: f64, shake_amp: f64, fall_v: f64, rise_v: f64, dust_scale: f64, spawn_period: f64, entrance_t: f64, land_wait: f64, entrance_bob: &str, camera: bool, entrance: &str, idle: &str, fall: &str) -> String {
    // camera targeting (SSF2 addToCamera/setCamBoxSize): the camera includes the body while it's
    // engaged; FM's camera frames targets itself, so only add/delete need porting.
    let cam_add = if camera { "\t\t\tmatch.getCamera().addTarget(self);\n" } else { "" };
    let cam_del = if camera { "\t\t\tmatch.getCamera().deleteTarget(self);\n" } else { "" };
    let bob = if entrance_bob.is_empty() { String::new() } else {
        format!("\t\t// entrance bob: the sub-clip's frame scripts (setYSpeed timeline), 30->60fps\n{entrance_bob}\t\tself.setY(self.getY() + m_vy.get());\n")
    };
    let cols_lit = cols.iter().map(|(x, _)| format!("{x:.1}")).collect::<Vec<_>>().join(", ");
    let land_lit = cols.iter().map(|(_, y)| format!("{y:.1}")).collect::<Vec<_>>().join(", ");
    format!(
        "// Thwomp — 1:1 from the SSF2 disasm (stage spawn cycle + Thwomp class), frame-doubled.\n\
         // Native HIT_BOXes (HitboxStats: two half boxes, angles 135/45) damage on contact.\n\n\
         function _prepLocalState(animation:String, ?index:Int=Math.NaN):Int {{\n\
         \tif (!__hasInitLocalStateMachine) {{ Common.initLocalStateMachine(); __hasInitLocalStateMachine = true; }}\n\
         \tif (index != Math.NaN) {{ index = __localStatePrepIndex++; }}\n\
         \tCommon.registerLocalState(index, animation);\n\treturn index;\n}}\n\
         var __hasInitLocalStateMachine = false;\nvar __localStatePrepIndex = -1;\n\
         // one local state per PHASE (several share an animation); toLocalState swaps the anim.\n\
         var LState = {{\n\
         \tUNINITIALIZED: _prepLocalState(\"#n/a\", -1),\n\
         \tREST: _prepLocalState(\"{entrance}\"),\n\
         \tENTRANCE: _prepLocalState(\"{entrance}\"),\n\
         \tFALL: _prepLocalState(\"{fall}\"),\n\
         \tLANDED: _prepLocalState(\"{idle}\"),\n\
         \tRISE: _prepLocalState(\"{idle}\")\n\
         }};\n\n\
         // SSF2 constants, frame-doubled / velocity-converted (see the header comment).\n\
         var COLUMNS = [{cols_lit}];\nvar LAND_YS = [{land_lit}];\nvar SPAWN_Y = {spawn_y:.1};\n\
         var SPAWN_PERIOD = {spawn_period:.0};\nvar ENTRANCE_T = {entrance_t:.0};\nvar FALL_V = {fall_v:.2};\nvar LAND_WAIT = {land_wait:.0};\nvar RISE_V = {rise_v:.2};\n\
         // persistent state (a plain var resets every frame on a custom game object).\n\
         var m_col = self.makeInt(0);\nvar m_timer = self.makeInt(0);\nvar m_vy = self.makeFloat(0.0);\n\
         var m_cycle = self.makeFrameTimer(SPAWN_PERIOD);\nvar m_cool = self.makeFrameTimer(60);\n\
         var m_init = self.makeBool(false);\n\n\
         function initialize() {{\n\tself.setState(PState.ACTIVE);\n\tCommon.toLocalState(LState.REST);\n}}\n\n\
         function update() {{\n\
         \t// match start: park at the spawn point; SSF2's first spawn lands at t=300f (=600 FM),\n\
         \t// so pre-advance the spawn clock by half a period.\n\
         \tif (!m_init.get()) {{\n\
         \t\tm_init.set(true);\n\
         \t\tself.setX(COLUMNS[0]);\n\
         \t\tself.setY(SPAWN_Y);\n\
         \t\tfor (i in 0...Math.floor(SPAWN_PERIOD / 2)) {{ m_cycle.tick(); }}\n\
         \t}}\n\
         \tm_cool.tick();\n\
         \tif (m_cool.completed) {{\n\
         \t\tself.reactivateHitboxes();\n\
         \t\tm_cool.reset();\n\
         \t}}\n\
         \t// spawn-to-spawn clock: SSF2 spawns every 600f (=1200 FM) regardless of phase timing.\n\
         \tm_cycle.tick();\n\n\
         \t// --------- SUBSTATE SYSTEM ----------\n\
         \tif (Common.inLocalState(LState.REST)) {{\n\
         \t\t// resting between spawns (parked at the spawn point above the stage)\n\
         \t\tif (m_cycle.completed) {{\n\
         \t\t\tm_col.set(Random.getInt(0, COLUMNS.length - 1));\n\
         \t\t\tself.setX(COLUMNS[m_col.get()]);\n\
         \t\t\tself.setY(SPAWN_Y);\n\
         \t\t\tm_timer.set(0);\n\
         \t\t\tm_cycle.reset();\n\
         \t\t\tm_vy.set(0);\n\
{cam_add}\
         \t\t\tCommon.toLocalState(LState.ENTRANCE);\n\
         \t\t}}\n\
         \t}} else if (Common.inLocalState(LState.ENTRANCE)) {{\n\
{bob}\
         \t\tm_timer.inc();\n\
         \t\tif (m_timer.get() >= ENTRANCE_T) {{\n\
         \t\t\tCommon.toLocalState(LState.FALL);\n\
         \t\t}}\n\
         \t}} else if (Common.inLocalState(LState.FALL)) {{\n\
         \t\t// constant terminal velocity (SSF2 gravity 30 capped at 30)\n\
         \t\tself.setY(self.getY() + FALL_V);\n\
         \t\tif (self.getY() >= LAND_YS[m_col.get()]) {{\n\
         \t\t\tself.setY(LAND_YS[m_col.get()]);\n\
         \t\t\tm_timer.set(0);\n\
         \t\t\tCommon.toLocalState(LState.LANDED);\n\
         \t\t\tmatch.getCamera().shake({shake_amp:.1});\n\
         \t\t\tmatch.createVfx(new VfxStats({{\n\
         \t\t\t\tspriteContent: \"global::vfx.vfx\",\n\
         \t\t\t\tanimation: GlobalVfx.DUST_POOF,\n\
         \t\t\t\tscaleX: {dust_scale:.1},\n\
         \t\t\t\tscaleY: {dust_scale:.1}\n\
         \t\t\t}}), self);\n\
         \t\t}}\n\
         \t}} else if (Common.inLocalState(LState.LANDED)) {{\n\
         \t\t// the column platform under it sinks; hold (SSF2 waitTimer 90f)\n\
         \t\tm_timer.inc();\n\
         \t\tif (m_timer.get() >= LAND_WAIT) {{\n\
         \t\t\t// LANDED and RISE share an animation, and toLocalState REPLAYS the registered\n\
         \t\t\t// animation from frame 0 — which restarted `idle` mid-playback the moment the\n\
         \t\t\t// thwomp left the ground. SSF2 changes no attack here, so the clip just keeps\n\
         \t\t\t// running; carry the frame across the phase change to match it.\n\
         \t\t\tvar _f = self.getCurrentFrame();\n\
         \t\t\tCommon.toLocalState(LState.RISE);\n\
         \t\t\tself.playFrame(_f);\n\
         \t\t}}\n\
         \t}} else if (Common.inLocalState(LState.RISE)) {{\n\
         \t\t// rise at SSF2 YSpeed -6 until past the spawn point, then rest\n\
         \t\tself.setY(self.getY() - RISE_V);\n\
         \t\tif (self.getY() <= SPAWN_Y) {{\n\
         \t\t\tself.setY(SPAWN_Y);\n\
         \t\t\tm_timer.set(0);\n\
{cam_del}\
         \t\t\tCommon.toLocalState(LState.REST);\n\
         \t\t}}\n\
         \t}}\n\
         }}\n")
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
fn emit_weather(model: &StageModel, lib: &Path) -> Result<(Vec<BootWeather>, String)> {
    let (mut boot, mut per_frame) = (Vec::new(), String::new());
    for (i, w) in model.weathers.iter().enumerate() {
        let (b, f) = emit_one_weather(model, lib, w, i)?;
        boot.extend(b);
        per_frame.push_str(&f);
    }
    Ok((boot, per_frame))
}

fn emit_one_weather(model: &StageModel, lib: &Path, w: &crate::stage_parser::Weather, idx: usize) -> Result<(Option<BootWeather>, String)> {
    // A count of zero means the class's own code did not yield one (the value is neither passed at
    // the construction nor declared as a default). Emitting anyway gives a loop over nothing: the
    // stage carries a weather entity and renders no weather, which looks like it works. Say so
    // instead, so it reads as a known gap rather than a silent one.
    if w.count == 0 {
        log::warn!("stage weather: found the particle ({}x{}) but not how many, so no weather is emitted", w.art.w, w.art.h);
        return Ok((None, String::new()));
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
    // which way the field drifts overall decides which edge a particle returns from
    let falling = motion.dy.mean() > 0.0;
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
    // Weather is attached to the VIEW, not to the stage: it fills the screen wherever the camera
    // goes, and a particle leaving one edge returns at the other. So a particle's stored position
    // is an offset from the camera (which reports the CENTRE of the view), resolved every frame.
    //
    // The field is sized from the camera too, not from SSF2's rectangle scaled up. SSF2's authored
    // area is that engine's screen; Fraymakers' screen is a different shape, so copying the numbers
    // across gives a field that misses the sides and overshoots the top. Sizing off the viewport is
    // what makes it the same field, and because both are "one screenful" the source's particle
    // COUNT carries over unchanged and looks as thick as it did. Re-derived per frame so the field
    // grows and shrinks with the camera instead of tearing at the edges when it pulls out.
    //
    // With no camera to ask, the SSF2 numbers are the fallback: a field over one patch of the stage
    // is wrong, but it is the source's own wrongness rather than a field of zero size.
    // a particle re-enters from the edge it drifts AWAY from, so which edge that is follows the
    // drift the source authored instead of being fixed to one of them.
    let wrap = if falling {
        "\t\t\tif (p.y > hh) { p.y = -hh; p.x = (Random.getInt(0, 200) / 100.0 - 1.0) * hw; }\n\
         \t\t\tif (p.y < -hh) { p.y = -hh; }\n"
    } else {
        "\t\t\tif (p.y < -hh) { p.y = hh; p.x = (Random.getInt(0, 200) / 100.0 - 1.0) * hw; }\n\
         \t\t\tif (p.y > hh) { p.y = hh; }\n"
    };
    let per_frame = format!(
        "\t\tvar cam = match.getCamera();\n\
         \t\tvar camX = 0.0;\n\
         \t\tvar camY = 0.0;\n\
         \t\tvar hw = {hw}.0;\n\
         \t\tvar hh = {hh}.0;\n\
         \t\tif (cam != null) {{\n\
         \t\t\tcamX = cam.getX();\n\
         \t\t\tcamY = cam.getY();\n\
         \t\t\t// scattered over 1.5 screens so particles arrive from off-view rather than\n\
         \t\t\t// appearing at the frame edge. the viewport is reported in WORLD units and already\n\
         \t\t\t// accounts for zoom, so dividing by the zoom scale pins the field to a constant and\n\
         \t\t\t// leaves the sides bare on any shot wider than one screen.\n\
         \t\t\thw = cam.getViewportWidth() * 0.75;\n\
         \t\t\thh = cam.getViewportHeight() * 0.75;\n\
         \t\t}}\n\
         \t\tfor (p in m_weatherFIELDIDX) {{\n\
         \t\t\t// the VFX's OWN alpha: it writes that onto its container every frame, so setting the
         \t\t\t// container instead is overwritten and a field told to fade never does
         \t\t\tp.v.setAlpha(p.a * m_weatherAlphaFIELDIDX);\n\
         			p.y += p.k;\n\
         \t\t\tp.x += p.wind;\n\
         \t\t\t// leaving the field puts it back on the far side, so the count never changes\n\
{wrap}\
         \t\t\tif (p.x < -hw) {{ p.x = hw; }}\n\
         \t\t\tif (p.x > hw) {{ p.x = -hw; }}\n\
         \t\t\tp.v.setX(camX + p.x);\n\
         \t\t\tp.v.setY(camY + p.y);\n\
         \t\t}}\n");
    let tag = idx.to_string();
    Ok((Some(boot), per_frame.replace("FIELDIDX", &tag)))
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
                BgLayerRef { starts_hidden, presence: presence.to_vec(), named_path: named_path.to_vec(), name: format!("{name} (plate)"), eid: plate_id, clip: clip.clone(),
                             frames: vec![plate_ref],
                             plane, segments: Vec::new(), pan, fades, authored_alpha },
                BgLayerRef { starts_hidden, presence: presence.to_vec(), named_path: named_path.to_vec(), name, eid: eid.to_string(), clip, frames: moving_refs, plane, segments: Vec::new(), pan, fades, authored_alpha },
            ]);
        }
    }
    let refs = write_element_frames(eid, frames, !segs.is_empty(), write_layer)?;
    Ok(vec![BgLayerRef { starts_hidden, presence: presence.to_vec(), named_path: named_path.to_vec(), name, eid: eid.to_string(), clip, frames: refs, plane, segments: segs, pan, fades, authored_alpha }])
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

/// A clip name usable as an hscript variable, or `None` when the source name is not one.
/// The stage script's helpers for clips its code makes from the library at run time: a handle
/// the script moves by its fields (`h.x = ..`, a scale flip, a frame read), placed onto its vfx
/// every frame, put into a plane's container and taken down when the source removes it.
fn lib_clip_helpers(table: &[String]) -> String {
    format!("// CLIPS THE STAGE'S CODE MAKES FROM THE LIBRARY AT RUN TIME (`getMCByLinkageName`): each is a\n\
// vfx this port ships and makes on demand, held by a handle the script moves by its fields\n\
// (in the source's units), flips, reads the frame of, and takes down\n\
var __libClips = [{}];\n\
var __libLive = [];\n\
function __libClip(n) {{\n\
\tvar e = null;\n\
\tfor (i in 0...__libClips.length) {{ if (__libClips[i].n == n) {{ e = __libClips[i]; }} }}\n\
\tif (e == null) {{ return null; }}\n\
\tvar h = {{ x: 0.0, y: 0.0, scaleX: 1.0, scaleY: 1.0, visible: true, alpha: 1.0, vs: [], parts: null, exports: null, base: {{ x: 0.0, y: 0.0 }}, plane: \"background\" }};\n\
\tfor (i in 0...e.parts.length) {{\n\
\t\tvar p = e.parts[i];\n\
\t\tvar v = match.createVfx(new VfxStats({{ spriteContent: self.getResource().getContent(p.id), animation: \"active\", x: __sx(0) + p.ax, y: __sy(0) + p.ay, loop: true, timeout: -1, relativeWith: false, resizeWith: false }}));\n\
\t\tif (v != null) {{ self.getBackgroundBehindContainer().addChild(v.getViewRootContainer()); m_owned.push(v); h.vs.push({{ v: v, ax: p.ax, ay: p.ay, w: p.w }}); }}\n\
\t}}\n\
\th.setX = function(px) {{ h.x = __isx(px); }};\n\
\th.setY = function(py) {{ h.y = __isy(py); }};\n\
\th.getX = function() {{ return __sx(h.x); }};\n\
\th.getY = function() {{ return __sy(h.y); }};\n\
\th.setScaleX = function(s) {{ h.scaleX = s; }};\n\
\th.getScaleX = function() {{ return h.scaleX; }};\n\
\th.setScaleY = function(s) {{ h.scaleY = s; }};\n\
\th.getScaleY = function() {{ return h.scaleY; }};\n\
\th.setVisible = function(b) {{ h.visible = b; }};\n\
\th.setAlpha = function(a) {{ h.alpha = a; }};\n\
\th.getCurrentFrame = function() {{ return (h.vs.length > 0) ? Math.ceil(h.vs[0].v.getCurrentFrame() / 2) : 0; }};\n\
\th.getTotalFrames = function() {{ return (h.vs.length > 0) ? Std.int(h.vs[0].v.getTotalFrames() / 2) : 0; }};\n\
\th.playFrameLabel = function(l) {{}};\n\
\th.playFrame = function(f) {{ for (i in 0...h.vs.length) {{ h.vs[i].v.playFrame(Std.int(f) * 2 - 1); }} }};\n\
\th.stop = function() {{ for (i in 0...h.vs.length) {{ h.vs[i].v.pause(); }} }};\n\
\th.play = function() {{ for (i in 0...h.vs.length) {{ h.vs[i].v.resume(); }} }};\n\
\th.destroy = function() {{\n\
\t\tfor (i in 0...h.vs.length) {{ h.vs[i].v.destroy(); }}\n\
\t\th.vs = [];\n\
\t\tvar kept = [];\n\
\t\tfor (i in 0...__libLive.length) {{ if (__libLive[i] != h) {{ kept.push(__libLive[i]); }} }}\n\
\t\t__libLive = kept;\n\
\t}};\n\
\t__libLive.push(h);\n\
\t__libPlace(h);\n\
\treturn h;\n\
}}\n\
// where the handle says the clip is, put onto its pictures: a flip turns the art about the\n\
// clip's own origin, as the source's does\n\
function __libPlace(h) {{\n\
\tfor (i in 0...h.vs.length) {{\n\
\t\tvar e = h.vs[i];\n\
\t\te.v.setX(h.base.x + __sx(h.x) + ((h.scaleX < 0) ? -(e.ax + e.w) : e.ax));\n\
\t\te.v.setY(h.base.y + __sy(h.y) + e.ay);\n\
\t\te.v.setScaleX(h.scaleX);\n\
\t\te.v.setScaleY(h.scaleY);\n\
\t\te.v.setVisible(h.visible);\n\
\t\te.v.setAlpha(h.alpha);\n\
\t}}\n\
}}\n\
function __libSettle() {{ for (i in 0...__libLive.length) {{ __libPlace(__libLive[i]); }} }}\n\
// the plane a made clip is put into, and taken out of: the container that plane's own\n\
// pieces draw in (the background plane sits behind the painted stage)\n\
function __addTo(plane, h) {{\n\
\tif (h == null || h.vs == null) {{ return h; }}\n\
\tvar c = (plane == \"foreground\") ? self.getForegroundStructuresContainer() : ((plane == \"midground\") ? self.getBackgroundContainer() : self.getBackgroundBehindContainer());\n\
\tfor (i in 0...h.vs.length) {{ c.addChild(h.vs[i].v.getViewRootContainer()); }}\n\
\treturn h;\n\
}}\n\
function __removeFrom(h) {{ if (h != null && h.destroy != null) {{ h.destroy(); }} return h; }}\n\
// ...or INTO ANOTHER CLIP (`bg.addChild(star)`): the made clip's place is then inside that\n\
// clip, from its origin, and it draws in that clip's plane\n\
function __addToOf(g, h) {{\n\
\tif (h == null || h.vs == null || g == null) {{ return h; }}\n\
\tif (g.getX != null && g.getY != null) {{ h.base = {{ x: g.getX() - __sx(0), y: g.getY() - __sy(0) }}; }}\n\
\t__addTo((g.plane != null) ? g.plane : \"background\", h);\n\
\t__libPlace(h);\n\
\treturn h;\n\
}}\n", table.join(", "))
}

/// A clip reached through its plane (`getBackground().clocktown_rainsplash_bg`) is the object
/// that took its place: the handle by that name (see `stage_runnable`, which does the same).
fn plane_reach_to_handles(src: &str, clip_handles: &[String]) -> String {
    let mut raw = src.to_string();
    for h in clip_handles {
        let src_name = handle_source_name(h);
        for accessor in ["self.getBackground()", "self.getForeground()", "self.getMidground()",
                         "match.getStage().getBackground()", "match.getStage().getForeground()", "match.getStage().getMidground()"] {
            // a WORD boundary after the name, or `getMidground().barrier` welds itself onto
            // `getMidground().barrierL` and the child comes out `barrier__clipL` (a name nothing
            // declares). the accessor and name are literal, only the tail is a boundary.
            raw = regex::Regex::new(&format!(r"{}\.{}\b", regex::escape(accessor), regex::escape(src_name)))
                .map(|re| re.replace_all(&raw, regex::NoExpand(h)).into_owned()).unwrap_or(raw);
        }
    }
    raw
}

/// A CHILD OF A CLIP THE PORT HAS NO PIECE FOR (`giygasTextBox.giygasSpeech`, a text field the
/// class writes its dialogue into) is asked through `__childOf`, which answers the group's own
/// member when it has one and otherwise a stand-in that takes the class's writes: the source
/// shows text this engine has no field for, and the script must run past it.
fn a_child_the_group_lacks(src: &str, clip_parts: &[(String, Vec<String>)]) -> String {
    const MEMBERS: &[&str] = &["parts", "labels", "name", "plane", "x", "y", "scaleX", "scaleY", "visible", "v", "vs", "base",
        "exports", "getX", "getY", "setX", "setY", "getScaleX", "getScaleY", "setScaleX", "setScaleY", "setVisible", "setAlpha",
        "getAnimation", "getCurrentFrame", "getTotalFrames", "playFrameLabel", "playFrame", "stop", "play", "destroy", "view", "alpha", "width", "height"];
    let handles: std::collections::BTreeSet<&str> = clip_parts.iter().map(|(c, _)| c.as_str()).collect();
    // ...and a factory's field that holds a clip (`it.stadiumScreen = grandstand.stadiumScreen`):
    // a member read through it is the same ask (pokemon colosseum's screen text boxes)
    let it_fields: std::collections::BTreeSet<String> = crate::re!(r"(?m)^\s*it\.(\w+)\s*=\s*([A-Za-z_]\w*)(?:\.[A-Za-z_]\w*)*\s*;")
        .captures_iter(src).filter(|c| handles.contains(&c[2])).map(|c| c[1].to_string()).collect();
    let src: &str = &if it_fields.is_empty() { src.to_string() } else {
        let it_re = crate::re!(r"\bit\.([A-Za-z_]\w*)\.([A-Za-z_]\w*)\b(\s*=[^=]|\s*\(|)");
        it_re.replace_all(src, |c: &regex::Captures| {
            let (f, child, after) = (&c[1], &c[2], &c[3]);
            let assigned = after.trim_start().starts_with('=');
            let called = after.trim_start().starts_with('(');
            if !it_fields.contains(f) || handles.contains(child) || handles.contains(handle_alias(child).as_str()) || MEMBERS.contains(&child) || assigned || called {
                return c[0].to_string();
            }
            format!("__childOf(it.{f}, \"{child}\"){after}")
        }).into_owned()
    };
    let re = crate::re!(r"\b([A-Za-z_]\w*)\.([A-Za-z_]\w*)\b(\s*=[^=]|\s*\(|)");
    re.replace_all(src, |c: &regex::Captures| {
        let (h, child, after) = (&c[1], &c[2], &c[3]);
        if std::env::var("PEPTIDE_TRACE_CHILD").is_ok() && handles.contains(h) {
            eprintln!("[child] {h}.{child} after={after:?} handle_child={} member={}", handles.contains(child), MEMBERS.contains(&child));
        }
        let assigned = after.trim_start().starts_with('=');
        let called = after.trim_start().starts_with('(');
        if !handles.contains(h) || handles.contains(child) || handles.contains(handle_alias(child).as_str()) || MEMBERS.contains(&child) || assigned || called {
            return c[0].to_string();
        }
        format!("__childOf({h}, \"{child}\"){after}")
    }).into_owned()
}

/// A CHAIN OF CLIP NAMES (`globox.eye1.eye`, a class reaching down its own tree) is the pieces
/// under every name in it: each name binds the pieces below it, so the chain is whichever of
/// its names holds the FEWEST pieces that the others all hold too. `globox.eye1.eye` is `eye1`
/// (the one eye's pieces; `eye` alone is both eyes). A chain some name in does not narrow is
/// left as it is.
fn a_chain_of_clip_names(src: &str, clip_parts: &[(String, Vec<String>)]) -> String {
    use std::collections::{BTreeSet, BTreeMap};
    let src = &a_child_the_group_lacks(src, clip_parts);
    if clip_parts.len() < 2 { return src.to_string(); }
    let parts: BTreeMap<&str, BTreeSet<&str>> = clip_parts.iter()
        .map(|(c, ps)| (c.as_str(), ps.iter().map(String::as_str).collect())).collect();
    let chain = crate::re!(r"\b([A-Za-z_]\w*)((?:\.[A-Za-z_]\w*)+)\b");
    chain.replace_all(src, |c: &regex::Captures| {
        // a name in the chain is the source's; a clashing one is reached by its handle's alias
        let names: Vec<String> = std::iter::once(&c[1]).chain(c[2].split('.').filter(|s| !s.is_empty()))
            .map(|s| if parts.contains_key(s) { s.to_string() } else { handle_alias(s) }).collect();
        // every name a clip handle, none of them a plain word the chain then reaches into
        let mut end = 0;
        while end < names.len() && parts.contains_key(names[end].as_str()) { end += 1; }
        if end < 2 { return c[0].to_string(); }
        let head = &names[..end];
        let common: Option<BTreeSet<&str>> = head.iter().map(|n| parts[n.as_str()].clone())
            .reduce(|a, b| a.intersection(&b).copied().collect());
        let Some(common) = common.filter(|s| !s.is_empty()) else { return c[0].to_string(); };
        let pick = head.iter().find(|n| parts[n.as_str()] == common);
        match pick {
            Some(n) => format!("{n}{}", names[end..].iter().map(|s| format!(".{s}")).collect::<String>()),
            None => c[0].to_string(),
        }
    }).into_owned()
}

/// A PAIR THE CAMERA IS HANDED AS A POINT: the engine wants its own Point, in this stage's px,
/// and a timed target's length is source frames, so it doubles like every other timing.
fn camera_points(src: &str) -> String {
    // the camera follows an ENTITY: the source hands it the object's clip, which here is the
    // object itself
    let src = &crate::re!(r"\.(addTarget|removeTarget)\(\s*([A-Za-z_][\w.\[\]]*)\.getViewRootContainer\(\)\s*\)")
        .replace_all(src, ".$1($2)").into_owned();
    let timed = crate::re!(r"\.addTimedTargetPoint\(\s*([A-Za-z_][\w.\[\]]*)\s*,\s*((?:[^()]|\([^()]*\))+)\)");
    let out = timed.replace_all(src, |c: &regex::Captures| format!(".addTimedTargetPoint(__pt({}), ({}) * 2)", &c[1], c[2].trim())).into_owned();
    let plain = crate::re!(r"\.(addTargetPoint|removeTargetPoint)\(\s*([A-Za-z_][\w.\[\]]*)\s*\)");
    plain.replace_all(&out, |c: &regex::Captures| format!(".{}(__pt({}))", &c[1], &c[2])).into_owned()
}

/// A CLASS'S OWN CONSTANTS, as the package states them: what its static initialiser assigns a
/// literal to (`SandOceanVehicle.FOUR_CAR_MODE = 2`). Read off each object's reconstructed
/// script, by the class's name, for every script that names them.
fn class_statics(model: &StageModel) -> Vec<(String, String)> {
    let lit = crate::re!(r#"(?m)^(?:var\s+|self\.)([A-Za-z_]\w*)\s*=\s*(-?\d+(?:\.\d+)?|"[^"]*"|true|false|null);"#);
    let mut out = Vec::new();
    for hz in &model.hazards {
        let (Some(class), Some(rs)) = (hz.class_name.as_deref(), hz.reconstructed_script.as_deref()) else { continue };
        for c in lit.captures_iter(rs) {
            let key = format!("{class}.{}", &c[1]);
            if !out.iter().any(|(k, _)| *k == key) { out.push((key, c[2].to_string())); }
        }
    }
    out
}

/// `Class.NAME` in a script, where that class states NAME as a constant, is the constant.
fn inline_class_statics(src: &str, statics: &[(String, String)]) -> String {
    let mut out = src.to_string();
    for (key, value) in statics {
        if !out.contains(key.as_str()) { continue; }
        out = regex::Regex::new(&format!(r"\b{}\b", regex::escape(key)))
            .map(|re| re.replace_all(&out, value.as_str()).into_owned()).unwrap_or(out);
    }
    out
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
    let mut fields: std::collections::BTreeSet<String> = Default::default();
    if let Some(src) = model.stage_script.as_deref() {
        if let Some(line) = src.lines().find(|l| l.starts_with("// [class-fields] ")) {
            for w in line["// [class-fields] ".len()..].split_whitespace() {
                fields.insert(w.split_once('=').map(|(n, _)| n).unwrap_or(w).to_string());
            }
        }
        for c in crate::re!(r"(?m)^\s*self\.(\w+)\s*=[^=]").captures_iter(src) { fields.insert(c[1].to_string()); }
    }
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
    -> Result<(String, Vec<Value>, Vec<(String, Vec<String>)>)> {
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
            path: layer.named_path.clone(), labels: vec!["active".to_string()], sections: None, carry: None, chain: None,
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
    // A CLIP'S CHILDREN AS ITS FIELDS. The source reaches a child clip through its parent
    // (`stadiumMidground.stadiumScreen`), and a reach that comes out of the passes still spelt
    // that way (the parent handed over as a value first) finds the child on the group itself:
    // every handle whose pieces all sit under another's is that handle's child, under the
    // source's own name for it.
    let handles: std::collections::BTreeSet<&str> = clip_parts.iter().map(|(c, _)| c.as_str()).collect();
    let mut pairs: std::collections::BTreeSet<(String, String)> = Default::default();
    for layer in promoted {
        for w in layer.named_path.windows(2) {
            if let (Some(a), Some(b)) = (clip_handle(&w[0]), clip_handle(&w[1])) {
                if handles.contains(a.as_str()) && handles.contains(b.as_str()) && a != b { pairs.insert((a, b)); }
            }
        }
    }
    // ...and a handle whose pieces all sit under another's (a plane's own children promoted
    // to pieces carry only their own name in their path): the plane holds the child too
    for (child, cps) in &clip_parts {
        let cset: std::collections::BTreeSet<&str> = cps.iter().map(|(n, _)| n.as_str()).collect();
        for (parent, pps) in &clip_parts {
            if parent == child { continue; }
            let pset: std::collections::BTreeSet<&str> = pps.iter().map(|(n, _)| n.as_str()).collect();
            if !cset.is_empty() && cset.is_subset(&pset) { pairs.insert((parent.clone(), child.clone())); }
        }
    }
    // a field set by its name is the script's to write: the runtime has no way to name one
    let mut fields = String::new();
    for (parent, child) in &pairs {
        fields.push_str(&format!("\t\t\tif ({parent} != null && {parent}.parts != null) {{ {parent}.{} = {child}; }}\n", handle_source_name(child)));
    }
    Ok((fields, entries, clip_parts.into_iter().map(|(c, ps)| (c, ps.into_iter().map(|(v, _)| v).collect())).collect()))
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
                      "phases": spans.iter().map(|(n, ..)| n.clone()).collect::<Vec<_>>() }));
    tables.elements.push(BootElement {
        index: i, id: eid.to_string(), segmented: true, x: ox, y: oy,
        at: match layer.pan { Some((px, py)) => BootAt::Pan(px, py), None => layer.plane.boot_at() },
        shown: Vec::new(), tint: None, alpha: None, hidden: layer.starts_hidden, ride: None, follow: None,
        path: layer.named_path.clone(), labels: spans.iter().map(|(n, ..)| n.clone()).collect(),
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
        path: layer.named_path.clone(), labels: spans.iter().map(|(n, ..)| n.clone()).collect(),
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
                          "phases": spans.iter().map(|(n, ..)| n.clone()).collect::<Vec<_>>() }));
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
        path: layer.named_path.clone(), labels: spans.iter().map(|(n, ..)| n.clone()).collect(),
        sections: chain.as_ref().map(|_| (spans.iter().map(|(n, ..)| anim_key(n)).collect(),
            spans.iter().map(|(.., l)| *l).collect())),
        carry: None, chain: None,
    });
    Ok(())
}

/// hscript the stage Script runs to spawn its hazards (createCustomGameObject + position).
/// Spawn lines split by what the SOURCE stage put behind the hazards switch: `(always, only when
/// hazards are on)`. A hazard is gated when the stage class only ever spawns its class inside its
/// own `isHazardsOn` branch; one the stage places regardless stays ungated, because in SSF2 it
/// hurts you regardless.
/// SSF2 puts an entity down in the stage's WORLD, but everything else this port measured came off
/// the background art, which the stage holds at an offset of its own. A spawn the stage states as a
/// constant therefore lands somewhere else entirely unless the two spaces are lined up first --
/// Bowser's Castle drops its lava half the stage above the pool that way.
///
/// Line them up from the content itself. A hazard the stage spawns at a stated place is also a
/// hazard whose art this port already found and measured, so the gap between what the stage SAYS
/// and where the art IS is the offset between the two spaces. Take the median of every hazard that
/// states one; with none stated, the art's own origin stands and nothing moves.
/// Fraymakers hands one script no way to call another's functions: a game object's handle is a
/// typed api, and a name that isn't on it is simply not there. `exports` is the way across -- a
/// value put on it is readable through the handle, functions included. So every converted object
/// publishes its own methods there, and the stage's calls go through it.
///
/// An SSF2 stage leans on this constantly: bomb factory's stage builds a block and immediately
/// tells it `setHome()`, `setCell(col,row)`, `startBomb()`. Without a channel those are three
/// errors a frame and a stage full of blocks that never arrange themselves.
/// the names a script publishes -- the same list `exports_publication` writes out.
/// The functions this script can answer with a VALUE rather than do: a body that is a single
/// `return` computes something and changes nothing, so it is safe to evaluate every frame and hand
/// the answer across. Everything else has an effect and must be asked for instead (`command_mailbox`).
fn value_functions(script: &str) -> Vec<String> {
    crate::re!(r"(?m)^(?://\s*\[getter\]\n)?function\s+(\w+)\s*\(\s*\)\s*\{\s*\n\s*return\s+[^;]+;\s*\n\}")
        .captures_iter(script).map(|c| c[1].to_string()).collect()
}

fn published_values(script: &str) -> Vec<String> { value_functions(script) }

fn published_methods(script: &str) -> Vec<String> {
    crate::re!(r"(?m)^\s*self\.exports\.(\w+)\s*=")
        .captures_iter(script)
        .map(|c| c[1].to_string())
        .collect()
}

/// A field of the object's the STAGE writes (bomb factory's stage marks a block for its special
/// event as it makes it) has to live on the channel too, or the object goes on reading its own
/// copy and never sees the write. So a plain module var becomes a channel field, reads and all.
fn vars_through_channel(script: &str) -> String {
    let decl = crate::re!(r"(?m)^var\s+([A-Za-z]\w*)\s*=\s*([^;\n]+);");
    let names: Vec<(String, String)> = decl.captures_iter(script)
        // a persistent slot or a frame timer is the engine's own storage and stays as it is
        .filter(|c| !c[2].contains("self.make"))
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect();
    let mut out = script.to_string();
    for (name, init) in &names {
        out = out.replace(&format!("var {name} = {init};"), &format!("self.exports.{name} = {init};"));
        // a bare read of the name, never one already qualified by a dot
        out = crate::dyn_re(&format!(r"(^|[^.\w]){}\b", regex::escape(name)))
            .replace_all(&out, format!("${{1}}self.exports.{name}"))
            .into_owned();
        // the declaration itself must not have been rewritten into a self-reference
        out = out.replace(&format!("self.exports.self.exports.{name}"), &format!("self.exports.{name}"));
    }
    out
}

/// The signature of each function a script defines: name and parameter count.
fn script_functions(script: &str) -> Vec<(String, usize)> {
    crate::re!(r"(?m)^function\s+(\w+)\s*\(([^)]*)\)")
        .captures_iter(script)
        .map(|c| {
            let args = c[2].trim();
            (c[1].to_string(), if args.is_empty() { 0 } else { args.split(',').count() })
        })
        .collect()
}

/// The floor an object or a fighter stands on, asked something: the source's floor is the object
/// that put it there, this engine's a face handle with no such name. A value read (`get…`,
/// `is…`, `has…`) is the owner's published value of that name; a call, or a call on the floor's
/// view root (its clip), goes through the ask channel to the owner; and nothing owns a plain
/// terrain face, which answers null and does nothing, as the source's plain platform does.
const FLOOR_HELPERS: &str = "// the object whose deck a floor face is, if any object publishes it\n\
function __floorOwner(h) {\n\
\tif (h == null) { return null; }\n\
\tvar all = match.getCustomGameObjects();\n\
\tif (all == null) { return null; }\n\
\tvar i = 0;\n\
\twhile (i < all.length) {\n\
\t\tvar o = all[i];\n\
\t\tif (o != null && o.exports != null && o.exports.__deck == h) { return o; }\n\
\t\ti = i + 1;\n\
\t}\n\
\treturn null;\n\
}\n\
// a value the floor's owner publishes, read by a function of its exports\n\
// a number asked of a floor: what no owner says, a plain platform answers with zero\n\
function __floorNum(h, f) {\n\
\tvar v = __floorGet(h, f);\n\
\treturn (v == null) ? 0 : v;\n\
}\n\
function __floorGet(h, f) {\n\
\tvar o = __floorOwner(h);\n\
\tif (o == null || o.exports == null) { return null; }\n\
\treturn f(o.exports);\n\
}\n";

fn route_floor_calls(body: &str) -> String {
    let recv = r"((?:[\w.\[\]]+(?:\(\))?)+)\.getCurrentFloor\(\)";
    let args = r"((?:[^()]|\((?:[^()]|\([^()]*\))*\))*)";
    let on_clip = crate::dyn_re(&format!(r"{recv}\.getViewRootContainer\(\)\.(\w+)\({args}\)"));
    let out = on_clip.replace_all(body, "__ask(__floorOwner($1.getCurrentFloor()), \"$2\", [$3])").into_owned();
    // a read the class does sums or number tests on (`getBounceSpeed() != 0`, `* -0.9`) is a
    // number, and a floor nobody owns answers it as a plain platform does: with zero
    let read_num = crate::dyn_re(&format!(r"{recv}\.((?:get|is|has)\w*)\(\)(\s*(?:[-+*/]|[!=<>]=?\s*-?\d))"));
    let out = read_num.replace_all(&out, "__floorNum($1.getCurrentFloor(), function(e) { return e.$2; })$3").into_owned();
    let read = crate::dyn_re(&format!(r"{recv}\.((?:get|is|has)\w*)\(\)"));
    let out = read.replace_all(&out, "__floorGet($1.getCurrentFloor(), function(e) { return e.$2; })").into_owned();
    let call = crate::dyn_re(&format!(r"{recv}\.(\w+)\({args}\)"));
    call.replace_all(&out, |c: &regex::Captures| {
        if crate::api_mappings::the_engine_answers_to(&c[2]) || c[2].starts_with("__") { return c[0].to_string(); }
        format!("__ask(__floorOwner({}.getCurrentFloor()), \"{}\", [{}])", &c[1], &c[2], c[3].trim())
    }).into_owned()
}

/// A method another script asks for, run by THIS one.
///
/// A function put on `exports` can be called from outside, but it cannot see the script that
/// defined it: `isBrick()` handed across came back "Unknown variable: inLocalState". Values cross
/// intact, so the ask crosses as a value -- a name and its arguments -- and the object runs the
/// real function itself, in its own scope, on its next frame.
/// A placed object takes its first step after the stage's setup has run: in the source the
/// stage's constructor runs before any frame, so nothing an object does comes before what the
/// setup told it (saffron city's platforms walked to no target for the frames before). The
/// stage marks every object it named once its setup is through (the stage handle the engine
/// gives an object carries no exports to read a flag off).
const GATE: &str = "if (self.exports.__setup != true) { return; }";

fn command_mailbox(script: &str, placed: bool) -> String {
    let skip = ["initialize", "update", "onTeardown", "onKill", "onStale", "__refreshExports",
                "afterPushState", "afterPopState", "afterFlushStates"];
    let values = value_functions(script);
    let fns: Vec<(String, usize)> = script_functions(script).into_iter()
        .filter(|(n, _)| !skip.contains(&n.as_str()) && !n.starts_with("__")
                      && !values.iter().any(|v| v == n))
        .collect();
    if fns.is_empty() { return String::new(); }
    let mut arms = String::new();
    for (n, argc) in &fns {
        let args: Vec<String> = (0..*argc).map(|i| format!("c.a[{i}]")).collect();
        arms.push_str(&format!("\t\tif (c.n == \"{n}\") {{ {n}({}); }}\n", args.join(", ")));
    }
    // ...and what another script asks of this object's own clip (a label played on the floor a
    // fighter stands on): the engine's own calls, on this object
    for n in ["playFrameLabel", "playAnimation", "setVisible"] {
        if !fns.iter().any(|(f, _)| f == n) {
            arms.push_str(&format!("\t\tif (c.n == \"{n}\") {{ self.{n}(c.a[0]); }}\n"));
        }
    }
    // A PLACED OBJECT IS WHERE THE TREE PUT IT UNTIL THE MATCH IS LIVE. Measured in the source:
    // a placed platform asked to go to its path's start during the stage's setup stays put and
    // walks its path from where it was placed. So an ask made before the match was live runs,
    // and the place it wrote for the object is dropped.
    if placed {
        arms.push_str("\t\tif (c.pre == true) { __wx = null; __wy = null; }\n");    }
    format!("\n// what other scripts asked this one to do. A function cannot be called from outside and\n\
             // still see this scope, so the ASK crosses as a value -- a name and its arguments -- and\n\
             // the doing happens here, in order, on the next frame.\n\
             function __runCommand() {{\n\
             {build}\
             \tvar q = self.exports.__q;\n\
             \tif (q == null) {{ return; }}\n\
             \twhile (q.length > 0) {{\n\
             \t\tvar c = q.shift();\n{arms}\t}}\n}}\n",
             build = if script.contains("function __buildOnce()") { "\t__buildOnce();\n" } else { "" })
}

fn exports_publication(script: &str) -> String {
    let fn_re = crate::re!(r"(?m)^function\s+(\w+)\s*\(");
    let names: Vec<&str> = fn_re.captures_iter(script)
        .map(|c| c.get(1).expect("name").as_str())
        // the engine's own lifecycle hooks are called by the engine, never across
        .filter(|n| !matches!(*n, "initialize" | "update" | "onTeardown" | "onKill" | "onStale"
                                | "afterPushState" | "afterPopState" | "afterFlushStates"))
        .collect();
    // A getter is published as a VALUE, refreshed every frame: a function handed across the channel
    // cannot reach back into the script that defined it, so `block.xCell` has to already BE a number
    // by the time the stage reads it.
    let getters: Vec<String> = value_functions(script);
    if names.is_empty() && getters.is_empty() { return String::new(); }
    let mut out = String::from(
        "\n\n// what another script can call on this object. FM gives a handle only its api, so a\n\
         // method of this script's own reaches the outside through `exports` or not at all.\n");
    for n in names.iter().filter(|n| !getters.iter().any(|g| g == *n)) {
        out.push_str(&format!("self.exports.{n} = {n};\n"));
    }
    for g in &getters {
        // declared empty, not called: at module scope the object's own state slots do not exist
        // yet, and asking then is a null access. The per-frame refresh fills it.
        out.push_str(&format!("self.exports.{g} = null;\n"));
    }
    if script.contains("function __runCommand()") {
        // only if it isn't already there. Module scope runs when the script first evaluates, which
        // is AFTER whoever made this object queued its first asks -- bomb factory's stage says
        // `setHome` and `setCell(col,row)` the moment it builds a block. Reset, those were thrown
        // away, so every block believed it sat at cell (0,0): one explosion's cross then matched
        // the whole grid and cleared it, and the stage only ever saw one cell occupied.
        out.push_str("if (self.exports.__q == null) { self.exports.__q = []; }\n");
    }
    out
}

/// Keep the published values current: a getter answers about state that changes, so the number on
/// the channel is refreshed each frame rather than frozen at whatever it was when the script loaded.
fn getter_refresh(script: &str) -> String {
    let getters = value_functions(script);
    if getters.is_empty() { return String::new(); }
    let body: String = getters.iter()
        .map(|g| format!("\tself.exports.{g} = {g}();\n")).collect();
    format!("\n// the values this object publishes, kept current\nfunction __refreshExports() {{\n{body}}}\n")
}

/// The stage's calls onto the objects it makes, routed through the channel those objects publish
/// (see `exports_publication`). A call is routed when the variable it sits on was assigned from
/// `createCustomGameObject` and the name is one that object's own script defines -- so an api call
/// (`setX`, `destroy`) is left exactly as it is, and only the object's own methods are redirected.
/// A stage asks the match for an object it PLACED, by the name it placed it under
/// (`match.getPlatform("left_plat")`). This engine has no such ask, so the stage keeps its own
/// list (see `__nameObject`) and the ask reads it. The line is restored from the strike-out the
/// translation gave it, and early: a slot filled this way is written, and the passes that look
/// for a slot nothing writes run before the object calls are routed (jungle island's barrel
/// paths were struck as reads of an unwritten slot).
fn named_asks(code: &str) -> String {
    let ask_named = crate::re!(r#"(?m)^(\s*)(?://\s*\[[A-Za-z0-9-]+(?::\s*\w+)?\]\s*)?(.*?)match\.getPlatform\(([^()]*(?:\([^()]*\))?[^()]*)\)(.*)$"#);
    // (a line can ask twice: home-run contest reads both barriers in one condition)
    let mut asked = code.to_string();
    loop {
        let next = ask_named.replace_all(&asked, |c: &regex::Captures| {
            format!("{}{}__objectNamed({}){}", &c[1], &c[2], c[3].trim(), &c[4])
        }).into_owned();
        if next == asked { break; }
        asked = next;
    }
    // A BLOCK THE STRIKE TOOK WITH THE ASK COMES BACK WITH IT. The strike commented the condition
    // and every line under it; restoring the condition alone left a live `if` over a dead body
    // (race to the finish's springboard, its conveyors, its mines).
    let mut out: Vec<String> = Vec::new();
    let mut depth: i32 = 0;
    // a block's else came back too: `} else {` right after the close, whether the strike wrote it
    // on its own line or squeezed it behind the close it kept
    let braces = |x: &str| x.matches('{').count() as i32 - x.matches('}').count() as i32;
    let mut just_closed = false;
    for (orig, now) in code.lines().zip(asked.lines()) {
        let t = now.trim_start();
        let indent = &now[..now.len() - t.len()];
        if depth > 0 {
            if let Some(rest) = t.strip_prefix("} // [SSF2-dead] } ") {
                // the close the strike kept, with the else it struck behind it
                depth = depth - 1 + braces(rest);
                out.push(format!("{indent}}} {rest}"));
                if depth <= 0 { depth = 0; just_closed = true; }
                continue;
            }
            if let Some(rest) = t.strip_prefix("// [SSF2-dead] ") {
                depth += braces(rest);
                out.push(format!("{indent}{rest}"));
                if depth <= 0 { depth = 0; just_closed = true; }
                continue;
            }
            // a live line inside the block counts its own braces
            depth += braces(t.split("//").next().unwrap_or(""));
            out.push(now.to_string());
            if depth <= 0 { depth = 0; just_closed = true; }
            continue;
        }
        if just_closed {
            if let Some(rest) = t.strip_prefix("// [SSF2-dead] ").filter(|r| r.starts_with("} else") || r.starts_with("else")) {
                depth = braces(rest).max(0) + if rest.starts_with("} else") { 1 } else { 0 };
                out.push(format!("{indent}{rest}"));
                just_closed = depth == 0;
                continue;
            }
        }
        just_closed = false;
        let restored = orig != now && !t.starts_with("//");
        if restored && t.ends_with('{') {
            depth = 1;
        }
        out.push(now.to_string());
    }
    let mut joined = out.join("\n");
    if asked.ends_with('\n') { joined.push('\n'); }
    joined
}

fn route_object_calls(code: &str, methods_by_id: &std::collections::BTreeMap<String, Vec<String>>,
                      values_by_id: &std::collections::BTreeMap<String, Vec<String>>,
                      placed_by_name: &std::collections::BTreeMap<String, String>) -> String {
    // A stage asks the match for an object it PLACED, by the name it placed it under. Fraymakers
    // has no such ask, so the stage keeps its own list (see `__nameObject`) and the ask reads it.
    // The line is restored from the strike-out the translation gave it: the call HAS somewhere to
    // go now.
    // ...by whatever it calls it: a literal name, or one it reads out of its own layout table.
    let asked = named_asks(code);
    let code = &asked;
    let push_named = crate::re!(r"([\w.\[\]]+)\.push\((__objectNamed\([^;]*\))\)");
    let code = &push_named.replace_all(code, "__pushIfReal($1, $2)").into_owned();
    // A SPAWN CAN COME BACK EMPTY TOO, and a list of things the stage made is no different from a
    // list of things it placed: race to the finish makes four damage zones from a class this
    // package has no content for, and asked each of the four nothings where to stand.
    let push_made = crate::re!(r"([\w.\[\]]+)\.push\((match\.createCustomGameObject\([^;]*\))\)");
    let code = &push_made.replace_all(code, "__pushIfReal($1, $2)").into_owned();
    // A REGION THE STAGE ASKS FOR BY NAME, and the test it runs against it. SSF2 hands back the
    // boundary clip and the stage asks whether a point is inside; here the region is the rectangle
    // it was drawn as, and the test is the same question of it.
    let ask_boundary = crate::re!(r#"(?m)^(\s*)(?://\s*\[[A-Za-z0-9-]+(?::\s*\w+)?\]\s*)?(.*?)match\.getCollisionBoundary\(([^()]*)\)(.*)$"#);
    let code = &ask_boundary.replace_all(code, |c: &regex::Captures| {
        format!("{}{}__objectBoundary({}){}", &c[1], &c[2], c[3].trim(), &c[4])
    }).into_owned();
    // ASKING WHETHER A THING IS IN A REGION, when the source asks it of the thing's whole
    // rectangle. This engine gives an object no rectangle to hand over -- there is no
    // `getBoundsRect` -- so the ask lands on nothing and takes the frame with it. Where the thing
    // IS answers the same question, and that is the boundary test this package already has.
    let hit_rect = crate::re!(r"([\w.\[\]]+)\.hitTestRect\(([\w.\[\]]+)\.getBoundsRect\(\)\)");
    let code = &hit_rect.replace_all(code, "__inBoundary($1, $2.getX(), $2.getY())").into_owned();
    let hit_point = crate::re!(r"([\w.\[\]]+)\.hitTestPoint\(([^;]*?)\)\s*\)");
    let code = &hit_point.replace_all(code, "__inBoundary($1, $2))").into_owned();
    // the attack record carries its own id, so the hit lands once per id rather than once a frame
    let hit_once = crate::re!(r"([\w.\[\]]+)\.addDamage\(([\w.]+)\.damage\)");
    let code = &hit_once.replace_all(code, "__hitOnce($1, $2.atk_id, $2.damage)").into_owned();
    // which variable holds which object: `V = match.createCustomGameObject(...getContent("ID")...)`
    let made = crate::re!(r#"(?m)^\s*(?:var\s+)?(\w+)\s*=\s*match\.createCustomGameObject\(\s*self\.getResource\(\)\.getContent\("([^"]+)"\)"#);
    let mut out = code.to_string();
    let mut seen: std::collections::BTreeSet<(String, String)> = Default::default();
    for c in made.captures_iter(code) {
        seen.insert((c[1].to_string(), c[2].to_string()));
    }
    // a variable holding a PLACED object is the same kind of handle as one holding a spawned one,
    // so calls on it cross the same channel.
    let named_read = crate::re!(r#"(?m)^\s*(?:var\s+)?(\w+)\s*=\s*__objectNamed\("([^"]+)"\)"#);
    for c in named_read.captures_iter(code) {
        if let Some(id) = placed_by_name.get(&c[2]) { seen.insert((c[1].to_string(), id.clone())); }
    }
    // hand the object the stage it was made by, on the same channel: an SSF2 object calls back
    // into the stage that owns it (bomb factory's block tells the stage which cell it just filled)
    // and there is no other way for it to hold one.
    let mut handed = String::new();
    for line in out.lines() {
        handed.push_str(line);
        handed.push('\n');
        if let Some(c) = made.captures(line) {
            let indent = &line[..line.len() - line.trim_start().len()];
            // guarded: a spawn can come back empty (a stage turns its hazards off, an object it
            // names isn't in the package), and the handover must not be what falls over then.
            handed.push_str(&format!(
                "{indent}if ({v} != null) {{ {v}.exports.__stage = self; }}\n", v = &c[1]));
        }
    }
    out = handed;
    // An array the stage pushes its objects into holds those objects, and a call on an element of
    // it is a call on one of them. Bomb factory keeps every block it makes in `m_blocks` and asks
    // each one `isBrick()` when a bomb goes off -- unrouted, that answers null, no brick is ever
    // broken, the grid only fills, and the match eventually locks up.
    let mut arrays: std::collections::BTreeSet<(String, String)> = Default::default();
    for (var, id) in &seen {
        let push = crate::dyn_re(&format!(r"(\w+)\.push\(\s*{}\s*\)", regex::escape(var)));
        for c in push.captures_iter(code) {
            arrays.insert((c[1].to_string(), id.clone()));
        }
    }
    // ...and an object the stage PLACED goes into such a list the same way, by the name it was
    // placed under. Palace of twilight keeps its pillars in one and moves them together.
    let placed_push = crate::re!(r#"__pushIfReal\(\s*(\w+)\s*,\s*__objectNamed\("([^"]+)"\)"#);
    let mut placed_lists: std::collections::BTreeSet<String> = Default::default();
    for c in placed_push.captures_iter(code) {
        placed_lists.insert(c[1].to_string());
        if let Some(id) = placed_by_name.get(&c[2]) {
            arrays.insert((c[1].to_string(), id.clone()));
        }
    }
    // NOTHING TO CARRY THE ASK, for a list of them. Same as for one object held on its own: what
    // the things in the list CAN do is known, so what they cannot is known too, and a call that
    // nothing can answer is struck rather than left to take the frame.
    // a list of things the stage PLACED counts even when none of them turned out to have a script
    // of its own: art placed by name answers to the engine's handle and nothing else.
    let mut listed: Vec<(String, Option<String>)> =
        arrays.iter().map(|(a, i)| (a.clone(), Some(i.clone()))).collect();
    for a in &placed_lists {
        if !listed.iter().any(|(n, _)| n == a) { listed.push((a.clone(), None)); }
    }
    for (arr, id) in &listed {
        let can: &[String] = id.as_ref().and_then(|i| methods_by_id.get(i)).map(|v| &**v).unwrap_or(&[]);
        let re = crate::dyn_re(&format!(
            r"(?m)^(\s*)(.*\b{}\[[^\]]*\]\.(\w+)\(.*)$", regex::escape(arr)));
        out = re.replace_all(&out, |c: &regex::Captures| {
            let (indent, whole, name) = (&c[1], &c[2], &c[3]);
            let net: i32 = whole.chars().map(|ch| match ch { '{' => 1, '}' => -1, _ => 0 }).sum();
            let answered = can.iter().any(|m| m == name)
                || crate::api_mappings::the_engine_answers_to(name)
                || name == "exports";
            if answered || whole.trim_start().starts_with("//") || net != 0 {
                return format!("{indent}{whole}");
            }
            format!("{indent}// [nothing in that list answers to {name}] {whole}")
        }).into_owned();
    }
    // ...AND A NAME HOLDING ONE OF THEM. The stage takes an element out of the list and keeps it
    // for a few lines -- `_v9000 = blocks[i]` -- and everything it asks of that name is an ask of
    // one of the things in the list. Peach's castle registers a listener and then writes a field
    // the block has not got, one line apart.
    // a list the ENGINE hands back holds the engine's own things, and they answer to its api and
    // to nothing else: the source reads SSF2's own fields off a fighter, and there are none here
    let engine_lists: Vec<(String, Option<String>)> =
        crate::re!(r"(?m)^\s*(?:var\s+)?(\w+)\s*=\s*(?:self\.)?match\.(\w+)\(")
            .captures_iter(&out)
            .filter(|c| crate::api_mappings::the_engine_answers_to(&c[2]))
            .map(|c| (c[1].to_string(), None))
            .collect();
    let mut held: Vec<(String, Option<String>)> = Vec::new();
    for (arr, id) in listed.iter().chain(engine_lists.iter()) {
        let re = crate::dyn_re(&format!(r"(?m)^\s*(?:var\s+)?(\w+)\s*=\s*{}\[", regex::escape(arr)));
        for c in re.captures_iter(&out) {
            held.push((c[1].to_string(), id.clone()));
        }
    }
    for (var, id) in &held {
        let can: &[String] = id.as_ref().and_then(|i| methods_by_id.get(i)).map(|v| &**v).unwrap_or(&[]);
        let re = crate::dyn_re(&format!(
            r"(?m)^(\s*)(.*\b{}\.(\w+)\s*[.(].*)$", regex::escape(var)));
        out = re.replace_all(&out, |c: &regex::Captures| {
            let (indent, whole, name) = (&c[1], &c[2], &c[3]);
            let net: i32 = whole.chars().map(|ch| match ch { '{' => 1, '}' => -1, _ => 0 }).sum();
            let answered = can.iter().any(|m| m == name)
                || crate::api_mappings::the_engine_answers_to(name)
                || name == "exports";
            if answered || whole.trim_start().starts_with("//") || net != 0 {
                return format!("{indent}{whole}");
            }
            format!("{indent}// [nothing in that list answers to {name}] {whole}")
        }).into_owned();
    }
    for (arr, id) in arrays {
        let Some(methods) = methods_by_id.get(&id) else { continue };
        for m in methods {
            // `arr[<anything>].m(…)` -> through the channel
            let call = crate::dyn_re(&format!(r"\b{}(\[[^\]]*\])\.{}\(\s*\)", regex::escape(&arr), regex::escape(m)));
            let is_value = values_by_id.get(&id).is_some_and(|v: &Vec<String>| v.iter().any(|x| x == m));
            out = if is_value {
                call.replace_all(&out, format!("{arr}${{1}}.exports.{m}")).into_owned()
            } else {
                call.replace_all(&out, format!("__ask({arr}${{1}}, \"{m}\", [])")).into_owned()
            };
            // ...AND THE SAME CALL WITH ARGUMENTS. Only the no-argument form was routed, so
            // `m_twilightObjects[i].setPos(x, y)` stayed a call on the engine's handle -- which
            // has no such name -- and palace of twilight's pillars never moved. An argument can
            // be a call of its own, the same as for an object held on its own.
            let calln = crate::dyn_re(&format!(
                r"\b{}(\[[^\]]*\])\.{}\(([^()]*(?:\([^()]*\)[^()]*)*)\)",
                regex::escape(&arr), regex::escape(m)));
            if !is_value {
                out = calln.replace_all(&out, |c: &regex::Captures| {
                    if c[2].trim().is_empty() { return c[0].to_string() }
                    format!("__ask({arr}{}, \"{m}\", [{}])", &c[1], c[2].trim())
                }).into_owned();
            }
            // and `arr[…].m` read as a property: SSF2 gets a value there, so ask for one
            let prop = crate::dyn_re(&format!(r"\b{}(\[[^\]]*\])\.{}\b(?:\s*\()?", regex::escape(&arr), regex::escape(m)));
            out = prop.replace_all(&out, |c: &regex::Captures| {
                if c[0].ends_with('(') { c[0].to_string() } else { format!("{arr}{}.exports.{m}", &c[1]) }
            }).into_owned();
        }
    }
    // NOTHING TO CARRY THE ASK. An object the stage makes does not always come with a script of
    // its own -- hyrule castle's tornado is placed from art and stats alone -- and the stage still
    // calls its class's methods on it. There is nowhere for those to go: the engine's handle has
    // no such name, so the ask answers with nothing and takes the frame. What the object CAN do is
    // known here, so what it cannot is known too.
    let asked: Vec<(String, String)> = seen.iter().map(|(v, i)| (v.clone(), i.clone())).collect();
    for (var, id) in &asked {
        let can: &[String] = methods_by_id.get(id).map(|v| &**v).unwrap_or(&[]);
        // ...whether the name is being CALLED or read and then used. `_v9000.touchCealing.set(0)`
        // reads a name the object has not got and then asks the nothing it answered with for a
        // method, which is the same undoable line one step longer.
        let re = crate::dyn_re(&format!(
            r"(?m)^(\s*)(.*\b{}\.(\w+)\s*[.(].*)$", regex::escape(var)));
        out = re.replace_all(&out, |c: &regex::Captures| {
            let (indent, whole, name) = (&c[1], &c[2], &c[3]);
            let net: i32 = whole.chars().map(|ch| match ch { '{' => 1, '}' => -1, _ => 0 }).sum();
            let answered = can.iter().any(|m| m == name)
                || crate::api_mappings::the_engine_answers_to(name)
                || name == "exports";
            if answered || whole.trim_start().starts_with("//") || net != 0 {
                return format!("{indent}{whole}");
            }
            format!("{indent}// [nothing it was made from answers to {name}] {whole}")
        }).into_owned();
    }
    for (var, id) in seen {
        let Some(methods) = methods_by_id.get(&id) else { continue };
        for m in methods {
            if values_by_id.get(&id).is_some_and(|v: &Vec<String>| v.iter().any(|x| x == m)) {
                // it answers with a value, so read one
                let q = crate::dyn_re(&format!(r"\b{}\.{}\(\s*\)", regex::escape(&var), regex::escape(m)));
                out = q.replace_all(&out, format!("{var}.exports.{m}")).into_owned();
                continue;
            }
            // no arguments: the ask is just the name
            let call0 = crate::dyn_re(&format!(r"\b{}\.{}\(\s*\)", regex::escape(&var), regex::escape(m)));
            out = call0.replace_all(&out, format!("__ask({var}, \"{m}\", [])")).into_owned();
            // with arguments: the arguments cross first, then the name
            // AN ARGUMENT CAN BE A CALL OF ITS OWN. Matching only argument text with no brackets
            // in it left `setBoundary(__objectBoundary("left"))` unrouted, so it stayed a call on
            // the engine's handle -- which has no such method, and the ask took the rest of the
            // stage's setup with it. One level of nesting covers what the sources actually write.
            let calln = crate::dyn_re(&format!(
                r"\b{}\.{}\(([^()]*(?:\([^()]*\)[^()]*)*)\)", regex::escape(&var), regex::escape(m)));
            out = calln.replace_all(&out, |c: &regex::Captures| {
                format!("__ask({var}, \"{m}\", [{}])", c[1].trim())
            }).into_owned();
            let field = crate::dyn_re(&format!(r"\b{}\.{}\s*=", regex::escape(&var), regex::escape(m)));
            out = field.replace_all(&out, format!("{var}.exports.{m} =")).into_owned();
        }
    }
    // A CALL THAT NOW HAS SOMEWHERE TO GO IS NOT AN UNKNOWN CALL.
    //
    // The translation pass strikes out anything it does not recognise, and one object calling
    // ANOTHER object's own method is exactly that: the name belongs to the converted class, not to
    // any api. It is struck out before this pass runs, and this pass then rewrites the call inside
    // the comment -- so the wiring is emitted, the channel is built, and the line that would use
    // it stays commented. Clocktown told its meteors to rewind that way and none of them heard it.
    //
    // So a line struck out for a method the channel now carries is restored. Named, not guessed:
    // the symbol in the comment has to be the one being asked for on that same line.
    // ...and the same for a line whose whole remaining content is a call this port DOES have: the
    // reach that was struck out (into a display tree Fraymakers has none of) has been rewritten to
    // land on the object that clip belongs to, so the line runs.
    let struck_obj = crate::re!(r"(?m)^(\s*)// \[SSF2-only: \w+\] (__object\w+\(.*)$");
    out = struck_obj.replace_all(&out, "$1$2").into_owned();
    let struck = crate::re!(r#"(?m)^(\s*)// \[SSF2-only: (\w+)\] (.*__ask\([^"]*"(\w+)".*)$"#);
    out = struck.replace_all(&out, |c: &regex::Captures| {
        if c[2] == c[4] { format!("{}{}", &c[1], &c[3]) } else { c[0].to_string() }
    }).into_owned();
    out
}

/// Does the class take itself off the stage? An object that ends its own life cannot be a template:
/// the template plays an animation and waits to be told, so the object simply never leaves. Bomb
/// factory's explosion destroys itself when its clip finishes -- templated, every blast left one
/// behind, and they piled up for the whole match.
fn class_ends_its_own_life(hz: &Hazard) -> bool {
    hz.reconstructed_script.as_deref()
        .is_some_and(|s| s.contains(".destroy()") || s.contains(".kill()"))
}

/// Does the stage call a method on this object that only its own class defines? The stage's code
/// names the object through whatever it assigned the spawn to, so follow that variable, and count
/// only the names the class itself declares -- `setX` is the engine's and every object has it.
fn stage_calls_own_methods(hz: &Hazard, model: &StageModel) -> bool {
    let (Some(class), Some(recon)) = (hz.class_name.as_deref(), hz.reconstructed_script.as_deref())
        else { return false };
    let script = model.stage_script.as_deref().unwrap_or("");
    let own: std::collections::BTreeSet<&str> = crate::re!(r"(?m)^function\s+(\w+)\s*\(")
        .captures_iter(recon)
        .map(|c| c.get(1).expect("name").as_str())
        .collect();
    crate::dyn_re(&format!(r"(\w+)\s*=\s*[\w.]*spawnEnemy\(\s*{}\s*\)", regex::escape(class)))
        .captures_iter(script)
        .any(|c| {
            let var = regex::escape(&c[1]);
            crate::dyn_re(&format!(r"\b{var}\.(\w+)\("))
                .captures_iter(script)
                .any(|m| own.contains(&m[1]))
        })
}

/// A faller the stage drops on a cycle -- the thwomp -- is emitted as ONE object that re-runs that
/// cycle itself, off the same columns and period the stage states. The stage's own spawn would then
/// stand a SECOND one up every time round, and they pile up: six of them a minute in. Keep the
/// object that recycles, drop the spawn that duplicates it, and the placement that follows it.
fn drop_recycled_spawns(code: &str, ids: &[String]) -> String {
    let mut out = String::new();
    let mut lines = code.lines().peekable();
    while let Some(line) = lines.next() {
        let made = ids.iter().find(|id| line.contains(&format!("getContent(\"{id}\")")));
        let Some(id) = made else { out.push_str(line); out.push('\n'); continue };
        let indent = &line[..line.len() - line.trim_start().len()];
        out.push_str(&format!("{indent}// [one recycling object stands in for this cycle's spawns] {}\n",
                              line.trim()));
        let _ = id;
        // whatever it was assigned to is placed on the next lines, and that placement goes too
        if let Some(var) = line.split('=').next().map(str::trim) {
            while lines.peek().is_some_and(|n| {
                let t = n.trim();
                t.starts_with(&format!("{var}.set")) || t.starts_with(&format!("{var} ="))
            }) {
                let n = lines.next().expect("peeked");
                out.push_str(&format!("{}// [with the spawn it placed] {}\n",
                                      &n[..n.len() - n.trim_start().len()], n.trim()));
            }
        }
    }
    out
}

/// A hazard the stage's OWN code makes over and over, so this port stands one up and lets it
/// re-run the cycle instead of leaving a new one behind every time.
///
/// The question is answered by reading the stage class, not by what the object is called: does its
/// update spawn this class on a repeat, rather than placing it once at boot. A stage that builds
/// its object once in `initialize` is not recycling anything; one that builds it inside `update`
/// is doing it on a cadence, and that is the cycle a single object can stand in for.
fn recycles_itself(hz: &Hazard, model: &StageModel) -> bool {
    let (Some(class), Some(script)) = (hz.class_name.as_deref(), model.stage_script.as_deref())
        else { return false };
    // A class that ends its own life is no cycle for one object to stand in for: the stage's
    // spawn is the cycle, and each one it makes goes when its own code says so. Only an object
    // that would otherwise pile up (nothing in its code ends it) is stood up once and recycled.
    if class_ends_its_own_life(hz) { return false }
    let Some(upd) = script.find("function update") else { return false };
    // only what the UPDATE does counts: a spawn in initialize happens once and is not a cycle
    let body = &script[upd..];
    let hits: Vec<String> = crate::dyn_re(&format!(r"(\w+)\s*=\s*[\w.]*spawnEnemy\(\s*{}\s*\)", regex::escape(class)))
        .captures_iter(body)
        .map(|c| c.get(1).expect("var").as_str().to_string())
        .collect();
    hits.iter().any(|var| {
        let var = var.as_str();
        // A stage that KEEPS what it makes is not recycling: bomb factory pushes every block it
        // builds into its own list and they all stand at once, so each spawn is a real object.
        // A stage that lets go of it -- bowser's castle places its faller and never refers to it
        // again -- is running a cycle that one standing object can serve.
        let v = regex::escape(var);
        // kept in a list, copied somewhere that outlives the call, or never a throwaway at all.
        let kept = crate::dyn_re(&format!(r"(\.push\(\s*{v}\s*\)|[\w.]+\s*=\s*{v}\s*;)")).is_match(body)
            // or it was never a throwaway to begin with. the decompiler names a function's own
            // temporaries `_v<n>`; anything with a real name is a FIELD, and a field holds the
            // object for later. clocktown keeps its tingle in one and asks whether the last is
            // still alive before making another, so it is never a stand-in cycle.
            || !crate::re!(r"^_v\d+$").is_match(var);
        !kept
    })
}

fn entity_origin(model: &StageModel) -> (f64, f64) {
    let script = model.stage_script.as_deref().unwrap_or("");
    let (mut dx, mut dy) = (Vec::new(), Vec::new());
    for hz in &model.hazards {
        let Some(class) = hz.class_name.as_deref() else { continue };
        // `V = SSF2API.spawnEnemy(Class)` and then the place it states for V
        let spawn = crate::dyn_re(&format!(
            r"(\w+)\s*=\s*[\w.]*spawnEnemy\(\s*{}\s*\)", regex::escape(class)));
        let Some(c) = spawn.captures(script) else { continue };
        let v = regex::escape(&c[1]);
        let at = |axis: &str| crate::dyn_re(&format!(r"{v}\.set{axis}\(\s*(-?[\d.]+)\s*\)"))
            .captures(script)
            .and_then(|m| m[1].parse::<f64>().ok());
        if let Some(x) = at("X") { dx.push(hz.x / model.scale - x); }
        if let Some(y) = at("Y") { dy.push(hz.y / model.scale - y); }
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
    let script: String = std::iter::once(model.stage_script.as_deref().unwrap_or(""))
        .chain(model.hazards.iter().filter_map(|h| h.reconstructed_script.as_deref()))
        .collect::<Vec<_>>().join("\n");
    let script = script.as_str();
    let self_spawned: std::collections::BTreeSet<String> = model.hazards.iter().enumerate()
        .filter_map(|(i, hz)| {
            let c = hz.class_name.as_deref()?;
            // ...unless this port emits it as one object that recycles: then the boot spawn is the
            // only one, and the stage's own is the duplicate that gets dropped instead.
            let hid = hazard_id(&model.id, i, hz);
            // the ask reads the same whether the class is still named as SSF2 wrote it or has
            // already been lowered onto the content this port ships it as
            (!recycles_itself(hz, model)
                && (script.contains(&format!("new {c}("))
                    // a stage names the class outright or as a string, and both are the same ask
                    || script.contains(&format!("spawnEnemy({c}"))
                    || script.contains(&format!("spawnEnemy(\"{c}\""))
                    || script.contains(&format!("spawnItem({c}"))
                    || script.contains(&format!("getContent(\"{hid}\")"))))
                .then_some(hid)
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

fn script_hx(id: &str, _animated: bool, hazard_spawns: &str, boot_spawns: &str, plat_segs: &[(String, Vec<String>)], weather: (&str, &str), weather_n: usize,
             ported: &str, clip_handles: &[String], origin: (f64, f64), scale: f64,
             pan_base: (f64, f64), named_boxes: &std::collections::BTreeMap<String, crate::stage_parser::Rect>,
             named_regs: &std::collections::BTreeMap<String, (f64, f64)>,
             terrain_labels: &[(String, u16)], placed_at: &str) -> String {
    // a port helper reached through a factory's own object (`it.__groupNamed(..)`, from the
    // source's `this[..]` inside a class the stage builds for itself) is the stage's function:
    // the object never carries it (a target test built its damage zones through one and died)
    // (a getter or setter the factory gives its object, `it.__get_data()`, is the object's own)
    let ported_owned = crate::re!(r"\bit\.(__[A-Za-z]\w*)\(").replace_all(ported, |c: &regex::Captures| {
        if c[1].starts_with("__get_") || c[1].starts_with("__set_") { c[0].to_string() } else { format!("{}(", &c[1]) }
    }).into_owned();
    // a member reached on a named object goes through the stub when nothing answers to the name
    let ported_owned = crate::re!(r#"__objectNamed\("([^"]+)"\)\."#).replace_all(&ported_owned, "__namedOr(\"$1\").").into_owned();
    // a clip this stage does not carry, struck where it was driven, is still read in a condition
    // (`null.alpha < 1`): what is not there has the properties of nothing shown
    let ported_owned = crate::re!(r"\bnull\.alpha\b").replace_all(&ported_owned, "1").into_owned();
    let ported_owned = crate::re!(r"\bnull\.visible\b").replace_all(&ported_owned, "false").into_owned();
    let ported_owned = crate::re!(r"\bnull\.(?:x|y|width|height|currentFrame|totalFrames|rotation|scaleX|scaleY)\b").replace_all(&ported_owned, "0").into_owned();
    // a clip under a named clip (`globox.eye1.eye`, the eye a class blinks) is a piece of the
    // named thing this port keeps, and the ask goes to that: a chain of child names collapses
    // onto the named thing (its published values are still reached through `exports`)
    let child_chain = crate::re!(r#"__namedOr\("([^"]+)"\)\.([A-Za-z_]\w*)\.([A-Za-z_])"#);
    let mut ported_owned = ported_owned;
    loop {
        let next = child_chain.replace_all(&ported_owned, |c: &regex::Captures| {
            if &c[2] == "exports" { c[0].to_string() } else { format!("__namedOr(\"{}\").{}", &c[1], &c[3]) }
        }).into_owned();
        if next == ported_owned { break; }
        ported_owned = next;
    }
    // an ask made of an object's CLIP (a label played on the platform's picture as it flies off)
    // goes to whatever draws for it: a surface's or a group's own view answers, and a real
    // object, whose container is the engine's, is asked through its mailbox
    let ported_owned = crate::re!(r#"((?:[\w.\[\]]+(?:\("[^"]*"\))?)+)\.getViewRootContainer\(\)\.(playFrameLabel|gotoAndStop|gotoAndPlay|setVisible)\(((?:[^()]|\([^()]*\))*)\)"#)
        .replace_all(&ported_owned, "__viewAsk($1, \"$2\", [$3])").into_owned();
    // who a NAMED thing ignores: a surface handle keeps the list itself, and a real object with
    // a deck (DAT61's barriers) keeps it on that deck, the face the engine collides with
    let ported_owned = crate::re!(r#"__namedOr\("([^"]+)"\)\.(addIgnoreObject|removeIgnoreObject|clearIgnoreObjects|setIgnoreObjectListInversed)\(((?:[^()]|\([^()]*\))*)\)"#)
        .replace_all(&ported_owned, "__objIgnore(__namedOr(\"$1\"), \"$2\", [$3])").into_owned();
    let ported: &str = &ported_owned;
    // the jump of the whole stage to a section, fanned out to every clip group. the stage's own
    // animation is not jumped: its api has no such call (the ask threw on every transition), and
    // what its labels marked is static in the emitted animation anyway; the labels stay as the
    // marks its class reads the plane's clock by
    let group_named: String = clip_handles.iter()
        .map(|h| format!("\tif (n == \"{}\" || n == \"{h}\") {{ return {h}; }}\n", handle_source_name(h)))
        .collect();
    // the stage's own labels on the source's frame count, for a class that reads its plane's currentLabel
    let label_marks: String = terrain_labels.iter()
        .map(|(l, f)| format!("{{ n: \"{}\", f: {} }}", l.replace('"', "\\\""), *f as usize + 1))
        .collect::<Vec<_>>().join(", ");
    // every name the script can answer to, once: a clip group by its var, anything else by lookup
    let plane_children: String = {
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        let mut parts: Vec<String> = Vec::new();
        for h in clip_handles { if seen.insert(h.clone()) { parts.push(format!("{}: {h}", handle_source_name(h))); } }
        for (n, _) in plat_segs { if seen.insert(n.clone()) { parts.push(format!("{n}: __objectNamed(\"{n}\")")); } }
        for n in named_boxes.keys() { if seen.insert(n.clone()) { parts.push(format!("{n}: __objectNamed(\"{n}\")")); } }
        parts.retain(|p| crate::re!(r"^[A-Za-z_]\w*:").is_match(p));
        parts.join(", ")
    };
    let plane_jumps: String = clip_handles.iter()
        .map(|h| format!("\tif ({h} != null) {{ {h}.playFrameLabel(n); }}\n"))
        .collect();
    // the source's coordinates put into this stage's, written once for the ported code to call
    // Camera-background pieces are kept SEPARATE (each on its own timeline, each tintable), which
    // means the engine no longer scrolls them -- so this does. One container per pan rate, made the
    // first time a piece asks for it, moved against the camera by the rate the source declared: a
    // piece that pans at 1 sits still in the world, one at 0 is pinned to the view.
    let pan_helpers = format!(
        "\nvar __panBaseX = {:.2};\nvar __panBaseY = {:.2};\nvar __panRates = [];\nvar __panConts = [];\n\
         var __panX0 = null;\nvar __panY0 = null;\n\
         function __panContainer(px, py) {{\n\
         \tvar i = 0;\n\
         \twhile (i < __panRates.length) {{\n\
         \t\tif (__panRates[i].x == px && __panRates[i].y == py) {{ return __panConts[i]; }}\n\
         \t\ti = i + 1;\n\
         \t}}\n\
         \tvar c = Container.create();\n\
         \tself.getBackgroundBehindContainer().addChild(c);\n\
         \t__panRates.push({{ x: px, y: py }});\n\
         \t__panConts.push(c);\n\
         \treturn c;\n\
         }}\n\
         function __panUpdate() {{\n\
         \tvar cam = match.getCamera();\n\
         \t// against where the camera STARTED, not where it is: the absolute position is a place\n\
         \t// in the world, and using it shoves the whole backdrop that far aside on frame one.\n\
         \tif (__panX0 == null) {{ __panX0 = cam.getX(); __panY0 = cam.getY(); }}\n\
         \tvar dx = cam.getX() - __panX0;\n\tvar dy = cam.getY() - __panY0;\n\
         \tvar i = 0;\n\
         \twhile (i < __panConts.length) {{\n\
         \t\t// centred on the view, then held back by its pan rate: a piece at 0 stays with the\n\
         \t\t// view, one at 1 stays in the world.\n\
         \t\t__panConts[i].x = cam.getX() - __panBaseX * 0.5 - dx * __panRates[i].x;\n\
         \t\t__panConts[i].y = cam.getY() - __panBaseY * 0.5 - dy * __panRates[i].y;\n\
         \t\ti = i + 1;\n\
         \t}}\n\
         }}\n",
        pan_base.0, pan_base.1);
    let coord_helpers = format!(
        "function __sx(v) {{ return (v - ({:.4})) * {:.6}; }}\n\tfunction __sy(v) {{ return (v - ({:.4})) * {:.6}; }}\n\
         // a place read off the engine, in the source's units\n\
         function __isx(v) {{ return v / {:.6} + ({:.4}); }}\n\tfunction __isy(v) {{ return v / {:.6} + ({:.4}); }}\n\
         // a step in the source's units, scaled and nothing else (a place already in this space, moved by one)\n\
         function __sd(v) {{ return v * {:.6}; }}\n\
         // a pair handed to the engine as a point, in this stage's px\n\
         function __pt(p) {{ return Point.create(__sx(p.x), __sy(p.y)); }}\n\
         function __srcLen(v) {{ return v / {:.6}; }}\n\
         // An ask to another object waits a frame before it is handed over. A script is not live the\n\
         // instant its object is made -- the module scope that creates the queue runs later -- so an\n\
         // ask pushed at creation lands in a queue nobody reads, and the object is never told what it\n\
         // is. Bomb factory built blocks that were never started: they sat blank and piled up.\n\
         var __outbox = [];\n\
         // an ask made before the match is live (from the stage's own setup) is marked: a placed\n\
         // object cannot be moved by one, the way the source's tree-placed objects cannot\n\
         var __live = false;\n\
         // an ask of an object that already has its mailbox lands in it now: the object reads it at\n\
         // its next update, which comes after this one the way the source's own updates follow the\n\
         // stage's (saffron city's platforms walked to no target for the frame an outbox cost)\n\
         function __ask(o, n, a) {{\n\
         \tif (o == null) {{ return; }}\n\
         \tif (o.exports != null) {{ if (o.exports.__q == null) {{ o.exports.__q = []; }} o.exports.__q.push({{ n: n, a: a, pre: !__live }}); return; }}\n\
         \t__outbox.push({{ o: o, n: n, a: a, pre: !__live }});\n\
         }}\n\
         function __deliverAsks() {{\n\
         \tvar i = 0;\n\
         \twhile (i < __outbox.length) {{\n\
         \t\tvar e = __outbox[i];\n\
         \t\tif (e.o == null) {{ i = i + 1; }}\n\
         \t\telse {{\n\
         \t\t\t// the queue is made HERE when the object has not made it yet (its own guard keeps what is already there)\n\
         \t\t\tif (e.o.exports == null) {{ i = i + 1; }}\n\
         \t\t\telse {{\n\
         \t\t\tif (e.o.exports.__q == null) {{ e.o.exports.__q = []; }}\n\
         \t\t\te.o.exports.__q.push({{ n: e.n, a: e.a, pre: e.pre }});\n\
         \t\t\ti = i + 1;\n\
         \t\t\t}}\n\
         \t\t}}\n\
         \t}}\n\
         \t__outbox = [];\n\
         }}\n\
         // The objects the tree PLACED, under the names the stage's own code reaches them by. SSF2\n\
         // asks the match for one (`getPlatform(\"left_plat\")`); here the stage keeps its own list,\n\
         // filed as each object is made.\n\
         // THE CLOCK THE SCHEDULES RUN ON. What a stage's clips were drawn against is its own
         // timeline, which advances a frame at a time -- and a stage whose art came out as ONE
         // frame has a timeline that cannot advance at all, so everything keyed to it stands still:
         // battlefield (melee)'s backdrop waits on its second phase forever and the stage shows a
         // black void. So the count is kept here, and the timeline is read when it is really running.
         var __clockFrames = 0;\n\
         function __clock() {{\n\
         \tvar f = self.getCurrentFrame();\n\
         \tif (f > 1) {{ return f; }}\n\
         \treturn __clockFrames;\n\
         }}\n\
         var __namedObjs = [];\n\
         function __nameObject(n, o) {{ if (o != null) {{ __namedObjs.push({{ n: n, o: o }}); }} }}\n\
         // a name nothing answers to, asked something anyway: a clip the class reaches by name that\n\
         // this port folded into flat art (smashville's musician), whose asks go nowhere. an\n\
         // existence test (`!= null`) still sees nothing; only a member reach is answered\n\
         var __namedStub = {{ visible: true, alpha: 1, x: 0, y: 0, exports: null }};\n\
         __namedStub.setVisible = function(v) {{}}; __namedStub.setAlpha = function(a) {{}};\n\
         __namedStub.playFrameLabel = function(l) {{}}; __namedStub.gotoAndStop = function(l) {{}}; __namedStub.gotoAndPlay = function(l) {{}};\n\
         __namedStub.play = function() {{}}; __namedStub.stop = function() {{}};\n\
         __namedStub.getViewRootContainer = function() {{ return __namedStub; }};\n\
         __namedStub.addChild = function(c) {{}}; __namedStub.removeChild = function(c) {{}}; __namedStub.addChildAt = function(c, i) {{}};\n\
         __namedStub.setX = function(v) {{}}; __namedStub.setY = function(v) {{}}; __namedStub.getX = function() {{ return 0; }}; __namedStub.getY = function() {{ return 0; }};\n\
         __namedStub.getCurrentFrame = function() {{ return 0; }}; __namedStub.getTotalFrames = function() {{ return 0; }}; __namedStub.getAnimation = function() {{ return \"\"; }}; __namedStub.playFrame = function(f) {{}};\n\
         // (a region the tree placed under the name answers a place, not an object: its place is\n\
         // carried over onto the stub, and the ask itself goes nowhere)\n\
         function __objIgnore(o, n, a) {{\n\
         \tif (o == null) {{ return; }}\n\
         \tif (n == \"addIgnoreObject\" && o.addIgnoreObject != null) {{ o.addIgnoreObject(a[0]); return; }}\n\
         \tif (n == \"removeIgnoreObject\" && o.removeIgnoreObject != null) {{ o.removeIgnoreObject(a[0]); return; }}\n\
         \tif (n == \"clearIgnoreObjects\" && o.clearIgnoreObjects != null) {{ o.clearIgnoreObjects(); return; }}\n\
         \tif (n == \"setIgnoreObjectListInversed\" && o.setIgnoreObjectListInversed != null) {{ o.setIgnoreObjectListInversed(a[0]); return; }}\n\
         \tvar d = (o.exports != null) ? o.exports.__deck : null;\n\
         \tif (d == null || a[0] == null) {{ return; }}\n\
         \t// the list is kept here too: the engine's remove of a name not on its list is a null it\n\
         \t// reads through, so only what was added is taken off\n\
         \tif (o.exports.__ign == null) {{ o.exports.__ign = []; }}\n\
         \tvar on = o.exports.__ign.indexOf(a[0]) >= 0;\n\
         \tif (n == \"addIgnoreObject\" && !on) {{ o.exports.__ign.push(a[0]); d.addToBlacklist(a[0]); }}\n\
         \tif (n == \"removeIgnoreObject\" && on) {{ o.exports.__ign.remove(a[0]); d.removeFromBlacklist(a[0]); }}\n\
         }}\n\
         function __viewAsk(o, n, a) {{\n\
         \tif (o == null) {{ return; }}\n\
         \tvar v = (o.getViewRootContainer != null) ? o.getViewRootContainer() : null;\n\
         \tif (v != null && n == \"setVisible\" && v.setVisible != null) {{ v.setVisible(a[0]); return; }}\n\
         \tif (v != null && n != \"setVisible\" && v.playFrameLabel != null) {{ v.playFrameLabel(a[0]); return; }}\n\
         \t__ask(o, (n == \"setVisible\") ? \"setVisible\" : \"playFrameLabel\", a);\n\
         }}\n\
         function __namedOr(n) {{\n\
         \tvar o = __objectNamed(n);\n\
         \tif (o == null) {{ return __namedStub; }}\n\
         \tif (o.setVisible == null && o.playFrameLabel == null && o.exports == null) {{\n\
         \t\t__namedStub.x = (o.x != null) ? o.x : 0; __namedStub.y = (o.y != null) ? o.y : 0;\n\
         \t\t__namedStub.getX = function() {{ return __namedStub.x; }}; __namedStub.getY = function() {{ return __namedStub.y; }};\n\
         \t\treturn __namedStub;\n\
         \t}}\n\
         \treturn o;\n\
         }}\n\
         function __objectNamed(n) {{\n\
         \tvar i = 0;\n\
         \twhile (i < __namedObjs.length) {{\n\
         \t\tif (__namedObjs[i].n == n) {{ return __namedObjs[i].o; }}\n\
         \t\ti = i + 1;\n\
         \t}}\n\
         \t// a surface the entity carries under that name answers as one handle...\n\
         \tvar h = __platHandle(n);\n\
         \tif (h != null) {{ return h; }}\n\
         \t// ...and so does a region the tree placed under it (its place is what the code reads)\n\
         \treturn __objectBoundary(n);\n\
         }}\n\
         // THE REGIONS THIS STAGE ASKS FOR BY NAME. SSF2 hands back the boundary clip itself and the
         // stage tests points against it -- crateria's acid spares whoever stands in its `safezone`
         // -- so the same region is kept here, as the rectangle the placement drew.
         var __boxes = [{boxes}];\n\
         function __objectBoundary(n) {{\n\
         \tvar i = 0;\n\
         \twhile (i < __boxes.length) {{\n\
         \t\tif (__boxes[i].n == n) {{\n\
         \t\t\tvar b = __boxes[i];\n\
         \t\t\t// a stage MOVES the regions it keeps (butter building walks its warning edges up\n\
         \t\t\t// with the tower), so the region answers the same asks the source makes of it\n\
         \t\t\tif (b.getX == null) {{\n\
         \t\t\t\t// ...and reads as the rectangle it is (the blast box's width, its y)\n\
         \t\t\t\tb.width = b.w;\n\
         \t\t\t\tb.height = b.h;\n\
         \t\t\t\tb.getX = function() {{ return b.x; }};\n\
         \t\t\t\tb.getY = function() {{ return b.y; }};\n\
         \t\t\t\tb.setX = function(v) {{ b.x = v; }};\n\
         \t\t\t\tb.setY = function(v) {{ b.y = v; }};\n\
         \t\t\t\t// ...and answers what a clip under that name would be asked (a region has no\n\
         \t\t\t\t// picture to show and no surface to switch, so those asks do nothing)\n\
         \t\t\t\tb.visible = true;\n\
         \t\t\t\tb.setVisible = function(v) {{ b.visible = v; }};\n\
         \t\t\t\tb.setAlpha = function(v) {{}};\n\
         \t\t\t\tb.playFrameLabel = function(l) {{}};\n\
         \t\t\t\tb.getAnimation = function() {{ return \"\"; }}; b.getCurrentFrame = function() {{ return 0; }}; b.getTotalFrames = function() {{ return 0; }}; b.playFrame = function(f) {{}};\n\
         \t\t\t\tb.updateStructureStats = function(o) {{}};\n\
         \t\t\t\tb.addIgnoreObject = function(o) {{}};\n\
         \t\t\t\tb.removeIgnoreObject = function(o) {{}};\n\
         \t\t\t\tb.clearIgnoreObject = function() {{}};\n\
         \t\t\t\tb.setIgnoreObjectListInversed = function(v) {{}};\n\
         \t\t\t\tb.getViewRootContainer = function() {{ return {{ visible: b.visible }}; }};\n\
         \t\t\t}}\n\
         \t\t\treturn b;\n\
         \t\t}}\n\
         \t\ti = i + 1;\n\
         \t}}\n\
         \treturn null;\n\
         }}\n\
         // a point is in the region when it is inside the rectangle the region was drawn as\n\
         function __inBoundary(b, px, py) {{\n\
         \tif (b == null) {{ return false; }}\n\
         \tvar x = __sx(px); var y = __sy(py);\n\
         \treturn x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h;\n\
         }}\n\
         function __objectVisible(n, v) {{\n\
         \tvar o = __objectNamed(n);\n\
         \tif (o != null) {{ o.setVisible(v); }}\n\
         }}\n\
         // A list the stage fills with the objects it looks up holds the ones it FOUND. An object\n\
         // this port does not have is absent from the list rather than a hole in it, which is what\n\
         // the code that walks the list a moment later can survive.\n\
         function __pushIfReal(list, o) {{ if (o != null) {{ list.push(o); }} }}\n\
         // every occurrence of one spelling replaced by another (the source's global RegExp replace)\n\
         function __replaceAll(s, p, r) {{ if (s == null || p == null || p == \"\") {{ return s; }} var out = \"\"; var t = Std.string(s); var i = 0; while (i < t.length) {{ if (t.substr(i, p.length) == p) {{ out = out + r; i = i + p.length; }} else {{ out = out + t.charAt(i); i = i + 1; }} }} return out; }}\n\
         // ONE HIT PER SWING. SSF2 stamps its attack with an id and refreshes that id on a timer;
         // a fighter takes the attack once per id, however many frames it is standing in it. The
         // damage is added the same way here -- without it, a stage that burns you every twelfth
         // frame burns you sixty times a second, and crateria's acid took a fighter to 999%.
         var __uid = 0;\n\
         function __nextUid() {{ __uid = __uid + 1; return __uid; }}\n\
         // a for-each loop walks a list by index: 1-based next entry, 0 when there is none\n\
         function __iterNext(o, i) {{ if (o == null || o.length == null) {{ return 0; }} return i < o.length ? i + 1 : 0; }}\n\
         function __iterValue(o, i) {{ return (o == null || i < 1) ? null : o[i - 1]; }}\n\
         function __iterName(o, i) {{ return i - 1; }}\n\
         var __hitBy = [];\n\
         function __hitOnce(o, id, dmg) {{\n\
         \tif (o == null) {{ return; }}\n\
         \tvar i = 0;\n\
         \twhile (i < __hitBy.length) {{\n\
         \t\tif (__hitBy[i].o == o) {{\n\
         \t\t\tif (__hitBy[i].id == id) {{ return; }}\n\
         \t\t\t__hitBy[i].id = id;\n\
         \t\t\to.addDamage(dmg);\n\
         \t\t\treturn;\n\
         \t\t}}\n\
         \t\ti = i + 1;\n\
         \t}}\n\
         \t__hitBy.push({{ o: o, id: id }});\n\
         \to.addDamage(dmg);\n\
         }}",
        origin.0, scale, origin.1, scale, scale, origin.0, scale, origin.1, scale, scale,
        boxes = named_boxes.iter()
            .map(|(n, r)| {
                // ...and the clip's own origin, the point its code moves it by
                let reg = named_regs.get(n).map(|(x, y)| format!(", rx: {x:.1}, ry: {y:.1}")).unwrap_or_default();
                format!("{{ n: \"{n}\", x: {:.1}, y: {:.1}, w: {:.1}, h: {:.1}{reg} }}", r.x, r.y, r.w, r.h)
            })
            .collect::<Vec<_>>().join(", "));
    // a speed the stage states or reads crosses the same way a fighter's does
    let speed_helpers = crate::api_mappings::speed_boundary_helpers_for(crate::api_mappings::SpeedUnits::Scaled);
    let coord_helpers = format!("{coord_helpers}{speed_helpers}{pan_helpers}");
    // The stage's timeline is its CLOCK as well as its art. Its pictures became objects that loop
    // on their own, so nothing DRAWN needs it running -- but the stage's own class counts on it to
    // know the time: clocktown turns its day over and starts its storm on a frame number. Paused,
    // that frame never changes, so the day never turns and the rain is never asked to start (or to
    // stop). So it stops only when nothing is reading it.
    // ...and STOPPING IT STOPS THE STAGE. `pause` is the whole entity's, not its timeline's: a
    // paused stage never runs its update, so it never spawns the elements it draws with, never
    // makes its hazards, and never answers anything. Five stages sat empty on that -- no error,
    // nothing in the match, and every check that reads the error channel called them clean.
    //
    // A timeline nobody reads costs nothing to leave running, which is what every stage that
    // works has always done. So it runs.
    let init = "\t// the timeline runs: the stage's clips play on it, its class may read the time \
                from it, and stopping it would stop the stage's own update with it".to_string();
    // the bodies the tree placed stand from the stage's first frame, ahead of the fighters
    let boot_handles: String = crate::re!(r"var (_hz\d+) = ").captures_iter(boot_spawns)
        .map(|c| format!("var {} = null;\n", &c[1])).collect();
    // A SURFACE THE STAGE REACHES BY NAME. The source hands its code the platform clip it asked
    // for; here the same surface is the entity's line segments, filed under that name, and the
    // handle turns each of them on and off together (galaxy tours keeps every section's ground
    // in place and makes one solid at a time).
    let seg_table: String = plat_segs.iter().map(|(n, labels)| {
        let ls: Vec<String> = labels.iter().map(|l| format!("\"{}\"", l.replace('"', ""))).collect();
        format!("{{ n: \"{n}\", s: [{}] }}", ls.join(", "))
    }).collect::<Vec<_>>().join(", ");
    // what the stage's code calls on the handles it takes by name: each is answered, the
    // structure stats for real and the rest (ignore lists, friction) as nothing, since the
    // engine's structures carry no such thing
    let handle_vars: std::collections::BTreeSet<String> = crate::re!(r"\b([A-Za-z_]\w*)\s*=\s*__objectNamed\(")
        .captures_iter(ported).map(|c| c[1].to_string()).collect();
    // ...each with the number of arguments the code hands it: the script language takes no
    // fewer and no more than a function names
    let mut handle_calls: std::collections::BTreeMap<String, usize> = Default::default();
    for v in &handle_vars {
        for c in crate::dyn_re(&format!(r"\b{}\.([A-Za-z_]\w*)\(([^()]*(?:\([^()]*\)[^()]*)*)\)", regex::escape(v))).captures_iter(ported) {
            let args = c[2].trim();
            let n = if args.is_empty() { 0 } else {
                let (mut depth, mut count) = (0i32, 1usize);
                for ch in args.chars() {
                    match ch { '(' | '[' | '{' => depth += 1, ')' | ']' | '}' => depth -= 1, ',' if depth == 0 => count += 1, _ => {} }
                }
                count
            };
            handle_calls.entry(c[1].to_string()).or_insert(n);
        }
    }
    for known in ["updateStructureStats", "setIgnoreObjectListInversed", "addIgnoreObject", "removeIgnoreObject", "clearIgnoreObjects",
                  "setVisible", "getViewRootContainer", "playFrameLabel"] {
        handle_calls.remove(known);
    }
    let stubs: String = handle_calls.iter()
        .map(|(m, n)| format!(", {m}: function({}) {{}}", (0..*n).map(|i| format!("a{i}")).collect::<Vec<_>>().join(", "))).collect();
    let boot_handles = format!(
        "{boot_handles}var __platSegs = [{seg_table}];\n\
         function __platStats(n, o) {{\n\
         \tvar i = 0;\n\
         \twhile (i < __platSegs.length) {{\n\
         \t\tif (__platSegs[i].n == n) {{\n\
         \t\t\tvar k = 0;\n\
         \t\t\twhile (k < __platSegs[i].s.length) {{\n\
         \t\t\t\tvar st = match.getStructureByName(__platSegs[i].s[k]);\n\
         \t\t\t\tif (st != null) {{ st.updateStructureStats(o); }}\n\
         \t\t\t\tk = k + 1;\n\
         \t\t\t}}\n\
         \t\t}}\n\
         \t\ti = i + 1;\n\
         \t}}\n\
         }}\n\
         function __platHandle(n) {{\n\
         \tvar i = 0;\n\
         \twhile (i < __platSegs.length) {{\n\
         \t\tif (__platSegs[i].n == n) {{\n\
         \t\t\tvar h = {{ n: n, inv: false, ign: []{stubs} }};\n\
         \t\t\th.updateStructureStats = function(o) {{ __platStats(n, o); }};\n\
         \t\t\t// who the surface ignores: a list of objects it does not collide with, or, inversed,\n\
         \t\t\t// the only objects it does (crystal smash's starting platform holds one player up)\n\
         \t\t\th.setIgnoreObjectListInversed = function(v) {{ h.inv = v; __platIgnores(h, null); }};\n\
         \t\t\th.addIgnoreObject = function(o) {{ h.ign.push(o); __platIgnores(h, null); }};\n\
         \t\t\th.removeIgnoreObject = function(o) {{ h.ign.remove(o); __platIgnores(h, o); }};\n\
         \t\t\th.clearIgnoreObjects = function() {{ var was = h.ign; h.ign = []; for (o in was) {{ __platIgnores(h, o); }} }};\n\
         \t\t\t// what the stage does to the surface's DRAWING goes to the clip group under the same\n\
         \t\t\t// name when there is one (hylian skies shows one terrain and hides the other)\n\
         \t\t\th.view = {{ visible: true }};\n\
             \t\t\t// ...and the asks made of its clip go to the surface's own drawing (smashville plays\n\
             \t\t\t// a label on its platform's clip as it flies off)\n\
             \t\t\th.view.setVisible = function(v) {{ h.setVisible(v); }};\n\
             \t\t\th.view.playFrameLabel = function(l) {{ h.playFrameLabel(l); }};\n\
             \t\t\th.view.gotoAndStop = function(l) {{ h.playFrameLabel(l); }};\n\
             \t\t\th.view.gotoAndPlay = function(l) {{ h.playFrameLabel(l); }};\n\
             \t\t\th.view.addChild = function(c) {{}}; h.view.removeChild = function(c) {{}}; h.view.setAlpha = function(a) {{}};\n\
         \t\t\t// where the surface is, as the code reads it (urban champion lists its loop platforms' x)\n\
         \t\t\tvar hb = __objectBoundary(n);\n\
         \t\t\t// what an object publishes, a surface answers too: its deck is its first structure,\n\
         \t\t\t// the face the engine names as a fighter's floor (kingdom1 weighs its see-saw by it)\n\
         \t\t\th.exports = {{ __deck: (__platSegs[i].s.length > 0 ? match.getStructureByName(__platSegs[i].s[0]) : null) }};\n\
         \t\t\th.x = (hb != null) ? hb.x : 0;\n\t\t\th.y = (hb != null) ? hb.y : 0;\n\
         \t\t\th.getX = function() {{ return h.x; }};\n\t\t\th.getY = function() {{ return h.y; }};\n\
         \t\t\th.setX = function(v) {{ h.x = v; }};\n\t\t\th.setY = function(v) {{ h.y = v; }};\n\
         \t\t\th.setVisible = function(v) {{ h.view.visible = v; var g = __groupNamed(n); if (g != null && g != h && g.parts != null) {{ g.setVisible(v); }} }};\n\
         \t\t\th.getViewRootContainer = function() {{ return h.view; }};\n\
         \t\t\th.playFrameLabel = function(l) {{ var g = __groupNamed(n); if (g != null && g != h && g.parts != null) {{ g.playFrameLabel(l); }} }};\n\
         \t\t\treturn h;\n\
         \t\t}}\n\
         \t\ti = i + 1;\n\
         \t}}\n\
         \treturn null;\n\
         }}\n\
         function __platIgnores(h, gone) {{\n\
         \tvar i = 0;\n\
         \twhile (i < __platSegs.length) {{\n\
         \t\tif (__platSegs[i].n == h.n) {{\n\
         \t\t\tvar k = 0;\n\
         \t\t\twhile (k < __platSegs[i].s.length) {{\n\
         \t\t\t\tvar st = match.getStructureByName(__platSegs[i].s[k]);\n\
         \t\t\t\tif (st != null) {{\n\
         \t\t\t\t\tif (gone != null) {{ st.removeFromBlacklist(gone); st.removeFromWhitelist(gone); }}\n\
         \t\t\t\t\tfor (o in h.ign) {{ if (h.inv) {{ st.addToWhitelist(o); }} else {{ st.addToBlacklist(o); }} }}\n\
         \t\t\t\t}}\n\
         \t\t\t\tk = k + 1;\n\
         \t\t\t}}\n\
         \t\t}}\n\
         \t\ti = i + 1;\n\
         \t}}\n\
         }}\n");
    let init = if boot_spawns.trim().is_empty() { init } else {
        format!("{init}\n\tvar owner = null;\n{}", boot_spawns.replace("\t\t\t", "\t").replace("\tvar _hz", "\t_hz"))
    };
    // hazards spawn DEFERRED in update() once the match is live (one-shot via a flag). owner is
    // NULL: a stage hazard belongs to no fighter, so it damages everyone (a null hitbox owner
    // passes the engine's team-hit validation), and createCustomGameObject accepts a null owner.
    // Weather spawns in the same deferred block as hazards (it needs a live match too) and then
    // runs every frame. Its state is one array declared alongside the hazard flag.
    // The class's OWN initialize runs once the objects it addresses exist -- not before them.
    // SSF2 runs it against a display tree that is already built; here the same moment is after the
    // converted clips have been created, which happens on the first live frame. Run at the top
    // instead, everything it does to its clips is done to nothing: clocktown hides its rain
    // splashes there, and they played from the first frame of the match.
    // The clips the stage class HIDES are the ones it counts as part of its weather -- it hides
    // them in the same breath as it builds the weather object. The source places them all through
    // the day (clocktown puts a splash down 265 times, only a dozen of them while it rains), so
    // placement alone does not say when they show: the rain does. Tie them to the same strength the
    // weather runs at, and they splash while it rains and not otherwise.
    let weather_clips: Vec<String> = {
        let re = crate::re!(r"(\w+)\.setVisible\(false\)");
        // only what the stage really hides, and only a clip this port actually has: a line the
        // translation struck out is not something the stage does, and a name read out of one is
        // not a handle -- gated on it, the stage reaches for a variable nothing declares and its
        // whole script stops at the first frame.
        let mut v: Vec<String> = ported.lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .flat_map(|l| re.captures_iter(l).map(|c| c[1].to_string()).collect::<Vec<_>>())
            .filter(|n| clip_handles.iter().any(|h| h == n))
            .collect();
        v.sort(); v.dedup(); v
    };
    let (src_init, src_update) = if ported.trim().is_empty() {
        (String::new(), String::new())
    } else {
        // the class builds what it holds before it is asked to do anything: butter building keeps
        // its whole layout table in its constructor, and without it the stage has no layouts.
        let built = if ported.contains("function __built()") { "\t\t\t__built();\n" } else { "" };
        // THE SOURCE'S UPDATE RUNS AT THE SOURCE'S RATE. Everything the class counts per update
        // (castle siege's section timers tick once a frame toward `30 * seconds`) was written for
        // thirty frames a second, and this engine calls update sixty times a second; run every
        // frame, the siege moved on in half the time. Every other frame here is one frame there.
        // ...and what the setup asked of the objects it made reaches them before their first
        // update: in the source those calls are made and answered in the same setup, and an ask
        // delivered a frame later left saffron city's platform walking to no target (its index
        // went to nothing on an empty list and stayed there)
        let deliver = if ported.contains("function __deliverAsks(") { "\t\t\t__deliverAsks();\n" } else { "" };
        (format!("{built}\t\t\t__source_initialize();\n{deliver}\t\t\t{{ var __k = 0; while (__k < __namedObjs.length) {{ var __o = __namedObjs[__k].o; if (__o != null && __o.exports != null) {{ __o.exports.__setup = true; }} __k = __k + 1; }} }}\n"),
         "\tif (__clockFrames % 2 == 0) { __source_update(); }\n".to_string())
    };
    let (weather_spawn, weather_frame) = weather;
    let (w_var, w_spawn, w_frame) = if weather_spawn.is_empty() {
        (String::new(), String::new(), String::new())
    } else {
        // one list and one strength PER FIELD: a stage that runs two can turn one off without
        // taking the other with it, which is what crateria does with its acid
        ((0..weather_n).map(|i| format!("var m_weather{i} = [];\nvar m_weatherAlpha{i} = 1.0;\n"))
             .collect::<String>(),
         weather_spawn.to_string(), weather_frame.to_string())
    };
    let (haz_var, haz_body) = if hazard_spawns.is_empty() && w_spawn.is_empty() {
        // no hazards: keep update() a clean empty body (byte-stable with hazardless stages).
        (String::new(), String::new())
    } else {
        ("var m_hazardsSpawned = false;\nvar m_hazardsOn = true;\nvar m_weatherSpawned = false;\n\
          // containers the stage's timeline puts up and takes away; what is inside each keeps\n\
          // looping on its own the whole time, the way a nested clip does\n\
          var m_shown = [];\n\
          var m_tint = [];\n                  // pieces that CROSS-FADE: the schedule of what each one's opacity is, when\n                  var m_alpha = [];\n\
          var m_anim = [];\n\
          // the source's coordinates, put into this stage's. FM = (source - origin) * scale, the\n\
          // same mapping the art came through, so a position the source states as a constant\n\
          // lands where the art it was measured against ended up.\n\
          COORD_HELPERS\n\
          // everything this stage makes, so it can be destroyed when the stage goes. a stage that\n\
          // spawns a couple of hundred objects and drops them on the floor leaks a set per match.\n\
          var m_owned = [];\n\
          // what this script has MADE, by the id it was made from, so the running stage can be\n\
          // asked what it built and answer for itself (`__verifyDump`). the verify gates read this\n\
          // against the plan the converter wrote, and a stage that was dropped, hung, or never\n\
          // reached its spawn block cannot give the right answer, which is the point.\n\
          var __made = [];\n\
          var __verifyFrames = 0;\n\
          var __verifyBuilt = false;\n\
          function __verifyDump() {\n\
          \tvar s = \"\";\n\
          \tvar i = 0;\n\
          \twhile (i < __made.length) {\n\
          \t\tvar m = __made[i];\n\
          \t\tvar o = m.o;\n\
          \t\tvar line = m.id + \"|\" + m.k;\n\
          \t\tif (o == null) {\n\
          \t\t\tline = line + \"|null\";\n\
          \t\t} else if (o.isDisposed()) {\n\
          \t\t\tline = line + \"|gone\";\n\
          \t\t} else {\n\
          \t\t\tline = line + \"|\" + o.getX() + \"|\" + o.getY();\n\
          \t\t\tif (m.k != \"s\") {\n\
          \t\t\t\tline = line + \"|\" + o.getCurrentFrame() + \"|\" + o.getTotalFrames();\n\
          \t\t\t}\n\
          \t\t}\n\
          \t\ts = s + line + \";\";\n\
          \t\ti = i + 1;\n\
          \t}\n\
          \treturn s;\n\
          }\n\
          // a piece drawn in front of the fighters that belongs to an object: the object moves and\n\
          // the piece goes with it, since the two are one thing in the source.\n\
          var __follow = [];\n\
          function __followUpdate() {\n\
          \tvar i = 0;\n\
          \twhile (i < __follow.length) {\n\
          \t\tvar f = __follow[i];\n\
          \t\tif (f.v != null && f.o != null) {\n\
          \t\t\tf.v.setX(f.o.getX() + ((f.dx != null) ? f.dx : 0));\n\
          \t\t\tf.v.setY(f.o.getY() + ((f.dy != null) ? f.dy : 0));\n\
          \t\t}\n\
          \t\ti = i + 1;\n\
          \t}\n\
          }\n\
          // a surface the plane's timeline moves by label rides its own picture: its structures\n\
          // sit where the picture's current frame has moved from the boot placement\n\
          var __platFollow = [];\n\
          function __platFollowUpdate() {\n\
          \tvar i = 0;\n\
          \twhile (i < __platFollow.length) {\n\
          \t\tvar e = __platFollow[i];\n\
          \t\tif (e.v != null) {\n\
          \t\t\tvar l = (e.v.exports != null && e.v.exports.__label != null) ? e.v.exports.__label : e.v.getAnimation();\n\
          \t\t\tvar d = null; var a = 0;\n\
          \t\t\twhile (a < e.at.length) { if (e.at[a].l == l) { d = e.at[a].d; } a = a + 1; }\n\
          \t\t\tif (d == null || d.length == 0) { i = i + 1; continue; }\n\
          \t\t\tvar j = Std.int((e.v.getCurrentFrame() - 1) / 2);\n\
          \t\t\tif (j < 0) { j = 0; }\n\
          \t\t\tif (j * 2 + 1 >= d.length) { j = Std.int(d.length / 2) - 1; }\n\
          \t\t\tvar dx = d[j * 2]; var dy = d[j * 2 + 1];\n\
          \t\t\tvar q = 0;\n\
          \t\t\twhile (q < __platSegs.length) {\n\
          \t\t\t\tif (__platSegs[q].n == e.n) {\n\
          \t\t\t\t\tvar s = 0;\n\
          \t\t\t\t\twhile (s < __platSegs[q].s.length) {\n\
          \t\t\t\t\t\tvar st = match.getStructureByName(__platSegs[q].s[s]);\n\
          \t\t\t\t\t\tif (st != null) {\n\
          \t\t\t\t\t\t\tif (e.bx.length <= s) { e.bx.push(st.getX()); e.by.push(st.getY()); }\n\
          \t\t\t\t\t\t\tst.setX(e.bx[s] + dx); st.setY(e.by[s] + dy);\n\
          \t\t\t\t\t\t}\n\
          \t\t\t\t\t\ts = s + 1;\n\
          \t\t\t\t\t}\n\
          \t\t\t\t}\n\
          \t\t\t\tq = q + 1;\n\
          \t\t\t}\n\
          \t\t}\n\
          \t\ti = i + 1;\n\
          \t}\n\
          }\n".replace("COORD_HELPERS", &coord_helpers),
         format!("\tif (!m_hazardsSpawned) {{\n\
                  \t\tvar chars = match.getCharacters();\n\
                  \t\tif (chars.length > 0) {{\n\
                  \t\t\tm_hazardsSpawned = true;\n\
                  \t\t\t// Both engines let a match turn hazards OFF, and a converted stage has to\n\
                  \t\t\t// mean it. WHAT goes quiet is the source stage's own decision, ported: in\n\
                  \t\t\t// SSF2 the engine does not apply the switch to a stage, the stage asks and\n\
                  \t\t\t// skips what it chooses to, so only what the original gated is gated here.\n\
                  \t\t\tvar cfg = match.getMatchSettingsConfig();\n\
                  \t\t\tm_hazardsOn = cfg == null || cfg.hazards;\n\
                  \t\t\tvar owner = null;\n\
{hazard_spawns}\
{src_init}\
                  \t\t\t__verifyBuilt = true;\n\
                  \t\t}}\n\
                  \t}}\n\
                  \t// the weather waits for a CAMERA, not just for the match: it is scattered over\n\
                  \t// the view and sized by it, and when the fighters first exist the camera is not\n\
                  \t// up yet -- spawning then falls back to the source area, sized for SSF2's screen\n\
                  \tif (!m_weatherSpawned) {{\n\
                  \t\tvar wc0 = match.getCamera();\n\
                  \t\tif (wc0 != null) {{\n\
                  \t\t\tm_weatherSpawned = true;\n\
{w_spawn}\
                  \t\t}}\n\
                  \t}}\n\tif (m_shown.length > 0) {{\n\
                  \t\tvar __f = __clock();\n\
                  \t\t// Walk each object's runs FORWARD, one step at a time, instead of scanning\n\
                  \t\t// all of them every frame. A stage can hold a couple of hundred objects with\n\
                  \t\t// a couple of hundred runs each, and re-reading every run each frame is tens\n\
                  \t\t// of thousands of comparisons per frame in a scripted loop -- which is felt\n\
                  \t\t// as the whole match slowing down. The frame only ever moves forward (or\n\
                  \t\t// wraps at the end of the day), so a cursor per object is enough.\n\
                  \t\tfor (e in m_shown) {{\n\
                  \t\t\tif (__f < e.at) {{ e.i = 0; }}\n\
                  \t\t\te.at = __f;\n\
                  \t\t\twhile (e.i < e.on.length && __f >= e.on[e.i][1]) {{ e.i = e.i + 1; }}\n\
                  \t\t\te.c.visible = e.i < e.on.length && __f >= e.on[e.i][0];\n\
                  \t\t}}\n\
                  \t}}\n\
                  \t{{\n\
                  \t\t// the camera-background containers ride the camera (see __panContainer)\n\
                  \t\t__panUpdate();\n\
                  \t\tvar __g = __clock();\n\
                  \t\t// same forward cursor the visibility schedule uses, and the shader is only\n\
                  \t\t// written when the key actually changes: a colour that holds for a hundred\n\
                  \t\t// frames costs one write, not a hundred.\n\
                  \t\t// the cross-fade the source authored: one step a frame, not a jump\n\
                  \t\tfor (e in m_alpha) {{\n\
                  \t\t\t// the element's OWN clock: its animation loops at its own length, so its opacity\n\
                  \t\t\t// has to wrap with it, or the two slide apart by a lap every time round\n\
                  \t\t\tvar __e = if (e.len > 0) __g % e.len else __g;\n\
                  \t\t\tif (__e < e.at) {{ e.i = 0; }}\n\
                  \t\t\te.at = __e;\n\
                  \t\t\twhile (e.i + 1 < e.k.length && __e >= e.k[e.i + 1][0]) {{ e.i = e.i + 1; }}\n\
                  \t\t\t// HOLD the key until the next one. Every frame of the source was read and a key\n\
                  \t\t\t// written wherever the opacity CHANGED, so a gap between keys is the source\n\
                  \t\t\t// holding still, not a slope to walk. Tweening across one invents a fade the\n\
                  \t\t\t// source never authored: a piece that goes dark by frame 228 and comes back at\n\
                  \t\t\t// 798 was ramping back up through all 570 frames between, so the day started\n\
                  \t\t\t// turning long before it turns. The fades ARE here -- as the run of keys every\n\
                  \t\t\t// other frame that the source drew them with.\n\
                  \t\t\te.c.alpha = e.k[e.i][1];\n\
                  \t\t}}\n\
                  \t\tfor (e in m_tint) {{\n\
                  \t\t\t// the element's OWN clock, same as opacity: a colour table only a few frames\n\
                  \t\t\t// long was walked by the stage's day-long counter, so after its last key the\n\
                  \t\t\t// cursor pinned there and the element kept that colour for the rest of the day.\n\
                  \t\t\tvar __e = if (e.len > 0) __g % e.len else __g;\n\
                  \t\t\tif (__e < e.at) {{ e.i = 0; }}\n\
                  \t\t\te.at = __e;\n\
                  \t\t\twhile (e.i + 1 < e.k.length && __e >= e.k[e.i + 1][0]) {{ e.i = e.i + 1; }}\n\
                  \t\t\t// HOLD, same as opacity and for the same reason: a key was written wherever the\n\
                  \t\t\t// colour CHANGED, so the gaps are the source sitting still and the slides it\n\
                  \t\t\t// does author are already here key by key.\n\
                  \t\t\tvar __t = e.k[e.i];\n\
                  \t\t\te.s.redMultiplier = __t[1];\n\
                  \t\t\te.s.greenMultiplier = __t[2];\n\
                  \t\t\te.s.blueMultiplier = __t[3];\n\
                  \t\t\te.s.redOffset = __t[4];\n\
                  \t\t\te.s.greenOffset = __t[5];\n\
                  \t\t\te.s.blueOffset = __t[6];\n\
                  \t\t}}\n\
                  \t}}\n\
                  \tif (m_anim.length > 0) {{\n\
                  \t\tvar __h = __clock();\n\
                  \t\t// An element that loops keeps only its own loop, one animation per stretch\n\
                  \t\t// of the day that repeats on it. This puts the element on the stretch the\n\
                  \t\t// day is in -- the same forward cursor, and the sprite is only told when\n\
                  \t\t// the stretch actually changes.\n\
                  \t\tfor (e in m_anim) {{\n\
                  \t\t\tif (__h < e.at) {{ e.i = 0; }}\n\
                  \t\t\te.at = __h;\n\
                  \t\t\twhile (e.i + 1 < e.k.length && __h >= e.k[e.i + 1][0]) {{ e.i = e.i + 1; }}\n\
                  \t\t\tif (e.i != e.j) {{\n\
                  \t\t\t\te.j = e.i;\n\
                  \t\t\t\te.v.playAnimation(e.k[e.i][1]);\n\
                  \t\t\t}}\n\
                  \t\t\t// ON THE STAGE'S CLOCK, not its own. An element used to be written out\n\
                  \t\t\t// against the whole day, so every piece of one thing -- the three sprites\n\
                  \t\t\t// that make up a torch flame -- advanced together. Keeping only the loop\n\
                  \t\t\t// and letting the engine run it free lets those pieces drift apart, and a\n\
                  \t\t\t// flame whose halves are on different frames shows a seam down the middle.\n\
                  \t\t\t// Holding the frame to the day puts them back in step, and it is the same\n\
                  \t\t\t// frame the long timeline would have been showing.\n\
                  \t\t\tvar __p = e.k[e.i][2];\n\
                  \t\t\tif (__p > 0) {{ e.v.playFrame(((__h - e.k[e.i][0]) % __p) + 1); }}\n\
                  \t\t}}\n\
                  \t}}\n{w_frame}"))
    };
    // One var per clip, named the way the stage's display tree names it. The stage class reaches
    // its objects through the tree (`getBackground().clocktown_rainsplash_bg`), and after the port
    // there is no tree -- so the name it used is bound to the object that took that clip's place.
    // ...and the stage's own script may reach for the helpers this block defines with no named
    // clip at all (a target test asks for its planes as a whole, `[__plane(), __plane()]`), so
    // the block is emitted whenever that script uses one of them
    let uses_clip_helpers = ["__plane(", "__groupNamed(", "__currentFrame(", "__currentLabel(", "__totalFrames("]
        .iter().any(|h| ported.contains(h));
    let clip_vars = if clip_handles.is_empty() && !uses_clip_helpers { String::new() } else {
        format!(
            "// The stage's own clips, under the names its class knows them by. A named clip is\n\
             // ONE thing to that class and several independently-looping objects underneath, which\n\
             // is what it is in the source too -- so the name is bound to the group, and the group\n\
             // passes on what is asked of it.\n\
             function __has(list, name) {{\n\
             \tfor (i in 0...list.length) {{ if (list[i] == name) {{ return true; }} }}\n\
             \treturn false;\n\
             }}\n\
             // a frame number, as the source plays one (`gotoAndPlay(3)`): every piece finds its own\n\
             function __isNum(v) {{\n\
             \tvar s = Std.string(v);\n\
             \tif (s.length == 0) {{ return false; }}\n\
             \tfor (i in 0...s.length) {{ var k = s.charCodeAt(i); if ((k < 48 || k > 57) && !(i == 0 && k == 45)) {{ return false; }} }}\n\
             \treturn true;\n\
             }}\n\
             // a jump of the whole stage to a section: every clip group that carries the label\n\
             // takes it, and so does the stage's own painted art when it carries frame labels\n\
             function __phase(l) {{\n\
             \tvar s = \"\";\n\
             \tfor (i in 0...l.length) {{ var k = l.charCodeAt(i); if ((k >= 48 && k <= 57) || (k >= 65 && k <= 90) || (k >= 97 && k <= 122)) {{ s += l.charAt(i); }} }}\n\
             \treturn s;\n\
             }}\n\
             // ...and where the timeline puts each placed object at that label: the object follows\n\
             var __placedAt = [{placed_at}];\n\
             function __planesPlayFrameLabel(l) {{\n\
             \tvar n = __phase(Std.string(l));\n\
             {plane_jumps}\
             \tvar s = Std.string(l);\n\
             \tfor (i in 0...__placedAt.length) {{\n\
             \t\tif (__placedAt[i].l == s) {{\n\
             \t\t\tvar o = __objectNamed(__placedAt[i].n);\n\
             \t\t\tif (o != null) {{ o.setX(__placedAt[i].x); o.setY(__placedAt[i].y); }}\n\
             \t\t}}\n\
             \t}}\n\
             }}\n\
             // A MATCH GLOBAL, kept by the stage that sets it: the engine sets none here, so one\n\
             // nothing set reads as unset, which is what a versus match had in the source too\n\
             var __globalNames = [];\n\
             var __globalValues = [];\n\
             function __globalVar(n) {{\n\
             \tfor (i in 0...__globalNames.length) {{ if (__globalNames[i] == n) {{ return __globalValues[i]; }} }}\n\
             \treturn null;\n\
             }}\n\
             function __setGlobalVar(n, v) {{\n\
             \tfor (i in 0...__globalNames.length) {{ if (__globalNames[i] == n) {{ __globalValues[i] = v; return; }} }}\n\
             \t__globalNames.push(n);\n\t__globalValues.push(v);\n\
             }}\n\
             // THE CLOCK OF A CLIP, as the source's code reads it. the stage's own plane is the\n\
             // stage's timeline (its labels are below); a clip that became a group answers with\n\
             // the clock its pieces publish; an object answers with its own\n\
             var __labelMarks = [{label_marks}];\n\
             function __srcFrameOf(o) {{ if (o == null || o.getCurrentFrame == null) {{ return 0; }} return Std.int((o.getCurrentFrame() - 1) / 2) + 1; }}\n\
             // who a fighter is, by the id its stats state (published by every script this converter makes)\n\
             function __statsNameOf(o) {{ if (o != null && o.exports != null && o.exports.__statsName != null) {{ return o.exports.__statsName; }} return \"\"; }}\n\
             function __rmTimer(x) {{ return self.removeTimer(x); }}\n\
             function __firstPart(o) {{\n\
             \tfor (i in 0...o.parts.length) {{ if (o.parts[i] != null && o.parts[i].exports != null) {{ return o.parts[i]; }} }}\n\
             \treturn null;\n\
             }}\n\
             function __currentFrame(o) {{\n\
             \tif (o == null) {{ return 0; }}\n\
             \tif (o == self) {{ return __srcFrameOf(self); }}\n\
             \tif (o.vs != null) {{ return o.getCurrentFrame(); }}\n\
             \tif (o.parts != null) {{ var p = __firstPart(o); return (p != null && p.exports.__frame != null) ? p.exports.__frame : 0; }}\n\
             \tif (o.exports != null && o.exports.__frame != null) {{ return o.exports.__frame; }}\n\
             \treturn __srcFrameOf(o);\n\
             }}\n\
             function __currentLabel(o) {{\n\
             \tif (o == null) {{ return \"\"; }}\n\
             \tif (o == self) {{\n\
             \t\tvar f = __srcFrameOf(self);\n\
             \t\tvar best = \"\";\n\
             \t\tfor (i in 0...__labelMarks.length) {{ if (__labelMarks[i].f <= f) {{ best = __labelMarks[i].n; }} }}\n\
             \t\treturn best;\n\
             \t}}\n\
             \tif (o.parts != null) {{ var p = __firstPart(o); return (p != null && p.exports.__label != null) ? p.exports.__label : \"\"; }}\n\
             \tif (o.exports != null && o.exports.__label != null) {{ return o.exports.__label; }}\n\
             \treturn o.getAnimation();\n\
             }}\n\
             function __totalFrames(o) {{\n\
             \tif (o == null) {{ return 0; }}\n\
             \tif (o == self) {{ return Std.int(self.getTotalFrames() / 2); }}\n\
             \tif (o.parts != null) {{ var p = __firstPart(o); return (p != null && p.exports.__total != null) ? p.exports.__total : 0; }}\n\
             \tif (o.exports != null && o.exports.__total != null) {{ return o.exports.__total; }}\n\
             \treturn Std.int(o.getTotalFrames() / 2);\n\
             }}\n\
             function __groupNamed(n) {{\n\
             {group_named}\
             \treturn null;\n\
             }}\n\
             // a plane as a whole: its children by name, each the object this port keeps for it\n\
             function __plane() {{\n\
             \tvar p = {{ {plane_children} }};\n\
             \t// ...and the asks made of the plane itself: a label played on it is the stage's own label\n\
             \tp.playFrameLabel = function(l) {{ __planesPlayFrameLabel(l); }};\n\
             \tp.gotoAndStop = function(l) {{ __planesPlayFrameLabel(l); }};\n\
             \tp.gotoAndPlay = function(l) {{ __planesPlayFrameLabel(l); }};\n\
             \tp.stop = function() {{}};\n\
             \tp.play = function() {{}};\n\
             \tp.visible = true;\n\
             \tp.setVisible = function(v) {{ p.visible = v; }};\n\
             \tp.setAlpha = function(v) {{}};\n\
             \t// its clock is the stage's own timeline\n\
             \tp.getAnimation = function() {{ return __currentLabel(self); }};\n\
             \tp.getCurrentFrame = function() {{ return __currentFrame(self); }};\n\
             \tp.getTotalFrames = function() {{ return __totalFrames(self); }};\n\
             \tp.__isPlane = true;\n\
             \treturn p;\n\
             }}\n\
             function __clipGroup(parts, labels, name, ws, plane) {{\n\
             \tvar g = {{}};\n\
             \tg.parts = parts;\n\
             \tg.visible = true;\n\
             \t// WHERE THE CLIP IS, as its class reads and writes it (`shadow.x = player.getX()`):\n\
             \t// the clip's own place is where the tree put it, and a move is every piece moving\n\
             \t// by the same amount\n\
             \tg.name = name;\n\
             \tvar b0 = (name == null) ? null : __objectBoundary(name);\n\
             \t// ...its own origin where the tree states one (the source moves the clip by it).\n\
             \t// the source writes the clip's place in ITS PLANE's space, and a script's number\n\
             \t// comes in converted as a terrain-local one (`__sx`), so the group keeps its place\n\
             \t// shifted by the two origins' distance: saffron city's platform class puts its art\n\
             \t// at its body less 92 in the background plane, whose origin is not the terrain's\n\
             \tvar pb = (plane == null) ? null : __objectBoundary(plane);\n\
             \tg.ox = (pb != null && pb.rx != null) ? pb.rx - __sx(0) : 0;\n\
             \tg.oy = (pb != null && pb.ry != null) ? pb.ry - __sy(0) : 0;\n\
             \tg.x = ((b0 != null) ? ((b0.rx != null) ? b0.rx : b0.x) : 0) - g.ox;\n\
             \tg.y = ((b0 != null) ? ((b0.ry != null) ? b0.ry : b0.y) : 0) - g.oy;\n\
             \tg.getX = function() {{ return g.x; }};\n\
             \tg.getY = function() {{ return g.y; }};\n\
             \t// its clock is the clock its pieces publish (rainbow route asks its mirror what it plays)\n\
             \tg.getAnimation = function() {{ return __currentLabel(g); }};\n\
             \tg.getCurrentFrame = function() {{ return __currentFrame(g); }};\n\
             \tg.getTotalFrames = function() {{ return __totalFrames(g); }};\n\
             \tg.playFrame = function(f) {{ g.playFrameLabel(f); }};\n\
             \t// a clip the stage stops holds every piece where it is until played again (a target\n\
             \t// test holds its targets and counters until the test starts)\n\
             \tg.stop = function() {{ for (i in 0...parts.length) {{ if (parts[i] != null && parts[i].pause != null) {{ parts[i].pause(); }} }} }};\n\
             \tg.play = function() {{ for (i in 0...parts.length) {{ if (parts[i] != null && parts[i].resume != null) {{ parts[i].resume(); }} }} }};\n\
             \tg.setX = function(v) {{\n\
             \t\tvar dx = v - g.x;\n\
             \t\tfor (i in 0...parts.length) {{ if (parts[i] != null) {{ parts[i].setX(parts[i].getX() + dx); }} }}\n\
             \t\tg.x = v;\n\
             \t}};\n\
             \tg.setY = function(v) {{\n\
             \t\tvar dy = v - g.y;\n\
             \t\tfor (i in 0...parts.length) {{ if (parts[i] != null) {{ parts[i].setY(parts[i].getY() + dy); }} }}\n\
             \t\tg.y = v;\n\
             \t}};\n\
             \t// a flip turns every piece about the clip's own origin, as the source's does: a piece's\n\
             \t// picture hangs off its own corner, so the corner crosses to the other side of the origin\n\
             \tg.scaleX = 1.0; g.scaleY = 1.0;\n\
             \tg.plane = plane;\n\
             \tg.getScaleX = function() {{ return g.scaleX; }};\n\
             \tg.getScaleY = function() {{ return g.scaleY; }};\n\
             \tg.setScaleX = function(s) {{\n\
             \t\tif ((s < 0) != (g.scaleX < 0)) {{\n\
             \t\t\tfor (i in 0...parts.length) {{ if (parts[i] != null) {{\n\
             \t\t\t\tvar w = (ws != null && i < ws.length) ? ws[i] : 0;\n\
             \t\t\t\tparts[i].setX(2 * g.x - parts[i].getX() - w);\n\
             \t\t\t}} }}\n\
             \t\t}}\n\
             \t\tg.scaleX = s;\n\
             \t\tfor (i in 0...parts.length) {{ if (parts[i] != null) {{ parts[i].setScaleX(s); }} }}\n\
             \t}};\n\
             \tg.setScaleY = function(s) {{ g.scaleY = s; for (i in 0...parts.length) {{ if (parts[i] != null) {{ parts[i].setScaleY(s); }} }} }};\n\
             \t// what each piece answers to by name, worked out while converting
             \tg.labels = labels;\n\
             \t// the OBJECT is shown or hidden, not the container it hangs in: the container's own
             \t// flag is written over as the object draws its next frame.
             \tg.setVisible = function(v) {{\n\
             \t\tg.visible = v;\n\
             \t\tfor (i in 0...parts.length) {{\n\
             \t\t\tif (parts[i] != null) {{ parts[i].setVisible(v); }}\n\
             \t\t}}\n\
             \t}};\n\
             \tg.setAlpha = function(a) {{\n\
             \t\tfor (i in 0...parts.length) {{\n\
             \t\t\tif (parts[i] != null) {{ parts[i].setAlpha(a); }}\n\
             \t\t}}\n\
             \t}};\n\
             \t// the clip's own timeline, asked for by label: each piece under the name carries\n\
             \t// that label as an animation of its own. the ask is left with the piece, whose own\n\
             \t// script plays it (the script keeps the clip's place and holds, and a change made\n\
             \t// behind its back is one it would undo on its next frame).\n\
             \tg.playFrameLabel = function(l) {{\n\
             \t\tg.visible = true;\n\
             \t\tfor (i in 0...parts.length) {{\n\
             \t\t\t// only the pieces that carry the label under that name: a label written on\n\
             \t\t\t// the clip itself is on none of them.\n\
             \t\t\tif (parts[i] != null && g.labels[i] != null && (__has(g.labels[i], l) || __isNum(l))) {{\n\
             \t\t\t\t// a piece with a script of its own publishes its frame, and takes the ask up\n\
             \t\t\t\tif (parts[i].exports != null && parts[i].exports.__frame != null) {{ parts[i].exports.__go = l; }}\n\
             \t\t\t\t// one without holds the frame it is sent to (the source's gotoAndStop), at\n\
             \t\t\t\t// this engine's two frames per source frame\n\
             \t\t\t\telse if (__isNum(l) && parts[i].playFrame != null) {{ var fno = Std.int(l); if (fno < 1) {{ fno = 1; }} parts[i].playFrame(fno * 2 - 1); parts[i].pause(); }}\n\
             \t\t\t}}\n\
             \t\t}}\n\
             \t}};\n\
             \treturn g;\n\
             }}\n\
             // a child of a clip the port has no piece for: the group's own member when it has one,\n\
             // else a stand-in that takes what the class writes (a text field's text)\n\
             var __kids = [];\n\
             function __childOf(g, n) {{\n\
             \tif (g == null) {{ return null; }}\n\
             \t// the group's own members are never asked for here (the rewrite keeps them as they are)\n\
             \tfor (i in 0...__kids.length) {{ if (__kids[i].g == g && __kids[i].n == n) {{ return __kids[i].k; }} }}\n\
             \tvar k = {{ text: \"\", x: 0.0, y: 0.0, visible: true, alpha: 1.0, scaleX: 1.0, scaleY: 1.0, width: 0.0, height: 0.0 }};\n\
             \tk.setVisible = function(b) {{ k.visible = b; }}; k.setAlpha = function(a) {{ k.alpha = a; }};\n\
             \tk.getX = function() {{ return k.x; }}; k.getY = function() {{ return k.y; }}; k.setX = function(v) {{ k.x = v; }}; k.setY = function(v) {{ k.y = v; }};\n\
             \tk.playFrameLabel = function(l) {{}}; k.playFrame = function(f) {{}}; k.gotoAndStop = function(l) {{}}; k.gotoAndPlay = function(l) {{}}; k.stop = function() {{}}; k.play = function() {{}};\n\
             \t// a stand-in plays nothing: its clock answers as a clip that has not started\n\
             \tk.getAnimation = function() {{ return \"\"; }}; k.getCurrentFrame = function() {{ return 0; }}; k.getTotalFrames = function() {{ return 0; }};\n\
             \t__kids.push({{ g: g, n: n, k: k }});\n\
             \treturn k;\n\
             }}\n{}\n",
            clip_handles.iter().map(|h| format!("var {h} = null;")).collect::<Vec<_>>().join("\n"))
    };
    // SSF2's weather object, in the terms this package ports weather into. The stage class owns
    // the weather's LIFE -- it fades it up when the storm rolls in and back down when it passes --
    // so the object it drives has to be the field that is actually on screen. Bound to nothing, a
    // stage that only rains for one stretch rains for the whole match.
    // the OTHER weathers a stage keeps: read, named, and inert, because this port built one field
    // and doing their fading to it would do it to the one that is falling.
    let weather_other = "function __weatherObjectOther() {\n\tvar w = {};\n\tw.update = function() {};\n\
                         \tw.setAlpha = function(a) {};\n\tw.activate = function() {};\n\
                         \tw.deactivate = function() {};\n\tw.getContainer = function() { return null; };\n\
                         \treturn w;\n}\n";
    // ONE OBJECT PER FIELD the port built. The stage class owns each weather's life -- it fades
    // one up when the storm rolls in and holds another at nothing until its own event -- so each
    // has to reach its own particles. Bound to a single field, turning one off took the other with
    // it, and crateria could not fall rain and acid at once.
    let weather_obj: String = (0..weather_n).map(|i| format!(
        "function __weatherObject{i}() {{\n\
         \tvar w = {{}};\n\
         \t// the particles move themselves every frame; the class only decides how visible they are\n\
         \tw.update = function() {{}};\n\
         \t// REMEMBERED, not just applied: the weather spawns once a camera exists, which is after\n\
         \t// the class has already said how visible it wants it. Setting only what is on screen\n\
         \t// now leaves every particle born later at full strength, so a stage told not to rain\n\
         \t// yet rains from the moment its field appears.\n\
         \tw.setAlpha = function(a) {{ m_weatherAlpha{i} = a; }};\n\
         \tw.activate = function() {{ m_weatherAlpha{i} = 1.0; }};\n\
         \tw.deactivate = function() {{ m_weatherAlpha{i} = 0.0; }};\n\
         \tw.getContainer = function() {{ return null; }};\n\
         \treturn w;\n\
         }}\n")).collect();
    let haz_var = format!("{clip_vars}{w_var}{weather_obj}{weather_other}{haz_var}");
    // The stage's OWN code runs after the setup, every frame, exactly as SSF2 runs it. Without
    // this the stage is furniture: everything it decides -- when a thwomp drops, when the clock
    // turns over, when it rains -- lives in that update and simply never happened.
    // after the class's own update, so it sees whatever the class just decided
    // ...and only when this stage HAS weather: the gate reads the strength the weather runs at, and
    // a stage with none never declares one.
    let weather_gate = if weather_clips.is_empty() || w_var.is_empty() { String::new() } else {
        let each: Vec<String> = weather_clips.iter().map(|c| format!(
            "\tif ({c} != null) {{ {c}.setVisible(m_weatherAlpha0 > 0); }}\n")).collect();
        each.join("")
    };
    let src_update = format!("{src_update}{weather_gate}");
    let update_fn = if haz_body.is_empty() && src_update.is_empty() {
        "function update() {\n\t__verifyFrames = __verifyFrames + 1;\n\t__clockFrames = __clockFrames + 1;\n}\n".to_string()
    } else {
        // the match is live once the stage's first step (its own setup runs inside it) is done
        format!("function update() {{\n\t__verifyFrames = __verifyFrames + 1;\n\t__clockFrames = __clockFrames + 1;\n{haz_body}{src_update}\t__live = true;\n}}\n")
    };
    dedupe_top_level_vars(&format!(
        // the helpers and the clip/weather handles come FIRST: the stage class's own fields are
        // initialised from them as the module is read, and a name used before it is defined stops
        // the whole script loading.
        "// API Script for {id} (converted from SSF2)\n\n\
{haz_var}\
{boot_handles}\
{ported}\
function initialize() {{\n\
{init}\n\
}}\n\
{update_fn}\
function onTeardown() {{\n\
\tfor (o in m_owned) {{ if (o != null) {{ o.destroy(); }} }}\n\
\tm_owned = [];\n\
}}\n\
function onKill() {{}}\n\
function onStale() {{}}\n\
function afterPushState() {{}}\n\
function afterPopState() {{}}\n\
function afterFlushStates() {{}}\n"
    ))
}

#[cfg(test)]
mod lowering_tests {
    use super::*;

    #[test]
    fn a_string_index_on_another_object_is_a_lookup_and_a_number_index_is_not() {
        let src = "\t\t_v4 = it.screen[(\"p\" + (i + 1)) + \"TextBox\"];\n\t\t_v2 = it.playerList[i];\n\t\tit.screen[\"name\" + k] = _v3;\n";
        let out = string_indexed_members(src);
        assert!(out.contains("_v4 = __fieldOf(it.screen, (\"p\" + (i + 1)) + \"TextBox\");"), "{out}");
        assert!(out.contains("_v2 = it.playerList[i];"), "{out}");
        assert!(out.contains("__setFieldOf(it.screen, \"name\" + k, _v3);"), "{out}");
    }

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
            ("thwomp_multi", thwomp_multi_script_hx(&cols, -67.0, 13.0, 19.5, 3.9, 2.6, 1200.0, 120.0, 180.0, "", true, "entrance", "idle", "fall")),
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
        let spawn = "\t\t\tStageBoot.spawn(owner, chars, m_hazardsOn);\n";
        let s = script_hx("demo", false, spawn, "", &[], ("", ""), 0, "", &[], (0.0, 0.0), 1.0, (0.0, 0.0), &Default::default(), &Default::default(), &[], "");
        assert!(s.contains("var owner = null;"), "hazard owner should be null: {s}");
        assert!(s.contains("StageBoot.spawn(owner"), "no spawn call: {s}");
        let boot = crate::emit::stage_tables::boot_runtime().expect("the runtime's stage boot");
        assert!(boot.contains("createCustomGameObject(self.getResource().getContent(o.id), owner)"),
                "the boot makes an object with the owner it is handed: {boot}");
    }

    /// The boot tables as a literal: every section present, no row with a trailing comma, and the
    /// numbers at the precision the boot has always had them.
    #[test]
    fn boot_tables_literal() {
        let mut t = StageTables { objects_declared: 2, ..Default::default() };
        t.structures.push(BootStructure { id: "demoplatform0".into(), name: Some("plat".into()) });
        t.objects.push(BootObject { index: 1, id: "demohazard1".into(), x: 12.345, y: -3.0, front: false, fg: None,
            name: None, gated: true, placed: false, deck: None });
        t.elements.push(BootElement { index: 0, id: "demo_el".into(), segmented: false, x: 728.0, y: -60.19000000000002,
            at: BootAt::Pan(0.25, 1.0), shown: vec![(0, 10)], tint: None, alpha: Some(0.5), hidden: true, ride: None, follow: None,
            path: Vec::new(), labels: Vec::new(), sections: None, carry: None, chain: None });
        let lit = t.literal();
        assert!(lit.starts_with("var DATA = { stage: { boot: {"), "{lit}");
        assert!(lit.contains("x: 12.3, y: -3.0"), "object places at one decimal: {lit}");
        assert!(lit.contains("x: 728, y: -60.19000000000002"), "an element's place is written as it always was: {lit}");
        assert!(lit.contains("at: { pan: true, px: 0.2500, py: 1.0000 }"), "{lit}");
        assert!(lit.contains("alpha: 0.500"), "{lit}");
        assert!(!lit.contains(",\n\t]") && !lit.contains(", ]"), "no trailing comma: {lit}");
        assert!(t.spawns_at_start() && !t.has_placed() && !t.is_empty());
    }
}

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

/// A MEMBER REACHED BY A STRING INDEX ON SOMETHING ELSE (`screen["p" + i + "TextBox"]`): the
/// source's objects are dynamic, this engine's are not, and an index read as a list stopped the
/// script. The read goes through a lookup the object may answer (`__get`), else nothing, which
/// the source's own truth tests already guard; a write goes to `__set` when the object has it.
const FIELD_OF_HELPERS: &str = "// a member reached by a computed name on an object the port cannot read that way (an index on\n\
// an object throws here, and so does a missing dotted field): nothing, which the truth test the\n\
// source makes on the result guards; a table whose keys the port read is answered by its own lookup\n\
function __fieldOf(o, n) {\n\treturn null;\n}\n\
function __setFieldOf(o, n, v) {\n}\n";

fn string_indexed_members(raw: &str) -> String {
    let set_re = crate::re!(r#"(?m)^(\s*)([A-Za-z_][\w.]*)\[((?:[^\[\]]*"[^\[\]]*))\]\s*=\s*([^=][^;]*);"#);
    let r = set_re.replace_all(raw, "${1}__setFieldOf($2, $3, $4);").into_owned();
    let get_re = crate::re!(r#"([A-Za-z_][\w.]*)\[((?:[^\[\]]*"[^\[\]]*))\]"#);
    get_re.replace_all(&r, "__fieldOf($1, $2)").into_owned()
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

/// The stage's side of an object's asks: every update, what any object has posted on its
/// exports is run here, on the stage's own methods and named clips.
fn stage_ask_block(model: &StageModel, body: &str) -> String {
    let call = crate::re!(r"SSF2API\.getStage\(\)\.(\w+)\((.*)\);");
    let mut arms: std::collections::BTreeMap<String, usize> = Default::default();
    for hz in &model.hazards {
        for a in &hz.anims {
            for (_, _, code) in &a.clip_scripts {
                for c in call.captures_iter(code) {
                    if !body.contains(&format!("function {}(", &c[1])) { continue; }
                    let args = c[2].trim();
                    let n = if args.is_empty() { 0 } else { args.matches(',').count() + 1 };
                    arms.insert(c[1].to_string(), n);
                }
            }
        }
    }
    let any_scripts = model.hazards.iter().any(|hz| hz.anims.iter().any(|a| !a.clip_scripts.is_empty()));
    if !any_scripts { return String::new() }
    let mut out = String::from("// what an object's own timeline asked of this stage, run here on the stage's own methods and clips\n\
function __playNamed(n, l) {\n\
\tvar g = __groupNamed(n);\n\
\tif (g != null && g.playFrameLabel != null) { g.playFrameLabel(l); return; }\n\
\tvar o = __objectNamed(n);\n\
\tif (o != null && o.exports != null) { __ask(o, \"playFrameLabel\", [l]); }\n\
}\n\
function __runStageAsk(m, a) {\n\
\tif (m == \"__setVisibleNamed\") { __setVisibleNamed(a[0], a[1]); return; }\n\
\tif (m == \"__playNamed\") { __playNamed(a[0], a[1]); return; }\n");
    for (m, n) in &arms {
        let args = (0..*n).map(|i| format!("a[{i}]")).collect::<Vec<_>>().join(", ");
        out.push_str(&format!("\tif (m == \"{m}\") {{ {m}({args}); return; }}\n"));
    }
    out.push_str("}\n\
function __drainStageAsks() {\n\
\tif (self.exports.__stageAsks != null && self.exports.__stageAsks.length > 0) {\n\
\t\tvar mine = self.exports.__stageAsks; self.exports.__stageAsks = [];\n\
\t\tvar k = 0;\n\
\t\twhile (k < mine.length) { __runStageAsk(mine[k].m, mine[k].a); k = k + 1; }\n\
\t}\n\
\tvar os = match.getCustomGameObjects();\n\
\tvar i = 0;\n\
\twhile (i < os.length) {\n\
\t\tvar o = os[i];\n\
\t\tif (o != null && o.exports != null && o.exports.__stageAsk != null && o.exports.__stageAsk.length > 0) {\n\
\t\t\tvar q = o.exports.__stageAsk; o.exports.__stageAsk = [];\n\
\t\t\tvar j = 0;\n\
\t\t\twhile (j < q.length) { __runStageAsk(q[j].m, q[j].a); j = j + 1; }\n\
\t\t}\n\
\t\ti = i + 1;\n\
\t}\n\
}\n");
    if !body.contains("function __setVisibleNamed(") {
        out.push_str(&format!("function __setVisibleNamed(n, v) {{\n\
\tvar g = __groupNamed(n);\n\
\tif (g != null && g.setVisible != null) {{ g.setVisible(v); return; }}\n\
{}}}\n", if body.contains("function __objectVisible(") { "\t__objectVisible(n, v);\n" } else { "" }));
    }
    out
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

/// A value the object reads OFF ITS STAGE is not there the instant the object is.
///
/// Fraymakers hands the object its stage a frame after making it, so a class that asks the stage
/// something in its own `initialize` -- central highway's pillars work out where to pull the camera
/// to from the stage's bounds -- asks nothing at all, and the read stops the script. The ask is
/// spelled out instead: nothing yet reads as zero, and the class carries on.
fn guard_stage_reads(script: &str) -> String {
    let re = crate::re!(r"self\.exports\.__stage\.exports\.(__v_\w+)");
    re.replace_all(script,
        "(self.exports.__stage == null ? 0 : self.exports.__stage.exports.$1)").into_owned()
}
