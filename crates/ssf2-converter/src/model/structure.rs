//! Blocks to structured statements.
//!
//! The code comes from a compiler that only emits structured control flow, so its graph is
//! reducible: every loop has one header, found by a back edge to a block that dominates it, and
//! every conditional rejoins at the block that post-dominates it. The structurer follows those:
//!
//! - a condition built of several tests (`a && b`, `a || !b`) is one test per block, all
//!   branching to the same places; those blocks are merged back into one condition first
//! - a loop is `while (true)` with its own `break` and `continue`; tidying gives it its real form
//! - a conditional's two ways run until they rejoin
//! - a jump that is neither (a `continue` that runs a `for`'s step first) repeats the few
//!   statements it jumps to, which keeps the meaning without a `goto` the target cannot express

use std::collections::{BTreeMap, BTreeSet};

use super::ast::*;
use super::decode::{Block, Decoded, Handler, Term};

const EXIT: usize = usize::MAX;
const ROOT: usize = usize::MAX - 1;

pub fn structure(dec: Decoded) -> Vec<Stmt> {
    let mut g = Graph::new(dec.blocks, dec.handlers);
    g.merge_conditions();
    g.analyse();
    let mut e = Emitter { g: &g, loops: Vec::new(), path: Vec::new(), trying: BTreeSet::new() };
    let mut body = e.seq(0, None, None);
    for h in &g.handlers {
        if !e.g.reachable_normally.contains(&h.target) && !e.emitted_handler(h) {
            body.push(Stmt::Unreadable(format!("a catch at {} outside any try", h.target)));
        }
    }
    body
}

struct Graph {
    blocks: BTreeMap<usize, Block>,
    handlers: Vec<Handler>,
    succ: BTreeMap<usize, Vec<usize>>,
    pred: BTreeMap<usize, Vec<usize>>,
    rpo: Vec<usize>,
    idom: BTreeMap<usize, usize>,
    ipdom: BTreeMap<usize, usize>,
    /// loop header -> the blocks of its loop
    loops: BTreeMap<usize, BTreeSet<usize>>,
    reachable_normally: BTreeSet<usize>,
}

impl Graph {
    fn new(blocks: BTreeMap<usize, Block>, handlers: Vec<Handler>) -> Graph {
        let mut g = Graph { blocks, handlers, succ: BTreeMap::new(), pred: BTreeMap::new(), rpo: vec![],
                            idom: BTreeMap::new(), ipdom: BTreeMap::new(), loops: BTreeMap::new(),
                            reachable_normally: BTreeSet::new() };
        g.edges();
        g
    }

    fn edges(&mut self) {
        self.succ.clear();
        self.pred.clear();
        for (s, b) in &self.blocks {
            let ss: Vec<usize> = b.term.succs().into_iter().filter(|t| self.blocks.contains_key(t)).collect();
            for t in &ss { self.pred.entry(*t).or_default().push(*s); }
            self.succ.insert(*s, ss);
        }
    }

