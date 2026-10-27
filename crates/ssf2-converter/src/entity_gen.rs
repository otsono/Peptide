/// Fraymakers .entity file generator
///
/// Generates the JSON entity file compatible with FrayTools.
/// Format based on the official Fraymakers character template:
/// https://github.com/Fraymakers/character-template
///
/// Key differences from our previous format:
/// - IMAGE symbols use `imageAsset` (GUID ref to .meta file) not `path`
/// - COLLISION_BOX keyframes reference symbol $ids (geometry in symbols)
/// - COLLISION_BODY keyframes reference symbol $ids
/// - All keyframes have `tweened`, `tweenType`, `pluginMetadata`
/// - Layers have `hidden`, `locked`, `pluginMetadata` (with box type metadata)
/// - Entity top-level has `pluginMetadata`, `plugins`, `version`, etc.
/// - Each PNG gets a .meta sidecar file with a GUID

use crate::extractor::CharacterData;
use crate::sprite_parser::{AnimationBoxData, BoxType};
use crate::image_extractor::ImageExtractionResult;
use serde_json::{json, Value};
use std::collections::BTreeMap;

// ─── UUID helpers ─────────────────────────────────────────────────────────────

fn det_uuid(seed: &str) -> String {
    crate::uuid_gen::det_uuid(seed)
}

fn uuid(char_id: &str, context: &str) -> String {
    det_uuid(&format!("{}::{}", char_id, context))
}

// ─── Box type → Fraymakers metadata type string ──────────────────────────────

fn box_type_to_fm(bt: BoxType) -> &'static str {
    match bt {
        BoxType::Hitbox     => "HIT_BOX",
        BoxType::Hurtbox    => "HURT_BOX",
        BoxType::GrabBox    => "GRAB_BOX",
        BoxType::ItemBox    => "NONE",  // no item-box concept in FM; a stray HURT_BOX here (esp. at
                                        // index 0, colliding with the real hitbox index) breaks the
                                        // entity's collision processing. NONE = a non-colliding box.
        // the source takes a hit here as shield damage, turning it back only off a perfect
        // shield; this engine's shield is its own state with no box to stand in for it, and as a
        // reflect box every hit on it came back
        BoxType::ShieldBox  => "NONE",
        BoxType::ReflectBox => "REFLECT_BOX",
        BoxType::AbsorbBox  => "ABSORB_BOX",
        BoxType::CounterBox => "COUNTER_BOX",
        BoxType::InertBox   => "NONE",
        BoxType::LedgeBox    => "LEDGE_GRAB_BOX",
        // GrabHoldBox uses a POINT layer, not COLLISION_BOX — handled separately.
        BoxType::GrabHoldBox  => "GRAB_HOLD_POINT",
    }
}

/// Convert an SSF2 instance name to a Fraymakers layer name.
/// SSF2 naming → FM naming:
///   attackBox  → hitbox0   (SSF2 "attack" = FM "hit")
///   attackBox2 → hitbox1
///   attackBox3 → hitbox2
///   hitBox     → hurtbox0  (SSF2 "hit" = FM "hurt")
///   hitBox2    → hurtbox1
///   hurtBox    → hurtbox0
///   grabBox    → grabbox0
///   ledgeBox   → ledgegrabbox0
///   etc.
fn ssf2_box_name_to_fm(inst_name: &str) -> String {
    let lower = inst_name.to_lowercase();

    // Extract numeric suffix: "attackBox2" → suffix=2, "hitBox" → suffix=0 (none = 0)
    // SSF2 uses 1-based suffixes (attackBox=first, attackBox2=second)
    // FM uses 0-based (hitbox0, hitbox1)
    let (prefix_lower, raw_num) = if let Some(pos) = lower.find(|c: char| c.is_ascii_digit()) {
        let num: usize = lower[pos..].parse().unwrap_or(1);
        (&lower[..pos], num)
    } else {
        (lower.as_str(), 1usize)
    };
    // Convert to 0-based: raw_num 1 → index 0, raw_num 2 → index 1
    let index = raw_num.saturating_sub(1);

    match prefix_lower {
        p if p.starts_with("attackbox") || p.starts_with("attack_box") =>
            format!("hitbox{}", index),
        p if p.starts_with("hitbox") || p.starts_with("hurtbox") =>
            format!("hurtbox{}", index),
        p if p.starts_with("grabbox") || p.starts_with("grab") =>
            format!("grabbox{}", index),
        p if p.starts_with("itembox") | p.starts_with("item_box") =>
            format!("itembox{}", index),
        p if p.starts_with("shieldbox") =>
            format!("shieldbox{}", index),
        p if p.starts_with("reflectbox") =>
            format!("reflectbox{}", index),
        p if p.starts_with("absorbbox") =>
            format!("absorbbox{}", index),
        p if p.starts_with("ledgebox") || p.starts_with("ledgegrab") =>
            format!("ledgegrabbox{}", index),
        p if p.starts_with("touchbox") =>
            format!("grabholdpoint{}", index),
        _ => format!("{}{}", prefix_lower.trim_end_matches('_'), index),
    }
}

/// The layer every box instance of one animation gets, as (prefix, index).
///
/// The engine knows a box by its layer's NAME as well as its type: a reflect box named after its
/// source instance (`reversebox0`) was typed REFLECT_BOX and still reflected nothing, while the
/// same box named `reflectbox0` turned the shot around. So a box that collides as something is
/// named for what it collides as, the way FrayTools names it (`HIT_BOX` -> `hitbox`), and only a
/// box that collides as nothing keeps a name taken from its source. Several source names can
/// land on one kind (a `hitBox` and a `hurtBox` are both hurt boxes); a name that already is its
/// kind's keeps its slot, and the others, and every repeat of a name, take the next free one.
fn box_layer_names(
    instances: &std::collections::BTreeSet<(String, usize)>,
) -> std::collections::BTreeMap<(String, usize), (String, usize)> {
    let wanted = |inst: &str| -> (String, usize) {
        let named = ssf2_box_name_to_fm(inst);
        let idx = fm_box_index(&named);
        let source_prefix = named[..named.len() - idx.to_string().len()].to_string();
        let bt = BoxType::from_instance_name(inst).unwrap_or(BoxType::Hurtbox);
        let prefix = match box_type_to_fm(bt) {
            t if t.ends_with("_BOX") => t.to_lowercase().replace('_', ""),
            _ => source_prefix,
        };
        (prefix, idx)
    };
    let mut out = std::collections::BTreeMap::new();
    let mut taken: std::collections::BTreeSet<(String, usize)> = Default::default();
    let firsts: Vec<&(String, usize)> = instances.iter().filter(|(_, occ)| *occ == 0).collect();
    // a name that already is its kind's first, so what the source named keeps its layer
    let (own, others): (Vec<_>, Vec<_>) = firsts.into_iter().partition(|(n, _)| {
        let (prefix, idx) = wanted(n);
        ssf2_box_name_to_fm(n) == format!("{prefix}{idx}")
    });
    for key in own.into_iter().chain(others) {
        let (prefix, mut idx) = wanted(&key.0);
        while taken.contains(&(prefix.clone(), idx)) { idx += 1; }
        taken.insert((prefix.clone(), idx));
        out.insert(key.clone(), (prefix, idx));
    }
    for key in instances.iter().filter(|(_, occ)| *occ > 0) {
        let (prefix, named_idx) = wanted(&key.0);
        let mut idx = named_idx + 1;
        while taken.contains(&(prefix.clone(), idx)) { idx += 1; }
        taken.insert((prefix.clone(), idx));
        out.insert(key.clone(), (prefix, idx));
    }
    out
}

/// Extract the numeric index from a Fraymakers box layer name.
/// "hitbox0" → 0, "hurtbox1" → 1, "grabbox0" → 0
fn fm_box_index(fm_name: &str) -> usize {
    fm_name.chars().rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars().rev()
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn box_color(bt: BoxType) -> &'static str {
    match bt {
        BoxType::Hitbox     => "0xff0000",
        BoxType::Hurtbox    => "0xfcba03",
        BoxType::GrabBox    => "0xff00ff",
        BoxType::ItemBox    => "0xffff00",
        BoxType::ShieldBox  => "0x48f748",
        BoxType::ReflectBox => "0x48f748",
        BoxType::AbsorbBox  => "0x42ecff",
        BoxType::CounterBox => "0x42ecff",
        BoxType::InertBox   => "0x888888",
        BoxType::LedgeBox    => "0xbababa",
        BoxType::GrabHoldBox => "0x9999ff",  // light blue
    }
}

// ─── Meta file generation ─────────────────────────────────────────────────────

/// Generate .meta JSON sidecar for a PNG image asset
pub fn generate_meta(guid: &str) -> String {
    serde_json::to_string_pretty(&json!({
        "export": false,
        "guid": guid,
        "id": "",
        "pluginMetadata": {},
        "plugins": [],
        "tags": [],
        "version": 2
    })).unwrap()
}

// ─── Main generator ───────────────────────────────────────────────────────────

/// Stretch every animation timeline 2x for the 30fps → 60fps move.
///
/// SSF2 plays at 30fps; Fraymakers runs at 60fps. To preserve each
/// animation's real-time playback speed, every keyframe must be held for two
/// Fraymakers frames instead of one. FrayTools timelines are laid out purely
/// by sequential keyframe `length`, so doubling every keyframe's `length`
/// doubles every layer's span and every keyframe's start position in
/// lockstep — image, collision-box, collision-body/ECB, frame-script and
/// label layers all scale together and cannot fall out of sync.
/// Fraymakers character-template animation order (docs 0.3.0). Used to (a) sort the
/// character's template animations into template order and (b) decide which animations are
/// "unused" — anything that is neither a template animation nor a direct (one-suffix) variant
/// of one, e.g. grab_pummel_out (parent grab_pummel isn't a template anim) or star_ko.
const TEMPLATE_ANIM_ORDER: &[&str] = &[
    "revival", "intro", "stand", "stand_turn", "walk_in", "walk_loop",
    "walk_out", "dash", "run", "run_turn", "skid", "jump_squat",
    "jump_in", "jump_loop", "jump_midair", "jump_out", "fall_in", "fall_loop",
    "land_light", "wall_jump_in", "wall_jump", "fall_special", "airdash_land", "airdash_freefall",
    "airdash_freefall_whiff", "land_heavy", "airdash_land_uncanceled", "airdash_land_whiff", "crouch_in", "crouch_loop",
    "crouch_out", "ledge_in", "ledge_loop", "ledge_climb_in", "ledge_climb", "ledge_roll_in",
    "ledge_roll", "ledge_jump_in", "ledge_jump", "ledge_attack_in", "ledge_attack", "shield_in",
    "shield_loop", "shield_hurt", "shield_out", "roll", "spot_dodge", "tech",
    "tech_roll", "hurt_light_low", "hurt_light_middle", "hurt_light_high", "hurt_medium", "hurt_heavy",
    "hurt_thrown", "tumble", "crash_bounce", "crash_loop", "crash_get_up", "crash_roll",
    "crash_attack", "airdash_forward", "airdash_forward_up", "airdash_forward_down", "airdash_up", "airdash_down",
    "airdash_back", "airdash_back_up", "airdash_back_down", "aerial_neutral", "aerial_neutral_land", "aerial_forward",
    "aerial_forward_land", "aerial_back", "aerial_back_land", "aerial_down", "aerial_down_land", "aerial_up",
    "aerial_up_land", "dash_attack", "jab1", "jab2", "jab3", "tilt_forward",
    "tilt_up", "tilt_down", "parry_in", "parry_fail", "parry_success", "strong_forward_in",
    "strong_forward_charge", "strong_forward_attack", "strong_up_in", "strong_up_charge", "strong_up_attack", "strong_down_in",
    "strong_down_charge", "strong_down_attack", "assist_call", "assist_call_air", "special_neutral", "special_neutral_air",
    "special_up", "special_up_air", "special_down", "special_down_loop", "special_down_endlag", "special_down_air",
    "special_down_air_loop", "special_down_air_endlag", "special_side", "special_side_air", "grab", "grab_hold",
    "throw_forward", "throw_back", "throw_up", "throw_down", "emote",
];

/// Reorder the entity's animations to match the character template, with all non-template,
/// non-variant animations grouped under a "---UNUSED ANIMATIONS BELOW---" separator.
fn reorder_animations(animations: &mut Vec<Value>, layers: &mut Vec<Value>, keyframes: &mut Vec<Value>, char_id: &str) {
    let idx = |name: &str| TEMPLATE_ANIM_ORDER.iter().position(|t| *t == name);
    let mut used: Vec<(usize, u8, Value)> = Vec::new();
    let mut unused: Vec<Value> = Vec::new();
    for a in animations.drain(..) {
        let name = a.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        // A direct variant shares the template anim as its immediate prefix (one suffix removed):
        // special_neutral_air→special_neutral, grab_pummel→grab. grab_pummel_out→grab_pummel (NOT a
        // template anim) so it is NOT a variant and lands in UNUSED.
        let parent_idx = name.rsplit_once('_').and_then(|(p, _)| idx(p));
        if let Some(i) = idx(&name) {
            used.push((i, 0, a));
        } else if let Some(i) = parent_idx {
            used.push((i, 1, a));
        } else {
            unused.push(a);
        }
    }
    used.sort_by_key(|(i, t, _)| (*i, *t));
    let mut out: Vec<Value> = used.into_iter().map(|(_, _, a)| a).collect();
    if !unused.is_empty() {
        let kf_id = uuid(char_id, "kf_unused_separator");
        let layer_id = uuid(char_id, "layer_unused_separator");
        let anim_id = uuid(char_id, "anim_unused_separator");
        keyframes.push(json!({ "$id": kf_id, "type": "FRAME_SCRIPT", "length": 1, "code": "", "pluginMetadata": {} }));
        layers.push(json!({ "$id": layer_id, "name": "Scripts", "type": "FRAME_SCRIPT",
            "keyframes": [kf_id], "hidden": false, "locked": false, "language": "", "pluginMetadata": {} }));
        out.push(json!({ "$id": anim_id, "name": "---UNUSED ANIMATIONS BELOW---",
            "layers": [layer_id], "pluginMetadata": {} }));
        let (ordered_count, unused_count) = (out.len() - 1, unused.len());
        out.extend(unused);
        log::info!("reorder: {} template-ordered animations, {} unused (below separator)",
            ordered_count, unused_count);
    }
    *animations = out;
}

/// Append every character-template animation that did NOT make it into the entity
/// (never converted, or dropped as empty) as an empty stub, grouped at the very
/// bottom under a blank "====MISSING ANIMATIONS====" separator. This surfaces the
/// gaps for the modder (e.g. tumble, the extra jabs, parry, the special-down loop)
/// with the same empty template shape they have in the official character template,
/// instead of silently omitting them. Call AFTER drop + reorder so the stubs are
/// neither dropped nor sorted back into the main list.
fn append_missing_template_animations(
    animations: &mut Vec<Value>, layers: &mut Vec<Value>, keyframes: &mut Vec<Value>, char_id: &str,
    image_slots: usize,
) {
    let present: std::collections::BTreeSet<String> = animations.iter()
        .filter_map(|a| a.get("name").and_then(Value::as_str).map(String::from))
        .collect();
    let missing: Vec<String> = crate::mappings::character_animation_template().iter()
        .map(|e| e.name.clone())
        .filter(|n| !present.contains(n))
        .collect();
    if missing.is_empty() { return; }

    // Blank separator animation (a single empty frame-script layer, like the UNUSED one).
    let sep_kf = uuid(char_id, "kf_missing_separator");
    let sep_layer = uuid(char_id, "layer_missing_separator");
    keyframes.push(json!({ "$id": sep_kf, "type": "FRAME_SCRIPT", "length": 1, "code": "", "pluginMetadata": {} }));
    layers.push(json!({ "$id": sep_layer, "name": "Scripts", "type": "FRAME_SCRIPT",
        "keyframes": [sep_kf], "hidden": false, "locked": false, "language": "", "pluginMetadata": {} }));
    animations.push(json!({ "$id": uuid(char_id, "anim_missing_separator"),
        "name": "====MISSING ANIMATIONS====", "layers": [sep_layer], "pluginMetadata": {} }));

    for name in &missing {
        // A stub still has to address every image slot. These stand in for moves the character
        // does not have, but the states are real and can be entered, and an animation that leaves
        // a layer unaddressed shows whatever the last animation put there -- so a missing move
        // would wear the leftovers of whatever preceded it.
        let mut layer_ids: Vec<String> = Vec::new();
        for slot in 0..image_slots.max(1) {
            let kf_id = uuid(char_id, &format!("kf_missing_{}_{}", name, slot));
            let layer_id = uuid(char_id, &format!("layer_missing_{}_{}", name, slot));
            keyframes.push(json!({ "$id": kf_id, "type": "IMAGE", "length": 1, "symbol": Value::Null,
                "tweened": false, "tweenType": "LINEAR", "pluginMetadata": {} }));
            layers.push(json!({ "$id": layer_id, "name": format!("Image {}", slot), "type": "IMAGE",
                "keyframes": [kf_id], "hidden": false, "locked": false, "pluginMetadata": {} }));
            layer_ids.push(layer_id);
        }
        animations.push(json!({ "$id": uuid(char_id, &format!("anim_missing_{}", name)),
            "name": name, "layers": layer_ids, "pluginMetadata": {} }));
    }
    log::info!("Added {} missing template animation stub(s) under ====MISSING ANIMATIONS====: {}",
        missing.len(), missing.join(", "));
}

