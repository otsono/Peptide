//! Bytecode to blocks of statements.
//!
//! A method's code is split into basic blocks, and each block is run on a symbolic operand stack:
//! what an instruction pushes is an expression, what it does is a statement. A value still on the
//! stack when a block ends reaches its successors through a join slot (`Expr::Join`) each of
//! them reads, so a value decided by a branch is always right, whatever shape it has. The
//! structurer turns the blocks into loops and conditionals; tidying folds the join slots back into
//! the expressions they were (`c ? a : b`, `a && b`).

use std::collections::{BTreeMap, BTreeSet};

use swf::avm2::read::Reader;
use swf::avm2::types::{AbcFile, DefaultValue, Index, Method, Multiname, Namespace, Op, TraitKind};
use swf::extensions::ReadSwfExt;

use super::ast::*;

/// What a method's code refers to by index, answered by name.
pub struct Pool<'a> {
    pub abc: &'a AbcFile,
}

impl<'a> Pool<'a> {
    pub fn string(&self, i: u32) -> String {
        if i == 0 { return String::new() }
        self.abc.constant_pool.strings.get(i as usize - 1)
            .map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default()
    }

    pub fn multiname(&self, i: u32) -> Option<&'a Multiname> {
        if i == 0 { return None }
        self.abc.constant_pool.multinames.get(i as usize - 1)
    }

    fn namespace(&self, i: u32) -> String {
        if i == 0 { return String::new() }
        match self.abc.constant_pool.namespaces.get(i as usize - 1) {
            Some(Namespace::Namespace(s) | Namespace::Package(s) | Namespace::PackageInternal(s)
                 | Namespace::Protected(s) | Namespace::Explicit(s) | Namespace::StaticProtected(s)
                 | Namespace::Private(s)) => self.string(s.0),
            None => String::new(),
        }
    }

    /// the name a multiname states (empty for one named at run time)
    pub fn name(&self, i: u32) -> String {
        match self.multiname(i) {
            Some(Multiname::QName { name, .. } | Multiname::QNameA { name, .. }
                 | Multiname::RTQName { name } | Multiname::RTQNameA { name }
                 | Multiname::Multiname { name, .. } | Multiname::MultinameA { name, .. }) => self.string(name.0),
            Some(Multiname::TypeName { base_type, parameters }) => format!("{}.<{}>", self.name(base_type.0),
                parameters.iter().map(|p| self.name(p.0)).collect::<Vec<_>>().join(", ")),
            _ => String::new(),
        }
    }

    /// a class's name as the package binds it: a timeline class keeps its package
    /// (`pichu_fla.DSpecial_43`), everything else is its own name
    pub fn class_name(&self, i: u32) -> String {
        match self.multiname(i) {
            Some(Multiname::QName { namespace, name }) => {
                let ns = self.namespace(namespace.0);
                let n = self.string(name.0);
                if ns.ends_with("_fla") { format!("{ns}.{n}") } else { n }
            }
            _ => self.name(i),
        }
    }

    /// a declared type, or `None` for "any"
    pub fn type_name(&self, i: u32) -> Option<String> {
        if i == 0 { return None }
        Some(self.class_name(i)).filter(|n| !n.is_empty() && n != "*")
    }

    /// how many operands a multiname takes off the stack: (a run-time namespace, a run-time name)
    fn runtime_parts(&self, i: u32) -> (bool, bool) {
        match self.multiname(i) {
            Some(Multiname::RTQName { .. } | Multiname::RTQNameA { .. }) => (true, false),
            Some(Multiname::RTQNameL | Multiname::RTQNameLA) => (true, true),
            Some(Multiname::MultinameL { .. } | Multiname::MultinameLA { .. }) => (false, true),
            _ => (false, false),
        }
    }

    pub fn default_value(&self, v: &DefaultValue) -> Expr {
        let pool = &self.abc.constant_pool;
        match v {
            DefaultValue::Int(i) => pool.ints.get(i.0 as usize - 1).map(|v| Expr::Int(*v as i64)).unwrap_or(Expr::Undefined),
            DefaultValue::Uint(i) => pool.uints.get(i.0 as usize - 1).map(|v| Expr::Int(*v as i64)).unwrap_or(Expr::Undefined),
            DefaultValue::Double(i) => pool.doubles.get(i.0 as usize - 1).map(|v| Expr::Num(*v)).unwrap_or(Expr::Undefined),
            DefaultValue::String(i) => Expr::Str(self.string(i.0)),
            DefaultValue::True => Expr::Bool(true),
            DefaultValue::False => Expr::Bool(false),
            DefaultValue::Null => Expr::Null,
            DefaultValue::Undefined => Expr::Undefined,
            _ => Expr::Unreadable("a namespace as a default".into()),
        }
    }
}

