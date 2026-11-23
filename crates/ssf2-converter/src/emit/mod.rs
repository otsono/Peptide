//! The object writer's shared parts: a fighter's script and its projectiles' and items', the pieces
//! (animations) each one's clip is cut into, and the tables its declared stats give the engine.
//!
//! Nothing here knows what kind of object it writes: a fighter is its declared class, its clip,
//! and the classes and clips they reach. Each object's code is written by `compile::native`.

pub mod native_fighter;
pub mod native_projectile;
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
/// the labels of every clip an object's animations show, as one table: the jumps its own class
/// makes, which play its clip (or the stance its clip places) by label
pub fn own_jumps(model: &SourceModel, pieces: &[Piece], root: Option<u16>) -> crate::compile::ClipJumps {
    let own: Vec<&Piece> = pieces.iter().filter(|p| !p.borrowed && !p.reversed && p.head == 0).collect();
    let mut j = crate::compile::ClipJumps { anims: own.iter().map(|p| p.fm_name.clone()).collect(), labels: BTreeMap::new(), autolink: None, declared: BTreeMap::new(), sounds: BTreeMap::new(), freeze_unstated: (false, false) };
    // (its clip's own labels first: each one plays the stance it places there from its start, or the
    // clip itself from the label)
    if let Some(sp) = root.and_then(|r| model.sprites.get(&r)) {
        for (f0, label) in &sp.labels {
            let stance = sp.frames.get(*f0).and_then(|placed| placed.iter().find(|p| p.name.as_deref() == Some("stance")).map(|p| p.char_id));
            let at = match stance {
                Some(st) => own.iter().filter(|p| p.sym == st).min_by_key(|p| p.from),
                None => own.iter().find(|p| Some(p.sym) == root && p.from == *f0 as u32 + 1),
            };
            if let Some(p) = at { j.labels.insert(label.clone(), (p.fm_name.clone(), true)); }
        }
    }
    let syms: BTreeSet<u16> = own.iter().map(|p| p.sym).collect();
    for sym in syms {
        let Some(sprite) = model.sprites.get(&sym) else { continue };
        for (f0, label) in &sprite.labels {
            if j.labels.contains_key(label) { continue }
            let f = *f0 as u32 + 1;
            let hits: Vec<&&Piece> = own.iter().filter(|p| p.sym == sym && f >= p.from && f < p.from + p.frames).collect();
            let at = if let [p] = hits.as_slice() { Some(*p) } else { hits.iter().find(|p| p.from == f).copied() };
            // (a label whose frames place a clip of its own, played whole as its own animation, where
            // the clip's own timeline holds it for fewer frames than it runs: that animation)
            let window = at.map(|p| if f == p.from { p.frames } else { p.frames.saturating_sub(f - p.from) }).unwrap_or(0);
            let state = pieces.iter().find(|p| p.sym != sym && p.label == *label && p.from == 1 && !p.borrowed && p.frames > window);
            if let Some(p) = state { j.labels.insert(label.clone(), (p.fm_name.clone(), true)); continue }
            if let Some(p) = at { j.labels.insert(label.clone(), (p.fm_name.clone(), f == p.from)); }
        }
    }
    j
}

thread_local! {
    /// the objects a fighter's code holds over the screen (its final smash cutscene's clip), by
    /// the name discovery gave them
    static SCREEN_OBJECTS: std::cell::RefCell<BTreeSet<String>> = Default::default();
}
thread_local! {
    /// the named children of each object's clip its code turns, drawn apart (`projectile_gen`
    /// `collect_parts`), by the object's content id
    static OBJECT_PARTS: std::cell::RefCell<BTreeMap<String, Vec<ObjPart>>> = Default::default();
}
/// a named child of an object's clip its code turns: drawn apart from the object's art, as the
/// animation `anim` of the object's entity, where the clip places it (the source's units), with its
/// timeline's length and labels
#[derive(Clone, Debug)]
pub struct ObjPart { pub path: String, pub anim: String, pub at: (f64, f64, f64, f64, f64), pub frames: u16, pub labels: Vec<(String, u16)> }
/// note an object's parts, as its art was extracted
pub fn add_object_parts(name: &str, parts: Vec<ObjPart>) {
    OBJECT_PARTS.with(|c| { c.borrow_mut().insert(crate::projectile_gen::projectile_content_id(name), parts); });
}
/// forget every object's parts (a conversion starting)
pub fn clear_object_parts() { OBJECT_PARTS.with(|c| c.borrow_mut().clear()); }
/// an object's parts, by its entity key
pub fn object_parts(key: &str) -> Vec<ObjPart> {
    OBJECT_PARTS.with(|c| { let c = c.borrow(); c.get(key).or_else(|| c.get(&crate::projectile_gen::projectile_content_id(key))).cloned().unwrap_or_default() })
}
/// note the objects this conversion's fighter holds over the screen
pub fn set_screen_objects(names: BTreeSet<String>) { SCREEN_OBJECTS.with(|c| *c.borrow_mut() = names); }
/// whether an object (by its entity key) is one its fighter holds over the screen
pub fn is_screen_object(key: &str) -> bool {
    SCREEN_OBJECTS.with(|c| c.borrow().iter().any(|n| crate::projectile_gen::projectile_content_id(n) == crate::projectile_gen::projectile_content_id(key) || n == key))
}

