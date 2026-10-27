//! One emit path: an engine object's script, assembled from the runtime, the object's data and
//! its compiled classes, in that order and in marked sections (see `docs/CONVERTER_DESIGN.md`).
//!
//! Nothing here knows what kind of object it writes: a fighter is its declared class, its clip,
//! and the classes and clips they reach. The runtime's files decide what the engine does with
//! them.

mod shake;
pub mod stage;
pub mod stage_tables;

use std::collections::{BTreeMap, BTreeSet};

use crate::compile::{self, Runtime};
use crate::model::ast::{self, Expr};
use crate::model::declare::Value;
use crate::model::SourceModel;

/// Where one engine animation's frames come from: the source label (the fighter clip's frame
/// label, whose `stance` clip plays it) and the stance clip frame of its first frame, 1-based.
#[derive(Clone, Debug)]
pub struct Piece {
    pub fm_name: String,
    /// the root clip's label the animation is under ("" when the root has none to go to)
    pub label: String,
    /// the clip whose frames it shows: the root's `stance`, else the root itself
    pub sym: u16,
    /// the stance clip frame the animation's first frame shows (1-based)
    pub from: u32,
    /// how many source frames it holds
    pub frames: u32,
    /// frames taken from the stance clip's head after the slice (a seamless loop)
    pub head: u32,
    pub reversed: bool,
    /// the frames are another animation's: the label's own animation is a different one, and
    /// this one borrows its frames (a ladder climb shown with the standing frames)
    pub borrowed: bool,
    /// where the source's item box sits, from the frame (0-based in the piece) it moves to
    /// there: (frame, x, y) at its centre, in the source's units
    pub item_box: Vec<(u32, f64, f64)>,
    /// the animation the clip plays on into past this one's last frame, where the engine's own
    /// state machine has none (see `SplitAnim::plays_on`)
    pub on: Option<String>,
}

/// the engine's animations a clip's frames are, for a jump its own code makes by label: each of
/// its labels that one animation shows, the animation and whether the label is its first frame,
/// with every animation the clip's frames are (a jump is the engine's own only while one of them
/// is playing: the fighter's clip is this one)
pub fn label_jumps(model: &SourceModel, pieces: &[Piece]) -> BTreeMap<String, crate::compile::ClipJumps> {
    let mut out = BTreeMap::new();
    let syms: BTreeSet<u16> = pieces.iter().map(|p| p.sym).collect();
    for sym in syms {
        let Some(c) = model.class_of(sym) else { continue };
        let Some(sprite) = model.sprites.get(&sym) else { continue };
        let own: Vec<&Piece> = pieces.iter().filter(|p| p.sym == sym && !p.borrowed && !p.reversed && p.head == 0).collect();
        let playhead = crate::compile::Playhead {
            starts: own.iter().map(|p| (p.fm_name.clone(), p.from, p.frames)).collect(),
            labels: sprite.labels.iter().map(|(f, l)| (*f as u32 + 1, l.clone())).collect(),
            total: sprite.frames.len() as u32,
        };
        let mut j = crate::compile::ClipJumps { anims: own.iter().map(|p| p.fm_name.clone()).collect(), labels: BTreeMap::new(), autolink: None, playhead: Some(playhead) };
        for (f0, label) in &sprite.labels {
            let f = *f0 as u32 + 1;
            let hits: Vec<&&Piece> = own.iter().filter(|p| f >= p.from && f < p.from + p.frames).collect();
            if let [p] = hits.as_slice() { j.labels.insert(label.clone(), (p.fm_name.clone(), f == p.from)); }
        }
        out.insert(c.name.clone(), j);
    }
    out
}

/// the scales between the two worlds
#[derive(Clone, Copy, Debug)]
pub struct Units {
    pub size: f64,
    pub speed: f64,
    /// the speed the source's engine throws a held item with, in its units
    pub item_toss: f64,
    /// an acceleration of the source's in this engine's units
    pub accel: f64,
}

pub struct Emitted {
    pub script: String,
    pub problems: Vec<String>,
    pub unknown: BTreeSet<String>,
    /// frames whose script goes on their keyframe, by (clip symbol, frame): the class and the
    /// script's function text
    pub frames: BTreeMap<(u16, u32), (String, String)>,
    /// the landings the engine makes, each by the animations it lands in
    pub landings: Vec<(crate::compile::Landing, Vec<String>)>,
    /// the clip whose frames the engine plays as the object's own (`native_clip`), when it is
    pub native: Option<u16>,
}

/// Which frames' scripts move onto their animation's keyframes, for one object: a frame of a clip
/// its animations show, except the frames the runtime runs itself. Those stay the clip's methods:
/// the clip's first frame (it runs as the clip is made), an animation's first frame (its entry
/// runs it), a frame a jump names outright (a jump runs the
/// frame it lands on at once, from the code that made it), a frame of a clip an effect also plays
/// (effects play their clips themselves), and a script method anything else calls by name.
pub struct FramePlan {
    /// class -> the methods written apart
    pub detach: BTreeMap<String, BTreeSet<String>>,
    /// (symbol, frame) -> (class, method)
    pub frame_of: BTreeMap<(u16, u32), (String, String)>,
}

pub fn frame_plan(model: &SourceModel, pieces: &[Piece], effect_syms: &BTreeSet<u16>) -> FramePlan {
    use crate::model::ast::{walk_exprs, Expr};
    // what jumps name outright, anywhere in the package: labels and frame numbers
    let mut labels: BTreeSet<String> = BTreeSet::new();
    let mut numbers: BTreeSet<u32> = BTreeSet::new();
    // how often each class's code asks for each name (a frame script's registration is one of them)
    let mut asked: BTreeMap<(String, String), usize> = BTreeMap::new();
    for c in model.classes.values() {
        for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor, &c.static_init]) {
            walk_exprs(&f.body, &mut |e| {
                match e {
                    Expr::Call(callee, args) => {
                        let jump = match &**callee {
                            Expr::Member(_, m) | Expr::Name(m) => matches!(m.as_str(), "gotoAndPlay" | "gotoAndStop" | "playFrame"),
                            _ => false,
                        };
                        if jump {
                            match args.first() {
                                Some(Expr::Str(l)) => { labels.insert(l.clone()); }
                                Some(Expr::Int(n)) => { numbers.insert(*n as u32); }
                                Some(Expr::Num(n)) => { numbers.insert(*n as u32); }
                                _ => {}
                            }
                        }
                    }
                    Expr::Member(_, n) | Expr::Name(n) => { *asked.entry((c.name.clone(), n.clone())).or_default() += 1; }
                    _ => {}
                }
            });
        }
    }
    let mut plan = FramePlan { detach: BTreeMap::new(), frame_of: BTreeMap::new() };
    let syms: BTreeSet<u16> = pieces.iter().map(|p| p.sym).collect();
    for sym in syms {
        if effect_syms.contains(&sym) { continue }
        let Some(c) = model.class_of(sym) else { continue };
        // the frames the runtime enters a clip on: its first (a clip runs it as it is made), and
        // each animation's first (the last clip frame of one that plays the clip backwards)
        let mut firsts: BTreeSet<u32> = pieces.iter().filter(|p| p.sym == sym)
            .map(|p| if p.reversed { p.from + p.frames - 1 } else { p.from }).collect();
        firsts.insert(1);
        let labelled: BTreeSet<u32> = model.sprites.get(&sym).map(|sp| sp.labels.iter()
            .filter(|(_, l)| labels.contains(l)).map(|(f, _)| *f as u32 + 1).collect()).unwrap_or_default();
        let mut registered: BTreeMap<&str, usize> = BTreeMap::new();
        for fs in &c.frame_scripts { *registered.entry(fs.method.as_str()).or_default() += 1; }
        for fs in &c.frame_scripts {
            let f = fs.frame + 1;
            let once = registered.get(fs.method.as_str()) == Some(&1);
            // asked for only by its registration
            let only_registered = asked.get(&(c.name.clone(), fs.method.clone())).copied().unwrap_or(0) <= 1;
            if firsts.contains(&f) || labelled.contains(&f) || numbers.contains(&f) || !once || !only_registered { continue }
            // a keyframe runs only on a frame an animation shows: a frame no piece of the clip covers
            // (a lead-in its labels start after) keeps its script on the clip
            if !pieces.iter().any(|p| p.sym == sym && f >= p.from && f < p.from + p.frames) { continue }
            if !c.methods.iter().any(|m| m.name == fs.method && !m.is_static) { continue }
            plan.detach.entry(c.name.clone()).or_default().insert(fs.method.clone());
            plan.frame_of.insert((sym, f), (c.name.clone(), fs.method.clone()));
        }
    }
    plan
}

/// The classes an object reaches: its own class and the classes it extends, every clip its
/// clips place (and their classes), and every class the code of those names.
pub fn reachable(model: &SourceModel, runtime: &Runtime, roots: &[String], clip_symbols: &[u16]) -> (BTreeSet<String>, BTreeSet<u16>) {
    let mut classes: BTreeSet<String> = BTreeSet::new();
    let mut symbols: BTreeSet<u16> = BTreeSet::new();
    let mut work_c: Vec<String> = roots.to_vec();
    let mut work_s: Vec<u16> = clip_symbols.to_vec();
    loop {
        if let Some(s) = work_s.pop() {
            if !symbols.insert(s) { continue }
            if let Some(c) = model.symbols.get(&s) { work_c.push(c.clone()) }
            if let Some(sp) = model.sprites.get(&s) {
                for placed in sp.frames.iter().flatten() {
                    let classed = model.symbols.contains_key(&placed.char_id);
                    let is_clip = model.sprites.contains_key(&placed.char_id);
                    if is_clip && (classed || placed.name.is_some()) { work_s.push(placed.char_id) }
                }
            }
            continue;
        }
        let Some(c) = work_c.pop() else { break };
        let Some(cm) = model.classes.get(&c).or_else(|| {
            model.classes.values().find(|x| compile::short(&x.name) == c)
        }) else { continue };
        if runtime.provides(&cm.name) || !classes.insert(cm.name.clone()) { continue }
        if let Some(s) = cm.symbol { work_s.push(s) }
        if !cm.super_name.is_empty() { work_c.push(cm.super_name.clone()) }
        let mut names: BTreeSet<String> = BTreeSet::new();
        let fs = cm.methods.iter().map(|m| &m.function).chain([&cm.constructor, &cm.static_init]);
        for f in fs {
            ast::walk_exprs(&f.body, &mut |e| if let Expr::Name(n) = e { names.insert(n.clone()); });
        }
        for f in &cm.fields {
            if let Some(t) = &f.type_name { names.insert(t.clone()); }
        }
        for n in names {
            if model.classes.contains_key(&n) || model.classes.values().any(|x| compile::short(&x.name) == n) { work_c.push(n) }
        }
    }
    (classes, symbols)
}

/// A declared value as a script literal.
pub fn literal(v: &Value, class_ident: &dyn Fn(&str) -> Option<String>) -> String {
    match v {
        Value::Null | Value::Computed(_) => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => {
            if n.is_nan() { "Math.NaN".into() }
            else if n.fract() == 0.0 && n.abs() < 2147483648.0 { format!("{}", *n as i64) }
            else { let s = format!("{n}"); if s.contains('.') || s.contains('e') { s } else { format!("{s}.0") } }
        }
        Value::Str(s) => compile::quote(s),
        Value::Class(c) => class_ident(c).unwrap_or_else(|| compile::quote(c)),
        Value::Arr(items) => format!("[{}]", items.iter().map(|x| literal(x, class_ident)).collect::<Vec<_>>().join(", ")),
        Value::Obj(kv) => {
            // an object that answers for its own keys: its fields read by name and by a key
            // worked out at run time alike (`AS3.at` asks `__get`); a key that is no plain name
            // lives only in its key map
            let plain = |k: &str| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !k.starts_with(|c: char| c.is_ascii_digit()) && !k.starts_with("__");
            let fields: Vec<String> = kv.iter().filter(|(k, _)| plain(k))
                .map(|(k, x)| format!("{}: {}", compile::ident(k), literal(x, class_ident))).collect();
            let gets: Vec<String> = kv.iter().filter(|(k, _)| plain(k))
                .map(|(k, _)| format!("case {}: return o.{};", compile::quote(k), compile::ident(k))).collect();
            let sets: Vec<String> = kv.iter().filter(|(k, _)| plain(k))
                .map(|(k, _)| format!("case {}: o.{} = v;", compile::quote(k), compile::ident(k))).collect();
            let names: Vec<String> = kv.iter().map(|(k, _)| compile::quote(k)).collect();
            let others: Vec<String> = kv.iter().filter(|(k, _)| !plain(k))
                .map(|(k, x)| format!("AS3.putKey(o, {}, {});", compile::quote(k), literal(x, class_ident))).collect();
            format!("(function() {{ var o = {{ {} }}; o.__names = [{}]; o.__get = function(k) {{ switch (k) {{ {} default: return o.__keys == null ? null : o.__keys.get(k); }} }}; o.__set = function(k, v) {{ switch (k) {{ {} default: AS3.putKey(o, k, v); }} }}; {} return o; }})()",
                fields.join(", "), names.join(", "), gets.join(" "), sets.join(" "), others.join(" "))
        }
    }
}

/// A piece as a script literal.
fn piece_literal(p: &Piece) -> String {
    format!("{{ anim: {}, label: {}, sym: {}, from: {}, frames: {}, head: {}, reversed: {}, borrowed: {}, itemBox: [{}], on: {} }}",
        compile::quote(&p.fm_name), compile::quote(&p.label), p.sym, p.from, p.frames, p.head, p.reversed, p.borrowed,
        p.item_box.iter().map(|(k, x, y)| format!("[{k}, {x}, {y}]")).collect::<Vec<_>>().join(", "),
        p.on.as_deref().map_or("null".to_string(), compile::quote))
}

/// One object the package declares, as its script is written: which kind of engine object it
/// runs on (the entry the runtime gives it), the class that runs it, the clip that is its art,
/// and its own declared stats.
pub struct ObjectSpec<'a> {
    pub kind: &'a str,
    /// the projectiles and items the conversion made that the package does not declare (the
    /// game's own items a fighter generates): their content, for the objects that make them
    pub others: &'a [String],
    /// the effects the conversion made, by linkage: every object can attach one
    pub effects: &'a BTreeMap<String, Vec<Piece>>,
    /// the entity each declared projectile or item was written as (several declarations can
    /// share one clip, and so one entity), by the declaration's name
    pub entities: &'a BTreeMap<String, String>,
    pub class: String,
    pub clip: String,
    /// the clip by its symbol, when it is known that way rather than by a linkage name
    pub clip_sym: Option<u16>,
    pub stats: Value,
    /// the code the autolink calls of a clip's class become (`autolink_calls`), by class
    pub autolink: BTreeMap<String, (String, String)>,
    /// data the conversion worked out for the object outside its package, as script statements
    /// (a stage's boot tables): written with the rest of its data
    pub data: String,
}

