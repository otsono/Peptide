//! The packages the game loads ALONGSIDE whatever is playing.
//!
//! A stage does not have to carry the classes it uses. The game keeps a handful of shared packages
//! and has them loaded the whole time, so a stage's own code can name a class that lives in one of
//! them and the game resolves it without the stage file holding anything. A port that reads only
//! the stage file sees the call, finds nothing behind it, and ships a spawn that resolves to
//! nothing: crystal smash asks for its crystals thirteen times and got thirteen dangling refs.
//!
//! Nothing here knows what any package holds. The manifest states WHICH packages are loaded that
//! way; a class the stage names is then looked for in all of them, so a class nobody has written
//! yet is found the same way the ones shipping today are.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One shared package, decompressed and ready to parse.
pub struct SharedPackage {
    /// what the manifest files it under (`enemies`, `items`, ...) -- for logs only
    pub id: String,
    /// the movie inside the package
    pub swf: Vec<u8>,
    /// the same movie with its container unwrapped, so its tags can be walked for art
    buf: Option<swf::SwfBuf>,
}

impl SharedPackage {
    /// The package's tags. Held by the package itself, so what is read out of them lives as long.
    pub fn tags(&'static self) -> Vec<swf::Tag<'static>> {
        self.buf.as_ref()
            .and_then(|b| swf::parse_swf(b).ok())
            .map(|p| p.tags)
            .unwrap_or_default()
    }
}

/// The directory the content being converted was read from. When that IS the game's data (every
/// fighter converted from an installed game is), the shared packages are its neighbours, and
/// nothing has to be told where the game is.
static INPUT_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Note where the content being converted came from.
pub fn note_input_dir(dir: &Path) {
    if let Ok(mut g) = INPUT_DIR.lock() { *g = Some(dir.to_path_buf()); }
}

/// The installed game the conversion was told about (`ConvertOptions::game`).
static GAME: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Note where the installed game is: its app, or its data folder.
pub fn note_game(path: &Path) {
    if let Ok(mut g) = GAME.lock() { *g = Some(path.to_path_buf()); }
}

/// The data folder of a game given as its app or as that folder itself.
fn data_of(path: &Path) -> Option<PathBuf> {
    // a mac bundle keeps its resources one level in; every other build keeps them beside the binary
    [path.join("Contents/Resources/data"), path.join("data"), path.to_path_buf()].into_iter()
        .find(|c| c.is_dir() && game_swf(c).is_some())
}

/// The game's data directory: told to us, worked out from the app path, or the directory the
/// input came from when the game's own movie sits one level above it.
fn data_dir() -> Option<PathBuf> {
    if let Some(d) = GAME.lock().ok().and_then(|g| g.clone()).and_then(|g| data_of(&g)) { return Some(d); }
    if let Ok(d) = std::env::var("PEPTIDE_SSF2_DATA") {
        let p = PathBuf::from(d);
        if p.is_dir() { return Some(p); }
    }
    if let Ok(app) = std::env::var("PEPTIDE_SSF2_APP") {
        if let Some(d) = data_of(&PathBuf::from(app)) { return Some(d); }
    }
    let input = INPUT_DIR.lock().ok()?.clone()?;
    game_swf(&input).map(|_| input)
}

/// The movie the game's own code is in, which the manifest is read out of.
fn game_swf(data: &Path) -> Option<PathBuf> {
    let up = data.parent()?;
    let c = up.join("SSF2.swf");
    c.is_file().then_some(c)
}

/// Whether `dir` is the game's data directory itself (the packages [`all_data_packages`] reads).
pub fn is_data_dir(dir: &Path) -> bool {
    let (Some(data), Ok(d)) = (data_dir(), dir.canonicalize()) else { return false };
    data.canonicalize().is_ok_and(|data| data == d)
}

/// The game's own movie, which carries the sounds its fighters and menus share. Not searched for
/// classes: a stage borrows CODE from the packages the manifest names, and the movie is the engine.
pub fn game_movie() -> Option<&'static Vec<u8>> {
    static MOVIE: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    MOVIE.get_or_init(|| {
        let dir = data_dir()?;
        let swf = game_swf(&dir)?;
        std::fs::read(&swf).ok()
    }).as_ref()
}

