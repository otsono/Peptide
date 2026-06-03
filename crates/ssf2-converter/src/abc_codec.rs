//! abc_codec — a LOSSLESS AVM2 ABC (ActionScript Byte Code) reader + writer.
//!
//! The existing `abc_parser` is a lossy, extraction-oriented reader (flattened
//! multinames, no namespaces/ns-sets, no method signatures/options, no metadata,
//! no exception tables) — it cannot reproduce the input. To INJECT a runtime
//! debug bridge into SSF2.swf we must parse the engine's ABC, add new strings /
//! namespaces / multinames / methods / method-bodies, and re-serialize *exactly*
//! (every untouched structure byte-identical). This module is that faithful
//! codec, following the AVM2 Overview spec.
//!
//! Design: every pool/table is preserved verbatim as typed structs so the writer
//! is the exact inverse of the reader. The method bytecode is kept opaque
//! (`Vec<u8>`); we don't need to decode opcodes to add definitions, and an
//! enter-frame hook only needs a small, well-understood splice (see `inject`).
//!
//! Round-trip invariant (tested against SSF2.swf): parse(b) -> write -> b.

use anyhow::{bail, Result};

// ─────────────────────────── varint primitives ────────────────────────────

/// Cursor over ABC bytes with the AVM2 variable-length integer readers.
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self { Reader { data, pos: 0 } }

    fn u8(&mut self) -> Result<u8> {
        let b = *self.data.get(self.pos).ok_or_else(|| anyhow::anyhow!("u8 EOF @ {}", self.pos))?;
        self.pos += 1;
        Ok(b)
    }
    fn u16(&mut self) -> Result<u16> {
        let lo = self.u8()? as u16; let hi = self.u8()? as u16; Ok(lo | (hi << 8))
    }
    fn d64(&mut self) -> Result<f64> {
        if self.pos + 8 > self.data.len() { bail!("d64 EOF"); }
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.data[self.pos..self.pos + 8]);
        self.pos += 8;
        Ok(f64::from_le_bytes(b))
    }
    /// AVM2 u32 (variable length, ≤5 bytes, 7 bits/byte, high bit continues).
    fn u32v(&mut self) -> Result<u32> {
        let mut result: u32 = 0;
        let mut shift = 0;
        for _ in 0..5 {
            let b = self.u8()?;
            result |= ((b & 0x7f) as u32) << shift;
            if b & 0x80 == 0 { break; }
            shift += 7;
        }
        Ok(result)
    }
    fn u30(&mut self) -> Result<u32> { Ok(self.u32v()? & 0x3fff_ffff) }
    fn s32(&mut self) -> Result<i32> { Ok(self.u32v()? as i32) }
    fn bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        if self.pos + n > self.data.len() { bail!("bytes({n}) EOF @ {}", self.pos); }
        let v = self.data[self.pos..self.pos + n].to_vec();
        self.pos += n;
        Ok(v)
    }
}

/// Writer mirroring `Reader`'s encodings. u30/u32/s32 share the same var encoding;
/// we re-emit the minimal form (which is what asc/the Flash compilers produce, so
/// the round-trip is byte-identical for SSF2.swf).
#[derive(Default)]
pub struct Writer {
    pub out: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self { Writer { out: Vec::new() } }
    fn u8(&mut self, v: u8) { self.out.push(v); }
    fn u16(&mut self, v: u16) { self.out.push((v & 0xff) as u8); self.out.push((v >> 8) as u8); }
    fn d64(&mut self, v: f64) { self.out.extend_from_slice(&v.to_le_bytes()); }
    fn u32v(&mut self, mut v: u32) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 { b |= 0x80; }
            self.out.push(b);
            if v == 0 { break; }
        }
    }
    fn u30(&mut self, v: u32) { self.u32v(v); }
    fn s32(&mut self, v: i32) { self.u32v(v as u32); }
    fn bytes(&mut self, b: &[u8]) { self.out.extend_from_slice(b); }
}

// ─────────────────────────── data model ───────────────────────────────────

#[derive(Debug, Clone)]
pub struct Namespace { pub kind: u8, pub name: u32 }

#[derive(Debug, Clone)]
pub struct NsSet { pub namespaces: Vec<u32> }

