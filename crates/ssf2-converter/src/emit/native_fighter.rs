//! A fighter written as plain Fraymakers code (`compile::native`), where its own code can be.
//!
//! Its own class (the package's character class) runs as the fighter itself, and the classes of
//! the clips its animations show are its frame scripts. Its script is `runtime/native/fighter.hx`
//! filled in: that code, the rules the source's engine plays every fighter by
//! (`runtime/native/fighter_rules.hx`), and the tables they read, worked out from what it declares.
//! Anything the writer has no plain way to say keeps the fighter on the runtime, with the reasons.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::{clip_tree, label_jumps, Piece, Units};
use crate::compile::{self, native, Runtime};
use crate::model::declare::{Fighter, Value};
use crate::model::SourceModel;

/// what the native writer needs of a fighter, worked out once
pub struct Parts<'a> {
    pub itself: Option<&'a crate::model::ClassModel>,
    pub classes: Vec<&'a crate::model::ClassModel>,
    pub root: u16,
}

/// the fighter's own class and the classes of its clips that have code
pub fn parts<'a>(model: &'a SourceModel, runtime: &Runtime, fighter: &Fighter) -> Result<Parts<'a>, String> {
    let class = fighter.class().unwrap_or_default();
    let itself = if class.is_empty() || compile::short(class) == "PeptideCharacter" { None } else {
        Some(model.classes.get(class).or_else(|| model.classes.values().find(|c| compile::short(&c.name) == compile::short(class)))
            .ok_or_else(|| format!("its class {class} is not in its package"))?)
    };
    let root = fighter.clip().and_then(|c| model.symbol_of(c)).ok_or("its clip is no symbol")?;
    let mut classes = Vec::new();
    let mut seen = BTreeSet::new();
    for sym in clip_tree(model, Some(root)) {
        let Some(c) = model.class_of(sym) else { continue };
        if runtime.provides(&c.name) || !seen.insert(c.name.clone()) { continue }
        if super::native_projectile::is_quiet(model, runtime, &c.name) { continue }
        // (a clip of another object's, placed in its art: its class asks for the projectile or item it is)
        if another_objects(c) { continue }
        classes.push(c);
    }
    Ok(Parts { itself, classes, root })
}

/// whether a class's code is another object's: it asks the API for the projectile or item it is
fn another_objects(c: &crate::model::ClassModel) -> bool {
    use crate::model::ast::Expr;
    let mut is = false;
    for f in c.methods.iter().map(|m| &m.function) {
        crate::model::ast::walk_exprs(&f.body, &mut |e| if let Expr::Call(g, a) = e {
            if matches!(a.as_slice(), [Expr::This]) && matches!(&**g, Expr::Member(_, m) if matches!(m.as_str(), "getProjectile" | "getItem" | "getEnemy")) { is = true }
        });
    }
    is
}

/// what the writer is handed about a fighter, besides its code
pub struct Fighting<'a> {
    pub model: &'a SourceModel,
    pub runtime: &'a Runtime,
    pub fighter: &'a Fighter,
    pub pieces: &'a [Piece],
    pub units: Units,
    pub effects: &'a BTreeMap<String, Vec<Piece>>,
    pub entities: &'a BTreeMap<String, String>,
    /// the animation each of its clip's labels is
    pub anim_of: &'a BTreeMap<String, String>,
}

fn plain(v: &Value) -> Option<String> {
    match v {
        Value::Num(n) => Some(num(*n)),
        Value::Bool(b) => Some(b.to_string()),
        Value::Str(s) => Some(compile::quote(s)),
        Value::Null => Some("null".into()),
        _ => None,
    }
}

