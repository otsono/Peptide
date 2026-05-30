/// Animation splitter — applies anim_split_rules.json to expand multi-label
/// SSF2 animations into separate Fraymakers animations.
///
/// Each `SplitAnim` describes one output Fraymakers animation.
/// It references the source animation by name plus a frame range [start, end).
/// The entity generator uses this to slice sprite_boxes and anim_images.

use std::collections::BTreeMap;
use crate::sprite_parser::AnimationBoxData;

/// One output Fraymakers animation, potentially a sub-range of an SSF2 animation.
#[derive(Debug, Clone)]
pub struct SplitAnim {
    /// Fraymakers animation name (e.g. "strong_forward_in")
    pub fm_name: String,
    /// Source SSF2/FM animation key in sprite_boxes / anim_images
    pub source_anim: String,
    /// Start frame within source animation (0-based, inclusive)
    pub start_frame: u16,
    /// End frame within source animation (exclusive); u16::MAX = use full length
    pub end_frame: u16,
    /// Frame labels within this split: (label_name, frame_within_split) — same order as AnimationBoxData::frame_labels
    pub labels: Vec<(String, u16)>,
    /// Whether this animation's tail should loop (AnimationStats endType=LOOP)
    pub loop_tail: bool,
    /// Frame within this split at which the loop starts (relative to split start)
    pub loop_frame: Option<u16>,
}

