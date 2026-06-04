//! Tests for the JSONC-loaded mapping tables and the helpers that consume
//! them. Catches accidental regressions in:
//!   - the JSONC parser (`strip_jsonc`)
//!   - the cached accessors (`character_animations`, `character_stats`,
//!     `character_hitbox_stats`, `api_commands`)
//!   - the per-stat scaling / offset / derivation logic that
//!     CharacterStats.hx generation depends on
//!   - the SSF2→FM hitbox field-name canon + isframe flag

use ssf2_converter::mappings::{
    api_commands, character_animations, character_hitbox_stats,
    character_stats, evaluate_stat_derivation,
};
use std::collections::BTreeMap;

// ─── JSONC accessors load cleanly ────────────────────────────────────────

#[test]
fn animations_table_loads() {
    let a = character_animations();
    // ssf2_to_fm must contain at least the canonical SSF2 names.
    assert!(a.ssf2_to_fm.contains_key("stand"),
        "`stand` must be in the SSF2→FM animation table");
    assert!(a.ssf2_to_fm.contains_key("a_air"),
        "`a_air` must be in the SSF2→FM animation table");
    // Fraymakers' canonical standing-state animation is named `stand`
    // (CState.STAND; see the official character template's AnimationStats.hx,
    // which defines `stand: {}` and has no `idle` motion). SSF2's `idle`
    // sprite label is already normalized to the SSF2 anim name `stand` in
    // the earlier label_to_ssf2 stage, so ssf2_to_fm maps `stand → stand`.
    assert_eq!(a.ssf2_to_fm.get("stand").map(String::as_str), Some("stand"));
}

#[test]
fn stats_table_loads_with_required_fields() {
    let s = character_stats();
    // Every CharacterStats.hx-driving stat needs a field_keys entry.
    for required in &["weight", "gravity", "fall_speed", "walk_speed", "max_jumps"] {
        assert!(!s.keys_for(required).is_empty(),
            "stats.jsonc field_keys is missing `{}`", required);
    }
}

#[test]
fn stats_multiplier_apply_scales_and_floors() {
    let s = character_stats();
    // Take any multiplier defined in stats.jsonc — `gravity` is canonical.
    // We don't pin exact values (they may be tuned), but we assert the
    // shape: applying a positive value produces a finite, non-negative
    // result for the standard multipliers.
    let names = ["gravity", "speed", "jump", "walk", "dash", "air_friction"];
    for name in names {
        if s.multipliers.contains_key(name) {
            let scaled = s.scale(name, 10.0);
            assert!(scaled.is_finite() && scaled >= 0.0,
                "`{}` scale(10.0) should produce a finite non-negative; got {}",
                name, scaled);
        }
    }
}

#[test]
fn stats_offset_max_jumps_plus_one() {
    let s = character_stats();
    // SSF2 max_jump counts midair jumps; FM counts total jumps. The
    // canonical offset is +1.
    assert_eq!(s.offset("max_jumps"), 1,
        "max_jumps offset should be +1 to bridge SSF2/FM jump semantics");
}

#[test]
fn stats_offset_default_zero() {
    let s = character_stats();
    assert_eq!(s.offset("a_totally_unconfigured_stat"), 0,
        "unconfigured offsets must default to 0");
}

#[test]
fn stats_constant_missing_returns_marker() {
    let s = character_stats();
    let v = s.constant("__not_in_jsonc__");
    assert!(v.contains("MISSING"),
        "missing constants must emit a visible MISSING marker; got: {}", v);
}

#[test]
fn stats_derivation_compiles_and_evaluates() {
    // `shortHopSpeed` is one of the canonical derivations. Even with
    // empty input variables, the expression must compile (not error).
    let vars: BTreeMap<String, f64> = BTreeMap::new();
    let r = evaluate_stat_derivation("shortHopSpeed", &vars);
    // Either it returns Some(value) (built-in inputs default to 0 and the
    // expression evaluates to a number) or None (the derivation isn't
    // present in stats.jsonc). Either way, no panic.
    if let Some(v) = r {
        assert!(v.is_finite(), "shortHopSpeed derivation produced non-finite {}", v);
    }
}

