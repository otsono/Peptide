//! Cross-platform host integration: locating the FrayTools executable, the
//! Fraymakers install, and editing a converted character's FrayTools publish
//! settings.
//!
//! All paths are computed per-OS (Windows / macOS / Linux). FrayTools stores
//! publish-folder paths with forward slashes (it's an Electron/Node app), so
//! the relative path we emit always uses `/` regardless of host.
//!
//! Ported from the old egui converter GUI's `platform.rs`. The sidecar-binary
//! and Node-harness helpers are gone: conversion is in-process now and the
//! FrayTools driver is pure-Rust CDP (`fraytools.rs`), so there is no
//! `ssf2_converter` binary and no `node` to locate.

// Some helpers (publish-folder editing, relative_path, find_project_file) are
// wired into the UI's convert/frayhook screens in Phase 5; allow them to sit
// unused until then rather than churn the warning surface.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

// ─── FrayTools executable ────────────────────────────────────────────────────────

/// A sensible default path to the FrayTools executable for this OS, if present.
/// The user can always override via the picker.
pub fn default_fraytools_exe() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let p = PathBuf::from("/Applications/FrayTools.app/Contents/MacOS/FrayTools");
        if p.is_file() {
            return Some(p);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(la) = std::env::var("LOCALAPPDATA") {
            let p = Path::new(&la).join("Programs").join("FrayTools").join("FrayTools.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// The user-facing FrayTools install path to display in Setup if one is found
/// on disk: the `.app` bundle on macOS (what the user picks), the executable on
/// Windows. Used to pre-fill + show a "detected" badge in the setup wizard.
pub fn detected_fraytools_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let app = PathBuf::from("/Applications/FrayTools.app");
        if app.exists() {
            return Some(app);
        }
    }
    default_fraytools_exe()
}

/// Resolve a user-picked FrayTools path to the actual executable. On macOS the
/// user may pick the `.app` bundle; resolve to `Contents/MacOS/<name>`.
/// On Windows/Linux they pick the executable directly.
pub fn resolve_fraytools_exe(picked: &str) -> String {
    if picked.ends_with(".app") {
        let app = Path::new(picked);
        let name = app.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "FrayTools".into());
        return app.join("Contents").join("MacOS").join(name).to_string_lossy().to_string();
    }
    picked.to_string()
}

// ─── Fraymakers install ────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn fraymakers_root_raw() -> Option<PathBuf> {
    // Per spec: %APPDATA%\Steam\steamapps\common\Fraymakers
    std::env::var("APPDATA").ok().map(|a| {
        Path::new(&a).join("Steam").join("steamapps").join("common").join("Fraymakers")
    })
}

#[cfg(target_os = "macos")]
fn fraymakers_root_raw() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library/Application Support/Steam/steamapps/common/Fraymakers")
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn fraymakers_root_raw() -> Option<PathBuf> {
    // Steam on Linux: ~/.steam/steam/... (also ~/.local/share/Steam/...).
    let home = dirs::home_dir()?;
    let a = home.join(".steam/steam/steamapps/common/Fraymakers");
    if a.is_dir() {
        return Some(a);
    }
    Some(home.join(".local/share/Steam/steamapps/common/Fraymakers"))
}

/// The Fraymakers Steam install dir, if it exists on this machine.
pub fn fraymakers_root() -> Option<PathBuf> {
    fraymakers_root_raw().filter(|d| d.is_dir())
}

/// The per-OS default Fraymakers install path, whether or not it exists yet.
/// Used by `Config::fraymakers_root` as the last-resort default (the harness
/// used to read this from `ui.rs::default_install_dir`).
pub fn default_fraymakers_root() -> Option<PathBuf> {
    fraymakers_root_raw()
}

/// `custom/<CharacterName>` under the Fraymakers install (not necessarily
/// existing yet).
pub fn fraymakers_custom_dir(char_name: &str) -> Option<PathBuf> {
    fraymakers_root().map(|r| r.join("custom").join(char_name))
}

// ─── Relative path (forward-slash, for FrayTools publishFolders) ─────────────────

