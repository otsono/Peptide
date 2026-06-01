/// Palette generation for Fraymakers costumes.
///
/// Two modes:
/// 1. **Real SSF2 data** (preferred): loads costume data extracted from misc.ssf via
///    the `extract_costumes` binary. Each SSF2 costume has `colors` (source palette, same for all
///    costumes) and `replacements` (what those colors become). These map directly to
///    Fraymakers palette color slots + maps.
///
/// 2. **Fallback k-means**: when no misc.ssf data is available, samples idle sprites,
///    quantizes to PALETTE_SIZE colors, and generates hue-shifted alt costumes.

use image::{GenericImageView, ImageBuffer, Rgba};
use serde_json::{json, Value};
use std::path::Path;
use std::fs;

const PALETTE_SIZE: usize = 32;
const KMEANS_ITER:  usize = 30;

// ─── UUID helper ──────────────────────────────────────────────────────────────

fn det_uuid(seed: &str) -> String {
    crate::uuid_gen::det_uuid(seed)
}

// ─── Result ───────────────────────────────────────────────────────────────────

pub struct PaletteResult {
    pub palettes_json:      String,
    pub palettes_meta_json: String,
    pub preview_png:        Vec<u8>,
    pub preview_meta_json:  String,
    pub collection_guid:    String,
    pub base_map_id:        String,
}

// ─── SSF2 costume data (from extract_costumes binary output) ────────────────
//
// Same shape and meaning as the costume payload `abc_parser` decodes straight
// from the misc.ssf bytecode, so we reuse that one canonical type instead of
// defining a parallel struct here.
pub use crate::abc_parser::CostumeData as Ssf2Costume;