/// The names slots are known by, wherever an object's slots are read.
#[derive(Default, Clone)]
pub struct Slots {
    /// the object's own (instance) slots
    pub this: BTreeMap<u32, String>,
    /// the package's global slots
    pub global: BTreeMap<u32, String>,
}

pub fn slot_names(pool: &Pool, traits: &[swf::avm2::types::Trait]) -> BTreeMap<u32, String> {
    traits.iter().filter_map(|t| match &t.kind {
        TraitKind::Slot { slot_id, .. } | TraitKind::Const { slot_id, .. }
        | TraitKind::Class { slot_id, .. } | TraitKind::Function { slot_id, .. } if *slot_id > 0 =>
            Some((*slot_id, pool.name(t.name.0))),
        _ => None,
    }).collect()
}

/// one decoded instruction
struct Ins {
    at: usize,
    next: usize,
    op: Op,
}

fn read_code(code: &[u8]) -> Result<Vec<Ins>, String> {
    let mut out = Vec::new();
    let mut r = Reader::new(code);
    while r.pos(code) < code.len() {
        let at = r.pos(code);
        let op = r.read_op().map_err(|e| format!("instruction at {at}: {e}"))?;
        out.push(Ins { at, next: r.pos(code), op });
    }
    Ok(out)
}

/// How a block ends.
#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    Return(Option<Expr>),
    Throw(Expr),
    Goto(usize),
    /// if the condition holds, to the first; else the second
    Branch(Expr, usize, usize),
    Switch(Expr, Vec<usize>, usize),
    /// the code ends without returning
    End,
}

impl Term {
    pub fn succs(&self) -> Vec<usize> {
        match self {
            Term::Goto(t) => vec![*t],
            Term::Branch(_, a, b) => if a == b { vec![*a] } else { vec![*a, *b] },
            Term::Switch(_, cases, d) => {
                let mut v: Vec<usize> = cases.clone();
                v.push(*d);
                let mut seen = BTreeSet::new();
                v.retain(|x| seen.insert(*x));
                v
            }
            _ => vec![],
        }
    }
}

/// A basic block, decoded.
#[derive(Clone, Debug)]
pub struct Block {
    pub start: usize,
    pub stmts: Vec<Stmt>,
    pub term: Term,
    /// how deep the operand stack is on the way in
    pub depth: usize,
}

/// An exception handler as the method's table states it.
#[derive(Clone, Debug)]
pub struct Handler {
    pub index: u32,
    pub from: usize,
    pub to: usize,
    pub target: usize,
    pub var: Option<String>,
    pub type_name: Option<String>,
}

pub struct Decoded {
    pub blocks: BTreeMap<usize, Block>,
    pub handlers: Vec<Handler>,
    pub temps: u32,
}

