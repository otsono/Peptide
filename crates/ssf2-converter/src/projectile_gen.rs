//! Summoner-agnostic projectile generator.
//!
//! SSF2 projectiles are not standalone scripts: each is an instance of the
//! engine class `SSF2Projectile` (a thin state/owner/stats wrapper), and its
//! per-frame motion is simulated by the SSF2 engine from the `getProjectileStats()`
//! physics object. So there is no per-projectile behavior script to decompile;
//! the FM projectile script is synthesized from those stats (see
//! `haxe_gen::generate_projectile_*`).
//!
//! This module owns the ONE entry point for emitting a projectile's files. It is
//! deliberately decoupled from the character path: a character summons projectiles
//! today, but stages and items will summon them too. They all build a
//! [`ProjectileGenCtx`] and call [`generate_projectiles`]; nothing here knows or
//! cares what kind of file the owner is.

use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::abc_parser::ProjectileData;
use crate::entity_gen;
use crate::haxe_gen;
use crate::image_extractor::{DiscoveredProjectile, ImageExtractionResult};
use crate::uuid_gen::det_uuid;

/// The FM resource content id for a projectile, derived from its SSF2 name.
/// SINGLE source of truth: the spawn rewrite (`match.createProjectile(getContent(…))`),
/// the manifest entry, and the projectile's own Stats all resolve through here, so
/// a character, stage, or item summoning the same projectile always agrees on the id.
pub fn projectile_content_id(ssf2_name: &str) -> String {
    // Folded, because the source does not spell it the same way twice. Chibirobo declares
    // `chibi_dair_proj` and fires `chibidairProj`, and the source engine finds the same
    // projectile from either -- so a port that keeps the case as written builds two ids for
    // one thing, and the one the move actually spawns matches nothing. Eighteen of them
    // across seven characters, every one a move that fires nothing at all.
    format!("{}Projectile", ssf2_name.replace('_', "").to_lowercase())
}

/// Everything the projectile generator needs that is supplied by the SUMMONER
/// (a character today; a stage or item later). Keeping this owner-agnostic is the
/// whole point — `generate_projectiles` works the same regardless of who summons.
pub struct ProjectileGenCtx<'a> {
    /// Owner namespace id (char_id today; stage_id / item_id later). Drives
    /// content-id namespacing, costumes, and deterministic uuids.
    pub owner_id: &'a str,
    /// `<owner>/library/entities` — where the projectile `.entity` files land.
    pub entities_dir: &'a Path,
    /// `<owner>/library/scripts` — a `Projectile/` subdir is created under it.
    pub scripts_root: &'a Path,
    pub parsed_swf: &'a swf::Swf<'a>,
    pub img_result: &'a ImageExtractionResult,
    /// SSF2 `getProjectileStats()` physics, keyed by SSF2 attack/projectile name.
    pub projectile_data: &'a BTreeMap<String, ProjectileData>,
    /// The projectiles the summoner KEEPS: it fires one and stores what it fired, so it
    /// can go on steering it. Named by their SSF2 names, as the summoner's own code names
    /// them. A projectile nobody keeps is one that flies off and has to die on its own,
    /// which is what the default handlers below are for; a kept one is driven by whoever
    /// keeps it and must not be given them.
    pub kept_by_summoner: &'a std::collections::BTreeSet<String>,
    /// What the SUMMONER keeps its own state in. A projectile reads its summoner's state by
    /// name, as one object reading another's field, and here a script's variables are its own:
    /// the only way across is what the summoner publishes. So these are the names to read
    /// through what it published.
    pub owner_vars: &'a std::collections::BTreeSet<String>,
    /// And what the summoner can be ASKED: its own methods, which cross the same way its state
    /// does and are no more reachable by name than a variable is.
    pub owner_fns: &'a std::collections::BTreeSet<String>,
    /// And its GETTERS: read as plain fields in the source, answered by a call here.
    pub owner_getters: &'a std::collections::BTreeSet<String>,
    /// The summoner's animations by the source's names: a projectile that asks which move its
    /// summoner is in asks by the source's name, and the engine answers with this one's.
    pub owner_anims: &'a BTreeMap<String, String>,
    pub palette_collection_guid: Option<&'a str>,
    pub palette_base_map_id: Option<&'a str>,
    /// The runtime runs each clip's own frame scripts (a fighter's projectiles), so the entity
    /// carries none of them.
    pub runtime_runs_frames: bool,
}

/// Emit every projectile's `.entity` + Script/Stats/AnimationStats/HitboxStats
/// files. Layout matches the real-FM-mod convention: a single
/// `library/scripts/Projectile/` directory holds all projectiles, each file
/// prefixed by the projectile name in PascalCase.
/// Every ABC block in the package, parsed once. A projectile's behaviour is decompiled out of
/// these, and re-parsing per projectile would walk the whole package again for each one.
pub(crate) fn parse_abcs(swf: &swf::Swf<'_>) -> Vec<crate::abc_parser::AbcFile> {
    swf.tags.iter().filter_map(|t| match t {
        swf::Tag::DoAbc(data) => crate::abc_parser::parse(data).ok(),
        swf::Tag::DoAbc2(abc) => crate::abc_parser::parse(abc.data).ok(),
        _ => None,
    }).collect()
}

/// What a projectile's own code declares: the behaviour class its declaration names (when it is
/// the package's own and not the api's binding), the outer clip, and the clip it flies with
/// (`attack_idle`). A clip a later entry declares wins a name the outer clip also has: the
/// outer's `frame1` is usually just a `stop()`, the inner's is the wiring.
///
/// The clips of its OTHER states are read on their own (`state_behaviour`): merged in here, a
/// state's `frame7` overwrote the flight's, and sora's keyblade ended itself on the seventh
/// frame of its throw with the script that ends its return.
pub(crate) fn clip_behaviour(
    abcs: &[crate::abc_parser::AbcFile],
    proj: &DiscoveredProjectile,
    class_api: Option<&str>,
) -> BTreeMap<String, String> {
    // the clip's own class and its stance's: what the timeline binds, not what the fighter's
    // table calls the projectile (pikachu's `thunderJolt` is a clip of its own, ten frames of
    // another jolt's steering, and the projectile fired under that name is `thunderJolt2`; read
    // by name it ran both)
    let mut clips: Vec<String> = Vec::new();
    if let Some(c) = class_api { clips.push(c.to_string()); }
    clips.push(proj.class_name.clone());
    if let Some(inner) = &proj.inner_sprite_name { if !clips.contains(inner) { clips.push(inner.clone()); } }
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for abc in abcs {
        for c in &clips {
            for (name, code) in crate::abc_parser::extract_class_methods(abc, c) {
                // A FRAME BOTH CLIPS CARRY. The outer clip's frame and the inner's both run in
                // the source, each on its own timeline; here the object has one, so the two
                // bodies run together, the outer's first. Kept as a replacement, the inner's
                // `frame2` threw away the outer's, which is the frame that binds the object to
                // its api handle and its owner (pikachu's jolt read both as nothing after that)
                match out.get(&name) {
                    Some(prior) if crate::abc_parser::is_root_xframe_method(&name) => {
                        let merged = merge_frame_bodies(prior, &code);
                        out.insert(name, merged);
                    }
                    _ => { out.insert(name, code); }
                }
            }
        }
    }
    // A `stop()` on its own is the timeline parking itself, which the entity's animation already
    // does; carrying it over would emit an initialize that says nothing.
    if out.get("frame1").is_some_and(|c| c.lines().filter(|l| !l.trim().is_empty()).count() <= 3
        && c.contains("stop()")) {
        out.remove("frame1");
    }
    out
}

/// Two bodies of the same frame, one function: the second's statements after the first's,
/// less the return that ended the first.
fn merge_frame_bodies(first: &str, second: &str) -> String {
    let body_of = |f: &str| -> String {
        let open = f.find('{').map(|i| i + 1).unwrap_or(0);
        let close = f.rfind('}').unwrap_or(f.len());
        f[open..close.max(open)].to_string()
    };
    let head_end = first.find('{').map(|i| i + 1).unwrap_or(0);
    let head = &first[..head_end];
    let mut a = body_of(first);
    let b = body_of(second);
    // the return that ended the first body would end the merged one before the second runs
    let trimmed = a.trim_end();
    if let Some(stripped) = trimmed.strip_suffix("return;") { a = stripped.to_string(); }
    format!("{head}{a}\n{b}}}")
}

