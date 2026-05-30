use std::fs::File;
use std::io::{BufReader, BufWriter};

use hlbc::opcodes::Opcode;
use hlbc::types::{Reg, RefString};
use hlbc::Bytecode;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: fray_patch <input.dat> <output.dat> [mode]");
        eprintln!("  mode: roundtrip (default) | probe");
        std::process::exit(2);
    }
    let input = &args[1];
    let output = &args[2];
    let mode = args.get(3).map(|s| s.as_str()).unwrap_or("roundtrip");

    let mut r = BufReader::new(File::open(input)?);
    let mut code = Bytecode::deserialize(&mut r).map_err(|e| anyhow::anyhow!("deserialize: {e:?}"))?;
    eprintln!(
        "loaded: version={} functions={} strings={} types={} globals={}",
        code.version,
        code.functions.len(),
        code.strings.len(),
        code.types.len(),
        code.globals.len()
    );

    match mode {
        "roundtrip" => {}
        "probe" => probe_edit(&mut code, 17746)?, // fraymakers.Main.onLoaded@17746
        "inspect" => {
            inspect(&code);
            return Ok(()); // nothing to write
        }
        "whoref" => {
            let needle = args.get(4).cloned().unwrap();
            // string indices whose text contains the needle
            let sidxs: Vec<usize> = (0..code.strings.len())
                .filter(|&i| code.strings[i].as_str().contains(&needle))
                .collect();
            eprintln!("strings matching {needle:?}: {sidxs:?}");
            // const-globals initialized to those strings
            let mut gidxs: Vec<usize> = vec![];
            if let Some(consts) = &code.constants {
                for c in consts {
                    if c.fields.first().map(|f| sidxs.contains(f)).unwrap_or(false) {
                        gidxs.push(c.global.0);
                    }
                }
            }
            eprintln!("const-globals for those strings: {gidxs:?}");
            // functions that load those strings (String opcode) or GetGlobal those globals
            for f in &code.functions {
                let hit = f.ops.iter().any(|op| match op {
                    Opcode::String { ptr, .. } => sidxs.contains(&ptr.0),
                    Opcode::GetGlobal { global, .. } => gidxs.contains(&global.0),
                    _ => false,
                });
                if hit {
                    let pn = f.parent.and_then(|rt| type_name_of(&code, rt)).unwrap_or("?");
                    eprintln!("  findex {:6} {}::{}", f.findex.0, pn, s(&code, f.name));
                }
            }
            return Ok(());
        }
        "fnsof" => {
            let tname = args.get(4).cloned().unwrap();
            eprintln!("functions with parent == {tname}:");
            for f in &code.functions {
                if f.parent.and_then(|rt| type_name_of(&code, rt)) == Some(tname.as_str()) {
                    let argc = code.types[f.t.0].get_type_fun().map(|tf| tf.args.len()).unwrap_or(0);
                    eprintln!("  findex {:6} name={:?} argc={argc}", f.findex.0, s(&code, f.name));
                }
            }
            return Ok(());
        }
        "fninfo" => {
            let fx: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap();
            let fi = function_index_by_findex(&code, fx).unwrap();
            let f = &code.functions[fi];
            eprintln!("findex {fx}: name={:?} (#{}) parent={:?}",
                s(&code, f.name), f.name.0,
                f.parent.map(|rt| (rt.0, type_name_of(&code, rt).map(|x| x.to_string()))));
            if let Some(tf) = code.types[f.t.0].get_type_fun() {
                let args: Vec<(usize, String)> = tf.args.iter()
                    .map(|a| (a.0, code.types[a.0].get_type_obj().map(|o| s(&code, o.name).to_string()).unwrap_or_else(|| format!("t{}", a.0)))).collect();
                let ret = tf.ret.0;
                let retn = code.types[ret].get_type_obj().map(|o| s(&code, o.name).to_string()).unwrap_or_else(|| format!("t{ret}"));
                eprintln!("  args: {args:?}  ret: ({ret}, {retn:?})");
            }
            return Ok(());
        }
        "dis" => {
            let fx: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap();
            disasm(&code, fx);
            return Ok(());
        }
        "typefields" => {
            let tname = args.get(4).cloned().unwrap();
            // allow "t123" or a bare index to address a type directly
            let direct: Option<usize> = tname.strip_prefix('t').and_then(|n| n.parse().ok())
                .or_else(|| tname.parse().ok());
            match direct.filter(|&i| i < code.types.len()).or_else(|| find_type(&code, &tname)) {
                Some(ti) => {
                    eprintln!("type {ti} = {tname}");
                    let vfields = match &code.types[ti] {
                        hlbc::types::Type::Virtual { fields } => Some(fields),
                        _ => None,
                    };
                    if let Some(o) = code.types[ti].get_type_obj() {
                        for (i, fl) in o.fields.iter().enumerate() {
                            let tn = type_name_of(&code, fl.t)
                                .map(|x| x.to_string())
                                .unwrap_or_else(|| format!("t{}", fl.t.0));
                            eprintln!("  field {i:3}: {:?} : {tn}", s(&code, fl.name));
                        }
                    } else if let Some(fields) = vfields {
                        eprintln!("  (virtual)");
                        for (i, fl) in fields.iter().enumerate() {
                            let tn = type_name_of(&code, fl.t)
                                .map(|x| x.to_string())
                                .unwrap_or_else(|| format!("t{}", fl.t.0));
                            eprintln!("  field {i:3}: {:?} : {tn}", s(&code, fl.name));
                        }
                    } else {
                        eprintln!("  (kind: {:?})", std::mem::discriminant(&code.types[ti]));
                    }
                }
                None => eprintln!("type not found: {tname}"),
            }
            return Ok(());
        }
        "callers" => {
            let target: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap();
            eprintln!("functions that Call*/Closure findex {target}:");
            for f in &code.functions {
                let hit = f.ops.iter().any(|op| match op {
                    Opcode::Call0 { fun, .. } | Opcode::Call1 { fun, .. }
                    | Opcode::Call2 { fun, .. } | Opcode::Call3 { fun, .. }
                    | Opcode::Call4 { fun, .. } | Opcode::CallN { fun, .. }
                    | Opcode::StaticClosure { fun, .. } | Opcode::InstanceClosure { fun, .. } => fun.0 == target,
                    _ => false,
                });
                if hit {
                    let pn = f.parent.and_then(|rt| type_name_of(&code, rt)).unwrap_or("?");
                    eprintln!("  {:6} {}::{}", f.findex.0, pn, s(&code, f.name));
                }
            }
            return Ok(());
        }
        "strgrep" => {
            let needle = args.get(4).cloned().unwrap();
            for (i, st) in code.strings.iter().enumerate() {
                if st.as_str().contains(&needle) {
                    eprintln!("  str {i:6}: {:?}", st.as_str());
                }
            }
            return Ok(());
        }
        "connect" => {
            // fray_patch <in> <out> connect <port> <token>
            let port: u16 = args.get(4).and_then(|s| s.parse().ok())
                .ok_or_else(|| anyhow::anyhow!("connect mode needs <port>"))?;
            let token = args.get(5).cloned().unwrap_or_default();
            connect_edit(&mut code, port, &token)?;
        }
        other => anyhow::bail!("unknown mode: {other}"),
    }

    let mut w = BufWriter::new(File::create(output)?);
    code.serialize(&mut w)
        .map_err(|e| anyhow::anyhow!("serialize: {e:?}"))?;
    eprintln!("wrote {output} (mode={mode})");
    Ok(())
}

/// Find the index into `code.functions` of the function whose findex == target.
fn function_index_by_findex(code: &Bytecode, findex: usize) -> Option<usize> {
    code.functions.iter().position(|f| f.findex.0 == findex)
}

// ---- name-based symbol resolver (robust across engine versions) -------------

fn s(code: &Bytecode, r: hlbc::types::RefString) -> &str {
    code.strings[r.0].as_str()
}

/// Type index of an Obj/Struct class by its fully-qualified name.
fn find_type(code: &Bytecode, name: &str) -> Option<usize> {
    (0..code.types.len()).find(|&i| {
        code.types[i]
            .get_type_obj()
            .map(|o| s(code, o.name) == name)
            .unwrap_or(false)
    })
}

/// Absolute field index (hierarchy-inclusive) of `field` within Obj type `tidx`.
fn find_field(code: &Bytecode, tidx: usize, field: &str) -> Option<usize> {
    code.types[tidx]
        .get_type_obj()
        .and_then(|o| o.fields.iter().position(|f| s(code, f.name) == field))
}

/// findex of a native function by name (natives live in a separate pool).
fn find_native(code: &Bytecode, name: &str) -> Option<usize> {
    code.natives
        .iter()
        .find(|n| s(code, n.name) == name)
        .map(|n| n.findex.0)
}

fn type_name_of(code: &Bytecode, rt: hlbc::types::RefType) -> Option<&str> {
    code.types
        .get(rt.0)
        .and_then(|t| t.get_type_obj())
        .map(|o| s(code, o.name))
}

/// findex of a function by short name, optionally constrained to a parent type.
fn find_fn(code: &Bytecode, name: &str, parent: Option<&str>) -> Option<usize> {
    code.functions
        .iter()
        .find(|f| {
            s(code, f.name) == name
                && match parent {
                    None => true,
                    Some(p) => f.parent.and_then(|rt| type_name_of(code, rt)) == Some(p),
                }
        })
        .map(|f| f.findex.0)
}

/// List a class's methods (proto names + findexes) — handy for discovery.
fn list_methods(code: &Bytecode, type_name: &str) {
    if let Some(ti) = find_type(code, type_name) {
        if let Some(o) = code.types[ti].get_type_obj() {
            eprintln!("  methods of {type_name} (type {ti}):");
            for p in &o.protos {
                eprintln!("    {} -> findex {}", s(code, p.name), p.findex.0);
            }
        }
    } else {
        eprintln!("  TYPE NOT FOUND: {type_name}");
    }
}

