use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use crate::swf_parser::SwfFile;
use crate::abc_parser::{self, XframeMap};

// ─── Output types for haxe_gen ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterData {
    pub name: String,
    pub attacks: Vec<Attack>,
    pub stats: CharacterStats,
    pub animations: BTreeMap<String, AnimationInfo>,
    pub scripts: Vec<ScriptInfo>,
    /// Instance variables declared on the SSF2 XxxExt class (slot/const traits).
    /// Carried over into Script.hx as top-level `var name;` declarations.
    #[serde(default)]
    pub ext_vars: Vec<String>,
    /// Initial values pulled out of the Ext-class constructor (iinit) for the
    /// `ext_vars`. `(name, rhs)`. Emitted at the top of initialize() unless
    /// the same name is already assigned in the merged-in SSF2 initialize body.
    #[serde(default)]
    pub ext_var_inits: Vec<(String, String)>,
    /// The type each field is declared with, `*` when unknown or declared two ways.
    #[serde(default)]
    pub ext_var_types: Vec<(String, String)>,
    /// the id the fighter's stats state it by (`normalStats_id`), which its code compares against
    #[serde(default)]
    pub stats_name: Option<String>,
    /// The names the source declares as getters: functions here, read as calls, and what a
    /// projectile asking its summoner one of them goes through.
    #[serde(default)]
    pub getters: Vec<String>,
    /// Per-projectile physics + hitboxes pulled from the Ext class's
    /// `getProjectileStats()` method. Keys match the SSF2 projectile name
    /// used in the projectile SymbolClass (e.g. `dee_nspec`). Consumed
    /// by haxe_gen's projectile-file generators to fill real values into
    /// ProjectileStats.hx / ProjectileHitboxStats.hx; falls back to
    /// scaffolding placeholders when a projectile has no entry here.
    #[serde(default)]
    pub projectile_data: BTreeMap<String, abc_parser::ProjectileData>,
    /// SSF2 animation name → Fraymakers animation name
    /// Built from xframe_map + SSF2→Fraymakers name table
    pub ssf2_to_fm_anim: BTreeMap<String, String>,
    /// SSF2 `attackSound{N}_id` table (1-based) backing `playAttackSound(N)`.
    /// Emitted into Script.hx as the `_attackSounds` array + helper. See
    /// `abc_parser::extract_indexed_string_fields`.
    #[serde(default)]
    pub attack_sounds: Vec<String>,
    /// SSF2 `attackVoice{N}_id` table (1-based) backing `playVoiceSound(N)`.
    #[serde(default)]
    pub voice_sounds: Vec<String>,
    /// The clip this character's stats name as its `thumbnail`: the picture SSF2's damage
    /// counter shows. The portrait the menu entity is built from, stated by the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// The named children of its clips that its code addresses, each a part riding it (see
    /// `entity_gen::ProjectilePartData`), and where each animation's clip places them.
    #[serde(skip)]
    pub parts: Vec<crate::entity_gen::ProjectilePartData>,
    #[serde(skip)]
    pub part_placements: Vec<crate::entity_gen::PartPlacement>,
    /// The platforms the fighter gives itself (`createSelfPlatform(x, y, w, h)`), one per call
    /// site in source pixels: each a `platformSprite{i}` animation a structure content rides on.
    #[serde(skip)]
    pub platforms: Vec<(f64, f64, f64, f64)>,
    /// The id the character declares for itself (`linkage_id` in its stats), what its own code
    /// and the things it summons compare against when asking who an object is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linkage_id: Option<String>,
    /// Populated when this character is a transformation / alternate
    /// form (Giga Bowser, Wario Man). Forwarded from
    /// `ExtractedCharacter.derived_from`. See path 2 plan §1.6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<abc_parser::DerivedFrom>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attack {
    pub name: String,           // Fraymakers name (e.g. "aerial_neutral")
    pub ssf2_name: String,      // Original SSF2 name (e.g. "a_air")
    pub hitboxes: Vec<Hitbox>,
    /// the attack's own numbers besides its boxes, as the source declares them
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stats: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hitbox {
    pub damage:           f64,
    pub angle:            f64,
    pub base_knockback:   f64,
    pub knockback_growth: f64,
    pub hitstop:          i32,
    pub self_hitstop:     i32,
    pub hitstun:          i32,
    /// SSF2 `shieldStunMultiplier` — how much longer a shield holds the defender for this hit.
    /// Fraymakers states the same thing, spelling the middle of it lower-case. 0 = not stated.
    pub shield_stun_multiplier: f64,
    /// every number the box states, by the source's own key and in its units: what a script
    /// that reads a box stat back while the move runs is asking for
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub raw: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterStats {
    pub weight:           f64,
    pub gravity:          f64,
    pub fall_speed:       f64,
    pub fast_fall_speed:  f64,
    pub walk_speed:       f64,
    pub dash_speed:       f64,
    pub air_mobility:     f64,
    pub max_jumps:        i32,
    pub jump_height:      f64,
    pub double_jump_height: f64,
    pub air_friction:     f64,
    /// SSF2 `shortHopSpeed` — initial velocity of a short hop (px/frame@30).
    /// 0 when SSF2 lacks it (then FM falls back to a jump-derived estimate).
    pub short_hop_speed:  f64,
    /// SSF2 `decel_rate` — grounded friction (px/frame²@30, stored negative).
    pub ground_friction:  f64,
    /// SSF2 `accel_rate` — grounded walk/run acceleration (px/frame²@30).
    pub ground_accel:     f64,
    /// SSF2 `accel_start` — initial grounded walk speed on press (px/frame@30).
    pub walk_initial:     f64,
    /// SSF2 `accel_start_dash` — initial dash speed on press (px/frame@30).
    pub dash_initial:     f64,
    /// SSF2 `jumpStartup` stat: # of grounded frames before the jump launches
    /// (the jump-squat duration). Used to slice jump_squat off the jump animation.
    pub jump_startup:     f64,
    /// SSF2 `roll_speed` — how fast the character rolls (px/frame@30). One number covers every
    /// roll the source has; fraymakers names four.
    pub roll_speed:       f64,
    /// SSF2 `roll_decay` — what a roll's speed is multiplied by each frame. With the speed
    /// it states how FAR a roll goes, which is the part that has to survive the port.
    pub roll_decay:       f64,
    /// SSF2 `max_jumpSpeed` — despite the name, how fast the character may move
    /// HORIZONTALLY while airborne. Read straight off the engine frame by frame: mario
    /// states 7 and drifts at exactly 7.0, fox states 7.3 and drifts at 7.3, kirby states
    /// 6.2 and drifts at 6.2, and each one holds there for as long as the direction is
    /// held. It is this engine's aerial speed cap by another name.
    pub air_speed_cap:    f64,
    /// SSF2 `dodgeSpeed` — the speed a dodge roll (and an air dodge) starts at (px/frame@30).
    pub dodge_speed:      f64,
    /// SSF2 `dodgeDecel` — what a dodge roll loses each frame (stated negative). With the speed,
    /// how far the roll goes.
    pub dodge_decel:      f64,
    /// SSF2 `height` — how tall the character's COLLISION BODY is, in source pixels. Every
    /// character states its own (kirby 40, mario 45, bowser 71); 0 when the source omits it.
    pub body_height:      f64,
    /// SSF2 `width` — how wide the same body is, in source pixels.
    pub body_width:       f64,
    /// Median scaleX from the root character MovieClip's xframe PlaceObject entries
    pub base_scale_x:     f64,
    /// Median scaleY from the root character MovieClip's xframe PlaceObject entries
    pub base_scale_y:     f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationInfo {
    pub name:   String,
    pub frames: u16,
    pub speed:  f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptInfo {
    pub name: String,
    pub code: String,
    pub is_ext_method: bool,
}

impl Default for CharacterStats {
    fn default() -> Self {
        Self {
            weight: 100.0, gravity: 0.0, fall_speed: 0.0, fast_fall_speed: 0.0,
            walk_speed: 0.0, dash_speed: 0.0, air_mobility: 0.0,
            max_jumps: 2, jump_height: 0.0, double_jump_height: 0.0, air_friction: 0.0,
            short_hop_speed: 0.0, ground_friction: 0.0, ground_accel: 0.0,
            walk_initial: 0.0, dash_initial: 0.0,
            jump_startup: 0.0,
            roll_speed: 0.0, roll_decay: 0.0, air_speed_cap: 0.0, dodge_speed: 0.0, dodge_decel: 0.0,
            body_height: 0.0, body_width: 0.0,
            base_scale_x: 1.0, base_scale_y: 1.0,
        }
    }
}

// ─── Main extraction logic ───────────────────────────────────────────────────

pub fn extract(swf: &SwfFile, char_name: &str, class_anim: &BTreeMap<String, String>, foreign_clips: &std::collections::BTreeSet<String>, own_clips: &std::collections::BTreeSet<String>) -> Result<CharacterData> {
    log::info!("Extracting character data for '{}'", char_name);

    let mut all_attacks: BTreeMap<String, Vec<Hitbox>> = BTreeMap::new();
    let mut all_attack_stats: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    let mut char_stats = CharacterStats::default();
    let mut animations: BTreeMap<String, AnimationInfo> = BTreeMap::new();
    let mut scripts: Vec<ScriptInfo> = Vec::new();
    // names the source says are read by calling them, gathered across every block
    let mut getters: Vec<String> = Vec::new();
    let mut ext_vars: Vec<String> = Vec::new();
    let mut ext_var_inits: Vec<(String, String)> = Vec::new();
    let mut ext_var_types: Vec<(String, String)> = Vec::new();
    let mut stats_name: Option<String> = None;
    let mut projectile_data: BTreeMap<String, abc_parser::ProjectileData> = BTreeMap::new();
    let mut xframe_map: XframeMap = BTreeMap::new();
    let mut derived_from: Option<abc_parser::DerivedFrom> = None;
    let mut attack_sounds: Vec<String> = Vec::new();
    let mut voice_sounds: Vec<String> = Vec::new();
    let mut thumbnail: Option<String> = None;
    let mut linkage_id: Option<String> = None;
    // Parity harness source-of-truth: when DUMP_PARITY is set, collect the RAW
    // SSF2 hitbox values (SSF2 key names: damage/direction/power/kbConstant/…)
    // per FM move name, before the SSF2→FM field mapping. Written to
    // /tmp/parity_<char>_ssf2.json for tools/parity_check.py to diff against the
    // generated HitboxStats.hx. (tools/parity_check.py documents the mapping.)
    let dump_parity = std::env::var("DUMP_PARITY").is_ok();
    let mut raw_attacks_dump: BTreeMap<String, Vec<BTreeMap<String, f64>>> = BTreeMap::new();
    // Every block, kept: a class the character builds is not always in the same block as
    // the code that builds it, and one block cannot see another's classes.
    let mut parsed_blocks: Vec<abc_parser::AbcFile> = Vec::new();

    // Parse each ABC block (usually just one)
    for (block_idx, abc_data) in swf.abc_blocks.iter().enumerate() {
        log::info!("Parsing ABC block {} ({} bytes)", block_idx, abc_data.len());

        match abc_parser::parse(abc_data) {
            Ok(abc) => {
                parsed_blocks.push(abc.clone());
                let extracted = abc_parser::extract_character_with_timeline(&abc, char_name, class_anim, foreign_clips, own_clips)?;
                if derived_from.is_none() { derived_from = extracted.derived_from.clone(); }
                if attack_sounds.is_empty() { attack_sounds = extracted.attack_sounds.clone(); }
                if voice_sounds.is_empty()  { voice_sounds  = extracted.voice_sounds.clone(); }
                if thumbnail.is_none()      { thumbnail     = extracted.thumbnail.clone(); }
                if linkage_id.is_none()     { linkage_id    = extracted.linkage_id.clone(); }

                // Merge attacks
                for (name, attack_data) in &extracted.attacks {
                    let hitboxes = convert_hitboxes(&attack_data.hitboxes);
                    all_attacks.entry(name.clone()).or_default().extend(hitboxes);
                    all_attack_stats.entry(name.clone()).or_default().extend(attack_data.stats.clone());
                    if dump_parity {
                        raw_attacks_dump.entry(name.clone()).or_default()
                            .extend(attack_data.hitboxes.iter().cloned());
                    }
                }

                // Use stats if found
                if let Some(s) = &extracted.stats {
                    if std::env::var("PEPTIDE_DUMP_OWN_STATS").is_ok() {
                        for (k, v) in &s.values { eprintln!("[own-stat] {k} = {v}"); }
                    }
                    char_stats = convert_stats(&s.values);
                    if stats_name.is_none() {
                        stats_name = s.names.get("normalStats_id").or_else(|| s.names.get("statsName")).cloned();
                    }
                }

                // Decompiled Ext methods → Script.hx
                for (method_name, code) in &extracted.ext_methods {
                    scripts.push(ScriptInfo {
                        name: method_name.clone(),
                        code: code.clone(),
                        is_ext_method: true,
                    });
                }

                // Ext-class instance variable declarations → Script.hx top.
                for v in &extracted.ext_vars {
                    if !ext_vars.contains(v) { ext_vars.push(v.clone()); }
                }
                for (n, ty) in &extracted.ext_var_types {
                    abc_parser::note_declared_type(&mut ext_var_types, n, Some(ty));
                }
                for (n, rhs) in &extracted.ext_var_inits {
                    if !ext_var_inits.iter().any(|(en, _)| en == n) {
                        ext_var_inits.push((n.clone(), rhs.clone()));
                    }
                }

                // Frame scripts → will go to .entity file (not Script.hx)
                for (method_name, actions) in &extracted.frame_scripts {
                    let code = render_frame_script(method_name, actions);
                    scripts.push(ScriptInfo { name: method_name.clone(), code, is_ext_method: false });
                }

                for g in &extracted.getters {
                    if !getters.contains(g) { getters.push(g.clone()); }
                }

                // A NAME TWO CLASSES BOTH DECLARED. The second kept its own under `__own`,
                // and whoever hands that name to a timer or a listener means its own -- but the
                // package is split across blocks, so a registration can sit in one and the
                // method in another. Handing it over is the one place the meaning is certain,
                // so that is where it is rewired.
                for n in &extracted.renamed_own {
                    let own = format!("{n}__own");
                    for pat in [
                        format!(r"((?:create|add|destroy|remove)Timer\([^()]*,\s*(?:self\.)*){}\b", regex::escape(n)),
                        format!(r"((?:add|remove)EventListener\([^()]*,\s*(?:self\.)*){}\b", regex::escape(n)),
                        // cancelling names the callback and nothing else
                        format!(r"((?:destroy|remove)Timer\(\s*(?:self\.)*){}\b", regex::escape(n)),
                        // and a plain call of it: donkey kong's jab called `checkControls()` and
                        // got the charge shot's, which ended the jab on its sixth frame
                        format!(r"((?:self\.)+){}\(", regex::escape(n)),
                    ] {
                        let re = crate::dyn_re(&pat);
                        for sc in scripts.iter_mut() {
                            sc.code = re.replace_all(&sc.code, format!("${{1}}{own}")).into_owned();
                        }
                    }
                }

                // A METHOD NAMED LIKE ONE OF THIS ENGINE'S HOOKS that the source itself hands to
                // a timer or a listener, or calls by name, is the class's own function, not the
                // hook: megaman's neutral special keeps an `update` its clip runs on a timer
                // while the special plays, and merged into the fighter's per-frame update it
                // drove his walk with the special's own speed model (0.33 a frame). Kept under
                // `__own`, and every hand-over and call rewired to it.
                for hook in ["update", "initialize", "inputUpdateHook", "handleLinkFrames", "onTeardown"] {
                    let referenced = scripts.iter().any(|sc| {
                        let c = &sc.code;
                        let reg = crate::dyn_re(&format!(
                            r"(?:create|add|destroy|remove)(?:Timer|EventListener)\([^()]*(?:self\.)*{}\b", hook));
                        reg.is_match(c) || c.contains(&format!("self.{hook}()")) || c.contains(&format!("self.self.{hook}()"))
                    });
                    let declared = scripts.iter().any(|sc| sc.is_ext_method && sc.name == hook);
                    if !(referenced && declared) { continue; }
                    let own = format!("{hook}__own");
                    log::info!("'{}' is a function the source registers or calls itself, not this engine's hook; kept as '{}'", hook, own);
                    for sc in scripts.iter_mut() {
                        if sc.is_ext_method && sc.name == hook {
                            sc.name = own.clone();
                            sc.code = sc.code.replacen(&format!("function {hook}("), &format!("function {own}("), 1);
                        }
                    }
                    for pat in [
                        format!(r"((?:create|add|destroy|remove)Timer\([^()]*,\s*(?:self\.)*){}\b", hook),
                        format!(r"((?:add|remove)EventListener\([^()]*,\s*(?:self\.)*){}\b", hook),
                        format!(r"((?:destroy|remove)Timer\(\s*(?:self\.)*){}\b", hook),
                        format!(r"((?:self\.)+){}\(", hook),
                    ] {
                        let re = crate::dyn_re(&pat);
                        for sc in scripts.iter_mut() {
                            sc.code = re.replace_all(&sc.code, format!("${{1}}{own}")).into_owned();
                        }
                    }
                }

                // Note: we no longer seed animations from raw symbol names here.
                // The xframe_map seeding below produces FM-named entries which avoids
                // duplicates like "bair" + "aerial_back".

                // Merge projectile stats from getProjectileStats()
                for (name, data) in &extracted.projectiles {
                    projectile_data.entry(name.clone()).or_insert_with(|| data.clone());
                }

                // Merge xframe_map (frame method → SSF2 anim name)
                xframe_map.extend(extracted.xframe_map.clone());

                log::info!("ABC block {}: {} attacks, stats={}, {} frame scripts, {} xframe mappings, {} symbols→animations",
                    block_idx,
                    extracted.attacks.len(),
                    extracted.stats.is_some(),
                    extracted.frame_scripts.len(),
                    extracted.xframe_map.len(),
                    animations.len(),
                );
            }
            Err(e) => {
                log::warn!("Failed to parse ABC block {}: {}", block_idx, e);
            }
        }
    }

    if dump_parity {
        // Dev-only diagnostic (DUMP_PARITY); use the OS temp dir so it works on Windows too.
        let path = std::env::temp_dir().join(format!("parity_{}_ssf2.json", char_name));
        if let Ok(j) = serde_json::to_string_pretty(&raw_attacks_dump) {
            let _ = std::fs::write(&path, j);
            log::info!("DUMP_PARITY: wrote raw SSF2 attack stats -> {}", path.display());
        }
    }

    // Convert attacks map to sorted vec
    let attacks: Vec<Attack> = all_attacks.into_iter().map(|(name, hitboxes)| Attack {
        stats: all_attack_stats.remove(&name).unwrap_or_default(),
        ssf2_name: name.clone(),
        name: name.clone(),
        hitboxes,
    }).collect();

    // Build SSF2 anim name → Fraymakers anim name map
    let ssf2_to_fm_anim = build_ssf2_to_fm_anim(&xframe_map);

    // Also seed animations from xframe_map so every known SSF2 anim appears.
    // For animations that get split into sub-anims (jab→1/2/3, taunt→side/up/down),
    // expand them here so the final animation list is correct.
    for ssf2_name in xframe_map.values() {
        let fm_name = ssf2_to_fm_anim.get(ssf2_name).cloned().unwrap_or_else(|| ssf2_name.clone());
        // Expand split animations
        let sub_names = expand_split_anim(&fm_name);
        if sub_names.is_empty() {
            animations.entry(fm_name.clone()).or_insert(AnimationInfo {
                name: fm_name,
                frames: 0,
                speed: 1.0,
            });
        } else {
            for sub in sub_names {
                animations.entry(sub.clone()).or_insert(AnimationInfo {
                    name: sub,
                    frames: 0,
                    speed: 1.0,
                });
            }
        }
    }

    // Deduplicate: remove raw SSF2/symbol names that have a known FM equivalent.
    // e.g. if both "bair" and "aerial_back" exist, drop "bair".
    // Also drop internal/helper symbols that aren't real character animations.
    // Include sub-anim names (jab1/jab2/jab3 etc) produced by split expansion
    let mut fm_names: std::collections::BTreeSet<String> = ssf2_to_fm_anim.values().cloned().collect();
    for base_fm in ssf2_to_fm_anim.values() {
        for sub in expand_split_anim(base_fm) {
            fm_names.insert(sub);
        }
    }
    animations.retain(|key, _| {
        // Keep if it's a known FM name
        if fm_names.contains(key) { return true; }
        // Drop if it's a raw SSF2 name that maps to a different FM name
        if let Some(fm) = ssf2_to_fm_anim.get(key) {
            if fm != key { return false; }
        }
        // Keep anything else (unmapped names stay as-is)
        true
    });

    // Detect IMPLICIT instance vars: `self.X = ...` in the translated script bodies (Script.hx
    // functions + frame scripts) where X is neither a declared ext_var nor one of the char's
    // own functions. SSF2 base-class / dynamically-assigned vars (speed, bounceSpeed, charge,
    // …) aren't slot/const traits on the char class, so without this they'd be emitted as
    // undefined `self.X` (e.g. `self.speed`) and crash at runtime. Construct + wrap them as
    // persistent state like the declared vars. A clean literal RHS drives type inference.
    {
    // A CLASS BUILT FROM ONE BLOCK AND DECLARED IN ANOTHER. Each block only sees its own
    // classes, so a `new X()` whose X lives elsewhere came through as a name nothing
    // declares. With every block in hand the rest of them can answer for it.
    {
        let built: std::collections::BTreeSet<String> = {
            let re = crate::re!(r"\bnew\s+([A-Za-z_]\w*)\s*\(");
            // the constructor's own initialisers as well as the methods: a character
            // builds most of what it keeps before it does anything with it
            scripts.iter().map(|sc| sc.code.clone())
                .chain(ext_var_inits.iter().map(|(_, rhs)| rhs.clone()))
                .flat_map(|code| re.captures_iter(&code).map(|c| c[1].to_string())
                    .collect::<Vec<_>>())
                .collect()
        };
        for name in built {
            if scripts.iter().any(|sc| sc.name == format!("{name}__new")) { continue }
            let Some(factory) = parsed_blocks.iter()
                .find_map(|abc| abc_parser::reconstruct_class_as_factory(abc, &name))
            else { continue };
            log::info!("class '{name}' is declared in another block of the package, \
                ported as a factory");
            scripts.push(ScriptInfo {
                name: format!("{name}__new"),
                code: factory,
                is_ext_method: true,
            });
            for sc in &mut scripts {
                sc.code = sc.code.replace(&format!("new {name}("), &format!("{name}__new("));
            }
            for (_, rhs) in &mut ext_var_inits {
                *rhs = rhs.replace(&format!("new {name}("), &format!("{name}__new("));
            }
        }
    }

        let own_fns: std::collections::BTreeSet<&str> =
            scripts.iter().filter(|s| s.is_ext_method).map(|s| s.name.as_str()).collect();
        let re = crate::re!(r"self\.([A-Za-z_]\w*)\s*=\s*([^=;][^;]*)");
        let mut implicit: std::collections::BTreeMap<String, Option<String>> = Default::default();
        let mut widened: std::collections::BTreeSet<String> = Default::default();
        for s in &scripts {
            for cap in re.captures_iter(&s.code) {
                let name = cap[1].to_string();
                if ext_vars.contains(&name) || own_fns.contains(name.as_str()) { continue; }
                let rhs = cap[2].trim().to_string();
                let is_lit = matches!(rhs.as_str(), "true" | "false") || rhs.parse::<i64>().is_ok();
                // A field nobody declares starts out undefined in the source, however many
                // frames later write it: the first literal write in file order is just the
                // clip that happened to come first (fox's jab found `pressed2` already true
                // because the fire fox clip wrote it, and chained on one tap). Only the
                // constructor's write is an initial value, and that pass records it.
                implicit.entry(name.clone()).or_insert(None);
                // A field that holds a THING anywhere holds a thing, whatever it was set to
                // first. Typed off the first literal alone, chibirobo's telly vision slot is
                // a bool because the field starts out false, and the engine refuses the
                // projectile it is later given -- so the character goes on making another
                // one every frame, having never managed to record the one it has.
                if !is_lit { widened.insert(name); }
            }
        }
        // A field the class only ever READS is still one of its fields -- it is written
        // somewhere this pass cannot see. Undeclared it reads as `Unknown variable` and
        // takes the whole handler with it, so it is constructed like the assigned ones.
        // What it holds is whatever wrote it; the guards the source already puts around
        // it read it the same way either way.
        // What the display object itself carries is not one of the class's fields:
        // these are the clip's own properties, and each has its own translation
        // (or none). Constructing them as state would give the class a second,
        // empty copy of something the object already has.
        const CLIP_PROPERTIES: &[&str] = &[
            "x", "y", "alpha", "visible", "rotation", "scaleX", "scaleY",
            "width", "height", "parent", "root", "stage", "name",
            "currentLabel", "currentFrame", "totalFrames", "framesLoaded",
        ];
        let read_re = crate::re!(r"self\.([A-Za-z_]\w*)\s*(?:[^\w(]|$)");
        for s in &scripts {
            for line in s.code.lines() {
                let line = line.trim_start();
                if line.starts_with("//") || line.starts_with('*') { continue }
                let live = line.split("//").next().unwrap_or(line);
                for cap in read_re.captures_iter(live) {
                    let name = cap[1].to_string();
                    if ext_vars.contains(&name) || own_fns.contains(name.as_str()) { continue }
                    if CLIP_PROPERTIES.contains(&name.as_str()) { continue }
                    implicit.entry(name).or_insert(None);
                }
            }
        }
        // A field that is READ but never ASSIGNED anywhere in the class is one the
        // flash runtime bound: an instance sitting on the timeline, reached by the
        // name the timeline gives it. Here the timeline is baked into animations at
        // convert time and there is no such object to reach, so a CALL on one has
        // nothing to call. Struck, the handler around it lives; read, it answers
        // null, which is the same answer "that clip is not here" gives.
        let assigned: std::collections::BTreeSet<String> = {
            let a = crate::re!(r"self\.([A-Za-z_]\w*)\s*=[^=]");
            scripts.iter().flat_map(|s| a.captures_iter(&s.code).map(|c| c[1].to_string())).collect()
        };
        let timeline_bound: Vec<String> = implicit.keys()
            .filter(|n| !assigned.contains(*n))
            .cloned()
            .collect();
        if !timeline_bound.is_empty() {
            log::info!("{} field(s) the timeline binds, whose calls have no object here: {}",
                timeline_bound.len(),
                timeline_bound.iter().take(8).cloned().collect::<Vec<_>>().join(", "));
            let net = |l: &str| -> i32 {
                let live = l.split("//").next().unwrap_or(l);
                live.matches('{').count() as i32 - live.matches('}').count() as i32
            };
            for s in &mut scripts {
                if !timeline_bound.iter().any(|n| s.code.contains(n.as_str())) { continue }
                let lines: Vec<&str> = s.code.lines().collect();
                let mut mark: Vec<Option<&str>> = vec![None; lines.len()];
                let mut i = 0usize;
                while i < lines.len() {
                    let trimmed = lines[i].trim_start();
                    // reaching INTO one is the same as calling on one: there is nothing
                    // there to index either
                    let hit = (!trimmed.starts_with("//")).then(|| {
                        timeline_bound.iter().find(|n| {
                            lines[i].contains(&format!("self.{n}."))
                                || lines[i].contains(&format!("self.{n}["))
                        })
                    }).flatten();
                    let Some(n) = hit else { i += 1; continue };
                    mark[i] = Some(n);
                    // A statement that OPENS a block takes the block with it: struck
                    // alone, its body and its closing brace are left with nothing to
                    // belong to, and an `else` after them has no `if`.
                    let mut depth = net(lines[i]);
                    let mut j = i + 1;
                    while depth > 0 && j < lines.len() {
                        mark[j] = Some(n);
                        depth += net(lines[j]);
                        j += 1;
                    }
                    // and the `else` that hangs off it, with its block
                    while j < lines.len() && lines[j].trim_start().starts_with("} else") {
                        let mut d = net(lines[j]);
                        mark[j] = Some(n);
                        j += 1;
                        while d > 0 && j < lines.len() {
                            mark[j] = Some(n);
                            d += net(lines[j]);
                            j += 1;
                        }
                    }
                    i = j.max(i + 1);
                }
                let mut out = String::with_capacity(s.code.len());
                for (idx, line) in lines.iter().enumerate() {
                    let trimmed = line.trim_start();
                    match mark[idx] {
                        Some(n) if !trimmed.starts_with("//") => {
                            let indent = &line[..line.len() - trimmed.len()];
                            out.push_str(&format!("{indent}// [SSF2-only: {n}] {trimmed}\n"));
                        }
                        _ => { out.push_str(line); out.push('\n'); }
                    }
                }
                s.code = out;
            }
        }

        for (name, lit) in implicit {
            let lit = if widened.contains(&name) { None } else { lit };
            log::info!("implicit instance var '{}' (used but not declared) — constructing as persistent state", name);
            if let Some(rhs) = lit {
                if !ext_var_inits.iter().any(|(n, _)| n == &name) { ext_var_inits.push((name.clone(), rhs)); }
            }
            ext_vars.push(name);
        }
    }

    // READING A GETTER IS CALLING IT, which is the whole point of one. Left bare, the read hands
    // the function itself to whatever wanted the number: lucario scales every hitbox it has by
    // its aura and got a function. Done once all the blocks have been read, since the frames
    // that do the reading and the class that declares it need not be in the same one.
    for g in &getters {
        // however the source reaches it: plainly, or through the object it is standing on,
        // which is how the decompiler writes it before that receiver is taken off again.
        let plain = crate::dyn_re(&format!(
            r"(^|[^\w.])(?:self\.)*{}\b", regex::escape(g)));
        for sc in scripts.iter_mut() {
            if &sc.name == g { continue }
            let mut out = String::with_capacity(sc.code.len());
            let mut last = 0usize;
            for m in plain.find_iter(&sc.code) {
                let end = m.end();
                out.push_str(&sc.code[last..end]);
                if !sc.code[end..].trim_start().starts_with('(') { out.push_str("()"); }
                last = end;
            }
            out.push_str(&sc.code[last..]);
            sc.code = out;
        }
    }

    // A GETTER IS A FUNCTION, NOT A SLOT. A clip that keeps a field under the same name as the
    // class's getter (lucario's aura sphere copies `auraMultiplier` off its summoner) flattened
    // into a slot beside the function, and the slot won: what the character published under
    // the name was the slot, and a projectile asking the summoner's getter got a wrapper that
    // held nothing. The reads are calls now (above), so the slot has no reader left.
    {
        let before = ext_vars.len();
        ext_vars.retain(|v| !getters.contains(v));
        ext_var_inits.retain(|(n, _)| !getters.contains(n));
        if ext_vars.len() != before {
            log::info!("{} slot(s) under a getter's name dropped: the getter answers", before - ext_vars.len());
        }
    }

    // SSF2 `removeTimer(callback)` identifies a timer by its CALLBACK; Fraymakers
    // `addTimer(interval, repeats, func, options?)` returns a uid and `removeTimer(uid)`
    // takes that uid. Passing the function straight through is a runtime type error
    // ("Can't cast (void ()) to i32"), the engine traps it, and the timer is never
    // removed — so every SSF2 poll that's supposed to stop itself runs forever.
    //
    // Bridge the two by remembering the uid: for a callback that is BOTH registered by a
    // named `addTimer` and cancelled by name, synthesize a persistent `<name>_timer`
    // slot, capture the uid into it at registration, and cancel through it. Emitted in
    // `self.<var>` form so the persistent-state pass wraps it into `.set()`/`.get()` with
    // every other instance var. A `removeTimer` whose argument isn't a bare identifier
    // (`removeTimer(effects.get(i))`) can't be resolved statically and is left alone.
    //
    // The slot defaults to -1, so if a later pass drops the registration (the callback is
    // cancelled from a script that survives while the one that registered it doesn't) the
    // cancel degrades to `removeTimer(-1)`, which the engine answers `false`. Same
    // not-removed behaviour as before, minus the trapped type error.
    //
    // Runs on the decompiled source, so it matches the SSF2 spellings (`createTimer` /
    // `destroyTimer`) as well as the Fraymakers ones — the commands.jsonc rename to
    // add/removeTimer happens downstream of here.
    {
        // decompiler output at this stage carries a doubled receiver
        // (`self.self.createTimer(…)`) and prefixes the callback (`self.dashCheck`);
        // both are normalized downstream, so match them loosely and re-emit verbatim.
        // The receiver is OPTIONAL. A source that cancels its own timer says so plainly --
        // `removeTimer(lock)`, no receiver -- and requiring one meant those never paired with
        // the registration, so the uid was never captured and the cancel went out holding a
        // function where the engine wants a number. The timer then never stops, and neither
        // does whatever it was polling for.
        const RECV: &str = r"((?:self\.)*)";
        const ADD: &str = r"((?:create|add)Timer)";
        const RM:  &str = r"((?:destroy|remove)Timer)";
        let add_re = crate::dyn_re(&format!(r"{RECV}{ADD}\(\s*([^,()]+),\s*([^,()]+),\s*(?:self\.)?([A-Za-z_]\w*)\s*([,)])"));
        let rm_re = crate::dyn_re(&format!(r"{RECV}{RM}\(\s*(?:self\.)?([A-Za-z_]\w*)\s*\)"));
        let mut registered: std::collections::BTreeSet<String> = Default::default();
        let mut cancelled: std::collections::BTreeSet<String> = Default::default();
        // a callback the timeline kept as its own (`X__own`) and the one a class registers or
        // cancels as `X` are the same poll to a timer: one slot, under the base name, matched
        // in either spelling (ness registered `releaseProjectile__own` on its frame and cancelled
        // `releaseProjectile`, so the uid was never captured and the release ran forever)
        // a class's own copy is `X__<Class>` (the timeline's is `X__own`)
        // the FIRST double underscore: a class short can carry one of its own
        // (`testButton__NeutralSpecial_Ground__40`), and split at the last it nested one slot
        // inside another
        let base = |n: &str| match n.split_once("__") {
            Some((b, rest)) if !b.is_empty() && !rest.is_empty() => b.to_string(),
            _ => n.to_string(),
        };
        for s in &scripts {
            for c in add_re.captures_iter(&s.code) { registered.insert(base(&c[5])); }
            for c in rm_re.captures_iter(&s.code)  { cancelled.insert(base(&c[3])); }
            if std::env::var("PEPTIDE_TIMER_DEBUG").is_ok() {
                for l in s.code.lines().filter(|l| l.contains("Timer")) {
                    eprintln!("[timer-raw] {}", l.trim());
                }
            }
        }
        // every callback cancelled by name gets a slot, registered or not: a cancel of a timer
        // nothing ever made is a no-op in the source (jigglypuff's roll cancels an end timer it
        // never sets), and handed the function itself this engine throws (`Can't cast (void ())
        // to i32`), so the roll never ended. Through a slot that stays -1 the cancel is the
        // harmless miss it was.
        let slots: Vec<(String, String)> = cancelled.iter()
            .map(|n| (n.clone(), format!("{n}_timer")))
            .filter(|(_, slot)| !ext_vars.contains(slot))
            .collect();
        let _ = &registered;
        for (callback, slot) in &slots {
            log::info!("timer '{}' cancelled by callback -> capturing its uid in persistent '{}'",
                callback, slot);
            for s in scripts.iter_mut() {
                // capture at registration. only the exact (interval, repeats, <callback>)
                // shape is touched, so an inline `function () { … }` callback (which
                // nothing can cancel by name anyway) is left as-is.
                let cap = crate::dyn_re(&format!(
                    r"{RECV}{ADD}\(\s*([^,()]+),\s*([^,()]+),\s*((?:self\.)?{}(?:__\w+)?)\s*([,)])",
                    regex::escape(callback)
                ));
                // written with the receiver spelled out, whether or not the source had one
                s.code = cap.replace_all(&s.code, format!(
                    "self.{slot} = self.${{2}}(${{3}}, ${{4}}, ${{5}}${{6}}"
                )).into_owned();
                // and cancel through the slot
                let can = crate::dyn_re(&format!(
                    r"{RECV}{RM}\(\s*(?:self\.)?{}(?:__\w+)?\s*\)", regex::escape(callback)
                ));
                s.code = can.replace_all(&s.code, format!("self.${{2}}(self.{slot})")).into_owned();
            }
            if !ext_var_inits.iter().any(|(n, _)| n == slot) {
                ext_var_inits.push((slot.clone(), "-1".to_string()));
            }
            ext_vars.push(slot.clone());
        }
    }

    // SSF2 get/setGlobalVariable("name"[, v]) are string-keyed PERSISTENT per-character
    // variables. Collect each distinct key as a persistent-state ext var so a
    // `var name = self.makeInt(0)/makeBool(false)` declaration is emitted; the calls
    // themselves are rewritten to name.get()/name.set(v) by the commands.jsonc regex
    // rules. Init kind: Int when the var is used numerically (set to an int literal, or
    // its get() sits next to an arithmetic / numeric-comparison operator), else unset.
    // hscript is dynamically typed, so the wrapper kind
    // only sets the value seen BEFORE the first set — picking the right default matters,
    // the get/set themselves work regardless.
    {
        let get_re = crate::re!(r#"getGlobalVariable\(\s*"(\w+)"\s*\)"#);
        let set_re = crate::re!(r#"setGlobalVariable\(\s*"(\w+)"\s*,\s*(-?\d+\b)?"#);
        let num_ctx = crate::re!(r#"getGlobalVariable\(\s*"(\w+)"\s*\)\s*(?:[-+*/]|[<>]=?|[!=]=)\s*-?\d|-?\d\s*(?:[-+*/]|[<>]=?|[!=]=)\s*(?:self\.)?getGlobalVariable\(\s*"(\w+)"\s*\)|(?:self\.)?getGlobalVariable\(\s*"(\w+)"\s*\)\s*[-+*/]"#);
        let mut names: std::collections::BTreeSet<String> = Default::default();
        let mut numeric: std::collections::BTreeSet<String> = Default::default();
        for s in &scripts {
            for c in get_re.captures_iter(&s.code) { names.insert(c[1].to_string()); }
            for c in set_re.captures_iter(&s.code) {
                names.insert(c[1].to_string());
                if c.get(2).is_some() { numeric.insert(c[1].to_string()); }
            }
            for c in num_ctx.captures_iter(&s.code) {
                for i in 1..=3 { if let Some(m) = c.get(i) { numeric.insert(m.as_str().to_string()); } }
            }
        }
        for name in names {
            if ext_vars.contains(&name) { continue; }
            // a flag nothing has set reads as unset in the source, which equals neither true nor
            // false (waluigi turns his down special off while `plantspawn == false`, and a match
            // that never set it keeps the move); a condition on it is false either way
            let init = if numeric.contains(&name) { "0" } else { "null" };
            log::info!("global var '{}' (SSF2 get/setGlobalVariable) -> persistent {} wrapper",
                name, if init == "0" { "Int" } else { "unset" });
            if !ext_var_inits.iter().any(|(n, _)| n == &name) {
                ext_var_inits.push((name.clone(), init.to_string()));
            }
            ext_vars.push(name);
        }
    }

    // Resolve var/function NAME COLLISIONS: a name that is BOTH a persistent instance var
    // and one of the char's own functions cannot coexist at Script.hx top level (and makes
    // `self.<name>` ambiguous). Rename the FUNCTION (its definition + calls — `<name>(`) to
    // `<name>_2`, leaving the VAR with the bare name (wrapped to .get()/.set()). Function
    // CALLS are `<name>(`; var accesses are `<name>` / `<name>.get()`, so the `(` cleanly
    // distinguishes them.
    {
        let var_set: std::collections::BTreeSet<String> = ext_vars.iter().cloned().collect();
        let collisions: std::collections::BTreeSet<String> = scripts.iter()
            .filter(|s| s.is_ext_method && var_set.contains(&s.name))
            .map(|s| s.name.clone())
            .collect();
        for name in &collisions {
            let new_name = format!("{}_2", name);
            log::info!("name collision '{}' (instance var + function) — renaming the function to '{}'", name, new_name);
            let re = crate::dyn_re(&format!(r"\b{}\(", regex::escape(name)));
            // a name HANDED to a timer or an event is the function too: the source passes the
            // method for the engine to call later (kirby's copied `frameCount` counts down on a
            // timer). left with the var's name it hands over a number, and the timer fires nothing
            let handed = crate::dyn_re(&format!(
                r"((?:create|add|destroy|remove)(?:Timer|EventListener)\([^()]*,\s*(?:self\.)*){}\b", regex::escape(name)));
            for s in &mut scripts {
                s.code = re.replace_all(&s.code, format!("{}(", new_name)).into_owned();
                s.code = handed.replace_all(&s.code, format!("${{1}}{}", new_name)).into_owned();
                if s.is_ext_method && &s.name == name { s.name = new_name.clone(); }
            }
        }
    }

    // Drop any character-defined `flipX`. Fraymakers provides Entity.flipX(v:Float):Float as a
    // built-in (negates v when facing left); SSF2 characters that define their own `flipX` are
    // just reimplementing it. Removing the definition takes `flipX` out of the char's own-method
    // set, so rewrite_own_method_refs keeps every reference as `self.flipX(...)` — binding to the
    // engine implementation instead of a local copy. Any bare `flipX(` left over (e.g. a lex-call
    // the decompiler didn't qualify) is requalified to `self.flipX(` so nothing dangles.
    {
        let had_flipx = scripts.iter().any(|s| s.is_ext_method && s.name == "flipX");
        if had_flipx {
            scripts.retain(|s| !(s.is_ext_method && s.name == "flipX"));
            // Requalify any bare flipX( call to self.flipX( (skip ones already qualified with `.`).
            let re = crate::re!(r"(^|[^.\w])flipX\(");
            for s in &mut scripts {
                s.code = re.replace_all(&s.code, "${1}self.flipX(").into_owned();
            }
            log::info!("removed character-defined flipX — all references now use the engine's self.flipX");
        }
    }

    // Synthesize SSF2 state-transition shortcut handlers (toLand / toCrashLand / toHelpless).
    // Fraymakers has no such methods — each is just `self.toState(CState.X)`. They are used as
    // add/removeEventListener callbacks, so they MUST be stable NAMED functions (an inline
    // closure would make a later removeEventListener fail to match and leave a dangling
    // listener). We emit a named function per referenced shortcut; rewrite_own_method_refs then
    // strips `self.` from each call site (`self.toLand` -> bare `toLand`) so the listener and its
    // remover both reference this one definition. The body's `self.toState(...)` is an FM
    // built-in and is left untouched.
    {
        let shortcuts: [(&str, &str); 3] = [
            ("toLand",      "CState.LAND"),
            ("toCrashLand", "CState.CRASH_BOUNCE"),
            ("toHelpless",  "CState.FALL_SPECIAL"),
        ];
        for (name, state) in shortcuts {
            let re = crate::dyn_re(&format!(r"\b{}\b", name));
            let referenced = scripts.iter().any(|s| re.is_match(&s.code));
            let already_defined = scripts.iter().any(|s| s.is_ext_method && s.name == name);
            if referenced && !already_defined {
                scripts.push(ScriptInfo {
                    name: name.to_string(),
                    code: format!("function {}(arg0) {{\n\tself.toState({});\n}}", name, state),
                    is_ext_method: true,
                });
                log::info!("synthesized state-shortcut handler {}() -> self.toState({})", name, state);
            }
        }
    }

    log::info!("Total: {} attacks, {} animations, {} ssf2→fm mappings extracted",
        attacks.len(), animations.len(), ssf2_to_fm_anim.len());

    // a field declared `int` keeps only the whole part of what is written to it
    for s in &mut scripts {
        s.code = crate::api_mappings::coerce_int_field_writes(&s.code, &ext_var_types);
    }

    Ok(CharacterData {
        ext_vars,
        ext_var_inits,
        ext_var_types,
        stats_name,
        getters,
        projectile_data,
        name: char_name.to_string(),
        attacks,
        stats: char_stats,
        animations,
        scripts,
        ssf2_to_fm_anim,
        attack_sounds,
        voice_sounds,
        thumbnail,
        linkage_id,
        parts: vec![],
        part_placements: vec![],
        platforms: vec![],
        derived_from,
    })
}

// ─── SSF2 → Fraymakers animation name table ───────────────────────────────────

/// The FM parent anims that split into sub-anims at the sprite level, paired with every
/// sub-anim name they can produce. One shared table drives both `expand_split_anim` (seeding
/// candidate names) and `is_split_sub_anim` (recognizing a produced sub-anim). The frame-range
/// splitting itself lives in `sprite_parser::sub_anim_splits`, which dispatches on this same
/// parent set. We can't know per-character how many sub-anims a sprite has, so we list enough
/// to cover the max we'll encounter (jab4).
pub const SPLIT_ANIMS: &[(&str, &[&str])] = &[
    // Jab: up to 4 hits (captainfalcon, pit, ryu)
    ("jab",   &["jab1", "jab2", "jab3", "jab4"]),
    // Taunt: always exactly 3 slots (the parent `taunt` keeps its name)
    ("taunt", &["taunt", "taunt_up", "taunt_down"]),
];

/// For an SSF2 animation that maps to a single FM name but splits into sub-animations at the
/// sprite level (e.g. jab → jab1/jab2/jab3), return ALL POSSIBLE sub-anim names. These seed the
/// animation list so even unseen sub-anims get placeholder entries; sprite_parser trims to only
/// those with actual frames.
pub fn expand_split_anim(fm_name: &str) -> Vec<String> {
    SPLIT_ANIMS.iter()
        .find(|(parent, _)| *parent == fm_name)
        .map(|(_, subs)| subs.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

/// The FM animation a split piece came FROM (`jab1` -> `jab`), or `None` if this name is not a
/// piece. The mapping files speak in parent names -- `a` maps to `jab`, never to `jab1` -- so
/// anything looking an SSF2 animation up from a piece has to ask for the parent.
pub fn split_parent(fm_name: &str) -> Option<&'static str> {
    SPLIT_ANIMS.iter()
        .find(|(parent, subs)| *parent != fm_name && subs.contains(&fm_name))
        .map(|(parent, _)| *parent)
}

fn build_ssf2_to_fm_anim(xframe_map: &XframeMap) -> BTreeMap<String, String> {
    // SSF2 animation name → Fraymakers animation name table is loaded from
    // mappings/character/animations.json (see crate::mappings).
    let lookup = &crate::mappings::character_animations().ssf2_to_fm;
    let mut result = BTreeMap::new();

    // Map every unique SSF2 anim name from xframe_map through the table
    for ssf2_name in xframe_map.values() {
        let fm_name = lookup.get(ssf2_name)
            .cloned()
            .unwrap_or_else(|| ssf2_name.clone()); // fallback: keep as-is
        result.insert(ssf2_name.clone(), fm_name);
    }

    result
}

// ─── Conversion helpers ─────────────────────────────────────────────────────────

fn convert_hitboxes(raw: &[BTreeMap<String, f64>]) -> Vec<Hitbox> {
    // SSF2 key → Fraymakers hitbox field mapping is loaded from
    // mappings/character/hitbox_stats.json (see crate::mappings).
    let cfg = crate::mappings::character_hitbox_stats();
    raw.iter().map(|obj| {
        // Value of a Fraymakers field = max over its SSF2 source keys that are
        // PRESENT. Only-present is load-bearing: SSF2 uses NEGATIVE values as
        // special-angle sentinels (e.g. direction=-2), and if an absent sibling
        // key (e.g. "angle") defaulted to 0.0 it would win the max() and clobber
        // the real negative value to 0 (wrong launch direction). Absent keys must
        // not participate. If no source key is present, the field is 0.0.
        let v = |fm: &str| cfg.keys_for(fm).iter()
            .filter_map(|k| obj.get(k).copied())
            .reduce(f64::max)
            .unwrap_or(0.0);
        // FIXED knockback carries no growth; the rule is shared so every path applies it.
        let fixed_knockback = crate::mappings::HitboxStatsMapping::is_fixed_knockback(obj);
        // a fixed hit's launch is what the source works out from the field, not the field itself
        let base = cfg.fixed_knockback(obj).unwrap_or_else(|| v("baseKnockback"));
        Hitbox {
            damage:           v("damage"),
            angle:            v("angle"),
            base_knockback:   base,
            knockback_growth: if fixed_knockback { 0.0 } else { v("knockbackGrowth") },
            hitstop:          v("hitstop") as i32,
            self_hitstop:     v("selfHitstop") as i32,
            hitstun:          v("hitstun") as i32,
            shield_stun_multiplier: v("shieldstunMultiplier"),
            raw: obj.clone(),
        }
    }).collect()
}

fn convert_stats(vals: &BTreeMap<String, f64>) -> CharacterStats {
    // SSF2 key name → Fraymakers field mapping is loaded from
    // mappings/character/stats.json (see crate::mappings).
    let stat_map = crate::mappings::character_stats();
    let get = |field: &str| {
        stat_map.keys_for(field).iter()
            .find_map(|k| vals.get(k)).copied().unwrap_or(0.0)
    };
    CharacterStats {
        weight:             get("weight"),
        gravity:            get("gravity"),
        fall_speed:         get("fall_speed"),
        fast_fall_speed:    get("fast_fall_speed"),
        walk_speed:         get("walk_speed"),
        dash_speed:         get("dash_speed"),
        air_mobility:       get("air_mobility"),
        // SSF2 max_jump counts midair jumps; Fraymakers counts total jumps.
        // The +1 offset is loaded from mappings/character/stats.json.
        max_jumps:          get("max_jumps") as i32 + stat_map.offset("max_jumps") as i32,
        jump_height:        get("jump_height"),
        double_jump_height: get("double_jump_height"),
        air_friction:       get("air_friction"),
        // Real SSF2 movement constants that were previously discarded in favour
        // of template constants — wired through so walk/run/jump feel matches
        // the source. Raw keys (px/frame or px/frame²@30); 0 ⇒ absent in SSF2.
        short_hop_speed:    vals.get("shortHopSpeed").copied().unwrap_or(0.0),
        ground_friction:    vals.get("decel_rate").copied().unwrap_or(0.0),
        ground_accel:       vals.get("accel_rate").copied().unwrap_or(0.0),
        walk_initial:       vals.get("accel_start").copied().unwrap_or(0.0),
        dash_initial:       vals.get("accel_start_dash").copied().unwrap_or(0.0),
        // Internal-only (NOT a Fraymakers stat): raw SSF2 jumpStartup frame count,
        // used solely to slice jump_squat off the front of the jump animation.
        jump_startup:       vals.get("jumpStartup").copied().unwrap_or(0.0),
        roll_speed:         get("roll_speed"),
        roll_decay:         get("roll_decay"),
        air_speed_cap:      get("air_speed_cap"),
        dodge_speed:        get("dodge_speed"),
        dodge_decel:        get("dodge_decel"),
        // The body the character states for itself, which is what its collision box should be.
        body_height:        get("body_height"),
        body_width:         get("body_width"),
        // base_scale_x/y are set later from sprite_parser::extract_xframe_scale
        base_scale_x:       1.0,
        base_scale_y:       1.0,
    }
}

fn render_frame_script(_method_name: &str, actions: &[crate::abc_parser::FrameAction]) -> String {
    // Actions now contain decompiled Haxe code in action.action (frame=0 sentinel)
    // Just concatenate the decompiled bodies
    actions.iter().map(|a| a.action.as_str()).collect::<Vec<_>>().join("\n")
}

