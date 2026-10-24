//! Reading a SWF timeline: what is on the display list, frame by frame.
//!
//! SSF2 content is SSF2 content. A character's arm and a stage's clock hand are both a character
//! placed on a depth by a matrix, moved and turned and faded by the frames that follow, and both
//! engines' converters need the same answer: what was on screen, where, and how, at frame N.
//!
//! That answer used to be worked out twice -- once in the stage parser and once (several times
//! over) in the character path -- and the two disagreed about how much of a placement to read. The
//! stage side read the matrix, the ratio, the blend, the colour transform and the visibility flag;
//! the character side read the matrix. So a fade in a stage backdrop converted as a fade and the
//! same fade on a character converted as a hard cut, and the bug had to be found once per content
//! type. This module is the one reader, so a placement detail learned anywhere is available
//! everywhere.
//!
//! What it does NOT do is decide what any of it MEANS: which depths are art, which are collision,
//! which clip is a hazard. That is content-specific and stays with the caller.

use std::collections::BTreeMap;

/// A placement's transform, in pixels (SWF stores twips).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Default for Matrix {
    fn default() -> Self {
        Matrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 }
    }
}

impl Matrix {
    fn of(m: &swf::Matrix) -> Self {
        Matrix {
            a: m.a.to_f64(), b: m.b.to_f64(), c: m.c.to_f64(), d: m.d.to_f64(),
            tx: m.tx.get() as f64 / 20.0, ty: m.ty.get() as f64 / 20.0,
        }
    }
}

/// One object on the display list: everything a placement carries.
///
/// The fields beyond the matrix are the ones that were being dropped. They are not decoration:
/// `alpha` is how Flash writes a fade or a flash, `visible` is how it hides something without
/// removing it, `ratio` pins a graphic to one of its frames, and `blend` changes how it composites.
/// A per-channel multiply and offset, as SWF stores a colour transform and as the engine's colour
/// shader consumes one. Identity is all-ones, all-zeros.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub mul: [f64; 4],
    pub add: [f64; 4],
}