/// Human label for a findex: "Parent::name@findex" (or native).
fn fn_label(code: &Bytecode, findex: usize) -> String {
    if let Some(fi) = function_index_by_findex(code, findex) {
        let f = &code.functions[fi];
        let pn = f.parent.and_then(|rt| type_name_of(code, rt)).unwrap_or("?");
        return format!("{pn}::{}@{findex}", s(code, f.name));
    }
    if let Some(n) = code.natives.iter().find(|n| n.findex.0 == findex) {
        return format!("native {}.{}@{findex}", s(code, n.lib), s(code, n.name));
    }
    format!("?@{findex}")
}

/// Disassemble a function by findex, annotating call targets, fields, strings,
/// globals and the function's own register types / arg signature.
fn disasm(code: &Bytecode, findex: usize) {
    let Some(fi) = function_index_by_findex(code, findex) else {
        eprintln!("no function with findex {findex}");
        return;
    };
    let f = &code.functions[fi];
    let pn = f.parent.and_then(|rt| type_name_of(code, rt)).unwrap_or("?");
    eprintln!("== {pn}::{}@{findex} (fn #{fi}) ==", s(code, f.name));
    // register types
    for (i, r) in f.regs.iter().enumerate() {
        let tn = type_name_of(code, *r)
            .map(|x| x.to_string())
            .unwrap_or_else(|| format!("t{}", r.0));
        eprintln!("   reg{i}: {tn}");
    }
    let field_name = |rt: hlbc::types::RefType, idx: usize| -> String {
        // walk obj fields (flattened) — best-effort name lookup
        code.types
            .get(rt.0)
            .and_then(|t| t.get_type_obj())
            .and_then(|o| o.fields.get(idx).map(|fl| s(code, fl.name).to_string()))
            .unwrap_or_else(|| format!("#{idx}"))
    };
    for (i, op) in f.ops.iter().enumerate() {
        let mut note = String::new();
        match op {
            Opcode::Call0 { fun, .. }
            | Opcode::Call1 { fun, .. }
            | Opcode::Call2 { fun, .. }
            | Opcode::Call3 { fun, .. }
            | Opcode::Call4 { fun, .. }
            | Opcode::CallN { fun, .. } => note = format!("  ; {}", fn_label(code, fun.0)),
            Opcode::CallMethod { field, .. } | Opcode::CallThis { field, .. } => {
                note = format!("  ; method proto #{}", field.0)
            }
            Opcode::Field { obj, field, .. } | Opcode::SetField { obj, field, .. } => {
                let rt = f.regs.get(obj.0 as usize).copied();
                if let Some(rt) = rt {
                    note = format!("  ; .{} (of {})", field_name(rt, field.0),
                        type_name_of(code, rt).unwrap_or("?"));
                }
            }
            Opcode::String { ptr, .. } => {
                note = format!("  ; {:?}", code.strings[ptr.0].as_str())
            }
            Opcode::Int { ptr, .. } => note = format!("  ; ={}", code.ints[ptr.0]),
            Opcode::GetGlobal { global, .. } | Opcode::SetGlobal { global, .. } => {
                // is it a const string global?
                if let Some(consts) = &code.constants {
                    if let Some(c) = consts.iter().find(|c| c.global.0 == global.0) {
                        if let Some(&sidx) = c.fields.first() {
                            note = format!("  ; const str {:?}", code.strings[sidx].as_str());
                        }
                    }
                }
            }
            Opcode::Type { ty, .. } => {
                note = format!("  ; type {}", type_name_of(code, *ty)
                    .map(|x| x.to_string()).unwrap_or_else(|| format!("t{}", ty.0)))
            }
            Opcode::DynGet { field, .. } | Opcode::DynSet { field, .. } => {
                note = format!("  ; dynfield str {:?}", code.strings[field.0].as_str())
            }
            _ => {}
        }
        eprintln!("  {i:4}: {op:?}{note}");
    }
}

/// Print resolutions for everything the injection needs (validates the resolver).
fn inspect(code: &Bytecode) {
    let t = |n: &str| match find_type(code, n) {
        Some(i) => format!("type {i}"),
        None => "MISSING".into(),
    };
    let f = |n: &str, p: Option<&str>| match find_fn(code, n, p) {
        Some(i) => format!("findex {i}"),
        None => "MISSING".into(),
    };
    let fld = |tn: &str, fnm: &str| match find_type(code, tn).and_then(|ti| find_field(code, ti, fnm)) {
        Some(i) => format!("field {i}"),
        None => "MISSING".into(),
    };

    eprintln!("== TYPES ==");
    for n in [
        "sys.net.Socket",
        "sys.net.Host",
        "sys.net._Socket.SocketOutput",
        "sys.net._Socket.SocketInput",
        "String",
        "haxe.io.Bytes",
        "fraymakers.core.FraymakersMatchSettings",
        "pxf.core.MatchSettings",
    ] {
        eprintln!("  {n:50} {}", t(n));
    }

    eprintln!("== FUNCTIONS ==");
    for (n, p) in [
        ("__constructor__", Some("sys.net.Socket")),
        ("connect", Some("sys.net.Socket")),
        ("setBlocking", Some("sys.net.Socket")),
        ("__constructor__", Some("sys.net.Host")),
        ("ofString", Some("haxe.io.Bytes")),
        ("startMatch", None),
        ("createMatchSettings", None),
        ("createMatchSettingsConfig", None),
        ("onLoaded", Some("fraymakers.Main")),
        ("update", Some("fraymakers.Main")),
    ] {
        eprintln!("  {n:22} parent={p:?} -> {}", f(n, p));
    }

    eprintln!("== FIELDS ==");
    for (tn, fnm) in [
        ("sys.net.Socket", "output"),
        ("sys.net.Socket", "input"),
        ("pxf.core.MatchSettings", "matchConfig"),
        ("pxf.core.MatchSettings", "playerConfigs"),
    ] {
        eprintln!("  {tn}.{fnm:16} {}", fld(tn, fnm));
    }

    eprintln!("== METHODS (string-write candidates) ==");
    list_methods(code, "sys.net._Socket.SocketOutput");
    list_methods(code, "haxe.io.Output");
}

// ---- edit helpers -----------------------------------------------------------

fn add_string(code: &mut Bytecode, val: &str) -> usize {
    let i = code.strings.len();
    code.strings.push(hlbc::Str::from(val));
    i
}

/// Create a real `String` *object* constant and return its GLOBAL index.
/// HL's `String` opcode only yields raw bytes — actual String objects come from
/// constant-initialized globals: ConstantDef{global, fields:[strPoolIdx, intPoolIdxOfLen]}.
/// GetGlobal on the returned index yields a usable String (valid .bytes/.length).
fn add_string_const(code: &mut Bytecode, val: &str) -> usize {
    let str_idx = add_string(code, val);
    let len_idx = add_int(code, val.chars().count() as i32);
    let g_idx = code.globals.len();
    code.globals.push(hlbc::types::RefType(13)); // String
    code.constants
        .get_or_insert_with(Vec::new)
        .push(hlbc::types::ConstantDef {
            global: hlbc::types::RefGlobal(g_idx),
            fields: vec![str_idx, len_idx],
        });
    g_idx
}

fn add_int(code: &mut Bytecode, val: i32) -> usize {
    // Reuse if already present (ints pool is small-ish); else append.
    if let Some(i) = code.ints.iter().position(|&x| x == val) {
        return i;
    }
    let i = code.ints.len();
    code.ints.push(val);
    i
}

fn require_type(code: &Bytecode, name: &str) -> anyhow::Result<usize> {
    find_type(code, name).ok_or_else(|| anyhow::anyhow!("type not found: {name}"))
}
fn require_fn(code: &Bytecode, name: &str, parent: Option<&str>) -> anyhow::Result<usize> {
    find_fn(code, name, parent).ok_or_else(|| anyhow::anyhow!("fn not found: {name} parent={parent:?}"))
}
fn require_field(code: &Bytecode, tname: &str, fname: &str) -> anyhow::Result<usize> {
    let ti = require_type(code, tname)?;
    find_field(code, ti, fname).ok_or_else(|| anyhow::anyhow!("field not found: {tname}.{fname}"))
}

/// Insert `ops` at the front of function `fidx`, keeping debug_info length in
/// sync and clearing the (now stale) assigns table. Front insertion is safe for
/// HL's relative jumps (source and target shift together).
fn insert_ops_front(f: &mut hlbc::types::Function, ops: Vec<Opcode>) {
    let n = ops.len();
    for (i, op) in ops.into_iter().enumerate() {
        f.ops.insert(i, op);
    }
    if let Some(dbg) = f.debug_info.as_mut() {
        let first = dbg.first().copied().unwrap_or((0, 0));
        for _ in 0..n {
            dbg.insert(0, first);
        }
    }
    // IMPORTANT: the `assigns` (debug var-name -> op-index) table must stay
    // `Some` — HL always reads its count when debug info is present, so setting
    // it to None desyncs the stream ("Negative index" on load). Just shift the
    // op-indices to account for the prepended ops.
    if let Some(assigns) = f.assigns.as_mut() {
        for (_name, pos) in assigns.iter_mut() {
            *pos += n;
        }
    }
}

/// Append registers of the given type indices; returns the index of the first.
fn add_regs(f: &mut hlbc::types::Function, types: &[usize]) -> u32 {
    let base = f.regs.len() as u32;
    for &t in types {
        f.regs.push(hlbc::types::RefType(t));
    }
    base
}

