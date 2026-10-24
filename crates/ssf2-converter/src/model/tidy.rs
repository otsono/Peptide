//! Structured statements to the code a person would have written.
//!
//! Every rewrite here keeps the meaning exactly; a shape that does not match stays as it is,
//! which is correct, only wordier. What it does:
//!
//! - a value decided by a branch (`if (c) j = a; else j = b;` then one read of `j`) becomes
//!   `c ? a : b` where it is read; `j = t; if (t) j = b;` becomes `t && b`
//! - a temp or join written once and read once, in the very next statement, before anything
//!   there that has an effect, is put where it is read
//! - `while (true) { if (!c) break; ... }` becomes `while (c) { ... }`, and the do-while shape
//!   becomes `do { ... } while (c)`
//! - a catch clause's plumbing (its scope object and its slot) reads as the exception itself

use std::collections::BTreeMap;

use super::ast::*;

pub fn tidy(mut body: Vec<Stmt>) -> Vec<Stmt> {
    body = map_stmts(body, &mut |e| match e {
        // a catch's exception, kept in its scope object's first slot: the exception itself
        Expr::Member(o, n) if matches!(*o, Expr::Caught(_)) && n == "#slot1" => *o,
        e => e,
    });
    body = drop_catch_plumbing(body);
    for _ in 0..64 {
        let before = body.clone();
        // a lookup through the scope chain of the very name it looks up is that name
        body = map_stmts(body, &mut |e| match e {
            Expr::Member(o, n) if matches!(&*o, Expr::ScopeOf(m) if *m == n) => Expr::Name(n),
            e => e,
        });
        let counts = Counts::of(&body);
        body = fold(body, &counts);
        body = loops(body);
        body = conditions(body);
        body = switches(body);
        if body == before { break }
    }
    body
}

// ---- counting reads and writes of the decompiler's own variables --------------------------------

#[derive(Default)]
struct Counts {
    reads: BTreeMap<(usize, u32), usize>,
    writes: BTreeMap<(usize, u32), usize>,
}

fn is_var(e: &Expr) -> bool { key(e).is_some() }

/// a temp or a join, as a key: temps sit apart from every block
fn key(e: &Expr) -> Option<(usize, u32)> {
    match e {
        Expr::Temp(t) => Some((usize::MAX, *t)),
        Expr::Join { block, slot } => Some((*block, *slot)),
        _ => None,
    }
}

impl Counts {
    fn of(body: &[Stmt]) -> Counts {
        let mut c = Counts::default();
        c.stmts(body);
        c
    }
    fn stmts(&mut self, body: &[Stmt]) {
        for s in body {
            if let Stmt::Assign(p, _) = s {
                let v = p.as_expr();
                if let Some(k) = key(&v) { *self.writes.entry(k).or_default() += 1 }
            }
            for e in s.exprs() {
                e.walk(&mut |x| {
                    if let Some(k) = key(x) { *self.reads.entry(k).or_default() += 1 }
                    if let Expr::Assign(p, _) = x {
                        let v = p.as_expr();
                        if let Some(k) = key(&v) { *self.writes.entry(k).or_default() += 1 }
                    }
                });
            }
            for b in s.blocks() { self.stmts(b) }
        }
    }
    fn reads(&self, v: &Expr) -> usize { key(v).and_then(|k| self.reads.get(&k).copied()).unwrap_or(0) }
    fn writes(&self, v: &Expr) -> usize { key(v).and_then(|k| self.writes.get(&k).copied()).unwrap_or(0) }
}

// ---- walking statements ---------------------------------------------------------------------------

fn map_stmts(body: Vec<Stmt>, f: &mut dyn FnMut(Expr) -> Expr) -> Vec<Stmt> {
    let mut g = |e: Expr| f(e);
    body.into_iter().map(|s| map_stmt(s, &mut g)).collect()
}


