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

// ─── Method Mappings ──────────────────────────────────────────────────────────
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
pub fn translate_ssf2_to_fm(code: &str) -> String {
    // First: strip SSF2API.isReady() guard blocks entirely (they're always-true in FM)
    let mut result = remove_readiness_guards(code);

    // ── self.self → self ──
    // SSF2 sub-MC closures capture the character as "self.self"
    // The assignment `self.self = /* ? */` is the sub-MC saving a ref to parent character.
    // In FM, Script.hx `self` already IS the character, so this is a no-op we can elide.
    result = result.replace("self.self = /* ? */;", "// self reference (implicit in FM)");
    result = result.replace("self.self = /* ? */", "self /* parent character */");
    result = result.replace("self.self.", "self.");
    // Also handle: if (SSF2API.isReady() && self.self) → if (true)
    // "SSF2API.isReady() && self.self" pattern → just drop the self.self check
    result = result.replace("&& self.self)", ")");
    result = result.replace("if (true && self.self)", "if (true /* FM: self always valid */)");
    // Other self.self null-guard patterns
    result = result.replace("|| self.self)", ") /* self always valid */");
    result = result.replace("if ((self.self && true)", "if ((true /* self always valid */)");
    // Also handle (self.self && true) without if prefix
    result = result.replace("(self.self && true)", "(true /* self always valid */)");
    // Final catch-all: any remaining self.self that wasn't a method call (boolean checks etc)
    result = result.replace("self.self", "self /* was self.self */");
    result = result.replace("if (self.self && ", "if (/* self always valid */ true && ");
    result = result.replace("|| self.self", " /* || self always valid */");

    // ── SSF2API static calls ──
    result = result.replace("SSF2API.print(", "Engine.log(");
    result = result.replace("SSF2API.isReady()", "true");
    result = result.replace("SSF2API.random()", "Random.getFloat(0, 1)");
    result = result.replace("SSF2API.randomInteger(", "Random.getInt(");
    result = result.replace("SSF2API.getElapsedFrames()", "Engine.getElapsedFrames()");
    result = result.replace("SSF2API.getCharacters()", "Match.getCharacters()");

    // ── Method renames ──
    result = result.replace(".endAttack()", ".endAnimation()");
    result = result.replace(".refreshAttackID()", ".reactivateHitboxes()");
    result = result.replace(".updateAttackBoxStats(", ".updateHitboxStats(");
    result = result.replace(".getControls()", ".getHeldControls()");
    result = result.replace(".setXSpeed(", ".setXVelocity(");
    result = result.replace(".setYSpeed(", ".setYVelocity(");
    result = result.replace(".getXSpeed()", ".getXVelocity()");
    result = result.replace(".getYSpeed()", ".getYVelocity()");

    // ── State transitions ──
    // SSF2 had dedicated helpers; FM uses toState(CState.X)
    result = result.replace(".toLand()", ".toState(CState.LAND)");
    result = result.replace(".toHelpless()", ".toState(CState.FALL_SPECIAL)");
    // toHeavyLand → LAND for now; annotated TODO for land_heavy animation override
    result = result.replace(
        ".toHeavyLand()",
        ".toState(CState.LAND) //TODO: create land_heavy anim & change to toState(CState.LAND, \"land_heavy\")",
    );

    // ── Jump / landing reset ──
    // SSF2 resetJumps() → FM preLand() (resets jumps, airdash, fastfall)
    result = result.replace(".resetJumps()", ".preLand()");

    // ── Velocity reset ──
    result = result.replace(".resetMovement()", ".setXVelocity(0); self.setYVelocity(0)");
    result = result.replace(".safeMove(", ".move(");

    // ── Ground detection ──
    result = result.replace(".isOnGround()", ".isOnFloor()");
    result = result.replace(".checkGround()", ".attachToFloor()");

    // ── Landing lag / autocancel ──
    result = result.replace(".setLandingLag(true)", ".updateAnimationStats({ autoCancel: true })");
    result = result.replace(".setLandingLag(false)", ".updateAnimationStats({ autoCancel: false })");

    // ── SSF2 global variable pattern ──
    // self.setGlobalVariable("key", val) → self.setCustomVar("key", val) or metadata
    result = result.replace(".setGlobalVariable(", ".updateAnimationStatsMetadata(/* TODO: setGlobalVariable */ ");
    result = result.replace(".getGlobalVariable(", ".getAnimationStatsMetadata(/* TODO: getGlobalVariable */ ");

    // ── Sound calls ──
    result = result.replace(".playVoiceSound(", ".playAttackVoice(/* voice index: */");
    result = result.replace(".playSoundFX(", "/* TODO: playSoundFX */Engine.playAudio(");

    // ── Event types ──
    result = result.replace("SSF2Event.STATE_CHANGE", "GameObjectEvent.LINK_FRAMES");
    result = result.replace("SSF2Event.HIT", "GameObjectEvent.HIT_DEALT");
    result = result.replace("SSF2Event.LAND", "GameObjectEvent.LAND");

    // ── Timer/effect patterns ──
    result = result.replace(".createTimer(", ".addTimer(");
    result = result.replace(".destroyTimer(", ".removeTimer(");
    result = result.replace(".removeAllEffects()", "/* TODO: removeAllEffects */");
    result = result.replace(".addEffectToList(", "/* TODO: addEffectToList */ ");

    // ── Hitbox property renames in object literals ──
    // SSF2 name          → FM HitboxStats field
    result = result.replace("direction:",    "angle:");
    result = result.replace("power:",         "baseKnockback:");
    result = result.replace("kbGrowth:",      "knockbackGrowth:");
    result = result.replace("kbConstant:",    "knockbackGrowth:");  // SSF2 kbConstant = flat KB growth
    result = result.replace("hitStun:",       "hitstop:");          // SSF2 hitStun = frames of hitStop on attacker
    result = result.replace("selfHitStun:",   "selfHitstop:");
    result = result.replace("hitLag:",        "hitstun:");          // SSF2 hitLag = frames target is stunned
    result = result.replace("selfHitLag:",    "selfHitstop:");

    // ── endAnimation on last frame: strip it ──
    // FM animations naturally end when the last frame plays. endAnimation() on the final
    // frame of a sub-MC is redundant and causes a double-end. Strip it.
    result = strip_last_frame_end_animation(&result);

    // ── Comment out SSF2 calls with no FM equivalent ──
    // These would cause compile errors in Fraymakers. They're left as commented stubs
    // so modders know what logic existed and can implement alternatives.
    // NOTE: setIntangibility is NOT in this list — it's handled by the full-script
    // fix_intangibility_pairs() pass in haxe_gen after all scripts are assembled.
    result = comment_out_unknown_calls(&result);

    result
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
/// no Fraymakers equivalent. Leaves a note so modders know what was there.
///
/// Only whole statements (lines ending with `;`) are commented. Lines that are
/// part of conditionals or assignments that contain an unknown call are prefixed
/// with `// [SSF2-only] ` so they don't compile but remain readable.
pub fn comment_out_unknown_calls(code: &str) -> String {
    // Methods that exist in SSF2 but not in the Fraymakers API.
    // Calls to these must be commented out to avoid compile errors.
    const COMMENT_METHODS: &[&str] = &[
        // Effects / visuals
        ".attachEffect(",
        ".clearEffectsOnStateChange(",
        ".addEffectToList(",   // already partially handled above, catch remainder
        // Projectiles (need CustomGameObject — manual port)
        ".fireProjectile(",
        ".getCurrentProjectile(",
        // Grab helpers (different grab model in FM)
        ".getGrabbedOpponent(",
        ".releaseOpponent(",
        ".swapDepthsWithGrabbedOpponent(",
        // State transitions (not yet translated — passed as event listeners)
        ".gotoAndStop(",        // timeline nav; no FM equivalent
        ".jumpToContinue(",     // SSF2 jump-to-continue loop; no FM equivalent
        // NOTE: .toLand( / .toHeavyLand( / .toHelpless( are translated above
        // to self.toState(CState.X). Only comment if they survived untranslated
        // (e.g. when used as event-listener references like addEventListener(..., self.toLand))
        // Ground/platform
        ".unnattachFromGround(",  // FM uses unattachFromFloor() — close but not identical
        // Attack helpers
        ".killAttackboxes(",    // no FM equivalent
        ".checkAtkilled(",      // no FM equivalent
        ".updateAttackStats(",  // use updateAnimationStats/updateHitboxStats instead
        // NOTE: .setLandingLag( → updateAnimationStats({autoCancel}), translated above
        // NOTE: .setIntangibility( → applyGlobalBodyStatus, handled in fix_intangibility_pairs
        // NOTE: .resetJumps( → .preLand(), translated above
        // NOTE: .resetMovement( → setXVelocity(0)/setYVelocity(0), translated above
        // NOTE: .safeMove( → .move(, translated above
        // NOTE: .isOnGround( → .isOnFloor(), translated above
        // NOTE: .checkGround( → .attachToFloor(), translated above
        // Item / misc
        ".pickupItem(",
        ".tossItem(",
        ".getItem(",
        ".getMC(",
        ".getExecTime(",
        ".getNearestLedge(",
        ".getCPULevel(",
        ".isCPU(",
        ".getMetalStatus(",
        ".isForcedCrash(",
        ".inUpperLeftWarningBounds(",
        ".stancePlayFrame(",
        // Sound (FM uses content IDs not SWF asset IDs — manual port)
        ".playAttackSound(",
        ".playSound(",
        // Timeline navigation (no timeline in FM)
        ".stop(",
    ];

    let lines: Vec<&str> = code.lines().collect();
    let mut out = Vec::with_capacity(lines.len());

    for line in &lines {
        let trimmed = line.trim();
        // Skip lines already commented out
        if trimmed.starts_with("//") {
            out.push(line.to_string());
            continue;
        }
        // Check if this line contains an unknown call
        let has_unknown = COMMENT_METHODS.iter().any(|m| line.contains(m));
        if has_unknown {
            // Determine a useful tag based on which method it is
            let tag = COMMENT_METHODS.iter()
                .find(|m| line.contains(*m))
                .map(|m| m.trim_matches('.').trim_matches('('))
                .unwrap_or("SSF2-only");
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push(format!("{}// [SSF2-only: {}] {}", indent, tag, trimmed));
        } else {
            out.push(line.to_string());
        }
    }

    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

/// Load SSF2→FM method mappings from the JSON file at `mappings/api_methods.json`
/// relative to the project root. Falls back to empty map if file not found.
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

/// Double the hit-freeze / hitstun durations in inline hitbox objects of
/// decompiled frame-script code (e.g. `updateAttackBoxStats(1, { hitStun: 2,
/// selfHitStun: 1 })`). SSF2 `hitStun`/`selfHitStun`/`hitLag` are frame counts;
/// the 255 "no override" sentinel and negatives are left unchanged.
pub fn double_frame_script_hit_durations(code: &str) -> String {
    let mut out = code.to_string();
    for marker in ["hitStun:", "selfHitStun:", "hitLag:"] {
        out = double_int_after_marker(&out, marker, 255);
    }
    out
}

/// Double the frame-delay argument of `createTimer(delay, repeatCount, cb)`
/// calls in decompiled frame-script code. Only the first argument — the delay,
/// in frames — is scaled; the repeat count and callback are left untouched
/// (doubling the delay alone already stretches the timer's total real time).
pub fn double_frame_script_timers(code: &str) -> String {
    double_int_after_marker(code, "createTimer(", i64::MAX)
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
