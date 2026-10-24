//! Trees to Haxe text.
//!
//! One rule per construct. Where AS3 and this interpreter agree the Haxe is plain; where they do
//! not the `AS3` layer answers (see `runtime/core/as3.hx` for each measured difference). Types
//! decide which: a test of something known to be a Boolean needs no truthiness, a comparison of
//! two integers needs no NaN rule. What is known comes from the declarations and from what each
//! variable is ever given.

use std::collections::{BTreeMap, BTreeSet};

use super::{ident, short, Package};
use crate::model::ast::*;
use crate::model::{ClassModel, MethodKind};

pub struct ClassWriter<'p, 'a> {
    pkg: &'p Package<'a>,
    class: &'p ClassModel,
    pub problems: Vec<String>,
    pub unknown: BTreeSet<String>,
    scopes: Vec<Scope>,
    uniq: u32,
    /// the methods of this class its code calls through `super`
    supers_used: BTreeSet<String>,
    /// the key sets of the object literals its code writes, each described once for the class
    /// (`X.__shapes`); none when compiling a lone function, which describes each literal inline
    shapes: Option<Vec<Vec<String>>>,
    /// methods written apart from the class (a frame's script that goes on its keyframe): each is
    /// compiled with the class, into `detached_text`, and not given to the object
    pub detached: BTreeSet<String>,
    pub detached_text: BTreeMap<String, String>,
    /// untyped fields that only ever hold whole numbers (`whole_number_fields`)
    int_fields: BTreeSet<String>,
    /// this clip's frames as the engine's animations, for its own jumps by label
    pub jumps: Option<super::ClipJumps>,
    /// the landings the engine makes for this class's code
    pub landings: Vec<super::Landing>,
}

#[derive(Default)]
struct Scope {
    /// register -> the name it is written as
    regs: BTreeMap<u32, String>,
    /// every name this function declares (parameters, variables, captured variables)
    names: BTreeSet<String>,
    int_regs: BTreeSet<u32>,
    bool_regs: BTreeSet<u32>,
    /// a catch's exception, by its number in the method's table
    caught: BTreeMap<u32, String>,
    is_static: bool,
    /// registers the function gives nothing but the object itself (the decompiler's copy of
    /// `this` before a write to one of its fields): a field reached through one is the object's
    me_regs: BTreeSet<u32>,
}

const TAB: &str = "\t";

fn tabs(n: usize) -> String { TAB.repeat(n) }

impl<'p, 'a> ClassWriter<'p, 'a> {
    pub fn new(pkg: &'p Package<'a>, class: &'p ClassModel) -> Self {
        let mut supers_used = BTreeSet::new();
        for f in class.methods.iter().map(|m| &m.function).chain([&class.constructor]) {
            walk_exprs(&f.body, &mut |e| if let Expr::SuperCall(n, _) | Expr::SuperMember(n) = e { if !n.is_empty() { supers_used.insert(n.clone()); } });
        }
        let int_fields = whole_number_fields(pkg, class);
        ClassWriter { pkg, class, int_fields, problems: Vec::new(), unknown: BTreeSet::new(), scopes: Vec::new(), uniq: 0, supers_used, shapes: None,
            detached: BTreeSet::new(), detached_text: BTreeMap::new(), jumps: None, landings: Vec::new() }
    }

    fn problem(&mut self, p: impl Into<String>) { self.problems.push(p.into()) }

    fn class_ident(&self) -> String {
        self.pkg.idents.get(&self.class.name).cloned().unwrap_or_else(|| ident(short(&self.class.name)))
    }

    /// the class this one extends, as written: content, runtime, or a name nobody defines
    fn base_ident(&mut self) -> Option<String> {
        let s = &self.class.super_name;
        if s.is_empty() || s == "Object" { return None }
        match self.pkg.class_ident(s) {
            Some(i) => Some(i),
            None => { self.unknown.insert(format!("class {s}")); Some(ident(short(s))) }
        }
    }

    // ---- the class section ---------------------------------------------------------------------------

    pub fn class_section(&mut self) -> String {
        let c = self.class;
        let id = self.class_ident();
        let mut out = String::new();
        let ext = if c.super_name.is_empty() { String::new() } else { format!(" extends {}", c.super_name) };
        out.push_str(&format!("// ---- class {}{ext}\n", c.name));
        out.push_str(&format!("var {id} = {{ name: {} }};\n", quote(&c.name)));
        self.shapes = Some(Vec::new());
        let shapes_at = out.len();
        let base = self.base_ident();

        // what an object of this class has: the base's, then its own fields and methods
        out.push_str(&format!("{id}.build = function(me) {{\n"));
        match &base {
            Some(b) => out.push_str(&format!("\t{b}.build(me);\n")),
            None => out.push_str("\tif (me.__is == null) me.__is = [];\n"),
        }
        out.push_str(&format!("\tme.__is.push({});\n", quote(&c.name)));
        let mut celled: Vec<(String, bool)> = Vec::new();
        for f in c.fields.iter().filter(|f| !f.is_static) {
            let v = self.field_default(f.default.as_ref(), f.type_name.as_deref());
            if self.stored(&f.name) {
                out.push_str(&format!("\tme.__c_{} = {};\n", ident(&f.name), cell_of(f.type_name.as_deref(), &v)));
                let num = matches!(f.type_name.as_deref().map(short), Some("Number" | "int" | "uint"));
                celled.push((ident(&f.name), num));
            } else {
                out.push_str(&format!("\tme.{} = {v};\n", ident(&f.name)));
            }
        }
        // once the object is gone from what the source shows (a clip taken off its parent), its
        // containers go back for the next object, and it keeps its values as its own
        // (and which containers it holds is part of what a rollback puts back)
        if !celled.is_empty() {
            let loose: Vec<String> = celled.iter().map(|(n, num)| format!("me.__c_{n} = PeptideRuntime.loose(me.__c_{n}, {num});")).collect();
            let held: Vec<String> = celled.iter().map(|(n, _)| format!("me.__c_{n}")).collect();
            let back: Vec<String> = celled.iter().enumerate().map(|(i, (n, _))| format!("me.__c_{n} = s[{i}];")).collect();
            out.push_str(&format!("\tPeptideRuntime.own(me, function() {{ {} }}, function() {{ return [{}]; }}, function(s) {{ {} }});\n",
                loose.join(" "), held.join(", "), back.join(" ")));
        }
        for n in self.supers_used.clone() {
            // a listed call the base answers at conversion: the call itself, made on this object
            let args: Vec<String> = (0..4).map(|i| format!("__a{i}")).collect();
            let base = match self.api_text("me", &n, &args) {
                Some(body) => format!("me.{m} != null ? me.{m} : function(?__a0, ?__a1, ?__a2, ?__a3) {{ return {body}; }}", m = ident(&n)),
                None => format!("me.{}", ident(&n)),
            };
            out.push_str(&format!("\tvar super_{} = {base};\n", ident(&n)));
        }
        for m in c.methods.iter().filter(|m| !m.is_static) {
            let name = match m.kind { MethodKind::Getter => format!("get_{}", m.name), MethodKind::Setter => format!("set_{}", m.name), _ => m.name.clone() };
            let f = self.function(&m.function, 1, Some(false));
            if m.kind == MethodKind::Method && self.detached.contains(&m.name) {
                self.detached_text.insert(m.name.clone(), f);
                continue;
            }
            // a frame script's error ends that script and nothing else, as the source's player
            // reports it and plays on. here an error escaping the function stops the engine's
            // whole update, and the engine runs the same frame again next time: saffron city's
            // charmander read a flame it never fired thirteen thousand times. the interpreter
            // catches an error only in the function it happens in, so the guard is the script's own
            let f = if c.frame_scripts.iter().any(|s| s.method == m.name) { guarded(&f) } else { f };
            out.push_str(&format!("\tme.{} = {f};\n", ident(&name)));
        }
        out.push_str("};\n");

        // the constructor
        let ctor = self.function_with_me(&c.constructor, 0);
        out.push_str(&format!("{id}.init = {ctor};\n"));
        let params: Vec<String> = c.constructor.params.iter().map(|p| ident(&p.name)).collect();
        out.push_str(&format!("{id}.create = function({}) {{ var me = {{}}; {id}.build(me); {id}.init(me{}); return me; }};\n",
            params.iter().map(|p| format!("?{p}")).collect::<Vec<_>>().join(", "),
            params.iter().map(|p| format!(", {p}")).collect::<String>()));

        // the static side
        for f in c.fields.iter().filter(|f| f.is_static) {
            let v = self.field_default(f.default.as_ref(), f.type_name.as_deref());
            if self.stored(&f.name) {
                out.push_str(&format!("{id}.__c_{} = {};\n", ident(&f.name), cell_of(f.type_name.as_deref(), &v)));
            } else {
                out.push_str(&format!("{id}.{} = {v};\n", ident(&f.name)));
            }
        }
        for m in c.methods.iter().filter(|m| m.is_static) {
            let name = match m.kind { MethodKind::Getter => format!("get_{}", m.name), MethodKind::Setter => format!("set_{}", m.name), _ => m.name.clone() };
            let f = self.function(&m.function, 0, Some(true));
            out.push_str(&format!("{id}.{} = {f};\n", ident(&name)));
        }
        if !c.static_init.body.iter().all(|s| matches!(s, Stmt::Return(None))) {
            let f = self.function(&c.static_init, 0, Some(true));
            out.push_str(&format!("{id}.cinit = {f};\n"));
        }
        // the key sets its literals use, before anything that makes one
        let shapes = self.shapes.take().unwrap_or_default();
        if !shapes.is_empty() {
            let list: Vec<String> = shapes.iter().map(|k| shape_literal(k)).collect();
            out.insert_str(shapes_at, &format!("{id}.__shapes = [\n\t{}\n];\n", list.join(",\n\t")));
        }
        out
    }

    fn field_default(&mut self, d: Option<&Expr>, ty: Option<&str>) -> String {
        match d {
            Some(e) => self.expr(e),
            None => match ty {
                Some("Number") => "Math.NaN".into(),
                Some("int") | Some("uint") => "0".into(),
                Some("Boolean") => "false".into(),
                _ => "null".into(),
            },
        }
    }

    /// the constructor: a function of the object it builds and the arguments
    fn function_with_me(&mut self, f: &Function, depth: usize) -> String {
        let mut text = self.function(f, depth, Some(false));
        // `function(?a) {` -> `function(me, ?a) {`
        if let Some(rest) = text.strip_prefix("function(") {
            text = if rest.starts_with(')') { format!("function(me{rest}") } else { format!("function(me, {rest}") };
        }
        text
    }

    // ---- functions ------------------------------------------------------------------------------------

    /// A function. `method`: Some(static?) for a method of this class, None for a closure or a
    /// lone function.
    pub fn function(&mut self, f: &Function, depth: usize, method: Option<bool>) -> String {
        self.function_body(f, depth, method)
    }

    fn function_body(&mut self, f: &Function, depth: usize, method: Option<bool>) -> String {
        let mut scope = Scope { is_static: method == Some(true), ..Default::default() };
        let outer: BTreeSet<String> = self.scopes.iter().flat_map(|s| s.names.iter().cloned()).collect();
        let taken = |n: &str, scope: &Scope| RESERVED_HERE.contains(&n) || scope.names.contains(n);
        for (i, p) in f.params.iter().enumerate() {
            let mut n = ident(&p.name);
            while taken(&n, &scope) { n.push('_'); }
            scope.regs.insert(i as u32 + 1, n.clone());
            scope.names.insert(n);
        }
        let nparams = f.params.len() as u32;
        if f.rest { self.problem("a rest parameter"); }
        let debug: BTreeMap<u32, String> = f.local_names.iter().cloned().collect();
        // every register the body uses, named by the package's debug names where it kept them
        let mut regs: BTreeSet<u32> = BTreeSet::new();
        walk_places(&f.body, &mut |r| { regs.insert(r); });
        for r in regs.iter().copied().filter(|r| *r > nparams) {
            let mut n = debug.get(&r).map(|s| ident(s)).unwrap_or_else(|| format!("r{r}"));
            while taken(&n, &scope) || outer.contains(&n) && !debug.contains_key(&r) { n.push('_'); }
            scope.regs.insert(r, n.clone());
            scope.names.insert(n);
        }
        let mut captured: Vec<String> = Vec::new();
        for (_, n) in &f.activation {
            let n = ident(n);
            if scope.names.insert(n.clone()) { captured.push(n); }
        }
        let (ints, bools) = infer_types(f);
        scope.int_regs = ints;
        scope.bool_regs = bools;
        scope.me_regs = this_copies(f);
        self.scopes.push(scope);

        let mut body = String::new();
        let ind = tabs(depth + 1);
        // every variable once, at the top: AS3 scopes a variable to its function, and a closure
        // made in a loop sees the one variable, not one per turn
        let mut decl: Vec<String> = Vec::new();
        for r in regs.iter().filter(|r| **r > nparams) { decl.push(self.scopes.last().unwrap().regs[r].clone()); }
        decl.extend(captured.iter().cloned());
        let mut vars: BTreeSet<String> = BTreeSet::new();
        walk_exprs(&f.body, &mut |e| match e {
            Expr::Temp(t) => { vars.insert(format!("__t{t}")); }
            Expr::Join { block, slot } => { vars.insert(format!("__j{block}_{slot}")); }
            _ => {}
        });
        walk_assigned_vars(&f.body, &mut vars);
        decl.extend(vars);
        for d in &decl { body.push_str(&format!("{ind}var {d};\n")); }
        // a default the signature declares, for an argument the call leaves out
        for (i, p) in f.params.iter().enumerate() {
            if let Some(d) = &p.default {
                if matches!(d, Expr::Null | Expr::Undefined) { continue }
                let n = self.scopes.last().unwrap().regs[&(i as u32 + 1)].clone();
                let v = self.expr(d);
                body.push_str(&format!("{ind}if ({n} == null) {n} = {v};\n"));
            }
        }
        body.push_str(&self.stmts(&f.body, depth + 1));
        let scope = self.scopes.pop().unwrap();
        let params: Vec<String> = (1..=nparams).map(|r| format!("?{}", scope.regs[&r])).collect();
        format!("function({}) {{\n{body}{}}}", params.join(", "), tabs(depth))
    }

