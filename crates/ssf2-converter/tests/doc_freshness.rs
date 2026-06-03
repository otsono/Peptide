//! Doc-freshness gate. Flags known-stale terms in the top-level docs so
//! a doc that drifts back to a paradigm we've replaced fails CI loudly
//! rather than silently misleading future contributors.
//!
//! Add a new pattern any time you delete a user-visible function /
//! flag / output field. The cost is a few lines; the saved future
//! confusion is large.
//!
//! Each pattern is a `StalePattern { needle, allowed_files,
//! reason }`. The check is: the `needle` substring must not appear in
//! any file under `MD_FILES` *except* the ones listed in
//! `allowed_files` (which is where the legitimate historical mention
//! lives — e.g. a "Status: implemented" plan doc that names the old
//! mechanism).
//!
//! NOT a substitute for human judgment. False positives mean we picked
//! a needle that's too generic; tighten it.

use std::fs;
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }

/// Repo root: the docs checked here (README.md, docs/*) live at the top level,
/// two levels up from this crate (`<repo>/crates/ssf2-converter`).
fn repo_root() -> PathBuf {
    manifest_dir()
        .parent().and_then(|p| p.parent()).map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Top-level docs that must stay current. The `docs/` historical plan
/// files are explicitly NOT in this list — they keep their old
/// language with a "Status: implemented" banner pointing at the
/// current docs.
const MD_FILES: &[&str] = &[
    "README.md",
    "DEVELOPMENT.md",
    "AGENT_CONTEXT.md",
    "CONTRIBUTING.md",
];

struct StalePattern {
    /// Substring that should not appear in any doc except the allow-list.
    needle: &'static str,
    /// Files where this term is allowed because it's historical /
    /// architectural context.
    allowed_files: &'static [&'static str],
    /// Why this pattern is stale — printed in the failure message.
    reason: &'static str,
}

const STALE_PATTERNS: &[StalePattern] = &[
    // ─── Removed AS3-side function references ──────────────────────────────
    StalePattern {
        needle: "extract_costume_data_from_apply_palette",
        allowed_files: &[],
        reason: "Function removed in Phase 2 cleanup (commit 7671defe). \
                 Costume extraction now uses scan_all_costume_methods + \
                 decode_costume_objects.",
    },
    StalePattern {
        needle: "extract_largest_numeric_object",
        allowed_files: &[],
        reason: "Function removed in Phase 2 cleanup (commit 7671defe).",
    },
    StalePattern {
        needle: "extract_frame_actions",
        allowed_files: &[],
        reason: "Function removed in Phase 2 cleanup (commit 7671defe).",
    },

    // ─── Old detection paradigm wording ─────────────────────────────────────
    StalePattern {
        needle: "Scan ABC for `XxxExt` classes",
        allowed_files: &["docs/path2_unification_plan.md"],
        reason: "Detection switched to walking Main's iinit constructor; \
                 see docs/constructor_walk_detection.md. The pre-path-2 \
                 wording survives only in path2_unification_plan.md (the \
                 historical plan).",
    },
    StalePattern {
        needle: "scans `*Ext` classes with marker methods",
        allowed_files: &["docs/path2_unification_plan.md"],
        reason: "Same as above — path 2 detection is gone except for the \
                 fallback enumeration; constructor walk is primary.",
    },

    // ─── Old CLI flag references ────────────────────────────────────────────
    StalePattern {
        needle: "--legacy-inline",
        allowed_files: &["docs/path2_unification_plan.md"],
        reason: "Flag was removed in Step C of the path 2 migration.",
    },
    StalePattern {
        needle: "extract_costumes binary",
        allowed_files: &[],
        reason: "There has never been (in current code) a standalone \
                 extract_costumes binary; costume extraction is \
                 in-process inside ssf2_converter.",
    },

    // ─── Old `extract_character_bundle` shim mention ────────────────────────
    StalePattern {
        needle: "extract_character_bundle",
        allowed_files: &["docs/path2_unification_plan.md"],
        reason: "The shim was inlined into extract_character in Step C of \
                 the path 2 migration. Current Stage A lives directly in \
                 extract_character.",
    },

    // ─── Pre-PascalCase-rename output paths (Stage A of the multi-char work) ──
    // The character entity is now library/entities/<Pascal>.entity and the
    // scripts subdir is library/scripts/<Pascal>/. These literal old paths
    // must never reappear in the docs. Needles are path-qualified so the
    // legitimate "(was Character.entity)" / "(was Character/)" historical
    // notes in DEVELOPMENT §9 don't trip the wire.
    StalePattern {
        needle: "entities/Character.entity",
        allowed_files: &["docs/multi_character_projects_plan.md"],
        reason: "Character.entity → <Pascal>.entity (e.g. Mario.entity) in the \
                 Stage A universal rename (commit 388e6faf). The plan doc keeps \
                 the old path in its before/after migration tables.",
    },
    StalePattern {
        needle: "scripts/Character/",
        // DEVELOPMENT.md §14 documents the rename as an explicit "old → new"
        // (`scripts/Character/` → `scripts/<Pascal>/`), which is legitimate
        // historical context, not a stale instruction.
        allowed_files: &["docs/multi_character_projects_plan.md", "DEVELOPMENT.md"],
        reason: "library/scripts/Character/ → library/scripts/<Pascal>/ (e.g. \
                 scripts/Mario/) in the Stage A universal rename (commit 388e6faf).",
    },
];

