//! A stage's scripts: the class its package registers as its stage, written as plain Fraymakers
//! code on `runtime/native/stage.hx` with its boot as calls to `stage_boot.hx`'s pieces; each
//! object class with code of its own on `object.hx` (an item's on `item.hx`); and each projectile
//! they fire. Written over the stage pipeline's scripts, after it has written everything else (art,
//! entities, the plan the verify gates read), so only the scripts change.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::stage_tables::StageTables;
use super::{entity_animations, Decls, ObjectSpec, Piece, Units};
use crate::compile::{self, Runtime};
use crate::model::ast::{Expr, Place, Stmt};
use crate::model::declare::Value;
use crate::model::SourceModel;

/// What the stage pipeline hands the stage's scripts: what it worked out, as data.
pub struct StageInput<'a> {
    /// the stage's own track (`ssf2_music`), what its code reads as the music playing
    pub music: Option<String>,
    /// where the stage's light starts, when its code moves it with the fighters (this engine's
    /// units): the stage draws their shadows from it (`followLight`)
    pub follow_light: Option<(f64, f64)>,
    /// the stage's floors as runs [x0, y0, x1, y1] (this engine's units): where those shadows land
    pub floors: Vec<[f64; 4]>,
    /// the boot (`DATA.stage.boot`)
    pub tables: &'a StageTables,
    /// the stage entity's animation that shows the stage's own art
    pub anim: &'a str,
    /// the size the camera-background pan containers centre on
    pub pan: (f64, f64),
    /// each object class the stage makes by class, by the content it was converted to
    pub objects: Vec<(String, String)>,
    /// the named surfaces the stage entity carries: each name, and its structures' names
    pub surfaces: &'a [(String, Vec<String>)],
    /// the pieces that declare their own ground friction, and the ones that declare a push (rows)
    pub frictions: Vec<String>,
    pub pushes: Vec<String>,
    /// each weather particle's clip by its library name, and the effect drawn for it
    pub particles: Vec<(String, String)>,
    /// how many of one particle are drawn at once
    pub particle_cap: u32,
    /// the clips the stage's code makes from the library: each name, and its pieces (rows)
    pub lib: Vec<(String, String)>,
    /// where the tree puts each placed object at a label of its plane (rows)
    pub placed_at: String,
    /// where the source's origin sits in this stage's space
    pub origin: (f64, f64),
    /// the objects the stage declares that have a class of their own
    pub rows: Vec<ObjectRow>,
}

/// One object the stage declares with a class of its own, as the stage pipeline emitted it.
pub struct ObjectRow {
    /// its place in the stage's list of objects (the boot's index)
    pub index: usize,
    /// the content it was converted to
    pub id: String,
    /// its class, as the package names it
    pub class: String,
    /// the clips that name it, nearest first: the one its class states for itself, the one the
    /// stage hands it
    pub clips: Vec<String>,
    /// the stage's own code makes it: the boot leaves it to that code
    pub by_code: bool,
    /// where the tree placed it (this engine's units), when the tree placed it: it is there as it
    /// is made, before its own code first asks where it is
    pub at: Option<(f64, f64)>,
    /// where its entity's centre is from its clip's registration (this engine's units), when its
    /// entity is drawn centred on it: the entity moves there, so the object stands where its code
    /// places the clip
    pub registration: Option<(f64, f64)>,
    /// the body the tree placed it as, when it was placed as one
    pub body: Option<Body>,
    /// the name the tree placed its clip under, where it placed it under one
    pub instance: Option<String>,
    /// where it stands from where its clip is registered (this engine's units), where that differs
    pub anchor: Option<(f64, f64)>,
    /// a placed body whose clip's name names another clip too: where its clip is registered is not known
    pub anchor_unknown: bool,
}

/// A placed body: the box its collision clip declares and the outline the clip draws (this
/// engine's units, the object's own space), and which ends of its floor are ledges.
pub struct Body {
    pub bounds: (f64, f64, f64, f64),
    pub faces: Vec<(&'static str, Vec<(f64, f64)>)>,
    pub ledges: (bool, bool),
}

/// The stage's script at `scripts/stage/<id>Script.hx`, then each classed object's and each
/// projectile's over the stand-in the stage pipeline left. Returns what the stage's script is, the
/// objects written, and the clips the stage's class sends to a frame; an error when any of them
/// cannot be written, since nothing else writes them.
pub fn stage(model: &crate::stage_parser::StageModel, dir: &Path, input: &StageInput) -> anyhow::Result<(Vec<String>, BTreeSet<String>)> {
    let id = &model.id;
    let located = Runtime::locate().ok_or_else(|| anyhow::anyhow!("runtime/ (the API tables and native skeletons) is not where the converter looks for it"))?;
    let runtime = Runtime::load(&located);
    let (src, renamed) = runtime.read_package(&std::fs::read(&model.source)?)?;
    let (class, _, sym) = src.stage()
        .ok_or_else(|| anyhow::anyhow!("the package registers no stage class with a clip of its own"))?;
    // each object's class is read from the package that has it: the stage's own, or one the game
    // keeps loaded alongside it (`lent_packages`)
    let lent = lent_packages(&runtime, &src, &input.rows);
    let models: Vec<&SourceModel> = std::iter::once(&src).chain(&lent).collect();
    // (a method of a class a lent package carries is one its objects publish, for the stage's code to call)
    // (and one of the stage's own package, for an object a lent package carries to call on the stage)
    if let Ok(mut l) = compile::native::lent_methods().lock() { for m in &models { for c in m.classes.values() { l.extend(c.methods.iter().map(|x| x.name.clone())); } } }
    // (the fields a lent package's classes keep, and the names the stage's code reads or writes on
    // its objects: what those objects publish for it)
    if let Ok(mut l) = compile::native::lent_fields().lock() { for m in &lent { for c in m.classes.values() { l.extend(c.fields.iter().filter(|f| !f.is_static).map(|f| f.name.clone())); } } }
    if let Ok(mut a) = compile::native::asked_fields().lock() {
        for c in src.classes.values() {
            for b in c.methods.iter().map(|m| &m.function.body).chain([&c.constructor.body]) {
                crate::model::ast::walk_exprs(b, &mut |e| if let crate::model::ast::Expr::Member(o, n) = e { if !matches!(**o, crate::model::ast::Expr::This) { a.insert(n.clone()); } });
            }
        }
    }
    for r in &input.rows { object_files(home(&models, &r.class), dir, r)?; }
    // the stage's own art is one animation of its entity, showing the stage clip's frames
    let entity = dir.join("library").join("entities").join(format!("{id}.entity"));
    let frames = src.sprites.get(&sym).map(|s| s.frame_count as u32).unwrap_or(1).max(1);
    let pieces: Vec<Piece> = entity_animations(&entity).contains(input.anim).then(|| Piece {
        fm_name: input.anim.to_string(), label: String::new(), sym, from: 1, frames, head: 0,
        reversed: false, borrowed: false, item_box: Vec::new(), on: None,
    }).into_iter().collect();
    let scaling = &crate::mappings::character_stats().scaling;
    let units = Units { size: model.scale, speed: scaling.velocity_scale(), item_toss: scaling.item_toss_speed, toss_angle: scaling.item_toss_angle, toss_frame: scaling.item_toss_frame, accel: scaling.accel_scale() };
    let none = Value::Null;
    // what the stage's classes declare they fire, by the name their code fires each under: every
    // script of the stage finds them there (`DATA.pkg.projectiles`)
    let projectiles = declared_projectiles(&models, &runtime, std::iter::once(class.as_str()).chain(input.rows.iter().map(|r| r.class.as_str())));
    let decls = Decls { attacks: &none, projectiles: &projectiles, items: &none, objects: input.objects.clone() };
    let scripts = dir.join("library").join("scripts").join("stage");
    // each object, run by its own class
    let mut reports = serde_json::Map::new();
    let mut written = Vec::new();
    // (each library clip's measured extent, for a floor an object makes the shape of one)
    let mut bounds: BTreeMap<String, (f64, f64, f64, f64)> = model.clip_bounds.iter().map(|(n, r)| (n.clone(), (r.x, r.y, r.w, r.h))).collect();
    // (and each a stage object's class shapes its body from, measured in the package that carries it)
    for hz in &model.hazards { for (n, r) in &hz.platform_shapes { bounds.entry(n.clone()).or_insert(*r); } }
    // (and each child of its own clip a stage object's class reads the box of, under its class's name)
    for hz in &model.hazards {
        let Some(k) = hz.class_name.as_deref() else { continue };
        for (n, r) in &hz.child_rects { bounds.entry(format!("{}>{n}", compile::short(k))).or_insert(*r); }
    }
    // (one whose class it cannot write is left out, with why: the rest of the stage is written)
    let mut left_out: Vec<String> = Vec::new();
    for row in &input.rows {
        match native_object(home(&models, &row.class), &runtime, dir, row, units, &bounds, &projectiles, input.origin, &input.objects) {
            Ok(gaps) => {
                written.push(row.id.clone());
                reports.insert(row.id.clone(), serde_json::json!({ "gaps": gaps }));
            }
            Err(why) => {
                left_out.push(row.id.clone());
                reports.insert(row.id.clone(), serde_json::json!({ "gaps": [format!("not written: {}", why.join("; "))] }));
            }
        }
    }
    // each projectile they fire, run by its own class too
    let mut fired = serde_json::Map::new();
    for (key, result) in stage_projectiles(&models, &runtime, dir, &decls, units, input.origin) {
        let o = result.map_err(|e| anyhow::anyhow!("the projectile {key}: {e}"))?;
        fired.insert(key, serde_json::json!({ "gaps": o.gaps, "unplaced": o.unplaced }));
    }
    // (each weather particle's clip class, as its effect and the size of its clip)
    let particles: BTreeMap<String, (String, f64, f64)> = input.particles.iter().map(|(n, id)| {
        let r = model.clip_bounds.get(n);
        (compile::short(n).to_string(), (id.clone(), r.map(|r| r.w).unwrap_or(0.0), r.map(|r| r.h).unwrap_or(0.0)))
    }).collect();
    // (its respawn beacons, in the source's units and stage space, for its code to read)
    let spots: Vec<String> = model.respawns.iter().map(|p| format!("{{ x: {:.2}, y: {:.2} }}", p.x / units.size + input.origin.0, p.y / units.size + input.origin.1)).collect();
    STAGE_SPAWNS.with(|c| *c.borrow_mut() = format!("[{}]", spots.join(", ")));
    let (script, mut gaps) = native_stage(&src, &runtime, &class, sym, &pieces, units, input, id, &regions_literal(model), &particles)
        .map_err(|why| anyhow::anyhow!("{}", why.join("; ")))?;
    for o in &left_out { gaps.push(format!("its object {o} is not written (its gaps say why)")); }
    std::fs::write(scripts.join(format!("{id}Script.hx")), &script)?;
    let report = serde_json::json!({
        "gaps": gaps,
        "unplaced": renamed,
        "objects": reports,
        "projectiles": fired,
    });
    std::fs::write(dir.join("port_report.json"), serde_json::to_string_pretty(&report)?)?;
    let played = sent_to_a_frame(&src, &class);
    Ok((written, played))
}

/// The clips a class sends to a frame by name (`platformHolder.gotoAndStop(3)`): the frame such a
/// clip shows is the code's to pick, not its own timeline's.
fn sent_to_a_frame(src: &SourceModel, class: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let chain = std::iter::once(class.to_string()).chain(src.supers(class));
    for c in chain.filter_map(|n| class_of(src, &n)) {
        for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor]) {
            crate::model::ast::walk_exprs(&f.body, &mut |e| {
                let Expr::Call(callee, _) = e else { return };
                let Expr::Member(on, m) = callee.as_ref() else { return };
                if !matches!(m.as_str(), "gotoAndStop" | "gotoAndPlay") { return }
                if let Expr::Member(_, n) | Expr::Name(n) = on.as_ref() { out.insert(n.clone()); }
            });
        }
    }
    out
}

