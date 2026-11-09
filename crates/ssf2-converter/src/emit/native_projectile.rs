//! A projectile written as plain Fraymakers code (`compile::native`), where its own code can be.
//!
//! Its script is `runtime/native/projectile.hx` filled in: its own classes' code, what its
//! declared stats have it do each frame (`runtime/projectile_stats.jsonc`), and its lifetime. Its
//! entity's keyframes call its frame scripts, and its animations move on into each other the way
//! its clip plays on (`nextAnimation`). Anything the writer has no plain way to say leaves the
//! projectile on the runtime, with the reason.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::{clip_tree, label_jumps, Decls, ObjectSpec, Piece, Units};
use crate::compile::{self, native, Runtime};
use crate::model::ast::{Expr, Place, Stmt};
use crate::model::declare::Value;
use crate::model::SourceModel;

/// what the projectile's owner declares for its attacks, for its code's reads of them
pub struct Owner<'a> {
    pub attacks: &'a Value,
    /// its own stats (`cData`)
    pub stats: &'a Value,
    /// each of the owner's animations, and the attack (its clip's label) it shows
    pub anims: Vec<(String, String)>,
    /// whether the owner is written as Fraymakers code
    pub native: bool,
}

/// The projectile's script and entity, written natively; or why it stays on the runtime.
pub fn write(char_dir: &Path, model: &SourceModel, runtime: &Runtime, spec: &ObjectSpec, decls: &Decls,
             pieces: &[Piece], units: Units, title: &str, entity_key: &str, owner: &Owner) -> Result<String, String> {
    let item = decls.items.entries().iter().any(|(n, d)| n == title || d.get("linkage_id").and_then(|v| v.str()) == Some(spec.clip.as_str()));
    let skeleton = runtime.native_files.get(if item { "item" } else { "projectile" }).ok_or("the runtime has no native/projectile.hx or native/item.hx")?;
    // its own class, where its package gives it one: the API's projectile, extended
    let itself = if matches!(compile::short(&spec.class), "PeptideProjectile" | "PeptideItem") { None } else {
        let c = model.classes.get(&spec.class).or_else(|| model.classes.values().find(|c| compile::short(&c.name) == compile::short(&spec.class)))
            .ok_or_else(|| format!("its class {} is not in its package", spec.class))?;
        Some(c)
    };
    let clip_sym = spec.clip_sym.or_else(|| model.symbol_of(&spec.clip)).ok_or("its clip is no symbol")?;
    if pieces.is_empty() { return Err("no animations".into()) }

    // the rate its attack hits again at, where it declares one (its own, else its idle attack's)
    let refresh = match spec.stats.get("refreshRate").or_else(|| spec.stats.get("attack_idle").and_then(|a| a.get("refreshRate"))) {
        Some(Value::Num(n)) if *n > 0.0 && *n < 999.0 => Some(num(*n)),
        _ => None,
    };

    // the classes whose code it runs: the classes of the clips its animations show. any other clip
    // inside it with code of its own keeps it on the runtime
    let shown: BTreeSet<u16> = pieces.iter().map(|p| p.sym).collect();
    let mut classes = Vec::new();
    for sym in clip_tree(model, Some(clip_sym)) {
        let Some(c) = model.class_of(sym) else { continue };
        if runtime.provides(&c.name) { continue }
        if shown.contains(&sym) { classes.push(c) }
        else if !is_quiet(model, runtime, &c.name) { return Err(format!("{} inside it has code of its own", c.name)) }
    }
    if classes.is_empty() && itself.is_none() { return Err("no code of its own".into()) }
    // a clip whose code its owner reaches into keeps the object the owner holds
    let names: BTreeSet<&str> = classes.iter().map(|c| c.name.as_str()).collect();
    for c in model.classes.values().filter(|c| !names.contains(c.name.as_str()) && !runtime.provides(&c.name)) {
        for f in c.methods.iter().map(|m| &m.function) {
            let mut listens = false;
            crate::model::ast::walk_exprs(&f.body, &mut |e| if let Expr::Str(s) = e { if matches!(s.as_str(), "projDestroyed" | "itemDestroyed") { listens = true } });
            if listens { return Err(format!("{} listens for a projectile's end", c.name)) }
        }
    }

    // a stat it declares, as code: one every declaration of its clip states alike
    let entity = spec.entities.get(title);
    let sharing: Vec<&Value> = decls.projectiles.entries().iter()
        .filter(|(n, d)| (entity.is_some() && spec.entities.get(n) == entity) || d.get("linkage_id").and_then(|v| v.str()) == Some(spec.clip.as_str()))
        .map(|(_, d)| d).collect();
    let agreed = |k: &str| sharing.iter().all(|d| format!("{:?}", d.get(k)) == format!("{:?}", spec.stats.get(k)));
    let declared = |k: &str| -> Option<String> {
        if !agreed(k) { return None }
        Some(match spec.stats.get(k) {
            None | Some(Value::Null) => "null".into(),
            Some(Value::Num(n)) => num(*n),
            Some(Value::Bool(b)) => b.to_string(),
            Some(Value::Str(s)) => compile::quote(s),
            _ => return None,
        })
    };

    // what its declared stats have it do each frame
    let stats: BTreeMap<String, f64> = spec.stats.entries().iter().filter_map(|(k, v)| match v {
        Value::Num(n) => Some((k.clone(), *n)), Value::Bool(b) => Some((k.clone(), if *b { 1.0 } else { 0.0 })), _ => None }).collect();
    let stat_names: BTreeMap<String, String> = spec.stats.entries().iter().filter_map(|(k, v)| v.str().map(|s| (k.clone(), s.to_string()))).collect();
    let effect = |n: &str| match crate::mappings::api_commands().global_vfx_map.iter().find(|(g, _)| g.as_str() == n) {
        Some((_, c)) => Some(format!("spriteContent: \"global::vfx.vfx\", animation: GlobalVfx.{c}")),
        None => spec.effects.get(n).and_then(|ps| ps.first()).map(|p|
            format!("spriteContent: self.getResource().getContent({}), animation: {}", compile::quote(n), compile::quote(&p.fm_name))),
    };
    let units_of = [("speed", units.speed), ("accel", units.accel), ("size", units.size)];
    let mut taken = compile::projectile_stats::table().rows_for(&stats, &stat_names, &effect, &|k| !agreed(k), &units_of, true);
    for l in taken.code.iter_mut() { *l = l.replace("{@clip}", &compile::quote(&spec.clip)); }

    // an attack stat its owner declares, by the owner's animation: the attack each shows
    let owner_stat = |k: &str| -> Option<(String, String)> {
        let mut cases: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (anim, label) in &owner.anims {
            let Some(v) = owner.attacks.get(label).and_then(|a| a.get(k)) else { continue };
            let v = match v { Value::Num(n) => num(*n), Value::Bool(b) => b.to_string(), Value::Str(s) => compile::quote(s), _ => continue };
            cases.entry(v).or_default().push(compile::quote(anim));
        }
        if cases.is_empty() { return Some(("null".into(), String::new())) }
        let name = format!("ownerAttack_{}", compile::ident(k));
        let lines: Vec<String> = cases.iter().map(|(v, anims)| format!("case {}: return {v};", anims.join(", "))).collect();
        native::helper(runtime, "helpers", "owner_attack", &name, &[("STAT", k.to_string())], &lines).ok().map(|def| (name, def))
    };

    // an effect its code makes: one of its own whose clip does nothing but play once, or a shared one
    // this engine has (as `object_script` decides it)
    let made = |name: &str| -> Option<(String, Option<u32>)> {
        if let Some((_, c)) = crate::mappings::api_commands().global_vfx_map.iter().find(|(g, _)| g.as_str() == name) {
            return Some((format!("spriteContent: \"global::vfx.vfx\", animation: GlobalVfx.{c}"), None));
        }
        let ps = spec.effects.get(name)?;
        let p = ps.iter().find(|p| p.from == 1)?;
        let gone = if ps.len() == 1 && (p.sym == 0 || model.class_of(p.sym).is_none()) { None } else { super::plays_once(model, p)? };
        Some((format!("spriteContent: self.getResource().getContent({}), animation: {}", compile::quote(name), compile::quote(&p.fm_name)), gone))
    };
    // a projectile its owner declares, for one its code fires
    let fired = |name: &str| -> Option<(String, [f64; 4])> {
        let decl = decls.projectiles.get(name)?;
        let key = spec.entities.get(name)?;
        let n = |ks: &[&str]| ks.iter().find_map(|k| match decl.get(k) { Some(Value::Num(v)) => Some(*v), _ => None }).unwrap_or(0.0);
        Some((crate::projectile_gen::projectile_content_id(key),
              [n(&["xoffset", "xOffset", "x_offset"]), n(&["yoffset", "yOffset", "y_offset"]), n(&["xspeed", "xSpeed", "x_speed"]), n(&["yspeed", "ySpeed", "y_speed"])]))
    };
    // a stat its owner states, for a read of it: what it declares, where its code has not changed it
    let owner_stat_of = |k: &str| -> Option<String> {
        match owner.stats.get(k)? { Value::Num(n) => Some(num(*n)), Value::Bool(b) => Some(b.to_string()), Value::Str(s) => Some(compile::quote(s)), _ => None }
    };
    let pkg = compile::Package::new(model, runtime);
    let owner_stores = |f: &str| pkg.stored.contains(f);
    let jumps = label_jumps(model, pieces);
    // (the labels and frames of its clips, for a jump its code works out as it runs)
    let labels = super::native_fighter::stance_labels(model, pieces);
    let frames_of = super::native_fighter::clip_frames_of(model, pieces);
    let no_links = BTreeSet::new();
    let object = native::Object {
        model, itself, classes: classes.clone(), jumps: &jumps, runtime, declared: &declared, switches: &taken.switches,
        owner_stat: &owner_stat, effect: &made, owner_anims: &owner.anims, fired: &fired, owner_declares: &owner_stat_of, owner_stores: &owner_stores, root: clip_sym, size: units.size, speed: units.speed, accel: units.accel, fighter: false, item,
        root_anim: &|_| None, walk_start: None, stance_labels: &labels, clip_frames: &frames_of, frame_links: &no_links, anim_clips: &[], owner_native: owner.native,
        effect_names: spec.effects.keys().cloned().chain(crate::mappings::api_commands().global_vfx_map.keys().cloned()).collect(),
        effect_pieces: spec.effects.iter().flat_map(|(n, ps)| ps.iter().map(move |p| (n.clone(), p.fm_name.clone(), p.from, p.frames))).collect(),
        parts: Vec::new(), content: String::new(), palette: None, effect_inners: BTreeMap::new(), effect_code: BTreeMap::new(),
    };
    let written = native::write(&object)?;

    // its script
    let life = spec.stats.get("time_max").and_then(|v| match v { Value::Num(n) => Some(*n), _ => None }).unwrap_or(60.0);
    let start = start_animation(char_dir, entity_key).or_else(|| pieces.first().map(|p| p.fm_name.clone())).unwrap_or_default();
    let first = pieces.iter().find(|p| p.fm_name == start && p.from == 1).and_then(|p| written.frames.get(&(p.sym, 1)));
    let reversible = !matches!(spec.stats.get("canBeReversed"), Some(Value::Bool(false)));
    let mut functions = written.functions.clone();
    if !taken.code.is_empty() {
        functions.push(format!("// ---- what its declared stats have it do each frame it flies: runtime/projectile_stats.jsonc\n{}", taken.code.join("\n")));
    }
    // its floor listeners, told of the floor the source's way
    let mut update: Vec<String> = Vec::new();
    if !written.floor.is_empty() {
        let told: Vec<String> = written.floor.iter().map(|(listening, h)| if written.floor_speed {
            format!("if ({listening}.get()) onFloor({h});") } else { format!("if ({listening}.get()) {h}(null);") }).collect();
        functions.push(native::helper(runtime, "floor", "told", "", &[], &told)?);
        if written.floor_speed { functions.push(native::helper(runtime, "floor", "speed", "", &[], &[])?); }
        if written.floor_speed { update.push("airFrame();".into()); }
        update.push("floorFrame();".into());
    }
    // its listeners for meeting its owner, told as it does
    if !written.touches.is_empty() {
        let told: Vec<String> = written.touches.iter().map(|(l, h)| format!("if ({l}.get()) {h}(e);")).collect();
        functions.push(native::helper(runtime, "touch", "owner", "", &[], &told)?);
        update.push("touchFrame();".into());
    }
    update.extend(taken.frame.iter().cloned());
    // its clip playing on from one animation into the next
    let on: Vec<String> = plays_into(model, pieces).iter().map(|(a, b)| format!("case {}: self.playAnimation({});", compile::quote(a), compile::quote(b))).collect();
    if !on.is_empty() {
        functions.push(native::helper(runtime, "helpers", "plays_on", "playOn", &[], &on)?);
        update.push("playOn();".into());
    }
    // the rate it hits again at, declared or written by its code
    let mut starts = taken.starts.clone();
    if refresh.is_some() || written.blocks.contains("refresh") {
        functions.push(native::helper(runtime, "refresh", "projectile", "", &[], &[])?);
        if let Some(r) = &refresh { starts.push(format!("setRefresh({r});")); }
    }
    // the label its clip is on, for its own code and for another object's that reads it
    let read_by_others = native::reads_others_label(model);
    let mut publish = written.publish.clone();
    if read_by_others || functions.iter().chain(written.start.iter()).chain(written.tick.iter()).any(|t| t.contains("clipLabel(")) || written.frames.values().any(|t| t.contains("clipLabel(")) {
        functions.push(native::helper(runtime, "fighter_tables", "clip_label", "", &[], &super::native_fighter::clip_labels(model, pieces))?);
        if read_by_others { publish.push("self.exports.__stanceLabel = clipLabel;".into()); }
    }
    let mut slots: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    slots.insert("fields", written.fields.clone());
    slots.insert("functions", functions.join("\n\n").lines().map(String::from).collect());
    if reversible { slots.insert("reflect", vec!["// turned back by a reflector".into(), "Common.enableReflectionListener({ mode: \"X\", replaceOwner: true });".into()]); }
    slots.insert("start", starts);
    slots.insert("first", first.into_iter().cloned().collect());
    slots.insert("update", update);
    slots.insert("own_start", written.start.clone());
    slots.insert("own_tick", written.tick.clone());
    slots.insert("publish", publish);
    slots.insert("ends", written.ends.iter().map(|(l, h)| format!("if ({l}.get()) {h}(null);")).collect());
    let script = native::fill_slots(skeleton, &slots)
        .replace("{title}", title)
        .replace("{classes}", &classes.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "))
        .replace("makeInt(LIFE)", &format!("makeInt({})", num(life * 2.0)));
    // (an item's handler, filled in with what it declares)
    let script = if !item { script } else {
        let n = |k: &str, d: f64| match spec.stats.get(k) { Some(Value::Num(v)) => *v, _ => d };
        let b = |k: &str, d: bool| match spec.stats.get(k) { Some(Value::Bool(v)) => *v, Some(Value::Num(v)) => *v != 0.0, _ => d };
        let words = [("TOSS", num(n("tossSpeed", units.item_toss))), ("PICKUP", b("canPickup", true).to_string()),
            ("WIDTH", num(n("width", 20.0))), ("HEIGHT", num(n("height", 20.0))), ("SIZE", num(units.size)), ("SPEED", num(units.speed)),
            ("GHOST", b("ghost", false).to_string()), ("GRAVITY", num(n("gravity", 0.0) * units.accel)),
            ("FALL_CAP", num(n("max_gravity", n("maxgravity", 0.0)) * units.speed)), ("IDLE_ANIM", compile::quote(&start))];
        let mut t = script;
        for (w, v) in words { t = crate::dyn_re(&format!(r"\b{w}\b")).replace_all(&t, v.as_str()).to_string(); }
        t
    };
    // (a first update with nothing to run)
    // (a part with nothing in it)
    let script = script.replace("\tif (!started.get()) {\n\t\tstarted.set(true);\n\t}\n", "")
        .replace("\t// what its own class runs each of the source's frames (every second engine frame)\n\tif (age.get() % 2 == 1) {\n\t}\n", "")
        .replace("\n// its end: whoever listens for it is told\nfunction onKill() {\n}\n", "");

    // its entity: each frame with code calls it, and each animation plays on into the next
    let content = crate::projectile_gen::projectile_content_id(entity_key);
    let entity_path = char_dir.join(format!("library/entities/{}.entity", crate::haxe_gen::sanitize_entity_name(&content)));
    keyframes_with(&entity_path, entity_key, model, pieces, &written.frames, true, &BTreeMap::new()).map_err(|e| e.to_string())?;
    let anim_stats = char_dir.join(format!("library/scripts/Projectile/{}AnimationStats.hx", crate::haxe_gen::snake_to_pascal(entity_key)));
    plays_on(&anim_stats, model, pieces).map_err(|e| e.to_string())?;
    Ok(script)
}

/// whether a class (and what it extends of the package's) only stops or plays its clip, sets its
/// own properties to plain values and registers its frame scripts: nothing anyone sees here
pub fn is_quiet(model: &SourceModel, runtime: &Runtime, class: &str) -> bool {
    let plain = |e: &Expr| matches!(e, Expr::Null | Expr::Undefined | Expr::Bool(_) | Expr::Num(_) | Expr::Int(_) | Expr::Str(_));
    let mut cur = model.classes.get(class);
    while let Some(c) = cur {
        for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor]) {
            let ok = f.body.iter().all(|s| match s {
                Stmt::Return(None) | Stmt::Expr(Expr::SuperCall(..)) => true,
                Stmt::Assign(Place::Member(t, _), v) => matches!(t, Expr::This) && plain(v),
                Stmt::Expr(Expr::Call(f, _)) => own_name(f).is_some_and(|m| matches!(m, "stop" | "play" | "addFrameScript")),
                _ => false,
            });
            if !ok { return false }
        }
        if runtime.provides(&c.super_name) || matches!(compile::short(&c.super_name), "MovieClip" | "Sprite") { return true }
        cur = model.classes.get(&c.super_name);
    }
    true
}

