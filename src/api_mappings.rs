/// SSF2 → Fraymakers API mapping table.
///
/// SSF2 (ActionScript 3) uses a MovieClip timeline model with SSF2API/SSF2Character
/// wrappers. Fraymakers uses Haxe with Entity/Character/GameObject class hierarchy.
///
/// This module provides:
///   1. Method-level mappings (SSF2 call → FM equivalent)
///   2. Property mappings (SSF2 property access → FM getter/setter)
///   3. Constant mappings (SSF2 state constants → FM CState/etc)
///   4. Pattern-level transformations (multi-statement → single FM call)

use std::collections::BTreeMap;

// ─── Legacy / TODO Mappings ───────────────────────────────────────────────────
//
// The five `build_*_map` functions below + `load_api_methods_json` are
// LEGACY tables from before the JSONC mapping system. They have no current
// callers anywhere in the workspace. They are KEPT (not deleted) until
// every entry has been confirmed to have a JSONC home — most do, a few
// may carry a note or special-case that hasn't been migrated.
//
// To migrate an entry: confirm the SSF2→FM mapping is present in
// `mappings/commands.jsonc` (replacements / call_splits / passthrough /
// ssf2_only) or `mappings/character/*.jsonc`, then delete the entry here.
// Once a whole map is empty, delete the function and its supporting types.
//
// Tracking: docs/codebase_analysis.md §2.1.

// (ssf2_receiver, ssf2_method) → (fm_receiver, fm_method, note)
// receiver = "" means static/global, "self" means self.self in SSF2 → self in FM

#[derive(Debug, Clone)]
pub struct MethodMapping {
    pub fm_receiver: &'static str,    // "" = same receiver, "self" = entity self
    pub fm_method: &'static str,
    pub arg_transform: ArgTransform,
    pub note: &'static str,
}

#[derive(Debug, Clone)]
pub enum ArgTransform {
    /// Pass args through unchanged
    Identity,
    /// Drop all args
    NoArgs,
    /// Remap arg indices: e.g. [1, 0] means swap first two
    Reorder(Vec<usize>),
    /// First N args only
    TakeFirst(usize),
    /// Custom transformation tag (handled in code)
    Custom(&'static str),
}

// TODO: migrate every entry below to `mappings/commands.jsonc` (mostly
// `replacements`; a few may need `regex_replacements` or `ssf2_only`), then
// remove this function and the MethodMapping / ArgTransform types it uses.
// Tracking: docs/codebase_analysis.md §2.1.
#[allow(dead_code)]
pub fn build_method_map() -> BTreeMap<(&'static str, &'static str), MethodMapping> {
    let mut m = BTreeMap::new();
    let id = ArgTransform::Identity;

    // ── SSF2Character → FM Character/Entity ──────────────────────────────

    // Movement & State
    m.insert(("", "endAttack"), MethodMapping {
        fm_receiver: "self", fm_method: "endAnimation",
        arg_transform: ArgTransform::NoArgs,
        note: "SSF2 endAttack() → FM endAnimation()",
    });
    m.insert(("", "setState"), MethodMapping {
        fm_receiver: "self", fm_method: "toState",
        arg_transform: id.clone(),
        note: "SSF2 setState(state) → FM toState(state)",
    });
    m.insert(("", "inState"), MethodMapping {
        fm_receiver: "self", fm_method: "inState",
        arg_transform: id.clone(),
        note: "",
    });
    m.insert(("", "isFacingRight"), MethodMapping {
        fm_receiver: "self", fm_method: "isFacingRight",
        arg_transform: ArgTransform::NoArgs,
        note: "",
    });
    m.insert(("", "isFacingLeft"), MethodMapping {
        fm_receiver: "self", fm_method: "isFacingLeft",
        arg_transform: ArgTransform::NoArgs,
        note: "",
    });

    // Position / velocity
    m.insert(("", "getX"), MethodMapping {
        fm_receiver: "self", fm_method: "getX",
        arg_transform: ArgTransform::NoArgs, note: "",
    });
    m.insert(("", "getY"), MethodMapping {
        fm_receiver: "self", fm_method: "getY",
        arg_transform: ArgTransform::NoArgs, note: "",
    });
    m.insert(("", "setX"), MethodMapping {
        fm_receiver: "self", fm_method: "setX",
        arg_transform: id.clone(), note: "",
    });
    m.insert(("", "setY"), MethodMapping {
        fm_receiver: "self", fm_method: "setY",
        arg_transform: id.clone(), note: "",
    });
    m.insert(("", "setXSpeed"), MethodMapping {
        fm_receiver: "self", fm_method: "setXVelocity",
        arg_transform: id.clone(), note: "SSF2 setXSpeed → FM setXVelocity",
    });
    m.insert(("", "setYSpeed"), MethodMapping {
        fm_receiver: "self", fm_method: "setYVelocity",
        arg_transform: id.clone(), note: "SSF2 setYSpeed → FM setYVelocity",
    });
    m.insert(("", "getXSpeed"), MethodMapping {
        fm_receiver: "self", fm_method: "getXVelocity",
        arg_transform: ArgTransform::NoArgs, note: "",
    });
    m.insert(("", "getYSpeed"), MethodMapping {
        fm_receiver: "self", fm_method: "getYVelocity",
        arg_transform: ArgTransform::NoArgs, note: "",
    });

    // Hitbox / Attack
    m.insert(("", "updateAttackBoxStats"), MethodMapping {
        fm_receiver: "self", fm_method: "updateHitboxStats",
        arg_transform: id.clone(),
        note: "SSF2 updateAttackBoxStats(id, stats) → FM updateHitboxStats(id, stats)",
    });
    m.insert(("", "refreshAttackID"), MethodMapping {
        fm_receiver: "self", fm_method: "reactivateHitboxes",
        arg_transform: ArgTransform::NoArgs,
        note: "SSF2 refreshAttackID → FM reactivateHitboxes",
    });

    // Animation
    m.insert(("", "gotoAndPlay"), MethodMapping {
        fm_receiver: "self", fm_method: "playAnimation",
        arg_transform: id.clone(),
        note: "SSF2 gotoAndPlay(label) → FM playAnimation(name)",
    });
    m.insert(("", "gotoAndStop"), MethodMapping {
        fm_receiver: "self", fm_method: "playAnimation",
        arg_transform: id.clone(),
        note: "SSF2 gotoAndStop(label) → FM playAnimation(name) /* TODO: stop after */",
    });
    m.insert(("", "play"), MethodMapping {
        fm_receiver: "self", fm_method: "resume",
        arg_transform: ArgTransform::NoArgs,
        note: "",
    });
    m.insert(("", "stop"), MethodMapping {
        fm_receiver: "self", fm_method: "pause",
        arg_transform: ArgTransform::NoArgs,
        note: "",
    });

    // Controls
    m.insert(("", "getControls"), MethodMapping {
        fm_receiver: "self", fm_method: "getHeldControls",
        arg_transform: ArgTransform::NoArgs,
        note: "SSF2 getControls → FM getHeldControls",
    });
    m.insert(("", "getPressedControls"), MethodMapping {
        fm_receiver: "self", fm_method: "getPressedControls",
        arg_transform: ArgTransform::NoArgs, note: "",
    });

    // Grabbing
    m.insert(("", "grab"), MethodMapping {
        fm_receiver: "self", fm_method: "attemptGrab",
        arg_transform: id.clone(),
        note: "SSF2 grab(target) → FM attemptGrab(foe)",
    });
    m.insert(("", "shootOutOpponent"), MethodMapping {
        fm_receiver: "self", fm_method: "releaseAllCharacters",
        arg_transform: ArgTransform::NoArgs,
        note: "",
    });

    // Projectile
    m.insert(("", "fireProjectile"), MethodMapping {
        fm_receiver: "self", fm_method: "/* TODO: spawn projectile */",
        arg_transform: id.clone(),
        note: "SSF2 fireProjectile needs manual conversion — FM uses CustomGameObject",
    });

    // Scale
    m.insert(("", "getScaleX"), MethodMapping {
        fm_receiver: "self", fm_method: "getScaleX",
        arg_transform: ArgTransform::NoArgs, note: "",
    });
    m.insert(("", "getScaleY"), MethodMapping {
        fm_receiver: "self", fm_method: "getScaleY",
        arg_transform: ArgTransform::NoArgs, note: "",
    });
    m.insert(("", "setScaleX"), MethodMapping {
        fm_receiver: "self", fm_method: "setScaleX",
        arg_transform: id.clone(), note: "",
    });
    m.insert(("", "setScaleY"), MethodMapping {
        fm_receiver: "self", fm_method: "setScaleY",
        arg_transform: id.clone(), note: "",
    });

    // Visibility
    m.insert(("", "setVisible"), MethodMapping {
        fm_receiver: "self", fm_method: "setVisible",
        arg_transform: id.clone(), note: "",
    });

    // Damage
    m.insert(("", "getDamage"), MethodMapping {
        fm_receiver: "self", fm_method: "getDamage",
        arg_transform: ArgTransform::NoArgs, note: "",
    });
    m.insert(("", "addDamage"), MethodMapping {
        fm_receiver: "self", fm_method: "addDamage",
        arg_transform: id.clone(), note: "",
    });

    // Events
    m.insert(("", "addEventListener"), MethodMapping {
        fm_receiver: "self", fm_method: "addEventListener",
        arg_transform: id.clone(), note: "Event types need remapping",
    });
    m.insert(("", "removeEventListener"), MethodMapping {
        fm_receiver: "self", fm_method: "removeEventListener",
        arg_transform: id.clone(), note: "",
    });

    // ── SSF2API static methods → FM equivalents ──────────────────────────

    m.insert(("SSF2API", "print"), MethodMapping {
        fm_receiver: "", fm_method: "Engine.log",
        arg_transform: id.clone(),
        note: "SSF2API.print → Engine.log",
    });
    m.insert(("SSF2API", "random"), MethodMapping {
        fm_receiver: "", fm_method: "Random.getFloat",
        arg_transform: ArgTransform::Custom("random_0_1"),
        note: "SSF2API.random() → Random.getFloat(0, 1)",
    });
    m.insert(("SSF2API", "randomInteger"), MethodMapping {
        fm_receiver: "", fm_method: "Random.getInt",
        arg_transform: id.clone(),
        note: "SSF2API.randomInteger(min,max) → Random.getInt(min,max)",
    });
    m.insert(("SSF2API", "getElapsedFrames"), MethodMapping {
        fm_receiver: "", fm_method: "Engine.getElapsedFrames",
        arg_transform: ArgTransform::NoArgs,
        note: "",
    });
    m.insert(("SSF2API", "isReady"), MethodMapping {
        fm_receiver: "", fm_method: "true /* SSF2API.isReady always true in FM */",
        arg_transform: ArgTransform::NoArgs,
        note: "Guard check — FM is always ready",
    });
    m.insert(("SSF2API", "playSound"), MethodMapping {
        fm_receiver: "self", fm_method: "/* TODO: playSound */",
        arg_transform: id.clone(),
        note: "SSF2API.playSound → FM AudioClip or entity sfx",
    });
    m.insert(("SSF2API", "stopSound"), MethodMapping {
        fm_receiver: "", fm_method: "/* TODO: stopSound */",
        arg_transform: id.clone(), note: "",
    });
    m.insert(("SSF2API", "shakeCamera"), MethodMapping {
        fm_receiver: "", fm_method: "Camera.shake",
        arg_transform: id.clone(),
        note: "SSF2API.shakeCamera(intensity) → Camera.shake(...)",
    });
    m.insert(("SSF2API", "lightFlash"), MethodMapping {
        fm_receiver: "", fm_method: "/* TODO: lightFlash */",
        arg_transform: id.clone(), note: "No direct FM equivalent",
    });
    m.insert(("SSF2API", "getCharacter"), MethodMapping {
        fm_receiver: "", fm_method: "/* TODO: getCharacter */",
        arg_transform: id.clone(), note: "FM uses Match.getCharacters()",
    });
    m.insert(("SSF2API", "getCharacters"), MethodMapping {
        fm_receiver: "", fm_method: "Match.getCharacters",
        arg_transform: ArgTransform::NoArgs, note: "",
    });
    m.insert(("SSF2API", "attachEffect"), MethodMapping {
        fm_receiver: "", fm_method: "/* TODO: Vfx.spawn */",
        arg_transform: id.clone(),
        note: "SSF2API.attachEffect → FM Vfx system",
    });

    m
}

// ─── Property Mappings ────────────────────────────────────────────────────────
// SSF2 property name → (FM getter, FM setter)

// TODO: migrate every entry below to `mappings/commands.jsonc :: replacements`
// (e.g. `.x = ` → `.setX(`, `.alpha = ` → `.setAlpha(`), then remove.
// Tracking: docs/codebase_analysis.md §2.1.
#[allow(dead_code)]
pub fn build_property_map() -> BTreeMap<&'static str, (&'static str, &'static str)> {
    let mut m = BTreeMap::new();

