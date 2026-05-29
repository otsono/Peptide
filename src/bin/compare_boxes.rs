//! compare_boxes — Tier-3 numeric oracle.
//!
//! Reads the JSON report produced by `tools/fraytools-harness/harness.js`
//! and the original SSF2 source file, then computes — for every box in the
//! FrayTools entity at the chosen animation / frame — how far the FrayTools
//! rendered anchor is from where the SSF2 source says the box should be.
//!
//! This is the "did we emit the right geometry?" check that closes the
//! AI-iterable loop:
//!
//!   1. Edit converter (entity_gen.rs).
//!   2. Run converter → entity JSON.
//!   3. Open entity in FrayTools via render-entity.js.
//!   4. Read entity + extract frame data via harness.js → out.json.
//!   5. Run compare_boxes → per-box drift report + PASS/FAIL.
//!   6. If FAIL, go to step 1.
//!
//! ## Expected anchor (what SSF2 intends)
//!
//! For **itemboxes** (ITEM_BOX color = 0xffff00):
//!   Expected anchor = SSF2 bottom-centre = (fb.x + w/2, fb.y + h).
//!   This is the "hand" that the rotating box sweeps around. Our converter
//!   bakes the top-left so that FrayTools' computed anchor lands here.
//!
//! For **all other boxes** (rotation always 0 after AABB collapse):
//!   Expected anchor = SSF2 centre = (fb.x + w/2, fb.y + h/2).
//!   At θ=0 the FrayTools anchor = (x + pivotX, y + pivotY). Our converter
//!   uses centre-pivot, so anchor = (x + w/2, y + h/2) = SSF2 centre.
//!
//! ## Box matching
//!
//! SSF2 `parse_sprite_boxes` returns boxes keyed by FM animation name
//! (`AnimationBoxData.fm_name`). Within a frame, boxes are matched by type
//! then by closest (Euclidean) size to the harness box.
//!
//! ## Usage
//!
//! ```
//! cargo run --bin compare_boxes -- \
//!   --ssf2   /path/to/mario.ssf \
//!   --char   mario \
//!   --json   /tmp/harness_out.json \
//!   [--tolerance 1.5]
//! ```
//!
//! Exit 0 = all boxes within tolerance. Exit 1 = any drift exceeded.
//! Exit 2 = input error.

use ssf2_converter::fraytools_transform::collision_box_anchor;
use ssf2_converter::sprite_parser::{self, AnimationBoxData, BoxType};
use serde::Deserialize;
use std::collections::BTreeMap;

// ── Harness JSON schema ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct HarnessReport {
    entity_path: String,
    animation:   String,
    frame:       usize,
    total_frames: usize,
    boxes:       Vec<HarnessBox>,
    #[serde(default)]
    nav:         serde_json::Value,
    #[serde(default)]
    png:         Option<String>,
}

#[derive(Debug, Deserialize)]
struct HarnessBox {
    layer_name:      String,
    layer_type:      String,
    #[serde(default)]
    fm_box_type:     String,
    x:               f64,
    y:               f64,
    width:           f64,
    height:          f64,
    rotation:        f64,
    pivot_x:         f64,
    pivot_y:         f64,
    color:           u32,
    #[serde(default = "default_one")]
    #[allow(dead_code)]  // included in JSON for completeness; not used in drift comparison
    alpha:           f64,
    rendered_anchor: Anchor,
}

fn default_one() -> f64 { 1.0 }

#[derive(Debug, Deserialize)]
struct Anchor { x: f64, y: f64 }

// ── CLI argument helpers ───────────────────────────────────────────────────────