/// What one object's script came to: what could not be written, the names nothing answers, and
/// the animations no source frames are found for.
struct ObjectOut {
    gaps: Vec<String>,
    unplaced: Vec<String>,
}

/// A class of the package's own, by its full or short name.
fn class_of<'a>(src: &'a SourceModel, name: &str) -> Option<&'a crate::model::ClassModel> {
    src.classes.get(name).or_else(|| {
        let mut hits = src.classes.values().filter(|c| compile::short(&c.name) == name);
        let first = hits.next()?;
        hits.next().is_none().then_some(first)
    })
}

/// The packages the game keeps loaded alongside a stage that hold the classes of the stage's
/// objects it does not carry itself: a stage can make an object of a class that lives in one of
/// them (`shared_packages`), and that object runs its class from there.
fn lent_packages(runtime: &Runtime, src: &SourceModel, rows: &[ObjectRow]) -> Vec<SourceModel> {
    let mut wanted: BTreeSet<&str> = rows.iter().map(|r| r.class.as_str()).filter(|c| class_of(src, c).is_none()).collect();
    let mut out = Vec::new();
    for pkg in crate::shared_packages::packages() {
        if wanted.is_empty() { break }
        let Ok((m, _)) = runtime.read_package(&pkg.swf) else { continue };
        let has: Vec<&str> = wanted.iter().copied().filter(|c| class_of(&m, c).is_some()).collect();
        if has.is_empty() { continue }
        for c in has { wanted.remove(c); }
        out.push(m);
    }
    out
}

/// The package a class is read from: the first of the stage's packages that has it, else the
/// stage's own.
fn home<'a>(models: &[&'a SourceModel], class: &str) -> &'a SourceModel {
    models.iter().copied().find(|m| class_of(m, class).is_some()).unwrap_or(models[0])
}


/// What some of the package's classes declare they fire (`getProjectileStats`, the nearest in
/// each one's chain), merged by name: the first class to declare a name keeps it.
fn declared_projectiles<'a>(models: &[&SourceModel], runtime: &Runtime, classes: impl Iterator<Item = &'a str>) -> Value {
    let mut out: Vec<(String, Value)> = Vec::new();
    for class in classes {
        let src = home(models, class);
        let Some(c) = class_of(src, class) else { continue };
        let pkg = compile::Package::new(src, runtime);
        let declared = pkg.chain(&c.name).0.into_iter()
            .find_map(|k| k.methods.iter().find(|m| m.name == "getProjectileStats" && !m.is_static))
            .and_then(|m| crate::model::declare::returned(&m.function.body));
        for (name, decl) in declared.as_ref().map(|v| v.entries()).unwrap_or_default() {
            if !out.iter().any(|(n, _)| n == name) { out.push((name.clone(), decl.clone())) }
        }
    }
    // ...and a projectile some code fires by its class (`spawnProjectile(CharmanderFire, owner)`):
    // its own stats are its declaration, under its class's name, and it runs that class
    for src in models.iter().copied() {
    let mut fired: BTreeSet<String> = BTreeSet::new();
    for c in src.classes.values() {
        for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor]) {
            crate::model::ast::walk_exprs(&f.body, &mut |e| {
                if let crate::model::ast::Expr::Call(callee, args) = e {
                    let spawn = matches!(&**callee, crate::model::ast::Expr::Member(_, m) | crate::model::ast::Expr::Name(m) if m == "spawnProjectile");
                    if let (true, Some(crate::model::ast::Expr::Name(x))) = (spawn, args.first()) { fired.insert(x.clone()); }
                }
            });
        }
    }
    for x in fired {
        if out.iter().any(|(n, _)| *n == x) || class_of(src, &x).is_none() { continue }
        let Some(cls) = class_of(src, &x) else { continue };
        let own = std::iter::once(cls.name.clone()).chain(src.supers(&cls.name))
            .filter_map(|n| class_of(src, &n))
            // (the model reads the source's `getOwnStats` under the converter's name for it)
            .find_map(|k| k.methods.iter().find(|m| (m.name == "ownStats" || m.name == "getOwnStats") && !m.is_static))
            .and_then(|m| crate::model::declare::returned(&m.function.body));
        let Some(Value::Obj(mut fields)) = own else { continue };
        if !fields.iter().any(|(k, _)| k == "classAPI") { fields.push(("classAPI".to_string(), Value::Class(x.clone()))); }
        out.push((x, Value::Obj(fields)));
    }
    }
    if out.is_empty() { Value::Null } else { Value::Obj(out) }
}