    // ---- statements ----------------------------------------------------------------------------------

    fn stmts(&mut self, list: &[Stmt], depth: usize) -> String {
        let mut out = String::new();
        for s in list { out.push_str(&self.stmt(s, depth)); }
        out
    }

    fn block(&mut self, list: &[Stmt], depth: usize) -> String {
        format!("{{\n{}{}}}", self.stmts(list, depth + 1), tabs(depth))
    }

    fn stmt(&mut self, s: &Stmt, depth: usize) -> String {
        let t = tabs(depth);
        match s {
            // a call that does nothing here, made only for its effect: nothing is written
            // (inert.jsonc), and the gap is said once at conversion
            Stmt::Expr(Expr::Call(callee, args)) if self.inert_call(callee, args) => {
                if let Expr::Member(_, m) = &**callee { self.unknown.insert(format!("BitmapData.{m} (draws nothing here)")); }
                String::new()
            }
            Stmt::Expr(e) => format!("{t}{};\n", statement_text(self.expr(e))),
            Stmt::Assign(p, v) => format!("{t}{};\n", self.assign(p, v)),
            Stmt::If(c, a, b) => {
                let cond = self.cond(c);
                let then = self.stmts(a, depth + 1);
                // a test whose branches come to nothing (a drawing call that does nothing here),
                // and whose working out does nothing either: nothing is written
                if then.trim().is_empty() && pure(c, &self.pkg.runtime.inert_values)
                    && (b.is_empty() || b.iter().all(|x| matches!(x, Stmt::Expr(Expr::Call(f, g)) if self.inert_call(f, g)))) {
                    return String::new();
                }
                let mut out = format!("{t}if ({cond}) {{\n{then}{t}}}");
                if !b.is_empty() {
                    if let [Stmt::If(..)] = b.as_slice() {
                        let inner = self.stmt(&b[0], depth);
                        out.push_str(&format!(" else {}", inner.trim_start()));
                            return out;
                    }
                    out.push_str(&format!(" else {}", self.block(b, depth)));
                }
                out.push('\n');
                out
            }
            Stmt::Loop(b) => format!("{t}while (true) {}\n", self.block(b, depth)),
            Stmt::While(c, b) if self.counting_loop(c, b).is_some() => {
                let rep = self.counting_loop(c, b).unwrap();
                self.stmt(&rep, depth)
            }
            Stmt::While(c, b) => {
                let cond = self.cond(c);
                format!("{t}while ({cond}) {}\n", self.block(b, depth))
            }
            Stmt::DoWhile(b, c) => {
                let body = self.block(b, depth);
                format!("{t}do {body} while ({});\n", self.cond(c))
            }
            Stmt::Break => format!("{t}break;\n"),
            Stmt::Continue => format!("{t}continue;\n"),
            Stmt::Return(None) => format!("{t}return;\n"),
            Stmt::Return(Some(e)) => format!("{t}return {};\n", self.expr(e)),
            Stmt::Throw(e) => format!("{t}throw {};\n", self.expr(e)),
            Stmt::Switch(v, arms, d) => {
                let mut out = format!("{t}switch ({}) {{\n", self.expr(v));
                for (keys, body) in arms {
                    let ks: Vec<String> = keys.iter().map(|k| self.expr(k)).collect();
                    out.push_str(&format!("{t}\tcase {}:\n", ks.join(", ")));
                    let b = self.stmts(body, depth + 2);
                    if b.is_empty() { out.push_str(&format!("{t}\t\t{{}}\n")) } else { out.push_str(&b) }
                }
                if !d.is_empty() {
                    out.push_str(&format!("{t}\tdefault:\n"));
                    out.push_str(&self.stmts(d, depth + 2));
                }
                out.push_str(&format!("{t}}}\n"));
                out
            }
            Stmt::Try(b, catches) => {
                let body = self.block(b, depth);
                let Some(first) = catches.first() else { return format!("{t}{body}\n") };
                let var = first.var.as_deref().map(ident).unwrap_or_else(|| format!("e{}", first.index));
                for c in catches { self.scopes.last_mut().unwrap().caught.insert(c.index, var.clone()); }
                let mut handler = String::new();
                let typed: Vec<&Catch> = catches.iter().filter(|c| c.type_name.is_some()).collect();
                if typed.is_empty() {
                    handler.push_str(&self.stmts(&first.body, depth + 1));
                } else {
                    let mut chain = String::new();
                    for (i, c) in catches.iter().enumerate() {
                        let test = match &c.type_name { Some(ty) => format!("AS3.is({var}, {})", quote(ty)), None => "true".into() };
                        let b = self.block(&c.body, depth + 1);
                        chain.push_str(&format!("{}if ({test}) {b}", if i == 0 { tabs(depth + 1) } else { " else ".into() }));
                    }
                    if catches.iter().all(|c| c.type_name.is_some()) { chain.push_str(&format!(" else throw {var};")); }
                    handler.push_str(&chain);
                    handler.push('\n');
                }
                format!("{t}try {body} catch ({var}:Dynamic) {{\n{handler}{t}}}\n")
            }
            Stmt::Unreadable(r) => { self.problem(format!("unreadable: {r}")); format!("{t}// [unreadable: {r}]\n") }
        }
    }

    fn assign(&mut self, p: &Place, v: &Expr) -> String {
        // a field of a value a container holds (a point, a rectangle): written into a copy put in
        // its place. an object of a class of the package's own keeps its fields in containers of
        // its own, and a copy of it is itself
        if let Place::Member(o, n) = p {
            if self.stored_read(o) && !self.stored(n) && !self.pkg.accessors.contains(n) {
                if let Some(place) = place_of(o) {
                    let read = self.expr(o);
                    let t = self.fresh("__cw");
                    self.scopes.last_mut().map(|s| s.names.insert(t.clone()));
                    let inner = self.assign(&Place::Member(Expr::Name(t.clone()), n.clone()), v);
                    let write = self.assign(&place, &Expr::Name(t.clone()));
                    self.scopes.last_mut().map(|s| s.names.remove(&t));
                    return format!("{{ var {t} = AS3.copyOf({read}); {inner}; {write}; }}");
                }
            }
        }
        // an element of an array a container holds: written into a copy put in its place
        if let Place::Index(o, k) = p {
            if self.stored_read(o) && literal_key(k).is_none() {
                if let Some(place) = place_of(o) {
                    let read = self.expr(o);
                    let t = self.fresh("__cw");
                    self.scopes.last_mut().map(|s| s.names.insert(t.clone()));
                    let inner = self.assign(&Place::Index(Expr::Name(t.clone()), k.clone()), v);
                    let write = self.assign(&place, &Expr::Name(t.clone()));
                    self.scopes.last_mut().map(|s| s.names.remove(&t));
                    return format!("{{ var {t} = AS3.copyOf({read}); {inner}; {write}; }}");
                }
            }
        }
        let val = self.expr(v);
        match p {
            Place::Local(r) => format!("{} = {val}", self.reg(*r)),
            Place::Temp(t) => format!("__t{t} = {val}"),
            Place::Join { block, slot } => format!("__j{block}_{slot} = {val}"),
            Place::Name(n) => {
                let in_static = self.scopes.iter().any(|s| s.is_static);
                if self.local_name(n).is_none() && !in_static && self.own_field(n, false) && !self.own_setter(n) {
                    return self.cell_write("me", n, &val, true);
                }
                if self.local_name(n).is_none() && !self.content_member(n) && self.own_field(n, true) {
                    if let Some(c) = self.content_static(n) { return self.cell_write(&c, n, &val, true) }
                }
                format!("{} = {val}", self.name_place(n))
            }
            Place::SuperMember(n) => format!("me.{} = {val}", ident(n)),
            Place::Member(o, n) => {
                if n.starts_with('#') { self.problem(format!("a slot by number ({n})")); }
                let obj = self.expr_atom(o);
                if self.is_me(o) && self.own_setter(n) {
                    return format!("me.set_{}({val})", ident(n));
                }
                if self.stored(n) {
                    let known = if self.is_me(o) { self.own_field(n, false) }
                        else { self.static_of(o).is_some_and(|c| self.pkg.class_named(&c).is_some_and(|m| m.fields.iter().any(|f| f.is_static && f.name == *n))) };
                    return self.cell_write(&obj, n, &val, known);
                }
                if self.pkg.accessors.contains(n) && !self.is_me(o) {
                    let v = self.fresh("__o");
                    return format!("{{ var {v} = {obj}; if ({v}.set_{n} != null) {v}.set_{n}({val}); else {v}.{n} = {val}; }}", n = ident(n));
                }
                format!("{obj}.{} = {val}", ident(n))
            }
            Place::Index(o, k) => {
                let obj = self.expr_atom(o);
                match literal_key(k) {
                    Some(name) if self.stored(&name) => self.cell_write(&obj, &name, &val, false),
                    Some(name) => format!("{obj}.{} = {val}", ident(&name)),
                    None => format!("AS3.put({obj}, {}, {val})", self.expr(k)),
                }
            }
        }
    }

    // ---- expressions ---------------------------------------------------------------------------------

    fn fresh(&mut self, base: &str) -> String {
        self.uniq += 1;
        format!("{base}{}", self.uniq)
    }

    fn reg(&self, r: u32) -> String {
        for s in self.scopes.iter().rev() { if let Some(n) = s.regs.get(&r) { return n.clone() } }
        format!("r{r}")
    }

    /// `this` as the instance (in a static method it is the class)
    /// a call to a method that does nothing here (inert.jsonc), on an object its class declares as
    /// that method's class, whose arguments do nothing on their own
    fn inert_call(&self, callee: &Expr, args: &[Expr]) -> bool {
        let Expr::Member(recv, m) = callee else { return false };
        let declared = match &**recv {
            Expr::Member(o, n) if self.is_me(o) => self.field_type(n),
            Expr::Name(n) if self.local_name(n).is_none() => self.field_type(n),
            _ => None,
        };
        let Some(class) = declared.map(|t| super::short(&t).to_string()) else { return false };
        if !self.pkg.runtime.inert_calls.get(&class).is_some_and(|ms| ms.contains(m)) { return false }
        args.iter().all(|a| pure(a, &self.pkg.runtime.inert_values))
    }

    /// a statement that comes to nothing here: a call that does nothing (inert.jsonc), or a test
    /// with no effect of its own whose branches come to nothing
    fn void_stmt(&self, st: &Stmt) -> bool {
        match st {
            Stmt::Expr(Expr::Call(f, a)) => self.inert_call(f, a),
            Stmt::If(c, a, b) => pure(c, &self.pkg.runtime.inert_values) && a.iter().chain(b.iter()).all(|x| self.void_stmt(x)),
            _ => false,
        }
    }

