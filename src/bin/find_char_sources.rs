//! Discriminator probe: scan every class + method body for the literal
//! string `normalStats_id`. Every character-shape stats object SSF2
//! produces carries this key — present in both:
//!   - `<X>Ext::getOwnStats` (typical: returns the inlined cData object)
//!   - `Main::get<Name>` (paired/secondary: returns `{cData,aData,pData,iData}`
//!     where `cData` itself carries `normalStats_id`).
//!
//! For each hit, extract the literal string value of `normalStats_id`
//! (the explicit character name SSF2 records). Then look for the four-
//! field bundle marker keys (`cData`, `aData`, `pData`, `iData`) to
//! distinguish "wrapped bundle" sources from "inline cData" sources.

use ssf2_converter::*;
use std::env;

fn body_pushes_string(body: &abc_parser::MethodBody, idx: u32) -> bool {
    // Check whether the bytecode contains `OP_PUSHSTRING <idx>` (0x2c
    // followed by the u30-encoded idx). Cheap byte-level scan — no
    // stack interpretation needed.
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

fn main() {
    let ssfs_dir = env::args().nth(1).unwrap_or_else(|| {
        "../ssf2-ssfs".to_string()
    });
    let only_ssf = env::args().nth(2); // optional: limit to one file

    let mut files: Vec<_> = std::fs::read_dir(&ssfs_dir).unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "ssf").unwrap_or(false))
        .filter(|p| p.file_stem().map(|s| s != "misc").unwrap_or(true))
        .collect();
    files.sort();
    if let Some(only) = only_ssf {
        files.retain(|p| p.file_stem().and_then(|s| s.to_str()) == Some(only.as_str()));
    }

    for path in &files {
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let Ok(bytes) = std::fs::read(path) else { continue };
        let Ok(swf_bytes) = ssf::decompress(&bytes) else { continue };
        let Ok(swf) = swf_parser::parse(&swf_bytes) else { continue };
        for abc_bytes in &swf.abc_blocks {
            let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };

            // Index lookup helpers.
            let str_idx = |s: &str| -> Option<u32> {
                abc.strings.iter().position(|x| x == s).map(|i| i as u32)
            };

            let Some(nsi_idx)  = str_idx("normalStats_id") else { continue };
            let cdata_idx  = str_idx("cData");
            let adata_idx  = str_idx("aData");
            let pdata_idx  = str_idx("pData");
            let idata_idx  = str_idx("iData");

            // For every (class, method) that owns a body which pushes
            // "normalStats_id", report the class + method name + whether
            // the body ALSO pushes all four bundle marker keys.
            for class in &abc.classes {
                for trait_ in class.instance_methods.iter().chain(class.class_methods.iter()) {
                    if trait_.kind != 1 && trait_.kind != 2 { continue; } // Method or Getter
                    let Some(body) = abc.method_bodies.iter()
                        .find(|b| b.method_idx == trait_.method_idx)
                    else { continue };
                    if !body_pushes_string(body, nsi_idx) { continue; }

                    let has_bundle = [cdata_idx, adata_idx, pdata_idx, idata_idx].iter()
                        .all(|i| i.map(|x| body_pushes_string(body, x)).unwrap_or(false));

                    let shape = if has_bundle { "BUNDLE" } else { "INLINE" };

                    // Read the literal value of `normalStats_id`.
                    // Pattern in bytecode: pushstring "normalStats_id" then push<some_string>
                    // then setproperty/initproperty 'normalStats_id'. The cheap
                    // discovery: the very next pushstring after "normalStats_id" is
                    // typically the value. Honor a small lookahead.
                    let value = find_next_pushed_string(&abc, body, nsi_idx);

                    println!(
                        "{stem}  class={cls:<28} method={mth:<28} shape={shape}  normalStats_id={val:?}",
                        stem = stem,
                        cls = class.name,
                        mth = trait_.name,
                        shape = shape,
                        val = value.unwrap_or_else(|| "(?)".to_string())
                    );
                }
            }

            // Also scan script-level traits (free functions, not inside a class)
            // because some characters may live as Main-level free methods.
            for script in &abc.scripts {
                for trait_ in &script.traits {
                    if trait_.kind != 1 && trait_.kind != 2 { continue; }
                    let Some(body) = abc.method_bodies.iter()
                        .find(|b| b.method_idx == trait_.method_idx)
                    else { continue };
                    if !body_pushes_string(body, nsi_idx) { continue; }
                    let has_bundle = [cdata_idx, adata_idx, pdata_idx, idata_idx].iter()
                        .all(|i| i.map(|x| body_pushes_string(body, x)).unwrap_or(false));
                    let shape = if has_bundle { "BUNDLE" } else { "INLINE" };
                    let value = find_next_pushed_string(&abc, body, nsi_idx);
                    println!(
                        "{stem}  class=(script)                  method={mth:<28} shape={shape}  normalStats_id={val:?}",
                        stem = stem,
                        mth = trait_.name,
                        shape = shape,
                        val = value.unwrap_or_else(|| "(?)".to_string())
                    );
                }
            }
        }
    }
}

/// In the body's bytecode, find the OP_PUSHSTRING <needle_idx>, then look
/// at the very next OP_PUSHSTRING and return that string's value. Used to
/// extract the literal value of `normalStats_id`.
fn find_next_pushed_string(abc: &abc_parser::AbcFile, body: &abc_parser::MethodBody, needle_idx: u32) -> Option<String> {
    let bytes = &body.bytecode;
    let needle = encode_u30(needle_idx);
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x2c && bytes[i+1..].starts_with(&needle) {
            // Skip past the needle's PUSHSTRING + its u30 operand.
            let mut j = i + 1 + needle.len();
            // Find the next PUSHSTRING.
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

fn read_u30(data: &[u8], i: &mut usize) -> u32 {
    let mut r = 0u32; let mut shift = 0;
    while *i < data.len() {
        let b = data[*i] as u32; *i += 1;
        r |= (b & 0x7f) << shift; shift += 7;
        if b & 0x80 == 0 { break; }
    }
    r
}