/// Load costume data for a character from ssf2_costumes.json.
/// Returns None if the file doesn't exist or the character isn't found.
pub fn load_ssf2_costumes(json_path: &Path, char_name: &str) -> Option<Vec<Ssf2Costume>> {
    let raw = fs::read_to_string(json_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;

    // Try exact name and common variations (mario, Mario, mario_bro, etc.)
    let lower = char_name.to_lowercase();
    let candidates = [
        lower.clone(),
        char_name.to_string(),
        char_name.replace(' ', "").to_lowercase(),
    ];

    let arr = candidates.iter().find_map(|k| v.get(k).and_then(|x| x.as_array()))
        // Fuzzy fallback: find any key that starts with or contains char_name
        .or_else(|| {
            v.as_object()?.iter().find_map(|(k, val)| {
                let kl = k.to_lowercase();
                if kl.starts_with(&lower) || lower.starts_with(&kl) {
                    val.as_array()
                } else {
                    None
                }
            })
        })?;

    let costumes = arr.iter().filter_map(|entry| {
        let name = entry.get("name").and_then(|n| n.as_str())?.to_string();
        let colors: Vec<u32> = entry.get("colors")?.as_array()?
            .iter().filter_map(|c| c.as_u64().map(|n| n as u32)).collect();
        let replacements: Vec<u32> = entry.get("replacements")?.as_array()?
            .iter().filter_map(|c| c.as_u64().map(|n| n as u32)).collect();
        if colors.len() != replacements.len() || colors.is_empty() { return None; }
        Some(Ssf2Costume { name, colors, replacements })
    }).collect::<Vec<_>>();

    if costumes.is_empty() { None } else { Some(costumes) }
}

fn argb_to_fm_hex(argb: u32) -> String {
    // SSF2 stores ARGB as 0xAARRGGBB
    // Fraymakers uses "0xFFRRGGBB" format
    let r = (argb >> 16) & 0xFF;
    let g = (argb >>  8) & 0xFF;
    let b =  argb        & 0xFF;
    format!("0xFF{:02X}{:02X}{:02X}", r, g, b)
}

// ─── Real SSF2 data path ──────────────────────────────────────────────────────

fn build_from_ssf2(
    char_id:   &str,
    char_name: &str,
    costumes:  &[Ssf2Costume],
) -> anyhow::Result<PaletteResult> {

    // Use the base costume's source colors as palette slots.
    // SSF2's `colors` array is identical across all costumes — it's the base sprite palette.
    let base = costumes.iter().find(|c| c.name == "Default")
        .or_else(|| costumes.first())
        .ok_or_else(|| anyhow::anyhow!("no costumes"))?;

    let slot_count = base.colors.len();
    log::info!("palette_gen (ssf2): {} color slots, {} costumes", slot_count, costumes.len());

    // Color slot declarations
    let color_slots: Vec<Value> = base.colors.iter().enumerate().map(|(i, &argb)| {
        let slot_id = det_uuid(&format!("{}::palette_color_{}", char_id, i));
        json!({
            "$id": slot_id,
            "color": argb_to_fm_hex(argb),
            "name": format!("color_{:02}", i),
            "pluginMetadata": {}
        })
    }).collect();

    // Build a map for each costume. The "Default" costume is forced to the front of
    // the exported list (Fraymakers shows the first map as the character's default);
    // a stable sort keeps every other costume in its original relative order.
    let mut ordered: Vec<&Ssf2Costume> = costumes.iter().collect();
    ordered.sort_by_key(|c| c.name != "Default");

    let mut maps: Vec<Value> = Vec::new();
    let base_map_id = det_uuid(&format!("{}::palette_map_base", char_id));

    for (ci, costume) in ordered.iter().enumerate() {
        let is_base = costume.name == "Default" || (ci == 0 && ordered.iter().all(|c| c.name != "Default"));
        let map_id = if is_base {
            base_map_id.clone()
        } else {
            det_uuid(&format!("{}::palette_map_{}", char_id, ci))
        };

        // Map slot[i] → replacement[i] for this costume
        let map_colors: Vec<Value> = base.colors.iter().enumerate().map(|(i, _)| {
            let slot_id = det_uuid(&format!("{}::palette_color_{}", char_id, i));
            // Use this costume's replacement; fall back to base color if out of range
            let target = costume.replacements.get(i)
                .copied()
                .unwrap_or(base.colors[i]);
            json!({ "paletteColorId": slot_id, "targetColor": argb_to_fm_hex(target) })
        }).collect();

        maps.push(json!({
            "$id": map_id,
            "colors": map_colors,
            "name": costume.name,
            "pluginMetadata": {
                "com.fraymakers.FraymakersMetadata": { "isBase": is_base }
            }
        }));
    }

    // Build preview PNG: slot_count × 1, one pixel per color slot
    let preview_img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_fn(slot_count as u32, 1, |x, _| {
        let argb = base.colors[x as usize];
        let r = ((argb >> 16) & 0xFF) as u8;
        let g = ((argb >>  8) & 0xFF) as u8;
        let b =  (argb        & 0xFF) as u8;
        Rgba([r, g, b, 255])
    });
    let mut preview_png = Vec::new();
    preview_img.write_to(&mut std::io::Cursor::new(&mut preview_png), image::ImageFormat::Png)?;

    build_output(char_id, char_name, color_slots, maps, preview_png, &base_map_id)
}

// ─── K-means fallback path ────────────────────────────────────────────────────

fn dist_sq(a: &[u8; 3], b: &[u8; 3]) -> u32 {
    let dr = a[0] as i32 - b[0] as i32;
    let dg = a[1] as i32 - b[1] as i32;
    let db = a[2] as i32 - b[2] as i32;
    (dr*dr + dg*dg + db*db) as u32
}

fn nearest(rgb: &[u8; 3], palette: &[[u8; 3]]) -> usize {
    palette.iter().enumerate()
        .min_by_key(|(_, c)| dist_sq(rgb, c))
        .map(|(i, _)| i).unwrap_or(0)
}

fn kmeans(samples: &[[u8; 3]], k: usize) -> Vec<[u8; 3]> {
    if samples.is_empty() { return vec![[128u8; 3]; k]; }
    let step = (samples.len() / k).max(1);
    let mut centroids: Vec<[u8; 3]> = (0..k)
        .map(|i| samples[(i * step).min(samples.len()-1)]).collect();
    for _ in 0..KMEANS_ITER {
        let mut sums   = vec![[0u64; 3]; k];
        let mut counts = vec![0u64; k];
        for s in samples {
            let idx = nearest(s, &centroids);
            sums[idx][0] += s[0] as u64; sums[idx][1] += s[1] as u64; sums[idx][2] += s[2] as u64;
            counts[idx] += 1;
        }
        let mut changed = false;
        for i in 0..k {
            if counts[i] > 0 {
                let nc = [(sums[i][0]/counts[i]) as u8, (sums[i][1]/counts[i]) as u8, (sums[i][2]/counts[i]) as u8];
                if nc != centroids[i] { changed = true; }
                centroids[i] = nc;
            }
        }
        if !changed { break; }
    }
    centroids.sort_by_key(|c| (c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114) / 1000);
    centroids
}

fn rotate_hue(rgb: [u8; 3], deg: f32) -> [u8; 3] {
    let r = rgb[0] as f32 / 255.0;
    let g = rgb[1] as f32 / 255.0;
    let b = rgb[2] as f32 / 255.0;
    let max = r.max(g).max(b); let min = r.min(g).min(b); let delta = max - min;
    let h = if delta < 0.001 { 0.0 }
        else if (max - r).abs() < 0.001 { 60.0 * (((g - b) / delta) % 6.0) }
        else if (max - g).abs() < 0.001 { 60.0 * ((b - r) / delta + 2.0) }
        else { 60.0 * ((r - g) / delta + 4.0) };
    let h = ((h + deg) % 360.0 + 360.0) % 360.0;
    let s = if max < 0.001 { 0.0 } else { delta / max };
    let v = max; let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r2, g2, b2) = if h < 60.0 { (c,x,0.0) } else if h < 120.0 { (x,c,0.0) }
        else if h < 180.0 { (0.0,c,x) } else if h < 240.0 { (0.0,x,c) }
        else if h < 300.0 { (x,0.0,c) } else { (c,0.0,x) };
    [((r2+m)*255.0).round() as u8, ((g2+m)*255.0).round() as u8, ((b2+m)*255.0).round() as u8]
}

