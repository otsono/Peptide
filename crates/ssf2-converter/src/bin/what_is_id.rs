use ssf2_converter::*;
use std::env;
use std::io::Cursor;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 { eprintln!("Usage: {} <ssf> <id>", args[0]); return; }
    let target_id: u16 = args[2].parse().unwrap();

    let ssf_data = std::fs::read(&args[1]).unwrap();
    let swf_buf = swf::decompress_swf(Cursor::new(&ssf_data)).unwrap();
    let swf = swf::parse_swf(&swf_buf).unwrap();

    let mut symbols: std::collections::BTreeMap<u16, String> = Default::default();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                symbols.insert(link.id, name);
            }
        }
    }

    let sym = symbols.get(&target_id).map(|s| s.as_str()).unwrap_or("(no symbol)");
    println!("ID {}: sym='{}'", target_id, sym);

    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineShape(s) if s.id == target_id => {
                println!("  -> DefineShape, bounds: {:?}", s.shape_bounds);
            }
            swf::Tag::DefineSprite(s) if s.id == target_id => {
                println!("  -> DefineSprite ({} frames)", s.num_frames);
            }
            swf::Tag::DefineBitsLossless(b) if b.id == target_id => {
                println!("  -> DefineBitsLossless {}x{}", b.width, b.height);
            }
            swf::Tag::DefineBitsJpeg3(b) if b.id == target_id => {
                println!("  -> DefineBitsJpeg3");
            }
            swf::Tag::DefineMorphShape(s) if s.id == target_id => {
                println!("  -> DefineMorphShape");
            }
            _ => {}
        }
    }
}
