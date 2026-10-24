//! The source's API names, renamed to the runtime's as a package is read.
//!
//! Content calls its engine by SSF2's names and bundles SSF2's SDK classes. The runtime answers to
//! its own names instead, listed against SSF2's in `runtime/api_names.jsonc`. This pass rewrites a
//! whole model through that table before anything else reads it: class references, `extends`,
//! declared types, member accesses, member definitions and overrides. The rename is uniform, so a
//! content class that overrides an API member is renamed with it and every call still reaches the
//! same method.

use std::collections::{BTreeMap, BTreeSet};

use super::ast::{Catch, Expr, Function, Place, Stmt};
use super::SourceModel;

/// SSF2's name -> the runtime's, for classes and for members.
#[derive(Default, Clone, Debug)]
pub struct Names {
    pub classes: BTreeMap<String, String>,
    pub members: BTreeMap<String, String>,
}

impl Names {
    /// Read the table (JSONC: `{ "classes": {..}, "members": {..} }`).
    pub fn parse(text: &str) -> Names {
        let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(text)).unwrap_or_default();
        let map = |k: &str| -> BTreeMap<String, String> {
            json.get(k).and_then(|v| v.as_object()).map(|o| o.iter()
                .filter_map(|(a, b)| Some((a.clone(), b.as_str()?.to_string()))).collect()).unwrap_or_default()
        };
        Names { classes: map("classes"), members: map("members") }
    }

    fn class(&self, n: &str) -> Option<&String> { self.classes.get(n) }
    fn member(&self, n: &str) -> Option<&String> { self.members.get(n) }
    /// a name the scope chain resolves: a class, else a member of the object
    fn any(&self, n: &str) -> Option<&String> { self.class(n).or_else(|| self.member(n)) }
}

/// Rename a model through the table. Returns the names content already uses for something else
/// that the table would rename a different name to: renaming would merge the two, so each is
/// reported for the table to be changed.
pub fn rename(model: &mut SourceModel, names: &Names) -> Vec<String> {
    let mut problems = Vec::new();
    // what content already calls things, before the rename
    let mut used: BTreeSet<String> = BTreeSet::new();
    for c in model.classes.values() {
        used.insert(super::super::compile::short(&c.name).to_string());
        used.extend(c.fields.iter().map(|f| f.name.clone()));
        used.extend(c.methods.iter().map(|m| m.name.clone()));
    }
    for (from, to) in names.members.iter().chain(names.classes.iter()) {
        if used.contains(to) && !used.contains(from) && !names.members.contains_key(to) && !names.classes.contains_key(to) {
            problems.push(format!("the package already uses `{to}`, which the api name table gives `{from}`"));
        }
    }

    let classes = std::mem::take(&mut model.classes);
    for (_, mut c) in classes {
        if let Some(to) = names.class(&c.name) { c.name = to.clone(); }
        if let Some(to) = names.class(&c.super_name) { c.super_name = to.clone(); }
        for f in &mut c.fields {
            if let Some(to) = names.member(&f.name) { f.name = to.clone(); }
            ty(&mut f.type_name, names);
            if let Some(e) = &mut f.default { expr(e, names); }
        }
        for m in &mut c.methods {
            if let Some(to) = names.member(&m.name) { m.name = to.clone(); }
            function(&mut m.function, names);
        }
        function(&mut c.constructor, names);
        function(&mut c.static_init, names);
        for fs in &mut c.frame_scripts {
            if let Some(to) = names.member(&fs.method) { fs.method = to.clone(); }
        }
        model.classes.insert(c.name.clone(), c);
    }
    for v in model.symbols.values_mut() {
        if let Some(to) = names.class(v) { *v = to.clone(); }
    }
    for f in &mut model.scripts { function(f, names); }
    problems
}

fn ty(t: &mut Option<String>, names: &Names) {
    if let Some(n) = t { if let Some(to) = names.class(n) { *n = to.clone(); } }
}