pub fn label_jumps(model: &SourceModel, pieces: &[Piece]) -> BTreeMap<String, crate::compile::ClipJumps> {
    let mut out = BTreeMap::new();
    let syms: BTreeSet<u16> = pieces.iter().map(|p| p.sym).collect();
    for sym in syms {
        let Some(c) = model.class_of(sym) else { continue };
        let Some(sprite) = model.sprites.get(&sym) else { continue };
        let own: Vec<&Piece> = pieces.iter().filter(|p| p.sym == sym && !p.borrowed && !p.reversed && p.head == 0).collect();
        let mut j = crate::compile::ClipJumps { anims: own.iter().map(|p| p.fm_name.clone()).collect(), labels: BTreeMap::new(), autolink: None, declared: BTreeMap::new(), sounds: BTreeMap::new(), freeze_unstated: (false, false) };
        for (f0, label) in &sprite.labels {
            let f = *f0 as u32 + 1;
            let hits: Vec<&&Piece> = own.iter().filter(|p| f >= p.from && f < p.from + p.frames).collect();
            // (where the whole clip and the label's own cut both show it, the cut that starts there)
            let at = if let [p] = hits.as_slice() { Some(*p) } else { hits.iter().find(|p| p.from == f).copied() };
            // (a label whose frames place a clip of its own, played whole as its own animation, where
            // the clip's own timeline holds it for fewer frames than it runs: that animation)
            let window = at.map(|p| if f == p.from { p.frames } else { p.frames.saturating_sub(f - p.from) }).unwrap_or(0);
            let state = pieces.iter().find(|p| p.sym != sym && p.label == *label && p.from == 1 && !p.borrowed && p.frames > window);
            if let Some(p) = state { j.labels.insert(label.clone(), (p.fm_name.clone(), true)); continue }
            if let Some(p) = at { j.labels.insert(label.clone(), (p.fm_name.clone(), f == p.from)); }
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
    /// the angle above level the source's engine throws a held item forward at, in degrees
    pub toss_angle: f64,
    /// the source frame of a throw stance a held item leaves the hand on
    pub toss_frame: f64,
    /// an acceleration of the source's in this engine's units
    pub accel: f64,
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

/// One object the package declares, as its script is written: the class that runs it, the clip
/// that is its art, and its own declared stats.
pub struct ObjectSpec<'a> {
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

/// The fighter's script, written after `haxe_gen::generate` has written everything else (art,
/// boxes, stats, the splits, and an entity with every frame and no code on it).
pub fn fighter(char_dir: &std::path::Path, char_name: &str, char_pascal: &str,
                       data: &crate::extractor::CharacterData,
                       sprite_boxes: &BTreeMap<String, crate::sprite_parser::AnimationBoxData>,
                       input_path: &std::path::Path) -> anyhow::Result<()> {
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
    let units = Units { size: scaling.size_multiplier, speed: scaling.velocity_scale(), item_toss: scaling.item_toss_speed, toss_angle: scaling.item_toss_angle, toss_frame: scaling.item_toss_frame, accel: scaling.accel_scale() };
    let mut projectile_pieces = crate::entity_gen::take_projectile_pieces();
    // the entity each declaration was written as: by its own name, else by its clip's
    let mut entities: BTreeMap<String, String> = BTreeMap::new();
    for (name, decl) in fighter.projectiles.entries().iter().chain(fighter.items.entries()) {
        let linkage = decl.get("linkage_id").and_then(|v| v.str()).map(String::from);
        // (or the entity written under its name with its fighter's own prefix off: a package of
        // several fighters names theirs `<fighter>_<name>`, and the entity drops the prefix)
        let id = crate::projectile_gen::projectile_content_id(name);
        let key = if projectile_pieces.contains_key(name) { Some(name.clone()) }
            else { linkage.filter(|l| projectile_pieces.contains_key(l)) }
            .or_else(|| projectile_pieces.keys().find(|k| crate::projectile_gen::projectile_content_id(&format!("{char_name}_{k}")) == id).cloned());
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
    // an item its code makes by name that it does not carry: the game's shared packages declare
    // it, and it is ported as one of the fighter's own items, its class run from that package
    let lent = lent_items(&runtime, &others.iter().filter(|n| !entities.contains_key(*n)).cloned().collect::<Vec<_>>());
    let mut with_lent = fighter.clone();
    let mut lent_of: BTreeMap<String, &SourceModel> = BTreeMap::new();
    for (m, decls) in &lent {
        for (key, decl) in decls {
            // (one the conversion drew no art for has nothing to show: not lent)
            if !projectile_pieces.keys().any(|k| k.eq_ignore_ascii_case(key)) { continue }
            // (a projectile by its class, `PeptideProjectile` extended; else an item)
            let class = decl.get("classAPI").and_then(|v| v.class()).and_then(|c| m.classes.get(c).or_else(|| m.classes.values().find(|x| compile::short(&x.name) == c)));
            let projectile = class.is_some_and(|c| std::iter::once(c.super_name.clone()).chain(m.supers(&c.name)).any(|s| compile::short(&s) == "PeptideProjectile"));
            if projectile {
                let mut list = with_lent.projectiles.entries().to_vec();
                list.push((key.clone(), decl.clone()));
                with_lent.projectiles = Value::Obj(list);
            } else {
                let mut items = with_lent.items.entries().to_vec();
                items.push((key.clone(), decl.clone()));
                with_lent.items = Value::Obj(items);
            }
            entities.insert(key.clone(), key.clone());
            lent_of.insert(key.clone(), m);
            unplaced.retain(|u| !u.starts_with(&format!("{key}: made from another package")));
            if let Some(c) = decl.get("classAPI").and_then(|v| v.class()).and_then(|c| m.classes.get(c).or_else(|| m.classes.values().find(|x| compile::short(&x.name) == c))) {
                if let Ok(mut l) = compile::native::lent_methods().lock() { l.extend(c.methods.iter().map(|x| x.name.clone())); }
            }
        }
    }
    let fighter = &with_lent;
    let effects = crate::entity_gen::take_effect_pieces();
    let effect_inners = crate::entity_gen::take_effect_inners();
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
    let mut native_cancels = attack_cancels(data, &model, &pieces, units, true);
    // what each attack's declared stats have its animations run on their first frame
    // (`code` in runtime/attack_stats.jsonc)
    // (they start a timer on a function written into its script once)
    let mut taken = std::collections::BTreeSet::new();
    for a in &data.attacks {
        let native_code = compile::attack_stats::table().first_frame(&a.stats);
        if native_code.is_empty() { continue }
        // every animation under the source label the attack's own animation is
        let fm = data.ssf2_to_fm_anim.get(&a.ssf2_name).cloned().unwrap_or_else(|| a.name.clone());
        let Some(label) = pieces.iter().find(|p| p.fm_name == fm).map(|p| p.label.clone()).filter(|l| !l.is_empty()) else { continue };
        for p in pieces.iter().filter(|p| p.label == label) {
            let c = native_cancels.entry(p.fm_name.clone()).or_default();
            for (first, t, key) in &native_code {
                if *first && p.fm_name != fm { continue }
                taken.insert(key.clone());
                // two rows that share a timer write its start once
                for line in t.lines() {
                    if c.lines().any(|l| l == line) { continue }
                    if !c.is_empty() { c.push('\n'); }
                    c.push_str(line);
                }
            }
        }
    }
    // what the source's engine does in its own states, on the first frame of the animation that
    // shows each (runtime/engine_animations.jsonc)
    let has = |a: &str| pieces.iter().any(|p| p.fm_name == a);
    for (anim, parts) in &compile::engine_anims::table().code {
        if !has(anim) { continue }
        for (_, code) in parts.iter().filter(|(needs, _)| needs.as_deref().is_none_or(has)) {
            let c = native_cancels.entry(anim.clone()).or_default();
            if !c.is_empty() { c.push('\n'); }
            c.push_str(code);
        }
    }
    // (the scale of the source's pixels, filled in)
    for c in native_cancels.values_mut() { *c = c.replace("{size}", &literal(&Value::Num(units.size), &|_| None)); }
    let keyed: String = native_cancels.values().cloned().collect::<Vec<_>>().join("\n");
    let mut native_stat_code = compile::attack_stats::table().code(&taken);
    // the functions the source's own states run, for the ones its animations' code names
    for (name, lines) in &compile::engine_anims::table().functions {
        if regex::Regex::new(&format!(r"\b{name}\b")).is_ok_and(|r| r.is_match(&keyed)) { native_stat_code.extend(lines.iter().cloned()); }
    }
    let size_text = literal(&Value::Num(units.size), &|_| None);
    for l in native_stat_code.iter_mut() { *l = l.replace("{size}", &size_text); }
    // its own code as Fraymakers code, where it can be (`native_fighter`)
    let anim_of: BTreeMap<String, String> = data.ssf2_to_fm_anim.iter().map(|(l, a)| (l.clone(), a.clone())).collect();
    let entity_keys: BTreeSet<String> = projectile_pieces.keys().cloned().collect();
    // (the links its animation stats give the engine, as written)
    let stats = std::fs::read_to_string(char_dir.join(format!("library/scripts/{char_pascal}/AnimationStats.hx"))).unwrap_or_default();
    let frame_links = native_fighter::frame_links(&stats);
    let fighting = native_fighter::Fighting { model: &model, runtime: &runtime, fighter, pieces: &pieces, units, effects: &effects, entities: &entities, anim_of: &anim_of, entity_keys: &entity_keys, frame_links: &frame_links, autolink: &calls, parts: &data.parts, part_placements: &data.part_placements, content: char_name,
        palette: std::fs::read_to_string(char_dir.join("library/costumes.palettes")).ok().and_then(|t| native_fighter::palette_of(&t)), effect_inners: &effect_inners };
    let stat_lines = native_stat_code;
    let (script, fighter_gaps, meta) = native_fighter::write(char_dir, char_pascal, char_name, &fighting, &native_cancels, &stat_lines)
        .map_err(|why| anyhow::anyhow!("{}", why.join("; ")))?;
    std::fs::write(dir.join("Script.hx"), &script)?;
    // what a clip's frame writes of its boxes as it starts an animation is that animation's own
    let box_writes = entry_box_writes(&model, &pieces);
    landing_stats(&dir, &box_writes, &label_jumps(&model, &pieces))?;
    autolink_entity(&entity_path, &dir.join("HitboxStats.hx"), char_name, &links, units.size)?;
    engine_animations(&entity_path, &dir.join("AnimationStats.hx"), char_name)?;
    native_fighter::write_attack_meta(&dir.join("AnimationStats.hx"), &meta)?;
    // each projectile and item the fighter declares, and each object it makes by class
    let owner = native_projectile::Owner { attacks: &fighter.attacks, stats: &fighter.stats, anims: pieces.iter().map(|p| (p.fm_name.clone(), p.label.clone())).collect(), native: true, origin: (0.0, 0.0) };
    let mut item_gaps: Vec<(String, String)> = Vec::new();
    let projectiles = fighter_projectiles(char_dir, &model, &runtime, fighter, units,
        &effects, &entities, &mut projectile_pieces, &mut unplaced, &owner, &mut item_gaps, &lent_of)?;
    // every item is a custom game object carrying the item handler
    for (name, _) in fighter.items.entries() {
        let Some(key) = entities.get(name) else { continue };
        custom_game_object(char_dir, key)?;
    }
    let report = serde_json::json!({
        // what its source does that its Fraymakers code does not, the fighter's own and each
        // projectile's and item's
        "gaps": fighter_gaps.iter().collect::<Vec<_>>(),
        "projectile_gaps": projectiles.iter().map(|(n, g)| (n.clone(), serde_json::json!(g))).collect::<serde_json::Map<_, _>>(),
        "unplaced": unplaced,
        "projectiles": projectiles.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        // the items whose own code is not written, which carry the item handler alone, and why
        "items_without_own_code": item_gaps.iter().map(|(n, why)| (n.clone(), serde_json::Value::String(why.clone()))).collect::<serde_json::Map<_, _>>(),
        // the box stats an animation took from what its clip writes as it starts (by the source's
        // box number and field names), so a check against the declared boxes knows them
        "box_writes": box_writes.iter().map(|(l, _)| (l.anim.clone().unwrap_or_default(), serde_json::Value::Object(
            l.boxes.iter().map(|(i, f)| (i.to_string(), serde_json::Value::Object(
                f.iter().map(|(k, v)| (k.clone(), serde_json::json!(v))).collect()))).collect()))).collect::<serde_json::Map<_, _>>(),
    });
    write_port_report(char_dir, char_name, report)?;
    // the plan the verify gates hold the fighter to gains the walk speeds its walk clips set
    let plan_path = char_dir.join("verify.json");
    if let Some(mut plan) = std::fs::read(&plan_path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
        plan["walk_script_speeds"] = serde_json::json!(walk_script_speeds(&model, fighter, &pieces, units.speed));
        std::fs::write(&plan_path, serde_json::to_string_pretty(&plan)?)?;
    }
    Ok(())
}

/// An item written as Fraymakers code, made a custom game object: its entity, its script and its
/// entry in the manifest say so (its stats file's fields are a game object's too)
pub(crate) fn custom_game_object(char_dir: &std::path::Path, key: &str) -> anyhow::Result<()> {
    let content = crate::projectile_gen::projectile_content_id(key);
    let set_type = |v: &mut serde_json::Value| {
        if let Some(m) = v.pointer_mut("/pluginMetadata/com.fraymakers.FraymakersMetadata/objectType") { *m = serde_json::json!("CUSTOM_GAME_OBJECT"); }
    };
    let entity = char_dir.join(format!("library/entities/{}.entity", crate::haxe_gen::sanitize_entity_name(&content)));
    let meta = char_dir.join(format!("library/scripts/Projectile/{}Script.hx.meta", crate::haxe_gen::snake_to_pascal(key)));
    for path in [&entity, &meta] {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let mut json: serde_json::Value = serde_json::from_slice(&bytes)?;
        set_type(&mut json);
        std::fs::write(path, serde_json::to_string_pretty(&json)?)?;
    }
    let manifest = char_dir.join("library/manifest.json");
    if let Ok(bytes) = std::fs::read(&manifest) {
        let mut json: serde_json::Value = serde_json::from_slice(&bytes)?;
        if let Some(list) = json["content"].as_array_mut() {
            for c in list.iter_mut().filter(|c| c["id"] == content.as_str()) { c["type"] = serde_json::json!("customGameObject"); }
        }
        std::fs::write(&manifest, serde_json::to_string_pretty(&json)?)?;
    }
    Ok(())
}

/// An attack that declares `cancel` can be cut short: while it plays, holding the button it was
/// made with once its clip says it is waiting (a clip that says nothing is) drops its fall and
/// sends the clip to its `finish` label, once (kirby's stone). The check is the attack's own, on
/// its first frame: a timer of the engine's that ends with the attack's state.
fn attack_cancels(data: &crate::extractor::CharacterData, model: &SourceModel, pieces: &[Piece], units: Units, native: bool) -> BTreeMap<String, String> {
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
                None if native => "stanceJump(\"finish\");".to_string(),
                None => "if (__st != null) __st.gotoAndPlay(\"finish\");".to_string(),
            };
            // (written as Fraymakers code: whether its clip is waiting is the clip's own container)
            let waiting = if native { "\tvar __w = {field:waiting};\n\tif (__w != null && __w.get() != true) return;" } else {
                "\tvar __st = PeptideRuntime.shownClip();\n\t// a clip that declares it says (unset is not waiting); one that does not is waiting\n\tvar __w = __st == null ? true : (__st.__c_waiting != null ? AS3.truthy(__st.__c_waiting.get()) : (__st.waiting == null ? true : AS3.truthy(__st.waiting)));\n\tif (!__w) return;" };
            out.insert(p.fm_name.clone(), format!(
"// its declared `cancel`: holding {what} once its clip is waiting cuts it short at `finish`
(function() {{ var armed = true; self.addTimer(1, -1, function(?t) {{
	if (!armed) return;
	var __c = self.getHeldControls();
	if (!({held})) return;
{waiting}
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
            for (k, v) in &l.attack { table.put(f, &mut ranks, k, *v, true, &|w| l.attack.iter().find(|(x, _)| x == w).map(|(_, n)| *n)); }
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

/// whether any script of the project makes the object of this content (names it to the engine):
/// one nothing makes is art no code reaches, whose stats never matter
fn made_anywhere(char_dir: &std::path::Path, key: &str, content: &str) -> bool {
    let quoted = format!("\"{content}\"");
    // (its own files name it as their own: not a maker)
    let own = crate::haxe_gen::snake_to_pascal(key);
    let mut dirs = vec![char_dir.join("library/scripts")];
    while let Some(d) = dirs.pop() {
        for f in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = f.path();
            if p.is_dir() { dirs.push(p); continue }
            if p.extension().and_then(|e| e.to_str()) != Some("hx") { continue }
            if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&own)) { continue }
            if std::fs::read_to_string(&p).is_ok_and(|t| t.contains(&quoted)) { return true }
        }
    }
    // (and the frame code its entities carry)
    std::fs::read_dir(char_dir.join("library/entities")).into_iter().flatten().flatten()
        .any(|f| std::fs::read_to_string(f.path()).is_ok_and(|t| t.contains(&format!("\\\"{content}\\\""))))
}

/// a projectile a shared package's item says it fires (`projectiles` in the item's declaration),
/// by its name or the clip it is drawn as: its name there and its declaration
fn shared_shot(runtime: &Runtime, key: &str) -> Option<(String, Value)> {
    // (every shared package's items' shots, read once)
    static SHOTS: std::sync::OnceLock<Vec<(String, Value)>> = std::sync::OnceLock::new();
    let cid = crate::projectile_gen::projectile_content_id;
    let all = SHOTS.get_or_init(|| crate::shared_packages::packages().iter().filter_map(|pkg| {
        let (m, _) = runtime.read_package(&pkg.swf).ok()?;
        Some(m.main_class()?.methods.iter()
            .filter_map(|meth| match crate::model::declare::returned(&meth.function.body) { Some(Value::Arr(list)) => Some(list), _ => None })
            .flatten()
            .flat_map(|d| d.get("projectiles").map(|v| v.entries().to_vec()).unwrap_or_default())
            .collect::<Vec<_>>())
    }).flatten().collect());
    all.iter().find(|(n, d)| cid(n) == cid(key) || d.get("linkage_id").and_then(|v| v.str()).is_some_and(|l| cid(l) == cid(key))).cloned()
}

/// The items a fighter's code makes by name without carrying them, as the game's shared packages
/// declare them: a package's main class returns its items as declarations, each naming the clip
/// its code makes it by (`linkage_id`) and the class that runs it (`classAPI`). Each package that
/// declares one, with the declarations it has, by the name the fighter makes it by.
fn lent_items(runtime: &Runtime, wanted: &[String]) -> Vec<(SourceModel, Vec<(String, Value)>)> {
    if wanted.is_empty() { return Vec::new() }
    // each shared package once: its model, and every declaration its main class lists
    let pkgs: Vec<(SourceModel, Vec<Value>)> = crate::shared_packages::packages().iter().filter_map(|pkg| {
        let (m, _) = runtime.read_package(&pkg.swf).ok()?;
        let decls: Vec<Value> = m.main_class()?.methods.iter()
            .filter_map(|meth| match crate::model::declare::returned(&meth.function.body) { Some(Value::Arr(list)) => Some(list), _ => None })
            .flatten().filter(|d| d.get("linkage_id").and_then(|v| v.str()).is_some() && d.get("classAPI").and_then(|v| v.class()).is_some()).collect();
        Some((m, decls))
    }).collect();
    // what a lent object's own code makes by name is lent too, until nothing new is asked for
    let mut want: Vec<String> = wanted.to_vec();
    let mut found: Vec<Vec<(String, Value)>> = vec![Vec::new(); pkgs.len()];
    let mut asked = 0;
    while asked < want.len() {
        let name = want[asked].clone();
        asked += 1;
        for (i, (m, decls)) in pkgs.iter().enumerate() {
            let Some(d) = decls.iter().find(|d| d.get("linkage_id").and_then(|v| v.str()).is_some_and(|l| l.eq_ignore_ascii_case(&name))) else { continue };
            if found.iter().flatten().any(|(k, _)| k.eq_ignore_ascii_case(&name)) { break }
            found[i].push((name.clone(), d.clone()));
            // (and the projectiles its declaration says it fires, `projectiles`, with it)
            for (pn, pd) in d.get("projectiles").map(|v| v.entries().to_vec()).unwrap_or_default() {
                if !found[i].iter().any(|(k, _)| k.eq_ignore_ascii_case(&pn)) { found[i].push((pn, pd)); }
            }
            let Some(c) = d.get("classAPI").and_then(|v| v.class()).and_then(|c| m.classes.get(c).or_else(|| m.classes.values().find(|x| compile::short(&x.name) == c))) else { break };
            for k in std::iter::once(c.name.clone()).chain(m.supers(&c.name)).filter_map(|n| m.classes.get(&n)) {
                for f in k.methods.iter().map(|x| &x.function).chain([&k.constructor]) {
                    crate::model::ast::walk_exprs(&f.body, &mut |e| if let crate::model::ast::Expr::Call(g, a) = e {
                        let made = matches!(&**g, crate::model::ast::Expr::Member(_, n) | crate::model::ast::Expr::Name(n)
                            if matches!(n.as_str(), "createProjectile" | "spawnProjectile" | "makePickup" | "makeFighterPickup" | "placePickup"));
                        if let (true, Some(crate::model::ast::Expr::Str(n))) = (made, a.first()) {
                            if !want.iter().any(|w| w.eq_ignore_ascii_case(n)) { want.push(n.clone()); }
                        }
                    });
                }
            }
            break;
        }
    }
    pkgs.into_iter().zip(found).filter(|(_, f)| !f.is_empty()).map(|((m, _), f)| (m, f)).collect()
}

/// The projectiles and items a fighter declares, and the objects it makes by class: each one the
/// pipeline wrote an entity for gets its script written as Fraymakers code, from its own
/// declaration (the class that runs it, the clip that is its art, its stats).
#[allow(clippy::too_many_arguments)]
fn fighter_projectiles(char_dir: &std::path::Path, model: &SourceModel, runtime: &Runtime,
                       fighter: &crate::model::declare::Fighter, units: Units,
                       effects: &BTreeMap<String, Vec<Piece>>, entities: &BTreeMap<String, String>,
                       pieces_of: &mut BTreeMap<String, Vec<Piece>>,
                       unplaced: &mut Vec<String>, owner: &native_projectile::Owner,
                       item_gaps: &mut Vec<(String, String)>,
                       lent: &BTreeMap<String, &SourceModel>) -> anyhow::Result<Vec<(String, Vec<String>)>> {
    let own_model = model;
    let mut out = Vec::new();
    // (the packages clips were borrowed from, read once each: their models and fighters)
    let mut borrowed: Vec<(std::rc::Rc<Vec<u8>>, SourceModel, Vec<crate::model::declare::Fighter>)> = Vec::new();
    let decls: Vec<&(String, Value)> = fighter.projectiles.entries().iter().chain(fighter.items.entries()).collect();
    // (and what an object's own class declares it fires, `getProjectileStats`: a projectile one
    // of its objects fires is declared there, not by the fighter)
    let class_declared: Vec<(String, Value)> = model.classes.values()
        .filter_map(|c| c.methods.iter().find(|m| m.name == "getProjectileStats" && !m.is_static))
        .filter_map(|m| crate::model::declare::returned(&m.function.body))
        .flat_map(|v| v.entries().to_vec()).collect();
    // one script per entity, from the first declaration written as it (a projectile fired by
    // another declaration sharing its clip takes that one's stats as it starts)
    for (entity_key, pieces) in std::mem::take(pieces_of) {
        // (an item another package lends it runs its class from that package)
        let model = lent.get(&entity_key).copied().unwrap_or(own_model);
        // written as it by name, else named as its clip (a declaration fired under one name that
        // draws with a clip of another has an entity for each)
        let by_clip = |d: &Value| d.get("linkage_id").and_then(|v| v.str()) == Some(entity_key.as_str());
        let found = decls.iter().copied().find(|(n, _)| entities.get(n) == Some(&entity_key))
            .or_else(|| decls.iter().copied().find(|(_, d)| by_clip(d)))
            .or_else(|| class_declared.iter().find(|(n, d)| n.eq_ignore_ascii_case(&entity_key) || by_clip(d)));
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
                declare_in_stats(char_dir, &entity_key, &stats)?;
                let content = crate::projectile_gen::projectile_content_id(&entity_key);
                let entity = char_dir.join(format!("library/entities/{}.entity", crate::haxe_gen::sanitize_entity_name(&content)));
                let has = entity_animations(&entity);
                let pieces: Vec<Piece> = pieces.into_iter().filter(|p| has.contains(&p.fm_name)).collect();
                // (a clip class is the code of the clip it draws, run as one of its shown clips; the
                // object itself is then the API's projectile)
                let clip_class = matches!(compile::short(&c.super_name), "MovieClip" | "Sprite");
                let spec = ObjectSpec { effects, entities, class: if clip_class { "PeptideProjectile".into() } else { c.name.clone() },
                    clip: entity_key.clone(), clip_sym: Some(sym), stats };
                let (text, gaps) = native_projectile::write(char_dir, model, runtime, &spec, &Decls::of(fighter), &pieces, units, &entity_key, &entity_key, owner, false)
                    .map_err(|e| anyhow::anyhow!("{entity_key}: {e}"))?;
                std::fs::write(&script, &text)?;
                unplaced.retain(|u| !u.starts_with(&format!("{entity_key}: made from another package")));
                out.push((entity_key, gaps));
                continue;
            }
            // (one nothing declares and no class of its own runs: the API's projectile, running
            // its clip's own code)
            // (a clip borrowed from another fighter's package, a power taken from that fighter: its
            // code and its declaration are that package's, and it is written from there)
            if let Some(bytes) = crate::image_extractor::borrowed_package(&entity_key) {
                if !borrowed.iter().any(|(b, _, _)| std::rc::Rc::ptr_eq(b, &bytes)) {
                    if let Ok((m, _)) = runtime.read_package(&bytes) { let fs = m.fighters(); borrowed.push((bytes.clone(), m, fs)); }
                }
                let cid = crate::projectile_gen::projectile_content_id;
                if let (Some((_, m, fs)), Some(sym), true) = (borrowed.iter().find(|(b, _, _)| std::rc::Rc::ptr_eq(b, &bytes)), pieces.first().map(|p| p.sym), script.exists()) {
                    let found = fs.iter().find_map(|f| f.projectiles.entries().iter()
                        .find(|(n, d)| cid(n) == cid(&entity_key) || d.get("linkage_id").and_then(|v| v.str()).is_some_and(|l| cid(l) == cid(&entity_key)))
                        .map(|(n, d)| (f, n.clone(), d.clone())));
                    let content = cid(&entity_key);
                    let entity = char_dir.join(format!("library/entities/{}.entity", crate::haxe_gen::sanitize_entity_name(&content)));
                    let has = entity_animations(&entity);
                    let pieces: Vec<Piece> = pieces.iter().filter(|p| has.contains(&p.fm_name)).cloned().collect();
                    // (declared by none of its fighters: a class of that package whose own stats name
                    // its clip, or a class by its name (an enemy a list hands out), runs it)
                    let own_stats = |c: &crate::model::ClassModel| c.methods.iter().find(|x| x.name == "ownStats" && !x.is_static)
                        .and_then(|x| crate::model::declare::returned(&x.function.body));
                    let enemy = if found.is_some() { None } else {
                        m.classes.values().find(|c| own_stats(c).is_some_and(|v| v.get("linkage_id").and_then(|l| l.str()).is_some_and(|l| l.eq_ignore_ascii_case(&entity_key))))
                            .or_else(|| m.classes.values().find(|c| compile::short(&c.name).eq_ignore_ascii_case(&entity_key) && own_stats(c).is_some()))
                    };
                    // (or one a class of that package says it fires, `getProjectileStats`: a pokemon's
                    // shot)
                    let matches = |n: &str, d: &Value| cid(n) == cid(&entity_key) || d.get("linkage_id").and_then(|v| v.str()).is_some_and(|l| cid(l) == cid(&entity_key));
                    let shot = if found.is_some() || enemy.is_some() { None } else {
                        m.classes.values().filter_map(|c| c.methods.iter().find(|x| x.name == "getProjectileStats" && !x.is_static))
                            .filter_map(|x| crate::model::declare::returned(&x.function.body))
                            .find_map(|v| v.entries().iter().find(|(n, d)| matches(n, d)).cloned()).map(|(n, d)| (fighter, n, d))
                            // (one its fighters declare as an item, which this fighter fires as its own
                            // projectile: pac-man's fruit kirby copies; its stats are the item's)
                            .or_else(|| fs.iter().find_map(|f| f.items.entries().iter().find(|(n, d)| matches(n, d)).cloned()).map(|(n, d)| (fighter, n, d)))
                            // (one a shared package's item says it fires: a star rod's star)
                            .or_else(|| shared_shot(runtime, &entity_key).map(|(n, d)| (fighter, n, d)))
                    };
                    let found = found.or(shot);
                    let (decls, title, stats, class, clip) = match (&found, enemy) {
                        (Some((f, n, d)), _) => (Decls::of(f), n.clone(), d.clone(),
                            d.get("classAPI").and_then(|v| v.class()).unwrap_or("PeptideProjectile").to_string(),
                            d.get("linkage_id").and_then(|v| v.str()).unwrap_or(&entity_key).to_string()),
                        (None, Some(c)) => (Decls::of(fighter), entity_key.clone(), own_stats(c).unwrap_or(Value::Null), c.name.clone(),
                            own_stats(c).and_then(|v| v.get("linkage_id").and_then(|l| l.str()).map(String::from)).unwrap_or_else(|| entity_key.clone())),
                        (None, None) => (Decls::of(fighter), entity_key.clone(), Value::Null, "PeptideProjectile".to_string(), entity_key.clone()),
                    };
                    let declared_by_class = enemy.is_some();
                    if declared_by_class { declare_in_stats(char_dir, &entity_key, &stats)?; }
                    let spec = ObjectSpec { effects, entities, class, clip, clip_sym: Some(sym), stats };
                    let (text, mut gaps) = native_projectile::write(char_dir, m, runtime, &spec, &decls, &pieces, units, &title, &entity_key, owner, false)
                        .map_err(|e| anyhow::anyhow!("{entity_key}: {e}"))?;
                    if found.is_none() && !declared_by_class { gaps.push("declared by nothing its package carries: its stats are the engine's defaults".into()); }
                    std::fs::write(&script, &text)?;
                    unplaced.retain(|u| !u.starts_with(&format!("{entity_key}: made from another package")));
                    out.push((entity_key, gaps));
                    continue;
                }
            }
            // (its art must be a clip of this package: one drawn from another package's clip, a
            // power taken from another fighter, has its code there)
            let own_clip = |sym: u16| model.sprites.contains_key(&sym)
                && model.class_of(sym).is_none_or(|c| std::iter::once(c.super_name.clone()).chain(model.supers(&c.name)).any(|s| matches!(compile::short(&s), "MovieClip" | "Sprite")));
            if let (Some(sym), true) = (pieces.first().map(|p| p.sym).filter(|s| own_clip(*s)), script.exists()) {
                let content = crate::projectile_gen::projectile_content_id(&entity_key);
                let entity = char_dir.join(format!("library/entities/{}.entity", crate::haxe_gen::sanitize_entity_name(&content)));
                let has = entity_animations(&entity);
                let pieces: Vec<Piece> = pieces.into_iter().filter(|p| has.contains(&p.fm_name)).collect();
                // (one a shared package's item says it fires, the star a star rod fires: that declaration)
                let lent = shared_shot(runtime, &entity_key);
                if let Some((_, d)) = &lent { declare_in_stats(char_dir, &entity_key, d)?; }
                let spec = ObjectSpec { effects, entities, class: "PeptideProjectile".into(),
                    clip: entity_key.clone(), clip_sym: Some(sym), stats: lent.as_ref().map(|(_, d)| d.clone()).unwrap_or(Value::Null) };
                let (text, mut gaps) = native_projectile::write(char_dir, model, runtime, &spec, &Decls::of(fighter), &pieces, units, &entity_key, &entity_key, owner, false)
                    .map_err(|e| anyhow::anyhow!("{entity_key}: {e}"))?;
                if lent.is_none() { gaps.push("declared by nothing it carries: its stats are the engine's defaults".into()); }
                std::fs::write(&script, &text)?;
                unplaced.retain(|u| !u.starts_with(&format!("{entity_key}: made from another package")));
                out.push((entity_key, gaps));
                continue;
            }
            pieces_of.insert(entity_key, pieces);
            continue;
        };
        declare_in_stats(char_dir, &entity_key, decl)?;
        let content = crate::projectile_gen::projectile_content_id(&entity_key);
        let entity = char_dir.join(format!("library/entities/{}.entity", crate::haxe_gen::sanitize_entity_name(&content)));
        let has = entity_animations(&entity);
        let (pieces, missing): (Vec<Piece>, Vec<Piece>) = pieces.into_iter().partition(|p| has.contains(&p.fm_name));
        unplaced.extend(missing.iter().map(|p| format!("{entity_key}: {} has no such animation", p.fm_name)));
        let spec = ObjectSpec {
            effects,
            entities,
            class: decl.get("classAPI").and_then(|v| v.class()).unwrap_or("PeptideProjectile").to_string(),
            clip: decl.get("linkage_id").and_then(|v| v.str()).unwrap_or(name).to_string(),
            clip_sym: None,
            stats: decl.clone(),
        };
        let script = char_dir.join(format!("library/scripts/Projectile/{}Script.hx", crate::haxe_gen::snake_to_pascal(&entity_key)));
        // its own code as Fraymakers code (`native_projectile`); an item whose own code cannot be
        // written carries the item handler alone, with the reason
        let item = native_projectile::is_item(&Decls::of(fighter), &spec, name);
        if !script.exists() { unplaced.push(format!("{entity_key}: no projectile script at {}", script.display())); continue }
        let mut written = native_projectile::write(char_dir, model, runtime, &spec, &Decls::of(fighter), &pieces, units, name, &entity_key, owner, false);
        if item {
            if let Err(why) = &written {
                item_gaps.push((entity_key.clone(), why.clone()));
                written = native_projectile::write(char_dir, model, runtime, &spec, &Decls::of(fighter), &pieces, units, name, &entity_key, owner, true);
            }
        }
        let (text, gaps) = written.map_err(|e| anyhow::anyhow!("{entity_key}: {e}"))?;
        std::fs::write(&script, &text)?;
        out.push((entity_key, gaps));
    }
    // (one declared by nothing that no script of the project makes is art no code reaches: its
    // stats never matter, so its defaults are no gap; judged once every script is written)
    for (key, gaps) in out.iter_mut() {
        let content = crate::projectile_gen::projectile_content_id(key);
        if gaps.iter().any(|g| g.starts_with("declared by nothing")) && !made_anywhere(char_dir, key, &content) {
            gaps.retain(|g| !g.starts_with("declared by nothing"));
        }
    }
    Ok(out)
}

/// what a declaration states, as a record of this engine's script: its plain fields (a number, a
/// truth, a text), by name. `None` when it states none
pub fn declared_record(d: &Value) -> Option<String> {
    let fields: Vec<String> = d.entries().iter().filter(|(k, _)| crate::re!(r"^[A-Za-z_]\w*$").is_match(k))
        .filter_map(|(k, v)| native_fighter::plain(v).map(|v| format!("{k}: {v}"))).collect();
    (!fields.is_empty()).then(|| format!("{{ {} }}", fields.join(", ")))
}

/// the declaration an object's entity is written from: the first of the fighter's projectiles and
/// items drawn as it (one fired under another name sharing its clip takes its own as it is fired)
pub fn entity_declaration<'a>(decls: &Decls<'a>, entities: &BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    decls.projectiles.entries().iter().chain(decls.items.entries()).find(|(n, _)| entities.get(n).map(String::as_str) == Some(key)).map(|(n, _)| n.as_str())
}

/// what a projectile fired by a name declares, where its entity is written from another
/// declaration sharing its clip: the record it takes on as it is fired
pub fn fired_record(decls: &Decls, entities: &BTreeMap<String, String>, name: &str) -> Option<String> {
    let key = entities.get(name)?;
    if entity_declaration(decls, entities, key) == Some(name) { return None }
    declared_record(decls.projectiles.get(name)?)
}

/// an object's declaration in its stats' metadata (`getGameObjectStatsMetadata()`), where another
/// object's code reads what it declares
fn declare_in_stats(char_dir: &std::path::Path, entity_key: &str, decl: &Value) -> anyhow::Result<()> {
    let Some(record) = declared_record(decl) else { return Ok(()) };
    let path = char_dir.join(format!("library/scripts/Projectile/{}Stats.hx", crate::haxe_gen::snake_to_pascal(entity_key)));
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(()) };
    if text.contains("\n    metadata: ") { return Ok(()) }
    let Some(end) = text.rfind('}') else { return Ok(()) };
    let body = text[..end].trim_end();
    let sep = if body.ends_with(',') || body.ends_with('{') { "" } else { "," };
    std::fs::write(&path, format!("{body}{sep}\n    metadata: {record}\n{}", &text[end..]))?;
    Ok(())
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
        jumps.insert("Clip".to_string(), crate::compile::ClipJumps { anims: vec!["aerial_up".into(), "aerial_up_land".into()], labels: BTreeMap::new(), autolink: None , declared: BTreeMap::new(), sounds: BTreeMap::new(), freeze_unstated: (false, false) });
        landing_stats(&dir, &[(landing, vec!["aerial_up_land".into()])], &jumps).unwrap();
        let anim = std::fs::read_to_string(dir.join("AnimationStats.hx")).unwrap();
        let hit = std::fs::read_to_string(dir.join("HitboxStats.hx")).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert!(anim.contains("aerial_up_land: { landType:LandType.NONE, allowMovement: false, leaveGroundType: LeaveGroundType.NONE }"), "{anim}");
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

/// an effect's clip that plays its frames once and goes, as the engine's own effect does: its class,
/// a plain clip's, does nothing but register its frame scripts, and the first of those stops it and
/// takes it off its parent (what comes after never runs). the frame that script is on, where it is
/// on the piece; none where it plays the piece out
fn plays_once(model: &SourceModel, p: &Piece) -> Option<Option<u32>> {
    use crate::model::ast::{BinOp, Stmt};
    let c = model.class_of(p.sym)?;
    if !matches!(compile::short(&c.super_name), "MovieClip" | "Sprite") { return None }
    // (the clip's own member, as `this.m` or bare)
    fn mine(e: &Expr) -> Option<&str> {
        match e { Expr::Member(o, m) if matches!(**o, Expr::This) => Some(m), Expr::Name(m) => Some(m), _ => None }
    }
    fn call(s: &Stmt) -> Option<(&Expr, &Vec<Expr>)> {
        match s { Stmt::Expr(Expr::Call(f, a)) => Some((&**f, a)), _ => None }
    }
    // the constructor registers its scripts and nothing else
    let quiet = c.constructor.body.iter().all(|s| matches!(s, Stmt::Return(None) | Stmt::Expr(Expr::SuperCall(..)))
        || call(s).is_some_and(|(f, _)| mine(f) == Some("addFrameScript")));
    if !quiet || c.methods.iter().any(|m| !c.frame_scripts.iter().any(|fs| fs.method == m.name)) { return None }
    // no script at all: it plays through once, as one with no class does
    let Some(first) = c.frame_scripts.iter().min_by_key(|fs| fs.frame) else { return Some(None) };
    let last = p.from + p.frames - 1;
    // (one that goes as it is made never shows)
    if first.frame < p.from || first.frame > last { return None }
    let m = c.methods.iter().find(|m| m.name == first.method)?;
    // taken off its parent, also where it asks first whether it has one
    fn off(s: &Stmt) -> bool {
        match s {
            Stmt::Expr(Expr::Call(f, a)) => matches!(&**f, Expr::Member(o, rc) if rc == "removeChild" && mine(o) == Some("parent")) && matches!(a.as_slice(), [Expr::This]),
            Stmt::If(cond, then, els) => els.is_empty() && then.iter().all(off) && !then.is_empty()
                && match cond { Expr::Binary(BinOp::Ne | BinOp::StrictNe, a, b) => mine(a) == Some("parent") && matches!(**b, Expr::Null), e => mine(e) == Some("parent") },
            _ => false,
        }
    }
    let mut gone = false;
    for s in &m.function.body {
        match call(s) {
            Some((f, a)) if mine(f) == Some("stop") && a.is_empty() => {}
            _ if off(s) => gone = true,
            _ if matches!(s, Stmt::Return(None)) => {}
            _ => return None,
        }
    }
    // (a script past the piece's last frame: the animation ends there itself)
    gone.then_some((first.frame < last).then_some(first.frame + 1))
}

/// what an effect's clip's own frames do, where all they do is turn the clip (stop it, play it on,
/// jump it to a label, take it off its parent): each as (its animation, the engine frame of it the
/// code is on, what it does as this engine's code with `e` the effect). none where its frames do
/// anything else, or nothing at all
pub fn effect_code(model: &SourceModel, pieces: &[Piece]) -> Option<Vec<(String, u32, String)>> {
    use crate::model::ast::Stmt;
    let sym = pieces.first()?.sym;
    let c = model.class_of(sym)?;
    if !matches!(compile::short(&c.super_name), "MovieClip" | "Sprite") || c.frame_scripts.is_empty() { return None }
    let mine = |e: &Expr| -> Option<String> { match e { Expr::Member(o, m) if matches!(**o, Expr::This) => Some(m.clone()), Expr::Name(m) => Some(m.clone()), _ => None } };
    // where a clip frame is: the animation showing it, and its engine frame there
    let at = |f: u32| pieces.iter().find(|p| f >= p.from && f < p.from + p.frames).map(|p| (p.fm_name.clone(), 2 * (f - p.from) + 1));
    let sprite = model.sprites.get(&sym)?;
    let label_at = |l: &str| sprite.labels.iter().find(|(_, x)| x == l).map(|(f, _)| *f as u32 + 1);
    let mut out = Vec::new();
    for fs in &c.frame_scripts {
        let m = c.methods.iter().find(|m| m.name == fs.method)?;
        let mut does: Vec<String> = Vec::new();
        for st in &m.function.body {
            match st {
                Stmt::Return(None) => {}
                Stmt::Expr(Expr::Call(f, a)) if matches!(&**f, Expr::Member(_, rc) if rc == "removeChild") && matches!(a.as_slice(), [Expr::This]) => does.push("e.kill();".into()),
                Stmt::Expr(Expr::Call(f, a)) => match (mine(f).as_deref(), a.as_slice()) {
                    (Some("stop"), []) => does.push("e.pause();".into()),
                    (Some("play"), []) => does.push("e.resume();".into()),
                    (Some(g @ ("gotoAndStop" | "gotoAndPlay")), [Expr::Str(l)]) => {
                        let (anim, frame) = at(label_at(l)?)?;
                        does.push(format!("effectGo(e, {}, {frame}, {});", compile::quote(&anim), g == "gotoAndStop"));
                    }
                    _ => return None,
                },
                // (taken off its parent: gone, as the effect is here)
                Stmt::If(_, then, els) if els.is_empty() && then.iter().all(|t| matches!(t, Stmt::Expr(Expr::Call(f, a)) if matches!(&**f, Expr::Member(_, rc) if rc == "removeChild") && matches!(a.as_slice(), [Expr::This]))) => does.push("e.kill();".into()),
                _ => return None,
            }
        }
        if does.is_empty() { continue }
        let (anim, frame) = at(fs.frame + 1)?;
        out.push((anim, frame, does.join(" ")));
    }
    (!out.is_empty()).then_some(out)
}

/// A clip and every clip placed inside it, as deep as they go.
pub fn clip_tree(model: &SourceModel, root: Option<u16>) -> BTreeSet<u16> {
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

thread_local! {
    /// the projects this conversion has written a fighter's report into, and the fighter it was
    static REPORTED: std::cell::RefCell<BTreeMap<std::path::PathBuf, String>> = Default::default();
}

/// A fighter's `port_report.json`. Several fighters of one package share a project (zelda and
/// sheik, wario and wario man): the first one's report is the project's, and each one after it
/// adds what it leaves out under its own name, rather than writing over it
fn write_port_report(char_dir: &std::path::Path, char_name: &str, report: serde_json::Value) -> anyhow::Result<()> {
    let path = char_dir.join("port_report.json");
    let first = REPORTED.with(|r| {
        let mut r = r.borrow_mut();
        match r.get(char_dir) { Some(owner) if owner != char_name => false, _ => { r.insert(char_dir.to_path_buf(), char_name.to_string()); true } }
    });
    let merged = if first { report } else {
        let mut base: serde_json::Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(serde_json::json!({}));
        let named = |v: &serde_json::Value| -> Vec<serde_json::Value> { v.as_array().map(|a| a.iter().map(|g| serde_json::json!(format!("{char_name}: {}", g.as_str().unwrap_or_default()))).collect()).unwrap_or_default() };
        for key in ["gaps", "unplaced", "projectiles"] {
            let add = if key == "projectiles" { report[key].as_array().cloned().unwrap_or_default() } else { named(&report[key]) };
            let list = base[key].as_array().cloned().unwrap_or_default().into_iter().chain(add).collect::<Vec<_>>();
            base[key] = serde_json::Value::Array(list);
        }
        for key in ["projectile_gaps", "items_without_own_code", "box_writes"] {
            let mut m = base[key].as_object().cloned().unwrap_or_default();
            for (k, v) in report[key].as_object().cloned().unwrap_or_default() { m.entry(k).or_insert(v); }
            base[key] = serde_json::Value::Object(m);
        }
        base
    };
    std::fs::write(&path, serde_json::to_string_pretty(&merged)?)?;
    Ok(())
}
