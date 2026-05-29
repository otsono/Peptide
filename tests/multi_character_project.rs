//! Multi-character project emission tests
//! (docs/multi_character_projects_plan.md §2). Asserts the merged
//! project shape, collision suffix rule, per-character menu naming,
//! and project-level conversion log.

use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest_dir() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }
fn ssf_path(name: &str) -> PathBuf {
    manifest_dir().parent().unwrap_or(Path::new("."))
        .join(format!("ssf2-ssfs/{}.ssf", name))
}
fn run_converter(ssf: &Path, out: &Path, extra: &[&str]) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ssf2_converter"));
    cmd.arg(ssf).arg("-o").arg(out);
    for a in extra { cmd.arg(a); }
    let status = cmd.status().expect("run converter");
    assert!(status.success(), "converter exited non-zero for {}", ssf.display());
}

#[test]
fn zelda_ssf_emits_one_merged_project() {
    let ssf = ssf_path("zelda");
    if !ssf.exists() { eprintln!("zelda.ssf missing; skipping"); return; }
    let out = tempfile::tempdir().expect("tempdir");
    run_converter(&ssf, out.path(), &[]);

    let project = out.path().join("zelda");
    assert!(project.exists(),                                "characters/zelda must exist");
    assert!(project.join("zelda.fraytools").exists(),        ".fraytools at project root");
    assert!(!out.path().join("sheik").exists(),              "sheik must NOT exist as a standalone project");
    assert!(!project.join("sheik.fraytools").exists(),       "sheik.fraytools must NOT exist");

    // Both characters in entities/ under <Pascal>.entity.
    let entities = project.join("library/entities");
    assert!(entities.join("Zelda.entity").exists(), "Zelda.entity missing");
    assert!(entities.join("Sheik.entity").exists(), "Sheik.entity missing");

    // Per-character menu entities at <Pascal>_Menu.entity (at least one
    // is expected since both have head sprites).
    let zelda_menu = entities.join("Zelda_Menu.entity").exists();
    let sheik_menu = entities.join("Sheik_Menu.entity").exists();
    assert!(zelda_menu || sheik_menu,
        "expected at least one <Pascal>_Menu.entity to exist");

    // Per-character scripts subdirs.
    for sub in &["Zelda", "Sheik"] {
        let dir = project.join(format!("library/scripts/{}", sub));
        for f in &["CharacterStats.hx", "AnimationStats.hx", "HitboxStats.hx", "Script.hx"] {
            assert!(dir.join(f).exists(),
                "{}/library/scripts/{}/{} must exist",
                project.display(), sub, f);
        }
    }

    // Collision suffix rule: Zelda (constructor-walk slot 0) keeps the
    // unsuffixed costumes.palettes; Sheik (slot 1) gets `2`.
    let lib = project.join("library");
    assert!(lib.join("costumes.palettes").exists(),           "slot 0 (zelda) costumes.palettes must exist");
    assert!(lib.join("costumes.palettes2").exists(),          "slot 1 (sheik) costumes.palettes2 must exist");
    assert!(lib.join("costumes.palettes.meta").exists(),      "slot 0 .meta must exist");
    assert!(lib.join("costumes.palettes2.meta").exists(),     "slot 1 .meta must exist");
    let sprites = project.join("library/sprites");
    assert!(sprites.join("palette_preview.png").exists(),     "slot 0 palette_preview.png must exist");
    assert!(sprites.join("palette_preview.png2").exists(),    "slot 1 palette_preview.png2 must exist");

    // Manifest has BOTH characters with distinct asset ids + per-char menu pointers.
    let manifest_raw = std::fs::read_to_string(lib.join("manifest.json")).unwrap();
    let m: serde_json::Value = serde_json::from_str(&manifest_raw).expect("manifest parses");
    let chars: Vec<&serde_json::Value> = m["content"].as_array().unwrap().iter()
        .filter(|c| c["type"] == "character").collect();
    assert_eq!(chars.len(), 2, "manifest must have two character entries; got {:?}", chars);
    let ids: Vec<&str> = chars.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"zelda") && ids.contains(&"sheik"),
        "manifest characters must be zelda+sheik; got {:?}", ids);
    for c in &chars {
        let id = c["id"].as_str().unwrap();
        let expected_entity_id = format!("{}_menu", id);
        assert_eq!(c["metadata"]["ui"]["entityId"], expected_entity_id,
            "{}: entityId must be {:?}", id, expected_entity_id);
        assert_eq!(c["objectStatsId"], format!("{}CharacterStats", id),
            "{}: objectStatsId must be character-namespaced", id);
        assert_eq!(c["scriptId"], format!("{}Script", id),
            "{}: scriptId must be character-namespaced", id);
    }

    // Project-level conversion log with characters: [...] array.
    let log_raw = std::fs::read_to_string(project.join("conversion_log.json")).unwrap();
    let log: serde_json::Value = serde_json::from_str(&log_raw).expect("log parses");
    assert_eq!(log["project"], "zelda");
    let chars_in_log: Vec<&str> = log["characters"].as_array().unwrap().iter()
        .map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(chars_in_log, vec!["zelda", "sheik"]);
}

#[test]
fn rollback_flag_emits_per_character_projects() {
    // --per-character-projects reverts to the pre-Stage-B layout: each
    // character gets its own .fraytools project. This is the rollback
    // escape hatch retained for one release per the plan.
    let ssf = ssf_path("zelda");
    if !ssf.exists() { eprintln!("zelda.ssf missing; skipping"); return; }
    let out = tempfile::tempdir().expect("tempdir");
    run_converter(&ssf, out.path(), &["--per-character-projects"]);

    for char_id in &["zelda", "sheik"] {
        let dir = out.path().join(char_id);
        assert!(dir.exists(),                                       "{}/ must exist under rollback", char_id);
        assert!(dir.join(format!("{}.fraytools", char_id)).exists(),"{}.fraytools must exist under rollback", char_id);
        assert!(dir.join("library/manifest.json").exists(),         "{} manifest must exist under rollback", char_id);
        assert!(dir.join("conversion_log.json").exists(),           "{} conversion_log must exist under rollback", char_id);
    }
}