/// All AVM2 multiname kinds, preserved exactly.
#[derive(Debug, Clone)]
pub enum Multiname {
    QName { ns: u32, name: u32 },        // 0x07
    QNameA { ns: u32, name: u32 },       // 0x0D
    RTQName { name: u32 },               // 0x0F
    RTQNameA { name: u32 },              // 0x10
    RTQNameL,                            // 0x11
    RTQNameLA,                           // 0x12
    Multiname { name: u32, ns_set: u32 },// 0x09
    MultinameA { name: u32, ns_set: u32 },// 0x0E
    MultinameL { ns_set: u32 },          // 0x1B
    MultinameLA { ns_set: u32 },         // 0x1C
    TypeName { name: u32, params: Vec<u32> }, // 0x1D
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub param_types: Vec<u32>,
    pub return_type: u32,
    pub name: u32,
    pub flags: u8,
    pub options: Vec<(u32, u8)>,     // (value index, value kind)
    pub param_names: Vec<u32>,
}
const METHOD_HAS_OPTIONAL: u8 = 0x08;
const METHOD_HAS_PARAM_NAMES: u8 = 0x80;

#[derive(Debug, Clone)]
pub struct Metadata { pub name: u32, pub items: Vec<(u32, u32)> }

#[derive(Debug, Clone)]
pub enum TraitKindData {
    Slot { slot_id: u32, type_name: u32, vindex: u32, vkind: u8 },   // kind 0 (Slot) / 6 (Const)
    Const { slot_id: u32, type_name: u32, vindex: u32, vkind: u8 },
    Method { disp_id: u32, method: u32 },     // kind 1
    Getter { disp_id: u32, method: u32 },     // kind 2
    Setter { disp_id: u32, method: u32 },     // kind 3
    Class { slot_id: u32, classi: u32 },      // kind 4
    Function { slot_id: u32, function: u32 }, // kind 5
}

#[derive(Debug, Clone)]
pub struct Trait {
    pub name: u32,            // multiname index
    pub kind_byte: u8,        // full kind byte (low nibble kind, high nibble attrs)
    pub data: TraitKindData,
    pub metadata: Vec<u32>,   // present iff (kind_byte>>4) & 0x04
}
const TRAIT_ATTR_METADATA: u8 = 0x04;

#[derive(Debug, Clone)]
pub struct InstanceInfo {
    pub name: u32,
    pub super_name: u32,
    pub flags: u8,
    pub protected_ns: u32,    // present iff flags & 0x08
    pub interfaces: Vec<u32>,
    pub iinit: u32,
    pub traits: Vec<Trait>,
}
const INSTANCE_FLAG_PROTECTED_NS: u8 = 0x08;

#[derive(Debug, Clone)]
pub struct ClassInfo { pub cinit: u32, pub traits: Vec<Trait> }

#[derive(Debug, Clone)]
pub struct ScriptInfo { pub init: u32, pub traits: Vec<Trait> }

#[derive(Debug, Clone)]
pub struct Exception {
    pub from: u32, pub to: u32, pub target: u32,
    pub exc_type: u32, pub var_name: u32,
}

#[derive(Debug, Clone)]
pub struct MethodBody {
    pub method: u32,
    pub max_stack: u32,
    pub local_count: u32,
    pub init_scope_depth: u32,
    pub max_scope_depth: u32,
    pub code: Vec<u8>,
    pub exceptions: Vec<Exception>,
    pub traits: Vec<Trait>,
}

/// The whole ABC block.
#[derive(Debug, Clone)]
pub struct Abc {
    pub minor: u16,
    pub major: u16,
    // constant pool
    pub ints: Vec<i32>,        // index 0 implicit (0); we store [1..]
    pub uints: Vec<u32>,
    pub doubles: Vec<f64>,
    pub strings: Vec<String>,  // raw utf8 (lossy-decoded but re-encoded from bytes_strings)
    pub strings_raw: Vec<Vec<u8>>, // exact bytes (source of truth for write)
    pub namespaces: Vec<Namespace>,
    pub ns_sets: Vec<NsSet>,
    pub multinames: Vec<Multiname>,
    // tables
    pub methods: Vec<MethodInfo>,
    pub metadata: Vec<Metadata>,
    pub instances: Vec<InstanceInfo>,
    pub classes: Vec<ClassInfo>,
    pub scripts: Vec<ScriptInfo>,
    pub bodies: Vec<MethodBody>,
}

