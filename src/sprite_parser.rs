/// sprite_parser — extracts per-animation, per-frame collision box geometry from SWF DefineSprite tags.
///
/// SSF2 encodes hitbox data entirely in the SWF timeline, not in AS3 code:
///   - Each animation lives in a named DefineSprite (e.g. `mario_fla.Jab_21`)
///   - Each frame of that sprite has PlaceObject tags for collision MovieClips (CollisonBox_6)
///   - The PlaceObject matrix encodes the box's position (tx/ty in twips) and size (a/d = scale)
///   - The PlaceObject instance name tells us the box type:
///       attackBox / attackBox2 / attackBox3 → Hitbox (active attack)
///       hitBox / hitBox2 / hitBox3 / hitBox4 / hitBox5 → Hurtbox (can be hit)
///       hurtBox → Hurtbox
///       grabBox / grabbox → GrabBox
///       itemBox → ItemBox (item pickup range)
///       shieldBox / shieldbox → ShieldBox
///       reflectBox → ReflectBox
///       absorbBox → AbsorbBox
///       ledgeBox / ledgegrabbox → LedgeBox
///       (anything else with "Box" suffix) → Hurtbox (fallback)
///
/// The base CollisonBox_6 shape is typically a 100×100 unit square centered at 0,0.
/// scale_x * BASE_SIZE = box width, scale_y * BASE_SIZE = box height.
/// tx/ty give the top-left (or center, depending on registration point) in pixels.

use std::collections::BTreeMap;
use std::io::Cursor;
use serde::{Deserialize, Serialize};

/// Base size of the CollisonBox shape in SSF2 (pixels after /20 twip conversion).
/// Measured from the actual shape bounds of the CollisonBox_6 DefineShape.
/// Fallback to 100.0 if shape not found.
const DEFAULT_BASE_SIZE: f64 = 100.0;

// ─── Output types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoxType {
    /// Active attack hitbox (dealg damage)
    Hitbox,
    /// Hurtbox (can receive damage)
    Hurtbox,
    /// Grab range
    GrabBox,
    /// Item pickup range
    ItemBox,
    /// Shield/parry box
    ShieldBox,
    /// Reflector hitbox
    ReflectBox,
    /// Absorb hitbox
    AbsorbBox,
    /// Ledge grab box
    LedgeBox,
    /// Grab hold point (where grabbed opponent is held)
    GrabHoldBox,
}

impl BoxType {
    /// Map SSF2 instance name → BoxType
    pub fn from_instance_name(name: &str) -> Option<BoxType> {
        let lower = name.to_lowercase();
        if lower.starts_with("attackbox") {
            Some(BoxType::Hitbox)
        } else if lower.starts_with("hitbox") || lower.starts_with("hurtbox") {
            Some(BoxType::Hurtbox)
        } else if lower.starts_with("grabbox") || lower.starts_with("grab") && lower.ends_with("box") {
            Some(BoxType::GrabBox)
        } else if lower.starts_with("itembox") || lower == "itembox" {
            Some(BoxType::ItemBox)
        } else if lower.starts_with("shieldbox") {
            Some(BoxType::ShieldBox)
        } else if lower.starts_with("reflectbox") {
            Some(BoxType::ReflectBox)
        } else if lower.starts_with("absorbbox") {
            Some(BoxType::AbsorbBox)
        } else if lower.starts_with("ledgebox") || lower.starts_with("ledgegrab") {
            Some(BoxType::LedgeBox)
        } else if lower.starts_with("touchbox") {
            Some(BoxType::GrabHoldBox)
        } else if lower.ends_with("box") {
            // generic fallback — treat unknown *box as hurtbox
            Some(BoxType::Hurtbox)
        } else {
            None
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            BoxType::Hitbox    => "HITBOX",
            BoxType::Hurtbox   => "HURTBOX",
            BoxType::GrabBox   => "GRAB_BOX",
            BoxType::ItemBox   => "ITEM_BOX",
            BoxType::ShieldBox => "SHIELD_BOX",
            BoxType::ReflectBox => "REFLECT_BOX",
            BoxType::AbsorbBox => "ABSORB_BOX",
            BoxType::LedgeBox  => "LEDGE_BOX",
            BoxType::GrabHoldBox => "GRAB_HOLD_BOX",
        }
    }
}

/// A single collision box placed on one frame of one animation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameBox {
    /// Box type derived from instance name
    pub box_type: BoxType,
    /// Original SSF2 instance name (e.g. "attackBox", "hitBox3")
    pub instance_name: String,
    /// X position in pixels (top-left, relative to character origin)
    pub x: f64,
    /// Y position in pixels (top-left, relative to character origin)
    /// Note: SWF Y axis points DOWN; negative = above foot.
    pub y: f64,
    /// Box width in pixels
    pub width: f64,
    /// Box height in pixels
    pub height: f64,
    /// Rotation in degrees (always 0 for hit/hurtboxes; itemBox can rotate)
    pub rotation: f64,
}

/// All collision boxes for one animation, indexed by frame number (0-based).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationBoxData {
    /// SSF2 animation name (e.g. "a_air", "stand")
    pub ssf2_name: String,
    /// Fraymakers animation name (e.g. "aerial_neutral", "idle")
    pub fm_name: String,
    /// Total frames in this animation (after sub-anim slicing if applicable)
    pub total_frames: u16,
    /// per-frame boxes: frame_index (0-based within this sub-anim) → list of boxes active on that frame
    pub frames: BTreeMap<u16, Vec<FrameBox>>,
    /// Frame offset into the original SSF2 sprite where this sub-anim starts
    pub sprite_frame_offset: u16,
    /// Frame labels within this animation: (label_name, frame_index relative to sub-anim start)
    pub frame_labels: Vec<(String, u16)>,
}

/// Root MovieClip placement transform for one xframe animation.
/// The root character MC places each animation sub-sprite at a specific
/// offset and scale. These must be applied to all sub-sprite-local coords
/// (boxes, image pivots) to produce correct world-space positions.
#[derive(Debug, Clone, Copy)]
pub struct XframeTransform {
    /// Root placement offset X in pixels (SSF2 coordinate space)
    pub tx: f64,
    /// Root placement offset Y in pixels (SSF2 coordinate space, y-down)
    pub ty: f64,
    /// Root placement scaleX
    pub sx: f64,
    /// Root placement scaleY
    pub sy: f64,
    /// Full affine matrix components for correct world-space composition
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
}