fn map_stmt(s: Stmt, f: &mut impl FnMut(Expr) -> Expr) -> Stmt {
    let ms = |b: Vec<Stmt>, f: &mut dyn FnMut(Expr) -> Expr| map_stmts(b, f);
    match s {
        Stmt::Expr(e) => Stmt::Expr(e.map(f)),
        Stmt::Assign(p, e) => Stmt::Assign(p.map(f), e.map(f)),
        Stmt::If(c, a, b) => { let c = c.map(f); Stmt::If(c, ms(a, f), ms(b, f)) }
        Stmt::Loop(b) => Stmt::Loop(ms(b, f)),
        Stmt::While(c, b) => { let c = c.map(f); Stmt::While(c, ms(b, f)) }
        Stmt::DoWhile(b, c) => { let b = ms(b, f); Stmt::DoWhile(b, c.map(f)) }
        Stmt::Return(e) => Stmt::Return(e.map(|e| e.map(f))),
        Stmt::Throw(e) => Stmt::Throw(e.map(f)),
        Stmt::Switch(i, cases, d) => {
            let i = i.map(f);
            let cases = cases.into_iter().map(|(k, b)| (k, ms(b, f))).collect();
            Stmt::Switch(i, cases, ms(d, f))
        }
        Stmt::Try(b, cs) => {
            let b = ms(b, f);
            Stmt::Try(b, cs.into_iter().map(|c| Catch { body: ms(c.body, f), ..c }).collect())
        }
        s => s,
    }
}

fn drop_catch_plumbing(body: Vec<Stmt>) -> Vec<Stmt> {
    body.into_iter().filter_map(|s| match s {
        Stmt::Assign(Place::Member(Expr::Caught(_), _), Expr::Caught(_)) => None,
        s => Some(recurse(s, &mut drop_catch_plumbing)),
    }).collect()
}

fn recurse(s: Stmt, f: &mut dyn FnMut(Vec<Stmt>) -> Vec<Stmt>) -> Stmt {
    match s {
        Stmt::If(c, a, b) => Stmt::If(c, f(a), f(b)),
        Stmt::Loop(b) => Stmt::Loop(f(b)),
        Stmt::While(c, b) => Stmt::While(c, f(b)),
        Stmt::DoWhile(b, c) => Stmt::DoWhile(f(b), c),
        Stmt::Switch(i, cases, d) => Stmt::Switch(i, cases.into_iter().map(|(k, b)| (k, f(b))).collect(), f(d)),
        Stmt::Try(b, cs) => Stmt::Try(f(b), cs.into_iter().map(|c| Catch { body: f(c.body), ..c }).collect()),
        s => s,
    }
}

// ---- folding ----------------------------------------------------------------------------------------

/// Whether, evaluating `e`, anything with an effect finishes before `target` is read; `None` when
/// `e` does not read `target`.
fn effect_before(e: &Expr, target: &Expr) -> Option<bool> {
    fn go(e: &Expr, target: &Expr, seen: &mut bool) -> bool {
        if e == target { return true }
        for c in e.children() {
            if go(c, target, seen) { return true }
        }
        if matches!(e, Expr::Call(..) | Expr::New(..) | Expr::SuperCall(..) | Expr::Assign(..)
            | Expr::Delete(..) | Expr::HasNext2 { .. }) { *seen = true }
        false
    }
    let mut seen = false;
    go(e, target, &mut seen).then_some(seen)
}

/// the same for a statement, in its evaluation order; `None` when it does not read `target` in
/// its own expressions, or when it evaluates them more than once (a loop condition)
fn stmt_effect_before(s: &Stmt, target: &Expr) -> Option<bool> {
    let exprs: Vec<&Expr> = match s {
        Stmt::Expr(_) | Stmt::Assign(..) | Stmt::If(..) | Stmt::Switch(..) | Stmt::Return(Some(_)) | Stmt::Throw(_) => s.exprs(),
        _ => return None,
    };
    let mut before = false;
    for e in exprs {
        match effect_before(e, target) {
            Some(b) => return Some(before || b),
            None => if e.has_effect() { before = true },
        }
    }
    None
}

