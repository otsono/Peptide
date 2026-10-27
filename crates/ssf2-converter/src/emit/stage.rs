//! A stage's script on the runtime path: the class its package registers as its stage, run by the
//! runtime on the clip that is the whole stage, with the boot the stage pipeline worked out
//! (`DATA.stage`). Each object class with code of its own runs on its custom game object the same
//! way (`runtime/entry/object.hx`). Written over the stage pipeline's scripts, after it has written
//! everything else (art, entities, the plan the verify gates read), so only the scripts change.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::stage_tables::StageTables;
use super::{clock_entity, entity_animations, object_script, Decls, Emitted, ObjectSpec, Piece, Units};
use crate::compile::{self, Runtime};
use crate::model::ast::{Expr, Place, Stmt};
use crate::model::declare::Value;
use crate::model::SourceModel;

/// What the stage pipeline hands the runtime path: what it worked out, as data.
pub struct StageInput<'a> {
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
}

/// A placed body: the box its collision clip declares and the outline the clip draws (this
/// engine's units, the object's own space), and which ends of its floor are ledges.
pub struct Body {
    pub bounds: (f64, f64, f64, f64),
    pub faces: Vec<(&'static str, Vec<(f64, f64)>)>,
    pub ledges: (bool, bool),
}

/// The stage's script, written the runtime's way over the one at `scripts/stage/<id>Script.hx`,
/// then each object's over its own. Returns what the stage's script is, or why it could not be
/// written (the stage pipeline's scripts then stay); an object it cannot write keeps its own.
/// Returns too the objects written.
pub fn runtime_stage(model: &crate::stage_parser::StageModel, dir: &Path, input: &StageInput) -> anyhow::Result<(Emitted, Vec<String>)> {
    let id = &model.id;
    let located = Runtime::locate().ok_or_else(|| anyhow::anyhow!("the runtime's files are not where the converter looks for them"))?;
    let runtime = Runtime::load(&located).for_kind("stage");
    let (src, renamed) = runtime.read_package(&std::fs::read(&model.source)?)?;
    let (class, clip, sym) = src.stage()
        .ok_or_else(|| anyhow::anyhow!("the package registers no stage class with a clip of its own"))?;
    // a stage and its objects reach each other the runtime's way or the stage pipeline's, never a
    // mix: an object the runtime cannot write (its class is another package's, the runtime does not
    // read that one yet) keeps the whole stage on the pipeline's scripts
    for r in &input.rows { object_files(&src, dir, r)?; }
    // the stage's own art is one animation of its entity, showing the stage clip's frames
    let entity = dir.join("library").join("entities").join(format!("{id}.entity"));
    let frames = src.sprites.get(&sym).map(|s| s.frame_count as u32).unwrap_or(1).max(1);
    let pieces: Vec<Piece> = entity_animations(&entity).contains(input.anim).then(|| Piece {
        fm_name: input.anim.to_string(), label: String::new(), sym, from: 1, frames, head: 0,
        reversed: false, borrowed: false, item_box: Vec::new(), on: None,
    }).into_iter().collect();
    let scaling = &crate::mappings::character_stats().scaling;
    let units = Units { size: model.scale, speed: scaling.velocity_scale(), item_toss: scaling.item_toss_speed, accel: scaling.accel_scale() };
    let none = Value::Null;
    // what the stage's classes declare they fire, by the name their code fires each under: every
    // script of the stage finds them there (`DATA.pkg.projectiles`)
    let projectiles = declared_projectiles(&src, &runtime, std::iter::once(class.as_str()).chain(input.rows.iter().map(|r| r.class.as_str())));
    let decls = Decls { attacks: &none, projectiles: &projectiles, items: &none, objects: input.objects.clone() };
    let effects = BTreeMap::new();
    let entities = BTreeMap::new();
    let rows: Vec<&ObjectRow> = input.rows.iter().collect();
    let tables = format!("{}{}", object_tables(&src, &runtime, &rows), clip_bounds_data(model, &src));
    let spec = ObjectSpec {
        kind: "stage",
        others: &[],
        effects: &effects,
        entities: &entities,
        class,
        clip,
        clip_sym: Some(sym),
        stats: Value::Null,
        autolink: BTreeMap::new(),
        data: format!("{}{tables}", stage_data(model, &src, input)),
    };
    let out = object_script(&src, &runtime, &spec, &decls, &pieces, units, id);
    let scripts = dir.join("library").join("scripts").join("stage");
    std::fs::write(scripts.join(format!("{id}Script.hx")), &out.script)?;
    // the entity's keyframes run the stage clip's frame scripts, where its frames have any (a clip
    // with none follows the engine's frame at each update)
    if pieces.iter().any(|p| src.class_of(p.sym).is_some_and(|c| !c.frame_scripts.is_empty())) {
        clock_entity(&entity, id, &src, &pieces, &out.frames, &BTreeMap::new(), None)?;
    }
    // each object, run by its own class
    let object_runtime = Runtime::load(&located).for_kind("object");
    let mut reports = serde_json::Map::new();
    let mut written = Vec::new();
    for row in &input.rows {
        let report = match runtime_object(&src, &object_runtime, dir, row, &decls, &tables, units, input.origin) {
            Ok(Some(o)) => {
                written.push(row.id.clone());
                serde_json::json!({ "problems": o.problems, "unknown": o.unknown, "unplaced": o.unplaced })
            }
            Ok(None) => serde_json::json!({ "problems": [format!("{}: its class is not in this package", row.class)] }),
            Err(e) => serde_json::json!({ "problems": [format!("its script stays the stage pipeline's: {e}")] }),
        };
        reports.insert(row.id.clone(), report);
    }
    // each projectile they fire, run by its own class too
    let projectile_runtime = Runtime::load(&located).for_kind("stage_projectile");
    let mut fired = serde_json::Map::new();
    for (key, result) in runtime_projectiles(&src, &projectile_runtime, dir, &decls, &tables, units, input.origin) {
        let report = match result {
            Ok(o) => serde_json::json!({ "problems": o.problems, "unknown": o.unknown, "unplaced": o.unplaced }),
            Err(e) => serde_json::json!({ "problems": [format!("its script stays the stage pipeline's: {e}")] }),
        };
        fired.insert(key, report);
    }
    let report = serde_json::json!({
        "problems": out.problems,
        "unplaced": renamed,
        "unknown": out.unknown,
        "objects": reports,
        "projectiles": fired,
    });
    std::fs::write(dir.join("runtime_report.json"), serde_json::to_string_pretty(&report)?)?;
    Ok((out, written))
}

/// What one object's script came to: what could not be written, the names nothing answers, and
/// the animations no source frames are found for.
struct ObjectOut {
    problems: Vec<String>,
    unknown: BTreeSet<String>,
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

/// One object's script, over the one at `scripts/hazard/<id>Script.hx`: its class, on the clip
/// that is its art, with the animations its entity has as the clip's frames. None when its class
/// is not the package's own.
#[allow(clippy::too_many_arguments)]
fn runtime_object(src: &SourceModel, runtime: &Runtime, dir: &Path, row: &ObjectRow, decls: &Decls,
                  tables: &str, units: Units, origin: (f64, f64)) -> anyhow::Result<Option<ObjectOut>> {
    let Some(class) = class_of(src, &row.class) else { return Ok(None) };
    let lib = dir.join("library");
    let (entity, script, root) = object_files(src, dir, row)?;
    let clip_name = root.and_then(|r| src.symbols.get(&r).cloned()).unwrap_or_else(|| row.clips.first().cloned().unwrap_or_default());
    unreserve_animations(&lib, &row.id)?;
    if let Some((dx, dy)) = row.registration { shift_entity(&entity, dx, dy)?; }
    let (pieces, unplaced) = root.map(|r| object_pieces(src, &entity, r)).unwrap_or_default();
    // the engine starts the object on the animation that shows its clip's first frame, where the
    // runtime starts the clip
    let first_label = root.and_then(|r| src.sprites.get(&r)).and_then(|s| s.labels.iter().find(|(f, _)| *f == 0)).map(|(_, l)| l.clone());
    if let Some(first) = pieces.iter().find(|p| (Some(p.sym) == root && p.from == 1) || (Some(p.sym) != root && Some(&p.label) == first_label.as_ref())) {
        starting_animation(&lib, &row.id, &first.fm_name)?;
    }
    let body = row.body.as_ref().map(|b| {
        let faces: Vec<String> = b.faces.iter().filter(|(_, pts)| pts.len() >= 2).map(|(ty, pts)| format!("[StructureType.{ty}, [{}]]",
            pts.iter().flat_map(|(x, y)| [format!("{x:.1}"), format!("{y:.1}")]).collect::<Vec<_>>().join(", "))).collect();
        format!("{{ box: [{:.1}, {:.1}, {:.1}, {:.1}], faces: [{}], ledges: [{}, {}] }}",
            b.bounds.0, b.bounds.1, b.bounds.2, b.bounds.3, faces.join(", "), b.ledges.0, b.ledges.1)
    }).unwrap_or_else(|| "null".into());
    let none = Value::Null;
    let decls = Decls { attacks: &none, projectiles: decls.projectiles, items: &none, objects: decls.objects.clone() };
    let effects = BTreeMap::new();
    let entities = BTreeMap::new();
    let spec = ObjectSpec {
        kind: "object",
        others: &[],
        effects: &effects,
        entities: &entities,
        class: class.name.clone(),
        clip: clip_name,
        clip_sym: root,
        stats: Value::Null,
        autolink: BTreeMap::new(),
        data: format!("DATA.origin = {{ x: {:.4}, y: {:.4} }};\nDATA.object.body = {body};\nDATA.object.at = {};\n{tables}{}",
            origin.0, origin.1, row.at.map(|(x, y)| format!("{{ x: {x:.4}, y: {y:.4} }}")).unwrap_or_else(|| "null".into()),
            object_invoke(src, runtime, &class.name)),
    };
    let out = object_script(src, runtime, &spec, &decls, &pieces, units, &row.id);
    std::fs::write(&script, &out.script)?;
    super::native_entity(&entity, &row.id, src, &pieces, &out)?;
    Ok(Some(ObjectOut { problems: out.problems, unknown: out.unknown, unplaced }))
}

/// What some of the package's classes declare they fire (`getProjectileStats`, the nearest in
/// each one's chain), merged by name: the first class to declare a name keeps it.
fn declared_projectiles<'a>(src: &SourceModel, runtime: &Runtime, classes: impl Iterator<Item = &'a str>) -> Value {
    let pkg = compile::Package::new(src, runtime);
    let mut out: Vec<(String, Value)> = Vec::new();
    for class in classes {
        let Some(c) = class_of(src, class) else { continue };
        let declared = pkg.chain(&c.name).0.into_iter()
            .find_map(|k| k.methods.iter().find(|m| m.name == "getProjectileStats" && !m.is_static))
            .and_then(|m| crate::model::declare::returned(&m.function.body));
        for (name, decl) in declared.as_ref().map(|v| v.entries()).unwrap_or_default() {
            if !out.iter().any(|(n, _)| n == name) { out.push((name.clone(), decl.clone())) }
        }
    }
    // ...and a projectile some code fires by its class (`spawnProjectile(CharmanderFire, owner)`):
    // its own stats are its declaration, under its class's name, and it runs that class
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
            // (the model reads the source's `getOwnStats` under the runtime's name for it)
            .find_map(|k| k.methods.iter().find(|m| (m.name == "ownStats" || m.name == "getOwnStats") && !m.is_static))
            .and_then(|m| crate::model::declare::returned(&m.function.body));
        let Some(Value::Obj(mut fields)) = own else { continue };
        if !fields.iter().any(|(k, _)| k == "classAPI") { fields.push(("classAPI".to_string(), Value::Class(x.clone()))); }
        out.push((x, Value::Obj(fields)));
    }
    if out.is_empty() { Value::Null } else { Value::Obj(out) }
}

