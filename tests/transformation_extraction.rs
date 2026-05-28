//! Transformation-character extraction tests. Verifies that bowser.ssf
//! and wario.ssf emit both their normal forms AND their Final-Smash
//! transformations as separate character packages, with the appropriate
//! TODO banner + `ssf2_source` metadata.
//!
//! Skipped silently if the corpus isn't on disk.

use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest_dir() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }

fn ssf_path(name: &str) -> PathBuf {
    manifest_dir().parent().unwrap_or(Path::new("."))
        .join(format!("ssf2-ssfs/{}.ssf", name))
}

fn run_converter(ssf: &Path, out: &Path) {
    let status = Command::new(env!("CARGO_BIN_EXE_ssf2_converter"))
        .arg(ssf).arg("-o").arg(out)
        .status().expect("run converter");
    assert!(status.success(), "converter exited non-zero for {}", ssf.display());
}

fn assert_transformation_package(out: &Path, parent: &str, transformation: &str, source_method: &str) {
    let dir = out.join(format!("{}/library/scripts/Character", transformation));
    for f in &["CharacterStats.hx", "AnimationStats.hx", "HitboxStats.hx", "Script.hx"] {
        let p = dir.join(f);
        assert!(p.exists(), "{} should exist for {}", p.display(), transformation);
    }
    let stats = std::fs::read_to_string(dir.join("CharacterStats.hx")).unwrap();
    assert!(stats.contains("TRANSFORMATION FORM"),
        "{}'s CharacterStats.hx must carry the TODO transformation banner", transformation);
    assert!(stats.contains(parent),
        "{}'s banner must mention parent {:?}", transformation, parent);
    assert!(stats.contains(source_method),
        "{}'s banner must mention source method {:?}", transformation, source_method);

    let log = std::fs::read_to_string(out.join(format!("{}/conversion_log.json", transformation))).unwrap();
    assert!(log.contains("ssf2_source"),
        "{}'s conversion_log.json must include ssf2_source", transformation);
    assert!(log.contains(parent),
        "{}'s conversion_log.json must reference parent {:?}", transformation, parent);
}

#[test]
fn bowser_ssf_emits_bowser_and_gigabowser() {
    let ssf = ssf_path("bowser");
    if !ssf.exists() { eprintln!("ssf2-ssfs/bowser.ssf missing; skipping"); return; }
    let out = tempfile::tempdir().expect("tempdir");
    run_converter(&ssf, out.path());

    assert!(out.path().join("bowser").exists(),       "characters/bowser must exist");
    assert!(out.path().join("gigabowser").exists(),   "characters/gigabowser must exist");

    // Parent does NOT have the transformation banner / ssf2_source.
    let parent_stats = std::fs::read_to_string(
        out.path().join("bowser/library/scripts/Character/CharacterStats.hx")).unwrap();
    assert!(!parent_stats.contains("TRANSFORMATION FORM"),
        "Bowser (parent) must not carry the TODO banner");

    // Transformation does.
    assert_transformation_package(out.path(), "bowser", "gigabowser", "Main::getGigaBowser");

    // Differentiating data: Giga's projectile pipeline must produce
    // GigaFireBreath{,Blue,Purple} stat/hitbox files — these come from
    // getGigaBowser's bundle's pData, NOT from getBowser's
    // fireBreath{,Blue,Purple}.
    let giga_proj_dir = out.path().join("gigabowser/library/scripts/Projectile");
    assert!(giga_proj_dir.join("GigaFireBreathHitboxStats.hx").exists(),
        "Giga's GigaFireBreathHitboxStats.hx must exist");
    assert!(giga_proj_dir.join("GigaFireBreathBlueHitboxStats.hx").exists(),
        "Giga's GigaFireBreathBlueHitboxStats.hx must exist");
    assert!(giga_proj_dir.join("GigaFireBreathPurpleHitboxStats.hx").exists(),
        "Giga's GigaFireBreathPurpleHitboxStats.hx must exist");
}

#[test]
fn wario_ssf_emits_wario_and_wario_man() {
    let ssf = ssf_path("wario");
    if !ssf.exists() { eprintln!("ssf2-ssfs/wario.ssf missing; skipping"); return; }
    let out = tempfile::tempdir().expect("tempdir");
    run_converter(&ssf, out.path());

    assert!(out.path().join("wario").exists(),     "characters/wario must exist");
    assert!(out.path().join("wario_man").exists(), "characters/wario_man must exist");

    let parent_stats = std::fs::read_to_string(
        out.path().join("wario/library/scripts/Character/CharacterStats.hx")).unwrap();
    assert!(!parent_stats.contains("TRANSFORMATION FORM"),
        "Wario (parent) must not carry the TODO banner");

    assert_transformation_package(out.path(), "wario", "wario_man", "Main::getWario_Man");
}
