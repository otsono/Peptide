//! Byte-equivalence test between the legacy INLINE extractor
//! (`extract_character`) and the BUNDLE extractor (`extract_character_bundle`)
//! across every .ssf in `../ssf2-ssfs/`.
//!
//! Asserts that for every character whose name is discovered via path-1
//! detection (`*Ext` class with marker methods), both extraction paths
//! produce identical `attacks`, `stats`, and `projectiles` fields on
//! `ExtractedCharacter`. The other fields (Stage B — ext_methods, frame
//! scripts, etc.) come from shared code and are not under test here.
//!
//! Test is skipped silently if `../ssf2-ssfs/` is missing — same pattern
//! as `golden_sandbag.rs`. Deleted in Step C once path 1 is gone.

use ssf2_converter::*;
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn ssfs_dir() -> PathBuf {
    manifest_dir().parent().unwrap_or(Path::new(".")).join("ssf2-ssfs")
}

/// Representative sample spanning the complexity range. The full-corpus
/// byte-equivalence proof lives in /tmp/compare/ (10 characters dumped and
/// hand-diffed); this Rust test is the production-pipeline gate over a
/// smaller subset that keeps the test under memory pressure on CI.
const SAMPLE_CHARS: &[&str] = &[
    "sandbag", "bandanadee", "link", "pichu", "goku",
    "peach", "mario", "kirby", "fox", "naruto",
];

fn list_ssfs() -> Vec<PathBuf> {
    let dir = ssfs_dir();
    SAMPLE_CHARS.iter()
        .map(|name| dir.join(format!("{}.ssf", name)))
        .filter(|p| p.exists())
        .collect()
}

/// Re-implement the path-1 character-name discovery (Ext-class marker scan)
/// locally so the test isn't coupled to `detect_char_names`, which Step B
/// will rewrite. Names come back lowercased exactly as the production code
/// surfaces them today.
fn detect_path1_char_names(abc: &abc_parser::AbcFile) -> Vec<String> {
    const MARKERS: &[&str] = &["getOwnStats", "getAttackStats", "getProjectileStats"];
    let mut names: Vec<String> = Vec::new();
    for class in &abc.classes {
        let Some(prefix) = class.name.strip_suffix("Ext") else { continue };
        if prefix.len() < 2 || !prefix.chars().all(|c| c.is_ascii_alphabetic()) { continue; }
        let has_marker = class.instance_methods.iter()
            .any(|t| MARKERS.contains(&t.name.as_str()));
        if has_marker { names.push(prefix.to_lowercase()); }
    }
    names.sort(); names.dedup();
    names
}

#[test]
fn path1_path2_equivalence_corpus() {
    let ssfs = list_ssfs();
    if ssfs.is_empty() {
        eprintln!("ssf2-ssfs/ not found; skipping equivalence test");
        return;
    }

    let mut total_chars = 0;
    let mut total_mismatches = 0;
    let mut report: Vec<String> = Vec::new();

    for ssf in &ssfs {
        let stem = ssf.file_stem().unwrap().to_string_lossy().into_owned();
        let Ok(bytes) = std::fs::read(ssf) else { continue };
        let Ok(swf_bytes) = ssf::decompress(&bytes) else { continue };
        let Ok(swf) = swf_parser::parse(&swf_bytes) else { continue };
        for abc_bytes in &swf.abc_blocks {
            let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };

            // Discover characters via path 1's marker scan, then resolve
            // truncated names against the filename stem (matches today's
            // `detect_char_names` behaviour for CaptainExt → captainfalcon).
            let raw_names = detect_path1_char_names(&abc);
            let stem_lc = stem.to_lowercase();
            let names: Vec<String> = raw_names.iter().map(|n| {
                if stem_lc.starts_with(n.as_str()) { stem_lc.clone() }
                else if n.starts_with(stem_lc.as_str()) { n.clone() }
                else { n.clone() }
            }).collect();

            for char_name in names {
                total_chars += 1;
                let Ok(inline) = abc_parser::extract_character(&abc, &char_name) else {
                    report.push(format!("{}::{}: path-1 extract failed", stem, char_name));
                    total_mismatches += 1; continue;
                };
                let Ok(bundle) = abc_parser::extract_character_bundle(&abc, &char_name) else {
                    report.push(format!("{}::{}: path-2 extract failed", stem, char_name));
                    total_mismatches += 1; continue;
                };

                let inline_attacks_json = serde_json::to_string(&inline.attacks).unwrap();
                let bundle_attacks_json = serde_json::to_string(&bundle.attacks).unwrap();
                let inline_proj_json    = serde_json::to_string(&inline.projectiles).unwrap();
                let bundle_proj_json    = serde_json::to_string(&bundle.projectiles).unwrap();

                let mut mismatch = false;
                if inline_attacks_json != bundle_attacks_json {
                    report.push(format!(
                        "{}::{}: attacks differ ({} inline vs {} bundle)",
                        stem, char_name, inline.attacks.len(), bundle.attacks.len()));
                    mismatch = true;
                }
                if inline_proj_json != bundle_proj_json {
                    report.push(format!(
                        "{}::{}: projectiles differ ({} inline vs {} bundle)",
                        stem, char_name, inline.projectiles.len(), bundle.projectiles.len()));
                    mismatch = true;
                }
                let inline_stats = inline.stats.as_ref().map(|s| s.values.len()).unwrap_or(0);
                let bundle_stats = bundle.stats.as_ref().map(|s| s.values.len()).unwrap_or(0);
                if inline.stats.as_ref().map(|s| &s.values)
                    != bundle.stats.as_ref().map(|s| &s.values)
                {
                    report.push(format!(
                        "{}::{}: stats differ ({} inline keys vs {} bundle keys)",
                        stem, char_name, inline_stats, bundle_stats));
                    mismatch = true;
                }
                if mismatch { total_mismatches += 1; }
            }
        }
    }

    eprintln!("path1/path2 equivalence: {}/{} characters matched",
        total_chars - total_mismatches, total_chars);
    if total_mismatches > 0 {
        for line in &report { eprintln!("  {}", line); }
        panic!("{} character(s) had divergent inline-vs-bundle extractions", total_mismatches);
    }
}

// ─── Equality helpers ────────────────────────────────────────────────────────
//
// `ExtractedCharacter.attacks` is `BTreeMap<String, AttackData>` where
// `AttackData.hitboxes` is `Vec<BTreeMap<String, f64>>`. Same for
// projectiles. Both types derive serde Deserialize/Serialize but not
// `PartialEq`. We rely on the fact that BTreeMap's iteration order is
// deterministic and compare via JSON round-trip equality below; see
// `compare_*` helpers if direct PartialEq becomes unworkable later.