impl Tint {
    pub const ID: Tint = Tint { mul: [1.0; 4], add: [0.0; 4] };
    pub fn of(ct: &swf::ColorTransform) -> Tint {
        Tint {
            mul: [f32::from(ct.r_multiply) as f64, f32::from(ct.g_multiply) as f64,
                  f32::from(ct.b_multiply) as f64, f32::from(ct.a_multiply) as f64],
            add: [ct.r_add as f64, ct.g_add as f64, ct.b_add as f64, ct.a_add as f64],
        }
    }
    pub fn is_id(&self) -> bool {
        self.mul.iter().all(|m| (m - 1.0).abs() < 1e-6) && self.add.iter().all(|a| a.abs() < 1e-6)
    }
    /// This transform seen THROUGH a parent's: the child's colours are scaled by the parent's
    /// multiply, and the parent's offset lands on top of the child's already-scaled offset. Same
    /// composition the display tree does, and the same one alpha already gets.
    pub fn under(&self, parent: &Tint) -> Tint {
        let mut t = Tint::ID;
        for i in 0..4 {
            t.mul[i] = self.mul[i] * parent.mul[i];
            t.add[i] = self.add[i] * parent.mul[i] + parent.add[i];
        }
        t
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// The placed character (shape or sprite) id.
    pub char_id: u16,
    /// The display-list slot. Stable across frames, which is what makes an object an OBJECT rather
    /// than a fresh thing each frame.
    pub depth: u16,
    pub matrix: Matrix,
    /// The PlaceObject instance name, when the source gave it one.
    pub name: Option<String>,
    /// A `ratio` on a sprite placement pins it to that frame ("Single Frame" in Flash).
    pub ratio: Option<u16>,
    /// Set when this placement is a MASK. Flash masks by depth range: the placement is not drawn,
    /// and instead clips everything from its own depth up to this one. It is the only thing in the
    /// format that says "this shape is a hole", so a reader that drops it cannot tell a mask from
    /// a piece of art placed over the stage.
    pub clip_depth: Option<u16>,
    /// The placement's COLOUR transform, per channel: a multiply and an offset. SSF2 washes a whole
    /// stage from day to night with this, so reading only its alpha loses the wash and leaves the
    /// art at its painted colours, changing only when a cel does -- a hard step where the source
    /// slides. Kept whole because the engine's colour shader takes exactly these terms.
    pub tint: Option<Tint>,
    pub blend: Option<swf::BlendMode>,
    /// The colour transform's alpha, as a multiplier in `0..=1` (1.0 when it has none).
    pub alpha: f64,
    /// The placement's visibility flag; `true` unless the source turned it off.
    pub visible: bool,
    /// The placement's display FILTERS, in the order Flash applies them (empty when it has none).
    /// Part of how the art looks, not a hint: a glow here is the halo around fourside's moon.
    pub filters: Vec<swf::Filter>,
}

/// Snapshot the display list at every `ShowFrame`.
///
/// Flash semantics, which is the part worth stating once: `Place`/`Replace` set a depth,
/// `Modify` UPDATES whatever that depth already holds (and carries only the fields it is
/// changing -- a tween is a run of Modifies each with a new matrix or colour transform),
/// `RemoveObject` clears it, and `ShowFrame` is when the frame is what it is. A depth keeps its
/// state until something changes it, so a placement made on frame 1 is still there on frame 500.
///
/// Always at least one frame: a sprite with no `ShowFrame` still has content.
pub fn frames(tags: &[swf::Tag]) -> Vec<Vec<Placed>> {
    let mut depth: BTreeMap<u16, Placed> = BTreeMap::new();
    let mut out: Vec<Vec<Placed>> = Vec::new();
    for tag in tags {
        match tag {
            swf::Tag::PlaceObject(po) => {
                if std::env::var("PEPTIDE_ALPHA").is_ok() {
                    if let Some(ct) = po.color_transform.as_ref() {
                        let am = f32::from(ct.a_multiply);
                        if ct.a_add != 0 || (am - 1.0).abs() > 0.001 {
                            eprintln!("ALPHAT depth={} a_mul={am:.3} a_add={}", po.depth, ct.a_add);
                        }
                    }
                }
                if std::env::var("PEPTIDE_TINT").is_ok() {
                    if let Some(ct) = po.color_transform.as_ref() {
                        let (r,g,b) = (f32::from(ct.r_multiply), f32::from(ct.g_multiply), f32::from(ct.b_multiply));
                        let (ra,ga,ba) = (ct.r_add, ct.g_add, ct.b_add);
                        if (r-1.0).abs()>0.001 || (g-1.0).abs()>0.001 || (b-1.0).abs()>0.001
                            || ra!=0 || ga!=0 || ba!=0 {
                            eprintln!("TINT depth={} mul=({r:.3},{g:.3},{b:.3}) add=({ra},{ga},{ba})", po.depth);
                        }
                    }
                }
                let name = po.name.as_ref()
                    .map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
                let alpha_of = |ct: &swf::ColorTransform| {
                    let m = f32::from(ct.a_multiply) as f64;
                    (m + f32::from(ct.a_add) as f64 / 255.0).clamp(0.0, 1.0)
                };
                if std::env::var("PEPTIDE_PLACE_TRACE").is_ok() {
                    eprintln!("[place] frame={} depth={} {:?} matrix={}", out.len(), po.depth, po.action, po.matrix.is_some());
                }
                match po.action {
                    swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) => {
                        // A Replace (or a move-style Place) that carries no matrix/name/ratio KEEPS
                        // the slot's state and only swaps the character -- a pool of variants
                        // cycling through one depth. Resetting to identity stacks them all at the
                        // clip origin.
                        let prev = depth.get(&po.depth);
                        let e = Placed {
                            char_id: cid,
                            depth: po.depth,
                            matrix: po.matrix.as_ref().map(Matrix::of)
                                .or_else(|| prev.map(|p| p.matrix)).unwrap_or_default(),
                            name: name.or_else(|| prev.and_then(|p| p.name.clone())),
                            ratio: po.ratio.or_else(|| prev.and_then(|p| p.ratio)),
                            clip_depth: po.clip_depth.or_else(|| prev.and_then(|p| p.clip_depth)),
                            tint: po.color_transform.as_ref().map(Tint::of)
                                .or_else(|| prev.and_then(|p| p.tint)),
                            blend: po.blend_mode.or_else(|| prev.and_then(|p| p.blend)),
                            filters: po.filters.clone()
                                .or_else(|| prev.map(|p| p.filters.clone())).unwrap_or_default(),
                            alpha: po.color_transform.as_ref().map(alpha_of)
                                .or_else(|| prev.map(|p| p.alpha)).unwrap_or(1.0),
                            visible: po.is_visible.or_else(|| prev.map(|p| p.visible)).unwrap_or(true),
                        };
                        depth.insert(po.depth, e);
                    }
                    swf::PlaceObjectAction::Modify => {
                        if let Some(e) = depth.get_mut(&po.depth) {
                            if let Some(m) = po.matrix.as_ref() { e.matrix = Matrix::of(m); }
                            if let Some(n) = name { e.name = Some(n); }
                            if let Some(r) = po.ratio { e.ratio = Some(r); }
                            if let Some(c) = po.clip_depth { e.clip_depth = Some(c); }
                            if let Some(b) = po.blend_mode { e.blend = Some(b); }
                            if let Some(f) = po.filters.clone() { e.filters = f; }
                            if let Some(ct) = po.color_transform.as_ref() {
                                e.alpha = alpha_of(ct);
                                e.tint = Some(Tint::of(ct));
                            }
                            if let Some(v) = po.is_visible { e.visible = v; }
                        }
                    }
                }
            }
            swf::Tag::RemoveObject(r) => { depth.remove(&r.depth); }
            swf::Tag::ShowFrame => out.push(depth.values().cloned().collect()),
            _ => {}
        }
    }
    if out.is_empty() { out.push(depth.values().cloned().collect()); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use swf::{Tag, PlaceObject, PlaceObjectAction, Twips, Fixed16, Matrix as SwfMatrix, ColorTransform, Fixed8};

    fn place(depth: u16, id: u16) -> Tag<'static> {
        Tag::PlaceObject(Box::new(PlaceObject {
            version: 3, action: PlaceObjectAction::Place(id), depth,
            matrix: Some(SwfMatrix { a: Fixed16::ONE, b: Fixed16::ZERO, c: Fixed16::ZERO, d: Fixed16::ONE,
                                     tx: Twips::from_pixels(10.0), ty: Twips::from_pixels(20.0) }),
            color_transform: None, ratio: None, name: None, clip_depth: None, class_name: None,
            filters: None, background_color: None, blend_mode: None, clip_actions: None,
            has_image: false, is_bitmap_cached: None, is_visible: None, amf_data: None,
        }))
    }