/// Decode one method's body into blocks. `nested` decodes a closure the body makes.
pub fn decode_body(pool: &Pool, method: u32, slots: &Slots,
                   nested: &mut dyn FnMut(u32) -> Function) -> Result<Decoded, String> {
    let m = &pool.abc.methods[method as usize];
    let body = m.body.and_then(|b| pool.abc.method_bodies.get(b.0 as usize))
        .ok_or_else(|| "no body".to_string())?;
    let ins = read_code(&body.code)?;
    let activation = slot_names(pool, &body.traits);
    let handlers: Vec<Handler> = body.exceptions.iter().enumerate().map(|(i, e)| Handler {
        index: i as u32, from: e.from_offset as usize, to: e.to_offset as usize,
        target: e.target_offset as usize,
        var: Some(pool.name(e.variable_name.0)).filter(|n| !n.is_empty()),
        type_name: pool.type_name(e.type_name.0),
    }).collect();

    // where blocks start
    let mut leaders: BTreeSet<usize> = BTreeSet::new();
    leaders.insert(0);
    for h in &handlers { leaders.insert(h.from); leaders.insert(h.to); leaders.insert(h.target); }
    for i in &ins {
        match &i.op {
            Op::Jump { offset } | Op::IfTrue { offset } | Op::IfFalse { offset } | Op::IfEq { offset }
            | Op::IfNe { offset } | Op::IfLt { offset } | Op::IfLe { offset } | Op::IfGt { offset }
            | Op::IfGe { offset } | Op::IfNlt { offset } | Op::IfNle { offset } | Op::IfNgt { offset }
            | Op::IfNge { offset } | Op::IfStrictEq { offset } | Op::IfStrictNe { offset } => {
                leaders.insert((i.next as i64 + *offset as i64) as usize);
                leaders.insert(i.next);
            }
            Op::LookupSwitch(ls) => {
                leaders.insert((i.at as i64 + ls.default_offset as i64) as usize);
                for c in ls.case_offsets.iter() { leaders.insert((i.at as i64 + *c as i64) as usize); }
                leaders.insert(i.next);
            }
            Op::ReturnValue | Op::ReturnVoid | Op::Throw => { leaders.insert(i.next); }
            _ => {}
        }
    }
    let code_end = body.code.len();
    let starts: Vec<usize> = leaders.into_iter().filter(|s| *s < code_end).collect();
    let index_of: BTreeMap<usize, usize> = ins.iter().enumerate().map(|(k, i)| (i.at, k)).collect();

    // blocks in the order their predecessors are decoded: what a block's stack holds on the way
    // in is what its first decoded predecessor left
    let mut depth_in: BTreeMap<usize, usize> = BTreeMap::new();
    depth_in.insert(0, 0);
    for h in &handlers { depth_in.insert(h.target, 1); }
    let handler_at: BTreeMap<usize, u32> = handlers.iter().map(|h| (h.target, h.index)).collect();
    let mut out: BTreeMap<usize, Block> = BTreeMap::new();
    // the entry first, the handlers last: a handler starts from the scopes the method set up
    let mut pending: Vec<usize> = handlers.iter().map(|h| h.target).collect();
    pending.push(0);
    let mut temps = 0u32;
    let mut scopes_in: BTreeMap<usize, Vec<Expr>> = BTreeMap::new();
    let mut held: BTreeMap<u32, Expr> = BTreeMap::new();
    // join slots are written at the end of the block that leaves the value, once every successor
    // is known; collect what each block left
    let mut left: BTreeMap<usize, Vec<Expr>> = BTreeMap::new();
    while let Some(start) = pending.pop() {
        if out.contains_key(&start) { continue }
        let Some(&first) = index_of.get(&start) else { continue };
        let end = starts.iter().copied().find(|s| *s > start).unwrap_or(code_end);
        let depth = depth_in.get(&start).copied().unwrap_or(0);
        let entry: Vec<Expr> = match handler_at.get(&start) {
            Some(ix) => vec![Expr::Caught(*ix)],
            None => (0..depth).map(|slot| Expr::Join { block: start, slot: slot as u32 }).collect(),
        };
        let mut sim = Sim {
            pool, slots, activation: &activation, nested: &mut *nested, stack: entry, stmts: Vec::new(),
            scopes: scopes_in.get(&start).cloned().unwrap_or_default(), temps: &mut temps,
            this_rebound: false, handlers: &handlers, held: &mut held,
        };
        let mut term = None;
        let mut k = first;
        while k < ins.len() && ins[k].at < end {
            let i = &ins[k];
            if let Some(t) = sim.step(i) { term = Some(t); break }
            k += 1;
        }
        let term = term.unwrap_or(if end < code_end { Term::Goto(end) } else { Term::End });
        let scopes = sim.scopes.clone();
        let exit = std::mem::take(&mut sim.stack);
        let stmts = std::mem::take(&mut sim.stmts);
        for s in term.succs() {
            depth_in.entry(s).or_insert(exit.len());
            scopes_in.entry(s).or_insert_with(|| scopes.clone());
            pending.push(s);
        }
        left.insert(start, exit);
        out.insert(start, Block { start, stmts, term, depth });
    }

    // what a block left on the stack goes to the join slots of every block it continues to
    for (start, exit) in left {
        if exit.is_empty() { continue }
        let succs = out[&start].term.succs();
        let block = out.get_mut(&start).unwrap();
        // a value that reads a join slot this block also writes is taken first, so the writes do
        // not see each other (a loop that carries a value round to itself)
        let writes: BTreeSet<usize> = succs.iter().copied().collect();
        let reads_written = |e: &Expr| {
            let mut hit = false;
            e.walk(&mut |x| if let Expr::Join { block, .. } = x { if writes.contains(block) { hit = true } });
            hit
        };
        let mut values = exit.clone();
        // a value that goes more than one way is computed once, here
        let many = succs.len() > 1;
        for v in values.iter_mut() {
            let copy = matches!(v, Expr::Local(_) | Expr::Temp(_) | Expr::This | Expr::Null | Expr::Undefined
                | Expr::Bool(_) | Expr::Num(_) | Expr::Int(_) | Expr::Str(_) | Expr::ScopeOf(_) | Expr::Activation
                | Expr::Global | Expr::Caught(_));
            if reads_written(v) || (many && !copy) {
                let t = temps; temps += 1;
                block.stmts.push(Stmt::Assign(Place::Temp(t), v.clone()));
                *v = Expr::Temp(t);
            }
        }
        let cond_reads = match &block.term {
            Term::Branch(c, _, _) | Term::Switch(c, _, _) => reads_written(c),
            _ => false,
        };
        if cond_reads {
            let t = temps; temps += 1;
            let c = match &mut block.term { Term::Branch(c, _, _) | Term::Switch(c, _, _) => c, _ => unreachable!() };
            block.stmts.push(Stmt::Assign(Place::Temp(t), c.clone()));
            *c = Expr::Temp(t);
        }
        for s in succs {
            let Some(depth) = out.get(&s).map(|b| b.depth) else {
                let b = out.get_mut(&start).unwrap();
                b.stmts.push(Stmt::Unreadable(format!("a jump to offset {s}, where no block starts")));
                continue;
            };
            if depth != values.len() {
                let b = out.get_mut(&start).unwrap();
                b.stmts.push(Stmt::Unreadable(format!("stack depth {} into a block that takes {depth}", values.len())));
                continue;
            }
            let b = out.get_mut(&start).unwrap();
            for (slot, v) in values.iter().enumerate() {
                let join = Expr::Join { block: s, slot: slot as u32 };
                if *v != join {
                    b.stmts.push(Stmt::Assign(Place::Join { block: s, slot: slot as u32 }, v.clone()));
                }
            }
        }
    }
    Ok(Decoded { blocks: out, handlers, temps })
}

