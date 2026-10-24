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
// Compliance trip-wire #2: no engine-internal symbol names (or RE'd engine
// code-line refs) in `.rs` COMMENTS, for either engine -- Fraymakers/FrayTools
// (HashLink) and SSF2 (the AVM2 engine RE'd from SSF2.swf).
//
// This is the comment-side companion to `doc_freshness.rs ::
// no_engine_internals_in_tracked_docs`. At Team Fray's request the Fraymakers
// symbol map and RE methodology stay out of the tracked repo; the SSF2 engine
// internals are kept out on the same footing (the SSF2 engine is McLeodGaming's,
// and its internals are not ours to document). That reaches into code comments:
// a comment must not name a non-hscript engine class / function / field, nor
// cite an engine code line (`name@12345`).
//
// SSF2 INPUT-SIDE symbols are fine -- the `.ssf`/SWF format, the SSF2 character
// modding API the converter reads, etc. The ban is on RUNNING-ENGINE internals
// (the live match/menu object graph), not the input the converter parses.
//
// Scope note (deliberately narrow): this is the ONLY doc rule that carries over
// to comments. The house voice / current-state-only / lowercase rules are for
// docs, not comments. And we only scan COMMENTS, never code -- the patcher
// legitimately names engine symbols in string literals to resolve them
// (`find_fn("spawnPlayer")`) and in user-facing diagnostics, which is fine.
//
// EXEMPT: the source that *is* the documented home of a symbol map. Fraymakers:
// the MANIFEST table (`src/manifest.rs`), the patch/dispatch shape (`connect_edit`
// in `src/main.rs`), and the RE tooling under `src/bin/`. SSF2: the AVM2 injector
// (`abc_inject.rs`), which is where the SSF2 engine symbols are resolved+patched.
// See AGENT_CONTEXT.md "engine-side knowledge is not in this repo" and
// CONTRIBUTING.md "special case".

/// True if `path` is the documented home of an engine symbol map (exempt).
fn is_symbol_map_home(path: &str) -> bool {
    let p = path.replace('\\', "/");
    p.ends_with("/src/main.rs")
        || p.ends_with("/src/manifest.rs")
        || p.contains("/src/bin/")
        || p.ends_with("/src/abc_inject.rs")
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


/// No content-name taxonomy comes back.
///
/// `hazard_kind` used to decide WHAT a stage object is by matching its symbol against words like
/// `thwomp` and `lava`, and then hand back stats invented for that word. It is gone: an object is
/// what its class declares and does what its own translated code does. This fails if anything of
/// the shape returns.
#[test]
fn content_name_keywords_only_shrink() {
    // every `l.contains("…")` inside the taxonomy, plus its cosmetic-word list
    const CEILING: usize = 0;
    let src = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/stage_parser.rs")
    ).expect("stage_parser.rs");
    let Some(start) = src.find("fn hazard_kind(") else {
        return; // gone entirely: the debt is paid, nothing to pin
    };
    let end = src[start..].find("\n}\n").map(|e| start + e).unwrap_or(src.len());
    let body = &src[start..end];
    let count = body.match_indices('"').count() / 2;
    assert!(
        count == CEILING,
        "hazard_kind now matches {count} content-name literals, and the ceiling is {CEILING}.\n\
         a keyword is never the fix: what an object IS comes from what its class extends and what \n\
         its own code does. read the source, do not add a word. see CLAUDE.md, first rule."
    );
}

/// A mapping's replacement text often carries a note, and that note is dropped into
/// the middle of emitted code. Later passes read that code by looking for the marks that
/// separate one piece of it from the next -- a `;` between statements, a `,` between the
/// fields of an object. A note carrying either ends a piece that has not ended: megaman's
/// sword effect lost every field after the one whose note held a comma. Notes say the
/// same thing without them.
#[test]
fn a_mapping_note_never_carries_a_separator() {
    let src = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/mappings/commands.jsonc")
    ).expect("commands.jsonc");
    let mut bad = Vec::new();
    for (n, line) in src.lines().enumerate() {
        // only the emitted text matters: `to` / `replacement` values, not the
        // jsonc `//` commentary around them
        let Some(v) = line.split_once("\"to\":").or_else(|| line.split_once("\"replacement\":"))
        else { continue };
        let mut rest = v.1;
        while let Some(open) = rest.find("/*") {
            let after = &rest[open + 2..];
            let Some(close) = after.find("*/") else { break };
            if after[..close].contains(';') || after[..close].contains(',') {
                bad.push(format!("  commands.jsonc:{}: {}", n + 1, line.trim()));
            }
            rest = &after[close + 2..];
        }
    }
    assert!(
        bad.is_empty(),
        "a note inside emitted code carries a `;` or a `,`, which ends a piece of code \
         that has not ended:\n{}\n\nsay it without them.",
        bad.join("\n")
    );
}


/// The compiler writes the source's own code; the runtime's code lives in `runtime/**/*.hx`. So no
/// string in the new converter's Rust defines a named Haxe function: behaviour a person would edit
/// never hides in a Rust string (see docs/CONVERTER_DESIGN.md).
#[test]
fn no_runtime_code_in_the_compilers_strings() {
    let root = repo_root().join("crates/ssf2-converter/src");
    let mut bad = Vec::new();
    for dir in ["model", "compile", "emit"] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap().flatten() {
            let p = entry.path();
            if p.extension().is_none_or(|x| x != "rs") { continue }
            let text = std::fs::read_to_string(&p).unwrap();
            for (i, line) in text.lines().enumerate() {
                let t = line.trim_start();
                if t.starts_with("//") { continue }
                if let Some(pos) = line.find("\"function ") {
                    let after = &line[pos + 10..];
                    if after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
                        bad.push(format!("{}:{}: {}", p.display(), i + 1, t));
                    }
                }
            }
        }
    }
    assert!(bad.is_empty(), "named Haxe functions in the compiler's strings:\n{}", bad.join("\n"));
}