fn replace(s: Stmt, target: &Expr, with: &Expr) -> Stmt {
    let mut done = false;
    let mut f = |e: Expr| if !done && e == *target { done = true; with.clone() } else { e };
    match s {
        // only this statement's own expressions: never inside the lists it holds
        Stmt::Expr(e) => Stmt::Expr(e.map(&mut f)),
        Stmt::Assign(p, e) => { let p = p.map(&mut f); Stmt::Assign(p, e.map(&mut f)) }
        Stmt::If(c, a, b) => Stmt::If(c.map(&mut f), a, b),
        Stmt::Switch(i, c, d) => Stmt::Switch(i.map(&mut f), c, d),
        Stmt::Return(Some(e)) => Stmt::Return(Some(e.map(&mut f))),
        Stmt::Throw(e) => Stmt::Throw(e.map(&mut f)),
        s => s,
    }
}

/// the decompiler's own variables and the registers an expression reads
fn var_deps(e: &Expr) -> Vec<Expr> {
    let mut out = Vec::new();
    e.walk(&mut |x| if matches!(x, Expr::Local(_) | Expr::Temp(_) | Expr::Join { .. } | Expr::This) { out.push(x.clone()) });
    out
}

/// whether reading an expression looks at anything a call or a store could change
fn reads_memory(e: &Expr) -> bool {
    let mut any = false;
    e.walk(&mut |x| if matches!(x, Expr::Member(..) | Expr::Index(..) | Expr::Name(_) | Expr::SuperMember(_)
        | Expr::Call(..) | Expr::New(..) | Expr::SuperCall(..) | Expr::HasNext(..) | Expr::HasNext2 { .. }
        | Expr::NextName(..) | Expr::NextValue(..)) { any = true });
    any
}

/// the variables and registers a statement writes, anywhere in it
fn writes_vars(s: &Stmt, out: &mut Vec<Expr>) {
    let note = |p: &Place, out: &mut Vec<Expr>| {
        if matches!(p, Place::Local(_) | Place::Temp(_) | Place::Join { .. }) { out.push(p.as_expr()) }
    };
    if let Stmt::Assign(p, _) = s { note(p, out) }
    for e in s.exprs() {
        e.walk(&mut |x| match x {
            Expr::Assign(p, _) => if matches!(**p, Place::Local(_) | Place::Temp(_) | Place::Join { .. }) { out.push(p.as_expr()) },
            Expr::HasNext2 { object, index } => { out.push(Expr::Local(*object)); out.push(Expr::Local(*index)) }
            _ => {}
        });
    }
    for b in s.blocks() { for t in b { writes_vars(t, out) } }
}

/// whether a statement can change what a memory read sees: a store to a property or a name, or
/// anything with an effect
fn may_write_memory(s: &Stmt) -> bool {
    if let Stmt::Assign(p, _) = s {
        if !matches!(p, Place::Local(_) | Place::Temp(_) | Place::Join { .. }) { return true }
    }
    if s.exprs().iter().any(|e| e.has_effect()) { return true }
    s.blocks().iter().any(|b| b.iter().any(may_write_memory))
}

fn reads_deep(s: &Stmt, var: &Expr) -> usize {
    let mut n = 0;
    for e in s.exprs() { n += e.mentions(var) }
    for b in s.blocks() { for t in b { n += reads_deep(t, var) } }
    n
}

fn subst_deep(s: Stmt, var: &Expr, with: &Expr) -> Stmt {
    let mut f = |e: Expr| if e == *var { with.clone() } else { e };
    map_stmt(s, &mut f)
}

/// whether `e`, computed before `between`, can be computed after them instead
fn can_move_past(e: &Expr, between: &[Stmt]) -> bool {
    let deps = var_deps(e);
    let mem = reads_memory(e);
    between.iter().all(|s| {
        let mut w = Vec::new();
        writes_vars(s, &mut w);
        let conflict = w.iter().any(|x| deps.contains(x)) || (mem && may_write_memory(s))
            || (e.has_effect() && (may_write_memory(s) || s.exprs().iter().any(|x| reads_memory(x))));
        !conflict
    })
}

