//! The compiler: the source model's trees to Haxe for Fraymakers' script interpreter.
//!
//! It knows AS3 and nothing about the game. A class becomes a class section in the source's own
//! shape (see `docs/CONVERTER_DESIGN.md`); a statement becomes the Haxe statement that means the
//! same; where the two languages disagree it calls the runtime's `AS3` layer
//! (`runtime/core/as3.hx`). What the game's API does is the runtime's business: a call on an API
//! object is written as the call the source makes, and the runtime answers it.
//!
//! The runtime's own files say which classes and members it provides (`Runtime::load`), so a name
//! the content inherits from an API class resolves to that member, and a name nothing provides is
//! reported instead of guessed at.

pub mod attack_stats;
pub mod autolink;
pub mod engine_anims;
pub mod events;
mod lower;
pub mod runtime;
pub mod suite;

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{ClassModel, MethodKind, SourceModel};
pub use runtime::Runtime;
pub use lower::quote;

/// A package, compiled.
pub struct Compiled {
    /// each class's section, by the class's source name
    pub classes: BTreeMap<String, String>,
    /// what could not be written, per class
    pub problems: Vec<String>,
    /// names content uses that neither it nor the runtime defines
    pub unknown: BTreeSet<String>,
}

/// what the whole package declares, for resolving names in any of its classes
pub struct Package<'a> {
    pub model: &'a SourceModel,
    pub runtime: &'a Runtime,
    /// source class name -> the identifier it is written as
    pub idents: BTreeMap<String, String>,
    /// names that are a getter or setter somewhere (content or runtime), read through the accessor
    pub accessors: BTreeSet<String>,
    /// names that are a plain field somewhere in the content
    pub plain_fields: BTreeSet<String>,
    /// every member name any class here or in the runtime defines: a call to a name outside it
    /// has nothing to answer it
    pub members: BTreeSet<String>,
    /// every member name a class of the package's own defines (a call by one of these names is
    /// content's, never an API call written in place)
    pub content_members: BTreeSet<String>,
    /// the fields whose values live in the engine's state containers (`me.__c_hp`), so a rollback
    /// restores them: every field a class of the package's own declares, by name, unless the name
    /// is also an accessor, a method, a static, or something of the runtime's
    pub stored: BTreeSet<String>,
}

impl<'a> Package<'a> {
    pub fn new(model: &'a SourceModel, runtime: &'a Runtime) -> Package<'a> {
        // a class is written by its own short name, unless two share it, or the engine or the
        // runtime already has a global by it (the content's own `CState` must not hide the engine's)
        let mut shorts: BTreeMap<String, usize> = BTreeMap::new();
        for n in model.classes.keys() { *shorts.entry(short(n).to_string()).or_default() += 1; }
        let idents = model.classes.keys().map(|n| {
            let s = short(n);
            let mut id = if shorts[s] > 1 { ident(n) } else { ident(s) };
            while runtime.takes(&id) { id.push('_'); }
            (n.clone(), id)
        }).collect();
        let mut accessors = BTreeSet::new();
        let mut plain_fields = BTreeSet::new();
        for c in model.classes.values().filter(|c| !runtime.provides(&c.name)) {
            for m in &c.methods {
                if matches!(m.kind, MethodKind::Getter | MethodKind::Setter) { accessors.insert(m.name.clone()); }
            }
            for f in &c.fields { plain_fields.insert(f.name.clone()); }
        }
        accessors.extend(runtime.accessors());
        let mut members: BTreeSet<String> = BTreeSet::new();
        for c in model.classes.values().filter(|c| !runtime.provides(&c.name)) {
            members.extend(c.methods.iter().map(|m| m.name.clone()));
            members.extend(c.fields.iter().map(|f| f.name.clone()));
        }
        let content_members = members.clone();
        // a name an object literal of the content's gives a value (`{ fn: f }`) is a member of
        // that object: a call by it (`data.fn()`) is answered by what the literal put there
        for c in model.classes.values().filter(|c| !runtime.provides(&c.name)) {
            for body in std::iter::once(&c.constructor.body).chain(c.methods.iter().map(|m| &m.function.body)) {
                crate::model::ast::walk_exprs(body, &mut |e| {
                    let crate::model::ast::Expr::Object(kv) = e else { return };
                    for (k, _) in kv {
                        if let crate::model::ast::Expr::Str(s) | crate::model::ast::Expr::Name(s) = k { members.insert(s.clone()); }
                    }
                });
            }
        }
        for c in runtime.classes.values() {
            members.extend(c.members.iter().cloned());
            members.extend(c.getters.iter().cloned());
        }
        let methods: BTreeSet<&str> = model.classes.values().flat_map(|c| c.methods.iter().map(|m| m.name.as_str())).collect();
        // a class's statics are its own values (constants, mostly) and stay plain: a container made
        // as a class's section runs is not one the engine keeps
        let statics: BTreeSet<&str> = model.classes.values().flat_map(|c| c.fields.iter().filter(|f| f.is_static).map(|f| f.name.as_str())).collect();
        let stored = if cells_on() {
            plain_fields.iter().filter(|n| !accessors.contains(*n) && !methods.contains(n.as_str()) && !statics.contains(n.as_str()) && !n.starts_with("__")
                && !runtime.classes.values().any(|c| c.members.contains(*n) || c.getters.contains(*n))
                && !runtime.globals.contains(*n)).cloned().collect()
        } else { BTreeSet::new() };
        Package { model, runtime, idents, accessors, plain_fields, members, content_members, stored }
    }

    /// the class a name refers to, by its full or short name
    pub fn class_named(&self, name: &str) -> Option<&'a ClassModel> {
        self.model.classes.get(name).or_else(|| {
            let mut hits = self.model.classes.values().filter(|c| short(&c.name) == name);
            let first = hits.next()?;
            hits.next().is_none().then_some(first)
        })
    }

    /// the identifier a class is written as: the package's own, or the runtime's
    pub fn class_ident(&self, name: &str) -> Option<String> {
        if self.runtime.provides(name) { return Some(ident(short(name))) }
        let c = self.class_named(name)?;
        self.idents.get(&c.name).cloned()
    }

    /// whether a class has a member: `None` when the class is neither the package's nor the
    /// runtime's (nothing to check it against)
    pub fn class_has(&self, class: &str, member: &str) -> Option<bool> {
        let known = self.class_named(class).is_some() || self.runtime.provides(class);
        if !known { return None }
        let (chain, base) = self.chain(class);
        let in_content = chain.iter().any(|c| c.fields.iter().any(|f| f.name == member) || c.methods.iter().any(|m| m.name == member));
        let base = base.or_else(|| self.runtime.provides(class).then(|| class.to_string()));
        Some(in_content || base.is_some_and(|b| self.runtime.has_member(&b, member)))
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

pub fn compile_package(model: &SourceModel, runtime: &Runtime) -> Compiled {
    let pkg = Package::new(model, runtime);
    let mut out = Compiled { classes: BTreeMap::new(), problems: Vec::new(), unknown: BTreeSet::new() };
    for c in model.classes.values() {
        if runtime.provides(&c.name) { continue }
        let mut w = lower::ClassWriter::new(&pkg, c);
        let text = w.class_section();
        out.problems.extend(w.problems.iter().map(|p| format!("{}: {p}", c.name)));
        out.unknown.extend(w.unknown.iter().cloned());
        out.classes.insert(c.name.clone(), text);
    }
    out
}

/// One class, compiled, with some of its methods written apart (`detached`): the class's section
/// without them, its problems and unknown names, and each detached method's function text.
#[allow(clippy::type_complexity)]
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
}