/// Every package the manifest names ANYWHERE, for finding DATA rather than code.
///
/// [`packages`] is the set the game keeps loaded, which is the right scope for a class: a stage
/// borrows code from those and nowhere else. A SOUND is not borrowed, it is named by id, and a
/// fighter names clips that live in packages outside that set -- mario asks for `jump_common` and
/// his two landing sounds and carries none of them. So the search for data is the whole manifest.
pub fn every_package() -> Vec<(String, Vec<u8>)> {
    let Some(dir) = data_dir() else { return Vec::new() };
    let Some(swf) = game_swf(&dir) else { return Vec::new() };
    let Ok(inv) = crate::ssf2_manifest::read(&swf, &dir) else { return Vec::new() };
    let mut out = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    for section in [&inv.misc, &inv.extra, &inv.modes, &inv.menu] {
        for (id, entry) in section {
            if !seen.insert(id.clone()) { continue }
            let file = if entry.file_pub.is_empty() { &entry.file } else { &entry.file_pub };
            let Ok(raw) = std::fs::read(dir.join(file)) else { continue };
            if let Ok(swf) = crate::ssf::decompress(&raw) { out.push((id.clone(), swf)); }
        }
    }
    out
}

/// EVERY package beside the game, whether the manifest names it or not.
///
/// The manifest accounts for the packages the game keeps loaded; it does not account for every
/// place a piece of DATA lives. Ten of the sounds mario names sit in a shared bank the manifest
/// never mentions, so a search that stops at the manifest calls them missing. Reading the lot
/// costs a couple of seconds, so it is the LAST place to look rather than the first.
pub fn all_data_packages() -> Vec<(String, Vec<u8>)> {
    let Some(dir) = data_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "ssf")).collect();
    files.sort();
    files.into_iter().filter_map(|p| {
        let raw = std::fs::read(&p).ok()?;
        let swf = crate::ssf::decompress(&raw).ok()?;
        Some((p.file_name()?.to_string_lossy().to_string(), swf))
    }).collect()
}

/// What the game's own roster calls a fighter id, when the game is at hand and lists it.
pub fn roster_name(id: &str) -> Option<String> {
    static ROSTER: OnceLock<std::collections::BTreeMap<String, String>> = OnceLock::new();
    ROSTER.get_or_init(|| {
        let Some(dir) = data_dir() else { return Default::default() };
        let Some(swf) = game_swf(&dir) else { return Default::default() };
        let Ok(inv) = crate::ssf2_manifest::read(&swf, &dir) else { return Default::default() };
        inv.character.into_iter().filter(|(_, e)| !e.name.is_empty())
            .map(|(id, e)| (id.to_lowercase(), e.name)).collect()
    }).get(&id.to_lowercase()).cloned()
}

/// Every package the manifest declares as loaded alongside, decompressed once.
pub fn packages() -> &'static [SharedPackage] {
    static PKGS: OnceLock<Vec<SharedPackage>> = OnceLock::new();
    PKGS.get_or_init(|| {
        let Some(dir) = data_dir() else {
            log::warn!("no SSF2 data directory (set $PEPTIDE_SSF2_DATA or $PEPTIDE_SSF2_APP): a \
                        class a stage names but does not carry cannot be resolved");
            return Vec::new();
        };
        let Some(swf) = game_swf(&dir) else {
            log::warn!("no game movie beside {}: the shared-package list is declared there", dir.display());
            return Vec::new();
        };
        let inv = match crate::ssf2_manifest::read(&swf, &dir) {
            Ok(i) => i,
            Err(e) => { log::warn!("cannot read the shared-package list: {e}"); return Vec::new(); }
        };
        let mut out = Vec::new();
        for (id, entry) in &inv.misc {
            let file = if entry.file_pub.is_empty() { &entry.file } else { &entry.file_pub };
            let path = dir.join(file);
            let Ok(raw) = std::fs::read(&path) else { continue };
            match crate::ssf::decompress(&raw) {
                Ok(swf) => {
                    let buf = swf::decompress_swf(&swf[..]).ok();
                    out.push(SharedPackage { id: id.clone(), swf, buf });
                }
                Err(e) => log::debug!("shared package {id} ({}) does not open: {e}", path.display()),
            }
        }
        log::info!("{} shared packages loaded from {}", out.len(), dir.display());
        out
    })
}

