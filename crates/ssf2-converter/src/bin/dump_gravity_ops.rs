//! Dump the raw opcode sequence surrounding every "gravity" (and other
//! stat-key) pushstring in a character's Main::get<X>() bundle method.
//! Used to diagnose why extract_ssf2_stats reads gravity=0 for some chars.
//!
//! Usage: dump_gravity_ops <swf.ssf> [statKey] [charName]

use ssf2_converter::*;
use std::env;

// AVM2 opcode names (subset, enough to read stat-push sequences).
fn op_name(op: u8) -> &'static str {
    match op {
        0x24 => "pushbyte",
        0x25 => "pushshort",
        0x2D => "pushint",
        0x2E => "pushuint",
        0x2F => "pushdouble",
        0x2C => "pushstring",
        0x20 => "pushnull",
        0x21 => "pushundefined",
        0x26 => "pushtrue",
        0x27 => "pushfalse",
        0x28 => "pushnan",
        0x30 => "pushscope",
        0x60 => "getlex",
        0x80 => "coerce",
        0x82 => "coerce_a",
        0x85 => "coerce_s",
        0x73 => "convert_i",
        0x75 => "convert_d",
        0x90 => "negate",
        0xA0 => "add",
        0xA1 => "subtract",
        0xA2 => "multiply",
        0xA3 => "divide",
        0x5D => "findpropstrict",
        0x5E => "findproperty",
        0x66 => "getproperty",
        0x68 => "initproperty",
        0x61 => "setproperty",
        0x46 => "callproperty",
        0x4F => "callpropvoid",
        0x4A => "constructprop",
        0x42 => "construct",
        0x56 => "newarray",
        0x55 => "newobject",
        0x62 => "getlocal",
        0x63 => "setlocal",
        0xD0 => "getlocal0",
        0xD1 => "getlocal1",
        0xD2 => "getlocal2",
        0xD3 => "getlocal3",
        0x10 => "jump",
        0x11 => "iftrue",
        0x12 => "iffalse",
        0x47 => "returnvoid",
        0x48 => "returnvalue",
        0x29 => "pop",
        0x2A => "dup",
        _ => "?",
    }
}

fn read_u30(data: &[u8], i: &mut usize) -> u32 {
    let mut result = 0u32;
    let mut shift = 0;
    loop {
        if *i >= data.len() { break; }
        let b = data[*i] as u32; *i += 1;
        result |= (b & 0x7F) << shift; shift += 7;
        if b & 0x80 == 0 || shift >= 35 { break; }
    }
    result
}

/// Decode one instruction at `i`, return (mnemonic_with_operands, next_i).
fn decode(bc: &[u8], abc: &abc_parser::AbcFile, mut i: usize) -> (String, usize) {
    let op = bc[i]; i += 1;
    let name = op_name(op);
    let s = match op {
        0x2C => { let idx = read_u30(bc, &mut i); // pushstring
            format!("pushstring \"{}\"", abc.strings.get(idx as usize).cloned().unwrap_or_default()) }
        0x2F => { let idx = read_u30(bc, &mut i); // pushdouble
            format!("pushdouble [{}] = {}", idx, abc.doubles.get(idx as usize).copied().unwrap_or(f64::NAN)) }
        0x2D => { let idx = read_u30(bc, &mut i);
            format!("pushint [{}] = {}", idx, abc.ints.get(idx as usize).copied().unwrap_or(0)) }
        0x2E => { let idx = read_u30(bc, &mut i);
            format!("pushuint [{}] = {}", idx, abc.uints.get(idx as usize).copied().unwrap_or(0)) }
        0x24 => { let v = bc[i] as i8; i += 1; format!("pushbyte {}", v) }
        0x25 => { let v = read_u30(bc, &mut i) as i16; format!("pushshort {}", v) }
        0x60 | 0x5D | 0x5E | 0x66 | 0x68 | 0x61 | 0x80 => {
            let idx = read_u30(bc, &mut i); format!("{} mn[{}]", name, idx) }
        0x46 | 0x4F | 0x4A => { let m = read_u30(bc, &mut i); let n = read_u30(bc, &mut i);
            format!("{} mn[{}], {}", name, m, n) }
        0x42 | 0x56 | 0x55 => { let n = read_u30(bc, &mut i); format!("{} {}", name, n) }
        0x62 | 0x63 => { let n = read_u30(bc, &mut i); format!("{} {}", name, n) }
        0x10 | 0x11 | 0x12 => { i += 3; format!("{} <s24>", name) }
        _ => name.to_string(),
    };
    (s, i)
}

fn main() {
    let path = env::args().nth(1).expect("usage: dump_gravity_ops <swf> [statKey] [charName]");
    let stat_key = env::args().nth(2).unwrap_or_else(|| "gravity".to_string());
    let char_override = env::args().nth(3);

    let bytes = std::fs::read(&path).expect("read");
    let swf_bytes = ssf::decompress(&bytes).expect("decompress");
    let swf = swf_parser::parse(&swf_bytes).expect("parse");

    // char name from filename stem unless overridden
    let char_name = char_override.unwrap_or_else(|| {
        path.rsplit('/').next().unwrap_or("").trim_end_matches(".ssf").to_lowercase()
    });

    for abc_bytes in &swf.abc_blocks {
        let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };
        let Some((body, mname)) = abc_parser::find_bundle_method(&abc, &char_name) else { continue };
        println!("=== {} :: Main::{} ({} bytes) — surrounding '{}' ===",
            char_name, mname, body.bytecode.len(), stat_key);

        let bc = &body.bytecode;
        // Pre-decode the whole body into (offset, text) so we can show context.
        let mut offsets = Vec::new();
        let mut i = 0;
        while i < bc.len() {
            let start = i;
            let (txt, ni) = decode(bc, &abc, i);
            offsets.push((start, txt));
            if ni <= i { break; }
            i = ni;
        }

        for (idx, (off, txt)) in offsets.iter().enumerate() {
            if txt == &format!("pushstring \"{}\"", stat_key) {
                println!("\n--- occurrence at byte {} ---", off);
                let lo = idx.saturating_sub(3);
                let hi = (idx + 6).min(offsets.len());
                for k in lo..hi {
                    let marker = if k == idx { ">>" } else { "  " };
                    println!("  {} {:5}: {}", marker, offsets[k].0, offsets[k].1);
                }
            }
        }
        return;
    }
    eprintln!("no bundle method found for '{}'", char_name);
}
