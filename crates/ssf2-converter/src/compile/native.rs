//! An object's own code written as plain Fraymakers code.
//!
//! A projectile's package ships a class for the clip it shows. When everything that class does is
//! something Fraymakers says itself (moving, its boxes, listening for a hit, a jump by label), the
//! class is written straight into the object's script: its fields are the engine's state
//! containers, its methods are the script's functions, its frame scripts are functions the
//! entity's keyframes call, and every API call is the engine's own (`runtime/native_calls.jsonc`).
//! Nothing of the runtime comes along.
//!
//! The writer is strict: anything it has no plain way to say (a closure, a clip's children, a
//! call with no row) is a reason, and the object stays on the runtime with that reason reported.
//! The question is always what the code does, never what the object is.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use crate::model::ast::{BinOp, Conv, Expr, Function, Place, Stmt, UnOp};
use crate::model::{ClassModel, MethodKind, SourceModel};

use super::{ident, quote, short, ClipJumps};

// ---- the call table ---------------------------------------------------------------------------

/// a row of `native_calls.jsonc`: the engine's code for a call, and what its value is
#[derive(Debug, Clone, Default)]
struct Row { code: String, returns: Ty }

#[derive(Debug, Default)]
struct Calls {
    rows: BTreeMap<String, Row>,
    /// a projectile stat written by its code, as the engine's own stat (`{v}` the value)
    projectile_stats: BTreeMap<String, String>,
}

fn calls() -> &'static Calls {
    static T: OnceLock<Calls> = OnceLock::new();
    T.get_or_init(|| {
        let Some(dir) = super::runtime::Runtime::locate() else { return Calls::default() };
        let Ok(text) = std::fs::read_to_string(dir.join("native_calls.jsonc")) else { return Calls::default() };
        let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text))
            .unwrap_or_else(|e| panic!("runtime/native_calls.jsonc is malformed: {e}"));
        let mut t = Calls::default();
        for (k, v) in json.as_object().into_iter().flatten() {
            if k == "projectile_stats" {
                t.projectile_stats = v.as_object().into_iter().flatten()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
                continue;
            }
            let row = match v {
                serde_json::Value::String(s) => Row { code: s.clone(), returns: Ty::Any },
                serde_json::Value::Object(o) => Row {
                    code: o.get("code").and_then(|c| c.as_str()).unwrap_or_default().to_string(),
                    returns: Ty::named(o.get("returns").and_then(|r| r.as_str()).unwrap_or("")),
                },
                _ => continue,
            };
            t.rows.insert(k.clone(), row);
        }
        t
    })
}

// ---- what a value is ----------------------------------------------------------------------------

/// what a value is, as far as a test of it needs to know: this engine passes a test only on a
/// literal `true`, where the source tests any value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ty { Bool, Num, Str, Object, #[default] Any }

impl Ty {
    fn named(n: &str) -> Ty {
        match n { "bool" => Ty::Bool, "number" => Ty::Num, "string" => Ty::Str, "object" => Ty::Object, _ => Ty::Any }
    }
    fn declared(t: Option<&str>) -> Ty {
        match t.map(short) { Some("Number" | "int" | "uint") => Ty::Num, Some("Boolean") => Ty::Bool, Some("String") => Ty::Str, _ => Ty::Any }
    }
}

/// what a field or variable holds: the object itself, its clip (a copy of `this` the decompiler
/// keeps for an increment), or a value kept in a state container
#[derive(Debug, Clone, PartialEq)]
enum Holds { Itself, Clip, Value(Ty) }

// ---- what the writer is given, and what it writes -----------------------------------------------

/// the object to write
pub struct Object<'a> {
    pub model: &'a SourceModel,
    /// the classes whose code it runs: the classes of the clips its animations show
    pub classes: Vec<&'a ClassModel>,
    /// each class's labels as the engine's animations (`emit::label_jumps`)
    pub jumps: &'a BTreeMap<String, ClipJumps>,
    pub runtime: &'a super::runtime::Runtime,
    /// a stat its declaration states, as code, where every declaration of its clip agrees on it
    pub declared: &'a dyn Fn(&str) -> Option<String>,
    /// the variable that switches off what a declared stat has it do each frame (`projectile_stats.jsonc`'s
    /// `switch`), by stat
    pub switches: &'a BTreeMap<String, String>,
    /// an attack stat its owner declares, as a function of the owner over the owner's animations
    /// (name, then its definition)
    pub owner_stat: &'a dyn Fn(&str) -> Option<(String, String)>,
    /// an effect it can make, by name: the engine's effect stats for it, and the engine frame it
    /// goes on where its clip takes itself off before its animation ends
    pub effect: &'a dyn Fn(&str) -> Option<(String, Option<u32>)>,
    pub size: f64,
    pub speed: f64,
    pub accel: f64,
}

/// the object's code, ready to go into its script
#[derive(Debug, Default)]
pub struct Written {
    /// its state containers, one line each
    pub fields: Vec<String>,
    /// its functions: its methods, its frame scripts, and the helpers they use
    pub functions: Vec<String>,
    /// the call each frame with code makes, by (clip symbol, 1-based frame)
    pub frames: BTreeMap<(u16, u32), String>,
}

/// Write the object's classes as Fraymakers code, or say why they cannot be.
pub fn write(o: &Object) -> Result<Written, String> {
    let mut w = Writer::new(o)?;
    w.write()
}

// ---- the writer ---------------------------------------------------------------------------------

struct Writer<'o, 'a> {
    o: &'o Object<'a>,
    /// field -> (its name in the script, what it holds)
    fields: BTreeMap<String, (String, Holds)>,
    /// method -> its function's name
    methods: BTreeMap<String, String>,
    /// the class being written, for its jumps by label
    class: String,
    /// the current function's registers, by number: (name, what it holds)
    locals: BTreeMap<u32, (String, Holds)>,
    /// helpers the code needs, by name
    helpers: BTreeMap<String, String>,
    /// state containers the helpers need, by name
    kept: BTreeMap<String, String>,
    /// fields some code reads or writes
    read: BTreeSet<String>,
    /// the call being written is a whole statement (nothing takes its value)
    top: bool,
    /// effects made so far in the function, for their variables' names
    effects: usize,
}