// ─────────────────────────── reader ───────────────────────────────────────

fn read_traits(r: &mut Reader) -> Result<Vec<Trait>> {
    let n = r.u30()?;
    let mut traits = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let name = r.u30()?;
        let kind_byte = r.u8()?;
        let kind = kind_byte & 0x0f;
        let attrs = kind_byte >> 4;
        let data = match kind {
            0 => { let slot_id = r.u30()?; let type_name = r.u30()?; let vindex = r.u30()?; let vkind = if vindex != 0 { r.u8()? } else { 0 }; TraitKindData::Slot { slot_id, type_name, vindex, vkind } }
            6 => { let slot_id = r.u30()?; let type_name = r.u30()?; let vindex = r.u30()?; let vkind = if vindex != 0 { r.u8()? } else { 0 }; TraitKindData::Const { slot_id, type_name, vindex, vkind } }
            1 => { let disp_id = r.u30()?; let method = r.u30()?; TraitKindData::Method { disp_id, method } }
            2 => { let disp_id = r.u30()?; let method = r.u30()?; TraitKindData::Getter { disp_id, method } }
            3 => { let disp_id = r.u30()?; let method = r.u30()?; TraitKindData::Setter { disp_id, method } }
            4 => { let slot_id = r.u30()?; let classi = r.u30()?; TraitKindData::Class { slot_id, classi } }
            5 => { let slot_id = r.u30()?; let function = r.u30()?; TraitKindData::Function { slot_id, function } }
            k => bail!("unknown trait kind {k}"),
        };
        let metadata = if attrs & TRAIT_ATTR_METADATA != 0 {
            let mc = r.u30()?; (0..mc).map(|_| r.u30()).collect::<Result<Vec<_>>>()?
        } else { Vec::new() };
        traits.push(Trait { name, kind_byte, data, metadata });
    }
    Ok(traits)
}