/// Double every keyframe `length` for the SSF2 30fps -> Fraymakers 60fps move (one source frame =
/// two FM frames). FrayTools timelines are laid out purely by sequential keyframe length, so
/// doubling each keyframe doubles every layer span + keyframe start in lockstep. Shared by the
/// stage emitter so stage/hazard animations get the SAME timing treatment as characters.
/// SSF2 runs at 30fps and Fraymakers at 60, so every ported timeline is twice as long. Anything
/// comparing against a frame NUMBER from the source has to travel the same distance.
pub const FRAME_RATIO: u64 = 2;

pub(crate) fn double_keyframe_lengths(keyframes: &mut [Value]) {
    for kf in keyframes {
        if let Some(len) = kf.get("length").and_then(Value::as_u64) {
            kf["length"] = json!(len * FRAME_RATIO);
        }
    }
}

/// The SSF2 animation a Fraymakers animation was converted from.
///
/// A SPLIT piece has to ask for its parent: the mapping files say `a` becomes `jab`, never
/// `jab1`, so looking one up by the piece name finds nothing -- and "nothing" is indistinguishable
/// from "no scripts", which is how sandbag's jab ended up with no frame scripts at all and no idea
/// where SSF2 leaves it.
pub(crate) fn ssf2_source_name(data: &CharacterData, source_anim: &str, anim_name: &str) -> Option<String> {
    let mut names: Vec<&str> = vec![source_anim, anim_name];
    for n in [source_anim, anim_name] {
        if let Some(parent) = crate::extractor::split_parent(n) { names.push(parent); }
    }
    for want in names {
        if let Some((ssf2, _)) = data.ssf2_to_fm_anim.iter().find(|(_, fm)| fm.as_str() == want) {
            return Some(ssf2.clone());
        }
    }
    None
}

/// Serialize a finished entity, with its redundancy removed first.
///
/// Every entity goes out through here so no emitter has to remember to compact -- see
/// [`crate::entity_compact`] for what "redundancy" means and why removing it is safe.
fn finish_entity(mut entity: Value) -> String {
    let st = crate::entity_compact::compact(&mut entity);
    if st.saved_anything() {
        log::debug!("compacted entity: symbols {} -> {}, keyframes {} -> {} ({} blanked)",
            st.symbols_before, st.symbols_after, st.keyframes_before, st.keyframes_after, st.blanked);
    }
    serde_json::to_string_pretty(&entity).unwrap_or_else(|_| "{}".to_string())
}

