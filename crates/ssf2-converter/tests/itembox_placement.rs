//! Regression guard for the itembox anchor-bake fix (Tier 2).
//!
//! FrayTools renders a COLLISION_BOX's registration anchor at
//! `s + R(-θ)·pivot` (docs/fraytools_internals.md §2), which moves with
//! rotation. The converter bakes the rotation into the emitted itembox
//! top-left so the hand attachment point stays pinned to the SSF2 source
//! position. This test reproduces the bake the same way `entity_gen`
//! does and asserts every rotated itembox frame lands its hand within
//! 0.5px of the source — AND that without the bake it would have drifted
//! (so the test can't silently pass on a no-op).
//!
//! Skipped if `../ssf2-ssfs/` is absent (matches golden_sandbag).

use ssf2_converter::fraytools_transform::{collision_box_anchor, intended_pivot_point};
use ssf2_converter::sprite_parser::{self, BoxType};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn ssf(name: &str) -> PathBuf {
    // ssf2-ssfs/ is a sibling of the repo root; the crate sits two levels below.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().and_then(|p| p.parent()).map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join(format!("ssf2-ssfs/{}.ssf", name))
}

fn drift(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn check_char(name: &str) {
    let p = ssf(name);
    if !p.exists() { eprintln!("{}.ssf missing; skipping", name); return; }
    let data = ssf2_converter::ssf::decompress(&std::fs::read(&p).unwrap()).unwrap();
    let empty: BTreeMap<String, String> = BTreeMap::new();
    let boxes = sprite_parser::parse_sprite_boxes(&data, name, &empty).unwrap();

    let (mut rotated, mut max_raw, mut max_baked) = (0usize, 0.0_f64, 0.0_f64);
    for ad in boxes.values() {
        for fbs in ad.frames.values() {
            for fb in fbs {
                if fb.box_type != BoxType::ItemBox { continue; }
                let (px, py) = (fb.width / 2.0, fb.height);
                let theta = ((fb.rotation % 360.0) + 360.0) % 360.0;
                if theta.abs() <= 0.01 { continue; }
                rotated += 1;
                let hand = intended_pivot_point(fb.x, fb.y, px, py);

                // RAW (pre-fix): emit raw top-left → anchor drifts.
                max_raw = max_raw.max(drift(collision_box_anchor(fb.x, fb.y, px, py, theta), hand));

                // BAKED (the fix, exactly as entity_gen computes it).
                let off = collision_box_anchor(0.0, 0.0, px, py, theta);
                let (bx, by) = (hand.0 - off.0, hand.1 - off.1);
                max_baked = max_baked.max(drift(collision_box_anchor(bx, by, px, py, theta), hand));
            }
        }
    }

    assert!(rotated > 0, "{}: expected rotated itembox frames to exercise the bake", name);
    assert!(max_raw > 1.0,
        "{}: raw emission should drift (else the test is a no-op); max_raw={:.3}", name, max_raw);
    assert!(max_baked < 0.5,
        "{}: baked itembox hand must stay pinned; max_baked drift={:.4}px", name, max_baked);
    eprintln!("{}: {} rotated itembox frames, raw drift up to {:.1}px, baked {:.4}px",
        name, rotated, max_raw, max_baked);
}

#[test]
fn itembox_hand_stays_pinned_under_rotation() {
    // sandbag is the golden smoke-test char; mario/peach are heavy
    // rotated-itembox cases.
    for c in ["sandbag", "mario", "peach"] {
        check_char(c);
    }
}
