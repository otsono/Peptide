//! Runtime-loaded conversion mapping tables.
//!
//! Mapping data lives in editable JSON files under `mappings/<category>/`.
//! Today only the `character` category exists; stages and other future
//! conversion targets get their own sibling directories (`mappings/stage/`,
//! ...). Each file is loaded at runtime so it can be tweaked without
//! recompiling. If a file is missing or malformed the built-in defaults
//! (embedded at compile time via `include_str!`) are used instead, so the
//! converter always has a valid table.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;

// ─── Embedded defaults ──────────────────────────────────────────────────────
// Guaranteed-valid fallback, also the source the on-disk files are copied from.

const DEFAULT_CHARACTER_ANIMATIONS: &str =
    include_str!("../mappings/character/animations.json");
const DEFAULT_CHARACTER_STATS: &str =
    include_str!("../mappings/character/stats.json");
// API command conversions are universal, not character-scoped, so this file
// lives at the top of mappings/ rather than under mappings/character/.
const DEFAULT_API_COMMANDS: &str =
    include_str!("../mappings/commands.json");

// ─── Schema ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct AnimationMappings {
    /// SSF2 xframe/animation name → Fraymakers animation slot.
    #[serde(default)]
    pub ssf2_to_fm: BTreeMap<String, String>,
    /// Sprite-symbol AnimLabel (lowercased, suffix stripped) → SSF2 animation name.
    #[serde(default)]
    pub label_to_ssf2: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct StatMultiplier {
    pub divisor: f64,
    pub target: f64,
    pub floor: f64,
}

impl StatMultiplier {
    /// Scale a raw SSF2 value into Fraymakers units.
    pub fn apply(&self, raw: f64) -> f64 {
        (raw / self.divisor * self.target).max(self.floor)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatMappings {
    /// Fraymakers stat field → ordered list of SSF2 key names to try.
    #[serde(default)]
    pub field_keys: BTreeMap<String, Vec<String>>,
    /// Named scaling factors applied to raw SSF2 stat values.
    #[serde(default)]
    pub multipliers: BTreeMap<String, StatMultiplier>,
}

impl StatMappings {
    /// SSF2 key names to try for a Fraymakers stat field, in priority order.
    pub fn keys_for(&self, field: &str) -> &[String] {
        self.field_keys.get(field).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Scale `raw` using the named multiplier. Falls back to the identity-with-floor
    /// behaviour (returning `raw`) if the multiplier is absent.
    pub fn scale(&self, name: &str, raw: f64) -> f64 {
        self.multipliers.get(name).map(|m| m.apply(raw)).unwrap_or(raw)
    }
}

/// One literal find -> replace pair in the API command translation table.
#[derive(Debug, Clone, Deserialize)]
pub struct Replacement {
    pub from: String,
    pub to: String,
}

/// A command parameter flagged as carrying a frame count, so the converter
/// doubles its value for the 30fps -> 60fps timing change.
#[derive(Debug, Clone, Deserialize)]
pub struct FrameParam {
    /// "call" — a positional argument of a function call; "field" — a named
    /// key in an object literal.
    pub kind: String,
    /// Function name (kind=call) or object-literal key (kind=field).
    pub name: String,
    /// kind=call only: 0-based index of the argument that is a frame count.
    #[serde(default)]
    pub arg: usize,
    /// Only entries with `isframe == true` are doubled.
    #[serde(default)]
    pub isframe: bool,
    /// Optional: a literal value at or above which the parameter is left
    /// unchanged (e.g. 255 for the hitStun/hitLag "no override" sentinel).
    #[serde(default)]
    pub sentinel: Option<i64>,
}

/// Universal SSF2 -> Fraymakers API command conversions: an ordered list of
/// literal string replacements applied to decompiled Haxe (order matters),
/// plus the per-parameter frame-count flags.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiCommands {
    #[serde(default)]
    pub replacements: Vec<Replacement>,
    #[serde(default)]
    pub frame_params: Vec<FrameParam>,
}

// ─── Loading ────────────────────────────────────────────────────────────────

/// Candidate locations for a mapping file, tried in order. The first that
/// exists and parses wins; this lets users drop an edited copy next to either
/// the working directory or the binary.
fn candidate_paths(rel: &str) -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from(rel)];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join(rel));
            // target/release/<bin> → repo root
            if let Some(up) = dir.parent().and_then(|p| p.parent()) {
                paths.push(up.join(rel));
            }
        }
    }
    paths
}

/// Load and parse a mapping file, preferring an on-disk copy and falling back
/// to the compiled-in default.
fn load<T: for<'de> Deserialize<'de>>(rel: &str, embedded: &str) -> T {
    for path in candidate_paths(rel) {
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<T>(&text) {
                Ok(parsed) => {
                    log::info!("Loaded mapping file {}", path.display());
                    return parsed;
                }
                Err(e) => {
                    log::warn!(
                        "Mapping file {} is malformed ({}); using built-in defaults",
                        path.display(), e
                    );
                    break;
                }
            },
            Err(_) => continue, // not at this location — keep looking
        }
    }
    serde_json::from_str(embedded)
        .expect("embedded default mapping JSON must be valid")
}

// ─── Cached accessors ───────────────────────────────────────────────────────

pub fn character_animations() -> &'static AnimationMappings {
    static CACHE: OnceLock<AnimationMappings> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/character/animations.json", DEFAULT_CHARACTER_ANIMATIONS)
    })
}

pub fn character_stats() -> &'static StatMappings {
    static CACHE: OnceLock<StatMappings> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/character/stats.json", DEFAULT_CHARACTER_STATS)
    })
}

pub fn api_commands() -> &'static ApiCommands {
    static CACHE: OnceLock<ApiCommands> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/commands.json", DEFAULT_API_COMMANDS)
    })
}