/// every animation that shows a label of a clip, by the clip's class: (animation, label, whether
/// the label is the animation's first frame)
fn stance_labels(model: &SourceModel, pieces: &[Piece]) -> BTreeMap<String, Vec<(String, String, bool)>> {
    let mut out: BTreeMap<String, Vec<(String, String, bool)>> = BTreeMap::new();
    // (a piece cut under another label's name still shows the clip's own frames: its own first)
    let mut own: Vec<&Piece> = pieces.iter().filter(|p| !p.reversed && p.head == 0).collect();
    own.sort_by_key(|p| p.borrowed);
    let syms: BTreeSet<u16> = own.iter().map(|p| p.sym).collect();
    for sym in syms {
        let Some(c) = model.class_of(sym) else { continue };
        let Some(sprite) = model.sprites.get(&sym) else { continue };
        let mine: Vec<&&Piece> = own.iter().filter(|p| p.sym == sym).collect();
        for (f0, label) in &sprite.labels {
            let f = *f0 as u32 + 1;
            let mut hit = false;
            for p in mine.iter().filter(|p| f >= p.from && f < p.from + p.frames) {
                out.entry(c.name.clone()).or_default().push((p.fm_name.clone(), label.clone(), f == p.from));
                hit = true;
            }
            // (a label between two of its animations: the clip plays on from it into the next)
            if !hit {
                if let Some(next) = mine.iter().filter(|p| p.from > f).min_by_key(|p| p.from) {
                    out.entry(c.name.clone()).or_default().push((next.fm_name.clone(), label.clone(), true));
                }
            }
        }
    }
    out
}

/// The fighter's code written natively, or why it cannot be yet (every reason, when `all`).
fn written<'a>(f: &Fighting<'a>, all: bool) -> Result<(native::Written, Parts<'a>), Vec<String>> {
    let parts = parts(f.model, f.runtime, f.fighter).map_err(|e| vec![e])?;
    let jumps = label_jumps(f.model, f.pieces);
    let labels = stance_labels(f.model, f.pieces);
    let mut frames_of: BTreeMap<String, Vec<(String, u32, u32)>> = BTreeMap::new();
    let mut shown: Vec<&Piece> = f.pieces.iter().filter(|p| !p.reversed && p.head == 0).collect();
    shown.sort_by_key(|p| p.borrowed);
    for p in shown {
        if let Some(c) = f.model.class_of(p.sym) { frames_of.entry(c.name.clone()).or_default().push((p.fm_name.clone(), p.from, p.frames)); }
    }
    let declares = |k: &str| -> Option<String> { f.fighter.stats.get(k).and_then(plain) };
    let no_stat = |_: &str| -> Option<(String, String)> { None };
    let model = f.model;
    let made = |name: &str| -> Option<(String, Option<u32>)> {
        if let Some((_, c)) = crate::mappings::api_commands().global_vfx_map.iter().find(|(g, _)| g.as_str() == name) {
            return Some((format!("spriteContent: \"global::vfx.vfx\", animation: GlobalVfx.{c}"), None));
        }
        let ps = f.effects.get(name)?;
        let p = ps.iter().find(|p| p.from == 1)?;
        let gone = if ps.len() == 1 && (p.sym == 0 || model.class_of(p.sym).is_none()) { None } else { super::plays_once(model, p)? };
        Some((format!("spriteContent: self.getResource().getContent({}), animation: {}", compile::quote(name), compile::quote(&p.fm_name)), gone))
    };
    let fired = |name: &str| -> Option<(String, [f64; 4])> {
        let decl = f.fighter.projectiles.get(name)?;
        let key = f.entities.get(name)?;
        let n = |ks: &[&str]| ks.iter().find_map(|k| match decl.get(k) { Some(Value::Num(v)) => Some(*v), _ => None }).unwrap_or(0.0);
        Some((crate::projectile_gen::projectile_content_id(key),
              [n(&["xoffset", "xOffset", "x_offset"]), n(&["yoffset", "yOffset", "y_offset"]), n(&["xspeed", "xSpeed", "x_speed"]), n(&["yspeed", "ySpeed", "y_speed"])]))
    };
    let pkg = compile::Package::new(f.model, f.runtime);
    let stores = |n: &str| pkg.stored.contains(n);
    let anims: Vec<(String, String)> = f.pieces.iter().map(|p| (p.fm_name.clone(), p.label.clone())).collect();
    let switches = BTreeMap::new();
    let root_anim = |l: &str| f.anim_of.get(l).cloned();
    let stat = |k: &str| match f.fighter.stats.get(k) { Some(Value::Num(n)) => *n, _ => 0.0 };
    let start = stat("accel_start") * stat("norm_xSpeed") * f.units.speed;
    let object = native::Object {
        model, itself: parts.itself, classes: parts.classes.clone(), jumps: &jumps, runtime: f.runtime, declared: &declares, switches: &switches,
        owner_stat: &no_stat, effect: &made, owner_anims: &anims, fired: &fired, owner_declares: &declares, owner_stores: &stores,
        root: parts.root, size: f.units.size, speed: f.units.speed, accel: f.units.accel, fighter: true,
        root_anim: &root_anim, walk_start: (start > 0.0).then_some(start), stance_labels: &labels, clip_frames: &frames_of, owner_native: false,
    };
    if all {
        let why = native::census(&object);
        if !why.is_empty() { return Err(why) }
    }
    let w = native::write(&object).map_err(|e| vec![e])?;
    Ok((w, parts))
}

