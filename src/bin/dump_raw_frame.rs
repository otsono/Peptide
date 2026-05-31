/// Dump raw PlaceObject matrices for a specific animation sprite at specific frames
use std::io::Cursor;
use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    let raw = std::fs::read("../ssf2-ssfs/sandbag.ssf")?;
    let swf_data = ssf2_converter::ssf::decompress(&raw)?;
    let swf_buf = swf::decompress_swf(Cursor::new(&swf_data))?;
    let swf = swf::parse_swf(&swf_buf)?;

    let mut symbols: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                symbols.insert(link.id, name);
            }
        }
    }

    // Find DownAir sprite
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sym = symbols.get(&sprite.id).map(|s| s.as_str()).unwrap_or("");
            if !sym.contains("DownAir") { continue; }
            println!("Sprite: {} (id={})", sym, sprite.id);

            let mut frame: u16 = 0;
            let mut disp: BTreeMap<u16, (u16, f64, f64, f64, f64, f64, f64)> = BTreeMap::new();

            for stag in &sprite.tags {
                match stag {
                    swf::Tag::ShowFrame => {
                        if frame <= 5 {
                            println!("\n  Frame {}:", frame);
                            for (&depth, &(cid, a, b, c, d, tx, ty)) in &disp {
                                let sym_name = symbols.get(&cid).cloned().unwrap_or_default();
                                let rot = b.atan2(a).to_degrees();
                                let sx = (a*a+b*b).sqrt();
                                let sy = (c*c+d*d).sqrt();
                                println!("    depth={} id={} sym={} | a={:.3} b={:.3} c={:.3} d={:.3} tx={:.2} ty={:.2} | sx={:.3} sy={:.3} rot={:.1}°",
                                    depth, cid, &sym_name[..sym_name.len().min(30)],
                                    a, b, c, d, tx, ty, sx, sy, rot);
                            }
                        }
                        frame += 1;
                    }
                    swf::Tag::PlaceObject(po) => {
                        let mat = po.matrix.map(|m| (
                            m.a.to_f64(), m.b.to_f64(), m.c.to_f64(), m.d.to_f64(),
                            m.tx.get() as f64 / 20.0, m.ty.get() as f64 / 20.0
                        ));
                        match &po.action {
                            swf::PlaceObjectAction::Place(cid) | swf::PlaceObjectAction::Replace(cid) => {
                                if let Some((a,b,c,d,tx,ty)) = mat {
                                    disp.insert(po.depth, (*cid, a, b, c, d, tx, ty));
                                }
                            }
                            swf::PlaceObjectAction::Modify => {
                                if let Some((a,b,c,d,tx,ty)) = mat {
                                    if let Some(e) = disp.get_mut(&po.depth) {
                                        e.1=a; e.2=b; e.3=c; e.4=d; e.5=tx; e.6=ty;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Also dump root MC for a_air_down label to see xframe matrix
    println!("\n\n=== Root MC xframe for a_air_down ===");
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            let sym = symbols.get(&sprite.id).map(|s| s.as_str()).unwrap_or("");
            if sym.to_lowercase() != "sandbag" { continue; }

            let mut in_airdowN = false;
            for stag in &sprite.tags {
                match stag {
                    swf::Tag::FrameLabel(fl) => {
                        let label = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                        in_airdowN = label == "a_air_down";
                    }
                    swf::Tag::PlaceObject(po) if in_airdowN => {
                        let inst = po.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string()).unwrap_or_default();
                        if inst == "stance" {
                            if let Some(m) = &po.matrix {
                                let a=m.a.to_f64(); let b=m.b.to_f64(); let c=m.c.to_f64(); let d=m.d.to_f64();
                                let tx=m.tx.get() as f64/20.0; let ty=m.ty.get() as f64/20.0;
                                println!("  stance: a={:.3} b={:.3} c={:.3} d={:.3} tx={:.2} ty={:.2}", a,b,c,d,tx,ty);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
