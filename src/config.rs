//! Persisted Peptide configuration + the resolver layer that the rest of the
//! binary reads through.
//!
//! Stored as JSON in the platform config dir:
//!   macOS:   ~/Library/Application Support/peptide/config.json
//!   Windows: %APPDATA%\peptide\config.json
//!   Linux:   ~/.config/peptide/config.json
//!
//! Peptide had no persisted config before — everything was env vars
//! (`FRAY_DIR`, `FRAY_CHAR`, …) plus per-OS defaults. Those env vars still win
//! (back-compat + scripting), then the persisted config, then a built-in default.
//! The setup screen writes this file; the engine harness reads it via the
//! resolver methods so there is one source of truth.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::platform;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    // ── Setup-screen fields ──────────────────────────────────────────────
    /// Fraymakers Steam install dir (was the `FRAY_DIR` env var only).
    pub fraymakers_root: String,
    /// FrayTools path — a `.app` bundle on macOS or the executable elsewhere.
    pub fraytools_path: String,
    /// The active project, tracked by character name. Output dir + `.fraytools`
    /// path are derived from it.
    pub current_char: String,
    /// Output directory for converted characters (per-char output = `<output>/<char>`).
    pub output_dir: String,

    // ── Converter knobs (carried from the old egui prefs) ────────────────
    /// Optional explicit `misc.ssf` path (empty = auto-detect next to input).
    pub misc_ssf: String,
    /// Auto-add the Fraymakers `custom/<Char>` folder to publish settings.
    pub fraymakers_auto_publish: bool,
    /// Whether the one-time Fraymakers publish prompt has been answered.
    pub fraymakers_prompt_decided: bool,

    // ── Engine-launch knobs (were env-only: FRAY_STAGE/ASSIST/BOOT) ───────
    pub stage: String,
    pub assist: String,
    pub boot_name: String,
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("peptide").join("config.json"))
}

impl Config {
    pub fn load() -> Config {
        if let Some(p) = config_path() {
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(cfg) = serde_json::from_str::<Config>(&text) {
                    return cfg;
                }
            }
        }
        Config::default()
    }

    pub fn save(&self) {
        if let Some(p) = config_path() {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(text) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(&p, text);
            }
        }
    }

    /// Absolute path to the persisted config file (for diagnostics / the UI).
    #[allow(dead_code)] // surfaced in the Setup screen (Phase 5)
    pub fn path() -> Option<PathBuf> {
        config_path()
    }

    // ── Resolvers (env override → config → default) ──────────────────────
    // The harness (ui.rs / gui.rs) reads through these so env vars keep working
    // for back-compat and scripting, persisted config fills the gap, and a
    // built-in default is the last resort.

    /// Fraymakers install dir. `FRAY_DIR` env → config → per-OS Steam default.
    pub fn fraymakers_root(&self) -> Option<PathBuf> {
        if let Some(d) = std::env::var_os("FRAY_DIR") {
            return Some(PathBuf::from(d));
        }
        if !self.fraymakers_root.is_empty() {
            return Some(PathBuf::from(&self.fraymakers_root));
        }
        platform::default_fraymakers_root()
    }

    /// Resolved FrayTools executable. config (`.app` resolved) → per-OS default.
    pub fn fraytools_exe(&self) -> Option<PathBuf> {
        if !self.fraytools_path.is_empty() {
            return Some(PathBuf::from(platform::resolve_fraytools_exe(&self.fraytools_path)));
        }
        platform::default_fraytools_exe()
    }

    /// Active character. `FRAY_CHAR` env → config → "sandbag".
    pub fn char_name(&self) -> String {
        if let Ok(c) = std::env::var("FRAY_CHAR") {
            if !c.is_empty() { return c; }
        }
        if !self.current_char.is_empty() { return self.current_char.clone(); }
        "sandbag".to_string()
    }

    /// Stage id. `FRAY_STAGE` env → config → "thespire".
    pub fn stage(&self) -> String {
        if let Ok(s) = std::env::var("FRAY_STAGE") {
            if !s.is_empty() { return s; }
        }
        if !self.stage.is_empty() { return self.stage.clone(); }
        "thespire".to_string()
    }

    /// Assist id. `FRAY_ASSIST` env → config → "commandervideoassist".
    pub fn assist(&self) -> String {
        if let Ok(a) = std::env::var("FRAY_ASSIST") {
            if !a.is_empty() { return a; }
        }
        if !self.assist.is_empty() { return self.assist.clone(); }
        "commandervideoassist".to_string()
    }

    /// Boot bytecode filename. `FRAY_BOOT` env → config → "hlboot-sdl.dat".
    pub fn boot_name(&self) -> String {
        if let Ok(b) = std::env::var("FRAY_BOOT") {
            if !b.is_empty() { return b; }
        }
        if !self.boot_name.is_empty() { return self.boot_name.clone(); }
        "hlboot-sdl.dat".to_string()
    }

    /// Output directory for conversions. config → "./characters".
    pub fn output_dir(&self) -> PathBuf {
        if !self.output_dir.is_empty() {
            return PathBuf::from(&self.output_dir);
        }
        PathBuf::from("./characters")
    }

    /// True when every required setup field resolves to something usable:
    /// Fraymakers root exists, FrayTools resolves to an existing exe, and a
    /// current character is set.
    pub fn setup_complete(&self) -> bool {
        let fray_ok = self.fraymakers_root().map(|p| p.is_dir()).unwrap_or(false);
        let ft_ok = self.fraytools_exe().map(|p| p.is_file()).unwrap_or(false);
        let char_ok = !self.char_name().is_empty();
        fray_ok && ft_ok && char_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let cfg = Config {
            fraymakers_root: "/games/Fraymakers".into(),
            fraytools_path: "/Applications/FrayTools.app".into(),
            current_char: "mario".into(),
            output_dir: "/work/out".into(),
            misc_ssf: "/ssfs/misc.ssf".into(),
            fraymakers_auto_publish: true,
            fraymakers_prompt_decided: true,
            stage: "battlefield".into(),
            assist: "someassist".into(),
            boot_name: "hlboot-sdl.dat".into(),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.current_char, "mario");
        assert_eq!(back.fraytools_path, "/Applications/FrayTools.app");
        assert!(back.fraymakers_auto_publish);
        assert_eq!(back.stage, "battlefield");
    }

    #[test]
    fn missing_fields_default_and_are_lenient() {
        // A partial config (old/forward-compat) must load, not error.
        let back: Config = serde_json::from_str(r#"{"current_char":"fox"}"#).unwrap();
        assert_eq!(back.current_char, "fox");
        assert_eq!(back.fraymakers_root, "");
        assert!(!back.fraymakers_auto_publish);
    }

    #[test]
    fn output_dir_falls_back_to_characters() {
        let cfg = Config::default();
        assert_eq!(cfg.output_dir(), PathBuf::from("./characters"));
        let cfg = Config { output_dir: "/x/y".into(), ..Default::default() };
        assert_eq!(cfg.output_dir(), PathBuf::from("/x/y"));
    }

    #[test]
    fn setup_incomplete_with_bogus_paths() {
        // Nonexistent paths → setup not complete (independent of any env vars,
        // since explicit non-empty fields take precedence over env/defaults...
        // except FRAY_DIR/FRAY_CHAR env could still satisfy those two. We assert
        // the FrayTools leg, which has no env override, fails on a bogus path.
        let cfg = Config {
            fraytools_path: "/no/such/fraytools/binary".into(),
            ..Default::default()
        };
        assert!(!cfg.fraytools_exe().map(|p| p.is_file()).unwrap_or(false));
        assert!(!cfg.setup_complete());
    }
}
