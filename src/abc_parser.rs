/// ABC (ActionScript Bytecode) parser for SSF2 character files.
///
/// SSF2 characters store all gameplay data in AS3 classes compiled to ABC bytecode:
///   - Attack data: object literals with damage, angle, knockback, etc.
///   - Character stats: getOwnStats() method returning physics values
///   - Frame scripts: per-frame logic in timeline classes
///
/// ABC format reference: https://web.archive.org/web/2024/https://adobe.com/content/dam/amd/en/devnet/actionscript/articles/avm2overview.pdf

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use crate::decompiler;

// ─── AVM2 opcodes we care about ──────────────────────────────────────────────
const OP_PUSHBYTE:      u8 = 0x24;
const OP_PUSHSHORT:     u8 = 0x25;
const OP_PUSHINT:       u8 = 0x2D;
const OP_PUSHUINT:      u8 = 0x2E;
const OP_PUSHDOUBLE:    u8 = 0x2F;
const OP_PUSHSTRING:    u8 = 0x2C;
const OP_PUSHTRUE:      u8 = 0x26;
const OP_PUSHFALSE:     u8 = 0x27;
const OP_PUSHNULL:      u8 = 0x20;
const OP_PUSHNAN:       u8 = 0x28;
const OP_NEWOBJECT:     u8 = 0x55;
const OP_NEWARRAY:      u8 = 0x56;
const OP_CALLPROPERTY:  u8 = 0x46;
const OP_CALLPROPVOID:  u8 = 0x4F;
const OP_SETPROPERTY:   u8 = 0x61;
const OP_GETPROPERTY:   u8 = 0x66;
const OP_FINDPROP:      u8 = 0x5C;
const OP_FINDPROPSTRICT:u8 = 0x5D;
const OP_GETLEX:        u8 = 0x60;
const OP_COERCE:        u8 = 0x80;
const OP_COERCE_A:      u8 = 0x82;
const OP_RETURNVALUE:   u8 = 0x48;
const OP_RETURNVOID:    u8 = 0x47;
const OP_GETLOCAL0:     u8 = 0xD0;
const OP_GETLOCAL1:     u8 = 0xD1;
const OP_GETLOCAL2:     u8 = 0xD2;
const OP_GETLOCAL3:     u8 = 0xD3;
const OP_GETLOCAL:      u8 = 0x62;
const OP_SETLOCAL:      u8 = 0x63;
const OP_SETLOCAL0:     u8 = 0xD4;
const OP_SETLOCAL1:     u8 = 0xD5;
const OP_SETLOCAL2:     u8 = 0xD6;
const OP_SETLOCAL3:     u8 = 0xD7;
const OP_INITPROPERTY:  u8 = 0x68;
const OP_CONSTRUCTPROP: u8 = 0x4A;
const OP_CONSTRUCT:     u8 = 0x42;
const OP_NOP:           u8 = 0x02;
const OP_POP:           u8 = 0x29;
const OP_DUP:           u8 = 0x2A;
const OP_SWAP:          u8 = 0x2B;
const OP_ADD:           u8 = 0xA0;
const OP_SUBTRACT:      u8 = 0xA1;
const OP_MULTIPLY:      u8 = 0xA2;
const OP_DIVIDE:        u8 = 0xA3;
const OP_NEGATE:        u8 = 0x90;
const OP_CONVERT_D:     u8 = 0x84;
const OP_CONVERT_I:     u8 = 0x83;
const OP_LABEL:         u8 = 0x09;
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