/// Emit a loop that sends a String over an Output one byte at a time:
/// `for i in 0..s.length { out.writeByte(s.bytes[i*2]); }`.
/// Reads the low byte of each UTF-16 code unit (correct for ASCII) — sidesteps
/// Bytes.ofString/utf16_to_utf8, which SIGSEGVs in this build. Self-contained
/// relative jumps, so it can be spliced anywhere. String fields: bytes=0, length=1.
#[allow(clippy::too_many_arguments)]
fn send_string_loop(
    r_str: Reg, r_out: Reg, r_ret: Reg,
    r_len: Reg, r_bytes: Reg, r_idx: Reg, r_one: Reg, r_off: Reg, r_ch: Reg,
    write_byte: usize, zero_idx: usize, one_idx: usize,
) -> Vec<Opcode> {
    use hlbc::types::{RefField, RefFun, RefInt};
    vec![
        Opcode::Field { dst: r_len, obj: r_str, field: RefField(1) },   // +0 len = s.length
        Opcode::Field { dst: r_bytes, obj: r_str, field: RefField(0) }, // +1 bytes = s.bytes
        Opcode::Int { dst: r_idx, ptr: RefInt(zero_idx) },             // +2 i = 0
        Opcode::Int { dst: r_one, ptr: RefInt(one_idx) },              // +3 one = 1
        Opcode::Label,                                                  // +4 LOOP
        Opcode::JSGte { a: r_idx, b: r_len, offset: 5 },               // +5 if i>=len -> END(+11)
        Opcode::Shl { dst: r_off, a: r_idx, b: r_one },                // +6 off = i*2
        // GetI16 (UTF-16 unit) is what the engine's charCodeAt uses; GetI8 is
        // mis-encoded/unused here. writeByte writes the low byte (ASCII).
        Opcode::GetI16 { dst: r_ch, bytes: r_bytes, index: r_off },    // +7 ch = bytes[i*2]
        Opcode::Call2 { dst: r_ret, fun: RefFun(write_byte), arg0: r_out, arg1: r_ch }, // +8
        Opcode::Incr { dst: r_idx },                                    // +9 i++
        Opcode::JAlways { offset: -7 },                                // +10 -> LOOP(+4)
    ]
}

