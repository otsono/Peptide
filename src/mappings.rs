//! Runtime-loaded conversion mapping tables.
//!
//! Mapping data lives in editable JSONC files under `mappings/<category>/`
//! (JSONC = JSON with `//` comments and trailing commas, so entries can be
//! annotated). Today only the `character` category exists; stages and other
//! future conversion targets get their own sibling directories. Each file is
//! loaded at runtime so it — including derivation formulas and comments — can
//! be tweaked without recompiling. If a file is missing or malformed the
//! built-in defaults (embedded at compile time via `include_str!`) are used
//! instead, so the converter always has a valid table.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;

// ─── Embedded defaults ──────────────────────────────────────────────────────
// Guaranteed-valid fallback, also the source the on-disk files are copied from.

const DEFAULT_CHARACTER_ANIMATIONS: &str =
    include_str!("../mappings/character/animations.jsonc");
const DEFAULT_CHARACTER_STATS: &str =
    include_str!("../mappings/character/stats.jsonc");
const DEFAULT_CHARACTER_HITBOX_STATS: &str =
    include_str!("../mappings/character/hitbox_stats.jsonc");
const DEFAULT_CHARACTER_ANIMATION_TEMPLATE: &str =
    include_str!("../mappings/character/animation_template.jsonc");
const DEFAULT_CHARACTER_STATS_TABLES: &str =
    include_str!("../mappings/character/stats_tables.jsonc");
const DEFAULT_PROJECTILE_TABLES: &str =
    include_str!("../mappings/projectile_tables.jsonc");
// API command conversions are universal, not character-scoped, so this file
// lives at the top of mappings/ rather than under mappings/character/.
const DEFAULT_API_COMMANDS: &str =
    include_str!("../mappings/commands.jsonc");

// Per-entity Script.hx template files (TOML). The Haxe lives in `'''…'''`
// literal blocks, byte-for-byte; placeholders are `{{slot}}`.
const DEFAULT_GLOBAL_HELPERS: &str     = include_str!("../mappings/global_helpers.toml");
const DEFAULT_CHARACTER_HELPERS: &str  = include_str!("../mappings/character_helpers.toml");
const DEFAULT_PROJECTILE_HELPERS: &str = include_str!("../mappings/projectile_helpers.toml");
const DEFAULT_STAGE_HELPERS: &str      = include_str!("../mappings/stage_helpers.toml");
const DEFAULT_ITEM_HELPERS: &str       = include_str!("../mappings/item_helpers.toml");

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
    /// Integer offsets added to a stat after extraction (e.g. max_jumps +1).
    #[serde(default)]
    pub offsets: BTreeMap<String, i64>,
    /// Stats computed from other already-converted stats, as expression
    /// strings (e.g. `"max(air_mobility_raw, aerial_friction) * 5.0"`).
    /// Each is compiled once at load time — see `evaluate_stat_derivation`.
    #[serde(default)]
    pub derivations: BTreeMap<String, String>,
    /// Flat default values emitted verbatim into CharacterStats.hx (numbers,
    /// strings or arrays — kept as raw JSON so each is written Haxe-literally).
    #[serde(default)]
    pub constants: BTreeMap<String, serde_json::Value>,
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

    /// Integer offset for a stat (0 if none configured).
    pub fn offset(&self, name: &str) -> i64 {
        self.offsets.get(name).copied().unwrap_or(0)
    }

    /// A flat constant rendered as a Haxe literal (bare number, quoted string,
    /// `[]` array). Returns a visible `/*MISSING*/` marker if unconfigured.
    pub fn constant(&self, name: &str) -> String {
        self.constants.get(name)
            .map(|v| v.to_string())
            .unwrap_or_else(|| format!("0 /*MISSING:{}*/", name))
    }
}

/// One Fraymakers hitbox field and the SSF2 key(s) it is converted from.
#[derive(Debug, Clone, Deserialize)]
pub struct HitboxField {
    /// Fraymakers HitboxStats field name.
    pub fm_field: String,
    /// SSF2 source key(s); the converter takes the max of their values.
    #[serde(default)]
    pub ssf2_keys: Vec<String>,
    /// If true, the value is a frame count and is doubled for 30fps -> 60fps.
    #[serde(default)]
    pub isframe: bool,
}

/// SSF2 -> Fraymakers hitbox-stats conversion (character-scoped).
#[derive(Debug, Clone, Deserialize)]
pub struct HitboxStatsMapping {
    #[serde(default)]
    pub fields: Vec<HitboxField>,
}

