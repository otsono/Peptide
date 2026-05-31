use ssf2_converter::*;
use std::env;
use std::io::Cursor;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 { eprintln!("Usage: {} <ssf> <sprite_id>", args[0]); return; }
    let target_id: u16 = args[2].parse().expect("sprite_id must be u16");

    let ssf_data = std::fs::read(&args[1]).expect("read");
    let swf_buf = swf::decompress_swf(Cursor::new(&ssf_data)).expect("decompress");
    let swf = swf::parse_swf(&swf_buf).expect("parse");

    let mut symbols: std::collections::BTreeMap<u16, String> = Default::default();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                symbols.insert(link.id, name);
            }
        }
    }

    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            if sprite.id != target_id { continue; }
            let sym = symbols.get(&sprite.id).map(|s| s.as_str()).unwrap_or("(unnamed)");
            println!("Sprite id={} '{}' ({} frames)", sprite.id, sym, sprite.num_frames);
            let mut frame = 0u16;
            for stag in &sprite.tags {
                match stag {
                    swf::Tag::ShowFrame => { frame += 1; }
                    swf::Tag::PlaceObject(po) => {
                        let name = po.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
                        match &po.action {
                            swf::PlaceObjectAction::Place(id) => {
                                let sym = symbols.get(id).map(|s| s.as_str()).unwrap_or("?");
                                println!("  [f{:2}] PLACE id={} sym='{}' name={:?}", frame, id, sym, name);
                            }
                            swf::PlaceObjectAction::Replace(id) => {
                                let sym = symbols.get(id).map(|s| s.as_str()).unwrap_or("?");
                                println!("  [f{:2}] REPLACE id={} sym='{}' name={:?}", frame, id, sym, name);
                            }
                            swf::PlaceObjectAction::Modify => {
                                println!("  [f{:2}] MODIFY depth={}", frame, po.depth);
                            }
                        }
                    }
                    swf::Tag::RemoveObject(ro) => {
                        println!("  [f{:2}] REMOVE depth={}", frame, ro.depth);
                    }
                    swf::Tag::FrameLabel(fl) => {
                        let label = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252);
                        println!("  [f{:2}] LABEL '{}'", frame, label);
                    }
                    _ => {}
                }
            }
        }
    }
}
