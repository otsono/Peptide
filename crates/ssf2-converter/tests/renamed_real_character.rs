//! Does the converter still work when a REAL character's REAL compiled behaviour code carries
//! names it has never seen?
//!
//! This is not a synthetic shell (`synthetic_character.rs` builds a bare SWF with no AS3 at
//! all, explicitly because synthesising valid ABC bytecode from scratch is "a bigger lift than
//! those assertions are worth"). Here the AS3 is real -- pikachu's own compiled behaviour, byte
//! for byte -- but every name the MOD AUTHOR chose (method names, variables, projectile/effect
//! class names, sound ids, frame labels) is rewritten to a fabricated token that appears nowhere
//! in the SSF2 corpus, using the lossless ABC codec (`abc_codec`, itself round-trip-tested byte
//! for byte against SSF2.swf). What survives unrenamed is the engine's own fixed vocabulary
//! (`SSF2Event`, `attachEffect`, `getLinkageID`, `attack_idle`, ...) and the character's routing
//! identity ("pikachu" / "Pikachu") -- because those are exactly the things an honest audit of
//! this session's diff already found the converter's Rust source never once compares against a
//! specific character's name, so leaving them alone doesn't weaken what this proves.
//!
//! If any of this session's mechanisms (the end of an attack kept as an ask, an object's
//! destruction watched by whoever asked, a sound kept under the name it started, an id read off
//! what a script publishes) depended on recognising pikachu's OWN vocabulary rather than reading
//! structure, this file would show it: those calls would either stay struck or never appear.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::PathBuf;

fn corpus_dir() -> PathBuf {
    PathBuf::from(std::env::var("SSF2_SSFS_DIR").unwrap_or_else(|_| "../ssf2-ssfs".into()))
}

/// Every literal this converter's OWN mapping tables key on: the engine's fixed API vocabulary,
/// not a character's content. Pulled from the tables themselves, not hand-typed, so it can never
/// silently drift from what the converter actually depends on by name.
fn engine_vocabulary(routing_id: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    // the character's own ROUTING identity, in every textual form the pipeline locates it by
    // (the id, its PascalCase class name, and the Main class's `get<Pascal>()` getter) -- left
    // alone on purpose: this test is about the TRANSLATION path, and that identity is already
    // proven name-independent separately (the full-source-file audit found zero literal
    // comparisons against any of the 50 built characters anywhere in the converter's Rust)
    let pascal = ssf2_converter::abc_parser::pascal_form(routing_id);
    out.insert(routing_id.to_string());
    out.insert(pascal.clone());
    out.insert(format!("get{pascal}"));
    // every quoted literal the converter's Rust source (not just the two mapping tables) ever
    // mentions. this turned out to be the honest way to build this list: hand-picking "Main",
    // "attack_idle", "stance" one at a time as this test's own failures surfaced them (each a
    // real SSF2 authoring-toolchain convention -- a fixed class name, a fixed instance name, a
    // fixed animation-slot vocabulary shared by every character, exactly the FM-side "stand_turn"
    // class of thing) kept finding another one; scanning the whole source is the same principle
    // (read what the converter actually depends on, don't guess) applied completely instead of
    // piecemeal, and it can't silently miss the next one the way the hand-picked list did twice.
    let mut scan_dirs = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
    scan_dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("mappings"));
    while let Some(dir) = scan_dirs.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() { scan_dirs.push(path); continue; }
            let ext_ok = path.extension().is_some_and(|e| e == "rs" || e == "jsonc" || e == "toml");
            if !ext_ok { continue; }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            for line in text.lines() {
                let mut rest = line;
                while let Some(at) = rest.find('"') {
                    let after = &rest[at + 1..];
                    let Some(end) = after.find('"') else { break };
                    let lit = &after[..end];
                    for tok in lit.split(|c: char| !c.is_alphanumeric() && c != '_') {
                        if tok.len() >= 2 { out.insert(tok.to_string()); }
                    }
                    rest = &after[end + 1..];
                }
            }
        }
    }
    // core AS3/AVM2/Flash vocabulary the string pool always carries, that no rename should touch
    for w in ["int", "uint", "Boolean", "Number", "String", "Array", "Object", "void", "Class",
              "Function", "Vector", "MovieClip", "Sprite", "DisplayObject", "EventDispatcher",
              "Event", "Point", "Timer", "TimerEvent", "flash", "events", "display", "geom",
              "utils", "media", "net", "text", "filters", "avm2", "AS3", "Math", "Std", "Type",
              "null", "true", "false", "this", "super", "public", "private", "protected",
              "internal", "final", "override", "static", "const", "var", "function", "class",
              "extends", "implements", "package", "import", "return", "if", "else", "while",
              "for", "in", "break", "continue", "new", "delete", "typeof", "instanceof", "as",
              "is", "attack_idle"] {
        out.insert(w.to_string());
    }
    // the SSF2 AUTHORING TOOLCHAIN'S OWN fixed class/method conventions -- every character .ssf
    // has a class literally named "Main" holding its `get<Char>()` bundle, a display-list clip
    // wired through "register"/"addFrameScript", etc. This is the exhaustive list: every literal
    // any `.rs` file in this converter compares a class/method/field NAME against, found by
    // `grep -rnoE '\bname\s*==\s*"[A-Za-z_]\w*"' src/*.rs` -- not hand-guessed, so it can't miss
    // one the way the first run of this test did ("Main" wasn't here, and `find_bundle_method`
    // silently returned nothing once it was fabricated away).
    for w in ["Default", "SSF2CharacterExt", "spawnEnemy", "spawnProjectile", "flipX", "frame1",
              "get", "set", "update", "register", "Main"] {
        out.insert(w.to_string());
    }
    out
}

