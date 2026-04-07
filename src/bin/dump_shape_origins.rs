/// Dump shape bounds to understand sprite registration/pivot points
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

    // Dump ALL shape tags to see what format they use
    let mut shape_count = 0;
    for tag in &swf.tags {
        let (id, bounds) = match tag {
            swf::Tag::DefineShape(s) => (s.id, &s.shape_bounds),
            _ => continue,
        };
        let sym = symbols.get(&id).cloned().unwrap_or_default();
        let x_min = bounds.x_min.get() as f64 / 20.0;
        let y_min = bounds.y_min.get() as f64 / 20.0;
        let x_max = bounds.x_max.get() as f64 / 20.0;
        let y_max = bounds.y_max.get() as f64 / 20.0;
        if !sym.is_empty() || shape_count < 5 {
            println!("Shape id={:<4} sym={:<30} bounds=({:7.1},{:7.1})-({:7.1},{:7.1}) pivot=({:.1},{:.1})",
                id, if sym.is_empty() { "(unnamed)" } else { &sym },
                x_min, y_min, x_max, y_max,
                -x_min, -y_min);
        }
        shape_count += 1;
    }
    println!("\nTotal shapes: {}", shape_count);

    // Also check bitmap definitions
    println!("\nBitmaps:");
    for tag in &swf.tags {
        let id = match tag {
            swf::Tag::DefineBitsLossless(b) => b.id,
            swf::Tag::DefineBitsJpeg3(b) => b.id,
            _ => continue,
        };
        let sym = symbols.get(&id).cloned().unwrap_or_default();
        println!("  Bitmap id={} sym={}", id, if sym.is_empty() { "(unnamed)" } else { &sym });
    }
}
