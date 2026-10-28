//! In-process conversion entry point.
//!
//! This is the library face of what used to be the `ssf2_converter` binary's
//! `main()`. The binary is gone — `peptide convert …` and the Peptide GUI both
//! call [`run_conversion`] directly, in-process. The orchestration and the
//! per-character pipeline that used to live in `src/main.rs` moved here verbatim.
//!
//! ## Concurrency
//! `run_conversion` is **not** safe to call concurrently. The conversion log is a
//! process-global `Mutex<ConversionLog>` (reset per character) and the extractors
//! use `thread_local!` caches. Callers driving conversions from a long-running GUI
//! must run each conversion on its own worker thread and never two at once.
//! Logging is via the `log` facade; the caller owns logger initialisation (the
//! library never calls `env_logger::init`).

use std::path::{Path, PathBuf};
use std::fs;

use anyhow::{Context, Result};

use crate::project::{ManifestCharEntry, MultiCharSlot, ProcessedCharacter};
use crate::{
    abc_parser, extractor, haxe_gen, image_extractor, mappings, sound_extractor, sprite_parser, ssf,
    swf_parser,
};

/// Inputs to a single conversion run. Mirrors the old CLI flags.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// The `.ssf` (or raw SWF) to convert.
    pub input: PathBuf,
    /// Output directory for generated Fraymakers files (CLI default: `./characters`).
    pub output: PathBuf,
    /// Character-name override. For a multi-character `.ssf`, selects only that
    /// character. Auto-detected from the SWF when `None`.
    pub name: Option<String>,
    /// Explicit `misc.ssf` for costume palettes. Auto-detected next to the input
    /// when `None`.
    pub misc_ssf: Option<PathBuf>,
    /// Rollback flag: emit each character of a multi-character `.ssf` as its own
    /// project (the pre-Stage-B layout) instead of one unified project.
    pub per_character_projects: bool,
}

impl ConvertOptions {
    /// Construct with the CLI defaults (`output = ./characters`, everything else off).
    pub fn new(input: impl Into<PathBuf>) -> Self {
        ConvertOptions {
            input: input.into(),
            output: PathBuf::from("./characters"),
            name: None,
            misc_ssf: None,
            per_character_projects: false,
        }
    }
}

/// Result of a conversion run — what the UI / CLI report afterwards.
#[derive(Debug, Clone)]
pub struct ConversionSummary {
    /// The directory the project was written to (`output`, or `output/<project_id>`
    /// for a unified multi-character project).
    pub project_dir: PathBuf,
    /// Character ids processed.
    pub characters: Vec<String>,
    /// True when this run produced one unified multi-character project.
    pub multi_char: bool,
    /// The unified project's id, when `multi_char`.
    pub project_id: Option<String>,
    /// Every `.fraytools` project file produced by this run.
    pub fraytools_files: Vec<PathBuf>,
    /// Per-character conversion-log blocks (multi-char only; single-char writes
    /// its own `conversion_log.json` inline).
    pub log_blocks: Vec<serde_json::Value>,
    /// Tier-1 validation warnings collected across all characters (also logged).
    pub warnings: Vec<String>,
}

/// Convert one `.ssf` into a Fraymakers character package (or a unified
/// multi-character project). In-process; see the module docs for the concurrency
/// contract. The caller initialises logging.
pub fn run_conversion(opts: ConvertOptions) -> Result<ConversionSummary> {
    // a fighter's and its projectiles' positions cross the size scale, in every script this
    // conversion translates (class methods, frame scripts, the projectiles' own); a stage's
    // conversion never comes through here
    let _positions_guard = crate::api_mappings::PositionsCrossGuard::install();
    log::info!("SSF2 → Fraymakers Converter");
    log::info!("Input: {}", opts.input.display());

    // what it borrows (shared sounds, shared classes) is looked for beside it, when beside it is
    // the game's own data
    if let Some(dir) = opts.input.parent() { crate::shared_packages::note_input_dir(dir); }
    // Read + decompress SSF
    let ssf_data = fs::read(&opts.input).with_context(|| format!("read {}", opts.input.display()))?;
    log::info!("Loaded {} bytes", ssf_data.len());
    let swf_data = ssf::decompress(&ssf_data)?;
    log::info!("Decompressed SWF: {} bytes", swf_data.len());

    let swf = swf_parser::parse(&swf_data)?;
    log::info!("Parsed SWF: v{}, {} ABC blocks", swf.version, swf.abc_blocks.len());

    // Parse the SWF tag tree once for the whole file. The tags (sprites, images,
    // sounds) are identical across every character in a multi-char .ssf, so the
    // per-character extractors all borrow this one parse instead of re-parsing.
    let parsed_swf_buf = swf::decompress_swf(&swf_data[..])
        .map_err(|e| anyhow::anyhow!("decompress SWF: {}", e))?;
    let parsed_swf = swf::parse_swf(&parsed_swf_buf)
        .map_err(|e| anyhow::anyhow!("parse SWF: {}", e))?;

    // ── Palette extraction from misc.ssf ──────────────────────────────────────
    // Explicit --misc-ssf, else misc.ssf next to the input. Extract all costume
    // data in-process to a temp JSON cache, deleted when done.
    let costumes_path: Option<PathBuf> = opts.misc_ssf.clone().and_then(|p| {
        match extract_costumes_to_temp(&p) {
            Ok(path) => Some(path),
            Err(e) => {
                log::warn!("Costume extraction from {:?} failed: {}", p, e);
                None
            }
        }
    }).or_else(|| {
        // THE COSTUMES ARE IN THE GAME'S DATA, wherever it keeps them. A loose copy names the
        // file misc.ssf; the installed game keeps the same table in one of its numbered DAT
        // archives. Without it every costume here is invented, and the default one shows the
        // art's BASE colours, which the source always recolours (mario's cap is green in the
        // raw art and red only through his default costume).
        let Some(misc_ssf) = costume_source_near(opts.input.parent()?) else {
            log::warn!("no costume table found beside the input -- costumes will be invented, and the default one will show the art's base colours");
            return None;
        };
        log::info!("Found the costume table in {} -- extracting costume palettes...", misc_ssf.display());
        match extract_costumes_to_temp(&misc_ssf) {
            Ok(path) => {
                log::info!("Costume palettes cached to {}", path.display());
                Some(path)
            }
            Err(e) => {
                log::warn!("Costume extraction failed: {} — palettes will use k-means fallback", e);
                None
            }
        }
    });
    let costumes_is_temp = costumes_path.is_some();

    // Determine which character names to process
    let char_names: Vec<String> = if let Some(name) = opts.name.clone() {
        vec![name]
    } else {
        let detected = detect_char_names(&swf, &opts.input);
        if detected.is_empty() {
            let stem = opts.input.file_stem().and_then(|s| s.to_str()).unwrap_or("character");
            // SSF2 `DAT<n>.ssf` archives are index-named (stages, items, UI, …), so a
            // stem-as-character fallback would mint a junk "DAT10" pseudo-character for
            // every non-character archive. Bail clearly instead — the stem fallback is
            // only meaningful for a hand-named single-character .swf.
            if is_dat_archive_stem(stem) {
                anyhow::bail!(
                    "{} is a valid SSF2 DAT archive but contains no convertible character \
                     (likely a stage, item, or other asset). Pass --name <id> to force a \
                     specific character if you believe one is present.",
                    opts.input.display()
                );
            }
            vec![stem.to_string()]
        } else {
            detected
        }
    };

    log::info!("Characters to process: {:?}", char_names);
    // one fighter asked for by name out of a package that holds others is its own project, beside
    // the others' projects: its identities are scoped to it so the two never collide in a match
    let _project_ns = (opts.name.is_some() && detect_char_names(&swf, &opts.input).len() > 1)
        .then(|| crate::uuid_gen::ProjectNsGuard::install(&char_names[0]));

    // Per-SSF emission shape (docs/multi_character_projects_plan.md §2):
    //   * single-character SSF (or --per-character-projects): each char in its own
    //     characters/<id>/ project.
    //   * multi-character SSF in default mode: one characters/<project_id>/ project
    //     with all characters as peer entities.
    let multi_char_mode = char_names.len() > 1 && !opts.per_character_projects;
    let project_id: Option<String> = if multi_char_mode {
        swf.abc_blocks.iter()
            .filter_map(|b| abc_parser::parse(b).ok())
            .find_map(|abc| abc_parser::extract_main_package_metadata(&abc).and_then(|md| md.id))
            .or_else(|| opts.input.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string()))
    } else { None };

    // Pre-compute each character's PascalCase form once.
    let all_pascals: Vec<String> = {
        let md = swf.abc_blocks.iter()
            .filter_map(|b| abc_parser::parse(b).ok())
            .find_map(|abc| abc_parser::extract_main_package_metadata(&abc));
        char_names.iter().map(|id| {
            md.as_ref()
                .and_then(|md| md.characters.iter().find(|(d, _)| d == id))
                .map(|(_, method)| abc_parser::pascal_form(method))
                .unwrap_or_else(|| abc_parser::pascal_form(id))
        }).collect()
    };

    let mut accumulated_logs: Vec<serde_json::Value> = Vec::new();
    let mut accumulated_manifest_chars: Vec<ManifestCharEntry> = Vec::new();
    let mut all_warnings: Vec<String> = Vec::new();
    let project_dir: PathBuf = match (&multi_char_mode, &project_id) {
        (true, Some(pid)) => opts.output.join(pid),
        _ => opts.output.clone(),
    };

    for (slot_idx, char_name) in char_names.iter().enumerate() {
        log::info!("─── Processing: {} ───", char_name);
        let slot = if multi_char_mode {
            Some(MultiCharSlot {
                project_dir: project_dir.clone(),
                slot_idx,
                pascals: all_pascals.clone(),
                char_ids: char_names.clone(),
            })
        } else { None };
        match process_character(
            &parsed_swf, &swf, char_name, &opts.output, costumes_path.as_deref(),
            &opts.input, slot.as_ref(), &mut all_warnings,
        ) {
            Ok(Some(artifacts)) => {
                accumulated_manifest_chars.push(artifacts.manifest_entry);
                accumulated_logs.push(artifacts.log_block);
            }
            Ok(None) => { /* single-character; finalized inside process_character */ }
            Err(e) => {
                log::error!("Failed to process {}: {}", char_name, e);
                all_warnings.push(format!("failed to process {char_name}: {e}"));
            }
        }
    }

    // Multi-char: write the project-level manifest + .fraytools + log after all
    // characters are processed. (Single-char finalizes inside process_character.)
    if multi_char_mode {
        if let Err(e) = finalize_multi_char_project(
            &project_dir, project_id.as_deref().unwrap_or("project"),
            &accumulated_manifest_chars, &accumulated_logs, &opts.input,
        ) {
            log::error!("Failed to finalize multi-char project: {}", e);
            all_warnings.push(format!("failed to finalize multi-char project: {e}"));
        }
    }

    // Clean up temp costumes file if we created it
    if costumes_is_temp {
        if let Some(ref p) = costumes_path {
            if let Err(e) = fs::remove_file(p) {
                log::warn!("Failed to remove temp costumes file: {}", e);
            } else {
                log::info!("Removed temp costumes cache");
            }
        }
    }

    // #13: lint every emitted .hx for structural breakage (unbalanced delimiters,
    // dangling string, empty interpolation) so a generation bug surfaces in the
    // convert output as a SCRIPTERR line instead of silently dying in-engine. Scope
    // to only THIS run's output dirs (single-char output is the shared root, so
    // linting project_dir would re-flag stale unrelated characters).
    let lint_dirs: Vec<PathBuf> = if multi_char_mode {
        vec![project_dir.clone()]
    } else {
        char_names.iter().map(|c| opts.output.join(c)).collect()
    };
    for dir in &lint_dirs {
        match prune_unplaced_sprites(dir) {
            Ok(0) => {}
            Ok(n) => log::info!("dropped {n} sprite(s) nothing places"),
            Err(e) => all_warnings.push(format!("pruning sprites in {}: {e}", dir.display())),
        }
        for problem in lint_emitted_hscript(dir) {
            log::error!("{problem}");
            all_warnings.push(problem);
        }
        for problem in lint_duplicate_guids(dir) {
            log::error!("{problem}");
            all_warnings.push(problem);
        }
    }

    // Collect the .fraytools files produced (for the UI's "open / publish" step).
    let fraytools_files = collect_fraytools_files(&project_dir, multi_char_mode, &char_names, &opts.output);

    Ok(ConversionSummary {
        project_dir,
        characters: char_names,
        multi_char: multi_char_mode,
        project_id,
        fraytools_files,
        log_blocks: accumulated_logs,
        warnings: all_warnings,
    })
}

