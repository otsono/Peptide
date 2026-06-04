//! Lightweight convention guards: turn "the doc said so" into "the test won't let
//! you". each enforces a rule we'd otherwise only catch in review.

use std::path::{Path, PathBuf};

mod common;

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

// ───────────────────────────────────────────────────────────────────────────
// Compliance trip-wire #2: no Fraymakers/FrayTools engine-internal symbol names
// (or RE'd engine code-line refs) in `.rs` COMMENTS.
//
// This is the comment-side companion to `doc_freshness.rs ::
// no_engine_internals_in_tracked_docs`. At Team Fray's request the engine symbol
// map and the engine RE methodology stay out of the tracked repo. That reaches
// into code comments: a comment must not name a non-hscript engine class /
// function / field, nor cite an engine code line (`name@12345`).
//
// Scope note (deliberately narrow): this is the ONLY doc rule that carries over
// to comments. The house voice / current-state-only / lowercase rules are for
// docs, not comments. And we only scan COMMENTS, never code -- the patcher
// legitimately names engine symbols in string literals to resolve them
// (`find_fn("spawnPlayer")`) and in user-facing diagnostics, which is fine.
//
// EXEMPT: the source that *is* the documented home of the symbol map -- the
// MANIFEST table (`src/manifest.rs`), the patch/dispatch shape (`connect_edit`
// in `src/main.rs`), and the RE tooling under `src/bin/`. See AGENT_CONTEXT.md
// "engine-side knowledge is not in this repo" and CONTRIBUTING.md "special case".

/// True if `path` is the documented home of the engine symbol map (exempt).
fn is_symbol_map_home(path: &str) -> bool {
    let p = path.replace('\\', "/");
    p.ends_with("/src/main.rs")
        || p.ends_with("/src/manifest.rs")
        || p.contains("/src/bin/")
}

/// `name@12345` style engine code-line reference inside `text` (a comment).
/// Matches an `@` immediately followed by 3+ ASCII digits.
fn cites_engine_code_line(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'@' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j - (i + 1) >= 3 {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Reconstruct, line by line, only the characters that lived inside a `//` line
/// comment or `/* */` block comment. Code -- including string/char literals --
/// is dropped, so a help string or error diagnostic that names an engine symbol
/// is never flagged; only prose comments are. Handles `"..."`, raw strings
/// (`r#"..."#`), and char literals (distinguished from lifetimes).
fn comment_lines(body: &str) -> Vec<String> {
    let c: Vec<char> = body.chars().collect();
    let n = c.len();
    let mut out: Vec<String> = vec![String::new()];
    let mut line = 0usize;
    let mut i = 0usize;
    while i < n {
        let ch = c[i];
        if ch == '\n' {
            out.push(String::new());
            line += 1;
            i += 1;
            continue;
        }
        // line comment
        if ch == '/' && i + 1 < n && c[i + 1] == '/' {
            i += 2;
            while i < n && c[i] != '\n' {
                out[line].push(c[i]);
                i += 1;
            }
            continue;
        }
        // block comment
        if ch == '/' && i + 1 < n && c[i + 1] == '*' {
            i += 2;
            while i < n {
                if c[i] == '\n' {
                    out.push(String::new());
                    line += 1;
                    i += 1;
                    continue;
                }
                if c[i] == '*' && i + 1 < n && c[i + 1] == '/' {
                    i += 2;
                    break;
                }
                out[line].push(c[i]);
                i += 1;
            }
            continue;
        }
        // raw string r"..." / r#"..."# (also br"...")
        if ch == 'r' && i + 1 < n && (c[i + 1] == '"' || c[i + 1] == '#') {
            let mut j = i + 1;
            let mut hashes = 0;
            while j < n && c[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < n && c[j] == '"' {
                i = j + 1;
                loop {
                    if i >= n {
                        break;
                    }
                    if c[i] == '\n' {
                        out.push(String::new());
                        line += 1;
                        i += 1;
                        continue;
                    }
                    if c[i] == '"' {
                        let mut k = i + 1;
                        let mut h = 0;
                        while k < n && h < hashes && c[k] == '#' {
                            h += 1;
                            k += 1;
                        }
                        if h == hashes {
                            i = k;
                            break;
                        }
                    }
                    i += 1;
                }
                continue;
            }
        }
        // normal string "..."
        if ch == '"' {
            i += 1;
            while i < n {
                if c[i] == '\\' {
                    i += 2;
                    continue;
                }
                if c[i] == '\n' {
                    out.push(String::new());
                    line += 1;
                    i += 1;
                    continue;
                }
                if c[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // char literal '...' vs lifetime/label
        if ch == '\'' {
            let is_escape = i + 1 < n && c[i + 1] == '\\';
            let is_simple = i + 2 < n && c[i + 2] == '\'';
            if is_escape || is_simple {
                i += 1;
                while i < n {
                    if c[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if c[i] == '\'' {
                        i += 1;
                        break;
                    }
                    if c[i] == '\n' {
                        out.push(String::new());
                        line += 1;
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        i += 1;
    }
    out
}

#[test]
fn no_engine_internals_in_code_comments() {
    let mut hits = Vec::new();
    let mut scan = |path: String, body: String| {
        if is_symbol_map_home(&path) {
            return;
        }
        for (idx, comment) in comment_lines(&body).iter().enumerate() {
            if comment.is_empty() {
                continue;
            }
            let mut why: Vec<&str> = common::ENGINE_SYMBOL_NEEDLES
                .iter()
                .copied()
                .filter(|needle| comment.contains(needle))
                .collect();
            if cites_engine_code_line(comment) {
                why.push("engine code-line ref (name@NNNNN)");
            }
            if !why.is_empty() {
                hits.push(format!(
                    "  {}:{}  [{}]  {}",
                    path,
                    idx + 1,
                    why.join(", "),
                    comment.trim()
                ));
            }
        }
    };
    visit_rs(&repo_root().join("src"), &mut scan);
    visit_rs(&repo_root().join("crates/ssf2-converter/src"), &mut scan);

    assert!(
        hits.is_empty(),
        "\n{} code comment(s) name a Fraymakers/FrayTools engine internal or cite an \
         engine code line:\n{}\n\n\
         Describe the behavior, not the engine symbol. The symbol map lives only in the \
         patcher source (src/manifest.rs, connect_edit in src/main.rs, src/bin/ RE tools); \
         everywhere else, comment around it. See AGENT_CONTEXT.md \"engine-side knowledge is \
         not in this repo\" -> \"code comments, specifically\".\n",
        hits.len(),
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