fn fold(body: Vec<Stmt>, n: &Counts) -> Vec<Stmt> {
    let mut v: Vec<Stmt> = body.into_iter().map(|s| recurse(s, &mut |b| fold(b, n))).collect();
    let mut k = 0;
    while k < v.len() {
        // a value no one reads
        if let Stmt::Assign(p, e) = &v[k] {
            let var = p.as_expr();
            if is_var(&var) && n.reads(&var) == 0 {
                if e.has_effect() { v[k] = Stmt::Expr(e.clone()) } else { v.remove(k); }
                continue;
            }
        }
        if let Stmt::If(..) = &v[k] {
            // a way that sets a variable to what the assignments just before the test already set
            // it to (a value passed through a branch that does not change it) sets nothing
            let mut pre: Vec<(Place, Expr)> = Vec::new();
            for s in v[..k].iter().rev() {
                match s {
                    Stmt::Assign(p, e) if is_var(&p.as_expr()) => pre.push((p.clone(), e.clone())),
                    _ => break,
                }
            }
            let known = |p: &Place, e: &Expr| {
                let copy = matches!(e, Expr::Local(_) | Expr::Temp(_) | Expr::Join { .. } | Expr::This | Expr::Null
                    | Expr::Undefined | Expr::Bool(_) | Expr::Num(_) | Expr::Int(_) | Expr::Str(_)
                    | Expr::ScopeOf(_) | Expr::Global | Expr::Activation | Expr::Caught(_));
                copy && pre.iter().find(|(q, _)| q == p).is_some_and(|(_, x)| x == e)
            };
            let Stmt::If(c, a, b) = &v[k] else { unreachable!() };
            let strip = |l: &[Stmt]| -> usize {
                l.iter().take_while(|s| matches!(s, Stmt::Assign(p, e) if known(p, e))).count()
            };
            let (sa, sb) = (strip(a), strip(b));
            if (sa > 0 || sb > 0) && (!c.has_effect() || pre.iter().all(|(_, x)| !reads_memory(x))) {
                let (c, a, b) = (c.clone(), a[sa..].to_vec(), b[sb..].to_vec());
                v[k] = Stmt::If(c, a, b);
                continue;
            }
        }
        if let Stmt::If(c, a, b) = &v[k] {
            // both ways set the same variables: `v = c ? a : b`, one variable at a time
            // (a test with an effect is fine for one variable: it is still evaluated once)
            let pairs: Option<Vec<(Place, Expr, Expr)>> = (a.len() == b.len() && !a.is_empty() && (a.len() == 1 || !c.has_effect())).then(|| {
                a.iter().zip(b.iter()).map(|(x, y)| match (x, y) {
                    (Stmt::Assign(p, ex), Stmt::Assign(q, ey)) if p == q && is_var(&p.as_expr()) => Some((p.clone(), ex.clone(), ey.clone())),
                    _ => None,
                }).collect::<Option<Vec<_>>>()
            }).flatten();
            if let Some(pairs) = pairs {
                let set: Vec<Expr> = pairs.iter().map(|(p, _, _)| p.as_expr()).collect();
                let clean = pairs.iter().all(|(_, x, y)| !set.iter().any(|s| x.mentions(s) + y.mentions(s) > 0))
                    && !set.iter().any(|s| c.mentions(s) > 0);
                if clean {
                    let c = c.clone();
                    let new: Vec<Stmt> = pairs.into_iter().map(|(p, x, y)| {
                        let e = if x == y { x } else { Expr::Cond(Box::new(c.clone()), Box::new(x), Box::new(y)) };
                        Stmt::Assign(p, e)
                    }).collect();
                    v.splice(k..=k, new);
                    continue;
                }
            }
            // the same first assignment both ways, of a value with no effect, before the test
            if let (Some(Stmt::Assign(p, x)), Some(Stmt::Assign(q, y))) = (a.first(), b.first()) {
                // (nothing a test does can change a value that reads no memory)
                if p == q && x == y && is_var(&p.as_expr()) && !x.has_effect() && !reads_memory(x)
                    && c.mentions(&p.as_expr()) == 0
                {
                    let (p, x) = (p.clone(), x.clone());
                    let (c, mut a, mut b) = match v.remove(k) { Stmt::If(c, a, b) => (c, a, b), _ => unreachable!() };
                    a.remove(0);
                    b.remove(0);
                    v.insert(k, Stmt::If(c, a, b));
                    v.insert(k, Stmt::Assign(p, x));
                    continue;
                }
            }
        }
        // t = e; r = t; ...t...  is  r = e; ...r...  while nothing writes r (a value stored and
        // kept on the stack: `r = new X()` then its uses)
        if k + 1 < v.len() {
            let pair = match (&v[k], &v[k + 1]) {
                (Stmt::Assign(Place::Temp(t), e), Stmt::Assign(Place::Local(r), Expr::Temp(t2))) if t == t2 => Some((*t, *r, e.clone())),
                _ => None,
            };
            if let Some((t, r, e)) = pair {
                let r = &r;
                let temp = Expr::Temp(t);
                if n.writes(&temp) == 1 {
                    let total = n.reads(&temp);
                    let mut seen = 1;
                    let mut last = Some(k + 1);
                    if seen < total {
                        last = None;
                        for (j, s) in v.iter().enumerate().skip(k + 2) {
                            seen += reads_deep(s, &temp);
                            if seen >= total { last = Some(j); break }
                        }
                    }
                    let local = Expr::Local(*r);
                    let clear = last.is_some_and(|m| v[k + 2..=m].iter().enumerate().all(|(i, s)| {
                        let mut w = Vec::new();
                        writes_vars(s, &mut w);
                        // the last read may be in the statement that stores to the register again
                        !w.contains(&local) || (k + 2 + i == m && matches!(s, Stmt::Assign(Place::Local(q), _) if q == r)
                            && { let mut w2 = Vec::new(); if let Stmt::Assign(_, rhs) = s { rhs.walk(&mut |x| if let Expr::Assign(p, _) = x { w2.push(p.as_expr()) }); } !w2.contains(&local) })
                    }));
                    if clear {
                        let m = last.unwrap();
                        for s in v.iter_mut().take(m + 1).skip(k + 2) {
                            let x = std::mem::replace(s, Stmt::Break);
                            *s = subst_deep(x, &temp, &local);
                        }
                        v[k] = Stmt::Assign(Place::Local(*r), e);
                        v.remove(k + 1);
                        continue;
                    }
                }
            }
        }
        // v = x; if (x) v = y;  is  v = x && y   (and `if (!x)` is `||`)
        if k + 1 < v.len() {
            if let (Stmt::Assign(p, x), Stmt::If(c, a, b)) = (&v[k], &v[k + 1]) {
                let var = p.as_expr();
                if is_var(&var) && !x.has_effect() && b.is_empty() && x.mentions(&var) == 0 {
                    if let [Stmt::Assign(q, y)] = a.as_slice() {
                        let joined = if q != p { None }
                            else if c == x { Some(Expr::And(Box::new(x.clone()), Box::new(y.clone()))) }
                            else if *c == x.clone().negated() { Some(Expr::Or(Box::new(x.clone()), Box::new(y.clone()))) }
                            else { None };
                        if let Some(e) = joined {
                            v[k] = Stmt::Assign(p.clone(), e);
                            v.remove(k + 1);
                            continue;
                        }
                    }
                }
            }
        }
        // written once: put it where it is read
        if let Stmt::Assign(p, e) = &v[k] {
            let var = p.as_expr();
            let total = n.reads(&var);
            if is_var(&var) && n.writes(&var) == 1 && total >= 1 && e.mentions(&var) == 0 {
                // the statements its reads are in
                let mut seen = 0;
                let mut last = None;
                for (j, s) in v.iter().enumerate().skip(k + 1) {
                    seen += reads_deep(s, &var);
                    if seen >= total { last = Some(j); break }
                }
                if let Some(m) = last {
                    let copyable = matches!(e, Expr::Local(_) | Expr::Temp(_) | Expr::Join { .. } | Expr::This
                        | Expr::Null | Expr::Undefined | Expr::Bool(_) | Expr::Num(_) | Expr::Int(_) | Expr::Str(_)
                        | Expr::ScopeOf(_) | Expr::Global | Expr::Activation | Expr::Caught(_));
                    let ok = if copyable {
                        // a copy: good wherever it is read, while nothing it reads changes
                        // (the statement it is read in may store to what it reads: an
                        // assignment reads its value before it stores)
                        let deps = var_deps(e);
                        let clear = |s: &Stmt| { let mut w = Vec::new(); writes_vars(s, &mut w); !w.iter().any(|x| deps.contains(x)) };
                        let last_ok = match &v[m] {
                            Stmt::Assign(_, rhs) => {
                                let mut w = Vec::new();
                                rhs.walk(&mut |x| if let Expr::Assign(p, _) = x { w.push(p.as_expr()) });
                                !w.iter().any(|x| deps.contains(x))
                            }
                            s => clear(s),
                        };
                        v[k + 1..m].iter().all(clear) && last_ok
                    } else if total == 1 {
                        // a computation: only where it is read once, in that statement's own
                        // expressions, with nothing in between or before it there that it could see
                        can_move_past(e, &v[k + 1..m]) && match stmt_effect_before(&v[m], &var) {
                            Some(false) => true,
                            Some(true) => !reads_memory(e) && !e.has_effect(),
                            None => false,
                        }
                    } else { false };
                    if ok {
                        let e = e.clone();
                        for s in v.iter_mut().take(m + 1).skip(k + 1) {
                            let t = std::mem::replace(s, Stmt::Break);
                            *s = if copyable { subst_deep(t, &var, &e) } else { replace(t, &var, &e) };
                        }
                        v.remove(k);
                        continue;
                    }
                }
            }
        }
        // a statement that is only a value with no effect
        if let Stmt::Expr(e) = &v[k] {
            if !e.has_effect() { v.remove(k); continue }
        }
        k += 1;
    }
    v
}

