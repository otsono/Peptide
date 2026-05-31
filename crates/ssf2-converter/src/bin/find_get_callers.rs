//! For a given .ssf, find every CALLPROPERTY / CALLPROPVOID / GETPROPERTY
//! / FINDPROPSTRICT instruction whose multiname name starts with `get`.
//! Report each call site: the class + method that contains it, the called
//! `get*` name, the opcode, and the decompiled body of the calling method
//! (truncated). Also dumps Main's complete trait list (instance + class +
//! constructor) for context.

use ssf2_converter::*;
use std::env;

const OP_CALLPROPERTY:  u8 = 0x46;
const OP_CALLPROPVOID:  u8 = 0x4F;
const OP_GETPROPERTY:   u8 = 0x66;
const OP_FINDPROPSTRICT:u8 = 0x5D;
const OP_CONSTRUCTPROP: u8 = 0x4A;
const OP_PUSHSTRING:    u8 = 0x2C;
const OP_PUSHBYTE:      u8 = 0x24;
const OP_PUSHSHORT:     u8 = 0x25;
const OP_PUSHINT:       u8 = 0x2D;
const OP_PUSHUINT:      u8 = 0x2E;
const OP_PUSHDOUBLE:    u8 = 0x2F;
const OP_COERCE:        u8 = 0x80;
const OP_GETLEX:        u8 = 0x60;
const OP_FINDPROP:      u8 = 0x5E;
const OP_INITPROPERTY:  u8 = 0x68;
const OP_SETPROPERTY:   u8 = 0x61;
const OP_GETLOCAL:      u8 = 0x62;
const OP_SETLOCAL:      u8 = 0x63;
const OP_NEWARRAY:      u8 = 0x56;
const OP_NEWOBJECT:     u8 = 0x55;
const OP_JUMP:          u8 = 0x10;
const OP_IFTRUE:        u8 = 0x11;
const OP_IFFALSE:       u8 = 0x12;
const OP_IFEQ:          u8 = 0x13;
const OP_IFNE:          u8 = 0x14;
const OP_IFLT:          u8 = 0x15;
const OP_IFLE:          u8 = 0x16;
const OP_IFGT:          u8 = 0x17;
const OP_IFGE:          u8 = 0x18;
const OP_IFSTRICTEQ:    u8 = 0x19;
const OP_IFSTRICTNE:    u8 = 0x1A;

fn read_u30(data: &[u8], i: &mut usize) -> Option<u32> {
    let mut r = 0u32; let mut shift = 0;
    while *i < data.len() {
        let b = data[*i] as u32; *i += 1;
        r |= (b & 0x7f) << shift; shift += 7;
        if b & 0x80 == 0 || shift >= 35 { break; }
    }
    Some(r)
}

fn skip_opcode_operands(op: u8, bc: &[u8], i: &mut usize) {
    match op {
        OP_PUSHDOUBLE | OP_PUSHSTRING | OP_PUSHINT | OP_PUSHUINT |
        OP_COERCE | OP_GETLEX | OP_FINDPROPSTRICT | OP_FINDPROP |
        OP_GETPROPERTY | OP_INITPROPERTY | OP_SETPROPERTY |
        OP_GETLOCAL | OP_SETLOCAL => { read_u30(bc, i); }
        OP_PUSHBYTE => { if *i < bc.len() { *i += 1; } }
        OP_PUSHSHORT => { read_u30(bc, i); }
        OP_CALLPROPERTY | OP_CALLPROPVOID | OP_CONSTRUCTPROP => {
            read_u30(bc, i); read_u30(bc, i);
        }
        OP_NEWARRAY | OP_NEWOBJECT => { read_u30(bc, i); }
        OP_JUMP | OP_IFTRUE | OP_IFFALSE | OP_IFEQ | OP_IFNE |
        OP_IFLT | OP_IFLE | OP_IFGT | OP_IFGE |
        OP_IFSTRICTEQ | OP_IFSTRICTNE => {
            if *i + 3 <= bc.len() { *i += 3; }
        }
        _ => {}
    }
}

fn op_name(op: u8) -> &'static str {
    match op {
        OP_CALLPROPERTY => "callproperty",
        OP_CALLPROPVOID => "callpropvoid",
        OP_GETPROPERTY  => "getproperty",
        OP_FINDPROPSTRICT => "findpropstrict",
        OP_CONSTRUCTPROP => "constructprop",
        _ => "?",
    }
}

