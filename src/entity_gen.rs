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
        BoxType::ItemBox    => "HURT_BOX",  // no item box type in FM, treat as hurtbox
        BoxType::ShieldBox  => "REFLECT_BOX",
        BoxType::ReflectBox => "REFLECT_BOX",
        BoxType::AbsorbBox  => "COUNTER_BOX",
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

pub fn generate_entity(
    data: &CharacterData,
    char_id: &str,
    sprite_boxes: &BTreeMap<String, AnimationBoxData>,
    img_result: &ImageExtractionResult,
) -> String {
    let mut keyframes: Vec<Value> = Vec::new();
    let mut layers: Vec<Value> = Vec::new();
    let mut symbols: Vec<Value> = Vec::new();
    let mut animations: Vec<Value> = Vec::new();

    // ── Build image asset GUID map (symbol_name → meta GUID) ──────────────────
    // Each image gets a deterministic GUID for its .meta file
    let mut image_guids: BTreeMap<String, String> = BTreeMap::new();
    for (_, img) in &img_result.images {
        let meta_guid = uuid(char_id, &format!("meta_{}", img.symbol_name));
        image_guids.insert(img.symbol_name.clone(), meta_guid);
    }

    // IMAGE symbols are now created per-placement (per anim/frame) to carry correct
    // world-space x/y/scaleX/scaleY from the root MC transform + local sub-sprite matrix.
    // We collect them during the animation loop below.
    // (The old per-bitmap shared symbol approach is replaced.)

    // ── Build frame script lookup ─────────────────────────────────────────────
    let _frame_script_map: BTreeMap<String, String> = data.scripts.iter()
        .filter(|s| !s.is_ext_method)
        .map(|s| (s.name.clone(), s.code.clone()))
        .collect();

    // ── Apply animation splits ────────────────────────────────────────────────
    // The splitter expands multi-label SSF2 animations into separate FM animations.
    // Each SplitAnim carries (fm_name, source_anim, start_frame, end_frame, loop info).
    let split_anims = crate::anim_splitter::split_animations(&data.animations, sprite_boxes);

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
        let frame_count = split_len.max(1);
        let anim_id = uuid(char_id, &format!("anim_{}", anim_name));
        let mut anim_layer_ids: Vec<String> = Vec::new();

        // ── 1. LABEL layer ────────────────────────────────────────────────────
        // Frame 0 always gets the FM animation name as a label.
        // Additional SSF2 inner sprite labels are placed at their original frame offsets.
        {
            let layer_id = uuid(char_id, &format!("layer_label_{}", anim_name));
            let mut label_kf_ids: Vec<Value> = Vec::new();

            // Use pre-sliced labels from the splitter (already rebased to this split's frame 0)
            // Frame 0 always gets the FM animation name; splitter labels augment from there.
            let mut all_labels: Vec<(u16, String)> = vec![(0, anim_name.to_string())];
            for (lbl, frame) in &split.labels {
                if *frame == 0 { continue; } // don't overwrite the fm_name at frame 0
                all_labels.push((*frame, lbl.clone()));
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
        // Each script gets a 1-frame keyframe. Frames without code get blank
        // keyframes (symbol: null). This matches FrayTools' expectation that
        // script keyframes fire once on their start frame only.
        {
            let layer_id = uuid(char_id, &format!("layer_script_{}", anim_name));

            // Find the SSF2 name: check source_anim first, then fm_name
            let ssf2_name = data.ssf2_to_fm_anim.iter()
                .find(|(_, fm)| fm.as_str() == source_anim.as_str())
                .or_else(|| data.ssf2_to_fm_anim.iter().find(|(_, fm)| fm.as_str() == anim_name.as_str()))
                .map(|(ssf2, _)| ssf2.clone());

            // Frame offset = sprite base offset + split start frame
            let sprite_frame_offset = sprite_boxes.get(source_anim.as_str())
                .map(|sb| sb.sprite_frame_offset as u32)
                .unwrap_or(0);
            let split_start = split.start_frame as u32;
            let frame_offset = sprite_frame_offset + split_start;

            // Collect per-frame scripts within the split's frame range
            let mut frame_code: BTreeMap<u32, String> = BTreeMap::new();
            if let Some(ref ssf2) = ssf2_name {
                let prefix = format!("{}__frame", ssf2);
                for script in &data.scripts {
                    if script.is_ext_method { continue; }
                    if let Some(rest) = script.name.strip_prefix(&prefix) {
                        if let Ok(global_frame) = rest.parse::<u32>() {
                            if global_frame >= frame_offset {
                                let local_frame = global_frame - frame_offset;
                                if local_frame < frame_count {
                                    let body = extract_function_body(&script.code);
                                    frame_code.insert(local_frame, body);
                                }
                            }
                        }
                    }
                }
            }

            // Build keyframe sequence: for each frame with code, emit a 1-frame
            // script keyframe. Fill gaps with blank keyframes. After the last
            // script frame, fill remaining frames with a blank keyframe.
            let mut script_kf_ids: Vec<String> = Vec::new();
            let mut cursor: u32 = 0;

            let script_frames: Vec<u32> = frame_code.keys().copied().collect();
            for &sf in &script_frames {
                // Gap before this script frame
                if sf > cursor {
                    let gap_kf_id = uuid(char_id, &format!("kf_script_gap_{}_{}", anim_name, cursor));
                    keyframes.push(json!({
                        "$id": gap_kf_id,
                        "type": "FRAME_SCRIPT",
                        "length": sf - cursor,
                        "code": "",
                        "pluginMetadata": {}
                    }));
                    script_kf_ids.push(gap_kf_id);
                }
                // Script keyframe (1 frame)
                let kf_id = uuid(char_id, &format!("kf_script_{}_f{}", anim_name, sf));
                keyframes.push(json!({
                    "$id": kf_id,
                    "type": "FRAME_SCRIPT",
                    "length": 1,
                    "code": frame_code[&sf],
                    "pluginMetadata": {}
                }));
                script_kf_ids.push(kf_id);
                cursor = sf + 1;
            }
            // Trailing blank to fill remaining frames
            if cursor < frame_count {
                let tail_kf_id = uuid(char_id, &format!("kf_script_tail_{}", anim_name));
                keyframes.push(json!({
                    "$id": tail_kf_id,
                    "type": "FRAME_SCRIPT",
                    "length": frame_count - cursor,
                    "code": "",
                    "pluginMetadata": {}
                }));
                script_kf_ids.push(tail_kf_id);
            }

            // If no scripts at all, still need one blank keyframe spanning the animation
            if script_kf_ids.is_empty() {
                let kf_id = uuid(char_id, &format!("kf_script_empty_{}", anim_name));
                keyframes.push(json!({
                    "$id": kf_id,
                    "type": "FRAME_SCRIPT",
                    "length": frame_count,
                    "code": "",
                    "pluginMetadata": {}
                }));
                script_kf_ids.push(kf_id);
            }

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

        // ── 3. COLLISION_BODY layer ───────────────────────────────────────────
        {
            let layer_id = uuid(char_id, &format!("layer_body_{}", anim_name));
            let kf_id = uuid(char_id, &format!("kf_body_{}", anim_name));
            let sym_id = uuid(char_id, &format!("sym_body_{}", anim_name));

            // Create a COLLISION_BODY symbol
            symbols.push(json!({
                "$id": sym_id,
                "alpha": Value::Null,
                "color": Value::Null,
                "foot": 0,
                "head": 86,
                "hipWidth": 40,
                "hipXOffset": 0,
                "hipYOffset": 0,
                "pluginMetadata": {},
                "type": "COLLISION_BODY",
                "x": 0
            }));

            keyframes.push(json!({
                "$id": kf_id,
                "type": "COLLISION_BODY",
                "length": frame_count,
                "symbol": sym_id,
                "tweened": false,
                "tweenType": "LINEAR",
                "pluginMetadata": {}
            }));
            layers.push(json!({
                "$id": layer_id,
                "name": "Body",
                "type": "COLLISION_BODY",
                "keyframes": [kf_id],
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
        if let Some(anim_box_data) = sprite_boxes.get(source_anim.as_str()) {
            let split_start_f = split.start_frame;
            let split_end_f   = if split.end_frame == u16::MAX { anim_box_data.total_frames } else { split.end_frame };
            let mut instances_in_anim: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for (f, boxes) in &anim_box_data.frames {
                if *f >= split_start_f && *f < split_end_f {
                    for b in boxes {
                        instances_in_anim.insert(b.instance_name.clone());
                    }
                }
            }

            for inst_name in instances_in_anim.iter() {
                let box_type = BoxType::from_instance_name(inst_name).unwrap_or(BoxType::Hurtbox);
                let fm_box_type = box_type_to_fm(box_type);
                let color = box_color(box_type);
                // Convert SSF2 instance name to FM layer name (hitBox→hurtbox0, attackBox→hitbox0)
                let fm_layer_name = ssf2_box_name_to_fm(inst_name);
                let box_idx = fm_box_index(&fm_layer_name);
                let layer_id = uuid(char_id, &format!("layer_box_{}_{}", anim_name, inst_name));

                let mut box_kf_ids: Vec<String> = Vec::new();
                let total = frame_count; // use split length, not source total
                let mut frame_idx: u32 = 0;

                // Collect sorted frames within split range, rebased to 0
                let mut active_frames: Vec<(u16, &crate::sprite_parser::FrameBox)> = Vec::new();
                for (f, boxes) in &anim_box_data.frames {
                    if *f >= split_start_f && *f < split_end_f {
                        if let Some(b) = boxes.iter().find(|b| b.instance_name == *inst_name) {
                            active_frames.push((f - split_start_f, b));
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
                        let gap_kf_id = uuid(char_id, &format!("kf_box_gap_{}_{}_{}", anim_name, inst_name, frame_idx));
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

                    let sym_id = uuid(char_id, &format!("sym_box_{}_{}_{}", anim_name, inst_name, start_frame));

                    if is_point {
                        // POINT symbol: bottom-center of the touchBox.
                        // In SSF2, touchBox marks the hold region; the grab hold
                        // position is at the bottom-center (where the opponent's feet anchor).
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
                        // COLLISION_BOX symbol
                        // itemBox rotates around the hand attachment point (bottom-center).
                        // All other boxes rotate around their center.
                        let (pivot_x, pivot_y) = if box_type == crate::sprite_parser::BoxType::ItemBox {
                            (round2(fb.width / 2.0), round2(fb.height))  // bottom-center = hand
                        } else {
                            (round2(fb.width / 2.0), round2(fb.height / 2.0))  // center
                        };
                        symbols.push(json!({
                            "$id": sym_id,
                            "alpha": 0.5,
                            "color": color,
                            "pivotX": pivot_x,
                            "pivotY": pivot_y,
                            "pluginMetadata": {},
                            // FrayTools uses CCW-positive; SWF atan2(b,a) is CW-positive in y-down.
                            "rotation": round2(-fb.rotation),
                            "scaleX": round2(fb.width),
                            "scaleY": round2(fb.height),
                            "type": "COLLISION_BOX",
                            "x": round2(fb.x),
                            "y": round2(fb.y)
                        }));
                    }

                    let kf_id = uuid(char_id, &format!("kf_box_{}_{}_{}", anim_name, inst_name, start_frame));
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
                    let tail_kf_id = uuid(char_id, &format!("kf_box_tail_{}_{}_{}", anim_name, inst_name, frame_idx));
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
            let num_slots = img_result.anim_images.get(source_anim.as_str())
                .map(|a| a.max_depth_slots.max(1))
                .unwrap_or(1);
            let img_split_start = split.start_frame;

            for slot in 0..num_slots {
                let img_layer_id = uuid(char_id, &format!("layer_image_{}_{}", anim_name, slot));
                let mut img_kf_ids: Vec<String> = Vec::new();

                if let Some(anim_imgs) = img_result.anim_images.get(source_anim.as_str()) {
                    let total = frame_count;
                    let mut f: u32 = 0;
                    while f < total {
                        // Offset into source animation frame table
                        let src_f = f + img_split_start as u32;
                        let entry = anim_imgs.frames.get(&(src_f as u16))
                            .and_then(|v| v.get(slot));

                        // Key for run-length: same symbol AND same world transform
                        let sym_name = entry.map(|e| e.symbol_name.as_str());
                        let shape_id = entry.map(|e| e.shape_id);
                        let world_tx  = entry.map(|e| round2(e.world_tx)).unwrap_or(0.0);
                        let world_ty  = entry.map(|e| round2(e.world_ty)).unwrap_or(0.0);
                        let world_sx  = entry.map(|e| round2(e.world_sx)).unwrap_or(1.0);
                        let world_sy  = entry.map(|e| round2(e.world_sy)).unwrap_or(1.0);
                        let world_rot = entry.map(|e| round2(e.world_rotation)).unwrap_or(0.0);

                        // Run-length encode consecutive frames with identical symbol + world transform
                        let mut run = 1u32;
                        while f + run < total {
                            let next = anim_imgs.frames.get(&((src_f + run) as u16))
                                .and_then(|v| v.get(slot));
                            let matches = next.map(|e| e.symbol_name.as_str()) == sym_name
                                && next.map(|e| round2(e.world_tx))       == Some(world_tx).filter(|_| sym_name.is_some())
                                && next.map(|e| round2(e.world_ty))       == Some(world_ty).filter(|_| sym_name.is_some())
                                && next.map(|e| round2(e.world_sx))       == Some(world_sx).filter(|_| sym_name.is_some())
                                && next.map(|e| round2(e.world_sy))       == Some(world_sy).filter(|_| sym_name.is_some())
                                && next.map(|e| round2(e.world_rotation)) == Some(world_rot).filter(|_| sym_name.is_some());
                            if matches { run += 1; } else { break; }
                        }

                        let kf_id = uuid(char_id, &format!("kf_image_{}_s{}_f{}", anim_name, slot, f));

                        // Resolve bitmap and create a per-placement IMAGE symbol with world coords
                        let bitmap_img = shape_id.and_then(|sid| {
                            let bmp_id = img_result.shape_to_bitmap.get(&sid).copied().unwrap_or(sid);
                            img_result.images.get(&bmp_id)
                        }).or_else(|| {
                            sym_name.and_then(|sn| {
                                img_result.images.values().find(|img| img.symbol_name == sn)
                            })
                        });

                        let symbol_ref = if let Some(img) = bitmap_img {
                            // Create a per-placement symbol (unique per anim/slot/frame)
                            let per_placement_sym_id = uuid(char_id,
                                &format!("sym_img_{}_s{}_f{}", anim_name, slot, f));
                            let meta_guid = image_guids.get(&img.symbol_name)
                                .cloned().unwrap_or_default();

                            // FrayTools coordinate model:
                            // (x, y) = position of the pivot point in entity space
                            // (pivotX, pivotY) = offset from image top-left to rotation center
                            // rotation happens around pivot
                            //
                            // SWF model: [a,b,c,d,tx,ty] is a full affine transform.
                            // tx,ty positions the bitmap origin (top-left), and the matrix
                            // rotates/scales around that origin.  To make rotation look correct
                            // in Fraymakers, we use the bitmap center as the pivot.  Then x,y
                            // becomes the world-space position of that center:
                            //   x = world_tx + a*(w/2) + c*(h/2)
                            //   y = world_ty + b*(w/2) + d*(h/2)
                            let img_w = img.width as f64;
                            let img_h = img.height as f64;
                            let fm_sx = round2(world_sx.abs());
                            let fm_sy = round2(world_sy.abs());

                            // Get world-space matrix components
                            let wa = entry.map(|e| e.world_a).unwrap_or(1.0);
                            let wb = entry.map(|e| e.world_b).unwrap_or(0.0);
                            let wc = entry.map(|e| e.world_c).unwrap_or(0.0);
                            let wd = entry.map(|e| e.world_d).unwrap_or(1.0);

                            // Pivot at bitmap center
                            let pivot_x = round2(img_w / 2.0);
                            let pivot_y = round2(img_h / 2.0);

                            // FrayTools model: (x,y) is the top-left of the UNROTATED image.
                            // Rotation happens around (x+pivotX, y+pivotY).
                            // So: rotation_center = world_tx + M*(pivotX, pivotY)
                            //     x = rotation_center_x - pivotX
                            //     y = rotation_center_y - pivotY
                            let center_x = world_tx + wa * pivot_x + wc * pivot_y;
                            let center_y = world_ty + wb * pivot_x + wd * pivot_y;
                            let fm_x = round2(center_x - pivot_x);
                            let fm_y = round2(center_y - pivot_y);

                            symbols.push(json!({
                                "$id": per_placement_sym_id,
                                "alpha": 1,
                                "imageAsset": meta_guid,
                                "pivotX": pivot_x,
                                "pivotY": pivot_y,
                                "pluginMetadata": {},
                                // FrayTools uses CCW-positive; SWF atan2(b,a) is CW-positive in y-down.
                                "rotation": round2(-world_rot),
                                "scaleX": fm_sx,
                                "scaleY": fm_sy,
                                "type": "IMAGE",
                                "x": fm_x,
                                "y": fm_y
                            }));
                            Some(per_placement_sym_id)
                        } else {
                            None
                        };

                        keyframes.push(json!({
                            "$id": kf_id,
                            "type": "IMAGE",
                            "length": run,
                            "symbol": symbol_ref.map(Value::String).unwrap_or(Value::Null),
                            "tweened": false,
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
                anim_layer_ids.push(img_layer_id);
            }
        }

        animations.push(json!({
            "$id": anim_id,
            "name": anim_name,
            "layers": anim_layer_ids,
            "pluginMetadata": {}
        }));
    }

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

    serde_json::to_string_pretty(&entity).unwrap_or_else(|_| "{}".to_string())
}

/// Generate entity with paletteMap filled in
pub fn generate_entity_with_palette(
    data: &CharacterData,
    char_id: &str,
    sprite_boxes: &BTreeMap<String, AnimationBoxData>,
    img_result: &ImageExtractionResult,
    palette_collection_guid: &str,
    palette_map_id: &str,
) -> String {
    let json_str = generate_entity(data, char_id, sprite_boxes, img_result);
    let mut entity: serde_json::Value = serde_json::from_str(&json_str).unwrap_or(serde_json::json!({}));
    entity["paletteMap"] = serde_json::json!({
        "paletteCollection": palette_collection_guid,
        "paletteMap": palette_map_id
    });
    serde_json::to_string_pretty(&entity).unwrap_or(json_str)
}

/// Get image GUIDs for .meta file generation
pub fn get_image_meta_guids(
    char_id: &str,
    img_result: &ImageExtractionResult,
) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for (_, img) in &img_result.images {
        let meta_guid = uuid(char_id, &format!("meta_{}", img.symbol_name));
        result.insert(img.png_path.clone(), meta_guid);
    }
    result
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Strip the `function name() {` wrapper from a script and return just the body,
/// with one level of leading tab removed from each line.
fn extract_function_body(code: &str) -> String {
    let mut lines: Vec<&str> = code.lines().collect();
    // Drop leading blank lines
    while lines.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
        lines.remove(0);
    }
    // Drop trailing blank lines
    while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        lines.pop();
    }
    if lines.is_empty() {
        return String::new();
    }
    // First line should be `function name() {` — drop it
    if lines.first().map(|l| l.trim_start().starts_with("function ")).unwrap_or(false) {
        lines.remove(0);
    }
    // Last line should be `}` — drop it
    if lines.last().map(|l| l.trim() == "}").unwrap_or(false) {
        lines.pop();
    }
    // De-indent by one tab
    let body: Vec<&str> = lines.iter()
        .map(|l| l.strip_prefix('\t').unwrap_or(l))
        .collect();
    body.join("\n")
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

    // Helper functions return (sym, kf, layer) tuples; caller pushes them.
    fn make_image_items(char_id: &str, head_guid: &str, pivot_x: f64, pivot_y: f64,
                        anim_name: &str, suffix: &str, x: f64, y: f64)
        -> (Value, Value, Value, String) {
        let sym_id = uuid(char_id, &format!("menu_sym_{}{}", anim_name, suffix));
        let kf_id = uuid(char_id, &format!("menu_kf_{}{}", anim_name, suffix));
        let layer_id = uuid(char_id, &format!("menu_layer_{}{}", anim_name, suffix));
        let sym = json!({ "$id": sym_id, "alpha": 1, "imageAsset": head_guid,
            "pivotX": pivot_x, "pivotY": pivot_y, "pluginMetadata": {},
            "rotation": 0, "scaleX": 1, "scaleY": 1, "type": "IMAGE",
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
            let (s, k, l, lid) = make_image_items(char_id, head_guid, pivot_x, pivot_y, $anim, $suf, $x, $y);
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
        let img = push_img!("icon", "", 0.0, 0.0);
        let aid = push_icon_aid!("icon");
        let script = push_script!("icon");
        animations.push(json!({ "$id": uuid(char_id, "menu_anim_icon"),
            "name": "icon", "layers": [img, aid, script], "pluginMetadata": {} }));
    }

    // ── icon_no_palette ──────────────────────────────────────────────────
    {
        let img = push_img!("icon_no_palette", "", 0.0, 0.0);
        let script = push_script!("icon_no_palette");
        let aid = push_icon_aid!("icon_no_palette");
        animations.push(json!({ "$id": uuid(char_id, "menu_anim_icon_no_palette"),
            "name": "icon_no_palette", "layers": [img, script, aid], "pluginMetadata": {} }));
    }

    // ── stock ────────────────────────────────────────────────────────────
    {
        let aid = push_hud_aid!("stock", 24.0);
        let img = push_img!("stock", "", -pivot_x, -pivot_y);
        animations.push(json!({ "$id": uuid(char_id, "menu_anim_stock"),
            "name": "stock", "layers": [aid, img], "pluginMetadata": {} }));
    }

    // ── hud variants ────────────────────────────────────────────────────
    for hud_name in &["hud", "hud_front", "hud_angry", "hud_angry_front",
                      "hud_happy", "hud_happy_front", "hud_hurt", "hud_hurt_front",
                      "hud_sad", "hud_sad_front"] {
        let img = push_img!(hud_name, "", -pivot_x, -pivot_y);
        let aid = push_hud_aid!(hud_name, 36.0);
        animations.push(json!({ "$id": uuid(char_id, &format!("menu_anim_{}", hud_name)),
            "name": *hud_name, "layers": [img, aid], "pluginMetadata": {} }));
    }

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

    serde_json::to_string_pretty(&entity).unwrap_or_else(|_| "{}" .to_string())
}

// ─── Projectile entity generation ─────────────────────────────────────────────

/// Information about a projectile extracted from the SWF.
#[derive(Debug, Clone)]
pub struct ProjectileInfo {
    /// The projectile's symbol name (e.g. "mario_fireball")
    pub name: String,
    /// The inner animation sprite name (e.g. "mario_fla.mario_fireball_mc_210")
    pub inner_sprite_name: Option<String>,
    /// Frame count of the inner animation sprite
    pub inner_frame_count: u16,
    /// Collision boxes from the inner sprite, if any
    pub boxes: Option<AnimationBoxData>,
    /// Image frames from the inner sprite (symbol name per frame)
    pub image_frames: Vec<Option<String>>,
    /// Meta GUIDs for each unique image used
    pub image_guids: BTreeMap<String, String>,
    /// Extra per-state data for multi-state projectiles (e.g. link_bomb).
    /// Each entry is (ssf2_label, image_frames, image_guids, boxes, frame_count).
    pub extra_states: Vec<ProjectileStateData>,
}

/// Image + box data for one state of a multi-state projectile.
#[derive(Debug, Clone)]
pub struct ProjectileStateData {
    pub label: String,
    pub image_frames: Vec<Option<String>>,
    pub image_guids: BTreeMap<String, String>,
    pub boxes: Option<AnimationBoxData>,
    pub frame_count: u16,
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

pub fn generate_projectile_entity(
    char_id: &str,
    proj: &ProjectileInfo,
) -> String {
    let mut keyframes: Vec<Value> = Vec::new();
    let mut layers: Vec<Value> = Vec::new();
    let mut symbols: Vec<Value> = Vec::new();
    let mut animations: Vec<Value> = Vec::new();

    let proj_id = &proj.name;
    let total_frames = proj.inner_frame_count.max(1) as u32;

    // ── Build image keyframes for idle animation ─────────────────────────
    let mut idle_image_kf_ids: Vec<String> = Vec::new();
    for frame in 0..total_frames {
        let sym_name = proj.image_frames.get(frame as usize).and_then(|s| s.as_ref());
        let kf_id = uuid(char_id, &format!("proj_{}_idle_img_kf_{}", proj_id, frame));
        if let Some(sym_name) = sym_name {
            if let Some(guid) = proj.image_guids.get(sym_name.as_str()) {
                let sym_id = uuid(char_id, &format!("proj_{}_idle_img_sym_{}", proj_id, frame));
                symbols.push(json!({
                    "$id": sym_id,
                    "alpha": 1,
                    "imageAsset": guid,
                    "pivotX": 0,
                    "pivotY": 0,
                    "pluginMetadata": {},
                    "rotation": 0,
                    "scaleX": 1,
                    "scaleY": 1,
                    "type": "IMAGE",
                    "x": 0,
                    "y": 0
                }));
                keyframes.push(json!({
                    "$id": kf_id,
                    "length": 1,
                    "pluginMetadata": {},
                    "symbol": sym_id,
                    "tweenType": "LINEAR",
                    "tweened": false,
                    "type": "IMAGE"
                }));
            } else {
                keyframes.push(json!({
                    "$id": kf_id,
                    "length": 1,
                    "pluginMetadata": {},
                    "symbol": Value::Null,
                    "tweenType": "LINEAR",
                    "tweened": false,
                    "type": "IMAGE"
                }));
            }
        } else {
            keyframes.push(json!({
                "$id": kf_id,
                "length": 1,
                "pluginMetadata": {},
                "symbol": Value::Null,
                "tweenType": "LINEAR",
                "tweened": false,
                "type": "IMAGE"
            }));
        }
        idle_image_kf_ids.push(kf_id);
    }

    let idle_image_layer_id = uuid(char_id, &format!("proj_{}_idle_img_layer", proj_id));
    layers.push(json!({
        "$id": idle_image_layer_id,
        "name": "Image Layer",
        "type": "IMAGE",
        "keyframes": idle_image_kf_ids,
        "hidden": false,
        "locked": false,
        "pluginMetadata": {}
    }));

    // ── Build collision box layers for idle (from inner sprite boxes) ────
    let mut idle_box_layer_ids: Vec<String> = Vec::new();
    if let Some(boxes) = &proj.boxes {
        // Collect all unique instance names across frames
        let mut instances_in_proj: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for frame_boxes in boxes.frames.values() {
            for b in frame_boxes {
                instances_in_proj.insert(b.instance_name.clone());
            }
        }

        for inst_name in instances_in_proj.iter() {
            let box_type = BoxType::from_instance_name(inst_name).unwrap_or(BoxType::Hurtbox);
            let fm_layer_name = ssf2_box_name_to_fm(inst_name);
            let fm_box_type = box_type_to_fm(box_type);
            let color = box_color(box_type);
            let box_idx = fm_box_index(&fm_layer_name);

            let mut box_kf_ids: Vec<String> = Vec::new();
            let mut frame_idx: u32 = 0;

            // Collect active frames for this instance
            let mut active_frames: Vec<(usize, &crate::sprite_parser::FrameBox)> = Vec::new();
            for f in 0..total_frames {
                if let Some(frame_boxes) = boxes.frames.get(&(f as u16)) {
                    if let Some(fb) = frame_boxes.iter().find(|b| &b.instance_name == inst_name) {
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
                    let gap_kf_id = uuid(char_id, &format!("proj_{}_box_gap_{}_{}", proj_id, inst_name, frame_idx));
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
                let sym_id = uuid(char_id, &format!("proj_{}_box_sym_{}_{}", proj_id, inst_name, start_frame));
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
                    let (piv_x, piv_y) = (round2(fb.width / 2.0), round2(fb.height / 2.0));
                    symbols.push(json!({
                        "$id": sym_id,
                        "alpha": 0.5,
                        "color": color,
                        "pivotX": piv_x,
                        "pivotY": piv_y,
                        "pluginMetadata": {},
                        "rotation": round2(-fb.rotation),
                        "scaleX": round2(fb.width),
                        "scaleY": round2(fb.height),
                        "type": "COLLISION_BOX",
                        "x": round2(fb.x),
                        "y": round2(fb.y)
                    }));
                }

                let kf_id = uuid(char_id, &format!("proj_{}_box_kf_{}_{}", proj_id, inst_name, start_frame));
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
                let tail_kf_id = uuid(char_id, &format!("proj_{}_box_tail_{}_{}", proj_id, inst_name, frame_idx));
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

            let layer_id = uuid(char_id, &format!("proj_{}_box_layer_{}", proj_id, inst_name));
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

    // ── projectileSpawn animation (1 frame, first image) ─────────────────
    {
        let (_, _, spawn_img) = if !proj.image_frames.is_empty() && proj.image_frames[0].is_some() {
            let sym_name = proj.image_frames[0].as_ref().unwrap();
            let guid = proj.image_guids.get(sym_name.as_str()).cloned().unwrap_or_default();
            let sym_id = uuid(char_id, &format!("proj_{}_spawn_sym", proj_id));
            symbols.push(json!({
                "$id": sym_id,
                "alpha": 1,
                "imageAsset": guid,
                "pivotX": 0,
                "pivotY": 0,
                "pluginMetadata": {},
                "rotation": 0,
                "scaleX": 1,
                "scaleY": 1,
                "type": "IMAGE",
                "x": 0,
                "y": 0
            }));
            let kf_id = uuid(char_id, &format!("proj_{}_spawn_kf", proj_id));
            keyframes.push(json!({
                "$id": kf_id,
                "length": 1,
                "pluginMetadata": {},
                "symbol": sym_id,
                "tweenType": "LINEAR",
                "tweened": false,
                "type": "IMAGE"
            }));
            let layer_id = uuid(char_id, &format!("proj_{}_spawn_layer", proj_id));
            layers.push(json!({
                "$id": layer_id,
                "name": "Image Layer",
                "type": "IMAGE",
                "keyframes": [kf_id],
                "hidden": false,
                "locked": false,
                "pluginMetadata": {}
            }));
            (sym_id, kf_id, layer_id)
        } else {
            // Blank spawn frame
            let kf_id = uuid(char_id, &format!("proj_{}_spawn_kf", proj_id));
            keyframes.push(json!({
                "$id": kf_id,
                "length": 1,
                "pluginMetadata": {},
                "symbol": Value::Null,
                "tweenType": "LINEAR",
                "tweened": false,
                "type": "IMAGE"
            }));
            let layer_id = uuid(char_id, &format!("proj_{}_spawn_layer", proj_id));
            layers.push(json!({
                "$id": layer_id,
                "name": "Image Layer",
                "type": "IMAGE",
                "keyframes": [kf_id],
                "hidden": false,
                "locked": false,
                "pluginMetadata": {}
            }));
            (String::new(), kf_id, layer_id)
        };
        animations.push(json!({
            "$id": uuid(char_id, &format!("proj_{}_anim_spawn", proj_id)),
            "name": "projectileSpawn",
            "layers": [spawn_img],
            "pluginMetadata": {}
        }));
    }

    // ── projectileIdle animation (all frames + boxes) ────────────────────
    {
        let mut idle_layers = vec![idle_image_layer_id.clone()];
        idle_layers.extend(idle_box_layer_ids);
        animations.push(json!({
            "$id": uuid(char_id, &format!("proj_{}_anim_idle", proj_id)),
            "name": "projectileIdle",
            "layers": idle_layers,
            "pluginMetadata": {}
        }));
    }

    // ── projectileDestroy animation (last frame + blank script) ──────────
    {
        let last_frame = total_frames.saturating_sub(1) as usize;
        let sym_name = proj.image_frames.get(last_frame).and_then(|s| s.as_ref());
        let destroy_kf_id = uuid(char_id, &format!("proj_{}_destroy_kf", proj_id));
        if let Some(sym_name) = sym_name {
            if let Some(guid) = proj.image_guids.get(sym_name.as_str()) {
                let sym_id = uuid(char_id, &format!("proj_{}_destroy_sym", proj_id));
                symbols.push(json!({
                    "$id": sym_id,
                    "alpha": 1,
                    "imageAsset": guid,
                    "pivotX": 0,
                    "pivotY": 0,
                    "pluginMetadata": {},
                    "rotation": 0,
                    "scaleX": 1,
                    "scaleY": 1,
                    "type": "IMAGE",
                    "x": 0,
                    "y": 0
                }));
                keyframes.push(json!({
                    "$id": destroy_kf_id,
                    "length": 1,
                    "pluginMetadata": {},
                    "symbol": sym_id,
                    "tweenType": "LINEAR",
                    "tweened": false,
                    "type": "IMAGE"
                }));
            } else {
                keyframes.push(json!({
                    "$id": destroy_kf_id,
                    "length": 1,
                    "pluginMetadata": {},
                    "symbol": Value::Null,
                    "tweenType": "LINEAR",
                    "tweened": false,
                    "type": "IMAGE"
                }));
            }
        } else {
            keyframes.push(json!({
                "$id": destroy_kf_id,
                "length": 1,
                "pluginMetadata": {},
                "symbol": Value::Null,
                "tweenType": "LINEAR",
                "tweened": false,
                "type": "IMAGE"
            }));
        }
        let destroy_img_layer = uuid(char_id, &format!("proj_{}_destroy_img_layer", proj_id));
        layers.push(json!({
            "$id": destroy_img_layer,
            "name": "Image Layer",
            "type": "IMAGE",
            "keyframes": [destroy_kf_id],
            "hidden": false,
            "locked": false,
            "pluginMetadata": {}
        }));

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

        animations.push(json!({
            "$id": uuid(char_id, &format!("proj_{}_anim_destroy", proj_id)),
            "name": "projectileDestroy",
            "layers": [destroy_img_layer, destroy_script_layer],
            "pluginMetadata": {}
        }));
    }

    // ── Extra animations for multi-state projectiles (e.g. link_bomb) ────────
    for state_data in &proj.extra_states {
        let state_slug = state_data.label.replace(|c: char| !c.is_alphanumeric(), "_");
        let anim_name = ssf2_proj_label_to_fm_anim(&state_data.label);

        // Image layer
        let img_layer_id = uuid(char_id, &format!("proj_{}_extra_{}_img_layer", proj_id, state_slug));
        let mut img_kf_ids: Vec<Value> = Vec::new();
        let total = state_data.frame_count as usize;
        for (fi, sym_opt) in state_data.image_frames.iter().enumerate().take(total) {
            let kf_id = uuid(char_id, &format!("proj_{}_extra_{}_img_kf_{}", proj_id, state_slug, fi));
            if let Some(sym_name) = sym_opt {
                if let Some(guid) = state_data.image_guids.get(sym_name.as_str()) {
                    let sym_id = uuid(char_id, &format!("proj_{}_extra_{}_sym_{}", proj_id, state_slug, fi));
                    symbols.push(json!({
                        "$id": sym_id, "alpha": 1, "imageAsset": guid,
                        "pivotX": 0, "pivotY": 0, "pluginMetadata": {},
                        "rotation": 0, "scaleX": 1, "scaleY": 1, "type": "IMAGE", "x": 0, "y": 0
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
        if img_kf_ids.is_empty() {
            let kf_id = uuid(char_id, &format!("proj_{}_extra_{}_img_empty", proj_id, state_slug));
            keyframes.push(json!({
                "$id": kf_id, "length": total.max(1) as u32,
                "pluginMetadata": {}, "symbol": Value::Null,
                "tweenType": "LINEAR", "tweened": false, "type": "IMAGE"
            }));
            img_kf_ids.push(kf_id.into());
        }
        layers.push(json!({
            "$id": img_layer_id, "name": "Image Layer", "type": "IMAGE",
            "keyframes": img_kf_ids, "hidden": false, "locked": false, "pluginMetadata": {}
        }));

        // Script layer (blank)
        let script_kf_id = uuid(char_id, &format!("proj_{}_extra_{}_script_kf", proj_id, state_slug));
        let script_layer_id = uuid(char_id, &format!("proj_{}_extra_{}_script_layer", proj_id, state_slug));
        keyframes.push(json!({
            "$id": script_kf_id, "length": total.max(1) as u32,
            "code": "", "pluginMetadata": {}, "symbol": Value::Null,
            "tweenType": "LINEAR", "tweened": false, "type": "FRAME_SCRIPT"
        }));
        layers.push(json!({
            "$id": script_layer_id, "name": "Frame Script Layer", "type": "FRAME_SCRIPT",
            "keyframes": [script_kf_id], "hidden": false, "locked": false, "pluginMetadata": {}
        }));

        animations.push(json!({
            "$id": uuid(char_id, &format!("proj_{}_extra_{}_anim", proj_id, state_slug)),
            "name": anim_name,
            "layers": [img_layer_id, script_layer_id],
            "pluginMetadata": {}
        }));
    }

    let entity_id = format!("{}Projectile", proj.name.replace('_', ""));
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

    serde_json::to_string_pretty(&entity).unwrap_or_else(|_| "{}".to_string())
}
