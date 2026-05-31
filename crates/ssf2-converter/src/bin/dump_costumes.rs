/// Dump ABC costume/palette data from an SSF2 file.
///
/// Finds the method body for getCostumeData and decodes its opcodes
/// to extract the actual costume color data.

fn read_u30(data: &[u8], pos: &mut usize) -> u32 {
    let mut result = 0u32;
    let mut shift = 0;
    loop {
        let b = data[*pos]; *pos += 1;
        result |= ((b & 0x7f) as u32) << shift;
        shift += 7;
        if b & 0x80 == 0 { break; }
    }
    result
}

fn read_s24(data: &[u8], pos: &mut usize) -> i32 {
    let b0 = data[*pos] as i32; *pos += 1;
    let b1 = data[*pos] as i32; *pos += 1;
    let b2 = data[*pos] as i32; *pos += 1;
    let v = b0 | (b1 << 8) | (b2 << 16);
    if v & 0x800000 != 0 { v | !0xFFFFFF } else { v }
}

struct AbcPool {
    strings: Vec<String>,
    ints: Vec<i32>,
    uints: Vec<u32>,
    doubles: Vec<f64>,
    multinames: Vec<String>,
}

fn parse_pool(abc: &[u8]) -> (AbcPool, usize) {
    let mut pos = 4usize;

    let n = read_u30(abc, &mut pos) as usize;
    let mut ints = vec![0i32];
    for _ in 0..n.saturating_sub(1) { ints.push(read_u30(abc, &mut pos) as i32); }

    let n = read_u30(abc, &mut pos) as usize;
    let mut uints = vec![0u32];
    for _ in 0..n.saturating_sub(1) { uints.push(read_u30(abc, &mut pos)); }

    let n = read_u30(abc, &mut pos) as usize;
    let mut doubles = vec![0f64];
    for _ in 0..n.saturating_sub(1) {
        doubles.push(f64::from_le_bytes(abc[pos..pos+8].try_into().unwrap()));
        pos += 8;
    }

    let n = read_u30(abc, &mut pos) as usize;
    let mut strings = vec![String::new()];
    for _ in 0..n.saturating_sub(1) {
        let slen = read_u30(abc, &mut pos) as usize;
        strings.push(String::from_utf8_lossy(&abc[pos..pos+slen]).to_string());
        pos += slen;
    }

    // namespace pool
    let n = read_u30(abc, &mut pos) as usize;
    for _ in 0..n.saturating_sub(1) { pos += 1; read_u30(abc, &mut pos); }
    // ns set pool
    let n = read_u30(abc, &mut pos) as usize;
    for _ in 0..n.saturating_sub(1) {
        let cnt = read_u30(abc, &mut pos) as usize;
        for _ in 0..cnt { read_u30(abc, &mut pos); }
    }
    // multiname pool
    let n = read_u30(abc, &mut pos) as usize;
    let mut multinames = vec![String::new()];
    for _ in 0..n.saturating_sub(1) {
        let kind = abc[pos]; pos += 1;
        let name = match kind {
            0x07 | 0x0D => { read_u30(abc, &mut pos); let ni = read_u30(abc, &mut pos) as usize; strings.get(ni).cloned().unwrap_or_default() }
            0x0F | 0x10 => { let ni = read_u30(abc, &mut pos) as usize; strings.get(ni).cloned().unwrap_or_default() }
            0x11 | 0x12 => String::new(),
            0x09 | 0x0E => { let ni = read_u30(abc, &mut pos) as usize; read_u30(abc, &mut pos); strings.get(ni).cloned().unwrap_or_default() }
            0x1B | 0x1C => { read_u30(abc, &mut pos); String::new() }
            _ => String::new(),
        };
        multinames.push(name);
    }

    (AbcPool { strings, ints, uints, doubles, multinames }, pos)
}