/// Each projectile the stage's classes declare, over the script the stage pipeline wrote for it
/// at `scripts/Projectile/`: its class (the one its declaration names, else the runtime's), on
/// the clip its declaration names, with the animations its entity has. Its entity's own frame
/// code goes, since the runtime runs its clip's frames now. Returned by the clip it draws.
#[allow(clippy::too_many_arguments)]
fn runtime_projectiles(src: &SourceModel, runtime: &Runtime, dir: &Path, decls: &Decls, tables: &str,
                       units: Units, origin: (f64, f64)) -> Vec<(String, anyhow::Result<ObjectOut>)> {
    let lib = dir.join("library");
    // the pieces the stage pipeline cut for each projectile's entity, by the clip it draws
    let mut pieces_of = crate::entity_gen::take_projectile_pieces();
    let mut out = Vec::new();
    for (name, decl) in decls.projectiles.entries() {
        let Some(linkage) = decl.get("linkage_id").and_then(|v| v.str()) else { continue };
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
                kind: "projectile",
                others: &[],
                effects: &effects,
                entities: &entities,
                class: decl.get("classAPI").and_then(|v| v.class()).unwrap_or("PeptideProjectile").to_string(),
                clip: linkage.to_string(),
                clip_sym: None,
                stats: decl.clone(),
                autolink: BTreeMap::new(),
                data: format!("DATA.origin = {{ x: {:.4}, y: {:.4} }};\n{tables}", origin.0, origin.1),
            };
            let emitted = object_script(src, runtime, &spec, decls, &pieces, units, name);
            std::fs::write(&script, &emitted.script)?;
            without_frame_code(&entity)?;
            super::native_entity(&entity, &key, src, &pieces, &emitted)?;
            Ok(ObjectOut { problems: emitted.problems, unknown: emitted.unknown,
                unplaced: missing.iter().map(|p| format!("{} has no such animation", p.fm_name)).collect() })
        })();
        out.push((key, result));
    }
    out
}