struct Sim<'a, 'p, 'n> {
    pool: &'a Pool<'p>,
    slots: &'a Slots,
    activation: &'a BTreeMap<u32, String>,
    nested: &'a mut (dyn FnMut(u32) -> Function + 'n),
    stack: Vec<Expr>,
    stmts: Vec<Stmt>,
    scopes: Vec<Expr>,
    temps: &'a mut u32,
    this_rebound: bool,
    handlers: &'a [Handler],
    /// registers that hold the activation or a catch scope (shared by every block)
    held: &'a mut BTreeMap<u32, Expr>,
}

impl Sim<'_, '_, '_> {
    fn pop(&mut self) -> Expr {
        self.stack.pop().unwrap_or_else(|| Expr::Unreadable("stack underflow".into()))
    }

    fn pop_n(&mut self, n: u32) -> Vec<Expr> {
        let mut v: Vec<Expr> = (0..n).map(|_| self.pop()).collect();
        v.reverse();
        v
    }

    fn temp(&mut self) -> u32 {
        let t = *self.temps;
        *self.temps += 1;
        t
    }

    /// A statement. Anything still on the stack was computed before it, so whatever of that it
    /// could change is taken into a temp first, in stack order.
    fn emit(&mut self, s: Stmt) {
        let writes_local: Option<u32> = match &s { Stmt::Assign(Place::Local(r), _) => Some(*r), _ => None };
        let writes_name: Option<String> = match &s {
            Stmt::Assign(Place::Name(n) | Place::Member(Expr::This, n), _) => Some(n.clone()),
            _ => None,
        };
        let only_writes = matches!(&s, Stmt::Assign(_, v) if !v.has_effect());
        for k in 0..self.stack.len() {
            let e = &self.stack[k];
            let keep = e.is_stable() || matches!(e, Expr::Local(r) if Some(*r) != writes_local)
                || matches!(e, Expr::ScopeOf(_))
                || matches!(e, Expr::Name(n) if only_writes && Some(n) != writes_name.as_ref());
            if keep { continue }
            let t = self.temp();
            let v = std::mem::replace(&mut self.stack[k], Expr::Temp(t));
            self.stmts.push(Stmt::Assign(Place::Temp(t), v));
        }
        self.stmts.push(s);
    }

    fn local(&self, r: u32) -> Expr {
        if r == 0 && !self.this_rebound { Expr::This } else { Expr::Local(r) }
    }

    /// the place a property operand names on `obj`
    fn place(&mut self, obj: Expr, index: u32) -> Place {
        let name = self.pool.name(index);
        match obj {
            Expr::ScopeOf(n) if n == name => Place::Name(name),
            Expr::Activation => Place::Name(name),
            o => Place::Member(o, name),
        }
    }

    /// take the run-time parts of a multiname off the stack (name above namespace), then the
    /// object: `(object, key)` where a key is a run-time name
    fn take_name(&mut self, index: u32) -> (Expr, Option<Expr>) {
        let (rt_ns, rt_name) = self.pool.runtime_parts(index);
        let key = if rt_name { Some(self.pop()) } else { None };
        if rt_ns { let _ = self.pop(); }
        let obj = self.pop();
        (obj, key)
    }

    fn property(&mut self, index: u32) -> Place {
        let (obj, key) = self.take_name(index);
        match key {
            Some(k) => Place::Index(obj, k),
            None => self.place(obj, index),
        }
    }

    fn slot(&self, obj: &Expr, id: u32) -> Place {
        let named = |m: &BTreeMap<u32, String>| m.get(&id).cloned();
        match obj {
            Expr::Activation => named(self.activation).map(Place::Name),
            Expr::This => named(&self.slots.this).map(|n| Place::Member(Expr::This, n)),
            Expr::Global => named(&self.slots.global).map(Place::Name),
            // a catch's variable is the first slot of its scope object
            Expr::Caught(i) if id == 1 => self.handlers.iter().find(|h| h.index == *i)
                .map(|_| Place::Member(obj.clone(), "#slot1".into())),
            _ => None,
        }.unwrap_or_else(|| Place::Member(obj.clone(), format!("#slot{id}")))
    }

    fn binary(&mut self, op: BinOp) {
        let b = self.pop();
        let a = self.pop();
        self.stack.push(Expr::Binary(op, Box::new(a), Box::new(b)));
    }

    fn unary(&mut self, op: UnOp) {
        let a = self.pop();
        self.stack.push(Expr::Unary(op, Box::new(a)));
    }

    fn convert(&mut self, c: Conv) {
        let a = self.pop();
        // converting what already is that kind changes nothing
        let same = match (&c, &a) {
            (Conv::Boolean, Expr::Bool(_)) | (Conv::Number, Expr::Num(_) | Expr::Int(_))
            | (Conv::Int, Expr::Int(_)) | (Conv::String, Expr::Str(_)) => true,
            (_, Expr::Convert(k, _)) => *k == c,
            _ => false,
        };
        self.stack.push(if same { a } else { Expr::Convert(c, Box::new(a)) });
    }

    fn step(&mut self, i: &Ins) -> Option<Term> {
        let target = |offset: &i32| (i.next as i64 + *offset as i64) as usize;
        match &i.op {
            // ---- constants
            Op::PushByte { value } => self.stack.push(Expr::Int(*value as i8 as i64)),
            Op::PushShort { value } => self.stack.push(Expr::Int(*value as i64)),
            Op::PushInt { value } => {
                let v = self.pool.abc.constant_pool.ints.get(value.0 as usize - 1).copied().unwrap_or(0);
                self.stack.push(Expr::Int(v as i64));
            }
            Op::PushUint { value } => {
                let v = self.pool.abc.constant_pool.uints.get(value.0 as usize - 1).copied().unwrap_or(0);
                self.stack.push(Expr::Int(v as i64));
            }
            Op::PushDouble { value } => {
                let v = self.pool.abc.constant_pool.doubles.get(value.0 as usize - 1).copied().unwrap_or(f64::NAN);
                self.stack.push(Expr::Num(v));
            }
            Op::PushNaN => self.stack.push(Expr::Num(f64::NAN)),
            Op::PushString { value } => self.stack.push(Expr::Str(self.pool.string(value.0))),
            Op::PushTrue => self.stack.push(Expr::Bool(true)),
            Op::PushFalse => self.stack.push(Expr::Bool(false)),
            Op::PushNull => self.stack.push(Expr::Null),
            Op::PushUndefined => self.stack.push(Expr::Undefined),
            Op::PushNamespace { .. } => self.stack.push(Expr::Unreadable("a namespace value".into())),

            // ---- registers
            Op::GetLocal { index } => {
                let e = self.held.get(index).cloned().unwrap_or_else(|| self.local(*index));
                self.stack.push(e)
            }
            Op::SetLocal { index } => {
                let v = self.pop();
                if matches!(v, Expr::Activation | Expr::Caught(_)) {
                    // the activation or a catch's scope kept in a register: plumbing, not a
                    // variable, and a read of the register is a read of that object
                    self.held.insert(*index, v);
                } else {
                    self.held.remove(index);
                    if *index == 0 { self.this_rebound = true; }
                    self.emit(Stmt::Assign(Place::Local(*index), v));
                }
            }
            Op::Kill { .. } => {}
            Op::IncLocal { index } | Op::IncLocalI { index } => {
                let r = self.local(*index);
                self.emit(Stmt::Assign(Place::Local(*index), Expr::Binary(BinOp::Add, Box::new(r), Box::new(Expr::Int(1)))));
            }
            Op::DecLocal { index } | Op::DecLocalI { index } => {
                let r = self.local(*index);
                self.emit(Stmt::Assign(Place::Local(*index), Expr::Binary(BinOp::Sub, Box::new(r), Box::new(Expr::Int(1)))));
            }

            // ---- the stack itself
            Op::Pop => {
                let v = self.pop();
                if v.has_effect() { self.emit(Stmt::Expr(v)); }
            }
            Op::Dup => {
                let v = self.pop();
                if v.is_stable() || matches!(v, Expr::Local(_) | Expr::Name(_) | Expr::ScopeOf(_)) {
                    self.stack.push(v.clone());
                    self.stack.push(v);
                } else {
                    let t = self.temp();
                    self.emit(Stmt::Assign(Place::Temp(t), v));
                    self.stack.push(Expr::Temp(t));
                    self.stack.push(Expr::Temp(t));
                }
            }
            Op::Swap => {
                let b = self.pop();
                let a = self.pop();
                self.stack.push(b);
                self.stack.push(a);
            }

            // ---- scopes
            Op::PushScope | Op::PushWith => {
                let v = self.pop();
                if matches!(i.op, Op::PushWith) { self.stmts.push(Stmt::Unreadable("a `with` block".into())); }
                self.scopes.push(v);
            }
            Op::PopScope => { self.scopes.pop(); }
            Op::GetScopeObject { index } => {
                let e = self.scopes.get(*index as usize).cloned().unwrap_or(if *index == 0 { Expr::This } else { Expr::Global });
                self.stack.push(e);
            }
            Op::GetOuterScope { .. } | Op::GetGlobalScope | Op::FindDef { .. } => self.stack.push(Expr::Global),
            Op::NewActivation => self.stack.push(Expr::Activation),
            Op::NewCatch { index } => self.stack.push(Expr::Caught(index.0)),
            Op::FindPropStrict { index } | Op::FindProperty { index } => {
                let (_, rt_name) = self.pool.runtime_parts(index.0);
                if rt_name { let _ = self.pop(); self.stack.push(Expr::Global) } else {
                    self.stack.push(Expr::ScopeOf(self.pool.name(index.0)))
                }
            }
            Op::GetLex { index } => self.stack.push(Expr::Name(self.pool.class_name(index.0))),

            // ---- properties
            Op::GetProperty { index } => {
                let p = self.property(index.0);
                self.stack.push(p.as_expr());
            }
            Op::SetProperty { index } | Op::InitProperty { index } => {
                let v = self.pop();
                let p = self.property(index.0);
                self.emit(Stmt::Assign(p, v));
            }
            Op::DeleteProperty { index } => {
                let p = self.property(index.0);
                self.stack.push(Expr::Delete(Box::new(p)));
            }
            Op::GetSuper { index } => {
                let (_obj, key) = self.take_name(index.0);
                self.stack.push(match key { Some(_) => Expr::Unreadable("a super property named at run time".into()),
                                            None => Expr::SuperMember(self.pool.name(index.0)) });
            }
            Op::SetSuper { index } => {
                let v = self.pop();
                let _ = self.take_name(index.0);
                self.emit(Stmt::Assign(Place::SuperMember(self.pool.name(index.0)), v));
            }
            Op::GetSlot { index } => {
                let obj = self.pop();
                let p = self.slot(&obj, *index);
                self.stack.push(p.as_expr());
            }
            Op::SetSlot { index } => {
                let v = self.pop();
                let obj = self.pop();
                let p = self.slot(&obj, *index);
                self.emit(Stmt::Assign(p, v));
            }
            Op::GetGlobalSlot { index } => {
                let p = self.slot(&Expr::Global, *index);
                self.stack.push(p.as_expr());
            }
            Op::SetGlobalSlot { index } => {
                let v = self.pop();
                let p = self.slot(&Expr::Global, *index);
                self.emit(Stmt::Assign(p, v));
            }

            // ---- calls
            Op::CallProperty { index, num_args } | Op::CallPropLex { index, num_args } | Op::CallPropVoid { index, num_args } => {
                let args = self.pop_n(*num_args);
                let callee = self.property(index.0).as_expr();
                let call = Expr::Call(Box::new(callee), args);
                if matches!(i.op, Op::CallPropVoid { .. }) { self.emit(Stmt::Expr(call)) } else { self.stack.push(call) }
            }
            Op::CallSuper { index, num_args } | Op::CallSuperVoid { index, num_args } => {
                let args = self.pop_n(*num_args);
                let _ = self.take_name(index.0);
                let call = Expr::SuperCall(self.pool.name(index.0), args);
                if matches!(i.op, Op::CallSuperVoid { .. }) { self.emit(Stmt::Expr(call)) } else { self.stack.push(call) }
            }
            Op::Call { num_args } => {
                let args = self.pop_n(*num_args);
                let _receiver = self.pop();
                let f = self.pop();
                self.stack.push(Expr::Call(Box::new(f), args));
            }
            Op::CallMethod { num_args, .. } | Op::CallStatic { num_args, .. } => {
                let _args = self.pop_n(*num_args);
                let _ = self.pop();
                self.stack.push(Expr::Unreadable("a call by dispatch id".into()));
            }
            Op::Construct { num_args } => {
                let args = self.pop_n(*num_args);
                let c = self.pop();
                self.stack.push(Expr::New(Box::new(c), args));
            }
            Op::ConstructProp { index, num_args } => {
                let args = self.pop_n(*num_args);
                let c = self.property(index.0).as_expr();
                self.stack.push(Expr::New(Box::new(c), args));
            }
            Op::ConstructSuper { num_args } => {
                let args = self.pop_n(*num_args);
                let _ = self.pop();
                self.emit(Stmt::Expr(Expr::SuperCall(String::new(), args)));
            }
            Op::ApplyType { num_types } => {
                let ts = self.pop_n(*num_types);
                let base = self.pop();
                self.stack.push(Expr::TypeApply(Box::new(base), ts));
            }

            // ---- making things
            Op::NewObject { num_args } => {
                let flat = self.pop_n(num_args * 2);
                let kv = flat.chunks(2).map(|p| (p[0].clone(), p[1].clone())).collect();
                self.stack.push(Expr::Object(kv));
            }
            Op::NewArray { num_args } => { let a = self.pop_n(*num_args); self.stack.push(Expr::Array(a)) }
            Op::NewFunction { index } => { let f = (self.nested)(index.0); self.stack.push(Expr::Function(Box::new(f))) }
            Op::NewClass { index } => { let _base = self.pop(); self.stack.push(Expr::Class(index.0)) }

            // ---- arithmetic and logic
            Op::Add => self.binary(BinOp::Add),
            Op::Subtract => self.binary(BinOp::Sub),
            Op::Multiply => self.binary(BinOp::Mul),
            Op::AddI | Op::SubtractI | Op::MultiplyI => {
                self.binary(match i.op { Op::AddI => BinOp::Add, Op::SubtractI => BinOp::Sub, _ => BinOp::Mul });
                self.convert(Conv::Int);
            }
            Op::Divide => self.binary(BinOp::Div),
            Op::Modulo => self.binary(BinOp::Mod),
            Op::LShift => self.binary(BinOp::Shl),
            Op::RShift => self.binary(BinOp::Shr),
            Op::URShift => self.binary(BinOp::UShr),
            Op::BitAnd => self.binary(BinOp::BitAnd),
            Op::BitOr => self.binary(BinOp::BitOr),
            Op::BitXor => self.binary(BinOp::BitXor),
            Op::BitNot => self.unary(UnOp::BitNot),
            Op::Negate => self.unary(UnOp::Neg),
            Op::NegateI => { self.unary(UnOp::Neg); self.convert(Conv::Int) }
            Op::Increment | Op::IncrementI | Op::Decrement | Op::DecrementI => {
                let a = self.pop();
                let op = if matches!(i.op, Op::Increment | Op::IncrementI) { BinOp::Add } else { BinOp::Sub };
                self.stack.push(Expr::Binary(op, Box::new(Expr::Convert(Conv::Number, Box::new(a))), Box::new(Expr::Int(1))));
                if matches!(i.op, Op::IncrementI | Op::DecrementI) { self.convert(Conv::Int) }
            }
            Op::Not => { let a = self.pop(); self.stack.push(a.negated()) }
            Op::Equals => self.binary(BinOp::Eq),
            Op::StrictEquals => self.binary(BinOp::StrictEq),
            Op::LessThan => self.binary(BinOp::Lt),
            Op::LessEquals => self.binary(BinOp::Le),
            Op::GreaterThan => self.binary(BinOp::Gt),
            Op::GreaterEquals => self.binary(BinOp::Ge),
            Op::In => self.binary(BinOp::In),
            Op::InstanceOf => self.binary(BinOp::InstanceOf),
            Op::IsType { index } => {
                let a = self.pop();
                self.stack.push(Expr::Binary(BinOp::Is, Box::new(a), Box::new(Expr::Name(self.pool.class_name(index.0)))));
            }
            Op::IsTypeLate => self.binary(BinOp::Is),
            Op::AsType { type_name } => {
                let a = self.pop();
                self.stack.push(Expr::Binary(BinOp::As, Box::new(a), Box::new(Expr::Name(self.pool.class_name(type_name.0)))));
            }
            Op::AsTypeLate => self.binary(BinOp::As),
            Op::TypeOf => self.unary(UnOp::TypeOf),

            // ---- conversions
            Op::ConvertI | Op::CoerceI => self.convert(Conv::Int),
            Op::ConvertU | Op::CoerceU => self.convert(Conv::Uint),
            Op::ConvertD | Op::CoerceD => self.convert(Conv::Number),
            Op::ConvertB | Op::CoerceB => self.convert(Conv::Boolean),
            Op::ConvertS => self.convert(Conv::String),
            // coerce_s keeps a null a null, where convert_s makes it "null"
            Op::CoerceS => { let a = self.pop(); self.stack.push(match a { Expr::Str(_) | Expr::Null => a, _ => Expr::Coerce("String".into(), Box::new(a)) }) }
            Op::ConvertO | Op::CoerceA | Op::CoerceO => {}
            Op::Coerce { index } => {
                let a = self.pop();
                let t = self.pool.class_name(index.0);
                self.stack.push(if a.is_stable() && !matches!(a, Expr::Temp(_) | Expr::Join { .. }) { a } else { Expr::Coerce(t, Box::new(a)) });
            }

            // ---- enumeration
            Op::HasNext2 { object_register, index_register } =>
                self.stack.push(Expr::HasNext2 { object: *object_register, index: *index_register }),
            Op::HasNext => { let ix = self.pop(); let o = self.pop(); self.stack.push(Expr::HasNext(Box::new(o), Box::new(ix))) }
            Op::NextName => { let ix = self.pop(); let o = self.pop(); self.stack.push(Expr::NextName(Box::new(o), Box::new(ix))) }
            Op::NextValue => { let ix = self.pop(); let o = self.pop(); self.stack.push(Expr::NextValue(Box::new(o), Box::new(ix))) }

            // ---- nothing to do
            Op::Label | Op::Nop | Op::Bkpt | Op::BkptLine { .. } | Op::Debug { .. } | Op::DebugFile { .. }
            | Op::DebugLine { .. } | Op::Timestamp | Op::Dxns { .. } | Op::CheckFilter => {}
            Op::DxnsLate => { let _ = self.pop(); }

            // ---- how a block ends
            Op::ReturnVoid => return Some(Term::Return(None)),
            Op::ReturnValue => { let v = self.pop(); return Some(Term::Return(Some(v))) }
            Op::Throw => { let v = self.pop(); return Some(Term::Throw(v)) }
            Op::Jump { offset } => return Some(Term::Goto(target(offset))),
            Op::IfTrue { offset } => { let c = self.pop(); return Some(Term::Branch(c, target(offset), i.next)) }
            Op::IfFalse { offset } => { let c = self.pop(); return Some(Term::Branch(c, i.next, target(offset))) }
            Op::IfEq { offset } | Op::IfNe { offset } | Op::IfLt { offset } | Op::IfLe { offset }
            | Op::IfGt { offset } | Op::IfGe { offset } | Op::IfStrictEq { offset } | Op::IfStrictNe { offset }
            | Op::IfNlt { offset } | Op::IfNle { offset } | Op::IfNgt { offset } | Op::IfNge { offset } => {
                let b = self.pop();
                let a = self.pop();
                let cmp = |op| Expr::Binary(op, Box::new(a.clone()), Box::new(b.clone()));
                let c = match i.op {
                    Op::IfEq { .. } => cmp(BinOp::Eq),
                    Op::IfNe { .. } => cmp(BinOp::Ne),
                    Op::IfLt { .. } => cmp(BinOp::Lt),
                    Op::IfLe { .. } => cmp(BinOp::Le),
                    Op::IfGt { .. } => cmp(BinOp::Gt),
                    Op::IfGe { .. } => cmp(BinOp::Ge),
                    Op::IfStrictEq { .. } => cmp(BinOp::StrictEq),
                    Op::IfStrictNe { .. } => cmp(BinOp::StrictNe),
                    // "not less than" is not "greater or equal" where a NaN is involved
                    Op::IfNlt { .. } => cmp(BinOp::Lt).negated(),
                    Op::IfNle { .. } => cmp(BinOp::Le).negated(),
                    Op::IfNgt { .. } => cmp(BinOp::Gt).negated(),
                    _ => cmp(BinOp::Ge).negated(),
                };
                return Some(Term::Branch(c, target(offset), i.next));
            }
            Op::LookupSwitch(ls) => {
                let ix = self.pop();
                let abs = |o: i32| (i.at as i64 + o as i64) as usize;
                return Some(Term::Switch(ix, ls.case_offsets.iter().map(|o| abs(*o)).collect(), abs(ls.default_offset)));
            }

            // ---- what content does not use: named, so a scan finds it
            other => {
                let what = format!("{other:?}");
                let name = what.split([' ', '{', '(']).next().unwrap_or("").to_string();
                self.stmts.push(Stmt::Unreadable(format!("instruction {name}")));
            }
        }
        None
    }
}

