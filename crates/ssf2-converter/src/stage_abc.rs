//! stage_abc — recover a stage's layer assignments + spawned actors from its AS3.
//!
//! the stage analog of the character `extract_character` path. every shipped SSF2 stage has a
//! document class (named = the stage id) that extends `SSF2Stage`; its `initialize` method is the
//! "get()" the author wrote: it reads named child clips off the plane accessors
//! (`getBackground()/getForeground()/getMidground()/getCameraBackgrounds()`) and bundles them for
//! the engine, and it engine-spawns the hazards/props (`SSF2API.spawnEnemy(<Class>)` + setX/setY).
//!
//! we recover both by running ONE stack-sim pass (the shared `abc_parser::scan_method` + the
//! property/lex/locals hooks) over `initialize` (and `update`, for hazards spawned lazily there).
//! the result is authoritative: clip-linkage-name -> plane, replacing the name heuristics, and a
//! list of spawned actors with their literal coords, replacing hand-declared hazard metadata.

use crate::abc_parser::{AbcFile, AbcVisitor, Class, MethodBody, StackVal, scan_method};
use std::collections::BTreeMap;

/// Which render plane a stage clip belongs to (from the `getX()` accessor it was read off).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StagePlane {
    Background,
    Foreground,
    Midground,
    /// camera-background parallax (`getCameraBackgrounds()[i].mc.<clip>`).
    CameraBg,
}

impl StagePlane {
    /// the lowercase plane name the stage parser's `plane_tag`/`art_kind` use
    /// (`background`/`foreground`/`midground`/`cambg`).
    pub fn code(self) -> &'static str {
        match self {
            StagePlane::Background => "background",
            StagePlane::Foreground => "foreground",
            StagePlane::Midground => "midground",
            StagePlane::CameraBg => "cambg",
        }
    }
    fn from_code(s: &str) -> Option<StagePlane> {
        Some(match s {
            "background" => StagePlane::Background,
            "foreground" => StagePlane::Foreground,
            "midground" => StagePlane::Midground,
            "cambg" => StagePlane::CameraBg,
            _ => return None,
        })
    }
    /// the plane an `SSF2Stage` accessor method returns, if it is one.
    fn of_accessor(method: &str) -> Option<StagePlane> {
        Some(match method {
            "getBackground" => StagePlane::Background,
            "getForeground" => StagePlane::Foreground,
            "getMidground" => StagePlane::Midground,
            "getCameraBackgrounds" => StagePlane::CameraBg,
            _ => return None,
        })
    }
}

/// An engine-spawned actor (`SSF2API.spawnEnemy(<Class>)`), with its literal spawn coords if the
/// AS3 set them with constant `setX`/`setY` (dynamic/random positions stay `None`).
#[derive(Clone, Debug)]
pub struct SpawnedActor {
    pub class_name: String,
    /// The api base this class extends -- `SSF2Enemy`, `SSF2Item`, `SSF2Projectile` and so on.
    /// This is WHAT THE OBJECT IS, taken from the class graph rather than guessed from its name.
    pub base: String,
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// The hazard CLASS's own declarations (the "get()" methods the author wrote): each
    /// `getAttackStats` hitbox map (damage/direction/power/kbConstant — the authoritative hit
    /// params, not a generic per-kind default) and the `getOwnStats` scalars (width/height/speeds).
    /// Empty when the class has neither getter.
    pub attack_hitboxes: Vec<std::collections::BTreeMap<String, f64>>,
    /// The same boxes, under the animation the class declares them for. Empty when the class states
    /// its hit without naming an animation.
    pub attack_by_anim: std::collections::BTreeMap<String, Vec<std::collections::BTreeMap<String, f64>>>,
    /// How often the class says each of those hits comes round again (`refreshRate`).
    pub attack_rates: std::collections::BTreeMap<String, f64>,
    pub own_stats: std::collections::BTreeMap<String, f64>,
    /// The yes/no declarations from the same place: whether fighters pass through it, whether a
    /// hit sends it flying. The source saying how the object behaves.
    pub own_flags: std::collections::BTreeMap<String, bool>,
    /// The projectiles the class declares (`getProjectileStats`), by the name it fires them under.
    pub projectiles: std::collections::BTreeMap<String, crate::abc_parser::ProjectileData>,
    /// The library clip the class names for itself (`getOwnStats().linkage_id`) -- the class saying
    /// which art it is, so its clip and collision box are found from what it declares.
    pub linkage_id: Option<String>,
    /// Animation labels the class plays via `forceAttack("<label>")` (Thwomp: entrance/idle/fall;
    /// HyruleTornado: stand) — the code-referenced handle to its art clip (the clip carrying those
    /// frame labels), so the art is found by what the script animates, not a library keyword match.
    pub anim_labels: Vec<String>,
    /// Behavior values stepped out of the class's update()/initialize() (shake amplitude, rise
    /// speed, fall gravity, self-platform, dust, sounds) — drives the FM CGO script from the
    /// hazard's own code instead of a template of constants.
    pub behavior: crate::abc_parser::EnemyBehavior,
    /// The class's lifecycle methods reconstructed as FM hscript via the character/projectile
    /// decompile→translate pipeline (the real update() state machine). Still needs a field-state +
    /// FrameTimer pass to run; used by the gated reconstruction path. None if no such methods.
    pub reconstructed_script: Option<String>,
    /// The engine-spawned faller cycle (delays/period/columns) stepped from the enemy + stage code.
    pub faller: Option<crate::abc_parser::FallerCycle>,
    /// Which shared package this class was found in, when the stage package does not define it.
    /// The art it names lives there too, so the index is carried until the clip is read.
    pub external_pkg: Option<usize>,
}

