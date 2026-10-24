/// Extracts bitmap images from SWF tags and writes them as PNGs.
///
/// SWF structure for character sprites:
///   DefineSprite (animation) → PlaceObject (shape_id) → DefineShape (bitmap fill) → DefineBitsLossless
///
/// We extract the raw bitmaps and name the PNGs by their symbol name (e.g. mario_i0.png).
/// We also build a shape_id → bitmap_id mapping so callers can resolve
/// DefineSprite PlaceObject references to actual image files.

use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ExtractedImage {
    pub bitmap_id: u16,
    pub symbol_name: String,
    pub width: u32,
    pub height: u32,
    pub png_path: String, // relative path within character output dir
    /// True if this image was rasterized from a vector DefineShape/Morph (usually an
    /// effect), false for a real character bitmap (DefineBits). The skew-bake palette
    /// re-snap only applies to character bitmaps, never vector-rendered effects.
    pub is_vector: bool,
}

/// Maps shape/character IDs to bitmap IDs (DefineShape → bitmap fill ID)
pub type ShapeToBitmapMap = BTreeMap<u16, u16>;

/// One frame's stacked image layers: (depth, symbol_name, local placement matrix).
type FrameLayers = Vec<(u16, String, ImageLocalMatrix)>;
/// Per-sprite frame table: sprite id → its frames, each a list of image layers.
type SpriteFrameTable = BTreeMap<u16, Vec<FrameLayers>>;

/// Local placement matrix for an image within its sub-sprite (in SSF2 local pixel space).
/// tx/ty are in pixels (already divided by 20 from twips).
/// sx/sy are scale magnitudes (always positive).
/// Full 2D affine matrix from SWF PlaceObject.
/// rotation is in degrees, derived from the matrix b/c shear components.
#[derive(Debug, Clone, Copy)]
pub struct ImageLocalMatrix {
    pub tx: f64,
    pub ty: f64,
    pub sx: f64,
    pub sy: f64,
    /// Rotation in degrees (from matrix a/b components). 0 = no rotation.
    pub rotation: f64,
    /// Full affine matrix components for skew detection & pre-rendering
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
}

impl Default for ImageLocalMatrix {
    fn default() -> Self { Self { tx: 0.0, ty: 0.0, sx: 1.0, sy: 1.0, rotation: 0.0, a: 1.0, b: 0.0, c: 0.0, d: 1.0 } }
}

impl ImageLocalMatrix {
    /// Construct from raw SWF matrix components.
    ///
    /// Scale/rotation decomposition:
    ///   sx = magnitude of x-axis column vector (always positive)
    ///   sy = magnitude of y-axis column vector, NEGATED when det<0 (encodes flip)
    ///   rotation = atan2(b, a) — angle of the x-axis column
    ///
    /// With sy negative (flip), FrayTools reconstruct:
    ///   x' = sx*cos(rot)*x - sy*sin(rot)*y
    ///   y' = sx*sin(rot)*x + sy*cos(rot)*y
    /// which exactly matches the original a/b/c/d when the flip is encoded in sy.
    pub fn from_abcd(a: f64, b: f64, c: f64, d: f64, tx: f64, ty: f64) -> Self {
        let sx_mag = (a*a + b*b).sqrt();
        let sy_mag = (c*c + d*d).sqrt();
        let det = a * d - b * c;
        // Encode the flip in sy: negative det means an odd number of reflections.
        // Convention: keep sx positive and put the flip sign into sy.
        let sy = if det < 0.0 { -sy_mag } else { sy_mag };
        Self {
            tx, ty,
            sx: sx_mag,
            sy,
            rotation: b.atan2(a).to_degrees(),
            a, b, c, d,
        }
    }

    /// Compose two matrices: apply `other` first, then `self`
    /// (i.e. self ∘ other). The SWF matrix maps (x,y) →
    /// (a·x + c·y + tx, b·x + d·y + ty).
    ///
    /// The previous implementation multiplied the wrong off-diagonal
    /// terms, which injected a false shear into every composed
    /// sub-sprite / effect matrix whenever rotation or non-uniform
    /// scale was involved.
    pub fn compose(&self, other: &ImageLocalMatrix) -> Self {
        let a = self.a * other.a + self.c * other.b;
        let b = self.b * other.a + self.d * other.b;
        let c = self.a * other.c + self.c * other.d;
        let d = self.b * other.c + self.d * other.d;
        let tx = self.a * other.tx + self.c * other.ty + self.tx;
        let ty = self.b * other.tx + self.d * other.ty + self.ty;
        Self::from_abcd(a, b, c, d, tx, ty)
    }

    /// Returns true if this matrix contains a skew (not just scale + rotation + flip).
    /// A pure rotation (with optional flip) has the x-axis and y-axis perpendicular.
    /// When sy < 0 (flip encoded), the y-axis column direction is negated.
    pub fn has_skew(&self) -> bool {
        let rot1 = self.b.atan2(self.a);
        // When sy is negative the y-axis direction is flipped, so compare against
        // the negated column to see if the axes are still perpendicular.
        let rot2 = if self.sy < 0.0 {
            self.c.atan2(-self.d) // negated y-axis
        } else {
            (-self.c).atan2(self.d)
        };
        // Normalize angle difference to (-π, π]
        let mut diff = rot1 - rot2;
        while diff > std::f64::consts::PI  { diff -= 2.0 * std::f64::consts::PI; }
        while diff < -std::f64::consts::PI { diff += 2.0 * std::f64::consts::PI; }
        diff.abs() > 0.02 // ~1 degree tolerance
    }
}

/// A single image layer placed at a specific depth in one frame.
#[derive(Debug, Clone)]
pub struct FrameImageEntry {
    pub depth: u16,
    pub shape_id: u16,
    pub symbol_name: String,
    /// The opacity the SOURCE gives this placement (1.0 when it has none). Flash writes a fade,
    /// a flash or a ghosting trail as an animated alpha on the placement, so a converter that
    /// carries only the matrix turns every one of them into a hard cut.
    pub alpha: f64,
    /// Local placement matrix within the sub-sprite (before root MC transform)
    pub local_matrix: ImageLocalMatrix,
    /// World-space position after applying root MC transform (pixels, SSF2 y-down).
    /// Use these for Fraymakers IMAGE symbol x/y/scaleX/scaleY (after y-flip).
    pub world_tx: f64,
    pub world_ty: f64,
    pub world_sx: f64,
    pub world_sy: f64,
    /// World-space rotation in degrees (local rotation + root MC rotation, if any).
    pub world_rotation: f64,
    /// World-space affine matrix components (local matrix * root transform).
    /// Used to compute pivot-corrected positions.
    pub world_a: f64,
    pub world_b: f64,
    pub world_c: f64,
    pub world_d: f64,
    /// World-space position of the animation's LOCAL ORIGIN (0,0).
    /// In SSF2/Flash, the sub-sprite rotates around its local origin.
    /// This is simply root_xf.apply(0,0) = (root_xf.tx, root_xf.ty).
    /// This is the TRUE rotation center in world space.
    pub anim_origin_x: f64,
    pub anim_origin_y: f64,
}

/// Per-animation per-frame image references.
/// Each frame may have multiple entries (one per depth/layer).
/// Entries within a frame are ordered by depth (ascending = back-to-front).
#[derive(Debug, Clone)]
pub struct AnimFrameImages {
    /// frame_index → ordered list of (depth, shape_id, symbol_name)
    pub frames: BTreeMap<u16, Vec<FrameImageEntry>>,
    pub total_frames: u16,
    /// Number of distinct depth slots used across all frames (= number of IMAGE layers to create)
    pub max_depth_slots: usize,
}

/// Result of image extraction
pub struct ImageExtractionResult {
    /// bitmap_id → ExtractedImage (the raw PNG files)
    pub images: BTreeMap<u16, ExtractedImage>,
    /// shape_id → bitmap_id (for resolving PlaceObject refs)
    pub shape_to_bitmap: ShapeToBitmapMap,
    /// shape_id → (pivot_x, pivot_y) in bitmap pixel space.
    /// This is where the shape's local (0,0) origin lands in the bitmap,
    /// computed as (-fill_tx / (fill_a/20), -fill_ty / (fill_d/20)).
    /// Only populated for shapes with a non-trivial fill offset.
    pub shape_pivot: BTreeMap<u16, (f64, f64)>,
    /// shape_id → (fill_scale_x, fill_scale_y), the bitmap-fill matrix scale
    /// (signed; negative = flip). SSF2 references a high-res bitmap through a
    /// DefineShape whose fill matrix scales it (often WAY down, e.g. a shared
    /// magic-particle bitmap displayed at 0.08×). The PNG is extracted at native
    /// resolution, so this scale MUST multiply into the IMAGE symbol's scale or
    /// the sprite renders 1/fill_scale too big. Absent → 1.0 (vector shapes, or
    /// bitmaps placed 1:1). See entity_gen IMAGE placement.
    pub shape_fill_scale: BTreeMap<u16, (f64, f64)>,
    /// fm_anim_name → per-frame image references
    pub anim_images: BTreeMap<String, AnimFrameImages>,
}

/// Extract all bitmap images from the SWF, build mappings, and save as PNGs.
pub fn extract_images(
    swf_data: &[u8],
    output_dir: &Path,
    char_name: &str,
    ssf2_to_fm: &BTreeMap<String, String>,
) -> Result<ImageExtractionResult> {
    let swf_buf = swf::decompress_swf(swf_data)
        .context("Failed to decompress SWF")?;
    let swf = swf::parse_swf(&swf_buf)
        .context("Failed to parse SWF tags")?;
    let xform_map = crate::sprite_parser::extract_xframe_transforms_from_swf(
        &swf, char_name, ssf2_to_fm,
    ).unwrap_or_default();
    extract_images_from_swf(&swf, output_dir, char_name, ssf2_to_fm, &xform_map)
}

/// `extract_images` against an already-parsed SWF and a precomputed
/// xform_map. Used by main.rs::process_character so the SWF and the
/// xform_map are computed exactly once per character.
pub fn extract_images_from_swf(
    swf: &swf::Swf<'_>,
    output_dir: &Path,
    char_name: &str,
    ssf2_to_fm: &BTreeMap<String, String>,
    xform_map: &BTreeMap<String, crate::sprite_parser::XframeTransform>,
) -> Result<ImageExtractionResult> {
    extract_images_from_swf_only(swf, output_dir, char_name, ssf2_to_fm, xform_map, None, &[])
}

/// As above, leaving the children placed under the names in `skip` out of every animation's
/// art: a child the fighter's code addresses is drawn as a part of its own.
pub fn extract_images_from_swf_skipping(
    swf: &swf::Swf<'_>,
    output_dir: &Path,
    char_name: &str,
    ssf2_to_fm: &BTreeMap<String, String>,
    xform_map: &BTreeMap<String, crate::sprite_parser::XframeTransform>,
    skip: &[String],
) -> Result<ImageExtractionResult> {
    extract_images_from_swf_only(swf, output_dir, char_name, ssf2_to_fm, xform_map, None, skip)
}

/// Every character id a clip reaches: itself, the clips and shapes it places, and the pictures
/// those shapes paint with. What a stage's projectile needs extracted, and nothing else in the
/// package (its backdrop plates are not that projectile's pictures).
pub fn ids_under_sprites(swf: &swf::Swf<'_>, roots: &[u16]) -> BTreeSet<u16> {
    let mut sprites: BTreeMap<u16, &swf::Sprite> = BTreeMap::new();
    let mut shape_fills: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineSprite(s) => { sprites.insert(s.id, s); }
            swf::Tag::DefineShape(sh) => {
                let mut ids = Vec::new();
                let mut note = |fs: &[swf::FillStyle]| for f in fs {
                    if let swf::FillStyle::Bitmap { id, .. } = f { if *id != 65535 { ids.push(*id); } }
                };
                note(&sh.styles.fill_styles);
                for r in &sh.shape {
                    if let swf::ShapeRecord::StyleChange(sc) = r {
                        if let Some(ns) = &sc.new_styles { note(&ns.fill_styles); }
                    }
                }
                shape_fills.insert(sh.id, ids);
            }
            _ => {}
        }
    }
    let mut seen: BTreeSet<u16> = BTreeSet::new();
    let mut todo: Vec<u16> = roots.to_vec();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) { continue }
        if let Some(fills) = shape_fills.get(&id) { todo.extend(fills.iter().copied()); }
        if let Some(sp) = sprites.get(&id) {
            for t in &sp.tags {
                if let swf::Tag::PlaceObject(po) = t {
                    if let swf::PlaceObjectAction::Place(c) | swf::PlaceObjectAction::Replace(c) = po.action { todo.push(c); }
                }
            }
        }
    }
    seen
}

