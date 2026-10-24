//! A method's bytecode as an instruction listing: every opcode, its operands with names and
//! constants resolved from the pool, and each branch's target as an address. What
//! `peptide ssf2 decompile <Class> <method> --asm` prints, for when the decompiler leaves a
//! branch out (`/* ? */`, an empty block) and the code itself has to be read.

use crate::abc_parser::AbcFile;

/// how an opcode's operands are laid out after it
#[derive(Clone, Copy)]
enum Ops {
    None,
    U8,
    U30,
    Name,
    NameArgs,
    U30U30,
    Str,
    Int,
    Uint,
    Double,
    Branch,
    Switch,
    Debug,
}

fn op(code: u8) -> Option<(&'static str, Ops)> {
    use Ops::*;
    Some(match code {
        0x01 => ("bkpt", None), 0x02 => ("nop", None), 0x03 => ("throw", None),
        0x04 => ("getsuper", Name), 0x05 => ("setsuper", Name), 0x06 => ("dxns", Str),
        0x07 => ("dxnslate", None), 0x08 => ("kill", U30), 0x09 => ("label", None),
        0x0C => ("ifnlt", Branch), 0x0D => ("ifnle", Branch), 0x0E => ("ifngt", Branch),
        0x0F => ("ifnge", Branch), 0x10 => ("jump", Branch), 0x11 => ("iftrue", Branch),
        0x12 => ("iffalse", Branch), 0x13 => ("ifeq", Branch), 0x14 => ("ifne", Branch),
        0x15 => ("iflt", Branch), 0x16 => ("ifle", Branch), 0x17 => ("ifgt", Branch),
        0x18 => ("ifge", Branch), 0x19 => ("ifstricteq", Branch), 0x1A => ("ifstrictne", Branch),
        0x1B => ("lookupswitch", Switch), 0x1C => ("pushwith", None), 0x1D => ("popscope", None),
        0x1E => ("nextname", None), 0x1F => ("hasnext", None), 0x20 => ("pushnull", None),
        0x21 => ("pushundefined", None), 0x23 => ("nextvalue", None), 0x24 => ("pushbyte", U8),
        0x25 => ("pushshort", U30), 0x26 => ("pushtrue", None), 0x27 => ("pushfalse", None),
        0x28 => ("pushnan", None), 0x29 => ("pop", None), 0x2A => ("dup", None),
        0x2B => ("swap", None), 0x2C => ("pushstring", Str), 0x2D => ("pushint", Int),
        0x2E => ("pushuint", Uint), 0x2F => ("pushdouble", Double), 0x30 => ("pushscope", None),
        0x31 => ("pushnamespace", U30), 0x32 => ("hasnext2", U30U30),
        0x35 => ("li8", None), 0x36 => ("li16", None), 0x37 => ("li32", None),
        0x38 => ("lf32", None), 0x39 => ("lf64", None), 0x3A => ("si8", None),
        0x3B => ("si16", None), 0x3C => ("si32", None), 0x3D => ("sf32", None),
        0x3E => ("sf64", None),
        0x40 => ("newfunction", U30), 0x41 => ("call", U30), 0x42 => ("construct", U30),
        0x43 => ("callmethod", U30U30), 0x44 => ("callstatic", U30U30),
        0x45 => ("callsuper", NameArgs), 0x46 => ("callproperty", NameArgs),
        0x47 => ("returnvoid", None), 0x48 => ("returnvalue", None),
        0x49 => ("constructsuper", U30), 0x4A => ("constructprop", NameArgs),
        0x4C => ("callproplex", NameArgs), 0x4E => ("callsupervoid", NameArgs),
        0x4F => ("callpropvoid", NameArgs),
        0x50 => ("sxi1", None), 0x51 => ("sxi8", None), 0x52 => ("sxi16", None),
        0x53 => ("applytype", U30), 0x55 => ("newobject", U30), 0x56 => ("newarray", U30),
        0x57 => ("newactivation", None), 0x58 => ("newclass", U30),
        0x59 => ("getdescendants", Name), 0x5A => ("newcatch", U30),
        0x5D => ("findpropstrict", Name), 0x5E => ("findproperty", Name),
        0x5F => ("finddef", Name), 0x60 => ("getlex", Name), 0x61 => ("setproperty", Name),
        0x62 => ("getlocal", U30), 0x63 => ("setlocal", U30), 0x64 => ("getglobalscope", None),
        0x65 => ("getscopeobject", U8), 0x66 => ("getproperty", Name),
        0x68 => ("initproperty", Name), 0x6A => ("deleteproperty", Name),
        0x6C => ("getslot", U30), 0x6D => ("setslot", U30), 0x6E => ("getglobalslot", U30),
        0x6F => ("setglobalslot", U30),
        0x70 => ("convert_s", None), 0x71 => ("esc_xelem", None), 0x72 => ("esc_xattr", None),
        0x73 => ("convert_i", None), 0x74 => ("convert_u", None), 0x75 => ("convert_d", None),
        0x76 => ("convert_b", None), 0x77 => ("convert_o", None), 0x78 => ("checkfilter", None),
        0x80 => ("coerce", Name), 0x81 => ("coerce_b", None), 0x82 => ("coerce_a", None),
        0x83 => ("coerce_i", None), 0x84 => ("coerce_d", None), 0x85 => ("coerce_s", None),
        0x86 => ("astype", Name), 0x87 => ("astypelate", None), 0x88 => ("coerce_u", None),
        0x89 => ("coerce_o", None),
        0x90 => ("negate", None), 0x91 => ("increment", None), 0x92 => ("inclocal", U30),
        0x93 => ("decrement", None), 0x94 => ("declocal", U30), 0x95 => ("typeof", None),
        0x96 => ("not", None), 0x97 => ("bitnot", None),
        0xA0 => ("add", None), 0xA1 => ("subtract", None), 0xA2 => ("multiply", None),
        0xA3 => ("divide", None), 0xA4 => ("modulo", None), 0xA5 => ("lshift", None),
        0xA6 => ("rshift", None), 0xA7 => ("urshift", None), 0xA8 => ("bitand", None),
        0xA9 => ("bitor", None), 0xAA => ("bitxor", None), 0xAB => ("equals", None),
        0xAC => ("strictequals", None), 0xAD => ("lessthan", None), 0xAE => ("lessequals", None),
        0xAF => ("greaterthan", None), 0xB0 => ("greaterequals", None),
        0xB1 => ("instanceof", None), 0xB2 => ("istype", Name), 0xB3 => ("istypelate", None),
        0xB4 => ("in", None),
        0xC0 => ("increment_i", None), 0xC1 => ("decrement_i", None), 0xC2 => ("inclocal_i", U30),
        0xC3 => ("declocal_i", U30), 0xC4 => ("negate_i", None), 0xC5 => ("add_i", None),
        0xC6 => ("subtract_i", None), 0xC7 => ("multiply_i", None),
        0xD0 => ("getlocal0", None), 0xD1 => ("getlocal1", None), 0xD2 => ("getlocal2", None),
        0xD3 => ("getlocal3", None), 0xD4 => ("setlocal0", None), 0xD5 => ("setlocal1", None),
        0xD6 => ("setlocal2", None), 0xD7 => ("setlocal3", None),
        0xEF => ("debug", Debug), 0xF0 => ("debugline", U30), 0xF1 => ("debugfile", Str),
        0xF2 => ("bkptline", U30),
        _ => return Option::None,
    })
}