/// the name of the clip's own member a call names (`this.stop` or a bare `stop`)
fn own_name(f: &Expr) -> Option<&str> {
    match f { Expr::Member(t, m) if matches!(**t, Expr::This) => Some(m), Expr::Name(m) => Some(m), _ => None }
}

/// the animation its stats start it in (`PState.ACTIVE`)
fn start_animation(char_dir: &Path, entity_key: &str) -> Option<String> {
    let stats = char_dir.join(format!("library/scripts/Projectile/{}Stats.hx", crate::haxe_gen::snake_to_pascal(entity_key)));
    let text = std::fs::read_to_string(stats).ok()?;
    crate::re!(r#"PState\.ACTIVE\s*=>\s*\{\s*animation:\s*"([^"]+)""#).captures(&text).map(|c| c[1].to_string())
}

/// the clip frame each source frame of an animation shows: its section in the order it plays
/// (backwards for one played backwards), then the lead-in it borrows from the clip's head
fn clip_frame(p: &Piece, k: u32) -> u32 {
    if k >= p.frames { 1 + (k - p.frames) } else if p.reversed { p.from + p.frames - 1 - k } else { p.from + k }
}

/// the entity's keyframes: a "Source frames" layer on each animation, whose keyframe on each
/// source frame with code calls that frame's function, and a "Source labels" layer with the
/// clip's labels inside it (a jump to one plays the animation at the label)
/// (`born`: its first frame runs once its maker is done, as a projectile's does; `extra`: more code
/// on an animation's engine frame, by its name and the frame's index)
/// whether code's first statement (past its comments) opens with `(`: after a block's close it
/// would read as a call of the block
pub fn opens_with_paren(code: &str) -> bool {
    code.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("//")).is_some_and(|l| l.starts_with('('))
}

