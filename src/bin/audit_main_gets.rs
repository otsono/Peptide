//! Across every .ssf, enumerate ALL methods on the `Main` class (instance
//! AND static) whose name starts with `get`. For each, classify whether
//! the body is a "character bundle" (pushes cData + aData + pData + iData
//! + normalStats_id) or something else.
//!
//! Output is one row per (ssf, method_name, kind, shape).

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

fn main() {
    let ssfs_dir = env::args().nth(1).unwrap_or_else(||
        "/Users/jimmy/.openclaw/workspace-main/ssf2-ssfs".to_string());

    let mut files: Vec<_> = std::fs::read_dir(&ssfs_dir).unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "ssf").unwrap_or(false))
        .collect();
    files.sort();

    let mut bundle_count = 0;
    let mut nonbundle_count = 0;
    let mut nomain_count = 0;
    let mut all_bundle_names: Vec<String> = Vec::new();
    let mut all_nonbundle_rows: Vec<String> = Vec::new();

    for path in &files {
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let Ok(bytes) = std::fs::read(path) else { continue };
        let Ok(swf_bytes) = ssf::decompress(&bytes) else { continue };
        let Ok(swf) = swf_parser::parse(&swf_bytes) else { continue };

        let mut found_main = false;
        for abc_bytes in &swf.abc_blocks {
            let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };
            let Some(main) = abc.classes.iter().find(|c| c.name == "Main") else { continue };
            found_main = true;

            let str_idx = |s: &str| -> Option<u32> {
                abc.strings.iter().position(|x| x == s).map(|i| i as u32)
            };
            let nsi = str_idx("normalStats_id");
            let cd  = str_idx("cData");
            let ad  = str_idx("aData");
            let pd  = str_idx("pData");
            let id  = str_idx("iData");

            let bundle_keys_present = nsi.is_some() && cd.is_some()
                && ad.is_some() && pd.is_some() && id.is_some();

            // Classify each get* trait on Main (instance + static)
            let pairs: Vec<(&str, &abc_parser::Trait)> = main.instance_methods.iter()
                .map(|t| ("inst", t))
                .chain(main.class_methods.iter().map(|t| ("stat", t)))
                .collect();

            for (kind, t) in pairs {
                if !t.name.starts_with("get") { continue; }
                let Some(body) = abc.method_bodies.iter()
                    .find(|b| b.method_idx == t.method_idx) else {
                    all_nonbundle_rows.push(format!(
                        "{:<18} {:<5} {:<28} NO_BODY", stem, kind, t.name));
                    nonbundle_count += 1;
                    continue;
                };
                let is_bundle = bundle_keys_present
                    && body_pushes_string(body, nsi.unwrap())
                    && body_pushes_string(body, cd.unwrap())
                    && body_pushes_string(body, ad.unwrap())
                    && body_pushes_string(body, pd.unwrap())
                    && body_pushes_string(body, id.unwrap());
                if is_bundle {
                    all_bundle_names.push(format!("{}:{}", stem, t.name));
                    bundle_count += 1;
                } else {
                    let bytecode_len = body.bytecode.len();
                    all_nonbundle_rows.push(format!(
                        "{:<18} {:<5} {:<28} NON_BUNDLE bytes={}",
                        stem, kind, t.name, bytecode_len));
                    nonbundle_count += 1;
                }
            }
        }
        if !found_main { nomain_count += 1; println!("{:<18} (no Main class)", stem); }
    }

    println!();
    println!("=== Summary ===");
    println!("SSFs without Main: {}", nomain_count);
    println!("BUNDLE get* methods: {}", bundle_count);
    println!("NON-BUNDLE get* methods: {}", nonbundle_count);

    println!();
    println!("=== Non-bundle get* methods on Main ===");
    for row in &all_nonbundle_rows { println!("  {}", row); }

    println!();
    println!("=== Every bundle method (ssf:method) ===");
    for row in &all_bundle_names { println!("  {}", row); }
}