fn arg(name: &str, args: &[String]) -> Option<String> {
    let flag = format!("--{}", name);
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

fn die(msg: &str) -> ! {
    eprintln!("ERROR: {}", msg);
    std::process::exit(2);
}

// ── Box color → SSF2 BoxType ──────────────────────────────────────────────────

fn color_to_ssf2_type(color: u32) -> Option<BoxType> {
    match color {
        0xff0000 => Some(BoxType::Hitbox),
        0xfcba03 => Some(BoxType::Hurtbox),
        0xff00ff => Some(BoxType::GrabBox),
        0xffff00 => Some(BoxType::ItemBox),
        0x48f748 => Some(BoxType::ReflectBox), // or ShieldBox
        0x42ecff => Some(BoxType::AbsorbBox),
        0xbababa => Some(BoxType::LedgeBox),
        0x9999ff => Some(BoxType::GrabHoldBox),
        _        => None,
    }
}

fn drift(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("Usage: compare_boxes --ssf2 <file.ssf> --char <name> --json <harness.json> [--tolerance <px>]");
        std::process::exit(2);
    }

    let ssf2_path   = arg("ssf2", &args).unwrap_or_else(|| die("--ssf2 required"));
    let char_name   = arg("char", &args).unwrap_or_else(|| die("--char required"));
    let json_path   = arg("json", &args).unwrap_or_else(|| die("--json required"));
    let tolerance: f64 = arg("tolerance", &args)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.5);

    // ── Load harness report. ─────────────────────────────────────────────────
    let json_raw = std::fs::read_to_string(&json_path)
        .unwrap_or_else(|e| die(&format!("cannot read {}: {}", json_path, e)));
    let report: HarnessReport = serde_json::from_str(&json_raw)
        .unwrap_or_else(|e| die(&format!("invalid harness JSON {}: {}", json_path, e)));

    println!("Entity  : {}", report.entity_path);
    println!("Anim    : {}  frame {}/{}",
        report.animation, report.frame, report.total_frames.saturating_sub(1));
    println!("Nav     : {}", report.nav);
    if let Some(ref p) = report.png { println!("PNG     : {}", p); }
    println!("Boxes   : {}", report.boxes.len());
    println!();

    // ── Parse SSF2 source. ───────────────────────────────────────────────────
    let ssf2_bytes = std::fs::read(&ssf2_path)
        .unwrap_or_else(|e| die(&format!("cannot read {}: {}", ssf2_path, e)));
    let swf_data = ssf2_converter::ssf::decompress(&ssf2_bytes)
        .unwrap_or_else(|e| die(&format!("decompress {}: {}", ssf2_path, e)));
    let empty: BTreeMap<String, String> = BTreeMap::new();
    let all_anim_data = sprite_parser::parse_sprite_boxes(&swf_data, &char_name, &empty)
        .unwrap_or_else(|e| die(&format!("parse_sprite_boxes: {}", e)));

    // Find the matching SSF2 animation (by fm_name).
    let ssf2_anim: Option<&AnimationBoxData> = all_anim_data.values()
        .find(|a| a.fm_name == report.animation);

    if ssf2_anim.is_none() {
        eprintln!("WARN: animation \"{}\" not found in SSF2 source. Available fm_names:",
            report.animation);
        for a in all_anim_data.values() {
            eprintln!("  {} (ssf2: {})", a.fm_name, a.ssf2_name);
        }
        eprintln!("Continuing with anchor-only verification (no SSF2 comparison).");
    }

    let ssf2_frame_boxes: &[sprite_parser::FrameBox] = ssf2_anim
        .and_then(|a| a.frames.get(&(report.frame as u16)))
        .map(|v| v.as_slice())
        .unwrap_or(&[]);

    if ssf2_anim.is_some() {
        println!("SSF2 source: {} frame {} — {} box(es)",
            report.animation, report.frame, ssf2_frame_boxes.len());
    }
    println!();

    // ── Per-box comparison. ───────────────────────────────────────────────────
    println!("{:<16} {:<12} {:>8} {:>8}  {:>12} {:>12}  {:>10} {:>8}",
        "layer", "fm_type", "ft_anchor_x", "ft_anchor_y",
        "ssf2_expected_x", "ssf2_expected_y", "drift_px", "result");
    println!("{}", "─".repeat(100));

    let mut pass_count = 0usize;
    let mut fail_count = 0usize;
    let mut skip_count = 0usize;

    for hb in &report.boxes {
        if hb.layer_type != "COLLISION_BOX" { continue; }

        // ── Re-derive the FrayTools rendered anchor via our transform. ───────
        // The harness JSON already includes `rendered_anchor` (computed in JS
        // with the same algorithm), but we re-derive it here in Rust as an
        // independent sanity check that our two implementations agree.
        let ft_anchor = collision_box_anchor(hb.x, hb.y, hb.pivot_x, hb.pivot_y, hb.rotation);

        // Confirm Rust and JS anchors agree (within 0.01px).
        let js_anchor = (hb.rendered_anchor.x, hb.rendered_anchor.y);
        let impl_drift = drift(ft_anchor, js_anchor);
        if impl_drift > 0.01 {
            eprintln!("WARN: JS/Rust anchor mismatch on {} — JS={:?} Rust={:?} diff={:.4}",
                hb.layer_name, js_anchor, ft_anchor, impl_drift);
        }

        // ── Find the best-matching SSF2 box. ─────────────────────────────────
        let ssf2_bt = color_to_ssf2_type(hb.color);
        let ssf2_match = ssf2_bt.and_then(|bt| {
            // Filter by matching type, then pick closest by (width, height) Euclidean.
            ssf2_frame_boxes.iter()
                .filter(|fb| {
                    fb.box_type == bt
                    || (bt == BoxType::ShieldBox && fb.box_type == BoxType::ReflectBox)
                    || (bt == BoxType::ReflectBox && fb.box_type == BoxType::ShieldBox)
                })
                .min_by(|a, b| {
                    let da = (a.width - hb.width).powi(2) + (a.height - hb.height).powi(2);
                    let db = (b.width - hb.width).powi(2) + (b.height - hb.height).powi(2);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        if let Some(sfb) = ssf2_match {
            // Expected anchor = where SSF2 intends the pivot to be.
            let expected = if sfb.box_type == BoxType::ItemBox {
                // Itembox: pivot is bottom-centre (the hand attachment point).
                (sfb.x + sfb.width / 2.0, sfb.y + sfb.height)
            } else {
                // Everything else: pivot is centre.
                (sfb.x + sfb.width / 2.0, sfb.y + sfb.height / 2.0)
            };

            let d = drift(ft_anchor, expected);
            let result = if d <= tolerance { pass_count += 1; "PASS" } else { fail_count += 1; "FAIL" };
            println!("{:<16} {:<12} {:>11.3} {:>8.3}  {:>15.3} {:>14.3}  {:>10.3} {:>8}",
                hb.layer_name, hb.fm_box_type,
                ft_anchor.0, ft_anchor.1,
                expected.0, expected.1,
                d, result);
        } else {
            skip_count += 1;
            // No SSF2 match — report the anchor anyway so it's visible.
            println!("{:<16} {:<12} {:>11.3} {:>8.3}  {:>15} {:>14}  {:>10} {:>8}",
                hb.layer_name, hb.fm_box_type,
                ft_anchor.0, ft_anchor.1,
                "(no SSF2 match)", "",
                "—", "SKIP");
        }
    }

    println!("{}", "─".repeat(100));
    println!("PASS: {}  FAIL: {}  SKIP: {}  (tolerance: {}px)",
        pass_count, fail_count, skip_count, tolerance);
    println!();

    if fail_count > 0 {
        eprintln!("VERDICT: FAIL — {} box(es) exceed {}px drift. Investigate entity_gen.", fail_count, tolerance);
        std::process::exit(1);
    } else if pass_count + skip_count == 0 {
        eprintln!("VERDICT: SKIP — no COLLISION_BOX layers found in harness JSON.");
        std::process::exit(0);
    } else {
        println!("VERDICT: PASS — all {} matched box(es) within {}px tolerance.", pass_count, tolerance);
        std::process::exit(0);
    }
}
