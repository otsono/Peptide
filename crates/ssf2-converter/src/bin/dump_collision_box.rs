use std::io::Cursor;
use std::collections::BTreeMap;

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_collision_box <file.ssf>");
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

    let mut shape_bounds: BTreeMap<u16, (f64,f64,f64,f64)> = BTreeMap::new();
    for tag in &swf.tags {
        let (id, b) = match tag {
            swf::Tag::DefineShape(s) => (s.id, &s.shape_bounds),
            _ => continue,
        };
        let (xmin,xmax,ymin,ymax) = (b.x_min.get() as f64/20.0, b.x_max.get() as f64/20.0, b.y_min.get() as f64/20.0, b.y_max.get() as f64/20.0);
        shape_bounds.insert(id, (xmin, ymin, xmax, ymax));
    }

    // Inspect sprite 990 (itemPlaceholder_mc_5)
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(s) = tag {
            if s.id != 990 { continue; }
            let sym = sym_names.get(&s.id).cloned().unwrap_or_default();
            println!("Sprite id={} sym='{}':", s.id, sym);
            for stag in &s.tags {
                if let swf::Tag::PlaceObject(po) = stag {
                    let child_id = match &po.action {
                        swf::PlaceObjectAction::Place(id) | swf::PlaceObjectAction::Replace(id) => *id,
                        swf::PlaceObjectAction::Modify => continue,
                    };
                    let child_sym = sym_names.get(&child_id).cloned().unwrap_or_default();
                    if let Some(m) = &po.matrix {
                        let tx = m.tx.get() as f64 / 20.0;
                        let ty = m.ty.get() as f64 / 20.0;
                        let sx = m.a.to_f64(); let sy = m.d.to_f64();
                        let bx = m.b.to_f64(); let by = m.c.to_f64();
                        let bd = shape_bounds.get(&child_id)
                            .map(|(x0,y0,x1,y1)| format!("shape: x={:.1}..{:.1} y={:.1}..{:.1} (w={:.1} h={:.1})", x0,x1,y0,y1,x1-x0,y1-y0))
                            .unwrap_or_else(|| format!("(no shape bounds for id={})", child_id));
                        println!("  child id={} sym='{}': tx={:.2} ty={:.2} sx={:.3} sy={:.3} bx={:.3} by={:.3} | {}", 
                            child_id, child_sym, tx, ty, sx, sy, bx, by, bd);
                    }
                }
            }
        }
    }
}