/// What a package declares that every object of it reads: a fighter's attacks, projectiles and
/// items, and the objects a stage makes by class (each class by the content it was converted to).
pub struct Decls<'a> {
    pub attacks: &'a Value,
    pub projectiles: &'a Value,
    pub items: &'a Value,
    pub objects: Vec<(String, String)>,
}

impl<'a> Decls<'a> {
    pub fn of(fighter: &'a crate::model::declare::Fighter) -> Self {
        Decls { attacks: &fighter.attacks, projectiles: &fighter.projectiles, items: &fighter.items, objects: Vec::new() }
    }
}

/// The tables every object of a package reads: what the package declares, and the content each
/// declared projectile and item was converted to.
fn package_data(decls: &Decls, others: &[String], entities: &BTreeMap<String, String>,
                ident_of: &dyn Fn(&str) -> Option<String>) -> String {
    let mut contents: Vec<(String, Value)> = Vec::new();
    let names = decls.projectiles.entries().iter().chain(decls.items.entries()).map(|(n, _)| n).chain(others.iter());
    for name in names {
        if contents.iter().any(|(n, _)| n == name) { continue }
        let entity = entities.get(name).unwrap_or(name);
        contents.push((name.clone(), Value::Str(crate::projectile_gen::projectile_content_id(entity))));
    }
    // the objects made by class, only where the package makes any
    let objects = if decls.objects.is_empty() { String::new() } else {
        let kv: Vec<(String, Value)> = decls.objects.iter().map(|(c, id)| (c.clone(), Value::Str(id.clone()))).collect();
        format!(", objects: {}", literal(&Value::Obj(kv), ident_of))
    };
    format!("DATA.pkg = {{ attacks: {}, projectiles: {}, items: {}, contents: {}{objects} }};\n",
        literal(decls.attacks, ident_of), literal(decls.projectiles, ident_of),
        literal(decls.items, ident_of), literal(&Value::Obj(contents), ident_of))
}

/// A fighter's script.
#[allow(clippy::too_many_arguments)]
pub fn fighter_script(model: &SourceModel, runtime: &Runtime, fighter: &crate::model::declare::Fighter,
                      pieces: &[Piece], units: Units, title: &str, others: &[String],
                      effects: &BTreeMap<String, Vec<Piece>>, entities: &BTreeMap<String, String>,
                      autolink: BTreeMap<String, (String, String)>) -> Emitted {
    let spec = ObjectSpec {
        kind: "fighter",
        others,
        effects,
        entities,
        class: fighter.class().unwrap_or_default().to_string(),
        clip: fighter.clip().unwrap_or_default().to_string(),
        clip_sym: None,
        stats: fighter.stats.clone(),
        autolink,
        data: String::new(),
    };
    object_script(model, runtime, &spec, &Decls::of(fighter), pieces, units, title)
}