// ─── Parsed ABC structures ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbcFile {
    pub strings: Vec<String>,
    pub ints: Vec<i32>,
    pub uints: Vec<u32>,
    pub doubles: Vec<f64>,
    pub multinames: Vec<Multiname>,
    pub methods: Vec<Method>,
    pub classes: Vec<Class>,
    pub scripts: Vec<Script>,
    pub method_bodies: Vec<MethodBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Multiname {
    pub kind: u8,
    pub name_idx: u32,
    pub ns_idx: u32,
    pub name: String, // resolved from string pool
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Method {
    pub name_idx: u32,
    pub name: String,
    pub param_count: u32,
    pub return_type_idx: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Class {
    pub name: String,
    pub super_name: String,
    pub instance_methods: Vec<Trait>,
    pub class_methods: Vec<Trait>,
    pub constructor_idx: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trait {
    pub name: String,
    pub kind: u8,
    pub method_idx: u32,
    pub slot_idx: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    pub init_method_idx: u32,
    pub traits: Vec<Trait>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodBody {
    pub method_idx: u32,
    pub max_stack: u32,
    pub local_count: u32,
    pub bytecode: Vec<u8>,
    pub activation_traits: Vec<Trait>,  // named slots from newactivation
}

// ─── Extracted character data ─────────────────────────────────────────────────

/// Maps SSF2 frame method name → SSF2 animation name (from self.xframe = "...").
/// e.g. "frame14" → "a", "frame29" → "a_air"
pub type XframeMap = BTreeMap<String, String>;

pub struct ExtractedCharacter {
    pub name: String,
    pub attacks: BTreeMap<String, AttackData>,
    /// Per-projectile physics + hitboxes pulled from the Ext class's
    /// `getProjectileStats()` method. Keys are SSF2 projectile names
    /// (matching the projectile sprite SymbolClass names).
    pub projectiles: BTreeMap<String, ProjectileData>,
    pub stats: Option<CharStats>,
    pub frame_scripts: BTreeMap<String, Vec<FrameAction>>,
    /// Decompiled Ext class methods translated to Fraymakers Haxe
    pub ext_methods: BTreeMap<String, String>,
    /// Names of instance Slot/Const traits on the Ext class (the SSF2
    /// `public var foo:T;` declarations).
    pub ext_vars: Vec<String>,
    /// Initial values for the ext_vars, pulled out of the Ext class
    /// constructor (iinit). `(name, rhs_expression)`; one per var that the
    /// constructor actually assigns.
    pub ext_var_inits: Vec<(String, String)>,
    /// frame method name → SSF2 animation name (from self.xframe = "...")
    pub xframe_map: XframeMap,
    /// Costumes from SSF2API::getCostumeData — name → list of ARGB color values
    pub costumes: Vec<CostumeData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostumeData {
    pub name: String,
    /// Source ARGB color values — the base sprite palette (same across all costumes)
    pub colors: Vec<u32>,
    /// Replacement ARGB color values — what each source color becomes for this costume
    pub replacements: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackData {
    pub hitboxes: Vec<BTreeMap<String, f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharStats {
    pub values: BTreeMap<String, f64>,
}

/// One projectile-stat entry extracted from SSF2's `getProjectileStats()`.
/// The Ext class returns a top-level object keyed by projectile name
/// (matching the projectile sprite's class name, e.g. `dee_nspec`).
/// Each value combines:
///   - flat-scalar physics fields (gravity, xSpeed, ySpeed, friction, …)
///   - a nested `attackBoxes` object with the projectile's hitboxes
/// Hitbox shape mirrors `AttackData.hitboxes` so it can share the
/// `hitbox_stats.jsonc` SSF2→FM field-name canon.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectileData {
    pub stats:    BTreeMap<String, f64>,
    pub hitboxes: Vec<BTreeMap<String, f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameAction {
    pub frame: u32,
    pub action: String,
    pub args: Vec<String>,
}

// ─── Reader helpers ───────────────────────────────────────────────────────────

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    #[allow(dead_code)]
    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_u8(&mut self) -> Result<u8> {
        if self.pos >= self.data.len() {
            return Err(anyhow!("read_u8: out of bounds at {}", self.pos));
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_u16(&mut self) -> Result<u16> {
        let lo = self.read_u8()? as u16;
        let hi = self.read_u8()? as u16;
        Ok(lo | (hi << 8))
    }

    #[allow(dead_code)]
    fn read_u32(&mut self) -> Result<u32> {
        let lo = self.read_u16()? as u32;
        let hi = self.read_u16()? as u32;
        Ok(lo | (hi << 16))
    }

    fn read_f64(&mut self) -> Result<f64> {
        if self.pos + 8 > self.data.len() {
            return Err(anyhow!("read_f64: out of bounds"));
        }
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.data[self.pos..self.pos + 8]);
        self.pos += 8;
        Ok(f64::from_le_bytes(bytes))
    }

    /// Variable-length encoded u30/u32
    fn read_u30(&mut self) -> Result<u32> {
        let mut result = 0u32;
        let mut shift = 0;
        loop {
            let b = self.read_u8()? as u32;
            result |= (b & 0x7F) << shift;
            shift += 7;
            if b & 0x80 == 0 || shift >= 35 {
                break;
            }
        }
        Ok(result)
    }

    /// Variable-length encoded s32
    fn read_s32(&mut self) -> Result<i32> {
        let v = self.read_u30()?;
        // sign-extend from 29 bits
        if v & 0x10000000 != 0 {
            Ok((v | 0xE0000000) as i32)
        } else {
            Ok(v as i32)
        }
    }

    fn read_string(&mut self) -> Result<String> {
        let len = self.read_u30()? as usize;
        if self.pos + len > self.data.len() {
            return Err(anyhow!("read_string: length {} out of bounds at pos {}", len, self.pos));
        }
        // ABC strings are spec'd as UTF-8 but SSF2 SWFs sometimes carry a
        // Windows-1252 byte (an `é` etc.) — `from_utf8_lossy` silently
        // inserts U+FFFD, which downstream filters (`looks_like_char_name`,
        // is-alphanumeric checks on xframe labels) then reject, losing the
        // trait. Log a debug-level warning when we actually replaced any
        // bytes so the loss is visible without spamming the normal log.
        let raw = &self.data[self.pos..self.pos + len];
        let s = match std::str::from_utf8(raw) {
            Ok(valid) => valid.to_string(),
            Err(_) => {
                if log::log_enabled!(log::Level::Debug) {
                    log::debug!(
                        "read_string: non-UTF8 bytes at pos {} (len={}); \
                         using lossy UTF-8 decode — character data may be \
                         silently filtered later",
                        self.pos, len
                    );
                }
                String::from_utf8_lossy(raw).to_string()
            }
        };
        self.pos += len;
        Ok(s)
    }

    #[allow(dead_code)]
    fn skip(&mut self, n: usize) -> Result<()> {
        if self.pos + n > self.data.len() {
            return Err(anyhow!("skip: {} out of bounds at {}", n, self.pos));
        }
        self.pos += n;
        Ok(())
    }
}

// ─── ABC parsing ─────────────────────────────────────────────────────────────

pub fn parse(data: &[u8]) -> Result<AbcFile> {
    let mut r = Reader::new(data);

    // Header: minor version, major version
    let _minor = r.read_u16()?;
    let _major = r.read_u16()?;

    // ── Constant pool ──────────────────────────────────────────────────────────

    // Integers
    let int_count = r.read_u30()? as usize;
    let mut ints = vec![0i32];
    for _ in 1..int_count {
        ints.push(r.read_s32()?);
    }

    // Unsigned integers
    let uint_count = r.read_u30()? as usize;
    let mut uints = vec![0u32];
    for _ in 1..uint_count {
        uints.push(r.read_u30()?);
    }

    // Doubles
    let double_count = r.read_u30()? as usize;
    let mut doubles = vec![f64::NAN];
    for _ in 1..double_count {
        doubles.push(r.read_f64()?);
    }

    // Strings
    let string_count = r.read_u30()? as usize;
    let mut strings = vec![String::new()];
    for _ in 1..string_count {
        strings.push(r.read_string()?);
    }

    log::debug!("ABC constants: {} ints, {} uints, {} doubles, {} strings",
        ints.len(), uints.len(), doubles.len(), strings.len());

    // Namespaces
    let ns_count = r.read_u30()? as usize;
    let mut namespaces: Vec<String> = vec![String::new()];
    for _ in 1..ns_count {
        let _kind = r.read_u8()?;
        let name_idx = r.read_u30()?;
        namespaces.push(strings.get(name_idx as usize).cloned().unwrap_or_default());
    }

    // Namespace sets
    let nsset_count = r.read_u30()? as usize;
    for _ in 1..nsset_count {
        let ns_count = r.read_u30()? as usize;
        for _ in 0..ns_count {
            r.read_u30()?;
        }
    }

    // Multinames
    let mn_count = r.read_u30()? as usize;
    let mut multinames = vec![Multiname { kind: 0, name_idx: 0, ns_idx: 0, name: String::new() }];
    for _ in 1..mn_count {
        let kind = r.read_u8()?;
        let mn = match kind {
            0x07 | 0x0D => { // QName, QNameA
                let ns = r.read_u30()?;
                let name_idx = r.read_u30()?;
                let name = strings.get(name_idx as usize).cloned().unwrap_or_default();
                Multiname { kind, name_idx, ns_idx: ns, name }
            }
            0x0F | 0x10 => { // RTQName, RTQNameA
                let name_idx = r.read_u30()?;
                Multiname { kind, name_idx, ns_idx: 0, name: strings.get(name_idx as usize).cloned().unwrap_or_default() }
            }
            0x11 | 0x12 => { // RTQNameL, RTQNameLA
                Multiname { kind, name_idx: 0, ns_idx: 0, name: String::new() }
            }
            0x09 | 0x0E => { // Multiname, MultinameA
                let name_idx = r.read_u30()?;
                let _ns_set = r.read_u30()?;
                let name = strings.get(name_idx as usize).cloned().unwrap_or_default();
                Multiname { kind, name_idx, ns_idx: 0, name }
            }
            0x1B | 0x1C => { // MultinameL, MultinameLA
                let _ns_set = r.read_u30()?;
                Multiname { kind, name_idx: 0, ns_idx: 0, name: String::new() }
            }
            0x1D => { // TypeName (generic)
                let _qname = r.read_u30()?;
                let param_count = r.read_u30()? as usize;
                for _ in 0..param_count { r.read_u30()?; }
                Multiname { kind, name_idx: 0, ns_idx: 0, name: String::new() }
            }
            _ => {
                log::warn!("Unknown multiname kind: 0x{:02X}", kind);
                Multiname { kind, name_idx: 0, ns_idx: 0, name: String::new() }
            }
        };
        multinames.push(mn);
    }

    // ── Methods ───────────────────────────────────────────────────────────────

    let method_count = r.read_u30()? as usize;
    let mut methods = Vec::with_capacity(method_count);
    for _ in 0..method_count {
        let param_count = r.read_u30()?;
        let return_type_idx = r.read_u30()?;
        for _ in 0..param_count { r.read_u30()?; } // param types
        let name_idx = r.read_u30()?;
        let flags = r.read_u8()?;
        if flags & 0x08 != 0 { // HAS_OPTIONAL
            let opt_count = r.read_u30()? as usize;
            for _ in 0..opt_count {
                r.read_u30()?; // value index
                r.read_u8()?;  // value kind
            }
        }
        if flags & 0x80 != 0 { // HAS_PARAM_NAMES
            for _ in 0..param_count { r.read_u30()?; }
        }
        let name = strings.get(name_idx as usize).cloned().unwrap_or_default();
        methods.push(Method { name_idx, name, param_count, return_type_idx });
    }

    // ── Metadata ──────────────────────────────────────────────────────────────
    let metadata_count = r.read_u30()? as usize;
    for _ in 0..metadata_count {
        r.read_u30()?; // name
        let item_count = r.read_u30()? as usize;
        for _ in 0..item_count {
            r.read_u30()?; // key
            r.read_u30()?; // value
        }
    }

    // ── Classes ───────────────────────────────────────────────────────────────
    let class_count = r.read_u30()? as usize;
    let mut classes = Vec::with_capacity(class_count);

    // Instance infos
    for _ in 0..class_count {
        let name_idx = r.read_u30()?;
        let super_name_idx = r.read_u30()?;
        let flags = r.read_u8()?;
        if flags & 0x08 != 0 { r.read_u30()?; } // protected ns
        let iface_count = r.read_u30()? as usize;
        for _ in 0..iface_count { r.read_u30()?; }
        let constructor_idx = r.read_u30()?;
        let trait_count = r.read_u30()? as usize;
        let mut instance_methods = Vec::new();
        for _ in 0..trait_count {
            if let Ok(t) = parse_trait(&mut r, &strings, &multinames) {
                instance_methods.push(t);
            }
        }
        let name = multinames.get(name_idx as usize).map(|m| m.name.clone()).unwrap_or_default();
        let super_name = multinames.get(super_name_idx as usize).map(|m| m.name.clone()).unwrap_or_default();
        // Also resolve namespace-qualified name for _fla.* classes
        let ns_name = if let Some(mn) = multinames.get(name_idx as usize) {
            if mn.kind == 0x07 || mn.kind == 0x0D {
                // QName: namespace_idx is stored, get namespace string
                let ns_idx = mn.ns_idx as usize;
                if ns_idx < namespaces.len() && !namespaces[ns_idx].is_empty() {
                    format!("{}.{}", namespaces[ns_idx], mn.name)
                } else {
                    mn.name.clone()
                }
            } else { mn.name.clone() }
        } else { name.clone() };
        let full_name = if ns_name != name && ns_name.contains("_fla.") { ns_name } else { name.clone() };
        classes.push(Class { name: full_name, super_name, instance_methods, class_methods: vec![], constructor_idx });
    }

    // Class infos (static traits)
    for i in 0..class_count {
        let _static_init = r.read_u30()?;
        let trait_count = r.read_u30()? as usize;
        for _ in 0..trait_count {
            if let Ok(t) = parse_trait(&mut r, &strings, &multinames) {
                classes[i].class_methods.push(t);
            }
        }
    }

    // ── Scripts ───────────────────────────────────────────────────────────────
    let script_count = r.read_u30()? as usize;
    let mut scripts = Vec::with_capacity(script_count);
    for _ in 0..script_count {
        let init_method_idx = r.read_u30()?;
        let trait_count = r.read_u30()? as usize;
        let mut traits = Vec::new();
        for _ in 0..trait_count {
            if let Ok(t) = parse_trait(&mut r, &strings, &multinames) {
                traits.push(t);
            }
        }
        scripts.push(Script { init_method_idx, traits });
    }

    // ── Method bodies ─────────────────────────────────────────────────────────
    let body_count = r.read_u30()? as usize;
    let mut method_bodies = Vec::with_capacity(body_count);
    for _ in 0..body_count {
        let method_idx = r.read_u30()?;
        let max_stack = r.read_u30()?;
        let local_count = r.read_u30()?;
        let _init_scope_depth = r.read_u30()?;
        let _max_scope_depth = r.read_u30()?;
        let code_len = r.read_u30()? as usize;
        let start = r.pos;
        let bytecode = if r.pos + code_len <= r.data.len() {
            let bc = r.data[start..start + code_len].to_vec();
            r.pos += code_len;
            bc
        } else {
            r.pos = r.data.len();
            vec![]
        };

        // Skip exception handlers
        let ex_count = r.read_u30().unwrap_or(0) as usize;
        for _ in 0..ex_count {
            r.read_u30().ok(); // from
            r.read_u30().ok(); // to
            r.read_u30().ok(); // target
            r.read_u30().ok(); // exc_type
            r.read_u30().ok(); // var_name
        }

        // Parse method body traits (activation slot names)
        let trait_count = r.read_u30().unwrap_or(0) as usize;
        let mut activation_traits = Vec::new();
        for _ in 0..trait_count {
            match parse_trait(&mut r, &strings, &multinames) {
                Ok(t) => activation_traits.push(t),
                Err(_) => break,
            }
        }

        method_bodies.push(MethodBody { method_idx, max_stack, local_count, bytecode, activation_traits });
    }

    log::info!("ABC: {} methods, {} classes, {} method bodies", methods.len(), classes.len(), method_bodies.len());

    Ok(AbcFile { strings, ints, uints, doubles, multinames, methods, classes, scripts, method_bodies })
}

fn parse_trait(r: &mut Reader, _strings: &[String], multinames: &[Multiname]) -> Result<Trait> {
    let name_idx = r.read_u30()?;
    let kind_byte = r.read_u8()?;
    let kind = kind_byte & 0x0F;
    let has_metadata = kind_byte & 0x40 != 0;
    let name = multinames.get(name_idx as usize).map(|m| m.name.clone()).unwrap_or_default();

    let (method_idx, slot_idx) = match kind {
        0 | 6 => { // Slot, Const
            let slot_id = r.read_u30()?;
            let _type_name = r.read_u30()?;
            let vindex = r.read_u30()?;
            if vindex != 0 { r.read_u8()?; } // vkind
            (0, slot_id)
        }
        1 | 2 | 3 => { // Method, Getter, Setter
            let _disp_id = r.read_u30()?;
            let method_idx = r.read_u30()?;
            (method_idx, 0)
        }
        4 | 5 => { // Class, Function
            let slot_id = r.read_u30()?;
            let idx = r.read_u30()?;
            (idx, slot_id)
        }
        _ => {
            return Err(anyhow!("Unknown trait kind: {}", kind));
        }
    };

    if has_metadata {
        let mc = r.read_u30()? as usize;
        for _ in 0..mc { r.read_u30()?; }
    }

    Ok(Trait { name, kind, method_idx, slot_idx })
}

// ─── Character data extraction ────────────────────────────────────────────────

/// Returns true if this frame method name is a root-MovieClip xframe setter
/// (e.g. "frame1", "frame23") as opposed to a real animation frame script
/// (e.g. "a__frame2", "stand__frame0").
/// Root xframe setters just do `self.xframe = "..."; self.stop();` and have
/// no gameplay value in Fraymakers — they only drive animation name extraction.
fn is_root_xframe_method(name: &str) -> bool {
    name.starts_with("frame")
        && name["frame".len()..].chars().all(|c| c.is_ascii_digit())
        && !name["frame".len()..].is_empty()
}

/// Extract the SSF2 animation name set by self.xframe from a frame* method's bytecode.
/// Scans for the first PUSHSTRING instruction whose value looks like an animation name.
fn extract_xframe_name(bytecode: &[u8], abc: &AbcFile) -> Option<String> {
    let mut i = 0;
    while i < bytecode.len() {
        let op = bytecode[i];
        i += 1;
        match op {
            OP_PUSHSTRING => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    if let Some(s) = abc.strings.get(idx as usize) {
                        // xframe values are short snake_case strings, not bytecode artifacts
                        if !s.is_empty() && s.len() < 40 && s.chars().all(|c| c.is_alphanumeric() || c == '_') {
                            return Some(s.clone());
                        }
                    }
                }
            }
            OP_PUSHBYTE => { i += 1; }
            OP_PUSHSHORT | OP_PUSHINT | OP_PUSHUINT | OP_PUSHDOUBLE => { read_u30_at(bytecode, &mut i); }
            OP_GETLEX | OP_FINDPROPSTRICT | OP_FINDPROP | OP_GETPROPERTY |
            OP_SETPROPERTY | OP_INITPROPERTY | OP_COERCE | OP_CALLPROPVOID |
            OP_CALLPROPERTY | OP_GETLOCAL | OP_SETLOCAL | OP_NEWARRAY | OP_NEWOBJECT => {
                read_u30_at(bytecode, &mut i);
            }
            OP_CONSTRUCTPROP => { read_u30_at(bytecode, &mut i); read_u30_at(bytecode, &mut i); }
            OP_JUMP | OP_IFTRUE | OP_IFFALSE | OP_IFEQ | OP_IFNE | OP_IFLT |
            OP_IFLE | OP_IFGT | OP_IFGE | OP_IFSTRICTEQ | OP_IFSTRICTNE => {
                if i + 3 <= bytecode.len() { i += 3; }
            }
            _ => {}
        }
    }
    None
}

/// Extract character data by analyzing ABC bytecode
pub fn extract_character(abc: &AbcFile, char_name: &str) -> Result<ExtractedCharacter> {
    let mut attacks: BTreeMap<String, AttackData> = BTreeMap::new();
    let mut projectiles: BTreeMap<String, ProjectileData> = BTreeMap::new();
    let mut stats: Option<CharStats> = None;
    let mut frame_scripts: BTreeMap<String, Vec<FrameAction>> = BTreeMap::new();
    let mut ext_methods: BTreeMap<String, String> = BTreeMap::new();
    let mut xframe_map: XframeMap = BTreeMap::new();

    // Build method name lookup: method_idx → name
    let mut method_names: BTreeMap<u32, String> = BTreeMap::new();
    for (_body_idx, body) in abc.method_bodies.iter().enumerate() {
        if let Some(method) = abc.methods.get(body.method_idx as usize) {
            if !method.name.is_empty() {
                method_names.insert(body.method_idx, method.name.clone());
            }
        }
    }

    // Find the XxxExt class (e.g. MarioExt) — holds getOwnStats + getAttackStats.
    // SSF2 class names don't always match the filename: CaptainFalcon → CaptainExt,
    // ChibiRobo → ChibiExt, GameAndWatch → gameandwatchExt, etc.
    // Strategy: try exact match, then prefix match on Ext classes, then any single Ext class.
    let char_lower = char_name.to_lowercase();
    let ext_class_name = format!("{}Ext",
        char_name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()
        + &char_name[1..]);
    let ext_classes: Vec<&Class> = abc.classes.iter()
        .filter(|c| c.name.ends_with("Ext") && c.name != "SSF2CharacterExt")
        .collect();
    let char_class = abc.classes.iter()
        // 1. Exact match: MarioExt
        .find(|c| c.name == ext_class_name)
        // 2. Case-insensitive contains: captainext contains "captain" (prefix of captainfalcon)
        .or_else(|| ext_classes.iter().copied().find(|c| {
            let cn = c.name.to_lowercase().replace("ext", "");
            char_lower.starts_with(&cn) || cn.starts_with(&char_lower)
        }))
        // 3. Only one non-base Ext class in the file
        .or_else(|| if ext_classes.len() == 1 { Some(ext_classes[0]) } else { None });

    log::info!("Character Ext class: {:?}", char_class.map(|c| &c.name));

    // Build a map from method_idx → trait name (e.g. getOwnStats, getAttackStats, frame1)
    let mut trait_name_for_method: BTreeMap<u32, String> = BTreeMap::new();
    for class in &abc.classes {
        for t in class.instance_methods.iter().chain(class.class_methods.iter()) {
            if !t.name.is_empty() {
                trait_name_for_method.insert(t.method_idx, t.name.clone());
            }
        }
    }

    // Build body lookup: method_idx → body
    let body_by_method: BTreeMap<u32, &MethodBody> = abc.method_bodies.iter()
        .map(|b| (b.method_idx, b))
        .collect();

    // --- Process MarioExt methods specifically ---
    let mut ext_vars: Vec<String> = Vec::new();
    let mut ext_var_inits: Vec<(String, String)> = Vec::new();
    if let Some(ext) = char_class {
        // Collect Slot (kind=0) / Const (kind=6) instance traits — the SSF2
        // class's `public var foo:T;` declarations. We carry these over to
        // Script.hx as top-level `var foo;` so the references later in the
        // translated methods aren't undeclared.
        for t in &ext.instance_methods {
            if (t.kind == 0 || t.kind == 6) && !t.name.is_empty() {
                ext_vars.push(t.name.clone());
            }
        }
        // Decompile the constructor (iinit) and pull out `self.<var> = expr;`
        // assignments for each ext_var. This recovers the initial values the
        // AS3 source wrote as `public var foo:T = expr;` (the compiler puts
        // them at the top of iinit) so the generator can emit them inside
        // initialize().
        if !ext_vars.is_empty() {
            if let Some(iinit_body) = body_by_method.get(&ext.constructor_idx) {
                let params: Vec<String> = if let Some(m) = abc.methods.get(iinit_body.method_idx as usize) {
                    (0..m.param_count).map(|i| format!("arg{}", i)).collect()
                } else { vec![] };
                let raw = decompiler::decompile_method(iinit_body, abc, "iinit", &params);
                let translated = crate::api_mappings::translate_ssf2_to_fm(&raw);
                let mut seen: std::collections::BTreeSet<String> = Default::default();
                for line in translated.lines() {
                    let trimmed = line.trim();
                    for v in &ext_vars {
                        let prefix = format!("self.{} = ", v);
                        if let Some(rest) = trimmed.strip_prefix(&prefix) {
                            if seen.insert(v.clone()) {
                                let rhs = rest.trim_end_matches(';').trim().to_string();
                                ext_var_inits.push((v.clone(), rhs));
                            }
                            break;
                        }
                    }
                }
            }
        }
        for t in &ext.instance_methods {
            let Some(body) = body_by_method.get(&t.method_idx) else { continue };
            match t.name.as_str() {
                "getOwnStats" => {
                    // getOwnStats contains the big character stats newobject.
                    // We scan the bytecode for specific SSF2 stat key pushes
                    // followed by numeric values.
                    if let Some(s) = extract_ssf2_stats(&body.bytecode, abc) {
                        log::info!("getOwnStats: extracted {} stat values", s.values.len());
                        stats = Some(s);
                    }
                }
                "getAttackStats" => {
                    let extracted = extract_attack_objects(&body.bytecode, abc);
                    log::info!("getAttackStats: extracted {} attacks", extracted.len());
                    attacks.extend(extracted);
                }
                "getProjectileStats" => {
                    let extracted = extract_projectile_objects(&body.bytecode, abc);
                    log::info!("getProjectileStats: extracted {} per-attack projectile entries", extracted.len());
                    projectiles.extend(extracted);
                }
                name if name.starts_with("frame") => {
                    // Extract xframe animation name first (always, for animation name mapping)
                    if let Some(anim_name) = extract_xframe_name(&body.bytecode, abc) {
                        xframe_map.insert(name.to_string(), anim_name);
                        // Root xframe setters (frame1, frame23, ...) only set self.xframe and stop.
                        // They have no gameplay value in Fraymakers — skip adding to frame_scripts.
                        if is_root_xframe_method(name) { continue; }
                    }
                    // Use decompiler for full Haxe output (real animation frame scripts only)
                    let params: Vec<String> = if let Some(method) = abc.methods.get(body.method_idx as usize) {
                        (0..method.param_count).map(|i| format!("arg{}", i)).collect()
                    } else { vec![] };
                    let code = decompiler::decompile_method(body, abc, name, &params);
                    frame_scripts.insert(name.to_string(), vec![FrameAction {
                        frame: 0,
                        action: code,
                        args: vec![],
                    }]);
                }
                // Decompile all other Ext methods for Script.hx
                // Skip slot/const traits (kind 0/6) — those are variable declarations, not methods
                name if !matches!(name, "getOwnStats" | "getAttackStats" | "getItemStats" | "getProjectileStats") => {
                    // Only decompile actual method traits (kind 1/2/3), not slots (kind 0/6)
                    // The trait.kind is stored in the Trait struct; method_idx > 0 means it's a real method
                    if t.kind & 0x0F != 0 || t.slot_idx == 0 {
                        // Get param count from method signature
                        let params: Vec<String> = if let Some(method) = abc.methods.get(body.method_idx as usize) {
                            (0..method.param_count).map(|i| format!("arg{}", i)).collect()
                        } else {
                            vec![]
                        };
                        let code = decompiler::decompile_method(body, abc, name, &params);
                        ext_methods.insert(name.to_string(), code);
                    }
                }
                _ => {}
            }
        }
    }

    // --- Also scan the main character class for frame scripts ---
    // The class name matches the filename for most chars but not all:
    // captainfalcon.ssf -> class 'captainfalcon'
    // gameandwatch.ssf -> class 'gameAndWatch'
    // Pick the class whose name most closely matches char_name AND has the most frame* methods.
    let main_class = abc.classes.iter()
        // 1. Exact case-insensitive match
        .find(|c| c.name.to_lowercase() == char_lower)
        // 2. Last resort: whichever non-Ext, non-fla, non-proj class has the MOST frame* methods.
        //    Handles cases like captainfalcon.ssf -> class 'falcon', chibirobo.ssf -> 'chibirobo', etc.
        .or_else(|| {
            abc.classes.iter()
                .filter(|c| {
                    let cn = c.name.to_lowercase();
                    !cn.ends_with("ext") && !cn.contains("_fla") && !cn.contains("api")
                        && !cn.contains("proj") && !cn.contains("helper") && !cn.contains("_hud")
                })
                .max_by_key(|c| c.instance_methods.iter().filter(|t| t.name.starts_with("frame")).count())
                .filter(|c| c.instance_methods.iter().any(|t| t.name.starts_with("frame")))
        });
    if let Some(mc) = main_class {
        log::info!("Main class '{}': {} frame methods", mc.name, mc.instance_methods.len());
        // Helper methods on the main class — anything that isn't a frame*
        // entry or a stat-getter — are decompiled and emitted in Script.hx
        // alongside the Ext-class methods. Calls to per-character helpers
        // like `aimTrampoline`, `chef`, `eat`, `gokuKaiokenHUD` would
        // otherwise resolve to nothing (the helpers exist only on the main
        // class, not the Ext class).
        const STAT_GETTERS: &[&str] = &["getOwnStats", "getAttackStats", "getItemStats", "getProjectileStats"];
        for t in &mc.instance_methods {
            if t.name.starts_with("frame") {
                let Some(body) = body_by_method.get(&t.method_idx) else { continue };
                // Extract xframe name
                if let Some(anim_name) = extract_xframe_name(&body.bytecode, abc) {
                    xframe_map.insert(t.name.clone(), anim_name);
                }
                let params: Vec<String> = if let Some(method) = abc.methods.get(body.method_idx as usize) {
                    (0..method.param_count).map(|i| format!("arg{}", i)).collect()
                } else { vec![] };
                let code = decompiler::decompile_method(body, abc, &t.name, &params);
                frame_scripts.insert(t.name.clone(), vec![FrameAction {
                    frame: 0,
                    action: code,
                    args: vec![],
                }]);
            } else if matches!(t.kind, 1 | 2 | 3) && !t.name.is_empty()
                && !STAT_GETTERS.contains(&t.name.as_str())
                && !ext_methods.contains_key(&t.name)
            {
                // kind 1/2/3 = Method/Getter/Setter; skip Slot/Const (0/6) and
                // anything the Ext class already provides under the same name.
                let Some(body) = body_by_method.get(&t.method_idx) else { continue };
                let params: Vec<String> = if let Some(method) = abc.methods.get(body.method_idx as usize) {
                    (0..method.param_count).map(|i| format!("arg{}", i)).collect()
                } else { vec![] };
                let code = decompiler::decompile_method(body, abc, &t.name, &params);
                ext_methods.insert(t.name.clone(), code);
            }
        }
    }

    // --- Sub-MovieClip helper methods ---
    // Some per-character helpers (e.g. pacman's `aimTrampoline` on
    // `pacman_fla.UpSpecial_37`) live on sub-MC `_fla.*` classes rather than
    // the main class or the Ext class. Their frame-script callers reference
    // them by bare name, so they need to land in Script.hx like the other
    // helpers — otherwise the calls resolve to nothing. Scope the scan to
    // classes that belong to THIS character (name starts with the char
    // lowercase prefix) so we don't suck in SSF2's framework classes
    // (Controls, Matrix3D, KeyboardOptions, etc.) that are unrelated and
    // would flood Script.hx with generic utility methods.
    {
        const STAT_GETTERS: &[&str] = &["getOwnStats", "getAttackStats", "getItemStats", "getProjectileStats"];
        for class in &abc.classes {
            // Skip the Ext class and the main class (already processed).
            if Some(class.name.as_str()) == char_class.map(|c| c.name.as_str()) { continue; }
            if Some(class.name.as_str()) == main_class.map(|c| c.name.as_str()) { continue; }
            // Limit to this character's classes: same prefix as the main
            // class name (or the char name), so `pacman_fla.UpSpecial_37` is
            // in but `Controls` / `Matrix3D` are out. Also drop projectile-
            // specific sub-MCs — those have their own converter path.
            let cn = class.name.to_lowercase();
            let main_prefix = main_class.map(|c| c.name.to_lowercase()).unwrap_or_default();
            let main_root = main_prefix.split(['.', '_']).next().unwrap_or(&char_lower);
            let belongs = !main_root.is_empty()
                && (cn.starts_with(main_root) || cn.starts_with(&char_lower));
            if !belongs { continue; }
            if cn.contains("proj") || cn.contains("helper") { continue; }
            for t in &class.instance_methods {
                if !matches!(t.kind, 1 | 2 | 3) { continue; }
                if t.name.is_empty() || t.name.starts_with("frame") { continue; }
                if STAT_GETTERS.contains(&t.name.as_str()) { continue; }
                if ext_methods.contains_key(&t.name) { continue; }
                let Some(body) = body_by_method.get(&t.method_idx) else { continue };
                let params: Vec<String> = if let Some(m) = abc.methods.get(body.method_idx as usize) {
                    (0..m.param_count).map(|i| format!("arg{}", i)).collect()
                } else { vec![] };
                let code = decompiler::decompile_method(body, abc, &t.name, &params);
                ext_methods.insert(t.name.clone(), code);
            }
        }
    }

    // --- Also scan script-level traits for frame* methods ---
    // In SSF2, the character's main MovieClip frame scripts are compiled as
    // script-level (top-level) functions, NOT as class instance methods.
    // These appear in abc.scripts[*].traits, not in any class.
    if xframe_map.is_empty() || frame_scripts.is_empty() {
        for script in &abc.scripts {
            for t in &script.traits {
                if !t.name.starts_with("frame") { continue; }
                let Some(body) = body_by_method.get(&t.method_idx) else { continue };
                if let Some(anim_name) = extract_xframe_name(&body.bytecode, abc) {
                    xframe_map.entry(t.name.clone()).or_insert(anim_name);
                }
                if !frame_scripts.contains_key(&t.name) {
                    let params: Vec<String> = if let Some(method) = abc.methods.get(body.method_idx as usize) {
                        (0..method.param_count).map(|i| format!("arg{}", i)).collect()
                    } else { vec![] };
                    let code = decompiler::decompile_method(body, abc, &t.name, &params);
                    frame_scripts.insert(t.name.clone(), vec![FrameAction {
                        frame: 0,
                        action: code,
                        args: vec![],
                    }]);
                }
            }
        }
        if !xframe_map.is_empty() {
            log::info!("Found {} xframe mappings in script-level traits", xframe_map.len());
        }
    }

    // --- Extract sub-MC frame scripts ---
    // SSF2 animation MovieClips (e.g. captainfalcon_fla.JabCombo_47) register per-frame
    // logic via addFrameScript(N, closure) inside their class iinit.
    // Each class name encodes the main-timeline frame number as a suffix (e.g. _47).
    // We map that back to an animation name via xframe_map (e.g. frame47 → "a").
    //
    // Find the addFrameScript multiname index
    let afs_mn_idx = abc.multinames.iter().position(|mn| mn.name == "addFrameScript");
    if let Some(afs_idx) = afs_mn_idx {
        // Encode afs_idx as u30 for pattern matching
        let mut afs_enc = Vec::new();
        let mut v = afs_idx as u32;
        loop { if v < 0x80 { afs_enc.push(v as u8); break; } afs_enc.push((v as u8 & 0x7F) | 0x80); v >>= 7; }
        let callpropvoid_afs: Vec<u8> = std::iter::once(0x4Fu8).chain(afs_enc.iter().copied()).collect();
        let callproperty_afs: Vec<u8> = std::iter::once(0x46u8).chain(afs_enc.iter().copied()).collect();

        // Build ssf2→fm map for extract_ssf2_anim_name lookups
        let xframe_fm_map: BTreeMap<String, String> = xframe_map.values()
            .map(|ssf2| (ssf2.clone(), ssf2.clone())) // identity for now, haxe_gen does FM translation
            .collect();

        let fla_classes: Vec<&Class> = abc.classes.iter()
            .filter(|c| c.name.contains("_fla."))
            .collect();
        log::info!("Found {} _fla. sub-MC classes", fla_classes.len());
        if fla_classes.is_empty() {
            // Dump first 10 class names for debugging
            let names: Vec<&str> = abc.classes.iter().take(20).map(|c| c.name.as_str()).collect();
            log::info!("First 20 class names: {:?}", names);
        }
        for class in &abc.classes {
            // Only process captainfalcon_fla.* style sub-MC classes
            if !class.name.contains("_fla.") { continue; }

            // Map sub-MC class name → SSF2 animation name using the same mapping
            // as sprite_parser (e.g. "JabCombo" → "a", "NAir" → "a_air")
            let ssf2_anim = crate::sprite_parser::extract_ssf2_anim_name(
                &class.name, &char_lower, &xframe_fm_map
            );
            let Some(anim_name) = ssf2_anim else {
                log::debug!("Sub-MC '{}': no animation name mapping, skipping", class.name);
                continue;
            };

            // Get iinit body
            let Some(iinit_body) = body_by_method.get(&class.constructor_idx) else {
                log::debug!("Sub-MC '{}': no body for constructor_idx {}", class.name, class.constructor_idx);
                continue;
            };
            let bc = &iinit_body.bytecode;
            log::debug!("Sub-MC '{}': iinit {} bytes, anim='{}'", class.name, bc.len(), anim_name);

            // Quick scan: does this iinit contain addFrameScript at all?
            let has_afs = bc.windows(callpropvoid_afs.len()).any(|w| w == callpropvoid_afs.as_slice())
                || bc.windows(callproperty_afs.len()).any(|w| w == callproperty_afs.as_slice());
            if !has_afs {
                log::debug!("Sub-MC '{}': no addFrameScript call in iinit", class.name);
            }

            // Scan iinit for addFrameScript(frameNum, closure) pairs
            // Pattern: pushbyte/pushshort <N>, newfunction <method_idx>, callpropvoid(addFrameScript, 2)
            let mut i = 0usize;
            while i < bc.len() {
                // Match callpropvoid(addFrameScript, argc=2)
                if bc[i..].starts_with(&callpropvoid_afs) || bc[i..].starts_with(&callproperty_afs) {
                    // Check arg count = 2 after the multiname
                    let after_mn = i + callpropvoid_afs.len();
                    let mut tmp = after_mn;
                    if let Some(argc) = read_u30_at(bc, &mut tmp) {
                        if argc >= 2 && argc % 2 == 0 {
                            // addFrameScript is variadic: addFrameScript(f0, fn0, f1, fn1, ...)
                            // SSF2 pattern: pushbyte N, getlocal0, getproperty MN (bound method ref)
                            // OR: pushbyte N, newfunction M (inline closure)
                            let scan_start = 0usize; // scan entire iinit
                            let mut pairs: Vec<(u32, String, Option<u32>)> = Vec::new(); // (frame, method_name, method_idx)
                            let mut j = scan_start;
                            let mut pending_frame: Option<u32> = None;
                            while j < i {
                                if bc[j] == OP_PUSHBYTE && j + 1 < bc.len() {
                                    pending_frame = Some(bc[j + 1] as u32);
                                    j += 2;
                                } else if bc[j] == OP_PUSHSHORT {
                                    let mut k = j + 1;
                                    if let Some(v) = read_u30_at(bc, &mut k) {
                                        pending_frame = Some(v);
                                        j = k;
                                    } else { j += 1; }
                                } else if bc[j] == OP_PUSHINT {
                                    let mut k = j + 1;
                                    if let Some(v) = read_u30_at(bc, &mut k) {
                                        let int_val = abc.ints.get(v as usize).copied().unwrap_or(0);
                                        pending_frame = Some(int_val as u32);
                                        j = k;
                                    } else { j += 1; }
                                } else if bc[j] == OP_GETPROPERTY {
                                    let mut k = j + 1;
                                    if let Some(mn_idx) = read_u30_at(bc, &mut k) {
                                        if let Some(frame) = pending_frame.take() {
                                            let method_name = abc.multinames.get(mn_idx as usize)
                                                .map(|m| m.name.clone()).unwrap_or_default();
                                            // Find the method_idx for this method name in this class
                                            let midx = class.instance_methods.iter()
                                                .find(|t| t.name == method_name)
                                                .map(|t| t.method_idx);
                                            pairs.push((frame, method_name, midx));
                                        }
                                        j = k;
                                    } else { j += 1; }
                                } else if bc[j] == 0x40 { // newfunction
                                    let mut k = j + 1;
                                    if let Some(midx) = read_u30_at(bc, &mut k) {
                                        if let Some(frame) = pending_frame.take() {
                                            pairs.push((frame, format!("closure_{}", midx), Some(midx)));
                                        }
                                        j = k;
                                    } else { j += 1; }
                                } else {
                                    // Skip other opcodes (getlocal0, etc.) without clearing pending_frame
                                    j += 1;
                                }
                            }

                            log::debug!("Sub-MC '{}' anim='{}': addFrameScript argc={}, found {} pairs",
                                class.name, anim_name, argc, pairs.len());

                            for (sub_f, method_name, midx) in &pairs {
                                let body = midx.and_then(|mi| abc.method_bodies.iter().find(|b| b.method_idx == mi));
                                if let Some(closure_body) = body {
                                    let param_count = abc.methods.get(*midx.as_ref().unwrap() as usize)
                                        .map(|m| m.param_count as usize).unwrap_or(0);
                                    let params: Vec<String> = (0..param_count)
                                        .map(|pi| format!("arg{}", pi)).collect();
                                    let code = decompiler::decompile_method(
                                        closure_body, abc,
                                        &format!("{}__frame{}", anim_name, sub_f),
                                        &params
                                    );
                                    let script_key = format!("{}__frame{}", anim_name, sub_f);
                                    frame_scripts.entry(script_key).or_insert_with(Vec::new)
                                        .push(FrameAction {
                                            frame: *sub_f,
                                            action: code,
                                            args: vec![],
                                        });
                                } else {
                                    log::debug!("  frame {} method '{}': no body found", sub_f, method_name);
                                }
                            }

                            i = tmp; // advance past the call
                            continue;
                        }
                    }
                }
                i += 1;
            }
        }

        // Count only sub-MC entries (keys containing "__frame")
        let sub_mc_count = frame_scripts.keys().filter(|k| k.contains("__frame")).count();
        let total_fs = frame_scripts.len();
        log::info!("Sub-MC frame scripts extracted: {} new entries (total frame_scripts now: {})", sub_mc_count, total_fs);
    }

    // --- Fallback: scan ALL method bodies for xframe mappings ---
    // In SSF2, setproperty(self.xframe = "a") appears inside anonymous timeline
    // frame methods linked via SymbolClass, not named class traits. These are
    // compile-time closures with no discoverable name via trait scanning.
    // Strategy: scan every method body, collect pushstring values that immediately
    // precede setproperty(xframe), and build the full ssf2 animation name set.
    if xframe_map.is_empty() {
        // Find the xframe multiname index
        let xframe_mn_idx = abc.multinames.iter().enumerate()
            .find(|(_, mn)| mn.name == "xframe")
            .map(|(i, _)| i);

        // Also check ALL multinames named 'xframe' (might be QName and Multiname variants)
        let xframe_mn_indices: Vec<usize> = abc.multinames.iter().enumerate()
            .filter(|(_, mn)| mn.name == "xframe")
            .map(|(i, _)| i)
            .collect();
        log::debug!("xframe multiname indices: {:?}", xframe_mn_indices);

        if let Some(xf_idx) = xframe_mn_idx {
            // Build patterns for ALL xframe multiname indices
            let mut all_patterns: Vec<Vec<u8>> = Vec::new();
            for idx in &xframe_mn_indices {
                let mut enc = Vec::new();
                let mut v = *idx as u32;
                loop {
                    if v < 0x80 { enc.push(v as u8); break; }
                    enc.push((v as u8 & 0x7F) | 0x80);
                    v >>= 7;
                }
                // setproperty = 0x61, initproperty = 0x68, setlex = nope
                all_patterns.push(std::iter::once(0x61u8).chain(enc.iter().copied()).collect());
                all_patterns.push(std::iter::once(0x68u8).chain(enc.iter().copied()).collect());
            }
            let _ = xf_idx; // suppress warning
            let _setprop_pattern = all_patterns[0].clone(); // for compat, keep first

            let mut seen_anims: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for body in &abc.method_bodies {
                let bc = &body.bytecode;
                // Scan for setproperty/initproperty(xframe) using all known multiname indices
                let mut i = 0usize;
                while i < bc.len() {
                    let matched = all_patterns.iter().any(|p| bc[i..].starts_with(p));
                    if matched {
                        // Scan backwards from i for the most recent pushstring
                        let mut j = i.saturating_sub(1);
                        while j > 0 {
                            if bc[j] == 0x2C {
                                // pushstring: decode u30 idx
                                let mut k = j + 1;
                                if let Some(si) = read_u30_at(bc, &mut k) {
                                    if let Some(s) = abc.strings.get(si as usize) {
                                        if !s.is_empty() && s.len() < 40
                                            && s.chars().all(|c| c.is_alphanumeric() || c == '_')
                                        {
                                            seen_anims.insert(s.clone());
                                        }
                                    }
                                }
                                break;
                            }
                            j -= 1;
                        }
                        let pat_len = all_patterns.iter().find(|p| bc[i..].starts_with(p.as_slice())).map(|p| p.len()).unwrap_or(1);
                        i += pat_len;
                    } else {
                        i += 1;
                    }
                }
            }

            if !seen_anims.is_empty() {
                log::info!("xframe fallback: found {} SSF2 animation names across all method bodies", seen_anims.len());
                // We can't map frame*→anim name one-to-one here (no frame method names),
                // but we CAN populate the extractor's ssf2_to_fm table used by sprite_parser.
                // Emit a synthetic mapping: ssf2_name → ssf2_name (identity) so the sprite
                // parser can at least match sprites. Real FM name mapping uses the static table.
                for anim in &seen_anims {
                    xframe_map.entry(format!("__all_{}", anim)).or_insert(anim.clone());
                }
            }
        }
    }

    // --- Fallback: scan all bodies if we got nothing ---
    if attacks.is_empty() {
        log::warn!("getAttackStats extraction yielded nothing, falling back to full scan");
        for body in &abc.method_bodies {
            let extracted = extract_attack_objects(&body.bytecode, abc);
            for (name, data) in extracted {
                attacks.entry(name).or_insert(data);
            }
        }
    }
    if stats.is_none() {
        for body in &abc.method_bodies {
            if let Some(s) = extract_stats_from_body(&body.bytecode, abc) {
                stats = Some(s);
                break;
            }
        }
    }

    // SSF2 costume/palette data lives in the engine SWF, not the character SWF.
    // Both getCostumeData and applyPalette are thin wrappers that call m_api at runtime.
    // We cannot extract the actual costume color data from here.
    // palette_gen.rs builds costumes from sprite imagery instead.
    let costumes: Vec<CostumeData> = vec![];
    log::info!("Extracted {} attacks, {} frame scripts, {} ext methods, {} xframe mappings, {} costumes, stats={}",
        attacks.len(), frame_scripts.len(), ext_methods.len(), xframe_map.len(), costumes.len(), stats.is_some());

    Ok(ExtractedCharacter {
        name: char_name.to_string(),
        attacks,
        projectiles,
        stats,
        frame_scripts,
        ext_methods,
        ext_vars,
        ext_var_inits,
        xframe_map,
        costumes,
    })
}

/// Simulate the AVM2 stack to extract object literals from bytecode.
/// SSF2 attack data structure:
///   newobject(N) where one key is 'attackBoxes' → value is newobject(M hitboxes)
///   each hitbox is newobject(10) with keys: damage, priority, hitStun, hitLag,
///     effect_id, direction, weightKB, power, kbConstant, effectSound
/// The top-level getAttackStats builds: newobject(attack_count) where keys are move names
#[derive(Debug, Clone)]
enum StackVal {
    Str(String),
    Num(f64),
    Bool(()),  // unused field — kept for stack value compatibility
    Null,
    /// A parsed object literal from newobject
    Obj(BTreeMap<String, StackVal>),
    /// A parsed array from newarray
    Arr(Vec<StackVal>),
    Unknown,
}

fn extract_attack_objects(bytecode: &[u8], abc: &AbcFile) -> BTreeMap<String, AttackData> {
    let mut result: BTreeMap<String, AttackData> = BTreeMap::new();
    let mut stack: Vec<StackVal> = Vec::new();
    let mut i = 0;

    while i < bytecode.len() {
        let op = bytecode[i];
        i += 1;

        match op {
            OP_PUSHSTRING => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    let s = abc.strings.get(idx as usize).cloned().unwrap_or_default();
                    stack.push(StackVal::Str(s));
                }
            }
            OP_PUSHDOUBLE => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    let v = abc.doubles.get(idx as usize).copied().unwrap_or(0.0);
                    stack.push(StackVal::Num(v));
                }
            }
            OP_PUSHBYTE => {
                if i < bytecode.len() {
                    let v = bytecode[i] as i8 as f64;
                    i += 1;
                    stack.push(StackVal::Num(v));
                }
            }
            OP_PUSHSHORT => {
                if let Some(v) = read_u30_at(bytecode, &mut i) {
                    stack.push(StackVal::Num(v as i16 as f64));
                }
            }
            OP_PUSHINT => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    let v = abc.ints.get(idx as usize).copied().unwrap_or(0) as f64;
                    stack.push(StackVal::Num(v));
                }
            }
            OP_PUSHUINT => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    let v = abc.uints.get(idx as usize).copied().unwrap_or(0) as f64;
                    stack.push(StackVal::Num(v));
                }
            }
            OP_PUSHTRUE  => stack.push(StackVal::Bool(())),
            OP_PUSHFALSE => stack.push(StackVal::Bool(())),
            OP_PUSHNULL | OP_PUSHNAN => stack.push(StackVal::Null),

            OP_NEWOBJECT => {
                if let Some(count) = read_u30_at(bytecode, &mut i) {
                    let count = count as usize;
                    let needed = count * 2;
                    let mut obj: BTreeMap<String, StackVal> = BTreeMap::new();
                    if stack.len() >= needed {
                        let pairs: Vec<_> = stack.drain(stack.len() - needed..).collect();
                        for chunk in pairs.chunks(2) {
                            if let StackVal::Str(k) = &chunk[0] {
                                obj.insert(k.clone(), chunk[1].clone());
                            }
                        }
                    }

                    // Check if this is a top-level attacks object:
                    // keys are move names ("a", "b", "a_air", etc.)
                    let attack_keys_found: Vec<_> = obj.keys()
                        .filter(|k| is_attack_name(k))
                        .cloned().collect();

                    if !attack_keys_found.is_empty() {
                        // This is the top-level move map
                        for move_name in &attack_keys_found {
                            let fm_name = normalize_attack_name(move_name);
                            if let Some(val) = obj.get(move_name) {
                                let hitboxes = extract_hitboxes_from_val(val);
                                if !hitboxes.is_empty() {
                                    result.insert(fm_name, AttackData { hitboxes });
                                }
                            }
                        }
                        // Also check non-attack-name keys that might contain moves (e.g. grouped)
                        stack.push(StackVal::Obj(obj));
                    } else {
                        stack.push(StackVal::Obj(obj));
                    }
                }
            }

            OP_NEWARRAY => {
                if let Some(count) = read_u30_at(bytecode, &mut i) {
                    let drain = stack.len().min(count as usize);
                    let _items: Vec<_> = if stack.len() >= drain {
                        stack.drain(stack.len() - drain..).collect()
                    } else { vec![] };
                    stack.push(StackVal::Unknown);
                }
            }

            OP_CALLPROPERTY | OP_CALLPROPVOID => {
                let _mn_idx = read_u30_at(bytecode, &mut i).unwrap_or(0);
                let arg_count = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
                let drain = stack.len().min(arg_count + 1);
                stack.drain(stack.len() - drain..);
                if op == OP_CALLPROPERTY { stack.push(StackVal::Unknown); }
            }

            OP_SETPROPERTY | OP_INITPROPERTY => {
                let _mn_idx = read_u30_at(bytecode, &mut i).unwrap_or(0);
                if stack.len() >= 2 { stack.truncate(stack.len() - 2); }
            }

            OP_GETPROPERTY => {
                // Bug §3.5: getproperty pops the receiver and pushes the
                // property VALUE, not its name. We don't track object
                // contents through this stack sim, so push Unknown — that
                // way arithmetic / conditional ops that consume the value
                // don't accidentally interpret the property-NAME as a
                // String operand.
                read_u30_at(bytecode, &mut i);
                if !stack.is_empty() { stack.pop(); }
                stack.push(StackVal::Unknown);
            }

            OP_FINDPROPSTRICT | OP_FINDPROP | OP_GETLEX => {
                read_u30_at(bytecode, &mut i);
                stack.push(StackVal::Unknown);
            }
            OP_COERCE | OP_COERCE_A | OP_CONVERT_D | OP_CONVERT_I => {
                if op == OP_COERCE { read_u30_at(bytecode, &mut i); }
            }
            OP_NOP | OP_LABEL => {}
            OP_POP => { stack.pop(); }
            OP_DUP => { if let Some(top) = stack.last().cloned() { stack.push(top); } }
            OP_SWAP => { let len = stack.len(); if len >= 2 { stack.swap(len-1, len-2); } }
            OP_NEGATE => {
                match stack.pop() {
                    Some(StackVal::Num(v)) => stack.push(StackVal::Num(-v)),
                    _ => stack.push(StackVal::Unknown),
                }
            }
            OP_ADD | OP_SUBTRACT | OP_MULTIPLY | OP_DIVIDE => {
                let b = stack.pop(); let a = stack.pop();
                match (a, b) {
                    (Some(StackVal::Num(a)), Some(StackVal::Num(b))) => {
                        let r = match op {
                            OP_ADD => a+b, OP_SUBTRACT => a-b,
                            OP_MULTIPLY => a*b, OP_DIVIDE => a/b, _ => 0.0
                        };
                        stack.push(StackVal::Num(r));
                    }
                    _ => stack.push(StackVal::Unknown),
                }
            }
            OP_CONSTRUCTPROP => {
                read_u30_at(bytecode, &mut i);
                let argc = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
                let drain = stack.len().min(argc + 1);
                stack.drain(stack.len() - drain..);
                stack.push(StackVal::Unknown);
            }
            OP_CONSTRUCT => {
                let argc = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
                let drain = stack.len().min(argc + 1);
                stack.drain(stack.len() - drain..);
                stack.push(StackVal::Unknown);
            }
            OP_GETLOCAL0 | OP_GETLOCAL1 | OP_GETLOCAL2 | OP_GETLOCAL3 => stack.push(StackVal::Unknown),
            OP_GETLOCAL => { read_u30_at(bytecode, &mut i); stack.push(StackVal::Unknown); }
            OP_SETLOCAL0 | OP_SETLOCAL1 | OP_SETLOCAL2 | OP_SETLOCAL3 => { stack.pop(); }
            OP_SETLOCAL => { read_u30_at(bytecode, &mut i); stack.pop(); }
            OP_RETURNVALUE => { stack.pop(); }
            OP_RETURNVOID => {}
            OP_JUMP | OP_IFTRUE | OP_IFFALSE | OP_IFEQ | OP_IFNE | OP_IFLT |
            OP_IFLE | OP_IFGT | OP_IFGE | OP_IFSTRICTEQ | OP_IFSTRICTNE => {
                if i + 3 <= bytecode.len() { i += 3; }
                if op != OP_JUMP { stack.pop(); }
            }
            _ => {}
        }

        if stack.len() > 512 { stack.drain(0..256); }
    }

    result
}

