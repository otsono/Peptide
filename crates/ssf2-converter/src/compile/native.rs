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
    /// the source's names for the kinds of object (`getType()`), as this engine's
    types: BTreeMap<String, String>,
    /// a projectile stat written by its code, as the engine's own stat (`{v}` the value)
    projectile_stats: BTreeMap<String, String>,
    /// a projectile's states, by the source's names, as the engine's test (`{self}` the projectile)
    projectile_states: BTreeMap<String, String>,
    /// the API's records made with `new`, by class, and the calls they answer
    records: BTreeMap<String, String>,
    record_calls: BTreeMap<String, String>,
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
            if k == "types" {
                t.types = v.as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
                continue;
            }
            if k == "records" || k == "record_calls" {
                let m: BTreeMap<String, String> = v.as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
                if k == "records" { t.records = m } else { t.record_calls = m }
                continue;
            }
            if k == "projectile_states" {
                t.projectile_states = v.as_object().into_iter().flatten()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
                continue;
            }
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

/// a source fighter state as this engine's (`runtime/states.jsonc`): the engine states it is, and
/// the one a fighter goes to when code sets it
#[derive(Debug, Default)]
struct State { is: Vec<String>, enter: Option<String> }

fn states() -> &'static BTreeMap<String, State> {
    static T: OnceLock<BTreeMap<String, State>> = OnceLock::new();
    T.get_or_init(|| {
        let Some(dir) = super::runtime::Runtime::locate() else { return BTreeMap::new() };
        let Ok(text) = std::fs::read_to_string(dir.join("states.jsonc")) else { return BTreeMap::new() };
        let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text))
            .unwrap_or_else(|e| panic!("runtime/states.jsonc is malformed: {e}"));
        json.as_object().into_iter().flatten().map(|(k, v)| (k.clone(), State {
            is: v["is"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect(),
            enter: v["enter"].as_str().map(String::from),
        })).collect()
    })
}

/// the engine's test of a fighter (`who`) being in any of the source's states, by their numbers
pub fn state_tests(runtime: &super::runtime::Runtime, who: &str, numbers: &[i64]) -> String {
    let mut tests = Vec::new();
    for n in numbers {
        let Some(name) = runtime.constants.iter().find(|(k, v)| k.starts_with("PeptideCState.") && **v == n.to_string()).map(|(k, _)| k["PeptideCState.".len()..].to_string()) else { continue };
        for t in states().get(&name).into_iter().flat_map(|s| s.is.iter()) {
            let test = if t.starts_with("CStateGroup.") { format!("{who}.inStateGroup({t})") } else { format!("{who}.inState({t})") };
            if !tests.contains(&test) { tests.push(test); }
        }
    }
    if tests.is_empty() { "false".into() } else { tests.join(" || ") }
}

// ---- what a value is ----------------------------------------------------------------------------

/// what a value is, as far as a test of it needs to know: this engine passes a test only on a
/// literal `true`, where the source tests any value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ty { Bool, Num, Str, Object, Rect, Body, #[default] Any }

impl Ty {
    fn named(n: &str) -> Ty {
        match n { "bool" => Ty::Bool, "number" => Ty::Num, "string" => Ty::Str, "object" => Ty::Object, "rect" | "record" => Ty::Rect, _ => Ty::Any }
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
    /// the object's own class, where its package gives it one (`classAPI`, extending the API's
    /// projectile): its code runs as the object itself, with `this` the object
    pub itself: Option<&'a ClassModel>,
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
    /// its owner's animations and the label each shows (its clip's label, for a read of it)
    pub owner_anims: &'a [(String, String)],
    /// a projectile its owner's package declares, by name: its content, and its declared offset
    /// and launch (x, y offsets; x, y speeds), in the source's units
    pub fired: &'a dyn Fn(&str) -> Option<(String, [f64; 4])>,
    /// a stat its owner declares, as code
    pub owner_declares: &'a dyn Fn(&str) -> Option<String>,
    /// whether its owner's code keeps a field in a state container (its package's code stores it),
    /// for a read or write of the owner's field: the owner's script publishes its object, and keeps
    /// a stored field as `__c_<name>` on it while it is on the runtime
    pub owner_stores: &'a dyn Fn(&str) -> bool,
    /// the clip the object is (its root): a class of it has no clip of the object's above it
    pub root: u16,
    pub size: f64,
    pub speed: f64,
    pub accel: f64,
    /// whether it is a fighter (its own class extends the API's character)
    pub fighter: bool,
    /// a fighter's: the animation each of its clip's labels is (an attack its code starts by name)
    pub root_anim: &'a dyn Fn(&str) -> Option<String>,
    /// a fighter's: the walk's own starting speed, in this engine's units, where it declares one
    pub walk_start: Option<f64>,
    /// a fighter's: every animation that shows a label of a clip, by clip class (a jump its code
    /// asks of the clip it shows, whichever that is)
    pub stance_labels: &'a BTreeMap<String, Vec<(String, String, bool)>>,
    /// a fighter's: the clip frames each animation shows (its first, how many), by clip class
    pub clip_frames: &'a BTreeMap<String, Vec<(String, u32, u32)>>,
    /// a projectile's: whether its owner is written as Fraymakers code (its fields are then reached
    /// through what it publishes, `exports.__fieldOf`)
    pub owner_native: bool,
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
    /// its floor listeners: the container saying whether each listens now, and the function
    /// (`runtime/native/floor.hx` tells them of the floor the source's way)
    pub floor: Vec<(String, String)>,
    /// whether a floor listener reads its speed (it is told of the touch with the speed it came down at)
    pub floor_speed: bool,
    /// its listeners for its own end, the same way: told from the engine's `onKill`
    pub ends: Vec<(String, String)>,
    /// its listeners for meeting its owner, the same way: told by `runtime/native/touch.hx`
    pub touches: Vec<(String, String)>,
    /// what its own class runs as it starts (its constructor's work, then its `initialize`) and
    /// each of the source's frames (its `update`)
    pub start: Vec<String>,
    pub tick: Vec<String>,
    /// what it publishes for other objects' code: its jumps by label
    pub publish: Vec<String>,
    /// the blocks of `runtime/native/fighter_rules.hx` its code calls into
    pub blocks: BTreeSet<String>,
    /// the stats of its attacks its code reads by name (a fighter's `declared` answers them)
    pub stat_keys: BTreeSet<String>,
    /// the container each class's field is kept in, by (class, field), for the fields its code uses
    pub field_ids: BTreeMap<(String, String), String>,
    /// a clip's fields its frames keep as their own variables, by class: each one's declaration as
    /// the clip starts
    pub frame_fields: BTreeMap<String, Vec<String>>,
    /// the frames whose code is written straight into their keyframes (the rest call their function)
    pub inlined: BTreeSet<(u16, u32)>,
}

/// whether a method of an object's classes is ever run: a frame script, one of the API's hooks, or
/// named anywhere in the package's code (a call, a listener, a timer). one never named is a
/// template's leftover, and is not written
fn reached(model: &SourceModel, c: &ClassModel, m: &crate::model::MethodModel) -> bool {
    if c.frame_scripts.iter().any(|fs| fs.method == m.name) { return true }
    if matches!(m.name.as_str(), "initialize" | "update") { return true }
    static NAMED: OnceLock<std::sync::Mutex<BTreeMap<usize, BTreeSet<String>>>> = OnceLock::new();
    let key = model as *const SourceModel as usize;
    let mut cache = NAMED.get_or_init(Default::default).lock().unwrap();
    let names = cache.entry(key).or_insert_with(|| {
        let mut out = BTreeSet::new();
        for k in model.classes.values() {
            for f in k.methods.iter().map(|m| &m.function).chain([&k.constructor, &k.static_init]) {
                crate::model::ast::walk_exprs(&f.body, &mut |e| match e {
                    Expr::Member(_, n) | Expr::Name(n) => { out.insert(n.clone()); }
                    Expr::Str(s) => { out.insert(s.clone()); }
                    _ => {}
                });
            }
        }
        out
    });
    names.contains(&m.name)
}

/// Write the object's classes as Fraymakers code, or say why they cannot be.
pub fn write(o: &Object) -> Result<Written, String> {
    let mut w = Writer::new(o)?;
    w.learn_fields();
    w.write()
}

/// Every reason the object's code is not written yet, one per function it has no plain form for
/// (`class.method: why`): the work list for taking it off the runtime.
pub fn census(o: &Object) -> Vec<String> {
    let mut w = match Writer::new(o) { Ok(w) => w, Err(e) => return vec![e] };
    w.learn_fields();
    let mut out = Vec::new();
    let classes: Vec<&ClassModel> = o.itself.into_iter().chain(o.classes.iter().copied()).collect();
    for c in classes {
        w.class = c.name.clone();
        for m in c.methods.iter().filter(|m| !declares(m) && (!o.fighter || reached(o.model, c, m))) {
            let name = w.methods[&member_key(&c.name, &method_name(m))].clone();
            w.in_frame = c.frame_scripts.iter().any(|fs| fs.method == m.name);
            if let Err(e) = w.function(&name, &m.function) { out.push(format!("{}.{}: {e}", short(&c.name), m.name)); }
        }
    }
    out
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
    /// the stats its code writes, by name, that this engine has none of: kept for its code to read back
    own_stats: BTreeSet<String>,
    /// its floor listeners, by function
    floor: BTreeMap<String, String>,
    /// whether one of them reads its speed
    floor_speed: bool,
    /// its listeners for its own end, by function
    ends: BTreeMap<String, String>,
    /// its listeners for meeting its owner, by function
    touches: BTreeMap<String, String>,
    /// the reads that are its owner (`self.getOwner()`, a field only ever set to it)
    owners: BTreeSet<String>,
    /// the blocks of `runtime/native/fighter_rules.hx` its code calls into
    blocks: BTreeSet<String>,
    /// the attack stats its code reads by name
    stat_keys: BTreeSet<String>,
    /// the helper that jumps its clip to a frame by number, written once a jump by number asks for it
    jump_by_number: Option<String>,
    /// the object an effect being written is made on (`self`, or one it keeps)
    effect_on: String,
    /// whether the value being written is a test (an `&&` or `||` is then the two tests)
    testing: bool,
    /// whether the function being written is a frame script
    in_frame: bool,
    /// the variables the function being written shares with the closures it makes
    captured: BTreeSet<String>,
    /// what each of them holds, as far as its code says
    captured_ty: BTreeMap<String, Ty>,
    /// the first pass, learning what each field holds, and what it has seen put in each
    learning: bool,
    seen: BTreeMap<String, Ty>,
    /// whether the fields' types were learned (and stay as learned)
    learned: bool,
    /// the fields kept as plain variables of the frames' own code (a clip's), by member key
    plain: BTreeSet<String>,
    /// every name the script itself defines, which a frame's own variables keep clear of
    globals: BTreeSet<String>,
    /// the next function written is a frame's code, written straight into its keyframe
    inline: bool,
    /// the function being written is that
    frame_top: bool,
}

impl<'o, 'a> Writer<'o, 'a> {
    fn new(o: &'o Object<'a>) -> Result<Self, String> {
        let mut w = Writer { o, fields: BTreeMap::new(), methods: BTreeMap::new(), class: String::new(),
                             locals: BTreeMap::new(), helpers: BTreeMap::new(), kept: BTreeMap::new(), read: BTreeSet::new(), top: false, effects: 0, own_stats: BTreeSet::new(), floor: BTreeMap::new(), floor_speed: false, ends: BTreeMap::new(), touches: BTreeMap::new(), owners: ["self.getOwner()".to_string()].into(), blocks: BTreeSet::new(), stat_keys: BTreeSet::new(), jump_by_number: None, effect_on: "self".into(), testing: false, in_frame: false, captured: BTreeSet::new(), captured_ty: BTreeMap::new(), learning: false, seen: BTreeMap::new(), learned: false, plain: BTreeSet::new(), globals: BTreeSet::new(), inline: false, frame_top: false };
        // every class's names, each kept as it is unless two classes share one
        let mut taken: BTreeSet<String> = ["self", "match", "stage", "initialize", "update", "onTeardown", "onKill", "life", "age", "started", "freshHit"]
            .into_iter().map(String::from).collect();
        // (and every name the native files define: the script's own parts and the rules' blocks)
        let defined = crate::re!(r"(?m)^(?:function|var) ([A-Za-z_]\w*)");
        for text in o.runtime.native_files.values() {
            for c in defined.captures_iter(text) { taken.insert(c[1].to_string()); }
        }
        let name_for = |n: &str, class: &ClassModel, taken: &mut BTreeSet<String>| {
            let mut id = ident(n);
            if !taken.insert(id.clone()) { id = format!("{}_{}", ident(short(&class.name)), id); taken.insert(id.clone()); }
            id
        };
        let all: Vec<&ClassModel> = o.itself.into_iter().chain(o.classes.iter().copied()).collect();
        for c in &all {
            let own = o.itself.is_some_and(|i| i.name == c.name);
            let base_ok = if own { if o.fighter { matches!(short(&c.super_name), "PeptideCharacter" | "PeptideCharacterExt") } else { short(&c.super_name) == "PeptideProjectile" } } else { matches!(short(&c.super_name), "MovieClip" | "Sprite") };
            if !base_ok { return Err(format!("{} extends {}, whose code is not written here", c.name, c.super_name)) }
            if !c.unreadable.is_empty() { return Err(format!("{} has code that could not be read", c.name)) }
            if c.fields.iter().any(|f| f.is_static) || c.methods.iter().any(|m| m.is_static) { return Err(format!("{} has statics", c.name)) }
            // the constructor only registers the frame scripts (the object's own class's sets its fields, written as its start)
            for s in c.constructor.body.iter().filter(|_| !own) {
                let fine = match s {
                    Stmt::Return(None) | Stmt::Expr(Expr::SuperCall(..)) => true,
                    Stmt::Expr(Expr::Call(f, _)) => matches!(&**f, Expr::Member(t, m) if matches!(**t, Expr::This) && m == "addFrameScript")
                        || matches!(&**f, Expr::Name(m) if m == "addFrameScript"),
                    _ => false,
                };
                if !fine { return Err(format!("{}'s constructor does more than register its frames", c.name)) }
            }
            for m in c.methods.iter().filter(|m| !declares(m)) { let id = name_for(&accessor_name(m), c, &mut taken); w.methods.insert(member_key(&c.name, &method_name(m)), id); }
            // whether a floor listener of its reads its speed (it is then told the speed it came down at)
            for m in &c.methods {
                crate::model::ast::walk_exprs(&m.function.body, &mut |e| if let Expr::Call(f, a) = e {
                    if !matches!(&**f, Expr::Member(_, n) if n == "addEventListener") { return }
                    let [ev, h, ..] = a.as_slice() else { return };
                    let floor = super::events::named(o.runtime, ev).and_then(|n| super::events::table().engine.get(&n).cloned()).is_some_and(|t| t.contains("COLLIDE_FLOOR"));
                    if !floor { return }
                    let body = match h {
                        Expr::Member(t, n) if matches!(**t, Expr::This) => c.methods.iter().find(|x| &x.name == n).map(|x| &x.function),
                        Expr::Name(n) => c.methods.iter().find(|x| &x.name == n).map(|x| &x.function),
                        Expr::Function(f) => Some(&**f),
                        _ => None,
                    };
                    if let Some(b) = body {
                        crate::model::ast::walk_exprs(&b.body, &mut |x| if let Expr::Member(_, n) = x { if matches!(n.as_str(), "getXSpeed" | "getYSpeed") { w.floor_speed = true } });
                    }
                });
            }
            // the stats its code writes that are neither the engine's nor a switch
            for m in &c.methods {
                crate::model::ast::walk_exprs(&m.function.body, &mut |e| if let Expr::Call(f, a) = e {
                    if matches!(&**f, Expr::Member(_, n) if n == "updateProjectileStats") {
                        if let [Expr::Object(kv)] = a.as_slice() { for (k, _) in kv { if let Expr::Str(k) = k {
                            if !calls().projectile_stats.contains_key(k) && !o.switches.contains_key(k) { w.own_stats.insert(k.clone()); }
                        } } }
                    }
                });
            }
            for f in &c.fields {
                let holds = if w.holds_itself(c, &f.name) { Holds::Itself } else { Holds::Value(Ty::declared(f.type_name.as_deref())) };
                let id = if holds == Holds::Itself { "self".to_string() } else { name_for(&f.name, c, &mut taken) };
                // (a fighter's clip's field is a variable of the clip's own frame code: the frames and
                // the script share one scope, and the engine rolls those back too)
                let own = o.itself.is_some_and(|i| i.name == c.name);
                let plain = o.fighter && !own && holds != Holds::Itself && c.symbol.is_some_and(|s| s != o.root);
                if plain { w.plain.insert(member_key(&c.name, &f.name)); }
                if w.holds_owner(c, &f.name) { w.owners.insert(if plain { id.clone() } else { format!("{id}.get()") }); }
                w.fields.insert(member_key(&c.name, &f.name), (id, holds));
            }
        }
        w.globals = taken;
        Ok(w)
    }

    /// what each field holds, learned from all the code before any of it is written (a method
    /// written before the frame that fills a field reads it right): a pass whose text is dropped
    fn learn_fields(&mut self) {
        let fresh = Writer::new(self.o).ok();
        self.learning = true;
        let classes: Vec<&ClassModel> = self.o.itself.into_iter().chain(self.o.classes.iter().copied()).collect();
        for c in classes {
            self.class = c.name.clone();
            for m in c.methods.iter().filter(|m| !declares(m)) {
                let name = self.methods[&member_key(&c.name, &method_name(m))].clone();
                self.in_frame = c.frame_scripts.iter().any(|fs| fs.method == m.name);
                let _ = self.function(&name, &m.function);
            }
        }
        // (everything else it gathered starts over)
        if let Some(mut f) = fresh {
            for (k, ty) in &self.seen {
                if let Some(e) = f.fields.get_mut(k) {
                    // (a typed one keeps its type: the source makes whatever goes in one of those, unless
                    // that is an object, as a sound kept where a number was)
                    if let Holds::Value(declared) = e.1 {
                        e.1 = Holds::Value(match (declared, *ty) {
                            (Ty::Any, t) => t,
                            (d, Ty::Any) => d,
                            (d, t) => join(d, t),
                        });
                    }
                }
            }
            f.learned = true;
            *self = f;
        }
    }

