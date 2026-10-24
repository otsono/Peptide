//! Runtime-loaded conversion mapping tables.
//!
//! Mapping data lives in editable JSONC/TOML files under `mappings/<category>/`
//! (JSONC = JSON with `//` comments and trailing commas, so entries can be
//! annotated). Every file is **read from disk at runtime** — including
//! derivation formulas, helper-template Haxe, and comments — so it can be
//! tweaked without recompiling. Nothing is baked into the binary; the files
//! must be present at one of the resolved locations (see [`candidate_paths`]).
//! If a required mapping file is missing or malformed, loading panics with the
//! list of locations tried rather than silently using stale data.
//!
//! The mappings directory ships next to the binary in a packaged build, and is
//! found via `CARGO_MANIFEST_DIR` in a dev/source checkout. Override the whole
//! directory with the `PEPTIDE_MAPPINGS_DIR` environment variable.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::physics_sim::ScaleParams;

/// The converter crate's source dir (`…/crates/ssf2-converter`), captured at
/// compile time. Only used to *locate* the mappings files at runtime in a dev /
/// source checkout — the file CONTENT is always read from disk, never embedded.
const CRATE_DIR: &str = env!("CARGO_MANIFEST_DIR");

// ─── Schema ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct AnimationMappings {
    /// SSF2 xframe/animation name → Fraymakers animation slot.
    #[serde(default)]
    pub ssf2_to_fm: BTreeMap<String, String>,
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
    /// The stats this engine HAS, for a write made while the match is running. A name
    /// not in here has nothing to land on and the engine refuses the whole write.
    #[serde(default)]
    pub fm_runtime_stats: Vec<String>,
    /// What a stat read while the match runs is called here, source name → engine name.
    #[serde(default)]
    pub fm_runtime_read_names: BTreeMap<String, String>,
    /// What a stat this engine does not have answers when read: the source's default for it.
    #[serde(default)]
    pub runtime_read_defaults: BTreeMap<String, serde_json::Value>,
    /// The unit each runtime stat is held in here ("accel" / "speed" -> the stat names), so a
    /// read can scale it back to the source's units.
    #[serde(default)]
    pub runtime_read_units: BTreeMap<String, Vec<String>>,
    /// A stat this engine keeps under another shape, read while the match runs: the source's
    /// name → the expression that answers it here (`{recv}` is the object asked).
    #[serde(default)]
    pub runtime_read_exprs: BTreeMap<String, String>,
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
    /// The scale knob (`size_multiplier` + fps), flattened from the top-level
    /// `size_multiplier`/`ssf2_fps`/`fm_fps` keys in `stats.jsonc`. This is the
    /// SINGLE TUNABLE KNOB: every physics stat (walk/dash/jump speeds, gravity,
    /// friction, accel) is derived from it via [`StatMappings::scale`], so editing
    /// `size_multiplier` rescales the whole physics profile consistently. The
    /// definition + default live in one place, [`ScaleParams`].
    #[serde(flatten)]
    pub scaling: ScaleParams,
}