/// a test reads only whether its value is truthy: a conversion to Boolean inside it says nothing,
/// and a negated equality is the other equality
fn condition(e: Expr) -> Expr {
    match e {
        Expr::Convert(Conv::Boolean, x) => condition(*x),
        Expr::And(a, b) => Expr::And(Box::new(condition(*a)), Box::new(condition(*b))),
        Expr::Or(a, b) => Expr::Or(Box::new(condition(*a)), Box::new(condition(*b))),
        Expr::Unary(UnOp::Not, x) => match condition(*x) {
            Expr::Binary(op @ (BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe), a, b) => Expr::Binary(match op {
                BinOp::Eq => BinOp::Ne, BinOp::Ne => BinOp::Eq, BinOp::StrictEq => BinOp::StrictNe, _ => BinOp::StrictEq,
            }, a, b),
            x => x.negated(),
        },
        e => e,
    }
}

fn conditions(body: Vec<Stmt>) -> Vec<Stmt> {
    body.into_iter().map(|s| match recurse(s, &mut conditions) {
        Stmt::If(c, a, b) => Stmt::If(condition(c), a, b),
        Stmt::While(c, b) => Stmt::While(condition(c), b),
        Stmt::DoWhile(b, c) => Stmt::DoWhile(b, condition(c)),
        s => s,
    }).collect()
}