pub fn parse(data: &[u8]) -> Result<Abc> {
    let mut r = Reader::new(data);
    let minor = r.u16()?;
    let major = r.u16()?;

    // ints
    let n = r.u30()?; let mut ints = Vec::new(); for _ in 1..n.max(1) { ints.push(r.s32()?); }
    let n = r.u30()?; let mut uints = Vec::new(); for _ in 1..n.max(1) { uints.push(r.u32v()?); }
    let n = r.u30()?; let mut doubles = Vec::new(); for _ in 1..n.max(1) { doubles.push(r.d64()?); }
    let n = r.u30()?; let mut strings_raw = Vec::new();
    for _ in 1..n.max(1) { let len = r.u30()? as usize; strings_raw.push(r.bytes(len)?); }
    let strings: Vec<String> = strings_raw.iter().map(|b| String::from_utf8_lossy(b).into_owned()).collect();

    let n = r.u30()?; let mut namespaces = Vec::new();
    for _ in 1..n.max(1) { let kind = r.u8()?; let name = r.u30()?; namespaces.push(Namespace { kind, name }); }
    let n = r.u30()?; let mut ns_sets = Vec::new();
    for _ in 1..n.max(1) { let c = r.u30()?; let v = (0..c).map(|_| r.u30()).collect::<Result<Vec<_>>>()?; ns_sets.push(NsSet { namespaces: v }); }

    let n = r.u30()?; let mut multinames = Vec::new();
    for _ in 1..n.max(1) {
        let kind = r.u8()?;
        let mn = match kind {
            0x07 => Multiname::QName { ns: r.u30()?, name: r.u30()? },
            0x0D => Multiname::QNameA { ns: r.u30()?, name: r.u30()? },
            0x0F => Multiname::RTQName { name: r.u30()? },
            0x10 => Multiname::RTQNameA { name: r.u30()? },
            0x11 => Multiname::RTQNameL,
            0x12 => Multiname::RTQNameLA,
            0x09 => Multiname::Multiname { name: r.u30()?, ns_set: r.u30()? },
            0x0E => Multiname::MultinameA { name: r.u30()?, ns_set: r.u30()? },
            0x1B => Multiname::MultinameL { ns_set: r.u30()? },
            0x1C => Multiname::MultinameLA { ns_set: r.u30()? },
            0x1D => { let name = r.u30()?; let c = r.u30()?; let params = (0..c).map(|_| r.u30()).collect::<Result<Vec<_>>>()?; Multiname::TypeName { name, params } }
            k => bail!("unknown multiname kind 0x{k:02x} @ {}", r.pos),
        };
        multinames.push(mn);
    }

    // methods
    let n = r.u30()?; let mut methods = Vec::new();
    for _ in 0..n {
        let pc = r.u30()?;
        let return_type = r.u30()?;
        let param_types = (0..pc).map(|_| r.u30()).collect::<Result<Vec<_>>>()?;
        let name = r.u30()?;
        let flags = r.u8()?;
        let options = if flags & METHOD_HAS_OPTIONAL != 0 {
            let oc = r.u30()?; (0..oc).map(|_| Ok((r.u30()?, r.u8()?))).collect::<Result<Vec<_>>>()?
        } else { Vec::new() };
        let param_names = if flags & METHOD_HAS_PARAM_NAMES != 0 {
            (0..pc).map(|_| r.u30()).collect::<Result<Vec<_>>>()?
        } else { Vec::new() };
        methods.push(MethodInfo { param_types, return_type, name, flags, options, param_names });
    }

    // metadata
    let n = r.u30()?; let mut metadata = Vec::new();
    for _ in 0..n {
        let name = r.u30()?;
        let ic = r.u30()?;
        let items = (0..ic).map(|_| Ok((r.u30()?, r.u30()?))).collect::<Result<Vec<_>>>()?;
        metadata.push(Metadata { name, items });
    }

    // classes (instance_info then class_info, same count)
    let class_count = r.u30()?;
    let mut instances = Vec::new();
    for _ in 0..class_count {
        let name = r.u30()?;
        let super_name = r.u30()?;
        let flags = r.u8()?;
        let protected_ns = if flags & INSTANCE_FLAG_PROTECTED_NS != 0 { r.u30()? } else { 0 };
        let ic = r.u30()?;
        let interfaces = (0..ic).map(|_| r.u30()).collect::<Result<Vec<_>>>()?;
        let iinit = r.u30()?;
        let traits = read_traits(&mut r)?;
        instances.push(InstanceInfo { name, super_name, flags, protected_ns, interfaces, iinit, traits });
    }
    let mut classes = Vec::new();
    for _ in 0..class_count {
        let cinit = r.u30()?;
        let traits = read_traits(&mut r)?;
        classes.push(ClassInfo { cinit, traits });
    }

    // scripts
    let n = r.u30()?; let mut scripts = Vec::new();
    for _ in 0..n {
        let init = r.u30()?;
        let traits = read_traits(&mut r)?;
        scripts.push(ScriptInfo { init, traits });
    }

    // method bodies
    let n = r.u30()?; let mut bodies = Vec::new();
    for _ in 0..n {
        let method = r.u30()?;
        let max_stack = r.u30()?;
        let local_count = r.u30()?;
        let init_scope_depth = r.u30()?;
        let max_scope_depth = r.u30()?;
        let code_len = r.u30()? as usize;
        let code = r.bytes(code_len)?;
        let ec = r.u30()?;
        let exceptions = (0..ec).map(|_| Ok(Exception {
            from: r.u30()?, to: r.u30()?, target: r.u30()?, exc_type: r.u30()?, var_name: r.u30()?,
        })).collect::<Result<Vec<_>>>()?;
        let traits = read_traits(&mut r)?;
        bodies.push(MethodBody { method, max_stack, local_count, init_scope_depth, max_scope_depth, code, exceptions, traits });
    }

    if r.pos != data.len() {
        bail!("ABC parse did not consume all bytes: pos={} len={}", r.pos, data.len());
    }

    Ok(Abc {
        minor, major, ints, uints, doubles, strings, strings_raw,
        namespaces, ns_sets, multinames, methods, metadata, instances, classes, scripts, bodies,
    })
}

// ─────────────────────────── writer ───────────────────────────────────────