struct MethodInfo { // unused, fields reference external indices
    param_count: u32, // number of parameters (ignored)
    name_idx: u32     // multiname index (ignored)
}

fn skip_methods(abc: &[u8], pos: &mut usize) -> Vec<MethodInfo> {
    let n = read_u30(abc, pos) as usize;
    let mut methods = Vec::with_capacity(n);
    for _ in 0..n {
        let param_count = read_u30(abc, pos);
        read_u30(abc, pos); // return type
        for _ in 0..param_count { read_u30(abc, pos); }
        let name_idx = read_u30(abc, pos);
        let flags = abc[*pos]; *pos += 1;
        if flags & 0x08 != 0 { // HAS_OPTIONAL
            let opt_count = read_u30(abc, pos) as usize;
            for _ in 0..opt_count { read_u30(abc, pos); *pos += 1; }
        }
        if flags & 0x80 != 0 { // HAS_PARAM_NAMES
            for _ in 0..param_count { read_u30(abc, pos); }
        }
        methods.push(MethodInfo { param_count, name_idx });
    }
    methods
}

fn skip_metadata(abc: &[u8], pos: &mut usize) {
    let n = read_u30(abc, pos) as usize;
    for _ in 0..n {
        read_u30(abc, pos);
        let cnt = read_u30(abc, pos) as usize;
        for _ in 0..cnt { read_u30(abc, pos); read_u30(abc, pos); }
    }
}

fn skip_traits(abc: &[u8], pos: &mut usize) -> Vec<(u32, u32)> { // (name_idx, method_idx)
    let n = read_u30(abc, pos) as usize;
    let mut result = vec![];
    for _ in 0..n {
        let name_mn = read_u30(abc, pos);
        let kind_flags = abc[*pos]; *pos += 1;
        let kind = kind_flags & 0x0F;
        match kind {
            0 | 6 => { read_u30(abc, pos); read_u30(abc, pos); read_u30(abc, pos); } // slot/const
            1 | 2 | 3 => { read_u30(abc, pos); let mi = read_u30(abc, pos); result.push((name_mn, mi)); } // method/getter/setter
            4 => { read_u30(abc, pos); read_u30(abc, pos); } // class
            5 => { read_u30(abc, pos); read_u30(abc, pos); } // function
            _ => {}
        }
        if kind_flags & 0x40 != 0 { // ATTR_Metadata
            let mc = read_u30(abc, pos) as usize;
            for _ in 0..mc { read_u30(abc, pos); }
        }
    }
    result
}