/// Extract per-projectile stat objects from a `getProjectileStats()`
/// bytecode body. Mirrors `extract_attack_objects` but recognises
/// projectile-shaped objects: an object is a projectile if it carries
/// any flat-scalar physics field (gravity / xSpeed / ySpeed / friction /
/// etc.) OR a nested `attackBoxes` hitbox map.
///
/// Returns a map keyed by the SSF2 projectile name (the key under which
/// the per-projectile object appears in the top-level returned object).
fn extract_projectile_objects(bytecode: &[u8], abc: &AbcFile) -> BTreeMap<String, ProjectileData> {
    let mut result: BTreeMap<String, ProjectileData> = BTreeMap::new();
    let mut stack: Vec<StackVal> = Vec::new();
    let mut i = 0;

    // Physics keys SSF2 commonly puts on a projectile-stats object. Any
    // top-level value object holding at least one of these (or an
    // `attackBoxes` key) is treated as a projectile entry.
    const PHYSICS_KEYS: &[&str] = &[
        "gravity", "friction", "weight", "fall_speed", "terminalVelocity",
        "xSpeed", "ySpeed", "x_speed", "y_speed",
        "groundSpeedCap", "aerialSpeedCap", "aerialFriction",
        "ground_speed_cap", "aerial_speed_cap", "aerial_friction",
    ];

    while i < bytecode.len() {
        let op = bytecode[i];
        i += 1;
        match op {
            OP_PUSHSTRING => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    let s = abc.strings.get(idx as usize).cloned().unwrap_or_default();
                    stack.push(StackVal::Str(s));
                }
            }
            OP_PUSHDOUBLE => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    let v = abc.doubles.get(idx as usize).copied().unwrap_or(0.0);
                    stack.push(StackVal::Num(v));
                }
            }
            OP_PUSHBYTE => {
                if i < bytecode.len() {
                    let v = bytecode[i] as i8 as f64;
                    i += 1;
                    stack.push(StackVal::Num(v));
                }
            }
            OP_PUSHSHORT => {
                if let Some(v) = read_u30_at(bytecode, &mut i) {
                    stack.push(StackVal::Num(v as i16 as f64));
                }
            }
            OP_PUSHINT => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    let v = abc.ints.get(idx as usize).copied().unwrap_or(0) as f64;
                    stack.push(StackVal::Num(v));
                }
            }
            OP_PUSHUINT => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    let v = abc.uints.get(idx as usize).copied().unwrap_or(0) as f64;
                    stack.push(StackVal::Num(v));
                }
            }
            OP_PUSHTRUE | OP_PUSHFALSE => stack.push(StackVal::Bool(())),
            OP_PUSHNULL | OP_PUSHNAN => stack.push(StackVal::Null),
            OP_NEWOBJECT => {
                if let Some(count) = read_u30_at(bytecode, &mut i) {
                    let count = count as usize;
                    let needed = count * 2;
                    let mut obj: BTreeMap<String, StackVal> = BTreeMap::new();
                    if stack.len() >= needed {
                        let pairs: Vec<_> = stack.drain(stack.len() - needed..).collect();
                        for chunk in pairs.chunks(2) {
                            if let StackVal::Str(k) = &chunk[0] {
                                obj.insert(k.clone(), chunk[1].clone());
                            }
                        }
                    }
                    // Top-level projectile-map detection: any key whose
                    // value object has physics-y / attackBoxes shape gets
                    // promoted to a ProjectileData entry.
                    let mut found = false;
                    for (proj_name, val) in &obj {
                        if let StackVal::Obj(inner) = val {
                            let is_proj = inner.contains_key("attackBoxes")
                                || PHYSICS_KEYS.iter().any(|k| inner.contains_key(*k));
                            if is_proj {
                                let mut stats: BTreeMap<String, f64> = BTreeMap::new();
                                for (k, v) in inner {
                                    if let StackVal::Num(n) = v {
                                        stats.insert(k.clone(), *n);
                                    }
                                }
                                let hitboxes = extract_hitboxes_from_val(val);
                                result.insert(proj_name.clone(), ProjectileData { stats, hitboxes });
                                found = true;
                            }
                        }
                    }
                    if !found { stack.push(StackVal::Obj(obj)); }
                }
            }
            OP_NEWARRAY => {
                if let Some(count) = read_u30_at(bytecode, &mut i) {
                    let drain = stack.len().min(count as usize);
                    let _: Vec<_> = if stack.len() >= drain {
                        stack.drain(stack.len() - drain..).collect()
                    } else { vec![] };
                    stack.push(StackVal::Unknown);
                }
            }
            OP_CALLPROPERTY | OP_CALLPROPVOID => {
                let _ = read_u30_at(bytecode, &mut i);
                let arg_count = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
                let drain = stack.len().min(arg_count + 1);
                stack.drain(stack.len() - drain..);
                if op == OP_CALLPROPERTY { stack.push(StackVal::Unknown); }
            }
            OP_SETPROPERTY | OP_INITPROPERTY => {
                let _ = read_u30_at(bytecode, &mut i);
                if stack.len() >= 2 { stack.truncate(stack.len() - 2); }
            }
            OP_GETPROPERTY => {
                // Bug §3.5: push the value (unknown to this stack sim),
                // not the property name. Pushing the name would let it
                // be consumed as a Str operand by downstream ops.
                read_u30_at(bytecode, &mut i);
                if !stack.is_empty() { stack.pop(); }
                stack.push(StackVal::Unknown);
            }
            OP_FINDPROPSTRICT | OP_FINDPROP | OP_GETLEX => {
                read_u30_at(bytecode, &mut i);
                stack.push(StackVal::Unknown);
            }
            OP_COERCE | OP_COERCE_A | OP_CONVERT_D | OP_CONVERT_I => {
                if op == OP_COERCE { read_u30_at(bytecode, &mut i); }
            }
            OP_NOP | OP_LABEL => {}
            OP_POP => { stack.pop(); }
            OP_DUP => { if let Some(top) = stack.last().cloned() { stack.push(top); } }
            OP_SWAP => { let len = stack.len(); if len >= 2 { stack.swap(len-1, len-2); } }
            OP_NEGATE => {
                match stack.pop() {
                    Some(StackVal::Num(v)) => stack.push(StackVal::Num(-v)),
                    _ => stack.push(StackVal::Unknown),
                }
            }
            OP_ADD | OP_SUBTRACT | OP_MULTIPLY | OP_DIVIDE => {
                let b = stack.pop(); let a = stack.pop();
                match (a, b) {
                    (Some(StackVal::Num(a)), Some(StackVal::Num(b))) => {
                        let r = match op {
                            OP_ADD => a+b, OP_SUBTRACT => a-b,
                            OP_MULTIPLY => a*b, OP_DIVIDE => a/b, _ => 0.0
                        };
                        stack.push(StackVal::Num(r));
                    }
                    _ => stack.push(StackVal::Unknown),
                }
            }
            OP_CONSTRUCTPROP => {
                read_u30_at(bytecode, &mut i);
                let argc = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
                let drain = stack.len().min(argc + 1);
                stack.drain(stack.len() - drain..);
                stack.push(StackVal::Unknown);
            }
            OP_CONSTRUCT => {
                let argc = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
                let drain = stack.len().min(argc + 1);
                stack.drain(stack.len() - drain..);
                stack.push(StackVal::Unknown);
            }
            OP_GETLOCAL0 | OP_GETLOCAL1 | OP_GETLOCAL2 | OP_GETLOCAL3 => stack.push(StackVal::Unknown),
            OP_GETLOCAL => { read_u30_at(bytecode, &mut i); stack.push(StackVal::Unknown); }
            OP_SETLOCAL0 | OP_SETLOCAL1 | OP_SETLOCAL2 | OP_SETLOCAL3 => { stack.pop(); }
            OP_SETLOCAL => { read_u30_at(bytecode, &mut i); stack.pop(); }
            OP_RETURNVALUE => { stack.pop(); }
            OP_RETURNVOID => {}
            OP_JUMP | OP_IFTRUE | OP_IFFALSE | OP_IFEQ | OP_IFNE | OP_IFLT |
            OP_IFLE | OP_IFGT | OP_IFGE | OP_IFSTRICTEQ | OP_IFSTRICTNE => {
                if i + 3 <= bytecode.len() { i += 3; }
                if op != OP_JUMP { stack.pop(); }
            }
            _ => {}
        }
        if stack.len() > 512 { stack.drain(0..256); }
    }

    result
}

