//! Decompile Main's constructor (iinit) for a given .ssf and print it
//! verbatim. The earlier probe (find_get_callers) showed that Main's
//! constructor is the sole call site for Main::get<X>() — so the iinit
//! body is the SSF2-side dispatcher we want to understand.

use ssf2_converter::*;
use std::env;

fn main() {
    let path = env::args().nth(1).unwrap_or_else(||
        "../ssf2-ssfs/zelda.ssf".to_string());

    let bytes = std::fs::read(&path).expect("read");
    let swf_bytes = ssf::decompress(&bytes).expect("decompress");
    let swf = swf_parser::parse(&swf_bytes).expect("parse");

    for abc_bytes in &swf.abc_blocks {
        let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };
        let Some(main) = abc.classes.iter().find(|c| c.name == "Main") else { continue };
        let Some(body) = abc.method_bodies.iter()
            .find(|b| b.method_idx == main.constructor_idx) else { continue };

        let pc = abc.methods.get(body.method_idx as usize)
            .map(|m| m.param_count as usize).unwrap_or(0);
        let params: Vec<String> = (0..pc).map(|i| format!("arg{}", i)).collect();

        println!("=== {} :: Main constructor (method_idx={}, bytecode {} bytes) ===",
            path.rsplit('/').next().unwrap_or(""), main.constructor_idx, body.bytecode.len());
        let d = decompiler::decompile_method(body, &abc, "Main", &params);
        println!("{}", d);
    }
}