fn build_from_sprites(
    char_id:     &str,
    char_name:   &str,
    sprites_dir: &Path,
) -> anyhow::Result<PaletteResult> {
    let all_pngs: Vec<_> = fs::read_dir(sprites_dir)?
        .filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "png").unwrap_or(false)).collect();

    let idle_pngs: Vec<_> = all_pngs.iter().filter(|p| {
        let n = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        n.contains("_i") || n.contains("stand") || n.contains("idle")
    }).cloned().collect();

    let source_pngs = if idle_pngs.len() >= 3 { &idle_pngs } else { &all_pngs };
    log::info!("palette_gen (kmeans): sampling {} sprites", source_pngs.len());

    let per_sprite = (20_000 / source_pngs.len().max(1)).max(10);
    let mut pixels: Vec<[u8; 3]> = Vec::new();
    for path in source_pngs {
        let img = match image::open(path) { Ok(i) => i, Err(_) => continue };
        let total = (img.width() * img.height()) as usize;
        let step = (total / per_sprite).max(1);
        for (i, (_, _, px)) in img.pixels().enumerate() {
            if i % step == 0 && px[3] > 64 { pixels.push([px[0], px[1], px[2]]); }
        }
    }
    if pixels.is_empty() { pixels = vec![[220,50,50],[50,50,220],[50,180,50]]; }

    let palette = kmeans(&pixels, PALETTE_SIZE);

    let mut preview_img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(PALETTE_SIZE as u32, 1);
    for (i, rgb) in palette.iter().enumerate() {
        preview_img.put_pixel(i as u32, 0, Rgba([rgb[0], rgb[1], rgb[2], 255]));
    }
    let mut preview_png = Vec::new();
    preview_img.write_to(&mut std::io::Cursor::new(&mut preview_png), image::ImageFormat::Png)?;

    let color_slots: Vec<Value> = palette.iter().enumerate().map(|(i, rgb)| {
        let slot_id = det_uuid(&format!("{}::palette_color_{}", char_id, i));
        json!({ "$id": slot_id, "color": format!("0xFF{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]),
                "name": format!("color_{:02}", i), "pluginMetadata": {} })
    }).collect();

    let base_map_id = det_uuid(&format!("{}::palette_map_base", char_id));

    let build_map_colors = |hue_rot: f32| -> Vec<Value> {
        palette.iter().enumerate().map(|(i, rgb)| {
            let slot_id = det_uuid(&format!("{}::palette_color_{}", char_id, i));
            let c = if hue_rot.abs() < 0.1 { *rgb } else { rotate_hue(*rgb, hue_rot) };
            json!({ "paletteColorId": slot_id, "targetColor": format!("0xFF{:02X}{:02X}{:02X}", c[0], c[1], c[2]) })
        }).collect()
    };

    let mut maps: Vec<Value> = vec![json!({
        "$id": base_map_id,
        "colors": build_map_colors(0.0),
        "name": "Default",
        "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "isBase": true } }
    })];

    for (idx, (name, hue)) in [
        ("Alt 1 (Red)",    180.0f32),
        ("Alt 2 (Green)",  120.0),
        ("Alt 3 (Blue)",    60.0),
        ("Alt 4 (Yellow)", -60.0),
        ("Alt 5 (White)",    0.0),
    ].iter().enumerate() {
        let map_id = det_uuid(&format!("{}::palette_map_alt{}", char_id, idx));
        let colors = if *name == "Alt 5 (White)" {
            palette.iter().enumerate().map(|(i, rgb)| {
                let slot_id = det_uuid(&format!("{}::palette_color_{}", char_id, i));
                let g = (rgb[0] as u32 * 299 + rgb[1] as u32 * 587 + rgb[2] as u32 * 114) / 1000;
                json!({ "paletteColorId": slot_id, "targetColor": format!("0xFF{:02X}{:02X}{:02X}", g, g, g) })
            }).collect::<Vec<_>>()
        } else { build_map_colors(*hue) };
        maps.push(json!({
            "$id": map_id, "colors": colors, "name": name,
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "isBase": false } }
        }));
    }

    build_output(char_id, char_name, color_slots, maps, preview_png, &base_map_id)
}