/// Drops the sprites nothing in the project uses: no entity places the picture, and no other file
/// of the project names it (by its guid or its id). Pieces a flattened picture draws are left
/// with no placement of their own, as are pictures drawn for animations no entity keeps; the
/// publish carries (and counts) every picture in the library. Each `verify.json` is then given
/// the art that is left.
fn prune_unplaced_sprites(dir: &Path) -> Result<usize> {
    fn files(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { files(&p, out) } else { out.push(p) }
        }
    }
    let mut all = Vec::new();
    files(dir, &mut all);
    let named = |p: &Path, ext: &str| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(ext));
    // what the project says about its pictures: the entities' placements, and anything else it
    // writes in text (a manifest, stats, a script)
    let mut text = String::new();
    for p in &all {
        if named(p, ".png") || named(p, ".png.meta") || named(p, ".wav") || named(p, ".ogg") || named(p, ".mp3") || named(p, ".fra") { continue }
        if let Ok(t) = fs::read_to_string(p) { text.push_str(&t); text.push('\n'); }
    }
    let mut metas = Vec::new();
    for meta in all.iter().filter(|p| named(p, ".png.meta")) {
        let Ok(v) = fs::read_to_string(meta).map(|t| serde_json::from_str::<serde_json::Value>(&t)) else { continue };
        let Ok(v) = v else { continue };
        let guid = v["guid"].as_str().unwrap_or("").to_string();
        let id = v["id"].as_str().filter(|i| !i.is_empty()).map(|i| format!("\"{i}\""));
        metas.push((meta, guid, id));
    }
    // every guid and quoted id the text names, found in one pass over it rather than one a picture
    let needles: std::collections::BTreeSet<&str> = metas.iter()
        .flat_map(|(_, g, i)| std::iter::once(g.as_str()).chain(i.as_deref()))
        .filter(|n| !n.is_empty()).collect();
    let mut seen = std::collections::HashSet::new();
    if !needles.is_empty() {
        let alt = needles.iter().map(|n| regex::escape(n)).collect::<Vec<_>>().join("|");
        let re = regex::RegexBuilder::new(&alt).size_limit(1 << 28).dfa_size_limit(1 << 28).build()?;
        let mut at = 0;
        // step one byte past each match's start, so a needle overlapping the one before is found too
        while let Some(m) = re.find_at(&text, at) {
            seen.insert(m.as_str());
            at = m.start() + text[m.start()..].chars().next().map_or(1, char::len_utf8);
        }
    }
    let mut dropped = 0usize;
    for (meta, guid, id) in metas {
        if guid.is_empty() || seen.contains(guid.as_str()) || id.as_deref().is_some_and(|i| seen.contains(i)) { continue }
        let png = meta.with_extension("");
        let _ = fs::remove_file(&png);
        fs::remove_file(meta)?;
        dropped += 1;
    }
    if dropped > 0 {
        for plan in all.iter().filter(|p| named(p, "verify.json")) {
            let Some(root) = plan.parent() else { continue };
            let Ok(mut v) = fs::read_to_string(plan).map(|t| serde_json::from_str::<serde_json::Value>(&t)) else { continue };
            let Ok(v) = v.as_mut() else { continue };
            if v.get("assets").is_none() { continue }
            let mut lib = Vec::new();
            files(&root.join("library"), &mut lib);
            let pngs: Vec<&PathBuf> = lib.iter().filter(|p| named(p, ".png")).collect();
            let bytes: u64 = pngs.iter().map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
            v["assets"] = serde_json::json!({ "images": pngs.len(), "bytes": bytes });
            fs::write(plan, serde_json::to_string_pretty(v)?)?;
        }
    }
    Ok(dropped)
}

/// Enumerate the `.fraytools` project files this run produced. Multi-char writes
/// one under `project_dir`; single-char writes one under each `output/<char>/`.
fn collect_fraytools_files(
    project_dir: &Path, multi_char: bool, char_names: &[String], output: &Path,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_first_in = |dir: &Path| {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "fraytools").unwrap_or(false) {
                    out.push(p);
                }
            }
        }
    };
    if multi_char {
        push_first_in(project_dir);
    } else {
        for c in char_names {
            push_first_in(&output.join(c));
        }
    }
    out
}

/// Two assets carrying one guid is a conflict FrayTools reports and the engine
/// resolves arbitrarily: whichever the project loads second wins, and the other's
/// references draw the wrong picture. Nothing downstream can tell them apart, so
/// catch it here where the two files are still visible side by side.
fn lint_duplicate_guids(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|x| x.to_str()) == Some("meta") {
                    out.push(p);
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);

    let mut seen: std::collections::BTreeMap<String, PathBuf> = Default::default();
    let mut problems = Vec::new();
    for f in files {
        let Ok(text) = fs::read_to_string(&f) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let Some(guid) = json.get("guid").and_then(|g| g.as_str()) else { continue };
        match seen.get(guid) {
            Some(first) => problems.push(format!(
                "two assets claim the same identity {guid}: {} and {} — \
                 FrayTools reports a guid conflict and one of them draws the other's picture",
                first.display(),
                f.display()
            )),
            None => { seen.insert(guid.to_string(), f); }
        }
    }
    problems
}