/// `extract_images_from_swf`, extracting only the character ids in `only` when one is given.
pub fn extract_images_from_swf_only(
    swf: &swf::Swf<'_>,
    output_dir: &Path,
    char_name: &str,
    ssf2_to_fm: &BTreeMap<String, String>,
    xform_map: &BTreeMap<String, crate::sprite_parser::XframeTransform>,
    only: Option<&BTreeSet<u16>>,
    skip: &[String],
) -> Result<ImageExtractionResult> {
    let wanted = |id: u16| only.is_none_or(|o| o.contains(&id));
    // Sprite PNGs are queued and encoded across all cores at the end (see PngBatch) rather
    // than one at a time inline, which is where a third of a conversion's CPU used to go.
    let mut pngs = PngBatch::default();

    // Build symbol table: char_id → class_name
    let mut symbols: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = String::from_utf8_lossy(link.class_name.as_bytes()).to_string();
                symbols.insert(link.id, name);
            }
        }
    }

    // 1. Build shape_id → bitmap_id map from DefineShape tags.
    //    Also compute shape_pivot: where shape (0,0) lands in bitmap pixel space.
    //    pivot_x = -fill_tx / (fill_a / 20),  pivot_y = -fill_ty / (fill_d / 20)
    let mut shape_to_bitmap: ShapeToBitmapMap = BTreeMap::new();
    let mut shape_pivot: BTreeMap<u16, (f64, f64)> = BTreeMap::new();
    let mut shape_fill_scale: BTreeMap<u16, (f64, f64)> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::DefineShape(shape) = tag {
            // Look for bitmap fill in fill styles.
            // Skip id=65535 (SWF null/clipping bitmap) — take the first real bitmap.
            for fill in &shape.styles.fill_styles {
                if let swf::FillStyle::Bitmap { id, matrix, .. } = fill {
                    if *id != 65535 {
                        shape_to_bitmap.insert(shape.id, *id);
                        // fill matrix: a/d are in twips-per-20-pixels, tx/ty in twips/20.
                        // scale_x = a/20 (pixels per shape-unit); pivot_x = -tx / scale_x
                        let fa = matrix.a.to_f64();
                        let fd = matrix.d.to_f64();
                        let ftx = matrix.tx.get() as f64 / 20.0;
                        let fty = matrix.ty.get() as f64 / 20.0;
                        let scale_x = fa / 20.0;
                        let scale_y = fd / 20.0;
                        if scale_x.abs() > 0.001 && scale_y.abs() > 0.001 {
                            // Store the fill scale so the IMAGE placement can fold it in
                            // (the PNG is at native res; this is how much SSF2 shrinks it).
                            shape_fill_scale.insert(shape.id, (scale_x, scale_y));
                            // fill matrix: shape_pt = fill_matrix * bitmap_pt
                            // So: bitmap_pt = inv(fill_matrix) * shape_pt
                            // For shape (0,0): bitmap_pt = (-tx/a, -ty/d) in pixel space
                            // But we want the OPPOSITE: offset FROM bitmap origin TO shape origin
                            // bitmap origin (0,0) → shape coords via fill matrix: (tx, ty)
                            // In pixels: (tx/(a/20), ty/(d/20)) = (tx*20/a, ty*20/d)
                            let px = ftx / scale_x;  // no negation
                            let py = fty / scale_y;
                            if px.abs() > 0.5 || py.abs() > 0.5 {
                                shape_pivot.insert(shape.id, (px, py));
                            }
                        }
                        break;
                    }
                }
            }
        }
    }
    log::info!("Shape→bitmap mappings: {} ({} with non-zero pivot)", shape_to_bitmap.len(), shape_pivot.len());

    // 2. Extract all bitmaps to PNGs
    let sprites_dir = output_dir.join("library/sprites");
    fs::create_dir_all(&sprites_dir)?;

    let mut images: BTreeMap<u16, ExtractedImage> = BTreeMap::new();

    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineBitsLossless(bmp) => {
                let id = bmp.id;
                if !wanted(id) { continue; }
                let w = bmp.width as u32;
                let h = bmp.height as u32;

                // Find the symbol name: prefer the shape's symbol that references this bitmap
                let sym = find_symbol_for_bitmap(id, &symbols, &shape_to_bitmap)
                    .unwrap_or_else(|| symbols.get(&id).cloned()
                        .unwrap_or_else(|| format!("bitmap_{}", id)));

                match decode_lossless(bmp) {
                    Ok(rgba_data) => {
                        let filename = format!("{}.png", sanitize_name(&sym));
                        let png_path = sprites_dir.join(&filename);
                        pngs.push(png_path.clone(), w, h, rgba_data)?;

                        images.insert(id, ExtractedImage {
                            bitmap_id: id,
                            symbol_name: sym,
                            width: w,
                            height: h,
                            png_path: format!("library/sprites/{}", filename),
                            is_vector: false,
                        });
                    }
                    Err(e) => {
                        log::debug!("Failed to decode lossless bitmap {}: {}", id, e);
                    }
                }
            }
            swf::Tag::DefineBitsJpeg3(jpeg) => {
                let id = jpeg.id;
                if !wanted(id) { continue; }
                let sym = find_symbol_for_bitmap(id, &symbols, &shape_to_bitmap)
                    .unwrap_or_else(|| symbols.get(&id).cloned()
                        .unwrap_or_else(|| format!("jpeg_{}", id)));

                match decode_jpeg3(jpeg) {
                    Ok((w, h, rgba_data)) => {
                        let filename = format!("{}.png", sanitize_name(&sym));
                        let png_path = sprites_dir.join(&filename);
                        pngs.push(png_path.clone(), w, h, rgba_data)?;

                        images.insert(id, ExtractedImage {
                            bitmap_id: id,
                            symbol_name: sym,
                            width: w,
                            height: h,
                            png_path: format!("library/sprites/{}", filename),
                            is_vector: false,
                        });
                    }
                    Err(e) => {
                        log::debug!("Failed to decode JPEG3 bitmap {}: {}", id, e);
                    }
                }
            }
            _ => {}
        }
    }

    // 2b. Rasterize VECTOR shapes (DefineShape with no bitmap fill, and
    //     DefineMorphShape) to PNGs so vector-drawn SSF2 VFX — dust swirls,
    //     sparks, trails, morphing waves — actually render in Fraymakers
    //     (which only displays bitmaps). They masquerade as bitmaps: keyed by
    //     the shape's own character id in `images` + `shape_to_bitmap`, with
    //     `shape_pivot` set so the existing placement math positions them.
    //
    //     Morph shapes: SSF2 places each morph at a *fixed* ratio (verified
    //     across chars — morphs are not ratio-tweened per frame; their
    //     animation comes from placing different morph shapes on different
    //     frames). So we rasterize each morph once at the ratio it is actually
    //     placed with (default 0 = start shape), which is exact for this data.
    let mut morph_ratio: BTreeMap<u16, u16> = BTreeMap::new();
    {
        let morph_ids: BTreeSet<u16> = swf.tags.iter().filter_map(|t| match t {
            swf::Tag::DefineMorphShape(m) => Some(m.id), _ => None,
        }).collect();
        if !morph_ids.is_empty() {
            for tag in &swf.tags {
                if let swf::Tag::DefineSprite(s) = tag {
                    for st in &s.tags {
                        if let swf::Tag::PlaceObject(po) = st {
                            if let swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) = &po.action {
                                if morph_ids.contains(cid) {
                                    // First non-zero ratio wins; else 0.
                                    let r = po.ratio.unwrap_or(0);
                                    let e = morph_ratio.entry(*cid).or_insert(0);
                                    if *e == 0 { *e = r; }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // A shape's bitmap fills are not all in its top-level fill table: a `StyleChange` can carry a
    // whole new one, and the scan above only reads the first. A shape filled that way reached the
    // VECTOR rasteriser, which has no bitmap fill and drew it as a flat silhouette -- which is what
    // donkeykong's, fox's and marth's menu portraits were. The stage art path already paints every
    // fill in every table through its own matrix, so that is what paints these.
    let fill_bitmaps: BTreeMap<u16, (u32, u32, Vec<u8>)> = {
        let mut m = BTreeMap::new();
        for tag in &swf.tags {
            match tag {
                swf::Tag::DefineBitsLossless(b) => {
                    if let Ok(rgba) = decode_lossless(b) { m.insert(b.id, (b.width as u32, b.height as u32, rgba)); }
                }
                swf::Tag::DefineBitsJpeg3(j) => {
                    if let Ok((w, h, rgba)) = decode_jpeg3(j) { m.insert(j.id, (w, h, rgba)); }
                }
                _ => {}
            }
        }
        m
    };
    let has_bitmap_fill = |shape: &swf::Shape| -> bool {
        let any = |fs: &[swf::FillStyle]| fs.iter().any(|f|
            matches!(f, swf::FillStyle::Bitmap { id, .. } if *id != 65535 && fill_bitmaps.contains_key(id)));
        any(&shape.styles.fill_styles) || shape.shape.iter().any(|r| match r {
            swf::ShapeRecord::StyleChange(sc) => sc.new_styles.as_ref().is_some_and(|ns| any(&ns.fill_styles)),
            _ => false,
        })
    };
    let mut vec_count = 0usize;
    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineShape(shape) if !shape_to_bitmap.contains_key(&shape.id) && wanted(shape.id) => {
                let painted = has_bitmap_fill(shape).then(|| {
                    let b = &shape.shape_bounds;
                    let (w, h) = (((b.x_max.to_pixels() - b.x_min.to_pixels()).ceil()).max(1.0),
                                  ((b.y_max.to_pixels() - b.y_min.to_pixels()).ceil()).max(1.0));
                    (w <= 4096.0 && h <= 4096.0)
                        .then(|| crate::stage_parser::bitmap_fill_layers(shape, &fill_bitmaps, w as u32, h as u32))
                        .flatten()
                        .map(|img| crate::vector_raster::Raster {
                            width: img.width(), height: img.height(), rgba: img.into_raw(),
                            origin_px: (b.x_min.to_pixels(), b.y_min.to_pixels()),
                        })
                }).flatten();
                if let Some(r) = painted.or_else(|| crate::vector_raster::rasterize_shape(
                    &shape.shape_bounds, &shape.styles.fill_styles, &shape.styles.line_styles, &shape.shape,
                )) {
                    let sym = format!("{}_vec_{}", char_name, shape.id);
                    let filename = format!("{}.png", sanitize_name(&sym));
                    pngs.push(sprites_dir.join(&filename), r.width, r.height, r.rgba.clone())?;
                    images.insert(shape.id, ExtractedImage {
                        bitmap_id: shape.id, symbol_name: sym, width: r.width, height: r.height,
                        png_path: format!("library/sprites/{}", filename),
                        is_vector: true,
                    });
                    shape_to_bitmap.insert(shape.id, shape.id);
                    shape_pivot.insert(shape.id, r.origin_px);
                    vec_count += 1;
                }
            }
            swf::Tag::DefineMorphShape(m) if !shape_to_bitmap.contains_key(&m.id) && wanted(m.id) => {
                let ratio = *morph_ratio.get(&m.id).unwrap_or(&0) as f64 / 65535.0;
                if let Some(r) = crate::vector_raster::rasterize_morph(m, ratio) {
                    let sym = format!("{}_vec_{}", char_name, m.id);
                    let filename = format!("{}.png", sanitize_name(&sym));
                    pngs.push(sprites_dir.join(&filename), r.width, r.height, r.rgba.clone())?;
                    images.insert(m.id, ExtractedImage {
                        bitmap_id: m.id, symbol_name: sym, width: r.width, height: r.height,
                        png_path: format!("library/sprites/{}", filename),
                        is_vector: true,
                    });
                    shape_to_bitmap.insert(m.id, m.id);
                    shape_pivot.insert(m.id, r.origin_px);
                    vec_count += 1;
                }
            }
            _ => {}
        }
    }
    log::info!("Rasterized {} vector shapes/morphs to PNGs", vec_count);

    log::info!("Extracted {} images to {}", images.len(), sprites_dir.display());

    // 3. xform_map is supplied by the caller so it's computed exactly
    //    once per character (was previously re-extracted here AND inside
    //    parse_sprite_boxes — three SWF parses across the two call sites).
    //
    // 4. Build per-animation per-frame image references from DefineSprite tags
    let mut anim_images = build_anim_frame_images(swf, char_name, ssf2_to_fm, &symbols, &shape_to_bitmap, xform_map, &images, skip);
    // Apply same fallbacks as sprite_parser for animations with no image data
    apply_image_fallbacks(&mut anim_images);
    log::info!("Animation image mappings: {} animations (after fallbacks)", anim_images.len());

    // Every queued sprite must be on disk BEFORE the shear pre-render: that pass re-opens
    // the sprites it bakes from, so anything still sitting in the batch would be invisible
    // to it and its baked output would silently go missing.
    pngs.flush()?;

    // 5. Pre-render frames whose world matrix contains shear — FrayTools
    //    keyframes can't express shear, so the shear is baked into a new
    //    bitmap and the frame becomes a plain translation.
    let skew_count = prerender_skewed_frames(
        &mut anim_images, &mut images, &shape_to_bitmap, &sprites_dir, char_name,
    );
    if skew_count > 0 {
        log::info!("Pre-rendered {} sheared frame placement(s) as baked bitmaps", skew_count);
    }

    Ok(ImageExtractionResult {
        images,
        shape_to_bitmap,
        shape_pivot,
        shape_fill_scale,
        anim_images,
    })
}

/// Pre-render frames whose WORLD placement matrix contains shear.
///
/// FrayTools' IMAGE keyframe expresses translation, rotation and
/// scaleX/scaleY but NOT shear. When a SWF places a sprite with a sheared
/// matrix, decomposing it to scale+rotation is lossy — the sprite appears to
/// shrink/balloon instead of stretching. For those frames we bake the full
/// linear part of the world matrix into a fresh bitmap and rewrite the entry
/// as a plain translation, which FrayTools can reproduce exactly.
///
/// Non-sheared frames (pure rotation + scale, with or without flip) are left
/// untouched — only genuine shear is baked.
fn prerender_skewed_frames(
    anim_images: &mut BTreeMap<String, AnimFrameImages>,
    images: &mut BTreeMap<u16, ExtractedImage>,
    shape_to_bitmap: &ShapeToBitmapMap,
    sprites_dir: &std::path::Path,
    char_name: &str,
) -> usize {
    use image::RgbaImage;

    // Collect (anim, frame, entry_idx) for entries whose world matrix shears.
    let mut work: Vec<(String, u16, usize)> = Vec::new();
    for (anim_name, anim_data) in anim_images.iter() {
        for (frame, entries) in &anim_data.frames {
            for (idx, e) in entries.iter().enumerate() {
                // Only bake direct-bitmap placements. When a DefineShape wraps a
                // bitmap fill (e.g. sandbag's trail sprites), the entry's world
                // matrix is in shape-coordinate space, not bitmap-pixel space —
                // baking the bitmap by that matrix would mis-place it. Those
                // stay on the faithful scale+rotation path.
                if shape_to_bitmap.contains_key(&e.shape_id) {
                    continue;
                }
                let wm = ImageLocalMatrix::from_abcd(
                    e.world_a, e.world_b, e.world_c, e.world_d, e.world_tx, e.world_ty);
                if wm.has_skew() {
                    work.push((anim_name.clone(), *frame, idx));
                }
            }
        }
    }
    if work.is_empty() { return 0; }

    // Synthetic bitmap ids for the baked PNGs — start above every real id.
    // u16 namespace, so we cap at u16::MAX. Past ~5,500 cache misses we
    // bail loudly instead of silently wrapping back into the real-id range
    // and colliding with existing shapes.
    let mut next_id: u16 = images.keys().max().copied().unwrap_or(0).max(59_999) + 1;
    // exact world linear part + source id → (id, sym, w, h, min_x, min_y)
    let mut cache: BTreeMap<String, (u16, String, u32, u32, f64, f64)> = BTreeMap::new();
    let mut count = 0usize;

    for (anim_name, frame, entry_idx) in &work {
        // Snapshot the fields we need (immutable borrow ends here).
        let (wa, wb, wc, wd, wtx, wty, shape_id, symbol_name) = {
            let e = &anim_images[anim_name].frames[frame][*entry_idx];
            (e.world_a, e.world_b, e.world_c, e.world_d, e.world_tx, e.world_ty,
             e.shape_id, e.symbol_name.clone())
        };

        // Resolve the source bitmap: shape_id → bitmap, else by symbol name.
        let bitmap_id = shape_to_bitmap.get(&shape_id).copied().unwrap_or(shape_id);
        let src_img = match images.get(&bitmap_id)
            .or_else(|| images.values().find(|img| img.symbol_name == symbol_name))
        {
            Some(img) => img.clone(),
            None => {
                log::debug!("prerender_skewed: no source image for sym='{}' shape={}", symbol_name, shape_id);
                continue;
            }
        };

        // Bake once per distinct (linear part, source bitmap). The key uses the
        // exact f64 bit patterns of the linear matrix: only frames whose matrix
        // is bit-for-bit identical share a baked bitmap, so two animations with
        // merely near-identical matrices can never collide and reuse each
        // other's bake. Truly identical placements still dedup, so this does
        // not bloat the output.
        let cache_key = format!("{:016x}_{:016x}_{:016x}_{:016x}_{}",
            wa.to_bits(), wb.to_bits(), wc.to_bits(), wd.to_bits(), src_img.bitmap_id);
        let (new_id, new_sym, _nw, _nh, min_x, min_y) = if let Some(c) = cache.get(&cache_key) {
            c.clone()
        } else {
            // png_path is "library/sprites/X.png" — resolve from the char output dir.
            let char_output_dir = sprites_dir.parent().and_then(|p| p.parent()).unwrap_or(sprites_dir);
            let src = match image::open(char_output_dir.join(&src_img.png_path)) {
                Ok(img) => img.to_rgba8(),
                Err(e) => {
                    log::debug!("prerender_skewed: failed to open '{}': {}", src_img.png_path, e);
                    continue;
                }
            };
            let (sw, sh) = (src.width() as f64, src.height() as f64);

            // Bounding box of the source rectangle under the linear map A_w.
            let corners = [(0.0, 0.0), (sw, 0.0), (0.0, sh), (sw, sh)];
            let tx: Vec<(f64, f64)> = corners.iter()
                .map(|(x, y)| (wa * x + wc * y, wb * x + wd * y))
                .collect();
            let min_x = tx.iter().map(|p| p.0).fold(f64::MAX, f64::min);
            let min_y = tx.iter().map(|p| p.1).fold(f64::MAX, f64::min);
            let max_x = tx.iter().map(|p| p.0).fold(f64::MIN, f64::max);
            let max_y = tx.iter().map(|p| p.1).fold(f64::MIN, f64::max);
            let dst_w = (max_x - min_x).ceil().max(1.0) as u32;
            let dst_h = (max_y - min_y).ceil().max(1.0) as u32;
            if dst_w > 4096 || dst_h > 4096 { continue; }

            // Inverse of [[wa, wc], [wb, wd]] for backward sampling.
            // Near-singular world matrix → no inverse, can't bake. Warn so
            // the user knows a sheared frame fell back to the (visually
            // wrong) scale+rotation path instead of being baked.
            let det = wa * wd - wb * wc;
            if det.abs() < 1e-9 {
                log::warn!(
                    "prerender_skewed_frames: skipping anim='{}' frame={} \
                     entry={} — world matrix is near-singular (det={:.3e}); \
                     the placement will use the scale+rotation fallback",
                    anim_name, frame, entry_idx, det
                );
                continue;
            }
            let (inv_a, inv_c) = ( wd / det, -wc / det);
            let (inv_b, inv_d) = (-wb / det,  wa / det);

            let mut dst = RgbaImage::new(dst_w, dst_h);
            let swi = src.width() as i64;
            let shi = src.height() as i64;

            // Sample the source with premultiplied alpha; out-of-bounds reads
            // return transparent black so sheared edges fade out cleanly.
            let sample = |xi: i64, yi: i64| -> [f64; 4] {
                if xi >= 0 && xi < swi && yi >= 0 && yi < shi {
                    let p = src.get_pixel(xi as u32, yi as u32).0;
                    let a = p[3] as f64 / 255.0;
                    [p[0] as f64 * a, p[1] as f64 * a, p[2] as f64 * a, p[3] as f64]
                } else {
                    [0.0; 4]
                }
            };
            // Catmull-Rom cubic weights for a fractional offset in [0,1).
            let cubic = |t: f64| -> [f64; 4] {
                let t2 = t * t;
                let t3 = t2 * t;
                [
                    -0.5 * t3 + t2 - 0.5 * t,
                     1.5 * t3 - 2.5 * t2 + 1.0,
                    -1.5 * t3 + 2.0 * t2 + 0.5 * t,
                     0.5 * t3 - 0.5 * t2,
                ]
            };
            // Bicubic (Catmull-Rom) sample of the source at a continuous point.
            // This interpolating kernel stays much sharper than bilinear and
            // its mild negative lobes preserve contrast, so the sprites' thin
            // dark outlines keep their definition instead of being averaged
            // toward the lighter interior.
            let bicubic = |sx: f64, sy: f64| -> [f64; 4] {
                let xf = sx.floor();
                let yf = sy.floor();
                let wx = cubic(sx - xf);
                let wy = cubic(sy - yf);
                let (x0, y0) = (xf as i64 - 1, yf as i64 - 1);
                let mut acc = [0.0f64; 4];
                // Anti-ringing: track the per-channel min/max of the 16 source taps and clamp
                // the result into that range. Catmull-Rom's negative lobes overshoot at
                // light/dark edges, producing pixels LIGHTER than any nearby source pixel; once
                // palette-snapped those become the lightest costume colour → scattered light
                // "noise" speckles all over a recoloured sprite (seen on jab). Clamping kills it.
                let mut mn = [f64::MAX; 4];
                let mut mx = [f64::MIN; 4];
                for (j, &wyj) in wy.iter().enumerate() {
                    for (i, &wxi) in wx.iter().enumerate() {
                        let w = wxi * wyj;
                        let c = sample(x0 + i as i64, y0 + j as i64);
                        for k in 0..4 { acc[k] += c[k] * w; mn[k] = mn[k].min(c[k]); mx[k] = mx[k].max(c[k]); }
                    }
                }
                for k in 0..4 { acc[k] = acc[k].clamp(mn[k], mx[k]); }
                acc
            };
            // Bicubic-resample every destination pixel into a premultiplied
            // float buffer. One interpolating tap per pixel — no supersampling,
            // which would blur thin dark outlines back toward their lighter
            // neighbours. Pixel (i,j) is centred on integer coord (i,j),
            // matching the previous sampler's convention (no positional shift).
            let npx = (dst_w as usize) * (dst_h as usize);
            let mut buf: Vec<[f64; 4]> = Vec::with_capacity(npx);
            for dy in 0..dst_h {
                for dx in 0..dst_w {
                    let fx = dx as f64 + min_x;
                    let fy = dy as f64 + min_y;
                    let sx = inv_a * fx + inv_c * fy;
                    let sy = inv_b * fx + inv_d * fy;
                    // Bicubic + unsharp is the base bake — it gives the clearest, smoothest
                    // sheared image. Recolorability is handled downstream (anti-ringing clamp
                    // above + the palette snap/despeckle in palette_gen), not by degrading the
                    // sampler.
                    buf.push(bicubic(sx, sy));
                }
            }

            // Unsharp mask: lifts edge contrast for a crisper base image. Its overshoot is
            // bounded by the anti-ringing clamp on the bicubic taps, and the recolor speckle it
            // would otherwise cause is cleaned by the palette despeckle in palette_gen.
            const UNSHARP_AMOUNT: f64 = 0.45;
            let gauss: Vec<f64> = {
                let sigma = 0.9_f64;
                let radius = (sigma * 3.0).ceil() as i64;
                let mut k: Vec<f64> = (-radius..=radius)
                    .map(|i| (-((i * i) as f64) / (2.0 * sigma * sigma)).exp())
                    .collect();
                let sum: f64 = k.iter().sum();
                for w in &mut k { *w /= sum; }
                k
            };
            let gradius = (gauss.len() / 2) as i64;
            let w = dst_w as i64;
            let h = dst_h as i64;
            let idx = |x: i64, y: i64| (y * w + x) as usize;
            // Separable Gaussian, zero-padded (premultiplied → transparent
            // outside): horizontal pass into `tmp`, then vertical into `blurred`.
            let mut tmp = vec![[0.0f64; 4]; npx];
            for y in 0..h {
                for x in 0..w {
                    let mut acc = [0.0f64; 4];
                    for (t, &gw) in gauss.iter().enumerate() {
                        let sx = x + t as i64 - gradius;
                        if sx < 0 || sx >= w { continue; }
                        let c = buf[idx(sx, y)];
                        for k in 0..4 { acc[k] += c[k] * gw; }
                    }
                    tmp[idx(x, y)] = acc;
                }
            }
            let mut blurred = vec![[0.0f64; 4]; npx];
            for y in 0..h {
                for x in 0..w {
                    let mut acc = [0.0f64; 4];
                    for (t, &gw) in gauss.iter().enumerate() {
                        let sy = y + t as i64 - gradius;
                        if sy < 0 || sy >= h { continue; }
                        let c = tmp[idx(x, sy)];
                        for k in 0..4 { acc[k] += c[k] * gw; }
                    }
                    blurred[idx(x, y)] = acc;
                }
            }

            // Finalize: apply the unsharp mask, then un-premultiply. Clamp
            // alpha to [0,255] and premultiplied colour to [0,alpha] so the
            // sharpen overshoot can't manufacture a bright or dark fringe.
            for y in 0..h {
                for x in 0..w {
                    let p = idx(x, y);
                    let mut out = [0.0f64; 4];
                    for k in 0..4 {
                        out[k] = buf[p][k] + UNSHARP_AMOUNT * (buf[p][k] - blurred[p][k]);
                    }
                    let alpha = out[3].clamp(0.0, 255.0);
                    if alpha > 0.5 {
                        let inv = 255.0 / alpha; // un-premultiply
                        dst.put_pixel(x as u32, y as u32, image::Rgba([
                            (out[0].clamp(0.0, alpha) * inv).round().clamp(0.0, 255.0) as u8,
                            (out[1].clamp(0.0, alpha) * inv).round().clamp(0.0, 255.0) as u8,
                            (out[2].clamp(0.0, alpha) * inv).round().clamp(0.0, 255.0) as u8,
                            alpha.round() as u8,
                        ]));
                    }
                }
            }

            // NOTE: the baked skew bitmap's bicubic/unsharp output is palette-snapped LATER,
            // in palette_gen::snap_skew_bitmaps_to_palette, which snaps every skew_*.png pixel
            // to the COSTUME color array (the swappable palette) — not the source bitmap's
            // sampled colours. Doing it there (a) targets the actual swap palette so no
            // non-costume colour survives to flicker, and (b) runs once after the costume
            // palette is finalized. So no per-bake snap here.

            let new_id = next_id;
            // Bail out cleanly if we'd overflow the u16 id namespace.
            // Wrapping back into 0 would collide with real shape ids.
            match next_id.checked_add(1) {
                Some(n) => next_id = n,
                None => {
                    log::warn!(
                        "prerender_skewed_frames: exhausted u16 synthetic-id \
                         namespace at id={}; subsequent sheared placements \
                         will fall back to the faithful scale+rotation path",
                        new_id
                    );
                    // Don't continue baking; emit nothing for this and any
                    // later entries. The faithful path is still correct,
                    // just visually wrong for shears.
                    break;
                }
            }
            let new_sym = format!("skew_{}_{}", char_name, new_id);
            let filename = format!("{}.png", sanitize_name(&new_sym));
            if let Err(e) = dst.save(sprites_dir.join(&filename)) {
                log::warn!("Failed to save pre-rendered skew bitmap: {}", e);
                continue;
            }
            images.insert(new_id, ExtractedImage {
                bitmap_id: new_id,
                symbol_name: new_sym.clone(),
                width: dst_w,
                height: dst_h,
                png_path: format!("library/sprites/{}", filename),
                is_vector: src_img.is_vector,
            });
            let v = (new_id, new_sym, dst_w, dst_h, min_x, min_y);
            cache.insert(cache_key, v.clone());
            v
        };

        // Rewrite the entry as a plain translation of the baked bitmap.
        // The baked bitmap's pixel (0,0) holds world content A_w·L at
        // (min_x, min_y); placed with an identity transform at
        // (world_tx + min_x, world_ty + min_y) it lands exactly where the
        // sheared placement intended.
        let e = anim_images.get_mut(anim_name).unwrap()
            .frames.get_mut(frame).unwrap().get_mut(*entry_idx).unwrap();
        e.shape_id = new_id;
        e.symbol_name = new_sym;
        e.world_tx = wtx + min_x;
        e.world_ty = wty + min_y;
        e.world_a = 1.0; e.world_b = 0.0; e.world_c = 0.0; e.world_d = 1.0;
        e.world_sx = 1.0; e.world_sy = 1.0; e.world_rotation = 0.0;
        e.local_matrix = ImageLocalMatrix::from_abcd(1.0, 0.0, 0.0, 1.0, e.world_tx, e.world_ty);
        count += 1;
    }

    count
}

/// Find the symbol name for a bitmap by looking up which shape references it
fn find_symbol_for_bitmap(
    bitmap_id: u16,
    symbols: &BTreeMap<u16, String>,
    shape_to_bitmap: &ShapeToBitmapMap,
) -> Option<String> {
    // Find a shape that references this bitmap and has a symbol name
    for (shape_id, bmp_id) in shape_to_bitmap {
        if *bmp_id == bitmap_id {
            if let Some(sym) = symbols.get(shape_id) {
                return Some(sym.clone());
            }
        }
    }
    None
}

/// Build per-animation per-frame image references by walking DefineSprite display lists.
/// For each animation sprite, track which shape (image) is placed on each frame.
fn build_anim_frame_images(
    swf: &swf::Swf,
    char_name: &str,
    ssf2_to_fm: &BTreeMap<String, String>,
    symbols: &BTreeMap<u16, String>,
    shape_to_bitmap: &ShapeToBitmapMap,
    xform_map: &BTreeMap<String, crate::sprite_parser::XframeTransform>,
    images: &BTreeMap<u16, ExtractedImage>,
    skip: &[String],
) -> BTreeMap<String, AnimFrameImages> {
    let char_lower = char_name.to_lowercase();
    // Animation identity comes from the TIMELINE (xframe FrameLabel + `stance`
    // instance), never from the sprite's symbol name. THE SAME shared map the box
    // extractor uses — images, boxes and transforms name the SAME animation per sprite.
    let anim_by_sprite = crate::sprite_parser::build_anim_sprite_map(swf, symbols, &char_lower, ssf2_to_fm);
    let mut result = BTreeMap::new();

    // Pre-build a lookup of all DefineSprite tags by id (including unnamed ones)
    let all_sprites: BTreeMap<u16, &swf::Sprite> = swf.tags.iter().filter_map(|t| {
        if let swf::Tag::DefineSprite(s) = t { Some((s.id, s)) } else { None }
    }).collect();

    // Pre-build effect sprite frame tables: sprite_id → Vec<frame> → Vec<(shape_id, sym, mat)>
    // Effect sprites are _fla. named sprites that are NOT top-level animation containers.
    // Recursively expands unnamed inner sprites.
    // `effect_sprites` holds the named "_fla." effect-sprite frame tables;
    // `unnamed_frames` holds every unnamed sub-sprite's frame table. Both are
    // needed downstream: a named effect placed in an animation expands via
    // `effect_sprites`, but an *unnamed* sub-sprite placed directly in an
    // animation timeline (e.g. sandbag's item_homerun body movieclips) must
    // expand via `unnamed_frames`.
    let (effect_sprites, unnamed_frames): (SpriteFrameTable, SpriteFrameTable) = {
        // Two-pass approach:
        // Pass 1: build inner unnamed sprite frame tables
        // Pass 2: for named effect sprites, inline the unnamed sub-sprites

        // First build all unnamed sprite frame tables
        let mut unnamed_frames: SpriteFrameTable = BTreeMap::new();
        for (&sid, sprite) in &all_sprites {
            if symbols.contains_key(&sid) { continue; } // skip named ones
            // The display list comes from the SHARED timeline reader (`swf_timeline`), which is
            // what knows Flash's semantics -- a depth keeps its placement, a Modify updates only
            // what it carries, hiding is not removing. This pass then keeps the part it needs: the
            // symbol behind each placed id, minus the collision boxes, which are not art.
            let frames: Vec<Vec<(u16, String, ImageLocalMatrix)>> = crate::swf_timeline::frames(&sprite.tags)
                .into_iter()
                .map(|frame| frame.into_iter().filter_map(|p| {
                    let m = ImageLocalMatrix::from_abcd(p.matrix.a, p.matrix.b, p.matrix.c, p.matrix.d,
                                                        p.matrix.tx, p.matrix.ty);
                    if let Some(sname) = symbols.get(&p.char_id) {
                        let lower = sname.to_lowercase();
                        if lower.contains("collisonbox") || lower.contains("collisionbox") { return None; }
                        return Some((p.char_id, sname.clone(), m));
                    }
                    let bitmap_id = shape_to_bitmap.get(&p.char_id)?;
                    let img = images.get(bitmap_id)?;
                    Some((p.char_id, img.symbol_name.clone(), m))
                }).collect())
                .collect();
            if !frames.is_empty() { unnamed_frames.insert(sid, frames); }
        }

        // Now build named effect sprite tables, inlining unnamed sub-sprites
        let mut map = BTreeMap::new();
        for tag in &swf.tags {
            if let swf::Tag::DefineSprite(sprite) = tag {
                let sym = match symbols.get(&sprite.id) { Some(s) => s.as_str(), None => continue };
                if !sym.contains("_fla.") { continue; }
                // Skip character ANIMATION sprites (those placed on the root timeline);
                // this block handles the remaining non-anim effect/trail sprites.
                if anim_by_sprite.contains_key(&sprite.id) { continue; }

                // Placements from the SHARED timeline reader; this pass only decides what each
                // one MEANS here (direct art, a bitmap-backed shape, or an unnamed sub-sprite to
                // expand), which is the content-specific half.
                let timeline = crate::swf_timeline::frames(&sprite.tags);
                // depth → (char_id, the frame it first appeared on). An unnamed sub-sprite plays
                // from where it was placed, so its inner frame is (now - then) unless a ratio pins
                // it to one (Flash's "Single Frame").
                let mut placed_on: BTreeMap<u16, (u16, u16)> = BTreeMap::new();
                let mut effect_frames: Vec<Vec<(u16, String, ImageLocalMatrix)>> = Vec::new();

                for (fi, frame) in timeline.iter().enumerate() {
                    let cur_frame = fi as u16;
                    for p in frame {
                        match placed_on.get(&p.depth) {
                            Some((cid, _)) if *cid == p.char_id => {}
                            _ => { placed_on.insert(p.depth, (p.char_id, cur_frame)); }
                        }
                    }
                    placed_on.retain(|d, _| frame.iter().any(|p| p.depth == *d));

                    let local_of = |p: &crate::swf_timeline::Placed| ImageLocalMatrix::from_abcd(
                        p.matrix.a, p.matrix.b, p.matrix.c, p.matrix.d, p.matrix.tx, p.matrix.ty);
                    // direct art first, then the sub-sprite expansions -- the order the layers are
                    // stacked in, which downstream depends on
                    let mut snap: Vec<(u16, String, ImageLocalMatrix)> = Vec::new();
                    for p in frame.iter().filter(|p| p.visible) {
                        if let Some(sname) = symbols.get(&p.char_id) {
                            let lower = sname.to_lowercase();
                            if lower.contains("collisonbox") || lower.contains("collisionbox") { continue; }
                            snap.push((p.char_id, sname.clone(), local_of(p)));
                        } else if let Some(bid) = shape_to_bitmap.get(&p.char_id) {
                            if let Some(img) = images.get(bid) {
                                snap.push((p.char_id, img.symbol_name.clone(), local_of(p)));
                            }
                        }
                    }
                    for p in frame.iter().filter(|p| p.visible) {
                        if symbols.contains_key(&p.char_id) || shape_to_bitmap.contains_key(&p.char_id) { continue; }
                        let Some(uframes) = unnamed_frames.get(&p.char_id) else { continue };
                        if uframes.is_empty() { continue; }
                        let started = placed_on.get(&p.depth).map(|(_, f)| *f).unwrap_or(cur_frame);
                        let uf_idx = match p.ratio {
                            Some(r) => (r as usize).min(uframes.len() - 1),
                            None => (cur_frame.saturating_sub(started) as usize).min(uframes.len() - 1),
                        };
                        let parent = local_of(p);
                        for (inner_id, inner_sym, inner_mat) in &uframes[uf_idx] {
                            snap.push((*inner_id, inner_sym.clone(), parent.compose(inner_mat)));
                        }
                    }
                    effect_frames.push(snap);
                }
                if !effect_frames.is_empty() {
                    log::debug!("image_extractor: effect sprite id={} '{}' has {} frames",
                        sprite.id, sym, effect_frames.len());
                    map.insert(sprite.id, effect_frames);
                }
            }
        }
        (map, unnamed_frames)
    };

    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sym = symbols.get(&sprite.id)
                .map(|s| s.as_str())
                .unwrap_or("");

            // Process character animation sprites. These are usually named
            // "{char}_fla.{Anim}_{N}", but a root-timeline `stance` sub-sprite can be
            // UNNAMED (sym="") and still be a real animation — the box pass processes
            // it via anim_by_sprite, so the image pass must too, or that animation comes
            // out with hurtboxes but no visual (e.g. zelda's `falling`/tumble stance).
            if !sym.contains("_fla.") && !anim_by_sprite.contains_key(&sprite.id) { continue; }

            let fm_name = match anim_by_sprite.get(&sprite.id) {
                // The shared map already resolved this sprite's FM animation name.
                Some((_ssf2, fm)) => fm.clone(),
                None => {
                    // Not a root-timeline animation — a sub-sprite carrying trail/effect
                    // images. Keep the raw symbol as the key (NOT a symbol-derived ANIM
                    // name); the trail/effect system references these by symbol.
                    sym.to_string()
                }
            };
            log::debug!("image_extractor: sprite '{}' → fm='{}'", sym, fm_name);

            // Root MC transform for this animation
            let root_xf = xform_map.get(&fm_name).copied()
                .unwrap_or(crate::sprite_parser::XframeTransform::default());

            let mut current_frame: u16 = 0;
            // depth → (shape_id, symbol_name, local_matrix) — the active display list
            let mut display_list: BTreeMap<u16, (u16, String, ImageLocalMatrix)> = BTreeMap::new();
            // depth → (effect_sprite_id, place_frame, parent_matrix, graphic_frame) — sub-sprite
            // placements; graphic_frame = the placement's ratio (a Graphic-symbol placement shows
            // the ONE frame the ratio selects — Flash's "Single Frame" — never self-animating).
            let mut sub_sprite_placements: BTreeMap<u16, (u16, u16, ImageLocalMatrix, Option<u16>)> = BTreeMap::new();
            let mut frames: BTreeMap<u16, Vec<FrameImageEntry>> = BTreeMap::new();
            // the depths holding a child left out by name: nothing placed or restated there is drawn
            let mut skipped_depths: BTreeSet<u16> = BTreeSet::new();
            // Per-frame placement state from the shared timeline reader: this loop tracks the ids
            // and matrices it needs for its own sub-sprite expansion, but the fields BEYOND the
            // matrix come from the one reader that knows them, rather than being tracked a second
            // time here and drifting from it.
            let timeline = crate::swf_timeline::frames(&sprite.tags);
            let mut frame_idx = 0usize;

            for stag in &sprite.tags {
                match stag {
                    swf::Tag::ShowFrame => {
                        let mut entries: Vec<FrameImageEntry> = Vec::new();
                        let placed_now = timeline.get(frame_idx);
                        let alpha_at = |depth: u16| -> f64 {
                            placed_now.and_then(|f| f.iter().find(|p| p.depth == depth))
                                .map(|p| p.alpha).unwrap_or(1.0)
                        };

                        // Animation local origin (0,0) in world space = root_xf.apply(0,0)
                        // This is the TRUE rotation center: Flash rotates sub-sprites around (0,0).
                        let anim_origin_x = root_xf.tx;
                        let anim_origin_y = root_xf.ty;

                        // Regular display list entries
                        for (&depth, (id, sym, mat)) in &display_list {
                            let w = placed_by(&root_xf, mat);
                            let (world_tx, world_ty, world_sx, world_sy) = (w.tx, w.ty, w.sx, w.sy);
                            let (wa, wb, wc, wd) = (w.a, w.b, w.c, w.d);
                            entries.push(FrameImageEntry {
                                depth,
                                shape_id: *id,
                                symbol_name: sym.clone(),
                                alpha: alpha_at(depth),
                                local_matrix: *mat,
                                world_tx,
                                world_ty,
                                world_sx,
                                world_sy,
                                world_rotation: w.rotation,
                                world_a: wa,
                                world_b: wb,
                                world_c: wc,
                                world_d: wd,
                                anim_origin_x,
                                anim_origin_y,
                            });
                        }

                        // Expand sub-sprite placements into their current inner
                        // frame. Named "_fla." effects live in effect_sprites;
                        // unnamed sub-sprites placed directly in the timeline
                        // (e.g. item_homerun's body movieclips) live in
                        // unnamed_frames.
                        for (&depth, (effect_id, place_frame, parent_mat, pinned)) in &sub_sprite_placements {
                            if let Some(effect_frames) = effect_sprites.get(effect_id)
                                .or_else(|| unnamed_frames.get(effect_id))
                            {
                                // a graphic-pinned placement shows the ratio-selected frame, always.
                                let eff_frame = match pinned {
                                    Some(r) => (*r as usize).min(effect_frames.len().saturating_sub(1)),
                                    None => (current_frame.saturating_sub(*place_frame) as usize)
                                        .min(effect_frames.len().saturating_sub(1)),
                                };
                                // The effect sub-sprite rotates around its own local origin (0,0).
                                // In world space that's root_xf.apply(parent_mat.tx, parent_mat.ty).
                                let (effect_origin_x, effect_origin_y) = root_xf.apply(parent_mat.tx, parent_mat.ty);
                                for (inner_id, inner_sym, inner_mat) in &effect_frames[eff_frame] {
                                    let composed = parent_mat.compose(inner_mat);
                                    let w = placed_by(&root_xf, &composed);
                                    let (world_tx, world_ty, world_sx, world_sy) = (w.tx, w.ty, w.sx, w.sy);
                                    let (wa, wb, wc, wd) = (w.a, w.b, w.c, w.d);
                                    let effect_depth = depth + 1000;
                                    entries.push(FrameImageEntry {
                                        depth: effect_depth,
                                        shape_id: *inner_id,
                                        symbol_name: inner_sym.clone(),
                                        // the sub-sprite's own placement carries the fade
                                        alpha: alpha_at(depth),
                                        local_matrix: composed,
                                        world_tx,
                                        world_ty,
                                        world_sx,
                                        world_sy,
                                        world_rotation: w.rotation,
                                        world_a: wa,
                                        world_b: wb,
                                        world_c: wc,
                                        world_d: wd,
                                        anim_origin_x: effect_origin_x,
                                        anim_origin_y: effect_origin_y,
                                    });
                                }
                            }
                        }

                        if !entries.is_empty() {
                            // Sort by depth so layers are ordered back-to-front
                            entries.sort_by_key(|e| e.depth);
                            frames.insert(current_frame, entries);
                        }
                        current_frame += 1;
                        // the shared reader's frames advance in step with this loop's ShowFrames
                        frame_idx += 1;
                    }
                    swf::Tag::PlaceObject(po) => {
                        let inst_name = po.name.as_ref()
                            .map(|n| String::from_utf8_lossy(n.as_bytes()).to_string())
                            .unwrap_or_default();

                        // Skip collision box instances
                        if crate::sprite_parser::BoxType::from_instance_name(&inst_name).is_some() {
                            continue;
                        }
                        if !skip.is_empty() {
                            if matches!(po.action, swf::PlaceObjectAction::Place(_) | swf::PlaceObjectAction::Replace(_)) {
                                if skip.contains(&inst_name) {
                                    skipped_depths.insert(po.depth);
                                    display_list.remove(&po.depth);
                                    sub_sprite_placements.remove(&po.depth);
                                    continue;
                                }
                                skipped_depths.remove(&po.depth);
                            } else if skipped_depths.contains(&po.depth) {
                                continue;
                            }
                        }

                        let depth = po.depth;
                        let local_mat = po.matrix.map(|m| {
                            let a = m.a.to_f64();
                            let b = m.b.to_f64();
                            let c = m.c.to_f64();
                            let d = m.d.to_f64();
                            ImageLocalMatrix::from_abcd(a, b, c, d,
                                m.tx.get() as f64 / 20.0, m.ty.get() as f64 / 20.0)
                        }).unwrap_or_default();

                        match &po.action {
                            swf::PlaceObjectAction::Place(char_id)
                            | swf::PlaceObjectAction::Replace(char_id) => {
                                if let Some(sym_name) = symbols.get(char_id) {
                                    let lower = sym_name.to_lowercase();
                                    if lower.contains("collisonbox") || lower.contains("collisionbox") {
                                        continue;
                                    }
                                    // Effect sprite (nested _fla. movieclip)? Track as sub-sprite.
                                    if lower.contains("_fla.") {
                                        if effect_sprites.contains_key(char_id) {
                                            sub_sprite_placements.insert(depth, (*char_id, current_frame, local_mat, po.ratio));
                                        }
                                        continue;
                                    }
                                    display_list.insert(depth, (*char_id, sym_name.clone(), local_mat));
                                } else if effect_sprites.contains_key(char_id) {
                                    // Unnamed sub-sprite that was pre-built as an effect sprite
                                    sub_sprite_placements.insert(depth, (*char_id, current_frame, local_mat, po.ratio));
                                } else if shape_to_bitmap.contains_key(char_id) {
                                    // Unnamed shape with a bitmap fill — resolve to bitmap symbol
                                    let bitmap_id = shape_to_bitmap[char_id];
                                    if let Some(img) = images.get(&bitmap_id) {
                                        display_list.insert(depth, (*char_id, img.symbol_name.clone(), local_mat));
                                    }
                                } else if all_sprites.contains_key(char_id) {
                                    // Unnamed sub-sprite not pre-built — track as effect
                                    sub_sprite_placements.insert(depth, (*char_id, current_frame, local_mat, po.ratio));
                                }
                            }
                            swf::PlaceObjectAction::Modify => {
                                // A MODIFY with no matrix only re-states the object (a
                                // "keep" frame) — preserve its existing transform rather
                                // than resetting it to identity, which would teleport it.
                                if po.matrix.is_some() {
                                    if let Some(entry) = display_list.get_mut(&depth) {
                                        entry.2 = local_mat;
                                    } else if let Some(entry) = sub_sprite_placements.get_mut(&depth) {
                                        // Update the parent matrix of a sub-sprite placement
                                        entry.2 = local_mat;
                                    }
                                }
                            }
                        }
                    }
                    swf::Tag::RemoveObject(ro) => {
                        display_list.remove(&ro.depth);
                        sub_sprite_placements.remove(&ro.depth);
                    }
                    _ => {}
                }
            }

            log::debug!("image_extractor: fm='{}' raw_frames={} from {} sprite frames", fm_name, frames.len(), sprite.num_frames);
            // Per-frame symbol trace, for asking what the SOURCE actually draws on a given frame
            // (PEPTIDE_TRACE_ANIM=tilt_up). The emitted keyframes are these, so a frame that looks
            // wrong in Fraymakers is answered here: either it came in wrong, or it was always so.
            if std::env::var("PEPTIDE_TRACE_ANIM").ok().as_deref() == Some(fm_name.as_str()) {
                let mut ks: Vec<_> = frames.keys().copied().collect();
                ks.sort();
                for f in ks {
                    let names: Vec<&str> = frames[&f].iter().map(|e| e.symbol_name.as_str()).collect();
                    eprintln!("[trace-anim] {fm_name} frame {f}: {names:?}");
                }
            }
            if !frames.is_empty() {
                // Fill in frames that didn't get an explicit ShowFrame snapshot (inherit previous)
                let total = sprite.num_frames;
                let mut last_entry: Option<Vec<FrameImageEntry>> = None;
                for f in 0..total {
                    if let Some(entry) = frames.get(&f) {
                        last_entry = Some(entry.clone());
                    } else if let Some(ref entry) = last_entry {
                        frames.insert(f, entry.clone());
                    }
                }
                // Compute max depth slots across all frames
                let max_depth_slots = frames.values().map(|v| v.len()).max().unwrap_or(1);

                // Check if this animation should be split into sub-animations
                // (same split table as sprite_parser)
                let frame_labels = crate::sprite_parser::extract_frame_labels_from_tags(&sprite.tags);
                let sub_splits = crate::sprite_parser::sub_anim_image_splits(&fm_name, &frame_labels, total);

                if sub_splits.is_empty() {
                    // Now that the image pass also processes unnamed stance sprites, two
                    // sprites can map to one fm name (a named visual + an image-less
                    // stance). Never let an image-less sprite clobber an entry that
                    // already has a visual.
                    let new_has_image = frames.values()
                        .any(|v| v.iter().any(|e| !e.symbol_name.is_empty()));
                    let existing_has_image = result.get(&fm_name)
                        .map(|p: &AnimFrameImages| p.frames.values().any(|v| v.iter().any(|e| !e.symbol_name.is_empty())))
                        .unwrap_or(false);
                    if new_has_image || !existing_has_image {
                    result.insert(fm_name, AnimFrameImages {
                        frames,
                        total_frames: total,
                        max_depth_slots,
                    });
                    }
                } else {
                    for (sub_fm_name, start_frame, end_frame) in sub_splits {
                        let slice_len = end_frame.saturating_sub(start_frame);
                        let sliced: BTreeMap<u16, Vec<FrameImageEntry>> = frames.iter()
                            .filter(|(&f, _)| f >= start_frame && f < end_frame)
                            .map(|(&f, v)| (f - start_frame, v.clone()))
                            .collect();
                        let sub_max = sliced.values().map(|v| v.len()).max().unwrap_or(1);
                        log::debug!("image_extractor: sub-anim '{}': frames {}..{} ({} img frames, {} depth slots)",
                            sub_fm_name, start_frame, end_frame, sliced.len(), sub_max);
                        result.insert(sub_fm_name, AnimFrameImages {
                            frames: sliced,
                            total_frames: slice_len,
                            max_depth_slots: sub_max,
                        });
                    }
                }
            }
        }
    }

    result
}

/// Apply image fallbacks for procedural/synthetic animations.
fn apply_image_fallbacks(result: &mut BTreeMap<String, AnimFrameImages>) {
    let mut to_insert: Vec<(String, AnimFrameImages)> = Vec::new();
    for (missing, donor) in crate::sprite_parser::ANIM_FALLBACKS {
        // Override if missing entirely OR if present but has no actual image frames
        let needs_fallback = match result.get(*missing) {
            None => true,
            Some(existing) => existing.frames.is_empty() || existing.frames.values().all(|entries| entries.iter().all(|e| e.symbol_name.starts_with("id_"))),
        };
        if !needs_fallback { continue; }
        if let Some(donor_data) = result.get(*donor) {
            to_insert.push((missing.to_string(), donor_data.clone()));
        }
    }
    for (name, data) in to_insert {
        result.insert(name, data);
    }
}

/// Decode DefineBitsLossless/DefineBitsLossless2 → RGBA pixels
pub(crate) fn decode_lossless(bmp: &swf::DefineBitsLossless) -> Result<Vec<u8>> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    let mut decoder = ZlibDecoder::new(&bmp.data[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;

    let w = bmp.width as usize;
    let h = bmp.height as usize;

    match bmp.format {
        swf::BitmapFormat::ColorMap8 { num_colors } => {
            let nc = num_colors as usize + 1;
            let has_alpha = bmp.version == 2;
            let bytes_per_color = if has_alpha { 4 } else { 3 };
            let palette_size = nc * bytes_per_color;

            if decompressed.len() < palette_size {
                anyhow::bail!("Palette data too short");
            }

            let palette = &decompressed[..palette_size];
            let pixel_data = &decompressed[palette_size..];

            // Row stride padded to 4-byte boundary
            let row_stride = (w + 3) & !3;
            let mut rgba = Vec::with_capacity(w * h * 4);

            for y in 0..h {
                let row_start = y * row_stride;
                for x in 0..w {
                    let idx = pixel_data.get(row_start + x).copied().unwrap_or(0) as usize;
                    let ci = idx.min(nc - 1);
                    let base = ci * bytes_per_color;
                    if has_alpha {
                        // DefineBitsLossless2 ColorMap8 palette is RGBA (not ARGB).
                        // SWF spec: format=3 with alpha uses swf_rgba = {R, G, B, A}.
                        // Only format=5 (Rgb32) uses ARGB order.
                        let r = palette[base];
                        let g = palette[base + 1];
                        let b = palette[base + 2];
                        let a = palette[base + 3];
                        rgba.extend_from_slice(&[r, g, b, a]);
                    } else {
                        let r = palette[base];
                        let g = palette[base + 1];
                        let b = palette[base + 2];
                        rgba.extend_from_slice(&[r, g, b, 255]);
                    }
                }
            }
            Ok(rgba)
        }
        swf::BitmapFormat::Rgb15 => {
            anyhow::bail!("RGB15 format not supported");
        }
        swf::BitmapFormat::Rgb32 => {
            let mut rgba = Vec::with_capacity(w * h * 4);
            if bmp.version == 2 {
                // ARGB premultiplied
                for pixel in decompressed.chunks_exact(4) {
                    let a = pixel[0];
                    let r = pixel[1];
                    let g = pixel[2];
                    let b = pixel[3];
                    if a == 0 {
                        rgba.extend_from_slice(&[0, 0, 0, 0]);
                    } else {
                        let r = ((r as u16 * 255) / a as u16).min(255) as u8;
                        let g = ((g as u16 * 255) / a as u16).min(255) as u8;
                        let b = ((b as u16 * 255) / a as u16).min(255) as u8;
                        rgba.extend_from_slice(&[r, g, b, a]);
                    }
                }
            } else {
                for pixel in decompressed.chunks_exact(4) {
                    let r = pixel[1];
                    let g = pixel[2];
                    let b = pixel[3];
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
            }
            Ok(rgba)
        }
    }
}

/// Decode DefineBitsJpeg3 → (width, height, RGBA pixels)
/// Decode a plain JPEG payload (the tags that carry no alpha channel of their own).
/// A picture stored as JPEG WITHOUT its tables, which the file keeps once and shares.
///
/// The tables sit in their own tag and the picture's own data starts straight at the image, so
/// neither half decodes alone: they are joined, dropping the end marker off the tables and the
/// start marker off the picture. Battlefield (Melee)'s whole backdrop is 26 of these, and read as
/// standalone JPEGs every one of them failed, so the stage came out black.
pub(crate) fn decode_jpeg_with_tables(tables: &[u8], data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    // a picture that carries its own tables needs no joining
    if tables.len() < 4 { return decode_jpeg_bytes(data) }
    let mut joined = Vec::with_capacity(tables.len() + data.len());
    joined.extend_from_slice(&tables[..tables.len() - 2]);   // without the end marker
    let body = if data.starts_with(&[0xFF, 0xD8]) { &data[2..] } else { data };
    joined.extend_from_slice(body);
    decode_jpeg_bytes(&joined).or_else(|_| decode_jpeg_bytes(data))
}

pub(crate) fn decode_jpeg_bytes(data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    use image::ImageReader;
    use std::io::Cursor;
    let img = ImageReader::new(Cursor::new(data)).with_guessed_format()?.decode()?;
    let rgba = img.to_rgba8();
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

pub(crate) fn decode_jpeg3(jpeg: &swf::DefineBitsJpeg3) -> Result<(u32, u32, Vec<u8>)> {
    use image::ImageReader;
    use std::io::Cursor;

    let reader = ImageReader::new(Cursor::new(&jpeg.data))
        .with_guessed_format()?;
    let img = reader.decode()?;
    let rgb = img.to_rgba8();
    let w = rgb.width();
    let h = rgb.height();
    let mut rgba = rgb.into_raw();

    if !jpeg.alpha_data.is_empty() {
        use flate2::read::ZlibDecoder;
        use std::io::Read;
        let mut decoder = ZlibDecoder::new(jpeg.alpha_data);
        let mut alpha = Vec::new();
        decoder.read_to_end(&mut alpha)?;

        for (i, a) in alpha.iter().enumerate() {
            if let Some(px) = rgba.get_mut(i * 4 + 3) {
                *px = *a;
            }
        }
    }

    Ok((w, h, rgba))
}

/// Write RGBA pixel data as PNG
fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    use image::{ImageBuffer, Rgba};
    let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(width, height, rgba.to_vec())
        .context("Failed to create image buffer")?;
    img.save(path)?;
    Ok(())
}

/// Sprite PNGs queued up and encoded across all cores instead of one at a time.
///
/// PNG encode (deflate + row filtering) is about a THIRD of a conversion's CPU and every
/// sprite is independent, so encoding them serially leaves every other core idle. Deferring
/// is safe because nothing reads a sprite back while the SWF walk is still running.
///
/// Queued pixel data is raw RGBA, which is far larger than the encoded file, so the batch
/// flushes itself once it is holding more than `FLUSH_BYTES` — a big character can't balloon
/// memory, and the parallelism is preserved because each flush still has many images in it.
#[derive(Default)]
pub(crate) struct PngBatch {
    jobs: Vec<(std::path::PathBuf, u32, u32, Vec<u8>)>,
    queued_bytes: usize,
}

impl PngBatch {
    /// Roughly 64 MB of raw RGBA in flight before an automatic flush.
    const FLUSH_BYTES: usize = 64 * 1024 * 1024;

    fn push(&mut self, path: std::path::PathBuf, width: u32, height: u32, rgba: Vec<u8>) -> Result<()> {
        self.queued_bytes += rgba.len();
        self.jobs.push((path, width, height, rgba));
        if self.queued_bytes >= Self::FLUSH_BYTES {
            self.flush()?;
        }
        Ok(())
    }

    /// Encode + write everything queued, across `available_parallelism` threads. Workers pull
    /// from a shared cursor so one enormous sprite can't leave the other threads idle.
    fn flush(&mut self) -> Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        if self.jobs.is_empty() {
            return Ok(());
        }
        let jobs = std::mem::take(&mut self.jobs);
        self.queued_bytes = 0;
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(jobs.len());
        let cursor = AtomicUsize::new(0);
        let failure: std::sync::Mutex<Option<anyhow::Error>> = std::sync::Mutex::new(None);
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| loop {
                    let i = cursor.fetch_add(1, Ordering::Relaxed);
                    let Some((path, w, h, rgba)) = jobs.get(i) else { break };
                    if let Err(e) = write_png(path, *w, *h, rgba) {
                        let mut slot = failure.lock().unwrap();
                        if slot.is_none() {
                            *slot = Some(e);
                        }
                    }
                });
            }
        });
        match failure.into_inner().unwrap() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

fn sanitize_name(name: &str) -> String {
    name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "_")
}