pub fn keyframes_with(path: &Path, key: &str, model: &SourceModel, pieces: &[Piece], frames: &BTreeMap<(u16, u32), String>,
                      born: bool, extra: &BTreeMap<(String, u32), String>) -> anyhow::Result<()> {
    let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    let mut json: serde_json::Value = serde_json::from_slice(&bytes)?;
    let lens: BTreeMap<String, u64> = {
        let kfs: BTreeMap<String, u64> = json["keyframes"].as_array().map(|a| a.iter().filter_map(|k|
            Some((k["$id"].as_str()?.to_string(), k["length"].as_u64().unwrap_or(1)))).collect()).unwrap_or_default();
        json["layers"].as_array().map(|a| a.iter().filter_map(|l| {
            let n: u64 = l["keyframes"].as_array()?.iter().filter_map(|k| kfs.get(k.as_str()?)).sum();
            Some((l["$id"].as_str()?.to_string(), n))
        }).collect()).unwrap_or_default()
    };
    let mut new_keyframes = Vec::new();
    let mut new_layers = Vec::new();
    let mut layers_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let id = |s: String| crate::uuid_gen::det_uuid(&s);
    for p in pieces {
        let Some(anim) = json["animations"].as_array().and_then(|a| a.iter().find(|x| x["name"] == p.fm_name.as_str())) else { continue };
        let total = anim["layers"].as_array().map(|ls| ls.iter().filter_map(|l| lens.get(l.as_str()?)).copied().max().unwrap_or(0)).unwrap_or(0) as u32;
        if total == 0 { continue }
        // its frames with code, by engine frame (two to a source frame)
        let mut at: Vec<(u32, String)> = Vec::new();
        for k in 0..(p.frames + p.head) {
            let f = clip_frame(p, k);
            let more = extra.get(&(p.fm_name.clone(), 2 * k));
            let call = frames.get(&(p.sym, f));
            if call.is_none() && more.is_none() { continue }
            // the first frame as the object is made runs once its maker is done (`update`)
            let mut code = match call { Some(c) if born && k == 0 && f == 1 => format!("if (started.get()) {c}"), Some(c) => c.clone(), None => String::new() };
            if let Some(m) = more {
                // (a block's close followed by a line opening with `(` reads as a call of the block)
                if code.trim_end().ends_with('}') && opens_with_paren(m) { code.push(';'); }
                if !code.is_empty() { code.push('\n'); }
                code.push_str(m);
            }
            if 2 * k < total { at.push((2 * k, code)); }
        }
        let tag = format!("{key}::{}::native", p.fm_name);
        if !at.is_empty() {
            let mut ids = Vec::new();
            let mut cursor = 0u32;
            for (i, code) in at {
                if i > cursor {
                    let g = id(format!("{tag}::gap{cursor}"));
                    new_keyframes.push(serde_json::json!({ "$id": g, "type": "FRAME_SCRIPT", "length": i - cursor, "code": "", "pluginMetadata": {} }));
                    ids.push(g);
                }
                let k = id(format!("{tag}::{i}"));
                new_keyframes.push(serde_json::json!({ "$id": k, "type": "FRAME_SCRIPT", "length": 1, "code": code, "pluginMetadata": {} }));
                ids.push(k);
                cursor = i + 1;
            }
            if cursor < total {
                let t = id(format!("{tag}::tail"));
                new_keyframes.push(serde_json::json!({ "$id": t, "type": "FRAME_SCRIPT", "length": total - cursor, "code": "", "pluginMetadata": {} }));
                ids.push(t);
            }
            let layer = id(format!("{tag}::layer"));
            new_layers.push(serde_json::json!({ "$id": layer, "name": "Source frames", "type": "FRAME_SCRIPT", "keyframes": ids, "hidden": false, "locked": false, "pluginMetadata": {} }));
            layers_of.entry(p.fm_name.clone()).or_default().push(layer);
        }
        // the clip's labels inside the animation
        if !p.reversed {
            let mut marks: Vec<(u32, String)> = model.sprites.get(&p.sym).map(|s| s.labels.iter()
                .map(|(f0, l)| (*f0 as u32 + 1, l.clone()))
                .filter(|(f, _)| *f > p.from && *f < p.from + p.frames)
                .map(|(f, l)| (2 * (f - p.from), l)).collect()).unwrap_or_default();
            if !marks.is_empty() {
                let shadowed = marks.iter().any(|(_, l)| *l == p.fm_name);
                marks.insert(0, (0, if shadowed { String::new() } else { p.fm_name.clone() }));
                marks.retain(|(i, _)| *i < total);
                let ltag = format!("{key}::{}::labels", p.fm_name);
                let mut lids = Vec::new();
                for (n, (at, name)) in marks.iter().enumerate() {
                    let next = marks.get(n + 1).map(|(i, _)| *i).unwrap_or(total);
                    let l = id(format!("{ltag}::{at}"));
                    new_keyframes.push(serde_json::json!({ "$id": l, "type": "LABEL", "length": (next - at).max(1), "name": name, "pluginMetadata": {} }));
                    lids.push(l);
                }
                let lid = id(format!("{ltag}::layer"));
                new_layers.push(serde_json::json!({ "$id": lid, "name": "Source labels", "type": "LABEL", "keyframes": lids, "hidden": false, "locked": false, "pluginMetadata": {} }));
                layers_of.entry(p.fm_name.clone()).or_default().push(lid);
            }
        }
    }
    // a conversion run again replaces what it wrote before
    let old: BTreeSet<String> = json["layers"].as_array().map(|a| a.iter()
        .filter(|l| l["name"] == "Source frames" || l["name"] == "Source labels" || (!born && l["type"] == "FRAME_SCRIPT")).filter_map(|l| l["$id"].as_str().map(String::from)).collect()).unwrap_or_default();
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
                for l in layers_of.get(&name).into_iter().flatten() { ls.push(serde_json::Value::String(l.clone())); }
            }
        }
    }
    std::fs::write(path, serde_json::to_string_pretty(&json)?)?;
    Ok(())
}

