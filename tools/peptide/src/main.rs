use std::fs::File;
use std::io::{BufReader, BufWriter};

mod asm;
#[allow(unused_imports)]
use asm::Asm;

// Shared friendly-command vocabulary. The patcher uses MOVES to generate the
// move-dispatch jump table; the bridge uses it to translate move names. One table.
mod commands;

use hlbc::opcodes::Opcode;
use hlbc::types::{Reg, RefString};
use hlbc::Bytecode;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: peptide <input.dat> <output.dat> [mode]");
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
        "methods" => {
            // list every function whose parent type name contains <needle>
            let needle = args.get(4).cloned().unwrap();
            for f in &code.functions {
                let pn = f.parent.and_then(|rt| type_name_of(&code, rt)).unwrap_or("");
                if pn.contains(&needle) {
                    eprintln!("  {:6} {}::{}", f.findex.0, pn, s(&code, f.name));
                }
            }
            return Ok(());
        }
        "connect" => {
            // peptide <in> <out> connect <port> <token> [<char> [<stage> [<assist>]]]
            // The socket bridge + command dispatch (s/t/q/x/c/p) are ALWAYS installed.
            // Providing a CHARACTER triggers HEADLESS fast-boot: skip the Title/menus
            // (no-op launchScreen) + filter the boot required-load to skip public:: base
            // content, and bake that char (+ optional stage/assist) as the launch default.
            // With NO character the game boots normally (title + full content load) and is
            // just a TCP bridge — headless does not trigger unless a character is given.
            let port: u16 = args.get(4).and_then(|s| s.parse().ok())
                .ok_or_else(|| anyhow::anyhow!("connect mode needs <port>"))?;
            let token = args.get(5).cloned().unwrap_or_default();
            let char_name = args.get(6).cloned();
            let stage_name = args.get(7).cloned();
            let assist_name = args.get(8).cloned();
            let headless = char_name.is_some();
            // install dir = parent of the boot file, used to build custom/<char>/<char>.fra
            let install_dir = std::path::Path::new(input).parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            connect_edit(&mut code, port, &token, headless, char_name, stage_name, assist_name, &install_dir)?;
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
        let lineinfo = f.debug_info.as_ref()
            .and_then(|d| d.get(i))
            .map(|(file, line)| {
                let fname = code.debug_files.as_ref()
                    .and_then(|files| files.get(*file))
                    .map(|s| s.as_str())
                    .unwrap_or("?");
                format!("  @{fname}:{line}")
            })
            .unwrap_or_default();
        eprintln!("  {i:4}:{lineinfo} {op:?}{note}");
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

// ---- headless match settings (data-file driven) -----------------------------

/// Built-in default for the headless `s`-command match settings. An on-disk
/// copy overrides this so values can be tweaked WITHOUT recompiling.
const MATCH_SETTINGS_DEFAULT: &str = include_str!("../match_settings.conf");

/// Resolve the headless match settings → `(lives, time)`. Reads the first
/// existing override file (`$PEPTIDE_MATCH_SETTINGS`, then `match_settings.conf`
/// next to the binary, then `./match_settings.conf`), falling back to the
/// baked-in default. See `tools/peptide/match_settings.conf`.
fn load_match_settings() -> (i32, i32) {
    let mut text = MATCH_SETTINGS_DEFAULT.to_string();
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("PEPTIDE_MATCH_SETTINGS") {
        candidates.push(p.into());
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // target/release/peptide → tools/peptide/match_settings.conf
            candidates.push(dir.join("../../match_settings.conf"));
            candidates.push(dir.join("match_settings.conf"));
        }
    }
    candidates.push(std::path::PathBuf::from("match_settings.conf"));
    for c in &candidates {
        if let Ok(s) = std::fs::read_to_string(c) {
            eprintln!("match-settings: loaded override {}", c.display());
            text = s;
            break;
        }
    }
    parse_match_settings(&text)
}

