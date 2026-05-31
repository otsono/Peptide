use ssf2_converter::*;
use std::env;
use std::io::Cursor;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <ssf_file>", args[0]);
        return;
    }

    let ssf_data = std::fs::read(&args[1]).expect("failed to read ssf");
    let swf_buf = swf::decompress_swf(Cursor::new(ssf_data)).expect("failed to decompress");
    let swf = swf::parse_swf(&swf_buf).expect("failed to parse swf");

    // Build SymbolClass map
    let mut symbols = std::collections::BTreeMap::new();
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

    // Dump top-level PlaceObject tags
    println!("=== Top-level stage PlaceObject tags ===");
    for tag in &swf.tags {
        if let swf::Tag::PlaceObject(po) = tag {
            let name = po.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
            match &po.action {
                swf::PlaceObjectAction::Place(id) => {
                    let sym = symbols.get(id).map(|s| s.as_str()).unwrap_or("?");
                    println!("  PLACE (depth {}) id={} sym='{}' name={:?}", po.depth, id, sym, name);
                }
                swf::PlaceObjectAction::Replace(id) => {
                    let sym = symbols.get(id).map(|s| s.as_str()).unwrap_or("?");
                    println!("  REPLACE (depth {}) id={} sym='{}' name={:?}", po.depth, id, sym, name);
                }
                _ => {}
            }
        }
        if let swf::Tag::ShowFrame = tag {
            println!("  FRAME");
            break;
        }
    }
}
