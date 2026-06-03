//! Mario physics-stat parity guard. Locks in the movement stats that were verified
//! 1:1 against both the SSF2 ground-truth model (`peptide ssf2 stats mario.ssf`) and
//! the live Fraymakers engine (gravity/friction read off `physics`, walk-terminal off
//! `physics.currentVelocityX`). If a `stats.jsonc` multiplier or the scaling math drifts,
//! these break before the converter silently ships a Mario that moves differently.
//!
//! Skipped silently when the SSF2 corpus isn't on disk (matches `golden_sandbag.rs`).

use ssf2_converter::{run_conversion, ConvertOptions};
use std::path::PathBuf;

mod common;

fn mario_ssf_path() -> PathBuf {
    common::ssf("mario")
}

/// Pull `field: <number>,` out of the generated CharacterStats.hx (tolerant of a
/// trailing `/*TODO*/` marker and whitespace). Returns the parsed f64.
fn stat(body: &str, field: &str) -> f64 {
    for line in body.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix(field) {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix(':') {
                // take everything up to the first ',' then strip a possible /*...*/
                let val = rest.split(',').next().unwrap_or("");
                let val = val.split("/*").next().unwrap_or("").trim();
                return val.parse::<f64>()
                    .unwrap_or_else(|_| panic!("CharacterStats.{field}: cannot parse {val:?}"));
            }
        }
    }
    panic!("CharacterStats.hx has no `{field}:` line");
}

#[test]
fn mario_movement_stats_match_ssf2_ground_truth() {
    let ssf = mario_ssf_path();
    if !common::present(&ssf) { return; }

    let out = tempfile::tempdir().expect("tempdir");
    let mut opts = ConvertOptions::new(&ssf);
    opts.output = out.path().to_path_buf();
    run_conversion(opts).expect("run_conversion for mario.ssf");

    let cs = out.path().join("mario/library/scripts/Mario/CharacterStats.hx");
    let body = std::fs::read_to_string(&cs)
        .unwrap_or_else(|e| panic!("read {}: {e}", cs.display()));

    // (field, expected, tolerance). Expected values are the ones confirmed live in
    // Fraymakers and derived from the SSF2 physics model for size_multiplier=1.9.
    let checks: &[(&str, f64, f64)] = &[
        ("gravity",          0.62,  0.02),
        ("shortHopSpeed",    9.03,  0.05),
        ("jumpSpeed",        16.53, 0.05),
        ("terminalVelocity", 12.35, 0.05),
        ("fastFallSpeed",    15.2,  0.05),
        ("friction",         0.49,  0.02),
        ("walkSpeedCap",     3.8,   0.05),
        ("dashSpeed",        10.45, 0.05),
    ];
    let mut errs: Vec<String> = Vec::new();
    for (field, expected, tol) in checks {
        let got = stat(&body, field);
        if (got - expected).abs() > *tol {
            errs.push(format!("  {field}: got {got}, expected {expected} (±{tol})"));
        }
    }
    assert!(errs.is_empty(),
        "mario physics drifted from the SSF2-verified ground truth:\n{}\n\
         if this is an intentional re-tune of stats.jsonc, update the expected values here.",
        errs.join("\n"));
}