/// POSIX-style (`/`-separated) relative path from `base` to `target`, both
/// absolute. FrayTools ignores absolute publishFolder paths and stores them
/// relative to the project dir, so this is what we must emit.
pub fn relative_path(base: &Path, target: &Path) -> String {
    let b: Vec<String> = base.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let t: Vec<String> = target.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let mut i = 0;
    while i < b.len() && i < t.len() && b[i] == t[i] {
        i += 1;
    }
    let mut comps: Vec<String> = std::iter::repeat_n("..".to_string(), b.len() - i).collect();
    comps.extend(t[i..].iter().cloned());
    if comps.is_empty() { ".".into() } else { comps.join("/") }
}

// ─── Publish-settings edit ────────────────────────────────────────────────────────

/// Create the Fraymakers `custom/<Char>` folder (if missing) and add it to the
/// character's `.fraytools` `publishFolders` as a relative path, unless an
/// entry already resolves to that folder. Idempotent and best-effort.
///
/// Returns the relative path that is present in publishFolders on success, or
/// an error string.
pub fn ensure_fraymakers_publish_folder(char_name: &str, char_output_path: &Path) -> Result<String, String> {
    let custom_dir = fraymakers_custom_dir(char_name)
        .ok_or_else(|| "Fraymakers install not found".to_string())?;

    std::fs::create_dir_all(&custom_dir)
        .map_err(|e| format!("create {}: {e}", custom_dir.display()))?;

    // Find the .fraytools project file in the character output dir.
    let proj = std::fs::read_dir(char_output_path)
        .map_err(|e| format!("read dir {}: {e}", char_output_path.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().map(|x| x == "fraytools").unwrap_or(false))
        .ok_or_else(|| format!("no .fraytools in {}", char_output_path.display()))?;

    let text = std::fs::read_to_string(&proj).map_err(|e| format!("read {}: {e}", proj.display()))?;
    let mut obj: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", proj.display()))?;

    let rel = relative_path(char_output_path, &custom_dir);

    let folders = obj
        .get_mut("publishFolders")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| "publishFolders missing or not an array".to_string())?;

    // Already present? (same relative string, or resolves to the same dir)
    let custom_canon = custom_dir.to_string_lossy().replace('\\', "/");
    let already = folders.iter().any(|e| {
        e.get("path").and_then(|p| p.as_str()).is_some_and(|p| {
            if p == rel {
                return true;
            }
            let resolved = char_output_path.join(p);
            // best-effort normalization
            resolved.to_string_lossy().replace('\\', "/").contains("custom")
                && custom_canon.ends_with(&format!("custom/{char_name}"))
                && resolved.file_name().map(|n| n == char_name).unwrap_or(false)
        })
    });
    if already {
        return Ok(rel);
    }

    folders.push(serde_json::json!({ "id": "fraymakers", "path": rel }));

    let out = serde_json::to_string_pretty(&obj).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&proj, out).map_err(|e| format!("write {}: {e}", proj.display()))?;
    Ok(rel)
}

/// Find the `.fraytools` project file inside a converted character directory.
pub fn find_project_file(char_output_path: &Path) -> Option<PathBuf> {
    std::fs::read_dir(char_output_path)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().map(|x| x == "fraytools").unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn relative_path_unix_matches_verified_fraymakers_layout() {
        // The exact case verified end-to-end on macOS: project at
        // /tmp/mario_ft/mario publishing into the Steam Fraymakers custom dir.
        let base = Path::new("/tmp/mario_ft/mario");
        let target = Path::new("/Users/example/Library/Application Support/Steam/steamapps/common/Fraymakers/custom/mario");
        assert_eq!(
            relative_path(base, target),
            "../../../Users/example/Library/Application Support/Steam/steamapps/common/Fraymakers/custom/mario"
        );
    }

    #[cfg(unix)]
    #[test]
    fn relative_path_sibling_and_identity() {
        assert_eq!(relative_path(Path::new("/a/b/proj"), Path::new("/a/b/proj/build")), "build");
        assert_eq!(relative_path(Path::new("/a/b/c"), Path::new("/a/b/c")), ".");
        assert_eq!(relative_path(Path::new("/a/b/c"), Path::new("/a/x/y")), "../../x/y");
    }
}