/// An entity's frame code taken off (its `Scripts` layers' keyframes left empty): the runtime runs
/// the clip's frames instead, from the keyframes `clock_entity` writes.
fn without_frame_code(path: &Path) -> anyhow::Result<()> {
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let ids: BTreeSet<String> = json["layers"].as_array().into_iter().flatten()
        .filter(|l| l["type"] == "FRAME_SCRIPT" && l["name"] == "Scripts")
        .flat_map(|l| l["keyframes"].as_array().cloned().unwrap_or_default())
        .filter_map(|k| k.as_str().map(String::from)).collect();
    let mut changed = false;
    for k in json["keyframes"].as_array_mut().into_iter().flatten() {
        if k["$id"].as_str().is_some_and(|id| ids.contains(id)) && k["code"].as_str().is_some_and(|c| !c.is_empty()) {
            k["code"] = serde_json::json!("");
            changed = true;
        }
    }
    if changed { std::fs::write(path, serde_json::to_string_pretty(&json)?)?; }
    Ok(())
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

/// One class's members as another script reaches them, numbered: its methods (a getter and a
/// setter by their accessors), then each field the engine keeps for it, as a write and a read.
fn members(pkg: &compile::Package, class: &str) -> (Vec<(String, usize)>, Vec<String>) {
    use crate::model::MethodKind;
    let mut methods: Vec<(String, usize)> = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    for c in pkg.chain(class).0 {
        for m in c.methods.iter().filter(|m| !m.is_static) {
            let name = match m.kind { MethodKind::Getter => format!("get_{}", m.name), MethodKind::Setter => format!("set_{}", m.name), MethodKind::Method => m.name.clone() };
            let id = compile::ident(&name);
            if !methods.iter().any(|(n, _)| *n == id) { methods.push((id, m.function.params.len())) }
        }
        for f in c.fields.iter().filter(|f| !f.is_static && pkg.stored.contains(&f.name)) {
            let id = compile::ident(&f.name);
            if !fields.contains(&id) { fields.push(id) }
        }
    }
    (methods, fields)
}

/// The tables every script of a stage reaches its objects through: the class each content was
/// converted from (`DATA.contentClass`), and each class's members as a stand-in holds them until
/// the object's own script has started (`DATA.faces`: `f(p, i)` is the member numbered `i`,
/// `c(p, j)` the field whose write is `j` and read `j + 1`; see `runtime/stage/bodies.hx`).
fn object_tables(src: &SourceModel, runtime: &Runtime, rows: &[&ObjectRow]) -> String {
    let pkg = compile::Package::new(src, runtime);
    let mut out = String::from("DATA.contentClass = new haxe.ds.StringMap();\nDATA.faces = new haxe.ds.StringMap();\n");
    let mut done: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        let Some(c) = class_of(src, &row.class) else { continue };
        let short = compile::short(&c.name).to_string();
        out.push_str(&format!("DATA.contentClass.set({}, {});\n", compile::quote(&row.id), compile::quote(&short)));
        if !done.insert(short.clone()) { continue }
        let (methods, fields) = members(&pkg, &c.name);
        // what it is, for a test of its class before it has started: its own classes, then the
        // runtime's it extends
        let (chain, base) = pkg.chain(&c.name);
        let mut is: Vec<String> = chain.iter().rev().map(|x| x.name.clone()).collect();
        let mut b = base;
        let mut api: Vec<String> = Vec::new();
        while let Some(n) = b.filter(|n| !api.contains(n)) {
            api.push(compile::short(&n).to_string());
            b = runtime.classes.get(compile::short(&n)).and_then(|rc| rc.extends.clone());
        }
        api.reverse();
        is.splice(0..0, api);
        let mut sets: Vec<String> = vec![format!("p.__is = [{}];", is.iter().map(|n| compile::quote(n)).collect::<Vec<_>>().join(", "))];
        sets.extend(methods.iter().enumerate().map(|(i, (n, _))| format!("p.{n} = f(p, {i});")));
        sets.extend(fields.iter().enumerate().map(|(k, n)| format!("p.__c_{n} = c(p, {});", methods.len() + 2 * k)));
        out.push_str(&format!("DATA.faces.set({}, function(p, f, c) {{ {} }});\n", compile::quote(&short), sets.join(" ")));
    }
    out
}