    // SSF2 → FM property mappings
    m.insert("x",           ("getX()",          "setX"));
    m.insert("y",           ("getY()",          "setY"));
    m.insert("scaleX",      ("getScaleX()",     "setScaleX"));
    m.insert("scaleY",      ("getScaleY()",     "setScaleY"));
    m.insert("alpha",       ("getAlpha()",      "setAlpha"));
    m.insert("visible",     ("getVisible()",    "setVisible"));
    m.insert("rotation",    ("getRotation()",   "setRotation"));
    m.insert("currentFrame",("getCurrentFrame()", "playFrame"));

    m
}

// ─── SSF2 State → FM CState Mappings ──────────────────────────────────────────

// TODO: migrate every entry below to `mappings/commands.jsonc :: replacements`
// (e.g. `CState.IDLE` → `CState.STAND`) or a new `state_map` section if a
// dedicated table makes sense, then remove.
// Tracking: docs/codebase_analysis.md §2.1.
#[allow(dead_code)]
pub fn build_state_map() -> BTreeMap<&'static str, &'static str> {
    let mut m = BTreeMap::new();

    // SSF2 CState / character states → FM CState constants
    // SSF2 uses numeric values; FM uses CState.CONSTANT
    m.insert("IDLE",            "CState.STAND");
    m.insert("STAND",          "CState.STAND");
    m.insert("WALK",           "CState.WALK_LOOP");
    m.insert("RUN",            "CState.RUN");
    m.insert("DASH",           "CState.DASH");
    m.insert("JUMP",           "CState.JUMP_IN");
    m.insert("JUMP_SQUAT",    "CState.JUMP_SQUAT");
    m.insert("FALL",           "CState.FALL");
    m.insert("FALL_SPECIAL",   "CState.FALL_SPECIAL");
    m.insert("LAND",           "CState.LAND");
    m.insert("CROUCH",         "CState.CROUCH_LOOP");
    m.insert("SHIELD",         "CState.SHIELD_LOOP");

    // Attacks
    m.insert("JAB",            "CState.JAB");
    m.insert("JAB1",           "CState.JAB");
    m.insert("JAB2",           "CState.JAB");
    m.insert("JAB3",           "CState.JAB");
    m.insert("DASH_ATTACK",    "CState.DASH_ATTACK");
    m.insert("TILT_FORWARD",   "CState.TILT_FORWARD");
    m.insert("TILT_UP",        "CState.TILT_UP");
    m.insert("TILT_DOWN",      "CState.TILT_DOWN");
    m.insert("STRONG_FORWARD", "CState.STRONG_FORWARD_ATTACK");
    m.insert("STRONG_UP",      "CState.STRONG_UP_ATTACK");
    m.insert("STRONG_DOWN",    "CState.STRONG_DOWN_ATTACK");
    m.insert("AERIAL_NEUTRAL", "CState.AERIAL_NEUTRAL");
    m.insert("AERIAL_FORWARD", "CState.AERIAL_FORWARD");
    m.insert("AERIAL_BACK",    "CState.AERIAL_BACK");
    m.insert("AERIAL_UP",      "CState.AERIAL_UP");
    m.insert("AERIAL_DOWN",    "CState.AERIAL_DOWN");
    m.insert("SPECIAL_NEUTRAL","CState.SPECIAL_NEUTRAL");
    m.insert("SPECIAL_SIDE",   "CState.SPECIAL_SIDE");
    m.insert("SPECIAL_UP",     "CState.SPECIAL_UP");
    m.insert("SPECIAL_DOWN",   "CState.SPECIAL_DOWN");
    m.insert("GRAB",           "CState.GRAB");
    m.insert("GRAB_HOLD",      "CState.GRAB_HOLD");
    m.insert("THROW_FORWARD",  "CState.THROW_FORWARD");
    m.insert("THROW_BACK",     "CState.THROW_BACK");
    m.insert("THROW_UP",       "CState.THROW_UP");
    m.insert("THROW_DOWN",     "CState.THROW_DOWN");

    // Defense
    m.insert("SHIELD_IN",     "CState.SHIELD_IN");
    m.insert("SHIELD_OUT",    "CState.SHIELD_OUT");
    m.insert("ROLL",          "CState.ROLL");
    m.insert("SPOT_DODGE",    "CState.SPOT_DODGE");
    m.insert("PARRY",         "CState.PARRY_IN");

    // Hurt / KO
    m.insert("HURT",           "CState.HURT_LIGHT");
    m.insert("TUMBLE",         "CState.TUMBLE");
    m.insert("KO",             "CState.KO");

    // Ledge
    m.insert("LEDGE_IN",      "CState.LEDGE_IN");
    m.insert("LEDGE_LOOP",    "CState.LEDGE_LOOP");
    m.insert("LEDGE_CLIMB",   "CState.LEDGE_CLIMB");
    m.insert("LEDGE_ATTACK",  "CState.LEDGE_ATTACK");
    m.insert("LEDGE_ROLL",    "CState.LEDGE_ROLL");
    m.insert("LEDGE_JUMP",    "CState.LEDGE_JUMP");

    m
}

// ─── SSF2 Event → FM GameObjectEvent Mappings ─────────────────────────────────

// TODO: migrate every entry below to `mappings/commands.jsonc :: replacements`
// (e.g. `GameObjectEvent.HIT` → `GameObjectEvent.HIT_DEALT`) — the
// `addEventListener(SSF2_EVENT.X, …)` calls are already rewritten via the
// literal table where applicable. Once parity is confirmed, remove.
// Tracking: docs/codebase_analysis.md §2.1.
#[allow(dead_code)]
pub fn build_event_map() -> BTreeMap<&'static str, &'static str> {
    let mut m = BTreeMap::new();

    m.insert("STATE_CHANGE",     "GameObjectEvent.LINK_FRAMES");
    m.insert("HIT",             "GameObjectEvent.HIT_DEALT");
    m.insert("HIT_RECEIVED",    "GameObjectEvent.HIT_RECEIVED");
    m.insert("LAND",            "GameObjectEvent.LAND");
    m.insert("GRAB",            "GameObjectEvent.GRAB_DEALT");
    m.insert("GRAB_RECEIVED",   "GameObjectEvent.GRAB_RECEIVED");
    m.insert("SHIELD_HIT",     "GameObjectEvent.SHIELD_HIT_DEALT");
    m.insert("HITSTOP_START",   "GameObjectEvent.ENTER_HITSTOP");
    m.insert("HITSTOP_END",     "GameObjectEvent.EXIT_HITSTOP");
    m.insert("LEFT_GROUND",     "GameObjectEvent.LEFT_GROUND");

    m
}

// ─── SSF2 Hitbox Property → FM HitboxStats Property Mappings ──────────────────

// TODO: this overlaps with `mappings/character/hitbox_stats.jsonc :: fields`
// (which already covers damage / angle / baseKnockback / knockbackGrowth /
// hitstop / hitstun). The remaining entries (shieldDamage, priority,
// hitSound, refreshRate, selfHitStun→selfHitstop) need to land either in
// hitbox_stats.jsonc as new `fm_field` rows or in commands.jsonc :: ssf2_only.
// Once that's done, remove this function.
// Tracking: docs/codebase_analysis.md §2.1.
#[allow(dead_code)]
pub fn build_hitbox_prop_map() -> BTreeMap<&'static str, &'static str> {
    let mut m = BTreeMap::new();

    m.insert("damage",         "damage");
    m.insert("direction",      "angle");
    m.insert("power",          "baseKnockback");
    m.insert("kbGrowth",       "knockbackGrowth");
    m.insert("kbConstant",     "baseKnockback");
    m.insert("hitStun",        "hitstun");
    m.insert("selfHitStun",    "selfHitstop");
    m.insert("shieldDamage",   "shieldDamageMultiplier");
    m.insert("priority",       "/* TODO: no FM equivalent for priority */");
    m.insert("hitSound",       "attackStrength");
    m.insert("refreshRate",    "/* TODO: no FM equivalent for refreshRate */");

    m
}

