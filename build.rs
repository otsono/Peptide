//! Build script: stage Peptide's runtime data files into a `data/` folder next to
//! the compiled binary on every `cargo build`.
//!
//! Peptide reads its assets from disk at runtime (peptide_ui.html, commands.hsx,
//! match_settings.conf, mappings/) — they are deliberately NOT embedded in the
//! binary (so they stay editable and a missing one fails loudly instead of silently
//! shipping a stale baked-in copy; see `read_asset` / `missing_assets_report`).
//!
//! The cost of that is a bare binary, detached from the source tree, can't find them
//! (cross-compiled, pulled from CI, or copied out of the repo). So we copy them into
//! `<target>/<profile>/data/` here, which is exactly the `<exe-dir>/data/<rel>`
//! location `asset_candidate_paths` checks first. A plain `cargo build` then yields a
//! runnable binary + its `data/` sibling, with no separate packaging step. The
//! make-app.sh / make-win.sh / make-linux.sh packagers still stage `data/` for the
//! final bundle; this just makes the dev/CI output self-sufficient too.

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env_var("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env_var("OUT_DIR"));

    // OUT_DIR = <target>/<profile>/build/<pkg>-<hash>/out → the binary's dir (the
    // profile dir) is three levels up. Works for both native and --target builds.
    let Some(profile_dir) = out_dir.ancestors().nth(3) else {
        println!("cargo:warning=peptide build.rs: could not locate the target profile dir from OUT_DIR; skipping data staging");
        return;
    };
    let data = profile_dir.join("data");

    // (source path relative to the crate root, destination rel under data/)
    let files = [
        ("commands.hsx", "commands.hsx"),
        ("match_settings.conf", "match_settings.conf"),
        ("src/peptide_ui.html", "peptide_ui.html"),
    ];
    for (src_rel, dst_rel) in files {
        let src = manifest.join(src_rel);
        println!("cargo:rerun-if-changed={}", src.display());
        copy_file(&src, &data.join(dst_rel));
    }

    // The converter's editable mapping tables (recursive — mappings/ has subdirs).
    let mappings_src = manifest.join("crates/ssf2-converter/mappings");
    println!("cargo:rerun-if-changed={}", mappings_src.display());
    copy_tree(&mappings_src, &data.join("mappings"));
}

fn env_var(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("peptide build.rs: ${key} not set"))
}

/// Copy one file, creating parent dirs. A missing source is a warning, not a hard
/// error: the runtime still fails loudly (the dialog / panic) if an asset is absent,
/// and we don't want a stray missing file to break the whole build.
fn copy_file(src: &Path, dst: &Path) {
    if !src.is_file() {
        println!("cargo:warning=peptide build.rs: asset {} not found; not staged", src.display());
        return;
    }
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::copy(src, dst) {
        println!("cargo:warning=peptide build.rs: failed to stage {} → {}: {e}", src.display(), dst.display());
    }
}

/// Recursively copy a directory tree (used for `mappings/`). Re-emits rerun-if-changed
/// for every file so editing a mapping table restages on the next build.
fn copy_tree(src: &Path, dst: &Path) {
    if !src.is_dir() {
        println!("cargo:warning=peptide build.rs: mappings dir {} not found; not staged", src.display());
        return;
    }
    let _ = std::fs::create_dir_all(dst);
    let entries = match std::fs::read_dir(src) {
        Ok(e) => e,
        Err(e) => {
            println!("cargo:warning=peptide build.rs: cannot read {}: {e}", src.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &dest);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
            copy_file(&path, &dest);
        }
    }
}