/// An object's own members by number (the same numbers `object_tables` gives them): what another
/// script asks of it, run in its own script.
fn object_invoke(src: &SourceModel, runtime: &Runtime, class: &str) -> String {
    let pkg = compile::Package::new(src, runtime);
    let (methods, fields) = members(&pkg, class);
    let mut cases: Vec<String> = methods.iter().enumerate().map(|(i, (n, params))| {
        let args: Vec<String> = (0..*params).map(|k| format!("a[{k}]")).collect();
        format!("case {i}: return me.{n}({});", args.join(", "))
    }).collect();
    for (k, n) in fields.iter().enumerate() {
        let j = methods.len() + 2 * k;
        cases.push(format!("case {j}: me.__c_{n}.set(a[0]); return null;"));
        cases.push(format!("case {}: return me.__c_{n}.get();", j + 1));
    }
    // as its script ends: what it holds becomes plain values on it (a getter's answer under the
    // property's name, a field out of its container) and its methods go, so another script still
    // holding it reads what it last was and asks nothing of a script that is gone
    let mut freeze: Vec<String> = Vec::new();
    for (n, params) in &methods {
        match n.strip_prefix("get_") {
            Some(p) if *params == 0 => freeze.push(format!("me.{} = me.{n}(); me.{n} = null;", compile::ident(p))),
            _ => freeze.push(format!("me.{n} = null;")),
        }
    }
    for n in &fields { freeze.push(format!("me.{n} = me.__c_{n}.get(); me.__c_{n} = null;")); }
    format!("DATA.object.invoke = function(me, i, a) {{ switch (i) {{ {} default: return null; }} }};\nDATA.object.freeze = function(me) {{ {} }};\n",
        cases.join(" "), freeze.join(" "))
}