impl Default for XframeTransform {
    fn default() -> Self {
        Self { tx: 0.0, ty: 0.0, sx: 1.0, sy: 1.0, a: 1.0, b: 0.0, c: 0.0, d: 1.0 }
    }
}

impl XframeTransform {
    /// Apply this root transform to a local point (lx, ly), returning world coords.
    pub fn apply(&self, lx: f64, ly: f64) -> (f64, f64) {
        (self.a * lx + self.b * ly + self.tx,
         self.c * lx + self.d * ly + self.ty)
    }

    /// Scale magnitude for a local scale value (ignores rotation/skew components).
    pub fn scale_x(&self) -> f64 { self.sx.abs() }
    pub fn scale_y(&self) -> f64 { self.sy.abs() }
}

// ─── Main entry point ────────────────────────────────────────────────────────

/// Extract per-animation root-MovieClip placement transforms.
///
/// The SSF2 root character sprite (e.g. "mario") places each animation
/// sub-sprite ("stance") on successive frames with a PlaceObject that
/// carries a translation (tx, ty) and scale (sx, sy). These transform
/// all local coordinates inside the sub-sprite into the character's
/// world space (where y=0 is the foot of the character).
///
/// Returns a map: FM animation name → XframeTransform.
/// For sub-animations (jab1/jab2 etc) the parent animation's transform is used.
pub fn extract_xframe_transforms(
    swf_data: &[u8],
    char_name: &str,
    ssf2_to_fm: &BTreeMap<String, String>,
) -> anyhow::Result<BTreeMap<String, XframeTransform>> {
    let swf_buf = swf::decompress_swf(Cursor::new(swf_data))?;
    let swf = swf::parse_swf(&swf_buf)?;

    let mut sym_names: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                sym_names.insert(link.id, name);
            }
        }
    }

    let char_lower = char_name.to_lowercase();
    let mut result: BTreeMap<String, XframeTransform> = BTreeMap::new();

    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sym = sym_names.get(&sprite.id).cloned().unwrap_or_default();
            if sym.to_lowercase() != char_lower { continue; }

            // Walk the root MC, tracking the current xframe label and the last
            // stance PlaceObject matrix seen.
            let mut current_frame: u16 = 0;
            let mut current_ssf2_label: Option<String> = None;
            // depth → last known matrix for stance
            let mut stance_matrix: Option<swf::Matrix> = None;

            for stag in &sprite.tags {
                match stag {
                    swf::Tag::FrameLabel(fl) => {
                        let label = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                        current_ssf2_label = Some(label);
                        // Reset stance matrix for this new label frame
                        stance_matrix = None;
                    }
                    swf::Tag::PlaceObject(po) => {
                        let inst = po.name.as_ref()
                            .map(|s| String::from_utf8_lossy(s.as_bytes()).to_string())
                            .unwrap_or_default();
                        if inst == "stance" {
                            if let Some(m) = &po.matrix {
                                stance_matrix = Some(*m);
                            }
                        }
                    }
                    swf::Tag::ShowFrame => {
                        // On ShowFrame: record the transform for the current label
                        if let (Some(label), Some(m)) = (&current_ssf2_label, &stance_matrix) {
                            let tx = m.tx.get() as f64 / 20.0;
                            let ty = m.ty.get() as f64 / 20.0;
                            // Store full affine matrix for correct world-space composition
                            let a = m.a.to_f64();
                            let b = m.b.to_f64();
                            let c = m.c.to_f64();
                            let d = m.d.to_f64();
                            let sx = (a*a + b*b).sqrt();
                            let sy = (c*c + d*d).sqrt();

                            // Map SSF2 label → FM name
                            let fm_name = ssf2_to_fm.get(label.as_str())
                                .cloned()
                                .or_else(|| static_ssf2_to_fm(label))
                                .unwrap_or_else(|| label.clone());

                            // Also insert for sub-animations (jab1/jab2/jab3/jab4, taunt_up/taunt_down)
                            let xform = XframeTransform { tx, ty, sx, sy, a, b, c, d };
                            result.entry(fm_name.clone()).or_insert(xform);

                            // Seed sub-anim names with the same transform
                            for sub in crate::extractor::expand_split_anim(&fm_name) {
                                result.entry(sub).or_insert(xform);
                            }
                        }
                        current_frame += 1;
                        // After ShowFrame, clear label so next ShowFrame (same label re-entry) doesn't re-record
                        current_ssf2_label = None;
                    }
                    _ => {}
                }
            }
            let _ = current_frame;
            break;
        }
    }

    log::info!("extract_xframe_transforms: {} animations mapped", result.len());
    Ok(result)
}