/// Produce the list of output animations for a character.
/// Returns one `SplitAnim` per Fraymakers animation to emit.
pub fn split_animations(
    source_anims: &BTreeMap<String, crate::extractor::AnimationInfo>,
    sprite_boxes: &BTreeMap<String, AnimationBoxData>,
) -> Vec<SplitAnim> {
    let mut out: Vec<SplitAnim> = Vec::new();

    for (anim_name, _anim_info) in source_anims {
        let sb = sprite_boxes.get(anim_name);
        let total = sb.map(|s| s.total_frames).unwrap_or(1);
        let labels: Vec<(String, u16)> = sb
            .map(|s| s.frame_labels.clone())
            .unwrap_or_default();

        // Build label map: label_name -> frame_offset (within this animation)
        let label_map: BTreeMap<&str, u16> = labels.iter()
            .map(|(lbl, f)| (lbl.as_str(), *f))
            .collect();

        // Helper: find frame for a label, defaulting to end of animation
        let _lf = |lbl: &str| -> u16 { *label_map.get(lbl).unwrap_or(&total) };

        match anim_name.as_str() {

            // ── Aerials: split active frames + land animation ──────────────────
            "aerial_neutral" | "aerial_forward" | "aerial_back" | "aerial_up" | "aerial_down" => {
                let land_frame = label_map.get("continue")
                    .or_else(|| label_map.get("repeat"))
                    .copied();
                if let Some(lf_continue) = land_frame {
                    push_split(&mut out, anim_name, anim_name, 0, lf_continue, &labels, false, None);
                    let land_name = format!("{}_land", anim_name);
                    push_split(&mut out, &land_name, anim_name, lf_continue, total, &labels, false, None);
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Strong attacks: in / charge / attack ───────────────────────────
            "strong_forward" | "strong_up" | "strong_down" => {
                if let (Some(&charge_f), Some(&attack_f)) = (label_map.get("charging"), label_map.get("attack")) {
                    let in_name    = format!("{}_in", anim_name);
                    let charge_name = format!("{}_charge", anim_name);
                    let attack_name = format!("{}_attack", anim_name);
                    push_split(&mut out, &in_name,     anim_name, 0,        charge_f, &labels, false, None);
                    push_split(&mut out, &charge_name, anim_name, charge_f, attack_f, &labels, true, Some(0));
                    push_split(&mut out, &attack_name, anim_name, attack_f, total,    &labels, false, None);
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Grab / carry: standing grab, dash grab, hold, pummel ───────────
            "grab" | "carry" => {
                if anim_name == "carry" && out.iter().any(|s| s.fm_name == "grab") {
                    // carry shares the same inner sprite as grab — skip, already emitted
                    continue;
                }
                let dashgrab_f = label_map.get("dashgrab").copied();
                let grabbed_f  = label_map.get("grabbed").copied()
                    .or_else(|| label_map.get("grabbed2").copied());
                let attack_f   = label_map.get("attack").copied();

                if let (Some(dg_f), Some(gb_f), Some(atk_f)) = (dashgrab_f, grabbed_f, attack_f) {
                    let grab_end = dg_f;
                    push_split(&mut out, "grab",        anim_name, 0,     grab_end, &labels, false, None);
                    push_split(&mut out, "dash_grab",   anim_name, dg_f,  gb_f,    &labels, false, None);
                    push_split(&mut out, "grab_hold",   anim_name, gb_f,  atk_f,   &labels, true, Some(0));
                    push_split(&mut out, "grab_pummel", anim_name, atk_f, total,   &labels, false, None);
                } else if label_map.contains_key("grabbed") {
                    // samus-style: grabbed comes before dashgrab
                    let gb_f  = label_map["grabbed"];
                    let _loop_f = label_map.get("loop").copied().unwrap_or(gb_f);
                    let atk_f  = label_map.get("attack").copied().unwrap_or(total);
                    let dg_f   = label_map.get("dashgrab").copied().unwrap_or(atk_f);
                    push_split(&mut out, "grab",        anim_name, 0,     gb_f,  &labels, false, None);
                    push_split(&mut out, "grab_hold",   anim_name, gb_f,  atk_f, &labels, true, Some(0));
                    push_split(&mut out, "grab_pummel", anim_name, atk_f, dg_f,  &labels, false, None);
                    push_split(&mut out, "dash_grab",   anim_name, dg_f,  total, &labels, false, None);
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Run: dash / run / run_turn ─────────────────────────────────────
            "run" => {
                // Pattern A: run, dash, turn  (mario/captainfalcon/naruto)
                // Pattern B: run, run, turn   (jigglypuff/link/samus) — first 'run' is loop
                let _first_label = labels.first().map(|(l, _)| l.as_str()).unwrap_or("");
                let has_dash = label_map.contains_key("dash");
                let turn_f = label_map.get("turn").copied();

                if has_dash {
                    // Pattern A: run(loop)=first, dash=initial dash, turn=turnaround
                    let dash_f = label_map["dash"];
                    let run_f  = 0u16;
                    let turn_f = turn_f.unwrap_or(total);
                    push_split(&mut out, "dash",     anim_name, dash_f, turn_f, &labels, false, None);
                    push_split(&mut out, "run",      anim_name, run_f,  dash_f, &labels, true, Some(0));
                    push_split(&mut out, "run_turn", anim_name, turn_f, total,  &labels, false, None);
                } else if let Some(turn_f) = turn_f {
                    // Pattern B: labels are [run_frame_0, run_frame_N, turn]
                    // first segment = loop, second run segment doesn't exist (same frames)
                    // just split run / run_turn
                    push_split(&mut out, "run",      anim_name, 0,      turn_f, &labels, true, Some(0));
                    push_split(&mut out, "run_turn", anim_name, turn_f, total,  &labels, false, None);
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Jump: forward / back ───────────────────────────────────────────
            "jump" => {
                if let Some(&bf) = label_map.get("backflip") {
                    // frames before backflip = forward jump (includes any 'done' tail)
                    let back_end = total;
                    push_split(&mut out, "jump",      anim_name, 0,   bf,       &labels, false, None);
                    push_split(&mut out, "jump_back", anim_name, bf,  back_end, &labels, false, None);
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Jump aerial (double jump): forward / back ─────────────────────
            "jump_aerial" => {
                if let Some(&bf) = label_map.get("backflip") {
                    push_split(&mut out, "jump_aerial",      anim_name, 0,  bf,    &labels, false, None);
                    push_split(&mut out, "jump_aerial_back", anim_name, bf, total, &labels, false, None);
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Idle: idle / idle_bored (uncrouch NOT extracted from idle) ─────
            "idle" => {
                let bored_f = label_map.get("bored")
                    .or_else(|| label_map.get("blink"))
                    .copied();
                let uncrouch_f = label_map.get("uncrouch").copied();
                if let Some(bf) = bored_f {
                    let idle_end = bf;
                    // idle loops at its own end
                    push_split(&mut out, "idle",       anim_name, 0,   idle_end, &labels, true, Some(0));
                    // bored: from bored label to uncrouch (or end)
                    let bored_end = uncrouch_f.unwrap_or(total);
                    push_split(&mut out, "idle_bored", anim_name, bf, bored_end, &labels, false, None);
                    // uncrouch NOT emitted from idle — crouch_out comes from crouch anim
                } else {
                    push_split(&mut out, "idle", anim_name, 0, uncrouch_f.unwrap_or(total), &labels, true, Some(0));
                }
            }

            // ── Crouch: crouch_in / crouch_loop (+ naruto crawl variants) ──────
            "crouch" => {
                let loop_f = label_map.get("loop")
                    .or_else(|| label_map.get("redo"))
                    .copied();
                if let Some(lf) = loop_f {
                    push_split(&mut out, "crouch_in",   anim_name, 0,   lf,    &labels, false, None);
                    // naruto also has crawlforward/crawlback after walking
                    let crawl_f = label_map.get("crawlforward").copied();
                    let crawlb_f = label_map.get("crawlback").copied();
                    let walk_f  = label_map.get("walking").copied();
                    if let Some(cf) = crawl_f {
                        let loop_end = walk_f.unwrap_or(cf);
                        push_split(&mut out, "crouch_loop",    anim_name, lf,     loop_end, &labels, true, Some(0));
                        push_split(&mut out, "crouch_forward", anim_name, cf, crawlb_f.unwrap_or(total), &labels, true, Some(0));
                        if let Some(cb_f) = crawlb_f {
                            push_split(&mut out, "crouch_back", anim_name, cb_f, total, &labels, true, Some(0));
                        }
                    } else {
                        push_split(&mut out, "crouch_loop", anim_name, lf, total, &labels, true, Some(0));
                    }
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Crouch_out: extract from 'uncrouch' label in crouch (if present)
            // Actually crouch_out comes from the crouch sprite's uncrouch label.
            // It's handled above in "crouch" — we don't emit it from "idle".

            // ── Special up: active frames / helpless fall ─────────────────────
            "special_up" | "special_up_air" => {
                let fall_f = label_map.get("fallLoop").copied();
                if let Some(ff) = fall_f {
                    push_split(&mut out, anim_name, anim_name, 0,   ff,    &labels, false, None);
                    // helpless: only emit once (special_up and special_up_air both reference it)
                    if !out.iter().any(|s| s.fm_name == "helpless") {
                        push_split(&mut out, "helpless", anim_name, ff, total, &labels, true, Some(0));
                    }
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Item smash: in / charge / attack ──────────────────────────────
            "item_smash" => {
                if let (Some(&charge_f), Some(&attack_f)) = (label_map.get("charging"), label_map.get("attack")) {
                    push_split(&mut out, "item_smash_in",     anim_name, 0,        charge_f, &labels, false, None);
                    push_split(&mut out, "item_smash_charge", anim_name, charge_f, attack_f, &labels, true, Some(0));
                    push_split(&mut out, "item_smash",        anim_name, attack_f, total,    &labels, false, None);
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Item throw: 4 directions ───────────────────────────────────────
            "item_throw" | "item_throw_air" => {
                let down_f    = label_map.get("toss_down").copied();
                let up_f      = label_map.get("toss_up").copied();
                let forward_f = label_map.get("toss_forward").copied();
                if let (Some(df), Some(uf), Some(ff)) = (down_f, up_f, forward_f) {
                    let back_end = df;
                    let down_end = uf;
                    let up_end   = ff;
                    push_split(&mut out, &format!("{}_back",    anim_name), anim_name, 0,   back_end, &labels, false, None);
                    push_split(&mut out, &format!("{}_down",    anim_name), anim_name, df,  down_end, &labels, false, None);
                    push_split(&mut out, &format!("{}_up",      anim_name), anim_name, uf,  up_end,   &labels, false, None);
                    push_split(&mut out, &format!("{}_forward", anim_name), anim_name, ff,  total,    &labels, false, None);
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Item screw: active / helpless ─────────────────────────────────
            "item_screw" => {
                if let Some(&ff) = label_map.get("fallLoop") {
                    push_split(&mut out, "item_screw", anim_name, 0,  ff,    &labels, false, None);
                    if !out.iter().any(|s| s.fm_name == "helpless") {
                        push_split(&mut out, "helpless", anim_name, ff, total, &labels, true, Some(0));
                    }
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Fly (tumble): regular / backflip ──────────────────────────────
            "fly" => {
                if let Some(&bf) = label_map.get("backflip") {
                    push_split(&mut out, "tumble",      anim_name, 0,  bf,    &labels, false, None);
                    push_split(&mut out, "tumble_back", anim_name, bf, total, &labels, false, None);
                } else {
                    // No backflip: just emit as tumble
                    push_split(&mut out, "tumble", anim_name, 0, total, &labels, false, None);
                }
            }

            // ── Helpless: reg / spec variants ────────────────────────────────
            "helpless" => {
                if let Some(&sf) = label_map.get("spec") {
                    let reg_f = label_map.get("reg").copied().unwrap_or(0);
                    push_split(&mut out, "helpless",         anim_name, reg_f, sf,    &labels, true, Some(0));
                    push_split(&mut out, "helpless_special", anim_name, sf,    total, &labels, true, Some(0));
                } else {
                    push_split(&mut out, "helpless", anim_name, 0, total, &labels, true, Some(0));
                }
            }

            // ── Ledge hang/lean: ledge_in / ledge_loop ────────────────────────
            "ledge_hang" | "ledge_lean" => {
                let loop_f = label_map.get("loop").copied();
                if let Some(lf) = loop_f {
                    let tether_f = label_map.get("tetherGrab").copied();
                    let in_end = lf;
                    push_split(&mut out, "ledge_in",   anim_name, 0,   in_end, &labels, false, None);
                    push_split(&mut out, "ledge_loop", anim_name, lf,  tether_f.unwrap_or(total), &labels, true, Some(0));
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Shield: shield_in / shield_loop (/ shield_out if shielddrop) ──
            "shield" => {
                let loop_f = label_map.get("loop")
                    .or_else(|| label_map.get("pause"))
                    .or_else(|| label_map.get("again"))
                    .or_else(|| label_map.get("stopped"))
                    .copied();
                if let Some(lf) = loop_f {
                    push_split(&mut out, "shield_in",   anim_name, 0,  lf,    &labels, false, None);
                    let drop_f = label_map.get("shielddrop").copied();
                    let loop_end = drop_f.unwrap_or(total);
                    push_split(&mut out, "shield_loop", anim_name, lf, loop_end, &labels, true, Some(0));
                    if let Some(df) = drop_f {
                        push_split(&mut out, "shield_out", anim_name, df, total, &labels, false, None);
                    }
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Fall: loop at 'loop'/'redo' start, or split off uspecFall ─────
            "fall" | "swim" | "wall_stick" => {
                let loop_f = label_map.get("loop")
                    .or_else(|| label_map.get("redo"))
                    .copied();
                let uspec_f = label_map.get("uspecFall").copied();
                if let Some(uf) = uspec_f {
                    push_split(&mut out, anim_name,     anim_name, 0,  uf,    &labels, true, loop_f);
                    push_split(&mut out, "fall_special", anim_name, uf, total, &labels, true, Some(0));
                } else if let Some(lf) = loop_f {
                    push_split(&mut out, anim_name, anim_name, 0, total, &labels, true, Some(lf));
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Buried / dizzy: loop at tail ──────────────────────────────────
            "buried" | "dizzy" => {
                let loop_f = label_map.get("loop")
                    .or_else(|| label_map.get("again"))
                    .copied();
                if let Some(lf) = loop_f {
                    push_split(&mut out, anim_name, anim_name, 0, total, &labels, true, Some(lf));
                } else {
                    push_full(&mut out, anim_name, total, &labels);
                }
            }

            // ── Land: light / heavy (dodgeland) ──────────────────────────────
            "land" => {
                if let Some(&df) = label_map.get("dodgeland") {
                    push_split(&mut out, "land_light", anim_name, 0,  df,    &labels, false, None);
                    push_split(&mut out, "land_heavy", anim_name, df, total, &labels, false, None);
                } else {
                    push_split(&mut out, "land_light", anim_name, 0, total, &labels, false, None);
                }
            }

            // ── Jab4 rapid: loop at 'again' ───────────────────────────────────
            "jab4" => {
                let loop_f = label_map.get("again").copied();
                push_split(&mut out, anim_name, anim_name, 0, total, &labels, loop_f.is_some(), loop_f);
            }

            // ── Idle-sprite bleedthrough: truncate at uncrouch/bored/blink ────
            // item_float, ladder, respawn, special, select_screen share idle sprite
            "item_float" | "ladder" | "respawn" | "special" | "select_screen" => {
                let end_f = label_map.get("bored")
                    .or_else(|| label_map.get("blink"))
                    .or_else(|| label_map.get("uncrouch"))
                    .copied()
                    .unwrap_or(total);
                push_split(&mut out, anim_name, anim_name, 0, end_f, &labels, false, None);
            }

            // ── Hurt / defeat / stunned / screenko / starko / star_ko:
            //    ignore tail (engine routes hurt variants) ─────────────────────
            "hurt" | "defeat" | "stunned" | "screenko" | "starko" | "star_ko" => {
                // Keep only the first segment before done1/hurt2 etc.
                let first_tail = label_map.get("done1")
                    .or_else(|| label_map.get("hurt2"))
                    .copied()
                    .unwrap_or(total);
                push_split(&mut out, anim_name, anim_name, 0, first_tail, &labels, false, None);
            }

            // ── Taunt sprite bleedthrough: ignore tail ────────────────────────
            "taunt_up" | "taunt_down" => {
                let end_f = label_map.get("taunt_side").copied().unwrap_or(total);
                push_split(&mut out, anim_name, anim_name, 0, end_f, &labels, false, None);
            }

            // ── Tech: ignore 'done' tail (1f exit marker) ────────────────────
            "tech" | "tech_roll" => {
                let end_f = label_map.get("done").copied().unwrap_or(total);
                push_split(&mut out, anim_name, anim_name, 0, end_f, &labels, false, None);
            }

            // ── Tilt_forward: 'remove' is endlag, include it ─────────────────
            // special_side_air afterHit: include in animation
            // special_down_air continue+afterHit: include in animation
            // throw_back loop+loop2: keep full (loop is grab hold, loop2 is endlag)
            // victory win1+loop: keep full
            // item_homerun attack: keep full (windup+active)
            // jab1 goback/continue: keep full
            // ledge_hang tetherGrab: already handled above

            // ── Default: emit as-is (no split needed) ────────────────────────
            _ => {
                push_full(&mut out, anim_name, total, &labels);
            }
        }
    }

    // Deduplicate by fm_name — keep the first occurrence of each
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    out.retain(|s| seen.insert(s.fm_name.clone()));

    out
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn push_full(out: &mut Vec<SplitAnim>, name: &str, total: u16, labels: &[(String, u16)]) {
    out.push(SplitAnim {
        fm_name:     name.to_string(),
        source_anim: name.to_string(),
        start_frame: 0,
        end_frame:   total,
        labels:      labels.to_vec(),
        loop_tail:   false,
        loop_frame:  None,
    });
}

fn push_split(
    out: &mut Vec<SplitAnim>,
    fm_name: &str,
    source_anim: &str,
    start: u16,
    end: u16,
    all_labels: &[(String, u16)],
    loop_tail: bool,
    loop_frame: Option<u16>,
) {
    // Slice labels that fall within [start, end), rebased to 0
    let labels: Vec<(String, u16)> = all_labels.iter()
        .filter(|(_, f)| *f >= start && (end == u16::MAX || *f < end))
        .map(|(l, f)| (l.clone(), f.saturating_sub(start)))
        .collect();

    out.push(SplitAnim {
        fm_name:     fm_name.to_string(),
        source_anim: source_anim.to_string(),
        start_frame: start,
        end_frame:   end,
        labels,
        loop_tail,
        loop_frame,
    });
}