fn write_traits(w: &mut Writer, traits: &[Trait]) {
    w.u30(traits.len() as u32);
    for t in traits {
        w.u30(t.name);
        w.u8(t.kind_byte);
        match &t.data {
            TraitKindData::Slot { slot_id, type_name, vindex, vkind }
            | TraitKindData::Const { slot_id, type_name, vindex, vkind } => {
                w.u30(*slot_id); w.u30(*type_name); w.u30(*vindex);
                if *vindex != 0 { w.u8(*vkind); }
            }
            TraitKindData::Method { disp_id, method }
            | TraitKindData::Getter { disp_id, method }
            | TraitKindData::Setter { disp_id, method } => { w.u30(*disp_id); w.u30(*method); }
            TraitKindData::Class { slot_id, classi } => { w.u30(*slot_id); w.u30(*classi); }
            TraitKindData::Function { slot_id, function } => { w.u30(*slot_id); w.u30(*function); }
        }
        if (t.kind_byte >> 4) & TRAIT_ATTR_METADATA != 0 {
            w.u30(t.metadata.len() as u32);
            for m in &t.metadata { w.u30(*m); }
        }
    }
}

pub fn write(abc: &Abc) -> Vec<u8> {
    let mut w = Writer::new();
    w.u16(abc.minor);
    w.u16(abc.major);

    // ints (count = entries+1; entry 0 implicit)
    w.u30(if abc.ints.is_empty() { 0 } else { abc.ints.len() as u32 + 1 });
    for v in &abc.ints { w.s32(*v); }
    w.u30(if abc.uints.is_empty() { 0 } else { abc.uints.len() as u32 + 1 });
    for v in &abc.uints { w.u32v(*v); }
    w.u30(if abc.doubles.is_empty() { 0 } else { abc.doubles.len() as u32 + 1 });
    for v in &abc.doubles { w.d64(*v); }
    w.u30(if abc.strings_raw.is_empty() { 0 } else { abc.strings_raw.len() as u32 + 1 });
    for s in &abc.strings_raw { w.u30(s.len() as u32); w.bytes(s); }

    w.u30(if abc.namespaces.is_empty() { 0 } else { abc.namespaces.len() as u32 + 1 });
    for ns in &abc.namespaces { w.u8(ns.kind); w.u30(ns.name); }
    w.u30(if abc.ns_sets.is_empty() { 0 } else { abc.ns_sets.len() as u32 + 1 });
    for s in &abc.ns_sets { w.u30(s.namespaces.len() as u32); for n in &s.namespaces { w.u30(*n); } }

    w.u30(if abc.multinames.is_empty() { 0 } else { abc.multinames.len() as u32 + 1 });
    for mn in &abc.multinames {
        match mn {
            Multiname::QName { ns, name } => { w.u8(0x07); w.u30(*ns); w.u30(*name); }
            Multiname::QNameA { ns, name } => { w.u8(0x0D); w.u30(*ns); w.u30(*name); }
            Multiname::RTQName { name } => { w.u8(0x0F); w.u30(*name); }
            Multiname::RTQNameA { name } => { w.u8(0x10); w.u30(*name); }
            Multiname::RTQNameL => { w.u8(0x11); }
            Multiname::RTQNameLA => { w.u8(0x12); }
            Multiname::Multiname { name, ns_set } => { w.u8(0x09); w.u30(*name); w.u30(*ns_set); }
            Multiname::MultinameA { name, ns_set } => { w.u8(0x0E); w.u30(*name); w.u30(*ns_set); }
            Multiname::MultinameL { ns_set } => { w.u8(0x1B); w.u30(*ns_set); }
            Multiname::MultinameLA { ns_set } => { w.u8(0x1C); w.u30(*ns_set); }
            Multiname::TypeName { name, params } => { w.u8(0x1D); w.u30(*name); w.u30(params.len() as u32); for p in params { w.u30(*p); } }
        }
    }

    w.u30(abc.methods.len() as u32);
    for m in &abc.methods {
        w.u30(m.param_types.len() as u32);
        w.u30(m.return_type);
        for p in &m.param_types { w.u30(*p); }
        w.u30(m.name);
        w.u8(m.flags);
        if m.flags & METHOD_HAS_OPTIONAL != 0 {
            w.u30(m.options.len() as u32);
            for (v, k) in &m.options { w.u30(*v); w.u8(*k); }
        }
        if m.flags & METHOD_HAS_PARAM_NAMES != 0 {
            for n in &m.param_names { w.u30(*n); }
        }
    }

    w.u30(abc.metadata.len() as u32);
    for md in &abc.metadata {
        w.u30(md.name);
        w.u30(md.items.len() as u32);
        for (k, v) in &md.items { w.u30(*k); w.u30(*v); }
    }

    w.u30(abc.instances.len() as u32);
    for inst in &abc.instances {
        w.u30(inst.name); w.u30(inst.super_name); w.u8(inst.flags);
        if inst.flags & INSTANCE_FLAG_PROTECTED_NS != 0 { w.u30(inst.protected_ns); }
        w.u30(inst.interfaces.len() as u32);
        for i in &inst.interfaces { w.u30(*i); }
        w.u30(inst.iinit);
        write_traits(&mut w, &inst.traits);
    }
    for c in &abc.classes { w.u30(c.cinit); write_traits(&mut w, &c.traits); }

    w.u30(abc.scripts.len() as u32);
    for s in &abc.scripts { w.u30(s.init); write_traits(&mut w, &s.traits); }

    w.u30(abc.bodies.len() as u32);
    for b in &abc.bodies {
        w.u30(b.method); w.u30(b.max_stack); w.u30(b.local_count);
        w.u30(b.init_scope_depth); w.u30(b.max_scope_depth);
        w.u30(b.code.len() as u32); w.bytes(&b.code);
        w.u30(b.exceptions.len() as u32);
        for e in &b.exceptions { w.u30(e.from); w.u30(e.to); w.u30(e.target); w.u30(e.exc_type); w.u30(e.var_name); }
        write_traits(&mut w, &b.traits);
    }

    w.out
}