// ─── Projectile image frame extraction ────────────────────────────────────────────────────────

/// Per-frame image data for a single projectile animation.
#[derive(Debug, Clone)]
pub struct ProjectileFrameImages {
    /// frame index → symbol name of the image placed on that frame (if any)
    pub frames: Vec<Option<String>>,
    /// frame index → the placed image's SSF2 local transform (scale/pos/rotation).
    /// Parallel to `frames`. Without applying this the image renders at its full
    /// native bitmap size (SSF2 scales it down) — i.e. WAY too big.
    pub matrices: Vec<Option<ImageLocalMatrix>>,
    /// symbol_name → meta GUID
    pub image_guids: BTreeMap<String, String>,
    /// frame index → EVERY image placed on that frame, in depth order, with its transform.
    /// A clip draws with more than one part at once (sora's fire is a ring of six fireballs;
    /// `frames` keeps only the first), and an entity that shows one of them shows a sixth of
    /// the effect.
    pub parts: Vec<Vec<(String, ImageLocalMatrix)>>,
}

/// `extract_projectile_frame_images` against an already-parsed SWF. Used
/// per projectile / per effect by main.rs / haxe_gen.rs to amortise the
/// SWF parse across all calls within one character's conversion.
/// A child a clip places under a name its code can address (`self.gordospin`): the clip it is,
/// and where the timeline puts it in the parent's space (its first placement).
#[derive(Debug, Clone)]
pub struct NamedChild {
    pub name: String,
    pub sprite_id: u16,
    pub matrix: ImageLocalMatrix,
}

