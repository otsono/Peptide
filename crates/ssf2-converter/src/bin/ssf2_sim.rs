//! ssf2_sim — compute SSF2 ground-truth motion + derived Fraymakers stats for a
//! character .ssf, using the single tunable `size_multiplier` knob.
//!
//! Usage:
//!   ssf2_sim <file.ssf> [charName] [--size-mult N] [--ssf2-fps N] [--fm-fps N]
//!
//! Prints: raw constants → simulated ground truth (px/s, apex px, body-relative)
//! → derived FM stat values, with the velocity/accel scale factors shown.

use ssf2_converter::physics_sim::*;
use ssf2_converter::*;
use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;

fn raw_stats_for(path: &std::path::Path, name_override: Option<&str>)
    -> Option<(String, BTreeMap<String, f64>)>
{
    let bytes = std::fs::read(path).ok()?;
    let swf_bytes = ssf::decompress(&bytes).ok()?;
    let swf = swf_parser::parse(&swf_bytes).ok()?;
    for abc_bytes in &swf.abc_blocks {
        let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };
        let mut names: Vec<String> = Vec::new();
        if let Some(n) = name_override {
            names.push(n.to_string());
        } else if let Some(meta) = abc_parser::extract_main_package_metadata(&abc) {
            for (id, _m) in &meta.characters { names.push(id.clone()); }
        }
        for name in &names {
            if let Ok(ch) = abc_parser::extract_character(&abc, name) {
                if let Some(s) = ch.stats {
                    return Some((name.clone(), s.values));
                }
            }
        }
    }
    None
}

fn main() {
    let mut args = env::args().skip(1).peekable();
    let mut positional: Vec<String> = Vec::new();
    let mut sp = ScaleParams::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--size-mult" => sp.size_multiplier = args.next().and_then(|v| v.parse().ok()).unwrap_or(sp.size_multiplier),
            "--ssf2-fps" => sp.ssf2_fps = args.next().and_then(|v| v.parse().ok()).unwrap_or(sp.ssf2_fps),
            "--fm-fps" => sp.fm_fps = args.next().and_then(|v| v.parse().ok()).unwrap_or(sp.fm_fps),
            _ => positional.push(a),
        }
    }
    if positional.is_empty() {
        eprintln!("usage: ssf2_sim <file.ssf> [charName] [--size-mult N]");
        std::process::exit(2);
    }
    let path = PathBuf::from(&positional[0]);
    let name_override = positional.get(1).map(|s| s.as_str());

    let Some((name, raw)) = raw_stats_for(&path, name_override) else {
        eprintln!("could not extract stats from {}", path.display());
        std::process::exit(1);
    };

    let p = Ssf2Physics::from_raw(&raw);
    let m = simulate(&p, &sp);
    let fm = derive_fm_stats(&p, &sp);

    println!("# SSF2 → Fraymakers physics report :: {name}");
    println!();
    println!("## Scale knob");
    println!("  size_multiplier = {}", sp.size_multiplier);
    println!("  ssf2_fps={}  fm_fps={}  fps_ratio={:.4}", sp.ssf2_fps, sp.fm_fps, sp.fps_ratio());
    println!("  -> velocity_scale = size_mult * fps_ratio      = {:.4}", sp.velocity_scale());
    println!("  -> accel_scale    = size_mult * fps_ratio^2    = {:.4}", sp.accel_scale());
    println!();
    println!("## Raw SSF2 constants (px/frame @ {}fps)", sp.ssf2_fps);
    println!("  walk cap norm_xSpeed={}  dash max_xSpeed={}", p.norm_x_speed, p.max_x_speed);
    println!("  jumpSpeed={}  midair={}  shortHop={}  startup={}f  midairJumps={}",
        p.jump_speed, p.jump_speed_midair, p.short_hop_speed, p.jump_startup, p.max_jump);
    println!("  gravity={}  terminal max_ySpeed={}  fastFall={}", p.gravity, p.max_y_speed, p.fast_fall_speed);
    println!("  accel_rate={}  accel_air={}  decel(fric)={}  decel_air={}",
        p.accel_rate, p.accel_rate_air, p.decel_rate, p.decel_rate_air);
    println!("  body width={}px height={}px", p.width, p.height);
    println!();
    println!("## SSF2 GROUND TRUTH (simulated @ {}fps)", sp.ssf2_fps);
    println!("  walk top speed   = {:.1} px/s  ({:.2} body-widths/s)", m.walk_top_pps, m.walk_top_bw);
    println!("  dash top speed   = {:.1} px/s", m.dash_top_pps);
    println!("  walk accel       = {:.0} frames to cap", m.walk_accel_frames);
    println!("  full jump apex   = {:.1} px  ({:.2} body-heights)", m.full_jump_apex_px, m.full_jump_apex_bh);
    println!("  full jump rise   = {:.3} s   air time {:.3} s", m.full_jump_rise_s, m.full_jump_air_s);
    println!("  short hop apex   = {:.1} px", m.short_hop_apex_px);
    println!("  double jump apex = {:.1} px", m.double_jump_apex_px);
    println!("  terminal vel     = {:.1} px/s", m.terminal_pps);
    println!("  fast-fall vel    = {:.1} px/s", m.fast_fall_pps);
    println!();
    println!("## DERIVED FRAYMAKERS STATS (one-knob scaling)");
    println!("  baseScaleX/Y       = {:.3}", fm.base_scale);
    println!("  gravity            = {:.4}", fm.gravity);
    println!("  terminalVelocity   = {:.4}", fm.terminal_velocity);
    println!("  fastFallSpeed      = {:.4}", fm.fast_fall_speed);
    println!("  jumpSpeed          = {:.4}", fm.jump_speed);
    println!("  doubleJumpSpeed    = {:.4}", fm.double_jump_speed);
    println!("  shortHopSpeed      = {:.4}", fm.short_hop_speed);
    println!("  walkSpeedCap       = {:.4}", fm.walk_speed_cap);
    println!("  dashSpeed          = {:.4}", fm.dash_speed);
    println!("  walkSpeedAccel     = {:.4}", fm.walk_accel);
    println!("  aerialSpeedAccel   = {:.4}", fm.aerial_accel);
    println!("  friction           = {:.4}", fm.friction);
    println!("  aerialFriction     = {:.4}", fm.aerial_friction);
}