/// Recursively extract hitboxes from a StackVal.
/// SSF2 attack objects have key 'attackBoxes' → object with 'attackBox', 'attackBox2', etc.
fn extract_hitboxes_from_val(val: &StackVal) -> Vec<BTreeMap<String, f64>> {
    match val {
        StackVal::Obj(obj) => {
            // Look for 'attackBoxes' key
            if let Some(boxes_val) = obj.get("attackBoxes") {
                return extract_hitboxes_from_val(boxes_val);
            }
            // Is this object itself a hitbox? (has damage/direction/power)
            let hitbox_keys = ["damage", "direction", "power", "kbConstant", "hitStun", "hitLag", "weightKB"];
            if obj.keys().any(|k| hitbox_keys.contains(&k.as_str())) {
                let mut hb = BTreeMap::new();
                for k in &hitbox_keys {
                    if let Some(StackVal::Num(v)) = obj.get(*k) {
                        hb.insert(k.to_string(), *v);
                    }
                }
                return vec![hb];
            }
            // Might be a container of hitboxes: {attackBox: {...}, attackBox2: {...}, ...}
            let mut hitboxes = Vec::new();
            for (k, v) in obj {
                if k.starts_with("attackBox") {
                    hitboxes.extend(extract_hitboxes_from_val(v));
                }
            }
            hitboxes
        }
        _ => vec![]
    }
}