/// each animation and the one that shows its clip's next frame, which the clip plays on into
fn next_pieces<'a>(model: &SourceModel, pieces: &'a [Piece]) -> Vec<(&'a Piece, &'a Piece)> {
    pieces.iter().filter(|p| !p.borrowed && !p.reversed && p.head == 0).filter_map(|p| {
        let total = model.sprites.get(&p.sym).map(|s| s.frames.len() as u32).unwrap_or(0);
        let next = if p.from + p.frames > total { 1 } else { p.from + p.frames };
        pieces.iter().find(|q| !q.borrowed && q.sym == p.sym && next >= q.from && next < q.from + q.frames).map(|q| (p, q))
    }).collect()
}

/// the animations that play on into another, by name (the projectile's update plays it: on a
/// projectile the engine's own end of an animation goes by its state, not `nextAnimation`)
fn plays_into(model: &SourceModel, pieces: &[Piece]) -> Vec<(String, String)> {
    next_pieces(model, pieces).into_iter().filter(|(p, q)| q.fm_name != p.fm_name).map(|(p, q)| (p.fm_name.clone(), q.fm_name.clone())).collect()
}

/// one that shows its own next frame again loops; one that plays on into another holds its last
/// frame for the update to move it on
fn plays_on(path: &Path, model: &SourceModel, pieces: &[Piece]) -> anyhow::Result<()> {
    let Ok(mut text) = std::fs::read_to_string(path) else { return Ok(()) };
    for (p, q) in next_pieces(model, pieces) {
        if q.fm_name != p.fm_name { continue }
        let row = format!("{}: {{ endType: AnimationEndType.NONE", p.fm_name);
        text = text.replace(&row, &format!("{}: {{ endType: AnimationEndType.LOOP", p.fm_name));
    }
    std::fs::write(path, text)?;
    Ok(())
}

fn num(n: f64) -> String { if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") } }