/// Parse `key = value` lines (with `#` comments). Missing keys keep their
/// built-in defaults (lives=999, time=0).
fn parse_match_settings(text: &str) -> (i32, i32) {
    let (mut lives, mut time) = (999i32, 0i32);
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() { continue; }
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim();
            match k.trim() {
                "lives" => if let Ok(n) = v.parse::<i32>() { lives = n; },
                "time"  => if let Ok(n) = v.parse::<i32>() { time = n; },
                _ => {}
            }
        }
    }
    (lives, time)
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
fn connect_edit(
    code: &mut Bytecode, port: u16, token: &str, headless: bool,
    char_name: Option<String>, stage_name: Option<String>, assist_name: Option<String>,
    install_dir: &str,
) -> anyhow::Result<()> {
    // Character/stage/assist baked as launch defaults. The char drives the self-bootstrap
    // (a custom .fra at <install>/custom/<char>/<char>.fra) and the bare-`s`/auto-launch
    // defaults; stage/assist default to the harness's standard pair. All are generic —
    // no hardcoded "sandbag" — derived from the injector args (sandbag is just the default).
    let cname = char_name.as_deref().unwrap_or("sandbag");
    let sname = stage_name.as_deref().unwrap_or("thespire");
    let aname = assist_name.as_deref().unwrap_or("commandervideoassist");
    let char_path = format!("{install_dir}/custom/{cname}/{cname}.fra");
    let char_pkgid = format!("{cname}.{cname}");
    let char_ns_key = format!("private::{cname}.{cname}");
    let char_resid = format!("private::{cname}");
    let stage_fqid = format!("public::{sname}.{sname}");
    let assist_fqid = format!("public::{aname}.{aname}");
    eprintln!("peptide: char={cname} stage={sname} assist={aname} headless={headless} path={char_path}");
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
    let ready_g = add_string_const(code, "READY\n"); // sent right after connect (engine is match-ready by preLoad)
    let pong_g = add_string_const(code, "PONG\n");
    let help_g = add_string_const(code, "help"); // console command to run as a test
    let ran_g = add_string_const(code, "RAN\n");
    let zero_idx = add_int(code, 0);
    let one_idx = add_int(code, 1);
    let p_idx = add_int(code, 'p' as i32); // 'ping' command byte
    let x_idx = add_int(code, 'x' as i32); // 'exit' command byte — clean engine shutdown
    // hxd.System.exit (Heaps app exit): clean process shutdown so the harness can quit the
    // engine WITHOUT `kill -9` mid-render (which leaves wedged U-state ./hl orphans on macOS).
    let hxd_exit = require_fn(code, "exit", Some("hxd.$System"))?;
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
    // Headless match settings (lives + timer) come from match_settings.conf
    // (see load_match_settings). Both fields exist in the matchSettings virtual
    // schema (t675: field 8 "lives", field 24 "time") and importJSON@5460 copies
    // them into the real MatchSettingsConfig (lives f16, time f19) during
    // _offlineMatchStart.
    let lives_str = add_string(code, "lives");
    let time_str = add_string(code, "time");
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
    let (cfg_lives, cfg_time) = load_match_settings();
    eprintln!("match-settings: lives={cfg_lives} time={cfg_time}");
    let lives_idx = add_int(code, cfg_lives);
    let time_idx = add_int(code, cfg_time);
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
    // namespace prefixes tried in order for bare names. private:: comes first so a
    // bare `s sandbag` resolves to the headless-loaded custom resource (the self-
    // bootstrap load + the `l` command both register it under private::sandbag);
    // built-in chars aren't in private::, so they harmlessly fall through to the rest.
    let ns_prefixes: Vec<usize> = ["private::", "custom::", "public::", "global::"]
        .iter().map(|p| add_string_const(code, p)).collect();
    // registry search: scan ResourceManager.pool (ArrayObj, f12) by index.
    // Index-loop is safe (no iterator-object hang). Pool is the ordered array that
    // addResource pushes every loaded resource onto (verified: addResource op44
    // hl.types.ArrayObj::push@89 on pool, md5 1b65af22). poolHash (f13) still
    // exists for getPXFResource lookups; we use pool for our safe iteration.
    let rm_statics_t = code.globals[3508].0; // pxf.io.$ResourceManager statics
    let poolhash_field = find_field(code, rm_statics_t, "poolHash")
        .ok_or_else(|| anyhow::anyhow!("poolHash field not found"))?;
    let pool_field = find_field(code, rm_statics_t, "pool")
        .ok_or_else(|| anyhow::anyhow!("pool field not found on RM statics"))?;
    // getFullyQualifiedResourceId@1788 (on AbstractResource): returns the
    // poolHash key for a given resource, so we can build the correct resolved ref.
    let getfqid_fn = require_fn(code, "getFullyQualifiedResourceId", Some("pxf.io.AbstractResource"))?;
    // get_Loaded@1839: only fully-loaded resources have gone through createFromBytes
    // and have f17 set; unloaded stubs have null f17 and would crash on Field read.
    let get_loaded_fn = require_fn(code, "get_Loaded", Some("pxf.io.AbstractResource"))?;
    // AbstractResource type (394): safe intermediate type for GetArray from pool.
    let absres_t = require_type(code, "pxf.io.AbstractResource")?;
    eprintln!("pool: pool_field={pool_field} getfqid={getfqid_fn} get_loaded={get_loaded_fn} absres_t={absres_t}");
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
    // fullIds must be REAL String objects (parseResourceIdentifier regex-matches them).
    // Baked from the injector args (bare-`s`/auto-launch defaults): char -> the
    // self-bootstrapped custom char (private::<char>), stage/assist -> public:: ids.
    let stage_fullid = add_string_const(code, &stage_fqid);
    let char_fullid = add_string_const(code, &char_resid);
    // The HUD's DamageCounter generates an assist sprite per player; a null assist
    // null-derefs .namespace in getContentIdentifierString. Supply a real assist.
    let assist_fullid = add_string_const(code, &assist_fqid);
    let assist_str = add_string(code, "assist");
    let launched_g = add_string_const(code, "LAUNCHED\n");
    let launched2_g = add_string_const(code, "LAUNCHED "); // verbose ack prefix
    let nl_g = add_string_const(code, "\n");
    let getcontentid_fn = require_fn(code, "getContentIdentifierString", Some("pxf.io.$ResourceManager"))?;
    // 'q' query command: report whether a match is live (answers "did it start?").
    let q_idx = add_int(code, 'q' as i32);
    let q_nomatch_g = add_string_const(code, "Q:NO_MATCH\n");
    let q_live_g = add_string_const(code, "Q:MATCH_LIVE\n");
    // 'k' command: dump pool keys (getFullyQualifiedResourceId for each pool element).
    // Reveals the ACTUAL namespace used by the headless UGC load path.
    let k_idx = add_int(code, 'k' as i32);
    let k_prefix_g = add_string_const(code, "K:");
    // diagnostics: did the local-UGC discovery queue/install any dirs?
    let k_dirs_pos_g = add_string_const(code, "K:DIRS_QUEUED>0\n");
    let k_dirs_zero_g = add_string_const(code, "K:DIRS_QUEUED=0\n");
    let k_inst_pos_g = add_string_const(code, "K:INSTALLED>0\n");
    let k_inst_zero_g = add_string_const(code, "K:INSTALLED=0\n");
    let _ = (k_inst_pos_g, k_inst_zero_g); // reserved for a follow-up installedUgc probe
    // UgcUtil statics g3449: directoriesToLoad=field 11 (ArrayObj),
    // installedUgc=field 10 (an object whose field 3 is an ArrayObj of items).
    let ugc_statics_g = 3449usize;
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
    // ---- move-dispatch + telemetry API (commands 'm' / 't') — resolved by name ----
    // Drive a move on the live player-0 Character via its own state machine
    // (Character.toState), and read its current state name (Character.getStateName)
    // — internal-function dispatch, NOT key-press simulation.
    let char_entity_t = require_type(code, "pxf.entity.Character")?;
    let to_state = require_fn(code, "toState", Some("pxf.entity.Character"))?;
    let get_state_name = require_fn(code, "getStateName", Some("pxf.entity.Character"))?;
    let cstate_statics_t = find_type(code, "pxf.entity.$CState")
        .ok_or_else(|| anyhow::anyhow!("pxf.entity.$CState type not found"))?;
    let cstate_global = code.globals.iter().position(|g| g.0 == cstate_statics_t)
        .ok_or_else(|| anyhow::anyhow!("$CState statics global not found"))?;
    let jab_field = find_field(code, cstate_statics_t, "JAB")
        .ok_or_else(|| anyhow::anyhow!("CState.JAB field not found"))?;
    let characters_field = find_field(code, match_t, "characters")
        .ok_or_else(|| anyhow::anyhow!("Match.characters field not found"))?;
    let m_idx = add_int(code, 'm' as i32);
    let t_idx = add_int(code, 't' as i32);
    let m_ack_g = add_string_const(code, "M:JAB\n");
    let m_ok_g = add_string_const(code, "M:OK\n"); // generic move-dispatch ack (move-by-name)
    let m_nomatch_g = add_string_const(code, "M:NOMATCH\n");
    // Move-by-name dispatch table. The client (bridge) sends `m <letter>` where the
    // selector byte = 'A' + ordinal (ordinal = index into commands::MOVES). We resolve
    // each move's CState field by NAME here (robust to findex/field drift), so the
    // engine-side jump table is GENERATED from the same shared table the client uses —
    // not hand-written per move. A move whose CState field is absent in this build
    // falls back to JAB (logged) so the dispatch never reads a bogus field.
    let space_idx = add_int(code, ' ' as i32);
    let negone_idx = add_int(code, -1);
    let move_fields: Vec<(usize, usize)> = commands::MOVES.iter().enumerate().map(|(i, (mv, fld))| {
        let fidx = find_field(code, cstate_statics_t, fld).unwrap_or_else(|| {
            eprintln!("move-dispatch: CState.{fld} (move {mv:?}) not found — falling back to JAB");
            jab_field
        });
        (add_int(code, 'A' as i32 + i as i32), fidx) // (letter int-pool idx, CState field idx)
    }).collect();
    eprintln!("move-dispatch: {} moves resolved (selector 'A'..); see commands::MOVES", move_fields.len());
    // ---- 'v' command: physics/vitals readback (criterion #6 numeric telemetry) ----
    // Reads player-0 Character body.x/y, physics.currentVelocityX/Y, damage._damage —
    // all Float (t6) — boxes each via ToDyn and formats with Std.string, then writes
    // one "P: x=.. y=.. vx=.. vy=.. dmg=..\n" line. Resolved by name (drift-robust).
    let v_idx = add_int(code, 'v' as i32);
    let std_string = find_fn(code, "string", Some("$Std"))
        .or_else(|| find_fn(code, "string", Some("Std")))
        .unwrap_or(5791); // $Std::string(Dynamic):String
    let body_t = require_type(code, "pxf.components.Body")?;
    let physics_t = require_type(code, "pxf.components.Physics")?;
    let damage_t = require_type(code, "pxf.components.Damage")?;
    let char_body_f = require_field(code, "pxf.entity.Character", "body")?;
    let char_physics_f = require_field(code, "pxf.entity.Character", "physics")?;
    let char_damage_f = require_field(code, "pxf.entity.Character", "damage")?;
    let body_x_f = find_field(code, body_t, "x").ok_or_else(|| anyhow::anyhow!("Body.x"))?;
    let body_y_f = find_field(code, body_t, "y").ok_or_else(|| anyhow::anyhow!("Body.y"))?;
    let phys_vx_f = find_field(code, physics_t, "currentVelocityX").ok_or_else(|| anyhow::anyhow!("Physics.currentVelocityX"))?;
    let phys_vy_f = find_field(code, physics_t, "currentVelocityY").ok_or_else(|| anyhow::anyhow!("Physics.currentVelocityY"))?;
    let dmg_f = find_field(code, damage_t, "_damage").ok_or_else(|| anyhow::anyhow!("Damage._damage"))?;
    let p_pre_g = add_string_const(code, "P:");
    let p_x_g = add_string_const(code, " x=");
    let p_y_g = add_string_const(code, " y=");
    let p_vx_g = add_string_const(code, " vx=");
    let p_vy_g = add_string_const(code, " vy=");
    let p_dmg_g = add_string_const(code, " dmg=");
    let p_nomatch_g = add_string_const(code, "P:NOMATCH\n");
    eprintln!("physics: Std.string={std_string} body.f={char_body_f} physics.f={char_physics_f} damage.f={char_damage_f}");
    // ---- hscript eval pipeline ('e' command): the engine bundles full hscript — the
    // same Parser + Interp that runs every character/assist script. `e` parses a script
    // string and executes it, so handler logic is readable Haxe text instead of
    // hand-emitted bytecode. SPIKE: a hardcoded script proves the in-engine pipeline;
    // the socket-driven arbitrary-script form (and engine-class registration) follow. ----
    let e_idx = add_int(code, 'e' as i32);
    let hs_parser_ctor = find_fn(code, "__constructor__", Some("hscript.$Parser")).unwrap_or(2284);
    let hs_parse = require_fn(code, "parseString", Some("hscript.Parser"))?;
    let hs_interp_ctor = find_fn(code, "__constructor__", Some("hscript.$Interp")).unwrap_or(2235);
    let hs_execute = require_fn(code, "execute", Some("hscript.Interp"))?;
    let hs_setvar = require_fn(code, "setVar", Some("hscript.Interp"))?;
    // The engine NEVER runs a bare interp: Main::init registers FraymakersScriptGlobals.
    // applyInterpreterGlobals as the globals-callback, and runs every program through
    // ApiScript.interpretScript (resets depth/declared, runs exprReturn, TRAPS errors).
    // We mirror that exactly so our top-scope interp is "loaded + ready" like the engine's.
    let hs_apply_globals = find_fn(code, "applyInterpreterGlobals", Some("fraymakers.api.$FraymakersScriptGlobals")).unwrap_or(18218);
    let hs_interp_script = find_fn(code, "interpretScript", Some("pxf.api.$ApiScript")).unwrap_or(2202);
    let eval_p0_g = add_string_const(code, "p0");  // bound to player-0 Character before each eval
    let eval_match_g = add_string_const(code, "match"); // bound to MatchController.currentMatch each eval
    let eval_chars_g = add_string_const(code, "characters"); // bound to the live character ArrayObj each eval
    // The command implementations, in hscript (ported from bytecode). Loaded ONCE into
    // the interp after applyInterpreterGlobals; every friendly command calls into these.
    let prelude_g = add_string_const(code, include_str!("../prelude.hsx"));
    let eval_cs_g = add_string_const(code, "CS");  // bound to the CState statics (move-id source)
    let hs_parser_t = require_type(code, "hscript.Parser")?;
    let hs_interp_t = require_type(code, "hscript.Interp")?;
    // hscript.Expr is an unnamed enum (t396) — not name-resolvable; index confirmed by
    // both parseString's return type and execute's 2nd arg. (Same hardcoded-index style
    // as RT(1957)/alloc_array=256 elsewhere; re-derive if the engine build changes.)
    let hs_expr_t: u32 = 396;
    let eval_script_g = add_string_const(code, "1 + 2");
    let eval_prefix_g = add_string_const(code, "E:");
    let eval_nl_g = add_string_const(code, "\n");
    eprintln!("eval: parseString={hs_parse} execute={hs_execute} Parser_t={hs_parser_t} Interp_t={hs_interp_t} Expr_t={hs_expr_t} parserCtor={hs_parser_ctor} interpCtor={hs_interp_ctor}");
    // ---- 'a' command: animation introspection (name + frame index/total) ----
    // Reads Character.animation -> currentAnimation (String) / currentFrame /
    // totalFrames (Int), writes "A:<name> frame <cur>/<total>". Lets the agent (and
    // a modder) see exactly where in an animation the character is — the observation
    // half of the drive-observe-iterate loop. Resolved by name (drift-robust).
    let a_idx = add_int(code, 'a' as i32);
    let anim_t = require_type(code, "pxf.components.Animation")?;
    let char_anim_f = require_field(code, "pxf.entity.Character", "animation")?;
    let anim_name_f = find_field(code, anim_t, "currentAnimation").ok_or_else(|| anyhow::anyhow!("Animation.currentAnimation"))?;
    let anim_cur_f = find_field(code, anim_t, "currentFrame").ok_or_else(|| anyhow::anyhow!("Animation.currentFrame"))?;
    let anim_total_f = find_field(code, anim_t, "totalFrames").ok_or_else(|| anyhow::anyhow!("Animation.totalFrames"))?;
    let a_pre_g = add_string_const(code, "A:");
    let a_frame_g = add_string_const(code, " frame ");
    let a_slash_g = add_string_const(code, "/");
    let a_nomatch_g = add_string_const(code, "A:NOMATCH\n");
    eprintln!("anim: Animation t={anim_t} char.anim.f={char_anim_f} name={anim_name_f} cur={anim_cur_f} total={anim_total_f}");
    // ---- 'f' (anim frame-step) + 'g' (anim resume): animation scrubbing ----
    // `step` pauses playback (Character.pauseAnimationPlayback) and advances one
    // frame via Animation.playFrame(anim, currentFrame+1), then reports A:<name>
    // frame cur/total — frame-by-frame inspection of a move. `play` resumes.
    let f_idx = add_int(code, 'f' as i32);
    let g_idx = add_int(code, 'g' as i32);
    let pause_field = require_field(code, "pxf.entity.Character", "pauseAnimationPlayback")?;
    let play_frame = require_fn(code, "playFrame", Some("pxf.components.Animation"))?;
    let g_ack_g = add_string_const(code, "PLAY\n");
    eprintln!("scrub: pauseAnimationPlayback.f={pause_field} playFrame={play_frame}");
    let t_prefix_g = add_string_const(code, "T:");
    let anim_prefix_g = add_string_const(code, "ANIM:"); // per-frame state-change telemetry
    let t_nomatch_g = add_string_const(code, "T:NOMATCH\n");
    eprintln!("move/telemetry: Character t={char_entity_t} toState={to_state} getStateName={get_state_name} CState t={cstate_statics_t} g={cstate_global} JAB.field={jab_field} characters.field={characters_field}");
    // ---- 'l' command: synchronous custom-.fra load (headless, no worker thread) ----
    // The async UGC/worker path stalls headless; instead we build a PXF Resource and
    // call Resource.fetchThreaded directly on the main thread (it's synchronous inside:
    // File.getBytes -> PXFResource.createFromBytes -> set_DataAsPxf), then finishLoading
    // + addResource. getPXFResource then returns it with _data populated (spawnPlayer's
    // requirement). See docs/ENGINE_RE_MAP_v2.md.
    let resource_t = require_type(code, "pxf.io.Resource")?;
    let resource_ctor = 17827usize; // pxf.io.$Resource.__constructor__ (Resource,String id,String path,t241 enc) — verified this build
    let fetch_threaded = require_fn(code, "fetchThreaded", Some("pxf.io.Resource"))?;
    let finish_loading = require_fn(code, "finishLoading", Some("pxf.io.AbstractResource"))?;
    let add_resource = require_fn(code, "addResource", Some("pxf.io.$ResourceManager"))?;
    let rt_statics_t = find_type(code, "pxf.io.$ResourceType")
        .ok_or_else(|| anyhow::anyhow!("pxf.io.$ResourceType type not found"))?;
    let rt_global = code.globals.iter().position(|g| g.0 == rt_statics_t)
        .ok_or_else(|| anyhow::anyhow!("$ResourceType statics global not found"))?;
    let pxf_field = find_field(code, rt_statics_t, "PXF")
        .ok_or_else(|| anyhow::anyhow!("ResourceType.PXF field not found"))?;
    let res_filepath_field = require_field(code, "pxf.io.AbstractResource", "_filePath")?;
    let res_type_field = require_field(code, "pxf.io.AbstractResource", "_type")?;
    let res_isabs_field = require_field(code, "pxf.io.AbstractResource", "_isAbsolute")?;
    let enc241_t = 241usize; // 4th ctor arg type (Ref<ResourceType>); passed null
    let l_idx = add_int(code, 'l' as i32);
    let sandbag_id_g = add_string_const(code, cname);
    let sandbag_path_g = add_string_const(code, &char_path);
    let l_prefix_g = add_string_const(code, "L:");
    let l_fail_g = add_string_const(code, "L:FAIL\n");
    let l_cmapnull_g = add_string_const(code, " CMAP:NULL\n");
    let sandbag_pkgid_g = add_string_const(code, &char_pkgid);
    let key_sb_g = add_string_const(code, &format!(" KEY={cname}\n"));
    let key_sbsb_g = add_string_const(code, &format!(" KEY={char_pkgid}\n"));
    let key_unknown_g = add_string_const(code, " KEY=?\n");
    let get_sprite_entity = require_fn(code, "getPXFSpriteEntity", Some("pxf.io.$ResourceManager"))?;
    let spr_ok_g = add_string_const(code, "SPR:1\n");
    let spr_null_g = add_string_const(code, "SPR:0\n");
    // NSPR probe: is the *namespaced* buried-VFX key (private::sandbag.sandbag) cached?
    // (SPR probes the bare "sandbag" key; the buried-VFX ctor reads the namespaced one.)
    let nspr_ok_g = add_string_const(code, "NSPR:1\n");
    let nspr_null_g = add_string_const(code, "NSPR:0\n");
    // sprite-entity fix (safe approach B): set RM.requiredMediaIds=["*"] before fetchThreaded so
    // the engine's OWN media-preload closure (run by finishLoading) preloads all entities into
    // _data.entityMap + the namespaced sprite cache; then re-cache the "sandbag" entity under the
    // BARE key (Character ctor looks up the bare spriteContent). No risky direct preload calls.
    let get_data_as_pxf = require_fn(code, "get_DataAsPxf", Some("pxf.io.AbstractResource"))?;
    // Deterministic sprite-entity builder: cacheSpriteEntityData(pxf, idx:Int) builds entities[idx]
    // into PXFResource.entityMap[entity.#2] (its id) — synchronous, no UnsafeCast, no flaky preload.
    let cache_sprite_entity_data = require_fn(code, "cacheSpriteEntityData", Some("pxf.structs.PXFResource"))?;
    let pxf_entities_field = find_field(code, pxfres_t, "entities")
        .ok_or_else(|| anyhow::anyhow!("PXFResource.entities field not found"))?;
    let cache_sprite_entity = require_fn(code, "cacheSpriteEntity", Some("pxf.io.$ResourceManager"))?;
    let sm_get = require_fn(code, "get", Some("haxe.ds.StringMap"))?;
    // pxfSpriteEntityCache field (g3508) — for the getPXFSpriteEntity self-heal patch.
    let spritecache_field = find_field(code, code.globals[3508].0, "pxfSpriteEntityCache")
        .ok_or_else(|| anyhow::anyhow!("pxfSpriteEntityCache field not found"))?;
    let pxf_entitymap_field = find_field(code, pxfres_t, "entityMap")
        .ok_or_else(|| anyhow::anyhow!("PXFResource.entityMap field not found"))?;
    let reqmedia_field = find_field(code, rm_statics_t, "requiredMediaIds")
        .ok_or_else(|| anyhow::anyhow!("requiredMediaIds field not found"))?;
    let sprite_entity_t = 746usize;      // pxf.structs.PXFSpriteEntity
    let cse_arg3_t = 108usize;           // cacheSpriteEntity 3rd arg type
    let star_g = add_string_const(code, "*");
    // The buried-character Vfx (Character.hx:762) reads spriteContent = statsProps.spriteContent,
    // whose runtime value is getResource().getContent("sandbag") = getFullyQualifiedResourceId(res)
    // + "." + "sandbag" = "private::sandbag.sandbag" (RE: getContent@2185 -> getFQContentId@1789).
    // We already re-cache under that key below — the bug was sourcing the entity from entityMap.get
    // (null), so the re-cache never ran. Fixed to source from getPXFSpriteEntity (SPR:1 non-null).
    let ns_sandbag_g = add_string_const(code, &char_ns_key);
    // resource-identifier (NOT content id) form, for the idempotence probe in the
    // self-bootstrapping 's' command: getPXFResource(this) non-null ⇒ already loaded.
    let res_resid_g = add_string_const(code, &char_resid);
    // In-session generic char: the `s` handler builds the launch char's strings at RUNTIME
    // from the command's parts[1] (falling back to the baked default char), so successive
    // `s` commands can switch characters without re-injecting. These const pieces + mutable
    // globals support that. customdir is derived from the install dir (no hardcoded path).
    let customdir_g = add_string_const(code, &format!("{install_dir}/custom/"));
    let slash_g = add_string_const(code, "/");
    let frasuffix_g = add_string_const(code, ".fra");
    let g_name = code.globals.len();  code.globals.push(hlbc::types::RefType(13)); // String: char name
    let g_resid = code.globals.len(); code.globals.push(hlbc::types::RefType(13)); // String: private::<name>
    let g_pkgid = code.globals.len(); code.globals.push(hlbc::types::RefType(13)); // String: <name>.<name>
    let g_nskey = code.globals.len(); code.globals.push(hlbc::types::RefType(13)); // String: private::<name>.<name>
    let g_path = code.globals.len();  code.globals.push(hlbc::types::RefType(13)); // String: <install>/custom/<name>/<name>.fra
    eprintln!("sprite-fix: get_DataAsPxf={get_data_as_pxf} cacheEntity={cache_sprite_entity} smGet={sm_get} entityMap.f={pxf_entitymap_field} requiredMediaIds.f={reqmedia_field}");
    eprintln!("load-cmd: Resource t={resource_t} ctor={resource_ctor} fetchThreaded={fetch_threaded} finishLoading={finish_loading} addResource={add_resource} RT.g={rt_global} PXF.field={pxf_field} _filePath={res_filepath_field} _type={res_type_field} _isAbsolute={res_isabs_field}");
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
    // Loaded-character sprite-cache key (e.g. "private::<id>.<id>"), set by the load
    // for WHATEVER char `s` loads. The getPXFSpriteEntity self-heal falls back to this
    // on a cache miss — generic, not sandbag-specific. Null until a char is loaded.
    let g_loaded_spritekey = code.globals.len();
    code.globals.push(hlbc::types::RefType(13)); // String
    // line-command buffer: "startMatch" takes runtime args (char/stage/assist) sent
    // over the socket. We accumulate the arg bytes into a haxe.io.Bytes scratch
    // buffer, then getString() it into a real String to split + parse.
    let bytes_t2 = require_type(code, "haxe.io.Bytes")?;
    let g_buf = code.globals.len();
    code.globals.push(hlbc::types::RefType(bytes_t2)); // haxe.io.Bytes
    let g_blen = code.globals.len();
    code.globals.push(hlbc::types::RefType(3)); // Int
    // Legacy one-shot guard global (now unused — see MULTI-LAUNCH note at the `s`
    // handler). Retained so harness global indices stay stable across the rest of
    // connect_edit; the `s` command may be sent repeatedly to start successive matches.
    let g_launched = code.globals.len();
    code.globals.push(hlbc::types::RefType(7)); // Bool
    // Persistent top-scope hscript interpreter: created ONCE (with applyInterpreterGlobals
    // -> the engine's global API), reused for every `e`. Null until first eval. This is the
    // single engine-linked interp into which all commands eventually move as one hscript file.
    let g_interp = code.globals.len();
    code.globals.push(hlbc::types::RefType(506)); // hscript.Interp

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
    // 61 keys-iterator(unused) 62 key(String) 63 contentMap(StringMap) 64 bool 65 RM statics
    // 66 pool (ArrayObj=38); 67 pool.array (NativeArray=11); 68 pool elem (absres_t=394)
    // 69 Character (char_entity_t); 70 $CState statics (cstate_statics_t)
    // 71 Resource (resource_t); 72 $ResourceType statics (rt_statics_t); 73 enc Ref<ResourceType> (t241)
    let base = add_regs(f, &[7, 7, sock_t, host_t, 3, out_t, 0, 3, 3, 7, sock_t, handle_t, 3, 3, str_t, enc_t, 3, bytes_t, 3, 3, 3, 3, nulli32_t, tilde_t, console_t, 669, 669, 4366, 9, 1957, 2536, 8, 11, 38, 4366, 675, 668, 6738, 13, 3, dmr_field_t, ms_statics_t, 669, mc_statics_t, match_t, core_statics_t, container_t, h2dobj_t, mode_t, 1194, 4482, bytes_t2, 38, str_t, 0, str_t, str_t, str_t, str_t, nulli32_t2, pxfres_t, keysiter_t, str_t, stringmap_t, 7, rm_statics_t, 38, 11, absres_t, char_entity_t, cstate_statics_t, resource_t, rt_statics_t, enc241_t, sprite_entity_t, cse_arg3_t]);
    let rr = |i: u32| Reg(base + i);
    // eval regs: hscript Parser, Interp, Expr (parseString result), Dynamic (execute result).
    // Appended after the main block so every existing rr(i) index is unchanged.
    let eval_regs_base = add_regs(f, &[hs_parser_t as usize, hs_interp_t as usize, hs_expr_t as usize, 9]);
    let (e_parser, e_interp, e_expr, e_result) =
        (Reg(eval_regs_base), Reg(eval_regs_base + 1), Reg(eval_regs_base + 2), Reg(eval_regs_base + 3));
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
        // BARE NAME (no "."): prefix expansion + cmap check.
        // For each namespace prefix (custom::, public::, global::), build the full id,
        // call getPXFResource to get a properly-typed PXFResource (r(60)), read
        // cmap_field from it (safe because getPXFResource returns pxfres_t). Accept
        // only if both resource AND cmap are non-null — this rejects stubs that loaded
        // under a different namespace without a populated content map. Last prefix
        // (global::) is always accepted as-is (fallback).
        // Jump wiring: each null check for prefix k jumps to the START of prefix k+1.
        // Collected in jump_to_next_prefix and patched below after all starts are known.
        // RS_NOTFOUND: pkgid = name + "." + name
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
        // getPXFResource returns pxfres_t → Field on cmap_field is safe (no cast needed).
        let l_prefix = ops.len();
        let mut found_pref = vec![];
        let mut prefix_starts = vec![];
        // (j_op, target_prefix_index): j_op is a JNull that should jump to prefix[target_prefix_index]
        let mut jump_to_next_prefix: Vec<(usize, usize)> = vec![];
        let n = nsp.len();
        for (k, &pref) in nsp.iter().enumerate() {
            let prefix_start = ops.len();
            prefix_starts.push(prefix_start);
            ops.push(Opcode::GetGlobal { dst: r(53), global: RefGlobal(pref) });
            ops.push(Opcode::Call2 { dst: r(57), fun: RefFun(str_add), arg0: r(53), arg1: r(56) });
            ops.push(Opcode::Call2 { dst: r(out), fun: RefFun(18224), arg0: r(57), arg1: r(38) });
            if k + 1 < n {
                ops.push(Opcode::Call1 { dst: r(58), fun: RefFun(getresid_fn), arg0: r(out) });
                ops.push(Opcode::Call1 { dst: r(60), fun: RefFun(getpxf_fn), arg0: r(58) });
                // LOAD-ON-DEMAND: getpxf null means the resource is in the pool (constructed
                // by importManifest) but its media was skipped by the boot required-load
                // filter (all public::). Load it synchronously — getResourceByID(resid,null)
                // -> cast to Resource -> fetchThreaded (sync read+decode) -> finishLoading —
                // then re-getpxf. r58 is the canonical pool key, so this matches whatever the
                // resolver would look up. Already-loaded resources (e.g. the self-bootstrapped
                // char) skip via the JNotNull guard; ids not in the pool skip via JNull.
                let j_already = ops.len();
                ops.push(Opcode::JNotNull { reg: r(60), offset: 0 });   // already loaded -> skip load
                ops.push(Opcode::Call2 { dst: r(68), fun: RefFun(18287), arg0: r(58), arg1: r(38) }); // getResourceByID
                let j_notpool = ops.len();
                ops.push(Opcode::JNull { reg: r(68), offset: 0 });      // not in pool -> skip load
                ops.push(Opcode::UnsafeCast { dst: r(71), src: r(68) }); // AbstractResource -> Resource
                ops.push(Opcode::Call1 { dst: r(6), fun: RefFun(fetch_threaded), arg0: r(71) });
                ops.push(Opcode::Call1 { dst: r(6), fun: RefFun(finish_loading), arg0: r(71) });
                ops.push(Opcode::Call1 { dst: r(60), fun: RefFun(getpxf_fn), arg0: r(58) }); // re-fetch
                let l_loaded = ops.len();
                if let Opcode::JNotNull { offset, .. } = &mut ops[j_already] { *offset = l_loaded as i32 - j_already as i32 - 1; }
                if let Opcode::JNull    { offset, .. } = &mut ops[j_notpool] { *offset = l_loaded as i32 - j_notpool as i32 - 1; }
                let j_res_null = ops.len();
                ops.push(Opcode::JNull { reg: r(60), offset: 0 });      // null resource -> next prefix
                jump_to_next_prefix.push((j_res_null, k + 1));
                // r(60) is pxfres_t (returned by getPXFResource) — safe to read cmap_field.
                ops.push(Opcode::Field { dst: r(63), obj: r(60), field: RefField(cmap_field) });
                let j_cmap_null = ops.len();
                ops.push(Opcode::JNull { reg: r(63), offset: 0 });      // null cmap -> next prefix
                jump_to_next_prefix.push((j_cmap_null, k + 1));
                let jf = ops.len();
                ops.push(Opcode::JAlways { offset: 0 });                 // cmap non-null -> DONE
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
        set(ops, j_to_prefix, l_prefix);
        set(ops, j_nsdone, l_done);
        // Patch prefix null-jumps to point to the start of the next prefix's code.
        // prefix_starts[k] is the op-index of the first op in prefix k's block.
        for (j_op, target_k) in jump_to_next_prefix {
            if let Some(&tgt) = prefix_starts.get(target_k) {
                set(ops, j_op, tgt);
            }
        }
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
    // (READY + g_ready are NOT sent here — too early: the TrainingMode mode resource
    // and other required match content aren't loaded until the second boot-load pass,
    // whose onComplete is Main.onLoaded. We signal READY from onLoaded instead, after
    // required content is loaded but before launchScreen builds the Title.)
    let _ = ready_g;
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
    // dispatch: 'x' -> clean engine exit (no kill -9 wedge); 'p' -> PONG ; 'c' -> ...
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(x_idx) });
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 1 });          // not 'x' -> skip exit (fall to 'p')
    ops.push(Opcode::Call0 { dst: r_ret, fun: RefFun(hxd_exit) });      // hxd.System.exit() — terminates
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
    // MULTI-LAUNCH: no one-shot guard. `s` re-runs every time its byte arrives over
    // TCP, so a single session can start successive matches with different args. The
    // socket read consumes the line (the `s` drain loop below reads through the trailing
    // '\n'), so one `s` runs exactly once; only a NEW `s` line re-launches. The
    // self-bootstrap below is idempotent (getPXFResource skip) and createMode +
    // startMatch re-initialize the match each time (verified: two launches, T:STAND, no
    // crash). (Legacy g_launched guard global retained unused to keep global indices stable.)
    let _ = g_launched;
    // NOTE: the self-bootstrap (custom-char load) now runs AFTER the line is read + parts
    // are split (below), so it can load the char NAMED in the `s` args (parts[1]) rather
    // than a fixed baked char — enabling in-session character switching. See the
    // "name-driven self-bootstrap" block further down.
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
    // ---- name-driven self-bootstrap: load the custom char NAMED in parts[1] ------------
    // Build this launch's char strings at RUNTIME: name = parts[1] (the `s` arg), or the
    // baked default char for a bare `s`. Then resid=private::<name>, pkgid=<name>.<name>,
    // nskey=private::<name>.<name>, path=<install>/custom/<name>/<name>.fra. This lets
    // successive `s` commands switch characters in one session. The load is at `s`-dispatch
    // time (not boot), so it adds no boot-time cost.
    ops.push(Opcode::Field { dst: rr(39), obj: rr(52), field: RefField(0) });   // parts.length
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(two_idx) });
    let idx_name_jdef = ops.len();
    ops.push(Opcode::JSLt { a: rr(39), b: rr(16), offset: 0 });                 // <2 tokens -> default char
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(one_idx) });
    ops.push(Opcode::GetArray { dst: rr(55), array: rr(32), index: rr(39) });   // name = parts[1]
    let idx_name_jdone = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });
    let idx_name_def = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(sandbag_id_g) }); // baked default name
    let idx_name_done = ops.len();
    if let Opcode::JSLt { offset, .. } = &mut ops[idx_name_jdef] { *offset = idx_name_def as i32 - idx_name_jdef as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_name_jdone] { *offset = idx_name_done as i32 - idx_name_jdone as i32 - 1; }
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_name), src: rr(55) });
    // resid = "private::" + name
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(ns_prefixes[0]) });
    ops.push(Opcode::Call2 { dst: rr(56), fun: RefFun(str_add), arg0: rr(53), arg1: rr(55) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_resid), src: rr(56) });
    // pkgid = name + "." + name
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(dot_g) });
    ops.push(Opcode::Call2 { dst: rr(56), fun: RefFun(str_add), arg0: rr(55), arg1: rr(53) });
    ops.push(Opcode::Call2 { dst: rr(56), fun: RefFun(str_add), arg0: rr(56), arg1: rr(55) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_pkgid), src: rr(56) });
    // nskey = "private::" + pkgid
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(ns_prefixes[0]) });
    ops.push(Opcode::Call2 { dst: rr(56), fun: RefFun(str_add), arg0: rr(53), arg1: rr(56) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_nskey), src: rr(56) });
    // path = customdir + name + "/" + name + ".fra"
    ops.push(Opcode::GetGlobal { dst: rr(56), global: RefGlobal(customdir_g) });
    ops.push(Opcode::Call2 { dst: rr(56), fun: RefFun(str_add), arg0: rr(56), arg1: rr(55) });
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(slash_g) });
    ops.push(Opcode::Call2 { dst: rr(56), fun: RefFun(str_add), arg0: rr(56), arg1: rr(53) });
    ops.push(Opcode::Call2 { dst: rr(56), fun: RefFun(str_add), arg0: rr(56), arg1: rr(55) });
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(frasuffix_g) });
    ops.push(Opcode::Call2 { dst: rr(56), fun: RefFun(str_add), arg0: rr(56), arg1: rr(53) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_path), src: rr(56) });
    // self-bootstrap (idempotent): getPXFResource(resid) non-null -> skip the load.
    ops.push(Opcode::GetGlobal { dst: rr(58), global: RefGlobal(g_resid) });
    ops.push(Opcode::Call1 { dst: rr(60), fun: RefFun(getpxf_fn), arg0: rr(58) });
    let idx_s_load_jskip = ops.len();
    ops.push(Opcode::JNotNull { reg: rr(60), offset: 0 });             // already loaded -> L_SKIP_LOAD
    ops.push(Opcode::New { dst: rr(71) });
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(g_name) });
    ops.push(Opcode::GetGlobal { dst: rr(56), global: RefGlobal(g_path) });
    ops.push(Opcode::Null { dst: rr(73) });
    ops.push(Opcode::Call4 { dst: r_ret, fun: RefFun(resource_ctor), arg0: rr(71), arg1: rr(55), arg2: rr(56), arg3: rr(73) });
    ops.push(Opcode::Bool { dst: rr(64), value: ValBool(true) });
    ops.push(Opcode::SetField { obj: rr(71), field: RefField(res_isabs_field), src: rr(64) });
    ops.push(Opcode::GetGlobal { dst: rr(72), global: RefGlobal(rt_global) });
    ops.push(Opcode::Field { dst: rr(16), obj: rr(72), field: RefField(pxf_field) });
    ops.push(Opcode::SetField { obj: rr(71), field: RefField(res_type_field), src: rr(16) });
    // RM.requiredMediaIds = ["*"] so finishLoading's preload populates entityMap
    ops.push(Opcode::Type { dst: rr(31), ty: RT(13) });
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(one_idx) });
    ops.push(Opcode::Call2 { dst: rr(32), fun: RefFun(256), arg0: rr(31), arg1: rr(39) });
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(zero_idx) });
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(star_g) });
    ops.push(Opcode::SetArray { array: rr(32), index: rr(39), src: rr(53) });
    ops.push(Opcode::Call1 { dst: rr(33), fun: RefFun(257), arg0: rr(32) });
    ops.push(Opcode::GetGlobal { dst: rr(65), global: RefGlobal(3508) });
    ops.push(Opcode::SetField { obj: rr(65), field: RefField(reqmedia_field), src: rr(33) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(fetch_threaded), arg0: rr(71) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(finish_loading), arg0: rr(71) });
    ops.push(Opcode::Call1 { dst: rr(60), fun: RefFun(get_data_as_pxf), arg0: rr(71) });
    ops.push(Opcode::Field { dst: rr(66), obj: rr(60), field: RefField(pxf_entities_field) });
    ops.push(Opcode::Field { dst: rr(39), obj: rr(66), field: RefField(0) });   // .length
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(zero_idx) });
    let idx_sl_loop = ops.len();
    let idx_sl_jge = ops.len();
    ops.push(Opcode::JSGte { a: rr(16), b: rr(39), offset: 0 });        // idx >= len -> build_done
    ops.push(Opcode::Call2 { dst: r_ret, fun: RefFun(cache_sprite_entity_data), arg0: rr(60), arg1: rr(16) });
    ops.push(Opcode::Incr { dst: rr(16) });
    let idx_sl_jback = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                           // -> build_loop
    let idx_sl_done = ops.len();
    // re-cache the main sprite entity under all 3 spriteContent key forms (buried-VFX fix)
    ops.push(Opcode::Field { dst: rr(63), obj: rr(60), field: RefField(pxf_entitymap_field) });
    let idx_sl_emap_jnull = ops.len();
    ops.push(Opcode::JNull { reg: rr(63), offset: 0 });                // null entityMap -> addres
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(g_name) });
    ops.push(Opcode::Call2 { dst: rr(28), fun: RefFun(sm_get), arg0: rr(63), arg1: rr(55) });
    let idx_sl_ent_jnull = ops.len();
    ops.push(Opcode::JNull { reg: rr(28), offset: 0 });                // null entity -> addres
    ops.push(Opcode::UnsafeCast { dst: rr(74), src: rr(28) });
    ops.push(Opcode::Null { dst: rr(75) });
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(g_name) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(cache_sprite_entity), arg0: rr(55), arg1: rr(74), arg2: rr(75) });
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(g_pkgid) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(cache_sprite_entity), arg0: rr(55), arg1: rr(74), arg2: rr(75) });
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(g_nskey) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(cache_sprite_entity), arg0: rr(55), arg1: rr(74), arg2: rr(75) });
    let idx_sl_addres = ops.len();
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(add_resource), arg0: rr(71) });
    // record the loaded char's sprite-cache key for the generic self-heal fallback
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(g_nskey) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_loaded_spritekey), src: rr(55) });
    let idx_sl_skip = ops.len();
    if let Opcode::JNotNull { offset, .. } = &mut ops[idx_s_load_jskip] { *offset = idx_sl_skip as i32 - idx_s_load_jskip as i32 - 1; }
    if let Opcode::JSGte  { offset, .. } = &mut ops[idx_sl_jge]   { *offset = idx_sl_done as i32 - idx_sl_jge as i32 - 1; }
    if let Opcode::JAlways{ offset, .. } = &mut ops[idx_sl_jback] { *offset = idx_sl_loop as i32 - idx_sl_jback as i32 - 1; }
    if let Opcode::JNull  { offset, .. } = &mut ops[idx_sl_emap_jnull] { *offset = idx_sl_addres as i32 - idx_sl_emap_jnull as i32 - 1; }
    if let Opcode::JNull  { offset, .. } = &mut ops[idx_sl_ent_jnull]  { *offset = idx_sl_addres as i32 - idx_sl_ent_jnull as i32 - 1; }
    // the self-bootstrap reused rr32 for the requiredMediaIds array; restore parts.array
    // (rr52 survived) so the downstream char/stage/assist resolver still sees the tokens.
    ops.push(Opcode::Field { dst: rr(32), obj: rr(52), field: RefField(1) }); // parts.array (restored)
    // ---- close any live match before starting a new one (multi-`s` re-launch) ----
    // Reset the menu-reveal one-shot so THIS launch's loading menu is also dismissed once
    // its match goes live (the reveal logic at the top of the dispatch re-fires next frame).
    ops.push(Opcode::Bool { dst: rr(9), value: ValBool(false) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_shown), src: rr(9) });
    // MatchController.cleanupMatch@18325(currentMatch): removes it from _matches, kills its
    // events, nulls currentMatch, tears down its entities. First launch -> currentMatch null
    // -> skipped. Without this, successive `s` commands stacked matches (old ones never closed).
    ops.push(Opcode::GetGlobal { dst: rr(43), global: RefGlobal(3511) });            // MatchController statics
    ops.push(Opcode::Field { dst: rr(44), obj: rr(43), field: RefField(cm_field) }); // currentMatch
    let idx_jnocm = ops.len();
    ops.push(Opcode::JNull { reg: rr(44), offset: 0 });                              // none -> skip cleanup
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(18325), arg0: rr(44) });        // cleanupMatch(currentMatch)
    let idx_after_cleanup = ops.len();
    if let Opcode::JNull { offset, .. } = &mut ops[idx_jnocm] { *offset = idx_after_cleanup as i32 - idx_jnocm as i32 - 1; }
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
    emit_resolve(&mut ops, 55, 25, stage_cmap_field); // stage ref (loads on demand)
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(three_idx) });
    ops.push(Opcode::GetArray { dst: rr(55), array: rr(32), index: rr(39) });
    emit_resolve(&mut ops, 55, 42, assist_cmap_field); // assist ref (loads on demand)
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
    // lives + timer from match_settings.conf. Same Int->ToDyn->DynSet path the
    // `port` field uses above; rr39 = int scratch, rr28 = dyn scratch (free here).
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(lives_idx) });
    ops.push(Opcode::ToDyn { dst: rr(28), src: rr(39) });
    ops.push(Opcode::DynSet { obj: rr(34), field: RS(lives_str), src: rr(28) });
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(time_idx) });
    ops.push(Opcode::ToDyn { dst: rr(28), src: rr(39) });
    ops.push(Opcode::DynSet { obj: rr(34), field: RS(time_str), src: rr(28) });
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
    // DIAGNOSTIC: report the buried-VFX key's cache status every q, to watch the
    // post-startMatch eviction timeline frame-by-frame (q is callable repeatedly).
    {
        let mut a = Asm::new(f.regs.len() as u32);
        let l_null = a.label();
        let l_done = a.label();
        a.op(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(ns_sandbag_g) });
        a.op(Opcode::Call1 { dst: rr(28), fun: RefFun(get_sprite_entity), arg0: rr(55) });
        a.jnull(rr(28), l_null);
        a.op(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(nspr_ok_g) });
        a.jalways(l_done);
        a.place(l_null);
        a.op(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(nspr_null_g) });
        a.place(l_done);
        a.op(Opcode::Null { dst: rr(15) });
        a.op(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
        a.op(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
        let (a_ops, a_regs) = a.finish();
        add_regs(f, &a_regs);
        ops.extend(a_ops);
    }
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

    // ---- 'k' command: dump pool keys to find actual namespace ----
    // Iterate RM.pool (ArrayObj f12), call getFullyQualifiedResourceId on each element,
    // write "K:<fqid>\n" for each. Uses GetArray into dynobj register (type 9 = rr(28))
    // + UnsafeCast to absres_t (394 = rr(68)) before calling getfqid — the same pattern
    // used by addDirToLoadQueue ops 47-50 (GetArray→dynobj→UnsafeCast→concrete call).
    let idx_k_check = ops.len();
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(k_idx) });
    let idx_jne_k = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });            // not 'k' -> L_ORIG
    ops.push(Opcode::Field { dst: r_out, obj: r_sock2, field: RefField(out_field) });
    // diagnostic: UgcUtil.directoriesToLoad (g3449 f11, ArrayObj) length > 0?
    ops.push(Opcode::GetGlobal { dst: rr(65), global: RefGlobal(ugc_statics_g) });
    ops.push(Opcode::Field { dst: rr(66), obj: rr(65), field: RefField(11) }); // directoriesToLoad
    let idx_k_dirs_null = ops.len();
    ops.push(Opcode::JNull { reg: rr(66), offset: 0 });                  // null -> skip dirs report
    ops.push(Opcode::Field { dst: rr(16), obj: rr(66), field: RefField(0) }); // .length
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(zero_idx) });
    let idx_k_dirs_jle = ops.len();
    ops.push(Opcode::JSLte { a: rr(16), b: rr(39), offset: 0 });          // <=0 -> dirs_zero
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(k_dirs_pos_g) });
    let idx_k_dirs_jdone = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });
    let idx_k_dirs_zero = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(k_dirs_zero_g) });
    let idx_k_dirs_done = ops.len();
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
    let idx_k_after_dirs = ops.len();
    // pool = RM.pool (field 12)
    ops.push(Opcode::GetGlobal { dst: rr(65), global: RefGlobal(3508) });
    ops.push(Opcode::Field { dst: rr(66), obj: rr(65), field: RefField(pool_field) });
    let idx_k_jnull_pool = ops.len();
    ops.push(Opcode::JNull { reg: rr(66), offset: 0 });                   // null pool -> k_done
    ops.push(Opcode::Field { dst: rr(39), obj: rr(66), field: RefField(0) }); // pool.length
    ops.push(Opcode::Field { dst: rr(67), obj: rr(66), field: RefField(1) }); // pool.array
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(zero_idx) });          // counter=0
    let idx_k_loop = ops.len();
    let idx_k_jge = ops.len();
    ops.push(Opcode::JSGte { a: rr(16), b: rr(39), offset: 0 });          // done -> k_done
    ops.push(Opcode::GetArray { dst: rr(28), array: rr(67), index: rr(16) }); // pool[i] → dynobj
    ops.push(Opcode::Incr { dst: rr(16) });                               // counter++
    let idx_k_jnull_elem = ops.len();
    ops.push(Opcode::JNull { reg: rr(28), offset: 0 });                   // null elem -> loop
    // UnsafeCast dynobj → AbstractResource (type 394 = absres_t), safe since pool has AbstractResource
    ops.push(Opcode::UnsafeCast { dst: rr(68), src: rr(28) });
    ops.push(Opcode::Call1 { dst: rr(57), fun: RefFun(getfqid_fn), arg0: rr(68) }); // getfqid
    let idx_k_jnull_fqid = ops.len();
    ops.push(Opcode::JNull { reg: rr(57), offset: 0 });                   // null fqid -> loop
    // write "K:" + fqid + "\n"
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(k_prefix_g) });
    ops.push(Opcode::Call2 { dst: rr(58), fun: RefFun(str_add), arg0: rr(53), arg1: rr(57) });
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(nl_g) });
    ops.push(Opcode::Call2 { dst: rr(58), fun: RefFun(str_add), arg0: rr(58), arg1: rr(53) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(58), arg2: rr(15) });
    let idx_k_jback = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                              // -> k_loop
    let idx_k_done = ops.len();
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    // k handler done; fall through to the 'l' check.

    // ---- 'l' command: synchronous custom-.fra load (sandbag), bypassing the worker thread ----
    // Build a PXF Resource for custom/sandbag/sandbag.fra and call fetchThreaded directly
    // (sync read+decode+set_DataAsPxf), then finishLoading + addResource. Ack "L:<fqid>" if
    // getPXFResource then finds it, else "L:FAIL".
    let idx_l_check = ops.len();
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(l_idx) });
    let idx_jne_l = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });            // not 'l' -> m-check
    ops.push(Opcode::Field { dst: r_out, obj: r_sock2, field: RefField(out_field) });
    // r71 = new Resource("sandbag", <abs .fra path>, null)
    ops.push(Opcode::New { dst: rr(71) });
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(sandbag_id_g) });
    ops.push(Opcode::GetGlobal { dst: rr(56), global: RefGlobal(sandbag_path_g) });
    ops.push(Opcode::Null { dst: rr(73) });                              // null enc (t241)
    ops.push(Opcode::Call4 { dst: r_ret, fun: RefFun(resource_ctor), arg0: rr(71), arg1: rr(55), arg2: rr(56), arg3: rr(73) });
    // force _isAbsolute = true (use _filePath verbatim) and _type = ResourceType.PXF
    ops.push(Opcode::Bool { dst: rr(64), value: ValBool(true) });
    ops.push(Opcode::SetField { obj: rr(71), field: RefField(res_isabs_field), src: rr(64) });
    ops.push(Opcode::GetGlobal { dst: rr(72), global: RefGlobal(rt_global) });
    ops.push(Opcode::Field { dst: rr(16), obj: rr(72), field: RefField(pxf_field) }); // PXF (Int)
    ops.push(Opcode::SetField { obj: rr(71), field: RefField(res_type_field), src: rr(16) });
    let _ = res_filepath_field; // (ctor already set _filePath from arg2)
    // set RM.requiredMediaIds = ["*"] so loadMedia's preload closure (run by finishLoading)
    // actually preloads our entities into _data.entityMap (empty headless otherwise -> SPR:0).
    ops.push(Opcode::Type { dst: rr(31), ty: RT(13) });                  // String element type
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(one_idx) });
    ops.push(Opcode::Call2 { dst: rr(32), fun: RefFun(256), arg0: rr(31), arg1: rr(39) }); // alloc_array<String>(1)
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(zero_idx) });
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(star_g) });
    ops.push(Opcode::SetArray { array: rr(32), index: rr(39), src: rr(53) });
    ops.push(Opcode::Call1 { dst: rr(33), fun: RefFun(257), arg0: rr(32) }); // wrap -> ArrayObj
    ops.push(Opcode::GetGlobal { dst: rr(65), global: RefGlobal(3508) });    // RM statics
    ops.push(Opcode::SetField { obj: rr(65), field: RefField(reqmedia_field), src: rr(33) });
    // synchronous read+decode (main thread): fetchThreaded -> File.getBytes -> createFromBytes -> set_DataAsPxf
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(fetch_threaded), arg0: rr(71) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(finish_loading), arg0: rr(71) }); // runs preload closure
    // Deterministically build every sprite entity into PXFResource.entityMap (the flaky
    // requiredMediaIds=["*"] preload sometimes leaves it empty -> SPR:0 -> crash). Loop
    // cacheSpriteEntityData(pxf, idx) over entities[0..len]; each call sets entityMap[entity.#2].
    ops.push(Opcode::Call1 { dst: rr(60), fun: RefFun(get_data_as_pxf), arg0: rr(71) }); // PXFResource
    ops.push(Opcode::Field { dst: rr(66), obj: rr(60), field: RefField(pxf_entities_field) }); // entities (ArrayObj)
    ops.push(Opcode::Field { dst: rr(39), obj: rr(66), field: RefField(0) }); // .length (Int)
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(zero_idx) });            // idx = 0
    let idx_l_bld_loop = ops.len();
    let idx_l_bld_jge = ops.len();
    ops.push(Opcode::JSGte { a: rr(16), b: rr(39), offset: 0 });             // idx >= len -> build_done
    ops.push(Opcode::Call2 { dst: r_ret, fun: RefFun(cache_sprite_entity_data), arg0: rr(60), arg1: rr(16) });
    ops.push(Opcode::Incr { dst: rr(16) });
    let idx_l_bld_jback = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                                 // -> build_loop
    let idx_l_bld_done = ops.len();
    // entity = entityMap.get("sandbag")  (built above; "sandbag" = the main sprite's id)
    ops.push(Opcode::Field { dst: rr(63), obj: rr(60), field: RefField(pxf_entitymap_field) }); // entityMap
    let idx_l_emap_jnull = ops.len();
    ops.push(Opcode::JNull { reg: rr(63), offset: 0 });
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(sandbag_id_g) });
    ops.push(Opcode::Call2 { dst: rr(28), fun: RefFun(sm_get), arg0: rr(63), arg1: rr(55) }); // entity (dyn)
    let idx_l_ent_jnull = ops.len();
    ops.push(Opcode::JNull { reg: rr(28), offset: 0 });
    ops.push(Opcode::UnsafeCast { dst: rr(74), src: rr(28) });           // -> PXFSpriteEntity (t746)
    ops.push(Opcode::Null { dst: rr(75) });                              // 3rd arg (t108)
    // re-cache under all 3 candidate spriteContent key formats (the buried-vfx uses the namespaced
    // "private::sandbag.sandbag"; the other two are harmless); cacheSpriteEntity just sets a map entry.
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(sandbag_id_g) });    // "sandbag"
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(cache_sprite_entity), arg0: rr(55), arg1: rr(74), arg2: rr(75) });
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(sandbag_pkgid_g) }); // "sandbag.sandbag"
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(cache_sprite_entity), arg0: rr(55), arg1: rr(74), arg2: rr(75) });
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(ns_sandbag_g) });    // "private::sandbag.sandbag"
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(cache_sprite_entity), arg0: rr(55), arg1: rr(74), arg2: rr(75) });
    let idx_l_skip_recache = ops.len();
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(add_resource), arg0: rr(71) });
    // verify: getPXFResource(getFullyQualifiedResourceId(res)) non-null?
    ops.push(Opcode::Call1 { dst: rr(57), fun: RefFun(getfqid_fn), arg0: rr(71) }); // fqid string
    ops.push(Opcode::Call1 { dst: rr(60), fun: RefFun(getpxf_fn), arg0: rr(57) });  // PXFResource | null
    let idx_l_jfail = ops.len();
    ops.push(Opcode::JNull { reg: rr(60), offset: 0 });                  // null -> L:FAIL
    // L:OK -> "L:" + fqid + (" CMAP:OK" | " CMAP:NULL") + already-newlined status
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(l_prefix_g) });
    ops.push(Opcode::Call2 { dst: rr(58), fun: RefFun(str_add), arg0: rr(53), arg1: rr(57) });
    // diagnostic: is the loaded PXFResource's characterPxfContentMap populated?
    ops.push(Opcode::Field { dst: rr(63), obj: rr(60), field: RefField(char_cmap_field) });
    let idx_l_cmap_jnull = ops.len();
    ops.push(Opcode::JNull { reg: rr(63), offset: 0 });                  // null cmap -> CMAP:NULL
    // probe the actual content key: exists(cmap,"sandbag")? then exists(cmap,"sandbag.sandbag")?
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(sandbag_id_g) });
    ops.push(Opcode::Call2 { dst: rr(64), fun: RefFun(sm_exists), arg0: rr(63), arg1: rr(55) });
    let idx_l_probe1_jfalse = ops.len();
    ops.push(Opcode::JFalse { cond: rr(64), offset: 0 });                // not "sandbag" -> probe2
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(key_sb_g) });
    let idx_l_probe1_jdone = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });
    let idx_l_probe2 = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(sandbag_pkgid_g) });
    ops.push(Opcode::Call2 { dst: rr(64), fun: RefFun(sm_exists), arg0: rr(63), arg1: rr(55) });
    let idx_l_probe2_jfalse = ops.len();
    ops.push(Opcode::JFalse { cond: rr(64), offset: 0 });                // not "sandbag.sandbag" -> unknown
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(key_sbsb_g) });
    let idx_l_probe2_jdone = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });
    let idx_l_probe_unknown = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(key_unknown_g) });
    let idx_l_probe_unknown_jdone = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });
    let idx_l_cmap_null = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(l_cmapnull_g) });
    let idx_l_cmap_done = ops.len();
    ops.push(Opcode::Call2 { dst: rr(58), fun: RefFun(str_add), arg0: rr(58), arg1: rr(53) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(58), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    // probe: did the media-preload populate pxfSpriteEntityCache for "sandbag"? (the
    // buried-character Vfx in Character ctor null-crashes when getSprite(id) is null)
    ops.push(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(sandbag_id_g) });
    ops.push(Opcode::Call1 { dst: rr(28), fun: RefFun(get_sprite_entity), arg0: rr(55) });
    let idx_l_spr_jnull = ops.len();
    ops.push(Opcode::JNull { reg: rr(28), offset: 0 });
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(spr_ok_g) });
    let idx_l_spr_jdone = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });
    let idx_l_spr_null = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(spr_null_g) });
    let idx_l_spr_done = ops.len();
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    // NSPR probe (built via the Asm label helper — first adoption): is the namespaced
    // buried-VFX key cached after load? Self-contained block, reuses scratch regs, no
    // new registers; all jumps internal so it splices into the op stream cleanly.
    {
        let mut a = Asm::new(f.regs.len() as u32);
        let l_null = a.label();
        let l_done = a.label();
        a.op(Opcode::GetGlobal { dst: rr(55), global: RefGlobal(ns_sandbag_g) });
        a.op(Opcode::Call1 { dst: rr(28), fun: RefFun(get_sprite_entity), arg0: rr(55) });
        a.jnull(rr(28), l_null);
        a.op(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(nspr_ok_g) });
        a.jalways(l_done);
        a.place(l_null);
        a.op(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(nspr_null_g) });
        a.place(l_done);
        a.op(Opcode::Null { dst: rr(15) });
        a.op(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
        a.op(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
        let (a_ops, a_regs) = a.finish();
        add_regs(f, &a_regs); // no reg() calls -> empty, no-op (keeps the pattern explicit)
        ops.extend(a_ops);
    }
    let idx_l_jdone = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                             // -> m-check (continue chain)
    let idx_l_fail = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(l_fail_g) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    // (l_fail falls through into the 'm' check, which rejects 'l')

    // ---- 'm' command: drive a move BY NAME on the live player-0 Character ----
    // Wire form: `m <letter>` where letter = 'A' + ordinal (ordinal indexes
    // commands::MOVES; the bridge does the name→letter mapping). Bare `m` (no
    // arg) = JAB. We walk MatchController.currentMatch -> Match.characters[0],
    // drain the rest of the line to read the selector byte, pick the CState via a
    // GENERATED jump table (one JEq arm per move, built from move_fields above),
    // and call the Character's own state machine toState(char, CState.X, null) —
    // internal dispatch, NOT key-press. Reports M:OK / M:NOMATCH.
    let idx_m_check = ops.len();
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(m_idx) });
    let idx_jne_m = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });            // not 'm' -> t-check
    {
        // Self-contained Asm block: all branches are internal labels (Asm resolves
        // every offset at build time, so a mis-jump is a compile error, not an
        // engine crash). On completion it falls off its end into the 't' check,
        // which rejects the still-'m' command byte and routes to L_ORIG.
        let mut a = Asm::new(f.regs.len() as u32);
        let nomatch = a.label();
        let dispatch = a.label();
        let drain_loop = a.label();
        let drain_done = a.label();
        let after = a.label();
        let r_mc    = a.reg(mc_statics_t);
        let r_cm    = a.reg(match_t);
        let r_chars = a.reg(38);             // ArrayObj
        let r_len   = a.reg(3);
        let r_one   = a.reg(3);
        let r_z     = a.reg(3);
        let r_arr   = a.reg(11);             // NativeArray
        let r_elem  = a.reg(9);              // dyn
        let r_char  = a.reg(char_entity_t);
        let r_sel   = a.reg(3);              // selector byte (-1 = none)
        let r_chr   = a.reg(3);              // recv scratch
        let r_tmp   = a.reg(3);
        let r_cs    = a.reg(cstate_statics_t);
        let r_state = a.reg(3);
        // output = sock2.output
        a.op(Opcode::Field { dst: rr(5), obj: rr(10), field: RefField(out_field) });
        // walk to player-0 Character
        a.op(Opcode::GetGlobal { dst: r_mc, global: RefGlobal(3511) });
        a.op(Opcode::Field { dst: r_cm, obj: r_mc, field: RefField(cm_field) });
        a.jnull(r_cm, nomatch);
        a.op(Opcode::Field { dst: r_chars, obj: r_cm, field: RefField(characters_field) });
        a.jnull(r_chars, nomatch);
        a.op(Opcode::Field { dst: r_len, obj: r_chars, field: RefField(0) }); // .length
        a.op(Opcode::Int { dst: r_one, ptr: RefInt(one_idx) });
        a.jslt(r_len, r_one, nomatch);       // length < 1 -> no player
        a.op(Opcode::Field { dst: r_arr, obj: r_chars, field: RefField(1) }); // .array
        a.op(Opcode::Int { dst: r_z, ptr: RefInt(zero_idx) });
        a.op(Opcode::GetArray { dst: r_elem, array: r_arr, index: r_z });
        a.jnull(r_elem, nomatch);
        a.op(Opcode::UnsafeCast { dst: r_char, src: r_elem });
        // drain the rest of the line, keeping the last non-space byte as the selector
        a.op(Opcode::Int { dst: r_sel, ptr: RefInt(negone_idx) });
        a.place(drain_loop);
        a.op(Opcode::Call1 { dst: r_chr, fun: RefFun(recv_char), arg0: rr(11) });
        a.op(Opcode::Int { dst: r_tmp, ptr: RefInt(zero_idx) });
        a.jslt(r_chr, r_tmp, drain_done);    // <0 (no more data) -> done
        a.op(Opcode::Int { dst: r_tmp, ptr: RefInt(nl_idx) });
        a.jeq(r_chr, r_tmp, drain_done);     // '\n' -> done
        a.op(Opcode::Int { dst: r_tmp, ptr: RefInt(space_idx) });
        a.jeq(r_chr, r_tmp, drain_loop);     // ' ' -> skip
        a.op(Opcode::Mov { dst: r_sel, src: r_chr }); // record selector
        a.jalways(drain_loop);
        a.place(drain_done);
        // default state = CState.JAB, then the generated selector chain may override
        a.op(Opcode::GetGlobal { dst: r_cs, global: RefGlobal(cstate_global) });
        a.op(Opcode::Field { dst: r_state, obj: r_cs, field: RefField(jab_field) });
        let set_labels: Vec<_> = move_fields.iter().map(|_| a.label()).collect();
        for ((letter_idx, _fld), &lbl) in move_fields.iter().zip(set_labels.iter()) {
            a.op(Opcode::Int { dst: r_tmp, ptr: RefInt(*letter_idx) });
            a.jeq(r_sel, r_tmp, lbl);        // selector == 'A'+i -> set move i
        }
        a.jalways(dispatch);                 // no match -> keep JAB default
        for ((_letter, fld), &lbl) in move_fields.iter().zip(set_labels.iter()) {
            a.place(lbl);
            a.op(Opcode::Field { dst: r_state, obj: r_cs, field: RefField(*fld) });
            a.jalways(dispatch);
        }
        a.place(dispatch);
        a.op(Opcode::Null { dst: rr(38) });  // null animName (String)
        a.op(Opcode::Call3 { dst: rr(6), fun: RefFun(to_state), arg0: r_char, arg1: r_state, arg2: rr(38) });
        a.op(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(m_ok_g) });
        a.op(Opcode::Null { dst: rr(15) });
        a.op(Opcode::Call3 { dst: rr(6), fun: RefFun(write_str), arg0: rr(5), arg1: rr(14), arg2: rr(15) });
        a.op(Opcode::Call1 { dst: rr(6), fun: RefFun(flush), arg0: rr(5) });
        a.jalways(after);
        a.place(nomatch);
        a.op(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(m_nomatch_g) });
        a.op(Opcode::Null { dst: rr(15) });
        a.op(Opcode::Call3 { dst: rr(6), fun: RefFun(write_str), arg0: rr(5), arg1: rr(14), arg2: rr(15) });
        a.op(Opcode::Call1 { dst: rr(6), fun: RefFun(flush), arg0: rr(5) });
        a.place(after);
        let (a_ops, a_regs) = a.finish();
        add_regs(f, &a_regs);
        ops.extend(a_ops);
    }
    // (m handler falls through into the 't' check, which rejects 'm' -> L_ORIG)
    let _ = (m_ack_g, jab_field); // m_ack_g retained for ABI stability; jab_field used above

    // ---- 't' command: telemetry — report player-0 Character state name ----
    // currentMatch -> characters[0] -> getStateName(char). Reports T:<state> /
    // T:NOMATCH. Sampled across frames, this detects progress vs. a frozen state.
    let idx_t_check = ops.len();
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(t_idx) });
    let idx_jne_t = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });            // not 't' -> L_ORIG
    ops.push(Opcode::Field { dst: r_out, obj: r_sock2, field: RefField(out_field) });
    ops.push(Opcode::GetGlobal { dst: rr(43), global: RefGlobal(3511) });
    ops.push(Opcode::Field { dst: rr(44), obj: rr(43), field: RefField(cm_field) });
    let idx_t_jnomatch = ops.len();
    ops.push(Opcode::JNull { reg: rr(44), offset: 0 });
    ops.push(Opcode::Field { dst: rr(66), obj: rr(44), field: RefField(characters_field) });
    let idx_t_jnochars = ops.len();
    ops.push(Opcode::JNull { reg: rr(66), offset: 0 });
    ops.push(Opcode::Field { dst: rr(16), obj: rr(66), field: RefField(0) });
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(zero_idx) });
    let idx_t_jempty = ops.len();
    ops.push(Opcode::JSLte { a: rr(16), b: rr(39), offset: 0 });
    ops.push(Opcode::Field { dst: rr(67), obj: rr(66), field: RefField(1) });
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(zero_idx) });
    ops.push(Opcode::GetArray { dst: rr(28), array: rr(67), index: rr(16) });
    let idx_t_jnull_elem = ops.len();
    ops.push(Opcode::JNull { reg: rr(28), offset: 0 });
    ops.push(Opcode::UnsafeCast { dst: rr(69), src: rr(28) });
    ops.push(Opcode::Call1 { dst: rr(57), fun: RefFun(get_state_name), arg0: rr(69) }); // String
    let idx_t_jnull_name = ops.len();
    ops.push(Opcode::JNull { reg: rr(57), offset: 0 });
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(t_prefix_g) });
    ops.push(Opcode::Call2 { dst: rr(58), fun: RefFun(str_add), arg0: rr(53), arg1: rr(57) });
    ops.push(Opcode::GetGlobal { dst: rr(53), global: RefGlobal(nl_g) });
    ops.push(Opcode::Call2 { dst: rr(58), fun: RefFun(str_add), arg0: rr(58), arg1: rr(53) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(58), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    let idx_t_jdone = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                             // -> L_ORIG
    let idx_t_nomatch = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(t_nomatch_g) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(14), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    // t handler done; fall through to the 'v' (physics) check.

    // ---- 'v' command: physics/vitals readback ----
    let idx_v_check = ops.len();
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(v_idx) });
    let idx_jne_v = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });            // not 'v' -> L_ORIG
    {
        // Self-contained Asm block: walk to player 0, format five Floats via
        // Std.string, write one "P: x=.. y=.. vx=.. vy=.. dmg=..\n" line.
        let mut a = Asm::new(f.regs.len() as u32);
        let nomatch = a.label();
        let after = a.label();
        let r_mc   = a.reg(mc_statics_t);
        let r_cm   = a.reg(match_t);
        let r_chs  = a.reg(38);
        let r_len  = a.reg(3);
        let r_one  = a.reg(3);
        let r_z    = a.reg(3);
        let r_arr  = a.reg(11);
        let r_el   = a.reg(9);
        let r_char = a.reg(char_entity_t);
        // One correctly-typed reg per component: HL computes a Field's byte offset from
        // the reg's STATIC type, so a Body-typed reg can't be reused to read Physics/Damage
        // fields (it read uninitialized memory → garbage velocities).
        let r_body = a.reg(body_t);
        let r_phys = a.reg(physics_t);
        let r_dmg  = a.reg(damage_t);
        let r_f    = a.reg(6);               // Float scratch
        let r_dyn  = a.reg(9);               // boxed Float
        let r_str  = a.reg(str_t);           // Std.string result
        let r_acc  = a.reg(str_t);           // accumulator
        let r_lbl  = a.reg(str_t);
        a.op(Opcode::Field { dst: rr(5), obj: rr(10), field: RefField(out_field) });
        a.op(Opcode::GetGlobal { dst: r_mc, global: RefGlobal(3511) });
        a.op(Opcode::Field { dst: r_cm, obj: r_mc, field: RefField(cm_field) });
        a.jnull(r_cm, nomatch);
        a.op(Opcode::Field { dst: r_chs, obj: r_cm, field: RefField(characters_field) });
        a.jnull(r_chs, nomatch);
        a.op(Opcode::Field { dst: r_len, obj: r_chs, field: RefField(0) });
        a.op(Opcode::Int { dst: r_one, ptr: RefInt(one_idx) });
        a.jslt(r_len, r_one, nomatch);
        a.op(Opcode::Field { dst: r_arr, obj: r_chs, field: RefField(1) });
        a.op(Opcode::Int { dst: r_z, ptr: RefInt(zero_idx) });
        a.op(Opcode::GetArray { dst: r_el, array: r_arr, index: r_z });
        a.jnull(r_el, nomatch);
        a.op(Opcode::UnsafeCast { dst: r_char, src: r_el });
        // acc = "P:"
        a.op(Opcode::GetGlobal { dst: r_acc, global: RefGlobal(p_pre_g) });
        // helper closure: append `label` then Std.string(<comp>.field)
        // (emitted inline below per value; Rust can't borrow `a` in a closure cleanly here)
        // x = body.x
        a.op(Opcode::Field { dst: r_body, obj: r_char, field: RefField(char_body_f) });
        a.op(Opcode::Field { dst: r_f, obj: r_body, field: RefField(body_x_f) });
        a.op(Opcode::ToDyn { dst: r_dyn, src: r_f });
        a.op(Opcode::Call1 { dst: r_str, fun: RefFun(std_string), arg0: r_dyn });
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(p_x_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_str });
        // y = body.y
        a.op(Opcode::Field { dst: r_f, obj: r_body, field: RefField(body_y_f) });
        a.op(Opcode::ToDyn { dst: r_dyn, src: r_f });
        a.op(Opcode::Call1 { dst: r_str, fun: RefFun(std_string), arg0: r_dyn });
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(p_y_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_str });
        // vx = physics.currentVelocityX
        a.op(Opcode::Field { dst: r_phys, obj: r_char, field: RefField(char_physics_f) });
        a.op(Opcode::Field { dst: r_f, obj: r_phys, field: RefField(phys_vx_f) });
        a.op(Opcode::ToDyn { dst: r_dyn, src: r_f });
        a.op(Opcode::Call1 { dst: r_str, fun: RefFun(std_string), arg0: r_dyn });
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(p_vx_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_str });
        // vy = physics.currentVelocityY
        a.op(Opcode::Field { dst: r_f, obj: r_phys, field: RefField(phys_vy_f) });
        a.op(Opcode::ToDyn { dst: r_dyn, src: r_f });
        a.op(Opcode::Call1 { dst: r_str, fun: RefFun(std_string), arg0: r_dyn });
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(p_vy_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_str });
        // dmg = damage._damage
        a.op(Opcode::Field { dst: r_dmg, obj: r_char, field: RefField(char_damage_f) });
        a.op(Opcode::Field { dst: r_f, obj: r_dmg, field: RefField(dmg_f) });
        a.op(Opcode::ToDyn { dst: r_dyn, src: r_f });
        a.op(Opcode::Call1 { dst: r_str, fun: RefFun(std_string), arg0: r_dyn });
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(p_dmg_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_str });
        // + "\n" and write
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(nl_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Null { dst: rr(15) });
        a.op(Opcode::Call3 { dst: rr(6), fun: RefFun(write_str), arg0: rr(5), arg1: r_acc, arg2: rr(15) });
        a.op(Opcode::Call1 { dst: rr(6), fun: RefFun(flush), arg0: rr(5) });
        a.jalways(after);
        a.place(nomatch);
        a.op(Opcode::GetGlobal { dst: r_acc, global: RefGlobal(p_nomatch_g) });
        a.op(Opcode::Null { dst: rr(15) });
        a.op(Opcode::Call3 { dst: rr(6), fun: RefFun(write_str), arg0: rr(5), arg1: r_acc, arg2: rr(15) });
        a.op(Opcode::Call1 { dst: rr(6), fun: RefFun(flush), arg0: rr(5) });
        a.place(after);
        let (a_ops, a_regs) = a.finish();
        add_regs(f, &a_regs);
        ops.extend(a_ops);
    }
    // (v handler falls through to the 'a' (animation) check)

    // ---- 'a' command: animation introspection ----
    let idx_a_check = ops.len();
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(a_idx) });
    let idx_jne_a = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });            // not 'a' -> L_ORIG
    {
        let mut a = Asm::new(f.regs.len() as u32);
        let nomatch = a.label();
        let after = a.label();
        let r_mc   = a.reg(mc_statics_t);
        let r_cm   = a.reg(match_t);
        let r_chs  = a.reg(38);
        let r_len  = a.reg(3);
        let r_one  = a.reg(3);
        let r_z    = a.reg(3);
        let r_arr  = a.reg(11);
        let r_el   = a.reg(9);
        let r_char = a.reg(char_entity_t);
        let r_anim = a.reg(anim_t);
        let r_name = a.reg(str_t);
        let r_int  = a.reg(3);
        let r_dyn  = a.reg(9);
        let r_str  = a.reg(str_t);
        let r_acc  = a.reg(str_t);
        let r_lbl  = a.reg(str_t);
        a.op(Opcode::Field { dst: rr(5), obj: rr(10), field: RefField(out_field) });
        a.op(Opcode::GetGlobal { dst: r_mc, global: RefGlobal(3511) });
        a.op(Opcode::Field { dst: r_cm, obj: r_mc, field: RefField(cm_field) });
        a.jnull(r_cm, nomatch);
        a.op(Opcode::Field { dst: r_chs, obj: r_cm, field: RefField(characters_field) });
        a.jnull(r_chs, nomatch);
        a.op(Opcode::Field { dst: r_len, obj: r_chs, field: RefField(0) });
        a.op(Opcode::Int { dst: r_one, ptr: RefInt(one_idx) });
        a.jslt(r_len, r_one, nomatch);
        a.op(Opcode::Field { dst: r_arr, obj: r_chs, field: RefField(1) });
        a.op(Opcode::Int { dst: r_z, ptr: RefInt(zero_idx) });
        a.op(Opcode::GetArray { dst: r_el, array: r_arr, index: r_z });
        a.jnull(r_el, nomatch);
        a.op(Opcode::UnsafeCast { dst: r_char, src: r_el });
        // anim = char.animation; name = anim.currentAnimation
        a.op(Opcode::Field { dst: r_anim, obj: r_char, field: RefField(char_anim_f) });
        a.jnull(r_anim, nomatch);
        a.op(Opcode::Field { dst: r_name, obj: r_anim, field: RefField(anim_name_f) });
        a.jnull(r_name, nomatch);
        // acc = "A:" + name + " frame " + str(currentFrame) + "/" + str(totalFrames) + "\n"
        a.op(Opcode::GetGlobal { dst: r_acc, global: RefGlobal(a_pre_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_name });
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(a_frame_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Field { dst: r_int, obj: r_anim, field: RefField(anim_cur_f) });
        a.op(Opcode::ToDyn { dst: r_dyn, src: r_int });
        a.op(Opcode::Call1 { dst: r_str, fun: RefFun(std_string), arg0: r_dyn });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_str });
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(a_slash_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Field { dst: r_int, obj: r_anim, field: RefField(anim_total_f) });
        a.op(Opcode::ToDyn { dst: r_dyn, src: r_int });
        a.op(Opcode::Call1 { dst: r_str, fun: RefFun(std_string), arg0: r_dyn });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_str });
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(nl_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Null { dst: rr(15) });
        a.op(Opcode::Call3 { dst: rr(6), fun: RefFun(write_str), arg0: rr(5), arg1: r_acc, arg2: rr(15) });
        a.op(Opcode::Call1 { dst: rr(6), fun: RefFun(flush), arg0: rr(5) });
        a.jalways(after);
        a.place(nomatch);
        a.op(Opcode::GetGlobal { dst: r_acc, global: RefGlobal(a_nomatch_g) });
        a.op(Opcode::Null { dst: rr(15) });
        a.op(Opcode::Call3 { dst: rr(6), fun: RefFun(write_str), arg0: rr(5), arg1: r_acc, arg2: rr(15) });
        a.op(Opcode::Call1 { dst: rr(6), fun: RefFun(flush), arg0: rr(5) });
        a.place(after);
        let (a_ops, a_regs) = a.finish();
        add_regs(f, &a_regs);
        ops.extend(a_ops);
    }
    // (a handler falls through to the 'f' (frame-step) check)

    // ---- 'f' command: animation frame-step (pause + advance one frame + report) ----
    let idx_f_check = ops.len();
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(f_idx) });
    let idx_jne_f = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });            // not 'f' -> g-check
    {
        let mut a = Asm::new(f.regs.len() as u32);
        let nomatch = a.label();
        let after = a.label();
        let r_mc = a.reg(mc_statics_t); let r_cm = a.reg(match_t); let r_chs = a.reg(38);
        let r_len = a.reg(3); let r_one = a.reg(3); let r_z = a.reg(3);
        let r_arr = a.reg(11); let r_el = a.reg(9); let r_char = a.reg(char_entity_t);
        let r_anim = a.reg(anim_t); let r_name = a.reg(str_t); let r_int = a.reg(3);
        let r_dyn = a.reg(9); let r_str = a.reg(str_t); let r_acc = a.reg(str_t); let r_lbl = a.reg(str_t);
        let r_true = a.reg(7); let r_rv = a.reg(0);
        a.op(Opcode::Field { dst: rr(5), obj: rr(10), field: RefField(out_field) });
        a.op(Opcode::GetGlobal { dst: r_mc, global: RefGlobal(3511) });
        a.op(Opcode::Field { dst: r_cm, obj: r_mc, field: RefField(cm_field) });
        a.jnull(r_cm, nomatch);
        a.op(Opcode::Field { dst: r_chs, obj: r_cm, field: RefField(characters_field) });
        a.jnull(r_chs, nomatch);
        a.op(Opcode::Field { dst: r_len, obj: r_chs, field: RefField(0) });
        a.op(Opcode::Int { dst: r_one, ptr: RefInt(one_idx) });
        a.jslt(r_len, r_one, nomatch);
        a.op(Opcode::Field { dst: r_arr, obj: r_chs, field: RefField(1) });
        a.op(Opcode::Int { dst: r_z, ptr: RefInt(zero_idx) });
        a.op(Opcode::GetArray { dst: r_el, array: r_arr, index: r_z });
        a.jnull(r_el, nomatch);
        a.op(Opcode::UnsafeCast { dst: r_char, src: r_el });
        // pause playback, then advance one frame via playFrame(anim, currentFrame+1)
        a.op(Opcode::Bool { dst: r_true, value: ValBool(true) });
        a.op(Opcode::SetField { obj: r_char, field: RefField(pause_field), src: r_true });
        a.op(Opcode::Field { dst: r_anim, obj: r_char, field: RefField(char_anim_f) });
        a.jnull(r_anim, nomatch);
        a.op(Opcode::Field { dst: r_int, obj: r_anim, field: RefField(anim_cur_f) });
        a.op(Opcode::Incr { dst: r_int });
        a.op(Opcode::Call2 { dst: r_rv, fun: RefFun(play_frame), arg0: r_anim, arg1: r_int });
        // report A:<name> frame <cur>/<total> (re-read after the step)
        a.op(Opcode::Field { dst: r_name, obj: r_anim, field: RefField(anim_name_f) });
        a.jnull(r_name, nomatch);
        a.op(Opcode::GetGlobal { dst: r_acc, global: RefGlobal(a_pre_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_name });
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(a_frame_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Field { dst: r_int, obj: r_anim, field: RefField(anim_cur_f) });
        a.op(Opcode::ToDyn { dst: r_dyn, src: r_int });
        a.op(Opcode::Call1 { dst: r_str, fun: RefFun(std_string), arg0: r_dyn });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_str });
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(a_slash_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Field { dst: r_int, obj: r_anim, field: RefField(anim_total_f) });
        a.op(Opcode::ToDyn { dst: r_dyn, src: r_int });
        a.op(Opcode::Call1 { dst: r_str, fun: RefFun(std_string), arg0: r_dyn });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_str });
        a.op(Opcode::GetGlobal { dst: r_lbl, global: RefGlobal(nl_g) });
        a.op(Opcode::Call2 { dst: r_acc, fun: RefFun(str_add), arg0: r_acc, arg1: r_lbl });
        a.op(Opcode::Null { dst: rr(15) });
        a.op(Opcode::Call3 { dst: rr(6), fun: RefFun(write_str), arg0: rr(5), arg1: r_acc, arg2: rr(15) });
        a.op(Opcode::Call1 { dst: rr(6), fun: RefFun(flush), arg0: rr(5) });
        a.jalways(after);
        a.place(nomatch);
        a.op(Opcode::GetGlobal { dst: r_acc, global: RefGlobal(a_nomatch_g) });
        a.op(Opcode::Null { dst: rr(15) });
        a.op(Opcode::Call3 { dst: rr(6), fun: RefFun(write_str), arg0: rr(5), arg1: r_acc, arg2: rr(15) });
        a.op(Opcode::Call1 { dst: rr(6), fun: RefFun(flush), arg0: rr(5) });
        a.place(after);
        let (a_ops, a_regs) = a.finish();
        add_regs(f, &a_regs);
        ops.extend(a_ops);
    }
    // ---- 'g' command: resume animation playback ----
    let idx_g_check = ops.len();
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(g_idx) });
    let idx_jne_g = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });            // not 'g' -> L_ORIG
    {
        let mut a = Asm::new(f.regs.len() as u32);
        let nomatch = a.label();
        let after = a.label();
        let r_mc = a.reg(mc_statics_t); let r_cm = a.reg(match_t); let r_chs = a.reg(38);
        let r_len = a.reg(3); let r_one = a.reg(3); let r_z = a.reg(3);
        let r_arr = a.reg(11); let r_el = a.reg(9); let r_char = a.reg(char_entity_t);
        let r_false = a.reg(7); let r_acc = a.reg(str_t);
        a.op(Opcode::Field { dst: rr(5), obj: rr(10), field: RefField(out_field) });
        a.op(Opcode::GetGlobal { dst: r_mc, global: RefGlobal(3511) });
        a.op(Opcode::Field { dst: r_cm, obj: r_mc, field: RefField(cm_field) });
        a.jnull(r_cm, nomatch);
        a.op(Opcode::Field { dst: r_chs, obj: r_cm, field: RefField(characters_field) });
        a.jnull(r_chs, nomatch);
        a.op(Opcode::Field { dst: r_len, obj: r_chs, field: RefField(0) });
        a.op(Opcode::Int { dst: r_one, ptr: RefInt(one_idx) });
        a.jslt(r_len, r_one, nomatch);
        a.op(Opcode::Field { dst: r_arr, obj: r_chs, field: RefField(1) });
        a.op(Opcode::Int { dst: r_z, ptr: RefInt(zero_idx) });
        a.op(Opcode::GetArray { dst: r_el, array: r_arr, index: r_z });
        a.jnull(r_el, nomatch);
        a.op(Opcode::UnsafeCast { dst: r_char, src: r_el });
        a.op(Opcode::Bool { dst: r_false, value: ValBool(false) });
        a.op(Opcode::SetField { obj: r_char, field: RefField(pause_field), src: r_false });
        a.place(nomatch);   // either way, just ack PLAY (resume is idempotent / no-match harmless)
        a.op(Opcode::GetGlobal { dst: r_acc, global: RefGlobal(g_ack_g) });
        a.op(Opcode::Null { dst: rr(15) });
        a.op(Opcode::Call3 { dst: rr(6), fun: RefFun(write_str), arg0: rr(5), arg1: r_acc, arg2: rr(15) });
        a.op(Opcode::Call1 { dst: rr(6), fun: RefFun(flush), arg0: rr(5) });
        a.place(after);
        let _ = after;
        let (a_ops, a_regs) = a.finish();
        add_regs(f, &a_regs);
        ops.extend(a_ops);
    }
    // (g handler falls through to L_ORIG)

    // ---- 'e' (eval) APPENDED at the end of the dispatch chain (after 'g'), so the
    // proven x->p->c->s->...->g chain is byte-identical to baseline. 'e' no-match -> L_ORIG. ----
    let idx_e_check = ops.len();
    // ---- 'e' (eval): parse + execute an hscript string, write "E:<result>\n". SPIKE:
    // a hardcoded script ("1 + 2") proves the in-engine hscript pipeline end-to-end;
    // the socket-driven arbitrary-script form follows once this is green. This single
    // hook is the foundation that replaces the per-command bytecode handlers. ----
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(e_idx) });
    let idx_jne_e = ops.len();
    ops.push(Opcode::JNotEq { a: r_c, b: rr(16), offset: 0 });          // not 'e' -> L_AFTER_E ('x' check)
    let _ = eval_script_g;
    // ---- read the rest of the line ("e <script…>") into g_buf, then getString -> rr14 ----
    // (mirrors the proven `s`-handler drain: accumulate bytes until '\n' or EOF.)
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(zero_idx) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_blen), src: rr(39) });
    let idx_e_drain = ops.len();
    ops.push(Opcode::Call1 { dst: r_c, fun: RefFun(recv_char), arg0: r_handle });
    ops.push(Opcode::Int { dst: r_zero, ptr: RefInt(zero_idx) });
    let idx_e_jslt = ops.len();
    ops.push(Opcode::JSLt { a: r_c, b: r_zero, offset: 0 });            // no more data -> getString
    ops.push(Opcode::Int { dst: rr(16), ptr: RefInt(nl_idx) });
    let idx_e_jeq = ops.len();
    ops.push(Opcode::JEq { a: r_c, b: rr(16), offset: 0 });             // '\n' -> getString
    ops.push(Opcode::GetGlobal { dst: rr(51), global: RefGlobal(g_buf) });
    ops.push(Opcode::GetGlobal { dst: rr(39), global: RefGlobal(g_blen) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(bytes_set), arg0: rr(51), arg1: rr(39), arg2: r_c });
    ops.push(Opcode::GetGlobal { dst: rr(39), global: RefGlobal(g_blen) });
    ops.push(Opcode::Incr { dst: rr(39) });
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_blen), src: rr(39) });
    let idx_e_jback = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                            // -> drain
    let idx_e_getstr = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(51), global: RefGlobal(g_buf) });
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(zero_idx) });
    ops.push(Opcode::GetGlobal { dst: rr(16), global: RefGlobal(g_blen) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call4 { dst: rr(14), fun: RefFun(bytes_getstring), arg0: rr(51), arg1: rr(39), arg2: rr(16), arg3: rr(15) });
    if let Opcode::JSLt { offset, .. } = &mut ops[idx_e_jslt] { *offset = idx_e_getstr as i32 - idx_e_jslt as i32 - 1; }
    if let Opcode::JEq  { offset, .. } = &mut ops[idx_e_jeq]  { *offset = idx_e_getstr as i32 - idx_e_jeq as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_e_jback] { *offset = idx_e_drain as i32 - idx_e_jback as i32 - 1; }
    // Save the command line: rr14 is reused by the prelude load + p0/characters bindings
    // below, so we parse the command AFTER the interp is ready, from this saved copy.
    ops.push(Opcode::Mov { dst: rr(55), src: rr(14) });
    // ---- get-or-create the PERSISTENT top-scope interp, loaded with the engine's global
    // API via applyInterpreterGlobals (CState/HitboxStats/events/… — exactly how Main::init
    // readies every script). Created once, reused for every eval; this is the single
    // engine-linked interp all commands eventually move into as one hscript file. ----
    let _ = (hs_execute, eval_cs_g);
    ops.push(Opcode::GetGlobal { dst: e_interp, global: RefGlobal(g_interp) });
    let idx_e_haveinterp = ops.len();
    ops.push(Opcode::JNotNull { reg: e_interp, offset: 0 });            // already built -> reuse
    ops.push(Opcode::New { dst: e_interp });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(hs_interp_ctor), arg0: e_interp });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(hs_apply_globals), arg0: e_interp }); // load engine API
    ops.push(Opcode::SetGlobal { global: RefGlobal(g_interp), src: e_interp });
    // load the hscript prelude (the ported command implementations) into the interp, once.
    ops.push(Opcode::New { dst: e_parser });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(hs_parser_ctor), arg0: e_parser });
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(prelude_g) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: e_expr, fun: RefFun(hs_parse), arg0: e_parser, arg1: rr(14), arg2: rr(15) });
    ops.push(Opcode::Call2 { dst: e_result, fun: RefFun(hs_interp_script), arg0: e_expr, arg1: e_interp });
    let idx_e_interp_ready = ops.len();
    if let Opcode::JNotNull { offset, .. } = &mut ops[idx_e_haveinterp] { *offset = idx_e_interp_ready as i32 - idx_e_haveinterp as i32 - 1; }
    // ---- bind p0 = MatchController.currentMatch.characters[0] (as Dynamic; null if no match) ----
    // so scripts can reach the live character: `p0.toState(...)`, `p0.body.x`, etc.
    ops.push(Opcode::GetGlobal { dst: rr(43), global: RefGlobal(3511) });
    ops.push(Opcode::Field { dst: rr(44), obj: rr(43), field: RefField(cm_field) });   // currentMatch
    // NOTE: `match` is an hscript facade defined in the prelude (pxf.core.Match has no
    // RTTI so its fields/methods don't reflect); we bind the reliable `characters` array
    // below and the facade reads it. eval_match_g kept for reference.
    let _ = eval_match_g;
    let idx_e_p0null = ops.len();
    ops.push(Opcode::JNull { reg: rr(44), offset: 0 });                                // no match -> p0 = null
    ops.push(Opcode::Field { dst: rr(33), obj: rr(44), field: RefField(characters_field) }); // ArrayObj
    // bind `characters` = the live character ArrayObj (reliable: field-index nav, hscript
    // handles Array natively — characters[0], characters.length, characters[0].getStateName()).
    ops.push(Opcode::ToDyn { dst: rr(28), src: rr(33) });
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(eval_chars_g) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(hs_setvar), arg0: e_interp, arg1: rr(14), arg2: rr(28) });
    let idx_e_chnull = ops.len();
    ops.push(Opcode::JNull { reg: rr(33), offset: 0 });
    ops.push(Opcode::Field { dst: rr(32), obj: rr(33), field: RefField(1) });          // .array (native)
    ops.push(Opcode::Int { dst: rr(39), ptr: RefInt(zero_idx) });
    ops.push(Opcode::GetArray { dst: rr(28), array: rr(32), index: rr(39) });          // characters[0] -> Dynamic
    let idx_e_p0done = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                                           // -> setVar
    let idx_e_bindnull = ops.len();
    ops.push(Opcode::Null { dst: rr(28) });
    let idx_e_setp0 = ops.len();
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(eval_p0_g) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(hs_setvar), arg0: e_interp, arg1: rr(14), arg2: rr(28) });
    if let Opcode::JNull { offset, .. } = &mut ops[idx_e_p0null] { *offset = idx_e_bindnull as i32 - idx_e_p0null as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_e_chnull] { *offset = idx_e_bindnull as i32 - idx_e_chnull as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_e_p0done] { *offset = idx_e_setp0 as i32 - idx_e_p0done as i32 - 1; }
    // parse the command line (saved in rr55) now that the interp + prelude + bindings are ready
    ops.push(Opcode::New { dst: e_parser });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(hs_parser_ctor), arg0: e_parser });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: e_expr, fun: RefFun(hs_parse), arg0: e_parser, arg1: rr(55), arg2: rr(15) });
    // result = ApiScript.interpretScript(expr, interp) — the engine's own run-a-program:
    // resets depth/declared, runs exprReturn, and TRAPS parse/runtime errors (returns null
    // instead of crashing the frame). This is what makes the interp "ready to accept commands".
    ops.push(Opcode::Call2 { dst: e_result, fun: RefFun(hs_interp_script), arg0: e_expr, arg1: e_interp });
    // out = "E:" + Std.string(result) + "\n"
    ops.push(Opcode::Call1 { dst: rr(53), fun: RefFun(std_string), arg0: e_result });
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(eval_prefix_g) });
    ops.push(Opcode::Call2 { dst: rr(53), fun: RefFun(str_add), arg0: rr(14), arg1: rr(53) });
    ops.push(Opcode::GetGlobal { dst: rr(14), global: RefGlobal(eval_nl_g) });
    ops.push(Opcode::Call2 { dst: rr(53), fun: RefFun(str_add), arg0: rr(53), arg1: rr(14) });
    ops.push(Opcode::Field { dst: r_out, obj: r_sock2, field: RefField(out_field) });
    ops.push(Opcode::Null { dst: rr(15) });
    ops.push(Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: rr(53), arg2: rr(15) });
    ops.push(Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out });
    let idx_e_jdone = ops.len();
    ops.push(Opcode::JAlways { offset: 0 });                            // -> L_ORIG (patched in the n-block)

    // L_ORIG = first original op after the prepended block (index n).
    let n = ops.len() as i32;
    if let Opcode::JTrue { offset, .. } = &mut ops[1] { *offset = idx_recv as i32 - 2; }
    if let Opcode::JFalse { offset, .. } = &mut ops[idx_jready] { *offset = n - idx_jready as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_jnull] { *offset = n - idx_jnull as i32 - 1; }
    if let Opcode::JSLt { offset, .. } = &mut ops[idx_jslt] { *offset = n - idx_jslt as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_e_jdone] { *offset = n - idx_e_jdone as i32 - 1; } // 'e' eval done -> L_ORIG
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_p] { *offset = idx_c_check as i32 - idx_jne_p as i32 - 1; }
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_c] { *offset = idx_s_check as i32 - idx_jne_c as i32 - 1; }
    // 's' falls through to the 'q' check; route 'not s' there too; 'q' routes to 'k'; 'k' to L_ORIG.
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_s] { *offset = idx_q_check as i32 - idx_jne_s as i32 - 1; }
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_q] { *offset = idx_k_check as i32 - idx_jne_q as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_q_jnull] { *offset = idx_q_nomatch as i32 - idx_q_jnull as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_q_jdone] { *offset = n - idx_q_jdone as i32 - 1; }
    // _matches diagnostic branch wiring: null/empty -> truly-none; non-empty path
    // falls through then JAlways -> L_ORIG.
    if let Opcode::JNull { offset, .. } = &mut ops[idx_q_jm_null] { *offset = idx_q_truly_none as i32 - idx_q_jm_null as i32 - 1; }
    if let Opcode::JSLte { offset, .. } = &mut ops[idx_q_jm_empty] { *offset = idx_q_truly_none as i32 - idx_q_jm_empty as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_q_jm_done] { *offset = n - idx_q_jm_done as i32 - 1; }
    // k-command jumps ('k' no-match now routes to the 'l' check, not L_ORIG)
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_k] { *offset = idx_l_check as i32 - idx_jne_k as i32 - 1; }
    // l-command jumps ('l' no-match -> m-check; getPXFResource null -> L:FAIL; done -> m-check)
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_l] { *offset = idx_m_check as i32 - idx_jne_l as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_l_jfail] { *offset = idx_l_fail as i32 - idx_l_jfail as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_l_cmap_jnull] { *offset = idx_l_cmap_null as i32 - idx_l_cmap_jnull as i32 - 1; }
    if let Opcode::JFalse { offset, .. } = &mut ops[idx_l_probe1_jfalse] { *offset = idx_l_probe2 as i32 - idx_l_probe1_jfalse as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_l_probe1_jdone] { *offset = idx_l_cmap_done as i32 - idx_l_probe1_jdone as i32 - 1; }
    if let Opcode::JFalse { offset, .. } = &mut ops[idx_l_probe2_jfalse] { *offset = idx_l_probe_unknown as i32 - idx_l_probe2_jfalse as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_l_probe2_jdone] { *offset = idx_l_cmap_done as i32 - idx_l_probe2_jdone as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_l_probe_unknown_jdone] { *offset = idx_l_cmap_done as i32 - idx_l_probe_unknown_jdone as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_l_spr_jnull] { *offset = idx_l_spr_null as i32 - idx_l_spr_jnull as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_l_spr_jdone] { *offset = idx_l_spr_done as i32 - idx_l_spr_jdone as i32 - 1; }
    // entity-build loop jumps (idx>=len -> done; tail -> loop)
    if let Opcode::JSGte { offset, .. } = &mut ops[idx_l_bld_jge] { *offset = idx_l_bld_done as i32 - idx_l_bld_jge as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_l_bld_jback] { *offset = idx_l_bld_loop as i32 - idx_l_bld_jback as i32 - 1; }
    // bare sprite re-cache guards (null entityMap / missing entity -> skip)
    if let Opcode::JNull { offset, .. } = &mut ops[idx_l_emap_jnull] { *offset = idx_l_skip_recache as i32 - idx_l_emap_jnull as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_l_ent_jnull] { *offset = idx_l_skip_recache as i32 - idx_l_ent_jnull as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_l_jdone] { *offset = idx_m_check as i32 - idx_l_jdone as i32 - 1; }
    // k diagnostic (directoriesToLoad) jumps
    if let Opcode::JNull { offset, .. } = &mut ops[idx_k_dirs_null] { *offset = idx_k_after_dirs as i32 - idx_k_dirs_null as i32 - 1; }
    if let Opcode::JSLte { offset, .. } = &mut ops[idx_k_dirs_jle] { *offset = idx_k_dirs_zero as i32 - idx_k_dirs_jle as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_k_dirs_jdone] { *offset = idx_k_dirs_done as i32 - idx_k_dirs_jdone as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_k_jnull_pool] { *offset = idx_k_done as i32 - idx_k_jnull_pool as i32 - 1; }
    if let Opcode::JSGte { offset, .. } = &mut ops[idx_k_jge] { *offset = idx_k_done as i32 - idx_k_jge as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_k_jnull_elem] { *offset = idx_k_loop as i32 - idx_k_jnull_elem as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_k_jnull_fqid] { *offset = idx_k_loop as i32 - idx_k_jnull_fqid as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_k_jback] { *offset = idx_k_loop as i32 - idx_k_jback as i32 - 1; }
    // m-command outer jump only ('m' no-match -> t-check). The handler body is now a
    // self-contained Asm block whose internal branches are resolved by Asm::finish(),
    // so there are no m_* external fixups to patch here anymore.
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_m] { *offset = idx_t_check as i32 - idx_jne_m as i32 - 1; }
    // t-command jumps ('t' no-match -> L_ORIG; failures -> T:NOMATCH; success -> L_ORIG)
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_t] { *offset = idx_v_check as i32 - idx_jne_t as i32 - 1; }
    // 'v' (physics) -> 'a' (animation) -> L_ORIG. Both bodies are self-contained Asm blocks.
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_v] { *offset = idx_a_check as i32 - idx_jne_v as i32 - 1; }
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_a] { *offset = idx_f_check as i32 - idx_jne_a as i32 - 1; }
    // 'f' (frame-step) -> 'g' (resume) -> L_ORIG. Self-contained Asm blocks.
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_f] { *offset = idx_g_check as i32 - idx_jne_f as i32 - 1; }
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_g] { *offset = idx_e_check as i32 - idx_jne_g as i32 - 1; } // g no-match -> 'e' check
    if let Opcode::JNotEq { offset, .. } = &mut ops[idx_jne_e] { *offset = n - idx_jne_e as i32 - 1; } // 'e' no-match -> L_ORIG
    if let Opcode::JNull { offset, .. } = &mut ops[idx_t_jnomatch] { *offset = idx_t_nomatch as i32 - idx_t_jnomatch as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_t_jnochars] { *offset = idx_t_nomatch as i32 - idx_t_jnochars as i32 - 1; }
    if let Opcode::JSLte { offset, .. } = &mut ops[idx_t_jempty] { *offset = idx_t_nomatch as i32 - idx_t_jempty as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_t_jnull_elem] { *offset = idx_t_nomatch as i32 - idx_t_jnull_elem as i32 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[idx_t_jnull_name] { *offset = idx_t_nomatch as i32 - idx_t_jnull_name as i32 - 1; }
    if let Opcode::JAlways { offset, .. } = &mut ops[idx_t_jdone] { *offset = n - idx_t_jdone as i32 - 1; }
    insert_ops_front(f, ops);
    eprintln!("connect_edit: injected {n} ops into update@{update_fx} (console+startMatch dispatch); port={port}");
    // ---- getPXFSpriteEntity self-heal (RELIABLE SPAWN) ------------------------
    // RE proof: there is NO cache reset/removal — the buried-VFX null is a populate-
    // vs-consume RACE (the async per-resource PXF preload caches the sprite key only
    // when sandbag's resource finishes loading; onMatchReady's ctor sometimes wins).
    // Our headless load reliably caches "private::sandbag.sandbag" (NSPR:1, never
    // removed). So: rewrite getPXFSpriteEntity@18289 to, on ANY cache miss, fall back
    // to that always-present entry — converting the race into a guaranteed hit for
    // sandbag's sprite. Reuses the function's own regs (0=key,1=val,2=map,3=RM,4=ret);
    // the original body becomes unreachable (we always Ret).
    {
        let mut a = Asm::new(0); // reuses the fn's own regs (0-4); allocates none
        let l_ret = a.label();
        a.op(Opcode::GetGlobal { dst: Reg(3), global: RefGlobal(3508) });
        a.op(Opcode::Field { dst: Reg(2), obj: Reg(3), field: RefField(spritecache_field) });
        a.op(Opcode::Call2 { dst: Reg(1), fun: RefFun(sm_get), arg0: Reg(2), arg1: Reg(0) });
        a.jnotnull(Reg(1), l_ret);                       // hit -> return it
        // miss: fall back to the loaded char's key (generic — set by the load; null if none)
        a.op(Opcode::GetGlobal { dst: Reg(0), global: RefGlobal(g_loaded_spritekey) });
        a.jnull(Reg(0), l_ret);                          // no loaded char -> return null (orig behavior)
        a.op(Opcode::Call2 { dst: Reg(1), fun: RefFun(sm_get), arg0: Reg(2), arg1: Reg(0) });
        a.place(l_ret);
        a.op(Opcode::UnsafeCast { dst: Reg(4), src: Reg(1) });
        a.op(Opcode::Ret { ret: Reg(4) });
        let (a_ops, a_regs) = a.finish();
        let gfi = function_index_by_findex(code, get_sprite_entity)
            .ok_or_else(|| anyhow::anyhow!("getPXFSpriteEntity@{get_sprite_entity} not found"))?;
        add_regs(&mut code.functions[gfi], &a_regs);
        insert_ops_front(&mut code.functions[gfi], a_ops);
        eprintln!("connect_edit: getPXFSpriteEntity@{get_sprite_entity} self-heal (miss -> private::sandbag.sandbag)");
    }

    // FAST BOOT / SKIP TITLE+MENU: no-op launchScreen@17771 (the LAST boot load-step).
    // It is the ONLY creator of the Title scene (showMenuById("Title")) AND the caller
    // of UgcUtil.loadUgc (the slow "load ALL custom content" scan). Replacing its body
    // with an immediate void Ret suppresses BOTH — no Title/MainMenu is ever built, and
    // no custom content is bulk-loaded. The match-ready signal (READY + g_ready) is now
    // emitted from the connect block on the first update() after CoreEngine.preLoad
    // (which created the game/menu containers + MatchController + set updateLoopReady),
    // so `s` dispatches without ever reaching a menu. startMatch reuses the existing
    // gameContainer (created in configLoaded, before preLoad).
    if headless {
        let lfi = function_index_by_findex(code, 17771)
            .ok_or_else(|| anyhow::anyhow!("launchScreen@17771 not found"))?;
        let lf = &mut code.functions[lfi];
        let vreg = add_regs(lf, &[0]); // void-typed scratch reg for the early Ret
        insert_ops_front(lf, vec![Opcode::Ret { ret: Reg(vreg) }]);
        eprintln!("connect_edit: [headless] no-op'd launchScreen@17771 (skip Title + loadUgc)");
    }
    // Signal READY + set g_ready. The hook point differs by mode so the harness's
    // "wait for READY, then send commands" handshake fires at the right moment:
    //   - headless: Main.onLoaded@17746 (onComplete of the second boot-load; the Title is
    //     no-op'd, so this is the earliest point all match content exists). `s` then
    //     dispatches straight into a match with no menu ever shown — the fast-boot flow.
    //   - non-headless: the END of launchScreen@17771 — i.e. AFTER the Title has been built
    //     and shown. This guarantees a normal boot is never preempted: READY (and therefore
    //     any `s` the harness sends) only happens once the title is up. A match launches
    //     only if `s` is explicitly sent, after the title — never auto-skipped.
    let menu_ready_g = add_string_const(code, "READY\n");
    let ready_hook = if headless { 17746 } else { 17771 };
    inject_ready_flag(code, ready_hook, g_ready, g_sock, out_field, write_str, flush, menu_ready_g, sock_t, out_t, str_t, enc_t, 17842)?;
    // FAST BOOT (deeper): filter the boot required-resources load to skip ALL public:: base
    // content (the ~10.6s of base-character renders we don't need for a 1-char training
    // match). Keeps global:: (hscript/vfx/vsmode) + private:: (common/fonts). The match's
    // stage/assist (also public::) are loaded on demand by the `s` handler from its args
    // (getResourceByID + fetchThreaded), so this is generic — no hardcoded resource names.
    // Headless-only: a normal boot needs the full content set for the menus/picker.
    if headless {
        inject_required_filter(code)?;
    }
    // Animation/state telemetry: hook Character.toState's exit so each completed state
    // transition emits "ANIM:<state>" over the harness socket (event-driven — fires exactly
    // on a change, no per-frame polling). Pinpoints which move was active at a crash.
    inject_anim_telemetry(code, to_state, g_sock, out_field, write_str, flush,
        get_state_name, str_add, anim_prefix_g, nl_g, sock_t, out_t, str_t, enc_t)?;
    Ok(())
}

