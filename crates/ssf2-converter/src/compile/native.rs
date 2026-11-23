//! An object's own code written as plain Fraymakers code.
//!
//! A package ships a class for each clip with code: the fighter's, a projectile's, a stage's and
//! its objects'. The class is written straight into the object's script: its fields are the
//! engine's state containers, its methods are the script's functions, its frame scripts are
//! functions the entity's keyframes call, and every API call is the engine's own
//! (`runtime/native_calls.jsonc`).
//!
//! A statement it has no plain way to say (a call with no row, a value worked out a way it cannot
//! follow) is left out of the script with a `(not ported: why)` note, and the reason joins the
//! object's gaps; the rest of its code is written as it is. The question is always what the code
//! does, never what the object is.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use crate::model::ast::{BinOp, Conv, Expr, Function, Place, Stmt, UnOp};
use crate::model::{ClassModel, MethodKind, SourceModel};

use super::{ident, quote, short, ClipJumps};

// ---- the call table ---------------------------------------------------------------------------

/// a row of `native_calls.jsonc`: the engine's code for a call, and what its value is
#[derive(Debug, Clone, Default)]
/// a row: the call's code, what its value is, and the block of `runtime/native/utils.hx` its code
/// calls, where it calls one
struct Row { code: String, returns: Ty, helper: Option<String>, gap: Option<String> }

#[derive(Debug, Default)]
struct Calls {
    rows: BTreeMap<String, Row>,
    /// the source's names for the kinds of object (`getType()`), as this engine's
    types: BTreeMap<String, String>,
    /// a projectile stat written by its code, as the engine's own stat (`{v}` the value)
    projectile_stats: BTreeMap<String, String>,
    /// an item stat written by its code, as the engine's stat (`{v}` the value)
    item_stats: BTreeMap<String, String>,
    /// an item stat its own code reads that its handler keeps as it runs (`{declared}` what it declares)
    item_stat_reads: BTreeMap<String, String>,
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
            if k == "projectile_stats" || k == "item_stats" || k == "item_stat_reads" {
                let m: BTreeMap<String, String> = v.as_object().into_iter().flatten()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
                match k.as_str() { "projectile_stats" => t.projectile_stats = m, "item_stats" => t.item_stats = m, _ => t.item_stat_reads = m }
                continue;
            }
            let row = match v {
                serde_json::Value::String(s) => Row { code: s.clone(), returns: Ty::Any, helper: None, gap: None },
                serde_json::Value::Object(o) => Row {
                    code: o.get("code").and_then(|c| c.as_str()).unwrap_or_default().to_string(),
                    returns: Ty::named(o.get("returns").and_then(|r| r.as_str()).unwrap_or("")),
                    helper: o.get("helper").and_then(|h| h.as_str()).map(String::from),
                    gap: o.get("gap").and_then(|h| h.as_str()).map(String::from),
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
pub enum Ty { Bool, Num, Str, Object, Rect, Body, Controls, #[default] Any }

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
    pub fired: &'a dyn Fn(&str) -> Option<(String, [f64; 4], Option<String>)>,
    /// a stat its owner declares, as code
    pub owner_declares: &'a dyn Fn(&str) -> Option<String>,
    /// whether its owner's code keeps a field in a state container (its package's code stores it),
    /// for a read or write of the owner's field: the owner's script publishes its object, and keeps
    /// a stored field as `__c_<name>` on it
    pub owner_stores: &'a dyn Fn(&str) -> bool,
    /// the clip the object is (its root): a class of it has no clip of the object's above it
    pub root: u16,
    pub size: f64,
    pub speed: f64,
    pub accel: f64,
    /// whether it is a fighter (its own class extends the API's character)
    pub fighter: bool,
    /// whether it is an item (a custom game object carrying `runtime/native/item.hx`'s handler)
    pub item: bool,
    /// whether it is a stage (its own class extends the API's stage)
    pub stage: bool,
    /// a stage's: the name each clip class it runs is placed under in its planes (its `self` is that
    /// named clip of the planes, `planePart`)
    pub clip_parts: BTreeMap<String, String>,
    /// a stage's: the content each class its code makes an object of was converted to, by the
    /// class's short name (`makeEnemy(CastleSiegeStatue1)`)
    pub made_objects: BTreeMap<String, String>,
    /// a stage's: each weather particle's clip class, by its short name, as the effect it is drawn as
    /// and the size of its clip (`clipRecord`)
    pub particles: BTreeMap<String, (String, f64, f64)>,
    /// a stage's (and its objects'): each library clip's extent the package measured, by its name
    /// (source units: x, y, width, height), for a floor its code makes the shape of a clip
    pub clip_bounds: BTreeMap<String, (f64, f64, f64, f64)>,
    /// a fighter's: the animation each of its clip's labels is (an attack its code starts by name)
    pub root_anim: &'a dyn Fn(&str) -> Option<String>,
    /// a fighter's: the walk's own starting speed, in this engine's units, where it declares one
    pub walk_start: Option<f64>,
    /// a fighter's: every animation that shows a label of a clip, by clip class (a jump its code
    /// asks of the clip it shows, whichever that is)
    pub stance_labels: &'a BTreeMap<String, Vec<(String, String, bool)>>,
    /// a fighter's: the clip frames each animation shows (its first, how many), by clip class
    pub clip_frames: &'a BTreeMap<String, Vec<(String, u32, u32)>>,
    /// a fighter's: the animations the engine links at the same frame, each way (it goes from one
    /// to the other as it lands or leaves the ground mid-move, keeping the frame)
    pub frame_links: &'a BTreeSet<(String, String)>,
    /// a fighter's: the clip each animation shows, as (animation, clip symbol, first frame, how many)
    pub anim_clips: &'a [(String, u16, u32, u32)],
    /// a projectile's: whether its owner is written as Fraymakers code (its fields are then reached
    /// through what it publishes, `exports.__fieldOf`)
    pub owner_native: bool,
    /// every effect it can make, by name (one its code names as it runs is any of these)
    pub effect_names: Vec<String>,
    /// each effect's animations, as (effect, animation, first clip frame, how many)
    pub effect_pieces: Vec<(String, String, u32, u32)>,
    /// a fighter's parts: the named clips its code turns, drawn apart from its animations' art
    pub parts: Vec<Part>,
    /// the name the tree placed its clip under (a stage object's `getMC().name`)
    pub instance: Option<String>,
    /// where the source's origin sits in the engine's space, in the source's units: a stage's code and
    /// its objects' read and write places in the stage's own space (none for a fighter's)
    pub origin: (f64, f64),
    /// the content its own animations are (a part's art is one of them); a projectile's: its own
    /// name, as its owner declares it
    pub content: String,
    /// its costumes' colours: the colours its art is drawn in, and what each costume (in the
    /// engine's costume order) turns each into, as ARGB
    pub palette: Option<(Vec<u32>, Vec<Vec<u32>>)>,
    /// the clips each effect places that its code turns, by the effect's name
    pub effect_inners: BTreeMap<String, Vec<crate::entity_gen::InnerClip>>,
    /// what each effect's clip's own frames do, by the effect's name (`emit::effect_code`)
    pub effect_code: BTreeMap<String, Vec<(String, u32, String)>>,
    /// whether it publishes a power for a fighter that swallows it: its effects, and the clips
    /// inside them, are written for that one to ask for by name
    pub power_record: bool,
    /// an object's parts: the named children of its clip its code turns, drawn apart (`projPart`)
    pub own_parts: Vec<crate::emit::ObjPart>,
    /// the clips with code of their own nested in its art, by class: each a record whose timeline
    /// runs as its parent's places it (`nestedClip`)
    pub nested: Vec<String>,
    /// the projectiles an item declares it fires, published for its holder's code to fire through
    /// it (`__fire`: a ray gun a fighter swings shoots its own shot)
    pub shots: Vec<String>,
}

/// one of a fighter's parts: how its code names it, and each animation that places it, as
/// (animation, its art there, x, y, rotation, scale x, scale y) in the source's units
#[derive(Clone, Default)]
pub struct Part {
    pub name: String,
    pub placed: Vec<(String, String, f64, f64, f64, f64, f64)>,
}

/// whether a clip is only art: no class, or one whose frames' code moves nothing but its own
/// playhead (a loop held on its frame). its art is drawn into its parent's, so turning it shows
/// nothing here either way
fn art_only(model: &SourceModel, sym: u16) -> bool {
    let Some(c) = model.class_of(sym) else { return true };
    c.frame_scripts.iter().all(|fs| c.methods.iter().find(|m| m.name == fs.method).is_some_and(|m| m.function.body.iter().all(|st| match st {
        Stmt::Return(None) => true,
        Stmt::Expr(Expr::Call(f, _)) => matches!(&**f, Expr::Member(t, m) if matches!(**t, Expr::This) && matches!(m.as_str(), "gotoAndStop" | "gotoAndPlay" | "stop" | "play" | "nextFrame" | "prevFrame"))
            || matches!(&**f, Expr::Name(m) if matches!(m.as_str(), "gotoAndStop" | "gotoAndPlay" | "stop" | "play" | "nextFrame" | "prevFrame")),
        _ => false,
    })))
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
    /// what it takes back as it ends: the listeners it put on other objects it keeps in its fields
    pub release: Vec<String>,
    /// its listeners for meeting its owner, the same way: told by `runtime/native/touch.hx`
    pub touches: Vec<(String, String)>,
    /// what its own class runs as it starts (its constructor's work, then its `initialize`) and
    /// each of the source's frames (its `update`)
    pub start: Vec<String>,
    pub tick: Vec<String>,
    /// what it runs each of this engine's frames (a part riding it)
    pub every_frame: Vec<String>,
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
    /// what the source does that nothing here does
    pub gaps: BTreeSet<String>,
    /// the animations its code jumps into at a label inside them: their first frame's own code
    /// waits out such a jump (`playLabelIn`)
    pub mid_targets: BTreeSet<String>,
}

/// whether a method of an object's classes is ever run: a frame script, one of the API's hooks, or
/// named anywhere in the package's code (a call, a listener, a timer). one never named is a
/// template's leftover, and is not written
fn reached(o: &Object, c: &ClassModel, m: &crate::model::MethodModel) -> bool {
    if c.frame_scripts.iter().any(|fs| fs.method == m.name) { return true }
    if matches!(m.name.as_str(), "initialize" | "update") { return true }
    // (an accessor is run by a read or a write of its property, which a count of names misses)
    if matches!(m.kind, MethodKind::Getter | MethodKind::Setter) { return true }
    // (a parent's method its class calls up to, merged in under its own name: called by that call)
    if m.name.ends_with("__super") { return true }
    // (another object's own method the API calls on it by name: the native skeletons name it)
    if !o.fighter && o.runtime.files.values().any(|f| f.contains(&format!(".{}", m.name)) || f.contains(&format!("\"{}\"", m.name))) { return true }
    let model = o.model;
    // each name's mentions, less those as the listener for an event nothing raises (never told,
    // so never run)
    type Counts = BTreeMap<String, i64>;
    static NAMED: OnceLock<std::sync::Mutex<BTreeMap<usize, Counts>>> = OnceLock::new();
    let key = model as *const SourceModel as usize;
    let mut cache = NAMED.get_or_init(Default::default).lock().unwrap();
    let names = cache.entry(key).or_insert_with(|| {
        let mut out = Counts::new();
        for k in model.classes.values() {
            for f in k.methods.iter().map(|m| &m.function).chain([&k.constructor, &k.static_init]) {
                crate::model::ast::walk_exprs(&f.body, &mut |e| match e {
                    Expr::Member(_, n) | Expr::Name(n) | Expr::Str(n) => { *out.entry(n.clone()).or_default() += 1; }
                    Expr::Call(g, a) if matches!(&**g, Expr::Member(_, l) | Expr::Name(l) if l == "addEventListener" || l == "removeEventListener") => {
                        let unheard = a.first().and_then(|ev| super::events::named(o.runtime, ev)).is_some_and(|n| super::events::raised_by_nothing(o.runtime, &n));
                        if let (true, Some(Expr::Member(_, h) | Expr::Name(h))) = (unheard, a.get(1)) { *out.entry(h.clone()).or_default() -= 1; }
                    }
                    _ => {}
                });
            }
        }
        out
    });
    names.get(&m.name).is_some_and(|n| *n > 0)
}

thread_local! {
    /// whether the object being written is a fighter (a clip's `getCharacter(this)` is then the
    /// fighter itself; in another object's clip it is the fighter that made the object)
    static WRITING_FIGHTER: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Write the object's classes as Fraymakers code, or say why they cannot be.
pub fn write(o: &Object) -> Result<Written, String> {
    WRITING_FIGHTER.with(|c| c.set(o.fighter || o.stage));
    let mut w = Writer::new(o)?;
    w.learn_fields();
    if o.power_record {
        if !o.effect_names.is_empty() { w.effect_art()?; }
        if !o.effect_inners.is_empty() { w.inner_clips()?; w.effect_jump()?; }
    }
    // (an object with parts draws them from its start, `projPart`)
    if !o.own_parts.is_empty() && !o.fighter && !o.stage { w.util("projPart")?; }
    w.write()
}

/// Every reason the object's code is not written yet, one per function it has no plain form for
/// (`class.method: why`): the work list for taking it off the runtime.
pub fn census(o: &Object) -> Vec<String> {
    let mut w = match Writer::new(o) { Ok(w) => w, Err(e) => return vec![e] };
    w.learn_fields();
    w.strict = true;
    let mut out = Vec::new();
    let classes: Vec<&ClassModel> = o.itself.into_iter().chain(o.classes.iter().copied()).collect();
    for c in classes {
        w.class = c.name.clone();
        for m in c.methods.iter().filter(|m| !declares(m) && reached(o, c, m)) {
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
    /// a field that only ever holds a new one of a class `runtime/native/classes.hx` writes out,
    /// flattened into the script under the field's name: field -> the class
    flat: BTreeMap<String, String>,
    /// the flattened fields written so far: its name in the script -> the class
    flat_used: BTreeMap<String, String>,
    /// the package's own classes an object holds one of in a field (`new InhaleController(this)`),
    /// written out under that field's name: each class, and the field's name it is written under
    pkg: Vec<&'a ClassModel>,
    pkg_ids: BTreeMap<String, String>,
    /// the package's own classes it keeps several of (a list of them): each one a record, its
    /// methods functions handed the record (`me`); the lists of them its fields keep, by field; and
    /// the one being written
    recs: Vec<&'a ClassModel>,
    rec_lists: BTreeMap<String, String>,
    /// a field that holds one record of a class of the package (`K_new`): its class
    rec_fields: BTreeMap<String, String>,
    /// lists of one of the API's classes with a record form (`classes.hx`, `K as a record`), by
    /// field: each element a record, its calls `K_call(element)`
    rt_lists: BTreeMap<String, String>,
    /// a field of the records its code writes out in place that every one of them fills with one of
    /// the API's classes with a record form (`{ timer: new FrameTimer(9) }`): that class
    lit_rt: BTreeMap<String, String>,
    /// fields of its classes that keep one of the API's classes with a record form as a record,
    /// where its code uses one as a value (a timer the code points another field at): that class
    rt_fields: BTreeMap<String, String>,
    /// list fields whose elements are lists its code pushes only records onto (learned)
    nested_recs: BTreeMap<String, bool>,
    /// a method's parameters every one of its calls hands a clip of the planes (learned)
    part_params: BTreeMap<(String, usize), bool>,
    /// the most arguments any of its own calls hands each of its methods (learned)
    max_args: BTreeMap<String, usize>,
    /// clip classes of its package it makes with `new` whose frames have code: records whose
    /// timeline runs (`clipRun`), their frame scripts handed the record
    clip_recs: BTreeSet<String>,
    /// handlers its code listens on another object with: the event's caller is that object there
    foreign_handlers: BTreeSet<String>,
    /// whether the listener being written is already primed with what its damage is (`damageStep`)
    primed: bool,
    /// list fields its code fills only with clips of the planes (or nothing): true while every one is
    part_lists: BTreeMap<String, bool>,
    /// variables that hold a clip of the planes now
    part_locals: BTreeSet<u32>,
    /// names its code writes onto clips of the planes that no display property is: fields of its own
    /// it keeps on them (`mirror.x_offset`)
    part_written: BTreeSet<String>,
    record: Option<String>,
    /// the variables of the function being written that hold a record of a list: the class, and
    /// the list's field
    rec_locals: BTreeMap<u32, (String, String)>,
    /// the variables of the function that hold the stage now (from the write that set them)
    stage_locals: BTreeSet<u32>,
    /// the variables of the function that hold a field of its own no code fills (`never_filled`)
    empty_locals: BTreeSet<u32>,
    /// the variables of the function that hold the object itself now, and what they were set from
    self_locals: BTreeMap<u32, Expr>,
    /// what each variable of the function was last set to, as written so far (what a loop over a
    /// variable's names walks: a list or a map)
    local_src: BTreeMap<u32, Expr>,
    /// its listeners on the game for the end of each of its frames (`gameTickEnd`), and the note of
    /// whether each listens now
    game_ticks: BTreeMap<String, String>,
    /// what it runs each of this engine's frames besides (the clips its effects place, followed)
    every_frame_extra: BTreeSet<String>,
    /// a value read already for the write being made (`assign`, a field of another class's)
    given: Option<(String, Ty)>,
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
    /// what the source does that nothing here does, noted for the report
    gaps: BTreeSet<String>,
    /// a statement with no plain form fails its function (the census), rather than being left out
    /// and said
    strict: bool,
    /// an item's box stats being written: its own code starting its attack (`armHits`)
    arming: bool,
    /// the animations its code jumps into at a label inside them (`playLabelIn`)
    mid_targets: BTreeSet<String>,
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
    release: BTreeSet<String>,
    /// the static methods of its package's classes its code calls by class name: (class, method)
    statics: BTreeSet<(String, String)>,
    /// the body of the function being written, for a variable's values read back
    body: Vec<Stmt>,
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
    /// how many parameters the function being written takes (its registers 1..=n)
    nparams: u32,
    /// the method being written (none in a closure it makes), and what the learning pass saw each
    /// return: a call of one that only ever returns true or false is a test as it is
    current_fn: String,
    returns: BTreeMap<String, Ty>,
    method_ty: BTreeMap<String, Ty>,
    /// what each of its own methods' arguments is handed, by (method, argument), learned from every
    /// call of it (a record every call hands it is read as one)
    param_ty: BTreeMap<(String, usize), Ty>,
    /// what a list a function is handed holds, by function and place, where every call hands it a
    /// list field of records; and the variables of the function being written that hold one
    param_elem: BTreeMap<(String, usize), Ty>,
    /// its fields that only ever hold a clip of the stage's planes (a part), and what was seen of each
    part_fields: BTreeMap<String, bool>,
    /// what only feeds drawing, by class (its fields), and the variables of the function being written
    drawn: BTreeMap<String, BTreeSet<String>>,
    drawn_locals: BTreeSet<u32>,
    local_elem: BTreeMap<u32, Ty>,
    /// what each of the decompiler's temporaries holds in the function being written
    temp_ty: BTreeMap<u32, Ty>,
    /// what each list field holds, learned from what its code pushes onto it
    list_elem: BTreeMap<String, Ty>,
    /// the defaults the function being written gives the arguments left off
    defaults: Vec<String>,
}

impl<'o, 'a> Writer<'o, 'a> {
    fn new(o: &'o Object<'a>) -> Result<Self, String> {
        let mut w = Writer { o, flat: BTreeMap::new(), flat_used: BTreeMap::new(), pkg: Vec::new(), pkg_ids: BTreeMap::new(), recs: Vec::new(), rec_lists: BTreeMap::new(), rec_fields: BTreeMap::new(), rt_lists: BTreeMap::new(), lit_rt: BTreeMap::new(), rt_fields: BTreeMap::new(), foreign_handlers: BTreeSet::new(), primed: false, clip_recs: BTreeSet::new(), max_args: BTreeMap::new(), part_params: BTreeMap::new(), nested_recs: BTreeMap::new(), part_lists: BTreeMap::new(), part_locals: BTreeSet::new(), part_written: BTreeSet::new(), record: None, rec_locals: BTreeMap::new(), stage_locals: BTreeSet::new(), empty_locals: BTreeSet::new(), self_locals: BTreeMap::new(), local_src: BTreeMap::new(), game_ticks: BTreeMap::new(), every_frame_extra: BTreeSet::new(), given: None, fields: BTreeMap::new(), methods: BTreeMap::new(), class: String::new(),
                             locals: BTreeMap::new(), helpers: BTreeMap::new(), gaps: BTreeSet::new(), mid_targets: BTreeSet::new(), kept: BTreeMap::new(), read: BTreeSet::new(), top: false, effects: 0, own_stats: BTreeSet::new(), floor: BTreeMap::new(), floor_speed: false, ends: BTreeMap::new(), release: BTreeSet::new(), statics: BTreeSet::new(), body: Vec::new(), touches: BTreeMap::new(), owners: ["self.getOwner()".to_string()].into(), blocks: BTreeSet::new(), stat_keys: BTreeSet::new(), jump_by_number: None, effect_on: "self".into(), testing: false, in_frame: false, captured: BTreeSet::new(), captured_ty: BTreeMap::new(), learning: false, seen: BTreeMap::new(), learned: false, plain: BTreeSet::new(), globals: BTreeSet::new(), inline: false, frame_top: false, nparams: 0, current_fn: String::new(), returns: BTreeMap::new(), method_ty: BTreeMap::new(), param_ty: BTreeMap::new(), param_elem: BTreeMap::new(), part_fields: BTreeMap::new(), drawn: BTreeMap::new(), drawn_locals: BTreeSet::new(), local_elem: BTreeMap::new(), temp_ty: BTreeMap::new(), list_elem: BTreeMap::new(), defaults: Vec::new(), strict: false, arming: false };
        // every class's names, each kept as it is unless two classes share one
        let mut taken: BTreeSet<String> = ["self", "match", "stage", "characters", "initialize", "update", "onTeardown", "onKill", "onDestroy", "life", "age", "started", "freshHit"]
            .into_iter().map(String::from).collect();
        // (the engine's own script globals, `engine.names`: content by one of those names would hide it)
        taken.extend(o.runtime.engine_names.iter().cloned());
        // (and every name the native files define: the script's own parts and the rules' blocks)
        let defined = crate::re!(r"(?m)^(?:function|var) ([A-Za-z_]\w*)");
        for text in o.runtime.native_files.values() {
            for c in defined.captures_iter(text) { taken.insert(c[1].to_string()); }
        }
        // (and every variable its functions declare: a field kept under one of their names is
        // lost to the script once such a function returns, so it takes another)
        for c in o.itself.into_iter().chain(o.classes.iter().copied()) {
            for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor]) {
                for (_, n) in f.local_names.iter().chain(f.activation.iter()) { taken.insert(ident(n)); }
            }
        }
        let name_for = |n: &str, class: &ClassModel, taken: &mut BTreeSet<String>| {
            let mut id = ident(n);
            if !taken.insert(id.clone()) { id = format!("{}_{}", ident(short(&class.name)), id); taken.insert(id.clone()); }
            id
        };
        let all: Vec<&ClassModel> = o.itself.into_iter().chain(o.classes.iter().copied()).collect();
        for c in &all {
            let own = o.itself.is_some_and(|i| i.name == c.name);
            let base_ok = if own { if o.fighter { matches!(short(&c.super_name), "PeptideCharacter" | "PeptideCharacterExt") } else if o.item { short(&c.super_name) == "PeptideItem" } else if o.stage { short(&c.super_name) == "PeptideStage" } else { matches!(short(&c.super_name), "PeptideProjectile" | "PeptideEnemy" | "PeptidePlatform") } } else { matches!(short(&c.super_name), "MovieClip" | "Sprite") };
            if !base_ok { return Err(format!("{} extends {}, whose code is not written here", c.name, c.super_name)) }
            if !c.unreadable.is_empty() { return Err(format!("{} has code that could not be read", c.name)) }
            // (a static field is one of its fields where there is one of it, a stage, or where nothing
            // writes it; a static method is its method)
            let written_static = |f: &crate::model::Field| all.iter().any(|k| k.methods.iter().any(|m| {
                let mut w = false;
                walk(&m.function.body, &mut |st| if let Stmt::Assign(Place::Member(t, n), _) = st { if n == &f.name && matches!(t, Expr::Name(cn) if short(cn) == short(&c.name)) { w = true } });
                w
            }));
            if c.fields.iter().any(|f| f.is_static && !o.stage && written_static(f)) { return Err(format!("{} has statics its code writes", c.name)) }
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
                    // (asked of itself, or named bare)
                    if !matches!(&**f, Expr::Member(_, n) | Expr::Name(n) if n == "addEventListener") { return }
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
                        crate::model::ast::walk_exprs(&b.body, &mut |x| if let Expr::Member(_, n) | Expr::Name(n) = x { if matches!(n.as_str(), "getXSpeed" | "getYSpeed") && !o.fighter { w.floor_speed = true } });
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
                    // (an item's, the same: one its handler keeps none of)
                    if o.item && matches!(&**f, Expr::Member(t, n) if n == "updateItemStats" && matches!(**t, Expr::This)) {
                        if let [Expr::Object(kv)] = a.as_slice() { for (k, _) in kv { if let Expr::Str(k) = k {
                            if !calls().item_stats.contains_key(k) { w.own_stats.insert(k.clone()); }
                        } } }
                    }
                });
            }
            for f in &c.fields {
                // (one declared as one of the API's records is that record)
                let record = f.type_name.as_deref().map(short).is_some_and(|t| {
                    let t = o.runtime.names.classes.get(t).map(String::as_str).unwrap_or(t);
                    calls().records.contains_key(t)
                });
                // (one declared a plain `Object` that only ever holds a record literal, or an element of
                // a field that only ever holds a list of them, is that record)
                let record = record || (f.type_name.as_deref().map(short) == Some("Object") && holds_records(c, &f.name))
                    // (one declared as one of Flash's classes the engine has its own of, a point: its
                    // fields read plainly)
                    || f.type_name.as_deref().is_some_and(|t| o.runtime.native(t).is_some() && !matches!(short(t), "Array" | "Vector" | "Dictionary" | "Object"));
                let holds = if w.holds_itself(c, &f.name) { Holds::Itself } else if record { Holds::Value(Ty::Rect) } else { Holds::Value(Ty::declared(f.type_name.as_deref())) };
                let id = if holds == Holds::Itself { "self".to_string() } else { name_for(&f.name, c, &mut taken) };
                // (a fighter's clip's field is a variable of the clip's own frame code: the frames and
                // the script share one scope, and the engine rolls those back too)
                let own = o.itself.is_some_and(|i| i.name == c.name);
                let plain = o.fighter && !own && holds != Holds::Itself && c.symbol.is_some_and(|s| s != o.root);
                if plain { w.plain.insert(member_key(&c.name, &f.name)); }
                if w.holds_owner(c, &f.name) { w.owners.insert(if plain { id.clone() } else { format!("{id}.get()") }); }
                let as_records = rt_value_fields(o, c);
                if let Some(k) = as_records.get(&f.name) { w.rt_fields.insert(member_key(&c.name, &f.name), k.clone()); }
                else if let Some(k) = flat_class(o, c, &f.name) { w.flat.insert(member_key(&c.name, &f.name), k); }
                w.fields.insert(member_key(&c.name, &f.name), (id, holds));
            }
            // (a clip class is dynamic in the source: a name its own code writes on itself that no
            // class of its declares, and the API's clip has no member by, is a field that write
            // makes, kept like any other)
            let clip_class = std::iter::once(c.super_name.clone()).chain(o.model.supers(&c.name)).any(|s| matches!(short(&s), "MovieClip" | "Sprite"));
            if clip_class {
                let mut dynamic = BTreeSet::new();
                for body in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
                    walk(body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, f), _) = st { dynamic.insert(f.clone()); });
                }
                let declared = |f: &str| std::iter::once(c.name.clone()).chain(o.model.supers(&c.name)).filter_map(|n| o.model.classes.get(&n))
                    .any(|k| k.fields.iter().any(|d| d.name == f) || k.methods.iter().any(|m| m.name == f));
                let fresh: Vec<String> = dynamic.into_iter().filter(|f| !declared(f) && !o.runtime.has_member("MovieClip", f) && !w.fields.contains_key(&member_key(&c.name, f))).collect();
                for f in fresh {
                    let id = name_for(&f, c, &mut taken);
                    w.fields.insert(member_key(&c.name, &f), (id, Holds::Value(Ty::Any)));
                }
            }
            // (and one its code makes as it runs, by putting a new one of such a class in it)
            let mut made = BTreeSet::new();
            for body in c.methods.iter().map(|m| &m.function.body) {
                walk(body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, f), Expr::New(..)) = st { made.insert(f.clone()); });
            }
            for f in made.into_iter().filter(|f| !c.fields.iter().any(|d| &d.name == f)) {
                let as_records = rt_value_fields(o, c);
                if let Some(k) = as_records.get(&f) {
                    let id = name_for(&f, c, &mut taken);
                    w.rt_fields.insert(member_key(&c.name, &f), k.clone());
                    w.fields.insert(member_key(&c.name, &f), (id, Holds::Value(Ty::Any)));
                    continue;
                }
                let Some(k) = flat_class(o, c, &f) else { continue };
                let id = name_for(&f, c, &mut taken);
                w.flat.insert(member_key(&c.name, &f), k);
                w.fields.insert(member_key(&c.name, &f), (id, Holds::Value(Ty::Any)));
            }
        }
        // a class of the package's own held in a field: its fields and methods under the field's name
        // (one held in more than one field is a record in each, made like any other)
        let mut held: BTreeMap<String, usize> = BTreeMap::new();
        for k in w.flat.values() { *held.entry(short(k).to_string()).or_default() += 1; }
        for (key, k) in w.flat.clone() {
            if flat_template(o.runtime, &k).is_some() { continue }
            let Some(pc) = o.model.classes.values().find(|c| c.name == k || short(&c.name) == k) else { continue };
            if held.get(short(&pc.name)).is_some_and(|n| *n > 1) { w.flat.remove(&key); continue }
            if !pc.unreadable.is_empty() { return Err(format!("{} has code that could not be read", pc.name)) }
            if pc.methods.iter().any(|m| m.is_static) { return Err(format!("{} has statics", pc.name)) }
            let fid = w.fields.get(&key).map(|(id, _)| id.clone()).ok_or("a field written out with no name")?;
            let mut names: Vec<(String, Option<String>)> = pc.fields.iter().filter(|f| !f.is_static).map(|f| (f.name.clone(), f.type_name.clone())).collect();
            for body in pc.methods.iter().map(|m| &m.function.body).chain([&pc.constructor.body]) {
                walk(body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, f), _) = st { if !names.iter().any(|(n, _)| n == f) { names.push((f.clone(), None)); } });
            }
            // (a field its constructor fills from what it is made with, where every maker hands it the
            // object itself: the object)
            let mut itself_fields: BTreeSet<String> = BTreeSet::new();
            for st in &pc.constructor.body {
                let Stmt::Assign(Place::Member(Expr::This, f), Expr::Local(r)) = st else { continue };
                let Some(i) = (*r as usize).checked_sub(1).filter(|i| *i < pc.constructor.params.len()) else { continue };
                let mut sites = 0;
                let mut all = true;
                for c in o.model.classes.values() {
                    for body in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
                        walk_exprs(body, &mut |e| if let Expr::New(n, a) = e {
                            if matches!(&**n, Expr::Name(n) if short(n) == short(&pc.name)) {
                                sites += 1;
                                all &= a.get(i).is_some_and(|x| matches!(x, Expr::This) || is_itself(x)
                                    || matches!(x, Expr::Member(t, g) if matches!(**t, Expr::This) && c.fields.iter().any(|d| &d.name == g) && c.methods.iter().any(|m| { let mut s = false; walk(&m.function.body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, h), v) = st { if h == g && is_itself(v) { s = true } }); s })));
                            }
                        });
                    }
                }
                if sites > 0 && all { itself_fields.insert(f.clone()); }
            }
            for (f, ty) in names {
                if itself_fields.contains(&f) { w.fields.insert(member_key(&pc.name, &f), ("self".into(), Holds::Itself)); continue }
                let id = format!("{fid}_{}", ident(&f));
                taken.insert(id.clone());
                // (one it holds an API class in, itself written out under its name: a FrameTimer)
                if let Some(k) = flat_class(o, pc, &f).filter(|k| flat_template(o.runtime, k).is_some()) { w.flat.insert(member_key(&pc.name, &f), k); }
                w.fields.insert(member_key(&pc.name, &f), (id, Holds::Value(Ty::declared(ty.as_deref()))));
            }
            for m in pc.methods.iter().filter(|m| !declares(m)) {
                let id = format!("{fid}_{}", ident(&accessor_name(m)));
                taken.insert(id.clone());
                w.methods.insert(member_key(&pc.name, &method_name(m)), id);
            }
            w.methods.insert(member_key(&pc.name, "new"), format!("{fid}_new"));
            taken.insert(format!("{fid}_new"));
            w.pkg_ids.insert(pc.name.clone(), fid);
            w.pkg.push(pc);
            w.flat.insert(key, pc.name.clone());
        }
        // a class of the package's own made anywhere else (into a list): a record, its methods
        // functions handed it
        let plain_class = |n: &str| o.model.classes.values().find(|c| short(&c.name) == short(n) && ((c.symbol.is_none() && matches!(short(&c.super_name), "Object" | ""))
                // (or a clip of its library whose frames have code that does more than move its own
                // timeline: a record whose timeline runs)
                || (c.symbol.is_some() && matches!(short(&c.super_name), "MovieClip" | "Sprite") && !c.frame_scripts.is_empty()
                    && !crate::emit::native_projectile::is_quiet_inside(o.model, o.runtime, &c.name)))
            // (not one an API class's record form is: a FrameTimer)
            && !o.runtime.provides(&c.name) && flat_template(o.runtime, short(n)).is_none()
            && !o.runtime.names.classes.iter().any(|(src, r)| short(r) == short(&c.name) && flat_template(o.runtime, src).is_some()));
        // (a clip with code of its own nested in its art: a record whose timeline runs)
        for n in &o.nested {
            if let Some(k) = o.model.classes.get(n) { if !w.recs.iter().any(|r| r.name == k.name) { w.recs.push(k); } }
        }
        let mut grow = true;
        while grow {
            grow = false;
            let scan: Vec<&ClassModel> = all.iter().copied().chain(w.pkg.iter().copied()).chain(w.recs.iter().copied()).collect();
            for c in scan {
                for body in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
                    walk_exprs(body, &mut |e| if let Expr::New(n, _) = e {
                        let Expr::Name(n) = &**n else { return };
                        let Some(k) = plain_class(n) else { return };
                        if w.pkg_ids.contains_key(&k.name) || w.recs.iter().any(|r| r.name == k.name) { return }
                        w.recs.push(k);
                        grow = true;
                    });
                    // (a field it keeps them in: one it pushes them onto, writes them into by index, or
                    // fills with a list of them)
                    let made = |e: &Expr| match e { Expr::New(n, _) => match &**n { Expr::Name(n) => plain_class(n).map(|k| k.name.clone()), _ => None }, _ => None };
                    walk_exprs(body, &mut |e| if let Expr::Call(f, a) = e {
                        if let (Expr::Member(l, m), [x]) = (&**f, a.as_slice()) {
                            if m != "push" { return }
                            let Expr::Member(t, fl) = &**l else { return };
                            if !matches!(**t, Expr::This) { return }
                            if let Some(k) = made(x) { w.rec_lists.insert(member_key(&c.name, fl), k); }
                        }
                    });
                    walk(body, &mut |st| match st {
                        Stmt::Assign(Place::Index(Expr::Member(t, fl), _), v) if matches!(**t, Expr::This) => {
                            if let Some(k) = made(v) { w.rec_lists.insert(member_key(&c.name, fl), k); }
                        }
                        Stmt::Assign(Place::Member(Expr::This, fl), Expr::Array(xs)) if !xs.is_empty() => {
                            let ks: BTreeSet<Option<String>> = xs.iter().map(made).collect();
                            if ks.len() == 1 { if let Some(Some(k)) = ks.into_iter().next() { w.rec_lists.insert(member_key(&c.name, fl), k); } }
                        }
                        // (one record in a field)
                        Stmt::Assign(Place::Member(Expr::This, fl), v) => {
                            if let Some(k) = made(v) { w.rec_fields.insert(member_key(&c.name, fl), k); }
                        }
                        _ => {}
                    });
                }
            }
        }
        // a field it keeps a list of one of the API's classes in, by index, where that class has a
        // record form (`classes.hx`): a list of records
        let rt_made = |e: &Expr| match e { Expr::New(n, _) => match &**n { Expr::Name(n) => {
            let src = o.runtime.names.classes.iter().find(|(_, r)| short(r) == short(n)).map(|(s, _)| s.clone()).unwrap_or_else(|| short(n).to_string());
            o.runtime.native_files.get("classes").filter(|t| t.lines().any(|l| l.trim_end() == format!("// ---- {src} as a record"))).map(|_| src)
        }, _ => None }, _ => None };
        for c in all.iter().copied() {
            for body in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
                walk(body, &mut |st| if let Stmt::Assign(Place::Index(Expr::Member(t, fl), _), v) = st {
                    if matches!(**t, Expr::This) { if let Some(k) = rt_made(v) { w.rt_lists.insert(member_key(&c.name, fl), k); } }
                });
            }
        }
        {
            // (a field every record written out in place fills with the same such class; one filled
            // with anything else, or written anywhere, is none)
            let mut seen: BTreeMap<String, Option<String>> = BTreeMap::new();
            for c in all.iter().copied() {
                for body in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
                    walk_exprs(body, &mut |e| if let Expr::Object(kv) = e {
                        for (k, v) in kv {
                            let Expr::Str(k) = k else { continue };
                            let got = rt_made(v);
                            let was = seen.entry(k.clone()).or_insert_with(|| got.clone());
                            if *was != got { *was = None; }
                        }
                    });
                    walk(body, &mut |st| if let Stmt::Assign(Place::Member(_, f), _) = st { seen.insert(f.clone(), None); });
                }
            }
            w.lit_rt = seen.into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect();
            // (and a field any class keeps one in as a record, named as another object reads it)
            for c in o.model.classes.values() {
                for (f, k) in rt_value_fields(o, c) { w.lit_rt.entry(f).or_insert(k); }
            }
        }
        w.clip_recs = w.recs.iter().filter(|k| k.symbol.is_some()).map(|k| k.name.clone()).collect();
        for k in w.recs.clone() {
            if !k.unreadable.is_empty() { return Err(format!("{} has code that could not be read", k.name)) }
            if k.methods.iter().any(|m| m.is_static) { return Err(format!("{} has statics", k.name)) }
            let mut names: Vec<(String, Option<String>)> = k.fields.iter().filter(|f| !f.is_static).map(|f| (f.name.clone(), f.type_name.clone())).collect();
            for body in k.methods.iter().map(|m| &m.function.body).chain([&k.constructor.body]) {
                walk(body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, f), _) = st { if !names.iter().any(|(n, _)| n == f) { names.push((f.clone(), None)); } });
            }
            for (f, ty) in names {
                if let Some(t) = flat_class(o, k, &f).filter(|t| flat_template(o.runtime, t).is_some()) { w.flat.insert(member_key(&k.name, &f), t); }
                let key = member_key(&k.name, &f);
                w.plain.insert(key.clone());
                w.fields.insert(key, (format!("me.{}", ident(&f)), Holds::Value(Ty::declared(ty.as_deref()))));
            }
            let kid = ident(short(&k.name));
            for m in k.methods.iter().filter(|m| !declares(m)) {
                let id = format!("{kid}_{}", ident(&accessor_name(m)));
                taken.insert(id.clone());
                w.methods.insert(member_key(&k.name, &method_name(m)), id);
            }
            w.methods.insert(member_key(&k.name, "new"), format!("{kid}_new"));
            w.methods.insert(member_key(&k.name, "copy"), format!("{kid}_copy"));
        }
        w.globals = taken;
        Ok(w)
    }

    /// what each field holds, learned from all the code before any of it is written (a method
    /// written before the frame that fills a field reads it right): a pass whose text is dropped
    fn learn_fields(&mut self) {
        let fresh = Writer::new(self.o).ok();
        self.learning = true;
        let classes: Vec<&ClassModel> = self.o.itself.into_iter().chain(self.o.classes.iter().copied()).chain(self.pkg.iter().copied()).collect();
        // (twice over: what one function learns, another written before it reads)
        for _ in 0..2 {
            for c in &classes {
                self.class = c.name.clone();
                // (its constructor first: what it fills its fields with as it is made)
                self.in_frame = false;
                let name = self.methods.get(&member_key(&c.name, "new")).cloned().unwrap_or_else(|| "construct".into());
                let body: Vec<Stmt> = c.constructor.body.iter().filter(|s| !matches!(s, Stmt::Expr(Expr::SuperCall(..)))).cloned().collect();
                let _ = self.function(&name, &Function { body, ..c.constructor.clone() });
                for m in c.methods.iter().filter(|m| !declares(m)) {
                    let name = self.methods[&member_key(&c.name, &method_name(m))].clone();
                    self.in_frame = c.frame_scripts.iter().any(|fs| fs.method == m.name);
                    let _ = self.function(&name, &m.function);
                }
            }
        }
        for k in self.recs.clone() {
            self.class = k.name.clone();
            self.record = Some(k.name.clone());
            // (its constructor first: what it puts in its fields)
            let body: Vec<Stmt> = k.constructor.body.iter().filter(|s| !matches!(s, Stmt::Expr(Expr::SuperCall(..)))).cloned().collect();
            let _ = self.function(&format!("{}_new", ident(short(&k.name))), &Function { body, ..k.constructor.clone() });
            for m in k.methods.iter().filter(|m| !declares(m)) {
                let name = self.methods[&member_key(&k.name, &method_name(m))].clone();
                let _ = self.function(&name, &m.function);
            }
            self.record = None;
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
            f.method_ty = self.returns.iter().filter(|(_, t)| **t == Ty::Bool).map(|(k, t)| (k.clone(), *t)).collect();
            f.param_ty = self.param_ty.iter().filter(|(_, t)| **t == Ty::Rect).map(|(k, t)| (k.clone(), *t)).collect();
            f.list_elem = self.list_elem.iter().filter(|(_, t)| **t == Ty::Rect).map(|(k, t)| (k.clone(), *t)).collect();
            f.param_elem = self.param_elem.iter().filter(|(_, t)| **t == Ty::Rect).map(|(k, t)| (k.clone(), *t)).collect();
            f.part_fields = self.part_fields.iter().filter(|(_, v)| **v).map(|(k, v)| (k.clone(), *v)).collect();
            f.part_lists = self.part_lists.iter().filter(|(_, v)| **v).map(|(k, v)| (k.clone(), *v)).collect();
            f.part_written = self.part_written.clone();
            f.foreign_handlers = self.foreign_handlers.clone();
            f.max_args = self.max_args.clone();
            f.part_params = self.part_params.clone();
            f.nested_recs = self.nested_recs.clone();
            // (a hurt its code answers first is handed a record: `setHurtInterrupt(checkHurt)`)
            for c in self.o.itself.into_iter().chain(self.o.classes.iter().copied()) {
                for m in &c.methods {
                    crate::model::ast::walk_exprs(&m.function.body, &mut |e| if let Expr::Call(g, a) = e {
                        if !matches!(&**g, Expr::Member(_, n) | Expr::Name(n) if n == "setHurtInterrupt") { return }
                        let named = match a.first() { Some(Expr::Member(t, n)) if matches!(**t, Expr::This) => Some(n.clone()), Some(Expr::Name(n)) => Some(n.clone()), _ => None };
                        if let Some(id) = named.and_then(|n| f.methods.get(&member_key(&c.name, &n)).cloned()) { f.param_ty.insert((id, 0), Ty::Rect); }
                    });
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
                    all &= matches!(v, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(o, n) if n == "getOwner" && me(o)))
                        // (in another object's clip, the fighter it belongs to: `getCharacter(this)`)
                        || (!self.o.fighter && !self.o.stage && matches!(v, Expr::Call(g, a) if matches!(a.as_slice(), [Expr::This]) && matches!(&**g, Expr::Member(c, n) if n == "getCharacter" && matches!(&**c, Expr::Name(k) if k.contains("PeptideAPI")))));
                }
            });
        }
        any && all
    }

    /// whether every value the class's code puts in a field is the object itself
    /// (`self.self = SSF2API.getProjectile(this)`)
    /// whether every class of the package that declares a field keeps the object it runs as in it
    fn holds_itself_everywhere(&self, field: &str) -> bool {
        let keepers: Vec<&ClassModel> = self.o.model.classes.values().filter(|c| !self.o.runtime.provides(&c.name) && c.fields.iter().any(|d| d.name == field && !d.is_static)).collect();
        !keepers.is_empty() && keepers.iter().all(|c| self.holds_itself(c, field))
    }

    fn holds_itself(&self, c: &ClassModel, field: &str) -> bool {
        self.holds_itself_seen(c, field, &mut BTreeSet::new())
    }

    /// (one put in it from another of its fields that holds the object counts too)
    fn holds_itself_seen(&self, c: &ClassModel, field: &str, seen: &mut BTreeSet<String>) -> bool {
        if !seen.insert(field.to_string()) { return false }
        let mut values: Vec<Expr> = Vec::new();
        for m in &c.methods {
            walk(&m.function.body, &mut |s| if let Stmt::Assign(Place::Member(t, f), v) = s {
                if matches!(t, Expr::This) && f == field { values.push(v.clone()); }
            });
        }
        !values.is_empty() && values.iter().all(|v| is_itself(v)
            || matches!(v, Expr::Member(t, g) if matches!(**t, Expr::This) && g != field && self.holds_itself_seen(c, g, seen)))
    }

    fn write(&mut self) -> Result<Written, String> {
        let mut out = Written::default();
        let classes: Vec<&ClassModel> = self.o.itself.into_iter().chain(self.o.classes.iter().copied()).chain(self.pkg.iter().copied()).collect();
        // a class of the package's own it keeps several of: records, made by `K_new`, copied by
        // `K_copy`, and its methods functions handed the record
        for k in self.recs.clone() {
            self.class = k.name.clone();
            self.record = Some(k.name.clone());
            let kid = ident(short(&k.name));
            let fields: Vec<(String, String)> = self.fields.iter().filter(|(key, _)| key.starts_with(&format!("{}::", k.name)))
                .map(|(key, (id, _))| (key.split_once("::").map(|x| x.1.to_string()).unwrap_or_default(), id.clone())).collect();
            let clip = self.clip_recs.contains(&k.name);
            // (a clip's frame scripts are registered by the timeline the record runs, not by its code)
            let body: Vec<Stmt> = k.constructor.body.iter().filter(|s| !matches!(s, Stmt::Expr(Expr::SuperCall(..))))
                .filter(|s| !(clip && matches!(s, Stmt::Expr(Expr::Call(g, _)) if matches!(&**g, Expr::Member(_, m) | Expr::Name(m) if m == "addFrameScript"))))
                .cloned().collect();
            let f = Function { body, ..k.constructor.clone() };
            let text = self.function(&format!("{kid}_new"), &f).map_err(|e| format!("{}'s constructor: {e}", short(&k.name)));
            let text = match text { Ok(t) => t, Err(e) => { self.record = None; return Err(e) } };
            // (the record made first, with each field it has, and handed back at the end)
            // (each at the value it declares, else nothing)
            let start: Vec<String> = fields.iter().map(|(f, _)| {
                let d = k.fields.iter().find(|x| &x.name == f).and_then(|x| x.default.as_ref()).and_then(literal).unwrap_or_else(|| "null".into());
                format!("{}: {d}", ident(f))
            }).collect();
            let mut lines: Vec<String> = text.lines().map(String::from).collect();
            if clip {
                // (a clip: a clip record, with the clips it places under names, its fields, and its
                // timeline running from its first frame, whose script runs as it enters it)
                let sym = k.symbol;
                let sp = sym.and_then(|s| self.o.model.sprites.get(&s));
                let total = sp.map(|x| x.frame_count.max(1)).unwrap_or(1);
                let labels: Vec<String> = sp.map(|x| x.labels.iter().map(|(f, l)| format!("[{}, {}]", f + 1, quote(l))).collect()).unwrap_or_default();
                let mut setup = vec!["\tvar me = clipRecord(null);".to_string()];
                let mut kids = Vec::new();
                clip_record_of(self.o.model, sym, "me", 0, &mut kids);
                setup.extend(kids.into_iter().map(|l| format!("\t{l}")));
                // (a field that names a clip its clip places is that clip, made above: not set back to nothing)
                let fields_set: Vec<String> = start.iter().filter(|kv| { let f = kv.split_once(": ").map(|x| x.0).unwrap_or(kv); !setup_has_kid(&setup, f) }).map(|kv| { let (f, d) = kv.split_once(": ").unwrap_or((kv, "null")); format!("\tme.{f} = {d};") }).collect();
                setup.extend(fields_set);
                // (a clip that is one of its effects: drawn as that effect wherever its code puts it, `screenClip`)
                let mut ps: Vec<(String, u32, u32)> = self.o.effect_pieces.iter().filter(|p| p.0 == short(&k.name)).map(|p| (p.1.clone(), p.2, p.3)).collect();
                if !ps.is_empty() {
                    ps.sort_by_key(|p| p.1);
                    self.util("screenClip")?;
                    setup.push(format!("\tme.id = {};", quote(short(&k.name))));
                    setup.push(format!("\tme.anims = [{}];", ps.iter().map(|(a, f, l)| format!("[{}, {}, {}]", quote(a), f.max(&1), l)).collect::<Vec<_>>().join(", ")));
                    // (the clips it places that its code turns, drawn apart from its art: each the
                    // effect's own animation of it, where the clip places it, with a timeline of its own)
                    for ic in self.o.effect_inners.get(short(&k.name)).cloned().unwrap_or_default() {
                        if !setup_has_kid(&setup, &ic.name) { continue }
                        let Some((_, m)) = ic.placed.first() else { continue };
                        let at = format!("me.{}", ic.name);
                        setup.push(format!("\t{at}.id = {}; {at}.anims = [[{}, 1, {}]]; {at}.x = {}; {at}.y = {}; {at}.scaleX = {}; {at}.scaleY = {}; {at}.parentOf = function() {{ return me; }}; clipRun({at}, {}, [], null);",
                            quote(short(&k.name)), quote(&ic.art), ic.frames.max(1), number(m.tx), number(m.ty), number(m.sx), number(m.sy), ic.frames.max(1)));
                    }
                }
                setup.push(format!("\tclipRun(me, {total}, [{}], {kid}_frames);", labels.join(", ")));
                if let Some(open) = lines.iter().position(|l| l.trim_end().ends_with('{')) { for (i, l) in setup.into_iter().enumerate() { lines.insert(open + 1 + i, l); } }
                self.util("clipRecord")?;
                self.util("clipRun")?;
                // (each frame's script, by the frame it is on)
                let mut cases = Vec::new();
                for fs in &k.frame_scripts {
                    if let Some(id) = self.methods.get(&member_key(&k.name, &fs.method)) { cases.push(format!("\t\tcase {}: {id}(me);", fs.frame + 1)); }
                }
                out.functions.push(format!("// the scripts a {} runs entering its frames\nfunction {kid}_frames(me, n) {{\n\tswitch (n) {{\n{}\n\t}}\n}}", short(&k.name), cases.join("\n")));
            } else if let Some(open) = lines.iter().position(|l| l.trim_end().ends_with('{')) { lines.insert(open + 1, format!("\tvar me = {{ {} }};", start.join(", "))); }
            if let Some(close) = lines.iter().rposition(|l| l.trim() == "}") { lines.insert(close, "\treturn me;".into()); }
            out.functions.push(format!("// a {} its code keeps several of: a record, its methods handed it\n{}", short(&k.name), lines.join("\n")));
            let copies: Vec<String> = fields.iter().map(|(f, id)| {
                let held = self.flat.get(&member_key(&k.name, f)).cloned();
                match held { Some(t) => format!("{}: {t}_copy({id})", ident(f)), None => format!("{}: {id}", ident(f)) }
            }).collect();
            out.functions.push(format!("function {kid}_copy(me) {{ return me == null ? null : {{ {} }}; }}", copies.join(", ")));
            for m in k.methods.iter().filter(|m| !declares(m)) {
                let name = self.methods[&member_key(&k.name, &method_name(m))].clone();
                let text = self.function(&name, &m.function).map_err(|e| format!("{}.{}: {e}", short(&k.name), m.name));
                let text = match text { Ok(t) => t, Err(e) => { self.record = None; return Err(e) } };
                out.functions.push(text);
            }
            self.record = None;
        }
        // a class of the package's own held in a field: its constructor, as the field's `new`
        for pc in self.pkg.clone() {
            self.class = pc.name.clone();
            let body: Vec<Stmt> = pc.constructor.body.iter().filter(|s| !matches!(s, Stmt::Expr(Expr::SuperCall(..)))).cloned().collect();
            let f = Function { body, ..pc.constructor.clone() };
            let name = self.methods[&member_key(&pc.name, "new")].clone();
            let text = self.function(&name, &f).map_err(|e| format!("{}'s constructor: {e}", short(&pc.name)))?;
            out.functions.push(format!("// a {} its code makes, written out under the field it is kept in\n{text}", short(&pc.name)));
        }
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
            for m in c.methods.iter().filter(|m| !declares(m) && reached(self.o, c, m)) {
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
        // (a static field its class's setup fills with a value written out: filled as it starts)
        let mut static_starts: Vec<String> = Vec::new();
        let field_list: Vec<(String, String)> = self.fields.iter().filter(|(k, (_, h))| matches!(h, Holds::Value(_)) && self.read.contains(*k)).map(|(k, (id, _))| (k.clone(), id.clone())).collect();
        for (key, id) in field_list {
            let (class, field) = key.split_once("::").unwrap_or(("", &key));
            let Some(k) = classes.iter().find(|c| c.name == class).copied() else { continue };
            let Some(f) = k.fields.iter().find(|f| f.name == field) else { continue };
            if !f.is_static || f.default.as_ref().and_then(literal).is_some() || id.starts_with("me.") || self.plain.contains(&key) { continue }
            let Some(v) = static_value(k, field) else { continue };
            let was = std::mem::replace(&mut self.class, class.to_string());
            if let Ok((t, _)) = self.expr(&v) { static_starts.push(format!("{id}.set({t});")); }
            self.class = was;
        }
        for (i, l) in static_starts.into_iter().enumerate() { out.start.insert(i, l); }
        // the fields its code uses, as the engine's containers
        for (key, (id, holds)) in &self.fields {
            let Holds::Value(ty) = holds else { continue };
            if !self.read.contains(key) { continue }
            let (class, field) = key.split_once("::").unwrap_or(("", key));
            let c = classes.iter().filter(|c| c.name == class).find_map(|c| c.fields.iter().find(|f| f.name == field));
            let default = c.and_then(|f| f.default.as_ref()).and_then(literal);
            let whole = c.is_some_and(|f| matches!(f.type_name.as_deref().map(short), Some("int" | "uint")));
            // (true-or-false and text fields are object containers: the engine keeps few of the others
            // for a whole match, and two fighters running the same script ran out)
            let make = match ty {
                Ty::Num if whole => format!("self.makeInt({})", default.clone().unwrap_or_else(|| "0".into())),
                Ty::Num => format!("self.makeFloat({})", default.clone().unwrap_or_else(|| "0".into())),
                Ty::Bool => format!("self.makeObject({})", default.clone().unwrap_or_else(|| "false".into())),
                _ => format!("self.makeObject({})", default.clone().unwrap_or_else(|| "null".into())),
            };
            // (a record's: on the record)
            if id.starts_with("me.") { continue }
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
        // (its listeners for the end of each frame, run after its own code each frame)
        for (h, flag) in &self.game_ticks { out.tick.push(format!("if ({flag}.get() == true) {h}(null);")); }
        out.floor = self.floor.iter().map(|(h, l)| (l.clone(), h.clone())).collect();
        out.floor_speed = self.floor_speed;
        out.ends = self.ends.iter().map(|(h, l)| (l.clone(), h.clone())).collect();
        out.release = self.release.iter().cloned().collect();
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
            // (its final smash, the source's `special` attack, for whatever gives it one: `grantFinal`)
            if let Some(anim) = (self.o.root_anim)("special") {
                self.blocks.insert("start_attack".into());
                out.publish.push(format!("self.exports.__finalSmash = function() {{ startAttack({}); }};", quote(&anim)));
                out.publish.push("self.addTimer(1, -1, finalHeldFrame, { persistent: true });".into());
                self.util("grantFinal")?;
            }
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
        // (an item's shots, fired by whoever holds it: `getItem().fireProjectile(name, x, y)`)
        if self.o.item {
            let mut cases = Vec::new();
            for n in self.o.shots.clone() {
                if (self.o.fired)(&n).is_none() { continue }
                let f = self.fire(&n, "self")?;
                cases.push(format!("case {}: return {f}(x, y);", quote(&n)));
            }
            if !cases.is_empty() { out.publish.push(format!("self.exports.__fire = function(name, ?x, ?y) {{ switch (name) {{ {} }} return null; }};", cases.join(" "))); }
        }
        // an object's own methods another object's code calls on it, on its exports
        if !self.o.fighter {
            if let Some(c) = self.o.itself {
                let mut asked: BTreeSet<String> = asked_of_others().lock().map(|a| a.clone()).unwrap_or_default();
                for k in self.o.model.classes.values() {
                    for body in k.methods.iter().map(|m| &m.function.body).chain([&k.constructor.body]) {
                        walk_exprs(body, &mut |e| if let Expr::Call(f, _) = e {
                            if let Expr::Member(o, n) = &**f { if !matches!(**o, Expr::This) { asked.insert(n.clone()); } }
                        } else if let Expr::Member(o, n) = e {
                            // (a getter another object's code reads on it)
                            if !matches!(**o, Expr::This) && c.methods.iter().any(|m| m.kind == MethodKind::Getter && m.name == *n) { asked.insert(n.clone()); }
                        });
                    }
                }
                for m in c.methods.iter().filter(|m| !m.is_static && !declares(m) && asked.contains(&m.name)) {
                    if let Some(id) = self.methods.get(&member_key(&c.name, &method_name(m))) {
                        if out.functions.iter().any(|t| t.contains(&format!("function {id}("))) { out.publish.push(format!("self.exports.{} = {id};", ident(&m.name))); }
                    }
                }
            }
        }
        // an object's own fields another object's code reads or writes on it (on it, or on its clip),
        // through what it publishes, as a fighter publishes its clips' (`exports.__fieldOf`)
        if !self.o.fighter {
            let mut asked: BTreeSet<String> = BTreeSet::new();
            for k in self.o.model.classes.values() {
                for body in k.methods.iter().map(|m| &m.function.body).chain([&k.constructor.body]) {
                    walk_exprs(body, &mut |e| if let Expr::Member(o, n) = e { if !matches!(**o, Expr::This) { asked.insert(n.clone()); } });
                    walk(body, &mut |st| if let Stmt::Assign(Place::Member(o, n), _) = st { if !matches!(o, Expr::This) { asked.insert(n.clone()); } });
                }
            }
            if let Ok(a) = asked_fields().lock() { asked.extend(a.iter().cloned()); }
            let mut cases: BTreeMap<&str, String> = BTreeMap::new();
            for ((_, field), id) in &out.field_ids {
                if !asked.contains(field) || cases.contains_key(field.as_str()) { continue }
                // (a plain variable's is a getter already; a container answers get and set itself)
                cases.insert(field, format!("case {}: return {id};", quote(field)));
            }
            if !cases.is_empty() {
                let list: Vec<String> = cases.into_values().collect();
                out.publish.push(format!("self.exports.__fieldOf = function(clip, name) {{ switch (name) {{ {} }} return null; }};", list.join(" ")));
            }
        }
        // a fighter's own methods another object's code calls on it (reached through
        // `exports.__source`)
        if self.o.fighter {
            if let Some(c) = self.o.itself {
                let mut asked: BTreeSet<String> = BTreeSet::new();
                for k in self.o.model.classes.values() {
                    for body in k.methods.iter().map(|m| &m.function.body).chain([&k.constructor.body]) {
                        walk_exprs(body, &mut |e| if let Expr::Call(f, _) = e {
                            if let Expr::Member(o, n) = &**f { if !matches!(**o, Expr::This) { asked.insert(n.clone()); } }
                        } else if let Expr::Member(o, n) = e {
                            // (a getter another object's code reads on it)
                            if !matches!(**o, Expr::This) && c.methods.iter().any(|m| m.kind == MethodKind::Getter && m.name == *n) { asked.insert(n.clone()); }
                        });
                    }
                }
                for m in c.methods.iter().filter(|m| !declares(m) && asked.contains(&m.name)) {
                    if let Some(id) = self.methods.get(&member_key(&c.name, &method_name(m))) {
                        if out.functions.iter().any(|t| t.contains(&format!("function {id}("))) { out.publish.push(format!("source.{} = {id};", ident(&m.name))); }
                    }
                }
            }
        }
        // its stats another object's code writes that this engine keeps none of, each published as
        // a setter (`who.exports.__stat.set(v)`); the engine's own stats are written straight on it
        let written_by_others = if self.o.fighter { BTreeSet::new() } else { others_write_stats(self.o.model) };
        let kept_before: BTreeSet<String> = self.kept.keys().cloned().collect();
        for k in &written_by_others {
            let published = if let Some(var) = self.o.switches.get(k) {
                super::projectile_stats::table().holds_test(k, "v").map(|t| format!("{{ set: function(v) {{ {var} = {t}; }} }}"))
            } else if self.own_stats.contains(k) {
                Some(self.own_stat(k))
            } else {
                match stat_row(k) {
                    Some(StatRow::Kept(t)) => Some(format!("{{ set: function(v) {{ {}; }} }}", self.stat_code(&t, "v"))),
                    _ => None,
                }
            };
            // (a stat nothing it does here reads changes nothing)
            if let Some(p) = published { out.publish.push(format!("self.exports.__{} = {p};", ident(k))); }
        }
        // (a container made for one of them just now is declared with the rest)
        for (k, v) in &self.kept { if !kept_before.contains(k) { out.fields.push(v.clone()); } }
        // the static methods its code calls by class name (one may ask for another as it is written)
        let mut done: BTreeSet<(String, String)> = BTreeSet::new();
        while let Some((cls, name)) = self.statics.iter().find(|x| !done.contains(*x)).cloned() {
            done.insert((cls.clone(), name.clone()));
            let Some(k) = self.o.model.classes.get(&cls) else { continue };
            let Some(x) = k.methods.iter().find(|x| x.is_static && x.name == name) else { continue };
            let fid = format!("{}_{}", ident(short(&cls)), ident(&name));
            let saved = std::mem::replace(&mut self.class, cls.clone());
            let r = self.function(&fid, &x.function);
            self.class = saved;
            out.functions.push(r.map_err(|e| format!("{}.{name}: {e}", short(&cls)))?);
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
        out.gaps = self.gaps.clone();
        out.mid_targets = self.mid_targets.clone();
        out.stat_keys = self.stat_keys.clone();
        self.write_flat(&mut out);
        self.write_parts(&mut out)?;
        out.every_frame.extend(self.every_frame_extra.iter().cloned());
        Ok(out)
    }

    /// its parts: each a vfx riding it where the animation it plays places it
    fn write_parts(&self, out: &mut Written) -> Result<(), String> {
        let parts: Vec<&Part> = self.o.parts.iter().filter(|p| !p.placed.is_empty()).collect();
        if parts.is_empty() { return Ok(()) }
        for p in &parts {
            let id = part_id(&p.name);
            out.fields.push(format!("// its {} clip, drawn apart from its animations' art (its code turns it), and the animation it was last placed for\nvar {id} = self.makeObject(null);\nvar {id}On = self.makeObject(null);", p.name));
        }
        let mut by_anim: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for p in &parts {
            let id = part_id(&p.name);
            for (anim, art, x, y, r, sx, sy) in &p.placed {
                by_anim.entry(anim.as_str()).or_default().push(format!("placePart({id}, {id}On, {}, {}, {}, {}, {}, {}, {});", quote(art), number(*x), number(*y), number(*r), number(*sx), number(*sy), quote(&p.name)));
            }
        }
        let mut lines = Vec::new();
        let hide: Vec<String> = parts.iter().map(|p| { let id = part_id(&p.name); format!("hidePart({id}, {id}On)") }).collect();
        lines.push("switch (anim) {".to_string());
        for (anim, places) in &by_anim {
            lines.push(format!("\tcase {}:", quote(anim)));
            // (a part this animation places none of is hidden)
            for p in &parts {
                let id = part_id(&p.name);
                if !p.placed.iter().any(|(a, ..)| a == anim) { lines.push(format!("\t\thidePart({id}, {id}On);")); }
            }
            for l in places { lines.push(format!("\t\t{l}")); }
        }
        lines.push("\tdefault:".to_string());
        for h in &hide { lines.push(format!("\t\t{h};")); }
        lines.push("}".to_string());
        let text = helper(self.o.runtime, "parts", "place_parts", "placeParts", &[("CONTENT", self.o.content.clone()), ("SIZE", number(self.o.size))], &lines)?;
        out.functions.push(text);
        out.every_frame.push("placeParts();".into());
        Ok(())
    }

    /// the flattened fields' state and calls, as far as the code uses them
    fn write_flat(&self, out: &mut Written) {
        let mut code: String = out.functions.iter().chain(out.start.iter()).chain(out.tick.iter()).chain(out.frames.values()).chain(out.fields.iter())
            .cloned().collect::<Vec<_>>().join("\n");
        for (id, k) in &self.flat_used {
            let t = flat_template(self.o.runtime, k).unwrap_or_default().replace("FIELD", id);
            // (each line with the comments above it)
            let mut units: Vec<(String, String)> = Vec::new();
            let mut notes = Vec::new();
            for l in t.lines().filter(|l| !l.starts_with("//:") && !l.trim().is_empty()) {
                if l.trim_start().starts_with("//") { notes.push(l.to_string()); continue }
                let name = crate::dyn_re(r"^(?:var|function)\s+(\w+)").captures(l).map(|c| c[1].to_string()).unwrap_or_default();
                let mut text = std::mem::take(&mut notes);
                text.push(l.to_string());
                units.push((name, text.join("\n")));
            }
            let mut kept: BTreeSet<usize> = BTreeSet::new();
            loop {
                let before = kept.len();
                for (i, (name, _)) in units.iter().enumerate() {
                    if kept.contains(&i) || name.is_empty() { continue }
                    if crate::dyn_re(&format!(r"\b{}\b", regex::escape(name))).is_match(&code) { kept.insert(i); code.push('\n'); code.push_str(&units[i].1); }
                }
                if kept.len() == before { break }
            }
            for (_, (_, text)) in units.iter().enumerate().filter(|(i, _)| kept.contains(i)) {
                if text.lines().last().is_some_and(|l| l.starts_with("var ")) { out.fields.push(text.clone()); } else { out.functions.push(text.clone()); }
            }
        }
    }

    // ---- functions and statements ---------------------------------------------------------------

    /// a method whose last parameters none of its own calls ever hands (and that is never handed
    /// out as a value, where whoever calls it might): those are their defaults throughout
    fn unpassed_defaults(&self, name: &str, f: &Function) -> Option<Function> {
        let passed = *self.max_args.get(name)?;
        let n = f.params.len();
        if passed >= n { return None }
        // (handed out anywhere as a value: anyone may call it with more)
        let m = self.methods.iter().find(|(_, v)| v.as_str() == name).map(|(k, _)| k.split_once("::").map(|x| x.1.to_string()).unwrap_or_default())?;
        let mut as_value = false;
        // (and called by that name on anything, anywhere in its package, with more)
        let mut most = passed;
        for c in self.o.model.classes.values() {
            for b in c.methods.iter().map(|x| &x.function.body).chain([&c.constructor.body]) {
                walk_exprs(b, &mut |e| if let Expr::Call(g, a) = e { if matches!(&**g, Expr::Member(_, x) | Expr::Name(x) if *x == m) { most = most.max(a.len()) } });
            }
        }
        if most >= n { return None }
        for c in self.o.model.classes.values() {
            for b in c.methods.iter().map(|x| &x.function.body).chain([&c.constructor.body]) {
                let (mut all, mut called) = (0usize, 0usize);
                walk_exprs(b, &mut |e| {
                    if matches!(e, Expr::Member(t, g) if matches!(**t, Expr::This) && *g == m) || matches!(e, Expr::Name(g) if *g == m) { all += 1 }
                    if let Expr::Call(g, _) = e { if matches!(&**g, Expr::Member(t, x) if matches!(**t, Expr::This) && *x == m) || matches!(&**g, Expr::Name(x) if *x == m) { called += 1 } }
                });
                if all > called { as_value = true }
            }
        }
        if as_value { return None }
        let mut fixed: BTreeMap<u32, Expr> = BTreeMap::new();
        for r in (passed as u32 + 1)..=(n as u32) {
            // (its default, as the body's first test of it sets it; one the body writes again is left)
            let lit = f.body.iter().find_map(|st| match st {
                Stmt::If(Expr::Binary(BinOp::Eq, a, b), then, els) if els.is_empty() && **a == Expr::Local(r) && matches!(**b, Expr::Null | Expr::Undefined) => match then.as_slice() {
                    [Stmt::Assign(Place::Local(x), v)] if *x == r && matches!(v, Expr::Bool(_) | Expr::Int(_) | Expr::Num(_) | Expr::Str(_) | Expr::Null) => Some(v.clone()),
                    _ => None,
                },
                _ => None,
            }).or_else(|| f.params.get(r as usize - 1).and_then(|p| p.default.clone())).unwrap_or(Expr::Null);
            let mut writes = 0;
            walk(&f.body, &mut |st| if matches!(st, Stmt::Assign(Place::Local(x), _) if *x == r) { writes += 1 });
            if writes > 1 { return None }
            fixed.insert(r, lit);
        }
        let body: Vec<Stmt> = f.body.iter().filter(|st| !matches!(st, Stmt::If(Expr::Binary(BinOp::Eq, a, _), _, _) if matches!(**a, Expr::Local(r) if fixed.contains_key(&r)))).cloned().collect();
        let body = map_stmts(body, &mut |e| match &e { Expr::Local(r) if fixed.contains_key(r) => fixed[r].clone(), _ => e });
        Some(Function { body, ..f.clone() })
    }

    fn function(&mut self, name: &str, f: &Function) -> Result<String, String> {
        let held = if self.o.fighter { own_clip_held(f) } else { None };
        let f = held.as_ref().unwrap_or(f);
        let folded = if self.learned { self.unpassed_defaults(name, f) } else { None };
        let f = folded.as_ref().unwrap_or(f);
        self.body = f.body.clone();
        let outer = std::mem::replace(&mut self.current_fn, name.to_string());
        let r = self.function_written(name, f);
        self.current_fn = outer;
        r
    }

    fn function_written(&mut self, name: &str, f: &Function) -> Result<String, String> {
        if f.rest { return Err("takes the rest of its arguments".into()) }
        self.locals.clear();
        self.temp_ty.clear();
        self.rec_locals.clear();
        self.stage_locals.clear();
        self.empty_locals.clear();
        self.self_locals.clear();
        self.local_src.clear();
        self.part_locals.clear();
        self.effects = 0;
        let debug: BTreeMap<u32, String> = f.local_names.iter().cloned().collect();
        let mut params = Vec::new();
        // (a record's method: handed the record first)
        if self.record.is_some() && !name.ends_with("_new") { params.push("me".to_string()); }
        let mut defaults: Vec<String> = Vec::new();
        self.local_elem.clear();
        if !self.drawn.contains_key(&self.class) {
            let none = BTreeSet::new();
            let inert = self.o.runtime.inert_calls.get("BitmapData").unwrap_or(&none);
            // (a stage draws its pictures: `pixels`)
            let fields = if self.o.stage { BTreeSet::new() } else { self.o.model.classes.get(&self.class).map(|c| Drawing::of(c, self.o.model, inert).fields).unwrap_or_default() };
            self.drawn.insert(self.class.clone(), fields);
        }
        {
            let none = BTreeSet::new();
            let inert = self.o.runtime.inert_calls.get("BitmapData").unwrap_or(&none);
            let d = Drawing { inert, model: self.o.model, fields: self.drawn.get(&self.class).cloned().unwrap_or_default() };
            self.drawn_locals = if self.o.stage { BTreeSet::new() } else { d.locals(&f.body, f.params.len() as u32, self.o.model) };
        }
        for (i, p) in f.params.iter().enumerate() {
            let n = ident(&p.name);
            // (one left off takes the default it declares)
            if let Some(d) = p.default.as_ref().and_then(literal) { if d != "null" { defaults.push(format!("\tif ({n} == null) {n} = {d};")); } }
            params.push(format!("?{n}"));
            // (untyped: what every call of it hands it, where that is a record)
            let ty = match Ty::declared(p.type_name.as_deref()) { Ty::Any => self.param_ty.get(&(name.to_string(), i)).copied().unwrap_or(Ty::Any), t => t };
            // (one its code only reads fields of, never calling it or handing it on: a record)
            let ty = if ty == Ty::Any && only_fields_read(&f.body, i as u32 + 1) { Ty::Rect } else { ty };
            self.locals.insert(i as u32 + 1, (n, Holds::Value(ty)));
            match self.param_elem.get(&(name.to_string(), i)).copied() { Some(e) if !self.learning || e == Ty::Rect => { self.local_elem.insert(i as u32 + 1, e); } _ => { self.local_elem.remove(&(i as u32 + 1)); } }
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
        let np = std::mem::replace(&mut self.nparams, nparams);
        self.defaults = defaults;
        // (a parameter every call hands a clip of the planes: that clip)
        for i in 0..nparams as usize {
            if self.part_params.get(&(name.to_string(), i)) == Some(&true) { self.part_locals.insert(i as u32 + 1); }
        }
        let r = self.function_body(name, f, &params, nparams, &debug, &captured);
        self.nparams = np;
        self.frame_top = top;
        self.captured = outer;
        r
    }

    fn function_body(&mut self, name: &str, f: &Function, params: &[String], nparams: u32, debug: &BTreeMap<u32, String>, captured: &[String]) -> Result<String, String> {
        // (the decompiler's leftovers tidied first)
        let mut body = f.body.clone();
        tidy(&mut body);
        records(&mut body);
        aliases(&mut body);
        // (debug, `PEPTIDE_NATIVE_AST=Class.function`: its code as the writer takes it)
        if std::env::var("PEPTIDE_NATIVE_AST").is_ok_and(|w| w == format!("{}.{name}", short(&self.class))) {
            let mut t = String::new();
            crate::model::ast::print_stmts(&mut t, &body, 1);
            eprintln!("[ast {}.{name}]\n{t}", short(&self.class));
        }
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
        // (and the values it joins from two ways, `__j<block>_<slot>`)
        let mut joins = BTreeSet::new();
        walk(&body, &mut |s| if let Stmt::Assign(Place::Join { block, slot }, _) = s { joins.insert((*block, *slot)); });
        walk_exprs(&body, &mut |e| if let Expr::Join { block, slot } = e { joins.insert((*block, *slot)); });
        for (b, sl) in &joins { decl.push(format!("\tvar __j{b}_{sl} = null;")); }
        // (a frame's own variables, in the script's scope, keep clear of the names the script defines)
        let clear = |n: String, w: &Self| if w.frame_top && w.globals.contains(&n) { format!("{name}_{n}") } else { n };
        // (a variable the function only ever sets to a record, or to nothing, is that record from the
        // start: a write to its fields can come before the decompile's set of it)
        let mut record_regs: BTreeSet<u32> = BTreeSet::new();
        let mut other_regs: BTreeSet<u32> = BTreeSet::new();
        {
            let rt = self.o.runtime;
            let is_rec = |v: &Expr| {
                let mut v = v;
                while let Expr::Coerce(_, x) | Expr::Convert(_, x) = v { v = x; }
                matches!(v, Expr::Object(_))
                    || matches!(v, Expr::New(c, _) if !matches!(&**c, Expr::Name(_) | Expr::TypeApply(..)))
                    || matches!(v, Expr::New(c, _) if matches!(&**c, Expr::Name(n) if matches!(short(n), "Object" | "Point" | "Rectangle" | "MovieClip" | "Sprite") || (rt.native(n).is_some() && short(n) != "Array" && short(n) != "Vector" && short(n) != "Dictionary")))
            };
            // (through the decompiler's temporaries: `$t = new Object(); r = $t`)
            let mut rec_temps: BTreeSet<u32> = BTreeSet::new();
            walk(&body, &mut |s| if let Stmt::Assign(Place::Temp(t), v) = s { if is_rec(v) { rec_temps.insert(*t); } });
            walk(&body, &mut |s| if let Stmt::Assign(Place::Local(r), v) = s {
                let rec = is_rec(v) || matches!(v, Expr::Temp(t) if rec_temps.contains(t));
                if rec { record_regs.insert(*r); } else if !matches!(v, Expr::Null | Expr::Undefined | Expr::Temp(_) | Expr::Local(_)) { other_regs.insert(*r); }
            });
        }
        for r in &regs {
            if copies.contains(r) {
                let n = clear(debug.get(r).map(|s| ident(s)).unwrap_or_else(|| format!("v{r}")), self);
                self.locals.insert(*r, (n, Holds::Clip));
                continue;
            }
            let n = clear(debug.get(r).map(|s| ident(s)).unwrap_or_else(|| format!("v{r}")), self);
            decl.push(format!("\tvar {n} = null;"));
            let ty = if record_regs.contains(r) && !other_regs.contains(r) { Ty::Rect } else { Ty::Any };
            self.locals.insert(*r, (n, Holds::Value(ty)));
        }
        // (a body's closing `return;` says nothing)
        if matches!(body.last(), Some(Stmt::Return(None))) { body.pop(); }
        let mut lines = decl;
        lines.append(&mut std::mem::take(&mut self.defaults));
        match self.block(&body, 1) {
            Ok(l) => lines.extend(l),
            Err(e) => {
                if std::env::var("PEPTIDE_NATIVE_WHY").is_ok() { eprintln!("{name}: {e}"); }
                return Err(e);
            }
        }
        let head = if name.is_empty() { "function".to_string() } else { format!("function {name}") };
        Ok(format!("{head}({}) {{\n{}\n}}", params.join(", "), lines.join("\n")).replace("{\n\n}", "{\n}"))
    }

    fn block(&mut self, list: &[Stmt], depth: usize) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        // (`PEPTIDE_CENSUS_ALL`: every statement that has no plain form yet is told, not only the first)
        let all = std::env::var("PEPTIDE_CENSUS_ALL").is_ok();
        let none = BTreeSet::new();
        let inert = self.o.runtime.inert_calls.get("BitmapData").unwrap_or(&none);
        let d = Drawing { inert, model: self.o.model, fields: self.drawn.get(&self.class).cloned().unwrap_or_default() };
        let mut dropped = false;
        let mut first: Option<String> = None;
        for s in list {
            // (drawing that comes to nothing here, and what only feeds it: said once)
            if !self.o.stage && d.void(s, &self.drawn_locals, self.o.model) {
                if !dropped { out.push(format!("{}// (not ported: the source draws pixels here, which this engine never shows)", "\t".repeat(depth))); dropped = true; }
                continue;
            }
            match self.stmt(s, depth) {
                Ok(t) => {
                    // (a block closed just before one that opens with `(` reads as calling it: ended first)
                    if t.iter().map(|l| l.trim()).find(|l| !l.is_empty() && !l.starts_with("//")).is_some_and(|l| l.starts_with('(')) {
                        if let Some(last) = out.iter_mut().rev().find(|l| { let l = l.trim(); !l.is_empty() && !l.starts_with("//") }) {
                            if last.trim_end().ends_with('}') { last.push(';'); }
                        }
                    }
                    out.extend(t)
                }
                Err(e) if all && self.strict => {
                    eprintln!("[census-all] {}: {e} @@ {}", self.current_fn, format!("{s:?}").chars().take(400).collect::<String>());
                    // (told, and still the block's failure: listing every one never passes it)
                    if first.is_none() { first = Some(e); }
                }
                Err(e) if self.strict => return Err(e),
                // a statement with no plain form yet is left out and said, in the script and in
                // the object's gaps; the rest of its code is written as it is
                Err(e) => {
                    let why = e.replace(['\n', '\r'], " ");
                    out.push(format!("{}// (not ported: {why})", "\t".repeat(depth)));
                    self.gaps.insert(format!("not ported: {why}"));
                }
            }
        }
        if let Some(e) = first { return Err(e) }
        Ok(out)
    }

    fn stmt(&mut self, s: &Stmt, depth: usize) -> Result<Vec<String>, String> {
        let tab = "\t".repeat(depth);
        Ok(match s {
            // an entry taken out of a map it keeps (`delete map[k]`), by the key it was put in under
            Stmt::Expr(Expr::Delete(d)) if matches!(&**d, Place::Index(a, _) if self.is_obj_dict(a) || self.is_dict(a)) => {
                let Place::Index(a, k) = &**d else { unreachable!() };
                let text = self.is_obj_dict(a);
                let (m, _) = self.expr(a)?;
                let (k, _) = self.expr(k)?;
                vec![if text { format!("{tab}{}.remove(Std.string({k}));", paren(&m)) } else { format!("{tab}{}.remove({k});", paren(&m)) }]
            }
            Stmt::Expr(e) => {
                let t = self.statement_call(e)?;
                if t.is_empty() { vec![] } else { t.lines().map(|l| format!("{tab}{l}")).collect() }
            }
            Stmt::Assign(p, v) => {
                if let Place::Local(i) = p { self.local_src.insert(*i, v.clone()); }
                self.assign(p, v)?.into_iter().map(|l| format!("{tab}{l}")).collect()
            }
            Stmt::If(c, then, els) => {
                let test = self.test(c)?;
                // (a test that always passes is its body)
                if test == "true" { return self.block(then, depth) }
                // (one that never passes is what it does otherwise: `false && ...` never looks further)
                if test == "false" || (test.starts_with("false && ") && !test.contains("||")) { return self.block(els, depth) }
                let mut out = vec![format!("{tab}if ({test}) {{")];
                out.extend(self.block(then, depth + 1)?);
                if !els.is_empty() {
                    out.push(format!("{tab}}} else {{"));
                    out.extend(self.block(els, depth + 1)?);
                }
                out.push(format!("{tab}}}"));
                out
            }
            // a loop over the names an object, list or map holds (`for (k in o)`): over its keys
            Stmt::While(c, b) if for_in(c).is_some() => {
                let Some((o, i)) = for_in(c) else { unreachable!() };
                let k = 60000 + i;
                let mut f = |e: Expr| match e {
                    Expr::NextName(_, x) if *x == Expr::Local(i) => Expr::Local(k),
                    Expr::NextValue(oo, x) if *x == Expr::Local(i) => Expr::Index(oo, Box::new(Expr::Local(k))),
                    e => e,
                };
                let body = map_stmts(b.clone(), &mut f);
                let name = format!("k{i}");
                self.locals.insert(k, (name.clone(), Holds::Value(Ty::Any)));
                let (ov, _) = self.expr(&Expr::Local(o))?;
                // (a list's places or a map's keys, written as the loop over them where what it walks
                // is known; the source's loop over nothing runs no times, where the engine's stops)
                // (through the variables it was copied from, to what was first put in one)
                let mut src = self.local_src.get(&o).cloned();
                for _ in 0..8 { match src { Some(Expr::Local(j)) => src = self.local_src.get(&j).cloned(), _ => break } }
                // (or a value written as a list in place, `[...]`: one of the API's lists)
                let literal = match &src {
                    Some(Expr::Array(_)) => true,
                    Some(e) => self.expr(e).map(|(t, _)| t.trim_start().starts_with('[')).unwrap_or(false),
                    None => false,
                };
                let kind = if literal { Some(true) } else { src.as_ref().and_then(|e| self.walks_list(e)) };
                let head = match kind {
                    Some(true) if literal => format!("for ({name} in 0...{ov}.length) {{"),
                    Some(true) => format!("if ({ov} != null) for ({name} in 0...{ov}.length) {{"),
                    Some(false) => format!("if ({ov} != null) for ({name} in {ov}.keys()) {{"),
                    None => { self.util("keysIn")?; format!("for ({name} in keysIn({ov})) {{") }
                };
                let mut out = vec![format!("{tab}{head}")];
                out.extend(self.block(&body, depth + 1)?);
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
            Stmt::Return(Some(e)) => {
                let (v, ty) = self.expr(e)?;
                if self.learning && !self.current_fn.is_empty() {
                    let was = self.returns.get(&self.current_fn).copied();
                    self.returns.insert(self.current_fn.clone(), match was { None => ty, Some(w) if w == ty => w, _ => Ty::Any });
                }
                vec![format!("{tab}return {v};")]
            }
            Stmt::Switch(v, cases, default) => {
                let mut out = vec![format!("{tab}switch ({}) {{", self.expr(v)?.0)];
                // (each case's code is all it runs: the source's run on into the next case is in it
                // already, as the model reads it)
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
            // an error it raises: raised here too, its message as it states it (the engine reports it)
            Stmt::Throw(e) => {
                let msg = match e { Expr::New(n, a) if matches!(&**n, Expr::Name(n) if short(n).ends_with("Error")) => a.first().cloned().unwrap_or(Expr::Str("error".into())), other => other.clone() };
                let (m, _) = self.expr(&msg)?;
                vec![format!("{tab}throw {m};")]
            }
            Stmt::Try(..) => return Err("throws or catches".into()),
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
                if self.o.clip_parts.contains_key(&self.class) && matches!(m.as_str(), "gotoAndStop" | "gotoAndPlay" | "stop" | "play") {
                    let who = format!("planePart({})", quote(&self.o.clip_parts[&self.class]));
                    self.util("planePart")?;
                    let (t, _) = self.api(&who, m, args)?;
                    return Ok(format!("{t};"));
                }
                // (a clip record of its own: its own timeline, below)
                if !self.record.as_ref().is_some_and(|k| self.clip_recs.contains(k)) {
                    match (m.as_str(), args.as_slice()) {
                        // the engine plays the clip the object shows on every frame, whatever a stop on
                        // it says
                        ("stop" | "play", []) => return Ok(String::new()),
                        ("gotoAndStop" | "gotoAndPlay", [Expr::Str(label)]) => return self.jump(label),
                        ("gotoAndStop" | "gotoAndPlay", [label]) if self.counts_frames() => return Ok(format!("{};", self.jump_to_value(label)?)),
                        _ => {}
                    }
                }
            }
        }
        // the clip that is an object's whole art taking itself off its parent: the object ends
        if let Expr::Call(f, a) = e {
            if matches!(&**f, Expr::Member(_, rc) if rc == "removeChild") && matches!(a.as_slice(), [Expr::This]) && self.is_root_clip() {
                return Ok("self.kill();".into());
            }
        }
        // a clip of the planes taking itself off its parent: no longer drawn
        if let Expr::Call(f, a) = e {
            if matches!(&**f, Expr::Member(_, rc) if rc == "removeChild") && matches!(a.as_slice(), [Expr::This]) && self.o.clip_parts.contains_key(&self.class) {
                self.util("planePart")?;
                return Ok(format!("if (planePart({0}) != null) planePart({0}).setVisible(false);", quote(&self.o.clip_parts[&self.class])));
            }
        }
        // a named clip its own clip places, moved to a frame: its art is drawn into the fighter's
        // frames, so nothing here turns it (noted where it was, and in the report)
        if let Expr::Call(f, _) = e {
            if let Expr::Member(c, m) = &**f {
                if matches!(m.as_str(), "gotoAndStop" | "gotoAndPlay" | "play" | "stop" | "nextFrame" | "prevFrame") {
                    // (one of its parts: its own vfx, sent to the frame)
                    if let Some(name) = self.part_of(c) {
                        let Expr::Call(_, args) = e else { unreachable!() };
                        let v = format!("{}.get()", part_id(&name));
                        let hold = matches!(m.as_str(), "gotoAndStop" | "stop");
                        return match (m.as_str(), args.as_slice()) {
                            ("stop" | "play", []) => Ok(format!("placeParts();\nif ({v} != null) {v}.{}();", if hold { "pause" } else { "resume" })),
                            // (to a label: the part's own, as its effect's animation carries its clip's labels)
                            ("gotoAndStop" | "gotoAndPlay", [Expr::Str(l)]) => Ok(format!("placeParts();\nif ({v} != null) {{ {v}.playFrameLabel({}); {v}.{}(); }}", quote(l), if hold { "pause" } else { "resume" })),
                            ("gotoAndStop" | "gotoAndPlay", [to]) => {
                                let (to, _) = self.expr(to)?;
                                self.effect_jump()?;
                                Ok(format!("partFrame({}, {to}, {hold});", part_id(&name)))
                            }
                            _ => Err(format!("calls {m} on its {name} part")),
                        };
                    }
                    // (a clip a clip record of its own places that is drawn apart, as the effect the
                    // record is drawn as carries it: its own timeline, `screenClip`)
                    if let Expr::Member(t, n) = &**c {
                        let drawn_apart = matches!(**t, Expr::This) && self.record.as_ref().is_some_and(|k| self.clip_recs.contains(k)
                            && self.o.effect_inners.get(short(k)).is_some_and(|ics| ics.iter().any(|ic| &ic.name == n)));
                        if drawn_apart {
                            let Expr::Call(_, args) = e else { unreachable!() };
                            self.util("clipRun")?;
                            let hold = matches!(m.as_str(), "gotoAndStop" | "stop");
                            let r = format!("me.{n}");
                            return Ok(match (m.as_str(), args.first()) {
                                ("gotoAndStop" | "gotoAndPlay", Some(a)) => { let (a, _) = self.expr(a)?; format!("clipGo({r}, {a}, {hold});") }
                                ("nextFrame" | "prevFrame", _) => format!("clipStep({r}, {});", if m == "nextFrame" { 1 } else { -1 }),
                                _ => format!("clipPlay({r}, {});", !hold),
                            });
                        }
                    }
                    // (or a clip inside one, whose art is drawn into the fighter's with it)
                    let mut at: &Expr = c;
                    let mut inside = None;
                    while let Expr::Member(o, n) = at {
                        if self.stance_child(at).is_some() || self.own_child(at).is_some() { break }
                        inside = Some(n.clone());
                        at = o;
                    }
                    let within = inside.is_some() && (self.stance_child(at).is_some() || self.own_child(at).is_some());
                    if let Some(name) = self.stance_child(c).map(|(n, _)| n).or_else(|| self.own_child(c)).or_else(|| if within { inside.clone() } else { None }) {
                        self.gaps.insert(format!("steps its {name} clip, whose art is drawn into the fighter's"));
                        return Ok(format!("// (its {name} clip's art is drawn into the fighter's: its {m} turns nothing here)"));
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
        // (a clip record's look: its plain field, drawn nowhere here)
        if let Place::Member(x, f) = p {
            if matches!(f.as_str(), "x" | "y" | "visible" | "alpha" | "scaleX" | "scaleY" | "rotation" | "width" | "height") && self.clip_rec_value(x) {
                let (r, _) = self.expr(x)?;
                let (val, _) = self.expr(v)?;
                return Ok(vec![format!("{}.{f} = {val};", paren(&r))]);
            }
        }
        // (one of the API's classes kept as a record: made in its record form, its properties
        // set through it)
        if let Place::Member(h, f) = p {
            if let (Expr::This, Some(k)) = (h, self.rt_fields.get(&member_key(&self.class, f)).cloned()) {
                let mut x = v; while let Expr::Coerce(_, y) = x { x = y; }
                if let Expr::New(_, a) = x {
                    self.rt_record(&k)?;
                    let a: Vec<String> = a.iter().map(|e| self.expr(e).map(|(t, _)| t)).collect::<Result<_, _>>()?;
                    let (fid, _) = self.field(f).ok_or("no field")?;
                    return Ok(vec![format!("{fid}.set({k}_new({}));", a.join(", "))]);
                }
            }
            if let Some(k) = self.lit_rt_held(h) {
                let t = flat_template(self.o.runtime, &format!("{k} as a record")).unwrap_or_default();
                if crate::dyn_re(&format!(r"\bfunction\s+{}_set_{}\(", regex::escape(&k), regex::escape(f))).is_match(&t) {
                    self.rt_record(&k)?;
                    let (held, _) = self.expr(h)?;
                    let (val, _) = self.expr(v)?;
                    return Ok(vec![format!("{k}_set_{f}({held}, {val});")]);
                }
            }
        }
        // (a value's metadata written under a key no code of the package reads back: nothing to keep)
        if let Place::Member(Expr::Member(_, md), k) = p {
            if md == "metadata" {
                let mut read = false;
                for c in self.o.model.classes.values() {
                    for b in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
                        walk_exprs(b, &mut |e| if matches!(e, Expr::Member(_, n) if n == k) || matches!(e, Expr::Str(n) if n == k) { read = true });
                    }
                }
                if !read { return Ok(vec![format!("// (its {k} written into a metadata no code reads it from)")]) }
            }
        }
        // (a field of something its code names by a field of its own, bare: through that field)
        if let Place::Member(Expr::Name(n), f) = p {
            if self.field(n).is_some() && !self.captured.contains(&ident(n)) && !self.locals.values().any(|(l, _)| l == n) {
                return self.assign(&Place::Member(Expr::Member(Box::new(Expr::This), n.clone()), f.clone()), v)
            }
        }
        // (a write through the scope chain of the very name it writes is that name, as a read is)
        if let Place::Member(Expr::ScopeOf(m), f) = p {
            if m == f {
                let to = if self.field(f).is_some() { Place::Member(Expr::This, f.clone()) } else { Place::Name(f.clone()) };
                return self.assign(&to, v)
            }
        }
        // (its own clip's look, written: its own, as this engine draws the clip as the object)
        if let Place::Member(Expr::Call(g, a), f) = p {
            if a.is_empty() && !self.o.fighter && matches!(f.as_str(), "scaleX" | "scaleY" | "alpha" | "rotation" | "visible")
                && (matches!(&**g, Expr::Member(o, m) if matches!(m.as_str(), "getMC" | "stanceClip" | "getStanceMC") && self.expr(o).is_ok_and(|(w, _)| w == "self"))
                    || matches!(&**g, Expr::Name(m) if matches!(m.as_str(), "getMC" | "stanceClip" | "getStanceMC") && self.method(m).is_none())) {
                let (val, _) = self.expr(v)?;
                let set = match f.as_str() { "scaleX" => "setScaleX", "scaleY" => "setScaleY", "alpha" => "setAlpha", "rotation" => "setRotation", _ => "setVisible" };
                return Ok(vec![format!("self.{set}({val});")]);
            }
        }
        // (the look of the clip a projectile's or item's animation shows, written in that clip's own
        // code: the object's, as this engine draws the clip as the object)
        if let Place::Member(Expr::This, f) = p {
            if !self.o.fighter && !self.o.stage && matches!(f.as_str(), "scaleX" | "scaleY" | "alpha" | "rotation" | "visible") && self.field(f).is_none()
                && self.o.itself.is_none_or(|c| c.name != self.class) {
                let (val, _) = if f == "visible" { (self.test(v)?, Ty::Bool) } else { self.expr(v)? };
                let set = match f.as_str() { "scaleX" => "setScaleX", "scaleY" => "setScaleY", "alpha" => "setAlpha", "rotation" => "setRotation", _ => "setVisible" };
                return Ok(vec![format!("self.{set}({val});")]);
            }
        }
        // (the place of the clip that is a projectile's or item's whole art, written in that clip's
        // own code: the object's, as this engine draws the clip as the object)
        if let Place::Member(Expr::This, f) = p {
            if matches!(f.as_str(), "x" | "y") && self.is_root_clip() && self.field(f).is_none() {
                let (val, _) = self.expr(v)?;
                let set = if f == "x" { "setX" } else { "setY" };
                return Ok(vec![format!("self.{set}({} * {});", paren(&val), number(self.o.size))]);
            }
        }
        // (its clip's filters, written: its colours here are the engine's shaders, `setColorFilters`)
        if let Place::Member(o, f) = p {
            // (the colour settings it read off them, put back: `colorFilters`)
            if f == "filters" && self.own_clip_call(o) {
                let (val, _) = self.expr(v)?;
                self.util("colorFilters")?;
                return Ok(vec![format!("colorFilters(self, {val});")]);
            }
        }
        // (an object its code uses as a map, made: a map by text)
        if let Place::Member(Expr::This, f) = p {
            if (matches!(v, Expr::Object(kv) if kv.is_empty()) || matches!(v, Expr::New(c, a) if a.is_empty() && matches!(&**c, Expr::Name(n) if short(n) == "Array"))) && self.is_obj_dict(&Expr::Member(Box::new(Expr::This), f.clone())) {
                return self.assign(p, &Expr::New(Box::new(Expr::Name("TextMap".into())), vec![]));
            }
        }
        // (a blend mode set on a part or an effect: a screen or add draws the conversion's copy of its
        // art that carries the colour in its alpha, any other the art itself, `screenLook`)
        if let Place::Member(o, f) = p {
            if f == "blendMode" {
                let mode = match v { Expr::Member(c, m) if matches!(&**c, Expr::Name(c) if short(c) == "BlendMode") => Some(m.as_str()), Expr::Str(m) => Some(m.as_str()), _ => None };
                let on = match mode.map(|m| m.to_ascii_uppercase()) {
                    Some(m) if m == "SCREEN" || m == "ADD" => true,
                    Some(m) if m == "NORMAL" || m == "LAYER" => false,
                    _ => {
                        self.gaps.insert("sets a blend mode, which this engine has none of as it runs".into());
                        return Ok(vec!["// (not ported: a blend mode set here; this engine has none as it runs)".into()]);
                    }
                };
                let target = match self.part_of(o) { Some(name) => format!("partNow({})", part_id(&name)), None => self.expr(o)?.0 };
                self.util("screenLook")?;
                return Ok(vec![format!("screenLook({target}, {on});")]);
            }
        }
        // (a constant of its class written the value it declares: nothing changes)
        if let Place::Member(Expr::This, f) = p {
            let declared = self.o.model.classes.get(&self.class).and_then(|c| c.fields.iter().find(|x| x.name == *f && x.is_const)).and_then(|x| x.default.as_ref()).and_then(literal);
            if declared.is_some() && declared == literal(v) { return Ok(vec![format!("// (its constant {f} written the value it declares)")]) }
        }
        // (a plain value a clip declares about itself that no code of its package reads back: what the
        // source's engine reads off the clip, which the conversion read off the tree)
        if let Place::Member(Expr::This, f) = p {
            let plain = matches!(v, Expr::Str(_) | Expr::Num(_) | Expr::Int(_) | Expr::Bool(_));
            if plain && self.field(f).is_none() && !matches!(f.as_str(), "visible" | "alpha" | "x" | "y" | "scaleX" | "scaleY" | "rotation" | "name") && !self.read_anywhere(f) {
                return Ok(vec![format!("// (it declares its {f}, which the conversion read off the tree)")]);
            }
        }
        // (the class a clip's frame declares the game builds over it: read at conversion, the object's own)
        if matches!(p, Place::Member(Expr::This, f) if f == "classAPI") && matches!(v, Expr::Name(_) | Expr::Member(..)) { return Ok(vec![]) }
        // a flattened field made anew, and a field of one written
        if let Place::Member(t, f) = p {
            let whole = Expr::Member(Box::new(t.clone()), f.clone());
            if let Some((id, k)) = self.flat_of(&whole) {
                if !id.contains('.') { self.kept.insert(format!("{id}_isMade"), format!("var {id}_isMade = false;")); }
                // (nothing, or a cast that answers null: not made)
                let mut inner = v;
                while let Expr::Coerce(_, x) = inner { inner = x; }
                if matches!(inner, Expr::Null | Expr::Undefined | Expr::Binary(BinOp::As, ..)) { return Ok(vec![format!("{id}_isMade = false;")]) }
                let args: Vec<String> = match inner {
                    Expr::New(_, a) => a.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?,
                    _ => Vec::new(),
                };
                return Ok(vec![format!("{}; {id}_isMade = true;", self.flat_call(&id, &k, "new", &args)?.0)]);
            }
            if let Some((id, k)) = self.flat_of(t) {
                let (val, _) = self.expr(v)?;
                return Ok(vec![format!("{};", self.flat_call(&id, &k, &format!("set_{f}"), &[val])?.0)]);
            }
        }
        match p {
            // a list of API records: started empty, an element made, as a copy written back
            Place::Member(Expr::This, f) if self.rt_list(f).is_some() && matches!(v, Expr::New(..) | Expr::Array(_) | Expr::Call(..)) => {
                let (fid, _) = self.field(f).ok_or("a list of no field")?;
                Ok(vec![format!("{fid}.set([]);")])
            }
            Place::Index(Expr::Member(t, f), i) if matches!(**t, Expr::This) && self.rt_list(f).is_some() => {
                let k = self.rt_list(f).unwrap_or_default();
                self.rt_record(&k)?;
                let (fid, _) = self.field(f).ok_or("a list of no field")?;
                let (ix, _) = self.expr(i)?;
                let val = match v {
                    Expr::New(_, args) => { let vs: Vec<String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?; format!("{k}_new({})", vs.join(", ")) }
                    other => self.expr(other)?.0,
                };
                Ok(vec![format!("{{ var __l = {fid}.get().copy(); __l[{ix}] = {val}; {fid}.set(__l); }}")])
            }
            Place::Member(Expr::Index(l, i), prop) if matches!(&**l, Expr::Member(t, f) if matches!(**t, Expr::This) && self.rt_list(f).is_some()) => {
                let Expr::Member(_, f) = &**l else { unreachable!() };
                let k = self.rt_list(f).unwrap_or_default();
                self.rt_record(&k)?;
                let (fid, _) = self.field(f).ok_or("a list of no field")?;
                let (ix, _) = self.expr(i)?;
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{{ var __l = {fid}.get().copy(); var __r = {k}_copy(__l[{ix}]); {k}_set_{prop}(__r, {val}); __l[{ix}] = __r; {fid}.set(__l); }}")])
            }
            // the clip it shows hidden or shown: the object itself, as it is the object's look
            Place::Member(Expr::This, f) if f == "visible" && self.field(f).is_none() && !self.o.clip_parts.contains_key(&self.class) => {
                let t = self.test(v)?;
                Ok(vec![format!("self.setVisible({t});")])
            }
            // the clip it shows scaled from inside (its parent, the object's own clip): the object itself
            Place::Member(Expr::Name(p), f) if p == "parent" && matches!(f.as_str(), "scaleX" | "scaleY")
                && self.o.model.classes.get(&self.class).and_then(|c| c.symbol) != Some(self.o.root) => {
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("self.set{}({val});", if f == "scaleX" { "ScaleX" } else { "ScaleY" })])
            }
            // a colour transform set on its own clip: the fighter shows it, as one shader the values
            // are copied into (the source copies the transform as it is set)
            Place::Member(Expr::Member(mc, t), f) if self.o.fighter && t == "transform" && f == "colorTransform"
                && matches!(&**mc, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "getMC" | "getStanceMC" | "stanceClip") && self.expr(o).is_ok_and(|(w, _)| w == "self"))) => {
                let (val, _) = self.expr(v)?;
                self.blocks.insert("tint".into());
                Ok(vec![format!("showTint({val});")])
            }
            // a colour transform set on a clip it holds (an effect, a part): a colour shader on it
            Place::Member(Expr::Member(x, t), f) if t == "transform" && f == "colorTransform" => {
                let clip = self.held_clip(x)?;
                let (val, _) = self.expr(v)?;
                if !self.helpers.contains_key("tintClip") { self.helpers.insert("tintClip".into(), helper(self.o.runtime, "helpers", "tint_clip", "tintClip", &[], &[])?); }
                Ok(vec![format!("tintClip({clip}, {val});")])
            }
            // (a field of the object itself, through a variable that holds it: written as the object's)
            Place::Member(Expr::Local(r), f) if self.self_locals.contains_key(r) => {
                let o = self.self_locals[r].clone();
                self.assign(&Place::Member(o, f.clone()), v)
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
            // a clip of a stage's planes writing its own visibility or opacity: its part's
            Place::Member(Expr::This, f) if self.o.clip_parts.contains_key(&self.class) && matches!(f.as_str(), "visible" | "alpha") && self.field(f).is_none() => {
                self.util("planePart")?;
                let p = format!("planePart({})", quote(&self.o.clip_parts[&self.class]));
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("if ({p} != null) {p}.{}({val});", if f == "visible" { "setVisible" } else { "setAlpha" })])
            }
            // a surface it holds let through or not: its structures' own
            Place::Member(o, f) if f == "fallthrough" && !matches!(o, Expr::This) && self.expr(o).is_ok_and(|(w, _)| w != "self" && !w.starts_with("planePart(")) => {
                let (who, _) = self.expr(o)?;
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("if ({who} != null && {who}.updateStructureStats != null) {who}.updateStructureStats({{ dropThrough: {val} == true }});")])
            }
            // whether an object's clip shows, written: whether the object does (its own, or another's)
            Place::Member(mc, f) if matches!(f.as_str(), "visible" | "alpha")
                && matches!(mc, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(_, m) | Expr::Name(m) if matches!(m.as_str(), "getMC" | "stanceClip" | "getStanceMC"))) => {
                let Expr::Call(g, _) = mc else { unreachable!() };
                let who = match &**g { Expr::Member(o, _) => self.expr(o)?.0, _ => "self".to_string() };
                if who.starts_with("planePart(") { return Err("writes a field of a clip of its planes' own clip".into()) }
                let (val, _) = self.expr(v)?;
                Ok(vec![if f == "visible" { format!("{who}.setVisible({val});") } else { format!("{who}.setAlpha({val});") }])
            }
            // the stage's light moved by its code: the engine's stays where the conversion put it
            Place::Member(Expr::Call(g, a), f) if a.is_empty() && matches!(&**g, Expr::Member(_, m) | Expr::Name(m) if m == "getLightSourceMC") => {
                let (val, _) = self.expr(v)?;
                // (the engine reads a stage's light from the animation it plays: `Stage.updateLightboxStats`
                // has no effect, playing another animation rebuilds the stage's containers, fighters and
                // all, and the frame is the stage's own clock)
                // (along the stage: the stage casts the fighters' shadows from where it puts it, `followLight`)
                // (its helper is the stage's own, written with the light it starts from: `native_stage`)
                if f == "x" && self.o.stage { return Ok(vec![format!("followLightAt({val});")]) }
                Ok(vec![format!("// (not ported: the source moves its light's {f} here; this engine's stage light cannot move as it runs)")])
            }
            // the stage's camera or death bounds moved by its code: the engine's stay where the
            // conversion put them
            Place::Member(Expr::Call(g, a), f) if a.is_empty() && matches!(&**g, Expr::Member(_, m) | Expr::Name(m) if matches!(m.as_str(), "getCameraBounds" | "getDeathBounds"))
                && (self.o.stage || matches!(&**g, Expr::Member(o, _) if self.expr(o).is_ok_and(|(w, _)| w == "stage"))) => {
                let (Expr::Member(_, m) | Expr::Name(m)) = &**g else { unreachable!() };
                let (val, _) = self.expr(v)?;
                self.util("stageBounds")?;
                Ok(vec![format!("stageBoundsSet({}, {}, {val});", m == "getCameraBounds", quote(f))])
            }
            // a field of a named clip of the planes written: shown, faded, let through (a field its own
            // class declares is its own, below)
            Place::Member(o, f) if !(matches!(o, Expr::This) && self.field(f).is_some()) && (self.expr(o).is_ok_and(|(w, _)| w.starts_with("planePart(")) || self.part_field(o)) => {
                let (p, _) = self.expr(o)?;
                // (its text: this engine draws no text a script writes)
                if matches!(f.as_str(), "text" | "htmlText") {
                    self.gaps.insert("writes the text of a clip of its planes, which this engine draws none of".into());
                    return Ok(vec!["// (the text of a clip of its planes: this engine draws no text a script writes)".into()]);
                }
                let (val, _) = self.expr(v)?;
                let call = match f.as_str() { "visible" => "setVisible", "alpha" => "setAlpha", "fallthrough" => "setFallthrough",
                    "x" => "setX", "y" => "setY", "scaleX" => "setScaleX", "scaleY" => "setScaleY", "rotation" => "setRotation", "width" => "setWidth", "height" => "setHeight",
                    // (a name no display property is, and no clip of its: a field its code keeps on the clip)
                    _ if !f.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && !matches!(f.as_str(), "currentFrame" | "totalFrames" | "currentLabel" | "numChildren" | "name" | "parent") => {
                        self.part_written.insert(f.clone());
                        return Ok(vec![format!("if ({p} != null) {p}.{} = {val};", ident(f))]);
                    }
                    _ => return Err(format!("writes the {f} of a clip of its planes, which no plain form answers yet")) };
                Ok(vec![format!("if ({p} != null) {p}.{call}({val});")])
            }
            // a static field of one of its classes, written by the class's name: its field
            Place::Member(Expr::Name(n), f) if self.static_class(n, f).is_some() => {
                let owner = self.static_class(n, f).unwrap_or_default();
                let saved = std::mem::replace(&mut self.class, owner);
                let r = self.assign(&Place::Member(Expr::This, f.clone()), v);
                self.class = saved;
                r
            }
            Place::Member(Expr::This, f) => {
                // (a value read already, where the write is another class's field: `given`)
                let given = self.given.take();
                if self.field(f).is_none() {
                    if let Some(set) = self.method(&format!("set:{f}")) { let (val, _) = match given { Some(g) => g, None => self.expr(v)? }; return Ok(vec![format!("{set}({val});")]) }
                }
                let (id, holds) = self.field(f).ok_or_else(|| format!("writes {f}, which is not its field"))?;
                if holds == Holds::Itself { return Ok(vec![]) }
                let (val, ty) = match given { Some(g) => g, None => self.expr(v)? };
                // what it learns of the field's value
                // (an object put in a field declared a number is an object's: a sound it played, kept to stop it)
                let key = member_key(&self.class, f);
                // (a rectangle put in a field that held nothing before reads as one)
                // (learning: every kind of value the code puts in it, joined)
                if self.learning {
                    // (a list written out whole, of records or of lists of them: its elements are records)
                    fn records_at_leaves(e: &Expr) -> bool { match e { Expr::Array(v) => !v.is_empty() && v.iter().all(records_at_leaves), Expr::Object(_) => true, _ => false } }
                    let mut lit = v;
                    while let Expr::Coerce(_, x) | Expr::Convert(_, x) = lit { lit = x; }
                    if matches!(lit, Expr::Array(_)) && records_at_leaves(lit) { self.list_elem.insert(key.clone(), Ty::Rect); }
                    // (a list made of its elements, each a record: pictures of its library, say)
                    let items = match lit { Expr::Array(v) => Some(v.as_slice()), Expr::New(c, a) if matches!(&**c, Expr::Name(n) if short(n) == "Array") && (a.len() > 1 || matches!(a.first(), Some(Expr::New(..)))) => Some(a.as_slice()), _ => None };
                    if let Some(items) = items.filter(|v| !v.is_empty()) {
                        if items.iter().all(|x| self.expr(x).is_ok_and(|(_, t)| t == Ty::Rect)) { self.list_elem.insert(key.clone(), Ty::Rect); }
                    }
                    // (a typed list of points or boxes, made in a temporary and filled there)
                    if let Expr::Temp(t) = lit {
                        let mut made: Option<Expr> = None;
                        let mut filled = false;
                        walk(&self.body, &mut |st| match st {
                            Stmt::Assign(Place::Temp(x), m) if x == t => made = Some(m.clone()),
                            Stmt::Assign(Place::Index(Expr::Temp(x), _), e) if x == t && matches!(e, Expr::New(c, _) if matches!(&**c, Expr::Name(n) if matches!(short(n), "Point" | "Rectangle"))) => filled = true,
                            _ => {}
                        });
                        let typed = matches!(&made, Some(Expr::New(c, _)) if matches!(&**c, Expr::TypeApply(_, ks) if matches!(ks.first(), Some(Expr::Name(n)) if matches!(short(n), "Point" | "Rectangle"))));
                        if typed || filled { self.list_elem.insert(key.clone(), Ty::Rect); }
                    }
                    let was = self.seen.get(&key).copied();
                    self.seen.insert(key.clone(), match was { None => ty, Some(w) => join(w, ty) });
                    // (a part of the planes, every time: the field is one)
                    let part = val.starts_with("planePart(") || val == "null";
                    let had = self.part_fields.get(&key).copied().unwrap_or(true);
                    self.part_fields.insert(key.clone(), had && part);
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
            // (a parameter given the fighter itself where none is passed: a value like any other)
            Place::Local(r) if matches!(v, Expr::This) && self.is_itself() && *r >= 1 && *r <= self.nparams => {
                let (n, _) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                Ok(vec![format!("{n} = self;")])
            }
            Place::Local(r) if matches!(v, Expr::This) => {
                let (n, _) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                self.locals.insert(*r, (n, Holds::Clip));
                Ok(vec![])
            }
            Place::Member(Expr::Local(r), f) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Clip) => self.assign(&Place::Member(Expr::This, f.clone()), v),
            // the look of an effect or object it keeps
            Place::Member(o, f) if !matches!(o, Expr::This) && matches!(f.as_str(), "x" | "y") && !self.is_clip(o) && !matches!(o, Expr::Name(n) if n == "parent")
                // (a record's, a point's, is its plain field: below)
                && !self.expr(o).is_ok_and(|(_, t)| t == Ty::Rect) && !matches!(o, Expr::Local(r) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Value(Ty::Rect))) => {
                let (who, _) = self.expr(o)?;
                // (its root clip's place, `getMC()`, is where it is: the object moved)
                if who == "self" && !self.o.fighter && !self.o.stage && matches!(o, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(_, m) | Expr::Name(m) if m == "getMC")) {
                    let (t, _) = self.api("self", if f == "x" { "setX" } else { "setY" }, std::slice::from_ref(v))?;
                    return Ok(vec![format!("{t};")]);
                }
                if who == "self" { return Err(format!("writes its clip's {f}")) }
                let (val, _) = self.expr(v)?;
                // (one it holds may be none: an effect with no art here is never made)
                let plain = who.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '(' || c == ')');
                let set = format!("{who}.set{}({});", f.to_uppercase(), fold(&format!("{} * {}", paren(&val), number(self.o.size))));
                Ok(vec![if plain { format!("if ({who} != null) {set}") } else { set }])
            }
            Place::Member(o, f) if !matches!(o, Expr::This) && matches!(f.as_str(), "scaleX" | "scaleY" | "alpha" | "rotation" | "visible")
                // (a record's, a clip its code made, is its plain field: below)
                && !self.expr(o).is_ok_and(|(_, t)| t == Ty::Rect) && !matches!(o, Expr::Local(r) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Value(Ty::Rect))) => {
                let (who, wt) = self.expr(o)?;
                let (val, _) = if f == "visible" { (self.test(v)?, Ty::Bool) } else { self.expr(v)? };
                let set = match f.as_str() { "scaleX" => "setScaleX", "scaleY" => "setScaleY", "alpha" => "setAlpha", "rotation" => "setRotation", _ => "setVisible" };
                // (one it holds may be none: a part its animation places none of, an effect never made;
                // or, held as a value of no known kind, a record of a clip its code made, whose look is
                // its plain field)
                Ok(vec![if who == "self" { format!("self.{set}({val});") }
                    else if wt == Ty::Any { format!("{{ var held = {who}; if (held != null) {{ if (held.{set} != null) held.{set}({val}); else held.{f} = {val}; }} }}") }
                    else { format!("{{ var held = {who}; if (held != null) held.{set}({val}); }}") }])
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
            // a field of its own class, written through the object from one of its clips
            Place::Member(o, f) if !self.is_itself() && !matches!(o, Expr::This) && self.expr(o).is_ok_and(|(w, _)| w == "self")
                && self.o.itself.is_some_and(|c| self.fields.contains_key(&member_key(&c.name, f)) || self.methods.contains_key(&member_key(&c.name, &format!("set:{f}")))) => {
                // (the value read here, in this class; stored as the fighter's own field)
                self.given = Some(self.expr(v)?);
                let saved = std::mem::replace(&mut self.class, self.o.itself.map(|c| c.name.clone()).unwrap_or_default());
                let r = self.assign(&Place::Member(Expr::This, f.clone()), v);
                self.class = saved;
                r
            }
            // (a field of another object an object writes: through what that object publishes of
            // the fields others' code writes on it)
            Place::Member(o, f) if !self.o.fighter && !matches!(o, Expr::This) && !self.is_clip(o) && !matches!(o, Expr::Name(_) | Expr::Local(_)) && self.expr(o).is_ok_and(|(w, _)| w != "self") => {
                let (who, _) = self.expr(o)?;
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{}({who}, \"self\", {}, {val});", self.owner_field("set")?, quote(f))])
            }
            Place::Member(o, f) if self.o.fighter && !matches!(o, Expr::This) && !self.is_clip(o) && !matches!(o, Expr::Name(_)) => {
                let (who, _) = self.expr(o)?;
                // the power it takes (`copy_power`)
                if who == "self" && f == "KirbyPower" {
                    self.copy_power()?;
                    let (val, _) = self.expr(v)?;
                    return Ok(vec![format!("takePower({val});")]);
                }
                if who == "self" { return Err(format!("writes {f} on itself")) }
                let (val, _) = self.expr(v)?;
                // (through what that object publishes of the fields others' code writes on it; a
                // record of its own, a clip its code made, is written as it is)
                let set = self.owner_field("set")?;
                Ok(vec![format!("if ({who} != null && {who}.isDisposed == null) {who}.{} = {val}; else {set}({who}, \"self\", {}, {val});", ident(f), quote(f))])
            }
            Place::Temp(t) => {
                let (val, ty) = self.expr(v)?;
                self.temp_ty.insert(*t, ty);
                Ok(vec![format!("__t{t} = {val};")])
            }
            Place::Join { block, slot } => {
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("__j{block}_{slot} = {val};")])
            }
            // (one of its own fields, named bare)
            Place::Name(n) if !self.captured.contains(&ident(n)) && self.field(n).is_some() => self.assign(&Place::Member(Expr::This, n.clone()), v),
            // (a static variable of a class of its package's: the one value every object of it shares)
            Place::Name(n) if !self.captured.contains(&ident(n)) && self.field(n).is_none() && self.static_owner(None, n).is_some() => {
                let k = self.static_owner(None, n).unwrap_or_default();
                let declared = self.o.model.classes.get(&k).and_then(|c| c.fields.iter().find(|x| x.name == *n)).filter(|x| x.is_const && !x.is_static).and_then(|x| x.default.as_ref()).and_then(literal);
                if declared.is_some() && declared == literal(v) { return Ok(vec![format!("// (its constant {n} written the value it declares)")]) }
                let (val, _) = self.expr(v)?;
                self.util("staticVar")?;
                Ok(vec![format!("setStaticVar({}, {val});", quote(&format!("{}.{n}", short(&k))))])
            }
            Place::Member(Expr::Name(c), f) if self.static_owner(Some(c), f).is_some() => {
                let k = self.static_owner(Some(c), f).unwrap_or_default();
                let (val, _) = self.expr(v)?;
                self.util("staticVar")?;
                Ok(vec![format!("setStaticVar({}, {val});", quote(&format!("{}.{f}", short(&k))))])
            }
            // (a field of a field of its own no code fills: nothing to write to while it is empty)
            Place::Member(o, n) if matches!(o, Expr::Member(t, f) if matches!(**t, Expr::This) && self.never_filled(f)) || matches!(o, Expr::Name(f) if self.never_filled(f)) || matches!(o, Expr::Local(r) if self.empty_locals.contains(r)) => {
                let (w, _) = self.expr(o)?;
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("if ({w} != null) {w}.{} = {val};", ident(n))])
            }
            // (a field another object of its package keeps, written by a stage's or an object's code:
            // through what that object publishes)
            Place::Member(o, f) if !self.o.fighter && !matches!(o, Expr::This) && !self.is_clip(o) && !self.is_stage_value(o)
                && self.expr(o).is_ok_and(|(w, t)| matches!(t, Ty::Object | Ty::Any) && w != "self" && !w.starts_with("planePart("))
                && self.o.model.classes.values().any(|c| short(&c.super_name) != "PeptideStage" && !self.o.runtime.provides(&c.name) && c.fields.iter().any(|x| &x.name == f && !x.is_static)) => {
                if let Ok(mut a) = asked_fields().lock() { a.insert(f.clone()); }
                let (who, _) = self.expr(o)?;
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{}({who}, \"self\", {}, {val});", self.owner_field("set")?, quote(f))])
            }
            // (a field of the stage's own class, written from another object's code: what the stage publishes)
            Place::Member(o, f) if !self.o.stage && self.is_stage_value(o)
                && self.stage_has_field(f) => {
                let (val, _) = self.expr(v)?;
                self.util("stageField")?;
                Ok(vec![format!("setStageField({}, {val});", quote(f))])
            }
            // (where its clip's parent, the object, is, written in its clip's code: the object moved)
            Place::Member(Expr::Name(p), f) if p == "parent" && matches!(f.as_str(), "x" | "y") && !self.o.stage && self.field("parent").is_none() => {
                let set = if f == "x" { "setX" } else { "setY" };
                let (t, _) = self.api("self", set, std::slice::from_ref(v))?;
                Ok(vec![format!("{t};")])
            }
            // (a property of its own clip, named bare in its clip's code: the clip's)
            Place::Name(n) if !self.captured.contains(&ident(n)) && matches!(n.as_str(), "visible" | "alpha" | "x" | "y" | "scaleX" | "scaleY" | "rotation") => self.assign(&Place::Member(Expr::This, n.clone()), v),
            Place::Name(n) if self.captured.contains(&ident(n)) => {
                let (val, ty) = self.expr(v)?;
                let had = self.captured_ty.get(&ident(n)).copied().unwrap_or(Ty::Any);
                self.captured_ty.insert(ident(n), merge(had, ty));
                Ok(vec![format!("{} = {val};", ident(n))])
            }
            Place::Index(Expr::This, i) if !self.o.clip_parts.contains_key(&self.class) && self.expr(i).is_ok_and(|(_, t)| t == Ty::Str) => {
                let (n, _) = self.expr(i)?;
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{}({n}, {val});", self.own_field_named(true)?)])
            }
            Place::Index(a, i) if self.is_obj_dict(a) => {
                let (a, _) = self.expr(a)?;
                let (i, _) = self.expr(i)?;
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{}.set(Std.string({i}), {val});", paren(&a))])
            }
            Place::Index(a, i) if self.is_dict(a) => {
                let (a, _) = self.expr(a)?;
                let (i, _) = self.expr(i)?;
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{}.set({i}, {val});", paren(&a))])
            }
            Place::Index(a, i) => {
                // (a clip of its planes put in a list field by index, or nothing: a list of them, while every one is)
                if let (Expr::Member(t, fl), true) = (a, self.learning) {
                    if matches!(**t, Expr::This) {
                        if let Ok((w, _)) = self.expr(v) {
                            let part = w.starts_with("planePart(") || matches!(v, Expr::Member(t2, g) if matches!(**t2, Expr::This) && self.part_fields.get(&member_key(&self.class, g)) == Some(&true));
                            let k = member_key(&self.class, fl);
                            if part { self.part_lists.entry(k).or_insert(true); } else if w != "null" { self.part_lists.insert(k, false); }
                        }
                    }
                }
                let (a, _) = self.expr(a)?;
                let (i, ti) = self.expr(i)?;
                if ti == Ty::Str { return Err("writes by a name worked out as it runs".into()) }
                let (val, _) = self.expr(v)?;
                Ok(vec![format!("{}[{i}] = {val};", paren(&a))])
            }
            Place::Local(r) => {
                // (one of the records of a list it keeps, whatever type it is written as)
                let bare = match v { Expr::Coerce(_, e) => &**e, e => e };
                match bare {
                    Expr::Index(l, _) if matches!(&**l, Expr::Member(t, fl) if matches!(**t, Expr::This) && self.rec_lists.contains_key(&member_key(&self.class, fl))) => {
                        let Expr::Member(_, fl) = &**l else { unreachable!() };
                        let k = self.rec_lists[&member_key(&self.class, fl)].clone();
                        self.rec_locals.insert(*r, (k, fl.clone()));
                    }
                    // (or a copy of a variable that holds one)
                    Expr::Local(o) if o != r && self.rec_locals.contains_key(o) => {
                        let held = self.rec_locals[o].clone();
                        self.rec_locals.insert(*r, held);
                    }
                    _ => { self.rec_locals.remove(r); }
                }
                let (val, ty) = self.expr(v)?;
                if val.starts_with("planePart(") { self.part_locals.insert(*r); } else { self.part_locals.remove(r); }
                if val == "stage" { self.stage_locals.insert(*r); } else { self.stage_locals.remove(r); }
                if val == "self" && !matches!(v, Expr::This) { self.self_locals.insert(*r, v.clone()); } else { self.self_locals.remove(r); }
                if matches!(v, Expr::Member(t, f) if matches!(**t, Expr::This) && self.never_filled(f)) { self.empty_locals.insert(*r); } else { self.empty_locals.remove(r); }
                // (a list of records written out, or one copied from a variable that holds one: so is this)
                {
                    let mut lit = v;
                    while let Expr::Coerce(_, x) | Expr::Convert(_, x) = lit { lit = x; }
                    let recs = matches!(lit, Expr::Array(xs) if !xs.is_empty() && xs.iter().all(|x| matches!(x, Expr::Object(_))))
                        || val.starts_with("[{")
                        || matches!(lit, Expr::Local(o) if self.local_elem.get(o) == Some(&Ty::Rect));
                    if recs { self.local_elem.insert(*r, Ty::Rect); }
                }
                // (a record written out in place: a record, whose fields its code reads plainly)
                let ty = { let mut lit = v; while let Expr::Coerce(_, x) | Expr::Convert(_, x) = lit { lit = x; } if matches!(lit, Expr::Object(kv) if !kv.is_empty()) { Ty::Rect } else { ty } };
                let (n, was) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                // (a record variable set to nothing, or to a value of no known kind, stays a record)
                let keep = was == Holds::Value(Ty::Rect) && matches!(ty, Ty::Object | Ty::Any);
                // (one that held its clip, given something else: that value now)
                if (ty != Ty::Any && !keep) || was == Holds::Clip { self.locals.insert(*r, (n.clone(), Holds::Value(ty))); }
                Ok(vec![format!("{n} = {val};")])
            }
            _ => { if std::env::var("PEPTIDE_NATIVE_WHY").is_ok() { eprintln!("[no plain form] write {p:?}"); } Err("writes somewhere other than its fields and variables".into()) }
        }
    }

    /// a jump by label, as the engine's animation that shows it
    fn jump(&mut self, label: &str) -> Result<String, String> {
        // (a label its clip does not have: the source stops there with an error, so nothing happens)
        let class = self.o.model.classes.get(&self.class);
        let has = class.and_then(|c| c.symbol).and_then(|s| self.o.model.sprites.get(&s)).map(|sp| sp.labels.iter().any(|(_, l)| l == label));
        if has == Some(false) { return Ok(format!("// (the source jumps to {label} here, which its clip does not have)")) }
        // (the object's own class: a label none of the clips its animations show has)
        if has.is_none() && self.is_itself() && !self.o.fighter {
            let anywhere = crate::emit::clip_tree(self.o.model, Some(self.o.root)).into_iter()
                .filter_map(|s| self.o.model.sprites.get(&s)).any(|sp| sp.labels.iter().any(|(_, l)| l == label));
            if !anywhere { return Ok(format!("// (the source jumps to {label} here, which its clip does not have)")) }
        }
        let unique = self.o.jumps.get(&self.class).and_then(|j| j.labels.get(label)).cloned()
            // (an object whose class names its clip rather than being bound to it, an enemy: the
            // labels of the clip it draws with)
            .or_else(|| if self.is_itself() && !self.o.fighter {
                self.o.model.class_of(self.o.root).and_then(|c| self.o.jumps.get(&c.name)).and_then(|j| j.labels.get(label)).cloned()
            } else { None });
        // a fighter's clip label that several animations show, or none: the one playing, if it
        // shows it (a ground and an air animation showing one clip whole stay apart, as the engine
        // links them); a part of the clip none shows is one this engine never plays
        let hits: Vec<(String, String, bool)> = if self.o.fighter { self.o.stance_labels.get(&self.class).map(|v| v.iter().filter(|(_, l, _)| l == label).cloned().collect()).unwrap_or_default() } else { Vec::new() };
        let shown_by: BTreeSet<&str> = hits.iter().map(|(a, _, _)| a.as_str()).collect();
        if (unique.is_none() || shown_by.len() > 1) && self.o.fighter {
            let Some((first, _, first_start)) = hits.first().cloned() else { return Ok(format!("// (the source goes on to {label} here, a part of its clip no animation here shows)")) };
            let mut out = vec!["switch (self.getAnimation()) {".to_string()];
            for (a, _, start) in &hits {
                let to = if *start { self.play_start(a)? } else { format!("self.playFrameLabel({})", quote(label)) };
                out.push(format!("\tcase {}: {to};", quote(a)));
            }
            // (from another animation of its clip: into the label; from one that shows another clip,
            // nothing, as the source's clip is no longer shown: its code moved the fighter on)
            let others: Vec<String> = self.clip_anims().into_iter().filter(|a| !shown_by.contains(a.as_str())).map(|a| quote(&a)).collect();
            if !others.is_empty() {
                let to = if first_start { self.play_start(&first)? } else { self.play_label_in(&first, label) };
                out.push(format!("\tcase {}: {to};", others.join(", ")));
            }
            out.push("}".into());
            return Ok(out.join("\n"));
        }
        // (an object's own class sending its clip to a part no animation here shows: nowhere, as a
        // fighter's does)
        if unique.is_none() && self.is_itself() && !self.o.fighter { return Ok(format!("// (the source goes on to {label} here, a part of its clip no animation here shows)")) }
        let (anim, start) = unique.ok_or_else(|| format!("jumps to {label}, which no animation shows"))?;
        let to = if start { self.play_start(&anim)? } else { self.play_label_in(&anim, label) };
        // (a fighter's: only while it shows its clip)
        let anims = if self.o.fighter { self.clip_anims() } else { Vec::new() };
        if anims.len() > 1 || anims.first().is_some_and(|a| *a != anim) {
            let cases: Vec<String> = anims.iter().map(|a| quote(a)).collect();
            return Ok(format!("switch (self.getAnimation()) {{\n\tcase {}: {to};\n}}", cases.join(", ")));
        }
        Ok(format!("{to};"))
    }

    /// the class of a record a value is, where it is one of a list it keeps (`this.foes[i]`, or a
    /// variable holding one)
    /// whether `x` is a clip record whose timeline runs (`clipRun`): the one being written, or one a
    /// field or variable of its holds
    fn clip_rec_value(&self, x: &Expr) -> bool {
        match x {
            Expr::This => self.record.as_ref().is_some_and(|k| self.clip_recs.contains(k)),
            Expr::Member(t, f) if matches!(**t, Expr::This) => self.rec_fields.get(&member_key(&self.class, f)).is_some_and(|k| self.clip_recs.contains(k)),
            _ => self.record_of(x).is_some_and(|k| self.clip_recs.contains(&k)),
        }
    }

    fn record_of(&self, x: &Expr) -> Option<String> {
        match x {
            Expr::Local(r) => self.rec_locals.get(r).map(|(k, _)| k.clone()),
            Expr::Index(l, _) => match &**l { Expr::Member(t, fl) if matches!(**t, Expr::This) => self.rec_lists.get(&member_key(&self.class, fl)).cloned(), _ => None },
            _ => None,
        }
    }

    /// the animations that show the clip of the class being written
    fn clip_anims(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (a, _, _) in self.o.stance_labels.get(&self.class).into_iter().flatten() { if !out.contains(a) { out.push(a.clone()); } }
        let sym = self.o.model.classes.get(&self.class).and_then(|c| c.symbol);
        for (a, s, _, _) in self.o.anim_clips.iter() { if Some(*s) == sym && !out.contains(a) { out.push(a.clone()); } }
        out
    }

    /// a fighter's jump to a label inside an animation: that animation started without its first
    /// frame's own code (the source goes straight to the label's frame), then the label
    fn play_label_in(&mut self, anim: &str, label: &str) -> String {
        if !self.o.fighter { return format!("{{ if (self.getAnimation() != {a}) self.playAnimation({a}); self.playFrameLabel({}); }}", quote(label), a = quote(anim)) }
        self.blocks.insert("mid_jump".into());
        self.mid_targets.insert(anim.to_string());
        format!("playLabelIn({}, {})", quote(anim), quote(label))
    }

    // ---- tests and values ---------------------------------------------------------------------

    /// a value as a test, the source's way: false for null, false, 0, NaN and ""
    fn test(&mut self, e: &Expr) -> Result<String, String> {
        // (a field written out under its name, tested: whether it has been made)
        if let Some((id, _)) = self.flat_of(e) {
            if !id.contains('.') { self.kept.insert(format!("{id}_isMade"), format!("var {id}_isMade = false;")); }
            return Ok(format!("{id}_isMade"));
        }
        // (an engine call's name read as a value and tested, not called: the method itself, which
        // the source's test always finds there)
        if let Expr::Member(_, n) = e {
            if calls().rows.contains_key(n.as_str()) && self.expr(e).is_err() { return Ok("true".into()) }
        }
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
            // (an engine object its code keeps, tested: there and not yet gone. the source's engine
            // lets code go on calling one it destroyed; this one stops the frame on the first call)
            Ty::Object if crate::re!(r"^[A-Za-z_][\w.]*(?:\.get\(\))?$").is_match(&t) && t != "self" =>
                format!("({t} != null && ({t}.isDisposed == null || !{t}.isDisposed()))"),
            Ty::Object | Ty::Rect | Ty::Body | Ty::Controls => format!("{t} != null"),
            Ty::Str => format!("({t} != null && {t} != \"\")"),
            Ty::Any => format!("({t} != null && {t} != false && {t} != 0)"),
        })
    }

    fn expr(&mut self, e: &Expr) -> Result<(String, Ty), String> {
        Ok(match e {
            // an item's class calling the API item's own method it overrides: its handler's `base_` one
            Expr::SuperCall(n, a) if self.o.item && !n.is_empty() => {
                let defined = self.o.runtime.native_files.get("item").is_some_and(|t| crate::dyn_re(&format!(r"\bfunction\s+base_{}\(", regex::escape(n))).is_match(t));
                if !defined { return Err(format!("calls the item's own {n}, which runtime/native/item.hx has no base_{n} for")) }
                let vs: Vec<String> = a.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
                (format!("base_{n}({})", vs.join(", ")), Ty::Any)
            }
            // a class of the API's own it extends asked to do its part first: where the API's class has
            // no such member, that part is nothing (the engine does its own)
            Expr::SuperCall(n, a) if !n.is_empty() && !self.o.item => {
                let base = self.o.model.classes.get(&self.class).map(|c| c.super_name.clone()).unwrap_or_default();
                if !self.o.runtime.provides(&base) { return Err(format!("calls the {n} of {}, a class of its package it extends", short(&base))) }
                // (the API's own call, where it has one: what the source's base does, not this class's
                // override of it)
                if let Ok(r) = self.api("self", n, a) { if !r.0.trim().is_empty() { return Ok(r) } }
                if self.o.runtime.has_member(&base, n) { return Err(format!("calls the API's own {n}, which has a part of its own to do")) }
                // (an attack started by its name, answered: whether it has that animation, played if so)
                if n == "startAttack" && !self.o.fighter && !self.top {
                    if let Some(x) = a.first() {
                        let (l, _) = self.expr(x)?;
                        return Ok((format!("{{ var started = self.hasAnimation({l}); if (started) self.playAnimation({l}); started; }}"), Ty::Bool));
                    }
                }
                if !self.top { return Err(format!("uses what the API's own {n} gives, which it has none of here")) }
                (String::new(), Ty::Any)
            }
            // a held clip's colour transform, read to be changed and set again: a fresh one (the
            // source's starts as its clip's, which shows none here until the code sets one)
            Expr::Member(x, n) if n == "colorTransform" && matches!(&**x, Expr::Member(_, t) if t == "transform") => (calls_ct(), Ty::Rect),
            // a clip one of its effects places that its code turns: its own vfx, riding the effect
            Expr::Member(o, n) if self.is_inner(n) && !matches!(**o, Expr::This) && !self.is_clip(o) && self.field(n).is_none() => {
                let (v, _) = self.expr(o)?;
                self.inner_clips()?;
                (format!("innerClip({v}, {})", quote(n)), Ty::Object)
            }
            // one of its parts, as a value: its vfx (placed first, so it is there)
            Expr::Member(_, _) if self.part_of(e).is_some() => {
                let name = self.part_of(e).unwrap_or_default();
                (format!("partNow({})", part_id(&name)), Ty::Object)
            }
            // a field of a flattened field's: its getter
            Expr::Member(o, p) if self.flat_of(o).is_some() => {
                let (id, k) = self.flat_of(o).unwrap_or_default();
                return self.flat_call(&id, &k, &format!("get_{p}"), &[]);
            }
            // (one tested as a value: whether it has been made)
            Expr::Member(_, f) if self.flat_of(e).is_some() && self.testing => {
                let (id, _) = self.flat_of(e).unwrap_or_default();
                if !id.contains('.') { self.kept.insert(format!("{id}_isMade"), format!("var {id}_isMade = false;")); }
                (format!("{id}_isMade"), Ty::Bool)
            }
            Expr::Member(_, f) if self.flat_of(e).is_some() => {
                let (_, k) = self.flat_of(e).unwrap_or_default();
                return Err(format!("uses its {f}, a {k} written out under its name, as a value"));
            }
            // an effect it makes drawn behind it: the effect made behind it (the effect is the value)
            Expr::Call(f, a) if matches!(&**f, Expr::Member(_, m) if m == "pushEffectBehind")
                && matches!(a.first(), Some(Expr::Call(g, _)) if matches!(&**g, Expr::Member(_, m) if matches!(m.as_str(), "attachEffect" | "createVfx"))) => {
                let Some(Expr::Call(g, eargs)) = a.first() else { unreachable!() };
                let mut eargs = eargs.clone();
                let behind = (Expr::Str("behind".into()), Expr::Bool(true));
                match eargs.get_mut(1) {
                    Some(Expr::Object(kv)) => kv.push(behind),
                    None => eargs.push(Expr::Object(vec![behind])),
                    Some(Expr::Null | Expr::Undefined) => eargs[1] = Expr::Object(vec![behind]),
                    _ => return Err("draws behind it an effect made with options worked out as it runs".into()),
                }
                return self.expr(&Expr::Call(g.clone(), eargs));
            }
            Expr::Null | Expr::Undefined => ("null".into(), Ty::Object),
            // the labels of its own root clip (`getMC().currentLabels`): the ones its clip declares
            Expr::Member(mc, n) if n == "currentLabels" && matches!(&**mc, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(o, m) if m == "getMC" && self.expr(o).is_ok_and(|(w, _)| w == "self"))) => {
                let labels = self.o.model.sprites.get(&self.o.root).map(|sp| sp.labels.iter().map(|(f, l)| format!("{{ name: {}, frame: {} }}", quote(l), f + 1)).collect::<Vec<_>>()).unwrap_or_default();
                (format!("[{}]", labels.join(", ")), Ty::Object)
            }
            // a field another object's own class keeps, read by the stage's code: what that object publishes
            Expr::Member(o, f) if self.o.stage && !matches!(**o, Expr::This) && self.expr(o).is_ok_and(|(w, t)| matches!(t, Ty::Object | Ty::Any) && !w.starts_with("planePart(") && !(f == "mc" && w.starts_with("cameraPlanes()[")))
                && !self.o.model.classes.values().any(|c| c.fields.iter().any(|x| &x.name == f) && short(&c.super_name) == "PeptideStage")
                && (lent_fields().lock().is_ok_and(|l| l.contains(f)) || self.o.model.classes.values().any(|c| short(&c.super_name) != "PeptideStage" && !self.o.runtime.provides(&c.name) && c.fields.iter().any(|x| &x.name == f && !x.is_static))) => {
                if let Ok(mut a) = asked_fields().lock() { a.insert(f.clone()); }
                let (who, _) = self.expr(o)?;
                return Ok((format!("{}({who}, \"self\", {})", self.owner_field("get")?, quote(f)), Ty::Any));
            }
            // a child of its own clip whose box its class reads (`getMC().hitBox`): there, as its clip carries
            // it; its box in its parent's space (`getRect(parent)`) where it stands, as measured
            Expr::Call(g, a) if a.len() == 1 && matches!(&**g, Expr::Member(c, m) if m == "getRect" && self.own_child_box(c).is_some()) => {
                let Expr::Member(c, _) = &**g else { unreachable!() };
                let (x, y, w, h) = self.own_child_box(c).ok_or("no box")?;
                (format!("new Rectangle(self.getX() / {s} + {}, self.getY() / {s} + {}, {}, {})", number(x), number(y), number(w), number(h), s = number(self.o.size)), Ty::Rect)
            }
            // its own clip's size, as measured (kept under no child's name)
            Expr::Member(c, n) if matches!(n.as_str(), "width" | "height") && !self.o.fighter && self.own_clip_call(c) && self.own_box().is_some() => {
                let (_, _, w, h) = self.own_box().ok_or("no box")?;
                (number(if n == "width" { w } else { h }), Ty::Num)
            }
            // (its size, as measured)
            Expr::Member(c, n) if matches!(n.as_str(), "width" | "height") && self.own_child_box(c).is_some() => {
                let (_, _, w, h) = self.own_child_box(c).ok_or("no box")?;
                (number(if n == "width" { w } else { h }), Ty::Num)
            }
            Expr::Member(_, _) if self.own_child_box(e).is_some() => ("true".into(), Ty::Bool),
            // a field of the stage's own class, read from another object's code: what the stage publishes
            Expr::Member(o, f) if !self.o.stage && self.is_stage_value(o)
                && self.stage_has_field(f) => {
                self.util("stageField")?;
                (format!("stageField({})", quote(f)), Ty::Any)
            }
            // its own clip's look, read: its own (the clip is drawn as the object)
            Expr::Member(mc, n) if !self.o.fighter && matches!(n.as_str(), "scaleX" | "scaleY" | "alpha" | "rotation" | "visible")
                && matches!(&**mc, Expr::Call(g, a) if a.is_empty() && (matches!(&**g, Expr::Name(m) if matches!(m.as_str(), "getMC" | "stanceClip" | "getStanceMC") && self.method(m).is_none())
                    || matches!(&**g, Expr::Member(o, m) if matches!(m.as_str(), "getMC" | "stanceClip" | "getStanceMC") && matches!(**o, Expr::This)))) => {
                let get = match n.as_str() { "scaleX" => "getScaleX", "scaleY" => "getScaleY", "alpha" => "getAlpha", "rotation" => "getRotation", _ => "getVisible" };
                (format!("self.{get}()"), if n == "visible" { Ty::Bool } else { Ty::Num })
            }
            // the name the tree placed its own clip under
            Expr::Member(mc, n) if n == "name" && self.o.instance.is_some()
                && matches!(&**mc, Expr::Call(g, a) if a.is_empty() && (matches!(&**g, Expr::Member(o, m) if m == "getMC" && matches!(**o, Expr::This)) || matches!(&**g, Expr::Name(m) if m == "getMC" && self.method(m).is_none()))) => {
                (quote(self.o.instance.as_deref().unwrap_or_default()), Ty::Str)
            }
            // (the language's `undefined` named outright: nothing)
            Expr::Name(n) if n == "undefined" && self.field(n).is_none() => ("null".into(), Ty::Object),
            Expr::Bool(b) => (b.to_string(), Ty::Bool),
            Expr::Num(n) => (number(*n), Ty::Num),
            Expr::Int(n) => (n.to_string(), Ty::Num),
            Expr::Str(s) => (quote(s), Ty::Str),
            Expr::Local(r) if self.part_locals.contains(r) => {
                let (n, _) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                self.util("planePart")?;
                (format!("planePart({n})"), Ty::Object)
            }
            Expr::Local(r) => {
                let (n, h) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                if h == Holds::Clip { if std::env::var("PEPTIDE_NATIVE_WHY").is_ok() { eprintln!("[no plain form] clip local {n} in {}", self.current_fn); } return Err("uses its clip as a value".into()) }
                (n, match h { Holds::Itself | Holds::Clip => Ty::Object, Holds::Value(t) => t })
            }
            // (a record's own code: the record)
            // (where a clip record's timeline is)
            Expr::Member(x, n) if matches!(n.as_str(), "currentFrame" | "currentLabel" | "currentFrameLabel" | "totalFrames") && self.clip_rec_value(x) => {
                let (r, _) = self.expr(x)?;
                let n = if n == "currentFrameLabel" { "currentLabel" } else { n.as_str() };
                (format!("{}.{n}", paren(&r)), if n == "currentLabel" { Ty::Str } else { Ty::Num })
            }
            Expr::Name(n) if matches!(n.as_str(), "currentFrame" | "currentLabel" | "totalFrames") && self.clip_rec_value(&Expr::This) && self.field(n).is_none() => (format!("me.{n}"), if n == "currentLabel" { Ty::Str } else { Ty::Num }),
            // (one of a record's own methods as a value: that method, handed the record)
            Expr::Member(t, g) if matches!(**t, Expr::This) && self.record.is_some() && self.field(g).is_none() && self.method(g).is_some() => {
                let id = self.method(g).unwrap_or_default();
                (format!("function(?a, ?b, ?c) {{ return {id}(me, a, b, c); }}"), Ty::Object)
            }
            // (where a clip of the object's own (not the object's class) is placed, named bare: the one
            // place every placement in the tree puts it, in the source's units)
            // (the clip that is an object's whole art: its place is the object's)
            Expr::Name(n) if matches!(n.as_str(), "x" | "y") && self.field(n).is_none() && self.is_root_clip() && self.record.is_none() => {
                (format!("(self.get{}() / {})", n.to_uppercase(), number(self.o.size)), Ty::Num)
            }
            Expr::Name(n) if matches!(n.as_str(), "x" | "y") && self.field(n).is_none() && !self.is_itself() && self.record.is_none() && self.one_placement().is_some() => {
                let (x, y) = self.one_placement().unwrap_or_default();
                (number(if n == "x" { x } else { y }), Ty::Num)
            }
            Expr::This if self.record.is_some() => ("me".into(), Ty::Object),
            // a field of a record of a list it keeps, read from outside the record: on the record
            Expr::Member(x, n) if self.record_of(x).is_some_and(|k| self.fields.contains_key(&member_key(&k, n))) => {
                let k = self.record_of(x).unwrap_or_default();
                let ty = match self.fields.get(&member_key(&k, n)) { Some((_, Holds::Value(t))) => *t, _ => Ty::Any };
                let (v, _) = self.expr(x)?;
                (format!("{}.{}", paren(&v), ident(n)), ty)
            }
            // (a clip of a stage's planes: the named clip it is placed as)
            Expr::This if self.o.clip_parts.contains_key(&self.class) => { self.util("planePart")?; (format!("planePart({})", quote(&self.o.clip_parts[&self.class])), Ty::Object) }
            Expr::This if self.is_itself() => ("self".into(), Ty::Object),
            // (the clip that is an object's whole art, as a value: the object)
            Expr::This if self.is_root_clip() => ("self".into(), Ty::Object),
            // (the object on the scope chain that has a field or method of its own: the object itself)
            Expr::ScopeOf(n) if self.is_itself() && (self.field(n).is_some() || self.method(n).is_some()) => ("self".into(), Ty::Object),
            Expr::Member(t, n) if n == "currentFrame" && self.counts_frames() && matches!(**t, Expr::This) && self.field(n).is_none() => (self.clip_frame_helper()?, Ty::Num),
            Expr::Member(t, n) if matches!(n.as_str(), "currentLabel" | "currentFrameLabel") && matches!(**t, Expr::This) && self.field(n).is_none() => {
                self.blocks.insert("clip_label".into());
                ("clipLabel()".into(), Ty::Str)
            }
            Expr::Name(n) if matches!(n.as_str(), "currentLabel" | "currentFrameLabel") && self.field(n).is_none() => {
                self.blocks.insert("clip_label".into());
                ("clipLabel()".into(), Ty::Str)
            }
            // where its clip's parent (the fighter) is, in the source's units
            Expr::Member(t, n) if matches!(&**t, Expr::Name(p) if p == "parent") && matches!(n.as_str(), "x" | "y") =>
                (format!("self.get{}() / {}", n.to_uppercase(), number(self.o.size)), Ty::Num),
            Expr::Member(t, f) if matches!(**t, Expr::This) && self.o.clip_parts.contains_key(&self.class) && self.field(f).is_none() && self.method(f).is_none() => {
                self.util("planePart")?;
                let who = format!("planePart({})", quote(&self.o.clip_parts[&self.class]));
                return Ok(match f.as_str() {
                    "visible" => (format!("({who} == null || {who}.getVisible())"), Ty::Bool),
                    "alpha" => (format!("({who} == null ? 1 : {who}.getAlpha())"), Ty::Num),
                    "currentFrame" => (format!("({who} == null ? 1 : {who}.getFrame())"), Ty::Num),
                    "currentLabel" | "currentFrameLabel" => (format!("({who} == null ? null : {who}.getLabel())"), Ty::Str),
                    _ => (format!("planePart({})", quote(f)), Ty::Object),
                });
            }
            // a child its stage's timeline places under the field's name, which its code never fills:
            // that clip of the planes (Flash fills the field as the clip is made)
            Expr::Member(t, f) if matches!(**t, Expr::This) && self.timeline_child(f) => {
                self.util("planePart")?;
                return Ok((format!("planePart({})", quote(f)), Ty::Object));
            }
            Expr::Member(t, f) if matches!(**t, Expr::This) => return self.own(f),
            Expr::Member(t, f) if matches!(&**t, Expr::Local(r) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Clip)) => return self.own(f),
            // a static field of one of its classes, read by the class's name: its field
            Expr::Member(o, f) if matches!(&**o, Expr::Name(n) if self.static_class(n, f).is_some()) => {
                let Expr::Name(n) = &**o else { unreachable!() };
                let owner = self.static_class(n, f).unwrap_or_default();
                let saved = std::mem::replace(&mut self.class, owner);
                let r = self.own(f);
                self.class = saved;
                return r;
            }
            // a field of its owner's (a read of one that is no call)
            Expr::Member(o, f) if !matches!(**o, Expr::This | Expr::Call(..)) && { let w = self.expr(o).map(|x| x.0).unwrap_or_default(); self.owners.contains(&w) } => {
                let (who, _) = self.expr(o)?;
                if self.o.owner_native { return Ok((format!("{}({who}, \"self\", {})", self.owner_field("get")?, quote(f)), Ty::Any)) }
                return Ok((if (self.o.owner_stores)(f) { format!("{who}.exports.__source.__c_{}.get()", ident(f)) } else { format!("{who}.exports.__source.{}", ident(f)) }, Ty::Any));
            }
            // a field of an element of a list of API records: its class's getter, handed it
            Expr::Member(x, prop) if matches!(&**x, Expr::Index(l, _) if matches!(&**l, Expr::Member(t, f) if matches!(**t, Expr::This) && self.rt_list(f).is_some())) => {
                let Expr::Index(l, i) = &**x else { unreachable!() };
                let Expr::Member(_, f) = &**l else { unreachable!() };
                let k = self.rt_list(f).unwrap_or_default();
                self.rt_record(&k)?;
                let (list, _) = self.expr(l)?;
                let (ix, _) = self.expr(i)?;
                return Ok((format!("{k}_get_{prop}({list}[{ix}])"), if prop == "completed" { Ty::Bool } else { Ty::Rect }));
            }
            // the size of a picture a field declares (`BitmapData`): its record's
            Expr::Member(o, f) if self.o.stage && matches!(f.as_str(), "width" | "height") && matches!(&**o, Expr::Member(t, g) if matches!(**t, Expr::This)
                && self.o.model.classes.get(&self.class).is_some_and(|c| c.fields.iter().any(|x| &x.name == g && x.type_name.as_deref().is_some_and(|t| drawn_type(self.o.model, t))))) => {
                let (v, _) = self.expr(o)?;
                (format!("{}.{f}", paren(&v)), Ty::Num)
            }
            // a field of a named clip of the planes: what it shows, its frame, its label, or a clip of
            // its by name
            Expr::Member(o, f) if self.expr(o).is_ok_and(|(w, _)| w.starts_with("planePart(")) || self.part_field(o) => {
                let (p, _) = self.expr(o)?;
                return Ok(match f.as_str() {
                    "visible" => (format!("({p} == null || {p}.getVisible())"), Ty::Bool),
                    "alpha" => (format!("({p} == null ? 1 : {p}.getAlpha())"), Ty::Num),
                    "currentFrame" => (format!("({p} == null ? 1 : {p}.getFrame())"), Ty::Num),
                    "currentLabel" | "currentFrameLabel" => (format!("({p} == null ? null : {p}.getLabel())"), Ty::Str),
                    "fallthrough" => (format!("({p} != null && {p}.getFallthrough())"), Ty::Bool),
                    // (where a region the tree placed under its name stands, and its size, in the source's units)
                    "x" => (format!("({p} == null ? 0 : {p}.getX())"), Ty::Num),
                    "y" => (format!("({p} == null ? 0 : {p}.getY())"), Ty::Num),
                    "width" => (format!("({p} == null ? 0 : {p}.getWidth())"), Ty::Num),
                    "height" => (format!("({p} == null ? 0 : {p}.getHeight())"), Ty::Num),
                    "scaleX" => (format!("({p} == null ? 1 : {p}.getScaleX())"), Ty::Num),
                    "scaleY" => (format!("({p} == null ? 1 : {p}.getScaleY())"), Ty::Num),
                    "rotation" => (format!("({p} == null ? 0 : {p}.getRotation())"), Ty::Num),
                    "numChildren" => (format!("({p} == null ? 0 : {p}.numKids())"), Ty::Num),
                    "currentLabels" => (format!("({p} == null ? [] : {p}.labelList())"), Ty::Rect),
                    "totalFrames" => (format!("({p} == null ? 1 : {p}.getTotalFrames())"), Ty::Num),
                    // (a field its code keeps on the clip)
                    _ if self.part_written.contains(f) => (format!("({p} == null ? null : {p}.{})", ident(f)), Ty::Any),
                    _ => { self.util("planePart")?; (format!("planePart({})", quote(f)), Ty::Object) }
                });
            }
            // one of a stage's camera backgrounds, as the clip of its planes it is (by its name)
            Expr::Member(o, f) if self.o.stage && f == "mc" && self.expr(o).is_ok_and(|(w, _)| (w.starts_with("cameraPlanes()[") || w.starts_with("(cameraPlanes())[")) && w.ends_with(']')) => {
                let (v, _) = self.expr(o)?;
                let i = v.trim_start_matches("(cameraPlanes())[").trim_start_matches("cameraPlanes()[").trim_end_matches(']').to_string();
                self.util("planePart")?;
                self.util("cameraBgName")?;
                return Ok((format!("planePart(cameraBgName({i}))"), Ty::Object));
            }
            // (a camera background held in a variable: its part, under `mc`)
            Expr::Member(o, f) if self.o.stage && f == "mc" && !matches!(**o, Expr::This) && !self.expr(o).is_ok_and(|(w, _)| w.starts_with("planePart(")) => {
                let (v, t) = self.expr(o)?;
                // (a record's: what it holds, a record where it holds a clip record)
                return Ok((format!("{v}.mc"), if t == Ty::Rect { Ty::Rect } else { Ty::Object }));
            }
            // a field of the object itself, read through a variable that holds it
            Expr::Member(o, n) if matches!(&**o, Expr::Local(r) if self.self_locals.contains_key(r)) => {
                let Expr::Local(r) = &**o else { unreachable!() };
                let src = self.self_locals[r].clone();
                return self.expr(&Expr::Member(Box::new(src), n.clone()));
            }
            // a constant of one of its package's classes, read through an object of it: its value
            Expr::Member(o, f) if !matches!(**o, Expr::This) && self.const_of(f).is_some() => (self.const_of(f).unwrap_or_default(), Ty::Any),
            // (a field every class of the package that has it keeps the object it runs as in, its
            // `self`: on another object, that object)
            Expr::Member(o, f) if !matches!(**o, Expr::This) && self.holds_itself_everywhere(f) => self.expr(o)?,
            // a field of another object's own class (one its package defines): through what it publishes
            Expr::Member(o, f) if !matches!(**o, Expr::This) && !matches!(f.as_str(), "x" | "y" | "length")
                // (from an object a call answers, a field its class's code writes and declares no slot of too:
                // a platform's timer, `getPlatform("house").waitTime`)
                && (!matches!(**o, Expr::Call(..)) || self.o.model.classes.values().any(|c| short(&c.super_name) != "PeptideStage" && (c.symbol.is_some() || self.o.runtime.provides(&c.super_name)) && writes_own(c, f)))
                && self.a_value(o) && !matches!(&**o, Expr::Local(r) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Clip))
                && self.o.model.classes.values().any(|c| (c.symbol.is_some() || self.o.runtime.provides(&c.super_name)) && (c.fields.iter().any(|x| &x.name == f && !x.is_static) || writes_own(c, f)))
                && self.expr(o).is_ok_and(|(w, t)| w != "self" && !self.owners.contains(&w) && matches!(t, Ty::Any | Ty::Object | Ty::Body)) => {
                let (who, _) = self.expr(o)?;
                return Ok((format!("{}({who}, \"self\", {})", self.owner_field("get")?, quote(f)), Ty::Any));
            }
            // the label an item's clip is on: the animation it plays (`item.hx` `stanceLabel`)
            Expr::Member(mc, n) if self.o.item && matches!(n.as_str(), "currentLabel" | "currentFrameLabel")
                && matches!(&**mc, Expr::Call(g, a) if a.is_empty() && (matches!(&**g, Expr::Name(s) if matches!(s.as_str(), "stanceClip" | "getStanceMC" | "getMC"))
                    || matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "stanceClip" | "getStanceMC" | "getMC") && self.expr(o).is_ok_and(|(w, _)| w == "self")))) => {
                return Ok(("stanceLabel()".into(), Ty::Str));
            }
            // where the clip its animation shows is, as its own clip reads it
            // how many frames the clip its animation shows has (`clipTotal`)
            Expr::Member(mc, n) if n == "totalFrames" && self.counts_frames()
                && matches!(&**mc, Expr::Call(g, a) if a.is_empty() && (matches!(&**g, Expr::Name(s) if matches!(s.as_str(), "stanceClip" | "getStanceMC"))
                    || matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "stanceClip" | "getStanceMC") && self.expr(o).is_ok_and(|(w, _)| w == "self")))) => {
                if !self.helpers.contains_key("clipTotal") {
                    let mut cases = Vec::new();
                    for anims in self.o.clip_frames.values() {
                        let total = anims.iter().map(|(_, from, n)| from + n - 1).max().unwrap_or(1);
                        let names: Vec<String> = anims.iter().map(|(a, _, _)| quote(a)).collect();
                        if !names.is_empty() { cases.push(format!("case {}: return {total};", names.join(", "))); }
                    }
                    self.helpers.insert("clipTotal".into(), helper(self.o.runtime, "helpers", "clip_total", "clipTotal", &[], &cases)?);
                }
                return Ok(("clipTotal()".into(), Ty::Num));
            }
            Expr::Member(mc, n) if (self.counts_frames() || n != "currentFrame") && matches!(n.as_str(), "currentLabel" | "currentFrameLabel" | "currentFrame")
                && matches!(&**mc, Expr::Call(g, a) if a.is_empty() && (matches!(&**g, Expr::Name(s) if matches!(s.as_str(), "stanceClip" | "getStanceMC") && !self.o.item)
                    || matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "stanceClip" | "getStanceMC") && self.expr(o).is_ok_and(|(w, _)| w == "self")))) => {
                if n == "currentFrame" { return Ok((self.clip_frame_helper()?, Ty::Num)) }
                self.blocks.insert("clip_label".into());
                return Ok(("clipLabel()".into(), Ty::Str));
            }
            // a field of its own class, read through the object from one of its clips
            Expr::Member(o, f) if !self.is_itself() && !matches!(**o, Expr::This) && self.expr(o).is_ok_and(|(w, _)| w == "self")
                && self.o.itself.is_some_and(|c| self.fields.contains_key(&member_key(&c.name, f)) || self.methods.contains_key(&member_key(&c.name, &format!("get:{f}")))) => {
                let saved = std::mem::replace(&mut self.class, self.o.itself.map(|c| c.name.clone()).unwrap_or_default());
                let r = self.own(f);
                self.class = saved;
                return r;
            }
            // a field of another of the classes it runs (the clip its animation shows, say), read
            // through the object from one of its clips
            Expr::Member(o, f) if !self.is_itself() && !matches!(**o, Expr::This) && self.expr(o).is_ok_and(|(w, _)| w == "self")
                && self.o.classes.iter().any(|c| c.name != self.class && self.fields.contains_key(&member_key(&c.name, f))) => {
                let owner = self.o.classes.iter().find(|c| c.name != self.class && self.fields.contains_key(&member_key(&c.name, f))).map(|c| c.name.clone()).unwrap_or_default();
                let saved = std::mem::replace(&mut self.class, owner);
                let r = self.own(f);
                self.class = saved;
                return r;
            }
            // a stat of the box an event's hit was made with: the engine's, in the source's units
            Expr::Member(b, k) if matches!(&**b, Expr::Member(d, n) if n == "attackBoxData" && matches!(&**d, Expr::Member(a, n) if n == "data" && matches!(**a, Expr::Local(_)))) => {
                let Expr::Member(d, _) = &**b else { unreachable!() };
                let Expr::Member(a, _) = &**d else { unreachable!() };
                let Expr::Local(r) = &**a else { unreachable!() };
                let (arg, _) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                // (whether it hit the way its owner faced: its owner's facing)
                if k == "isForward" { return Ok((format!("({arg}.data.hitboxStats.owner == null || {arg}.data.hitboxStats.owner.isFacingRight())"), Ty::Bool)) }
                // (whether it hits with an effect: whether it makes the one it hits flinch, as the box is written)
                if k == "hasEffect" { return Ok((format!("({arg}.data.hitboxStats.flinch != false)"), Ty::Bool)) }
                // (one this engine keeps none of: nothing, as the box has none here)
                // (one this engine has no stat for: what the box declares, from its attacker's
                // animation's metadata, `hitBoxMeta`)
                if crate::mappings::character_hitbox_stats().no_counterpart.iter().any(|x| x == k) {
                    self.util("hitBoxMeta")?;
                    return Ok((format!("hitBoxMeta({arg}).{k}"), Ty::Any));
                }
                let (f, scale) = hitbox_field(k)?;
                let v = format!("{arg}.data.hitboxStats.{f}");
                return Ok((match scale { Some(sc) => format!("{v} / {}", number(sc)), None => v }, Ty::Num));
            }
            // a field of the record an item's handler hands its frame interrupt
            Expr::Member(a, field) if self.o.item && matches!(**a, Expr::Local(_)) && super::events::table().interrupt.contains_key(field) => {
                let Expr::Local(r) = &**a else { unreachable!() };
                let (arg, _) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                return Ok((format!("{arg}.{field}"), Ty::named(&super::events::table().interrupt[field])));
            }
            // a field of the event a listener is handed
            Expr::Member(d, field) if matches!(&**d, Expr::Member(a, n) if n == "data" && matches!(**a, Expr::Local(_))) => {
                let Expr::Member(a, _) = &**d else { unreachable!() };
                let Expr::Local(r) = &**a else { unreachable!() };
                let (arg, _) = self.locals.get(r).cloned().ok_or("an unknown variable")?;
                // (the stats of the box that hit it: the engine's box, as the source's record)
                if matches!(field.as_str(), "attackBoxData" | "callerAttackBoxData") { return Ok((format!("{}({arg}.data.hitboxStats)", self.box_record()?), Ty::Rect)) }
                // (the caller, in a handler its code listens on another object with: that object)
                if field == "caller" && self.foreign_handlers.contains(&self.current_fn) { return Ok((format!("{arg}.data.self"), Ty::Object)) }
                // (the damage it tells of: a hit's own, or how far an update moved it, `damageStep`)
                if field == "damage" { self.util("damageStep")?; return Ok((format!("damageStep({arg})"), Ty::Num)) }
                return Ok(match record(&arg, field)? {
                    Some(v) => v,
                    None => {
                        self.gaps.insert(format!("reads the {field} of an event, which no event of the source's carries"));
                        // (nothing, as a record: what its code reads of it, behind its own test of it)
                        ("null".into(), Ty::Rect)
                    }
                });
            }
            Expr::Name(n) if self.timeline_child(n) => {
                self.util("planePart")?;
                return Ok((format!("planePart({})", quote(n)), Ty::Object));
            }
            // (a variable of the function's own, which a closure of it shares, comes first: it shadows
            // a field of the same name, as the source's scope chain reads it)
            Expr::Name(n) if !self.captured.contains(&ident(n)) && (self.field(n).is_some() || self.method(n).is_some() || self.method(&format!("get:{n}")).is_some()) => return self.own(n),
            // a weather particle's clip class as a value: the effect it is drawn as, with its size
            Expr::Name(n) if self.o.particles.contains_key(short(n)) => {
                let (id, w, h) = self.o.particles[short(n)].clone();
                (format!("{{ id: {}, w: {}, h: {} }}", quote(&id), number(w), number(h)), Ty::Rect)
            }
            // a variable its function shares with a closure it makes
            Expr::Temp(t) => (format!("__t{t}"), self.temp_ty.get(t).copied().unwrap_or(Ty::Any)),
            Expr::Join { block, slot } => (format!("__j{block}_{slot}"), Ty::Any),
            Expr::Name(n) if self.captured.contains(&ident(n)) => (ident(n), self.captured_ty.get(&ident(n)).copied().unwrap_or(Ty::Any)),
            // the clip it shows sits in the object: its parent is the object
            Expr::Name(n) if n == "parent" => ("self".into(), Ty::Body),
            // (the clip at the top of its tree: the object's own)
            Expr::Name(n) if n == "root" => ("self".into(), Ty::Body),
            // a constant one of the package's classes states: its value
            Expr::Member(c, n) if matches!(&**c, Expr::Name(cn) if self.o.model.classes.values().any(|k| short(&k.name) == short(cn) && k.fields.iter().any(|f| f.is_static && &f.name == n && f.default.as_ref().and_then(literal).is_some()))) => {
                let Expr::Name(cn) = &**c else { unreachable!() };
                let k = self.o.model.classes.values().find(|k| short(&k.name) == short(cn)).ok_or("no class")?;
                let f = k.fields.iter().find(|f| f.is_static && &f.name == n).ok_or("no field")?;
                let v = f.default.as_ref().and_then(literal).unwrap_or_default();
                let ty = if v.starts_with('"') { Ty::Str } else if v == "true" || v == "false" { Ty::Bool } else { Ty::Num };
                (v, ty)
            }
            // a constant its own class states, read by its bare name: its value
            // a static variable of a class its code changes as it runs: the value they all share
            Expr::Name(n) if self.field(n).is_none() && self.static_owner(None, n).is_some_and(|k| self.static_written(&k, n)) => {
                let k = self.static_owner(None, n).unwrap_or_default();
                let d = self.o.model.classes.get(&k).and_then(|c| c.fields.iter().find(|x| x.name == *n)).and_then(|x| x.default.as_ref()).and_then(literal).unwrap_or_else(|| "null".into());
                self.util("staticVar")?;
                return Ok((format!("staticVar({}, {d})", quote(&format!("{}.{n}", short(&k)))), Ty::Any));
            }
            Expr::Member(c, f) if matches!(&**c, Expr::Name(cn) if self.static_owner(Some(cn), f).is_some_and(|k| self.static_written(&k, f))) => {
                let Expr::Name(cn) = &**c else { unreachable!() };
                let k = self.static_owner(Some(cn), f).unwrap_or_default();
                let d = self.o.model.classes.get(&k).and_then(|c| c.fields.iter().find(|x| x.name == *f)).and_then(|x| x.default.as_ref()).and_then(literal).unwrap_or_else(|| "null".into());
                self.util("staticVar")?;
                return Ok((format!("staticVar({}, {d})", quote(&format!("{}.{f}", short(&k)))), Ty::Any));
            }
            Expr::Name(n) if self.field(n).is_none() && self.o.model.classes.get(&self.class).is_some_and(|k| static_value(k, n).is_some()) => {
                let k = self.o.model.classes.get(&self.class).ok_or("no class")?;
                let d = static_value(k, n).ok_or("no value")?;
                match literal(&d) {
                    Some(v) => { let ty = if v.starts_with('"') { Ty::Str } else if v == "true" || v == "false" { Ty::Bool } else { Ty::Num }; (v, ty) }
                    // (a list or record it states: written where it is read)
                    None => self.expr(&d)?,
                }
            }
            // the frame a clip of the planes is on, read by its own code: its part's
            Expr::Name(n) if n == "currentFrame" && self.field(n).is_none() && self.o.clip_parts.contains_key(&self.class) => {
                self.util("planePart")?;
                let p = quote(&self.o.clip_parts[&self.class]);
                (format!("(planePart({p}) == null ? 1 : planePart({p}).getFrame())"), Ty::Num)
            }
            Expr::Member(t, n) if n == "currentFrame" && matches!(**t, Expr::This) && self.field(n).is_none() && self.o.clip_parts.contains_key(&self.class) => {
                self.util("planePart")?;
                let p = quote(&self.o.clip_parts[&self.class]);
                (format!("(planePart({p}) == null ? 1 : planePart({p}).getFrame())"), Ty::Num)
            }
            // how many frames the clip the code is on has: the clip's own count
            Expr::Name(n) if n == "totalFrames" && self.field(n).is_none() => (self.total_frames()?, Ty::Num),
            Expr::Member(t, n) if n == "totalFrames" && matches!(**t, Expr::This) && self.field(n).is_none() => (self.total_frames()?, Ty::Num),
            // the frame its clip is on
            Expr::Name(n) if n == "currentFrame" && self.counts_frames() => (self.clip_frame_helper()?, Ty::Num),
            Expr::Member(t, n) if n == "currentFrame" && self.counts_frames() && (matches!(**t, Expr::This) || self.is_clip(t)) => (self.clip_frame_helper()?, Ty::Num),
            // the clip an effect it keeps sits in: the effect, while it is there
            Expr::Member(o, n) if n == "parent" && !matches!(**o, Expr::This) && !self.is_clip(o) => {
                // (an object's clip's: the match's drawing order, `drawOrderParent`)
                if !self.o.stage && matches!(&**o, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(_, m) | Expr::Name(m) if m == "getMC")) {
                    self.util("drawOrderParent")?;
                    return Ok(("drawOrderParent()".into(), Ty::Rect));
                }
                let (v, _) = self.expr(o)?;
                if v == "self" { return Err("reads its clip's parent".into()) }
                (format!("({v} == null || {v}.isDisposed() ? null : {v})"), Ty::Object)
            }
            // a field of a record its code got back (its controls: `c.LEFT`, held or not)
            Expr::Member(o, n) if n.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()) && self.a_value(o) => {
                let (v, t) = self.expr(o)?;
                (format!("{}.{n}", paren(&v)), if t == Ty::Controls { Ty::Bool } else { Ty::Any })
            }
            Expr::Member(o, n) if self.a_value(o) && matches!(n.as_str(), "scaleX" | "scaleY" | "alpha" | "rotation") && !self.is_clip(o)
                // (a record's, a clip its code made, is its plain field, as its writes are)
                && !self.expr(o).is_ok_and(|(_, t)| t == Ty::Rect) && !matches!(&**o, Expr::Local(r) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Value(Ty::Rect))) => {
                let (v, _) = self.expr(o)?;
                let get = match n.as_str() { "scaleX" => "getScaleX", "scaleY" => "getScaleY", "alpha" => "getAlpha", _ => "getRotation" };
                (format!("{}.{get}()", paren(&v)), Ty::Num)
            }
            // a record it was handed (the stage's bounds in the source's units, its palette): its fields
            // (a clip record's drawing: the record, which keeps what is drawn into it)
            Expr::Member(o, n) if n == "graphics" && self.a_value(o) && self.expr(o).is_ok_and(|(_, t)| t == Ty::Rect) => self.expr(o)?,
            Expr::Member(o, n) if self.a_value(o) && self.expr(o).is_ok_and(|(_, t)| t == Ty::Rect) => {
                let (v, _) = self.expr(o)?;
                (format!("{}.{n}", paren(&v)), if matches!(n.as_str(), "x" | "y" | "width" | "height" | "currentFrame" | "totalFrames") { Ty::Num } else { Ty::Rect })
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
                    // (this engine compares two whole numbers by their difference, which overflows past
                    // 2^31: 0 > -2^31 is false there. its least and greatest are 2^30 either side, so any
                    // value the code meets compares right against them)
                    ("int", "MAX_VALUE") => "1073741823".into(), ("int", _) => "-1073741824".into(),
                    ("uint", "MAX_VALUE") => "4294967295.0".into(), _ => "0".into(),
                }, Ty::Num)
            }
            // a field of a record one of its own declaration methods declares: its value, as written
            Expr::Member(c, f) if matches!(&**c, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(o, m) if self.expr(o).is_ok_and(|(w, _)| w == "self") && self.declared_field(m, f).is_some())) => {
                let Expr::Call(g, _) = &**c else { unreachable!() };
                let Expr::Member(_, m) = &**g else { unreachable!() };
                (self.declared_field(m, f).unwrap_or_default(), Ty::Any)
            }
            // how many clips the clip its animation shows places (by name)
            Expr::Member(o, n) if n == "numChildren" && self.o.fighter && self.own_stance_value(o) => {
                let f = self.stance_kids();
                return Ok((format!("{f}().length"), Ty::Num));
            }
            // where the clip its animation shows sits in its root, and where a named clip that clip
            // places sits in it: the placements each animation's frames make
            Expr::Member(o, n) if matches!(n.as_str(), "x" | "y") && self.o.fighter && (self.stance_child(o).is_some() || self.own_stance(o)) => {
                return Ok((self.placed_at(o, n == "x")?, Ty::Num));
            }
            // where an object (or its clip, drawn as it) is, in the source's units
            Expr::Member(o, n) if matches!(n.as_str(), "x" | "y") && self.a_value(o) && self.expr(o).is_ok_and(|(_, t)| t == Ty::Body) => {
                let (v, _) = self.expr(o)?;
                // (its own animation's clip's place in itself, which no plain form reads yet; its root
                // clip's, `getMC()`, is where it is)
                let root = matches!(&**o, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(_, m) | Expr::Name(m) if m == "getMC"));
                if v == "self" && !self.o.fighter && !root { return Err("reads where its own clip sits in it".into()) }
                (format!("{}.get{}() / {}", paren(&v), n.to_uppercase(), number(self.o.size)), Ty::Num)
            }
            Expr::Call(f, a) if a.is_empty() && matches!(&**f, Expr::Member(o, m) if matches!(m.as_str(), "getMC" | "stanceClip" | "getStanceMC") && !matches!(**o, Expr::This)) => {
                let Expr::Member(o, _) = &**f else { unreachable!() };
                let (who, _) = self.expr(o)?;
                // (the camera's: its view, as the source reads it)
                if who == "match.getCamera()" { self.util("cameraClip")?; return Ok(("cameraClip()".into(), Ty::Rect)) }
                // (its own: the object, as this engine draws its clip as the object)
                (who, Ty::Body)
            }
            // its clip's filters, read: none here (its colours are the engine's shaders, `setColorFilters`)
            // (the colour settings its code last put on it, `colorFilters`; none, nothing)
            Expr::Member(o, n) if n == "filters" && self.own_clip_call(o) => {
                self.util("colorFilters")?;
                return Ok(("colorFiltersOf(self)".into(), Ty::Any));
            }
            // a field of a field of its own that no code ever fills (the source's engine fills it, in a
            // mode this engine has none of): nothing, as that field is
            Expr::Member(o, n) if matches!(&**o, Expr::Member(t, f) if matches!(**t, Expr::This) && self.never_filled(f)) || matches!(&**o, Expr::Name(f) if self.never_filled(f)) || matches!(&**o, Expr::Local(r) if self.empty_locals.contains(r)) => {
                let (v, _) = self.expr(o)?;
                return Ok((format!("({v} == null ? null : {v}.{})", ident(n)), Ty::Any));
            }
            // a getter of another object of its package's: what that object publishes for it
            Expr::Member(o, n) if !matches!(**o, Expr::This) && !self.is_clip(o) && self.a_value(o)
                && self.o.model.classes.values().any(|c| !self.o.runtime.provides(&c.name) && c.methods.iter().any(|m| m.kind == MethodKind::Getter && m.name == *n))
                && self.expr(o).is_ok_and(|(w, t)| w != "self" && matches!(t, Ty::Object | Ty::Any)) => {
                let (who, _) = self.expr(o)?;
                let n = ident(n);
                return Ok((format!("({who} != null && !{who}.isDisposed() && {who}.exports.{n} != null ? {who}.exports.{n}() : null)"), Ty::Any));
            }
            // where the stage's midground sits in the stage's space: at its origin
            Expr::Member(o, n) if matches!(n.as_str(), "x" | "y") && matches!(&**o, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(_, m) if m == "getMidground")) => {
                return Ok((number(if n == "x" { self.o.origin.0 } else { self.o.origin.1 }), Ty::Num));
            }
            // where an effect or object it keeps is, as its write above puts it there
            Expr::Member(o, n) if matches!(n.as_str(), "x" | "y") && !matches!(**o, Expr::This) && !self.is_clip(o) && !matches!(&**o, Expr::Name(p) if p == "parent")
                && self.owner_clip(o).is_none() && self.expr(o).is_ok_and(|(w, t)| t == Ty::Object && w != "self") => {
                let (who, _) = self.expr(o)?;
                return Ok((format!("({who} == null ? 0 : {who}.get{}() / {})", n.to_uppercase(), number(self.o.size)), Ty::Num));
            }
            // a field of its owner's clip (its root, or the clip its animation shows), or where it is
            Expr::Member(mc, n) if self.owner_clip(mc).is_some() && !matches!(n.as_str(), "currentFrameLabel" | "currentLabel") => {
                let (who, clip) = self.owner_clip(mc).unwrap_or_default();
                if matches!(n.as_str(), "x" | "y") { return Ok((format!("{who}.get{}() / {}", n.to_uppercase(), number(self.o.size)), Ty::Num)) }
                // (a field every clip class of its package that puts anything in it puts the object
                // itself in: that object)
                let holders: Vec<&ClassModel> = self.o.model.classes.values().filter(|c| {
                    let mut set = false;
                    for m in &c.methods { walk(&m.function.body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, f), _) = st { if f == n { set = true } }); }
                    set
                }).collect();
                if !holders.is_empty() && holders.iter().all(|c| self.holds_itself(c, n)) { return Ok((who, Ty::Object)) }
                return Ok((format!("{}({who}, {}, {})", self.owner_field("get")?, quote(&clip), quote(n)), Ty::Any));
            }
            // whether an object's clip shows: whether the object does
            Expr::Member(mc, n) if n == "visible"
                && matches!(&**mc, Expr::Call(f, a) if a.is_empty() && matches!(&**f, Expr::Member(_, g) if g == "getMC")) => {
                let Expr::Call(f, _) = &**mc else { unreachable!() };
                let Expr::Member(who, _) = &**f else { unreachable!() };
                let (who, _) = self.expr(who)?;
                return Ok((format!("{who}.getVisible()"), Ty::Bool));
            }
            // the label another object's clip is on: its owner's, from the animation it plays
            Expr::Member(mc, n) if matches!(n.as_str(), "currentFrameLabel" | "currentLabel")
                && matches!(&**mc, Expr::Call(f, a) if a.is_empty() && matches!(&**f, Expr::Member(_, g) if g == "getMC")) => {
                let Expr::Call(f, _) = &**mc else { unreachable!() };
                let Expr::Member(who, _) = &**f else { unreachable!() };
                let (who, _) = self.expr(who)?;
                // (a fighter's own: the label of the animation it plays, the same way)
                if who == "self" && !self.o.fighter { return Err("reads its own clip's label".into()) }
                return Ok((format!("{}({who})", self.owner_label()?), Ty::Str));
            }
            // the label another object's clip is on, as it publishes it (`exports.__stanceLabel`)
            Expr::Member(mc, n) if matches!(n.as_str(), "currentFrameLabel" | "currentLabel")
                && matches!(&**mc, Expr::Call(f, a) if a.is_empty() && matches!(&**f, Expr::Member(o, g) if matches!(g.as_str(), "getStanceMC" | "stanceClip") && self.expr(o).is_ok_and(|(w, _)| w != "self"))) => {
                let Expr::Call(f, _) = &**mc else { unreachable!() };
                let Expr::Member(who, _) = &**f else { unreachable!() };
                let (who, _) = self.expr(who)?;
                if !self.helpers.contains_key("stanceLabelOf") { self.helpers.insert("stanceLabelOf".into(), helper(self.o.runtime, "utils", "stanceLabelOf", "stanceLabelOf", &[], &[])?); }
                return Ok((format!("stanceLabelOf({who})"), Ty::Str));
            }
            // the label an effect it holds is on: the animation it plays (each label of an effect
            // is an animation of it)
            Expr::Member(o, f) if matches!(f.as_str(), "currentFrameLabel" | "currentLabel") && self.a_value(o) && !self.is_clip(o)
                && !matches!(**o, Expr::Call(..)) && self.field_holds_effect(o) => {
                let (v, _) = self.expr(o)?;
                if !self.helpers.contains_key("labelOf") { self.helpers.insert("labelOf".into(), helper(self.o.runtime, "utils", "labelOf", "labelOf", &[], &[])?); }
                return Ok((format!("labelOf({v})"), Ty::Str));
            }
            // a field of another object's, which its script keeps on the object it publishes
            Expr::Member(o, f) if self.o.fighter && self.a_value(o) && !self.is_clip(o) && f != "data" && f.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && self.expr(o).is_ok_and(|(w, t)| w != "self" && matches!(t, Ty::Object | Ty::Any)) => {
                let (who, _) = self.expr(o)?;
                let get = if (self.o.owner_stores)(f) { format!("__c_{}.get()", ident(f)) } else { ident(f) };
                return Ok((format!("({who} == null || {who}.exports.__source == null ? null : {who}.exports.__source.{get})"), Ty::Any));
            }
            // a cast written as a call (`MovieClip(x)`): the value itself
            Expr::Call(f, args) if args.len() == 1 && matches!(&**f, Expr::Name(n) | Expr::Member(_, n) if self.is_class_name(n) && self.method(n).is_none() && (matches!(&**f, Expr::Name(_)) || matches!(&**f, Expr::Member(t, _) if matches!(**t, Expr::This)))) => return self.expr(&args[0]),
            Expr::Call(f, args) => return self.call(f, args),
            Expr::Unary(UnOp::Not, x) => (format!("!{}", paren(&self.test(x)?)), Ty::Bool),
            Expr::Unary(UnOp::Neg, x) => { let (t, _) = self.expr(x)?; (format!("-{}", paren(&t)), Ty::Num) }
            // whether the clip it shows places a named clip of its own (`hasName()`, from the clips' placements)
            // an object tested against one of its package's classes: the class it publishes
            // a child of a clip of the stage's planes tested as a picture, or taken as one (`pxPlaced`)
            Expr::Binary(BinOp::Is, x, k) if self.o.stage && matches!(&**k, Expr::Name(n) if short(n) == "Bitmap") => {
                let (v, _) = self.expr(x)?;
                (format!("({v} != null && {v}.kind == \"bitmap\")"), Ty::Bool)
            }
            Expr::Binary(BinOp::As, x, k) if self.o.stage && matches!(&**k, Expr::Name(n) if short(n) == "Bitmap") => {
                let (v, _) = self.expr(x)?;
                (v, Ty::Rect)
            }
            // a picture shown on its own (`new Bitmap(data)`): `pixels`, drawn by `libClips`
            Expr::New(c, args) if self.o.stage && matches!(&**c, Expr::Name(n) if short(n) == "Bitmap") => {
                self.util("pixels")?;
                let d = match args.first() { Some(x) => self.expr(x)?.0, None => "null".into() };
                (format!("pxBitmap({d})"), Ty::Rect)
            }
            // a picture made by its code: an empty one its size, or one of its library's (the stage's
            // sprite shows it under `bmp_<name>`)
            Expr::New(c, args) if self.o.stage && matches!(&**c, Expr::Name(n) if self.picture_class(n)) => {
                let Expr::Name(n) = &**c else { unreachable!() };
                self.util("pixels")?;
                let vs: Vec<String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
                if short(n) == "BitmapData" {
                    let w = vs.first().cloned().unwrap_or_else(|| "0".into());
                    let h = vs.get(1).cloned().unwrap_or_else(|| "0".into());
                    (format!("pxCanvas(null, {w}, {h})"), Ty::Rect)
                } else {
                    let (w, h) = self.o.model.classes.values().find(|k| short(&k.name) == short(n)).and_then(|k| k.symbol)
                        .and_then(|sy| self.o.model.bitmaps.get(&sy).copied()).unwrap_or((0, 0));
                    (format!("pxCanvas({}, {w}, {h})", quote(&format!("bmp_{}", ident(short(n))))), Ty::Rect)
                }
            }
            Expr::Binary(BinOp::Is, x, k) if matches!(&**k, Expr::Name(n) if self.o.model.classes.values().any(|c| short(&c.name) == short(n) && !self.o.runtime.provides(&c.name))) => {
                let Expr::Name(n) = &**k else { unreachable!() };
                let (v, _) = self.expr(x)?;
                (format!("({v} != null && {v}.exports != null && {v}.exports.__className == {})", quote(short(n))), Ty::Bool)
            }
            Expr::Binary(op @ (BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe), a, b)
                if matches!((&**a, &**b), (x, Expr::Null | Expr::Undefined) | (Expr::Null | Expr::Undefined, x) if self.stance_child(x).is_some()) => {
                let x = if matches!(**b, Expr::Null | Expr::Undefined) { a } else { b };
                let has = self.child_test(x)?;
                return Ok((if matches!(op, BinOp::Ne | BinOp::StrictNe) { has } else { format!("!{has}") }, Ty::Bool));
            }
            // the API itself tested for being there: it always is
            Expr::Binary(op @ (BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe), a, b)
                if matches!((&**a, &**b), (Expr::Name(n), Expr::Null | Expr::Undefined) | (Expr::Null | Expr::Undefined, Expr::Name(n)) if matches!(short(n), "PeptideAPI" | "PeptideUtils")) =>
                ((matches!(op, BinOp::Ne | BinOp::StrictNe)).to_string(), Ty::Bool),
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
            Expr::Member(h, p) if self.lit_rt_held(h).is_some() => {
                return self.lit_rt_use(h, p, None)?.ok_or_else(|| format!("reads {p} of a record's {}, which runtime/native/classes.hx writes none of", self.lit_rt_held(h).unwrap_or_default()));
            }
            Expr::Object(kv) => {
                let mut parts = Vec::new();
                for (k, v) in kv {
                    let Expr::Str(k) = k else { return Err("makes a record with fields named as it runs".into()) };
                    if !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || k.chars().next().is_some_and(|c| c.is_ascii_digit()) { return Err(format!("makes a record with a field named {k}")) }
                    // (one of the API's classes kept as a record: its record form)
                    if let (Some(rk), Expr::New(_, a)) = (self.lit_rt.get(k).cloned(), v) {
                        self.rt_record(&rk)?;
                        let a: Vec<String> = a.iter().map(|x| self.expr(x).map(|(t, _)| t)).collect::<Result<_, _>>()?;
                        parts.push(format!("{k}: {rk}_new({})", a.join(", ")));
                        continue;
                    }
                    parts.push(format!("{k}: {}", self.expr(v)?.0));
                }
                (if parts.is_empty() { "{}".into() } else { format!("{{ {} }}", parts.join(", ")) }, Ty::Object)
            }
            // one of its own fields by a name its code builds (`this["block" + i]`), and a clip of the
            // stage's planes by one (`getMidground()["region" + i]`)
            Expr::Index(a, i) if matches!(**a, Expr::This) && !self.o.clip_parts.contains_key(&self.class) && self.expr(i).is_ok_and(|(_, t)| t == Ty::Str) => {
                let (n, _) = self.expr(i)?;
                return Ok((format!("{}({n})", self.own_field_named(false)?), Ty::Any));
            }
            Expr::Index(a, i) if self.expr(a).is_ok_and(|(w, _)| w.starts_with("planePart(")) || (self.part_field(a) && self.expr(i).is_ok_and(|(_, t)| t == Ty::Str)) => {
                let (n, _) = self.expr(i)?;
                self.util("planePart")?;
                return Ok((format!("planePart({n})"), Ty::Object));
            }
            // an entry of a dictionary it keeps (a map by any key)
            Expr::Index(a, i) if self.is_dict(a) => {
                let (a, _) = self.expr(a)?;
                let (i, _) = self.expr(i)?;
                (format!("{}.get({i})", paren(&a)), Ty::Rect)
            }
            Expr::Index(a, i) if self.is_obj_dict(a) => {
                let (a, _) = self.expr(a)?;
                let (i, _) = self.expr(i)?;
                (format!("{}.get(Std.string({i}))", paren(&a)), Ty::Rect)
            }
            // a record written out in place (or a variable only ever set to one) read by a name worked
            // out as it runs: the entry of that name, by a switch over the names it states
            Expr::Index(a, i) if self.literal_of(a).is_some() && !matches!(**i, Expr::Str(_) | Expr::Int(_) | Expr::Num(_)) && self.expr(i).is_ok_and(|(_, t)| t != Ty::Num) => {
                let kv = self.literal_of(a).unwrap_or_default();
                let mut h: u64 = 0xcbf29ce484222325;
                for b in format!("{kv:?}").bytes() { h = (h ^ b as u64).wrapping_mul(0x100000001b3); }
                let f = format!("entryOf{:x}", h & 0xffffff);
                if !self.helpers.contains_key(&f) {
                    let mut lines = vec!["// an entry of a record its code writes out, by a name worked out as it runs".to_string(), format!("function {f}(k) {{"), "\tswitch (k) {".to_string()];
                    for (k, v) in &kv {
                        let Expr::Str(k) = k else { return Err("reads a record by a name worked out as it runs".into()) };
                        let (v, _) = self.expr(v)?;
                        lines.push(format!("\t\tcase {}: return {v};", quote(k)));
                    }
                    lines.push("\t}".into());
                    lines.push("\treturn null;".into());
                    lines.push("}".into());
                    self.helpers.insert(f.clone(), lines.join("\n"));
                }
                let (i, _) = self.expr(i)?;
                return Ok((format!("{f}({i})"), Ty::Rect));
            }
            // an element of a list of clips of its planes: that clip
            Expr::Index(a, i0) if matches!(&**a, Expr::Member(t, fl) if matches!(**t, Expr::This) && self.part_lists.get(&member_key(&self.class, fl)) == Some(&true)) => {
                let (l, _) = self.expr(a)?;
                let (i, _) = self.expr(i0)?;
                self.util("planePart")?;
                return Ok((format!("planePart({}[{i}])", paren(&l)), Ty::Object));
            }
            Expr::Index(a, i0) => {
                let i = i0;
                // (an element of a list field its code only pushes records onto)
                // (a static list its class states, of records: `new Point(..)`s or `{..}`s)
                let records = |v: Expr| -> Option<Ty> {
                    let mut v = &v; while let Expr::Coerce(_, x) | Expr::Convert(_, x) = v { v = x; }
                    let items = match v { Expr::Array(xs) => Some(xs.clone()), Expr::New(c, xs) if matches!(&**c, Expr::Name(c) if short(c) == "Array") => Some(xs.clone()), _ => None }?;
                    (!items.is_empty() && items.iter().all(|x| matches!(x, Expr::Object(_)) || matches!(x, Expr::New(c, _) if matches!(&**c, Expr::Name(c) if matches!(short(c), "Point" | "Rectangle"))))).then_some(Ty::Rect)
                };
                let set_once = |n: &str| self.o.model.classes.get(&self.class).and_then(|k| static_value(k, n).or_else(|| static_set(k, n))).and_then(records);
                let learned = match &**a { Expr::Member(t, fl) if matches!(**t, Expr::This) => self.list_elem.get(&member_key(&self.class, fl)).copied().or_else(|| self.pushes_records(fl).then_some(Ty::Rect)).or_else(|| set_once(fl)), Expr::Local(r) => self.local_elem.get(r).copied(),
                    // (a list field of its own named bare, as a closure names it, or one its class's setup gives)
                    Expr::Name(n) if self.field(n).is_some() => self.list_elem.get(&member_key(&self.class, n)).copied().or_else(|| set_once(n)),
                    Expr::Name(n) => set_once(n),
                    // (an element of a list inside a list field of lists of records)
                    Expr::Index(l, _) if matches!(&**l, Expr::Member(t, fl) if matches!(**t, Expr::This) && self.nested_recs.get(&member_key(&self.class, fl)) == Some(&true)) => Some(Ty::Rect),
                    _ => None };
                let (a, ta) = self.expr(a)?;
                let (i, ti) = self.expr(i)?;
                // (a field of a record, or of the controls, named plainly)
                if let Expr::Str(k) = &**i0 {
                    if matches!(ta, Ty::Rect | Ty::Controls) && !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.starts_with(|c: char| c.is_ascii_digit()) { return Ok((format!("{}.{k}", paren(&a)), if ta == Ty::Controls { Ty::Bool } else { Ty::Any })) }
                }
                if ti == Ty::Str { return Err("reads by a name worked out as it runs".into()) }
                // (an item of a list of records is a record)
                (format!("{}[{i}]", paren(&a)), if ta == Ty::Rect || learned == Some(Ty::Rect) { Ty::Rect } else { Ty::Any })
            }
            // a clip its code makes to hold others (a weather field's container): a clip record
            Expr::New(c, a) if a.is_empty() && matches!(&**c, Expr::Name(n) if matches!(short(n), "MovieClip" | "Sprite")) => {
                self.util("clipRecord")?;
                ("clipRecord(null)".into(), Ty::Rect)
            }
            // a typed list (`new Vector.<T>()`): a list
            Expr::New(c, _) if matches!(&**c, Expr::TypeApply(..)) => ("[]".into(), Ty::Object),
            // a clip made from a particle's class held as a value (`new arg0()`): a clip record
            Expr::New(c, a) if a.is_empty() && !matches!(&**c, Expr::Name(_)) => {
                let (x, _) = self.expr(c)?;
                self.util("clipRecord")?;
                (format!("clipRecord({x}.id, {x}.w, {x}.h)"), Ty::Rect)
            }
            // one of Flash's classes the engine has its own version of (`runtime/native_classes.jsonc`)
            // a record of one of the package's own classes it keeps several of
            Expr::New(c, args) if matches!(&**c, Expr::Name(n) if self.recs.iter().any(|k| short(&k.name) == short(n))) => {
                let Expr::Name(n) = &**c else { unreachable!() };
                let id = format!("{}_new", ident(short(n)));
                // (what each maker hands it: its constructor's arguments' kinds)
                let typed: Vec<(String, Ty)> = args.iter().map(|x| self.expr(x)).collect::<Result<_, String>>()?;
                if self.learning {
                    for (i, (_, t)) in typed.iter().enumerate() {
                        let k = (id.clone(), i);
                        let was = self.param_ty.get(&k).copied();
                        self.param_ty.insert(k, match was { None => *t, Some(w) => join(w, *t) });
                    }
                }
                let vs: Vec<String> = typed.into_iter().map(|(v, _)| v).collect();
                (format!("{id}({})", vs.join(", ")), Ty::Object)
            }
            Expr::New(c, args) => {
                let Expr::Name(n) = &**c else { return Err("makes an object of a class worked out as it runs".into()) };
                // (a clip of its package's whose own code only moves its own timeline: a clip record,
                // drawn nowhere here)
                let pc = self.o.model.classes.get(n).or_else(|| self.o.model.classes.values().find(|c| short(&c.name) == n.as_str()));
                if let Some(pc) = pc {
                    // (one that is one of its effects: drawn as that effect where its code puts it)
                    if matches!(short(&pc.super_name), "MovieClip" | "Sprite") && crate::emit::native_projectile::is_quiet_inside(self.o.model, self.o.runtime, &pc.name) {
                        if let Some(made) = self.screen_clip(short(n)) { return Ok((made, Ty::Rect)) }
                    }
                    if matches!(short(&pc.super_name), "MovieClip" | "Sprite") && crate::emit::native_projectile::is_quiet_inside(self.o.model, self.o.runtime, &pc.name) {
                        self.gaps.insert(format!("makes a {} clip, which is drawn nowhere here", short(n)));
                        self.util("clipRecord")?;
                        // (with the clips it places under names, each a record of its own, as its code reaches them)
                        let mut lines = Vec::new();
                        clip_record_of(self.o.model, pc.symbol, "r", 0, &mut lines);
                        if lines.is_empty() { return Ok(("clipRecord(null)".into(), Ty::Rect)) }
                        let f = format!("newClip{}", ident(short(n)));
                        if !self.helpers.contains_key(&f) {
                            self.helpers.insert(f.clone(), format!("// a {} its code makes, with the clips it places under names\nfunction {f}() {{\n\tvar r = clipRecord(null);\n{}\n\treturn r;\n}}", short(n), lines.iter().map(|l| format!("\t{l}")).collect::<Vec<_>>().join("\n")));
                        }
                        return Ok((format!("{f}()"), Ty::Rect));
                    }
                }
                // (a game object of its package's with no clip, made with `new`: nothing here runs it,
                // so it is a record with nothing in it)
                if let Some(k) = self.o.model.classes.values().find(|c| (c.name == *n || short(&c.name) == n.as_str()) && c.symbol.is_none() && self.o.runtime.provides(&c.super_name) && short(&c.super_name) != "PeptideStage") {
                    self.gaps.insert(format!("makes a {} with `new`, a game object with no clip, which nothing here runs", short(&k.name)));
                    return Ok(("{}".into(), Ty::Rect));
                }
                if self.o.model.classes.contains_key(n) || self.o.model.classes.values().any(|c| short(&c.name) == n.as_str()) { return Err(format!("makes an object of its package's {n}")) }
                let make = self.o.runtime.native(n).and_then(|k| k.make.clone()).ok_or_else(|| format!("makes an object of {n}, which has no row in native_classes.jsonc"))?;
                let mut texts = Vec::new();
                let mut tys = Vec::new();
                for a in args { let (t, ty) = self.expr(a)?; texts.push(t); tys.push(ty); }
                // (a list made of its elements: all but one number, its length)
                if short(n) == "Array" && !texts.is_empty() && !(texts.len() == 1 && tys[0] == Ty::Num) { return Ok((format!("[{}]", texts.join(", ")), Ty::Object)) }
                // (one whose arguments its form reads more than once, and one of them is worked out as it
                // runs: a function of its form, handed each once)
                let reread = (0..10).any(|i| make.matches(&format!("{{{i}}}")).count() > 1 && texts.get(i).is_some_and(|v| !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')));
                if reread && !make.contains("{args}") {
                    let f = format!("make{}", ident(short(n)));
                    if !self.helpers.contains_key(&f) {
                        let mut body = make.clone();
                        for i in 0..10 { body = body.replace(&format!("{{{i}}}"), &format!("a{i}")); }
                        let ps: Vec<String> = (0..10).filter(|i| make.contains(&format!("{{{i}}}"))).map(|i| format!("?a{i}")).collect();
                        self.helpers.insert(f.clone(), format!("// a {} made, its arguments handed once\nfunction {f}({}) {{\n\treturn {body};\n}}", short(n), ps.join(", ")));
                    }
                    let used = (0..10).filter(|i| make.contains(&format!("{{{i}}}"))).max().map(|m| m + 1).unwrap_or(0);
                    let vs: Vec<String> = (0..used).map(|i| texts.get(i).cloned().unwrap_or_else(|| "null".into())).collect();
                    return Ok((format!("{f}({})", vs.join(", ")), if matches!(short(n), "Array" | "Vector" | "Dictionary") { Ty::Object } else { Ty::Rect }));
                }
                let mut out = make.replace("{args}", &texts.join(", "));
                for i in 0..10 {
                    let slot = format!("{{{i}}}");
                    let Some(v) = texts.get(i) else { out = out.replace(&slot, "null"); continue };
                    out = out.replace(&slot, &paren(v));
                }
                // (a list or a map is no record: its own calls are its own)
                (out, if matches!(short(n), "Array" | "Vector" | "Dictionary") { Ty::Object } else { Ty::Rect })
            }
            Expr::Assign(..) => return Err("uses an assignment as a value".into()),
            // a clip class of its library named as a value (an effect to make of it): its name
            Expr::Name(n) if self.o.model.classes.values().any(|c| short(&c.name) == short(n) && c.symbol.is_some()) => (quote(short(n)), Ty::Str),
            other => { if std::env::var("PEPTIDE_CENSUS_ALL").is_ok() { eprintln!("[census-expr] {other:?}"); } return Err(describe(other)) }
        })
    }

    /// one of its own fields or methods, read
    fn own(&mut self, n: &str) -> Result<(String, Ty), String> {
        // (a field its code never writes that names a clip its clip places is that clip, which
        // Flash binds to it: drawn whole into its art here, with no object of its own to turn)
        // (one of its parts, drawn apart: `projPart`'s record)
        if let Some(made) = self.own_part(n) { return Ok((made, Ty::Rect)) }
        if let Some(at) = self.placed_child(n) {
            // (a record holding its look as the clip's placement starts it, which its code reads
            // and writes as the source's does; drawn nowhere, so a change its code makes to it shows
            // nothing: said where its code makes one)
            if self.turns_placed(n) { self.gaps.insert(format!("turns its {n} clip, whose art is drawn into its own")); }
            let id = format!("{}__clip", ident(n));
            self.kept.insert(id.clone(), format!("// the look of its {n} clip, as its code turns it\nvar {id} = self.makeObject(null);"));
            if !self.helpers.contains_key("placedClip") {
                self.helpers.insert("placedClip".into(), "// a clip its clip places, which its code turns: a record of its look, as its placement starts it\nfunction placedClip(c, x, y, r, sx, sy) {\n\tvar v = c.get();\n\tif (v == null) { v = { x: x, y: y, rotation: r, scaleX: sx, scaleY: sy, alpha: 1, visible: true, currentFrame: 1 }; c.set(v); }\n\treturn v;\n}".into());
            }
            let (x, y, r, sx, sy) = at;
            // (with the clips it places under names, each a record of its own, as its code reaches them)
            let sym = self.o.model.classes.get(&self.class).and_then(|c| c.symbol)
                .and_then(|s| self.o.model.sprites.get(&s)).and_then(|sp| sp.frames.iter().flatten().find(|p| p.name.as_deref() == Some(n)).map(|p| p.char_id));
            let mut kids = Vec::new();
            clip_record_of(self.o.model, sym, "v", 0, &mut kids);
            if !kids.is_empty() {
                let f = format!("{}__kids", ident(n));
                if !self.helpers.contains_key(&f) {
                    self.helpers.insert(f.clone(), format!("// the clips its {n} clip places under names, each a record of its own\nfunction {f}(v) {{\n\tif (v.kids != null && v.kids.length > 0) return v;\n\tif (v.kids == null) v.kids = [];\n{}\n\treturn v;\n}}", kids.iter().map(|l| format!("\t{l}")).collect::<Vec<_>>().join("\n")));
                }
                return Ok((format!("{f}(placedClip({id}, {}, {}, {}, {}, {}))", number(x), number(y), number(r), number(sx), number(sy)), Ty::Rect));
            }
            return Ok((format!("placedClip({id}, {}, {}, {}, {}, {})", number(x), number(y), number(r), number(sx), number(sy)), Ty::Rect));
        }
        if let Some((id, holds)) = self.field(n) {
            let key = member_key(&self.class, n);
            let plain = self.plain.contains(&key);
            self.read.insert(key);
            return Ok(match holds { Holds::Itself | Holds::Clip => (id, Ty::Object), Holds::Value(t) if plain => (id, t), Holds::Value(t) => (format!("{id}.get()"), t) });
        }
        if let Some(id) = self.method(n) { return Ok((id, Ty::Object)) }
        // (one read through its getter)
        if let Some(id) = self.method(&format!("get:{n}")) { return Ok((format!("{id}()"), Ty::Any)) }
        // the power it holds, copied from another fighter
        if n == "KirbyPower" && self.o.fighter { self.copy_power()?; return Ok(("powerName()".into(), Ty::Str)) }
        // (a clip's own property its package never sets anywhere: Flash gives undefined, nothing)
        if self.o.model.classes.get(&self.class).is_some_and(|c| matches!(short(&c.super_name), "MovieClip" | "Sprite")) && !self.set_anywhere(n) {
            return Ok(("null".into(), Ty::Object));
        }
        // (the clip that is an object's whole art: its place is the object's, in the source's units,
        // and the object is what holds it)
        if self.is_root_clip() {
            match n {
                "x" => return Ok((format!("(self.getX() / {})", number(self.o.size)), Ty::Num)),
                "y" => return Ok((format!("(self.getY() / {})", number(self.o.size)), Ty::Num)),
                "parent" => return Ok(("self".into(), Ty::Object)),
                _ => {}
            }
        }
        Err(format!("reads {n}, which is not its own"))
    }

    /// a closure the code makes, as a function of the script's own: one that captures none of the
    /// function's variables reads only the object's fields and methods, so it is the same anywhere
    fn closure(&mut self, f: &Function) -> Result<String, String> {
        // (a variable of its maker's it reaches under its package's name: that variable)
        let qualified = |n: &str| n.contains('.') && self.captured.contains(&ident(short(n))) && !self.o.model.classes.values().any(|c| c.name == n);
        let renamed = Function { body: map_stmts(f.body.clone(), &mut |e| match &e { Expr::Name(n) if qualified(n) => Expr::Name(short(n).to_string()), _ => e }), ..f.clone() };
        let f = &renamed;
        let mut captures = false;
        crate::model::ast::walk_exprs(&f.body, &mut |e| if let Expr::Name(n) = e {
            if self.field(n).is_none() && self.method(n).is_none() && n != "Math" && !n.contains("Peptide") { captures = true }
        });
        // (one that reads its maker's variables is written where it is made, and sees them)
        if captures || !f.activation.is_empty() {
            // (a name it calls that is the object's own API call, `getOwner()`, is the object's: no variable)
            let mut called: BTreeSet<String> = BTreeSet::new();
            crate::model::ast::walk_exprs(&f.body, &mut |e| if let Expr::Call(g, _) = e { if let Expr::Name(n) = &**g { called.insert(n.clone()); } });
            // (and the API's own calls the writer settles itself: an effect it makes)
            let api: BTreeSet<String> = called.into_iter().filter(|n| calls().rows.contains_key(n) || matches!(n.as_str(), "createVfx" | "createVfxOverlay" | "gotoAndStop" | "gotoAndPlay" | "stop" | "play")
                || (self.o.stage && matches!(n.as_str(), "getBackground" | "getForeground" | "getMidground" | "getShadowMask" | "getShadowMaskMC" | "getCameraBackgrounds"))).collect();
            let unknown: Vec<String> = { let mut u = Vec::new(); crate::model::ast::walk_exprs(&f.body, &mut |e| if let Expr::Name(n) = e {
                if self.field(n).is_none() && self.method(n).is_none() && n != "Math" && !n.contains("Peptide") && !self.captured.contains(&ident(n)) && !f.activation.iter().any(|(_, a)| a == n) && !self.o.model.classes.values().any(|c| short(&c.name) == n || c.name == *n) && self.o.runtime.native(n).is_none() && !api.contains(n) { u.push(n.clone()) }
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
        // (an object's, to a label written out: its own jump by that label)
        if let (Expr::Str(l), false) = (label, self.o.fighter) { return Ok(self.jump(l)?.trim_end_matches(';').to_string()) }
        let (l, ty) = self.expr(label)?;
        // (by number: no label of its clip is needed)
        if ty == Ty::Num {
            self.clip_frame_helper()?;
            if let Some(def) = self.jump_by_number.clone() { self.helpers.insert("toClipFrame".into(), def); }
            return Ok(format!("toClipFrame({l})"));
        }
        let saved = std::mem::take(&mut self.class);
        let r = self.stance_jump("");
        self.class = saved;
        r?;
        if ty == Ty::Str { return Ok(format!("stanceJump({l})")) }
        self.clip_frame_helper()?;
        if let Some(def) = self.jump_by_number.clone() { self.helpers.insert("toClipFrame".into(), def); }
        Ok(format!("if (Std.isOfType({l}, String)) stanceJump({l}); else toClipFrame({l})"))
    }

    /// the helper that reads an engine box's stats as the source's record of them (in the source's
    /// units, a fixed launch already in its base): the box's owner's player number and team with them
    fn box_record(&mut self) -> Result<String, String> {
        if !self.helpers.contains_key("boxRecordOf") {
            let mut fields = Vec::new();
            for k in ["damage", "direction", "power", "kbConstant", "hitStun"] {
                let Ok((field, scale)) = hitbox_field(k) else { continue };
                let back = scale.map(|s| format!(" / {}", number(s))).unwrap_or_default();
                fields.push(format!("{k}: b.{field}{back}"));
            }
            fields.push("weightKB: 0, isForward: (b.owner == null || b.owner.isFacingRight()), effect_id: null".into());
            fields.push("player_id: (b.owner == null ? 0 : match.getCharacters().indexOf(b.owner.getRootOwner()) + 1)".into());
            fields.push("team_id: (b.owner == null ? -1 : b.owner.getRootOwner().getTeam())".into());
            self.helpers.insert("boxRecordOf".into(), format!("// a box's stats as the source's record: the engine's box, in the source's units\nfunction boxRecordOf(b) {{\n\treturn b == null ? null : {{ {} }};\n}}", fields.join(", ")));
        }
        Ok("boxRecordOf".into())
    }

    /// the class of its own whose static field a read by class name (`castlesiege.ROCK_TIME`) names
    fn static_class(&self, n: &str, f: &str) -> Option<String> {
        let all = self.o.itself.into_iter().chain(self.o.classes.iter().copied());
        all.filter(|c| short(&c.name) == short(n) && c.fields.iter().any(|x| x.is_static && x.name == f)).map(|c| c.name.clone()).next()
    }

    /// the helper that reads (or writes) one of its own class's fields by a name worked out as it runs
    fn own_field_named(&mut self, set: bool) -> Result<String, String> {
        let name = format!("{}_{}", if set { "setFieldNamed" } else { "fieldNamed" }, ident(short(&self.class)));
        if self.helpers.contains_key(&name) { return Ok(name) }
        let fields: Vec<String> = self.o.model.classes.get(&self.class).map(|c| c.fields.iter().filter(|f| !f.is_static).map(|f| f.name.clone()).collect()).unwrap_or_default();
        let mut cases = Vec::new();
        for f in fields {
            let Some((id, holds)) = self.field(&f) else { continue };
            let plain = self.plain.contains(&member_key(&self.class, &f)) || holds == Holds::Itself;
            cases.push(if set {
                if plain { format!("case {}: {id} = v;", quote(&f)) } else { format!("case {}: {id}.set(v);", quote(&f)) }
            } else if plain { format!("case {}: return {id};", quote(&f)) } else { format!("case {}: return {id}.get();", quote(&f)) });
        }
        let def = helper(self.o.runtime, "helpers", if set { "own_field_set" } else { "own_field_named" }, &name, &[], &cases)?;
        self.helpers.insert(name.clone(), def);
        Ok(name)
    }

    /// a constant some class of its package states for a name (one value, whichever class states it)
    fn const_of(&self, f: &str) -> Option<String> {
        let mut vals: BTreeSet<String> = BTreeSet::new();
        for c in self.o.model.classes.values().filter(|c| !self.o.runtime.provides(&c.name)) {
            for x in c.fields.iter().filter(|x| x.name == f && x.is_static && x.is_const) {
                vals.insert(x.default.as_ref().and_then(literal)?);
            }
        }
        if vals.len() == 1 { vals.into_iter().next() } else { None }
    }

    /// whether a value is a field its class only ever sets to a clip it makes (`new MovieClip()`)
    fn holds_clip_record(&self, e: &Expr) -> bool {
        let Expr::Member(t, f) = e else { return false };
        if !matches!(**t, Expr::This) { return false }
        let Some(c) = self.o.model.classes.get(&self.class) else { return false };
        let mut any = false;
        let mut other = false;
        for b in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
            walk(b, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, n), v) = st {
                if n != f { return }
                if matches!(v, Expr::New(k, _) if matches!(&**k, Expr::Name(k) if matches!(short(k), "MovieClip" | "Sprite"))) { any = true } else { other = true }
            });
        }
        any && !other
    }

    /// what a loop over a value's names walks, where the value says: a list (true) or a map (false)
    fn walks_list(&self, e: &Expr) -> Option<bool> {
        match e {
            Expr::Array(_) => Some(true),
            Expr::New(c, _) if matches!(&**c, Expr::Name(n) if short(n) == "Array" || short(n).starts_with("Vector")) => Some(true),
            Expr::Member(..) if self.is_dict(e) || self.is_obj_dict(e) => Some(false),
            Expr::Member(t, f) if matches!(**t, Expr::This) => {
                let ty = self.o.model.classes.get(&self.class)?.fields.iter().find(|d| d.name == *f)?.type_name.clone()?;
                let ty = short(&ty);
                (ty == "Array" || ty.starts_with("Vector")).then_some(true)
            }
            _ => None,
        }
    }

    /// whether a value is a dictionary its class keeps in a field (only ever made as one)
    fn is_dict(&self, e: &Expr) -> bool {
        let Expr::Member(t, f) = e else { return false };
        // (another object's field: the map its own class keeps there)
        let class = if matches!(**t, Expr::This) { self.class.clone() } else { match self.field_keeper(t, f) { Some(k) => k, None => return false } };
        self.dict_in(&class, f)
    }

    fn dict_in(&self, class: &str, f: &str) -> bool {
        let Some(c) = self.o.model.classes.get(class) else { return false };
        let mut any = false;
        let mut other = false;
        for b in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
            walk(b, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, n), v) = st {
                if n != f { return }
                if matches!(v, Expr::New(k, _) if matches!(&**k, Expr::Name(k) if short(k) == "Dictionary")) { any = true } else { other = true }
            });
        }
        any && !other
    }

    /// the one class of the package that declares a field another object's is read by (`who.f`),
    /// where `who` is not this object
    fn field_keeper(&self, who: &Expr, f: &str) -> Option<String> {
        if matches!(who, Expr::This) { return None }
        let mut keepers = self.o.model.classes.values().filter(|c| c.fields.iter().any(|d| d.name == f && !d.is_static) && !self.o.runtime.provides(&c.name));
        let k = keepers.next()?;
        if keepers.next().is_some() { return None }
        (k.name != self.class).then(|| k.name.clone())
    }

    /// a field it keeps an object in that its code uses as a map (made empty, filled and read by keys
    /// worked out as it runs, never by a field's name): a map by text, as the source's object keys
    fn is_obj_dict(&self, e: &Expr) -> bool {
        let Expr::Member(t, f) = e else { return false };
        let class = if matches!(**t, Expr::This) { self.class.clone() } else { match self.field_keeper(t, f) { Some(k) => k, None => return false } };
        self.obj_dict_in(&class, f)
    }

    fn obj_dict_in(&self, class: &str, f: &str) -> bool {
        let Some(c) = self.o.model.classes.get(class) else { return false };
        let (mut made, mut other, mut keyed, mut named) = (false, false, false, false);
        // (a list made empty and then written by text keys is a map by text too)
        let mut list = false;
        let mut texts = false;
        let field = |x: &Expr| matches!(x, Expr::Member(t, n) if matches!(**t, Expr::This) && n == f);
        for b in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
            walk(b, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, n), v) = st {
                if n != f { return }
                if matches!(v, Expr::Object(kv) if kv.is_empty()) { made = true }
                else if matches!(v, Expr::New(c, a) if a.is_empty() && matches!(&**c, Expr::Name(n) if short(n) == "Array")) { made = true; list = true }
                // (let go, set to nothing: still the field it keeps its map in)
                else if matches!(v, Expr::Null | Expr::Undefined) {}
                else { other = true }
            });
            walk(b, &mut |st| if let Stmt::Assign(Place::Index(a, k), _) = st { if field(a) { if matches!(k, Expr::Str(_)) { texts = true } else { keyed = true } } });
            walk_exprs(b, &mut |x| match x {
                Expr::Index(a, k) if field(a) && !matches!(&**k, Expr::Str(_)) => keyed = true,
                Expr::Member(a, _) if field(a) => named = true,
                _ => {}
            });
        }
        made && !other && !named && (keyed || (list && texts))
    }

    /// the API class a field keeps a list of records of, where it keeps one
    fn rt_list(&self, f: &str) -> Option<String> { self.rt_lists.get(&member_key(&self.class, f)).cloned() }

    /// the record form of one of the API's classes, written once (`classes.hx`)
    /// the API class a record written out in place keeps in the field `h` reads, if it is one
    fn lit_rt_held(&self, h: &Expr) -> Option<String> {
        let Expr::Member(o, f) = h else { return None };
        if matches!(**o, Expr::This) { return self.rt_fields.get(&member_key(&self.class, f)).cloned() }
        self.lit_rt.get(f).cloned()
    }

    /// a call (`args`) or a read on one of the API's classes a record written out in place keeps:
    /// that class's record form, handed what the record keeps
    fn lit_rt_use(&mut self, h: &Expr, m: &str, args: Option<&[Expr]>) -> Result<Option<(String, Ty)>, String> {
        let Some(k) = self.lit_rt_held(h) else { return Ok(None) };
        let name = if args.is_some() { m.to_string() } else { format!("get_{m}") };
        let t = flat_template(self.o.runtime, &format!("{k} as a record")).unwrap_or_default();
        if !crate::dyn_re(&format!(r"\bfunction\s+{}_{}\(", regex::escape(&k), regex::escape(&name))).is_match(&t) { return Ok(None) }
        self.rt_record(&k)?;
        let (held, _) = self.expr(h)?;
        let mut all = vec![held];
        for a in args.unwrap_or(&[]) { all.push(self.expr(a)?.0); }
        let ty = match name.as_str() { "get_completed" => Ty::Bool, "get_elapsedFrames" | "get_duration" => Ty::Num, "get_data" => Ty::Rect, _ => Ty::Any };
        Ok(Some((format!("{k}_{name}({})", all.join(", ")), ty)))
    }

    fn rt_record(&mut self, k: &str) -> Result<(), String> {
        let name = format!("{k}_record");
        if !self.helpers.contains_key(&name) { self.helpers.insert(name, helper(self.o.runtime, "classes", &format!("{k} as a record"), "", &[], &[])?); }
        Ok(())
    }

    /// whether its clip's frames are counted here: a fighter's, or an object's whose animations
    /// show its clips' frames (`clip_frames`)
    fn counts_frames(&self) -> bool { self.o.fighter || self.o.clip_frames.values().any(|a| !a.is_empty()) }

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

    /// how many frames the clip of the class being written has (one count: every clip it is bound to alike)
    fn total_frames(&self) -> Result<String, String> {
        let counts: BTreeSet<usize> = self.o.model.sprites.iter()
            .filter(|(sym, _)| self.o.model.class_of(**sym).is_some_and(|c| c.name == self.class))
            .map(|(_, sp)| sp.frame_count).collect();
        match counts.len() {
            1 => Ok(counts.into_iter().next().unwrap_or_default().to_string()),
            0 => Err("reads totalFrames of a clip it is bound to none of".into()),
            _ => Err("reads totalFrames of clips of different lengths".into()),
        }
    }

    /// a flattened field of its own, as (its name in the script, the class it holds)
    fn flat_of(&self, e: &Expr) -> Option<(String, String)> {
        let Expr::Member(t, f) = e else { return None };
        // (a field of the object's own class, reached from one of its clips through the object)
        if !matches!(**t, Expr::This) && !self.is_clip(t) {
            let me = self.o.itself?;
            let through = match &**t { Expr::Member(tt, g) if matches!(**tt, Expr::This) => self.field(g).is_some_and(|(_, h)| h == Holds::Itself), _ => is_itself(t) };
            if !through { return None }
            let k = self.flat.get(&member_key(&me.name, f))?.clone();
            return Some((self.fields.get(&member_key(&me.name, f))?.0.clone(), k));
        }
        let k = self.flat.get(&member_key(&self.class, f))?.clone();
        Some((self.field(f)?.0, k))
    }

    /// a call of a flattened field's class, as the function `classes.hx` writes for it
    fn flat_call(&mut self, id: &str, class: &str, name: &str, args: &[String]) -> Result<(String, Ty), String> {
        // (one of the package's own classes: its fields and methods under the field's name)
        if self.pkg_ids.contains_key(class) {
            if let Some(f) = name.strip_prefix("get_") {
                if let Some(m) = self.methods.get(&member_key(class, &format!("get:{f}"))).cloned() { return Ok((format!("{m}()"), Ty::Any)) }
                if let Some(m) = self.methods.get(&member_key(class, f)).cloned() { return Ok((m, Ty::Object)) }
                let key = member_key(class, f);
                let (fid, holds) = self.fields.get(&key).cloned().ok_or_else(|| format!("reads {f} of its {}, which it has none of", short(class)))?;
                self.read.insert(key);
                let ty = match holds { Holds::Value(t) => t, _ => Ty::Any };
                return Ok((format!("{fid}.get()"), ty));
            }
            if let Some(f) = name.strip_prefix("set_") {
                if let Some(m) = self.methods.get(&member_key(class, &format!("set:{f}"))).cloned() { return Ok((format!("{m}({})", args.join(", ")), Ty::Any)) }
                let key = member_key(class, f);
                let (fid, _) = self.fields.get(&key).cloned().ok_or_else(|| format!("writes {f} of its {}, which it has none of", short(class)))?;
                self.read.insert(key);
                return Ok((format!("{fid}.set({})", args.join(", ")), Ty::Any));
            }
            let m = self.methods.get(&member_key(class, name)).cloned().ok_or_else(|| format!("calls {name} of its {}, which it has none of", short(class)))?;
            let ty = self.method_ty.get(&m).copied().unwrap_or(Ty::Any);
            return Ok((format!("{m}({})", args.join(", ")), ty));
        }
        // (one inside a record: the record form of its class, handed the record it keeps)
        if id.starts_with("me.") {
            let t = flat_template(self.o.runtime, &format!("{class} as a record")).ok_or_else(|| format!("keeps a {class} in a record, which runtime/native/classes.hx writes no record form of"))?;
            if !crate::dyn_re(&format!(r"\bfunction\s+{}_{}\(", regex::escape(class), regex::escape(name))).is_match(&t) { return Err(format!("uses {name} of a {class} in a record, which runtime/native/classes.hx writes none of")) }
            self.helpers.entry(format!("{class}_record")).or_insert_with(|| t.lines().filter(|l| !l.starts_with("//:")).collect::<Vec<_>>().join("\n"));
            let ty = if name == "get_completed" { Ty::Bool } else { Ty::Any };
            if name == "new" { return Ok((format!("{id} = {class}_new({})", args.join(", ")), Ty::Any)) }
            let all: Vec<String> = std::iter::once(id.to_string()).chain(args.iter().cloned()).collect();
            return Ok((format!("{class}_{name}({})", all.join(", ")), ty));
        }
        let t = flat_template(self.o.runtime, class).unwrap_or_default();
        let defines = crate::dyn_re(&format!(r"\bfunction\s+FIELD_{}\(", regex::escape(name))).is_match(&t);
        if !defines { return Err(format!("uses {name} of its {class}, which runtime/native/classes.hx writes none of")) }
        self.flat_used.insert(id.to_string(), class.to_string());
        let ty = if t.lines().any(|l| l.trim() == format!("//: bool {name}")) { Ty::Bool } else if t.lines().any(|l| l.trim() == format!("//: record {name}")) { Ty::Rect } else { Ty::Any };
        Ok((format!("{id}_{name}({})", args.join(", ")), ty))
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
        // (a static value its class states, named bare, is a value too)
        match o { Expr::This => false, Expr::Name(n) => self.captured.contains(&ident(n)) || self.field(n).is_some() || self.o.model.classes.get(&self.class).is_some_and(|k| static_value(k, n).is_some()), _ => true }
    }

    /// a helper of `helpers.hx` that takes the source's size, written once
    /// a function of `runtime/native/utils.hx`, written once
    fn util(&mut self, name: &str) -> Result<String, String> {
        // (the stage's camera settings, as the conversion declares them)
        // (the hit the star a fighter is shot out as lands, as the installed game declares it,
        // through the box-stat mapping: `star_hit`)
        if name == "ejectFoes" && !self.helpers.contains_key(name) {
            self.helpers.insert(name.into(), helper(self.o.runtime, "utils", name, name, &[("SIZE", number(self.o.size)), ("SPEED", number(self.o.speed)), ("STAR_HIT", star_hit())], &[])?);
        }
        // (the source's animation names, as the engine's: `mappings/character/animations.jsonc`)
        if name == "bodyCopy" && !self.helpers.contains_key(name) {
            let cases: Vec<String> = crate::mappings::character_animations().ssf2_to_fm.iter().map(|(k, v)| format!("case {}: anim = {};", quote(k), quote(v))).collect();
            self.helpers.insert(name.into(), helper(self.o.runtime, "utils", name, name, &[("SIZE", number(self.o.size))], &cases)?);
            for n in needs(self.o.runtime, name) { self.util(&n)?; }
        }
        if name == "cameraParameter" && !self.helpers.contains_key(name) {
            let cases: Vec<String> = crate::stage_emit::CAMERA_STATS.iter().map(|(k, v)| format!("case {}: return {};", quote(k), number(*v))).collect();
            self.helpers.insert(name.into(), helper(self.o.runtime, "utils", name, name, &[], &cases)?);
        }
        if !self.helpers.contains_key(name) {
            self.helpers.insert(name.into(), helper(self.o.runtime, "utils", name, name, &[("SIZE", number(self.o.size)), ("SPEED", number(self.o.speed)), ("ACCEL", number(self.o.accel)), ("ORIGIN_X", number(self.o.origin.0)), ("ORIGIN_Y", number(self.o.origin.1)), ("CONTENT", quote(&crate::projectile_gen::projectile_content_id(&self.o.content)))], &[])?);
            for n in needs(self.o.runtime, name) { self.util(&n)?; }
        }
        Ok(name.into())
    }

    fn sized_helper(&mut self, key: &str, name: &str) -> Result<String, String> {
        if !self.helpers.contains_key(name) { self.helpers.insert(name.into(), helper(self.o.runtime, "helpers", key, name, &[("SIZE", number(self.o.size))], &[])?); }
        Ok(name.into())
    }

    /// whether a value is a copy of its own clip
    /// a box the source numbers from one, as the engine's index from zero: worked out as it runs
    /// where the code does
    fn box_ix(&mut self, e: &Expr) -> Result<String, String> {
        if let Ok(i) = box_index(e) { return Ok(i) }
        let (v, _) = self.expr(e)?;
        Ok(format!("Std.int(Math.max(0, {} - 1))", paren(&v)))
    }

    /// whether a value its code reads is a field of its own that holds its owner: every value its
    /// classes put in that field is its owner, or one a handler is handed after a reflect has made
    /// the reflector its owner here too (`replaceOwner`)
    fn field_of_owner(&self, who: &str) -> bool {
        if self.o.fighter { return false }
        let Some(id) = who.strip_suffix(".get()") else { return false };
        let Some((key, _)) = self.fields.iter().find(|(_, (fid, _))| fid == id) else { return false };
        let Some((class, field)) = key.split_once("::") else { return false };
        let Some(c) = self.o.model.classes.get(class) else { return false };
        let mut owner = false;
        let mut other = false;
        for body in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
            walk(body, &mut |s| if let Stmt::Assign(Place::Member(Expr::This, f), v) = s {
                if f != field { return }
                if matches!(v, Expr::Call(g, a) if a.is_empty() && (matches!(&**g, Expr::Name(n) if n == "getOwner")
                    || matches!(&**g, Expr::Member(o, n) if n == "getOwner" && (matches!(**o, Expr::This) || is_itself(o) || matches!(&**o, Expr::Member(t, _) if matches!(**t, Expr::This)))))) { owner = true }
                else if !matches!(v, Expr::Local(_)) { other = true }
            });
        }
        owner && !other
    }

    /// the measured box of a child of its own clip its class reads (`getMC().name`)
    /// its own clip's extent, as measured
    fn own_box(&self) -> Option<(f64, f64, f64, f64)> {
        let mut names: Vec<String> = Vec::new();
        if let Some(c) = self.o.itself { names.push(short(&c.name).to_string()); names.push(short(&c.super_name).to_string()); }
        names.push(short(&self.class).to_string());
        names.iter().find_map(|k| self.o.clip_bounds.get(&format!("{k}>")).copied())
    }

    fn own_child_box(&self, e: &Expr) -> Option<(f64, f64, f64, f64)> {
        // (through a cast written as a call, `MovieClip(getMC().hitBox)`)
        if let Expr::Call(f, args) = e {
            if args.len() == 1 && matches!(&**f, Expr::Name(n) | Expr::Member(_, n) if self.is_class_name(n) && self.method(n).is_none()) { return self.own_child_box(&args[0]) }
        }
        let Expr::Member(c, n) = e else { return None };
        if !self.own_clip_call(c) { return None }
        // (under its own class, or the class of the package it extends; else the one child of that
        // name any object's class measured)
        let mut names: Vec<String> = Vec::new();
        if let Some(c) = self.o.itself { names.push(short(&c.name).to_string()); names.push(short(&c.super_name).to_string()); }
        for k in &names { if let Some(r) = self.o.clip_bounds.get(&format!("{k}>{n}")) { return Some(*r) } }
        let suffix = format!(">{n}");
        let found: Vec<&(f64, f64, f64, f64)> = self.o.clip_bounds.iter().filter(|(k, _)| k.ends_with(&suffix)).map(|(_, v)| v).collect();
        let first = **found.first()?;
        found.iter().all(|r| **r == first).then_some(first)
    }

    /// whether a name is a class's (Flash's own the engine has a form of, or the package's)
    fn is_class_name(&self, n: &str) -> bool {
        // (a number, text or truth called on a value converts it: no cast)
        if matches!(short(n), "Number" | "int" | "uint" | "String" | "Boolean" | "Array" | "Object" | "Date" | "Error" | "RegExp" | "XML" | "XMLList") { return false }
        self.o.runtime.native(n).is_some() || self.o.runtime.provides(n) || self.o.model.classes.values().any(|c| short(&c.name) == short(n))
    }

    fn is_clip(&self, e: &Expr) -> bool { matches!(e, Expr::Local(r) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Clip)) }

    /// whether the class being written is the object's own (its `this` the object itself)
    fn is_itself(&self) -> bool { self.o.itself.is_some_and(|c| c.name == self.class) }

    /// whether any class of the package writes a field or property of this name anywhere
    fn set_anywhere(&self, n: &str) -> bool {
        let mut found = false;
        for c in self.o.model.classes.values() {
            for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor, &c.static_init]) {
                walk(&f.body, &mut |st| if let Stmt::Assign(Place::Member(_, g) | Place::Name(g), _) = st { if g == n { found = true } });
                if found { return true }
            }
            if c.fields.iter().any(|d| d.name == n) { return true }
        }
        found
    }

    /// whether the code is the clip that is a projectile's or item's whole art (its root clip),
    /// whose own place and parent are the object's
    fn is_root_clip(&self) -> bool {
        !self.o.fighter && !self.o.stage && self.o.model.class_of(self.o.root).is_some_and(|c| c.name == self.class)
    }

    /// the rules of a fighter that takes another's power: the block, and the jump and label it uses
    fn copy_power(&mut self) -> Result<(), String> {
        if !self.o.fighter { return Err("takes a power, and is no fighter".into()) }
        self.blocks.insert("copy_power".into());
        self.blocks.insert("clip_label".into());
        self.stance_jump("")?;
        Ok(())
    }

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
        if let Some(r) = self.picture_call(f, args)? { return Ok(r) }
        let top = self.top;
        let r = self.call_written(f, args)?;
        // (a call whose row does nothing gives no value to use)
        if !top && r.0.trim().is_empty() {
            let m = match f { Expr::Member(_, m) => m.clone(), other => describe(other) };
            return Err(format!("uses what its {m} gives, which nothing here does"));
        }
        Ok(r)
    }

    fn call_written(&mut self, f: &Expr, args: &[Expr]) -> Result<(String, Ty), String> {
        // (a clip record's timeline, moved: `clipRun`'s)
        let clip_op = match f {
            Expr::Member(recv, m) if self.clip_rec_value(recv) => Some(((**recv).clone(), m.clone())),
            Expr::Name(m) if self.clip_rec_value(&Expr::This) && self.method(m).is_none() => Some((Expr::This, m.clone())),
            _ => None,
        };
        if let Some((recv, m)) = clip_op.filter(|(_, m)| matches!(m.as_str(), "gotoAndStop" | "gotoAndPlay" | "play" | "stop" | "nextFrame" | "prevFrame")) {
            let (r, _) = self.expr(&recv)?;
            self.util("clipRun")?;
            let a = match args.first() { Some(x) => Some(self.expr(x)?.0), None => None };
            return Ok((match (m.as_str(), a) {
                ("gotoAndStop", Some(a)) => format!("clipGo({r}, {a}, true)"),
                ("gotoAndPlay", Some(a)) => format!("clipGo({r}, {a}, false)"),
                ("nextFrame", _) => format!("clipStep({r}, 1)"),
                ("prevFrame", _) => format!("clipStep({r}, -1)"),
                ("play", _) => format!("clipPlay({r}, true)"),
                ("stop", _) => format!("clipPlay({r}, false)"),
                _ => return Err(format!("calls {m} on a clip with nothing to go to")),
            }, Ty::Any));
        }
        // (a call on one of the API's classes a record written out in place keeps: its record form)
        if let Expr::Member(h, m) = f {
            if let Some(r) = self.lit_rt_use(h, m, Some(args))? { return Ok(r) }
        }
        // another object's stat asked through a getter whose name its code builds from the object's
        // kind (`o["get" + kind + "Stat"](k)`): that object's stat, whatever its kind
        if let (Expr::Index(o, k), [_]) = (f, args) {
            if let Expr::Local(r) = &**k {
                let mut values: Vec<Expr> = Vec::new();
                walk(&self.body, &mut |st| if let Stmt::Assign(Place::Local(x), v) = st { if x == r { values.push(v.clone()); } });
                fn ends(e: &Expr) -> (Option<String>, Option<String>) {
                    match e {
                        Expr::Str(s) => (Some(s.clone()), Some(s.clone())),
                        Expr::Binary(BinOp::Add, a, b) => (ends(a).0, ends(b).1),
                        _ => (None, None),
                    }
                }
                let getter = !values.is_empty() && values.iter().all(|v| { let (a, b) = ends(v); a.is_some_and(|a| a.starts_with("get")) && b.is_some_and(|b| b.ends_with("Stat")) });
                if getter { return self.call_written(&Expr::Member(o.clone(), "getGameObjectStat".into()), args) }
            }
        }
        // a named clip of its own clip taken off it (`getMC().removeChild(getMC().ground)`): drawn
        // into its art here, with the rest
        if let (Expr::Member(c, m), [x]) = (f, args) {
            let own = |e: &Expr| matches!(e, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(o, n) if matches!(n.as_str(), "getMC" | "stanceClip" | "getStanceMC") && matches!(**o, Expr::This)));
            if m == "removeChild" && !self.o.fighter && own(c) && matches!(x, Expr::Member(o, _) if own(o)) {
                let Expr::Member(_, n) = x else { unreachable!() };
                self.gaps.insert(format!("takes its {n} clip off its clip, which this engine draws into its art"));
                return Ok((format!("// (not ported: it takes its {n} clip off; this engine draws it with the rest of its art)"), Ty::Any));
            }
        }
        // an object's clip put back in its parent at a place in the drawing order (`parent.addChildAt(o.getMC(), i)`):
        // the match's drawing order (`drawOrderParent`)
        if let (Expr::Member(p, m), [x, i]) = (f, args) {
            if matches!(m.as_str(), "addChildAt" | "setChildIndex") && matches!(&**p, Expr::Member(_, n) if n == "parent")
                && matches!(x, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(_, n) | Expr::Name(n) if matches!(n.as_str(), "getMC" | "stanceClip" | "getStanceMC"))) {
                let Expr::Call(g, _) = x else { unreachable!() };
                let who = match &**g { Expr::Member(o, _) => self.expr(o)?.0, _ => "self".to_string() };
                let (i, _) = self.expr(i)?;
                self.util("drawOrderParent")?;
                return Ok((format!("drawOrderSet({who}, {i})"), Ty::Any));
            }
        }
        // a static method of one of its package's classes, called by the class's name: written as a
        // function of its own (`Class_method`) once the rest is, the class's other static methods with it
        if let Expr::Member(c, m) = f {
            if let Expr::Name(cn) = &**c {
                let k = self.o.model.classes.values().find(|k| short(&k.name) == short(cn) && !self.o.runtime.provides(&k.name) && k.methods.iter().any(|x| x.is_static && &x.name == m)).cloned();
                if let Some(k) = k {
                    let id = format!("{}_{}", ident(short(&k.name)), ident(m));
                    for x in k.methods.iter().filter(|x| x.is_static) {
                        self.methods.insert(member_key(&k.name, &x.name), format!("{}_{}", ident(short(&k.name)), ident(&x.name)));
                        self.statics.insert((k.name.clone(), x.name.clone()));
                    }
                    let vs: Vec<String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
                    // (a parameter every call hands a clip of the planes: that clip, as its own methods' are)
                    if self.learning {
                        for (i, v) in vs.iter().enumerate() {
                            let had = self.part_params.get(&(id.clone(), i)).copied().unwrap_or(true);
                            self.part_params.insert((id.clone(), i), had && v.starts_with("planePart("));
                        }
                    }
                    return Ok((format!("{id}({})", vs.join(", ")), self.method_ty.get(&id).copied().unwrap_or(Ty::Any)));
                }
            }
        }
        // a call of a flattened field's class (first: nothing else reads it as a value)
        if let Expr::Member(recv, m) = f {
            if let Some((id, k)) = self.flat_of(recv) {
                let vs: Vec<String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
                if let Some(mid) = self.methods.get(&member_key(&k, m)).cloned() { self.learn_elems(&mid, args); }
                return self.flat_call(&id, &k, m, &vs);
            }
        }
        // (a flattened field's object asked whether it is gone, or ended, where its class has no
        // method of its own for it: whether it is made, and made no more)
        if let Expr::Member(recv, m) = f {
            if args.is_empty() && matches!(m.as_str(), "isDisposed" | "destroy") {
                if let Some((id, k)) = self.flat_of(recv) {
                    if !self.methods.contains_key(&member_key(&k, m)) && !id.contains('.') {
                        self.kept.insert(format!("{id}_isMade"), format!("var {id}_isMade = false;"));
                        return Ok((if m == "isDisposed" { format!("!{id}_isMade") } else { format!("{id}_isMade = false") }, if m == "isDisposed" { Ty::Bool } else { Ty::Any }));
                    }
                }
            }
        }
        let top = std::mem::replace(&mut self.top, false);
        if let Expr::Member(recv, m) = f {
            // its costumes' colours
            if matches!(m.as_str(), "paletteData" | "getPaletteSwapData") && args.is_empty() && self.o.palette.is_some() { self.expr(recv)?; return Ok((self.costume_palette()?, Ty::Rect)) }
            // AS3's conversions, as the decompile writes them on the clip (`self.uint(x)`)
            if matches!(**recv, Expr::This) && self.method(m).is_none() && matches!(m.as_str(), "uint" | "int" | "Number" | "String" | "Boolean") {
                return self.call_written(&Expr::Name(m.clone()), args);
            }
        }
        // a call on the record a field holds: the record's class's function, handed it
        if let Expr::Member(recv, m) = f {
            if let Expr::Member(t, fl) = &**recv {
                if matches!(**t, Expr::This) {
                    if let Some(k) = self.rec_fields.get(&member_key(&self.class, fl)).cloned() {
                        let id = self.methods.get(&member_key(&k, m)).cloned().ok_or_else(|| format!("calls {m} of a {}, which it has none of", short(&k)))?;
                        let (who, _) = self.expr(recv)?;
                        let mut vs: Vec<String> = vec![who.clone()];
                        for x in args { vs.push(self.expr(x)?.0); }
                        let ty = self.method_ty.get(&id).copied().unwrap_or(Ty::Any);
                        return Ok((format!("({who} == null ? null : {id}({}))", vs.join(", ")), ty));
                    }
                }
            }
        }
        // a call on a record in a list it keeps: the record's class's function, handed it; one that
        // writes the record works on a copy, written back as a new list (a rollback puts the old back)
        if let Expr::Member(recv, m) = f {
            if let Expr::Index(l, i) = &**recv {
                if let Expr::Member(t, fl) = &**l {
                    let owner = if matches!(**t, Expr::This) { Some(self.class.clone()) } else { None };
                    if let Some(k) = owner.and_then(|c| self.rec_lists.get(&member_key(&c, fl)).cloned()) {
                        let kid = ident(short(&k));
                        let id = self.methods.get(&member_key(&k, m)).cloned().ok_or_else(|| format!("calls {m} of a {}, which it has none of", short(&k)))?;
                        let (list, _) = self.expr(l)?;
                        let (ix, _) = self.expr(i)?;
                        let vs: Vec<String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
                        let rest = if vs.is_empty() { String::new() } else { format!(", {}", vs.join(", ")) };
                        let ty = self.method_ty.get(&id).copied().unwrap_or(Ty::Any);
                        if !record_writes(self.o.model, &k, m) { return Ok((format!("{id}({list}[{ix}]{rest})"), ty)) }
                        let (fid, _) = self.field(fl).ok_or("a list of no field")?;
                        let set = if self.plain.contains(&member_key(&self.class, fl)) { format!("{fid} = __l;") } else { format!("{fid}.set(__l);") };
                        return Ok((format!("{{ var __l = {list}.copy(); var __r = {kid}_copy(__l[{ix}]); {id}(__r{rest}); __l[{ix}] = __r; {set} }}"), Ty::Any));
                    }
                }
            }
        }
        // (what a list field holds, learned from what its code pushes onto it)
        if let Expr::Member(recv, m) = f {
            // (onto a list inside a list field: a list of lists of records, while every push is one)
            if m == "push" && self.learning && args.len() == 1 {
                if let Expr::Index(l, _) = &**recv {
                    if let Expr::Member(t, fl) = &**l {
                        if matches!(**t, Expr::This) {
                            let rec = matches!(&args[0], Expr::Object(_)) || matches!(&args[0], Expr::New(c, _) if matches!(&**c, Expr::Name(n) if matches!(short(n), "Point" | "Rectangle")))
                                || self.expr(&args[0]).is_ok_and(|(_, t)| t == Ty::Rect);
                            let k = member_key(&self.class, fl);
                            let had = self.nested_recs.get(&k).copied().unwrap_or(true);
                            self.nested_recs.insert(k, had && rec);
                        }
                    }
                }
            }
            if m == "push" && self.learning && args.len() == 1 {
                if let Expr::Local(r) = &**recv {
                    if let Ok((_, ty)) = self.expr(&args[0]) {
                        let ty = if matches!(&args[0], Expr::Object(_)) { Ty::Rect } else { ty };
                        let was = self.local_elem.get(r).copied();
                        self.local_elem.insert(*r, match was { None => ty, Some(w) => join(w, ty) });
                    }
                }
                if let Expr::Member(t, fl) = &**recv {
                    if matches!(**t, Expr::This) {
                        // (a clip of its planes pushed, or nothing: a list of them, while every one is)
                        if let Ok((w, _)) = self.expr(&args[0]) {
                            let part = w.starts_with("planePart(") || matches!(&args[0], Expr::Member(t2, g) if matches!(**t2, Expr::This) && self.part_fields.get(&member_key(&self.class, g)) == Some(&true));
                            let k = member_key(&self.class, fl);
                            if part || w == "null" {
                                if part { self.part_lists.entry(k).or_insert(true); }
                            } else {
                                self.part_lists.insert(k, false);
                            }
                        }
                        if let Ok((_, ty)) = self.expr(&args[0]) {
                            // (a record literal pushed: a list of records, whose fields its code reads plainly)
                            let ty = if matches!(&args[0], Expr::Object(_)) { Ty::Rect } else { ty };
                            // (a field of its own pushed: what learning has seen it hold, its type not yet set)
                            let ty = match (&args[0], ty) {
                                (Expr::Member(t2, g), Ty::Any) if matches!(**t2, Expr::This) => self.seen.get(&member_key(&self.class, g)).copied().unwrap_or(Ty::Any),
                                (_, t) => t,
                            };
                            let k = member_key(&self.class, fl);
                            let was = self.list_elem.get(&k).copied();
                            self.list_elem.insert(k, match was { None => ty, Some(w) => join(w, ty) });
                        }
                    }
                }
            }
        }
        // a class made by calling `new` on it (`Vector.new()`), and a particle's clip made from its class
        // held as a value (`arg0.new()`): a clip record
        if let Expr::Member(recv, m) = f {
            if m == "new" && !matches!(**recv, Expr::This) {
                // (a typed list, `Vector.<T>.new()`: a list)
                if matches!(&**recv, Expr::TypeApply(..)) && args.is_empty() { return Ok(("[]".into(), Ty::Object)) }
                if let Expr::Name(n) = &**recv { if self.o.runtime.native(n).is_some() || matches!(short(n), "MovieClip" | "Sprite") { return self.expr(&Expr::New(Box::new(Expr::Name(n.clone())), args.to_vec())) } }
                if args.is_empty() {
                    let (x, _) = self.expr(recv)?;
                    self.util("clipRecord")?;
                    return Ok((format!("clipRecord({x}.id, {x}.w, {x}.h)"), Ty::Rect));
                }
            }
            // a clip a clip record holds, by name
            if m == "getChildByName" && !matches!(**recv, Expr::This) && (self.holds_clip_record(recv) || self.expr(recv).is_ok_and(|(_, t)| t == Ty::Rect)) {
                let (c, _) = self.expr(recv)?;
                let (k, _) = self.expr(args.first().ok_or("names no clip")?)?;
                self.util("clipRecord")?;
                self.util("clipAdd")?;
                return Ok((format!("clipChild({c}, {k})"), Ty::Rect));
            }
            // a clip put in (or taken out of) a clip record its code holds
            if matches!(m.as_str(), "addChild" | "removeChild" | "addChildAt") && !matches!(**recv, Expr::This) && (self.holds_clip_record(recv) || self.expr(recv).is_ok_and(|(_, t)| t == Ty::Rect)) {
                let (c, _) = self.expr(recv)?;
                let (k, _) = self.expr(args.first().ok_or("adds no clip")?)?;
                self.util("clipRecord")?;
                self.util("clipAdd")?;
                return Ok((format!("{}({c}, {k})", if m == "removeChild" { "clipRemove" } else { "clipAdd" }), Ty::Rect));
            }
        }
        // a call on an element of a list of API records: its class's function, on a copy written
        // back; and a function an element's field holds (`timerList[i].data.fn()`), called
        if let Expr::Member(recv, m) = f {
            if let Expr::Index(l, i) = &**recv {
                if let Expr::Member(t, fl) = &**l {
                    if matches!(**t, Expr::This) && self.rt_list(fl).is_some() {
                        let k = self.rt_list(fl).unwrap_or_default();
                        self.rt_record(&k)?;
                        let (fid, _) = self.field(fl).ok_or("a list of no field")?;
                        let (ix, _) = self.expr(i)?;
                        let vs: Vec<String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
                        let rest = if vs.is_empty() { String::new() } else { format!(", {}", vs.join(", ")) };
                        return Ok((format!("{{ var __l = {fid}.get().copy(); var __r = {k}_copy(__l[{ix}]); {k}_{m}(__r{rest}); __l[{ix}] = __r; {fid}.set(__l); }}"), Ty::Any));
                    }
                }
            }
            if let Expr::Member(x, _) = &**recv {
                if matches!(&**x, Expr::Index(l, _) if matches!(&**l, Expr::Member(t, fl) if matches!(**t, Expr::This) && self.rt_list(fl).is_some())) {
                    let (who, _) = self.expr(recv)?;
                    let vs: Vec<String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
                    return Ok((format!("{who}.{m}({})", vs.join(", ")), Ty::Any));
                }
            }
        }
        // a call on a variable holding a record of a list: the same, the copy written back where the
        // record sits in the list now
        if let Expr::Member(recv, m) = f {
            if let Expr::Local(r) = &**recv {
                if let Some((k, fl)) = self.rec_locals.get(r).cloned() {
                    let kid = ident(short(&k));
                    let id = self.methods.get(&member_key(&k, m)).cloned().ok_or_else(|| format!("calls {m} of a {}, which it has none of", short(&k)))?;
                    let (x, _) = self.expr(recv)?;
                    let vs: Vec<String> = args.iter().map(|a| Ok(self.expr(a)?.0)).collect::<Result<_, String>>()?;
                    let rest = if vs.is_empty() { String::new() } else { format!(", {}", vs.join(", ")) };
                    let ty = self.method_ty.get(&id).copied().unwrap_or(Ty::Any);
                    if !record_writes(self.o.model, &k, m) { return Ok((format!("{id}({x}{rest})"), ty)) }
                    let (list, _) = self.expr(&Expr::Member(Box::new(Expr::This), fl.clone()))?;
                    let (fid, _) = self.field(&fl).ok_or("a list of no field")?;
                    let set = if self.plain.contains(&member_key(&self.class, &fl)) { format!("{fid} = __l;") } else { format!("{fid}.set(__l);") };
                    return Ok((format!("{{ var __l = {list}.copy(); var __r = {kid}_copy({x}); {id}(__r{rest}); var __i = __l.indexOf({x}); if (__i >= 0) __l[__i] = __r; {set} {x} = __r; }}"), Ty::Any));
                }
            }
        }
        // the source's autolink, where the conversion made it the engine's own for the clip: turning
        // it on or off sets the angle of the animation's hitboxes, toward the animation's own point
        if let Expr::Member(_, m) = f {
            if self.o.fighter && matches!(m.as_str(), "startAutolink" | "stopAutolink" | "setAutolinkPoint" | "setAutolinkRule") {
                if let Some((on, off)) = self.o.jumps.get(&self.class).and_then(|j| j.autolink.clone()) {
                    let code = match m.as_str() { "startAutolink" => on, "stopAutolink" => off, _ => String::new() };
                    return Ok((code.replace("{fm}", "self"), Ty::Any));
                }
            }
        }
        let Expr::Member(recv, m) = f else {
            // AS3's conversions, called as functions
            if let (Expr::Name(n), [x]) = (f, args) {
                match short(n) {
                    "Number" => { let (v, t) = self.expr(x)?; return Ok((if t == Ty::Num { v } else { format!("Std.parseFloat(Std.string({v}))") }, Ty::Num)) }
                    "int" | "uint" => { let (v, t) = self.expr(x)?; return Ok((match t { Ty::Num => format!("Std.int({v})"), Ty::Str => format!("Std.parseInt({v})"), _ => format!("Std.int(Std.parseFloat(Std.string({v})))") }, Ty::Num)) }
                    "String" => { let (v, _) = self.expr(x)?; return Ok((format!("Std.string({v})"), Ty::Str)) }
                    "Boolean" => return Ok((self.test(x)?, Ty::Bool)),
                    _ => {}
                }
            }
            // one of its own methods, called bare
            if let Expr::Name(n) = f {
                if let Some(id) = self.method(n) { return self.own_call(&id, args) }
                // (an effect it makes, called bare: made on itself, as one called on it is)
                if self.is_itself() && matches!(n.as_str(), "createVfx" | "createVfxOverlay") {
                    let saved = std::mem::replace(&mut self.effect_on, "self".into());
                    let r = if top { self.top = true; let r = self.effect(args); self.top = false; r } else { self.kept_effect(args) };
                    self.effect_on = saved;
                    return r;
                }
                // (a variable of its own holding the closure it calls)
                if self.captured.contains(&ident(n)) {
                    let vs: Vec<String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
                    return Ok((format!("({0} != null ? {0}({1}) : null)", ident(n), vs.join(", ")), Ty::Any));
                }
                if self.is_itself() { return self.api("self", n, args) }
                // (a clip its clip places, asked for by a name written out: that clip)
                if n == "getChildByName" { if let [Expr::Str(c)] = args { return self.expr(&Expr::Member(Box::new(Expr::This), c.clone())) } }
                return Err(format!("calls {n} on its clip"));
            }
            return Err(format!("calls {}", describe(f)));
        };
        // a date it keeps set to another hour (the engine's dates do not change: a new one, the same
        // but for that hour, kept in its place)
        if m == "setHours" && args.len() == 1 && matches!(&**recv, Expr::Member(t, _) if matches!(**t, Expr::This)) {
            let Expr::Member(_, f) = &**recv else { unreachable!() };
            let (id, _) = self.field(f).ok_or_else(|| format!("sets the hour of {f}, which is not its field"))?;
            let key = member_key(&self.class, f);
            let plain = self.plain.contains(&key);
            self.read.insert(key);
            let (h, _) = self.expr(&args[0])?;
            let d = if plain { id.clone() } else { format!("{id}.get()") };
            let made = format!("new Date({d}.getFullYear(), {d}.getMonth(), {d}.getDate(), {h}, {d}.getMinutes(), {d}.getSeconds())");
            return Ok((if plain { format!("if ({d} != null) {id} = {made}") } else { format!("if ({d} != null) {id}.set({made})") }, Ty::Any));
        }
        // one of its own methods
        if matches!(**recv, Expr::This) {
            if let Some(id) = self.method(m) { return self.own_call(&id, args) }
            // (a field of its own holding a function: that function called, where it holds one)
            if self.field(m).is_some_and(|(_, h)| matches!(h, Holds::Value(_))) {
                let (f, _) = self.expr(&Expr::Member(Box::new(Expr::This), m.clone()))?;
                let vs: Vec<String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
                return Ok((format!("({f} != null ? {f}({}) : null)", vs.join(", ")), Ty::Any));
            }
            // (a clip its clip places, asked for by a name written out: that clip, as its name reads it)
            if m == "getChildByName" && !self.is_itself() {
                if let [Expr::Str(n)] = args { return self.expr(&Expr::Member(Box::new(Expr::This), n.clone())) }
            }
            // (the object's own class calls the API on itself)
            if !self.is_itself() { return Err(format!("calls {m} on its clip")) }
        }
        // the API's own questions about the object
        if let Expr::Name(c) = &**recv {
            match (short(c), m.as_str(), args) {
                // (the fighter a clip of a fighter-made object's belongs to: the fighter that made it)
                (_, "getCharacter" | "getPlayer", [Expr::This]) if c.contains("PeptideAPI") && !self.o.fighter && !self.o.stage => return Ok(("self.getRootOwner()".into(), Ty::Object)),
                (_, "getProjectile" | "getItem" | "getEnemy" | "getCharacter" | "getPlayer", [Expr::This]) if c.contains("PeptideAPI") => return Ok(("self".into(), Ty::Object)),
                // a player by number (from 1), as the match's fighters stand in player order
                (_, "getPlayer", [n]) if c.contains("PeptideAPI") => {
                    let (n, _) = self.expr(n)?;
                    return Ok((format!("match.getCharacters()[Std.int({n}) - 1]"), Ty::Object));
                }
                (_, "isReady", []) if c.contains("PeptideAPI") => return Ok(("true".into(), Ty::Bool)),
                // the end of every frame of the game, heard by a fighter: the end of its own update
                (_, "addEventListener" | "removeEventListener", [ev, h, ..]) if c.contains("PeptideAPI") && self.o.fighter
                    && super::events::named(self.o.runtime, ev).as_deref() == Some("gameTickEnd") => {
                    let h = self.handler(h)?;
                    self.blocks.insert("tick_end".into());
                    return Ok((format!("{}({h})", if m == "addEventListener" { "listenTickEnd" } else { "unlistenTickEnd" }), Ty::Any));
                }
                // the ground on a line, and an enemy made: a fighter's blocks
                (_, "hitTestGround" | "hitTestGroundBetweenPoints", _) if c.contains("PeptideAPI") && self.o.fighter => {
                    if m != "hitTestGround" { self.blocks.insert("ground_test".into()); }
                    let vs: Result<Vec<String>, String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                    let vs = vs?;
                    let opt = |i: usize| vs.get(i).cloned().unwrap_or_else(|| "null".into());
                    if m == "hitTestGround" { self.util("groundAt")?; }
                    return Ok((if m == "hitTestGround" { format!("groundAt({0}, {1}, {2})", opt(0), opt(1), opt(2)) }
                        else { format!("(({0}) == null || ({1}) == null ? null : groundBetween(({0}).x, ({0}).y, ({1}).x, ({1}).y, {2}))", opt(0), opt(1), opt(2)) }, Ty::Object));
                }
                // a clip recoloured by its code, the clip being the one the object draws with (its
                // own clip class's `this`): the object itself
                (_, "replacePalette", [Expr::This, pal, ..]) if c.contains("PeptideUtils") => {
                    let (pal, _) = self.expr(pal)?;
                    self.util("paletteOn")?;
                    return Ok((format!("paletteOn(self, {pal})"), Ty::Any));
                }
                // whether its final smash cutscene still plays
                (_, "isFSCutscenePlaying", []) if c.contains("PeptideAPI") => {
                    self.util("fsCutscene")?;
                    return Ok(("fsCutscenePlaying()".into(), Ty::Bool));
                }
                // a list the game hands out (`getPokemonStatsList`): the classes its package
                // registers in it, by name, the list asked for (else the call's own default)
                (_, call, rest) if c.contains("PeptideAPI") && crate::api_mappings::REGISTERED_LISTS.iter().any(|(k, ..)| *k == call) => {
                    let (_, package, prop) = crate::api_mappings::REGISTERED_LISTS.iter().find(|(k, ..)| *k == call).copied().unwrap_or_default();
                    let lists = crate::shared_packages::registered_lists(package, prop);
                    if lists.is_empty() { return Err(format!("reads the list the {package} package registers, which no package here registers")) }
                    let which = match rest.first() { Some(x) => self.expr(x)?.0, None => "null".into() };
                    let arr = |v: &Vec<String>| format!("[{}]", v.iter().map(|n| quote(n)).collect::<Vec<_>>().join(", "));
                    let first = lists.keys().next().cloned().unwrap_or_default();
                    let mut code = arr(&lists[&first]);
                    for (k, v) in lists.iter().filter(|(k, _)| **k != first) {
                        code = format!("({which} == {} ? {} : {code})", quote(k), arr(v));
                    }
                    // (asked for none, the source's default: its common list)
                    if let Some(common) = lists.get("common") { code = format!("({which} == null ? {} : {code})", arr(common)); }
                    return Ok((code, Ty::Object));
                }
                // one of those classes made by the name the list gave: the object that class was
                // written as, fired here as an enemy is (`spawnPokemon`)
                (_, "spawnPokemon", [k, ..]) if c.contains("PeptideAPI") => {
                    let (k, _) = self.expr(k)?;
                    let names: Vec<String> = crate::api_mappings::REGISTERED_LISTS.iter()
                        .flat_map(|(_, package, prop)| crate::shared_packages::registered_lists(package, prop).into_values().flatten())
                        .collect();
                    let mut cases = Vec::new();
                    for n in &names {
                        if let Ok(f) = self.fire(n, "self") { cases.push(format!("\t\tcase {}: o = {f}(0, 0);", quote(n))); }
                    }
                    if cases.is_empty() { return Err("spawns a class its package lists, none of which was written here".into()) }
                    let name = "spawnListed".to_string();
                    if !self.helpers.contains_key(&name) {
                        self.helpers.insert(name.clone(), format!("// one of the classes a list the game hands out holds, made by its name: the object it was\n// written as, made as an enemy is\nfunction {name}(k) {{\n\tvar o = null;\n\tswitch (k) {{\n{}\n\t}}\n\tif (o != null) {{ o.exports.__isEnemy = true; o.exports.__np = true; }}\n\treturn o;\n}}", cases.join("\n")));
                    }
                    return Ok((format!("{name}({k})"), Ty::Object));
                }
                (_, "makeEnemy", [k]) if c.contains("PeptideAPI") && self.o.fighter => {
                    let n = match k { Expr::Str(n) => n.clone(), Expr::Name(n) => short(n).to_string(), _ => return Err("makes an enemy named as it runs".into()) };
                    let f = self.fire(&n, "self")?;
                    let name = format!("makeEnemy_{}", ident(&n));
                    if !self.helpers.contains_key(&name) { self.helpers.insert(name.clone(), helper(self.o.runtime, "helpers", "make_enemy", &name, &[("FIRE", f)], &[])?); }
                    return Ok((format!("{name}()"), Ty::Object));
                }
                // an object a stage's code makes by its class (a stage object's code too)
                (_, "makeEnemy" | "spawnPlatform" | "placePickup", _) if c.contains("PeptideAPI") && !self.o.fighter && matches!(args.first(), Some(Expr::Name(_) | Expr::Str(_))) => {
                    return self.api("self", &format!("PeptideAPI.{m}"), args);
                }
                // an effect made by the API: on the object whose code makes it, as its own
                (_, "createVfx" | "createVfxOverlay", _) if c.contains("PeptideAPI") => {
                    let saved = std::mem::replace(&mut self.effect_on, "self".into());
                    let r = if top { self.top = true; let r = self.effect(args); self.top = false; r } else { self.kept_effect(args) };
                    self.effect_on = saved;
                    return r;
                }
                // the API's own calls with a row (`PeptideAPI.print`)
                // a projectile of a class its package converted, made by its class: the content that class
                // was converted to, fired by the object given (else the one asking)
                (_, "spawnProjectile", [k, rest @ ..]) if c.contains("PeptideAPI") && matches!(k, Expr::Name(_) | Expr::Str(_)) => {
                    let (Expr::Name(n) | Expr::Str(n)) = k else { unreachable!() };
                    let id = crate::projectile_gen::projectile_content_id(short(n));
                    let owner = match rest.first() { Some(o) => self.expr(o)?.0, None => "self".into() };
                    return Ok((format!("{{ var firer = {owner}; var made = firer == null ? null : firer.getResource().getContent({}); made == null ? null : match.createProjectile(made, firer); }}", quote(&id)), Ty::Object));
                }
                // an item of its package placed by a fighter by class: made as one of its own items
                // (lying free where its code puts it, as the source places one; its content is the one
                // made by its class, under the class's name)
                (_, "placePickup", [Expr::Name(k) | Expr::Str(k)]) if c.contains("PeptideAPI") && self.o.fighter => {
                    let id = crate::projectile_gen::projectile_content_id(short(k));
                    return Ok((format!("{{ var made = self.getResource().getContent({}); var it = made == null ? null : match.createCustomGameObject(made, self); if (it != null) {{ it.exports.__isItem = true; it.exports.__np = true; it.exports.__itemName = {}; }} it; }}", quote(&id), quote(short(k))), Ty::Object));
                }
                // a listener on the game for the end of each frame: run at the end of its own, while it listens
                (_, "addEventListener" | "removeEventListener", [ev, h, ..]) if c.contains("PeptideAPI")
                    && super::events::named(self.o.runtime, ev).is_some_and(|n| n == "gameTickEnd") => {
                    let h = self.handler(h)?;
                    let flag = format!("{h}__tickEnd");
                    self.kept.insert(flag.clone(), format!("// whether {h} listens for the end of each frame now\nvar {flag} = self.makeObject(false);"));
                    self.game_ticks.insert(h.clone(), flag.clone());
                    return Ok((format!("{flag}.set({})", m == "addEventListener"), Ty::Any));
                }
                // a clip of its library made by name that is one of its effects: drawn as that
                // effect where its code puts it (`screenClip`)
                (_, "clipByLinkage", [Expr::Str(n)]) if c.contains("PeptideAPI") && self.screen_clip(n).is_some() => {
                    return Ok((self.screen_clip(n).unwrap_or_default(), Ty::Rect));
                }
                // a fighter's own clip made by its linkage (`getCharacterStat("linkage_id")`): a copy of its body
                (_, "clipByLinkage", [Expr::Call(g, a)]) if c.contains("PeptideAPI") && matches!(a.as_slice(), [Expr::Str(k)] if k == "linkage_id")
                    && matches!(&**g, Expr::Member(_, m) if m == "getCharacterStat") => {
                    let Expr::Member(f, _) = &**g else { unreachable!() };
                    let (who, _) = self.expr(f)?;
                    self.util("bodyCopy")?;
                    return Ok((format!("bodyCopy({who})"), Ty::Rect));
                }
                // a listener on the game for an item made: told by each item as it starts (`itemCreated`)
                (_, "addEventListener" | "removeEventListener", [ev, h, ..]) if c.contains("PeptideAPI")
                    && super::events::named(self.o.runtime, ev).is_some_and(|n| n == "gameItemCreated") => {
                    let h = self.handler(h)?;
                    self.util("itemCreated")?;
                    return Ok((format!("itemCreatedListen({h}, {})", m == "addEventListener"), Ty::Any));
                }
                // a listener on the game for one this engine's match raises: the match's own
                (_, "addEventListener" | "removeEventListener", [ev, h, ..]) if c.contains("PeptideAPI")
                    && super::events::named(self.o.runtime, ev).is_some_and(|n| super::events::table().matched.contains_key(&n)) => {
                    let n = super::events::named(self.o.runtime, ev).unwrap_or_default();
                    let e = super::events::table().matched[&n].clone();
                    let h = self.handler(h)?;
                    return Ok((if m == "addEventListener" { format!("match.addEventListener({e}, {h}, {{ persistent: true }})") } else { format!("match.removeEventListener({e}, {h})") }, Ty::Any));
                }
                // a listener on the game for an event it never raises: never told
                (_, "addEventListener" | "removeEventListener", [ev, ..]) if c.contains("PeptideAPI")
                    && super::events::named(self.o.runtime, ev).is_some_and(|n| !super::events::table().game.contains(&n)) => {
                    let n = super::events::named(self.o.runtime, ev).unwrap_or_default();
                    // (one the source's game never raises either is never told there: as it is here)
                    if !super::events::table().game_raised.contains(&n) {
                        return Ok((format!("// (the source listens to the game for {n} here, which its game never raises)"), Ty::Any));
                    }
                    self.gaps.insert(format!("listens to the game for {n}, which the game here never raises"));
                    return Ok((format!("// (the source listens to the game for {n} here, which the game here never raises)"), Ty::Any));
                }
                (_, _, _) if c.contains("PeptideAPI") => {
                    // (one asked by a name written out: its row by name, where it has one)
                    let named = matches!(args.first(), Some(Expr::Str(_))).then(|| calls().rows.get(&format!("PeptideAPI.{m}#name")).cloned()).flatten();
                    let row = named.or_else(|| calls().rows.get(&format!("PeptideAPI.{m}")).cloned()).ok_or_else(|| format!("calls PeptideAPI.{m}, which has no row in native_calls.jsonc"))?;
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
        if !self.o.stage {
            if let Expr::Call(g, a) = &**recv {
                if a.is_empty() && (matches!(&**g, Expr::Name(s) if matches!(s.as_str(), "stanceClip" | "getStanceMC"))
                    || matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "stanceClip" | "getStanceMC") && self.expr(o).is_ok_and(|(w, _)| w == "self"))) {
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
                // (a clip record of its own taking itself off what it was put in: taken off there, `clipRemove`)
                if matches!(x, Expr::This) && self.record.as_ref().is_some_and(|k| self.clip_recs.contains(k)) {
                    self.util("clipAdd")?;
                    return Ok(("if (me.parentOf != null) clipRemove(me.parentOf(), me)".into(), Ty::Any));
                }
                if matches!(x, Expr::This) || self.is_clip(x) { return Ok(("// (not ported: a part of its clip takes itself off; this engine draws it with the rest)".into(), Ty::Any)) }
                let (v, _) = self.expr(x)?;
                return Ok((format!("if ({v} != null && !{v}.isDisposed()) {v}.kill()"), Ty::Any));
            }
        }
        // a clip it holds (an effect) sent to a frame, and held or played on: a clip inside one is
        // drawn into its art here, so nothing sends that anywhere
        // a clip it holds stepped a frame on or back, and held there
        if matches!(m.as_str(), "nextFrame" | "prevFrame") && args.is_empty() && !matches!(**recv, Expr::This) && !self.is_clip(recv) {
            let (who, _) = self.expr(recv)?;
            self.effect_jump()?;
            return Ok((format!("effectStep({who}, {})", if m == "nextFrame" { 1 } else { -1 }), Ty::Any));
        }
        if matches!(m.as_str(), "gotoAndStop" | "gotoAndPlay" | "stop" | "play") && !matches!(**recv, Expr::This) && !self.is_clip(recv) {
            // (a named clip of the stage's planes: what the stage publishes for it)
            if self.expr(recv).is_ok_and(|(w, _)| w.starts_with("planePart(")) {
                let (who, _) = self.expr(recv)?;
                return self.api(&who, m, args);
            }
            // (one of its parts: its timeline, `projPart`'s record)
            if let Expr::Member(o, n) = &**recv {
                if matches!(**o, Expr::This) {
                    if let Some(r) = self.own_part(n) {
                        self.util("clipRun")?;
                        let hold = matches!(m.as_str(), "gotoAndStop" | "stop");
                        return Ok((match (m.as_str(), args.first()) {
                            ("gotoAndStop" | "gotoAndPlay", Some(a)) => { let (a, _) = self.expr(a)?; format!("clipGo({r}, {a}, {hold})") }
                            _ => format!("clipPlay({r}, {})", !hold),
                        }, Ty::Any));
                    }
                }
            }
            // (a clip its clip places, which its code turns: drawn into its art, so held nowhere)
            if let Expr::Member(o, n) = &**recv {
                if matches!(**o, Expr::This) && self.placed_child(n).is_some() {
                    self.gaps.insert(format!("steps its {n} clip, whose art is drawn into the object's"));
                    return Ok((format!("// (its {n} clip's art is drawn into the object's: its {m} turns nothing here)"), Ty::Any));
                }
            }
            if let Expr::Member(o, n) = &**recv {
                // (inside an effect of the power it holds: the clip the one it copied draws apart, `powerInner`)
                if !matches!(**o, Expr::This) && self.field(n).is_none() && !self.is_inner(n) && self.o.fighter && takes_power(self.o.model, self.o.runtime) {
                    self.copy_power()?;
                    let (holder, _) = self.expr(o)?;
                    let hold = matches!(m.as_str(), "gotoAndStop" | "stop");
                    let to = match (m.as_str(), args.first()) { ("gotoAndStop" | "gotoAndPlay", Some(f)) => self.expr(f)?.0, _ => "null".into() };
                    return Ok((format!("powerInner({holder}, {}, {to}, {hold})", quote(n)), Ty::Any));
                }
                // (a clip a record of its keeps, a part of the planes or an effect: sent the way it answers)
                if !matches!(**o, Expr::This) && self.field(n).is_none() && !self.is_inner(n) && self.expr(o).is_ok_and(|(_, t)| t == Ty::Rect) {
                    let (who, _) = self.expr(recv)?;
                    self.util("clipJump")?;
                    let hold = matches!(m.as_str(), "gotoAndStop" | "stop");
                    let to = match (m.as_str(), args.first()) { ("gotoAndStop" | "gotoAndPlay", Some(f)) => self.expr(f)?.0, _ => "null".into() };
                    return Ok((format!("clipJump({who}, {to}, {hold})"), Ty::Any));
                }
                // (a named clip of another object's clip: the part it draws apart and publishes, `partOf`)
                if let Expr::Call(g, a) = &**o {
                    if a.is_empty() && matches!(&**g, Expr::Member(x, s) if matches!(s.as_str(), "getStanceMC" | "stanceClip" | "getMC") && self.expr(x).is_ok_and(|(w, _)| w != "self")) {
                        let Expr::Member(x, _) = &**g else { unreachable!() };
                        let (who, _) = self.expr(x)?;
                        self.util("partOf")?;
                        let r = format!("partOf({who}, {})", quote(n));
                        let hold = matches!(m.as_str(), "gotoAndStop" | "stop");
                        return Ok((match (m.as_str(), args.first()) {
                            ("gotoAndStop" | "gotoAndPlay", Some(t)) => { let (t, _) = self.expr(t)?; format!("{{ var pt = {r}; if (pt != null) clipGo(pt, {t}, {hold}); }}") }
                            _ => format!("{{ var pt = {r}; if (pt != null) clipPlay(pt, {}); }}", !hold),
                        }, Ty::Any));
                    }
                }
                // (a clip nested in an object's own clip, whose art is drawn whole into each of the
                // object's frames: its playhead is drawn nowhere)
                if !matches!(**o, Expr::This) && (self.field(n).is_none() || !self.writes_field(n)) && !self.is_inner(n) && !self.o.stage && self.nested_in_drawn(o) {
                    self.gaps.insert(format!("steps its {n} clip, whose art is drawn into the object's"));
                    return Ok((format!("// (its {n} clip's art is drawn into the object's: its {m} turns nothing here)"), Ty::Any));
                }
                if !matches!(**o, Expr::This) && self.field(n).is_none() && !self.is_inner(n) {
                    if std::env::var("PEPTIDE_NATIVE_WHY").is_ok() { eprintln!("[no plain form] jump on {:?} = {:?} / recv {:?}", o, self.expr(o), self.expr(recv)); }
                    return Err(format!("sends {n}, a clip inside one it holds, to a frame"))
                }
            }
            let (who, _) = self.expr(recv)?;
            let hold = matches!(m.as_str(), "gotoAndStop" | "stop");
            return match args {
                [] => Ok((format!("if ({who} != null) {who}.{}()", if hold { "pause" } else { "resume" }), Ty::Any)),
                [to] => {
                    let (to, ty) = self.expr(to)?;
                    self.effect_jump()?;
                    let by = if ty == Ty::Str || to.starts_with('"') { "Label" } else { "Frame" };
                    Ok((format!("effect{by}({who}, {to}, {hold})"), Ty::Any))
                }
                _ => Err(format!("calls {m} with more than a frame")),
            };
        }
        let (who, who_ty) = self.expr(recv)?;
        // a slice of a text: this engine's texts have no `slice` of their own (`textSlice`)
        if m == "slice" && matches!(who_ty, Ty::Str | Ty::Any) && (1..=2).contains(&args.len()) {
            if !self.helpers.contains_key("textSlice") { self.helpers.insert("textSlice".into(), helper(self.o.runtime, "utils", "textSlice", "textSlice", &[], &[])?); }
            let vs: Vec<String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
            // (a text cut as Flash cuts one; a value of no known kind, whichever it is)
            return Ok(if who_ty == Ty::Str { (format!("textSlice({who}, {})", vs.join(", ")), Ty::Str) } else { (format!("anySlice({who}, {})", vs.join(", ")), Ty::Any) });
        }
        // a point of the midground taken to the screen's space or back: the stage's own space here,
        // where its regions answer point tests too, so the point itself
        if matches!(m.as_str(), "localToGlobal" | "globalToLocal") && args.len() == 1
            && matches!(&**recv, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(_, n) | Expr::Name(n) if n == "getMidground")) {
            let (p, _) = self.expr(&args[0])?;
            // (one a clip record of the screen gave, its HUD's: where it falls in the camera's view)
            let from_screen = m == "globalToLocal" && matches!(&args[0], Expr::Call(h, _) if matches!(&**h, Expr::Member(x, k) if k == "localToGlobal" && self.expr(x).is_ok_and(|(_, t)| t == Ty::Rect)));
            if from_screen { self.util("cameraClip")?; self.util("screenToStage")?; return Ok((format!("screenToStage({p})"), Ty::Rect)) }
            return Ok((p, Ty::Rect));
        }
        // a text's first match of a text put in place of (a pattern is the engine's own: below)
        if m == "replace" && args.len() == 2 && matches!(args[0], Expr::Str(_)) && !matches!(**recv, Expr::This) {
            let (t, _) = self.expr(recv)?;
            let (a0, _) = self.expr(&args[0])?;
            let (b, _) = self.expr(&args[1])?;
            self.util("replaceFirst")?;
            return Ok((format!("replaceFirst({t}, {a0}, {b})"), Ty::Str));
        }
        // a clip the clip its animation shows places, by its place
        if m == "getChildAt" && args.len() == 1 && self.o.fighter && self.own_stance_value(recv) {
            let f = self.stance_kids();
            let (i, _) = self.expr(&args[0])?;
            return Ok((format!("{f}()[Std.int({i})]"), Ty::Rect));
        }
        // a function run on each element of a list (`forEach`): handed the element, its place and the list
        if m == "forEach" && args.len() == 1 && !matches!(**recv, Expr::This) {
            let (l, _) = self.expr(recv)?;
            let (f, _) = self.expr(&args[0])?;
            return Ok((format!("{{ var eachOf = {l}; var eachFn = {f}; if (eachOf != null && eachFn != null) {{ var eachAt = 0; for (eachIt in eachOf) {{ eachFn(eachIt, eachAt, eachOf); eachAt++; }} }} }}"), Ty::Any));
        }
        // a number written in a base (`toString(16)`)
        if m == "toString" && args.len() == 1 {
            if !self.helpers.contains_key("numberIn") { self.helpers.insert("numberIn".into(), helper(self.o.runtime, "utils", "numberIn", "numberIn", &[], &[])?); }
            let (b, _) = self.expr(&args[0])?;
            return Ok((format!("numberIn({who}, {b})"), Ty::Str));
        }
        // a call a record answers
        // (a field of one that no record answers the call of: an object the record holds)
        // (or a record one of the API's classes it keeps under its name holds: a timer's `data`)
        let flat_held = matches!(&**recv, Expr::Member(o, _) if matches!(&**o, Expr::Member(t, f) if matches!(**t, Expr::This) && self.flat.contains_key(&member_key(&self.class, f))) || self.lit_rt_held(o).is_some());
        if who_ty == Ty::Rect && (!matches!(&**recv, Expr::Member(..)) || flat_held || calls().record_calls.contains_key(m.as_str())) {
            // (one with no row: the function the record holds under that name, called)
            let Some(row) = calls().record_calls.get(m.as_str()).cloned() else {
                let vs: Vec<String> = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
                let f = format!("{}.{}", paren(&who), ident(m));
                return Ok((format!("({f} != null ? {f}({}) : null)", vs.join(", ")), Ty::Any));
            };
            // (a copy of one is a record too, as is a point it answers)
            if m == "localToGlobal" { self.util("clipRecord")?; self.util("clipAdd")?; }
            return self.fill(&Row { code: row, returns: match m.as_str() { "clone" | "localToGlobal" => Ty::Rect, "intersects" => Ty::Bool, _ => Ty::Any }, helper: None, gap: None }, &who, args, m);
        }
        // one of its own declaration methods: the record it declares
        if who == "self" && args.is_empty() {
            if let Some(r) = self.declared_record(m) { return Ok((r, Ty::Rect)) }
        }
        // a method of the object's own class, called through the object from another of its classes
        if who == "self" && !self.is_itself() {
            if let Some(id) = self.itself_method(m) { return self.own_call(&id, args) }
        }
        // (its own, or one it puts on another object: the effect by its name, on that object)
        if matches!(m.as_str(), "createVfx" | "createVfxOverlay") {
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
        let mut named = match args.first() { Some(Expr::Str(_)) => None, Some(e) => Some(self.expr(e)?.0), None => return Err("makes an effect of no name".into()) };
        let name = match args.first() { Some(Expr::Str(n)) => n.clone(), _ => String::new() };
        let mut found = if named.is_some() { Some((String::new(), None)) } else { (self.o.effect)(&name) };
        // (one a fighter has no art for, where it takes powers: the effect of the power it holds,
        // made by the one it copied, `powerEffect`)
        let from_power = found.is_none() && self.o.fighter && takes_power(self.o.model, self.o.runtime);
        if from_power { named = Some(quote(&name)); found = Some((String::new(), None)); }
        let Some((stats, gone)) = found else {
            // (one with no art in this conversion shows nothing on either path)
            //
            if self.top { return Ok((format!("// (the source shows its effect {name} here, which has no art in this conversion)"), Ty::Any)) }
            // (as a value: the effect it would keep is none)
            return Ok(("null".into(), Ty::Object));
        };
        let mut opts: BTreeMap<String, &Expr> = BTreeMap::new();
        match args.get(1) {
            None | Some(Expr::Null) | Some(Expr::Undefined) => {}
            Some(Expr::Object(kv)) => for (k, v) in kv {
                let Expr::Str(k) = k else { return Err("makes an effect with options named as it runs".into()) };
                // (whether it turns with its maker, `flip`: the engine's `flipWith`)
                if k == "flip" { opts.insert(k.clone(), v); continue; }
                // (an option with no form here is left out and said; one that is neither an option the
                // source's effects take nor a property of a clip does nothing there, and nothing here)
                if !matches!(k.as_str(), "x" | "y" | "scaleX" | "scaleY" | "rotation" | "behind" | "parentLock" | "syncHitStun" | "flipOnX" | "loop" | "resize" | "absolute") {
                    if matches!(k.as_str(), "alpha" | "visible" | "width" | "height" | "rotationX" | "rotationY" | "rotationZ" | "scaleZ" | "z" | "blendMode" | "filters" | "transform" | "name" | "cacheAsBitmap" | "mask") {
                        self.gaps.insert(format!("names an effect option {k}, which is not ported"));
                    }
                    continue;
                }
                opts.insert(k.clone(), v);
            },
            _ => return Err("makes an effect with options worked out as it runs".into()),
        }
        let flag = |k: &str| -> Result<bool, String> { match opts.get(k) { None => Ok(false), Some(Expr::Bool(b)) => Ok(*b), _ => Err(format!("makes an effect with {k} worked out as it runs")) } };
        let locked = flag("parentLock")?;
        // (behind it, or by a test worked out as it runs)
        let behind = match opts.get("behind") {
            None | Some(Expr::Bool(false)) => None,
            Some(Expr::Bool(true)) => Some("VfxLayer.CHARACTERS_BACK".to_string()),
            Some(e) => { let t = self.test(e)?; Some(format!("({t} ? VfxLayer.CHARACTERS_BACK : VfxLayer.CHARACTERS_FRONT)")) }
        };
        // (one sized with its maker as the maker changes size: `resize`)
        let resized = matches!(opts.get("resize"), Some(Expr::Bool(true)));
        // (one placed where the stage is, not off its maker: `absolute`)
        let absolute = matches!(opts.get("absolute"), Some(Expr::Bool(true)));
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
        if let Some(l) = &behind { st.push(format!("layer: {l}")); }
        if resized && !locked { st.push("resizeWith: true".into()); }
        if looped { st.retain(|x| !x.starts_with("timeout:")); st.push("loop: true".into()); }
        // (one that does not turn with its maker, or turns as a test worked out as it runs says)
        let unturned = match opts.get("flip") {
            None | Some(Expr::Bool(true)) => None,
            Some(Expr::Bool(false)) => Some("false".to_string()),
            Some(e) => Some(self.test(e)?),
        };
        if let (Some(t), false) = (&unturned, locked) { st.push(format!("flipWith: {t}")); }
        let mut after = Vec::new();
        if locked {
            // riding the object, which mirrors the offset with it
            st.push(format!("x: {}.flipX({}), y: {}, relativeWith: true, resizeWith: true, flipWith: true", self.effect_on, off(&ox), off(&oy)));
        } else if absolute {
            if ox.is_some() { after.push(format!("setX({})", off(&ox))); }
            if oy.is_some() { after.push(format!("setY({})", off(&oy))); }
        } else {
            if ox.is_some() { after.push(format!("setX({}.getX() + {})", self.effect_on, off(&ox))); }
            if oy.is_some() { after.push(format!("setY({}.getY() + {})", self.effect_on, off(&oy))); }
        }
        if let Some(t) = num(self, "rotation")? { after.push(format!("setRotation({t})")); }
        let on = self.effect_on.clone();
        // (one named as it runs: its art looked up by the name, `namedEffect`)
        let make = if let (true, Some(n)) = (from_power, &named) {
            self.copy_power()?;
            let extra: Vec<String> = st.into_iter().filter(|x| !x.is_empty()).collect();
            format!("powerEffect({n}, {on}, {{ {} }})", extra.join(", "))
        } else if let Some(n) = &named {
            self.effect_art()?;
            let extra: Vec<String> = st.into_iter().filter(|x| !x.is_empty()).collect();
            if extra.is_empty() { format!("namedEffect({n}, {on}, {{}})") } else { format!("namedEffect({n}, {on}, {{ {} }})", extra.join(", ")) }
        } else if self.o.effect_code.contains_key(&name) {
            // (kept: its clip's own frames turn it)
            self.effect_code_helper()?;
            let made = format!("match.createVfx(new VfxStats({{ {} }}), {on})", st.join(", "));
            let made = if self.o.effect_inners.contains_key(&name) { self.inner_clips()?; format!("withInners({made}, {})", quote(&name)) } else { made };
            format!("keepCoded({made}, {})", quote(&name))
        } else if self.o.effect_inners.contains_key(&name) {
            // (with the clips it places that its code turns)
            self.inner_clips()?;
            format!("withInners(match.createVfx(new VfxStats({{ {} }}), {on}), {})", st.join(", "), quote(&name))
        } else { format!("match.createVfx(new VfxStats({{ {} }}), {on})", st.join(", ")) };
        if after.is_empty() { return Ok((make, Ty::Object)) }
        self.effects += 1;
        let v = if self.effects == 1 { "effect".to_string() } else { format!("effect{}", self.effects) };
        let set: Vec<String> = after.iter().map(|a| format!("{v}.{a};")).collect();
        Ok((format!("var {v} = {make};\nif ({v} != null) {{ {} }}", set.join(" ")), Ty::Object))
    }

    /// a clip of its library that is one of its effects, made as a clip record drawn as that effect
    /// where its code puts it (`screenClip`): its animations in its timeline's order
    fn screen_clip(&mut self, n: &str) -> Option<String> {
        let mut ps: Vec<(String, u32, u32)> = self.o.effect_pieces.iter().filter(|p| p.0 == n).map(|p| (p.1.clone(), p.2, p.3)).collect();
        if ps.is_empty() { return None }
        ps.sort_by_key(|p| p.1);
        self.util("screenClip").ok()?;
        let rows: Vec<String> = ps.iter().map(|(a, f, l)| format!("[{}, {}, {}]", quote(a), f.max(&1), l)).collect();
        // (what its clip's own frames do, `effect_code`: run as its timeline enters each, the
        // engine's frame of the animation as the source's frame of the timeline)
        let mut cases = Vec::new();
        for (anim, frame, does) in self.o.effect_code.get(n).cloned().unwrap_or_default() {
            let Some((_, from, _)) = ps.iter().find(|p| p.0 == anim) else { continue };
            cases.push(format!("case {}: {}", (*from).max(1) + (frame.max(1) - 1) / 2, does.replace("(e,", "(r,").replace("(e)", "(r)")));
        }
        if !cases.is_empty() { self.effect_code_helper().ok()?; }
        let frames = if cases.is_empty() { String::new() } else { format!(", function(r, f) {{ switch (f) {{ {} }} }}", cases.join(" ")) };
        Some(format!("screenClip({}, [{}]{frames})", quote(n), rows.join(", ")))
    }

    /// the helpers that send an effect it holds to a label or a frame
    fn effect_jump(&mut self) -> Result<(), String> {
        if self.helpers.contains_key("effectLabel") { return Ok(()) }
        // (by the animation it plays: the effects whose animation of that name start alike)
        let mut by_anim: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        let mut layouts: BTreeMap<&str, Vec<&(String, String, u32, u32)>> = BTreeMap::new();
        for p in &self.o.effect_pieces { layouts.entry(p.0.as_str()).or_default().push(p); }
        for ps in layouts.values() {
            let names: Vec<String> = ps.iter().map(|p| quote(&p.1)).collect();
            let froms: Vec<String> = ps.iter().map(|p| p.2.to_string()).collect();
            let lens: Vec<String> = ps.iter().map(|p| p.3.to_string()).collect();
            let call = format!("goToFrameOf(effect, f, [{}], [{}], [{}]);", names.join(", "), froms.join(", "), lens.join(", "));
            for p in ps { by_anim.entry(p.1.as_str()).or_default().insert(call.clone()); }
        }
        let mut cases = Vec::new();
        for (a, calls) in by_anim {
            // (two effects that lay out the same animation name differently are told apart by none)
            if calls.len() > 1 { continue }
            cases.push(format!("case {}: {}", quote(a), calls.into_iter().next().unwrap_or_default()));
        }
        self.helpers.insert("effectLabel".into(), helper(self.o.runtime, "helpers", "effect_jump", "effectLabel", &[], &cases)?);
        Ok(())
    }

    /// the helper that keeps the effects whose clips' own frames turn them, doing what those do
    fn effect_code_helper(&mut self) -> Result<(), String> {
        if self.helpers.contains_key("keepCoded") { return Ok(()) }
        let mut cases = Vec::new();
        for (name, rows) in &self.o.effect_code {
            for (anim, frame, does) in rows { cases.push(format!("case {}: {does}", quote(&format!("{name}/{anim}/{frame}")))); }
        }
        self.helpers.insert("keepCoded".into(), helper(self.o.runtime, "helpers", "effect_code", "keepCoded", &[], &cases)?);
        self.every_frame_extra.insert("runEffectCode();".into());
        Ok(())
    }

    /// the helpers that make and keep the clips its effects place that its code turns
    fn inner_clips(&mut self) -> Result<(), String> {
        if self.helpers.contains_key("withInners") { return Ok(()) }
        let mut of = Vec::new();
        let mut place = Vec::new();
        for (effect, clips) in &self.o.effect_inners {
            let list: Vec<String> = clips.iter().map(|c| format!("[{}, {}]", quote(&c.name), quote(&c.art))).collect();
            of.push(format!("case {}: return [{}];", quote(effect), list.join(", ")));
            for c in clips {
                for (anim, m) in &c.placed {
                    place.push(format!("case {}: return [{}, {}, {}, {}, {}];", quote(&format!("{effect}/{}/{anim}", c.name)),
                        number(m.tx), number(m.ty), number(m.rotation), number(m.sx), number(m.sy)));
                }
            }
        }
        let main = helper(self.o.runtime, "helpers", "inner_clips", "withInners", &[], &of)?;
        let at = helper(self.o.runtime, "helpers", "inner_place", "innerPlace", &[], &place)?;
        self.helpers.insert("withInners".into(), format!("{main}\n\n{at}"));
        self.every_frame_extra.insert("followInners();".into());
        Ok(())
    }

    /// the art of each effect it can make, by name, for one its code names as it runs
    fn effect_art(&mut self) -> Result<(), String> {
        if self.helpers.contains_key("namedEffect") { return Ok(()) }
        let mut cases = Vec::new();
        for n in self.o.effect_names.clone() {
            let Some((stats, gone)) = (self.o.effect)(&n) else { continue };
            let timeout = gone.map(|g| format!(", timeout: {}", 2 * (g.max(1) - 1))).unwrap_or_default();
            cases.push(format!("case {}: return new VfxStats({{ {stats}{timeout} }});", quote(&n)));
        }
        let mut made = if self.o.effect_inners.is_empty() { "match.createVfx(stats, on)".to_string() } else { self.inner_clips()?; "withInners(match.createVfx(stats, on), name)".to_string() };
        if !self.o.effect_code.is_empty() { self.effect_code_helper()?; made = format!("keepCoded({made}, name)"); }
        self.helpers.insert("namedEffect".into(), helper(self.o.runtime, "helpers", "named_effect", "namedEffect", &[("MADE", made)], &cases)?);
        Ok(())
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
        // (a fighter's rules carry it; any other object carries it as a helper of its own)
        if self.o.fighter { self.blocks.insert("place_effect".into()); }
        else if !self.helpers.contains_key("placeEffect") { self.helpers.insert("placeEffect".into(), helper(self.o.runtime, "fighter_rules", "place_effect", "placeEffect", &[], &[])?); }
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

    /// a field of the stage named for a child its clip places, which no code of the class writes:
    /// the child itself (the field Flash declares and fills for a named instance)
    fn timeline_child(&self, f: &str) -> bool {
        if !self.o.stage { return false }
        let Some(c) = self.o.model.classes.get(&self.class) else { return false };
        if !c.fields.iter().any(|x| x.name == f && !x.is_static) { return false }
        let mut written = false;
        for m in c.methods.iter().map(|m| &m.function).chain(std::iter::once(&c.constructor)) {
            walk(&m.body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, g) | Place::Name(g), _) = st { if g == f { written = true; } });
        }
        if written { return false }
        let root = if self.is_itself() { Some(self.o.root) } else { c.symbol };
        crate::emit::clip_tree(self.o.model, root).into_iter().filter_map(|s| self.o.model.sprites.get(&s))
            .any(|sp| sp.frames.iter().flatten().any(|p| p.name.as_deref() == Some(f)))
    }

    /// a class whose objects are pictures: Flash's `BitmapData`, or one of its library's that extends it
    fn picture_class(&self, n: &str) -> bool {
        short(n) == "BitmapData" || (short(n) != "Bitmap" && drawn_type(self.o.model, n) && self.o.model.classes.values().any(|k| short(&k.name) == short(n) && short(&k.super_name) == "BitmapData"))
    }

    /// what a stage's code draws (`runtime/native/utils.hx` `pixels`): a picture's drawing calls, and
    /// the name of the picture a placed one shows
    fn picture_call(&mut self, f: &Expr, args: &[Expr]) -> Result<Option<(String, Ty)>, String> {
        if !self.o.stage { return Ok(None) }
        // (a clip of its library made by name: `libClips`, which its stage carries when its code makes any)
        if let (Expr::Member(_, m) | Expr::Name(m), [Expr::Str(n)]) = (f, args) {
            if matches!(m.as_str(), "getMCByLinkageName" | "clipByLinkage") { return Ok(Some((format!("libClip({})", quote(n)), Ty::Rect))) }
        }
        if let Expr::Name(n) = f {
            if n == "getQualifiedClassName" {
                if let [x @ Expr::Member(_, b)] = args { if b == "bitmapData" { let (v, _) = self.expr(x)?; return Ok(Some((format!("({v} == null ? null : {v}.cls)"), Ty::Str))) } }
            }
            return Ok(None);
        }
        let Expr::Member(recv, m) = f else { return Ok(None) };
        if !matches!(m.as_str(), "copyPixels" | "fillRect" | "setPixel" | "setPixel32" | "colorTransform" | "lock" | "unlock" | "draw") { return Ok(None) }
        // (a picture's data, a picture its code holds, or a field it declares as one)
        let declared = |w: &Self, e: &Expr| matches!(e, Expr::Member(t, f) if matches!(**t, Expr::This)
            && w.o.model.classes.get(&w.class).is_some_and(|c| c.fields.iter().any(|x| &x.name == f && x.type_name.as_deref().is_some_and(|t| drawn_type(w.o.model, t)))));
        let picture = matches!(&**recv, Expr::Member(_, b) if b == "bitmapData") || declared(self, recv)
            || self.expr(recv).is_ok_and(|(w, t)| t == Ty::Rect && !w.starts_with("planePart("));
        if !picture { return Ok(None) }
        self.util("pixels")?;
        let (c, _) = self.expr(recv)?;
        let a = |w: &mut Self, i: usize| -> Result<String, String> { Ok(w.expr(args.get(i).ok_or("a drawing call short of its arguments")?)?.0) };
        let out = match m.as_str() {
            "copyPixels" => format!("pxCopy({c}, {}, {}, {})", a(self, 0)?, a(self, 1)?, a(self, 2)?),
            "fillRect" => { let r = a(self, 0)?; format!("{{ var __r = {r}; pxFill({c}, __r.x, __r.y, __r.width, __r.height, {}); }}", a(self, 1)?) }
            "setPixel" | "setPixel32" => format!("pxFill({c}, {}, {}, 1, 1, {})", a(self, 0)?, a(self, 1)?, a(self, 2)?),
            "colorTransform" => format!("pxTint({c}, {})", a(self, 1)?),
            // (a clip drawn into it: a gap, this engine drawing no clip into a picture)
            "draw" => { self.gaps.insert("draws a clip into a picture, which this engine does not".into()); "// (a clip drawn into a picture: not drawn here)".into() }
            _ => String::new(),
        };
        Ok(Some((out, Ty::Any)))
    }

    /// one of its fields that only ever holds a clip of the stage's planes (learned, every write a part)
    fn part_field(&self, e: &Expr) -> bool {
        let Expr::Member(t, f) = e else { return false };
        matches!(**t, Expr::This) && self.learned && self.part_fields.get(&member_key(&self.class, f)) == Some(&true)
    }

    /// (a list of records handed on, a field's or a variable's: its elements are records there too)
    fn learn_elems(&mut self, id: &str, args: &[Expr]) {
        if !self.learning { return }
        for (i, a) in args.iter().enumerate() {
            let elem = match a {
                Expr::Member(o, fl) if matches!(**o, Expr::This) => self.list_elem.get(&member_key(&self.class, fl)).copied().unwrap_or(Ty::Any),
                Expr::Local(r) => self.local_elem.get(r).copied().unwrap_or(Ty::Any),
                // (an empty list fits any)
                Expr::Array(v) if v.is_empty() => continue,
                _ => Ty::Any,
            };
            let k = (id.to_string(), i);
            let was = self.param_elem.get(&k).copied();
            self.param_elem.insert(k, match was { None => elem, Some(w) if w == elem => w, _ => Ty::Any });
        }
    }

    fn own_call(&mut self, id: &str, args: &[Expr]) -> Result<(String, Ty), String> {
        let typed: Vec<(String, Ty)> = args.iter().map(|x| self.expr(x)).collect::<Result<_, String>>()?;
        if self.learning {
            for (i, (_, t)) in typed.iter().enumerate() {
                let k = (id.to_string(), i);
                let was = self.param_ty.get(&k).copied();
                self.param_ty.insert(k.clone(), match was { None => *t, Some(w) => join(w, *t) });
            }
            self.learn_elems(id, args);
            for (i, (w, _)) in typed.iter().enumerate() {
                let k = (id.to_string(), i);
                let part = w.starts_with("planePart(");
                let had = self.part_params.get(&k).copied().unwrap_or(true);
                self.part_params.insert(k, had && part);
            }
            let was = self.max_args.get(id).copied().unwrap_or(0);
            self.max_args.insert(id.to_string(), was.max(args.len()));
        }
        let mut vs: Vec<String> = typed.into_iter().map(|(v, _)| v).collect();
        // (a record's own method: handed the record)
        if let Some(k) = &self.record { if id.starts_with(&format!("{}_", ident(short(k)))) { vs.insert(0, "me".into()); } }
        Ok((format!("{id}({})", vs.join(", ")), self.method_ty.get(id).copied().unwrap_or(Ty::Any)))
    }

    /// an API call on an engine object (`who`: `self`, or the variable holding another)
    fn api(&mut self, who: &str, m: &str, args: &[Expr]) -> Result<(String, Ty), String> {
        // the camera's clip: its view, as the source reads it
        if who == "match.getCamera()" && m == "getMC" && args.is_empty() { self.util("cameraClip")?; return Ok(("cameraClip()".into(), Ty::Rect)) }
        // an item put back to lying idle: its handler's own state, as the source's handler has it
        if self.o.item && who == "self" && m == "toIdle" && args.is_empty() { return Ok(("itemTo(0)".into(), Ty::Any)) }
        // (an item's attack its own code starts anew: its boxes hit whatever state it is in)
        if self.o.item && who == "self" && m == "refreshAttackID" && args.is_empty() { return Ok(("armHits()".into(), Ty::Any)) }
        // (an item's box stats its own code writes: its attack set up, so its boxes hit too)
        if self.o.item && who == "self" && m == "updateHitboxStats" && !self.arming {
            self.arming = true;
            let r = self.api(who, m, args);
            self.arming = false;
            let (t, ty) = r?;
            return Ok((if t.starts_with("//") { t } else { format!("{t};\narmAll()") }, ty));
        }
        let mine = who == "self";
        // a stage's planes: named values, which nothing here drives yet (a jump, a field or a child of
        // one is said in its gaps)
        if who.starts_with("stagePlane(") { return Err(format!("drives its {} plane, which no plain form drives yet", who.trim_start_matches("stagePlane(").trim_end_matches(')'))) }
        // an object made by its class: the content the class was converted to, made on the stage
        // (an enemy a fighter's package makes by class: the content that class shipped as, fired by
        // the fighter it belongs to)
        if m == "PeptideAPI.makeEnemy" && !self.o.stage && self.o.made_objects.is_empty() {
            if let Some(Expr::Name(c) | Expr::Str(c)) = args.first() {
                if self.o.model.classes.values().any(|k| short(&k.name) == short(c)) {
                    let id = crate::projectile_gen::projectile_content_id(short(c));
                    return Ok((format!("{{ var firer = self.getRootOwner() != null ? self.getRootOwner() : self; var made = firer.getResource().getContent({}); made == null ? null : match.createProjectile(made, firer); }}", quote(&id)), Ty::Object));
                }
            }
        }
        // (an item another fighter-side object makes by its class, a pokemon's egg: the content
        // that class shipped as, a custom game object of whoever owns the maker)
        if m == "PeptideAPI.placePickup" && !self.o.stage && self.o.made_objects.is_empty() {
            if let Some(Expr::Name(c) | Expr::Str(c)) = args.first() {
                if self.o.model.classes.values().any(|k| short(&k.name) == short(c)) {
                    let id = crate::projectile_gen::projectile_content_id(short(c));
                    return Ok((format!("{{ var holder = self.getRootOwner() != null ? self.getRootOwner() : self; var made = holder.getResource().getContent({}); made == null ? null : match.createCustomGameObject(made, holder); }}", quote(&id)), Ty::Object));
                }
            }
        }
        if matches!(m, "PeptideAPI.makeEnemy" | "PeptideAPI.spawnPlatform" | "PeptideAPI.placePickup") {
            if let Some(Expr::Name(c) | Expr::Str(c)) = args.first() {
                let id = self.o.made_objects.get(short(c)).cloned().ok_or_else(|| format!("makes an object of {c}, a class this package does not convert"))?;
                let made = format!("match.createCustomGameObject(self.getResource().getContent({}), null)", quote(&id));
                // (counted as made, where the stage's script keeps that list for the verify gates)
                if self.o.stage {
                    let f = self.util("madeObject")?;
                    return Ok((format!("{f}({}, {made})", quote(&id)), Ty::Object));
                }
                return Ok((made, Ty::Object));
            }
        }
        // a named clip of the stage's planes: what the stage publishes for it (`planePart`)
        if who.starts_with("planePart(") {
            let a = |w: &mut Self, i: usize| -> Result<String, String> { Ok(w.expr(args.get(i).ok_or("names no frame")?)?.0) };
            return match m {
                "gotoAndPlay" => Ok((format!("if ({who} != null) {who}.playLabel({}, false)", a(self, 0)?), Ty::Any)),
                "gotoAndStop" => Ok((format!("if ({who} != null) {who}.playLabel({}, true)", a(self, 0)?), Ty::Any)),
                "play" => Ok((format!("if ({who} != null) {who}.hold(false)"), Ty::Any)),
                "stop" => Ok((format!("if ({who} != null) {who}.hold(true)"), Ty::Any)),
                "setFallthrough" => Ok((format!("if ({who} != null) {who}.setFallthrough({})", a(self, 0)?), Ty::Any)),
                // (the order a plane draws its clips in: this engine draws each piece on its own layer)
                "swapChildren" | "swapChildrenAt" | "setChildIndex" => Ok(("// (not ported: a clip moved in its plane's drawing order; this engine draws each piece on its own layer)".into(), Ty::Any)),
                // (a clip taken out of its plane: one that draws nothing, a region its code reads and
                // takes off, is gone with nothing to show)
                // (a clip its code made from the library taken out, `libClips`; any other is a clip with
                // nothing drawn for it here)
                // (a point on the screen, in the plane's space: the midground's is the stage's)
                "globalToLocal" => { self.util("cameraClip")?; self.util("screenToStage")?; Ok((format!("screenToStage({})", a(self, 0)?), Ty::Rect)) }
                "removeChild" => Ok((format!("if ({who} != null) {who}.removeLib({})", a(self, 0)?), Ty::Any)),
                // (every one taken out: the library clips its code put in; what the tree placed draws on)
                "removeChildren" => Ok((format!("if ({who} != null) {who}.removeLibs()"), Ty::Any)),
                "addChild" => { let x = a(self, 0)?; Ok((format!("({who} == null ? {x} : {who}.addLib({x}))"), Ty::Rect)) }
                // (a clip of it by name: the part under that name)
                "getChildByName" => { self.util("planePart")?; Ok((format!("planePart({})", a(self, 0)?), Ty::Object)) }
                // (a child of it by place: a picture placed there, `pixels`)
                "getChildAt" => { self.util("pixels")?; Ok((format!("({who} == null ? null : {who}.childAt({}))", a(self, 0)?), Ty::Rect)) }
                _ => Err(format!("calls {m} on a clip of its planes, which no plain form answers yet")),
            };
        }
        // (a plane is the stage clip's child of its name: the midground's is its terrain)
        let plane = match m { "getBackground" => Some("background"), "getForeground" => Some("foreground"), "getMidground" => Some("terrain"), "getShadowMask" | "getShadowMaskMC" => Some("shadowMask"), _ => None };
        if let Some(p) = plane.filter(|_| args.is_empty() && ((self.o.stage && mine) || who == "stage")) { self.util("planePart")?; return Ok((format!("planePart({})", quote(p)), Ty::Object)) }
        if args.is_empty() && m == "getCameraBackgrounds" && ((self.o.stage && mine) || who == "stage") { self.util("planePart")?; self.util("cameraBgName")?; self.util("cameraPlanes")?; return Ok(("cameraPlanes()".into(), Ty::Object)) }
        // its costumes' colours
        if matches!(m, "paletteData" | "getPaletteSwapData") && args.is_empty() && self.o.palette.is_some() { return Ok((self.costume_palette()?, Ty::Rect)) }
        if mine && self.o.fighter {
            if let Some(r) = self.fighter_api(m, args) { return r }
        }
        match (m, args) {
            ("addEventListener" | "removeEventListener", [ev, handler, ..]) => return self.listener(who, m, ev, handler),
            ("addTimer", [frames, repeat, f, rest @ ..]) if mine => return self.timer(frames, repeat, f, rest),
            // (on another object: that object's timer, running its handler a source frame each two of
            // this engine's, as the timers of its own)
            ("addTimer", [frames, repeat, f, ..]) if !mine && !who.starts_with("planePart(") => {
                let (n, _) = self.expr(frames)?;
                let (t, _) = self.expr(repeat)?;
                let h = match f { Expr::Member(x, g) if matches!(**x, Expr::This) && self.record.is_some() && self.method(g).is_some() => self.expr(f)?.0, _ => self.handler(f)? };
                self.util("otherTimer")?;
                return Ok((format!("otherTimer({who}, Std.int(2 * Math.max(1, {})), ({t} <= 0 ? -1 : {t}), {h})", paren(&n)), Ty::Num));
            }
            // (one of another object's, taken off by the function it runs: as `otherTimer` kept it)
            ("removeTimer", [f]) if !mine && !who.starts_with("planePart(") => {
                let h = match f { Expr::Member(x, g) if matches!(**x, Expr::This) && self.record.is_some() && self.method(g).is_some() => self.expr(f)?.0, _ => self.handler(f)? };
                self.util("otherTimer")?;
                return Ok((format!("otherUntimer({who}, {h})"), Ty::Any));
            }
            // (fired through its owner's call: from the owner, the same way, as the source's runs it)
            // (one made again from a record its stats were exported to)
            // (one by a name worked out as it runs: each name its code writes that it declares a
            // projectile under, fired by that name)
            ("createProjectile", [name, rest @ ..]) if mine && self.o.fighter && !matches!(name, Expr::Str(n) if (self.o.fired)(n).is_some()) && self.expr(name).is_ok_and(|(_, t)| matches!(t, Ty::Str | Ty::Any)) => {
                let named_only = self.expr(name).is_ok_and(|(_, t)| t == Ty::Str);
                let mut names: BTreeSet<String> = BTreeSet::new();
                for c in self.o.itself.into_iter().chain(self.o.classes.iter().copied()) {
                    for b in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
                        walk_exprs(b, &mut |e| if let Expr::Str(n) = e { names.insert(n.clone()); });
                    }
                }
                let names: Vec<String> = names.into_iter().filter(|n| (self.o.fired)(n).is_some()).collect();
                if names.is_empty() { return Err("fires a projectile by a name worked out as it runs, none of whose names it declares".into()) }
                let (n, _) = self.expr(name)?;
                let vs: Vec<String> = rest.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<_, String>>()?;
                let mut cases = Vec::new();
                for x in &names { let f = self.fire(x, "self")?; cases.push(format!("case {}: {f}({})", quote(x), vs.join(", "))); }
                let by_name = format!("(switch ({n}) {{ {}; default: null; }})", cases.join("; "));
                if named_only { return Ok((by_name, Ty::Object)) }
                // (a value of no known kind: a name, else the record of one exported)
                if !self.helpers.contains_key("fireStats") { self.helpers.insert("fireStats".into(), helper(self.o.runtime, "utils", "fireStats", "fireStats", &[], &[])?); }
                let args = std::iter::once(n.clone()).chain(vs.into_iter().take(2)).collect::<Vec<_>>().join(", ");
                return Ok((format!("(Std.isOfType({n}, String) ? {by_name} : fireStats({args}))"), Ty::Object));
            }
            ("createProjectile", [rec, rest @ ..]) if mine && self.o.fighter && !matches!(rec, Expr::Str(_)) => {
                if !self.helpers.contains_key("fireStats") { self.helpers.insert("fireStats".into(), helper(self.o.runtime, "utils", "fireStats", "fireStats", &[], &[])?); }
                let (r, _) = self.expr(rec)?;
                let vs: Result<Vec<String>, String> = rest.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                let vs = vs?;
                let args = std::iter::once(r).chain(vs.into_iter().take(2)).collect::<Vec<_>>().join(", ");
                return Ok((format!("fireStats({args})"), Ty::Object));
            }
            // (its owner firing one like itself, from the stats it exports: its own declared name)
            ("createProjectile", [Expr::Call(g, a), rest @ ..]) if !self.o.fighter && !self.o.content.is_empty() && (self.owners.contains(who) || self.field_of_owner(who)) && a.is_empty()
                && matches!(&**g, Expr::Member(o, n) if n == "exportStats" && self.expr(o).is_ok_and(|(w, _)| w == "self")) => {
                let name = self.o.content.clone();
                let f = self.fire(&name, who)?;
                let vs: Result<Vec<String>, String> = rest.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                return Ok((format!("{f}({})", vs?.join(", ")), Ty::Object));
            }
            ("createProjectile", [Expr::Str(name), rest @ ..]) if mine || self.owners.contains(who) || self.field_of_owner(who) => {
                let f = self.fire(name, if mine { "self" } else { who })?;
                let vs: Result<Vec<String>, String> = rest.iter().map(|x| Ok(self.expr(x)?.0)).collect();
                return Ok((format!("{f}({})", vs?.join(", ")), Ty::Object));
            }
            // an attack's stats written on a projectile: none is a projectile's here but the rate it
            // hits again at
            ("updateAnimationStats", [Expr::Object(pairs)]) if mine => {
                if pairs.iter().any(|(k, _)| !matches!(k, Expr::Str(_))) { return Err("writes an attack stat by a name worked out as it runs".into()) }
                // (the rate it hits again at: `runtime/native/refresh.hx`)
                let Some((_, v)) = pairs.iter().find(|(k, _)| matches!(k, Expr::Str(k) if k == "refreshRate")) else { return Ok((String::new(), Ty::Any)) };
                self.blocks.insert("refresh".into());
                let (v, _) = self.expr(v)?;
                return Ok((format!("setRefresh({v})"), Ty::Any));
            }
            // the state it is in, as the source numbers it: the source's state whose engine states it is
            // in (the one naming fewest first, so the narrower wins)
            ("getState", []) if self.o.fighter || !mine => {
                if !self.helpers.contains_key("sourceStateOf") {
                    let mut rows: Vec<(usize, String, String)> = Vec::new();
                    for (name, st) in states() {
                        let Some(num) = self.o.runtime.constants.get(&format!("PeptideCState.{name}")) else { continue };
                        if st.is.is_empty() { continue }
                        let tests: Vec<String> = st.is.iter().map(|t| if t.starts_with("CStateGroup.") { format!("o.inStateGroup({t})") } else { format!("o.inState({t})") }).collect();
                        rows.push((st.is.len(), num.clone(), tests.join(" || ")));
                    }
                    rows.sort();
                    let body: Vec<String> = rows.iter().map(|(_, n, t)| format!("\tif ({t}) return {n};")).collect();
                    self.helpers.insert("sourceStateOf".into(), format!("// the state an object is in, as the source numbers it (`runtime/states.jsonc`)\nfunction sourceStateOf(o) {{\n\tif (o == null || o.inState == null) return -1;\n{}\n\treturn -1;\n}}", body.join("\n")));
                }
                return Ok((format!("sourceStateOf({who})"), Ty::Num));
            }
            // a fighter's state: a projectile is in none of them
            ("inState", [_]) if mine && !self.o.fighter && !self.o.item => return Ok(("false".into(), Ty::Bool)),
            // another fighter's stats its code writes: kept where that fighter reads its own back, and
            // the engine's own where it keeps the stat (in this engine's units)
            ("updateCharacterStats", [Expr::Object(pairs)]) if !mine => {
                let st = crate::mappings::character_stats();
                let mut out = Vec::new();
                for (k, v) in pairs {
                    let Expr::Str(k) = k else { return Err("writes a stat by a name worked out as it runs".into()) };
                    let (val, ty) = self.expr(v)?;
                    out.push(format!("if ({who} != null && {who}.exports != null && {who}.exports.__source != null) {who}.exports.__source.__stats.set({}, {val})", quote(k)));
                    if let Some((code, h)) = st.status_writes.get(k) { self.util(h)?; out.push(code.replace("{who}", who).replace("{v}", &paren(&val))); continue }
                    let name = st.fm_runtime_read_names.get(k).cloned().unwrap_or_else(|| k.clone());
                    if !st.fm_runtime_stats.contains(&name) || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { continue }
                    let value = if matches!(ty, Ty::Bool | Ty::Str) { val.clone() } else {
                        let unit = st.runtime_read_units.iter().find(|(_, l)| l.contains(&name)).map(|(u, _)| u.as_str());
                        let f = match unit { Some("size") => self.o.size, Some("accel") => self.o.accel, Some("speed" | "launch") => self.o.speed, _ => 1.0 };
                        fold(&format!("{} * {}", paren(&val), number(f)))
                    };
                    out.push(format!("if ({who} != null) {who}.updateCharacterStats({{ {name}: {value} }})"));
                }
                return Ok((out.join(";\n"), Ty::Any));
            }
            // another fighter's attack turned off or on by the source's name: the animations that name
            // is on this engine (`mappings/character/animations.jsonc`), and the actions they start
            ("allowAttack" | "setAttackEnabled", [on, Expr::Str(label), ..]) if !mine => {
                let on = self.test(on)?;
                let anim = crate::mappings::character_animations().ssf2_to_fm.get(label).cloned().unwrap_or_else(|| label.clone());
                self.util("foeAllowAttack")?;
                return Ok((format!("foeAllowAttack({who}, {on}, [{}])", quote(&anim)), Ty::Any));
            }
            ("setState", [st]) if !mine => {
                let name = self.state_name(st)?;
                // (one this engine's fighters have none of: nothing)
                let Some(to) = states().get(&name).and_then(|s| s.enter.clone()) else { return Ok((format!("// (not ported: the source puts a fighter in its {name} state here, which this engine's fighters have none of)"), Ty::Any)) };
                return Ok((format!("{who}.toState({to})"), Ty::Any));
            }
            // another projectile's own stats written: the engine's own stat on it, and through the
            // setter it publishes for one this engine keeps none of (`exports.__stat`)
            ("updateProjectileStats", [Expr::Object(pairs)]) if !mine => {
                let mut lines = Vec::new();
                for (k, v) in pairs {
                    let Expr::Str(k) = k else { return Err("writes a stat by a name worked out as it runs".into()) };
                    let (val, _) = self.expr(v)?;
                    let id = ident(k);
                    lines.push(match stat_row(k) {
                        Some(StatRow::Engine(t)) => format!("{};", crate::dyn_re(r"\bself\.").replace_all(&self.stat_code(&t, &val), format!("{who}.").as_str())),
                        Some(StatRow::Note(t)) => t,
                        _ => format!("if ({who}.exports.__{id} != null) {who}.exports.__{id}.set({val});"),
                    });
                }
                let body = lines.iter().map(|l| format!("\t{l}")).collect::<Vec<_>>().join("\n");
                return Ok((format!("if ({who} != null) {{\n{body}\n}}"), Ty::Any));
            }
            // an item's state, by the source's number for it, as its handler keeps it
            ("inState", [Expr::Member(c, n)]) if mine && self.o.item && matches!(&**c, Expr::Name(c) if short(c) == "PeptideIState") => {
                let v = self.o.runtime.constants.get(&format!("PeptideIState.{n}")).cloned().ok_or_else(|| format!("tests an item state {n} the API has no number for"))?;
                return Ok((format!("inItemState({v})"), Ty::Bool));
            }
            // another object's item state: as its handler keeps it, where it is an item
            ("inState", [Expr::Member(c, n)]) if !mine && matches!(&**c, Expr::Name(c) if short(c) == "PeptideIState") => {
                let v = self.o.runtime.constants.get(&format!("PeptideIState.{n}")).cloned().ok_or_else(|| format!("tests an item state {n} the API has no number for"))?;
                return Ok((format!("({who}.exports.__inItemState != null && {who}.exports.__inItemState({v}))"), Ty::Bool));
            }
            // its own stats written by its code: the engine's stat for each it keeps
            ("updateItemStats", [Expr::Object(pairs)]) if mine && self.o.item => {
                let (mut set, mut kept) = (Vec::new(), Vec::new());
                for (k, v) in pairs {
                    let Expr::Str(k) = k else { return Err("writes an item stat by a name worked out as it runs".into()) };
                    if self.own_stats.contains(k) {
                        let id = self.own_stat(k);
                        let (v, _) = self.expr(v)?;
                        kept.push(format!("{id}.set({v});"));
                        continue;
                    }
                    let Some(t) = calls().item_stats.get(k).cloned() else { continue };
                    let (v, _) = self.expr(v)?;
                    // (one the item handler keeps itself is a statement of its own, `;` ended)
                    if t.ends_with(';') { kept.push(t.replace("{v}", &v)); } else { set.push(self.stat_code(&t, &v)); }
                }
                if !set.is_empty() { kept.insert(0, format!("self.updateGameObjectStats({{ {} }});", set.join(", "))); }
                if kept.is_empty() { return Ok(("// (its code writes item stats this engine keeps none of)".into(), Ty::Any)) }
                return Ok((kept.join("\n").trim_end_matches(';').to_string(), Ty::Any));
            }
            // its holder told to throw it: the throw its handler makes
            ("toToss", []) if !mine && self.o.item => return Ok(("tossByHolder()".into(), Ty::Any)),
            // an enemy's or a target's state: gone or not
            ("inState", [Expr::Member(c, n)]) if matches!(&**c, Expr::Name(c) if matches!(short(c), "PeptideEState" | "PeptideTState")) => {
                return Ok((if n == "DEAD" { format!("({who} == null || {who}.isDisposed())") } else { format!("({who} != null && !{who}.isDisposed())") }, Ty::Bool));
            }
            // a projectile's state, by the source's name for it
            ("inState", [Expr::Member(c, n)]) if matches!(&**c, Expr::Name(c) if short(c) == "PeptidePState") => {
                let t = calls().projectile_states.get(n).ok_or_else(|| format!("tests a projectile's state {n}, which has no row in native_calls.jsonc"))?;
                return Ok((t.replace("{self}", who), Ty::Bool));
            }
            // the rate another object's attack hits again at: through the setter it publishes
            ("updateAnimationStats" | "updateAttackStats", [Expr::Object(pairs)]) if !mine && self.o.fighter
                && pairs.len() == 1 && matches!(&pairs[0].0, Expr::Str(k) if k == "refreshRate") => {
                let (v, _) = self.expr(&pairs[0].1)?;
                return Ok((format!("if ({who} != null && {who}.exports.__refresh != null) {who}.exports.__refresh({v})"), Ty::Any));
            }
            // attack stats written on another object, that this engine keeps none of: nothing here
            ("updateAnimationStats" | "updateAttackStats", [Expr::Object(pairs)]) if !mine && self.o.fighter
                && pairs.iter().all(|(k, _)| matches!(k, Expr::Str(k) if !super::attack_stats::table().maps(k) && k != "refreshRate")) => {
                return Ok(("// (not ported: attack stats this engine keeps none of, written on another object)".into(), Ty::Any));
            }
            ("inState", [st]) => {
                let name = self.state_name(st)?;
                let tests: Vec<String> = states().get(&name).into_iter().flat_map(|s| s.is.iter()).map(|t| if t.starts_with("CStateGroup.") {
                    format!("{who}.inStateGroup({t})") } else { format!("{who}.inState({t})") }).collect();
                return Ok((if tests.is_empty() { "false".into() } else { tests.join(" || ") }, Ty::Bool));
            }
            ("removeTimer", [f]) if mine => {
                let id = self.timer_of(f)?;
                // (and forgets the id: the engine gives it out again, to another of its timers)
                return Ok((format!("self.removeTimer({id}.get());\n{id}.set(-1)"), Ty::Any));
            }
            // an item's jump to a label: in the clip its state shows (`item.hx` `playStance`)
            ("playFrame", [label]) if mine && self.o.item => { let (l, _) = self.expr(label)?; return Ok((format!("playStance({l})"), Ty::Any)) }
            ("playFrame", [Expr::Str(label)]) if mine => return Ok((self.jump(label)?.trim_end_matches(';').to_string(), Ty::Any)),
            // (to a frame worked out as it runs: by label or by number, whichever it is)
            ("playFrame", [v]) if mine => return Ok((self.jump_to_value(v)?, Ty::Any)),
            ("updateProjectileStats", [Expr::Object(pairs)]) if mine => return self.projectile_stats(pairs),
            // its own size: the `width` and `height` it declares (nothing, when it declares none),
            // or what its code set them to since
            ("getWidth" | "getHeight", []) if mine && !self.o.fighter => {
                let k = if m == "getWidth" { "width" } else { "height" };
                if self.own_stats.contains(k) { return Ok((format!("{}.get()", self.own_stat(k)), Ty::Num)) }
                let v = (self.o.declared)(k).ok_or_else(|| format!("reads its {k}, which its declarations do not agree on"))?;
                return Ok((if v == "null" { "0".into() } else { v }, Ty::Num));
            }
            ("getProjectileStat" | "getItemStat" | "getEnemyStat", [Expr::Str(k)]) if mine && self.own_stats.contains(k) => return Ok((format!("{}.get()", self.own_stat(k)), Ty::Any)),
            // (one its handler keeps as it runs: as the handler reads it, runtime/native_calls.jsonc `item_stat_reads`)
            ("getItemStat", [Expr::Str(k)]) if mine && self.o.item && calls().item_stat_reads.contains_key(k) => {
                let declared = (self.o.declared)(k).unwrap_or_else(|| "null".into());
                return Ok((calls().item_stat_reads[k].replace("{declared}", &declared), Ty::Any));
            }
            ("getProjectileStat" | "getItemStat" | "getEnemyStat", [Expr::Str(k)]) if mine => {
                return (self.o.declared)(k).map(|v| (v, Ty::Any)).ok_or_else(|| format!("reads its stat {k}, which its declarations do not agree on"));
            }
            // its own stat by the source's name: what it declares (or its code wrote since), else the engine's
            ("getGameObjectStat", [Expr::Str(k)]) if mine => {
                let declared = if self.own_stats.contains(k) { format!("{}.get()", self.own_stat(k)) } else { (self.o.declared)(k).unwrap_or_else(|| "null".into()) };
                let g = self.util("gameObjectStat")?;
                return Ok((format!("{g}(self, {}, {declared})", quote(k)), Ty::Any));
            }
            // what another object declares: its stats' metadata, which holds its declaration, read by
            // the stat's own name (the engine's own stat, where it declares none: `getGameObjectStat`)
            ("getProjectileStat" | "getItemStat" | "getEnemyStat" | "getGameObjectStat", [k]) if !mine => {
                let named = match k { Expr::Str(k) if crate::re!(r"^[A-Za-z_]\w*$").is_match(k) => Some(k.clone()), _ => None };
                let Some(k) = named else {
                    self.gaps.insert(format!("{m}: reads another object's stat by a name worked out as it runs"));
                    return Ok(("null".into(), Ty::Any));
                };
                let d = self.util("declaredOf")?;
                if m == "getGameObjectStat" {
                    let g = self.util("gameObjectStat")?;
                    return Ok((format!("{g}({who}, {}, {d}({who}).{k})", quote(&k)), Ty::Any));
                }
                return Ok((format!("{d}({who}).{k}"), Ty::Any));
            }
            // (its own box's, or one of an object it made: an item it holds)
            // (one this engine keeps none of reads as nothing, as its writes land on nothing)
            // (a fighter's own box's: what the box declares, which its attack's metadata carries,
            // `boxMeta`; a box declaring none reads as nothing)
            ("getHitboxStat", [i, Expr::Str(k)]) if mine && self.o.fighter && crate::mappings::character_hitbox_stats().no_counterpart.iter().any(|x| x == k) => {
                let i = self.box_ix(i)?;
                self.blocks.insert("hits".into());
                return Ok((format!("boxMeta({} + 1).{k}", paren(&i)), Ty::Any));
            }
            // (an object's own box's, by its number: what the box declares)
            ("getHitboxStat", [i, Expr::Str(k)]) if mine && !self.o.fighter && crate::mappings::character_hitbox_stats().no_counterpart.iter().any(|x| x == k)
                && box_index(i).is_ok() && (self.o.declared)(&format!("attackBoxes.attackBox{}.{k}", box_name_suffix(i))).is_some() => {
                return Ok(((self.o.declared)(&format!("attackBoxes.attackBox{}.{k}", box_name_suffix(i))).unwrap_or_else(|| "null".into()), Ty::Any));
            }
            // (another fighter's box's: what the attack it is in declares, from its animation's metadata)
            ("getHitboxStat", [i, Expr::Str(k)]) if !mine && self.o.fighter && crate::mappings::character_hitbox_stats().no_counterpart.iter().any(|x| x == k) => {
                let n = self.box_ix(i)?;
                return Ok((format!("({who} == null || {who}.getAnimationStatsMetadata() == null || {who}.getAnimationStatsMetadata().boxes == null || {who}.getAnimationStatsMetadata().boxes[{n}] == null ? null : {who}.getAnimationStatsMetadata().boxes[{n}].{k})"), Ty::Any));
            }
            ("getHitboxStat", [_, Expr::Str(k)]) if (mine || self.o.fighter) && crate::mappings::character_hitbox_stats().no_counterpart.iter().any(|x| x == k) => {
                self.gaps.insert(format!("reads its box's {k}, which this engine keeps none of"));
                return Ok(("null".into(), Ty::Any));
            }
            ("getHitboxStat", [i, Expr::Str(k)]) if mine || self.o.fighter => {
                let (field, scale) = hitbox_field(k)?;
                let i = self.box_ix(i)?;
                let back = scale.map(|s| format!(" / {}", number(s))).unwrap_or_default();
                return Ok((format!("{who}.getHitboxStats({i}).{field}{back}"), Ty::Num));
            }
            ("updateHitboxStats", [i, Expr::Member(t, f)]) if matches!(**t, Expr::This) && (mine || self.o.fighter) && self.only_record(f).is_some() => {
                let record = self.only_record(f).ok_or_else(|| format!("writes box stats from {f}, a record worked out as it runs"))?;
                return self.api(who, m, &[i.clone(), record]);
            }
            // (a record worked out as it runs: each field the engine keeps, written as it is then)
            ("updateHitboxStats", [i, r]) if !matches!(r, Expr::Object(_)) => {
                let i = self.box_ix(i)?;
                let (r, _) = self.expr(r)?;
                if !self.helpers.contains_key("boxWrite") {
                    let hb = crate::mappings::character_hitbox_stats();
                    let mut cases = Vec::new();
                    for f in &hb.fields {
                        for k in &f.ssf2_keys {
                            let Ok((field, scale)) = hitbox_field(k) else { continue };
                            if matches!(k.as_str(), "weightKB" | "power" | "kbConstant") { continue }
                            let v = match scale { Some(sc) => format!("r.{k} * {}", number(sc)), None => format!("r.{k}") };
                            cases.push(format!("if (r.{k} != null) s.{field} = {v};"));
                        }
                    }
                    for fl in &hb.flags { cases.push(format!("if (r.{} != null && (r.{} == true) == {}) s.{} = {};", fl.ssf2_key, fl.ssf2_key, fl.when, fl.fm_field, fl.value)); }
                    // (a fixed launch: the source's rule written out, else the box's own power and growth)
                    let (base, bs) = hitbox_field("power")?;
                    let (growth, gs) = hitbox_field("kbConstant")?;
                    let sc = |v: &str, s: Option<f64>| match s { Some(s) => format!("{v} * {}", number(s)), None => v.to_string() };
                    let rule = hb.fields.iter().find(|f| f.fm_field == "fixed_knockback").and_then(|r| Some((r.per_unit?, r.constant?)));
                    let kbc = "(r.kbConstant == null ? 100 : r.kbConstant)";
                    let pw = "(r.power == null ? 0 : r.power)";
                    match rule {
                        Some((per, plus)) => cases.push(format!("if (r.weightKB != null && r.weightKB > 0) {{ s.{base} = {}; s.{growth} = 0; }} else {{ if (r.power != null) s.{base} = {}; if (r.kbConstant != null) s.{growth} = {}; }}",
                            sc(&format!("((r.weightKB * {} + {}) * {kbc} / 100 + {pw})", number(per), number(plus)), bs), sc("r.power", bs), sc("r.kbConstant", gs))),
                        None => { cases.push(format!("if (r.power != null) s.{base} = {};", sc("r.power", bs))); cases.push(format!("if (r.kbConstant != null) s.{growth} = {};", sc("r.kbConstant", gs))); }
                    }
                    self.helpers.insert("boxWrite".into(), helper(self.o.runtime, "helpers", "box_write", "boxWrite", &[], &cases)?);
                }
                return Ok((format!("boxWrite({who}, {i}, {r})"), Ty::Any));
            }
            // a box's stats as the source's record (to deal outright, `takeDamage`): the engine's own
            // box, of the animation the label names, read back in the source's units
            ("exportAttackBoxStats" | "exportHitboxStats", [i, rest @ ..]) => {
                let i = self.box_ix(i)?;
                let anim = match rest.first() { Some(Expr::Str(l)) => format!(", {}", quote(l)), None | Some(Expr::Null | Expr::Undefined) => String::new(), Some(_) => return Err("exports a box of an animation worked out as it runs".into()) };
                return Ok((format!("{}({who}.getHitboxStats({i}{anim}))", self.box_record()?), Ty::Rect));
            }
            // a floor the shape of a library clip: the clip's measured extent, as its own floor
            ("createSelfPlatformWithMC", [Expr::Call(g, a), rest @ ..]) if mine && !self.o.fighter
                && matches!(&**g, Expr::Member(_, n) if matches!(n.as_str(), "getMCByLinkageName" | "clipByLinkage")) && matches!(a.as_slice(), [Expr::Str(_)]) => {
                let Some(Expr::Str(n)) = a.first() else { unreachable!() };
                let (x, y, w, h) = *self.o.clip_bounds.get(n).ok_or_else(|| format!("makes a floor the shape of {n}, a clip with no extent measured"))?;
                let terrain = match rest.first() { Some(t) => self.expr(t)?.0, None => "false".into() };
                return Ok((format!("ownFloor({}, {}, {}, {}, {terrain})", number(x), number(y), number(w), number(h)), Ty::Object));
            }
            // (a clip its code made and drew into: the extent it drew)
            ("createSelfPlatformWithMC", [c, rest @ ..]) if mine && !self.o.fighter && self.expr(c).is_ok_and(|(_, t)| t == Ty::Rect)
                && !matches!(c, Expr::Call(g, _) if matches!(&**g, Expr::Member(_, n) | Expr::Name(n) if matches!(n.as_str(), "getMCByLinkageName" | "clipByLinkage"))) => {
                let (c, _) = self.expr(c)?;
                let terrain = match rest.first() { Some(t) => self.expr(t)?.0, None => "false".into() };
                return Ok((format!("ownFloorOfClip({c}, {terrain})"), Ty::Object));
            }
            // (one named as it runs from a prefix: whichever measured library clip of that prefix it names)
            ("createSelfPlatformWithMC", [Expr::Call(g, a), rest @ ..]) if mine && !self.o.fighter
                && matches!(&**g, Expr::Member(_, n) if matches!(n.as_str(), "getMCByLinkageName" | "clipByLinkage")) && matches!(a.as_slice(), [Expr::Binary(BinOp::Add, p, _)] if matches!(&**p, Expr::Str(_))) => {
                let Some(Expr::Binary(_, p, _)) = a.first() else { unreachable!() };
                let Expr::Str(prefix) = &**p else { unreachable!() };
                let named: Vec<(String, (f64, f64, f64, f64))> = self.o.clip_bounds.iter().filter(|(k, _)| k.starts_with(prefix.as_str()) && !k.contains('>')).map(|(k, v)| (k.clone(), *v)).collect();
                if named.is_empty() { return Err(format!("makes a floor the shape of a clip named {prefix}…, and no clip of that name is measured")) }
                let f = format!("floorOf{}", ident(prefix));
                if !self.helpers.contains_key(&f) {
                    let mut lines = vec![format!("// a floor the shape of the library clip named {prefix}… it names, as measured"), format!("function {f}(n, terrain) {{"), "\tswitch (n) {".to_string()];
                    for (k, (x, y, w, h)) in &named { lines.push(format!("\t\tcase {}: return ownFloor({}, {}, {}, {}, terrain);", quote(k), number(*x), number(*y), number(*w), number(*h))); }
                    lines.push("\t}".into());
                    lines.push("\treturn null;".into());
                    lines.push("}".into());
                    self.helpers.insert(f.clone(), lines.join("\n"));
                }
                let (n, _) = self.expr(&a[0])?;
                let terrain = match rest.first() { Some(t) => self.expr(t)?.0, None => "false".into() };
                return Ok((format!("{f}({n}, {terrain})"), Ty::Object));
            }
            // an attack a stage object starts on itself: the animation its label is
            // (one named as it runs: the jump by label its clips' labels make, `jumpTo`, which answers
            // that it started)
            ("forceAttack" | "startAttack", [label, ..]) if mine && !self.o.fighter && !self.o.item && !matches!(label, Expr::Str(_))
                && self.o.classes.iter().any(|c| self.o.jumps.get(&c.name).is_some_and(|j| !j.labels.is_empty())) => {
                let (l, _) = self.expr(label)?;
                if !self.helpers.contains_key("startNamed") {
                    self.helpers.insert("startNamed".into(), "// an attack it starts on itself by a label worked out as it runs: the jump to it\nfunction startNamed(l) {\n\tjumpTo(l);\n\treturn true;\n}".into());
                }
                return Ok((format!("startNamed({l})"), Ty::Bool));
            }
            // its final smash cutscene: its own `<name>_hud` clip, made as an object of its (its class
            // runs it) and held over the screen, bottom centre, as the source's game shows it
            ("startFinalCutscene", []) if mine && self.o.fighter => {
                let clip = self.o.model.fighters().first().and_then(|f| f.clip().map(String::from)).unwrap_or_default();
                let want = format!("{clip}_hud");
                let Some(name) = self.o.model.symbols.values().find(|n| n.eq_ignore_ascii_case(&want)).cloned() else {
                    return Err("plays a final smash cutscene, which its package has no clip for".into());
                };
                let f = self.fire(&name, "self")?;
                self.util("fsCutscene")?;
                return Ok((format!("fsCutsceneStart({f}(0, 0))"), Ty::Any));
            }
            // turning metal: the game's metal look, as its shared packages declare it, laid over the
            // fighter as the engine's colour filter (`metalOn`)
            ("setMetalStatus", [on, ..]) => {
                let (on, _) = self.expr(on)?;
                let look = crate::shared_packages::metal_look().ok_or("turns metal, and no package here declares the metal look")?;
                let g = |k: &str, d: f64| look.get(k).copied().unwrap_or(d);
                let words = [("SATURATION", number(g("saturation", -100.0) / 100.0)),
                             ("BRIGHTNESS", number((g("brightness", 0.0) + g("redOffset", 0.0)) / 100.0)),
                             ("CONTRAST", number(g("contrast", 0.0) / 100.0 * g("redMultiplier", 1.0)))];
                if !self.helpers.contains_key("metalOn") { self.helpers.insert("metalOn".into(), helper(self.o.runtime, "utils", "metalOn", "metalOn", &words, &[])?); }
                return Ok((format!("metalOn({who}, {on})"), Ty::Any));
            }
            // (asked of the fighter by one of its objects: the fighter's cutscene, made by the object
            // for the fighter)
            ("startFinalCutscene", []) if !mine && !self.o.fighter && !self.o.stage => {
                let name = self.o.model.fighters().first().and_then(|f| f.clip().map(String::from))
                    .and_then(|clip| { let want = format!("{clip}_hud"); self.o.model.symbols.values().find(|n| n.eq_ignore_ascii_case(&want)).cloned() })
                    .ok_or("plays a final smash cutscene, which its package has no clip for")?;
                let content = crate::projectile_gen::projectile_content_id(&name);
                self.util("fsCutscene")?;
                return Ok((format!("{{ var fsBy = {who}; var fsMade = fsBy == null ? null : match.createProjectile(fsBy.getResource().getContent({}), fsBy); fsCutsceneStart(fsMade); }}", quote(&content)), Ty::Any));
            }
            ("endFinalCutscene", []) if !self.o.fighter && !self.o.stage => {
                self.util("fsCutscene")?;
                return Ok(("fsCutsceneEnd()".into(), Ty::Any));
            }
            ("endFinalCutscene", []) if mine && self.o.fighter => {
                self.util("fsCutscene")?;
                return Ok(("fsCutsceneEnd()".into(), Ty::Any));
            }
            ("forceAttack" | "startAttack", [Expr::Str(label), ..]) if mine && !self.o.fighter && !self.o.item => {
                // (asked by a clip shown inside its art, which has no such label: the attack is the
                // object's, the label of the clip the object draws with)
                let class = self.o.model.classes.get(&self.class);
                let has = class.and_then(|c| c.symbol).and_then(|s| self.o.model.sprites.get(&s)).map(|sp| sp.labels.iter().any(|(_, l)| l == label));
                let root = self.o.model.class_of(self.o.root).and_then(|c| self.o.jumps.get(&c.name)).and_then(|j| j.labels.get(label.as_str())).cloned();
                if let (Some(false), Some((anim, start))) = (has, root) {
                    let t = if start { self.play_start(&anim)? } else { self.play_label_in(&anim, label) };
                    return Ok((t.trim_end_matches(';').to_string(), Ty::Any));
                }
                return Ok((self.jump(label)?.trim_end_matches(';').to_string(), Ty::Any));
            }
            // (by a label worked out as it runs: each label its clip has, jumped to as written)
            ("forceAttack" | "startAttack", [label, ..]) if mine && !self.o.fighter && !self.o.item && self.expr(label).is_ok_and(|(_, t)| t == Ty::Str)
                && self.o.jumps.get(&self.class).is_some_and(|j| !j.labels.is_empty()) => {
                let (l, _) = self.expr(label)?;
                let labels: Vec<String> = self.o.jumps[&self.class].labels.keys().cloned().collect();
                let mut out = vec![format!("switch ({l}) {{")];
                for x in labels { out.push(format!("\tcase {}: {}", quote(&x), self.jump(&x)?.replace('\n', "\n\t"))); }
                out.push("}".into());
                return Ok((out.join("\n"), Ty::Any));
            }
            ("forceAttack" | "startAttack", [label, ..]) if mine && !self.o.fighter && !self.o.item => return Ok((self.jump_to_value(label)?, Ty::Any)),
            // whether fighters drop through it: its own floors, or the surface asked
            ("setFallthrough", [v]) if mine && !self.o.fighter && !self.o.stage => { let (v, _) = self.expr(v)?; return Ok((format!("facesThrough({v})"), Ty::Any)) }
            // (whether they stand fast: the other way round)
            ("setNoDropThrough", [v]) if mine && !self.o.fighter && !self.o.stage => { let v = self.test(v)?; return Ok((format!("facesThrough(!{})", paren(&v)), Ty::Any)) }
            ("getNoDropThrough", []) if mine && !self.o.fighter && !self.o.stage => return Ok(("!throughOf()".into(), Ty::Bool)),
            ("getFallthrough", []) if mine && !self.o.fighter && !self.o.stage => return Ok(("throughOf()".into(), Ty::Bool)),
            ("setFallthrough", [v]) => { let (v, _) = self.expr(v)?; return Ok((format!("if ({who} != null) {who}.updateStructureStats({{ dropThrough: {v} == true }})"), Ty::Any)) }
            // (its own boxes, or another object's: a projectile it fired)
            ("updateHitboxStats", [i, Expr::Object(pairs)]) => {
                let i = self.box_ix(i)?;
                let mut set = Vec::new();
                // a fixed launch worked out as it runs: the source's rule written out, where it is
                // one (above 0), else the box's own power and growth
                let named = |n: &str| pairs.iter().find(|(k, _)| matches!(k, Expr::Str(k) if k == n)).map(|(_, v)| v);
                let live_fixed = named("weightKB").is_some_and(|w| literal(w).is_none());
                if live_fixed {
                    let hb = crate::mappings::character_hitbox_stats();
                    let rule = hb.fields.iter().find(|f| f.fm_field == "fixed_knockback").ok_or("writes a fixed launch the mapping has no rule for")?;
                    let (per, plus) = (rule.per_unit.ok_or("writes a fixed launch the mapping has no rule for")?, rule.constant.ok_or("writes a fixed launch the mapping has no rule for")?);
                    let w = self.expr(named("weightKB").unwrap())?.0;
                    let kbc = match named("kbConstant") { Some(v) => self.expr(v)?.0, None => "100".into() };
                    let pw = match named("power") { Some(v) => self.expr(v)?.0, None => "0".into() };
                    let (base, bs) = hitbox_field("power")?;
                    let (growth, gs) = hitbox_field("kbConstant")?;
                    let sc = |v: String, s: Option<f64>| match s { Some(s) => format!("{} * {}", paren(&v), number(s)), None => v };
                    let fixed = sc(format!("({} * {} + {}) * {} / 100 + {}", paren(&w), number(per), number(plus), paren(&kbc), paren(&pw)), bs);
                    set.push(format!("{base}: ({} > 0 ? {fixed} : {})", paren(&w), sc(pw.clone(), bs)));
                    set.push(format!("{growth}: ({} > 0 ? 0 : {})", paren(&w), sc(kbc.clone(), gs)));
                }
                for (k, v) in pairs {
                    let Expr::Str(k) = k else { return Err("writes a box stat by a name worked out as it runs".into()) };
                    if live_fixed && matches!(k.as_str(), "weightKB" | "kbConstant" | "power") { continue }
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
                    // a stat this engine has no field for lands on nothing, as nothing reads it
                    let Ok((field, scale)) = hitbox_field(k) else { continue };
                    let (v, _) = self.expr(v)?;
                    set.push(match scale { Some(s) => format!("{field}: {} * {}", paren(&v), number(s)), None => format!("{field}: {v}") });
                }
                // (a damage written on a box whose freezes are unstated moves them too: the source works
                // an unstated freeze out from the damage the hit lands with, by the rule in
                // hitbox_stats.jsonc)
                let freeze = self.o.jumps.get(&self.class).map(|j| j.freeze_unstated).unwrap_or((false, false));
                if let Some(d) = named("damage") {
                    let hb = crate::mappings::character_hitbox_stats();
                    let (d, _) = self.expr(d)?;
                    for (on, fm, src) in [(freeze.0, "hitstop", "hitStun"), (freeze.1, "selfHitstop", "selfHitStun")] {
                        if !on || named(src).is_some() { continue }
                        let Some(rule) = hb.fields.iter().find(|f| f.fm_field == fm) else { continue };
                        let (Some(per), Some(c)) = (rule.unstated_per_damage, rule.unstated_constant) else { continue };
                        set.push(format!("{fm}: Math.floor({} * {} + {}) * 2", paren(&d), number(per), number(c)));
                    }
                }
                // (one that writes nothing this engine keeps writes nothing)
                if set.is_empty() { return Ok(("// (the source writes box stats here that this engine keeps none of)".into(), Ty::Any)) }
                return Ok((format!("{who}.updateHitboxStats({i}, {{ {} }})", set.join(", ")), Ty::Any));
            }
            // another object's attack stat the engine keeps natively, read back from its own field
            // a stat its owner states: what its code set it to since, else what it declares
            ("getCharacterStat", [Expr::Str(k)]) if self.owners.contains(who) => {
                // (one its owner never states: the source's default for it, `runtime_read_defaults`;
                // one with none read as nothing set, 0, and said in its gaps)
                let default = crate::mappings::character_stats().runtime_read_defaults.get(k).map(|v| v.to_string());
                let Some(declared) = (self.o.owner_declares)(k).or(default) else {
                    self.gaps.insert(format!("reads its owner's stat {k}, which its owner does not state: read as 0"));
                    return Ok(("0".into(), Ty::Num));
                };
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
            ("getLedges", []) if self.o.fighter && who == "stage" => {
                self.blocks.insert("ledges".into());
                return Ok(("stageLedges()".into(), Ty::Rect));
            }
            // (the stage's own: what its code keeps of them, which its writes move)
            ("getDeathBounds" | "getCameraBounds", []) if self.o.stage && (who == "stage" || who == "self") => {
                self.util("stageBounds")?;
                return Ok((format!("stageBoundsRec({})", m == "getCameraBounds"), Ty::Rect));
            }
            ("getDeathBounds" | "getCameraBounds", []) if who == "stage" => {
                let f = self.sized_helper("source_rect", "sourceRect")?;
                return Ok((format!("{f}(stage.{m}())"), Ty::Rect));
            }
            // another fighter's own stats: what this engine can say of them (`foeOwnStats`)
            ("getOwnStats" | "ownStats", []) if !mine && self.o.stage => {
                self.util("foeOwnStats")?;
                self.gaps.insert("reads another fighter's own stats, of which this engine says its name".into());
                return Ok((format!("foeOwnStats({who})"), Ty::Rect));
            }
            // a stat another fighter states: what its code set, as it publishes it, else the engine's
            ("getCharacterStat", [k]) if !mine => {
                // (by a name the engine keeps a stat under, or one it maps to one; any other is only what
                // that fighter publishes: the engine throws on a stat it has none of)
                let engine = match k {
                    Expr::Str(n) => {
                        let st = crate::mappings::character_stats();
                        let fm = st.fm_runtime_read_names.get(n).cloned().or_else(|| st.fm_runtime_stats.iter().find(|x| *x == n).cloned());
                        // (one this engine keeps under another shape: read its way)
                        fm.map(|f| format!("{who}.getCharacterStat({})", quote(&f)))
                            .or_else(|| st.runtime_read_exprs.get(n).map(|e| e.replace("{recv}", who).replace("{velocity_scale}", &number(self.o.speed))))
                    }
                    _ => None,
                };
                let (k, _) = self.expr(k)?;
                let engine = engine.unwrap_or_else(|| if matches!(k.chars().next(), Some('"')) { "null".to_string() } else { format!("{who}.getCharacterStat({k})") });
                return Ok((format!("({who}.exports.__source != null && {who}.exports.__source.__stats != null && {who}.exports.__source.__stats.exists({k}) ? {who}.exports.__source.__stats.get({k}) : {engine})"), Ty::Any));
            }
            // another fighter's numbered sound (its owner's, from a projectile's frame): the one its
            // attack names, read as that attack's stat is, played from that fighter's own sounds
            ("playAttackSfx" | "playVoiceSfx", [n, ..]) if !mine && matches!(n, Expr::Int(_) | Expr::Num(_)) => {
                let n = match n { Expr::Int(n) => *n, Expr::Num(v) => *v as i64, _ => 1 };
                let key = format!("{}{}_id", if m == "playVoiceSfx" { "attackVoice" } else { "attackSound" }, n.max(1));
                let (id, _) = self.api(who, "getAnimationStat", &[Expr::Str(key)])?;
                return Ok((format!("{{ var sfx = {id}; sfx == null ? null : AudioClip.play({who}.getResource().getContent(sfx)); }}"), Ty::Object));
            }
            // an attack its code starts on its owner (or any other fighter), and a rocket it sets off on
            // its owner: what that fighter publishes
            ("forceAttack" | "startAttack", [label, rest @ ..]) if (self.owners.contains(who) && !self.o.fighter) || !mine => {
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
                return Ok((format!("controlsOf({who}, {p})"), Ty::Controls));
            }
            // the animation another fighter plays, as the source names it: the label its script publishes
            ("getAnimation", []) if !mine && !self.owners.contains(who) => {
                return Ok((format!("({who} != null && {who}.exports.__label != null ? {who}.exports.__label() : null)"), Ty::Str));
            }
            // the attack a fighter plays itself, as the source names it: its root clip's label
            ("getAnimation", []) if mine && self.o.fighter => return Ok(("rootLabel()".into(), Ty::Str)),
            // (an object's own: the label of its clip it is on, `clipLabel`)
            ("getAnimation", []) if mine && !self.o.fighter && !self.o.stage => { self.blocks.insert("clip_label".into()); return Ok(("clipLabel()".into(), Ty::Str)) }
            // the animation its owner plays, as the source names it: its clip's label
            ("getAnimation", []) if self.owners.contains(who) => return Ok((format!("{}({who})", self.owner_label()?), Ty::Str)),
            // a projectile's, item's or enemy's own attack stat: what its code wrote since, else what it
            // declares (its stats' metadata), as the source's object keeps its attack in its declaration
            ("getAnimationStat", [Expr::Str(k)]) if mine && !self.o.fighter => {
                if self.own_stats.contains(k) { return Ok((format!("{}.get()", self.own_stat(k)), Ty::Any)) }
                let d = self.util("declaredOf")?;
                return Ok((format!("{d}(self).{}", ident(k)), Ty::Any));
            }
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
                return Ok((format!("({who} != null && {who}.exports.__attackStat != null ? {who}.exports.__attackStat({}, {who}.exports.__attackMeta != null ? {who}.exports.__attackMeta().{} : null) : null)", quote(k), ident(k)), Ty::Any));
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
        // (a stage's and its objects' own form of a row, where it has one: places in the stage's space)
        let staged = (self.o.origin != (0.0, 0.0)).then(|| calls().rows.get(&format!("{m}@stage")).cloned()).flatten();
        let package_method = !mine && (self.o.model.classes.values().any(|c| !self.o.runtime.provides(&c.name) && c.methods.iter().any(|x| x.name == m))
            || lent_methods().lock().is_ok_and(|l| l.iter().any(|x| x == m)));
        let row = staged.or_else(|| calls().rows.get(m).cloned());
        // (a method of another object's own class that the API's row of that name cannot take, as
        // it is handed: the object's own)
        let row = match row {
            Some(r) if package_method && (0..4).any(|i| r.code.contains(&format!("{{{i}}}")) && args.len() <= i) => None,
            r => r,
        };
        let row = match row {
            Some(r) => r,
            None => {
                // a method of another object's own class (of its package, or of an object it makes
                // that another package lends it): that object publishes it on its exports
                let defined = !mine && (self.o.model.classes.values().any(|c| !self.o.runtime.provides(&c.name) && c.methods.iter().any(|x| x.name == m))
                    || lent_methods().lock().is_ok_and(|l| l.iter().any(|x| x == m)));
                if !defined { return Err(format!("calls {m}, which has no row in native_calls.jsonc")) }
                let vs = args.iter().map(|x| Ok(self.expr(x)?.0)).collect::<Result<Vec<String>, String>>()?.join(", ");
                if let Ok(mut a) = asked_of_others().lock() { a.insert(m.to_string()); }
                // (one already gone has no script left to run it, and a surface publishes nothing)
                return Ok((format!("({who} != null && {who}.exports != null && !{who}.isDisposed() && {who}.exports.{m} != null ? {who}.exports.{m}({vs}) : ({who} != null && {who}.exports != null && {who}.exports.__source != null ? {who}.exports.__source.{m}({vs}) : null))"), Ty::Any));
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
            // (a zero written on the ground under a held direction is the walk's start speed, the
            // source's walk rule: a speed written as some other number is the engine's own write)
            // (an x speed written with no second argument is the source's world space, rightward
            // positive, where this engine's is along the way it faces: turned the way it faces, as
            // sandbag's air down special dashes the way it is held, whichever way he faces. one
            // written `false` is along its facing already, a zero has no way, and one worked out
            // from the speed the engine reports is along its facing as that is)
            ("setXSpeed", [v]) if !matches!(v, Expr::Int(0)) && !matches!(v, Expr::Num(n) if *n == 0.0) && {
                let mut read = false;
                crate::model::ast::walk_exprs(&[Stmt::Expr(v.clone())], &mut |e| if let Expr::Call(g, _) = e {
                    if matches!(&**g, Expr::Member(_, n) if matches!(n.as_str(), "getXSpeed" | "getNetXSpeed" | "getXVelocity")) { read = true; }
                });
                !read
            } => {
                let (v, _) = self.expr(v)?;
                let speed = fold(&format!("{} * {}", paren(&v), number(self.o.speed)));
                Ok((format!("self.setXSpeed(self.flipX({speed}))"), Ty::Any))
            }
            ("setXSpeed", [v]) if self.o.walk_start.is_some() => {
                let (v, _) = self.expr(v)?;
                let speed = fold(&format!("{} * {}", paren(&v), number(self.o.speed)));
                // (a walk that starts from nothing marks every write in it: the next one lets a held zero go)
                let from_nothing = self.o.walk_start == Some(0.0);
                if (speed.parse::<f64>().is_ok_and(|n| n != 0.0) && !from_nothing) || !self.in_frame { return Ok((format!("self.setXSpeed({speed})"), Ty::Any)) }
                if from_nothing { block(self, "walk_hold"); }
                // (whether the frame is one of a walk's is settled per animation, `native_fighter::walk_starts`)
                Ok((format!("self.setXSpeed({speed} /*walk*/)"), Ty::Any))
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
                // (one no animation shows has no frames to start: the source's start goes to a label its
                // clip does not have, which stops the code it is in there, so this code stops there too)
                let Some(anim) = (self.o.root_anim)(label) else {
                    return Ok((format!("return /* the attack {label}: no animation of its shows it, and the source's code stops at it */"), Ty::Any));
                };
                block(self, "start_attack");
                let mut t = format!("startAttack({})", quote(&anim));
                if let Some(Expr::Str(f)) = rest.first() { t.push_str(&format!(";
self.playFrameLabel({})", quote(f))); }
                Ok((t, Ty::Any))
            }
            // an attack it starts by a name worked out as it runs: the animation that shows it, found then
            ("startAttack" | "forceAttack", [label, rest @ ..]) => {
                let (l, _) = self.expr(label)?;
                let frame = match rest.first() { Some(Expr::Str(f)) => format!(", {}", quote(f)), _ => String::new() };
                block(self, "start_attack");
                self.stance_jump("")?;
                Ok((format!("forceAttackAsked({l}{frame})"), Ty::Any))
            }
            // the power it holds, copied from another fighter (`runtime/native/fighter_rules.hx` `copy_power`)
            ("copyPower", []) => { self.copy_power()?; Ok(("powerName()".into(), Ty::Str)) }
            // a jump by label on the clip it shows (whichever animation plays)
            ("stancePlayFrame" | "playFrame", [Expr::Str(label)]) => Ok((self.stance_jump(label)?, Ty::Any)),
            ("stancePlayFrame" | "playFrame", [label]) => Ok((self.jump_to_value(label)?, Ty::Any)),
            // its landing: on touching down, the attack goes on from `continue`, or it lands
            ("setLandingLag", [on]) => {
                let on = self.test(on)?;
                // (its own, or the one a class of its package it extends has)
                let cont = self.itself_method("jumpToContinue").or_else(|| self.methods.iter().find(|(k, _)| k.ends_with("::jumpToContinue")).map(|(_, v)| v.clone()));
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
            ("endAttack" | "endAnimation", []) => Ok(("self.endAnimation()".into(), Ty::Any)),
            ("getControls", a) => {
                block(self, "controls");
                let p = match a.first() { None => "false".to_string(), Some(x) => self.test(x)? };
                Ok((format!("controls({p})"), Ty::Controls))
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
                    if let Some((code, h)) = st.status_writes.get(k) { self.util(h)?; out.push(code.replace("{who}", "self").replace("{v}", &paren(&val))); continue }
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
                // (the source's path to it: that one alone, or none)
                Ok((if m == "getNearestPath" { format!("(nearest({t}) == null ? [] : [nearest({t})])") } else { format!("nearest({t})") }, Ty::Object))
            }
            // (the source's engine picks a homing target in its own range and says so by its
            // `homingTarget` event; nothing here picks one, so there is none to head for)
            ("getHomingTarget", []) => { self.util("homingPick")?; Ok(("homingPick()".into(), Ty::Body)) }
            // the hat of the power it holds (`hatFrame`), where it takes powers
            ("copyHatClip", []) if takes_power(self.o.model, self.o.runtime) => { self.copy_power()?; Ok(("hatVfx.get()".into(), Ty::Object)) }
            ("homeTowardsTarget", [sp, t]) => {
                block(self, "home");
                let (sp, _) = self.expr(sp)?;
                let (t, _) = self.expr(t)?;
                Ok((format!("homeTowards({sp}, {t})"), Ty::Any))
            }
            ("getNearestLedge", []) => { block(self, "ledges"); Ok(("nearestLedge()".into(), Ty::Rect)) }
            ("recover", [n]) => { block(self, "recover"); let (n, _) = self.expr(n)?; Ok((format!("recover({n})"), Ty::Bool)) }
            ("forceOnGround", _) => { block(self, "floor_hold"); Ok(("holdToFloor()".into(), Ty::Any)) }
            ("allowAttack", [on, Expr::Str(label), rest @ ..]) => {
                block(self, "attack"); block(self, "actions");
                self.stat_keys.insert("reenableTimer".into());
                self.stat_keys.insert(format!("anims:{label}"));
                let on = self.test(on)?;
                let frames = match rest.first() { None | Some(Expr::Null | Expr::Undefined) => String::new(), Some(x) => format!(", {}", self.expr(x)?.0) };
                // (turned off for as long as the attack declares, read at conversion: `native_fighter::reenable`)
                let frames = if frames.is_empty() { " /*reenable*/".to_string() } else { frames };
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
                let Some(to) = states().get(&name).and_then(|s| s.enter.clone()) else { return Ok((format!("// (not ported: the source puts it in its {name} state here, which this engine's fighters have none of)"), Ty::Any)) };
                Ok((format!("self.toState({to})"), Ty::Any))
            }
            // the item it holds: one its own code made (no other here is held by a fighter)
            ("getItem", []) => { block(self, "items"); Ok(("heldItem()".into(), Ty::Object)) }
            ("makePickup" | "generateItem", [Expr::Str(name), ..]) => {
                block(self, "items");
                self.stat_keys.insert(format!("item:{name}"));
                Ok((format!("generateItem({})", quote(name)), Ty::Object))
            }
            // (one named as it runs: any of the items it declares)
            ("makePickup" | "generateItem", [name, ..]) => {
                block(self, "items");
                let (name, _) = self.expr(name)?;
                Ok((format!("generateItem({name})"), Ty::Object))
            }
            // an item of another fighter's, made by that one for it (through the power it publishes)
            ("makeFighterPickup" | "generateCharacterItem", [name, by, ..]) => {
                block(self, "items");
                let (name, _) = self.expr(name)?;
                let (by, _) = self.expr(by)?;
                Ok((format!("generateItem({name}, {by})"), Ty::Object))
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


    /// a row filled in: the object it is asked of, its arguments, the units
    fn fill(&mut self, row: &Row, who: &str, args: &[Expr], m: &str) -> Result<(String, Ty), String> {
        if let Some(g) = &row.gap { self.gaps.insert(g.clone()); }
        if let Some(h) = &row.helper {
            if h == "cameraParameter" { self.util(h)?; }
            self.util(h)?;
        }
        // (an origin of nothing adds and takes nothing: the row is the plain one)
        let mut code = row.code.clone();
        // (the stage's own track, named only in the stage's script: none, read from anywhere else)
        if !self.o.stage { code = code.replace("STAGE_MUSIC", "null"); }
        for (k, o) in [("{ox}", self.o.origin.0), ("{oy}", self.o.origin.1)] {
            if o != 0.0 { continue }
            for i in 0..4 { code = code.replace(&format!("({{{i}}} - {k})"), &format!("{{{i}}}")); }
            code = code.replace(&format!(" + {k}"), "").replace(&format!(" - {k}"), "");
        }
        let mut t = code.replace("{self}", who)
            .replace("{size}", &number(self.o.size)).replace("{speed}", &number(self.o.speed)).replace("{accel}", &number(self.o.accel))
            .replace("{ox}", &number(self.o.origin.0)).replace("{oy}", &number(self.o.origin.1));
        // (an argument the call may leave off, `{,1}`: written after a comma where it passes one)
        for i in 0..8 {
            let opt = format!("{{,{i}}}");
            if t.contains(&opt) { t = t.replace(&opt, &if i < args.len() { format!(", {{{i}}}") } else { String::new() }); }
        }
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
        // one a fighter does not declare: fired from the power it holds, where it takes powers
        if (self.o.fired)(name).is_none() && self.o.fighter && from == "self" && takes_power(self.o.model, self.o.runtime) {
            self.copy_power()?;
            let f = format!("firePower_{}", ident(name));
            if !self.helpers.contains_key(&f) {
                self.helpers.insert(f.clone(), format!("function {f}(?x, ?y, ?place) {{ return firePowerShot({}, x, y); }}", quote(name)));
            }
            return Ok(f);
        }
        let (content, [dx, dy, xs, ys], declares) = (self.o.fired)(name).ok_or_else(|| format!("fires {name}, which its owner does not declare"))?;
        let by_owner = from != "self";
        let f = if by_owner { format!("ownerFires_{}", ident(name)) } else { format!("fire_{}", ident(name)) };
        let from = if by_owner { "self.getOwner()" } else { "self" };
        if self.helpers.contains_key(&f) { return Ok(f) }
        let speed = number(self.o.speed);
        let words = [("PROJECTILE", name.to_string()), ("CONTENT", content), ("FROM", from.to_string()), ("DX", number(dx)), ("DY", number(dy)),
                     ("XS", fold(&format!("{} * {speed}", number(xs)))), ("YS", fold(&format!("{} * {speed}", number(ys)))), ("SIZE", number(self.o.size)),
                     // (fired under a name its entity is not written from: it takes that one's declaration on)
                     // (nothing to take on drops the statement)
                     // (an object whose stats declare no metadata has none to merge into: it is given its own)
                     ("DECLARES", declares.map(|r| format!("if (p.getGameObjectStatsMetadata() == null) p.updateGameObjectStats({{ metadata: {r} }}); else p.updateGameObjectStatsMetadata({r})")).unwrap_or_default())];
        let def = helper(self.o.runtime, "helpers", if self.o.fighter { "fighter_fire" } else { "fire" }, &f, &words, &[])?;
        let def = def.lines().filter(|l| l.trim() != ";").collect::<Vec<_>>().join("\n");
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

    /// a `native_calls.jsonc` `projectile_stats` row's code for the value `v`
    fn stat_code(&self, t: &str, v: &str) -> String {
        fold(&t.replace("{v}", &paren(v)).replace("{size}", &number(self.o.size))
            .replace("{speed}", &number(self.o.speed)).replace("{accel}", &number(self.o.accel)))
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
            // (a method of one of the package's own classes it holds in a field)
            Expr::Member(t, n) if self.flat_of(t).is_some_and(|(_, k)| self.pkg_ids.contains_key(&k)) => {
                let (_, k) = self.flat_of(t).unwrap_or_default();
                self.methods.get(&member_key(&k, n)).cloned()
            }
            Expr::Name(n) if self.method(n).is_some() => self.method(n),
            Expr::Function(c) => Some(self.closure(c)?),
            // (one of its own engine calls: a function that makes it; named bare, the API's own
            // method on it, as `destroy`)
            Expr::Member(_, n) | Expr::Name(n) if matches!(f, Expr::Name(_)) || matches!(f, Expr::Member(t, _) if self.expr(t).is_ok_and(|(w, _)| w == "self")) => {
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
        let id = format!("{name}__timer");
        self.kept.insert(id.clone(), format!("// the engine's timer running {name}, while there is one\nvar {id} = self.makeInt(-1);"));
        Ok(id)
    }

    /// a source timer (`createTimer(frames, repeats, fn)`): the engine's, of two engine frames to
    /// a source frame, forever for none. one function keeps one timer: starting it again restarts it
    fn timer(&mut self, frames: &Expr, repeat: &Expr, f: &Expr, rest: &[Expr]) -> Result<(String, Ty), String> {
        // (its timers are kept whatever it asks: `persistent` says nothing more here)
        let fighter = self.o.fighter;
        let plain_opts = |e: &Expr| matches!(e, Expr::Null | Expr::Undefined | Expr::Bool(_)) || matches!(e, Expr::Object(kv) if kv.iter().all(|(k, _)| matches!(k, Expr::Str(k) if k == "persistent" || k == "hitStunPause" || k == "pauseHitstun" || k == "inverseCondition" || (fighter && k == "condition"))));
        // (a fighter's timer with a condition runs its code only while that holds)
        let condition = rest.iter().find_map(|e| match e { Expr::Object(kv) => kv.iter().find(|(k, _)| matches!(k, Expr::Str(k) if k == "condition")).map(|(_, v)| v.clone()), _ => None });
        // (and one with an inverse condition runs it only while that does not hold: a final smash
        // waiting out its cutscene)
        let inverse = rest.iter().find_map(|e| match e { Expr::Object(kv) => kv.iter().find(|(k, _)| matches!(k, Expr::Str(k) if k == "inverseCondition")).map(|(_, v)| v.clone()), _ => None });
        // (one held while it is frozen by a hit: the engine's freeze pauses it)
        let paused = rest.iter().any(|e| matches!(e, Expr::Object(kv) if kv.iter().any(|(k, v)| matches!(k, Expr::Str(k) if k == "pauseHitstun") && matches!(v, Expr::Bool(true)))));
        if !rest.is_empty() && !rest.iter().all(plain_opts) { return Err("makes a timer with options".into()) }
        let id = self.timer_of(f)?;
        let name = id.strip_suffix("__timer").unwrap_or(&id).to_string();
        let (n, _) = self.expr(frames)?;
        let every = match n.parse::<f64>() { Ok(v) => number((2.0 * v.max(1.0)).round()), Err(_) => format!("Std.int(2 * Math.max(1, {}))", paren(&n)) };
        let times = match self.expr(repeat)?.0 {
            t if t.parse::<f64>().is_ok_and(|v| v <= 0.0) => "-1".to_string(),
            t if t.parse::<f64>().is_ok() => t,
            t => format!("({t} <= 0 ? -1 : {t})"),
        };
        if self.o.fighter {
            // a fighter's timer is the engine's: it drops with the state it was made in unless asked to
            // stay, and waits out its freezes. one a source frame long made in a frame's code is run by
            // that code right after making it, as its first tick (the source ticks the frame it is made
            // in too); one that runs out forgets its id (the engine gives it out again)
            let _ = paused;
            let keep = rest.iter().any(|e| matches!(e, Expr::Bool(true)) || matches!(e, Expr::Object(kv) if kv.iter().any(|(k, v)| matches!(k, Expr::Str(k) if k == "persistent") && matches!(v, Expr::Bool(true)))));
            self.blocks.insert("refresh".into());
            // (a condition written in place that only returns a value: that value, tested as it runs)
            let test = match condition {
                Some(Expr::Function(func)) => match func.body.iter().filter(|s| !matches!(s, Stmt::Return(None))).collect::<Vec<_>>().as_slice() {
                    [Stmt::Return(Some(v))] if func.params.is_empty() => Some(self.test(v)?),
                    _ => return Err("makes a timer whose condition does more than return a value".into()),
                },
                Some(c) => Some(self.test(&Expr::Call(Box::new(c), vec![]))?),
                None => None,
            };
            let not = match inverse {
                Some(Expr::Function(_)) => return Err("makes a timer whose inverse condition is written in place".into()),
                Some(c) => Some(format!("!({})", self.test(&Expr::Call(Box::new(c), vec![]))?)),
                None => None,
            };
            let test = match (test, not) { (Some(a), Some(b)) => Some(format!("({a}) && {b}")), (a, b) => a.or(b) };
            let call = match &test { Some(t) => format!("if ({t}) {name}()"), None => format!("{name}()") };
            let now = self.in_frame && n.parse::<f64>().is_ok_and(|v| v.round() <= 1.0);
            let left = if !now { times.clone() } else { match times.parse::<f64>() {
                Ok(v) if v < 0.0 => "-1".into(),
                Ok(v) => number(v - 1.0),
                Err(_) => format!("({times} < 0 ? -1 : {times} - 1)"),
            } };
            let finite = left != "-1";
            let tick = if finite {
                self.kept.insert(format!("{id}Left"), format!("// the ticks {name}'s timer has left\nvar {id}Left = self.makeInt(0);"));
                format!("function() {{ {id}Left.set({id}Left.get() - 1); if ({id}Left.get() == 0) {id}.set(-1); {call}; }}")
            } else { match &test { Some(t) => format!("function() {{ if ({t}) {name}(); }}"), None => name.clone() } };
            let make = format!("{id}.set(self.addTimer({every}, {left}, {tick}, {{ persistent: {keep}, pauseCondition: frozen }}))");
            let make = match left.parse::<f64>() {
                Ok(0.0) => format!("{id}.set(-1)"),
                Ok(_) if finite => format!("{id}Left.set({left});\n{make}"),
                Ok(_) => make,
                Err(_) => format!("{id}.set(-1);\nif ({left} != 0) {{\n\t{id}Left.set({left});\n\t{make};\n}}"),
            };
            let first = if now { format!(";\n{call}") } else { String::new() };
            // (a longer one made in a frame's code is due a source frame sooner the first time, then
            // on its period: one engine timer made as the source makes it, ticking every frame and
            // counting down to each tick, so timers that come due on one frame run in the order
            // they were made, as the source's do)
            let longer = n.parse::<f64>().ok().filter(|v| self.in_frame && v.round() > 1.0);
            if let Some(v) = longer {
                let opts = format!("{{ persistent: {keep}, pauseCondition: frozen }}");
                let due = format!("{id}Due");
                self.kept.insert(due.clone(), format!("// the engine frames until {name}'s timer next ticks\nvar {due} = self.makeInt(0);"));
                let stop = format!("{{ self.removeTimer({id}.get()); {id}.set(-1); }}");
                let body = if finite { format!("{id}Left.set({id}Left.get() - 1); if ({id}Left.get() == 0) {stop} {call};") } else { format!("{call};") };
                let ticker = format!("function() {{ {due}.set({due}.get() - 1); if ({due}.get() > 0) return; {due}.set({every}); {body} }}");
                let make = format!("{due}.set({});\n{id}.set(self.addTimer(1, -1, {ticker}, {opts}))", number(2.0 * v.round() - 2.0));
                let make = if finite { format!("{id}Left.set({left});\n{make}") } else { make };
                return Ok((format!("self.removeTimer({id}.get());\n{make}"), Ty::Any));
            }
            return Ok((format!("self.removeTimer({id}.get());\n{make}{first}"), Ty::Any));
        }
        let opts = "{ persistent: true }";
        Ok((format!("self.removeTimer({id}.get());\n{id}.set(self.addTimer({every}, {times}, {name}, {opts}))"), Ty::Any))
    }

    /// a listener for an event the engine raises, on the engine's object, by one of its own methods
    fn listener(&mut self, who: &str, m: &str, ev: &Expr, handler: &Expr) -> Result<(String, Ty), String> {
        // (a closure written in place: kept under a name of the script's, set as it is listened with,
        // so the notes and calls its listening takes have a name to go by)
        if let Expr::Function(f) = handler {
            let c = self.closure(f)?;
            if !c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
                let mut hsh: u64 = 0xcbf29ce484222325;
                for b in c.bytes() { hsh = (hsh ^ b as u64).wrapping_mul(0x100000001b3); }
                let id = format!("listener{:x}", hsh & 0xffffff);
                self.kept.insert(id.clone(), format!("// a closure its code listens with, kept under a name\nvar {id} = null;"));
                let had = self.captured.insert(id.clone());
                let r = self.listener(who, m, ev, &Expr::Name(id.clone()));
                if !had { self.captured.remove(&id); }
                let (t, ty) = r?;
                return Ok((if m == "addEventListener" { format!("{id} = {c};\n{t}") } else { t }, ty));
            }
        }
        let name = super::events::named(self.o.runtime, ev).ok_or("listens for an event named as it runs")?;
        // its homing target picked, a knock-out it is credited with, an attack of its cut off by
        // another: told by the pick, the knock-out, the start (`homingPick`, `koPoint`, `attackCancel`)
        // (a fighter's size status changed, on whichever fighter it listens: told by the change, `sizeStatus`)
        if name == "charSizeChange" {
            let h = self.handler(handler)?;
            self.util("sizeStatus")?;
            let f = if who == "self" && !self.o.fighter { "self.getRootOwner()".to_string() } else { who.to_string() };
            return Ok((format!("sizeListen({f}, {h}, {})", m == "addEventListener"), Ty::Any));
        }
        // (a knock-out point of a fighter's, on whichever fighter it listens)
        if name == "charKOPoint" {
            let h = self.handler(handler)?;
            self.util("koPoint")?;
            let f = if who == "self" && !self.o.fighter { "self.getRootOwner()".to_string() } else { who.to_string() };
            return Ok((if m == "addEventListener" { format!("koPointListen({f}, {h})") } else { format!("koPointUnlisten({f}, {h})") }, Ty::Any));
        }
        if who == "self" && matches!(name.as_str(), "homingTarget" | "charAttackCancelled") && (self.o.fighter || name == "homingTarget") {
            let h = self.handler(handler)?;
            let (util, list, listen) = match name.as_str() {
                "homingTarget" => ("homingPick", "homingHeard", None),
                _ => ("attackCancel", "attackCancelHeard", Some("attackCancelListen")),
            };
            self.util(util)?;
            return Ok((match (m == "addEventListener", listen) {
                (true, Some(l)) => format!("{l}({h})"),
                (true, None) => format!("{list}.remove({h});\n{list}.push({h})"),
                (false, _) => format!("{list}.remove({h})"),
            }, Ty::Any));
        }
        // an attack of its turned back on (`attackEnabled`): told by the turn (`allowAttack`)
        if name == "attackEnabled" && who == "self" && self.o.fighter {
            let h = self.handler(handler)?;
            self.blocks.insert("attack".into()); self.blocks.insert("actions".into());
            return Ok((if m == "addEventListener" { format!("attackEnabledHeard.get().remove({h});\nattackEnabledHeard.get().push({h})") } else { format!("attackEnabledHeard.get().remove({h})") }, Ty::Any));
        }
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
        if matches!(name.as_str(), "projDestroyed" | "itemDestroyed" | "enemyDestroyed") && who != "self" {
            let h = self.handler(handler)?;
            return Ok((if m == "addEventListener" { format!("{who}.addEventListener(EntityEvent.DISPOSE, {h}, {{ persistent: true }})") } else { format!("{who}.removeEventListener(EntityEvent.DISPOSE, {h})") }, Ty::Any));
        }
        // one its item handler raises: its own list of listeners, which the handler tells
        if self.o.item && who == "self" {
            if let Some(list) = super::events::table().item.get(&name).cloned() {
                let h = self.handler(handler)?;
                return Ok((if m == "addEventListener" { format!("{list}.get().remove({h});\n{list}.get().push({h})") } else { format!("{list}.get().remove({h})") }, Ty::Any));
            }
        }
        // another object's every frame (a clip's `enterFrame`): a timer of that object's, each source
        // frame, ending with it
        if name == "enterFrame" && who != "self" {
            let h = self.handler(handler)?;
            let id = format!("{h}__frames");
            self.kept.insert(id.clone(), format!("// the timer that runs {h} each frame of the object it listens on\nvar {id} = self.makeInt(-1);"));
            return Ok((if m == "addEventListener" {
                format!("if ({who} != null && !{who}.isDisposed()) {id}.set({who}.addTimer(2, -1, {h}, {{ persistent: true }}))")
            } else {
                format!("if ({who} != null && !{who}.isDisposed() && {id}.get() >= 0) {who}.removeTimer({id}.get());\n{id}.set(-1)")
            }, Ty::Any));
        }
        // its own damage's updates: from what it is as its code begins listening (`damageStep`)
        if name == "damageChanged" && who == "self" && m == "addEventListener" && !self.primed {
            self.util("damageStep")?;
            self.primed = true;
            let r = self.listener(who, m, ev, handler);
            self.primed = false;
            return Ok((format!("damagePrime();\n{}", r?.0), Ty::Any));
        }
        // a fighter turned metal or back (`charMetalChange`): told by the turn (`metalOn`)
        if name == "charMetalChange" {
            let h = self.handler(handler)?;
            return Ok((if m == "addEventListener" {
                format!("if ({who} != null) {{ if ({who}.exports.__metalHeard == null) {who}.exports.__metalHeard = []; {who}.exports.__metalHeard.remove({h}); {who}.exports.__metalHeard.push({h}); }}")
            } else {
                format!("if ({who} != null && {who}.exports.__metalHeard != null) {who}.exports.__metalHeard.remove({h})")
            }, Ty::Any));
        }
        // a fighter turned into its other form (`charTransform`): told by the turn (`finalForm`)
        if name == "charTransform" {
            let h = self.handler(handler)?;
            return Ok((if m == "addEventListener" {
                format!("if ({who} != null) {{ if ({who}.exports.__transformHeard == null) {who}.exports.__transformHeard = []; {who}.exports.__transformHeard.remove({h}); {who}.exports.__transformHeard.push({h}); }}")
            } else {
                format!("if ({who} != null && {who}.exports.__transformHeard != null) {who}.exports.__transformHeard.remove({h})")
            }, Ty::Any));
        }
        // an item it holds thrown (`itemTossed` heard on the fighter): told by the item's handler as
        // it lets the item go
        if name == "itemTossed" && !(self.o.item && who == "self") {
            let h = self.handler(handler)?;
            return Ok((if m == "addEventListener" {
                format!("if ({who} != null) {{ if ({who}.exports.__tossHeard == null) {who}.exports.__tossHeard = []; {who}.exports.__tossHeard.remove({h}); {who}.exports.__tossHeard.push({h}); }}")
            } else {
                format!("if ({who} != null && {who}.exports.__tossHeard != null) {who}.exports.__tossHeard.remove({h})")
            }, Ty::Any));
        }
        // a fighter knocked out by its own fall, with no one's hit behind it: the engine's knock-out,
        // told only when the fighter's own count of them goes up (`selfDestructHeard`)
        if name == "charSelfDestruct" {
            let h = self.handler(handler)?;
            self.util("selfDestructHeard")?;
            return Ok((if m == "addEventListener" { format!("selfDestructHeard({who}, {h})") } else { format!("selfDestructDrop({who}, {h})") }, Ty::Any));
        }
        let ends = (name == "projDestroyed" || (self.o.item && name == "itemDestroyed")) && who == "self";
        let table = super::events::table();
        let raised = table.engine.get(&name).or_else(|| table.state.get(&name));
        // (one neither this engine nor the native skeletons ever raise is heard by no one)
        if raised.is_none() && !ends && !self.o.runtime.files.values().any(|t| t.contains(&format!("\"{name}\""))) {
            // (one the source's engine never raises either is never told there: as it is here)
            if super::events::table().never_raised.contains(&name) {
                return Ok((format!("// (it listens for the source's {name}, which its engine never raises)"), Ty::Any));
            }
            self.gaps.insert(format!("listens for {name}, which nothing here raises"));
            return Ok((format!("// (it listens for the source's {name}, which nothing here raises)"), Ty::Any));
        }
        let engine = if ends { &name } else { raised.ok_or_else(|| format!("listens for {name}, which this engine does not raise"))? };
        let h = self.handler(handler)?;
        // (heard on another object: the event's caller, in that handler, is that object)
        if who != "self" { self.foreign_handlers.insert(h.clone()); }

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
            // (told plainly, the listener is itself; told with the speed it came down at, a function that does;
            // a fighter's, a source frame after it touches down, as the source tells a fighter)
            let wrapper = if self.o.fighter { format!("{h}Landed") } else if self.floor_speed { format!("{h}OnFloor") } else { h.clone() };
            self.floor.insert(h.clone(), listening.clone());
            self.kept.insert(listening.clone(), format!("// whether {h} listens for the floor now\nvar {listening} = self.{}(false);", bool_make(self.o.fighter)));
            if self.o.fighter {
                let def = helper(self.o.runtime, "helpers", "fighter_floor_listener", &wrapper, &[("LISTENER", h.clone()), ("LISTENING", listening.clone())], &[])?;
                self.helpers.insert(wrapper.clone(), def);
            } else if self.floor_speed {
                let def = helper(self.o.runtime, "helpers", "floor_listener", &wrapper, &[("LISTENER", h.clone())], &[])?;
                self.helpers.insert(wrapper.clone(), def);
            }
            let (opts, owned, unowned) = self.clip_listen(engine, &wrapper);
            return Ok((if m == "addEventListener" {
                format!("self.addEventListener({engine}, {wrapper}, {opts});\n{listening}.set(true){owned}")
            } else {
                format!("self.removeEventListener({engine}, {wrapper});\n{listening}.set(false){unowned}")
            }, Ty::Any));
        }
        // (a note of whether it listens, for its code's question: `hasEventListener`)
        let note = if who == "self" && self.o.fighter {
            let listening = format!("{h}Listening");
            self.kept.insert(listening.clone(), format!("// whether {h} listens now\nvar {listening} = self.{}(false);", bool_make(self.o.fighter)));
            format!(";\n{listening}.set({})", m == "addEventListener")
        } else { String::new() };
        let (opts, owned, unowned) = if who == "self" { self.clip_listen(engine, &h) } else { ("{ persistent: true }", String::new(), String::new()) };
        // (one put on an object a field of its keeps: taken back as it ends, so it is never told once
        // this one is gone)
        if who != "self" && m == "addEventListener" && !self.o.fighter && who.ends_with(".get()") && who[..who.len() - 6].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            self.release.insert(format!("if ({who} != null && !{who}.isDisposed()) {who}.removeEventListener({engine}, {h});"));
        }
        // (a projectile's own hit listener: told too of the hits it landed before its first frame ran)
        // (only one its first frame adds: a later one is listening before any hit it could hear)
        let early = if who == "self" && !self.o.fighter && !self.o.stage && !self.o.item && self.current_fn.ends_with("frame1")
            && matches!(engine.as_str(), "GameObjectEvent.HIT_DEALT" | "GameObjectEvent.SHIELD_HIT_DEALT") {
            self.util("heardEarly")?;
            format!(";\nheardEarly({engine}, {h})")
        } else { String::new() };
        Ok((if m == "addEventListener" {
            format!("{who}.addEventListener({engine}, {h}, {opts}){note}{owned}{early}")
        } else {
            format!("{who}.removeEventListener({engine}, {h}){note}{unowned}")
        }, Ty::Any))
    }

    /// a named clip the clip its animation shows places (`getStanceMC().name`), whose own frames
    /// run no code: its name, and each animation whose clip places it with the runs of the
    /// animation's source frames it is on (none where it is on all of them)
    fn stance_child(&mut self, e: &Expr) -> Option<(String, Vec<(String, Vec<(usize, usize)>)>)> {
        if !self.o.fighter { return None }
        let Expr::Member(mc, name) = e else { return None };
        let Expr::Call(g, a) = &**mc else { return None };
        if !a.is_empty() || !matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "getStanceMC" | "stanceClip") && self.expr(o).is_ok_and(|(w, _)| w == "self")) { return None }
        let mut out: Vec<(String, Vec<(usize, usize)>)> = Vec::new();
        for (anim, sym, from, n) in self.o.anim_clips {
            let Some(sp) = self.o.model.sprites.get(sym) else { continue };
            let mut on = Vec::new();
            for f in sp.frames.iter().skip(from.saturating_sub(1) as usize).take(*n as usize) {
                let here: Vec<&crate::swf_timeline::Placed> = f.iter().filter(|p| p.name.as_deref() == Some(name.as_str())).collect();
                // (one whose own frames run code is played here as its code says: not this)
                if here.iter().any(|p| !art_only(self.o.model, p.char_id)) { return None }
                on.push(!here.is_empty());
            }
            if !on.iter().any(|x| *x) || out.iter().any(|(a, _)| a == anim) { continue }
            let mut runs = Vec::new();
            if !on.iter().all(|x| *x) {
                let mut i = 0;
                while i < on.len() {
                    if on[i] { let s = i; while i < on.len() && on[i] { i += 1; } runs.push((s, i)); } else { i += 1; }
                }
            }
            out.push((anim.clone(), runs));
        }
        (!out.is_empty()).then(|| (name.clone(), out))
    }

    /// a named clip its own clip places (`this.name`, in a clip's code), whose own frames run no code
    /// whether a value is an effect its code keeps: a field every write of which makes an effect
    fn field_holds_effect(&self, e: &Expr) -> bool {
        let Expr::Member(t, f) = e else { return false };
        if !matches!(**t, Expr::This) { return false }
        let Some(c) = self.o.model.classes.get(&self.class) else { return false };
        let mut any = false;
        let mut only = true;
        for body in c.methods.iter().map(|m| &m.function.body) {
            walk(body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, g), v) = st {
                if g != f { return }
                let effect = matches!(v, Expr::Call(h, _) if matches!(&**h, Expr::Member(_, m) if matches!(m.as_str(), "createVfx" | "attachEffect" | "lockEffect")));
                if effect { any = true } else if !matches!(v, Expr::Null | Expr::Undefined) { only = false }
            });
        }
        any && only
    }

    /// whether some effect of its places a clip of this name that its code turns
    fn is_inner(&self, n: &str) -> bool { self.o.effect_inners.values().flatten().any(|c| c.name == n) }

    /// a clip it holds, for a colour set on it: one of its parts, or a value (an effect)
    fn held_clip(&mut self, x: &Expr) -> Result<String, String> {
        if let Some(name) = self.part_of(x) { return Ok(format!("partNow({})", part_id(&name))) }
        if matches!(x, Expr::This) || self.is_clip(x) { return Err("sets a colour on a clip of its own clip".into()) }
        Ok(self.expr(x)?.0)
    }

    /// its costumes' colours, as the source reads them (`getPaletteSwapData`)
    fn costume_palette(&mut self) -> Result<String, String> {
        if !self.helpers.contains_key("costumePalette") {
            let Some((colors, maps)) = self.o.palette.clone() else { return Err("reads its costumes' colours, which this conversion has none of".into()) };
            // (as the source's numbers are: unsigned, so a colour its code compares with one it
            // writes out matches; a hex literal here is a 32-bit int, negative past 0x7FFFFFFF)
            let hex = |v: &[u32]| v.iter().map(|c| if *c > 0x7FFF_FFFF { format!("{c}.0") } else { format!("0x{c:08X}") }).collect::<Vec<_>>().join(", ");
            let lines = vec![format!("var colors = [{}];", hex(&colors)), "var costumes = [".to_string()]
                .into_iter().chain(maps.iter().map(|m| format!("\t[{}],", hex(m)))).chain(["];".to_string()]).collect::<Vec<_>>();
            self.helpers.insert("costumePalette".into(), helper(self.o.runtime, "helpers", "costume_palette", "costumePalette", &[], &lines)?);
        }
        Ok("costumePalette()".into())
    }

    /// one of its parts, as the code reaches it (its own field, or its stance clip's)
    fn part_of(&mut self, e: &Expr) -> Option<String> {
        if !self.o.fighter { return None }
        let Expr::Member(t, name) = e else { return None };
        // (one the animation places under two clips: the one inside this clip, by its name)
        let name = match &**t {
            Expr::Member(_, parent) if self.o.parts.iter().any(|p| p.name == format!("{parent}_{name}")) => format!("{parent}_{name}"),
            _ => name.clone(),
        };
        let name = &name;
        if !self.o.parts.iter().any(|p| &p.name == name) { return None }
        let root = |w: &mut Self, t: &Expr| matches!(t, Expr::This) || w.is_clip(t)
            || matches!(t, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "getStanceMC" | "stanceClip") && w.expr(o).is_ok_and(|(x, _)| x == "self")));
        // (or inside a clip its own clip places, by the names its clips place it under:
        // `stance.aura1.cspark`, the part drawn apart with its place composed down to it)
        let mut at: &Expr = t;
        while let Expr::Member(o, _) = at { if root(self, at) { break } at = o; }
        root(self, at).then(|| name.clone())
    }

    fn own_child(&self, e: &Expr) -> Option<String> {
        if !self.o.fighter { return None }
        let Expr::Member(t, name) = e else { return None };
        if !matches!(&**t, Expr::This) && !matches!(&**t, Expr::Local(r) if self.locals.get(r).is_some_and(|(_, h)| *h == Holds::Clip)) { return None }
        let sym = self.o.model.classes.get(&self.class)?.symbol?;
        let sp = self.o.model.sprites.get(&sym)?;
        let placed: Vec<&crate::swf_timeline::Placed> = sp.frames.iter().flatten().filter(|p| p.name.as_deref() == Some(name.as_str())).collect();
        if placed.is_empty() || placed.iter().any(|p| !art_only(self.o.model, p.char_id)) { return None }
        Some(name.clone())
    }

    /// whether a value is a clip an object's own clip places (its own, through names that are no
    /// field of its, or another object's, through `getStanceMC()` / `getMC()`)
    fn nested_in_drawn(&mut self, e: &Expr) -> bool {
        match e {
            Expr::This => true,
            // (a field its code never writes is the placed clip Flash binds to it)
            Expr::Member(o, n) => (self.field(n).is_none() || !self.writes_field(n)) && !self.is_inner(n) && self.nested_in_drawn(o),
            Expr::Call(g, a) => a.is_empty() && matches!(&**g, Expr::Member(_, m) if matches!(m.as_str(), "getStanceMC" | "stanceClip" | "getMC")),
            _ => false,
        }
    }

    /// a field of an object's (not a fighter's or a stage's) its code never writes that names a
    /// clip its clip places, which Flash binds to it: where that placement first puts it (x, y,
    /// rotation in degrees, scales)
    /// one of its parts (a named child of its clip its code turns, drawn apart), as the record its
    /// code holds (`projPart`)
    fn own_part(&mut self, n: &str) -> Option<String> {
        if self.o.fighter || self.o.stage || self.writes_field(n) { return None }
        let p = self.o.own_parts.iter().find(|p| p.path == n)?.clone();
        self.util("projPart").ok()?;
        Some(part_call(&p))
    }

    /// whether any class of the package changes a clip its clip places under this name: writes a
    /// property of it, or sends it to a frame
    fn turns_placed(&self, n: &str) -> bool {
        let is = |e: &Expr| matches!(e, Expr::Member(t, f) if f == n && matches!(**t, Expr::This)) || matches!(e, Expr::Name(f) if f == n);
        let mut found = false;
        for c in self.o.model.classes.values() {
            for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor]) {
                walk(&f.body, &mut |st| if let Stmt::Assign(Place::Member(o, _), _) = st { if is(o) { found = true } });
                walk_exprs(&f.body, &mut |e| if let Expr::Call(g, _) = e {
                    if matches!(&**g, Expr::Member(o, m) if is(o) && matches!(m.as_str(), "gotoAndStop" | "gotoAndPlay" | "play" | "stop" | "nextFrame" | "prevFrame")) { found = true }
                });
                if found { return true }
            }
        }
        false
    }

    fn placed_child(&self, n: &str) -> Option<(f64, f64, f64, f64, f64)> {
        if self.o.fighter || self.o.stage || self.field(n).is_none() || self.writes_field(n) { return None }
        let sym = self.o.model.classes.get(&self.class).and_then(|c| c.symbol)?;
        let p = self.o.model.sprites.get(&sym)?.frames.iter().flatten().find(|p| p.name.as_deref() == Some(n))?;
        let m = &p.matrix;
        Some((m.tx, m.ty, m.b.atan2(m.a).to_degrees(), m.a.hypot(m.b), m.c.hypot(m.d)))
    }

    /// whether a value is one of its own clips by the API's call (`getMC()`, `getStanceMC()`), on
    /// itself or bare
    fn own_clip_call(&self, e: &Expr) -> bool {
        let Expr::Call(g, a) = e else { return false };
        a.is_empty() && match &**g {
            Expr::Member(t, m) => matches!(**t, Expr::This) && matches!(m.as_str(), "getMC" | "getStanceMC" | "stanceClip"),
            Expr::Name(m) => matches!(m.as_str(), "getMC" | "getStanceMC" | "stanceClip") && self.method(m).is_none(),
            _ => false,
        }
    }

    /// the record written out in place a value is: itself, or the one a variable of the function
    /// is only ever set to
    fn literal_of(&self, e: &Expr) -> Option<Vec<(Expr, Expr)>> {
        match e {
            Expr::Object(kv) => Some(kv.clone()),
            Expr::Local(r) => {
                let mut found: Option<Vec<(Expr, Expr)>> = None;
                let mut other = false;
                walk(&self.body, &mut |st| if let Stmt::Assign(Place::Local(x), v) = st {
                    if x != r { return }
                    let mut v = v;
                    while let Expr::Coerce(_, y) | Expr::Convert(_, y) = v { v = y; }
                    match v { Expr::Object(kv) if found.is_none() => found = Some(kv.clone()), Expr::Null | Expr::Undefined => {}, _ => other = true }
                });
                if other { None } else { found }
            }
            // (a field of its own only ever set to one, as its class is made)
            Expr::Member(t, f) if matches!(**t, Expr::This) => {
                let c = self.o.model.classes.get(&self.class)?;
                let mut found: Option<Vec<(Expr, Expr)>> = None;
                let mut other = false;
                for b in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
                    walk(b, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, g), v) = st {
                        if g != f { return }
                        let mut v = v;
                        while let Expr::Coerce(_, y) | Expr::Convert(_, y) = v { v = y; }
                        match v { Expr::Object(kv) if found.is_none() => found = Some(kv.clone()), Expr::Null | Expr::Undefined => {}, _ => other = true }
                    });
                }
                if other { None } else { found }
            }
            _ => None,
        }
    }

    /// whether every value its code pushes onto a list field of its own is a record: a library clip
    /// made by name, a record written out, or a field of its own only ever set to one of those
    fn pushes_records(&self, fl: &str) -> bool {
        let Some(c) = self.o.model.classes.get(&self.class) else { return false };
        let made = |e: &Expr| matches!(e, Expr::Object(_)) || matches!(e, Expr::Call(g, a) if a.len() == 1 && matches!(&**g, Expr::Member(_, m) | Expr::Name(m) if matches!(m.as_str(), "getMCByLinkageName" | "clipByLinkage")));
        let field_made = |g: &str| {
            let mut any = false; let mut all = true;
            for b in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
                walk(b, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, h), v) = st { if h == g { if made(v) { any = true } else if !matches!(v, Expr::Null | Expr::Undefined) { all = false } } });
            }
            any && all
        };
        let mut pushed: Vec<Expr> = Vec::new();
        for b in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
            walk_exprs(b, &mut |e| if let Expr::Call(g, a) = e {
                if let (Expr::Member(l, m), [x]) = (&**g, a.as_slice()) {
                    if m == "push" && matches!(&**l, Expr::Member(t, f) if matches!(**t, Expr::This) && f == fl) { pushed.push(x.clone()); }
                }
            });
        }
        !pushed.is_empty() && pushed.iter().all(|x| made(x) || matches!(x, Expr::Member(t, g) if matches!(**t, Expr::This) && field_made(g)))
    }

    /// whether the stage's own class keeps a field of this name (a package with no stage class of
    /// its own, lent to the stage, reaches whatever the stage publishes)
    fn stage_has_field(&self, f: &str) -> bool {
        let stages: Vec<&ClassModel> = self.o.model.classes.values().filter(|c| short(&c.super_name) == "PeptideStage").collect();
        stages.is_empty() || stages.iter().any(|c| c.fields.iter().any(|x| x.name == f && !x.is_static) || c.methods.iter().any(|m| { let mut set = false; walk(&m.function.body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, g), _) = st { if g == f { set = true } }); set }))
    }

    /// whether a value is the stage: the API's, or a variable of the function only ever set to it
    fn is_stage_value(&mut self, e: &Expr) -> bool {
        if self.expr(e).is_ok_and(|(w, _)| w == "stage") { return true }
        let Expr::Local(r) = e else { return false };
        if self.stage_locals.contains(r) { return true }
        let mut set: Vec<Expr> = Vec::new();
        walk(&self.body, &mut |st| if let Stmt::Assign(Place::Local(x), v) = st { if x == r { set.push(v.clone()) } });
        let set: Vec<Expr> = set.into_iter().filter(|v| !matches!(v, Expr::Null | Expr::Undefined)).collect();
        !set.is_empty() && set.iter().all(|v| !matches!(v, Expr::Local(y) if y == r) && self.is_stage_value(v))
    }

    /// the class of its package (its own, or the one named) that declares a static variable (or a
    /// constant) of this name
    fn static_owner(&self, class: Option<&str>, n: &str) -> Option<String> {
        let has = |c: &ClassModel| c.fields.iter().any(|x| x.name == n && (x.is_static || x.is_const));
        match class {
            Some(cn) => self.o.model.classes.values().find(|c| (c.name == cn || short(&c.name) == short(cn)) && has(c)).map(|c| c.name.clone()),
            None => {
                // (its own class, then the package classes it extends)
                let mut cur = self.o.model.classes.get(&self.class);
                while let Some(c) = cur {
                    if has(c) { return Some(c.name.clone()) }
                    cur = self.o.model.classes.get(&c.super_name);
                }
                None
            }
        }
    }

    /// whether any code of its package writes a class's static variable as it runs (beyond the class's
    /// own setup)
    fn static_written(&self, class: &str, n: &str) -> bool {
        let mut any = false;
        for k in self.o.model.classes.values() {
            let own = k.name == class;
            for f in k.methods.iter().map(|m| &m.function).chain([&k.constructor]) {
                walk(&f.body, &mut |st| match st {
                    Stmt::Assign(Place::Name(g), _) if own && g == n => any = true,
                    Stmt::Assign(Place::Member(Expr::Name(c), g), _) if g == n && (c == class || short(c) == short(class)) => any = true,
                    _ => {}
                });
            }
        }
        any
    }

    /// whether any code of its package reads a member of this name (anything's)
    fn read_anywhere(&self, n: &str) -> bool {
        let mut read = false;
        for k in self.o.model.classes.values() {
            for f in k.methods.iter().map(|m| &m.function).chain([&k.constructor, &k.static_init]) {
                crate::model::ast::walk_exprs(&f.body, &mut |e| if matches!(e, Expr::Member(_, m) if m == n) { read = true });
            }
        }
        read
    }

    /// whether any code of the class being written writes one of its fields
    fn never_filled(&self, f: &str) -> bool {
        if self.field(f).is_none() || self.writes_field(f) || self.placed_child(f).is_some() { return false }
        // (nor by another object's code, on it)
        let mut any = false;
        for k in self.o.model.classes.values() {
            for b in k.methods.iter().map(|m| &m.function.body).chain([&k.constructor.body, &k.static_init.body]) {
                walk(b, &mut |st| if let Stmt::Assign(Place::Member(_, g), _) = st { if g == f { any = true } });
            }
        }
        // (and declared empty)
        let empty = self.o.model.classes.get(&self.class).and_then(|c| c.fields.iter().find(|x| x.name == f)).is_none_or(|x| x.default.as_ref().is_none_or(|d| matches!(d, Expr::Null | Expr::Undefined)));
        !any && empty
    }

    /// whether any code of the class being written writes one of its fields (`never_filled`: one no
    /// code fills at all, which holds what it is declared with)
    fn writes_field(&self, n: &str) -> bool {
        let Some(c) = self.o.model.classes.get(&self.class) else { return true };
        let mut any = false;
        for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor]) {
            walk(&f.body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, g), _) = st { if g == n { any = true } });
        }
        any
    }

    /// whether a value is the clip a fighter's own animation shows: the call itself, or a field of its
    /// own only ever set to it
    /// where the tree places the clip of the class being written, where every placement agrees
    fn one_placement(&self) -> Option<(f64, f64)> {
        let sym = self.o.model.classes.get(&self.class)?.symbol?;
        let mut at: Option<(f64, f64)> = None;
        for sp in self.o.model.sprites.values() {
            for p in sp.frames.iter().flatten().filter(|p| p.char_id == sym) {
                let here = (p.matrix.tx, p.matrix.ty);
                match at { None => at = Some(here), Some(a) if (a.0 - here.0).abs() > 0.01 || (a.1 - here.1).abs() > 0.01 => return None, _ => {} }
            }
        }
        at
    }

    fn own_stance_value(&mut self, e: &Expr) -> bool {
        if !self.o.fighter { return false }
        if self.own_stance(e) { return true }
        let Expr::Member(t, f) = e else { return false };
        if !matches!(**t, Expr::This) { return false }
        let Some(c) = self.o.model.classes.get(&self.class) else { return false };
        let mut set: Vec<Expr> = Vec::new();
        for b in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
            walk(b, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, g), v) = st { if g == f { set.push(v.clone()) } });
        }
        !set.is_empty() && set.iter().all(|v| matches!(v, Expr::Null | Expr::Undefined) || self.own_stance(v))
    }

    /// the clips the clip a fighter's animation shows places under names, each a record kept by its
    /// name (drawn into the fighter's art): a function of its own script
    fn stance_kids(&mut self) -> String {
        let f = "stanceKids".to_string();
        if self.helpers.contains_key(&f) { return f }
        let mut cases: Vec<String> = Vec::new();
        for (anim, sym, from, n) in self.o.anim_clips {
            let Some(sp) = self.o.model.sprites.get(sym) else { continue };
            let mut names: Vec<String> = Vec::new();
            for fr in sp.frames.iter().skip(from.saturating_sub(1) as usize).take(*n as usize) {
                for p in fr { if let Some(nm) = p.name.as_deref() { if !names.iter().any(|x| x == nm) { names.push(nm.to_string()) } } }
            }
            if names.is_empty() { continue }
            let q = quote(anim);
            if cases.iter().any(|c| c.starts_with(&format!("\t\tcase {q}:"))) { continue }
            cases.push(format!("\t\tcase {q}: names = [{}];", names.iter().map(|x| quote(x)).collect::<Vec<_>>().join(", ")));
        }
        // (one drawn apart as a part: the look its part is placed with, `partLook`)
        let part_names: Vec<String> = self.o.parts.iter().filter(|p| !p.placed.is_empty()).map(|p| quote(&p.name)).collect();
        let part_pick = if part_names.is_empty() { String::new() } else { format!("[{}].indexOf(n) >= 0 ? partLook(n) : ", part_names.join(", ")) };
        self.kept.insert("stanceKidRecs".into(), "// the clips its animations' clip places under names, as records (drawn into its art)\nvar stanceKidRecs = self.makeObject(null);".into());
        if part_names.is_empty() { self.gaps.insert("turns the clips its clip places, read by their place in it, whose art is drawn into the fighter's".into()); }
        self.helpers.insert(f.clone(), format!("// the clips the clip its animation shows places under names, as records kept by name\nfunction stanceKids() {{\n\tvar names = [];\n\tswitch (self.getAnimation()) {{\n{}\n\t}}\n\tif (stanceKidRecs.get() == null) stanceKidRecs.set(new haxe.ds.StringMap());\n\tvar m = stanceKidRecs.get();\n\tvar out = [];\n\tfor (n in names) {{\n\t\tif (!m.exists(n)) m.set(n, {}{{ name: n, x: 0.0, y: 0.0, scaleX: 1.0, scaleY: 1.0, alpha: 1.0, rotation: 0.0, visible: true }});\n\t\tout.push(m.get(n));\n\t}}\n\treturn out;\n}}", cases.join("\n"), part_pick));
        f
    }

    /// whether a value is the clip its own animation shows (`getStanceMC()` on itself)
    fn own_stance(&mut self, e: &Expr) -> bool {
        let Expr::Call(g, a) = e else { return false };
        a.is_empty() && matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "getStanceMC" | "stanceClip") && self.expr(o).is_ok_and(|(w, _)| w == "self"))
    }

    /// where the clip its animation shows sits in its root (`own_stance`), or a named clip that clip
    /// places sits in it (`stance_child`), in the source's units: a function of its own script, by
    /// the animation it plays and the source frame it is on
    fn placed_at(&mut self, e: &Expr, x: bool) -> Result<String, String> {
        let axis = if x { "X" } else { "Y" };
        let pick = |m: &crate::swf_timeline::Matrix| if x { m.tx } else { m.ty };
        let (name, f) = if self.own_stance(e) { ("stance".to_string(), format!("stance{axis}")) } else {
            let (n, _) = self.stance_child(e).ok_or("reads a clip its clip places")?;
            let f = format!("{}{axis}", ident(&n));
            (n, f)
        };
        if self.helpers.contains_key(&f) { return Ok(format!("{f}()")) }
        let mut cases: Vec<String> = Vec::new();
        for (anim, sym, from, n) in self.o.anim_clips {
            let vals: Vec<f64> = if name == "stance" {
                // (the root's placement of this clip, wherever the root places it)
                let at = self.o.model.sprites.values().flat_map(|s| s.frames.iter().flatten())
                    .find(|p| p.char_id == *sym && p.name.as_deref() == Some("stance")).map(|p| pick(&p.matrix)).unwrap_or(0.0);
                vec![at]
            } else {
                let Some(sp) = self.o.model.sprites.get(sym) else { continue };
                let on: Vec<Option<f64>> = sp.frames.iter().skip(from.saturating_sub(1) as usize).take(*n as usize)
                    .map(|fr| fr.iter().find(|p| p.name.as_deref() == Some(name.as_str())).map(|p| pick(&p.matrix))).collect();
                if on.iter().all(|v| v.is_none()) { continue }
                // (a frame it is not on reads where it last was, else where it first is)
                let first = on.iter().flatten().next().copied().unwrap_or(0.0);
                let mut last = first;
                on.iter().map(|v| { if let Some(v) = v { last = *v } last }).collect()
            };
            let q = quote(anim);
            if cases.iter().any(|c| c.starts_with(&format!("\t\tcase {q}:"))) { continue }
            if vals.iter().all(|v| *v == vals[0]) {
                if vals[0] != 0.0 { cases.push(format!("\t\tcase {q}: return {};", number(vals[0]))); }
            } else {
                let list: Vec<String> = vals.iter().map(|v| number(*v)).collect();
                cases.push(format!("\t\tcase {q}: var l = [{}]; return l[k < l.length ? k : l.length - 1];", list.join(", ")));
            }
        }
        let mut lines = vec![format!("// where its {name} clip sits ({axis}), by the animation it plays and its source frame"), format!("function {f}() {{")];
        if cases.iter().any(|c| c.contains("var l")) { lines.push("\tvar k = Std.int((self.getCurrentFrame() - 1) / 2);".into()); }
        if !cases.is_empty() {
            lines.push("\tswitch (self.getAnimation()) {".into());
            lines.extend(cases);
            lines.push("\t}".into());
        }
        lines.push("\treturn 0;".into());
        lines.push("}".into());
        self.helpers.insert(f.clone(), lines.join("\n"));
        Ok(format!("{f}()"))
    }

    /// the test whether the clip its animation shows places a named clip: a function of its own
    /// script, by the animations whose clip does (and the source frames of the animation it is on,
    /// where not all)
    fn child_test(&mut self, e: &Expr) -> Result<String, String> {
        let (name, anims) = self.stance_child(e).ok_or("reads a clip its clip places")?;
        let f = format!("has{}{}", name[..1].to_uppercase(), ident(&name[1..]));
        if !self.helpers.contains_key(&f) {
            let whole: Vec<String> = anims.iter().filter(|(_, r)| r.is_empty()).map(|(a, _)| quote(a)).collect();
            let mut lines = vec![format!("// whether the clip it shows places its {name} clip"), format!("function {f}() {{")];
            let some: Vec<&(String, Vec<(usize, usize)>)> = anims.iter().filter(|(_, r)| !r.is_empty()).collect();
            // (the source frame it is on: two engine frames each)
            if !some.is_empty() { lines.push("\tvar k = Std.int((self.getCurrentFrame() - 1) / 2);".into()); }
            lines.push("\tswitch (self.getAnimation()) {".into());
            if !whole.is_empty() { lines.push(format!("\t\tcase {}: return true;", whole.join(", "))); }
            for (a, runs) in some {
                let tests: Vec<String> = runs.iter().map(|(s, e)| if *s == 0 { format!("k < {e}") } else { format!("(k >= {s} && k < {e})") }).collect();
                lines.push(format!("\t\tcase {}: return {};", quote(a), tests.join(" || ")));
            }
            lines.push("\t}".into());
            lines.push("\treturn false;".into());
            lines.push("}".into());
            self.helpers.insert(f.clone(), lines.join("\n"));
        }
        Ok(format!("{f}()"))
    }

    /// a fighter's listener added by one of its stance clips' code goes with the clip, as the
    /// source's does: the options it is added with, what notes it as the clip's as it is added, and
    /// as it is taken off. the engine drops a listener added without `persistent` as the animation
    /// changes (or the state), so a clip one animation shows adds it so for an event it hears in
    /// that animation (`runtime/events.jsonc` `heard_in_animation`), and so does a clip shown by
    /// animations the engine links at the same frame, which adds it again in the one linked to
    /// (`frame_links`); any other is kept until the fighter leaves the clip for another
    /// (`fresh_clip`). its own class and root clip keep theirs
    fn clip_listen(&mut self, engine: &str, h: &str) -> (&'static str, String, String) {
        const KEPT: &str = "{ persistent: true }";
        const FREE: &str = "{ persistent: false }";
        if !self.o.fighter || self.is_itself() { return (KEPT, String::new(), String::new()) }
        if self.o.model.class_of(self.o.root).is_some_and(|c| c.name == self.class) { return (KEPT, String::new(), String::new()) }
        let mut anims: Vec<&str> = self.o.clip_frames.get(&self.class).map(|v| v.iter().map(|(a, _, _)| a.as_str()).collect()).unwrap_or_default();
        anims.sort();
        anims.dedup();
        let heard = super::events::table().heard_in_animation.iter().any(|e| e == engine);
        if heard && anims.len() == 1 { return (FREE, String::new(), String::new()) }
        // (every animation that shows it is linked to another that does: the fighter only goes
        // between them by the engine's link, which keeps the clip as the source does)
        let linked = |a: &str| self.o.frame_links.iter().any(|(x, y)| (x == a && anims.contains(&y.as_str())) || (y == a && anims.contains(&x.as_str())));
        if heard && anims.len() > 1 && anims.iter().all(|a| linked(a)) {
            self.blocks.insert("frame_links".into());
            return (FREE, format!(";\nlinkListen({engine}, {h})"), format!(";\nlinkUnlisten({engine}, {h})"));
        }
        self.blocks.insert("fresh_clip".into());
        (KEPT, format!(";\nclipListen({}, {engine}, {h})", quote(&self.class)), String::new())
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
        // (a variable of its own code holding the closure it listens with)
        if let Expr::Name(n) = handler { if self.captured.contains(&ident(n)) { return Ok(ident(n)) } }
        // (named bare, the API's own method on it, as `destroy`)
        let (o, n) = match handler {
            Expr::Member(o, n) => ((**o).clone(), n.clone()),
            Expr::Name(n) => (Expr::This, n.clone()),
            _ => return Err("listens with something other than its own method".into()),
        };
        let n = &n;
        if !matches!(o, Expr::This) && self.expr(&o)?.0 != "self" { return Err("listens with another object's call".into()) }
        if let Some(h) = self.itself_method(n) { return Ok(h) }
        let name = format!("{}OnEvent", ident(n));
        if self.helpers.contains_key(&name) { return Ok(name) }
        let code = if self.o.fighter {
            match self.fighter_api(n, &[]) { Some(r) => r?.0, None => {
                let row = calls().rows.get(n).cloned().filter(|r| !r.code.contains("{0}"))
                    .ok_or_else(|| format!("listens with its own {n}, which has no row in native_calls.jsonc"))?;
                self.fill(&row, "self", &[], n)?.0
            } }
        } else {
            let row = calls().rows.get(n).cloned().filter(|r| !r.code.contains("{0}"))
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
        // (what it writes goes on its stats' metadata too, where the others read it: `declaredOf`)
        let mut shown = Vec::new();
        let named = crate::re!(r"^[A-Za-z_]\w*$");
        for (k, v) in pairs {
            let Expr::Str(k) = k else { return Err("writes a stat by a name worked out as it runs".into()) };
            // (a value that makes or calls something is worked out once, by the write itself)
            if named.is_match(k) && !matches!(v, Expr::Call(..) | Expr::New(..)) {
                let (val, _) = self.expr(v)?;
                shown.push(format!("{k}: {val}"));
            }
            if let Some(var) = self.o.switches.get(k) {
                // off for nothing, on again for what it declares
                let when = super::projectile_stats::table().when_of(k);
                let holds = |n: f64| super::projectile_stats::holds(Some(&when), n).to_string();
                let on = match v {
                    Expr::Null | Expr::Undefined | Expr::Bool(false) => "false".to_string(),
                    Expr::Bool(true) => "true".to_string(),
                    _ if (self.o.declared)(k).is_some_and(|d| Some(d) == literal(v)) => "true".to_string(),
                    Expr::Num(n) => holds(*n),
                    Expr::Int(n) => holds(*n as f64),
                    Expr::Unary(UnOp::Neg, x) if matches!(**x, Expr::Num(_) | Expr::Int(_)) => holds(match **x { Expr::Num(n) => -n, Expr::Int(n) => -(n as f64), _ => 0.0 }),
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
            out.push(self.stat_code(&t, &val));
        }
        if !shown.is_empty() { let m = self.util("metadataWrite")?; out.push(format!("{m}(self, {{ {} }})", shown.join(", "))) }
        Ok((out.join(";\n"), Ty::Any))
    }
}

// ---- the helpers' text: runtime/native/helpers.hx ------------------------------------------------

/// a block of `runtime/native/helpers.hx` (or of another native file, `file`), as a function named
/// `name`: its capitalised `words` filled in, its `// {cases}` line replaced by `cases`, and its
/// `//:` notes for whoever edits it left out
pub fn helper(runtime: &super::runtime::Runtime, file: &str, key: &str, name: &str, words: &[(&str, String)], cases: &[String]) -> Result<String, String> {
    let text = runtime.native_files.get(file).ok_or_else(|| format!("runtime/native/{file}.hx is missing"))?;
    let marker = format!("// ---- {key}");
    let start = text.lines().position(|l| l.trim_end() == marker).ok_or_else(|| format!("native/{file}.hx has no {key}"))?;
    let block: Vec<&str> = text.lines().skip(start + 1).take_while(|l| !l.starts_with("// ---- ")).filter(|l| !l.starts_with("//:")).collect();
    let mut out = block.join("\n").trim().to_string();
    let mut words: Vec<(&str, String)> = words.to_vec();
    words.push(("NAME", name.to_string()));
    // (one pass: a word's value is never read for another word)
    words.sort_by_key(|(w, _)| std::cmp::Reverse(w.len()));
    let alts: Vec<String> = words.iter().filter(|(w, _)| !w.is_empty()).map(|(w, _)| regex::escape(w)).collect();
    if !alts.is_empty() {
        out = crate::dyn_re(&format!(r"\b(?:{})\b", alts.join("|"))).replace_all(&out, |c: &regex::Captures| {
            words.iter().find(|(w, _)| *w == &c[0]).map(|(_, v)| v.clone()).unwrap_or_default()
        }).to_string();
    }
    let mut slots = BTreeMap::new();
    slots.insert("cases", cases.to_vec());
    Ok(fill_slots(&out, &slots).trim_end().to_string())
}

/// the other helpers a helper block of `utils.hx` calls (its `//: needs a b` lines)
fn needs(runtime: &super::runtime::Runtime, key: &str) -> Vec<String> {
    let Some(text) = runtime.native_files.get("utils") else { return Vec::new() };
    let marker = format!("// ---- {key}");
    let Some(start) = text.lines().position(|l| l.trim_end() == marker) else { return Vec::new() };
    text.lines().skip(start + 1).take_while(|l| !l.starts_with("// ---- "))
        .filter_map(|l| l.strip_prefix("//: needs ")).flat_map(|r| r.split_whitespace().map(String::from).collect::<Vec<_>>()).collect()
}

/// a name a written script declares twice at its top level (`var` or `function`): the second hides
/// the first from every function that reads it, so two parts of the script would share one name
pub fn declared_twice(script: &str) -> Option<String> {
    let mut seen = BTreeSet::new();
    for line in script.lines() {
        let name = line.strip_prefix("var ").or_else(|| line.strip_prefix("function "))
            .map(|r| r.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or(""));
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            if !seen.insert(n.to_string()) { return Some(n.to_string()) }
        }
    }
    None
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
        // (a cast to another class is the value itself: `SSF2Utils.cast(getCharacter(this), KirbyExt)`)
        Expr::Call(f, a) if matches!(&**f, Expr::Member(c, m) if matches!(&**c, Expr::Name(n) if n.contains("PeptideUtils")) && matches!(m.as_str(), "cast" | "cast_"))
            && !a.is_empty() => is_itself(&a[0]),
        Expr::Call(f, a) => matches!(&**f, Expr::Member(c, m) if matches!(&**c, Expr::Name(n) if n.contains("PeptideAPI"))
            && (matches!(m.as_str(), "getProjectile" | "getItem" | "getEnemy") || (m == "getCharacter" && WRITING_FIGHTER.with(|w| w.get())))) && matches!(a.as_slice(), [Expr::This]),
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

/// a field of the source's record of an event, as this engine's event has it
/// (`runtime/events.jsonc` `record`): `None` for a field no record of the source's carries
fn record(arg: &str, field: &str) -> Result<Option<(String, Ty)>, String> {
    let t = super::events::table();
    if let Some((code, is)) = t.record.get(field) { return Ok(Some((code.replace("{arg}", arg), Ty::named(is)))) }
    if t.unrecorded.iter().any(|f| f == field) { return Err(format!("reads the {field} of the event it is handed")) }
    Ok(None)
}

/// the hit a fighter shot out as a star lands (`ejectFoes`), as the installed game declares it,
/// in this engine's fields by the box-stat mapping; none where the game is not to hand
fn star_hit() -> String {
    let Some(raw) = crate::shared_packages::star_attack() else { return "null".into() };
    let m = crate::mappings::character_hitbox_stats();
    let get = |k: &str| raw.get(k).copied();
    let damage = get("damage").unwrap_or(0.0);
    let (base, growth) = if crate::mappings::HitboxStatsMapping::is_fixed_knockback(&raw) {
        (m.fixed_knockback(&raw).unwrap_or(0.0) * m.scale_for("baseKnockback"), 0.0)
    } else {
        (get("power").unwrap_or(0.0) * m.scale_for("baseKnockback"), get("kbConstant").unwrap_or(0.0) * m.scale_for("knockbackGrowth"))
    };
    let freeze = match get("hitStun") { Some(n) if n > 0.0 => n + m.plus_frames_for("hitstop"), _ => m.unstated_freeze("hitstop", damage).unwrap_or(0.0) } * 2.0;
    let stun = match get("hitLag") { Some(n) if n > 0.0 => n * 2.0, Some(n) => n, None => -1.0 };
    format!("{{ damage: {}, angle: {}, baseKnockback: {}, knockbackGrowth: {}, hitstop: {}, hitstun: {} }}",
        number(damage), number(get("direction").unwrap_or(0.0)), number(base), number(growth), number(freeze), number(stun))
}

/// whether a record's setup makes a clip record for a clip its clip places under this name
fn setup_has_kid(setup: &[String], name: &str) -> bool {
    setup.iter().any(|l| l.contains(&format!("me.{name} = clipRecord(")))
}

/// the call that makes (or finds) one of an object's parts, `projPart`
pub fn part_call(p: &crate::emit::ObjPart) -> String {
    let labels: Vec<String> = p.labels.iter().map(|(l, f)| format!("[{f}, {}]", quote(l))).collect();
    let (x, y, r, sx, sy) = p.at;
    format!("projPart({}, {}, {}, {}, {}, {}, {}, {}, [{}])", quote(&p.path), number(x), number(y), number(r), number(sx), number(sy), quote(&p.anim), p.frames, labels.join(", "))
}

/// a source box number as the name the source declares it under (`attackBox`, `attackBox2`, ...): the suffix
fn box_name_suffix(e: &Expr) -> String {
    let n = match e { Expr::Int(n) => *n, Expr::Num(n) => *n as i64, _ => 1 };
    if n <= 1 { String::new() } else { n.to_string() }
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
    // (a call's own `(-1)` is its argument, not a number in brackets)
    crate::re!(r"(^|[^\w)\]])\(-1\) \* ").replace_all(&out, "${1}-").to_string()
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
    // (debug, `PEPTIDE_NATIVE_WHY`: the expression itself)
    if std::env::var("PEPTIDE_NATIVE_WHY").is_ok() { eprintln!("[no plain form] {e:?}"); }
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

/// a variable given a field of its own once, a field the function never writes (the decompiler's
/// `_v1 = this.self` before an increment through it), read as that field wherever it is used
fn aliases(body: &mut Vec<Stmt>) {
    let mut sets: BTreeMap<u32, Vec<Expr>> = BTreeMap::new();
    let mut written: BTreeSet<String> = BTreeSet::new();
    walk(body, &mut |s| match s {
        Stmt::Assign(Place::Local(r), v) => sets.entry(*r).or_default().push(v.clone()),
        Stmt::Assign(Place::Member(Expr::This, f), _) => { written.insert(f.clone()); }
        _ => {}
    });
    // (a register a loop steps through by name is named by number there, so it stays itself)
    let mut stepped: BTreeSet<u32> = BTreeSet::new();
    walk_exprs(body, &mut |e| if let Expr::HasNext2 { object, index } = e { stepped.insert(*object); stepped.insert(*index); });
    let alias: BTreeMap<u32, Expr> = sets.into_iter().filter(|(r, _)| !stepped.contains(r)).filter_map(|(r, vs)| match vs.as_slice() {
        [v @ Expr::Member(t, f)] if matches!(**t, Expr::This) && !written.contains(f) => Some((r, v.clone())),
        _ => None,
    }).collect();
    if alias.is_empty() { return }
    fn swap(list: Vec<Stmt>, alias: &BTreeMap<u32, Expr>) -> Vec<Stmt> {
        let mut f = |e: Expr| match e { Expr::Local(r) if alias.contains_key(&r) => alias[&r].clone(), e => e };
        list.into_iter().filter_map(|s| Some(match s {
            Stmt::Assign(Place::Local(r), _) if alias.contains_key(&r) => return None,
            Stmt::Expr(e) => Stmt::Expr(e.map(&mut f)),
            Stmt::Assign(p, e) => Stmt::Assign(p.map(&mut f), e.map(&mut f)),
            Stmt::Return(Some(e)) => Stmt::Return(Some(e.map(&mut f))),
            Stmt::Throw(e) => Stmt::Throw(e.map(&mut f)),
            Stmt::If(c, a, b) => Stmt::If(c.map(&mut f), swap(a, alias), swap(b, alias)),
            Stmt::Loop(b) => Stmt::Loop(swap(b, alias)),
            Stmt::While(c, b) => Stmt::While(c.map(&mut f), swap(b, alias)),
            Stmt::DoWhile(b, c) => Stmt::DoWhile(swap(b, alias), c.map(&mut f)),
            Stmt::Switch(c, cs, d) => Stmt::Switch(c.map(&mut f), cs.into_iter().map(|(k, b)| (k, swap(b, alias))).collect(), swap(d, alias)),
            Stmt::Try(b, cs) => Stmt::Try(swap(b, alias), cs.into_iter().map(|mut c| { c.body = swap(c.body, alias); c }).collect()),
            other => other,
        })).collect()
    }
    *body = swap(std::mem::take(body), &alias);
}

/// the value a static field of a class holds: the one it declares, else the one its class's static
/// setup puts in it once, where nothing else writes it
fn static_value(k: &ClassModel, n: &str) -> Option<Expr> {
    let f = k.fields.iter().find(|f| f.is_static && f.name == n)?;
    if let Some(d) = &f.default { return Some(d.clone()) }
    let mut set: Vec<Expr> = Vec::new();
    walk(&k.static_init.body, &mut |st| match st {
        Stmt::Assign(Place::Member(o, g), v) if g == n && matches!(o, Expr::This | Expr::Name(_)) => set.push(v.clone()),
        Stmt::Assign(Place::Name(g), v) if g == n => set.push(v.clone()),
        _ => {}
    });
    let mut elsewhere = false;
    for b in k.methods.iter().map(|m| &m.function.body).chain([&k.constructor.body]) {
        walk(b, &mut |st| if let Stmt::Assign(Place::Member(_, g), _) = st { if g == n { elsewhere = true } });
    }
    (set.len() == 1 && !elsewhere).then(|| set.remove(0))
}

/// the one value its class's setup gives `n`, where nothing else writes it
fn static_set(k: &ClassModel, n: &str) -> Option<Expr> {
    let mut set: Vec<Expr> = Vec::new();
    walk(&k.static_init.body, &mut |st| match st {
        Stmt::Assign(Place::Member(o, g), v) if g == n && matches!(o, Expr::This | Expr::Name(_)) => set.push(v.clone()),
        Stmt::Assign(Place::Name(g), v) if g == n => set.push(v.clone()),
        _ => {}
    });
    let mut elsewhere = false;
    for b in k.methods.iter().map(|m| &m.function.body).chain([&k.constructor.body]) {
        walk(b, &mut |st| match st { Stmt::Assign(Place::Member(_, g), _) | Stmt::Assign(Place::Name(g), _) if g == n => elsewhere = true, _ => {} });
    }
    (set.len() == 1 && !elsewhere).then(|| set.remove(0))
}

/// the object and index a loop's test steps through by name (`hasnext2`), where it is one
fn for_in(c: &Expr) -> Option<(u32, u32)> {
    let mut at = None;
    crate::model::ast::walk_exprs(&[Stmt::Expr(c.clone())], &mut |e| if let Expr::HasNext2 { object, index } = e { at = Some((*object, *index)); });
    at
}

/// every expression of a list of statements, put through `f`
/// a function whose variables that only ever hold one of its own clips (`getStanceMC()`, `getMC()`)
/// read as that call wherever they are read: the clip, reached the ways its code reaches it
fn own_clip_held(f: &Function) -> Option<Function> {
    let own = |v: &Expr| matches!(v, Expr::Call(g, a) if a.is_empty() && matches!(&**g, Expr::Member(t, m) if matches!(**t, Expr::This) && matches!(m.as_str(), "getStanceMC" | "stanceClip" | "getMC")));
    let mut calls: BTreeMap<u32, Expr> = BTreeMap::new();
    let mut other: BTreeSet<u32> = BTreeSet::new();
    walk(&f.body, &mut |st| if let Stmt::Assign(Place::Local(r), v) = st {
        if own(v) && calls.get(r).is_none_or(|c| c == v) { calls.insert(*r, v.clone()); }
        else if !matches!(v, Expr::Null | Expr::Undefined) { other.insert(*r); }
    });
    calls.retain(|r, _| !other.contains(r) && *r as usize > f.params.len());
    if calls.is_empty() { return None }
    // (its writes go: every read is the call)
    fn unwrite(list: Vec<Stmt>, calls: &BTreeMap<u32, Expr>) -> Vec<Stmt> {
        list.into_iter().filter(|s| !matches!(s, Stmt::Assign(Place::Local(r), _) if calls.contains_key(r))).map(|s| match s {
            Stmt::If(c, a, b) => Stmt::If(c, unwrite(a, calls), unwrite(b, calls)),
            Stmt::Loop(b) => Stmt::Loop(unwrite(b, calls)),
            Stmt::While(c, b) => Stmt::While(c, unwrite(b, calls)),
            Stmt::DoWhile(b, c) => Stmt::DoWhile(unwrite(b, calls), c),
            Stmt::Switch(c, cs, d) => Stmt::Switch(c, cs.into_iter().map(|(k, b)| (k, unwrite(b, calls))).collect(), unwrite(d, calls)),
            Stmt::Try(b, cs) => Stmt::Try(unwrite(b, calls), cs.into_iter().map(|mut c| { c.body = unwrite(c.body, calls); c }).collect()),
            other => other,
        }).collect()
    }
    let body = map_stmts(unwrite(f.body.clone(), &calls), &mut |e| match &e { Expr::Local(r) => calls.get(r).cloned().unwrap_or(e), _ => e });
    Some(Function { body, ..f.clone() })
}

/// the statements that give a clip record (`at`) each clip its library clip places under a name,
/// as a record of its own under that name (a clip its code makes and reaches into, drawn nowhere)
fn clip_record_of(model: &SourceModel, sym: Option<u16>, at: &str, depth: usize, out: &mut Vec<String>) {
    let Some(sp) = sym.and_then(|s| model.sprites.get(&s)).filter(|_| depth < 4) else { return };
    let mut named: BTreeMap<String, (u16, f64, f64)> = BTreeMap::new();
    for p in sp.frames.iter().flatten() {
        if let Some(n) = p.name.as_deref() { named.entry(n.to_string()).or_insert((p.char_id, p.matrix.tx, p.matrix.ty)); }
    }
    for (n, (s, x, y)) in named.iter().filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
        let path = format!("{at}.{n}");
        // (where its clip first places it)
        let place = if *x != 0.0 || *y != 0.0 { format!(" {path}.x = {}; {path}.y = {};", number(*x), number(*y)) } else { String::new() };
        out.push(format!("{path} = clipRecord(null); {path}.name = {};{place} {at}.kids.push({path});", quote(n)));
        clip_record_of(model, Some(*s), &path, depth + 1, out);
    }
}

pub fn map_stmts(list: Vec<Stmt>, f: &mut impl FnMut(Expr) -> Expr) -> Vec<Stmt> {
    list.into_iter().map(|s| match s {
        Stmt::Expr(e) => Stmt::Expr(e.map(f)),
        Stmt::Assign(p, e) => Stmt::Assign(p.map(f), e.map(f)),
        Stmt::Return(Some(e)) => Stmt::Return(Some(e.map(f))),
        Stmt::Throw(e) => Stmt::Throw(e.map(f)),
        Stmt::If(c, a, b) => Stmt::If(c.map(f), map_stmts(a, f), map_stmts(b, f)),
        Stmt::Loop(b) => Stmt::Loop(map_stmts(b, f)),
        Stmt::While(c, b) => Stmt::While(c.map(f), map_stmts(b, f)),
        Stmt::DoWhile(b, c) => Stmt::DoWhile(map_stmts(b, f), c.map(f)),
        Stmt::Switch(c, cs, d) => Stmt::Switch(c.map(f), cs.into_iter().map(|(k, b)| (k, map_stmts(b, f))).collect(), map_stmts(d, f)),
        Stmt::Try(b, cs) => Stmt::Try(map_stmts(b, f), cs.into_iter().map(|mut c| { c.body = map_stmts(c.body, f); c }).collect()),
        other => other,
    }).collect()
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

pub fn walk(list: &[Stmt], f: &mut dyn FnMut(&Stmt)) {
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

/// what a projectile stat's `native_calls.jsonc` row does: the engine's own stat (`self.` calls),
/// a note of what the source's write does here, or a container its own script keeps
pub enum StatRow { Engine(String), Note(String), Kept(String) }

pub fn stat_row(k: &str) -> Option<StatRow> {
    let t = calls().projectile_stats.get(k)?.clone();
    Some(if t.trim_start().starts_with("//") { StatRow::Note(t) } else if t.trim_start().starts_with("self.") { StatRow::Engine(t) } else { StatRow::Kept(t) })
}

/// a colour transform made fresh: the engine's colour shader (`native_classes.jsonc`)
fn calls_ct() -> String { "new RgbaColorShader()".into() }

/// the container that holds a part's vfx
fn part_id(name: &str) -> String { format!("{}Part", ident(name)) }

/// the block of `runtime/native/classes.hx` that writes out a class, by its short name
fn flat_template(rt: &super::runtime::Runtime, class: &str) -> Option<String> {
    let text = rt.native_files.get("classes")?;
    let marker = format!("// ---- {class}");
    let start = text.lines().position(|l| l.trim_end() == marker)?;
    Some(text.lines().skip(start + 1).take_while(|l| !l.starts_with("// ---- ")).collect::<Vec<_>>().join("\n"))
}

/// whether `c`'s own code writes its field `f`
fn writes_own(c: &ClassModel, f: &str) -> bool {
    let mut yes = false;
    for b in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
        walk(b, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, g), _) = st { if g == f { yes = true } });
    }
    yes
}

/// the fields of `c` that keep one of the API's classes with a record form (`K as a record` in
/// classes.hx) as a record: one its code uses as a value (puts in another field, hands on), and one
/// its code fills only from those (the timer running now)
fn rt_value_fields(o: &Object, c: &ClassModel) -> BTreeMap<String, String> {
    let record_form = |k: &str| o.runtime.native_files.get("classes").is_some_and(|t| t.lines().any(|l| l.trim_end() == format!("// ---- {k} as a record")));
    let bodies: Vec<&Vec<Stmt>> = c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]).collect();
    let mut names: BTreeSet<String> = c.fields.iter().map(|f| f.name.clone()).collect();
    for b in &bodies { walk(b, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, f), _) = st { names.insert(f.clone()); }); }
    let me = |e: &Expr, f: &str| matches!(e, Expr::Member(t, g) if matches!(**t, Expr::This) && g == f);
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for f in &names {
        let Some(k) = flat_class(o, c, f).filter(|k| record_form(k)) else { continue };
        let (mut all, mut recv) = (0usize, 0usize);
        for b in &bodies {
            walk_exprs(b, &mut |e| {
                if me(e, f) { all += 1 }
                if let Expr::Member(x, _) = e { if me(x, f) { recv += 1 } }
            });
            walk(b, &mut |st| if let Stmt::Assign(Place::Member(x, _) | Place::Index(x, _), _) = st { if me(x, f) { recv += 1 } });
        }
        if all > recv { out.insert(f.clone(), k); }
    }
    // (a field filled only from those, or with nothing)
    loop {
        let mut grew = false;
        for g in &names {
            if out.contains_key(g) { continue }
            let mut from: BTreeSet<String> = BTreeSet::new();
            let mut other = false;
            for b in &bodies {
                walk(b, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, x), v) = st {
                    if x != g { return }
                    let mut v = v; while let Expr::Coerce(_, y) = v { v = y; }
                    match v {
                        Expr::Member(t, a) if matches!(**t, Expr::This) => match out.get(a) { Some(k) => { from.insert(k.clone()); } None => other = true },
                        Expr::Null | Expr::Undefined => {}
                        _ => other = true,
                    }
                });
            }
            if !other && from.len() == 1 { out.insert(g.clone(), from.into_iter().next().unwrap_or_default()); grew = true; }
        }
        if !grew { break }
    }
    out
}

/// the class a field of `c` holds, where every value the code puts in it is a new one of the same
/// class `runtime/native/classes.hx` writes out (or nothing)
fn flat_class(o: &Object, c: &ClassModel, field: &str) -> Option<String> {
    let mut kinds = BTreeSet::new();
    let mut other = false;
    for body in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
        walk(body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, f), v) = st {
            if f != field { return }
            // (a new clip cast to the class, `new MovieClip() as RainWeather`, is no instance of it:
            // the cast answers null, a placeholder)
            let mut v = v;
            while let Expr::Coerce(_, x) = v { v = x; }
            if let Expr::Binary(BinOp::As, x, t) = v {
                let made = match &**x { Expr::New(n, _) => match &**n { Expr::Name(n) => Some(short(n).to_string()), _ => None }, _ => None };
                let to = match &**t { Expr::Name(t) => Some(short(t).to_string()), _ => None };
                if made.is_some() && made != to { return }
            }
            match v {
                Expr::New(n, _) => match &**n { Expr::Name(n) => { kinds.insert(short(n).to_string()); } _ => other = true },
                Expr::Null | Expr::Undefined => {}
                _ => other = true,
            }
        });
    }
    if other || kinds.len() != 1 { return None }
    let k = kinds.into_iter().next()?;
    // (by the source's name for it, where the converter renames the API's class)
    let k = o.runtime.names.classes.iter().find(|(_, r)| r.as_str() == k).map(|(s, _)| s.clone()).unwrap_or(k);
    if flat_template(o.runtime, &k).is_some() { return Some(k) }
    // (or one of the package's own, a plain class with no clip)
    o.model.classes.values().find(|c| short(&c.name) == k && c.symbol.is_none() && matches!(short(&c.super_name), "Object" | "")).map(|c| c.name.clone())
}

/// the projectile stats any class of the package writes on another object
/// (`o.updateProjectileStats({...})`): an object that could be written so publishes what it does
/// each it keeps itself as a setter (`exports.__stat`)
pub fn others_write_stats(model: &crate::model::SourceModel) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for c in model.classes.values() {
        for body in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
            walk_exprs(body, &mut |e| if let Expr::Call(f, a) = e {
                if matches!(&**f, Expr::Member(o, n) if n == "updateProjectileStats" && !matches!(**o, Expr::This)) {
                    if let [Expr::Object(kv)] = a.as_slice() { for (k, _) in kv { if let Expr::Str(k) = k { keys.insert(k.clone()); } } }
                }
            });
        }
    }
    keys
}

/// whether any class of the package reads the label another object's clip is on
/// (`o.getStanceMC().currentLabel`): an object that could be read so publishes its own
/// whether a method of one of the package's classes writes its record (its own fields, or another
/// of its methods that does)
fn record_writes(model: &crate::model::SourceModel, class: &str, method: &str) -> bool {
    let Some(c) = model.classes.get(class) else { return true };
    let mut seen = BTreeSet::new();
    let mut todo = vec![method.to_string()];
    while let Some(m) = todo.pop() {
        if !seen.insert(m.clone()) { continue }
        let Some(f) = c.methods.iter().find(|x| x.name == m) else { continue };
        let mut writes = false;
        walk(&f.function.body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, _), _) = st { writes = true });
        walk_exprs(&f.function.body, &mut |e| if let Expr::Call(g, _) = e {
            match &**g {
                // (a call on a field it keeps: a timer it ticks writes it)
                Expr::Member(t, n) if matches!(**t, Expr::This) => todo.push(n.clone()),
                Expr::Member(t, _) if matches!(&**t, Expr::Member(tt, _) if matches!(**tt, Expr::This)) => writes = true,
                _ => {}
            }
        });
        if writes { return true }
    }
    false
}

/// whether a package's code takes another fighter's power (writes its own `KirbyPower`, or asks
/// which one it holds)
pub fn takes_power(model: &crate::model::SourceModel, runtime: &super::runtime::Runtime) -> bool {
    let mut found = false;
    // (its own classes: not the API's, which a package carries too)
    for c in model.classes.values().filter(|c| !runtime.provides(&c.name)) {
        for body in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
            walk_exprs(body, &mut |e| match e {
                Expr::Member(_, n) if n == "KirbyPower" => found = true,
                Expr::Call(f, _) if matches!(&**f, Expr::Member(_, m) if matches!(m.as_str(), "copyPower" | "getCurrentKirbyPower")) => found = true,
                _ => {}
            });
        }
    }
    found
}

pub fn reads_others_label(model: &crate::model::SourceModel) -> bool {
    let mut found = false;
    for c in model.classes.values() {
        for body in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
            walk_exprs(body, &mut |e| if let Expr::Member(mc, n) = e {
                if matches!(n.as_str(), "currentLabel" | "currentFrameLabel") && matches!(&**mc, Expr::Call(g, a) if a.is_empty()
                    && matches!(&**g, Expr::Member(o, s) if matches!(s.as_str(), "getStanceMC" | "stanceClip") && !matches!(**o, Expr::This))) {
                    found = true;
                }
            });
        }
    }
    found
}

fn walk_exprs(list: &[Stmt], f: &mut dyn FnMut(&Expr)) {
    crate::model::ast::walk_exprs(list, &mut |e| f(e));
}

fn walk_places(list: &[Stmt], f: &mut dyn FnMut(u32)) {
    walk(list, &mut |s| if let Stmt::Assign(Place::Local(r), _) = s { f(*r) });
    walk_exprs(list, &mut |e| if let Expr::Local(r) = e { f(*r) });
}

/// the container a note of true or false is kept in: an object (the engine keeps few true-or-false
/// containers for a whole match)
fn bool_make(_fighter: bool) -> &'static str { "makeObject" }

/// what a frame's `return` is written as in its keyframe, for the emitter to finish (a frame's code
/// ends with what the rules run after it, which a `return` must run too)
pub const FRAME_RETURN: &str = "return; //frame";

/// the methods a fighter's code calls on another object, which that object publishes for it
pub fn asked_of_others() -> &'static std::sync::Mutex<BTreeSet<String>> {
    static ASKED: std::sync::OnceLock<std::sync::Mutex<BTreeSet<String>>> = std::sync::OnceLock::new();
    ASKED.get_or_init(Default::default)
}

/// the methods of the objects another package lends a fighter (an item its code makes that the
/// game's shared packages hold): its code calls them on those objects, which publish them
/// the names other packages' code reads or writes on an object (a stage's code on the objects a lent
/// package carries): what an object publishes for them, with its own package's
pub fn asked_fields() -> &'static std::sync::Mutex<BTreeSet<String>> {
    static T: OnceLock<std::sync::Mutex<BTreeSet<String>>> = OnceLock::new();
    T.get_or_init(|| std::sync::Mutex::new(BTreeSet::new()))
}

/// the fields of the classes a lent package carries, for the stage's code that reads them
pub fn lent_fields() -> &'static std::sync::Mutex<BTreeSet<String>> {
    static T: OnceLock<std::sync::Mutex<BTreeSet<String>>> = OnceLock::new();
    T.get_or_init(|| std::sync::Mutex::new(BTreeSet::new()))
}

pub fn lent_methods() -> &'static std::sync::Mutex<BTreeSet<String>> {
    static LENT: std::sync::OnceLock<std::sync::Mutex<BTreeSet<String>>> = std::sync::OnceLock::new();
    LENT.get_or_init(Default::default)
}

/// a frame's function's body, as its keyframe's code: its lines, a tab less
fn frame_body(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 { return String::new() }
    lines[1..lines.len() - 1].iter().map(|l| l.strip_prefix('\t').unwrap_or(l)).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::fold;

    #[test]
    fn fold_leaves_a_calls_argument_alone() {
        assert_eq!(fold("flipX(-1) * 1.3"), "flipX(-1) * 1.3");
        assert_eq!(fold("(-1) * x"), "-x");
        assert_eq!(fold("2 * 1.3"), "2.6");
    }
}

/// whether every value a class's code puts in a field is a record literal, or an element of one of
/// its fields that only ever holds a list of record literals
fn holds_records(c: &ClassModel, field: &str) -> bool {
    let bodies: Vec<&Vec<Stmt>> = c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]).collect();
    // each value put in a field, with the body it is put in
    let assigned = |f: &str| -> Vec<(Expr, &Vec<Stmt>)> {
        let mut out = Vec::new();
        for b in &bodies {
            walk(b, &mut |st| if let Stmt::Assign(Place::Member(t, n), v) = st { if matches!(t, Expr::This) && n == f { out.push((v.clone(), *b)) } });
        }
        out
    };
    fn strip(e: &Expr) -> Expr { let mut e = e.clone(); while let Expr::Coerce(_, x) | Expr::Convert(_, x) = e { e = *x; } e }
    let lists = |f: &str| { let vs = assigned(f); !vs.is_empty() && vs.iter().all(|(v, _)| matches!(strip(v), Expr::Array(ref items) if items.iter().all(|i| matches!(strip(i), Expr::Object(_))))) };
    // an element of one of its fields holding records, read
    let element = |e: &Expr| matches!(strip(e), Expr::Index(ref l, _) if matches!(&**l, Expr::Member(t, n) if matches!(**t, Expr::This) && lists(n)));
    let vs = assigned(field);
    !vs.is_empty() && vs.iter().all(|(v, body)| match strip(v) {
        Expr::Object(_) | Expr::Null => true,
        Expr::Index(ref l, _) if matches!(&**l, Expr::Member(t, n) if matches!(**t, Expr::This) && lists(n)) => true,
        // (an element of a list its function fills only with such elements)
        Expr::Index(ref l, _) => {
            let Expr::Local(r) = **l else { return false };
            let mut pushes = 0;
            let mut other = false;
            crate::model::ast::walk_exprs(body, &mut |e| if let Expr::Call(g, a) = e {
                if let Expr::Member(t, n) = &**g { if matches!(**t, Expr::Local(x) if x == r) && n == "push" { pushes += 1; if !a.iter().all(&element) { other = true } } }
            });
            pushes > 0 && !other
        }
        _ => false,
    })
}

// ---- drawing that comes to nothing --------------------------------------------------------------

/// the classes whose values only draw pixels or say where to (`runtime/inert.jsonc` names what this
/// engine never draws): a field declared one of these, or a class that extends one, holds drawing
const DRAWN: [&str; 6] = ["BitmapData", "Bitmap", "Rectangle", "Point", "ColorTransform", "Matrix"];

fn drawn_type(model: &crate::model::SourceModel, t: &str) -> bool {
    let mut n = short(t).to_string();
    for _ in 0..8 {
        if DRAWN.contains(&n.as_str()) { return true }
        match model.classes.values().find(|k| short(&k.name) == n) { Some(k) => n = short(&k.super_name).to_string(), None => return false }
    }
    false
}

/// a value that is only ever drawing: a new one of a drawing class, or one cast to one
fn drawing_value(v: &Expr, model: &crate::model::SourceModel) -> bool {
    let mut v = v;
    while let Expr::Coerce(_, x) | Expr::Convert(_, x) = v { v = x; }
    match v {
        Expr::New(c, _) => matches!(&**c, Expr::Name(n) if drawn_type(model, n)),
        Expr::Call(g, _) => matches!(&**g, Expr::Member(c, m) if m == "create" && matches!(&**c, Expr::Name(n) if drawn_type(model, n))),
        Expr::Binary(BinOp::As, _, c) => matches!(&**c, Expr::Name(n) if drawn_type(model, n)),
        _ => false,
    }
}

/// whether a variable is only ever read for its fields (`r.x`), never called, handed on, compared
/// or written: what its code holds is a record
fn only_fields_read(body: &[Stmt], r: u32) -> bool {
    let mut fields = 0usize;
    let mut other = 0usize;
    let mut called = 0usize;
    crate::model::ast::walk_exprs(body, &mut |e| match e {
        Expr::Member(o, _) if matches!(**o, Expr::Local(x) if x == r) => fields += 1,
        // (tested against nothing: whether it was handed one)
        Expr::Binary(BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe, a, b) if matches!((&**a, &**b), (Expr::Local(x), Expr::Null | Expr::Undefined) | (Expr::Null | Expr::Undefined, Expr::Local(x)) if *x == r) => fields += 1,
        Expr::Local(x) if *x == r => other += 1,
        // (a call on it: an object, no record)
        Expr::Call(f, _) if matches!(&**f, Expr::Member(o, m) if matches!(**o, Expr::Local(x) if x == r) && !calls().record_calls.contains_key(m.as_str())) => called += 1,
        _ => {}
    });
    if called > 0 { return false }
    let mut written = false;
    walk(body, &mut |st| if let Stmt::Assign(Place::Local(x) | Place::Member(Expr::Local(x), _), _) = st { if *x == r { written = true; } });
    // (each field read walks the variable once more, inside the read)
    fields > 0 && other == fields && !written
}

/// what a place or value is held under: one of its own fields, or a variable
#[derive(Clone, Copy, PartialEq)]
enum DrawRoot<'e> { Field(&'e str), Local(u32), Other }

fn draw_root(e: &Expr) -> DrawRoot<'_> {
    match e {
        Expr::Member(t, f) if matches!(**t, Expr::This) => DrawRoot::Field(f),
        Expr::Local(r) => DrawRoot::Local(*r),
        Expr::Member(x, _) | Expr::Index(x, _) | Expr::Coerce(_, x) | Expr::Convert(_, x) => draw_root(x),
        Expr::Binary(BinOp::As, x, _) => draw_root(x),
        _ => DrawRoot::Other,
    }
}

/// the fields of a class, and per function its variables, that only ever feed drawing: written
/// from values that do nothing on their own, and read only by drawing calls or by writes to others
/// of the same kind. a statement that only draws, or only fills one of them, comes to nothing here
pub struct Drawing<'r> {
    inert: &'r BTreeSet<String>,
    model: &'r crate::model::SourceModel,
    pub fields: BTreeSet<String>,
}

impl<'r> Drawing<'r> {
    fn held(&self, root: DrawRoot, locals: &BTreeSet<u32>) -> bool {
        match root { DrawRoot::Field(f) => self.fields.contains(f), DrawRoot::Local(r) => locals.contains(&r), DrawRoot::Other => false }
    }
    /// a call that only draws: a drawing method of a picture's data, or of something held for drawing
    fn drawing_call(&self, e: &Expr, locals: &BTreeSet<u32>) -> bool {
        let Expr::Call(g, _) = e else { return false };
        let Expr::Member(recv, m) = &**g else { return false };
        let Expr::Call(_, args) = e else { return false };
        self.inert.contains(m) && (matches!(&**recv, Expr::Member(_, b) if b == "bitmapData") || self.held(draw_root(recv), locals))
            && args.iter().all(|a| Self::quiet(a, self.model))
    }
    /// a value whose working out does nothing on its own: reads, arithmetic, casts, a child of a
    /// clip by place or name, a class's name, a new value of a drawing class
    fn quiet(e: &Expr, model: &crate::model::SourceModel) -> bool {
        let mut ok = true;
        e.walk(&mut |x| match x {
            Expr::Call(g, _) => {
                let n = match &**g { Expr::Member(_, n) | Expr::Name(n) => n.as_str(), _ => "" };
                // (an accessor asked of nothing: a getter)
                let getter = n.starts_with("get") && matches!(x, Expr::Call(_, a) if a.is_empty());
                if !getter && !matches!(n, "getChildAt" | "getChildByName" | "getQualifiedClassName" | "create") { ok = false }
            }
            Expr::New(c, _) => if !matches!(&**c, Expr::Name(n) if drawn_type(model, n)) { ok = false },
            Expr::Function(_) | Expr::Assign(..) => ok = false,
            _ => {}
        });
        ok
    }
    pub fn void(&self, s: &Stmt, locals: &BTreeSet<u32>, model: &crate::model::SourceModel) -> bool {
        match s {
            Stmt::Expr(e) => self.drawing_call(e, locals),
            Stmt::Assign(p, v) => {
                let root = match p { Place::Local(r) => DrawRoot::Local(*r), Place::Member(Expr::This, f) => DrawRoot::Field(f), Place::Member(o, _) | Place::Index(o, _) => draw_root(o), _ => DrawRoot::Other };
                self.held(root, locals) && Self::quiet(v, model)
            }
            Stmt::If(c, a, b) => (!a.is_empty() || !b.is_empty()) && Self::quiet(c, model)
                && a.iter().chain(b.iter()).all(|x| self.void(x, locals, model)),
            _ => false,
        }
    }
    /// the fields and variables read where it counts (outside what comes to nothing)
    fn live(&self, list: &[Stmt], locals: &BTreeSet<u32>, model: &crate::model::SourceModel, fields: &mut BTreeSet<String>, vars: &mut BTreeSet<u32>) {
        for s in list {
            if self.void(s, locals, model) { continue }
            for e in s.exprs() {
                e.walk(&mut |x| match x {
                    Expr::Member(t, f) if matches!(**t, Expr::This) => { fields.insert(f.clone()); }
                    Expr::Local(r) => { vars.insert(*r); }
                    Expr::Function(func) => { let mut g = BTreeSet::new(); crate::model::ast::walk_exprs(&func.body, &mut |y| { if let Expr::Member(t, f) = y { if matches!(**t, Expr::This) { g.insert(f.clone()); } } }); fields.extend(g); }
                    _ => {}
                });
            }
            for b in s.blocks() { self.live(b, locals, model, fields, vars) }
        }
    }
    /// a function's variables that only feed drawing (its own, not its arguments)
    pub fn locals(&self, body: &[Stmt], nparams: u32, model: &crate::model::SourceModel) -> BTreeSet<u32> {
        // (each one every value it is given is drawing: a picture, or a clip's child cast to one)
        let mut all: BTreeMap<u32, bool> = BTreeMap::new();
        fn assigned(list: &[Stmt], model: &crate::model::SourceModel, out: &mut BTreeMap<u32, bool>) {
            for s in list {
                if let Stmt::Assign(Place::Local(r), v) = s { let had = out.get(r).copied().unwrap_or(true); out.insert(*r, had && (drawing_value(v, model) || matches!(v, Expr::Null | Expr::Undefined))); }
                for b in s.blocks() { assigned(b, model, out) }
            }
        }
        assigned(body, model, &mut all);
        let mut l: BTreeSet<u32> = all.into_iter().filter(|(r, ok)| *ok && *r > nparams).map(|(r, _)| r).collect();
        loop {
            let (mut f, mut v) = (BTreeSet::new(), BTreeSet::new());
            self.live(body, &l, model, &mut f, &mut v);
            let next: BTreeSet<u32> = l.iter().copied().filter(|r| !v.contains(r)).collect();
            if next == l { return l }
            l = next;
        }
    }
    pub fn of(class: &ClassModel, model: &'r crate::model::SourceModel, inert: &'r BTreeSet<String>) -> Self {
        // (a field declared one, or one with no type its code only ever fills with one)
        let mut filled: BTreeMap<String, bool> = BTreeMap::new();
        let drawing_value = |v: &Expr| drawing_value(v, model);
        for m in class.methods.iter().map(|m| &m.function).chain(std::iter::once(&class.constructor)) {
            walk(&m.body, &mut |st| if let Stmt::Assign(Place::Member(Expr::This, f), v) = st {
                let ok = drawing_value(v);
                let had = filled.get(f).copied().unwrap_or(true);
                filled.insert(f.clone(), had && ok);
            });
        }
        let untyped = |t: Option<&str>| t.is_none_or(|t| matches!(short(t), "Object" | "*"));
        let mut fields: BTreeSet<String> = class.fields.iter().filter(|f| !f.is_static && (f.type_name.as_deref().is_some_and(|t| drawn_type(model, t))
            || (untyped(f.type_name.as_deref()) && filled.get(&f.name) == Some(&true)))).map(|f| f.name.clone()).collect();
        // (and one its code only sets, never declaring it: a property of the object)
        for (f, all) in &filled { if *all && !class.fields.iter().any(|x| &x.name == f) { fields.insert(f.clone()); } }
        let mut d = Drawing { inert, model, fields };
        if d.fields.is_empty() { return d }
        let fns: Vec<&crate::model::ast::Function> = class.methods.iter().map(|m| &m.function).chain(std::iter::once(&class.constructor)).collect();
        loop {
            let mut live = BTreeSet::new();
            for f in &fns {
                let l = d.locals(&f.body, f.params.len() as u32, model);
                let mut v = BTreeSet::new();
                d.live(&f.body, &l, model, &mut live, &mut v);
            }
            let next: BTreeSet<String> = d.fields.iter().filter(|f| !live.contains(*f)).cloned().collect();
            if next == d.fields { return d }
            d.fields = next;
        }
    }
}