/// every reason the fighter's code is not written natively yet
pub fn census(f: &Fighting) -> Vec<String> {
    match written(f, true) { Ok(_) => Vec::new(), Err(e) => e }
}

/// The fighter's script, written natively, with its entity's keyframes calling its frames; or
/// every reason it stays on the runtime.
pub fn write(char_dir: &Path, char_pascal: &str, title: &str, f: &Fighting, first_frame: &BTreeMap<String, String>, stat_code: &[String])
    -> Result<String, Vec<String>> {
    let (w, parts) = written(f, true)?;
    let rt = f.runtime;
    let skeleton = rt.native_files.get("fighter").ok_or_else(|| vec!["the runtime has no native/fighter.hx".to_string()])?;
    let one = |e: String| vec![e];
    // what its animations run on their first frame for what their attacks declare: plain code
    // only (a row the runtime answers keeps it there)
    let runtime_names = crate::re!(r"\b(PeptideRuntime|AS3|Attacks|Fighters|Objects|Events|Timers|DATA)\.");
    let mut why = Vec::new();
    for (anim, code) in first_frame {
        if runtime_names.is_match(code) { why.push(format!("{anim}: what its declared stats have it do reads the runtime")); }
    }
    let code_text = stat_code.join("\n");
    if runtime_names.is_match(&code_text) { why.push("what its declared stats have it do reads the runtime".into()); }
    if !why.is_empty() { return Err(why) }

    // the rules every fighter is played by, and what its code calls of them
    let mut blocks: BTreeSet<String> = w.blocks.clone();
    for b in ["attack", "sounds", "refresh", "hits", "asked", "start_attack"] { blocks.insert(b.into()); }
    // the attack a rocket rides in: the one whose clip has a `rocket` frame to start it at (its air
    // one where it has two)
    let mut rocketing: Vec<&str> = f.pieces.iter().filter(|p| f.model.sprites.get(&p.sym).is_some_and(|sp| sp.labels.iter().any(|(_, l)| l == "rocket")))
        .map(|p| p.label.as_str()).filter(|l| !l.is_empty()).collect();
    rocketing.sort_by_key(|l| (!l.ends_with("_air"), l.to_string()));
    rocketing.dedup();
    let rocket = rocketing.first().map(|l| compile::quote(l));
    if rocket.is_some() { blocks.insert("rocket".into()); }
    let mut functions = w.functions.clone();
    let order = ["attack", "sounds", "refresh", "hits", "asked", "rocket", "walk_start", "end_move", "controls", "start_attack", "floor_hold", "effects", "items", "actions", "place_effect", "last_used", "char_stats", "rects", "move_clear", "fired_stats", "timers", "ledges", "own_floor", "nearest"];
    for b in order.iter().filter(|b| blocks.contains(**b)) {
        let words: Vec<(&str, String)> = match *b {
            "walk_start" => {
                let stat = |k: &str| match f.fighter.stats.get(k) { Some(Value::Num(n)) => *n, _ => 0.0 };
                vec![("START", num(stat("accel_start") * stat("norm_xSpeed") * f.units.speed))]
            }
            "items" => vec![("TOSS", num(f.units.item_toss)), ("SIZE", num(f.units.size)), ("SPEED", num(f.units.speed))],
            "rects" | "move_clear" | "ledges" | "own_floor" => vec![("SIZE", num(f.units.size))],
            "rocket" => vec![("ATTACK", rocket.clone().unwrap_or_else(|| "null".into())), ("SPEED", num(f.units.speed)),
                ("BLOCKED", native::state_tests(rt, "self", &[10, 13, 14, 15, 19, 20])), ("ATTACKING", native::state_tests(rt, "self", &[9])),
                ("STOPS", native::state_tests(rt, "self", &[22, 16, 18]))],
            _ => Vec::new(),
        };
        functions.push(format!("// ---- {b}: runtime/native/fighter_rules.hx\n{}", native::helper(rt, "fighter_rules", b, "", &words, &[]).map_err(one)?));
    }
    // the tables the rules read: the attack each animation is, and what each attack declares
    let attacks = &f.fighter.attacks;
    let table = |key: &str, lines: &[String]| native::helper(rt, "fighter_tables", key, "", &[], lines).map_err(|e| vec![e]);
    let mut cases: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in f.pieces { if !p.label.is_empty() && attacks.get(&p.label).is_some() { cases.entry(p.label.clone()).or_default().push(compile::quote(&p.fm_name)); } }
    let labels: Vec<String> = cases.iter().map(|(label, anims)| { let mut a = anims.clone(); a.dedup(); format!("case {}: return {};", a.join(", "), compile::quote(label)) }).collect();
    let mut keys: BTreeSet<String> = w.stat_keys.iter().filter(|k| !k.contains(':') && !k.ends_with('*')).cloned().collect();
    for k in ["cancelSoundOnEnd", "cancelVoiceOnEnd", "refreshRate"] { keys.insert(k.into()); }
    let box_keys = ["hitLag", "bypassGrabbed", "bypassNonGrabbed", "hasEffect", "reverse_character", "poison", "poisonInterval", "poisonLength"];
    let mut t = Vec::new();
    let mut boxes = Vec::new();
    for (label, a) in attacks.entries() {
        if !cases.contains_key(label) { continue }
        // (a sound numbered as it runs reads any of the attack's numbered ones)
        let mut ks: BTreeSet<String> = keys.clone();
        for pre in ["attackSound", "attackVoice"] {
            if w.stat_keys.contains(&format!("{pre}*")) { ks.extend(a.entries().iter().map(|(k, _)| k.clone()).filter(|k| k.starts_with(pre) && k.ends_with("_id"))); }
        }
        let stats: Vec<String> = ks.iter().filter_map(|k| a.get(k).and_then(plain).filter(|v| v != "null").map(|v| format!("\t\tcase {}: return {v};", compile::quote(k)))).collect();
        if !stats.is_empty() {
            t.push(format!("case {}:", compile::quote(label)));
            t.push("\tswitch (k) {".into());
            t.extend(stats);
            t.push("\t}".into());
        }
        let mut per: Vec<String> = Vec::new();
        for (bn, b) in a.get("attackBoxes").map(|b| b.entries().to_vec()).unwrap_or_default() {
            let i = if bn == "attackBox" { 1 } else { match bn.strip_prefix("attackBox").and_then(|n| n.parse::<u32>().ok()) { Some(n) => n, None => continue } };
            let st: Vec<String> = box_keys.iter().filter_map(|k| b.get(k).and_then(plain).filter(|v| v != "null").map(|v| format!("\t\t\tcase {}: return {v};", compile::quote(k)))).collect();
            if st.is_empty() { continue }
            per.push(format!("\tcase {i}:"));
            per.push("\t\tswitch (k) {".into());
            per.extend(st);
            per.push("\t\t}".into());
        }
        if !per.is_empty() {
            boxes.push(format!("case {}:", compile::quote(label)));
            boxes.push("switch (i) {".into());
            boxes.extend(per);
            boxes.push("}".into());
        }
    }
    functions.push(table("attack_label", &labels)?);
    // what other objects' code asks of it: its labels' animations, its clips' fields
    let anim_cases: Vec<String> = f.anim_of.iter().map(|(l, a)| format!("case {}: return {};", compile::quote(l), compile::quote(a))).collect();
    functions.push(table("anim_of", &anim_cases)?);
    let mut by_class: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for ((class, field), id) in &w.field_ids { by_class.entry(class.as_str()).or_default().push(format!("\t\tcase {}: return {id};", compile::quote(field))); }
    let mut field_cases = Vec::new();
    for (class, lines) in by_class {
        field_cases.push(format!("case {}:", compile::quote(class)));
        field_cases.push("\tswitch (name) {".into());
        field_cases.extend(lines);
        field_cases.push("\t}".into());
    }
    let itself = parts.itself.map(|c| compile::quote(&c.name)).unwrap_or_else(|| "null".into());
    let root = f.model.class_of(parts.root).map(|c| compile::quote(&c.name)).unwrap_or_else(|| "null".into());
    functions.push(native::helper(rt, "fighter_tables", "field_of", "", &[("ITSELF", itself), ("ROOT", root)], &field_cases).map_err(one)?);
    let mut stance: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in f.pieces { if let Some(c) = f.model.class_of(p.sym) { stance.entry(c.name.clone()).or_default().push(compile::quote(&p.fm_name)); } }
    let stance_cases: Vec<String> = stance.iter().map(|(c, a)| { let mut a = a.clone(); a.dedup(); format!("case {}: return {};", a.join(", "), compile::quote(c)) }).collect();
    functions.push(table("stance_class", &stance_cases)?);
    functions.push(table("declared", &t)?);
    functions.push(table("declared_box", &boxes)?);
    if blocks.contains("items") { functions.extend(item_tables(f).map_err(|e| vec![e])?); }
    if code_text.contains("clipLabel(") || functions.iter().any(|t| t.contains("clipLabel(")) { functions.push(table("clip_label", &clip_labels(f))?); }
    if blocks.contains("fired_stats") {
        let wanted: BTreeSet<&str> = w.stat_keys.iter().filter_map(|k| k.strip_prefix("projectile:")).collect();
        let mut l = Vec::new();
        for (name, d) in f.fighter.projectiles.entries() {
            let st: Vec<String> = wanted.iter().filter_map(|k| d.get(k).and_then(plain).map(|v| format!("\t\tcase {}: return {v};", compile::quote(k)))).collect();
            if st.is_empty() { continue }
            l.push(format!("case {}:", compile::quote(name)));
            l.push("\tswitch (k) {".into());
            l.extend(st);
            l.push("\t}".into());
        }
        functions.push(table("fired_declared", &l)?);
    }
    if blocks.contains("actions") {
        let mut l = Vec::new();
        for k in w.stat_keys.iter().filter_map(|k| k.strip_prefix("anims:")) {
            let mut a: Vec<String> = f.pieces.iter().filter(|p| p.label == k).map(|p| compile::quote(&p.fm_name)).collect();
            a.dedup();
            l.push(format!("case {}: return [{}];", compile::quote(k), a.join(", ")));
        }
        functions.push(table("label_anims", &l)?);
    }
    if !stat_code.is_empty() {
        functions.push(format!("// ---- what its attacks' declared stats and the source's own states have it do: runtime/attack_stats.jsonc, runtime/engine_animations.jsonc\n{code_text}"));
    }

    let mut slots: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    slots.insert("fields", w.fields.clone());
    slots.insert("functions", functions.join("\n\n").lines().map(String::from).collect());
    let mut publish = w.publish.clone();
    publish.push("// what another object's code asks of it: an attack started, a rocket ridden, its clips' fields".into());
    publish.push("self.exports.__forceAttack = forceAttackAsked;".into());
    if rocket.is_some() { publish.push("self.exports.__rocket = rocketAsked;".into()); }
    publish.push("self.exports.__fieldOf = fieldOf;".into());
    slots.insert("publish", publish);
    slots.insert("start", vec!["listenForHits();".into()]);
    slots.insert("own_start", w.start.clone());
    let mut update = vec!["hitFrame();".to_string(), "attackFrame();".into()];
    if blocks.contains("last_used") { update.push("usedFrame();".into()); }
    if blocks.contains("own_floor") { update.push("ownFloorFrame();".into()); }
    if blocks.contains("items") { update.push("itemFrame();".into()); }
    slots.insert("update", update);
    slots.insert("own_tick", w.tick.clone());
    let anims: BTreeSet<String> = f.pieces.iter().map(|p| p.fm_name.clone()).collect();
    let names: Vec<String> = anims.iter().map(|a| compile::quote(a)).collect();
    let classes: Vec<&str> = parts.itself.into_iter().chain(parts.classes.iter().copied()).map(|c| c.name.as_str()).collect();
    let script = native::fill_slots(skeleton, &slots)
        .replace("{title}", title)
        .replace("{classes}", &classes.join(", "))
        .replace("/* {anims} */", &names.join(", "))
        .replace("\t// what its own class runs each of the source's frames (every second engine frame)\n\tif (frames.get() % 2 == 1) {\n\t}\n", "");

    // its entity: each frame with code calls it, and what each animation's attack declares runs on its first
    let entity = char_dir.join(format!("library/entities/{char_pascal}.entity"));
    let frames = w.frames.clone();
    let mut extra: BTreeMap<(String, u32), String> = BTreeMap::new();
    let field_ref = crate::re!(r"\{field:(\w+)\}");
    for (anim, code) in first_frame {
        // (a field of the clip the animation shows: its container, or none where its code keeps none)
        let class = f.pieces.iter().find(|p| &p.fm_name == anim).and_then(|p| f.model.class_of(p.sym)).map(|c| c.name.clone()).unwrap_or_default();
        let code = field_ref.replace_all(code, |c: &regex::Captures| w.field_ids.get(&(class.clone(), c[1].to_string())).cloned().unwrap_or_else(|| "null".into())).to_string();
        extra.insert((anim.clone(), 0), code);
    }
    super::native_projectile::keyframes_with(&entity, title, f.model, f.pieces, &frames, false, &extra).map_err(|e| vec![e.to_string()])?;
    Ok(script)
}

