//! For a given .ssf, decompile <X>Ext::get{Own,Attack,Item,Projectile}Stats
//! and Main::get<X>(), and write both to disk so we can diff them by hand
//! / by `diff`. Used to test whether the BUNDLE source is field-by-field
//! equivalent to the INLINE source.

use ssf2_converter::*;
use std::env;
use std::io::Write;

fn main() {
    let path = env::args().nth(1).expect("usage: diff_inline_vs_bundle <file.ssf>");
    let ext_class_arg = env::args().nth(2).expect("usage: ... <ExtClassName>");
    let main_method_arg = env::args().nth(3).expect("usage: ... <MainMethodName>");
    let out_prefix = env::args().nth(4).unwrap_or_else(|| "out".to_string());

    let bytes = std::fs::read(&path).expect("read");
    let swf_bytes = ssf::decompress(&bytes).expect("decompress");
    let swf = swf_parser::parse(&swf_bytes).expect("parse");

    for abc_bytes in &swf.abc_blocks {
        let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };

        let dump = |class_name: &str, method_name: &str, out_path: &str| {
            let class = match abc.classes.iter().find(|c| c.name == class_name) {
                Some(c) => c,
                None => { eprintln!("no class {}", class_name); return; }
            };
            let trait_ = match class.instance_methods.iter()
                .chain(class.class_methods.iter())
                .find(|t| t.name == method_name)
            {
                Some(t) => t,
                None => { eprintln!("no method {} on {}", method_name, class_name); return; }
            };
            let body = match abc.method_bodies.iter()
                .find(|b| b.method_idx == trait_.method_idx)
            {
                Some(b) => b,
                None => { eprintln!("no body for method_idx {}", trait_.method_idx); return; }
            };
            let pc = abc.methods.get(body.method_idx as usize)
                .map(|m| m.param_count as usize).unwrap_or(0);
            let params: Vec<String> = (0..pc).map(|i| format!("arg{}", i)).collect();
            let d = decompiler::decompile_method(body, &abc, method_name, &params);
            let mut f = std::fs::File::create(out_path).expect("create");
            f.write_all(d.as_bytes()).expect("write");
            eprintln!("wrote {}", out_path);
        };

        for canon in &["getOwnStats", "getAttackStats", "getItemStats", "getProjectileStats"] {
            dump(&ext_class_arg, canon, &format!("{}.{}.inline.hx", out_prefix, canon));
        }
        dump("Main", &main_method_arg, &format!("{}.bundle.hx", out_prefix));
    }
}