    /// whether every value the class's code puts in a field is the object's owner
    fn holds_owner(&self, c: &ClassModel, field: &str) -> bool {
        let mut any = false;
        let mut all = true;
        let fields: BTreeSet<String> = c.fields.iter().filter(|f| self.holds_itself(c, &f.name)).map(|f| f.name.clone()).collect();
        let me = |e: &Expr| matches!(e, Expr::This) || matches!(e, Expr::Member(t, f) if matches!(**t, Expr::This) && fields.contains(f)) || is_itself(e);
        for m in &c.methods {
            walk(&m.function.body, &mut |s| if let Stmt::Assign(Place::Member(t, f), v) = s {
                if matches!(t, Expr::This) && f == field {
                    any = true;
                    all &= matches!(v, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(o, n) if n == "getOwner" && me(o)));
                }
            });
        }
        any && all
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
        let classes: Vec<&ClassModel> = self.o.itself.into_iter().chain(self.o.classes.iter().copied()).collect();
        let mut inlined: Vec<((u16, u32), String, String)> = Vec::new();
        // the object's own class: its constructor's work and its hooks
        if let Some(c) = self.o.itself {
            self.class = c.name.clone();
            let mut body: Vec<Stmt> = c.constructor.body.iter().filter(|s| !matches!(s, Stmt::Expr(Expr::SuperCall(..)))).cloned().collect();
            if matches!(body.last(), Some(Stmt::Return(None))) { body.pop(); }
            if !body.is_empty() {
                let f = Function { body, ..c.constructor.clone() };
                let text = self.function("construct", &f).map_err(|e| format!("{}'s constructor: {e}", short(&c.name)))?;
                out.functions.push(format!("// what its class sets up as it is made\n{text}"));
                out.start.push("construct();".into());
            }
            // the lifetime its class's own stats work out (`getOwnStats`'s `time_max`)
            let own_stats = c.methods.iter().find(|m| declares(m) && m.name == "getOwnStats");
            if let Some(Stmt::Return(Some(Expr::Object(kv)))) = own_stats.and_then(|m| m.function.body.first()) {
                if let Some((_, v)) = kv.iter().find(|(k, _)| matches!(k, Expr::Str(k) if k == "time_max")) {
                    self.locals.clear();
                    let (t, _) = self.expr(v)?;
                    out.start.insert(0, format!("life.set(Std.int({} * 2));", paren(&t)));
                }
            }
            if let Some(id) = self.method("initialize") { out.start.push(format!("{id}();")); }
            if let Some(id) = self.method("update") { out.tick.push(format!("{id}();")); }
        }
        for c in &classes {
            self.class = c.name.clone();
            let frame_of: BTreeMap<&str, u32> = c.frame_scripts.iter().map(|fs| (fs.method.as_str(), fs.frame)).collect();
            for m in c.methods.iter().filter(|m| !declares(m) && (!self.o.fighter || reached(self.o.model, c, m))) {
                let name = self.methods[&member_key(&c.name, &method_name(m))].clone();
                self.in_frame = frame_of.contains_key(m.name.as_str());
                // (a fighter's frame's code is its keyframe's: written there as it is, unless other code
                // calls it by name too)
                let at = frame_of.get(m.name.as_str()).copied().zip(c.symbol);
                self.inline = self.o.fighter && at.is_some();
                let text = self.function(&name, &m.function).map_err(|e| format!("{}.{}: {e}", short(&c.name), m.name));
                self.inline = false;
                let text = text?;
                if let (Some((f, sym)), true) = (at, self.o.fighter) {
                    inlined.push(((sym, f + 1), name.clone(), text));
                    continue;
                }
                out.functions.push(text);
                if let Some((f, sym)) = at {
                    out.frames.insert((sym, f + 1), format!("{name}();"));
                }
            }
        }
        // the fields its code uses, as the engine's containers
        for (key, (id, holds)) in &self.fields {
            let Holds::Value(ty) = holds else { continue };
            if !self.read.contains(key) { continue }
            let (class, field) = key.split_once("::").unwrap_or(("", key));
            let c = classes.iter().filter(|c| c.name == class).find_map(|c| c.fields.iter().find(|f| f.name == field));
            let default = c.and_then(|f| f.default.as_ref()).and_then(literal);
            let whole = c.is_some_and(|f| matches!(f.type_name.as_deref().map(short), Some("int" | "uint")));
            // (a fighter's true-or-false and text fields are object containers: the engine keeps
            // few of the others for a whole match, and two fighters running the same script ran out)
            let make = match ty {
                Ty::Num if whole => format!("self.makeInt({})", default.clone().unwrap_or_else(|| "0".into())),
                Ty::Num => format!("self.makeFloat({})", default.clone().unwrap_or_else(|| "0".into())),
                Ty::Bool if !self.o.fighter => format!("self.makeBool({})", default.clone().unwrap_or_else(|| "false".into())),
                Ty::Bool => format!("self.makeObject({})", default.clone().unwrap_or_else(|| "false".into())),
                Ty::Str if !self.o.fighter => format!("self.makeString({})", default.clone().unwrap_or_else(|| "null".into())),
                _ => format!("self.makeObject({})", default.clone().unwrap_or_else(|| "null".into())),
            };
            if self.plain.contains(key) {
                // (a clip's: declared by its own frame code as the clip starts; another object reads it
                // through a getter, the scope being the script's)
                let start = default.clone().unwrap_or_else(|| match ty { Ty::Num => "0".into(), Ty::Bool => "false".into(), _ => "null".into() });
                out.frame_fields.entry(class.to_string()).or_default().push(format!("var {id} = {start};"));
                out.field_ids.insert((class.to_string(), field.to_string()), format!("{{ get: function() {{ return {id}; }}, set: function(v) {{ {id} = v; }} }}"));
                continue;
            }
            out.fields.push(format!("var {id} = {make};"));
            out.field_ids.insert((class.to_string(), field.to_string()), id.clone());
        }
        out.fields.extend(self.kept.values().cloned());
        out.floor = self.floor.iter().map(|(h, l)| (l.clone(), h.clone())).collect();
        out.floor_speed = self.floor_speed;
        out.ends = self.ends.iter().map(|(h, l)| (l.clone(), h.clone())).collect();
        out.touches = self.touches.iter().map(|(h, l)| (l.clone(), h.clone())).collect();
        // its jumps by label, for another object's code to ask of it
        let mut cases = Vec::new();
        for c in self.o.classes.iter().filter(|_| !self.o.fighter) {
            self.class = c.name.clone();
            let labels: Vec<String> = self.o.jumps.get(&c.name).map(|j| j.labels.keys().cloned().collect()).unwrap_or_default();
            for l in labels {
                let code = self.jump(&l)?;
                cases.push(format!("case {}:", quote(&l)));
                cases.extend(code.lines().map(|x| format!("\t{x}")));
            }
        }
        if self.o.fighter {
            // (a fighter's: on the clip it shows, whichever that is)
            if !self.o.stance_labels.is_empty() {
                self.class = String::new();
                self.stance_jump("")?;
                out.publish.push("self.exports.__playFrame = stanceJump;".into());
            }
        } else if !cases.is_empty() {
            out.functions.push(helper(self.o.runtime, "helpers", "jump_to", "jumpTo", &[], &cases)?);
            out.publish.push("self.exports.__playFrame = jumpTo;".into());
        }
        out.functions.extend(self.helpers.values().cloned());
        // (a frame other code calls by name stays a function its keyframe calls)
        let called = |name: &str, out: &Written, inlined: &[((u16, u32), String, String)]| {
            let re = regex::Regex::new(&format!(r"\b{}\(", regex::escape(name))).unwrap();
            out.functions.iter().chain(out.start.iter()).chain(out.tick.iter()).any(|t| re.is_match(t))
                || inlined.iter().any(|(_, n, t)| n != name && re.is_match(t))
        };
        for (at, name, text) in &inlined {
            if called(name, &out, &inlined) {
                out.functions.push(text.replace(FRAME_RETURN, "return;"));
                out.frames.insert(*at, format!("{name}();"));
            } else {
                // (a frame with no code of its own keeps no keyframe code)
                let body = frame_body(text);
                if body.trim().is_empty() { continue }
                out.frames.insert(*at, body);
                out.inlined.insert(*at);
            }
        }
        out.blocks = self.blocks.clone();
        out.stat_keys = self.stat_keys.clone();
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
        // (an argument read as a plain value is one a call hands it, as any variable)
        let _ = (reads, through);
        // the variables a closure of its captures: plain variables of the function, which a
        // closure written in place sees (the interpreter's closures keep their maker's variables)
        let captured: Vec<String> = f.activation.iter().map(|(_, n)| ident(n)).collect();
        // (a closure written in place sees its maker's too)
        let mut seen: BTreeSet<String> = if name.is_empty() { self.captured.clone() } else { BTreeSet::new() };
        seen.extend(captured.iter().cloned());
        let outer = std::mem::replace(&mut self.captured, seen);
        // (a frame's code written into its keyframe; a function it makes is a function again)
        let top = std::mem::replace(&mut self.frame_top, std::mem::take(&mut self.inline));
        let r = self.function_body(name, f, &params, nparams, &debug, &captured);
        self.frame_top = top;
        self.captured = outer;
        r
    }