impl<'o, 'a> Writer<'o, 'a> {
    fn new(o: &'o Object<'a>) -> Result<Self, String> {
        let mut w = Writer { o, fields: BTreeMap::new(), methods: BTreeMap::new(), class: String::new(),
                             locals: BTreeMap::new(), helpers: BTreeMap::new(), kept: BTreeMap::new(), read: BTreeSet::new(), top: false, effects: 0 };
        // every class's names, each kept as it is unless two classes share one
        let mut taken: BTreeSet<String> = ["self", "match", "stage", "initialize", "update", "onTeardown", "onKill", "life", "age", "started", "freshHit"]
            .into_iter().map(String::from).collect();
        let name_for = |n: &str, class: &ClassModel, taken: &mut BTreeSet<String>| {
            let mut id = ident(n);
            if !taken.insert(id.clone()) { id = format!("{}_{}", ident(short(&class.name)), id); taken.insert(id.clone()); }
            id
        };
        for c in &o.classes {
            if !matches!(short(&c.super_name), "MovieClip" | "Sprite") { return Err(format!("{} extends {}, whose code is not written here", c.name, c.super_name)) }
            if !c.unreadable.is_empty() { return Err(format!("{} has code that could not be read", c.name)) }
            if c.fields.iter().any(|f| f.is_static) || c.methods.iter().any(|m| m.is_static) { return Err(format!("{} has statics", c.name)) }
            if c.methods.iter().any(|m| m.kind != MethodKind::Method) { return Err(format!("{} has accessors", c.name)) }
            // the constructor only registers the frame scripts
            for s in &c.constructor.body {
                let fine = match s {
                    Stmt::Return(None) | Stmt::Expr(Expr::SuperCall(..)) => true,
                    Stmt::Expr(Expr::Call(f, _)) => matches!(&**f, Expr::Member(t, m) if matches!(**t, Expr::This) && m == "addFrameScript")
                        || matches!(&**f, Expr::Name(m) if m == "addFrameScript"),
                    _ => false,
                };
                if !fine { return Err(format!("{}'s constructor does more than register its frames", c.name)) }
            }
            for m in &c.methods { let id = name_for(&m.name, c, &mut taken); w.methods.insert(m.name.clone(), id); }
            for f in &c.fields {
                let holds = if w.holds_itself(c, &f.name) { Holds::Itself } else { Holds::Value(Ty::declared(f.type_name.as_deref())) };
                let id = if holds == Holds::Itself { "self".to_string() } else { name_for(&f.name, c, &mut taken) };
                w.fields.insert(f.name.clone(), (id, holds));
            }
        }
        Ok(w)
    }

    /// whether every value the class's code puts in a field is the object itself
    /// (`self.self = SSF2API.getProjectile(this)`)
    fn holds_itself(&self, c: &ClassModel, field: &str) -> bool {
        let mut any = false;
        let mut all = true;
        for m in &c.methods {
            walk(&m.function.body, &mut |s| if let Stmt::Assign(Place::Member(t, f), v) = s {
                if matches!(t, Expr::This) && f == field {
                    any = true;
                    all &= is_itself(v);
                }
            });
        }
        any && all
    }

    fn write(&mut self) -> Result<Written, String> {
        let mut out = Written::default();
        let classes = self.o.classes.clone();
        for c in &classes {
            self.class = c.name.clone();
            let frame_of: BTreeMap<&str, u32> = c.frame_scripts.iter().map(|fs| (fs.method.as_str(), fs.frame)).collect();
            for m in &c.methods {
                let name = self.methods[&m.name].clone();
                let text = self.function(&name, &m.function).map_err(|e| format!("{}.{}: {e}", short(&c.name), m.name))?;
                out.functions.push(text);
                if let (Some(f), Some(sym)) = (frame_of.get(m.name.as_str()), c.symbol) {
                    out.frames.insert((sym, f + 1), format!("{name}();"));
                }
            }
        }
        // the fields its code uses, as the engine's containers
        for (field, (id, holds)) in &self.fields {
            let Holds::Value(ty) = holds else { continue };
            if !self.read.contains(field) { continue }
            let c = classes.iter().find_map(|c| c.fields.iter().find(|f| &f.name == field));
            let default = c.and_then(|f| f.default.as_ref()).and_then(literal);
            let whole = c.is_some_and(|f| matches!(f.type_name.as_deref().map(short), Some("int" | "uint")));
            let make = match ty {
                Ty::Num if whole => format!("self.makeInt({})", default.unwrap_or_else(|| "0".into())),
                Ty::Num => format!("self.makeFloat({})", default.unwrap_or_else(|| "0".into())),
                Ty::Bool => format!("self.makeBool({})", default.unwrap_or_else(|| "false".into())),
                Ty::Str => format!("self.makeString({})", default.unwrap_or_else(|| "null".into())),
                _ => format!("self.makeObject({})", default.unwrap_or_else(|| "null".into())),
            };
            out.fields.push(format!("var {id} = {make};"));
        }
        out.fields.extend(self.kept.values().cloned());
        out.functions.extend(self.helpers.values().cloned());
        Ok(out)
    }

    // ---- functions and statements ---------------------------------------------------------------