// ─── Shared output builder ────────────────────────────────────────────────────

fn build_output(
    char_id:      &str,
    char_name:    &str,
    color_slots:  Vec<Value>,
    maps:         Vec<Value>,
    preview_png:  Vec<u8>,
    base_map_id:  &str,
) -> anyhow::Result<PaletteResult> {
    let collection_guid    = det_uuid(&format!("{}::palettes_guid",       char_id));
    let preview_meta_guid  = det_uuid(&format!("{}::palette_preview_meta", char_id));
    let palettes_meta_guid = det_uuid(&format!("{}::palettes_file_meta",   char_id));

    let palettes_json = serde_json::to_string_pretty(&json!({
        "colors": color_slots,
        "export": true,
        "guid": collection_guid,
        "id": format!("{}Costumes", char_name),
        "imageAsset": preview_meta_guid,
        "maps": maps,
        "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "version": "0.3.1" } },
        "plugins": ["com.fraymakers.FraymakersMetadata"],
        "tags": [],
        "version": 1
    }))?;

    let palettes_meta_json = serde_json::to_string_pretty(&json!({
        "export": true,
        "guid": palettes_meta_guid,
        "id": format!("{}Costumes", char_name),
        "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
    }))?;

    let preview_meta_json = serde_json::to_string_pretty(&json!({
        "export": false,
        "guid": preview_meta_guid,
        "id": "",
        "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2
    }))?;

    Ok(PaletteResult {
        palettes_json, palettes_meta_json, preview_png, preview_meta_json,
        collection_guid, base_map_id: base_map_id.to_string(),
    })
}

// ─── Public entry point ───────────────────────────────────────────────────────

