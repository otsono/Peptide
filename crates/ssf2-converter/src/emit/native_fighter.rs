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
    /// every projectile entity its package has, by its key (an enemy made by its class is one
    /// named as the class)
    pub entity_keys: &'a BTreeSet<String>,
    /// the animations the engine links at the same frame, each way (`frame_links`)
    pub frame_links: &'a BTreeSet<(String, String)>,
    /// the code its clips' autolink calls become, by class (`autolink_calls`)
    pub autolink: &'a BTreeMap<String, (String, String)>,
    /// its parts' frames and where each animation places them (`convert`'s fighter parts)
    pub parts: &'a [crate::entity_gen::ProjectilePartData],
    pub part_placements: &'a [crate::entity_gen::PartPlacement],
    /// the content its own animations are
    pub content: &'a str,
    /// its costumes' colours (`costumes.palettes`, as written)
    pub palette: Option<(Vec<u32>, Vec<Vec<u32>>)>,
    /// the clips each effect places that its code turns
    pub effect_inners: &'a BTreeMap<String, Vec<crate::entity_gen::InnerClip>>,
}

/// the animations the engine links at the same frame, each way: what its animation stats give a
/// link-frames land or leave type, and the animation it links to
pub fn frame_links(stats: &str) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for line in stats.lines() {
        let (Some(o), Some(c)) = (line.find('{'), line.rfind('}')) else { continue };
        let Some(name) = line[..o].trim().strip_suffix(':').map(str::trim) else { continue };
        let props: BTreeMap<&str, &str> = line[o + 1..c].split(',').filter_map(|p| p.split_once(':')).map(|(k, v)| (k.trim(), v.trim().trim_matches('"'))).collect();
        for (kind, to) in [("landType", "landAnimation"), ("leaveGroundType", "leaveGroundAnimation")] {
            if props.get(kind).is_some_and(|t| t.ends_with(".LINK_FRAMES")) {
                if let Some(a) = props.get(to) { out.insert((name.to_string(), a.to_string())); out.insert((a.to_string(), name.to_string())); }
            }
        }
    }
    out
}