    fn preds(&self, b: usize) -> &[usize] {
        self.pred.get(&b).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// `a && b` and `a || b` as tests: a block that only tests, reached only from another test
    /// that shares one of its targets, is part of that test's condition
    fn merge_conditions(&mut self) {
        loop {
            let mut changed = false;
            let starts: Vec<usize> = self.blocks.keys().copied().collect();
            for b in starts {
                let Some(Block { term: Term::Branch(c, t, f), .. }) = self.blocks.get(&b).cloned() else { continue };
                let pure = |x: usize, g: &Graph| -> Option<(Expr, usize, usize)> {
                    let blk = g.blocks.get(&x)?;
                    if x == b || !blk.stmts.is_empty() || blk.depth != 0 || g.preds(x) != [b] { return None }
                    match &blk.term { Term::Branch(c2, t2, f2) => Some((c2.clone(), *t2, *f2)), _ => None }
                };
                let merged = if let Some((c2, t2, f2)) = pure(t, self) {
                    if f2 == f { Some((t, Expr::And(Box::new(c.clone()), Box::new(c2)), t2, f)) }
                    else if t2 == f { Some((t, Expr::And(Box::new(c.clone()), Box::new(c2.negated())), f2, f)) }
                    else { None }
                } else { None }.or_else(|| {
                    let (c2, t2, f2) = pure(f, self)?;
                    if t2 == t { Some((f, Expr::Or(Box::new(c.clone()), Box::new(c2)), t, f2)) }
                    else if f2 == t { Some((f, Expr::Or(Box::new(c.clone()), Box::new(c2.negated())), t, t2)) }
                    else { None }
                });
                if let Some((gone, cond, nt, nf)) = merged {
                    self.blocks.remove(&gone);
                    self.blocks.get_mut(&b).unwrap().term = Term::Branch(cond, nt, nf);
                    self.edges();
                    changed = true;
                }
            }
            if !changed { break }
        }
    }

    fn analyse(&mut self) {
        // reverse postorder from the entry
        let mut seen = BTreeSet::new();
        let mut post = Vec::new();
        fn dfs(g: &Graph, b: usize, seen: &mut BTreeSet<usize>, post: &mut Vec<usize>) {
            let mut stack = vec![(b, 0usize)];
            seen.insert(b);
            while let Some((n, i)) = stack.pop() {
                let ss = g.succ.get(&n).cloned().unwrap_or_default();
                if i < ss.len() {
                    stack.push((n, i + 1));
                    let s = ss[i];
                    if seen.insert(s) { stack.push((s, 0)); }
                } else {
                    post.push(n);
                }
            }
        }
        if self.blocks.contains_key(&0) { dfs(self, 0, &mut seen, &mut post); }
        self.reachable_normally = seen.clone();
        for h in self.handlers.clone() {
            if self.blocks.contains_key(&h.target) && !seen.contains(&h.target) { dfs(self, h.target, &mut seen, &mut post); }
        }
        post.reverse();
        self.rpo = post;
        // one root above the entry and every handler, so any two blocks share a dominator
        let mut order: BTreeMap<usize, usize> = self.rpo.iter().enumerate().map(|(i, b)| (*b, i + 1)).collect();
        order.insert(ROOT, 0);

        // dominators (Cooper, Harvey, Kennedy)
        let roots: BTreeSet<usize> = std::iter::once(0).chain(self.handlers.iter().map(|h| h.target))
            .filter(|r| order.contains_key(r)).collect();
        let mut idom: BTreeMap<usize, usize> = BTreeMap::new();
        idom.insert(ROOT, ROOT);
        for r in &roots { idom.insert(*r, ROOT); }
        let intersect = |idom: &BTreeMap<usize, usize>, mut a: usize, mut b: usize| {
            while a != b {
                while order[&a] > order[&b] { a = idom[&a]; }
                while order[&b] > order[&a] { b = idom[&b]; }
            }
            a
        };
        loop {
            let mut changed = false;
            for &n in &self.rpo {
                if roots.contains(&n) { continue }
                let mut new: Option<usize> = None;
                for &p in self.preds(n) {
                    if !idom.contains_key(&p) { continue }
                    new = Some(match new { None => p, Some(x) => intersect(&idom, p, x) });
                }
                if let Some(d) = new { if idom.get(&n) != Some(&d) { idom.insert(n, d); changed = true; } }
            }
            if !changed { break }
        }
        self.idom = idom;

        // loops: a back edge goes to a block that dominates where it comes from
        let dominates = |a: usize, mut b: usize, idom: &BTreeMap<usize, usize>| loop {
            if a == b { return true }
            match idom.get(&b) { Some(&d) if d != b => b = d, _ => return false }
        };
        let mut loops: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for (&n, ss) in &self.succ {
            for &h in ss {
                if order.contains_key(&n) && dominates(h, n, &self.idom) {
                    let body = loops.entry(h).or_default();
                    body.insert(h);
                    let mut work = vec![n];
                    while let Some(x) = work.pop() {
                        if body.insert(x) { for &p in self.preds(x) { work.push(p) } }
                    }
                }
            }
        }
        self.loops = loops;

        // post-dominators, on the reversed graph from a single exit
        let exits: Vec<usize> = self.rpo.iter().copied().filter(|b| self.succ.get(b).is_none_or(|s| s.is_empty())).collect();
        let mut rseen = BTreeSet::new();
        let mut rpost = Vec::new();
        let rsucc = |n: usize| -> Vec<usize> { if n == EXIT { exits.clone() } else { self.preds(n).to_vec() } };
        {
            let mut stack = vec![(EXIT, 0usize)];
            rseen.insert(EXIT);
            while let Some((n, i)) = stack.pop() {
                let ss = rsucc(n);
                if i < ss.len() {
                    stack.push((n, i + 1));
                    if rseen.insert(ss[i]) { stack.push((ss[i], 0)); }
                } else { rpost.push(n); }
            }
        }
        rpost.reverse();
        let rorder: BTreeMap<usize, usize> = rpost.iter().enumerate().map(|(i, b)| (*b, i)).collect();
        let mut ipdom: BTreeMap<usize, usize> = BTreeMap::new();
        ipdom.insert(EXIT, EXIT);
        let rintersect = |ipdom: &BTreeMap<usize, usize>, mut a: usize, mut b: usize| {
            while a != b {
                while rorder[&a] > rorder[&b] { a = ipdom[&a]; }
                while rorder[&b] > rorder[&a] { b = ipdom[&b]; }
            }
            a
        };
        loop {
            let mut changed = false;
            for &n in &rpost {
                if n == EXIT { continue }
                let mut new: Option<usize> = None;
                let ss: Vec<usize> = { let s = self.succ.get(&n).cloned().unwrap_or_default(); if s.is_empty() { vec![EXIT] } else { s } };
                for p in ss {
                    if !ipdom.contains_key(&p) { continue }
                    new = Some(match new { None => p, Some(x) => rintersect(&ipdom, p, x) });
                }
                if let Some(d) = new { if ipdom.get(&n) != Some(&d) { ipdom.insert(n, d); changed = true; } }
            }
            if !changed { break }
        }
        self.ipdom = ipdom;
    }

    /// where a chain of blocks that only jump on ends
    fn through_jumps(&self, mut b: usize) -> usize {
        let mut seen = BTreeSet::new();
        while let Some(Block { stmts, term: Term::Goto(t), .. }) = self.blocks.get(&b) {
            if !stmts.is_empty() || !seen.insert(b) { break }
            b = *t;
        }
        b
    }

    fn follow_of(&self, h: usize) -> Option<usize> {
        let body = &self.loops[&h];
        let mut exits: Vec<usize> = Vec::new();
        for b in body {
            for s in self.succ.get(b).cloned().unwrap_or_default() {
                if !body.contains(&s) && !exits.contains(&s) { exits.push(s) }
            }
        }
        // the header's own way out, when it tests; else where the loop post-dominates to; else the
        // earliest exit
        if let Some(Block { term: Term::Branch(_, t, f), .. }) = self.blocks.get(&h) {
            if !body.contains(t) { return Some(*t) }
            if !body.contains(f) { return Some(*f) }
        }
        if exits.len() == 1 { return Some(exits[0]) }
        if let Some(p) = self.ipdom.get(&h).filter(|p| exits.contains(p)) { return Some(*p) }
        exits.into_iter().min()
    }
}

struct LoopCtx {
    header: usize,
    body: BTreeSet<usize>,
    follow: Option<usize>,
}

struct Emitter<'a> {
    g: &'a Graph,
    loops: Vec<LoopCtx>,
    path: Vec<usize>,
    trying: BTreeSet<(usize, usize)>,
}

