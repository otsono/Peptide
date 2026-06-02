//! physics_sim — deterministic re-implementation of SSF2's per-frame physics
//! integration, plus the SSF2 → Fraymakers scaling derivation.
//!
//! WHY THIS EXISTS
//! ---------------
//! SSF2 stat constants (norm_xSpeed, jumpSpeed, gravity, …) are expressed in
//! SSF2 pixels-per-frame at **30 fps**. Fraymakers runs at **60 fps** and
//! renders the same sprite at `size_multiplier`× scale. To make a converted
//! character *move the same way relative to its own body*, every motion value
//! must be rescaled by a factor that is fully determined by two things:
//!
//!   * the spatial scale  `size_multiplier`  (how much bigger the sprite is in
//!     Fraymakers — currently ~1.9, but an experimental knob, see below), and
//!   * the frame-rate ratio `ssf2_fps / fm_fps` (= 30/60 = 0.5, fixed physics).
//!
//! A velocity in px/frame and an acceleration in px/frame² scale differently
//! under a frame-rate change, so we derive TWO factors from the one knob:
//!
//!   velocity_scale = size_multiplier * (ssf2_fps / fm_fps)        // ≈ 1.9*0.5 = 0.95
//!   accel_scale    = size_multiplier * (ssf2_fps / fm_fps)^2      // ≈ 1.9*0.25 = 0.475
//!
//! `size_multiplier` is the SINGLE tunable knob. Tweak it (in stats.jsonc or
//! [`ScaleParams`]) and *everything* derived here — walk/dash/jump speeds,
//! gravity, friction, accel — rescales consistently. Nothing else is a magic
//! number.
//!
//! The simulator below integrates the raw SSF2 constants exactly as the engine
//! does (verified against the decompiled SSF2 physics; see
//! docs/ssf2-physics-model.md) so we get engine-independent GROUND TRUTH motion
//! (top speeds in px/s, jump apex in px, airtimes in seconds, and body-relative
//! equivalents) that the live Fraymakers measurement is checked against.

use std::collections::BTreeMap;

/// Frame-rate + spatial-scale parameters that drive the SSF2→FM conversion.
/// The whole point: `size_multiplier` is the one knob to tweak.
#[derive(Debug, Clone, Copy)]
pub struct ScaleParams {
    /// Spatial scale: how much larger the sprite renders in Fraymakers than in
    /// SSF2. The experimental knob. (Was hardcoded 1.9.)
    pub size_multiplier: f64,
    /// SSF2 simulation rate (Hz). Fixed at 30 for SSF2 Beta.
    pub ssf2_fps: f64,
    /// Fraymakers simulation rate (Hz). Fixed at 60.
    pub fm_fps: f64,
}

impl Default for ScaleParams {
    fn default() -> Self {
        ScaleParams { size_multiplier: 1.9, ssf2_fps: 30.0, fm_fps: 60.0 }
    }
}

impl ScaleParams {
    /// fps ratio (ssf2/fm). 0.5 for 30→60.
    pub fn fps_ratio(&self) -> f64 { self.ssf2_fps / self.fm_fps }

    /// Multiplier for a per-frame VELOCITY (px/frame): jump speeds, walk/dash
    /// caps, fall/terminal speeds, fast-fall, initial dash speed, roll speeds.
    pub fn velocity_scale(&self) -> f64 { self.size_multiplier * self.fps_ratio() }

    /// Multiplier for a per-frame ACCELERATION (px/frame²): gravity, walk/run
    /// accel, ground/air friction (decel), air mobility.
    pub fn accel_scale(&self) -> f64 { self.size_multiplier * self.fps_ratio().powi(2) }
}