fn extract_stats_from_body(bytecode: &[u8], abc: &AbcFile) -> Option<CharStats> {
    // Simulate stack; look for newobject whose keys include stat names
    let mut stack: Vec<StackVal> = Vec::new();
    let mut i = 0;

    while i < bytecode.len() {
        let op = bytecode[i];
        i += 1;
        match op {
            OP_PUSHSTRING => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    stack.push(StackVal::Str(abc.strings.get(idx as usize).cloned().unwrap_or_default()));
                }
            }
            OP_PUSHDOUBLE => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    stack.push(StackVal::Num(abc.doubles.get(idx as usize).copied().unwrap_or(0.0)));
                }
            }
            OP_PUSHBYTE => {
                if i < bytecode.len() { let v = bytecode[i] as i8 as f64; i += 1; stack.push(StackVal::Num(v)); }
            }
            OP_PUSHSHORT => {
                if let Some(v) = read_u30_at(bytecode, &mut i) { stack.push(StackVal::Num(v as i16 as f64)); }
            }
            OP_PUSHINT => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    stack.push(StackVal::Num(abc.ints.get(idx as usize).copied().unwrap_or(0) as f64));
                }
            }
            OP_PUSHUINT => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    stack.push(StackVal::Num(abc.uints.get(idx as usize).copied().unwrap_or(0) as f64));
                }
            }
            OP_PUSHTRUE  => stack.push(StackVal::Bool(())),
            OP_PUSHFALSE => stack.push(StackVal::Bool(())),
            OP_PUSHNULL | OP_PUSHNAN => stack.push(StackVal::Null),
            OP_NEWOBJECT => {
                if let Some(count) = read_u30_at(bytecode, &mut i) {
                    let count = count as usize;
                    let needed = count * 2;
                    let mut obj: BTreeMap<String, StackVal> = BTreeMap::new();
                    if stack.len() >= needed {
                        let pairs: Vec<_> = stack.drain(stack.len() - needed..).collect();
                        for chunk in pairs.chunks(2) {
                            if let StackVal::Str(k) = &chunk[0] {
                                obj.insert(k.clone(), chunk[1].clone());
                            }
                        }
                    }
                    // Check if this looks like character stats:
                    // SSF2 uses: gravity, weight1, norm_xSpeed, max_xSpeed, max_ySpeed,
                    //   fastFallSpeed, jumpSpeed, jumpSpeedMidair, accel_rate_air, decel_rate_air
                    let stat_keys = ["gravity", "weight1", "norm_xSpeed", "max_xSpeed",
                                     "fastFallSpeed", "jumpSpeed", "jumpSpeedMidair",
                                     "accel_rate_air", "decel_rate_air", "max_ySpeed",
                                     "accel_rate", "walkSpeed", "dashSpeed", "airMobility"];
                    let numeric_stats: BTreeMap<String, f64> = obj.iter()
                        .filter_map(|(k, v)| {
                            if let StackVal::Num(n) = v { Some((k.clone(), *n)) } else { None }
                        }).collect();
                    // Require at least 3 stat keys to be confident
                    let match_count = numeric_stats.keys().filter(|k| stat_keys.contains(&k.as_str())).count();
                    if match_count >= 3 {
                        return Some(CharStats { values: numeric_stats });
                    }
                    stack.push(StackVal::Obj(obj));
                }
            }
            OP_COERCE | OP_GETLEX | OP_FINDPROPSTRICT | OP_FINDPROP | OP_GETPROPERTY |
            OP_INITPROPERTY | OP_SETPROPERTY => { read_u30_at(bytecode, &mut i); }
            OP_CALLPROPERTY | OP_CALLPROPVOID | OP_CONSTRUCTPROP => {
                read_u30_at(bytecode, &mut i); read_u30_at(bytecode, &mut i);
            }
            OP_GETLOCAL | OP_SETLOCAL | OP_CONSTRUCT | OP_NEWARRAY => { read_u30_at(bytecode, &mut i); }
            OP_JUMP | OP_IFTRUE | OP_IFFALSE | OP_IFEQ | OP_IFNE | OP_IFLT |
            OP_IFLE | OP_IFGT | OP_IFGE | OP_IFSTRICTEQ | OP_IFSTRICTNE => {
                if i + 3 <= bytecode.len() { i += 3; }
            }
            _ => {}
        }
        if stack.len() > 256 { stack.drain(0..128); }
    }
    None
}

