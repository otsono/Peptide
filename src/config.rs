//! Persisted user preferences, stored as JSON in the platform config dir
//! (Windows: %APPDATA%\ssf2-converter-gui\prefs.json; macOS:
//! ~/Library/Application Support/ssf2-converter-gui/prefs.json; Linux:
//! ~/.config/ssf2-converter-gui/prefs.json).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Prefs {
    /// User-picked FrayTools path (a .app bundle on macOS or the executable).
    pub fraytools_path: String,
    /// Output directory for converted characters.
    pub output_dir: String,
    /// Optional explicit misc.ssf path (empty = auto-detect next to input).
    pub misc_ssf: String,
    /// Auto-add the Fraymakers custom/<Char> folder to publish settings.
    pub fraymakers_auto_publish: bool,
    /// Whether the one-time Fraymakers prompt has been answered (Yes or
    /// Don't-ask-again); a plain "Not now" leaves this false so we re-ask.
    pub fraymakers_prompt_decided: bool,
}

fn prefs_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ssf2-converter-gui").join("prefs.json"))
}

impl Prefs {
    pub fn load() -> Prefs {
        if let Some(p) = prefs_path() {
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(prefs) = serde_json::from_str::<Prefs>(&text) {
                    return prefs;
                }
            }
        }
        Prefs::default()
    }

    pub fn save(&self) {
        if let Some(p) = prefs_path() {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(text) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(&p, text);
            }
        }
    }
}