/// What the AS3 says about a stage: the authoritative plane map + the spawned actors.
#[derive(Clone, Debug, Default)]
pub struct StageAbcModel {
    /// clip linkage/instance name -> the plane it was assigned to in `initialize`.
    pub planes: BTreeMap<String, StagePlane>,
    /// engine-spawned hazards/props from `initialize` + `update`.
    pub actors: Vec<SpawnedActor>,
    /// the document class extending `SSF2Stage` (= the stage id).
    pub doc_class: String,
    /// The stage class's OWN behaviour, decompiled and translated.
    ///
    /// A stage is not a passive backdrop: its class decides when a thwomp drops and onto which
    /// column, when the clock advances, when it rains. That code is the stage, and reading it is
    /// the only way a port can be faithful about WHEN things happen -- the alternative is the
    /// converter re-implementing each stage's cadence from constants it stepped out, which is a
    /// guess dressed as data.
    pub reconstructed_script: Option<String>,
    /// Classes the stage's own code makes objects of that this package does not define.
    ///
    /// The game keeps shared packages loaded alongside, so a stage can name a class it does not
    /// carry and have it resolve. Read from the stage's own spawn calls, so it says nothing about
    /// WHAT the class is -- only that the stage asked for one and the package cannot answer.
    pub external_classes: Vec<String>,
}

/// the document class is the one extending `SSF2Stage`. (mirror of the character path's
/// "locate the entry class" step in `find_bundle_method`.)
pub fn find_stage_class(abc: &AbcFile) -> Option<&Class> {
    abc.classes.iter().find(|c| c.super_name == "SSF2Stage")
}

/// resolve an instance method's body by name (mirror of `find_bundle_method`'s trait->body step).
fn method_body<'a>(abc: &'a AbcFile, class: &Class, name: &str) -> Option<&'a MethodBody> {
    let t = class.instance_methods.iter().find(|t| t.name == name)?;
    abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx)
}