/// An object's script: the runtime, the classes the object reaches, its data, and its entry.
pub fn object_script(model: &SourceModel, runtime: &Runtime, spec: &ObjectSpec, decls: &Decls,
                     pieces: &[Piece], units: Units, title: &str) -> Emitted {
    let mut out = String::new();
    let mut problems = Vec::new();
    let mut unknown = BTreeSet::new();
    let class = spec.class.clone();
    let clip = spec.clip.clone();
    let clip_sym = spec.clip_sym.or_else(|| model.symbol_of(&clip));
    // the effects' clips too: any object can attach one, and its clip runs here
    let effect_syms: Vec<u16> = spec.effects.values().filter_map(|p| p.first().map(|p| p.sym)).collect();
    // a stage's camera backgrounds are clips of its own, which its code can reach into
    let camera_syms: Vec<u16> = if spec.kind != "stage" { Vec::new() } else {
        model.declared("camera").as_ref().and_then(|c| c.get("backgrounds").cloned())
            .map(|b| match b { Value::Arr(items) => items, _ => Vec::new() }).unwrap_or_default().iter()
            .filter_map(|b| b.get("linkage_id").and_then(|v| v.str()).and_then(|l| model.symbol_of(l))).collect()
    };
    let (classes, symbols) = reachable(model, runtime, std::slice::from_ref(&class),
        &clip_sym.into_iter().chain(effect_syms).chain(camera_syms).collect::<Vec<_>>());
    let pkg = compile::Package::new(model, runtime);

    out.push_str(&format!("// {title}: converted from SSF2 by Peptide.\n//\n// sections, in order: the runtime (copies of runtime/*.hx: edit those, not these), this\n// {}'s classes as the source defines them, its data, and the entry the engine calls.\n\n", spec.kind));

    // the runtime goes here once the rest is written: only what the rest reaches of it
    let runtime_at = out.len();

    // the engine's state containers, made before anything else runs (the engine makes them only
    // at the script's top level, and the classes' statics take theirs as their sections run):
    // the statics, the script's own object's fields, and room for the six largest other classes
    // it carries at once, twice over (a clip and the one replacing it)
    let mut sizes: Vec<(usize, usize)> = Vec::new();
    let (mut own_n, mut own_o) = (0usize, 0usize);
    for m in pkg.chain(&spec.class).0 {
        for f in m.fields.iter().filter(|f| !f.is_static && pkg.stored.contains(&f.name)) {
            if matches!(f.type_name.as_deref().map(compile::short), Some("Number" | "int" | "uint")) { own_n += 1 } else { own_o += 1 }
        }
    }
    for c in &classes {
        let Some(m) = pkg.class_named(c) else { continue };
        for f in m.fields.iter().filter(|f| f.is_static && pkg.stored.contains(&f.name)) {
            if matches!(f.type_name.as_deref().map(compile::short), Some("Number" | "int" | "uint")) { own_n += 1 } else { own_o += 1 }
        }
    }
    for c in &classes {
        let Some(m) = pkg.class_named(c) else { continue };
        if m.name == spec.class { continue }
        let (mut nums, mut objs) = (0usize, 0usize);
        for f in m.fields.iter().filter(|f| !f.is_static && pkg.stored.contains(&f.name)) {
            if matches!(f.type_name.as_deref().map(compile::short), Some("Number" | "int" | "uint")) { nums += 1 } else { objs += 1 }
        }
        sizes.push((nums, objs));
    }
    sizes.sort_by_key(|(n, o)| std::cmp::Reverse(n + o));
    let (nums, objs) = sizes.iter().take(6).fold((0, 0), |a, (n, o)| (a.0 + n, a.1 + o));
    out.push_str(&format!("// ---- state: the engine's containers for everything a rollback puts back\nRollback.start();\nPeptideRuntime.makePool({{ num: {}, obj: {} }});\n\n",
        8 + own_n + 2 * nums, 8 + own_o + 2 * objs));

    // the classes, bases before what extends them
    let mut order: Vec<String> = Vec::new();
    let mut placed: BTreeSet<String> = BTreeSet::new();
    fn place(n: &str, model: &SourceModel, classes: &BTreeSet<String>, placed: &mut BTreeSet<String>, order: &mut Vec<String>) {
        if placed.contains(n) || !classes.contains(n) { return }
        placed.insert(n.to_string());
        if let Some(c) = model.classes.get(n) {
            let base = model.classes.get(&c.super_name).map(|b| b.name.clone())
                .or_else(|| model.classes.values().find(|x| compile::short(&x.name) == c.super_name).map(|b| b.name.clone()));
            if let Some(b) = base { place(&b, model, classes, placed, order) }
        }
        order.push(n.to_string());
    }
    for c in &classes { place(c, model, &classes, &mut placed, &mut order) }
    let effect_syms: BTreeSet<u16> = spec.effects.values().flat_map(|ps| ps.iter().map(|p| p.sym)).collect();
    let plan = frame_plan(model, pieces, &effect_syms);
    let none = BTreeSet::new();
    let mut jumps = label_jumps(model, pieces);
    // a projectile's playhead is the engine's: its code reads where its clip is off the engine
    if spec.kind != "projectile" { for j in jumps.values_mut() { j.playhead = None; } }
    for (c, code) in &spec.autolink { jumps.entry(c.clone()).or_default().autolink = Some(code.clone()); }
    let mut detached: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut found: Vec<crate::compile::Landing> = Vec::new();
    // each class's static initializer, in the order the classes are made (a base before its own)
    let mut cinits: Vec<String> = Vec::new();
    let cinit_at = crate::re!(r"(?m)^([A-Za-z_]\w*)\.cinit = ");
    // whether any of the package's code walks a clip's children by place (the only way code reaches
    // a child placed under no name)
    let mut walks_children = false;
    // which of a clip's rarely asked methods the package's code names anywhere (DATA.clipUses):
    // a clip makes only those, each being an interpreted closure made for every clip the timelines
    // place (seven a fighter's attack). code that names none makes none
    // (any of the package's code: a clip made in one script is reached from another, as a stage's
    // code moves the clip of an object it spawned, so each script makes what any of them names)
    let mut clip_uses: BTreeSet<&str> = BTreeSet::new();
    for cm in model.classes.values().filter(|cm| !runtime.provides(&cm.name)) {
        for f in cm.methods.iter().map(|m| &m.function).chain([&cm.constructor, &cm.static_init]) {
            ast::walk_exprs(&f.body, &mut |e| if let Expr::Member(_, n) = e {
                if let Some((m, _)) = CLIP_OPTIONAL.iter().find(|(_, needle)| needle == n) { clip_uses.insert(m); }
            });
        }
    }
    // every name the package's code reads (a member, a bare name) and every text it holds: a named
    // child none of them can reach is not made (below)
    let mut read_names: BTreeSet<String> = BTreeSet::new();
    // (a text that is only ever an object's key is data, as an attack's stats name its boxes)
    let mut said: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut starts: BTreeSet<String> = BTreeSet::new();
    for cm in model.classes.values().filter(|cm| !runtime.provides(&cm.name)) {
        for f in cm.methods.iter().map(|m| &m.function).chain([&cm.constructor, &cm.static_init]) {
            ast::walk_exprs(&f.body, &mut |e| match e {
                Expr::Member(_, n) | Expr::Name(n) => { read_names.insert(n.clone()); }
                Expr::Str(s) if !s.is_empty() => { said.entry(s.clone()).or_default().0 += 1; }
                Expr::Object(pairs) => for (k, _) in pairs {
                    if let Expr::Str(s) = k { said.entry(s.clone()).or_default().1 += 1; }
                },
                // a name built as the code runs starts with the text on the left of its `+`
                Expr::Binary(ast::BinOp::Add, l, _) => {
                    let mut l = &**l;
                    while let Expr::Binary(ast::BinOp::Add, x, _) = l { l = &**x; }
                    if let Expr::Str(s) = l { if !s.is_empty() { starts.insert(s.clone()); } }
                }
                _ => {}
            });
        }
    }
    let texts: BTreeSet<String> = said.into_iter().filter(|(_, (all, keys))| all > keys).map(|(s, _)| s).collect();
    for c in &order {
        let Some((text, p, u, d, l)) = compile::compile_class_detached(model, runtime, c, plan.detach.get(c).unwrap_or(&none), jumps.get(c).cloned()) else { continue };
        cinits.extend(cinit_at.captures_iter(&text).map(|m| format!("{}.cinit", &m[1])));
        found.extend(l);
        problems.extend(p.into_iter().map(|x| format!("{c}: {x}")));
        unknown.extend(u);
        if !runtime.provides(c) {
            for (method, needle) in CLIP_OPTIONAL {
                if text.contains(needle) || d.iter().any(|(_, t)| t.contains(needle)) { clip_uses.insert(method); }
            }
        }
        walks_children |= !runtime.provides(c) && WALKS_CHILDREN.iter().any(|n| text.contains(n) || d.iter().any(|(_, t)| t.contains(n)));
        out.push_str(&text);
        out.push('\n');
        for (m, t) in d { detached.insert((c.clone(), m), t); }
    }
    let frames: BTreeMap<(u16, u32), (String, String)> = plan.frame_of.iter()
        .filter_map(|(k, (c, m))| Some((*k, (c.clone(), detached.get(&(c.clone(), m.clone()))?.clone())))).collect();

    // the data
    // a class the data names: its section when this script carries it, else its name
    let ident_of = |n: &str| {
        let carried = runtime.provides(n) || pkg.class_named(n).is_some_and(|c| classes.contains(&c.name));
        if carried { pkg.class_ident(n) } else { None }
    };
    out.push_str(&format!("// ---- data: {title} (generated from the package)\nvar DATA = {{}};\n"));
    // the statics the classes work out, set once as the object starts (`PeptideRuntime.statics`)
    out.push_str(&format!("DATA.cinits = [{}];\n", cinits.join(", ")));
    out.push_str(&format!("DATA.units = {{ size: {}, speed: {}, itemToss: {}, accel: {} }};\n", units.size, units.speed, units.item_toss, units.accel));
    // the source's events the engine raises, by name (runtime/events.jsonc)
    out.push_str(&compile::events::table().data());
    out.push_str(&format!("DATA.clipUses = {{ {} }};\n", clip_uses.iter().map(|m| format!("{m}: true")).collect::<Vec<_>>().join(", ")));
    out.push_str("DATA.timelines = new haxe.ds.IntMap();\nDATA.classes = new haxe.ds.IntMap();\n");
    // the library bitmaps the timelines place, each a picture whose data is its class
    let mut bitmaps: BTreeSet<u16> = BTreeSet::new();
    // the clips whose frames place any children here
    let mut placing: BTreeSet<u16> = BTreeSet::new();
    for s in &symbols {
        let Some(sp) = model.sprites.get(s) else { continue };
        // a frame's children as logic sees them: the clips it places that have a class or a name,
        // and anything else it places under a name (a text field the code writes to: devil's
        // machine types into one), which is a clip with nothing in it here. a bitmap with a class
        // and no name is there only for code that walks the children (race to the finish asks a
        // placed picture its class): with none, a fighter's art frames are not made and dropped
        // as its clips play
        let mut names: BTreeSet<String> = BTreeSet::new();
        // a named child no code can reach (nothing reads its name, holds it as a text, builds it
        // from a text it starts with or walks the children), whose clip keeps to itself, is not
        // made: a fighter's frames each place its boxes under names its code never asks for (an
        // attack's stats name them as keys, which is data), and making them was most of the cost
        // of a change of state. a stage's planes are found by name, so a stage
        // makes every one; the runtime asks for a clip's `stance` and `xframe` itself
        let unreached = |p: &&crate::swf_timeline::Placed| -> bool {
            let Some(n) = &p.name else { return false };
            spec.kind != "stage" && !walks_children && n != "stance" && n != "xframe"
                && !read_names.contains(n) && !texts.contains(n) && !starts.iter().any(|t| n.starts_with(t.as_str()))
                && keeps_to_itself(model, &pkg, p.char_id, &mut BTreeSet::new())
        };
        let frames: Vec<String> = sp.frames.iter().map(|placed| {
            let kids: Vec<String> = placed.iter().filter(|p| {
                (model.sprites.contains_key(&p.char_id) && model.symbols.contains_key(&p.char_id)) || p.name.is_some()
                    || (walks_children && model.bitmaps.contains_key(&p.char_id) && model.symbols.contains_key(&p.char_id))
            }).filter(|p| !unreached(p)).map(|p| {
                if model.bitmaps.contains_key(&p.char_id) { bitmaps.insert(p.char_id); }
                if let Some(n) = &p.name { names.insert(n.clone()); }
                // where the frame puts it (x, y, scale x and y, rotation in degrees), when it moves it
                // off the parent's origin: code reads a placed child's place (target test luigi's
                // damage zones stand where their markers do)
                let m = &p.matrix;
                let at = if *m == crate::swf_timeline::Matrix::default() { String::new() } else {
                    let sx = (m.a * m.a + m.b * m.b).sqrt();
                    let sy = (m.c * m.c + m.d * m.d).sqrt() * if m.a * m.d - m.b * m.c < 0.0 { -1.0 } else { 1.0 };
                    let rot = m.b.atan2(m.a).to_degrees();
                    let n = |v: f64| { let r = (v * 1000.0).round() / 1000.0; if r == 0.0 { "0".to_string() } else { format!("{r}") } };
                    format!(", [{}, {}, {}, {}, {}]", n(m.tx), n(m.ty), n(sx), n(sy), n(rot))
                };
                format!("[{}, {}, {}{at}]", p.depth, p.char_id, p.name.as_deref().map(compile::quote).unwrap_or_else(|| "null".into()))
            }).collect();
            format!("[{}]", kids.join(", "))
        }).collect();
        if frames.iter().any(|f| f != "[]") { placing.insert(*s); }
        let labels: Vec<String> = sp.labels.iter().map(|(f, l)| format!("[{}, {}]", f + 1, compile::quote(l))).collect();
        // the named children go to the fields of the same name
        let bind = if names.is_empty() { "null".to_string() } else {
            let cases: Vec<String> = names.iter().map(|n| {
                let id = compile::ident(n);
                if pkg.stored.contains(n) {
                    format!("case {}: if (me.__c_{id} != null) me.__c_{id}.set(c); else me.{id} = c;", compile::quote(n))
                } else {
                    format!("case {}: me.{id} = c;", compile::quote(n))
                }
            }).collect();
            format!("function(me, n, c) {{ switch (n) {{ {} }} }}", cases.join(" "))
        };
        out.push_str(&format!("DATA.timelines.set({s}, {{ frames: [{}], labels: [{}], bind: {bind} }});\n", frames.join(", "), labels.join(", ")));
        if let Some(c) = model.symbols.get(s) {
            if let Some(id) = pkg.class_ident(c).filter(|_| classes.contains(c) || runtime.provides(c)) {
                out.push_str(&format!("DATA.classes.set({s}, {id});\n"));
            }
        }
    }
    if !bitmaps.is_empty() {
        out.push_str("DATA.bitmaps = new haxe.ds.IntMap();\n");
        for b in &bitmaps {
            let (w, h) = model.bitmaps[b];
            out.push_str(&format!("DATA.bitmaps.set({b}, {{ name: {}, w: {w}, h: {h} }});\n", compile::quote(&model.symbols[b])));
        }
    }
    let pieces_lit: Vec<String> = pieces.iter().map(piece_literal).collect();
    out.push_str(&format!("DATA.pieces = [\n\t{}\n];\n", pieces_lit.join(",\n\t")));
    // the animation a label starts, from any other animation of the same clip: where a landing
    // made by code that is not the clip's own (the fighter's landing lag) takes the fighter
    let mut lands: Vec<String> = Vec::new();
    for j in label_jumps(model, pieces).values() {
        for (label, (anim, start)) in &j.labels {
            if !start { continue }
            for from in j.anims.iter().filter(|a| *a != anim) {
                lands.push(format!("case {}: return {};", compile::quote(&format!("{label}/{from}")), compile::quote(anim)));
            }
        }
    }
    lands.sort();
    lands.dedup();
    // each landing by the animations it lands in: its clip's own, or every animation its label
    // starts (a landing the fighter's own code sets up for whatever move it is in)
    let landings: Vec<(crate::compile::Landing, Vec<String>)> = found.into_iter().map(|l| {
        let anims: BTreeSet<String> = match &l.anim {
            Some(a) => [a.clone()].into(),
            None => jumps.values().filter_map(|j| match j.labels.get(&l.label) { Some((a, true)) => Some(a.clone()), _ => None }).collect(),
        };
        (l, anims.into_iter().collect())
    }).collect();
    let landed: BTreeSet<&String> = landings.iter().flat_map(|(_, a)| a.iter()).collect();
    out.push_str(&format!("DATA.isLanding = function(a) {{ switch (a) {{ {} default: return false; }} }};\n",
        landed.iter().map(|a| format!("case {}: return true;", compile::quote(a))).collect::<Vec<_>>().join(" ")));
    out.push_str(&format!("DATA.lands = function(label, from) {{ switch (label + \"/\" + from) {{ {} default: return null; }} }};\n", lands.join(" ")));
    out.push_str(&format!("DATA.object = {{ kind: {}, linkage: {}, cls: {}, clip: {}, stats: {} }};\n",
        compile::quote(spec.kind), compile::quote(&clip), ident_of(&class).unwrap_or_else(|| "null".into()),
        clip_sym.map(|s| s.to_string()).unwrap_or_else(|| "null".into()), literal(&spec.stats, &ident_of)));
    out.push_str(&package_data(decls, spec.others, spec.entities, &ident_of));
    // what the conversion worked out for this object outside its package (a stage's boot)
    out.push_str(&spec.data);
    // how a hitbox's stats cross between the source's fields and this engine's (the measured
    // factors in mappings/character/hitbox_stats.jsonc)
    let hb = crate::mappings::character_hitbox_stats();
    let rows: Vec<String> = hb.fields.iter().filter(|f| !f.ssf2_keys.is_empty()).map(|f| format!(
        "{{ fm: {}, keys: [{}], scale: {}, frames: {}, plus: {}, perDamage: {}, constant: {} }}", compile::quote(&f.fm_field),
        f.ssf2_keys.iter().map(|k| compile::quote(k)).collect::<Vec<_>>().join(", "),
        f.scale.map(|v| v.to_string()).unwrap_or_else(|| "null".into()), f.isframe, f.plus_frames,
        f.unstated_per_damage.map(|v| v.to_string()).unwrap_or_else(|| "null".into()),
        f.unstated_constant.map(|v| v.to_string()).unwrap_or_else(|| "null".into()))).collect();
    out.push_str(&format!("DATA.hitboxFields = [\n\t{}\n];\nDATA.frameRatio = {};\n", rows.join(",\n\t"), crate::entity_gen::FRAME_RATIO));
    let flags: Vec<String> = hb.flags.iter().map(|f| format!("{{ key: {}, when: {}, fm: {}, value: {} }}",
        compile::quote(&f.ssf2_key), f.when, compile::quote(&f.fm_field), f.value)).collect();
    out.push_str(&format!("DATA.hitboxFlags = [{}];\n", flags.join(", ")));
    // ...and the launch a FIXED (`weightKB`) hit gives, which is not the field's own number
    let fixed = hb.fields.iter().find(|f| f.fm_field == "fixed_knockback")
        .and_then(|f| Some(format!("{{ perUnit: {}, constant: {} }}", f.per_unit?, f.constant?)))
        .unwrap_or_else(|| "null".into());
    out.push_str(&format!("DATA.fixedKnockback = {fixed};\n"));
    // its entity's keyframes run its frames, and the engine plays them (see `clock_entity`)
    // a fighter's stats as this engine keeps them: the names the source reads them by, the unit
    // of each, and a record of one stat for a write (mappings/character/stats.jsonc)
    {
        let st = crate::mappings::character_stats();
        let mut names: Vec<(String, Value)> = st.fm_runtime_read_names.iter().map(|(k, v)| (k.clone(), Value::Str(v.clone()))).collect();
        names.push(("weight1".into(), Value::Str("weight".into())));
        let units: Vec<String> = st.runtime_read_units.iter().map(|(u, list)| format!("{}: [{}]", compile::ident(u),
            list.iter().map(|n| compile::quote(n)).collect::<Vec<_>>().join(", "))).collect();
        let cases: Vec<String> = st.fm_runtime_stats.iter().filter(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .map(|n| format!("case {}: return {{ {n}: v }};", compile::quote(n))).collect();
        // what a read answers for a stat no character states (stats.jsonc `runtime_read_defaults`)
        let defaults: Vec<String> = st.runtime_read_defaults.iter()
            .map(|(k, v)| format!("case {}: return {v};", compile::quote(k))).collect();
        out.push_str(&format!("DATA.stats = {{ names: {}, units: {{ {} }}, defaults: function(k) {{ switch (k) {{ {} default: return null; }} }}, record: function(k, v) {{ switch (k) {{ {} default: return null; }} }} }};\n",
            literal(&Value::Obj(names), &|_| None), units.join(", "), defaults.join(", "), cases.join(" ")));
    }
    // the API calls its code can reach by a name it builds as it runs
    if let Some(t) = compile::api_lookup_table(model, runtime) { out.push_str(&t); }
    // the shared effects its code names, as this engine's global ones (mappings/commands.jsonc)
    out.push_str("DATA.globalVfx = new haxe.ds.StringMap();\n");
    let lits = compile::string_literals(model);
    for (name, constant) in &crate::mappings::api_commands().global_vfx_map {
        if lits.contains(name) {
            out.push_str(&format!("DATA.globalVfx.set({}, GlobalVfx.{constant});\n", compile::quote(name)));
        }
    }
    // the effects, by linkage: the clip each runs and the animations that show it
    out.push_str("DATA.effects = new haxe.ds.StringMap();\n");
    for (name, ps) in spec.effects {
        out.push_str(&format!("DATA.effects.set({}, [{}]);\n", compile::quote(name),
            ps.iter().map(piece_literal).collect::<Vec<_>>().join(", ")));
    }
    // a stage's object can have no clip by design (a zone its code sizes): it runs on an empty one
    if clip_sym.is_none() && spec.kind != "object" { problems.push(format!("the {} clip {clip} is no symbol", spec.kind)); }

    // a projectile's own frame work: only the pieces its declaration or its code names
    if spec.kind == "projectile" {
        // what it says itself: its declared stats, its own class and the ones it extends, and the
        // code on its own frames (not the package's other classes, which every script carries)
        let mut text = format!("{:?}", spec.stats);
        // (its code is the classes bound to its clip and to the clips placed inside it, where the
        // declaration names a base: pikachu's jolt hops from a clip it places)
        let tree = clip_tree(model, clip_sym);
        let own: Vec<String> = std::iter::once(class.clone())
            .chain(tree.iter().filter_map(|s| model.class_of(*s)).map(|c| c.name.clone()))
            .flat_map(|n| { let mut v = vec![n.clone()]; v.extend(model.supers(&n)); v }).collect();
        // (the API classes it extends are the runtime's, which names everything)
        for name in own {
            if runtime.provides(&name) { continue }
            let Some(c) = model.classes.get(&name) else { continue };
            for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor, &c.static_init]) {
                ast::walk_exprs(&f.body, &mut |e| if let Expr::Member(_, n) | Expr::Name(n) | Expr::Str(n) = e { text.push(' '); text.push_str(n); });
            }
        }
        for ((s, _), (_, t)) in &frames { if tree.contains(s) { text.push_str(t) } }
        let item = decls.items.entries().iter().any(|(n, d)| n == title
            || d.get("linkage_id").and_then(|v| v.str()) == Some(clip.as_str()));
        out.push_str(&projectile_frames(&text, item));
    }
    let native = if spec.kind == "projectile" { native_clip(model, pieces, &placing) } else { None };
    if let Some(sym) = native {
        // and each animation's next: the one that shows the clip's next frame once it is played
        // out (the clip plays on into its next section; a projectile's animation that ends on its
        // own ends the projectile, so the update moves it on)
        out.push_str("DATA.native = true;\nDATA.sections = new haxe.ds.StringMap();\n");
        let total = model.sprites.get(&sym).map(|s| s.frames.len() as u32).unwrap_or(0);
        for p in pieces.iter().filter(|p| !p.borrowed) {
            let next = if p.from + p.frames > total { 1 } else { p.from + p.frames };
            let Some(q) = pieces.iter().find(|q| !q.borrowed && next >= q.from && next < q.from + q.frames) else { continue };
            if q.fm_name != p.fm_name {
                out.push_str(&format!("DATA.sections.set({}, {});\n", compile::quote(&p.fm_name), compile::quote(&q.fm_name)));
            }
        }
    }
    // the entry: what the engine calls
    let entry = format!("entry/{}.hx", spec.kind);
    match runtime.files.get(&entry) {
        Some(text) => out.push_str(&format!("\n// ---- runtime: runtime/{entry}\n{text}")),
        None => problems.push(format!("the runtime has no {entry}")),
    }
    // the runtime: its language layer first, then everything else in path order, pruned to what
    // this object's script and the code on its keyframes reach
    let mut files: Vec<(&str, &str)> = runtime.files.iter().filter(|(p, _)| !p.starts_with("entry/"))
        .map(|(p, t)| (p.as_str(), t.as_str())).collect();
    files.sort_by_key(|(p, _)| (!p.starts_with("core/as3"), !p.starts_with("core/"), p.to_string()));
    let mut seed = out[runtime_at..].to_string();
    for (_, text) in frames.values() { seed.push_str(text); }
    // what the entity's keyframes call: the engine's own clip's frames call nothing of the runtime's
    // clock (`clock_entity`)
    seed.push_str(if native.is_some() { " PeptideRuntime.shownClip PeptideRuntime.frameDone PeptideRuntime.passing " }
        else { " PeptideRuntime.kf PeptideRuntime.at PeptideRuntime.runAt " });
    let kept = shake::shake(&files, &seed);
    let kept = format!("{kept}{}", rollback_fields(&kept));
    out.insert_str(runtime_at, &kept);
    Emitted { script: out, problems, unknown, frames, landings, native }
}

