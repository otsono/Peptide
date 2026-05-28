//! End-to-end Sheik extraction test. Sheik has no `SheikExt` class in
//! zelda.ssf — only a `Main::getSheik` bundle and her own `sheik`
//! MovieClip. Path 2 picks her up automatically.
//!
//! Asserts she gets a sane output package (non-empty CharacterStats /
//! AnimationStats / HitboxStats / Script). Skipped silently if the
//! corpus isn't on disk (matches `golden_sandbag.rs` pattern).

use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest_dir() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }

fn zelda_ssf_path() -> PathBuf {
    manifest_dir().parent().unwrap_or(Path::new(".")).join("ssf2-ssfs/zelda.ssf")
}

#[test]
fn sheik_emits_full_package_from_zelda_ssf() {
    let ssf = zelda_ssf_path();
    if !ssf.exists() {
        eprintln!("ssf2-ssfs/zelda.ssf not found; skipping sheik test");
        return;
    }

    let out = tempfile::tempdir().expect("tempdir");
    let status = Command::new(env!("CARGO_BIN_EXE_ssf2_converter"))
        .arg(&ssf).arg("--name").arg("sheik")
        .arg("-o").arg(out.path())
        .status().expect("run converter");
    assert!(status.success(), "converter exited non-zero for sheik");

    let sheik = out.path().join("sheik/library/scripts/Character");
    for f in &["CharacterStats.hx", "AnimationStats.hx", "HitboxStats.hx", "Script.hx"] {
        let p = sheik.join(f);
        assert!(p.exists(), "expected {} to exist", p.display());
        let body = std::fs::read_to_string(&p).expect("read");
        assert!(body.len() > 100, "{} is suspiciously short ({} bytes)", f, body.len());
    }

    // CharacterStats.hx must NOT carry the transformation banner — Sheik's
    // cData.normalStats_id is `sheik` (matches her derived id).
    let stats_body = std::fs::read_to_string(sheik.join("CharacterStats.hx")).unwrap();
    assert!(!stats_body.contains("TRANSFORMATION FORM"),
        "Sheik must not have the transformation TODO banner");

    // conversion_log.json must NOT include ssf2_source for the same reason.
    let log = std::fs::read_to_string(out.path().join("sheik/conversion_log.json")).unwrap();
    assert!(!log.contains("ssf2_source"),
        "Sheik's conversion_log.json must not include ssf2_source metadata");

    // HitboxStats.hx should mention one of her signature attacks
    // (needle / chain / lightarrow are the canonical Sheik moves).
    let hb = std::fs::read_to_string(sheik.join("HitboxStats.hx")).unwrap().to_lowercase();
    assert!(hb.contains("needle") || hb.contains("chain") || hb.contains("lightarrow")
            || hb.contains("vanish") || hb.contains("sheik"),
        "Sheik's HitboxStats.hx should reference at least one signature move");
}