// ─── SSF2 "self.self" Pattern ──────────────────────────────────────────────────
// In decompiled SSF2 sub-MC code, "self.self" refers to the character instance.
// In Fraymakers, the Script.hx `self` already IS the character/entity.
// So "self.self.endAttack()" → "self.endAnimation()"
//
// This is handled at the text level in the post-processor.

/// Remove SSF2 readiness guard if-blocks that are always-true in Fraymakers.
///
/// SSF2 wraps initialization code with guards like:
///   `if (SSF2API.isReady()) { ... }`
///   `if (self && SSF2API.isReady()) { ... }`
///   `if (SSF2API.isReady() && self) { ... }`
///
/// In Fraymakers Script.hx, `self` is always valid and the API is always ready,
/// so these guards are unnecessary. This function removes the if-wrapper and
/// dedents the body by one level, effectively inlining the body.
pub fn remove_readiness_guards(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if is_isready_guard(trimmed) {
            // Count braces to find the matching close-brace of this if block
            let mut depth: i32 = 0;
            let mut found_open = false;
            let mut body_start = i;
            let mut j = i;

            'scan: while j < lines.len() {
                for ch in lines[j].chars() {
                    match ch {
                        '{' => {
                            depth += 1;
                            if !found_open {
                                found_open = true;
                                body_start = j;
                            }
                        }
                        '}' => {
                            depth -= 1;
                            if found_open && depth == 0 {
                                break 'scan;
                            }
                        }
                        _ => {}
                    }
                }
                j += 1;
            }

            if found_open && depth == 0 {
                // Emit body lines (body_start+1 .. j), each dedented by one tab
                // Add a comment so the reader knows a guard was stripped
                out.push("// [FM] isReady guard removed — always true in Fraymakers".to_string());
                for body_line in &lines[body_start + 1..j] {
                    out.push(strip_one_tab(body_line).to_string());
                }
                i = j + 1; // skip the closing `}` line
                continue;
            } else {
                // Couldn't parse — emit original
                out.push(line.to_string());
                i += 1;
                continue;
            }
        }

        out.push(line.to_string());
        i += 1;
    }

    // Re-join, preserving trailing newline if original had one
    let mut joined = out.join("\n");
    if code.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

/// Returns true if a trimmed line is a SSF2 readiness guard if-statement.
fn is_isready_guard(trimmed: &str) -> bool {
    // Must start with `if (` or `if(`
    if !trimmed.starts_with("if (") && !trimmed.starts_with("if(") {
        return false;
    }
    // Must contain SSF2API.isReady() — the canonical readiness check
    trimmed.contains("SSF2API.isReady()")
}

/// Strip one tab (or 4 spaces) from the start of a line.
fn strip_one_tab(line: &str) -> &str {
    if line.starts_with('\t') {
        &line[1..]
    } else if line.starts_with("    ") {
        &line[4..]
    } else {
        line
    }
}

/// Apply text-level SSF2→FM API translations to decompiled Haxe code.
/// This is a post-processing step run on the output of the decompiler.
///
/// Pipeline order:
///   1. `remove_readiness_guards`      — strip `SSF2API.isReady()` guards
///   2. `double_frame_counts`           — 30→60 fps scaling on frame-typed
///                                        fields BEFORE any rename or split,
///                                        so SSF2 field names still match
///   3. `apply_call_splits`             — fan out SSF2 umbrella calls
///                                        (e.g. updateAttackStats) to their
///                                        FM target methods using
///                                        `call_splits` from commands.jsonc
///   4. literal `replacements`          — text find→replace pairs (ordered;
///                                        SSF2 field names like `hitStun:`
///                                        get renamed to FM `hitstop:` here)
///   5. `regex_replacements`            — regex-based renames
///   6. `strip_last_frame_end_animation` — drop redundant terminal calls
///   7. `comment_out_unknown_calls`     — `ssf2_only` markers + log unknowns
pub fn translate_ssf2_to_fm(code: &str) -> String {
    let mut result = remove_readiness_guards(code);

    // 30→60fps frame doubling on SSF2-named frame fields (hitStun, hitLag,
    // refreshRate, etc.). Runs BEFORE call_splits and the rename pass so
    // SSF2 field names still match `frame_params` entries. Previously this
    // ran only in entity_gen; moved here so Script.hx + ext-methods get it
    // too (otherwise frame fields inside updateHitboxStats calls in
    // Script.hx kept SSF2 30fps values).
    result = double_frame_counts(&result);

    // Fan out SSF2 umbrella calls into one or more FM calls per the
    // `call_splits` table. Runs BEFORE the rename pass so the rename
    // pass can finish field-name conversions on the emitted FM calls.
    result = apply_call_splits(&result);

    // Literal SSF2 → Fraymakers find/replace pairs. Order matters.
    for r in &crate::mappings::api_commands().replacements {
        result = result.replace(&r.from, &r.to);
    }

    // Regex replacements — arg-dropping, arg-aware dispatch, etc.
    for re in regex_replacement_cache() {
        let next = re.regex.replace_all(&result, re.replacement.as_str());
        if matches!(&next, std::borrow::Cow::Owned(_)) {
            result = next.into_owned();
        }
    }

    // Context-aware self.attachEffect(...) → match.createVfx(...) rewrite.
    // Needs the per-character effect→primary-animation map (set up by
    // haxe_gen::generate via `with_effect_animations`); a static regex
    // can't pick the right animation name for effects that split into
    // multiple FrameLabel-derived animations.
    result = rewrite_attach_effect_calls(&result);

    result = strip_last_frame_end_animation(&result);
    result = comment_out_unknown_calls(&result);
    result
}

thread_local! {
    /// Per-character map of `<effect_name>` → first/primary animation name
    /// in its emitted `.entity` file. Set at the top of `haxe_gen::generate`
    /// via `with_effect_animations` and cleared on return. Used only by
    /// `rewrite_attach_effect_calls`. Empty map = no effects discovered
    /// (or we're outside a `with_effect_animations` scope) → all calls
    /// fall through to the `ssf2_only` marker.
    static EFFECT_PRIMARY_ANIMS: std::cell::RefCell<BTreeMap<String, String>>
        = std::cell::RefCell::new(BTreeMap::new());
}

/// RAII guard that installs an effect→primary-animation map on
/// construction and clears it on drop. Use at the top of a
/// per-character generation pass so any `translate_ssf2_to_fm` call
/// made on this thread (Character.entity frame scripts, Script.hx,
/// per-attack scripts) gets context-aware attachEffect rewriting.
pub struct EffectAnimGuard;

impl EffectAnimGuard {
    pub fn install(map: BTreeMap<String, String>) -> Self {
        EFFECT_PRIMARY_ANIMS.with(|cell| { *cell.borrow_mut() = map; });
        EffectAnimGuard
    }
}

impl Drop for EffectAnimGuard {
    fn drop(&mut self) {
        EFFECT_PRIMARY_ANIMS.with(|cell| { cell.borrow_mut().clear(); });
    }
}