/// Each projectile the stage's classes declare, over the script the stage pipeline wrote for it
/// at `scripts/Projectile/`: its class (the one its declaration names, else the API's projectile),
/// on the clip its declaration names, with the animations its entity has. Returned by the clip it
/// draws.
#[allow(clippy::too_many_arguments)]
fn stage_projectiles(models: &[&SourceModel], runtime: &Runtime, dir: &Path, decls: &Decls,
                       units: Units, origin: (f64, f64)) -> Vec<(String, anyhow::Result<ObjectOut>)> {
    let lib = dir.join("library");
    // the pieces the stage pipeline cut for each projectile's entity, by the clip it draws
    let mut pieces_of = crate::entity_gen::take_projectile_pieces();
    let mut out = Vec::new();
    for (name, decl) in decls.projectiles.entries() {
        let Some(linkage) = decl.get("linkage_id").and_then(|v| v.str()) else { continue };
        // read from the package that has its class, else the one that has its clip
        let class = decl.get("classAPI").and_then(|v| v.class());
        let src = class.filter(|c| models.iter().any(|m| class_of(m, c).is_some())).map(|c| home(models, c))
            .or_else(|| models.iter().copied().find(|m| m.symbol_of(linkage).is_some())).unwrap_or(models[0]);
        // one script per entity, from the first declaration that draws with it
        let Some(key) = pieces_of.keys().find(|k| k.eq_ignore_ascii_case(linkage)).cloned() else { continue };
        let pieces = pieces_of.remove(&key).unwrap_or_default();
        let content = crate::projectile_gen::projectile_content_id(&key);
        let entity = lib.join("entities").join(format!("{}.entity", crate::haxe_gen::sanitize_entity_name(&content)));
        let script = lib.join("scripts").join("Projectile").join(format!("{}Script.hx", crate::haxe_gen::snake_to_pascal(&key)));
        let result = (|| -> anyhow::Result<ObjectOut> {
            if !entity.exists() || !script.exists() { anyhow::bail!("no entity and script were written for it") }
            let has = entity_animations(&entity);
            let (pieces, missing): (Vec<Piece>, Vec<Piece>) = pieces.into_iter().partition(|p| has.contains(&p.fm_name));
            let effects = BTreeMap::new();
            let entities = BTreeMap::new();
            let spec = ObjectSpec {
                effects: &effects,
                entities: &entities,
                class: class.unwrap_or("PeptideProjectile").to_string(),
                clip: linkage.to_string(),
                clip_sym: None,
                stats: decl.clone(),
            };
            // written as plain Fraymakers code, its owner the stage object that fires it
            let owner = super::native_projectile::Owner { attacks: &Value::Null, stats: &Value::Null, anims: Vec::new(), native: true, origin };
            let (text, gaps) = super::native_projectile::write(dir, src, runtime, &spec, decls, &pieces, units, name, &key, &owner, false)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            std::fs::write(&script, text)?;
            Ok(ObjectOut { gaps,
                unplaced: missing.iter().map(|p| format!("{} has no such animation", p.fm_name)).collect() })
        })();
        out.push((key, result));
    }
    out
}

/// Where an object's entity and script are, and the clip that is its art: what its script is
/// written from, or why it cannot be.
fn object_files(src: &SourceModel, dir: &Path, row: &ObjectRow) -> anyhow::Result<(std::path::PathBuf, std::path::PathBuf, Option<u16>)> {
    let class = class_of(src, &row.class)
        .ok_or_else(|| anyhow::anyhow!("its object {} is of a class from another package ({})", row.id, row.class))?;
    let lib = dir.join("library");
    let entity = lib.join("entities").join(format!("{}.entity", row.id));
    let script = lib.join("scripts").join("hazard").join(format!("{}Script.hx", row.id));
    if !entity.exists() || !script.exists() {
        anyhow::bail!("no entity and script were written for its object {}", row.id);
    }
    // an object with no clip (a zone the code sizes and places) runs on an empty one
    let root = row.clips.iter().find_map(|n| src.symbol_of(n))
        .or_else(|| clip_naming(src, &class.name))
        .or(class.symbol);
    Ok((entity, script, root))
}

/// The clip whose own frame script names a class as the one the game builds over it (`classAPI`):
/// a collision clip the tree places is that class's object.
fn clip_naming(src: &SourceModel, class: &str) -> Option<u16> {
    fn names(stmts: &[Stmt], class: &str) -> bool {
        stmts.iter().any(|s| {
            let assigns = matches!(s, Stmt::Assign(Place::Member(_, m) | Place::Name(m), v)
                if m == "classAPI" && matches!(strip(v), Expr::Name(n) | Expr::Member(_, n) if compile::short(n) == class));
            assigns || s.blocks().iter().any(|b| names(b, class))
        })
    }
    fn strip(e: &Expr) -> &Expr {
        match e { Expr::Coerce(_, x) | Expr::Convert(_, x) => strip(x), _ => e }
    }
    let short = compile::short(class);
    src.classes.values().filter(|c| !c.frame_scripts.is_empty()).find_map(|c| {
        let named = c.frame_scripts.iter().filter_map(|fs| c.methods.iter().find(|m| m.name == fs.method))
            .any(|m| names(&m.function.body, short));
        if named { c.symbol.or_else(|| src.symbol_of(&c.name)) } else { None }
    })
}

/// An object's pieces: each animation its entity has, as the frames of its clip under the label
/// the animation is named for (the label's `stance` when the clip places one there), and the
/// first animation as the clip's own first frames when no label names it. Returns too the
/// animations no frames were found for.
fn object_pieces(src: &SourceModel, entity: &Path, root: u16) -> (Vec<Piece>, Vec<String>) {
    let anims = entity_lengths(entity);
    let sprite = src.sprites.get(&root);
    let labels: Vec<(usize, String)> = sprite.map(|s| s.labels.clone()).unwrap_or_default();
    // where the section of the clip from frame `f0` shows its frames: the `stance` it places there,
    // else the clip itself, and how many frames that runs before the next section or the end
    let place = |f0: usize| -> (u16, u32, u32) {
        let stance = sprite.and_then(|s| s.frames.get(f0))
            .and_then(|placed| placed.iter().find(|p| p.name.as_deref() == Some("stance")))
            .map(|p| p.char_id).filter(|c| src.sprites.contains_key(c));
        match stance {
            Some(s) => (s, 1, src.sprites.get(&s).map(|x| x.frame_count as u32).unwrap_or(1)),
            None => {
                let total = sprite.map(|s| s.frame_count).unwrap_or(1);
                let next = labels.iter().map(|(f, _)| *f).filter(|f| *f > f0).min().unwrap_or(total);
                (root, f0 as u32 + 1, next.saturating_sub(f0) as u32)
            }
        }
    };
    // the stances the clip's sections place: a label of one of them is a section of its own (a
    // piranha plant's stance runs `loop`, `attack` and `die`)
    let stances: Vec<u16> = sprite.map(|sp| sp.frames.iter().flatten()
        .filter(|p| p.name.as_deref() == Some("stance") && src.sprites.contains_key(&p.char_id))
        .map(|p| p.char_id).collect::<BTreeSet<u16>>().into_iter().collect()).unwrap_or_default();
    // a stance section's animation is named under the clip's label that places the stance
    // (`<label>_<stance label>`), or by the stance label alone
    let section_stance = |f0: usize| -> Option<u16> {
        sprite.and_then(|sp| sp.frames.get(f0)).and_then(|placed| placed.iter()
            .find(|p| p.name.as_deref() == Some("stance") && src.sprites.contains_key(&p.char_id)).map(|p| p.char_id))
    };
    let in_stance = |name: &str| -> Option<(u16, u32, u32, String)> {
        let found = |st: u16, l_ok: &dyn Fn(&str) -> bool| -> Option<(u16, u32, u32, String)> {
            let sp = src.sprites.get(&st)?;
            let (f0, l) = sp.labels.iter().find(|(_, l)| l_ok(l))?;
            let next = sp.labels.iter().map(|(f, _)| *f).filter(|f| f > f0).min().unwrap_or(sp.frame_count);
            Some((st, *f0 as u32 + 1, next.saturating_sub(*f0) as u32, l.clone()))
        };
        labels.iter().find_map(|(f0, root)| {
            let st = section_stance(*f0)?;
            found(st, &|l| anim_name(&format!("{root}_{l}")) == name)
        }).or_else(|| stances.iter().find_map(|st| found(*st, &|l| anim_name(l) == name)))
    };
    let mut pieces = Vec::new();
    let mut unplaced = Vec::new();
    for (k, (name, len)) in anims.iter().enumerate() {
        if *len == 0 { continue }
        let root_label = labels.iter().any(|(_, l)| anim_name(l) == *name);
        if !root_label {
            if let Some((sym, from, span, label)) = in_stance(name) {
                let frames = (*len / 2).min(span.max(1)).max(1);
                pieces.push(Piece { fm_name: name.clone(), label, sym, from, frames, head: 0, reversed: false, borrowed: false, item_box: Vec::new(), on: None });
                continue;
            }
        }
        // the section a label names, else the first animation as the clip's first frame
        let at = labels.iter().find(|(_, l)| anim_name(l) == *name).cloned()
            .or_else(|| (k == 0).then(|| (0, labels.iter().find(|(f, _)| *f == 0).map(|(_, l)| l.clone()).unwrap_or_default())));
        let Some((f0, label)) = at else {
            unplaced.push(format!("{name}: no label of its clip names it"));
            continue
        };
        let (sym, from, span) = place(f0);
        // no more frames than the source shows there (the engine's animation can run longer)
        let frames = (*len / 2).min(span.max(1)).max(1);
        pieces.push(Piece { fm_name: name.clone(), label, sym, from, frames, head: 0, reversed: false, borrowed: false, item_box: Vec::new(), on: None });
    }
    (pieces, unplaced)
}

