//! What a package declares about itself, read as the data it is.
//!
//! A package's main class registers what it contains (`register("characters", [this.getX()])`),
//! and each registration is a method that returns plain data: the fighter's stats (with the class
//! that runs it and the clip that is its art), its attacks, its projectiles, its items. Those are
//! object and array literals in the code, so they are evaluated here, not pattern-matched: the
//! same reading works for any package that declares itself the same way.

use std::collections::BTreeMap;

use super::ast::{Expr, Place, Stmt};
use super::{ClassModel, SourceModel};

/// A declared value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    /// a class, named by the declaration (`classAPI: PichuExt`)
    Class(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
    /// something a declaration computes rather than states
    Computed(String),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self { Value::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v), _ => None }
    }
    pub fn str(&self) -> Option<&str> { match self { Value::Str(s) => Some(s), _ => None } }
    pub fn class(&self) -> Option<&str> { match self { Value::Class(s) => Some(s), _ => None } }
    pub fn entries(&self) -> &[(String, Value)] { match self { Value::Obj(kv) => kv, _ => &[] } }
}

/// A fighter, as its package declares it.
#[derive(Clone, Debug)]
pub struct Fighter {
    /// the registration method (`getPichu`)
    pub getter: String,
    /// its stats: `cData`
    pub stats: Value,
    /// its attacks, by animation: `aData`
    pub attacks: Value,
    /// its projectiles, by name: `pData`
    pub projectiles: Value,
    /// its items: `iData`
    pub items: Value,
}

impl Fighter {
    /// the class that runs it
    pub fn class(&self) -> Option<&str> { self.stats.get("classAPI").and_then(|v| v.class()) }
    /// the clip that is its art
    pub fn clip(&self) -> Option<&str> { self.stats.get("linkage_id").and_then(|v| v.str()) }
}

/// Evaluate a declared expression. `env` holds the locals the method assigned before it returns.
pub fn eval(e: &Expr, env: &BTreeMap<u32, Value>) -> Value {
    match e {
        Expr::Null | Expr::Undefined => Value::Null,
        Expr::Bool(b) => Value::Bool(*b),
        Expr::Num(n) => Value::Num(*n),
        Expr::Int(i) => Value::Num(*i as f64),
        Expr::Str(s) => Value::Str(s.clone()),
        Expr::Unary(super::ast::UnOp::Neg, x) => match eval(x, env) { Value::Num(n) => Value::Num(-n), v => v },
        Expr::Name(n) => Value::Class(n.clone()),
        Expr::Local(r) => env.get(r).cloned().unwrap_or(Value::Null),
        Expr::Coerce(_, x) | Expr::Convert(_, x) => eval(x, env),
        Expr::Array(items) => Value::Arr(items.iter().map(|x| eval(x, env)).collect()),
        Expr::Object(kv) => Value::Obj(kv.iter().map(|(k, v)| {
            let key = match eval(k, env) { Value::Str(s) => s, other => format!("{other:?}") };
            (key, eval(v, env))
        }).collect()),
        Expr::New(c, args) if args.is_empty() && matches!(&**c, Expr::Name(n) if n == "Object") => Value::Obj(vec![]),
        Expr::New(c, args) if args.is_empty() && matches!(&**c, Expr::Name(n) if n == "Array") => Value::Arr(vec![]),
        other => Value::Computed(other.to_string()),
    }
}

/// What a method with no arguments returns, evaluated: its straight-line assignments to its own
/// registers, then its return.
pub fn returned(body: &[Stmt]) -> Option<Value> {
    let mut env: BTreeMap<u32, Value> = BTreeMap::new();
    for s in body {
        match s {
            Stmt::Assign(Place::Local(r), e) => { let v = eval(e, &env); env.insert(*r, v); }
            // a list built up a push at a time (`var a = new Array(); a.push({...}); return a;`)
            Stmt::Expr(Expr::Call(callee, args)) => {
                let Expr::Member(o, m) = &**callee else { continue };
                let (Expr::Local(r), "push") = (&**o, m.as_str()) else { continue };
                let items: Vec<Value> = args.iter().map(|a| eval(a, &env)).collect();
                if let Some(Value::Arr(list)) = env.get_mut(r) { list.extend(items); }
            }
            Stmt::Return(Some(e)) => return Some(eval(e, &env)),
            _ => {}
        }
    }
    None
}

impl SourceModel {
    /// The package's main class: the one bound to the root timeline's symbol 0.
    pub fn main_class(&self) -> Option<&ClassModel> { self.class_of(0) }

    /// What the main class registers under a kind (`"characters"`): the registration calls its
    /// constructor makes, each resolved to the method it names.
    pub fn registered(&self, kind: &str) -> Vec<(String, Value)> {
        let Some(main) = self.main_class() else { return vec![] };
        let mut out = Vec::new();
        for s in &main.constructor.body {
            let Stmt::Expr(Expr::Call(callee, args)) = s else { continue };
            let named = matches!(&**callee, Expr::Name(n) if n == "register")
                || matches!(&**callee, Expr::Member(o, n) if n == "register" && matches!(**o, Expr::This));
            if !named || args.first() != Some(&Expr::Str(kind.into())) { continue }
            let Some(Expr::Array(items)) = args.get(1) else { continue };
            for it in items {
                let Expr::Call(g, _) = it else { continue };
                let getter = match &**g {
                    Expr::Member(o, n) if matches!(**o, Expr::This) => n.clone(),
                    Expr::Name(n) => n.clone(),
                    _ => continue,
                };
                let Some(m) = main.methods.iter().find(|m| m.name == getter) else { continue };
                if let Some(v) = returned(&m.function.body) { out.push((getter, v)); }
            }
        }
        out
    }

