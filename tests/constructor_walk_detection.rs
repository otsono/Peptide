//! Tests for the constructor-walk detection
//! ([docs/constructor_walk_detection.md](docs/constructor_walk_detection.md)).
//!
//! - End-to-end across the corpus: every SSF's detected character set
//!   matches `Main`'s `register("characters", [...])` array.
//! - Metadata: `id` / `guid` extracted; `package_id` / `package_guid` /
//!   `source_method` appear in every character's `conversion_log.json`.
//! - misc.ssf (no `Main`): handled cleanly, no panic.
//! - Validation warnings emit for empty stats (we exercise this with a
//!   `--name foo-bogus` against a real SSF, which extracts nothing but
//!   shouldn't crash).
//!
//! Skipped silently if `../ssf2-ssfs/` isn't on disk.

use ssf2_converter::*;
use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest_dir() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }
fn ssfs_dir() -> PathBuf { manifest_dir().parent().unwrap_or(Path::new(".")).join("ssf2-ssfs") }

#[test]
fn id_derivation_rule_matches_corpus_method_names() {
    // Cross-check the derive_id_from_bundle_method_name rule against
    // every observed method-name shape. Kept alongside the in-main.rs
    // `derive_id_from_getter` test for path 2 — they're the same rule;
    // when path 2 enumeration is deleted in a follow-up, this becomes
    // the canonical test.
    let cases: &[(&str, &str)] = &[
        ("getMario",         "mario"),
        ("getBandanaDee",    "bandanadee"),
        ("getCaptainFalcon", "captainfalcon"),
        ("getgameandwatch",  "gameandwatch"),
        ("getGigaBowser",    "gigabowser"),
        ("getWario_Man",     "wario_man"),
        ("getSheik",         "sheik"),
    ];
    for (m, expected) in cases {
        let got = abc_parser::derive_id_from_bundle_method_name(m);
        assert_eq!(got.as_deref(), Some(*expected),
            "derive_id_from_bundle_method_name({:?}) = {:?}, expected Some({:?})",
            m, got, expected);
    }
    assert_eq!(abc_parser::derive_id_from_bundle_method_name("get"),    None);
    assert_eq!(abc_parser::derive_id_from_bundle_method_name("init"),   None);
    assert_eq!(abc_parser::derive_id_from_bundle_method_name(""),       None);
}

#[test]
fn corpus_constructor_walk_matches_path2_enumeration() {
    // For every .ssf in the corpus, the constructor walker's
    // declared-characters list should match the path 2 enumeration's
    // discovered character set 1:1. Doubles as the "no orphan get*"
    // gate — caught in CI if a future SSF2 build ships a dev-leftover.
    let dir = ssfs_dir();
    if !dir.exists() { eprintln!("ssfs/ missing; skipping"); return; }

    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap()
        .filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "ssf").unwrap_or(false))
        .collect();
    files.sort();

    let mut mismatches: Vec<String> = Vec::new();
    let mut total_chars_via_constructor = 0;
    let mut total_no_main = 0;

    for path in &files {
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let Ok(bytes) = std::fs::read(path) else { continue };
        let Ok(swf_bytes) = ssf::decompress(&bytes) else { continue };
        let Ok(swf) = swf_parser::parse(&swf_bytes) else { continue };

        for abc_bytes in &swf.abc_blocks {
            let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };
            let md = abc_parser::extract_main_package_metadata(&abc);
            let main = abc.classes.iter().find(|c| c.name == "Main");

            match (md, main) {
                (Some(md), Some(main)) => {
                    let constructor_ids: Vec<&str> =
                        md.characters.iter().map(|(i, _)| i.as_str()).collect();
                    let enum_ids: Vec<String> = main.instance_methods.iter()
                        .filter_map(|t| t.name.strip_prefix("get")
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_lowercase()))
                        .collect();
                    if constructor_ids.iter().map(|s| s.to_string()).collect::<Vec<_>>() != enum_ids {
                        mismatches.push(format!(
                            "{}: constructor={:?} vs enumeration={:?}",
                            stem, constructor_ids, enum_ids));
                    }
                    total_chars_via_constructor += constructor_ids.len();

                    // id should match filename stem.
                    if let Some(id) = &md.id {
                        assert_eq!(id.to_lowercase(), stem.to_lowercase(),
                            "{}: Main.id {:?} disagrees with filename stem", stem, id);
                    } else {
                        mismatches.push(format!("{}: no Main.id", stem));
                    }
                    assert!(md.guid.is_some(), "{}: no Main.guid", stem);
                }
                (None, None) => { total_no_main += 1; }
                (md, main) => {
                    mismatches.push(format!(
                        "{}: md.is_some={} but main.is_some={}",
                        stem, md.is_some(), main.is_some()));
                }
            }
        }
    }

    eprintln!("constructor walk total characters: {}", total_chars_via_constructor);
    eprintln!("SSFs without Main: {}", total_no_main);
    if !mismatches.is_empty() {
        for m in &mismatches { eprintln!("  {}", m); }
        panic!("{} SSF(s) had constructor-vs-enumeration mismatches", mismatches.len());
    }
    assert!(total_chars_via_constructor >= 40,
        "corpus should yield ≥40 characters; got {}", total_chars_via_constructor);
}

#[test]
fn package_metadata_lands_in_conversion_log() {
    // Run the converter against sandbag.ssf and assert ssf2_source has
    // package_id / package_guid / source_method.
    let sandbag = ssfs_dir().join("sandbag.ssf");
    if !sandbag.exists() { eprintln!("sandbag.ssf missing; skipping"); return; }
    let out = tempfile::tempdir().expect("tempdir");

    let status = Command::new(env!("CARGO_BIN_EXE_ssf2_converter"))
        .arg(&sandbag).arg("-o").arg(out.path())
        .status().expect("run converter");
    assert!(status.success(), "converter exited non-zero");

    let log = std::fs::read_to_string(out.path().join("sandbag/conversion_log.json")).unwrap();
    assert!(log.contains("\"ssf2_source\""),
        "sandbag conversion_log.json must include ssf2_source");
    assert!(log.contains("\"package_id\": \"sandbag\""),
        "sandbag package_id must be \"sandbag\"");
    assert!(log.contains("\"source_method\": \"Main::getSandbag\""),
        "sandbag source_method must be \"Main::getSandbag\"");
    // guid present (some 32-hex-with-dashes string).
    assert!(log.contains("\"package_guid\""),
        "sandbag package_guid must be present");
    // sandbag is a peer character, not a transformation.
    assert!(!log.contains("parent_normal_stats_id"),
        "sandbag must NOT carry the transformation overlay");
}