/// The tags of one sprite of the package.
pub fn sprite_tags<'a>(swf: &'a swf::Swf<'a>, sprite_id: u16) -> Option<&'a [swf::Tag<'a>]> {
    swf.tags.iter().find_map(|t| match t {
        swf::Tag::DefineSprite(s) if s.id == sprite_id => Some(s.tags.as_slice()),
        _ => None,
    })
}

/// The named clip children a sprite places on its timeline, each once, under its first
/// placement. A clip placed without a name is looked through (a wrapper holds the clip that
/// holds the named child), with the placements composed; a named one is not, since its own
/// children are its own (`notes.n1` is asked of `notes`).
pub fn named_children_of_sprite(swf: &swf::Swf<'_>, sprite_id: u16) -> Vec<NamedChild> {
    let sprites: std::collections::BTreeSet<u16> = swf.tags.iter().filter_map(|t| match t {
        swf::Tag::DefineSprite(s) => Some(s.id), _ => None }).collect();
    let mut out: Vec<NamedChild> = Vec::new();
    fn walk(swf: &swf::Swf<'_>, sprites: &std::collections::BTreeSet<u16>, sprite_id: u16,
            at: &ImageLocalMatrix, depth: u32, out: &mut Vec<NamedChild>) {
        if depth > 4 { return }
        let Some(tags) = sprite_tags(swf, sprite_id) else { return };
        let mut seen_depths: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
        for tag in tags {
            let swf::Tag::PlaceObject(po) = tag else { continue };
            let cid = match &po.action {
                swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) => *cid,
                _ => continue,
            };
            let name = po.name.as_ref().map(|n| String::from_utf8_lossy(n.as_bytes()).to_string()).unwrap_or_default();
            if std::env::var("PEPTIDE_DUMP_PARTS").is_ok() {
                eprintln!("[parts-walk] sprite {sprite_id} depth {} places {cid} (sprite: {}) name '{name}'", po.depth, sprites.contains(&cid));
            }
            if !sprites.contains(&cid) || !seen_depths.insert(po.depth) { continue }
            let here = at.compose(&po_to_mat(po));
            if name.is_empty() {
                walk(swf, sprites, cid, &here, depth + 1, out);
            } else if !out.iter().any(|c| c.name == name) {
                out.push(NamedChild { name, sprite_id: cid, matrix: here });
            }
        }
    }
    walk(swf, &sprites, sprite_id, &ImageLocalMatrix::default(), 0, &mut out);
    out
}