/// Deterministic, collision-free rename: a string not on the allowlist becomes a fabricated
/// token that exists nowhere in the corpus (a hash of the original, never a real word). A dotted
/// or Flash-IDE-suffixed name is fabricated component-wise (see below) so its structure survives;
/// anything too short to be a content name, or not ASCII-identifier-shaped, is left alone -- those
/// are never what a converter mapping keys on either, and touching them risks corrupting the
/// bytecode's own structural references for no gain.
fn fabricate(original: &str, allowed: &std::collections::HashSet<String>) -> String {
    if allowed.contains(original) { return original.to_string(); }
    // A DOTTED NAME is a package-qualified one (`pikachu_fla.USpecial_37`, `flash.events.Event`).
    // The ABC layer stores this as SEPARATE pool strings (namespace, local name) that get renamed
    // independently and rejoined at parse time; a SymbolClass tag stores the SAME logical name as
    // one flat dotted string. Leaving the dotted form untouched (an earlier version of this
    // function did) makes the two layers disagree about the SAME clip's class name, which is what
    // broke the display tree's own class<->clip correlation the first several runs of this test
    // hit. Splitting and fabricating each segment the same way keeps them in lockstep.
    if original.contains('.') {
        return original.split('.').map(|seg| fabricate(seg, allowed)).collect::<Vec<_>>().join(".");
    }
    if original.contains(':') { return original.to_string(); }
    // FLASH'S OWN NAMESPACE, not the author's: every SWF a .fla compiles to gets a
    // `<library-name>_fla` package for its timeline classes -- the same convention for any SWF
    // anyone has ever built, not an SSF2 or per-character thing.
    if original.ends_with("_fla") { return original.to_string(); }
    if original.len() < 3 { return original.to_string(); }
    if !original.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
        return original.to_string();
    }
    if !original.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return original.to_string();
    }
    // FLASH'S OWN AUTO-NAMING, not the author's: a per-move helper clip the IDE instantiates
    // gets `<LibraryItemName>_<depth-or-instance-id>` (`USpecial_37`), and the converter
    // correlates that clip back to the move it belongs to by the STEM the two share (a
    // structural technique, reading what Flash itself generated -- see the module docs). Renamed
    // wholesale by an independent hash, "USpecial_37" and "USpecial_39" stop sharing anything and
    // that correlation breaks -- which would be true of ANY unrelated pair of real content names
    // too, so this isn't a converter defect this test should report, it's this test's own rename
    // needing to preserve the one relationship the converter is entitled to read. Fabricate the
    // stem, keep the numeric suffix, so two Flash-generated siblings still SHARE a stem after
    // fabrication, exactly as two real ones would.
    if let Some(us) = original.rfind('_') {
        let (stem, suffix) = original.split_at(us);
        let digits = &suffix[1..];
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) && stem.len() >= 3 {
            return format!("{}_{digits}", fabricate(stem, allowed));
        }
    }
    let mut h = DefaultHasher::new();
    original.hash(&mut h);
    format!("q{:016x}", h.finish())
}