// ─────────────────────────── builder helpers ──────────────────────────────

impl Abc {
    /// Intern a UTF-8 string, returning its 1-based pool index (0 = "*"/any).
    /// Reuses an existing entry when present.
    pub fn intern_string(&mut self, s: &str) -> u32 {
        let bytes = s.as_bytes();
        if let Some(i) = self.strings_raw.iter().position(|b| b == bytes) {
            return i as u32 + 1;
        }
        self.strings_raw.push(bytes.to_vec());
        self.strings.push(s.to_string());
        self.strings_raw.len() as u32
    }

    /// Intern a namespace (kind + name string index).
    pub fn intern_namespace(&mut self, kind: u8, name: u32) -> u32 {
        if let Some(i) = self.namespaces.iter().position(|n| n.kind == kind && n.name == name) {
            return i as u32 + 1;
        }
        self.namespaces.push(Namespace { kind, name });
        self.namespaces.len() as u32
    }

    /// Append a new method_info, returning its index.
    pub fn add_method(&mut self, m: MethodInfo) -> u32 {
        self.methods.push(m);
        self.methods.len() as u32 - 1
    }

    /// Append a new method body.
    pub fn add_body(&mut self, b: MethodBody) {
        self.bodies.push(b);
    }

    /// Add a Method trait (kind 1) to an instance's trait list.
    pub fn add_instance_method_trait(&mut self, class_idx: usize, name_mn: u32, method: u32) {
        self.instances[class_idx].traits.push(Trait {
            name: name_mn,
            kind_byte: 0x01, // Method, no attrs
            data: TraitKindData::Method { disp_id: 0, method },
            metadata: Vec::new(),
        });
    }

    /// Intern a namespace set, returning its 1-based index.
    pub fn intern_ns_set(&mut self, namespaces: Vec<u32>) -> u32 {
        for (i, s) in self.ns_sets.iter().enumerate() {
            if s.namespaces == namespaces { return i as u32 + 1; }
        }
        self.ns_sets.push(NsSet { namespaces });
        self.ns_sets.len() as u32
    }

    /// Intern a runtime-named multiname (`MultinameL`, name popped from the
    /// stack at runtime), bound to namespace-set `ns_set`. Returns its index.
    pub fn intern_multinamel(&mut self, ns_set: u32) -> u32 {
        for (i, mn) in self.multinames.iter().enumerate() {
            if let Multiname::MultinameL { ns_set: s } = mn { if *s == ns_set { return i as u32 + 1; } }
        }
        self.multinames.push(Multiname::MultinameL { ns_set });
        self.multinames.len() as u32
    }