/// The fighter's script, written after `haxe_gen::generate` has written everything else (art,
/// boxes, stats, the splits, and an entity with every frame and no code on it).
pub fn runtime_fighter(char_dir: &std::path::Path, char_name: &str, char_pascal: &str,
                       data: &crate::extractor::CharacterData,
                       sprite_boxes: &BTreeMap<String, crate::sprite_parser::AnimationBoxData>,
                       input_path: &std::path::Path) -> anyhow::Result<Emitted> {
    let runtime = Runtime::locate().map(|d| Runtime::load(&d)).unwrap_or_default();
    let (model, renamed) = runtime.read_package(&std::fs::read(input_path)?)?;
    let fighters = model.fighters();
    let fighter = fighters.iter()
        .find(|f| f.stats.get("statsName").and_then(|v| v.str()).is_some_and(|n| n.eq_ignore_ascii_case(char_name))
            || f.getter.to_lowercase() == format!("get{}", char_name.to_lowercase()))
        .or(fighters.first())
        .ok_or_else(|| anyhow::anyhow!("the package declares no fighter"))?;
    let splits = crate::anim_splitter::split_animations(&data.animations, sprite_boxes,
        data.stats.jump_startup.round().max(0.0) as u16, &data.scripts);
    let mut unplaced: Vec<String> = renamed;
    // each label's own clip length, as the fighter's timeline places it
    let anims = fighter.clip().map(|c| model.animations(c)).unwrap_or_default();
    let stance_len: BTreeMap<String, u16> = anims.iter()
        .filter_map(|a| Some((a.label.clone(), model.sprites.get(&a.stance_symbol?)?.frame_count as u16))).collect();
    // the clip each label places as `stance`, whose frames its animations show
    let stance_of: BTreeMap<String, u16> = anims.iter().filter_map(|a| Some((a.label.clone(), a.stance_symbol?))).collect();
    let pieces: Vec<Piece> = splits.iter().filter_map(|s| {
        // where the piece's frames are: its sprite's place in the label's clip, or, for a label
        // whose clip no sprite was cut from, the label's clip itself from its first frame
        let item_box = |start: u16, frames: u16| -> Vec<(u32, f64, f64)> {
            let Some(b) = sprite_boxes.get(&s.source_anim) else { return vec![] };
            let mut out: Vec<(u32, f64, f64)> = Vec::new();
            for k in 0..frames {
                let Some(fb) = b.frames.get(&(start + k)).and_then(|v| v.iter().find(|x| matches!(x.box_type, crate::sprite_parser::BoxType::ItemBox))) else { continue };
                let (x, y) = (fb.x + fb.width / 2.0, fb.y + fb.height / 2.0);
                if out.last().is_none_or(|l| l.1 != x || l.2 != y) { out.push((k as u32, x, y)); }
            }
            out
        };
        let (label, offset, total, whole) = match sprite_boxes.get(&s.source_anim) {
            Some(b) => (b.ssf2_name.clone(), b.sprite_frame_offset as u32, b.total_frames, false),
            None => {
                let Some(label) = crate::entity_gen::ssf2_source_name(data, &s.source_anim, &s.fm_name) else {
                    unplaced.push(format!("{} (from {}): no source label", s.fm_name, s.source_anim));
                    return None;
                };
                let total = stance_len.get(&label).copied().unwrap_or(1);
                // the split was cut without the clip's frames to go by: the piece is the whole clip
                (label, 0, total, true)
            }
        };
        let start = if whole { 0 } else { s.start_frame };
        let end = if whole || s.end_frame == u16::MAX { total } else { s.end_frame };
        let Some(&sym) = stance_of.get(&label) else {
            unplaced.push(format!("{} (label {label}): the label places no stance", s.fm_name));
            return None;
        };
        Some(Piece {
            fm_name: s.fm_name.clone(),
            sym,
            from: offset + start as u32 + 1,
            frames: end.saturating_sub(start).max(1) as u32,
            head: s.append_head_frames as u32,
            reversed: s.reversed,
            borrowed: data.ssf2_to_fm_anim.get(&label).is_some_and(|own| *own != s.source_anim),
            item_box: item_box(start, end.saturating_sub(start)),
            on: s.plays_on.clone(),
            label,
        })
    }).collect();
    // the engine plays only what its entity has: a piece the entity left out is reported
    let has = entity_animations(&char_dir.join(format!("library/entities/{char_pascal}.entity")));
    let (pieces, missing): (Vec<Piece>, Vec<Piece>) = pieces.into_iter().partition(|p| has.contains(&p.fm_name));
    unplaced.extend(missing.iter().map(|p| format!("{} (label {}): the entity has no such animation", p.fm_name, p.label)));
    let scaling = &crate::mappings::character_stats().scaling;
    let units = Units { size: scaling.size_multiplier, speed: scaling.velocity_scale(), item_toss: scaling.item_toss_speed, accel: scaling.accel_scale() };
    let mut projectile_pieces = crate::entity_gen::take_projectile_pieces();
    // the entity each declaration was written as: by its own name, else by its clip's
    let mut entities: BTreeMap<String, String> = BTreeMap::new();
    for (name, decl) in fighter.projectiles.entries().iter().chain(fighter.items.entries()) {
        let linkage = decl.get("linkage_id").and_then(|v| v.str()).map(String::from);
        let key = if projectile_pieces.contains_key(name) { Some(name.clone()) }
            else { linkage.filter(|l| projectile_pieces.contains_key(l)) };
        if let Some(k) = key { entities.insert(name.clone(), k); }
    }
    // an entity is the fighter's own when a declaration is written as it or names it as its clip
    let clips: BTreeSet<String> = fighter.projectiles.entries().iter().chain(fighter.items.entries())
        .filter_map(|(_, d)| d.get("linkage_id").and_then(|v| v.str()).map(String::from)).collect();
    let matched: BTreeSet<&String> = entities.values().collect();
    let others: Vec<String> = projectile_pieces.keys().filter(|n| !matched.contains(n)).cloned().collect();
    for n in others.iter().filter(|n| !clips.contains(*n)) {
        unplaced.push(format!("{n}: made from another package, its script is the current pipeline's"));
    }
    // an enemy the fighter makes by its class (`spawnEnemy(WaluigiPlant)`) is the content its class
    // names as its clip in its own stats (`ownStats`'s `linkage_id`): found under the class's name too
    let mut others = others;
    for c in model.classes.values() {
        let Some(m) = c.methods.iter().find(|m| m.name == "ownStats" && !m.is_static) else { continue };
        let Some(stats) = crate::model::declare::returned(&m.function.body) else { continue };
        let Some(linkage) = stats.get("linkage_id").and_then(|v| v.str()) else { continue };
        let Some(key) = projectile_pieces.keys().find(|k| k.eq_ignore_ascii_case(linkage)).cloned() else { continue };
        let short = compile::short(&c.name).to_string();
        if entities.contains_key(&short) { continue }
        entities.insert(short.clone(), key);
        others.push(short);
    }
    let effects = crate::entity_gen::take_effect_pieces();
    let dir = char_dir.join(format!("library/scripts/{char_pascal}"));
    let entity_path = char_dir.join(format!("library/entities/{char_pascal}.entity"));
    // the source's autolink as the engine's own (runtime/autolink.jsonc)
    let links = autolink_plan(&model, &pieces);
    let calls = {
        let json: serde_json::Value = std::fs::read(&entity_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let fields: Vec<String> = compile::autolink::table().box_fields.iter().map(|(k, _)| k.clone()).collect();
        let stated = stated_fields(&std::fs::read_to_string(dir.join("HitboxStats.hx")).unwrap_or_default(), &fields);
        autolink_calls(&links, &entity_hitboxes(&json), &stated)
    };
    for l in &links { unplaced.extend(l.dropped.iter().map(|d| format!("{} autolink, {d}: the engine's autolink has no such setting", l.anim))); }
    let out = fighter_script(&model, &runtime, fighter, &pieces, units, char_name, &others, &effects, &entities, calls);
    // the animation each of the source's labels is here (an attack started from outside the
    // fighter's code is entered by it)
    let anim_of: Vec<String> = data.ssf2_to_fm_anim.iter()
        .map(|(l, a)| format!("case {}: return {};", compile::quote(l), compile::quote(a))).collect();
    let script = format!("{}DATA.animOf = function(l) {{ switch (l) {{ {} default: return null; }} }};\n", out.script, anim_of.join(" "));
    std::fs::write(dir.join("Script.hx"), &script)?;
    let mut cancels = attack_cancels(data, &model, &pieces, units);
    // what the source's engine does in its own states, on the first frame of the animation that
    // shows each (runtime/engine_animations.jsonc)
    for (anim, code) in &compile::engine_anims::table().code {
        if !pieces.iter().any(|p| &p.fm_name == anim) { continue }
        let c = cancels.entry(anim.clone()).or_default();
        if !c.is_empty() { c.push('\n'); }
        c.push_str(code);
    }
    clock_entity(&char_dir.join(format!("library/entities/{char_pascal}.entity")), char_name, &model, &pieces, &out.frames, &cancels, None)?;
    landing_stats(&dir, &out.landings, &label_jumps(&model, &pieces))?;
    // what a clip's frame writes of its boxes as it starts an animation is that animation's own
    let box_writes = entry_box_writes(&model, &pieces);
    landing_stats(&dir, &box_writes, &label_jumps(&model, &pieces))?;
    autolink_entity(&entity_path, &dir.join("HitboxStats.hx"), char_name, &links, units.size)?;
    engine_animations(&entity_path, &dir.join("AnimationStats.hx"), char_name)?;
    // each projectile and item the fighter declares runs on the runtime too
    let projectiles = runtime_projectiles(char_dir, &model, &runtime, fighter, units, &others,
        &effects, &entities, &mut projectile_pieces, &mut unplaced)?;
    let report = serde_json::json!({
        "problems": out.problems.iter().chain(projectiles.iter().flat_map(|(_, e)| e.problems.iter())).collect::<Vec<_>>(),
        "unplaced": unplaced,
        "unknown": out.unknown.iter().chain(projectiles.iter().flat_map(|(_, e)| e.unknown.iter())).collect::<BTreeSet<_>>(),
        "projectiles": projectiles.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        // the box stats an animation took from what its clip writes as it starts (by the source's
        // box number and field names), so a check against the declared boxes knows them
        "box_writes": box_writes.iter().map(|(l, _)| (l.anim.clone().unwrap_or_default(), serde_json::Value::Object(
            l.boxes.iter().map(|(i, f)| (i.to_string(), serde_json::Value::Object(
                f.iter().map(|(k, v)| (k.clone(), serde_json::json!(v))).collect()))).collect()))).collect::<serde_json::Map<_, _>>(),
    });
    std::fs::write(char_dir.join("runtime_report.json"), serde_json::to_string_pretty(&report)?)?;
    // the plan the verify gates hold the fighter to gains the walk speeds its walk clips set
    let plan_path = char_dir.join("verify.json");
    if let Some(mut plan) = std::fs::read(&plan_path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
        plan["walk_script_speeds"] = serde_json::json!(walk_script_speeds(&model, fighter, &pieces, units.speed));
        std::fs::write(&plan_path, serde_json::to_string_pretty(&plan)?)?;
    }
    Ok(out)
}

/// An entity's animations carry the source's frames on their keyframes: each animation's first
/// keyframe enters it, and the first engine frame of every source frame with a script runs that
/// frame (`PeptideRuntime.kf`). The engine plays the animation; the runtime follows it.
///
/// An object whose clip the engine plays as its own (`native`: the clip, and whether its first
/// frame waits for the engine's second frame) has each frame's code on its keyframe instead: the
/// code itself, or the clip's own method for that frame, run on the clip.
pub(crate) fn clock_entity(path: &std::path::Path, key: &str, model: &SourceModel, pieces: &[Piece],
                frames: &BTreeMap<(u16, u32), (String, String)>, extra: &BTreeMap<String, String>,
                native: Option<(u16, bool)>) -> anyhow::Result<()> {
    let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    let mut json: serde_json::Value = serde_json::from_slice(&bytes)?;
    let scripted = |sym: u16, f: u32| model.class_of(sym)
        .is_some_and(|c| c.frame_scripts.iter().any(|fs| fs.frame + 1 == f));
    // a keyframe's code: the frame's script itself when it moved here, else the runtime's
    // trigger, with a note saying what runs
    let code = |sym: u16, f: u32, entry: bool| -> String {
        if let Some((class, text)) = frames.get(&(sym, f)) {
            // the clip is handed to the script as its argument: a keyframe's own variables are not
            // reliably its closure's (the engine runs keyframe code in a scope it reuses)
            let text = text.replacen("function()", "function(me)", 1);
            return format!("// frame {f} of {class}, run as the engine enters it\nPeptideRuntime.runAt(PeptideRuntime.at({sym}, {f}), {text});");
        }
        let method = model.class_of(sym).and_then(|c| c.frame_scripts.iter().find(|fs| fs.frame + 1 == f).map(|fs| (c.name.clone(), fs.method.clone())));
        let why = match (&method, entry) {
            (Some((c, m)), true) => format!("// the animation's first frame: the runtime enters it and runs {c}'s `{m}`"),
            (None, true) => "// the animation's first frame: the runtime enters it".to_string(),
            (Some((c, m)), false) => format!("// frame {f}: {c}'s `{m}`, kept a method because a jump names this frame"),
            (None, false) => format!("// frame {f}"),
        };
        format!("{why}\nPeptideRuntime.kf({sym}, {f});")
    };
    // a frame's code on its keyframe, run on the clip: the frame's own script, or its method
    let native_code = |sym: u16, f: u32| -> Option<String> {
        if let Some((class, text)) = frames.get(&(sym, f)) {
            let text = text.replacen("function()", "function(me)", 1);
            return Some(format!("// frame {f} of {class}\n({text})(PeptideRuntime.shownClip());\nPeptideRuntime.frameDone();"));
        }
        let (c, m) = model.class_of(sym).and_then(|c| c.frame_scripts.iter().find(|fs| fs.frame + 1 == f).map(|fs| (c.name.clone(), fs.method.clone())))?;
        Some(format!("// frame {f} of {c}: its `{m}`\nPeptideRuntime.shownClip().{}();\nPeptideRuntime.frameDone();", compile::ident(&m)))
    };
    let mut new_keyframes: Vec<serde_json::Value> = Vec::new();
    let mut new_layers: Vec<serde_json::Value> = Vec::new();
    let mut layer_of: BTreeMap<String, String> = BTreeMap::new();
    let mut label_of: BTreeMap<String, String> = BTreeMap::new();
    {
        let kfs: BTreeMap<String, u64> = json["keyframes"].as_array().map(|a| a.iter().filter_map(|k|
            Some((k["$id"].as_str()?.to_string(), k["length"].as_u64().unwrap_or(1)))).collect()).unwrap_or_default();
        let lens: BTreeMap<String, u64> = json["layers"].as_array().map(|a| a.iter().filter_map(|l| {
            let n: u64 = l["keyframes"].as_array()?.iter().filter_map(|k| kfs.get(k.as_str()?)).sum();
            Some((l["$id"].as_str()?.to_string(), n))
        }).collect()).unwrap_or_default();
        for p in pieces {
            let Some(anim) = json["animations"].as_array().and_then(|a| a.iter().find(|x| x["name"] == p.fm_name.as_str())) else { continue };
            let total = anim["layers"].as_array().map(|ls| ls.iter().filter_map(|l| lens.get(l.as_str()?)).copied().max().unwrap_or(0)).unwrap_or(0) as u32;
            if total == 0 { continue }
            // (engine frame, source frame) for each keyframe: the entry, then the scripted frames
            let mut at: Vec<(u32, u32)> = vec![(0, if p.reversed { p.from + p.frames - 1 } else { p.from })];
            // the engine's own clip: only frames with code, and the clip's first frame on the
            // engine's second when it must wait for its maker
            let mine = native.filter(|(s, _)| *s == p.sym);
            if let Some((_, wait)) = mine {
                at.clear();
                for k in 0..p.frames {
                    let f = p.from + k;
                    if native_code(p.sym, f).is_none() { continue }
                    at.push((if f == 1 && wait { 1 } else { 2 * k }, f));
                }
            }
            for k in (1..p.frames).filter(|_| mine.is_none()) {
                let f = if p.reversed { p.from + p.frames - 1 - k } else { p.from + k };
                if scripted(p.sym, f) { at.push((2 * k, f)); }
            }
            for k in (0..p.head).filter(|_| mine.is_none()) {
                if scripted(p.sym, 1 + k) { at.push((2 * (p.frames + k), 1 + k)); }
            }
            let tag = format!("{key}::{}::clock", p.fm_name);
            let mut ids: Vec<String> = Vec::new();
            let mut cursor = 0u32;
            for (i, f) in at.into_iter().filter(|(i, _)| *i < total) {
                if i > cursor {
                    let id = crate::uuid_gen::det_uuid(&format!("{tag}::gap{cursor}"));
                    new_keyframes.push(serde_json::json!({ "$id": id, "type": "FRAME_SCRIPT", "length": i - cursor, "code": "", "pluginMetadata": {} }));
                    ids.push(id);
                }
                let id = crate::uuid_gen::det_uuid(&format!("{tag}::{i}"));
                let text = if mine.is_some() {
                    let mut own = native_code(p.sym, f).unwrap_or_default();
                    // an animation's first frame, passed through by a jump into its middle, does not run
                    if i == 0 { own = format!("if (PeptideRuntime.passing != true) {{\n{own}\n}}"); }
                    if i == 1 { format!("// this is frame 1 of the clip in SSF2, moved to the animation's second engine frame for\n// compatibility: this package's code sends an object it has just made to another frame before\n// that object's first frame runs, and SSF2 then never runs it (it runs a new clip's first frame\n// at the end of the frame, after its maker's code), where Fraymakers runs a new object's first\n// keyframe while making it. here it runs only if the object is still on this animation\n{own}") } else { own }
                } else {
                    match (i == 0, extra.get(&p.fm_name)) {
                        (true, Some(x)) => format!("{}\n{x}", code(p.sym, f, true)),
                        _ => code(p.sym, f, i == 0),
                    }
                };
                new_keyframes.push(serde_json::json!({ "$id": id, "type": "FRAME_SCRIPT", "length": 1,
                    "code": text, "pluginMetadata": {} }));
                ids.push(id);
                cursor = i + 1;
            }
            if cursor < total {
                let id = crate::uuid_gen::det_uuid(&format!("{tag}::tail"));
                new_keyframes.push(serde_json::json!({ "$id": id, "type": "FRAME_SCRIPT", "length": total - cursor, "code": "", "pluginMetadata": {} }));
                ids.push(id);
            }
            let layer = crate::uuid_gen::det_uuid(&format!("{tag}::layer"));
            new_layers.push(serde_json::json!({ "$id": layer, "name": "Source frames", "type": "FRAME_SCRIPT",
                "keyframes": ids, "hidden": false, "locked": false, "pluginMetadata": {} }));
            layer_of.insert(p.fm_name.clone(), layer);
            // the source's labels, for a jump its code makes by one, where the animation has none
            // (a projectile's entity is written without them)
            let labelled = anim["layers"].as_array().is_some_and(|ls| ls.iter().any(|l| l.as_str().and_then(|id| json["layers"].as_array()?.iter().find(|x| x["$id"] == id)).is_some_and(|x| x["type"] == "LABEL" && x["name"] != "Source labels")));
            if !labelled && !p.reversed {
                let mut marks: Vec<(u32, String)> = model.sprites.get(&p.sym).map(|s| s.labels.iter()
                    .map(|(f0, l)| (*f0 as u32 + 1, l.clone()))
                    .filter(|(f, _)| *f > p.from && *f < p.from + p.frames)
                    .map(|(f, l)| (2 * (f - p.from), l)).collect()).unwrap_or_default();
                let shadowed = marks.iter().any(|(_, l)| *l == p.fm_name);
                marks.insert(0, (0, if shadowed { String::new() } else { p.fm_name.clone() }));
                marks.retain(|(i, _)| *i < total);
                let ltag = format!("{key}::{}::labels", p.fm_name);
                let mut lids: Vec<String> = Vec::new();
                for (n, (at, name)) in marks.iter().enumerate() {
                    let next = marks.get(n + 1).map(|(i, _)| *i).unwrap_or(total);
                    let id = crate::uuid_gen::det_uuid(&format!("{ltag}::{at}"));
                    new_keyframes.push(serde_json::json!({ "$id": id, "type": "LABEL", "length": (next - at).max(1), "name": name, "pluginMetadata": {} }));
                    lids.push(id);
                }
                let lid = crate::uuid_gen::det_uuid(&format!("{ltag}::layer"));
                new_layers.push(serde_json::json!({ "$id": lid, "name": "Source labels", "type": "LABEL",
                    "keyframes": lids, "hidden": false, "locked": false, "pluginMetadata": {} }));
                label_of.insert(p.fm_name.clone(), lid);
            }
        }
    }
    if layer_of.is_empty() { return Ok(()) }
    // a conversion run again replaces the layer it wrote before
    let old: BTreeSet<String> = json["layers"].as_array().map(|a| a.iter()
        .filter(|l| l["name"] == "Source frames" || l["name"] == "Source labels").filter_map(|l| l["$id"].as_str().map(String::from)).collect()).unwrap_or_default();
    let old_kfs: BTreeSet<String> = json["layers"].as_array().map(|a| a.iter()
        .filter(|l| old.contains(l["$id"].as_str().unwrap_or("")))
        .flat_map(|l| l["keyframes"].as_array().cloned().unwrap_or_default())
        .filter_map(|k| k.as_str().map(String::from)).collect()).unwrap_or_default();
    if let Some(a) = json["layers"].as_array_mut() { a.retain(|l| !old.contains(l["$id"].as_str().unwrap_or(""))); a.extend(new_layers); }
    if let Some(a) = json["keyframes"].as_array_mut() { a.retain(|k| !old_kfs.contains(k["$id"].as_str().unwrap_or(""))); a.extend(new_keyframes); }
    if let Some(a) = json["animations"].as_array_mut() {
        for anim in a.iter_mut() {
            let name = anim["name"].as_str().unwrap_or("").to_string();
            if let Some(ls) = anim["layers"].as_array_mut() {
                ls.retain(|l| !old.contains(l.as_str().unwrap_or("")));
                if let Some(layer) = layer_of.get(&name) { ls.push(serde_json::Value::String(layer.clone())); }
                if let Some(layer) = label_of.get(&name) { ls.push(serde_json::Value::String(layer.clone())); }
            }
        }
    }
    std::fs::write(path, serde_json::to_string_pretty(&json)?)?;
    Ok(())
}

/// A projectile's entity: its keyframes, each frame's code on its own where the engine plays the
/// clip as its own (`native_clip`)
pub(crate) fn native_entity(entity: &std::path::Path, key: &str, model: &SourceModel,
                            pieces: &[Piece], emitted: &Emitted, wait: bool) -> anyhow::Result<()> {
    clock_entity(entity, key, model, pieces, &emitted.frames, &BTreeMap::new(), emitted.native.map(|s| (s, wait)))
}

/// An attack that declares `cancel` can be cut short: while it plays, holding the button it was
/// made with once its clip says it is waiting (a clip that says nothing is) drops its fall and
/// sends the clip to its `finish` label, once (kirby's stone). The check is the attack's own, on
/// its first frame: a timer of the engine's that ends with the attack's state.
fn attack_cancels(data: &crate::extractor::CharacterData, model: &SourceModel, pieces: &[Piece], units: Units) -> BTreeMap<String, String> {
    let jumps = label_jumps(model, pieces);
    let mut out = BTreeMap::new();
    for a in data.attacks.iter().filter(|a| a.stats.get("cancel").is_some_and(|v| *v != 0.0)) {
        let fm = data.ssf2_to_fm_anim.get(&a.ssf2_name).cloned().unwrap_or_else(|| a.name.clone());
        for p in pieces.iter().filter(|p| p.fm_name == fm) {
            let special = p.fm_name.starts_with("special");
            let held = if special { "__c.SPECIAL" } else { "(__c.ATTACK || __c.GRAB)" };
            let finish = model.class_of(p.sym).and_then(|c| jumps.get(&c.name)).and_then(|j| j.labels.get("finish"));
            let jump = match finish {
                Some((anim, true)) => format!("self.playAnimation(\"{anim}\");"),
                Some((anim, false)) => format!("if (self.getAnimation() == \"{anim}\") self.playFrameLabel(\"finish\");"),
                None => "if (__st != null) __st.gotoAndPlay(\"finish\");".to_string(),
            };
            out.insert(p.fm_name.clone(), format!(
"// its declared `cancel`: holding {what} once its clip is waiting cuts it short at `finish`
(function() {{ var armed = true; self.addTimer(1, -1, function(?t) {{
	if (!armed) return;
	var __c = self.getHeldControls();
	if (!({held})) return;
	var __st = PeptideRuntime.shownClip();
	// a clip that declares it says (unset is not waiting); one that does not is waiting
	var __w = __st == null ? true : (__st.__c_waiting != null ? AS3.truthy(__st.__c_waiting.get()) : (__st.waiting == null ? true : AS3.truthy(__st.waiting)));
	if (!__w) return;
	armed = false;
	self.setYSpeed(self.isOnFloor() ? 0 : -4 * {speed} + self.getCharacterStat(\"gravity\") * 1.5);
	{jump}
}}, {{ persistent: false }}); }})();", what = if special { "the special button" } else { "the attack button" }, speed = units.speed));
        }
    }
    out
}

/// A landing's own animation takes what its listener wrote: the attack's stats on its
/// AnimationStats line (the table's rows, with the floor's steering), the boxes' on its
/// HitboxStats entry (made from the move's own entry when it has none).
fn landing_stats(dir: &std::path::Path, landings: &[(crate::compile::Landing, Vec<String>)],
                 jumps: &BTreeMap<String, crate::compile::ClipJumps>) -> anyhow::Result<()> {
    let table = crate::compile::attack_stats::table();
    let mut anim_fields: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut box_fields: BTreeMap<String, BTreeMap<u32, Vec<(String, String)>>> = BTreeMap::new();
    for (l, anims) in landings {
        for a in anims {
            let f = anim_fields.entry(a.clone()).or_default();
            let mut ranks = BTreeMap::new();
            for (k, v) in &l.attack { table.put(f, &mut ranks, k, *v, true); }
            for (i, pairs) in &l.boxes {
                let raw: std::collections::BTreeMap<String, f64> = pairs.iter().cloned().collect();
                let b = box_fields.entry(a.clone()).or_default().entry(*i).or_default();
                for (field, text) in crate::haxe_gen::hitbox_override_fields(&raw) { b.retain(|(g, _)| *g != field); b.push((field, text)); }
            }
        }
    }
    // a line's `{ ... }` with these fields set
    let set = |line: &str, fields: &[(String, String)]| -> String {
        let (Some(o), Some(c)) = (line.find('{'), line.rfind('}')) else { return line.to_string() };
        let mut props: Vec<String> = line[o + 1..c].split(',').map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty() && !fields.iter().any(|(f, _)| p.split(':').next().map(str::trim) == Some(f.as_str()))).collect();
        props.extend(fields.iter().map(|(f, t)| format!("{f}: {t}")));
        format!("{}{{ {} }}{}", &line[..o], props.join(", "), &line[c + 1..])
    };
    if !anim_fields.is_empty() {
        let path = dir.join("AnimationStats.hx");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let out: String = text.lines().map(|l| {
                let name = l.trim_start().split(':').next().unwrap_or("").trim();
                match anim_fields.get(name) { Some(f) if !f.is_empty() => set(l, f), _ => l.to_string() }
            }).collect::<Vec<_>>().join("\n");
            std::fs::write(&path, out + "\n")?;
        }
    }
    if !box_fields.is_empty() {
        let path = dir.join("HitboxStats.hx");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let mut lines: Vec<String> = text.lines().map(String::from).collect();
            let block = |lines: &[String], anim: &str| -> Option<(usize, usize)> {
                let start = lines.iter().position(|l| l.trim_start().starts_with(&format!("{anim}: {{")) && l.starts_with('\t') && !l.starts_with("\t\t"))?;
                let end = (start + 1..lines.len()).find(|&i| lines[i].trim_start().starts_with('}'))?;
                Some((start, end))
            };
            for (anim, boxes) in &box_fields {
                if block(&lines, anim).is_none() {
                    // made from the entry of the move whose clip it is part of
                    let base = jumps.values().find(|j| j.anims.contains(anim))
                        .and_then(|j| j.anims.iter().find(|a| *a != anim && block(&lines, a).is_some()).cloned());
                    let Some((s, e)) = base.as_deref().and_then(|b| block(&lines, b)) else { continue };
                    let mut copy: Vec<String> = lines[s..=e].to_vec();
                    copy[0] = copy[0].replacen(base.as_deref().unwrap_or(""), anim, 1);
                    if let Some(last) = copy.last_mut() { if !last.trim_end().ends_with(',') { last.push(','); } }
                    lines.splice(s..s, copy);
                }
                let Some((s, e)) = block(&lines, anim) else { continue };
                for line in lines.iter_mut().take(e).skip(s + 1) {
                    let t = line.trim_start().to_string();
                    for (n, fields) in boxes {
                        // the source numbers its boxes from 1, this engine from 0
                        if t.starts_with(&format!("hitbox{}:", n.saturating_sub(1))) { *line = set(line, fields); }
                    }
                }
            }
            std::fs::write(&path, lines.join("\n") + "\n")?;
        }
    }
    Ok(())
}

/// The animations an entity file has.
pub(crate) fn entity_animations(path: &std::path::Path) -> BTreeSet<String> {
    std::fs::read(path).ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("animations").and_then(|a| a.as_array()).map(|a| a.iter()
            .filter_map(|x| x.get("name").and_then(|n| n.as_str()).map(String::from)).collect()))
        .unwrap_or_default()
}

/// The runtime's path for the projectiles and items a fighter declares: each one the current
/// pipeline wrote an entity for gets its script written the runtime's way, from its own
/// declaration (the class that runs it, the clip that is its art, its stats).
#[allow(clippy::too_many_arguments)]
fn runtime_projectiles(char_dir: &std::path::Path, model: &SourceModel, runtime: &Runtime,
                       fighter: &crate::model::declare::Fighter, units: Units, others: &[String],
                       effects: &BTreeMap<String, Vec<Piece>>, entities: &BTreeMap<String, String>,
                       pieces_of: &mut BTreeMap<String, Vec<Piece>>,
                       unplaced: &mut Vec<String>) -> anyhow::Result<Vec<(String, Emitted)>> {
    let mut out = Vec::new();
    let decls: Vec<&(String, Value)> = fighter.projectiles.entries().iter().chain(fighter.items.entries()).collect();
    let wait = sends_new_objects_on(model, runtime);
    // one script per entity, from the first declaration written as it (a projectile fired by
    // another declaration sharing its clip takes that one's stats as it starts)
    for (entity_key, pieces) in std::mem::take(pieces_of) {
        // written as it by name, else named as its clip (a declaration fired under one name that
        // draws with a clip of another has an entity for each)
        let by_clip = |d: &Value| d.get("linkage_id").and_then(|v| v.str()) == Some(entity_key.as_str());
        let found = decls.iter().find(|(n, _)| entities.get(n) == Some(&entity_key))
            .or_else(|| decls.iter().find(|(_, d)| by_clip(d)));
        let Some((name, decl)) = found.map(|e| (&e.0, &e.1)) else {
            // not declared, but the package explains it: a class of its own (an enemy it spawns
            // by class) runs on the clip the entity draws; anything else keeps its script
            let script = char_dir.join(format!("library/scripts/Projectile/{}Script.hx", crate::haxe_gen::snake_to_pascal(&entity_key)));
            // the class whose own stats name this clip as its art (an enemy: `ownStats`'s
            // `linkage_id`) first, with those stats; else a class written as the clip
            let own_stats = |c: &crate::model::ClassModel| c.methods.iter().find(|m| m.name == "ownStats" && !m.is_static)
                .and_then(|m| crate::model::declare::returned(&m.function.body));
            let enemy = model.classes.values().find(|c| own_stats(c).is_some_and(|v|
                v.get("linkage_id").and_then(|l| l.str()).is_some_and(|l| l.eq_ignore_ascii_case(&entity_key))));
            let stats = enemy.and_then(own_stats).unwrap_or(Value::Null);
            let class = enemy.or_else(|| model.classes.get(&entity_key)).or_else(|| model.classes.values().find(|c| compile::short(&c.name) == entity_key));
            if let (Some(c), Some(sym), true) = (class, pieces.first().map(|p| p.sym), script.exists()) {
                let content = crate::projectile_gen::projectile_content_id(&entity_key);
                let entity = char_dir.join(format!("library/entities/{}.entity", crate::haxe_gen::sanitize_entity_name(&content)));
                let has = entity_animations(&entity);
                let pieces: Vec<Piece> = pieces.into_iter().filter(|p| has.contains(&p.fm_name)).collect();
                let spec = ObjectSpec { kind: "projectile", others, effects, entities, class: c.name.clone(),
                    clip: entity_key.clone(), clip_sym: Some(sym), stats, autolink: BTreeMap::new(), data: String::new() };
                let emitted = object_script(model, runtime, &spec, &Decls::of(fighter), &pieces, units, &entity_key);
                std::fs::write(&script, &emitted.script)?;
                native_entity(&entity, &entity_key, model, &pieces, &emitted, wait)?;
                unplaced.retain(|u| !u.starts_with(&format!("{entity_key}: made from another package")));
                out.push((entity_key, emitted));
                continue;
            }
            pieces_of.insert(entity_key, pieces);
            continue;
        };
        let content = crate::projectile_gen::projectile_content_id(&entity_key);
        let entity = char_dir.join(format!("library/entities/{}.entity", crate::haxe_gen::sanitize_entity_name(&content)));
        let has = entity_animations(&entity);
        let (pieces, missing): (Vec<Piece>, Vec<Piece>) = pieces.into_iter().partition(|p| has.contains(&p.fm_name));
        unplaced.extend(missing.iter().map(|p| format!("{entity_key}: {} has no such animation", p.fm_name)));
        let spec = ObjectSpec {
            kind: "projectile",
            others,
            effects,
            entities,
            class: decl.get("classAPI").and_then(|v| v.class()).unwrap_or("PeptideProjectile").to_string(),
            clip: decl.get("linkage_id").and_then(|v| v.str()).unwrap_or(name).to_string(),
            clip_sym: None,
            stats: decl.clone(),
            autolink: BTreeMap::new(),
            data: String::new(),
        };
        let emitted = object_script(model, runtime, &spec, &Decls::of(fighter), &pieces, units, name);
        let script = char_dir.join(format!("library/scripts/Projectile/{}Script.hx", crate::haxe_gen::snake_to_pascal(&entity_key)));
        if script.exists() {
            std::fs::write(&script, &emitted.script)?;
            native_entity(&entity, &entity_key, model, &pieces, &emitted, wait)?;
        }
        else { unplaced.push(format!("{entity_key}: no projectile script at {}", script.display())); }
        out.push((entity_key, emitted));
    }
    Ok(out)
}

/// The x speeds the engine's walk animations have their clips set, in this engine's units: a
/// fighter whose walk its own frame scripts drive declares its speed there, not in its stats.
fn walk_script_speeds(model: &SourceModel, fighter: &crate::model::declare::Fighter, pieces: &[Piece], speed: f64) -> BTreeMap<String, f64> {
    use crate::model::ast::Expr;
    let anims = fighter.clip().map(|c| model.animations(c)).unwrap_or_default();
    let mut out = BTreeMap::new();
    for p in pieces.iter().filter(|p| p.fm_name.starts_with("walk")) {
        let Some(class) = anims.iter().find(|a| a.label == p.label).and_then(|a| a.stance_class.as_ref()) else { continue };
        let Some(c) = model.classes.get(class) else { continue };
        // the scripts on the frames this animation shows (frame_scripts count from 0)
        let shown = |f: u32| f + 1 >= p.from && f + 1 < p.from + p.frames;
        for fs in c.frame_scripts.iter().filter(|fs| shown(fs.frame)) {
            let Some(m) = c.methods.iter().find(|m| m.name == fs.method) else { continue };
            crate::model::ast::walk_exprs(&m.function.body, &mut |e| {
                let Expr::Call(callee, args) = e else { return };
                if !matches!(&**callee, Expr::Member(_, n) if n == "setXSpeed") { return }
                let v = match args.first() { Some(Expr::Num(n)) => *n, Some(Expr::Int(i)) => *i as f64, _ => return };
                // the first speed it sets is the walk it walks at
                if v > 0.0 { out.entry(p.fm_name.clone()).or_insert((v * speed * 10000.0).round() / 10000.0); }
            });
        }
    }
    out
}

/// What a rollback puts back of an object's own fields: every field the runtime's kept files
/// write on an object (`o.__name = ...`), and on its engine object's exports, as a record a frame
/// (`runtime/core/rollback.hx`). The ones written once as an object is made and never again are
/// left out; they cannot differ between two frames.
fn rollback_fields(runtime_text: &str) -> String {
    const FIXED: &[&str] = &["__is", "__type", "__fm", "__mc", "__tl", "__sym", "__scripts", "__object", "__vfx",
        "__pieces", "__global", "__locked", "__born", "__name", "__linkageId", "__declared", "__source", "__errors",
        "__gaps", "__verifyPulse", "__bound", "__cellHolds", "__kids", "__enteredOn", "__held", "__half",
        "__part", "__playing", "__loops", "__was", "__anim"];
    let assign = crate::re!(r"\.(__[A-Za-z0-9_]+)\s*(?:=[^=]|\+\+|--|\+=|-=)");
    let export = crate::re!(r"exports\.(__[A-Za-z0-9_]+)\s*(?:=[^=]|\+\+|--|\+=|-=)");
    let code = shake::code_text(runtime_text);
    let ex: std::collections::BTreeSet<String> = export.captures_iter(&code).map(|c| c[1].to_string())
        .filter(|n| !FIXED.contains(&n.as_str())).collect();
    let names: Vec<String> = assign.captures_iter(&code).map(|c| c[1].to_string())
        .filter(|n| !FIXED.contains(&n.as_str()) && !n.starts_with("__c_") && !ex.contains(n))
        .collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let ex: Vec<String> = ex.into_iter().collect();
    // a field ever given a list or a map is copied with the record (it is changed in place); any
    // other is read as it is, which is most of them and costs nothing
    let collection = crate::re!(r"\.(__[A-Za-z0-9_]+)\s*=\s*(?:\[|new haxe\.ds\.|[A-Za-z_.]*\.copy\(\))");
    let lists: std::collections::BTreeSet<String> = collection.captures_iter(&code).map(|c| c[1].to_string()).collect();
    let read = |n: &String, v: &str| if lists.contains(n) { format!("Rollback.copy({v})") } else { v.to_string() };
    let save = |list: &[String]| list.iter().map(|n| read(n, &format!("o.{n}"))).collect::<Vec<_>>().join(", ");
    let load = |list: &[String]| list.iter().enumerate().map(|(i, n)| format!("o.{n} = {};", read(n, &format!("s[{i}]")))).collect::<Vec<_>>().join(" ");
    format!("// ---- rollback: the fields the runtime above writes on an object (generated from it)\n\
        Rollback.saveObject = function(o) {{ return [{}]; }};\n\
        Rollback.loadObject = function(o, s) {{ {} }};\n\
        Rollback.saveExports = function(o) {{ return o == null ? null : [{}]; }};\n\
        Rollback.loadExports = function(o, s) {{ if (o == null || s == null) return; {} }};\n",
        save(&names), load(&names), save(&ex), load(&ex))
}

#[cfg(test)]
mod landing_stats_tests {
    use super::*;

    /// a landing listener's writes land on its landing animation's own stats: the attack's on its
    /// AnimationStats line, a box's on a HitboxStats entry made from the move's own
    #[test]
    fn a_landing_takes_its_listeners_stats() {
        let dir = std::env::temp_dir().join(format!("peptide-landing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("AnimationStats.hx"), "{\n\taerial_up: {landType:LandType.NONE},\n\taerial_up_land: {landType:LandType.NONE},\n}\n").unwrap();
        std::fs::write(dir.join("HitboxStats.hx"), "{\n\taerial_up: {\n\t\thitbox0: { damage: 5, angle: 80, limb: AttackLimb.FOOT }\n\t},\n}\n").unwrap();
        let landing = crate::compile::Landing {
            label: "continue".into(), anim: Some("aerial_up_land".into()),
            attack: vec![("allowControl".into(), 0.0), ("cancelWhenAirborne".into(), 1.0)],
            boxes: vec![(1, vec![("damage".into(), 9.0)])],
        };
        let mut jumps = BTreeMap::new();
        jumps.insert("Clip".to_string(), crate::compile::ClipJumps { anims: vec!["aerial_up".into(), "aerial_up_land".into()], labels: BTreeMap::new(), autolink: None , playhead: None});
        landing_stats(&dir, &[(landing, vec!["aerial_up_land".into()])], &jumps).unwrap();
        let anim = std::fs::read_to_string(dir.join("AnimationStats.hx")).unwrap();
        let hit = std::fs::read_to_string(dir.join("HitboxStats.hx")).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert!(anim.contains("aerial_up_land: { landType:LandType.NONE, allowMovement: false, leaveGroundCancel: true }"), "{anim}");
        assert!(anim.contains("aerial_up: {landType:LandType.NONE}"), "{anim}");
        assert!(hit.contains("\taerial_up_land: {"), "{hit}");
        assert!(hit.contains("hitbox0: { angle: 80, limb: AttackLimb.FOOT, damage: 9 }"), "{hit}");
        assert!(hit.contains("hitbox0: { damage: 5, angle: 80, limb: AttackLimb.FOOT }"), "the move's own entry stays: {hit}");
    }
}

/// An animation whose clip's code runs the source's autolink: where it is on at the animation's
/// start, and each engine frame its code turns it on (toward a point), moves the point, or turns
/// it off.
#[derive(Clone, Debug)]
pub struct AnimLink {
    pub anim: String,
    pub class: String,
    /// (engine frame, the point while on), in frame order
    pub marks: Vec<(u32, Option<(f64, f64)>)>,
    pub dropped: Vec<String>,
}

/// each animation whose clip's code runs the autolink, from the clip's own frames (a move made of
/// several animations has it in the one whose frames start it)
fn autolink_plan(model: &SourceModel, pieces: &[Piece]) -> Vec<AnimLink> {
    use crate::compile::autolink::Step;
    let mut out = Vec::new();
    for p in pieces.iter().filter(|p| !p.reversed) {
        let Some(c) = model.class_of(p.sym) else { continue };
        let Some(al) = compile::autolink::of_class(c) else { continue };
        let inside = |f: u32| f >= p.from && f < p.from + p.frames;
        let turn = |on: Option<(f64, f64)>, step: &Step| match step {
            Step::Start(x, y) => Some((*x, *y)),
            Step::Point(x, y) => on.map(|_| (*x, *y)),
            Step::Stop => None,
        };
        // a move the conversion cut into several animations (a charge, then its attack) has the
        // autolink its clip's earlier frames left on
        let mut on: Option<(f64, f64)> = al.steps.iter().filter(|(f, _)| *f < p.from).fold(None, |on, (_, s)| turn(on, s));
        if on.is_none() && !al.steps.iter().any(|(f, s)| inside(*f) && matches!(s, Step::Start(..))) { continue }
        let mut marks = vec![(0, on)];
        for (f, step) in al.steps.iter().filter(|(f, _)| inside(*f)) {
            on = turn(on, step);
            let at = 2 * (f - p.from);
            marks.retain(|(i, _)| *i != at);
            marks.push((at, on));
        }
        out.push(AnimLink { anim: p.fm_name.clone(), class: c.name.clone(), marks, dropped: al.dropped.clone() });
    }
    out
}

/// the hitboxes (by number) each animation of an entity has
fn entity_hitboxes(json: &serde_json::Value) -> BTreeMap<String, Vec<u32>> {
    let names: BTreeMap<&str, &str> = json["layers"].as_array().map(|a| a.iter()
        .filter_map(|l| Some((l["$id"].as_str()?, l["name"].as_str()?))).collect()).unwrap_or_default();
    json["animations"].as_array().map(|a| a.iter().filter_map(|anim| {
        let mut boxes: Vec<u32> = anim["layers"].as_array()?.iter()
            .filter_map(|l| names.get(l.as_str()?)?.strip_prefix("hitbox")?.parse().ok()).collect();
        boxes.sort();
        boxes.dedup();
        Some((anim["name"].as_str()?.to_string(), boxes))
    }).collect()).unwrap_or_default()
}

/// what each hitbox states of the given fields in the stats file, per animation and box
fn stated_fields(text: &str, fields: &[String]) -> BTreeMap<(String, u32), Vec<(String, String)>> {
    let mut out = BTreeMap::new();
    let mut anim: Option<String> = None;
    for line in text.lines() {
        if line.starts_with('\t') && !line.starts_with("\t\t") {
            anim = line.trim().strip_suffix('{').map(|h| h.trim().trim_end_matches(':').trim().to_string());
            continue;
        }
        let (Some(a), t) = (&anim, line.trim_start()) else { continue };
        let Some(n) = t.strip_prefix("hitbox").and_then(|r| r.split(':').next()).and_then(|n| n.parse::<u32>().ok()) else { continue };
        let (Some(o), Some(c)) = (t.find('{'), t.rfind('}')) else { continue };
        let got: Vec<(String, String)> = t[o + 1..c].split(',').filter_map(|p| {
            let (k, v) = p.split_once(':')?;
            fields.iter().any(|f| f == k.trim()).then(|| (k.trim().to_string(), v.trim().to_string()))
        }).collect();
        out.insert((a.clone(), n), got);
    }
    out
}

/// the code the clip's own calls become, by class: turning the autolink on sets every hitbox of
/// the animation playing to what an autolinked box states (`runtime/autolink.jsonc`), turning it
/// off puts back what each box states of those fields; moving the point is the animation's autolink point, and so nothing (`{fm}`: the
/// fighter's engine handle)
fn autolink_calls(links: &[AnimLink], boxes: &BTreeMap<String, Vec<u32>>, stated: &BTreeMap<(String, u32), Vec<(String, String)>>) -> BTreeMap<String, (String, String)> {
    let fields = &compile::autolink::table().box_fields;
    let set: String = fields.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", ");
    let mut by_class: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for l in links {
        let Some(bs) = boxes.get(&l.anim).filter(|b| !b.is_empty()) else { continue };
        let on: Vec<String> = bs.iter().map(|b| format!("{{fm}}.updateHitboxStats({b}, {{ {set} }});")).collect();
        let off: Vec<String> = bs.iter().filter_map(|b| stated.get(&(l.anim.clone(), *b)).filter(|f| !f.is_empty()).map(|f|
            format!("{{fm}}.updateHitboxStats({b}, {{ {} }});", f.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", ")))).collect();
        let e = by_class.entry(l.class.clone()).or_default();
        e.0.push(format!("case {}: {{ {} }}", compile::quote(&l.anim), on.join(" ")));
        e.1.push(format!("case {}: {{ {} }}", compile::quote(&l.anim), off.join(" ")));
    }
    by_class.into_iter().map(|(c, (on, off))| (c, (
        format!("switch ({{fm}}.getAnimation()) {{ {} default: null; }}", on.join(" ")),
        format!("switch ({{fm}}.getAnimation()) {{ {} default: null; }}", off.join(" ")),
    ))).collect()
}

/// the autolink as the animations' own: each hitbox of an animation whose code has it on from its
/// first frame states what an autolinked box does, and each such animation carries the point as an
/// autolink point for every hitbox, from each frame its code names one (none while it is off)
fn autolink_entity(entity: &std::path::Path, stats: &std::path::Path, key: &str, links: &[AnimLink], scale: f64) -> anyhow::Result<()> {
    if links.is_empty() { return Ok(()) }
    let Ok(bytes) = std::fs::read(entity) else { return Ok(()) };
    let mut json: serde_json::Value = serde_json::from_slice(&bytes)?;
    let boxes = entity_hitboxes(&json);
    let t = compile::autolink::table();
    let k = if t.point_scaled { scale } else { 1.0 };
    let kfs: BTreeMap<String, u64> = json["keyframes"].as_array().map(|a| a.iter().filter_map(|x|
        Some((x["$id"].as_str()?.to_string(), x["length"].as_u64().unwrap_or(1)))).collect()).unwrap_or_default();
    let lens: BTreeMap<String, u64> = json["layers"].as_array().map(|a| a.iter().filter_map(|l| {
        let n: u64 = l["keyframes"].as_array()?.iter().filter_map(|x| kfs.get(x.as_str()?)).sum();
        Some((l["$id"].as_str()?.to_string(), n))
    }).collect()).unwrap_or_default();
    let (mut new_kfs, mut new_syms, mut new_layers) = (Vec::new(), Vec::new(), Vec::new());
    let mut layers_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut from_start: BTreeSet<String> = BTreeSet::new();
    for l in links {
        let Some(bs) = boxes.get(&l.anim).filter(|b| !b.is_empty()) else { continue };
        let Some(anim) = json["animations"].as_array().and_then(|a| a.iter().find(|x| x["name"] == l.anim.as_str())) else { continue };
        let total = anim["layers"].as_array().map(|ls| ls.iter().filter_map(|x| lens.get(x.as_str()?)).copied().max().unwrap_or(0)).unwrap_or(0) as u32;
        if total == 0 { continue }
        if l.marks.first().is_some_and(|(i, on)| *i == 0 && on.is_some()) { from_start.insert(l.anim.clone()); }
        let mut runs: Vec<(u32, Option<(f64, f64)>)> = vec![(0, None)];
        for (i, on) in l.marks.iter().filter(|(i, _)| *i < total) {
            if runs.last().is_some_and(|(j, _)| j == i) { runs.pop(); }
            runs.push((*i, *on));
        }
        for b in bs {
            let tag = format!("{key}::{}::autolink{b}", l.anim);
            let mut ids = Vec::new();
            for (n, (at, on)) in runs.iter().enumerate() {
                let next = runs.get(n + 1).map(|(i, _)| *i).unwrap_or(total);
                if next <= *at { continue }
                let id = crate::uuid_gen::det_uuid(&format!("{tag}::{at}"));
                let sym = on.filter(|(x, _)| !x.is_nan()).map(|(x, y)| {
                    let s = crate::uuid_gen::det_uuid(&format!("{tag}::{at}::point"));
                    new_syms.push(serde_json::json!({ "$id": s, "type": "POINT", "x": (x * k * 100.0).round() / 100.0, "y": (y * k * 100.0).round() / 100.0,
                        "rotation": 0, "alpha": 1, "color": "0xff0000", "pluginMetadata": {} }));
                    s
                });
                new_kfs.push(serde_json::json!({ "$id": id, "type": "POINT", "length": next - at, "symbol": sym,
                    "tweened": false, "tweenType": "LINEAR", "pluginMetadata": {} }));
                ids.push(id);
            }
            let layer = crate::uuid_gen::det_uuid(&format!("{tag}::layer"));
            new_layers.push(serde_json::json!({ "$id": layer, "name": format!("autolinkpoint{b}"), "type": "POINT",
                "keyframes": ids, "hidden": false, "locked": false, "pluginMetadata": {} }));
            layers_of.entry(l.anim.clone()).or_default().push(layer);
        }
    }
    if layers_of.is_empty() { return Ok(()) }
    // a conversion run again replaces the layers it wrote before
    let mine: BTreeSet<String> = new_layers.iter().filter_map(|l| l["$id"].as_str().map(String::from)).collect();
    if let Some(a) = json["layers"].as_array_mut() { a.retain(|l| !mine.contains(l["$id"].as_str().unwrap_or(""))); a.extend(new_layers); }
    let kf_ids: BTreeSet<String> = new_kfs.iter().filter_map(|x| x["$id"].as_str().map(String::from)).collect();
    if let Some(a) = json["keyframes"].as_array_mut() { a.retain(|x| !kf_ids.contains(x["$id"].as_str().unwrap_or(""))); a.extend(new_kfs); }
    let sym_ids: BTreeSet<String> = new_syms.iter().filter_map(|x| x["$id"].as_str().map(String::from)).collect();
    if let Some(a) = json["symbols"].as_array_mut() { a.retain(|x| !sym_ids.contains(x["$id"].as_str().unwrap_or(""))); a.extend(new_syms); }
    if let Some(a) = json["animations"].as_array_mut() {
        for anim in a.iter_mut() {
            let Some(ls) = layers_of.get(anim["name"].as_str().unwrap_or("")) else { continue };
            if let Some(arr) = anim["layers"].as_array_mut() {
                arr.retain(|x| !mine.contains(x.as_str().unwrap_or("")));
                arr.extend(ls.iter().map(|x| serde_json::Value::String(x.clone())));
            }
        }
    }
    std::fs::write(entity, serde_json::to_string_pretty(&json)?)?;
    // the hitboxes of an animation whose code has the autolink on from its start state its angle
    if let Ok(text) = std::fs::read_to_string(stats) {
        let mut anim: Option<String> = None;
        let out: Vec<String> = text.lines().map(|line| {
            if line.starts_with('\t') && !line.starts_with("\t\t") {
                anim = line.trim().strip_suffix('{').map(|h| h.trim().trim_end_matches(':').trim().to_string());
                return line.to_string();
            }
            let Some(a) = anim.as_ref().filter(|a| from_start.contains(*a)) else { return line.to_string() };
            let tt = line.trim_start();
            let Some(n) = tt.strip_prefix("hitbox").and_then(|r| r.split(':').next()).and_then(|n| n.parse::<u32>().ok()) else { return line.to_string() };
            if !boxes.get(a).is_some_and(|b| b.contains(&n)) { return line.to_string() }
            let (Some(o), Some(c)) = (line.find('{'), line.rfind('}')) else { return line.to_string() };
            let mut props: Vec<String> = line[o + 1..c].split(',').map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty() && !t.box_fields.iter().any(|(k, _)| p.split(':').next().map(str::trim) == Some(k.as_str()))).collect();
            props.extend(t.box_fields.iter().map(|(k, v)| format!("{k}: {v}")));
            format!("{}{{ {} }}{}", &line[..o], props.join(", "), &line[c + 1..])
        }).collect();
        std::fs::write(stats, out.join("\n") + "\n")?;
    }
    Ok(())
}

