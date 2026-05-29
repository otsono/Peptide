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
            base_scale_x: 1.0, base_scale_y: 1.0,
        }
    }
}

// ─── Main extraction logic ───────────────────────────────────────────────────

pub fn extract(swf: &SwfFile, char_name: &str) -> Result<CharacterData> {
    log::info!("Extracting character data for '{}'", char_name);

    let mut all_attacks: BTreeMap<String, Vec<Hitbox>> = BTreeMap::new();
    let mut char_stats = CharacterStats::default();
    let mut animations: BTreeMap<String, AnimationInfo> = BTreeMap::new();
    let mut scripts: Vec<ScriptInfo> = Vec::new();
    let mut ext_vars: Vec<String> = Vec::new();
    let mut ext_var_inits: Vec<(String, String)> = Vec::new();
    let mut projectile_data: BTreeMap<String, abc_parser::ProjectileData> = BTreeMap::new();
    let mut xframe_map: XframeMap = BTreeMap::new();
    let mut derived_from: Option<abc_parser::DerivedFrom> = None;
    let mut attack_sounds: Vec<String> = Vec::new();
    let mut voice_sounds: Vec<String> = Vec::new();

    // Parse each ABC block (usually just one)
    for (block_idx, abc_data) in swf.abc_blocks.iter().enumerate() {
        log::info!("Parsing ABC block {} ({} bytes)", block_idx, abc_data.len());

        match abc_parser::parse(abc_data) {
            Ok(abc) => {
                let extracted = abc_parser::extract_character(&abc, char_name)?;
                if derived_from.is_none() { derived_from = extracted.derived_from.clone(); }
                if attack_sounds.is_empty() { attack_sounds = extracted.attack_sounds.clone(); }
                if voice_sounds.is_empty()  { voice_sounds  = extracted.voice_sounds.clone(); }

                // Merge attacks
                for (name, attack_data) in &extracted.attacks {
                    let hitboxes = convert_hitboxes(&attack_data.hitboxes);
                    all_attacks.entry(name.clone()).or_default().extend(hitboxes);
                }

                // Use stats if found
                if let Some(s) = &extracted.stats {
                    char_stats = convert_stats(&s.values);
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

    // Convert attacks map to sorted vec
    let attacks: Vec<Attack> = all_attacks.into_iter().map(|(name, hitboxes)| Attack {
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
    let internal_prefixes = ["groundref", "itemplaceholder", "collisonbox", "chargespark", "fireball", "mariocut", "finalsmash"];
    animations.retain(|key, _| {
        // Keep if it's a known FM name
        if fm_names.contains(key) { return true; }
        // Drop if it's a raw SSF2 name that maps to a different FM name
        if let Some(fm) = ssf2_to_fm_anim.get(key) {
            if fm != key { return false; }
        }
        // Drop internal/helper symbols
        let lower = key.to_lowercase();
        if internal_prefixes.iter().any(|p| lower.starts_with(p)) { return false; }
        // Keep anything else (unmapped names stay as-is)
        true
    });

    log::info!("Total: {} attacks, {} animations, {} ssf2→fm mappings extracted",
        attacks.len(), animations.len(), ssf2_to_fm_anim.len());

    Ok(CharacterData {
        ext_vars,
        ext_var_inits,
        projectile_data,
        name: char_name.to_string(),
        attacks,
        stats: char_stats,
        animations,
        scripts,
        ssf2_to_fm_anim,
        attack_sounds,
        voice_sounds,
        derived_from,
    })
}

// ─── SSF2 → Fraymakers animation name table ───────────────────────────────────

/// For SSF2 animations that map to a single FM name but get split into sub-animations
/// at the sprite level (e.g. jab → jab1/jab2/jab3), return ALL POSSIBLE sub-anim names
/// that could be produced. These seed the animation list so even unseen sub-anims get
/// placeholder entries; sprite_parser trims to only those with actual frames.
///
/// We don't know at this stage how many sub-anims the sprite will have (it varies per
/// character), so we emit enough names to cover the maximum we'll encounter (jab4).
pub fn expand_split_anim(fm_name: &str) -> Vec<String> {
    match fm_name {
        // Jab: up to 4 hits (captainfalcon, pit, ryu)
        "jab"   => vec!["jab1".into(), "jab2".into(), "jab3".into(), "jab4".into()],
        // Taunt: always exactly 3 slots
        "taunt" => vec!["taunt".into(), "taunt_up".into(), "taunt_down".into()],
        _ => vec![],
    }
}

/// Returns true if this FM animation name is a sub-animation produced by splitting.
pub fn is_split_sub_anim(fm_name: &str) -> bool {
    matches!(fm_name, "jab1" | "jab2" | "jab3" | "jab4" | "taunt_up" | "taunt_down")
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
        // Value of a Fraymakers field = max over its SSF2 source keys
        // (an absent key counts as 0.0).
        let v = |fm: &str| cfg.keys_for(fm).iter()
            .map(|k| obj.get(k).copied().unwrap_or(0.0))
            .reduce(f64::max)
            .unwrap_or(0.0);
        Hitbox {
            damage:           v("damage"),
            angle:            v("angle"),
            base_knockback:   v("baseKnockback"),
            knockback_growth: v("knockbackGrowth"),
            hitstop:          v("hitstop") as i32,
            self_hitstop:     v("selfHitstop") as i32,
            hitstun:          v("hitstun") as i32,
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