/// Phase 1a: at onLoaded, connect a client socket to 127.0.0.1:<port> and send
/// `AUTH <token>` + a hello line. Proves socket injection + the auth handshake.
fn connect_edit(code: &mut Bytecode, port: u16, token: &str) -> anyhow::Result<()> {
    // Resolve everything by name (version-robust). Constructors have empty
    // names, so pin their findexes but verify their signatures.
    let sock_t = require_type(code, "sys.net.Socket")?;
    let host_t = require_type(code, "sys.net.Host")?;
    let out_t = require_type(code, "sys.net._Socket.SocketOutput")?;
    let str_t = require_type(code, "String")?;
    let connect = require_fn(code, "connect", Some("sys.net.Socket"))?;
    let write_str = require_fn(code, "writeString", Some("haxe.io.Output"))?;
    let flush = require_fn(code, "flush", Some("haxe.io.Output"))?;
    let out_field = require_field(code, "sys.net.Socket", "output")?;
    // Socket.init() (not the bare ctor) creates __s + input + output.
    let sock_init_method = require_fn(code, "init", Some("sys.net.Socket"))?;
    let ip_field = require_field(code, "sys.net.Host", "ip")?;
    let socket_init = find_native(code, "socket_init")
        .ok_or_else(|| anyhow::anyhow!("native socket_init not found"))?;
    // writeString is (Output, String, Encoding) — Haxe's optional ?encoding is a
    // required 3rd HL param. Grab the Encoding enum type so we can pass null
    // (ofString treats null encoding as UTF-8).
    let enc_t = {
        let wi = function_index_by_findex(code, write_str)
            .ok_or_else(|| anyhow::anyhow!("writeString fn missing"))?;
        code.types[code.functions[wi].t.0]
            .get_type_fun()
            .and_then(|tf| tf.args.get(2))
            .map(|a| a.0)
            .ok_or_else(|| anyhow::anyhow!("writeString has no encoding arg"))?
    };

    eprintln!("resolved: Socket={sock_t} Host={host_t} Out={out_t} connect={connect} writeString={write_str} out.field={out_field} Socket.init={sock_init_method} host.ip={ip_field} socket_init={socket_init}");

    // Constants. Skip Host.host_resolve (it SIGSEGVs mid-boot); set Host.ip
    // directly to 127.0.0.1 in network byte order (bytes 127,0,0,1 = 0x0100007F
    // on this little-endian host) and hand that to connect().
    const LOOPBACK_IP: i32 = 0x0100_007F; // 16777343
    let hello = format!("AUTH {token}\nHELLO_FROM_FRAYMAKERS\n");
    let port_idx = add_int(code, port as i32);
    let ip_const = add_int(code, LOOPBACK_IP);
    // Send the handshake byte-by-byte (writeByte) instead of writeString, which
    // routes through Bytes.ofString/utf16_to_utf8. ASCII out needs no conversion.
    let write_byte = require_fn(code, "writeByte", Some("sys.net._Socket.SocketOutput"))?;
    let set_blocking = require_fn(code, "setBlocking", Some("sys.net.Socket"))?;
    let recv_char = find_native(code, "socket_recv_char")
        .ok_or_else(|| anyhow::anyhow!("native socket_recv_char not found"))?;
    let s_field = require_field(code, "sys.net.Socket", "__s")?;
    let handle_t = code.types[sock_t]
        .get_type_obj()
        .and_then(|o| o.fields.get(s_field))
        .map(|fld| fld.t.0)
        .ok_or_else(|| anyhow::anyhow!("Socket.__s field type missing"))?;
    let hello_g = add_string_const(code, &hello); // real String object (GetGlobal)
    let pong_g = add_string_const(code, "PONG\n");
    let help_g = add_string_const(code, "help"); // console command to run as a test
    let ran_g = add_string_const(code, "RAN\n");
    let zero_idx = add_int(code, 0);
    let one_idx = add_int(code, 1);
    let p_idx = add_int(code, 'p' as i32); // 'ping' command byte
    let c_idx = add_int(code, 'c' as i32); // 'console' command byte
    let s_idx = add_int(code, 's' as i32); // 'start match' command byte
    let stage_str = add_string(code, "stage");
    let character_str = add_string(code, "character");
    let port_str = add_string(code, "port");
    let matchrules_str = add_string(code, "matchRules");
    // defaultMatchRules is a static on pxf.core.$MatchSettings — the same value the
    // engine's MatchSettings ctor injects into its default matchConfig. We replace
    // field 1 (matchConfig) entirely, so we must re-supply matchRules or the match
    // scene null-derefs it on a later frame (clean exit 244, no crash.log).
    let ms_statics_t = find_type(code, "pxf.core.$MatchSettings")
        .ok_or_else(|| anyhow::anyhow!("$MatchSettings type not found"))?;
    let ms_global = code.globals.iter().position(|g| g.0 == ms_statics_t)
        .ok_or_else(|| anyhow::anyhow!("$MatchSettings statics global not found"))?;
    let dmr_field = find_field(code, ms_statics_t, "defaultMatchRules")
        .ok_or_else(|| anyhow::anyhow!("defaultMatchRules field not found"))?;
    let dmr_field_t = code.types[ms_statics_t]
        .get_type_obj()
        .and_then(|o| o.fields.get(dmr_field))
        .map(|fld| fld.t.0)
        .ok_or_else(|| anyhow::anyhow!("defaultMatchRules field type missing"))?;
    eprintln!("matchRules: $MatchSettings global={ms_global} field={dmr_field} fieldtype={dmr_field_t}");
    let resource_str = add_string(code, "resource");
    // Mode-based launch: create a real TrainingMode (createMode@18338 builds a
    // FraymakersMode whose hscript bridge + event wiring are set up), then call its
    // startMatch@6227 with {characters, matchSettings, pauseMenu}. This runs the
    // engine's own offline-match flow (gates transition, menu suspend/restore) —
    // no menu hand-teardown, and the mode stays alive to restore menus on match end.
    let mode_fullid = add_string_const(code, "global::vsmode.trainingmode");
    let characters_str = add_string(code, "characters");
    let matchsettings_str = add_string(code, "matchSettings");
    let pausemenu_str = add_string(code, "pauseMenu");
    let create_mode = require_fn(code, "createMode", Some("fraymakers.util.$FraymakersClassFactory"))?;
    let mode_start_match = require_fn(code, "startMatch", Some("fraymakers.core.FraymakersMode"))?;
    let mode_t = find_type(code, "fraymakers.core.FraymakersMode")
        .ok_or_else(|| anyhow::anyhow!("FraymakersMode type not found"))?;
    eprintln!("mode-launch: createMode={create_mode} FraymakersMode.startMatch={mode_start_match} FraymakersMode t={mode_t}");
    // line-command primitives (build a real String from received arg bytes, split it)
    let bytes_alloc = require_fn(code, "alloc", Some("haxe.io.$Bytes"))?;
    let bytes_set = require_fn(code, "set", Some("haxe.io.Bytes"))?;
    let bytes_getstring = require_fn(code, "getString", Some("haxe.io.Bytes"))?;
    let str_split = require_fn(code, "split", Some("String"))?;
    let space_g = add_string_const(code, " ");
    let buf_cap_idx = add_int(code, 512);
    let nl_idx = add_int(code, '\n' as i32);
    let two_idx = add_int(code, 2);
    let three_idx = add_int(code, 3);
    let four_idx = add_int(code, 4);
    eprintln!("line-cmd: alloc={bytes_alloc} set={bytes_set} getString={bytes_getstring} split={str_split}");
    // short-name resolution: a bare "sandbag" (no "::") is tried against each
    // namespace prefix; the first whose resource actually exists wins.
    let indexof_fn = require_fn(code, "indexOf", Some("String"))?;
    let str_add = require_fn(code, "__add__", Some("$String"))?;
    let getresid_fn = require_fn(code, "getResourceIdentifierString", Some("pxf.io.$ResourceManager"))?;
    let getpxf_fn = require_fn(code, "getPXFResource", Some("pxf.io.$ResourceManager"))?;
    let pxfres_t = require_type(code, "pxf.structs.PXFResource")?;
    let nulli32_t2 = {
        let ci = function_index_by_findex(code, indexof_fn).unwrap();
        code.types[code.functions[ci].t.0].get_type_fun()
            .and_then(|tf| tf.args.get(2)).map(|a| a.0).unwrap()
    };
    let colon2_g = add_string_const(code, "::");
    let dot_g = add_string_const(code, ".");
    // namespace prefixes tried in order for bare names (custom content first)
    let ns_prefixes: Vec<usize> = ["custom::", "public::", "global::"]
        .iter().map(|p| add_string_const(code, p)).collect();
    // registry search: iterate ResourceManager.poolHash (all loaded resources) and
    // check each resource's per-type plain StringMap content map for the bare id.
    let rm_statics_t = code.globals[3508].0; // pxf.io.$ResourceManager statics
    let poolhash_field = find_field(code, rm_statics_t, "poolHash")
        .ok_or_else(|| anyhow::anyhow!("poolHash field not found"))?;
    let sm_keys = require_fn(code, "keys", Some("haxe.ds.StringMap"))?;
    let sm_exists = require_fn(code, "exists", Some("haxe.ds.StringMap"))?;
    let keysiter_t = {
        let ki = function_index_by_findex(code, sm_keys).unwrap();
        code.types[code.functions[ki].t.0].get_type_fun().map(|tf| tf.ret.0).unwrap()
    };
    let stringmap_t = require_type(code, "haxe.ds.StringMap")?;
    // per-content-type plain StringMap fields on PXFResource. There is no
    // assistPxfContentMap — assists are character-type entities, so the assist
    // search reuses characterPxfContentMap.
    let char_cmap_field = find_field(code, pxfres_t, "characterPxfContentMap")
        .ok_or_else(|| anyhow::anyhow!("characterPxfContentMap field not found"))?;
    let stage_cmap_field = find_field(code, pxfres_t, "stagePxfContentMap")
        .ok_or_else(|| anyhow::anyhow!("stagePxfContentMap field not found"))?;
    let assist_cmap_field = char_cmap_field;
    eprintln!("resolve: indexOf={indexof_fn} getResId={getresid_fn} getPXF={getpxf_fn} poolHash={poolhash_field} keys={sm_keys} exists={sm_exists} iter_t={keysiter_t} cmaps(char={char_cmap_field},assist={assist_cmap_field},stage={stage_cmap_field})");
    // fullIds must be REAL String objects (parseResourceIdentifier regex-matches them)
    let stage_fullid = add_string_const(code, "public::thespire.thespire");
    let char_fullid = add_string_const(code, "public::commandervideo.commandervideo");
    // The HUD's DamageCounter generates an assist sprite per player; a null assist
    // null-derefs .namespace in getContentIdentifierString. Supply a real assist.
    let assist_fullid = add_string_const(code, "public::commandervideo.commandervideoassist");
    let assist_str = add_string(code, "assist");
    let launched_g = add_string_const(code, "LAUNCHED\n");
    let launched2_g = add_string_const(code, "LAUNCHED "); // verbose ack prefix
    let nl_g = add_string_const(code, "\n");
    let getcontentid_fn = require_fn(code, "getContentIdentifierString", Some("pxf.io.$ResourceManager"))?;
    // 'q' query command: report whether a match is live (answers "did it start?").
    let q_idx = add_int(code, 'q' as i32);
    let q_nomatch_g = add_string_const(code, "Q:NO_MATCH\n");
    let q_live_g = add_string_const(code, "Q:MATCH_LIVE\n");
    // Diagnostic: currentMatch (statics f6) is null right after `s` even when the
    // match is alive, because it's only set in onMatchReady. _matches (statics
    // f13, an ArrayObj) is pushed in the same onMatchReady, so its length tells us
    // whether a Match object actually exists yet. This disambiguates "match live,
    // q reads the wrong ref" (matches>0) from "match never started" (matches==0).
    let q_matches_g = add_string_const(code, "Q:MATCHES_NONEMPTY\n");
    let mc_statics_t = code.globals[3511].0; // pxf.controllers.$MatchController statics
    let cm_field = find_field(code, mc_statics_t, "currentMatch")
        .ok_or_else(|| anyhow::anyhow!("currentMatch field not found"))?;
    let matches_field = find_field(code, mc_statics_t, "_matches")
        .ok_or_else(|| anyhow::anyhow!("_matches field not found"))?;
    let match_t = find_type(code, "pxf.core.Match")
        .ok_or_else(|| anyhow::anyhow!("pxf.core.Match type not found"))?;
    eprintln!("query: $MatchController statics t={mc_statics_t} currentMatch field={cm_field} Match t={match_t}");
    // Reveal-the-match plumbing: the match renders in CoreEngine.gameContainer
    // (added by the always-subscribed gameStarted handler); menuContainer is a
    // sibling painted on top. Hiding menuContainer's h2d display object reveals
    // the match — non-destructively, so currentMatch / the live match stay intact.
    let core_statics_t = code.globals[3458].0; // pxf.core.$CoreEngine statics
    let menuc_field = find_field(code, core_statics_t, "menuContainer")
        .ok_or_else(|| anyhow::anyhow!("menuContainer field not found"))?;
    let container_t = find_type(code, "pxf.display.Container")
        .ok_or_else(|| anyhow::anyhow!("pxf.display.Container type not found"))?;
    let dispobj_field = find_field(code, container_t, "displayObject")
        .ok_or_else(|| anyhow::anyhow!("displayObject field not found"))?;
    let h2dobj_t = find_type(code, "h2d.Object")
        .ok_or_else(|| anyhow::anyhow!("h2d.Object type not found"))?;
    let visible_field = find_field(code, h2dobj_t, "visible")
        .ok_or_else(|| anyhow::anyhow!("h2d.Object.visible field not found"))?;
    eprintln!("reveal: CoreEngine t={core_statics_t} menuContainer={menuc_field} Container t={container_t} displayObject={dispobj_field} h2d.Object t={h2dobj_t} visible={visible_field}");

    // Console (Tildebugger) passthrough: Tildebugger.console.runCommand(cmd).
    // ImprovedConsole extends h2d.Console, so the field works as the receiver.
    let run_command = require_fn(code, "runCommand", Some("h2d.Console"))?;
    let set_enabled = require_fn(code, "set_enabled", Some("pxf.core.ImprovedConsole"))?;
    let tilde_global = 3456usize; // pxf.core.$Tildebugger static (TODO: resolve by name)
    let tilde_t = code.globals[tilde_global].0;
    let console_field = find_field(code, tilde_t, "console")
        .ok_or_else(|| anyhow::anyhow!("Tildebugger.console field not found"))?;
    let console_t = code.types[tilde_t]
        .get_type_obj()
        .and_then(|o| o.fields.get(console_field))
        .map(|f| f.t.0)
        .ok_or_else(|| anyhow::anyhow!("console field type missing"))?;
    eprintln!("runCommand={run_command} tilde_t={tilde_t} console_field={console_field} console_t={console_t}");
    // exact type of String.bytes (field 0) — for the GetI8 char-loop register
    let bytes_t = code.types[str_t]
        .get_type_obj()
        .and_then(|o| o.fields.first())
        .map(|fld| fld.t.0)
        .ok_or_else(|| anyhow::anyhow!("String.bytes field missing"))?;
    eprintln!("String.bytes type = {bytes_t}");
    // Read chars via the engine's own charCodeAt(String,i32)->null<i32> (a real
    // function call; raw GetI16/GetI8 opcodes crash when we emit them).
    let char_code_at = require_fn(code, "charCodeAt", Some("String"))?;
    let nulli32_t = {
        let ci = function_index_by_findex(code, char_code_at)
            .ok_or_else(|| anyhow::anyhow!("charCodeAt fn missing"))?;
        code.types[code.functions[ci].t.0]
            .get_type_fun()
            .map(|tf| tf.ret.0)
            .ok_or_else(|| anyhow::anyhow!("charCodeAt has no return type"))?
    };
    eprintln!("charCodeAt={char_code_at} ret(null<i32>)={nulli32_t}");

    // Persistent global for the socket (survives past the frame; the poll loop
    // reads it later) + a once-guard bool so the connect runs a single time.
    let g_sock = code.globals.len();
    code.globals.push(hlbc::types::RefType(sock_t));
    let g_done = code.globals.len();
    code.globals.push(hlbc::types::RefType(7)); // Bool
    // ready flag: set true by MainMenu.__constructor__ (content load complete).
    // Commands are not processed until this is set (they buffer in TCP).
    let g_ready = code.globals.len();
    code.globals.push(hlbc::types::RefType(7)); // Bool
    // one-shot guard: once a match goes live we tear down the leftover menu stack
    // (MainMenu/Local Play/etc.) so the match scene underneath becomes visible.
    let g_shown = code.globals.len();
    code.globals.push(hlbc::types::RefType(7)); // Bool
    // line-command buffer: "startMatch" takes runtime args (char/stage/assist) sent
    // over the socket. We accumulate the arg bytes into a haxe.io.Bytes scratch
    // buffer, then getString() it into a real String to split + parse.
    let bytes_t2 = require_type(code, "haxe.io.Bytes")?;
    let g_buf = code.globals.len();
    code.globals.push(hlbc::types::RefType(bytes_t2)); // haxe.io.Bytes
    let g_blen = code.globals.len();
    code.globals.push(hlbc::types::RefType(3)); // Int
    // one-shot guard: the raw socket read currently re-delivers the buffered 's'
    // line every frame (consumption quirk under investigation), so gate startMatch
    // to a single launch per engine run (the harness relaunches per test anyway).
    let g_launched = code.globals.len();
    code.globals.push(hlbc::types::RefType(7)); // Bool

    // Inject into the per-frame update (post-boot), NOT onLoaded: networking
    // string/host calls SIGSEGV during early config-load, but are fine once the
    // engine is fully up. The connect runs once (guard); the recv runs every frame.
    let update_fx = require_fn(code, "update", Some("fraymakers.Main"))?;
    let fidx = function_index_by_findex(code, update_fx)
        .ok_or_else(|| anyhow::anyhow!("update@{update_fx} not found"))?;
    let f = &mut code.functions[fidx];

    // regs 0-13: done,true,sock,host,port,out,ret,ip,byte,blockf,sock2,handle,c,zero
    // 14 str, 15 enc(unused), 16 len, 17 bytes(hl.Bytes=14), 18 sidx, 19 one, 20 off, 21 ch
    // ...22 nulli32_t 23 tilde 24 console | launch regs:
    // 25 ref0(669) 26 ref1(669) 27 pdyn(4366) 28 dynv(9) 29 pvirt(1957) 30 player(2536)
    // 31 typev(8) 32 natarr(11) 33 arr(38) 34 cfgdyn(4366) 35 cfgvirt(675) 36 config(668)
    // 37 settings(6738) 38 nullstr(13) 39 int(3)
    // ...40 matchRules value (dmr_field_t) 41 $MatchSettings statics (ms_statics_t) 42 assist ref(669)
    // 43 $MatchController statics (mc_statics_t) 44 currentMatch (match_t)
    // 45 CoreEngine statics (core_statics_t) 46 menuContainer (container_t) 47 displayObject (h2dobj_t)
    // 48 mode (mode_t=3522) 49 modeConfig virtual (1194) 50 startMatch config virtual (4482)
    // 51 Bytes scratch (bytes_t2) 52 ArrayObj split parts (38) 53 String scratch (delim/arg) 54 dyn element
    // 55 name 56 pkgid 57 candidate 58 resId (all String) 59 null<i32> 60 PXFResource (existence check)
    // 61 keys-iterator 62 key(String) 63 contentMap(StringMap) 64 bool 65 RM statics
    let base = add_regs(f, &[7, 7, sock_t, host_t, 3, out_t, 0, 3, 3, 7, sock_t, handle_t, 3, 3, str_t, enc_t, 3, bytes_t, 3, 3, 3, 3, nulli32_t, tilde_t, console_t, 669, 669, 4366, 9, 1957, 2536, 8, 11, 38, 4366, 675, 668, 6738, 13, 3, dmr_field_t, ms_statics_t, 669, mc_statics_t, match_t, core_statics_t, container_t, h2dobj_t, mode_t, 1194, 4482, bytes_t2, 38, str_t, 0, str_t, str_t, str_t, str_t, nulli32_t2, pxfres_t, keysiter_t, str_t, stringmap_t, 7, rm_statics_t]);
    let rr = |i: u32| Reg(base + i);
    let (r_done, r_true, r_sock, r_host, r_port, r_out, r_ret, r_ip, r_byte, r_blockf, r_sock2, r_handle, r_c, r_zero) =
        (rr(0), rr(1), rr(2), rr(3), rr(4), rr(5), rr(6), rr(7), rr(8), rr(9), rr(10), rr(11), rr(12), rr(13));

    use hlbc::types::{RefFun, RefField, RefInt, RefGlobal, ValBool};
    // Emit a content-id resolver into `ops`: reads the String in reg `name`, writes
    // a content-ref (669) into reg `out`. If the name already has "::" it's parsed
    // as a full id; otherwise it's expanded to package.id (bare "x" -> "x.x") and
    // each namespace prefix is tried, picking the first whose resource exists.
    // Scratch: rr16,39(int) rr53,56,57,58(String) rr59(null<i32>) rr60(PXFResource); rr38 must be null.
    let nsp = ns_prefixes.clone();
    // cmap_field selects which per-type StringMap to search for a bare id
    // (characterPxfContentMap / assistPxfContentMap / stagePxfContentMap).
    let emit_resolve = |ops: &mut Vec<Opcode>, name: u32, out: u32, cmap_field: usize| {
        let r = |i: u32| Reg(base + i);
        // "::" present? -> full id
        ops.push(Opcode::GetGlobal { dst: r(53), global: RefGlobal(colon2_g) });
        ops.push(Opcode::Null { dst: r(59) });
        ops.push(Opcode::Call3 { dst: r(16), fun: RefFun(indexof_fn), arg0: r(name), arg1: r(53), arg2: r(59) });
        ops.push(Opcode::Int { dst: r(39), ptr: RefInt(zero_idx) });
        let j_full = ops.len();
        ops.push(Opcode::JSGte { a: r(16), b: r(39), offset: 0 });
        // "." present? -> use name as pkgid (prefix path); else registry-search by bare id
        ops.push(Opcode::GetGlobal { dst: r(53), global: RefGlobal(dot_g) });
        ops.push(Opcode::Null { dst: r(59) });
        ops.push(Opcode::Call3 { dst: r(16), fun: RefFun(indexof_fn), arg0: r(name), arg1: r(53), arg2: r(59) });
        ops.push(Opcode::Int { dst: r(39), ptr: RefInt(zero_idx) });
        let j_pkgname = ops.len();
        ops.push(Opcode::JSGte { a: r(16), b: r(39), offset: 0 });
        // BARE NAME (no "." ): the registry-search loop below hangs (iterator
        // semantics bug), so skip it and use prefix-expansion (x -> <ns>::x.x),
        // which resolves characters/stages reliably. Bare assists (package != id)
        // must be given in `package.id` form. Registry search kept below (dead)
        // for later revival via a non-hanging iteration.
        let j_skipreg = ops.len();
        ops.push(Opcode::JAlways { offset: 0 });                         // -> RS_NOTFOUND (prefix path)
        // ---- registry search: scan poolHash, find a resource whose cmap has `name` ----
        ops.push(Opcode::GetGlobal { dst: r(65), global: RefGlobal(3508) });
        ops.push(Opcode::Field { dst: r(63), obj: r(65), field: RefField(poolhash_field) });
        ops.push(Opcode::Call1 { dst: r(61), fun: RefFun(sm_keys), arg0: r(63) });
        let rs_loop = ops.len();
        ops.push(Opcode::CallMethod { dst: r(64), field: RefField(0), args: vec![r(61)] }); // hasNext
        let j_notfound = ops.len();
        ops.push(Opcode::JFalse { cond: r(64), offset: 0 });            // -> RS_NOTFOUND
        ops.push(Opcode::CallMethod { dst: r(62), field: RefField(1), args: vec![r(61)] }); // next -> key
        ops.push(Opcode::Call1 { dst: r(60), fun: RefFun(getpxf_fn), arg0: r(62) });
        let j_res_ok = ops.len();
        ops.push(Opcode::JNotNull { reg: r(60), offset: 0 });           // resource ok -> check cmap
        let j_back1 = ops.len();
        ops.push(Opcode::JAlways { offset: 0 });                        // null resource -> loop
        let l_check_cmap = ops.len();
        ops.push(Opcode::Field { dst: r(63), obj: r(60), field: RefField(cmap_field) });
        let j_cmap_ok = ops.len();
        ops.push(Opcode::JNotNull { reg: r(63), offset: 0 });           // cmap ok -> exists
        let j_back2 = ops.len();
        ops.push(Opcode::JAlways { offset: 0 });                        // null cmap -> loop
        let l_check_exists = ops.len();
        ops.push(Opcode::Call2 { dst: r(64), fun: RefFun(sm_exists), arg0: r(63), arg1: r(name) });
        let j_found = ops.len();
        ops.push(Opcode::JTrue { cond: r(64), offset: 0 });             // found -> build ref
        let j_back3 = ops.len();
        ops.push(Opcode::JAlways { offset: 0 });                        // not in this resource -> loop
        // FOUND: out = parseResId(key + "." + name)
        let l_found = ops.len();
        ops.push(Opcode::GetGlobal { dst: r(53), global: RefGlobal(dot_g) });
        ops.push(Opcode::Call2 { dst: r(57), fun: RefFun(str_add), arg0: r(62), arg1: r(53) });
        ops.push(Opcode::Call2 { dst: r(57), fun: RefFun(str_add), arg0: r(57), arg1: r(name) });
        ops.push(Opcode::Call2 { dst: r(out), fun: RefFun(18224), arg0: r(57), arg1: r(38) });
        let j_found_done = ops.len();
        ops.push(Opcode::JAlways { offset: 0 });                        // -> DONE
        // RS_NOTFOUND: pkgid = name + "." + name, then prefix path
        let l_notfound = ops.len();
        ops.push(Opcode::GetGlobal { dst: r(53), global: RefGlobal(dot_g) });
        ops.push(Opcode::Call2 { dst: r(56), fun: RefFun(str_add), arg0: r(name), arg1: r(53) });
        ops.push(Opcode::Call2 { dst: r(56), fun: RefFun(str_add), arg0: r(56), arg1: r(name) });
        let j_to_prefix = ops.len();
        ops.push(Opcode::JAlways { offset: 0 });                        // -> L_PREFIX
        // L_PKGNAME: pkgid = name (caller already gave package.id)
        let l_pkgname = ops.len();
        ops.push(Opcode::Mov { dst: r(56), src: r(name) });
        // L_PREFIX: try each namespace prefix with pkgid in r56.
        // (Reverted to the ORIGINAL accept-on-resource-non-null logic: the
        // content-map-aware variant regressed launching — LAUNCHED 1->0, grep-
        // verified — likely a jump-offset bug in the added per-prefix branches.
        // Baseline behavior restored: launches, then crashes at spawnPlayer on the
        // null characterPxfContentMap for global:: stubs. The namespace fix must
        // be redone carefully with op-index tracing; see docs/INTEGRITY_HALT.md +
        // docs/ENGINE_RE_MAP_qoracle.md. cmap_field stays in the signature for the
        // eventual correct fix.)
        let _ = cmap_field;
        let l_prefix = ops.len();
        let mut found_pref = vec![];
        let n = nsp.len();
        for (k, &pref) in nsp.iter().enumerate() {
            ops.push(Opcode::GetGlobal { dst: r(53), global: RefGlobal(pref) });
            ops.push(Opcode::Call2 { dst: r(57), fun: RefFun(str_add), arg0: r(53), arg1: r(56) });
            ops.push(Opcode::Call2 { dst: r(out), fun: RefFun(18224), arg0: r(57), arg1: r(38) });
            if k + 1 < n {
                ops.push(Opcode::Call1 { dst: r(58), fun: RefFun(getresid_fn), arg0: r(out) });
                ops.push(Opcode::Call1 { dst: r(60), fun: RefFun(getpxf_fn), arg0: r(58) });
                let jf = ops.len();
                ops.push(Opcode::JNotNull { reg: r(60), offset: 0 });
                found_pref.push(jf);
            }
        }
        let j_nsdone = ops.len();
        ops.push(Opcode::JAlways { offset: 0 });
        // L_FULL
        let l_full = ops.len();
        ops.push(Opcode::Call2 { dst: r(out), fun: RefFun(18224), arg0: r(name), arg1: r(38) });
        let l_done = ops.len();
        let set = |ops: &mut Vec<Opcode>, at: usize, tgt: usize| {
            let off = tgt as i32 - at as i32 - 1;
            match &mut ops[at] {
                Opcode::JSGte { offset, .. } | Opcode::JFalse { offset, .. }
                | Opcode::JTrue { offset, .. } | Opcode::JNotNull { offset, .. }
                | Opcode::JNull { offset, .. }
                | Opcode::JAlways { offset, .. } => *offset = off,
                _ => unreachable!(),
            }
        };
        set(ops, j_full, l_full);
        set(ops, j_pkgname, l_pkgname);
        set(ops, j_skipreg, l_notfound);
        set(ops, j_notfound, l_notfound);
        set(ops, j_res_ok, l_check_cmap);
        set(ops, j_back1, rs_loop);
        set(ops, j_cmap_ok, l_check_exists);
        set(ops, j_back2, rs_loop);
        set(ops, j_found, l_found);
        set(ops, j_back3, rs_loop);
        set(ops, j_found_done, l_done);
        set(ops, j_to_prefix, l_prefix);
        set(ops, j_nsdone, l_done);
        for jf in found_pref { set(ops, jf, l_done); }
    };
    // ---- once-guard + connect + auth handshake (runs a single time) ----
    let mut ops = vec![
        Opcode::GetGlobal { dst: r_done, global: RefGlobal(g_done) },   // 0
        Opcode::JTrue { cond: r_done, offset: 0 },                      // 1 -> L_RECV (patched)
        Opcode::Bool { dst: r_true, value: ValBool(true) },             // 2
        Opcode::SetGlobal { global: RefGlobal(g_done), src: r_true },   // 3
        Opcode::Call0 { dst: r_ret, fun: RefFun(socket_init) },
        Opcode::New { dst: r_sock },
        Opcode::Call1 { dst: r_ret, fun: RefFun(sock_init_method), arg0: r_sock },
        Opcode::New { dst: r_host },
        Opcode::Int { dst: r_ip, ptr: RefInt(ip_const) },
        Opcode::SetField { obj: r_host, field: RefField(ip_field), src: r_ip },
        Opcode::Int { dst: r_port, ptr: RefInt(port_idx) },
        Opcode::Call3 { dst: r_ret, fun: RefFun(connect), arg0: r_sock, arg1: r_host, arg2: r_port },
        Opcode::SetGlobal { global: RefGlobal(g_sock), src: r_sock },
        // setBlocking(false) so the per-frame recv never blocks the render loop
        Opcode::Bool { dst: r_blockf, value: ValBool(false) },
        Opcode::Call2 { dst: r_ret, fun: RefFun(set_blocking), arg0: r_sock, arg1: r_blockf },
        Opcode::Field { dst: r_out, obj: r_sock, field: RefField(out_field) },
    ];
    // Send AUTH/HELLO from a real String via a char-loop (writeByte of s.bytes[i*2])
    // — avoids the crashing Bytes.ofString/utf16_to_utf8 native.
    let _ = (r_byte, write_str); // (kept for reference; unused now)
    // Send AUTH/HELLO via writeString of a REAL String object (GetGlobal of a
    // string constant). Now that the String is valid, ofString should work.
    let _ = (one_idx, bytes_t, char_code_at, nulli32_t);
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(hello_g) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    // allocate the line-command buffer once (haxe.io.Bytes.alloc(512))
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(buf_cap_idx) });
    ops.push(Opcode::Call1 { dst: rr(51), fun: RefFun(bytes_alloc), arg0: rr(16) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_buf), src: rr(51) });

    // ---- per-frame receive (runs every frame, but only once load-complete) ----
    let idx_recv = ops.len();
    // gate: if !g_ready (MainMenu not constructed yet) -> skip (cmds buffer in TCP)
    ops.push(Opcode::GetGlobal { dst: rr(9), global: RefGlobal(g_ready) });
    let idx_jready = ops.len();
    ops.push(Opcode::JFalse { cond: rr(9), offset: 0 });               // not ready -> L_ORIG
    // reveal-once: the mode's MATCH_STARTED flow removes the menu *background*, but
    // the menu *screens* (MainMenu) remain because we launched the mode without the
    // menu navigation that normally pops them. Once a match is live, destroy the
    // leftover active menu screens. (The match is mode-owned now, so this no longer
    // orphans it the way it did with the bare MatchController path.)
    ops.push(Opcode::GetGlobal { dst: rr(9), global: RefGlobal(g_shown) });
    let idx_jshown = ops.len();
    ops.push(Opcode::JTrue { cond: rr(9), offset: 0 });                // already done -> recv
    ops.push(Opcode::GetGlobal { dst: rr(43), global: RefGlobal(3511) });
    ops.push(Opcode::Field { dst: rr(44), obj: rr(43), field: RefField(cm_field) }); // currentMatch
    let idx_jnomatch = ops.len();
    ops.push(Opcode::JNull { reg: rr(44), offset: 0 });                // no match yet -> recv
    ops.push(Opcode::Call0 { dst: r_ret, fun: RefFun(19543) });        // destroyAllActiveMenus
    ops.push(Opcode::Bool { dst: rr(1), value: ValBool(true) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_shown), src: rr(1) });
    let idx_after_reveal = ops.len();
    if let Opcode::JTrue { offset, .. } = &mut ops[idx_jshown] { *offset = idx_after_reveal as i32 - idx_jshown as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_jnomatch] { *offset = idx_after_reveal as i32 - idx_jnomatch as i32 - 1; }
    let _ = (menuc_field, dispobj_field, visible_field);
    ops.push(Opcode::GetGlobal { dst: r_sock2, global: RefGlobal(g_sock) });
    let idx_jnull = ops.len();
    ops.push(Opcode::JNull { reg: r_sock2, offset: 0 });                // not connected -> L_ORIG
    ops.push(Opcode::Field { dst: r_handle, obj: r_sock2, field: RefField(s_field) });
    ops.push(Opcode::Call1 { dst: r_c, fun: RefFun(recv_char), arg0: r_handle });
    ops.push(Opcode::Int { dst: r_zero, ptr: RefInt(zero_idx) });
    let idx_jslt = ops.len();
    ops.push(Opcode::JSLt { a: r_c, b: r_zero, offset: 0 });            // no data (c<0) -> L_ORIG
    let _ = write_byte;
    // dispatch: 'p' -> PONG ; 'c' -> runCommand(console,"help") + "RAN"
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(p_idx) });
    let idx_jne_p = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });          // not 'p' -> 'c' check
    ops.push(Opcode::Field { dst: r_out, obj: r_sock2, field: RefField(out_field) });
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(pong_g) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    // 'c' check (c=='p' falls through here and skips, since c!='c')
    let idx_c_check = ops.len();
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(c_idx) });
    let idx_jne_c = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });          // not 'c' -> L_ORIG
    ops.push(Opcode::GetGlobal { dst: rr(23), global: RefGlobal(tilde_global) });
    ops.push(Opcode::Field { dst: rr(24), obj: rr(23), field: RefField(console_field) });
    // enable the console first so its log display exists (handleCommand echoes to it)
    ops.push(Opcode::Bool { dst: rr(9), value: ValBool(true) });
    ops.push(Opcode::Call2 { dst: rr(1), fun: RefFun(set_enabled), arg0: rr(24), arg1: rr(9) });
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(help_g) });
    ops.push(Opcode::Call2 { dst: r_ret, fun: RefFun(run_command), arg0: rr(24), arg1: rr(14) });
    ops.push(Opcode::Field { dst: r_out, obj: r_sock2, field: RefField(out_field) });
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(ran_g) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });

    // 's' check -> build a full MatchSettings (1 player on thespire) + startMatch.
    let idx_s_check = ops.len();
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(s_idx) });
    let idx_jne_s = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });          // not 's' -> L_ORIG
    use hlbc::types::{RefString as RS, RefType as RT};
    // one-shot guard: the non-consuming socket read re-delivers the line every
    // frame, so only launch once per engine run (skip to q-check thereafter).
    ops.push(Opcode::GetGlobal { dst: rr(9), global: RefGlobal(g_launched) });
    let idx_jlaunched = ops.len();
    ops.push(Opcode::JTrue { cond: rr(9), offset: 0 });                // already launched -> q_check
    // refs: parseResourceIdentifier(fullId, null) -> content-ref@669
    ops.push(Opcode::Null { dst: rr(38) });                            // null namespace
    // ---- read the rest of the line ("s <char> <stage> <assist>") into g_buf ----
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(zero_idx) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_blen), src: rr(39) });
    let idx_s_drain = ops.len();
    ops.push(Opcode::Call1 { dst: r_c, fun: RefFun(recv_char), arg0: r_handle });
    ops.push(Opcode::Int { dst: r_zero, ptr: RefInt(zero_idx) });
    let idx_s_jslt = ops.len();
    ops.push(Opcode::JSLt { a: r_c, b: r_zero, offset: 0 });            // no more data -> S_BUILD
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(nl_idx) });
    let idx_s_jeq = ops.len();
    ops.push(Opcode::JEq { a: r_c, b: rr(16), offset: 0 });             // '\n' -> S_BUILD
    ops.push(Opcode::GetGlobal { dst: rr(51), global: RefGlobal(g_buf) });
    ops.push(Opcode::GetGlobal { dst: rr(39), global: RefGlobal(g_blen) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(bytes_set), arg0: rr(51), arg1: rr(39), arg2: r_c });
    ops.push(Opcode::GetGlobal { dst: rr(39), global: RefGlobal(g_blen) });
    ops.push(Opcode::Incr { dst: rr(39) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_blen), src: rr(39) });
    let idx_s_jback = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                            // -> S_DRAIN
    let idx_s_build = ops.len();
    // line = g_buf.getString(0, g_blen, null)
    ops.push(Opcode::GetGlobal { dst: rr(51), global: RefGlobal(g_buf) });
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(zero_idx) });
    ops.push(Opcode::GetGlobal { dst: rr(16), global: RefGlobal(g_blen) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call4 { dst: rr(14), fun: RefFun(bytes_getstring), arg0: rr(51), arg1: rr(39), arg2: rr(16), arg3: rr(15) });
    // parts = line.split(" ")  -> ArrayObj ; partsArr = parts.array (native, type 11)
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(space_g) });
    ops.push(Opcode::Call2 { dst: rr(52), fun: RefFun(str_split), arg0: rr(14), arg1: rr(53) });
    ops.push(Opcode::Field { dst: rr(32), obj: rr(52), field: RefField(1) }); // parts.array
    // ---- create a real TrainingMode: createMode({ resource: <trainingmode ref> }) ----
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(mode_fullid) });
    ops.push(Opcode::Call2 { dst: rr(25), fun: RefFun(18224), arg0: rr(14), arg1: rr(38) }); // mode resource ref
    ops.push(Opcode::New { dst: rr(34) });                             // modeConfig dynobj
    ops.push(Opcode::DynSet { obj: rr(34), field: RS(resource_str), src: rr(25) });
    ops.push(Opcode::ToVirtual { dst: rr(49), src: rr(34) });          // -> modeConfig virtual@1194
    ops.push(Opcode::Call1 { dst: rr(48), fun: RefFun(create_mode), arg0: rr(49) }); // FraymakersMode
    // ---- char/stage/assist refs: parse from args if 4+ tokens, else baked-in defaults ----
    ops.push(Opcode::Field { dst: rr(16), obj: rr(52), field: RefField(0) }); // parts.length
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(four_idx) });
    let idx_s_jconst = ops.len();
    ops.push(Opcode::JSLt { a: rr(16), b: rr(39), offset: 0 });         // <4 tokens -> L_CONST
    // L_ARGS: char=resolve(parts[1]), stage=resolve(parts[2]), assist=resolve(parts[3]).
    // GetArray reads into a String reg (split backing is NativeArray<String>); the
    // resolver expands bare names + tries namespaces.
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(one_idx) });
    ops.push(Opcode::GetArray { dst: rr(55), array: rr(32), index: rr(39) });
    emit_resolve(&mut ops, 55, 26, char_cmap_field); // char ref
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(two_idx) });
    ops.push(Opcode::GetArray { dst: rr(55), array: rr(32), index: rr(39) });
    emit_resolve(&mut ops, 55, 25, stage_cmap_field); // stage ref
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(three_idx) });
    ops.push(Opcode::GetArray { dst: rr(55), array: rr(32), index: rr(39) });
    emit_resolve(&mut ops, 55, 42, assist_cmap_field); // assist ref
    let idx_s_jargsdone = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                            // -> L_AFTERREFS
    let idx_s_const = ops.len();
    // L_CONST: baked-in defaults
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(char_fullid) });
    ops.push(Opcode::Call2 { dst: rr(26), fun: RefFun(18224), arg0: rr(14), arg1: rr(38) });
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(stage_fullid) });
    ops.push(Opcode::Call2 { dst: rr(25), fun: RefFun(18224), arg0: rr(14), arg1: rr(38) });
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(assist_fullid) });
    ops.push(Opcode::Call2 { dst: rr(42), fun: RefFun(18224), arg0: rr(14), arg1: rr(38) });
    let idx_s_afterrefs = ops.len();
    // patch the line-reader + ref-source jumps
    if let Opcode::JSLt { offset, .. } = &mut ops[idx_s_jslt] { *offset = idx_s_build as i32 - idx_s_jslt as i32 - 1; }
    if let Opcode::JEq { offset, .. } = &mut ops[idx_s_jeq] { *offset = idx_s_build as i32 - idx_s_jeq as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_s_jback] { *offset = idx_s_drain as i32 - idx_s_jback as i32 - 1; }
    if let Opcode::JSLt { offset, .. } = &mut ops[idx_s_jconst] { *offset = idx_s_const as i32 - idx_s_jconst as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_s_jargsdone] { *offset = idx_s_afterrefs as i32 - idx_s_jargsdone as i32 - 1; }
    // ---- player config data { character: rr26, assist: rr42, port: 0 } ----
    ops.push(Opcode::New { dst: rr(27) });
    ops.push(Opcode::DynSet { obj: rr(27), field: RS(character_str), src: rr(26) });
    ops.push(Opcode::DynSet { obj: rr(27), field: RS(assist_str), src: rr(42) });
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(zero_idx) });
    ops.push(Opcode::ToDyn { dst: rr(28), src: rr(39) });
    ops.push(Opcode::DynSet { obj: rr(27), field: RS(port_str), src: rr(28) });
    ops.push(Opcode::ToVirtual { dst: rr(29), src: rr(27) });          // -> player virtual@1957
    // characters = [playerVirtual]
    ops.push(Opcode::Type { dst: rr(31), ty: RT(1957) });
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(one_idx) });
    ops.push(Opcode::Call2 { dst: rr(32), fun: RefFun(256), arg0: rr(31), arg1: rr(39) }); // alloc_array
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(zero_idx) });
    ops.push(Opcode::SetArray { array: rr(32), index: rr(39), src: rr(29) });
    ops.push(Opcode::Call1 { dst: rr(33), fun: RefFun(257), arg0: rr(32) });   // wrap -> ArrayObj
    // matchSettings = { stage: rr25, matchRules: defaultMatchRules } (virtual@675)
    ops.push(Opcode::New { dst: rr(34) });
    ops.push(Opcode::DynSet { obj: rr(34), field: RS(stage_str), src: rr(25) });
    ops.push(Opcode::GetGlobal { dst: rr(41), global: RefGlobal(ms_global) });
    ops.push(Opcode::Field { dst: rr(40), obj: rr(41), field: RefField(dmr_field) });
    ops.push(Opcode::DynSet { obj: rr(34), field: RS(matchrules_str), src: rr(40) });
    ops.push(Opcode::ToVirtual { dst: rr(35), src: rr(34) });          // -> matchSettings virtual@675
    // config = { characters, matchSettings, pauseMenu: null } (virtual@4482)
    ops.push(Opcode::New { dst: rr(27) });                             // dynobj (4366)
    ops.push(Opcode::DynSet { obj: rr(27), field: RS(characters_str), src: rr(33) });
    ops.push(Opcode::DynSet { obj: rr(27), field: RS(matchsettings_str), src: rr(35) });
    ops.push(Opcode::DynSet { obj: rr(27), field: RS(pausemenu_str), src: rr(38) }); // pauseMenu = null
    ops.push(Opcode::ToVirtual { dst: rr(50), src: rr(27) });          // -> config virtual@4482
    // mode.startMatch(config)  — runs the engine's offline-match flow (gates, menu suspend/restore)
    ops.push(Opcode::Call2 { dst: r_ret, fun: RefFun(mode_start_match), arg0: rr(48), arg1: rr(50) });
    let _ = (one_idx, launched_g);
    // ack: "LAUNCHED <charId> <stageId> <assistId>\n" — echoes resolved content ids
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(launched2_g) });
    ops.push(Opcode::Call1 { dst: rr(53), fun: RefFun(getcontentid_fn), arg0: rr(26) }); // char id
    ops.push(Opcode::Call2 { dst: rr(14), fun: RefFun(str_add), arg0: rr(14), arg1: rr(53) });
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(space_g) });
    ops.push(Opcode::Call2 { dst: rr(14), fun: RefFun(str_add), arg0: rr(14), arg1: rr(53) });
    ops.push(Opcode::Call1 { dst: rr(53), fun: RefFun(getcontentid_fn), arg0: rr(25) }); // stage id
    ops.push(Opcode::Call2 { dst: rr(14), fun: RefFun(str_add), arg0: rr(14), arg1: rr(53) });
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(space_g) });
    ops.push(Opcode::Call2 { dst: rr(14), fun: RefFun(str_add), arg0: rr(14), arg1: rr(53) });
    ops.push(Opcode::Call1 { dst: rr(53), fun: RefFun(getcontentid_fn), arg0: rr(42) }); // assist id
    ops.push(Opcode::Call2 { dst: rr(14), fun: RefFun(str_add), arg0: rr(14), arg1: rr(53) });
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(nl_g) });
    ops.push(Opcode::Call2 { dst: rr(14), fun: RefFun(str_add), arg0: rr(14), arg1: rr(53) });
    ops.push(Opcode::Field { dst: r_out, obj: r_sock2, field: RefField(out_field) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });

    // 'q' query -> report whether MatchController.currentMatch is live.
    let idx_q_check = ops.len();
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(q_idx) });
    let idx_jne_q = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });          // not 'q' -> L_ORIG
    ops.push(Opcode::Field { dst: r_out, obj: r_sock2, field: RefField(out_field) });
    ops.push(Opcode::GetGlobal { dst: rr(43), global: RefGlobal(3511) });
    ops.push(Opcode::Field { dst: rr(44), obj: rr(43), field: RefField(cm_field) }); // currentMatch
    let idx_q_jnull = ops.len();
    ops.push(Opcode::JNull { reg: rr(44), offset: 0 });                 // null -> NO_MATCH write
    // live:
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(q_live_g) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    let idx_q_jdone = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                            // -> L_ORIG
    // currentMatch null: check _matches.length to tell "match exists" from "none".
    let idx_q_nomatch = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(43), global: RefGlobal(3511) });
    ops.push(Opcode::Field { dst: rr(45), obj: rr(43), field: RefField(matches_field) }); // _matches (ArrayObj)
    let idx_q_jm_null = ops.len();
    ops.push(Opcode::JNull { reg: rr(45), offset: 0 });                 // _matches null -> NO_MATCH
    ops.push(Opcode::Field { dst: rr(46), obj: rr(45), field: RefField(0) }); // ArrayObj.length
    ops.push(Opcode::Int { dst: rr(47), ptr: RefInt(zero_idx) });
    let idx_q_jm_empty = ops.len();
    ops.push(Opcode::JSLte { a: rr(46), b: rr(47), offset: 0 });        // length<=0 -> NO_MATCH
    // _matches non-empty: a Match object exists even though currentMatch is null.
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(q_matches_g) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    let idx_q_jm_done = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                            // -> L_ORIG
    // truly no match:
    let idx_q_truly_none = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(q_nomatch_g) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });

    // L_ORIG = first original op after the prepended block (index n).
    let n = ops.len() as i32;
    if let Opcode::JTrue { offset, .. } = &mut ops[1] { *offset = idx_recv as i32 - 2; }
    if let Opcode::JFalse { offset, .. } = &mut ops[idx_jready] { *offset = n - idx_jready as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_jnull] { *offset = n - idx_jnull as i32 - 1; }
    if let Opcode::JSLt { offset, .. } = &mut ops[idx_jslt] { *offset = n - idx_jslt as i32 - 1; }
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_p] { *offset = idx_c_check as i32 - idx_jne_p as i32 - 1; }
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_c] { *offset = idx_s_check as i32 - idx_jne_c as i32 - 1; }
    // 's' falls through to the 'q' check; route 'not s' there too, then 'q' to L_ORIG.
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_s] { *offset = idx_q_check as i32 - idx_jne_s as i32 - 1; }
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_q] { *offset = n - idx_jne_q as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_q_jnull] { *offset = idx_q_nomatch as i32 - idx_q_jnull as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_q_jdone] { *offset = n - idx_q_jdone as i32 - 1; }
    // _matches diagnostic branch wiring: null/empty -> truly-none; non-empty path
    // falls through then JAlways -> L_ORIG.
    if let Opcode::JNull { offset, .. } = &mut ops[idx_q_jm_null] { *offset = idx_q_truly_none as i32 - idx_q_jm_null as i32 - 1; }
    if let Opcode::JSLte { offset, .. } = &mut ops[idx_q_jm_empty] { *offset = idx_q_truly_none as i32 - idx_q_jm_empty as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_q_jm_done] { *offset = n - idx_q_jm_done as i32 - 1; }
    insert_ops_front(f, ops);
    eprintln!("connect_edit: injected {n} ops into update@{update_fx} (console+startMatch dispatch); port={port}");

    // When content loads (Title.customContentLoaded@21850 — title now shows
    // "press any button"), auto-trigger the EXACT press-any-button handler
    // Title.start@21860 (does all the prep + transition into the menu). Appended
    // so it runs AFTER the title finishes setting itself up, like a real press.
    inject_press_start(code, 21850, 21860)?;
    // Then signal READY from MainMenu's constructor (now reached via the advance).
    let menu_ready_g = add_string_const(code, "READY\n");
    inject_ready_flag(code, 18549, g_ready, g_sock, out_field, write_str, flush, menu_ready_g, sock_t, out_t, str_t, enc_t, 17842)?;
    Ok(())
}

