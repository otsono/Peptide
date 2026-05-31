/// Diagnostic: dump DefineShape bounding boxes to understand image registration points.
use std::collections::BTreeMap;
use std::io::Cursor;

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_shape_bounds <file.ssf> [symbol_filter]");
    let filter = std::env::args().nth(2);
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

    // Build shape → bitmap map
    let mut shape_to_bitmap: BTreeMap<u16, u16> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::DefineShape(shape) = tag {
            for fill in &shape.styles.fill_styles {
                if let swf::FillStyle::Bitmap { id, matrix, .. } = fill {
                    shape_to_bitmap.insert(shape.id, *id);
                    let sym = sym_names.get(&shape.id).cloned().unwrap_or_default();
                    let bmp_sym = sym_names.get(id).cloned().unwrap_or_default();
                    
                    if let Some(ref f) = filter {
                        if !sym.to_lowercase().contains(f.as_str()) && !bmp_sym.to_lowercase().contains(f.as_str()) {
                            break;
                        }
                    }
                    
                    let xmin = shape.shape_bounds.x_min.get() as f64 / 20.0;
                    let ymin = shape.shape_bounds.y_min.get() as f64 / 20.0;
                    let xmax = shape.shape_bounds.x_max.get() as f64 / 20.0;
                    let ymax = shape.shape_bounds.y_max.get() as f64 / 20.0;
                    let w = xmax - xmin;
                    let h = ymax - ymin;
                    
                    let bfm = matrix;
                    let btx = bfm.tx.get() as f64 / 20.0;
                    let bty = bfm.ty.get() as f64 / 20.0;
                    let bsx = bfm.a.to_f64();
                    let bsy = bfm.d.to_f64();
                    
                    println!("Shape id={} sym='{}' → bitmap id={} sym='{}'", shape.id, sym, id, bmp_sym);
                    println!("  shape_bounds: xmin={:.1} ymin={:.1} xmax={:.1} ymax={:.1} (w={:.1} h={:.1})", xmin, ymin, xmax, ymax, w, h);
                    println!("  bitmap_fill_matrix: tx={:.3} ty={:.3} sx={:.3} sy={:.3}", btx, bty, bsx, bsy);
                    break;
                }
            }
        }
    }
}