/// Every named clip placed anywhere under a sprite, through named and unnamed clips alike,
/// each name once (its first placement), with the placements composed down to it. A fighter's
/// code reaches a child of its animation's clip by its own name however deep the clip nests it
/// (`self.waftloc`, placed inside the stance a wrapper holds).
pub fn named_descendants_of_sprite(swf: &swf::Swf<'_>, sprite_id: u16) -> Vec<NamedChild> {
    let sprites: std::collections::BTreeSet<u16> = swf.tags.iter().filter_map(|t| match t {
        swf::Tag::DefineSprite(s) => Some(s.id), _ => None }).collect();
    let mut out: Vec<NamedChild> = Vec::new();
    fn walk(swf: &swf::Swf<'_>, sprites: &std::collections::BTreeSet<u16>, sprite_id: u16,
            at: &ImageLocalMatrix, depth: u32, out: &mut Vec<NamedChild>) {
        if depth > 6 { return }
        let Some(tags) = sprite_tags(swf, sprite_id) else { return };
        // one visit per placement, not per depth: a later frame puts another child at a depth an
        // earlier one held (meta knight's attacks swap one colour of trail for another there)
        let mut seen: std::collections::BTreeSet<(u16, u16, String)> = std::collections::BTreeSet::new();
        for tag in tags {
            let swf::Tag::PlaceObject(po) = tag else { continue };
            let cid = match &po.action {
                swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) => *cid,
                _ => continue,
            };
            let name = po.name.as_ref().map(|n| String::from_utf8_lossy(n.as_bytes()).to_string()).unwrap_or_default();
            if !sprites.contains(&cid) || !seen.insert((po.depth, cid, name.clone())) { continue }
            let here = at.compose(&po_to_mat(po));
            if !name.is_empty() && !out.iter().any(|c| c.name == name) {
                out.push(NamedChild { name, sprite_id: cid, matrix: here });
            }
            walk(swf, sprites, cid, &here, depth + 1, out);
        }
    }
    walk(swf, &sprites, sprite_id, &ImageLocalMatrix::default(), 0, &mut out);
    out
}

pub fn extract_projectile_frame_images_from_swf(
    swf: &swf::Swf<'_>,
    char_id: &str,
    inner_sprite_id: u16,
    img_result: &ImageExtractionResult,
) -> Result<ProjectileFrameImages> {
    extract_projectile_frame_images_from_swf_skipping(swf, char_id, inner_sprite_id, img_result, &[])
}

/// As above, leaving out the children placed under the names in `skip`: a child the clip's code
/// addresses is drawn as a part of its own, not baked into the parent's frames as well.
pub fn extract_projectile_frame_images_from_swf_skipping(
    swf: &swf::Swf<'_>,
    // Image GUIDs are now symbol-only (see image_meta_guid), so the owner id is
    // no longer part of the seed. Kept in the signature for caller compatibility.
    _char_id: &str,
    inner_sprite_id: u16,
    img_result: &ImageExtractionResult,
    skip: &[String],
) -> Result<ProjectileFrameImages> {
    // Build symbol map
    let mut symbols: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                if !name.is_empty() { symbols.insert(link.id, name); }
            }
        }
    }

    // Collect all sprites
    let mut all_sprites: BTreeMap<u16, &swf::Sprite> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(s) = tag { all_sprites.insert(s.id, s); }
    }

    // Pass 1: build sprite frame tables for ALL sprites (named and unnamed)
    // For unnamed sprites: resolve shape_id → bitmap_id → symbol_name via img_result.shape_to_bitmap.
    // For named sprites: use symbol names directly; these form inner animation sprites for projectiles.
    let mut unnamed_frames: SpriteFrameTable = BTreeMap::new();
    for (&sid, sprite) in &all_sprites {
        let mut disp: BTreeMap<u16, (u16, String, ImageLocalMatrix)> = BTreeMap::new();
        let mut frames: Vec<Vec<(u16, String, ImageLocalMatrix)>> = Vec::new();
        // a child left out by name is left out of the wrapper that holds it as well
        let mut skipped_depths: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
        for stag in &sprite.tags {
            if let swf::Tag::PlaceObject(po) = stag {
                if !skip.is_empty() {
                    let placed_name = po.name.as_ref().map(|n| String::from_utf8_lossy(n.as_bytes()).to_string()).unwrap_or_default();
                    if matches!(po.action, swf::PlaceObjectAction::Place(_) | swf::PlaceObjectAction::Replace(_)) {
                        if skip.contains(&placed_name) { skipped_depths.insert(po.depth); disp.remove(&po.depth); continue; }
                        skipped_depths.remove(&po.depth);
                    } else if skipped_depths.contains(&po.depth) { continue; }
                }
            }
            if let swf::Tag::RemoveObject(ro) = stag { skipped_depths.remove(&ro.depth); }
            if std::env::var("PEPTIDE_DUMP_EFFECTS").is_ok() {
                if let swf::Tag::PlaceObject(po) = stag {
                    if let swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) = &po.action {
                        eprintln!("[place-kind1] sprite {sid} places {cid}: named={:?} sprite={} shape_to_bitmap={:?}", symbols.get(cid), all_sprites.contains_key(cid), img_result.shape_to_bitmap.get(cid));
                    }
                }
            }
            if std::env::var("PEPTIDE_DUMP_BLENDS").is_ok() {
                if let swf::Tag::PlaceObject(po) = stag {
                    if po.blend_mode.is_some() || po.color_transform.as_ref().is_some_and(|c| c.a_multiply.to_f32() < 0.99) || po.filters.is_some() {
                        eprintln!("[blend] sprite {} depth {} action {:?} blend {:?} alpha {:?} filters {}", sid, po.depth, po.action, po.blend_mode,
                            po.color_transform.as_ref().map(|c| c.a_multiply.to_f32()), po.filters.as_ref().map(|f| f.len()).unwrap_or(0));
                    }
                }
            }
            match stag {
                swf::Tag::ShowFrame => {
                    frames.push(disp.values().map(|(id, sym, mat)| (*id, sym.clone(), *mat)).collect());
                }
                swf::Tag::PlaceObject(po) => {
                    // a placement that states no matrix keeps the one its depth already has
                    // (a Replace swaps the picture and nothing else): read as identity, sora's
                    // keyblade glow lost its 0.18 scale on its second frame and drew 250 px wide
                    let mat = match po.matrix {
                        Some(_) => po_to_mat(po),
                        None => disp.get(&po.depth).map(|e| e.2).unwrap_or_default(),
                    };
                    match &po.action {
                        swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) => {
                            if let Some(sname) = symbols.get(cid) {
                                // Named symbol
                                disp.insert(po.depth, (*cid, sname.clone(), mat));
                            } else if let Some(&bitmap_id) = img_result.shape_to_bitmap.get(cid) {
                                // Unnamed shape with a bitmap fill — resolve to symbol name
                                if let Some(img) = img_result.images.get(&bitmap_id) {
                                    disp.insert(po.depth, (*cid, img.symbol_name.clone(), with_pivot(mat, img_result.shape_pivot.get(cid))));
                                }
                            } else if all_sprites.contains_key(cid) {
                                // a clip inside a clip: noted with the frame it was placed on, and
                                // flattened into the pictures it draws once every table is built
                                disp.insert(po.depth, (*cid, format!("__nested@{}", frames.len()), mat));
                            }
                        }
                        swf::PlaceObjectAction::Modify => {
                            // A MODIFY with no matrix only re-states the object —
                            // keep its existing transform.
                            if po.matrix.is_some() {
                                if let Some(e) = disp.get_mut(&po.depth) { e.2 = mat; }
                            }
                        }
                    }
                }
                swf::Tag::RemoveObject(ro) => { disp.remove(&ro.depth); }
                _ => {}
            }
        }
        if !frames.is_empty() { unnamed_frames.insert(sid, frames); }
    }
    // A CLIP INSIDE A CLIP. A wrapper's own table so far names the clip it holds, not the
    // pictures that clip draws; those come from the held clip's own table, at the frame it has
    // reached (placed on frame p, on the wrapper's frame f it shows its frame f - p, looping).
    // Read one level only, krystal's shield frame (a wrapper holding a wrapper holding the
    // picture) drew nothing.
    fn flatten_nested(
        id: u16, frame: usize, at: &ImageLocalMatrix, tables: &SpriteFrameTable, depth: u32,
        out: &mut Vec<(u16, String, ImageLocalMatrix)>,
    ) {
        if depth > 6 { return }
        let Some(frames) = tables.get(&id) else { return };
        if frames.is_empty() { return }
        let entries = &frames[frame % frames.len()];
        for (cid, sym, mat) in entries {
            if let Some(placed) = sym.strip_prefix("__nested@").and_then(|p| p.parse::<usize>().ok()) {
                flatten_nested(*cid, frame.saturating_sub(placed), &at.compose(mat), tables, depth + 1, out);
            } else {
                out.push((*cid, sym.clone(), at.compose(mat)));
            }
        }
    }
    let raw_tables = unnamed_frames.clone();
    for (sid, frames) in unnamed_frames.iter_mut() {
        if !frames.iter().flatten().any(|(_, sym, _)| sym.starts_with("__nested@")) { continue }
        for (f, frame) in frames.iter_mut().enumerate() {
            let mut flat: Vec<(u16, String, ImageLocalMatrix)> = Vec::new();
            flatten_nested(*sid, f, &ImageLocalMatrix::default(), &raw_tables, 0, &mut flat);
            *frame = flat;
        }
    }

    // Pass 2: flatten target sprite
    let sprite = match all_sprites.get(&inner_sprite_id) {
        Some(s) => s,
        None => return Ok(ProjectileFrameImages { frames: vec![], matrices: vec![], image_guids: BTreeMap::new(), parts: vec![] }),
    };

    let mut disp: BTreeMap<u16, (u16, String, ImageLocalMatrix)> = BTreeMap::new();
    // depth -> (sprite id, parent matrix, place frame, graphic frame): a placement with a ratio
    // is a Graphic symbol showing the ONE frame the ratio selects (Flash's "Single Frame").
    let mut unnamed_placements: BTreeMap<u16, (u16, ImageLocalMatrix, u16, Option<u16>)> = BTreeMap::new();
    let mut cur_frame: u16 = 0;
    let mut effect_frames: Vec<Vec<(u16, String, ImageLocalMatrix)>> = Vec::new();
    // the depths holding a child left out by name: nothing placed or restated there is drawn
    let mut skipped_depths: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();

    for stag in &sprite.tags {
        match stag {
            swf::Tag::ShowFrame => {
                let mut snap: Vec<(u16, String, ImageLocalMatrix)> =
                    disp.values().map(|(id, sym, mat)| (*id, sym.clone(), *mat)).collect();
                for (&_depth, (unnamed_id, parent_mat, place_frame, pinned)) in &unnamed_placements {
                    if let Some(uframes) = unnamed_frames.get(unnamed_id) {
                        let uf_idx = match pinned {
                            Some(r) => (*r as usize).min(uframes.len().saturating_sub(1)),
                            None => (cur_frame.saturating_sub(*place_frame) as usize)
                                .min(uframes.len().saturating_sub(1)),
                        };
                        for (inner_id, inner_sym, inner_mat) in &uframes[uf_idx] {
                            let composed = parent_mat.compose(inner_mat);
                            snap.push((*inner_id, inner_sym.clone(), composed));
                        }
                    }
                }
                effect_frames.push(snap);
                cur_frame += 1;
            }
            swf::Tag::PlaceObject(po) => {
                // as above: no matrix stated means the depth's own is kept
                let mat = match po.matrix {
                    Some(_) => po_to_mat(po),
                    None => disp.get(&po.depth).map(|e| e.2)
                        .or_else(|| unnamed_placements.get(&po.depth).map(|e| e.1))
                        .unwrap_or_default(),
                };
                if std::env::var("PEPTIDE_DUMP_EFFECTS").is_ok() {
                    eprintln!("[place] sprite {} frame {} depth {} action {:?} matrix {:?} ratio {:?} cxform {:?} blend {:?} visible {:?}",
                        inner_sprite_id, cur_frame, po.depth, po.action, po.matrix.map(|m| (m.a.to_f32(), m.d.to_f32(), m.tx.get() as f32 / 20.0, m.ty.get() as f32 / 20.0)), po.ratio,
                        po.color_transform.as_ref().map(|c| (c.a_multiply.to_f32(), c.a_add, c.r_multiply.to_f32())), po.blend_mode, po.is_visible);
                }
                if !skip.is_empty() {
                    let placed_name = po.name.as_ref().map(|n| String::from_utf8_lossy(n.as_bytes()).to_string()).unwrap_or_default();
                    if matches!(po.action, swf::PlaceObjectAction::Place(_) | swf::PlaceObjectAction::Replace(_)) {
                        if skip.contains(&placed_name) {
                            skipped_depths.insert(po.depth);
                            disp.remove(&po.depth);
                            unnamed_placements.remove(&po.depth);
                            continue;
                        }
                        skipped_depths.remove(&po.depth);
                    } else if skipped_depths.contains(&po.depth) {
                        continue;
                    }
                }
                if std::env::var("PEPTIDE_DUMP_EFFECTS").is_ok() {
                    if let swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) = &po.action {
                        eprintln!("[place-kind] sprite {} places {cid}: named={:?} sub={} shape_to_bitmap={:?} image={}", inner_sprite_id,
                            symbols.get(cid), unnamed_frames.contains_key(cid), img_result.shape_to_bitmap.get(cid),
                            img_result.shape_to_bitmap.get(cid).and_then(|b| img_result.images.get(b)).map(|i| i.symbol_name.clone()).unwrap_or_default());
                    }
                }
                match &po.action {
                    swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) => {
                        let named = symbols.get(cid);
                        let is_collisonbox = named.map(|s| {
                            let l = s.to_lowercase();
                            l.contains("collisonbox") || l.contains("collisionbox")
                        }).unwrap_or(false);
                        if is_collisonbox {
                            // collision box — not a visual, skip
                        } else if unnamed_frames.contains_key(cid) {
                            // A sub-sprite (named OR unnamed) → recurse into its
                            // frames. Checking this BEFORE the named-symbol case
                            // is essential: a NAMED nested sprite (e.g.
                            // mario_fla.MarioCut_InPATriangle) is an animation,
                            // not an image — treating it as an image emits a
                            // dangling imageAsset (no PNG is ever written for it).
                            unnamed_placements.insert(po.depth, (*cid, mat, cur_frame, po.ratio));
                        } else if let Some(sname) = named {
                            // Named bitmap/shape placed directly → an image.
                            disp.insert(po.depth, (*cid, sname.clone(), mat));
                        } else if let Some(&bitmap_id) = img_result.shape_to_bitmap.get(cid) {
                            // Direct shape placement with bitmap fill
                            if let Some(img) = img_result.images.get(&bitmap_id) {
                                disp.insert(po.depth, (*cid, img.symbol_name.clone(), with_pivot(mat, img_result.shape_pivot.get(cid))));
                            }
                        }
                    }
                    swf::PlaceObjectAction::Modify => {
                        // A MODIFY with no matrix only re-states the object —
                        // keep its existing transform.
                        if po.matrix.is_some() {
                            if let Some(e) = disp.get_mut(&po.depth) { e.2 = mat; }
                            if let Some(e) = unnamed_placements.get_mut(&po.depth) { e.1 = mat; }
                        }
                    }
                }
            }
            swf::Tag::RemoveObject(ro) => {
                disp.remove(&ro.depth);
                unnamed_placements.remove(&ro.depth);
                skipped_depths.remove(&ro.depth);
            }
            _ => {}
        }
    }

    // Convert effect_frames → per-frame symbol names
    let mut frames: Vec<Option<String>> = Vec::new();
    let mut matrices: Vec<Option<ImageLocalMatrix>> = Vec::new();
    let mut image_guids: BTreeMap<String, String> = BTreeMap::new();
    let mut parts: Vec<Vec<(String, ImageLocalMatrix)>> = Vec::new();
    let image_symbols: std::collections::BTreeSet<&str> = img_result.images.values().map(|i| i.symbol_name.as_str()).collect();

    for frame_entries in &effect_frames {
        // a part is a picture the package ships; a named clip that is not one (a nested
        // animation the walk did not flatten) is no image to point an entity at
        let all: Vec<(String, ImageLocalMatrix)> = frame_entries.iter()
            .filter(|(_, sym, _)| !sym.is_empty() && image_symbols.contains(sym.as_str()))
            .map(|(_, sym, mat)| (sym.clone(), *mat)).collect();
        for (sym, _) in &all { image_guids.entry(sym.clone()).or_insert_with(|| crate::uuid_gen::image_meta_guid(sym)); }
        parts.push(all);
        // Each entry now has the resolved symbol_name already (from Pass 1 & 2 fixes).
        // Take the first non-empty symbol name in the frame, WITH its local matrix.
        let entry = frame_entries.iter().find(|(_id, sym, _mat)| !sym.is_empty());
        let sym_name = entry.map(|(_id, sym, _mat)| sym.clone());
        let mat = entry.map(|(_id, _sym, mat)| *mat);
        if let Some(ref sym) = sym_name {
            // Symbol-only GUID (matches get_image_meta_guids) so projectile/effect
            // entity refs resolve to the shared sprite .meta regardless of which
            // char owns them — multi-char projectile/effect sprites no longer
            // dangle into placeholders.
            let meta_guid = crate::uuid_gen::image_meta_guid(sym);
            image_guids.insert(sym.clone(), meta_guid);
        }
        frames.push(sym_name);
        matrices.push(mat);
    }

    Ok(ProjectileFrameImages { frames, matrices, image_guids, parts })
}