/// Everything a projectile's clips say, in one bag, for the passes that only READ its code (the
/// fields it writes on its summoner, the names it fires): the flight's and the outer clip's
/// methods under their own names, every other state's under `name@label`.
pub(crate) fn all_behaviour(
    abcs: &[crate::abc_parser::AbcFile],
    proj: &DiscoveredProjectile,
) -> BTreeMap<String, String> {
    let mut out = clip_behaviour(abcs, proj, None);
    for st in &proj.states {
        if st.label == "attack_idle" { continue }
        for (name, code) in state_behaviour(abcs, st) {
            out.insert(format!("{name}@{}", st.label), code);
        }
    }
    out
}

/// What one of a projectile's other states declares: the clip placed under that label, whose
/// frames run while the state does and whose handlers those frames name.
pub(crate) fn state_behaviour(
    abcs: &[crate::abc_parser::AbcFile],
    state: &crate::image_extractor::ProjectileState,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if state.inner_sprite_name.is_empty() { return out }
    for abc in abcs {
        for (name, code) in crate::abc_parser::extract_class_methods(abc, &state.inner_sprite_name) {
            out.insert(name, code);
        }
    }
    out
}

/// A timeline method's body as a frame script: `frameN` -> (N, the body translated), for the
/// frames whose scripts say something. `from` is the first frame kept: the flight's `frame1` is
/// its constructor and goes to initialize, a state's runs the moment the state starts.
fn frame_scripts_of(
    behaviour: &BTreeMap<String, String>,
    from: u16,
    translate: &dyn Fn(&str) -> String,
) -> BTreeMap<u16, String> {
    behaviour.iter()
        .filter_map(|(name, code)| {
            let n: u16 = name.strip_prefix("frame")?.parse().ok()?;
            if n < from { return None }
            let body = translate(code);
            // the decompiler hands back `function frameN() { … }`; the frame wants the body
            let inner = body.split_once('{').map(|(_, r)| r).unwrap_or(&body);
            let inner = inner.rsplit_once('}').map(|(l, _)| l).unwrap_or(inner);
            let inner = inner.trim_matches('\n').trim_end();
            (!inner.trim().is_empty()).then(|| (n, inner.to_string()))
        })
        .collect()
}

