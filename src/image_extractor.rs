/// Extracts bitmap images from SWF tags and writes them as PNGs.
///
/// SWF structure for character sprites:
///   DefineSprite (animation) → PlaceObject (shape_id) → DefineShape (bitmap fill) → DefineBitsLossless
///
/// We extract the raw bitmaps and name the PNGs by their symbol name (e.g. mario_i0.png).
/// We also build a shape_id → bitmap_id mapping so callers can resolve
/// DefineSprite PlaceObject references to actual image files.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ExtractedImage {
    pub bitmap_id: u16,
    pub symbol_name: String,
    pub width: u32,
    pub height: u32,
    pub png_path: String, // relative path within character output dir
}

/// Maps shape/character IDs to bitmap IDs (DefineShape → bitmap fill ID)
pub type ShapeToBitmapMap = BTreeMap<u16, u16>;

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
    pub fn from_abcd(a: f64, b: f64, c: f64, d: f64, tx: f64, ty: f64) -> Self {
        Self {
            tx, ty,
            sx: (a*a + b*b).sqrt(),
            sy: (c*c + d*d).sqrt(),
            rotation: b.atan2(a).to_degrees(),
            a, b, c, d,
        }
    }

    /// Compose two matrices (self * other), producing a new affine matrix.
    pub fn compose(&self, other: &ImageLocalMatrix) -> Self {
        let a = self.a * other.a + self.b * other.c;
        let b = self.a * other.b + self.b * other.d;
        let c = self.c * other.a + self.d * other.c;
        let d = self.c * other.b + self.d * other.d;
        let tx = self.a * other.tx + self.b * other.ty + self.tx;
        let ty = self.c * other.tx + self.d * other.ty + self.ty;
        Self::from_abcd(a, b, c, d, tx, ty)
    }

    /// Returns true if this matrix contains a skew (not just scale + rotation).
    /// A pure rotation has atan2(b,a) == atan2(-c,d). Skew means they differ.
    pub fn has_skew(&self) -> bool {
        let rot1 = self.b.atan2(self.a);
        let rot2 = (-self.c).atan2(self.d);
        (rot1 - rot2).abs() > 0.02 // ~1 degree tolerance
    }
}

/// A single image layer placed at a specific depth in one frame.
#[derive(Debug, Clone)]
pub struct FrameImageEntry {
    pub depth: u16,
    pub shape_id: u16,
    pub symbol_name: String,
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

