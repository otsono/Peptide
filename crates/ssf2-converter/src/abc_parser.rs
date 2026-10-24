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
use std::sync::Arc;
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
const OP_PUSHUNDEFINED:  u8 = 0x21;
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
// negated comparison branches (see decompiler.rs) — same 3-byte offset as their
// positive twins, so the scan desyncs on any method that uses one.
const OP_IFNLT:         u8 = 0x0C;
const OP_IFNLE:         u8 = 0x0D;
const OP_IFNGT:         u8 = 0x0E;
const OP_IFNGE:         u8 = 0x0F;

// ─── Parsed ABC structures ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbcFile {
    /// ABC string constant pool. Interned as `Arc<str>` so resolving a name
    /// onto a `Multiname` / `Method` is a refcount bump, not a fresh allocation.
    pub strings: Vec<Arc<str>>,
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
    pub name: Arc<str>, // resolved from string pool (shared, not re-allocated)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Method {
    pub name_idx: u32,
    pub name: Arc<str>,
    pub param_count: u32,
    pub return_type_idx: u32,
    /// Declared defaults for the OPTIONAL trailing parameters, resolved against the numeric pools
    /// (`None` for a non-numeric default). A call site is free to pass fewer arguments than the
    /// signature takes, and what the object is actually built with then lives here rather than at
    /// the call: bowserscastle constructs its weather with one argument and takes the count and
    /// area from these.
    #[serde(default)]
    pub optionals: Vec<Option<f64>>,
    /// The same defaults rendered as literals, every kind the source can declare: a number, a
    /// bool, a string, null. game and watch's `startTimer(start = true)` is called bare and the
    /// source starts the timer; read without its `true` the port stopped it.
    #[serde(default)]
    pub optional_defaults: Vec<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Class {
    pub name: String,
    pub super_name: String,
    pub instance_methods: Vec<Trait>,
    pub class_methods: Vec<Trait>,
    pub constructor_idx: u32,
    /// The class's static initializer: where a `static const` with a computed value (a list of
    /// points) is filled in. 0 when the file carries none.
    pub class_init_idx: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trait {
    pub name: String,
    pub kind: u8,
    pub method_idx: u32,
    pub slot_idx: u32,
    /// Slot/Const default value, when numeric (the ABC trait's vindex/vkind resolved against the
    /// int/uint/double pools) — e.g. a class constant like `SINK_SPEED = 30`.
    #[serde(default)]
    pub default: Option<f64>,
    /// Slot/Const default value when it is a STRING -- an animation or state name the
    /// class states once and its methods read by name.
    #[serde(default)]
    pub default_str: Option<String>,
    /// Slot/Const default when it is `true`, `false` or `null` (or `undefined`, which reads as
    /// null here), as that literal. These are not numbers: a field declared `= null` and read as
    /// 0 fails every `== null` test the class makes of it.
    #[serde(default)]
    pub default_lit: Option<String>,
    /// The slot's declared type, when it names one (`int`, `Boolean`, ...): what a slot with no
    /// stated value starts as follows from it.
    #[serde(default)]
    pub type_name: Option<String>,
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
    /// The names the source declares as GETTERS. Reading one is calling it, which is the whole
    /// point of a getter, and a port that leaves the read bare hands the function itself to
    /// whatever wanted the number.
    pub getters: Vec<String>,
    /// Names two classes in this package both declared, where the second had to keep its own
    /// under `<name>__own`. Whoever registers one of these by name means its own, and the
    /// package is split across blocks, so the rewiring finishes where they are all together.
    pub renamed_own: Vec<String>,
    /// Names of instance Slot/Const traits on the Ext class (the SSF2
    /// `public var foo:T;` declarations).
    pub ext_vars: Vec<String>,
    /// Initial values for the ext_vars, pulled out of the Ext class
    /// constructor (iinit). `(name, rhs_expression)`; one per var that the
    /// constructor actually assigns.
    pub ext_var_inits: Vec<(String, String)>,
    /// The type each field is declared with (`int`, `Number`, ...), `*` when two declarations
    /// disagree. A write to an `int` field keeps only the whole part in the source.
    pub ext_var_types: Vec<(String, String)>,
    /// frame method name → SSF2 animation name (from self.xframe = "...")
    pub xframe_map: XframeMap,
    /// SSF2 `attackSound{N}_id` values (1-based, contiguous) — the per-attack
    /// SFX table indexed by `playAttackSound(N)`. Entries are SSF2 sound ids
    /// (global ids like `brawl_swing_l` or per-character bank ids).
    pub attack_sounds: Vec<String>,
    /// SSF2 `attackVoice{N}_id` values (1-based, contiguous) — the voice/grunt
    /// table indexed by `playVoiceSound(N)`. Voice playback does not overlap.
    pub voice_sounds: Vec<String>,
    /// The clip this character's stats name as its `thumbnail` -- the picture SSF2's damage
    /// counter shows for it. The source stating which of its clips is the portrait, so the
    /// converter never has to guess from a name.
    pub thumbnail: Option<String>,
    /// The id the character's stats declare for itself (`linkage_id`): what the source's own
    /// code compares against when it asks an object who owns it.
    pub linkage_id: Option<String>,
    /// Populated when this character's bundle method's `cData.normalStats_id`
    /// disagrees with `char_name` — i.e. this is a transformation /
    /// alternate form of a parent character (Giga Bowser, Wario Man).
    /// Fraymakers has no native transformation API; the converter emits
    /// the alternate form as a standalone character package and surfaces
    /// this metadata in `CharacterStats.hx` (TODO banner) and
    /// `conversion_stats.json` (`ssf2_source`).
    pub derived_from: Option<DerivedFrom>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedFrom {
    /// `cData.normalStats_id` — the parent character's id.
    pub parent_normal_stats_id: String,
    /// The Main method that produced this bundle (e.g. `getGigaBowser`).
    pub source_method: String,
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
    /// the attack's own numbers besides its boxes (`chargetime_max`, …): what the source reads
    /// back through `getAttackStat`
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stats: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharStats {
    pub values: BTreeMap<String, f64>,
    /// the stats it states as text (the id it is known by among its stats)
    #[serde(default)]
    pub names: BTreeMap<String, String>,
}

/// One projectile-stat entry extracted from SSF2's `getProjectileStats()`.
/// The Ext class returns a top-level object keyed by projectile name
/// (matching the projectile sprite's class name, e.g. `dee_nspec`).
/// Each value combines:
///   - flat-scalar physics fields (gravity, xSpeed, ySpeed, friction, …)
///   - a nested `attackBoxes` object with the projectile's hitboxes
/// Hitbox shape mirrors `AttackData.hitboxes` so it can share the
/// `hitbox_stats.jsonc` SSF2→FM field-name canon.
/// The key a declared box carries its index under (`attackBox` 0, `attackBox2` 1, …), so its stats
/// land on the box the entity draws under that name.
pub const BOX_INDEX_KEY: &str = "__box";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectileData {
    pub stats:    BTreeMap<String, f64>,
    /// The `true`/`false` fields the declaration carries (`ghost`, `canBePocketed`, …). Kept
    /// because they are behaviour: dropping them silently is how a projectile ends up ignoring
    /// half of what its own package says about it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub flags:    BTreeMap<String, bool>,
    /// The named fields (`linkage_id`, `effect_id`, …).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names:    BTreeMap<String, String>,
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

/// Read an ABC file.
///
/// The reading is Ruffle's (`crate::abc_ruffle`); this crate keeps the shape and everything built
/// on it. The hand-rolled walk that used to be here is [`parse_by_hand`], kept as the thing the
/// parity test checks the mapping against -- a second opinion is worth having about a format where
/// an off-by-one renames every symbol without failing anything.
pub fn parse(data: &[u8]) -> Result<AbcFile> {
    crate::abc_ruffle::parse(data)
}

/// The hand-rolled ABC walk. Superseded by [`parse`]; see `tests/abc_reader_parity.rs`.
pub fn parse_by_hand(data: &[u8]) -> Result<AbcFile> {
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
    let mut strings: Vec<Arc<str>> = vec![Arc::from("")];
    for _ in 1..string_count {
        strings.push(Arc::from(r.read_string()?));
    }

    log::debug!("ABC constants: {} ints, {} uints, {} doubles, {} strings",
        ints.len(), uints.len(), doubles.len(), strings.len());

    // Namespaces
    let ns_count = r.read_u30()? as usize;
    let mut namespaces: Vec<String> = vec![String::new()];
    for _ in 1..ns_count {
        let _kind = r.read_u8()?;
        let name_idx = r.read_u30()?;
        namespaces.push(strings.get(name_idx as usize).map(|s| s.to_string()).unwrap_or_default());
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
    let mut multinames = vec![Multiname { kind: 0, name_idx: 0, ns_idx: 0, name: Arc::from("") }];
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
                Multiname { kind, name_idx: 0, ns_idx: 0, name: Arc::from("") }
            }
            0x09 | 0x0E => { // Multiname, MultinameA
                let name_idx = r.read_u30()?;
                let _ns_set = r.read_u30()?;
                let name = strings.get(name_idx as usize).cloned().unwrap_or_default();
                Multiname { kind, name_idx, ns_idx: 0, name }
            }
            0x1B | 0x1C => { // MultinameL, MultinameLA
                let _ns_set = r.read_u30()?;
                Multiname { kind, name_idx: 0, ns_idx: 0, name: Arc::from("") }
            }
            0x1D => { // TypeName (generic)
                // A GENERIC IS STILL CALLED SOMETHING. `Vector.<int>` names Vector and then what
                // it holds; left nameless, it reads as a name worked out at runtime, and the
                // statement around it is rebuilt as one -- `m_underWaterList = new Vector.<int>()`
                // came back as `Vector.m_underWaterList = int.new()`, which assigns a field of the
                // wrong thing and leaves the real one null for the rest of the match. What it
                // holds does not survive the trip anyway, so it is carried by its base name.
                let base = r.read_u30()?;
                let param_count = r.read_u30()? as usize;
                for _ in 0..param_count { r.read_u30()?; }
                Multiname { kind, name_idx: base, ns_idx: 0, name: Arc::from("") }
            }
            _ => {
                log::warn!("Unknown multiname kind: 0x{:02X}", kind);
                Multiname { kind, name_idx: 0, ns_idx: 0, name: Arc::from("") }
            }
        };
        multinames.push(mn);
    }
    // ...and it is named from the one it is built on, which is only known once they all are.
    let generic_bases: Vec<(usize, usize)> = multinames.iter().enumerate()
        .filter(|(_, m)| m.kind == 0x1D && m.name.is_empty())
        .map(|(i, m)| (i, m.name_idx as usize))
        .collect();
    for (at, base) in generic_bases {
        if let Some(name) = multinames.get(base).map(|m| m.name.clone()) {
            if !name.is_empty() { multinames[at].name = name; }
        }
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
        let mut optionals: Vec<Option<f64>> = Vec::new();
        let mut optional_defaults: Vec<Option<String>> = Vec::new();
        if flags & 0x08 != 0 { // HAS_OPTIONAL
            let opt_count = r.read_u30()? as usize;
            for _ in 0..opt_count {
                let vidx = r.read_u30()? as usize;
                let kind = r.read_u8()?;
                optionals.push(match kind {
                    0x03 => ints.get(vidx).map(|v| *v as f64),      // Int
                    0x04 => uints.get(vidx).map(|v| *v as f64),     // UInt
                    0x06 => doubles.get(vidx).copied(),             // Double
                    _ => None,
                });
                optional_defaults.push(match kind {
                    0x03 => ints.get(vidx).map(|v| format!("{v}")),
                    0x04 => uints.get(vidx).map(|v| format!("{v}")),
                    0x06 => doubles.get(vidx).map(|v| if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", *v as i64) } else { format!("{v}") }),
                    0x01 => strings.get(vidx).map(|v| format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))), // Utf8
                    0x0B => Some("true".to_string()),
                    0x0A => Some("false".to_string()),
                    _ => None, // a null default (0x0C) changes nothing on the way in
                });
            }
        }
        if flags & 0x80 != 0 { // HAS_PARAM_NAMES
            for _ in 0..param_count { r.read_u30()?; }
        }
        let name = strings.get(name_idx as usize).cloned().unwrap_or_default();
        methods.push(Method { name_idx, name, param_count, return_type_idx, optionals, optional_defaults });
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
            if let Ok(t) = parse_trait(&mut r, &strings, &multinames, &ints, &uints, &doubles) {
                instance_methods.push(t);
            }
        }
        let name = multinames.get(name_idx as usize).map(|m| m.name.to_string()).unwrap_or_default();
        let super_name = multinames.get(super_name_idx as usize).map(|m| m.name.to_string()).unwrap_or_default();
        // Also resolve namespace-qualified name for _fla.* classes
        let ns_name = if let Some(mn) = multinames.get(name_idx as usize) {
            if mn.kind == 0x07 || mn.kind == 0x0D {
                // QName: namespace_idx is stored, get namespace string
                let ns_idx = mn.ns_idx as usize;
                if ns_idx < namespaces.len() && !namespaces[ns_idx].is_empty() {
                    format!("{}.{}", namespaces[ns_idx], mn.name)
                } else {
                    mn.name.to_string()
                }
            } else { mn.name.to_string() }
        } else { name.clone() };
        let full_name = if ns_name != name && ns_name.contains("_fla.") { ns_name } else { name.clone() };
        classes.push(Class { name: full_name, super_name, instance_methods, class_methods: vec![], constructor_idx, class_init_idx: 0 });
    }

    // Class infos (static traits). One class_info per class, in order — the first
    // loop pushed exactly `class_count` entries, so we can walk them directly.
    for class in classes.iter_mut() {
        class.class_init_idx = r.read_u30()?;
        let trait_count = r.read_u30()? as usize;
        for _ in 0..trait_count {
            if let Ok(t) = parse_trait(&mut r, &strings, &multinames, &ints, &uints, &doubles) {
                class.class_methods.push(t);
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
            if let Ok(t) = parse_trait(&mut r, &strings, &multinames, &ints, &uints, &doubles) {
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
            match parse_trait(&mut r, &strings, &multinames, &ints, &uints, &doubles) {
                Ok(t) => activation_traits.push(t),
                Err(_) => break,
            }
        }

        method_bodies.push(MethodBody { method_idx, max_stack, local_count, bytecode, activation_traits });
    }

    log::info!("ABC: {} methods, {} classes, {} method bodies", methods.len(), classes.len(), method_bodies.len());

    Ok(AbcFile { strings, ints, uints, doubles, multinames, methods, classes, scripts, method_bodies })
}

/// Every constant and field a class states a value for (or a type whose zero is the value):
/// `(name, value as the target writes it)`, statics first. What a projectile's behaviour class
/// keeps as `HAND_X_OFFSET = 30` is read by its clips as a bare name, so the script has to
/// declare it with that value.
pub fn class_const_values(abc: &AbcFile, class_name: &str) -> Vec<(String, String)> {
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return vec![] };
    let mut out: Vec<(String, String)> = Vec::new();
    for t in class.class_methods.iter().chain(class.instance_methods.iter()) {
        if !(t.kind == 0 || t.kind == 6) || t.name.is_empty() { continue }
        if out.iter().any(|(n, _)| n == &t.name) { continue }
        if let Some(rhs) = const_rhs(t) { out.push((t.name.clone(), rhs)); }
    }
    out
}

/// Whether a class is the api layer's own binding rather than content: its methods forward to
/// the engine object it wraps (`return _api.getX()`), which is the shape of the surface the
/// package ships for every object kind, not of anything a character wrote. A declaration that
/// names such a class as its behaviour has no code of its own to carry over.
pub fn is_api_binding_class(abc: &AbcFile, class_name: &str) -> bool {
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return false };
    let body_by_method: BTreeMap<u32, &MethodBody> =
        abc.method_bodies.iter().map(|b| (b.method_idx, b)).collect();
    let (mut methods, mut forwards) = (0usize, 0usize);
    for t in &class.instance_methods {
        if !matches!(t.kind, 1..=3) || t.name.is_empty() { continue }
        let Some(body) = body_by_method.get(&t.method_idx) else { continue };
        let params: Vec<String> = abc.methods.get(body.method_idx as usize)
            .map(|m| (0..m.param_count).map(|i| format!("arg{i}")).collect())
            .unwrap_or_default();
        let code = decompiler::decompile_method(body, abc, &t.name, &params);
        methods += 1;
        if code.contains("_api.") || code.contains("m_api.") { forwards += 1; }
    }
    methods >= 4 && forwards * 2 >= methods
}

/// Note the type a field is declared with; two declarations that disagree leave it unknown.
pub(crate) fn note_declared_type(types: &mut Vec<(String, String)>, name: &str, ty: Option<&str>) {
    let ty = ty.unwrap_or("*");
    match types.iter_mut().find(|(n, _)| n == name) {
        Some((_, t)) => if t != ty { *t = "*".to_string(); },
        None => types.push((name.to_string(), ty.to_string())),
    }
}

/// The value a class states for one of its constants, written the way the target reads it.
pub(crate) fn const_rhs(t: &Trait) -> Option<String> {
    if let Some(v) = t.default {
        return Some(if v.fract() == 0.0 && v.abs() < 9e15 {
            format!("{}", v as i64)
        } else {
            format!("{v}")
        });
    }
    if let Some(s) = t.default_str.as_ref() { return Some(format!("{s:?}")); }
    if let Some(l) = t.default_lit.as_ref() { return Some(l.clone()); }
    // a typed slot with no stated value starts at its type's zero: a counter declared `int`
    // is 0 before anything writes it (dream land's whispy counts fighters from it every idle
    // frame, and a null there reads past the list)
    match t.type_name.as_deref() {
        Some("int") | Some("uint") | Some("Number") => Some("0".to_string()),
        Some("Boolean") => Some("false".to_string()),
        _ => None,
    }
}

fn parse_trait(r: &mut Reader, strings: &[Arc<str>], multinames: &[Multiname], ints: &[i32], uints: &[u32], doubles: &[f64]) -> Result<Trait> {
    let name_idx = r.read_u30()?;
    let kind_byte = r.read_u8()?;
    let kind = kind_byte & 0x0F;
    let has_metadata = kind_byte & 0x40 != 0;
    let name = multinames.get(name_idx as usize).map(|m| m.name.to_string()).unwrap_or_default();

    let mut default: Option<f64> = None;
    let mut default_str: Option<String> = None;
    let mut default_lit: Option<String> = None;
    let mut type_name: Option<String> = None;
    let (method_idx, slot_idx) = match kind {
        0 | 6 => { // Slot, Const
            let slot_id = r.read_u30()?;
            let type_idx = r.read_u30()?;
            if type_idx != 0 {
                type_name = multinames.get(type_idx as usize).map(|m| m.name.to_string());
            }
            let vindex = r.read_u30()?;
            if vindex != 0 {
                let vkind = r.read_u8()?;
                default = match vkind {
                    0x03 => ints.get(vindex as usize).map(|v| *v as f64),
                    0x04 => uints.get(vindex as usize).map(|v| *v as f64),
                    0x06 => doubles.get(vindex as usize).copied(),
                    _ => None,
                };
                // A constant is as often a NAME as a number -- which animation to play,
                // which state to go to. Read as nothing, the move it names never plays.
                if vkind == 0x01 {
                    default_str = strings.get(vindex as usize).map(|v| v.to_string());
                }
                default_lit = match vkind {
                    0x0B => Some("true".to_string()),
                    0x0A => Some("false".to_string()),
                    0x0C | 0x00 => Some("null".to_string()),
                    _ => None,
                };
            }
            (0, slot_id)
        }
        1..=3 => { // Method, Getter, Setter
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

    Ok(Trait { name, kind, method_idx, slot_idx, default, default_str, default_lit, type_name })
}

// ─── Character data extraction ────────────────────────────────────────────────

/// Returns true if this frame method name is a root-MovieClip xframe setter
/// (e.g. "frame1", "frame23") as opposed to a real animation frame script
/// (e.g. "a__frame2", "stand__frame0").
/// Root xframe setters just do `self.xframe = "..."; self.stop();` and have
/// no gameplay value in Fraymakers — they only drive animation name extraction.
pub(crate) fn is_root_xframe_method(name: &str) -> bool {
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
                            return Some(s.to_string());
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
            OP_IFLE | OP_IFGT | OP_IFGE | OP_IFSTRICTEQ | OP_IFSTRICTNE
                if i + 3 <= bytecode.len() => { i += 3; }
            _ => {}
        }
    }
    None
}

/// Extract character data by analyzing ABC bytecode. Path-2 implementation:
/// stats / attacks / projectiles come from `Main::get<X>()` (the "bundle"
/// — see `docs/path2_unification_plan.md`); behavior code (frame scripts,
/// ext methods, ext vars + inits) comes from the per-character `<X>Ext`
/// class plus the main character MovieClip and per-character sub-MCs.
pub fn extract_character(abc: &AbcFile, char_name: &str) -> Result<ExtractedCharacter> {
    extract_character_with_timeline(abc, char_name, &BTreeMap::new(), &Default::default(), &Default::default())
}

/// As `extract_character`, but with the main timeline's own answer for what each sub-MC class is
/// (see `sprite_parser::class_to_ssf2_anim`). Without it a clip whose label the mapping file does
/// not list cannot be named, and its frame scripts are dropped.