/// Parse the SWF file and extract per-animation per-frame collision box data.
/// `ssf2_to_fm` maps SSF2 animation names to Fraymakers animation names.
pub fn parse_sprite_boxes(
    swf_data: &[u8],
    char_name: &str,
    ssf2_to_fm: &BTreeMap<String, String>,
) -> anyhow::Result<BTreeMap<String, AnimationBoxData>> {
    let swf_buf = swf::decompress_swf(Cursor::new(swf_data))?;
    let swf = swf::parse_swf(&swf_buf)?;

    // Build id → symbol name map
    let mut sym_names: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                sym_names.insert(link.id, name);
            }
        }
    }

    // Find the base size of the collision box shape
    // Look for the DefineShape that the CollisonBox sprite (id=112 for mario) references
    let box_base_size = find_collision_box_base_size(&swf, &sym_names);
    log::info!("CollisonBox base size: {:.1}px", box_base_size);

    // Build reverse map: FM anim name → SSF2 anim name (for lookup)
    // and a set of all SSF2 anim names we care about
    let _known_ssf2_names: std::collections::HashSet<&str> = ssf2_to_fm.keys().map(|s| s.as_str()).collect();

    // Extract root MovieClip placement transforms for each animation
    let xform_map = extract_xframe_transforms(swf_data, char_name, ssf2_to_fm)
        .unwrap_or_default();
    log::info!("Root MC transforms: {} animations", xform_map.len());

    // Find all character animation sprites
    // Pattern: "{char}_fla.{AnimName}_{index}" or just "{char}_{animtype}"
    let char_lower = char_name.to_lowercase();
    let mut result: BTreeMap<String, AnimationBoxData> = BTreeMap::new();

    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sym = sym_names.get(&sprite.id).cloned().unwrap_or_default();

            // Only process sprites that belong to this character
            if !sym.to_lowercase().contains(&char_lower) {
                continue;
            }

            // Try to extract SSF2 animation name from the symbol name
            let ssf2_name = match extract_ssf2_anim_name(&sym, &char_lower, ssf2_to_fm) {
                Some(n) => n,
                None => continue,
            };

            // Convert SSF2 name → FM name.
            let fm_name = ssf2_to_fm.get(&ssf2_name)
                .cloned()
                .or_else(|| static_ssf2_to_fm(&ssf2_name))
                .unwrap_or_else(|| ssf2_name.clone());

            // Look up root MC transform for this animation
            let xform = xform_map.get(&fm_name).copied().unwrap_or_default();

            // Collect internal frame labels (for sub-animation splitting)
            let frame_labels = extract_frame_labels(sprite);

            let frames = extract_frame_boxes(sprite, &sym_names, box_base_size, xform);

            log::debug!("Sprite '{}' → ssf2='{}' fm='{}': {} frames with boxes, {} labels",
                sym, ssf2_name, fm_name, frames.len(), frame_labels.len());

            // Check if this animation should be split into sub-animations
            let sub_splits = sub_anim_splits(&fm_name, &frame_labels, sprite.num_frames);

            if sub_splits.is_empty() {
                // Single animation — insert as-is
                if frames.is_empty() { continue; }
                result.insert(fm_name.clone(), AnimationBoxData {
                    ssf2_name,
                    fm_name,
                    total_frames: sprite.num_frames,
                    frames,
                    sprite_frame_offset: 0,
                    frame_labels: frame_labels.clone(),
                });
            } else {
                // Split into multiple FM animations
                for (sub_fm_name, start_frame, end_frame) in sub_splits {
                    let slice_len = end_frame.saturating_sub(start_frame);
                    // Remap frame indices: subtract start_frame so they are 0-based within sub-anim
                    let sliced_frames: BTreeMap<u16, Vec<FrameBox>> = frames.iter()
                        .filter(|(&f, _)| f >= start_frame && f < end_frame)
                        .map(|(&f, boxes)| (f - start_frame, boxes.clone()))
                        .collect();
                    // Remap frame labels the same way
                    let sliced_labels: Vec<(String, u16)> = frame_labels.iter()
                        .filter(|(_, f)| *f >= start_frame && *f < end_frame)
                        .map(|(name, f)| (name.clone(), f - start_frame))
                        .collect();

                    log::debug!("  sub-anim '{}': frames {}..{} ({} frames with boxes, {} labels)",
                        sub_fm_name, start_frame, end_frame, sliced_frames.len(), sliced_labels.len());

                    result.insert(sub_fm_name.clone(), AnimationBoxData {
                        ssf2_name: ssf2_name.clone(),
                        fm_name: sub_fm_name,
                        total_frames: slice_len,
                        frames: sliced_frames,
                        sprite_frame_offset: start_frame,
                        frame_labels: sliced_labels,
                    });
                }
            }
        }
    }

    log::info!("sprite_parser: extracted box data for {}/{} animations before fallbacks",
        result.len(), ssf2_to_fm.len());

    // Apply fallbacks: for animations with no sprite data, clone from the closest related state.
    // These are procedural states in SSF2 that reuse another animation's pose.
    apply_fallbacks(&mut result);

    log::info!("sprite_parser: {}/{} animations have box data after fallbacks",
        result.len(), ssf2_to_fm.len());

    Ok(result)
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

// ─── Sub-animation splitting ─────────────────────────────────────────────────

/// Extract all FrameLabel tags from a DefineSprite, returning (label, frame_number) pairs
/// sorted by frame number.
fn extract_frame_labels(sprite: &swf::Sprite) -> Vec<(String, u16)> {
    let mut frame_num: u16 = 0;
    let mut labels: Vec<(String, u16)> = Vec::new();
    for tag in &sprite.tags {
        match tag {
            swf::Tag::ShowFrame => { frame_num += 1; }
            swf::Tag::FrameLabel(fl) => {
                let label = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                labels.push((label, frame_num));
            }
            _ => {}
        }
    }
    labels.sort_by_key(|(_, f)| *f);
    labels
}

/// Public alias for sub_anim_splits — used by image_extractor to apply the same split.
pub fn sub_anim_image_splits(
    fm_name: &str,
    frame_labels: &[(String, u16)],
    total_frames: u16,
) -> Vec<(String, u16, u16)> {
    sub_anim_splits(fm_name, frame_labels, total_frames)
}

/// SSF2 frame labels that are control-flow markers, not real sub-animation boundaries.
/// These appear in SSF2 ActionScript as `gotoAndPlay("again")` / `gotoAndPlay("finish")`
/// to implement looping/chaining; they don't correspond to distinct Fraymakers animations.
const CONTROL_FLOW_LABELS: &[&str] = &[
    "goback",   // loop back to start of current hit
    "again",    // restart the combo chain if button pressed
    "finish",   // tail-end recovery animation
    "loop",     // generic loop point
    "afterHit", // post-hit recovery
    "endlag",   // end-lag frames
    "continue", // generic continue marker
    "done",     // end of sub-sequence (used in Jump, not jab)
];

/// Taunt sprite label names that map to the three Fraymakers taunt slots.
const TAUNT_LABEL_NAMES: &[&str] = &[
    "taunt_side", "taunt_neutral", "taunt_updown",
];
const TAUNT_FM_NAMES: &[&str] = &["taunt", "taunt_up", "taunt_down"];

/// Given an FM animation name and the internal frame labels of its SSF2 sprite,
/// return a list of (fm_sub_anim_name, start_frame_inclusive, end_frame_exclusive).
/// Returns empty vec if no splitting is needed (animation maps 1:1).
///
/// # Jab splitting strategy
/// Every character uses different label names for jab hits (begin/hit2/hit3,
/// combo1/combo2/combo3, jab2/jab3, kick2, hitHold, etc.).
/// Rather than matching by name, we:
///   1. Filter out known control-flow labels (again, finish, loop, goback, ...)
///   2. Whatever real labels remain — map them positionally to jab1/jab2/jab3/...
///   3. Each real label marks the START of its sub-animation slice.
///   4. If only one real label exists (or the first label starts well into the sprite)
///      we include everything from frame 0 as jab1, starting at the first real label.
fn sub_anim_splits(
    fm_name: &str,
    frame_labels: &[(String, u16)],
    total_frames: u16,
) -> Vec<(String, u16, u16)> {
    match fm_name {
        "jab" => split_jab(frame_labels, total_frames),
        "taunt" => split_taunt(frame_labels, total_frames),
        _ => vec![],
    }
}