/// Hook the exit of `pxf.entity.Character::toState` so every COMPLETED state transition
/// emits "ANIM:<stateName>\n" to the harness socket. Inserted before the function's final
/// Ret, so it runs on the success path (state set) — guard early-returns (rejected
/// transitions) don't reach it, which is correct (no change to report). reg0 = the Character.
fn inject_anim_telemetry(
    code: &mut Bytecode, to_state: usize, g_sock: usize, out_field: usize, write_str: usize,
    flush: usize, get_state_name: usize, str_add: usize, anim_prefix_g: usize, nl_g: usize,
    sock_t: usize, out_t: usize, str_t: usize, enc_t: usize,
) -> anyhow::Result<()> {
    use hlbc::types::{RefField, RefFun, RefGlobal};
    let fidx = function_index_by_findex(code, to_state)
        .ok_or_else(|| anyhow::anyhow!("Character.toState@{to_state} not found"))?;
    let f = &mut code.functions[fidx];
    let base = add_regs(f, &[sock_t, out_t, str_t, str_t, enc_t, 0]);
    let (r_sock, r_out, r_name, r_msg, r_null, r_ret) =
        (Reg(base), Reg(base + 1), Reg(base + 2), Reg(base + 3), Reg(base + 4), Reg(base + 5));
    let mut ops = vec![
        Opcode::GetGlobal { dst: r_sock, global: RefGlobal(g_sock) },          // 0
        Opcode::JNull { reg: r_sock, offset: 0 },                              // 1 -> skip (not connected)
        Opcode::Field { dst: r_out, obj: r_sock, field: RefField(out_field) }, // 2
        Opcode::Call1 { dst: r_name, fun: RefFun(get_state_name), arg0: Reg(0) }, // 3 (reg0 = char)
        Opcode::JNull { reg: r_name, offset: 0 },                              // 4 -> skip
        Opcode::GetGlobal { dst: r_msg, global: RefGlobal(anim_prefix_g) },     // 5
        Opcode::Call2 { dst: r_msg, fun: RefFun(str_add), arg0: r_msg, arg1: r_name }, // 6
        Opcode::GetGlobal { dst: r_name, global: RefGlobal(nl_g) },             // 7 (reuse r_name for "\n")
        Opcode::Call2 { dst: r_msg, fun: RefFun(str_add), arg0: r_msg, arg1: r_name }, // 8
        Opcode::Null { dst: r_null },                                          // 9
        Opcode::Call3 { dst: r_ret, fun: RefFun(write_str), arg0: r_out, arg1: r_msg, arg2: r_null }, // 10
        Opcode::Call1 { dst: r_ret, fun: RefFun(flush), arg0: r_out },         // 11
    ];
    let skip = ops.len() as i32; // 12 — falls through to the function's Ret
    if let Opcode::JNull { offset, .. } = &mut ops[1] { *offset = skip - 1 - 1; }
    if let Opcode::JNull { offset, .. } = &mut ops[4] { *offset = skip - 4 - 1; }
    insert_ops_end(f, ops);
    eprintln!("connect_edit: anim telemetry hooked into Character.toState@{to_state}");
    Ok(())
}