/// #13: walk every emitted `.hx` under `dir` and structurally lint it, returning
/// one `SCRIPTERR:` line per problem. The engine executes generated hscript with a
/// try/catch that, by default, swallows parse/runtime failures silently — a
/// generation bug then reads in-engine as "the move just doesn't do anything." This
/// catches the realistic generation failures (unbalanced delimiters, an unterminated
/// string, an empty `${}`) at convert time so they surface in the session instead.
fn lint_emitted_hscript(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().map(|x| x == "hx").unwrap_or(false) {
                    out.push(p);
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    let mut problems = Vec::new();
    for f in &files {
        let Ok(src) = fs::read_to_string(f) else { continue };
        let rel = f.strip_prefix(dir).unwrap_or(f).display();
        if let Some(msg) = hscript_structural_problem(&src) {
            problems.push(format!("SCRIPTERR: {rel}: {msg}"));
        }
    }
    problems
}

/// Lightweight hscript structural check: scan outside string literals and comments,
/// require balanced `(){}[]` and a terminated string, and reject empty `${}`
/// interpolation. Returns the first problem found, or None if the file looks sound.
/// NOT a full parser — it only catches the breakage our own emitter can produce.
fn hscript_structural_problem(src: &str) -> Option<String> {
    let b = src.as_bytes();
    let mut stack: Vec<(u8, usize)> = Vec::new(); // (closing char, byte offset of opener)
    let mut line = 1usize;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        // comments
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                if b[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        // string literals (" or '); ' supports ${...} interpolation
        if c == b'"' || c == b'\'' {
            let quote = c;
            let start_line = line;
            i += 1;
            let mut terminated = false;
            while i < b.len() {
                let d = b[i];
                if d == b'\\' {
                    i += 2;
                    continue;
                }
                if d == b'\n' {
                    line += 1;
                    i += 1;
                    continue;
                }
                if quote == b'\'' && d == b'$' && i + 1 < b.len() && b[i + 1] == b'{' {
                    if i + 2 < b.len() && b[i + 2] == b'}' {
                        return Some(format!("empty string interpolation `${{}}` near line {start_line}"));
                    }
                    // skip the interpolation braces so an inner } doesn't end scan early
                    let mut depth = 0i32;
                    i += 1; // at '{'
                    loop {
                        if i >= b.len() {
                            return Some(format!("unterminated `${{...}}` interpolation opened near line {start_line}"));
                        }
                        match b[i] {
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            b'\n' => line += 1,
                            _ => {}
                        }
                        i += 1;
                    }
                    continue;
                }
                if d == quote {
                    terminated = true;
                    i += 1;
                    break;
                }
                i += 1;
            }
            if !terminated {
                return Some(format!("unterminated string literal opened near line {start_line}"));
            }
            continue;
        }
        match c {
            b'(' => stack.push((b')', line)),
            b'{' => stack.push((b'}', line)),
            b'[' => stack.push((b']', line)),
            b')' | b'}' | b']' => match stack.pop() {
                Some((expected, _)) if expected == c => {}
                Some((expected, opened)) => {
                    return Some(format!(
                        "mismatched delimiter: expected `{}` but found `{}` at line {line} (opener at line {opened})",
                        expected as char, c as char
                    ));
                }
                None => {
                    return Some(format!("stray closing `{}` at line {line}", c as char));
                }
            },
            _ => {}
        }
        i += 1;
    }
    if let Some((expected, opened)) = stack.last() {
        return Some(format!(
            "unbalanced delimiter: `{}` opened at line {opened} never closed",
            match expected {
                b')' => '(',
                b'}' => '{',
                _ => '[',
            }
        ));
    }
    None
}

/// Extract all costume palettes from misc.ssf in-process and write to a temp JSON
/// file. Returns the path to the temp file on success.
/// The archive beside `dir`'s inputs that carries the game's costume table: `misc.ssf` when a
/// loose copy is laid out that way, otherwise whichever `.ssf` in the directory holds costume
/// methods (the installed game keeps it in a numbered DAT archive). Found by what the archive
/// holds, never by its number, and remembered per directory so a roster of conversions scans
/// once.
fn costume_source_near(dir: &Path) -> Option<PathBuf> {
    let named = dir.join("misc.ssf");
    if named.exists() { return Some(named) }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        dir.hash(&mut h);
        h.finish()
    };
    let memo = std::env::temp_dir().join(format!("peptide-costume-source-{key:016x}.txt"));
    if let Ok(p) = fs::read_to_string(&memo) {
        let p = PathBuf::from(p.trim());
        if p.exists() { return Some(p) }
    }
    let mut files: Vec<PathBuf> = fs::read_dir(dir).ok()?.flatten().map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("ssf")))
        .collect();
    files.sort();
    let found = files.into_iter().find(|p| {
        let Ok(raw) = fs::read(p) else { return false };
        let Ok(swf_data) = ssf::decompress(&raw) else { return false };
        let Ok(swf) = swf_parser::parse(&swf_data) else { return false };
        swf.abc_blocks.iter().filter_map(|b| abc_parser::parse(b).ok()).any(|abc| {
            abc_parser::scan_all_costume_methods(&abc).iter()
                .any(|(k, v)| k != "unknown" && v.len() >= 10)
        })
    })?;
    let _ = fs::write(&memo, found.display().to_string());
    Some(found)
}