/// A shape's raster starts at the shape's own bounds, not at its origin: the placement puts
/// the shape's (0,0) where the matrix says, and the picture's top-left is the bounds' minimum
/// carried through the same matrix. Without this a rasterized vector effect hung off the
/// spawn point by its whole bounds (sora's blizzard drew below the feet where the source draws
/// it over the chest). The character path does the same (`raster`'s `origin_px`).
fn with_pivot(mat: ImageLocalMatrix, pivot: Option<&(f64, f64)>) -> ImageLocalMatrix {
    let Some(&(ox, oy)) = pivot else { return mat };
    if ox == 0.0 && oy == 0.0 { return mat }
    ImageLocalMatrix::from_abcd(mat.a, mat.b, mat.c, mat.d,
        mat.tx + ox * mat.a + oy * mat.c, mat.ty + ox * mat.b + oy * mat.d)
}

/// A part placed inside an animation's clip, where the root places that clip: the two matrices
/// composed in the SWF's own layout, so a clip the root places mirrored (marth's back air) draws
/// its parts mirrored, where the scale alone kept them facing forward on the wrong side
fn placed_by(root: &crate::sprite_parser::XframeTransform, l: &ImageLocalMatrix) -> ImageLocalMatrix {
    let (ra, rb, rc, rd) = (root.a, root.b, root.c, root.d);
    let (tx, ty) = root.apply(l.tx, l.ty);
    ImageLocalMatrix::from_abcd(ra * l.a + rc * l.b, rb * l.a + rd * l.b,
                                ra * l.c + rc * l.d, rb * l.c + rd * l.d, tx, ty)
}

fn po_to_mat(po: &swf::PlaceObject) -> ImageLocalMatrix {
    po.matrix.map(|m| {
        let a = m.a.to_f64(); let b = m.b.to_f64();
        let c = m.c.to_f64(); let d = m.d.to_f64();
        ImageLocalMatrix::from_abcd(a, b, c, d,
            m.tx.get() as f64 / 20.0, m.ty.get() as f64 / 20.0)
    }).unwrap_or_default()
}

// ─── Projectile and menu sprite discovery ───────────────────────────────────────

/// A single named state within a multi-state projectile outer wrapper.
/// SSF2 uses frame labels on the outer sprite to switch between inner sprites.
#[derive(Debug, Clone)]
pub struct ProjectileState {
    /// SSF2 frame label (e.g. "attack_hold", "attack_idle", "attack_toss")
    pub label: String,
    /// Inner animation sprite ID for this state
    pub inner_sprite_id: u16,
    /// Inner animation sprite name (e.g. "link_fla.BombHeld_152")
    pub inner_sprite_name: String,
    /// Frame count of the inner sprite
    pub inner_frame_count: u16,
    /// where the projectile's clip places this stance: its art and boxes are in the stance's
    /// own space, and the projectile's origin is the outer clip's
    pub stance: ImageLocalMatrix,
}

/// A projectile sprite discovered in the SWF.
#[derive(Debug, Clone)]
pub struct DiscoveredProjectile {
    /// Root projectile sprite ID
    pub sprite_id: u16,
    /// Root projectile name (e.g. "mario_fireball")
    pub name: String,
    /// The class the root sprite is bound to (its symbol name), which stays put when the
    /// projectile is renamed to what the fighter's table calls it: the clip's own code is this
    /// class's and its stance's, never a class that happens to share the table's name.
    pub class_name: String,
    /// Inner animation sprite ID (the 'stance' child), if any (single-state projectiles)
    pub inner_sprite_id: Option<u16>,
    /// Inner animation sprite name (e.g. "mario_fla.mario_fireball_mc_210")
    pub inner_sprite_name: Option<String>,
    /// Frame count of the inner animation sprite
    pub inner_frame_count: u16,
    /// where the projectile's clip places the stance (see `ProjectileState::stance`)
    pub inner_stance: ImageLocalMatrix,
    /// For multi-state projectiles (outer wrapper has multiple frame labels),
    /// each entry is one SSF2 state with its own inner sprite.
    /// Empty for single-state projectiles.
    pub states: Vec<ProjectileState>,
    /// SSF2 frame labels found INSIDE the inner animation sprite, in
    /// timeline order, as `(frame_1_based, label)`. These are the
    /// projectile's own animation labels — extracted in parallel to the
    /// character's xframe extraction. Empty when the inner sprite has
    /// no FrameLabel tags (in which case the generators fall back to
    /// the FM template's `projectileSpawn`/`projectileIdle`/`projectileDestroy`).
    pub inner_labels: Vec<(u16, String)>,
    /// How many frames the projectile's OWN timeline has, as against the clip placed inside it.
    /// A clip with one frame has no timeline to hold: what moves inside it is the inner clip's.
    pub own_frame_count: u16,
}

/// A visual effect sprite discovered in the SWF root. Effects are like
/// projectiles in that they live on the root with a SymbolClass linkage
/// ID, but they don't carry an `attack_idle` FrameLabel or a `stance`
/// PlaceObject — they're pure visual playback. Converted to standalone
/// `library/entities/<name>.entity` files with no scripts/stats; the
/// character's Script.hx spawns them via `match.createVfx(...)`.
#[derive(Debug, Clone)]
pub struct DiscoveredEffect {
    /// Sprite ID of the effect's root timeline
    pub sprite_id: u16,
    /// SymbolClass name (e.g. `effect_land`, `dee_fs_sparkle`).
    /// Becomes the entity `id` and the `getContent()` lookup key.
    pub name: String,
    /// Frame count of the effect's timeline
    pub frame_count: u16,
    /// SSF2 FrameLabel tags found inside the effect sprite, in
    /// timeline order: `(1-based frame, label)`. Used to name the
    /// emitted entity animations; falls back to a single `vfx`
    /// animation when there are no labels.
    pub inner_labels: Vec<(u16, String)>,
}

/// The head/portrait sprite discovered in the SWF.
#[derive(Debug, Clone)]
pub struct DiscoveredHead {
    /// Sprite ID of the head sprite
    pub sprite_id: u16,
    /// Name of the head sprite (e.g. "mario_head")
    pub name: String,
    /// The image symbol placed inside (e.g. "mario_dm0")
    pub image_symbol: Option<String>,
    /// Shape ID that the head sprite places
    pub image_shape_id: Option<u16>,
}