/// Rewrite `self.attachEffect("name")` and `self.attachEffect("name", { props })`
/// to `match.createVfx(new VfxStats({ spriteContent: …, animation: "<primary>", <translated props> }), self)`.
///
/// Animation-name resolution:
///   - If `name` is in the thread_local map (a local effect we extracted),
///     use that effect's primary animation name — picks the right name
///     even when the entity has multiple FrameLabel-derived animations.
///   - If `name` is not in the map, fall back to `"active"`. Most
///     unknown references are FM-global effects (`global_dust_blast`,
///     `global_spark`, `itempickup_effect`, …) whose entities live in
///     the engine's standard library; `"active"` is the project-wide
///     default animation name for VFX entities we emit, so any local
///     fallback also lands on a valid animation. Worst case the
///     animation is wrong → runtime warning; never worse than the
///     original SSF2 call which we couldn't translate at all.
///
/// Per-prop translation:
///   - The 2-arg form parses the `{…}` block with paren-aware comma
///     splitting (so `Random.getFloat(0, 1)` is one value, not two).
///     Each `key: value` is routed through the `attach_effect_props`
///     mapping table in commands.jsonc, which supports direct renames,
///     1→N expansions (e.g. `parentLock` → `relativeWith` +
///     `resizeWith` + `flipWith`), and explicit `todo` notes for keys
///     with no clean FM equivalent (`syncHitStun`, `loop`, `behind`, …).
///   - Unmapped props emit a `// TODO: <key>: <value> — note` line
///     above the call, preserving the original value alongside the
///     reason. The call line itself never carries free-form comments.
pub fn rewrite_attach_effect_calls(code: &str) -> String {
    static RE_LINE_2ARG: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static RE_ANY_2ARG: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static RE_1ARG: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    // Two 2-arg patterns. The first is line-anchored (`(?m)^([ \t]*)…`)
    // and captures the line's indent so the emitted `// TODO:` lines
    // can mirror it. The second is the mid-line fallback (assignments
    // like `self.effect = self.attachEffect(…)` or calls that follow
    // an inline comment); it can't safely inject TODO lines without
    // breaking the surrounding expression, so unmapped props are
    // dropped silently in that case. The two passes run in order, so
    // the line-anchored form is preferred whenever it can match.
    let re_line_2arg = RE_LINE_2ARG.get_or_init(|| {
        regex::Regex::new(r#"(?m)^([ \t]*)\bself\.attachEffect\(\s*"([^"]*)"\s*,\s*\{([^{}]*)\}\s*\)"#).unwrap()
    });
    let re_any_2arg = RE_ANY_2ARG.get_or_init(|| {
        regex::Regex::new(r#"\bself\.attachEffect\(\s*"([^"]*)"\s*,\s*\{([^{}]*)\}\s*\)"#).unwrap()
    });
    let re_1arg = RE_1ARG.get_or_init(|| {
        regex::Regex::new(r#"\bself\.attachEffect\(\s*"([^"]*)"\s*\)"#).unwrap()
    });

    // Build the translated `new VfxStats({…})` literal from the parsed
    // props. Returns the bag body (no surrounding `{}`) plus a list of
    // TODO notes for unmapped props.
    let build_bag = |name: &str, props_block: &str| -> (String, Vec<String>) {
        let mut fm_fields: Vec<(String, String)> = Vec::new();
        let mut todo_lines: Vec<String> = Vec::new();
        for (key, value) in parse_prop_bag(props_block) {
            translate_attach_effect_prop(&key, &value, &mut fm_fields, &mut todo_lines);
        }
        let mut bag = build_vfx_head(name);
        for (k, v) in &fm_fields {
            bag.push_str(&format!(", {}: {}", k, v));
        }
        (bag, todo_lines)
    };

    // Pass 1: line-anchored 2-arg form — emit TODOs above the call.
    let after_line_2arg = re_line_2arg.replace_all(code, |caps: &regex::Captures| {
        let indent = &caps[1];
        let name = &caps[2];
        let props_block = &caps[3];
        let (bag, todo_lines) = build_bag(name, props_block);

        let mut out = String::new();
        for note in &todo_lines {
            out.push_str(indent);
            out.push_str("// TODO: ");
            out.push_str(note);
            out.push('\n');
        }
        out.push_str(indent);
        out.push_str(&format!(
            "match.createVfx(new VfxStats({{ {bag} }}), self)"
        ));
        out
    });

    // Pass 2: mid-line 2-arg fallback — translate inline. Embedded
    // calls can't host above-line TODOs (would break the surrounding
    // expression), so any unmapped props ride along as a trailing
    // `/* TODO: … */` block-comment instead. That keeps the warning
    // visible without altering line topology.
    let after_any_2arg = re_any_2arg.replace_all(&after_line_2arg, |caps: &regex::Captures| {
        let name = &caps[1];
        let props_block = &caps[2];
        let (bag, todos) = build_bag(name, props_block);
        let call = format!("match.createVfx(new VfxStats({{ {bag} }}), self)");
        if todos.is_empty() {
            call
        } else {
            format!("{} /* TODO: {} */", call, todos.join(" | TODO: "))
        }
    });

    // 1-arg form — no props to translate; the head alone is the bag.
    let after_1arg = re_1arg.replace_all(&after_any_2arg, |caps: &regex::Captures| {
        let name = &caps[1];
        let head = build_vfx_head(name);
        format!("match.createVfx(new VfxStats({{ {head} }}), self)")
    });
    after_1arg.into_owned()
}

/// Build the `spriteContent: …, animation: …` head of the VfxStats bag
/// for an effect named `name`. Shape selection:
///   - If `name` matches `global_vfx_map`, emit the Fraymakers global
///     shape:
///       `spriteContent: "global::vfx.vfx", animation: GlobalVfx.<C>`
///     The constant is unquoted because it's an accessor on the
///     `GlobalVfx` class, not a string literal — the engine resolves
///     the underlying animation name at runtime.
///   - Otherwise emit the per-character shape that resolves `name`
///     against the local effect entity we generated:
///       `spriteContent: self.getResource().getContent("name"),
///        animation: "<primary or active fallback>"`
///     where the animation name comes from `EFFECT_PRIMARY_ANIMS` (set
///     up per-character by `haxe_gen::generate`), or `"active"` when
///     the name isn't a known local effect.
pub fn build_vfx_head(name: &str) -> String {
    let cfg = crate::mappings::api_commands();
    if let Some(constant) = cfg.global_vfx_map.get(name) {
        format!(
            "spriteContent: \"global::vfx.vfx\", animation: GlobalVfx.{c}",
            c = constant,
        )
    } else {
        let anim = EFFECT_PRIMARY_ANIMS.with(|cell| {
            cell.borrow().get(name).cloned().unwrap_or_else(|| "active".to_string())
        });
        format!(
            "spriteContent: self.getResource().getContent(\"{n}\"), animation: \"{a}\"",
            n = name, a = anim,
        )
    }
}

/// Split a `{ key1: val1, key2: val2, … }` body (without the braces) into
/// `(key, value)` pairs, respecting parentheses so values like
/// `Random.getFloat(0, 1)` aren't split on the inner comma. Whitespace
/// around keys and values is trimmed.
fn parse_prop_bag(body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut depth: i32 = 0;
    let mut cur = String::new();
    for ch in body.chars() {
        match ch {
            '(' | '[' => { depth += 1; cur.push(ch); }
            ')' | ']' => { depth -= 1; cur.push(ch); }
            ',' if depth == 0 => {
                if let Some((k, v)) = cur.split_once(':') {
                    let k = k.trim().to_string();
                    let v = v.trim().to_string();
                    if !k.is_empty() && !v.is_empty() {
                        out.push((k, v));
                    }
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        if let Some((k, v)) = cur.split_once(':') {
            let k = k.trim().to_string();
            let v = v.trim().to_string();
            if !k.is_empty() && !v.is_empty() {
                out.push((k, v));
            }
        }
    }
    out
}

/// Route one SSF2 prop through `attach_effect_props` in commands.jsonc.
/// On a successful mapping, push `(fm_field, value)` pairs into
/// `fm_fields`. On an explicit `todo` or an unknown key, push a
/// `// TODO:` note onto `todo_lines`. Both arrays are caller-owned so
/// the rewrite can emit them in source order.
fn translate_attach_effect_prop(
    key: &str,
    value: &str,
    fm_fields: &mut Vec<(String, String)>,
    todo_lines: &mut Vec<String>,
) {
    use crate::mappings::AttachEffectPropMapping as M;
    let cfg = crate::mappings::api_commands();
    match cfg.attach_effect_props.get(key) {
        Some(M::Simple(fm)) => {
            fm_fields.push((fm.clone(), value.to_string()));
        }
        Some(M::Detailed { target, expand_to, todo }) => {
            if !expand_to.is_empty() {
                for fm in expand_to {
                    fm_fields.push((fm.clone(), value.to_string()));
                }
            } else if let Some(fm) = target {
                fm_fields.push((fm.clone(), value.to_string()));
            }
            if let Some(note) = todo {
                // Even routed entries can carry a porter-facing caveat
                // (matches call_splits's todo semantics).
                todo_lines.push(format!("{}: {} — {}", key, value, note));
            } else if expand_to.is_empty() && target.is_none() {
                // Entry exists but routes nowhere → treat as unmapped.
                todo_lines.push(format!("{}: {} — entry present but no target/expand_to", key, value));
            }
        }
        None => {
            todo_lines.push(format!("{}: {} — no mapping in attach_effect_props", key, value));
        }
    }
}

/// Strip `self.endAnimation()` calls that appear alone on a line inside the
/// last-numbered frame function of each animation group.
///
/// Pattern: a function named `<anim>__frame<N>` where no higher-numbered frame
/// function exists for that animation in the same script. In that case endAnimation
/// is redundant — FM already ends the animation when the final frame completes.
pub fn strip_last_frame_end_animation(code: &str) -> String {
    // Collect all (anim_prefix, max_frame_num) seen across all functions
    let mut max_frames: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    let frame_fn_re = simple_frame_fn_re();

    for (prefix, frame_num) in iter_frame_fns(code, &frame_fn_re) {
        let entry = max_frames.entry(prefix.to_string()).or_insert(0);
        if frame_num > *entry {
            *entry = frame_num;
        }
    }

    // Now process line by line: when inside a last-frame function and we see
    // a line that is ONLY `self.endAnimation();`, replace with a comment.
    let lines: Vec<&str> = code.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut in_last_frame = false;

    for line in &lines {
        let trimmed = line.trim();
        // Check if this line opens a last-frame function
        if let Some((prefix, frame_num)) = parse_frame_fn_header(trimmed, &frame_fn_re) {
            in_last_frame = max_frames.get(&prefix).copied() == Some(frame_num);
        } else if trimmed == "}" {
            // Leaving any function
            in_last_frame = false;
        }

        if in_last_frame && trimmed == "self.endAnimation();" {
            // Strip it — FM animation ends naturally on the last frame
            out.push(format!(
                "{}// [FM] endAnimation removed — redundant on last frame",
                &line[..line.len() - trimmed.len()]
            ));
        } else {
            out.push(line.to_string());
        }
    }

    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

// Simple &str-based frame function pattern matching (avoids regex dep)
struct FrameFnPattern;
fn simple_frame_fn_re() -> FrameFnPattern { FrameFnPattern }

fn iter_frame_fns<'a>(code: &'a str, _pat: &FrameFnPattern) -> impl Iterator<Item=(String, u32)> + 'a {
    code.lines().filter_map(|line| parse_frame_fn_header(line.trim(), &FrameFnPattern))
}

fn parse_frame_fn_header(trimmed: &str, _pat: &FrameFnPattern) -> Option<(String, u32)> {
    // Match: function <prefix>__frame<N>()
    if !trimmed.starts_with("function ") { return None; }
    let rest = &trimmed["function ".len()..];
    let paren = rest.find('(')?;
    let name = &rest[..paren];
    // Must contain __frame
    let frame_pos = name.rfind("__frame")?;
    let prefix = name[..frame_pos].to_string();
    let frame_str = &name[frame_pos + "__frame".len()..];
    let frame_num: u32 = frame_str.parse().ok()?;
    Some((prefix, frame_num))
}

/// Fix `setIntangibility` pairs across the full assembled Script.hx.
///
/// SSF2 pattern:
///   `anim__frame1`:  self.setIntangibility(true);
///   `anim__frame20`: self.setIntangibility(false);
///
/// FM equivalent: a single `applyGlobalBodyStatus(BodyStatus.INTANGIBLE, N)` call
/// where N = (false_frame - true_frame). The false call is removed entirely.
///
/// This is a full-script pass (not per-frame-body) because we need to look
/// across multiple frame functions to compute the duration.
pub fn fix_intangibility_pairs(full_script: &str) -> String {
    // 1. Collect all (anim_prefix, frame_num, line_index) for setIntangibility calls
    let lines: Vec<&str> = full_script.lines().collect();
    let pat = FrameFnPattern;

    // Track current function context while scanning
    let mut current_prefix: Option<String> = None;
    let mut current_frame: u32 = 0;

    // (prefix, frame, line_idx, is_true)
    let mut calls: Vec<(String, u32, usize, bool)> = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if let Some((pfx, fnum)) = parse_frame_fn_header(trimmed, &pat) {
            current_prefix = Some(pfx);
            current_frame = fnum;
        }
        if trimmed == "}" {
            // Don't clear prefix here — frame functions can contain inner braces
        }
        if let Some(ref pfx) = current_prefix {
            if trimmed == "self.setIntangibility(true);" {
                calls.push((pfx.clone(), current_frame, idx, true));
            } else if trimmed == "self.setIntangibility(false);" {
                calls.push((pfx.clone(), current_frame, idx, false));
            }
        }
    }

    // 2. Pair each true with its nearest following false in the same prefix.
    // Sort calls by (prefix, frame_num) so numeric order is used, not textual.
    calls.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut replacements: Vec<(usize, String)> = Vec::new(); // (line_idx, new_line)
    let mut removed: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (i, (pfx, true_frame, true_line, is_true)) in calls.iter().enumerate() {
        if !is_true { continue; }
        // Find the next false call in the same animation (by frame number, already sorted)
        if let Some((_, false_frame, false_line, _)) = calls[i+1..].iter()
            .find(|(p, _, _, is_t)| p == pfx && !is_t)
        {
            let duration = false_frame.saturating_sub(*true_frame);
            let indent = &lines[*true_line][..lines[*true_line].len() - lines[*true_line].trim_start().len()];
            replacements.push((
                *true_line,
                format!("{}self.applyGlobalBodyStatus(BodyStatus.INTANGIBLE, {});", indent, duration),
            ));
            removed.insert(*false_line);
        } else {
            // No matching false — use 0 as duration and leave a TODO
            let indent = &lines[*true_line][..lines[*true_line].len() - lines[*true_line].trim_start().len()];
            replacements.push((
                *true_line,
                format!("{}self.applyGlobalBodyStatus(BodyStatus.INTANGIBLE, 0 /*TODO: calculate duration*/); //[FM] no matching setIntangibility(false) found", indent),
            ));
        }
    }

    // 3. Collect all unmatched false calls (no preceding true in this script)
    let mut unmatched_false: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (_, _, false_line, is_true) in &calls {
        if !is_true && !removed.contains(false_line) {
            // This false was never consumed by a true
            unmatched_false.insert(*false_line);
        }
    }

    // 4. Rebuild, applying replacements and removing/commenting paired false calls
    let replace_map: std::collections::HashMap<usize, String> =
        replacements.into_iter().collect();

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (idx, line) in lines.iter().enumerate() {
        if removed.contains(&idx) {
            // Paired false — duration already encoded in the true
            out.push(format!(
                "{}// [FM] setIntangibility(false) removed — duration encoded above",
                &line[..line.len() - line.trim_start().len()]
            ));
        } else if unmatched_false.contains(&idx) {
            // Unpaired false — intangibility set outside this script (e.g. by state entry)
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push(format!(
                "{}// [SSF2-only: setIntangibility] {} //TODO: intangibility set outside this script",
                indent, line.trim()
            ));
        } else if let Some(replacement) = replace_map.get(&idx) {
            out.push(replacement.clone());
        } else {
            out.push(line.to_string());
        }
    }

    let mut joined = out.join("\n");
    if full_script.ends_with('\n') { joined.push('\n'); }
    joined
}

/// Comment out entire statements containing SSF2-only method calls that have
/// no Fraymakers equivalent. The list of names comes from
/// `commands.jsonc :: ssf2_only` (see crate::mappings).
///
/// Each matched line is replaced with `// [SSF2-only: NAME] <original>` so it
/// won't compile but stays readable. Lines that already start with `//` are
/// left alone (idempotent).
///
/// Side-effect: when log level is at least `debug`, any `.NAME(` call whose
/// name doesn't appear in ANY section of commands.jsonc (replacements,
/// passthrough_fm_apis, ssf2_only, or frame_params) is logged once globally.
/// This is what makes `passthrough_fm_apis` functional — listing a name
/// there suppresses it from this "unknown call" stream. Run with
/// `RUST_LOG=debug` to surface new SSF2 calls that need attention.
/// Split SSF2 umbrella calls (currently `updateAttackStats`) into one or
/// more grouped FM calls per the `call_splits` table in commands.jsonc.
///
/// Source pattern: `<receiver>.<method>({ k1: v1, k2: v2, … })`. The
/// receiver may be any chain (`self`, `match.getCamera()`,
/// `self.lightningBolts[…]`) — it's preserved verbatim on every emitted
/// call. Calls whose argument isn't an object literal are left alone.
///
/// Per-field semantics (drawn from CallSplit.fields):
///   - mapping `"updateAnimationStats.leaveGroundCancel"`  →
///     groups under target `updateAnimationStats` with FM field
///     `leaveGroundCancel: <value>`
///   - source fields absent from the mapping become a `// TODO: …` line
///   - all fields sharing a target method merge into ONE call with
///     `, `-joined pairs, no trailing comma
///   - source line indentation propagates to every emitted line
pub fn apply_call_splits(code: &str) -> String {
    let cfg = crate::mappings::api_commands();
    if cfg.call_splits.is_empty() { return code.to_string(); }
    let mut current = code.to_string();
    for (source_method, split) in &cfg.call_splits {
        current = apply_one_call_split(&current, source_method, split);
    }
    current
}

fn apply_one_call_split(code: &str, method: &str, split: &crate::mappings::CallSplit) -> String {
    let needle = format!(".{}(", method);
    let bytes = code.as_bytes();
    let mut result = String::with_capacity(code.len());
    let mut cursor = 0usize;

    while cursor < bytes.len() {
        let Some(rel) = code[cursor..].find(&needle) else {
            result.push_str(&code[cursor..]);
            break;
        };
        let dot_pos = cursor + rel;
        // Don't replace inside a single-line comment — if the line up to
        // dot_pos contains a `//` (outside of a string), skip this match.
        if line_is_commented_at(code, dot_pos) {
            result.push_str(&code[cursor..=dot_pos]);
            cursor = dot_pos + 1;
            continue;
        }
        let paren_open = dot_pos + needle.len() - 1;
        let receiver_start = find_receiver_start(code, dot_pos);
        let receiver = &code[receiver_start..dot_pos];

        let Some(paren_close) = find_matching_close(code, paren_open) else {
            // Unclosed call — give up on this match, advance past `.`.
            result.push_str(&code[cursor..=dot_pos]);
            cursor = dot_pos + 1;
            continue;
        };

        let args = code[paren_open + 1..paren_close].trim();
        if !args.starts_with('{') || !args.ends_with('}') {
            // Not an object-literal argument — leave the call alone.
            result.push_str(&code[cursor..=paren_close]);
            cursor = paren_close + 1;
            continue;
        }
        let body = &args[1..args.len() - 1];
        let fields = parse_object_fields(body);

        let line_start = code[..receiver_start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let indent: String = code[line_start..receiver_start].chars()
            .take_while(|c| c.is_whitespace() && *c != '\n').collect();

        // Consume trailing `;` and any spaces between `)` and `;`.
        let mut full_end = paren_close + 1;
        while full_end < bytes.len() && bytes[full_end] == b' ' { full_end += 1; }
        if full_end < bytes.len() && bytes[full_end] == b';' { full_end += 1; }

        let rendered = render_call_split(receiver, &fields, split, &indent);
        // When the render is empty (every field was skipped) AND the only
        // text between cursor and the call is leading whitespace on this
        // line, also consume the trailing newline so the line vanishes
        // cleanly instead of leaving a blank-with-indent.
        if rendered.is_empty()
            && code[..receiver_start].rfind('\n').map(|i| i + 1).unwrap_or(0) >= cursor
            && code[cursor..receiver_start].chars().all(|c| c.is_whitespace() && c != '\n')
        {
            // Skip up to and including the next '\n' after the call.
            let mut skip_end = full_end;
            while skip_end < bytes.len() && bytes[skip_end] != b'\n' {
                if !(bytes[skip_end] as char).is_whitespace() { break; }
                skip_end += 1;
            }
            if skip_end < bytes.len() && bytes[skip_end] == b'\n' {
                skip_end += 1;
                cursor = skip_end;
                continue;
            }
        }
        result.push_str(&code[cursor..receiver_start]);
        result.push_str(&rendered);
        cursor = full_end;
    }
    result
}

/// Per-field decision when resolving a CallSplit field against a source.
enum FieldOutcome {
    /// Route into the named target method's grouped call. `todo_note`,
    /// if present, also emits an inline `// TODO:` comment for the user
    /// to verify the value.
    Route { target_method: String, fm_field: String, value: String, todo_note: Option<String> },
    /// Emit only as a TODO comment line — no routed call.
    Todo { src_name: String, src_value: String, note: Option<String> },
    /// Drop the field entirely (no emission, no TODO) — used by
    /// `skip_if_value` matches.
    Skip,
}

fn render_call_split(
    receiver: &str,
    source_fields: &[(String, String)],
    split: &crate::mappings::CallSplit,
    indent: &str,
) -> String {
    use crate::mappings::CallSplitFieldMapping as M;
    use std::collections::BTreeMap;

    let parse_target = |s: &str| -> Option<(String, String)> {
        s.rfind('.').map(|d| (s[..d].to_string(), s[d + 1..].to_string()))
    };

    let mut outcomes: Vec<FieldOutcome> = Vec::with_capacity(source_fields.len());
    for (field_name, value) in source_fields {
        let trimmed = value.trim();
        let outcome = match split.fields.get(field_name) {
            None => FieldOutcome::Todo {
                src_name: field_name.clone(),
                src_value: value.clone(),
                note: None,
            },
            Some(M::Simple(s)) => match parse_target(s) {
                Some((tm, ff)) => FieldOutcome::Route {
                    target_method: tm, fm_field: ff, value: value.clone(), todo_note: None,
                },
                None => FieldOutcome::Todo {
                    src_name: field_name.clone(),
                    src_value: value.clone(),
                    note: Some("malformed mapping in commands.jsonc".to_string()),
                },
            },
            Some(M::Detailed { target, value_map, skip_if_value, todo }) => {
                if let Some(sv) = skip_if_value {
                    if trimmed == sv.trim() {
                        FieldOutcome::Skip
                    } else {
                        resolve_detailed(field_name, value, trimmed, target, value_map, todo, &parse_target)
                    }
                } else {
                    resolve_detailed(field_name, value, trimmed, target, value_map, todo, &parse_target)
                }
            }
        };
        outcomes.push(outcome);
    }

    // Group Route outcomes by target method (preserve first-seen order),
    // collect TODO lines (from both Todo outcomes and Route todo_notes).
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut todo_lines: Vec<String> = Vec::new();

    for o in outcomes {
        match o {
            FieldOutcome::Route { target_method, fm_field, value, todo_note } => {
                if !groups.contains_key(&target_method) {
                    order.push(target_method.clone());
                }
                if let Some(n) = &todo_note {
                    todo_lines.push(format!("// TODO: {}: {}", fm_field, n));
                }
                groups.entry(target_method).or_default().push((fm_field, value));
            }
            FieldOutcome::Todo { src_name, src_value, note } => {
                let n = note.as_deref().unwrap_or("no FM mapping in call_splits");
                todo_lines.push(format!("// TODO: {}: {} — {}", src_name, src_value, n));
            }
            FieldOutcome::Skip => {}
        }
    }

    let mut lines: Vec<String> = Vec::new();
    lines.extend(todo_lines);
    for target_method in &order {
        let pairs = groups.get(target_method).unwrap();
        let joined: Vec<String> = pairs.iter().map(|(f, v)| format!("{}: {}", f, v)).collect();
        lines.push(format!("{}.{}({{ {} }});", receiver, target_method, joined.join(", ")));
    }

    if lines.is_empty() {
        // Everything skipped (or source object was empty). Drop the
        // source line entirely so the output has no stray no-op.
        return String::new();
    }
    lines.join(&format!("\n{}", indent))
}

/// Resolve a Detailed field mapping when `skip_if_value` didn't apply.
/// Handles the value_map lookup + target routing.
fn resolve_detailed(
    field_name: &str,
    raw_value: &str,
    trimmed_value: &str,
    target: &Option<String>,
    value_map: &std::collections::BTreeMap<String, String>,
    todo: &Option<String>,
    parse_target: &dyn Fn(&str) -> Option<(String, String)>,
) -> FieldOutcome {
    if !value_map.is_empty() {
        match value_map.get(trimmed_value) {
            Some(mapped_value) => match target.as_deref().and_then(|s| parse_target(s)) {
                Some((tm, ff)) => FieldOutcome::Route {
                    target_method: tm, fm_field: ff, value: mapped_value.clone(),
                    todo_note: todo.clone(),
                },
                None => FieldOutcome::Todo {
                    src_name: field_name.to_string(),
                    src_value: raw_value.to_string(),
                    note: todo.clone().or_else(|| Some("value_map matched but no target defined".to_string())),
                },
            },
            None => FieldOutcome::Todo {
                src_name: field_name.to_string(),
                src_value: raw_value.to_string(),
                note: todo.clone().or_else(|| Some("value not in value_map".to_string())),
            },
        }
    } else if let Some(t) = target {
        match parse_target(t) {
            Some((tm, ff)) => FieldOutcome::Route {
                target_method: tm, fm_field: ff, value: raw_value.to_string(),
                todo_note: todo.clone(),
            },
            None => FieldOutcome::Todo {
                src_name: field_name.to_string(),
                src_value: raw_value.to_string(),
                note: todo.clone().or_else(|| Some("malformed target".to_string())),
            },
        }
    } else {
        FieldOutcome::Todo {
            src_name: field_name.to_string(),
            src_value: raw_value.to_string(),
            note: todo.clone(),
        }
    }
}

/// Parse the inside of an object literal (everything between `{` and `}`,
/// exclusive) into `(field_name, value_expr)` pairs. Tracks brace/paren/
/// bracket depth + string state so nested objects, function calls in
/// values, and quoted commas don't break field splitting.
fn parse_object_fields(body: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = body.chars().collect();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == ',') { i += 1; }
        if i >= chars.len() { break; }
        // Quoted key form: `"foo bar": value`. Accept the same set of
        // characters as inside any string literal — read until the closing
        // quote (honouring backslash escapes), then proceed to the `:`
        // separator as with bare identifier keys.
        let field_name: String = if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            i += 1;
            let mut name = String::new();
            while i < chars.len() && chars[i] != quote {
                if chars[i] == '\\' {
                    i += 1;
                    if i >= chars.len() { break; }
                    name.push(chars[i]);
                    i += 1;
                    continue;
                }
                name.push(chars[i]);
                i += 1;
            }
            if i < chars.len() && chars[i] == quote { i += 1; }
            name
        } else {
            let name_start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
            chars[name_start..i].iter().collect()
        };
        if field_name.is_empty() { break; }
        while i < chars.len() && chars[i].is_whitespace() { i += 1; }
        if i >= chars.len() || chars[i] != ':' { break; }
        i += 1;
        while i < chars.len() && chars[i].is_whitespace() { i += 1; }
        let val_start = i;
        let mut depth = 0i32;
        let mut in_str: Option<char> = None;
        while i < chars.len() {
            let c = chars[i];
            if let Some(q) = in_str {
                if c == '\\' { i += 1; if i < chars.len() { i += 1; } continue; }
                if c == q { in_str = None; }
            } else {
                match c {
                    '"' | '\'' => in_str = Some(c),
                    '{' | '[' | '(' => depth += 1,
                    '}' | ']' | ')' => depth -= 1,
                    ',' if depth == 0 => break,
                    _ => {}
                }
            }
            i += 1;
        }
        let value: String = chars[val_start..i].iter().collect::<String>().trim().to_string();
        if !value.is_empty() {
            out.push((field_name, value));
        }
    }
    out
}

/// Walk backward from a `.<method>(` position to find the start of the
/// receiver expression. Accepts identifier chars, `.`, balanced `[…]`
/// and `(…)` (for method-chain / index expressions).
fn find_receiver_start(code: &str, dot_pos: usize) -> usize {
    let bytes = code.as_bytes();
    let mut i = dot_pos;
    loop {
        if i == 0 { return 0; }
        let prev = bytes[i - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'.' {
            i -= 1;
        } else if prev == b']' {
            match find_matching_open(code, i - 1, b'[', b']') {
                Some(open) => i = open,
                None => return i,
            }
        } else if prev == b')' {
            match find_matching_open(code, i - 1, b'(', b')') {
                Some(open) => i = open,
                None => return i,
            }
        } else {
            return i;
        }
    }
}

/// Walk forward from an opening bracket/paren/brace to find the matching
/// close. Tracks string state so quoted delimiters don't disturb the count.
fn find_matching_close(code: &str, open_pos: usize) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut depth = 1i32;
    let mut i = open_pos + 1;
    let mut in_str: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = in_str {
            if c == b'\\' { i += 2; continue; }
            if c == q { in_str = None; }
        } else {
            match c {
                b'"' | b'\'' => in_str = Some(c),
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => {
                    depth -= 1;
                    if depth == 0 { return Some(i); }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn find_matching_open(code: &str, close_pos: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut depth = 1i32;
    let mut i = close_pos;
    while i > 0 {
        i -= 1;
        if bytes[i] == close { depth += 1; }
        else if bytes[i] == open {
            depth -= 1;
            if depth == 0 { return Some(i); }
        }
    }
    None
}

/// True if `pos` sits after a `//` on the same line (i.e. inside a
/// single-line comment). Used to skip call_splits matches that landed
/// inside comments.
fn line_is_commented_at(code: &str, pos: usize) -> bool {
    let line_start = code[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_prefix = &code[line_start..pos];
    let mut in_str: Option<char> = None;
    let chars: Vec<char> = line_prefix.chars().collect();
    let mut j = 0;
    while j < chars.len() {
        let c = chars[j];
        if let Some(q) = in_str {
            if c == '\\' { j += 1; if j < chars.len() { j += 1; } continue; }
            if c == q { in_str = None; }
        } else {
            match c {
                '"' | '\'' => in_str = Some(c),
                '/' if j + 1 < chars.len() && chars[j + 1] == '/' => return true,
                _ => {}
            }
        }
        j += 1;
    }
    false
}

/// Inferred type for an SSF2 instance variable, picked from `ext_var_inits`.
/// Drives which `self.make<Kind>(default)` wrapper is used when rewriting
/// the var declaration, and whether `.inc()`/`.dec()` are emitted for
/// increment/decrement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtVarType { Bool, Int, Object }

/// Classify each ext_var as Bool / Int / Object based on its initial-value
/// expression in `ext_var_inits`. Vars with no init expression — or whose
/// init isn't a clean literal — fall back to Object (which can hold any
/// value; only the optional `.inc()` / `.dec()` shorthands need Int).
pub fn infer_ext_var_types(
    ext_vars: &[String],
    ext_var_inits: &[(String, String)],
) -> std::collections::BTreeMap<String, ExtVarType> {
    let init_lookup: std::collections::BTreeMap<&String, &String> =
        ext_var_inits.iter().map(|(n, e)| (n, e)).collect();
    let mut out = std::collections::BTreeMap::new();
    for name in ext_vars {
        let kind = match init_lookup.get(name).map(|s| s.trim()) {
            Some("true") | Some("false") => ExtVarType::Bool,
            // Integer literals → Int (`5`, `-3`, `0`). The wrapper exposes
            // `.inc()` / `.dec()` and `.get()` / `.set()` semantics.
            Some(s) if s.parse::<i64>().is_ok() => ExtVarType::Int,
            // Non-integer numeric literals (e.g. `0.5`, `1.5e2`) must NOT
            // classify as Int — `self.makeInt(0)` would round on `.set(0.5)`
            // and `.inc()`/`.dec()` would step by 1 rather than the source-
            // logical step. Fall through to Object, which is the safe FM
            // wrapper for any non-bool / non-int value.
            _ => ExtVarType::Object,
        };
        out.insert(name.clone(), kind);
    }
    out
}

/// Wrap SSF2 instance-variable accesses into Fraymakers' persistent-state
/// wrappers. Pattern (from Fraymakers/character-template Script.hx):
///
///   ```text
///   var counter = self.makeInt(0);   // declaration
///   counter.set(5);                  // assignment
///   counter.get();                   // read
///   counter.inc();                   // ++
///   counter.dec();                   // --
///   ```
///
/// Affects only references to names in `var_types`; other `self.X` calls
/// (FM API methods, fields) are left alone. Safe to run multiple times —
/// the rewrites are idempotent because they no longer contain `self.<name>`.
///
/// Called on Script.hx and on each translated entity frame-script body
/// with the same type map, so cross-file references stay consistent.
pub fn wrap_persistent_state(
    code: &str,
    var_types: &std::collections::BTreeMap<String, ExtVarType>,
) -> String {
    if var_types.is_empty() { return code.to_string(); }
    let mut result = code.to_string();
    for (name, kind) in var_types {
        // Increment / decrement (Int only — Object/Bool don't have .inc/.dec).
        if *kind == ExtVarType::Int {
            let re_inc = regex::Regex::new(&format!(r"\bself\.{}\+\+", regex::escape(name))).unwrap();
            result = re_inc.replace_all(&result, format!("{}.inc()", name)).into_owned();
            let re_dec = regex::Regex::new(&format!(r"\bself\.{}--", regex::escape(name))).unwrap();
            result = re_dec.replace_all(&result, format!("{}.dec()", name)).into_owned();
        }
        // Assignment (`self.foo = X` → `foo.set(X)`). Hand-rolled scan
        // (instead of a regex) so multi-line RHS — e.g. an inline closure
        // literal `function () { … };` that the decompiler produces — is
        // captured correctly. We track brace + string state to find the
        // `;` at depth-zero rather than the first `;` we see.
        result = rewrite_persistent_assignment(&result, name);
        // Read (`self.foo` not followed by `=`, `++`, `--`). Field/method
        // chains (`self.foo.bar`) get `foo.get().bar` which is the
        // desired form. The assignment pass above has already rewritten
        // `self.foo = …`, so this pass is read-only — no `self.foo =`
        // remains for it to mis-match.
        let read_re = regex::Regex::new(&format!(r"\bself\.{}\b", regex::escape(name))).unwrap();
        result = read_re.replace_all(&result, format!("{}.get()", name)).into_owned();
    }
    result
}

/// Rewrite every top-level `self.<name> = <rhs>;` to `<name>.set(<rhs>);`.
/// `<rhs>` may span newlines and contain nested `{ … }` / `( … )` /
/// `[ … ]` / strings — the scan tracks bracket depth and string state and
/// only matches the depth-zero `;` as the terminator.
///
/// Matches:
/// - `self.foo = expr;`                    (single-line)
/// - `self.foo = function () { … };`       (multi-line closure)
/// - `self.foo = { a: 1, b: 2 };`          (object literal)
/// - `self.foo  =  Random.getFloat(0, 1);` (whitespace + nested call)
///
/// Skips:
/// - `self.foo == bar`   (the `==` after the `=` is consumed by the next-
///   char check; we require a single `=` followed by something other than `=`)
/// - `self.foo += 1`     (`+=` doesn't match the bare `=` boundary)
/// - `self.foobar = …`   (token boundary check: char before `self.foo` must
///   not be alphanumeric/underscore; char after `foo` must not be either)
fn rewrite_persistent_assignment(code: &str, name: &str) -> String {
    let needle = format!("self.{}", name);
    let bytes = code.as_bytes();
    let nl = needle.len();
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0usize;

    while cursor < bytes.len() {
        // Find the next candidate occurrence of `self.<name>`.
        let rel = match code[cursor..].find(&needle) {
            Some(r) => r,
            None => { out.push_str(&code[cursor..]); break; }
        };
        let start = cursor + rel;

        // Token boundary on the LEFT — char before `self` must not be a word char.
        let lhs_ok = start == 0 || {
            let prev = bytes[start - 1];
            !(prev.is_ascii_alphanumeric() || prev == b'_')
        };
        // Token boundary on the RIGHT — char after the matched name must not
        // be a word char (otherwise `self.foo` is a prefix of `self.foobar`).
        let after_name = start + nl;
        let rhs_ok = after_name >= bytes.len() || {
            let next = bytes[after_name];
            !(next.is_ascii_alphanumeric() || next == b'_')
        };
        if !lhs_ok || !rhs_ok {
            out.push_str(&code[cursor..=start]);
            cursor = start + 1;
            continue;
        }

        // Skip optional whitespace, then require a single `=` not followed
        // by `=`. (Excludes `==`, `+=`, `-=`, etc. — `+=` doesn't reach here
        // because the byte before `=` would be `+`, not whitespace.)
        let mut p = after_name;
        while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t') { p += 1; }
        if p >= bytes.len() || bytes[p] != b'=' {
            out.push_str(&code[cursor..=start]);
            cursor = start + 1;
            continue;
        }
        if p + 1 < bytes.len() && bytes[p + 1] == b'=' {
            // `==` — comparison, not assignment. Skip this match.
            out.push_str(&code[cursor..=start]);
            cursor = start + 1;
            continue;
        }
        let eq_pos = p;

        // Find the depth-zero, outside-of-string `;`.
        let mut q = eq_pos + 1;
        let mut depth: i32 = 0;
        let mut in_str: Option<u8> = None;
        let mut end_semi: Option<usize> = None;
        while q < bytes.len() {
            let c = bytes[q];
            if let Some(quote) = in_str {
                if c == b'\\' { q = q.saturating_add(2); continue; }
                if c == quote { in_str = None; }
            } else {
                match c {
                    b'"' | b'\'' => in_str = Some(c),
                    b'{' | b'(' | b'[' => depth += 1,
                    b'}' | b')' | b']' => {
                        if depth == 0 { break; }
                        depth -= 1;
                    }
                    b';' if depth == 0 => { end_semi = Some(q); break; }
                    _ => {}
                }
            }
            q += 1;
        }
        let semi = match end_semi {
            Some(s) => s,
            None => {
                // Unterminated — give up on this site, advance past `self.`.
                out.push_str(&code[cursor..=start]);
                cursor = start + 1;
                continue;
            }
        };

        // Compose: keep up-to start, emit `<name>.set(<rhs>);`.
        out.push_str(&code[cursor..start]);
        // Strip surrounding whitespace from the RHS so `set(  x  )` doesn't appear.
        let rhs = code[eq_pos + 1..semi].trim();
        out.push_str(&format!("{}.set({});", name, rhs));
        cursor = semi + 1;
    }
    out
}

pub fn comment_out_unknown_calls(code: &str) -> String {
    let cfg = crate::mappings::api_commands();
    // Build the `.NAME(` match strings from the JSON ssf2_only list.
    let markers: Vec<(String, &str)> = cfg.ssf2_only.iter()
        .map(|e| (format!(".{}(", e.name), e.name.as_str()))
        .collect();

    let lines: Vec<&str> = code.lines().collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut ssf2_hits: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with("//") { out.push(line.to_string()); continue; }
        let hit = markers.iter().find(|(m, _)| line.contains(m));
        if let Some((_, name)) = hit {
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push(format!("{}// [SSF2-only: {}] {}", indent, name, trimmed));
            *ssf2_hits.entry((*name).to_string()).or_insert(0) += 1;
        } else {
            out.push(line.to_string());
        }
    }
    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }

    // Always accrue counts into the conversion log (the JSON file is written
    // unconditionally at the end of the run). The debug-level log line is
    // separately gated inside log_unknown_calls.
    if !ssf2_hits.is_empty() {
        let mut log = conversion_log().lock().unwrap();
        for (name, n) in ssf2_hits {
            *log.ssf2_only.entry(name).or_insert(0) += n;
        }
    }
    log_unknown_calls(&joined, cfg);
    joined
}

/// One regex_replacements entry, pre-compiled from the JSONC config.
struct CompiledRegexReplacement {
    regex: regex::Regex,
    replacement: String,
}

/// Pre-compile every `regex_replacements` entry once per process. Bad patterns
/// log a warning and are silently skipped, so a typo in commands.jsonc can't
/// break the conversion.
fn regex_replacement_cache() -> &'static [CompiledRegexReplacement] {
    static CACHE: std::sync::OnceLock<Vec<CompiledRegexReplacement>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let cfg = crate::mappings::api_commands();
        let mut out = Vec::with_capacity(cfg.regex_replacements.len());
        for r in &cfg.regex_replacements {
            match regex::Regex::new(&r.pattern) {
                Ok(re) => out.push(CompiledRegexReplacement {
                    regex: re,
                    replacement: r.replacement.clone(),
                }),
                Err(e) => log::warn!(
                    "[api_mappings] regex_replacements pattern failed to compile — \
                     pattern={:?} note={:?} error={}",
                    r.pattern, r.note, e
                ),
            }
        }
        out
    })
}

/// Per-character bookkeeping written to `conversion_log.json` at the end of a
/// run. `unknown` are calls that didn't appear in ANY commands.jsonc section
/// (genuine gaps); `ssf2_only` are calls we know lack a Fraymakers equivalent
/// and have intentionally commented out. The "already logged" set keeps each
/// unknown name's debug-log line down to one occurrence per process.
#[derive(Debug, Default, Clone)]
pub struct ConversionLog {
    pub unknown: std::collections::BTreeMap<String, usize>,
    pub ssf2_only: std::collections::BTreeMap<String, usize>,
    pub already_logged: std::collections::BTreeSet<String>,
}

fn conversion_log() -> &'static std::sync::Mutex<ConversionLog> {
    static LOG: std::sync::OnceLock<std::sync::Mutex<ConversionLog>> = std::sync::OnceLock::new();
    LOG.get_or_init(|| std::sync::Mutex::new(ConversionLog::default()))
}

/// Reset the conversion log — call at the start of each character so per-
/// character counts and the once-per-name dedup start from a clean slate.
pub fn reset_conversion_log() {
    *conversion_log().lock().unwrap() = ConversionLog::default();
}

/// Snapshot the current log (does not reset). Used at the end of a run to
/// write `conversion_log.json` next to the exported character.
pub fn snapshot_conversion_log() -> ConversionLog {
    conversion_log().lock().unwrap().clone()
}

/// Walk `code` for `.NAME(` call sites and log any name that isn't covered
/// by any of the four commands.jsonc sections. Locally-defined helper names
/// will appear here too — the log is a hint, not a strict gap report.
fn log_unknown_calls(code: &str, cfg: &crate::mappings::ApiCommands) {
    // Build the union of all known names across every section.
    let mut known: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let extract = |s: &str, out: &mut std::collections::BTreeSet<String>| {
        let bytes = s.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            // start of identifier?
            if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                { i += 1; }
                if i < bytes.len() && bytes[i] == b'(' {
                    out.insert(String::from_utf8_lossy(&bytes[start..i]).into_owned());
                }
            } else {
                i += 1;
            }
        }
    };
    for r in &cfg.replacements {
        extract(&r.from, &mut known);
        extract(&r.to,   &mut known);
    }
    for p in &cfg.passthrough_fm_apis { known.insert(p.name.clone()); }
    for o in &cfg.ssf2_only           { known.insert(o.name.clone()); }
    for f in &cfg.frame_params        { known.insert(f.name.clone()); }

    let bytes = code.as_bytes();
    let mut i = 0usize;
    let debug = log::log_enabled!(log::Level::Debug);
    let mut log_state = conversion_log().lock().unwrap();
    while i < bytes.len() {
        if bytes[i] == b'.'
            && i + 1 < bytes.len()
            && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
        {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_')
            { j += 1; }
            if j < bytes.len() && bytes[j] == b'(' {
                let name = String::from_utf8_lossy(&bytes[start..j]).into_owned();
                if !known.contains(&name) {
                    *log_state.unknown.entry(name.clone()).or_insert(0) += 1;
                    if debug && log_state.already_logged.insert(name.clone()) {
                        log::debug!(
                            "[api_mappings] unknown call '{}' — add to replacements / \
                             passthrough_fm_apis / ssf2_only in commands.jsonc as appropriate",
                            name
                        );
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
}

/// Load SSF2→FM method mappings from the JSON file at `mappings/api_methods.json`
/// relative to the project root. Falls back to empty map if file not found.
// TODO: this loader reads `mappings/api_methods.json`, which doesn't
// exist in the repo and isn't called from anywhere. The schema it parses
// (`{ "method_name": { "fm": "<replacement>" } }`) was superseded by
// `mappings/commands.jsonc :: replacements`. Confirm no out-of-tree
// caller depends on this signature, then remove.
// Tracking: docs/codebase_analysis.md §2.1.
#[allow(dead_code)]
pub fn load_api_methods_json(mappings_dir: &std::path::Path) -> Vec<(String, String)> {
    let path = mappings_dir.join("api_methods.json");
    let Ok(text) = std::fs::read_to_string(&path) else { return vec![]; };
    let mut pairs = Vec::new();
    // Simple JSON scan: extract "method_name": { "fm": "replacement" ... } pairs
    // We use a regex-free approach: scan for quoted keys and their fm values
    let mut pos = 0;
    let bytes = text.as_bytes();
    while pos < bytes.len() {
        // Find a key (method name) — bare string between quotes
        if let Some(key_start) = text[pos..].find('"') {
            let abs_ks = pos + key_start + 1;
            if let Some(key_end) = text[abs_ks..].find('"') {
                let key = &text[abs_ks..abs_ks + key_end];
                let after_key = abs_ks + key_end + 1;
                // Look for {"fm": "..."}  nearby
                if let Some(obj_start) = text[after_key..].find('{') {
                    let abs_os = after_key + obj_start;
                    if let Some(obj_end) = text[abs_os..].find('}') {
                        let obj = &text[abs_os..abs_os + obj_end + 1];
                        if let Some(fm_pos) = obj.find("\"fm\"") {
                            let after_fm = abs_os + fm_pos + 4;
                            if let Some(val_start) = text[after_fm..].find('"') {
                                let abs_vs = after_fm + val_start + 1;
                                if let Some(val_end) = text[abs_vs..].find('"') {
                                    let fm_val = &text[abs_vs..abs_vs + val_end];
                                    // Only include if key is a valid method name (no spaces, no slashes)
                                    if !key.contains(' ') && !key.contains('/') && !key.starts_with('_') {
                                        pairs.push((key.to_string(), fm_val.to_string()));
                                    }
                                    pos = abs_vs + val_end + 1;
                                    continue;
                                }
                            }
                        }
                        pos = abs_os + obj_end + 1;
                        continue;
                    }
                }
                pos = after_key;
                continue;
            }
        }
        break;
    }
    pairs
}

// ─── 30fps → 60fps frame-count scaling ───────────────────────────────────────
//
// SSF2 plays at 30fps, Fraymakers at 60fps. Numeric arguments in decompiled
// frame-script Haxe that represent frame counts must be doubled so the script
// runs at the same real time. Only specific, known frame-count fields are
// touched — speeds, positions, IDs, angles, repeat counts and multipliers are
// deliberately left alone.

/// Double the first non-negative integer literal that follows each occurrence
/// of `marker` (after optional spaces/tabs). Values `>= skip_at` are treated as
/// sentinels and left unchanged; non-literal arguments (expressions, negatives)
/// are skipped because no digits immediately follow the marker.
fn double_int_after_marker(code: &str, marker: &str, skip_at: i64) -> String {
    let mut out = String::with_capacity(code.len() + 16);
    let bytes = code.as_bytes();
    let mut i = 0usize;
    while i < code.len() {
        if code[i..].starts_with(marker) {
            out.push_str(marker);
            let mut j = i + marker.len();
            while j < code.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                out.push(bytes[j] as char);
                j += 1;
            }
            let start = j;
            while j < code.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > start {
                match code[start..j].parse::<i64>() {
                    Ok(n) if n < skip_at => out.push_str(&(n * 2).to_string()),
                    _ => out.push_str(&code[start..j]),
                }
            }
            i = j;
        } else {
            let ch = code[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Double the integer literal at positional argument `arg_idx` of every
/// `fn_name(...)` call. Arguments before the target are walked over with
/// bracket-depth tracking, so commas inside nested calls/arrays/objects don't
/// miscount. Non-literal and negative arguments are left unchanged; a literal
/// `>= skip_at` is treated as a sentinel and left unchanged.
fn double_call_arg(code: &str, fn_name: &str, arg_idx: usize, skip_at: i64) -> String {
    let marker = format!("{}(", fn_name);
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len() + 16);
    let mut cursor = 0usize;
    while let Some(rel) = code[cursor..].find(&marker) {
        let arg0 = cursor + rel + marker.len(); // start of argument 0
        // Walk to the start of argument `arg_idx`, depth-tracking brackets.
        let mut j = arg0;
        let mut depth = 0i32;
        let mut cur = 0usize;
        let mut bailed = false;
        while cur < arg_idx && j < code.len() {
            match bytes[j] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    if depth == 0 { bailed = true; break; } // call closed early
                    depth -= 1;
                }
                b',' if depth == 0 => cur += 1,
                _ => {}
            }
            j += 1;
        }
        // Copy verbatim up to the target argument's start.
        out.push_str(&code[cursor..j]);
        cursor = j;
        if bailed || cur < arg_idx { continue; } // argument not present
        // Skip leading whitespace, then double the integer literal if present.
        let mut k = j;
        while k < code.len() && (bytes[k] == b' ' || bytes[k] == b'\t') { k += 1; }
        out.push_str(&code[j..k]);
        let start = k;
        while k < code.len() && bytes[k].is_ascii_digit() { k += 1; }
        if k > start {
            match code[start..k].parse::<i64>() {
                Ok(n) if n < skip_at => out.push_str(&(n * 2).to_string()),
                _ => out.push_str(&code[start..k]),
            }
        }
        cursor = k;
    }
    out.push_str(&code[cursor..]);
    out
}

/// Double every command parameter flagged `isframe` in mappings/commands.json
/// — `createTimer` delays, `stancePlayFrame` indices, and object-literal
/// fields like `hitStun` / `refreshRate` / `chargetime_max`. This is the
/// single, data-driven path for frame-count scaling in decompiled frame-script
/// Haxe; which parameters are doubled is controlled entirely by the JSON.
pub fn double_frame_counts(code: &str) -> String {
    let mut out = code.to_string();
    for p in &crate::mappings::api_commands().frame_params {
        if !p.isframe { continue; }
        let skip_at = p.sentinel.unwrap_or(i64::MAX);
        out = match p.kind.as_str() {
            "field" => double_int_after_marker(&out, &format!("{}:", p.name), skip_at),
            "call"  => double_call_arg(&out, &p.name, p.arg, skip_at),
            _ => out, // unknown kind — leave untouched
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_translations() {
        let input = concat!(
            "self.self.endAttack();\n",
            "SSF2API.print(\"hello\");\n",
            "self.self.updateAttackBoxStats(1, { damage: 5, direction: 45, power: 10 });\n",
            "self.self.refreshAttackID();",
        );

        let output = translate_ssf2_to_fm(input);
        assert!(output.contains("self.endAnimation()"), "endAttack not translated");
        assert!(output.contains("Engine.log(\"hello\")"), "print not translated");
        assert!(output.contains("self.updateHitboxStats"), "updateAttackBoxStats not translated");
        assert!(output.contains("angle: 45"), "direction: not renamed to angle:");
        assert!(output.contains("baseKnockback: 10"), "power: not renamed");
        assert!(output.contains("self.reactivateHitboxes()"), "refreshAttackID not translated");
    }

    #[test]
    fn test_isready_guard_removed() {
        let input = concat!(
            "function a__frame0() {\n",
            "\tif (SSF2API.isReady()) {\n",
            "\t\tself.looped = false;\n",
            "\t\tself.playsound = SSF2API.random();\n",
            "\t}\n",
            "\treturn;\n",
            "}",
        );
        let output = translate_ssf2_to_fm(input);
        // The if-block wrapper should be gone
        assert!(!output.contains("SSF2API.isReady()"), "guard not removed");
        assert!(!output.contains("if ("), "if block should be stripped");
        // Body should be inlined (one tab dedented)
        assert!(output.contains("self.looped = false;"), "body not inlined");
        assert!(output.contains("Random.getFloat"), "random not translated");
    }

    #[test]
    fn test_self_and_isready_guard_removed() {
        let input = concat!(
            "\tif (self && SSF2API.isReady()) {\n",
            "\t\tself.x = 10;\n",
            "\t}",
        );
        let output = translate_ssf2_to_fm(input);
        assert!(!output.contains("SSF2API.isReady()"), "guard not removed");
        assert!(output.contains("self.x = 10;"), "body not inlined");
    }

    #[test]
    fn test_nested_isready_guard() {
        // Guard with nested braces inside
        let input = concat!(
            "\tif (SSF2API.isReady()) {\n",
            "\t\tif (x > 0) {\n",
            "\t\t\tself.y = 1;\n",
            "\t\t}\n",
            "\t}",
        );
        let output = translate_ssf2_to_fm(input);
        assert!(!output.contains("SSF2API.isReady()"), "outer guard not removed");
        // inner if should survive
        assert!(output.contains("if (x > 0)"), "inner if should survive");
        assert!(output.contains("self.y = 1;"), "body not inlined");
    }
}
