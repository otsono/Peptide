//! Shared test helper: locate the SSF2 `.ssf` corpus.
//!
//! The corpus is the developer's own SSF2 files. it's never committed (it's
//! McLeodGaming's copyrighted content, and `.gitignore` excludes `*.ssf`), so
//! tests that need it check `.exists()` and skip cleanly when it's absent. that
//! keeps a fresh checkout's `cargo build` and `cargo test` green for anyone.
//!
//! Resolution order:
//!   1. `$SSF2_SSFS_DIR` if set -- point this at wherever you keep the corpus.
//!   2. otherwise the sibling `ssf2-ssfs/` of the repo root (the corpus lives
//!      next to the repo, not inside it).

#![allow(dead_code)]

use std::path::PathBuf;

/// Directory holding the `.ssf` corpus (see module docs for resolution order).
pub fn ssfs_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SSF2_SSFS_DIR") {
        return PathBuf::from(dir);
    }
    // CARGO_MANIFEST_DIR = <repo>/crates/ssf2-converter; the corpus is the repo
    // root's sibling, so go up three levels then into `ssf2-ssfs`.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".."))
        .join("ssf2-ssfs")
}

/// Path to one corpus file by character id (no extension), e.g. `ssf("sandbag")`.
pub fn ssf(name: &str) -> PathBuf {
    ssfs_dir().join(format!("{name}.ssf"))
}

/// `true` if `path` exists. When it doesn't, prints a one-line, self-documenting
/// skip note (so anyone running `cargo test` learns how to point at the corpus)
/// and returns `false`. Use it as the skip guard: `if !common::present(&p) { return; }`.
pub fn present(path: &std::path::Path) -> bool {
    if path.exists() {
        return true;
    }
    eprintln!(
        "skip: SSF2 corpus not found at {} -- set $SSF2_SSFS_DIR or place your \
         SSF2 .ssf files at ../ssf2-ssfs/ (they're not committed; bring your own).",
        path.display()
    );
    false
}