/// Split a jab sprite into jab1/jab2/jab3/... by filtering control-flow labels
/// and mapping remaining labels positionally.
fn split_jab(frame_labels: &[(String, u16)], total_frames: u16) -> Vec<(String, u16, u16)> {
    // Filter to only real sub-animation boundary labels
    let real_labels: Vec<(u16, &str)> = frame_labels.iter()
        .filter(|(label, _)| !CONTROL_FLOW_LABELS.contains(&label.as_str()))
        .map(|(label, frame)| (*frame, label.as_str()))
        .collect();

    if real_labels.is_empty() {
        // No labels at all — single jab1 covering the whole sprite
        return vec![("jab1".to_string(), 0, total_frames)];
    }

    // Build ranges: each real label starts a sub-animation that ends at the next one
    let mut splits = Vec::new();
    for (i, &(start_frame, _label)) in real_labels.iter().enumerate() {
        let end_frame = real_labels.get(i + 1)
            .map(|&(f, _)| f)
            .unwrap_or(total_frames);
        let fm_name = format!("jab{}", i + 1);
        splits.push((fm_name, start_frame, end_frame));
        log::trace!("  jab split: '{}' (ssf2 label='{}') frames {}..{}",
            splits.last().unwrap().0, _label, start_frame, end_frame);
    }
    splits
}

/// Split a taunt sprite using the three known SSF2 taunt label names.
/// Unlike jab, taunts have fixed semantic label names across all characters.
fn split_taunt(frame_labels: &[(String, u16)], total_frames: u16) -> Vec<(String, u16, u16)> {
    let label_map: std::collections::HashMap<&str, u16> = frame_labels.iter()
        .map(|(l, f)| (l.as_str(), *f))
        .collect();

    // All three taunt labels must be present
    if TAUNT_LABEL_NAMES.iter().any(|l| !label_map.contains_key(l)) {
        // This character doesn't have the standard 3-taunt layout — don't split
        return vec![];
    }

    let starts: Vec<u16> = TAUNT_LABEL_NAMES.iter()
        .map(|l| *label_map.get(l).unwrap())
        .collect();

    starts.iter().enumerate()
        .map(|(i, &start)| {
            let end = starts.get(i + 1).copied().unwrap_or(total_frames);
            (TAUNT_FM_NAMES[i].to_string(), start, end)
        })
        .collect()
}

// ─── Fallbacks ────────────────────────────────────────────────────────────────

/// For animations with no extracted sprite data, clone box data from the most
/// appropriate related animation. The cloned data keeps the same box shapes but
/// marks the animation name correctly so it lands in the right entity layer.
fn apply_fallbacks(result: &mut BTreeMap<String, AnimationBoxData>) {
    // Table: missing FM anim name → best donor FM anim name
    let fallbacks: &[(&str, &str)] = &[
        // Damage / launched states
        ("stunned",           "hurt"),
        ("star_ko",           "hurt"),
        ("starko",            "hurt"),
        ("screenko",          "hurt"),
        ("buried",            "crouch"),
        // Airborne/misc states
        ("fly",               "jump_aerial"),
        ("swim",              "fall"),
        ("ladder",            "idle"),
        ("wall_stick",        "fall"),
        ("special",           "idle"),
        ("carry",             "grab"),
        // Landing variants
        ("land_heavy",        "land"),
        ("ledge_lean",        "ledge_hang"),
        // Win/lose/respawn
        ("victory",           "taunt"),
        ("defeat",            "hurt"),
        ("respawn",           "idle"),
        // Special air variants
        ("special_down_air",  "special_down"),
        ("special_neutral_air", "special_neutral"),
        ("special_side_air",  "special_side"),
        ("special_up_air",    "special_up"),
        // Item variants
        ("item_float",        "idle"),
        ("item_screw",        "special_up"),
    ];

    let mut to_insert: Vec<AnimationBoxData> = Vec::new();

    for (missing, donor) in fallbacks {
        if result.contains_key(*missing) { continue; }
        if let Some(donor_data) = result.get(*donor) {
            log::debug!("Fallback: '{}' ← '{}' ({} frames)", missing, donor, donor_data.total_frames);
            let mut cloned = donor_data.clone();
            cloned.fm_name = missing.to_string();
            to_insert.push(cloned);
        } else {
            log::debug!("Fallback: '{}' ← '{}' (donor also missing)", missing, donor);
        }
    }

    for data in to_insert {
        result.insert(data.fm_name.clone(), data);
    }
}

/// Find the pixel size of one side of the CollisionBox base shape.
/// SSF2 uses a square shape; we want the width (= height for square).
/// Measure a collision-box character's intrinsic size in pixels.
/// The box character is usually a DefineSprite wrapping a square DefineShape;
/// descend through sprites, folding in each placement matrix's scale so a
/// non-identity inner transform is accounted for.
fn measure_box_char(swf: &swf::Swf, char_id: u16, depth: u8) -> Option<f64> {
    if depth > 4 { return None; }
    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineShape(shape) if shape.id == char_id => {
                let b = &shape.shape_bounds;
                let w = (b.x_max.get() - b.x_min.get()) as f64 / 20.0;
                let h = (b.y_max.get() - b.y_min.get()) as f64 / 20.0;
                return Some((w + h) / 2.0);
            }
            swf::Tag::DefineSprite(sprite) if sprite.id == char_id => {
                for stag in &sprite.tags {
                    if let swf::Tag::PlaceObject(po) = stag {
                        let child = match &po.action {
                            swf::PlaceObjectAction::Place(id)
                            | swf::PlaceObjectAction::Replace(id) => *id,
                            _ => continue,
                        };
                        let scale = po.matrix
                            .map(|m| (m.a.to_f64().abs() + m.d.to_f64().abs()) / 2.0)
                            .unwrap_or(1.0);
                        return measure_box_char(swf, child, depth + 1).map(|s| s * scale);
                    }
                }
                return None;
            }
            _ => {}
        }
    }
    None
}

