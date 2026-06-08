//! Stage-porting pipeline tests: parse an SSF2 stage and emit a Fraymakers stage
//! package, asserting the geometry and the entity graph. Corpus-gated (skips
//! cleanly without the developer's `.ssf` files).

mod common;

use ssf2_converter::{emit_stage, parse_stage, parse_stage_opts};

/// Every stage in the corpus must parse (covers the terrain-naming variety across
/// SSF2 stages, not just battlefield). Corpus-gated.
#[test]
fn all_corpus_stages_parse() {
    let dir = common::ssfs_dir().join("stages");
    if !common::present(&dir) {
        return;
    }
    let mut total = 0;
    let mut failed = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) != Some("ssf") {
            continue;
        }
        total += 1;
        if let Err(e) = parse_stage_opts(&p, false) {
            failed.push(format!("{}: {e}", p.file_name().unwrap().to_string_lossy()));
        }
    }
    assert!(failed.is_empty(), "{}/{} stages failed to parse:\n{}", failed.len(), total, failed.join("\n"));
}

/// Moving platforms: SSF2 stages with moving platforms (the `moving`-named containers)
/// are detected and flagged. The motion isn't ported (bespoke per stage), but the platform
/// is kept as static collision and a warning surfaces it. Battlefield (no moving platforms)
/// must NOT be flagged. Corpus-gated.
#[test]
fn moving_platforms_detected_and_flagged() {
    let dir = common::ssfs_dir().join("stages");
    if !common::present(&dir) {
        return;
    }
    // a stage with moving platforms (the `moving` signal lives on the parent container and
    // is propagated to the collision child).
    let tos = dir.join("towerofsalvation.ssf");
    if common::present(&tos) {
        let m = parse_stage(&tos).expect("parse towerofsalvation");
        let moving = m.platforms.iter().filter(|p| p.moving).count();
        assert!(moving > 0, "towerofsalvation has moving platforms");
        assert!(m.warnings.iter().any(|w| w.contains("moving platform")),
            "a moving-platform warning surfaces, got {:?}", m.warnings);
    }
    // battlefield has none — no false positive, no warning.
    let bf = dir.join("battlefield.ssf");
    if common::present(&bf) {
        let m = parse_stage(&bf).expect("parse battlefield");
        assert!(m.platforms.iter().all(|p| !p.moving), "battlefield has no moving platforms");
        assert!(!m.warnings.iter().any(|w| w.contains("moving platform")), "no moving-platform warning");
    }
}

/// Battlefield is the iteration target: a 4-platform stage with death/camera
/// boxes and 4 spawn points. Parse it and check the extracted geometry is sane.
#[test]
fn battlefield_parses_to_geometry() {
    let p = common::ssfs_dir().join("stages").join("battlefield.ssf");
    if !common::present(&p) {
        return;
    }
    let m = parse_stage(&p).expect("parse battlefield");

    // geometry is scaled SSF2 -> FM by `size_multiplier` (so the stage matches the
    // scaled-up fighters and the art fills the FM camera). Battlefield's ~523px SSF2 floor
    // becomes ~523*scale in FM.
    assert!(m.scale > 1.0, "stage scaled up to FM (size_multiplier), got {}", m.scale);
    // a wide solid main floor + three drop-through soft platforms.
    let floor = m.main_floor().expect("main floor");
    assert!(!floor.drop_through, "main floor must be solid");
    assert!((floor.rect.w - 523.0 * m.scale).abs() < 30.0,
        "battlefield floor ~{:.0} (523*{}), got {:.0}", 523.0 * m.scale, m.scale, floor.rect.w);
    let soft = m.platforms.iter().filter(|p| p.drop_through).count();
    assert_eq!(soft, 3, "battlefield has 3 soft platforms, got {soft}");

    // boundaries present and the death box strictly contains the camera box
    // (the blast zone is the outermost boundary).
    let death = m.death_box.expect("death box");
    let cam = m.camera_box.expect("camera box");
    assert!(death.left() < cam.left() && death.right() > cam.right(), "death box wider than camera box");
    assert!(death.top() < cam.top() && death.bottom() > cam.bottom(), "death box taller than camera box");

    // 4 entrances + 4 respawns, one per player slot, indices 0..3.
    assert_eq!(m.entrances.len(), 4, "4 entrance points");
    assert_eq!(m.respawns.len(), 4, "4 respawn points");
    let mut idx: Vec<usize> = m.entrances.iter().map(|s| s.index).collect();
    idx.sort();
    assert_eq!(idx, vec![0, 1, 2, 3]);

    // battlefield carries the full SSF2 stage linkage set, so validation is clean.
    assert!(m.warnings.is_empty(), "battlefield should validate clean, got: {:?}", m.warnings);

    // metadata: clean display name (override map) + the original SSF2 soundtrack pulled
    // from the DAT, and a real FM bgm to actually play.
    assert_eq!(m.id, "battlefield", "model id is the raw SSF2 id (suffix applied by the cmd)");
    assert_eq!(m.display_name, "Battlefield");
    assert!(m.ssf2_music.contains(&"bgm_battlefield".to_string()),
        "extracted SSF2 soundtrack, got {:?}", m.ssf2_music);
    assert!(m.fm_music.iter().all(|b| b.starts_with("bgm_")) && !m.fm_music.is_empty(),
        "a real FM bgm is referenced, got {:?}", m.fm_music);

    // art: the painted backdrop (`background` plane) and the foreground both rasterize.
    // The collision masks (the `terrain` plane) are NOT art, so the stage-depth bucket is
    // empty (the fix for the doubled-stage render). Battlefield has no `_cambg` layers, so
    // its background is fixed (no parallax).
    assert!(m.art.background.is_some(), "painted backdrop rasterizes");
    assert!(m.art.foreground.is_some(), "foreground rasterizes");
    assert!(m.art.stage_frames.is_empty(), "collision masks must not render as stage art");
    assert!(m.art.parallax.is_empty(), "battlefield has no parallax (fixed background)");
}