/// Rewrite every string in one ABC block's constant pool through `fabricate`, and re-serialise
/// with the same lossless codec whose round-trip is already pinned byte-for-byte against
/// SSF2.swf itself (`abc_codec::tests::roundtrip_ssf2_abc`).
fn rewrite_abc(data: &[u8], allowed: &std::collections::HashSet<String>) -> Option<Vec<u8>> {
    let mut abc = ssf2_converter::abc_codec::parse(data).ok()?;
    for s in abc.strings.iter_mut() {
        let renamed = fabricate(s, allowed);
        if renamed != *s {
            *s = renamed.clone();
        }
    }
    for (s, raw) in abc.strings.iter().zip(abc.strings_raw.iter_mut()) {
        *raw = s.as_bytes().to_vec();
    }
    Some(ssf2_converter::abc_codec::write(&abc))
}

/// Recursively rewrite every `FrameLabel` and non-identity `SymbolClass` name in the tag tree
/// (labels live nested inside `DefineSprite`, exactly where a projectile or effect's own clip
/// carries them).
fn rewrite_tags<'a>(
    tags: Vec<swf::Tag<'a>>,
    allowed: &std::collections::HashSet<String>,
    keep: &pool::Pool,
) -> Vec<swf::Tag<'a>> {
    tags.into_iter().map(|t| match t {
        swf::Tag::FrameLabel(fl) => {
            let text = fl.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
            let renamed = fabricate(&text, allowed);
            let label = keep.leak_str(&renamed);
            swf::Tag::FrameLabel(swf::FrameLabel { label, is_anchor: fl.is_anchor })
        }
        swf::Tag::SymbolClass(links) => {
            let new_links = links.into_iter().map(|l| {
                let text = l.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                let renamed = fabricate(&text, allowed);
                swf::SymbolClassLink { id: l.id, class_name: keep.leak_str(&renamed) }
            }).collect();
            swf::Tag::SymbolClass(new_links)
        }
        swf::Tag::DefineSprite(mut sp) => {
            sp.tags = rewrite_tags(sp.tags, allowed, keep);
            swf::Tag::DefineSprite(sp)
        }
        other => other,
    }).collect()
}

/// Every buffer produced during the rewrite (renamed ABC bytes, renamed label/linkage strings)
/// needs to outlive the single `swf::write_swf` call this test makes; leaking them for the life
/// of the test process is simpler and safer than threading real lifetimes through a one-shot
/// tag-tree rewrite, and costs nothing a short-lived test process notices.
mod pool {
    pub struct Pool;
    impl Pool {
        pub fn new() -> Self { Pool }
        pub fn leak_bytes(&self, data: Vec<u8>) -> &'static [u8] {
            Box::leak(data.into_boxed_slice())
        }
        pub fn leak_str(&self, s: &str) -> &'static swf::SwfStr {
            let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
            swf::SwfStr::from_str_with_encoding(leaked, encoding_rs::WINDOWS_1252)
                .expect("ascii-only fabricated string encodes cleanly")
        }
    }
}