/// Extract the largest object in the body that has the most numeric key-value pairs.
/// Used as fallback for getOwnStats when the stat key heuristic doesn't match.
/// Targeted extractor: scan bytecode for SSF2 stat key-value pairs.
/// Looks for consecutive (pushstring key, push_num value) pairs where key is a known stat name.
fn extract_ssf2_stats(bytecode: &[u8], abc: &AbcFile) -> Option<CharStats> {
    const STAT_KEYS: &[&str] = &[
        "gravity", "weight1", "norm_xSpeed", "max_xSpeed", "max_ySpeed",
        "fastFallSpeed", "jumpSpeed", "jumpSpeedMidair", "shortHopSpeed",
        "accel_rate", "accel_rate_air", "decel_rate", "decel_rate_air",
        "accel_start", "accel_start_dash", "max_jump", "dodgeSpeed", "dodgeDecel",
        "roll_speed", "roll_decay", "max_projectile", "width", "height",
        "jumpStartup", "max_jumpSpeed", "groundToAirMultiplier",
    ];

    let mut values: BTreeMap<String, f64> = BTreeMap::new();
    let mut i = 0;

    while i < bytecode.len() {
        let op = bytecode[i]; i += 1;
        // Look for pushstring followed immediately by a numeric push
        if op == OP_PUSHSTRING {
            if let Some(str_idx) = read_u30_at(bytecode, &mut i) {
                let key = abc.strings.get(str_idx as usize).cloned().unwrap_or_default();
                if STAT_KEYS.contains(&key.as_str()) && i < bytecode.len() {
                    // Next op should be a numeric push
                    let next_op = bytecode[i]; i += 1;
                    let val = match next_op {
                        OP_PUSHBYTE => {
                            if i < bytecode.len() { let v = bytecode[i] as i8 as f64; i += 1; Some(v) } else { None }
                        }
                        OP_PUSHSHORT => {
                            read_u30_at(bytecode, &mut i).map(|v| v as i16 as f64)
                        }
                        OP_PUSHINT => {
                            read_u30_at(bytecode, &mut i)
                                .and_then(|idx| abc.ints.get(idx as usize).copied())
                                .map(|v| v as f64)
                        }
                        OP_PUSHUINT => {
                            read_u30_at(bytecode, &mut i)
                                .and_then(|idx| abc.uints.get(idx as usize).copied())
                                .map(|v| v as f64)
                        }
                        OP_PUSHDOUBLE => {
                            read_u30_at(bytecode, &mut i)
                                .and_then(|idx| abc.doubles.get(idx as usize).copied())
                        }
                        _ => {
                            // Back up one byte since it wasn't a numeric push
                            i -= 1;
                            None
                        }
                    };
                    if let Some(v) = val {
                        values.insert(key, v);
                    }
                }
                // Continue; don't double-consume
                continue;
            }
        }
        // Skip operand bytes for other instructions to keep position correct
        match op {
            OP_PUSHDOUBLE | OP_PUSHSTRING | OP_PUSHINT | OP_PUSHUINT |
            OP_COERCE | OP_GETLEX | OP_FINDPROPSTRICT | OP_FINDPROP |
            OP_GETPROPERTY | OP_INITPROPERTY | OP_SETPROPERTY | OP_GETLOCAL | OP_SETLOCAL => {
                read_u30_at(bytecode, &mut i);
            }
            OP_PUSHBYTE => { if i < bytecode.len() { i += 1; } }
            OP_PUSHSHORT => { read_u30_at(bytecode, &mut i); }
            OP_CALLPROPERTY | OP_CALLPROPVOID | OP_CONSTRUCTPROP => {
                read_u30_at(bytecode, &mut i); read_u30_at(bytecode, &mut i);
            }
            OP_CONSTRUCT | OP_NEWARRAY | OP_NEWOBJECT => { read_u30_at(bytecode, &mut i); }
            OP_JUMP | OP_IFTRUE | OP_IFFALSE | OP_IFEQ | OP_IFNE | OP_IFLT |
            OP_IFLE | OP_IFGT | OP_IFGE | OP_IFSTRICTEQ | OP_IFSTRICTNE => {
                if i + 3 <= bytecode.len() { i += 3; }
            }
            _ => {}
        }
    }

    if values.len() >= 3 {
        Some(CharStats { values })
    } else {
        None
    }
}

#[allow(dead_code)]
fn extract_largest_numeric_object(bytecode: &[u8], abc: &AbcFile) -> Option<CharStats> {
    let mut stack: Vec<StackVal> = Vec::new();
    let mut best: Option<BTreeMap<String, f64>> = None;
    let mut i = 0;

    while i < bytecode.len() {
        let op = bytecode[i]; i += 1;
        match op {
            OP_PUSHSTRING => { if let Some(idx) = read_u30_at(bytecode, &mut i) { stack.push(StackVal::Str(abc.strings.get(idx as usize).cloned().unwrap_or_default())); } }
            OP_PUSHDOUBLE => { if let Some(idx) = read_u30_at(bytecode, &mut i) { stack.push(StackVal::Num(abc.doubles.get(idx as usize).copied().unwrap_or(0.0))); } }
            OP_PUSHBYTE   => { if i < bytecode.len() { let v = bytecode[i] as i8 as f64; i += 1; stack.push(StackVal::Num(v)); } }
            OP_PUSHSHORT  => { if let Some(v) = read_u30_at(bytecode, &mut i) { stack.push(StackVal::Num(v as i16 as f64)); } }
            OP_PUSHINT    => { if let Some(idx) = read_u30_at(bytecode, &mut i) { stack.push(StackVal::Num(abc.ints.get(idx as usize).copied().unwrap_or(0) as f64)); } }
            OP_PUSHUINT   => { if let Some(idx) = read_u30_at(bytecode, &mut i) { stack.push(StackVal::Num(abc.uints.get(idx as usize).copied().unwrap_or(0) as f64)); } }
            OP_PUSHTRUE   => stack.push(StackVal::Bool(())),
            OP_PUSHFALSE  => stack.push(StackVal::Bool(())),
            OP_PUSHNULL | OP_PUSHNAN => stack.push(StackVal::Null),
            OP_NEWOBJECT => {
                if let Some(count) = read_u30_at(bytecode, &mut i) {
                    let count = count as usize;
                    let needed = count * 2;
                    let mut numeric: BTreeMap<String, f64> = BTreeMap::new();
                    if stack.len() >= needed {
                        let pairs: Vec<_> = stack.drain(stack.len() - needed..).collect();
                        for chunk in pairs.chunks(2) {
                            if let (StackVal::Str(k), StackVal::Num(v)) = (&chunk[0], &chunk[1]) {
                                numeric.insert(k.clone(), *v);
                            }
                        }
                    }
                    // Keep the largest purely numeric object
                    if numeric.len() >= 5 {
                        if best.as_ref().map_or(true, |b: &BTreeMap<String, f64>| numeric.len() > b.len()) {
                            best = Some(numeric.clone());
                        }
                    }
                    stack.push(StackVal::Unknown);
                }
            }
            OP_COERCE | OP_GETLEX | OP_FINDPROPSTRICT | OP_FINDPROP | OP_GETPROPERTY |
            OP_INITPROPERTY | OP_SETPROPERTY => { read_u30_at(bytecode, &mut i); }
            OP_CALLPROPERTY | OP_CALLPROPVOID | OP_CONSTRUCTPROP => {
                read_u30_at(bytecode, &mut i); read_u30_at(bytecode, &mut i);
            }
            OP_GETLOCAL | OP_SETLOCAL | OP_CONSTRUCT | OP_NEWARRAY => { read_u30_at(bytecode, &mut i); }
            OP_JUMP | OP_IFTRUE | OP_IFFALSE | OP_IFEQ | OP_IFNE | OP_IFLT |
            OP_IFLE | OP_IFGT | OP_IFGE | OP_IFSTRICTEQ | OP_IFSTRICTNE => {
                if i + 3 <= bytecode.len() { i += 3; }
            }
            _ => {}
        }
        if stack.len() > 256 { stack.drain(0..128); }
    }
    best.map(|values| CharStats { values })
}

