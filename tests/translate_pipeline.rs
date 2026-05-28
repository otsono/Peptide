//! Coverage for `api_mappings::translate_ssf2_to_fm` and every pass that
//! runs inside it.
//!
//! Pipeline order under test (from the docstring on `translate_ssf2_to_fm`):
//!   1. `remove_readiness_guards`
//!   2. `double_frame_counts`
//!   3. `apply_call_splits`
//!   4. literal `replacements` (commands.jsonc)
//!   5. `regex_replacements` (commands.jsonc)
//!   6. `rewrite_attach_effect_calls`
//!   7. `strip_last_frame_end_animation`
//!   8. `comment_out_unknown_calls`
//!
//! Each pass is tested individually; the full pipeline is then tested for
//! ordering interactions.

use ssf2_converter::api_mappings::*;
use std::collections::BTreeMap;

// ─── remove_readiness_guards ─────────────────────────────────────────────

#[test]
fn readiness_guard_simple_isready() {
    let input = "function a__frame0() {\n\
        \tif (SSF2API.isReady()) {\n\
        \t\tself.x = 1;\n\
        \t}\n\
        }";
    let out = remove_readiness_guards(input);
    assert!(!out.contains("SSF2API.isReady()"), "guard should be removed");
    assert!(out.contains("self.x = 1;"), "body should remain");
    // body should be dedented one level
    assert!(out.contains("\tself.x = 1;"));
    assert!(!out.contains("\t\tself.x = 1;"));
}

#[test]
fn readiness_guard_compound_self_and_isready() {
    let input = "\tif (self && SSF2API.isReady()) {\n\
        \t\tself.y = 2;\n\
        \t}";
    let out = remove_readiness_guards(input);
    assert!(!out.contains("SSF2API.isReady()"));
    assert!(out.contains("self.y = 2;"));
}

#[test]
fn readiness_guard_nested_inner_if_survives() {
    let input = "\tif (SSF2API.isReady()) {\n\
        \t\tif (x > 0) {\n\
        \t\t\tself.z = 1;\n\
        \t\t}\n\
        \t}";
    let out = remove_readiness_guards(input);
    assert!(!out.contains("SSF2API.isReady()"));
    assert!(out.contains("if (x > 0)"), "inner if should survive");
    assert!(out.contains("self.z = 1;"));
}

#[test]
fn readiness_guard_non_match_passes_through() {
    let input = "if (foo == bar) {\n\tself.x = 1;\n}";
    let out = remove_readiness_guards(input);
    assert_eq!(out, input, "non-guard if should pass through unchanged");
}

// ─── double_frame_counts ─────────────────────────────────────────────────
//
// Driven by commands.jsonc :: frame_params. We assert that known frame-typed
// fields and calls get their integer literal doubled, and that sentinel
// values are left alone.

#[test]
fn double_frame_counts_hitstun_field() {
    // hitStun: in object literal is flagged as a frame count.
    let input = "{ damage: 5, hitStun: 10, angle: 45 }";
    let out = double_frame_counts(input);
    // damage and angle aren't frame counts; they stay 5/45.
    // hitStun should be doubled to 20.
    assert!(out.contains("hitStun: 20"),
        "hitStun should double 10→20; got: {}", out);
    assert!(out.contains("damage: 5"), "damage should be untouched");
    assert!(out.contains("angle: 45"), "angle should be untouched");
}

#[test]
fn double_frame_counts_sentinel_left_alone() {
    // 255 is the SSF2 "no override" sentinel for hitStun. Should not double.
    let input = "{ hitStun: 255 }";
    let out = double_frame_counts(input);
    assert!(out.contains("hitStun: 255"),
        "sentinel 255 should not be doubled; got: {}", out);
}

#[test]
fn double_frame_counts_createtimer_call_arg() {
    // createTimer(delay, …) — first arg is a frame count.
    let input = "self.createTimer(5, 1);";
    let out = double_frame_counts(input);
    assert!(out.contains("createTimer(10,"),
        "createTimer arg 0 should double 5→10; got: {}", out);
}

#[test]
fn double_frame_counts_non_literal_arg_untouched() {
    // Non-literal arg means no digits immediately follow the marker — leave alone.
    let input = "self.createTimer(someVar, 1);";
    let out = double_frame_counts(input);
    assert!(out.contains("createTimer(someVar"));
}

// ─── apply_call_splits ───────────────────────────────────────────────────
//
// updateAttackStats is the canonical call_split in commands.jsonc:
//   - leaveGroundCancel / xSpeedConservation / ySpeedConservation /
//     allowMovement / etc. → updateAnimationStats.<field>
//   - hitStun → updateHitboxStats.hitstun (frame-doubled by the prior pass)
//   - direction → updateHitboxStats.angle, etc.
// Fields with no mapping become `// TODO: …` lines.

