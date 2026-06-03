//! Reverse-engineer the SSF2 engine SWF physics integration.
//!
//! Usage:
//!   engine_physics <SSF2.swf> survey         -- list classes, count, supers
//!   engine_physics <SSF2.swf> strings <pat>  -- grep string pool
//!   engine_physics <SSF2.swf> class <name>   -- list methods of a class
//!   engine_physics <SSF2.swf> findstr <s>    -- which class::method bodies push string <s>
//!   engine_physics <SSF2.swf> decompile <class> <method>  -- decompile one method
//!   engine_physics <SSF2.swf> decompile-all <class>       -- decompile every method on a class
//!   engine_physics <SSF2.swf> grepfield <pat> -- list classes whose method bodies reference any string matching <pat>

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

/// Does the body push string index `idx` via OP_PUSHSTRING (0x2c)?
fn body_pushes_string(body: &abc_parser::MethodBody, idx: u32) -> bool {
    let bytes = &body.bytecode;
    let target = encode_u30(idx);
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == 0x2c && bytes[i + 1..].starts_with(&target) {
            return true;
        }
        i += 1;
    }
    false
}

fn load_abc(path: &str) -> abc_parser::AbcFile {
    let bytes = std::fs::read(path).expect("read swf");
    let swf_bytes = ssf::decompress(&bytes).expect("decompress");
    let swf = swf_parser::parse(&swf_bytes).expect("parse swf");
    // The engine SWF may carry several ABC blocks; merge by picking the one
    // with the most classes (the main game ABC). But to be safe, return the
    // first that parses with classes; callers iterate via a wrapper below.
    let mut best: Option<abc_parser::AbcFile> = None;
    for block in &swf.abc_blocks {
        if let Ok(abc) = abc_parser::parse(block) {
            let better = best.as_ref().map(|b| abc.classes.len() > b.classes.len()).unwrap_or(true);
            if better { best = Some(abc); }
        }
    }
    best.expect("no abc blocks parsed")
}

fn decompile_trait(abc: &abc_parser::AbcFile, class: &str, t: &abc_parser::Trait) -> Option<String> {
    let body = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx)?;
    let pc = abc.methods.get(body.method_idx as usize).map(|m| m.param_count as usize).unwrap_or(0);
    let params: Vec<String> = (0..pc).map(|i| format!("arg{}", i)).collect();
    let qualified = format!("{}::{}", class, t.name);
    Some(decompiler::decompile_method(body, abc, &qualified, &params))
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: engine_physics <swf> <cmd> [args]");
        std::process::exit(1);
    }
    let swf_path = &args[1];
    let cmd = args[2].as_str();
    let abc = load_abc(swf_path);

    match cmd {
        "survey" => {
            println!("classes: {}  methods: {}  bodies: {}  strings: {}",
                abc.classes.len(), abc.methods.len(), abc.method_bodies.len(), abc.strings.len());
            let mut names: Vec<_> = abc.classes.iter()
                .map(|c| (c.name.clone(), c.super_name.clone(), c.instance_methods.len()))
                .collect();
            names.sort();
            for (n, s, m) in names {
                println!("  {:<40} : {:<28} ({} methods)", n, s, m);
            }
        }
        "strings" => {
            let pat = args[3].to_lowercase();
            let mut hits: Vec<&String> = abc.strings.iter()
                .filter(|s| s.to_lowercase().contains(&pat)).collect();
            hits.sort();
            hits.dedup();
            for s in hits { println!("{}", s); }
        }
        "class" => {
            let cname = &args[3];
            let Some(c) = abc.classes.iter().find(|c| &c.name == cname) else {
                eprintln!("no class {}", cname); return;
            };
            println!("class {} extends {}", c.name, c.super_name);
            println!("-- instance methods --");
            for t in &c.instance_methods {
                let kind = match t.kind { 1 => "method", 2 => "getter", 3 => "setter", _ => "slot/other" };
                println!("  [{}] {} (findex {})", kind, t.name, t.method_idx);
            }
            println!("-- class (static) methods --");
            for t in &c.class_methods {
                let kind = match t.kind { 1 => "method", 2 => "getter", 3 => "setter", _ => "slot/other" };
                println!("  [{}] {} (findex {})", kind, t.name, t.method_idx);
            }
        }
        "findstr" => {
            let needle = &args[3];
            let Some(idx) = abc.strings.iter().position(|s| s == needle) else {
                eprintln!("string {:?} not in pool", needle); return;
            };
            let idx = idx as u32;
            for c in &abc.classes {
                for t in c.instance_methods.iter().chain(c.class_methods.iter()) {
                    if let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) {
                        if body_pushes_string(body, idx) {
                            println!("{}::{} (findex {})", c.name, t.name, t.method_idx);
                        }
                    }
                }
            }
        }
        "grepfield" => {
            // For each string matching pat, list classes referencing it.
            let pat = args[3].to_lowercase();
            let matching: Vec<(u32, String)> = abc.strings.iter().enumerate()
                .filter(|(_, s)| s.to_lowercase().contains(&pat))
                .map(|(i, s)| (i as u32, s.clone())).collect();
            for c in &abc.classes {
                let mut hit_strs = std::collections::BTreeSet::new();
                for t in c.instance_methods.iter().chain(c.class_methods.iter()) {
                    if let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) {
                        for (idx, s) in &matching {
                            if body_pushes_string(body, *idx) { hit_strs.insert(s.clone()); }
                        }
                    }
                }
                if !hit_strs.is_empty() {
                    println!("{:<36} {:?}", c.name, hit_strs);
                }
            }
        }
        "decompile" => {
            let cname = &args[3];
            let mname = &args[4];
            let Some(c) = abc.classes.iter().find(|c| &c.name == cname) else {
                eprintln!("no class {}", cname); return;
            };
            for t in c.instance_methods.iter().chain(c.class_methods.iter()) {
                if &t.name == mname {
                    println!("// {}::{} findex={}", cname, t.name, t.method_idx);
                    if let Some(src) = decompile_trait(&abc, cname, t) { println!("{}", src); }
                }
            }
        }
        "decompile-all" => {
            let cname = &args[3];
            let Some(c) = abc.classes.iter().find(|c| &c.name == cname) else {
                eprintln!("no class {}", cname); return;
            };
            for t in c.instance_methods.iter().chain(c.class_methods.iter()) {
                if t.kind != 1 && t.kind != 2 && t.kind != 3 { continue; }
                println!("// ===== {}::{} findex={} =====", cname, t.name, t.method_idx);
                if let Some(src) = decompile_trait(&abc, cname, t) { println!("{}", src); }
            }
        }
        "disasm" => {
            let cname = &args[3];
            let mname = &args[4];
            let Some(c) = abc.classes.iter().find(|c| &c.name == cname) else {
                eprintln!("no class {}", cname); return;
            };
            for t in c.instance_methods.iter().chain(c.class_methods.iter()) {
                if &t.name == mname {
                    if let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) {
                        println!("// disasm {}::{} findex={}", cname, t.name, t.method_idx);
                        disasm(&abc, &body.bytecode);
                    }
                }
            }
        }
        _ => eprintln!("unknown cmd {}", cmd),
    }
}