/// `DATA.stage`: the boot, the regions the stage's code asks for by name (each as the rectangle
/// its placement drew and the point its clip is placed by, in this engine's units), the size the
/// camera-background pan containers centre on, and the camera backgrounds the package declares.
/// Then what the planes bind and draw: each backdrop element by the names over it and the labels
/// it plays, the named surfaces, the pieces with a friction or a push of their own, the weather's
/// particles, the clips the code makes from the library, where a label puts a placed object, and
/// the objects the stage's own code makes (the boot leaves them to it).
fn stage_data(model: &crate::stage_parser::StageModel, src: &crate::model::SourceModel, input: &StageInput) -> String {
    let list = |rows: Vec<String>| if rows.is_empty() { "[]".to_string() } else { format!("[\n\t{}\n]", rows.join(",\n\t")) };
    let strs = |v: &[String]| v.iter().map(|s| compile::quote(s)).collect::<Vec<_>>().join(", ");
    let elements = input.tables.elements.iter().map(|e| format!("{{ i: {}, path: [{}], labels: [{}] }}",
        e.index, strs(&e.path), strs(&e.labels))).collect();
    let surfaces = input.surfaces.iter().map(|(n, s)| format!("{{ n: {}, s: [{}] }}", compile::quote(n), strs(s))).collect();
    let particles = input.particles.iter().map(|(n, id)| format!("{{ n: {}, id: {} }}", compile::quote(n), compile::quote(id))).collect();
    // each field's particles as a variable of the stage's own (the field's index, as its effect
    // is named), the list the weather fills
    let prefix = format!("{}_weather", model.id);
    let fields: Vec<(&str, &str)> = input.particles.iter()
        .filter_map(|(_, id)| id.strip_prefix(&prefix).map(|i| (id.as_str(), i))).collect();
    let lists: String = fields.iter().map(|(_, i)| format!("var m_weather{i} = [];\n")).collect();
    let weather_lists = fields.iter().map(|(id, i)| format!("{{ id: {}, l: m_weather{i} }}", compile::quote(id))).collect();
    let lib = input.lib.iter().map(|(n, parts)| format!("{{ n: {}, sym: {}, parts: [{parts}] }}", compile::quote(n),
        src.symbol_of(n).map(|s| s.to_string()).unwrap_or_else(|| "null".into()))).collect();
    let by_code: Vec<String> = input.rows.iter().filter(|r| r.by_code).map(|r| r.id.clone()).collect();
    format!("{lists}{}{}DATA.stage.weatherLists = {};\nDATA.stage.elements = {};\nDATA.stage.surfaces = {};\nDATA.stage.frictions = [{}];\nDATA.stage.pushes = [{}];\n\
DATA.stage.particles = {};\nDATA.stage.particleCap = {};\nDATA.stage.lib = {};\nDATA.stage.placedAt = [{}];\nDATA.stage.origin = {{ x: {:.4}, y: {:.4} }};\n\
DATA.origin = DATA.stage.origin;\nDATA.stage.byCode = [{}];\n",
        stage_boot_data(model, src, input), clip_bounds_data(model, src), list(weather_lists), list(elements), list(surfaces), input.frictions.join(", "), input.pushes.join(", "),
        list(particles), input.particle_cap, list(lib), input.placed_at, input.origin.0, input.origin.1, strs(&by_code))
}