/// Determine the base (unscaled, pre-PlaceObject-matrix) size of the
/// collision-box shape in pixels.
///
/// SSF2 places every collision box as the same character — a small square
/// shape scaled per instance by the PlaceObject matrix. We find that
/// character by tallying which char id the box-named instances
/// (attackBox / hitBox / grabBox / …) actually place, then measure its
/// shape. The symbol is not reliably named "CollisonBox" (sandbag's isn't),
/// so name-based lookup is only a last-resort fallback.
fn find_collision_box_base_size(swf: &swf::Swf, sym_names: &BTreeMap<u16, String>) -> f64 {
    // Tally char ids placed under box-typed instance names (excluding
    // itemBox, which is a separate character with its own geometry).
    let mut tally: BTreeMap<u16, u32> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            for stag in &sprite.tags {
                if let swf::Tag::PlaceObject(po) = stag {
                    let child = match &po.action {
                        swf::PlaceObjectAction::Place(id)
                        | swf::PlaceObjectAction::Replace(id) => *id,
                        _ => continue,
                    };
                    let Some(name) = po.name.as_ref()
                        .map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string())
                    else { continue };
                    if let Some(bt) = BoxType::from_instance_name(&name) {
                        if bt != BoxType::ItemBox {
                            *tally.entry(child).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }
    if let Some((&char_id, _)) = tally.iter().max_by_key(|(_, &count)| count) {
        if let Some(size) = measure_box_char(swf, char_id, 0) {
            log::info!("CollisonBox char id={}: measured base size {:.1}px", char_id, size);
            return size;
        }
    }

    // Fallback: locate the box character by symbol name.
    let collison_sprite_id = sym_names.iter()
        .find(|(_, name)| {
            let l = name.to_lowercase();
            l.contains("collisonbox") || l.contains("collisionbox")
        })
        .map(|(id, _)| *id);
    if let Some(sprite_id) = collison_sprite_id {
        if let Some(size) = measure_box_char(swf, sprite_id, 0) {
            log::info!("CollisonBox (by name) id={}: base size {:.1}px", sprite_id, size);
            return size;
        }
    }

    log::warn!("CollisonBox shape not found; using default base size {:.0}px", DEFAULT_BASE_SIZE);
    DEFAULT_BASE_SIZE
}

/// Extract SSF2 animation name from a symbol name like "mario_fla.NAir_40".
/// Uses the ssf2_to_fm map to validate/match against known animation names.
pub fn extract_ssf2_anim_name(
    sym: &str,
    _char_lower: &str,
    ssf2_to_fm: &BTreeMap<String, String>,
) -> Option<String> {
    // Symbol format: "{char}_fla.{AnimLabel}_{index}"
    // e.g. "mario_fla.NAir_40" → local = "NAir_40" → strip suffix → "NAir" → normalize → "a_air"
    let local = if sym.contains('.') {
        sym.split('.').last()?
    } else {
        sym
    };

    // Strip trailing _NNN
    let stripped = strip_numeric_suffix(local);

    // Try direct match against SSF2 anim names (case-insensitive)
    // Build a normalized version: "NAir" → "nair", then match against known patterns
    let normalized = normalize_anim_label(stripped);

    // First try exact match
    if ssf2_to_fm.contains_key(&normalized) {
        return Some(normalized);
    }

    // Try known label → SSF2 name mappings
    // These map the AnimLabel in the symbol to the SSF2 xframe name
    let label_to_ssf2: &[(&str, &str)] = &[
        ("idle", "stand"),
        ("stand", "stand"),
        ("entrance", "entrance"),
        ("revival", "revival"),
        ("win", "win"),
        ("lose", "lose"),
        ("walk", "walk"),
        ("run", "run"),
        ("jump", "jump"),
        ("doublejump", "jump_midair"),
        ("djump", "jump_midair"),
        ("fall", "fall"),
        ("land", "land"),
        ("heavyland", "heavyland"),
        ("skid", "skid"),
        ("crouch", "crouch"),
        // Attacks - jab / normals
        // All these map to SSF2 'a' (jab) — they get split into jab1/jab2/jab3 by sub_anim_splits()
        ("jab",      "a"),
        ("jabcombo", "a"),   // captainfalcon_fla.JabCombo, etc.
        ("jablight", "a"),
        ("jabs",     "a"),
        ("jab1", "a"),
        ("jab2", "a"),
        ("dashattack", "a_forward"),
        ("forwardtilt", "a_forward_tilt"),
        ("ftilt", "a_forward_tilt"),
        ("uptilt", "a_up_tilt"),
        ("utilt", "a_up_tilt"),
        ("downtilt", "a_down"),
        ("dtilt", "a_down"),
        ("crouchattack", "crouch_attack"),
        ("forwardsmash", "a_forwardsmash"),
        ("fsmash", "a_forwardsmash"),
        ("upsmash", "a_up"),
        ("usmash", "a_up"),
        // Aerials
        ("nair", "a_air"),
        ("neutralair", "a_air"),
        ("fair", "a_air_forward"),
        ("forwardair", "a_air_forward"),
        ("bair", "a_air_backward"),
        ("backair", "a_air_backward"),
        ("uair", "a_air_up"),
        ("upair", "a_air_up"),
        ("dair", "a_air_down"),
        ("downair", "a_air_down"),
        // Specials
        ("neutralb", "b"),
        ("neutralspecial", "b"),
        ("neutralbair", "b_air"),
        ("sidespecial", "b_forward"),
        ("sideb", "b_forward"),
        ("sidebair", "b_forward_air"),
        ("upspecial", "b_up"),
        ("upb", "b_up"),
        ("upbair", "b_up_air"),
        ("downspecial", "b_down"),
        ("downb", "b_down"),
        ("downbair", "b_down_air"),
        // Throws — both word orders ("ThrowForward" and "ForwardThrow")
        ("throwforward", "throw_forward"),
        ("throwback", "throw_back"),
        ("throwup", "throw_up"),
        ("throwdown", "throw_down"),
        ("forwardthrow", "throw_forward"),
        ("backthrow", "throw_back"),
        ("upthrow", "throw_up"),
        ("downthrow", "throw_down"),
        ("grab", "grab"),
        // Defense
        ("shield", "defend"),
        ("roll", "dodgeroll"),
        ("airdodge", "airdodge"),
        ("sidestep", "sidestep"),
        // Damage
        ("hurt", "hurt"),
        ("hurts", "hurt"),       // SSF2 uses plural: Hurts_67
        ("stun", "stunned"),
        ("stunned", "stunned"),
        ("dizzy", "dizzy"),
        ("sleep", "sleep"),
        ("tumble", "falling"),
        ("tumblefall", "falling"), // TumbleFall_81
        ("sentflying", "crash"),   // SentFlying_72 = knockdown/star-KO trajectory
        ("knockdown", "crash"),
        ("frozen", "frozen"),
        ("egg", "egg"),
        ("star", "star_ko"),
        ("starko", "starko"),
        ("pitfall", "pitfall"),
        // Edge
        ("hang", "hang"),
        ("climbup", "climbup"),
        ("hangclimb", "climbup"),   // HangClimb_74
        ("edgelean", "edgelean"),
        ("rollup", "rollup"),
        ("hangroll", "rollup"),     // HangRoll_75
        ("ledgeattack", "ledge_attack"),
        ("hangattack", "ledge_attack"), // HangAttack_76
        // Defense
        ("guard", "defend"),        // Guard_78
        ("spotdodge", "sidestep"),  // SpotDodge_71
        // Misc
        ("taunts", "taunt"),        // Taunts_86
        ("taunt", "taunt"),
        ("getupattack", "getup_attack"),
        ("getup", "tech_ground"),   // GetUp_82
        ("carry", "carry"),
        ("tech", "tech_ground"),
        ("techground", "tech_ground"), // TechGround_203
        ("techroll", "tech_roll"),
        ("getuproll", "tech_roll"),  // GetUpRoll_77
        // Run
        ("run", "run"),
        ("dash", "run"),
        ("turn", "run"),      // Turn_14 = dash turn, part of run state
        ("revival", "revival"),
        ("win", "win"),
        // Specials (SSF2-specific label names)
        ("nspecial", "b"),
        ("nspecialair", "b_air"),
        ("sspecial", "b_forward"),
        ("sspecialair", "b_forward_air"),
        ("uspecial", "b_up"),
        ("uspecialair", "b_up_air"),
        ("dspecial", "b_down"),
        ("dspecialair", "b_down_air"),
        ("screwattack", "b_up"),    // Mario's up-b is screwattack
        ("specialland", "land"),    // SpecialLand_19
        // Smash attacks (alternate label forms)
        ("dsmash", "a_down"),       // DSmash_29 = down smash, not in xframe table? map to strong_down
        // Throws
        ("fthrow", "throw_forward"),
        ("bthrow", "throw_back"),
        ("uthrow", "throw_up"),
        ("dthrow", "throw_down"),
        ("grabpummel", "grab"),      // Grab_Pummel_66
        // Items
        ("itemswing", "item_jab"),
        ("itemdashattack", "item_dash"),
        ("itemthrows", "toss"),
        ("itemthrowsair", "toss_air"),
        ("itempickup", "item_pickup"),
        ("itemraise", "item_raise"),
        ("itemshoot", "item_shoot"),
        ("itemsmash", "item_smash"),
        ("itemtilt", "item_tilt"),
        ("itemfan", "item_fan"),
        ("itemhomerun", "item_homerun"),
        ("itemhomrun", "item_homerun"), // alternate spelling
    ];

    for (label, ssf2) in label_to_ssf2 {
        if normalized == *label {
            return Some(ssf2.to_string());
        }
    }

    None
}

/// Static SSF2 xframe name → Fraymakers animation name lookup.
/// Mirrors extractor::build_ssf2_to_fm_anim without needing the dynamic xframe map.
/// Used when a character's bytecode doesn't call setXFrame for a given animation
/// (e.g. their jab sprite exists but they use a custom label instead of 'a').
fn static_ssf2_to_fm(ssf2_name: &str) -> Option<String> {
    let table: &[(&str, &str)] = &[
        ("stand",           "idle"),
        ("walk",            "walk"),
        ("run",             "run"),
        ("jump",            "jump"),
        ("jump_midair",     "jump_aerial"),
        ("fall",            "fall"),
        ("land",            "land"),
        ("heavyland",       "land_heavy"),
        ("skid",            "skid"),
        ("crouch",          "crouch"),
        ("entrance",        "entry"),
        ("revival",         "respawn"),
        ("win",             "victory"),
        ("lose",            "defeat"),
        ("a",               "jab"),
        ("a_forward",       "dash_attack"),
        ("a_forward_tilt",  "tilt_forward"),
        ("a_up_tilt",       "tilt_up"),
        ("a_down",          "tilt_down"),
        ("crouch_attack",   "tilt_down"),
        ("a_forwardsmash",  "strong_forward"),
        ("a_up",            "strong_up"),
        ("a_air",           "aerial_neutral"),
        ("a_air_forward",   "aerial_forward"),
        ("a_air_backward",  "aerial_back"),
        ("a_air_up",        "aerial_up"),
        ("a_air_down",      "aerial_down"),
        ("b",               "special_neutral"),
        ("b_air",           "special_neutral_air"),
        ("b_forward",       "special_side"),
        ("b_forward_air",   "special_side_air"),
        ("b_up",            "special_up"),
        ("b_up_air",        "special_up_air"),
        ("b_down",          "special_down"),
        ("b_down_air",      "special_down_air"),
        ("throw_forward",   "throw_forward"),
        ("throw_back",      "throw_back"),
        ("throw_up",        "throw_up"),
        ("throw_down",      "throw_down"),
        ("ledge_attack",    "ledge_attack"),
        ("getup_attack",    "getup_attack"),
        ("defend",          "shield"),
        ("dodgeroll",       "roll"),
        ("airdodge",        "airdodge"),
        ("sidestep",        "sidestep"),
        ("grab",            "grab"),
        ("carry",           "carry"),
        ("hurt",            "hurt"),
        ("stunned",         "stunned"),
        ("dizzy",           "dizzy"),
        ("sleep",           "sleep"),
        ("falling",         "tumble"),
        ("crash",           "knockdown"),
        ("frozen",          "frozen"),
        ("egg",             "egg"),
        ("star",            "star_ko"),
        ("pitfall",         "buried"),
        ("hang",            "ledge_hang"),
        ("climbup",         "ledge_climb"),
        ("edgelean",        "ledge_lean"),
        ("rollup",          "ledge_roll"),
        ("wallstick",       "wall_stick"),
        ("taunt",           "taunt"),
        ("swim",            "swim"),
        ("ladder",          "ladder"),
        ("flying",          "fly"),
        ("tech_ground",     "tech"),
        ("tech_roll",       "tech_roll"),
        ("toss",            "item_throw"),
        ("toss_air",        "item_throw_air"),
        ("item_jab",        "item_jab"),
    ];
    table.iter().find(|(k, _)| *k == ssf2_name).map(|(_, v)| v.to_string())
}

fn strip_numeric_suffix(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    // Strip trailing _NNN
    if let Some(pos) = s.rfind('_') {
        let suffix = &s[pos + 1..];
        if suffix.chars().all(|c| c.is_ascii_digit()) {
            end = pos;
        }
    }
    &s[..end]
}

fn normalize_anim_label(s: &str) -> String {
    // CamelCase → lowercase, remove underscores for matching
    s.to_lowercase().replace('_', "")
}

/// Extract per-frame collision box data from a single DefineSprite.
/// Returns a map of frame_index → list of boxes.
///
/// Strategy: simulate the Flash display list.
/// PlaceObject::Place(id) with name = new object at depth
/// PlaceObject::Modify = move/transform existing object at depth (no character change)
/// PlaceObject::Replace(id) = replace object at depth
/// Extract collision boxes for a specific DefineSprite by ID.
/// Used for projectile inner sprites (e.g. mario_fireball_mc).
/// Returns AnimationBoxData with frames keyed 0-based.
pub fn extract_boxes_for_sprite_id(
    swf_data: &[u8],
    sprite_id: u16,
) -> anyhow::Result<Option<AnimationBoxData>> {
    let swf_buf = swf::decompress_swf(Cursor::new(swf_data))?;
    let swf = swf::parse_swf(&swf_buf)?;

    let mut sym_names: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                sym_names.insert(link.id, name);
            }
        }
    }

    let box_base_size = find_collision_box_base_size(&swf, &sym_names);
    let identity = XframeTransform::default();

    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            if sprite.id != sprite_id { continue; }
            let frames = extract_frame_boxes(sprite, &sym_names, box_base_size, identity);
            if frames.is_empty() { return Ok(None); }
            return Ok(Some(AnimationBoxData {
                ssf2_name: String::new(),
                fm_name: String::new(),
                total_frames: sprite.num_frames,
                frames,
                sprite_frame_offset: 0,
                frame_labels: vec![],
            }));
        }
    }
    Ok(None)
}