/// Junglehijinx is the corpus's parallax stage: its `<id>_bg` carries `_cambg` camera-
/// background layers (trees, sun, sunrays). They must classify as parallax (not folded into
/// the fixed backdrop), and the emitter must produce a `parallax0` animation + camera
/// background. Corpus-gated.
#[test]
fn junglehijinx_has_parallax() {
    let p = common::ssfs_dir().join("stages").join("junglehijinx.ssf");
    if !common::present(&p) {
        return;
    }
    let m = parse_stage(&p).expect("parse junglehijinx");
    // the `_cambg` parallax composites WITH the far backdrop sky (so the sun rays draw in
    // front of the sky, not occluded behind it), and the stageMC `background` plane (island)
    // stays a fixed near-layer.
    assert!(m.art.parallax.len() >= 3, "junglehijinx has multiple per-rate parallax layers, got {}", m.art.parallax.len());
    // each layer has its own pan rate (wider layers scroll more); they are NOT all equal.
    let rates: Vec<f64> = m.art.parallax.iter().map(|p| p.x_pan).collect();
    assert!(rates.iter().any(|&r| r > 0.3) && rates.iter().any(|&r| r < 0.05), "per-layer rates differ, got {rates:?}");
    // SSF2 `_cambg` discrete layers pan (the autoPanMultiplier feeds PAN_MODE); BOUNDS is the
    // tiling/wrapping mode.
    assert!(m.art.parallax.iter().all(|p| p.mode == ssf2_converter::ParallaxMode::Pan),
        "cambg layers use PAN mode (BOUNDS is for tiling backdrops)");
    assert!(m.art.background.is_some(), "fixed near-background (island) present");
    // the island terrain is sloped, so the main floor traces a polyline, not a flat line.
    let floor = m.main_floor().expect("main floor");
    let profile = floor.profile.as_ref().expect("curved floor has a traced profile");
    assert!(profile.len() >= 3, "junglehijinx floor is a polyline, got {} points", profile.len());
    let ys: Vec<f64> = profile.iter().map(|p| p.1).collect();
    let span = ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min);
    assert!(span > 20.0, "floor surface actually rises/falls (slope ~{span:.0}px)");

    let tmp = std::env::temp_dir().join("peptide-stage-parallax-test");
    let _ = std::fs::remove_dir_all(&tmp);
    let (dir, _) = emit_stage(&m, &tmp).expect("emit");
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("library").join("entities").join(format!("{}.entity", m.id))).unwrap()).unwrap();
    let anim_names: Vec<&str> = v["animations"].as_array().unwrap().iter()
        .filter_map(|a| a["name"].as_str()).collect();
    assert!(anim_names.contains(&"parallax0"), "emits a parallax0 animation, got {anim_names:?}");
    let stats = std::fs::read_to_string(dir.join("library").join("scripts").join("stage").join(format!("{}StageStats.hx", m.id))).unwrap();
    assert!(stats.contains("animationId: \"parallax0\""), "StageStats declares the camera background");
    // the Floor line segment carries the multi-point polyline.
    let max_pts = v["symbols"].as_array().unwrap().iter()
        .filter(|s| s["type"] == "LINE_SEGMENT")
        .map(|s| s["points"].as_array().map(|a| a.len()).unwrap_or(0)).max().unwrap_or(0);
    assert!(max_pts > 4, "the floor line segment is a polyline (> 2 points), got {} coords", max_pts);
}