/// The animation a clip's label became on an object's entity (`stage_emit::sanitize_anim`, then a
/// word the engine's parser keeps for itself made a name: `unreserve_animations`).
fn anim_name(label: &str) -> String {
    let s: String = label.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if s.is_empty() { return "anim".to_string() }
    compile::ident(&if s.starts_with(|c: char| c.is_ascii_digit()) { format!("a{s}") } else { s })
}

/// Every picture and box of an entity moved by (dx, dy).
fn shift_entity(path: &Path, dx: f64, dy: f64) -> anyhow::Result<()> {
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    for s in json["symbols"].as_array_mut().into_iter().flatten() {
        if let (Some(x), Some(y)) = (s["x"].as_f64(), s["y"].as_f64()) {
            s["x"] = serde_json::json!(x + dx);
            s["y"] = serde_json::json!(y + dy);
        }
    }
    std::fs::write(path, serde_json::to_string_pretty(&json)?)?;
    Ok(())
}

/// The animation an object's stats start it on, as its active state's.
fn starting_animation(lib: &Path, id: &str, anim: &str) -> anyhow::Result<()> {
    let path = lib.join("scripts").join("hazard").join(format!("{id}GameObjectStats.hx"));
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(()) };
    let re = crate::re!(r#"(PState\.ACTIVE\s*=>\s*\{\s*animation:\s*)"[^"]*""#);
    let out = re.replace(&text, format!("${{1}}{}", compile::quote(anim)).as_str()).into_owned();
    if out != text { std::fs::write(&path, out)?; }
    Ok(())
}

/// An object's animations named by a word the engine's parser keeps for itself (a block's
/// `break`): the stats written under that name do not parse, so the animation takes the name
/// `compile::ident` gives the word, on its entity and in its stats.
fn unreserve_animations(lib: &Path, id: &str) -> anyhow::Result<()> {
    let entity = lib.join("entities").join(format!("{id}.entity"));
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&entity)?)?;
    let mut renamed: Vec<(String, String)> = Vec::new();
    for a in json["animations"].as_array_mut().into_iter().flatten() {
        let Some(n) = a["name"].as_str().map(String::from) else { continue };
        let fixed = compile::ident(&n);
        if fixed != n {
            a["name"] = serde_json::Value::String(fixed.clone());
            renamed.push((n, fixed));
        }
    }
    if renamed.is_empty() { return Ok(()) }
    std::fs::write(&entity, serde_json::to_string_pretty(&json)?)?;
    for kind in ["AnimationStats", "HitboxStats"] {
        let path = lib.join("scripts").join("hazard").join(format!("{id}{kind}.hx"));
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let out: Vec<String> = text.lines().map(|l| {
            let top = l.starts_with('\t') && !l.starts_with("\t\t");
            match renamed.iter().find(|(n, _)| top && l.trim_start().starts_with(&format!("{n}:"))) {
                Some((n, f)) => l.replacen(&format!("{n}:"), &format!("{f}:"), 1),
                None => l.to_string(),
            }
        }).collect();
        std::fs::write(&path, out.join("\n"))?;
    }
    Ok(())
}

/// Each animation of an entity file with how many engine frames it runs, in its order.
fn entity_lengths(path: &Path) -> Vec<(String, u32)> {
    let Some(json) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { return vec![] };
    let kfs: BTreeMap<String, u64> = json["keyframes"].as_array().map(|a| a.iter().filter_map(|k|
        Some((k["$id"].as_str()?.to_string(), k["length"].as_u64().unwrap_or(1)))).collect()).unwrap_or_default();
    let lens: BTreeMap<String, u64> = json["layers"].as_array().map(|a| a.iter().filter_map(|l| {
        let n: u64 = l["keyframes"].as_array()?.iter().filter_map(|k| kfs.get(k.as_str()?)).sum();
        Some((l["$id"].as_str()?.to_string(), n))
    }).collect()).unwrap_or_default();
    json["animations"].as_array().map(|a| a.iter().filter_map(|anim| {
        let name = anim["name"].as_str()?.to_string();
        let total = anim["layers"].as_array().map(|ls| ls.iter().filter_map(|l| lens.get(l.as_str()?)).copied().max().unwrap_or(0)).unwrap_or(0);
        Some((name, total as u32))
    }).collect()).unwrap_or_default()
}

/// Every reason the stage's own classes have no plain form yet, one per function: its class, and
/// each class of a clip inside the stage's clip with code of its own.
#[allow(clippy::too_many_arguments)]
fn stage_census(src: &SourceModel, runtime: &Runtime, class: &str, sym: u16, pieces: &[Piece], units: Units, written: bool, out: &mut Option<compile::native::Written>, objects: &[(String, String)], particles: &BTreeMap<String, (String, f64, f64)>, input_origin: (f64, f64)) -> Vec<String> {
    let particles = particles.clone();
    let made_objects: BTreeMap<String, String> = objects.iter().map(|(c, id)| (compile::short(c).to_string(), id.clone())).collect();
    let Some(itself) = class_of(src, class) else { return vec![format!("its class {class} is not in its package")] };
    let mut classes = Vec::new();
    for s in super::clip_tree(src, Some(sym)) {
        let Some(c) = src.class_of(s) else { continue };
        if runtime.provides(&c.name) || c.name == itself.name || classes.iter().any(|x: &&crate::model::ClassModel| x.name == c.name) { continue }
        // (a clip whose code only sets its own fields to plain values and hides itself is a marker:
        // the conversion read what it marks, and nothing here runs it)
        if super::native_projectile::is_quiet(src, runtime, &c.name) { continue }
        // (a backdrop clip whose frame code only moves its own timeline by label: the chains of the
        // sectioned pieces drawn for it carry those jumps)
        if timeline_only(src, &c.name) { continue }
        classes.push(c);
    }
    let jumps = super::label_jumps(src, pieces);
    let labels = super::native_fighter::stance_labels(src, pieces);
    let frames_of = super::native_fighter::clip_frames_of(src, pieces);
    // (each clip class it runs, by the name it is placed under: its `self` is that clip of the planes)
    let parts_of: BTreeMap<String, String> = placed_names(src, sym).into_iter().filter(|(c, _)| classes.iter().any(|k| &k.name == c)).collect();
    let none = |_: &str| -> Option<String> { None };
    let no_stat = |_: &str| -> Option<(String, String)> { None };
    let no_effect = |_: &str| -> Option<(String, Option<u32>)> { None };
    let no_fired = |_: &str| -> Option<(String, [f64; 4], Option<String>)> { None };
    let no_store = |_: &str| false;
    let no_anim = |_: &str| -> Option<String> { None };
    let switches = BTreeMap::new();
    let links = BTreeSet::new();
    let object = compile::native::Object {
        model: src, itself: Some(itself), classes, jumps: &jumps, runtime, declared: &none, switches: &switches,
        owner_stat: &no_stat, effect: &no_effect, owner_anims: &[], fired: &no_fired, owner_declares: &none, owner_stores: &no_store,
        root: sym, size: units.size, speed: units.speed, accel: units.accel, fighter: false, item: false, stage: true, clip_parts: parts_of.clone(), made_objects: made_objects.clone(), particles: particles.clone(), clip_bounds: BTreeMap::new(),
        root_anim: &no_anim, walk_start: None, stance_labels: &labels, clip_frames: &frames_of, frame_links: &links, anim_clips: &[], owner_native: false,
        effect_names: Vec::new(), effect_pieces: Vec::new(), parts: Vec::new(), content: String::new(), palette: None,
        effect_inners: BTreeMap::new(), effect_code: BTreeMap::new(), instance: None, origin: input_origin, power_record: false, own_parts: Vec::new(), nested: Vec::new(),
    };
    if !written { return compile::native::census(&object) }
    match compile::native::write(&object) { Ok(w) => { *out = Some(w); Vec::new() } Err(e) => vec![e] }
}