impl HitboxStatsMapping {
    /// SSF2 source keys for a Fraymakers hitbox field (empty if unmapped).
    pub fn keys_for(&self, fm_field: &str) -> &[String] {
        self.fields.iter()
            .find(|f| f.fm_field == fm_field)
            .map(|f| f.ssf2_keys.as_slice())
            .unwrap_or(&[])
    }

    /// Whether a Fraymakers hitbox field is a frame count (doubled for 60fps).
    pub fn is_frame(&self, fm_field: &str) -> bool {
        self.fields.iter()
            .find(|f| f.fm_field == fm_field)
            .map(|f| f.isframe)
            .unwrap_or(false)
    }
}

/// One literal find -> replace pair in the API command translation table.
#[derive(Debug, Clone, Deserialize)]
pub struct Replacement {
    pub from: String,
    pub to: String,
}

/// A regex-based replacement, for cases the literal table can't express:
/// dropping/rewriting arguments, dispatching on argument shape, etc.
/// `pattern` is a Rust `regex` crate pattern; `replacement` follows the
/// standard `$1` / `${name}` capture-substitution syntax.
#[derive(Debug, Clone, Deserialize)]
pub struct RegexReplacement {
    pub pattern: String,
    pub replacement: String,
    /// Human-readable label used in error messages if the pattern fails to
    /// compile. Optional.
    #[serde(default)]
    pub note: String,
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

/// A named API call carried by a passthrough or ssf2_only entry, with an
/// optional human-readable note. The note isn't consumed by the converter.
#[derive(Debug, Clone, Deserialize)]
pub struct NamedApi {
    pub name: String,
    #[serde(default)]
    pub note: String,
}

/// Per-source-method routing table for SSF2 "umbrella" calls that need
/// to be split into multiple FM calls. Map key (in `ApiCommands`) is the
/// SSF2 method name (e.g. `"updateAttackStats"`); each entry below is the
/// mapping that drives the split.
///
/// `fields` maps SSF2 field name → either a simple route string
/// (`"<target_method>.<fm_field_name>"`) or an object form with
/// value-conditional logic — see `CallSplitFieldMapping`. Fields sharing
/// a target method get GROUPED into one combined call; fields with no
/// entry become `// TODO:` comments.
#[derive(Debug, Clone, Deserialize)]
pub struct CallSplit {
    pub fields: std::collections::BTreeMap<String, CallSplitFieldMapping>,
}

/// Per-field routing entry. Most fields are simple `target.fm_field`
/// renames — those use the `Simple` variant. The `Detailed` variant
/// supports value-conditional rewriting (e.g. only emit when the value
/// is `true`) and skip rules (drop the field entirely when the value
/// matches a sentinel like `-1`).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum CallSplitFieldMapping {
    /// `"<target_method>.<fm_field_name>"` — source value passes through.
    Simple(String),
    /// Object form with optional value_map / skip_if_value / todo.
    Detailed {
        /// `"<target_method>.<fm_field_name>"` destination. Optional when
        /// the entry exists solely to express a skip rule.
        #[serde(default)]
        target: Option<String>,
        /// If non-empty: only emit when the source value (trimmed) is one
        /// of these keys; the emitted value is the mapped string. Values
        /// not in the map fall through to TODO.
        #[serde(default)]
        value_map: std::collections::BTreeMap<String, String>,
        /// If the source value (trimmed) equals this, drop the field
        /// silently — no emission, no TODO.
        #[serde(default)]
        skip_if_value: Option<String>,
        /// Extra note appended to the inline TODO comment that's emitted
        /// alongside every successful routing from this entry (and to the
        /// TODO line emitted when the value doesn't match value_map).
        #[serde(default)]
        todo: Option<String>,
    },
}

/// Per-prop mapping for SSF2 `self.attachEffect("name", { props })` calls.
/// Drives the inline translation that injects translated fields into
/// `new VfxStats({…})` and emits `// TODO:` lines for props with no
/// clean FM equivalent. See `attach_effect_props` in commands.jsonc.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AttachEffectPropMapping {
    /// `"fmPropName"` — direct rename; source value passes through.
    Simple(String),
    /// Object form for non-trivial mappings.
    Detailed {
        /// FM prop name; source value passes through. Same as the
        /// `Simple` variant but in object form for readability when
        /// combined with a `todo` note.
        #[serde(default)]
        target: Option<String>,
        /// Expand a single SSF2 prop into multiple FM props that all
        /// receive the same source value. Used for `parentLock` →
        /// `relativeWith` + `resizeWith` + `flipWith`.
        #[serde(default)]
        expand_to: Vec<String>,
        /// If set, the prop has no FM equivalent — emit a `// TODO:`
        /// line above the call carrying this note and the source value.
        #[serde(default)]
        todo: Option<String>,
    },
}