    fn function(&mut self, name: &str, f: &Function) -> Result<String, String> {
        if f.rest { return Err("takes the rest of its arguments".into()) }
        self.locals.clear();
        self.effects = 0;
        let debug: BTreeMap<u32, String> = f.local_names.iter().cloned().collect();
        let mut params = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            let n = ident(&p.name);
            params.push(format!("?{n}"));
            self.locals.insert(i as u32 + 1, (n, Holds::Value(Ty::declared(p.type_name.as_deref()))));
        }
        // an event a listener is handed is this engine's: it is read only through the fields of the
        // source's record this engine's event has (`record`)
        let nparams = f.params.len() as u32;
        let mut reads = 0usize;
        let mut through = 0usize;
        fn count(list: &[Stmt], nparams: u32, reads: &mut usize, through: &mut usize) {
            crate::model::ast::walk_exprs(list, &mut |e| {
                if let Expr::Local(r) = e { if *r >= 1 && *r <= nparams { *reads += 1 } }
                if let Expr::Member(d, _) = e { if let Expr::Member(a, n) = &**d { if n == "data" && matches!(**a, Expr::Local(r) if r >= 1 && r <= nparams) { *through += 1 } } }
            });
        }
        count(&f.body, nparams, &mut reads, &mut through);
        if reads > through { return Err("reads its arguments".into()) }
        if !f.activation.is_empty() { return Err("a closure captures its variables".into()) }
        // (the decompiler's leftovers tidied first)
        let mut body = f.body.clone();
        tidy(&mut body);
        // its variables, at the top, as the source scopes them to the function (not a copy of its
        // clip, which reads as its own fields)
        let mut regs = BTreeSet::new();
        walk_places(&body, &mut |r| { if r > nparams { regs.insert(r); } });
        let mut copies = BTreeSet::new();
        walk(&body, &mut |s| if let Stmt::Assign(Place::Local(r), v) = s { if matches!(v, Expr::This) { copies.insert(*r); } });
        walk(&body, &mut |s| if let Stmt::Assign(Place::Local(r), v) = s { if !matches!(v, Expr::This) { copies.remove(r); } });
        let mut decl = Vec::new();
        for r in &regs {
            if copies.contains(r) {
                let n = debug.get(r).map(|s| ident(s)).unwrap_or_else(|| format!("v{r}"));
                self.locals.insert(*r, (n, Holds::Clip));
                continue;
            }
            let n = debug.get(r).map(|s| ident(s)).unwrap_or_else(|| format!("v{r}"));
            decl.push(format!("\tvar {n} = null;"));
            self.locals.insert(*r, (n, Holds::Value(Ty::Any)));
        }
        // (a body's closing `return;` says nothing)
        if matches!(body.last(), Some(Stmt::Return(None))) { body.pop(); }
        let mut lines = decl;
        for s in &body { lines.extend(self.stmt(s, 1)?); }
        Ok(format!("function {name}({}) {{\n{}\n}}", params.join(", "), lines.join("\n")).replace("{\n\n}", "{\n}"))
    }

    fn block(&mut self, list: &[Stmt], depth: usize) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for s in list { out.extend(self.stmt(s, depth)?); }
        Ok(out)
    }

    fn stmt(&mut self, s: &Stmt, depth: usize) -> Result<Vec<String>, String> {
        let tab = "\t".repeat(depth);
        Ok(match s {
            Stmt::Expr(e) => {
                let t = self.statement_call(e)?;
                if t.is_empty() { vec![] } else { t.lines().map(|l| format!("{tab}{l}")).collect() }
            }
            Stmt::Assign(p, v) => self.assign(p, v)?.into_iter().map(|l| format!("{tab}{l}")).collect(),
            Stmt::If(c, then, els) => {
                let test = self.test(c)?;
                // (a test that always passes is its body)
                if test == "true" { return self.block(then, depth) }
                let mut out = vec![format!("{tab}if ({test}) {{")];
                out.extend(self.block(then, depth + 1)?);
                if !els.is_empty() {
                    out.push(format!("{tab}}} else {{"));
                    out.extend(self.block(els, depth + 1)?);
                }
                out.push(format!("{tab}}}"));
                out
            }
            Stmt::While(c, b) => {
                let mut out = vec![format!("{tab}while ({}) {{", self.test(c)?)];
                out.extend(self.block(b, depth + 1)?);
                out.push(format!("{tab}}}"));
                out
            }
            Stmt::Loop(b) => {
                let mut out = vec![format!("{tab}while (true) {{")];
                out.extend(self.block(b, depth + 1)?);
                out.push(format!("{tab}}}"));
                out
            }
            Stmt::DoWhile(b, c) => {
                let mut out = vec![format!("{tab}do {{")];
                out.extend(self.block(b, depth + 1)?);
                out.push(format!("{tab}}} while ({});", self.test(c)?));
                out
            }
            Stmt::Break => vec![format!("{tab}break;")],
            Stmt::Continue => vec![format!("{tab}continue;")],
            Stmt::Return(None) => vec![format!("{tab}return;")],
            Stmt::Return(Some(e)) => vec![format!("{tab}return {};", self.expr(e)?.0)],
            Stmt::Switch(v, cases, default) => {
                let mut out = vec![format!("{tab}switch ({}) {{", self.expr(v)?.0)];
                for (values, body) in cases {
                    let vs: Result<Vec<String>, String> = values.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                    out.push(format!("{tab}\tcase {}:", vs?.join(", ")));
                    out.extend(self.block(strip_break(body)?, depth + 2)?);
                }
                if !default.is_empty() {
                    out.push(format!("{tab}\tdefault:"));
                    out.extend(self.block(strip_break(default)?, depth + 2)?);
                }
                out.push(format!("{tab}}}"));
                out
            }
            Stmt::Throw(_) | Stmt::Try(..) => return Err("throws or catches".into()),
            Stmt::Unreadable(why) => return Err(format!("unreadable code ({why})")),
        })
    }

    /// a statement that is a call: what it does, or nothing for one that does nothing here
    fn statement_call(&mut self, e: &Expr) -> Result<String, String> {
        if let Expr::Call(f, args) = e {
            let own = match &**f {
                Expr::Member(t, m) if matches!(**t, Expr::This) => Some(m),
                Expr::Name(m) if !self.methods.contains_key(m) => Some(m),
                _ => None,
            };
            if let Some(m) = own {
                {
                    match (m.as_str(), args.as_slice()) {
                        // the engine plays the clip the object shows on every frame, whatever a stop on
                        // it says (the runtime does the same: Projectiles.beforeTick)
                        ("stop" | "play", []) => return Ok(String::new()),
                        ("gotoAndStop" | "gotoAndPlay", [Expr::Str(label)]) => return self.jump(label),
                        _ => {}
                    }
                }
            }
        }
        self.top = true;
        let t = self.expr(e)?.0;
        self.top = false;
        // (one that ends in a block, or says nothing, is whole as it is)
        Ok(if t.is_empty() || t.ends_with('}') || t.ends_with(';') { t } else { format!("{t};") })
    }

    fn assign(&mut self, p: &Place, v: &Expr) -> Result<Vec<String>, String> {
        match p {
            Place::Member(Expr::This, f) => {
                let (id, holds) = self.fields.get(f).cloned().ok_or_else(|| format!("writes {f}, which is not its field"))?;
                if holds == Holds::Itself { return Ok(vec![]) }
                let (val, ty) = self.expr(v)?;
                // what it learns of the field's value
                if let Some(e) = self.fields.get_mut(f) { if e.1 == Holds::Value(Ty::Any) && ty != Ty::Any { e.1 = Holds::Value(ty) } }
                let _ = id;
                self.read.insert(f.clone());
                Ok(vec![format!("{}.set({val});", self.fields[f].0)])
            }
            // (a copy of its clip, as the decompiler keeps one for `this.n++`: its reads are its own)
            Place::Local(r) if matches!(v, Expr::This) => {
                let (n, _) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                self.locals.insert(*r, (n, Holds::Clip));
                Ok(vec![])
            }
            Place::Member(Expr::Local(r), f) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Clip) => self.assign(&Place::Member(Expr::This, f.clone()), v),
            Place::Local(r) => {
                let (val, ty) = self.expr(v)?;
                let (n, _) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                if ty != Ty::Any { self.locals.insert(*r, (n.clone(), Holds::Value(ty))); }
                Ok(vec![format!("{n} = {val};")])
            }
            _ => Err("writes somewhere other than its fields and variables".into()),
        }
    }

    /// a jump by label, as the engine's animation that shows it
    fn jump(&mut self, label: &str) -> Result<String, String> {
        let (anim, start) = self.o.jumps.get(&self.class).and_then(|j| j.labels.get(label)).cloned()
            .ok_or_else(|| format!("jumps to {label}, which no animation shows"))?;
        Ok(if start {
            format!("self.playAnimation({});", quote(&anim))
        } else {
            // (a label inside an animation: that animation, at the label)
            format!("if (self.getAnimation() != {a}) self.playAnimation({a});\nself.playFrameLabel({});", quote(label), a = quote(&anim))
        })
    }

    // ---- tests and values ---------------------------------------------------------------------

    /// a value as a test, the source's way: false for null, false, 0, NaN and ""
    fn test(&mut self, e: &Expr) -> Result<String, String> {
        let (t, ty) = self.expr(e)?;
        Ok(match ty {
            Ty::Bool => t,
            Ty::Num => format!("{t} != 0"),
            // (the object itself always is)
            Ty::Object if t == "self" => "true".into(),
            Ty::Object => format!("{t} != null"),
            Ty::Str => format!("({t} != null && {t} != \"\")"),
            Ty::Any => format!("({t} != null && {t} != false && {t} != 0)"),
        })
    }

    fn expr(&mut self, e: &Expr) -> Result<(String, Ty), String> {
        Ok(match e {
            Expr::Null | Expr::Undefined => ("null".into(), Ty::Object),
            Expr::Bool(b) => (b.to_string(), Ty::Bool),
            Expr::Num(n) => (number(*n), Ty::Num),
            Expr::Int(n) => (n.to_string(), Ty::Num),
            Expr::Str(s) => (quote(s), Ty::Str),
            Expr::Local(r) => {
                let (n, h) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                if h == Holds::Clip { return Err("uses its clip as a value".into()) }
                (n, match h { Holds::Itself | Holds::Clip => Ty::Object, Holds::Value(t) => t })
            }
            Expr::Member(t, f) if matches!(**t, Expr::This) => return self.own(f),
            Expr::Member(t, f) if matches!(&**t, Expr::Local(r) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Clip)) => return self.own(f),
            // a field of the event a listener is handed
            Expr::Member(d, field) if matches!(&**d, Expr::Member(a, n) if n == "data" && matches!(**a, Expr::Local(_))) => {
                let Expr::Member(a, _) = &**d else { unreachable!() };
                let Expr::Local(r) = &**a else { unreachable!() };
                let (arg, _) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                return record(&arg, field);
            }
            Expr::Name(n) if self.fields.contains_key(n) || self.methods.contains_key(n) => return self.own(n),
            Expr::Member(o, n) if matches!(&**o, Expr::Name(c) if c == "Math") => (format!("Math.{n}"), Ty::Num),
            Expr::Call(f, args) => return self.call(f, args),
            Expr::Unary(UnOp::Not, x) => (format!("!{}", paren(&self.test(x)?)), Ty::Bool),
            Expr::Unary(UnOp::Neg, x) => { let (t, _) = self.expr(x)?; (format!("-{}", paren(&t)), Ty::Num) }
            Expr::Binary(op, a, b) => return self.binary(*op, a, b),
            Expr::And(a, b) => {
                let (a, b) = (self.test(a)?, self.test(b)?);
                (match (a.as_str(), b.as_str()) { ("true", _) => b, (_, "true") => a, _ => format!("{} && {}", paren(&a), paren(&b)) }, Ty::Bool)
            }
            Expr::Or(a, b) => {
                let (a, b) = (self.test(a)?, self.test(b)?);
                (match (a.as_str(), b.as_str()) { ("true", _) | (_, "true") => "true".into(), _ => format!("{} || {}", paren(&a), paren(&b)) }, Ty::Bool)
            }
            Expr::Cond(c, a, b) => {
                let c = self.test(c)?;
                let (a, ta) = self.expr(a)?;
                let (b, tb) = self.expr(b)?;
                (format!("({c} ? {a} : {b})"), if ta == tb { ta } else { Ty::Any })
            }
            Expr::Coerce(t, x) => { let (v, ty) = self.expr(x)?; (v, if ty == Ty::Any { Ty::declared(Some(t)) } else { ty }) }
            Expr::Convert(c, x) => match c {
                Conv::Number | Conv::Object => self.expr(x)?,
                Conv::Int | Conv::Uint => { let (v, _) = self.expr(x)?; (format!("Std.int({v})"), Ty::Num) }
                Conv::Boolean => (self.test(x)?, Ty::Bool),
                Conv::String => { let (v, _) = self.expr(x)?; (format!("Std.string({v})"), Ty::Str) }
            },
            Expr::Array(items) => {
                let vs: Result<Vec<String>, String> = items.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                (format!("[{}]", vs?.join(", ")), Ty::Object)
            }
            Expr::Function(_) => return Err("makes a closure".into()),
            Expr::Index(..) => return Err("reads by index".into()),
            Expr::New(..) => return Err("makes an object".into()),
            Expr::Assign(..) => return Err("uses an assignment as a value".into()),
            other => return Err(describe(other)),
        })
    }

    /// one of its own fields or methods, read
    fn own(&mut self, n: &str) -> Result<(String, Ty), String> {
        if let Some((id, holds)) = self.fields.get(n).cloned() {
            self.read.insert(n.to_string());
            return Ok(match holds { Holds::Itself | Holds::Clip => (id, Ty::Object), Holds::Value(t) => (format!("{id}.get()"), t) });
        }
        if let Some(id) = self.methods.get(n) { return Ok((id.clone(), Ty::Object)) }
        Err(format!("reads {n}, which is not its own"))
    }

    fn binary(&mut self, op: BinOp, a: &Expr, b: &Expr) -> Result<(String, Ty), String> {
        let (x, tx) = self.expr(a)?;
        let (y, ty) = self.expr(b)?;
        let (sym, out) = match op {
            BinOp::Add => ("+", if tx == Ty::Str || ty == Ty::Str { Ty::Str } else if tx == Ty::Num && ty == Ty::Num { Ty::Num } else { Ty::Any }),
            BinOp::Sub => ("-", Ty::Num), BinOp::Mul => ("*", Ty::Num), BinOp::Div => ("/", Ty::Num), BinOp::Mod => ("%", Ty::Num),
            BinOp::Eq | BinOp::StrictEq => ("==", Ty::Bool), BinOp::Ne | BinOp::StrictNe => ("!=", Ty::Bool),
            BinOp::Lt => ("<", Ty::Bool), BinOp::Le => ("<=", Ty::Bool), BinOp::Gt => (">", Ty::Bool), BinOp::Ge => (">=", Ty::Bool),
            BinOp::BitAnd => ("&", Ty::Num), BinOp::BitOr => ("|", Ty::Num), BinOp::BitXor => ("^", Ty::Num),
            BinOp::Shl => ("<<", Ty::Num), BinOp::Shr => (">>", Ty::Num),
            _ => return Err(format!("uses {op:?}")),
        };
        Ok((format!("{} {sym} {}", paren(&x), paren(&y)), out))
    }

    // ---- calls --------------------------------------------------------------------------------

    fn call(&mut self, f: &Expr, args: &[Expr]) -> Result<(String, Ty), String> {
        let top = std::mem::replace(&mut self.top, false);
        let Expr::Member(recv, m) = f else {
            // one of its own methods, called bare
            if let Expr::Name(n) = f {
                if let Some(id) = self.methods.get(n).cloned() { return self.own_call(&id, args) }
                return Err(format!("calls {n} on its clip"));
            }
            return Err(format!("calls {}", describe(f)));
        };
        // one of its own methods
        if matches!(**recv, Expr::This) {
            if let Some(id) = self.methods.get(m).cloned() { return self.own_call(&id, args) }
            return Err(format!("calls {m} on its clip"));
        }
        // the API's own questions about the object
        if let Expr::Name(c) = &**recv {
            match (short(c), m.as_str(), args) {
                (_, "getProjectile" | "getItem" | "getEnemy", [Expr::This]) if c.contains("PeptideAPI") => return Ok(("self".into(), Ty::Object)),
                (_, "isReady", []) if c.contains("PeptideAPI") => return Ok(("true".into(), Ty::Bool)),
                // the API's own calls with a row (`PeptideAPI.print`)
                (_, _, _) if c.contains("PeptideAPI") => {
                    let row = calls().rows.get(&format!("PeptideAPI.{m}")).cloned().ok_or_else(|| format!("calls PeptideAPI.{m}, which has no row in native_calls.jsonc"))?;
                    if row.code.is_empty() && top { return Ok((String::new(), Ty::Any)) }
                    return self.fill(&row, "self", args, m);
                }
                ("Math", _, _) => {
                    let vs: Result<Vec<String>, String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                    return Ok((format!("Math.{m}({})", vs?.join(", ")), Ty::Num));
                }
                _ => {}
            }
        }
        let (who, _) = self.expr(recv)?;
        if top && who == "self" && matches!(m.as_str(), "createVfx" | "createVfxOverlay") { return self.effect(args) }
        self.api(&who, m, args)
    }

    /// an effect it makes and lets go of, named outright, with its options written out: the
    /// engine's own effect, where the source puts it
    fn effect(&mut self, args: &[Expr]) -> Result<(String, Ty), String> {
        let Some(Expr::Str(name)) = args.first() else { return Err("makes an effect named as it runs".into()) };
        let (stats, gone) = (self.o.effect)(name).ok_or_else(|| format!("makes the effect {name}, which it has no clip for"))?;
        let mut opts: BTreeMap<String, &Expr> = BTreeMap::new();
        match args.get(1) {
            None | Some(Expr::Null) | Some(Expr::Undefined) => {}
            Some(Expr::Object(kv)) => for (k, v) in kv {
                let Expr::Str(k) = k else { return Err("makes an effect with options named as it runs".into()) };
                if !matches!(k.as_str(), "x" | "y" | "scaleX" | "scaleY" | "rotation" | "behind" | "parentLock" | "syncHitStun") {
                    return Err(format!("makes an effect with its {k} set"));
                }
                opts.insert(k.clone(), v);
            },
            _ => return Err("makes an effect with options worked out as it runs".into()),
        }
        let flag = |k: &str| -> Result<bool, String> { match opts.get(k) { None => Ok(false), Some(Expr::Bool(b)) => Ok(*b), _ => Err(format!("makes an effect with {k} worked out as it runs")) } };
        let (locked, behind) = (flag("parentLock")?, flag("behind")?);
        let num = |w: &mut Self, k: &str| -> Result<Option<String>, String> { opts.get(k).map(|e| Ok(w.expr(e)?.0)).transpose() };
        let (ox, oy) = (num(self, "x")?, num(self, "y")?);
        let size = number(self.o.size);
        let off = |o: &Option<String>| o.as_ref().map(|t| fold(&format!("{} * {size}", paren(t)))).unwrap_or_else(|| "0".into());
        let mut st = vec![stats];
        if let Some(n) = gone { st.push(format!("timeout: {}", 2 * (n.max(1) - 1))); }
        for k in ["scaleX", "scaleY"] { if let Some(t) = num(self, k)? { st.push(format!("{k}: {t}")); } }
        if behind { st.push("layer: VfxLayer.CHARACTERS_BACK".into()); }
        let mut after = Vec::new();
        if locked {
            // riding the object, which mirrors the offset with it
            st.push(format!("x: self.flipX({}), y: {}, relativeWith: true, resizeWith: true, flipWith: true", off(&ox), off(&oy)));
        } else {
            if ox.is_some() { after.push(format!("setX(self.getX() + {})", off(&ox))); }
            if oy.is_some() { after.push(format!("setY(self.getY() + {})", off(&oy))); }
        }
        if let Some(t) = num(self, "rotation")? { after.push(format!("setRotation({t})")); }
        let make = format!("match.createVfx(new VfxStats({{ {} }}), self)", st.join(", "));
        if after.is_empty() { return Ok((make, Ty::Any)) }
        self.effects += 1;
        let v = if self.effects == 1 { "effect".to_string() } else { format!("effect{}", self.effects) };
        let set: Vec<String> = after.iter().map(|a| format!("{v}.{a};")).collect();
        Ok((format!("var {v} = {make};\nif ({v} != null) {{ {} }}", set.join(" ")), Ty::Any))
    }

    fn own_call(&mut self, id: &str, args: &[Expr]) -> Result<(String, Ty), String> {
        let vs: Result<Vec<String>, String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect();
        Ok((format!("{id}({})", vs?.join(", ")), Ty::Any))
    }

    /// an API call on an engine object (`who`: `self`, or the variable holding another)
    fn api(&mut self, who: &str, m: &str, args: &[Expr]) -> Result<(String, Ty), String> {
        let mine = who == "self";
        match (m, args) {
            ("addEventListener" | "removeEventListener", [ev, handler, ..]) => return self.listener(who, m, ev, handler),
            ("addTimer", [frames, repeat, f, rest @ ..]) if mine => return self.timer(frames, repeat, f, rest),
            ("removeTimer", [f]) if mine => {
                let id = self.timer_of(f)?;
                return Ok((format!("self.removeTimer({id}.get())"), Ty::Any));
            }
            ("playFrame", [Expr::Str(label)]) if mine => return Ok((self.jump(label)?.trim_end_matches(';').to_string(), Ty::Any)),
            ("updateProjectileStats", [Expr::Object(pairs)]) if mine => return self.projectile_stats(pairs),
            ("getProjectileStat", [Expr::Str(k)]) if mine => {
                return (self.o.declared)(k).map(|v| (v, Ty::Any)).ok_or_else(|| format!("reads its stat {k}, which its declarations do not agree on"));
            }
            ("getHitboxStat", [i, Expr::Str(k)]) if mine => {
                let (field, scale) = hitbox_field(k)?;
                let i = box_index(i)?;
                let back = scale.map(|s| format!(" / {}", number(s))).unwrap_or_default();
                return Ok((format!("self.getHitboxStats({i}).{field}{back}"), Ty::Num));
            }
            ("updateHitboxStats", [i, Expr::Object(pairs)]) if mine => {
                let i = box_index(i)?;
                let mut set = Vec::new();
                for (k, v) in pairs {
                    let Expr::Str(k) = k else { return Err("writes a box stat by a name worked out as it runs".into()) };
                    if crate::mappings::character_hitbox_stats().no_counterpart.iter().any(|n| n == k) { continue }
                    let (field, scale) = hitbox_field(k)?;
                    let (v, _) = self.expr(v)?;
                    set.push(match scale { Some(s) => format!("{field}: {} * {}", paren(&v), number(s)), None => format!("{field}: {v}") });
                }
                return Ok((format!("self.updateHitboxStats({i}, {{ {} }})", set.join(", ")), Ty::Any));
            }
            ("getAnimationStat", [Expr::Str(k)]) if !mine => {
                let (name, def) = (self.o.owner_stat)(k).ok_or_else(|| format!("reads its owner's attack stat {k}, which no attack of its owner declares"))?;
                self.helpers.insert(name.clone(), def);
                return Ok((format!("{name}({who})"), Ty::Any));
            }
            _ => {}
        }
        let row = calls().rows.get(m).cloned().ok_or_else(|| format!("calls {m}, which has no row in native_calls.jsonc"))?;
        self.fill(&row, who, args, m)
    }

    /// a row filled in: the object it is asked of, its arguments, the units
    fn fill(&mut self, row: &Row, who: &str, args: &[Expr], m: &str) -> Result<(String, Ty), String> {
        let mut t = row.code.replace("{self}", who)
            .replace("{size}", &number(self.o.size)).replace("{speed}", &number(self.o.speed)).replace("{accel}", &number(self.o.accel));
        for (i, a) in args.iter().enumerate() {
            let (v, _) = self.expr(a)?;
            t = t.replace(&format!("{{{i}}}"), &paren(&v));
        }
        if t.contains("{0}") || t.contains("{1}") { return Err(format!("calls {m} with fewer arguments than its row takes")) }
        Ok((fold(&t), row.returns))
    }

    /// the container that holds the engine's id for the timer that runs one of its own methods
    fn timer_of(&mut self, f: &Expr) -> Result<String, String> {
        let name = match f {
            Expr::Member(t, n) if matches!(**t, Expr::This) => self.methods.get(n).cloned(),
            Expr::Name(n) => self.methods.get(n).cloned(),
            _ => None,
        }.ok_or("times something other than its own method")?;
        let id = format!("{name}Timer");
        self.kept.insert(id.clone(), format!("// the engine's timer running {name}, while there is one\nvar {id} = self.makeInt(-1);"));
        Ok(id)
    }

    /// a source timer (`createTimer(frames, repeats, fn)`): the engine's, of two engine frames to
    /// a source frame, forever for none. one function keeps one timer: starting it again restarts it
    fn timer(&mut self, frames: &Expr, repeat: &Expr, f: &Expr, rest: &[Expr]) -> Result<(String, Ty), String> {
        if !rest.is_empty() && !matches!(rest, [Expr::Null | Expr::Undefined]) { return Err("makes a timer with options".into()) }
        let id = self.timer_of(f)?;
        let name = id.trim_end_matches("Timer").to_string();
        let (n, _) = self.expr(frames)?;
        let every = match n.parse::<f64>() { Ok(v) => number((2.0 * v.max(1.0)).round()), Err(_) => format!("Std.int(2 * Math.max(1, {}))", paren(&n)) };
        let times = match self.expr(repeat)?.0 {
            t if t.parse::<f64>().is_ok_and(|v| v <= 0.0) => "-1".to_string(),
            t if t.parse::<f64>().is_ok() => t,
            t => format!("({t} <= 0 ? -1 : {t})"),
        };
        Ok((format!("self.removeTimer({id}.get());\n{id}.set(self.addTimer({every}, {times}, {name}, {{ persistent: true }}))"), Ty::Any))
    }

    /// a listener for an event the engine raises, on the engine's object, by one of its own methods
    fn listener(&mut self, who: &str, m: &str, ev: &Expr, handler: &Expr) -> Result<(String, Ty), String> {
        let name = super::events::named(self.o.runtime, ev).ok_or("listens for an event named as it runs")?;
        let engine = super::events::table().engine.get(&name).ok_or_else(|| format!("listens for {name}, which this engine does not raise"))?;
        let own = match handler {
            Expr::Member(t, n) if matches!(**t, Expr::This) => self.methods.get(n).cloned(),
            Expr::Name(n) => self.methods.get(n).cloned(),
            _ => None,
        };
        let h = match own {
            Some(h) => h,
            // one of the object's API calls, handed over as the listener (`self.self.destroy`): a
            // function that makes that call
            None => {
                let Expr::Member(o, n) = handler else { return Err("listens with something other than its own method".into()) };
                if self.expr(o)?.0 != "self" { return Err("listens with another object's call".into()) }
                let row = calls().rows.get(n).filter(|r| !r.code.contains("{0}")).cloned()
                    .ok_or_else(|| format!("listens with its own {n}, which has no row in native_calls.jsonc"))?;
                let name = format!("{}OnEvent", ident(n));
                let code = self.fill(&row, "self", &[], n)?.0;
                self.helpers.insert(name.clone(), format!("// its own {n}, as a listener\nfunction {name}(?e) {{\n\t{code};\n}}"));
                name
            }
        };
        if engine.contains("COLLIDE_FLOOR") {
            let speeds = self.o.classes.iter().flat_map(|c| c.methods.iter()).filter(|x| self.methods.get(&x.name) == Some(&h)).any(|x| {
                let mut reads = false;
                crate::model::ast::walk_exprs(&x.function.body, &mut |e| if let Expr::Member(_, n) = e { if matches!(n.as_str(), "getXSpeed" | "getYSpeed") { reads = true } });
                reads
            });
            if speeds { return Err("reads its speed as it touches the floor".into()) }
        }
        Ok((if m == "addEventListener" {
            format!("{who}.addEventListener({engine}, {h}, {{ persistent: true }})")
        } else {
            format!("{who}.removeEventListener({engine}, {h})")
        }, Ty::Any))
    }

    /// its projectile stats written by its code: the engine's own stat, or the switch of what a
    /// declared stat has it do each frame
    fn projectile_stats(&mut self, pairs: &[(Expr, Expr)]) -> Result<(String, Ty), String> {
        let mut out = Vec::new();
        for (k, v) in pairs {
            let Expr::Str(k) = k else { return Err("writes a stat by a name worked out as it runs".into()) };
            if let Some(var) = self.o.switches.get(k) {
                // off for nothing, on again for what it declares
                let on = match v {
                    Expr::Null | Expr::Undefined | Expr::Bool(false) => "false".to_string(),
                    Expr::Num(n) if *n == 0.0 => "false".to_string(),
                    Expr::Int(0) => "false".to_string(),
                    Expr::Bool(true) => "true".to_string(),
                    _ if (self.o.declared)(k).is_some_and(|d| Some(d) == literal(v)) => "true".to_string(),
                    _ => return Err(format!("writes {k} to something it does not declare")),
                };
                out.push(format!("{var} = {on}"));
                continue;
            }
            let t = calls().projectile_stats.get(k).ok_or_else(|| format!("writes its stat {k}, which has no row in native_calls.jsonc"))?.clone();
            let (val, _) = self.expr(v)?;
            out.push(fold(&t.replace("{v}", &paren(&val)).replace("{size}", &number(self.o.size))
                .replace("{speed}", &number(self.o.speed)).replace("{accel}", &number(self.o.accel))));
        }
        Ok((out.join(";\n"), Ty::Any))
    }
}