/// The stage's script as plain Fraymakers code: its boot as calls to `runtime/native/stage_boot.hx`'s
/// pieces, the upkeep it has something for, and its own classes' code; or every reason it is not.
#[allow(clippy::too_many_arguments)]
fn native_stage(src: &SourceModel, runtime: &Runtime, class: &str, sym: u16, pieces: &[Piece], units: Units, input: &StageInput, id: &str, regions: &str, particles: &BTreeMap<String, (String, f64, f64)>) -> Result<(String, Vec<String>), Vec<String>> {
    let t = input.tables;
    let mut written = None;
    let reasons = stage_census(src, runtime, class, sym, pieces, units, true, &mut written, &input.objects, particles, input.origin);
    let Some(w) = written else { return Err(reasons) };
    let skeleton = runtime.native_files.get("stage").ok_or_else(|| vec!["runtime/native/stage.hx is missing".to_string()])?;
    // the boot, as calls to the pieces (the named clips are the code's to ask for, and no native code asks yet)
    let rename = |text: String| crate::re!(r"StageBoot\.(\w)(\w*)\(").replace_all(&text, |c: &regex::Captures| format!("boot{}{}(", c[1].to_uppercase(), &c[2])).to_string();
    let placed = rename(t.placed_calls("", "null"));
    let spawn = rename(t.spawn_calls("", "null", "chars", "hazardsOn", |g, made| format!("stageGroups.set({}, {made});", compile::quote(&g.name))));
    if let Some(left) = placed.lines().chain(spawn.lines()).find(|l| l.contains("StageBoot.")) { return Err(vec![format!("its boot calls {}, which is not written natively yet", left.trim())]) }
    // the pieces it uses
    let mut blocks: Vec<&str> = vec!["sized", "named", "rides"];
    let uses = |n: &str| placed.contains(n) || spawn.contains(n);
    if uses("bootStructure(") { blocks.push("structure") }
    if uses("bootEffect(") || uses("bootStill(") { blocks.extend(["container", "pan", "effect"]) }
    if uses("bootSegment(") || uses("bootStill(") { blocks.extend(["container", "pan", "segment"]) }
    if uses("bootChained(") { blocks.extend(["container", "pan", "sectioned", "chained"]) }
    if uses("bootCarry(") { blocks.push("carry") }
    if uses("bootStill(") { blocks.push("still") }
    if uses("bootObject(") { blocks.push("object") }
    if uses("bootDeck(") { blocks.push("deck") }
    if uses("bootGroup(") { blocks.push("group") }
    if uses("bootCamAlpha(") { blocks.push("cam_alpha") }
    if uses("bootCamFade(") { blocks.push("cam_fade") }
    let mut upkeep = Vec::new();
    if t.has_riders() || uses("bootObject(") { blocks.push("follow"); upkeep.push("followUpkeep();".to_string()) }
    if t.has_shown() { blocks.push("shown"); upkeep.push("shownUpkeep(t);".to_string()) }
    if t.has_pan() { upkeep.push("panUpkeep();".into()) }
    if t.has_fade() || t.has_tint() { blocks.push("key") }
    if t.has_fade() { blocks.push("alphas"); upkeep.push("alphasUpkeep(t);".into()) }
    if t.has_tint() { blocks.push("tints"); upkeep.push("tintsUpkeep(t);".into()) }
    let surfaces = !input.surfaces.is_empty();
    if surfaces { blocks.push("surfaces") }
    if surfaces || !input.frictions.is_empty() || !input.pushes.is_empty() { blocks.push("ground"); upkeep.push("groundUpkeep();".into()) }
    if t.has_follow() { blocks.push("riding"); upkeep.push("ridingUpkeep();".into()) }
    // the named clips of its planes, which its code and its objects' code reach (always: an object's
    // code asks the stage for them)
    blocks.push("parts");
    // the weather its code makes: its clips, drawn each frame
    let weather = w.functions.iter().chain(w.start.iter()).chain(w.tick.iter()).any(|f| f.contains("weatherRoot"));
    if weather { blocks.push("weather"); upkeep.push("weatherUpkeep();".into()) }
    let need_clip = weather && !w.functions.iter().any(|f| f.lines().any(|l| l.starts_with("function") && l.contains(" clipRecord(")));
    let mut seen = BTreeSet::new();
    blocks.retain(|b| seen.insert(*b));
    // each weather field's list of drawn particles, as the stage's own variable (the field's index,
    // as its effect is named)
    let prefix = format!("{id}_weather");
    let weather_fields: Vec<(String, String)> = input.particles.iter()
        .filter_map(|(_, pid)| pid.strip_prefix(&prefix).map(|i| (pid.clone(), i.to_string()))).collect();
    let strs = |v: &[String]| v.iter().map(|s| compile::quote(s)).collect::<Vec<_>>().join(", ");
    let rows = |v: Vec<String>| if v.is_empty() { "[]".to_string() } else { format!("[{}]", v.join(", ")) };
    // (how many children each named clip of its tree places on its first frame: what its code
    // counts through)
    // ...and each of them in order: a picture placed there (a Flash `Bitmap`) is its library name,
    // where its corner sits on the stage and its scale (this engine's units), and its size
    type M = (f64, f64, f64, f64, f64, f64);
    let mul = |p: M, l: &crate::swf_timeline::Matrix| -> M { (p.0 * l.a + p.2 * l.b, p.1 * l.a + p.3 * l.b, p.0 * l.c + p.2 * l.d, p.1 * l.c + p.3 * l.d,
        p.0 * l.tx + p.2 * l.ty + p.4, p.1 * l.tx + p.3 * l.ty + p.5) };
    let mut kids: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    let mut todo: Vec<(u16, M, usize)> = vec![(sym, (1.0, 0.0, 0.0, 1.0, 0.0, 0.0), 0)];
    let mut seen = BTreeSet::new();
    while let Some((s, world, depth)) = todo.pop() {
        if depth > 12 || !seen.insert((s, (world.4 * 10.0) as i64, (world.5 * 10.0) as i64)) { continue }
        let Some(sp) = src.sprites.get(&s) else { continue };
        for placed in sp.frames.iter().flatten() {
            let w = mul(world, &placed.matrix);
            if let Some(n) = placed.name.as_ref() {
                if !kids.contains_key(n) {
                    let first: Vec<&crate::swf_timeline::Placed> = src.sprites.get(&placed.char_id).and_then(|c| c.frames.first()).map(|f| f.iter().collect()).unwrap_or_default();
                    let list = first.iter().map(|c| match src.bitmaps.get(&c.char_id) {
                        Some((bw, bh)) => {
                            let cw = mul(w, &c.matrix);
                            format!("{{ b: {}, x: {:.2}, y: {:.2}, sx: {:.4}, sy: {:.4}, w: {bw}, h: {bh} }}", compile::quote(src.symbols.get(&c.char_id).map(String::as_str).unwrap_or("")),
                                (cw.4 - input.origin.0) * units.size, (cw.5 - input.origin.1) * units.size, cw.0 * units.size, cw.3 * units.size)
                        }
                        None => "null".to_string(),
                    }).collect();
                    kids.insert(n.clone(), (first.len(), list));
                }
            }
            todo.push((placed.char_id, w, depth + 1));
        }
    }
    let words = [("PAN_X", format!("{}", input.pan.0)), ("PAN_Y", format!("{}", input.pan.1)), ("REGIONS", regions.to_string()),
        ("KIDS", rows(kids.iter().map(|(n, (k, c))| format!("{{ n: {}, k: {k}, c: [{}] }}", compile::quote(n), c.join(", "))).collect())),
        ("SURFACES", rows(input.surfaces.iter().map(|(n, s)| format!("{{ n: {}, s: [{}] }}", compile::quote(n), strs(s))).collect())),
        ("FRICTIONS", rows(input.frictions.clone())), ("PUSHES", rows(input.pushes.clone())), ("ACCEL", format!("{}", units.accel)),
        ("SURFACES_ON", surfaces.to_string()), ("CAP", input.particle_cap.to_string()), ("SIZE", format!("{}", units.size)),
        ("BY_CODE", format!("[{}]", input.rows.iter().filter(|r| r.by_code).map(|r| compile::quote(&r.id)).collect::<Vec<_>>().join(", "))),
        ("WEATHER_LISTS", rows(weather_fields.iter().map(|(id, i)| format!("{{ id: {}, l: m_weather{i} }}", compile::quote(id))).collect())),
        ("ELEMENTS", rows(t.elements.iter().map(|e| format!("{{ i: {}, seg: {}, path: [{}], labels: [{}], sections: {} }}",
            e.index, e.segmented, strs(&e.path), strs(&e.labels), super::stage_tables::sections_literal(e))).collect()))];
    let mut functions = Vec::new();
    for b in blocks { functions.push(compile::native::helper(runtime, "stage_boot", b, "", &words, &[]).map_err(|e| vec![e])?) }
    if need_clip { functions.push(compile::native::helper(runtime, "utils", "clipRecord", "clipRecord", &[], &[]).map_err(|e| vec![e])?) }
    functions.extend(w.functions.iter().cloned());
    // each clip of its planes with frame code of its own: run as its timeline reaches each frame
    let mut timelines: BTreeSet<String> = BTreeSet::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for (cls, name) in placed_names(src, sym) {
        if !seen.insert((cls.clone(), name.clone())) { continue }
        let Some(csym) = src.classes.get(&cls).and_then(|c| c.symbol) else { continue };
        let cases: Vec<String> = w.frames.iter().filter(|((s, _), _)| *s == csym)
            .map(|((_, f), code)| format!("case {f}: {}", code.lines().map(str::trim).collect::<Vec<_>>().join(" "))).collect();
        if cases.is_empty() { continue }
        // (two clips placed under one name: each its own timeline)
        let mut fname = format!("timeline_{}", compile::ident(&name));
        if !timelines.insert(fname.clone()) { fname = format!("{fname}_{}", compile::ident(&cls)); timelines.insert(fname.clone()); }
        functions.push(compile::native::helper(runtime, "stage_boot", "timeline", &fname, &[("PART", compile::quote(&name)), ("NAMEAt", format!("{fname}At"))], &cases).map_err(|e| vec![e])?);
        if !functions.iter().any(|f| f.contains("planePart(n)")) {
            functions.push(compile::native::helper(runtime, "utils", "planePart", "planePart", &[], &[]).map_err(|e| vec![e])?);
        }
        upkeep.push(format!("{fname}();"));
    }
    // (clips its code makes from the library: drawn where its code puts them each frame; a part
    // answers its code's taking clips off or putting them in through these, so they are always there)
    {
        functions.push(compile::native::helper(runtime, "utils", "libClips", "libClips", &[("SIZE", format!("{}", units.size))], &[]).map_err(|e| vec![e])?);
        upkeep.push("libUpkeep();".into());
    }
    // (a light its code keeps over the fighters: the stage casts their shadows from it each frame)
    if input.follow_light.is_some() {
        functions.push(compile::native::helper(runtime, "utils", "followLight", "followLight", &[("SIZE", format!("{}", units.size))], &[]).map_err(|e| vec![e])?);
        upkeep.push("followShadows();".into());
    }
    let mut slots: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut fields = w.fields.clone();
    fields.extend(weather_fields.iter().map(|(_, i)| format!("var m_weather{i} = [];")));
    slots.insert("fields", fields);
    slots.insert("functions", functions.join("\n\n").lines().map(String::from).collect());
    slots.insert("placed", placed.lines().map(String::from).collect());
    slots.insert("spawn", spawn.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect());
    slots.insert("own_start", w.start.clone());
    slots.insert("own_tick", w.tick.clone());
    slots.insert("upkeep", upkeep);
    let mut publish = w.publish.clone();
    publish.push("self.exports.__named = objectNamed;".into());
    // (its respawn beacons, which its code asks the engine for: `getSpawnPositionMCs`)
    publish.push(format!("self.exports.__spawns = {};", STAGE_SPAWNS.with(|c| c.borrow().clone())));
    publish.push("self.exports.__part = partNamed;".into());
    // (how many clips its weather clip holds, for the verify gates)
    if weather { publish.push("self.exports.__weatherHeld = function() { return weatherRoot.kids.length; };".into()); }
    // (its camera backgrounds' names, each the name its pieces are drawn under)
    let cams: Vec<String> = src.declared("camera").as_ref().and_then(|c| c.get("backgrounds").cloned())
        .map(|b| match b { Value::Arr(items) => items, _ => Vec::new() }).unwrap_or_default().iter()
        .map(|b| b.get("linkage_id").and_then(|v| v.str()).map(compile::quote).unwrap_or_else(|| "null".into())).collect();
    publish.push(format!("self.exports.__cameraBgs = [{}];", cams.join(", ")));
    slots.insert("publish", publish);
    let script = compile::native::fill_slots(skeleton, &slots).replace("{title}", id).replace("{classes}", class)
        // (the stage's own sprite, which shows the pictures its code draws from)
        .replace("getContent(STAGE_ID)", &format!("getContent({})", compile::quote(id)))
        .replace("STAGE_MUSIC", &input.music.as_deref().map(compile::quote).unwrap_or_else(|| "null".into()))
        .replace("LIB_ROWS", &format!("[{}]", input.lib.iter().map(|(n, parts)| format!("{{ n: {}, parts: [{parts}] }}", compile::quote(n))).collect::<Vec<_>>().join(", ")))
        .replace("FOLLOW_LIGHT", &input.follow_light.map(|(x, y)| format!("[{x}, {y}]")).unwrap_or_else(|| "null".into()))
        .replace("FOLLOW_FLOORS", &format!("[{}]", input.floors.iter().map(|f| format!("[{:.1}, {:.1}, {:.1}, {:.1}]", f[0], f[1], f[2], f[3])).collect::<Vec<_>>().join(", ")))
        .replace("\t// what its own class runs each of the source's frames (every second engine frame)\n\tif (stageFrames % 2 == 1) {\n\t}\n", "");
    if let Some(n) = compile::native::declared_twice(&script) { return Err(vec![format!("its script declares {n} twice")]) }
    Ok((script, w.gaps.iter().cloned().collect()))
}