/// A method's signature, as the package declares it.
pub fn signature(pool: &Pool, m: &Method) -> (Vec<Param>, bool, Option<String>) {
    let count = m.params.len();
    let params = m.params.iter().enumerate().map(|(i, p)| Param {
        name: p.name.map(|n| pool.string(n.0)).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("arg{i}")),
        type_name: pool.type_name(p.kind.0),
        default: p.default_value.as_ref().map(|d| pool.default_value(d)),
    }).collect();
    let _ = count;
    let rest = m.flags.contains(swf::avm2::types::MethodFlags::NEED_REST);
    (params, rest, pool.type_name(m.return_type.0))
}

/// the register names a method's debug info keeps
pub fn local_names(pool: &Pool, method: u32) -> Vec<(u32, String)> {
    let m = &pool.abc.methods[method as usize];
    let Some(body) = m.body.and_then(|b| pool.abc.method_bodies.get(b.0 as usize)) else { return vec![] };
    let Ok(ins) = read_code(&body.code) else { return vec![] };
    let mut out: Vec<(u32, String)> = Vec::new();
    for i in ins {
        if let Op::Debug { is_local_register: true, register_name, register } = i.op {
            let n = pool.string(register_name.0);
            if !n.is_empty() && !out.iter().any(|(r, _)| *r == register as u32 + 1) {
                // debug registers count from the first parameter
                out.push((register as u32 + 1, n));
            }
        }
    }
    out
}

pub fn index<T>(i: u32) -> Index<T> { Index::new(i) }
