//! Tests for the geometry primitives in `sprite_parser` and
//! `image_extractor` — matrix decomposition, box-type classification,
//! box-name normalization, and shear detection.

use ssf2_converter::sprite_parser::BoxType;
use ssf2_converter::image_extractor::ImageLocalMatrix;

// ─── BoxType::from_instance_name ─────────────────────────────────────────

#[test]
fn box_type_attackbox_is_hitbox() {
    assert_eq!(BoxType::from_instance_name("attackBox"), Some(BoxType::Hitbox));
    assert_eq!(BoxType::from_instance_name("attackBox2"), Some(BoxType::Hitbox));
    assert_eq!(BoxType::from_instance_name("attackBox12"), Some(BoxType::Hitbox));
}

#[test]
fn box_type_hitbox_is_hurtbox() {
    // SSF2 "hit" semantics = FM "hurt".
    assert_eq!(BoxType::from_instance_name("hitBox"), Some(BoxType::Hurtbox));
    assert_eq!(BoxType::from_instance_name("hitBox3"), Some(BoxType::Hurtbox));
    assert_eq!(BoxType::from_instance_name("hurtBox"), Some(BoxType::Hurtbox));
}

#[test]
fn box_type_grab_variants() {
    assert_eq!(BoxType::from_instance_name("grabBox"), Some(BoxType::GrabBox));
    assert_eq!(BoxType::from_instance_name("grabbox2"), Some(BoxType::GrabBox));
}

#[test]
fn box_type_touchbox_is_grab_hold() {
    // touchBox is the SSF2 "grabbed-opponent position" marker — POINT layer in FM.
    assert_eq!(BoxType::from_instance_name("touchBox"), Some(BoxType::GrabHoldBox));
}

#[test]
fn box_type_specialty_boxes() {
    assert_eq!(BoxType::from_instance_name("shieldBox"), Some(BoxType::ShieldBox));
    assert_eq!(BoxType::from_instance_name("reflectBox"), Some(BoxType::ReflectBox));
    assert_eq!(BoxType::from_instance_name("absorbBox"), Some(BoxType::AbsorbBox));
    assert_eq!(BoxType::from_instance_name("ledgeBox"), Some(BoxType::LedgeBox));
    assert_eq!(BoxType::from_instance_name("ledgegrabbox"), Some(BoxType::LedgeBox));
    assert_eq!(BoxType::from_instance_name("itemBox"), Some(BoxType::ItemBox));
}

#[test]
fn box_type_unknown_ending_in_box_falls_back_to_hurtbox() {
    // Generic *box fallback per `BoxType::from_instance_name`.
    assert_eq!(BoxType::from_instance_name("totallyUnknownBox"), Some(BoxType::Hurtbox));
}

#[test]
fn box_type_non_box_names_are_none() {
    assert_eq!(BoxType::from_instance_name("stance"), None);
    assert_eq!(BoxType::from_instance_name("foo"), None);
    assert_eq!(BoxType::from_instance_name(""), None);
}

#[test]
fn box_type_as_str_round_trip() {
    // The as_str names are stable identifiers used in metadata / logs.
    assert_eq!(BoxType::Hitbox.as_str(), "HITBOX");
    assert_eq!(BoxType::Hurtbox.as_str(), "HURTBOX");
    assert_eq!(BoxType::GrabBox.as_str(), "GRAB_BOX");
    assert_eq!(BoxType::ItemBox.as_str(), "ITEM_BOX");
    assert_eq!(BoxType::GrabHoldBox.as_str(), "GRAB_HOLD_BOX");
}

// ─── ImageLocalMatrix::from_abcd ─────────────────────────────────────────