    /// `while (n < X) { n = n + 1 }`, its only work counting `n` up to a bound nothing in it
    /// changes (the rest of its body comes to nothing here): the count is the bound, given at once.
    /// krazoa palace's code counts across each row of a picture whose pixels this engine never
    /// draws, four thousand turns a frame for nothing. what the loop leaves behind stays: the
    /// counter at the bound, and the registers the decompiled step goes through
    fn counting_loop(&self, c: &Expr, body: &[Stmt]) -> Option<Stmt> {
        let Expr::Binary(BinOp::Lt, n, bound) = c else { return None };
        let values = &self.pkg.runtime.inert_values;
        // (both whole numbers, so counting by one stops at the bound itself)
        if !pure(bound, values) || !self.is_int(n) || !self.is_int(bound) { return None }
        // the counter: a field of the object, or a register
        let counter_place = match &**n {
            Expr::Member(o, f) if self.is_me(o) => Place::Member((**o).clone(), f.clone()),
            Expr::Local(r) => Place::Local(*r),
            _ => return None,
        };
        let reads_counter = |e: &Expr| -> bool {
            let mut hit = false;
            let probe = vec![Stmt::Expr(e.clone())];
            crate::model::ast::walk_exprs(&probe, &mut |x| match (x, &**n) {
                (Expr::Member(o, f), Expr::Member(_, g)) if f == g && (self.is_me(o)) => hit = true,
                (Expr::Local(a), Expr::Local(b)) if a == b => hit = true,
                _ => {}
            });
            hit
        };
        if reads_counter(bound) { return None }
        // the body: copies of the object, the counter plus one into a register or straight back,
        // the register into the counter, and what comes to nothing
        let mut copies: Vec<u32> = Vec::new();
        let mut stepped: Option<u32> = None;
        let mut written = false;
        let plus_one = |v: &Expr| -> bool {
            let strip = |e: &Expr| -> Expr { match e { Expr::Convert(_, x) => (**x).clone(), o => o.clone() } };
            matches!(v, Expr::Binary(BinOp::Add, a, one) if matches!(**one, Expr::Int(1)) && strip(a) == **n)
        };
        for st in body {
            match st {
                Stmt::Assign(Place::Local(r), Expr::This) if !self.scopes.iter().any(|s| s.is_static) => copies.push(*r),
                Stmt::Assign(Place::Local(r), v) if !written && matches!(counter_place, Place::Local(c) if c == *r) && plus_one(v) => written = true,
                Stmt::Assign(Place::Local(r), v) if plus_one(v) && stepped.is_none() => stepped = Some(*r),
                Stmt::Assign(Place::Member(o, f), v) if !written && matches!(&counter_place, Place::Member(_, g) if g == f)
                    && (matches!(o, Expr::This) || matches!(o, Expr::Local(r) if copies.contains(r)))
                    && (plus_one(v) || matches!(v, Expr::Local(r) if Some(*r) == stepped)) => written = true,
                other if self.void_stmt(other) => {}
                _ => return None,
            }
        }
        if !written { return None }
        // if (n < X) { n = X; [step register = X;] [copies = this;] }
        let mut set = vec![Stmt::Assign(counter_place, (**bound).clone())];
        if let Some(r) = stepped { set.push(Stmt::Assign(Place::Local(r), (**bound).clone())); }
        for r in copies { set.push(Stmt::Assign(Place::Local(r), Expr::This)); }
        Some(Stmt::If(c.clone(), set, vec![]))
    }

    fn is_me(&self, e: &Expr) -> bool {
        let own_copy = |r: &u32| self.scopes.last().is_some_and(|s| s.me_regs.contains(r));
        (matches!(e, Expr::This) || matches!(e, Expr::Local(r) if own_copy(r))) && !self.scopes.iter().any(|s| s.is_static)
    }

    /// the value wrapped so it can be an operand
    fn expr_atom(&mut self, e: &Expr) -> String {
        let s = self.expr(e);
        // a conversion that writes nothing of its own is as atomic as what it converts
        let mut e = e;
        while let Expr::Convert(_, x) | Expr::Coerce(_, x) = e {
            if s.starts_with("AS3.") { break }
            e = x;
        }
        let atomic = matches!(e, Expr::Local(_) | Expr::Temp(_) | Expr::Join { .. } | Expr::This | Expr::Name(_)
            | Expr::Member(..) | Expr::Index(..) | Expr::Call(..) | Expr::Str(_) | Expr::Null | Expr::Undefined
            | Expr::Bool(_) | Expr::Array(_) | Expr::SuperMember(_) | Expr::SuperCall(..))
            || matches!(e, Expr::Int(i) if *i >= 0) || matches!(e, Expr::Num(n) if *n >= 0.0);
        if atomic { s } else { format!("({s})") }
    }

    /// a test: truthiness as AS3 has it
    fn cond(&mut self, e: &Expr) -> String {
        match e {
            Expr::And(a, b) => format!("{} && {}", self.cond_atom(a), self.cond_atom(b)),
            Expr::Or(a, b) => format!("{} || {}", self.cond_atom(a), self.cond_atom(b)),
            Expr::Unary(UnOp::Not, x) => format!("!{}", self.cond_atom(x)),
            e if self.is_bool(e) => self.expr(e),
            e => format!("AS3.truthy({})", self.expr(e)),
        }
    }

    fn cond_atom(&mut self, e: &Expr) -> String {
        let s = self.cond(e);
        if matches!(e, Expr::And(..) | Expr::Or(..) | Expr::Binary(..) | Expr::Cond(..)) { format!("({s})") } else { s }
    }

    pub fn expr(&mut self, e: &Expr) -> String {
        match e {
            Expr::Undefined | Expr::Null => "null".into(),
            Expr::Bool(b) => b.to_string(),
            Expr::Num(n) => number(*n),
            // past 2^30 a whole Float: the engine's interpreter compares two ints by their
            // difference, which overflows (a best score seeded with int.MAX_VALUE)
            Expr::Int(i) => if *i > -(1 << 30) && *i < (1 << 30) { i.to_string() } else { format!("{i}.0") },
            Expr::Str(s) => quote(s),
            Expr::This => if self.scopes.iter().any(|s| s.is_static) { self.class_ident() } else { "me".into() },
            Expr::Local(r) => self.reg(*r),
            Expr::Temp(t) => format!("__t{t}"),
            Expr::Join { block, slot } => format!("__j{block}_{slot}"),
            Expr::Name(n) => self.name_value(n),
            // what holds a name (`n++` on a member reads it, adds and writes it back through its holder):
            // the object for one of its members, the class for a static
            Expr::ScopeOf(n) => {
                let in_static = self.scopes.iter().any(|s| s.is_static);
                if self.local_name(n).is_none() && !in_static
                    && (self.content_member(n) || self.runtime_base().is_some_and(|b| self.pkg.runtime.has_member(&b, n))) {
                    "me".to_string()
                } else if let Some(c) = self.content_static(n) {
                    c
                } else {
                    self.problem(format!("a scope object for {n}"));
                    self.name_value(n)
                }
            }
            Expr::Member(o, n) => self.member(o, n),
            Expr::Index(o, k) => {
                let obj = self.expr_atom(o);
                match literal_key(k) {
                    Some(name) if self.stored(&name) => self.cell_read(&obj, &name, false),
                    Some(name) => format!("{obj}.{}", ident(&name)),
                    // a whole-number key on an array is read in place, the general read kept for
                    // anything else (a negative or past-the-end key, an object): the read is the
                    // inner step of a loop over a picture or a list, and a call there is the cost
                    None if self.is_int(k) => {
                        let (o, i) = (self.fresh("__a"), self.fresh("__k"));
                        format!("({{ var {o} = {obj}; var {i} = {}; ({o} != null && {o}.push != null && {i} >= 0 && {i} < {o}.length) ? {o}[{i}] : AS3.at({o}, {i}); }})", self.expr(k))
                    }
                    None => format!("AS3.at({obj}, {})", self.expr(k)),
                }
            }
            Expr::Call(callee, args) => self.call(callee, args),
            Expr::New(c, args) => self.new_(c, args),
            Expr::SuperMember(n) => format!("super_{}", ident(n)),
            Expr::SuperCall(n, args) => {
                let a = self.args(args);
                if n.is_empty() {
                    match self.base_ident() {
                        Some(b) => format!("{b}.init(me{})", if a.is_empty() { String::new() } else { format!(", {a}") }),
                        None => "null".into(),
                    }
                } else {
                    format!("super_{}({a})", ident(n))
                }
            }
            Expr::Unary(op, x) => match op {
                UnOp::Not => format!("!{}", self.cond_atom(x)),
                UnOp::Neg => format!("-{}", self.expr_atom(x)),
                UnOp::BitNot => format!("~{}", self.expr_atom(x)),
                UnOp::TypeOf => format!("AS3.typeOf({})", self.expr(x)),
            },
            Expr::Binary(op, a, b) => self.binary(*op, a, b),
            Expr::And(a, b) | Expr::Or(a, b) => {
                if self.is_bool(a) && self.is_bool(b) {
                    let o = if matches!(e, Expr::And(..)) { "&&" } else { "||" };
                    return format!("{} {o} {}", self.cond_atom(a), self.cond_atom(b));
                }
                // AS3 gives back the operand it settled on, and reads the first only once
                let v = self.fresh("__v");
                let first = self.expr(a);
                let second = self.expr(b);
                let pick = if matches!(e, Expr::And(..)) { format!("AS3.truthy({v}) ? {second} : {v}") } else { format!("AS3.truthy({v}) ? {v} : {second}") };
                format!("({{ var {v} = {first}; {pick}; }})")
            }
            Expr::Cond(c, a, b) => {
                let c = self.cond(c);
                format!("{c} ? {} : {}", self.expr_atom(a), self.expr_atom(b))
            }
            Expr::Convert(c, x) => match c {
                Conv::Int => if self.is_int(x) { self.expr(x) } else { format!("AS3.int({})", self.expr(x)) },
                Conv::Uint => format!("AS3.uint({})", self.expr(x)),
                Conv::Number => if self.is_int(x) || matches!(**x, Expr::Num(_)) { self.expr(x) } else { format!("AS3.num({})", self.expr(x)) },
                Conv::String => if matches!(**x, Expr::Str(_)) { self.expr(x) } else { format!("AS3.str({})", self.expr(x)) },
                Conv::Boolean => if self.is_bool(x) { self.expr(x) } else { format!("AS3.truthy({})", self.expr(x)) },
                Conv::Object => self.expr(x),
            },
            Expr::Coerce(_, x) => self.expr(x),
            Expr::Array(items) => format!("[{}]", self.args(items)),
            Expr::Object(kv) => {
                let static_keys = kv.iter().all(|(k, _)| literal_key(k).is_some());
                if static_keys && kv.is_empty() {
                    "{}".into()
                } else if static_keys {
                    let keys: Vec<String> = kv.iter().map(|(k, _)| literal_key(k).unwrap()).collect();
                    let mut fields: Vec<String> = kv.iter().map(|(k, v)| format!("{}: {}", ident(&literal_key(k).unwrap()), self.expr(v))).collect();
                    // the keys it has, so it can be read and enumerated by a key worked out as it
                    // runs (a record here has no other way to be asked for a field by name)
                    let shape = match &mut self.shapes {
                        Some(list) => {
                            let i = match list.iter().position(|k| *k == keys) { Some(i) => i, None => { list.push(keys.clone()); list.len() - 1 } };
                            format!("{}.__shapes[{i}]", self.class_ident())
                        }
                        None => shape_literal(&keys),
                    };
                    fields.push(format!("__shape: {shape}"));
                    format!("{{ {} }}", fields.join(", "))
                } else {
                    let flat: Vec<String> = kv.iter().flat_map(|(k, v)| [self.expr(k), self.expr(v)]).collect();
                    format!("AS3.object([{}])", flat.join(", "))
                }
            }
            Expr::Function(f) => {
                let depth = self.scopes.len();
                self.function(f, depth, None)
            }
            Expr::Assign(p, v) => format!("({})", self.assign(p, v)),
            Expr::Delete(p) => match &**p {
                Place::Member(o, n) => format!("AS3.remove({}, {})", self.expr(o), quote(n)),
                Place::Index(o, k) => format!("AS3.remove({}, {})", self.expr(o), self.expr(k)),
                _ => { self.problem("delete of a variable"); "false".into() }
            },
            Expr::HasNext2 { object, index } => {
                let (o, i) = (self.reg(*object), self.reg(*index));
                format!("({{ {i} = AS3.nextIndex({o}, {i}); {i} != 0; }})")
            }
            Expr::NextName(o, i) => format!("AS3.nextName({}, {})", self.expr(o), self.expr(i)),
            Expr::NextValue(o, i) => format!("AS3.nextValue({}, {})", self.expr(o), self.expr(i)),
            Expr::HasNext(o, i) => format!("AS3.nextIndex({}, {})", self.expr(o), self.expr(i)),
            Expr::Global => { self.problem("the global object"); "null".into() }
            Expr::Activation => { self.problem("the activation object"); "null".into() }
            Expr::Caught(i) => {
                for s in self.scopes.iter().rev() { if let Some(n) = s.caught.get(i) { return n.clone() } }
                format!("e{i}")
            }
            Expr::Class(_) => { self.problem("a class defined in place"); "null".into() }
            Expr::TypeApply(..) => { self.problem("a generic type as a value"); "null".into() }
            Expr::Unreadable(r) => { self.problem(format!("unreadable: {r}")); "null".into() }
        }
    }

