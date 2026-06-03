/// Diagnostic: dump image placement matrices to understand coordinate origins.
use std::collections::BTreeMap;
use std::io::Cursor;

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_image_placement <file.ssf> [anim_filter]");
    let anim_filter = std::env::args().nth(2).unwrap_or_else(|| "idle".to_string());
    let data = std::fs::read(&path).expect("read file");
    let swf_buf = swf::decompress_swf(Cursor::new(&data)).expect("decompress");
    let swf = swf::parse_swf(&swf_buf).expect("parse");

    let mut sym_names: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                sym_names.insert(link.id, name);
            }
        }
    }

    // Find the root MC placement for 'idle'/'stand' (stance matrix) and then
    // find the sub-sprite and its image placements
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sym = sym_names.get(&sprite.id).cloned().unwrap_or_default();
            // Look for animation sub-sprites that contain image placements + hitboxes
            if !sym.contains("_fla.") { continue; }
            if !sym.to_lowercase().contains(&anim_filter.to_lowercase()) { continue; }
            
            println!("=== Sprite id={} '{}' ===", sprite.id, sym);
            let mut frame = 0u16;
            for stag in &sprite.tags {
                match stag {
                    swf::Tag::ShowFrame => frame += 1,
                    swf::Tag::PlaceObject(po) => {
                        let inst = po.name.as_ref()
                            .map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string())
                            .unwrap_or_default();
                        let char_id = match &po.action {
                            swf::PlaceObjectAction::Place(id) => *id,
                            swf::PlaceObjectAction::Replace(id) => *id,
                            swf::PlaceObjectAction::Modify => continue,
                        };
                        let csym = sym_names.get(&char_id).cloned().unwrap_or_default();
                        if let Some(m) = &po.matrix {
                            let tx = m.tx.get() as f64 / 20.0;
                            let ty = m.ty.get() as f64 / 20.0;
                            let sx = m.a.to_f64();
                            let sy = m.d.to_f64();
                            println!("  [f{:3}] PLACE id={} sym='{}' inst='{}' tx={:.2} ty={:.2} sx={:.3} sy={:.3}",
                                frame, char_id, csym, inst, tx, ty, sx, sy);
                        } else {
                            println!("  [f{:3}] PLACE id={} sym='{}' inst='{}' (no matrix)",
                                frame, char_id, csym, inst);
                        }
                    }
                    _ => {}
                }
            }
            println!();
        }
    }

    // Also show the root MC stance placements for the animation
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sym = sym_names.get(&sprite.id).cloned().unwrap_or_default();
            if sym.to_lowercase() != "mario" { continue; }
            println!("=== ROOT MC '{}' stance placements ===", sym);
            let mut frame = 0u16;
            let mut label = String::new();
            for stag in &sprite.tags {
                match stag {
                    swf::Tag::ShowFrame => frame += 1,
                    swf::Tag::FrameLabel(fl) => {
                        label = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                    }
                    swf::Tag::PlaceObject(po) => {
                        let inst = po.name.as_ref()
                            .map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string())
                            .unwrap_or_default();
                        if inst != "stance" { continue; }
                        let char_id = match &po.action {
                            swf::PlaceObjectAction::Place(id) | swf::PlaceObjectAction::Replace(id) => *id,
                            swf::PlaceObjectAction::Modify => continue,
                        };
                        let csym = sym_names.get(&char_id).cloned().unwrap_or_default();
                        if !csym.to_lowercase().contains(&anim_filter.to_lowercase()) { continue; }
                        if let Some(m) = &po.matrix {
                            let tx = m.tx.get() as f64 / 20.0;
                            let ty = m.ty.get() as f64 / 20.0;
                            let sx = m.a.to_f64();
                            let sy = m.d.to_f64();
                            println!("  [f{:3}] label='{}' STANCE id={} sym='{}' tx={:.2} ty={:.2} sx={:.3} sy={:.3}",
                                frame, label, char_id, csym, tx, ty, sx, sy);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