#[test]
fn no_stale_terms_in_top_level_docs() {
    let mut failures: Vec<String> = Vec::new();

    for md in MD_FILES {
        let path = repo_root().join(md);
        let body = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => {
                // Not all docs are required to exist (CONTRIBUTING.md
                // could be added later, etc.) — skip missing files.
                continue;
            }
        };

        for pat in STALE_PATTERNS {
            if pat.allowed_files.contains(md) { continue; }
            if !body.contains(pat.needle) { continue; }

            // Found a stale term. Surface line numbers for fast triage.
            let hits: Vec<(usize, &str)> = body.lines().enumerate()
                .filter(|(_, l)| l.contains(pat.needle))
                .map(|(n, l)| (n + 1, l))
                .collect();
            let mut msg = format!(
                "\n  {} contains stale term {:?}\n    reason: {}\n    hits:",
                md, pat.needle, pat.reason);
            for (n, l) in hits {
                msg.push_str(&format!("\n      L{}: {}", n, l.trim()));
            }
            failures.push(msg);
        }
    }

    if !failures.is_empty() {
        let combined = failures.join("\n");
        panic!(
            "\nDoc-freshness check found {} stale reference(s):\n{}\n\n\
             Fix: edit the listed doc(s) so the language reflects current code, \
             OR (if the historical mention is legitimate) extend the StalePattern's \
             allowed_files list in tests/doc_freshness.rs with a comment explaining \
             why.\n",
            failures.len(), combined);
    }
}

#[test]
fn historical_plan_docs_carry_status_banner() {
    // docs/path2_unification_plan.md and docs/constructor_walk_detection.md
    // describe paradigms that are now implemented and (in path 2's case)
    // partially superseded. Each MUST carry a banner that tells a reader
    // "this is historical." Otherwise someone reads it cold and acts on
    // a plan that's already shipped.
    let banner_needle = "Status: implemented";
    for plan in &["docs/path2_unification_plan.md",
                  "docs/constructor_walk_detection.md"]
    {
        let path = repo_root().join(plan);
        let body = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue, // doc may have been intentionally removed
        };
        // Check the top ~15 lines so a banner at the end doesn't pass.
        let head: String = body.lines().take(20).collect::<Vec<_>>().join("\n");
        assert!(head.contains(banner_needle),
            "\n  {} is missing the {:?} banner near the top.\n\
             Implemented plan docs must announce themselves so readers \
             don't act on superseded designs.\n",
            plan, banner_needle);
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Compliance trip-wire: no Fraymakers/FrayTools engine-internal symbol names in
// the tracked docs.
//
// At Team Fray's request, the tracked docs must not name specific
// non-hscript engine classes / functions / fields, nor document the engine's
// internal symbol map (see AGENT_CONTEXT.md "engine-side knowledge is not in
// this repo", NOTICE.md, CONTRIBUTING.md). It is fine to document what Peptide
// does, the patcher architecture/workflow, and the public *hscript* scripting
// API (CState.JAB, match.getCharacters(), …) — so those are NOT needles here.
//
// Each needle is a distinctive engine/bundle symbol token with no legitimate
// place in a tracked doc. If one trips, the fix is to remove the engine-internal
// reference from the doc (and keep it in the gitignored docs/ scratch space),
// NOT to add it to an allow-list.
const ENGINE_SYMBOL_NEEDLES: &[&str] = &[
    "fraymakers.Main",          // the patched engine entry point (name the function nowhere)
    "MatchController",
    "PXFResource",
    "spawnPlayer",
    "cacheSpriteEntityData",
    "characterPxfContentMap",
    "ThreadTaskManager",
    "getPXFResource",
    "getPXFSpriteEntity",
    "set_DataAsPxf",
    "fetchThreaded",
    "poolHash",
    "importManifest",
    "calculateAbsolutePivotPosition", // FrayTools bundle internal
    "Tildebugger",
    "hxd.System",
];

/// Every tracked Markdown doc — the engine-symbol trip-wire scans all of them,
/// not just the four "stay-current" top-level docs above.
const TRACKED_DOCS: &[&str] = &[
    "CLAUDE.md",
    "README.md",
    "DEVELOPMENT.md",
    "AGENT_CONTEXT.md",
    "CONTRIBUTING.md",
    "NOTICE.md",
    "TESTING.md",
    "docs/PEPTIDE_GUIDE.md",
    "docs/PEPTIDE_DESIGN.md",
    "docs/STATUS.md",
];

#[test]
fn no_engine_internals_in_tracked_docs() {
    let mut failures: Vec<String> = Vec::new();

    for doc in TRACKED_DOCS {
        let path = repo_root().join(doc);
        let body = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue, // doc may not exist in every checkout
        };

        for needle in ENGINE_SYMBOL_NEEDLES {
            if !body.contains(needle) { continue; }
            let hits: Vec<(usize, &str)> = body.lines().enumerate()
                .filter(|(_, l)| l.contains(needle))
                .map(|(n, l)| (n + 1, l))
                .collect();
            let mut msg = format!(
                "\n  {} names engine internal {:?}", doc, needle);
            for (n, l) in hits {
                msg.push_str(&format!("\n      L{}: {}", n, l.trim()));
            }
            failures.push(msg);
        }
    }

    if !failures.is_empty() {
        panic!(
            "\nCompliance trip-wire: {} tracked doc(s) name Fraymakers/FrayTools \
             engine internals:\n{}\n\n\
             At Team Fray's request the tracked docs must not name specific \
             non-hscript engine classes/functions/fields or document the engine symbol \
             map. Remove the reference from the doc and keep it in the gitignored \
             docs/ scratch space (e.g. docs/ENGINE_INTERNALS.local.md). See \
             AGENT_CONTEXT.md \"engine-side knowledge is not in this repo\".\n",
            failures.len(), failures.join("\n"));
    }
}

fn _quiet(_: &Path) {} // keeps the `Path` import warning-free