/// RemoveObject = remove from display list
///
/// We track the display list across frames and snapshot the current boxes each frame.
fn extract_frame_boxes(
    sprite: &swf::Sprite,
    sym_names: &BTreeMap<u16, String>,
    base_size: f64,
    root_xform: XframeTransform,
) -> BTreeMap<u16, Vec<FrameBox>> {
    // Display list: depth → (char_id, instance_name, matrix)
    let mut display_list: BTreeMap<u16, DisplayItem> = BTreeMap::new();
    let mut current_frame: u16 = 0;
    let mut result: BTreeMap<u16, Vec<FrameBox>> = BTreeMap::new();

    // Pre-identify the collision box char id from symbol names
    // Any symbol whose name contains "collisonbox" or "collisionbox"
    let is_collision_char: std::collections::HashSet<u16> = sym_names.iter()
        .filter(|(_, n)| {
            let l = n.to_lowercase();
            l.contains("collisonbox") || l.contains("collisionbox")
        })
        .map(|(id, _)| *id)
        .collect();

    for tag in &sprite.tags {
        match tag {
            swf::Tag::ShowFrame => {
                // Snapshot collision boxes active this frame
                let boxes: Vec<FrameBox> = display_list.values()
                    .filter(|item| {
                        // Include if it's a known collision char id
                        // OR if the instance name looks like a box
                        is_collision_char.contains(&item.char_id)
                            || BoxType::from_instance_name(&item.instance_name).is_some()
                    })
                    .filter_map(|item| {
                        let box_type = BoxType::from_instance_name(&item.instance_name)?;
                        let sx = root_xform.sx;
                        let sy = root_xform.sy;

                        if box_type == BoxType::ItemBox {
                            // itemBox (id=991): the PlaceObject tx/ty is the hand attachment
                            // point, which is at the BOTTOM of the shape (origin of id=991).
                            // The shape hangs upward from this point.
                            let (lcx, lcy, lw, lh, rot) = matrix_to_itembox(&item.matrix);
                            // Hand position in world space
                            let whx = root_xform.tx + lcx * sx;
                            let why = root_xform.ty + lcy * sy;
                            let ww = lw * sx.abs();
                            let wh = lh * sy.abs();
                            // Top-left: hand is at bottom-center, so top = hand_y - height
                            return Some(FrameBox {
                                box_type,
                                instance_name: item.instance_name.clone(),
                                x: whx - ww / 2.0,
                                y: why - wh,
                                width: ww,
                                height: wh,
                                rotation: rot,
                            });
                        }

                        // CollisonBox_6: 50×50 centered at origin.
                        // matrix_to_box returns (top_left_x, top_left_y, width, height).
                        let (local_x, local_y, local_w, local_h) = matrix_to_box(&item.matrix, base_size);
                        let world_x = root_xform.tx + local_x * sx;
                        let world_y = root_xform.ty + local_y * sy;
                        let world_w = (local_w * sx).abs();
                        let world_h = (local_h * sy).abs();
                        Some(FrameBox {
                            box_type,
                            instance_name: item.instance_name.clone(),
                            x: world_x,
                            y: world_y,
                            width: world_w,
                            height: world_h,
                            rotation: 0.0,
                        })
                    })
                    .collect();

                if !boxes.is_empty() {
                    result.insert(current_frame, boxes);
                }
                current_frame += 1;
            }

            swf::Tag::PlaceObject(po) => {
                let inst_name = po.name
                    .map(|s| s.to_str_lossy(encoding_rs::WINDOWS_1252).to_string())
                    .unwrap_or_default();

                match &po.action {
                    swf::PlaceObjectAction::Place(char_id) => {
                        // Only track if it's a collision box char OR has a box-like name
                        let is_box = is_collision_char.contains(char_id)
                            || BoxType::from_instance_name(&inst_name).is_some();
                        if is_box || !inst_name.is_empty() {
                            display_list.insert(po.depth, DisplayItem {
                                char_id: *char_id,
                                instance_name: inst_name,
                                matrix: po.matrix.unwrap_or(swf::Matrix::IDENTITY),
                            });
                        }
                    }
                    swf::PlaceObjectAction::Replace(char_id) => {
                        let is_box = is_collision_char.contains(char_id)
                            || BoxType::from_instance_name(&inst_name).is_some();
                        if is_box || !inst_name.is_empty() {
                            let entry = display_list.entry(po.depth).or_insert(DisplayItem {
                                char_id: *char_id,
                                instance_name: inst_name.clone(),
                                matrix: swf::Matrix::IDENTITY,
                            });
                            entry.char_id = *char_id;
                            if !inst_name.is_empty() { entry.instance_name = inst_name; }
                            if let Some(m) = po.matrix { entry.matrix = m; }
                        }
                    }
                    swf::PlaceObjectAction::Modify => {
                        // Update position of existing object
                        if let Some(entry) = display_list.get_mut(&po.depth) {
                            if let Some(m) = po.matrix { entry.matrix = m; }
                            if !inst_name.is_empty() { entry.instance_name = inst_name; }
                        }
                    }
                }
            }

            swf::Tag::RemoveObject(ro) => {
                display_list.remove(&ro.depth);
            }

            _ => {}
        }
    }

    result
}