/// Rename a bare name everywhere it is CODE, and nowhere it is a string: a label spelled like
/// a method (`playAnimation("bounce")` beside a `bounce` the timeline had to keep under
/// `__own`) is still the label.
fn rename_outside_strings(code: &str, re: &regex::Regex, to: &str) -> String {
    static STR: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let strings = STR.get_or_init(|| regex::Regex::new(r#""(?:[^"\\]|\\.)*""#).unwrap());
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0;
    for m in strings.find_iter(code) {
        out.push_str(&re.replace_all(&code[cursor..m.start()], to));
        out.push_str(m.as_str());
        cursor = m.end();
    }
    out.push_str(&re.replace_all(&code[cursor..], to));
    out
}

pub fn extract_character_with_timeline(
    abc: &AbcFile,
    char_name: &str,
    class_anim: &BTreeMap<String, String>,
    // clips the TIMELINE places under another object (a projectile's stance clips): their code
    // is that object's, whatever their names look like (image_extractor::projectile_clip_classes)
    foreign_clips: &std::collections::BTreeSet<String>,
    // clips the fighter's own timeline places, at any depth: whose methods are the fighter's
    // (sprite_parser::build_xframe_sprite_map). the display tree says so; the names do not
    own_clips: &std::collections::BTreeSet<String>,
) -> Result<ExtractedCharacter> {
    let mut attacks: BTreeMap<String, AttackData> = BTreeMap::new();
    let mut projectiles: BTreeMap<String, ProjectileData> = BTreeMap::new();
    let mut stats: Option<CharStats> = None;
    let mut frame_scripts: BTreeMap<String, Vec<FrameAction>> = BTreeMap::new();
    let mut ext_methods: BTreeMap<String, String> = BTreeMap::new();
    let mut xframe_map: XframeMap = BTreeMap::new();
    // names a second class had to keep under `__own`; rewired where the blocks meet
    let mut kept_own: Vec<String> = Vec::new();
    // the names the TIMELINE had to keep under `__own` (a subset of the above): the only renames
    // its frame scripts are rewired for
    let mut timeline_renames: Vec<String> = Vec::new();
    // (clip label, method name): a helper clip that kept its own under `__own`, whose frame
    // scripts are rewired to it once every frame script is in
    let mut own_by_label: Vec<(String, String, String)> = Vec::new();
    // the methods each clip class ended up with, by their final names, so a class's own bodies
    // can be pointed at the class's own copies of a shared name
    let mut methods_by_label: Vec<(String, String)> = Vec::new();
    // (clip label, field, default): what a helper clip's own fields start as each time its
    // animation starts, since the source builds a fresh instance every time
    let mut fields_by_label: Vec<(String, String, String)> = Vec::new();
    let mut declared_defaults: Vec<(String, String, String)> = Vec::new();
    // (clip label, field, kept as): a field a clip declares under a name the fighter also keeps
    let mut own_fields_by_label: Vec<(String, String, String)> = Vec::new();
    // what the source says is read by calling it
    let mut getters: Vec<String> = Vec::new();
    let mut attack_sounds: Vec<String> = Vec::new();
    let mut voice_sounds: Vec<String> = Vec::new();
    let mut thumbnail: Option<String> = None;
    let mut linkage_id: Option<String> = None;

    // ── Stage A: stats / attacks / projectiles from Main::get<X>() ─────
    let mut derived_from: Option<DerivedFrom> = None;
    if let Some((body, method_name)) = find_bundle_method(abc, char_name) {
        log::info!("path-2: extracting Stage A from Main::{}", method_name);
        attacks     = extract_attack_objects(&body.bytecode, abc);
        projectiles = extract_projectile_objects(&body.bytecode, abc);
        stats       = extract_ssf2_stats(&body.bytecode, abc);

        // SSF2 sound tables — the 1-based attackSound{N}_id / attackVoice{N}_id
        // fields that playAttackSound(N) / playVoiceSound(N) index into.
        // the clip this character names as its damage-counter picture
        thumbnail = extract_string_stat(body, abc, "thumbnail");
        linkage_id = extract_string_stat(body, abc, "linkage_id");
        attack_sounds = extract_indexed_string_fields(body, abc, "attackSound", "_id");
        voice_sounds  = extract_indexed_string_fields(body, abc, "attackVoice", "_id");
        log::info!("path-2: {} attack sound(s), {} voice sound(s)",
            attack_sounds.len(), voice_sounds.len());

        // Detect transformation: cData.normalStats_id ≠ derived char_name.
        // Recorded as metadata; Fraymakers has no native transformation
        // surface, so emission downstream is unchanged but the
        // CharacterStats.hx header gets a TODO banner.
        if let Some(nsi) = extract_normal_stats_id(body, abc) {
            if nsi != char_name {
                log::info!("path-2: {:?} is a transformation of {:?} (via Main::{})",
                    char_name, nsi, method_name);
                derived_from = Some(DerivedFrom {
                    parent_normal_stats_id: nsi,
                    source_method: format!("Main::{}", method_name),
                });
            }
        }
    } else {
        log::warn!("path-2: no Main::get* matching {:?}; Stage B behavior code will still be extracted", char_name);
    }

    // Build method name lookup: method_idx → name
    let mut method_names: BTreeMap<u32, String> = BTreeMap::new();
    for body in abc.method_bodies.iter() {
        if let Some(method) = abc.methods.get(body.method_idx as usize) {
            if !method.name.is_empty() {
                method_names.insert(body.method_idx, method.name.to_string());
            }
        }
    }

    // Find the XxxExt class (e.g. MarioExt) — holds getOwnStats + getAttackStats.
    // SSF2 class names don't always match the filename: CaptainFalcon → CaptainExt,
    // ChibiRobo → ChibiExt, GameAndWatch → gameandwatchExt, etc.
    // Strategy: try exact match, then prefix match on Ext classes, then any single Ext class.
    let char_lower = char_name.to_lowercase();
    // a package with no name of its own is searched by whoever borrows from it, so the name can
    // be empty here; taking the tail of an empty one is a slice past its end
    let ext_class_name = format!("{}Ext",
        char_name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()
        + char_name.char_indices().nth(1).map_or("", |(i, _)| &char_name[i..]));
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
    let mut ext_var_types: Vec<(String, String)> = Vec::new();
    if let Some(ext) = char_class {
        // Collect Slot (kind=0) / Const (kind=6) instance traits — the SSF2
        // class's `public var foo:T;` declarations. We carry these over to
        // Script.hx as top-level `var foo;` so the references later in the
        // translated methods aren't undeclared.
        // A constant the class states on ITSELF rather than on each instance is still a
        // constant its methods read. Read off `self` it is the same name, so it is carried
        // the same way; without it every comparison against it is a comparison with null.
        for t in &ext.class_methods {
            if (t.kind == 0 || t.kind == 6) && !t.name.is_empty() {
                if !ext_vars.contains(&t.name) { ext_vars.push(t.name.clone()); }
                note_declared_type(&mut ext_var_types, &t.name, t.type_name.as_deref());
                if let Some(rhs) = const_rhs(t) {
                    if !ext_var_inits.iter().any(|(n, _)| n == &t.name) {
                        ext_var_inits.push((t.name.clone(), rhs));
                    }
                }
            }
        }
        // debug (`PEPTIDE_DUMP_TRAITS=<class>`): every instance trait of the class, with its kind
        if std::env::var("PEPTIDE_DUMP_TRAITS").is_ok_and(|w| ext.name.contains(&w)) {
            for t in &ext.instance_methods { eprintln!("[trait] {} {} kind={}", ext.name, t.name, t.kind); }
        }
        for t in &ext.instance_methods {
            if (t.kind == 0 || t.kind == 6) && !t.name.is_empty() {
                ext_vars.push(t.name.clone());
                note_declared_type(&mut ext_var_types, &t.name, t.type_name.as_deref());
                // A constant the class DECLARES a value for. Without it every read of
                // it is null, and the move that compares against it never fires: dedede
                // charges against a CHARGE_MAX of nothing. The constructor pass below
                // covers the ones written there; this covers the ones the declaration
                // itself carries.
                if let Some(rhs) = const_rhs(t) {
                    if !ext_var_inits.iter().any(|(n, _)| n == &t.name) {
                        ext_var_inits.push((t.name.clone(), rhs));
                    }
                }
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
                // a constructor write to a field the class never declared is that field's
                // initial value all the same (the class is dynamic in the source)
                let undeclared_lit = crate::re!(r#"^self\.([A-Za-z_]\w*) = (true|false|-?\d+(?:\.\d+)?|"[^"]*"|null);$"#);
                let mut also: Vec<(String, String)> = Vec::new();
                for line in translated.lines() {
                    let trimmed = line.trim();
                    if let Some(c) = undeclared_lit.captures(trimmed) {
                        let n = c[1].to_string();
                        if !ext_vars.contains(&n) && !also.iter().any(|(k, _)| *k == n) {
                            also.push((n, c[2].to_string()));
                            continue;
                        }
                    }
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
                for (n, rhs) in also {
                    ext_vars.push(n.clone());
                    ext_var_inits.push((n, rhs));
                }
            }
        }
        for t in &ext.instance_methods {
            let Some(body) = body_by_method.get(&t.method_idx) else { continue };
            match t.name.as_str() {
                // Stat getters are no-ops here — stats/attacks/projectiles
                // come from Main::get<X>() (Stage A, applied at the top of
                // this function). The names are still listed in the
                // catch-all exclusion below so they don't end up in
                // Script.hx as helper methods.
                "getOwnStats" | "getAttackStats" | "getProjectileStats" | "getItemStats" => {}
                name if is_root_xframe_method(name) => {
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
                // a method, a getter or a setter (kind 1..3) and nothing else: a slot's
                // `method_idx` is not a method at all, and read as one it decompiled some other
                // method's body under the slot's name (pacman's `dotArray`, a field, came out as
                // a function too; the field lost its name to the collision and stayed empty)
                name if !matches!(name, "getOwnStats" | "getAttackStats" | "getItemStats" | "getProjectileStats")
                    && matches!(t.kind & 0x0F, 1..=3) => {
                        // Get param count from method signature
                        let params: Vec<String> = if let Some(method) = abc.methods.get(body.method_idx as usize) {
                            (0..method.param_count).map(|i| format!("arg{}", i)).collect()
                        } else {
                            vec![]
                        };
                        let code = decompiler::decompile_method(body, abc, name, &params);
                        // a getter is read by calling it, and the read sites have to say so
                        if t.kind & 0x0F == 2 && !getters.contains(&name.to_string()) {
                            getters.push(name.to_string());
                        }
                        ext_methods.insert(name.to_string(), code);
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
                .max_by_key(|c| c.instance_methods.iter().filter(|t| is_root_xframe_method(&t.name)).count())
                .filter(|c| c.instance_methods.iter().any(|t| is_root_xframe_method(&t.name)))
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
        // names the timeline had to keep under a different one, so its own code still reaches it
        let mut renamed_on_timeline: Vec<String> = Vec::new();
        for t in &mc.instance_methods {
            if is_root_xframe_method(&t.name) {
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
            } else if matches!(t.kind, 1..=3) && !t.name.is_empty()
                && !STAT_GETTERS.contains(&t.name.as_str())
            {
                // kind 1/2/3 = Method/Getter/Setter; skip Slot/Const (0/6).
                //
                // A name belongs to the class that declares it, and two classes in one package
                // are free to use the same one for different things. Dropping the timeline's
                // copy because another class got there first leaves the timeline's own frames
                // registering a handler by name and reaching somebody else's body: kirby's
                // inhale polls for a foe it already holds, and instead runs the version that
                // expects to be HANDED one, on nothing, every frame it inhales.
                let Some(body) = body_by_method.get(&t.method_idx) else { continue };
                let params: Vec<String> = if let Some(method) = abc.methods.get(body.method_idx as usize) {
                    (0..method.param_count).map(|i| format!("arg{}", i)).collect()
                } else { vec![] };
                let taken = ext_methods.contains_key(&t.name);
                let name = if taken { format!("{}__own", t.name) } else { t.name.clone() };
                let code = with_param_defaults(decompiler::decompile_method(body, abc, &name, &params), abc.methods.get(body.method_idx as usize));
                if t.kind == 2 && !getters.contains(&name) { getters.push(name.clone()); }
                if taken {
                    log::info!("'{}' is declared by two classes here; the timeline keeps its own as '{}'",
                        t.name, name);
                    renamed_on_timeline.push(t.name.clone());
                    timeline_renames.push(t.name.clone());
                    kept_own.push(t.name.clone());
                }
                ext_methods.insert(name, code);
            }
        }
        // the timeline asked for its own by the name it wrote, so point those at where it went
        for n in &renamed_on_timeline {
            let re = crate::dyn_re(&format!(r"\b{}\b", regex::escape(n)));
            let to = format!("{n}__own");
            for v in frame_scripts.values_mut() {
                for a in v.iter_mut() { a.action = rename_outside_strings(&a.action, &re, &to); }
            }
            if let Some(code) = ext_methods.remove(&to) {
                ext_methods.insert(to.clone(), rename_outside_strings(&code, &re, &to));
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
        let clip_lit = crate::re!(r#"^\s*self\.([A-Za-z_]\w*)\s*=\s*(true|false|-?[\d.]+|"[^"]*"|null)\s*;"#);
        let clip_write = crate::re!(r"(?:^|[^\w.])self\.([A-Za-z_]\w*)\s*=[^=]");
        for class in &abc.classes {
            // Skip the Ext class and the main class (already processed).
            if Some(class.name.as_str()) == char_class.map(|c| c.name.as_str()) { continue; }
            if Some(class.name.as_str()) == main_class.map(|c| c.name.as_str()) { continue; }
            // this fighter's clips are the ones its own timeline places, at any depth. a name
            // says nothing: in a package two fighters share, every clip class carries the
            // package's name, and one fighter's neutral special is spelled like the other's
            if !own_clips.contains(&class.name) { continue; }
            // a clip the timeline places inside a projectile is the projectile's: its handlers
            // (a keyblade's wall bounce) belong on the projectile's script, and folded in here
            // they ran on nobody (sora's `wall` sat in the fighter, struck line by line)
            if foreign_clips.contains(&class.name) { continue; }
            // A CLASS THAT IS ITS OWN OBJECT IS NOT PART OF THE FIGHTER. What extends the
            // fighter's own base, or a plain clip, is the fighter's; what extends an item or a
            // projectile base is the thing the fighter throws, and its update belongs to that
            // thing. Link's bomb class was folded into link himself: its update ran on him every
            // frame, timing a bomb he was not holding against a limit he does not have, and
            // "exploded" him out of every walk.
            let own_base = char_class.map(|c| c.super_name.as_str()).unwrap_or("");
            let is_clip = class.super_name.ends_with("MovieClip");
            if !is_clip && !own_base.is_empty() && class.super_name != own_base {
                log::info!("{} extends {}, not {}: its own object, not folded into the fighter",
                    class.name, class.super_name, own_base);
                continue;
            }
            for t in &class.instance_methods {
                if !matches!(t.kind, 1..=3) { continue; }
                // a timeline frame script is `frame<N>`; a method that merely starts with the
                // word (kirby's copied `frameCount`) is the clip's own and folds like any other
                if t.name.is_empty() || is_root_xframe_method(&t.name) { continue; }
                if STAT_GETTERS.contains(&t.name.as_str()) { continue; }
                let Some(body) = body_by_method.get(&t.method_idx) else { continue };
                let params: Vec<String> = if let Some(m) = abc.methods.get(body.method_idx as usize) {
                    (0..m.param_count).map(|i| format!("arg{}", i)).collect()
                } else { vec![] };
                // A name belongs to the class that declares it. Dropping this one because
                // another class got there first leaves this class's own frames registering a
                // handler by name and reaching somebody else's body.
                let taken = ext_methods.contains_key(&t.name);
                let class_short: String = class.name.rsplit('.').next().unwrap_or(&class.name).chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
                // A NAME MORE THAN ONE CLASS DECLARES is kept per class, under the class's own
                // name: goku's six specials each declare a `checkControls` of their own, and one
                // shared suffix for "the other one" kept only the last (the neutral special's
                // release check was the charge shot's, and the charge never ended)
                let name = if taken { format!("{}__{}", t.name, class_short) } else { t.name.clone() };
                let code = with_param_defaults(decompiler::decompile_method(body, abc, &name, &params), abc.methods.get(body.method_idx as usize));
                if t.kind == 2 && !getters.contains(&name) { getters.push(name.clone()); }
                if let Some(label) = class_anim.get(&class.name) { methods_by_label.push((label.clone(), name.clone())); }
                if taken {
                    log::info!("'{}' is declared by two classes here; {} keeps its own as '{}'",
                        t.name, class.name, name);
                    kept_own.push(t.name.clone());
                    if let Some(label) = class_anim.get(&class.name) {
                        own_by_label.push((label.clone(), t.name.clone(), name.clone()));
                    }
                }
                ext_methods.insert(name, code);
            }
            // A CLIP'S FIELDS START OVER WITH THE CLIP. The source builds a fresh instance of a
            // helper clip every time its animation plays, and the instance's fields start at
            // the values the class declares. Flattened into one script they became one
            // persistent set shared by every clip that spells a name the same way: fox's jab
            // read a `pressed2` another clip had left true and chained into its second hit on
            // a single tap. Each clip's declared fields are set back to their defaults on the
            // first frame of its animation.
            if let Some(label) = class_anim.get(&class.name) {
                // a declared slot carries its default; a slot the clip's code never writes is
                // one the timeline binds (its boxes) or a typed nothing, and is left alone,
                // since a write here would make it a persistent field of the fighter
                for t in &class.instance_methods {
                    if (t.kind == TRAIT_SLOT || t.kind == TRAIT_CONST) && !t.name.is_empty() {
                        declared_defaults.push((label.clone(), t.name.clone(),
                            const_rhs(t).unwrap_or_else(|| "null".to_string())));
                        note_declared_type(&mut ext_var_types, &t.name, t.type_name.as_deref());
                    }
                }
                // what the clip's own code writes is its own field: a clip class is dynamic in
                // the source, so a field it simply writes exists on the fresh instance and
                // starts undefined, or at what its constructor writes. `pressed2` is one.
                let mut own_writes: std::collections::BTreeMap<String, String> = Default::default();
                let ctor_body = body_by_method.get(&class.constructor_idx).map(|b| {
                    let params: Vec<String> = abc.methods.get(b.method_idx as usize)
                        .map(|m| (0..m.param_count).map(|i| format!("arg{i}")).collect()).unwrap_or_default();
                    decompiler::decompile_method(b, abc, "__ctor", &params)
                });
                if let Some(body) = &ctor_body {
                    for line in body.lines() {
                        if let Some(c) = clip_lit.captures(line) { own_writes.insert(c[1].to_string(), c[2].to_string()); }
                    }
                }
                for t in &class.instance_methods {
                    if !matches!(t.kind, 1..=3) { continue; }
                    let Some(body) = body_by_method.get(&t.method_idx) else { continue };
                    let params: Vec<String> = abc.methods.get(body.method_idx as usize)
                        .map(|m| (0..m.param_count).map(|i| format!("arg{i}")).collect()).unwrap_or_default();
                    let code = decompiler::decompile_method(body, abc, &t.name, &params);
                    for c in clip_write.captures_iter(&code) {
                        own_writes.entry(c[1].to_string()).or_insert_with(|| "null".to_string());
                    }
                }
                let written: std::collections::BTreeSet<String> = own_writes.keys().cloned().collect();
                // A FIELD THE CLIP DECLARES IS THE CLIP'S, whatever the fighter calls its own, and
                // whether it is declared as a plain field or as a property with accessors (one
                // build of a package does the one, another the other). bomberman's neutral
                // special keeps a `bombArray` of its own, copied from the fighter's and reset to
                // null on its first frame; sharing the fighter's slot, that reset threw away
                // every bomb the fighter had out. Kept under the class's name, like a method two
                // classes declare. A slot the clip only reads is left shared.
                for t in &class.instance_methods {
                    let declared = matches!(t.kind, TRAIT_SLOT | TRAIT_CONST | TRAIT_GETTER | TRAIT_SETTER);
                    if !declared || t.name.is_empty() || t.name == "self" || !ext_vars.contains(&t.name) { continue }
                    // a slot the clip only reads, with no value of its own, is one its timeline
                    // binds (a picture placed on it: dedede's `gordo`), the same one the fighter
                    // reaches by that name
                    if const_rhs(t).is_none() && !written.contains(&t.name) { continue }
                    if own_fields_by_label.iter().any(|(l, n, _)| l == label && *n == t.name) { continue }
                    let class_short: String = class.name.rsplit('.').next().unwrap_or(&class.name)
                        .chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
                    let to = format!("{}__{}", t.name, class_short);
                    own_fields_by_label.push((label.clone(), t.name.clone(), to.clone()));
                    note_declared_type(&mut ext_var_types, &to, t.type_name.as_deref());
                    fields_by_label.push((label.clone(), to,
                        const_rhs(t).unwrap_or_else(|| "null".to_string())));
                }
                for (n, default) in own_writes {
                    if n == "self" || ext_vars.contains(&n) { continue; }
                    let default = if default == "null" {
                        declared_defaults.iter().find(|(l, f, _)| l == label && *f == n)
                            .map(|(_, _, d)| d.clone()).unwrap_or(default)
                    } else { default };
                    fields_by_label.push((label.clone(), n, default));
                }
                // a slot the clip only READS, declared with a value, is a constant of the clip:
                // pikachu's down special parks its thunder cloud `BOLTDISTANCE` above the
                // fighter and never writes the name. it starts at the declared value too
                for t in &class.instance_methods {
                    if !(t.kind == TRAIT_SLOT || t.kind == TRAIT_CONST) || t.name.is_empty() { continue; }
                    let Some(rhs) = const_rhs(t) else { continue };
                    if t.name == "self" || ext_vars.contains(&t.name) { continue; }
                    if fields_by_label.iter().any(|(l, f, _)| l == label && *f == t.name) { continue; }
                    // kept under its own name already (it shares the fighter's)
                    if own_fields_by_label.iter().any(|(l, f, _)| l == label && *f == t.name) { continue; }
                    fields_by_label.push((label.clone(), t.name.clone(), rhs));
                }
            }
        }
        // this class asked for its own by the name it wrote, so point those at where it went.
        // ITS OWN BODIES ONLY: the timeline's frame scripts mean the timeline's method, the one
        // that kept the plain name. Rewritten too, samus's jab called the charge shot's control
        // check from its fourth frame and ended there.
        let _ = &kept_own;
    }

    // --- Also scan script-level traits for frame* methods ---
    // In SSF2, the character's main MovieClip frame scripts are compiled as
    // script-level (top-level) functions, NOT as class instance methods.
    // These appear in abc.scripts[*].traits, not in any class.
    if xframe_map.is_empty() || frame_scripts.is_empty() {
        for script in &abc.scripts {
            for t in &script.traits {
                if !is_root_xframe_method(&t.name) { continue; }
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
    let afs_mn_idx = abc.multinames.iter().position(|mn| &*mn.name == "addFrameScript");
    if let Some(afs_idx) = afs_mn_idx {
        // Encode afs_idx as u30 for pattern matching
        let mut afs_enc = Vec::new();
        let mut v = afs_idx as u32;
        loop { if v < 0x80 { afs_enc.push(v as u8); break; } afs_enc.push((v as u8 & 0x7F) | 0x80); v >>= 7; }
        let callpropvoid_afs: Vec<u8> = std::iter::once(0x4Fu8).chain(afs_enc.iter().copied()).collect();
        let callproperty_afs: Vec<u8> = std::iter::once(0x46u8).chain(afs_enc.iter().copied()).collect();

        
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

            // WHICH animation this clip belongs to is a question only the TIMELINE can answer: the
            // root places a clip under the `stance` instance on a labelled frame, and that label is
            // the animation. A clip the walk never reaches is not part of any animation, and that
            // is an answer, not a gap to paper over with its symbol name.
            //
            // Symbol names cannot be used for this even as a fallback. They are Flash-IDE
            // artifacts and they go stale: donkeykong's file still carries `ichigo_*` clips from
            // another character, and reading those names attributed Ichigo's frame scripts to
            // DonkeyKong's `special`.
            let ssf2_anim = class_anim.get(&class.name).cloned();
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
                                                .map(|m| m.name.to_string()).unwrap_or_default();
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
                                    frame_scripts.entry(script_key).or_default()
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
            .find(|(_, mn)| &*mn.name == "xframe")
            .map(|(i, _)| i);

        // Also check ALL multinames named 'xframe' (might be QName and Multiname variants)
        let xframe_mn_indices: Vec<usize> = abc.multinames.iter().enumerate()
            .filter(|(_, mn)| &*mn.name == "xframe")
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
                                            seen_anims.insert(s.to_string());
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

    // Per-character costume data isn't extracted here — both
    // getCostumeData() and applyPalette() are thin wrappers around the
    // engine SWF's runtime API. The real costumes for misc.ssf-style
    // characters come from `scan_all_costume_methods` against misc.ssf
    // (driven from main.rs); palette_gen.rs falls back to k-means on
    // sprite imagery when that isn't available.
    // WHAT THE CHARACTER BUILDS FOR ITSELF. A class the character's own code says `new` about
    // is part of that character's behaviour, and left behind it is a name nothing declares --
    // the whole handler that reaches for it stops there. Ported as a factory (see
    // reconstruct_class_as_factory), so `new X(a)` becomes `X__new(a)` and everything the
    // caller does with what comes back reads the same.
    //
    // Which classes: the ones this package carries that the character's own code builds. The
    // package states both, so nothing here has to know what any of them are.
    {
        let built: std::collections::BTreeSet<String> = {
            let re = crate::re!(r"\bnew\s+([A-Za-z_]\w*)\s*\(");
            ext_methods.values()
                .flat_map(|code| re.captures_iter(code).map(|c| c[1].to_string()))
                .collect()
        };
        for name in &built {
            if class_named(abc, name).is_none() { continue }
            let Some(factory) = reconstruct_class_as_factory(abc, name) else { continue };
            log::info!("class '{name}' the character builds for itself, ported as a factory");
            ext_methods.insert(format!("{name}__new"), factory);
            for code in ext_methods.values_mut() {
                *code = code.replace(&format!("new {name}("), &format!("{name}__new("));
            }
        }

        // A class it never builds but reads names off is a place the package keeps names.
        let named: std::collections::BTreeSet<String> = {
            let re = crate::re!(r"\b([A-Z][A-Za-z0-9_]*)\.[A-Za-z_]");
            ext_methods.values()
                .flat_map(|code| re.captures_iter(code).map(|c| c[1].to_string()))
                .collect()
        };
        // NOT a name either engine states itself. A package carries its own copy of names
        // the engine also keeps -- the two are different sets of numbers for the same idea
        // -- and writing ours out would put the source's numbers where the engine's belong.
        // The same goes for the source engine's own namespaces, which the port answers by
        // renaming or by striking. Which names those are is not a list to keep: the
        // mappings already name every one they have an answer for, on both sides.
        let targets_own: std::collections::BTreeSet<String> = {
            let re = crate::re!(r"\b([A-Z][A-Za-z0-9_]*)\.[A-Za-z_]");
            let cmds = crate::mappings::api_commands();
            cmds.replacements.iter().flat_map(|r| [r.from.clone(), r.to.clone()])
                .chain(cmds.regex_replacements.iter()
                    .flat_map(|r| [r.pattern.clone(), r.replacement.clone()]))
                .flat_map(|t| re.captures_iter(&t).map(|c| c[1].to_string()).collect::<Vec<_>>())
                .collect()
        };
        for name in named {
            if built.contains(&name) { continue }
            if targets_own.contains(&name) { continue }
            if class_named(abc, &name).is_none() { continue }
            let Some(consts) = reconstruct_class_as_constants(abc, &name) else { continue };
            log::info!("class '{name}' the character reads names off, ported as those names");
            ext_methods.insert(format!("{name}__names"), consts);
        }
    }

    log::info!("Extracted {} attacks, {} frame scripts, {} ext methods, {} xframe mappings, stats={}",
        attacks.len(), frame_scripts.len(), ext_methods.len(), xframe_map.len(), stats.is_some());

    // A FRAME SCRIPT OF A CLIP MEANS THAT CLIP'S METHOD. The clip's frame scripts run in the
    // clip's own scope in the source, so a bare call there reaches the clip's method, not the
    // timeline's of the same name: donkey kong's jab clip calls its own `checkControls`, and
    // flattened the call reached the charge shot's, which ended the jab on its sixth frame.
    // Done here, after the sub-MC scan has filled the frame scripts.
    for (label, n, to) in &own_by_label {
        let bare = crate::dyn_re(&format!(r"((?:self\.)+){}\(", regex::escape(n)));
        let handed = crate::dyn_re(&format!(r"((?:create|add|destroy|remove)(?:Timer|EventListener)\([^()]*,\s*(?:self\.)*){}\b", regex::escape(n)));
        let cancelled = crate::dyn_re(&format!(r"((?:destroy|remove)Timer\(\s*(?:self\.)*){}\b", regex::escape(n)));
        for (k, v) in frame_scripts.iter_mut() {
            if !k.starts_with(&format!("{label}__")) { continue; }
            for a in v.iter_mut() {
                a.action = bare.replace_all(&a.action, format!("${{1}}{to}(")).into_owned();
                a.action = handed.replace_all(&a.action, format!("${{1}}{to}")).into_owned();
                a.action = cancelled.replace_all(&a.action, format!("${{1}}{to}")).into_owned();
            }
        }
        // ...and the class's own methods reach their own copy the same way (its release check
        // cancels its own timer by name)
        for (l, m) in &methods_by_label {
            if l != label { continue; }
            if let Some(code) = ext_methods.remove(m) {
                let code = bare.replace_all(&code, format!("${{1}}{to}(")).into_owned();
                let code = handed.replace_all(&code, format!("${{1}}{to}")).into_owned();
                let code = cancelled.replace_all(&code, format!("${{1}}{to}")).into_owned();
                ext_methods.insert(m.clone(), code);
            }
        }
    }
    // A FIELD A CLIP KEEPS UNDER THE NAME OF A FUNCTION THE PORT HAS. Everything shares one
    // scope here, so the clip's write replaced the function: megaman's up special keeps
    // `grounded = isOnFloor()`, and his landing handler, also `grounded`, became `true`. Asked
    // once every class is in, since the function may come from a class read after the clip.
    {
        let mut renamed: Vec<(usize, String)> = Vec::new();
        for (i, (label, f, _)) in fields_by_label.iter().enumerate() {
            if !ext_methods.contains_key(f) { continue }
            // its own clip's method of that name is the same object's, not a collision
            if methods_by_label.iter().any(|(l, m)| l == label && m == f) { continue }
            if own_fields_by_label.iter().any(|(l, n, _)| l == label && n == f) { continue }
            let to = format!("{f}__{}", label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect::<String>());
            own_fields_by_label.push((label.clone(), f.clone(), to.clone()));
            renamed.push((i, to));
        }
        for (i, to) in renamed { fields_by_label[i].1 = to; }
    }
    // ...and a clip's own FIELDS the same way: `self.<field>` in its frames and methods is the
    // clip's copy; the fighter's is reached through whatever the clip holds the fighter in
    for (label, n, to) in &own_fields_by_label {
        // the field, not a call of the fighter's method of the same name
        let own = crate::dyn_re(&format!(r"(^|[^\w.])self\.{}\b(\s*)([^\s(]|$)", regex::escape(n)));
        let to_field = format!("${{1}}self.{to}${{2}}${{3}}");
        for (k, v) in frame_scripts.iter_mut() {
            if !k.starts_with(&format!("{label}__")) { continue; }
            for a in v.iter_mut() {
                a.action = own.replace_all(&a.action, to_field.as_str()).into_owned();
            }
        }
        for (l, m) in &methods_by_label {
            if l != label { continue; }
            if let Some(code) = ext_methods.remove(m) {
                let code = own.replace_all(&code, to_field.as_str()).into_owned();
                ext_methods.insert(m.clone(), code);
            }
        }
    }
    // ...and what a clip's own FRAME SCRIPTS write, which are its fields too (fox's jab writes
    // `pressed1` from its second frame and nowhere else); a name already listed keeps its default
    {
        let write = crate::re!(r"(?:^|[^\w.])self\.([A-Za-z_]\w*)\s*=[^=]");
        let labels: std::collections::BTreeSet<String> = fields_by_label.iter().map(|(l, _, _)| l.clone())
            .chain(own_by_label.iter().map(|(l, _, _)| l.clone()))
            .chain(class_anim.values().cloned()).collect();
        let mut more: Vec<(String, String, String)> = Vec::new();
        for label in &labels {
            for (k, v) in frame_scripts.iter() {
                if !k.starts_with(&format!("{label}__")) { continue; }
                for a in v.iter() {
                    for c in write.captures_iter(&a.action) {
                        let n = c[1].to_string();
                        if n == "self" || ext_vars.contains(&n) { continue; }
                        if fields_by_label.iter().chain(more.iter()).any(|(l, f, _)| l == label && *f == n) { continue; }
                        if own_fields_by_label.iter().any(|(l, f, _)| l == label && *f == n) { continue; }
                        let default = declared_defaults.iter().find(|(l, f, _)| l == label && *f == n)
                            .map(|(_, _, d)| d.clone()).unwrap_or_else(|| "null".to_string());
                        more.push((label.clone(), n, default));
                    }
                }
            }
        }
        fields_by_label.extend(more);
        if std::env::var("PEPTIDE_DBG_RESET").is_ok() {
            for (l, f, d) in &fields_by_label { eprintln!("[reset] {l} {f} = {d}"); }
            eprintln!("[reset] ext_vars has pressed1: {}", ext_vars.contains(&"pressed1".to_string()));
        }
    }
    // the resets go at the top of the clip's first frame; a frame that already has a script
    // keeps it and gets the resets ahead of it (a second function of the same name would be
    // dropped, and the clip's own first-frame code may already read what is reset)
    for (label, field, default) in fields_by_label.iter().rev() {
        let key = format!("{label}__frame0");
        let head = format!("function {key}() {{\n");
        let reset = format!("\tself.{field} = {default};\n");
        let entry = frame_scripts.entry(key.clone()).or_default();
        match entry.iter_mut().find(|a| a.action.starts_with(&head)) {
            Some(a) => a.action.insert_str(head.len(), &reset),
            None => entry.insert(0, FrameAction { frame: 0, action: format!("{head}{reset}\treturn;\n}}"), args: vec![] }),
        }
    }

    // WHAT THE FIGHTER INHERITS FROM A BASE ITS OWN FILE SHIPS. The fighter's class extends a
    // helper class the package carries (sonic's up tilt pulls its victim along with a method it
    // inherits, `setupAutolinkAngle`, and the hit handler it hangs), and only the fighter's own
    // methods were translated, so every call into the base was struck. The base's methods the
    // fighter's code names, and those they name in turn, are translated with it, with the fields
    // they keep. What is not reached from the fighter's code stays out.
    if let Some(cc) = char_class {
        let short = |n: &str| n.rsplit(['.', ':']).next().unwrap_or(n).to_string();
        let base_short = short(&cc.super_name);
        if let Some(base) = abc.classes.iter().find(|k| k.name != cc.name && short(&k.name) == base_short) {
            let mut code_seen: String = ext_methods.values().cloned().collect::<Vec<_>>().join("\n");
            for acts in frame_scripts.values() { for a in acts { code_seen.push_str(&a.action); code_seen.push('\n'); } }
            let names = |code: &str, n: &str| crate::dyn_re(&format!(r"\b{}\b", regex::escape(n))).is_match(code);
            loop {
                let mut added = false;
                for t in &base.instance_methods {
                    if !matches!(t.kind & 0x0F, 1..=3) || t.name.is_empty() || ext_methods.contains_key(&t.name) { continue }
                    if matches!(t.name.as_str(), "getOwnStats" | "getAttackStats" | "getItemStats" | "getProjectileStats") || !names(&code_seen, &t.name) { continue }
                    // a name the mapping tables already translate (or say has no counterpart) is theirs
                    // (one translated where it is called still has the fighter's own body behind it)
                    if crate::api_mappings::the_mappings_handle(&t.name)
                        && !crate::mappings::api_commands().translated_in_code.contains(&t.name) { continue }
                    let Some(body) = body_by_method.get(&t.method_idx) else { continue };
                    let params: Vec<String> = abc.methods.get(body.method_idx as usize)
                        .map(|m| (0..m.param_count).map(|i| format!("arg{}", i)).collect()).unwrap_or_default();
                    // a name this engine answers itself is its own (a fighter's `flipX`)
                    if crate::api_mappings::the_engine_answers_to(&t.name) { continue }
                    let code = with_param_defaults(decompiler::decompile_method(body, abc, &t.name, &params), abc.methods.get(body.method_idx as usize));
                    // the engine's surface as the package declares it hands each call on to the
                    // engine object behind it under the same name: nothing of the content's own
                    // (the method naming ITSELF on `self`, to hand itself to a listener, is not that)
                    let forwards = crate::dyn_re(&format!(r"(?:^|[^\w.])(?:self\.)?(\w+)\.{}\b", regex::escape(&t.name)))
                        .captures_iter(&code).any(|c| &c[1] != "self");
                    if forwards { continue }
                    if t.kind & 0x0F == 2 && !getters.contains(&t.name) { getters.push(t.name.clone()); }
                    log::info!("'{}' is inherited from {}, translated with the fighter", t.name, base.name);
                    code_seen.push_str(&code);
                    ext_methods.insert(t.name.clone(), code);
                    added = true;
                }
                if !added { break }
            }
            // and the base's own fields those methods keep
            for t in &base.instance_methods {
                if matches!(t.kind & 0x0F, 0 | 6) && !t.name.is_empty() && !ext_vars.contains(&t.name)
                    && code_seen.contains(&format!("self.{}", t.name)) {
                    ext_vars.push(t.name.clone());
                    note_declared_type(&mut ext_var_types, &t.name, t.type_name.as_deref());
                    // with the value the base states for it (its knockback constants)
                    if let Some(rhs) = const_rhs(t) {
                        if !ext_var_inits.iter().any(|(n, _)| *n == t.name) { ext_var_inits.push((t.name.clone(), rhs)); }
                    }
                }
            }
        }
    }

    Ok(ExtractedCharacter {
        getters,
        // ONLY THE TIMELINE'S OWN RENAMES reach the frame scripts. A helper clip class that kept
        // its own name under `__own` had its own bodies rewired above; the timeline's scripts
        // handing that name to a timer mean the timeline's method, which kept the plain name.
        // Rewired too, samus's jab handed the charge shot's control check to its timer and
        // ended on its fourth frame.
        renamed_own: timeline_renames,
        name: char_name.to_string(),
        attacks,
        projectiles,
        stats,
        frame_scripts,
        ext_methods,
        ext_vars,
        ext_var_inits,
        ext_var_types,
        xframe_map,
        attack_sounds,
        voice_sounds,
        thumbnail,
        linkage_id,
        derived_from,
    })
}

/// Every method a CLIP's class declares, decompiled.
///
/// A projectile's behaviour lives here, not in its stats: the clip's class wires up what happens
/// when it hits a wall or touches the ground, and its frame scripts run when the timeline reaches
/// them. That is the same shape a stage hazard has, so it ports the same way -- translate the
/// class rather than recognise the thing.
///
/// Returns method name -> decompiled body, still in SSF2 terms; `api_mappings::translate_ssf2_to_fm`
/// is what turns those into engine calls.
pub fn extract_class_methods(abc: &AbcFile, class_name: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return out };
    let body_by_method: BTreeMap<u32, &MethodBody> =
        abc.method_bodies.iter().map(|b| (b.method_idx, b)).collect();
    for t in &class.instance_methods {
        // methods only: a slot or a const is a variable, not behaviour
        if !matches!(t.kind, 1..=3) || t.name.is_empty() { continue }
        let Some(body) = body_by_method.get(&t.method_idx) else { continue };
        let params: Vec<String> = abc.methods.get(body.method_idx as usize)
            .map(|m| (0..m.param_count).map(|i| format!("arg{i}")).collect())
            .unwrap_or_default();
        out.insert(t.name.clone(), decompiler::decompile_method(body, abc, &t.name, &params));
    }
    out
}

/// Find the body of `Main::get<X>()` where the lowercased method-name
/// suffix matches `char_name`. The match is the derived-id rule from
/// `docs/path2_unification_plan.md` §1: strip `get`, lowercase, compare
/// — no camelCase splitting, no underscore handling beyond preservation.
/// Returns the bundle method's body and the original method name.
pub fn find_bundle_method<'a>(abc: &'a AbcFile, char_name: &str)
    -> Option<(&'a MethodBody, String)>
{
    let main = abc.classes.iter().find(|c| c.name == "Main")?;
    let getter = main.instance_methods.iter().find(|t| {
        if !t.name.starts_with("get") { return false; }
        t.name[3..].to_lowercase() == char_name
    })?;
    let body = abc.method_bodies.iter().find(|b| b.method_idx == getter.method_idx)?;
    Some((body, getter.name.clone()))
}

/// The clip a character names as ITS OWN art: `linkage_id` in its bundle.
///
/// This is the file stating which of its clips is the fighter, so nothing has to work it out
/// from symbol names or from how a timeline is shaped. A package can also carry a stale list of
/// what it owns (one fighter's copied from another's), and this is not that list: it is the
/// fighter's own declaration, next to the stats and the thumbnail it also names.
pub fn declared_root_clip(abc: &AbcFile, char_name: &str) -> Option<String> {
    let (body, _) = find_bundle_method(abc, char_name)?;
    extract_string_stat(body, abc, "linkage_id").filter(|s| !s.is_empty())
}

/// Strip `get`, lowercase the remainder, preserve `_`. Returns None
/// for an empty/missing suffix. Mirrors `main.rs::derive_id_from_getter`
/// — kept in sync; both will collapse into one identity once the path 2
/// enumeration fallback is removed in a follow-up commit.
pub fn derive_id_from_bundle_method_name(method_name: &str) -> Option<String> {
    let suffix = method_name.strip_prefix("get")?;
    if suffix.is_empty() { return None; }
    Some(suffix.to_lowercase())
}

/// Derive the PascalCase form used for entity filenames and the
/// `library/scripts/<Pascal>/` subdir per
/// `docs/multi_character_projects_plan.md`. Rule: strip `get` (if
/// present), strip `_` characters, uppercase the first character.
///
/// Examples:
///   `getMario`        → `Mario`
///   `getSandbag`      → `Sandbag`
///   `getBandanaDee`   → `BandanaDee`
///   `getCaptainFalcon`→ `CaptainFalcon`
///   `getGigaBowser`   → `GigaBowser`
///   `getWario_Man`    → `WarioMan`
///   `getMegaMan`      → `MegaMan`
///   `getgameandwatch` → `Gameandwatch`  (no SSF2-side case info)
///   `sandbag`         → `Sandbag`       (no `get` prefix, fallback)
///   `wario_man`       → `Warioman`      (fallback path loses word boundary)
///
/// The fallback path (input without a `get` prefix) is meant for cases
/// where we don't have the SSF2 method name handy — `--name` override,
/// filename fallback for misc.ssf-like SWFs. Multi-char projects
/// always come through detection with the method name available, so
/// the fallback never fires for Wario Man et al.
pub fn pascal_form(method_or_id: &str) -> String {
    let suffix = method_or_id.strip_prefix("get").unwrap_or(method_or_id);
    let no_underscores: String = suffix.chars().filter(|c| *c != '_').collect();
    let mut chars = no_underscores.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Metadata declared by `Main`'s constructor — the SSF "table of contents".
/// Populated by `extract_main_package_metadata` from a single iinit body
/// walk. Used by detection (the `characters` list IS the roster) and by
/// the conversion log (id/guid go into `ssf2_source`).
#[derive(Debug, Clone, Default)]
pub struct MainPackageMetadata {
    /// Value of `register("id", ...)` — the SSF package id; matches the
    /// filename stem in every observed corpus SSF. None if the id call
    /// is absent (none observed) or the value didn't match the
    /// expected string-literal shape.
    pub id: Option<String>,
    /// Value of `register("guid", ...)` — per-SSF UUID. Useful for
    /// downstream traceability.
    pub guid: Option<String>,
    /// `(derived_id, method_name)` pairs collected from
    /// `register("characters", [self.getX(), self.getY(), ...])`.
    /// Order matches the array order in the iinit (which mirrors the
    /// engine's roster order). Empty when the walk failed — callers
    /// fall back to the path 2 instance-method enumeration.
    pub characters: Vec<(String, String)>,
    /// `register("music", {id:...}, …)` track ids (the `bgm_*` resource names) in
    /// order. For a stage, its intended SSF2 soundtrack; empty for characters / when
    /// no music block is present.
    pub music: Vec<String>,
    /// `register("camera", { backgrounds: [ … ] })` — the stage's camera-background (parallax)
    /// layers, in the order the package declares them.
    pub camera_backgrounds: Vec<CameraBackground>,
    /// `register("resources", { movieclips: [ … ] })` — every clip the package says it OWNS, by
    /// linkage id, in declaration order.
    ///
    /// This is the file stating its own contents, so it is the authority on which clips are the
    /// package's and what they are called. Anything found by scanning symbol names instead is a
    /// guess about content, and a guess cannot be right for content nobody has written yet.
    pub movieclips: Vec<String>,
    /// `register("resources", { sounds: [ … ] })` — the sounds the package declares, same rule.
    pub sounds: Vec<String>,
}

/// One declared camera background: a clip the engine scrolls behind the stage at its own rate.
///
/// This is a DECLARATION, not a placement: the clip is named by linkage and never appears on the
/// stage timeline as a parallax-tagged instance, so a converter that looks only at the timeline
/// (for a `_cambg` instance name, say) sees a stage with no parallax at all and bakes the whole
/// backdrop flat.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraBackground {
    /// The clip's linkage id (`clocktown_bg`).
    pub linkage_id: String,
    /// Per-axis pan rate: the layer moves this fraction of the camera's movement. `None` when the
    /// declaration does not state one at all, which is NOT the same as stating zero -- a background
    /// that says nothing is left to the engine to work out, and one that says zero is pinned to the
    /// view. Battlefield's declares only its linkage, and read as a zero it stopped scrolling.
    pub x_pan: Option<f64>,
    pub y_pan: Option<f64>,
    /// Whether the engine derives the pan itself rather than using the declared multipliers.
    pub auto_pan: bool,
}

/// Read Main's constructor and pull out `id`, `guid`, and the
/// `characters` array. Returns None if Main is absent.
pub fn extract_main_package_metadata(abc: &AbcFile) -> Option<MainPackageMetadata> {
    let main = abc.classes.iter().find(|c| c.name == "Main")?;
    let body = abc.method_bodies.iter().find(|b| b.method_idx == main.constructor_idx)?;
    Some(MainPackageMetadata {
        id:         scan_register_string_arg(body, abc, "id"),
        guid:       scan_register_string_arg(body, abc, "guid"),
        characters: scan_register_characters_array(body, abc),
        music:      scan_register_music_ids(body, abc),
        camera_backgrounds: scan_register_camera_backgrounds(body, abc),
        movieclips: scan_register_resource_list(body, abc, "movieclips"),
        sounds:     scan_register_resource_list(body, abc, "sounds"),
    })
}

/// The `resources` list a package declares under `which` (`movieclips` / `sounds`).
///
/// The block is a key followed by the literal strings that belong to it, and the next key (or the
/// `register` call) closes it. Read this way it needs no knowledge of what any of the names MEAN,
/// which is the point: the file is listing its own contents and the converter is only writing them
/// down.
fn scan_register_resource_list(body: &MethodBody, abc: &AbcFile, which: &str) -> Vec<String> {
    let key_idx = |k: &str| abc.strings.iter().position(|s| &**s == k);
    let Some(want) = key_idx(which) else { return Vec::new() };
    // the sibling keys that END this list
    let stops: Vec<usize> = ["movieclips", "sounds", "fonts", "bitmaps"].iter()
        .filter(|k| **k != which).filter_map(|k| key_idx(k)).collect();
    let bc = &body.bytecode;
    let (mut i, mut collecting) = (0usize, false);
    let mut out: Vec<String> = Vec::new();
    while i < bc.len() {
        let op = bc[i];
        let at = i;
        i += 1;
        match op {
            OP_PUSHSTRING => {
                let Some(idx) = read_u30_at(bc, &mut i) else { break };
                let idx = idx as usize;
                if idx == want { collecting = true; continue; }
                if collecting {
                    if stops.contains(&idx) { collecting = false; continue; }
                    if let Some(s) = abc.strings.get(idx) { out.push(s.to_string()); }
                }
            }
            OP_CALLPROPVOID | OP_CALLPROPERTY => {
                let Some(mn) = read_u30_at(bc, &mut i) else { break };
                let _ = read_u30_at(bc, &mut i);
                if collecting && abc.multinames.get(mn as usize).is_some_and(|m| &*m.name == "register") { break; }
            }
            _ => { let mut j = at + 1; skip_opcode_operands(op, bc, &mut j); i = j; }
        }
    }
    out.dedup();
    out
}

/// Read the `camera.backgrounds` entries out of Main's constructor.
///
/// The block is a literal object per layer: a `linkage_id` string then the pan multipliers, all
/// pushed in order between the `"backgrounds"` key and the `register` call that consumes it. So
/// each `linkage_id` opens an entry and the numbers that follow fill it.
fn scan_register_camera_backgrounds(body: &MethodBody, abc: &AbcFile) -> Vec<CameraBackground> {
    let key_idx = |k: &str| abc.strings.iter().position(|s| &**s == k);
    let (Some(bg_key), Some(link_key)) = (key_idx("backgrounds"), key_idx("linkage_id")) else { return Vec::new() };
    let x_key = key_idx("xPanMultiplier");
    let y_key = key_idx("yPanMultiplier");
    let auto_key = key_idx("autoPanMultiplier");

    let bc = &body.bytecode;
    let (mut i, mut collecting) = (0usize, false);
    let mut out: Vec<CameraBackground> = Vec::new();
    let mut field: Option<&str> = None;
    while i < bc.len() {
        let op = bc[i];
        let at = i;
        i += 1;
        match op {
            OP_PUSHSTRING => {
                let Some(idx) = read_u30_at(bc, &mut i) else { break };
                let idx = idx as usize;
                if idx == bg_key { collecting = true; field = None; continue; }
                if !collecting { continue; }
                if std::env::var("PEPTIDE_CAMBG_KEYS").is_ok() {
                    if let Some(t) = abc.strings.get(idx) { eprintln!("[cambg-key] {t:?}"); }
                }
                if idx == link_key { field = Some("linkage"); continue; }
                if Some(idx) == x_key { field = Some("x"); continue; }
                if Some(idx) == y_key { field = Some("y"); continue; }
                if Some(idx) == auto_key { field = Some("auto"); continue; }
                if field == Some("linkage") {
                    if let Some(s) = abc.strings.get(idx) {
                        out.push(CameraBackground { linkage_id: s.to_string(), ..Default::default() });
                    }
                    field = None;
                }
            }
            OP_PUSHDOUBLE | OP_PUSHINT | OP_PUSHSHORT | OP_PUSHBYTE if collecting => {
                let v = match op {
                    OP_PUSHDOUBLE => { let Some(x) = read_u30_at(bc, &mut i) else { break }; abc.doubles.get(x as usize).copied() }
                    OP_PUSHINT => { let Some(x) = read_u30_at(bc, &mut i) else { break }; abc.ints.get(x as usize).map(|v| *v as f64) }
                    OP_PUSHSHORT => { let Some(x) = read_u30_at(bc, &mut i) else { break }; Some(x as i32 as f64) }
                    _ => { let v = bc.get(i).map(|b| *b as i8 as f64); i += 1; v }
                };
                if std::env::var("PEPTIDE_CAMBG_KEYS").is_ok() {
                    eprintln!("[cambg-num] field={field:?} v={v:?}");
                }
                if let (Some(v), Some(e)) = (v, out.last_mut()) {
                    match field { Some("x") => e.x_pan = Some(v), Some("y") => e.y_pan = Some(v), _ => {} }
                }
                field = None;
            }
            OP_PUSHTRUE | OP_PUSHFALSE if collecting => {
                if let Some(e) = out.last_mut() {
                    if field == Some("auto") { e.auto_pan = op == OP_PUSHTRUE; }
                }
                field = None;
            }
            OP_CALLPROPVOID | OP_CALLPROPERTY => {
                let Some(mn) = read_u30_at(bc, &mut i) else { break };
                let _ = read_u30_at(bc, &mut i);
                if collecting && abc.multinames.get(mn as usize).is_some_and(|m| &*m.name == "register") { break; }
            }
            _ => { let mut j = at + 1; skip_opcode_operands(op, bc, &mut j); i = j; }
        }
    }
    out
}

/// Walk Main's constructor to collect the `register("music", {id:...}, …)` track
/// ids — every `bgm_*` string between the `"music"` key and the `register` call that
/// consumes it (the SSF2 stage's intended soundtrack, in order).
fn scan_register_music_ids(body: &MethodBody, abc: &AbcFile) -> Vec<String> {
    let Some(music_key) = abc.strings.iter().position(|s| &**s == "music") else { return Vec::new() };
    let bc = &body.bytecode;
    let (mut i, mut out, mut collecting) = (0usize, Vec::new(), false);
    while i < bc.len() {
        let op = bc[i];
        if op == OP_PUSHSTRING {
            let mut j = i + 1;
            if let Some(s_idx) = read_u30_at(bc, &mut j) {
                if s_idx as usize == music_key {
                    collecting = true;
                } else if collecting {
                    if let Some(s) = abc.strings.get(s_idx as usize) {
                        if s.starts_with("bgm_") { out.push(s.to_string()); }
                    }
                }
            }
            i = j;
            continue;
        }
        // the first call after the "music" key is `register("music", …)` — end of block.
        if collecting && (op == OP_CALLPROPVOID || op == OP_CALLPROPERTY) { break; }
        i += 1;
        skip_opcode_operands(op, bc, &mut i);
    }
    out
}

/// Walk a method body to find `pushstring KEY ; pushstring VALUE ;
/// callpropvoid register, 2` and return `VALUE`. Used for the
/// string-valued `register` keys (`id`, `guid`).
fn scan_register_string_arg(body: &MethodBody, abc: &AbcFile, key: &str) -> Option<String> {
    let key_idx = abc.strings.iter().position(|s| &**s == key)? as u32;
    let bc = &body.bytecode;
    let mut i = 0;
    while i < bc.len() {
        let op = bc[i];
        if op == OP_PUSHSTRING {
            let mut j = i + 1;
            let s_idx = read_u30_at(bc, &mut j)?;
            // The id/guid pattern is `pushstring KEY ; pushstring VALUE`
            // back-to-back — value immediately follows.
            if s_idx == key_idx && j < bc.len() && bc[j] == OP_PUSHSTRING {
                let mut k = j + 1;
                let v_idx = read_u30_at(bc, &mut k)?;
                return abc.strings.get(v_idx as usize).map(|s| s.to_string());
            }
            i = j;
            continue;
        }
        i += 1;
        skip_opcode_operands(op, bc, &mut i);
    }
    None
}

/// Walk a method body to find `pushstring "characters" ; ... ; newarray N`
/// and collect every `callproperty` / `callpropvoid` whose multiname
/// starts with `get` between those two points. Each collected name is
/// folded through `derive_id_from_bundle_method_name` and paired with
/// its raw method name.
fn scan_register_characters_array(body: &MethodBody, abc: &AbcFile) -> Vec<(String, String)> {
    let Some(key_idx) = abc.strings.iter()
        .position(|s| &**s == "characters").map(|i| i as u32)
    else { return Vec::new() };

    let bc = &body.bytecode;
    let mut i = 0;
    while i < bc.len() {
        let op = bc[i];
        if op == OP_PUSHSTRING {
            let mut j = i + 1;
            let Some(s_idx) = read_u30_at(bc, &mut j) else { return Vec::new() };
            if s_idx == key_idx {
                // Walk forward, collecting getters, until newarray.
                let mut k = j;
                let mut out: Vec<(String, String)> = Vec::new();
                while k < bc.len() {
                    let op2 = bc[k];
                    if op2 == OP_NEWARRAY {
                        return out;
                    }
                    if op2 == OP_CALLPROPERTY || op2 == OP_CALLPROPVOID {
                        let mut m = k + 1;
                        let Some(mn_idx) = read_u30_at(bc, &mut m) else { break };
                        // arg count — read + discard
                        read_u30_at(bc, &mut m);
                        let name = abc.multinames.get(mn_idx as usize)
                            .map(|mn| mn.name.to_string()).unwrap_or_default();
                        if let Some(id) = derive_id_from_bundle_method_name(&name) {
                            out.push((id, name));
                        }
                        k = m;
                        continue;
                    }
                    k += 1;
                    skip_opcode_operands(op2, bc, &mut k);
                }
                return out;
            }
            i = j;
            continue;
        }
        i += 1;
        skip_opcode_operands(op, bc, &mut i);
    }
    Vec::new()
}

/// Advance `i` past the operand bytes of the opcode whose byte was
/// already consumed (so on entry, `i` points one past the opcode
/// byte). Mirrors the dispatch in `scan_method`, kept thin enough to
/// be the truth-source for the small dedicated walkers (constructor
/// + normalStats_id) without dragging in the full AVM2 stack
/// interpreter.
fn skip_opcode_operands(op: u8, bc: &[u8], i: &mut usize) {
    match op {
        OP_PUSHDOUBLE | OP_PUSHSTRING | OP_PUSHINT | OP_PUSHUINT |
        OP_COERCE     | OP_GETLEX     | OP_FINDPROPSTRICT | OP_FINDPROP |
        OP_GETPROPERTY| OP_INITPROPERTY| OP_SETPROPERTY |
        OP_GETLOCAL   | OP_SETLOCAL   | OP_NEWARRAY   | OP_NEWOBJECT => {
            read_u30_at(bc, i);
        }
        OP_PUSHBYTE   => { if *i < bc.len() { *i += 1; } }
        OP_PUSHSHORT  => { read_u30_at(bc, i); }
        OP_CALLPROPERTY | OP_CALLPROPVOID | OP_CONSTRUCTPROP => {
            read_u30_at(bc, i); read_u30_at(bc, i);
        }
        OP_CONSTRUCT => { read_u30_at(bc, i); }
        OP_JUMP | OP_IFTRUE | OP_IFFALSE | OP_IFEQ | OP_IFNE | OP_IFLT |
        OP_IFLE | OP_IFGT  | OP_IFGE    | OP_IFSTRICTEQ | OP_IFSTRICTNE
            if *i + 3 <= bc.len() => { *i += 3; }
        _ => {}
    }
}

/// Scan a method body for the literal string value of `normalStats_id`.
/// The AVM2 pattern emitted by SSF2's source is:
///     pushstring "normalStats_id"
///     pushstring "<value>"
///     setproperty/initproperty
/// We find the first occurrence of `pushstring "normalStats_id"` and
/// return the very next pushed string. Used by `extract_character` to
/// detect transformation/alternate-form bundles (their `normalStats_id`
/// is the parent character's id, not their own derived id).
pub fn extract_normal_stats_id(body: &MethodBody, abc: &AbcFile) -> Option<String> {
    extract_string_stat(body, abc, "normalStats_id")
}

/// The STRING a character's own stat object gives for `key`.
///
/// The stats a character declares are not all numbers: `thumbnail` names the clip the damage
/// counter shows, `seriesIcon` names its emblem, `linkage_id` names its art. Those are the source
/// SAYING which of its clips is which, so nothing downstream has to recognise a name.
///
/// The literal is built key-then-value, so the value is the next string pushed after the key.
pub fn extract_string_stat(body: &MethodBody, abc: &AbcFile, key: &str) -> Option<String> {
    let needle_idx = abc.strings.iter().position(|s| &**s == key)? as u32;
    let bytes = &body.bytecode;
    let mut i = 0;
    while i < bytes.len() {
        let op = bytes[i];
        if op == OP_PUSHSTRING {
            let mut j = i + 1;
            if let Some(idx) = read_u30_at(bytes, &mut j) {
                if idx == needle_idx {
                    // Walk forward to the next OP_PUSHSTRING and take its value.
                    let mut k = j;
                    while k < bytes.len() {
                        if bytes[k] == OP_PUSHSTRING {
                            let mut m = k + 1;
                            let v = read_u30_at(bytes, &mut m)?;
                            return abc.strings.get(v as usize).map(|s| s.to_string());
                        }
                        k += 1;
                    }
                    return None;
                }
                i = j; continue;
            }
        }
        i += 1;
    }
    None
}

/// Extract the string value of a single object-literal field from a bundle
/// method body, using the same `pushstring <field-name>; … ; pushstring
/// <value>` pattern as `extract_normal_stats_id`. Returns None if the field
/// name isn't in the string pool, or isn't pushed in this body.
fn extract_field_string(body: &MethodBody, abc: &AbcFile, field: &str) -> Option<String> {
    let needle_idx = abc.strings.iter().position(|s| &**s == field)? as u32;
    let bytes = &body.bytecode;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == OP_PUSHSTRING {
            let mut j = i + 1;
            if let Some(idx) = read_u30_at(bytes, &mut j) {
                if idx == needle_idx {
                    // Value is the next string pushed after the field name.
                    let mut k = j;
                    while k < bytes.len() {
                        if bytes[k] == OP_PUSHSTRING {
                            let mut m = k + 1;
                            let v = read_u30_at(bytes, &mut m)?;
                            return abc.strings.get(v as usize).map(|s| s.to_string());
                        }
                        k += 1;
                    }
                    return None;
                }
                i = j; continue;
            }
        }
        i += 1;
    }
    None
}

/// Collect a contiguous 1-based run of indexed string fields from a bundle
/// body: `{prefix}1{suffix}`, `{prefix}2{suffix}`, … stopping at the first
/// index whose field isn't present in this body. Backs SSF2's
/// `attackSound{N}_id` / `attackVoice{N}_id` sound tables (the targets of
/// `playAttackSound(N)` / `playVoiceSound(N)`, both 1-based).
pub fn extract_indexed_string_fields(
    body: &MethodBody, abc: &AbcFile, prefix: &str, suffix: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut n = 1usize;
    loop {
        let field = format!("{prefix}{n}{suffix}");
        match extract_field_string(body, abc, &field) {
            Some(v) => { out.push(v); n += 1; }
            None => break,
        }
    }
    out
}

/// Symbolic value pushed onto the simulated AVM2 stack by `scan_method`
/// and inspected by `AbcVisitor` implementations to recognise object
/// literals (`newobject`) that carry SSF2 attack / projectile / stat /
/// costume data.
#[derive(Debug, Clone)]
pub(crate) enum StackVal {
    Str(String),
    Num(f64),
    /// `true` / `false` as the source wrote it. A declaration like `ghost: true` says how the
    /// thing behaves, so which one it is has to survive the walk.
    Bool(bool),
    Null,
    /// A parsed object literal from newobject
    Obj(BTreeMap<String, StackVal>),
    /// A parsed array from newarray
    Arr(Vec<StackVal>),
    /// A visitor-defined marker carried on the simulated stack (e.g. a plane
    /// accessor's result, or a `getlex` class name). The scanner never creates
    /// these itself — only the property/lex hooks do — so default visitors
    /// (which don't override the hooks) never see them.
    Tag(String),
    Unknown,
}

// ─── Shared AVM2 stack-sim scanner ───────────────────────────────────────────
//
// SSF2 keeps several flavours of structured data inside AVM2 bytecode:
//   1. `getAttackStats()` — one top-level newobject keyed by move name,
//       each value is a hitbox object literal.
//   2. `getProjectileStats()` — one top-level newobject keyed by
//      projectile name, each value is a physics + attackBoxes object.
//   3. `getOwnStats()` / fallback — one newobject with character physics keys.
//   4. `misc.ssf :: getCostumeData()` (and per-character costume methods)
//      — one or more newobjects matching one of two palette-table shapes.
//
// Each used to live in its own bespoke stack interpreter (~150 lines × 4 = ~600
// lines of nearly-identical opcode handlers). They've now been unified
// behind `scan_method` + the `AbcVisitor` trait. Each former extractor
// is a thin wrapper that builds a visitor, calls `scan_method`, and
// reads the visitor's accumulator. See the visitors immediately below.
//
// We did NOT consolidate `extract_ssf2_stats` — it's a fundamentally
// different algorithm (a linear pushstring-followed-by-numeric-push scan
// that doesn't model the stack at all). Folding it into the visitor
// model would require either complicating the scanner or weakening the
// extractor's recognition.

/// What `AbcVisitor::on_newobject` wants the scanner to do with the
/// freshly-built object literal.
pub(crate) enum NewObjectAction {
    /// Push StackVal::Obj(obj) onto the stack — the default for visitors
    /// that may need to look at this object again as a sub-value in a
    /// later newobject.
    PushObj(BTreeMap<String, StackVal>),
    /// Push some other StackVal (typically StackVal::Unknown) instead
    /// of the obj. Used by visitors that recorded the obj as a
    /// top-level extraction target and don't need its identity on the
    /// Halt scanning entirely. Used by single-shot extractors that have
    /// already found what they came for (e.g. CharacterStats).
    Stop,
}

/// Hook surface for `scan_method`. Default impls match the
/// attack/projectile/stats simulators' common semantics; the costume
/// visitor overrides a couple of behaviours where its parsing differs.
pub(crate) trait AbcVisitor {
    /// Called when a `newobject(N)` opcode completes. The scanner has
    /// already drained 2N items from the stack and built the obj map;
    /// the visitor decides what to record, then returns an action
    /// telling the scanner what to push (or whether to stop).
    fn on_newobject(
        &mut self,
        obj: BTreeMap<String, StackVal>,
        current_char: &Option<String>,
    ) -> NewObjectAction {
        let _ = current_char;
        NewObjectAction::PushObj(obj)
    }

    /// Called when a `newarray(N)` opcode completes with `items` drained
    /// in declared order. Default: drop the items and push
    /// StackVal::Unknown — matches the attack/projectile/stats sims,
    /// which never inspect array contents. The costume visitor returns
    /// `StackVal::Arr(items)` so it can read color literals back.
    fn on_newarray(&mut self, items: Vec<StackVal>) -> StackVal {
        let _ = items;
        StackVal::Unknown
    }

    /// If true, the scanner uses the costume-style getproperty / set-
    /// property handling: track a runtime-key or static-name match
    /// against `current_char`, and on STATIC getproperty preserve the
    /// receiver on the stack (so `_loc1_.mario.push(...)` still binds
    /// the right receiver). False (default) = pop receiver and push
    /// Unknown, matching the attack/projectile sims' post-§3.5-fix
    /// behaviour.
    fn costume_getproperty_semantics(&self) -> bool { false }

    // ── Property/lex hooks (for the stage plane-map + actor visitors) ──
    // All default to `None`, meaning "scanner keeps its existing behavior"
    // (push Unknown). So the attack/projectile/stats/costume visitors, which
    // don't override these, are byte-for-byte unaffected. A stage visitor
    // returns `Some(StackVal::Tag(...))` to carry a marker (a plane, a class
    // name, a spawned-actor handle) forward on the simulated stack.

    /// `getlex <Name>` / `findpropstrict <Name>`: the resolved name. Return a
    /// marker to put on the stack (e.g. the class name for a following call).
    fn on_getlex(&mut self, _name: &str) -> Option<StackVal> { None }
    /// `callproperty/callpropvoid <method>(args)` on `receiver` (already drained
    /// from the stack: receiver + args). For callproperty the return is pushed
    /// (Unknown if None); for callpropvoid it's only a side-effect hook.
    fn on_callproperty(&mut self, _method: &str, _args: &[StackVal], _receiver: &StackVal) -> Option<StackVal> { None }
    /// `constructprop <Class>(args)`. Return a marker for the constructed value.
    fn on_constructprop(&mut self, _class: &str, _args: &[StackVal]) -> Option<StackVal> { None }
    /// `getproperty <prop>` on `receiver`. Return a marker to propagate the
    /// receiver's tag through a chained access (e.g. `getBackground().clip`).
    fn on_getproperty(&mut self, _prop: &str, _receiver: &StackVal) -> Option<StackVal> { None }
    /// Opt in to locals tracking: `setlocal N` stores the stack top into a
    /// locals array and `getlocal N` reloads it, so a visitor can follow a
    /// value stashed in a temp (e.g. a spawned actor reloaded for `setX`).
    /// Default false = `getlocal` pushes Unknown, exactly as before.
    fn track_locals(&self) -> bool { false }
}

/// One pass over a method body. Built-in handlers cover every opcode
/// every former simulator handled. The visitor's hooks decide what to
/// record on newobject / newarray and (optionally) how to treat
/// getproperty — see the trait above.
pub(crate) fn scan_method<V: AbcVisitor>(bytecode: &[u8], abc: &AbcFile, visitor: &mut V) {
    let mut stack: Vec<StackVal> = Vec::new();
    let mut current_char: Option<String> = None;
    let mut i = 0;
    let costume_mode = visitor.costume_getproperty_semantics();
    let track_locals = visitor.track_locals();
    let mut locals: Vec<StackVal> = Vec::new();
    fn set_local(locals: &mut Vec<StackVal>, n: usize, v: StackVal) {
        if n >= locals.len() { locals.resize(n + 1, StackVal::Unknown); }
        locals[n] = v;
    }

    while i < bytecode.len() {
        let op = bytecode[i];
        i += 1;
        match op {
            // ── Constant pushes ─────────────────────────────────────────
            OP_PUSHSTRING => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    let s = abc.strings.get(idx as usize).map(|s| s.to_string()).unwrap_or_default();
                    stack.push(StackVal::Str(s));
                }
            }
            OP_PUSHDOUBLE => {
                if let Some(idx) = read_u30_at(bytecode, &mut i) {
                    stack.push(StackVal::Num(abc.doubles.get(idx as usize).copied().unwrap_or(0.0)));
                }
            }
            OP_PUSHBYTE => {
                if i < bytecode.len() {
                    stack.push(StackVal::Num(bytecode[i] as i8 as f64));
                    i += 1;
                }
            }
            OP_PUSHSHORT => {
                if let Some(v) = read_u30_at(bytecode, &mut i) {
                    stack.push(StackVal::Num(v as i16 as f64));
                }
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
            OP_PUSHTRUE | OP_PUSHFALSE => stack.push(StackVal::Bool(op == OP_PUSHTRUE)),
            OP_PUSHNULL | OP_PUSHNAN => stack.push(StackVal::Null),

            // ── newobject (the only visitor-driven push) ────────────────
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
                    match visitor.on_newobject(obj, &current_char) {
                        NewObjectAction::PushObj(o)  => stack.push(StackVal::Obj(o)),
                        NewObjectAction::Stop        => return,
                    }
                }
            }

            // ── newarray (visitor-overridable) ──────────────────────────
            OP_NEWARRAY => {
                if let Some(count) = read_u30_at(bytecode, &mut i) {
                    let count = count as usize;
                    let drain_start = stack.len().saturating_sub(count);
                    let items: Vec<StackVal> = stack.drain(drain_start..).collect();
                    stack.push(visitor.on_newarray(items));
                }
            }

            // ── Calls & construction (drain args+receiver, hook, push) ──
            OP_CALLPROPERTY | OP_CALLPROPVOID => {
                let mn_idx = read_u30_at(bytecode, &mut i).unwrap_or(0);
                let argc = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
                let method = abc.multinames.get(mn_idx as usize).map(|m| m.name.to_string()).unwrap_or_default();
                let drain = stack.len().min(argc + 1);
                let drained: Vec<StackVal> = stack.drain(stack.len() - drain..).collect();
                let (receiver, args) = drained.split_first()
                    .map(|(r, a)| (r.clone(), a.to_vec()))
                    .unwrap_or((StackVal::Null, Vec::new()));
                let hooked = visitor.on_callproperty(&method, &args, &receiver);
                if op == OP_CALLPROPERTY { stack.push(hooked.unwrap_or(StackVal::Unknown)); }
            }
            OP_CONSTRUCTPROP => {
                let mn_idx = read_u30_at(bytecode, &mut i).unwrap_or(0);
                let argc = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
                let class = abc.multinames.get(mn_idx as usize).map(|m| m.name.to_string()).unwrap_or_default();
                let drain = stack.len().min(argc + 1);
                let drained: Vec<StackVal> = stack.drain(stack.len() - drain..).collect();
                let args: Vec<StackVal> = drained.into_iter().skip(1).collect();
                let hooked = visitor.on_constructprop(&class, &args);
                stack.push(hooked.unwrap_or(StackVal::Unknown));
            }
            OP_CONSTRUCT => {
                let argc = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
                let drain = stack.len().min(argc + 1);
                stack.drain(stack.len() - drain..);
                stack.push(StackVal::Unknown);
            }

            // ── Property reads/writes ───────────────────────────────────
            // setproperty/initproperty are the same in every former sim:
            // drain 2 items. The costume sim ALSO inspects the multiname
            // for a static char name. Track that opportunistically.
            OP_SETPROPERTY | OP_INITPROPERTY => {
                let mn_idx = read_u30_at(bytecode, &mut i).unwrap_or(0);
                if costume_mode {
                    let mn = abc.multinames.get(mn_idx as usize);
                    let is_runtime = mn.map(|m| m.kind == 0x1B || m.kind == 0x1C).unwrap_or(false);
                    let static_name = mn.and_then(|m| if m.name.is_empty() { None } else { Some(m.name.to_string()) });
                    if is_runtime {
                        let _val = stack.pop();
                        let key = stack.pop();
                        stack.pop(); // object
                        if let Some(StackVal::Str(s)) = &key {
                            if looks_like_char_name(s) { current_char = Some(s.clone()); }
                        }
                    } else {
                        stack.pop(); stack.pop();
                        if let Some(name) = static_name {
                            if looks_like_char_name(&name) { current_char = Some(name); }
                        }
                    }
                } else if stack.len() >= 2 {
                    stack.truncate(stack.len() - 2);
                }
            }

            // getproperty splits cleanly into two modes:
            //  - default: pop receiver, push Unknown (post-§3.5 fix)
            //  - costume: track current_char from multiname, and on
            //    STATIC getproperty preserve the receiver so chained
            //    `_loc1_.mario.push(...)` still resolves.
            OP_GETPROPERTY => {
                let mn_idx = read_u30_at(bytecode, &mut i).unwrap_or(0);
                if costume_mode {
                    let mn = abc.multinames.get(mn_idx as usize);
                    let is_runtime = mn.map(|m| m.kind == 0x1B || m.kind == 0x1C).unwrap_or(false);
                    let static_name = mn.and_then(|m| if m.name.is_empty() { None } else { Some(m.name.to_string()) });
                    if is_runtime {
                        let key = stack.pop();
                        stack.pop(); // receiver
                        if let Some(StackVal::Str(s)) = &key {
                            if looks_like_char_name(s) { current_char = Some(s.clone()); }
                        }
                        stack.push(StackVal::Null);
                    } else {
                        let top = stack.pop().unwrap_or(StackVal::Null);
                        if let Some(name) = static_name {
                            if looks_like_char_name(&name) { current_char = Some(name); }
                            stack.push(top); // preserve receiver for chained access
                        } else {
                            stack.push(StackVal::Null);
                        }
                    }
                } else {
                    let mn = abc.multinames.get(mn_idx as usize);
                    let name = mn.map(|m| m.name.to_string()).unwrap_or_default();
                    // a stage visitor (track_locals) wants an accurate stack: a RUNTIME multiname
                    // (RTQNameL 0x1B / MultinameL 0x1C, e.g. `arr[0]`) takes its index off the stack
                    // BELOW the receiver, so pop it first or a chain desyncs (`getCameraBackgrounds()[0].mc`).
                    if track_locals && mn.map(|m| m.kind == 0x1B || m.kind == 0x1C).unwrap_or(false) {
                        stack.pop();
                    }
                    let receiver = stack.pop().unwrap_or(StackVal::Null);
                    stack.push(visitor.on_getproperty(&name, &receiver).unwrap_or(StackVal::Unknown));
                }
            }

            OP_FINDPROPSTRICT | OP_FINDPROP | OP_GETLEX => {
                let mn_idx = read_u30_at(bytecode, &mut i).unwrap_or(0);
                let name = abc.multinames.get(mn_idx as usize).map(|m| m.name.to_string()).unwrap_or_default();
                stack.push(visitor.on_getlex(&name).unwrap_or(StackVal::Unknown));
            }

            // ── Coerce / convert — operand-bearing but mostly no-ops ────
            OP_COERCE | OP_COERCE_A | OP_CONVERT_D | OP_CONVERT_I => {
                if op == OP_COERCE { read_u30_at(bytecode, &mut i); }
            }

            // ── Plumbing ───────────────────────────────────────────────
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
            OP_GETLOCAL0 | OP_GETLOCAL1 | OP_GETLOCAL2 | OP_GETLOCAL3 => {
                let n = (op - OP_GETLOCAL0) as usize;
                stack.push(if track_locals { locals.get(n).cloned().unwrap_or(StackVal::Unknown) } else { StackVal::Unknown });
            }
            OP_GETLOCAL => {
                let n = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
                stack.push(if track_locals { locals.get(n).cloned().unwrap_or(StackVal::Unknown) } else { StackVal::Unknown });
            }
            OP_SETLOCAL0 | OP_SETLOCAL1 | OP_SETLOCAL2 | OP_SETLOCAL3 => {
                let v = stack.pop().unwrap_or(StackVal::Null);
                if track_locals { set_local(&mut locals, (op - OP_SETLOCAL0) as usize, v); }
            }
            OP_SETLOCAL => {
                let n = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
                let v = stack.pop().unwrap_or(StackVal::Null);
                if track_locals { set_local(&mut locals, n, v); }
            }
            OP_RETURNVALUE => { stack.pop(); }
            OP_RETURNVOID => {}
            OP_JUMP | OP_IFTRUE | OP_IFFALSE | OP_IFEQ | OP_IFNE | OP_IFLT |
            OP_IFLE | OP_IFGT | OP_IFGE | OP_IFNLT | OP_IFNLE | OP_IFNGT | OP_IFNGE |
            OP_IFSTRICTEQ | OP_IFSTRICTNE => {
                if i + 3 <= bytecode.len() { i += 3; }
                if op != OP_JUMP { stack.pop(); }
            }
            _ => {}
        }
        // Bound the stack so a malformed body can't blow memory.
        if stack.len() > 512 { stack.drain(0..256); }
    }
}

/// Helper used by the costume visitor + the costume-mode getproperty
/// branch of `scan_method`. Lowercase alphabetic-only strings between 3
/// and 24 chars long are accepted as character ids.
fn looks_like_char_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric())
        && s.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false)
        && s.len() >= 3 && s.len() <= 24
}

/// Visitor for `getAttackStats()`-style bodies. A top-level newobject
/// whose keys are SSF2 attack-move names (`a`, `b_air`, `a_air_forward`,
/// …) is the attack table; each value carries the hitbox data.
struct AttackVisitor {
    result: BTreeMap<String, AttackData>,
}

impl AbcVisitor for AttackVisitor {
    fn on_newobject(
        &mut self,
        obj: BTreeMap<String, StackVal>,
        _current_char: &Option<String>,
    ) -> NewObjectAction {
        let attack_keys_found: Vec<_> = obj.keys()
            .filter(|k| is_attack_name(k))
            .cloned().collect();
        for move_name in &attack_keys_found {
            if let Some(val) = obj.get(move_name) {
                let hitboxes = extract_hitboxes_from_val(val);
                let mut stats = BTreeMap::new();
                if let StackVal::Obj(o) = val {
                    for (k, v) in o {
                        match v {
                            StackVal::Num(n) => { stats.insert(k.clone(), *n); }
                            StackVal::Bool(b) => { stats.insert(k.clone(), if *b { 1.0 } else { 0.0 }); }
                            _ => {}
                        }
                    }
                }
                if !hitboxes.is_empty() || !stats.is_empty() {
                    self.result.insert(normalize_attack_name(move_name), AttackData { hitboxes, stats });
                }
            }
        }
        NewObjectAction::PushObj(obj)
    }
}

pub(crate) fn extract_attack_objects(bytecode: &[u8], abc: &AbcFile) -> BTreeMap<String, AttackData> {
    let mut v = AttackVisitor { result: BTreeMap::new() };
    scan_method(bytecode, abc, &mut v);
    v.result
}

/// Captures the single getAttackStats newobject that yields the MOST hitboxes — for a HAZARD,
/// whose getAttackStats returns the attack object DIRECTLY (`{ attackBoxes: {...} }`), not the
/// character pattern of an object keyed by move name. (The nested `attackBoxes` map fires its own
/// newobject too; "most hitboxes" picks the complete one and dedups the partial.)
/// The hit a class declares, kept UNDER THE ANIMATION IT BELONGS TO.
///
/// A class states its boxes per animation (`{ fall: { attackBoxes: { … } } }`), which is the source
/// saying when the thing hurts: bowser's castle's thwomp hits on its way down and at no other time.
/// Flattened to a list of boxes the label is lost, and the port put the hit on every animation the
/// object has -- so the thwomp hurt anyone who touched it while it sat at the top waiting.
pub(crate) fn extract_attack_stats_by_anim(abc: &AbcFile, class_name: &str)
    -> BTreeMap<String, Vec<BTreeMap<String, f64>>>
{
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return BTreeMap::new() };
    for want in ["getAttackStats", "getOwnStats"] {
        let Some(t) = class.instance_methods.iter().find(|t| &*t.name == want) else { continue };
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
        let mut v = HazardAttackByAnimVisitor { best: BTreeMap::new(), rates: BTreeMap::new() };
        scan_method(&body.bytecode, abc, &mut v);
        if !v.best.is_empty() { return v.best; }
    }
    BTreeMap::new()
}

/// How often the class says the hit comes round again, per animation (`refreshRate`). A hazard a
/// fighter stands in re-hits on that cadence, and reading it off the class is the difference
/// between the source's own rhythm and a number the port chose.
pub(crate) fn extract_attack_rates_by_anim(abc: &AbcFile, class_name: &str) -> BTreeMap<String, f64> {
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return BTreeMap::new() };
    for want in ["getAttackStats", "getOwnStats"] {
        let Some(t) = class.instance_methods.iter().find(|t| &*t.name == want) else { continue };
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
        let mut v = HazardAttackByAnimVisitor { best: BTreeMap::new(), rates: BTreeMap::new() };
        scan_method(&body.bytecode, abc, &mut v);
        if !v.rates.is_empty() { return v.rates; }
    }
    BTreeMap::new()
}

struct HazardAttackByAnimVisitor {
    best: BTreeMap<String, Vec<BTreeMap<String, f64>>>,
    rates: BTreeMap<String, f64>,
}
impl AbcVisitor for HazardAttackByAnimVisitor {
    fn on_newobject(&mut self, obj: BTreeMap<String, StackVal>, _c: &Option<String>) -> NewObjectAction {
        // The object keyed by ANIMATION is the one whose entries hold an `attackBoxes` map. Asking
        // only "does this entry contain boxes" matches the boxes' own container too, and that one
        // has more entries, so the labels came back as the names of the boxes.
        let mut found: BTreeMap<String, Vec<BTreeMap<String, f64>>> = BTreeMap::new();
        let mut rates: BTreeMap<String, f64> = BTreeMap::new();
        for (k, v) in &obj {
            let StackVal::Obj(inner) = v else { continue };
            if !inner.contains_key("attackBoxes") { continue }
            let hbs = extract_hitboxes_from_val(v);
            if hbs.is_empty() { continue }
            if let Some(StackVal::Num(r)) = inner.get("refreshRate") { rates.insert(k.clone(), *r); }
            found.insert(k.clone(), hbs);
        }
        if found.len() > self.best.len() { self.best = found; self.rates = rates; }
        NewObjectAction::PushObj(obj)
    }
}

struct HazardAttackVisitor {
    best: Vec<BTreeMap<String, f64>>,
}
impl AbcVisitor for HazardAttackVisitor {
    fn on_newobject(&mut self, obj: BTreeMap<String, StackVal>, _c: &Option<String>) -> NewObjectAction {
        let hbs = extract_hitboxes_from_val(&StackVal::Obj(obj.clone()));
        if hbs.len() > self.best.len() {
            self.best = hbs;
        }
        NewObjectAction::PushObj(obj)
    }
}

/// The behavior values a hazard class's `update()`/`initialize()` actually use, recovered by
/// stepping the class instead of hardcoding them: the camera-shake amplitude (`shake`/`shakeCamera`
/// argument), the rise speed (`setYSpeed`), the fall gravity (`updateEnemyStats({gravity})`), the
/// self-platform box (`createSelfPlatform`), the dust effect (`attachEffect`), and the sounds
/// (`playSound`). The FM CGO script is driven by these so the hazard's behavior comes from its own
/// code, not a per-hazard template of constants.
#[derive(Default, Clone, Debug)]
pub struct EnemyBehavior {
    pub shake: Option<f64>,
    pub rise_yspeed: Option<f64>,
    pub fall_gravity: Option<f64>,
    pub self_platform: Option<(f64, f64, f64, f64)>,
    pub dust: Option<(String, f64, f64)>,
    pub sounds: Vec<String>,
    /// The class adds itself as a camera target (`addToCamera`) — the FM port mirrors it with
    /// `match.getCamera().addTarget(self)` while the hazard is engaged.
    pub camera_target: bool,
}
struct BehaviorVisitor {
    b: EnemyBehavior,
}
impl AbcVisitor for BehaviorVisitor {
    fn on_callproperty(&mut self, method: &str, args: &[StackVal], _r: &StackVal) -> Option<StackVal> {
        let num = |i: usize| if let Some(StackVal::Num(n)) = args.get(i) { Some(*n) } else { None };
        match method {
            "shake" | "shakeCamera" => { if let Some(n) = num(0) { self.b.shake = Some(n); } }
            "setYSpeed" => { if let Some(n) = num(0) { if n < 0.0 { self.b.rise_yspeed = Some(n); } } }
            "updateEnemyStats" => {
                if let Some(StackVal::Obj(o)) = args.first() {
                    if let Some(StackVal::Num(g)) = o.get("gravity") { if *g > 0.0 { self.b.fall_gravity = Some(*g); } }
                }
            }
            "createSelfPlatform" => {
                if let (Some(a), Some(b), Some(c), Some(d)) = (num(0), num(1), num(2), num(3)) {
                    self.b.self_platform = Some((a, b, c, d));
                }
            }
            "attachEffect" => {
                if let Some(StackVal::Str(name)) = args.first() {
                    let (mut sx, mut sy) = (1.0, 1.0);
                    if let Some(StackVal::Obj(o)) = args.get(1) {
                        if let Some(StackVal::Num(v)) = o.get("scaleX") { sx = *v; }
                        if let Some(StackVal::Num(v)) = o.get("scaleY") { sy = *v; }
                    }
                    self.b.dust = Some((name.clone(), sx, sy));
                }
            }
            "playSound" => { if let Some(StackVal::Str(s)) = args.first() { if !self.b.sounds.contains(s) { self.b.sounds.push(s.clone()); } } }
            "addToCamera" => { self.b.camera_target = true; }
            _ => {}
        }
        None
    }
}
/// A stage's particle WEATHER: many copies of one small clip drifting across the camera.
///
/// Not a hazard and not backdrop art. It is neither placed on the timeline (so the art walk never
/// sees it) nor damaging (so the hazard path ignores it), which is why a stage can convert with
/// every layer correct and still be missing the thing filling the air.
#[derive(Clone, Debug, PartialEq)]
pub struct StageWeather {
    /// Linkage of the particle clip (bowserscastle's is `ember`).
    pub art: String,
    /// How many exist at once.
    pub count: u32,
    /// The area they are scattered over and wrap within, in SSF2 units.
    pub width: f64,
    pub height: f64,
    /// How the source moves one particle per frame, when its class could be read.
    pub motion: Option<WeatherMotion>,
    /// The stage field this weather is kept in, when it keeps one. A stage can have several
    /// (crateria falls rain and acid) and only the field this port built is the one whose strength
    /// means anything here.
    pub field: String,
}

/// Find a stage's weather by stepping its own `initialize`.
///
/// The shape in the bytecode is: push the particle CLASS, construct a weather object from it, then
/// hand that object the area and the count. Keyed on that shape rather than on any stage or class
/// name, so a second stage doing the same thing is found the same way.
pub fn extract_stage_weather(abc: &AbcFile, stage_class: &str) -> Vec<StageWeather> {
    let Some(class) = abc.classes.iter().find(|c| c.name == stage_class)
        .or_else(|| abc.classes.iter().find(|c| c.name.eq_ignore_ascii_case(stage_class))) else { return Vec::new() };
    // Every method, not just `initialize`: a stage builds its weather where it likes (smashville
    // does it in `syncStage`, clocktown behind a state machine).
    for t in class.instance_methods.iter() {
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
        let w = weather_in_body(abc, &body.bytecode);
        if !w.is_empty() { return w }
    }
    Vec::new()
}

/// The weather construction inside one method body, keyed on the ENGINE CALL that defines it.
///
/// A stage hands its weather's container to the engine's weather layer:
///
/// ```text
///   constructprop <WeatherClass>(<particle class>, …)   // built from one small clip
///   initproperty  <field>                               // kept on the stage
///   … getWeatherMC()                                    // the engine's weather layer
///   getproperty <field>; callproperty getContainer; callpropvoid addChild
/// ```
///
/// `getWeatherMC` is the signature. It is in every package's api layer but only weather code CALLS
/// it, so "the object whose container goes into the weather layer" identifies the weather exactly,
/// on every stage that has any, without knowing a single class name. Keying on a name containing
/// "weather" instead finds two of the eight stages in the corpus that call it.
fn weather_in_body(abc: &AbcFile, bc: &[u8]) -> Vec<StageWeather> {
    (|| -> Option<Vec<StageWeather>> {
    // pass 1: field -> (particle art, the constructor arguments)
    // Arguments are kept POSITIONALLY: a class or string argument holds its slot as `None` so the
    // numbers still line up with the constructor's parameters. Collapsing to "the numbers seen"
    // shifts every one of them by however many non-numeric arguments came first.
    type CtorArgs = (Vec<Option<f64>>, Vec<Option<String>>);
    let mut built: Vec<(String, String, Option<String>, CtorArgs)> = Vec::new();
    let (mut i, mut last_lex) = (0usize, None::<String>);
    let mut recent: Vec<(Option<f64>, Option<String>)> = Vec::new();
    let mut pending: Option<(String, Option<String>, CtorArgs)> = None;
    while i < bc.len() {
        let op = bc[i];
        let at = i;
        i += 1;
        match op {
            OP_GETLEX => {
                let mn = read_u30_at(bc, &mut i)?;
                last_lex = abc.multinames.get(mn as usize).map(|m| m.name.to_string());
                recent.push((None, None));
            }
            OP_PUSHSTRING => {
                let idx = read_u30_at(bc, &mut i)?;
                recent.push((None, abc.strings.get(idx as usize).map(|s| s.to_string())));
            }
            OP_CONSTRUCTPROP => {
                let mn = read_u30_at(bc, &mut i)?;
                let argc = read_u30_at(bc, &mut i)? as usize;
                let cname = abc.multinames.get(mn as usize).map(|m| m.name.to_string()).unwrap_or_default();
                // The arguments were pushed BEFORE the call, so they are the tail of what we have
                // seen. The particle class is the most recent name; the numbers among the args are
                // whatever the stage chose to override.
                let take = recent.len().min(argc);
                let tail = &recent[recent.len() - take..];
                // Reading the tail is only right when every argument was seen. If any push went
                // untracked the tail is SHORT, and indexing it from zero slides every argument
                // into the slot of the one before it, so a field's width arrives as its drift.
                // Pad the front instead: an argument we could not read stays unknown in place.
                let mut slots = vec![(None, None); argc - take];
                slots.extend(tail.iter().cloned());
                let args = (slots.iter().map(|a| a.0).collect(), slots.iter().map(|a| a.1.clone()).collect());
                recent.clear();
                pending = Some((cname, last_lex.clone(), args));
            }
            OP_PUSHDOUBLE => {
                let idx = read_u30_at(bc, &mut i)?;
                recent.push((abc.doubles.get(idx as usize).copied(), None));
            }
            OP_PUSHINT => {
                let idx = read_u30_at(bc, &mut i)?;
                recent.push((abc.ints.get(idx as usize).map(|v| *v as f64), None));
            }
            OP_PUSHSHORT => {
                let v = read_u30_at(bc, &mut i)? as i32;
                recent.push((Some(v as f64), None));
            }
            OP_PUSHBYTE => {
                recent.push((Some(bc.get(i).map(|b| *b as i8 as f64)?), None));
                i += 1;
            }
            // an argument that is not a number or a string still HOLDS ITS SLOT
            OP_PUSHTRUE => recent.push((Some(1.0), None)),
            OP_PUSHFALSE => recent.push((Some(0.0), None)),
            OP_PUSHNULL | OP_PUSHNAN | OP_PUSHUNDEFINED => recent.push((None, None)),
            // the same value twice in a row is pushed once and duplicated
            OP_DUP => { let t = recent.last().cloned().unwrap_or((None, None)); recent.push(t); }
            OP_SETPROPERTY | OP_INITPROPERTY => {
                let mn = read_u30_at(bc, &mut i)?;
                let field = abc.multinames.get(mn as usize).map(|m| m.name.to_string()).unwrap_or_default();
                if let Some((cname, art, args)) = pending.take() { built.push((field, cname, art, args)); }
            }
            _ => { let mut j = at + 1; skip_opcode_operands(op, bc, &mut j); i = j; }
        }
    }
    if built.is_empty() { return None; }

    // pass 2: after the weather-layer call, the field whose container is added is the weather
    let (mut i, mut saw_layer, mut last_field) = (0usize, false, None::<String>);
    let mut weather_fields: Vec<String> = Vec::new();
    while i < bc.len() {
        let op = bc[i];
        let at = i;
        i += 1;
        match op {
            OP_GETPROPERTY => {
                let mn = read_u30_at(bc, &mut i)?;
                last_field = abc.multinames.get(mn as usize).map(|m| m.name.to_string());
            }
            OP_CALLPROPERTY | OP_CALLPROPVOID => {
                let mn = read_u30_at(bc, &mut i)?;
                let _argc = read_u30_at(bc, &mut i)?;
                let name = abc.multinames.get(mn as usize).map(|m| m.name.to_string()).unwrap_or_default();
                match name.as_str() {
                    "getWeatherMC" => saw_layer = true,
                    // EVERY one handed over, in the order the stage hands them. A stage can run
                    // more than one field at once -- crateria falls rain and acid, and holds the
                    // acid at nothing until its event -- and keeping only the first built the one
                    // the player sees while the other never existed.
                    "getContainer" if saw_layer => {
                        if let Some(f) = last_field.clone() {
                            if built.iter().any(|(bf, _, _, _)| *bf == f) && !weather_fields.contains(&f) {
                                weather_fields.push(f);
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => { let mut j = at + 1; skip_opcode_operands(op, bc, &mut j); i = j; }
        }
    }
    // A stage that builds one weather object and hands it over without going through a field still
    // reads as weather: the layer call is the evidence, the single construction is the object.
    // A stage that builds one weather object and hands it over without going through a field still
    // reads as weather: the layer call is the evidence, the single construction is the object.
    let chosen: Vec<(String, String, Option<String>, CtorArgs)> = if !weather_fields.is_empty() {
        weather_fields.iter()
            .filter_map(|f| built.iter().find(|(bf, _, _, _)| bf == f).cloned())
            .collect()
    } else if saw_layer && built.len() == 1 {
        built.clone()
    } else {
        Vec::new()
    };
    Some(chosen.into_iter().filter_map(|(field, cname, art, args)| {
        let art = art?;
        let (nums, strs) = args;
        let (width, height, count) = weather_numbers(abc, &cname, &nums);
        // a class that branches on a string it was handed (`"left"` / `"right"`) has one mover per
        // direction; the one named for the string it was given is the one that runs.
        let dir = strs.iter().flatten().next().cloned();
        let motion = weather_motion(abc, &cname, &nums, dir.as_deref());
        Some(StageWeather { art, width, height, count, motion, field })
    }).collect())
    })().unwrap_or_default()
}

/// The count and area a weather object is actually built with.
///
/// The stage may pass them or leave them out, so the values come from the CLASS: its constructor
/// says which parameter lands in which field (`getlocal_N` then a store), and its signature carries
/// the defaults for the ones the stage did not pass. bowserscastle passes only the particle class
/// and takes everything else from those defaults, so reading the call site alone finds nothing.
fn weather_numbers(abc: &AbcFile, class_name: &str, args: &[Option<f64>]) -> (f64, f64, u32) {
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return (0.0, 0.0, 0) };
    let Some(ctor) = abc.methods.get(class.constructor_idx as usize) else { return (0.0, 0.0, 0) };
    let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == class.constructor_idx) else { return (0.0, 0.0, 0) };

    // param index (1-based, as getlocal numbers them) -> the field it is stored in
    let mut param_field: Vec<(usize, String)> = Vec::new();
    let bc = &body.bytecode;
    let (mut i, mut last_local) = (0usize, None::<usize>);
    while i < bc.len() {
        let op = bc[i];
        let at = i;
        i += 1;
        match op {
            OP_GETLOCAL1 => last_local = Some(1),
            OP_GETLOCAL2 => last_local = Some(2),
            OP_GETLOCAL3 => last_local = Some(3),
            OP_GETLOCAL => { last_local = read_u30_at(bc, &mut i).map(|n| n as usize); }
            OP_SETPROPERTY | OP_INITPROPERTY => {
                let mn = read_u30_at(bc, &mut i).unwrap_or(0);
                let f = abc.multinames.get(mn as usize).map(|m| m.name.to_string()).unwrap_or_default();
                if let Some(n) = last_local.take() { param_field.push((n, f)); }
            }
            _ => { let mut j = at + 1; skip_opcode_operands(op, bc, &mut j); i = j; }
        }
    }

    // the value for a parameter: what the stage passed, else what the signature declares
    let pc = ctor.param_count as usize;
    let first_optional = pc.saturating_sub(ctor.optionals.len());
    let value_of = |p1: usize| -> Option<f64> {
        let idx = p1.checked_sub(1)?;              // getlocal is 1-based over params
        if let Some(Some(v)) = args.get(idx) { return Some(*v); }
        if idx >= first_optional { return ctor.optionals.get(idx - first_optional).copied().flatten(); }
        None
    };
    let find = |needle: &str| -> Option<f64> {
        param_field.iter()
            .find(|(_, f)| f.to_ascii_lowercase().contains(needle))
            .and_then(|(p, _)| value_of(*p))
    };
    let count = find("cnt").or_else(|| find("count")).or_else(|| find("num")).unwrap_or(0.0);
    // the area a class scatters over is named for the SIZE it is (`m_owidth`) or for the AXIS it
    // spans (`hArea`/`vArea`), and both spellings are in the corpus.
    let width = find("width").or_else(|| find("harea")).unwrap_or(0.0);
    let height = find("height").or_else(|| find("varea")).unwrap_or(0.0);
    (width, height, count.max(0.0) as u32)
}

/// What a stage puts behind the HAZARDS SWITCH, read out of the stage's own code.
///
/// Both engines let a match turn hazards off, and in SSF2 the switch is not something the engine
/// applies to a stage: the stage asks (`SSF2API.isHazardsOn()`) and decides for itself what to skip.
/// 47 stages in the corpus ask, and they do not all mean the same thing by it, so which parts of a
/// converted stage go quiet is a question only the source can answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HazardGate {
    /// Whether the stage asks at all. A stage that never asks runs everything regardless, and a
    /// port that gates anyway is quieter than the original.
    pub checked: bool,
    /// The classes the stage only ever spawns while the switch is on.
    pub gated_classes: Vec<String>,
}

/// Read the hazards switch out of a stage class.
///
/// The shape is a call to the switch followed by a branch, so the gated region is everything up to
/// that branch's target. What counts as gated is what the stage SPAWNS in there: a class it creates
/// only while the switch is on is a class the port should create only while the switch is on.
/// Keyed on that shape rather than on any stage or hazard name, so every stage that asks is read
/// the same way.
pub fn extract_hazard_gate(abc: &AbcFile, stage_class: &str) -> HazardGate {
    let mut gate = HazardGate::default();
    // A stage class is usually named for its id, but not always with the same capitalisation
    // (`dreamland` declares `Dreamland`), so fall back to a case-insensitive match rather than
    // reporting a stage that asks as one that doesn't.
    let Some(class) = abc.classes.iter().find(|c| c.name == stage_class)
        .or_else(|| abc.classes.iter().find(|c| c.name.eq_ignore_ascii_case(stage_class)))
    else { return gate };
    for m in ["update", "initialize"] {
        let Some(t) = class.instance_methods.iter().find(|t| &*t.name == m) else { continue };
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
        scan_hazard_gate(&body.bytecode, abc, &mut gate);
    }
    gate.gated_classes.sort();
    gate.gated_classes.dedup();
    gate
}

fn scan_hazard_gate(bc: &[u8], abc: &AbcFile, gate: &mut HazardGate) {
    // Regions of the body that only run with the switch on. A stage can ask more than once.
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut i = 0usize;
    while i < bc.len() {
        let op = bc[i];
        let op_at = i;
        i += 1;
        match op {
            OP_CALLPROPERTY => {
                let mn = read_u30_at(bc, &mut i).unwrap_or(0);
                let _argc = read_u30_at(bc, &mut i).unwrap_or(0);
                let name = abc.multinames.get(mn as usize).map(|m| m.name.to_string()).unwrap_or_default();
                if name != "isHazardsOn" { continue; }
                gate.checked = true;
                // `if (isHazardsOn())` compiles to the call then a jump-when-false, so the body is
                // everything between the branch and its target. Anything else (the answer stored in
                // a field first, an inverted test) still counts as asking, but its extent is not
                // one this shape can read, and a region guessed wrong is worse than none.
                if bc.get(i) == Some(&OP_IFFALSE) {
                    let mut j = i + 1;
                    if let Some(off) = read_s24_at(bc, &mut j) {
                        let target = j as i64 + off as i64;
                        if target > j as i64 && target <= bc.len() as i64 {
                            regions.push((j, target as usize));
                        }
                    }
                }
            }
            _ => { let mut j = op_at + 1; skip_opcode_operands(op, bc, &mut j); i = j; }
        }
    }
    if regions.is_empty() { return; }

    // Second pass: inside a gated region, the thing most recently named is the class being spawned.
    let mut i = 0usize;
    let mut last_lex: Option<String> = None;
    while i < bc.len() {
        let op = bc[i];
        let op_at = i;
        i += 1;
        let gated = regions.iter().any(|(a, b)| op_at >= *a && op_at < *b);
        match op {
            OP_GETLEX => {
                let mn = read_u30_at(bc, &mut i).unwrap_or(0);
                last_lex = abc.multinames.get(mn as usize).map(|m| m.name.to_string());
            }
            OP_CALLPROPERTY | OP_CALLPROPVOID => {
                let mn = read_u30_at(bc, &mut i).unwrap_or(0);
                let _argc = read_u30_at(bc, &mut i).unwrap_or(0);
                let name = abc.multinames.get(mn as usize).map(|m| m.name.to_string()).unwrap_or_default();
                if gated && (name == "spawnEnemy" || name == "spawnProjectile") {
                    if let Some(c) = last_lex.take() { gate.gated_classes.push(c); }
                }
            }
            _ => { let mut j = op_at + 1; skip_opcode_operands(op, bc, &mut j); i = j; }
        }
    }
}

fn read_s24_at(bc: &[u8], i: &mut usize) -> Option<i32> {
    if *i + 3 > bc.len() { return None; }
    let v = bc[*i] as i32 | (bc[*i + 1] as i32) << 8 | (bc[*i + 2] as i8 as i32) << 16;
    *i += 3;
    Some(v)
}

pub(crate) fn extract_enemy_behavior(abc: &AbcFile, class_name: &str) -> EnemyBehavior {
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return EnemyBehavior::default() };
    let mut v = BehaviorVisitor { b: EnemyBehavior::default() };
    // EVERY method the class has, not a list of the five someone thought to name. What is being
    // read here is what the class DECLARES about itself -- the box it makes solid, how hard it
    // shakes the camera -- and a class is free to declare that wherever it likes: bomb factory's
    // block makes its terrain in `actuallyMakeTheTerrain`, so a fixed list found nothing and its
    // blocks were pictures a fighter fell straight through.
    for t in &class.instance_methods {
        if t.kind != TRAIT_METHOD { continue; }
        if DECLARATION_GETTERS.contains(&&*t.name) { continue; }
        if let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) {
            scan_method(&body.bytecode, abc, &mut v);
        }
    }
    v.b
}

/// Collects the animation labels a hazard class plays via `forceAttack("<label>")` — stepping the
/// class's lifecycle methods to learn its REAL animation set (the Thwomp's entrance/idle/fall, the
/// HyruleTornado's "stand"), the code-referenced handle to its art clip, instead of matching a
/// library symbol by keyword.
#[derive(Default)]
struct ForceAttackVisitor {
    labels: Vec<String>,
}
impl AbcVisitor for ForceAttackVisitor {
    fn on_callproperty(&mut self, method: &str, args: &[StackVal], _r: &StackVal) -> Option<StackVal> {
        if method == "forceAttack" {
            if let Some(StackVal::Str(s)) = args.first() {
                if !s.is_empty() && !self.labels.contains(s) { self.labels.push(s.clone()); }
            }
        }
        None
    }
}
/// How an SWF sub-clip's own timeline ENDS its playback, recovered from its Flash-generated
/// `<doc>_fla.<Clip>_NN` class (the 1-based `frameN` instance methods carry the frame scripts,
/// decompiled through the standard pipeline). A clip with no bound class or no hold genuinely loops.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TimelineHold {
    /// `stop()` at 1-based frame N: plays to N and freezes there.
    StopAt(u32),
    /// `gotoAndStop(target)` at 1-based frame N: plays to N, then freezes at `target`
    /// (a frame-label name, or a 1-based frame number rendered as digits).
    GotoStop(String, u32),
}

/// A stage's engine-spawned FALLER cycle (the thwomp pattern), stepped from the code: the enemy
/// class's entrance delay + landed wait (its initialize's FrameTimers, in declaration order), the
/// stage class's spawn cadence (the sum of its two largest timers: active + rest phases of the
/// spawn machine — an approximation until the stage update() is fully reconstructed), and the
/// spawn column x choices (the int-array literal in the stage update near spawnEnemy, terrain-x).
#[derive(Clone, Debug, Default)]
pub struct FallerCycle {
    pub entrance_delay: Option<f64>,
    pub land_wait: Option<f64>,
    pub spawn_period: Option<f64>,
    pub columns: Vec<f64>,
}

pub(crate) fn extract_faller_cycle(abc: &AbcFile, enemy_class: &str) -> Option<FallerCycle> {
    fn eval_expr(e: &str) -> Option<f64> {
        let e = e.trim();
        if let Some((a, b)) = e.split_once('*') {
            return Some(a.trim().parse::<f64>().ok()? * b.trim().parse::<f64>().ok()?);
        }
        e.parse::<f64>().ok()
    }
    let timer_re = crate::re!(r"new FrameTimer\(([^)]+)\)");
    let decompiled = |class: &Class, m: &str| -> String {
        class.instance_methods.iter().find(|t| t.name == m)
            .and_then(|t| abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx))
            .map(|b| decompiler::decompile_method(b, abc, m, &[]))
            .unwrap_or_default()
    };
    let enemy = abc.classes.iter().find(|c| c.name == enemy_class)?;
    let enemy_timers: Vec<f64> = timer_re.captures_iter(&decompiled(enemy, "initialize"))
        .filter_map(|c| eval_expr(&c[1])).collect();
    let mut out = FallerCycle {
        entrance_delay: enemy_timers.first().copied(),
        land_wait: enemy_timers.get(1).copied(),
        ..Default::default()
    };
    if let Some(stage) = abc.classes.iter().find(|c| c.super_name == "SSF2Stage") {
        let mut stage_timers: Vec<f64> = timer_re.captures_iter(&decompiled(stage, "initialize"))
            .filter_map(|c| eval_expr(&c[1])).collect();
        stage_timers.sort_by(|a, b| b.partial_cmp(a).unwrap());
        if stage_timers.len() >= 2 { out.spawn_period = Some(stage_timers[0] + stage_timers[1]); }
        else if let Some(&t) = stage_timers.first() { out.spawn_period = Some(t); }
        // spawn columns: the int-array literal in the stage update near the spawnEnemy call.
        let upd = decompiled(stage, "update");
        if upd.contains("spawnEnemy") {
            let arr_re = crate::re!(r"\[(-?\d+(?:, *-?\d+){2,})\]");
            if let Some(c) = arr_re.captures(&upd) {
                out.columns = c[1].split(',').filter_map(|v| v.trim().parse::<f64>().ok()).collect();
            }
        }
    }
    Some(out)
}

/// Debug aid (`PEPTIDE_DUMP_CLASS=<name>`): decompile every instance method of a class through
/// the standard pipeline and print it — the quickest way to read an SSF2 class's authored logic.
pub(crate) fn dump_class(abc: &AbcFile, class_name: &str) {
    let Some(class) = abc.classes.iter()
        .find(|c| c.name == class_name || c.name.ends_with(&format!(".{class_name}"))) else { return };
    eprintln!("[dump-class] {} extends {}", class.name, class.super_name);
    for t in &class.instance_methods {
        if let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) {
            let code = decompiler::decompile_method(body, abc, &t.name, &[]);
            eprintln!("--- {} ---\n{}", t.name, code);
        }
    }
}

/// The `setYSpeed(N)` timeline a sub-clip's frame scripts drive (the thwomp entrance's
/// descend-hover-rise bob): `(1-based frame, speed)` per frameN method that sets one. Source
/// 30fps SSF2 units.
pub(crate) fn extract_frame_velocities(abc: &AbcFile, class_name: &str) -> Vec<(u32, f64)> {
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return Vec::new() };
    let re = crate::re!(r"setYSpeed\((-?\d+(?:\.\d+)?)\)");
    let mut out: Vec<(u32, f64)> = Vec::new();
    for t in &class.instance_methods {
        let Some(n) = t.name.strip_prefix("frame").and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
        let code = decompiler::decompile_method(body, abc, &t.name, &[]);
        if let Some(c) = re.captures(&code) {
            if let Ok(v) = c[1].parse::<f64>() { out.push((n, v)); }
        }
    }
    out.sort_by_key(|(f, _)| *f);
    out
}

pub(crate) fn extract_timeline_hold(abc: &AbcFile, class_name: &str) -> Option<TimelineHold> {
    let class = abc.classes.iter().find(|c| c.name == class_name)?;
    let stop_re = crate::re!(r"\bstop\(\)");
    let goto_re = crate::re!(r#"gotoAndStop\((?:"([^"]+)"|(\d+))"#);
    // playback freezes at the FIRST frame whose script holds (it never plays past it).
    let mut best: Option<(u32, TimelineHold)> = None;
    for t in &class.instance_methods {
        let Some(n) = t.name.strip_prefix("frame").and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
        let code = decompiler::decompile_method(body, abc, &t.name, &[]);
        let hold = if let Some(c) = goto_re.captures(&code) {
            let target = c.get(1).or_else(|| c.get(2)).map(|m| m.as_str().to_string()).unwrap_or_default();
            Some(TimelineHold::GotoStop(target, n))
        } else if stop_re.is_match(&code) {
            Some(TimelineHold::StopAt(n))
        } else { None };
        if let Some(h) = hold {
            if best.as_ref().map(|(bn, _)| n < *bn).unwrap_or(true) { best = Some((n, h)); }
        }
    }
    best.map(|(_, h)| h)
}

/// A clip that plays ITS OWN labelled segment, and the labels it chooses between.
///
/// A segment that sends playback back to the label it sits on is a LOOP, stated by the source
/// rather than left to Flash's default -- and when the label is built from a random number
/// (`"loop_" + randomInteger(1, 4)`), the clip is really several loops and the object picks one.
/// Clocktown's meteor is four eight-frame tumbles chosen at spawn, which read as one thirty-three
/// frame animation if the labels are ignored.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SelfPlayedLabels {
    /// The frames (1-based) whose script re-plays a label.
    pub at: Vec<u32>,
    /// The labels chosen between, in order, when the label is picked at random. Empty when the
    /// script names one label outright.
    pub pick_between: Vec<String>,
}

/// Read a clip's frame scripts for it playing one of its OWN labels (`stancePlayFrame(...)`).
pub(crate) fn extract_self_played_labels(abc: &AbcFile, class_name: &str) -> Option<SelfPlayedLabels> {
    let class = abc.classes.iter().find(|c| c.name == class_name)?;
    // `stancePlayFrame("prefix_" + field)` — the label is built, so the segment is chosen
    let built = crate::re!(r#"stancePlayFrame\("([^"]*)"\s*\+\s*self\.(\w+)"#);
    let plain = crate::re!(r#"stancePlayFrame\("([^"]+)"\s*\)"#);
    // `self.field = safeRandomInteger(1, 4)` / `randomInteger(1, 4)` — what it is chosen from
    let pick = crate::re!(r"self\.(\w+)\s*=\s*[\w.]*[Rr]andomInteger\((\d+),\s*(\d+)\)");
    let (mut at, mut prefix_field, mut ranges) = (Vec::new(), None, std::collections::BTreeMap::new());
    for t in &class.instance_methods {
        let Some(n) = t.name.strip_prefix("frame").and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
        let code = decompiler::decompile_method(body, abc, &t.name, &[]);
        for c in pick.captures_iter(&code) {
            if let (Ok(lo), Ok(hi)) = (c[2].parse::<u32>(), c[3].parse::<u32>()) {
                ranges.insert(c[1].to_string(), (lo, hi));
            }
        }
        if let Some(c) = built.captures(&code) {
            at.push(n);
            prefix_field = Some((c[1].to_string(), c[2].to_string()));
        } else if plain.is_match(&code) {
            at.push(n);
        }
    }
    if at.is_empty() { return None; }
    at.sort_unstable();
    let pick_between = prefix_field.and_then(|(prefix, field)| {
        let (lo, hi) = ranges.get(&field)?;
        Some((*lo..=*hi).map(|i| format!("{prefix}{i}")).collect::<Vec<_>>())
    }).unwrap_or_default();
    Some(SelfPlayedLabels { at, pick_between })
}

/// What a clip's own timeline does at the end of a labelled segment, from the Flash frame script
/// on that frame. These are what make a multi-label clip a STATE MACHINE rather than a loop: the
/// segments are chained (or branched between) by them, and a segment no jump ever reaches is dead
/// during normal play — playing the clip end to end shows animations the source never shows.
#[derive(Clone, Debug, PartialEq)]
pub enum LabelJump {
    /// `gotoAndPlay("target")`, unconditional.
    Always(String),
    /// `if (safeRandom() > t) gotoAndPlay("target")`: taken `pct` percent of the time, otherwise
    /// playback runs on into whatever frame follows.
    Chance { pct: u32, target: String },
    /// `stop()`: the playhead halts on this frame and stays until something outside the clip
    /// moves it (the stage's code, by label). A plane that holds a section is one of these.
    Stop,
    /// `gotoAndStop("target")`: the playhead moves to the label and halts on its first frame.
    Hold(String),
    /// `gotoAndPlay(currentFrame - k)` or `gotoAndPlay(k)`: a move by FRAME (1-based), resolved to
    /// the label whose span holds that frame once the labels are known. A clip that steps back a
    /// frame from its second frame is holding its first segment as a two-frame loop.
    ToFrame(u32),
}

/// Every timeline move a Flash clip class performs on itself (`gotoAndPlay`, `gotoAndStop`,
/// `stop`), by 1-based frame. Universal: any SSF2 clip whose segments are chained this way, not
/// just backdrop art.
/// Every frame script a clip class carries, as the decompiler reads it, by frame number (1-based).
/// A plane's timeline runs these as it reaches the frame: mushroom kingdom 3's terrain flips the
/// `fallthrough` of each section at each phase label.
pub(crate) fn extract_frame_scripts(abc: &AbcFile, class_name: &str) -> Vec<(u32, String)> {
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return Vec::new() };
    let mut out = Vec::new();
    for t in &class.instance_methods {
        let Some(n) = t.name.strip_prefix("frame").and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
        out.push((n, decompiler::decompile_method(body, abc, &t.name, &[])));
    }
    out.sort_by_key(|(f, _)| *f);
    out
}

pub(crate) fn extract_label_jumps(abc: &AbcFile, class_name: &str) -> Vec<(u32, LabelJump)> {
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return Vec::new() };
    let goto_re = crate::re!(r#"gotoAndPlay\("([^"]+)"\)"#);
    let goto_stop_re = crate::re!(r#"gotoAndStop\("([^"]+)"\)"#);
    let goto_back_re = crate::re!(r"gotoAndPlay\(\s*(?:self\.)?currentFrame\s*-\s*(\d+)\s*\)");
    let goto_num_re = crate::re!(r"gotoAndPlay\(\s*(\d+)\s*\)");
    // a stop a frame back (`gotoAndStop(currentFrame - 1)` on every even frame of a digit clip)
    // is the clip holding where it is: each pair of frames is one held picture
    let stop_back_re = crate::re!(r"gotoAndStop\(\s*(?:(?:self\.)?currentFrame\s*-\s*\d+|\d+)\s*\)");
    let stop_re = crate::re!(r"(?m)(?:^|[^A-Za-z0-9_.])(?:self\.)?stop\(\)");
    // the SSF2 RNG is uniform [0,1), so `> t` fires with probability 1-t and `< t` with t.
    let rand_re = crate::re!(r"safeRandom\(\)\s*([<>])\s*([0-9]*\.?[0-9]+)");
    let mut out = Vec::new();
    for t in &class.instance_methods {
        let Some(n) = t.name.strip_prefix("frame").and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
        let code = decompiler::decompile_method(body, abc, &t.name, &[]);
        let Some(g) = goto_re.captures(&code) else {
            if let Some(h) = goto_stop_re.captures(&code) {
                out.push((n, LabelJump::Hold(h[1].to_string())));
            } else if stop_back_re.is_match(&code) {
                out.push((n, LabelJump::Stop));
            } else if let Some(b) = goto_back_re.captures(&code) {
                let k: u32 = b[1].parse().unwrap_or(0);
                out.push((n, LabelJump::ToFrame(n.saturating_sub(k).max(1))));
            } else if let Some(b) = goto_num_re.captures(&code) {
                out.push((n, LabelJump::ToFrame(b[1].parse::<u32>().unwrap_or(1).max(1))));
            } else if stop_re.is_match(&code) {
                out.push((n, LabelJump::Stop));
            }
            continue;
        };
        let target = g[1].to_string();
        // a random test anywhere in the same frame script is taken to guard the jump: these
        // scripts are a couple of statements long, so there is nothing else for it to guard.
        let jump = match rand_re.captures(&code).and_then(|c| {
            let thr = c[2].parse::<f64>().ok()?;
            Some(if &c[1] == ">" { 1.0 - thr } else { thr })
        }) {
            Some(p) => LabelJump::Chance { pct: (p * 100.0).round().clamp(0.0, 100.0) as u32, target },
            None => LabelJump::Always(target),
        };
        out.push((n, jump));
    }
    out.sort_by_key(|(f, _)| *f);
    out
}

pub(crate) fn extract_force_attack_labels(abc: &AbcFile, class_name: &str) -> Vec<String> {
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return vec![] };
    let mut v = ForceAttackVisitor::default();
    // every method, not a list of six: a class plays its clips from wherever it likes, and the
    // ones it plays from its own helpers were invisible. Bomb factory's block names five labels
    // that way, so two of its six states had to be GUESSED an animation by value order.
    for t in &class.instance_methods {
        if t.kind != TRAIT_METHOD { continue; }
        if DECLARATION_GETTERS.contains(&&*t.name) { continue; }
        if let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) {
            scan_method(&body.bytecode, abc, &mut v);
        }
    }
    v.labels
}

/// Reconstruct a hazard class's lifecycle methods (initialize + update + helpers) as FM hscript by
/// running them through the SAME `decompile_method` → `translate_ssf2_to_fm` pipeline the character
/// and projectile ports use — the real state machine, not a hand-written template. The output still
/// needs a field-state pass (`self.m_x` → `self.makeInt(...)`) + a FrameTimer helper before it RUNS;
/// returned for the gated reconstruction path to iterate on. Empty if the class has no such methods.
/// Methods that are DATA rather than behaviour: read elsewhere as declarations (the body size, the
/// hit params) and so not part of the reconstructed code.
const DECLARATION_GETTERS: &[&str] = &["getOwnStats", "getAttackStats"];

/// Reconstruct a class's behaviour: every method it defines, decompiled and translated.
///
/// Every method, deliberately. A fixed list of lifecycle names ("initialize", "update", ...) drops
/// whatever the class factored out into a helper, and classes do that constantly -- clocktown's
/// stage class keeps its clock in `updateTime` and its weather in `runRain`, bowserscastle's has
/// `bowserLaugh`, and a list would take the `update` that CALLS them and leave the bodies behind.
/// What a class does is all of what it defines.
/// ABC trait kinds: a slot, a method, a constant. The method list a class carries holds all three.
const TRAIT_SLOT: u8 = 0;
const TRAIT_METHOD: u8 = 1;
/// A getter is as much part of a class's interface as a method: bomb factory's stage asks each
/// block for its `xCell`, which the class answers with one. Skipped as "not a method" they were
/// dropped, so the stage compared a cell number against nothing and cleared no row.
const TRAIT_GETTER: u8 = 2;
const TRAIT_SETTER: u8 = 3;
const TRAIT_CONST: u8 = 6;

pub(crate) fn reconstruct_class_script(abc: &AbcFile, class_name: &str) -> Option<String> {
    // the class with every base the package carries, base-most first: what a base declares is
    // the class's too, and an overriding method that calls up to the base keeps the base's
    // version under its own name and calls that
    let chain = class_chain(abc, class_name);
    chain.last()?;
    let mut out = String::new();
    let statics: Vec<(&str, String)> = chain.iter().flat_map(|c| c.class_methods.iter())
        .filter(|t| t.kind == TRAIT_SLOT || t.kind == TRAIT_CONST)
        .filter_map(|t| const_rhs(t).map(|v| (&*t.name, v)))
        .collect();
    let consts: Vec<(&str, String)> = chain.iter().flat_map(|c| c.instance_methods.iter())
        .filter(|t| t.kind == TRAIT_SLOT || t.kind == TRAIT_CONST)
        .filter_map(|t| const_rhs(t).map(|v| (&*t.name, v)))
        .chain(statics.iter().map(|(n, v)| (*n, v.clone())))
        .collect();
    for (name, value) in &consts {
        out.push_str(&format!("var {name} = {value};\n"));
    }
    // a static whose value is computed (a list of points, a product of frame counts) is filled in
    // by the class's own static initializer: its assignments, base-first, become the script's
    // own values (fairy glade's `PIRANHA_SPAWN_LOCATIONS`)
    let mut statics_done: Vec<String> = consts.iter().map(|(n, _)| n.to_string()).collect();
    let mut cinit_lines: Vec<String> = Vec::new();
    for c in &chain {
        if c.class_init_idx == 0 { continue }
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == c.class_init_idx) else { continue };
        let raw = crate::decompiler::decompile_method(body, abc, "__cinit", &[]);
        let translated = crate::api_mappings::translate_ssf2_to_fm(&raw);
        for l in translated.lines().skip_while(|l| !l.starts_with("function __cinit(")).skip(1).take_while(|l| !l.starts_with('}')) {
            let l = l.trim();
            let Some(rest) = l.strip_prefix("self.") else { continue };
            let Some(name) = rest.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')).next() else { continue };
            if name.is_empty() || !rest[name.len()..].trim_start().starts_with('=') || statics_done.iter().any(|d| d == name) { continue }
            statics_done.push(name.to_string());
            cinit_lines.push(format!("var {rest}"));
        }
    }
    for l in &cinit_lines { out.push_str(l); out.push('\n'); }
    if !consts.is_empty() || !cinit_lines.is_empty() { out.push('\n'); }
    // a field declared with a type starts at that type's zero (`name=0`): a phase counter
    // declared `int` is 0 before the class writes it, and read as null it matches no phase
    // a slot whose name is not a plain identifier (a private or namespace-qualified member whose
    // resolved QName carries punctuation, or a garbled read) can never be a var the script names,
    // and emitting it as `var <junk> = null` stops the whole script parsing. drop it.
    let is_ident = |n: &str| {
        let mut cs = n.chars();
        cs.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    // the marker packs `name=value` pairs separated by whitespace, so only a value with NO
    // whitespace can ride it: a numeric zero (`m_action=0`) is fine, but a string default full
    // of spaces and newlines (kingdom1's `hudTextString` HUD template) would be shattered by the
    // reader's split into a broken `name="firstword` plus a bogus var per remaining word. a
    // slot whose default carries whitespace ships as the bare name (null-initialised); its
    // default is set by the class's own code at runtime.
    let fields: Vec<String> = chain.iter().flat_map(|c| c.instance_methods.iter())
        .filter(|t| t.kind == TRAIT_SLOT && t.default.is_none())
        .filter(|t| is_ident(&t.name))
        .map(|t| match const_rhs(t) {
            Some(v) if !v.chars().any(char::is_whitespace) => format!("{}={v}", t.name),
            _ => t.name.to_string(),
        })
        .collect();
    if !fields.is_empty() {
        out.push_str(&format!("// [class-fields] {}\n\n", fields.join(" ")));
    }
    // one construction, base-first: each class's own setup in turn, as the source runs them
    // (two functions of one name would leave only the last, and the base's setup with it)
    let mut built: Vec<String> = Vec::new();
    for c in &chain {
        if let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == c.constructor_idx) {
            let raw = crate::decompiler::decompile_method(body, abc, "__built", &[]);
            let translated = crate::api_mappings::translate_ssf2_to_fm(&raw);
            let kept: Vec<&str> = translated.lines()
                .filter(|l| !l.contains("super(") && !l.trim().starts_with("//"))
                .collect();
            if kept.iter().any(|l| l.contains("self.")) {
                // the body between the signature line and the closing brace
                let inner: Vec<&str> = translated.lines()
                    .skip_while(|l| !l.starts_with("function __built("))
                    .skip(1)
                    .take_while(|l| !l.starts_with('}'))
                    .filter(|l| l.trim() != "return;")
                    .collect();
                built.push(inner.join("\n"));
            }
        }
    }
    if !built.is_empty() {
        out.push_str("function __built() {\n");
        for b in &built { out.push_str(b); out.push('\n'); }
        out.push_str("\treturn;\n}\n\n");
    }
    // every method by name, the most derived body winning; a base's body a derived one calls
    // up to is kept as `<name>__base` and the call goes there
    let mut bodies: Vec<(String, String)> = Vec::new();
    let super_call = crate::re!(r"(?m)^(\s*)// (?:super|self)\.(\w+)\((.*)\)\s*$");
    for c in &chain {
        for t in &c.instance_methods {
            if t.kind != TRAIT_METHOD && t.kind != TRAIT_GETTER { continue; }
            if DECLARATION_GETTERS.contains(&&*t.name) { continue; }
            let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
            let params: Vec<String> = abc.methods.get(body.method_idx as usize)
                .map(|m| (0..m.param_count).map(|i| format!("arg{i}")).collect())
                .unwrap_or_default();
            let raw = crate::decompiler::decompile_method(body, abc, &t.name, &params);
            let mut text = String::new();
            if t.kind == TRAIT_GETTER { text.push_str("// [getter]\n"); }
            text.push_str(&crate::api_mappings::translate_ssf2_to_fm(&raw));
            text.push('\n');
            let mut renames: Vec<(usize, String)> = Vec::new();
            let text = super_call.replace_all(&text, |cap: &regex::Captures| {
                let name = cap[2].to_string();
                match bodies.iter().position(|(n, _)| *n == name) {
                    Some(i) => {
                        let base_name = format!("{name}__base");
                        renames.push((i, base_name.clone()));
                        format!("{}{base_name}({});", &cap[1], &cap[3])
                    }
                    None => cap[0].to_string(),
                }
            }).into_owned();
            for (i, base_name) in renames {
                let (n, b) = bodies[i].clone();
                bodies[i] = (base_name.clone(), b.replacen(&format!("function {n}("), &format!("function {base_name}("), 1));
            }
            match bodies.iter().position(|(n, _)| *n == *t.name) {
                Some(i) => bodies[i] = (t.name.to_string(), text),
                None => bodies.push((t.name.to_string(), text)),
            }
        }
    }
    for (_, text) in &bodies { out.push_str(text); }
    for (name, _) in &consts {
        out = out.replace(&format!("self.{name}"), name);
    }
    // ...and a static named THROUGH the class is the same static. `PeachsCastleBlock.RED` is how
    // a class reads its own kinds as often as it reads them bare, and the class is not a thing
    // here -- only the names it states are. They are written above, so the reads point at them.
    for (name, _) in &statics {
        out = out.replace(&format!("{class_name}.{name}"), name);
    }
    (!out.trim().is_empty()).then_some(out)
}