pub(super) fn plain(v: &Value) -> Option<String> {
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
pub fn stance_labels(model: &SourceModel, pieces: &[Piece]) -> BTreeMap<String, Vec<(String, String, bool)>> {
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

/// the clip frames each animation shows (its first, how many), by the clip's class
pub fn clip_frames_of(model: &SourceModel, pieces: &[Piece]) -> BTreeMap<String, Vec<(String, u32, u32)>> {
    let mut frames_of: BTreeMap<String, Vec<(String, u32, u32)>> = BTreeMap::new();
    let mut shown: Vec<&Piece> = pieces.iter().filter(|p| !p.reversed && p.head == 0).collect();
    shown.sort_by_key(|p| p.borrowed);
    for p in shown {
        if let Some(c) = model.class_of(p.sym) { frames_of.entry(c.name.clone()).or_default().push((p.fm_name.clone(), p.from, p.frames)); }
    }
    frames_of
}

/// its costumes' colours from the palettes the conversion writes: the colours its art is drawn in
/// (in the source's order) and, per costume in the engine's order, what each turns into
pub fn palette_of(text: &str) -> Option<(Vec<u32>, Vec<Vec<u32>>)> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let hex = |e: &serde_json::Value, k: &str| e.get(k).and_then(|s| s.as_str()).and_then(|s| u32::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok());
    let colors: Vec<u32> = v.get("colors")?.as_array()?.iter().filter_map(|e| hex(e, "color")).collect();
    let maps: Vec<Vec<u32>> = v.get("maps")?.as_array()?.iter().map(|m| {
        m.get("colors").and_then(|c| c.as_array()).map(|cs| cs.iter().filter_map(|e| hex(e, "targetColor")).collect()).unwrap_or_default()
    }).collect();
    (!colors.is_empty()).then_some((colors, maps))
}

/// its parts' art as effect animations (a jump of one by frame plays them as an effect's): the
/// whole timeline first, then each label's cut of it
fn part_pieces(parts: &[crate::entity_gen::ProjectilePartData]) -> Vec<(String, String, u32, u32)> {
    let mut out = Vec::new();
    for p in parts {
        let total = p.frame_count.max(1) as u32;
        out.push((p.anim.clone(), p.anim.clone(), 1, total));
        for (i, (label, at)) in p.labels.iter().enumerate() {
            let end = p.labels.get(i + 1).map(|(_, f)| *f as u32).unwrap_or(total + 1);
            out.push((p.anim.clone(), format!("{}_{}", p.anim, crate::haxe_gen::label_ident(label)), *at as u32, end.saturating_sub(*at as u32).max(1)));
        }
    }
    out
}

/// its parts, by name, as each animation places them (one with no art of its own shows nothing)
fn shown_parts(placements: &[crate::entity_gen::PartPlacement]) -> Vec<native::Part> {
    let mut by: BTreeMap<String, native::Part> = BTreeMap::new();
    for pl in placements.iter().filter(|p| !p.art.is_empty()) {
        let e = by.entry(pl.name.clone()).or_insert_with(|| native::Part { name: pl.name.clone(), placed: Vec::new() });
        let m = &pl.matrix;
        e.placed.push((pl.fm_anim.clone(), pl.art.clone(), m.tx, m.ty, m.rotation, m.sx, m.sy));
    }
    by.into_values().collect()
}

/// The fighter's code written natively, or why it cannot be yet (every reason, when `all`).
fn written<'a>(f: &Fighting<'a>, all: bool) -> Result<(native::Written, Parts<'a>), Vec<String>> {
    let parts = parts(f.model, f.runtime, f.fighter).map_err(|e| vec![e])?;
    let mut jumps = label_jumps(f.model, f.pieces);
    for (c, code) in f.autolink { jumps.entry(c.clone()).or_default().autolink = Some(code.clone()); }
    let labels = stance_labels(f.model, f.pieces);
    let frames_of = clip_frames_of(f.model, f.pieces);
    let declares = |k: &str| -> Option<String> { f.fighter.stats.get(k).and_then(plain) };
    let no_stat = |_: &str| -> Option<(String, String)> { None };
    let model = f.model;
    let made = |name: &str| -> Option<(String, Option<u32>)> {
        if let Some((_, c)) = crate::mappings::api_commands().global_vfx_map.iter().find(|(g, _)| g.as_str() == name) {
            return Some((format!("spriteContent: \"global::vfx.vfx\", animation: GlobalVfx.{c}"), None));
        }
        let ps = f.effects.get(name)?;
        let p = ps.iter().find(|p| p.from == 1)?;
        // (one whose clip's own frames turn it is kept playing, and its code done for it: `effect_code`)
        if super::effect_code(model, ps).is_some() && super::plays_once(model, p).is_none() {
            return Some((format!("spriteContent: self.getResource().getContent({}), animation: {}, loop: true", compile::quote(name), compile::quote(&p.fm_name)), None));
        }
        let gone = if ps.len() == 1 && (p.sym == 0 || model.class_of(p.sym).is_none()) { None } else { super::plays_once(model, p)? };
        Some((format!("spriteContent: self.getResource().getContent({}), animation: {}", compile::quote(name), compile::quote(&p.fm_name)), gone))
    };
    let fired = |name: &str| -> Option<(String, [f64; 4], Option<String>)> {
        let key = f.entities.get(name).or_else(|| f.entity_keys.get(name))?;
        // (an enemy made by its class declares no launch)
        let Some(decl) = f.fighter.projectiles.get(name) else { return Some((crate::projectile_gen::projectile_content_id(key), [0.0; 4], None)) };
        let n = |ks: &[&str]| ks.iter().find_map(|k| match decl.get(k) { Some(Value::Num(v)) => Some(*v), _ => None }).unwrap_or(0.0);
        Some((crate::projectile_gen::projectile_content_id(key),
              [n(&["xoffset", "xOffset", "x_offset"]), n(&["yoffset", "yOffset", "y_offset"]), n(&["xspeed", "xSpeed", "x_speed"]), n(&["yspeed", "ySpeed", "y_speed"])],
              super::fired_record(&super::Decls::of(f.fighter), f.entities, name)))
    };
    let pkg = compile::Package::new(f.model, f.runtime);
    let stores = |n: &str| pkg.stored.contains(n);
    let anims: Vec<(String, String)> = f.pieces.iter().map(|p| (p.fm_name.clone(), p.label.clone())).collect();
    let switches = BTreeMap::new();
    let root_anim = |l: &str| f.anim_of.get(l).cloned();
    let stat = |k: &str| match f.fighter.stats.get(k) { Some(Value::Num(n)) => *n, _ => 0.0 };
    let start = stat("accel_start") * stat("norm_xSpeed") * f.units.speed;
    let mut clips: Vec<(String, u16, u32, u32)> = f.pieces.iter().filter(|p| !p.reversed && p.head == 0).map(|p| (p.fm_name.clone(), p.sym, p.from, p.frames)).collect();
    clips.sort();
    let object = native::Object {
        model, itself: parts.itself, classes: parts.classes.clone(), jumps: &jumps, runtime: f.runtime, declared: &declares, switches: &switches,
        owner_stat: &no_stat, effect: &made, owner_anims: &anims, fired: &fired, owner_declares: &declares, owner_stores: &stores,
        root: parts.root, size: f.units.size, speed: f.units.speed, accel: f.units.accel, fighter: true, item: false, stage: false, clip_parts: BTreeMap::new(), made_objects: BTreeMap::new(), particles: BTreeMap::new(), clip_bounds: BTreeMap::new(),
        root_anim: &root_anim, walk_start: (start > 0.0).then_some(start), stance_labels: &labels, clip_frames: &frames_of, frame_links: f.frame_links, anim_clips: &clips, owner_native: false,
        effect_names: f.effects.keys().cloned().chain(crate::mappings::api_commands().global_vfx_map.keys().cloned()).collect(),
        effect_pieces: f.effects.iter().flat_map(|(n, ps)| ps.iter().map(move |p| (n.clone(), p.fm_name.clone(), p.from, p.frames)))
            .chain(part_pieces(f.parts))
            .chain(f.effect_inners.iter().flat_map(|(n, cs)| cs.iter().map(move |c| (format!("{n}/{}", c.name), c.art.clone(), 1, c.frames.max(1))))).collect(),
        parts: shown_parts(f.part_placements),
        content: f.content.to_string(),
        palette: f.palette.clone(),
        effect_inners: f.effect_inners.clone(),
        effect_code: f.effects.iter().filter_map(|(n, ps)| {
            let p = ps.iter().find(|p| p.from == 1)?;
            if super::plays_once(model, p).is_some() { return None }
            super::effect_code(model, ps).map(|c| (n.clone(), c))
        }).collect(),
        instance: None, origin: (0.0, 0.0), power_record: power_of(f).is_some(),
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
    -> Result<(String, BTreeSet<String>, BTreeMap<String, String>), Vec<String>> {
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
    // (by the animations that show it: the attack is the one its label table says the animation
    // is, as an air piece of a clip it shares carries the clip's label)
    let mut rocketing: Vec<String> = f.pieces.iter().filter(|p| f.model.sprites.get(&p.sym).is_some_and(|sp| sp.labels.iter().any(|(_, l)| l == "rocket")))
        .filter_map(|p| f.anim_of.iter().find(|(_, a)| **a == p.fm_name).map(|(l, _)| l.clone()).or_else(|| (!p.label.is_empty()).then(|| p.label.clone()))).collect();
    rocketing.sort_by_key(|l| (!l.ends_with("_air"), l.to_string()));
    rocketing.dedup();
    let rocket = rocketing.first().map(|l| compile::quote(l));
    if rocket.is_some() { blocks.insert("rocket".into()); }
    let mut functions = w.functions.clone();
    let order = ["attack", "sounds", "refresh", "hits", "asked", "rocket", "controls", "start_attack", "floor_hold", "effects", "items", "actions", "place_effect", "tint", "tick_end", "mid_jump", "last_used", "char_stats", "ledges", "home", "own_floor", "nearest", "ground_test", "recover", "copy_power"];
    for b in order.iter().filter(|b| blocks.contains(**b)) {
        let words: Vec<(&str, String)> = match *b {
            "items" => vec![("TOSS", num(f.units.item_toss)), ("SIZE", num(f.units.size)), ("SPEED", num(f.units.speed))],
            "ledges" | "own_floor" | "ground_test" => vec![("SIZE", num(f.units.size))],
            "copy_power" => {
                // (the move that swallows, read from the clip whose code takes the power)
                let m = copy_move(f);
                let q = |v: Option<&String>| v.map(|t| compile::quote(t)).unwrap_or_else(|| "null".into());
                let moves = m.as_ref().map(|m| m.moves.iter().map(|x| compile::quote(x)).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                vec![("SIZE", num(f.units.size)), ("SPEED", num(f.units.speed)), ("COPY_MOVES", format!("[{moves}]")),
                     ("SWALLOW", q(m.as_ref().map(|m| &m.swallow))), ("SPIT", q(m.as_ref().and_then(|m| m.spit.as_ref()))),
                     ("SUCKING", q(m.as_ref().and_then(|m| m.sucking.as_ref()))), ("POWER_FIELD", q(m.as_ref().map(|m| &m.field))),
                     ("STRIKE_FRAMES", strike_frames(&char_dir.join(format!("library/entities/{char_pascal}.entity"))))]
            }
            "home" => vec![("SPEED", num(f.units.speed))],
            "rocket" => vec![("ATTACK", rocket.clone().unwrap_or_else(|| "null".into())), ("SPEED", num(f.units.speed)),
                ("BLOCKED", native::state_tests(rt, "self", &[10, 13, 14, 15, 19, 20])), ("ATTACKING", native::state_tests(rt, "self", &[9])),
                ("STOPS", native::state_tests(rt, "self", &[22, 16, 18]))],
            _ => Vec::new(),
        };
        // (a fighter that takes powers plays a power's own move where it has no animation for it)
        let cases: Vec<String> = if *b == "asked" && blocks.contains("copy_power") { vec!["if (borrowedMove(label)) return;".into()] } else { Vec::new() };
        functions.push(format!("// ---- {b}: runtime/native/fighter_rules.hx\n{}", native::helper(rt, "fighter_rules", b, "", &words, &cases).map_err(one)?));
    }
    // the tables the rules read: the attack each animation is, and what each attack declares
    let attacks = &f.fighter.attacks;
    let table = |key: &str, lines: &[String]| native::helper(rt, "fighter_tables", key, "", &[], lines).map_err(|e| vec![e]);
    let mut cases: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in f.pieces { if !p.label.is_empty() && attacks.get(&p.label).is_some() { cases.entry(p.label.clone()).or_default().push(compile::quote(&p.fm_name)); } }
    let mut keys: BTreeSet<String> = w.stat_keys.iter().filter(|k| !k.contains(':') && !k.ends_with('*')).cloned().collect();
    for k in ["cancelSoundOnEnd", "cancelVoiceOnEnd", "refreshRate"] { keys.insert(k.into()); }
    let box_keys = ["hitLag", "bypassGrabbed", "bypassNonGrabbed", "hasEffect", "reverse_character", "poison", "poisonInterval", "poisonLength"];
    // (each animation carries what the source says of it as its metadata: the label it shows, the
    // class of its clip, the labels inside it; an attack's, the attack, what it declares that this
    // engine has no stat for, and its boxes' by the source's box number)
    let mut fields_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in f.pieces {
        if fields_of.contains_key(&p.fm_name) { continue }
        let mut fs = Vec::new();
        if !p.label.is_empty() { fs.push(format!("label: {}", compile::quote(&p.label))); }
        if let Some(c) = f.model.class_of(p.sym) { fs.push(format!("clip: {}", compile::quote(&c.name))); }
        if let Some((before, inside)) = clip_label_data(f.model, p) {
            fs.push(format!("clipLabel: {}", before.map(|b| compile::quote(&b)).unwrap_or_else(|| "null".into())));
            if !inside.is_empty() { fs.push(format!("clipLabels: [{}]", inside.iter().map(|(k, l)| format!("[{k}, {}]", compile::quote(l))).collect::<Vec<_>>().join(", "))); }
        }
        fields_of.insert(p.fm_name.clone(), fs);
    }
    let mut meta: BTreeMap<String, String> = BTreeMap::new();
    for (label, a) in attacks.entries() {
        let Some(anims) = cases.get(label) else { continue };
        // (a sound numbered as it runs reads any of the attack's numbered ones)
        let mut ks: BTreeSet<String> = keys.clone();
        for pre in ["attackSound", "attackVoice"] {
            if w.stat_keys.contains(&format!("{pre}*")) { ks.extend(a.entries().iter().map(|(k, _)| k.clone()).filter(|k| k.starts_with(pre) && k.ends_with("_id"))); }
        }
        let mut fields = vec![format!("attack: {}", compile::quote(label))];
        fields.extend(ks.iter().filter(|k| compile::ident(k) == **k).filter_map(|k| a.get(k).and_then(plain).filter(|v| v != "null").map(|v| format!("{k}: {v}"))));
        let mut per: BTreeMap<u32, String> = BTreeMap::new();
        for (bn, b) in a.get("attackBoxes").map(|b| b.entries().to_vec()).unwrap_or_default() {
            let i = if bn == "attackBox" { 1 } else { match bn.strip_prefix("attackBox").and_then(|n| n.parse::<u32>().ok()) { Some(n) => n, None => continue } };
            let st: Vec<String> = box_keys.iter().filter_map(|k| b.get(k).and_then(plain).filter(|v| v != "null").map(|v| format!("{k}: {v}"))).collect();
            if !st.is_empty() { per.insert(i, format!("{{ {} }}", st.join(", "))); }
        }
        if let Some(last) = per.keys().max().copied() {
            let list: Vec<String> = (1..=last).map(|i| per.get(&i).cloned().unwrap_or_else(|| "null".into())).collect();
            fields.push(format!("boxes: [{}]", list.join(", ")));
        }
        for anim in anims {
            let own = fields_of.remove(anim.trim_matches('"')).unwrap_or_default();
            let all: Vec<String> = own.into_iter().chain(fields.iter().cloned()).collect();
            meta.insert(anim.trim_matches('"').to_string(), format!("{{ {} }}", all.join(", ")));
        }
    }
    for (anim, fs) in fields_of { if !fs.is_empty() { meta.insert(anim, format!("{{ {} }}", fs.join(", "))); } }
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
    // (a clip class with nothing to run here, whose frames only set its fields to plain values: each
    // field is what the frame of the label it shows set it to, as another object's code reads it)
    if let Some(rc) = f.model.class_of(parts.root).filter(|c| !w.field_ids.keys().any(|(k, _)| k == &c.name)) {
        let set = root_fields(f.model, rc, parts.root);
        if !set.is_empty() {
            field_cases.push(format!("case {}:", compile::quote(&rc.name)));
            field_cases.push("\tswitch (name) {".into());
            for (field, by_label) in set {
                // (a field each frame sets to its own label is the label)
                let labels: BTreeSet<&str> = f.model.sprites.get(&parts.root).map(|sp| sp.labels.iter().map(|(_, l)| l.as_str()).collect()).unwrap_or_default();
                if by_label.iter().all(|(l, v)| *v == compile::quote(l)) && labels.iter().all(|l| by_label.iter().any(|(b, _)| b == l)) {
                    field_cases.push(format!("\t\tcase {}: return {{ get: function() {{ return rootLabel(); }} }};", compile::quote(&field)));
                    continue;
                }
                let cases: Vec<String> = by_label.iter().map(|(l, v)| format!("case {}: return {v};", compile::quote(l))).collect();
                field_cases.push(format!("\t\tcase {}: return {{ get: function() {{ switch (rootLabel()) {{ {} }} return null; }} }};", compile::quote(&field), cases.join(" ")));
            }
            field_cases.push("\t}".into());
        }
    }
    let itself = parts.itself.map(|c| compile::quote(&c.name)).unwrap_or_else(|| "null".into());
    let root = f.model.class_of(parts.root).map(|c| compile::quote(&c.name)).unwrap_or_else(|| "null".into());
    functions.push(native::helper(rt, "fighter_tables", "field_of", "", &[("ITSELF", itself), ("ROOT", root)], &field_cases).map_err(one)?);
    functions.push(table("root_label_meta", &[])?);
    if blocks.contains("items") { functions.extend(item_tables(f).map_err(|e| vec![e])?); }
    // (the label its clip is on, for its own code and for another object's that reads it)
    let read_by_others = native::reads_others_label(f.model);
    if read_by_others || code_text.contains("clipLabel(") || functions.iter().any(|t| t.contains("clipLabel(")) { functions.push(table("clip_label_meta", &[])?); }
    if !stat_code.is_empty() {
        functions.push(format!("// ---- what its attacks' declared stats and the source's own states have it do: runtime/attack_stats.jsonc, runtime/engine_animations.jsonc\n{code_text}"));
    }

    let mut slots: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    slots.insert("fields", w.fields.clone());
    let mut functions_extra: Vec<String> = Vec::new();
    let mut publish = w.publish.clone();
    publish.push("// what another object's code asks of it: an attack started, a rocket ridden, its clips' fields".into());
    publish.push("self.exports.__forceAttack = forceAttackAsked;".into());
    if rocket.is_some() { publish.push("self.exports.__rocket = rocketAsked;".into()); }
    publish.push("self.exports.__fieldOf = fieldOf;".into());
    publish.push("self.exports.__attackStat = attackStat;".into());
    publish.push("self.exports.__attackMeta = attackMeta;".into());
    publish.push("self.exports.__label = rootLabel;".into());
    publish.push("self.exports.__moveCode = moveCode;".into());
    // (a fighter that takes powers has the one it holds make the items it does not declare)
    if blocks.contains("copy_power") { publish.push("self.exports.__powerItem = powerItem;".into()); }
    // the power it gives a fighter that swallows it, where it declares one
    if let Some(power) = power_of(f) {
        let mut cases = Vec::new();
        for (name, decl) in f.fighter.projectiles.entries() {
            let Some(key) = f.entities.get(name) else { continue };
            let content = crate::projectile_gen::projectile_content_id(key);
            let n = |ks: &[&str]| ks.iter().find_map(|k| match decl.get(k) { Some(Value::Num(v)) => Some(*v), _ => None }).unwrap_or(0.0);
            let [dx, dy, xs, ys] = [n(&["xoffset", "xOffset", "x_offset"]), n(&["yoffset", "yOffset", "y_offset"]), n(&["xspeed", "xSpeed", "x_speed"]), n(&["yspeed", "ySpeed", "y_speed"])];
            cases.push(format!("case {}: return {{ content: {}, dx: {}, dy: {}, xs: {}, ys: {} }};", compile::quote(name), compile::quote(&content),
                num(dx), num(dy), num(xs * f.units.speed), num(ys * f.units.speed)));
        }
        functions_extra.push(native::helper(f.runtime, "fighter_rules", "kirby_power", "", &[("POWER", compile::quote(&power)), ("SIZE", num(f.units.size))], &cases).map_err(|e| vec![e])?);
        // (and its effects, the clips inside them, and its items, where it has them)
        let written: String = functions.join("\n");
        let has = |fun: &str| written.contains(&format!("function {fun}("));
        let mut record = vec![format!("id: {}", compile::quote(&power)), "fire: firePower".to_string()];
        if has("namedEffect") { record.push("effect: namedEffect".into()); }
        if has("innerClip") && has("effectFrame") {
            functions_extra.push(native::helper(f.runtime, "fighter_rules", "kirby_power_inner", "", &[], &[]).map_err(|e| vec![e])?);
            record.push("inner: powerInnerFor".into());
        }
        if has("itemContent") {
            functions_extra.push(native::helper(f.runtime, "fighter_rules", "kirby_power_item", "", &[], &[]).map_err(|e| vec![e])?);
            record.push("item: powerItemFor".into());
        }
        publish.push(format!("self.exports.__kirbyPower = {{ {} }};", record.join(", ")));
    }
    if read_by_others { publish.push("self.exports.__stanceLabel = clipLabel;".into()); }
    slots.insert("publish", publish);
    functions.extend(functions_extra);
    slots.insert("functions", functions.join("\n\n").lines().map(String::from).collect());
    slots.insert("start", vec!["listenForHits();".into()]);
    slots.insert("own_start", w.start.clone());
    let mut update = vec!["hitFrame();".to_string(), "attackFrame();".into()];
    if blocks.contains("last_used") { update.push("usedFrame();".into()); }
    if blocks.contains("own_floor") { update.push("ownFloorFrame();".into()); }
    if blocks.contains("items") { update.push("itemFrame();".into()); }
    if blocks.contains("copy_power") { update.push("powerFrame();".into()); }
    // a clip its animations show, made anew as the fighter comes to it: its fields, its frame code's
    // own variables, declared as it starts
    let entries = clip_entries(f, &w);
    if entries.iter().any(|e| e.guard.is_some()) || blocks.contains("fresh_clip") {
        let block = native::helper(rt, "fighter_rules", "fresh_clip", "", &[], &[]).map_err(one)?;
        if let Some(fs) = slots.get_mut("functions") { fs.push(String::new()); fs.extend(block.lines().map(String::from)); }
        update.push("shownFrame();".into());
    }
    // its clip playing on past an animation's last frame into the one that shows its next frame
    // (a charge's base into its attack), where the engine would end the move
    let mut on: Vec<String> = f.pieces.iter().filter_map(|p| p.on.as_ref().map(|q| format!("case {}: self.playAnimation({});", compile::quote(&p.fm_name), compile::quote(q)))).collect();
    on.dedup();
    if !on.is_empty() {
        let block = native::helper(rt, "helpers", "plays_on", "playOn", &[], &on).map_err(one)?;
        if let Some(fs) = slots.get_mut("functions") { fs.push(String::new()); fs.extend(block.lines().map(String::from)); }
        update.push("playOn();".into());
    }
    // (last: the end of the frame, after everything else its update does)
    let tick_end = blocks.contains("tick_end");
    if blocks.contains("frame_links") {
        let block = native::helper(rt, "fighter_rules", "frame_links", "", &[], &[]).map_err(one)?;
        if let Some(fs) = slots.get_mut("functions") { fs.push(String::new()); fs.extend(block.lines().map(String::from)); }
        if let Some(st) = slots.get_mut("start") { st.push("listenForLinks();".into()); }
        update.push("linkFrame();".into());
    }
    update.extend(w.every_frame.iter().cloned());
    if tick_end { update.push("tickEndFrame();".into()); }
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
    if let Some(n) = native::declared_twice(&script) { if let Ok(p) = std::env::var("PEPTIDE_DUMP_TWICE") { let _ = std::fs::write(p, &script); } return Err(vec![format!("its script declares {n} twice")]) }

    // its entity: each frame with code calls it, and what each animation's attack declares runs on its first
    let entity = char_dir.join(format!("library/entities/{char_pascal}.entity"));
    let mut frames = w.frames.clone();
    for code in frames.values_mut() { *code = code.replace(native::FRAME_RETURN, "return;"); }
    walk_starts(f, &mut frames);
    // (an animation its code jumps into at a label: its first frame's own code waits that out,
    // its clip's declarations do not)
    for p in f.pieces.iter().filter(|p| w.mid_targets.contains(&p.fm_name) && !p.reversed && p.head == 0) {
        let at = (p.sym, p.from);
        if let Some(code) = frames.get_mut(&at).filter(|c| !c.trim().is_empty()) {
            *code = format!("if (!midJump.get()) {{\n{}\n}}", code.lines().map(|l| format!("\t{l}")).collect::<Vec<_>>().join("\n"));
        }
    }
    for e in &entries {
        let decl = match &e.guard {
            None => e.decls.join("\n"),
            Some(anims) => {
                let test: Vec<String> = anims.iter().map(|a| format!("shownBefore.get() != {}", compile::quote(a))).collect();
                format!("if ({}) {{\n{}\n}}", test.join(" && "), e.decls.iter().map(|d| format!("\t{d}")).collect::<Vec<_>>().join("\n"))
            }
        };
        let code = frames.entry((e.sym, e.frame)).or_default();
        // (a block's close followed by a line opening with `(` reads as a call of the block)
        let sep = if decl.trim_end().ends_with('}') && super::native_projectile::opens_with_paren(code) { ";\n" } else { "\n" };
        *code = if code.is_empty() { decl } else { format!("{decl}{sep}{code}") };
    }
    let mut extra: BTreeMap<(String, u32), String> = BTreeMap::new();
    let field_ref = crate::re!(r"\{field:(\w+)\}");
    for (anim, code) in first_frame {
        // (a field of the clip the animation shows: its container, or none where its code keeps none)
        let class = f.pieces.iter().find(|p| &p.fm_name == anim).and_then(|p| f.model.class_of(p.sym)).map(|c| c.name.clone()).unwrap_or_default();
        let code = field_ref.replace_all(code, |c: &regex::Captures| w.field_ids.get(&(class.clone(), c[1].to_string())).cloned().unwrap_or_else(|| "null".into())).to_string();
        extra.insert((anim.clone(), 0), code);
    }
    // (its content is found in its own files even while its code acts as another fighter: `act_as`)
    // (a sound named outright is the engine's play, unless an attack of its stops its sounds as it
    // ends, which needs the one it played last)
    let stops = ["cancelSoundOnEnd", "cancelVoiceOnEnd"].iter().any(|k| script.contains(&format!("case \"{k}\"")));
    let named_sound = regex::Regex::new(r#"playSoundOf\(("[^"\\]*"), (?:true|false)\)"#).expect("sound pattern");
    // (a stat of the attack it is in is read off the attack's metadata: named outright, the field;
    // numbered as it runs, the declared one of that number)
    let stat_named = regex::Regex::new(r#"attackStat\("([A-Za-z_][A-Za-z0-9_]*)"\)"#).expect("stat pattern");
    let stat_numbered = regex::Regex::new(r#"attackStat\("(attackSound|attackVoice)" \+ (Std\.int\(Math\.max\(1, (?:[^()]|\([^()]*\))*\)\)) \+ "_id"\)"#).expect("numbered pattern");
    let numbered = |pre: &str| -> Vec<u32> {
        let mut n: BTreeSet<u32> = BTreeSet::new();
        for (_, a) in attacks.entries() { for (k, _) in a.entries() { if let Some(i) = k.strip_prefix(pre).and_then(|r| r.strip_suffix("_id")).and_then(|r| r.parse().ok()) { n.insert(i); } } }
        n.into_iter().collect()
    };
    // (an attack turned off with no count: back on after what it declares, as read here)
    let reenable = regex::Regex::new(r#"allowAttack\(([^;]*?), "([^"]*)" /\*reenable\*/\)"#).expect("reenable pattern");
    let by_label = regex::Regex::new(r#"(allowAttack\(|attackAllowed\()([^;"]*?)"([^"]*)""#).expect("label pattern");
    let own_files = |c: &str| {
        let c = c.replace("self.getResource()", "ownResource");
        let c = stat_named.replace_all(&c, "attackStat(\"$1\", attackMeta().$1)").into_owned();
        let c = stat_numbered.replace_all(&c, |m: &regex::Captures| {
            let n = &m[2];
            let chain: String = numbered(&m[1]).iter().rev().fold("null".to_string(), |rest, i| format!("({n} == {i} ? attackMeta().{}{i}_id : {rest})", &m[1]));
            format!("attackStat(\"{}\" + {n} + \"_id\", {chain})", &m[1])
        }).into_owned();
        let c = reenable.replace_all(&c, |m: &regex::Captures| {
            let d = attacks.get(&m[2]).and_then(|a| a.get("reenableTimer")).and_then(|v| match v { Value::Num(n) if *n > 0.0 => Some(num(*n)), _ => None });
            match d { Some(d) => format!("allowAttack({}, \"{}\", {d})", &m[1], &m[2]), None => format!("allowAttack({}, \"{}\")", &m[1], &m[2]) }
        }).into_owned();
        // (an attack turned off or asked of by its label: the label's animations, written in)
        let c = by_label.replace_all(&c, |m: &regex::Captures| {
            // (by the source's name for the move, else the clip label its animations show)
            let mut a: Vec<String> = f.anim_of.get(&m[3]).map(|x| compile::quote(x)).into_iter()
                .chain(f.pieces.iter().filter(|p| p.label == m[3]).map(|p| compile::quote(&p.fm_name))).collect();
            a.dedup();
            format!("{}{}[{}]", &m[1], &m[2], a.join(", "))
        }).into_owned();
        if stops { c } else { named_sound.replace_all(&c, "AudioClip.play(ownResource.getContent($1))").into_owned() }
    };
    for code in frames.values_mut() { *code = own_files(code); }
    for code in extra.values_mut() { *code = own_files(code); }
    super::native_projectile::keyframes_with(&entity, title, f.model, f.pieces, &frames, false, &extra).map_err(|e| vec![e.to_string()])?;
    if blocks.contains("items") { item_box_layers(&entity, title, f.pieces, f.units.size).map_err(|e| vec![e.to_string()])?; }
    let act_as = native::helper(rt, "fighter_rules", "act_as", "", &[], &move_code(&entity)).map_err(one)?;
    let script = own_files(&script);
    // (its own files, first thing after its header, before any of its code reads them)
    let head = script.lines().take_while(|l| l.starts_with("//")).map(|l| l.len() + 1).sum::<usize>().min(script.len());
    let script = format!("{}// the files its content is found in, whoever its code is acting as\nvar ownResource = self.getResource();\n\n{}\n\n// ---- act_as: runtime/native/fighter_rules.hx\n{act_as}\n", &script[..head], &script[head..]);
    let mut gaps = w.gaps.clone();
    if blocks.contains("copy_power") && copy_move(f).is_none() { gaps.insert("takes powers, but no clip of its says where it swallows (its code writes no KirbyPower in a frame)".into()); }
    Ok((script, gaps, meta))
}

/// the label its clip is on (the last one at or before the frame it shows), as the source reads it
pub fn clip_labels(model: &SourceModel, pieces: &[Piece]) -> Vec<String> {
    let mut t = Vec::new();
    for p in pieces.iter().filter(|p| !p.reversed && p.head == 0) {
        let Some(sprite) = model.sprites.get(&p.sym) else { continue };
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

/// the tables its items block reads: where its item box is on each frame of each animation, and the
/// content each item it can make is
fn item_tables(f: &Fighting) -> Result<Vec<String>, String> {
    let rt = f.runtime;
    let mut content = Vec::new();
    let mut names = Vec::new();
    for (name, _) in f.fighter.items.entries() {
        if let Some(key) = f.entities.get(name) {
            content.push(format!("case {}: return {};", compile::quote(name), compile::quote(&crate::projectile_gen::projectile_content_id(key))));
            names.push(compile::quote(name));
        }
    }
    Ok(vec![native::helper(rt, "fighter_tables", "item_box", "", &[("SIZE", num(f.units.size))], &[])?,
            native::helper(rt, "fighter_tables", "item_content", "", &[("NAMES", names.join(", "))], &content)?])
}

fn num(n: f64) -> String { if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") } }

/// the move a fighter that takes powers swallows with, as its own clip states it: the clip whose
/// frame code takes the power (`KirbyPower = this.power`) is the move (each animation showing it,
/// by its label); the label over that frame is where it swallows, the label over the frame that
/// lets the one it holds out (`ejectOpponent`) where it spits, the `xframe` its first frame sets
/// what it is while it draws one in, and the field the power is read from is the one the source's
/// game hands it the swallowed one's power in
pub struct CopyMove { pub moves: Vec<String>, pub swallow: String, pub spit: Option<String>, pub sucking: Option<String>, pub field: String }

/// the moves a fighter that copies another runs the code of (the ones `copy_power` rehearses)
const COPIED_MOVES: [&str; 2] = ["special_neutral", "special_neutral_air"];

/// the code each frame of its copied moves runs, read back off its entity, as the cases of one
/// switch by animation and frame (a frame from 1, as the engine counts it)
fn move_code(entity: &Path) -> Vec<String> {
    let json: serde_json::Value = std::fs::read(entity).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let keys: BTreeMap<&str, &serde_json::Value> = json["keyframes"].as_array().map(|a| a.iter().filter_map(|k| Some((k["$id"].as_str()?, k))).collect()).unwrap_or_default();
    let layers: BTreeMap<&str, &serde_json::Value> = json["layers"].as_array().map(|a| a.iter().filter_map(|l| Some((l["$id"].as_str()?, l))).collect()).unwrap_or_default();
    let mut cases = Vec::new();
    for anim in json["animations"].as_array().into_iter().flatten() {
        let Some(name) = anim["name"].as_str().filter(|n| COPIED_MOVES.contains(n)) else { continue };
        for l in anim["layers"].as_array().into_iter().flatten().filter_map(|id| layers.get(id.as_str()?)).filter(|l| l["type"] == "FRAME_SCRIPT") {
            let mut at = 0;
            for k in l["keyframes"].as_array().into_iter().flatten().filter_map(|id| keys.get(id.as_str()?)) {
                if let Some(code) = k["code"].as_str().filter(|c| !c.trim().is_empty()) {
                    cases.push(format!("\tcase {}:", compile::quote(&format!("{name}#{}", at + 1))));
                    cases.extend(code.lines().map(|c| format!("\t\t{c}")));
                }
                at += k["length"].as_u64().unwrap_or(1);
            }
        }
    }
    if cases.is_empty() { return cases }
    let mut out = vec!["switch (anim + \"#\" + frame) {".to_string()];
    out.extend(cases);
    out.push("}".into());
    out
}

/// the frame each animation of the entity first shows a hitbox on (from 1), as a script list of
/// [animation, frame] pairs: where a pose of its own lands its blow
fn strike_frames(entity: &Path) -> String {
    let json: serde_json::Value = std::fs::read(entity).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let keys: BTreeMap<&str, &serde_json::Value> = json["keyframes"].as_array().map(|a| a.iter().filter_map(|k| Some((k["$id"].as_str()?, k))).collect()).unwrap_or_default();
    let layers: BTreeMap<&str, &serde_json::Value> = json["layers"].as_array().map(|a| a.iter().filter_map(|l| Some((l["$id"].as_str()?, l))).collect()).unwrap_or_default();
    let mut out = Vec::new();
    for anim in json["animations"].as_array().into_iter().flatten() {
        let Some(name) = anim["name"].as_str() else { continue };
        let first = anim["layers"].as_array().into_iter().flatten().filter_map(|id| layers.get(id.as_str()?))
            .filter(|l| l["name"].as_str().is_some_and(|n| n.starts_with("hitbox")))
            .filter_map(|l| {
                let mut at = 0;
                for k in l["keyframes"].as_array()?.iter().filter_map(|id| keys.get(id.as_str()?)) {
                    if k["symbol"].is_string() { return Some(at + 1) }
                    at += k["length"].as_u64().unwrap_or(1);
                }
                None
            }).min();
        if let Some(n) = first { out.push(format!("[{}, {n}]", compile::quote(name))); }
    }
    format!("[{}]", out.join(", "))
}

pub fn copy_move(f: &Fighting) -> Option<CopyMove> {
    use crate::model::ast::{Expr, Place, Stmt};
    for c in f.model.classes.values() {
        let Some(sym) = c.symbol else { continue };
        let labels = f.model.sprites.get(&sym).map(|sp| sp.labels.clone()).unwrap_or_default();
        let label_at = |frame: u32| labels.iter().filter(|(f0, _)| *f0 as u32 <= frame).max_by_key(|(f0, _)| *f0).map(|(_, l)| l.clone());
        let body_of = |m: &str| c.methods.iter().find(|x| x.name == m).map(|x| &x.function.body);
        let mut take: Option<(u32, String)> = None;
        let mut spit = None;
        let mut sucking = None;
        for fs in &c.frame_scripts {
            let Some(body) = body_of(&fs.method) else { continue };
            crate::compile::native::walk(body, &mut |st| match st {
                Stmt::Assign(Place::Member(_, n), Expr::Member(_, v)) if n == "KirbyPower" && take.is_none() => take = Some((fs.frame, v.clone())),
                Stmt::Assign(Place::Member(t, n), Expr::Str(v)) if fs.frame == 0 && n == "xframe" && matches!(t, Expr::This) => sucking = Some(v.clone()),
                _ => {}
            });
            crate::model::ast::walk_exprs(body, &mut |e| if let Expr::Call(g, _) = e {
                if matches!(&**g, Expr::Member(_, m) if matches!(m.as_str(), "ejectOpponent" | "shootOutOpponent")) && spit.is_none() { spit = label_at(fs.frame); }
            });
        }
        let Some((frame, field)) = take else { continue };
        let swallow = label_at(frame)?;
        let mut moves: Vec<String> = Vec::new();
        for p in f.pieces.iter().filter(|p| p.sym == sym && !p.label.is_empty()) { if !moves.contains(&p.label) { moves.push(p.label.clone()); } }
        if moves.is_empty() { continue }
        return Some(CopyMove { moves, swallow, spit, sucking, field });
    }
    None
}

/// the power a fighter gives one that swallows it: the name its stats declare (`power`)
fn power_of(f: &Fighting) -> Option<String> {
    f.fighter.stats.get("power").and_then(|v| v.str()).filter(|p| !p.is_empty()).map(String::from)
}

/// what each labelled frame of a clip's code sets its own fields to, where it sets a plain value:
/// field -> (label, value as written)
fn root_fields(model: &SourceModel, class: &crate::model::ClassModel, sym: u16) -> BTreeMap<String, Vec<(String, String)>> {
    use crate::model::ast::{Expr, Place, Stmt};
    let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let Some(sprite) = model.sprites.get(&sym) else { return out };
    for (frame, label) in &sprite.labels {
        let Some(fs) = class.frame_scripts.iter().find(|fs| fs.frame as usize == *frame) else { continue };
        let Some(m) = class.methods.iter().find(|m| m.name == fs.method) else { continue };
        for st in &m.function.body {
            let Stmt::Assign(Place::Member(t, name), v) = st else { continue };
            if !matches!(t, Expr::This) { continue }
            let v = match v { Expr::Str(x) => compile::quote(x), Expr::Num(n) => n.to_string(), Expr::Int(n) => n.to_string(), Expr::Bool(b) => b.to_string(), Expr::Null => "null".into(), _ => continue };
            out.entry(name.clone()).or_default().push((label.clone(), v));
        }
    }
    out
}

/// where a clip its animations show starts, and what its frame code declares there
struct Entry { sym: u16, frame: u32, decls: Vec<String>, guard: Option<Vec<String>> }

/// each clip class's fields, declared by its frame code where the fighter comes to the clip: on the
/// first frame of each animation showing it. plainly where one animation shows the whole clip from its
/// first frame, as an attack, and its code never jumps it; else unless the fighter was showing that
/// clip already (a loop, or a jump between its labels' animations, keeps the clip it has)
fn clip_entries(f: &Fighting, w: &native::Written) -> Vec<Entry> {
    use crate::model::ast::Expr;
    let mut out = Vec::new();
    for (class, decls) in &w.frame_fields {
        let Some(c) = f.model.classes.get(class) else { continue };
        let Some(sym) = c.symbol else { continue };
        let pieces: Vec<&super::Piece> = f.pieces.iter().filter(|p| p.sym == sym).collect();
        if pieces.is_empty() { continue }
        let mut jumps = false;
        for m in &c.methods {
            crate::model::ast::walk_exprs(&m.function.body, &mut |e| if let Expr::Call(g, _) = e {
                if matches!(&**g, Expr::Member(_, n) | Expr::Name(n) if matches!(n.as_str(), "gotoAndPlay" | "gotoAndStop" | "nextFrame" | "prevFrame" | "play")) { jumps = true }
            });
        }
        let one = pieces.len() == 1 && { let p = pieces[0]; p.from == 1 && !p.reversed && p.head == 0 && f.fighter.attacks.get(&p.label).is_some() };
        let mut anims: Vec<String> = pieces.iter().map(|p| p.fm_name.clone()).collect();
        anims.sort();
        anims.dedup();
        let mut at: BTreeSet<u32> = BTreeSet::new();
        for p in &pieces { at.insert(if p.reversed { p.from + p.frames - 1 } else { p.from }); }
        for frame in at {
            out.push(Entry { sym, frame, decls: decls.clone(), guard: if one && !jumps { None } else { Some(anims.clone()) } });
        }
    }
    out
}

/// a zero x speed its frame code writes: in an animation it walks or runs in, the walk's own
/// starting speed (its declared accel_start of norm_xSpeed), as the source's walk applies it after
/// the code; anywhere else, zero
fn walk_starts(f: &Fighting, frames: &mut BTreeMap<(u16, u32), String>) {
    let walks: BTreeSet<&str> = crate::mappings::character_animation_template().iter().filter(|t| t.walks).map(|t| t.name.as_str()).collect();
    let stat = |k: &str| match f.fighter.stats.get(k) { Some(Value::Num(n)) => *n, _ => 0.0 };
    let start = num(stat("accel_start") * stat("norm_xSpeed") * f.units.speed);
    let pattern = regex::Regex::new(r"self\.setXSpeed\(([^;]*?) /\*walk\*/\)").expect("walk pattern");
    for ((sym, frame), code) in frames.iter_mut() {
        if !code.contains("/*walk*/") { continue }
        let walking = f.pieces.iter().any(|p| p.sym == *sym && *frame >= p.from && *frame < p.from + p.frames && walks.contains(p.fm_name.as_str()));
        *code = pattern.replace_all(code, |c: &regex::Captures| {
            let v = &c[1];
            if !walking { format!("self.setXSpeed({v})") }
            else if v.parse::<f64>().is_ok_and(|n| n == 0.0) { format!("self.setXSpeed({start})") }
            else { format!("self.setXSpeed({v} == 0 ? {start} : {v})") }
        }).into_owned();
    }
}

/// its animations' stats with their attack metadata set: the last pass over the file, as the
/// others rewrite its lines field by field
/// (`meta`: each animation's metadata as `write` worked it out, the animation's name to its literal)
pub fn write_attack_meta(stats: &std::path::Path, meta: &BTreeMap<String, String>) -> anyhow::Result<()> {
    if meta.is_empty() { return Ok(()) }
    let Ok(text) = std::fs::read_to_string(stats) else { return Ok(()) };
    let out: Vec<String> = text.lines().map(|l| {
        let name = l.trim_start().split(':').next().unwrap_or("").trim();
        let Some(m) = meta.get(name).filter(|_| l.starts_with('\t') && !l.starts_with("\t\t")) else { return l.to_string() };
        let Some(c) = l.rfind('}') else { return l.to_string() };
        let body = l[..c].trim_end();
        let sep = if body.ends_with('{') { " " } else { ", " };
        format!("{body}{sep}metadata: {m} {}", &l[c..])
    }).collect();
    std::fs::write(stats, out.join("\n") + "\n")?;
    Ok(())
}

/// where it holds an item, frame by frame, as the engine's own box: a custom box (`CUSTOM_BOX_A`)
/// on each animation that places one, a box a pixel each way around the source's item box from
/// the frame it moves to on (`getCollisionBoxes(CollisionBoxType.CUSTOMA)`)
fn item_box_layers(entity: &std::path::Path, key: &str, pieces: &[Piece], size: f64) -> anyhow::Result<()> {
    let Ok(bytes) = std::fs::read(entity) else { return Ok(()) };
    let mut json: serde_json::Value = serde_json::from_slice(&bytes)?;
    let id = |s: String| crate::uuid_gen::det_uuid(&s);
    let lens: BTreeMap<String, u64> = json["keyframes"].as_array().map(|a| a.iter().filter_map(|k| Some((k["$id"].as_str()?.to_string(), k["length"].as_u64().unwrap_or(1)))).collect()).unwrap_or_default();
    let layer_len: BTreeMap<String, u64> = json["layers"].as_array().map(|a| a.iter().filter_map(|l| {
        Some((l["$id"].as_str()?.to_string(), l["keyframes"].as_array()?.iter().filter_map(|k| lens.get(k.as_str()?)).sum()))
    }).collect()).unwrap_or_default();
    let (mut keyframes, mut symbols, mut layers) = (Vec::new(), Vec::new(), Vec::new());
    let mut added: BTreeMap<String, String> = BTreeMap::new();
    for p in pieces.iter().filter(|p| !p.item_box.is_empty()) {
        if added.contains_key(&p.fm_name) { continue }
        let Some(anim) = json["animations"].as_array().and_then(|a| a.iter().find(|x| x["name"] == p.fm_name.as_str())) else { continue };
        let total = anim["layers"].as_array().map(|ls| ls.iter().filter_map(|l| layer_len.get(l.as_str()?)).copied().max().unwrap_or(0)).unwrap_or(0);
        if total == 0 { continue }
        let tag = format!("{key}::{}::itembox", p.fm_name);
        let mut ids = Vec::new();
        for (n, (k, x, y)) in p.item_box.iter().enumerate() {
            let from = if n == 0 { 0 } else { (2 * k) as u64 };
            let to = p.item_box.get(n + 1).map(|(k, _, _)| (2 * k) as u64).unwrap_or(total).min(total);
            if to <= from { continue }
            let sym = id(format!("{tag}::sym{n}"));
            symbols.push(serde_json::json!({ "$id": sym, "alpha": 0.5, "color": "0x00ff00", "pivotX": 1.0, "pivotY": 1.0, "pluginMetadata": {},
                "rotation": 0.0, "scaleX": 2.0, "scaleY": 2.0, "type": "COLLISION_BOX", "x": x * size - 1.0, "y": y * size - 1.0 }));
            let k = id(format!("{tag}::{n}"));
            keyframes.push(serde_json::json!({ "$id": k, "length": to - from, "pluginMetadata": {}, "symbol": sym, "tweenType": "LINEAR", "tweened": false, "type": "COLLISION_BOX" }));
            ids.push(k);
        }
        if ids.is_empty() { continue }
        let layer = id(format!("{tag}::layer"));
        layers.push(serde_json::json!({ "$id": layer, "defaultAlpha": 0.5, "defaultColor": "0x00ff00", "hidden": false, "locked": false, "name": "customboxa0",
            "pluginMetadata": { "com.fraymakers.FraymakersMetadata": { "collisionBoxType": "CUSTOM_BOX_A", "index": 0 } }, "type": "COLLISION_BOX", "keyframes": ids }));
        added.insert(p.fm_name.clone(), layer);
    }
    if added.is_empty() { return Ok(()) }
    if let Some(a) = json["keyframes"].as_array_mut() { a.extend(keyframes); }
    if let Some(a) = json["symbols"].as_array_mut() { a.extend(symbols); }
    if let Some(a) = json["layers"].as_array_mut() { a.extend(layers); }
    if let Some(anims) = json["animations"].as_array_mut() {
        for a in anims.iter_mut() {
            let Some(l) = a["name"].as_str().and_then(|n| added.get(n)).cloned() else { continue };
            if let Some(ls) = a["layers"].as_array_mut() { ls.push(serde_json::Value::String(l)); }
        }
    }
    std::fs::write(entity, serde_json::to_string_pretty(&json)?)?;
    Ok(())
}

/// the label a piece's clip is on as it starts (the last at or before its first frame) and the ones
/// inside it, by the piece's own frame; none for a piece shown backwards or after a head
fn clip_label_data(model: &SourceModel, p: &Piece) -> Option<(Option<String>, Vec<(u32, String)>)> {
    if p.reversed || p.head != 0 { return None }
    let sprite = model.sprites.get(&p.sym)?;
    let mut labels: Vec<(u32, &str)> = sprite.labels.iter().map(|(f0, l)| (*f0 as u32 + 1, l.as_str())).collect();
    labels.sort();
    let before = labels.iter().rfind(|(fr, _)| *fr <= p.from).map(|(_, l)| l.to_string());
    let inside: Vec<(u32, String)> = labels.iter().filter(|(fr, _)| *fr > p.from && *fr < p.from + p.frames).map(|(fr, l)| (fr - p.from, l.to_string())).collect();
    if before.is_none() && inside.is_empty() { return None }
    Some((before, inside))
}