/// Universal SSF2 -> Fraymakers API command conversions.
///
/// Every section here is consumed by the converter — there are no
/// documentation-only sections.
///   - `replacements`        — ordered literal find→replace pairs (order matters)
///   - `regex_replacements`  — regex-based renames, applied AFTER the literal
///                              pass; used for arg-dropping / arg-aware cases
///   - `call_splits`         — SSF2 umbrella calls (e.g. updateAttackStats)
///                              that need their object-literal args split
///                              and routed to multiple FM target methods
///   - `frame_params`        — per-parameter frame-count flags for 30→60fps
///   - `passthrough_fm_apis` — calls that ARE valid Fraymakers API; left
///                              untouched and treated as known calls
///   - `ssf2_only`           — calls with no Fraymakers equivalent; replaced
///                              by `// [SSF2-only: NAME] …` markers
#[derive(Debug, Clone, Deserialize)]
pub struct ApiCommands {
    #[serde(default)]
    pub replacements: Vec<Replacement>,
    #[serde(default)]
    pub regex_replacements: Vec<RegexReplacement>,
    #[serde(default)]
    pub call_splits: std::collections::BTreeMap<String, CallSplit>,
    #[serde(default)]
    pub attach_effect_props: std::collections::BTreeMap<String, AttachEffectPropMapping>,
    #[serde(default)]
    pub global_vfx_map: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub global_sound_map: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub frame_params: Vec<FrameParam>,
    #[serde(default)]
    pub passthrough_fm_apis: Vec<NamedApi>,
    #[serde(default)]
    pub ssf2_only: Vec<NamedApi>,
}

// ─── Script.hx output templates ──────────────────────────────────────────────
//
// The Haxe strings the codegen emits into Script.hx (and projectile Script.hx)
// live here, not hardcoded in Rust, so their shape can change without
// recompiling — same philosophy as `global_sound_map` etc. Grouped by feature.
// Each value is a string filled by `str::replace`-ing `{{slot}}` placeholders;
// the control flow / data assembly stays in Rust. Empty by default —
// `require_template` panics with the qualified key if a needed one is missing.

/// playAttackSound / playVoiceSound helper block (api_mappings::generate_sound_helpers).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AudioTemplates {
    pub header_comment: String,
    pub attack_sounds_decl: String,
    pub voice_sounds_decl: String,
    pub active_voice_clip_decl: String,
    pub play_attack_sound_fn: String,
    pub play_voice_sound_fn: String,
    pub play_resolved_sound_resolver: String,
    pub global_sound_case: String,
    pub voice_teardown_cleanup: String,
    pub placeholder_array_entry_todo: String,
}

/// Per-frame playSound("id") rewrite (api_mappings::build_sound_call).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PlaySoundTemplates {
    pub global: String,
    pub asset: String,
    pub placeholder_no_static_id: String,
    pub placeholder_unmapped: String,
}

/// Jab chain helpers (haxe_gen::generate_jab_scripts).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct JabTemplates {
    pub chain_helpers: String,
}

/// Character Script.hx framework scaffolding (haxe_gen::generate_script).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FrameworkTemplates {
    pub header: String,
    pub instance_vars_comment: String,
    pub ext_var_decl: String,
    pub ext_var_init_assign: String,
    pub general_functions_begin: String,
    pub general_functions_end: String,
    pub decompiled_ext_header: String,
    pub link_frames_listener: String,
    pub fn_close: String,
    pub initialize_header: String,
    pub initialize_sig: String,
    pub input_update_hook_header: String,
    pub input_update_hook_sig: String,
    pub handle_link_frames_header: String,
    pub handle_link_frames_sig: String,
    pub update_sig: String,
    pub onteardown_sig: String,
}

/// Projectile Script.hx generator (haxe_gen::generate_projectile_script).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProjectileTemplates {
    pub single_state: String,
    pub multi_state: String,
    pub lstate_idle_prep: String,
    pub lstate_prep_line: String,
    pub update_branch: String,
}