    // 1. Build shape_id → bitmap_id map from DefineShape tags
    let mut shape_to_bitmap: ShapeToBitmapMap = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::DefineShape(shape) = tag {

            // Look for bitmap fill in fill styles.
            // Skip id=65535 (SWF null/clipping bitmap) — take the first real bitmap.
            for fill in &shape.styles.fill_styles {
                if let swf::FillStyle::Bitmap { id, .. } = fill {
                    if *id != 65535 {
                        shape_to_bitmap.insert(shape.id, *id);
                        break;
                    }
                }
            }
        }
    }
    log::info!("Shape→bitmap mappings: {}", shape_to_bitmap.len());

    // 2. Extract all bitmaps to PNGs
    let sprites_dir = output_dir.join("library/sprites");
    fs::create_dir_all(&sprites_dir)?;

    let mut images: BTreeMap<u16, ExtractedImage> = BTreeMap::new();

    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineBitsLossless(bmp) => {
                let id = bmp.id;
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
                        write_png(&png_path, w, h, &rgba_data)?;

                        images.insert(id, ExtractedImage {
                            bitmap_id: id,
                            symbol_name: sym,
                            width: w,
                            height: h,
                            png_path: format!("library/sprites/{}", filename),
                        });
                    }
                    Err(e) => {
                        log::debug!("Failed to decode lossless bitmap {}: {}", id, e);
                    }
                }
            }
            swf::Tag::DefineBitsJpeg3(jpeg) => {
                let id = jpeg.id;
                let sym = find_symbol_for_bitmap(id, &symbols, &shape_to_bitmap)
                    .unwrap_or_else(|| symbols.get(&id).cloned()
                        .unwrap_or_else(|| format!("jpeg_{}", id)));

                match decode_jpeg3(jpeg) {
                    Ok((w, h, rgba_data)) => {
                        let filename = format!("{}.png", sanitize_name(&sym));
                        let png_path = sprites_dir.join(&filename);
                        write_png(&png_path, w, h, &rgba_data)?;

                        images.insert(id, ExtractedImage {
                            bitmap_id: id,
                            symbol_name: sym,
                            width: w,
                            height: h,
                            png_path: format!("library/sprites/{}", filename),
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

    log::info!("Extracted {} images to {}", images.len(), sprites_dir.display());

    // 3. Extract root MC transforms for applying to image positions
    let xform_map = crate::sprite_parser::extract_xframe_transforms(swf_data, char_name, ssf2_to_fm)
        .unwrap_or_default();

    // 4. Build per-animation per-frame image references from DefineSprite tags
    let mut anim_images = build_anim_frame_images(&swf, char_name, ssf2_to_fm, &symbols, &shape_to_bitmap, &xform_map, &images);
    // Apply same fallbacks as sprite_parser for animations with no image data
    apply_image_fallbacks(&mut anim_images);
    log::info!("Animation image mappings: {} animations (after fallbacks)", anim_images.len());

    // 5. Pre-render skewed frames as new bitmaps
    // Fraymakers doesn't support skew transforms, so we bake the full affine
    // into a new PNG and replace the frame entry with identity placement.
    let skew_count = prerender_skewed_frames(
        &mut anim_images, &mut images, &shape_to_bitmap, &sprites_dir, char_name,
    );
    if skew_count > 0 {
        log::info!("Pre-rendered {} skewed frame placements as new bitmaps", skew_count);
    }

    Ok(ImageExtractionResult {
        images,
        shape_to_bitmap,
        anim_images,
    })
}

/// Pre-render any frame image entries that have skew transforms.
/// Loads the source bitmap, applies the full affine matrix, writes a new PNG.
/// Replaces the entry's symbol/matrix with the pre-rendered version.
fn prerender_skewed_frames(
    anim_images: &mut BTreeMap<String, AnimFrameImages>,
    images: &mut BTreeMap<u16, ExtractedImage>,
    shape_to_bitmap: &ShapeToBitmapMap,
    sprites_dir: &std::path::Path,
    char_name: &str,
) -> usize {
    use image::{GenericImageView, RgbaImage, Rgba};
    let mut count = 0;
    let mut prerendered_cache: BTreeMap<String, (String, u32, u32)> = BTreeMap::new();

    // Collect all (anim, frame, entry_idx) that need pre-rendering
    let mut work: Vec<(String, u16, usize)> = Vec::new();
    for (anim_name, anim_data) in anim_images.iter() {
        for (frame, entries) in &anim_data.frames {
            for (idx, entry) in entries.iter().enumerate() {
                if entry.local_matrix.has_skew() {
                    log::debug!("prerender_skewed: queuing anim={} frame={} sym={} a={:.3} b={:.3} c={:.3} d={:.3}",
                        anim_name, frame, entry.symbol_name, entry.local_matrix.a, entry.local_matrix.b, entry.local_matrix.c, entry.local_matrix.d);
                    work.push((anim_name.clone(), *frame, idx));
                }
            }
        }
    }
    log::debug!("prerender_skewed: {} entries to process", work.len());

    for (anim_name, frame, entry_idx) in &work {
        let entry = &anim_images[anim_name].frames[frame][*entry_idx];
        let mat = entry.local_matrix;

        // Resolve source image: try shape_id → bitmap_id first,
        // then fall back to looking up by symbol_name (for named sub-sprites like sand_sprite2)
        let bitmap_id = shape_to_bitmap.get(&entry.shape_id).copied().unwrap_or(entry.shape_id);
        let src_img = match images.get(&bitmap_id)
            .or_else(|| images.values().find(|img| img.symbol_name == entry.symbol_name))
        {
            Some(img) => img,
            None => {
                log::debug!("prerender_skewed: no source image for sym='{}' shape={}", entry.symbol_name, entry.shape_id);
                continue;
            }
        };

        // Cache key: shape_id + quantized matrix (avoid redundant renders)
        let cache_key = format!("{}_{:.2}_{:.2}_{:.2}_{:.2}",
            entry.shape_id, mat.a, mat.b, mat.c, mat.d);

        let (new_sym, new_w, new_h) = if let Some(cached) = prerendered_cache.get(&cache_key) {
            cached.clone()
        } else {
            // Load source image
            let src_path = sprites_dir.parent()
                .unwrap_or(sprites_dir)
                .join(&src_img.png_path);
            let src = match image::open(&src_path) {
                Ok(img) => img.to_rgba8(),
                Err(_) => continue,
            };
            let (sw, sh) = (src.width() as f64, src.height() as f64);

            // Compute bounding box of transformed image
            let corners = [
                (0.0, 0.0),
                (sw, 0.0),
                (0.0, sh),
                (sw, sh),
            ];
            let transformed: Vec<(f64, f64)> = corners.iter()
                .map(|(x, y)| (mat.a * x + mat.b * y, mat.c * x + mat.d * y))
                .collect();
            let min_x = transformed.iter().map(|p| p.0).fold(f64::MAX, f64::min);
            let min_y = transformed.iter().map(|p| p.1).fold(f64::MAX, f64::min);
            let max_x = transformed.iter().map(|p| p.0).fold(f64::MIN, f64::max);
            let max_y = transformed.iter().map(|p| p.1).fold(f64::MIN, f64::max);

            let dst_w = (max_x - min_x).ceil() as u32;
            let dst_h = (max_y - min_y).ceil() as u32;
            if dst_w == 0 || dst_h == 0 || dst_w > 4096 || dst_h > 4096 { continue; }

            // Inverse affine for backward mapping
            let det = mat.a * mat.d - mat.b * mat.c;
            if det.abs() < 1e-10 { continue; }
            let inv_a =  mat.d / det;
            let inv_b = -mat.b / det;
            let inv_c = -mat.c / det;
            let inv_d =  mat.a / det;

            let mut dst = RgbaImage::new(dst_w, dst_h);
            for dy in 0..dst_h {
                for dx in 0..dst_w {
                    // Map destination pixel back to source coordinates
                    let fx = dx as f64 + min_x;
                    let fy = dy as f64 + min_y;
                    let sx = inv_a * fx + inv_b * fy;
                    let sy = inv_c * fx + inv_d * fy;
                    let sxi = sx.round() as i64;
                    let syi = sy.round() as i64;
                    if sxi >= 0 && sxi < sw as i64 && syi >= 0 && syi < sh as i64 {
                        dst.put_pixel(dx, dy, *src.get_pixel(sxi as u32, syi as u32));
                    }
                }
            }

            // Write new PNG
            let new_sym_name = format!("skew_{}_{}_f{}", char_name, anim_name.replace('/', "_"), frame);
            let filename = format!("{}.png", sanitize_name(&new_sym_name));
            let png_path = sprites_dir.join(&filename);
            if let Err(e) = dst.save(&png_path) {
                log::warn!("Failed to save pre-rendered skew bitmap: {}", e);
                continue;
            }

            // Register as a new extracted image with a synthetic bitmap ID
            let new_id = 60000 + count as u16;
            images.insert(new_id, ExtractedImage {
                bitmap_id: new_id,
                symbol_name: new_sym_name.clone(),
                width: dst_w,
                height: dst_h,
                png_path: format!("library/sprites/{}", filename),
            });

            prerendered_cache.insert(cache_key.clone(), (new_sym_name.clone(), dst_w, dst_h));
            (new_sym_name, dst_w, dst_h)
        };

        // Replace the frame entry with identity placement at adjusted position
        let entry = &mut anim_images.get_mut(anim_name).unwrap().frames.get_mut(frame).unwrap()[*entry_idx];
        // The pre-rendered bitmap's origin is offset by (min_x, min_y) from the original
        // transform, so adjust tx/ty to account for this
        entry.symbol_name = new_sym.clone();
        entry.local_matrix = ImageLocalMatrix::from_abcd(1.0, 0.0, 0.0, 1.0,
            entry.local_matrix.tx, entry.local_matrix.ty);
        entry.world_sx = new_w as f64;
        entry.world_sy = new_h as f64;

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
) -> BTreeMap<String, AnimFrameImages> {
    use crate::sprite_parser::extract_ssf2_anim_name;

    let char_lower = char_name.to_lowercase();
    let mut result = BTreeMap::new();

    // Pre-build a lookup of all DefineSprite tags by id (including unnamed ones)
    let all_sprites: BTreeMap<u16, &swf::Sprite> = swf.tags.iter().filter_map(|t| {
        if let swf::Tag::DefineSprite(s) = t { Some((s.id, s)) } else { None }
    }).collect();

    // Pre-build effect sprite frame tables: sprite_id → Vec<frame> → Vec<(shape_id, sym, mat)>
    // Effect sprites are _fla. named sprites that are NOT top-level animation containers.
    // Recursively expands unnamed inner sprites.
    let effect_sprites: BTreeMap<u16, Vec<Vec<(u16, String, ImageLocalMatrix)>>> = {
        // Two-pass approach:
        // Pass 1: build inner unnamed sprite frame tables
        // Pass 2: for named effect sprites, inline the unnamed sub-sprites

        // First build all unnamed sprite frame tables
        let mut unnamed_frames: BTreeMap<u16, Vec<Vec<(u16, String, ImageLocalMatrix)>>> = BTreeMap::new();
        for (&sid, sprite) in &all_sprites {
            if symbols.contains_key(&sid) { continue; } // skip named ones
            let mut disp: BTreeMap<u16, (u16, String, ImageLocalMatrix)> = BTreeMap::new();
            let mut frames: Vec<Vec<(u16, String, ImageLocalMatrix)>> = Vec::new();
            for stag in &sprite.tags {
                match stag {
                    swf::Tag::ShowFrame => {
                        frames.push(disp.values().map(|(id, sym, mat)| (*id, sym.clone(), *mat)).collect());
                    }
                    swf::Tag::PlaceObject(po) => {
                        let local_mat = po.matrix.map(|m| {
                            let a = m.a.to_f64(); let b = m.b.to_f64();
                            let c = m.c.to_f64(); let d = m.d.to_f64();
                            ImageLocalMatrix::from_abcd(a, b, c, d,
                                m.tx.get() as f64 / 20.0, m.ty.get() as f64 / 20.0)
                        }).unwrap_or_default();
                        match &po.action {
                            swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) => {
                                if let Some(sname) = symbols.get(cid) {
                                    let lower = sname.to_lowercase();
                                    if !lower.contains("collisonbox") && !lower.contains("collisionbox") {
                                        disp.insert(po.depth, (*cid, sname.clone(), local_mat));
                                    }
                                } else if let Some(&bitmap_id) = shape_to_bitmap.get(cid) {
                                    if let Some(img) = images.get(&bitmap_id) {
                                        disp.insert(po.depth, (*cid, img.symbol_name.clone(), local_mat));
                                    }
                                }
                            }
                            swf::PlaceObjectAction::Modify => {
                                if let Some(e) = disp.get_mut(&po.depth) { e.2 = local_mat; }
                            }
                        }
                    }
                    swf::Tag::RemoveObject(ro) => { disp.remove(&ro.depth); }
                    _ => {}
                }
            }
            if !frames.is_empty() { unnamed_frames.insert(sid, frames); }
        }

        // Now build named effect sprite tables, inlining unnamed sub-sprites
        let mut map = BTreeMap::new();
        for tag in &swf.tags {
            if let swf::Tag::DefineSprite(sprite) = tag {
                let sym = match symbols.get(&sprite.id) { Some(s) => s.as_str(), None => continue };
                if !sym.contains("_fla.") { continue; }
                if extract_ssf2_anim_name(sym, &char_lower, ssf2_to_fm).is_some() { continue; }

                let mut disp: BTreeMap<u16, (u16, String, ImageLocalMatrix)> = BTreeMap::new();
                // depth → (unnamed_sprite_id, place_parent_mat, frame_started)
                let mut unnamed_placements: BTreeMap<u16, (u16, ImageLocalMatrix, u16)> = BTreeMap::new();
                let mut cur_frame: u16 = 0;
                let mut effect_frames: Vec<Vec<(u16, String, ImageLocalMatrix)>> = Vec::new();

                for stag in &sprite.tags {
                    match stag {
                        swf::Tag::ShowFrame => {
                            let mut snap: Vec<(u16, String, ImageLocalMatrix)> = disp.values()
                                .map(|(id, sym, mat)| (*id, sym.clone(), *mat)).collect();
                            // Expand unnamed sub-sprites: pick their frame at (cur_frame - place_frame)
                            for (&_depth, (unnamed_id, parent_mat, place_frame)) in &unnamed_placements {
                                if let Some(uframes) = unnamed_frames.get(unnamed_id) {
                                    let uf_idx = (cur_frame.saturating_sub(*place_frame) as usize)
                                        .min(uframes.len().saturating_sub(1));
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
                            let local_mat = po.matrix.map(|m| {
                                let a = m.a.to_f64(); let b = m.b.to_f64();
                                let c = m.c.to_f64(); let d = m.d.to_f64();
                                ImageLocalMatrix::from_abcd(a, b, c, d,
                                    m.tx.get() as f64 / 20.0, m.ty.get() as f64 / 20.0)
                            }).unwrap_or_default();
                            match &po.action {
                                swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) => {
                                    if let Some(sname) = symbols.get(cid) {
                                        let lower = sname.to_lowercase();
                                        if !lower.contains("collisonbox") && !lower.contains("collisionbox") {
                                            disp.insert(po.depth, (*cid, sname.clone(), local_mat));
                                        }
                                    } else if let Some(&bitmap_id) = shape_to_bitmap.get(cid) {
                                        if let Some(img) = images.get(&bitmap_id) {
                                            disp.insert(po.depth, (*cid, img.symbol_name.clone(), local_mat));
                                        }
                                    } else if unnamed_frames.contains_key(cid) {
                                        unnamed_placements.insert(po.depth, (*cid, local_mat, cur_frame));
                                    }
                                }
                                swf::PlaceObjectAction::Modify => {
                                    if let Some(e) = disp.get_mut(&po.depth) { e.2 = local_mat; }
                                    if let Some(e) = unnamed_placements.get_mut(&po.depth) { e.1 = local_mat; }
                                }
                            }
                        }
                        swf::Tag::RemoveObject(ro) => {
                            disp.remove(&ro.depth);
                            unnamed_placements.remove(&ro.depth);
                        }
                        _ => {}
                    }
                }
                if !effect_frames.is_empty() {
                    log::debug!("image_extractor: effect sprite id={} '{}' has {} frames",
                        sprite.id, sym, effect_frames.len());
                    map.insert(sprite.id, effect_frames);
                }
            }
        }
        map
    };

    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sym = symbols.get(&sprite.id)
                .map(|s| s.as_str())
                .unwrap_or("");

            // Only process character animation sprites
            if !sym.contains("_fla.") { continue; }

            let fm_name = match extract_ssf2_anim_name(sym, &char_lower, ssf2_to_fm) {
                Some(ssf2_name) => ssf2_to_fm.get(&ssf2_name).cloned().unwrap_or(ssf2_name),
                None => {
                    // No mapping but still process — use raw symbol name as key
                    // e.g. "sandbag_fla.UpThrow_69" contains trail/effect images
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
            // depth → (effect_sprite_id, place_frame, parent_matrix) — sub-sprite placements
            let mut sub_sprite_placements: BTreeMap<u16, (u16, u16, ImageLocalMatrix)> = BTreeMap::new();
            let mut frames: BTreeMap<u16, Vec<FrameImageEntry>> = BTreeMap::new();

            for stag in &sprite.tags {
                match stag {
                    swf::Tag::ShowFrame => {
                        let mut entries: Vec<FrameImageEntry> = Vec::new();

                        // Regular display list entries
                        for (&depth, (id, sym, mat)) in &display_list {
                            let (world_tx, world_ty) = root_xf.apply(mat.tx, mat.ty);
                            let world_sx = mat.sx * root_xf.scale_x();
                            let world_sy = mat.sy * root_xf.scale_y();
                            entries.push(FrameImageEntry {
                                depth,
                                shape_id: *id,
                                symbol_name: sym.clone(),
                                local_matrix: *mat,
                                world_tx,
                                world_ty,
                                world_sx,
                                world_sy,
                                world_rotation: mat.rotation,
                            });
                        }

                        // Expand effect sub-sprite placements into their current inner frame
                        for (&depth, (effect_id, place_frame, parent_mat)) in &sub_sprite_placements {
                            if let Some(effect_frames) = effect_sprites.get(effect_id) {
                                let eff_frame = (current_frame.saturating_sub(*place_frame)) as usize;
                                // Clamp to last frame (effect may loop or hold)
                                let eff_frame = eff_frame.min(effect_frames.len().saturating_sub(1));
                                for (inner_id, inner_sym, inner_mat) in &effect_frames[eff_frame] {
                                    let composed = parent_mat.compose(inner_mat);
                                    let (world_tx, world_ty) = root_xf.apply(composed.tx, composed.ty);
                                    let world_sx = composed.sx * root_xf.scale_x();
                                    let world_sy = composed.sy * root_xf.scale_y();
                                    let effect_depth = depth + 1000;
                                    entries.push(FrameImageEntry {
                                        depth: effect_depth,
                                        shape_id: *inner_id,
                                        symbol_name: inner_sym.clone(),
                                        local_matrix: composed,
                                        world_tx,
                                        world_ty,
                                        world_sx,
                                        world_sy,
                                        world_rotation: composed.rotation,
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
                    }
                    swf::Tag::PlaceObject(po) => {
                        let inst_name = po.name.as_ref()
                            .map(|n| String::from_utf8_lossy(n.as_bytes()).to_string())
                            .unwrap_or_default();

                        // Skip collision box instances
                        if crate::sprite_parser::BoxType::from_instance_name(&inst_name).is_some() {
                            continue;
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
                                            sub_sprite_placements.insert(depth, (*char_id, current_frame, local_mat));
                                        }
                                        continue;
                                    }
                                    display_list.insert(depth, (*char_id, sym_name.clone(), local_mat));
                                } else if effect_sprites.contains_key(char_id) {
                                    // Unnamed sub-sprite that was pre-built as an effect sprite
                                    sub_sprite_placements.insert(depth, (*char_id, current_frame, local_mat));
                                } else if shape_to_bitmap.contains_key(char_id) {
                                    // Unnamed shape with a bitmap fill — resolve to bitmap symbol
                                    let bitmap_id = shape_to_bitmap[char_id];
                                    if let Some(img) = images.get(&bitmap_id) {
                                        display_list.insert(depth, (*char_id, img.symbol_name.clone(), local_mat));
                                    }
                                } else if all_sprites.contains_key(char_id) {
                                    // Unnamed sub-sprite not pre-built — track as effect
                                    sub_sprite_placements.insert(depth, (*char_id, current_frame, local_mat));
                                }
                            }
                            swf::PlaceObjectAction::Modify => {
                                if let Some(entry) = display_list.get_mut(&depth) {
                                    entry.2 = local_mat;
                                } else if let Some(entry) = sub_sprite_placements.get_mut(&depth) {
                                    // Update the parent matrix of a sub-sprite placement
                                    entry.2 = local_mat;
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
                let frame_labels = extract_frame_labels_from_sprite(&sprite.tags);
                let sub_splits = crate::sprite_parser::sub_anim_image_splits(&fm_name, &frame_labels, total);

                if sub_splits.is_empty() {
                    result.insert(fm_name, AnimFrameImages {
                        frames,
                        total_frames: total,
                        max_depth_slots,
                    });
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

/// Apply image fallbacks for procedural/synthetic animations (same table as sprite_parser)
/// Extract frame labels from a sprite tag list (same logic as sprite_parser)
fn extract_frame_labels_from_sprite(tags: &[swf::Tag]) -> Vec<(String, u16)> {
    let mut frame_num: u16 = 0;
    let mut labels: Vec<(String, u16)> = Vec::new();
    for tag in tags {
        match tag {
            swf::Tag::ShowFrame => { frame_num += 1; }
            swf::Tag::FrameLabel(fl) => {
                let label = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                labels.push((label, frame_num));
            }
            _ => {}
        }
    }
    labels.sort_by_key(|(_, f)| *f);
    labels
}

fn apply_image_fallbacks(result: &mut BTreeMap<String, AnimFrameImages>) {
    let fallbacks: &[(&str, &str)] = &[
        ("stunned", "hurt"), ("star_ko", "hurt"), ("starko", "hurt"),
        ("screenko", "hurt"), ("buried", "crouch"), ("fly", "jump_aerial"),
        ("swim", "fall"), ("ladder", "idle"), ("wall_stick", "fall"),
        ("special", "idle"), ("carry", "grab"), ("land_heavy", "land"),
        ("ledge_lean", "ledge_hang"), ("victory", "taunt"), ("defeat", "hurt"),
        ("respawn", "idle"), ("special_down_air", "special_down"),
        ("item_float", "idle"), ("item_screw", "special_up"),
        ("tumble", "fall"), ("frozen", "idle"),
    ];

    let mut to_insert: Vec<(String, AnimFrameImages)> = Vec::new();
    for (missing, donor) in fallbacks {
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
fn decode_lossless(bmp: &swf::DefineBitsLossless) -> Result<Vec<u8>> {
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
fn decode_jpeg3(jpeg: &swf::DefineBitsJpeg3) -> Result<(u32, u32, Vec<u8>)> {
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
        let mut decoder = ZlibDecoder::new(&jpeg.alpha_data[..]);
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

fn sanitize_name(name: &str) -> String {
    name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "_")
}

// ─── Projectile image frame extraction ────────────────────────────────────────────────────────

/// Per-frame image data for a single projectile animation.
#[derive(Debug, Clone)]
pub struct ProjectileFrameImages {
    /// frame index → symbol name of the image placed on that frame (if any)
    pub frames: Vec<Option<String>>,
    /// symbol_name → meta GUID
    pub image_guids: BTreeMap<String, String>,
}

/// Extract per-frame image data for a projectile's inner sprite.
///
/// Uses the same two-pass effect-sprite flattening as `extract_images`.
/// Given the inner sprite ID (e.g. 194 for mario_fireball_mc), returns
/// the image symbol name placed on each frame.
pub fn extract_projectile_frame_images(
    swf_data: &[u8],
    char_id: &str,
    inner_sprite_id: u16,
    img_result: &ImageExtractionResult,
) -> Result<ProjectileFrameImages> {
    use std::io::Cursor;
    let swf_buf = swf::decompress_swf(Cursor::new(swf_data)).context("decompress")?;
    let swf = swf::parse_swf(&swf_buf).context("parse")?;

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

    // Pass 1: unnamed sprite frame tables
    // These sprites have no SymbolClass entry; they contain raw shape placements.
    // We resolve shape_id → bitmap_id → symbol_name via img_result.shape_to_bitmap.
    let mut unnamed_frames: BTreeMap<u16, Vec<Vec<(u16, String, ImageLocalMatrix)>>> = BTreeMap::new();
    for (&sid, sprite) in &all_sprites {
        if symbols.contains_key(&sid) { continue; }
        let mut disp: BTreeMap<u16, (u16, String, ImageLocalMatrix)> = BTreeMap::new();
        let mut frames: Vec<Vec<(u16, String, ImageLocalMatrix)>> = Vec::new();
        for stag in &sprite.tags {
            match stag {
                swf::Tag::ShowFrame => {
                    frames.push(disp.values().map(|(id, sym, mat)| (*id, sym.clone(), *mat)).collect());
                }
                swf::Tag::PlaceObject(po) => {
                    let mat = po_to_mat(po);
                    match &po.action {
                        swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) => {
                            if let Some(sname) = symbols.get(cid) {
                                // Named symbol
                                disp.insert(po.depth, (*cid, sname.clone(), mat));
                            } else if let Some(&bitmap_id) = img_result.shape_to_bitmap.get(cid) {
                                // Unnamed shape with a bitmap fill — resolve to symbol name
                                if let Some(img) = img_result.images.get(&bitmap_id) {
                                    disp.insert(po.depth, (*cid, img.symbol_name.clone(), mat));
                                }
                            }
                        }
                        swf::PlaceObjectAction::Modify => {
                            if let Some(e) = disp.get_mut(&po.depth) { e.2 = mat; }
                        }
                    }
                }
                swf::Tag::RemoveObject(ro) => { disp.remove(&ro.depth); }
                _ => {}
            }
        }
        if !frames.is_empty() { unnamed_frames.insert(sid, frames); }
    }

    // Pass 2: flatten target sprite
    let sprite = match all_sprites.get(&inner_sprite_id) {
        Some(s) => s,
        None => return Ok(ProjectileFrameImages { frames: vec![], image_guids: BTreeMap::new() }),
    };

    let mut disp: BTreeMap<u16, (u16, String, ImageLocalMatrix)> = BTreeMap::new();
    let mut unnamed_placements: BTreeMap<u16, (u16, ImageLocalMatrix, u16)> = BTreeMap::new();
    let mut cur_frame: u16 = 0;
    let mut effect_frames: Vec<Vec<(u16, String, ImageLocalMatrix)>> = Vec::new();

    for stag in &sprite.tags {
        match stag {
            swf::Tag::ShowFrame => {
                let mut snap: Vec<(u16, String, ImageLocalMatrix)> =
                    disp.values().map(|(id, sym, mat)| (*id, sym.clone(), *mat)).collect();
                for (&_depth, (unnamed_id, parent_mat, place_frame)) in &unnamed_placements {
                    if let Some(uframes) = unnamed_frames.get(unnamed_id) {
                        let uf_idx = (cur_frame.saturating_sub(*place_frame) as usize)
                            .min(uframes.len().saturating_sub(1));
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
                let mat = po_to_mat(po);
                match &po.action {
                    swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) => {
                        if let Some(sname) = symbols.get(cid) {
                            let lower = sname.to_lowercase();
                            if !lower.contains("collisonbox") && !lower.contains("collisionbox") {
                                disp.insert(po.depth, (*cid, sname.clone(), mat));
                            }
                        } else if unnamed_frames.contains_key(cid) {
                            // Unnamed sub-sprite
                            unnamed_placements.insert(po.depth, (*cid, mat, cur_frame));
                        } else if let Some(&bitmap_id) = img_result.shape_to_bitmap.get(cid) {
                            // Direct shape placement with bitmap fill
                            if let Some(img) = img_result.images.get(&bitmap_id) {
                                disp.insert(po.depth, (*cid, img.symbol_name.clone(), mat));
                            }
                        }
                    }
                    swf::PlaceObjectAction::Modify => {
                        if let Some(e) = disp.get_mut(&po.depth) { e.2 = mat; }
                        if let Some(e) = unnamed_placements.get_mut(&po.depth) { e.1 = mat; }
                    }
                }
            }
            swf::Tag::RemoveObject(ro) => {
                disp.remove(&ro.depth);
                unnamed_placements.remove(&ro.depth);
            }
            _ => {}
        }
    }

    // Convert effect_frames → per-frame symbol names
    let mut frames: Vec<Option<String>> = Vec::new();
    let mut image_guids: BTreeMap<String, String> = BTreeMap::new();

    for frame_entries in &effect_frames {
        // Each entry now has the resolved symbol_name already (from Pass 1 & 2 fixes).
        // Take the first non-empty symbol name in the frame.
        let sym_name = frame_entries.iter().find_map(|(_shape_id, sym_name, _mat)| {
            if sym_name.is_empty() { None } else { Some(sym_name.clone()) }
        });
        if let Some(ref sym) = sym_name {
            let meta_guid = crate::uuid_gen::det_uuid(&format!("{}::meta_{}", char_id, sym));
            image_guids.insert(sym.clone(), meta_guid);
        }
        frames.push(sym_name);
    }

    Ok(ProjectileFrameImages { frames, image_guids })
}

fn po_to_mat(po: &swf::PlaceObject) -> ImageLocalMatrix {
    po.matrix.map(|m| {
        let a = m.a.to_f64(); let b = m.b.to_f64();
        let c = m.c.to_f64(); let d = m.d.to_f64();
        ImageLocalMatrix {
            tx: m.tx.get() as f64 / 20.0,
            ty: m.ty.get() as f64 / 20.0,
            sx: (a*a + b*b).sqrt(),
            sy: (c*c + d*d).sqrt(),
            rotation: b.atan2(a).to_degrees(),
            a, b, c, d,
        }
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
}

/// A projectile sprite discovered in the SWF.
#[derive(Debug, Clone)]
pub struct DiscoveredProjectile {
    /// Root projectile sprite ID
    pub sprite_id: u16,
    /// Root projectile name (e.g. "mario_fireball")
    pub name: String,
    /// Inner animation sprite ID (the 'stance' child), if any (single-state projectiles)
    pub inner_sprite_id: Option<u16>,
    /// Inner animation sprite name (e.g. "mario_fla.mario_fireball_mc_210")
    pub inner_sprite_name: Option<String>,
    /// Frame count of the inner animation sprite
    pub inner_frame_count: u16,
    /// For multi-state projectiles (outer wrapper has multiple frame labels),
    /// each entry is one SSF2 state with its own inner sprite.
    /// Empty for single-state projectiles.
    pub states: Vec<ProjectileState>,
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

/// Scan the SWF for projectile sprites and the head/portrait sprite.
///
/// Projectiles are identified by having an `attack_idle` frame label.
/// Head sprites are identified by the naming pattern `{char}_head`.
///
/// Returns (projectiles, head_sprite)
pub fn discover_projectiles_and_head(
    swf_data: &[u8],
    char_name: &str,
) -> Result<(Vec<DiscoveredProjectile>, Option<DiscoveredHead>)> {
    use std::io::Cursor;
    let swf_buf = swf::decompress_swf(Cursor::new(swf_data)).context("decompress SWF")?;
    let swf = swf::parse_swf(&swf_buf).context("parse SWF")?;

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

    // Scan DefineSprite tags
    let mut projectiles: Vec<DiscoveredProjectile> = Vec::new();
    let mut head: Option<DiscoveredHead> = None;
    let char_lower = char_name.to_lowercase();

    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sprite_name = symbols.get(&sprite.id).cloned().unwrap_or_default();
            if sprite_name.is_empty() { continue; }

            // Check for head sprite: "{char}_head" or "{char}head" or "jiggly_head" etc.
            let name_lower = sprite_name.to_lowercase();
            if name_lower.ends_with("_head") || name_lower.ends_with("head") {
                // Verify it belongs to this character
                // Some chars abbreviate: jiggly_head for jigglypuff, etc.
                let is_char_head = name_lower.starts_with(&char_lower)
                    || name_lower == format!("{}_head", char_lower)
                    || name_lower.ends_with("_head")  // Any _head sprite in a char SSF is the char's head
                    ;
                if is_char_head {
                    // Find what image it places.
                    // First try PlaceObject inside the sprite tags.
                    let mut image_symbol = None;
                    let mut image_shape_id = None;
                    for stag in &sprite.tags {
                        if let swf::Tag::PlaceObject(po) = stag {
                            match &po.action {
                                swf::PlaceObjectAction::Place(id) => {
                                    image_shape_id = Some(*id);
                                    image_symbol = symbols.get(id).cloned();
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    // Fallback: try known naming patterns for head portrait images
                    if image_symbol.is_none() {
                        let patterns: Vec<String> = vec![
                            format!("{}_dm0", char_lower),
                            format!("{}_pa0", char_lower),
                            format!("{}_dmpa", char_lower),
                        ];
                        for pat in &patterns {
                            if let Some((id, name)) = symbols.iter().find(|(_, n)| n.to_lowercase() == *pat) {
                                image_symbol = Some(name.clone());
                                image_shape_id = Some(*id);
                                log::debug!("Head sprite '{}': used pattern fallback '{}' → id={}", sprite_name, pat, id);
                                break;
                            }
                        }
                    }
                    // Last resort: search all symbols for portrait-like names
                    if image_symbol.is_none() {
                        for (id, name) in &symbols {
                            let nl = name.to_lowercase();
                            // Skip internal sprites and animation frames
                            if nl.contains("_fla.") || nl.contains("_i") && nl.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                                continue;
                            }
                            // Match common portrait patterns:
                            // {char}_pa0, {char}_dmpa, {char}_dm0, {abbr}_PA, {abbr}_cssp2
                            let is_portrait = nl.ends_with("_pa0") || nl.ends_with("_dmpa")
                                || nl.ends_with("_dm0") || nl.ends_with("_pa") || nl.ends_with("_pa_nhb")
                                || nl.contains("_cssp");
                            if is_portrait {
                                image_symbol = Some(name.clone());
                                image_shape_id = Some(*id);
                                log::debug!("Head sprite '{}': used heuristic fallback → id={} sym={}", sprite_name, id, name);
                                break;
                            }
                        }
                    }
                    head = Some(DiscoveredHead {
                        sprite_id: sprite.id,
                        name: sprite_name.clone(),
                        image_symbol,
                        image_shape_id,
                    });
                }
            }

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
            if name_lower == char_lower { continue; }

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

            // Only keep states vec if it has more than just attack_idle
            let states = if states.len() > 1 { states } else { Vec::new() };

            projectiles.push(DiscoveredProjectile {
                sprite_id: sprite.id,
                name: sprite_name,
                inner_sprite_id,
                inner_sprite_name,
                inner_frame_count,
                states,
            });
        }
    }

    Ok((projectiles, head))
}
