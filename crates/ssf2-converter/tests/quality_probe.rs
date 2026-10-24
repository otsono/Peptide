//! One-off probe: every alpha the stage's own classes assign at runtime.
use std::path::PathBuf;

#[test]
#[ignore]
fn dump_runtime_alpha_writes() {
    let dir = std::env::var("SSF2_SSFS_DIR").unwrap_or("../ssf2-ssfs".into());
    let p = PathBuf::from(dir).join("stages/clocktown.ssf");
    if !p.exists() { eprintln!("corpus absent; skipping"); return; }
    let data = std::fs::read(&p).expect("read");
    let raw = ssf2_converter::ssf::decompress(&data).unwrap_or(data);
    let swf = ssf2_converter::swf_parser::parse(&raw).expect("parse");
    for blk in &swf.abc_blocks {
        let Ok(abc) = ssf2_converter::abc_parser::parse(blk) else { continue };
        for c in abc.classes.iter() {
            for t in c.instance_methods.iter().chain(c.class_methods.iter()) {
                let mi = t.method_idx;
                let Some(b) = abc.method_bodies.iter().find(|b| b.method_idx == mi) else { continue };
                let m = &abc.methods[mi as usize];
                let params: Vec<String> = (0..m.param_count).map(|i| format!("arg{i}")).collect();
                let src = ssf2_converter::decompiler::decompile_method(b, &abc, &t.name, &params);
                for line in src.lines() {
                    let l = line.trim();
                    if l.contains("alpha") && (l.contains('=') || l.contains("setAlpha")) {
                        eprintln!("ALPHA [{}::{}] {}", c.name, t.name, l);
                    }
                }
            }
        }
    }
}