fn extract_costumes_to_temp(misc_ssf: &Path) -> Result<PathBuf> {
    let raw = fs::read(misc_ssf).context("read misc.ssf")?;
    let swf_data = ssf::decompress(&raw).context("decompress misc.ssf")?;
    let swf = swf_parser::parse(&swf_data).context("parse misc.ssf")?;

    let mut all_costumes: std::collections::BTreeMap<String, Vec<abc_parser::CostumeData>> =
        std::collections::BTreeMap::new();

    for abc_bytes in &swf.abc_blocks {
        let abc = abc_parser::parse(abc_bytes).context("parse ABC")?;
        let found = abc_parser::scan_all_costume_methods(&abc);
        for (char_name, costumes) in found {
            all_costumes.entry(char_name).or_default().extend(costumes);
        }
    }

    // Drop noise: unknown key, or fewer than 10 costumes
    all_costumes.retain(|k, v| k != "unknown" && v.len() >= 10);

    let json_val: serde_json::Value = all_costumes.iter().map(|(char_name, costumes)| {
        let arr: serde_json::Value = costumes.iter().map(|c| serde_json::json!({
            "name":         c.name,
            "colors":       c.colors,
            "replacements": c.replacements,
        })).collect::<Vec<_>>().into();
        (char_name.clone(), arr)
    }).collect::<serde_json::Map<_, _>>().into();

    // a private file of its own: the game's data directory is not necessarily writable, and two
    // conversions running at once must not share one
    let temp_path = std::env::temp_dir().join(format!("peptide-costumes-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    fs::write(&temp_path, serde_json::to_string(&json_val)?).context("write costumes cache")?;

    log::info!("Extracted {} characters' costume data from {}", all_costumes.len(), misc_ssf.display());
    Ok(temp_path)
}

/// Derive the canonical character id from a `Main::get<X>()` method name: strip
/// `get`, lowercase the remainder, preserve explicit `_`.
fn derive_id_from_getter(method_name: &str) -> Option<String> {
    let stripped = method_name.strip_prefix("get")?;
    if stripped.is_empty() { return None; }
    Some(stripped.to_lowercase())
}

/// Detect all character names in a SWF (constructor walk, with a `Main::get*`
/// enumeration fallback). Empty for SWFs without a `Main` class.
/// True when a file stem looks like an SSF2 `DAT<n>` archive name (case-insensitive
/// `dat` followed by one or more digits), e.g. `DAT0`, `dat123`. Used to suppress
/// filename-based heuristics that assume the input was named after its character.
fn is_dat_archive_stem(stem: &str) -> bool {
    let lower = stem.to_lowercase();
    lower
        .strip_prefix("dat")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

fn detect_char_names(swf: &swf_parser::SwfFile, _input_path: &Path) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for abc_bytes in &swf.abc_blocks {
        let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };

        // Primary: constructor walk.
        if let Some(md) = abc_parser::extract_main_package_metadata(&abc) {
            if !md.characters.is_empty() {
                for (id, _method) in &md.characters {
                    if seen.insert(id.clone()) { names.push(id.clone()); }
                }
                continue;
            }
            log::warn!("constructor walk returned empty characters[] — falling back to instance-method enumeration");
        }

        // Fallback: enumerate Main's instance get* methods.
        let Some(main) = abc.classes.iter().find(|c| c.name == "Main") else { continue };
        for t in &main.instance_methods {
            if let Some(id) = derive_id_from_getter(&t.name) {
                if seen.insert(id.clone()) { names.push(id); }
            }
        }
    }

    names
}

#[allow(clippy::too_many_arguments)]
fn process_character(
    parsed_swf: &swf::Swf,
    swf: &swf_parser::SwfFile,
    char_name: &str,
    output: &Path,
    costumes: Option<&Path>,
    input_path: &Path,
    multi_char_slot: Option<&MultiCharSlot>,
    warnings_out: &mut Vec<String>,
) -> Result<Option<ProcessedCharacter>> {
    // Fresh conversion log for this character.
    crate::api_mappings::reset_conversion_log();

    // Extract character data (ABC: attacks, stats, frame scripts, xframe map)
    // What the main timeline says each sub-MC is, so a clip whose label is not in the mapping
    // file still gets named -- and keeps its frame scripts.
    let class_anim = sprite_parser::class_to_ssf2_anim(parsed_swf, &swf.symbols, &char_name.to_lowercase());
    if std::env::var("PEPTIDE_DUMP_CLASS_ANIM").is_ok() {
        for (c, a) in &class_anim { eprintln!("[class-anim] {c} -> {a}"); }
    }
    // the clips a projectile carries are the projectile's, whatever the fighter's script sweep
    // would make of their names
    let foreign_clips = image_extractor::projectile_clip_classes(parsed_swf);
    // the clips this fighter's own timeline places, at any depth: the ones whose methods are the
    // fighter's. read off the display tree, since in a package two fighters share every clip
    // class carries the package's name and not either fighter's
    let own_clips: std::collections::BTreeSet<String> =
        sprite_parser::build_xframe_sprite_map(parsed_swf, &swf.symbols, &char_name.to_lowercase())
            .keys().filter_map(|id| swf.symbols.get(id).cloned()).collect();
    // the numbers the source's api gives its character states, for the scripts that compare
    // against them: read off the state class the file itself ships
    {
        let mut states: std::collections::BTreeMap<i64, String> = Default::default();
        for abc in crate::projectile_gen::parse_abcs(parsed_swf) {
            for (name, v) in abc_parser::class_const_values(&abc, "CState") {
                if let Ok(n) = v.parse::<i64>() { states.entry(n).or_insert(name); }
            }
        }
        crate::api_mappings::set_source_state_names(states);
    }
    let mut char_data = extractor::extract(swf, char_name, &class_anim, &foreign_clips, &own_clips)?;
    log::info!("Extracted: {} attacks, {} animations, {} ssf2→fm mappings",
        char_data.attacks.len(), char_data.animations.len(), char_data.ssf2_to_fm_anim.len());

    // Lift Main's package metadata once per character.
    let package_metadata = swf.abc_blocks.iter()
        .filter_map(|b| abc_parser::parse(b).ok())
        .find_map(|abc| abc_parser::extract_main_package_metadata(&abc));
    let validation = run_tier1_validation(char_name, &char_data, package_metadata.as_ref(), input_path);
    warnings_out.extend(validation.iter().cloned());

    // PascalCase form for entity filenames + scripts subdir.
    let char_pascal: String = package_metadata.as_ref()
        .and_then(|md| md.characters.iter().find(|(id, _)| id == char_name))
        .map(|(_, method)| abc_parser::pascal_form(method))
        .unwrap_or_else(|| abc_parser::pascal_form(char_name));

    // The engine's base scale is the size multiplier alone. The scale the root clip places each
    // animation at is already in the art and the boxes (both are read in the root's space, its
    // transform applied), so taking it again here drew and hit-tested a fighter placed at 1.4
    // (bandana dee) 1.4 times too large: its kick reached 119 source px where the source's reaches 85.
    let (xframe_x, xframe_y) = sprite_parser::extract_xframe_scale_from_swf(parsed_swf, char_name)
        .unwrap_or((1.0, 1.0));
    let size_mult = mappings::character_stats().scaling.size_multiplier;
    char_data.stats.base_scale_x = size_mult;
    char_data.stats.base_scale_y = size_mult;
    log::info!("Character base scale: {:.4} (root placement {:.4}/{:.4}, already in the art)",
        size_mult, xframe_x, xframe_y);

    // Root MC transforms — computed once and shared.
    let xform_map = sprite_parser::extract_xframe_transforms_from_swf(
        parsed_swf, char_name, &char_data.ssf2_to_fm_anim,
    ).unwrap_or_default();
    if std::env::var("PEPTIDE_DUMP_XFORM").is_ok() {
        for (a, t) in &xform_map { eprintln!("[xform] {a} tx={:.2} ty={:.2} sx={:.3} sy={:.3}", t.tx, t.ty, t.sx, t.sy); }
    }

    // the frames whose scripts end a move, per animation: what the jab splitter reads to tell a
    // clip's unlabelled first hit from a set-up frame (installed before the boxes and the images
    // are cut, so both cuts agree)
    {
        let mut ends: std::collections::BTreeMap<String, std::collections::BTreeSet<u16>> = Default::default();
        for sc in &char_data.scripts {
            if sc.is_ext_method || !sc.code.contains("endAttack") { continue; }
            let Some((ssf2, n)) = sc.name.rsplit_once("__frame") else { continue };
            let (Some(fm), Ok(n)) = (char_data.ssf2_to_fm_anim.get(ssf2), n.parse::<u16>()) else { continue };
            ends.entry(fm.clone()).or_default().insert(n);
        }
        sprite_parser::install_ending_frames(ends);
        // what the jab clip's frame scripts say, frame by frame: the cut is read off them
        if std::env::var("PEPTIDE_TRACE_JAB").is_ok() {
            for sc in &char_data.scripts {
                let Some((ssf2, n)) = sc.name.rsplit_once("__frame") else { continue };
                if char_data.ssf2_to_fm_anim.get(ssf2).map(String::as_str) != Some("jab") { continue; }
                eprintln!("[trace-jab-script] frame {n}: ends={} code={}", sc.code.contains("endAttack"), sc.code.replace('\n', " ").chars().take(160).collect::<String>());
            }
        }
    }
    // Per-frame collision box geometry.
    let sprite_boxes = sprite_parser::parse_sprite_boxes_from_swf(
        parsed_swf, char_name, &char_data.ssf2_to_fm_anim, &xform_map,
    ).unwrap_or_else(|e| {
        log::warn!("sprite_parser failed: {}", e);
        Default::default()
    });
    log::info!("Sprite boxes: {} animations with geometry", sprite_boxes.len());
    // the boxes as extracted, for tools/tests/box_runtime.py to hold against what the source's
    // engine places at run time: per animation its source label, where it starts in its clip,
    // each frame's boxes by instance name, and the class of the clip each label plays
    if let Ok(path) = std::env::var("PEPTIDE_DUMP_BOXES") {
        let labels = sprite_parser::build_xframe_sprite_map_with(parsed_swf, &swf.symbols, &char_name.to_lowercase(), false);
        let classes: std::collections::BTreeMap<String, String> = labels.iter()
            .filter_map(|(id, label)| swf.symbols.get(id).map(|c| (c.rsplit('.').next().unwrap_or(c).to_string(), label.clone())))
            .collect();
        let anims: Vec<serde_json::Value> = sprite_boxes.iter().map(|(fm, b)| serde_json::json!({
            "fm": fm, "label": b.ssf2_name, "offset": b.sprite_frame_offset, "total": b.total_frames,
            "frames": b.frames.iter().map(|(f, boxes)| (f.to_string(), boxes.iter()
                .map(|x| serde_json::json!([x.instance_name, x.x, x.y, x.width, x.height])).collect::<Vec<_>>()))
                .collect::<std::collections::BTreeMap<_, _>>(),
        })).collect();
        let _ = std::fs::write(&path, serde_json::to_string(&serde_json::json!({ "classes": classes, "animations": anims })).unwrap_or_default());
    }

    let char_output_dir = match multi_char_slot {
        Some(s) => s.project_dir.clone(),
        None => output.join(char_name),
    };
    // Clean the generated `library/` subtree before (re)writing, so a file that a
    // prior conversion produced but this one no longer does (e.g. an old
    // palette_preview.png after a rename) doesn't orphan — orphans can even collide
    // on deterministic GUIDs. Multi-char characters SHARE one project library/, so
    // only the FIRST slot cleans it; later slots add into it.
    let is_first_slot = multi_char_slot.map(|s| s.slot_idx == 0).unwrap_or(true);
    if is_first_slot {
        let lib = char_output_dir.join("library");
        if lib.exists() {
            if let Err(e) = std::fs::remove_dir_all(&lib) {
                log::warn!("could not clean stale library dir {}: {}", lib.display(), e);
            }
        }
    }
    // THE NAMED CHILDREN THE FIGHTER'S CODE ADDRESSES. Each animation's clip places children
    // under names, and the fighter's code reaches them as fields (`self.shieldframe.alpha`,
    // `self.waftloc.x`). Each such child is a part riding the fighter: its frames an animation
    // on the character entity, drawn by a vfx that follows the fighter, and a record the code
    // talks to; the placement is per animation, since each animation's clip places it where it
    // does, and where an animation places none the part is not shown.
    let fighter_part_children: Vec<(String, String, image_extractor::NamedChild)> = {
        let code: String = char_data.scripts.iter().map(|sc| sc.code.as_str()).collect::<Vec<_>>().join("\n");
        let roots = sprite_parser::build_xframe_sprite_map_with(parsed_swf, &swf.symbols, &char_name.to_lowercase(), false);
        if std::env::var("PEPTIDE_DUMP_PARTS").is_ok() {
            let deep = sprite_parser::build_xframe_sprite_map_with(parsed_swf, &swf.symbols, &char_name.to_lowercase(), true);
            eprintln!("[fparts] roots {} (deep {}): {:?}", roots.len(), deep.len(), roots.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>());
            for (sid, label) in &roots {
                let kids = image_extractor::named_descendants_of_sprite(parsed_swf, *sid);
                if !kids.is_empty() { eprintln!("[fparts] {sid} ({label}, fm={:?}) names {:?}", char_data.ssf2_to_fm_anim.get(label), kids.iter().map(|c| c.name.clone()).collect::<Vec<_>>()); }
            }
        }
        let mut out: Vec<(String, String, image_extractor::NamedChild)> = Vec::new();
        // a name the script already owns (a slot of its class, a method) is that, not a child:
        // dedede's clip places a `gordo` where the held gordo sits, and `gordo` is the slot the
        // fired one lives in
        let taken: std::collections::BTreeSet<String> = char_data.ext_vars.iter().cloned()
            .chain(char_data.scripts.iter().map(|sc| sc.name.clone()))
            .chain(char_data.getters.iter().cloned())
            .collect();
        // ...unless nothing ever PUTS anything in it. a slot the source's own code fills is that
        // slot (dedede's `gordo` holds the one it fired), and one it only ever reads is the child
        // the clip places under that name, which the source's engine binds for it (ryu's
        // `purpletrail` is the trail on his kick, shown for one costume and hidden for the other)
        let filled = |name: &str| {
            // the decompile writes the fighter itself as `self.self.`, so any run of them counts.
            // set to nothing is not filled (a clip's declared field is set back to its default
            // where its animation starts, and nulling a slot puts nothing in it)
            let re = crate::dyn_re(&format!(r"(?m)(^|[^.\w])(?:self\.)*{}\s*=([^=][^;\n]*)", regex::escape(name)));
            char_data.scripts.iter().any(|sc| re.captures_iter(&sc.code)
                .any(|c| !matches!(c[2].trim(), "null" | "undefined")))
        };
        for (sid, label) in &roots {
            let Some(fm) = char_data.ssf2_to_fm_anim.get(label).cloned() else { continue };
            for child in image_extractor::named_descendants_of_sprite(parsed_swf, *sid) {
                if taken.contains(&child.name) && filled(&child.name) { continue }
                // reached as the fighter's own field, or as the stance clip's (`getStanceMC().waftloc`)
                let re = crate::dyn_re(&format!(r"(self\.|getStanceMC\(\)\.){}\b", regex::escape(&child.name)));
                if !re.is_match(&code) { continue }
                if out.iter().any(|(n, a, _)| *n == child.name && *a == fm) { continue }
                out.push((child.name.clone(), fm.clone(), child));
            }
        }
        out
    };
    let fighter_part_names: Vec<String> = {
        let mut v: Vec<String> = fighter_part_children.iter().map(|(n, _, _)| n.clone()).collect();
        v.sort(); v.dedup(); v
    };
    let mut img_result = image_extractor::extract_images_from_swf_skipping(
        parsed_swf, &char_output_dir, char_name, &char_data.ssf2_to_fm_anim, &xform_map, &fighter_part_names,
    ).unwrap_or_else(|e| {
        log::warn!("image_extractor failed: {}", e);
        image_extractor::ImageExtractionResult {
            images: Default::default(),
            shape_to_bitmap: Default::default(),
            shape_pivot: Default::default(),
            shape_fill_scale: Default::default(),
            anim_images: Default::default(),
        }
    });
    log::info!("Extracted {} sprite images, {} anim image maps",
        img_result.images.len(), img_result.anim_images.len());

    // Sounds: flat library/audio/*.wav (single) or library/audio/<char>/*.wav (multi).
    let sounds_dir = match multi_char_slot {
        Some(_) => char_output_dir.join(format!("library/audio/{}", char_name)),
        None    => char_output_dir.join("library/audio"),
    };
    let mut sounds = match sound_extractor::extract_all_sounds(parsed_swf, &sounds_dir, char_name) {
        Ok(s) => s,
        Err(e) => { log::warn!("sound_extractor failed: {}", e); vec![] }
    };
    // A SOUND THE CHARACTER ASKS FOR THAT ITS OWN FILE DOES NOT CARRY. The game keeps shared
    // packages loaded and a sound resolves out of them exactly as a class does, which is how a
    // fighter can name a clip it never ships: mario asks for `jump_common`, his two landing
    // sounds and the six his screw attack plays, and carried none of them, so each of those
    // plays came out silent.
    {
        let have: std::collections::BTreeSet<&str> = sounds.iter().map(|s| s.name.as_str()).collect();
        let mut want: std::collections::BTreeSet<String> = char_data.scripts.iter()
            .flat_map(|sc| sound_extractor::sound_ids_named_in(&sc.code))
            .filter(|id| !have.contains(id.as_str()))
            .collect();
        if !want.is_empty() {
            let asked = want.len();
            let extra = sound_extractor::pull_shared_sounds(&mut want, &sounds_dir, char_name);
            log::info!("{} of {asked} sounds the character names came from a shared package",
                extra.len());
            if !want.is_empty() {
                log::warn!("sounds {char_name} asks for that no package carries: {}",
                    want.iter().cloned().collect::<Vec<_>>().join(", "));
            }
            sounds.extend(extra);
        }
    }
    let sounds = sounds;

    // Discover projectiles, effects, head sprite.
    let (mut projectiles, effects, head_sprite) = image_extractor::discover_projectiles_and_head_from_swf(
        parsed_swf, char_name, char_data.thumbnail.as_deref(),
    ).unwrap_or_else(|e| {
        log::warn!("discover_projectiles_and_head failed: {}", e);
        (vec![], vec![], None)
    });
    // A THING SPAWNED BY ITS CLASS (`spawnEnemy(PacManHydrant)`). The class declares what it
    // is: its clip (`linkage_id`), its numbers and its flags in `getOwnStats`, and its methods
    // are the object's own. Entered in the projectile table under the class's name, it is
    // spawned, drawn and run the way every other projectile of this character is.
    let mut class_fired: Vec<String> = Vec::new();
    {
        let abcs = crate::projectile_gen::parse_abcs(parsed_swf);
        let by_class: Vec<String> = char_data.scripts.iter()
            .flat_map(|sc| crate::api_mappings::projectile_names_fired_in(&sc.code))
            .filter(|n| !char_data.projectile_data.contains_key(n))
            .filter(|n| abcs.iter().any(|abc| abc_parser::class_named(abc, n).is_some()))
            .collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        for class in by_class {
            let Some(abc) = abcs.iter().find(|abc| abc_parser::class_named(abc, &class).is_some()) else { continue };
            // ...and what the class's own code fires (the hydrant's water shots), so the name is
            // resolved to its clip the way one the fighter fires is
            for (_, code) in abc_parser::extract_class_methods(abc, &class) {
                class_fired.extend(crate::api_mappings::projectile_names_fired_in(&code));
            }
            let Some(link) = abc_parser::extract_own_linkage_for(abc, &class) else {
                log::info!("'{class}' is spawned by class but declares no clip of its own; not carried");
                continue;
            };
            let mut names: std::collections::BTreeMap<String, String> = Default::default();
            names.insert("linkage_id".into(), link.clone());
            names.insert("classAPI".into(), class.clone());
            let data = abc_parser::ProjectileData {
                stats: abc_parser::extract_own_stats_for(abc, &class),
                flags: abc_parser::extract_own_flags_for(abc, &class),
                names,
                hitboxes: Vec::new(),
            };
            log::info!("'{class}' is spawned by class: its declaration says it draws with '{link}'");
            char_data.projectile_data.insert(class.clone(), data);
            // and what the class declares it fires (its own `getProjectileStats`): those are in
            // the table too, so the shots it makes are found the way the character's own are
            for (name, decl) in abc_parser::extract_projectiles_for(abc, &class) {
                char_data.projectile_data.entry(name).or_insert(decl);
            }
            for mut p in image_extractor::discover_projectiles_by_linkage(parsed_swf, std::slice::from_ref(&link)) {
                if projectiles.iter().any(|q| crate::projectile_gen::projectile_content_id(&q.name) == crate::projectile_gen::projectile_content_id(&class)) { continue }
                p.name = class.clone();
                projectiles.push(p);
            }
        }
    }

    // WHICH CLIP A FIRED NAME DRAWS WITH, said by the source rather than guessed from it. A
    // package's projectile table gives every name it fires a `linkage_id`, which is the clip.
    // fox fires `laser` and ships `lazor`; its table says so. and three of its throws fire
    // different names that all draw with `UThrowLaser`, so one clip answers to several names
    // with its own stats behind each -- which a rule about how names LOOK could never reach.
    {
        let fired: Vec<String> = char_data.scripts.iter()
            .flat_map(|sc| crate::api_mappings::projectile_names_fired_in(&sc.code))
            .chain(class_fired.iter().cloned())
            .collect();
        let folded = |s: &str| crate::projectile_gen::projectile_content_id(s);
        for name in fired {
            let want = folded(&name);
            if projectiles.iter().any(|p| folded(&p.name) == want) { continue }
            // the table is keyed as the source spells it, and a spawn need not spell it the same
            let Some(link) = char_data.projectile_data.iter()
                .find(|(k, _)| folded(k) == want)
                .and_then(|(_, d)| d.names.get("linkage_id"))
            else { continue };
            let link_id = folded(link);
            let Some(src) = projectiles.iter().find(|p| folded(&p.name) == link_id).cloned()
            else { continue };
            log::info!("'{}' is what this character fires as '{}', its own table says so", link, name);
            let mut copy = src;
            copy.name = name;
            projectiles.push(copy);
        }
    }

    // A PROJECTILE THE CHARACTER SPAWNS THAT ITS OWN FILE DOES NOT CARRY, the same way the
    // sounds above work. what its code names is what the source says exists; where the clip
    // lives is the game's business, not the fighter's.
    {
        let have: std::collections::BTreeSet<String> = projectiles.iter()
            .map(|p| crate::projectile_gen::projectile_content_id(&p.name)).collect();
        // read from the SOURCE's own spawn calls: at this point the scripts are still SSF2's,
        // so the name is the one it fires by, and the id is derived the same way a discovered
        // clip's is.
        let mut want: std::collections::BTreeSet<String> = char_data.scripts.iter()
            .flat_map(|sc| crate::api_mappings::projectile_names_fired_in(&sc.code))
            .map(|n| crate::projectile_gen::projectile_content_id(&n))
            .filter(|id| !have.contains(id))
            .collect();
        if !want.is_empty() {
            let asked = want.len();
            let extra = image_extractor::pull_shared_projectiles(&mut want, input_path.parent());
            log::info!("{} of {asked} projectile(s) the character spawns came from a shared package",
                extra.len());
            if !want.is_empty() {
                log::warn!("projectiles {char_name} spawns that no package carries: {}",
                    want.iter().cloned().collect::<Vec<_>>().join(", "));
            }
            projectiles.extend(extra);
        }
    }

    // A PROJECTILE THAT SPAWNS ANOTHER: a spring places a during-clip that hands off to the
    // persistent spring and then kills itself, so the spring the player sees is named only by a
    // projectile, never by the character. Discovery above reads the CHARACTER's spawn calls, so
    // that second clip is missed and never emitted (the handoff spawns nothing and the spring
    // vanishes). Follow the projectiles' OWN spawn calls too, to a fixpoint: discover each by the
    // linkage the source names, and pull it from a shared package when this file does not carry it.
    {
        let abcs = crate::projectile_gen::parse_abcs(parsed_swf);
        let cid = crate::projectile_gen::projectile_content_id;
        loop {
            let have: std::collections::BTreeSet<String> =
                projectiles.iter().map(|p| cid(&p.name)).collect();
            let nested: std::collections::BTreeSet<String> = projectiles.iter()
                .flat_map(|p| crate::projectile_gen::all_behaviour(&abcs, p).into_values().collect::<Vec<_>>())
                .flat_map(|code| crate::api_mappings::projectile_names_fired_in(&code))
                .filter(|n| !have.contains(&cid(n)))
                .collect();
            if nested.is_empty() { break }
            let names: Vec<String> = nested.iter().cloned().collect();
            let mut added = false;
            // what a projectile fires is named in the same table the character's own spawns are:
            // the file's `linkage_id` says which clip draws it (sonic's spring hands off to
            // `sonic_spring_ground`, which the table draws with `sonic_spring1`), and that clip
            // is found by that linkage, in this file, before any package beside it is asked
            for name in &names {
                let want = cid(name);
                if projectiles.iter().any(|p| cid(&p.name) == want) { continue }
                let Some(link) = char_data.projectile_data.iter()
                    .find(|(k, _)| cid(k) == want)
                    .and_then(|(_, d)| d.names.get("linkage_id")).cloned()
                else { continue };
                let src = projectiles.iter().find(|p| cid(&p.name) == cid(&link)).cloned()
                    .or_else(|| image_extractor::discover_projectiles_by_linkage(parsed_swf, std::slice::from_ref(&link)).into_iter().next());
                let Some(src) = src else { continue };
                log::info!("'{}' is what a projectile fires as '{}', the file's own table says so", link, name);
                let mut copy = src;
                copy.name = name.clone();
                projectiles.push(copy);
                added = true;
            }
            for p in image_extractor::discover_projectiles_by_linkage(parsed_swf, &names) {
                if !projectiles.iter().any(|q| cid(&q.name) == cid(&p.name)) {
                    log::info!("projectile '{}' is spawned by another projectile, emitting it too", p.name);
                    projectiles.push(p);
                    added = true;
                }
            }
            // whatever the SWF did not carry, try the shared packages the same way character spawns do
            let mut want: std::collections::BTreeSet<String> = names.iter().map(|n| cid(n))
                .filter(|id| !projectiles.iter().any(|q| cid(&q.name) == *id))
                .collect();
            if !want.is_empty() {
                let extra = image_extractor::pull_shared_projectiles(&mut want, input_path.parent());
                if !extra.is_empty() { added = true; projectiles.extend(extra); }
                if !want.is_empty() {
                    log::warn!("projectile(s) a projectile spawns that no package carries: {}",
                        want.iter().cloned().collect::<Vec<_>>().join(", "));
                }
            }
            if !added { break }  // nothing new could be found; stop rather than spin
        }
    }

    // In a multi-char .ssf the SWF prefixes a sub-character's projectiles with its id
    // (e.g. sheik's "sheik_uspecDamage"), but the fireProjectile spawn call uses the
    // LOGICAL name ("uspecDamage"). Strip a leading "<subchar>_" so the projectile's
    // content id matches the spawn (else the createProjectile dangles). Single-char is
    // a no-op (its own name isn't prefixed).
    let strip_prefixes: Vec<String> = multi_char_slot
        .map(|s| s.char_ids.iter().map(|c| format!("{}_", c)).collect())
        .unwrap_or_default();
    if !strip_prefixes.is_empty() {
        for p in &mut projectiles {
            for pfx in &strip_prefixes {
                if let Some(rest) = p.name.strip_prefix(pfx.as_str()) {
                    p.name = rest.to_string();
                    break;
                }
            }
        }
    }
    // A head clip is a little scene: pieces stacked at depths, the character art scaled down
    // inside it, and a mask cropping the result to a window. Composite it into the ONE portrait
    // the menu entity uses everywhere -- a second image in that sheet is one the packer can
    // silently drop.
    let mut head_sprite = head_sprite;
    {
        if let Some(head) = head_sprite.as_mut() {
            match image_extractor::composite_head(parsed_swf, head.sprite_id, &char_output_dir, char_name, true) {
                Ok(Some(img)) => {
                    log::info!("Composited head '{}' from its own placements ({}x{})",
                        head.name, img.width, img.height);
                    head.image_symbol = Some(img.symbol_name.clone());
                    head.image_shape_id = Some(img.bitmap_id);
                    img_result.images.insert(img.bitmap_id, img);
                }
                Ok(None) => {}
                Err(e) => log::warn!("compositing head '{}' failed: {}", head.name, e),
            }
        }
    }

    log::info!("Discovered {} projectiles, {} effects, head={}",
        projectiles.len(), effects.len(),
        head_sprite.as_ref().map(|h| h.name.as_str()).unwrap_or("none"));

    // the fighter's parts: one animation per distinct child clip, the placements per animation
    if !fighter_part_children.is_empty() {
        let mut parts: Vec<crate::entity_gen::ProjectilePartData> = Vec::new();
        let mut placements: Vec<crate::entity_gen::PartPlacement> = Vec::new();
        for (name, fm, child) in &fighter_part_children {
            let art = match parts.iter().find(|p| p.path == *name && p.frame_count > 0 && p.image_guids.keys().next().is_some() && p.anim.ends_with(&format!("_{}", child.sprite_id))) {
                Some(p) => p.anim.clone(),
                None => {
                    let anim = format!("part_{}_{}", name, child.sprite_id);
                    match image_extractor::extract_projectile_frame_images_from_swf(parsed_swf, char_name, child.sprite_id, &img_result) {
                        Ok(pfi) => {
                            let labels: Vec<(String, u16)> = image_extractor::sprite_tags(parsed_swf, child.sprite_id)
                                .map(sprite_parser::extract_frame_labels_from_tags).unwrap_or_default();
                            parts.push(crate::entity_gen::ProjectilePartData {
                                path: name.clone(), anim: anim.clone(), matrix: child.matrix,
                                frame_count: pfi.frames.len() as u16,
                                image_frames: pfi.frames, image_matrices: pfi.matrices, image_parts: pfi.parts, image_guids: pfi.image_guids,
                                labels,
                            });
                            anim
                        }
                        Err(e) => { log::warn!("part '{name}': image extraction failed: {e}"); continue }
                    }
                }
            };
            // a child with no picture of its own has no art to show; its record still runs
            let art = if parts.iter().any(|p| p.anim == art && p.image_guids.is_empty()) { String::new() } else { art };
            placements.push(crate::entity_gen::PartPlacement { name: name.clone(), fm_anim: fm.clone(), art, matrix: child.matrix });
        }
        // one record per name: the first clip found carries the record, the rest are art it can switch to
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        let records: Vec<crate::entity_gen::ProjectilePartData> = parts.iter().filter(|p| seen.insert(p.path.clone())).cloned().collect();
        for sc in char_data.scripts.iter_mut() {
            // the stance clip's child is the fighter's part
            for r in &records {
                sc.code = sc.code.replace(&format!("self.getStanceMC().{}", r.path), &format!("self.{}", r.path))
                    .replace(&format!("getStanceMC().{}", r.path), &format!("self.{}", r.path));
            }
            crate::projectile_gen::rewrite_part_refs(&records, &mut sc.code);
        }
        char_data.parts = parts;
        char_data.part_placements = placements;
        log::info!("fighter parts: {} ({} placements)", fighter_part_names.join(", "), char_data.part_placements.len());
    }

    // the platforms the fighter gives itself (fox's landmaster carries two): the same port a
    // projectile's get, a structure content riding the fighter
    {
        let prefix = char_name.to_lowercase().replace(' ', "");
        let mut plats: Vec<(f64, f64, f64, f64)> = Vec::new();
        for sc in char_data.scripts.iter_mut() {
            crate::projectile_gen::rewrite_self_platform_calls(&mut sc.code, &prefix, &mut plats);
        }
        if !plats.is_empty() { log::info!("fighter platforms: {}", plats.len()); }
        char_data.platforms = plats;
    }

    // the items the fighter's code generates: pulled with no physics of their own, they fall
    // as the source's engine drops a thrown item
    {
        // by the names the code writes, or the ones it lists when it picks at run time
        let ids: Vec<String> = char_data.scripts.iter()
            .filter(|sc| sc.code.contains("generateItem(") || sc.code.contains("generateCharacterItem("))
            .flat_map(|sc| crate::api_mappings::projectile_names_fired_in(&sc.code))
            .map(|n| crate::projectile_gen::projectile_content_id(&n))
            .collect();
        crate::api_mappings::set_generated_items(ids);
    }

    // Generate Fraymakers files: art, boxes, stats and the entity, then the fighter's script from
    // the runtime (see docs/CONVERTER_DESIGN.md)
    haxe_gen::generate(output, char_name, &char_pascal, &char_data, &sprite_boxes, &img_result,
        costumes, &sounds, &projectiles, &effects, head_sprite.as_ref(), parsed_swf,
        multi_char_slot, input_path)?;
    // a package whose fighter the runtime cannot read (one that declares none) has no script to
    // write: the rest of the project is still written, and its log says why the script is missing
    let mut validation = validation;
    match crate::emit::runtime_fighter(&char_output_dir, char_name, &char_pascal, &char_data, &sprite_boxes, input_path) {
        Ok(out) => log::info!("runtime path: {} problems, {} names nothing defines", out.problems.len(), out.unknown.len()),
        Err(e) => {
            log::error!("{char_name}: no fighter script: {e}");
            validation.push(format!("no fighter script: {e}"));
            warnings_out.push(format!("{char_name}: no fighter script: {e}"));
        }
    }
    log::info!("Generated Fraymakers files for {}", char_name);

    let projectile_names: Vec<String> = projectiles.iter().map(|p| p.name.clone()).collect();
    let menu_entity_id = match multi_char_slot {
        Some(_) => format!("{}_menu", char_name),
        None    => "menu".to_string(),
    };

    // Ask what the engine would ask: does every content id these scripts name actually exist?
    // Asked LAST, with everything written: asked any earlier it reads a half-built package next
    // to whatever the run before left behind, and answers about neither.
    crate::content_audit::audit_and_log(&char_output_dir.join("library"), char_name);

    if multi_char_slot.is_none() {
        // Single-char path: inline conversion-log write.
        write_conversion_log(&char_output_dir, char_name, &char_data,
            package_metadata.as_ref(), &validation)?;
        return Ok(None);
    }

    // Multi-char path: return artifacts for the finalizer.
    let log_block = build_conversion_log_block(char_name, &char_data,
        package_metadata.as_ref(), &validation);
    Ok(Some(ProcessedCharacter {
        manifest_entry: ManifestCharEntry {
            char_id:          char_name.to_string(),
            display_name:     char_pascal.clone(),
            projectile_names,
            menu_entity_id,
        },
        log_block,
    }))
}

/// Assemble the per-character `ssf2_source` + `validation_warnings` payload used
/// inside the multi-char project log's `characters: [...]` array.
fn build_conversion_log_block(
    char_name: &str,
    char_data: &extractor::CharacterData,
    md: Option<&abc_parser::MainPackageMetadata>,
    validation_warnings: &[String],
) -> serde_json::Value {
    let snap = crate::api_mappings::snapshot_conversion_log();
    let to_entries = |m: std::collections::BTreeMap<String, usize>| -> Vec<serde_json::Value> {
        let mut v: Vec<(String, usize)> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.into_iter()
            .map(|(name, count)| serde_json::json!({ "name": name, "count": count }))
            .collect()
    };
    let mut payload = serde_json::json!({
        "id": char_name,
        "unknown":   to_entries(snap.unknown),
        "ssf2_only": to_entries(snap.ssf2_only),
    });
    if let Some(md) = md {
        let source_method = md.characters.iter()
            .find(|(id, _)| id == char_name)
            .map(|(_, m)| format!("Main::{}", m));
        let mut ssf2_source = serde_json::Map::new();
        if let Some(id) = &md.id   { ssf2_source.insert("package_id".into(),   serde_json::json!(id)); }
        if let Some(g)  = &md.guid { ssf2_source.insert("package_guid".into(), serde_json::json!(g));  }
        if let Some(sm) = source_method {
            ssf2_source.insert("source_method".into(), serde_json::json!(sm));
        }
        if let Some(df) = &char_data.derived_from {
            ssf2_source.insert("parent_normal_stats_id".into(),
                serde_json::json!(df.parent_normal_stats_id));
            ssf2_source.insert("source_method".into(), serde_json::json!(df.source_method));
            ssf2_source.insert("note".into(), serde_json::json!(
                "Fraymakers has no native transformation API. Both \
                 characters ship as peer entities in the same project so \
                 that, when the API lands, the parent's Script.hx can \
                 swap between them at runtime."
            ));
        }
        payload.as_object_mut().unwrap()
            .insert("ssf2_source".to_string(), serde_json::Value::Object(ssf2_source));
    }
    if !validation_warnings.is_empty() {
        payload.as_object_mut().unwrap().insert(
            "validation_warnings".to_string(),
            serde_json::json!(validation_warnings),
        );
    }
    payload
}

/// Write the multi-char project's `<project>.fraytools` + merged
/// `library/manifest.json` + project-level `conversion_log.json`.
fn finalize_multi_char_project(
    project_dir: &Path,
    project_id: &str,
    chars: &[ManifestCharEntry],
    char_logs: &[serde_json::Value],
    input_path: &Path,
) -> Result<()> {
    log::info!("Finalising multi-char project: {} ({} characters)", project_id, chars.len());

    fs::write(project_dir.join(format!("{}.fraytools", project_id)),
        crate::fraytools_project::generate_fraytools_project(project_id))?;

    let manifest = haxe_gen::generate_multi_char_manifest(project_id, chars);
    fs::write(project_dir.join("library/manifest.json"), &manifest)?;
    fs::write(project_dir.join("library/manifest.json.meta"),
        haxe_gen::generate_manifest_meta_pub(&crate::uuid_gen::det_uuid(
            &format!("{}::manifest::meta", project_id))))?;

    let payload = serde_json::json!({
        "project":      project_id,
        "input":        input_path.file_name().and_then(|s| s.to_str()),
        "characters":   char_logs,
    });
    fs::write(project_dir.join("conversion_log.json"),
        serde_json::to_string_pretty(&payload)? + "\n")?;
    Ok(())
}

/// Tier-1 validation warnings — soft logs only, never hard-fail.
fn run_tier1_validation(
    char_name: &str,
    char_data: &extractor::CharacterData,
    md: Option<&abc_parser::MainPackageMetadata>,
    input_path: &Path,
) -> Vec<String> {
    let mut warnings = Vec::new();

    if char_data.attacks.is_empty() {
        warnings.push("attacks map is empty — extractor produced no attack data".to_string());
    }
    let s = &char_data.stats;
    let stats_look_empty = s.weight == 0.0 && s.gravity == 0.0 && s.walk_speed == 0.0
        && s.dash_speed == 0.0 && s.jump_height == 0.0;
    if stats_look_empty {
        warnings.push("stats look like defaults (weight=0, gravity=0, …) — Stage A may have failed".to_string());
    }

    if let Some(md) = md {
        let declared: Vec<&str> = md.characters.iter().map(|(id, _)| id.as_str()).collect();
        if !declared.is_empty() && !declared.contains(&char_name) {
            warnings.push(format!(
                "character {:?} not in Main's declared roster {:?} — likely a `--name` override against a phantom",
                char_name, declared,
            ));
        }
        if let (Some(id), Some(stem)) = (md.id.as_deref(), input_path.file_stem().and_then(|s| s.to_str())) {
            let stem_lc = stem.to_lowercase();
            // SSF2 `DAT<n>.ssf` archives are named by index, never by character, so
            // the stem can't match the id — skip the "may have been renamed" check
            // for them (it's meant to catch a hand-renamed single-character .swf).
            if !is_dat_archive_stem(stem) && id.to_lowercase() != stem_lc {
                warnings.push(format!(
                    "Main.id {:?} disagrees with filename stem {:?} — file may have been renamed",
                    id, stem_lc,
                ));
            }
        }
    }

    if !warnings.is_empty() {
        log::warn!("tier-1 validation: {} warning(s) for {}", warnings.len(), char_name);
        for w in &warnings { log::warn!("  - {}", w); }
    }
    warnings
}

/// Write `<char_dir>/conversion_log.json` for a single-character project.
fn write_conversion_log(
    char_dir: &Path,
    char_name: &str,
    char_data: &extractor::CharacterData,
    md: Option<&abc_parser::MainPackageMetadata>,
    validation_warnings: &[String],
) -> Result<()> {
    let snap = crate::api_mappings::snapshot_conversion_log();
    let to_entries = |m: std::collections::BTreeMap<String, usize>| -> Vec<serde_json::Value> {
        let mut v: Vec<(String, usize)> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.into_iter()
            .map(|(name, count)| serde_json::json!({ "name": name, "count": count }))
            .collect()
    };
    let mut payload = serde_json::json!({
        "character": char_name,
        "unknown": to_entries(snap.unknown),
        "ssf2_only": to_entries(snap.ssf2_only),
    });

    if let Some(md) = md {
        let source_method = md.characters.iter()
            .find(|(id, _)| id == char_name)
            .map(|(_, m)| format!("Main::{}", m));

        let mut ssf2_source = serde_json::Map::new();
        if let Some(id) = &md.id   { ssf2_source.insert("package_id".into(),   serde_json::json!(id)); }
        if let Some(g)  = &md.guid { ssf2_source.insert("package_guid".into(), serde_json::json!(g));  }
        if let Some(sm) = source_method {
            ssf2_source.insert("source_method".into(), serde_json::json!(sm));
        }
        if let Some(df) = &char_data.derived_from {
            ssf2_source.insert("parent_normal_stats_id".into(),
                serde_json::json!(df.parent_normal_stats_id));
            ssf2_source.insert("source_method".into(), serde_json::json!(df.source_method));
            ssf2_source.insert("note".into(), serde_json::json!(
                "Fraymakers has no native transformation API; \
                 this character is emitted as a standalone package and \
                 must be wired manually in the parent's Script.hx."
            ));
        }
        payload.as_object_mut().unwrap()
            .insert("ssf2_source".to_string(), serde_json::Value::Object(ssf2_source));
    } else if let Some(df) = &char_data.derived_from {
        payload.as_object_mut().unwrap().insert(
            "ssf2_source".to_string(),
            serde_json::json!({
                "parent_normal_stats_id": df.parent_normal_stats_id,
                "source_method": df.source_method,
                "note": "Fraymakers has no native transformation API; \
                    this character is emitted as a standalone package and \
                    must be wired manually in the parent's Script.hx.",
            }),
        );
    }

    if !validation_warnings.is_empty() {
        payload.as_object_mut().unwrap().insert(
            "validation_warnings".to_string(),
            serde_json::json!(validation_warnings),
        );
    }
    fs::create_dir_all(char_dir)?;
    fs::write(
        char_dir.join("conversion_log.json"),
        serde_json::to_string_pretty(&payload)? + "\n",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_id_from_getter_covers_every_corpus_shape() {
        let cases: &[(&str, &str)] = &[
            ("getMario",         "mario"),
            ("getBandanaDee",    "bandanadee"),
            ("getCaptainFalcon", "captainfalcon"),
            ("getChibiRobo",     "chibirobo"),
            ("getDonkeyKong",    "donkeykong"),
            ("getgameandwatch",  "gameandwatch"),
            ("getMegaMan",       "megaman"),
            ("getMetaKnight",    "metaknight"),
            ("getPacMan",        "pacman"),
            ("getBlackMage",     "blackmage"),
            ("getGigaBowser",    "gigabowser"),
            ("getWario_Man",     "wario_man"),
            ("getSheik",         "sheik"),
        ];
        for (m, expected) in cases {
            let got = derive_id_from_getter(m);
            assert_eq!(got.as_deref(), Some(*expected),
                "derive_id_from_getter({:?}) = {:?}, expected Some({:?})", m, got, expected);
        }
    }

    #[test]
    fn derive_id_rejects_non_get_prefix() {
        assert_eq!(derive_id_from_getter("init"), None);
        assert_eq!(derive_id_from_getter(""), None);
        assert_eq!(derive_id_from_getter("get"), None);
    }

    #[test]
    fn hscript_lint_accepts_sound_script() {
        let ok = r#"
            function init() {
                // a comment with unbalanced { ( [ chars
                var s = "string with } ] ) braces";
                var t = 'interp ${entity.getName()} and an escaped \' quote';
                /* block } comment */
                self.doThing({ a: 1, b: [2, 3] });
            }
        "#;
        assert_eq!(hscript_structural_problem(ok), None);
    }

    #[test]
    fn hscript_lint_catches_unbalanced_brace() {
        let bad = "function f() {\n  if (x) {\n    y();\n}\n";
        assert!(hscript_structural_problem(bad).is_some());
    }

    #[test]
    fn hscript_lint_catches_mismatch_and_stray_close() {
        assert!(hscript_structural_problem("foo([1, 2)").is_some());
        assert!(hscript_structural_problem("a) + b").is_some());
    }

    #[test]
    fn hscript_lint_catches_unterminated_string_and_empty_interp() {
        assert!(hscript_structural_problem("var s = \"oops;").is_some());
        assert!(hscript_structural_problem("var s = 'val ${}';").is_some());
    }

    #[test]
    fn hscript_lint_ignores_delims_in_strings_and_comments() {
        assert_eq!(hscript_structural_problem("var s = \"){]}[(\";"), None);
        assert_eq!(hscript_structural_problem("// )]}\nvar x = 1;"), None);
    }
}

#[cfg(test)]
mod duplicate_guid_lint_tests {
    use super::*;

    fn meta(dir: &Path, name: &str, guid: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), format!("{{\"guid\":\"{guid}\"}}")).unwrap();
    }

    #[test]
    fn two_assets_claiming_one_identity_are_reported() {
        let root = std::env::temp_dir().join("peptide-dupguid-test-report");
        let _ = fs::remove_dir_all(&root);
        meta(&root, "a.png.meta", "11111111-1111-5111-8111-111111111111");
        meta(&root.join("Shared/DAT59"), "a.png.meta", "11111111-1111-5111-8111-111111111111");
        let problems = lint_duplicate_guids(&root);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("same identity"), "{problems:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn distinct_identities_are_quiet() {
        let root = std::env::temp_dir().join("peptide-dupguid-test-quiet");
        let _ = fs::remove_dir_all(&root);
        meta(&root, "a.png.meta", "11111111-1111-5111-8111-111111111111");
        meta(&root.join("Shared/DAT59"), "a.png.meta", "22222222-2222-5222-8222-222222222222");
        assert!(lint_duplicate_guids(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