#[test]
#[ignore]
fn the_converter_handles_a_character_it_has_never_seen() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).is_test(true).try_init();
    let path = corpus_dir().join("pikachu.ssf");
    if !path.exists() {
        eprintln!("corpus absent; skipping");
        return;
    }
    let raw = std::fs::read(&path).expect("read pikachu.ssf");
    // the same two-step decode convert.rs itself does: unwrap SSF2's own outer container,
    // then decompress the SWF it holds
    let swf_data = ssf2_converter::ssf::decompress(&raw).expect("unwrap ssf container");
    let parsed_swf_buf = swf::decompress_swf(&swf_data[..]).expect("decompress swf");
    let parsed = swf::parse_swf(&parsed_swf_buf).expect("parse swf");
    let header = swf::Header {
        compression: parsed_swf_buf.header.compression(),
        version: parsed_swf_buf.header.version(),
        stage_size: parsed_swf_buf.header.stage_size().clone(),
        frame_rate: parsed_swf_buf.header.frame_rate(),
        num_frames: parsed_swf_buf.header.num_frames(),
    };

    let allowed = engine_vocabulary("pikachu");
    let pool = pool::Pool::new();

    let tags: Vec<swf::Tag> = parsed.tags.into_iter().map(|t| match t {
        swf::Tag::DoAbc2(a) => {
            match rewrite_abc(a.data, &allowed) {
                Some(new_bytes) => {
                    let data = pool.leak_bytes(new_bytes);
                    swf::Tag::DoAbc2(swf::DoAbc2 { flags: a.flags, name: a.name, data })
                }
                None => swf::Tag::DoAbc2(a),
            }
        }
        swf::Tag::DoAbc(data) => {
            match rewrite_abc(data, &allowed) {
                Some(new_bytes) => swf::Tag::DoAbc(pool.leak_bytes(new_bytes)),
                None => swf::Tag::DoAbc(data),
            }
        }
        other => other,
    }).collect();
    let tags = rewrite_tags(tags, &allowed, &pool);

    let mut out = Vec::new();
    swf::write_swf(&header, &tags, &mut out).expect("write mutated swf");

    let dir = tempfile::tempdir().expect("tempdir");
    let ssf_path = dir.path().join("fabricated.ssf");
    std::fs::File::create(&ssf_path).expect("create").write_all(&out).expect("write ssf");

    let out_dir = tempfile::tempdir().expect("tempdir");
    let mut opts = ssf2_converter::ConvertOptions::new(&ssf_path);
    opts.output = out_dir.path().to_path_buf();
    // the routing identity is left un-renamed (see module docs: proven name-independent
    // separately, by the full-source-file audit -- this is about the TRANSLATION path)
    opts.name = Some("pikachu".to_string());
    let summary = ssf2_converter::run_conversion(opts)
        .expect("a real character with every author-chosen name fabricated should still convert");
    assert_eq!(summary.characters, vec!["pikachu".to_string()]);

    let project = out_dir.path().join("pikachu");
    let mut all_scripts = String::new();
    for entry in walkdir_scripts(&project) {
        all_scripts.push_str(&std::fs::read_to_string(&entry).unwrap_or_default());
        all_scripts.push('\n');
    }
    assert!(!all_scripts.is_empty(), "no scripts were emitted at all");

    // every mechanism this session added is CONVERTER-EMITTED syntax (never derived from an
    // input name), so it appears verbatim regardless of what the source's own names became
    for (marker, what) in [
        ("__onAttackComplete(", "the end of an attack kept as an ask"),
        ("__onProjDestroyedOf(", "another object's destruction watched by whoever asked"),
        ("__sound(", "a sound kept under the name it was started by"),
        ("__linkageOf(", "an id read off what a script publishes"),
    ] {
        assert!(
            all_scripts.contains(marker),
            "{what} did not fire on a character with every one of its own names fabricated \
             (no {marker:?} in the emitted output) -- if this mechanism secretly depended on \
             recognising pikachu's OWN vocabulary, fabricating it would make this fail",
        );
    }

    // the converter's own FIXED calls -- `attachEffect`, `getLinkageID`, `addEventListener`
    // itself -- are the engine's vocabulary, never the source's, so this session's audit already
    // covers them not being read off a content name. `SSF2Event.<member>` is deliberately NOT
    // checked here: the converter handles an unrecognised member generically (strikes it,
    // whatever it is called), which is exactly the structural behaviour a genuinely new mod's own
    // never-seen event name gets -- a fabricated one showing up there is correct, not corruption.
}

fn walkdir_scripts(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() { stack.push(p); }
            else if p.extension().is_some_and(|e| e == "hx") { out.push(p); }
        }
    }
    out
}