thread_local! {
    /// the structure contents the projectiles of this conversion gave themselves (a platform a
    /// clip stands fighters on): `(content id, stats script id, script id)`, drained into the
    /// manifest by whoever builds it
    static STRUCTURE_CONTENTS: std::cell::RefCell<Vec<(String, String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}
thread_local! {
    /// the projectiles this run wrote from a declaration, by where their scripts go
    static DECLARED: std::cell::RefCell<std::collections::BTreeSet<std::path::PathBuf>> = const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
}
/// whether a projectile is to be written: always when its character declares it, else only when
/// no character of this run declared it to the same place
fn declared_here(at: &std::path::Path, declared: bool) -> bool {
    DECLARED.with(|d| if declared { d.borrow_mut().insert(at.to_path_buf()); true } else { !d.borrow().contains(at) })
}
pub(crate) fn note_structure_content(id: String, stats_id: String, script_id: String) {
    STRUCTURE_CONTENTS.with(|c| c.borrow_mut().push((id, stats_id, script_id)));
}
pub fn take_structure_contents() -> Vec<(String, String, String)> {
    STRUCTURE_CONTENTS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// A platform the clip gives itself, as the source states it: `createSelfPlatform(x, y, w, h,
/// solid)`, the box's offset and size in source pixels. The call becomes `__selfPlatform` on the
/// structure content emitted for it, and the numbers come back so the entity can carry the line.
fn self_platforms(behaviour: &mut BTreeMap<String, String>, content_prefix: &str) -> Vec<(f64, f64, f64, f64)> {
    let mut out: Vec<(f64, f64, f64, f64)> = Vec::new();
    for code in behaviour.values_mut() {
        rewrite_self_platform_calls(code, content_prefix, &mut out);
    }
    if !out.is_empty() {
        behaviour.insert("__selfPlatform".into(), self_platform_helpers());
    }
    out
}

/// The two helpers behind `__selfPlatform(...)`: making a platform structure that rides the
/// object, and removing one (or every one) it made.
pub(crate) fn self_platform_helpers() -> String {
    "\
var __selfPlatforms = [];\n\
function __selfPlatform(cid, ox, oy, w, h) {\n\
\tvar st = match.createStructure(self.getResource().getContent(cid));\n\
\tif (st != null) {\n\
\t\tvar dx = __sp(ox + (w / 2));\n\t\tvar dy = __sp(oy);\n\
\t\tst.setX(self.getX() + dx);\n\t\tst.setY(self.getY() + dy);\n\
\t\tif (st.exports != null && st.exports.__ride != null) { st.exports.__ride(self, dx, dy); }\n\
\t\t__selfPlatforms.push(st);\n\
\t}\n\treturn st;\n}\n\
// the source removes one it names, or with no name every one it made\n\
function __removeSelfPlatform(?st) {\n\
\tif (st != null) { if (st.kill != null) { st.kill(); } __selfPlatforms.remove(st); return; }\n\
\twhile (__selfPlatforms.length > 0) { var p = __selfPlatforms.pop(); if (p != null && p.kill != null) { p.kill(); } }\n\
}\n".to_string()
}

/// `createSelfPlatform(x, y, w, h, ...)` in `code` becomes `__selfPlatform("<prefix>platform<i>", x, y, w, h)`,
/// one content per call site, and a `removeSelfPlatform` the helper's remove.
pub(crate) fn rewrite_self_platform_calls(code: &mut String, content_prefix: &str, out: &mut Vec<(f64, f64, f64, f64)>) {
    // a fighter's frame scripts reach here already struck as having no counterpart; the port is one
    *code = code.replace("// [SSF2-only: createSelfPlatform] ", "");
    let re = crate::re!(r"(?:[A-Za-z_]\w*(?:\([^()]*\))?\.)*createSelfPlatform\(([^()]*)\)");
    // the helper is a bare function of the script, whatever receiver the source wrote
    let bare_remove = crate::re!(r"(?:[A-Za-z_]\w*(?:\([^()]*\))?\.)*__removeSelfPlatform\(");
    {
        let mut next = String::with_capacity(code.len());
        let mut rest = code.as_str();
        while let Some(m) = re.find(rest) {
            let caps = re.captures(&rest[m.start()..m.end()]).expect("captures");
            let args: Vec<&str> = caps[1].split(',').map(|a| a.trim()).collect();
            let nums: Vec<f64> = args.iter().take(4).filter_map(|a| a.parse::<f64>().ok()).collect();
            next.push_str(&rest[..m.start()]);
            if nums.len() == 4 {
                let i = out.len();
                out.push((nums[0], nums[1], nums[2], nums[3]));
                next.push_str(&format!("__selfPlatform(\"{content_prefix}platform{i}\", {}, {}, {}, {})", nums[0], nums[1], nums[2], nums[3]));
            } else {
                next.push_str(&rest[m.start()..m.end()]);
            }
            rest = &rest[m.end()..];
        }
        next.push_str(rest);
        *code = next.replace(".removeSelfPlatform(", ".__removeSelfPlatform(");
        *code = bare_remove.replace_all(code, "__removeSelfPlatform(").into_owned();
    }
}

/// The script a self-platform structure runs: it rides whatever told it to, and ends with it.
pub(crate) fn self_platform_script() -> String {
    "// a platform a projectile gave itself: it rides the projectile, and ends when it ends\n\
var __rider = null;\nvar __dx = 0;\nvar __dy = 0;\n\
function initialize() {\n\tself.exports.__ride = function(o, dx, dy) { __rider = o; __dx = dx; __dy = dy; };\n}\n\
function update() {\n\tif (__rider != null) {\n\t\tif (__rider.isDisposed()) { __rider = null; self.kill(); return; }\n\
\t\tself.setX(__rider.getX() + __dx);\n\t\tself.setY(__rider.getY() + __dy);\n\t}\n}\n\
function onTeardown() {}\nfunction onKill() {}\nfunction onStale() {}\nfunction afterPushState() {}\nfunction afterPopState() {}\nfunction afterFlushStates() {}\n".to_string()
}

/// The classes a piece of code says `new` about.
fn classes_built_in(code: &str) -> Vec<String> {
    crate::re!(r"\bnew\s+([A-Za-z_]\w*)\s*\(").captures_iter(code).map(|c| c[1].to_string()).collect()
}

pub fn generate_projectiles(
    ctx: &ProjectileGenCtx,
    projectiles: &[DiscoveredProjectile],
) -> Result<()> {
    let owner_id = ctx.owner_id;
    let abcs = parse_abcs(ctx.parsed_swf);
    // a field written through (`self.f.x = v`), for the records of an object's own
    let through_re = regex::Regex::new(r"self\.([A-Za-z_]\w*)\.[A-Za-z_]\w*\s*=[^=]").expect("through-write regex");
    // an item's frame interrupt, installed by its class (`setFrameInterrupt(handleInterrupt)`)
    let frame_interrupt_re = regex::Regex::new(r"self\.setFrameInterrupt\(\s*(?:self\.)?(\w+)\s*\)").expect("frame interrupt regex");
    for proj in projectiles {
        // The projectile's OWN code, read before the entity is built so its timeline scripts can
        // go on the timeline they belong to. The declaration names the class it runs; the api's
        // own binding (`SSF2Projectile` itself) has nothing of the package's in it.
        let decl = haxe_gen::best_match_projectile_data(&proj.name, ctx.projectile_data);
        // a package holding several characters (bowser and giga bowser) finds the same clips for
        // each, and only the one whose code declares a projectile knows what it is: a later
        // character that declares nothing of it leaves the earlier one's files standing
        if !declared_here(&ctx.scripts_root.join("Projectile").join(haxe_gen::snake_to_pascal(&proj.name)), decl.is_some()) {
            log::info!("projectile {} is declared by another character of this package; its files stand", proj.name);
            continue;
        }
        let class_api: Option<String> = decl
            .and_then(|d| d.names.get("classAPI").cloned())
            .filter(|c| !abcs.iter().any(|abc| crate::abc_parser::is_api_binding_class(abc, c)));
        let mut behaviour = clip_behaviour(&abcs, proj, class_api.as_deref());
        // WHAT THE PROJECTILE BUILDS FOR ITSELF. A class its own code says `new` about is part
        // of its behaviour (rayman's plum keeps its life on two FrameTimers), and left behind it
        // is a name nothing declares: the timer never existed and every tick of it threw. Ported
        // as a factory the way the character's own are (abc_parser::reconstruct_class_as_factory).
        {
            let built: std::collections::BTreeSet<String> = behaviour.values()
                .flat_map(|code| classes_built_in(code))
                .collect();
            for name in &built {
                let Some(abc) = abcs.iter().find(|abc| crate::abc_parser::class_named(abc, name).is_some()) else { continue };
                let Some(factory) = crate::abc_parser::reconstruct_class_as_factory(abc, name) else { continue };
                log::info!("class '{name}' the projectile '{}' builds for itself, ported as a factory", proj.name);
                behaviour.insert(format!("{name}__new"), factory);
                for code in behaviour.values_mut() {
                    *code = code.replace(&format!("new {name}("), &format!("{name}__new("));
                }
            }
        }
        // the values the behaviour class states for its own constants, read by name from its clips
        let class_consts: Vec<(String, String)> = class_api.as_deref()
            .map(|c| abcs.iter().flat_map(|abc| crate::abc_parser::class_const_values(abc, c)).collect())
            .unwrap_or_default();
        // the labels its states answer to, as the script's own state names
        let lstates: BTreeMap<String, String> = haxe_gen::projectile_lstates(
            &proj.states.iter().map(|s| s.label.clone()).collect::<Vec<_>>())
            .into_iter().map(|(label, lname, _)| (label, lname)).collect();
        let translate_frame = |code: &str| -> String {
            let code = crate::api_mappings::rewrite_owner_label_jumps(code);
            // a stat asked of the attack someone is in (black mage's meteor reads its caster's
            // charge limit on the frame it starts to burn): struck, the limit was nothing, the
            // burn's damage came out infinite and the engine hung on the hit
            let code = crate::api_mappings::rewrite_attack_stat_reads(&code);
            let body = crate::api_mappings::as_projectile(|| crate::api_mappings::translate_ssf2_to_fm(&code));
            let body = crate::api_mappings::rewrite_projectile_force_attack(&body, &lstates);
            crate::api_mappings::rename_animation_literals(&body, ctx.owner_anims)
        };

        // Extract image+box data for each extra state (multi-state projectiles like link_bomb).
        // THE NAMED CHILDREN ITS CODE ADDRESSES. A clip places children under names, and its
        // code reaches them as fields (`self.gordospin.rotation`, `self.food.playFrameLabel(...)`,
        // `self.notes.n1.playFrame(...)`). Each such child is a part of its own here: its frames
        // an animation on this entity, drawn by a vfx riding the projectile, and a record the
        // code talks to. Left baked into the parent's art, a spin or a chosen frame was struck.
        // The children of its state clips count the same way, under the one namespace the
        // object has (waluigi's plant moves its `posPoint` in every throw it has).
        if std::env::var("PEPTIDE_DUMP_PARTS").is_ok() {
            eprintln!("[parts] projectile {} root={} inner={:?} states={:?}", proj.name, proj.sprite_id, proj.inner_sprite_id,
                proj.states.iter().map(|st| (st.label.clone(), st.inner_sprite_id)).collect::<Vec<_>>());
            for tag in &ctx.parsed_swf.tags {
                if let swf::Tag::DefineSprite(sp) = tag {
                    let names: Vec<String> = sp.tags.iter().filter_map(|t| match t {
                        swf::Tag::PlaceObject(po) => po.name.as_ref().map(|n| String::from_utf8_lossy(n.as_bytes()).to_string()),
                        _ => None }).filter(|n| !n.is_empty()).collect();
                    if !names.is_empty() { eprintln!("[parts] sprite {} names {:?}", sp.id, names); }
                }
            }
        }
        let (parts, skip_names) = {
            let mut code_text: String = behaviour.values().cloned().collect();
            // a name the object's own code already owns (a method, a field it assigns) is not a child
            let taken: Vec<String> = behaviour.keys().cloned().chain(haxe_gen::own_fields_in(&code_text)).collect();
            for state in &proj.states {
                if state.label == "attack_idle" { continue }
                code_text.push_str(&state_behaviour(&abcs, state).values().cloned().collect::<String>());
            }
            let mut out: Vec<entity_gen::ProjectilePartData> = Vec::new();
            let mut skip: Vec<String> = Vec::new();
            // its own clip, the clip it flies in, and each of its states' (a child placed on the
            // outer clip is reached by the code the same way as one on the inner)
            let mut roots: Vec<u16> = vec![proj.sprite_id];
            roots.extend(proj.inner_sprite_id);
            roots.extend(proj.states.iter().filter(|st| st.label != "attack_idle").map(|st| st.inner_sprite_id));
            for root in roots {
                let mut found: Vec<entity_gen::ProjectilePartData> = Vec::new();
                for n in collect_parts(ctx, root, "", &crate::image_extractor::ImageLocalMatrix::default(), &code_text, owner_id, &taken, &mut found) {
                    if !skip.contains(&n) { skip.push(n); }
                }
                for f in found {
                    if !out.iter().any(|o| o.path == f.path) { out.push(f); }
                }
            }
            (out, skip)
        };
        for code in behaviour.values_mut() { rewrite_part_refs(&parts, code); }
        // an item's frame interrupt (the holder's attacks it turns into throws) and its holder
        // are what it publishes: rewritten here, before any pass strikes them as calls this
        // engine's handle cannot answer
        {
            for code in behaviour.values_mut() {
                *code = frame_interrupt_re.replace_all(code, "self.exports.__frameInterrupt = $1").into_owned();
                *code = code.replace("self.getHolder()", "self.exports.__holder");
            }
        }
        // A FIELD ONLY EVER WRITTEN THROUGH. `self.posPoint.x = 41.5` on a name nothing declares,
        // no clip places and no method answers to (waluigi's plant keeps a point of where its
        // next spawn goes) is a record of the object's own: made empty here, and written as the
        // source writes it.
        let through_written: Vec<String> = {
            let text: String = behaviour.values().cloned().collect();
            let own = haxe_gen::own_fields_in(&text);
            let mut names: Vec<String> = Vec::new();
            // a method of the object's own with a body is not a record; a bodyless stub the
            // decompiler leaves for a declared slot is the slot, which is what this declares
            let says_something = |code: &str| code.lines().any(|l| { let t = l.trim(); !t.is_empty() && t != "return;" && !t.starts_with("//") && t != "}" && !t.starts_with("function ") });
            for c in through_re.captures_iter(&text) {
                let n = c[1].to_string();
                if std::env::var("PEPTIDE_DUMP_PARTS").is_ok() {
                    eprintln!("[parts] through-written '{n}': own={} method={:?}", own.contains(&n), behaviour.get(&n).map(|c| says_something(c)));
                }
                // what a script publishes is the engine's slot, not a field of its own
                if n == "exports" || own.contains(&n) || names.contains(&n) || behaviour.get(&n).is_some_and(|c| says_something(c))
                    || parts.iter().any(|p| p.path == n)
                    || crate::api_mappings::RESERVED_EXT_VARS.contains(&n.as_str()) { continue }
                // a name the code also reads as a method or hands elsewhere is not a plain record
                if text.contains(&format!("self.{n}(")) { continue }
                names.push(n);
            }
            names
        };
        for n in &through_written {
            let re = crate::dyn_re(&format!(r"self\.{}\b", regex::escape(n)));
            for code in behaviour.values_mut() { *code = re.replace_all(code, n.as_str()).into_owned(); }
        }
        let mut parts_hscript = parts_hscript(&parts, &projectile_content_id(&proj.name), &haxe_gen::own_fields_in(&behaviour.values().cloned().collect::<String>()), &[]);
        for n in &through_written { parts_hscript.0.push_str(&format!("// a record of its own, written through as the source writes it\nvar {n} = {{}};\n")); }
        let mut extra_states: Vec<entity_gen::ProjectileStateData> = Vec::new();
        for state in &proj.states {
            if state.label == "attack_idle" {
                continue; // already extracted above
            }
            // the state's own clip: its frames on its own timeline, its handlers on the script.
            // a handler named like one another clip already declared keeps its own body under
            // the state's name, and the state's own code is rewired to it
            let mut sb = state_behaviour(&abcs, state);
            for code in sb.values_mut() { rewrite_part_refs(&parts, code); }
            let suffix: String = state.label.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
            let handlers: Vec<String> = sb.keys().filter(|n| !crate::abc_parser::is_root_xframe_method(n)).cloned().collect();
            for name in &handlers {
                if !behaviour.contains_key(name) { continue }
                let renamed = format!("{name}__{suffix}");
                let mut code = sb.remove(name).unwrap_or_default();
                code = code.replacen(&format!("function {name}("), &format!("function {renamed}("), 1);
                for c in sb.values_mut() { *c = c.replace(&format!("self.{name}"), &format!("self.{renamed}")); }
                code = code.replace(&format!("self.{name}"), &format!("self.{renamed}"));
                sb.insert(renamed, code);
            }
            let state_frames = frame_scripts_of(&sb, 1, &translate_frame);
            for (name, code) in sb {
                if !crate::abc_parser::is_root_xframe_method(&name) { behaviour.insert(name, code); }
            }
            let (sf, sm, sg, sp) = match crate::image_extractor::extract_projectile_frame_images_from_swf_skipping(
                ctx.parsed_swf, owner_id, state.inner_sprite_id, ctx.img_result, &skip_names,
            ) {
                Ok(pfi) => { let pfi = placed_by(pfi, &state.stance); (pfi.frames, pfi.matrices, pfi.image_guids, pfi.parts) }
                Err(e) => {
                    log::warn!("State '{}' image extraction failed: {}", state.label, e);
                    (vec![], vec![], std::collections::BTreeMap::new(), vec![])
                }
            };
            let state_boxes = crate::sprite_parser::extract_boxes_for_sprite_id_placed(
                ctx.parsed_swf,
                state.inner_sprite_id,
                as_xform(&state.stance),
            )
            .unwrap_or_default();
            extra_states.push(entity_gen::ProjectileStateData {
                label: state.label.clone(),
                sprite_id: state.inner_sprite_id,
                image_frames: sf,
                image_matrices: sm,
                image_parts: sp,
                image_guids: sg,
                boxes: state_boxes,
                frame_count: state.inner_frame_count,
                frame_scripts: state_frames,
            });
        }
        // Extract image frames from the inner sprite using effect-sprite flattening.
        let (image_frames, image_matrices, image_guids, image_parts) = if let Some(inner_id) = proj.inner_sprite_id {
            match crate::image_extractor::extract_projectile_frame_images_from_swf_skipping(
                ctx.parsed_swf, owner_id, inner_id, ctx.img_result, &skip_names,
            ) {
                Ok(pfi) => {
                    let pfi = placed_by(pfi, &proj.inner_stance);
                    log::debug!("Projectile '{}': {} image frames", proj.name, pfi.frames.len());
                    if std::env::var("PEPTIDE_DUMP_EFFECTS").is_ok() {
                        for (i, ps) in pfi.parts.iter().enumerate() {
                            eprintln!("[proj] {} frame {} parts={:?}", proj.name, i, ps.iter().map(|(n, m)| (n.clone(), (m.tx, m.ty, m.sx, m.sy))).collect::<Vec<_>>());
                        }
                    }
                    (pfi.frames, pfi.matrices, pfi.image_guids, pfi.parts)
                }
                Err(e) => {
                    log::warn!("Failed to extract images for projectile '{}': {}", proj.name, e);
                    (vec![], vec![], std::collections::BTreeMap::new(), vec![])
                }
            }
        } else {
            (vec![], vec![], std::collections::BTreeMap::new(), vec![])
        };

        // Extract collision boxes from the inner sprite.
        let boxes = if let Some(inner_id) = proj.inner_sprite_id {
            match crate::sprite_parser::extract_boxes_for_sprite_id_placed(ctx.parsed_swf, inner_id, as_xform(&proj.inner_stance)) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("Failed to extract boxes for projectile '{}': {}", proj.name, e);
                    None
                }
            }
        } else {
            None
        };

        // a platform the clip gives itself: a structure content beside it, riding it
        let platforms = self_platforms(&mut behaviour, &projectile_content_id(&proj.name));
        // A TIMELINE THE SOURCE HOLDS holds here: this object can be told to hold the frame it is
        // on and keeps moving while it does, which is what the source's hold is. (A fighter is
        // not: holding one takes it out of what the camera reads and the engine dies on the next
        // frame it draws, so a fighter's hold stays struck.)
        // ...and only where there is a timeline to hold: a clip with one frame of its own shows
        // the same picture whatever it is told, and what moves inside it is the clip placed
        // within, whose own timeline the source leaves running (measured: the source's fireball
        // holds its outer clip on frame 1 while the clip inside it runs its 20).
        for code in behaviour.values_mut() {
            *code = if proj.own_frame_count > 1 {
                crate::api_mappings::a_held_timeline_holds_here(code)
            } else {
                crate::api_mappings::a_one_frame_clip_has_nothing_to_hold(code)
            };
        }
        // what it is, asked of itself in order to fire another one: megaman's wave fires the next
        // wave from its own stats. the only thing the port can hand on is which projectile it is,
        // and the fire looks that up the way it looks up any named one
        for code in behaviour.values_mut() {
            *code = own_kind_asked(code, &proj.name);
        }
        let behaviour = behaviour;

        // frame1 is the clip's first frame's script, and runs where the source runs it: as the
        // first frame plays. folded into initialize it ran for a projectile its maker sends straight
        // to a label at the spawn, which never plays its first frame in the source (megaman's water
        // wave reset its count with every segment and never stopped), and twice for one that does
        // (this engine runs the first frame's script the moment the object is made, before its maker
        // has even asked for the label; so frame1 is a function the first update runs unless a
        // label was asked at the spawn, see `__frame1` in the projectile template)
        let frame_scripts: std::collections::BTreeMap<u16, String> = frame_scripts_of(&behaviour, 2, &translate_frame);

        let proj_info = entity_gen::ProjectileInfo {
            name: proj.name.clone(),
            sprite_id: proj.sprite_id,
            inner_sprite_id: proj.inner_sprite_id,
            inner_sprite_name: proj.inner_sprite_name.clone(),
            inner_frame_count: proj.inner_frame_count,
            boxes,
            image_frames,
            image_matrices,
            image_parts,
            image_guids,
            extra_states,
            parts,
            inner_labels: proj.inner_labels.clone(),
            frame_scripts,
            platforms: platforms.iter().map(|p| (p.2, p.3)).collect(),
        };

        // named by the id it is addressed by, not by the name the source spells it with: the
        // same clip can be emitted twice, once as the thing this fires and once as art that
        // stands on its own, and one name for both means whichever is written second is the
        // only one that survives -- the move then fires art with no behaviour behind it
        let filename = format!("{}.entity",
            haxe_gen::sanitize_entity_name(&projectile_content_id(&proj.name)));
        // A field the clip sets on itself from one of its own frames is the clip's variable,
        // not something the engine has: written through the api object it throws and the frame
        // stops there. Addressed without it here, and declared in the script below.
        let mut proj_info = proj_info;
        // The clip's own fields are declared wherever it first writes them, which is usually the
        // frame that sets it up rather than the frame that uses it -- so the two are read
        // TOGETHER, the same way the script that declares them reads them. Looking only at the
        // frames left `self.character` standing in a frame that was handed one at spawn.
        let frame_field_text: String = proj_info.frame_codes().into_iter().cloned()
            .chain(behaviour.values().cloned())
            .collect();
        for code in proj_info.frame_codes_mut() {
            *code = own_kind_asked(code, &proj.name);
        }
        let mut own_fields_here = haxe_gen::own_fields_in(&frame_field_text);
        // the parts and the records of its own are its fields as much as what it assigns
        own_fields_here.extend(proj_info.parts.iter().filter(|p| !p.path.contains('.')).map(|p| p.path.clone()));
        own_fields_here.extend(through_written.iter().cloned());
        for f in &own_fields_here {
            for code in proj_info.frame_codes_mut() {
                *code = code.replace(&format!("self.{f}"), f);
            }
        }
        // and the behaviour class's own constants, which its clips read off the api object
        for (n, _) in &class_consts {
            for code in proj_info.frame_codes_mut() {
                *code = code.replace(&format!("self.{n}"), n);
            }
        }
        // and a handler the clip hands to a listener is one of its own methods, which come over
        // as plain functions -- so the reference loses the api object here too.
        let handlers: Vec<String> = behaviour.keys()
            .filter(|n| !crate::abc_parser::is_root_xframe_method(n)).cloned().collect();
        for name in handlers {
            for code in proj_info.frame_codes_mut() {
                *code = code.replace(&format!("self.{name}"), &name);
            }
        }
        // and the summoner's own state is reached through what the summoner published --
        // in the clip's own code as much as on its frames, since it reads that state from both.
        for v in ctx.owner_vars {
            for pat in [format!(".{v}.get("), format!(".{v}.set(")] {
                let to = pat.replacen('.', ".exports.", 1);
                for code in proj_info.frame_codes_mut() {
                    *code = code.replace(&pat, &to);
                }
            }
            for code in proj_info.frame_codes_mut() {
                *code = crate::api_mappings::write_through_published_state(code, v);
            }
        }
        for f in ctx.owner_fns {
            // a name this engine answers to as well is the engine's here, not the summoner's
            if crate::api_mappings::the_engine_answers_to(f) { continue }
            for code in proj_info.frame_codes_mut() {
                *code = code.replace(&format!(".{f}("), &format!(".exports.{f}("));
            }
        }
        for g in ctx.owner_getters {
            if crate::api_mappings::the_engine_answers_to(g) { continue }
            for code in proj_info.frame_codes_mut() {
                *code = crate::api_mappings::read_owner_getter_through_exports(code, g);
            }
        }
        {
            let owner_list: Vec<String> = ctx.owner_vars.iter().cloned().collect();
            for code in proj_info.frame_codes_mut() {
                *code = crate::api_mappings::read_owner_state_through_exports(code, &owner_list);
            }
        }
        // a frame of a projectile is part of the projectile's script the same way a character's
        // is: it calls that script's functions by name, and a call the engine cannot answer
        // takes the rest of the frame with it.
        let own_here: Vec<String> = behaviour.keys().cloned().collect();
        // the animations the entity actually gets: every label its clip carries (the same list
        // its animation stats are written from), so a floor landing that asks for its
        // `continue` finds it. read as the trio alone, pikachu's bolt lost its landing and fell
        // through the floor for good
        let own_anims: Vec<String> = {
            let mut v = haxe_gen::projectile_anim_names(&proj_info).all;
            v.sort_unstable();
            v.dedup();
            v
        };
        for code in proj_info.frame_codes_mut() {
            *code = code.replace("self.inState(IState.ACTIVE)", "(self.inState(PState.ACTIVE) && self.exports.__holder == null)");
            *code = crate::api_mappings::strike_unplayable_own_animations(code, &own_anims);
            *code = crate::api_mappings::strike_unplayable_owner_animations(code);
            *code = haxe_gen::strike_calls_the_engine_cannot_answer(code, &own_here);
        }

        // The script and the frames that run in its scope, settled together before either is
        // written: a clip registers a poll on one frame and cancels it on another, so neither
        // half is visible to a pass that sees only one of them.
        let entity_id = proj.name.replace('_', "");
        let proj_script = {
            let anims = haxe_gen::projectile_anim_names(&proj_info);
            let mut script = haxe_gen::generate_projectile_script(
                owner_id, &entity_id, &proj_info.extra_states,
                haxe_gen::best_match_projectile_data(&proj.name, ctx.projectile_data),
                &behaviour, anims.destroy != anims.active, &frame_field_text, &anims.active,
                ctx.kept_by_summoner.contains(&proj.name), &class_consts, ctx.owner_anims,
                &haxe_gen::projectile_label_flow(&proj_info), (&parts_hscript.0, &parts_hscript.1));
            // after translation, since only then does reading the summoner's state look like
            // reading it: the wrappers are put on along the way.
            for v in ctx.owner_vars {
                for pat in [format!(".{v}.get("), format!(".{v}.set(")] {
                    script = script.replace(&pat, &pat.replacen('.', ".exports.", 1));
                }
                script = crate::api_mappings::write_through_published_state(&script, v);
            }
            // and asking the summoner one of its own questions goes through what it published
            for f in ctx.owner_fns {
                // a name this engine answers to as well is the engine's here, not the summoner's
                if crate::api_mappings::the_engine_answers_to(f) { continue }
                script = script.replace(&format!(".{f}("), &format!(".exports.{f}("));
            }
            // and a getter of the summoner's, read as a plain field the way the source reads
            // one: a call through what it published
            for g in ctx.owner_getters {
                if crate::api_mappings::the_engine_answers_to(g) { continue }
                script = crate::api_mappings::read_owner_getter_through_exports(&script, g);
            }
            // and reading its state by naming it, which is how the source reaches it
            let owner_list: Vec<String> = ctx.owner_vars.iter().cloned().collect();
            script = crate::api_mappings::read_owner_state_through_exports(&script, &owner_list);
            // A SUMMONER'S SLOT READ BY ITS BARE NAME. The source is standing on the object that
            // has it, so it just says the name -- and the projectile never writes it, so nothing
            // declares it here either and the read is of nothing at all. It belongs to whoever
            // summoned this, and is read the way everything else of theirs is.
            let mine = haxe_gen::own_fields_in(&script);
            let script = owner_state_by_bare_name(&script, ctx.owner_vars, &mine);
            // held is a state of its own in the source: a held item is not active (the check is
            // spelled with the state's name only once the script is translated)
            let script = script.replace("self.inState(IState.ACTIVE)", "(self.inState(PState.ACTIVE) && self.exports.__holder == null)");
            // an animation it asks its summoner for is the summoner's to have
            let script = crate::api_mappings::strike_unplayable_own_animations(&script, &own_anims);
            let script = crate::api_mappings::strike_unplayable_owner_animations(&script);
            let script = haxe_gen::strike_calls_the_engine_cannot_answer(&script, &[]);
            // and what it asks of the things it holds, by names nobody has
            haxe_gen::strike_calls_on_held_things(&script)
        };
        let mut proj_script = proj_script;
        haxe_gen::pair_timers_across_frames(&mut proj_script, &mut proj_info.frame_scripts);
        for st in proj_info.extra_states.iter_mut() {
            haxe_gen::pair_timers_across_frames(&mut proj_script, &mut st.frame_scripts);
        }
        // A helper a frame asks for, checked on the finished text: the frames are translated
        // after the script decides what it needs, so asking earlier misses exactly the frames
        // that do the asking. The frames run in the script's scope, so the script carries it.
        let frames_text: String = proj_info.frame_codes().into_iter().cloned().collect::<Vec<_>>().join("\n");
        for (needle, helper) in [
            // a clip that counts its own frames in the source's units (a return tween over six)
            ("__srcFrame(", crate::api_mappings::SRC_FRAME),
            ("__srcTotalFrames(", crate::api_mappings::SRC_FRAME),
            ("_currentProjectile(", crate::api_mappings::CURRENT_PROJECTILE),
            ("_pick(", crate::api_mappings::TABLE_PICK),
            ("_getGlobal(", crate::api_mappings::GLOBAL_STORE),
            ("_setGlobal(", crate::api_mappings::GLOBAL_STORE),
        ] {
            let wanted = proj_script.contains(needle) || frames_text.contains(needle);
            let defined = proj_script.contains(&format!("function {}", needle.trim_end_matches('(')));
            if wanted && !defined {
                proj_script.insert_str(0, helper);
                proj_script.insert(0, '\n');
            }
        }
        // ...and the frames the same way: they run in the script's scope, and a frame is where
        // lucario's aura sphere writes its `trailing` flag, which its class reads off the summoner
        {
            let mine = haxe_gen::own_fields_in(&proj_script);
            // a call struck for its name that the projectile answers itself: its own functions
            // win over what a table says of a call by that name (a missile's `jumpToContinue`)
            proj_script = crate::api_mappings::uncomment_local_fn_calls(&proj_script);
            let own = crate::api_mappings::functions_defined_in(&proj_script);
            for code in proj_info.frame_codes_mut() {
                *code = crate::api_mappings::uncomment_calls_to_with(code, &own, &own);
            }
            proj_script = crate::api_mappings::handed_api_methods(&proj_script, &own);
            proj_script = crate::api_mappings::timer_removals_through_records(&proj_script);
            proj_script = crate::api_mappings::hitbox_writes_carried(&proj_script);
            proj_script = crate::api_mappings::hitbox_values_converted(&proj_script);
            proj_script = crate::api_mappings::published_writes_through_setters(&proj_script);
            {
                let mut all = proj_script.clone();
                for code in proj_info.frame_codes() { all.push('\n'); all.push_str(code); }
                proj_script = crate::api_mappings::timers_cancelled_by_function_in(&proj_script, &all);
                for code in proj_info.frame_codes_mut() {
                    *code = crate::api_mappings::timers_cancelled_by_function_in(code, &all);
                }
            }
            // a jump to the label the clip's first animation is named for lands on that label's
            // frame INSIDE it, not on the animation's first frame: the first animation carries the
            // clip's lead-in frames before its label (megaman's water wave replays its `loop`,
            // whose label sits at frame 3, and restarted from frame 1 ran its first frame's script
            // every cycle, resetting the wave's count)
            {
                let names = haxe_gen::projectile_anim_names(&proj_info);
                if let Some(first) = proj_info.inner_labels.iter().min_by_key(|(f, _)| *f) {
                    let ident = haxe_gen::label_ident(&first.1);
                    if first.0 > 1 && names.all.first().is_some_and(|n| *n == ident) {
                        let ef = (first.0 as u32 - 1) * 2 + 1;
                        let land = format!("{{ if (self.getAnimation() != \"{ident}\") {{ self.playAnimation(\"{ident}\"); }} self.playFrame({ef}); }}");
                        let plain = format!("self.playAnimation(\"{ident}\");");
                        let own = format!("__playOwn(\"{ident}\");");
                        proj_script = proj_script.replace(&plain, &land).replace(&own, &land);
                        for code in proj_info.frame_codes_mut() { *code = code.replace(&plain, &land).replace(&own, &land); }
                    }
                }
            }
            proj_script = crate::api_mappings::own_end_tells_first(&proj_script);
            proj_script = bounds_writes_to_the_script(&proj_script);
            proj_script = crate::api_mappings::listeners_on_others_persist_until_end(&proj_script);
            proj_script = haxe_gen::publish_projectile_methods(&proj_script);
            let own_list: Vec<String> = own.iter().cloned().collect();
            for code in proj_info.frame_codes_mut() {
                *code = haxe_gen::ask_other_objects_through_exports(code, &own_list);
                *code = crate::api_mappings::hitbox_values_converted(code);
                *code = crate::api_mappings::published_writes_through_setters(code);
                *code = crate::api_mappings::own_end_tells_first(code);
                *code = bounds_writes_to_the_script(code);
                *code = crate::api_mappings::listeners_on_others_persist_until_end(code);
                *code = owner_state_by_bare_name(code, ctx.owner_vars, &mine);
                *code = crate::api_mappings::handed_api_methods(code, &own);
                *code = crate::api_mappings::timer_removals_through_records(code);
                *code = crate::api_mappings::hitbox_writes_carried(code);
            }
            // the helper that converts them, put on after the passes above (which would read its
            // fields as the summoner's)
            let frames_use = proj_info.frame_codes().iter().any(|c| c.contains("__hbFields("));
            if (frames_use || proj_script.contains("__hbFields(")) && !proj_script.contains("function __hbFields(") {
                proj_script.insert_str(0, &crate::api_mappings::hitbox_fields_helper());
            }
            let reads_use = proj_info.frame_codes().iter().any(|c| c.contains("__hbRead("));
            if (reads_use || proj_script.contains("__hbRead(")) && !proj_script.contains("function __hbRead(") {
                proj_script.insert_str(0, &crate::api_mappings::hitbox_reads_helper());
            }
        }
        let mut proj_json = entity_gen::generate_projectile_entity(owner_id, &proj_info, ctx.runtime_runs_frames);
        // Fill in paletteMap if available.
        if let (Some(cg), Some(pm)) = (ctx.palette_collection_guid, ctx.palette_base_map_id) {
            let mut proj_val: serde_json::Value =
                serde_json::from_str(&proj_json).unwrap_or(serde_json::json!({}));
            proj_val["paletteMap"] = serde_json::json!({
                "paletteCollection": cg,
                "paletteMap": pm
            });
            proj_json = serde_json::to_string_pretty(&proj_val).unwrap_or(proj_json);
        }
        haxe_gen::note_entity_written(&ctx.entities_dir.join(&filename));
        fs::write(ctx.entities_dir.join(&filename), proj_json)?;
        log::info!("Generated projectile entity: {} ({} frames)", filename, proj.inner_frame_count);

        // ── projectile script files ──────────────────────────────────────────
        let pascal = haxe_gen::snake_to_pascal(&proj.name);
        let proj_scripts_dir = ctx.scripts_root.join("Projectile");
        fs::create_dir_all(&proj_scripts_dir)?;

        let proj_ssf2_match = haxe_gen::best_match_projectile_data(&proj.name, ctx.projectile_data);
        let (stats_text, survives) = bounds_are_the_scripts(&haxe_gen::generate_projectile_stats(owner_id, &entity_id, &proj_info, proj_ssf2_match));
        let proj_script = proj_script.replace("/*SURVIVES_BOUNDS*/false", if survives { "true" } else { "false" });
        fs::write(proj_scripts_dir.join(format!("{}Script.hx", pascal)), proj_script)?;
        fs::write(
            proj_scripts_dir.join(format!("{}Script.hx.meta", pascal)),
            haxe_gen::script_meta(
                &format!("{}ProjectileScript", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileScript::meta", owner_id, proj.name)),
                haxe_gen::ScriptMetaKind::ProjectileScript,
            ),
        )?;
        // the platforms it gives itself: one structure content each, on a shared follower script
        if !platforms.is_empty() {
            let script_id = format!("{entity_id}PlatformScript");
            fs::write(proj_scripts_dir.join(format!("{pascal}PlatformScript.hx")), self_platform_script())?;
            fs::write(proj_scripts_dir.join(format!("{pascal}PlatformScript.hx.meta")),
                haxe_gen::script_meta(&script_id, &det_uuid(&format!("{}::{}::PlatformScript::meta", owner_id, proj.name)), haxe_gen::ScriptMetaKind::StructureScript))?;
            let content = projectile_content_id(&proj.name);
            for (i, _) in platforms.iter().enumerate() {
                let stats_id = format!("{entity_id}Platform{i}Stats");
                fs::write(proj_scripts_dir.join(format!("{pascal}Platform{i}Stats.hx")), format!(
                    "// a platform {} gives itself: the line it carries, riding it\n{{\n\tspriteContent: self.getResource().getContent(\"{content}\"),\n\tanimationId: \"platformSprite{i}\",\n\tstartX: 0,\n\tstartY: 0\n}}\n", proj.name))?;
                fs::write(proj_scripts_dir.join(format!("{pascal}Platform{i}Stats.hx.meta")),
                    haxe_gen::script_meta(&stats_id, &det_uuid(&format!("{}::{}::Platform{i}Stats::meta", owner_id, proj.name)), haxe_gen::ScriptMetaKind::CharacterAnimationStats))?;
                note_structure_content(format!("{content}platform{i}"), stats_id, script_id.clone());
            }
        }
        fs::write(
            proj_scripts_dir.join(format!("{}AnimationStats.hx", pascal)),
            haxe_gen::generate_projectile_animation_stats(&proj_info),
        )?;
        fs::write(
            proj_scripts_dir.join(format!("{}AnimationStats.hx.meta", pascal)),
            haxe_gen::script_meta(
                &format!("{}ProjectileAnimationStats", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileAnimationStats::meta", owner_id, proj.name)),
                haxe_gen::ScriptMetaKind::ProjectileAnimationStats,
            ),
        )?;
        fs::write(proj_scripts_dir.join(format!("{}Stats.hx", pascal)), &stats_text)?;
        fs::write(
            proj_scripts_dir.join(format!("{}Stats.hx.meta", pascal)),
            haxe_gen::script_meta(
                &format!("{}ProjectileStats", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileStats::meta", owner_id, proj.name)),
                haxe_gen::ScriptMetaKind::ProjectileStats,
            ),
        )?;
        fs::write(
            proj_scripts_dir.join(format!("{}HitboxStats.hx", pascal)),
            haxe_gen::generate_projectile_hitbox_stats(owner_id, &entity_id, &proj_info, proj_ssf2_match),
        )?;
        fs::write(
            proj_scripts_dir.join(format!("{}HitboxStats.hx.meta", pascal)),
            haxe_gen::script_meta(
                &format!("{}ProjectileHitboxStats", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileHitboxStats::meta", owner_id, proj.name)),
                haxe_gen::ScriptMetaKind::ProjectileHitboxStats,
            ),
        )?;
        log::info!("Generated projectile scripts for {} → {}*.hx", proj.name, pascal);
    }
    Ok(())
}


/// The named children of `sprite_id` that the code addresses under `prefix` (`self.a`,
/// `self.a.b`), as parts: each addressed child is extracted on its own (less the children of
/// its own the code addresses), and the names left out of the parent's art are answered.
fn collect_parts(
    ctx: &ProjectileGenCtx,
    sprite_id: u16,
    prefix: &str,
    at: &crate::image_extractor::ImageLocalMatrix,
    code_text: &str,
    owner_id: &str,
    taken: &[String],
    out: &mut Vec<entity_gen::ProjectilePartData>,
) -> Vec<String> {
    let mut skip: Vec<String> = Vec::new();
    for child in crate::image_extractor::named_children_of_sprite(ctx.parsed_swf, sprite_id) {
        if prefix.is_empty() && taken.contains(&child.name) { continue }
        let path = if prefix.is_empty() { child.name.clone() } else { format!("{prefix}.{}", child.name) };
        let re = crate::dyn_re(&format!(r"self\.{}\b", regex::escape(&path)));
        if std::env::var("PEPTIDE_DUMP_PARTS").is_ok() {
            eprintln!("[parts] sprite {sprite_id}: child '{path}' (sprite {}) addressed={}", child.sprite_id, re.is_match(code_text));
        }
        if !re.is_match(code_text) { continue }
        skip.push(child.name.clone());
        let here = at.compose(&child.matrix);
        let inner_skip = collect_parts(ctx, child.sprite_id, &path, &here, code_text, owner_id, taken, out);
        let pfi = match crate::image_extractor::extract_projectile_frame_images_from_swf_skipping(
            ctx.parsed_swf, owner_id, child.sprite_id, ctx.img_result, &inner_skip,
        ) {
            Ok(p) => p,
            Err(e) => { log::warn!("part '{path}': image extraction failed: {e}"); continue }
        };
        let labels: Vec<(String, u16)> = crate::image_extractor::sprite_tags(ctx.parsed_swf, child.sprite_id)
            .map(crate::sprite_parser::extract_frame_labels_from_tags).unwrap_or_default();
        out.push(entity_gen::ProjectilePartData {
            anim: format!("part_{}", path.replace('.', "_")),
            path,
            matrix: here,
            frame_count: pfi.frames.len() as u16,
            image_frames: pfi.frames,
            image_matrices: pfi.matrices,
            image_parts: pfi.parts,
            image_guids: pfi.image_guids,
            labels,
        });
    }
    skip
}

/// What the script declares and does for its parts: the records and the vfx behind them
/// (`decl`), and the making of each as the object initializes (`init`).
pub(crate) fn parts_hscript(parts: &[entity_gen::ProjectilePartData], content_id: &str, own_fields: &[String], placements: &[entity_gen::PartPlacement]) -> (String, String) {
    if parts.is_empty() {
        return ("function __partsUpdate() {}\nfunction __partsDispose() {}\n".to_string(), String::new());
    }
    let mut decl = String::new();
    decl.push_str("// the clip's named children its code addresses: each a part of its own riding this object,\n\
// a vfx drawn from the child's own frames and a record the code talks to (its rotation, its\n\
// place, its frame), which the vfx follows every frame. offsets are in the source's units\n");
    for p in parts {
        if p.path.contains('.') { continue }
        if own_fields.contains(&p.path) { continue }
        decl.push_str(&format!("var {} = null;\n", p.path));
    }
    decl.push_str(&format!("var __parts = [];\n\
// where each of a fighter's animations places a part (empty for a projectile, whose one clip\n\
// places its parts once): the part, the animation, the art it shows there, the placement\n\
var _ppName = [];\n\
var _ppAnim = [];\n\
var _ppArt = [];\n\
var _ppOx = [];\n\
var _ppOy = [];\n\
var _ppRot = [];\n\
var _ppSx = [];\n\
var _ppSy = [];\n\
function __partPlace(p) {{\n\
\tvar a = self.getAnimation();\n\
\tvar __i = 0;\n\
\tvar any = false;\n\
\twhile (__i < _ppName.length) {{\n\
\t\tif (_ppName[__i] == p.name) {{\n\
\t\t\tany = true;\n\
\t\t\tif (_ppAnim[__i] == a || _ppAnim[__i] == __partSrc(a)) {{\n\
\t\t\t\tif (p.__at != a) {{ p.__at = a; p.x = _ppOx[__i]; p.y = _ppOy[__i]; p.rotation = _ppRot[__i]; p.scaleX = _ppSx[__i]; p.scaleY = _ppSy[__i]; p.alpha = 1; p.visible = true; if (p.vfx != null && _ppArt[__i] != \"\" && p.vfx.getAnimation() != _ppArt[__i]) {{ p.vfx.playAnimation(_ppArt[__i]); }} }}\n\
\t\t\t\treturn true;\n\
\t\t\t}}\n\
\t\t}}\n\
\t\t__i = __i + 1;\n\
\t}}\n\
\tif (any) {{ p.__at = null; }}\n\
\treturn !any;\n\
}}\n\
function __part(name, anim, ox, oy, rot, sx, sy, labels) {{\n\
\tvar p = {{ vfx: null, name: name, anim: anim, labels: labels, x: ox, y: oy, rotation: rot, alpha: 1, visible: true, scaleX: sx, scaleY: sy, __at: null }};\n\
\t// a child with no picture of its own (a vector shape, a placed point) is a record alone\n\
\tif (anim != null) {{ p.vfx = match.createVfx(new VfxStats({{ spriteContent: self.getResource().getContent(\"{content_id}\"), animation: anim, loop: true, timeout: -1 }}), self); }}\n\
\tp.setRotation = function(r) {{ p.rotation = r; }};\n\
\tp.getRotation = function() {{ return p.rotation; }};\n\
\t// its place is kept in the source's units (the code reads `.x` raw); a setX/getX crosses the\n\
\t// boundary like any other, since the passes scale every setX argument on the way\n\
\tp.setX = function(v) {{ p.x = __isp(v); }};\n\
\tp.getX = function() {{ return __sp(p.x); }};\n\
\tp.setY = function(v) {{ p.y = __isp(v); }};\n\
\tp.getY = function() {{ return __sp(p.y); }};\n\
\tp.setAlpha = function(a) {{ p.alpha = a; }};\n\
\tp.getAlpha = function() {{ return p.alpha; }};\n\
\tp.setVisible = function(v) {{ p.visible = v; }};\n\
\tp.getVisible = function() {{ return p.visible; }};\n\
\tp.setScaleX = function(v) {{ p.scaleX = v; }};\n\
\tp.setScaleY = function(v) {{ p.scaleY = v; }};\n\
\tp.getScaleX = function() {{ return p.scaleX; }};\n\
\tp.getScaleY = function() {{ return p.scaleY; }};\n\
\tp.playFrame = function(f) {{ if (p.vfx != null) {{ p.vfx.playAnimation(anim); p.vfx.playFrame(Std.int(f) * 2 - 1); }} }};\n\
\t// a label it has plays as its animation; a frame number shows that frame and holds it (the\n\
\t// source's gotoAndStop(n), which is how a clip picks one of its pictures); anything else is nothing\n\
\tp.playFrameLabel = function(l) {{ if (p.vfx == null) {{ return; }} if (Std.isOfType(l, Float)) {{ p.playFrame(l); p.vfx.pause(); }} else if (p.labels.indexOf(l) >= 0) {{ p.vfx.playAnimation(anim + \"_\" + l); }} }};\n\
\tp.playAnimation = p.playFrameLabel;\n\
\tp.gotoAndPlay = function(l) {{ if (Std.isOfType(l, Float)) {{ p.playFrame(l); }} else {{ p.playFrameLabel(l); }} }};\n\
\tp.gotoAndStop = function(l) {{ p.gotoAndPlay(l); if (p.vfx != null) {{ p.vfx.pause(); }} }};\n\
\tp.stop = function() {{ if (p.vfx != null) {{ p.vfx.pause(); }} }};\n\
\tp.play = function() {{ if (p.vfx != null) {{ p.vfx.resume(); }} }};\n\
\tp.getCurrentFrame = function() {{ return p.vfx != null ? Std.int((p.vfx.getCurrentFrame() + 1) / 2) : 0; }};\n\
\t__parts.push(p);\n\
\tself.exports.__parts = __parts;\n\
\treturn p;\n\
}}\n\
function __partsUpdate() {{\n\
\tvar __i = 0;\n\
\twhile (__i < __parts.length) {{\n\
\t\tvar p = __parts[__i];\n\
\t\tvar v = p.vfx;\n\
\t\tif (v != null && !v.isDisposed() && !__partPlace(p)) {{ v.setVisible(false); }}\n\
\t\telse if (v != null && !v.isDisposed()) {{\n\
\t\t\tvar fx = self.isFacingRight() ? 1 : -1;\n\
\t\t\tvar rr = self.getRotation() * Math.PI / 180;\n\
\t\t\tvar dx = __sp(p.x);\n\
\t\t\tvar dy = __sp(p.y);\n\
\t\t\tv.setX(self.getX() + fx * (dx * Math.cos(rr) - dy * Math.sin(rr)));\n\
\t\t\tv.setY(self.getY() + (dx * Math.sin(rr) + dy * Math.cos(rr)));\n\
\t\t\tif (fx < 0) {{ v.faceLeft(); }} else {{ v.faceRight(); }}\n\
\t\t\tv.setRotation(self.getRotation() + fx * p.rotation);\n\
\t\t\tv.setAlpha(p.alpha);\n\
\t\t\tv.setVisible(p.visible);\n\
\t\t\tv.setScaleX(p.scaleX);\n\
\t\t\tv.setScaleY(p.scaleY);\n\
\t\t}}\n\
\t\t__i = __i + 1;\n\
\t}}\n\
}}\n\
function __partsDispose() {{\n\
\tvar __i = 0;\n\
\twhile (__i < __parts.length) {{ var v = __parts[__i].vfx; if (v != null && !v.isDisposed()) {{ v.dispose(); }} __i = __i + 1; }}\n\
\t__parts = [];\n\
}}\n"));
    if placements.is_empty() {
        decl.push_str("function __partSrc(a) { return a; }\n");
    } else {
        decl.push_str("// the clip an animation was cut from (`shield_in` is a piece of `shield`): a part placed on the clip is placed on every piece\n\
function __partSrc(a) {\n\
\tvar __i = 0;\n\
\twhile (__i < _lblAnim.length) { if (_lblAnim[__i] == a) { return _lblSrc[__i]; } __i = __i + 1; }\n\
\treturn a;\n\
}\n");
    }
    let mut init = String::new();
    // parents before their children, so a child lands on the record it belongs to
    let mut ordered: Vec<&entity_gen::ProjectilePartData> = parts.iter().collect();
    ordered.sort_by_key(|p| p.path.matches('.').count());
    for p in ordered {
        let m = &p.matrix;
        let labels: Vec<String> = p.labels.iter().map(|(l, _)| format!("\"{}\"", haxe_gen::label_ident(l))).collect();
        let art = if p.image_guids.is_empty() { "null".to_string() } else { format!("\"{}\"", p.anim) };
        init.push_str(&format!("    {} = __part(\"{}\", {}, {:.4}, {:.4}, {:.4}, {:.4}, {:.4}, [{}]);\n",
            p.path, p.path, art, m.tx, m.ty, m.rotation, m.sx, m.sy, labels.join(", ")));
    }
    if !placements.is_empty() {
        let q = |v: &str| format!("\"{v}\"");
        let col = |f: &dyn Fn(&entity_gen::PartPlacement) -> String| placements.iter().map(f).collect::<Vec<_>>().join(", ");
        decl.push_str(&format!("_ppName = [{}];\n_ppAnim = [{}];\n_ppArt = [{}];\n_ppOx = [{}];\n_ppOy = [{}];\n_ppRot = [{}];\n_ppSx = [{}];\n_ppSy = [{}];\n",
            col(&|r| q(&r.name)), col(&|r| q(&r.fm_anim)), col(&|r| q(&r.art)),
            col(&|r| format!("{:.4}", r.matrix.tx)), col(&|r| format!("{:.4}", r.matrix.ty)), col(&|r| format!("{:.4}", r.matrix.rotation)),
            col(&|r| format!("{:.4}", r.matrix.sx)), col(&|r| format!("{:.4}", r.matrix.sy))));
    }
    (decl, init)
}

/// The code's reach into a part, as the record takes it: a property written on it is a call
/// (`self.p.rotation = r` is `p.setRotation(r)`, which the vfx follows), and a part of this
/// object is addressed as the field it is here.
pub(crate) fn rewrite_part_refs(parts: &[entity_gen::ProjectilePartData], code: &mut String) {
    for part in parts {
        // a write, not a compare: the value starts with something other than a second `=`
        let re = crate::dyn_re(&format!(r"self\.{}\.(rotation|x|y|alpha|visible|scaleX|scaleY)\s*=[ \t]*([^=;\s][^;]*);", regex::escape(&part.path)));
        *code = re.replace_all(code, |c: &regex::Captures| {
            let prop = &c[1];
            let setter = format!("set{}{}", prop[..1].to_uppercase(), &prop[1..]);
            format!("self.{}.{setter}({});", part.path, &c[2])
        }).into_owned();
    }
    for part in parts {
        if part.path.contains('.') { continue }
        let re = crate::dyn_re(&format!(r"self\.{}\b", regex::escape(&part.path)));
        *code = re.replace_all(code, part.path.as_str()).into_owned();
    }
}

/// A SUMMONER'S SLOT READ BY ITS BARE NAME, pointed at the summoner: `v.get(` / `v.set(` for a
/// name the summoner keeps and this script does not.
fn owner_state_by_bare_name(code: &str, owner_vars: &std::collections::BTreeSet<String>, mine: &[String]) -> String {
    let mut script = code.to_string();
    for v in owner_vars {
        if mine.iter().any(|f| f == v) { continue }
        for tail in ["get(", "set("] {
            let bare = format!("{v}.{tail}");
            let owned = format!("self.getOwner().exports.{v}.{tail}");
            let mut acc = String::with_capacity(script.len());
            let mut rest = script.as_str();
            while let Some(at) = rest.find(&bare) {
                let before_ok = rest[..at].chars().last()
                    .map(|c| !c.is_alphanumeric() && c != '_' && c != '.')
                    .unwrap_or(true);
                acc.push_str(&rest[..at]);
                acc.push_str(if before_ok { &owned } else { &bare });
                rest = &rest[at + bare.len()..];
            }
            acc.push_str(rest);
            script = acc;
        }
    }
    script
}

/// A projectile asking for its own stats (to fire another of itself) names its own kind here.
fn own_kind_asked(code: &str, name: &str) -> String {
    crate::re!(r"\b(?:self\.)+exportStats\(\s*\)").replace_all(code, format!("{name:?}").as_str()).into_owned()
}

/// The death bounds are the projectile's script's to keep (see `__outOfBounds`): the engine's own
/// is turned off, and whether this survives them is handed to the script.
fn bounds_are_the_scripts(stats: &str) -> (String, bool) {
    let re = crate::re!(r"deathBoundsDestroy\s*:\s*(true|false)");
    let survives = re.captures(stats).is_some_and(|c| &c[1] == "false");
    if re.is_match(stats) {
        return (re.replace(stats, "deathBoundsDestroy: false").into_owned(), survives);
    }
    // no word on it: the engine's own default, which ends it at the bounds
    let out = match stats.find("spriteContent") {
        Some(i) => { let mut o = stats.to_string(); o.insert_str(i, "deathBoundsDestroy: false,\n    "); o }
        None => stats.to_string(),
    };
    (out, false)
}

/// A write that turns the bounds on or off as it runs (pikachu's bolt gives up surviving them once it
/// has fallen a while) goes to the script's own flag; the engine's stays off.
fn bounds_writes_to_the_script(code: &str) -> String {
    let re = crate::re!(r"(?m)^(\s*)(self\.updateProjectileStats\(\{[^{}]*?)deathBoundsDestroy\s*:\s*([^,}]+)([^{}]*\}\);)");
    re.replace_all(code, "${1}__survivesBounds = !(${3}); ${2}deathBoundsDestroy: false${4}").into_owned()
}

/// a projectile's art as its clip places the stance it is drawn in: the stance's frames are in
/// its own space, and the projectile's origin is the outer clip's
fn placed_by(mut pfi: crate::image_extractor::ProjectileFrameImages, stance: &crate::image_extractor::ImageLocalMatrix)
    -> crate::image_extractor::ProjectileFrameImages {
    for m in pfi.matrices.iter_mut().flatten() { *m = stance.compose(m); }
    for frame in pfi.parts.iter_mut() { for (_, m) in frame.iter_mut() { *m = stance.compose(m); } }
    pfi
}

/// the same placement, as the box extraction takes it
fn as_xform(m: &crate::image_extractor::ImageLocalMatrix) -> crate::sprite_parser::XframeTransform {
    crate::sprite_parser::XframeTransform { tx: m.tx, ty: m.ty, sx: m.sx, sy: m.sy, a: m.a, b: m.b, c: m.c, d: m.d }
}