// ---- small pieces -----------------------------------------------------------------------------

fn is_itself(v: &Expr) -> bool {
    match v {
        Expr::Coerce(_, x) | Expr::Convert(_, x) => is_itself(x),
        Expr::Call(f, a) => matches!(&**f, Expr::Member(c, m) if matches!(&**c, Expr::Name(n) if n.contains("PeptideAPI"))
            && matches!(m.as_str(), "getProjectile" | "getItem" | "getEnemy")) && matches!(a.as_slice(), [Expr::This]),
        _ => false,
    }
}

/// a field of the source's record of an event, as this engine's event has it: whoever was hit
/// (or hit it) is its `foe`, and the one listening is the object itself
fn record(arg: &str, field: &str) -> Result<(String, Ty), String> {
    match field {
        "receiver" | "opponent" | "grabbed" | "grabber" | "projectile" => Ok((format!("{arg}.data.foe"), Ty::Object)),
        "caller" => Ok(("self".into(), Ty::Object)),
        _ => Err(format!("reads the {field} of the event it is handed")),
    }
}

/// a source box number as the engine's box index (`attackBox` is 1 there, 0 here)
fn box_index(e: &Expr) -> Result<String, String> {
    match e {
        Expr::Int(n) => Ok((n - 1).max(0).to_string()),
        Expr::Num(n) if n.fract() == 0.0 => Ok(((*n as i64) - 1).max(0).to_string()),
        _ => Err("names a box worked out as it runs".into()),
    }
}