#[test]
fn call_split_splits_into_grouped_calls() {
    // A multi-field call should be split, with fields grouped by target.
    // Both `cancelWhenAirborne` and `allowControl` route to
    // updateAnimationStats per commands.jsonc :: call_splits.
    let input = "self.updateAttackStats({ cancelWhenAirborne: false, allowControl: true });";
    let out = apply_call_splits(input);
    assert!(out.contains("self.updateAnimationStats("),
        "should emit a updateAnimationStats call; got: {}", out);
    assert!(out.contains("leaveGroundCancel: false"),
        "cancelWhenAirborne → leaveGroundCancel rename should land; got: {}", out);
    assert!(out.contains("allowMovement: true"),
        "allowControl → allowMovement rename should land; got: {}", out);
    // Only one call (not two), because both fields route to the same target.
    let call_count = out.matches("updateAnimationStats(").count();
    assert_eq!(call_count, 1,
        "fields targeting the same method should merge into one call; got {}: {}",
        call_count, out);
}

#[test]
fn call_split_unknown_field_emits_todo() {
    let input = "self.updateAttackStats({ totallyMadeUpField: 42 });";
    let out = apply_call_splits(input);
    assert!(out.contains("TODO"),
        "unmapped field should emit a TODO; got: {}", out);
    assert!(out.contains("totallyMadeUpField"),
        "TODO should mention the source field name; got: {}", out);
}

#[test]
fn call_split_non_object_arg_left_alone() {
    // updateAttackStats with a non-object arg should pass through unchanged.
    let input = "self.updateAttackStats(someVar);";
    let out = apply_call_splits(input);
    assert!(out.contains("self.updateAttackStats(someVar)"),
        "non-object-arg call should be untouched; got: {}", out);
}

#[test]
fn call_split_inside_comment_skipped() {
    // A `.updateAttackStats(` substring inside a `//` comment should NOT be
    // rewritten — line_is_commented_at() guards this.
    let input = "// example: self.updateAttackStats({ leaveGroundCancel: false });";
    let out = apply_call_splits(input);
    assert!(out.contains("// example: self.updateAttackStats("),
        "matches inside comments should be left alone; got: {}", out);
    // No new line should have been emitted (the comment is intact).
    assert!(!out.contains("\nself.updateAnimationStats"));
}

#[test]
fn call_split_preserves_receiver() {
    // The receiver expression must round-trip exactly, including chains.
    let input = "match.getCharacter(0).updateAttackStats({ cancelWhenAirborne: false });";
    let out = apply_call_splits(input);
    assert!(out.contains("match.getCharacter(0).updateAnimationStats("),
        "receiver should be preserved verbatim; got: {}", out);
}

#[test]
fn call_split_preserves_indent() {
    // Source line indent should propagate to every emitted line.
    let input = "\t\tself.updateAttackStats({ cancelWhenAirborne: false });";
    let out = apply_call_splits(input);
    for line in out.lines() {
        if !line.trim().is_empty() {
            assert!(line.starts_with("\t\t"),
                "emitted line should keep the source indent; got: {:?}", line);
        }
    }
}

#[test]
fn call_split_value_map_translates_value() {
    // `canFallOff: true` is value-mapped to `landType: LandType.NORMAL`,
    // with a `todo` note attached. Other values fall through to TODO.
    let input = "self.updateAttackStats({ canFallOff: true });";
    let out = apply_call_splits(input);
    assert!(out.contains("landType: LandType.NORMAL"),
        "true should map to LandType.NORMAL; got: {}", out);
    assert!(out.contains("TODO"),
        "todo note should be emitted even on success; got: {}", out);
}

#[test]
fn call_split_skip_if_value_drops_field() {
    // `air_ease: -1` is `skip_if_value: "-1"` — should vanish silently.
    let input = "self.updateAttackStats({ air_ease: -1 });";
    let out = apply_call_splits(input);
    // The original call line should be gone entirely (whole-line removal
    // when every field is skipped).
    assert!(!out.contains("updateAttackStats"),
        "call with only skip_if_value fields should disappear; got: {}", out);
    assert!(!out.contains("TODO"),
        "skip_if_value should not produce a TODO; got: {}", out);
}

// ─── rewrite_attach_effect_calls ─────────────────────────────────────────