/// Generate palette data for a character.
///
/// If `costumes_json` points to a valid ssf2_costumes.json and the character
/// is found in it, use real SSF2 data. Otherwise fall back to k-means from sprites.
pub fn generate_palettes_and_remap(
    char_id:      &str,
    char_name:    &str,
    sprites_dir:  &Path,
    costumes_json: Option<&Path>,
) -> anyhow::Result<PaletteResult> {
    // Try real SSF2 data first
    let result = if let Some(costumes) = costumes_json
        .and_then(|json_path| load_ssf2_costumes(json_path, char_name))
    {
        log::info!("palette_gen: using real SSF2 costume data ({} costumes) for {}", costumes.len(), char_name);
        build_from_ssf2(char_id, char_name, &costumes)?
    } else {
        if costumes_json.is_some() {
            log::warn!("palette_gen: '{}' not found in costumes json, falling back to k-means", char_name);
        }
        log::info!("palette_gen: using k-means sprite sampling for {}", char_name);
        build_from_sprites(char_id, char_name, sprites_dir)?
    };

    // Snap every baked skew_* bitmap to the costume color array. The skew bake's bicubic
    // resampling introduces colours that aren't in the costume palette (e.g. (96,94,105) on
    // sandbag's jab), and Fraymakers' palette-swap can't map a non-costume colour → the
    // sprite flickers. Snapping each pixel to the nearest costume colour keeps every skew
    // bitmap fully inside the swappable palette. (skew_* are the stretched/skewed Image-layer
    // character bitmaps; vector-rendered effects are never baked to skew_* files.)
    // The costume color array + which slots are CHANGEABLE (recolored by at least one costume
    // map). Slots that no map ever changes are "fixed" (eyes/special); body pixels must not
    // mis-snap onto those or they'd refuse to recolor. Parsed from the generated palettes JSON.
    let parse_hex = |s: &str| u32::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok();
    let (costume_colors, changeable): (Vec<u32>, Vec<bool>) =
        match serde_json::from_str::<Value>(&result.palettes_json) {
            Ok(v) => {
                let colors: Vec<u32> = v.get("colors").and_then(|c| c.as_array()).map(|arr| {
                    arr.iter().filter_map(|e| e.get("color").and_then(|s| s.as_str()))
                        .filter_map(&parse_hex).collect()
                }).unwrap_or_default();
                let mut chg_count = vec![0usize; colors.len()];
                let mut nmaps = 0usize;
                if let Some(maps) = v.get("maps").and_then(|m| m.as_array()) {
                    for mp in maps {
                        if let Some(cols) = mp.get("colors").and_then(|c| c.as_array()) {
                            nmaps += 1;
                            for (i, e) in cols.iter().enumerate() {
                                if i >= colors.len() { break; }
                                if let Some(t) = e.get("targetColor").and_then(|s| s.as_str()).and_then(parse_hex) {
                                    // compare RGB (ignore alpha byte)
                                    if (t & 0xFFFFFF) != (colors[i] & 0xFFFFFF) { chg_count[i] += 1; }
                                }
                            }
                        }
                    }
                }
                // A slot is CHANGEABLE if a MAJORITY of costumes recolour it. The fixed/eye
                // ramp (recoloured by at most a couple of alt skins) counts as fixed, so body
                // pixels that bicubic-blended onto it are corrected rather than left grey.
                let chg: Vec<bool> = chg_count.iter().map(|&c| c * 2 > nmaps).collect();
                (colors, chg)
            }
            Err(_) => (Vec::new(), Vec::new()),
        };
    snap_skew_bitmaps_to_palette(sprites_dir, &costume_colors, &changeable);
    Ok(result)
}