/// Fetch a Script.hx template, panicking with the qualified key if it's empty.
/// Option A fail-fast: templates live only in commands.jsonc, so an empty
/// value means the section/key is missing or blank — fail loudly rather than
/// emit broken Haxe. Genuinely-empty pieces (e.g. the `update`/`onTeardown`
/// header comments) are NOT routed through this — they stay as `""` literals.
pub fn require_template<'a>(path: &str, val: &'a str) -> &'a str {
    if val.is_empty() {
        // Point the modder at the right file: the path's first segment names the
        // entity-scope TOML; legacy (still-in-JSONC) paths fall back to commands.jsonc.
        let file = match path.split('.').next() {
            Some("global")     => "global_helpers.toml",
            Some("character")  => "character_helpers.toml",
            Some("projectile") => "projectile_helpers.toml",
            Some("stage")      => "stage_helpers.toml",
            Some("item")       => "item_helpers.toml",
            _                  => "commands.jsonc",
        };
        panic!("script_templates.{path} is empty — check {file}");
    }
    val
}

// ─── Per-entity Script.hx template files (TOML) ──────────────────────────────
//
// The Haxe the codegen emits lives in `mappings/<scope>_helpers.toml`, grouped
// by which entity type it applies to (not by codegen function). Each value is a
// `'''…'''` literal block (or single-quoted one-liner) filled by `str::replace`-
// ing `{{slot}}` placeholders. Data tables that DRIVE codegen (animation lists,
// field-name arrays, physics maps) stay in Rust — only Haxe scaffolding moves.

/// Character *Stats.hx generators (HitboxStats / CharacterStats / AnimationStats).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CharStatsTemplates {
    // HitboxStats.hx
    pub hitbox_header: String,
    pub hitbox_section_comment: String,
    pub hitbox_attack_open: String,
    pub hitbox_field_main: String,
    pub hitbox_field_hitstun: String,
    pub hitbox_field_close: String,
    pub hitbox_attack_close: String,
    pub hitbox_emote: String,
    pub hitbox_todo_attack: String,
    pub hitbox_extras_header: String,
    pub hitbox_footer: String,
    // CharacterStats.hx
    pub char_stats_banner: String,
    pub char_stats_main: String,
    pub char_stats_section_comment: String,
    pub char_stats_field_anno: String,
    pub char_stats_field_plain: String,
    pub char_stats_footer: String,
    // AnimationStats.hx
    pub anim_stats_header: String,
    pub anim_stats_entry: String,
    pub anim_stats_split_header: String,
    pub anim_stats_footer: String,
}

/// Projectile *Stats.hx generators (AnimationStats / Stats / HitboxStats).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProjStatsTemplates {
    pub proj_anim_wrapper: String,
    pub proj_anim_entry_destroy: String,
    pub proj_anim_entry_normal: String,
    pub proj_stats_body: String,
    pub proj_stats_physics_line: String,
    pub proj_stats_todo_line: String,
    pub proj_stats_source_matched: String,
    pub proj_stats_source_default: String,
    pub proj_hitbox_body: String,
    pub proj_hitbox_active_block: String,
    pub proj_hitbox_empty_block: String,
    pub proj_hitbox_idle_matched: String,
    pub proj_hitbox_idle_default: String,
    pub proj_hitbox_source_matched: String,
    pub proj_hitbox_source_default: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GlobalHelpers {
    pub playsound: PlaySoundTemplates,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CharacterHelpers {
    pub audio: AudioTemplates,
    pub jab: JabTemplates,
    pub framework: FrameworkTemplates,
    pub stats: CharStatsTemplates,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProjectileHelpers {
    pub script: ProjectileTemplates,
    pub stats: ProjStatsTemplates,
}

/// Placeholder scopes — no templates yet (TODO files).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct StageHelpers {}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ItemHelpers {}

/// Aggregate of the 5 per-entity template files.
#[derive(Debug, Clone, Default)]
pub struct HelperTemplates {
    pub global: GlobalHelpers,
    pub character: CharacterHelpers,
    pub projectile: ProjectileHelpers,
    pub stage: StageHelpers,
    pub item: ItemHelpers,
}

/// Like `load`, but for TOML template files (no comment-stripping needed —
/// TOML has native comments; literal `'''` blocks preserve the Haxe verbatim).
fn load_toml<T: for<'de> Deserialize<'de>>(rel: &str, embedded: &str) -> T {
    for path in candidate_paths(rel) {
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<T>(&text) {
                Ok(parsed) => {
                    log::info!("Loaded template file {}", path.display());
                    return parsed;
                }
                Err(e) => {
                    log::warn!(
                        "Template file {} is malformed ({}); using built-in defaults",
                        path.display(), e
                    );
                    break;
                }
            },
            Err(_) => continue,
        }
    }
    toml::from_str(embedded).expect("embedded default template TOML must be valid")
}

/// The per-entity Script.hx templates, loaded once from the 5 TOML files.
pub fn script_templates() -> &'static HelperTemplates {
    static CACHE: OnceLock<HelperTemplates> = OnceLock::new();
    CACHE.get_or_init(|| HelperTemplates {
        global:     load_toml("mappings/global_helpers.toml",     DEFAULT_GLOBAL_HELPERS),
        character:  load_toml("mappings/character_helpers.toml",  DEFAULT_CHARACTER_HELPERS),
        projectile: load_toml("mappings/projectile_helpers.toml", DEFAULT_PROJECTILE_HELPERS),
        stage:      load_toml("mappings/stage_helpers.toml",      DEFAULT_STAGE_HELPERS),
        item:       load_toml("mappings/item_helpers.toml",       DEFAULT_ITEM_HELPERS),
    })
}