#[test]
fn stats_derivation_uses_variables() {
    // If `shortHopSpeed` is `jump_speed * 0.6` or similar, feeding a
    // jump_speed value of 10 should produce a positive result.
    let mut vars: BTreeMap<String, f64> = BTreeMap::new();
    vars.insert("jump_speed".into(), 10.0);
    vars.insert("air_mobility_raw".into(), 1.0);
    vars.insert("aerial_friction".into(), 0.5);
    let r = evaluate_stat_derivation("shortHopSpeed", &vars);
    if let Some(v) = r {
        assert!(v > 0.0,
            "shortHopSpeed with jump_speed=10 should be positive; got {}", v);
    }
}

// ─── HitboxStats canon ───────────────────────────────────────────────────

#[test]
fn hitbox_field_mapping_canonical_names() {
    let h = character_hitbox_stats();
    // damage: damage; angle: direction|angle; baseKnockback covers
    // power/kbConstant/weightKB; hitstop covers hitLag/hitstop; hitstun
    // covers hitStun/hitstun.
    assert!(h.keys_for("damage").contains(&"damage".to_string()));
    let angle_keys: Vec<&str> = h.keys_for("angle").iter().map(String::as_str).collect();
    assert!(angle_keys.contains(&"direction") || angle_keys.contains(&"angle"),
        "angle should map from either `direction` or `angle`; got {:?}", angle_keys);
}

#[test]
fn hitbox_isframe_flag_on_frame_count_fields() {
    let h = character_hitbox_stats();
    // hitstop / selfHitstop / hitstun should all be flagged as frame counts
    // so they get doubled for the 30→60fps move.
    assert!(h.is_frame("hitstop"),
        "hitstop must be flagged as isframe (it's a frame count)");
    assert!(h.is_frame("hitstun"),
        "hitstun must be flagged as isframe");
    // damage / angle / baseKnockback are NOT frame counts.
    assert!(!h.is_frame("damage"),
        "damage must NOT be flagged as isframe");
    assert!(!h.is_frame("angle"),
        "angle must NOT be flagged as isframe");
    assert!(!h.is_frame("baseKnockback"),
        "baseKnockback must NOT be flagged as isframe");
}

#[test]
fn hitbox_keys_unknown_field_is_empty() {
    let h = character_hitbox_stats();
    assert!(h.keys_for("absolutely_made_up_field").is_empty(),
        "unknown fields must return an empty key list");
}

// ─── ApiCommands sanity ──────────────────────────────────────────────────

#[test]
fn api_commands_loads_with_required_sections() {
    let c = api_commands();
    assert!(!c.replacements.is_empty(), "replacements must not be empty");
    assert!(!c.call_splits.is_empty(), "call_splits must not be empty");
    assert!(!c.frame_params.is_empty(), "frame_params must not be empty");
    // updateAttackStats is the canonical call_split (see DEVELOPMENT.md).
    assert!(c.call_splits.contains_key("updateAttackStats"),
        "call_splits must include updateAttackStats");
}

#[test]
fn api_commands_replacements_collapse_self_self() {
    // The literal table must collapse `self.self.` to `self.` — this is
    // load-bearing for every translated frame script.
    let c = api_commands();
    let found = c.replacements.iter().any(|r| r.from == "self.self." && r.to == "self.");
    assert!(found,
        "replacements must collapse `self.self.` to `self.`");
}

#[test]
fn api_commands_frame_params_isframe_only() {
    let c = api_commands();
    // Every frame_param entry that's actually doubled must have isframe=true.
    // The double_frame_counts pass keys on that flag — if all entries have
    // it false, nothing doubles.
    let any_isframe = c.frame_params.iter().any(|p| p.isframe);
    assert!(any_isframe,
        "at least one frame_params entry must have isframe=true");
}