// Decode opcodes from a method body and print them with pool lookups
fn decode_body(code: &[u8], pool: &AbcPool) -> Vec<String> {
    let mut pos = 0;
    let mut ops = vec![];
    while pos < code.len() {
        let op = code[pos]; pos += 1;
        let desc = match op {
            0x24 => { let v = code[pos] as i8; pos += 1; format!("pushbyte {}", v) }
            0x25 => { let v = read_u30(code, &mut pos) as i16; format!("pushshort {}", v) }
            0x2C => { let i = read_u30(code, &mut pos) as usize; format!("pushstring {:?}", pool.strings.get(i).cloned().unwrap_or_default()) }
            0x2D => { let i = read_u30(code, &mut pos) as usize; format!("pushint {}", pool.ints.get(i).copied().unwrap_or(0)) }
            0x2E => { let i = read_u30(code, &mut pos) as usize; format!("pushuint {}", pool.uints.get(i).copied().unwrap_or(0)) }
            0x2F => { let i = read_u30(code, &mut pos) as usize; format!("pushdouble {}", pool.doubles.get(i).copied().unwrap_or(0.0)) }
            0x26 => "pushtrue".into(),
            0x27 => "pushfalse".into(),
            0x20 => "pushnull".into(),
            0x28 => "pushnan".into(),
            0x55 => { let n = read_u30(code, &mut pos); format!("newobject({})", n) }
            0x56 => { let n = read_u30(code, &mut pos); format!("newarray({})", n) }
            0x46 => { let mn = read_u30(code, &mut pos) as usize; let argc = read_u30(code, &mut pos); format!("callproperty {:?} argc={}", pool.multinames.get(mn).cloned().unwrap_or_default(), argc) }
            0x4F => { let mn = read_u30(code, &mut pos) as usize; let argc = read_u30(code, &mut pos); format!("callpropvoid {:?} argc={}", pool.multinames.get(mn).cloned().unwrap_or_default(), argc) }
            0x60 => { let mn = read_u30(code, &mut pos) as usize; format!("getlex {:?}", pool.multinames.get(mn).cloned().unwrap_or_default()) }
            0x61 => { let mn = read_u30(code, &mut pos) as usize; format!("setproperty {:?}", pool.multinames.get(mn).cloned().unwrap_or_default()) }
            0x66 => { let mn = read_u30(code, &mut pos) as usize; format!("getproperty {:?}", pool.multinames.get(mn).cloned().unwrap_or_default()) }
            0x68 => { let mn = read_u30(code, &mut pos) as usize; format!("initproperty {:?}", pool.multinames.get(mn).cloned().unwrap_or_default()) }
            0x5C => { let mn = read_u30(code, &mut pos) as usize; format!("findprop {:?}", pool.multinames.get(mn).cloned().unwrap_or_default()) }
            0x5D => { let mn = read_u30(code, &mut pos) as usize; format!("findpropstrict {:?}", pool.multinames.get(mn).cloned().unwrap_or_default()) }
            0x48 => "returnvalue".into(),
            0x47 => "returnvoid".into(),
            0xD0 => "getlocal_0".into(),
            0xD1 => "getlocal_1".into(),
            0xD2 => "getlocal_2".into(),
            0xD3 => "getlocal_3".into(),
            0x62 => { let i = read_u30(code, &mut pos); format!("getlocal {}", i) }
            0x63 => { let i = read_u30(code, &mut pos); format!("setlocal {}", i) }
            0xD4 => "setlocal_0".into(),
            0xD5 => "setlocal_1".into(),
            0xD6 => "setlocal_2".into(),
            0xD7 => "setlocal_3".into(),
            0x02 => "nop".into(),
            0x29 => "pop".into(),
            0x2A => "dup".into(),
            0x2B => "swap".into(),
            0x80 => { let mn = read_u30(code, &mut pos) as usize; format!("coerce {:?}", pool.multinames.get(mn).cloned().unwrap_or_default()) }
            0x82 => "coerce_a".into(),
            0x4A => { let mn = read_u30(code, &mut pos) as usize; let argc = read_u30(code, &mut pos); format!("constructprop {:?} argc={}", pool.multinames.get(mn).cloned().unwrap_or_default(), argc) }
            0x42 => { let argc = read_u30(code, &mut pos); format!("construct argc={}", argc) }
            0xA0 => "add".into(),
            0xA1 => "subtract".into(),
            0xA2 => "multiply".into(),
            0xA3 => "divide".into(),
            0x90 => "negate".into(),
            0x10 => { let off = read_s24(code, &mut pos); format!("jump {}", off) }
            0x0C => { let off = read_s24(code, &mut pos); format!("ifnlt {}", off) }
            0x0D => { let off = read_s24(code, &mut pos); format!("ifnle {}", off) }
            0x0E => { let off = read_s24(code, &mut pos); format!("ifngt {}", off) }
            0x0F => { let off = read_s24(code, &mut pos); format!("ifnge {}", off) }
            0x13 => { let off = read_s24(code, &mut pos); format!("ifeq {}", off) }
            0x14 => { let off = read_s24(code, &mut pos); format!("ifne {}", off) }
            0x15 => { let off = read_s24(code, &mut pos); format!("iflt {}", off) }
            0x16 => { let off = read_s24(code, &mut pos); format!("ifle {}", off) }
            0x17 => { let off = read_s24(code, &mut pos); format!("ifgt {}", off) }
            0x18 => { let off = read_s24(code, &mut pos); format!("ifge {}", off) }
            0x19 => { let off = read_s24(code, &mut pos); format!("ifstricteq {}", off) }
            0x1A => { let off = read_s24(code, &mut pos); format!("ifstrictne {}", off) }
            0x08 => { let i = read_u30(code, &mut pos); format!("kill {}", i) }
            0x09 => { let off = read_s24(code, &mut pos); format!("label {}", off) }
            0x1B => { // lookupswitch
                let default_off = read_s24(code, &mut pos);
                let case_count = read_u30(code, &mut pos);
                let mut cases = vec![];
                for _ in 0..=case_count { cases.push(read_s24(code, &mut pos)); }
                format!("lookupswitch default={} cases={:?}", default_off, cases)
            }
            0x30 => "pushscope".into(),
            0x1D => "popscope".into(),
            0x65 => { let i = read_u30(code, &mut pos); format!("getscopeobject {}", i) }
            0x6E => { let mn = read_u30(code, &mut pos) as usize; let argc = read_u30(code, &mut pos); format!("callproplex {:?} argc={}", pool.multinames.get(mn).cloned().unwrap_or_default(), argc) }
            0x4B => { let mn = read_u30(code, &mut pos) as usize; let argc = read_u30(code, &mut pos); format!("callsuper {:?} argc={}", pool.multinames.get(mn).cloned().unwrap_or_default(), argc) }
            0x45 => { let mn = read_u30(code, &mut pos) as usize; let argc = read_u30(code, &mut pos); format!("callsuper {:?} argc={}", pool.multinames.get(mn).cloned().unwrap_or_default(), argc) }
            0x41 => { let argc = read_u30(code, &mut pos); format!("call argc={}", argc) }
            0x40 => { let mi = read_u30(code, &mut pos); format!("newfunction {}", mi) }
            0xAB => "instanceof".into(),
            0xB1 => "instanceof".into(),
            0x96 => "not".into(),
            0xA8 => "bitand".into(),
            0xA9 => "bitor".into(),
            0xAA => "bitxor".into(),
            0xA5 => "lshift".into(),
            0xA6 => "rshift".into(),
            0x73 => "convert_i".into(),
            0x74 => "convert_u".into(),
            0x75 => "convert_d".into(),
            0x70 => "convert_s".into(),
            0x76 => "convert_b".into(),
            0x6F => { let mn = read_u30(code, &mut pos) as usize; let argc = read_u30(code, &mut pos); format!("callsupervoid {:?} argc={}", pool.multinames.get(mn).cloned().unwrap_or_default(), argc) }
            _ => format!("op_{:02x}", op),
        };
        ops.push(desc);
    }
    ops
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("usage: dump_costumes <file.ssf>");
    let raw = std::fs::read(&path)?;

    let swf_buf = swf::decompress_swf(&raw[..])?;
    let swf_movie = swf::parse_swf(&swf_buf)?;

    let keywords = ["costume", "Costume", "palette", "Palette",
                    "getCostumeData", "applyPalette", "PaletteSwap"];

    for (bi, tag) in swf_movie.tags.iter().enumerate() {
        let abc_bytes: &[u8] = match tag {
            swf::Tag::DoAbc(data) => data,
            swf::Tag::DoAbc2(abc) => &abc.data,
            _ => continue,
        };

        let (pool, mut pos) = parse_pool(abc_bytes);

        // Check if this block has any keyword strings
        let has_keywords = pool.strings.iter().any(|s| keywords.iter().any(|k| s.contains(k)));
        if !has_keywords { continue; }

        println!("=== ABC block {} ({} strings) ===", bi, pool.strings.len());

        // Print all keyword strings
        for (i, s) in pool.strings.iter().enumerate() {
            if keywords.iter().any(|k| s.contains(k)) {
                println!("  str[{i}] = {s:?}");
            }
        }

        // Now parse method_info, metadata, classes, scripts, method_bodies
        let methods = skip_methods(abc_bytes, &mut pos);
        println!("\n  {} methods", methods.len());

        skip_metadata(abc_bytes, &mut pos);

        // classes
        let class_count = read_u30(abc_bytes, &mut pos) as usize;
        println!("  {} classes", class_count);
        let mut all_traits: Vec<(String, String, u32)> = vec![]; // (class, method, method_idx)
        for _ in 0..class_count {
            let name_mn = read_u30(abc_bytes, &mut pos) as usize;
            let class_name = pool.multinames.get(name_mn).cloned().unwrap_or_default();
            read_u30(abc_bytes, &mut pos); // super
            let flags = abc_bytes[pos]; pos += 1;
            if flags & 0x08 != 0 { read_u30(abc_bytes, &mut pos); } // protected ns
            let iface_count = read_u30(abc_bytes, &mut pos) as usize;
            for _ in 0..iface_count { read_u30(abc_bytes, &mut pos); }
            read_u30(abc_bytes, &mut pos); // constructor
            let traits = skip_traits(abc_bytes, &mut pos);
            for (mn_idx, mi) in traits {
                let mn = pool.multinames.get(mn_idx as usize).cloned().unwrap_or_default();
                if keywords.iter().any(|k| mn.contains(k)) {
                    println!("  TRAIT instance {}::{} method_idx={}", class_name, mn, mi);
                    all_traits.push((class_name.clone(), mn, mi));
                }
            }
        }
        for _ in 0..class_count {
            let traits = skip_traits(abc_bytes, &mut pos);
            for (mn_idx, mi) in traits {
                let mn = pool.multinames.get(mn_idx as usize).cloned().unwrap_or_default();
                if keywords.iter().any(|k| mn.contains(k)) {
                    println!("  TRAIT static method_idx={} name={}", mi, mn);
                    all_traits.push(("(static)".into(), mn, mi));
                }
            }
        }

        // scripts
        let script_count = read_u30(abc_bytes, &mut pos) as usize;
        for _ in 0..script_count {
            read_u30(abc_bytes, &mut pos);
            skip_traits(abc_bytes, &mut pos);
        }

        // method bodies
        let body_count = read_u30(abc_bytes, &mut pos) as usize;
        println!("  {} method bodies", body_count);
        for _ in 0..body_count {
            let mi = read_u30(abc_bytes, &mut pos);
            read_u30(abc_bytes, &mut pos); // max_stack
            read_u30(abc_bytes, &mut pos); // local_count
            read_u30(abc_bytes, &mut pos); // init_scope_depth
            read_u30(abc_bytes, &mut pos); // max_scope_depth
            let code_len = read_u30(abc_bytes, &mut pos) as usize;
            let code = &abc_bytes[pos..pos+code_len];
            pos += code_len;
            // exception table
            let exc_count = read_u30(abc_bytes, &mut pos) as usize;
            for _ in 0..exc_count {
                read_u30(abc_bytes, &mut pos); read_u30(abc_bytes, &mut pos);
                read_u30(abc_bytes, &mut pos); read_u30(abc_bytes, &mut pos);
                read_u30(abc_bytes, &mut pos);
            }
            skip_traits(abc_bytes, &mut pos);

            if all_traits.iter().any(|(_, _, t_mi)| *t_mi == mi) {
                let (cls, mname, _) = all_traits.iter().find(|(_, _, t_mi)| *t_mi == mi).unwrap();
                println!("\n  >>> {}::{} method_idx={} ({} bytes)", cls, mname, mi, code_len);
                for op in decode_body(code, &pool) {
                    println!("      {}", op);
                }
            }
        }
    }

    Ok(())
}