    fn function_body(&mut self, name: &str, f: &Function, params: &[String], nparams: u32, debug: &BTreeMap<u32, String>, captured: &[String]) -> Result<String, String> {
        // (the decompiler's leftovers tidied first)
        let mut body = f.body.clone();
        tidy(&mut body);
        records(&mut body);
        // its variables, at the top, as the source scopes them to the function (not a copy of its
        // clip, which reads as its own fields)
        let mut regs = BTreeSet::new();
        walk_places(&body, &mut |r| { if r > nparams { regs.insert(r); } });
        let mut copies = BTreeSet::new();
        walk(&body, &mut |s| if let Stmt::Assign(Place::Local(r), v) = s { if matches!(v, Expr::This) { copies.insert(*r); } });
        walk(&body, &mut |s| if let Stmt::Assign(Place::Local(r), v) = s { if !matches!(v, Expr::This) { copies.remove(r); } });
        if self.frame_top { if let Some(n) = captured.iter().find(|n| self.globals.contains(*n)) { return Err(format!("a frame's variable {n}, which its closures share, is named like one of the script's")) } }
        let mut decl: Vec<String> = captured.iter().map(|n| format!("\tvar {n} = null;")).collect();
        // (the decompiler's own temporaries, as variables of the function)
        let mut temps = BTreeSet::new();
        walk(&body, &mut |s| if let Stmt::Assign(Place::Temp(t), _) = s { temps.insert(*t); });
        walk_exprs(&body, &mut |e| if let Expr::Temp(t) = e { temps.insert(*t); });
        for t in &temps { decl.push(format!("\tvar __t{t} = null;")); }
        // (a frame's own variables, in the script's scope, keep clear of the names the script defines)
        let clear = |n: String, w: &Self| if w.frame_top && w.globals.contains(&n) { format!("{name}_{n}") } else { n };
        for r in &regs {
            if copies.contains(r) {
                let n = clear(debug.get(r).map(|s| ident(s)).unwrap_or_else(|| format!("v{r}")), self);
                self.locals.insert(*r, (n, Holds::Clip));
                continue;
            }
            let n = clear(debug.get(r).map(|s| ident(s)).unwrap_or_else(|| format!("v{r}")), self);
            decl.push(format!("\tvar {n} = null;"));
            self.locals.insert(*r, (n, Holds::Value(Ty::Any)));
        }
        // (a body's closing `return;` says nothing)
        if matches!(body.last(), Some(Stmt::Return(None))) { body.pop(); }
        let mut lines = decl;
        for s in &body {
            match self.stmt(s, 1) {
                Ok(l) => lines.extend(l),
                Err(e) => {
                    if std::env::var("PEPTIDE_NATIVE_WHY").is_ok() { eprintln!("{name}: {e}\n  at {s:?}"); }
                    return Err(e);
                }
            }
        }
        let head = if name.is_empty() { "function".to_string() } else { format!("function {name}") };
        Ok(format!("{head}({}) {{\n{}\n}}", params.join(", "), lines.join("\n")).replace("{\n\n}", "{\n}"))
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
            Stmt::Return(None) => vec![format!("{tab}{}", if self.frame_top { FRAME_RETURN } else { "return;" })],
            Stmt::Return(Some(e)) => vec![format!("{tab}return {};", self.expr(e)?.0)],
            Stmt::Switch(v, cases, default) => {
                let mut out = vec![format!("{tab}switch ({}) {{", self.expr(v)?.0)];
                for (i, (values, _)) in cases.iter().enumerate() {
                    let vs: Result<Vec<String>, String> = values.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                    out.push(format!("{tab}\tcase {}:", vs?.join(", ")));
                    // (a case that runs on into the next has the next's body after its own, up to a break)
                    let mut body: Vec<Stmt> = Vec::new();
                    for (_, b) in &cases[i..] {
                        body.extend(b.iter().cloned());
                        if matches!(b.last(), Some(Stmt::Break | Stmt::Return(_))) { break }
                    }
                    if !matches!(body.last(), Some(Stmt::Break | Stmt::Return(_))) { body.extend(default.iter().cloned()); }
                    out.extend(self.block(strip_break(&body)?, depth + 2)?);
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
                Expr::Name(m) if self.method(m).is_none() => Some(m),
                _ => None,
            };
            if let Some(m) = own {
                {
                    match (m.as_str(), args.as_slice()) {
                        // the engine plays the clip the object shows on every frame, whatever a stop on
                        // it says (the runtime does the same: Projectiles.beforeTick)
                        ("stop" | "play", []) => return Ok(String::new()),
                        ("gotoAndStop" | "gotoAndPlay", [Expr::Str(label)]) => return self.jump(label),
                        ("gotoAndStop" | "gotoAndPlay", [label]) if self.o.fighter => return Ok(format!("{};", self.jump_to_value(label)?)),
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
            // the clip it shows hidden or shown: the object itself, as it is the object's look
            Place::Member(Expr::This, f) if f == "visible" && self.field(f).is_none() => {
                let t = self.test(v)?;
                Ok(vec![format!("self.setVisible({t});")])
            }
            // the clip it shows scaled from inside (its parent, the object's own clip): the object itself
            Place::Member(Expr::Name(p), f) if p == "parent" && matches!(f.as_str(), "scaleX" | "scaleY")
                && self.o.model.classes.get(&self.class).and_then(|c| c.symbol) != Some(self.o.root) => {
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("self.set{}({val});", if f == "scaleX" { "ScaleX" } else { "ScaleY" })])
            }
            // a field of its owner's clip
            Place::Member(mc, f) if self.owner_clip(mc).is_some() => {
                let (who, clip) = self.owner_clip(mc).unwrap_or_default();
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{}({who}, {}, {}, {val});", self.owner_field("set")?, quote(&clip), quote(f))])
            }
            // a field of its owner's
            Place::Member(o, f) if !matches!(o, Expr::This | Expr::Call(..)) && { let w = self.expr(o).map(|x| x.0).unwrap_or_default(); self.owners.contains(&w) } => {
                let (who, _) = self.expr(o)?;
                let (val, _) = self.expr(v)?;
                if self.o.owner_native { return Ok(vec![format!("{}({who}, \"self\", {}, {val});", self.owner_field("set")?, quote(f))]) }
                Ok(vec![if (self.o.owner_stores)(f) { format!("{who}.exports.__source.__c_{}.set({val});", ident(f)) } else { format!("{who}.exports.__source.{} = {val};", ident(f)) }])
            }
            Place::Member(Expr::This, f) => {
                if self.field(f).is_none() {
                    if let Some(set) = self.method(&format!("set:{f}")) { let (val, _) = self.expr(v)?; return Ok(vec![format!("{set}({val});")]) }
                }
                let (id, holds) = self.field(f).ok_or_else(|| format!("writes {f}, which is not its field"))?;
                if holds == Holds::Itself { return Ok(vec![]) }
                let (val, ty) = self.expr(v)?;
                // what it learns of the field's value
                // (an object put in a field declared a number is an object's: a sound it played, kept to stop it)
                let key = member_key(&self.class, f);
                // (a rectangle put in a field that held nothing before reads as one)
                // (learning: every kind of value the code puts in it, joined)
                if self.learning {
                    let was = self.seen.get(&key).copied();
                    self.seen.insert(key.clone(), match was { None => ty, Some(w) => join(w, ty) });
                }
                // (it never goes back: a sound kept in a field the code later zeroes is still an object's)
                if let Some(e) = self.fields.get_mut(&key).filter(|_| !self.learned) {
                    if let Holds::Value(had) = e.1 {
                        let now = match (had, ty) {
                            (Ty::Any, t) => t,
                            (Ty::Num | Ty::Bool | Ty::Str, Ty::Object | Ty::Rect | Ty::Body) => ty,
                            (Ty::Object, Ty::Rect | Ty::Body) => ty,
                            _ => had,
                        };
                        e.1 = Holds::Value(now);
                    }
                }
                let plain = self.plain.contains(&key);
                self.read.insert(key);
                Ok(vec![if plain { format!("{id} = {val};") } else { format!("{id}.set({val});") }])
            }
            // (a copy of its clip, as the decompiler keeps one for `this.n++`: its reads are its own)
            Place::Local(r) if matches!(v, Expr::This) => {
                let (n, _) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                self.locals.insert(*r, (n, Holds::Clip));
                Ok(vec![])
            }
            Place::Member(Expr::Local(r), f) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Clip) => self.assign(&Place::Member(Expr::This, f.clone()), v),
            // the look of an effect or object it keeps
            Place::Member(o, f) if !matches!(o, Expr::This) && matches!(f.as_str(), "x" | "y") && !self.is_clip(o) && !matches!(o, Expr::Name(n) if n == "parent") => {
                let (who, _) = self.expr(o)?;
                if who == "self" { return Err(format!("writes its clip's {f}")) }
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{who}.set{}({});", f.to_uppercase(), fold(&format!("{} * {}", paren(&val), number(self.o.size))))])
            }
            Place::Member(o, f) if !matches!(o, Expr::This) && matches!(f.as_str(), "scaleX" | "scaleY" | "alpha" | "rotation" | "visible") => {
                let (who, _) = self.expr(o)?;
                let (val, _) = if f == "visible" { (self.test(v)?, Ty::Bool) } else { self.expr(v)? };
                let set = match f.as_str() { "scaleX" => "setScaleX", "scaleY" => "setScaleY", "alpha" => "setAlpha", "rotation" => "setRotation", _ => "setVisible" };
                Ok(vec![format!("{who}.{set}({val});")])
            }
            // a field of a record it keeps
            Place::Member(o, f) if self.a_value(o) && !matches!(o, Expr::Local(_)) && self.expr(o).is_ok_and(|(_, t)| t == Ty::Rect) => {
                let (who, _) = self.expr(o)?;
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{who}.{f} = {val};")])
            }
            Place::Member(Expr::Local(r), f) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Value(Ty::Rect)) => {
                let n = self.locals.get(r).map(|x| x.0.clone()).unwrap_or_default();
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{n}.{f} = {val};")])
            }
            // a field of its own class, written through the fighter from one of its clips
            Place::Member(o, f) if self.o.fighter && !self.is_itself() && !matches!(o, Expr::This) && self.expr(o).is_ok_and(|(w, _)| w == "self")
                && self.o.itself.is_some_and(|c| self.fields.contains_key(&member_key(&c.name, f)) || self.methods.contains_key(&member_key(&c.name, &format!("set:{f}")))) => {
                let saved = std::mem::replace(&mut self.class, self.o.itself.map(|c| c.name.clone()).unwrap_or_default());
                let r = self.assign(&Place::Member(Expr::This, f.clone()), v);
                self.class = saved;
                r
            }
            Place::Member(o, f) if self.o.fighter && !matches!(o, Expr::This) && !self.is_clip(o) && !matches!(o, Expr::Name(_)) => {
                let (who, _) = self.expr(o)?;
                if who == "self" { return Err(format!("writes {f} on itself")) }
                let (val, _) = self.expr(v)?;
                let set = if (self.o.owner_stores)(f) { format!("__c_{}.set({val})", ident(f)) } else { format!("{} = {val}", ident(f)) };
                Ok(vec![format!("if ({who} != null && {who}.exports.__source != null) {who}.exports.__source.{set};")])
            }
            Place::Temp(t) => {
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("__t{t} = {val};")])
            }
            Place::Name(n) if self.captured.contains(&ident(n)) => {
                let (val, ty) = self.expr(v)?;
                let had = self.captured_ty.get(&ident(n)).copied().unwrap_or(Ty::Any);
                self.captured_ty.insert(ident(n), merge(had, ty));
                Ok(vec![format!("{} = {val};", ident(n))])
            }
            Place::Index(a, i) => {
                let (a, _) = self.expr(a)?;
                let (i, ti) = self.expr(i)?;
                if ti == Ty::Str { return Err("writes by a name worked out as it runs".into()) }
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{}[{i}] = {val};", paren(&a))])
            }
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
        // (a label its clip does not have: the source stops there with an error, so nothing happens)
        let class = self.o.model.classes.get(&self.class);
        let has = class.and_then(|c| c.symbol).and_then(|s| self.o.model.sprites.get(&s)).map(|sp| sp.labels.iter().any(|(_, l)| l == label));
        if has == Some(false) { return Ok(format!("// (the source jumps to {label} here, which its clip does not have)")) }
        let unique = self.o.jumps.get(&self.class).and_then(|j| j.labels.get(label)).cloned();
        // a fighter's clip label that several animations show, or none: the one playing, if it
        // shows it; a part of the clip none shows is one this engine never plays
        if unique.is_none() && self.o.fighter {
            let hits: Vec<(String, String, bool)> = self.o.stance_labels.get(&self.class).map(|v| v.iter().filter(|(_, l, _)| l == label).cloned().collect()).unwrap_or_default();
            let Some((first, _, first_start)) = hits.first().cloned() else { return Ok(format!("// (the source goes on to {label} here, a part of its clip no animation here shows)")) };
            let mut out = vec!["switch (self.getAnimation()) {".to_string()];
            for (a, _, start) in &hits {
                let to = if *start { self.play_start(a)? } else { format!("self.playFrameLabel({})", quote(label)) };
                out.push(format!("\tcase {}: {to};", quote(a)));
            }
            let to = if first_start { self.play_start(&first)? } else { format!("{{ self.playAnimation({}); self.playFrameLabel({}); }}", quote(&first), quote(label)) };
            out.push(format!("\tdefault: {to};"));
            out.push("}".into());
            return Ok(out.join("\n"));
        }
        let (anim, start) = unique.ok_or_else(|| format!("jumps to {label}, which no animation shows"))?;
        Ok(if start {
            format!("{};", self.play_start(&anim)?)
        } else {
            // (a label inside an animation: that animation, at the label)
            format!("if (self.getAnimation() != {a}) self.playAnimation({a});\nself.playFrameLabel({});", quote(label), a = quote(&anim))
        })
    }

    // ---- tests and values ---------------------------------------------------------------------

    /// a value as a test, the source's way: false for null, false, 0, NaN and ""
    fn test(&mut self, e: &Expr) -> Result<String, String> {
        // (a test of `a && b` is the two tests)
        let was = std::mem::replace(&mut self.testing, matches!(e, Expr::And(..) | Expr::Or(..)));
        let r = self.expr(e);
        self.testing = was;
        let (t, ty) = r?;
        // (a value that is a literal true or false already)
        if t == "true" || t == "false" { return Ok(t) }
        Ok(match ty {
            Ty::Bool => t,
            Ty::Num => format!("{t} != 0"),
            // (the object itself always is)
            Ty::Object | Ty::Body if t == "self" => "true".into(),
            Ty::Object | Ty::Rect | Ty::Body => format!("{t} != null"),
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
            Expr::This if self.is_itself() => ("self".into(), Ty::Object),
            Expr::Member(t, n) if n == "currentFrame" && self.o.fighter && matches!(**t, Expr::This) && self.field(n).is_none() => (self.clip_frame_helper()?, Ty::Num),
            Expr::Member(t, n) if matches!(n.as_str(), "currentLabel" | "currentFrameLabel") && self.o.fighter && matches!(**t, Expr::This) && self.field(n).is_none() => {
                self.blocks.insert("clip_label".into());
                ("clipLabel()".into(), Ty::Str)
            }
            Expr::Name(n) if matches!(n.as_str(), "currentLabel" | "currentFrameLabel") && self.o.fighter && self.field(n).is_none() => {
                self.blocks.insert("clip_label".into());
                ("clipLabel()".into(), Ty::Str)
            }
            // where its clip's parent (the fighter) is, in the source's units
            Expr::Member(t, n) if matches!(&**t, Expr::Name(p) if p == "parent") && matches!(n.as_str(), "x" | "y") =>
                (format!("self.get{}() / {}", n.to_uppercase(), number(self.o.size)), Ty::Num),
            Expr::Member(t, f) if matches!(**t, Expr::This) => return self.own(f),
            Expr::Member(t, f) if matches!(&**t, Expr::Local(r) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Clip)) => return self.own(f),
            // a field of its owner's (a read of one that is no call)
            Expr::Member(o, f) if !matches!(**o, Expr::This | Expr::Call(..)) && { let w = self.expr(o).map(|x| x.0).unwrap_or_default(); self.owners.contains(&w) } => {
                let (who, _) = self.expr(o)?;
                if self.o.owner_native { return Ok((format!("{}({who}, \"self\", {})", self.owner_field("get")?, quote(f)), Ty::Any)) }
                return Ok((if (self.o.owner_stores)(f) { format!("{who}.exports.__source.__c_{}.get()", ident(f)) } else { format!("{who}.exports.__source.{}", ident(f)) }, Ty::Any));
            }
            // where the clip its animation shows is, as its own clip reads it
            Expr::Member(mc, n) if self.o.fighter && matches!(n.as_str(), "currentLabel" | "currentFrameLabel" | "currentFrame")
                && matches!(&**mc, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "stanceClip" | "getStanceMC") && self.expr(o).is_ok_and(|(w, _)| w == "self"))) => {
                if n == "currentFrame" { return Ok((self.clip_frame_helper()?, Ty::Num)) }
                self.blocks.insert("clip_label".into());
                return Ok(("clipLabel()".into(), Ty::Str));
            }
            // a field of its own class, read through the fighter from one of its clips
            Expr::Member(o, f) if self.o.fighter && !self.is_itself() && !matches!(**o, Expr::This) && self.expr(o).is_ok_and(|(w, _)| w == "self")
                && self.o.itself.is_some_and(|c| self.fields.contains_key(&member_key(&c.name, f)) || self.methods.contains_key(&member_key(&c.name, &format!("get:{f}")))) => {
                let saved = std::mem::replace(&mut self.class, self.o.itself.map(|c| c.name.clone()).unwrap_or_default());
                let r = self.own(f);
                self.class = saved;
                return r;
            }
            // a field of the event a listener is handed
            Expr::Member(d, field) if matches!(&**d, Expr::Member(a, n) if n == "data" && matches!(**a, Expr::Local(_))) => {
                let Expr::Member(a, _) = &**d else { unreachable!() };
                let Expr::Local(r) = &**a else { unreachable!() };
                let (arg, _) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                return record(&arg, field);
            }
            Expr::Name(n) if self.field(n).is_some() || self.method(n).is_some() || self.method(&format!("get:{n}")).is_some() => return self.own(n),
            // a variable its function shares with a closure it makes
            Expr::Temp(t) => (format!("__t{t}"), Ty::Any),
            Expr::Name(n) if self.captured.contains(&ident(n)) => (ident(n), self.captured_ty.get(&ident(n)).copied().unwrap_or(Ty::Any)),
            // the clip it shows sits in the object: its parent is the object
            Expr::Name(n) if n == "parent" => ("self".into(), Ty::Body),
            // the frame its clip is on
            Expr::Name(n) if n == "currentFrame" && self.o.fighter => (self.clip_frame_helper()?, Ty::Num),
            Expr::Member(t, n) if n == "currentFrame" && self.o.fighter && (matches!(**t, Expr::This) || self.is_clip(t)) => (self.clip_frame_helper()?, Ty::Num),
            // the clip an effect it keeps sits in: the effect, while it is there
            Expr::Member(o, n) if n == "parent" && !matches!(**o, Expr::This) && !self.is_clip(o) => {
                let (v, _) = self.expr(o)?;
                if v == "self" { return Err("reads its clip's parent".into()) }
                (format!("({v} == null || {v}.isDisposed() ? null : {v})"), Ty::Object)
            }
            // a field of a record its code got back (its controls: `c.LEFT`)
            Expr::Member(o, n) if n.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()) && self.a_value(o) => {
                let (v, _) = self.expr(o)?;
                (format!("{}.{n}", paren(&v)), Ty::Any)
            }
            Expr::Member(o, n) if self.a_value(o) && matches!(n.as_str(), "scaleX" | "scaleY" | "alpha" | "rotation") && !self.is_clip(o) => {
                let (v, _) = self.expr(o)?;
                let get = match n.as_str() { "scaleX" => "getScaleX", "scaleY" => "getScaleY", "alpha" => "getAlpha", _ => "getRotation" };
                (format!("{}.{get}()", paren(&v)), Ty::Num)
            }
            // a record it was handed (the stage's bounds in the source's units, its palette): its fields
            Expr::Member(o, n) if self.a_value(o) && self.expr(o).is_ok_and(|(_, t)| t == Ty::Rect) => {
                let (v, _) = self.expr(o)?;
                (format!("{}.{n}", paren(&v)), if matches!(n.as_str(), "x" | "y" | "width" | "height") { Ty::Num } else { Ty::Rect })
            }
            Expr::Member(o, n) if n == "length" => { let (v, _) = self.expr(o)?; (format!("{}.length", paren(&v)), Ty::Num) }
            Expr::New(c, a) if matches!(&**c, Expr::Name(n) if calls().records.contains_key(short(n))) => {
                let Expr::Name(n) = &**c else { unreachable!() };
                let mut t = calls().records[short(n)].clone();
                for (i, x) in a.iter().enumerate() { let (v, _) = self.expr(x)?; t = t.replace(&format!("{{{i}}}"), &paren(&v)); }
                (crate::re!(r"\{\d\}").replace_all(&t, "null").to_string(), Ty::Rect)
            }
            Expr::New(c, a) if matches!(&**c, Expr::Name(n) if short(n) == "Point") && a.len() <= 2 => {
                let x = match a.first() { Some(e) => self.expr(e)?.0, None => "0".into() };
                let y = match a.get(1) { Some(e) => self.expr(e)?.0, None => "0".into() };
                (format!("{{ x: {x}, y: {y} }}"), Ty::Rect)
            }
            Expr::New(c, a) if a.is_empty() && matches!(&**c, Expr::Name(n) if matches!(short(n), "Array" | "Object")) => {
                let Expr::Name(n) = &**c else { unreachable!() };
                (if short(n) == "Array" { "[]".into() } else { "{}".into() }, Ty::Object)
            }
            Expr::Member(o, n) if matches!(&**o, Expr::Name(c) if c == "Math") => (format!("Math.{n}"), Ty::Num),
            // AS3's number limits
            Expr::Member(o, n) if matches!(&**o, Expr::Name(c) if matches!(short(c), "Number" | "int" | "uint")) && matches!(n.as_str(), "MAX_VALUE" | "MIN_VALUE") => {
                let Expr::Name(c) = &**o else { unreachable!() };
                (match (short(c), n.as_str()) {
                    ("Number", "MAX_VALUE") => "1.7976931348623157e308".into(), ("Number", _) => "5e-324".into(),
                    ("int", "MAX_VALUE") => "2147483647".into(), ("int", _) => "-2147483648".into(),
                    ("uint", "MAX_VALUE") => "4294967295.0".into(), _ => "0".into(),
                }, Ty::Num)
            }
            // a field of a record one of its own declaration methods declares: its value, as written
            Expr::Member(c, f) if matches!(&**c, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(o, m) if self.expr(o).is_ok_and(|(w, _)| w == "self") && self.declared_field(m, f).is_some())) => {
                let Expr::Call(g, _) = &**c else { unreachable!() };
                let Expr::Member(_, m) = &**g else { unreachable!() };
                (self.declared_field(m, f).unwrap_or_default(), Ty::Any)
            }
            // where an object (or its clip, drawn as it) is, in the source's units
            Expr::Member(o, n) if matches!(n.as_str(), "x" | "y") && self.a_value(o) && self.expr(o).is_ok_and(|(_, t)| t == Ty::Body) => {
                let (v, _) = self.expr(o)?;
                (format!("{}.get{}() / {}", paren(&v), n.to_uppercase(), number(self.o.size)), Ty::Num)
            }
            Expr::Call(f, a) if a.is_empty() && matches!(&**f, Expr::Member(o, m) if matches!(m.as_str(), "getMC" | "stanceClip" | "getStanceMC") && !matches!(**o, Expr::This)) => {
                let Expr::Member(o, _) = &**f else { unreachable!() };
                let (who, _) = self.expr(o)?;
                if who == "self" && !self.o.fighter { return Err("uses its own clip as a value".into()) }
                (who, Ty::Body)
            }
            // a field of its owner's clip (its root, or the clip its animation shows), or where it is
            Expr::Member(mc, n) if self.owner_clip(mc).is_some() && !matches!(n.as_str(), "currentFrameLabel" | "currentLabel") => {
                let (who, clip) = self.owner_clip(mc).unwrap_or_default();
                if matches!(n.as_str(), "x" | "y") { return Ok((format!("{who}.get{}() / {}", n.to_uppercase(), number(self.o.size)), Ty::Num)) }
                return Ok((format!("{}({who}, {}, {})", self.owner_field("get")?, quote(&clip), quote(n)), Ty::Any));
            }
            // the label another object's clip is on: its owner's, from the animation it plays
            Expr::Member(mc, n) if matches!(n.as_str(), "currentFrameLabel" | "currentLabel")
                && matches!(&**mc, Expr::Call(f, a) if a.is_empty() && matches!(&**f, Expr::Member(_, g) if g == "getMC")) => {
                let Expr::Call(f, _) = &**mc else { unreachable!() };
                let Expr::Member(who, _) = &**f else { unreachable!() };
                let (who, _) = self.expr(who)?;
                if who == "self" { return Err("reads its own clip's label".into()) }
                return Ok((format!("{}({who})", self.owner_label()?), Ty::Str));
            }
            // a field of another object's, which its script keeps on the object it publishes
            Expr::Member(o, f) if self.o.fighter && self.a_value(o) && !self.is_clip(o) && f != "data" && f.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && self.expr(o).is_ok_and(|(w, t)| w != "self" && matches!(t, Ty::Object | Ty::Any)) => {
                let (who, _) = self.expr(o)?;
                let get = if (self.o.owner_stores)(f) { format!("__c_{}.get()", ident(f)) } else { ident(f) };
                return Ok((format!("({who} == null || {who}.exports.__source == null ? null : {who}.exports.__source.{get})"), Ty::Any));
            }
            Expr::Call(f, args) => return self.call(f, args),
            Expr::Unary(UnOp::Not, x) => (format!("!{}", paren(&self.test(x)?)), Ty::Bool),
            Expr::Unary(UnOp::Neg, x) => { let (t, _) = self.expr(x)?; (format!("-{}", paren(&t)), Ty::Num) }
            Expr::Binary(op, a, b) => return self.binary(*op, a, b),
            // (as a value, the source's `a || b` is one of them: `a` if it passes, else `b`)
            Expr::And(x, y) | Expr::Or(x, y) if !self.testing && !self.both_bool(x, y) => {
                let or = matches!(e, Expr::Or(..));
                let t = self.test(x)?;
                let (xv, tx) = self.expr(x)?;
                let (yv, ty) = self.expr(y)?;
                let out = merge(tx, ty);
                (if or { format!("({t} ? {xv} : {yv})") } else { format!("({t} ? {yv} : {xv})") }, out)
            }
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
                // (one side worked out, the other not: the one known)
                (format!("({c} ? {a} : {b})"), merge(ta, tb))
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
            Expr::Function(f) => return Ok((self.closure(f)?, Ty::Object)),
            // a record, its fields named outright
            Expr::Object(kv) => {
                let mut parts = Vec::new();
                for (k, v) in kv {
                    let Expr::Str(k) = k else { return Err("makes a record with fields named as it runs".into()) };
                    if !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || k.chars().next().is_some_and(|c| c.is_ascii_digit()) { return Err(format!("makes a record with a field named {k}")) }
                    parts.push(format!("{k}: {}", self.expr(v)?.0));
                }
                (if parts.is_empty() { "{}".into() } else { format!("{{ {} }}", parts.join(", ")) }, Ty::Object)
            }
            Expr::Index(a, i) => {
                let (a, ta) = self.expr(a)?;
                let (i, ti) = self.expr(i)?;
                if ti == Ty::Str { return Err("reads by a name worked out as it runs".into()) }
                // (an item of a list of records is a record)
                (format!("{}[{i}]", paren(&a)), if ta == Ty::Rect { Ty::Rect } else { Ty::Any })
            }
            Expr::New(..) => return Err("makes an object".into()),
            Expr::Assign(..) => return Err("uses an assignment as a value".into()),
            other => return Err(describe(other)),
        })
    }

    /// one of its own fields or methods, read
    fn own(&mut self, n: &str) -> Result<(String, Ty), String> {
        if let Some((id, holds)) = self.field(n) {
            let key = member_key(&self.class, n);
            let plain = self.plain.contains(&key);
            self.read.insert(key);
            return Ok(match holds { Holds::Itself | Holds::Clip => (id, Ty::Object), Holds::Value(t) if plain => (id, t), Holds::Value(t) => (format!("{id}.get()"), t) });
        }
        if let Some(id) = self.method(n) { return Ok((id, Ty::Object)) }
        // (one read through its getter)
        if let Some(id) = self.method(&format!("get:{n}")) { return Ok((format!("{id}()"), Ty::Any)) }
        Err(format!("reads {n}, which is not its own"))
    }

    /// a closure the code makes, as a function of the script's own: one that captures none of the
    /// function's variables reads only the object's fields and methods, so it is the same anywhere
    fn closure(&mut self, f: &Function) -> Result<String, String> {
        let mut captures = false;
        crate::model::ast::walk_exprs(&f.body, &mut |e| if let Expr::Name(n) = e {
            if self.field(n).is_none() && self.method(n).is_none() && n != "Math" && !n.contains("Peptide") { captures = true }
        });
        // (one that reads its maker's variables is written where it is made, and sees them)
        if captures || !f.activation.is_empty() {
            let unknown: Vec<String> = { let mut u = Vec::new(); crate::model::ast::walk_exprs(&f.body, &mut |e| if let Expr::Name(n) = e {
                if self.field(n).is_none() && self.method(n).is_none() && n != "Math" && !n.contains("Peptide") && !self.captured.contains(&ident(n)) && !f.activation.iter().any(|(_, a)| a == n) && !self.o.model.classes.values().any(|c| short(&c.name) == n) { u.push(n.clone()) }
            }); u };
            if !unknown.is_empty() { return Err(format!("a closure that reads {}, which is no variable of its maker", unknown[0])) }
            let (locals, effects) = (std::mem::take(&mut self.locals), self.effects);
            let text = self.function("", f);
            self.locals = locals;
            self.effects = effects;
            return Ok(paren(&text?));
        }
        let n = self.helpers.keys().filter(|k| k.starts_with("closure")).count() + 1;
        let name = format!("closure{n}");
        let (locals, effects) = (std::mem::take(&mut self.locals), self.effects);
        let text = self.function(&name, f);
        self.locals = locals;
        self.effects = effects;
        self.helpers.insert(name.clone(), format!("// a function its code hands over\n{}", text?));
        // (as a listener: found by name like any of its own)
        self.methods.insert(member_key(&self.class, &name), name.clone());
        Ok(name)
    }

    /// a jump its clip makes to a frame worked out as it runs: by label for a text, by number for a
    /// number, and by whichever it turns out to be where the code does not say
    fn jump_to_value(&mut self, label: &Expr) -> Result<String, String> {
        let (l, ty) = self.expr(label)?;
        let saved = std::mem::take(&mut self.class);
        let r = self.stance_jump("");
        self.class = saved;
        r?;
        if ty == Ty::Str { return Ok(format!("stanceJump({l})")) }
        self.clip_frame_helper()?;
        if let Some(def) = self.jump_by_number.clone() { self.helpers.insert("toClipFrame".into(), def); }
        if ty == Ty::Num { return Ok(format!("toClipFrame({l})")) }
        Ok(format!("if (Std.isOfType({l}, String)) stanceJump({l}); else toClipFrame({l})"))
    }

    /// the helpers that read the frame its clip is on and jump it to one by number
    fn clip_frame_helper(&mut self) -> Result<String, String> {
        if !self.helpers.contains_key("clipFrame") {
            let mut at = Vec::new();
            let mut to = Vec::new();
            for anims in self.o.clip_frames.values() {
                // (the animations with the same first frame answer alike)
                let mut by_from: BTreeMap<u32, Vec<String>> = BTreeMap::new();
                for (a, from, _) in anims { by_from.entry(*from).or_default().push(quote(a)); }
                for (from, names) in by_from { at.push(format!("case {}: return {from} + k;", names.join(", "))); }
                let names: Vec<String> = anims.iter().map(|(a, _, _)| quote(a)).collect();
                let froms: Vec<String> = anims.iter().map(|(_, f, _)| f.to_string()).collect();
                let lens: Vec<String> = anims.iter().map(|(_, _, n)| n.to_string()).collect();
                to.push(format!("case {}: goToFrame(f, [{}], [{}], [{}]);", names.join(", "), names.join(", "), froms.join(", "), lens.join(", ")));
            }
            self.helpers.insert("clipFrame".into(), helper(self.o.runtime, "helpers", "clip_frame", "clipFrame", &[], &at)?);
            self.jump_by_number = Some(helper(self.o.runtime, "helpers", "to_clip_frame", "toClipFrame", &[], &to)?);
        }
        Ok("clipFrame()".into())
    }

    /// one of the class being written's own methods, by name
    fn method(&self, n: &str) -> Option<String> { self.methods.get(&member_key(&self.class, n)).cloned() }
    /// one of the object's own class's methods (a clip's code calls them on the object)
    fn itself_method(&self, n: &str) -> Option<String> { self.o.itself.and_then(|c| self.methods.get(&member_key(&c.name, n)).cloned()) }
    /// one of the class being written's own fields, by name
    fn field(&self, n: &str) -> Option<(String, Holds)> { self.fields.get(&member_key(&self.class, n)).cloned() }

    /// whether both sides of an `&&` or `||` are true-or-false already (it is then the plain one)
    fn both_bool(&mut self, a: &Expr, b: &Expr) -> bool {
        let saved = (self.helpers.clone(), self.kept.clone(), self.blocks.clone(), self.stat_keys.clone(), self.read.clone());
        let r = matches!(self.expr(a), Ok((_, Ty::Bool))) && matches!(self.expr(b), Ok((_, Ty::Bool)));
        (self.helpers, self.kept, self.blocks, self.stat_keys, self.read) = saved;
        r
    }

    /// a call of its owner's clip (`getMC()` its root, `getStanceMC()` the one its animation shows):
    /// the owner, and which clip
    fn owner_clip(&mut self, e: &Expr) -> Option<(String, String)> {
        let Expr::Call(f, a) = e else { return None };
        let Expr::Member(o, m) = &**f else { return None };
        if !a.is_empty() { return None }
        let clip = match m.as_str() { "getMC" => "root", "getStanceMC" | "stanceClip" => "stance", _ => return None };
        let who = self.expr(o).ok()?.0;
        // (any other object's: each kind of fighter publishes its clips' fields)
        (who != "self").then(|| (who, clip.to_string()))
    }

    /// the helper that reads (or writes) a field of its owner's clips through what the owner publishes
    fn owner_field(&mut self, how: &str) -> Result<String, String> {
        let (key, name) = if how == "set" { ("owner_set_field", "setOwnerField") } else { ("owner_field", "ownerField") };
        if !self.helpers.contains_key(name) { self.helpers.insert(name.into(), helper(self.o.runtime, "helpers", key, name, &[], &[])?); }
        Ok(name.into())
    }

    /// the one record its class ever puts in a field, where it puts nothing else there
    fn only_record(&self, f: &str) -> Option<Expr> {
        let c = self.o.model.classes.get(&self.class)?;
        let mut found: Option<Expr> = None;
        let mut other = false;
        for m in &c.methods {
            walk(&m.function.body, &mut |s| if let Stmt::Assign(Place::Member(Expr::This, g), v) = s {
                if g != f { return }
                let mut v = v;
                while let Expr::Coerce(_, x) = v { v = x; }
                match (v, &found) {
                    (Expr::Object(_), None) => found = Some(v.clone()),
                    (Expr::Object(_), Some(r)) if r == v => {}
                    _ => other = true,
                }
            });
        }
        if other { None } else { found }
    }

    /// whether an expression is a value its code holds (not its clip, and not a name of the API's,
    /// though a variable it shares with a closure is one)
    fn a_value(&self, o: &Expr) -> bool {
        match o { Expr::This => false, Expr::Name(n) => self.captured.contains(&ident(n)), _ => true }
    }

    /// a helper of `helpers.hx` that takes the source's size, written once
    fn sized_helper(&mut self, key: &str, name: &str) -> Result<String, String> {
        if !self.helpers.contains_key(name) { self.helpers.insert(name.into(), helper(self.o.runtime, "helpers", key, name, &[("SIZE", number(self.o.size))], &[])?); }
        Ok(name.into())
    }

    /// whether a value is a copy of its own clip
    fn is_clip(&self, e: &Expr) -> bool { matches!(e, Expr::Local(r) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Clip)) }

    /// whether the class being written is the object's own (its `this` the object itself)
    fn is_itself(&self) -> bool { self.o.itself.is_some_and(|c| c.name == self.class) }

    /// the helper that answers the label its owner's clip is on
    fn owner_label(&mut self) -> Result<String, String> {
        let name = "ownerLabel".to_string();
        if self.helpers.contains_key(&name) { return Ok(name) }
        let mut cases: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for (anim, label) in self.o.owner_anims { if !label.is_empty() { cases.entry(label.as_str()).or_default().push(quote(anim)); } }
        if cases.is_empty() { return Err("reads its owner's clip label, and its owner's animations show none".into()) }
        let lines: Vec<String> = cases.iter().map(|(label, anims)| format!("case {}: return {};", anims.join(", "), quote(label))).collect();
        let def = helper(self.o.runtime, "helpers", "owner_label", &name, &[], &lines)?;
        self.helpers.insert(name.clone(), def);
        Ok(name)
    }

    fn binary(&mut self, op: BinOp, a: &Expr, b: &Expr) -> Result<(String, Ty), String> {
        // (a cast says nothing here: the value is what it is)
        if op == BinOp::As { return self.expr(a) }
        // an object's kind against the source's name for one: the engine's kind
        if matches!(op, BinOp::Eq | BinOp::StrictEq | BinOp::Ne | BinOp::StrictNe) {
            let kind = |x: &Expr| match x { Expr::Call(f, args) if args.is_empty() => match &**f { Expr::Member(o, m) if m == "getType" => Some((**o).clone()), _ => None }, _ => None };
            let (obj, name) = match (kind(a), kind(b), a, b) {
                (Some(o), _, _, Expr::Str(n)) | (_, Some(o), Expr::Str(n), _) => (Some(o), n.clone()),
                _ => (None, String::new()),
            };
            if let Some(o) = obj {
                let t = calls().types.get(&name).cloned().ok_or_else(|| format!("compares a kind to {name}, which has no row in native_calls.jsonc"))?;
                let (who, _) = self.expr(&o)?;
                let eq = if matches!(op, BinOp::Eq | BinOp::StrictEq) { "==" } else { "!=" };
                return Ok((format!("{who}.getType() {eq} {t}"), Ty::Bool));
            }
        }
        // the state an event says it left or went to, against one of the source's
        if matches!(op, BinOp::Eq | BinOp::StrictEq | BinOp::Ne | BinOp::StrictNe) {
            let field = |x: &Expr| match x { Expr::Member(d, f) if matches!(f.as_str(), "fromState" | "toState") && matches!(&**d, Expr::Member(a, n) if n == "data" && matches!(**a, Expr::Local(_))) => Some((f.clone(), (**d).clone())), _ => None };
            if let Some(((f, d), st)) = field(a).map(|x| (x, b)).or_else(|| field(b).map(|x| (x, a))) {
                let name = self.state_name(st)?;
                let Expr::Member(arg, _) = &d else { unreachable!() };
                let (arg, _) = self.expr(arg)?;
                let tests: Vec<String> = states().get(&name).into_iter().flat_map(|s| s.is.iter()).filter(|t| t.starts_with("CState.")).map(|t| format!("{arg}.data.{f} == {t}")).collect();
                let t = if tests.is_empty() { "false".to_string() } else { tests.join(" || ") };
                return Ok((if matches!(op, BinOp::Eq | BinOp::StrictEq) { t } else { format!("!({t})") }, Ty::Bool));
            }
        }
        // its state against one of the source's: the engine's states it is
        if matches!(op, BinOp::Eq | BinOp::StrictEq | BinOp::Ne | BinOp::StrictNe) {
            let state_of = |x: &Expr| match x { Expr::Call(f, args) if args.is_empty() => match &**f { Expr::Member(o, m) if m == "getState" => Some((**o).clone()), _ => None }, _ => None };
            if let Some((o, st)) = state_of(a).map(|o| (o, b)).or_else(|| state_of(b).map(|o| (o, a))) {
                let name = self.state_name(st)?;
                let (who, _) = self.expr(&o)?;
                let tests: Vec<String> = states().get(&name).into_iter().flat_map(|s| s.is.iter()).map(|t| if t.starts_with("CStateGroup.") {
                    format!("{who}.inStateGroup({t})") } else { format!("{who}.inState({t})") }).collect();
                let t = if tests.is_empty() { "false".to_string() } else { tests.join(" || ") };
                return Ok((if matches!(op, BinOp::Eq | BinOp::StrictEq) { t } else { format!("!({t})") }, Ty::Bool));
            }
        }
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
            // AS3's conversions, called as functions
            if let (Expr::Name(n), [x]) = (f, args) {
                match short(n) {
                    "Number" => { let (v, t) = self.expr(x)?; return Ok((if t == Ty::Num { v } else { format!("Std.parseFloat(Std.string({v}))") }, Ty::Num)) }
                    "int" | "uint" => { let (v, t) = self.expr(x)?; return Ok((if t == Ty::Num { format!("Std.int({v})") } else { format!("Std.int(Std.parseFloat(Std.string({v})))") }, Ty::Num)) }
                    "String" => { let (v, _) = self.expr(x)?; return Ok((format!("Std.string({v})"), Ty::Str)) }
                    "Boolean" => return Ok((self.test(x)?, Ty::Bool)),
                    _ => {}
                }
            }
            // one of its own methods, called bare
            if let Expr::Name(n) = f {
                if let Some(id) = self.method(n) { return self.own_call(&id, args) }
                if self.is_itself() { return self.api("self", n, args) }
                return Err(format!("calls {n} on its clip"));
            }
            return Err(format!("calls {}", describe(f)));
        };
        // one of its own methods
        if matches!(**recv, Expr::This) {
            if let Some(id) = self.method(m) { return self.own_call(&id, args) }
            // (the object's own class calls the API on itself)
            if !self.is_itself() { return Err(format!("calls {m} on its clip")) }
        }
        // the API's own questions about the object
        if let Expr::Name(c) = &**recv {
            match (short(c), m.as_str(), args) {
                (_, "getProjectile" | "getItem" | "getEnemy" | "getCharacter", [Expr::This]) if c.contains("PeptideAPI") => return Ok(("self".into(), Ty::Object)),
                (_, "isReady", []) if c.contains("PeptideAPI") => return Ok(("true".into(), Ty::Bool)),
                // the ground on a line, and an enemy made: a fighter's blocks
                (_, "hitTestGround" | "hitTestGroundBetweenPoints", _) if c.contains("PeptideAPI") && self.o.fighter => {
                    self.blocks.insert("ground_test".into());
                    let vs: Result<Vec<String>, String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                    let vs = vs?;
                    let opt = |i: usize| vs.get(i).cloned().unwrap_or_else(|| "null".into());
                    return Ok((if m == "hitTestGround" { format!("groundBetween({0}, {1} - 1, {0}, {1} + 1, {2})", opt(0), opt(1), opt(2)) }
                        else { format!("(({0}) == null || ({1}) == null ? null : groundBetween(({0}).x, ({0}).y, ({1}).x, ({1}).y, {2}))", opt(0), opt(1), opt(2)) }, Ty::Object));
                }
                (_, "makeEnemy", [k]) if c.contains("PeptideAPI") && self.o.fighter => {
                    let n = match k { Expr::Str(n) => n.clone(), Expr::Name(n) => short(n).to_string(), _ => return Err("makes an enemy named as it runs".into()) };
                    let f = self.fire(&n, "self")?;
                    let name = format!("makeEnemy_{}", ident(&n));
                    if !self.helpers.contains_key(&name) { self.helpers.insert(name.clone(), helper(self.o.runtime, "helpers", "make_enemy", &name, &[("FIRE", f)], &[])?); }
                    return Ok((format!("{name}()"), Ty::Object));
                }
                // an effect made by the API: on the object whose code makes it, as its own
                (_, "createVfx" | "createVfxOverlay", _) if c.contains("PeptideAPI") => {
                    let saved = std::mem::replace(&mut self.effect_on, "self".into());
                    let r = if top { self.top = true; let r = self.effect(args); self.top = false; r } else { self.kept_effect(args) };
                    self.effect_on = saved;
                    return r;
                }
                // the API's own calls with a row (`PeptideAPI.print`)
                (_, _, _) if c.contains("PeptideAPI") => {
                    let row = calls().rows.get(&format!("PeptideAPI.{m}")).cloned().ok_or_else(|| format!("calls PeptideAPI.{m}, which has no row in native_calls.jsonc"))?;
                    if row.code.is_empty() && top { return Ok((String::new(), Ty::Any)) }
                    return self.fill(&row, "self", args, m);
                }
                // the API's helpers written as plain functions (`runtime/native/utils.hx`)
                (n, _, _) if n.contains("PeptideUtils") && self.o.runtime.native_files.get("utils").is_some_and(|t| t.lines().any(|l| l.trim_end() == format!("// ---- {m}"))) => {
                    if !self.helpers.contains_key(m.as_str()) { self.helpers.insert(m.clone(), helper(self.o.runtime, "utils", m, m, &[], &[])?); }
                    let vs: Result<Vec<String>, String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                    return Ok((format!("{m}({})", vs?.join(", ")), if m == "getVelocityVector" { Ty::Rect } else { Ty::Num }));
                }
                // the API's other helpers with a row (`PeptideUtils.toDegrees`)
                (n, _, _) if calls().rows.contains_key(&format!("{n}.{m}")) => {
                    let row = calls().rows[&format!("{n}.{m}")].clone();
                    return self.fill(&row, "self", args, m);
                }
                ("Math", _, _) => {
                    let vs: Result<Vec<String>, String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                    return Ok((format!("Math.{m}({})", vs?.join(", ")), Ty::Num));
                }
                _ => {}
            }
        }
        // another object's clip sent to a label: its own script's jump, through what it publishes
        if let Expr::Call(g, a) = &**recv {
            if a.is_empty() && matches!(m.as_str(), "gotoAndStop" | "gotoAndPlay") && matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "stanceClip" | "getStanceMC" | "getMC") && self.expr(o).is_ok_and(|(w, _)| w != "self")) {
                let Expr::Member(o, _) = &**g else { unreachable!() };
                let (who, _) = self.expr(o)?;
                let [l] = args else { return Err(format!("calls {m} with other than one frame")) };
                let (l, _) = self.expr(l)?;
                return Ok((format!("if ({who} != null && {who}.exports.__playFrame != null) {who}.exports.__playFrame({l})"), Ty::Any));
            }
        }
        // the clip its animation shows (its stance), asked of directly: its jumps, as its own
        if self.o.fighter {
            if let Expr::Call(g, a) = &**recv {
                if a.is_empty() && matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "stanceClip" | "getStanceMC") && self.expr(o).is_ok_and(|(w, _)| w == "self")) {
                    return match (m.as_str(), args) {
                        ("gotoAndStop" | "gotoAndPlay", [l]) => Ok((self.jump_to_value(l)?, Ty::Any)),
                        ("stop" | "play", []) => Ok((String::new(), Ty::Any)),
                        _ => Err(format!("calls {m} on the clip its animation shows")),
                    };
                }
            }
        }
        // an effect it keeps taken off where it sits: gone; its own clip taken off its parent is a
        // part of the fighter's art this engine draws with it
        if m == "removeChild" && self.o.fighter {
            if let [x] = args {
                if matches!(x, Expr::This) || self.is_clip(x) { return Ok(("// (not ported: a part of its clip takes itself off; this engine draws it with the rest)".into(), Ty::Any)) }
                let (v, _) = self.expr(x)?;
                return Ok((format!("if ({v} != null && !{v}.isDisposed()) {v}.kill()"), Ty::Any));
            }
        }
        let (who, who_ty) = self.expr(recv)?;
        // a call a record answers
        if who_ty == Ty::Rect {
            let row = calls().record_calls.get(m.as_str()).cloned().ok_or_else(|| format!("calls {m} on a record"))?;
            return self.fill(&Row { code: row, returns: Ty::Any }, &who, args, m);
        }
        // one of its own declaration methods: the record it declares
        if who == "self" && args.is_empty() {
            if let Some(r) = self.declared_record(m) { return Ok((r, Ty::Rect)) }
        }
        // a method of the object's own class, called through the object from another of its classes
        if who == "self" && !self.is_itself() {
            if let Some(id) = self.itself_method(m) { return self.own_call(&id, args) }
        }
        if (who == "self" || self.o.fighter) && matches!(m.as_str(), "createVfx" | "createVfxOverlay") {
            let saved = std::mem::replace(&mut self.effect_on, who.clone());
            let r = if top { self.top = true; let r = self.effect(args); self.top = false; r } else { self.kept_effect(args) };
            self.effect_on = saved;
            return r;
        }
        self.api(&who, m, args)
    }

    /// an effect it makes and lets go of, named outright, with its options written out: the
    /// engine's own effect, where the source puts it
    fn effect(&mut self, args: &[Expr]) -> Result<(String, Ty), String> {
        let Some(Expr::Str(name)) = args.first() else { return Err("makes an effect named as it runs".into()) };
        let Some((stats, gone)) = (self.o.effect)(name) else {
            // (one with no art in this conversion shows nothing on either path)
            // (the runtime would not make it either)
            if self.top { return Ok((format!("// (the source shows its effect {name} here, which has no art in this conversion)"), Ty::Any)) }
            // (as a value: the effect it would keep is none)
            return Ok(("null".into(), Ty::Object));
        };
        let mut opts: BTreeMap<String, &Expr> = BTreeMap::new();
        match args.get(1) {
            None | Some(Expr::Null) | Some(Expr::Undefined) => {}
            Some(Expr::Object(kv)) => for (k, v) in kv {
                let Expr::Str(k) = k else { return Err("makes an effect with options named as it runs".into()) };
                if !matches!(k.as_str(), "x" | "y" | "scaleX" | "scaleY" | "rotation" | "behind" | "parentLock" | "syncHitStun" | "flipOnX" | "loop") {
                    return Err(format!("makes an effect with its {k} set"));
                }
                opts.insert(k.clone(), v);
            },
            _ => return Err("makes an effect with options worked out as it runs".into()),
        }
        let flag = |k: &str| -> Result<bool, String> { match opts.get(k) { None => Ok(false), Some(Expr::Bool(b)) => Ok(*b), _ => Err(format!("makes an effect with {k} worked out as it runs")) } };
        let (locked, behind) = (flag("parentLock")?, flag("behind")?);
        // (one asked to loop plays on until it is taken off)
        let looped = matches!(opts.get("loop"), Some(Expr::Bool(true)) | Some(Expr::Int(_)) | Some(Expr::Num(_)));
        let num = |w: &mut Self, k: &str| -> Result<Option<String>, String> { opts.get(k).map(|e| Ok(w.expr(e)?.0)).transpose() };
        let (ox, oy) = (num(self, "x")?, num(self, "y")?);
        let size = number(self.o.size);
        let off = |o: &Option<String>| o.as_ref().map(|t| fold(&format!("{} * {size}", paren(t)))).unwrap_or_else(|| "0".into());
        // (an offset of nothing moves nothing)
        let (ox, oy) = (ox.filter(|t| !t.parse::<f64>().is_ok_and(|v| v == 0.0)), oy.filter(|t| !t.parse::<f64>().is_ok_and(|v| v == 0.0)));
        let mut st = vec![stats];
        if let Some(n) = gone { st.push(format!("timeout: {}", 2 * (n.max(1) - 1))); }
        for k in ["scaleX", "scaleY"] { if let Some(t) = num(self, k)? { st.push(format!("{k}: {t}")); } }
        if behind { st.push("layer: VfxLayer.CHARACTERS_BACK".into()); }
        if looped { st.retain(|x| !x.starts_with("timeout:")); st.push("loop: true".into()); }
        let mut after = Vec::new();
        if locked {
            // riding the object, which mirrors the offset with it
            st.push(format!("x: {}.flipX({}), y: {}, relativeWith: true, resizeWith: true, flipWith: true", self.effect_on, off(&ox), off(&oy)));
        } else {
            if ox.is_some() { after.push(format!("setX({}.getX() + {})", self.effect_on, off(&ox))); }
            if oy.is_some() { after.push(format!("setY({}.getY() + {})", self.effect_on, off(&oy))); }
        }
        if let Some(t) = num(self, "rotation")? { after.push(format!("setRotation({t})")); }
        let on = self.effect_on.clone();
        let make = format!("match.createVfx(new VfxStats({{ {} }}), {on})", st.join(", "));
        if after.is_empty() { return Ok((make, Ty::Any)) }
        self.effects += 1;
        let v = if self.effects == 1 { "effect".to_string() } else { format!("effect{}", self.effects) };
        let set: Vec<String> = after.iter().map(|a| format!("{v}.{a};")).collect();
        Ok((format!("var {v} = {make};\nif ({v} != null) {{ {} }}", set.join(" ")), Ty::Any))
    }

    /// an effect its code keeps (a value): made where it is, then placed off it by the offsets asked
    fn kept_effect(&mut self, args: &[Expr]) -> Result<(String, Ty), String> {
        let (t, ty) = self.effect(args)?;
        if !t.contains('\n') { return Ok((t, ty)) }
        // (made again without its placing, which `placeEffect` does)
        let Some(Expr::Object(kv)) = args.get(1) else { return Err("keeps an effect it places after making it".into()) };
        let placed = |k: &str| kv.iter().find(|(x, _)| matches!(x, Expr::Str(x) if x == k)).map(|(_, v)| v.clone());
        let rest: Vec<(Expr, Expr)> = kv.iter().filter(|(k, _)| !matches!(k, Expr::Str(k) if k == "x" || k == "y" || k == "rotation")).cloned().collect();
        let (make, _) = self.effect(&[args[0].clone(), Expr::Object(rest)])?;
        let size = number(self.o.size);
        let off = |w: &mut Self, k: &str| -> Result<String, String> { Ok(match placed(k) { None => "0".into(), Some(e) => fold(&format!("{} * {size}", paren(&w.expr(&e)?.0))) }) };
        let (x, y) = (off(self, "x")?, off(self, "y")?);
        self.blocks.insert("place_effect".into());
        let rot = match placed("rotation") { None => "null".to_string(), Some(e) => self.expr(&e)?.0 };
        Ok((format!("placeEffect({make}, {x}, {y}, {rot})"), ty))
    }

    /// a call of one of its own declaration methods: the record it declares, as written
    fn declared_record(&self, n: &str) -> Option<String> {
        let c = self.o.itself?;
        let m = c.methods.iter().find(|m| m.name == n && declares(m))?;
        Some(plain_value(&crate::model::declare::returned(&m.function.body)?))
    }

    /// a field of one of its own declaration methods' records, as written
    fn declared_field(&self, n: &str, f: &str) -> Option<String> {
        let c = self.o.itself?;
        let m = c.methods.iter().find(|m| m.name == n && declares(m))?;
        crate::model::declare::returned(&m.function.body)?.get(f).map(plain_value)
    }

    fn own_call(&mut self, id: &str, args: &[Expr]) -> Result<(String, Ty), String> {
        let vs: Result<Vec<String>, String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect();
        Ok((format!("{id}({})", vs?.join(", ")), Ty::Any))
    }

    /// an API call on an engine object (`who`: `self`, or the variable holding another)
    fn api(&mut self, who: &str, m: &str, args: &[Expr]) -> Result<(String, Ty), String> {
        let mine = who == "self";
        if mine && self.o.fighter {
            if let Some(r) = self.fighter_api(m, args) { return r }
        }
        match (m, args) {
            ("addEventListener" | "removeEventListener", [ev, handler, ..]) => return self.listener(who, m, ev, handler),
            ("addTimer", [frames, repeat, f, rest @ ..]) if mine => return self.timer(frames, repeat, f, rest),
            // (fired through its owner's call: from the owner, the same way, as the source's runs it)
            ("createProjectile", [Expr::Str(name), rest @ ..]) if mine || self.owners.contains(who) => {
                let f = self.fire(name, if mine { "self" } else { who })?;
                let vs: Result<Vec<String>, String> = rest.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                return Ok((format!("{f}({})", vs?.join(", ")), Ty::Object));
            }
            // an attack's stats written on a projectile: none is a projectile's here but the rate it
            // hits again at, which is the runtime's still
            ("updateAnimationStats", [Expr::Object(pairs)]) if mine => {
                if pairs.iter().any(|(k, _)| !matches!(k, Expr::Str(_))) { return Err("writes an attack stat by a name worked out as it runs".into()) }
                // (the rate it hits again at: `runtime/native/refresh.hx`)
                let Some((_, v)) = pairs.iter().find(|(k, _)| matches!(k, Expr::Str(k) if k == "refreshRate")) else { return Ok((String::new(), Ty::Any)) };
                self.blocks.insert("refresh".into());
                let (v, _) = self.expr(v)?;
                return Ok((format!("setRefresh({v})"), Ty::Any));
            }
            // a fighter's state: a projectile is in none of them
            ("inState", [_]) if mine && !self.o.fighter => return Ok(("false".into(), Ty::Bool)),
            ("setState", [st]) if !mine => {
                let name = self.state_name(st)?;
                let to = states().get(&name).and_then(|s| s.enter.clone()).ok_or_else(|| format!("sets a fighter's state to {name}, which this engine's fighters have none of"))?;
                return Ok((format!("{who}.toState({to})"), Ty::Any));
            }
            // another projectile's own stats written: the engine's own stat on it
            ("updateProjectileStats", [Expr::Object(pairs)]) if !mine && self.o.fighter => {
                let mut out = Vec::new();
                for (k, v) in pairs {
                    let Expr::Str(k) = k else { return Err("writes a stat by a name worked out as it runs".into()) };
                    let t = calls().projectile_stats.get(k).filter(|t| t.starts_with("self.")).ok_or_else(|| format!("writes another projectile's stat {k}, which has no row in native_calls.jsonc"))?.clone();
                    let (val, _) = self.expr(v)?;
                    out.push(fold(&format!("{who}{}", &t[4..]).replace("{v}", &paren(&val)).replace("{size}", &number(self.o.size))
                        .replace("{speed}", &number(self.o.speed)).replace("{accel}", &number(self.o.accel))));
                }
                return Ok((out.join(";\n"), Ty::Any));
            }
            // a projectile's state, by the source's name for it
            ("inState", [Expr::Member(c, n)]) if matches!(&**c, Expr::Name(c) if short(c) == "PeptidePState") => {
                let t = calls().projectile_states.get(n).ok_or_else(|| format!("tests a projectile's state {n}, which has no row in native_calls.jsonc"))?;
                return Ok((t.replace("{self}", who), Ty::Bool));
            }
            // attack stats written on another object, that this engine keeps none of: nothing here,
            // as the runtime keeps them for nothing that reads them
            ("updateAnimationStats" | "updateAttackStats", [Expr::Object(pairs)]) if !mine && self.o.fighter
                && pairs.iter().all(|(k, _)| matches!(k, Expr::Str(k) if !super::attack_stats::table().maps(k) && k != "refreshRate")) => {
                return Ok(("// (not ported, as on the runtime: attack stats this engine keeps none of, written on another object)".into(), Ty::Any));
            }
            ("inState", [st]) => {
                let name = self.state_name(st)?;
                let tests: Vec<String> = states().get(&name).into_iter().flat_map(|s| s.is.iter()).map(|t| if t.starts_with("CStateGroup.") {
                    format!("{who}.inStateGroup({t})") } else { format!("{who}.inState({t})") }).collect();
                return Ok((if tests.is_empty() { "false".into() } else { tests.join(" || ") }, Ty::Bool));
            }
            ("removeTimer", [f]) if mine && self.o.fighter => {
                let id = self.timer_of(f)?;
                self.blocks.insert("timers".into());
                return Ok((format!("stopTimer({id}.get())"), Ty::Any));
            }
            ("removeTimer", [f]) if mine => {
                let id = self.timer_of(f)?;
                // (and forgets the id: the engine gives it out again, to another of its timers)
                return Ok((format!("self.removeTimer({id}.get());\n{id}.set(-1)"), Ty::Any));
            }
            ("playFrame", [Expr::Str(label)]) if mine => return Ok((self.jump(label)?.trim_end_matches(';').to_string(), Ty::Any)),
            ("playFrame", _) if mine => return Err("jumps its own clip to a label worked out as it runs".into()),
            ("updateProjectileStats", [Expr::Object(pairs)]) if mine => return self.projectile_stats(pairs),
            ("getProjectileStat", [Expr::Str(k)]) if mine && self.own_stats.contains(k) => return Ok((format!("{}.get()", self.own_stat(k)), Ty::Any)),
            ("getProjectileStat", [Expr::Str(k)]) if mine => {
                return (self.o.declared)(k).map(|v| (v, Ty::Any)).ok_or_else(|| format!("reads its stat {k}, which its declarations do not agree on"));
            }
            ("getHitboxStat", [i, Expr::Str(k)]) if mine => {
                let (field, scale) = hitbox_field(k)?;
                let i = box_index(i)?;
                let back = scale.map(|s| format!(" / {}", number(s))).unwrap_or_default();
                return Ok((format!("self.getHitboxStats({i}).{field}{back}"), Ty::Num));
            }
            ("updateHitboxStats", [i, Expr::Member(t, f)]) if matches!(**t, Expr::This) && (mine || self.o.fighter) => {
                let record = self.only_record(f).ok_or_else(|| format!("writes box stats from {f}, a record worked out as it runs"))?;
                return self.api(who, m, &[i.clone(), record]);
            }
            ("updateHitboxStats", [i, Expr::Object(pairs)]) if mine || self.o.fighter => {
                let i = box_index(i)?;
                let mut set = Vec::new();
                for (k, v) in pairs {
                    let Expr::Str(k) = k else { return Err("writes a box stat by a name worked out as it runs".into()) };
                    // a fixed launch: worked out from the fields written with it, as a declared one is
                    if k == "weightKB" {
                        let lits: BTreeMap<String, f64> = pairs.iter().filter_map(|(k, v)| match (k, literal(v)) { (Expr::Str(k), Some(t)) => t.parse::<f64>().ok().map(|n| (k.clone(), n)), _ => None }).collect();
                        let Some(w) = lits.get("weightKB") else { return Err("writes a fixed launch (weightKB) worked out as it runs".into()) };
                        if *w <= 0.0 { continue }
                        let hb = crate::mappings::character_hitbox_stats();
                        let launch = hb.fixed_knockback(&lits).ok_or("writes a fixed launch the mapping has no rule for")?;
                        let (field, scale) = hitbox_field("baseKnockback").unwrap_or(("baseKnockback".into(), None));
                        set.retain(|x: &String| !x.starts_with("baseKnockback:") && !x.starts_with("knockbackGrowth:"));
                        set.push(format!("{field}: {}", number(launch * scale.unwrap_or(1.0))));
                        set.push("knockbackGrowth: 0".into());
                        continue;
                    }
                    if matches!(k.as_str(), "kbConstant" | "power") && pairs.iter().any(|(x, v)| matches!(x, Expr::Str(x) if x == "weightKB") && literal(v).is_some_and(|t| t.parse::<f64>().is_ok_and(|n| n > 0.0))) { continue }
                    // a true-or-false field the engine states under its own (`flags`): stated when it holds
                    let hb = crate::mappings::character_hitbox_stats();
                    if let Some(flag) = hb.flags.iter().find(|f| f.ssf2_key == *k) {
                        let on = match literal(v).as_deref() { Some("true") => true, Some("false" | "0" | "null") => false, Some(n) if n.parse::<f64>().is_ok() => true, _ => return Err(format!("writes box flag {k} worked out as it runs")) };
                        if on == flag.when { set.push(format!("{}: {}", flag.fm_field, flag.value)); }
                        continue;
                    }
                    // a stat this engine has no field for lands on nothing, as the runtime drops it
                    let Ok((field, scale)) = hitbox_field(k) else { continue };
                    let (v, _) = self.expr(v)?;
                    set.push(match scale { Some(s) => format!("{field}: {} * {}", paren(&v), number(s)), None => format!("{field}: {v}") });
                }
                return Ok((format!("{who}.updateHitboxStats({i}, {{ {} }})", set.join(", ")), Ty::Any));
            }
            // another object's attack stat the engine keeps natively, read back from its own field
            // a stat its owner states: what its code set it to since, else what it declares (its
            // script keeps what its code sets on the object it publishes, while it is on the runtime)
            ("getCharacterStat", [Expr::Str(k)]) if self.owners.contains(who) => {
                let declared = (self.o.owner_declares)(k).ok_or_else(|| format!("reads its owner's stat {k}, which its owner does not state"))?;
                let name = format!("ownerStat_{}", ident(k));
                if !self.helpers.contains_key(&name) {
                    let def = helper(self.o.runtime, "helpers", "owner_stat", &name, &[("STAT", k.clone()), ("DECLARED", declared)], &[])?;
                    self.helpers.insert(name.clone(), def);
                }
                return Ok((format!("{name}({who})"), Ty::Any));
            }
            ("moveIfClear" | "safeMove", [dx, dy]) => {
                let f = self.sized_helper("move_if_clear", "moveIfClear")?;
                let (dx, _) = self.expr(dx)?;
                let (dy, _) = self.expr(dy)?;
                return Ok((format!("{f}({who}, {dx}, {dy})"), Ty::Bool));
            }
            // a stat of a projectile it fired: what its declaration states (by the name it was fired by)
            ("getProjectileStat", [Expr::Str(k)]) if self.o.fighter && !mine => {
                self.blocks.insert("fired_stats".into());
                self.stat_keys.insert(format!("projectile:{k}"));
                return Ok((format!("firedStat({who}, {})", quote(k)), Ty::Any));
            }
            ("getLedges", []) if self.o.fighter && who == "stage" => {
                self.blocks.insert("ledges".into());
                return Ok(("stageLedges()".into(), Ty::Rect));
            }
            ("getDeathBounds" | "getCameraBounds", []) if who == "stage" => {
                let f = self.sized_helper("source_rect", "sourceRect")?;
                return Ok((format!("{f}(stage.{m}())"), Ty::Rect));
            }
            // a stat another fighter states: what its code set, as it publishes it, else the engine's
            ("getCharacterStat", [k]) if self.o.fighter && !mine => {
                let (k, _) = self.expr(k)?;
                return Ok((format!("({who}.exports.__source != null && {who}.exports.__source.__stats != null && {who}.exports.__source.__stats.exists({k}) ? {who}.exports.__source.__stats.get({k}) : {who}.getCharacterStat({k}))"), Ty::Any));
            }
            // what an item its fighter made declares
            ("getItemStat", [k]) if self.o.fighter && !mine => {
                self.blocks.insert("items".into());
                let (k, _) = self.expr(k)?;
                return Ok((format!("itemStat({who}, {k})"), Ty::Any));
            }
            // an attack its code starts on its owner, and a rocket it sets off on it: what the owner publishes
            ("forceAttack" | "startAttack", [label, rest @ ..]) if self.owners.contains(who) && !self.o.fighter => {
                let (l, _) = self.expr(label)?;
                let fr = match rest.first() { None | Some(Expr::Null | Expr::Undefined) => "null".to_string(), Some(x) => self.expr(x)?.0 };
                return Ok((format!("if ({who}.exports.__forceAttack != null) {who}.exports.__forceAttack({l}, {fr})"), Ty::Any));
            }
            ("rocketFighter" | "rocketCharacter", [sp, an, de, rest @ ..]) if self.owners.contains(who) && !self.o.fighter => {
                let (sp, an, de) = (self.expr(sp)?.0, self.expr(an)?.0, self.expr(de)?.0);
                let rot = match rest.first() { None => "false".to_string(), Some(x) => self.test(x)? };
                return Ok((format!("if ({who}.exports.__rocket != null) {who}.exports.__rocket({sp}, {an}, {de}, {rot})"), Ty::Any));
            }
            // another fighter's controls, by the source's names
            ("getControls", a) if !mine => {
                if !self.helpers.contains_key("controlsOf") { self.helpers.insert("controlsOf".into(), helper(self.o.runtime, "helpers", "controls_of", "controlsOf", &[], &[])?); }
                let p = match a.first() { None => "false".to_string(), Some(x) => self.test(x)? };
                return Ok((format!("controlsOf({who}, {p})"), Ty::Object));
            }
            // the animation another fighter plays, as the source names it: the label its script publishes
            ("getAnimation", []) if !mine && !self.owners.contains(who) => {
                return Ok((format!("({who} != null && {who}.exports.__label != null ? {who}.exports.__label() : null)"), Ty::Str));
            }
            // the animation its owner plays, as the source names it: its clip's label
            ("getAnimation", []) if self.owners.contains(who) => return Ok((format!("{}({who})", self.owner_label()?), Ty::Str)),
            ("getAnimationStat", [Expr::Str(k)]) if !mine && super::attack_stats::table().read(k).is_some() => {
                let (field, back) = super::attack_stats::table().read(k).unwrap_or_default();
                return Ok((back.replace("{v}", &format!("{who}.getAnimationStat({})", quote(&field))), Ty::Any));
            }
            // one that puts it in a body status: whether it holds that status now
            ("getAnimationStat", [Expr::Str(k)]) if !mine && super::attack_stats::table().read_status(k).is_some() => {
                return Ok((format!("{who}.hasBodyStatus({})", super::attack_stats::table().read_status(k).unwrap_or_default()), Ty::Bool));
            }
            // another fighter's attack stat: what its script answers (each kind of fighter publishes it)
            ("getAnimationStat", [Expr::Str(k)]) if !mine && !self.owners.contains(who) => {
                return Ok((format!("({who} != null && {who}.exports.__attackStat != null ? {who}.exports.__attackStat({}) : null)", quote(k)), Ty::Any));
            }
            ("getAnimationStat", [Expr::Str(k)]) if !mine => {
                let (name, def) = (self.o.owner_stat)(k).ok_or_else(|| format!("reads its owner's attack stat {k}, which no attack of its owner declares"))?;
                // (one no attack of its owner declares is nothing, as the source answers)
                if def.is_empty() { return Ok((name, Ty::Any)) }
                self.helpers.insert(name.clone(), def);
                return Ok((format!("{name}({who})"), Ty::Any));
            }
            _ => {}
        }
        if m == "getType" && args.is_empty() {
            let name = "kindName".to_string();
            if !self.helpers.contains_key(&name) {
                let lines: Vec<String> = calls().types.iter().map(|(n, t)| format!("case {t}: return {};", quote(n))).collect();
                let def = helper(self.o.runtime, "helpers", "kind_name", &name, &[], &lines)?;
                self.helpers.insert(name.clone(), def);
            }
            return Ok((format!("{name}({who})"), Ty::Str));
        }
        let row = match calls().rows.get(m).cloned().or_else(|| self.engine_row(m)) {
            Some(r) => r,
            None => {
                // a method of another object's own class: its script keeps it on the object it publishes
                let defined = !mine && self.o.model.classes.values().any(|c| !self.o.runtime.provides(&c.name) && c.methods.iter().any(|x| x.name == m));
                if !defined { return Err(format!("calls {m}, which has no row in native_calls.jsonc")) }
                let vs: Result<Vec<String>, String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                return Ok((format!("({who} != null && {who}.exports.__source != null ? {who}.exports.__source.{m}({}) : null)", vs?.join(", ")), Ty::Any));
            }
        };
        self.fill(&row, who, args, m)
    }

    /// a call a fighter makes on itself that its rules answer (`runtime/native/fighter_rules.hx`):
    /// None for one that is a plain row like any object's
    fn fighter_api(&mut self, m: &str, args: &[Expr]) -> Option<Result<(String, Ty), String>> {
        let block = |w: &mut Self, b: &str| { w.blocks.insert(b.to_string()); };
        Some((|| -> Result<(String, Ty), String> { match (m, args) {
            // an x speed its code sets: the walk's own start under a held direction
            ("setXSpeed", [v]) if self.o.walk_start.is_some() => {
                block(self, "walk_start");
                let (v, _) = self.expr(v)?;
                Ok((format!("setXSpeedFromCode({})", fold(&format!("{} * {}", paren(&v), number(self.o.speed)))), Ty::Any))
            }
            // its attack's stats written as it plays
            ("updateAnimationStats" | "updateAttackStats", [Expr::Object(pairs)]) => self.attack_stats(pairs),
            ("getAnimationStat" | "getAttackStat", [Expr::Str(k)]) => {
                block(self, "attack");
                self.stat_keys.insert(k.clone());
                Ok((format!("attackStat({})", quote(k)), Ty::Any))
            }
            // its numbered sounds: the attack's own
            ("playAttackSfx" | "playVoiceSfx", [n]) | ("playAttackSfx" | "playVoiceSfx", [n, ..]) => {
                let voice = m == "playVoiceSfx";
                let prefix = if voice { "attackVoice" } else { "attackSound" };
                let n = match n { Expr::Int(n) => *n, Expr::Num(v) if v.fract() == 0.0 => *v as i64, other => {
                    // (numbered as it runs: any of the attack's numbered ones)
                    block(self, "attack"); block(self, "sounds");
                    self.stat_keys.insert(format!("{prefix}*"));
                    let (n, _) = self.expr(other)?;
                    return Ok((format!("playSoundOf(attackStat(\"{prefix}\" + Std.int(Math.max(1, {n})) + \"_id\"), {voice})"), Ty::Object));
                } };
                let key = format!("{}{}_id", if voice { "attackVoice" } else { "attackSound" }, n.max(1));
                block(self, "attack"); block(self, "sounds");
                self.stat_keys.insert(key.clone());
                Ok((format!("playSoundOf(attackStat({}), {voice})", quote(&key)), Ty::Object))
            }
            ("playSound", [id]) | ("playVoice", [id]) => {
                block(self, "attack"); block(self, "sounds");
                let (id, _) = self.expr(id)?;
                Ok((format!("playSoundOf({id}, {})", m == "playVoice"), Ty::Object))
            }
            // an attack it starts by name: the engine's state for it, on the animation that shows it
            ("startAttack" | "forceAttack", [Expr::Str(label), rest @ ..]) => {
                let anim = (self.o.root_anim)(label).ok_or_else(|| format!("starts the attack {label}, which no animation shows"))?;
                block(self, "start_attack");
                let mut t = format!("startAttack({})", quote(&anim));
                if let Some(Expr::Str(f)) = rest.first() { t.push_str(&format!(";
self.playFrameLabel({})", quote(f))); }
                Ok((t, Ty::Any))
            }
            // a jump by label on the clip it shows (whichever animation plays)
            ("stancePlayFrame" | "playFrame", [Expr::Str(label)]) => Ok((self.stance_jump(label)?, Ty::Any)),
            ("stancePlayFrame" | "playFrame", [label]) => Ok((self.jump_to_value(label)?, Ty::Any)),
            // its landing: on touching down, the attack goes on from `continue`, or it lands
            ("setLandingLag", [on]) => {
                let on = self.test(on)?;
                let cont = self.itself_method("jumpToContinue");
                let name = "setLandingLag".to_string();
                if !self.helpers.contains_key(&name) {
                    let cont = match cont { Some(c) => c, None => {
                        self.blocks.insert("attack".into());
                        let saved = std::mem::take(&mut self.class);
                        let r = self.stance_jump("");
                        self.class = saved;
                        r?;
                        self.helpers.insert("jumpToContinue".into(), helper(self.o.runtime, "helpers", "jump_to_continue", "jumpToContinue", &[], &[])?);
                        "jumpToContinue".into()
                    } };
                    let def = helper(self.o.runtime, "helpers", "landing_lag", &name, &[("CONTINUE", cont)], &[])?;
                    self.helpers.insert(name.clone(), def);
                }
                Ok((format!("setLandingLag({on})"), Ty::Any))
            }
            ("hasEventListener", [ev, h]) => {
                let _ = super::events::named(self.o.runtime, ev).ok_or("asks about a listener for an event named as it runs")?;
                let h = match h {
                    Expr::Member(t, n) if matches!(**t, Expr::This) => self.method(n),
                    Expr::Name(n) => self.method(n),
                    _ => None,
                }.ok_or("asks about a listener other than its own method")?;
                let listening = format!("{h}Listening");
                self.kept.insert(listening.clone(), format!("// whether {h} listens now\nvar {listening} = self.{}(false);", bool_make(self.o.fighter)));
                Ok((format!("{listening}.get()"), Ty::Bool))
            }
            ("endAttack" | "endAnimation", []) => { block(self, "end_move"); Ok(("endMove()".into(), Ty::Any)) }
            ("getControls", a) => {
                block(self, "controls");
                let p = match a.first() { None => "false".to_string(), Some(x) => self.test(x)? };
                Ok((format!("controls({p})"), Ty::Object))
            }
            ("getLastUsed", a) => {
                block(self, "attack"); block(self, "last_used");
                let n = match a.first() { None | Some(Expr::Null | Expr::Undefined) => "null".to_string(), Some(x) => self.expr(x)?.0 };
                Ok((format!("lastUsed({n})"), Ty::Num))
            }
            ("setLastUsed", [n, v]) => {
                block(self, "attack"); block(self, "last_used");
                let (n, _) = self.expr(n)?;
                let (v, _) = self.expr(v)?;
                Ok((format!("setLastUsed({n}, {v})"), Ty::Any))
            }
            // its stats its code writes: kept for it to read back, and the engine's own moved by the
            // same share the source's declared one moves (the conversion worked the engine's out from it)
            ("updateCharacterStats", [Expr::Object(pairs)]) => {
                block(self, "char_stats");
                let st = crate::mappings::character_stats();
                let mut out = Vec::new();
                for (k, v) in pairs {
                    let Expr::Str(k) = k else { return Err("writes a stat by a name worked out as it runs".into()) };
                    let (val, ty) = self.expr(v)?;
                    out.push(format!("source.__stats.set({}, {val})", quote(k)));
                    let name = st.fm_runtime_read_names.get(k).cloned().unwrap_or_else(|| k.clone());
                    if !st.fm_runtime_stats.contains(&name) || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { continue }
                    let src = (self.o.owner_declares)(k).and_then(|d| d.parse::<f64>().ok()).filter(|d| *d != 0.0);
                    let value = match (src, ty) {
                        (_, Ty::Bool | Ty::Str) => val.clone(),
                        (Some(d), _) => format!("engineStat({}) * {} / {}", quote(&name), paren(&val), number(d)),
                        (None, _) => {
                            let unit = st.runtime_read_units.iter().find(|(_, l)| l.contains(&name)).map(|(u, _)| u.as_str());
                            let f = match unit { Some("size") => self.o.size, Some("accel") => self.o.accel, Some("speed" | "launch") => self.o.speed, _ => 1.0 };
                            fold(&format!("{} * {}", paren(&val), number(f)))
                        }
                    };
                    out.push(format!("self.updateCharacterStats({{ {name}: {value} }})"));
                }
                Ok((out.join(";\n"), Ty::Any))
            }
            ("getDeathBounds" | "getCameraBounds", []) => {
                let f = self.sized_helper("source_rect", "sourceRect")?;
                Ok((format!("{f}(stage.{m}())"), Ty::Rect))
            }
            // what its code keeps by name, on the object it publishes
            ("sharedVar", [k]) => { let (k, _) = self.expr(k)?; Ok((format!("source.__vars.get({k})"), Ty::Any)) }
            ("setSharedVar", [k, v]) => { let (k, _) = self.expr(k)?; let (v, _) = self.expr(v)?; Ok((format!("source.__vars.set({k}, {v})"), Ty::Any)) }
            ("moveIfClear" | "safeMove", [dx, dy]) => {
                let f = self.sized_helper("move_if_clear", "moveIfClear")?;
                let (dx, _) = self.expr(dx)?;
                let (dy, _) = self.expr(dy)?;
                Ok((format!("{f}(self, {dx}, {dy})"), Ty::Bool))
            }
            // a floor of its own, carried with it
            ("makeOwnFloor", a) if a.len() >= 4 => {
                block(self, "own_floor");
                let vs: Result<Vec<String>, String> = a.iter().take(4).map(|x| Ok(self.expr(x)?.0)).collect();
                let solid = match a.get(4) { None => "false".to_string(), Some(x) => self.test(x)? };
                Ok((format!("makeOwnFloor({}, {solid})", vs?.join(", ")), Ty::Object))
            }
            ("dropOwnFloor", []) => { block(self, "own_floor"); Ok(("dropOwnFloor()".into(), Ty::Any)) }
            ("getNearestPath" | "getNearest", [t, ..]) => {
                block(self, "nearest");
                let (t, _) = self.expr(t)?;
                Ok((if m == "getNearestPath" { format!("nearestPath({t})") } else { format!("nearest({t})") }, Ty::Object))
            }
            ("recover", [n]) => { block(self, "recover"); let (n, _) = self.expr(n)?; Ok((format!("recover({n})"), Ty::Bool)) }
            ("forceOnGround", _) => { block(self, "floor_hold"); Ok(("holdToFloor()".into(), Ty::Any)) }
            ("allowAttack", [on, Expr::Str(label), rest @ ..]) => {
                block(self, "attack"); block(self, "actions");
                self.stat_keys.insert("reenableTimer".into());
                self.stat_keys.insert(format!("anims:{label}"));
                let on = self.test(on)?;
                let frames = match rest.first() { None | Some(Expr::Null | Expr::Undefined) => String::new(), Some(x) => format!(", {}", self.expr(x)?.0) };
                Ok((format!("allowAttack({on}, {}{frames})", quote(label)), Ty::Any))
            }
            ("attackAllowed", [Expr::Str(label)]) => {
                block(self, "attack"); block(self, "actions");
                self.stat_keys.insert(format!("anims:{label}"));
                Ok((format!("attackAllowed({})", quote(label)), Ty::Bool))
            }
            // its state, set and tested by the source's names (`runtime/states.jsonc`)
            ("setState", [st]) => {
                let name = self.state_name(st)?;
                let to = states().get(&name).and_then(|s| s.enter.clone()).ok_or_else(|| format!("sets its state to {name}, which this engine's fighters have none of"))?;
                Ok((format!("self.toState({to})"), Ty::Any))
            }
            // the item it holds: one its own code made (no other here is held by a fighter)
            ("getItem", []) => { block(self, "items"); Ok(("heldItem()".into(), Ty::Object)) }
            ("makePickup" | "generateItem", [Expr::Str(name), ..]) => {
                block(self, "items");
                self.stat_keys.insert(format!("item:{name}"));
                Ok((format!("generateItem({})", quote(name)), Ty::Object))
            }
            ("throwPickup" | "tossItem", a) => {
                block(self, "items");
                let v = match a.first() { None | Some(Expr::Null | Expr::Undefined) => String::new(), Some(x) => self.expr(x)?.0 };
                Ok((format!("tossItem({v})"), Ty::Any))
            }
            ("dropItem", a) => {
                block(self, "items");
                let v = match a.first() { None => String::new(), Some(x) => self.test(x)? };
                Ok((format!("dropItem({v})"), Ty::Any))
            }
            ("removeItem", []) => { block(self, "items"); Ok(("removeItem()".into(), Ty::Any)) }
            // an item off the stage: the stage's items are not the source's here, so there is none to take
            ("pickupItem", _) => Ok((String::new(), Ty::Any)),
            // the effects its moves keep, taken off together
            ("addEffectToList", [fx]) => { block(self, "effects"); let (fx, _) = self.expr(fx)?; Ok((format!("addEffectToList({fx})"), Ty::Object)) }
            ("removeAllEffects", _) => { block(self, "effects"); Ok(("removeAllEffects()".into(), Ty::Any)) }
            ("clearEffectsOnStateChange", _) => { block(self, "effects"); Ok(("clearEffectsOnStateChange()".into(), Ty::Any)) }
            ("stopListening", []) => { block(self, "effects"); Ok(("stopListening()".into(), Ty::Any)) }
            // a stat it states: what its code set since, else what it declares
            ("getCharacterStat", [Expr::Str(k)]) => {
                let declared = (self.o.owner_declares)(k).unwrap_or_else(|| "null".into());
                let name = format!("ownStat_{}", ident(k));
                if !self.helpers.contains_key(&name) {
                    let def = helper(self.o.runtime, "helpers", "own_stat", &name, &[("STAT", k.clone()), ("DECLARED", declared)], &[])?;
                    self.helpers.insert(name.clone(), def);
                }
                Ok((format!("{name}()"), Ty::Any))
            }
            _ => Err(String::new()),
        } })()).filter(|r| !matches!(r, Err(e) if e.is_empty()))
    }

    /// an attack's stats its code writes as it plays: what this engine has a stat for is the
    /// animation's own stat on the spot (`runtime/attack_stats.jsonc`), its refresh rate restarts
    /// the count, and the rest is kept for the attack's rules and its code to read back
    fn attack_stats(&mut self, pairs: &[(Expr, Expr)]) -> Result<(String, Ty), String> {
        let table = super::attack_stats::table();
        let declared = self.o.jumps.get(&self.class).map(|j| j.declared.clone()).unwrap_or_default();
        let lit = |e: &Expr| -> Option<f64> { literal(e).and_then(|t| match t.as_str() { "true" => Some(1.0), "false" | "null" => Some(0.0), n => n.parse().ok() }) };
        let stated: Vec<(String, f64)> = pairs.iter().filter_map(|(k, v)| match k { Expr::Str(s) => lit(v).map(|n| (s.clone(), n)), _ => None }).collect();
        let along = |w: &str| stated.iter().find(|(x, _)| x == w).map(|(_, n)| *n).or_else(|| declared.get(w).copied());
        let mut native: Vec<(String, String)> = Vec::new();
        let mut ranks = BTreeMap::new();
        let mut after = Vec::new();
        for (k, v) in pairs {
            let Expr::Str(k) = k else { return Err("writes an attack stat by a name worked out as it runs".into()) };
            if k == "refreshRate" {
                self.blocks.insert("attack".into()); self.blocks.insert("refresh".into());
                let (v, _) = self.expr(v)?;
                after.push(format!("attackSetRefresh({v})"));
                continue;
            }
            match lit(v) {
                Some(n) if table.put(&mut native, &mut ranks, k, n, false, &along) => {
                    for c in table.clamps(k, n) { after.push(format!("if (self.getYSpeed() > {c}) self.setYSpeed({c})")); }
                }
                None if table.dynamic(k).is_some() => {
                    let (f, t) = table.dynamic(k).unwrap_or_default();
                    let test = self.test(v)?;
                    native.retain(|(g, _)| *g != f);
                    native.push((f, t.replace("(AS3.truthy({v})", &format!("({test}"))));
                }
                _ => {
                    // kept for the rules and its code
                    self.blocks.insert("attack".into());
                    let (val, _) = self.expr(v)?;
                    after.push(format!("attackWrite({}, {val})", quote(k)));
                }
            }
        }
        let mut out = Vec::new();
        if !native.is_empty() {
            let body: Vec<String> = native.iter().map(|(f, t)| format!("{f}: {t}")).collect();
            out.push(format!("self.updateAnimationStats({{ {} }})", body.join(", ")));
        }
        out.extend(after);
        Ok((out.join(";\n"), Ty::Any))
    }

    /// a jump by label on the clip it shows, from code that may run under any of them: the
    /// animation that shows the label in the clip of the animation it plays
    /// a jump to an animation's first frame (one already on it stays there)
    fn play_start(&mut self, anim: &str) -> Result<String, String> {
        if !self.helpers.contains_key("playStart") {
            let def = helper(self.o.runtime, "helpers", "play_start", "playStart", &[], &[])?;
            self.helpers.insert("playStart".into(), def);
        }
        Ok(format!("playStart({})", quote(anim)))
    }

    fn stance_jump(&mut self, label: &str) -> Result<String, String> {
        // (the clip being written is the one it shows: its own jump)
        if self.o.jumps.contains_key(&self.class) { return self.jump(label) }
        let name = "stanceJump".to_string();
        if !self.helpers.contains_key(&name) {
            self.play_start("")?;
            let mut lines = Vec::new();
            for (class, hits) in self.o.stance_labels {
                // (each clip, by the animations that show it: a label's animations, and whether it
                // is the first frame of each)
                let anims: Vec<String> = self.o.clip_frames.get(class).map(|v| v.iter().map(|(a, _, _)| quote(a)).collect()).unwrap_or_default();
                if anims.is_empty() { continue }
                lines.push(format!("case {}:", anims.join(", ")));
                lines.push("\tswitch (label) {".into());
                let mut by_label: BTreeMap<&str, Vec<(&str, bool)>> = BTreeMap::new();
                for (to, l, start) in hits { by_label.entry(l.as_str()).or_default().push((to.as_str(), *start)); }
                for (l, to) in by_label {
                    let names: Vec<String> = to.iter().map(|(a, _)| quote(a)).collect();
                    let starts: Vec<String> = to.iter().map(|(_, s)| s.to_string()).collect();
                    lines.push(format!("\t\tcase {}: goToLabel({}, [{}], [{}]);", quote(l), quote(l), names.join(", "), starts.join(", ")));
                }
                lines.push("\t}".into());
            }
            if lines.is_empty() { return Err(format!("jumps the clip it shows to {label}, and no clip of its shows a label")) }
            let def = helper(self.o.runtime, "helpers", "stance_jump", &name, &[], &lines)?;
            self.helpers.insert(name.clone(), def);
        }
        Ok(format!("stanceJump({})", quote(label)))
    }

    /// the runtime's own row for a call (`api_calls.jsonc`) where it is the engine's calls alone:
    /// it names nothing the runtime defines and asks nothing of the runtime's object
    fn engine_row(&self, m: &str) -> Option<Row> {
        let t = self.o.runtime.calls.get(m)?;
        // (one the runtime notes as a gap does nothing there: nothing here, and no value)
        if t.starts_with("PeptideRuntime.gap(") { return Some(Row { code: "null".into(), returns: Ty::Object }) }
        if ["{recv}", "{frame}", "{used}", "{own}"].iter().any(|x| t.contains(x)) { return None }
        // (a number the runtime makes sure is one: the source's code hands it one)
        let t = crate::re!(r"AS3\.num\((\{\d\})\)").replace_all(t, "$1").to_string();
        let rt = self.o.runtime;
        let reaches = crate::re!(r"\b([A-Za-z_]\w*)\.").captures_iter(&t).any(|c| {
            let n = &c[1];
            rt.globals.contains(n) && !rt.natives.contains_key(n) && !rt.engine_names.contains(n)
        });
        if reaches { return None }
        Some(Row { code: t.replace("{eng}", "{self}"), returns: Ty::Any })
    }

    /// a row filled in: the object it is asked of, its arguments, the units
    fn fill(&mut self, row: &Row, who: &str, args: &[Expr], m: &str) -> Result<(String, Ty), String> {
        let mut t = row.code.replace("{self}", who)
            .replace("{size}", &number(self.o.size)).replace("{speed}", &number(self.o.speed)).replace("{accel}", &number(self.o.accel));
        for (i, a) in args.iter().enumerate() {
            if t.contains(&format!("{{t{i}}}")) { let tv = self.test(a)?; t = t.replace(&format!("{{t{i}}}"), &paren(&tv)); }
            // (an argument the row does not take is not written)
            if !t.contains(&format!("{{{i}}}")) { continue }
            let (v, _) = self.expr(a)?;
            t = t.replace(&format!("{{{i}}}"), &paren(&v));
        }
        if t.contains("{0}") || t.contains("{1}") { return Err(format!("calls {m} with fewer arguments than its row takes")) }
        Ok((fold(&t), row.returns))
    }

    /// the function that fires a projectile its owner declares, as the source's engine fires one
    /// from another projectile's code: off where this one is, by the offset it declares and the
    /// one asked (`place` takes the one asked as the place itself), launched the way this one
    /// faces, belonging to the owner, and a source frame late (made in the projectiles' pass)
    fn fire(&mut self, name: &str, from: &str) -> Result<String, String> {
        let (content, [dx, dy, xs, ys]) = (self.o.fired)(name).ok_or_else(|| format!("fires {name}, which its owner does not declare"))?;
        let by_owner = from != "self";
        let f = if by_owner { format!("ownerFires_{}", ident(name)) } else { format!("fire_{}", ident(name)) };
        let from = if by_owner { "self.getOwner()" } else { "self" };
        if self.helpers.contains_key(&f) { return Ok(f) }
        let speed = number(self.o.speed);
        let words = [("PROJECTILE", name.to_string()), ("CONTENT", content), ("FROM", from.to_string()), ("DX", number(dx)), ("DY", number(dy)),
                     ("XS", fold(&format!("{} * {speed}", number(xs)))), ("YS", fold(&format!("{} * {speed}", number(ys)))), ("SIZE", number(self.o.size))];
        let def = helper(self.o.runtime, "helpers", if self.o.fighter { "fighter_fire" } else { "fire" }, &f, &words, &[])?;
        self.helpers.insert(f.clone(), def);
        Ok(f)
    }

    /// the source's name for a fighter state the code names (`CState.ATTACKING`, or its number)
    fn state_name(&self, st: &Expr) -> Result<String, String> {
        match st {
            Expr::Member(c, n) if matches!(&**c, Expr::Name(c) if short(c) == "PeptideCState") => Ok(n.clone()),
            Expr::Int(n) => self.o.runtime.constants.iter().find(|(k, v)| k.starts_with("PeptideCState.") && **v == n.to_string())
                .map(|(k, _)| k["PeptideCState.".len()..].to_string()).ok_or_else(|| "names a state by a number it has no name for".into()),
            _ => Err("tests a state worked out as it runs".into()),
        }
    }

    /// the container a stat of its own is kept in (one this engine has no counterpart for), from
    /// what it declares
    fn own_stat(&mut self, k: &str) -> String {
        let id = format!("stat_{}", ident(k));
        let start = (self.o.declared)(k).unwrap_or_else(|| "null".into());
        self.kept.insert(id.clone(), format!("// its {k}, which only its own code reads here\nvar {id} = self.makeObject({start});"));
        id
    }

    /// the container that holds the engine's id for the timer that runs one of its own methods
    fn timer_of(&mut self, f: &Expr) -> Result<String, String> {
        let name = match f {
            Expr::Member(t, n) if matches!(**t, Expr::This) => self.method(n),
            Expr::Name(n) => self.method(n),
            Expr::Function(c) => Some(self.closure(c)?),
            // (one of its own engine calls: a function that makes it)
            Expr::Member(t, n) if self.expr(t).is_ok_and(|(w, _)| w == "self") => {
                let (call, _) = self.api("self", n, &[])?;
                let name = format!("call_{}", ident(n));
                if !self.helpers.contains_key(&name) {
                    let def = helper(self.o.runtime, "helpers", "own_call", &name, &[("CALL", call)], &[])?;
                    self.helpers.insert(name.clone(), def);
                }
                Some(name)
            }
            _ => None,
        }.ok_or("times something other than its own method")?;
        let id = format!("{name}Timer");
        // (a fighter's is the record its timers block keeps, `startTimer`)
        let make = if self.o.fighter { "self.makeObject(null)" } else { "self.makeInt(-1)" };
        self.kept.insert(id.clone(), format!("// the engine's timer running {name}, while there is one\nvar {id} = {make};"));
        Ok(id)
    }

    /// a source timer (`createTimer(frames, repeats, fn)`): the engine's, of two engine frames to
    /// a source frame, forever for none. one function keeps one timer: starting it again restarts it
    fn timer(&mut self, frames: &Expr, repeat: &Expr, f: &Expr, rest: &[Expr]) -> Result<(String, Ty), String> {
        // (its timers are kept whatever it asks: `persistent` says nothing more here)
        let plain_opts = |e: &Expr| matches!(e, Expr::Null | Expr::Undefined | Expr::Bool(_)) || matches!(e, Expr::Object(kv) if kv.iter().all(|(k, _)| matches!(k, Expr::Str(k) if k == "persistent" || k == "hitStunPause" || k == "pauseHitstun" || k == "inverseCondition")));
        // (`inverseCondition` the runtime keeps no more of than this does)
        // (one held while it is frozen by a hit: the engine's freeze pauses it)
        let paused = rest.iter().any(|e| matches!(e, Expr::Object(kv) if kv.iter().any(|(k, v)| matches!(k, Expr::Str(k) if k == "pauseHitstun") && matches!(v, Expr::Bool(true)))));
        if !rest.is_empty() && !rest.iter().all(plain_opts) { return Err("makes a timer with options".into()) }
        let id = self.timer_of(f)?;
        let name = id.trim_end_matches("Timer").to_string();
        let (n, _) = self.expr(frames)?;
        let every = match n.parse::<f64>() { Ok(v) => number((2.0 * v.max(1.0)).round()), Err(_) => format!("Std.int(2 * Math.max(1, {}))", paren(&n)) };
        let times = match self.expr(repeat)?.0 {
            t if t.parse::<f64>().is_ok_and(|v| v <= 0.0) => "-1".to_string(),
            t if t.parse::<f64>().is_ok() => t,
            t => format!("({t} <= 0 ? -1 : {t})"),
        };
        if self.o.fighter {
            // a fighter's timer drops with the state it was made in unless asked to stay, waits out its
            // freezes, and, made in a frame's code, comes due two engine frames sooner the first time
            // (the source ticks the frame it is made in too)
            let _ = paused;
            let keep = rest.iter().any(|e| matches!(e, Expr::Bool(true)) || matches!(e, Expr::Object(kv) if kv.iter().any(|(k, v)| matches!(k, Expr::Str(k) if k == "persistent") && matches!(v, Expr::Bool(true)))));
            self.blocks.insert("timers".into());
            self.blocks.insert("refresh".into());
            let n = match n.parse::<f64>() { Ok(v) => number(v.max(1.0).round()), Err(_) => format!("Std.int(Math.max(1, {}))", paren(&n)) };
            return Ok((format!("stopTimer({id}.get());\n{id}.set(startTimer({n}, {times}, {name}, {keep}, {}))", self.in_frame), Ty::Any));
        }
        let opts = "{ persistent: true }";
        Ok((format!("self.removeTimer({id}.get());\n{id}.set(self.addTimer({every}, {times}, {name}, {opts}))"), Ty::Any))
    }

    /// a listener for an event the engine raises, on the engine's object, by one of its own methods
    fn listener(&mut self, who: &str, m: &str, ev: &Expr, handler: &Expr) -> Result<(String, Ty), String> {
        let name = super::events::named(self.o.runtime, ev).ok_or("listens for an event named as it runs")?;
        if name == "charAttackComplete" && who == "self" && self.o.fighter {
            let h = self.handler(handler)?;
            self.blocks.insert("attack".into());
            return Ok((if m == "addEventListener" { format!("attackDone.get().remove({h});\nattackDone.get().push({h})") } else { format!("attackDone.get().remove({h})") }, Ty::Any));
        }
        // meeting its owner: told by its own frame check, through a note of whether it listens
        if matches!(name.as_str(), "projCollide" | "projCollideOwner") && who == "self" && !self.o.fighter {
            let h = self.handler(handler)?;
            let listening = format!("{h}Touching");
            self.touches.insert(h.clone(), listening.clone());
            self.kept.insert(listening.clone(), format!("// whether {h} listens for meeting its owner now\nvar {listening} = self.makeBool(false);"));
            return Ok((format!("{listening}.set({})", m == "addEventListener"), Ty::Any));
        }
        if matches!(name.as_str(), "projDestroyed" | "itemDestroyed" | "enemyDestroyed") && who != "self" && self.o.fighter {
            let h = self.handler(handler)?;
            return Ok((if m == "addEventListener" { format!("{who}.addEventListener(EntityEvent.DISPOSE, {h}, {{ persistent: true }})") } else { format!("{who}.removeEventListener(EntityEvent.DISPOSE, {h})") }, Ty::Any));
        }
        let ends = name == "projDestroyed" && who == "self";
        let table = super::events::table();
        let raised = table.engine.get(&name).or_else(|| table.state.get(&name));
        // (one neither this engine nor the runtime ever raises is heard by no one on either path)
        if raised.is_none() && !ends && !self.o.runtime.files.values().any(|t| t.contains(&format!("\"{name}\""))) {
            return Ok((format!("// (it listens for the source's {name}, which nothing here raises)"), Ty::Any));
        }
        let engine = if ends { &name } else { raised.ok_or_else(|| format!("listens for {name}, which this engine does not raise"))? };
        let h = self.handler(handler)?;

        // its own end: told from the engine's `onKill`, through a note of whether it listens
        if ends {
            let listening = format!("{h}Listening");
            self.ends.insert(h.clone(), listening.clone());
            self.kept.insert(listening.clone(), format!("// whether {h} listens for its end now\nvar {listening} = self.{}(false);", bool_make(self.o.fighter)));
            return Ok((format!("{listening}.set({})", m == "addEventListener"), Ty::Any));
        }
        // its own floor listener: told the source's way (`runtime/native/floor.hx`), through a
        // function that runs it with the speed it came down at, and a note of whether it listens
        if who == "self" && engine.contains("COLLIDE_FLOOR") {
            let listening = format!("{h}Listening");
            // (told plainly, the listener is itself; told with the speed it came down at, a function that does)
            let wrapper = if self.floor_speed { format!("{h}OnFloor") } else { h.clone() };
            self.floor.insert(h.clone(), listening.clone());
            self.kept.insert(listening.clone(), format!("// whether {h} listens for the floor now\nvar {listening} = self.{}(false);", bool_make(self.o.fighter)));
            if self.floor_speed {
                let def = helper(self.o.runtime, "helpers", "floor_listener", &wrapper, &[("LISTENER", h.clone())], &[])?;
                self.helpers.insert(wrapper.clone(), def);
            }
            let owned = self.clip_listen(engine, &wrapper);
            return Ok((if m == "addEventListener" {
                format!("self.addEventListener({engine}, {wrapper}, {{ persistent: true }});\n{listening}.set(true){owned}")
            } else {
                format!("self.removeEventListener({engine}, {wrapper});\n{listening}.set(false)")
            }, Ty::Any));
        }
        // (a note of whether it listens, for its code's question: `hasEventListener`)
        let note = if who == "self" && self.o.fighter {
            let listening = format!("{h}Listening");
            self.kept.insert(listening.clone(), format!("// whether {h} listens now\nvar {listening} = self.{}(false);", bool_make(self.o.fighter)));
            format!(";\n{listening}.set({})", m == "addEventListener")
        } else { String::new() };
        let owned = if who == "self" { self.clip_listen(engine, &h) } else { String::new() };
        Ok((if m == "addEventListener" {
            format!("{who}.addEventListener({engine}, {h}, {{ persistent: true }}){note}{owned}")
        } else {
            format!("{who}.removeEventListener({engine}, {h}){note}")
        }, Ty::Any))
    }

    /// a fighter's listener added by one of its stance clips' code: noted as that clip's, so it goes
    /// when the fighter leaves the clip (`fresh_clip`). its own class and root clip stay
    fn clip_listen(&mut self, engine: &str, h: &str) -> String {
        if !self.o.fighter || self.is_itself() { return String::new() }
        if self.o.model.class_of(self.o.root).is_some_and(|c| c.name == self.class) { return String::new() }
        self.blocks.insert("fresh_clip".into());
        format!(";\nclipListen({}, {engine}, {h})", quote(&self.class))
    }

    /// the function a listener (or timer) it hands over is: one of its own methods, a closure, or
    /// one of its own API calls (`self.self.destroy`), as a function that makes that call
    fn handler(&mut self, handler: &Expr) -> Result<String, String> {
        let own = match handler {
            Expr::Member(t, n) if matches!(**t, Expr::This) => self.method(n),
            Expr::Name(n) => self.method(n),
            Expr::Function(f) => Some(self.closure(f)?),
            _ => None,
        };
        if let Some(h) = own { return Ok(h) }
        let Expr::Member(o, n) = handler else { return Err("listens with something other than its own method".into()) };
        if self.expr(o)?.0 != "self" { return Err("listens with another object's call".into()) }
        if let Some(h) = self.itself_method(n) { return Ok(h) }
        let name = format!("{}OnEvent", ident(n));
        if self.helpers.contains_key(&name) { return Ok(name) }
        let code = if self.o.fighter {
            match self.fighter_api(n, &[]) { Some(r) => r?.0, None => {
                let row = calls().rows.get(n).cloned().or_else(|| self.engine_row(n)).filter(|r| !r.code.contains("{0}"))
                    .ok_or_else(|| format!("listens with its own {n}, which has no row in native_calls.jsonc"))?;
                self.fill(&row, "self", &[], n)?.0
            } }
        } else {
            let row = calls().rows.get(n).cloned().or_else(|| self.engine_row(n)).filter(|r| !r.code.contains("{0}"))
                .ok_or_else(|| format!("listens with its own {n}, which has no row in native_calls.jsonc"))?;
            self.fill(&row, "self", &[], n)?.0
        };
        let def = helper(self.o.runtime, "helpers", "call_listener", &name, &[("CALL", n.clone())], &[format!("{code};")])?;
        self.helpers.insert(name.clone(), def);
        Ok(name)
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
            if self.own_stats.contains(k) {
                let id = self.own_stat(k);
                let (val, _) = self.expr(v)?;
                out.push(format!("{id}.set({val})"));
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

// ---- the helpers' text: runtime/native/helpers.hx ------------------------------------------------

/// a block of `runtime/native/helpers.hx` (or of another native file, `file`), as a function named
/// `name`: its capitalised `words` filled in, its `// {cases}` line replaced by `cases`, and its
/// `//:` notes for whoever edits it left out
pub fn helper(runtime: &super::runtime::Runtime, file: &str, key: &str, name: &str, words: &[(&str, String)], cases: &[String]) -> Result<String, String> {
    let text = runtime.native_files.get(file).ok_or_else(|| format!("the runtime has no native/{file}.hx"))?;
    let marker = format!("// ---- {key}");
    let start = text.lines().position(|l| l.trim_end() == marker).ok_or_else(|| format!("native/{file}.hx has no {key}"))?;
    let block: Vec<&str> = text.lines().skip(start + 1).take_while(|l| !l.starts_with("// ---- ")).filter(|l| !l.starts_with("//:")).collect();
    let mut out = block.join("\n").trim().to_string();
    let mut words: Vec<(&str, String)> = words.to_vec();
    words.push(("NAME", name.to_string()));
    words.sort_by_key(|(w, _)| std::cmp::Reverse(w.len()));
    for (w, v) in &words { out = crate::dyn_re(&format!(r"\b{w}\b")).replace_all(&out, regex::NoExpand(v)).to_string(); }
    let mut slots = BTreeMap::new();
    slots.insert("cases", cases.to_vec());
    Ok(fill_slots(&out, &slots).trim_end().to_string())
}

/// a text with each `// {slot}` line replaced by the slot's lines, at that line's indent (an
/// empty slot takes its line out)
pub fn fill_slots(text: &str, slots: &BTreeMap<&str, Vec<String>>) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let t = line.trim_start();
        let slot = t.strip_prefix("// {").and_then(|r| r.strip_suffix('}')).filter(|n| slots.contains_key(n) || !n.contains(' '));
        match slot {
            Some(name) => {
                let indent = &line[..line.len() - t.len()];
                for l in slots.get(name).into_iter().flatten() {
                    if l.is_empty() { out.push('\n') } else { out.push_str(&format!("{indent}{l}\n")) }
                }
            }
            None => { out.push_str(line); out.push('\n'); }
        }
    }
    out
}

// ---- small pieces -----------------------------------------------------------------------------

/// a declared value as a plain Haxe value (a record's fields named, a class by its name)
fn plain_value(v: &crate::model::declare::Value) -> String {
    use crate::model::declare::Value;
    match v {
        Value::Null | Value::Computed(_) => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => number(*n),
        Value::Str(s) | Value::Class(s) => quote(s),
        Value::Arr(items) => format!("[{}]", items.iter().map(plain_value).collect::<Vec<_>>().join(", ")),
        Value::Obj(kv) => {
            let parts: Vec<String> = kv.iter().filter(|(k, _)| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.starts_with(|c: char| c.is_ascii_digit()))
                .map(|(k, v)| format!("{k}: {}", plain_value(v))).collect();
            if parts.is_empty() { "{}".into() } else { format!("{{ {} }}", parts.join(", ")) }
        }
    }
}

/// what a value is that is one of two: what they agree on, the one known where the other is not,
/// and a rectangle where the other is only some object (or none)
fn merge(a: Ty, b: Ty) -> Ty {
    match (a, b) {
        _ if a == b => a,
        (Ty::Any, t) | (t, Ty::Any) => t,
        (Ty::Rect, Ty::Object) | (Ty::Object, Ty::Rect) => Ty::Rect,
        (Ty::Body, Ty::Object) | (Ty::Object, Ty::Body) => Ty::Body,
        _ => Ty::Any,
    }
}

/// what a field holds that its code fills with both: the same where they agree, an object where a
/// number or a flag is later an object (a sound it played), the more particular object, and
/// anything otherwise (a flag it later counts with)
fn join(a: Ty, b: Ty) -> Ty {
    match (a, b) {
        _ if a == b => a,
        (Ty::Any, _) | (_, Ty::Any) => Ty::Any,
        (Ty::Num | Ty::Bool | Ty::Str, Ty::Object) | (Ty::Object, Ty::Num | Ty::Bool | Ty::Str) => Ty::Object,
        (Ty::Object, t @ (Ty::Rect | Ty::Body)) | (t @ (Ty::Rect | Ty::Body), Ty::Object) => t,
        _ => Ty::Any,
    }
}

/// a class's member, as the writer keeps it: each class's members are its own
fn member_key(class: &str, n: &str) -> String { format!("{class}::{n}") }

/// a method's name as the writer keeps it: an accessor's says which it is (`get:x`, `set:x`)
fn method_name(m: &crate::model::MethodModel) -> String {
    match m.kind { MethodKind::Getter => format!("get:{}", m.name), MethodKind::Setter => format!("set:{}", m.name), MethodKind::Method => m.name.clone() }
}
/// a method's function's name: an accessor's says which it is (`get_x`, `set_x`)
fn accessor_name(m: &crate::model::MethodModel) -> String {
    match m.kind { MethodKind::Getter => format!("get_{}", m.name), MethodKind::Setter => format!("set_{}", m.name), MethodKind::Method => m.name.clone() }
}

fn is_itself(v: &Expr) -> bool {
    match v {
        Expr::Coerce(_, x) | Expr::Convert(_, x) | Expr::Binary(BinOp::As, x, _) => is_itself(x),
        Expr::Call(f, a) => matches!(&**f, Expr::Member(c, m) if matches!(&**c, Expr::Name(n) if n.contains("PeptideAPI"))
            && matches!(m.as_str(), "getProjectile" | "getItem" | "getEnemy" | "getCharacter")) && matches!(a.as_slice(), [Expr::This]),
        _ => false,
    }
}

/// a method that only returns a record: a declaration the conversion has read already
/// (`getOwnStats`, `getAttackStats`), not code that runs
fn declares(m: &crate::model::MethodModel) -> bool {
    // (built up in its variables first, then returned)
    let body: Vec<&Stmt> = m.function.body.iter().filter(|s| !matches!(s, Stmt::Return(None))).collect();
    let Some((last, before)) = body.split_last() else { return false };
    let mut e = match last { Stmt::Return(Some(e)) => e, _ => return false };
    while let Expr::Coerce(_, x) | Expr::Convert(_, x) = e { e = x; }
    // (or a variable it built the record in)
    let built = |r: u32| before.iter().any(|s| matches!(s, Stmt::Assign(Place::Local(x), Expr::Object(_) | Expr::Coerce(_, _)) if *x == r));
    m.function.params.is_empty() && (matches!(e, Expr::Object(_)) || matches!(e, Expr::Local(r) if built(*r)))
        && before.iter().all(|s| matches!(s, Stmt::Assign(Place::Local(_), _)))
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
    // (times or over 1, plus 0, times -1)
    let one = crate::re!(r" [*/] 1([^.\d\w]|$)");
    let out = one.replace_all(&out, "$1").to_string();
    let zero = crate::re!(r" [+-] 0([);,\]}]|$)");
    let out = zero.replace_all(&out, "$1").to_string();
    out.replace("(-1) * ", "-")
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

/// a case's body without the `break` that ends it (a case here never runs on into the next). a
/// break anywhere else in it, outside a loop of its own, leaves the switch in the source and
/// would leave a loop around it here
fn strip_break(body: &[Stmt]) -> Result<&[Stmt], String> {
    let body = match body.split_last() { Some((Stmt::Break, rest)) => rest, _ => body };
    fn breaks(list: &[Stmt]) -> bool {
        list.iter().any(|s| match s {
            Stmt::Break => true,
            Stmt::If(_, a, b) => breaks(a) || breaks(b),
            Stmt::Switch(..) | Stmt::Loop(_) | Stmt::While(..) | Stmt::DoWhile(..) => false,
            Stmt::Try(b, cs) => breaks(b) || cs.iter().any(|c| breaks(&c.body)),
            _ => false,
        })
    }
    if breaks(body) { return Err("a case it leaves part way".into()) }
    Ok(body)
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

/// a record its code puts in a field and hands to a call further on in the same run of statements
/// (`newStats = { damage: 2 }; updateAttackBoxStats(1, newStats)`) is that record, written there
fn records(body: &mut [Stmt]) {
    let mut held: BTreeMap<String, Expr> = BTreeMap::new();
    for s in body.iter_mut() {
        match s {
            Stmt::Assign(Place::Member(Expr::This, f), v) => {
                let mut v = &*v;
                while let Expr::Coerce(_, x) = v { v = x; }
                if matches!(v, Expr::Object(_)) { held.insert(f.clone(), v.clone()); } else { held.remove(f); }
            }
            Stmt::Expr(Expr::Call(_, args)) => {
                for a in args.iter_mut() {
                    if let Expr::Member(t, f) = a { if matches!(**t, Expr::This) { if let Some(r) = held.get(f) { *a = r.clone(); } } }
                }
            }
            Stmt::Expr(_) => {}
            _ => held.clear(),
        }
    }
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

/// the container a note of true or false is kept in: a fighter's an object (the engine keeps few
/// true-or-false containers for a whole match)
fn bool_make(fighter: bool) -> &'static str { if fighter { "makeObject" } else { "makeBool" } }

/// what a frame's `return` is written as in its keyframe, for the emitter to finish (a frame's code
/// ends with what the rules run after it, which a `return` must run too)
pub const FRAME_RETURN: &str = "return; //frame";

/// a frame's function's body, as its keyframe's code: its lines, a tab less
fn frame_body(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 { return String::new() }
    lines[1..lines.len() - 1].iter().map(|l| l.strip_prefix('\t').unwrap_or(l)).collect::<Vec<_>>().join("\n")
}