impl StatMappings {
    /// SSF2 key names to try for a Fraymakers stat field, in priority order.
    pub fn keys_for(&self, field: &str) -> &[String] {
        self.field_keys.get(field).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Scale a raw SSF2 stat into Fraymakers units, derived ENTIRELY from the
    /// `size_multiplier` knob (+ fps ratio). `name` classifies the stat as a
    /// velocity or an acceleration — the two scale differently under a
    /// frame-rate change. Any floor configured in `multipliers[name]` is kept
    /// as a safety net so a degenerate raw value never produces a sub-playable
    /// stat. Unknown names fall back to the legacy divisor/target multiplier.
    pub fn scale(&self, name: &str, raw: f64) -> f64 {
        let factor = match name {
            // accelerations (px/frame²)
            "gravity" | "air_friction" | "friction" | "walk_accel" | "air_accel" => {
                self.scaling.accel_scale()
            }
            // velocities (px/frame)
            "speed" | "jump" | "walk" | "dash" | "fall" => self.scaling.velocity_scale(),
            // not a physics scale we own → legacy behaviour (e.g. frame-count
            // multipliers configured elsewhere).
            _ => return self.multipliers.get(name).map(|m| m.apply(raw)).unwrap_or(raw),
        };
        // no floor on a physics scale: a value reaches here only when the source stated one (a
        // missing one takes the template default before it is scaled), and a floor clamped the
        // lightest fighters to someone else's number (kirby's gravity 0.276 came out 0.3)
        raw * factor
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
    /// Multiplied into the value on the way out. The knockback fields carry one: the two engines
    /// launch a victim at different speeds for the same numbers, and the factor is measured.
    #[serde(default)]
    pub scale: Option<f64>,
    /// Added to the value BEFORE the frame doubling. A freeze field carries 1: SSF2 counts the
    /// frames a hit freezes the victim for AFTER the frame it lands on, and Fraymakers counts
    /// the whole freeze including that frame. Measured on both engines.
    #[serde(default)]
    pub plus_frames: f64,
    /// What one unit of a FIXED launch is worth before the scale (see `fixed_knockback` in
    /// hitbox_stats.jsonc), and what the engine adds to it whatever the field says.
    #[serde(default)]
    pub per_unit: Option<f64>,
    #[serde(default)]
    pub constant: Option<f64>,
    /// A freeze a hit does not STATE, worked out from its damage the way the source does:
    /// `floor(damage * unstated_per_damage + unstated_constant)` source frames.
    #[serde(default)]
    pub unstated_per_damage: Option<f64>,
    #[serde(default)]
    pub unstated_constant: Option<f64>,
}

/// SSF2 -> Fraymakers hitbox-stats conversion (character-scoped).
#[derive(Debug, Clone, Deserialize)]
pub struct HitboxStatsMapping {
    #[serde(default)]
    pub fields: Vec<HitboxField>,
    /// Source hitbox fields this engine has no counterpart for, each refused BY NAME when asked.
    /// A write carrying one is rejected WHOLE, so they are dropped from runtime writes.
    #[serde(default)]
    pub no_counterpart: Vec<String>,
    /// The stats this engine HAS. Anything a source write names that is not in here
    /// has nothing to land on, and the engine refuses the whole write for it.
    #[serde(default)]
    pub fm_stats: Vec<String>,
    /// A source box's true-or-false field stated under an engine field (see hitbox_stats.jsonc).
    #[serde(default)]
    pub flags: Vec<HitboxFlag>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HitboxFlag {
    pub ssf2_key: String,
    pub when: bool,
    pub fm_field: String,
    pub value: String,
}

impl HitboxStatsMapping {
    /// the engine fields a source box's flags give it, as `(field, value as written)`
    pub fn flag_fields(&self, raw: &std::collections::BTreeMap<String, f64>) -> Vec<(String, String)> {
        self.flags.iter()
            .filter(|f| raw.get(f.ssf2_key.as_str()).is_some_and(|v| (*v != 0.0) == f.when))
            .map(|f| (f.fm_field.clone(), f.value.clone())).collect()
    }

    /// Whether a source hitbox states FIXED knockback (`weightKB`), which does not grow with the
    /// victim's damage. Measured on both engines: such a hit launches identically at 0% and at
    /// 62%, where a `power` hit doubles over the same range. Every path that emits a hitbox asks
    /// this, so a projectile and a stage hazard read the same rule a fighter's own move does.
    pub fn is_fixed_knockback(obj: &std::collections::BTreeMap<String, f64>) -> bool {
        obj.get("weightKB").is_some_and(|w| *w > 0.0)
    }

    /// The launch a FIXED hit gives, in the source's own units: what the field states is not it.
    /// `(weightKB * per_unit + constant) * kbConstant/100 + power`, for a victim of the table's
    /// weight (the source scales it against the victim's, and nothing here carries that).
    pub fn fixed_knockback(&self, obj: &std::collections::BTreeMap<String, f64>) -> Option<f64> {
        let w = obj.get("weightKB").copied().filter(|w| *w > 0.0)?;
        let f = self.fields.iter().find(|f| f.fm_field == "fixed_knockback")?;
        let growth = obj.get("kbConstant").copied().unwrap_or(100.0) / 100.0;
        // the box's own power is added on top (mario's down air states 30 of each, and launched
        // at 0.39 of the source's with the power left out, 0.66 with it)
        let power = obj.get("power").copied().unwrap_or(0.0);
        Some((w * f.per_unit? + f.constant?) * growth + power)
    }

    /// The freeze, in SOURCE frames, a hit of `damage` gets for a field it does not state, when the
    /// mapping says how the source works it out.
    pub fn unstated_freeze(&self, fm_field: &str, damage: f64) -> Option<f64> {
        let f = self.fields.iter().find(|f| f.fm_field == fm_field)?;
        Some((damage * f.unstated_per_damage? + f.unstated_constant?).floor())
    }

    /// What is added to a Fraymakers field's value before the frame doubling (0.0 if none).
    pub fn plus_frames_for(&self, fm_field: &str) -> f64 {
        self.fields.iter().find(|f| f.fm_field == fm_field).map(|f| f.plus_frames).unwrap_or(0.0)
    }

    /// What a Fraymakers hitbox field's value is multiplied by on the way out (1.0 if none).
    pub fn scale_for(&self, fm_field: &str) -> f64 {
        self.fields.iter()
            .find(|f| f.fm_field == fm_field)
            .and_then(|f| f.scale)
            .unwrap_or(1.0)
    }

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
/// A call rewritten by its parsed arguments rather than by a pattern over its text: the
/// arguments are read with brackets and strings balanced, so a nested call or a string holding a
/// bracket is one argument, as it is to the language.
///   call      the method name (`setScale`)
///   receiver  "any" (default: whatever the call is made on), "self" (only on `self`), "other" (anything but `self`), or "none"
///             (a bare call)
///   args      the number of arguments the rewrite applies to; any when absent
///   template  the replacement: `{recv}` the receiver, `{1}`.. an argument, `{args}` all of them,
///             `{1.name}` the value of field `name` of an object-literal argument (the rewrite
///             does not apply where that field is missing)
#[derive(Debug, Clone, Deserialize)]
pub struct CallRewrite {
    pub call: String,
    #[serde(default)]
    pub receiver: String,
    #[serde(default)]
    pub args: Option<usize>,
    pub template: String,
    #[serde(default)]
    pub note: String,
}

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
    /// Optional: added to the value before doubling. The freeze fields carry 1, because SSF2
    /// counts the frames a hit freezes for AFTER the frame it lands on and Fraymakers counts
    /// the whole freeze (see mappings/character/hitbox_stats.jsonc, measured on both).
    #[serde(default)]
    pub plus: i64,
    /// Optional: when true, a literal `1` is left unchanged instead of doubled.
    /// SSF2 plays at 30fps and FM at 60fps, so most frame counts double — but a
    /// per-frame timer (`createTimer(1, …)`) should keep firing every engine
    /// frame for responsiveness rather than becoming an every-other-frame poll.
    #[serde(default)]
    pub keep_one: bool,
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
        /// The unit a numeric value is converted by on the way through: `speed` (velocity
        /// scale), `accel` (acceleration scale), `decay` (a NEGATIVE per-frame slowdown, as
        /// the source states one, becomes this engine's positive friction). A value that is
        /// not a plain number, or a decay that is not negative, passes through with a TODO.
        #[serde(default)]
        unit: Option<String>,
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
///   - `haxe_builtins`       — Haxe/hscript stdlib methods the decompiler
///                              surfaces as bare calls; known, never a gap
///   - `ssf2_only`           — calls with no Fraymakers equivalent; replaced
///                              by `// [SSF2-only: NAME] …` markers
#[derive(Debug, Clone, Deserialize)]
pub struct ApiCommands {
    #[serde(default)]
    pub replacements: Vec<Replacement>,
    #[serde(default)]
    pub regex_replacements: Vec<RegexReplacement>,
    #[serde(default)]
    pub call_rewrites: Vec<CallRewrite>,
    #[serde(default)]
    pub call_splits: std::collections::BTreeMap<String, CallSplit>,
    /// What a call means when the object it is asked of is a PROJECTILE rather than a fighter:
    /// the rest of this table reads the fighter's meaning.
    #[serde(default)]
    pub projectile_self: ProjectileSelf,
    #[serde(default)]
    pub attach_effect_props: std::collections::BTreeMap<String, AttachEffectPropMapping>,
    #[serde(default)]
    pub global_vfx_map: std::collections::BTreeMap<String, String>,
    /// What the source's api answers for an object's type, by this engine's entity kind.
    #[serde(default)]
    pub entity_type_names: std::collections::BTreeMap<String, String>,
    /// The source's character states by name, and the states here each covers.
    #[serde(default)]
    pub source_state_names: std::collections::BTreeMap<String, Vec<String>>,
    /// Accessors the source answers from a declared stat, by the stat's name.
    #[serde(default)]
    pub stat_accessors: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub global_sound_map: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub frame_params: Vec<FrameParam>,
    #[serde(default)]
    pub passthrough_fm_apis: Vec<NamedApi>,
    #[serde(default)]
    pub haxe_builtins: Vec<NamedApi>,
    #[serde(default)]
    pub ssf2_only: Vec<NamedApi>,
    /// Calls a converter pass translates by name (not a table row): a base-class method of the
    /// same name is theirs, not something to carry over.
    #[serde(default)]
    pub translated_in_code: Vec<String>,
}

// ─── Script.hx output templates ──────────────────────────────────────────────
//
// The Haxe strings the codegen emits into Script.hx (and projectile Script.hx)
// live here, not hardcoded in Rust, so their shape can change without
// recompiling — same philosophy as `global_sound_map` etc. Grouped by feature.
// Each value is a string filled by `str::replace`-ing `{{slot}}` placeholders;
// the control flow / data assembly stays in Rust. Empty by default —
// `require_template` panics with the qualified key if a needed one is missing.

/// Per-frame playSound("id") rewrite (api_mappings::build_sound_call).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PlaySoundTemplates {
    pub global: String,
    pub asset: String,
    pub placeholder_no_static_id: String,
    pub placeholder_unmapped: String,
}

/// Projectile Script.hx generator (haxe_gen::generate_projectile_script).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProjectileTemplates {
    pub single_state: String,
    pub multi_state: String,
    /// Used only for a projectile whose package ships no code for it.
    pub default_handlers: String,
    pub default_listeners: String,
    /// What a projectile uses to end itself once it is dying.
    pub death_watch: String,
    pub death_watch_handler: String,
    pub lstate_idle_prep: String,
    pub lstate_prep_line: String,
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
    pub stats: CharStatsTemplates,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProjectileHelpers {
    pub script: ProjectileTemplates,
    pub stats: ProjStatsTemplates,
}

/// Aggregate of the per-entity template files.
#[derive(Debug, Clone, Default)]
pub struct HelperTemplates {
    pub global: GlobalHelpers,
    pub character: CharacterHelpers,
    pub projectile: ProjectileHelpers,
}

/// Like `load`, but for TOML template files (no comment-stripping needed —
/// TOML has native comments; literal `'''` blocks preserve the Haxe verbatim).
fn load_toml<T: for<'de> Deserialize<'de>>(rel: &str) -> T {
    let (text, path) = read_mapping_file(rel);
    toml::from_str(&text)
        .unwrap_or_else(|e| panic!("template file {} is malformed: {e}", path.display()))
}

/// The per-entity Script.hx templates, loaded once from their TOML files.
pub fn script_templates() -> &'static HelperTemplates {
    static CACHE: OnceLock<HelperTemplates> = OnceLock::new();
    CACHE.get_or_init(|| HelperTemplates {
        global:     load_toml("mappings/global_helpers.toml"),
        character:  load_toml("mappings/character_helpers.toml"),
        projectile: load_toml("mappings/projectile_helpers.toml"),
    })
}

// ─── Loading ────────────────────────────────────────────────────────────────

/// Strip JSONC down to plain JSON: remove `//` line and `/* */` block
/// comments and drop trailing commas, so `serde_json` can parse it.
/// String-aware — comment markers and commas inside string literals are
/// preserved. `serde_json` is used (not a JSON5 crate) because it keeps the
/// integer/float distinction the `constants` section relies on.
pub(crate) fn strip_jsonc(src: &str) -> String {
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

/// Candidate locations for a mapping file (`rel` is like
/// `mappings/character/stats.jsonc`), tried in order. The first that exists
/// wins. Resolution order:
///   1. `$PEPTIDE_MAPPINGS_DIR/<rel-without-leading-"mappings/">` (explicit override)
///   2. the current working directory (`./mappings/…`)
///   3. the packaged layout: a `data/` subfolder next to the binary (`<exe-dir>/data/mappings/…`)
///   4. legacy packaged layout: directly next to the binary (`<exe-dir>/mappings/…`)
///   5. `<exe-dir>/../../mappings/…` (a `target/<profile>/<bin>` dev build)
///   6. the converter crate's source dir (`CRATE_DIR/mappings/…` — dev/source checkout)
fn candidate_paths(rel: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    // 1. explicit dir override — strip the leading "mappings/" so the env var
    //    points straight at a mappings dir.
    if let Ok(dir) = std::env::var("PEPTIDE_MAPPINGS_DIR") {
        let sub = rel.strip_prefix("mappings/").unwrap_or(rel);
        paths.push(PathBuf::from(dir).join(sub));
    }
    // 2. cwd-relative
    paths.push(PathBuf::from(rel));
    // 3/4/5. data/ subfolder next to the binary, directly next to it, and a dev
    //        target/<profile>/<bin> layout.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("data").join(rel)); // packaged: <exe-dir>/data/mappings/…
            paths.push(dir.join(rel));              // legacy: <exe-dir>/mappings/…
            if let Some(up) = dir.parent().and_then(|p| p.parent()) {
                paths.push(up.join(rel));
            }
        }
    }
    // 5. the converter crate's source tree (absolute; always valid in a checkout)
    paths.push(PathBuf::from(CRATE_DIR).join(rel));
    paths
}

/// The raw text of a mapping/template file, for a check that wants to read the vocabulary the
/// templates are written IN rather than the values they parse to.
pub fn mapping_file_text(rel: &str) -> Option<String> {
    candidate_paths(rel).into_iter().find_map(|p| std::fs::read_to_string(p).ok())
}

/// Read a mapping/template file from disk, trying each candidate location.
/// Returns the file text and the path it came from. Panics (with the list of
/// locations tried) if the file is not found at any location — nothing is baked
/// into the binary, so a missing mappings dir is a hard, loud error.
fn read_mapping_file(rel: &str) -> (String, PathBuf) {
    let tried = candidate_paths(rel);
    for path in &tried {
        if let Ok(text) = std::fs::read_to_string(path) {
            log::info!("Loaded mapping file {}", path.display());
            return (text, path.clone());
        }
    }
    panic!(
        "mapping file {rel:?} not found. Tried:\n{}\n\
         Ship the `mappings/` dir next to the binary, run from the repo, or set \
         PEPTIDE_MAPPINGS_DIR.",
        tried.iter().map(|p| format!("  - {}", p.display())).collect::<Vec<_>>().join("\n")
    );
}

/// Load and parse a JSONC mapping file from disk (no compiled-in fallback).
fn load<T: for<'de> Deserialize<'de>>(rel: &str) -> T {
    let (text, path) = read_mapping_file(rel);
    serde_json::from_str(&strip_jsonc(&text))
        .unwrap_or_else(|e| panic!("mapping file {} is malformed: {e}", path.display()))
}

// ─── Cached accessors ───────────────────────────────────────────────────────

pub fn character_animations() -> &'static AnimationMappings {
    static CACHE: OnceLock<AnimationMappings> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/character/animations.jsonc")
    })
}