/// `DATA.clipBounds`: the extent of each library clip the code names in a string (source pixels,
/// its own space), for a surface the code makes the shape of a clip (`createSelfPlatformWithMC`).
fn clip_bounds_data(model: &crate::stage_parser::StageModel, src: &crate::model::SourceModel) -> String {
    let mut named: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for c in src.classes.values() {
        for m in c.methods.iter().map(|m| &m.function).chain([&c.constructor]) {
            crate::model::ast::walk_exprs(&m.body, &mut |e| if let crate::model::ast::Expr::Str(t) = e { named.insert(t.clone()); });
        }
    }
    let mut out = String::from("DATA.clipBounds = new haxe.ds.StringMap();\n");
    // ...or one a name the code builds starts as (sand ocean asks for `"terrain_" + vehicleName`)
    let asked = |n: &str| named.contains(n) || named.iter().any(|t| t.len() >= 3 && n.starts_with(t.as_str()));
    for (n, r) in model.clip_bounds.iter().filter(|(n, _)| asked(n)) {
        out.push_str(&format!("DATA.clipBounds.set({}, {{ x: {:.2}, y: {:.2}, w: {:.2}, h: {:.2} }});\n", compile::quote(n), r.x, r.y, r.w, r.h));
    }
    // the box of each clip a timeline places under a name, by symbol: what `getRect` reads
    let placed: std::collections::BTreeSet<u16> = src.sprites.values().chain([&src.root])
        .flat_map(|sp| sp.frames.iter().flatten()).filter(|p| p.name.is_some()).map(|p| p.char_id).collect();
    out.push_str("DATA.symBounds = new haxe.ds.IntMap();\n");
    for id in placed {
        if let Some(r) = model.sprite_bounds.get(&id) {
            out.push_str(&format!("DATA.symBounds.set({id}, {{ x: {:.2}, y: {:.2}, w: {:.2}, h: {:.2} }});\n", r.x, r.y, r.w, r.h));
        }
    }
    out
}