#[test]
fn attach_effect_1arg_unknown_falls_back_to_active() {
    let input = "self.attachEffect(\"some_unmapped_effect\");";
    let out = rewrite_attach_effect_calls(input);
    assert!(out.contains("match.createVfx(new VfxStats("),
        "1-arg attachEffect should become createVfx; got: {}", out);
    assert!(out.contains("animation: \"active\""),
        "unknown effects fall back to the 'active' animation; got: {}", out);
    assert!(out.contains("getContent(\"some_unmapped_effect\")"),
        "spriteContent should reference the local effect by name; got: {}", out);
}

#[test]
fn attach_effect_global_vfx_map_routes_to_constant() {
    // global_dust_blast is in commands.jsonc :: global_vfx_map → GlobalVfx.DUST_BLAST
    let input = "self.attachEffect(\"global_dust_blast\");";
    let out = rewrite_attach_effect_calls(input);
    assert!(out.contains("spriteContent: \"global::vfx.vfx\""),
        "global effects should use the global sprite resource; got: {}", out);
    assert!(out.contains("animation: GlobalVfx."),
        "global effects should use a GlobalVfx.* constant (unquoted); got: {}", out);
    // The constant name must not be string-quoted.
    assert!(!out.contains("animation: \"GlobalVfx."),
        "GlobalVfx constant must be unquoted; got: {}", out);
}

#[test]
fn attach_effect_per_character_anim_map_used() {
    // EffectAnimGuard installs a per-character effect→primary-animation map.
    // attachEffect("trail_foo") with "trail_foo" in the map should use the
    // mapped animation, not the "active" fallback.
    let mut map = BTreeMap::new();
    map.insert("trail_foo".to_string(), "spin".to_string());
    let _guard = EffectAnimGuard::install(map);
    let input = "self.attachEffect(\"trail_foo\");";
    let out = rewrite_attach_effect_calls(input);
    assert!(out.contains("animation: \"spin\""),
        "should pull animation name from per-character map; got: {}", out);
    drop(_guard);
}

#[test]
fn attach_effect_2arg_translates_props() {
    // attach_effect_props in commands.jsonc maps:
    //   x → x (Simple)
    //   parentLock → relativeWith + resizeWith + flipWith (expand_to)
    let input = "\tself.attachEffect(\"trail_foo\", { x: 10, y: 20 });";
    let out = rewrite_attach_effect_calls(input);
    assert!(out.contains("match.createVfx(new VfxStats("),
        "2-arg form should also become createVfx; got: {}", out);
    // Mapped props get carried through.
    assert!(out.contains("x: 10") || out.contains("x:10"),
        "x prop should land in the bag; got: {}", out);
}

#[test]
fn attach_effect_2arg_unknown_prop_emits_todo() {
    let input = "\tself.attachEffect(\"trail_foo\", { wibbleWobble: 99 });";
    let out = rewrite_attach_effect_calls(input);
    assert!(out.contains("TODO") && out.contains("wibbleWobble"),
        "unmapped prop should emit a TODO; got: {}", out);
}

#[test]
fn attach_effect_value_with_inner_call_is_one_field() {
    // parse_prop_bag must respect parens — Random.getFloat(0, 1) is ONE
    // value, not two split on the inner comma.
    let input = "\tself.attachEffect(\"trail_foo\", { x: Random.getFloat(0, 1) });";
    let out = rewrite_attach_effect_calls(input);
    // The Random.getFloat call should round-trip as a single argument.
    assert!(out.contains("Random.getFloat(0, 1)"),
        "nested call commas must not split fields; got: {}", out);
}

// ─── strip_last_frame_end_animation ──────────────────────────────────────

#[test]
fn strip_endanimation_only_on_last_frame() {
    // Two anims; each has only one frame so each frame is the last.
    let input = "function a__frame0() {\n\
        \tself.endAnimation();\n\
        }\n\
        function b__frame2() {\n\
        \tself.endAnimation();\n\
        }\n";
    let out = strip_last_frame_end_animation(input);
    // Both calls land in the last frame of their group (a has only frame0,
    // b has only frame2), so both should be stripped.
    assert!(!out.contains("self.endAnimation();"),
        "all last-frame endAnimation calls should be stripped; got: {}", out);
}

#[test]
fn strip_endanimation_keeps_non_last_frame() {
    // a__frame0 is NOT the last frame; a__frame5 is. Only frame5's call is stripped.
    let input = "function a__frame0() {\n\
        \tself.endAnimation();\n\
        }\n\
        function a__frame5() {\n\
        \tself.endAnimation();\n\
        }\n";
    let out = strip_last_frame_end_animation(input);
    // frame0's call must survive.
    let lines: Vec<&str> = out.lines().collect();
    let in_frame0_section = lines.iter().any(|l| l.contains("a__frame0"));
    assert!(in_frame0_section);
    // Count surviving endAnimation lines that aren't comments.
    let surviving = out.lines()
        .filter(|l| l.trim().starts_with("self.endAnimation();"))
        .count();
    assert_eq!(surviving, 1,
        "exactly one endAnimation should survive (the non-last-frame one); got: {}",
        out);
}