pub fn character_stats() -> &'static StatMappings {
    static CACHE: OnceLock<StatMappings> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/character/stats.jsonc")
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
        load("mappings/character/animation_template.jsonc")
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
        load("mappings/character/stats_tables.jsonc")
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct PhysMap {
    pub ssf2: String,
    pub fm: String,
    /// How the value converts: "velocity" (per-frame speed), "accel" (per-frame^2), "size"
    /// (source pixels, which the entity states in its own units), or absent for a plain copy.
    #[serde(default)]
    pub scale: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FieldValue { pub field: String, pub value: String }

#[derive(Debug, Clone, Deserialize)]
pub struct FlagMap {
    pub ssf2: String,
    pub fm: String,
    /// The two engines ask the opposite question (survive vs destroy).
    #[serde(default)]
    pub invert: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProjectileTables {
    /// Every stat the engine will take on a projectile.
    #[serde(default)]
    pub fm_stats: Vec<String>,
    pub physics_map: Vec<PhysMap>,
    #[serde(default)]
    pub flag_map: Vec<FlagMap>,
    pub scaffolding: Vec<FieldValue>,
    /// Source keys with no engine ProjectileStats field that the port applies anyway (in the
    /// projectile's script or at the spawn). Kept out of the "no mapping" TODOs.
    #[serde(default)]
    pub handled_in_script: Vec<HandledElsewhere>,
    /// What a projectile has when it states nothing: the source's own starting values, in its
    /// own units, applied the same way a stated one is.
    #[serde(default)]
    pub source_defaults: std::collections::BTreeMap<String, f64>,
    /// The same for the flags a projectile declares about itself (reversible, absorbable).
    #[serde(default)]
    pub source_flag_defaults: std::collections::BTreeMap<String, bool>,
}

/// A source stat with no engine ProjectileStats field that the port still applies elsewhere.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HandledElsewhere {
    pub ssf2: String,
    /// One-line note the stats file prints instead of a "no mapping" TODO.
    #[serde(default)]
    pub r#where: String,
}

/// The published api's method names.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FmApi {
    pub methods: Vec<String>,
    /// What a game object answers to, which is less than a character does.
    pub game_object_methods: Vec<String>,
    /// What the match itself answers to. A name it does not have is a null, not a fall-through.
    pub match_methods: Vec<String>,
    /// Not the engine's at all: arrays, strings, numbers, and this converter's own wrappers.
    pub standard_methods: Vec<String>,
}

