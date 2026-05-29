//! Cross-platform host integration: locating the converter binary, `node`,
//! the FrayTools executable, the Fraymakers install, and editing a converted
//! character's FrayTools publish settings.
//!
//! All paths are computed per-OS (Windows / macOS / Linux). FrayTools stores
//! publish-folder paths with forward slashes (it's an Electron/Node app), so
//! the relative path we emit always uses `/` regardless of host.

use std::path::{Path, PathBuf};

// ─── ssf2_converter sidecar binary ─────────────────────────────────────────────

/// The `ssf2_converter` CLI binary, expected next to this GUI executable
/// (they share the workspace `target/` dir in dev, and ship together).
pub fn converter_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = if cfg!(windows) { "ssf2_converter.exe" } else { "ssf2_converter" };
    let cand = dir.join(name);
    cand.is_file().then_some(cand)
}

// ─── node + the FrayTools export harness ────────────────────────────────────────

/// Locate the `node` executable. GUI apps don't always inherit a useful PATH,
/// so we check common install locations first, then walk PATH ourselves.
pub fn find_node() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "node.exe" } else { "node" };

    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        if let Ok(pf) = std::env::var("ProgramFiles") {
            candidates.push(Path::new(&pf).join("nodejs").join(exe));
        }
        if let Ok(la) = std::env::var("LOCALAPPDATA") {
            candidates.push(Path::new(&la).join("Programs").join("nodejs").join(exe));
        }
    } else {
        for p in ["/opt/homebrew/bin/node", "/usr/local/bin/node", "/usr/bin/node"] {
            candidates.push(PathBuf::from(p));
        }
    }
    for c in &candidates {
        if c.is_file() {
            return Some(c.clone());
        }
    }

    if let Ok(path) = std::env::var("PATH") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for dir in path.split(sep) {
            if dir.is_empty() {
                continue;
            }
            let cand = Path::new(dir).join(exe);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Locate `tools/fraytools-harness/export-in-fraytools.js` by walking up from
/// this executable (dev: it's in the repo) — returns None if not found.
pub fn find_export_script() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    for _ in 0..10 {
        let cand = dir.join("tools/fraytools-harness/export-in-fraytools.js");
        if cand.is_file() {
            return Some(cand);
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => break,
        }
    }
    None
}

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
    let mut comps: Vec<String> = std::iter::repeat("..".to_string()).take(b.len() - i).collect();
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
        e.get("path").and_then(|p| p.as_str()).map_or(false, |p| {
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
        let target = Path::new("/Users/jimmy/Library/Application Support/Steam/steamapps/common/Fraymakers/custom/mario");
        assert_eq!(
            relative_path(base, target),
            "../../../Users/jimmy/Library/Application Support/Steam/steamapps/common/Fraymakers/custom/mario"
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
