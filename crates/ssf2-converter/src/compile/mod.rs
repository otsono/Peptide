//! The translation: the source model's code to Fraymakers' script language.
//!
//! It knows AS3 and nothing about the game. The writer (`native`) turns each statement into the
//! Haxe statement that means the same, and each API call into its row in
//! `runtime/native_calls.jsonc`; the tables beside it say what the source's declared stats,
//! events and engine states do. What the API declares (its classes and their members) is
//! `runtime/api_classes.jsonc`, read by `Runtime::load`, so a name the content inherits from an API
//! class resolves to that member, and a name nothing provides is reported instead of guessed at.

pub mod attack_stats;
pub mod projectile_stats;
pub mod autolink;
pub mod engine_anims;
pub mod events;
pub mod native;
pub mod runtime;

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{ClassModel, MethodKind, SourceModel};
pub use runtime::Runtime;

/// what the whole package declares, for resolving names in any of its classes
pub struct Package<'a> {
    pub model: &'a SourceModel,
    pub runtime: &'a Runtime,
    /// the fields whose values live in the engine's state containers, so a rollback restores them:
    /// every field a class of the package's own declares, by name, unless the name is also an
    /// accessor, a method, a static, or a member of an API class
    pub stored: BTreeSet<String>,
}

impl<'a> Package<'a> {
    pub fn new(model: &'a SourceModel, runtime: &'a Runtime) -> Package<'a> {
        let mut accessors = BTreeSet::new();
        let mut plain_fields = BTreeSet::new();
        for c in model.classes.values().filter(|c| !runtime.provides(&c.name)) {
            for m in &c.methods {
                if matches!(m.kind, MethodKind::Getter | MethodKind::Setter) { accessors.insert(m.name.clone()); }
            }
            for f in &c.fields { plain_fields.insert(f.name.clone()); }
        }
        accessors.extend(runtime.accessors());
        let methods: BTreeSet<&str> = model.classes.values().flat_map(|c| c.methods.iter().map(|m| m.name.as_str())).collect();
        // a class's statics are its own values (constants, mostly) and stay plain: a container made
        // as a class's section runs is not one the engine keeps
        let statics: BTreeSet<&str> = model.classes.values().flat_map(|c| c.fields.iter().filter(|f| f.is_static).map(|f| f.name.as_str())).collect();
        let stored = if cells_on() {
            plain_fields.iter().filter(|n| !accessors.contains(*n) && !methods.contains(n.as_str()) && !statics.contains(n.as_str()) && !n.starts_with("__")
                && !runtime.classes.values().any(|c| c.members.contains(*n) || c.getters.contains(*n))
                && !runtime.globals.contains(*n)).cloned().collect()
        } else { BTreeSet::new() };
        Package { model, runtime, stored }
    }

    /// the class a name refers to, by its full or short name
    pub fn class_named(&self, name: &str) -> Option<&'a ClassModel> {
        self.model.classes.get(name).or_else(|| {
            let mut hits = self.model.classes.values().filter(|c| short(&c.name) == name);
            let first = hits.next()?;
            hits.next().is_none().then_some(first)
        })
    }



    /// a class and every class it extends that the package defines, nearest first, then the name
    /// of the first one it does not (a runtime class, or one nobody defines)
    pub fn chain(&self, class: &str) -> (Vec<&'a ClassModel>, Option<String>) {
        let mut out = Vec::new();
        let mut cur = self.class_named(class);
        let mut seen = BTreeSet::new();
        while let Some(c) = cur {
            if !seen.insert(c.name.clone()) { break }
            out.push(c);
            if c.super_name.is_empty() || c.super_name == "Object" { return (out, None) }
            if self.runtime.provides(&c.super_name) { return (out, Some(c.super_name.clone())) }
            match self.class_named(&c.super_name) {
                Some(s) => cur = Some(s),
                None => return (out, Some(c.super_name.clone())),
            }
        }
        (out, None)
    }
}

/// content's fields live in the engine's state containers (off with `PEPTIDE_CELLS=0`)
pub fn cells_on() -> bool { std::env::var("PEPTIDE_CELLS").map(|v| v != "0").unwrap_or(true) }

/// A text as a string literal of the target language.
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A class's name without its package.
pub fn short(name: &str) -> &str { name.rsplit(['.', ':']).next().unwrap_or(name) }

/// Words the target language keeps for itself, which the source may use as names.
const RESERVED: &[&str] = &[
    "abstract", "break", "case", "cast", "catch", "class", "continue", "default", "do", "dynamic",
    "else", "enum", "extends", "extern", "false", "final", "for", "function", "if", "implements",
    "import", "in", "inline", "interface", "macro", "new", "null", "operator", "overload", "override",
    "package", "private", "public", "return", "static", "switch", "this", "throw", "true", "try",
    "typedef", "untyped", "using", "var", "while",
];

/// A source name as an identifier the target accepts: the same name wherever it can be.
pub fn ident(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else if c == '$' { 'S' } else { '_' }).collect();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) { s.insert(0, '_'); }
    if RESERVED.contains(&s.as_str()) { s.push('_'); }
    s
}

/// a landing the engine makes (a floor-touch listener whose only work was to take the move to a
/// label): the label, the animation it starts when the code that set it up is that clip's own,
/// and the stats the listener wrote, which are the landing animation's own
#[derive(Clone, Debug, Default)]
pub struct Landing {
    pub label: String,
    pub anim: Option<String>,
    pub attack: Vec<(String, f64)>,
    pub boxes: Vec<(u32, Vec<(String, f64)>)>,
}

/// a clip's frames as the engine's animations, for its own code's jumps by label
#[derive(Clone, Debug, Default)]
pub struct ClipJumps {
    /// every animation that shows the clip's frames
    pub anims: Vec<String>,
    /// label -> (the animation that shows it, whether it is that animation's first frame)
    pub labels: BTreeMap<String, (String, bool)>,
    /// the code its calls that turn the source's autolink on and off become, over `{fm}` (the
    /// fighter's engine handle), when the conversion made the autolink the engine's own
    pub autolink: Option<(String, String)>,
    /// the stats the attack the clip shows declares (one every attack it shows agrees on), for a
    /// write its code makes to read alongside its own
    pub declared: BTreeMap<String, f64>,
    /// the numbered sounds (`attackSound1_id`, `attackVoice2_id` ..) of the one attack the clip
    /// shows, for a play of them its code makes: none where the clip shows anything else, or where
    /// the attack stops its last sound or voice as it ends
    pub sounds: BTreeMap<String, String>,
}