/// The boot, the regions, the pan size and the camera backgrounds.
fn stage_boot_data(model: &crate::stage_parser::StageModel, src: &crate::model::SourceModel, input: &StageInput) -> String {
    let regions: Vec<String> = model.named_boxes.iter().map(|(n, r)| {
        let (rx, ry) = model.named_regs.get(n).copied().unwrap_or((r.x, r.y));
        format!("{{ n: {}, x: {:.1}, y: {:.1}, w: {:.1}, h: {:.1}, rx: {rx:.1}, ry: {ry:.1} }}", compile::quote(n), r.x, r.y, r.w, r.h)
    }).collect();
    let regions = if regions.is_empty() { "[]".to_string() } else { format!("[\n\t{}\n]", regions.join(",\n\t")) };
    let linkages: Vec<Option<String>> = src.declared("camera").as_ref().and_then(|c| c.get("backgrounds").cloned())
        .map(|b| match b { Value::Arr(items) => items, _ => Vec::new() }).unwrap_or_default().iter()
        .map(|b| b.get("linkage_id").and_then(|v| v.str()).map(String::from)).collect();
    let cam_bgs: Vec<String> = linkages.iter().map(|l| l.as_deref().map(compile::quote).unwrap_or_else(|| "null".into())).collect();
    // each one's own clip, for the code that reaches into it (galaxy tours picks its backdrop by
    // sending a clip inside it to a frame)
    let cam_syms: Vec<String> = linkages.iter().map(|l| l.as_deref().and_then(|l| src.symbol_of(l))
        .map(|s| s.to_string()).unwrap_or_else(|| "null".into())).collect();
    // the track the stage declares, which its code can time itself by (final destination)
    let music = model.ssf2_music.first().map(|m| compile::quote(m)).unwrap_or_else(|| "null".into());
    format!("DATA.stage = {{}};\nDATA.stage.boot = {};\nDATA.stage.regions = {regions};\nDATA.stage.pan = {{ x: {:.2}, y: {:.2} }};\nDATA.stage.cameraBackgrounds = [{}];\nDATA.stage.cameraBackgroundClips = [{}];\nDATA.stage.music = {music};\n",
        input.tables.boot(), input.pan.0, input.pan.1, cam_bgs.join(", "), cam_syms.join(", "))
}
