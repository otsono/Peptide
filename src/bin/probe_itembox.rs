//! Tier-2 probe: does FrayTools render the itembox's hand-anchor where
//! the SSF2 source actually puts the hand?
//!
//! Ground truth comes from the SSF2 source (via `sprite_parser`), NOT
//! from the converted entity — so the measurement is independent of how
//! the converter chooses to emit the box. For each itembox FrameBox we:
//!
//!   1. Take the true hand = the box's bottom-centre in world space
//!      (`fb.x + w/2`, `fb.y + h`) — the hand attachment point SSF2
//!      places the item box around.
//!   2. RAW emission (what the converter did before the bake): emit
//!      top-left `(fb.x, fb.y)` with pivot = (w/2, h) and rotation θ.
//!      Compute FrayTools' rendered anchor and measure the drift from
//!      the true hand. (This is the bug.)
//!   3. BAKED emission (the fix): solve top-left = hand − anchor(0,0,
//!      pivot,θ), emit that, and re-measure. Drift must be ~0.
//!
//! This validates the bake math against every real itembox frame in a
//! character, with no entity parsing or frame-index alignment.
//!
//! Usage:  probe_itembox <file.ssf> [char_name] [max_rows]

use ssf2_converter::fraytools_transform::{collision_box_anchor, intended_pivot_point};
use ssf2_converter::sprite_parser::{self, BoxType};
use std::collections::BTreeMap;

fn drift(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn main() {
    let path = std::env::args().nth(1)
        .expect("usage: probe_itembox <file.ssf> [char_name] [max_rows]");
    let char_name = std::env::args().nth(2).unwrap_or_else(|| {
        std::path::Path::new(&path).file_stem().unwrap().to_string_lossy().into_owned()
    });
    let max_rows: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(12);

    let swf_data = ssf2_converter::ssf::decompress(&std::fs::read(&path).expect("read"))
        .expect("decompress");
    // The ssf2→fm anim map only labels animations; box geometry is
    // map-independent, so an empty map is fine for this probe.
    let empty: BTreeMap<String, String> = BTreeMap::new();
    let boxes = sprite_parser::parse_sprite_boxes(&swf_data, &char_name, &empty)
        .expect("parse_sprite_boxes");

    println!("entity: {} (char {})", path, char_name);
    println!("{:<18} {:>6} {:>7} {:>7} {:>7}  {:>10} {:>10}",
        "anim/frame", "rot°", "w", "h", "—", "RAW drift", "BAKED drift");
    println!("{}", "-".repeat(80));

    let mut rows = 0usize;
    let (mut n, mut n_rot) = (0usize, 0usize);
    let (mut max_raw, mut max_baked) = (0.0_f64, 0.0_f64);
    let mut baked_fail = 0usize;

    let mut anims: Vec<_> = boxes.keys().cloned().collect();
    anims.sort();
    for anim in anims {
        let data = &boxes[&anim];
        for (frame, fbs) in &data.frames {
            for fb in fbs {
                if fb.box_type != BoxType::ItemBox { continue; }
                n += 1;
                let w = fb.width; let h = fb.height;
                let (px, py) = (w / 2.0, h);                 // bottom-centre pivot
                let theta = ((fb.rotation % 360.0) + 360.0) % 360.0;
                let hand = intended_pivot_point(fb.x, fb.y, px, py); // true hand

                // RAW: emit raw top-left, measure rendered-anchor drift.
                let raw_anchor = collision_box_anchor(fb.x, fb.y, px, py, theta);
                let raw_drift = drift(raw_anchor, hand);

                // BAKED: solve top-left so the rendered anchor lands on the hand.
                let off = collision_box_anchor(0.0, 0.0, px, py, theta);
                let baked_tl = (hand.0 - off.0, hand.1 - off.1);
                let baked_anchor = collision_box_anchor(baked_tl.0, baked_tl.1, px, py, theta);
                let baked_drift = drift(baked_anchor, hand);

                if theta.abs() > 0.01 { n_rot += 1; }
                max_raw = max_raw.max(raw_drift);
                max_baked = max_baked.max(baked_drift);
                if baked_drift > 0.01 { baked_fail += 1; }

                if rows < max_rows && theta.abs() > 0.01 {
                    println!("{:<18} {:>6.1} {:>7.1} {:>7.1} {:>7}  {:>10.2} {:>10.4}",
                        format!("{}/{}", anim, frame), theta, w, h, "", raw_drift, baked_drift);
                    rows += 1;
                }
            }
        }
    }

    println!("{}", "-".repeat(80));
    println!("itembox frames: {n}  |  rotated: {n_rot}  |  max RAW drift: {max_raw:.2}px  |  max BAKED drift: {max_baked:.4}px");
    if baked_fail == 0 && max_raw > 0.5 {
        println!("VERDICT: bake fixes it. RAW emission drifts up to {max_raw:.1}px on rotated frames;");
        println!("         BAKED emission lands every hand within 0.01px of the SSF2 source.");
    } else if baked_fail > 0 {
        println!("VERDICT: BAKE INCOMPLETE — {baked_fail} frames still drift after baking. Investigate.");
    } else {
        println!("VERDICT: no rotated itemboxes / nothing to bake for {char_name}.");
    }
}