fn function(f: &mut Function, names: &Names) {
    for p in &mut f.params {
        ty(&mut p.type_name, names);
        if let Some(d) = &mut p.default { expr(d, names); }
    }
    ty(&mut f.return_type, names);
    stmts(&mut f.body, names);
}

fn stmts(list: &mut [Stmt], names: &Names) {
    for s in list { stmt(s, names); }
}

fn stmt(s: &mut Stmt, names: &Names) {
    match s {
        Stmt::Expr(e) | Stmt::Throw(e) => expr(e, names),
        Stmt::Assign(p, e) => { place(p, names); expr(e, names); }
        Stmt::If(c, a, b) => { expr(c, names); stmts(a, names); stmts(b, names); }
        Stmt::Loop(b) => stmts(b, names),
        Stmt::While(c, b) => { expr(c, names); stmts(b, names); }
        Stmt::DoWhile(b, c) => { stmts(b, names); expr(c, names); }
        Stmt::Return(Some(e)) => expr(e, names),
        Stmt::Switch(v, cases, d) => {
            expr(v, names);
            for (vals, body) in cases { for x in vals.iter_mut() { expr(x, names); } stmts(body, names); }
            stmts(d, names);
        }
        Stmt::Try(body, catches) => {
            stmts(body, names);
            for Catch { type_name, body, .. } in catches { ty(type_name, names); stmts(body, names); }
        }
        Stmt::Return(None) | Stmt::Break | Stmt::Continue | Stmt::Unreadable(_) => {}
    }
}

fn place(p: &mut Place, names: &Names) {
    match p {
        Place::Name(n) => { if let Some(to) = names.any(n) { *n = to.clone(); } }
        Place::Member(o, n) => { expr(o, names); if let Some(to) = names.member(n) { *n = to.clone(); } }
        Place::Index(o, i) => { expr(o, names); expr(i, names); }
        Place::SuperMember(n) => { if let Some(to) = names.member(n) { *n = to.clone(); } }
        Place::Local(_) | Place::Join { .. } | Place::Temp(_) => {}
    }
}

fn expr(e: &mut Expr, names: &Names) {
    match e {
        Expr::Name(n) | Expr::ScopeOf(n) => { if let Some(to) = names.any(n) { *n = to.clone(); } }
        Expr::Member(o, n) => { expr(o, names); if let Some(to) = names.member(n) { *n = to.clone(); } }
        Expr::SuperMember(n) => { if let Some(to) = names.member(n) { *n = to.clone(); } }
        Expr::SuperCall(n, args) => {
            if let Some(to) = names.member(n) { *n = to.clone(); }
            for a in args { expr(a, names); }
        }
        Expr::Coerce(t, x) => { if let Some(to) = names.class(t) { *t = to.clone(); } expr(x, names); }
        Expr::Index(a, b) | Expr::Binary(_, a, b) | Expr::And(a, b) | Expr::Or(a, b)
        | Expr::NextName(a, b) | Expr::NextValue(a, b) | Expr::HasNext(a, b) => { expr(a, names); expr(b, names); }
        Expr::Call(c, args) | Expr::New(c, args) | Expr::TypeApply(c, args) => {
            expr(c, names);
            for a in args { expr(a, names); }
        }
        Expr::Unary(_, x) | Expr::Convert(_, x) => expr(x, names),
        Expr::Cond(a, b, c) => { expr(a, names); expr(b, names); expr(c, names); }
        Expr::Array(items) => for x in items { expr(x, names); },
        Expr::Object(kv) => for (_, v) in kv { expr(v, names); },
        Expr::Function(f) => function(f, names),
        Expr::Assign(p, v) => { place(p, names); expr(v, names); }
        Expr::Delete(p) => place(p, names),
        Expr::Undefined | Expr::Null | Expr::Bool(_) | Expr::Num(_) | Expr::Int(_) | Expr::Str(_)
        | Expr::This | Expr::Local(_) | Expr::Join { .. } | Expr::Temp(_) | Expr::HasNext2 { .. }
        | Expr::Global | Expr::Activation | Expr::Caught(_) | Expr::Class(_) | Expr::Unreadable(_) => {}
    }
}