/// Filter `pxf.io.$ResourceManager::queueRequiredResources@18234` so the boot load skips
/// `public::` resources (base-character renders — the bulk of the slow second load). We
/// insert a 6-op test right after the loop's `get_Required` check (op22): if the resource's
/// fqid starts with "public::", jump to the loop-tail (skip queuing it). Mid-function
/// insertion is jump-safe here because only THREE jumps span op22 (op6 loop-exit, op21
/// not-required, op121 loop-back); we adjust exactly those by ±N. All build-specific op
/// indices are asserted before patching so a layout change fails loudly instead of
/// silently corrupting control flow.
fn inject_required_filter(code: &mut Bytecode) -> anyhow::Result<()> {
    use hlbc::types::{RefFun, RefGlobal, RefInt};
    let getfqid = require_fn(code, "getFullyQualifiedResourceId", Some("pxf.io.AbstractResource"))?;
    let indexof = require_fn(code, "indexOf", Some("String"))?;
    // 3rd arg (startIndex: Null<Int>) type of String.indexOf
    let startidx_t = {
        let ci = function_index_by_findex(code, indexof)
            .ok_or_else(|| anyhow::anyhow!("indexOf fn missing"))?;
        code.types[code.functions[ci].t.0].get_type_fun()
            .and_then(|tf| tf.args.get(2)).map(|a| a.0)
            .ok_or_else(|| anyhow::anyhow!("indexOf startIndex arg type missing"))?
    };
    let pub_g = add_string_const(code, "public::");
    let zero_c = add_int(code, 0);
    let qfi = function_index_by_findex(code, 18234)
        .ok_or_else(|| anyhow::anyhow!("queueRequiredResources@18234 not found"))?;
    // scratch regs: r_str(String), r_null(startidx_t), r_idx(Int), r_zero(Int)
    let base = add_regs(&mut code.functions[qfi], &[13, startidx_t, 3, 3]);
    let (r_str, r_null, r_idx, r_zero) = (Reg(base), Reg(base + 1), Reg(base + 2), Reg(base + 3));
    // SKIP if the fqid starts with "public::" (jump to loop-tail). The match's stage/assist
    // are ALSO public:: but the `s` handler loads them on demand (getResourceByID +
    // fetchThreaded), so we can skip the entire public:: namespace generically here — no
    // hardcoded resource names. The char is private:: (loaded by the self-bootstrap).
    let filt = vec![
        Opcode::Call1 { dst: Reg(13), fun: RefFun(getfqid), arg0: Reg(9) },                          // 22: fqid (reg9 = loop resource)
        Opcode::GetGlobal { dst: r_str, global: RefGlobal(pub_g) },                                   // 23
        Opcode::Null { dst: r_null },                                                                 // 24
        Opcode::Call3 { dst: r_idx, fun: RefFun(indexof), arg0: Reg(13), arg1: r_str, arg2: r_null }, // 25
        Opcode::Int { dst: r_zero, ptr: RefInt(zero_c) },                                             // 26
        Opcode::JEq { a: r_idx, b: r_zero, offset: 99 },                                              // 27: indexOf==0 (starts public::) -> loop-tail(127)
    ];
    let n = filt.len() as i32; // 6
    let f = &mut code.functions[qfi];
    // Assert + adjust the three jumps that span the insertion point (op22).
    match &mut f.ops[6]   { Opcode::JSGte  { offset, .. } => *offset += n, o => anyhow::bail!("qrr op6 not JSGte: {o:?}") }
    match &mut f.ops[21]  { Opcode::JFalse { offset, .. } => *offset += n, o => anyhow::bail!("qrr op21 not JFalse: {o:?}") }
    match &mut f.ops[121] { Opcode::JAlways{ offset, .. } => *offset -= n, o => anyhow::bail!("qrr op121 not JAlways: {o:?}") }
    let at = 22usize;
    for (i, op) in filt.into_iter().enumerate() { f.ops.insert(at + i, op); }
    if let Some(dbg) = f.debug_info.as_mut() {
        let fill = dbg.get(at).copied().unwrap_or((0, 0));
        for _ in 0..(n as usize) { dbg.insert(at, fill); }
    }
    if let Some(assigns) = f.assigns.as_mut() {
        for (_nm, pos) in assigns.iter_mut() { if *pos >= at { *pos += n as usize; } }
    }
    eprintln!("inject_required_filter: queueRequiredResources@18234 — skip public:: in boot load");
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
    let _ = load_ugc; // fast-boot: no longer called here (see note below)
    let mut ops = vec![
        // Kick off custom-content (UGC) loading. Our injected boot path
        // (Title.start → MainMenu) bypasses Main::launchScreen, which normally
        // calls UgcUtil.loadUgc. We call loadInLocalUgc@17842 (no-arg, no guards)
        // which scans getCwd()/custom async.
        //
        // The threaded local-UGC load (addDirToLoadQueue → load@1845 → fetch →
        // ThreadTaskManager.queueTask@25758 = deque_push) only ENQUEUES tasks; a
        // worker thread spawned by ThreadTaskManager.init@25781 (HaxeThread.create
        // running the deque_pop@26003 loop) must drain them. init is normally
        // called from CoreEngine.init@17729. We call init@25781 (no args) FIRST,
        // defensively, so the worker thread exists to drain our queued .fra loads
        // even if our headless boot path skipped the normal init. (Verified: the
        // ONLY function that pops the task deque is init's spawned worker; without
        // it the `k` pool-key dump showed only private::common, never custom.)
        Opcode::Call0 { dst: r_ret, fun: RefFun(25781) },
        // FAST BOOT: do NOT call loadInLocalUgc here — that scanned custom/ and loaded
        // EVERY custom character (the slow "loading all custom content" screen). We only
        // need the ONE char `s` requests, which the `s`/`l` self-bootstrap loads
        // synchronously on demand. Base/global content (vfx, stages, assists) is loaded
        // by the normal config boot, not UGC, so it's unaffected. (Was: Call0 load_ugc.)
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
