//! Dump the raw PlaceObject matrices for every collision box in
//! sandbag's aerial_down sprite, per frame. Used to confirm the
//! matrix_to_box scale-decomposition bug (a/d-only read collapses
//! rotated boxes to 0×0).

use std::collections::BTreeMap;
use std::io::Cursor;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(||
        "../ssf2-ssfs/sandbag.ssf".to_string());
    let data = std::fs::read(&path).expect("read");
    let buf = swf::decompress_swf(Cursor::new(&data)).expect("decompress");
    let swf = swf::parse_swf(&buf).expect("parse");

    let mut sym_names: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for l in links {
                sym_names.insert(l.id, l.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
            }
        }
    }

    // Find the aerial_down sprite. SSF2 sandbag names it sandbag_fla.DAir_* or
    // similar; scan for a sprite whose symbol name contains "dair" or "adown".
    let target = std::env::args().nth(2).unwrap_or_else(|| "dair".to_string());
    let mut found = false;
    for tag in &swf.tags {
        let swf::Tag::DefineSprite(s) = tag else { continue };
        let sym = sym_names.get(&s.id).cloned().unwrap_or_default();
        if !sym.to_lowercase().contains(&target.to_lowercase()) { continue; }
        found = true;
        println!("=== sprite id={} sym='{}' ===", s.id, sym);

        let mut frame = 0u16;
        // depth → (name, matrix)
        let mut dl: BTreeMap<u16, (String, swf::Matrix)> = BTreeMap::new();
        for stag in &s.tags {
            match stag {
                swf::Tag::ShowFrame => {
                    for (depth, (name, m)) in &dl {
                        let nl = name.to_lowercase();
                        if !(nl.contains("box") || nl.contains("hit")) { continue; }
                        let a = m.a.to_f64(); let b = m.b.to_f64();
                        let c = m.c.to_f64(); let d = m.d.to_f64();
                        let ad_w = (a * 100.0).abs();
                        let ad_h = (d * 100.0).abs();
                        let proper_w = ((a*a + b*b).sqrt() * 100.0).abs();
                        let proper_h = ((c*c + d*d).sqrt() * 100.0).abs();
                        println!("  f{:<3} depth={:<4} name={:<14} a={:+.3} b={:+.3} c={:+.3} d={:+.3} | a/d_w={:6.1} a/d_h={:6.1} | proper_w={:6.1} proper_h={:6.1}",
                            frame, depth, name, a, b, c, d, ad_w, ad_h, proper_w, proper_h);
                    }
                    frame += 1;
                }
                swf::Tag::PlaceObject(po) => {
                    let nm = po.name.map(|s| s.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
                    match &po.action {
                        swf::PlaceObjectAction::Place(_) | swf::PlaceObjectAction::Replace(_) => {
                            let e = dl.entry(po.depth).or_insert((String::new(), swf::Matrix::IDENTITY));
                            if let Some(n) = nm { if !n.is_empty() { e.0 = n; } }
                            if let Some(mm) = po.matrix { e.1 = mm; }
                        }
                        swf::PlaceObjectAction::Modify => {
                            if let Some(e) = dl.get_mut(&po.depth) {
                                if let Some(n) = nm { if !n.is_empty() { e.0 = n; } }
                                if let Some(mm) = po.matrix { e.1 = mm; }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if !found { println!("(no sprite matching {:?})", target); }
}