/// the stats and points the source's engine gives the animations of its own states
/// (runtime/engine_animations.jsonc): each stat on its animation's line in the stats file, and
/// each point as a layer the whole length of every animation named for it
fn engine_animations(entity: &std::path::Path, stats: &std::path::Path, key: &str) -> anyhow::Result<()> {
    let t = compile::engine_anims::table();
    if let Ok(text) = std::fs::read_to_string(stats) {
        let out: Vec<String> = text.lines().map(|line| {
            let name = line.trim_start().split(':').next().unwrap_or("").trim();
            let Some(add) = t.stats.get(name) else { return line.to_string() };
            if !line.starts_with('\t') || line.starts_with("\t\t") { return line.to_string() }
            let (Some(o), Some(c)) = (line.find('{'), line.rfind('}')) else { return line.to_string() };
            let fields: Vec<&str> = add.split(',').map(str::trim).filter(|f| !f.is_empty()).collect();
            let names: Vec<&str> = fields.iter().filter_map(|f| f.split(':').next().map(str::trim)).collect();
            let mut props: Vec<String> = line[o + 1..c].split(',').map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty() && !names.contains(&p.split(':').next().map(str::trim).unwrap_or(""))).collect();
            props.extend(fields.iter().map(|f| f.to_string()));
            format!("{}{{{}}}{}", &line[..o], props.join(", "), &line[c + 1..])
        }).collect();
        std::fs::write(stats, out.join("\n") + "\n")?;
    }
    if t.points.is_empty() { return Ok(()) }
    let Ok(bytes) = std::fs::read(entity) else { return Ok(()) };
    let mut json: serde_json::Value = serde_json::from_slice(&bytes)?;
    let kfs: BTreeMap<String, u64> = json["keyframes"].as_array().map(|a| a.iter().filter_map(|x|
        Some((x["$id"].as_str()?.to_string(), x["length"].as_u64().unwrap_or(1)))).collect()).unwrap_or_default();
    let lens: BTreeMap<String, u64> = json["layers"].as_array().map(|a| a.iter().filter_map(|l| {
        let n: u64 = l["keyframes"].as_array()?.iter().filter_map(|x| kfs.get(x.as_str()?)).sum();
        Some((l["$id"].as_str()?.to_string(), n))
    }).collect()).unwrap_or_default();
    let names: BTreeMap<String, String> = json["layers"].as_array().map(|a| a.iter()
        .filter_map(|l| Some((l["$id"].as_str()?.to_string(), l["name"].as_str()?.to_string()))).collect()).unwrap_or_default();
    let (mut new_kfs, mut new_syms, mut new_layers) = (Vec::new(), Vec::new(), Vec::new());
    let mut add: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (layer_name, (x, y, anims)) in &t.points {
        for anim_name in anims {
            let Some(anim) = json["animations"].as_array().and_then(|a| a.iter().find(|a| a["name"] == anim_name.as_str())) else { continue };
            let layers: Vec<String> = anim["layers"].as_array().map(|ls| ls.iter().filter_map(|l| l.as_str().map(String::from)).collect()).unwrap_or_default();
            // one the entity already carries is its own
            if layers.iter().any(|l| names.get(l) == Some(layer_name)) { continue }
            let total = layers.iter().filter_map(|l| lens.get(l)).copied().max().unwrap_or(0);
            if total == 0 { continue }
            let tag = format!("{key}::{anim_name}::{layer_name}");
            let sym = crate::uuid_gen::det_uuid(&format!("{tag}::point"));
            let kf = crate::uuid_gen::det_uuid(&format!("{tag}::kf"));
            let layer = crate::uuid_gen::det_uuid(&format!("{tag}::layer"));
            new_syms.push(serde_json::json!({ "$id": sym, "type": "POINT", "x": x, "y": y, "rotation": 0, "alpha": 1, "color": "0x00ff00", "pluginMetadata": {} }));
            new_kfs.push(serde_json::json!({ "$id": kf, "type": "POINT", "length": total, "symbol": sym, "tweened": false, "tweenType": "LINEAR", "pluginMetadata": {} }));
            new_layers.push(serde_json::json!({ "$id": layer, "name": layer_name, "type": "POINT", "keyframes": [kf], "hidden": false, "locked": false, "pluginMetadata": {} }));
            add.entry(anim_name.clone()).or_default().push(layer);
        }
    }
    if add.is_empty() { return Ok(()) }
    if let Some(a) = json["layers"].as_array_mut() { a.extend(new_layers); }
    if let Some(a) = json["keyframes"].as_array_mut() { a.extend(new_kfs); }
    if let Some(a) = json["symbols"].as_array_mut() { a.extend(new_syms); }
    if let Some(a) = json["animations"].as_array_mut() {
        for anim in a.iter_mut() {
            let Some(ls) = add.get(anim["name"].as_str().unwrap_or("")) else { continue };
            if let Some(arr) = anim["layers"].as_array_mut() { arr.extend(ls.iter().map(|x| serde_json::Value::String(x.clone()))); }
        }
    }
    std::fs::write(entity, serde_json::to_string_pretty(&json)?)?;
    Ok(())
}