pub fn compile_class_detached(model: &SourceModel, runtime: &Runtime, class: &str, detached: &BTreeSet<String>, jumps: Option<ClipJumps>)
    -> Option<(String, Vec<String>, BTreeSet<String>, BTreeMap<String, String>, Vec<Landing>)> {
    let pkg = Package::new(model, runtime);
    let c = model.classes.get(class)?;
    let mut w = lower::ClassWriter::new(&pkg, c);
    w.detached = detached.clone();
    w.jumps = jumps;
    let text = w.class_section();
    Some((text, w.problems.clone(), w.unknown.clone(), w.detached_text.clone(), w.landings.clone()))
}

/// Every string the package's code writes, whole.
pub fn string_literals(model: &SourceModel) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for c in model.classes.values() {
        for m in &c.methods {
            crate::model::ast::walk_exprs(&m.function.body, &mut |e| if let crate::model::ast::Expr::Str(s) = e { out.insert(s.clone()); });
        }
    }
    out
}

/// The API calls a package's code can reach by a name it builds as it runs (`o["get" + kind +
/// "Stat"]`), as functions of the object: a name it writes whole, or one that starts with one
/// of its strings and ends with another. None: the package names none. The table is a
/// `DATA.api` entry per name, which `AS3.at` answers an engine object's missing member from.
pub fn api_lookup_table(model: &SourceModel, runtime: &Runtime) -> Option<String> {
    let lits: Vec<String> = string_literals(model).into_iter().filter(|s| s.len() >= 3).collect();
    let formed = |n: &str| lits.iter().any(|l| l == n)
        || lits.iter().any(|a| n.starts_with(a.as_str()) && n.len() > a.len()
            && lits.iter().any(|b| b != a && n.ends_with(b.as_str()) && a.len() + b.len() <= n.len()));
    let names: Vec<String> = runtime.calls.keys().filter(|n| formed(n)).cloned().collect();
    if names.is_empty() { return None }
    let pkg = Package::new(model, runtime);
    let c = model.classes.values().next()?;
    let mut w = lower::ClassWriter::new(&pkg, c);
    let args: Vec<String> = (0..4).map(|i| format!("__a{i}")).collect();
    let mut out = String::from("DATA.api = new haxe.ds.StringMap();\n");
    for n in names {
        if let Some(body) = w.api_text("__r", &n, &args) {
            out.push_str(&format!("DATA.api.set({}, function(__r, ?__a0, ?__a1, ?__a2, ?__a3) {{ return {body}; }});\n", quote(&n)));
        }
    }
    Some(out)
}

/// A lone function body, compiled outside any class: what the language suite runs.
pub fn compile_function(f: &crate::model::ast::Function, runtime: &Runtime) -> (String, Vec<String>) {
    let model = SourceModel::empty();
    let pkg = Package::new(&model, runtime);
    let class = ClassModel::empty("");
    let mut w = lower::ClassWriter::new(&pkg, &class);
    let text = w.function(f, 0, None);
    (text, w.problems.clone())
}
