//! A speed written into a move has to be converted, not copied.
//!
//! `setXSpeed(17)` in an SSF2 move means "move 17 source pixels this frame", and neither half of
//! that means the same thing after conversion: the frame is one of sixty rather than one of
//! thirty, and the pixel belongs to a world scaled by `size_multiplier`. Copying the number
//! through leaves the move covering the right ground per frame across twice as many frames, so it
//! travels half again as far as it should. Measured live on a side special before this was fixed:
//! 2.0x the source distance where 1.3x is correct.
//!
//! The conversion happens at the BOUNDARY, not in the script's arithmetic: the decompiler leaves
//! every speed as the source wrote it, the translation wraps each write in `__svx`/`__svy` and
//! each read in `__isv`, and those helpers carry the scale. So a speed read back, kept in a
//! local and written again crosses once each way and comes out as it went in (ness's rocket
//! bounced at two thirds of its speed while the decompiler scaled the write as well).
//!
//! The DIRECTION has to be converted too. SSF2's setXSpeed is world-space and Fraymakers' is
//! facing-relative -- measured in both engines: facing LEFT, SSF2's setXSpeed(15) moves the
//! character right and Fraymakers' moves it left -- so the argument is wrapped in `flipX`, which
//! turns a world-space number into the facing-relative one Fraymakers wants.

use ssf2_converter::mappings::character_stats;

fn scale() -> f64 { character_stats().scaling.velocity_scale() }

#[test]
fn an_authored_speed_keeps_the_source_number() {
    let out = ssf2_converter::decompiler::render_call_for_test("setXSpeed", &["17"]);
    assert_eq!(out, "self.setXSpeed(self.flipX(17))", "the source's number, facing-relative");
}

#[test]
fn a_source_speed_held_in_a_variable_is_left_alone_too() {
    // sandbag's aerial down special dashes at `leftSpeed`/`rightSpeed`, set to -15 and 15 in its
    // constructor: the variable holds a source number, and the boundary converts it on the write
    let out = ssf2_converter::decompiler::render_call_for_test("setXSpeed", &["leftSpeed.get()"]);
    assert_eq!(out, "self.setXSpeed(self.flipX(leftSpeed.get()))", "no scale inlined: {out}");
}

#[test]
fn a_computed_speed_is_left_alone() {
    // a speed read back is reported facing-relative already, so it takes no flip either
    let out = ssf2_converter::decompiler::render_call_for_test("setXSpeed", &["self.getXSpeed()"]);
    assert_eq!(out, "self.setXSpeed(self.getXSpeed())", "a computed speed passes through: {out}");
}

#[test]
fn the_boundary_carries_the_scale() {
    // the write crosses through the helper, and the helper is where the factor lives
    let out = ssf2_converter::api_mappings::translate_ssf2_to_fm("self.setXSpeed(self.flipX(17));");
    assert!(out.contains("__svx(self.flipX(17))"), "the write crosses through __svx: {out}");
    let read = ssf2_converter::api_mappings::translate_ssf2_to_fm("var v = self.getXSpeed();");
    assert!(read.contains("__isv(self.getXSpeed())"), "the read crosses through __isv: {read}");
    let helpers = ssf2_converter::api_mappings::speed_boundary_helpers_for_test();
    let vs = scale();
    assert!(helpers.contains(&format!("function __sv(v) {{ return v * {vs:.4}; }}")), "{helpers}");
    assert!(helpers.contains(&format!("function __isv(v) {{ return v / {vs:.4}; }}")), "{helpers}");
}

#[test]
fn zero_stays_zero() {
    let out = ssf2_converter::decompiler::render_call_for_test("setXSpeed", &["0"]);
    assert_eq!(out, "self.setXSpeed(0)", "a stop is a stop in either engine, and needs no flip");
}
