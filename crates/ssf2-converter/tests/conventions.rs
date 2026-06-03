//! Lightweight convention guards: turn "the doc said so" into "the test won't let
//! you". each enforces a rule we'd otherwise only catch in review.

use std::path::{Path, PathBuf};

/// CARGO_MANIFEST_DIR = <repo>/crates/ssf2-converter; go up two for the repo root.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

const TRACKED_DOCS: &[&str] = &[
    "CLAUDE.md",
    "README.md",
    "DEVELOPMENT.md",
    "AGENT_CONTEXT.md",
    "TESTING.md",
    "CONTRIBUTING.md",
    "NOTICE.md",
    "docs/PEPTIDE_GUIDE.md",
    "docs/PEPTIDE_DESIGN.md",
    "docs/STATUS.md",
];

/// Docs use `--`, never em-dashes (`—`). em-dashes inside ``` code fences are
/// literal and exempt. (matches the house writing style; see the user-voice rule.)
#[test]
fn no_em_dashes_in_doc_prose() {
    let mut bad = Vec::new();
    for doc in TRACKED_DOCS {
        let body = match std::fs::read_to_string(repo_root().join(doc)) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut in_fence = false;
        for (i, line) in body.lines().enumerate() {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if !in_fence && line.contains('—') {
                bad.push(format!("  {}:{}  {}", doc, i + 1, line.trim()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "\nem-dash(es) in doc prose -- use \"--\" instead:\n{}\n",
        bad.join("\n")
    );
}

/// "ONE command vocabulary, TWO engines" (AGENT_CONTEXT.md): a host-facing
/// feature must behave identically on Fraymakers and SSF2 via the `DebugTarget`
/// seam, never an `if engine == ...` branch in feature/command logic. the only
/// allowed per-engine branch is the transport shell (the GUI html picks which
/// socket to talk to), which this scan skips by only reading `.rs`.
#[test]
fn no_per_engine_branching_in_rust() {
    let mut hits = Vec::new();
    visit_rs(&repo_root().join("src"), &mut |path, body| {
        for (i, line) in body.lines().enumerate() {
            let l = line.trim();
            if l.starts_with("//") {
                continue;
            }
            if l.contains("== \"fraymakers\"")
                || l.contains("== \"ssf2\"")
                || l.contains(".engine() ==")
            {
                hits.push(format!("  {}:{}  {}", path, i + 1, l));
            }
        }
    });
    assert!(
        hits.is_empty(),
        "\nper-engine branch in feature logic. define it once through the DebugTarget \
         seam (a trait method + the helper in commands.hsx AND ssf2_target), not an \
         `if engine == ...`. see AGENT_CONTEXT.md \"ONE command vocabulary, TWO engines\":\n{}\n",
        hits.join("\n")
    );
}

fn visit_rs(dir: &Path, f: &mut impl FnMut(String, String)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            visit_rs(&p, f);
        } else if p.extension().is_some_and(|x| x == "rs") {
            if let Ok(body) = std::fs::read_to_string(&p) {
                f(p.display().to_string(), body);
            }
        }
    }
}
