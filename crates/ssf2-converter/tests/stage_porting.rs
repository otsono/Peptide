//! Stage-porting pipeline tests: parse an SSF2 stage and emit a Fraymakers stage
//! package, asserting the geometry and the entity graph. Corpus-gated (skips
//! cleanly without the developer's `.ssf` files).

mod common;

use ssf2_converter::{emit_stage, parse_stage};

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
        if let Err(e) = parse_stage(&p) {
            failed.push(format!("{}: {e}", p.file_name().unwrap().to_string_lossy()));
        }
    }
    assert!(failed.is_empty(), "{}/{} stages failed to parse:\n{}", failed.len(), total, failed.join("\n"));
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

    // a wide solid main floor + three drop-through soft platforms.
    let floor = m.main_floor().expect("main floor");
    assert!(!floor.drop_through, "main floor must be solid");
    assert!(floor.rect.w > 400.0, "battlefield floor is ~520px wide, got {}", floor.rect.w);
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
}