/// run the plane + actor recovery over a stage's `initialize` (+ `update`).
pub fn extract_stage(abc: &AbcFile) -> Option<StageAbcModel> {
    let class = find_stage_class(abc)?;
    let mut v = StageVisitor::default();
    if let Some(body) = method_body(abc, class, "initialize") {
        scan_method(&body.bytecode, abc, &mut v);
    }
    // hazards spawned lazily (e.g. bowserscastle's Thwomp) live in `update`.
    if let Some(body) = method_body(abc, class, "update") {
        scan_method(&body.bytecode, abc, &mut v);
    }
    // Every game object the package DECLARES is an object to port. The spawn-site scan above can
    // only see an actor when the class reference and the `spawnEnemy` call are close enough for the
    // stack sim to carry the tag between them, so it finds clocktown's TingleBalloon (three opcodes
    // apart) and misses its ClockTownFairy (two hundred). The class graph has no such gap: it says
    // what the package's objects ARE. The scan is kept for what only it knows -- the literal spawn
    // coordinates at the call site.
    let declared = crate::ssf2_objects::game_object_classes(&abc.classes);
    for go in &declared {
        match v.actors.iter_mut().find(|a| a.class_name == go.class_name) {
            Some(found) => found.base = go.base.clone(),
            None => v.actors.push(SpawnedActor {
                class_name: go.class_name.clone(), base: go.base.clone(), x: None, y: None,
                attack_hitboxes: Vec::new(), attack_by_anim: Default::default(), attack_rates: Default::default(), own_stats: Default::default(), own_flags: Default::default(), projectiles: Default::default(), linkage_id: None, anim_labels: Vec::new(),
                behavior: Default::default(), reconstructed_script: None, faller: None, external_pkg: None }),
        }
    }

    // Pull each spawned hazard's OWN declarations from its class's get* methods — the authoritative
    // damage/hit params + size/speeds, parsed straight from the SSF2 source instead of guessed.
    let mut actors = v.actors;
    for a in &mut actors {
        a.attack_hitboxes = crate::abc_parser::extract_attack_stats_for(abc, &a.class_name);
        a.attack_by_anim = crate::abc_parser::extract_attack_stats_by_anim(abc, &a.class_name);
        a.attack_rates = crate::abc_parser::extract_attack_rates_by_anim(abc, &a.class_name);
        a.own_stats = crate::abc_parser::extract_own_stats_for(abc, &a.class_name);
        a.own_flags = crate::abc_parser::extract_own_flags_for(abc, &a.class_name);
        a.projectiles = crate::abc_parser::extract_projectiles_for(abc, &a.class_name);
        a.linkage_id = crate::abc_parser::extract_own_linkage_for(abc, &a.class_name);
        a.anim_labels = crate::abc_parser::extract_force_attack_labels(abc, &a.class_name);
        a.behavior = crate::abc_parser::extract_enemy_behavior(abc, &a.class_name);
        // its projectiles' declared launch and offsets, live while its own spawns are rewritten;
        // the launch is scaled at the spawn (the boundary pass runs before the spawn is built)
        let _launch = crate::api_mappings::ProjectileLaunchGuard::from_declarations(&a.projectiles, false);
        a.reconstructed_script = {
            let _stage = crate::api_mappings::StageContentGuard::install();
            crate::abc_parser::reconstruct_enemy_script(abc, &a.class_name)
        };
        if std::env::var("PEPTIDE_DUMP_RECON").is_ok() {
            if let Some(src) = a.reconstructed_script.as_deref() {
                eprintln!("[recon {}]\n{src}", a.class_name);
            }
        }
        a.faller = crate::abc_parser::extract_faller_cycle(abc, &a.class_name);
    }
    // this class runs AS the stage, so a request for the stage is a request for itself
    let mut reconstructed_script = {
        let _stage = crate::api_mappings::SelfIsStageGuard::install();
        let _content = crate::api_mappings::StageContentGuard::install();
        crate::abc_parser::reconstruct_class_script(abc, &class.name)
    };
    // A PLAIN CLASS THE STAGE BUILDS FOR ITSELF. Not a game object, not a hazard -- a helper it
    // makes with `new X()` and then talks to. Nothing ported it, so the make came back with
    // nothing and every call on it was refused: a target test builds its whole target set that
    // way and asked it, once a frame, whether it was ready. The character port already turns
    // such a class into a factory; the same reconstruction serves a stage. Which classes is not
    // a question anyone has to answer -- the stage's own code says which ones it builds.
    if let Some(src) = reconstructed_script.as_mut() {
        let built: std::collections::BTreeSet<String> = regex::Regex::new(r"\bnew\s+([A-Za-z_]\w*)\s*\(")
            .expect("new regex")
            .captures_iter(src).map(|c| c[1].to_string()).collect();
        let mut factories = String::new();
        for name in &built {
            if !abc.classes.iter().any(|k| k.name == *name) { continue }
            // a helper runs inside the stage's script, so its reaches for "the stage" and through
            // its planes are the stage's own (pokemon colosseum's stadium keeps the midground)
            let factory = {
                let _stage = crate::api_mappings::SelfIsStageGuard::install();
                crate::abc_parser::reconstruct_class_as_factory(abc, name)
            };
            let Some(factory) = factory else { continue };
            log::info!("class '{name}' the stage builds for itself, ported as a factory");
            if std::env::var("PEPTIDE_DUMP_FACTORY").is_ok() { eprintln!("[factory {name}]\n{factory}"); }
            factories.push_str(&factory);
            factories.push('\n');
            *src = src.replace(&format!("new {name}("), &format!("{name}__new("));
        }
        if !factories.is_empty() { *src = format!("{factories}\n{src}"); }
    }
    // A PROJECTILE ANY OF THE PACKAGE'S CODE FIRES BY ITS CLASS (`spawnProjectile(CharmanderFire,
    // owner)` in a stance's frame script) ships like one a class declares: its own declarations are
    // what it is, and it goes under its class's name, the name the code fires it by.
    {
        #[derive(Default)]
        struct Fired { classes: std::collections::BTreeSet<String> }
        impl crate::abc_parser::AbcVisitor for Fired {
            fn on_getlex(&mut self, name: &str) -> Option<crate::abc_parser::StackVal> {
                Some(crate::abc_parser::StackVal::Tag(format!("lex:{name}")))
            }
            fn on_callproperty(&mut self, method: &str, args: &[crate::abc_parser::StackVal], _r: &crate::abc_parser::StackVal) -> Option<crate::abc_parser::StackVal> {
                if method == "spawnProjectile" {
                    if let Some(crate::abc_parser::StackVal::Tag(t)) = args.first() {
                        if let Some(c) = t.strip_prefix("lex:") { self.classes.insert(c.to_string()); }
                    }
                }
                None
            }
        }
        let mut fired = Fired::default();
        for body in &abc.method_bodies { crate::abc_parser::scan_method(&body.bytecode, abc, &mut fired); }
        for class in fired.classes.iter().filter(|c| abc.classes.iter().any(|k| k.name == **c)) {
            if actors.iter().any(|a| a.projectiles.contains_key(class)) { continue }
            let mut names = std::collections::BTreeMap::new();
            if let Some(l) = crate::abc_parser::extract_own_linkage_for(abc, class) { names.insert("linkage_id".to_string(), l); }
            if names.is_empty() { continue }
            let pd = crate::abc_parser::ProjectileData {
                stats: crate::abc_parser::extract_own_stats_for(abc, class),
                flags: crate::abc_parser::extract_own_flags_for(abc, class),
                names,
                hitboxes: crate::abc_parser::extract_attack_stats_for(abc, class),
            };
            if let Some(a) = actors.first_mut() { a.projectiles.insert(class.clone(), pd); }
        }
    }
    // What the stage asks for and this package cannot answer. The spawn call names the class, so
    // the ask is read straight off the stage's own code.
    let mut external_classes: Vec<String> = Vec::new();
    if let Some(src) = reconstructed_script.as_deref() {
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let re = RE.get_or_init(|| regex::Regex::new(
            r#"(?:spawnEnemy|spawnItem)\(\s*"?([A-Za-z_]\w*)"?\s*\)"#).expect("spawn regex"));
        for c in re.captures_iter(src) {
            let name = c[1].to_string();
            if actors.iter().any(|a| a.class_name == name) { continue }
            if abc.classes.iter().any(|k| k.name == name) { continue }
            if external_classes.contains(&name) { continue }
            external_classes.push(name);
        }
    }
    // WHAT THIS STAGE IS MADE OF, said once per stage so the same question can be asked of all of
    // them at once rather than read out of a tree by eye: how many objects it declares, how many
    // of them carry their own code across, and whether the stage's own class did.
    //
    // The last count is what the stage NAMES and this file does not define. That is not a gap by
    // itself -- a stage can ask for a class the game keeps loaded elsewhere, and those are looked
    // for in the other packages further on -- so it is counted here as a question, not a loss.
    {
        let with_code = actors.iter().filter(|a| a.reconstructed_script.is_some()).count();
        let borrowed = actors.iter().filter(|a| a.external_pkg.is_some()).count();
        log::info!(
            "stage audit: objects={} with-own-code={} borrowed-from-another-package={} \
             stage-class-ported={} named-but-not-in-this-file={}",
            actors.len(), with_code, borrowed,
            reconstructed_script.is_some(), external_classes.len());
    }
    Some(StageAbcModel { planes: v.planes, actors, doc_class: class.name.clone(), reconstructed_script,
        external_classes })
}

/// stack-sim visitor: tags plane-accessor results + getlex class names so the chained
/// `getProperty <clip>` reads bind a clip to a plane, and `spawnEnemy` + `setX/setY` record actors.
#[derive(Default)]
struct StageVisitor {
    planes: BTreeMap<String, StagePlane>,
    actors: Vec<SpawnedActor>,
}

impl AbcVisitor for StageVisitor {
    fn track_locals(&self) -> bool { true }

    fn on_getlex(&mut self, name: &str) -> Option<StackVal> {
        // carry the resolved name so a following `spawnEnemy(<Class>)` can read its argument.
        Some(StackVal::Tag(format!("lex:{name}")))
    }

    fn on_callproperty(&mut self, method: &str, args: &[StackVal], receiver: &StackVal) -> Option<StackVal> {
        // a plane accessor -> tag its result so the chained getproperty binds clips to this plane.
        if let Some(plane) = StagePlane::of_accessor(method) {
            return Some(StackVal::Tag(format!("plane:{}", plane.code())));
        }
        // SSF2API.spawnEnemy(<Class>) -> a new actor; tag the result for the following setX/setY.
        // (an item it spawns by class is one too: it runs its own class as an object)
        if method == "spawnEnemy" || method == "spawnProjectile" || method == "spawnItem" {
            if let Some(StackVal::Tag(t)) = args.first() {
                if let Some(class) = t.strip_prefix("lex:") {
                    let idx = self.actors.len();
                    self.actors.push(SpawnedActor { class_name: class.to_string(), base: String::new(), x: None, y: None,
                        attack_hitboxes: Vec::new(), attack_by_anim: Default::default(), attack_rates: Default::default(), own_stats: std::collections::BTreeMap::new(), own_flags: Default::default(), projectiles: Default::default(), linkage_id: None, anim_labels: Vec::new(),
                        behavior: crate::abc_parser::EnemyBehavior::default(), reconstructed_script: None, faller: None, external_pkg: None });
                    return Some(StackVal::Tag(format!("actor:{idx}")));
                }
            }
            return None;
        }
        // <actor>.setX(literal) / .setY(literal): record the spawn coords.
        if (method == "setX" || method == "setY") && !args.is_empty() {
            if let (StackVal::Tag(t), StackVal::Num(n)) = (receiver, &args[0]) {
                if let Some(idx) = t.strip_prefix("actor:").and_then(|s| s.parse::<usize>().ok()) {
                    if let Some(a) = self.actors.get_mut(idx) {
                        if method == "setX" { a.x = Some(*n); } else { a.y = Some(*n); }
                    }
                }
            }
        }
        None
    }

    fn on_getproperty(&mut self, prop: &str, receiver: &StackVal) -> Option<StackVal> {
        // a getproperty off a plane-tagged receiver binds that clip to the plane. propagate the tag
        // so a chain (`getCameraBackgrounds()[0].mc.<clip>`) keeps the plane through `[0]`/`.mc`.
        if let StackVal::Tag(t) = receiver {
            if let Some(plane) = t.strip_prefix("plane:").and_then(StagePlane::from_code) {
                if !prop.is_empty() {
                    // record only real clip linkage names, not array indexes / `mc` containers.
                    if prop != "mc" {
                        self.planes.entry(prop.to_string()).or_insert(plane);
                    }
                }
                return Some(StackVal::Tag(t.clone()));
            }
        }
        None
    }
}