thread_local! {
    /// the stage being written's respawn beacons, as a literal of records in the source's units
    static STAGE_SPAWNS: std::cell::RefCell<String> = std::cell::RefCell::new("[]".into());
}

/// The regions the tree placed under a name: each one's rectangle and the point its clip is placed
/// by (this engine's units).
fn regions_literal(model: &crate::stage_parser::StageModel) -> String {
    let regions: Vec<String> = model.named_boxes.iter().map(|(n, r)| {
        let (rx, ry) = model.named_regs.get(n).copied().unwrap_or((r.x, r.y));
        let (sx, sy, rot) = model.named_turns.get(n).copied().unwrap_or((1.0, 1.0, 0.0));
        let (px, py) = model.named_parents.get(n).copied().unwrap_or((0.0, 0.0));
        format!("{{ n: {}, x: {:.1}, y: {:.1}, w: {:.1}, h: {:.1}, rx: {rx:.1}, ry: {ry:.1}, px: {px:.1}, py: {py:.1}, sx: {sx:.4}, sy: {sy:.4}, rot: {rot:.2} }}", compile::quote(n), r.x, r.y, r.w, r.h)
    }).collect();
    if regions.is_empty() { "[]".to_string() } else { format!("[\n\t{}\n]", regions.join(",\n\t")) }
}