/// Reconstruct a helper class the character's own code builds with `new`, as a factory
/// that hands back one of them.
///
/// The target has no classes, so a class becomes a function that builds a structure: its
/// fields are that structure's fields and its methods are functions kept on it, closing
/// over it. The caller's `new X(a)` becomes `X__new(a)` and everything it does with the
/// result afterwards -- `it.update()`, `it.bulletStatsName = "..."` -- reads the same.
///
/// Written this way rather than as a flat script because a character can build more than
/// one, and because flat functions would put the class's own state in the same place as
/// the character's, where two fields of the same name are one field.
/// A class and every class it extends THAT THE PACKAGE ITSELF CARRIES, base-most first. The
/// engine's own bases (the enemy, the platform, a clip) are not in the package and end the
/// chain. What a class inherits is written once in its base, so a port that reads only the
/// class's own methods loses the base's: casino night zone's bumpers set their rectangle and
/// their attack record in the base every one of them extends.
pub(crate) fn class_chain<'a>(abc: &'a AbcFile, class_name: &str) -> Vec<&'a Class> {
    let mut chain: Vec<&Class> = Vec::new();
    let mut cur = abc.classes.iter().find(|c| c.name == class_name);
    while let Some(c) = cur {
        if chain.iter().any(|k| k.name == c.name) { break }
        // the package's own api layer sits under the author's classes: shims that forward
        // every call to a handle on the engine. That layer is the engine's, not the class's,
        // and the chain ends there (the stage class extends it; merging it in put the shims'
        // handle fields into the stage script and the engine refused to load it)
        if !chain.is_empty() && is_api_shim(abc, c) { break }
        chain.push(c);
        cur = class_named(abc, &c.super_name).filter(|b| b.name != c.name);
    }
    chain.reverse();
    chain
}