/// Snap every `skew_*.png` bitmap's opaque pixels to the nearest colour in the costume
/// color array (`colors` is ARGB). `changeable[i]` marks slots that at least one costume map
/// recolours; slots no map ever changes are "fixed" (eyes/special). Alpha is untouched.
fn snap_skew_bitmaps_to_palette(sprites_dir: &Path, colors: &[u32], changeable: &[bool]) {
    let palette: Vec<[u8; 3]> = colors.iter()
        .map(|&argb| [((argb >> 16) & 0xFF) as u8, ((argb >> 8) & 0xFF) as u8, (argb & 0xFF) as u8])
        .collect();
    if palette.is_empty() { return; }
    // Fixed (never-recoloured) palette colours, and the changeable-only sub-palette.
    let aligned = changeable.len() == palette.len();
    let fixed_set: std::collections::HashSet<[u8; 3]> = if aligned {
        palette.iter().zip(changeable).filter(|(_, &c)| !c).map(|(p, _)| *p).collect()
    } else { std::collections::HashSet::new() };
    let changeable_palette: Vec<[u8; 3]> = if aligned {
        palette.iter().zip(changeable).filter(|(_, &c)| c).map(|(p, _)| *p).collect()
    } else { palette.clone() };
    let entries = match fs::read_dir(sprites_dir) { Ok(e) => e, Err(_) => return };
    let mut snapped = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !(name.starts_with("skew_") && name.ends_with(".png")) { continue; }
        let mut img = match image::open(&path) { Ok(i) => i.to_rgba8(), Err(_) => continue };
        // Edge-colour bleed: copy the nearest SOLID (alpha>=250) pixel's RGB into the
        // antialiased (0<alpha<250) edge pixels, so the edge inherits the body colour
        // instead of the light bicubic blend it would otherwise carry (which snaps to the
        // lightest costume colour → light "speckle" along the silhouette when recoloured).
        // Alpha is preserved, so the edge still antialiases — just in the body's hue.
        bleed_edges(&mut img);
        for px in img.pixels_mut() {
            if px.0[3] == 0 { continue; }
            let rgb = [px.0[0], px.0[1], px.0[2]];
            let c = palette[nearest(&rgb, &palette)];
            if c != rgb { px.0[0] = c[0]; px.0[1] = c[1]; px.0[2] = c[2]; }
        }
        // Fix fixed-colour mis-snaps: the source palette has near-duplicate body vs fixed/eye
        // greys (e.g. body (151,149,160) vs eye (148,148,152)). Bicubic blending makes some
        // BODY pixels snap onto a FIXED colour, which then refuses to recolour → grey clusters
        // on a recoloured body. Any fixed-colour pixel surrounded mostly by changeable colours
        // is such a mis-snap → re-snap it to the nearest CHANGEABLE colour. Coherent fixed
        // regions (real eyes — fixed neighbours) are preserved.
        if !fixed_set.is_empty() && !changeable_palette.is_empty() {
            fix_fixed_mis_snaps(&mut img, &fixed_set, &changeable_palette);
        }
        // NOTE: no despeckle — it flattened fine shading gradients (lost detail). The fixed-
        // ramp fix above is the only recolorability correction; the bicubic detail is kept.
        let _ = despeckle;
        if img.save(&path).is_ok() { snapped += 1; }
    }
    if snapped > 0 {
        log::info!("palette_gen: snapped {} skew bitmap(s) to the {}-colour costume palette",
            snapped, palette.len());
    }
}

/// Re-snap fixed-colour pixels that sit in a changeable-colour region to the nearest CHANGEABLE
/// palette colour. Fixes body pixels that bicubic-blended onto a near-duplicate fixed/eye grey
/// (which wouldn't recolour). Pixels in a coherent fixed region (real eyes) are kept.
fn fix_fixed_mis_snaps(img: &mut image::RgbaImage, fixed_set: &std::collections::HashSet<[u8; 3]>, changeable_palette: &[[u8; 3]]) {
    let (w, h) = img.dimensions();
    let (w, h) = (w as i64, h as i64);
    // Iterate so a fixed-colour blob surrounded by body erodes from its boundary inward and
    // fully clears (skew bitmaps are sheared limbs/body — they carry no coherent eye region;
    // a real eye would be an all-fixed core with no changeable neighbours and is preserved).
    for _ in 0..12 {
        let snap: Vec<[u8; 4]> = img.pixels().map(|p| p.0).collect();
        let mut any = false;
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                if snap[i][3] == 0 { continue; }
                let rgb = [snap[i][0], snap[i][1], snap[i][2]];
                if !fixed_set.contains(&rgb) { continue; }
                let (mut opaque, mut chg) = (0u32, 0u32);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if dx == 0 && dy == 0 { continue; }
                        let (nx, ny) = (x + dx, y + dy);
                        if nx < 0 || ny < 0 || nx >= w || ny >= h { continue; }
                        let n = snap[(ny * w + nx) as usize];
                        if n[3] == 0 { continue; }
                        opaque += 1;
                        if !fixed_set.contains(&[n[0], n[1], n[2]]) { chg += 1; }
                    }
                }
                // Touching the body (>=3 changeable neighbours) → erode this fixed pixel to the
                // nearest changeable colour. Interior eye pixels (all-fixed neighbours) stay.
                if opaque >= 3 && chg >= 3 {
                    let c = changeable_palette[nearest(&rgb, changeable_palette)];
                    let p = img.get_pixel_mut(x as u32, y as u32);
                    p.0[0] = c[0]; p.0[1] = c[1]; p.0[2] = c[2];
                    any = true;
                }
            }
        }
        if !any { break; }
    }
}

