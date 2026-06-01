//! A from-spec re-implementation of the small slice of FrayTools' render
//! transform that decides *where a collision box's registration anchor
//! lands on screen* once rotation is applied.
//!
//! This is OUR OWN CODE, written to match the BEHAVIOUR documented in
//! `docs/fraytools_internals.md` §1–§4 (observed by black-box RE for
//! interoperability). No FrayTools source is copied. It exists so the
//! `probe_itembox` binary can predict FrayTools' placement and prove
//! converter bugs by measurement instead of guessing — see the 5-commit
//! itembox churn in `git log`.
//!
//! ## The observed model (in our words)
//!
//! For a COLLISION_BOX keyframe with stored top-left `(x, y)`, pivot
//! offset `(pivotX, pivotY)` (relative to top-left, in pixels — NOT
//! scaled, unlike IMAGE), and `rotation` θ in degrees, FrayTools:
//!
//! 1. Negates the stored Y to get a render-space position `s = (x, −y)`
//!    and a render-space pivot `c = (pivotX, −pivotY)`.   [§1]
//! 2. Computes the registration anchor by rotating the pivot-offset
//!    vector around the position by `−θ` degrees and adding it:
//!        p = s + R(−θ)·c                                  [§2]
//!    (with an exact-multiple-of-360 fast path that skips the rotation).
//! 3. Negates the anchor's Y back into stored space: `p.y *= −1`.
//!
//! The consequence — and the whole reason the itembox is fiddly — is
//! that the anchor **moves as θ changes**. At θ=0 it sits at
//! `(x+pivotX, y+pivotY)` (the un-rotated pivot point); at θ≠0 it drifts.
//! So a box whose pivot is the "hand" does NOT keep the hand fixed under
//! rotation unless the converter compensates.

/// `(magnitude, angle_radians)` → cartesian. Matches FrayTools' `polar`.
fn polar(mag: f64, angle_rad: f64) -> (f64, f64) {
    (mag * angle_rad.cos(), mag * angle_rad.sin())
}

/// FrayTools `calculateAbsolutePivotPosition(pos, pivot, angle_deg)`:
/// rotate the `pivot` offset vector around the origin by `angle_deg`
/// and add to `pos`. Exact-multiple-of-360 short-circuits to `pos +
/// pivot` (FrayTools special-cases this; we mirror it so our output is
/// bit-for-bit aligned with the no-rotation path).
pub fn calculate_absolute_pivot_position(
    pos: (f64, f64),
    pivot: (f64, f64),
    angle_deg: f64,
) -> (f64, f64) {
    if angle_deg % 360.0 == 0.0 {
        return (pos.0 + pivot.0, pos.1 + pivot.1);
    }
    let mag = (pivot.0 * pivot.0 + pivot.1 * pivot.1).sqrt();
    let ang = pivot.1.atan2(pivot.0);
    let (rx, ry) = polar(mag, ang + angle_deg * std::f64::consts::PI / 180.0);
    (pos.0 + rx, pos.1 + ry)
}

/// Where FrayTools renders a COLLISION_BOX's registration anchor (the
/// pivot point), in STORED entity coordinates (Y-down, same space the
/// `.entity` keyframe `x`/`y` live in).
///
/// `x, y`           — stored keyframe top-left.
/// `pivot_x,pivot_y`— stored pivot offset from top-left (pixels).
/// `rotation_deg`   — stored keyframe rotation.
///
/// Returns the absolute anchor `(ax, ay)` in stored coordinates. For a
/// hand-anchored itembox (pivot = bottom-centre = (w/2, h)) this is
/// where the hand actually ends up on screen.
pub fn collision_box_anchor(
    x: f64, y: f64,
    pivot_x: f64, pivot_y: f64,
    rotation_deg: f64,
) -> (f64, f64) {
    // §1 negate Y into render space.
    let s = (x, -y);
    let c = (pivot_x, -pivot_y);
    // §2 anchor = s + R(-θ)·c.
    let p = calculate_absolute_pivot_position(s, c, -rotation_deg);
    // §3 negate Y back into stored space.
    (p.0, -p.1)
}

/// The position the converter *intends* the pivot point to occupy: the
/// un-rotated pivot point `(x + pivotX, y + pivotY)`. For an itembox
/// this is the hand attachment point. The bug is the gap between this
/// and `collision_box_anchor(...)` once rotation is non-zero.
pub fn intended_pivot_point(x: f64, y: f64, pivot_x: f64, pivot_y: f64) -> (f64, f64) {
    (x + pivot_x, y + pivot_y)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn approx(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    #[test]
    fn zero_rotation_anchor_equals_intended_pivot() {
        // θ=0: the rendered anchor sits exactly on the un-rotated pivot
        // point. (This is why non-rotated itemboxes have always looked
        // fine — the bug only bites rotated frames.)
        let (x, y, px, py) = (100.0, 50.0, 20.0, 40.0);
        let anchor = collision_box_anchor(x, y, px, py, 0.0);
        let intended = intended_pivot_point(x, y, px, py);
        assert!(approx(anchor, intended), "anchor {:?} vs intended {:?}", anchor, intended);
    }

    #[test]
    fn multiple_of_360_takes_fast_path() {
        let a = collision_box_anchor(10.0, 20.0, 5.0, 7.0, 360.0);
        let b = collision_box_anchor(10.0, 20.0, 5.0, 7.0, 0.0);
        assert!(approx(a, b), "360° must match 0°: {:?} vs {:?}", a, b);
    }

    #[test]
    fn ninety_degree_anchor_drifts_off_the_intended_pivot() {
        // The crux: at θ=90 the rendered anchor is NOT the intended pivot
        // point. Quantifies the drift so the probe + fix have a fixture.
        // s=(x,-y)=(0,0); c=(pivotX,-pivotY)=(10,-30).
        // R(-90)·c: cos(-90)=0, sin(-90)=-1 →
        //   rx = 0*10 - (-1)*(-30) = -30 ... using R(θ)=[[cosθ,-sinθ],[sinθ,cosθ]]
        //   Actually compute via the same routine to avoid sign slips:
        let x = 0.0; let y = 0.0; let px = 10.0; let py = 30.0;
        let anchor = collision_box_anchor(x, y, px, py, 90.0);
        let intended = intended_pivot_point(x, y, px, py); // (10, 30)
        assert!(!approx(anchor, intended),
            "expected drift at 90°, but anchor {:?} == intended {:?}", anchor, intended);
        // Drift magnitude is non-trivial (sanity: > 1px).
        let d = ((anchor.0 - intended.0).powi(2) + (anchor.1 - intended.1).powi(2)).sqrt();
        assert!(d > 1.0, "drift {:.3}px unexpectedly small", d);
    }

    #[test]
    fn anchor_to_keep_hand_fixed_is_recoverable() {
        // The fix direction: to keep the rendered anchor at a target
        // hand position H for a given θ, the converter must choose the
        // top-left so that collision_box_anchor(...) == H. We can solve
        // it by inverting: required_topleft = H - (anchor - topleft).
        // Verify the round-trip for a rotated case.
        let (px, py, theta) = (15.0, 30.0, 47.0);
        let hand = (123.0, -88.0);
        // anchor as a function of top-left is affine; offset = anchor(0,0,..).
        let off = collision_box_anchor(0.0, 0.0, px, py, theta);
        let topleft = (hand.0 - off.0, hand.1 - off.1);
        let got = collision_box_anchor(topleft.0, topleft.1, px, py, theta);
        assert!(approx(got, hand), "solved top-left should land hand at {:?}, got {:?}", hand, got);
    }
}