    fn args(&mut self, args: &[Expr]) -> String {
        args.iter().map(|a| self.expr(a)).collect::<Vec<_>>().join(", ")
    }

    fn binary(&mut self, op: BinOp, a: &Expr, b: &Expr) -> String {
        let plain = |s: &mut Self, sym: &str| format!("{} {sym} {}", s.expr_atom(a), s.expr_atom(b));
        match op {
            // joined to a string, a value is written as AS3 writes it (an object as its name, never
            // walked: the engine's own conversion follows every field and overflows on the first
            // loop, a clip's parent and child)
            BinOp::Add if self.is_string(a) || self.is_string(b) => {
                let side = |s: &mut Self, e: &Expr| if s.is_primitive(e) { s.expr_atom(e) } else { format!("AS3.str({})", s.expr(e)) };
                let (x, y) = (side(self, a), side(self, b));
                format!("{x} + {y}")
            }
            BinOp::Add => plain(self, "+"),
            BinOp::Sub => plain(self, "-"),
            BinOp::Mul => plain(self, "*"),
            BinOp::Div => plain(self, "/"),
            BinOp::Mod => plain(self, "%"),
            BinOp::Shl => plain(self, "<<"),
            BinOp::Shr => plain(self, ">>"),
            BinOp::BitAnd => plain(self, "&"),
            BinOp::BitOr => plain(self, "|"),
            BinOp::BitXor => plain(self, "^"),
            BinOp::UShr => format!("AS3.ushr({}, {})", self.expr(a), self.expr(b)),
            BinOp::Eq | BinOp::StrictEq => plain(self, "=="),
            BinOp::Ne | BinOp::StrictNe => plain(self, "!="),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                if self.is_int(a) && self.is_int(b) {
                    let sym = match op { BinOp::Lt => "<", BinOp::Le => "<=", BinOp::Gt => ">", _ => ">=" };
                    plain(self, sym)
                } else {
                    let f = match op { BinOp::Lt => "lt", BinOp::Le => "le", BinOp::Gt => "gt", _ => "ge" };
                    format!("AS3.{f}({}, {})", self.expr(a), self.expr(b))
                }
            }
            BinOp::In => format!("AS3.has({}, {})", self.expr(b), self.expr(a)),
            BinOp::Is | BinOp::InstanceOf => format!("AS3.is({}, {})", self.expr(a), self.type_name(b)),
            BinOp::As => format!("AS3.as({}, {})", self.expr(a), self.type_name(b)),
        }
    }

    /// a type as the `is`/`as` rules name it
    fn type_name(&mut self, t: &Expr) -> String {
        match t {
            Expr::Name(n) => quote(self.pkg.class_named(n).map(|c| c.name.as_str()).unwrap_or(n)),
            other => { let s = self.expr(other); format!("{s}.name") }
        }
    }

    // ---- names -------------------------------------------------------------------------------------------

    fn local_name(&self, n: &str) -> Option<String> {
        let found = |id: String| self.scopes.iter().rev().find(|s| s.names.contains(&id)).map(|_| id);
        // a variable of an enclosing function can be named with its namespace (`pkg.effectMC`)
        found(ident(n)).or_else(|| (short(n) != n).then(|| found(ident(short(n)))).flatten())
    }

    /// whether this class, or a content class it extends, declares an instance member
    /// the class a value is declared to be, where the source says: a field's declared type, a
    /// coercion to a class
    fn static_class(&self, e: &Expr) -> Option<String> {
        let field = |n: &str| {
            let (chain, _) = self.pkg.chain(&self.class.name);
            chain.iter().flat_map(|c| c.fields.iter()).find(|f| f.name == n).and_then(|f| f.type_name.clone())
        };
        let t = match e {
            Expr::Member(o, n) if self.is_me(o) => field(n),
            Expr::Name(n) if self.local_name(n).is_none() && self.content_member(n) => field(n),
            Expr::Coerce(t, _) => Some(t.clone()),
            // a class named as the receiver: its static side
            Expr::Name(n) if self.local_name(n).is_none() && !self.content_member(n)
                && (self.pkg.runtime.provides(n) || self.pkg.class_named(n).is_some()) => Some(n.clone()),
            _ => None,
        }?;
        let t = short(&t).to_string();
        (!matches!(t.as_str(), "*" | "Object" | "")).then_some(t)
    }

    fn content_member(&self, n: &str) -> bool {
        let (chain, _) = self.pkg.chain(&self.class.name);
        chain.iter().any(|c| c.fields.iter().any(|f| !f.is_static && f.name == n) || c.methods.iter().any(|m| !m.is_static && m.name == n))
    }

    fn content_static(&self, n: &str) -> Option<String> {
        let (chain, _) = self.pkg.chain(&self.class.name);
        chain.iter().find(|c| c.fields.iter().any(|f| f.is_static && f.name == n) || c.methods.iter().any(|m| m.is_static && m.name == n))
            .and_then(|c| self.pkg.idents.get(&c.name).cloned())
    }

    fn runtime_base(&self) -> Option<String> { self.pkg.chain(&self.class.name).1 }

    /// a field kept in the engine's state containers
    fn stored(&self, n: &str) -> bool { self.pkg.stored.contains(n) }

    /// a name some class of the package's own defines as a method
    fn content_members_method(&self, n: &str) -> bool {
        self.pkg.model.classes.values().any(|c| c.methods.iter().any(|m| m.name == n))
    }

    /// a read of a field kept in a container
    fn stored_read(&self, e: &Expr) -> bool {
        match e {
            Expr::Member(_, n) => self.stored(n),
            Expr::Name(n) => self.local_name(n).is_none() && (self.own_field(n, false) || self.own_field(n, true)),
            _ => false,
        }
    }

    /// a field this class or one it extends declares (static or not), kept in a container
    fn own_field(&self, n: &str, is_static: bool) -> bool {
        let (chain, _) = self.pkg.chain(&self.class.name);
        self.stored(n) && chain.iter().any(|c| c.fields.iter().any(|f| f.is_static == is_static && f.name == n))
    }

    /// the class a receiver names, when it is one of the package's own classes by name
    fn static_of(&self, o: &Expr) -> Option<String> {
        match o {
            Expr::Name(n) if self.local_name(n).is_none() && !self.content_member(n) && self.pkg.class_named(n).is_some() =>
                self.pkg.class_named(n).map(|c| c.name.clone()),
            _ => None,
        }
    }

    /// a stored field read: through its container on an object known to have one, else through
    /// the container when the object has one and the plain field when it does not (an object of a
    /// class of another package's, a literal)
    fn cell_read(&mut self, obj: &str, n: &str, known: bool) -> String {
        let n = ident(n);
        if known { return format!("{obj}.__c_{n}.get()") }
        let v = self.fresh("__o");
        format!("({{ var {v} = {obj}; {v}.__c_{n} != null ? {v}.__c_{n}.get() : {v}.{n}; }})")
    }

    fn cell_write(&mut self, obj: &str, n: &str, val: &str, known: bool) -> String {
        let n = ident(n);
        if known { return format!("{obj}.__c_{n}.set({val})") }
        let v = self.fresh("__o");
        let x = self.fresh("__x");
        format!("{{ var {v} = {obj}; var {x} = {val}; if ({v}.__c_{n} != null) {v}.__c_{n}.set({x}); else {v}.{n} = {x}; }}")
    }

    fn own_getter(&self, n: &str) -> bool {
        let (chain, base) = self.pkg.chain(&self.class.name);
        chain.iter().any(|c| c.methods.iter().any(|m| m.kind == MethodKind::Getter && m.name == n))
            || base.is_some_and(|b| self.pkg.runtime.has_getter(&b, n))
    }

    fn own_setter(&self, n: &str) -> bool {
        let (chain, base) = self.pkg.chain(&self.class.name);
        chain.iter().any(|c| c.methods.iter().any(|m| m.kind == MethodKind::Setter && m.name == n))
            || base.is_some_and(|b| self.pkg.runtime.classes.get(&b).is_some_and(|rc| rc.setters.contains(n)))
    }

    /// A name read through the scope chain, in the order AS3 looks: the function's own variables
    /// (and those of the functions around it), the object's members, the class's statics, the
    /// classes and globals.
    fn name_value(&mut self, n: &str) -> String {
        if let Some(l) = self.local_name(n) { return l }
        let in_static = self.scopes.iter().any(|s| s.is_static);
        if !in_static && self.content_member(n) {
            if self.own_getter(n) { return format!("me.get_{}()", ident(n)) }
            if self.own_field(n, false) { return self.cell_read("me", n, true) }
            return format!("me.{}", ident(n));
        }
        if let Some(c) = self.content_static(n) {
            if self.own_field(n, true) { return self.cell_read(&c, n, true) }
            return format!("{c}.{}", ident(n))
        }
        if let Some(b) = self.runtime_base() {
            if !in_static && self.pkg.runtime.has_member(&b, n) {
                if self.pkg.runtime.has_getter(&b, n) { return format!("me.get_{}()", ident(n)) }
                return format!("me.{}", ident(n));
            }
        }
        if let Some(c) = self.pkg.class_ident(n) { return c }
        match n {
            "undefined" => return "null".into(),
            "NaN" => return "Math.NaN".into(),
            "Infinity" => return "Math.POSITIVE_INFINITY".into(),
            _ => {}
        }
        if AS3_GLOBALS.contains(&n) { return n.to_string() }
        if self.pkg.runtime.globals.contains(n) { return ident(n) }
        // a method the runtime's base keeps as a static, read as a value (handed to a listener):
        // a call bound to this object, through its row where it has one
        if !in_static {
            if let Some(owner) = self.runtime_base().and_then(|b| self.pkg.runtime.static_of(&b, n)) {
                let args: Vec<String> = (0..4).map(|i| format!("__a{i}")).collect();
                let body = self.api_text("me", n, &args)
                    .unwrap_or_else(|| format!("{owner}.{}(me, __a0, __a1, __a2, __a3)", ident(n)));
                return format!("PeptideRuntime.bound(me, \"{n}\", function(?__a0, ?__a1, ?__a2, ?__a3) {{ return {body}; }})");
            }
        }
        // inherited from a class nobody here defines: the object's, as far as can be told
        if !in_static && self.runtime_base().is_some() && !self.class.super_name.is_empty() {
            self.unknown.insert(format!("{}.{n}", self.class.super_name));
            return format!("me.{}", ident(n));
        }
        self.unknown.insert(n.to_string());
        ident(n)
    }

    fn name_place(&mut self, n: &str) -> String {
        let v = self.name_value(n);
        // a getter read cannot be written to: its setter is
        if let Some(p) = v.strip_prefix("me.get_").and_then(|p| p.strip_suffix("()")) { return format!("me.{p}") }
        v
    }

    fn member(&mut self, o: &Expr, n: &str) -> String {
        if n.starts_with('#') { self.problem(format!("a slot by number ({n})")); }
        if let Expr::Name(cls) = o {
            // a class's static, a built-in's constant
            if cls == "Math" && self.local_name(cls).is_none() && !self.content_member(cls) {
                return format!("Math.{}", math_constant(n));
            }
            // the numeric classes' constants
            if matches!(cls.as_str(), "Number" | "int" | "uint") && self.local_name(cls).is_none() && !self.content_member(cls) {
                if let Some(v) = number_constant(cls, n) { return v }
            }
        }
        let obj = self.expr_atom(o);
        // a listed call handed around as a value (a listener) when nothing in the runtime answers
        // it by name: a function that makes the call, on the object it was taken from
        if self.pkg.runtime.calls.contains_key(n) && !self.pkg.runtime.classes.values().any(|c| c.members.contains(n)) {
            let r = self.fresh("__r");
            let args: Vec<String> = (0..4).map(|i| format!("__a{i}")).collect();
            if let Some(body) = self.api_text(&r, n, &args) {
                // the same function each time it is taken from the same object, so a listener
                // added with it is removed with it
                let call = format!("PeptideRuntime.bound({r}, \"{n}\", function(?__a0, ?__a1, ?__a2, ?__a3) {{ return {body}; }})");
                // content defines one by this name too: an object's own, where it has one
                let v = if self.pkg.content_members.contains(n) { format!("{r}.{m} != null ? {r}.{m} : {call}", m = ident(n)) } else { call };
                return format!("({{ var {r} = {obj}; {v}; }})");
            }
        }
        if self.is_me(o) {
            if self.own_getter(n) { return format!("me.get_{}()", ident(n)) }
            if self.stored(n) { return self.cell_read("me", n, self.own_field(n, false)) }
            return format!("me.{}", ident(n));
        }
        if self.stored(n) {
            let known = self.static_of(o).is_some_and(|c| self.pkg.class_named(&c).is_some_and(|m| m.fields.iter().any(|f| f.is_static && f.name == n)));
            return self.cell_read(&obj, n, known);
        }
        if self.pkg.accessors.contains(n) {
            let v = self.fresh("__o");
            return format!("({{ var {v} = {obj}; {v}.get_{m} != null ? {v}.get_{m}() : {v}.{m}; }})", m = ident(n));
        }
        format!("{obj}.{}", ident(n))
    }

    /// an API call written as the Fraymakers call it is (`api_calls.jsonc`), when the member is
    /// listed and content has none by that name
    fn api_call(&mut self, recv: &str, m: &str, args: &[Expr]) -> Option<String> {
        if !self.pkg.runtime.calls.contains_key(m) { return None }
        if let Some(t) = self.native_attack_stats(recv, m, args) { return Some(t) }
        // the source's autolink, where the conversion made it the engine's own: turning it on or
        // off sets the angle of the animation's hitboxes; its point is the animation's own
        if let Some((on, off)) = self.jumps.as_ref().and_then(|j| j.autolink.clone()) {
            if !self.pkg.content_members.contains(m) {
                let code = match m {
                    "startAutolink" => Some(on),
                    "stopAutolink" => Some(off),
                    "setAutolinkPoint" | "setAutolinkRule" => Some(String::new()),
                    _ => None,
                };
                if let Some(code) = code {
                    if code.is_empty() { return Some("null".into()) }
                    let f = self.fresh("__f");
                    return Some(format!("({{ var {f} = {recv}.__fm; {}; }})", code.replace("{fm}", &f)));
                }
            }
        }
        // a landing listener: one whose only work is to take the move to its landing section (it
        // takes itself off, may write the attack's stats, and jumps to one label) is the engine's
        // own landing into the animation that label starts, keeping the attack's state
        if matches!(m, "addEventListener" | "removeEventListener") && !self.pkg.content_members.contains(m) {
            if let [ev, h, ..] = args {
                // a listener that is one of the source's own landing calls: the engine's landing state
                // (the listener's own object's call: the clip's, or the object it listens on, as
                // `self.self.addEventListener(GROUND_TOUCH, self.self.toHeavyLand)`)
                let on_recv = match h { Expr::Member(o, _) if !self.is_me(o) => self.expr(o) == recv, _ => false };
                let lands_in = match h { Expr::Member(o, n) if (self.is_me(o) || on_recv) && !self.pkg.content_members.contains(n) => super::attack_stats::table().landings.get(n).cloned(), _ => None };
                if is_ground_touch(ev) && lands_in.is_some() {
                    let f = self.fresh("__f");
                    let set = if m == "removeEventListener" { "landType: LandType.NONE".to_string() }
                        else { format!("landType: LandType.NORMAL, landAnimation: {}", quote(&lands_in.unwrap_or_default())) };
                    return Some(format!("({{ var {f} = {recv}.__fm; {f}.updateAnimationStats({{ {set} }}); }})"));
                }
                if is_ground_touch(ev) {
                    if let Some(mut landing) = self.landing_of(h) {
                        let label = landing.label.clone();
                        let a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
                        let r = if recv.contains(['(', '[']) { self.fresh("__r") } else { recv.to_string() };
                        let old = self.api_text(&r, m, &a)?;
                        let f = self.fresh("__f");
                        let t = if m == "removeEventListener" {
                            format!("{r}.__landsTo = null; {f}.updateAnimationStats({{ landType: LandType.NONE }}); {old}")
                        } else {
                            let own = match self.jumps.as_ref().and_then(|j| j.labels.get(&label)) { Some((a, true)) => Some(a.clone()), _ => None };
                            let anim = match &own { Some(a) => quote(a), None => format!("DATA.lands({}, {f}.getAnimation())", quote(&label)) };
                            landing.anim = own;
                            self.landings.push(landing);
                            let l = self.fresh("__l");
                            // the animation is kept on the object too: one made on the floor lands at once
                            format!("var {l} = {anim}; {r}.__landsTo = {l}; if ({l} != null) {f}.updateAnimationStats({{ landType: LandType.TOUCH, landAnimation: {l} }}) else {old}")
                        };
                        let bind = if r != recv { format!("var {r} = {recv}; ") } else { String::new() };
                        return Some(format!("({{ {bind}var {f} = {r}.__fm; {t}; }})"));
                    }
                }
            }
        }
        // a listener for an event the engine raises: the engine's own listener, on the object's
        // handle (an object with none, the runtime's)
        if matches!(m, "addEventListener" | "removeEventListener" | "hasEventListener") && !self.pkg.content_members.contains(m) {
            let named = args.first().and_then(|e| super::events::named(self.pkg.runtime, e));
            if let Some((ev, engine)) = named.and_then(|n| Some((n.clone(), super::events::table().engine.get(&n)?.clone()))) {
                if let Some(t) = self.engine_listener(recv, m, args, &ev, &engine) { return Some(t) }
            }
        }
        // a fighter's jump by label from its clip's own code, while the engine shows that clip
        if m == "playFrame" && !self.pkg.content_members.contains(m) {
            if let [Expr::Str(label)] = args {
                let r = if recv.contains(['(', '[']) { self.fresh("__r") } else { recv.to_string() };
                let old = self.api_text(&r, m, &[format!("\"{label}\"")])?;
                if let Some(t) = self.native_jump(&format!("{r}.__fm"), label, &old) {
                    return Some(if r != recv { format!("({{ var {r} = {recv}; {t}; }})") } else { t });
                }
            }
        }
        // an attack stat the engine keeps natively, read back from its own field
        if m == "getAnimationStat" && !self.pkg.content_members.contains(m) {
            if let [Expr::Str(key)] = args {
                if let Some((field, back)) = super::attack_stats::table().read(key) {
                    let r = self.fresh("__v");
                    return Some(format!("({{ var {r} = {recv}.__fm.getAnimationStat(\"{field}\"); {}; }})", back.replace("{v}", &r)));
                }
            }
        }
        let a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
        if self.pkg.content_members.contains(m) {
            // content defines one by this name too: an object that has its own answers, any
            // other gets the call
            let r = self.fresh("__r");
            let t = self.api_text(&r, m, &a)?;
            return Some(format!("({{ var {r} = {recv}; {r}.{id} != null ? {r}.{id}({args}) : {t}; }})",
                id = ident(m), args = a.join(", ")));
        }
        self.api_text(recv, m, &a)
    }

    /// `recv.addEventListener(event, fn, opts)` (or its removal, or the question) for an event the
    /// engine raises, as the engine's call on the object's handle with the listener it hands the
    /// source's record to (`Events.handler`: the same one for the same listener, so one added is
    /// taken off). a fighter's lasts until its state changes unless added to persist, as the
    /// engine's own do; anything else's until it is taken off
    fn engine_listener(&mut self, recv: &str, m: &str, args: &[Expr], ev: &str, engine: &str) -> Option<String> {
        let a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
        let r = self.fresh("__r");
        let f = self.fresh("__f");
        let h = self.fresh("__h");
        let old = self.api_text(&r, m, &a)?;
        let fnx = a.get(1)?;
        let mut binds = vec![format!("var {r} = {recv}"), format!("var {f} = {r}.__fm")];
        let native = match m {
            "addEventListener" => {
                let keep = match a.get(2) {
                    Some(o) => { let v = self.fresh("__o"); binds.push(format!("var {v} = {o}")); format!("({v} != null && AS3.truthy(AS3.at({v}, \"persistent\"))) || ") }
                    None => String::new(),
                };
                format!("{{ var {h} = Events.handler({r}, {q}, {fnx}); {f}.removeEventListener({engine}, {h}); {f}.addEventListener({engine}, {h}, {{ persistent: {keep}{f}.getType() != EntityType.CHARACTER }}); }}", q = quote(ev))
            }
            "removeEventListener" => format!("{f}.removeEventListener({engine}, Events.handler({r}, {q}, {fnx}))", q = quote(ev)),
            _ => format!("{f}.hasEventListener({engine}, Events.handler({r}, {q}, {fnx}))", q = quote(ev)),
        };
        Some(format!("({{ {}; {f} != null ? {native} : {old}; }})", binds.join("; ")))
    }

    /// an attack's stats written while it plays, as the engine's own animation stats: each stat
    /// `runtime/attack_stats.jsonc` maps, given as a literal, is set with `updateAnimationStats`
    /// on the spot; the rest, if any, go the listed call's way
    fn native_attack_stats(&mut self, recv: &str, m: &str, args: &[Expr]) -> Option<String> {
        if m != "updateAnimationStats" || self.pkg.content_members.contains(m) { return None }
        let [Expr::Object(pairs)] = args else { return None };
        let table = super::attack_stats::table();
        let lit = |e: &Expr| -> Option<f64> {
            match e {
                Expr::Num(v) => Some(*v),
                Expr::Int(v) => Some(*v as f64),
                Expr::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                Expr::Unary(UnOp::Neg, x) => match &**x { Expr::Num(v) => Some(-v), Expr::Int(v) => Some(-(*v as f64)), _ => None },
                Expr::Coerce(_, x) | Expr::Convert(_, x) => match &**x { Expr::Num(v) => Some(*v), Expr::Int(v) => Some(*v as f64), Expr::Bool(b) => Some(if *b { 1.0 } else { 0.0 }), _ => None },
                _ => None,
            }
        };
        let mut native: Vec<(String, String)> = Vec::new();
        let mut ranks = BTreeMap::new();
        let mut clamps: Vec<String> = Vec::new();
        let mut rest: Vec<(Expr, Expr)> = Vec::new();
        for (k, v) in pairs {
            let key = match k { Expr::Str(s) => s.clone(), _ => { rest.push((k.clone(), v.clone())); continue } };
            match lit(v) {
                Some(n) if table.put(&mut native, &mut ranks, &key, n, false) => clamps.extend(table.clamps(&key, n)),
                _ => rest.push((k.clone(), v.clone())),
            }
        }
        if native.is_empty() { return None }
        let r = if recv.contains(['(', '[']) { self.fresh("__r") } else { recv.to_string() };
        let body: Vec<String> = native.iter().map(|(f, t)| format!("{f}: {t}")).collect();
        let mut parts = vec![format!("{r}.__fm.updateAnimationStats({{ {} }})", body.join(", "))];
        for c in clamps { parts.push(format!("if ({r}.__fm.getYVelocity() > {c}) {r}.__fm.setYVelocity({c})")); }
        if !rest.is_empty() {
            let obj = self.expr(&Expr::Object(rest));
            parts.push(self.api_text(&r, m, &[obj])?);
        }
        // the engine's stats are a fighter's: anything else (a projectile's clip writing its
        // attack's stats) goes the listed call's way with all of them
        let all = self.expr(&Expr::Object(pairs.clone()));
        let other = self.api_text(&r, m, &[all])?;
        let bind = if r != recv { format!("var {r} = {recv}; ") } else { String::new() };
        Some(format!("({{ {bind}if ({r}.__fm != null && {r}.__fm.getType() == EntityType.CHARACTER) {{ {}; }} else {other}; }})", parts.join("; ")))
    }

    /// what a landing listener does: a method of this object's whose statements only take this
    /// same listener off, write the attack's or its boxes' stats as plain values, and jump to one
    /// literal label
    fn landing_of(&self, h: &Expr) -> Option<super::Landing> {
        let name = match h { Expr::Member(o, n) if self.is_me(o) => n, Expr::Name(n) => n, _ => return None };
        let (chain, _) = self.pkg.chain(&self.class.name);
        let method = chain.iter().flat_map(|c| c.methods.iter()).find(|x| x.name == *name && !x.is_static && x.kind == MethodKind::Method)?;
        let mut out = super::Landing::default();
        let mut label = None;
        let pairs = |o: &[(Expr, Expr)]| -> Option<Vec<(String, f64)>> {
            o.iter().map(|(k, v)| match k { Expr::Str(k) => literal_num(v).map(|n| (k.clone(), n)), _ => None }).collect()
        };
        for s in &method.function.body {
            match s {
                Stmt::Return(None) => {}
                Stmt::Expr(Expr::Call(c, a)) => {
                    let Expr::Member(_, m) = &**c else { return None };
                    match (m.as_str(), a.as_slice()) {
                        ("removeEventListener", [ev, x]) if is_ground_touch(ev) && matches!(x, Expr::Member(_, n2) | Expr::Name(n2) if n2 == name) => {}
                        ("updateAttackStats" | "updateAnimationStats", [Expr::Object(o)]) => out.attack.extend(pairs(o)?),
                        ("updateHitboxStats", [i, Expr::Object(o)]) => out.boxes.push((literal_num(i)? as u32, pairs(o)?)),
                        ("gotoAndStop" | "gotoAndPlay" | "playFrame" | "stancePlayFrame", [Expr::Str(l)]) if label.is_none() => label = Some(l.clone()),
                        _ => return None,
                    }
                }
                _ => return None,
            }
        }
        out.label = label?;
        Some(out)
    }

    /// a jump by label as the engine's own: the animation that shows the label when it is that
    /// animation's first frame, else the label in the animation playing. only while the engine
    /// shows this clip (one of its animations plays); otherwise the jump goes `old`'s way
    fn native_jump(&mut self, fm: &str, label: &str, old: &str) -> Option<String> {
        let j = self.jumps.clone()?;
        let (anim, start) = j.labels.get(label)?.clone();
        let f = self.fresh("__f");
        let a = self.fresh("__a");
        Some(if start {
            let guard: Vec<String> = j.anims.iter().map(|x| format!("{a} == \"{x}\"")).collect();
            format!("({{ var {f} = {fm}; var {a} = {f}.getAnimation(); if ({}) {f}.playAnimation(\"{anim}\") else {old}; }})", guard.join(" || "))
        } else {
            format!("({{ var {f} = {fm}; if ({f}.getAnimation() == \"{anim}\") {f}.playFrameLabel(\"{label}\") else {old}; }})")
        })
    }

    /// a listed call's template filled in with a receiver and argument texts
    pub(crate) fn api_text(&mut self, recv: &str, m: &str, args: &[String]) -> Option<String> {
        let t = self.pkg.runtime.calls.get(m)?.clone();
        let scaling = &crate::mappings::character_stats().scaling;
        // a receiver that is worked out (a call, an index) and used more than once is worked out once
        let uses = t.matches("{recv}").count() + t.matches("{eng}").count();
        let bind = uses > 1 && recv.contains(['(', '[']);
        let r = if bind { self.fresh("__r") } else { recv.to_string() };
        let mut out = t.replace("{eng}", &format!("{r}.__fm")).replace("{recv}", &r)
            .replace("{size}", &scaling.size_multiplier.to_string())
            .replace("{speed}", &scaling.velocity_scale().to_string());
        // an argument used more than once is worked out once, like the receiver
        let mut binds: Vec<(String, String)> = Vec::new();
        if bind { binds.push((r.clone(), recv.to_string())); }
        for (i, v) in args.iter().enumerate() {
            let slot = format!("{{{i}}}");
            let plain = v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
            let v = if out.matches(&slot).count() > 1 && !plain {
                let a = self.fresh("__a");
                binds.push((a.clone(), v.clone()));
                a
            } else { format!("({v})") };
            out = out.replace(&slot, &v);
        }
        // an argument the call leaves out is the source's undefined
        for i in args.len()..8 { out = out.replace(&format!("{{{i}}}"), "null"); }
        // bare when it is one call or one value: a statement that opens with a bracket reads as a
        // call on whatever came before it (a closing `}` included). bracketed when it computes
        // bracketed unless it is one call or one value: a template is spliced into any expression
        let mut depth = 0i32;
        let mut top = String::new();
        for ch in t.chars() {
            match ch { '(' | '{' | '[' => depth += 1, ')' | '}' | ']' => depth -= 1, _ if depth == 0 => top.push(ch), _ => {} }
        }
        let computes = top.contains(' ') || top.starts_with('!');
        // several calls in a row: a block, whose value is the last one's
        let out = if top.contains(';') { format!("({{ {out}; }})") } else if computes { format!("({out})") } else { out };
        if binds.is_empty() { return Some(out) }
        let vars: String = binds.iter().map(|(n, v)| format!("var {n} = {v}; ")).collect();
        Some(format!("({{ {vars}{out}; }})"))
    }

    /// a call
    fn call(&mut self, callee: &Expr, args: &[Expr]) -> String {
        // the clip's own jump by label, while the engine shows this clip: the engine's own jump
        if let (Expr::Member(recv, m), [Expr::Str(label)]) = (callee, args) {
            if matches!(m.as_str(), "gotoAndPlay" | "gotoAndStop") && self.is_me(recv) {
                let old = format!("me.{m}(\"{label}\")");
                if let Some(t) = self.native_jump("self", label, &old) { return t }
            }
        }
        // a method the runtime's base class keeps as its static (`PeptideStage.x = function(me, ...)`),
        // asked of this object: the static, handed it (a row for it, where there is one, still wins)
        let own_call = match callee {
            Expr::Member(o, m) if self.is_me(o) => Some(m),
            Expr::Name(n) if self.local_name(n).is_none() => Some(n),
            _ => None,
        };
        if let Some(m) = own_call {
            let in_static = self.scopes.iter().any(|s| s.is_static);
            if !in_static && !self.content_member(m) && !self.pkg.runtime.calls.contains_key(m.as_str()) {
                if let Some(owner) = self.runtime_base().and_then(|b| self.pkg.runtime.static_of(&b, m)) {
                    let a = self.args(args);
                    return format!("{owner}.{}(me{}{a})", ident(m), if a.is_empty() { "" } else { ", " });
                }
            }
        }
        // an array a container holds, changed in place: a copy is changed and put in its place,
        // since a rollback puts back what the container holds, not what was done to it
        if let Expr::Member(recv, m) = callee {
            if ARRAY_MUTATORS.contains(&m.as_str()) && self.stored_read(recv) {
                if let Some(place) = place_of(recv) {
                    let read = self.expr(recv);
                    let (t, r) = (self.fresh("__cw"), self.fresh("__cr"));
                    self.scopes.last_mut().map(|s| s.names.insert(t.clone()));
                    let call = self.call(&Expr::Member(Box::new(Expr::Name(t.clone())), m.clone()), args);
                    let write = self.assign(&place, &Expr::Name(t.clone()));
                    self.scopes.last_mut().map(|s| s.names.remove(&t));
                    return format!("({{ var {t} = AS3.copyOf({read}); var {r} = {call}; {write}; {r}; }})");
                }
            }
        }
        self.call_inner(callee, args)
    }

    fn call_inner(&mut self, callee: &Expr, args: &[Expr]) -> String {
        // a field that holds a function, called: the function its container holds
        let field_call = match callee {
            Expr::Member(_, n) => self.stored(n) && !self.content_members_method(n),
            Expr::Name(n) => self.local_name(n).is_none() && (self.own_field(n, false) || self.own_field(n, true)),
            _ => false,
        };
        if field_call {
            let f = self.expr(callee);
            return format!("({f})({})", self.args(args));
        }
        // an API member asked of this object by its bare name, inside a class that extends an API class
        if let Expr::Name(n) = callee {
            if self.local_name(n).is_none() && !self.content_member(n) && self.runtime_base().is_some() {
                if let Some(c) = self.api_call("me", n, args) { return c }
            }
        }
        // AS3's own top-level functions and the parts of its standard library that differ here
        if let Expr::Name(n) = callee {
            if self.local_name(n).is_none() && !self.content_member(n) && self.content_static(n).is_none() {
                let a = self.args(args);
                let one = |f: &str| format!("{f}({a})");
                match n.as_str() {
                    "int" => return if args.len() == 1 && self.is_int(&args[0]) { a } else { one("AS3.int") },
                    "uint" => return one("AS3.uint"),
                    "Number" => return one("AS3.num"),
                    "String" => return one("AS3.str"),
                    "Boolean" => return one("AS3.truthy"),
                    "isNaN" => return format!("Math.isNaN(AS3.num({a}))"),
                    "isFinite" => return format!("Math.isFinite(AS3.num({a}))"),
                    "parseInt" => return format!("Std.parseInt({})", self.expr(&args[0])),
                    "parseFloat" => return format!("Std.parseFloat({})", self.expr(&args[0])),
                    "trace" => return format!("AS3.trace([{a}])"),
                    "getQualifiedClassName" => return one("AS3.className"),
                    "getDefinitionByName" => return one("AS3.definition"),
                    "Array" => return format!("AS3.array([{a}])"),
                    "Object" => return if args.is_empty() { "{}".into() } else { a },
                    // `Class(x)` is a cast: the value itself, once it is that class
                    _ if args.len() == 1 && self.pkg.class_ident(n).is_some() => return a,
                    _ => {}
                }
            }
        }
        if let Expr::Member(o, m) = callee {
            let static_side = matches!(&**o, Expr::Name(n) if self.local_name(n).is_none() && !self.content_member(n)
                && (self.pkg.runtime.provides(n) || self.pkg.class_named(n).is_some()));
            // this object's own member, where its class has one
            let own = self.is_me(o) && self.content_member(m);
            if !static_side && !own && self.pkg.runtime.calls.contains_key(m.as_str()) {
                let recv = self.expr_atom(o);
                if let Some(c) = self.api_call(&recv, m, args) { return c }
            }
            // a static of the runtime's that is plain arithmetic: written in place (`Class.member` rows)
            if let Expr::Name(cls) = &**o {
                let key = format!("{cls}.{m}");
                if static_side && self.pkg.runtime.calls.contains_key(&key) {
                    let a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
                    if let Some(c) = self.api_text("null", &key, &a) { return c }
                }
            }
            if let Expr::Name(cls) = &**o {
                if cls == "Math" && self.local_name(cls).is_none() {
                    let a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
                    return match m.as_str() {
                        "random" => "AS3.random()".into(),
                        // AS3's max and min take any number of values
                        "max" | "min" if a.len() > 2 => a[1..].iter().fold(a[0].clone(), |acc, x| format!("Math.{m}({acc}, {x})")),
                        _ => format!("Math.{m}({})", a.join(", ")),
                    };
                }
            }
            // an array's own methods that differ here
            match (m.as_str(), args.len()) {
                ("sort", 0) | ("sortOn", _) => return format!("AS3.{m}({}, [{}])", self.expr(o), self.args(args)),
                ("sort", _) if !matches!(args[0], Expr::Function(_) | Expr::Member(..) | Expr::Name(_) | Expr::Local(_)) =>
                    return format!("AS3.sort({}, [{}])", self.expr(o), self.args(args)),
                ("forEach" | "some" | "every", 1) => return format!("AS3.{m}({}, {})", self.expr(o), self.expr(&args[0])),
                _ => {}
            }
            // the language's methods on numbers, strings and arrays that this engine's values do
            // not have or answer differently (a class of the package's own that defines one keeps it)
            let own = self.static_class(o).is_some_and(|t| self.pkg.class_has(&t, m) == Some(true) && self.pkg.class_named(&t).is_some());
            if !own {
                match (m.as_str(), args.len()) {
                    ("toString" | "slice" | "concat", _) => return format!("AS3.{m}({}, [{}])", self.expr(o), self.args(args)),
                    ("toFixed", _) => return format!("AS3.toFixed({}{})", self.expr(o), args.first().map(|a| format!(", {}", self.expr(a))).unwrap_or_default()),
                    ("replace", 2) => return format!("AS3.replace({}, {}, {})", self.expr(o), self.expr(&args[0]), self.expr(&args[1])),
                    ("reverse", 0) => return format!("AS3.reverse({})", self.expr(o)),
                    _ => {}
                }
            }
            // a member nothing here or in the runtime defines: reported, since nothing answers it
            // (an array's, a string's and the language's own members aside)
            if !self.pkg.members.contains(m.as_str()) && !self.pkg.members.contains(&ident(m)) && !LANGUAGE_MEMBERS.contains(&m.as_str())
                && !matches!(&**o, Expr::Name(c) if c == "Math") {
                self.unknown.insert(format!("member {m}"));
            } else if let Some(t) = self.static_class(o) {
                // a receiver the source declares a class for is held to that class
                if self.pkg.class_has(&t, m) == Some(false) { self.unknown.insert(format!("{t}.{m}")); }
            }
            let obj = self.expr_atom(o);
            let a = self.args(args);
            // a member that takes the rest of its arguments as one array is handed them as one
            if self.pkg.runtime.variadic.contains(m.as_str()) && !self.content_member(m) {
                return format!("{obj}.{}([{a}])", ident(m));
            }
            if self.is_me(o) && !self.content_member(m) {
                if let Some(b) = self.runtime_base() {
                    if !self.pkg.runtime.has_member(&b, m) { self.unknown.insert(format!("{}.{m}", self.class.super_name)); }
                }
            }
            // one of content's own methods asked of another object: it may be an object whose class
            // this package does not carry (an item from another package), so the call is made only
            // when it has the method, and reported as a gap when it does not
            let api = self.pkg.runtime.classes.values().any(|c| c.members.contains(m.as_str()) || c.getters.contains(m.as_str()));
            let static_side = matches!(&**o, Expr::Name(n) if self.local_name(n).is_none() && !self.content_member(n)
                && (self.pkg.runtime.provides(n) || self.pkg.class_named(n).is_some()));
            if !self.is_me(o) && !api && !static_side && !LANGUAGE_MEMBERS.contains(&m.as_str()) && self.pkg.members.contains(m.as_str()) {
                let v = self.fresh("__o");
                let f = ident(m);
                return format!("({{ var {v} = {obj}; {v}.{f} != null ? {v}.{f}({a}) : PeptideRuntime.missing({}); }})", quote(m));
            }
            return format!("{obj}.{}({a})", ident(m));
        }
        if let Expr::Name(n) = callee {
            if self.pkg.runtime.variadic.contains(n.as_str()) && self.local_name(n).is_none() && !self.content_member(n) {
                let f = self.name_value(n);
                return format!("{f}([{}])", self.args(args));
            }
        }
        let f = self.expr_atom(callee);
        format!("{f}({})", self.args(args))
    }

    fn new_(&mut self, c: &Expr, args: &[Expr]) -> String {
        let a = self.args(args);
        match c {
            Expr::Name(n) => match n.as_str() {
                "Array" => format!("AS3.array([{a}])"),
                "Object" => "{}".into(),
                _ => {
                    let cls = match self.pkg.class_ident(n) {
                        Some(i) => i,
                        None => { self.unknown.insert(format!("class {n}")); ident(n) }
                    };
                    format!("{cls}.create({a})")
                }
            },
            // a typed array (`Vector.<int>`) is an array here
            Expr::TypeApply(..) => format!("AS3.array([{a}])"),
            other => { let x = self.expr_atom(other); format!("{x}.create({a})") }
        }
    }

    // ---- types ---------------------------------------------------------------------------------------------

    fn field_type(&self, n: &str) -> Option<String> {
        let (chain, _) = self.pkg.chain(&self.class.name);
        chain.iter().flat_map(|c| c.fields.iter()).find(|f| f.name == n).and_then(|f| f.type_name.clone())
    }

    fn method_returns(&self, n: &str) -> Option<String> {
        let (chain, _) = self.pkg.chain(&self.class.name);
        chain.iter().flat_map(|c| c.methods.iter()).find(|m| m.name == n).and_then(|m| m.function.return_type.clone())
    }

    fn is_bool(&self, e: &Expr) -> bool {
        match e {
            Expr::Bool(_) => true,
            Expr::Binary(op, ..) => matches!(op, BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe | BinOp::Lt
                | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In | BinOp::InstanceOf | BinOp::Is),
            Expr::Unary(UnOp::Not, _) | Expr::Convert(Conv::Boolean, _) | Expr::HasNext2 { .. } => true,
            Expr::And(a, b) | Expr::Or(a, b) => self.is_bool(a) && self.is_bool(b),
            Expr::Cond(_, a, b) => self.is_bool(a) && self.is_bool(b),
            Expr::Local(r) => self.scopes.last().is_some_and(|s| s.bool_regs.contains(r)),
            Expr::Member(o, n) if self.is_me(o) => self.field_type(n).as_deref() == Some("Boolean"),
            Expr::Name(n) if self.local_name(n).is_none() => self.field_type(n).as_deref() == Some("Boolean"),
            Expr::Call(c, _) => match &**c {
                Expr::Member(o, m) if self.is_me(o) => self.method_returns(m).as_deref() == Some("Boolean"),
                Expr::Name(m) if self.local_name(m).is_none() => self.method_returns(m).as_deref() == Some("Boolean"),
                _ => false,
            },
            _ => false,
        }
    }

    /// a string, by what the source states
    fn is_string(&self, e: &Expr) -> bool {
        let typed = |t: Option<String>| t.as_deref().map(short) == Some("String");
        match e {
            Expr::Str(_) | Expr::Convert(Conv::String, _) => true,
            Expr::Coerce(t, _) => short(t) == "String",
            Expr::Binary(BinOp::Add, a, b) => self.is_string(a) || self.is_string(b),
            Expr::Member(o, n) if self.is_me(o) => typed(self.field_type(n)),
            Expr::Name(n) if self.local_name(n).is_none() => typed(self.field_type(n)),
            _ => false,
        }
    }

    /// a string, a number or a boolean, by what the source states: AS3 and this engine write it alike
    fn is_primitive(&self, e: &Expr) -> bool {
        let typed = |t: Option<String>| matches!(t.as_deref().map(short), Some("String" | "Number" | "int" | "uint" | "Boolean"));
        match e {
            Expr::Str(_) | Expr::Num(_) | Expr::Int(_) | Expr::Bool(_) => true,
            Expr::Convert(c, _) => !matches!(c, Conv::Object),
            Expr::Coerce(t, _) => matches!(short(t), "String" | "Number" | "int" | "uint" | "Boolean"),
            Expr::Binary(BinOp::Add, a, b) => self.is_primitive(a) && self.is_primitive(b) || self.is_string(a) || self.is_string(b),
            Expr::Binary(BinOp::As, ..) => false,
            Expr::Binary(..) | Expr::Unary(..) => true,
            Expr::Member(_, n) if n == "length" => true,
            Expr::Member(o, n) if self.is_me(o) => typed(self.field_type(n)),
            Expr::Name(n) if self.local_name(n).is_none() => typed(self.field_type(n)),
            _ => self.is_int(e),
        }
    }

    fn is_int(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(i) => *i >= i32::MIN as i64 && *i <= i32::MAX as i64,
            Expr::Convert(Conv::Int, _) => true,
            Expr::Binary(op, a, b) => match op {
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => true,
                BinOp::Add | BinOp::Sub | BinOp::Mul => self.is_int(a) && self.is_int(b),
                _ => false,
            },
            Expr::Unary(UnOp::Neg, x) => self.is_int(x),
            Expr::Local(r) => self.scopes.last().is_some_and(|s| s.int_regs.contains(r)),
            // an array's or a string's length is an int in AS3
            Expr::Member(_, n) if n == "length" => true,
            Expr::Member(o, n) if self.is_me(o) => matches!(self.field_type(n).as_deref(), Some("int")) || self.int_fields.contains(n),
            Expr::Name(n) if self.local_name(n).is_none() => matches!(self.field_type(n).as_deref(), Some("int")) || self.int_fields.contains(n),
            _ => false,
        }
    }
}