fn rd_u30(b: &[u8], i: &mut usize) -> u32 {
    let mut r = 0u32; let mut s = 0;
    while *i < b.len() { let x = b[*i] as u32; *i += 1; r |= (x & 0x7f) << s; s += 7; if x & 0x80 == 0 { break; } }
    r
}
fn rd_s24(b: &[u8], i: &mut usize) -> i32 {
    let v = (b[*i] as i32) | ((b[*i+1] as i32) << 8) | ((b[*i+2] as i32) << 16);
    *i += 3;
    if v & 0x800000 != 0 { v - 0x1000000 } else { v }
}

/// Minimal AVM2 disassembler covering ops relevant to physics arithmetic.
fn disasm(abc: &abc_parser::AbcFile, code: &[u8]) {
    let mn = |i: u32| -> String {
        abc.multinames.get(i as usize).map(|m| m.name.clone()).unwrap_or_else(|| format!("mn#{}", i))
    };
    let mut i = 0usize;
    while i < code.len() {
        let off = i;
        let op = code[i]; i += 1;
        let mut text = String::new();
        match op {
            0x24 => { let v = code[i] as i8; i += 1; text = format!("pushbyte {}", v); }
            0x25 => { let v = rd_u30(code, &mut i); text = format!("pushshort {}", v); }
            0x26 => text = "pushtrue".into(),
            0x27 => text = "pushfalse".into(),
            0x28 => text = "pushnan".into(),
            0x2c => { let v = rd_u30(code, &mut i); text = format!("pushstring {:?}", abc.strings.get(v as usize)); }
            0x2d => { let v = rd_u30(code, &mut i); text = format!("pushint {:?}", abc.ints.get(v as usize)); }
            0x2e => { let v = rd_u30(code, &mut i); text = format!("pushuint {:?}", abc.uints.get(v as usize)); }
            0x2f => { let v = rd_u30(code, &mut i); text = format!("pushdouble {:?}", abc.doubles.get(v as usize)); }
            0x30 => text = "pushscope".into(),
            0x20 => text = "pushnull".into(),
            0x21 => text = "pushundefined".into(),
            0xd0 => text = "getlocal_0".into(),
            0xd1 => text = "getlocal_1".into(),
            0xd2 => text = "getlocal_2".into(),
            0xd3 => text = "getlocal_3".into(),
            0x62 => { let v = rd_u30(code, &mut i); text = format!("getlocal {}", v); }
            0xd4 => text = "setlocal_0".into(),
            0xd5 => text = "setlocal_1".into(),
            0xd6 => text = "setlocal_2".into(),
            0xd7 => text = "setlocal_3".into(),
            0x63 => { let v = rd_u30(code, &mut i); text = format!("setlocal {}", v); }
            0x60 => { let v = rd_u30(code, &mut i); text = format!("getlex {}", mn(v)); }
            0x66 => { let v = rd_u30(code, &mut i); text = format!("getproperty {}", mn(v)); }
            0x61 => { let v = rd_u30(code, &mut i); text = format!("setproperty {}", mn(v)); }
            0x68 => { let v = rd_u30(code, &mut i); text = format!("initproperty {}", mn(v)); }
            0x4a => { let v = rd_u30(code, &mut i); text = format!("constructprop {}", mn(v)); }
            0x46 => { let m = rd_u30(code, &mut i); let n = rd_u30(code, &mut i); text = format!("callproperty {} ({} args)", mn(m), n); }
            0x4f => { let m = rd_u30(code, &mut i); let n = rd_u30(code, &mut i); text = format!("callpropvoid {} ({} args)", mn(m), n); }
            0x45 => { let m = rd_u30(code, &mut i); let n = rd_u30(code, &mut i); text = format!("callsuper {} ({} args)", mn(m), n); }
            0x47 => text = "returnvoid".into(),
            0x48 => text = "returnvalue".into(),
            0xa0 => text = "add".into(),
            0xa1 => text = "subtract".into(),
            0xa2 => text = "multiply".into(),
            0xa3 => text = "divide".into(),
            0xa4 => text = "modulo".into(),
            0x90 => text = "negate".into(),
            0xc0 => text = "increment_i".into(),
            0x91 => text = "increment".into(),
            0x93 => text = "decrement".into(),
            0x96 => text = "not".into(),
            0xab => text = "equals".into(),
            0xad => text = "lessthan".into(),
            0xae => text = "lessequals".into(),
            0xaf => text = "greaterthan".into(),
            0xb0 => text = "greaterequals".into(),
            0x12 => { let t = rd_s24(code, &mut i); text = format!("iffalse -> {}", (i as i32 + t)); }
            0x13 => { let t = rd_s24(code, &mut i); text = format!("iftrue -> {}", (i as i32 + t)); }
            0x14 => { let t = rd_s24(code, &mut i); text = format!("ifne -> {}", (i as i32 + t)); }
            0x15 => { let t = rd_s24(code, &mut i); text = format!("iflt -> {}", (i as i32 + t)); }
            0x16 => { let t = rd_s24(code, &mut i); text = format!("ifle -> {}", (i as i32 + t)); }
            0x17 => { let t = rd_s24(code, &mut i); text = format!("ifgt -> {}", (i as i32 + t)); }
            0x18 => { let t = rd_s24(code, &mut i); text = format!("ifge -> {}", (i as i32 + t)); }
            0x19 => { let t = rd_s24(code, &mut i); text = format!("ifstricteq -> {}", (i as i32 + t)); }
            0x1a => { let t = rd_s24(code, &mut i); text = format!("ifstrictne -> {}", (i as i32 + t)); }
            0x0c => { let t = rd_s24(code, &mut i); text = format!("ifnlt -> {}", (i as i32 + t)); }
            0x0d => { let t = rd_s24(code, &mut i); text = format!("ifnle -> {}", (i as i32 + t)); }
            0x0e => { let t = rd_s24(code, &mut i); text = format!("ifngt -> {}", (i as i32 + t)); }
            0x0f => { let t = rd_s24(code, &mut i); text = format!("ifnge -> {}", (i as i32 + t)); }
            0x10 => { let t = rd_s24(code, &mut i); text = format!("jump -> {}", (i as i32 + t)); }
            0x11 => { let t = rd_s24(code, &mut i); text = format!("iftrue2 -> {}", (i as i32 + t)); }
            0x73 => text = "convert_i".into(),
            0x75 => text = "convert_d".into(),
            0x76 => text = "convert_b".into(),
            0x70 => text = "convert_s".into(),
            0x29 => text = "pop".into(),
            0x2a => text = "dup".into(),
            0x2b => text = "swap".into(),
            0x65 => { let v = rd_u30(code, &mut i); text = format!("getscopeobject {}", v); }
            0x6c => { let v = rd_u30(code, &mut i); text = format!("getslot {}", v); }
            0x6d => { let v = rd_u30(code, &mut i); text = format!("setslot {}", v); }
            0x80 => { let v = rd_u30(code, &mut i); text = format!("coerce {}", mn(v)); }
            0x32 => { let a = rd_u30(code, &mut i); let b = rd_u30(code, &mut i); text = format!("hasnext2 {},{}", a, b); }
            0x09 => text = "label".into(),
            0xef => { i += 8; text = "debug".into(); }
            0xf0 => { rd_u30(code, &mut i); text = "debugline".into(); }
            0xf1 => { rd_u30(code, &mut i); text = "debugfile".into(); }
            _ => { text = format!("op 0x{:02x}", op); }
        }
        println!("{:>5}: {}", off, text);
    }
}