fn scan_calls<'a>(bc: &[u8], abc: &'a abc_parser::AbcFile)
    -> Vec<(u8, &'a str)>
{
    let mut out = Vec::new();
    let mut i = 0;
    while i < bc.len() {
        let op = bc[i]; i += 1;
        if matches!(op, OP_CALLPROPERTY | OP_CALLPROPVOID | OP_GETPROPERTY
                       | OP_FINDPROPSTRICT | OP_CONSTRUCTPROP)
        {
            let mut j = i;
            let mn_idx = read_u30(bc, &mut j).unwrap_or(0);
            let name = abc.multinames.get(mn_idx as usize)
                .map(|m| m.name.as_str()).unwrap_or("?");
            out.push((op, name));
            // consume the rest of this opcode's operands
            i -= 1;
            skip_opcode_operands(op, bc, &mut i);
        } else {
            i -= 1;
            skip_opcode_operands(op, bc, &mut i);
            i += 1;  // skip_opcode_operands handles operands but we need to bump past the opcode itself
                     // for opcodes with no operands
            // Actually we double-stepped. Let me redo:
        }
    }
    out
}

// The skip routine above is fiddly. Take a simpler version: a one-pass
// scanner that walks opcode+operands cleanly.
fn scan_calls_simple<'a>(bc: &[u8], abc: &'a abc_parser::AbcFile)
    -> Vec<(u8, &'a str)>
{
    let mut out = Vec::new();
    let mut i = 0;
    while i < bc.len() {
        let op = bc[i];
        let op_pos = i;
        i += 1;
        if matches!(op, OP_CALLPROPERTY | OP_CALLPROPVOID | OP_GETPROPERTY
                       | OP_FINDPROPSTRICT | OP_CONSTRUCTPROP)
        {
            let mut j = i;
            let mn_idx = read_u30(bc, &mut j).unwrap_or(0);
            let name = abc.multinames.get(mn_idx as usize)
                .map(|m| m.name.as_str()).unwrap_or("?");
            out.push((op, name));
            // advance i past the operands of this opcode
            i = op_pos + 1;
            skip_opcode_operands(op, bc, &mut i);
        } else {
            skip_opcode_operands(op, bc, &mut i);
        }
    }
    out
}

fn main() {
    let path = env::args().nth(1).unwrap_or_else(||
        "../ssf2-ssfs/zelda.ssf".to_string());

    let bytes = std::fs::read(&path).expect("read");
    let swf_bytes = ssf::decompress(&bytes).expect("decompress");
    let swf = swf_parser::parse(&swf_bytes).expect("parse");

    for abc_bytes in &swf.abc_blocks {
        let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };

        // 1. Dump Main's full trait list.
        if let Some(main) = abc.classes.iter().find(|c| c.name == "Main") {
            println!("=== Main class — instance methods ===");
            for t in &main.instance_methods {
                println!("  kind={}  method_idx={:<6}  slot_idx={}  name={}",
                    t.kind, t.method_idx, t.slot_idx, t.name);
            }
            println!("=== Main class — class (static) methods ===");
            for t in &main.class_methods {
                println!("  kind={}  method_idx={:<6}  slot_idx={}  name={}",
                    t.kind, t.method_idx, t.slot_idx, t.name);
            }
            println!("=== Main class — constructor_idx={} ===", main.constructor_idx);
        } else {
            println!("(no Main class)");
        }
        println!();

        // 2. Find every CALLPROPERTY/CALLPROPVOID/etc. site whose
        //    multiname name starts with "get" — captures get* calls
        //    regardless of whether the receiver is `Main` or something else.
        // 3. Specifically highlight the bundle-method names.
        let bundle_method_names: Vec<&str> = abc.classes.iter()
            .find(|c| c.name == "Main")
            .map(|m| m.instance_methods.iter()
                .filter(|t| t.name.starts_with("get"))
                .map(|t| t.name.as_str())
                .collect())
            .unwrap_or_default();
        println!("Bundle method names to search for: {:?}", bundle_method_names);
        println!();

        println!("=== All call sites of any get* method (any class / method body) ===");
        // method_idx → (class_name, method_name) ownership map for nice context.
        let mut owner_of: std::collections::BTreeMap<u32, (String, String)> = Default::default();
        for c in &abc.classes {
            for t in c.instance_methods.iter().chain(c.class_methods.iter()) {
                owner_of.entry(t.method_idx).or_insert_with(||
                    (c.name.clone(), t.name.clone()));
            }
        }
        for s in &abc.scripts {
            for t in &s.traits {
                owner_of.entry(t.method_idx).or_insert_with(||
                    ("(script)".to_string(), t.name.clone()));
            }
        }
        // Iterate every body, find each get* call site
        for body in &abc.method_bodies {
            let calls = scan_calls_simple(&body.bytecode, &abc);
            for (op, name) in calls {
                if !name.starts_with("get") || name.len() <= 3 { continue; }
                // Only interesting if the call name matches one of Main's bundle methods
                // (a generic getX could mean anything otherwise).
                if !bundle_method_names.contains(&name) { continue; }
                let (cls, mth) = owner_of.get(&body.method_idx)
                    .cloned().unwrap_or_else(||
                        ("(unknown)".to_string(), format!("method_idx={}", body.method_idx)));
                println!("  {} → {}::{}    (in {}::{})",
                    op_name(op), "Main(?)", name, cls, mth);
            }
        }
    }
}
