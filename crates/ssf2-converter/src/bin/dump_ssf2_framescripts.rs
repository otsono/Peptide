//! Dump raw decompiled SSF2 frame scripts (pre-translation) for one or more
//! SSF2 animation labels. Usage:
//!   dump_ssf2_framescripts <ssf2_anim_label> [more labels...]
//! Prints each `<label>__frameN` method's decompiled SSF2 body verbatim, so it
//! can be placed side-by-side against the converted FM entity frame scripts.

use ssf2_converter::*;
use std::env;

fn main() {
    let raw = std::fs::read("../ssf2-ssfs/sandbag.ssf").unwrap();
    let swf_data = ssf2_converter::ssf::decompress(&raw).unwrap();
    let swf = swf_parser::parse(&swf_data).unwrap();
    let data = extractor::extract(&swf, "sandbag").unwrap();

    let labels: Vec<String> = env::args().skip(1).collect();

    for label in &labels {
        let prefix = format!("{}__frame", label);
        println!("\n######## SSF2 frame scripts for label: {} ########", label);
        // collect frame indices
        let mut entries: Vec<(u32, &str)> = Vec::new();
        for s in &data.scripts {
            if s.is_ext_method { continue; }
            if let Some(rest) = s.name.strip_prefix(&prefix) {
                if let Ok(n) = rest.parse::<u32>() {
                    entries.push((n, s.code.as_str()));
                }
            }
        }
        entries.sort_by_key(|e| e.0);
        if entries.is_empty() {
            println!("  (no frame scripts found with prefix {})", prefix);
        }
        for (n, code) in entries {
            println!("---- {}__frame{} ----", label, n);
            println!("{}", code);
        }
    }
}