/// a source box stat as this engine's field, and the factor it is scaled by on the way out
fn hitbox_field(k: &str) -> Result<(String, Option<f64>), String> {
    let hb = crate::mappings::character_hitbox_stats();
    let f = hb.fields.iter().find(|f| f.ssf2_keys.iter().any(|x| x == k)).ok_or_else(|| format!("names box stat {k}, which this engine has none of"))?;
    // (one counted in frames: the source's frames, and the one this engine counts the freeze from, each two here)
    if f.isframe {
        if f.plus_frames != 0.0 { return Err(format!("names box stat {k}, counted from another frame here")) }
        return Ok((f.fm_field.clone(), Some(crate::entity_gen::FRAME_RATIO as f64)));
    }
    Ok((f.fm_field.clone(), f.scale.filter(|s| *s != 1.0)))
}

/// numbers a row multiplies or divides worked out here (`0 * 0.65` is 0, `x * 1` is x), where
/// nothing on their left binds to the first of them first
fn fold(t: &str) -> String {
    let re = crate::re!(r"(-?\b\d+(?:\.\d+)?) ([*/]) (\d+(?:\.\d+)?)\b");
    let mut out = t.to_string();
    loop {
        let Some(c) = re.captures_iter(&out).find(|c| {
            let at = c.get(0).unwrap().start();
            let before = out[..at].trim_end();
            !before.ends_with(['*', '/', '%', '.']) && !before.ends_with(|ch: char| ch.is_ascii_alphanumeric() || ch == '_')
                && !out[c.get(0).unwrap().end()..].starts_with('.')
        }) else { break };
        let (a, b): (f64, f64) = (c[1].parse().unwrap_or(0.0), c[3].parse().unwrap_or(1.0));
        let v = if &c[2] == "*" { a * b } else { a / b };
        let r = c.get(0).unwrap().range();
        out.replace_range(r, &number((v * 1e9).round() / 1e9));
    }
    // (times or over 1)
    let one = crate::re!(r" [*/] 1([^.\d\w]|$)");
    one.replace_all(&out, "$1").to_string()
}