pub fn fm_api() -> &'static FmApi {
    static CACHE: OnceLock<FmApi> = OnceLock::new();
    CACHE.get_or_init(|| load("mappings/fm_api.jsonc"))
}

pub fn projectile_tables() -> &'static ProjectileTables {
    static CACHE: OnceLock<ProjectileTables> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/projectile_tables.jsonc")
    })
}

/// Per-stage display + music overrides for the SSF2 -> Fraymakers stage converter
/// (`mappings/stage/metadata.jsonc`). Looked up by the SSF2 stage id; missing entries
/// fall back to a title-cased name + `default_music`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StageMetadataMap {
    /// FM bgm referenced when a stage has no `music` override. Must be a real public
    /// Fraymakers resource (the SSF2 soundtrack is not shipped with FM).
    #[serde(default)]
    pub default_music: String,
    /// SSF2 id -> overrides.
    #[serde(default)]
    pub stages: std::collections::BTreeMap<String, StageMetadataEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StageMetadataEntry {
    /// Human display name (e.g. "Final Destination"). None -> title-cased id.
    pub name: Option<String>,
    /// FM bgm resource ids to play. Empty -> `default_music`.
    #[serde(default)]
    pub music: Vec<String>,
    /// Source series, for the description. Optional.
    pub series: Option<String>,
}

pub fn stage_metadata() -> &'static StageMetadataMap {
    static CACHE: OnceLock<StageMetadataMap> = OnceLock::new();
    CACHE.get_or_init(|| load("mappings/stage/metadata.jsonc"))
}

pub fn character_hitbox_stats() -> &'static HitboxStatsMapping {
    static CACHE: OnceLock<HitboxStatsMapping> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/character/hitbox_stats.jsonc")
    })
}

pub fn api_commands() -> &'static ApiCommands {
    static CACHE: OnceLock<ApiCommands> = OnceLock::new();
    CACHE.get_or_init(|| {
        load("mappings/commands.jsonc")
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

/// `commands.jsonc :: projectile_self`.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct ProjectileSelf {
    /// split destinations (`method.field`) a projectile does not take on itself
    #[serde(default)]
    pub call_split_drops: Vec<String>,
}
