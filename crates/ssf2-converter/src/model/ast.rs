//! The tree a method decompiles to: AS3's meaning, nothing about either engine.
//!
//! Every node is a real node. Nothing here is text standing in for structure, and nothing names a
//! helper of the target language: the compiler decides how each node is written in Haxe.

use std::fmt::{self, Write as _};

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Int(i64),
    Str(String),
    This,
    /// a register (a parameter or a local variable)
    Local(u32),
    /// a value that reaches `block` on the operand stack, in slot `slot`: where a value is
    /// decided by a branch (`c ? a : b`, `a && b`) each way in assigns it and the block reads it
    Join { block: usize, slot: u32 },
    /// a value the decompiler keeps so it is computed once and in its place
    Temp(u32),
    /// a name looked up through the scope chain: a member of the object, a local captured by a
    /// closure, a class, a global function. which one is for whoever knows the scopes to say
    Name(String),
    Member(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    /// `callee(args)`; a method call is a call on a `Member`
    Call(Box<Expr>, Vec<Expr>),
    New(Box<Expr>, Vec<Expr>),
    SuperMember(String),
    SuperCall(String, Vec<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `a && b`: `a` when it is falsy, else `b`
    And(Box<Expr>, Box<Expr>),
    /// `a || b`: `a` when it is truthy, else `b`
    Or(Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    /// a conversion the bytecode states (`int(x)`, `String(x)`, ...)
    Convert(Conv, Box<Expr>),
    /// a value checked against a declared type on its way somewhere typed
    Coerce(String, Box<Expr>),
    Array(Vec<Expr>),
    Object(Vec<(Expr, Expr)>),
    Function(Box<Function>),
    /// an assignment whose value is used
    Assign(Box<Place>, Box<Expr>),
    Delete(Box<Place>),
    /// the enumeration step of a `for in`/`for each`: advances the index register past the next
    /// property of the object register and reports whether there was one
    HasNext2 { object: u32, index: u32 },
    NextName(Box<Expr>, Box<Expr>),
    NextValue(Box<Expr>, Box<Expr>),
    /// `hasnext obj, index`: the index after `index`, 0 at the end
    HasNext(Box<Expr>, Box<Expr>),
    /// the global object
    Global,
    /// the method's activation: where its variables live when a closure captures them
    Activation,
    /// an exception a catch clause receives
    Caught(u32),
    /// a class defined in place (a script's initializer defines its classes this way)
    Class(u32),
    /// the object on the scope chain that has `name` (what a lookup of `name` resolves against)
    ScopeOf(String),
    /// a generic type applied to its parameters (`Vector.<int>`)
    TypeApply(Box<Expr>, Vec<Expr>),
    /// something the decoder could not read, and why
    Unreadable(String),
    /// what code adds each time it runs as a share of a source frame, as it runs (the runtime's
    /// step): for code run both every frame and once
    Step,
    /// 1 the first time this write (by its site) runs on this object in a source frame, 0 after:
    /// a write code run twice a source frame makes only when something happens
    Once(Box<Expr>, String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp { Not, Neg, BitNot, TypeOf }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Shl, Shr, UShr, BitAnd, BitOr, BitXor,
    Eq, Ne, StrictEq, StrictNe, Lt, Le, Gt, Ge,
    In, InstanceOf, Is, As,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conv { Int, Uint, Number, String, Boolean, Object }

#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    Local(u32),
    Join { block: usize, slot: u32 },
    Temp(u32),
    Name(String),
    Member(Expr, String),
    Index(Expr, Expr),
    SuperMember(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Expr(Expr),
    Assign(Place, Expr),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    /// `while (true)`: the raw loop the structurer makes; tidying turns it into the loop it is
    Loop(Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    DoWhile(Vec<Stmt>, Expr),
    Break,
    Continue,
    Return(Option<Expr>),
    Throw(Expr),
    /// `switch`: the body for each set of case values (compared strictly), and the default
    Switch(Expr, Vec<(Vec<Expr>, Vec<Stmt>)>, Vec<Stmt>),
    Try(Vec<Stmt>, Vec<Catch>),
    Unreadable(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Catch {
    /// which exception this is in the method's table
    pub index: u32,
    pub var: Option<String>,
    pub type_name: Option<String>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub type_name: Option<String>,
    /// the declared default of an optional parameter
    pub default: Option<Expr>,
}

/// A function: a method's signature and its body.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub params: Vec<Param>,
    /// takes the rest of its arguments as an array
    pub rest: bool,
    pub return_type: Option<String>,
    /// the names of registers that hold named variables, where the package kept them
    pub local_names: Vec<(u32, String)>,
    /// the variables that live in the activation (captured by a closure), by slot
    pub activation: Vec<(u32, String)>,
    pub body: Vec<Stmt>,
}

// ---- walking ------------------------------------------------------------------------------------

impl Expr {
    /// every direct child, in evaluation order
    pub fn children(&self) -> Vec<&Expr> {
        use Expr::*;
        match self {
            Member(o, _) | Unary(_, o) | Convert(_, o) | Coerce(_, o) => vec![o],
            Index(a, b) | Binary(_, a, b) | And(a, b) | Or(a, b) | NextName(a, b) | NextValue(a, b)
            | HasNext(a, b) => vec![a, b],
            Call(c, args) | New(c, args) | TypeApply(c, args) => std::iter::once(&**c).chain(args.iter()).collect(),
            SuperCall(_, args) | Array(args) => args.iter().collect(),
            Cond(c, a, b) => vec![c, a, b],
            Object(kv) => kv.iter().flat_map(|(k, v)| [k, v]).collect(),
            Assign(p, v) => p.exprs().into_iter().chain(std::iter::once(&**v)).collect(),
            Delete(p) => p.exprs(),
            _ => vec![],
        }
    }

    pub fn walk(&self, f: &mut impl FnMut(&Expr)) {
        f(self);
        for c in self.children() { c.walk(f); }
    }

    pub fn map(self, f: &mut impl FnMut(Expr) -> Expr) -> Expr {
        use Expr::*;
        let b = |e: Box<Expr>, f: &mut dyn FnMut(Expr) -> Expr| Box::new(map_dyn(*e, f));
        let e = match self {
            Member(o, n) => Member(b(o, f), n),
            Unary(op, o) => Unary(op, b(o, f)),
            Convert(c, o) => Convert(c, b(o, f)),
            Coerce(t, o) => Coerce(t, b(o, f)),
            Index(x, y) => Index(b(x, f), b(y, f)),
            Binary(op, x, y) => Binary(op, b(x, f), b(y, f)),
            And(x, y) => And(b(x, f), b(y, f)),
            Or(x, y) => Or(b(x, f), b(y, f)),
            NextName(x, y) => NextName(b(x, f), b(y, f)),
            NextValue(x, y) => NextValue(b(x, f), b(y, f)),
            HasNext(x, y) => HasNext(b(x, f), b(y, f)),
            Call(c, a) => Call(b(c, f), a.into_iter().map(|x| map_dyn(x, f)).collect()),
            New(c, a) => New(b(c, f), a.into_iter().map(|x| map_dyn(x, f)).collect()),
            TypeApply(c, a) => TypeApply(b(c, f), a.into_iter().map(|x| map_dyn(x, f)).collect()),
            SuperCall(n, a) => SuperCall(n, a.into_iter().map(|x| map_dyn(x, f)).collect()),
            Array(a) => Array(a.into_iter().map(|x| map_dyn(x, f)).collect()),
            Cond(c, x, y) => Cond(b(c, f), b(x, f), b(y, f)),
            Object(kv) => Object(kv.into_iter().map(|(k, v)| (map_dyn(k, f), map_dyn(v, f))).collect()),
            Assign(p, v) => Assign(Box::new(p.map(f)), b(v, f)),
            Delete(p) => Delete(Box::new(p.map(f))),
            other => other,
        };
        f(e)
    }

    /// whether evaluating it can change anything or depends on something a change could reach
    pub fn is_stable(&self) -> bool {
        matches!(self, Expr::Undefined | Expr::Null | Expr::Bool(_) | Expr::Num(_) | Expr::Int(_)
            | Expr::Str(_) | Expr::This | Expr::Temp(_) | Expr::Join { .. } | Expr::Global
            | Expr::Activation | Expr::Class(_) | Expr::Function(_) | Expr::Caught(_))
    }

    /// whether evaluating it can have an effect (a call, an assignment, an enumeration step)
    pub fn has_effect(&self) -> bool {
        let mut any = false;
        self.walk(&mut |e| if matches!(e, Expr::Call(..) | Expr::New(..) | Expr::SuperCall(..)
            | Expr::Assign(..) | Expr::Delete(..) | Expr::HasNext2 { .. }) { any = true });
        any
    }

    pub fn mentions(&self, target: &Expr) -> usize {
        let mut n = 0;
        self.walk(&mut |e| if e == target { n += 1 });
        n
    }

    pub fn negated(self) -> Expr {
        match self {
            Expr::Unary(UnOp::Not, inner) => *inner,
            e => Expr::Unary(UnOp::Not, Box::new(e)),
        }
    }
}

fn map_dyn(e: Expr, f: &mut dyn FnMut(Expr) -> Expr) -> Expr {
    let mut g = |x: Expr| f(x);
    e.map(&mut g)
}

impl Place {
    pub fn exprs(&self) -> Vec<&Expr> {
        match self {
            Place::Member(o, _) => vec![o],
            Place::Index(o, k) => vec![o, k],
            _ => vec![],
        }
    }
    pub fn map(self, f: &mut impl FnMut(Expr) -> Expr) -> Place {
        match self {
            Place::Member(o, n) => Place::Member(o.map(f), n),
            Place::Index(o, k) => Place::Index(o.map(f), k.map(f)),
            p => p,
        }
    }
    /// the value this place reads as
    pub fn as_expr(&self) -> Expr {
        match self {
            Place::Local(r) => Expr::Local(*r),
            Place::Join { block, slot } => Expr::Join { block: *block, slot: *slot },
            Place::Temp(t) => Expr::Temp(*t),
            Place::Name(n) => Expr::Name(n.clone()),
            Place::Member(o, n) => Expr::Member(Box::new(o.clone()), n.clone()),
            Place::Index(o, k) => Expr::Index(Box::new(o.clone()), Box::new(k.clone())),
            Place::SuperMember(n) => Expr::SuperMember(n.clone()),
        }
    }
}

impl Stmt {
    /// every expression directly in this statement (not in nested statements)
    pub fn exprs(&self) -> Vec<&Expr> {
        match self {
            Stmt::Expr(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => vec![e],
            Stmt::Assign(p, e) => p.exprs().into_iter().chain(std::iter::once(e)).collect(),
            Stmt::If(c, _, _) | Stmt::While(c, _) | Stmt::DoWhile(_, c) | Stmt::Switch(c, _, _) => vec![c],
            _ => vec![],
        }
    }
    /// every nested statement list
    pub fn blocks(&self) -> Vec<&Vec<Stmt>> {
        match self {
            Stmt::If(_, a, b) => vec![a, b],
            Stmt::Loop(a) | Stmt::While(_, a) | Stmt::DoWhile(a, _) => vec![a],
            Stmt::Switch(_, cases, d) => cases.iter().map(|(_, b)| b).chain(std::iter::once(d)).collect(),
            Stmt::Try(b, cs) => std::iter::once(b).chain(cs.iter().map(|c| &c.body)).collect(),
            _ => vec![],
        }
    }
}

/// every expression anywhere in a statement list, nested ones included (closure bodies too)
pub fn walk_exprs(stmts: &[Stmt], f: &mut impl FnMut(&Expr)) {
    for s in stmts {
        if let Stmt::Unreadable(r) = s { f(&Expr::Unreadable(r.clone())) }
        for e in s.exprs() {
            e.walk(&mut |x| {
                f(x);
                if let Expr::Function(func) = x { walk_exprs_dyn(&func.body, f) }
            });
        }
        for b in s.blocks() { walk_exprs(b, f) }
    }
}

fn walk_exprs_dyn(stmts: &[Stmt], f: &mut dyn FnMut(&Expr)) {
    let mut g = |e: &Expr| f(e);
    walk_exprs(stmts, &mut g)
}

/// how many places in a body could not be read
pub fn unreadable(stmts: &[Stmt]) -> Vec<String> {
    let mut out = Vec::new();
    walk_exprs(stmts, &mut |e| if let Expr::Unreadable(r) = e { out.push(r.clone()) });
    out
}

// ---- printing, AS3-like, for reading what was decoded -------------------------------------------

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use BinOp::*;
        f.write_str(match self {
            Add => "+", Sub => "-", Mul => "*", Div => "/", Mod => "%", Shl => "<<", Shr => ">>",
            UShr => ">>>", BitAnd => "&", BitOr => "|", BitXor => "^", Eq => "==", Ne => "!=",
            StrictEq => "===", StrictNe => "!==", Lt => "<", Le => "<=", Gt => ">", Ge => ">=",
            In => "in", InstanceOf => "instanceof", Is => "is", As => "as",
        })
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use Expr::*;
        let list = |v: &[Expr]| v.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ");
        match self {
            Undefined => write!(f, "undefined"),
            Null => write!(f, "null"),
            Bool(b) => write!(f, "{b}"),
            Num(n) => if n.fract() == 0.0 && n.abs() < 1e15 { write!(f, "{}", *n as i64) } else { write!(f, "{n}") },
            Int(i) => write!(f, "{i}"),
            Str(s) => write!(f, "{s:?}"),
            This => write!(f, "this"),
            Local(r) => write!(f, "r{r}"),
            Join { block, slot } => write!(f, "$j{block}_{slot}"),
            Temp(t) => write!(f, "$t{t}"),
            Name(n) => write!(f, "{n}"),
            Member(o, n) => write!(f, "{}.{n}", paren(o)),
            Index(o, k) => write!(f, "{}[{k}]", paren(o)),
            Call(c, a) => write!(f, "{}({})", paren(c), list(a)),
            New(c, a) => write!(f, "new {}({})", paren(c), list(a)),
            SuperMember(n) => write!(f, "super.{n}"),
            SuperCall(n, a) if n.is_empty() => write!(f, "super({})", list(a)),
            SuperCall(n, a) => write!(f, "super.{n}({})", list(a)),
            Unary(op, e) => match op {
                UnOp::Not => write!(f, "!{}", paren(e)),
                UnOp::Neg => write!(f, "-{}", paren(e)),
                UnOp::BitNot => write!(f, "~{}", paren(e)),
                UnOp::TypeOf => write!(f, "typeof {}", paren(e)),
            },
            Binary(op, a, b) => write!(f, "{} {op} {}", paren(a), paren(b)),
            And(a, b) => write!(f, "{} && {}", paren(a), paren(b)),
            Or(a, b) => write!(f, "{} || {}", paren(a), paren(b)),
            Cond(c, a, b) => write!(f, "{} ? {} : {}", paren(c), paren(a), paren(b)),
            Convert(c, e) => write!(f, "{c:?}({e})"),
            Coerce(t, e) => write!(f, "({e} : {t})"),
            Array(a) => write!(f, "[{}]", list(a)),
            Object(kv) => write!(f, "{{{}}}", kv.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", ")),
            Function(func) => {
                let ps: Vec<String> = func.params.iter().map(|p| p.name.clone()).collect();
                let mut body = String::new();
                print_stmts(&mut body, &func.body, 1);
                write!(f, "function({}) {{\n{body}}}", ps.join(", "))
            }
            Assign(p, v) => write!(f, "({} = {v})", p.as_expr()),
            Delete(p) => write!(f, "delete {}", p.as_expr()),
            HasNext2 { object, index } => write!(f, "hasnext2(r{object}, r{index})"),
            NextName(o, i) => write!(f, "nextname({o}, {i})"),
            NextValue(o, i) => write!(f, "nextvalue({o}, {i})"),
            HasNext(o, i) => write!(f, "hasnext({o}, {i})"),
            Global => write!(f, "global"),
            Activation => write!(f, "activation"),
            Caught(i) => write!(f, "caught{i}"),
            Class(i) => write!(f, "class#{i}"),
            ScopeOf(n) => write!(f, "scopeOf({n})"),
            TypeApply(c, a) => write!(f, "{}.<{}>", paren(c), list(a)),
            Unreadable(r) => write!(f, "/*unreadable: {r}*/"),
            Step => write!(f, "step"),
            Once(o, site) => write!(f, "once({o}, {site})"),
        }
    }
}

fn paren(e: &Expr) -> String {
    match e {
        Expr::Binary(..) | Expr::And(..) | Expr::Or(..) | Expr::Cond(..) | Expr::Assign(..) => format!("({e})"),
        _ => e.to_string(),
    }
}

pub fn print_stmts(out: &mut String, stmts: &[Stmt], depth: usize) {
    let tab = "\t".repeat(depth);
    for s in stmts {
        match s {
            Stmt::Expr(e) => { let _ = writeln!(out, "{tab}{e};"); }
            Stmt::Assign(p, e) => { let _ = writeln!(out, "{tab}{} = {e};", p.as_expr()); }
            Stmt::If(c, a, b) => {
                let _ = writeln!(out, "{tab}if ({c}) {{");
                print_stmts(out, a, depth + 1);
                if !b.is_empty() {
                    let _ = writeln!(out, "{tab}}} else {{");
                    print_stmts(out, b, depth + 1);
                }
                let _ = writeln!(out, "{tab}}}");
            }
            Stmt::Loop(b) => { let _ = writeln!(out, "{tab}while (true) {{"); print_stmts(out, b, depth + 1); let _ = writeln!(out, "{tab}}}"); }
            Stmt::While(c, b) => { let _ = writeln!(out, "{tab}while ({c}) {{"); print_stmts(out, b, depth + 1); let _ = writeln!(out, "{tab}}}"); }
            Stmt::DoWhile(b, c) => { let _ = writeln!(out, "{tab}do {{"); print_stmts(out, b, depth + 1); let _ = writeln!(out, "{tab}}} while ({c});"); }
            Stmt::Break => { let _ = writeln!(out, "{tab}break;"); }
            Stmt::Continue => { let _ = writeln!(out, "{tab}continue;"); }
            Stmt::Return(None) => { let _ = writeln!(out, "{tab}return;"); }
            Stmt::Return(Some(e)) => { let _ = writeln!(out, "{tab}return {e};"); }
            Stmt::Throw(e) => { let _ = writeln!(out, "{tab}throw {e};"); }
            Stmt::Switch(i, cases, d) => {
                let _ = writeln!(out, "{tab}switch ({i}) {{");
                for (ks, b) in cases {
                    let _ = writeln!(out, "{tab}case {}:", ks.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(", case "));
                    print_stmts(out, b, depth + 1);
                }
                let _ = writeln!(out, "{tab}default:");
                print_stmts(out, d, depth + 1);
                let _ = writeln!(out, "{tab}}}");
            }
            Stmt::Try(b, cs) => {
                let _ = writeln!(out, "{tab}try {{");
                print_stmts(out, b, depth + 1);
                for c in cs {
                    let _ = writeln!(out, "{tab}}} catch ({}: {}) {{", c.var.as_deref().unwrap_or("e"), c.type_name.as_deref().unwrap_or("*"));
                    print_stmts(out, &c.body, depth + 1);
                }
                let _ = writeln!(out, "{tab}}}");
            }
            Stmt::Unreadable(r) => { let _ = writeln!(out, "{tab}/*unreadable: {r}*/"); }
        }
    }
}