/// the label its clip is on (the last one at or before the frame it shows), as the source reads it
fn clip_labels(f: &Fighting) -> Vec<String> {
    let mut t = Vec::new();
    for p in f.pieces.iter().filter(|p| !p.reversed && p.head == 0) {
        let Some(sprite) = f.model.sprites.get(&p.sym) else { continue };
        let mut labels: Vec<(u32, &str)> = sprite.labels.iter().map(|(f0, l)| (*f0 as u32 + 1, l.as_str())).collect();
        labels.sort();
        let before = labels.iter().rfind(|(fr, _)| *fr <= p.from).map(|(_, l)| *l);
        let inside: Vec<(u32, &str)> = labels.iter().filter(|(fr, _)| *fr > p.from && *fr < p.from + p.frames).map(|(fr, l)| (fr - p.from, *l)).collect();
        if before.is_none() && inside.is_empty() { continue }
        t.push(format!("case {}:", compile::quote(&p.fm_name)));
        for (k, l) in inside.iter().rev() { t.push(format!("\tif (k >= {k}) return {};", compile::quote(l))); }
        t.push(format!("\treturn {};", before.map(compile::quote).unwrap_or_else(|| "null".into())));
    }
    t
}

/// the tables its items block reads: where its item box is on each frame of each animation, the
/// content each item it can make is, and what each declares
fn item_tables(f: &Fighting) -> Result<Vec<String>, String> {
    let rt = f.runtime;
    let mut rows: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in f.pieces.iter().filter(|p| !p.item_box.is_empty()) {
        let row: Vec<String> = p.item_box.iter().map(|(k, x, y)| format!("{k}, {}, {}", num(*x), num(*y))).collect();
        rows.entry(row.join(", ")).or_default().push(compile::quote(&p.fm_name));
    }
    let boxes: Vec<String> = rows.iter().map(|(row, anims)| format!("case {}: return boxAt(k, [{row}]);", anims.join(", "))).collect();
    let mut content = Vec::new();
    for (name, _) in f.fighter.items.entries() {
        if let Some(key) = f.entities.get(name) { content.push(format!("case {}: return {};", compile::quote(name), compile::quote(&crate::projectile_gen::projectile_content_id(key)))); }
    }
    let mut declared = Vec::new();
    for (name, d) in f.fighter.items.entries() {
        let st: Vec<String> = d.entries().iter().filter_map(|(k, v)| plain(v).map(|v| format!("\t\tcase {}: return {v};", compile::quote(k)))).collect();
        if st.is_empty() { continue }
        declared.push(format!("case {}:", compile::quote(name)));
        declared.push("\tswitch (k) {".into());
        declared.extend(st);
        declared.push("\t}".into());
    }
    Ok(vec![native::helper(rt, "fighter_tables", "item_box", "", &[], &boxes)?,
            native::helper(rt, "fighter_tables", "item_content", "", &[], &content)?,
            native::helper(rt, "fighter_tables", "item_declared", "", &[], &declared)?])
}

fn num(n: f64) -> String { if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") } }