    fn modify_alpha(depth: u16, a: f32) -> Tag<'static> {
        Tag::PlaceObject(Box::new(PlaceObject {
            version: 3, action: PlaceObjectAction::Modify, depth,
            matrix: None,
            color_transform: Some(ColorTransform {
                r_multiply: Fixed8::ONE, g_multiply: Fixed8::ONE, b_multiply: Fixed8::ONE,
                a_multiply: Fixed8::from_f32(a),
                r_add: 0, g_add: 0, b_add: 0, a_add: 0,
            }),
            ratio: None, name: None, clip_depth: None, class_name: None, filters: None,
            background_color: None, blend_mode: None, clip_actions: None, has_image: false,
            is_bitmap_cached: None, is_visible: None, amf_data: None,
        }))
    }

    fn hide(depth: u16) -> Tag<'static> {
        Tag::PlaceObject(Box::new(PlaceObject {
            version: 3, action: PlaceObjectAction::Modify, depth,
            matrix: None, color_transform: None, ratio: None, name: None, clip_depth: None,
            class_name: None, filters: None, background_color: None, blend_mode: None,
            clip_actions: None, has_image: false, is_bitmap_cached: None,
            is_visible: Some(false), amf_data: None,
        }))
    }

    /// A depth keeps its placement until something changes it.
    #[test]
    fn placement_persists_across_frames() {
        let f = frames(&[place(1, 7), Tag::ShowFrame, Tag::ShowFrame, Tag::ShowFrame]);
        assert_eq!(f.len(), 3);
        for frame in &f {
            assert_eq!(frame.len(), 1);
            assert_eq!(frame[0].char_id, 7);
            assert_eq!(frame[0].matrix.tx, 10.0);
        }
    }

    /// A Modify carrying a colour transform is how Flash tweens a fade -- the thing the character
    /// path was dropping.
    #[test]
    fn modify_carries_the_fade() {
        let f = frames(&[
            place(1, 7), Tag::ShowFrame,
            modify_alpha(1, 0.5), Tag::ShowFrame,
            modify_alpha(1, 0.0), Tag::ShowFrame,
        ]);
        assert_eq!(f[0][0].alpha, 1.0);
        assert!((f[1][0].alpha - 0.5).abs() < 0.01, "got {}", f[1][0].alpha);
        assert_eq!(f[2][0].alpha, 0.0);
        // and it does not disturb the transform it was not changing
        assert_eq!(f[2][0].matrix.tx, 10.0);
    }

    /// Hidden is not removed: the object stays on its depth, marked invisible.
    #[test]
    fn visibility_is_read_and_kept() {
        let f = frames(&[place(1, 7), Tag::ShowFrame, hide(1), Tag::ShowFrame]);
        assert!(f[0][0].visible);
        assert!(!f[1][0].visible);
        assert_eq!(f[1].len(), 1, "hiding is not removing");
    }

    /// RemoveObject clears the slot.
    #[test]
    fn remove_clears_the_depth() {
        let f = frames(&[
            place(1, 7), Tag::ShowFrame,
            Tag::RemoveObject(swf::RemoveObject { depth: 1, character_id: None }), Tag::ShowFrame,
        ]);
        assert_eq!(f[0].len(), 1);
        assert!(f[1].is_empty());
    }

    /// Content with no ShowFrame is still one frame of content.
    #[test]
    fn always_at_least_one_frame() {
        assert_eq!(frames(&[place(1, 7)]).len(), 1);
    }
}