    /// Add an instance Slot trait (a persistent var) of type `*` to a class.
    pub fn add_instance_slot(&mut self, class_idx: usize, name_mn: u32) {
        self.instances[class_idx].traits.push(Trait {
            name: name_mn,
            kind_byte: 0x00, // Slot, no attrs
            data: TraitKindData::Slot { slot_id: 0, type_name: 0, vindex: 0, vkind: 0 },
            metadata: Vec::new(),
        });
    }

    /// Intern a QName multiname (ns, name), returning its 1-based index.
    pub fn intern_qname(&mut self, ns: u32, name: u32) -> u32 {
        for (i, mn) in self.multinames.iter().enumerate() {
            if let Multiname::QName { ns: a, name: b } = mn { if *a == ns && *b == name { return i as u32 + 1; } }
        }
        self.multinames.push(Multiname::QName { ns, name });
        self.multinames.len() as u32
    }

    /// Find a class index by its instance name — matches either the local name
    /// ("Main") or the fully-qualified name ("com.mcleodgaming.ssf2.Main").
    pub fn find_class_by_name(&self, name: &str) -> Option<usize> {
        for (i, inst) in self.instances.iter().enumerate() {
            if self.multiname_local(inst.name).as_deref() == Some(name)
                || self.multiname_qualified(inst.name).as_deref() == Some(name)
            {
                return Some(i);
            }
        }
        None
    }

    /// Fully-qualified "namespace.local" of a QName-style multiname, if resolvable.
    pub fn multiname_qualified(&self, idx: u32) -> Option<String> {
        if idx == 0 { return None; }
        let mn = self.multinames.get(idx as usize - 1)?;
        let (ns_idx, name_idx) = match mn {
            Multiname::QName { ns, name } | Multiname::QNameA { ns, name } => (*ns, *name),
            _ => return None,
        };
        let local = if name_idx == 0 { return None } else { self.strings.get(name_idx as usize - 1)?.clone() };
        let ns = self.namespaces.get(ns_idx.checked_sub(1)? as usize)?;
        let pkg = if ns.name == 0 { String::new() } else { self.strings.get(ns.name as usize - 1).cloned().unwrap_or_default() };
        if pkg.is_empty() { Some(local) } else { Some(format!("{pkg}.{local}")) }
    }

    /// Resolve a multiname's local name string (best-effort, for QName/Multiname).
    pub fn multiname_local(&self, idx: u32) -> Option<String> {
        if idx == 0 { return None; }
        let mn = self.multinames.get(idx as usize - 1)?;
        let name_idx = match mn {
            Multiname::QName { name, .. } | Multiname::QNameA { name, .. }
            | Multiname::RTQName { name } | Multiname::RTQNameA { name }
            | Multiname::Multiname { name, .. } | Multiname::MultinameA { name, .. } => *name,
            Multiname::TypeName { name, .. } => return self.multiname_local(*name),
            _ => return None,
        };
        if name_idx == 0 { return None; }
        self.strings.get(name_idx as usize - 1).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip the real SSF2.swf ABC block: parse -> write must be byte-identical.
    #[test]
    fn roundtrip_ssf2_abc() {
        let swf_path = "/Users/jimmy/Downloads/SSF2BetaMac_v1.4.0.1-standalone 2/SSF2.app/Contents/Resources/SSF2.swf";
        let Ok(data) = std::fs::read(swf_path) else {
            eprintln!("SSF2.swf not present; skipping round-trip");
            return;
        };
        let buf = swf::decompress_swf(&data[..]).expect("decompress");
        let parsed = swf::parse_swf(&buf).expect("parse swf");
        let abc_bytes: &[u8] = parsed.tags.iter().find_map(|t| {
            if let swf::Tag::DoAbc2(a) = t { Some(a.data) } else { None }
        }).expect("DoAbc2");

        let abc = parse(abc_bytes).expect("parse abc");
        let re = write(&abc);
        assert_eq!(re.len(), abc_bytes.len(), "ABC length differs: {} vs {}", re.len(), abc_bytes.len());
        assert!(re == abc_bytes, "ABC bytes differ after round-trip");
        eprintln!("round-trip OK: {} bytes; strings={} multinames={} methods={} classes={} bodies={}",
            re.len(), abc.strings.len(), abc.multinames.len(), abc.methods.len(), abc.instances.len(), abc.bodies.len());
    }
}