/// One projectile, read off its root sprite: the `stance` child under each label is the clip it
/// draws with, `attack_idle` the one it flies with, and the labels inside that clip its own
/// animations. Shared by the character walk (every sprite carrying `attack_idle`) and a stage's
/// (the sprites its classes declare by linkage).
fn projectile_of_sprite(swf: &swf::Swf<'_>, symbols: &BTreeMap<u16, String>, sprite: &swf::Sprite, sprite_name: String) -> DiscoveredProjectile {

    // Walk the outer sprite timeline to collect all frame-label → stance placements.
    // Single-state: one label ('attack_idle') + one PlaceObject(stance).
    // Multi-state: multiple labels each followed by a new PlaceObject(stance).
    let mut states: Vec<ProjectileState> = Vec::new();
    let mut cur_label: Option<String> = None;
    for stag in &sprite.tags {
        match stag {
            swf::Tag::FrameLabel(fl) => {
                cur_label = Some(fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
            }
            swf::Tag::PlaceObject(po) => {
                let po_name = po.name.as_ref()
                    .map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
                if po_name.as_deref() == Some("stance") {
                    let stance = po.matrix.map(|m| ImageLocalMatrix::from_abcd(m.a.to_f64(), m.b.to_f64(), m.c.to_f64(), m.d.to_f64(),
                        m.tx.get() as f64 / 20.0, m.ty.get() as f64 / 20.0)).unwrap_or_default();
                    if let swf::PlaceObjectAction::Place(id) = &po.action {
                        let inner_sym = symbols.get(id).cloned().unwrap_or_default();
                        let inner_frames = swf.tags.iter().find_map(|t| {
                            if let swf::Tag::DefineSprite(s) = t {
                                if s.id == *id { return Some(s.num_frames); }
                            }
                            None
                        }).unwrap_or(1);
                        if let Some(label) = cur_label.take() {
                            states.push(ProjectileState {
                                label,
                                inner_sprite_id: *id,
                                inner_sprite_name: inner_sym,
                                inner_frame_count: inner_frames,
                                stance,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Single-state shortcut: find the attack_idle entry
    let idle_state = states.iter().find(|s| s.label == "attack_idle");
    let inner_sprite_id = idle_state.map(|s| s.inner_sprite_id);
    let inner_sprite_name = idle_state.map(|s| s.inner_sprite_name.clone());
    let inner_frame_count = idle_state.map(|s| s.inner_frame_count)
        .unwrap_or_else(|| sprite.num_frames.max(1));
    let inner_stance = idle_state.map(|s| s.stance).unwrap_or_default();

    // Only keep states vec if it has more than just attack_idle
    let states = if states.len() > 1 { states } else { Vec::new() };

    // Walk the inner sprite's tags for its own FrameLabel pings —
    // these are the projectile's per-frame SSF2 animation labels
    // (parallel to character xframe labels). Order by frame.
    let mut inner_labels: Vec<(u16, String)> = Vec::new();
    if let Some(inner_id) = inner_sprite_id {
        if let Some(inner_sprite) = swf.tags.iter().find_map(|t| {
            if let swf::Tag::DefineSprite(s) = t {
                if s.id == inner_id { return Some(s); }
            }
            None
        }) {
            let mut frame: u16 = 1;
            for tag in &inner_sprite.tags {
                match tag {
                    swf::Tag::FrameLabel(fl) => {
                        let label = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                        if !label.is_empty() {
                            inner_labels.push((frame, label));
                        }
                    }
                    swf::Tag::ShowFrame => { frame += 1; }
                    _ => {}
                }
            }
        }
    }

    if std::env::var("PEPTIDE_TRACE_LABELS").is_ok() {
        eprintln!("[trace-proj] {sprite_name} states={:?} labels={inner_labels:?}",
                  states.iter().map(|s| (s.label.clone(), s.inner_frame_count)).collect::<Vec<_>>());
    }
    DiscoveredProjectile {
        sprite_id: sprite.id,
        class_name: sprite_name.clone(),
        name: sprite_name,
        inner_sprite_id,
        inner_sprite_name,
        inner_frame_count,
        inner_stance,
        states,
        inner_labels,
        own_frame_count: sprite.num_frames,
    }
}

/// Every clip class the timeline places inside a projectile: the projectile sprites themselves
/// (the ones carrying `attack_idle`) and the `stance` clip under each of their labels. What
/// these clips do is the projectile's, not the fighter's that fires it.
pub fn projectile_clip_classes(swf: &swf::Swf<'_>) -> BTreeSet<String> {
    let mut symbols: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                if !name.is_empty() { symbols.insert(link.id, name); }
            }
        }
    }
    let mut out: BTreeSet<String> = BTreeSet::new();
    for tag in &swf.tags {
        let swf::Tag::DefineSprite(sprite) = tag else { continue };
        let Some(name) = symbols.get(&sprite.id).cloned() else { continue };
        let has_idle = sprite.tags.iter().any(|t| matches!(t, swf::Tag::FrameLabel(fl)
            if fl.label.to_str_lossy(encoding_rs::WINDOWS_1252) == "attack_idle"));
        if !has_idle { continue }
        let p = projectile_of_sprite(swf, &symbols, sprite, name.clone());
        out.insert(name);
        if let Some(inner) = p.inner_sprite_name { out.insert(inner); }
        for st in p.states { if !st.inner_sprite_name.is_empty() { out.insert(st.inner_sprite_name); } }
    }
    out
}

/// The projectiles a stage's classes declare, found by the linkage each declaration names.
/// A projectile that carries no `attack_idle` of its own is one clip, drawn as it is.
pub fn discover_projectiles_by_linkage(swf: &swf::Swf<'_>, linkages: &[String]) -> Vec<DiscoveredProjectile> {
    let mut symbols: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                if !name.is_empty() { symbols.insert(link.id, name); }
            }
        }
    }
    let mut out = Vec::new();
    for tag in &swf.tags {
        let swf::Tag::DefineSprite(sprite) = tag else { continue };
        let Some(name) = symbols.get(&sprite.id).cloned() else { continue };
        if !linkages.iter().any(|l| l.eq_ignore_ascii_case(&name)) { continue }
        let mut p = projectile_of_sprite(swf, &symbols, sprite, name);
        if p.inner_sprite_id.is_none() {
            p.inner_sprite_id = Some(sprite.id);
            p.inner_sprite_name = Some(p.name.clone());
            p.inner_frame_count = sprite.num_frames.max(1);
        }
        out.push(p);
    }
    out
}

/// Scan the SWF for projectile, effect, and head/portrait sprites.
///
///   - Projectiles → have an `attack_idle` FrameLabel + a `stance`
///     PlaceObject inside.
///   - Effects → a root-level SymbolClass'd sprite with NEITHER of
///     those, AND not matched by the head/UI carve-outs (`*_head`,
///     `*_hud`, `*_icon`, `*_Symbol`, the character's own sprite, or
///     auto-generated `_fla.*` timelines).
///   - Head → matched by `*_head` etc.; see `find_receiver_start`-style
///     name heuristics for menu.entity rendering.
///
/// Returns (projectiles, effects, head_sprite).
pub fn discover_projectiles_and_head(
    swf_data: &[u8],
    char_name: &str,
    declared_thumbnail: Option<&str>,
) -> Result<(Vec<DiscoveredProjectile>, Vec<DiscoveredEffect>, Option<DiscoveredHead>)> {
    use std::io::Cursor;
    let swf_buf = swf::decompress_swf(Cursor::new(swf_data)).context("decompress SWF")?;
    let swf = swf::parse_swf(&swf_buf).context("parse SWF")?;
    discover_projectiles_and_head_from_swf(&swf, char_name, declared_thumbnail)
}

/// `discover_projectiles_and_head` against an already-parsed SWF.
pub fn discover_projectiles_and_head_from_swf(
    swf: &swf::Swf<'_>,
    char_name: &str,
    declared_thumbnail: Option<&str>,
) -> Result<(Vec<DiscoveredProjectile>, Vec<DiscoveredEffect>, Option<DiscoveredHead>)> {
    // Build SymbolClass map: id → name
    let mut symbols: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                if !name.is_empty() {
                    symbols.insert(link.id, name);
                }
            }
        }
    }

    // Pre-compute which IDs are bitmaps. The head sprite usually places a
    // mix of Shapes (outlines/backgrounds) and one Bitmap (the actual pixel
    // art portrait). We need to prefer the bitmap when scanning inner tags.
    let mut bitmap_ids: BTreeSet<u16> = BTreeSet::new();
    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineBitsLossless(b) => { bitmap_ids.insert(b.id); }
            swf::Tag::DefineBitsJpeg2 { id, .. } => { bitmap_ids.insert(*id); }
            swf::Tag::DefineBitsJpeg3(d) => { bitmap_ids.insert(d.id); }
            _ => {}
        }
    }

    // Scan DefineSprite tags
    let mut projectiles: Vec<DiscoveredProjectile> = Vec::new();
    let mut head: Option<DiscoveredHead> = None;
    let char_lower = char_name.to_lowercase();

    // Build candidate-head ranking. Every SSF2 character has pixel-art at
    // the SWF root, but the naming varies — the cross-character survey
    // showed `*_head` (most chars: dee_head, blackmage_head, fox_head,
    // bomberman_head, etc.) and `*_icon` (sandbag_icon). `*_hud` is the
    // in-game damage-counter HUD (animated, hundreds of frames), NOT a
    // menu portrait — explicitly excluded. Head sprites named without an
    // underscore (dkhead, dkicon) only contain shapes — they need shape
    // rasterization which is a separate code path.
    // The character's own root MovieClip names it too, and often differently from the id
    // (`dkong` for `donkeykong`), which is the half of its identity `dkhead` is built from.
    let root_name = crate::sprite_parser::find_root_sprite_id(swf, &symbols, &char_lower)
        .and_then(|id| symbols.get(&id).map(|n| n.to_lowercase()));
    let mut candidates: Vec<&swf::Sprite> = Vec::new();
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sprite_name = symbols.get(&sprite.id).cloned().unwrap_or_default();
            if sprite_name.is_empty() { continue; }
            let nl = sprite_name.to_lowercase();
            // Skip names that look like in-game gameplay assets (cutins,
            // weapons, HUDs).
            if nl.contains("cutin") || nl.contains("trail") || nl.contains("fs_")
                || nl.ends_with("_hud") || nl == format!("{}_hud", char_lower)
            {
                continue;
            }
            // THE portrait is the clip the character's own stats name as its `thumbnail` --
            // what SSF2's damage counter shows for it. Nothing is recognised here; the source
            // said which clip it is.
            let declared = declared_thumbnail.is_some_and(|t| t.eq_ignore_ascii_case(&sprite_name));
            // Only when a package declares none: the naming convention, as a last resort. An
            // EMPTY clip never counts (several characters ship a `_head` symbol with nothing in
            // it), and neither does an auto-generated `_fla.` timeline that happens to carry the
            // word: a headbutt animation is a move, not a picture.
            let by_convention = declared_thumbnail.is_none()
                && (nl.contains("head") || nl == format!("{char_lower}_icon"))
                && sprite.num_frames <= 1
                && !nl.contains("_fla.");
            let has_content = sprite.tags.iter().any(|t| matches!(t, swf::Tag::PlaceObject(_)));
            if (declared || by_convention) && has_content {
                candidates.push(sprite);
            }
        }
    }
    // Stable preference: _head > _icon. _head is the standard convention;
    // _icon is the CSS thumbnail (sandbag uses this since it has no _head).
    // Only reachable when nothing was declared. A file can carry someone else's head clip (an
    // example, a cameo), so the fallback pick is tied to THIS character's identity: how much of
    // the name it shares with the id the package registers or with its own root MovieClip.
    // `dkhead` shares "dk" with `dkong`; an unrelated `ichigo_head_example` shares nothing.
    let shared = |n: &str| -> usize {
        let pre = |a: &str, b: &str| a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
        pre(n, &char_lower).max(root_name.as_deref().map(|r| pre(n, r)).unwrap_or(0))
    };
    if declared_thumbnail.is_none() {
        candidates.sort_by_key(|s| {
            let n = symbols.get(&s.id).map(|s| s.to_lowercase()).unwrap_or_default();
            let kind = if n.ends_with("_head") { 0 } else if n.contains("head") { 1 } else { 2 };
            (kind, std::cmp::Reverse(shared(&n)))
        });
        candidates.retain(|s| {
            let n = symbols.get(&s.id).map(|s| s.to_lowercase()).unwrap_or_default();
            shared(&n) >= 2
        });
    }

    for sprite in candidates {
        let sprite_name = symbols.get(&sprite.id).cloned().unwrap_or_default();
        // Walk all inner PlaceObject tags. Prefer a placement whose target
        // is a Bitmap with a SymbolClass name — the actual portrait. Fall
        // back to any placement with a symbol name, then to anything.
        let mut best_bitmap: Option<(u16, String)> = None;
        let mut best_named: Option<(u16, String)> = None;
        let mut first_placement: Option<u16> = None;
        for stag in &sprite.tags {
            if let swf::Tag::PlaceObject(po) = stag {
                if let swf::PlaceObjectAction::Place(id) = &po.action {
                    if first_placement.is_none() { first_placement = Some(*id); }
                    let sym = symbols.get(id).cloned().unwrap_or_default();
                    if !sym.is_empty() {
                        if bitmap_ids.contains(id) && best_bitmap.is_none() {
                            best_bitmap = Some((*id, sym.clone()));
                        }
                        if best_named.is_none() {
                            best_named = Some((*id, sym));
                        }
                    }
                }
            }
        }
        let (image_shape_id, image_symbol) = match (best_bitmap, best_named, first_placement) {
            (Some((id, sym)), _, _) => (Some(id), Some(sym)),
            (None, Some((id, sym)), _) => (Some(id), Some(sym)),
            (None, None, Some(id)) => (Some(id), None),
            _ => (None, None),
        };

        // a portrait built out of bare SHAPES has no symbol on any placement; it is still the
        // portrait, and the compositing path is what turns it into one image.
        if image_symbol.is_some() || image_shape_id.is_some() {
            log::debug!("Head sprite '{}': resolved image symbol '{}' (id={:?})",
                sprite_name, image_symbol.as_deref().unwrap_or(""), image_shape_id);
            head = Some(DiscoveredHead {
                sprite_id: sprite.id,
                name: sprite_name,
                image_symbol,
                image_shape_id,
            });
            break;
        }
    }

    // The remainder of the scan (projectile detection) walks the same
    // DefineSprite tags but doesn't depend on `head`.
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sprite_name = symbols.get(&sprite.id).cloned().unwrap_or_default();
            if sprite_name.is_empty() { continue; }

            // Check for projectile: has 'attack_idle' label
            let has_attack_idle = sprite.tags.iter().any(|t| {
                if let swf::Tag::FrameLabel(fl) = t {
                    fl.label.to_str_lossy(encoding_rs::WINDOWS_1252) == "attack_idle"
                } else {
                    false
                }
            });
            if !has_attack_idle { continue; }

            // Skip the root character sprite
            if sprite_name.to_lowercase() == char_lower { continue; }

            projectiles.push(projectile_of_sprite(swf, &symbols, sprite, sprite_name));
        }
    }

    // ── Effects pass ────────────────────────────────────────────────────
    // A second walk picks up root-level SymbolClass'd sprites that aren't
    // projectiles, the character itself, or UI carve-outs. Effects are
    // pure visual sprites (no `attack_idle`, no `stance`).
    //
    // We exclude BOTH the projectile root sprite IDs AND each projectile's
    // inner animation sprite IDs. The inner sprite is where the projectile
    // actually renders its frames (e.g. `deeFinalSmashProjectile` is the
    // inner sprite under `dee_finalsmash`'s `stance` PlaceObject). Without
    // this exclusion, those inner sprites get re-discovered here and
    // emitted as phantom effect entities — and their ids tend to
    // case-collide with the projectile entity's `<name>Projectile` id
    // (e.g. `deeFinalSmashProjectile` vs `deefinalsmashProjectile`),
    // which FrayTools rejects as a duplicate-id error when it normalizes
    // ids for cross-resource lookup.
    let mut projectile_ids: std::collections::BTreeSet<u16> =
        projectiles.iter().map(|p| p.sprite_id).collect();
    for p in &projectiles {
        if let Some(inner) = p.inner_sprite_id {
            projectile_ids.insert(inner);
        }
        for s in &p.states {
            projectile_ids.insert(s.inner_sprite_id);
        }
    }
    let head_id = head.as_ref().map(|h| h.sprite_id);
    let mut effects: Vec<DiscoveredEffect> = Vec::new();
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let name = symbols.get(&sprite.id).cloned().unwrap_or_default();
            if name.is_empty() { continue; }
            // Auto-generated timeline classes — never effects.
            if name.contains("_fla.") { continue; }
            // Already classified.
            if projectile_ids.contains(&sprite.id) { continue; }
            if Some(sprite.id) == head_id { continue; }
            // The character's own sprite — has a `stance` PlaceObject but
            // no `attack_idle` (which is why the projectile pass skipped
            // it). Skip by name match.
            if name.to_lowercase() == char_lower { continue; }
            // UI carve-outs: HUDs, franchise symbols, head/icon helpers
            // that menu.entity already handles. Match by suffix or by
            // shared SSF2 convention.
            let nl = name.to_lowercase();
            if nl.ends_with("_hud") || nl.ends_with("_symbol")
                || nl.ends_with("_icon") || nl.ends_with("icon")
                || nl.ends_with("_head") || nl == "head"
                || nl == "sparkle"                       // generic 1-frame placeholder
            { continue; }
            // Also skip sprites that carry a `stance` PlaceObject — these
            // are sub-characters or sub-projectiles (rare but observed).
            let has_stance = sprite.tags.iter().any(|t| {
                if let swf::Tag::PlaceObject(po) = t {
                    po.name.as_ref()
                        .map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252) == "stance")
                        .unwrap_or(false)
                } else { false }
            });
            if has_stance { continue; }

            // Collect inner-sprite FrameLabel tags for the entity's
            // animation names. Same shape as projectile inner_labels.
            let mut inner_labels: Vec<(u16, String)> = Vec::new();
            let mut frame: u16 = 1;
            for t in &sprite.tags {
                match t {
                    swf::Tag::FrameLabel(fl) => {
                        let label = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                        if !label.is_empty() { inner_labels.push((frame, label)); }
                    }
                    swf::Tag::ShowFrame => { frame += 1; }
                    _ => {}
                }
            }

            effects.push(DiscoveredEffect {
                sprite_id: sprite.id,
                name,
                frame_count: sprite.num_frames.max(1),
                inner_labels,
            });
        }
    }

    Ok((projectiles, effects, head))
}

/// Composite a character's portrait clip into one image.
///
/// A portrait clip is a little scene, not a picture: it stacks pieces at different depths, scales
/// the character art down inside itself, nests other clips, and MASKS ranges of depths to windows
/// so only the head shows. Everything here comes from the clip -- the depth order, each piece's
/// own matrix, and the masks. A placement carrying a `clip_depth` masks the depths it covers,
/// which is how SWF states masking, so this needs no idea what it is looking at and a portrait
/// nobody has drawn yet composites the same way.
///
/// NOTHING RESAMPLES THE PIXELS. A costume recolors by matching colors EXACTLY, so a resampled
/// pixel is either an invented color the palette says nothing about or a neighbour's color in the
/// wrong place, and either way it survives the swap wearing the base art's color. The composite is
/// built at the resolution of its largest piece and the engine scales it into its frame at draw
/// time, after the swap, which is where a scale belongs.
///
/// With `apply_masks` the result is cropped to what the clip's windows frame -- the head the
/// damage counter shows. Without, it is the whole picture, which is what the character-select
/// screen shows.
pub fn composite_head(
    swf: &swf::Swf<'_>,
    head_sprite_id: u16,
    output_dir: &Path,
    char_name: &str,
    apply_masks: bool,
) -> Result<Option<ExtractedImage>> {
    let ctx = ClipContext::new(swf);
    let Some(r) = ctx.render(head_sprite_id, 0) else { return Ok(None) };
    if std::env::var("PEPTIDE_HEAD_DEBUG").is_ok() {
        eprintln!("[head] canvas {}x{} origin ({:.1},{:.1}) scale {:.2} window {:?}",
            r.w, r.h, r.x, r.y, r.scale, r.window);
    }

    // Keep only what the clip's windows frame. Everything outside them is the rest of the picture.
    let window = r.window;
    let (canvas, cw, ch) = match (apply_masks, window) {
        (true, Some((x0, y0, x1, y1))) => {
            let cx0 = ((x0 - r.x).floor().max(0.0)) as u32;
            let cy0 = ((y0 - r.y).floor().max(0.0)) as u32;
            let cx1 = (((x1 - r.x).ceil()) as u32).min(r.w).max(cx0 + 1);
            let cy1 = (((y1 - r.y).ceil()) as u32).min(r.h).max(cy0 + 1);
            crop(&r.rgba, r.w, cx0, cy0, cx1 - cx0, cy1 - cy0)
        }
        _ => (r.rgba, r.w, r.h),
    };

    // Keep the window's own margins. What sits where inside the frame is the source's framing,
    // and every character's window is about the same size (52x38 in nearly all of them), so
    // keeping it is what makes one character's head the same size as the next one's, the way
    // SSF2's counter shows them. Trimming to the art would scale each picture by how much of its
    // own window it happens to fill. With no window there is nothing to frame, so the whole
    // picture is trimmed to what was painted; an empty result means the clip drew nothing.
    let (canvas, cw, ch) = match window {
        Some(_) if apply_masks => (canvas, cw, ch),
        _ => {
            let Some(t) = trim_transparent(canvas, cw, ch) else { return Ok(None) };
            t
        }
    };

    let sym = if apply_masks {
        format!("{char_name}_head_composite")
    } else {
        format!("{char_name}_head_full")
    };
    let filename = format!("{}.png", sanitize_name(&sym));
    let sprites_dir = output_dir.join("library/sprites");
    std::fs::create_dir_all(&sprites_dir)?;
    let mut batch = PngBatch::default();
    batch.push(sprites_dir.join(&filename), cw, ch, canvas)?;
    batch.flush()?;
    Ok(Some(ExtractedImage {
        bitmap_id: head_sprite_id,
        symbol_name: sym,
        width: cw,
        height: ch,
        png_path: format!("library/sprites/{filename}"),
        is_vector: true,
    }))
}

/// A rendered clip: its pixels, and where its top-left sits in the coordinates of whatever placed
/// it. `window` is the rectangle its masks frame, in those same coordinates.
struct Rendered {
    rgba: Vec<u8>,
    w: u32,
    h: u32,
    x: f64,
    y: f64,
    /// Pixels per unit of the clip's own coordinates. A clip is rendered at the resolution of its
    /// largest piece, so whoever places it has to divide by this to know how BIG it is.
    scale: f64,
    window: Option<(f64, f64, f64, f64)>,
}

/// One rasterised placement, already sized and positioned by its own matrix.
struct Piece { x: f64, y: f64, w: u32, h: u32, r: crate::vector_raster::Raster }

/// The definitions a clip draws from: shapes, bitmaps, and the other clips it nests.
struct ClipContext<'a> {
    shapes: BTreeMap<u16, &'a swf::Shape>,
    sprites: BTreeMap<u16, &'a swf::Sprite<'a>>,
    bitmaps: BTreeMap<u16, (u32, u32, Vec<u8>)>,
}

/// How deep a clip may nest before this gives up. Portraits nest a level or two; the limit is
/// there so a clip that places itself can't spin.
const MAX_CLIP_DEPTH: u32 = 4;

impl<'a> ClipContext<'a> {
    fn new(swf: &'a swf::Swf<'a>) -> Self {
        let mut ctx = ClipContext {
            shapes: BTreeMap::new(), sprites: BTreeMap::new(), bitmaps: BTreeMap::new(),
        };
        for tag in &swf.tags {
            match tag {
                swf::Tag::DefineShape(s) => { ctx.shapes.insert(s.id, s); }
                swf::Tag::DefineSprite(s) => { ctx.sprites.insert(s.id, s); }
                swf::Tag::DefineBitsLossless(b) => {
                    if let Ok(rgba) = decode_lossless(b) {
                        ctx.bitmaps.insert(b.id, (b.width as u32, b.height as u32, rgba));
                    }
                }
                swf::Tag::DefineBitsJpeg3(j) => {
                    if let Ok((w, h, rgba)) = decode_jpeg3(j) { ctx.bitmaps.insert(j.id, (w, h, rgba)); }
                }
                // A picture can be stored any of the ways the format allows, and a clip that
                // places one we cannot read loses that piece silently -- which is how a portrait
                // ended up as its background plus whatever else was in the clip.
                swf::Tag::DefineBitsJpeg2 { id, jpeg_data } => {
                    if let Ok((w, h, rgba)) = decode_jpeg_bytes(jpeg_data) { ctx.bitmaps.insert(*id, (w, h, rgba)); }
                }
                swf::Tag::DefineBits { id, jpeg_data } => {
                    if let Ok((w, h, rgba)) = decode_jpeg_bytes(jpeg_data) { ctx.bitmaps.insert(*id, (w, h, rgba)); }
                }
                _ => {}
            }
        }
        ctx
    }

    /// One placed piece, at its own resolution: a bitmap, a shape, or a nested clip rendered the
    /// same way this one is.
    fn raster(&self, id: u16, depth: u32) -> Option<(crate::vector_raster::Raster, f64)> {
        if let Some((bw, bh, rgba)) = self.bitmaps.get(&id) {
            return Some((crate::vector_raster::Raster {
                width: *bw, height: *bh, rgba: rgba.clone(), origin_px: (0.0, 0.0),
            }, 1.0));
        }
        if let Some(shape) = self.shapes.get(&id) {
            let b = &shape.shape_bounds;
            let (bw, bh) = ((b.x_max.to_pixels() - b.x_min.to_pixels()).ceil().max(1.0),
                            (b.y_max.to_pixels() - b.y_min.to_pixels()).ceil().max(1.0));
            let painted = (bw <= 4096.0 && bh <= 4096.0)
                .then(|| crate::stage_parser::bitmap_fill_layers(shape, &self.bitmaps, bw as u32, bh as u32))
                .flatten()
                .map(|img| crate::vector_raster::Raster {
                    width: img.width(), height: img.height(), rgba: img.into_raw(),
                    origin_px: (b.x_min.to_pixels(), b.y_min.to_pixels()),
                });
            return painted.or_else(|| crate::vector_raster::rasterize_shape(
                &shape.shape_bounds, &shape.styles.fill_styles, &shape.styles.line_styles, &shape.shape))
                .map(|r| (r, 1.0));
        }
        if depth < MAX_CLIP_DEPTH && self.sprites.contains_key(&id) {
            // a nested clip is a picture too: render it, masks and all, and place the result
            let r = self.render(id, depth + 1)?;
            // its origin is stated in ITS pixels; the caller works in clip units
            return Some((crate::vector_raster::Raster {
                width: r.w, height: r.h, rgba: r.rgba,
                origin_px: (r.x / r.scale, r.y / r.scale),
            }, r.scale));
        }
        None
    }