/// A stage object's script as plain Fraymakers code: its own class (and the clips its animations
/// show) through the native writer, on `runtime/native/object.hx` with the rules it uses from
/// `object_rules.hx`. Every file is written only once all of it has a plain form.
#[allow(clippy::too_many_arguments)]
fn native_object(src: &SourceModel, runtime: &Runtime, dir: &Path, row: &ObjectRow, units: Units, bounds: &BTreeMap<String, (f64, f64, f64, f64)>, projectiles: &Value, input_origin: (f64, f64), objects: &[(String, String)]) -> Result<Vec<String>, Vec<String>> {
    let mut gaps: Vec<String> = Vec::new();
    let made_objects: BTreeMap<String, String> = objects.iter().map(|(c, id)| (compile::short(c).to_string(), id.clone())).collect();
    let one = |e: String| vec![e];
    let c = class_of(src, &row.class).ok_or_else(|| one(format!("its class {} is not in its package", row.class)))?;
    let merged = super::native_projectile::inherited_class(src, runtime, c).map_err(one)?;
    let itself = merged.as_ref().unwrap_or(c);
    let (entity, script, root) = object_files(src, dir, row).map_err(|e| one(e.to_string()))?;
    let (pieces, _) = root.map(|r| object_pieces(src, &entity, r)).unwrap_or_default();
    // the classes whose code it runs: the clips its animations show (any other clip inside it with
    // code of its own is said in its gaps)
    let shown: BTreeSet<u16> = pieces.iter().map(|p| p.sym).collect();
    let mut classes = Vec::new();
    for sym in super::clip_tree(src, root) {
        let Some(k) = src.class_of(sym) else { continue };
        if runtime.provides(&k.name) || k.name == itself.name || k.name == c.name || classes.iter().any(|x: &&crate::model::ClassModel| x.name == k.name) { continue }
        if super::native_projectile::is_quiet(src, runtime, &k.name) { continue }
        // (a clip nested in its art: its own timeline, or what it declares about itself, is drawn
        // nowhere)
        if !shown.contains(&sym) && super::native_projectile::is_quiet_inside(src, runtime, &k.name) { continue }
        // (a clip nested in its art with code of its own: drawn whole, its code said, not run)
        if !shown.contains(&sym) { gaps.push(format!("{} inside its art has code of its own, which nothing here runs", compile::short(&k.name))); continue }
        classes.push(k);
    }
    // what it declares about itself (`ownStats`): its stats, read by name
    let pkg = compile::Package::new(src, runtime);
    let own: Option<Value> = pkg.chain(&c.name).0.into_iter()
        .find_map(|k| k.methods.iter().find(|m| (m.name == "ownStats" || m.name == "getOwnStats") && !m.is_static))
        .and_then(|m| crate::model::declare::returned(&m.function.body));
    let plain = |v: &Value| match v { Value::Num(n) => Some(format!("{n}")), Value::Bool(b) => Some(b.to_string()), Value::Str(s) => Some(compile::quote(s)), Value::Null => Some("null".into()), _ => None };
    let declared = |k: &str| own.as_ref().and_then(|o| o.get(k)).and_then(plain);
    // (an item, one a fighter picks up: it carries the item handler)
    let is_item = compile::short(&itself.super_name) == "PeptideItem";
    let mut jumps = super::label_jumps(src, &pieces);
    jumps.entry(itself.name.clone()).or_insert_with(|| super::own_jumps(src, &pieces, root));
    let labels = super::native_fighter::stance_labels(src, &pieces);
    let mut frames_of = super::native_fighter::clip_frames_of(src, &pieces);
    // (a clip with no class of its own still has its frames counted, under its symbol)
    for p in pieces.iter().filter(|p| !p.reversed && p.head == 0 && src.class_of(p.sym).is_none()) {
        frames_of.entry(format!("#{}", p.sym)).or_default().push((p.fm_name.clone(), p.from, p.frames));
    }
    let no_stat = |_: &str| -> Option<(String, String)> { None };
    let no_effect = |_: &str| -> Option<(String, Option<u32>)> { None };
    // a projectile its stage declares, by the name its code fires it under: its content (the fired
    // name, as the stage's package registers it), its offset and its launch
    let no_fired = |name: &str| -> Option<(String, [f64; 4], Option<String>)> {
        let decl = projectiles.get(name)?;
        let n = |ks: &[&str]| ks.iter().find_map(|k| match decl.get(k) { Some(Value::Num(v)) => Some(*v), _ => None }).unwrap_or(0.0);
        Some((crate::projectile_gen::projectile_content_id(name),
              [n(&["xoffset", "xOffset", "x_offset"]), n(&["yoffset", "yOffset", "y_offset"]), n(&["xspeed", "xSpeed", "x_speed"]), n(&["yspeed", "ySpeed", "y_speed"])], None))
    };
    let none = |_: &str| -> Option<String> { None };
    let no_store = |_: &str| false;
    let no_anim = |_: &str| -> Option<String> { None };
    let switches = BTreeMap::new();
    let links = BTreeSet::new();
    let object = compile::native::Object {
        model: src, itself: Some(itself), classes, jumps: &jumps, runtime, declared: &declared, switches: &switches,
        owner_stat: &no_stat, effect: &no_effect, owner_anims: &[], fired: &no_fired, owner_declares: &none, owner_stores: &no_store,
        root: root.unwrap_or(0), size: units.size, speed: units.speed, accel: units.accel, fighter: false, item: is_item, stage: false, clip_parts: BTreeMap::new(), made_objects: made_objects.clone(), particles: BTreeMap::new(), clip_bounds: bounds.clone(),
        root_anim: &no_anim, walk_start: None, stance_labels: &labels, clip_frames: &frames_of, frame_links: &links, anim_clips: &[], owner_native: false,
        effect_names: crate::mappings::api_commands().global_vfx_map.keys().cloned().collect(), effect_pieces: Vec::new(), parts: Vec::new(),
        content: String::new(), palette: None, effect_inners: BTreeMap::new(), effect_code: BTreeMap::new(), instance: row.instance.clone(), origin: input_origin, power_record: false, own_parts: Vec::new(), nested: Vec::new(),
    };
    let w = compile::native::write(&object).map_err(one)?;
    let skeleton = runtime.native_files.get("object").ok_or_else(|| one("runtime/native/object.hx is missing".into()))?;
    let text_of = |w: &compile::native::Written| w.functions.join("\n") + &w.start.join("\n") + &w.tick.join("\n") + &w.frames.values().cloned().collect::<Vec<_>>().join("\n");
    let text = text_of(&w);
    // the rules it uses
    let body = row.body.as_ref().map(|b| {
        let faces: Vec<String> = b.faces.iter().filter(|(_, pts)| pts.len() >= 2).map(|(ty, pts)| format!("[StructureType.{ty}, [{}]]",
            pts.iter().flat_map(|(x, y)| [format!("{x:.1}"), format!("{y:.1}")]).collect::<Vec<_>>().join(", "))).collect();
        format!("{{ box: [{:.1}, {:.1}, {:.1}, {:.1}], faces: [{}], ledges: [{}, {}] }}",
            b.bounds.0, b.bounds.1, b.bounds.2, b.bounds.3, faces.join(", "), b.ledges.0, b.ledges.1)
    });
    let floors = text.contains("ownFloor(") || text.contains("dropOwnFloor(");
    // (the label of its clip it is on, for its own code that asks)
    let labelled = text.contains("clipLabel(");
    let mut blocks: Vec<&str> = Vec::new();
    if body.is_some() || floors { blocks.push("faces") }
    if body.is_some() { blocks.push("placed_body") }
    if floors { blocks.push("own_floor") }
    // its attacks' refresh rates, by the animation each is (its declared attacks, `getAttackStats`)
    let attacks: Option<Value> = pkg.chain(&c.name).0.into_iter()
        .find_map(|k| k.methods.iter().find(|m| m.name == "getAttackStats" && !m.is_static))
        .and_then(|m| crate::model::declare::returned(&m.function.body));
    let mut rehit = Vec::new();
    for (label, a) in attacks.as_ref().map(|a| a.entries().to_vec()).unwrap_or_default() {
        let Some(Value::Num(r)) = a.get("refreshRate").cloned() else { continue };
        if r <= 0.0 || r >= 999.0 { continue }
        for p in pieces.iter().filter(|p| p.label == label) { rehit.push(format!("case {}: rehitEvery = {};", compile::quote(&p.fm_name), (r * 2.0) as i64)) }
    }
    if !rehit.is_empty() { blocks.push("rehit") }
    let through = text.contains("facesThrough(") || text.contains("throughOf(");
    if through { if !blocks.contains(&"faces") { blocks.insert(0, "faces") } blocks.push("faces_through") }
    let words = [("SIZE", format!("{}", units.size)), ("BODY", body.clone().unwrap_or_else(|| "null".into())),
        ("BODY_ON", body.is_some().to_string()), ("FLOOR_ON", floors.to_string())];
    let mut functions = Vec::new();
    for b in &blocks { functions.push(compile::native::helper(runtime, "object_rules", b, "", &words, if *b == "rehit" { &rehit } else { &[] }).map_err(one)?) }
    if labelled { functions.push(compile::native::helper(runtime, "fighter_tables", "clip_label", "", &[], &super::native_fighter::clip_labels(src, &pieces)).map_err(one)?) }
    functions.extend(w.functions.iter().cloned());
    // as it is made: where the tree placed it, what it declares, its body, and its own hurt
    let mut begin = Vec::new();
    if let Some((x, y)) = row.at { begin.push(format!("self.setX({x:.4});")); begin.push(format!("self.setY({y:.4});")); }
    let stats: Vec<String> = ["gravity", "max_ySpeed", "surviveDeathBounds", "canReceiveKnockback", "ghost"].iter()
        .filter_map(|k| declared(k).map(|v| format!("{k}: {v}"))).collect();
    if !stats.is_empty() {
        if !w.functions.iter().any(|f| f.lines().any(|l| l.starts_with("function") && l.contains(" enemyStats("))) {
            functions.push(compile::native::helper(runtime, "utils", "enemyStats", "enemyStats", &[("ACCEL", format!("{}", units.accel)), ("SPEED", format!("{}", units.speed))], &[]).map_err(one)?);
        }
        begin.push(format!("enemyStats(self, {{ {} }});", stats.join(", ")));
    }
    if body.is_some() { begin.push("placedBody();".into()) }
    let mut every = Vec::new();
    if body.is_some() { every.push("facesFollow(bodyFaces);".to_string()) }
    if floors { every.push("facesFollow(ownFaces);".into()) }
    if !rehit.is_empty() { every.push("rehitUpkeep();".into()) }
    every.extend(w.every_frame.iter().cloned());
    let mut end = Vec::new();
    if body.is_some() { end.push("releaseFaces(bodyFaces);".to_string()); end.push("bodyFaces = [];".into()) }
    if floors { end.push("dropOwnFloor();".into()) }
    end.extend(w.release.iter().cloned());
    let mut slots: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    slots.insert("fields", w.fields.clone());
    slots.insert("functions", functions.join("\n\n").lines().map(String::from).collect());
    // (the class it is, for another object's code that tests it against one)
    begin.push(format!("self.exports.__className = {};", compile::quote(compile::short(&c.name))));
    // (where it stands from where its clip is registered, this engine's units, for its code's places;
    // one whose code reads its place, with that not known, reads it from where it stands)
    if row.anchor_unknown && ["placeX(", "placeY(", "setPlaceX(", "setPlaceY(", "startPosition("].iter().any(|k| text.contains(k)) {
        gaps.push("reads its place from where its clip is registered, which is not known (its clip's name names another clip of the tree too)".into());
    }
    if let Some((ax, ay)) = row.anchor { begin.push(format!("self.exports.__anchor = {{ x: {ax:.2}, y: {ay:.2} }};")); }
    // (and every class it is, down to the API's: another object's code asks for every one of a kind)
    let (line, base) = pkg.chain(&c.name);
    let kinds: Vec<String> = line.iter().map(|k| compile::quote(compile::short(&k.name))).chain(base.map(|b| compile::quote(compile::short(&b)))).collect();
    begin.push(format!("self.exports.__classes = [{}];", kinds.join(", ")));
    // (a body: another object's code lets things through its faces)
    if blocks.contains(&"faces") { begin.push("self.exports.__ignore = ignoreThrough;".into()); }
    // (one that keeps the hits it lands before its first frame, `heardEarly`: kept from its start)
    if text.contains("earlyListen()") { begin.insert(0, "earlyListen();".into()); }
    slots.insert("begin", begin);
    // (its construction, and the static values set up before it, as it is made: the rest as it starts)
    // (an item's handler starts it its own way: all of it as it starts)
    let made = if is_item { 0 } else { w.start.iter().position(|l| l == "construct();").map(|i| i + 1).unwrap_or(0) };
    slots.insert("construct", w.start[..made].to_vec());
    slots.insert("own_start", w.start[made..].to_vec());
    slots.insert("own_tick", w.tick.clone());
    slots.insert("every_frame", every);
    // (its methods another object's code calls start it first where it has not yet: `startIfWaiting`)
    let method_pub = crate::re!(r"^self\.exports\.([A-Za-z_]\w*) = ([A-Za-z_]\w*);$");
    let publish: Vec<String> = w.publish.iter().map(|l| match method_pub.captures(l) {
        Some(c) if !c[1].starts_with("__") && !is_item && w.functions.iter().any(|f| f.contains(&format!("function {}(", &c[2]))) => {
            // (its own parameters, as its function declares them)
            let head = format!("function {}(", &c[2]);
            let params: Vec<String> = w.functions.iter().find_map(|f| f.find(&head).map(|i| {
                let rest = &f[i + head.len()..];
                rest[..rest.find(')').unwrap_or(0)].split(',').map(|p| p.trim().trim_start_matches('?').to_string()).filter(|p| !p.is_empty()).collect()
            })).unwrap_or_default();
            let decl: Vec<String> = params.iter().map(|p| format!("?{p}")).collect();
            format!("self.exports.{} = function({}) {{ startIfWaiting(); return {}({}); }};", &c[1], decl.join(", "), &c[2], params.join(", "))
        }
        _ => l.clone(),
    }).collect();
    slots.insert("publish", publish);
    slots.insert("end", end);
    // (its first frame's code, once its maker's code is done: the frame it starts on)
    let first_label = root.and_then(|r| src.sprites.get(&r)).and_then(|s| s.labels.iter().find(|(f, _)| *f == 0)).map(|(_, l)| l.clone());
    let start = pieces.iter().find(|p| (Some(p.sym) == root && p.from == 1) || (Some(p.sym) != root && Some(&p.label) == first_label.as_ref()));
    slots.insert("first", start.and_then(|p| w.frames.get(&(p.sym, p.from))).cloned().into_iter().collect());
    let names: Vec<&str> = std::iter::once(itself.name.as_str()).chain(object.classes.iter().map(|k| k.name.as_str())).collect();
    // an item: the item handler, filled in with what it declares
    let out = if is_item {
        let item_skel = runtime.native_files.get("item").ok_or_else(|| one("runtime/native/item.hx is missing".into()))?;
        let mut islots = slots.clone();
        let moved = islots.remove("begin").unwrap_or_default();
        islots.insert("start", moved);
        let moved = islots.remove("every_frame").unwrap_or_default();
        islots.insert("update", moved);
        let moved = islots.remove("end").unwrap_or_default();
        islots.insert("ends", moved);
        let num = |k: &str, d: f64| own.as_ref().and_then(|o| o.get(k)).and_then(|v| match v { Value::Num(n) => Some(*n), _ => None }).unwrap_or(d);
        let flag = |k: &str, d: bool| own.as_ref().and_then(|o| o.get(k)).map(|v| match v { Value::Bool(b) => *b, Value::Num(n) => *n != 0.0, _ => d }).unwrap_or(d);
        let idle = start.map(|p| p.fm_name.clone()).unwrap_or_default();
        let words = [("TOSS", format!("{}", num("tossSpeed", units.item_toss))), ("PICKUP", flag("canPickup", true).to_string()),
            ("WIDTH", format!("{}", num("width", 20.0))), ("HEIGHT", format!("{}", num("height", 20.0))), ("SIZE", format!("{}", units.size)), ("SPEED", format!("{}", units.speed)),
            ("GHOST", flag("ghost", false).to_string()), ("GRAVITY", format!("{}", num("gravity", 0.0) * units.accel)),
            ("FALL_CAP", format!("{}", num("max_gravity", num("maxgravity", num("max_ySpeed", 0.0))) * units.speed)), ("IDLE_ANIM", compile::quote(&idle))];
        let mut text = compile::native::fill_slots(item_skel, &islots).replace("{title}", &row.id).replace("{classes}", &names.join(", "))
            .replace("makeInt(LIFE)", &format!("makeInt({})", (num("time_max", 999999.0) * 2.0) as i64));
        for (w, v) in words { text = crate::dyn_re(&format!(r"\b{w}\b")).replace_all(&text, v.as_str()).to_string(); }
        text
    } else { compile::native::fill_slots(skeleton, &slots).replace("{title}", &row.id).replace("{classes}", &names.join(", "))
        .replace("\t// what its own class runs each of the source's frames (every second engine frame)\n\tif (frames.get() % 2 == 1) {\n\t}\n", "")
        .replace("\tif (!started.get()) {\n\t\tstarted.set(true);\n\t}\n", "\tif (!started.get()) started.set(true);\n") };
    if let Some(n) = compile::native::declared_twice(&out) { return Err(one(format!("its script declares {n} twice"))) }
    // its files: the script, and its entity with each frame's code on its keyframe
    let lib = dir.join("library");
    unreserve_animations(&lib, &row.id).map_err(|e| one(e.to_string()))?;
    if let Some((dx, dy)) = row.registration { shift_entity(&entity, dx, dy).map_err(|e| one(e.to_string()))?; }
    let first_label = root.and_then(|r| src.sprites.get(&r)).and_then(|s| s.labels.iter().find(|(f, _)| *f == 0)).map(|(_, l)| l.clone());
    if let Some(first) = pieces.iter().find(|p| (Some(p.sym) == root && p.from == 1) || (Some(p.sym) != root && Some(&p.label) == first_label.as_ref())) {
        starting_animation(&lib, &row.id, &first.fm_name).map_err(|e| one(e.to_string()))?;
    }
    std::fs::write(&script, &out).map_err(|e| one(e.to_string()))?;
    super::native_projectile::keyframes_with(&entity, &row.id, src, &pieces, &w.frames, true, &BTreeMap::new()).map_err(|e| one(e.to_string()))?;
    let anim_stats = lib.join("scripts").join("hazard").join(format!("{}AnimationStats.hx", row.id));
    super::native_projectile::plays_on(&anim_stats, src, &pieces).map_err(|e| one(e.to_string()))?;
    gaps.extend(w.gaps.iter().cloned());
    Ok(gaps)
}