/// Append ops just before the final Ret of a function (so they run after the
/// original body). Existing forward jumps are unaffected (they target indices
/// before the inserted block); debug_info gets matching entries.
fn insert_ops_end(f: &mut hlbc::types::Function, ops: Vec<Opcode>) {
    let at = f.ops.len().saturating_sub(1); // before the trailing Ret
    let n = ops.len();
    for (i, op) in ops.into_iter().enumerate() {
        f.ops.insert(at + i, op);
    }
    if let Some(dbg) = f.debug_info.as_mut() {
        let fill = dbg.get(at).copied().or_else(|| dbg.last().copied()).unwrap_or((0, 0));
        for _ in 0..n {
            dbg.insert(at, fill);
        }
    }
    if let Some(assigns) = f.assigns.as_mut() {
        for (_n, pos) in assigns.iter_mut() {
            if *pos >= at {
                *pos += n;
            }
        }
    }
}

/// Append `this.<startHandler>()` to a function whose reg0 is the same object —
/// runs the real press-any-button handler (Title.start) after the original body.
fn inject_press_start(code: &mut Bytecode, fn_findex: usize, start_findex: usize) -> anyhow::Result<()> {
    let fidx = function_index_by_findex(code, fn_findex)
        .ok_or_else(|| anyhow::anyhow!("fn @{fn_findex} not found"))?;
    let f = &mut code.functions[fidx];
    let base = add_regs(f, &[0]); // void ret
    use hlbc::types::RefFun;
    insert_ops_end(f, vec![Opcode::Call1 { dst: Reg(base), fun: RefFun(start_findex), arg0: Reg(0) }]);
    eprintln!("inject_press_start: Title.start@{start_findex} appended to @{fn_findex}");
    Ok(())
}