/// Every package the manifest names, in any of its lists: the ones loaded alongside
/// ([`packages`], first and in the same order) and then the rest (the modes', the menus'). A
/// clip a stage's code makes by name can live in any of them (the target tests' background is
/// a mode's), so a search for one that `packages` does not answer goes on through these.
pub fn manifest_packages() -> &'static [SharedPackage] {
    static ALL: OnceLock<Vec<SharedPackage>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut out: Vec<SharedPackage> = packages().iter().map(|p| SharedPackage {
            id: p.id.clone(), swf: p.swf.clone(), buf: swf::decompress_swf(&p.swf[..]).ok(),
        }).collect();
        let have: std::collections::BTreeSet<String> = out.iter().map(|p| p.id.clone()).collect();
        for (id, swf) in every_package() {
            if have.contains(&id) { continue }
            let buf = swf::decompress_swf(&swf[..]).ok();
            out.push(SharedPackage { id, swf, buf });
        }
        out
    })
}

/// One of the game's music tracks by its id (`bgm_battlefield`): the manifest's entry for it (its
/// name) and its package, decompressed. A stage names its tracks by id; each lives in a package
/// of its own.
pub fn music_package(id: &str) -> Option<(crate::ssf2_manifest::Entry, Vec<u8>)> {
    let dir = data_dir()?;
    let swf = game_swf(&dir)?;
    static INV: OnceLock<Option<crate::ssf2_manifest::Inventory>> = OnceLock::new();
    let inv = INV.get_or_init(|| crate::ssf2_manifest::read(&swf, &dir).ok()).as_ref()?;
    let entry = inv.music.get(id).or_else(|| inv.music.iter().find(|(k, _)| k.eq_ignore_ascii_case(id)).map(|(_, e)| e))?.clone();
    let file = if entry.file_pub.is_empty() { &entry.file } else { &entry.file_pub };
    let raw = std::fs::read(dir.join(file)).ok()?;
    let movie = crate::ssf::decompress(&raw).ok()?;
    Some((entry, movie))
}

/// The lists a package registers under a name of its own (`register("pokemon", { common: [...],
/// rare: [...] })`), by list, each as the short names of the classes it holds. The package is the
/// one the manifest files under `package`; a list the game hands out (`getPokemonStatsList`) is
/// what that package registered, so the classes a call can pick are the ones written there.
pub fn registered_lists(package: &str, prop: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    use crate::model::ast::{walk_exprs, Expr};
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    static MEMO: OnceLock<Mutex<BTreeMap<(String, String), BTreeMap<String, Vec<String>>>>> = OnceLock::new();
    let memo = MEMO.get_or_init(|| Mutex::new(BTreeMap::new()));
    let key = (package.to_string(), prop.to_string());
    if let Some(v) = memo.lock().ok().and_then(|m| m.get(&key).cloned()) { return v }
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let short = |n: &str| n.rsplit(['.', ':']).next().unwrap_or(n).to_string();
    if let Some(pkg) = manifest_packages().iter().find(|p| p.id == package) {
        if let Ok(model) = crate::model::SourceModel::read(&pkg.swf) {
            for c in model.classes.values() {
                let bodies = std::iter::once(&c.constructor.body).chain(c.methods.iter().map(|m| &m.function.body));
                for body in bodies {
                    walk_exprs(body, &mut |e| {
                        let Expr::Call(g, a) = e else { return };
                        if !matches!(&**g, Expr::Member(_, n) | Expr::Name(n) if n == "register") { return }
                        let [Expr::Str(name), Expr::Object(pairs)] = a.as_slice() else { return };
                        if name != prop { return }
                        for (k, v) in pairs {
                            let (Expr::Str(k), Expr::Array(items)) = (k, v) else { continue };
                            let names: Vec<String> = items.iter().filter_map(|x| match x {
                                Expr::Name(n) => Some(short(n)),
                                Expr::Member(_, n) => Some(n.clone()),
                                _ => None,
                            }).collect();
                            out.entry(k.clone()).or_default().extend(names);
                        }
                    });
                }
            }
        }
    }
    if let Ok(mut m) = memo.lock() { m.insert(key, out.clone()); }
    out
}