// ─── Loading ────────────────────────────────────────────────────────────────

/// Strip JSONC down to plain JSON: remove `//` line and `/* */` block
/// comments and drop trailing commas, so `serde_json` can parse it.
/// String-aware — comment markers and commas inside string literals are
/// preserved. `serde_json` is used (not a JSON5 crate) because it keeps the
/// integer/float distinction the `constants` section relies on.
fn strip_jsonc(src: &str) -> String {
    // Pass 1 — remove comments.
    let mut decommented = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            decommented.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() { decommented.push(n); }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => { in_string = true; decommented.push(c); }
            '/' if chars.peek() == Some(&'/') => {
                for n in chars.by_ref() {
                    if n == '\n' { decommented.push('\n'); break; }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' { break; }
                    prev = n;
                }
            }
            _ => decommented.push(c),
        }
    }
    // Pass 2 — drop trailing commas (a `,` whose next non-whitespace char
    // is `}` or `]`).
    let glyphs: Vec<char> = decommented.chars().collect();
    let mut out = String::with_capacity(glyphs.len());
    let mut in_string = false;
    let mut i = 0;
    while i < glyphs.len() {
        let c = glyphs[i];
        if in_string {
            out.push(c);
            if c == '\\' {
                if i + 1 < glyphs.len() { out.push(glyphs[i + 1]); i += 2; continue; }
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == '"' { in_string = true; out.push(c); i += 1; continue; }
        if c == ',' {
            let mut j = i + 1;
            while j < glyphs.len() && glyphs[j].is_whitespace() { j += 1; }
            if j < glyphs.len() && (glyphs[j] == '}' || glyphs[j] == ']') {
                i += 1; // skip the trailing comma
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

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

/// Load and parse a JSONC mapping file, preferring an on-disk copy and
/// falling back to the compiled-in default.
fn load<T: for<'de> Deserialize<'de>>(rel: &str, embedded: &str) -> T {
    for path in candidate_paths(rel) {
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<T>(&strip_jsonc(&text)) {
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
    serde_json::from_str(&strip_jsonc(embedded))
        .expect("embedded default mapping JSONC must be valid")
}

// ─── Cached accessors ───────────────────────────────────────────────────────

pub fn character_animations() -> &'static AnimationMappings {
    static CACHE: OnceLock<AnimationMappings> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/character/animations.jsonc", DEFAULT_CHARACTER_ANIMATIONS)
    })
}

pub fn character_stats() -> &'static StatMappings {
    static CACHE: OnceLock<StatMappings> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/character/stats.jsonc", DEFAULT_CHARACTER_STATS)
    })
}

/// One Fraymakers character-template animation + its AnimationStats.hx property
/// overrides. `props` is the object-literal body (empty = `{}`). Ordered list
/// loaded from `mappings/character/animation_template.jsonc`.
#[derive(Debug, Clone, Deserialize)]
pub struct AnimTemplateEntry {
    pub name: String,
    #[serde(default)]
    pub props: String,
}

pub fn character_animation_template() -> &'static Vec<AnimTemplateEntry> {
    static CACHE: OnceLock<Vec<AnimTemplateEntry>> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/character/animation_template.jsonc", DEFAULT_CHARACTER_ANIMATION_TEMPLATE)
    })
}