fn literal(e: &Expr) -> Option<String> {
    match e {
        Expr::Null | Expr::Undefined => Some("null".into()),
        Expr::Bool(b) => Some(b.to_string()),
        Expr::Num(n) => Some(number(*n)),
        Expr::Int(n) => Some(n.to_string()),
        Expr::Str(s) => Some(quote(s)),
        Expr::Unary(UnOp::Neg, x) => literal(x).map(|v| format!("-{v}")),
        Expr::Coerce(_, x) | Expr::Convert(_, x) => literal(x),
        _ => None,
    }
}

fn number(n: f64) -> String {
    if n.is_nan() { return "Math.NaN".into() }
    if n.fract() == 0.0 && n.abs() < 1e15 { return format!("{}", n as i64) }
    let s = format!("{n}");
    s
}

/// a value bracketed when it is worked out (so it reads right inside another)
fn paren(t: &str) -> String {
    let plain = t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '"')
        || (t.ends_with(')') && !t.starts_with('(') && !t.contains(' '))
        || (t.ends_with("()") && !t.contains(' '));
    if plain || (t.starts_with('(') && t.ends_with(')') && balanced_outer(t)) { t.to_string() } else { format!("({t})") }
}

/// whether a text's first bracket closes at its very end
fn balanced_outer(t: &str) -> bool {
    let mut depth = 0;
    for (i, c) in t.char_indices() {
        match c { '(' => depth += 1, ')' => { depth -= 1; if depth == 0 && i != t.len() - 1 { return false } } _ => {} }
    }
    true
}