thread_local! {
    static PROJECTILE_PIECES: std::cell::RefCell<BTreeMap<String, Vec<crate::emit::Piece>>> = Default::default();
}
/// For the runtime's path: which source frames each projectile entity's animations show, by the
/// entity's id, as the entity was written.
pub fn take_projectile_pieces() -> BTreeMap<String, Vec<crate::emit::Piece>> {
    PROJECTILE_PIECES.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

thread_local! {
    static EFFECT_PIECES: std::cell::RefCell<BTreeMap<String, Vec<crate::emit::Piece>>> = Default::default();
    static BORROWING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Effects built while this is on come from another package: their symbols are that package's
/// numbers, which name nothing (or something else) here, so their pieces carry no symbol.
pub fn set_borrowing(on: bool) { BORROWING.with(|c| c.set(on)); }
/// The same for the effects the conversion wrote, by the effect's linkage name.
pub fn take_effect_pieces() -> BTreeMap<String, Vec<crate::emit::Piece>> {
    EFFECT_PIECES.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

pub fn generate_entity(
    data: &CharacterData,
    char_id: &str,
    sprite_boxes: &BTreeMap<String, AnimationBoxData>,
    img_result: &ImageExtractionResult,
    // How many of jab1..jab4 carry real image content. Decides which empty
    // animations are kept as placeholders for Script.hx jab-chain references.
    populated_jabs: usize,
) -> String {
    let mut keyframes: Vec<Value> = Vec::new();
    let mut layers: Vec<Value> = Vec::new();
    let mut symbols: Vec<Value> = Vec::new();
    let mut animations: Vec<Value> = Vec::new();

    // ── Build image asset GUID map (symbol_name → meta GUID) ──────────────────
    // Each image gets a deterministic GUID for its .meta file
    let mut image_guids: BTreeMap<String, String> = BTreeMap::new();
    for img in img_result.images.values() {
        let meta_guid = crate::uuid_gen::image_meta_guid(&img.symbol_name);
        image_guids.insert(img.symbol_name.clone(), meta_guid);
    }

    // IMAGE symbols are now created per-placement (per anim/frame) to carry correct
    // world-space x/y/scaleX/scaleY from the root MC transform + local sub-sprite matrix.
    // We collect them during the animation loop below.
    // (The old per-bitmap shared symbol approach is replaced.)

    // ── Apply animation splits ────────────────────────────────────────────────
    // The splitter expands multi-label SSF2 animations into separate FM animations.
    // Each SplitAnim carries (fm_name, source_anim, start_frame, end_frame, loop info).
    let split_anims = crate::anim_splitter::split_animations(&data.animations, sprite_boxes, data.stats.jump_startup.round().max(0.0) as u16, &data.scripts);

    crate::api_mappings::set_current_labels(&[]);
    for split in &split_anims {
        let anim_name = &split.fm_name;
        let source_anim = &split.source_anim;
        let anim_info = data.animations.get(source_anim)
            .or_else(|| data.animations.get(anim_name));
        // frame_count is the length of this split slice
        let split_len = if split.end_frame == u16::MAX {
            sprite_boxes.get(source_anim)
                .map(|sb| sb.total_frames as u32)
                .or_else(|| anim_info.map(|ai| ai.frames as u32))
                .unwrap_or(1)
                .saturating_sub(split.start_frame as u32)
        } else {
            (split.end_frame as u32).saturating_sub(split.start_frame as u32)
        };
        let base_len = split_len.max(1);
        let head = split.append_head_frames as u32;     // extra frames taken from source [0, head)
        // every frame is kept: the runtime runs the source's frame scripts itself, and a frame
        // whose script ends the move is never shown because that script runs before the frame is
        let frame_count = base_len + head;               // total emitted frames
        let split_start_u = split.start_frame as u32;
        // Map a logical (emitted) frame index to its SOURCE frame: the slice maps to
        // [start, end); any appended frames wrap back to the source head [0, head).
        let src_frame = move |f: u32| -> u16 {
            if f < base_len { (split_start_u + f) as u16 } else { (f - base_len) as u16 }
        };
        // the boxes of a frame. the sprite parser may have split this slice off already and kept
        // its boxes under the slice's own name, counted from its first frame (pikachu's skull
        // bash flies on `special_side_attack`, frames 13.. of `b_forward`): read there first, or
        // the slice looks for them past the end of the part left under the source's name and
        // has no boxes at all
        let own_boxes = anim_name != source_anim && split_start_u > 0
            && sprite_boxes.contains_key(anim_name.as_str());
        let boxes_at = |f: u32| -> Option<&Vec<crate::sprite_parser::FrameBox>> {
            if own_boxes && f < base_len {
                sprite_boxes.get(anim_name.as_str()).and_then(|bd| bd.frames.get(&(f as u16)))
            } else {
                sprite_boxes.get(source_anim.as_str()).and_then(|bd| bd.frames.get(&src_frame(f)))
            }
        };
        let has_boxes = own_boxes || sprite_boxes.contains_key(source_anim.as_str());

        let anim_id = uuid(char_id, &format!("anim_{}", anim_name));
        let mut anim_layer_ids: Vec<String> = Vec::new();
        // IMAGE layers are collected separately so they can be emitted FIRST in
        // the final layer list. In FrayTools, lower index = drawn first = behind;
        // putting image layers before the collision boxes (hurt/hit/body/item/etc.)
        // keeps the sprite behind the boxes so they stay visible when editing.
        let mut anim_image_layer_ids: Vec<String> = Vec::new();

        // ── 1. LABEL layer ────────────────────────────────────────────────────
        // Frame 0 always gets the FM animation name as a label.
        // Additional SSF2 inner sprite labels are placed at their original frame offsets.
        {
            let layer_id = uuid(char_id, &format!("layer_label_{}", anim_name));
            let mut label_kf_ids: Vec<Value> = Vec::new();

            // Use pre-sliced labels from the splitter (already rebased to this split's frame 0)
            // Frame 0 always gets the FM animation name; splitter labels augment from there.
            // a source label later in the animation that shares its name is the one a jump by that
            // name means (the engine goes to the first label it finds): the first frame then goes
            // unnamed (waluigi's dice jumps to its `loop` label, inside its `loop` animation)
            let shadowed = split.labels.iter().any(|(l, f)| *f > 0 && l == anim_name);
            let mut all_labels: Vec<(u16, String)> = vec![(0, if shadowed { String::new() } else { anim_name.to_string() })];
            for (lbl, frame) in &split.labels {
                if *frame == 0 { continue; } // don't overwrite the fm_name at frame 0
                // a label on a frame the terminator trim removed names the end of the move: it
                // lands on the last frame, which plays and ends, rather than stretching the
                // animation back out to a frame the source never shows
                let frame = (*frame as u32).min(frame_count.saturating_sub(1)) as u16;
                all_labels.push((frame, lbl.clone()));
            }
            all_labels.sort_by_key(|(f, _)| *f);
            all_labels.dedup_by_key(|(f, _)| *f);

            // Emit LABEL keyframes with correct lengths to fill the timeline
            let mut cursor: u32 = 0;
            for (i, (frame, label)) in all_labels.iter().enumerate() {
                let f = *frame as u32;
                // Gap before this label (unlabeled frames)
                if f > cursor {
                    let gap_id = uuid(char_id, &format!("kf_label_gap_{}_{}", anim_name, cursor));
                    keyframes.push(json!({
                        "$id": gap_id,
                        "type": "LABEL",
                        "length": f - cursor,
                        "name": "",
                        "pluginMetadata": {}
                    }));
                    label_kf_ids.push(gap_id.into());
                }
                // This label's keyframe — length extends to next label or end of animation
                let next_frame = all_labels.get(i + 1).map(|(nf, _)| *nf as u32).unwrap_or(frame_count);
                let length = (next_frame - f).max(1);
                let kf_id = uuid(char_id, &format!("kf_label_{}_f{}", anim_name, f));
                keyframes.push(json!({
                    "$id": kf_id,
                    "type": "LABEL",
                    "length": length,
                    "name": label,
                    "pluginMetadata": {}
                }));
                label_kf_ids.push(kf_id.into());
                cursor = f + length;
            }

            layers.push(json!({
                "$id": layer_id,
                "name": "Labels",
                "type": "LABEL",
                "keyframes": label_kf_ids,
                "hidden": false,
                "locked": false,
                "pluginMetadata": {}
            }));
            anim_layer_ids.push(layer_id);
        }

        // ── 2. FRAME_SCRIPT layer ─────────────────────────────────────────────
        // one blank keyframe spanning the animation: the runtime runs the source's frame scripts
        // itself, and the fighter's script puts its own code on keyframes (`emit::clock_entity`)
        {
            let layer_id = uuid(char_id, &format!("layer_script_{}", anim_name));
            let tail_kf_id = uuid(char_id, &format!("kf_script_tail_{}", anim_name));
            keyframes.push(json!({
                "$id": tail_kf_id,
                "type": "FRAME_SCRIPT",
                "length": frame_count,
                "code": "",
                "pluginMetadata": {}
            }));
            let script_kf_ids = vec![tail_kf_id];
            layers.push(json!({
                "$id": layer_id,
                "name": "Scripts",
                "type": "FRAME_SCRIPT",
                "keyframes": script_kf_ids,
                "hidden": false,
                "locked": false,
                "language": "",
                "pluginMetadata": {}
            }));
            anim_layer_ids.push(layer_id);
        }

        // The launched-state slots (tumble, hurt_heavy) anchor the BODY on the
        // entity position -- the official character template's convention -- but
        // their SSF2 source xframes carry root-timeline stance placements that
        // are preview positions, not runtime anchors (mario's "flying" sits a
        // full body-height BELOW the origin; every grounded xframe sits feet-at-
        // origin). Correction: move the hurtbox union's centre onto the origin
        // and shift the whole box set (hurtboxes, itemboxes, ECB) by that one
        // delta. The IMAGE placement applies the same correction (tumble
        // re-centres per-frame because it also strips rotation; hurt_heavy
        // translates by this exact shift, keeping art-box registration intact).
        // Every other animation keeps its placement and boxes verbatim.
        let box_shift: (f64, f64) = if anim_name == "tumble" || anim_name == "hurt_heavy" {
            let mut aabb: Option<(f64, f64, f64, f64)> = None; // left,right,top,bottom
            {
                for lf in 0..frame_count {
                    if let Some(boxes) = boxes_at(lf) {
                        for b in boxes {
                            if b.box_type != crate::sprite_parser::BoxType::Hurtbox { continue; }
                            let (l, r, t, btm) = (b.x, b.x + b.width, b.y, b.y + b.height);
                            aabb = Some(match aabb {
                                None => (l, r, t, btm),
                                Some((al, ar, at, ab)) => (al.min(l), ar.max(r), at.min(t), ab.max(btm)),
                            });
                        }
                    }
                }
            }
            aabb.map(|(l, r, t, b)| (-(l + r) / 2.0, -(t + b) / 2.0)).unwrap_or((0.0, 0.0))
        } else {
            (0.0, 0.0)
        };

        // ── 3. COLLISION_BODY layer (per-frame ECB from hurtbox bounds) ───────
        {
            let layer_id = uuid(char_id, &format!("layer_body_{}", anim_name));

            // FrayTools draws the COLLISION_BODY (ECB) as a 4-vertex diamond:
            //   foot vertex (0, -foot),  head vertex (0, -head),
            //   hip vertices (±hipWidth/2 + hipXOffset, -(head+foot)/2 + hipYOffset)
            // foot/head are heights measured UP from the character origin; the
            // hip's vertical position is an offset from the foot/head midpoint.
            // The foot and head vertices are pinned to x = 0.
            //
            // Per frame we fit the diamond to the union AABB of that frame's
            // HURTBOX-typed boxes: head at its top, hips at its sides at mid-height,
            // and the foot at the fighter's own point (the floor is tested there).
            // hipXOffset re-centres the hip on the box; hipYOffset stays 0.
            let default_body = (0.0_f64, 86.0_f64, 40.0_f64, 0.0_f64); // foot,head,hipWidth,hipXOffset

            let mut per_frame: Vec<(f64, f64, f64, f64)> = Vec::with_capacity(frame_count as usize);
            {
                // Per-frame body from the frame's HURTBOX union, or None when the
                // frame has NO hurtboxes at all.
                let raw: Vec<Option<(f64, f64, f64, f64)>> = (0..frame_count).map(|f| {
                    let mut aabb: Option<(f64, f64, f64, f64)> = None; // left,right,top,bottom
                    if let Some(boxes) = boxes_at(f) {
                        for b in boxes {
                            if b.box_type != crate::sprite_parser::BoxType::Hurtbox { continue; }
                            let (l, r) = (b.x + box_shift.0, b.x + b.width + box_shift.0);
                            let (t, btm) = (b.y + box_shift.1, b.y + b.height + box_shift.1);
                            aabb = Some(match aabb {
                                None => (l, r, t, btm),
                                Some((al, ar, at, ab)) => (al.min(l), ar.max(r), at.min(t), ab.max(btm)),
                            });
                        }
                    }
                    // the foot is the fighter's own point: the source tests the floor there in
                    // every state, where a foot at the hurtboxes' bottom edge sat 14 above it in a
                    // curled-up hurt frame and landed pikachu's fighter on a platform its feet
                    // never reached (its head stays at least a pixel above it)
                    aabb.map(|(l, r, t, _btm)| (
                        0.0,                   // foot  = the fighter's own point
                        round2((-t).max(1.0)), // head  = up-height of the top edge
                        round2(r - l),         // hipWidth = box width
                        round2((l + r) / 2.0), // hipXOffset = box centre x
                    ))
                }).collect();
                // A frame with NO hurtboxes keeps the ECB exactly as it was — hold the
                // previous body so the RLE below emits NO new keyframe for it (the ECB
                // persists until the next hurtbox frame). Leading no-hurtbox frames
                // backfill from the first real body, so the ECB never shows a default
                // diamond mid-animation. All-empty animation → default.
                let first_real = raw.iter().flatten().next().copied();
                let mut last = first_real.unwrap_or(default_body);
                for b in &raw {
                    if let Some(bb) = b { last = *bb; }
                    per_frame.push(last);
                }
            }

            // Run-length encode consecutive frames with an identical body.
            let mut body_kf_ids: Vec<String> = Vec::new();
            let mut f: u32 = 0;
            while f < frame_count {
                let body = per_frame[f as usize];
                let mut run = 1u32;
                while (f + run) < frame_count {
                    let nb = per_frame[(f + run) as usize];
                    let same = (nb.0 - body.0).abs() < 0.01 && (nb.1 - body.1).abs() < 0.01
                        && (nb.2 - body.2).abs() < 0.01 && (nb.3 - body.3).abs() < 0.01;
                    if same { run += 1; } else { break; }
                }
                let sym_id = uuid(char_id, &format!("sym_body_{}_{}", anim_name, f));
                let kf_id = uuid(char_id, &format!("kf_body_{}_{}", anim_name, f));
                // The ECB is a 4-vertex diamond: foot (0,-foot), head (0,-head), hips
                // (hipXOffset ± hipWidth/2, midY). The foot/head verts are PINNED to
                // x=0, so for a convex diamond the hips must straddle x=0, i.e.
                // |hipXOffset| < hipWidth/2. When the hurtbox union is far off-centre
                // (e.g. zelda fall, centre ~-38 vs width ~38) the raw offset pushes both
                // hips to one side and the shape collapses into an ARROW — which renders
                // wrong and crashes FrayTools on edit. Clamp the offset to keep the hips
                // straddling the centre (a comfortable margin) so it's always a valid
                // diamond.
                let half_w = (body.2 / 2.0).max(0.0);
                let max_off = (half_w - half_w * 0.25).max(0.0); // hips keep ≥25% past centre
                let hip_x_offset = round2(body.3.clamp(-max_off, max_off));
                symbols.push(json!({
                    "$id": sym_id,
                    "alpha": Value::Null,
                    "color": Value::Null,
                    "foot": body.0,
                    "head": body.1,
                    "hipWidth": body.2,
                    "hipXOffset": hip_x_offset,
                    "hipYOffset": 0,
                    "pluginMetadata": {},
                    "type": "COLLISION_BODY"
                }));
                keyframes.push(json!({
                    "$id": kf_id,
                    "type": "COLLISION_BODY",
                    "length": run,
                    "symbol": sym_id,
                    "tweened": false,
                    "tweenType": "LINEAR",
                    "pluginMetadata": {}
                }));
                body_kf_ids.push(kf_id);
                f += run;
            }

            layers.push(json!({
                "$id": layer_id,
                "name": "Body",
                "type": "COLLISION_BODY",
                "keyframes": body_kf_ids,
                "hidden": false,
                "locked": false,
                "defaultAlpha": 0.5,
                "defaultColor": "0xffa500",
                "defaultFoot": 0,
                "defaultHead": 86,
                "defaultHipWidth": 40,
                "defaultHipXOffset": 0,
                "defaultHipYOffset": 0,
                "pluginMetadata": {}
            }));
            anim_layer_ids.push(layer_id);
        }

        // ── 4. COLLISION_BOX layers ───────────────────────────────────────────
        if has_boxes {
            // Walk logical frames 0..frame_count and resolve each to its source frame (handles
            // both the [start,end) slice and any appended head frames for a wrapped loop).
            // A name is not unique on a frame. SSF2 numbers a second box by its name
            // (`attackBox`, `attackBox2`), but nothing makes it: kirby's drill puts TWO plain
            // `attackBox` boxes on each active frame. Keyed on the name alone, both wanted the
            // same layer and the second was dropped -- the drill hit with one box where the
            // source hits with two. So a layer is a name AND which occurrence of it this is,
            // and the occurrence takes the next free slot for its kind.
            let mut instances_in_anim: std::collections::BTreeSet<(String, usize)> = Default::default();
            for lf in 0..frame_count {
                if let Some(boxes) = boxes_at(lf) {
                    let mut seen: std::collections::BTreeMap<&str, usize> = Default::default();
                    for b in boxes {
                        let n = seen.entry(b.instance_name.as_str()).or_insert(0);
                        instances_in_anim.insert((b.instance_name.clone(), *n));
                        *n += 1;
                    }
                }
            }
            // slots the source NAMED are reserved first, so a repeat never lands on one
            let layer_names = box_layer_names(&instances_in_anim);

            for (inst_name, occurrence) in instances_in_anim.iter() {
                let occurrence = *occurrence;
                let box_type = BoxType::from_instance_name(inst_name).unwrap_or(BoxType::Hurtbox);
                let fm_box_type = box_type_to_fm(box_type);
                let color = box_color(box_type);
                let (prefix, box_idx) = layer_names[&(inst_name.clone(), occurrence)].clone();
                let fm_layer_name = format!("{prefix}{box_idx}");
                // the first box of a name keeps the key it always had, so only a character that
                // actually repeats one sees its ids move
                let occ_key = if occurrence == 0 { String::new() } else { format!("_{occurrence}") };
                let layer_id = uuid(char_id, &format!("layer_box_{}_{}{}", anim_name, inst_name, occ_key));

                let mut box_kf_ids: Vec<String> = Vec::new();
                let total = frame_count; // use split length, not source total
                let mut frame_idx: u32 = 0;

                // Collect this instance's boxes per logical frame (rebased to 0..frame_count).
                let mut active_frames: Vec<(u16, &crate::sprite_parser::FrameBox)> = Vec::new();
                for lf in 0..frame_count {
                    if let Some(boxes) = boxes_at(lf) {
                        if let Some(b) = boxes.iter().filter(|b| b.instance_name == *inst_name)
                            .nth(occurrence)
                        {
                            active_frames.push((lf as u16, b));
                        }
                    }
                }
                active_frames.sort_by_key(|(f, _)| *f);

                if active_frames.is_empty() { continue; }

                // Build runs: merge consecutive frames with identical geometry
                // into one keyframe; a gap in frame numbers means a blank keyframe.
                // This correctly handles RemoveObject (box absent on frame N even
                // if it was present on N-1 and reappears on N+1).
                let is_point = box_type == crate::sprite_parser::BoxType::GrabHoldBox;
                let kf_type = if is_point { "POINT" } else { "COLLISION_BOX" };

                let mut i = 0;
                while i < active_frames.len() {
                    let (start_frame, fb) = active_frames[i];

                    // Merge consecutive frames with the same box geometry
                    let mut run_end = i; // inclusive index of last frame in run
                    while run_end + 1 < active_frames.len() {
                        let (next_f, next_fb) = active_frames[run_end + 1];
                        let (cur_f, _) = active_frames[run_end];
                        // Must be exactly the next frame AND same geometry
                        let consecutive = next_f == cur_f + 1;
                        let same_geom = (next_fb.x - fb.x).abs() < 0.01
                            && (next_fb.y - fb.y).abs() < 0.01
                            && (next_fb.width - fb.width).abs() < 0.01
                            && (next_fb.height - fb.height).abs() < 0.01
                            && (next_fb.rotation - fb.rotation).abs() < 0.01;
                        if consecutive && same_geom {
                            run_end += 1;
                        } else {
                            break;
                        }
                    }
                    let run_len = (active_frames[run_end].0 - start_frame + 1) as u32;

                    // Gap keyframe (blank) before this run
                    if start_frame as u32 > frame_idx {
                        let gap_kf_id = uuid(char_id, &format!("kf_box_gap_{}_{}{}_{}", anim_name, inst_name, occ_key, frame_idx));
                        keyframes.push(json!({
                            "$id": gap_kf_id,
                            "type": kf_type,
                            "length": (start_frame as u32) - frame_idx,
                            "symbol": Value::Null,
                            "tweened": false,
                            "tweenType": "LINEAR",
                            "pluginMetadata": {}
                        }));
                        box_kf_ids.push(gap_kf_id);
                        // frame_idx updated at end of loop body
                    }

                    let sym_id = uuid(char_id, &format!("sym_box_{}_{}{}_{}", anim_name, inst_name, occ_key, start_frame));

                    if is_point {
                        // POINT symbol: bottom-center of the touchBox.
                        // In SSF2, touchBox marks the hold region; the grab hold
                        // position is at the bottom-center (where the opponent's feet anchor).
                        let cx = round2(fb.x + box_shift.0 + fb.width / 2.0);
                        let cy = round2(fb.y + box_shift.1 + fb.height);
                        symbols.push(json!({
                            "$id": sym_id,
                            "alpha": 1,
                            "color": color,
                            "pluginMetadata": {},
                            "rotation": 0,
                            "type": "POINT",
                            "x": cx,
                            "y": cy
                        }));
                    } else {
                        // COLLISION_BOX symbol. FrayTools rotates a box around
                        // (x + pivotX, y + pivotY) and HONORS off-center pivots
                        // (verified: a 41x21 menu box round-trips through FrayTools
                        // keeping pivot (18,18), not the center (20.5,10.5)). So we
                        // place the pivot at the true SSF2 anchor:
                        //   - itemBox → bottom-center (the hand attachment point),
                        //     so the box sweeps around the hand as it rotates.
                        //   - every other box → geometric center.
                        // (x, y) is the un-rotated top-left (Theory A, confirmed by
                        // matching framy hurtboxes against the body image).
                        let (pivot_x, pivot_y) = if box_type == crate::sprite_parser::BoxType::ItemBox {
                            (fb.width / 2.0, fb.height)        // bottom-center = hand
                        } else {
                            (fb.width / 2.0, fb.height / 2.0)  // center
                        };
                        // SWF `atan2(b, a)` in y-down is CW-positive; FrayTools uses
                        // the same convention (see f472a2dd for the IMAGE path), so
                        // emit the angle as-is, normalized to 0-360.
                        let theta = ((fb.rotation % 360.0) + 360.0) % 360.0;

                        // ── itemBox anchor bake (measured, not guessed) ──
                        // The itemBox is the only box that carries rotation (all
                        // others are AABB-collapsed to 0 in sprite_parser).
                        // FrayTools renders a COLLISION_BOX's registration anchor
                        // at  s + R(-θ)·pivot  (docs/fraytools_internals.md §2) —
                        // it MOVES with θ. Emitting the raw top-left lets the hand
                        // drift up to ~49px on rotated frames (measured by
                        // `probe_itembox`). anchor(x,y) is affine in (x,y), so to
                        // pin the hand we solve  topleft = hand − anchor(0,0,pivot,θ).
                        // At θ=0 this collapses to (fb.x, fb.y), so non-rotated
                        // boxes stay byte-identical.
                        let (box_x, box_y) = if box_type == crate::sprite_parser::BoxType::ItemBox {
                            let hand = crate::fraytools_transform::intended_pivot_point(
                                fb.x, fb.y, pivot_x, pivot_y);
                            let off = crate::fraytools_transform::collision_box_anchor(
                                0.0, 0.0, pivot_x, pivot_y, theta);
                            (hand.0 - off.0, hand.1 - off.1)
                        } else {
                            (fb.x, fb.y)
                        };
                        symbols.push(json!({
                            "$id": sym_id,
                            "alpha": 0.5,
                            "color": color,
                            "pivotX": round2(pivot_x),
                            "pivotY": round2(pivot_y),
                            "pluginMetadata": {},
                            "rotation": round2(theta),
                            "scaleX": round2(fb.width),
                            "scaleY": round2(fb.height),
                            "type": "COLLISION_BOX",
                            "x": round2(box_x + box_shift.0),
                            "y": round2(box_y + box_shift.1)
                        }));
                    }

                    let kf_id = uuid(char_id, &format!("kf_box_{}_{}{}_{}", anim_name, inst_name, occ_key, start_frame));
                    keyframes.push(json!({
                        "$id": kf_id,
                        "type": kf_type,
                        "length": run_len,
                        "symbol": sym_id,
                        "tweened": false,
                        "tweenType": "LINEAR",
                        "pluginMetadata": {}
                    }));
                    box_kf_ids.push(kf_id);
                    frame_idx = start_frame as u32 + run_len;
                    i = run_end + 1;
                }

                // Tail gap
                if frame_idx < total {
                    let tail_kf_id = uuid(char_id, &format!("kf_box_tail_{}_{}{}_{}", anim_name, inst_name, occ_key, frame_idx));
                    keyframes.push(json!({
                        "$id": tail_kf_id,
                        "type": kf_type,
                        "length": total - frame_idx,
                        "symbol": Value::Null,
                        "tweened": false,
                        "tweenType": "LINEAR",
                        "pluginMetadata": {}
                    }));
                    box_kf_ids.push(tail_kf_id);
                }

                if box_kf_ids.is_empty() { continue; }

                if box_type == crate::sprite_parser::BoxType::GrabHoldBox {
                    // grabHoldPoint uses a POINT layer, not COLLISION_BOX
                    layers.push(json!({
                        "$id": layer_id,
                        "name": fm_layer_name,
                        "type": "POINT",
                        "keyframes": box_kf_ids,
                        "hidden": false,
                        "locked": false,
                        "pluginMetadata": {
                            "com.fraymakers.FraymakersMetadata": {
                                "pointType": "GRAB_HOLD_POINT"
                            }
                        }
                    }));
                } else {
                    layers.push(json!({
                        "$id": layer_id,
                        "name": fm_layer_name,
                        "type": "COLLISION_BOX",
                        "keyframes": box_kf_ids,
                        "hidden": false,
                        "locked": false,
                        "defaultAlpha": 0.5,
                        "defaultColor": color,
                        "pluginMetadata": {
                            "com.fraymakers.FraymakersMetadata": {
                                "collisionBoxType": fm_box_type,
                                "index": box_idx
                            }
                        }
                    }));
                }
                anim_layer_ids.push(layer_id);
            }
        }

        // ── 5. IMAGE layers (one per depth slot, back-to-front) ────────────────────
        {
            // Every animation addresses the SAME number of slots: the deepest any of them needs.
            // The ones it uses carry its images; the rest are blanked below.
            let entity_image_slots = img_result.anim_images.values()
                .map(|a| a.max_depth_slots.max(1)).max().unwrap_or(1);
            // the pictures of a slice the image pass split off itself are kept under the slice's
            // own name, counted from its first frame, as its boxes are (luigi's side special:
            // under the source's name only the charge's 15 frames are left, and the attack drew
            // the charge again and again where it stretches out)
            let own_images = anim_name != source_anim && split_start_u > 0
                && img_result.anim_images.contains_key(anim_name.as_str());
            let image_key = if own_images { anim_name.as_str() } else { source_anim.as_str() };
            let image_frame = |f: u32| -> u16 { if own_images && f < base_len { f as u16 } else { src_frame(f) } };
            let num_slots = img_result.anim_images.get(image_key)
                .map(|a| a.max_depth_slots.max(1))
                .unwrap_or(1);

            for slot in 0..num_slots {
                let img_layer_id = uuid(char_id, &format!("layer_image_{}_{}", anim_name, slot));
                let mut img_kf_ids: Vec<String> = Vec::new();

                if let Some(anim_imgs) = img_result.anim_images.get(image_key) {
                    let total = frame_count;

                    // LOOP the visual timeline like Flash does for a nested MovieClip
                    // whose frame count is shorter than the parent (gameplay) timeline.
                    // SSF2's `fall` is an 8-frame looping sprite running under a 20-frame
                    // hurtbox timeline, so source frames past the sprite's last frame wrap
                    // modulo its length (frame 8 → sprite frame 0) instead of coming back
                    // empty and making the character vanish mid-animation. When the sprite
                    // and gameplay timelines are equal length this is the identity.
                    let img_len = anim_imgs.frames.keys().max()
                        .map(|m| *m as u32 + 1).unwrap_or(1).max(1);
                    let mut held: Vec<Option<crate::image_extractor::FrameImageEntry>> = (0..total)
                        .map(|f| {
                            let looped = (image_frame(f) as u32 % img_len) as u16;
                            anim_imgs.frames.get(&looped).and_then(|v| v.get(slot)).cloned()
                        })
                        .collect();
                    // An exit slot filled from its entry animation plays the entry BACKWARDS.
                    if split.reversed { held.reverse(); }

                    // Each frame's picture and where the entity places it: x, y, scale x and y,
                    // turn and opacity.
                    //
                    // FrayTools IMAGE-symbol model:
                    //   world(L) = (x, y) + R(rotation)·diag(sx,sy)·L
                    // The bitmap's local (0,0) is placed at (x, y) and the image rotates/scales
                    // around it. The SWF places the bitmap with world matrix
                    //   world_pt = M·L + (tx, ty),  M = R(rot)·diag(sx,sy).
                    // Faithfully reproducing it (x=tx, y=ty, rotation/scale from the
                    // decomposition) is exact for every frame, so a spin (orbiting translation +
                    // rotation) stays anchored automatically, with no special-casing.
                    //
                    // For a DefineShape with a bitmap fill the bitmap is offset inside the shape;
                    // shape_pivot is that offset, so the bitmap origin lands at M·shape_pivot.
                    // The shape's bitmap-FILL scale is folded into the placement: SSF2 references
                    // a high-res bitmap through a DefineShape whose fill matrix scales it (often
                    // far down), so without it the sprite renders 1/fill too big (zelda's shared
                    // magic particle at fill 0.078 came out ~13x oversized). Absent fill (vector
                    // shapes, 1:1 bitmaps) it is (1,1). Negative scales are flips, which FrayTools
                    // supports.
                    let place = |f: u32| -> Option<(String, crate::flatten::Pose)> {
                        let entry = held[f as usize].as_ref()?;
                        let shape_id = entry.shape_id;
                        let img = img_result.images.get(&img_result.shape_to_bitmap.get(&shape_id).copied().unwrap_or(shape_id))
                            .or_else(|| img_result.images.values().find(|img| img.symbol_name == entry.symbol_name))?;
                        let meta_guid = image_guids.get(&img.symbol_name).cloned().unwrap_or_default();
                        let (fsx, fsy) = img_result.shape_fill_scale.get(&shape_id).copied().unwrap_or((1.0, 1.0));
                        // Where a placed shape lands is ONE conversion, shared with the stage art
                        // path (see `fm_placement`).
                        let placement = crate::fm_placement::image_placement(
                            &crate::fm_placement::WorldPlacement {
                                tx: round2(entry.world_tx), ty: round2(entry.world_ty),
                                sx: round2(entry.world_sx), sy: round2(entry.world_sy),
                                rotation: round2(entry.world_rotation),
                                a: entry.world_a, b: entry.world_b, c: entry.world_c, d: entry.world_d,
                            },
                            img_result.shape_pivot.get(&shape_id).copied().unwrap_or((0.0, 0.0)),
                            (fsx, fsy),
                        );
                        // A turn animation is drawn facing the direction being turned FROM, and
                        // fraymakers flips the character's facing when the turn STARTS, so the
                        // art mirrors for the whole animation.
                        let placement = if anim_name == "stand_turn" || anim_name == "run_turn" {
                            crate::fm_placement::mirrored(placement)
                        } else { placement };
                        let (fm_sx, fm_sy) = (placement.scale_x, placement.scale_y);
                        // the pivot is the piece's own registration point, in its picture's pixels:
                        // where the source turns and scales it from, so a tween turns it about the
                        // same point (the engine draws a still frame the same whatever the pivot)
                        let reg = img_result.shape_pivot.get(&shape_id).copied().unwrap_or((0.0, 0.0));
                        let (mut pivot_x, mut pivot_y) = (round2(-reg.0), round2(-reg.1));
                        let (mut fm_x, mut fm_y, mut rot) = (placement.x, placement.y, placement.rotation);
                        // FM's engine applies its OWN rotation during the TUMBLE state, about the
                        // entity position, so for tumble ONLY the authored rotation is stripped
                        // and the pose centred on the origin, so the engine's spin rotates it in
                        // place. FrayTools places an IMAGE so its centre lands at
                        // (x + scaleX*w/2, y + scaleY*h/2) with SIGNED scales.
                        if anim_name == "tumble" {
                            fm_x = round2(-fm_sx * img.width as f64 / 2.0);
                            fm_y = round2(-fm_sy * img.height as f64 / 2.0);
                            rot = 0.0;
                            (pivot_x, pivot_y) = (0.0, 0.0);
                        }
                        // hurt_heavy is also a launched pose (see box_shift above): the art moves
                        // by the same hurtbox-union-to-origin delta as the boxes. No rotation strip.
                        if anim_name == "hurt_heavy" {
                            fm_x = round2(fm_x + box_shift.0);
                            fm_y = round2(fm_y + box_shift.1);
                        }
                        // the opacity the SOURCE gives this placement: Flash animates a fade, a
                        // flash or a ghosting trail as alpha
                        Some((meta_guid, [fm_x, fm_y, fm_sx, fm_sy, round2(rot), round2(entry.alpha), pivot_x, pivot_y]))
                    };
                    let mut placed: Vec<Option<(String, crate::flatten::Pose)>> = (0..total).map(place).collect();
                    // each run of one picture on the layer takes the pivot its tweens fit best: the
                    // registration point (a turn about it) or the picture's corner (a scale about
                    // it, as FrayTools keeps the corner's offset from the pivot at the start scale
                    // while nothing turns)
                    let mut i = 0usize;
                    while i < placed.len() {
                        let Some((guid, _)) = placed[i].clone() else { i += 1; continue };
                        let mut j = i;
                        while j < placed.len() && placed[j].as_ref().is_some_and(|(g, _)| *g == guid) { j += 1; }
                        let poses: Vec<crate::flatten::Pose> = placed[i..j].iter().map(|p| p.as_ref().unwrap().1).collect();
                        let corner = crate::flatten::at_corner(&poses);
                        if crate::flatten::keyframes_at(&corner) < crate::flatten::keyframes_at(&poses) {
                            for (k, q) in corner.into_iter().enumerate() { placed[i + k].as_mut().unwrap().1 = q; }
                        }
                        i = j;
                    }

                    // Keyframes: a run of frames placed alike is one keyframe; a run where the
                    // same picture moves in a straight line (place, scale, turn and opacity, each
                    // the even step between the run's ends) is one tweened keyframe, the run's
                    // last frame starting the next. a turn or a scale goes about the pivot (the
                    // piece's registration point), as the source's does.
                    let mut f: u32 = 0;
                    while f < total {
                        let cur = &placed[f as usize];
                        let mut run = 1u32;
                        while f + run < total && placed[(f + run) as usize] == *cur { run += 1; }
                        let mut tweened = false;
                        if let (1, Some((guid, from))) = (run, cur) {
                            let mut best = None;
                            for g in (f + 2)..total {
                                let Some((gg, to)) = &placed[g as usize] else { break };
                                if gg != guid { break }
                                let span = (g - f) as f64;
                                let straight = ((f + 1)..g).all(|m| match &placed[m as usize] {
                                    Some((mg, mid)) if mg == guid => crate::flatten::on_line(from, to, mid, (m - f) as f64 / span),
                                    _ => false,
                                });
                                if straight { best = Some(g) } else { break }
                            }
                            if let Some(g) = best { run = g - f; tweened = true; }
                        }

                        let kf_id = uuid(char_id, &format!("kf_image_{}_s{}_f{}", anim_name, slot, f));
                        let symbol_ref = cur.as_ref().map(|(guid, v)| {
                            // a per-placement symbol (unique per anim/slot/frame)
                            let id = uuid(char_id, &format!("sym_img_{}_s{}_f{}", anim_name, slot, f));
                            symbols.push(json!({
                                "$id": id,
                                "alpha": v[5],
                                "imageAsset": guid,
                                "pivotX": v[6],
                                "pivotY": v[7],
                                "pluginMetadata": {},
                                // SWF world_rot = atan2(b,a) in y-down coords, normalised to
                                // FrayTools' 0-360 range (no negation)
                                "rotation": v[4],
                                "scaleX": v[2],
                                "scaleY": v[3],
                                "type": "IMAGE",
                                "x": v[0],
                                "y": v[1]
                            }));
                            id
                        });

                        keyframes.push(json!({
                            "$id": kf_id,
                            "type": "IMAGE",
                            "length": run,
                            "symbol": symbol_ref.map(Value::String).unwrap_or(Value::Null),
                            "tweened": tweened,
                            "tweenType": "LINEAR",
                            "pluginMetadata": {}
                        }));
                        img_kf_ids.push(kf_id);
                        f += run;
                    }
                } else if slot == 0 {
                    // No image data — single null keyframe for slot 0 only
                    let kf_id = uuid(char_id, &format!("kf_image_{}_s0_f0", anim_name));
                    keyframes.push(json!({
                        "$id": kf_id,
                        "type": "IMAGE",
                        "length": frame_count,
                        "symbol": Value::Null,
                        "tweened": false,
                        "tweenType": "LINEAR",
                        "pluginMetadata": {}
                    }));
                    img_kf_ids.push(kf_id);
                }

                layers.push(json!({
                    "$id": img_layer_id,
                    "name": format!("Image {}", slot),
                    "type": "IMAGE",
                    "keyframes": img_kf_ids,
                    "hidden": false,
                    "locked": false,
                    "pluginMetadata": {}
                }));
                anim_image_layer_ids.push(img_layer_id);
            }

            // Then BLANK the slots this animation does not use.
            //
            // A layer the engine is not told about keeps whatever the previous animation put
            // there. Sandbag's revival is five layers deep (the neutral pose plus its platform
            // and glow); walk is one. Walk after revival therefore inherits four layers it never
            // asked for, and the platform rides along under the character for as long as the next
            // animation stays shallow. It reads as a stray frame of the wrong move, which is a
            // hard thing to trace back to a layer count.
            //
            // SSF2 does not have the problem because a Flash timeline clears its own layers. Here
            // each animation has to say so, which costs one empty keyframe per unused slot: the
            // layer exists, is addressed, and is explicitly holding nothing.
            for slot in num_slots..entity_image_slots {
                let img_layer_id = uuid(char_id, &format!("layer_image_{}_{}", anim_name, slot));
                let kf_id = uuid(char_id, &format!("kf_image_blank_{}_{}", anim_name, slot));
                keyframes.push(json!({
                    "$id": kf_id, "type": "IMAGE", "length": frame_count.max(1),
                    "symbol": Value::Null, "tweened": false, "tweenType": "LINEAR",
                    "pluginMetadata": {}
                }));
                layers.push(json!({
                    "$id": img_layer_id,
                    "name": format!("Image {}", slot),
                    "type": "IMAGE",
                    "keyframes": [kf_id],
                    "hidden": false,
                    "locked": false,
                    "pluginMetadata": {}
                }));
                anim_image_layer_ids.push(img_layer_id);
            }
        }

        // Emit IMAGE layers first (drawn behind), then the collision/label/script
        // layers, preserving relative order within each group.
        let mut ordered_layer_ids = anim_image_layer_ids;
        ordered_layer_ids.extend(anim_layer_ids);

        animations.push(json!({
            "$id": anim_id,
            "name": anim_name,
            "layers": ordered_layer_ids,
            "pluginMetadata": {}
        }));
    }

    // the named children of its clips that its code addresses, each a part riding it
    push_part_animations(&mut keyframes, &mut layers, &mut symbols, char_id, "part", &data.parts, &mut animations);
    // the platforms it gives itself, each a line a structure content rides on
    let plats: Vec<(f64, f64)> = data.platforms.iter().map(|(_, _, w, h)| (*w, *h)).collect();
    push_platform_animations(&mut keyframes, &mut layers, &mut symbols, &mut animations, char_id, "self", &plats);

    // 30fps → 60fps: hold every keyframe for two frames (see fn docs).
    double_keyframe_lengths(&mut keyframes);

    // Drop animations whose image timeline is entirely blank (no IMAGE
    // keyframe carries a symbol). Empty stubs clutter the FrayTools editor
    // and confuse modders. KEEP_EMPTY allowlists empty animations that the
    // jab-chain boilerplate in Script.hx still references by name — those
    // would crash at runtime if dropped.
    //
    // For populated_jabs == 1 (single-jab characters) Script.hx emits no
    // chain at all, so jab2/jab3 are not referenced and can be dropped like
    // the other empties. For populated_jabs >= 2 the chain is emitted; the
    // chain reaches up to jab3, so jab3 must exist if it's the last link
    // and would otherwise be empty (populated_jabs == 2). populated_jabs
    // >= 3 means jab2/jab3 are themselves populated and don't need an
    // allowlist entry.
    let keep_empty: &[&str] = match populated_jabs {
        2 => &["jab3"],
        _ => &[],
    };
    {
        let layer_type: std::collections::BTreeMap<String, String> = layers.iter()
            .filter_map(|l| Some((l["$id"].as_str()?.to_string(), l["type"].as_str()?.to_string())))
            .collect();
        let layer_kfs: std::collections::BTreeMap<String, Vec<String>> = layers.iter()
            .filter_map(|l| {
                let id = l["$id"].as_str()?.to_string();
                let kfs = l["keyframes"].as_array()?.iter()
                    .filter_map(|v| Some(v.as_str()?.to_string())).collect();
                Some((id, kfs))
            }).collect();
        let kf_has_symbol: std::collections::BTreeMap<String, bool> = keyframes.iter()
            .filter_map(|k| Some((
                k["$id"].as_str()?.to_string(),
                k.get("symbol").map(|v| !v.is_null()).unwrap_or(false),
            ))).collect();

        let mut drop_layers: std::collections::BTreeSet<String> = Default::default();
        let mut dropped_names: Vec<String> = Vec::new();
        animations.retain(|a| {
            let name = match a["name"].as_str() { Some(n) => n, None => return true };
            if keep_empty.contains(&name) { return true; }
            // a platform line carries no picture and is whole as it is
            if name.starts_with("platformSprite") { return true; }
            let lids = match a["layers"].as_array() { Some(v) => v, None => return true };
            let has_image = lids.iter().filter_map(|v| v.as_str()).any(|lid| {
                if layer_type.get(lid).map(String::as_str) != Some("IMAGE") { return false; }
                layer_kfs.get(lid).map(|ks|
                    ks.iter().any(|k| *kf_has_symbol.get(k).unwrap_or(&false))
                ).unwrap_or(false)
            });
            if !has_image {
                dropped_names.push(name.to_string());
                for v in lids { if let Some(s) = v.as_str() { drop_layers.insert(s.to_string()); } }
            }
            has_image
        });
        let mut drop_kfs: std::collections::BTreeSet<String> = Default::default();
        for lid in &drop_layers {
            if let Some(ks) = layer_kfs.get(lid) {
                for k in ks { drop_kfs.insert(k.clone()); }
            }
        }
        layers.retain(|l| l["$id"].as_str().map(|id| !drop_layers.contains(id)).unwrap_or(true));
        keyframes.retain(|k| k["$id"].as_str().map(|id| !drop_kfs.contains(id)).unwrap_or(true));
        if !dropped_names.is_empty() {
            log::info!("Dropped {} empty animation(s): {}", dropped_names.len(), dropped_names.join(", "));
            note_dropped_animations(dropped_names.iter().map(|n| n.to_string()).collect());
        }
    }

    // Order template animations per the character template; relegate everything that is
    // neither a template animation nor a direct variant of one to an UNUSED section.
    reorder_animations(&mut animations, &mut layers, &mut keyframes, char_id);
    let entity_image_slots = img_result.anim_images.values()
        .map(|a| a.max_depth_slots.max(1)).max().unwrap_or(1);
    append_missing_template_animations(&mut animations, &mut layers, &mut keyframes, char_id,
        entity_image_slots);

    let entity = json!({
        "animations": animations,
        "export": true,
        "guid": uuid(char_id, "entity_guid"),
        "id": char_id,
        "keyframes": keyframes,
        "layers": layers,
        "paletteMap": Value::Null,
        "pluginMetadata": {
            "com.fraymakers.FraymakersMetadata": {
                "objectType": "CHARACTER",
                "version": "0.4.0"
            }
        },
        "plugins": ["com.fraymakers.FraymakersMetadata"],
        "symbols": symbols,
        "tags": [],
        "terrains": [],
        "tilesets": [],
        "version": 14
    });

    finish_entity(entity)
}

/// The throws whose attack says it turns its fighter around (`flip`). The source turns the
/// thrower as the throw begins and states the throw for the way it then faces (its clip drawn
/// mirrored, its launch forward); this engine turns no one in a throw, so these throws are drawn
/// and launched already turned, and the runtime turns the thrower once the throw is over. The
/// throws are the engine's throw animations (`mappings/character/stats_tables.jsonc`).
pub fn turned_throws(data: &CharacterData) -> std::collections::BTreeSet<String> {
    let tables = crate::mappings::character_stats_tables();
    let throws: std::collections::BTreeSet<&str> = tables.hitbox_sections.iter()
        .filter(|s| s.section == "THROWS").flat_map(|s| s.moves.iter().map(String::as_str)).collect();
    data.attacks.iter()
        .filter(|a| throws.contains(a.name.as_str()) && a.stats.get("flip").is_some_and(|v| *v != 0.0))
        .map(|a| a.name.clone()).collect()
}

/// Every frame of the named animations mirrored about the fighter's origin: pictures, boxes,
/// the body and points. A symbol another animation also shows is copied first.
pub fn draw_turned(json_str: &str, anims: &std::collections::BTreeSet<String>) -> String {
    if anims.is_empty() { return json_str.to_string(); }
    let Ok(mut entity) = serde_json::from_str::<serde_json::Value>(json_str) else { return json_str.to_string() };
    let layer_ids = |e: &serde_json::Value, pick: &dyn Fn(&str) -> bool| -> std::collections::BTreeSet<String> {
        e["animations"].as_array().into_iter().flatten()
            .filter(|a| a["name"].as_str().is_some_and(pick))
            .flat_map(|a| a["layers"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(str::to_string)))
            .collect()
    };
    let mine = layer_ids(&entity, &|n| anims.contains(n));
    let others = layer_ids(&entity, &|n| !anims.contains(n));
    let kf_ids = |e: &serde_json::Value, ls: &std::collections::BTreeSet<String>| -> std::collections::BTreeSet<String> {
        e["layers"].as_array().into_iter().flatten()
            .filter(|l| l["$id"].as_str().is_some_and(|i| ls.contains(i)))
            .flat_map(|l| l["keyframes"].as_array().into_iter().flatten().filter_map(|k| k.as_str().map(str::to_string)))
            .collect()
    };
    let my_kfs = kf_ids(&entity, &mine);
    let other_kfs = kf_ids(&entity, &others);
    let syms_of = |e: &serde_json::Value, ks: &std::collections::BTreeSet<String>| -> std::collections::BTreeSet<String> {
        e["keyframes"].as_array().into_iter().flatten()
            .filter(|k| k["$id"].as_str().is_some_and(|i| ks.contains(i)))
            .filter_map(|k| k["symbol"].as_str().map(str::to_string)).collect()
    };
    let shared = syms_of(&entity, &other_kfs);
    let my_syms = syms_of(&entity, &my_kfs);
    let mut copies: Vec<serde_json::Value> = Vec::new();
    let mut renamed: BTreeMap<String, String> = BTreeMap::new();
    let mirror = |s: &mut serde_json::Value| {
        let num = |s: &serde_json::Value, k: &str| s[k].as_f64().unwrap_or(0.0);
        match s["type"].as_str() {
            Some("IMAGE") => {
                s["x"] = json!(round2(-num(s, "x")));
                s["scaleX"] = json!(-num(s, "scaleX"));
                s["rotation"] = json!(crate::fm_placement::normalise_degrees(-num(s, "rotation")));
            }
            Some("COLLISION_BOX") => s["x"] = json!(round2(-(num(s, "x") + num(s, "scaleX")))),
            Some("COLLISION_BODY") => s["hipXOffset"] = json!(-num(s, "hipXOffset")),
            Some("POINT") => s["x"] = json!(round2(-num(s, "x"))),
            _ => {}
        }
    };
    if let Some(syms) = entity["symbols"].as_array_mut() {
        for sym in syms.iter_mut() {
            let Some(id) = sym["$id"].as_str().map(str::to_string) else { continue };
            if !my_syms.contains(&id) { continue; }
            if shared.contains(&id) {
                let mut c = sym.clone();
                let nid = format!("{id}-turned");
                c["$id"] = json!(nid.clone());
                mirror(&mut c);
                renamed.insert(id, nid);
                copies.push(c);
            } else {
                mirror(sym);
            }
        }
        syms.extend(copies);
    }
    if let Some(kfs) = entity["keyframes"].as_array_mut() {
        for k in kfs.iter_mut() {
            if !k["$id"].as_str().is_some_and(|i| my_kfs.contains(i)) { continue; }
            if let Some(n) = k["symbol"].as_str().and_then(|s| renamed.get(s)).cloned() { k["symbol"] = json!(n); }
        }
    }
    serde_json::to_string_pretty(&entity).unwrap_or_else(|_| json_str.to_string())
}

/// An entity already built, pointed at its costume palette.
pub fn attach_palette(json_str: &str, palette_collection_guid: &str, palette_map_id: &str) -> String {
    let json_str = json_str.to_string();
    let mut entity: serde_json::Value = serde_json::from_str(&json_str).unwrap_or(serde_json::json!({}));
    entity["paletteMap"] = serde_json::json!({
        "paletteCollection": palette_collection_guid,
        "paletteMap": palette_map_id
    });
    serde_json::to_string_pretty(&entity).unwrap_or(json_str)
}

/// Get image GUIDs for .meta file generation
pub fn get_image_meta_guids(
    img_result: &ImageExtractionResult,
) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for img in img_result.images.values() {
        let meta_guid = crate::uuid_gen::image_meta_guid(&img.symbol_name);
        result.insert(img.png_path.clone(), meta_guid);
    }
    result
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// ─── Menu entity generation ───────────────────────────────────────────────────

/// Information about a menu image extracted from the SWF.
#[derive(Debug, Clone)]
pub struct MenuImageInfo {
    /// The symbol name of the head/portrait image (e.g. "mario_dm0")
    pub head_symbol: String,
    /// Width of the head image in pixels
    pub head_width: u32,
    /// Height of the head image in pixels
    pub head_height: u32,
    /// Meta GUID for the head image
    pub head_meta_guid: String,
}

/// The frames the engine shows a character's picture in: the damage counter's head window, the
/// stock icon, and the small roster icon. Same numbers the entity's own visual-aid boxes carry,
/// which is what a character is authored against in FrayTools.
pub const HUD_BOX: f64 = 36.0;
const STOCK_BOX: f64 = 24.0;
const ICON_BOX: f64 = 21.0;

/// Scale that FILLS `bw`x`bh` with `w`x`h`, keeping the picture's shape. The frame is squarer than
/// a portrait's window, so filling it means the sides run past the frame and the engine's own mask
/// crops them, which is how a base-game character's picture sits in the counter: head to the
/// edges, not floating in the middle with margins.
///
/// Never enlarges. A picture smaller than its frame is left at its own size rather than blown up,
/// since the engine point-samples and blowing it up only makes bigger pixels.
fn fit_scale(w: f64, h: f64, bw: f64, bh: f64) -> f64 {
    if w <= 0.0 || h <= 0.0 { return 1.0 }
    (bw / w).max(bh / h).min(1.0)
}

/// Generate a menu.entity file for the character's UI images.
///
/// SSF2 provides one portrait image (from the `_head` sprite, e.g. `mario_dm0`).
/// We reuse it across all required Fraymakers menu animations:
///   - `full` — full character portrait (character select screen)
///   - `css` — character select screen thumbnail (2 layers: foreground + background)
///   - `icon` — small character icon
///   - `icon_no_palette` — icon without palette swap
///   - `stock` — stock icon (lives remaining)
///   - `hud` / `hud_front` — HUD portrait (default expression)
///   - `hud_angry` / `hud_angry_front` — HUD portrait (angry)
///   - `hud_happy` / `hud_happy_front` — HUD portrait (happy)
///   - `hud_hurt` / `hud_hurt_front` — HUD portrait (hurt)
///   - `hud_sad` / `hud_sad_front` — HUD portrait (sad)
///
/// Since SSF2 only has one portrait, all HUD variants use the same image.
pub fn generate_menu_entity(
    char_id: &str,
    menu_info: &MenuImageInfo,
) -> String {
    let mut keyframes: Vec<Value> = Vec::new();
    let mut layers: Vec<Value> = Vec::new();
    let mut symbols: Vec<Value> = Vec::new();
    let mut animations: Vec<Value> = Vec::new();

    let head_guid = &menu_info.head_meta_guid;
    let img_w = menu_info.head_width as f64;
    let img_h = menu_info.head_height as f64;
    let pivot_x = round2(img_w / 2.0);
    let pivot_y = round2(img_h / 2.0);
    // ONE portrait, used by every slot. A second image in the menu sheet is one the packer can
    // silently drop, and an entity pointing at a frame that is not there is a character that
    // does not draw.
    let hud_guid = menu_info.head_meta_guid.clone();
    let (hud_w, hud_h) = (img_w, img_h);
    let hud_pivot_x = round2(hud_w / 2.0);
    let hud_pivot_y = round2(hud_h / 2.0);

    // Helper functions return (sym, kf, layer) tuples; caller pushes them.
    fn make_image_items(char_id: &str, head_guid: &str, pivot_x: f64, pivot_y: f64,
                        anim_name: &str, suffix: &str, x: f64, y: f64, scale: f64)
        -> (Value, Value, Value, String) {
        let sym_id = uuid(char_id, &format!("menu_sym_{}{}", anim_name, suffix));
        let kf_id = uuid(char_id, &format!("menu_kf_{}{}", anim_name, suffix));
        let layer_id = uuid(char_id, &format!("menu_layer_{}{}", anim_name, suffix));
        let sym = json!({ "$id": sym_id, "alpha": 1, "imageAsset": head_guid,
            "pivotX": pivot_x, "pivotY": pivot_y, "pluginMetadata": {},
            "rotation": 0, "scaleX": round2(scale), "scaleY": round2(scale), "type": "IMAGE",
            "x": round2(x), "y": round2(y) });
        let kf = json!({ "$id": kf_id, "length": 1, "pluginMetadata": {},
            "symbol": sym_id, "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" });
        let layer = json!({ "$id": layer_id, "name": "Image Layer", "type": "IMAGE",
            "keyframes": [kf_id], "hidden": false, "locked": false, "pluginMetadata": {} });
        (sym, kf, layer, layer_id)
    }

    fn make_blank_image(char_id: &str, anim_name: &str, suffix: &str) -> (Value, Value, String) {
        let kf_id = uuid(char_id, &format!("menu_kf_{}{}_blank", anim_name, suffix));
        let layer_id = uuid(char_id, &format!("menu_layer_{}{}_blank", anim_name, suffix));
        let kf = json!({ "$id": kf_id, "length": 1, "pluginMetadata": {},
            "symbol": Value::Null, "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" });
        let layer = json!({ "$id": layer_id, "name": "Image Layer", "type": "IMAGE",
            "keyframes": [kf_id], "hidden": false, "locked": false, "pluginMetadata": {} });
        (kf, layer, layer_id)
    }

    fn make_hud_aid(char_id: &str, anim_name: &str, size: f64) -> (Value, Value, Value, String) {
        let sym_id = uuid(char_id, &format!("menu_hud_aid_sym_{}", anim_name));
        let kf_id = uuid(char_id, &format!("menu_hud_aid_kf_{}", anim_name));
        let layer_id = uuid(char_id, &format!("menu_hud_aid_layer_{}", anim_name));
        let sym = json!({ "$id": sym_id, "alpha": Value::Null, "color": Value::Null,
            "pivotX": round2(size/2.0), "pivotY": round2(size/2.0), "pluginMetadata": {},
            "rotation": 0, "scaleX": size, "scaleY": size, "type": "COLLISION_BOX",
            "x": 0, "y": 0 });
        let kf = json!({ "$id": kf_id, "length": 1, "pluginMetadata": {},
            "symbol": sym_id, "tweenType": "LINEAR", "tweened": false, "type": "COLLISION_BOX" });
        let layer = json!({ "$id": layer_id, "name": "HUD Visual Aid", "type": "COLLISION_BOX",
            "keyframes": [kf_id], "hidden": false, "locked": false, "pluginMetadata": {} });
        (sym, kf, layer, layer_id)
    }

    fn make_blank_script(char_id: &str, anim_name: &str) -> (Value, Value, String) {
        let kf_id = uuid(char_id, &format!("menu_script_kf_{}", anim_name));
        let layer_id = uuid(char_id, &format!("menu_script_layer_{}", anim_name));
        let kf = json!({ "$id": kf_id, "length": 1, "pluginMetadata": {},
            "symbol": Value::Null, "tweenType": "LINEAR", "tweened": false, "type": "FRAME_SCRIPT" });
        let layer = json!({ "$id": layer_id, "name": "Frame Script Layer", "type": "FRAME_SCRIPT",
            "keyframes": [kf_id], "hidden": false, "locked": false, "pluginMetadata": {} });
        (kf, layer, layer_id)
    }

    fn make_icon_aid(char_id: &str, anim_name: &str) -> (Value, Value, Value, String) {
        let sym_id = uuid(char_id, &format!("menu_icon_aid_sym_{}", anim_name));
        let kf_id = uuid(char_id, &format!("menu_icon_aid_kf_{}", anim_name));
        let layer_id = uuid(char_id, &format!("menu_icon_aid_layer_{}", anim_name));
        let sym = json!({ "$id": sym_id, "alpha": Value::Null, "color": Value::Null,
            "pivotX": 18, "pivotY": 18, "pluginMetadata": {},
            "rotation": 0, "scaleX": 41, "scaleY": 21, "type": "COLLISION_BOX",
            "x": 0, "y": 0 });
        let kf = json!({ "$id": kf_id, "length": 1, "pluginMetadata": {},
            "symbol": sym_id, "tweenType": "LINEAR", "tweened": false, "type": "COLLISION_BOX" });
        let layer = json!({ "$id": layer_id, "name": "Size Visual Aid", "type": "COLLISION_BOX",
            "keyframes": [kf_id], "hidden": false, "locked": false, "pluginMetadata": {} });
        (sym, kf, layer, layer_id)
    }

    // Macro to push items returned by helpers
    macro_rules! push_img {
        ($anim:expr, $suf:expr, $x:expr, $y:expr) => {{
            let (s, k, l, lid) = make_image_items(char_id, head_guid, pivot_x, pivot_y, $anim, $suf, $x, $y, 1.0);
            symbols.push(s); keyframes.push(k); layers.push(l); lid
        }}
    }
    macro_rules! push_hud_img {
        ($anim:expr, $suf:expr, $box:expr) => {{
            // Fill the frame the engine draws this slot in: the portrait is the head already,
            // so filling means the head reaches the edges the way a base-game character's does,
            // with the engine's own mask cropping the overhang.
            let sc = fit_scale(hud_w, hud_h, $box, $box);
            let (s, k, l, lid) = make_image_items(char_id, &hud_guid, hud_pivot_x, hud_pivot_y,
                $anim, $suf, -hud_pivot_x * sc, -hud_pivot_y * sc, sc);
            symbols.push(s); keyframes.push(k); layers.push(l); lid
        }}
    }
    macro_rules! push_blank_img {
        ($anim:expr, $suf:expr) => {{
            let (k, l, lid) = make_blank_image(char_id, $anim, $suf);
            keyframes.push(k); layers.push(l); lid
        }}
    }
    macro_rules! push_hud_aid {
        ($anim:expr, $size:expr) => {{
            let (s, k, l, lid) = make_hud_aid(char_id, $anim, $size);
            symbols.push(s); keyframes.push(k); layers.push(l); lid
        }}
    }
    macro_rules! push_script {
        ($anim:expr) => {{
            let (k, l, lid) = make_blank_script(char_id, $anim);
            keyframes.push(k); layers.push(l); lid
        }}
    }
    macro_rules! push_icon_aid {
        ($anim:expr) => {{
            let (s, k, l, lid) = make_icon_aid(char_id, $anim);
            symbols.push(s); keyframes.push(k); layers.push(l); lid
        }}
    }

    // ── full: big portrait ───────────────────────────────────────────────
    {
        let img_layer = push_img!("full", "", -pivot_x, -pivot_y);
        let blank_layer = push_blank_img!("full", "");
        animations.push(json!({ "$id": uuid(char_id, "menu_anim_full"),
            "name": "full", "layers": [img_layer, blank_layer], "pluginMetadata": {} }));
    }

    // ── css: character select screen (2 image layers) ────────────────────
    {
        let fg = push_img!("css", "_fg", -pivot_x, -pivot_y);
        let bg = push_img!("css", "_bg", -pivot_x, -pivot_y);
        animations.push(json!({ "$id": uuid(char_id, "menu_anim_css"),
            "name": "css", "layers": [fg, bg], "pluginMetadata": {} }));
    }

    // ── icon ─────────────────────────────────────────────────────────────
    {
        let img = push_hud_img!("icon", "", ICON_BOX);
        let aid = push_icon_aid!("icon");
        let script = push_script!("icon");
        animations.push(json!({ "$id": uuid(char_id, "menu_anim_icon"),
            "name": "icon", "layers": [img, aid, script], "pluginMetadata": {} }));
    }

    // ── icon_no_palette ──────────────────────────────────────────────────
    {
        let img = push_hud_img!("icon_no_palette", "", ICON_BOX);
        let script = push_script!("icon_no_palette");
        let aid = push_icon_aid!("icon_no_palette");
        animations.push(json!({ "$id": uuid(char_id, "menu_anim_icon_no_palette"),
            "name": "icon_no_palette", "layers": [img, script, aid], "pluginMetadata": {} }));
    }

    // ── stock ────────────────────────────────────────────────────────────
    {
        let aid = push_hud_aid!("stock", 24.0);
        let img = push_hud_img!("stock", "", STOCK_BOX);
        animations.push(json!({ "$id": uuid(char_id, "menu_anim_stock"),
            "name": "stock", "layers": [aid, img], "pluginMetadata": {} }));
    }

    // ── hud variants ────────────────────────────────────────────────────
    // SSF2 has one portrait, so every expression is the same head. The `_front` layer is drawn in
    // front of the counter's frame and draws the head too: a front animation that draws NOTHING
    // stops the character being drawn at all. Each front sits with its own hud animation.
    for hud_name in &["hud", "hud_angry", "hud_happy", "hud_hurt", "hud_sad"] {
        let img = push_hud_img!(hud_name, "", HUD_BOX);
        let aid = push_hud_aid!(hud_name, 36.0);
        animations.push(json!({ "$id": uuid(char_id, &format!("menu_anim_{}", hud_name)),
            "name": *hud_name, "layers": [img, aid], "pluginMetadata": {} }));

        let front = format!("{hud_name}_front");
        let fimg = push_hud_img!(&front, "", HUD_BOX);
        let faid = push_hud_aid!(&front, 36.0);
        animations.push(json!({ "$id": uuid(char_id, &format!("menu_anim_{}", front)),
            "name": front, "layers": [fimg, faid], "pluginMetadata": {} }));
    }

    // 30fps → 60fps: hold every keyframe for two frames (see fn docs).
    double_keyframe_lengths(&mut keyframes);

    let entity = json!({
        "animations": animations,
        "export": true,
        "guid": uuid(char_id, "menu_entity_guid"),
        "id": "menu",
        "keyframes": keyframes,
        "layers": layers,
        "paletteMap": Value::Null,
        "pluginMetadata": {
            "com.fraymakers.FraymakersMetadata": {
                "spritesheetGroup": "menu",
                "version": "0.3.1"
            }
        },
        "plugins": ["com.fraymakers.FraymakersMetadata"],
        "symbols": symbols,
        "tags": ["menu"],
        "terrains": [],
        "tilesets": [],
        "version": 14
    });

    finish_entity(entity)
}

// ─── Projectile entity generation ─────────────────────────────────────────────

/// Information about a projectile extracted from the SWF.
#[derive(Debug, Clone)]
pub struct ProjectileInfo {
    /// The projectile's symbol name (e.g. "mario_fireball")
    pub name: String,
    /// its own clip, and the clip its frames are drawn from when it places one as `stance`
    pub sprite_id: u16,
    pub inner_sprite_id: Option<u16>,
    /// The inner animation sprite name (e.g. "mario_fla.mario_fireball_mc_210")
    pub inner_sprite_name: Option<String>,
    /// Frame count of the inner animation sprite
    pub inner_frame_count: u16,
    /// Collision boxes from the inner sprite, if any
    pub boxes: Option<AnimationBoxData>,
    /// Image frames from the inner sprite (symbol name per frame)
    pub image_frames: Vec<Option<String>>,
    /// Per-frame SSF2 local transform (parallel to image_frames). Applied to the
    /// IMAGE symbol so the sprite renders at its SSF2 display size, not full native.
    pub image_matrices: Vec<Option<crate::image_extractor::ImageLocalMatrix>>,
    /// Every part the clip draws on each frame, in depth order, with its transform. A clip is
    /// several drawings at once (sora's keyblade is the key AND a glow arc swinging round it,
    /// and the one-image path kept the arc and dropped the key), so each part gets a layer.
    pub image_parts: Vec<Vec<(String, crate::image_extractor::ImageLocalMatrix)>>,
    /// Meta GUIDs for each unique image used
    pub image_guids: BTreeMap<String, String>,
    /// Extra per-state data for multi-state projectiles (e.g. link_bomb).
    /// Each entry is (ssf2_label, image_frames, image_guids, boxes, frame_count).
    pub extra_states: Vec<ProjectileStateData>,
    /// The named children of its clip that its code addresses, each a part riding it.
    pub parts: Vec<ProjectilePartData>,
    /// The platforms the clip gives itself (`createSelfPlatform`), one `(width, height)` per
    /// call site in source pixels: each becomes a `platformSprite{i}` animation a structure
    /// content stands on, riding the projectile.
    pub platforms: Vec<(f64, f64)>,
    /// The class's OWN timeline scripts, by 1-based source frame: `frame10` and friends, already
    /// translated. A projectile's clip carries code on its frames exactly as a character's does,
    /// and it is that code -- not anything this converter thinks a projectile should do -- that
    /// says what this one does.
    pub frame_scripts: BTreeMap<u16, String>,
    /// SSF2 frame labels found INSIDE the inner animation sprite, in
    /// timeline order. `(1-based frame, label)`. Empty for projectiles
    /// without timeline labels — those fall back to the FM template
    /// trio (`projectileSpawn` / `projectileIdle` / `projectileDestroy`).
    pub inner_labels: Vec<(u16, String)>,
}

impl ProjectileInfo {
    /// Every frame script the entity carries, the flight's and each state's: one pass over
    /// the code runs over all of them.
    pub fn frame_codes(&self) -> Vec<&String> {
        self.frame_scripts.values()
            .chain(self.extra_states.iter().flat_map(|s| s.frame_scripts.values()))
            .collect()
    }
    pub fn frame_codes_mut(&mut self) -> Vec<&mut String> {
        self.frame_scripts.values_mut()
            .chain(self.extra_states.iter_mut().flat_map(|s| s.frame_scripts.values_mut()))
            .collect()
    }
}

/// A named child of a projectile's clip that its code addresses, drawn as a part of its own
/// riding the projectile: its frames become the animation `anim` on the projectile's entity,
/// and each of its labels a `{anim}_{label}` cut from them.
#[derive(Debug, Clone)]
pub struct ProjectilePartData {
    /// how the code names it: `gordospin`, or `notes.n1` for a child of a child
    pub path: String,
    pub anim: String,
    /// where the timeline puts it, in the projectile's space (a child of a child composed)
    pub matrix: crate::image_extractor::ImageLocalMatrix,
    pub image_frames: Vec<Option<String>>,
    pub image_matrices: Vec<Option<crate::image_extractor::ImageLocalMatrix>>,
    pub image_parts: Vec<Vec<(String, crate::image_extractor::ImageLocalMatrix)>>,
    pub image_guids: BTreeMap<String, String>,
    pub frame_count: u16,
    /// (label, 1-based frame)
    pub labels: Vec<(String, u16)>,
}

/// Where one of a fighter's animations places a part: the part, the animation, the art the
/// part shows there (its `part_*` animation) and the placement in the fighter's space.
#[derive(Debug, Clone)]
pub struct PartPlacement {
    pub name: String,
    pub fm_anim: String,
    pub art: String,
    pub matrix: crate::image_extractor::ImageLocalMatrix,
}

/// The animations a set of parts gives an entity: one per part (its whole timeline) and one
/// per label of it, cut from the same frames.
fn push_part_animations(
    keyframes: &mut Vec<Value>, layers: &mut Vec<Value>, symbols: &mut Vec<Value>,
    char_id: &str, id_prefix: &str, parts: &[ProjectilePartData], animations: &mut Vec<Value>,
) {
    for part in parts {
        let total = part.frame_count as usize;
        let mut cuts: Vec<(String, usize, usize)> = vec![(part.anim.clone(), 0, total.max(1))];
        for (i, (label, at)) in part.labels.iter().enumerate() {
            let start = (*at as usize).saturating_sub(1);
            let end = part.labels.get(i + 1).map(|(_, f)| (*f as usize).saturating_sub(1)).unwrap_or(total).max(start + 1);
            cuts.push((format!("{}_{}", part.anim, crate::haxe_gen::label_ident(label)), start, end));
        }
        for (name, start, end) in cuts {
            let key = format!("{id_prefix}_{name}");
            let img_layer_ids = push_part_layers(keyframes, layers, symbols, char_id,
                &key, start..end, &part.image_parts,
                (&part.image_frames, &part.image_matrices), &part.image_guids);
            animations.push(json!({
                "$id": uuid(char_id, &format!("{key}_anim")),
                "name": name,
                "layers": img_layer_ids.iter().cloned().map(Value::from).collect::<Vec<Value>>(),
                "pluginMetadata": {}
            }));
        }
    }
}

thread_local! {
    static DROPPED_ANIMATIONS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// The animations the character entity left out as empty, for the script's tables to leave out
/// too: a name only a table has is asked of the engine and refused.
fn note_dropped_animations(names: Vec<String>) { DROPPED_ANIMATIONS.with(|c| *c.borrow_mut() = names); }
pub fn take_dropped_animations() -> Vec<String> { DROPPED_ANIMATIONS.with(|c| std::mem::take(&mut *c.borrow_mut())) }

/// A platform an object gives itself: the source builds a box at an offset from the object
/// and fighters stand on it (rayman's plum, fox's landmaster). Here a structure rides the
/// object (its own `__selfPlatform` and the follower script beside its stats), and this is
/// the line it carries: as wide as the box, centred on the structure's origin, ledges at both
/// ends.
fn push_platform_animations(
    keyframes: &mut Vec<Value>, layers: &mut Vec<Value>, symbols: &mut Vec<Value>, animations: &mut Vec<Value>,
    char_id: &str, id_prefix: &str, platforms: &[(f64, f64)],
) {
    for (i, (w, _h)) in platforms.iter().enumerate() {
        let w = (*w * crate::mappings::character_stats().scaling.size_multiplier).max(8.0);
        let sym = det_uuid(&format!("{char_id}::{id_prefix}::platform{i}::sym"));
        symbols.push(json!({
            "$id": sym, "type": "LINE_SEGMENT", "alpha": 0.5, "color": "0xeeeeee",
            "points": [-w / 2.0, 0.0, w / 2.0, 0.0],
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "structureType": "FLOOR", "leftLedge": true, "rightLedge": true, "dropThrough": false } }
        }));
        let kf = det_uuid(&format!("{char_id}::{id_prefix}::platform{i}::kf"));
        keyframes.push(json!({ "$id": kf, "length": 1, "pluginMetadata": {}, "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "LINE_SEGMENT" }));
        let line = det_uuid(&format!("{char_id}::{id_prefix}::platform{i}::layer"));
        layers.push(json!({
            "$id": line, "hidden": false, "locked": false, "name": "Line Segment Layer", "type": "LINE_SEGMENT",
            "keyframes": [kf], "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "lineSegmentType": "LINE_SEGMENT_STRUCTURE" } }
        }));
        animations.push(json!({
            "$id": det_uuid(&format!("{char_id}::{id_prefix}::platform{i}::anim")),
            "name": format!("platformSprite{i}"), "pluginMetadata": {}, "layers": [line]
        }));
    }
}

/// Image + box data for one state of a multi-state projectile.
#[derive(Debug, Clone)]
pub struct ProjectileStateData {
    pub label: String,
    /// the clip the state places as `stance`, whose frames it draws
    pub sprite_id: u16,
    pub image_frames: Vec<Option<String>>,
    pub image_matrices: Vec<Option<crate::image_extractor::ImageLocalMatrix>>,
    pub image_parts: Vec<Vec<(String, crate::image_extractor::ImageLocalMatrix)>>,
    pub image_guids: BTreeMap<String, String>,
    pub boxes: Option<AnimationBoxData>,
    pub frame_count: u16,
    /// This state's OWN timeline scripts, by 1-based source frame (frame 1 included: it runs
    /// the moment the state's clip appears, which is the moment the state starts). A state's
    /// clip carries its own code, and a script that belonged to another state's frame 7 was
    /// ending the flight of sora's keyblade on frame 7 of its throw.
    pub frame_scripts: BTreeMap<u16, String>,
}

/// Generate a projectile.entity file for a single SSF2 projectile.
///
/// SSF2 projectile structure:
///   Root sprite (e.g. `mario_fireball`, 1 frame):
///     - LABEL 'attack_idle'
///     - PlaceObject with name='stance' → inner animation sprite
///   Inner sprite (e.g. `mario_fla.mario_fireball_mc_210`, N frames):
///     - Collision boxes (hitBox, attackBox)
///     - Image placements per frame
///     - Labels (e.g. 'loop')
///
/// Fraymakers projectile animations:
///   - `projectileSpawn` — 1 frame, first frame image
///   - `projectileIdle` — all frames with hitbox/hurtbox
///   - `projectileDestroy` — last frame image + blank frame script
/// Map an SSF2 projectile outer-sprite frame label to a Fraymakers animation name.
/// attack_idle → projectileIdle (the default), attack_hold → projectileHeld, etc.
pub fn ssf2_proj_label_to_fm_anim(label: &str) -> String {
    match label {
        "attack_idle" => "projectileIdle".to_string(),
        "attack_hold" => "projectileHeld".to_string(),
        "attack_toss" => "projectileActive".to_string(),
        other => format!("projectile_{}", other.replace(|c: char| !c.is_alphanumeric(), "_")),
    }
}

/// FrayTools IMAGE-symbol transform (scaleX, scaleY, rotation, x, y) from an
/// optional SSF2 local matrix. Projectiles/effects are standalone (no root MC to
/// compose), so the local matrix applies directly. None → identity (the old default,
/// which rendered large bitmaps at full native size — i.e. way too big).
fn proj_image_xform(mat: Option<&crate::image_extractor::ImageLocalMatrix>) -> (f64, f64, f64, f64, f64) {
    match mat {
        Some(m) => (
            round2(m.sx),
            round2(m.sy),
            round2(((m.rotation % 360.0) + 360.0) % 360.0),
            round2(m.tx),
            round2(m.ty),
        ),
        None => (1.0, 1.0, 0.0, 0.0, 0.0),
    }
}

/// The parts a clip draws, as IMAGE layers: layer k shows the k-th placement (depth order) of
/// every frame in `frames`, blank where a frame has fewer. When the clip's parts are not known
/// (`parts` empty) the one-image-per-frame view stands in. The layer ids, deepest first.
#[allow(clippy::too_many_arguments)]
fn push_part_layers(
    keyframes: &mut Vec<Value>,
    layers: &mut Vec<Value>,
    symbols: &mut Vec<Value>,
    char_id: &str,
    key: &str,
    frames: std::ops::Range<usize>,
    parts: &[Vec<(String, crate::image_extractor::ImageLocalMatrix)>],
    single: (&[Option<String>], &[Option<crate::image_extractor::ImageLocalMatrix>]),
    guids: &BTreeMap<String, String>,
) -> Vec<String> {
    let part_at = |f: usize, k: usize| -> Option<(String, crate::image_extractor::ImageLocalMatrix)> {
        if parts.is_empty() {
            if k > 0 { return None }
            let sym = single.0.get(f).and_then(|s| s.clone())?;
            Some((sym, single.1.get(f).and_then(|m| *m).unwrap_or_default()))
        } else {
            parts.get(f).and_then(|p| p.get(k)).cloned()
        }
    };
    let count = if parts.is_empty() { 1 } else {
        frames.clone().map(|f| parts.get(f).map(|p| p.len()).unwrap_or(0)).max().unwrap_or(0).max(1)
    };
    let mut ids: Vec<String> = Vec::new();
    for k in 0..count {
        let mut kf_ids: Vec<String> = Vec::new();
        for f in frames.clone() {
            let suffix = if k == 0 { format!("{f}") } else { format!("p{k}_{f}") };
            let kf_id = uuid(char_id, &format!("{key}_img_kf_{suffix}"));
            let placed = part_at(f, k).and_then(|(sym, mat)| guids.get(sym.as_str()).cloned().map(|g| (g, mat)));
            match placed {
                Some((guid, mat)) => {
                    let sym_id = uuid(char_id, &format!("{key}_img_sym_{suffix}"));
                    let (sx, sy, rot, x, y) = proj_image_xform(Some(&mat));
                    symbols.push(json!({
                        "$id": sym_id, "alpha": 1, "imageAsset": guid, "pivotX": 0, "pivotY": 0,
                        "pluginMetadata": {}, "rotation": rot, "scaleX": sx, "scaleY": sy,
                        "type": "IMAGE", "x": x, "y": y
                    }));
                    keyframes.push(json!({ "$id": kf_id, "length": 1, "pluginMetadata": {},
                        "symbol": sym_id, "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" }));
                }
                None => keyframes.push(json!({ "$id": kf_id, "length": 1, "pluginMetadata": {},
                    "symbol": Value::Null, "tweenType": "LINEAR", "tweened": false, "type": "IMAGE" })),
            }
            kf_ids.push(kf_id);
        }
        let layer_id = if k == 0 { uuid(char_id, &format!("{key}_img_layer")) } else { uuid(char_id, &format!("{key}_img_layer_p{k}")) };
        layers.push(json!({
            "$id": layer_id, "name": if k == 0 { "Image Layer".to_string() } else { format!("Image Layer {}", k + 1) },
            "type": "IMAGE", "keyframes": kf_ids, "hidden": false, "locked": false, "pluginMetadata": {}
        }));
        // the deepest part first, so a later one draws over it as the clip does
        ids.insert(0, layer_id);
    }
    ids
}

/// One FRAME_SCRIPT layer for a projectile timeline: each script at its own 1-based source
/// frame, blank keyframes between. The id of the layer it pushed.
fn push_script_layer(
    keyframes: &mut Vec<Value>,
    layers: &mut Vec<Value>,
    char_id: &str,
    key: &str,
    total_frames: u32,
    scripts: &BTreeMap<u16, String>,
) -> String {
    let mut kf_ids: Vec<String> = Vec::new();
    let mut cursor: u32 = 0;
    for (src_frame, code) in scripts {
        let at = (*src_frame as u32).saturating_sub(1);
        if at >= total_frames { continue }
        if at > cursor {
            let gap = uuid(char_id, &format!("{key}_gap_{cursor}"));
            keyframes.push(json!({ "$id": gap, "type": "FRAME_SCRIPT",
                "length": at - cursor, "code": "", "pluginMetadata": {} }));
            kf_ids.push(gap);
        }
        let kf = uuid(char_id, &format!("{key}_kf_{at}"));
        keyframes.push(json!({ "$id": kf, "type": "FRAME_SCRIPT",
            "length": 1, "code": code, "pluginMetadata": {} }));
        kf_ids.push(kf);
        cursor = at + 1;
    }
    if cursor < total_frames {
        let tail = uuid(char_id, &format!("{key}_tail_{cursor}"));
        keyframes.push(json!({ "$id": tail, "type": "FRAME_SCRIPT",
            "length": total_frames - cursor, "code": "", "pluginMetadata": {} }));
        kf_ids.push(tail);
    }
    let layer_id = uuid(char_id, &format!("{key}_layer"));
    layers.push(json!({
        "$id": layer_id, "name": "Scripts", "type": "FRAME_SCRIPT",
        "keyframes": kf_ids, "hidden": false, "locked": false, "pluginMetadata": {}
    }));
    layer_id
}

/// `runtime_runs_frames`: the runtime runs the clip's own frame scripts (a fighter's projectile),
/// so its script layers stay blank.
pub fn generate_projectile_entity(
    char_id: &str,
    proj: &ProjectileInfo,
    runtime_runs_frames: bool,
) -> String {
    let none: BTreeMap<u16, String> = BTreeMap::new();
    let mut keyframes: Vec<Value> = Vec::new();
    let mut layers: Vec<Value> = Vec::new();
    let mut symbols: Vec<Value> = Vec::new();
    let mut animations: Vec<Value> = Vec::new();

    let proj_id = &proj.name;
    let total_frames = proj.inner_frame_count.max(1) as u32;
    // what each animation shows of the source's clips (for the runtime's path): the clip placed
    // as `stance` under the idle state's label, else the projectile's own clip
    let (idle_sym, idle_label) = match proj.inner_sprite_id {
        Some(id) => (id, "attack_idle".to_string()),
        None => (proj.sprite_id, String::new()),
    };
    let piece = |anim: &str, label: &str, sym: u16, from: u32, frames: u32, borrowed: bool| crate::emit::Piece {
        fm_name: anim.to_string(), label: label.to_string(), sym, from, frames: frames.max(1),
        head: 0, reversed: false, borrowed, item_box: Vec::new(), on: None,
    };
    let mut pieces: Vec<crate::emit::Piece> = Vec::new();
    // The same names the stats files use. When these two disagree the state map points at an
    // animation the entity does not have, and the projectile spawns and then sits there: the
    // engine has nothing to play for the state it was put in.
    let anim_names = crate::haxe_gen::projectile_anim_names(proj);

    // ── Image layers for the idle animation: every part the clip draws ───
    let idle_image_layer_ids: Vec<String> = push_part_layers(&mut keyframes, &mut layers, &mut symbols, char_id,
        &format!("proj_{}_idle", proj_id), 0..total_frames as usize, &proj.image_parts,
        (&proj.image_frames, &proj.image_matrices), &proj.image_guids);
    let _idle_image_layer_id = idle_image_layer_ids.last().cloned().expect("an image layer");

    // ── The clip's OWN timeline scripts ──────────────────────────────────
    // A projectile's clip carries code on its frames exactly as a character's does, and it is
    // that code -- not anything this converter decides a projectile ought to do -- which says
    // what this one does. Dropped, a projectile did only what the converter wrote for it.
    // Laid out in SOURCE frames: `double_keyframe_lengths` at the end of this function puts the
    // whole entity on this engine's clock in one place, so doubling here as well would put a
    // script on frame 10 at frame 40.
    let idle_script_layer: Option<String> = (!proj.frame_scripts.is_empty()).then(|| {
        push_script_layer(&mut keyframes, &mut layers, char_id, &format!("proj_{}_scr", proj_id), total_frames,
            if runtime_runs_frames { &none } else { &proj.frame_scripts })
    });

    // ── Build collision box layers for idle (from inner sprite boxes) ────
    let mut idle_box_layer_ids: Vec<String> = Vec::new();
    if let Some(boxes) = &proj.boxes {
        // Collect all unique instance names across frames
        // a name is not unique on a frame; see the character path above
        let mut instances_in_proj: std::collections::BTreeSet<(String, usize)> = Default::default();
        for frame_boxes in boxes.frames.values() {
            let mut seen: std::collections::BTreeMap<&str, usize> = Default::default();
            for b in frame_boxes {
                let n = seen.entry(b.instance_name.as_str()).or_insert(0);
                instances_in_proj.insert((b.instance_name.clone(), *n));
                *n += 1;
            }
        }
        let layer_names = box_layer_names(&instances_in_proj);

        for (inst_name, occurrence) in instances_in_proj.iter() {
            let occurrence = *occurrence;
            let occ_key = if occurrence == 0 { String::new() } else { format!("_{occurrence}") };
            let box_type = BoxType::from_instance_name(inst_name).unwrap_or(BoxType::Hurtbox);
            let fm_box_type = box_type_to_fm(box_type);
            let color = box_color(box_type);
            let (prefix, box_idx) = layer_names[&(inst_name.clone(), occurrence)].clone();
            let fm_layer_name = format!("{prefix}{box_idx}");

            let mut box_kf_ids: Vec<String> = Vec::new();
            let mut frame_idx: u32 = 0;

            // Collect active frames for this instance
            let mut active_frames: Vec<(usize, &crate::sprite_parser::FrameBox)> = Vec::new();
            for f in 0..total_frames {
                if let Some(frame_boxes) = boxes.frames.get(&(f as u16)) {
                    if let Some(fb) = frame_boxes.iter()
                        .filter(|b| &b.instance_name == inst_name).nth(occurrence)
                    {
                        active_frames.push((f as usize, fb));
                    }
                }
            }

            let is_point = box_type == crate::sprite_parser::BoxType::GrabHoldBox;
            let kf_type = if is_point { "POINT" } else { "COLLISION_BOX" };

            // Build keyframes using run-length encoding
            let mut i = 0;
            while i < active_frames.len() {
                let (start_frame, fb) = active_frames[i];
                let mut run_end = i;
                while run_end + 1 < active_frames.len() {
                    let (next_f, next_fb) = active_frames[run_end + 1];
                    let (cur_f, _) = active_frames[run_end];
                    let consecutive = next_f == cur_f + 1;
                    let same_geom = (next_fb.x - fb.x).abs() < 0.01
                        && (next_fb.y - fb.y).abs() < 0.01
                        && (next_fb.width - fb.width).abs() < 0.01
                        && (next_fb.height - fb.height).abs() < 0.01
                        && (next_fb.rotation - fb.rotation).abs() < 0.01;
                    if consecutive && same_geom { run_end += 1; } else { break; }
                }
                let run_len = (active_frames[run_end].0 - start_frame + 1) as u32;

                // Gap keyframe
                if start_frame as u32 > frame_idx {
                    let gap_kf_id = uuid(char_id, &format!("proj_{}_box_gap_{}{}_{}", proj_id, inst_name, occ_key, frame_idx));
                    keyframes.push(json!({
                        "$id": gap_kf_id,
                        "type": kf_type,
                        "length": (start_frame as u32) - frame_idx,
                        "symbol": Value::Null,
                        "tweened": false,
                        "tweenType": "LINEAR",
                        "pluginMetadata": {}
                    }));
                    box_kf_ids.push(gap_kf_id);
                }

                // Box symbol
                let sym_id = uuid(char_id, &format!("proj_{}_box_sym_{}{}_{}", proj_id, inst_name, occ_key, start_frame));
                if is_point {
                    let cx = round2(fb.x + fb.width / 2.0);
                    let cy = round2(fb.y + fb.height);
                    symbols.push(json!({
                        "$id": sym_id,
                        "alpha": 1,
                        "color": color,
                        "pluginMetadata": {},
                        "rotation": 0,
                        "type": "POINT",
                        "x": cx,
                        "y": cy
                    }));
                } else {
                    // Mirror the Character.entity collision-box emission exactly
                    // (see the path above): ItemBox pivots on its bottom-centre
                    // (the hand) and bakes the rotation into the top-left so the
                    // hand stays pinned under rotation; every other box type is
                    // already AABB-collapsed to rotation 0 in sprite_parser, so
                    // it uses a centre pivot and raw top-left. Without this,
                    // rotated projectile itemboxes drift exactly like the
                    // character path did before the bake.
                    let (pivot_x, pivot_y) = if box_type == crate::sprite_parser::BoxType::ItemBox {
                        (fb.width / 2.0, fb.height)        // bottom-centre = hand
                    } else {
                        (fb.width / 2.0, fb.height / 2.0)  // centre
                    };
                    let theta = ((fb.rotation % 360.0) + 360.0) % 360.0;
                    let (box_x, box_y) = if box_type == crate::sprite_parser::BoxType::ItemBox {
                        let hand = crate::fraytools_transform::intended_pivot_point(
                            fb.x, fb.y, pivot_x, pivot_y);
                        let off = crate::fraytools_transform::collision_box_anchor(
                            0.0, 0.0, pivot_x, pivot_y, theta);
                        (hand.0 - off.0, hand.1 - off.1)
                    } else {
                        (fb.x, fb.y)
                    };
                    symbols.push(json!({
                        "$id": sym_id,
                        "alpha": 0.5,
                        "color": color,
                        "pivotX": round2(pivot_x),
                        "pivotY": round2(pivot_y),
                        "pluginMetadata": {},
                        "rotation": round2(theta),
                        "scaleX": round2(fb.width),
                        "scaleY": round2(fb.height),
                        "type": "COLLISION_BOX",
                        "x": round2(box_x),
                        "y": round2(box_y)
                    }));
                }

                let kf_id = uuid(char_id, &format!("proj_{}_box_kf_{}{}_{}", proj_id, inst_name, occ_key, start_frame));
                keyframes.push(json!({
                    "$id": kf_id,
                    "type": kf_type,
                    "length": run_len,
                    "symbol": sym_id,
                    "tweened": false,
                    "tweenType": "LINEAR",
                    "pluginMetadata": {}
                }));
                box_kf_ids.push(kf_id);
                frame_idx = start_frame as u32 + run_len;
                i = run_end + 1;
            }

            // Tail gap
            if frame_idx < total_frames {
                let tail_kf_id = uuid(char_id, &format!("proj_{}_box_tail_{}{}_{}", proj_id, inst_name, occ_key, frame_idx));
                keyframes.push(json!({
                    "$id": tail_kf_id,
                    "type": kf_type,
                    "length": total_frames - frame_idx,
                    "symbol": Value::Null,
                    "tweened": false,
                    "tweenType": "LINEAR",
                    "pluginMetadata": {}
                }));
                box_kf_ids.push(tail_kf_id);
            }

            if box_kf_ids.is_empty() { continue; }

            let layer_id = uuid(char_id, &format!("proj_{}_box_layer_{}{}", proj_id, inst_name, occ_key));
            if is_point {
                layers.push(json!({
                    "$id": layer_id,
                    "name": fm_layer_name,
                    "type": "POINT",
                    "keyframes": box_kf_ids,
                    "hidden": false,
                    "locked": false,
                    "pluginMetadata": {
                        "com.fraymakers.FraymakersMetadata": {
                            "pointType": "GRAB_HOLD_POINT"
                        }
                    }
                }));
            } else {
                layers.push(json!({
                    "$id": layer_id,
                    "name": fm_layer_name,
                    "type": "COLLISION_BOX",
                    "keyframes": box_kf_ids,
                    "hidden": false,
                    "locked": false,
                    "defaultAlpha": 0.5,
                    "defaultColor": color,
                    "pluginMetadata": {
                        "com.fraymakers.FraymakersMetadata": {
                            "collisionBoxType": fm_box_type,
                            "index": box_idx
                        }
                    }
                }));
            }
            idle_box_layer_ids.push(layer_id);
        }
    }

    // ── projectileSpawn animation (1 frame, first image, every part) ─────
    {
        let spawn_layers = push_part_layers(&mut keyframes, &mut layers, &mut symbols, char_id,
            &format!("proj_{}_spawn", proj_id), 0..1usize, &proj.image_parts,
            (&proj.image_frames, &proj.image_matrices), &proj.image_guids);
        pieces.push(piece(&anim_names.spawn, &idle_label, idle_sym, 1, 1, true));
        animations.push(json!({
            "$id": uuid(char_id, &format!("proj_{}_anim_spawn", proj_id)),
            "name": anim_names.spawn,
            "layers": spawn_layers,
            "pluginMetadata": {}
        }));
    }

    // ── projectileIdle animation (all frames + boxes) ────────────────────
    {
        let mut idle_layers = idle_image_layer_ids.clone();
        if let Some(l) = &idle_script_layer { idle_layers.push(l.clone()); }
        idle_layers.extend(idle_box_layer_ids.iter().cloned());
        pieces.push(piece(&anim_names.active, &idle_label, idle_sym, 1, total_frames, false));
        animations.push(json!({
            "$id": uuid(char_id, &format!("proj_{}_anim_idle", proj_id)),
            "name": anim_names.active,
            "layers": idle_layers,
            "pluginMetadata": {}
        }));
    }

    // ── projectileDestroy animation (last frame + blank script) ──────────
    {
        let last_frame = total_frames.saturating_sub(1) as usize;
        let destroy_layers = push_part_layers(&mut keyframes, &mut layers, &mut symbols, char_id,
            &format!("proj_{}_destroy", proj_id), last_frame..last_frame + 1, &proj.image_parts,
            (&proj.image_frames, &proj.image_matrices), &proj.image_guids);
        let destroy_script_kf = uuid(char_id, &format!("proj_{}_destroy_script_kf", proj_id));
        keyframes.push(json!({
            "$id": destroy_script_kf,
            "length": 1,
            "pluginMetadata": {},
            "symbol": Value::Null,
            "tweenType": "LINEAR",
            "tweened": false,
            "type": "FRAME_SCRIPT"
        }));
        let destroy_script_layer = uuid(char_id, &format!("proj_{}_destroy_script_layer", proj_id));
        layers.push(json!({
            "$id": destroy_script_layer,
            "name": "Frame Script Layer",
            "type": "FRAME_SCRIPT",
            "keyframes": [destroy_script_kf],
            "hidden": false,
            "locked": false,
            "pluginMetadata": {}
        }));

        pieces.push(piece(&anim_names.destroy, &idle_label, idle_sym, total_frames, 1, true));
        animations.push(json!({
            "$id": uuid(char_id, &format!("proj_{}_anim_destroy", proj_id)),
            "name": anim_names.destroy,
            "layers": destroy_layers.iter().cloned().map(Value::from).chain(std::iter::once(Value::from(destroy_script_layer))).collect::<Vec<Value>>(),
            "pluginMetadata": {}
        }));
    }

    // ── THE CLIP'S OWN SECTIONS ──────────────────────────────────────────────
    // A projectile clip with labels is several animations on one timeline, and the class jumps
    // between them by name: birdo's egg flies in `idle` (a two-frame loop its own frame script
    // restarts), dies in `die` (a bounce, then the frame that ends it) and carries `suspend` and
    // `chibi` for what another fighter may do to it. Laid out as ONE animation, the first loop
    // played forever and a play of `die` landed on a one-frame stub. So each label is cut out of
    // the full layers into an animation of its own; whole-clip stubs that took a label's name go.
    {
        let ident = crate::haxe_gen::label_ident;
        let mut labels: Vec<(u32, String)> = proj.inner_labels.iter()
            .map(|(f, l)| ((*f as u32).saturating_sub(1).min(total_frames.saturating_sub(1)), ident(l)))
            .collect();
        labels.sort_by_key(|(f, _)| *f);
        labels.dedup_by(|a, b| a.0 == b.0);
        if labels.len() >= 2 {
            // frames before the first label belong to it: the clip starts there and runs in
            let mut windows: Vec<(String, u32, u32)> = Vec::new();
            for (i, (f, name)) in labels.iter().enumerate() {
                let start = if i == 0 { 0 } else { *f };
                let end = labels.get(i + 1).map(|(nf, _)| *nf).unwrap_or(total_frames);
                if end > start { windows.push((name.clone(), start, end)); }
            }
            // one layer's keyframes, cut to a window: the part of each that overlaps
            let kf_index: std::collections::BTreeMap<String, usize> = keyframes.iter().enumerate()
                .filter_map(|(i, k)| k["$id"].as_str().map(|id| (id.to_string(), i))).collect();
            let full_layers: Vec<Value> = {
                let mut v: Vec<Value> = idle_image_layer_ids.iter()
                    .map(|id| layers.iter().find(|l| l["$id"] == json!(id)).cloned().expect("image layer")).collect();
                if let Some(sl) = &idle_script_layer { v.push(layers.iter().find(|l| l["$id"] == json!(sl)).cloned().expect("script layer")); }
                for b in &idle_box_layer_ids { v.push(layers.iter().find(|l| l["$id"] == json!(b)).cloned().expect("box layer")); }
                v
            };
            let mut new_kfs: Vec<Value> = Vec::new();
            let mut new_layers: Vec<Value> = Vec::new();
            let mut seg_anims: Vec<Value> = Vec::new();
            for (name, start, end) in &windows {
                let mut layer_ids: Vec<String> = Vec::new();
                for layer in &full_layers {
                    let lid = layer["$id"].as_str().unwrap_or_default().to_string();
                    let mut ids: Vec<String> = Vec::new();
                    let mut at: u32 = 0;
                    for (k, kid) in layer["keyframes"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                        let Some(&ki) = kid.as_str().and_then(|id| kf_index.get(id)) else { continue };
                        let len = keyframes[ki]["length"].as_u64().unwrap_or(1) as u32;
                        let (a, b) = (at.max(*start), (at + len).min(*end));
                        at += len;
                        if b <= a { continue }
                        let mut kf = keyframes[ki].clone();
                        let nid = uuid(char_id, &format!("proj_{}_seg_{}_{}_{}", proj_id, name, lid, k));
                        kf["$id"] = json!(nid);
                        kf["length"] = json!(b - a);
                        new_kfs.push(kf);
                        ids.push(nid);
                    }
                    if ids.is_empty() { continue }
                    let mut nl = layer.clone();
                    let nlid = uuid(char_id, &format!("proj_{}_seg_{}_layer_{}", proj_id, name, lid));
                    nl["$id"] = json!(nlid);
                    nl["keyframes"] = json!(ids);
                    new_layers.push(nl);
                    layer_ids.push(nlid);
                }
                pieces.push(piece(name, &idle_label, idle_sym, start + 1, end - start, false));
                seg_anims.push(json!({
                    "$id": uuid(char_id, &format!("proj_{}_anim_seg_{}", proj_id, name)),
                    "name": name, "layers": layer_ids, "pluginMetadata": {}
                }));
            }
            let taken: std::collections::BTreeSet<String> = windows.iter().map(|(n, _, _)| n.clone()).collect();
            animations.retain(|a| !a["name"].as_str().is_some_and(|n| taken.contains(n)));
            animations.extend(seg_anims);
            keyframes.extend(new_kfs);
            layers.extend(new_layers);
            // what no animation names any more goes with it
            let used_layers: std::collections::BTreeSet<String> = animations.iter()
                .flat_map(|a| a["layers"].as_array().cloned().unwrap_or_default())
                .filter_map(|v| v.as_str().map(str::to_string)).collect();
            layers.retain(|l| l["$id"].as_str().is_some_and(|id| used_layers.contains(id)));
            let used_kfs: std::collections::BTreeSet<String> = layers.iter()
                .flat_map(|l| l["keyframes"].as_array().cloned().unwrap_or_default())
                .filter_map(|v| v.as_str().map(str::to_string)).collect();
            keyframes.retain(|k| k["$id"].as_str().is_some_and(|id| used_kfs.contains(id)));
            let used_syms: std::collections::BTreeSet<String> = keyframes.iter()
                .filter_map(|k| k["symbol"].as_str().map(str::to_string)).collect();
            symbols.retain(|sy| sy["$id"].as_str().is_some_and(|id| used_syms.contains(id)));
        }
    }

    // ── Extra animations for multi-state projectiles (e.g. link_bomb) ────────
    for state_data in &proj.extra_states {
        let state_slug = state_data.label.replace(|c: char| !c.is_alphanumeric(), "_");
        let anim_name = ssf2_proj_label_to_fm_anim(&state_data.label);

        // Image layers: every part the state's clip draws
        let total = state_data.frame_count as usize;
        let img_layer_ids = push_part_layers(&mut keyframes, &mut layers, &mut symbols, char_id,
            &format!("proj_{}_extra_{}", proj_id, state_slug), 0..total.max(1), &state_data.image_parts,
            (&state_data.image_frames, &state_data.image_matrices), &state_data.image_guids);

        // the state's own timeline scripts, on the state's own timeline
        let script_layer_id = push_script_layer(&mut keyframes, &mut layers, char_id,
            &format!("proj_{}_extra_{}_script", proj_id, state_slug), total.max(1) as u32,
            if runtime_runs_frames { &none } else { &state_data.frame_scripts });

        pieces.push(piece(&anim_name, &state_data.label, state_data.sprite_id, 1, state_data.frame_count as u32, false));
        animations.push(json!({
            "$id": uuid(char_id, &format!("proj_{}_extra_{}_anim", proj_id, state_slug)),
            "name": anim_name,
            "layers": img_layer_ids.iter().cloned().map(Value::from).chain(std::iter::once(Value::from(script_layer_id))).collect::<Vec<Value>>(),
            "pluginMetadata": {}
        }));
    }

    // the named children its code addresses: one animation each, and one per label
    push_part_animations(&mut keyframes, &mut layers, &mut symbols, char_id, &format!("proj_{proj_id}"), &proj.parts, &mut animations);

    // 30fps → 60fps: hold every keyframe for two frames (see fn docs).
    double_keyframe_lengths(&mut keyframes);

    // A projectile whose clip carries ONE label spawns, lives and dies under that one name, so
    // the three animations collapse into it. Two animations of the same name is not a thing an
    // entity can have, and the one that matters is the one with the frames and the boxes.
    {
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut keep: Vec<Value> = Vec::new();
        let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
        for a in &animations {
            let name = a["name"].as_str().unwrap_or_default().to_string();
            let layers = a["layers"].as_array().map(|l| l.len()).unwrap_or(0);
            match by_name.get(&name) {
                Some(&i) if keep[i]["layers"].as_array().map(|l| l.len()).unwrap_or(0) >= layers => {}
                Some(&i) => keep[i] = a.clone(),
                None => {
                    by_name.insert(name.clone(), keep.len());
                    seen.insert(name);
                    keep.push(a.clone());
                }
            }
        }
        animations = keep;
    }

    // a platform the clip gives itself: a floor line a structure content stands on
    push_platform_animations(&mut keyframes, &mut layers, &mut symbols, &mut animations, char_id, &proj_id.to_string(), &proj.platforms);
    // the pieces the runtime plays: one per animation the entity kept (a name two of them shared
    // is the one that shows the clip's own frames, not a borrowed one)
    {
        let kept: std::collections::BTreeSet<&str> = animations.iter().filter_map(|a| a["name"].as_str()).collect();
        let mut out: Vec<crate::emit::Piece> = Vec::new();
        for p in pieces {
            if !kept.contains(p.fm_name.as_str()) { continue }
            // a later cut of the same name is the one the entity kept (a label's own window
            // replaces the whole-clip stub that took its name); a borrowed one never replaces its own
            match out.iter().position(|o| o.fm_name == p.fm_name) {
                Some(i) if !p.borrowed || out[i].borrowed => out[i] = p,
                Some(_) => {}
                None => out.push(p),
            }
        }
        PROJECTILE_PIECES.with(|c| { c.borrow_mut().insert(proj.name.clone(), out); });
    }

    let entity_id = crate::projectile_gen::projectile_content_id(&proj.name);
    let entity = json!({
        "animations": animations,
        "export": true,
        "guid": uuid(char_id, &format!("proj_{}_entity_guid", proj_id)),
        "id": entity_id,
        "keyframes": keyframes,
        "layers": layers,
        "paletteMap": Value::Null,
        "pluginMetadata": {
            "com.fraymakers.FraymakersMetadata": {
                "objectType": "PROJECTILE",
                "version": "0.1.1"
            }
        },
        "plugins": ["com.fraymakers.FraymakersMetadata"],
        "symbols": symbols,
        "tags": [],
        "terrains": [],
        "tilesets": [],
        "version": 14
    });

    finish_entity(entity)
}

/// Compute the list of animation names a `generate_effect_entity` call would
/// emit, given an effect's discovered FrameLabels. Used to build the
/// effect→primary-animation map that drives the context-aware
/// `self.attachEffect("name")` → `match.createVfx(...)` rewrite. Keep
/// this in lockstep with the `segments` block of `generate_effect_entity`.
pub fn effect_animation_names(effect: &crate::image_extractor::DiscoveredEffect) -> Vec<String> {
    let total_frames = effect.frame_count.max(1) as usize;
    if effect.inner_labels.is_empty() {
        return vec!["active".to_string()];
    }
    let mut names: Vec<String> = Vec::new();
    for (i, (frame_1based, label)) in effect.inner_labels.iter().enumerate() {
        let start = (frame_1based.saturating_sub(1)) as usize;
        let end = if i + 1 < effect.inner_labels.len() {
            effect.inner_labels[i + 1].0.saturating_sub(1) as usize
        } else {
            total_frames
        };
        if end > start {
            let anim_name: String = label
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect();
            names.push(anim_name);
        }
    }
    if let Some(first_start) = effect.inner_labels.first().map(|(f, _)| f.saturating_sub(1) as usize) {
        if first_start > 0 {
            names.insert(0, "active".to_string());
        }
    }
    if names.is_empty() {
        names.push("active".to_string());
    }
    names
}

/// Generate a `<effect_name>.entity` file for one SSF2 effect sprite.
///
/// Effects are pure visual playback — no scripts, no stats, no manifest
/// entry. The character's `Script.hx` triggers them with
/// `match.createVfx(new VfxStats({ spriteContent: self.getResource()
/// .getContent("<name>"), animation: "<label-or-vfx>" }), self)`.
///
/// SSF2 effect structure:
///   Effect sprite (e.g. `effect_land`, N frames):
///     - FrameLabels at various 1-based frames split it into named
///       sub-animations.
///     - Image placements per frame.
///
/// Fraymakers effect animations:
///   - One animation per inner FrameLabel (animation name = label).
///   - If there are no labels, a single `vfx` animation covering all frames.
///
/// Each animation has a single Image Layer; no collision/script layers.
pub fn generate_effect_entity(
    char_id: &str,
    effect: &crate::image_extractor::DiscoveredEffect,
    img_result: &crate::image_extractor::ImageExtractionResult,
    parsed_swf: &swf::Swf<'_>,
) -> String {
    let mut keyframes: Vec<Value> = Vec::new();
    let mut layers: Vec<Value> = Vec::new();
    let mut symbols: Vec<Value> = Vec::new();
    let mut animations: Vec<Value> = Vec::new();

    let effect_id = &effect.name;
    let total_frames = effect.frame_count.max(1) as u32;

    // Pull per-frame images by walking the effect sprite's timeline.
    let frame_images = crate::image_extractor::extract_projectile_frame_images_from_swf(
        parsed_swf,
        char_id,
        effect.sprite_id,
        img_result,
    ).unwrap_or(crate::image_extractor::ProjectileFrameImages {
        frames: vec![],
        matrices: vec![],
        image_guids: std::collections::BTreeMap::new(),
        parts: vec![],
    });

    if std::env::var("PEPTIDE_DUMP_EFFECTS").is_ok() {
        for (i, (f, m)) in frame_images.frames.iter().zip(frame_images.matrices.iter()).enumerate() {
            eprintln!("[effect] {} frame {} img={:?} matrix={:?}", effect_id, i, f, m.as_ref().map(|m| (m.tx, m.ty, m.sx, m.sy)));
        }
    }
    // Determine animation segments: (name, start_0based, end_exclusive).
    // Mirror `effect_animation_names`: same ordering and same fallbacks.
    // No labels → single "active" animation covering the whole sprite.
    let segments: Vec<(String, usize, usize)> = if effect.inner_labels.is_empty() {
        vec![("active".to_string(), 0usize, total_frames as usize)]
    } else {
        let mut segs: Vec<(String, usize, usize)> = Vec::new();
        for (i, (frame_1based, label)) in effect.inner_labels.iter().enumerate() {
            let start = (frame_1based.saturating_sub(1)) as usize;
            let end = if i + 1 < effect.inner_labels.len() {
                effect.inner_labels[i + 1].0.saturating_sub(1) as usize
            } else {
                total_frames as usize
            };
            if end > start {
                let anim_name: String = label
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '_' })
                    .collect();
                segs.push((anim_name, start, end));
            }
        }
        // Cover any frames before the first label with a leading `active` segment.
        if let Some(first_start) = effect.inner_labels.first().map(|(f, _)| f.saturating_sub(1) as usize) {
            if first_start > 0 {
                segs.insert(0, ("active".to_string(), 0, first_start));
            }
        }
        if segs.is_empty() {
            segs.push(("active".to_string(), 0, total_frames as usize));
        }
        segs
    };

    EFFECT_PIECES.with(|c| {
        c.borrow_mut().insert(effect.name.clone(), segments.iter().map(|(anim, start, end)| crate::emit::Piece {
            fm_name: anim.clone(), label: String::new(), sym: if BORROWING.with(|b| b.get()) { 0 } else { effect.sprite_id },
            from: *start as u32 + 1, frames: end.saturating_sub(*start).max(1) as u32,
            head: 0, reversed: false, borrowed: false, item_box: Vec::new(), on: None,
        }).collect());
    });
    for (seg_idx, (anim_name, start, end)) in segments.iter().enumerate() {
        let seg_len = end.saturating_sub(*start).max(1);
        // every part the clip draws at once gets a layer of its own: layer k shows the k-th
        // placement (depth order) of each frame, and a frame with fewer parts leaves it blank
        let part_count = (*start..*start + seg_len)
            .map(|f| frame_images.parts.get(f).map(|p| p.len()).unwrap_or(0))
            .max().unwrap_or(0).max(1);
        let mut layer_ids: Vec<Value> = Vec::new();
        for part in 0..part_count {
            let mut img_kf_ids: Vec<Value> = Vec::new();
            for offset in 0..seg_len {
                let frame_idx = *start + offset;
                let kf_id = uuid(char_id, &format!("eff_{}_seg{}_p{}_img_kf_{}", effect_id, seg_idx, part, frame_idx));
                let placed = frame_images.parts.get(frame_idx).and_then(|p| p.get(part));
                if let Some((sym_name, mat)) = placed {
                    if let Some(guid) = frame_images.image_guids.get(sym_name.as_str()) {
                        let sym_id = uuid(char_id, &format!("eff_{}_seg{}_p{}_img_sym_{}", effect_id, seg_idx, part, frame_idx));
                        let (sx, sy, rot, x, y) = proj_image_xform(Some(mat));
                        symbols.push(json!({
                            "$id": sym_id, "alpha": 1, "imageAsset": guid,
                            "pivotX": 0, "pivotY": 0, "pluginMetadata": {},
                            "rotation": rot, "scaleX": sx, "scaleY": sy, "type": "IMAGE", "x": x, "y": y
                        }));
                        keyframes.push(json!({
                            "$id": kf_id, "length": 1, "pluginMetadata": {},
                            "symbol": sym_id, "tweenType": "LINEAR", "tweened": false, "type": "IMAGE"
                        }));
                        img_kf_ids.push(kf_id.into());
                        continue;
                    }
                }
                keyframes.push(json!({
                    "$id": kf_id, "length": 1, "pluginMetadata": {},
                    "symbol": Value::Null, "tweenType": "LINEAR", "tweened": false, "type": "IMAGE"
                }));
                img_kf_ids.push(kf_id.into());
            }
            let img_layer_id = uuid(char_id, &format!("eff_{}_seg{}_p{}_img_layer", effect_id, seg_idx, part));
            layers.push(json!({
                "$id": img_layer_id, "name": format!("Image Layer {}", part + 1), "type": "IMAGE",
                "keyframes": img_kf_ids, "hidden": false, "locked": false, "pluginMetadata": {}
            }));
            // the deepest part first, so the later one draws over it as the clip does
            layer_ids.insert(0, img_layer_id.into());
        }

        animations.push(json!({
            "$id": uuid(char_id, &format!("eff_{}_seg{}_anim", effect_id, seg_idx)),
            "name": anim_name,
            "layers": layer_ids,
            "pluginMetadata": {}
        }));
    }

    // 30fps → 60fps: hold every keyframe for two frames.
    double_keyframe_lengths(&mut keyframes);

    // FrayTools requires every entity to carry the FraymakersMetadata plugin
    // with an objectType, and a non-null paletteMap. Without these, the entity
    // editor throws "Invalid MetadataFieldType for value: null" and
    // "Cannot read property 'paletteCollection' of null" → the whole effect
    // renders as "Error rendering component". VFX entities use objectType
    // "VFX" (they're spawned via match.createVfx); paletteMap may be filled in
    // by the caller (haxe_gen) and defaults to {} otherwise.
    let entity = json!({
        "animations": animations,
        "export": true,
        "guid": uuid(char_id, &format!("eff_{}_entity_guid", effect_id)),
        "id": effect_id,
        "keyframes": keyframes,
        "layers": layers,
        "paletteMap": {},
        "pluginMetadata": {
            "com.fraymakers.FraymakersMetadata": {
                "objectType": "VFX",
                "version": "0.1.0"
            }
        },
        "plugins": ["com.fraymakers.FraymakersMetadata"],
        "symbols": symbols,
        "tags": [],
        "terrains": [],
        "tilesets": [],
        "version": 14
    });

    serde_json::to_string_pretty(&entity).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod box_layer_name_tests {
    use super::box_layer_names;

    fn names(list: &[(&str, usize)]) -> Vec<String> {
        let set = list.iter().map(|(n, o)| (n.to_string(), *o)).collect();
        let out = box_layer_names(&set);
        list.iter().map(|(n, o)| { let (p, i) = &out[&(n.to_string(), *o)]; format!("{p}{i}") }).collect()
    }

    #[test]
    fn a_box_is_named_for_what_it_collides_as() {
        // the engine keys a reflect box on its layer name; the source calls it `reverseBox`
        assert_eq!(names(&[("reverseBox", 0), ("attackBox", 0), ("hand", 0)]),
                   ["reflectbox0", "hitbox0", "ledgegrabbox0"]);
    }

    #[test]
    fn names_sharing_a_kind_take_free_slots() {
        // `hitBox` and `hurtBox` are both hurt boxes, and both used to be named `hurtbox0`
        let got = names(&[("hurtBox", 0), ("hitBox", 0), ("hitBox", 1)]);
        let mut all = got.clone(); all.sort(); all.dedup();
        assert_eq!(all.len(), 3, "every layer needs its own name: {got:?}");
    }

    #[test]
    fn a_box_that_collides_as_nothing_keeps_its_source_name() {
        assert_eq!(names(&[("camBox", 0), ("itemBox", 0)]), ["cambox0", "itembox0"]);
    }
}

#[cfg(test)]
mod turned_tests {
    use super::*;

    #[test]
    fn a_turned_animation_is_mirrored_about_the_origin_and_a_shared_symbol_is_copied() {
        let entity = json!({
            "animations": [
                { "name": "throw_back", "layers": ["l_img", "l_box", "l_pt"] },
                { "name": "stand", "layers": ["l_stand"] }
            ],
            "layers": [
                { "$id": "l_img", "keyframes": ["k1"] }, { "$id": "l_box", "keyframes": ["k2"] },
                { "$id": "l_pt", "keyframes": ["k3"] }, { "$id": "l_stand", "keyframes": ["k4"] }
            ],
            "keyframes": [
                { "$id": "k1", "symbol": "s_img" }, { "$id": "k2", "symbol": "s_box" },
                { "$id": "k3", "symbol": "s_pt" }, { "$id": "k4", "symbol": "s_pt" }
            ],
            "symbols": [
                { "$id": "s_img", "type": "IMAGE", "x": 40.0, "scaleX": 1.15, "scaleY": -1.15, "rotation": 180.0 },
                { "$id": "s_box", "type": "COLLISION_BOX", "x": 10.0, "scaleX": 20.0 },
                { "$id": "s_pt", "type": "POINT", "x": -23.0 }
            ]
        });
        let anims: std::collections::BTreeSet<String> = ["throw_back".to_string()].into();
        let out: serde_json::Value = serde_json::from_str(&draw_turned(&entity.to_string(), &anims)).unwrap();
        let sym = |id: &str| out["symbols"].as_array().unwrap().iter().find(|s| s["$id"] == id).unwrap().clone();
        // the picture is drawn the other way round: x and its x scale negated, its turn reversed
        assert_eq!(sym("s_img")["x"], json!(-40.0));
        assert_eq!(sym("s_img")["scaleX"], json!(-1.15));
        assert_eq!(sym("s_img")["rotation"], json!(180.0));
        // a box keeps its width and covers the mirrored span
        assert_eq!(sym("s_box")["x"], json!(-30.0));
        // the point the standing animation also shows is left as it was, and the throw gets a copy
        assert_eq!(sym("s_pt")["x"], json!(-23.0));
        assert_eq!(sym("s_pt-turned")["x"], json!(23.0));
        let k3 = out["keyframes"].as_array().unwrap().iter().find(|k| k["$id"] == "k3").unwrap().clone();
        assert_eq!(k3["symbol"], json!("s_pt-turned"));
    }
}