/// names this compiler writes itself, which a source name must not take
const RESERVED_HERE: &[&str] = &["me", "AS3", "self", "match", "stage", "camera", "Math", "Std"];

/// members of AS3's own types (Array, String, Object, Function) that behave here as they do there
const LANGUAGE_MEMBERS: &[&str] = &["push", "pop", "shift", "unshift", "splice", "slice", "concat", "join",
    "indexOf", "lastIndexOf", "reverse", "sort", "filter", "map", "charAt", "charCodeAt", "split", "substr",
    "substring", "toLowerCase", "toUpperCase", "toString", "hasOwnProperty", "call", "apply", "toFixed"];

/// AS3's top-level names that are the same here
const AS3_GLOBALS: &[&str] = &["Math", "NaN", "Infinity"];

/// A key set as the record the runtime reads literals through (`AS3.at` and the rest): the keys,
/// and a reader and a writer by key.
fn shape_literal(keys: &[String]) -> String {
    let gets: Vec<String> = keys.iter().map(|k| format!("case {}: return o.{};", quote(k), ident(k))).collect();
    let sets: Vec<String> = keys.iter().map(|k| format!("case {}: o.{} = v;", quote(k), ident(k))).collect();
    format!("{{ names: [{}], get: function(o, k) {{ switch (k) {{ {} default: return o.__keys == null ? null : o.__keys.get(k); }} }}, set: function(o, k, v) {{ switch (k) {{ {} default: AS3.putKey(o, k, v); }} }} }}",
        keys.iter().map(|k| quote(k)).collect::<Vec<_>>().join(", "), gets.join(" "), sets.join(" "))
}