/// A class of the package's api layer: it keeps a handle to the engine (`m_api` / `_api`) and
/// its methods forward to it. Read off the class itself, since every package carries this layer
/// under the same shape and an author's class never holds such a handle.
fn is_api_shim(abc: &AbcFile, class: &Class) -> bool {
    let has_handle = class.instance_methods.iter()
        .any(|t| (t.kind == TRAIT_SLOT || t.kind == TRAIT_CONST) && matches!(&*t.name, "m_api" | "_api" | "$ext"));
    if has_handle { return true }
    // ...or its methods are nothing but forwards to such a handle held by its base
    let forwards = class.instance_methods.iter().filter(|t| t.kind == TRAIT_METHOD).take(6).filter(|t| {
        abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx).is_some_and(|body| {
            let raw = crate::decompiler::decompile_method(body, abc, &t.name, &[]);
            raw.contains("m_api.") || raw.contains("_api.")
        })
    }).count();
    forwards >= 2
}

/// A method by name on the class or the nearest base above it that the package carries.
pub(crate) fn method_in_chain<'a>(abc: &'a AbcFile, class_name: &str, name: &str) -> Option<&'a Trait> {
    class_chain(abc, class_name).iter().rev()
        .find_map(|c| c.instance_methods.iter().find(|t| &*t.name == name))
}