impl Emitter<'_> {
    fn emitted_handler(&self, _h: &Handler) -> bool { true }

    /// where the ways out of a test at `b` meet again, within the loop being emitted: the block
    /// that post-dominates it; or, where a way returns (so only the exit post-dominates), the
    /// first block every way that goes on reaches
    fn merge_of(&self, b: usize) -> Option<usize> {
        let inside = |m: usize| match self.loops.last() {
            Some(l) => l.body.contains(&m) && m != l.header,
            None => true,
        };
        match self.g.ipdom.get(&b) {
            Some(&m) if m != EXIT => return inside(m).then_some(m),
            None => return None,
            _ => {}
        }
        let succs = self.g.succ.get(&b).cloned().unwrap_or_default();
        let reach = |from: usize| -> BTreeSet<usize> {
            let mut seen = BTreeSet::new();
            let mut work = vec![from];
            while let Some(x) = work.pop() {
                if !inside(x) || !seen.insert(x) { continue }
                for s in self.g.succ.get(&x).cloned().unwrap_or_default() { work.push(s) }
            }
            seen
        };
        // ways that end (a return, a throw) do not decide where the others meet
        let sets: Vec<BTreeSet<usize>> = succs.iter().map(|s| reach(*s))
            .filter(|set| set.iter().any(|x| !self.g.succ.get(x).is_none_or(|v| v.is_empty()) || set.len() > 1))
            .collect();
        if sets.len() < 2 { return None }
        let order: BTreeMap<usize, usize> = self.g.rpo.iter().enumerate().map(|(i, x)| (*x, i)).collect();
        let common: Vec<usize> = sets[0].iter().copied().filter(|x| sets[1..].iter().all(|s| s.contains(x))).collect();
        common.into_iter().filter(|x| *x != b).min_by_key(|x| order.get(x).copied().unwrap_or(usize::MAX))
    }

    /// a jump to where a loop goes on or goes round
    fn loop_control(&self, b: usize) -> Option<Stmt> {
        let l = self.loops.last()?;
        if b == l.header { return Some(Stmt::Continue) }
        if Some(b) == l.follow { return Some(Stmt::Break) }
        for outer in self.loops.iter().rev().skip(1) {
            if b == outer.header || Some(b) == outer.follow {
                return Some(Stmt::Unreadable(format!("a jump from an inner loop to an outer one's {}",
                    if b == outer.header { "start" } else { "end" })));
            }
        }
        None
    }

    fn seq(&mut self, mut b: usize, stop: Option<usize>, mut enter: Option<usize>) -> Vec<Stmt> {
        let mark = self.path.len();
        let mut out = Vec::new();
        loop {
            if Some(b) == stop { break }
            let entering = enter.take() == Some(b);
            if !entering {
                if let Some(ctrl) = self.loop_control(b) { out.push(ctrl); break }
                if self.g.loops.contains_key(&b) {
                    let body = self.g.loops[&b].clone();
                    let follow = self.g.follow_of(b);
                    self.loops.push(LoopCtx { header: b, body, follow });
                    let inner = self.seq(b, None, Some(b));
                    self.loops.pop();
                    out.push(Stmt::Loop(inner));
                    match follow { Some(f) => { b = f; continue } None => break }
                }
                if let Some((from, to)) = self.try_at(b) {
                    let (stmt, after) = self.try_block(from, to, stop);
                    out.push(stmt);
                    match after { Some(a) => { b = a; continue } None => break }
                }
            }
            if self.path.contains(&b) {
                out.push(Stmt::Unreadable(format!("a jump back to {b} that is not a loop")));
                break;
            }
            self.path.push(b);
            let Some(block) = self.g.blocks.get(&b) else {
                out.push(Stmt::Unreadable(format!("a jump to {b}, where no code is")));
                break;
            };
            out.extend(block.stmts.iter().cloned());
            match &block.term {
                Term::Return(v) => { out.push(Stmt::Return(v.clone())); break }
                Term::Throw(e) => { out.push(Stmt::Throw(e.clone())); break }
                Term::End => break,
                Term::Goto(t) => { b = *t; }
                Term::Branch(c, t, f) => {
                    let merge = self.merge_of(b);
                    let (c, t, f) = (c.clone(), *t, *f);
                    let inner_stop = merge.or(stop);
                    let then_ = self.seq(t, inner_stop, None);
                    let else_ = self.seq(f, inner_stop, None);
                    out.push(Stmt::If(c, then_, else_));
                    match merge { Some(m) => { b = m; } None => break }
                }
                Term::Switch(ix, cases, default) => {
                    let merge = self.merge_of(b);
                    let inner_stop = merge.or(stop);
                    let (ix, cases, default) = (ix.clone(), cases.clone(), *default);
                    let mut arms: Vec<(Vec<i64>, usize)> = Vec::new();
                    for (k, t) in cases.iter().enumerate() {
                        if *t == default { continue }
                        match arms.iter_mut().find(|(_, x)| x == t) {
                            Some((ks, _)) => ks.push(k as i64),
                            None => arms.push((vec![k as i64], *t)),
                        }
                    }
                    let arms: Vec<(Vec<Expr>, Vec<Stmt>)> = arms.into_iter()
                        .map(|(ks, t)| (ks.into_iter().map(Expr::Int).collect(), self.seq(t, inner_stop, None))).collect();
                    let d = self.seq(default, inner_stop, None);
                    out.push(Stmt::Switch(ix, arms, d));
                    match merge { Some(m) => { b = m; } None => break }
                }
            }
        }
        self.path.truncate(mark);
        out
    }

    /// a protected range that starts at `b` and is not already being emitted
    fn try_at(&self, b: usize) -> Option<(usize, usize)> {
        self.g.handlers.iter().filter(|h| h.from == b && !self.trying.contains(&(h.from, h.to)))
            .map(|h| (h.from, h.to)).max_by_key(|(_, to)| *to)
    }

    fn try_block(&mut self, from: usize, to: usize, stop: Option<usize>) -> (Stmt, Option<usize>) {
        // where the protected code goes when it finishes normally: the jump its last block makes
        let last = self.g.blocks.range(..to).next_back().map(|(_, b)| b);
        // (the compiler jumps over the handlers through a block that only jumps on: where the
        // protected code and its handlers meet again is where that chain ends)
        let after = match last.map(|b| &b.term) {
            Some(Term::Goto(t)) if *t >= to => Some(self.g.through_jumps(*t)),
            _ => None,
        }.or(stop);
        self.trying.insert((from, to));
        let body = self.seq(from, after, None);
        let handlers: Vec<Handler> = self.g.handlers.iter().filter(|h| h.from == from && h.to == to).cloned().collect();
        let catches = handlers.into_iter().map(|h| Catch {
            index: h.index, var: h.var.clone(), type_name: h.type_name.clone(),
            body: self.seq(h.target, after, None),
        }).collect();
        self.trying.remove(&(from, to));
        (Stmt::Try(body, catches), after)
    }
}