/// the box stats each animation's first frame writes as plain values (a combo's later hits,
/// which the source keeps in one clip and restates as each begins: mario's jab), as the
/// animation's own HitboxStats. a write handed a field is read through the plain object the same
/// frame put in it first
fn entry_box_writes(model: &SourceModel, pieces: &[Piece]) -> Vec<(crate::compile::Landing, Vec<String>)> {
    use crate::model::ast::{Expr, Place, Stmt};
    let num = |e: &Expr| -> Option<f64> {
        match e {
            Expr::Num(v) => Some(*v), Expr::Int(v) => Some(*v as f64), Expr::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Expr::Unary(crate::model::ast::UnOp::Neg, x) => match &**x { Expr::Num(v) => Some(-v), Expr::Int(v) => Some(-(*v as f64)), _ => None },
            _ => None,
        }
    };
    let pairs = |o: &[(Expr, Expr)]| -> Vec<(String, f64)> {
        o.iter().filter_map(|(k, v)| match k { Expr::Str(k) => num(v).map(|n| (k.clone(), n)), _ => None }).collect()
    };
    let own = |e: &Expr| -> Option<String> {
        match e { Expr::Member(o, n) if matches!(**o, Expr::This) => Some(n.clone()), Expr::Name(n) => Some(n.clone()), _ => None }
    };
    let mut out = Vec::new();
    for p in pieces.iter().filter(|p| !p.reversed) {
        let Some(c) = model.class_of(p.sym) else { continue };
        let Some(fs) = c.frame_scripts.iter().find(|fs| fs.frame + 1 == p.from) else { continue };
        let Some(m) = c.methods.iter().find(|m| m.name == fs.method) else { continue };
        let mut fields: BTreeMap<String, Vec<(String, f64)>> = BTreeMap::new();
        let mut boxes: Vec<(u32, Vec<(String, f64)>)> = Vec::new();
        for st in &m.function.body {
            match st {
                Stmt::Assign(Place::Member(Expr::This, f), Expr::Object(o)) | Stmt::Assign(Place::Name(f), Expr::Object(o)) => { fields.insert(f.clone(), pairs(o)); }
                Stmt::Expr(Expr::Call(callee, args)) => {
                    let Expr::Member(_, name) = &**callee else { continue };
                    if name != "updateAttackBoxStats" && name != "updateHitboxStats" { continue }
                    let [i, v] = args.as_slice() else { continue };
                    let Some(i) = num(i) else { continue };
                    let got = match v { Expr::Object(o) => pairs(o), _ => own(v).and_then(|f| fields.get(&f).cloned()).unwrap_or_default() };
                    if !got.is_empty() { boxes.push((i as u32, got)); }
                }
                _ => {}
            }
        }
        if boxes.is_empty() { continue }
        out.push((crate::compile::Landing { label: String::new(), anim: Some(p.fm_name.clone()), attack: vec![], boxes }, vec![p.fm_name.clone()]));
    }
    out
}