// ─── Stats-codegen data tables (moved out of Rust) ───────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct HitboxSection { pub section: String, #[serde(default)] pub moves: Vec<String> }

#[derive(Debug, Clone, Deserialize)]
pub struct FieldAnno { pub field: String, #[serde(default)] pub anno: String }

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct StatsFields {
    pub ecb: Vec<FieldAnno>,
    pub camera: Vec<String>,
    pub roll: Vec<String>,
    pub airdash: Vec<String>,
    pub shield: Vec<String>,
    pub voice: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LimbRule { #[serde(default)] pub contains: Vec<String>, pub limb: String }

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LimbRules { pub default: String, pub rules: Vec<LimbRule> }

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CharacterStatsTables {
    pub hitbox_sections: Vec<HitboxSection>,
    pub stats_fields: StatsFields,
    pub limb_rules: LimbRules,
}

pub fn character_stats_tables() -> &'static CharacterStatsTables {
    static CACHE: OnceLock<CharacterStatsTables> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/character/stats_tables.jsonc", DEFAULT_CHARACTER_STATS_TABLES)
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct PhysMap { pub ssf2: String, pub fm: String }

#[derive(Debug, Clone, Deserialize)]
pub struct FieldValue { pub field: String, pub value: String }

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProjectileTables {
    pub physics_map: Vec<PhysMap>,
    pub scaffolding: Vec<FieldValue>,
}

pub fn projectile_tables() -> &'static ProjectileTables {
    static CACHE: OnceLock<ProjectileTables> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/projectile_tables.jsonc", DEFAULT_PROJECTILE_TABLES)
    })
}

pub fn character_hitbox_stats() -> &'static HitboxStatsMapping {
    static CACHE: OnceLock<HitboxStatsMapping> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/character/hitbox_stats.jsonc", DEFAULT_CHARACTER_HITBOX_STATS)
    })
}

pub fn api_commands() -> &'static ApiCommands {
    static CACHE: OnceLock<ApiCommands> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/commands.jsonc", DEFAULT_API_COMMANDS)
    })
}

// ─── Stat derivations (compiled expressions) ────────────────────────────────

/// A stat-derivation expression compiled once into a fasteval instruction.
/// `slab` owns the parsed/compiled arena; `instr` is evaluated against it.
struct CompiledDerivation {
    slab: fasteval::Slab,
    instr: fasteval::Instruction,
}

/// Parse + compile one derivation expression. Done once, at config-load time.
fn compile_derivation(expr: &str) -> Result<CompiledDerivation, fasteval::Error> {
    use fasteval::Compiler;
    let parser = fasteval::Parser::new();
    let mut slab = fasteval::Slab::new();
    let instr = parser
        .parse(expr, &mut slab.ps)?
        .from(&slab.ps)
        .compile(&slab.ps, &mut slab.cs);
    Ok(CompiledDerivation { slab, instr })
}

/// Every stat derivation, compiled once. Built lazily from `character_stats`.
fn compiled_stat_derivations() -> &'static BTreeMap<String, CompiledDerivation> {
    static CACHE: OnceLock<BTreeMap<String, CompiledDerivation>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut map = BTreeMap::new();
        for (name, expr) in &character_stats().derivations {
            match compile_derivation(expr) {
                Ok(compiled) => { map.insert(name.clone(), compiled); }
                Err(e) => log::warn!(
                    "stat derivation '{}' failed to compile ({}): {}", name, e, expr
                ),
            }
        }
        map
    })
}

/// Evaluate the named stat derivation. `vars` exposes the already-converted
/// stat values to the expression; `max`, `min` (fasteval built-ins) and
/// `clamp(x, lo, hi)` are available. Returns `None` if the derivation is
/// absent or failed to compile.
pub fn evaluate_stat_derivation(name: &str, vars: &BTreeMap<String, f64>) -> Option<f64> {
    use fasteval::Evaler;
    let compiled = compiled_stat_derivations().get(name)?;
    let mut ns = |fname: &str, args: Vec<f64>| -> Option<f64> {
        if args.is_empty() {
            // Bare identifier — resolve as a variable.
            return vars.get(fname).copied();
        }
        match (fname, args.as_slice()) {
            ("clamp", [x, lo, hi]) => Some(x.max(*lo).min(*hi)),
            _ => None, // unknown function — fasteval handles max/min/... itself
        }
    };
    compiled.instr.eval(&compiled.slab, &mut ns).ok()
}