/// The class by that name, whether the package states it plainly or with the package
/// path it sits under.
pub(crate) fn class_named<'a>(abc: &'a AbcFile, name: &str) -> Option<&'a Class> {
    abc.classes.iter().find(|c| c.name == name)
        .or_else(|| abc.classes.iter().find(|c| {
            c.name.rsplit([':', '.']).next().is_some_and(|tail| tail == name)
        }))
}

/// A method's parameters as an hscript signature: every one optional (the source calls with as
/// many arguments as it likes), and one the source declares a numeric default for carries it.
/// Without the default a bare call reads the parameter as nothing: castle siege's action timer
/// is ticked with `tick()` and the source's `tick(frames = 1)` added one a frame; ported without
/// the one it added nothing, and the siege never moved.
/// The signature, and the lines that give a missing argument its declared default: the target's
/// script ignores a default written in the signature (measured: a bare call still read the
/// parameter as null), so it is applied on the way in.
fn optional_sig(m: Option<&Method>) -> (Vec<String>, Vec<String>) {
    let Some(m) = m else { return (Vec::new(), Vec::new()) };
    let pc = m.param_count as usize;
    let first_optional = pc.saturating_sub(m.optionals.len());
    let sig = (0..pc).map(|i| format!("?arg{i}")).collect();
    let defaults = (0..pc).filter_map(|i| {
        let lit = if i >= first_optional { m.optional_defaults.get(i - first_optional).cloned().flatten() } else { None }?;
        Some(format!("\t\tif (arg{i} == null) {{ arg{i} = {lit}; }}\n"))
    }).collect();
    (sig, defaults)
}