/// a case's body without the `break` that ends it (a case here never runs on into the next)
fn strip_break(body: &[Stmt]) -> Result<&[Stmt], String> {
    match body.split_last() {
        Some((Stmt::Break, rest)) => Ok(rest),
        Some((Stmt::Return(_), _)) | None => Ok(body),
        _ => Err("a case that runs on into the next".into()),
    }
}

fn describe(e: &Expr) -> String {
    match e {
        Expr::Name(n) => format!("reads {n}"),
        Expr::Member(_, n) => format!("reads .{n}"),
        Expr::This => "uses its clip as a value".into(),
        Expr::Temp(_) | Expr::Join { .. } => "keeps a value the decompiler could not place".into(),
        _ => "has code with no plain form here".into(),
    }
}

/// the decompiler's leftovers, tidied: a variable set and then read once, on the very next
/// statement and nowhere else, is the value it was set to, written there (`v2 = time + 1;
/// this.time = v2` is `this.time = time + 1`)
fn tidy(body: &mut Vec<Stmt>) {
    let mut uses: BTreeMap<u32, usize> = BTreeMap::new();
    walk_exprs(body, &mut |e| if let Expr::Local(r) = e { *uses.entry(*r).or_default() += 1 });
    walk(body, &mut |s| if let Stmt::Assign(Place::Local(r), _) = s { *uses.entry(*r).or_default() += 1 });
    fn pass(list: &mut Vec<Stmt>, uses: &BTreeMap<u32, usize>) {
        for s in list.iter_mut() {
            match s {
                Stmt::If(_, a, b) => { pass(a, uses); pass(b, uses); }
                Stmt::Loop(b) | Stmt::While(_, b) | Stmt::DoWhile(b, _) => pass(b, uses),
                Stmt::Switch(_, cases, d) => { for (_, b) in cases.iter_mut() { pass(b, uses); } pass(d, uses); }
                _ => {}
            }
        }
        let mut i = 0;
        while i + 1 < list.len() {
            let inline = match &list[i] {
                Stmt::Assign(Place::Local(r), v) if uses.get(r) == Some(&2) && !v.has_effect() && !matches!(v, Expr::This) => {
                    let target = Expr::Local(*r);
                    let here: usize = list[i + 1].exprs().iter().map(|e| e.mentions(&target)).sum();
                    (here == 1).then(|| (*r, v.clone()))
                }
                _ => None,
            };
            if let Some((r, v)) = inline {
                let next = list.remove(i + 1);
                list[i] = put(next, r, &v);
            } else { i += 1 }
        }
    }
    pass(body, &uses);
}

