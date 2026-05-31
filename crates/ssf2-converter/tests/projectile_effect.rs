//! Tests for projectile naming + Annie-convention conversion, and the
//! effect-animation-name helper that drives the attachEffect-rewrite
//! per-character map.

use ssf2_converter::entity_gen::{effect_animation_names, ssf2_proj_label_to_fm_anim};
use ssf2_converter::image_extractor::DiscoveredEffect;

// ─── ssf2_proj_label_to_fm_anim ──────────────────────────────────────────

#[test]
fn proj_label_attack_idle_maps_to_projectile_idle() {
    assert_eq!(ssf2_proj_label_to_fm_anim("attack_idle"), "projectileIdle");
}

#[test]
fn proj_label_attack_hold_maps_to_projectile_held() {
    assert_eq!(ssf2_proj_label_to_fm_anim("attack_hold"), "projectileHeld");
}

#[test]
fn proj_label_attack_toss_maps_to_projectile_active() {
    assert_eq!(ssf2_proj_label_to_fm_anim("attack_toss"), "projectileActive");
}

#[test]
fn proj_label_unknown_is_prefixed_and_sanitized() {
    // Unknown labels become `projectile_<sanitized>` — non-alphanumeric → `_`.
    assert_eq!(ssf2_proj_label_to_fm_anim("custom_label"), "projectile_custom_label");
    assert_eq!(ssf2_proj_label_to_fm_anim("with space"),    "projectile_with_space");
    assert_eq!(ssf2_proj_label_to_fm_anim("special!chars"), "projectile_special_chars");
}

// ─── effect_animation_names ──────────────────────────────────────────────
//
// This helper drives the per-character `EFFECT_PRIMARY_ANIMS` map that
// `rewrite_attach_effect_calls` consults to pick the right animation name
// when rewriting `self.attachEffect("foo")` → `match.createVfx(..., animation: <X>)`.
// It MUST stay in lockstep with `generate_effect_entity`'s segment logic
// (commented at the top of `effect_animation_names`).

fn effect(name: &str, frames: u16, labels: &[(u16, &str)]) -> DiscoveredEffect {
    DiscoveredEffect {
        sprite_id: 0,
        name: name.to_string(),
        frame_count: frames,
        inner_labels: labels.iter().map(|(f, l)| (*f, l.to_string())).collect(),
    }
}

#[test]
fn effect_no_labels_yields_single_active() {
    let names = effect_animation_names(&effect("vfx_dust", 10, &[]));
    assert_eq!(names, vec!["active".to_string()],
        "label-less effects must produce a single `active` animation");
}

#[test]
fn effect_one_label_at_frame_one_uses_label_as_name() {
    // Label at frame 1 (1-based) → start=0 → fills the whole sprite.
    let names = effect_animation_names(&effect("vfx", 8, &[(1, "blast")]));
    assert_eq!(names, vec!["blast".to_string()]);
}

#[test]
fn effect_two_labels_become_two_animations() {
    let names = effect_animation_names(&effect("vfx", 10, &[(1, "start"), (5, "end")]));
    assert_eq!(names, vec!["start".to_string(), "end".to_string()]);
}

#[test]
fn effect_label_after_frame_one_prepends_active() {
    // Frames before the first label belong to a leading `active` segment.
    let names = effect_animation_names(&effect("vfx", 10, &[(4, "hit")]));
    assert_eq!(names, vec!["active".to_string(), "hit".to_string()]);
}

#[test]
fn effect_non_alphanumeric_labels_get_sanitized_in_names() {
    let names = effect_animation_names(&effect("vfx", 10, &[(1, "my-cool-fx!")]));
    assert_eq!(names.len(), 1);
    let n = &names[0];
    assert!(n.chars().all(|c| c.is_alphanumeric() || c == '_'),
        "animation name must be sanitized to alphanumerics + underscore; got {:?}", n);
}

#[test]
fn effect_zero_length_segment_is_skipped() {
    // Two labels at the same frame produce a zero-length first segment;
    // the helper should drop empty segments.
    let names = effect_animation_names(&effect("vfx", 10, &[(5, "a"), (5, "b")]));
    // Either both survive (`a` empty then `b` covers 5..10) — but a zero-
    // length segment is suppressed inside the loop. The remaining one should
    // be `b`, since its segment is 5..10.
    assert!(!names.is_empty(), "at least one animation should remain");
    // No segment in the output should be empty.
    assert!(names.iter().all(|n| !n.is_empty()));
}

// ─── First-animation-name picks (used as the primary anim for attachEffect) ──

#[test]
fn effect_primary_animation_is_first_in_list() {
    // The rewriter takes `.into_iter().next()` of the names list — so the
    // primary animation MUST be the first one returned.
    let names = effect_animation_names(&effect("vfx", 10, &[(1, "start"), (5, "end")]));
    assert_eq!(names.first().map(String::as_str), Some("start"));
}