fn number_constant(class: &str, n: &str) -> Option<String> {
    Some(match (class, n) {
        ("Number", "MAX_VALUE") => "1.7976931348623157e308".into(),
        ("Number", "MIN_VALUE") => "5e-324".into(),
        ("Number", "NaN") => "Math.NaN".into(),
        ("Number", "POSITIVE_INFINITY") => "Math.POSITIVE_INFINITY".into(),
        ("Number", "NEGATIVE_INFINITY") => "Math.NEGATIVE_INFINITY".into(),
        ("int", "MAX_VALUE") => "2147483647".into(),
        ("int", "MIN_VALUE") => "(-2147483648)".into(),
        ("uint", "MAX_VALUE") => "4294967295.0".into(),
        ("uint", "MIN_VALUE") => "0".into(),
        _ => return None,
    })
}

fn math_constant(n: &str) -> String {
    match n {
        "E" => "exp(1)".into(),
        "LN2" => "log(2)".into(),
        "LN10" => "log(10)".into(),
        "LOG2E" => "(1 / Math.log(2))".into(),
        "LOG10E" => "(1 / Math.log(10))".into(),
        "SQRT2" => "sqrt(2)".into(),
        "SQRT1_2" => "sqrt(0.5)".into(),
        other => other.to_string(),
    }
}

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