struct DisplayItem {
    char_id: u16,
    instance_name: String,
    matrix: swf::Matrix,
}

/// Convert a SWF matrix to (x, y, width, height) in pixels.
/// The CollisonBox shape is base_size × base_size pixels, **centered at (0,0)**.
/// Matrix scale (a,d) gives the actual dimensions; tx/ty give the **center** position.
/// Returns (top_left_x, top_left_y, width, height) — all in pixels.
fn matrix_to_box(m: &swf::Matrix, base_size: f64) -> (f64, f64, f64, f64) {
    // tx/ty are in twips (1/20 pixel) → convert to pixels
    let cx = m.tx.get() as f64 / 20.0;  // center x
    let cy = m.ty.get() as f64 / 20.0;  // center y

    // a = scale_x, d = scale_y (Fixed16 → f64)
    let scale_x = m.a.to_f64();
    let scale_y = m.d.to_f64();

    let w = (scale_x * base_size).abs();
    let h = (scale_y * base_size).abs();

    // The CollisonBox shape spans -25..25 (for base_size=50), so tx/ty is the CENTER.
    // Convert center → top-left:
    let x = cx - w / 2.0;
    let y = cy - h / 2.0;

    (x, y, w, h)
}

/// Extract itemBox geometry from its PlaceObject matrix.
/// The itemBox (id=991) contains itemPlaceholder_mc_5 (id=990) at offset (-1.45, -20.95),
/// which contains a thin ~3.7x21.9 shape. The PlaceObject places id=991 with scale+rotation.
/// The relevant geometry for Fraymakers is: center = (tx, ty), effective size from scale.
/// Returns (center_x, center_y, scaled_width, scaled_height, rotation_degrees).
fn matrix_to_itembox(m: &swf::Matrix) -> (f64, f64, f64, f64, f64) {
    let cx = m.tx.get() as f64 / 20.0;
    let cy = m.ty.get() as f64 / 20.0;

    // The inner placeholder shape is roughly 3.7w x 21.9h centered near its origin.
    // The itemBox inner offset is (-1.45, -20.95) relative to id=991's origin,
    // meaning the shape's midpoint is approximately at (-1.45 + 1.85, -20.95 + 10.95) = (0.4, -10.0)
    // relative to id=991's origin. Close enough to (0, -10.0) for practical purposes.
    //
    // In SSF2, the item box represents the grab range for holding items.
    // It's placed with scale controlling overall size and b/c matrix for rotation.
    // We extract: scale magnitude → size, atan2(b, a) → rotation.
    let sx = m.a.to_f64();
    let _sy = m.d.to_f64();
    let bx = m.b.to_f64();
    let _by = m.c.to_f64();

    // Rotation from the matrix (degrees, CCW positive in SWF = CW positive visually)
    let rotation_rad = bx.atan2(sx);  // atan2(b, a) gives rotation
    let rotation_deg = rotation_rad.to_degrees();

    // Scale magnitude (handles rotation shear)
    let scale = (sx * sx + bx * bx).sqrt();

    // The placeholder inner shape: ~3.7w x 21.9h
    // After scale: these are approximate — the visual grab box.
    let inner_w = 3.7;
    let inner_h = 21.9;
    let w = inner_w * scale;
    let h = inner_h * scale;

    (cx, cy, w, h, rotation_deg)
}

