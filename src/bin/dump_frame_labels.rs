use std::io::Cursor;
use std::collections::BTreeMap;

fn main() {
    let ssf_data = std::fs::read("/Users/jimmy/.openclaw/workspace-main/ssf2-ssfs/sandbag.ssf").unwrap();
    let swf_buf = swf::decompress_swf(Cursor::new(&ssf_data)).unwrap();
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

    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sym = symbols.get(&sprite.id).cloned().unwrap_or_default();
            if sym.to_lowercase() != "sandbag" { continue; }

            println!("=== Root MC frame labels ===");
            for stag in &sprite.tags {
                match stag {
                    swf::Tag::FrameLabel(fl) => {
                        let label = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                        println!("  {}", label);
                    }
                    _ => {}
                }
            }
            break;
        }
    }
}
