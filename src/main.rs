use ssf2_converter::*;
use ssf2_converter::sound_extractor;

// ── Gated memory-limit allocator (diagnostic) ───────────────────────────────
// Default-disabled (limit 0 → no behavior change). Set CONV_MEM_LIMIT_MB to a
// number to cap live heap: once live bytes exceed the cap, alloc returns null so
// Rust's alloc-error handler aborts with "memory allocation of N bytes failed"
// (N = the allocation that tipped over) and, under RUST_BACKTRACE=1, the site.
// Used to localize the chibirobo/dedede convert-time OOM. The atomics add a
// negligible load on each (de)alloc and are inert when the limit is 0.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct LimitAlloc;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static LIMIT: AtomicUsize = AtomicUsize::new(0); // bytes; 0 = disabled

unsafe impl GlobalAlloc for LimitAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let lim = LIMIT.load(Relaxed);
        if lim != 0 {
            let prev = LIVE.fetch_add(layout.size(), Relaxed);
            if prev + layout.size() > lim {
                LIVE.fetch_sub(layout.size(), Relaxed);
                return std::ptr::null_mut(); // → handle_alloc_error aborts with the size
            }
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if LIMIT.load(Relaxed) != 0 { LIVE.fetch_sub(layout.size(), Relaxed); }
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static GLOBAL: LimitAlloc = LimitAlloc;

use clap::Parser;
use anyhow::{Result, Context};
use std::path::PathBuf;
use std::fs;

use ssf2_converter::project::{MultiCharSlot, ManifestCharEntry, ProcessedCharacter};

#[derive(Parser, Debug)]
#[command(name = "SSF2 to Fraymakers Converter")]
#[command(about = "Converts Super Smash Flash 2 character data to Fraymakers format", long_about = None)]
struct Args {
    /// Path to the .ssf file
    #[arg(value_name = "FILE")]
    input: PathBuf,

    /// Output directory for generated Fraymakers files
    #[arg(short, long, value_name = "DIR", default_value = "./characters")]
    output: PathBuf,

    /// Character name override (auto-detected from SWF if not provided).
    /// For multi-character SSFs, this selects only that character.
    #[arg(short, long)]
    name: Option<String>,

    /// Path to misc.ssf for costume palette data.
    /// Auto-detected from same directory as input if not provided.
    #[arg(long, value_name = "FILE")]
    misc_ssf: Option<PathBuf>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Rollback flag: emit each character of a multi-character SSF as its
    /// own .fraytools project (the pre-Stage-B layout). The default is
    /// the unified multi-character project per
    /// docs/multi_character_projects_plan.md §2. Slated for removal in
    /// a follow-up once FrayTools editor compatibility is confirmed.
    #[arg(long)]
    per_character_projects: bool,
}

fn main() -> Result<()> {
    // Diagnostic heap cap (see LimitAlloc). CONV_MEM_LIMIT_MB=2000 caps live heap
    // at 2 GB; unset/0 disables (normal operation).
    if let Ok(mb) = std::env::var("CONV_MEM_LIMIT_MB") {
        if let Ok(n) = mb.trim().parse::<usize>() {
            LIMIT.store(n.saturating_mul(1024 * 1024), Relaxed);
            if n > 0 { eprintln!("CONV_MEM_LIMIT_MB={n} → live-heap cap {n} MB", ); }
        }
    }
    let args = Args::parse();

    if args.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Info)
            .init();
    }

    log::info!("SSF2 → Fraymakers Converter");
    log::info!("Input: {}", args.input.display());

    // Read + decompress SSF
    let ssf_data = std::fs::read(&args.input)?;
    log::info!("Loaded {} bytes", ssf_data.len());
    let swf_data = ssf::decompress(&ssf_data)?;
    log::info!("Decompressed SWF: {} bytes", swf_data.len());

    let swf = swf_parser::parse(&swf_data)?;
    log::info!("Parsed SWF: v{}, {} ABC blocks", swf.version, swf.abc_blocks.len());

    // ── Palette extraction from misc.ssf ──────────────────────────────────────
    // Look for misc.ssf next to the input file (or in the same directory).
    // Extract all costume data in-process and cache to a temp JSON file.
    // This file is passed to the character processor then deleted when done.
    let costumes_path: Option<PathBuf> = args.misc_ssf.clone().and_then(|p| {
        // Explicit --misc-ssf provided: extract from it
        match extract_costumes_to_temp(&p) {
            Ok(path) => Some(path),
            Err(e) => {
                log::warn!("Costume extraction from {:?} failed: {}", p, e);
                None
            }
        }
    }).or_else(|| {
        let misc_ssf = args.input.parent()?.join("misc.ssf");
        if !misc_ssf.exists() {
            log::info!("misc.ssf not found next to input — skipping palette extraction");
            return None;
        }
        log::info!("Found misc.ssf — extracting costume palettes...");
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
    // Always delete temp file after — both explicit and auto-detected paths create temp files
    let costumes_is_temp = costumes_path.is_some();

    // Determine which character names to process
    let char_names: Vec<String> = if let Some(name) = args.name {
        vec![name]
    } else {
        // Auto-detect all root character MCs in the SWF
        let detected = detect_char_names(&swf, &args.input);
        if detected.is_empty() {
            // Fallback: use filename
            let fallback = args.input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("character")
                .to_string();
            vec![fallback]
        } else {
            detected
        }
    };

    log::info!("Characters to process: {:?}", char_names);

    // Decide on the per-SSF emission shape per
    // docs/multi_character_projects_plan.md §2:
    //
    //   * single-character SSF (or --per-character-projects rollback):
    //     each character lives in its own characters/<id>/ project
    //     (the pre-Stage-B layout, retained for one release).
    //   * multi-character SSF in default mode: one
    //     characters/<project_id>/ project containing all characters as
    //     peer entities, merged manifest, shared sprites/audio,
    //     collision-suffixed costumes.
    //
    // project_id comes from MainPackageMetadata.id (the SSF's own
    // package id, which matches the filename stem in every observed
    // corpus SSF — Tier 1 validation enforces this).
    let multi_char_mode = char_names.len() > 1 && !args.per_character_projects;
    let project_id: Option<String> = if multi_char_mode {
        swf.abc_blocks.iter()
            .filter_map(|b| abc_parser::parse(b).ok())
            .find_map(|abc| abc_parser::extract_main_package_metadata(&abc)
                .and_then(|md| md.id))
            .or_else(|| {
                args.input.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string())
            })
    } else { None };

    // Pre-compute each character's PascalCase form once so multi-char
    // slot bookkeeping can name menu entities + scripts subdirs without
    // re-doing the lookup per character.
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
    let project_dir: std::path::PathBuf = match (&multi_char_mode, &project_id) {
        (true, Some(pid)) => args.output.join(pid),
        _ => args.output.clone(),
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
            &swf_data, &swf, char_name, &args.output, costumes_path.as_deref(),
            &args.input, slot.as_ref(),
        ) {
            Ok(Some(artifacts)) => {
                accumulated_manifest_chars.push(artifacts.manifest_entry);
                accumulated_logs.push(artifacts.log_block);
            }
            Ok(None) => { /* single-character; finalized inside process_character */ }
            Err(e) => log::error!("Failed to process {}: {}", char_name, e),
        }
    }

    // Multi-char: write the project-level manifest + .fraytools + log
    // after all characters are processed. (Single-char path finalizes
    // inside process_character itself.)
    if multi_char_mode {
        if let Err(e) = finalize_multi_char_project(
            &project_dir, project_id.as_deref().unwrap_or("project"),
            &accumulated_manifest_chars, &accumulated_logs, &args.input,
        ) {
            log::error!("Failed to finalize multi-char project: {}", e);
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

    Ok(())
}

/// Extract all costume palettes from misc.ssf in-process and write to a temp JSON file.
/// Returns the path to the temp file on success.
fn extract_costumes_to_temp(misc_ssf: &std::path::Path) -> Result<PathBuf> {
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

    // Write to a temp file next to misc.ssf
    let temp_path = misc_ssf.parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(".ssf2_costumes_cache.json");
    fs::write(&temp_path, serde_json::to_string(&json_val)?)
        .context("write costumes cache")?;

    log::info!("Extracted {} characters' costume data from misc.ssf", all_costumes.len());
    Ok(temp_path)
}

/// Methods that, when present on a class, identify it as a real
/// SSF2 character — as opposed to a `*Ext` class that happens to share
/// the naming convention but is actually a projectile, an engine base
/// class, or some other helper.
///
/// Derive the canonical character id from a `Main::get<X>()` method name.
/// Per `docs/path2_unification_plan.md` §1: strip `get`, lowercase the
/// remainder, preserve explicit `_` characters. Examples:
///   getMario        → "mario"
///   getBandanaDee   → "bandanadee"
///   getGigaBowser   → "gigabowser"
///   getWario_Man    → "wario_man"
///   getgameandwatch → "gameandwatch"
fn derive_id_from_getter(method_name: &str) -> Option<String> {
    let stripped = method_name.strip_prefix("get")?;
    if stripped.is_empty() { return None; }
    Some(stripped.to_lowercase())
}

/// Detect all character names in a SWF.
///
/// Primary path — constructor walk
/// ([docs/constructor_walk_detection.md](docs/constructor_walk_detection.md)):
/// `Main`'s constructor literally lists the roster as
/// `register("characters", [self.getX(), self.getY(), ...])`. We walk
/// the iinit bytecode for that pattern and use the array as the
/// canonical source. Audit confirms it's universal across the 45-SSF
/// corpus, picks up zero dev-leftover orphans, and matches the path 2
/// enumeration's output 1:1.
///
/// Fallback — instance-method enumeration
/// ([docs/path2_unification_plan.md](docs/path2_unification_plan.md)
/// §1): enumerate every `Main::get*` instance method. Used only when
/// the constructor walk returns empty (defensive — handles a
/// hypothetical future SSF whose constructor builds the array
/// dynamically). Removed in a follow-up commit once we've confirmed
/// nothing surprised us.
///
/// Returns an empty vec for SWFs without a `Main` class (the misc.ssf
/// case); the caller falls back to the filename stem.
fn detect_char_names(swf: &ssf2_converter::swf_parser::SwfFile, _input_path: &PathBuf) -> Vec<String> {
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
            // Constructor present but characters[] empty / unparseable
            // — fall through to enumeration with a warning.
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


fn process_character(
    swf_data: &[u8],
    swf: &ssf2_converter::swf_parser::SwfFile,
    char_name: &str,
    output: &PathBuf,
    costumes: Option<&std::path::Path>,
    input_path: &std::path::Path,
    multi_char_slot: Option<&MultiCharSlot>,
) -> Result<Option<ProcessedCharacter>> {
    // Fresh conversion log for this character — counts unknown / SSF2-only
    // calls so we can write conversion_log.json next to the exported files
    // and surface them in the GUI popup.
    ssf2_converter::api_mappings::reset_conversion_log();

    // Parse the SWF exactly once for the duration of this character. Every
    // downstream extractor accepts `&swf::Swf` via its `_from_swf` entry
    // point so we don't re-decompress+re-parse 7+N+M times (where N is the
    // projectile count and M the effect count). Previously each per-char
    // extractor re-ran swf::decompress_swf + swf::parse_swf on the same
    // buffer — measurable cost on projectile-heavy chars.
    let parsed_swf_buf = swf::decompress_swf(swf_data)
        .map_err(|e| anyhow::anyhow!("decompress SWF for {}: {}", char_name, e))?;
    let parsed_swf = swf::parse_swf(&parsed_swf_buf)
        .map_err(|e| anyhow::anyhow!("parse SWF for {}: {}", char_name, e))?;

    // Extract character data (ABC: attacks, stats, frame scripts, xframe map)
    let mut char_data = extractor::extract(swf, char_name)?;
    log::info!("Extracted: {} attacks, {} animations, {} ssf2→fm mappings",
        char_data.attacks.len(), char_data.animations.len(), char_data.ssf2_to_fm_anim.len());

    // Lift Main's package metadata (id/guid/declared roster) once per
    // character. Cheap — single iinit walk per ABC block. Per the
    // constructor-walk plan, this lives in conversion_log.json only,
    // not on CharacterData.
    let package_metadata = swf.abc_blocks.iter()
        .filter_map(|b| abc_parser::parse(b).ok())
        .find_map(|abc| abc_parser::extract_main_package_metadata(&abc));
    let validation = run_tier1_validation(char_name, &char_data, package_metadata.as_ref(), input_path);

    // PascalCase form for entity filenames + scripts subdir, per
    // docs/multi_character_projects_plan.md §1. The constructor walker
    // gave us each character's Main::get<X> method name — we derive
    // <Pascal> from that (strip `get`, strip `_`, uppercase first
    // char). When the method name isn't available (--name override or
    // filename fallback), pascal_form falls back to acting on the char
    // id directly.
    let char_pascal: String = package_metadata.as_ref()
        .and_then(|md| md.characters.iter().find(|(id, _)| id == char_name))
        .map(|(_, method)| abc_parser::pascal_form(method))
        .unwrap_or_else(|| abc_parser::pascal_form(char_name));

    // Extract median xframe scale from root character MovieClip
    let (base_scale_x, base_scale_y) = sprite_parser::extract_xframe_scale_from_swf(&parsed_swf, char_name)
        .unwrap_or_else(|e| {
            log::warn!("extract_xframe_scale failed: {}, defaulting to 1.0", e);
            (1.0, 1.0)
        });
    // Apply the global SSF2 → Fraymakers size multiplier (SSF2 sprites are ~1.9× smaller).
    // The factor lives in mappings/character/stats.jsonc :: size_multiplier for easy editing.
    let size_mult = mappings::character_stats().size_multiplier;
    char_data.stats.base_scale_x = base_scale_x * size_mult;
    char_data.stats.base_scale_y = base_scale_y * size_mult;
    log::info!("Character base scale: scaleX={:.4}, scaleY={:.4} (raw {:.4}/{:.4} × {} size multiplier)",
        char_data.stats.base_scale_x, char_data.stats.base_scale_y, base_scale_x, base_scale_y, size_mult);

    // Root MC transforms — computed once and shared between sprite-box
    // extraction and image extraction (both used to compute their own
    // copy, doubling the work).
    let xform_map = sprite_parser::extract_xframe_transforms_from_swf(
        &parsed_swf, char_name, &char_data.ssf2_to_fm_anim,
    ).unwrap_or_default();

    // Extract per-frame collision box geometry
    let sprite_boxes = sprite_parser::parse_sprite_boxes_from_swf(
        &parsed_swf, char_name, &char_data.ssf2_to_fm_anim, &xform_map,
    ).unwrap_or_else(|e| {
        log::warn!("sprite_parser failed: {}", e);
        Default::default()
    });
    log::info!("Sprite boxes: {} animations with geometry", sprite_boxes.len());

    // For multi-char projects the shared output dir is the project_dir
    // (e.g. <output>/zelda/) and every character writes into it. For
    // single-char it's <output>/<char_id>/ as before.
    let char_output_dir = match multi_char_slot {
        Some(s) => s.project_dir.clone(),
        None => output.join(char_name),
    };
    let img_result = image_extractor::extract_images_from_swf(
        &parsed_swf, &char_output_dir, char_name, &char_data.ssf2_to_fm_anim, &xform_map,
    ).unwrap_or_else(|e| {
        log::warn!("image_extractor failed: {}", e);
        image_extractor::ImageExtractionResult {
            images: Default::default(),
            shape_to_bitmap: Default::default(),
            shape_pivot: Default::default(),
            anim_images: Default::default(),
        }
    });
    log::info!("Extracted {} sprite images, {} anim image maps",
        img_result.images.len(), img_result.anim_images.len());

    // Extract sounds. Per docs/multi_character_projects_plan.md §3:
    //   * single-character project: flat library/audio/*.wav (unchanged).
    //   * multi-character project: library/audio/<char_id>/*.wav so each
    //     character's audio is namespaced under its own subdir even when
    //     the project ships multiple characters' sounds side-by-side.
    let sounds_dir = match multi_char_slot {
        Some(_) => char_output_dir.join(format!("library/audio/{}", char_name)),
        None    => char_output_dir.join("library/audio"),
    };
    let sounds = match sound_extractor::extract_all_sounds(swf_data, &sounds_dir, char_name) {
        Ok(s) => s,
        Err(e) => { log::warn!("sound_extractor failed: {}", e); vec![] }
    };

    // Discover projectiles, effects, and head sprite
    let (projectiles, effects, head_sprite) = image_extractor::discover_projectiles_and_head_from_swf(
        &parsed_swf, char_name,
    ).unwrap_or_else(|e| {
        log::warn!("discover_projectiles_and_head failed: {}", e);
        (vec![], vec![], None)
    });
    log::info!("Discovered {} projectiles, {} effects, head={}",
        projectiles.len(),
        effects.len(),
        head_sprite.as_ref().map(|h| h.name.as_str()).unwrap_or("none"));

    // Generate Fraymakers files. In multi-char mode haxe_gen skips the
    // project-level manifest + .fraytools writes — those are handled by
    // finalize_multi_char_project after every character has been processed.
    haxe_gen::generate(output, char_name, &char_pascal, &char_data, &sprite_boxes, &img_result,
        costumes, &sounds, &projectiles, &effects, head_sprite.as_ref(), &parsed_swf,
        multi_char_slot)?;
    log::info!("Generated Fraymakers files for {}", char_name);

    // Build the per-character outputs the project finalizer needs.
    let projectile_names: Vec<String> = projectiles.iter().map(|p| p.name.clone()).collect();
    let menu_entity_id = match multi_char_slot {
        Some(_) => format!("{}_menu", char_name),
        None    => "menu".to_string(),
    };

    if multi_char_slot.is_none() {
        // Single-char path: keep the existing inline conversion-log write.
        write_conversion_log(&char_output_dir, char_name, &char_data,
            package_metadata.as_ref(), &validation)?;
        return Ok(None);
    }

    // Multi-char path: return artifacts for the project finalizer to
    // merge into the project-level manifest + conversion_log.
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

/// Assemble the per-character `ssf2_source` + `validation_warnings`
/// payload used inside the multi-char project log's `characters: [...]`
/// array. Mirrors the single-character `write_conversion_log` shape so
/// the GUI popup can handle both: it can branch on whether the
/// top-level log has a `characters` array.
fn build_conversion_log_block(
    char_name: &str,
    char_data: &extractor::CharacterData,
    md: Option<&abc_parser::MainPackageMetadata>,
    validation_warnings: &[String],
) -> serde_json::Value {
    let snap = ssf2_converter::api_mappings::snapshot_conversion_log();
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
/// `library/manifest.json` + project-level `conversion_log.json` with
/// the per-character `characters: [...]` array. Called once after every
/// character in the SSF has been emitted.
fn finalize_multi_char_project(
    project_dir: &std::path::Path,
    project_id: &str,
    chars: &[ManifestCharEntry],
    char_logs: &[serde_json::Value],
    input_path: &std::path::Path,
) -> Result<()> {
    log::info!("Finalising multi-char project: {} ({} characters)", project_id, chars.len());

    // .fraytools
    fs::write(project_dir.join(format!("{}.fraytools", project_id)),
        ssf2_converter::fraytools_project::generate_fraytools_project(project_id))?;

    // manifest.json — merged content[] with one type:"character" entry per char
    let manifest = haxe_gen::generate_multi_char_manifest(project_id, chars);
    fs::write(project_dir.join("library/manifest.json"), &manifest)?;
    fs::write(project_dir.join("library/manifest.json.meta"),
        haxe_gen::generate_manifest_meta_pub(&ssf2_converter::uuid_gen::det_uuid(
            &format!("{}::manifest::meta", project_id))))?;

    // conversion_log.json — project-scoped with characters: [...] array
    let payload = serde_json::json!({
        "project":      project_id,
        "input":        input_path.file_name().and_then(|s| s.to_str()),
        "characters":   char_logs,
    });
    fs::write(project_dir.join("conversion_log.json"),
        serde_json::to_string_pretty(&payload)? + "\n")?;
    Ok(())
}

/// Tier 1 validation warnings — soft logs only, never hard-fail. Emitted
/// into `conversion_log.json.validation_warnings`. Three checks:
///   1. Extracted stats / attacks are non-empty (catches silent
///      extractor regressions).
///   2. `char_name` appears in the constructor's declared `characters[]`
///      (catches a `--name` override that targets a phantom).
///   3. `register("id", ...)` matches the input filename stem
///      (catches a renamed SSF — useful for traceability).
fn run_tier1_validation(
    char_name: &str,
    char_data: &extractor::CharacterData,
    md: Option<&abc_parser::MainPackageMetadata>,
    input_path: &std::path::Path,
) -> Vec<String> {
    let mut warnings = Vec::new();

    if char_data.attacks.is_empty() {
        warnings.push("attacks map is empty — extractor produced no attack data".to_string());
    }
    // CharacterStats.default() is all zeros except max_jumps=2 and base_scale_*=1.
    // A character whose stats are "almost defaults" indicates Stage A failed.
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
            if id.to_lowercase() != stem_lc {
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

/// Write `<char_dir>/conversion_log.json` summarising calls that the
/// converter couldn't fully map: `unknown` are genuine gaps (no entry in any
/// commands.jsonc section), `ssf2_only` are calls we deliberately surfaced as
/// `// [SSF2-only: …]` comments because they have no Fraymakers equivalent.
/// Also carries an `ssf2_source` block with the SSF package id/guid, the
/// `Main::get<X>` source method, and (for transformation characters) the
/// parent's normalStats_id + an explanatory note. Validation warnings
/// land under `validation_warnings`.
/// Written unconditionally so the GUI can show a post-conversion
/// popup, and so CLI users get the same artifact alongside the character.
fn write_conversion_log(
    char_dir: &std::path::Path,
    char_name: &str,
    char_data: &extractor::CharacterData,
    md: Option<&abc_parser::MainPackageMetadata>,
    validation_warnings: &[String],
) -> Result<()> {
    let snap = ssf2_converter::api_mappings::snapshot_conversion_log();
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

    // ssf2_source: present whenever we have Main metadata (i.e. always
    // for character SSFs; absent for misc.ssf-style packages). Always
    // includes package_id / package_guid / source_method; adds the
    // transformation overlay when normalStats_id mismatches the
    // derived id.
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
            // For transformations, derived_from.source_method overrides
            // any value we might have looked up from md.characters —
            // they're identical, but derived_from is the authoritative
            // path.
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
        // No Main metadata but we still got a derived_from from Stage A.
        // Preserves the Step D behaviour for transformations.
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
    std::fs::create_dir_all(char_dir)?;
    std::fs::write(
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
        // Lowercase + strip "get" + preserve explicit `_` chars. Cases
        // span the corpus shapes — single word, camelCase, mid-word
        // underscore, all-lowercase prefix. Matches the existing
        // converter's output directory names for all 44 normal
        // characters, plus the three new sub-character ids.
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
                "derive_id_from_getter({:?}) = {:?}, expected Some({:?})",
                m, got, expected);
        }
    }

    #[test]
    fn derive_id_rejects_non_get_prefix() {
        assert_eq!(derive_id_from_getter("init"), None);
        assert_eq!(derive_id_from_getter(""), None);
        // "get" alone (no suffix) is also rejected — empty id is useless.
        assert_eq!(derive_id_from_getter("get"), None);
    }
}