/// Remove isolated single-pixel palette outliers (speckle) while preserving coherent regions
/// and thin outlines. A pixel is "speckle" if it has >= 5 opaque 8-neighbours yet <= 1 of them
/// share its exact colour; such a pixel is replaced with the most common neighbour colour.
/// Two passes catch small (2-3px) clusters. Operates on already-palette-snapped RGB.
fn despeckle(img: &mut image::RgbaImage) {
    let (w, h) = img.dimensions();
    let (w, h) = (w as i64, h as i64);
    for _ in 0..2 {
        let snap: Vec<[u8; 4]> = img.pixels().map(|p| p.0).collect();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                if snap[i][3] == 0 { continue; }
                let me = [snap[i][0], snap[i][1], snap[i][2]];
                let mut counts: std::collections::HashMap<[u8; 3], u32> = std::collections::HashMap::new();
                let (mut same, mut opaque) = (0u32, 0u32);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if dx == 0 && dy == 0 { continue; }
                        let (nx, ny) = (x + dx, y + dy);
                        if nx < 0 || ny < 0 || nx >= w || ny >= h { continue; }
                        let n = snap[(ny * w + nx) as usize];
                        if n[3] == 0 { continue; }
                        opaque += 1;
                        let rgb = [n[0], n[1], n[2]];
                        *counts.entry(rgb).or_insert(0) += 1;
                        if rgb == me { same += 1; }
                    }
                }
                if opaque >= 5 && same <= 1 {
                    if let Some((&rgb, _)) = counts.iter().max_by_key(|(_, c)| **c) {
                        let p = img.get_pixel_mut(x as u32, y as u32);
                        p.0[0] = rgb[0]; p.0[1] = rgb[1]; p.0[2] = rgb[2];
                    }
                }
            }
        }
    }
}

/// Dilate the RGB of SOLID pixels (alpha >= 250) outward into the antialiased fringe
/// (0 < alpha < 250) so each edge pixel takes its nearest solid neighbour's colour. Alpha is
/// untouched (the silhouette antialiasing is preserved); only the colour the fringe carries
/// changes, so it no longer reads as a light blend. Iterates a few rings to cover thick edges.
fn bleed_edges(img: &mut image::RgbaImage) {
    let (w, h) = img.dimensions();
    let (w, h) = (w as i64, h as i64);
    let mut filled: Vec<bool> = img.pixels().map(|p| p.0[3] >= 250).collect();
    for _ in 0..8 {
        let snap_rgb: Vec<[u8; 3]> = img.pixels().map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
        let prev_filled = filled.clone();
        let mut any = false;
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                if prev_filled[i] { continue; }
                if img.get_pixel(x as u32, y as u32).0[3] == 0 { continue; } // fully transparent
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w || ny >= h { continue; }
                    let ni = (ny * w + nx) as usize;
                    if prev_filled[ni] {
                        let n = snap_rgb[ni];
                        let p = img.get_pixel_mut(x as u32, y as u32);
                        p.0[0] = n[0]; p.0[1] = n[1]; p.0[2] = n[2]; // keep this pixel's own alpha
                        filled[i] = true;
                        any = true;
                        break;
                    }
                }
            }
        }
        if !any { break; }
    }
}