/// Raw SSF2 physics constants for one character (subset we simulate), pulled
/// straight from the bytecode (see `dump_raw_stats`). Units: SSF2 px/frame@30,
/// px/frame²@30, or pixels. Missing keys fall back to documented defaults.
#[derive(Debug, Clone)]
pub struct Ssf2Physics {
    pub gravity: f64,          // px/frame² down (positive = downward)
    pub max_y_speed: f64,      // terminal fall speed cap (px/frame)
    pub fast_fall_speed: f64,  // fast-fall speed (px/frame)
    pub jump_speed: f64,       // initial ground jump velocity magnitude (px/frame)
    pub jump_speed_midair: f64,// midair (double) jump velocity magnitude
    pub short_hop_speed: f64,  // short-hop initial velocity magnitude
    pub max_jump: i32,         // number of MIDAIR jumps (SSF2 convention)
    pub jump_startup: i32,     // jump-squat frames before launch
    pub norm_x_speed: f64,     // walk top speed cap (px/frame)
    pub max_x_speed: f64,      // dash/run top speed cap (px/frame)
    pub accel_rate: f64,       // ground walk acceleration (px/frame²)
    pub accel_rate_air: f64,   // air horizontal acceleration / mobility
    pub accel_start: f64,      // initial walk speed on press
    pub accel_start_dash: f64, // initial dash speed on press
    pub decel_rate: f64,       // ground friction (px/frame², stored negative)
    pub decel_rate_air: f64,   // air friction (px/frame², stored negative)
    pub width: f64,            // hurtbox width (px) — body-relative normaliser
    pub height: f64,           // hurtbox height (px)
}

fn g(m: &BTreeMap<String, f64>, k: &str, dflt: f64) -> f64 {
    m.get(k).copied().unwrap_or(dflt)
}

impl Ssf2Physics {
    /// Build from the raw extracted stat map (keys as in `dump_raw_stats`).
    pub fn from_raw(m: &BTreeMap<String, f64>) -> Self {
        Ssf2Physics {
            gravity:           g(m, "gravity", 1.0),
            max_y_speed:       g(m, "max_ySpeed", 14.0),
            fast_fall_speed:   g(m, "fastFallSpeed", 16.0),
            jump_speed:        g(m, "jumpSpeed", 15.0),
            jump_speed_midair: g(m, "jumpSpeedMidair", g(m, "jumpSpeed", 15.0)),
            short_hop_speed:   g(m, "shortHopSpeed", 6.5),
            max_jump:          g(m, "max_jump", 1.0) as i32,
            jump_startup:      g(m, "jumpStartup", 2.0) as i32,
            norm_x_speed:      g(m, "norm_xSpeed", 8.0),
            max_x_speed:       g(m, "max_xSpeed", g(m, "norm_xSpeed", 8.0)),
            accel_rate:        g(m, "accel_rate", 1.0),
            accel_rate_air:    g(m, "accel_rate_air", 0.7),
            accel_start:       g(m, "accel_start", 0.0),
            accel_start_dash:  g(m, "accel_start_dash", 0.0),
            decel_rate:        g(m, "decel_rate", -1.0),
            decel_rate_air:    g(m, "decel_rate_air", -0.15),
            width:             g(m, "width", 24.0),
            height:            g(m, "height", 52.0),
        }
    }
}

/// Ground-truth motion metrics produced by simulating the raw constants.
/// `*_pps` are pixels-per-SECOND (frame-rate independent). `*_px` are pixel
/// distances. `*_s` are seconds. `*_bw` / `*_bh` are body-relative (per body
/// width / height) — the engine-independent invariants we want to preserve.
#[derive(Debug, Clone, Default)]
pub struct MotionMetrics {
    // Horizontal
    pub walk_top_pps: f64,
    pub walk_accel_frames: f64,   // frames@30 to reach walk cap from rest
    pub dash_top_pps: f64,
    // Vertical / jumps
    pub full_jump_apex_px: f64,
    pub full_jump_rise_s: f64,    // seconds rest→apex
    pub full_jump_air_s: f64,     // seconds full hop (launch→land at start height)
    pub short_hop_apex_px: f64,
    pub double_jump_apex_px: f64,
    pub terminal_pps: f64,
    pub fast_fall_pps: f64,
    // Body-relative invariants
    pub walk_top_bw: f64,         // body-widths / sec
    pub full_jump_apex_bh: f64,   // body-heights of apex
    // Bookkeeping
    pub body_w: f64,
    pub body_h: f64,
}