/// the calls through which code reaches a clip's children by place rather than by name (the
/// others that take a child need the child in hand already)
/// the clip an object's frames are the engine's own on: one clip every animation shows forwards
/// from its own frames (none reversed or led into; an engine state's animation that borrows one of
/// them runs that frame as it is entered), each of whose frame scripts is on a frame an
/// animation shows, and that places no children here (nothing of it to keep up as it plays). its
/// frames' code is on the entity's keyframes, the engine plays it, and the runtime keeps no
/// playhead of its own for it
pub fn native_clip(model: &SourceModel, pieces: &[Piece], placing: &BTreeSet<u16>) -> Option<u16> {
    let sym = pieces.first()?.sym;
    if pieces.iter().any(|p| p.sym != sym || p.reversed || p.head > 0) { return None }
    if placing.contains(&sym) { return None }
    let shown = |f: u32| pieces.iter().any(|p| f >= p.from && f < p.from + p.frames);
    if let Some(c) = model.class_of(sym) {
        if c.frame_scripts.iter().any(|fs| !shown(fs.frame + 1)) { return None }
    }
    Some(sym)
}

/// whether any of the package's code sends an object it has just made to a frame before its first
/// frame runs (`p = fireProjectile(..); p.gotoAndPlay("x")`, or the jump straight on the call that
/// made it): the calls are `runtime/new_objects.jsonc`'s
pub fn sends_new_objects_on(model: &SourceModel, runtime: &Runtime) -> bool {
    use crate::model::ast::{Place, Stmt};
    fn name_of(e: &Expr) -> Option<String> {
        match e {
            Expr::Local(r) => Some(format!("#{r}")),
            Expr::Name(n) => Some(n.clone()),
            Expr::Member(o, n) if matches!(**o, Expr::This) => Some(n.clone()),
            Expr::Coerce(_, x) | Expr::Convert(_, x) => name_of(x),
            _ => None,
        }
    }
    fn place_name(p: &Place) -> Option<String> {
        match p {
            Place::Local(r) => Some(format!("#{r}")),
            Place::Name(n) => Some(n.clone()),
            Place::Member(Expr::This, n) => Some(n.clone()),
            _ => None,
        }
    }
    let makes = |e: &Expr| -> bool {
        let mut hit = false;
        e.walk(&mut |x| if let Expr::Call(c, _) = x {
            if let Expr::Member(_, m) | Expr::Name(m) = &**c { if runtime.makes.contains(m) { hit = true } }
        });
        hit
    };
    fn walk(body: &[Stmt], made: &mut BTreeSet<String>, makes: &dyn Fn(&Expr) -> bool, jumps: &BTreeSet<String>) -> bool {
        for s in body {
            if let Stmt::Assign(p, v) = s {
                if let Some(n) = place_name(p) { if makes(v) { made.insert(n); } else { made.remove(&n); } }
            }
            let mut hit = false;
            for e in s.exprs() {
                e.walk(&mut |x| if let Expr::Call(c, _) = x {
                    if let Expr::Member(recv, j) = &**c {
                        if jumps.contains(j) && (makes(recv) || name_of(recv).is_some_and(|n| made.contains(&n))) { hit = true }
                    }
                });
            }
            if hit { return true }
            for b in s.blocks() { if walk(b, made, makes, jumps) { return true } }
        }
        false
    }
    model.classes.values().filter(|c| !runtime.provides(&c.name)).any(|c| {
        c.methods.iter().map(|m| &m.function).chain([&c.constructor]).any(|f| walk(&f.body, &mut BTreeSet::new(), &makes, &runtime.jumps))
    })
}