#[allow(dead_code)]
fn extract_frame_actions(bytecode: &[u8], abc: &AbcFile) -> Vec<FrameAction> {
    let mut actions = Vec::new();
    let mut i = 0;
    let mut last_frame_num: u32 = 0;

    while i < bytecode.len() {
        let op = bytecode[i];
        i += 1;
        match op {
            OP_PUSHBYTE => {
                if i < bytecode.len() {
                    last_frame_num = bytecode[i] as u32;
                    i += 1;
                }
            }
            OP_PUSHSHORT => {
                if let Some(v) = read_u30_at(bytecode, &mut i) {
                    last_frame_num = v;
                }
            }
            OP_CALLPROPVOID | OP_CALLPROPERTY => {
                if let Some(mn_idx) = read_u30_at(bytecode, &mut i) {
                    let _arg_count = read_u30_at(bytecode, &mut i).unwrap_or(0);
                    let name = abc.multinames.get(mn_idx as usize).map(|m| m.name.clone()).unwrap_or_default();
                    if !name.is_empty() {
                        actions.push(FrameAction {
                            frame: last_frame_num,
                            action: name,
                            args: vec![],
                        });
                    }
                }
            }
            OP_COERCE | OP_GETLEX | OP_FINDPROPSTRICT | OP_FINDPROP | OP_GETPROPERTY |
            OP_INITPROPERTY | OP_SETPROPERTY => { read_u30_at(bytecode, &mut i); }
            OP_CONSTRUCTPROP => { read_u30_at(bytecode, &mut i); read_u30_at(bytecode, &mut i); }
            OP_GETLOCAL | OP_SETLOCAL | OP_NEWARRAY | OP_NEWOBJECT | OP_CONSTRUCT => {
                read_u30_at(bytecode, &mut i);
            }
            OP_JUMP | OP_IFTRUE | OP_IFFALSE | OP_IFEQ | OP_IFNE | OP_IFLT |
            OP_IFLE | OP_IFGT | OP_IFGE | OP_IFSTRICTEQ | OP_IFSTRICTNE => {
                if i + 3 <= bytecode.len() { i += 3; }
            }
            _ => {}
        }
    }
    actions
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn read_u30_at(data: &[u8], i: &mut usize) -> Option<u32> {
    let mut result = 0u32;
    let mut shift = 0;
    loop {
        if *i >= data.len() { return None; }
        let b = data[*i] as u32;
        *i += 1;
        result |= (b & 0x7F) << shift;
        shift += 7;
        if b & 0x80 == 0 || shift >= 35 { break; }
    }
    Some(result)
}

/// Is this string an SSF2 attack/move name?
fn is_attack_name(s: &str) -> bool {
    matches!(s, 
        "a" | "a_tilt" | "a_forward" | "a_forward_tilt" | "a_up_tilt" | "a_down_tilt" |
        "crouch_attack" | "a_forwardsmash" | "a_up" | "a_down" |
        "a_air" | "a_air_forward" | "a_air_backward" | "a_air_up" | "a_air_down" |
        "b" | "b_air" | "b_forward" | "b_forward_air" | "b_up" | "b_up_air" |
        "b_down" | "b_down_air" |
        "throw_up" | "throw_forward" | "throw_back" | "throw_down" |
        "ledge_attack" | "getup_attack" | "special" |
        "jab" | "jab1" | "jab2" | "jab3" | "dash" |
        "ftilt" | "utilt" | "dtilt" | "fsmash" | "usmash" | "dsmash" |
        "nair" | "fair" | "bair" | "uair" | "dair" |
        "nspecial" | "sspecial" | "uspecial" | "dspecial"
    )
}

fn normalize_attack_name(s: &str) -> String {
    let map: &[(&str, &str)] = &[
        ("a",              "jab1"),
        ("a_tilt",         "jab1"),
        ("a_forward",      "dash_attack"),
        ("a_forward_tilt", "tilt_forward"),
        ("a_up_tilt",      "tilt_up"),
        ("a_down_tilt",    "tilt_down"),
        ("crouch_attack",  "tilt_down"),
        ("a_forwardsmash", "strong_forward_attack"),
        ("a_up",           "strong_up_attack"),
        ("a_down",         "strong_down_attack"),
        ("a_air",          "aerial_neutral"),
        ("a_air_forward",  "aerial_forward"),
        ("a_air_backward", "aerial_back"),
        ("a_air_up",       "aerial_up"),
        ("a_air_down",     "aerial_down"),
        ("b",              "special_neutral"),
        ("b_air",          "special_neutral_air"),
        ("b_forward",      "special_side"),
        ("b_forward_air",  "special_side_air"),
        ("b_up",           "special_up"),
        ("b_up_air",       "special_up_air"),
        ("b_down",         "special_down"),
        ("b_down_air",     "special_down_air"),
        ("throw_up",       "throw_up"),
        ("throw_forward",  "throw_forward"),
        ("throw_back",     "throw_back"),
        ("throw_down",     "throw_down"),
        ("ledge_attack",   "ledge_attack"),
        ("getup_attack",   "crash_attack"),
    ];
    for (from, to) in map {
        if s == *from { return to.to_string(); }
    }
    s.to_string()
}

/// Does this object look like SSF2 attack hitbox data?
#[allow(dead_code)]
fn is_attack_object(obj: &BTreeMap<String, f64>) -> bool {
    if obj.is_empty() { return false; }
    let attack_keys = ["damage", "direction", "power", "kbConstant", "weightKB",
                       "hitStun", "selfHitStun", "hitLag", "angle"];
    obj.keys().any(|k| attack_keys.contains(&k.as_str()))
}

/// Does this object look like character physics stats?
#[allow(dead_code)]
fn is_stats_object(obj: &BTreeMap<String, f64>) -> bool {
    if obj.is_empty() { return false; }
    let stat_keys = ["weight", "gravity", "fallSpeed", "fastFallSpeed",
                     "walkSpeed", "dashSpeed", "airMobility", "maxJumps",
                     "jumpHeight", "doubleJumpHeight", "airFriction"];
    obj.keys().any(|k| stat_keys.contains(&k.as_str()))
}


/// Extract costume data from SSF2API::getCostumeData (static method).
///
/// getCostumeData returns an Array of objects: [{name:"Default", colors:[0xFFRRGGBB,...]}, ...]
/// We simulate the AVM2 stack to reconstruct these objects.
pub fn extract_costume_data(abc: &AbcFile) -> Vec<CostumeData> {
    let api_class = abc.classes.iter().find(|c| c.name == "SSF2API");
    let Some(api) = api_class else {
        log::debug!("extract_costume_data: no SSF2API class");
        return vec![];
    };
    let mi = match api.class_methods.iter().find(|t| t.name == "getCostumeData") {
        Some(t) => t.method_idx,
        None => { log::debug!("extract_costume_data: no getCostumeData trait"); return vec![]; }
    };
    let body = match abc.method_bodies.iter().find(|b| b.method_idx == mi) {
        Some(b) => b,
        None => { log::debug!("extract_costume_data: no body for mi={}", mi); return vec![]; }
    };
    log::info!("extract_costume_data: decoding getCostumeData ({} bytes)", body.bytecode.len());
    // Decode the 13-byte wrapper: getlex CLASS → callproperty METHOD → returnvalue
    // The real costume data is in CLASS::METHOD. Decode the wrapper to find which method.
    {
        let code = &body.bytecode;
        let mut pos = 0usize;
        // skip getlocal_0 + pushscope
        while pos < code.len() && code[pos] != 0x60 { pos += 1; }
        if code[pos] == 0x60 {
            pos += 1;
            let class_mn = read_u30_at(code, &mut pos).unwrap_or(0) as usize;
            log::info!("getCostumeData wrapper: getlex multiname[{}] = {:?}",
                class_mn, abc.multinames.get(class_mn));
            // find callproperty
            while pos < code.len() && code[pos] != 0x46 { pos += 1; }
            if code[pos] == 0x46 {
                pos += 1;
                let method_mn = read_u30_at(code, &mut pos).unwrap_or(0) as usize;
                let argc = read_u30_at(code, &mut pos).unwrap_or(0);
                log::info!("getCostumeData wrapper: callproperty multiname[{}] = {:?} argc={}",
                    method_mn, abc.multinames.get(method_mn), argc);

                // Now find the class with that name and the method with that name
                let class_name = abc.multinames.get(class_mn).map(|m| m.name.as_str()).unwrap_or("");
                let method_name = abc.multinames.get(method_mn).map(|m| m.name.as_str()).unwrap_or("");
                log::info!("getCostumeData delegates to {}::{}", class_name, method_name);

                // Scan ALL method bodies with many pushuint (0x2E) calls — color palette data
                let mut best: Vec<(u32, usize, u32)> = vec![]; // (method_idx, bytes, pushuint_count)
                for body2 in &abc.method_bodies {
                    let pu_count = body2.bytecode.iter().filter(|&&b| b == 0x2E).count() as u32;
                    if pu_count >= 8 {
                        best.push((body2.method_idx, body2.bytecode.len(), pu_count));
                    }
                }
                best.sort_by_key(|&(_, _, pu)| std::cmp::Reverse(pu));
                let trait_for: BTreeMap<u32, (&str, &str)> = abc.classes.iter()
                    .flat_map(|cls| cls.instance_methods.iter().chain(cls.class_methods.iter())
                        .map(move |t| (t.method_idx, (cls.name.as_str(), t.name.as_str()))))
                    .collect();
                for (mi2, bytes, pu) in best.iter().take(10) {
                    let (cn, mn) = trait_for.get(mi2).copied().unwrap_or(("?", "?"));
                    log::info!("  pushuint-heavy: mi={} {}::{} bytes={} pushuints={}", mi2, cn, mn, bytes, pu);
                }
            }
        }
    }

    let code = &body.bytecode;
    let mut pos = 0usize;
    let mut stack: Vec<StackVal> = Vec::new();
    let mut costumes: Vec<CostumeData> = Vec::new();

    macro_rules! ru30 {
        () => { read_u30_at(code, &mut pos).unwrap_or(0) }
    }

    while pos < code.len() {
        let op = code[pos]; pos += 1;
        match op {
            0x24 => { // pushbyte
                let v = if pos < code.len() { let b = code[pos] as i8; pos += 1; b as f64 } else { 0.0 };
                stack.push(StackVal::Num(v));
            }
            0x25 => { let v = ru30!() as i16; stack.push(StackVal::Num(v as f64)); }
            0x2C => { // pushstring
                let i = ru30!() as usize;
                stack.push(StackVal::Str(abc.strings.get(i).cloned().unwrap_or_default()));
            }
            0x2D => { // pushint
                let i = ru30!() as usize;
                stack.push(StackVal::Num(abc.ints.get(i).copied().unwrap_or(0) as f64));
            }
            0x2E => { // pushuint
                let i = ru30!() as usize;
                stack.push(StackVal::Num(abc.uints.get(i).copied().unwrap_or(0) as f64));
            }
            0x2F => { // pushdouble
                let i = ru30!() as usize;
                stack.push(StackVal::Num(abc.doubles.get(i).copied().unwrap_or(0.0)));
            }
            0x26 => stack.push(StackVal::Bool(())),
            0x27 => stack.push(StackVal::Bool(())),
            0x20 | 0x28 => stack.push(StackVal::Null),
            0x56 => { // newarray(n)
                let n = ru30!() as usize;
                let start = stack.len().saturating_sub(n);
                let items = stack.drain(start..).collect();
                stack.push(StackVal::Arr(items));
            }
            0x55 => { // newobject(n) — 2n items on stack: key0,val0,key1,val1,...
                let n = ru30!() as usize;
                let start = stack.len().saturating_sub(n * 2);
                let items: Vec<StackVal> = stack.drain(start..).collect();
                let mut obj: BTreeMap<String, StackVal> = BTreeMap::new();
                let mut i = 0;
                while i + 1 < items.len() {
                    if let StackVal::Str(k) = &items[i] {
                        obj.insert(k.clone(), items[i+1].clone());
                    }
                    i += 2;
                }
                // Is this a costume entry? { name: "...", colors: [...] }
                if let (Some(StackVal::Str(name)), Some(StackVal::Arr(colors))) =
                    (obj.get("name"), obj.get("colors"))
                {
                    let color_vals: Vec<u32> = colors.iter().filter_map(|c| {
                        if let StackVal::Num(v) = c { Some(*v as u32) } else { None }
                    }).collect();
                    if !color_vals.is_empty() {
                        log::debug!("  costume {:?}: {} colors", name, color_vals.len());
                        costumes.push(CostumeData { name: name.clone(), colors: color_vals, replacements: vec![] });
                    }
                }
                stack.push(StackVal::Obj(obj));
            }
            // ops with 2 u30 args
            0x46 | 0x4F | 0x6E | 0x4B | 0x45 | 0x4A => { ru30!(); ru30!(); stack.push(StackVal::Null); }
            // ops with 1 u30 arg (read-side)
            0x60 | 0x5C | 0x5D | 0x80 | 0x65 => { ru30!(); stack.push(StackVal::Null); }
            // ops with 1 u30 arg (write-side — no push)
            0x61 | 0x66 | 0x68 | 0x62 | 0x63 | 0x08 => { ru30!(); }
            // branches (s24)
            0x10 | 0x0C | 0x0D | 0x0E | 0x0F |
            0x13 | 0x14 | 0x15 | 0x16 | 0x17 | 0x18 | 0x19 | 0x1A => { pos += 3; }
            // locals → push placeholder
            0xD0 | 0xD1 | 0xD2 | 0xD3 => stack.push(StackVal::Null),
            // stack ops
            0x29 => { stack.pop(); }
            0x2A => { if let Some(t) = stack.last().cloned() { stack.push(t); } }
            0x2B => { let n = stack.len(); if n >= 2 { stack.swap(n-1, n-2); } }
            // scope
            0x30 | 0x1D => {}
            // return
            0x47 | 0x48 => break,
            // nop / coerce_a / convert_*
            0x02 | 0x82 | 0x73 | 0x74 | 0x75 | 0x76 | 0x70 => {}
            // arithmetic (binary) — pop 2, push 1
            0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA8 | 0xA9 | 0xAA | 0xA5 | 0xA6 | 0xA7 => {
                stack.pop(); if stack.is_empty() { stack.push(StackVal::Null); }
            }
            // arithmetic (unary) — pop 1, push 1
            0x90 | 0x96 | 0xAB | 0xB1 => {
                if !stack.is_empty() { *stack.last_mut().unwrap() = StackVal::Null; }
            }
            _ => {}
        }
    }

    log::info!("extract_costume_data: {} costumes found", costumes.len());
    costumes
}

/// Extract costume palette data by decoding the applyPalette method.
///
/// SSF2 applies costumes via Flash ColorTransform: each costume index maps to
/// (redMultiplier, greenMultiplier, blueMultiplier, redOffset, greenOffset, blueOffset).
/// The applyPalette method body contains a switch-like structure that pushes these
/// values and calls setTransform on each sprite's ColorTransform.
///
/// We scan for ColorTransform constructor calls with numeric args to extract costume data.
pub fn extract_costume_data_from_apply_palette(abc: &AbcFile) -> Option<Vec<CostumeData>> {
    // Find the character class (e.g. "mario") — applyPalette is an instance method
    let apply_palette_body = abc.classes.iter()
        .flat_map(|cls| cls.instance_methods.iter())
        .find(|t| t.name == "applyPalette")
        .and_then(|t| abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx));

    if apply_palette_body.is_none() {
        // Try class methods too
        log::debug!("extract_apply_palette: applyPalette not found as instance method, trying class methods");
    }

    let body = apply_palette_body?;
    log::info!("extract_apply_palette: found applyPalette ({} bytes)", body.bytecode.len());

    // Simulate the stack looking for sequences of 8 numeric pushes followed by
    // constructprop ColorTransform or callproperty setTransform
    let code = &body.bytecode;
    let mut pos = 0usize;
    let mut stack: Vec<StackVal> = Vec::new();
    let mut costumes: Vec<CostumeData> = Vec::new();
    let mut current_nums: Vec<f64> = Vec::new();

    macro_rules! ru30 { () => { read_u30_at(code, &mut pos).unwrap_or(0) } }

    while pos < code.len() {
        let op = code[pos]; pos += 1;
        match op {
            0x24 => {
                let v = if pos < code.len() { let b = code[pos] as i8; pos += 1; b as f64 } else { 0.0 };
                stack.push(StackVal::Num(v));
                current_nums.push(v);
            }
            0x25 => { let v = ru30!() as i16 as f64; stack.push(StackVal::Num(v)); current_nums.push(v); }
            0x2D => { let i = ru30!() as usize; let v = abc.ints.get(i).copied().unwrap_or(0) as f64; stack.push(StackVal::Num(v)); current_nums.push(v); }
            0x2E => { let i = ru30!() as usize; let v = abc.uints.get(i).copied().unwrap_or(0) as f64; stack.push(StackVal::Num(v)); current_nums.push(v); }
            0x2F => { let i = ru30!() as usize; let v = abc.doubles.get(i).copied().unwrap_or(0.0); stack.push(StackVal::Num(v)); current_nums.push(v); }
            0x2C => { let i = ru30!() as usize; let s = abc.strings.get(i).cloned().unwrap_or_default(); stack.push(StackVal::Str(s)); current_nums.clear(); }
            0x4A => { // constructprop
                let mn = ru30!() as usize; let argc = ru30!();
                let name = abc.multinames.get(mn).map(|m| m.name.as_str()).unwrap_or("");
                if name == "ColorTransform" && argc >= 6 {
                    // last 6 nums on stack: rMult, gMult, bMult, aMult, rOff, gOff, bOff, aOff
                    // or just rMult, gMult, bMult, aMult, rOff, gOff, bOff
                    let nums: Vec<f64> = stack.iter().rev().take(argc as usize)
                        .filter_map(|v| if let StackVal::Num(n) = v { Some(*n) } else { None })
                        .collect::<Vec<_>>().into_iter().rev().collect();
                    if nums.len() >= 3 {
                        // Convert multipliers to 0-255 RGB
                        // Flash ColorTransform: 1.0 = no change, values 0-1 for multiply
                        let r = ((nums[0].abs()) * 255.0).min(255.0) as u8;
                        let g = ((nums[1].abs()) * 255.0).min(255.0) as u8;
                        let b = ((nums[2].abs()) * 255.0).min(255.0) as u8;
                        log::debug!("  ColorTransform({:?}) → rgb=({},{},{})", nums, r, g, b);
                        let idx = costumes.len();
                        costumes.push(CostumeData {
                            name: format!("Alt {}", idx + 1),
                            colors: vec![0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)],
                            replacements: vec![],
                        });
                    }
                    current_nums.clear();
                }
                for _ in 0..argc { stack.pop(); }
                stack.push(StackVal::Null);
            }
            // standard stack ops
            0x46 | 0x4F | 0x6E | 0x4B | 0x45 => { ru30!(); ru30!(); stack.push(StackVal::Null); current_nums.clear(); }
            0x60 | 0x5C | 0x5D | 0x80 | 0x65 => { ru30!(); stack.push(StackVal::Null); }
            0x61 | 0x66 | 0x68 | 0x62 | 0x63 | 0x08 => { ru30!(); }
            0x10 | 0x0C | 0x0D | 0x0E | 0x0F | 0x13 | 0x14 | 0x15 | 0x16 | 0x17 | 0x18 | 0x19 | 0x1A => { pos += 3; }
            0xD0 | 0xD1 | 0xD2 | 0xD3 => stack.push(StackVal::Null),
            0x56 => { let n = ru30!() as usize; let start = stack.len().saturating_sub(n); let items = stack.drain(start..).collect(); stack.push(StackVal::Arr(items)); }
            0x55 => { let n = ru30!() as usize; let start = stack.len().saturating_sub(n*2); stack.drain(start..); stack.push(StackVal::Obj(BTreeMap::new())); }
            0x29 => { stack.pop(); }
            0x2A => { if let Some(t) = stack.last().cloned() { stack.push(t); } }
            0x2B => { let n = stack.len(); if n >= 2 { stack.swap(n-1, n-2); } }
            0x30 | 0x1D => {}
            0x47 | 0x48 => break,
            0x02 | 0x82 | 0x73 | 0x74 | 0x75 | 0x76 | 0x70 => {}
            0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA8 | 0xA9 | 0xAA | 0xA5 | 0xA6 | 0xA7 => { stack.pop(); if stack.is_empty() { stack.push(StackVal::Null); } }
            0x90 | 0x96 | 0xAB | 0xB1 => { if !stack.is_empty() { *stack.last_mut().unwrap() = StackVal::Null; } }
            0x26 => stack.push(StackVal::Bool(())),
            0x27 => stack.push(StackVal::Bool(())),
            0x20 | 0x28 => stack.push(StackVal::Null),
            _ => {}
        }
    }

    if costumes.is_empty() {
        log::info!("extract_apply_palette: no ColorTransform constructors found in applyPalette");
        return None;
    }

    // Insert "Default" costume at index 0
    costumes.insert(0, CostumeData { name: "Default".to_string(), colors: vec![], replacements: vec![] });
    log::info!("extract_apply_palette: found {} costumes", costumes.len());
    Some(costumes)
}