fn number(n: f64) -> String {
    if n.is_nan() { return "Math.NaN".into() }
    if n.is_infinite() { return if n > 0.0 { "Math.POSITIVE_INFINITY".into() } else { "Math.NEGATIVE_INFINITY".into() } }
    if n.fract() == 0.0 && n.abs() < 2147483648.0 { return format!("{}", n as i64) }
    let s = format!("{n}");
    if s.contains('.') || s.contains('e') { s } else { format!("{s}.0") }
}

/// a key that is a plain name, which reads and writes as a field
fn literal_key(k: &Expr) -> Option<String> {
    match k {
        Expr::Str(s) if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !s.starts_with(|c: char| c.is_ascii_digit()) && !RESERVED_HERE.contains(&s.as_str()) => Some(s.clone()),
        _ => None,
    }
}

/// every register a body reads or writes (not its closures')
fn walk_places(body: &[Stmt], f: &mut impl FnMut(u32)) {
    fn e(x: &Expr, f: &mut dyn FnMut(u32)) {
        match x {
            Expr::Local(r) => f(*r),
            Expr::HasNext2 { object, index } => { f(*object); f(*index) }
            Expr::Assign(p, _) => if let Place::Local(r) = **p { f(r) },
            Expr::Function(_) => return,
            _ => {}
        }
        for c in x.children() { e(c, f) }
    }
    fn s(list: &[Stmt], f: &mut dyn FnMut(u32)) {
        for st in list {
            if let Stmt::Assign(Place::Local(r), _) = st { f(*r) }
            for x in st.exprs() { e(x, f) }
            for b in st.blocks() { s(b, f) }
        }
    }
    s(body, f)
}

/// every temp and join a body assigns (so each is declared even where it is never read)
fn walk_assigned_vars(body: &[Stmt], out: &mut BTreeSet<String>) {
    for st in body {
        match st {
            Stmt::Assign(Place::Temp(t), _) => { out.insert(format!("__t{t}")); }
            Stmt::Assign(Place::Join { block, slot }, _) => { out.insert(format!("__j{block}_{slot}")); }
            _ => {}
        }
        for b in st.blocks() { walk_assigned_vars(b, out) }
    }
}