/// whether a clip's class only stops, plays or sends its own timeline to a label (and registers its
/// frame scripts): its timeline, which the sections drawn for it chain the same way
fn timeline_only(src: &SourceModel, class: &str) -> bool {
    let Some(c) = src.classes.get(class) else { return false };
    fn own(f: &Expr) -> Option<&str> { match f { Expr::Member(t, m) if matches!(**t, Expr::This) => Some(m.as_str()), Expr::Name(m) => Some(m.as_str()), _ => None } }
    let mut jumps = false;
    let ok = c.methods.iter().map(|m| &m.function).chain([&c.constructor]).all(|f| f.body.iter().all(|s| match s {
        Stmt::Return(None) | Stmt::Expr(Expr::SuperCall(..)) => true,
        Stmt::Expr(Expr::Call(g, a)) => match (own(g), a.as_slice()) {
            (Some("stop" | "play" | "addFrameScript"), _) => true,
            (Some("gotoAndStop" | "gotoAndPlay"), [Expr::Str(_)]) => { jumps = true; true }
            _ => false,
        },
        _ => false,
    }));
    ok && jumps
}

/// the name each clip class is placed under in a clip's tree (the first it is placed by)
fn placed_names(src: &SourceModel, root: u16) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut todo = vec![root];
    while let Some(sym) = todo.pop() {
        if !seen.insert(sym) { continue }
        let Some(sp) = src.sprites.get(&sym) else { continue };
        for placed in sp.frames.iter().flatten() {
            // (one nobody named: by its symbol, as the walk of the planes files it)
            if let Some(c) = src.class_of(placed.char_id) {
                out.entry(c.name.clone()).or_insert_with(|| placed.name.clone().unwrap_or_else(|| format!("#{}", placed.char_id)));
            }
            todo.push(placed.char_id);
        }
    }
    out
}
