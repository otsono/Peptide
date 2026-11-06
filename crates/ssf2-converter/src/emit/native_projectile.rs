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
    /// each of the owner's animations, and the attack (its clip's label) it shows
    pub anims: Vec<(String, String)>,
}

/// The projectile's script and entity, written natively; or why it stays on the runtime.
pub fn write(char_dir: &Path, model: &SourceModel, runtime: &Runtime, spec: &ObjectSpec, decls: &Decls,
             pieces: &[Piece], units: Units, title: &str, entity_key: &str, owner: &Owner) -> Result<String, String> {
    let skeleton = runtime.native_files.get("projectile").ok_or("the runtime has no native/projectile.hx")?;
    if compile::short(&spec.class) != "PeptideProjectile" { return Err(format!("its class {} is its own", spec.class)) }
    let item = decls.items.entries().iter().any(|(n, d)| n == title || d.get("linkage_id").and_then(|v| v.str()) == Some(spec.clip.as_str()));
    if item { return Err("an item, which its holder moves".into()) }
    let clip_sym = spec.clip_sym.or_else(|| model.symbol_of(&spec.clip)).ok_or("its clip is no symbol")?;
    if pieces.is_empty() { return Err("no animations".into()) }

    // what the source's engine does for it each frame that has no plain form here yet
    for k in ["bounce", "refreshRate", "controlDirection", "homingSpeed"] {
        if spec.stats.get(k).is_some_and(|v| matches!(v, Value::Num(n) if *n > 0.0) || matches!(v, Value::Bool(true))) {
            return Err(format!("declares {k}"));
        }
    }

    // the classes whose code it runs: the classes of the clips its animations show. any other clip
    // inside it with code of its own keeps it on the runtime
    let shown: BTreeSet<u16> = pieces.iter().map(|p| p.sym).collect();
    let mut classes = Vec::new();
    for sym in clip_tree(model, Some(clip_sym)) {
        let Some(c) = model.class_of(sym) else { continue };
        if runtime.provides(&c.name) { continue }
        if shown.contains(&sym) { classes.push(c) }
        else if !quiet(model, runtime, &c.name) { return Err(format!("{} inside it has code of its own", c.name)) }
    }
    if classes.is_empty() { return Err("no code of its own".into()) }
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
        if cases.is_empty() { return None }
        let name = format!("ownerAttack_{}", compile::ident(k));
        let mut def = format!("// what the attack its owner is in declares for its {k}, by the animation that shows it\nfunction {name}(owner) {{\n\tswitch (owner.getAnimation()) {{\n");
        for (v, anims) in &cases { def.push_str(&format!("\t\tcase {}: return {v};\n", anims.join(", "))); }
        def.push_str("\t}\n\treturn null;\n}");
        Some((name, def))
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
    let jumps = label_jumps(model, pieces);
    let object = native::Object {
        model, classes: classes.clone(), jumps: &jumps, runtime, declared: &declared, switches: &taken.switches,
        owner_stat: &owner_stat, effect: &made, size: units.size, speed: units.speed, accel: units.accel,
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
    let mut slots: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    slots.insert("fields", written.fields.clone());
    slots.insert("functions", functions.join("\n\n").lines().map(String::from).collect());
    if reversible { slots.insert("reflect", vec!["// turned back by a reflector".into(), "Common.enableReflectionListener({ mode: \"X\", replaceOwner: true });".into()]); }
    slots.insert("start", taken.starts.clone());
    slots.insert("first", first.into_iter().cloned().collect());
    slots.insert("update", taken.frame.iter().cloned().collect());
    let script = fill(skeleton, &slots)
        .replace("{title}", title)
        .replace("{classes}", &classes.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "))
        .replace("makeInt(LIFE)", &format!("makeInt({})", num(life * 2.0)));
    // (a first update with nothing to run)
    let script = script.replace("\tif (!started.get()) {\n\t\tstarted.set(true);\n\t}\n", "");

    // its entity: each frame with code calls it, and each animation plays on into the next
    let content = crate::projectile_gen::projectile_content_id(entity_key);
    let entity_path = char_dir.join(format!("library/entities/{}.entity", crate::haxe_gen::sanitize_entity_name(&content)));
    keyframes(&entity_path, entity_key, model, pieces, &written.frames).map_err(|e| e.to_string())?;
    let anim_stats = char_dir.join(format!("library/scripts/Projectile/{}AnimationStats.hx", crate::haxe_gen::snake_to_pascal(entity_key)));
    plays_on(&anim_stats, model, pieces).map_err(|e| e.to_string())?;
    Ok(script)
}

/// whether a class (and what it extends of the package's) only stops or plays its clip, sets its
/// own properties to plain values and registers its frame scripts: nothing anyone sees here
fn quiet(model: &SourceModel, runtime: &Runtime, class: &str) -> bool {
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
fn keyframes(path: &Path, key: &str, model: &SourceModel, pieces: &[Piece], frames: &BTreeMap<(u16, u32), String>) -> anyhow::Result<()> {
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
            let Some(call) = frames.get(&(p.sym, f)) else { continue };
            // the first frame as the object is made runs once its maker is done (`update`)
            let code = if k == 0 && f == 1 { format!("if (started.get()) {call}") } else { call.clone() };
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
                for l in layers_of.get(&name).into_iter().flatten() { ls.push(serde_json::Value::String(l.clone())); }
            }
        }
    }
    std::fs::write(path, serde_json::to_string_pretty(&json)?)?;
    Ok(())
}

/// each animation plays on into the one that shows its clip's next frame, as the clip does
/// (`nextAnimation`); one that shows its own next frame again loops
fn plays_on(path: &Path, model: &SourceModel, pieces: &[Piece]) -> anyhow::Result<()> {
    let Ok(mut text) = std::fs::read_to_string(path) else { return Ok(()) };
    for p in pieces.iter().filter(|p| !p.borrowed && !p.reversed && p.head == 0) {
        let total = model.sprites.get(&p.sym).map(|s| s.frames.len() as u32).unwrap_or(0);
        let next = if p.from + p.frames > total { 1 } else { p.from + p.frames };
        let Some(q) = pieces.iter().find(|q| !q.borrowed && q.sym == p.sym && next >= q.from && next < q.from + q.frames) else { continue };
        let end = if q.fm_name == p.fm_name { "endType: AnimationEndType.LOOP".to_string() }
            else { format!("endType: AnimationEndType.AUTO, nextAnimation: {}", compile::quote(&q.fm_name)) };
        let row = format!("{}: {{ endType: AnimationEndType.NONE", p.fm_name);
        text = text.replace(&row, &format!("{}: {{ {end}", p.fm_name));
    }
    std::fs::write(path, text)?;
    Ok(())
}

/// the skeleton with each `// {slot}` line replaced by the slot's lines, at that line's indent
/// (an empty slot takes its line out)
fn fill(skeleton: &str, slots: &BTreeMap<&str, Vec<String>>) -> String {
    let mut out = String::new();
    for line in skeleton.lines() {
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

fn num(n: f64) -> String { if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") } }