/// The game's metal look, as its shared packages declare it (`getMetalCostumeData`'s `default`): a
/// record of the colour adjustment by name (saturation, brightness, contrast, multipliers, offsets)
pub fn metal_look() -> Option<std::collections::BTreeMap<String, f64>> {
    static MEMO: OnceLock<Option<std::collections::BTreeMap<String, f64>>> = OnceLock::new();
    MEMO.get_or_init(|| {
        for pkg in manifest_packages() {
            let Ok(model) = crate::model::SourceModel::read(&pkg.swf) else { continue };
            for c in model.classes.values() {
                let Some(m) = c.methods.iter().find(|m| m.name == "getMetalCostumeData") else { continue };
                let Some(v) = crate::model::declare::returned(&m.function.body) else { continue };
                let Some(def) = v.get("default") else { continue };
                let out: std::collections::BTreeMap<String, f64> = def.entries().iter()
                    .filter_map(|(k, x)| match x { crate::model::declare::Value::Num(n) => Some((k.clone(), *n)), _ => None }).collect();
                if !out.is_empty() { return Some(out) }
            }
        }
        None
    }).clone()
}

/// The box a fighter shot out as a star hits with, as the installed game's own fighter declares it
/// as it is made (the attack named `star` its code fills in): each numeric stat by name. Read from
/// the game movie itself, so nothing of it is carried here.
pub fn star_attack() -> Option<std::collections::BTreeMap<String, f64>> {
    use crate::model::ast::{walk_exprs, Expr};
    static MEMO: OnceLock<Option<std::collections::BTreeMap<String, f64>>> = OnceLock::new();
    MEMO.get_or_init(|| {
        let data = data_dir()?;
        let raw = std::fs::read(game_swf(&data)?).ok()?;
        let bytes = crate::ssf::decompress(&raw).unwrap_or(raw);
        let model = crate::model::SourceModel::read(&bytes).ok()?;
        let mut found: Option<std::collections::BTreeMap<String, f64>> = None;
        for c in model.classes.values() {
            for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor]) {
                walk_exprs(&f.body, &mut |e| {
                    if found.is_some() { return }
                    let Expr::Call(g, a) = e else { return };
                    let Expr::Member(on, m) = &**g else { return };
                    if m != "importAttackData" { return }
                    let named_star = matches!(&**on, Expr::Call(h, ha) if matches!(&**h, Expr::Member(_, n) if n == "getAttack") && matches!(ha.as_slice(), [Expr::Str(s)] if s == "star"));
                    if !named_star { return }
                    let Some(Expr::Object(kv)) = a.first() else { return };
                    // (its first box's numbers)
                    let boxes = kv.iter().find(|(k, _)| matches!(k, Expr::Str(s) if s == "attackBoxes")).map(|(_, v)| v);
                    let Some(Expr::Object(bs)) = boxes else { return };
                    let Some((_, Expr::Object(b))) = bs.first() else { return };
                    let mut out = std::collections::BTreeMap::new();
                    for (k, v) in b {
                        let (Expr::Str(k), n) = (k, v) else { continue };
                        let n = match n { Expr::Int(i) => *i as f64, Expr::Num(x) => *x, _ => continue };
                        out.insert(k.clone(), n);
                    }
                    if !out.is_empty() { found = Some(out) }
                });
            }
        }
        found
    }).clone()
}
