//! Probe: why do some effect entities come out with zero images?
//! For each discovered effect, report frame_count, how many frames the
//! flattener resolved to an image symbol, and (if zero) dump the raw
//! PlaceObject character IDs in the effect sprite's frames + whether each
//! resolves to a named symbol / a shape→bitmap / a (nested) sprite.

use ssf2_converter::image_extractor;
use std::collections::BTreeMap;

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe_effects <file.ssf> [char]");
    let char_name = std::env::args().nth(2).unwrap_or_else(|| {
        std::path::Path::new(&path).file_stem().unwrap().to_string_lossy().into_owned()
    });
    let ssf = std::fs::read(&path).unwrap();
    let swf_data = ssf2_converter::ssf::decompress(&ssf).unwrap();
    let empty: BTreeMap<String, String> = BTreeMap::new();

    let img = image_extractor::extract_images(&swf_data, std::path::Path::new("/tmp/_probe_imgs"), &char_name, &empty)
        .expect("extract_images");
    let (_projs, effects, _head) =
        image_extractor::discover_projectiles_and_head(&swf_data, &char_name).expect("discover");

    let swf_buf = swf::decompress_swf(&swf_data[..]).unwrap();
    let swf = swf::parse_swf(&swf_buf).unwrap();

    // sprite tag tables for raw inspection
    let mut sprites: BTreeMap<u16, &swf::Sprite> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(s) = tag { sprites.insert(s.id, s); }
    }

    // cid → SWF tag type (so we can classify "unknown" characters).
    let mut cid_kind: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineShape(s) => {
                let has_bmp = s.styles.fill_styles.iter().any(|f| matches!(f,
                    swf::FillStyle::Bitmap { id, .. } if *id != 65535));
                cid_kind.insert(s.id, format!("DefineShape (bitmap_fill={})", has_bmp));
            }
            swf::Tag::DefineSprite(s) => { cid_kind.insert(s.id, "DefineSprite".into()); }
            swf::Tag::DefineBitsLossless(b) => { cid_kind.insert(b.id, "DefineBitsLossless".into()); }
            swf::Tag::DefineBitsJpeg2 { id, .. } => { cid_kind.insert(*id, "DefineBitsJpeg2".into()); }
            swf::Tag::DefineBitsJpeg3(j) => { cid_kind.insert(j.id, "DefineBitsJpeg3".into()); }
            swf::Tag::DefineMorphShape(m) => { cid_kind.insert(m.id, "DefineMorphShape (vector tween)".into()); }
            swf::Tag::DefineText(t) => { cid_kind.insert(t.id, "DefineText".into()); }
            _ => {}
        }
    }
    let kind = |cid: u16| cid_kind.get(&cid).cloned().unwrap_or_else(|| "(no Define* tag)".into());
    let mut symbols: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for l in links {
                let n = l.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                if !n.is_empty() { symbols.insert(l.id, n); }
            }
        }
    }

    println!("char {}  — {} effects", char_name, effects.len());
    for e in &effects {
        let fi = image_extractor::extract_projectile_frame_images_from_swf(&swf, &char_name, e.sprite_id, &img)
            .unwrap_or(image_extractor::ProjectileFrameImages { frames: vec![], image_guids: BTreeMap::new() });
        let with_img = fi.frames.iter().filter(|f| f.is_some()).count();
        let flag = if with_img == 0 { "  <-- BLANK" } else { "" };
        println!("\n● {} (sprite {}): {} frames, {} resolved images{}",
            e.name, e.sprite_id, e.frame_count, with_img, flag);
        if with_img > 0 { continue; }

        // Dump what the effect sprite places, frame by frame.
        if let Some(sprite) = sprites.get(&e.sprite_id) {
            let mut depth_place: BTreeMap<u16, u16> = BTreeMap::new();
            let mut frame = 0;
            for stag in &sprite.tags {
                match stag {
                    swf::Tag::PlaceObject(po) => {
                        if let swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) = &po.action {
                            depth_place.insert(po.depth, *cid);
                        }
                    }
                    swf::Tag::RemoveObject(ro) => { depth_place.remove(&ro.depth); }
                    swf::Tag::ShowFrame => {
                        if frame < 3 {
                            for (d, cid) in &depth_place {
                                let kind = if let Some(n) = symbols.get(cid) {
                                    format!("named symbol '{}'", n)
                                } else if img.shape_to_bitmap.contains_key(cid) {
                                    "shape→bitmap".into()
                                } else if sprites.contains_key(cid) {
                                    let inner = sprites.get(cid).unwrap();
                                    let placed: Vec<u16> = inner.tags.iter().filter_map(|t| match t {
                                        swf::Tag::PlaceObject(p) => match &p.action {
                                            swf::PlaceObjectAction::Place(c)|swf::PlaceObjectAction::Replace(c)=>Some(*c),
                                            _=>None }, _=>None }).collect();
                                    let resolved: Vec<String> = placed.iter().map(|c| {
                                        if symbols.contains_key(c) { format!("{}=sym", c) }
                                        else if img.shape_to_bitmap.contains_key(c) { format!("{}=shape→bmp", c) }
                                        else { format!("{}=[{}]", c, kind(*c)) }
                                    }).collect();
                                    format!("NESTED sprite {} places: [{}]", cid, resolved.join(", "))
                                } else {
                                    format!("UNRESOLVED → {}", kind(*cid))
                                };
                                println!("    frame {} depth {} → cid {} = {}", frame, d, cid, kind);
                            }
                        }
                        frame += 1;
                    }
                    _ => {}
                }
            }
        }
    }
}