/// A decompiled method with its declared parameter defaults applied on the way in: the target's
/// script ignores a default written in the signature, so a bare call read the parameter as
/// null. Inserted after the function's opening line.
pub fn with_param_defaults(code: String, m: Option<&Method>) -> String {
    let Some(m) = m else { return code };
    let pc = m.param_count as usize;
    let first_optional = pc.saturating_sub(m.optional_defaults.len());
    let lines: Vec<String> = (0..pc).filter_map(|i| {
        let lit = if i >= first_optional { m.optional_defaults.get(i - first_optional).cloned().flatten() } else { None }?;
        Some(format!("\tif (arg{i} == null) {{ arg{i} = {lit}; }}\n"))
    }).collect();
    if lines.is_empty() { return code; }
    match code.find("{\n") {
        Some(at) => format!("{}{}{}", &code[..at + 2], lines.join(""), &code[at + 2..]),
        None => code,
    }
}

pub fn reconstruct_class_as_factory(abc: &AbcFile, class_name: &str) -> Option<String> {
    let class = class_named(abc, class_name)?;
    let ctor_params: Vec<String> = abc.method_bodies.iter()
        .find(|b| b.method_idx == class.constructor_idx)
        .and_then(|b| abc.methods.get(b.method_idx as usize))
        .map(|m| (0..m.param_count).map(|i| format!("arg{i}")).collect())
        .unwrap_or_default();

    // optional, for the same reason a method's are: the source builds a class with as
    // many arguments as it feels like and the target counts them
    let (sig, defaults) = optional_sig(abc.method_bodies.iter()
        .find(|b| b.method_idx == class.constructor_idx)
        .and_then(|b| abc.methods.get(b.method_idx as usize)));
    let mut out = format!("function {class_name}__new({}) {{\n{}\tvar it = {{}};\n",
        sig.join(", "), defaults.join("").replace("\t\tif", "\tif"));

    // what the class states about itself, before anything runs
    for t in &class.instance_methods {
        if t.kind != TRAIT_SLOT && t.kind != TRAIT_CONST { continue }
        let rhs = const_rhs(t).unwrap_or_else(|| "null".to_string());
        out.push_str(&format!("\tit.{} = {rhs};\n", t.name));
    }

    // AN ACCESSOR IS CODE, NOT A VALUE. A getter is read without parentheses and answers
    // from the object's state, so it is kept as a FIELD the reader can read the way the source
    // does -- and that field is worked out again every time anything on the object runs, since
    // that is when the state it answers from can have changed. A getter worked out once, at
    // construction, is a value frozen before the object has done anything: a frame timer's
    // `completed` read true from its first frame and never otherwise, and a stage asked its
    // arwing to arrive on every frame of the match.
    //
    // A setter is the same the other way round: a write to its name must run its body, so
    // the body is kept as `__set_<name>` and every write to the name, here and wherever the
    // object is handed to, becomes a call to it (see `factory_setters`).
    let getters: Vec<&Trait> = class.instance_methods.iter().filter(|t| t.kind == TRAIT_GETTER).collect();
    let setters: Vec<&Trait> = class.instance_methods.iter().filter(|t| t.kind == TRAIT_SETTER).collect();
    let refresh: String = getters.iter().map(|t| format!("\t\tit.{0} = it.__get_{0}();\n", t.name)).collect();
    for t in &class.instance_methods {
        if t.kind != TRAIT_METHOD && t.kind != TRAIT_GETTER && t.kind != TRAIT_SETTER { continue }
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else {
            continue
        };
        let params: Vec<String> = abc.methods.get(body.method_idx as usize)
            .map(|m| (0..m.param_count).map(|i| format!("arg{i}")).collect())
            .unwrap_or_default();
        let raw = crate::decompiler::decompile_method(body, abc, &t.name, &params);
        let translated = crate::api_mappings::translate_ssf2_to_fm(&raw);
        let lines = body_of(&translated);
        // optional, like every other ported method: whoever calls one back decides how
        // many arguments it gets, and the target refuses the call when they do not match
        let (sig, defaults) = optional_sig(abc.methods.get(body.method_idx as usize));
        let fname = match t.kind {
            TRAIT_GETTER => format!("__get_{}", t.name),
            TRAIT_SETTER => format!("__set_{}", t.name),
            _ => t.name.clone(),
        };
        out.push_str(&format!("\tit.{fname} = function({}) {{\n{}", sig.join(", "), defaults.join("")));
        // a method or a setter changes what the getters answer, so they are worked out again
        // on the way out of it: before each return, and at the end
        // a write to a setter's name inside the class runs the setter (the refresh lines
        // added below assign the getter fields and must stay assignments, so this is done
        // to the body first)
        let body_lines: Vec<String> = lines.iter().map(|l| {
            let mut l = l.trim_end().to_string();
            for s in &setters {
                let re = crate::dyn_re(&format!(r"^(\s*)(?:self|it)\.{}\s*=\s*([^=][^;]*);", regex::escape(&s.name)));
                l = re.replace(&l, format!("${{1}}it.__set_{}(${{2}});", s.name)).to_string();
            }
            l
        }).collect();
        for line in &body_lines {
            if t.kind != TRAIT_GETTER && line.trim().starts_with("return") && !refresh.is_empty() {
                out.push_str(&refresh);
            }
            out.push_str(&format!("\t\t{line}\n"));
        }
        if t.kind != TRAIT_GETTER && !body_lines.last().is_some_and(|l| l.trim().starts_with("return")) {
            out.push_str(&refresh);
        }
        out.push_str("\t};\n");
    }

    // WHAT IT BUILDS BEFORE IT STARTS, last: the constructor reaches for what the class
    // can do -- it hands one of its own methods to a listener -- so everything it can do
    // has to be on it before the constructor runs, even though the source states the
    // constructor first.
    if let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == class.constructor_idx) {
        let raw = crate::decompiler::decompile_method(body, abc, "__built", &ctor_params);
        let translated = crate::api_mappings::translate_ssf2_to_fm(&raw);
        for line in body_of(&translated) {
            if line.contains("super(") { continue }
            // the constructor's own `return` ends the factory before it hands anything back
            if line.trim() == "return;" { continue }
            let mut l = line.trim_end().to_string();
            for s in &setters {
                let re = crate::dyn_re(&format!(r"^(\s*)(?:self|it)\.{}\s*=\s*([^=][^;]*);", regex::escape(&s.name)));
                l = re.replace(&l, format!("${{1}}it.__set_{}(${{2}});", s.name)).to_string();
            }
            out.push_str(&format!("\t{l}\n"));
        }
    }

    // what the getters answer once everything is built
    for t in &getters {
        out.push_str(&format!("\tit.{0} = it.__get_{0}();\n", t.name));
    }
    out.push_str("\treturn it;\n}\n");
    // inside the class, what it says about itself is about the one being built
    let mut out = out.replace("self.", "it.");

    // and a field it names on its own, without saying whose it is, is still its own:
    // a class reads its fields bare as often as it says `self` first.
    // Which names are its own, taken from what the structure being built actually gets:
    // a class does not always DECLARE its fields where the declaration can be read, but
    // it always sets them, and what it sets is what it has.
    let own: std::collections::BTreeSet<String> = {
        let re = crate::re!(r"(?m)^\s*it\.([A-Za-z_]\w*)\s*=");
        re.captures_iter(&out).map(|c| c[1].to_string()).collect()
    };
    for name in &own {
        if name.is_empty() { continue }
        out = bind_bare_name_to_instance(&out, name);
    }
    // A write to one of its own fields comes back through the decompiler naming the
    // field twice, once as the thing being written and once as what it is written on
    // (`m_duration.m_duration = 4`). A field's own name repeated can only be that
    // field, so it is said once.
    for name in &own {
        if name.is_empty() { continue }
        out = out.replace(&format!("it.{name}.{name}"), &format!("it.{name}"));
    }
    // the setters, named on the first line for whoever hands the object around (after the
    // rewrites above, which would otherwise rename them as fields of the thing being built)
    if !setters.is_empty() {
        let marker = format!("\tvar it = {{}};\n\t// setters: {}\n",
            setters.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(","));
        out = out.replacen("\tvar it = {};\n", &marker, 1);
    }
    Some(out)
}

