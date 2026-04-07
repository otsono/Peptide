/// Find shapes that reference specific bitmaps
use std::io::Cursor;
use std::collections::BTreeMap;

fn main() {
    let raw = std::fs::read("/Users/jimmy/.openclaw/workspace-main/ssf2-ssfs/sandbag.ssf").unwrap();
    let swf_buf = swf::decompress_swf(Cursor::new(&raw)).unwrap();
    let swf = swf::parse_swf(&swf_buf).unwrap();

    let mut symbols: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                symbols.insert(link.id, name);
            }
        }
    }

    let targets = [19u16, 256, 255, 261]; // sand_sprite0, sand_sprite2, sand_sprite1, sand_blink0
    
    println!("Looking for shapes referencing bitmaps {:?}", targets);
    for tag in &swf.tags {
        if let swf::Tag::DefineShape(shape) = tag {
            for fill in &shape.styles.fill_styles {
                if let swf::FillStyle::Bitmap { id, matrix, .. } = fill {
                    if targets.contains(id) {
                        let sym = symbols.get(id).cloned().unwrap_or_default();
                        let b = &shape.shape_bounds;
                        let mat_tx = matrix.tx.get() as f64 / 20.0;
                        let mat_ty = matrix.ty.get() as f64 / 20.0;
                        let mat_a = matrix.a.to_f64();
                        let mat_d = matrix.d.to_f64();
                        println!("  Shape {} → bitmap {} ({}) bounds=({:.1},{:.1})-({:.1},{:.1}) fill_matrix: tx={:.1} ty={:.1} a={:.3} d={:.3}",
                            shape.id, id, sym,
                            b.x_min.get() as f64 / 20.0, b.y_min.get() as f64 / 20.0,
                            b.x_max.get() as f64 / 20.0, b.y_max.get() as f64 / 20.0,
                            mat_tx, mat_ty, mat_a, mat_d);
                    }
                }
            }
        }
    }
    
    // Also check: what does PlaceObject for DownAir actually reference?
    // Is it really placing bitmap 256 directly, or through a sprite?
    println!("\nChecking DownAir sprite (id=359) PlaceObject character IDs:");
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            if sprite.id != 359 { continue; }
            let mut frame = 0u16;
            for stag in &sprite.tags {
                match stag {
                    swf::Tag::ShowFrame => frame += 1,
                    swf::Tag::PlaceObject(po) => {
                        if frame <= 2 {
                            match &po.action {
                                swf::PlaceObjectAction::Place(cid) => {
                                    let sym = symbols.get(cid).cloned().unwrap_or_default();
                                    println!("  f{} depth={} PLACE id={} sym={}",
                                        frame, po.depth, cid, if sym.is_empty() { "(none)" } else { &sym });
                                }
                                swf::PlaceObjectAction::Replace(cid) => {
                                    let sym = symbols.get(cid).cloned().unwrap_or_default();
                                    println!("  f{} depth={} REPLACE id={} sym={}",
                                        frame, po.depth, cid, if sym.is_empty() { "(none)" } else { &sym });
                                }
                                swf::PlaceObjectAction::Modify => {
                                    println!("  f{} depth={} MODIFY", frame, po.depth);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