/// a statement with a variable's one read replaced by a value (in its own expressions only)
fn put(s: Stmt, r: u32, v: &Expr) -> Stmt {
    let mut f = |e: Expr| if e == Expr::Local(r) { v.clone() } else { e };
    match s {
        Stmt::Expr(e) => Stmt::Expr(e.map(&mut f)),
        Stmt::Assign(p, e) => Stmt::Assign(p.map(&mut f), e.map(&mut f)),
        Stmt::Return(Some(e)) => Stmt::Return(Some(e.map(&mut f))),
        Stmt::If(c, a, b) => Stmt::If(c.map(&mut f), a, b),
        Stmt::While(c, b) => Stmt::While(c.map(&mut f), b),
        Stmt::Switch(c, cs, d) => Stmt::Switch(c.map(&mut f), cs, d),
        other => other,
    }
}

fn walk(list: &[Stmt], f: &mut dyn FnMut(&Stmt)) {
    for s in list {
        f(s);
        match s {
            Stmt::If(_, a, b) => { walk(a, f); walk(b, f); }
            Stmt::Loop(b) | Stmt::While(_, b) | Stmt::DoWhile(b, _) => walk(b, f),
            Stmt::Switch(_, cases, d) => { for (_, b) in cases { walk(b, f); } walk(d, f); }
            Stmt::Try(b, cs) => { walk(b, f); for c in cs { walk(&c.body, f); } }
            _ => {}
        }
    }
}

fn walk_exprs(list: &[Stmt], f: &mut dyn FnMut(&Expr)) {
    crate::model::ast::walk_exprs(list, &mut |e| f(e));
}

fn walk_places(list: &[Stmt], f: &mut dyn FnMut(u32)) {
    walk(list, &mut |s| if let Stmt::Assign(Place::Local(r), _) = s { f(*r) });
    walk_exprs(list, &mut |e| if let Expr::Local(r) = e { f(*r) });
}
