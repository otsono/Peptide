use ssf2_converter::*;
use std::env;
use std::io::Cursor;
use std::collections::BTreeMap;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 { eprintln!("Usage: {} <ssf>", args[0]); return; }

    let ssf_data = std::fs::read(&args[1]).unwrap();
    let swf_buf = swf::decompress_swf(Cursor::new(&ssf_data)).unwrap();
    let swf = swf::parse_swf(&swf_buf).unwrap();

    let mut symbols: BTreeMap<u16, String> = Default::default();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                symbols.insert(link.id, name);
            }
        }
    }

    let mut all_sprites: BTreeMap<u16, &swf::Sprite> = Default::default();
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(s) = tag { all_sprites.insert(s.id, s); }
    }

    // Find outer projectile wrappers: named sprites that place an inner "stance" sprite
    for (&sid, sprite) in &all_sprites {
        let sym = match symbols.get(&sid) { Some(s) => s.clone(), None => continue };
        // collect frame→(label, inner_id)
        let mut frame: u16 = 0;
        let mut states: Vec<(String, u16, String)> = Vec::new(); // (label, inner_id, inner_sym)
        let mut cur_label = String::new();
        let mut found_stance = false;
        for stag in &sprite.tags {
            match stag {
                swf::Tag::ShowFrame => frame += 1,
                swf::Tag::FrameLabel(fl) => {
                    cur_label = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                }
                swf::Tag::PlaceObject(po) => {
                    let name = po.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
                    if name.as_deref() == Some("stance") {
                        found_stance = true;
                        if let swf::PlaceObjectAction::Place(cid) = &po.action {
                            let inner_sym = symbols.get(cid).cloned().unwrap_or_default();
                            if !cur_label.is_empty() {
                                states.push((cur_label.clone(), *cid, inner_sym));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // Only print if multiple states
        if states.len() > 1 {
            println!("\n=== {} (id={}, {} frames) ===", sym, sid, sprite.num_frames);
            for (label, inner_id, inner_sym) in &states {
                let inner_frames = all_sprites.get(inner_id).map(|s| s.num_frames).unwrap_or(0);
                // Get inner labels
                let inner_labels: Vec<String> = all_sprites.get(inner_id).map(|s| {
                    let mut labs = Vec::new();
                    for t in &s.tags {
                        if let swf::Tag::FrameLabel(fl) = t {
                            labs.push(fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
                        }
                    }
                    labs
                }).unwrap_or_default();
                println!("  state='{}' → inner='{}' ({} frames) inner_labels={:?}", label, inner_sym, inner_frames, inner_labels);
            }
        }
    }
}