/// Scan ALL method bodies for SSF2 costume data patterns.
///
/// SSF2 misc.ssf stores costume data per-character as methods that return arrays of
/// {name: String, colors: [uint, uint, ...]} objects. This function finds them all.
///
/// Returns a map of class_name → Vec<CostumeData>.
/// Scan all method bodies for costume data. Returns per-character map.
/// Handles two layouts:
///   A) misc.ssf getCostumeData: one big method, character key from getproperty on a string constant
///   B) character-file layout: small per-costume method, char name inferred from class name
pub fn scan_all_costume_methods(abc: &AbcFile) -> BTreeMap<String, Vec<CostumeData>> {
    let mut results: BTreeMap<String, Vec<CostumeData>> = BTreeMap::new();
    for body in &abc.method_bodies {
        let per_char = decode_costume_objects(&body.bytecode, abc);
        for (char_name, costumes) in per_char {
            if !costumes.is_empty() {
                results.entry(char_name).or_default().extend(costumes);
            }
        }
    }
    results
}

/// Simulate AVM2 stack execution to extract costume palette data.
/// Returns per-character costume lists, keyed by character name string.
/// Handles both misc.ssf (single method, array-of-char-keyed objects)
/// and character-file (small per-costume method) layouts.
fn decode_costume_objects(code: &[u8], abc: &AbcFile) -> BTreeMap<String, Vec<CostumeData>> {
    #[derive(Clone, Debug)]
    enum V {
        Null,
        Num(f64),
        Str(String),
        Arr(Vec<V>),
        Obj(BTreeMap<String, V>),
    }

    fn arr_to_u32(v: Option<&V>) -> Vec<u32> {
        match v {
            Some(V::Arr(arr)) => arr.iter().filter_map(|x| {
                if let V::Num(n) = x { Some(*n as u32) } else { None }
            }).collect(),
            _ => vec![],
        }
    }

    let mut pos = 0usize;
    let mut stack: Vec<V> = Vec::new();
    // per-character costume accumulator
    let mut per_char: BTreeMap<String, Vec<CostumeData>> = BTreeMap::new();
    // tracks which character name was last used as an array key
    // (set when we see getproperty/setproperty with a plain string constant)
    let mut current_char: Option<String> = None;
    // per-char alt counter
    let mut alt_counters: BTreeMap<String, usize> = BTreeMap::new();

    macro_rules! r30 { () => { read_u30_at(code, &mut pos).unwrap_or(0) } }
    macro_rules! pop  { () => { stack.pop().unwrap_or(V::Null) } }

    // Helper: is a string a known SSF2 character name?
    // We accept any lowercase alphabetic string that looks like a char id.
    fn looks_like_char_name(s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())
            && s.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false)
            && s.len() >= 3 && s.len() <= 24
    }

    while pos < code.len() {
        let op = code[pos]; pos += 1;
        match op {
            // Push literals
            0x24 => { let v = code.get(pos).copied().unwrap_or(0) as i8; pos += 1; stack.push(V::Num(v as f64)); }
            0x25 => { let v = r30!() as i16; stack.push(V::Num(v as f64)); }
            0x2C => { let i = r30!() as usize; stack.push(V::Str(abc.strings.get(i).cloned().unwrap_or_default())); }
            0x2D => { let i = r30!() as usize; stack.push(V::Num(*abc.ints.get(i).unwrap_or(&0) as f64)); }
            0x2E => { let i = r30!() as usize; stack.push(V::Num(*abc.uints.get(i).unwrap_or(&0) as f64)); }
            0x2F => { let i = r30!() as usize; stack.push(V::Num(*abc.doubles.get(i).unwrap_or(&0.0))); }
            0x26 | 0x27 | 0x20 | 0x28 => stack.push(V::Null),

            // getproperty / setproperty / initproperty — track char name from runtime key
            // Opcode 0x66 = getproperty, 0x61 = setproperty, 0x68 = initproperty
            // The multiname index tells us if it's a static or runtime (MultinameL) name.
            0x66 => {
                let mn_idx = r30!() as usize;
                let mn = abc.multinames.get(mn_idx);
                let is_runtime = mn.map(|m| m.kind == 0x1B || m.kind == 0x1C).unwrap_or(false);
                let static_name = mn.and_then(|m| if m.name.is_empty() { None } else { Some(m.name.clone()) });
                if is_runtime {
                    // Runtime key was top of stack before the receiver
                    // stack is: [..., receiver, key] — but getproperty pops receiver, uses name from stack
                    // Actually for MultinameL: pops the name then the object, pushes result
                    let key = pop!();
                    pop!(); // receiver / object
                    if let V::Str(s) = &key {
                        if looks_like_char_name(s) {
                            current_char = Some(s.clone());
                        }
                    }
                    stack.push(V::Null); // result of getproperty (the array slot)
                } else {
                    // Static name property access
                    let top = pop!();
                    if let Some(name) = static_name {
                        if looks_like_char_name(&name) {
                            current_char = Some(name);
                        }
                        stack.push(top); // preserve for chained calls
                    } else {
                        stack.push(V::Null);
                    }
                }
            }
            0x61 | 0x68 => {
                let mn_idx = r30!() as usize;
                let mn = abc.multinames.get(mn_idx);
                let is_runtime = mn.map(|m| m.kind == 0x1B || m.kind == 0x1C).unwrap_or(false);
                let static_name = mn.and_then(|m| if m.name.is_empty() { None } else { Some(m.name.clone()) });
                if is_runtime {
                    let _val = pop!();
                    let key = pop!();
                    pop!(); // object
                    if let V::Str(s) = &key {
                        if looks_like_char_name(s) { current_char = Some(s.clone()); }
                    }
                } else {
                    pop!(); pop!();
                    if let Some(name) = static_name {
                        if looks_like_char_name(&name) { current_char = Some(name); }
                    }
                }
            }

            // newarray
            0x56 => {
                let n = r30!() as usize;
                let start = stack.len().saturating_sub(n);
                let items: Vec<V> = stack.drain(start..).collect();
                stack.push(V::Arr(items));
            }

            // newobject — the key opcode; build a costume if it matches the SSF2 palette format
            0x55 => {
                let n = r30!() as usize;
                let start = stack.len().saturating_sub(n * 2);
                let pairs: Vec<V> = stack.drain(start..).collect();
                let mut obj: BTreeMap<String, V> = BTreeMap::new();
                let mut i = 0;
                while i + 1 < pairs.len() {
                    if let V::Str(k) = &pairs[i] { obj.insert(k.clone(), pairs[i+1].clone()); }
                    i += 2;
                }

                // Pattern A: misc.ssf — {team|base, paletteSwap:{colors,replacements}}
                if let Some(V::Obj(ps)) = obj.get("paletteSwap") {
                    let colors = arr_to_u32(ps.get("colors"));
                    let replacements = arr_to_u32(ps.get("replacements"));
                    if colors.len() >= 4 && colors.len() == replacements.len() {
                        let char_key = current_char.clone().unwrap_or_else(|| "unknown".to_string());
                        let alt_n = alt_counters.entry(char_key.clone()).or_insert(0);
                        let costume_name = if obj.contains_key("base") {
                            "Default".to_string()
                        } else if let Some(V::Str(team)) = obj.get("team") {
                            let mut c = team.chars();
                            match c.next() {
                                None    => team.clone(),
                                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                            }
                        } else {
                            *alt_n += 1;
                            format!("Alt {}", alt_n)
                        };
                        per_char.entry(char_key).or_default()
                            .push(CostumeData { name: costume_name, colors, replacements });
                    }
                }
                // Pattern B: character-file — {name:String, colors:[uint...]}
                else if let (Some(V::Str(name)), Some(V::Arr(color_arr))) = (obj.get("name"), obj.get("colors")) {
                    let colors: Vec<u32> = color_arr.iter().filter_map(|v| {
                        if let V::Num(n) = v { Some(*n as u32) } else { None }
                    }).collect();
                    if colors.len() >= 4 {
                        let char_key = current_char.clone().unwrap_or_else(|| "unknown".to_string());
                        per_char.entry(char_key).or_default()
                            .push(CostumeData { name: name.clone(), colors, replacements: vec![] });
                    }
                }
                stack.push(V::Obj(obj));
            }

            // callproperty / callpropvoid — track "push" calls so we know the target char
            // For callpropvoid "push", the receiver is _loc1_["mario"] already resolved
            0x46 | 0x4F => {
                let mn_idx = r30!() as usize;
                let argc = r30!() as usize;
                let mn_name = abc.multinames.get(mn_idx)
                    .map(|m| m.name.as_str()).unwrap_or("");
                let drain = (argc + 1).min(stack.len());
                if mn_name == "push" {
                    // stack top = arg (the costume object), then receiver = char array
                    // current_char is already set from the preceding getproperty
                    stack.drain(stack.len()-drain..);
                } else {
                    stack.drain(stack.len()-drain..);
                }
                if op == 0x46 { stack.push(V::Null); }
            }
            // constructprop / construct / callsuper etc.
            0x4A | 0x6E | 0x4B | 0x45 => {
                r30!(); let argc = r30!() as usize;
                let drain = (argc + 1).min(stack.len());
                stack.drain(stack.len()-drain..);
                stack.push(V::Null);
            }

            0x60 | 0x5C | 0x5D | 0x65 | 0x80 => { r30!(); stack.push(V::Null); }
            0x62 | 0x63 | 0x08 => { r30!(); }
            0x29 => { pop!(); }
            0x2A => { if let Some(v) = stack.last().cloned() { stack.push(v); } }
            0x2B => { let n = stack.len(); if n >= 2 { stack.swap(n-1, n-2); } }
            0xD0 | 0xD1 | 0xD2 | 0xD3 => stack.push(V::Null),
            // Branch opcodes (3-byte offsets)
            0x10 | 0x0C | 0x0D | 0x0E | 0x0F | 0x13 | 0x14 | 0x15 | 0x16 | 0x17 | 0x18 | 0x19 | 0x1A => {
                if pos + 3 <= code.len() { pos += 3; }
            }
            0x47 | 0x48 => break,
            // Arithmetic — consume 1 or 2, push result
            0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA8 | 0xA9 | 0xAA | 0xA5 | 0xA6 | 0xA7 => {
                if !stack.is_empty() { stack.pop(); }
                if stack.is_empty() { stack.push(V::Null); } else { *stack.last_mut().unwrap() = V::Null; }
            }
            0x90 | 0x96 | 0xAB | 0xB1 => { if !stack.is_empty() { *stack.last_mut().unwrap() = V::Null; } }
            0x30 | 0x1D | 0x02 | 0x82 | 0x73 | 0x74 | 0x75 | 0x76 | 0x70 => {}
            _ => {}
        }
    }

    per_char
}


// DEBUG REMOVED