fn u30(bc: &[u8], at: &mut usize) -> u32 {
    let mut v = 0u32;
    let mut shift = 0;
    while *at < bc.len() {
        let b = bc[*at] as u32;
        *at += 1;
        v |= (b & 0x7F) << shift;
        shift += 7;
        if b & 0x80 == 0 || shift >= 35 { break; }
    }
    v
}

fn s24(bc: &[u8], at: &mut usize) -> i32 {
    if *at + 3 > bc.len() { *at = bc.len(); return 0; }
    let v = bc[*at] as i32 | (bc[*at + 1] as i32) << 8 | (bc[*at + 2] as i32) << 16;
    *at += 3;
    if v & 0x80_0000 != 0 { v - 0x100_0000 } else { v }
}

fn name(abc: &AbcFile, i: u32) -> String {
    match abc.multinames.get(i as usize) {
        Some(m) if !m.name.is_empty() => m.name.to_string(),
        // a name worked out at run time (from the stack), or one the pool leaves blank
        Some(m) => format!("<runtime name, kind {:#x}>", m.kind),
        None => format!("<name {i}>"),
    }
}

/// The method body's code, one instruction a line: its address, mnemonic and operands.
pub fn listing(bc: &[u8], abc: &AbcFile) -> String {
    let mut out = String::new();
    let mut at = 0usize;
    while at < bc.len() {
        let start = at;
        let code = bc[at];
        at += 1;
        let Some((mnem, ops)) = op(code) else {
            out.push_str(&format!("{start:05}  .byte {code:#04x}\n"));
            continue;
        };
        let args = match ops {
            Ops::None => String::new(),
            Ops::U8 => {
                let v = bc.get(at).copied().unwrap_or(0);
                at += 1;
                if code == 0x24 { (v as i8).to_string() } else { v.to_string() }
            }
            Ops::U30 => {
                let v = u30(bc, &mut at);
                if code == 0x25 { (v as i16).to_string() } else { v.to_string() }
            }
            Ops::Name => name(abc, u30(bc, &mut at)),
            Ops::NameArgs => {
                let n = u30(bc, &mut at);
                let argc = u30(bc, &mut at);
                format!("{} ({argc} args)", name(abc, n))
            }
            Ops::U30U30 => {
                let a = u30(bc, &mut at);
                let b = u30(bc, &mut at);
                format!("{a}, {b}")
            }
            Ops::Str => {
                let i = u30(bc, &mut at);
                format!("{:?}", abc.strings.get(i as usize).map(|s| s.to_string()).unwrap_or_default())
            }
            Ops::Int => abc.ints.get(u30(bc, &mut at) as usize).copied().unwrap_or(0).to_string(),
            Ops::Uint => abc.uints.get(u30(bc, &mut at) as usize).copied().unwrap_or(0).to_string(),
            Ops::Double => abc.doubles.get(u30(bc, &mut at) as usize).copied().unwrap_or(f64::NAN).to_string(),
            Ops::Branch => {
                let off = s24(bc, &mut at);
                format!("-> {:05}", at as i64 + off as i64)
            }
            Ops::Switch => {
                // offsets count from the switch's own address
                let default = s24(bc, &mut at);
                let n = u30(bc, &mut at);
                let mut cases = Vec::new();
                for _ in 0..=n { cases.push(format!("{:05}", start as i64 + s24(bc, &mut at) as i64)); }
                format!("default -> {:05}, cases -> [{}]", start as i64 + default as i64, cases.join(", "))
            }
            Ops::Debug => {
                let kind = bc.get(at).copied().unwrap_or(0);
                at += 1;
                let reg_name = u30(bc, &mut at);
                let reg = bc.get(at).copied().unwrap_or(0);
                at += 1;
                let extra = u30(bc, &mut at);
                let shown = abc.strings.get(reg_name as usize).map(|s| s.to_string()).unwrap_or_default();
                format!("{kind}, {shown:?}, r{reg}, {extra}")
            }
        };
        if args.is_empty() {
            out.push_str(&format!("{start:05}  {mnem}\n"));
        } else {
            out.push_str(&format!("{start:05}  {mnem} {args}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_abc() -> AbcFile {
        AbcFile { strings: vec!["hi".into()], ints: vec![], uints: vec![], doubles: vec![], multinames: vec![],
            methods: vec![], classes: vec![], scripts: vec![], method_bodies: vec![] }
    }

    #[test]
    fn lists_operands_and_branch_targets() {
        // getlocal0; pushbyte -2; iffalse +2 (to 00009); pushstring "hi"; returnvalue; returnvoid
        let bc = [0xD0, 0x24, 0xFE, 0x12, 0x02, 0x00, 0x00, 0x2C, 0x00, 0x48, 0x47];
        let text = listing(&bc, &empty_abc());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "00000  getlocal0");
        assert_eq!(lines[1], "00001  pushbyte -2");
        assert_eq!(lines[2], "00003  iffalse -> 00009");
        assert_eq!(lines[3], "00007  pushstring \"hi\"");
        assert_eq!(lines[4], "00009  returnvalue");
    }
}
