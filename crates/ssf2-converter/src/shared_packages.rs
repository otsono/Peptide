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

/// The game's data directory: told to us, worked out from the app path, or the directory the
/// input came from when the game's own movie sits one level above it.
fn data_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("PEPTIDE_SSF2_DATA") {
        let p = PathBuf::from(d);
        if p.is_dir() { return Some(p); }
    }
    if let Ok(app) = std::env::var("PEPTIDE_SSF2_APP") {
        let app = PathBuf::from(app);
        // a mac bundle keeps its resources one level in; every other build keeps them beside the binary
        if let Some(d) = [app.join("Contents/Resources/data"), app.join("data")].into_iter().find(|c| c.is_dir()) {
            return Some(d);
        }
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