/// Set g_ready=true at the start of MainMenu's constructor AND send a marker over
/// the harness socket so we can see when the menu is built (= content loaded).
#[allow(clippy::too_many_arguments)]
fn inject_ready_flag(
    code: &mut Bytecode, ctor_findex: usize, g_ready: usize, g_sock: usize,
    out_field: usize, write_str: usize, flush: usize, marker_g: usize,
    sock_t: usize, out_t: usize, str_t: usize, enc_t: usize,
    load_ugc: usize,
) -> anyhow::Result<()> {
    let fidx = function_index_by_findex(code, ctor_findex)
        .ok_or_else(|| anyhow::anyhow!("ctor @{ctor_findex} not found"))?;
    let f = &mut code.functions[fidx];
    let base = add_regs(f, &[7, sock_t, out_t, str_t, enc_t, 0]);
    let (r_b, r_sock, r_out, r_str, r_enc, r_ret) =
        (Reg(base), Reg(base + 1), Reg(base + 2), Reg(base + 3), Reg(base + 4), Reg(base + 5));
    use hlbc::types::{RefField, RefFun, RefGlobal};
    let mut ops = vec![
        // Kick off custom-content (UGC) loading. Our injected boot path
        // (Title.start → MainMenu) bypasses Main::launchScreen, which is what
        // normally calls UgcUtil.loadUgc; we use loadInLocalUgc@17842 (local-only, no Steam/guards) — so custom/ + workshop .fra files are
        // never added to the ResourceManager pool, and spawning a custom char
        // null-derefs characterPxfContentMap. loadUgc@argc0 scans custom/ async;
        // it's idempotent (guarded by beforeFirstLoad/activelyLoading). The
        // harness then waits (FRAY_POST_READY_DELAY) before sending `s`.
        Opcode::Call0 { dst: r_ret, fun: RefFun(load_ugc) },
        Opcode::Bool { dst: r_b, value: hlbc::types::ValBool(true) },
        Opcode::SetGlobal { global: RefGlobal(g_ready), src: r_b },
        Opcode::GetGlobal { dst: r_sock, global: RefGlobal(g_sock) },
    ];
    let idx_jnull = ops.len();
    ops.push(Opcode::JNull { reg: r_sock, offset: 0 }); // no socket -> skip send
    ops.push(Opcode::Field { dst: r_out, obj: r_sock, field: RefField(out_field) });
    ops.push(Opcode::GetGlobal { dst: r_str, global: RefGlobal(marker_g) });
    ops.push(Opcode::Null { dst: r_enc });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: r_str, arg2: r_enc });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    let n = ops.len() as i32;
    if let Opcode::JNull { offset, .. } = &mut ops[idx_jnull] { *offset = n - idx_jnull as i32 - 1; }
    let _ = (out_t, str_t, enc_t);
    insert_ops_front(f, ops);
    eprintln!("inject_ready_flag: patched ctor @{ctor_findex}");
    Ok(())
}