// ─── comment_out_unknown_calls ───────────────────────────────────────────

#[test]
fn comment_out_unknown_marks_ssf2_only_lines() {
    // fireProjectile appears in commands.jsonc :: ssf2_only.
    let input = "function foo() {\n\
        \tself.fireProjectile(1, 2);\n\
        }\n";
    let out = comment_out_unknown_calls(input);
    assert!(out.contains("// [SSF2-only: fireProjectile]"),
        "ssf2_only call should be commented with marker; got: {}", out);
}

#[test]
fn comment_out_unknown_idempotent_on_already_commented() {
    // Lines that already start with // should be left alone.
    let input = "\t// self.resetRotation();\n";
    let out = comment_out_unknown_calls(input);
    assert_eq!(out, input,
        "lines already starting with // should pass through; got: {}", out);
}

// ─── infer_ext_var_types ─────────────────────────────────────────────────

#[test]
fn infer_ext_var_types_bool() {
    let vars = vec!["a".to_string()];
    let inits = vec![("a".to_string(), "true".to_string())];
    let m = infer_ext_var_types(&vars, &inits);
    assert_eq!(m.get("a"), Some(&ExtVarType::Bool));
}

#[test]
fn infer_ext_var_types_int() {
    let vars = vec!["count".to_string()];
    let inits = vec![("count".to_string(), "5".to_string())];
    let m = infer_ext_var_types(&vars, &inits);
    assert_eq!(m.get("count"), Some(&ExtVarType::Int));
}

#[test]
fn infer_ext_var_types_object_default() {
    // No init expression → Object.
    let vars = vec!["foo".to_string()];
    let m = infer_ext_var_types(&vars, &[]);
    assert_eq!(m.get("foo"), Some(&ExtVarType::Object));
}

#[test]
fn infer_ext_var_types_object_for_complex_init() {
    let vars = vec!["bar".to_string()];
    let inits = vec![("bar".to_string(), "self.getX()".to_string())];
    let m = infer_ext_var_types(&vars, &inits);
    assert_eq!(m.get("bar"), Some(&ExtVarType::Object));
}

/// Bug §3.7 from docs/codebase_analysis.md: a float init like `0.5` currently
/// classifies as `Int` because the `s.parse::<f64>().is_ok()` arm in
/// `infer_ext_var_types` returns `ExtVarType::Int` instead of a non-Int
/// variant. The fix lands in Phase 2; this test goes from ignored to passing
/// when it does. (Remove `#[ignore]` after the fix.)
#[test]
#[ignore = "documents bug §3.7; remove ignore after Phase 2 fix lands"]
fn infer_ext_var_types_float_not_int() {
    let vars = vec!["scale".to_string()];
    let inits = vec![("scale".to_string(), "0.5".to_string())];
    let m = infer_ext_var_types(&vars, &inits);
    assert_ne!(m.get("scale"), Some(&ExtVarType::Int),
        "float init like 0.5 must not classify as Int; got: {:?}", m.get("scale"));
}

// ─── wrap_persistent_state ───────────────────────────────────────────────

#[test]
fn wrap_persistent_state_int_inc_dec_set_get() {
    let mut var_types = BTreeMap::new();
    var_types.insert("counter".to_string(), ExtVarType::Int);
    let input = "self.counter = 5;\n\
        self.counter++;\n\
        self.counter--;\n\
        var n = self.counter;\n";
    let out = wrap_persistent_state(input, &var_types);
    assert!(out.contains("counter.set(5);"), "= → .set(); got: {}", out);
    assert!(out.contains("counter.inc()"), "++ → .inc(); got: {}", out);
    assert!(out.contains("counter.dec()"), "-- → .dec(); got: {}", out);
    assert!(out.contains("counter.get()"), "read → .get(); got: {}", out);
}

#[test]
fn wrap_persistent_state_object_no_inc_dec() {
    // ExtVarType::Object doesn't get .inc / .dec rewrites.
    let mut var_types = BTreeMap::new();
    var_types.insert("foo".to_string(), ExtVarType::Object);
    let input = "self.foo++;\n";
    let out = wrap_persistent_state(input, &var_types);
    // Without the int rewrite, `self.foo++` falls through to the read pass
    // (and stays mostly intact, possibly `foo.get()++`). Either way, no
    // `.inc()` should appear.
    assert!(!out.contains(".inc()"),
        "Object vars don't get .inc(); got: {}", out);
}