/// Emit the FM stage package and assert the `.entity` graph is internally
/// consistent: every animation layer resolves, every keyframe's symbol resolves.
#[test]
fn battlefield_emits_consistent_entity() {
    let p = common::ssfs_dir().join("stages").join("battlefield.ssf");
    if !common::present(&p) {
        return;
    }
    let m = parse_stage(&p).expect("parse");
    let tmp = std::env::temp_dir().join("peptide-stage-test");
    let _ = std::fs::remove_dir_all(&tmp);
    let (dir, fraytools) = emit_stage(&m, &tmp).expect("emit");
    assert!(fraytools.exists(), "wrote .fraytools");

    let entity_path = dir.join("library").join("entities").join(format!("{}.entity", m.id));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&entity_path).unwrap()).unwrap();

    let layer_ids: std::collections::HashSet<&str> =
        v["layers"].as_array().unwrap().iter().map(|l| l["$id"].as_str().unwrap()).collect();
    let symbol_ids: std::collections::HashSet<&str> =
        v["symbols"].as_array().unwrap().iter().map(|s| s["$id"].as_str().unwrap()).collect();
    let kf_ids: std::collections::HashSet<&str> =
        v["keyframes"].as_array().unwrap().iter().map(|k| k["$id"].as_str().unwrap()).collect();

    // every layer referenced by the stage animation exists
    let anim = &v["animations"][0];
    for lid in anim["layers"].as_array().unwrap() {
        assert!(layer_ids.contains(lid.as_str().unwrap()), "animation layer {lid} resolves");
    }
    // every keyframe's symbol + every layer's keyframes resolve
    for k in v["keyframes"].as_array().unwrap() {
        if let Some(s) = k["symbol"].as_str() {
            assert!(symbol_ids.contains(s), "keyframe symbol {s} resolves");
        }
    }
    for l in v["layers"].as_array().unwrap() {
        for k in l["keyframes"].as_array().unwrap() {
            assert!(kf_ids.contains(k.as_str().unwrap()), "layer keyframe resolves");
        }
    }

    // objectType STAGE, type:"stage" manifest entry.
    assert_eq!(v["pluginMetadata"]["com.fraymakers.FraymakersMetadata"]["objectType"], "STAGE");
    let man: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("library").join("manifest.json")).unwrap()).unwrap();
    assert_eq!(man["content"][0]["type"], "stage");

    // the required Fraymakers stage layers are present (emit_stage bails otherwise, but
    // assert here too so a regression names the missing layer). For battlefield the only
    // IMAGE layers are the backdrop + foreground (no collision-silhouette stage art): the
    // doubled-render fix.
    let layers = v["layers"].as_array().unwrap();
    let named = |n: &str| layers.iter().any(|l| l["name"] == n);
    let meta_eq = |key: &str, val: &str| layers.iter().any(|l|
        l.pointer(&format!("/pluginMetadata/com.fraymakers.FraymakersMetadata/{key}")).and_then(|x| x.as_str()) == Some(val));
    assert!(meta_eq("containerType", "CHARACTERS_CONTAINER"), "has Characters container");
    assert!(meta_eq("pointType", "ENTRANCE_POINT"), "has an entrance point");
    assert!(meta_eq("pointType", "RESPAWN_POINT"), "has a respawn point");
    assert!(named("Background Art"), "has the painted backdrop layer");
    assert!(named("Foreground Art"), "has the foreground layer");
    let image_layers = layers.iter().filter(|l| l["type"] == "IMAGE").count();
    assert_eq!(image_layers, 2, "battlefield has exactly 2 art layers (backdrop + foreground), no collision silhouette");
}