#[test]
fn matrix_identity_decomposition() {
    let m = ImageLocalMatrix::from_abcd(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    assert!((m.sx - 1.0).abs() < 1e-9);
    assert!((m.sy - 1.0).abs() < 1e-9);
    assert!(m.rotation.abs() < 1e-9);
    assert!(!m.has_skew(), "identity has no skew");
}

#[test]
fn matrix_pure_scale_decomposition() {
    let m = ImageLocalMatrix::from_abcd(2.0, 0.0, 0.0, 3.0, 10.0, 20.0);
    assert!((m.sx - 2.0).abs() < 1e-9);
    assert!((m.sy - 3.0).abs() < 1e-9);
    assert!((m.tx - 10.0).abs() < 1e-9);
    assert!((m.ty - 20.0).abs() < 1e-9);
    assert!(m.rotation.abs() < 1e-9);
    assert!(!m.has_skew());
}

#[test]
fn matrix_90deg_rotation_decomposition() {
    // 90° CW in y-down screen space: (a, b, c, d) = (0, 1, -1, 0)
    let m = ImageLocalMatrix::from_abcd(0.0, 1.0, -1.0, 0.0, 0.0, 0.0);
    assert!((m.sx - 1.0).abs() < 1e-9, "scaleX should be 1; got {}", m.sx);
    assert!((m.sy - 1.0).abs() < 1e-9, "scaleY should be 1; got {}", m.sy);
    assert!((m.rotation - 90.0).abs() < 1e-6, "rotation should be 90°; got {}", m.rotation);
    assert!(!m.has_skew(), "pure rotation has no skew");
}

#[test]
fn matrix_flip_encoded_as_negative_sy() {
    // Pure horizontal flip: (a, b, c, d) = (-1, 0, 0, 1) — det = -1
    // Convention: sx positive, sy negative when det < 0.
    let m = ImageLocalMatrix::from_abcd(-1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    assert!((m.sx - 1.0).abs() < 1e-9, "sx magnitude is 1; got {}", m.sx);
    // The flip sign lands on sy.
    assert!(m.sy < 0.0, "negative det should produce negative sy; got {}", m.sy);
    // Rotation is the angle of the x-axis column = 180° for (-1, 0).
    assert!((m.rotation.abs() - 180.0).abs() < 1e-6,
        "horizontal flip is 180° rotation; got {}", m.rotation);
}

#[test]
fn matrix_shear_detected() {
    // A genuine shear: (a, b, c, d) = (1, 0, 0.5, 1) — x stays unit-length,
    // y stays unit-length, but the y axis is sheared so they're not perpendicular.
    let m = ImageLocalMatrix::from_abcd(1.0, 0.0, 0.5, 1.0, 0.0, 0.0);
    assert!(m.has_skew(), "matrix with off-axis y column should be flagged as sheared");
}

#[test]
fn matrix_pure_scale_no_skew() {
    // Just (3, 0, 0, 2) — wide scale, no shear.
    let m = ImageLocalMatrix::from_abcd(3.0, 0.0, 0.0, 2.0, 0.0, 0.0);
    assert!(!m.has_skew());
}

#[test]
fn matrix_compose_identity_left() {
    let a = ImageLocalMatrix::from_abcd(2.0, 0.0, 0.0, 3.0, 10.0, 20.0);
    let id = ImageLocalMatrix::from_abcd(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    let r = id.compose(&a);
    assert!((r.a - a.a).abs() < 1e-9);
    assert!((r.b - a.b).abs() < 1e-9);
    assert!((r.c - a.c).abs() < 1e-9);
    assert!((r.d - a.d).abs() < 1e-9);
    assert!((r.tx - a.tx).abs() < 1e-9);
    assert!((r.ty - a.ty).abs() < 1e-9);
}

#[test]
fn matrix_compose_identity_right() {
    let a = ImageLocalMatrix::from_abcd(2.0, 0.0, 0.0, 3.0, 10.0, 20.0);
    let id = ImageLocalMatrix::from_abcd(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    let r = a.compose(&id);
    assert!((r.a - a.a).abs() < 1e-9);
    assert!((r.d - a.d).abs() < 1e-9);
    // a's translation should be unchanged when composed on the right of identity.
    assert!((r.tx - a.tx).abs() < 1e-9);
    assert!((r.ty - a.ty).abs() < 1e-9);
}

#[test]
fn matrix_compose_translation_then_scale() {
    // Translation-first, then scale (outer=scale, inner=translation).
    //   outer = (sx=2,sy=2)
    //   inner = (tx=10,ty=20)
    // Result: scaling the translated point. SWF maps (x,y) →
    //   (a*x + c*y + tx, b*x + d*y + ty). So composing outer ∘ inner
    //   should produce a matrix that translates by (2*10, 2*20) after
    //   the inner identity matrix's translation goes through outer's
    //   linear part.
    let outer = ImageLocalMatrix::from_abcd(2.0, 0.0, 0.0, 2.0, 0.0, 0.0);
    let inner = ImageLocalMatrix::from_abcd(1.0, 0.0, 0.0, 1.0, 10.0, 20.0);
    let r = outer.compose(&inner);
    // Linear part stays scale-by-2.
    assert!((r.a - 2.0).abs() < 1e-9);
    assert!((r.d - 2.0).abs() < 1e-9);
    // Translation gets scaled: inner.tx=10 → 20, inner.ty=20 → 40.
    assert!((r.tx - 20.0).abs() < 1e-9, "tx should be 20; got {}", r.tx);
    assert!((r.ty - 40.0).abs() < 1e-9, "ty should be 40; got {}", r.ty);
}