// ─── fix_intangibility_pairs ─────────────────────────────────────────────

#[test]
fn intangibility_pair_rewritten_to_apply_global_body_status() {
    // The header parser only matches the canonical `function <name>__frame<N>() {`
    // shape used by entity frame scripts; not the truncated test fixture I had
    // before. Use a realistic form.
    let input = "function a__frame3() {\n\
        \tself.setIntangibility(true);\n\
        }\n\
        function a__frame15() {\n\
        \tself.setIntangibility(false);\n\
        }\n";
    let out = fix_intangibility_pairs(input);
    // The true → false pair should collapse to applyGlobalBodyStatus.
    assert!(out.contains("applyGlobalBodyStatus(BodyStatus.INTANGIBLE,"),
        "paired setIntangibility should collapse to applyGlobalBodyStatus; got:\n{}", out);
    // Duration = 15 - 3 = 12.
    assert!(out.contains("BodyStatus.INTANGIBLE, 12"),
        "duration should be (false_frame - true_frame) = 12; got:\n{}", out);
    // The false-side call should be gone (replaced with a comment marker).
    assert!(!out.lines().any(|l| l.trim() == "self.setIntangibility(false);"),
        "matching false() call should be removed; got:\n{}", out);
}

#[test]
fn intangibility_unpaired_false_surfaces_marker() {
    // A bare setIntangibility(false) with no preceding true → surface as ssf2-only.
    let input = "function a__frame0() {\n\
        \tself.setIntangibility(false);\n\
        }\n";
    let out = fix_intangibility_pairs(input);
    assert!(out.contains("[SSF2-only: setIntangibility]"),
        "unpaired false should be flagged; got:\n{}", out);
}

// ─── translate_ssf2_to_fm — end-to-end ordering checks ───────────────────

#[test]
fn translate_full_pipeline_order_isready_then_doubling_then_rename() {
    // isReady guard wraps a hitStun field. Pipeline must:
    //   1. strip the guard (so the hitStun field is visible)
    //   2. double hitStun: 10 → 20 (BEFORE the rename)
    //   3. rename hitStun: → hitstop: (literal replacement)
    // Result: a top-level `hitstop: 20`, no `if (SSF2API.isReady()`.
    let input = "function a__frame0() {\n\
        \tif (SSF2API.isReady()) {\n\
        \t\tself.updateHitboxStats(1, { hitStun: 10 });\n\
        \t}\n\
        }\n";
    let out = translate_ssf2_to_fm(input);
    assert!(!out.contains("SSF2API.isReady()"),
        "guard should be stripped; got:\n{}", out);
    // hitStun is in commands.jsonc :: frame_params (isframe: true) AND in
    // replacements as `hitStun:` → `hitstop:`. Both must run.
    assert!(out.contains("hitstop: 20"),
        "hitStun: 10 should be doubled then renamed to hitstop: 20; got:\n{}", out);
}

#[test]
fn translate_full_pipeline_self_self_collapse() {
    // SSF2 decompiled output is full of `self.self.` chains. The literal
    // replacement table collapses them to `self.`.
    let input = "self.self.endAttack();\n";
    let out = translate_ssf2_to_fm(input);
    assert!(out.contains("self.endAnimation()"),
        "self.self.endAttack → self.endAnimation; got: {}", out);
    assert!(!out.contains("self.self"),
        "self.self should be gone; got: {}", out);
}

#[test]
fn translate_full_pipeline_attach_effect_via_full_translate() {
    // attach_effect runs as part of the full pipeline.
    let input = "self.attachEffect(\"global_dust_blast\");\n";
    let out = translate_ssf2_to_fm(input);
    assert!(out.contains("match.createVfx("),
        "attachEffect should be rewritten to createVfx; got: {}", out);
    assert!(out.contains("GlobalVfx."),
        "global_dust_blast should route via GlobalVfx; got: {}", out);
}

#[test]
fn translate_pipeline_idempotent_on_already_translated() {
    // Running the pipeline twice on FM code should give the same output as
    // running it once — this catches passes that aren't idempotent.
    let input = "self.self.endAttack();\n";
    let once = translate_ssf2_to_fm(input);
    let twice = translate_ssf2_to_fm(&once);
    assert_eq!(once, twice,
        "pipeline must be idempotent on already-translated input;\nonce:  {}\ntwice: {}",
        once, twice);
}