/// which registers only ever hold an int, and which only a Boolean: a register is one if every
/// value it is given is (its parameter's declared type counts as a value it is given)
/// a class's untyped fields that only ever hold whole numbers: each starts as a whole number and
/// every write the class makes to it is one (a literal, a count of something, one of these plus or
/// minus another), and nothing else in the package writes a member of that name on some other
/// object. such a field compares and counts as a number without the runtime's general rules:
/// krazoa palace's stage walks a picture with two of them as its counters, four thousand turns a
/// frame, and ran at 26 frames a second through the general comparison
fn whole_number_fields(pkg: &Package, class: &ClassModel) -> BTreeSet<String> {
    let untyped = |t: &Option<String>| matches!(t.as_deref().map(super::short), None | Some("*") | Some("Object"));
    let mut fields: BTreeSet<String> = class.fields.iter()
        .filter(|f| !f.is_static && untyped(&f.type_name) && matches!(f.default, None | Some(Expr::Int(_))))
        .map(|f| f.name.clone()).collect();
    if fields.is_empty() { return fields }
    let declared_int: BTreeSet<String> = class.fields.iter()
        .filter(|f| matches!(f.type_name.as_deref().map(super::short), Some("int" | "uint")))
        .map(|f| f.name.clone()).collect();
    // a member of that name written on any other object anywhere in the package: not ours to judge
    for c in pkg.model.classes.values() {
        for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor]) {
            let mine = std::ptr::eq(c, class);
            let copies = this_copies(f);
            fn each(list: &[Stmt], out: &mut Vec<(Expr, String)>) {
                for st in list {
                    if let Stmt::Assign(Place::Member(o, n), _) = st { out.push((o.clone(), n.clone())); }
                    for b in st.blocks() { each(b, out) }
                }
            }
            let mut writes = Vec::new();
            each(&f.body, &mut writes);
            for (o, n) in writes {
                let on_me = matches!(o, Expr::This) || matches!(o, Expr::Local(r) if copies.contains(&r)) || matches!(&o, Expr::ScopeOf(m) if *m == n);
                if !(mine && on_me) { fields.remove(&n); }
            }
        }
    }
    // the writes the class makes, each with its function's registers to read a value through
    struct Write<'f> { value: Expr, defs: BTreeMap<u32, Vec<&'f Expr>> }
    let mut writes: Vec<(String, Write)> = Vec::new();
    for f in class.methods.iter().map(|m| &m.function).chain([&class.constructor]) {
        let copies = this_copies(f);
        let mut defs: BTreeMap<u32, Vec<&Expr>> = BTreeMap::new();
        let mut mine: Vec<(String, Expr)> = Vec::new();
        fn each<'f>(list: &'f [Stmt], copies: &BTreeSet<u32>, defs: &mut BTreeMap<u32, Vec<&'f Expr>>, mine: &mut Vec<(String, Expr)>) {
            for st in list {
                match st {
                    Stmt::Assign(Place::Local(r), v) => defs.entry(*r).or_default().push(v),
                    Stmt::Assign(Place::Member(o, n), v) if matches!(o, Expr::This) || matches!(o, Expr::Local(r) if copies.contains(r))
                        || matches!(o, Expr::ScopeOf(m) if m == n) => mine.push((n.clone(), v.clone())),
                    Stmt::Assign(Place::Name(n), v) => mine.push((n.clone(), v.clone())),
                    _ => {}
                }
                for b in st.blocks() { each(b, copies, defs, mine) }
            }
        }
        each(&f.body, &copies, &mut defs, &mut mine);
        for (n, v) in mine { writes.push((n, Write { value: v, defs: defs.clone() })); }
    }
    fn whole(e: &Expr, w: &BTreeMap<u32, Vec<&Expr>>, fields: &BTreeSet<String>, ints: &BTreeSet<String>, depth: u32) -> bool {
        if depth > 8 { return false }
        let me = |o: &Expr| matches!(o, Expr::This | Expr::Local(_) | Expr::ScopeOf(_));
        match e {
            Expr::Int(i) => *i >= i32::MIN as i64 && *i <= i32::MAX as i64,
            Expr::Convert(Conv::Int, _) => true,
            Expr::Convert(Conv::Number, x) => whole(x, w, fields, ints, depth + 1),
            Expr::Unary(UnOp::Neg, x) => whole(x, w, fields, ints, depth + 1),
            Expr::Binary(op, a, b) => match op {
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => true,
                BinOp::Add | BinOp::Sub | BinOp::Mul => whole(a, w, fields, ints, depth + 1) && whole(b, w, fields, ints, depth + 1),
                _ => false,
            },
            Expr::Member(_, n) if n == "length" => true,
            Expr::Member(o, n) if me(o) => fields.contains(n) || ints.contains(n),
            Expr::Name(n) => fields.contains(n) || ints.contains(n),
            Expr::Local(r) => w.get(r).is_some_and(|vs| !vs.is_empty() && vs.iter().all(|v| whole(v, w, fields, ints, depth + 1))),
            _ => false,
        }
    }
    // optimistic, then drop any field given something that is not a whole number, until none is
    loop {
        let bad: Vec<String> = writes.iter()
            .filter(|(n, wr)| fields.contains(n) && !whole(&wr.value, &wr.defs, &fields, &declared_int, 0))
            .map(|(n, _)| n.clone()).collect();
        if bad.is_empty() { break }
        for n in bad { fields.remove(&n); }
    }
    fields
}

/// whether working out `e` does nothing besides giving its value: reads, arithmetic, and the making
/// of a value class (inert.jsonc `values`)
fn pure(e: &Expr, values: &BTreeSet<String>) -> bool {
    match e {
        Expr::Undefined | Expr::Null | Expr::Bool(_) | Expr::Num(_) | Expr::Int(_) | Expr::Str(_) | Expr::This | Expr::Local(_) | Expr::Name(_) => true,
        Expr::Member(o, _) => pure(o, values),
        Expr::Index(o, k) => pure(o, values) && pure(k, values),
        Expr::Unary(_, x) | Expr::Convert(_, x) => pure(x, values),
        Expr::Binary(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) => pure(a, values) && pure(b, values),
        Expr::New(c, a) => matches!(&**c, Expr::Name(n) if values.contains(super::short(n))) && a.iter().all(|x| pure(x, values)),
        _ => false,
    }
}

/// the registers past the parameters a function assigns, every time, the object itself
fn this_copies(f: &Function) -> BTreeSet<u32> {
    let mut given: BTreeMap<u32, bool> = BTreeMap::new();
    fn collect(list: &[Stmt], given: &mut BTreeMap<u32, bool>) {
        for st in list {
            if let Stmt::Assign(Place::Local(r), v) = st {
                let only = given.entry(*r).or_insert(true);
                *only = *only && matches!(v, Expr::This);
            }
            for b in st.blocks() { collect(b, given) }
        }
    }
    collect(&f.body, &mut given);
    given.into_iter().filter(|(r, only)| *only && *r > f.params.len() as u32).map(|(r, _)| r).collect()
}

fn infer_types(f: &Function) -> (BTreeSet<u32>, BTreeSet<u32>) {
    let mut given: BTreeMap<u32, Vec<Expr>> = BTreeMap::new();
    fn collect(list: &[Stmt], given: &mut BTreeMap<u32, Vec<Expr>>) {
        for st in list {
            if let Stmt::Assign(Place::Local(r), v) = st { given.entry(*r).or_default().push(v.clone()) }
            for b in st.blocks() { collect(b, given) }
        }
    }
    collect(&f.body, &mut given);
    let mut ints: BTreeSet<u32> = BTreeSet::new();
    let mut bools: BTreeSet<u32> = BTreeSet::new();
    for (i, p) in f.params.iter().enumerate() {
        let r = i as u32 + 1;
        match p.type_name.as_deref() {
            Some("int") | Some("uint") if !given.contains_key(&r) => { ints.insert(r); }
            Some("Boolean") if !given.contains_key(&r) => { bools.insert(r); }
            _ => {}
        }
    }
    let param_type = |r: u32| f.params.get(r as usize - 1).and_then(|p| p.type_name.clone());
    // optimistic, then drop any register given something that is not
    let mut cand_i: BTreeSet<u32> = given.keys().copied().filter(|r| *r > f.params.len() as u32 || matches!(param_type(*r).as_deref(), Some("int") | Some("uint"))).collect();
    let mut cand_b: BTreeSet<u32> = given.keys().copied().filter(|r| *r > f.params.len() as u32 || param_type(*r).as_deref() == Some("Boolean")).collect();
    fn int_like(e: &Expr, ints: &BTreeSet<u32>) -> bool {
        match e {
            Expr::Int(i) => *i >= i32::MIN as i64 && *i <= i32::MAX as i64,
            Expr::Convert(Conv::Int, _) => true,
            Expr::Local(r) => ints.contains(r),
            Expr::Binary(op, a, b) => match op {
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => true,
                BinOp::Add | BinOp::Sub | BinOp::Mul => int_like(a, ints) && int_like(b, ints),
                _ => false,
            },
            Expr::Member(_, n) => n == "length",
            _ => false,
        }
    }
    fn bool_like(e: &Expr, bools: &BTreeSet<u32>) -> bool {
        match e {
            Expr::Bool(_) | Expr::Unary(UnOp::Not, _) | Expr::Convert(Conv::Boolean, _) => true,
            Expr::Binary(op, ..) => matches!(op, BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe
                | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In | BinOp::InstanceOf | BinOp::Is),
            Expr::And(a, b) | Expr::Or(a, b) => bool_like(a, bools) && bool_like(b, bools),
            Expr::Local(r) => bools.contains(r),
            _ => false,
        }
    }
    loop {
        let before = (cand_i.len(), cand_b.len());
        let snapshot_i = cand_i.clone();
        cand_i.retain(|r| given[r].iter().all(|v| int_like(v, &snapshot_i)));
        let snapshot_b = cand_b.clone();
        cand_b.retain(|r| given[r].iter().all(|v| bool_like(v, &snapshot_b)));
        if (cand_i.len(), cand_b.len()) == before { break }
    }
    ints.extend(cand_i);
    bools.extend(cand_b);
    (ints, bools)
}

/// An expression written as a statement. A statement that opens with a bracket reads as a call on
/// whatever came before it (a closing `}` included), so an expression bracketed whole is written
/// without its outer pair: a block stays a block, anything else is the same expression.
fn statement_text(e: String) -> String {
    let e = strip_enclosing(e);
    // still opening with a bracket (`(a ? b : c).x()`): a block of its own, so it is not read as
    // a call on whatever came before it
    if e.starts_with('(') { format!("{{ {e}; }}") } else { e }
}

fn strip_enclosing(e: String) -> String {
    let b = e.as_bytes();
    if b.first() != Some(&b'(') || b.last() != Some(&b')') { return e }
    // the first bracket must close at the very end, or the brackets do not enclose the whole
    let mut depth = 0i32;
    let mut in_str = false;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'"' if i == 0 || b[i - 1] != b'\\' => in_str = !in_str,
            b'(' if !in_str => depth += 1,
            b')' if !in_str => { depth -= 1; if depth == 0 && i != b.len() - 1 { return e } }
            _ => {}
        }
    }
    strip_enclosing(e[1..e.len() - 1].to_string())
}

/// a plain number a value states (a boolean as 1 or 0)
fn literal_num(e: &Expr) -> Option<f64> {
    match e {
        Expr::Num(v) => Some(*v),
        Expr::Int(v) => Some(*v as f64),
        Expr::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Expr::Unary(UnOp::Neg, x) => literal_num(x).map(|v| -v),
        Expr::Coerce(_, x) | Expr::Convert(_, x) => literal_num(x),
        _ => None,
    }
}

/// the event raised as an object touches the floor, by the source's constant or its value
fn is_ground_touch(e: &Expr) -> bool {
    matches!(e, Expr::Member(_, n) if n == "GROUND_TOUCH") || matches!(e, Expr::Str(s) if s == "groundTouch")
}

/// a field's container, holding its first value: a number's keeps a number, anything else's any value
fn cell_of(type_name: Option<&str>, value: &str) -> String {
    match type_name.map(super::short) {
        Some("Number" | "int" | "uint") => format!("PeptideRuntime.cellNum({value})"),
        _ => format!("PeptideRuntime.cell({value})"),
    }
}

/// the methods that change an array in place
const ARRAY_MUTATORS: &[&str] = &["push", "pop", "shift", "unshift", "splice", "sort", "sortOn", "reverse"];

/// where a read of a field writes back to
fn place_of(e: &Expr) -> Option<Place> {
    match e {
        Expr::Member(o, n) => Some(Place::Member((**o).clone(), n.clone())),
        Expr::Name(n) => Some(Place::Name(n.clone())),
        _ => None,
    }
}

/// a compiled function with its body inside a guard that reports what it throws
fn guarded(f: &str) -> String {
    match (f.find('{'), f.rfind('}')) {
        (Some(a), Some(b)) if a < b => format!("{}{{ try {{{}}} catch (__e:Dynamic) {{ PeptideRuntime.report(Std.string(__e)); }} }}",
            &f[..a], &f[a + 1..b]),
        _ => f.to_string(),
    }
}