// ---- loops ---------------------------------------------------------------------------------------------

/// whether a list jumps to its own loop's start anywhere (not counting loops nested in it)
fn continues(body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::Continue => true,
        Stmt::Loop(_) | Stmt::While(..) | Stmt::DoWhile(..) => false,
        s => s.blocks().iter().any(|b| continues(b)),
    })
}

/// the jump back at the end of a loop's body goes back anyway
fn strip_trailing_continue(mut body: Vec<Stmt>) -> Vec<Stmt> {
    match body.pop() {
        Some(Stmt::Continue) => {}
        Some(Stmt::If(c, a, b)) => body.push(Stmt::If(c, strip_trailing_continue(a), strip_trailing_continue(b))),
        Some(s) => body.push(s),
        None => {}
    }
    body
}

fn loops(body: Vec<Stmt>) -> Vec<Stmt> {
    body.into_iter().map(|s| {
        let s = recurse(s, &mut loops);
        match s {
            Stmt::Loop(b) => loop_form(strip_trailing_continue(b)),
            Stmt::If(c, a, b) if a.is_empty() && !b.is_empty() => Stmt::If(c.negated(), b, a),
            s => s,
        }
    }).collect()
}

fn loop_form(mut b: Vec<Stmt>) -> Stmt {
    // while (c) { ... }
    if b.len() == 1 {
        if let Stmt::If(c, t, e) = &b[0] {
            if e.as_slice() == [Stmt::Break] { return Stmt::While(c.clone(), strip_trailing_continue(t.clone())) }
            if t.as_slice() == [Stmt::Break] { return Stmt::While(c.clone().negated(), strip_trailing_continue(e.clone())) }
        }
    }
    // do { ... } while (c)
    if let Some(Stmt::If(c, t, e)) = b.last().cloned() {
        let head = &b[..b.len() - 1];
        if !continues(head) {
            let cont = |l: &[Stmt]| l.is_empty() || l == [Stmt::Continue];
            if cont(&t) && e.as_slice() == [Stmt::Break] { b.pop(); return Stmt::DoWhile(b, c) }
            if t.as_slice() == [Stmt::Break] && cont(&e) { b.pop(); return Stmt::DoWhile(b, c.negated()) }
        }
    }
    Stmt::Loop(b)
}