/// Point a bare mention of one of the class's own fields at the one being built.
///
/// Not the ones already said as `it.name`, not a longer name that merely starts or ends
/// with this one, and not where it is the name being given a value on the structure.
fn bind_bare_name_to_instance(code: &str, name: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(rel) = rest.find(name) {
        let (before, tail) = rest.split_at(rel);
        let after = &tail[name.len()..];
        let prev_ok = before.chars().next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.'));
        let next_ok = after.chars().next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        out.push_str(before);
        if prev_ok && next_ok { out.push_str("it."); }
        out.push_str(name);
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Reconstruct a class the character's code only ever READS names off, as the set of
/// names it states. A class of nothing but constants is a place to keep names, and here
/// that is a structure with those names on it.
pub(crate) fn reconstruct_class_as_constants(abc: &AbcFile, class_name: &str) -> Option<String> {
    let class = class_named(abc, class_name)?;
    let mut entries: Vec<String> = Vec::new();
    // ordinal fallback: a class of names states them in the order it lists them, and the
    // source only ever compares one against another, so what matters is that they differ
    let mut next = 0i64;
    for t in class.class_methods.iter().chain(class.instance_methods.iter()) {
        if t.kind != TRAIT_SLOT && t.kind != TRAIT_CONST { continue }
        if t.name.is_empty() { continue }
        let rhs = const_rhs(t).unwrap_or_else(|| { next += 1; format!("{}", next - 1) });
        entries.push(format!("\t{}: {rhs}", t.name));
    }
    if entries.is_empty() { return None }
    Some(format!("var {class_name} = {{\n{}\n}};\n", entries.join(",\n")))
}

/// The lines inside a rendered `function name(...) { ... }`, without its wrapper./// The lines inside a rendered `function name(...) { ... }`, without its wrapper.
fn body_of(rendered: &str) -> Vec<String> {
    let mut lines: Vec<&str> = rendered.lines().collect();
    while lines.first().is_some_and(|l| !l.contains('{')) { lines.remove(0); }
    if !lines.is_empty() { lines.remove(0); }
    while lines.last().is_some_and(|l| l.trim().is_empty()) { lines.pop(); }
    // exactly ONE closer: the function's own. The ones before it close the blocks
    // inside it, and taking those leaves every one of them open.
    if lines.last().is_some_and(|l| l.trim() == "}") { lines.pop(); }
    while lines.last().is_some_and(|l| l.trim().is_empty()) { lines.pop(); }
    lines.iter()
        .map(|l| l.strip_prefix('\t').unwrap_or(l).to_string())
        // a call up to what the class extends has nothing above it here
        .map(|l| if l.contains("super(") || l.contains("super.") {
            format!("// [SSF2-only: super] {}", l.trim_start())
        } else { l })
        .collect()
}

pub(crate) fn reconstruct_enemy_script(abc: &AbcFile, class_name: &str) -> Option<String> {
    reconstruct_class_script(abc, class_name)
}

/// The getAttackStats hitbox maps a HAZARD class declares (damage/direction/power/kbConstant + any
/// geometry), authoritative per-hazard instead of a generic per-kind default. Empty if the class
/// has no getAttackStats or no recoverable hitbox. Pairs with [`extract_own_stats_for`].
pub(crate) fn extract_attack_stats_for(abc: &AbcFile, class_name: &str) -> Vec<BTreeMap<String, f64>> {
    let Some(class) = abc.classes.iter().find(|c| c.name == class_name) else { return vec![] };
    // A class states its hit wherever it keeps it. Most put it in `getAttackStats`; a projectile
    // usually carries it inside its OWN stats instead, under the attack it is doing
    // (`attack_idle: { attackBoxes: { attackBox: { damage: 3, … } } }`). Reading only the first
    // shipped saffroncity's leaf and charmander fire with no damage at all, because the boxes they
    // declare are in the other place.
    for want in ["getAttackStats", "getOwnStats"] {
        let Some(t) = class.instance_methods.iter().find(|t| &*t.name == want) else { continue };
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
        let mut v = HazardAttackVisitor { best: vec![] };
        scan_method(&body.bytecode, abc, &mut v);
        if !v.best.is_empty() { return v.best; }
    }
    vec![]
}

/// The flat scalar fields a hazard class's `getOwnStats` declares (size, speeds, timers — e.g. the
/// HyruleTornado's `xSpeed`/`maxTime`). Authoritative motion/own params instead of guessed defaults.
pub(crate) fn extract_own_stats_for(abc: &AbcFile, class_name: &str) -> BTreeMap<String, f64> {
    let Some(t) = method_in_chain(abc, class_name, "getOwnStats") else { return BTreeMap::new() };
    let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { return BTreeMap::new() };
    let mut v = OwnStatsVisitor { best: BTreeMap::new() };
    scan_method(&body.bytecode, abc, &mut v);
    if std::env::var("PEPTIDE_DUMP_OWN_STATS").is_ok() {
        for (k, val) in &v.best { eprintln!("[own-stat] {k} = {val}"); }
    }
    v.best
}
/// The projectiles a class declares in `getProjectileStats`, keyed by the name its own code fires
/// them under. The declaration carries the linkage the art is registered as, so a stage object's
/// projectile is found the way a fighter's is: by what the class says, not by what it is called.
pub(crate) fn extract_projectiles_for(abc: &AbcFile, class_name: &str) -> BTreeMap<String, ProjectileData> {
    let Some(t) = method_in_chain(abc, class_name, "getProjectileStats") else { return BTreeMap::new() };
    let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { return BTreeMap::new() };
    extract_projectile_objects(&body.bytecode, abc)
}
/// The yes/no declarations a class makes about itself in `getOwnStats`.
///
/// A class states how it behaves as much as how big it is: whether fighters pass through it,
/// whether a hit sends it flying. Those are the source's own words about the object and decide
/// what the port has to arrange, so they are read alongside the numbers rather than dropped.
pub(crate) fn extract_own_flags_for(abc: &AbcFile, class_name: &str) -> BTreeMap<String, bool> {
    let Some(t) = method_in_chain(abc, class_name, "getOwnStats") else { return BTreeMap::new() };
    let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { return BTreeMap::new() };
    let mut v = OwnFlagsVisitor { best: BTreeMap::new() };
    scan_method(&body.bytecode, abc, &mut v);
    v.best
}
struct OwnFlagsVisitor {
    best: BTreeMap<String, bool>,
}
impl AbcVisitor for OwnFlagsVisitor {
    fn on_newobject(&mut self, obj: BTreeMap<String, StackVal>, _c: &Option<String>) -> NewObjectAction {
        let flags: BTreeMap<String, bool> = obj.iter()
            .filter_map(|(k, v)| if let StackVal::Bool(b) = v { Some((k.clone(), *b)) } else { None })
            .collect();
        // the outermost object is the one the method returns, and it is built last
        if !flags.is_empty() { self.best = flags; }
        NewObjectAction::PushObj(obj)
    }
}

/// The library clip a hazard class names for itself in `getOwnStats` (`linkage_id`). This is the
/// class SAYING which art it is, so a port can find its clip -- and its collision box -- without a
/// keyword taxonomy that only knows the hazards someone thought to name.
pub(crate) fn extract_own_linkage_for(abc: &AbcFile, class_name: &str) -> Option<String> {
    let t = method_in_chain(abc, class_name, "getOwnStats")?;
    let body = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx)?;
    let mut v = OwnLinkageVisitor { found: None };
    scan_method(&body.bytecode, abc, &mut v);
    v.found
}
struct OwnLinkageVisitor {
    found: Option<String>,
}
impl AbcVisitor for OwnLinkageVisitor {
    fn on_newobject(&mut self, obj: BTreeMap<String, StackVal>, _c: &Option<String>) -> NewObjectAction {
        if self.found.is_none() {
            if let Some(StackVal::Str(s)) = obj.get("linkage_id") {
                if !s.is_empty() { self.found = Some(s.clone()); }
            }
        }
        NewObjectAction::PushObj(obj)
    }
}

struct OwnStatsVisitor {
    best: BTreeMap<String, f64>,
}
impl AbcVisitor for OwnStatsVisitor {
    fn on_newobject(&mut self, obj: BTreeMap<String, StackVal>, _c: &Option<String>) -> NewObjectAction {
        let nums: BTreeMap<String, f64> = obj.iter()
            .filter_map(|(k, v)| if let StackVal::Num(n) = v { Some((k.clone(), *n)) } else { None })
            .collect();
        // The OUTERMOST object is the one the method returns, and an object's members are built
        // before the object that holds them, so the outermost is the LAST one built. Taking
        // whichever had the most numbers instead picked a nested hitbox whenever it happened to
        // carry more of them than the body did: saffroncity's venusaur leaf states its own
        // width and height and lost both to its attackBox, so the object could not be sized and
        // did not ship at all.
        if !nums.is_empty() {
            self.best = nums;
        }
        NewObjectAction::PushObj(obj)
    }
}

/// Extract per-projectile stat objects from a `getProjectileStats()`
/// bytecode body. Mirrors `extract_attack_objects` but recognises
/// projectile-shaped objects: an object is a projectile if it carries
/// any flat-scalar physics field (gravity / xSpeed / ySpeed / friction /
/// etc.) OR a nested `attackBoxes` hitbox map.
///
/// Returns a map keyed by the SSF2 projectile name (the key under which
/// the per-projectile object appears in the top-level returned object).
/// SSF2 physics-stat keys commonly found on a projectile-stats object.
/// Any inner value-object holding at least one of these (or an

/// Visitor for `getProjectileStats()`-style bodies. The top-level
/// newobject's KEYS are projectile names; the VALUES are physics +
/// attackBoxes objects we promote to `ProjectileData`.
struct ProjectileVisitor {
    result: BTreeMap<String, ProjectileData>,
}

impl AbcVisitor for ProjectileVisitor {
    /// Carry the class a `getlex` names, so a declaration's `classAPI` arrives as the class the
    /// package wrote rather than as an unknown. That field is the package saying WHAT the thing
    /// is, and it is the only thing here that separates a projectile from the character's own
    /// stats block or from an attack.
    fn on_getlex(&mut self, name: &str) -> Option<StackVal> {
        Some(StackVal::Tag(name.to_string()))
    }

    fn on_newobject(
        &mut self,
        obj: BTreeMap<String, StackVal>,
        _current_char: &Option<String>,
    ) -> NewObjectAction {
        // A projectile declaration SAYS what it is: it carries `classAPI`, naming the api class
        // the thing is (`SSF2Projectile`). That is the package's own statement, so nothing here
        // needs a vocabulary of physics fields -- which is what this used to key on, and which
        // matched every ATTACK in the file (they carry `attackBoxes` too) while missing the
        // projectiles entirely, because the objects that declare `classAPI` are nested one level
        // deeper than the map that names them.
        //
        // The NAME lives in the parent, so entries are recorded when the parent is built, and the
        // object is always handed back to the stack so the parent can still see its children.
        for (name, val) in &obj {
            let StackVal::Obj(inner) = val else { continue };
            // Every entry in getProjectileStats IS a projectile: the METHOD says so, not the name of
            // the class it names. So the only thing to check is that this is a declaration and not a
            // nested attackBoxes/hitbox object -- a declaration carries `classAPI` (the behaviour
            // class it runs, `SSF2Projectile` or a custom one like `SoraStrikeRaid`), a hitbox does
            // not. Do NOT filter on what that class is called: a custom behaviour class carries no
            // "Projectile" in its name and is still a projectile, because it is here.
            if !matches!(inner.get("classAPI"), Some(StackVal::Str(_) | StackVal::Tag(_))) { continue }
            let mut stats: BTreeMap<String, f64> = BTreeMap::new();
            let mut flags: BTreeMap<String, bool> = BTreeMap::new();
            let mut names: BTreeMap<String, String> = BTreeMap::new();
            for (k, v) in inner {
                match v {
                    StackVal::Num(n) => { stats.insert(k.clone(), *n); }
                    StackVal::Bool(b) => { flags.insert(k.clone(), *b); }
                    StackVal::Str(t) | StackVal::Tag(t) => { names.insert(k.clone(), t.clone()); }
                    _ => {}
                }
            }
            let hitboxes = extract_hitboxes_from_val(val);
            self.result.insert(name.clone(), ProjectileData { stats, flags, names, hitboxes });
        }
        NewObjectAction::PushObj(obj)
    }
}

pub fn extract_projectile_objects(bytecode: &[u8], abc: &AbcFile) -> BTreeMap<String, ProjectileData> {
    let mut v = ProjectileVisitor { result: BTreeMap::new() };
    scan_method(bytecode, abc, &mut v);
    v.result
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
            // These names are how a box is RECOGNISED. What is kept is everything it states: a
            // list of names to keep is a list that has to grow before a field can be used, and the
            // thwomp's `selfHitStun` -- how long the attacker freezes -- was dropped on the way in,
            // so no mapping downstream could have carried it however well it was written.
            let hitbox_keys = ["damage", "direction", "power", "kbConstant", "hitStun", "hitLag", "weightKB"];
            if obj.keys().any(|k| hitbox_keys.contains(&k.as_str())) {
                // a stated true or false is kept as 1 or 0: goku's forward tilt states its boxes
                // `reversableAngle: false`, and dropped here they converted as reversible
                let hb: BTreeMap<String, f64> = obj.iter()
                    .filter_map(|(k, v)| match v {
                        StackVal::Num(n) => Some((k.clone(), *n)),
                        StackVal::Bool(b) => Some((k.clone(), if *b { 1.0 } else { 0.0 })),
                        _ => None,
                    })
                    .collect();
                return vec![hb];
            }
            // Might be a container of hitboxes: {attackBox: {...}, attackBox2: {...}, ...}
            let mut hitboxes = Vec::new();
            for (k, v) in obj {
                if let Some(suffix) = k.strip_prefix("attackBox") {
                    // which box it is, by its name (`attackBox` the first, `attackBox2` the
                    // second), kept so its stats land on the box the entity draws under that name
                    let idx = if suffix.is_empty() { 0.0 } else { suffix.parse::<f64>().map(|n| n - 1.0).unwrap_or(0.0) };
                    for mut hb in extract_hitboxes_from_val(v) {
                        hb.entry(BOX_INDEX_KEY.to_string()).or_insert(idx);
                        hitboxes.push(hb);
                    }
                }
            }
            hitboxes.sort_by(|a, b| a.get(BOX_INDEX_KEY).partial_cmp(&b.get(BOX_INDEX_KEY)).unwrap_or(std::cmp::Ordering::Equal));
            // A projectile states its boxes one level deeper, under the state that arms them
            // (`attack_idle: { attackBoxes: … }`), so an object that carries none of its own is
            // asked about what it contains rather than answered with nothing.
            if hitboxes.is_empty() {
                for v in obj.values() {
                    if matches!(v, StackVal::Obj(_)) {
                        hitboxes.extend(extract_hitboxes_from_val(v));
                    }
                }
            }
            hitboxes
        }
        _ => vec![]
    }
}

/// Visitor for `getOwnStats()`-style bodies (the heuristic fallback,
/// distinct from the targeted `extract_ssf2_stats` linear scan). Looks
/// for the first newobject whose keys overlap the canonical SSF2
/// character-stat names by at least 3; stops scanning the moment it
/// finds one.
struct StatsVisitor {
    found: Option<CharStats>,
}

const STATS_FALLBACK_KEYS: &[&str] = &[
    "gravity", "weight1", "norm_xSpeed", "max_xSpeed",
    "fastFallSpeed", "jumpSpeed", "jumpSpeedMidair",
    "accel_rate_air", "decel_rate_air", "max_ySpeed",
    "accel_rate", "walkSpeed", "dashSpeed", "airMobility",
];

impl AbcVisitor for StatsVisitor {
    fn on_newobject(
        &mut self,
        obj: BTreeMap<String, StackVal>,
        _current_char: &Option<String>,
    ) -> NewObjectAction {
        let numeric_stats: BTreeMap<String, f64> = obj.iter()
            .filter_map(|(k, v)| {
                if let StackVal::Num(n) = v { Some((k.clone(), *n)) } else { None }
            }).collect();
        let match_count = numeric_stats.keys()
            .filter(|k| STATS_FALLBACK_KEYS.contains(&k.as_str()))
            .count();
        if match_count >= 3 {
            let names = obj.iter()
                .filter_map(|(k, v)| if let StackVal::Str(t) = v { Some((k.clone(), t.clone())) } else { None })
                .collect();
            self.found = Some(CharStats { values: numeric_stats, names });
            return NewObjectAction::Stop;
        }
        NewObjectAction::PushObj(obj)
    }
}

fn extract_stats_from_body(bytecode: &[u8], abc: &AbcFile) -> Option<CharStats> {
    let mut v = StatsVisitor { found: None };
    scan_method(bytecode, abc, &mut v);
    v.found
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
                let key = abc.strings.get(str_idx as usize).map(|s| s.to_string()).unwrap_or_default();
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
                        // FIRST-WINS: the genuine top-level character stats object
                        // appears first in the bundle method; later per-projectile
                        // data objects re-use keys like "gravity"/"width"/"height"
                        // (usually pushbyte 0), which last-wins would let clobber
                        // the real value. Keep the first occurrence.
                        values.entry(key).or_insert(v);
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
            OP_IFLE | OP_IFGT | OP_IFGE | OP_IFSTRICTEQ | OP_IFSTRICTNE
                if i + 3 <= bytecode.len() => { i += 3; }
            _ => {}
        }
    }

    if values.len() >= 3 {
        Some(CharStats { values, names: BTreeMap::new() })
    } else {
        None
    }
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

pub(crate) fn normalize_attack_name(s: &str) -> String {
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

/// Visitor for `misc.ssf :: getCostumeData()` and per-character costume
/// methods. Two palette-table shapes are recognised:
///
///   - Pattern A (misc.ssf): `{ team | base, paletteSwap: { colors,
///     replacements } }`. Costume name comes from `base` (Default) or
///     `team` (capitalised) or a synthesised `Alt N` counter.
///
///   - Pattern B (character file): `{ name: "...", colors: [uint…] }`.
///     Costume name is the literal `name`; no `replacements`.
///
/// The visitor relies on `scan_method`'s costume-mode getproperty
/// semantics to keep `current_char` in sync across multi-character
/// misc.ssf bodies (where the runtime key on `_loc1_[char]` IS the
/// character id, and the static name on `obj.mario` IS the character id).
struct CostumeVisitor {
    per_char: BTreeMap<String, Vec<CostumeData>>,
    alt_counters: BTreeMap<String, usize>,
}

impl AbcVisitor for CostumeVisitor {
    fn costume_getproperty_semantics(&self) -> bool { true }

    fn on_newarray(&mut self, items: Vec<StackVal>) -> StackVal {
        // Costume needs to read array contents back as colour literals,
        // so build a real Arr instead of the default Unknown placeholder.
        StackVal::Arr(items)
    }

    fn on_newobject(
        &mut self,
        obj: BTreeMap<String, StackVal>,
        current_char: &Option<String>,
    ) -> NewObjectAction {
        // Pattern A — misc.ssf style.
        if let Some(StackVal::Obj(ps)) = obj.get("paletteSwap") {
            let colors = stackval_arr_to_u32(ps.get("colors"));
            let replacements = stackval_arr_to_u32(ps.get("replacements"));
            if colors.len() >= 4 && colors.len() == replacements.len() {
                let char_key = current_char.clone().unwrap_or_else(|| "unknown".to_string());
                let alt_n = self.alt_counters.entry(char_key.clone()).or_insert(0);
                let costume_name = if obj.contains_key("base") {
                    "Default".to_string()
                } else if let Some(StackVal::Str(team)) = obj.get("team") {
                    // Capitalise the team name ("red" → "Red").
                    let mut c = team.chars();
                    match c.next() {
                        None    => team.clone(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                } else {
                    *alt_n += 1;
                    format!("Alt {}", alt_n)
                };
                self.per_char.entry(char_key).or_default()
                    .push(CostumeData { name: costume_name, colors, replacements });
            }
        }
        // Pattern B — character-file style.
        else if let (Some(StackVal::Str(name)), Some(StackVal::Arr(color_arr))) =
            (obj.get("name"), obj.get("colors"))
        {
            let colors: Vec<u32> = color_arr.iter().filter_map(|v| {
                if let StackVal::Num(n) = v { Some(*n as u32) } else { None }
            }).collect();
            if colors.len() >= 4 {
                let char_key = current_char.clone().unwrap_or_else(|| "unknown".to_string());
                self.per_char.entry(char_key).or_default()
                    .push(CostumeData { name: name.clone(), colors, replacements: vec![] });
            }
        }
        NewObjectAction::PushObj(obj)
    }
}

/// Pull every numeric item out of a possibly-Arr StackVal as u32.
fn stackval_arr_to_u32(v: Option<&StackVal>) -> Vec<u32> {
    match v {
        Some(StackVal::Arr(arr)) => arr.iter().filter_map(|x| {
            if let StackVal::Num(n) = x { Some(*n as u32) } else { None }
        }).collect(),
        _ => vec![],
    }
}

fn decode_costume_objects(code: &[u8], abc: &AbcFile) -> BTreeMap<String, Vec<CostumeData>> {
    let mut v = CostumeVisitor {
        per_char: BTreeMap::new(),
        alt_counters: BTreeMap::new(),
    };
    scan_method(code, abc, &mut v);
    v.per_char
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[test]
    fn a_package_with_no_name_of_its_own_does_not_take_the_run_down() {
        // a package searched by whoever borrows from it need not be named, and the tail of an
        // empty name is a slice past its end. this is the name that gets built from it.
        for name in ["", "a", "mario"] {
            let built = format!("{}Ext",
                name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()
                + name.char_indices().nth(1).map_or("", |(i, _)| &name[i..]));
            assert!(built.ends_with("Ext"), "name for {name:?} came out wrong: {built}");
        }
    }

    use super::*;

    // ── Bytecode-construction helpers ────────────────────────────────────
    //
    // Each `push_*` helper writes one AVM2 opcode + operand to a Vec<u8>.
    // The companion `mk_abc()` builds the constant pool slots the
    // opcodes index into. With these we can hand-craft tiny method bodies
    // that exercise the shared scanner against each visitor without
    // having to round-trip through a real SWF.

    fn write_u30(bc: &mut Vec<u8>, mut v: u32) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 { bc.push(b); break; }
            bc.push(b | 0x80);
        }
    }

    fn push_string(bc: &mut Vec<u8>, idx: u32) { bc.push(0x2C); write_u30(bc, idx); }
    fn push_double(bc: &mut Vec<u8>, idx: u32) { bc.push(0x2F); write_u30(bc, idx); }
    fn push_byte  (bc: &mut Vec<u8>, v: i8)    { bc.push(0x24); bc.push(v as u8); }
    fn push_uint  (bc: &mut Vec<u8>, idx: u32) { bc.push(0x2E); write_u30(bc, idx); }
    fn newobject  (bc: &mut Vec<u8>, n: u32)   { bc.push(0x55); write_u30(bc, n); }
    fn newarray   (bc: &mut Vec<u8>, n: u32)   { bc.push(0x56); write_u30(bc, n); }

    /// Build an `AbcFile` whose constant pools are seeded with the
    /// given strings / doubles / uints (indices start at 1, as AVM2's
    /// constant pool has a sentinel at index 0).
    fn mk_abc(strings: &[&str], doubles: &[f64], uints: &[u32]) -> AbcFile {
        let mut all_strings: Vec<Arc<str>> = vec![Arc::from("")];
        all_strings.extend(strings.iter().map(|s| Arc::from(*s)));
        let mut all_doubles = vec![f64::NAN];
        all_doubles.extend(doubles.iter().copied());
        let mut all_uints = vec![0u32];
        all_uints.extend(uints.iter().copied());
        AbcFile {
            strings: all_strings,
            ints: vec![0],
            uints: all_uints,
            doubles: all_doubles,
            multinames: vec![Multiname { kind: 0, name_idx: 0, ns_idx: 0, name: Arc::from("") }],
            methods: vec![],
            classes: vec![],
            scripts: vec![],
            method_bodies: vec![],
        }
    }

    // ── AttackVisitor ────────────────────────────────────────────────────

    #[test]
    fn attack_visitor_recognises_a_air_object() {
        // Build bytecode that constructs:
        //   { a_air: { damage: 5, direction: 45, power: 10 } }
        let abc = mk_abc(&["a_air", "damage", "direction", "power"], &[], &[]);
        let mut bc = Vec::new();
        push_string(&mut bc, 1); // "a_air" (outer key)
        push_string(&mut bc, 2); // "damage"
        push_byte(&mut bc, 5);
        push_string(&mut bc, 3); // "direction"
        push_byte(&mut bc, 45);
        push_string(&mut bc, 4); // "power"
        push_byte(&mut bc, 10);
        newobject(&mut bc, 3);   // inner hitbox = { damage, direction, power }
        newobject(&mut bc, 1);   // outer = { a_air: inner }
        bc.push(0x48);            // returnvalue
        let result = extract_attack_objects(&bc, &abc);
        assert!(result.contains_key("aerial_neutral"),
            "a_air must normalise to aerial_neutral; got: {:?}", result.keys().collect::<Vec<_>>());
        let attack = &result["aerial_neutral"];
        assert_eq!(attack.hitboxes.len(), 1, "one hitbox expected; got: {:?}", attack.hitboxes);
        assert_eq!(attack.hitboxes[0].get("damage").copied(), Some(5.0));
        assert_eq!(attack.hitboxes[0].get("direction").copied(), Some(45.0));
        assert_eq!(attack.hitboxes[0].get("power").copied(), Some(10.0));
    }

    #[test]
    fn attack_visitor_ignores_non_attack_objects() {
        let abc = mk_abc(&["foo", "bar"], &[], &[]);
        let mut bc = Vec::new();
        push_string(&mut bc, 1); push_byte(&mut bc, 1);
        push_string(&mut bc, 2); push_byte(&mut bc, 2);
        newobject(&mut bc, 2);
        let result = extract_attack_objects(&bc, &abc);
        assert!(result.is_empty(), "unmatched keys should produce no attacks; got: {:?}", result);
    }

    // ── ProjectileVisitor ────────────────────────────────────────────────

    #[test]
    fn projectile_visitor_takes_what_declares_itself_a_projectile() {
        // { mario_fireball: { classAPI: "SSF2Projectile", gravity: 0.5, xSpeed: 10 } }
        let abc = mk_abc(&["mario_fireball", "gravity", "xSpeed", "classAPI", "SSF2Projectile"], &[0.5], &[]);
        let mut bc = Vec::new();
        push_string(&mut bc, 1);           // outer key
        push_string(&mut bc, 4);           // "classAPI"
        push_string(&mut bc, 5);           // "SSF2Projectile"
        push_string(&mut bc, 2);           // "gravity"
        push_double(&mut bc, 1);            // 0.5
        push_string(&mut bc, 3);           // "xSpeed"
        push_byte  (&mut bc, 10);
        newobject  (&mut bc, 3);            // the declaration
        newobject  (&mut bc, 1);            // the map that names it
        let result = extract_projectile_objects(&bc, &abc);
        assert!(result.contains_key("mario_fireball"),
            "projectile key must be picked up; got: {:?}", result.keys().collect::<Vec<_>>());
        let proj = &result["mario_fireball"];
        assert_eq!(proj.stats.get("gravity").copied(), Some(0.5));
        assert_eq!(proj.stats.get("xSpeed").copied(), Some(10.0));
    }

    #[test]
    fn projectile_visitor_leaves_attacks_alone() {
        // An ATTACK carries hitboxes and physics-shaped fields too, and every character has
        // dozens of them. Only what declares itself a projectile counts, or a character's whole
        // moveset arrives as projectiles and the real ones (nested a level deeper) never do.
        let abc = mk_abc(&["a_forward", "damage", "gravity"], &[8.0, 0.5], &[]);
        let mut bc = Vec::new();
        push_string(&mut bc, 1);
        push_string(&mut bc, 2);
        push_double(&mut bc, 1);
        push_string(&mut bc, 3);
        push_double(&mut bc, 2);
        newobject  (&mut bc, 2);
        newobject  (&mut bc, 1);
        let result = extract_projectile_objects(&bc, &abc);
        assert!(result.is_empty(), "an attack is not a projectile; got: {:?}", result.keys().collect::<Vec<_>>());
    }

    #[test]
    fn projectile_visitor_skips_non_physics_inner() {
        // Inner obj has no projectile-shape keys → not recognised.
        let abc = mk_abc(&["foo", "label"], &[], &[]);
        let mut bc = Vec::new();
        push_string(&mut bc, 1);
        push_string(&mut bc, 2);
        push_byte  (&mut bc, 1);
        newobject  (&mut bc, 1);
        newobject  (&mut bc, 1);
        let result = extract_projectile_objects(&bc, &abc);
        assert!(result.is_empty(),
            "no-physics inner objects must not be recognised; got: {:?}", result);
    }

    // ── StatsVisitor ─────────────────────────────────────────────────────

    #[test]
    fn stats_visitor_accepts_threshold_of_3_keys() {
        // { gravity: 1, weight1: 100, jumpSpeed: 9 }  → 3 stat keys → accept
        let abc = mk_abc(&["gravity", "weight1", "jumpSpeed"], &[], &[]);
        let mut bc = Vec::new();
        push_string(&mut bc, 1); push_byte(&mut bc, 1);
        push_string(&mut bc, 2); push_byte(&mut bc, 100);
        push_string(&mut bc, 3); push_byte(&mut bc, 9);
        newobject(&mut bc, 3);
        let result = extract_stats_from_body(&bc, &abc);
        let stats = result.expect("3+ stat keys must yield CharStats");
        assert_eq!(stats.values.get("gravity").copied(), Some(1.0));
        assert_eq!(stats.values.get("weight1").copied(), Some(100.0));
        assert_eq!(stats.values.get("jumpSpeed").copied(), Some(9.0));
    }

    #[test]
    fn stats_visitor_rejects_two_keys() {
        // Only 2 stat keys → below threshold → None.
        let abc = mk_abc(&["gravity", "weight1"], &[], &[]);
        let mut bc = Vec::new();
        push_string(&mut bc, 1); push_byte(&mut bc, 1);
        push_string(&mut bc, 2); push_byte(&mut bc, 100);
        newobject(&mut bc, 2);
        assert!(extract_stats_from_body(&bc, &abc).is_none());
    }

    #[test]
    fn stats_visitor_stops_after_first_match() {
        // First newobject matches and triggers Stop; a second newobject
        // (with different values) must NOT overwrite the first.
        let abc = mk_abc(&["gravity", "weight1", "jumpSpeed"], &[], &[]);
        let mut bc = Vec::new();
        // First (matching) object
        push_string(&mut bc, 1); push_byte(&mut bc, 1);
        push_string(&mut bc, 2); push_byte(&mut bc, 100);
        push_string(&mut bc, 3); push_byte(&mut bc, 9);
        newobject(&mut bc, 3);
        // Second (also matching, but different values) — should never run.
        push_string(&mut bc, 1); push_byte(&mut bc, 99);
        push_string(&mut bc, 2); push_byte(&mut bc, 99);
        push_string(&mut bc, 3); push_byte(&mut bc, 99);
        newobject(&mut bc, 3);
        let stats = extract_stats_from_body(&bc, &abc).expect("first match should be returned");
        // Values must be from the FIRST newobject (1 / 100 / 9), not the second (99s).
        assert_eq!(stats.values.get("gravity").copied(), Some(1.0));
        assert_eq!(stats.values.get("weight1").copied(), Some(100.0));
    }

    // ── CostumeVisitor ───────────────────────────────────────────────────

    #[test]
    fn costume_visitor_pattern_b_character_file() {
        // { name: "Default", colors: [0xff112233, 0xff445566, 0xff778899, 0xffaabbcc] }
        let abc = mk_abc(&["Default", "name", "colors"], &[], &[0xff112233, 0xff445566, 0xff778899, 0xffaabbcc]);
        let mut bc = Vec::new();
        // Build the colors array on the stack first.
        push_uint(&mut bc, 1);  push_uint(&mut bc, 2);
        push_uint(&mut bc, 3);  push_uint(&mut bc, 4);
        newarray(&mut bc, 4);
        // Now the outer obj.
        push_string(&mut bc, 2);                     // "name"
        push_string(&mut bc, 1);                     // "Default"
        push_string(&mut bc, 3);                     // "colors"
        // ← The newarray result is already on the stack just below "name"/"Default"/"colors".
        //   But the obj wants pairs in (key, value) order from the bottom.
        //   Simpler: rebuild fresh — pop the existing array.
        // Reset: drop the array we just built and start over with the
        // canonical key/value order.
        let mut bc = Vec::new();
        push_string(&mut bc, 2);                     // key "name"
        push_string(&mut bc, 1);                     // value "Default"
        push_string(&mut bc, 3);                     // key "colors"
        push_uint(&mut bc, 1); push_uint(&mut bc, 2); push_uint(&mut bc, 3); push_uint(&mut bc, 4);
        newarray(&mut bc, 4);                         // value = arr
        newobject(&mut bc, 2);                        // { name, colors }
        let result = decode_costume_objects(&bc, &abc);
        let costumes = result.get("unknown").expect("no current_char → 'unknown' bucket");
        assert_eq!(costumes.len(), 1, "exactly one costume expected");
        assert_eq!(costumes[0].name, "Default");
        assert_eq!(costumes[0].colors, vec![0xff112233, 0xff445566, 0xff778899, 0xffaabbcc]);
    }

    #[test]
    fn costume_visitor_rejects_under_four_colors() {
        // colors.len() < 4 → silent skip (matches the legacy behaviour).
        let abc = mk_abc(&["Default", "name", "colors"], &[], &[0x11, 0x22]);
        let mut bc = Vec::new();
        push_string(&mut bc, 2);
        push_string(&mut bc, 1);
        push_string(&mut bc, 3);
        push_uint(&mut bc, 1); push_uint(&mut bc, 2);
        newarray(&mut bc, 2);
        newobject(&mut bc, 2);
        let result = decode_costume_objects(&bc, &abc);
        assert!(result.is_empty() || result.values().all(|v| v.is_empty()),
            "under-4-colour objects must not produce a CostumeData; got: {:?}", result);
    }

    // ── Shared scanner edge cases ────────────────────────────────────────

    #[test]
    fn scanner_handles_empty_bytecode() {
        let abc = mk_abc(&[], &[], &[]);
        let result = extract_attack_objects(&[], &abc);
        assert!(result.is_empty());
    }

    #[test]
    fn scanner_survives_unknown_opcode() {
        // Opcode 0xFF (undefined) → default branch in scan_method's match
        // should be a no-op. Bytecode that reaches a newobject after an
        // unknown opcode should still extract correctly.
        let abc = mk_abc(&["a_air", "damage"], &[], &[]);
        let mut bc = Vec::new();
        bc.push(0xFF);            // garbage / unknown opcode
        push_string(&mut bc, 1);
        push_string(&mut bc, 2); push_byte(&mut bc, 5);
        newobject(&mut bc, 1);     // inner hitbox { damage: 5 }
        newobject(&mut bc, 1);     // outer { a_air: inner }
        let result = extract_attack_objects(&bc, &abc);
        assert!(result.contains_key("aerial_neutral"),
            "unknown opcode must not derail recognition; got: {:?}", result.keys().collect::<Vec<_>>());
    }

    #[test]
    fn scanner_returnvalue_pops_one() {
        // After returnvalue, the scanner should pop the top of stack but
        // KEEP scanning subsequent ops (which may produce more
        // newobjects). This pins the behaviour against the OLD
        // decode_costume_objects' break-on-return, which we deliberately
        // unified to pop-and-continue.
        let abc = mk_abc(&["a_air", "damage"], &[], &[]);
        let mut bc = Vec::new();
        push_byte(&mut bc, 0);
        bc.push(0x48); // returnvalue — pops the dummy 0
        // Even after returnvalue, the scanner continues. Build a
        // recognised attack obj next:
        push_string(&mut bc, 1);
        push_string(&mut bc, 2); push_byte(&mut bc, 5);
        newobject(&mut bc, 1);
        newobject(&mut bc, 1);
        let result = extract_attack_objects(&bc, &abc);
        assert!(result.contains_key("aerial_neutral"),
            "post-returnvalue opcodes must still be processed; got: {:?}", result.keys().collect::<Vec<_>>());
    }
}

// ── weather motion: what the source's particles actually DO ──────────────────

/// A per-frame quantity a weather class computes, in the one shape those classes use:
/// a constant plus a random spread (`1 + safeRandom() * 2`, `safeRandom() * fallSpeed`).
/// `base + rand * U(0,1)`, in SSF2 units per frame; +y is DOWN, +x is RIGHT.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drift {
    pub base: f64,
    pub rand: f64,
}