/// Simulate a jump given an initial upward speed magnitude `v0` (px/frame).
/// Returns (apex_height_px, rise_frames, total_air_frames) integrating exactly
/// as the engine does: each frame v += gravity (clamped to terminal on the way
/// down), then position += v. Y is positive-down, so jumping is negative v.
fn simulate_jump(p: &Ssf2Physics, v0: f64) -> (f64, f64, f64) {
    // Guard: gravity must be a positive, plausible value. Several SSF2 characters
    // store gravity in a form the linear stat scan doesn't capture, leaving it 0
    // (the converter then falls back to a template default). With g<=0 the jump
    // never returns, so report NaN rather than running to the iteration cap.
    if !(p.gravity > 0.0) {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let mut y = 0.0f64;       // displacement from launch (down positive)
    let mut v = -v0;          // upward
    let mut min_y = 0.0f64;   // most-negative (highest)
    let mut rise_frames = 0.0f64;
    let mut frames = 0.0f64;
    let mut reached_apex = false;
    // Integrate until the character returns to (or below) launch height.
    // Cap iterations to avoid runaways on degenerate inputs.
    for _ in 0..100_000 {
        v += p.gravity;
        if v > p.max_y_speed { v = p.max_y_speed; }
        y += v;
        frames += 1.0;
        if y < min_y { min_y = y; }
        if !reached_apex {
            if v >= 0.0 { reached_apex = true; rise_frames = frames; }
        }
        if reached_apex && y >= 0.0 { break; }
    }
    (-min_y, rise_frames, frames)
}

/// Frames to accelerate from `start` to `cap` at `accel` (px/frame²).
fn frames_to_cap(start: f64, cap: f64, accel: f64) -> f64 {
    if accel <= 0.0 || start >= cap { return 0.0; }
    ((cap - start) / accel).ceil()
}

/// Run the full ground-truth simulation for a character.
pub fn simulate(p: &Ssf2Physics, sp: &ScaleParams) -> MotionMetrics {
    let fps = sp.ssf2_fps;
    let mut m = MotionMetrics::default();
    m.body_w = p.width;
    m.body_h = p.height;

    // Horizontal top speeds: the engine accelerates to the cap and holds it,
    // so sustained top speed == the cap (in px/frame), converted to px/s.
    m.walk_top_pps = p.norm_x_speed * fps;
    m.dash_top_pps = p.max_x_speed * fps;
    m.walk_accel_frames = frames_to_cap(p.accel_start, p.norm_x_speed, p.accel_rate);

    // Vertical.
    let (apex, rise_f, air_f) = simulate_jump(p, p.jump_speed);
    m.full_jump_apex_px = apex;
    m.full_jump_rise_s = rise_f / fps;
    m.full_jump_air_s = air_f / fps;
    m.short_hop_apex_px = simulate_jump(p, p.short_hop_speed).0;
    m.double_jump_apex_px = simulate_jump(p, p.jump_speed_midair).0;
    m.terminal_pps = p.max_y_speed * fps;
    m.fast_fall_pps = p.fast_fall_speed * fps;

    // Body-relative invariants.
    if p.width > 0.0 { m.walk_top_bw = m.walk_top_pps / p.width; }
    if p.height > 0.0 { m.full_jump_apex_bh = apex / p.height; }
    m
}

/// The Fraymakers stat values derived from raw SSF2 constants via the single
/// `size_multiplier` knob. Per-frame velocities use `velocity_scale`, per-frame
/// accelerations use `accel_scale`. Y-velocities (jump) are emitted as the
/// magnitude (the converter applies the FM sign convention).
#[derive(Debug, Clone, Default)]
pub struct ScaledFmStats {
    pub gravity: f64,
    pub terminal_velocity: f64,
    pub fast_fall_speed: f64,
    pub jump_speed: f64,
    pub double_jump_speed: f64,
    pub short_hop_speed: f64,
    pub walk_speed_cap: f64,
    pub dash_speed: f64,
    pub walk_accel: f64,
    pub aerial_accel: f64,
    pub friction: f64,
    pub aerial_friction: f64,
    pub base_scale: f64,
    /// The two derived factors, surfaced for reporting/debugging.
    pub velocity_scale: f64,
    pub accel_scale: f64,
}

/// Derive Fraymakers stats from raw SSF2 constants using only `sp` (the knob).
pub fn derive_fm_stats(p: &Ssf2Physics, sp: &ScaleParams) -> ScaledFmStats {
    let vs = sp.velocity_scale();
    let as_ = sp.accel_scale();
    ScaledFmStats {
        gravity:            p.gravity * as_,
        terminal_velocity:  p.max_y_speed * vs,
        fast_fall_speed:    p.fast_fall_speed * vs,
        jump_speed:         p.jump_speed * vs,
        double_jump_speed:  p.jump_speed_midair * vs,
        short_hop_speed:    p.short_hop_speed * vs,
        walk_speed_cap:     p.norm_x_speed * vs,
        dash_speed:         p.max_x_speed * vs,
        walk_accel:         p.accel_rate * as_,
        aerial_accel:       p.accel_rate_air * as_,
        friction:           p.decel_rate.abs() * as_,
        aerial_friction:    p.decel_rate_air.abs() * as_,
        base_scale:         sp.size_multiplier,
        velocity_scale:     vs,
        accel_scale:        as_,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn sandbag() -> BTreeMap<String, f64> {
        let mut m = BTreeMap::new();
        for (k, v) in [
            ("gravity", 1.2), ("max_ySpeed", 14.0), ("fastFallSpeed", 16.0),
            ("jumpSpeed", 15.0), ("jumpSpeedMidair", 15.0), ("shortHopSpeed", 6.5),
            ("max_jump", 1.0), ("jumpStartup", 2.0), ("norm_xSpeed", 8.0),
            ("max_xSpeed", 8.0), ("accel_rate", 1.1), ("accel_rate_air", 0.7),
            ("accel_start", 0.0), ("accel_start_dash", 15.7), ("decel_rate", -0.97),
            ("decel_rate_air", -0.15), ("width", 24.0), ("height", 52.0),
        ] { m.insert(k.to_string(), v); }
        m
    }

    #[test]
    fn sandbag_ground_truth() {
        let p = Ssf2Physics::from_raw(&sandbag());
        let sp = ScaleParams::default();
        let m = simulate(&p, &sp);
        // Walk: 8 px/frame * 30 = 240 px/s.
        assert!((m.walk_top_pps - 240.0).abs() < 1e-6);
        // Terminal: 14 * 30 = 420 px/s.
        assert!((m.terminal_pps - 420.0).abs() < 1e-6);
        // Jump apex: continuous estimate v0^2/2g = 225/2.4 = 93.75 px; discrete
        // semi-implicit Euler (gravity-then-move) gives ~86.4. Exact value
        // depends on the engine's intra-frame order (see ssf2-physics-model.md);
        // bound it to the physically-reasonable band both orders bracket.
        assert!(m.full_jump_apex_px > 80.0 && m.full_jump_apex_px < 100.0,
            "apex={}", m.full_jump_apex_px);
        // ~1.6–1.9 body-heights.
        assert!(m.full_jump_apex_bh > 1.5 && m.full_jump_apex_bh < 2.0,
            "apex_bh={}", m.full_jump_apex_bh);
    }

    #[test]
    fn scale_factors_track_the_one_knob() {
        let mut sp = ScaleParams::default();
        assert!((sp.velocity_scale() - 0.95).abs() < 1e-9);
        assert!((sp.accel_scale() - 0.475).abs() < 1e-9);
        // Tweak the single knob → both factors move together.
        sp.size_multiplier = 2.0;
        assert!((sp.velocity_scale() - 1.0).abs() < 1e-9);
        assert!((sp.accel_scale() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn derived_stats_scale_with_knob() {
        let p = Ssf2Physics::from_raw(&sandbag());
        let s1 = derive_fm_stats(&p, &ScaleParams::default());
        // jump 15 * 0.95 = 14.25, gravity 1.2 * 0.475 = 0.57.
        assert!((s1.jump_speed - 14.25).abs() < 1e-6, "{}", s1.jump_speed);
        assert!((s1.gravity - 0.57).abs() < 1e-6, "{}", s1.gravity);
        assert!((s1.walk_speed_cap - 7.6).abs() < 1e-6, "{}", s1.walk_speed_cap);
        // Apex preserved in body-heights: FM apex / FM-scaled body-height should
        // equal SSF2 apex / SSF2 body-height (scale-invariant).
        let m_ss = simulate(&p, &ScaleParams::default());
        let fm_p = Ssf2Physics {
            gravity: s1.gravity, max_y_speed: s1.terminal_velocity,
            jump_speed: s1.jump_speed, height: p.height * s1.base_scale,
            ..p.clone()
        };
        // Simulate the FM-scaled jump at 60fps semantics: same integration, the
        // apex in px should be ~base_scale × the SSF2 apex.
        let fm_apex = simulate_jump(&fm_p, fm_p.jump_speed).0;
        let ratio = fm_apex / m_ss.full_jump_apex_px;
        assert!((ratio - s1.base_scale).abs() < 0.1,
            "apex ratio {} vs base_scale {}", ratio, s1.base_scale);
    }
}