// ---- switches -------------------------------------------------------------------------------------

/// `switch (v) { case A: ... }` compiles to a chain of strict comparisons that works out which case
/// matched (`A !== v ? (B !== v ? 2 : 1) : 0`) and a jump on that number. Read back as the source's
/// own switch on `v`, each numbered case under the value it stands for.
fn switches(body: Vec<Stmt>) -> Vec<Stmt> {
    body.into_iter().map(|s| match recurse(s, &mut switches) {
        Stmt::Switch(ix, arms, default) => match case_chain(&ix) {
            Some((subject, values, fallback)) => {
                let arms = arms.into_iter().filter_map(|(ks, b)| {
                    let keys: Vec<Expr> = ks.iter().filter_map(|k| match k {
                        Expr::Int(i) if (*i as usize) < values.len() && *i != fallback => Some(values[*i as usize].clone()),
                        _ => None,
                    }).collect();
                    (!keys.is_empty()).then_some((keys, b))
                }).collect();
                Stmt::Switch(subject, arms, default)
            }
            None => Stmt::Switch(ix, arms, default),
        },
        s => s,
    }).collect()
}

/// `K0 !== v ? (K1 !== v ? ... : 1) : 0` ending in the number no case matched: the subject `v`,
/// each case's value by its number, and that last number
fn case_chain(e: &Expr) -> Option<(Expr, Vec<Expr>, i64)> {
    let mut values: Vec<Expr> = Vec::new();
    let mut subject: Option<Expr> = None;
    let mut cur = e;
    loop {
        match cur {
            Expr::Int(d) => {
                if values.is_empty() || *d != values.len() as i64 { return None }
                return Some((subject?, values, *d));
            }
            Expr::Cond(c, a, b) => {
                let (k, v, hit_first) = match &**c {
                    Expr::Binary(BinOp::StrictNe, k, v) => (k, v, false),
                    Expr::Binary(BinOp::StrictEq, k, v) => (k, v, true),
                    _ => return None,
                };
                let (hit, rest) = if hit_first { (a, b) } else { (b, a) };
                if **hit != Expr::Int(values.len() as i64) { return None }
                match &subject {
                    None => subject = Some((**v).clone()),
                    Some(s) if s == &**v => {}
                    _ => return None,
                }
                if k.has_effect() { return None }
                values.push((**k).clone());
                cur = rest;
            }
            _ => return None,
        }
    }
}