/// whether a placed clip does nothing anyone can see once nobody's code reaches it: its class (and
/// each it extends that the package defines, down to Flash's own clip) only sets its own
/// properties to plain values, stops or plays itself and registers its frame scripts, and every
/// clip its frames place is the same (a box the source's engine reads off the art, which this
/// one has in its entity)
fn keeps_to_itself(model: &SourceModel, pkg: &compile::Package, sym: u16, seen: &mut BTreeSet<u16>) -> bool {
    use crate::model::ast::{Place, Stmt};
    if !seen.insert(sym) { return true }
    fn plain(e: &Expr) -> bool {
        matches!(e, Expr::Undefined | Expr::Null | Expr::Bool(_) | Expr::Num(_) | Expr::Int(_) | Expr::Str(_))
            || matches!(e, Expr::Unary(_, x) | Expr::Coerce(_, x) | Expr::Convert(_, x) if plain(x))
    }
    fn own(e: &Expr) -> bool { matches!(e, Expr::This) }
    fn quiet(body: &[Stmt]) -> bool {
        body.iter().all(|s| match s {
            Stmt::Return(None) => true,
            Stmt::Assign(Place::Member(o, _), v) => own(o) && plain(v),
            Stmt::Assign(Place::Name(_), v) => plain(v),
            Stmt::Expr(Expr::SuperCall(..)) => true,
            Stmt::Expr(Expr::Call(c, a)) => {
                let m = match &**c { Expr::Member(o, m) if own(o) => m, Expr::Name(m) => m, _ => return false };
                matches!(m.as_str(), "stop" | "play" | "addFrameScript")
                    && a.iter().all(|x| plain(x) || matches!(x, Expr::Member(o, _) if own(o)) || matches!(x, Expr::Name(_)))
            }
            _ => false,
        })
    }
    if let Some(cls) = model.symbols.get(&sym) {
        if pkg.runtime.provides(cls) && !matches!(compile::short(cls), "MovieClip" | "Sprite") { return false }
        if !pkg.runtime.provides(cls) {
            let (chain, base) = pkg.chain(cls);
            if chain.is_empty() || !base.as_deref().is_some_and(|b| matches!(compile::short(b), "MovieClip" | "Sprite")) { return false }
            for c in &chain {
                let bodies = c.methods.iter().map(|m| &m.function).chain([&c.constructor, &c.static_init]);
                for f in bodies { if !quiet(&f.body) { return false } }
            }
        }
    }
    let Some(sp) = model.sprites.get(&sym) else { return true };
    sp.frames.iter().flatten().filter(|p| model.sprites.contains_key(&p.char_id))
        .all(|p| keeps_to_itself(model, pkg, p.char_id, seen))
}

const WALKS_CHILDREN: [&str; 4] = ["getChildAt", "numChildren", "removeChildAt", "getObjectsUnderPoint"];
/// a clip's methods the runtime itself never asks for, each made only for a package whose code
/// names it (by the name the code would write: a getter by its property)
const CLIP_OPTIONAL: [(&str, &str); 10] = [
    ("nextFrame", "nextFrame"), ("prevFrame", "prevFrame"), ("addChildAt", "addChildAt"),
    ("removeChildren", "removeChildren"), ("get_numChildren", "numChildren"), ("setChildIndex", "setChildIndex"),
    ("swapChildren", "swapChildren"), ("getChildAt", "getChildAt"), ("globalToLocal", "globalToLocal"),
    ("get_graphics", "graphics"),
];

/// A projectile's `Projectiles.frame` and `Projectiles.engineFrame`: the runtime's pieces its own
/// script names (its declared stats, the code that writes them, the events it listens for), in the
/// order the source's engine runs them. A projectile that names none of a piece's words goes
/// without that piece.
/// A clip and every clip placed inside it, as deep as they go.
fn clip_tree(model: &SourceModel, root: Option<u16>) -> BTreeSet<u16> {
    let mut seen = BTreeSet::new();
    let mut work: Vec<u16> = root.into_iter().collect();
    while let Some(s) = work.pop() {
        if !seen.insert(s) { continue }
        if let Some(sp) = model.sprites.get(&s) {
            work.extend(sp.frames.iter().flatten().map(|p| p.char_id).filter(|c| model.sprites.contains_key(c)));
        }
    }
    seen
}

fn projectile_frames(text: &str, item: bool) -> String {
    let says = |words: &[&str]| words.iter().any(|w| crate::dyn_re(&format!(r"\b{w}\b")).is_match(text));
    let ground = says(&["groundTouch", "GROUND_TOUCH"]);
    let mut frame = vec!["if (Projectiles.gone(me)) return;", "if (Projectiles.lifetime(me)) return;"];
    if ground { frame.push("Projectiles.floorStart(me);") }
    if ground || says(&["__landsTo"]) { frame.push("Projectiles.landing(me);") }
    if says(&["trailEffect"]) { frame.push("Projectiles.trail(me);") }
    frame.extend(["Projectiles.hasOwnCode(me);"]);
    let mut engine = vec!["if (Projectiles.gone(me)) return;"];
    if says(&["createSelfPlatform", "__selfPlatforms"]) { engine.push("Projectiles.platforms(me);") }
    if says(&["refreshRate"]) { engine.push("Projectiles.refresh(me);") }
    engine.extend(["Attacks.carry(me);", "Projectiles.lateHit(me);"]);
    if item { engine.extend(["Projectiles.held(me);", "if (me.__fm.exports.__holder != null) return;"]) }
    if ground { engine.push("Projectiles.floorRecord(me);") }
    if says(&["xdecay", "homingSpeed"]) { engine.push("Projectiles.steer(me);") }
    if says(&["projCollide", "projCollideOwner", "PROJ_COLLIDE", "PROJ_COLLIDE_OWNER"]) { engine.push("Projectiles.touchOwner(me);") }
    if says(&["influenceXMovement", "influenceYMovement"]) { engine.push("Projectiles.influence(me);") }
    if says(&["controlDirection"]) { engine.push("Projectiles.control(me);") }
    if says(&["rotate"]) { engine.push("Projectiles.rotate(me);") }
    format!("Projectiles.frame = function(me) {{\n\t{}\n}};\nProjectiles.engineFrame = function(me) {{\n\t{}\n}};\n",
        frame.join("\n\t"), engine.join("\n\t"))
}
