/// Diagnostic: dump DefineSprite frame structure from SSF2 SWF.
use std::collections::BTreeMap;
use std::io::Cursor;

fn char_id_from_action(action: &swf::PlaceObjectAction) -> Option<u16> {
    match action {
        swf::PlaceObjectAction::Place(id) => Some(*id),
        swf::PlaceObjectAction::Replace(id) => Some(*id),
        swf::PlaceObjectAction::Modify => None,
    }
}

fn dump_sprite(sprite: &swf::Sprite, sym_names: &BTreeMap<u16, String>, indent: &str, filter: Option<&str>) {
    let sym = sym_names.get(&sprite.id).cloned().unwrap_or_default();
    if let Some(f) = filter {
        if !sym.to_lowercase().contains(f) { return; }
    }
    println!("{}=== Sprite id={} '{}' ({} frames) ===", indent, sprite.id, sym, sprite.num_frames);
    let mut sf = 0u16;
    for stag in &sprite.tags {
        match stag {
            swf::Tag::ShowFrame => sf += 1,
            swf::Tag::FrameLabel(fl) => {
                let label = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252);
                println!("{}  [{:3}] LABEL '{}'", indent, sf, label);
            }
            swf::Tag::PlaceObject(po) => {
                let inst = po.name.map(|s| s.to_str_lossy(encoding_rs::WINDOWS_1252).to_string()).unwrap_or_default();
                let cid = char_id_from_action(&po.action).unwrap_or(0);
                let csym = sym_names.get(&cid).cloned().unwrap_or_default();
                let is_hitbox = inst.contains("hitBox") || inst.contains("attackBox") || inst.contains("hurtBox");
                if is_hitbox {
                    if let Some(m) = &po.matrix {
                        // SWF matrix tx/ty are in twips (1/20th pixel), a/d are scale
                        let tx = m.tx.get() as f64 / 20.0;
                        let ty = m.ty.get() as f64 / 20.0;
                        let sx = m.a;
                        let sy = m.d;
                        println!("{}  [{:3}] BOX name='{}' tx={:.1} ty={:.1} scale_x={:.3} scale_y={:.3}", indent, sf, inst, tx, ty, sx, sy);
                    } else {
                        println!("{}  [{:3}] BOX name='{}' id={} sym='{}' (no matrix)", indent, sf, inst, cid, csym);
                    }
                } else if !csym.is_empty() && csym != "Main" {
                    println!("{}  [{:3}] PLACE id={} sym='{}' name='{}'", indent, sf, cid, csym, inst);
                }
            }
            swf::Tag::DoAction(avm1) => {
                println!("{}  [{:3}] DoAction ({} bytes)", indent, sf, avm1.len());
            }
            swf::Tag::DefineSprite(inner) => {
                dump_sprite(inner, sym_names, &format!("{}  ", indent), filter);
            }
            _ => {}
        }
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_sprites <file.ssf> [filter]");
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

    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            dump_sprite(sprite, &sym_names, "", filter.as_deref());
        }
    }
}
