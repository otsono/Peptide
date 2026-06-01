//! Scan every .ssf in the corpus. For each ABC block, find every method body
//! that is a "character bundle" (returns `{cData, aData, pData, iData}`) by
//! re-using the find_char_sources discriminator: a body that pushes the
//! strings normalStats_id, cData, aData, pData, AND iData.
//!
//! Extract the literal value of `normalStats_id` from each body, plus the
//! enclosing class + method name. Then group within each SSF by
//! normalStats_id and report any group with > 1 method. Those are
//! transformations (or alternate forms / skins).

use ssf2_converter::*;
use std::env;

fn encode_u30(mut v: u32) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 { out.push(b); break; }
        out.push(b | 0x80);
    }
    out
}

fn read_u30(data: &[u8], i: &mut usize) -> u32 {
    let mut r = 0u32; let mut shift = 0;
    while *i < data.len() {
        let b = data[*i] as u32; *i += 1;
        r |= (b & 0x7f) << shift; shift += 7;
        if b & 0x80 == 0 { break; }
    }
    r
}

fn body_pushes_string(body: &abc_parser::MethodBody, idx: u32) -> bool {
    let bytes = &body.bytecode;
    let target = encode_u30(idx);
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x2c && bytes[i+1..].starts_with(&target) {
            return true;
        }
        i += 1;
    }
    false
}

fn find_next_pushed_string(abc: &abc_parser::AbcFile, body: &abc_parser::MethodBody, needle_idx: u32) -> Option<String> {
    let bytes = &body.bytecode;
    let needle = encode_u30(needle_idx);
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x2c && bytes[i+1..].starts_with(&needle) {
            let mut j = i + 1 + needle.len();
            while j < bytes.len() {
                if bytes[j] == 0x2c {
                    let mut k = j + 1;
                    let v = read_u30(bytes, &mut k);
                    if let Some(s) = abc.strings.get(v as usize) {
                        return Some(s.clone());
                    }
                    return None;
                }
                j += 1;
            }
            return None;
        }
        i += 1;
    }
    None
}

fn main() {
    let ssfs_dir = env::args().nth(1).unwrap_or_else(||
        "../ssf2-ssfs".to_string());

    let mut files: Vec<_> = std::fs::read_dir(&ssfs_dir).unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "ssf").unwrap_or(false))
        .filter(|p| p.file_stem().map(|s| s != "misc").unwrap_or(true))
        .collect();
    files.sort();

    let mut grand_total_transformations = 0;

    for path in &files {
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let Ok(bytes) = std::fs::read(path) else { continue };
        let Ok(swf_bytes) = ssf::decompress(&bytes) else { continue };
        let Ok(swf) = swf_parser::parse(&swf_bytes) else { continue };

        // Map: normalStats_id -> Vec<(class_name, method_name)>
        let mut by_id: std::collections::BTreeMap<String, Vec<(String, String)>> = Default::default();

        for abc_bytes in &swf.abc_blocks {
            let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };

            let str_idx = |s: &str| -> Option<u32> {
                abc.strings.iter().position(|x| x == s).map(|i| i as u32)
            };
            let Some(nsi)  = str_idx("normalStats_id") else { continue };
            let Some(cd)   = str_idx("cData") else { continue };
            let Some(ad)   = str_idx("aData") else { continue };
            let Some(pd)   = str_idx("pData") else { continue };
            let Some(id)   = str_idx("iData") else { continue };

            let check_body = |class_name: &str, method_name: &str, body: &abc_parser::MethodBody,
                              by_id: &mut std::collections::BTreeMap<String, Vec<(String, String)>>| {
                if !body_pushes_string(body, nsi)  { return; }
                if !body_pushes_string(body, cd)   { return; }
                if !body_pushes_string(body, ad)   { return; }
                if !body_pushes_string(body, pd)   { return; }
                if !body_pushes_string(body, id)   { return; }
                if let Some(v) = find_next_pushed_string(&abc, body, nsi) {
                    by_id.entry(v).or_default().push((class_name.to_string(), method_name.to_string()));
                }
            };

            for class in &abc.classes {
                for t in class.instance_methods.iter().chain(class.class_methods.iter()) {
                    let Some(body) = abc.method_bodies.iter()
                        .find(|b| b.method_idx == t.method_idx) else { continue };
                    check_body(&class.name, &t.name, body, &mut by_id);
                }
            }
            for script in &abc.scripts {
                for t in &script.traits {
                    let Some(body) = abc.method_bodies.iter()
                        .find(|b| b.method_idx == t.method_idx) else { continue };
                    check_body("(script)", &t.name, body, &mut by_id);
                }
            }
        }

        // Report any normalStats_id with more than one bundle method.
        for (id, methods) in &by_id {
            if methods.len() > 1 {
                grand_total_transformations += 1;
                println!("{}  normalStats_id={:?}  ({} bundles):", stem, id, methods.len());
                for (cls, m) in methods {
                    println!("    class={:<20} method={}", cls, m);
                }
            }
        }
    }

    println!();
    println!("Total normalStats_id values with >1 bundle method: {}", grand_total_transformations);
}