impl Drift {
    fn cst(v: f64) -> Self { Drift { base: v, rand: 0.0 } }
    fn neg(self) -> Self { Drift { base: -self.base, rand: -self.rand } }
    fn add(self, o: Self) -> Self { Drift { base: self.base + o.base, rand: self.rand + o.rand } }
    fn sub(self, o: Self) -> Self { self.add(o.neg()) }
    /// Only a scale by a CONSTANT is representable; anything else is not this shape.
    fn mul(self, o: Self) -> Option<Self> {
        if o.rand == 0.0 { Some(Drift { base: self.base * o.base, rand: self.rand * o.base }) }
        else if self.rand == 0.0 { Some(Drift { base: o.base * self.base, rand: o.rand * self.base }) }
        else { None }
    }
    /// Divided by a plain number, which scales what the drift spans. Only a constant divisor
    /// makes sense: dividing BY a spread is not a drift any more.
    fn div(self, o: Self) -> Option<Self> {
        (o.rand == 0.0 && o.base != 0.0)
            .then(|| Drift { base: self.base / o.base, rand: self.rand / o.base })
    }
    pub fn is_zero(&self) -> bool { self.base == 0.0 && self.rand == 0.0 }
    /// The mean value, which is what decides which way the field drifts overall.
    pub fn mean(&self) -> f64 { self.base + self.rand / 2.0 }
}

/// How a weather class moves ONE particle each frame, read out of that class's own code.
///
/// Weather is not decoration the converter is free to invent: rain falls fast and straight, embers
/// rise and wander, and a port that picks one of those for every stage gets the other stages wrong.
/// Both are the same three statements over a particle (`y += <drift>`, `x += <drift>`, wrap at the
/// edge), so reading the two drifts off the source covers the pattern rather than one stage of it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeatherMotion {
    /// Per-frame vertical drift, +down (rain positive, embers negative).
    pub dy: Drift,
    /// Per-frame horizontal drift, +right.
    pub dx: Drift,
    /// The size the source gives each particle when it makes it, as a drift over `U(0,1)`. SSF2's
    /// rain draws every drop from one clip and then scales it per drop, which is what makes a
    /// field of them read as depth rather than as a hundred copies of one drop.
    pub scale: Option<Drift>,
}

/// Multiname kinds whose NAME comes off the stack (`o[i]`), so a read or write through one pops
/// an extra operand: MultinameL / MultinameLA / RTQnameL / RTQnameLA.
fn is_runtime_name(kind: u8) -> bool { matches!(kind, 0x11 | 0x12 | 0x1B | 0x1C) }

/// One value in the symbolic walk: a drift, or something this pass cannot read.
#[derive(Clone, Copy)]
enum Sym { Val(Drift), Unknown }

impl Sym {
    fn val(self) -> Option<Drift> { match self { Sym::Val(d) => Some(d), Sym::Unknown => None } }
}

/// Walk a method body over an abstract stack, resolving `field` reads through `fields` and
/// `getlocal N` through `params`. Calls `on_store(name, value)` for every `setproperty`.
///
/// Deliberately shallow: anything it cannot model pushes `Unknown`, which propagates, so an
/// unreadable expression yields no answer instead of a wrong one.
fn walk_drift(
    abc: &AbcFile,
    bc: &[u8],
    fields: &BTreeMap<String, Drift>,
    params: &BTreeMap<usize, Drift>,
    mut on_store: impl FnMut(&str, Sym, Sym),
) {
    let mut st: Vec<Sym> = Vec::new();
    let mut i = 0usize;
    while i < bc.len() {
        let op = bc[i];
        let at = i;
        i += 1;
        match op {
            OP_PUSHBYTE => { let v = match bc.get(i) { Some(b) => *b as i8 as f64, None => return }; i += 1; st.push(Sym::Val(Drift::cst(v))); }
            OP_PUSHSHORT => { let Some(v) = read_u30_at(bc, &mut i) else { return }; st.push(Sym::Val(Drift::cst(v as f64))); }
            OP_PUSHINT => { let Some(x) = read_u30_at(bc, &mut i) else { return };
                            st.push(abc.ints.get(x as usize).map(|v| Sym::Val(Drift::cst(*v as f64))).unwrap_or(Sym::Unknown)); }
            OP_PUSHDOUBLE => { let Some(x) = read_u30_at(bc, &mut i) else { return };
                               st.push(abc.doubles.get(x as usize).map(|v| Sym::Val(Drift::cst(*v))).unwrap_or(Sym::Unknown)); }
            OP_GETLOCAL1 => st.push(params.get(&1).map(|d| Sym::Val(*d)).unwrap_or(Sym::Unknown)),
            OP_GETLOCAL2 => st.push(params.get(&2).map(|d| Sym::Val(*d)).unwrap_or(Sym::Unknown)),
            OP_GETLOCAL3 => st.push(params.get(&3).map(|d| Sym::Val(*d)).unwrap_or(Sym::Unknown)),
            OP_GETLOCAL => { let Some(n) = read_u30_at(bc, &mut i) else { return };
                             st.push(params.get(&(n as usize)).map(|d| Sym::Val(*d)).unwrap_or(Sym::Unknown)); }
            OP_GETPROPERTY => {
                let Some(mn) = read_u30_at(bc, &mut i) else { return };
                let Some(m) = abc.multinames.get(mn as usize) else { return };
                // a runtime-named read (`o[i]`) takes its name off the stack as well
                if is_runtime_name(m.kind) { st.pop(); }
                let name = m.name.to_string();
                st.pop();                                    // the object being read from
                st.push(fields.get(&name).map(|d| Sym::Val(*d)).unwrap_or(Sym::Unknown));
            }
            OP_CALLPROPERTY => {
                let Some(mn) = read_u30_at(bc, &mut i) else { return };
                let Some(argc) = read_u30_at(bc, &mut i) else { return };
                let name = abc.multinames.get(mn as usize).map(|m| m.name.to_string()).unwrap_or_default();
                let mut arg0 = Sym::Unknown;
                for k in 0..argc { let v = st.pop().unwrap_or(Sym::Unknown); if k == 0 { arg0 = v; } }
                st.pop();                                    // the receiver
                // the engine's uniform random in [0,1): the whole reason a drift carries a spread
                let is_rand = name.eq_ignore_ascii_case("safeRandom") || name.eq_ignore_ascii_case("random");
                // rounding does not change what a drift SPANS, only which values inside it can come
                // up, so it passes the value through. SSF2 quantises its rain-drop sizes that way
                // (`round(x * 10) / 10`), and reading the call as unknown lost the size entirely.
                let is_round = argc == 1
                    && (name.eq_ignore_ascii_case("round") || name.eq_ignore_ascii_case("floor")
                        || name.eq_ignore_ascii_case("ceil"));
                st.push(if is_rand && argc == 0 { Sym::Val(Drift { base: 0.0, rand: 1.0 }) }
                        else if is_round { arg0 } else { Sym::Unknown });
            }
            OP_ADD | OP_SUBTRACT | OP_MULTIPLY | OP_DIVIDE => {
                let b = st.pop().unwrap_or(Sym::Unknown);
                let a = st.pop().unwrap_or(Sym::Unknown);
                let r = match (a.val(), b.val()) {
                    (Some(x), Some(y)) => match op {
                        OP_ADD => Some(x.add(y)),
                        OP_SUBTRACT => Some(x.sub(y)),
                        OP_DIVIDE => x.div(y),
                        _ => x.mul(y),
                    },
                    _ => None,
                };
                st.push(r.map(Sym::Val).unwrap_or(Sym::Unknown));
            }
            OP_NEGATE => { let a = st.pop().unwrap_or(Sym::Unknown); st.push(a.val().map(|d| Sym::Val(d.neg())).unwrap_or(Sym::Unknown)); }
            OP_COERCE_A | OP_COERCE | OP_NOP => { if op == OP_COERCE { let _ = read_u30_at(bc, &mut i); } }
            OP_DUP => { let t = st.last().copied().unwrap_or(Sym::Unknown); st.push(t); }
            OP_SETPROPERTY | OP_INITPROPERTY => {
                let Some(mn) = read_u30_at(bc, &mut i) else { return };
                let Some(m) = abc.multinames.get(mn as usize) else { return };
                let v = st.pop().unwrap_or(Sym::Unknown);
                if is_runtime_name(m.kind) { st.pop(); }
                let obj = st.pop().unwrap_or(Sym::Unknown);
                on_store(m.name.as_ref(), v, obj);
            }
            // value-producing ops this walk does not READ but does have to keep the stack straight
            // for: they leave one unreadable value behind, and dropping the whole stack instead
            // loses the readable operands already pushed under them (an ember's `1 + random() * 2`
            // is exactly that shape).
            OP_GETLEX | OP_FINDPROP | OP_FINDPROPSTRICT => { let _ = read_u30_at(bc, &mut i); st.push(Sym::Unknown); }
            OP_PUSHNULL | OP_PUSHTRUE | OP_PUSHFALSE | OP_PUSHNAN | OP_GETLOCAL0 => st.push(Sym::Unknown),
            OP_CALLPROPVOID => {
                let Some(_) = read_u30_at(bc, &mut i) else { return };
                let Some(argc) = read_u30_at(bc, &mut i) else { return };
                for _ in 0..argc { st.pop(); }
                st.pop();
            }
            OP_CONSTRUCTPROP => {
                let Some(_) = read_u30_at(bc, &mut i) else { return };
                let Some(argc) = read_u30_at(bc, &mut i) else { return };
                for _ in 0..argc { st.pop(); }
                st.pop();
                st.push(Sym::Unknown);
            }
            _ => {
                // an op this walk does not model: its result is not readable, and neither is
                // anything computed from it, so drop the stack rather than mis-pair operands.
                let mut j = at + 1;
                skip_opcode_operands(op, bc, &mut j);
                i = j;
                st.clear();
            }
        }
    }
}

/// Read a weather class's per-frame particle motion.
///
/// Two passes over the class: the constructor says what each per-particle field holds (a literal
/// spread, or a constructor parameter the stage passed), and the mover method says how those
/// fields are applied to `x` and `y`.
pub fn weather_motion(abc: &AbcFile, class_name: &str, args: &[Option<f64>], dir: Option<&str>) -> Option<WeatherMotion> {
    let class = abc.classes.iter().find(|c| c.name == class_name)?;
    let ctor = abc.methods.get(class.constructor_idx as usize)?;
    let cbody = abc.method_bodies.iter().find(|b| b.method_idx == class.constructor_idx)?;

    // constructor parameters, by the number `getlocal` gives them: what the stage passed, else
    // what the signature declares as the default.
    let pc = ctor.param_count as usize;
    let first_optional = pc.saturating_sub(ctor.optionals.len());
    let mut params: BTreeMap<usize, Drift> = BTreeMap::new();
    for p in 1..=pc {
        let idx = p - 1;
        let v = args.get(idx).copied().flatten()
            .or_else(|| if idx >= first_optional { ctor.optionals.get(idx - first_optional).copied().flatten() } else { None });
        if let Some(v) = v { params.insert(p, Drift::cst(v)); }
    }

    // pass 1: every field the constructor gives a readable value (on `this` or on the particle).
    let mut fields: BTreeMap<String, Drift> = BTreeMap::new();
    walk_drift(abc, &cbody.bytecode, &BTreeMap::new(), &params, |name, v, _| {
        if let Some(d) = v.val() { fields.insert(name.to_string(), d); }
    });

    // pass 2: the mover. `x`/`y` are written as the current value plus a drift, so the drift is
    // what the store holds minus the position itself -- which reads directly, because a position
    // read is Unknown and so only the pure-drift form survives the walk. Instead of subtracting,
    // match the shape: a store of `<pos> ± <drift>` leaves `Unknown ± drift` on the stack, so the
    // walk is re-run with the position field resolving to zero.
    let mut zero_pos = fields.clone();
    zero_pos.insert("x".into(), Drift::cst(0.0));
    zero_pos.insert("y".into(), Drift::cst(0.0));
    let mut motion = WeatherMotion::default();
    // the per-particle size, set once when the particle is made (constructor pass)
    motion.scale = fields.get("scaleX").or_else(|| fields.get("scaleY")).copied()
        .filter(|d| d.base.is_finite() && d.rand.is_finite() && (d.base != 0.0 || d.rand != 0.0));
    let mut seen = (false, false);
    let mut order: Vec<_> = class.instance_methods.iter().collect();
    if let Some(d) = dir.map(|d| d.to_ascii_lowercase()).filter(|d| !d.is_empty()) {
        // a class that was handed a direction has one mover per direction (`moveLeft`/`moveRight`)
        // and runs the one named for what it was given. If nothing is named for it, the class does
        // not branch on the string and every method stays a candidate.
        let named: Vec<_> = order.iter().copied().filter(|t| t.name.to_ascii_lowercase().contains(&d)).collect();
        if !named.is_empty() { order = named; }
    }
    for t in order {
        let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
        walk_drift(abc, &body.bytecode, &zero_pos, &BTreeMap::new(), |name, v, _| {
            let Some(d) = v.val() else { return };
            // the wrap (`y = -height`, `x = random * hArea`) is a JUMP, not a drift: it only ever
            // stores a whole position, never the position plus something. Those come out of the
            // walk with the position reading as zero, so they look like a drift of the full value.
            // The move is the FIRST store in the mover, before any wrap, so keep the first only.
            match name {
                "y" if !seen.1 => { motion.dy = d; seen.1 = true; }
                "x" if !seen.0 => { motion.dx = d; seen.0 = true; }
                _ => {}
            }
        });
        if seen.0 && seen.1 { break; }
    }
    (seen.0 || seen.1).then_some(motion)
}

/// What one of a stage's own clips SAYS it is, read off its frame script.
///
/// A stage's tree is not a picture with some collision guessed out of it: every clip in it states
/// its own role on frame 1 (`self.type = "platform"`), and a clip whose behaviour belongs to a
/// class says which class (`self.classAPI = CentralHighwayPlatform`). Both are facts the file
/// carries, so reading them needs no vocabulary and cannot miss a clip whose author named it
/// something nobody has seen.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClipRole {
    /// The game's own word for what this clip is: `terrain`, `platform`, `l_ledge`, `p1_spawn`, …
    pub kind: String,
    /// The class that drives it, when the clip names one.
    pub class_api: Option<String>,
    /// Whether the clip hides ITSELF on its first frame. A stage keeps pieces it only shows at a
    /// moment of its choosing -- crateria's acid warning sign -- and the clip says so in its own
    /// frame script rather than in the placement.
    pub starts_hidden: bool,
    /// A platform that says nobody drops through it (`self.noDropThrough = true`): the game
    /// honours that flag on a platform, so it lands like a platform and holds like a floor.
    pub no_drop_through: bool,
    /// A platform that declares a BOUNCE (`self.bounce_speed = 25; self.bounce_frame = "bounce"`):
    /// the game's platform layer sends a fighter that lands on it up at that speed and plays that
    /// label on the clip. The speed and the label, as declared.
    pub bounce: Option<(String, String)>,
    /// A terrain piece that declares a sideways push (`self.x_influence = -5.9`, a lamp's slope):
    /// the game's terrain layer moves a fighter standing on it by that much a frame. As declared.
    pub x_influence: Option<String>,
    /// The key the clip moves in step with (`self.syncPlatform = "x"`): a ledge beacon and a
    /// terrain piece that declare the same key are one moving thing to the game.
    pub sync_platform: Option<String>,
    /// The ground friction the piece declares for whoever stands on it (`self.decel_friction`),
    /// and the walk acceleration (`self.accel_friction`), in the source's px a frame squared:
    /// race to the finish's ice.
    pub decel_friction: Option<f64>,
    pub accel_friction: Option<f64>,
    /// The game's own class the clip asks to be run by (`self.className = "DynamicPlatform"`): a
    /// class no package ships, so what it does is read off the declarations beside it.
    pub class_name: Option<String>,
    /// The movements such a clip declares, in index order (`self.movement1 = { moveTime: 265,
    /// waitTime: 10, xSpeed: -2.4, xAccel: 0.1, xDecel: 0.1 }`), each as the literal's text.
    pub movements: Vec<String>,
    /// Which movement it starts on (`self.startIndex = 0`).
    pub start_index: u32,
    /// Whether a fighter can drop through it (`self.fallthrough = false`), when it says.
    pub fallthrough: Option<bool>,
    /// The picture a collision clip names for itself in the foreground (`self.foreground = "x"`).
    pub foreground: Option<String>,
    /// The children the clip hides on its first frame (`self.ground.visible = false`): drawn in
    /// the file, never shown by the game.
    pub hidden_children: Vec<String>,
}

/// Every clip class in the package that declares a role, keyed by class name (local, no namespace).
pub(crate) fn declared_clip_roles(abc: &AbcFile) -> std::collections::BTreeMap<String, ClipRole> {
    let ty_re = crate::re!(r#"self\.type = "([^"]+)""#);
    let api_re = crate::re!(r"self\.classAPI = ([A-Za-z_][A-Za-z0-9_.]*)");
    let hide_re = crate::re!(r"self\.(?:visible = false|setVisible\(false\))");
    let hide_child_re = crate::re!(r"self\.(\w+)\.visible = false");
    let nodrop_re = crate::re!(r"self\.noDropThrough = true");
    let bounce_speed_re = crate::re!(r"self\.bounce_speed = (-?\d+(?:\.\d+)?)");
    let bounce_frame_re = crate::re!(r#"self\.bounce_frame = "([^"]+)""#);
    let push_re = crate::re!(r"self\.x_influence = (-?\d+(?:\.\d+)?)");
    let sync_re = crate::re!(r#"self\.syncPlatform = "([^"]+)""#);
    let decel_fric_re = crate::re!(r"self\.decel_friction = (-?\d+(?:\.\d+)?)");
    let accel_fric_re = crate::re!(r"self\.accel_friction = (-?\d+(?:\.\d+)?)");
    let class_name_re = crate::re!(r#"self\.className = "([^"]+)""#);
    let movement_re = crate::re!(r"self\.movement(\d+) = (\{[^}]*\});");
    let start_re = crate::re!(r"self\.startIndex = (\d+)");
    let fall_re = crate::re!(r"self\.fallthrough = (true|false)");
    let fg_re = crate::re!(r#"self\.foreground = "([^"]+)""#);
    let mut out = std::collections::BTreeMap::new();
    for class in &abc.classes {
        let mut role = ClipRole::default();
        for t in &class.instance_methods {
            if t.name.strip_prefix("frame").and_then(|n| n.parse::<u32>().ok()).is_none() { continue }
            let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
            let n = t.name.strip_prefix("frame").and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);
            let code = decompiler::decompile_method(body, abc, &t.name, &[]);
            if let Some(c) = ty_re.captures(&code) { role.kind = c[1].to_string(); }
            if n == 1 && hide_re.is_match(&code) { role.starts_hidden = true; }
            if n == 1 {
                for c in hide_child_re.captures_iter(&code) { role.hidden_children.push(c[1].to_string()); }
            }
            if nodrop_re.is_match(&code) { role.no_drop_through = true; }
            if let Some(c) = push_re.captures(&code) { role.x_influence = Some(c[1].to_string()); }
            if let Some(c) = sync_re.captures(&code) { role.sync_platform = Some(c[1].to_string()); }
            if let Some(c) = decel_fric_re.captures(&code) { role.decel_friction = c[1].parse().ok(); }
            if let Some(c) = accel_fric_re.captures(&code) { role.accel_friction = c[1].parse().ok(); }
            if let Some(c) = class_name_re.captures(&code) { role.class_name = Some(c[1].to_string()); }
            let mut moves: Vec<(u32, String)> = movement_re.captures_iter(&code).map(|c| (c[1].parse().unwrap_or(0), c[2].to_string())).collect();
            if !moves.is_empty() { moves.sort_by_key(|(i, _)| *i); role.movements.extend(moves.into_iter().map(|(_, m)| m)); }
            if let Some(c) = start_re.captures(&code) { role.start_index = c[1].parse().unwrap_or(0); }
            if let Some(c) = fall_re.captures(&code) { role.fallthrough = Some(&c[1] == "true"); }
            if let Some(c) = fg_re.captures(&code) { role.foreground = Some(c[1].to_string()); }
            if let Some(c) = bounce_speed_re.captures(&code) {
                let speed = c[1].to_string();
                let label = bounce_frame_re.captures(&code).map(|l| l[1].to_string())
                    .or_else(|| role.bounce.as_ref().map(|(_, l)| l.clone())).unwrap_or_else(|| "bounce".to_string());
                role.bounce = Some((speed, label));
            }
            if let Some(c) = api_re.captures(&code) {
                role.class_api = Some(c[1].rsplit('.').next().unwrap_or(&c[1]).to_string());
            }
        }
        if role.kind.is_empty() && role.class_api.is_none() && !role.starts_hidden && !role.no_drop_through && role.bounce.is_none() && role.x_influence.is_none() && role.sync_platform.is_none() && role.decel_friction.is_none() && role.class_name.is_none() && role.foreground.is_none() && role.hidden_children.is_empty() { continue }
        let local = class.name.rsplit('.').next().unwrap_or(&class.name).to_string();
        out.insert(local, role);
    }
    out
}

#[cfg(test)]
mod instance_binding_tests {
    use super::*;

    #[test]
    fn a_field_named_bare_is_the_instance_own() {
        let src = "\tit.data = m_data;\n\tit.m_data = null;\n";
        let out = bind_bare_name_to_instance(src, "m_data");
        assert!(out.contains("it.data = it.m_data;"), "{out}");
        assert!(out.contains("it.m_data = null;"), "not doubled: {out}");
    }

    #[test]
    fn a_longer_name_is_left_alone() {
        let src = "x = m_elapsedFrames + elapsedFramesTotal;";
        let out = bind_bare_name_to_instance(src, "elapsedFrames");
        assert_eq!(out, src, "{out}");
    }
}