// ─── Root MovieClip xframe scale extraction ──────────────────────────────────

/// Extract the median scaleX and scaleY from the root character MovieClip's
/// xframe PlaceObject entries (the `stance` sub-sprites placed on each frame).
///
/// SSF2 characters have a root DefineSprite whose SymbolClass name matches the
/// character name exactly (e.g. "mario"). Each frame of this sprite places
/// animation sub-sprites via PlaceObject with instance name "stance".
/// The matrix on these PlaceObjects contains the visual scale of the character.
///
/// We use the median (absolute value) to ignore outliers like "flying" (near-zero
/// to hide the sprite) or "frozen"/"egg" (oversized).
///
/// Returns `(median_scale_x, median_scale_y)`, or `(1.0, 1.0)` if no data found.
pub fn extract_xframe_scale(
    swf_data: &[u8],
    char_name: &str,
) -> anyhow::Result<(f64, f64)> {
    let swf_buf = swf::decompress_swf(Cursor::new(swf_data))?;
    let swf = swf::parse_swf(&swf_buf)?;

    let mut sym_names: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                sym_names.insert(link.id, name);
            }
        }
    }

    let char_lower = char_name.to_lowercase();
    let mut scale_xs: Vec<f64> = Vec::new();
    let mut scale_ys: Vec<f64> = Vec::new();

    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sym = sym_names.get(&sprite.id).cloned().unwrap_or_default();
            // Match the root character sprite (exact name match, not _fla. sub-sprites)
            if sym.to_lowercase() != char_lower { continue; }

            log::info!("Found root character MovieClip: id={} sym='{}' ({} frames)",
                sprite.id, sym, sprite.num_frames);

            for stag in &sprite.tags {
                if let swf::Tag::PlaceObject(po) = stag {
                    let inst = po.name.as_ref()
                        .map(|s| String::from_utf8_lossy(s.as_bytes()).to_string())
                        .unwrap_or_default();
                    // Only collect scale from "stance" placements (the xframe sub-sprites)
                    if inst != "stance" { continue; }

                    if let Some(m) = &po.matrix {
                        let sx = m.a.to_f64().abs();
                        let sy = m.d.to_f64().abs();
                        // Skip near-zero scales (hidden sprites, e.g. "flying")
                        if sx > 0.01 && sy > 0.01 {
                            scale_xs.push(sx);
                            scale_ys.push(sy);
                        }
                    } else {
                        // No matrix = identity
                        scale_xs.push(1.0);
                        scale_ys.push(1.0);
                    }
                }
            }
            break; // Only one root sprite per character
        }
    }

    if scale_xs.is_empty() {
        log::warn!("No xframe stance placements found for '{}', defaulting to 1.0", char_name);
        return Ok((1.0, 1.0));
    }

    scale_xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    scale_ys.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let med_x = median_f64(&scale_xs);
    let med_y = median_f64(&scale_ys);

    log::info!("xframe scale for '{}': median scaleX={:.4}, scaleY={:.4} ({} samples)",
        char_name, med_x, med_y, scale_xs.len());

    Ok((med_x, med_y))
}

fn median_f64(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 { return 0.0; }
    if n % 2 == 1 { sorted[n / 2] } else { (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0 }
}