/// Find a constructor by matching its exact argument signature (arg0 is `this`).
/// Constructors are named "__constructor__" (or empty) with no reliable parent
/// link, so we match on arg types. Returns the function's own findex.
fn verify_ctor(code: &Bytecode, type_name: &str, arg_type_names: &[&str]) -> anyhow::Result<usize> {
    let want: Vec<usize> = arg_type_names
        .iter()
        .map(|n| if *n == "String" { Ok(13) } else { require_type(code, n) })
        .collect::<anyhow::Result<_>>()?;
    let mut found = None;
    for f in &code.functions {
        let name = s(code, f.name);
        if name != "__constructor__" && !name.is_empty() {
            continue;
        }
        if let Some(tf) = code.types[f.t.0].get_type_fun() {
            let got: Vec<usize> = tf.args.iter().map(|a| a.0).collect();
            if got == want {
                if found.is_some() {
                    anyhow::bail!("ambiguous constructor for {type_name} (args {arg_type_names:?})");
                }
                found = Some(f.findex.0);
            }
        }
    }
    found.ok_or_else(|| anyhow::anyhow!("constructor not found for {type_name} with args {arg_type_names:?}"))
}

/// M1: inject `throw "HARNESS_PROBE_OK"` at the very start of a function, to
/// prove (a) opcode injection round-trips, (b) the function executes at boot.
fn probe_edit(code: &mut Bytecode, target_findex: usize) -> anyhow::Result<()> {
    // Add our marker string to the constant pool (appended -> no index shifts).
    let marker = "HARNESS_PROBE_OK";
    let str_idx = code.strings.len();
    code.strings.push(hlbc::Str::from(marker));
    eprintln!("added string #{str_idx} = {marker:?}");

    let fidx = function_index_by_findex(code, target_findex)
        .ok_or_else(|| anyhow::anyhow!("function findex {target_findex} not found"))?;
    let f = &mut code.functions[fidx];
    eprintln!(
        "patching fn idx {fidx} (findex {}), name#{}, {} regs, {} ops",
        f.findex.0,
        f.name.0,
        f.regs.len(),
        f.ops.len()
    );

    // Reuse an existing String-typed register if present (type 13 == String),
    // else add a new one.
    let str_reg = match f.regs.iter().position(|t| t.0 == 13) {
        Some(i) => i as u32,
        None => {
            let i = f.regs.len() as u32;
            f.regs.push(hlbc::types::RefType(13));
            i
        }
    };
    eprintln!("using String reg {str_reg}");

    // Prepend: String rN = "HARNESS_PROBE_OK"; Throw rN
    let inject = vec![
        Opcode::String {
            dst: Reg(str_reg),
            ptr: RefString(str_idx),
        },
        Opcode::Throw {
            exc: Reg(str_reg),
        },
    ];
    insert_ops_front(f, inject);
    eprintln!("injected ops at front; now {} ops", f.ops.len());
    Ok(())
}