    /// What the main class registers under a kind as a plain value (`register("stage", Cls)`,
    /// `register("id", "name")`), evaluated: the first registration of that kind.
    pub fn declared(&self, kind: &str) -> Option<Value> {
        let main = self.main_class()?;
        main.constructor.body.iter().find_map(|s| {
            let Stmt::Expr(Expr::Call(callee, args)) = s else { return None };
            let named = matches!(&**callee, Expr::Name(n) if n == "register")
                || matches!(&**callee, Expr::Member(o, n) if n == "register" && matches!(**o, Expr::This));
            if !named || args.first() != Some(&Expr::Str(kind.into())) { return None }
            Some(eval(args.get(1)?, &BTreeMap::new()))
        })
    }

    /// The stage the package declares: the class it registers as its stage, and the clip that is
    /// the whole stage (the package format links it `stage_<id>`, by the id the package
    /// registers), as (class, clip linkage, clip symbol).
    pub fn stage(&self) -> Option<(String, String, u16)> {
        let class = self.declared("stage")?.class()?.to_string();
        let class = self.classes.get(&class).map(|c| c.name.clone())
            .or_else(|| self.classes.values().find(|c| super::super::compile::short(&c.name) == class).map(|c| c.name.clone()))?;
        let clip = format!("stage_{}", self.declared("id")?.str()?);
        let sym = self.symbol_of(&clip)?;
        Some((class, clip, sym))
    }

    /// The fighters the package declares.
    pub fn fighters(&self) -> Vec<Fighter> {
        self.registered("characters").into_iter().map(|(getter, v)| Fighter {
            getter,
            stats: v.get("cData").cloned().unwrap_or(Value::Null),
            attacks: v.get("aData").cloned().unwrap_or(Value::Null),
            projectiles: v.get("pData").cloned().unwrap_or(Value::Null),
            items: v.get("iData").cloned().unwrap_or(Value::Null),
        }).collect()
    }

    /// The package's own classes that extend `root` somewhere up their chain (a stage's class,
    /// found by what it extends rather than by any name it carries).
    pub fn descendants(&self, root: &str) -> Vec<&ClassModel> {
        self.classes.values().filter(|c| {
            super::super::compile::short(&c.name) != root
                && self.supers(&c.name).iter().any(|s| super::super::compile::short(s) == root)
        }).collect()
    }

    /// Each of the package's own classes under one of `roots`, with its kind: the class directly
    /// under the root that it reaches (an enemy, a platform). The api layer itself (the roots and
    /// the kinds directly beneath them) describes the hierarchy and is not content.
    pub fn kinds_under(&self, roots: &[&str]) -> Vec<(String, String)> {
        let short = |n: &str| super::super::compile::short(n).to_string();
        let kinds: std::collections::BTreeSet<String> = self.classes.values()
            .filter(|c| roots.contains(&short(&c.super_name).as_str())).map(|c| short(&c.name)).collect();
        let mut out = Vec::new();
        for c in self.classes.values() {
            let name = short(&c.name);
            if roots.contains(&name.as_str()) || kinds.contains(&name) { continue }
            let chain: Vec<String> = std::iter::once(c.name.clone()).chain(self.supers(&c.name)).map(|n| short(&n)).collect();
            if let Some(i) = chain.iter().position(|n| roots.contains(&n.as_str())) {
                if i >= 1 { out.push((name, chain[i - 1].clone())); }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// The symbol a class is bound to.
    pub fn symbol_of(&self, class: &str) -> Option<u16> {
        self.symbols.iter().find(|(_, c)| c.as_str() == class).map(|(id, _)| *id)
    }

    /// A fighter clip's animations, as its timeline states them: each label, the frame it is on,
    /// and the clip placed there as `stance` (the animation's own clip, and its class).
    pub fn animations(&self, clip: &str) -> Vec<Animation> {
        let Some(sym) = self.symbol_of(clip) else { return vec![] };
        let Some(sprite) = self.sprites.get(&sym) else { return vec![] };
        sprite.labels.iter().map(|(frame, label)| {
            let stance = sprite.frames.get(*frame).and_then(|placed| placed.iter().find(|p| p.name.as_deref() == Some("stance")));
            Animation {
                label: label.clone(),
                frame: *frame,
                stance_symbol: stance.map(|p| p.char_id),
                stance_class: stance.and_then(|p| self.symbols.get(&p.char_id).cloned()),
            }
        }).collect()
    }
}

/// One animation of a fighter clip.
#[derive(Clone, Debug)]
pub struct Animation {
    pub label: String,
    /// the root timeline's frame the label is on (0-based)
    pub frame: usize,
    pub stance_symbol: Option<u16>,
    pub stance_class: Option<String>,
}