    /// Render one clip's first frame: every piece it places, in depth order, through its masks.
    fn render(&self, sprite_id: u16, depth: u32) -> Option<Rendered> {
        let sprite = self.sprites.get(&sprite_id)?;

        // The picture is the clip's FIRST frame: everything placed before it shows at once.
        struct Placed { depth: u16, clip: Option<u16>, id: u16, mat: ImageLocalMatrix }
        let mut placed: Vec<Placed> = Vec::new();
        for stag in &sprite.tags {
            match stag {
                swf::Tag::PlaceObject(po) => {
                    if let swf::PlaceObjectAction::Place(id) = &po.action {
                        placed.push(Placed { depth: po.depth, clip: po.clip_depth, id: *id, mat: po_to_mat(po.as_ref()) });
                    }
                }
                swf::Tag::ShowFrame => break,
                _ => {}
            }
        }
        if placed.is_empty() { return None }
        placed.sort_by_key(|p| p.depth);

        // RESOLUTION. A clip usually places its art scaled DOWN inside itself (mario's picture
        // sits at 0.55), and baking that shrink in would be a resample. So the composite is built
        // at the resolution of its largest piece and every piece is drawn at its own pixels.
        // Capped so a huge picture can't blow the canvas up.
        let smallest = placed.iter()
            .filter(|p| p.clip.is_none())
            .map(|p| p.mat.a.abs().min(p.mat.d.abs()))
            .filter(|s| *s > 0.01)
            .fold(1.0f64, f64::min);
        let factor = (1.0 / smallest).clamp(1.0, 4.0);

        let piece = |id: u16, mat: &ImageLocalMatrix| -> Option<Piece> {
            let (r, own) = self.raster(id, depth)?;
            // `own` is the piece's own pixels per unit (1 for a shape or a bitmap, more for a
            // nested clip rendered at its art's resolution), so the placement's scale is applied
            // in UNITS and the result converted back to this clip's pixels.
            let (sx, sy) = (mat.a.abs().max(1e-6) * factor / own, mat.d.abs().max(1e-6) * factor / own);
            let (w, h) = (((r.width as f64) * sx).round().max(1.0) as u32,
                          ((r.height as f64) * sy).round().max(1.0) as u32);
            Some(Piece {
                x: (mat.tx + r.origin_px.0 * mat.a.abs()) * factor,
                y: (mat.ty + r.origin_px.1 * mat.d.abs()) * factor,
                w, h, r,
            })
        };

        // A placement carrying a clip_depth paints nothing of its own: it is a window over the
        // depths it covers, and only what falls inside it survives.
        let mut masks: Vec<(u16, u16, Piece)> = Vec::new();
        let mut drawn: Vec<(u16, Piece)> = Vec::new();
        for p in &placed {
            let Some(pc) = piece(p.id, &p.mat) else { continue };
            match p.clip {
                Some(to) => masks.push((p.depth, to, pc)),
                None => drawn.push((p.depth, pc)),
            }
        }
        if drawn.is_empty() { return None }
        if std::env::var("PEPTIDE_HEAD_DEBUG").is_ok() {
            for (d, p) in &drawn {
                eprintln!("[piece] depth={d} at ({:.1},{:.1}) size {}x{} (raster {}x{})",
                    p.x, p.y, p.w, p.h, p.r.width, p.r.height);
            }
            for (f, t, m) in &masks {
                eprintln!("[mask ] depths {f}..{t} at ({:.1},{:.1}) size {}x{}", m.x, m.y, m.w, m.h);
            }
        }

        let min_x = drawn.iter().map(|(_, p)| p.x).fold(f64::INFINITY, f64::min).floor();
        let min_y = drawn.iter().map(|(_, p)| p.y).fold(f64::INFINITY, f64::min).floor();
        let max_x = drawn.iter().map(|(_, p)| p.x + p.w as f64).fold(f64::NEG_INFINITY, f64::max).ceil();
        let max_y = drawn.iter().map(|(_, p)| p.y + p.h as f64).fold(f64::NEG_INFINITY, f64::max).ceil();
        let (cw, ch) = ((max_x - min_x) as u32, (max_y - min_y) as u32);
        if cw == 0 || ch == 0 || cw > 4096 || ch > 4096 { return None }

        let mut canvas = vec![0u8; (cw as usize) * (ch as usize) * 4];
        for (d, p) in &drawn {
            // every window covering this depth has to let the pixel through
            let covering: Vec<(&Piece, f64)> = masks.iter()
                .filter(|(from, to, _)| d > from && d <= to)
                .map(|(_, _, m)| (m, peak_alpha(&m.r)))
                .collect();
            for dy in 0..p.h {
                for dx in 0..p.w {
                    let (tx, ty) = ((p.x - min_x).round() as i64 + dx as i64,
                                    (p.y - min_y).round() as i64 + dy as i64);
                    if tx < 0 || ty < 0 || tx >= cw as i64 || ty >= ch as i64 { continue }
                    let si = sample_index(&p.r, dx, dy, p.w, p.h);
                    let mut sa = p.r.rgba[si + 3] as f64 / 255.0;
                    for (m, peak) in &covering {
                        sa *= mask_alpha_at(m, *peak, tx as f64 + min_x, ty as f64 + min_y);
                        if sa <= 0.0 { break }
                    }
                    if sa <= 0.0 { continue }
                    let di = ((ty as u32 * cw + tx as u32) * 4) as usize;
                    // straight-alpha source-over: dividing by the result alpha is what keeps a
                    // half-covered pixel its own COLOR instead of a darkened blend toward nothing
                    let da = canvas[di + 3] as f64 / 255.0;
                    let oa = sa + da * (1.0 - sa);
                    if oa > 0.0 {
                        for k in 0..3 {
                            let sc = p.r.rgba[si + k] as f64;
                            let dc = canvas[di + k] as f64;
                            canvas[di + k] = ((sc * sa + dc * da * (1.0 - sa)) / oa)
                                .round().clamp(0.0, 255.0) as u8;
                        }
                    }
                    canvas[di + 3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }

        // What the windows frame, in this clip's own coordinates.
        let window = masks.iter().fold(None, |acc: Option<(f64, f64, f64, f64)>, (_, _, m)| {
            let r = (m.x, m.y, m.x + m.w as f64, m.y + m.h as f64);
            Some(match acc {
                Some(w) => (w.0.min(r.0), w.1.min(r.1), w.2.max(r.2), w.3.max(r.3)),
                None => r,
            })
        });

        Some(Rendered { rgba: canvas, w: cw, h: ch, x: min_x, y: min_y, scale: factor, window })
    }
}

/// Nearest source pixel for a destination pixel of a scaled piece: a placement scaled by its
/// matrix lands at the size it is placed at, with its colors untouched (which keeps a costume's
/// exact-color matching working).
fn sample_index(r: &crate::vector_raster::Raster, dx: u32, dy: u32, w: u32, h: u32) -> usize {
    let sx = (((dx as f64 + 0.5) * r.width as f64 / w as f64) as u32).min(r.width - 1);
    let sy = (((dy as f64 + 0.5) * r.height as f64 / h as f64) as u32).min(r.height - 1);
    ((sy * r.width + sx) * 4) as usize
}

/// How much of a pixel a window lets through, at a point in the clip's own coordinates.
///
/// A mask masks by its SHAPE, not by how opaque it was drawn: a window painted at 62% alpha still
/// shows what is under it at full strength. So coverage is measured against the mask's own peak
/// alpha, and read smoothly, which keeps a curved window's edge from stair-stepping. Interpolating
/// alpha is safe where interpolating color would not be: a costume matches colors exactly, and
/// these pixels keep the color the art gave them.
fn mask_alpha_at(m: &Piece, peak: f64, x: f64, y: f64) -> f64 {
    let (lx, ly) = (x - m.x, y - m.y);
    if lx < 0.0 || ly < 0.0 || lx >= m.w as f64 || ly >= m.h as f64 || peak <= 0.0 { return 0.0 }
    let (fx, fy) = (lx * m.r.width as f64 / m.w as f64 - 0.5,
                    ly * m.r.height as f64 / m.h as f64 - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let a_at = |px: f64, py: f64| -> f64 {
        let cx = (px.max(0.0) as u32).min(m.r.width - 1);
        let cy = (py.max(0.0) as u32).min(m.r.height - 1);
        m.r.rgba[((cy * m.r.width + cx) * 4 + 3) as usize] as f64
    };
    let top = a_at(x0, y0) * (1.0 - tx) + a_at(x0 + 1.0, y0) * tx;
    let bot = a_at(x0, y0 + 1.0) * (1.0 - tx) + a_at(x0 + 1.0, y0 + 1.0) * tx;
    ((top * (1.0 - ty) + bot * ty) / peak).min(1.0)
}

/// The most opaque pixel in a raster, as a 0-255 value.
fn peak_alpha(r: &crate::vector_raster::Raster) -> f64 {
    r.rgba.chunks_exact(4).map(|px| px[3]).max().unwrap_or(0) as f64
}

/// A rectangle of an RGBA image.
fn crop(rgba: &[u8], w: u32, x: u32, y: u32, cw: u32, ch: u32) -> (Vec<u8>, u32, u32) {
    let mut out = vec![0u8; (cw as usize) * (ch as usize) * 4];
    for row in 0..ch {
        let src = (((y + row) * w + x) * 4) as usize;
        let dst = ((row * cw) * 4) as usize;
        out[dst..dst + (cw as usize) * 4].copy_from_slice(&rgba[src..src + (cw as usize) * 4]);
    }
    (out, cw, ch)
}

/// Drop fully transparent rows and columns from the edges. `None` when nothing was painted.
fn trim_transparent(rgba: Vec<u8>, w: u32, h: u32) -> Option<(Vec<u8>, u32, u32)> {
    let at = |x: u32, y: u32| rgba[((y * w + x) * 4 + 3) as usize];
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            if at(x, y) != 0 {
                x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y);
            }
        }
    }
    if x1 < x0 || y1 < y0 { return None }
    let (nw, nh) = (x1 - x0 + 1, y1 - y0 + 1);
    if (nw, nh) == (w, h) { return Some((rgba, w, h)) }
    let mut out = vec![0u8; (nw as usize) * (nh as usize) * 4];
    for y in 0..nh {
        let src = (((y + y0) * w + x0) * 4) as usize;
        let dst = ((y * nw) * 4) as usize;
        out[dst..dst + (nw as usize) * 4].copy_from_slice(&rgba[src..src + (nw as usize) * 4]);
    }
    Some((out, nw, nh))
}

/// A PROJECTILE A CHARACTER SPAWNS THAT ITS OWN FILE DOES NOT CARRY. The game keeps shared
/// packages loaded and a clip resolves out of them exactly as a class does, which is how a
/// fighter can spawn something it never ships: chibirobo fires a blaster and carries no such
/// clip at all, so the shot came out of a reference to nothing.
///
/// Same walk the shared sounds take: the packages the game keeps loaded, then every package
/// beside it, then the game's own movie last, stopping as soon as nothing is still wanted.
pub fn pull_shared_projectiles(
    want: &mut BTreeSet<String>,
    beside: Option<&std::path::Path>,
) -> Vec<DiscoveredProjectile> {
    let mut got: Vec<DiscoveredProjectile> = Vec::new();
    if want.is_empty() { return got }

    // Discovery is scoped to the character whose file it is walking, and a shared package has
    // no such root, so it cannot be asked "find chibirobo's clips" here. What it CAN be asked is
    // "find this clip", so the package's own symbol names are matched against what is wanted,
    // folded the same way a spawn is, and the clip that answers is scoped to itself.
    // Discovery is scoped to whoever owns the clips it is walking, so a package is asked under
    // every scope it could answer to: its own name, which is how a CHARACTER package finds the
    // clips it ships, and the names of any symbols that answer to what is wanted, which is how
    // a shared asset package finds a loose one. kirby's copy abilities need the first: falco's
    // laser is falco's clip, in falco's file, and only falco's scope finds it there.
    fn take(
        bytes: &[u8], from: &str, package_id: &str,
        want: &mut BTreeSet<String>, got: &mut Vec<DiscoveredProjectile>,
    ) {
        let Ok(buf) = swf::decompress_swf(bytes) else { return };
        let Ok(parsed) = swf::parse_swf(&buf) else { return };
        let abcs = crate::projectile_gen::parse_abcs(&parsed);
        // WHAT THE PACKAGE CALLS ITSELF, said by the package. Searching one means searching it
        // under the name whoever owns its clips answers to, and the file is not obliged to be
        // called that: the game keeps its fighters in numbered archives, so a package holding
        // one is searched under a number and finds nothing of it. The package states the name
        // itself, in the class that holds the fighter's own stats.
        let mut scopes: Vec<String> = Vec::new();
        for abc in &abcs {
            for c in &abc.classes {
                let Some(stem) = c.name.strip_suffix("Ext") else { continue };
                if stem.is_empty() || c.name == "SSF2CharacterExt" { continue }
                let n = stem.to_string();
                if !scopes.contains(&n) { scopes.push(n) }
            }
        }
        let from_file = package_id.trim_end_matches(".ssf").to_string();
        if !from_file.is_empty() && !scopes.contains(&from_file) { scopes.push(from_file); }
        for tag in &parsed.tags {
            if let swf::Tag::SymbolClass(links) = tag {
                for link in links {
                    let n = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                    if !n.is_empty()
                        && want.contains(&crate::projectile_gen::projectile_content_id(&n))
                        && !scopes.contains(&n)
                    {
                        scopes.push(n);
                    }
                }
            }
        }
        // everything this package draws with, kept whether it was asked for or not: a name is
        // often fired through a DIFFERENT clip, and which one is stated by the package's own
        // table further down.
        let mut seen: Vec<DiscoveredProjectile> = Vec::new();
        for scope in &scopes {
            let Ok((found, _, _)) =
                discover_projectiles_and_head_from_swf(&parsed, scope, None) else { continue };
            let mut mine: Vec<DiscoveredProjectile> = Vec::new();
            for p in found {
                let id = crate::projectile_gen::projectile_content_id(&p.name);
                if want.remove(&id) { mine.push(p.clone()); }
                if !seen.iter().any(|q| q.name == p.name) { seen.push(p); }
            }
            if !mine.is_empty() {
                log::info!("{} projectile(s) came across from {from}", mine.len());
                got.extend(mine);
            }
        }
        // nothing left to answer, or nothing here that could answer it: reading a package's
        // table means walking its whole class graph, which is not worth doing for a package that
        // draws none of the clips in question
        if want.is_empty() || seen.is_empty() { return }
        // WHAT A FIRED NAME DRAWS WITH, said by the package that owns the clip. A fighter's own
        // file is already read this way: its table gives every name it fires a `linkage_id`, and
        // a name that is not a clip is resolved through it. Borrowing from another package needs
        // the same reading of THAT package -- luigi fires `luigi_fireball` and ships
        // `luigi_fireball_wrapper`, and only luigi's table says so, so whoever borrows the
        // fireball without reading it finds a clip of the wrong name and moves on. That is
        // every copy ability a fighter has of another's move.
        let mut table: std::collections::BTreeMap<String, crate::abc_parser::ProjectileData> =
            Default::default();
        // read under every name the package answers to: the one it gave for itself, and the one
        // its file is called, which for a folder of source files are usually the same
        for abc in &abcs {
            for scope in &scopes {
                if let Ok(ex) = crate::abc_parser::extract_character_with_timeline(
                    abc, scope, &Default::default(), &Default::default(), &Default::default()) {
                    for (k, v) in ex.projectiles { table.entry(k).or_insert(v); }
                }
            }
        }
        if table.is_empty() { return }
        let folded = |s: &str| crate::projectile_gen::projectile_content_id(s);
        let mut through_table: Vec<DiscoveredProjectile> = Vec::new();
        for id in want.iter().cloned().collect::<Vec<_>>() {
            let Some(link) = table.iter().find(|(k, _)| folded(k) == id)
                .and_then(|(_, d)| d.names.get("linkage_id"))
            else { continue };
            let link_id = folded(link);
            let Some(src) = seen.iter().find(|p| folded(&p.name) == link_id).cloned() else { continue };
            // named as the borrower fires it, drawn with the clip the owner says it draws with
            let mut copy = src;
            copy.name = id.trim_end_matches("Projectile").to_string();
            log::info!("'{link}' is what {from} draws '{id}' with, its own table says so");
            want.remove(&id);
            through_table.push(copy);
        }
        if !through_table.is_empty() {
            log::info!("{} projectile(s) came across from {from} through its own table",
                through_table.len());
            got.extend(through_table);
        }
    }

    // THE FILES BESIDE THE ONE BEING CONVERTED, first because they are the ones the user chose.
    // The game keeps its fighters in numbered archives and splits a fighter's clips from the
    // code that names them, so a package there can hold a clip and say nothing about which name
    // fires it. A folder of source files is one file per fighter, clips and table together, and
    // that is what answers a copy ability. The two are not the same set either -- a fighter the
    // game has and the folder does not is still reachable below.
    if let Some(dir) = beside {
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir).into_iter().flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "ssf" || e == "swf"))
            .collect();
        files.sort();
        for f in files {
            if want.is_empty() { break }
            let Ok(raw) = std::fs::read(&f) else { continue };
            let Some(name) = f.file_name().and_then(|n| n.to_str()) else { continue };
            let swf = crate::ssf::decompress(&raw).unwrap_or(raw);
            take(&swf, &format!("the '{name}' package beside it"), name, want, &mut got);
        }
    }
    // a fighter converted from the installed game sits IN the data directory, so the walk
    // above has already asked every package below, each with all it could answer still wanted
    let walked_data = beside.is_some_and(crate::shared_packages::is_data_dir);
    if !walked_data {
        for (id, swf) in crate::shared_packages::every_package() {
            if want.is_empty() { break }
            take(&swf, &format!("the shared '{id}' package"), &id, want, &mut got);
        }
        if !want.is_empty() {
            for (id, swf) in crate::shared_packages::all_data_packages() {
                if want.is_empty() { break }
                take(&swf, &format!("the '{id}' package"), &id, want, &mut got);
            }
        }
    }
    if !want.is_empty() {
        if let Some(movie) = crate::shared_packages::game_movie() {
            take(movie, "the game's own movie", "", want, &mut got);
        }
    }
    got
}
