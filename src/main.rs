use ssf2_converter::*;
use ssf2_converter::sound_extractor;

use clap::Parser;
use anyhow::{Result, Context};
use std::path::PathBuf;
use std::fs;

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
}

fn main() -> Result<()> {
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

    for char_name in &char_names {
        log::info!("─── Processing: {} ───", char_name);
        if let Err(e) = process_character(
            &swf_data, &swf, char_name, &args.output, costumes_path.as_deref()
        ) {
            log::error!("Failed to process {}: {}", char_name, e);
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
/// The SSF2 engine routes character logic through these accessors, so
/// every real character implements at least one of them. Projectile
/// `*Ext` classes (`DeeSpearExt extends SSF2Projectile`) don't, nor
/// does the framework's own `SSF2CharacterExt` base class. Requiring
/// at least one of these to be present rejects both kinds of false
/// positive.
const CHARACTER_MARKER_METHODS: &[&str] = &[
    "getOwnStats",
    "getAttackStats",
    "getProjectileStats",
];

/// Detect all root character names in a SWF by looking for `{Name}Ext`
/// ABC classes that ALSO implement at least one of the character-marker
/// methods listed above. The naming convention alone is not sufficient
/// — for example bandanadee.ssf carries:
///   - `BandanaDeeExt extends SSF2Character`   ← real character
///   - `DeeSpearExt   extends SSF2Projectile`  ← a projectile, NOT a character
/// and some SWFs carry `SSF2CharacterExt extends SSF2Character` as the
/// framework's own base class. The marker-method check filters both
/// out.
/// Returns true iff the given ABC class looks like a real character's
/// `*Ext` extension class — the test that `detect_char_names` applies
/// per class. Split out so it can be unit-tested directly against
/// hand-crafted `abc_parser::Class` values.
fn is_character_ext_class(class: &abc_parser::Class) -> bool {
    let Some(prefix) = class.name.strip_suffix("Ext") else { return false };
    if prefix.len() < 2 || !prefix.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    class.instance_methods.iter()
        .any(|t| CHARACTER_MARKER_METHODS.contains(&t.name.as_str()))
}

fn detect_char_names(swf: &ssf2_converter::swf_parser::SwfFile, input_path: &PathBuf) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();

    for abc_bytes in &swf.abc_blocks {
        let Ok(abc) = abc_parser::parse(abc_bytes) else { continue; };
        for class in &abc.classes {
            if is_character_ext_class(class) {
                // Safe: strip_suffix succeeded in the predicate.
                let prefix = class.name.strip_suffix("Ext").unwrap();
                names.push(prefix.to_lowercase());
            }
        }
    }

    // Deduplicate, preserve order
    let mut seen = std::collections::HashSet::new();
    names.retain(|n| seen.insert(n.clone()));

    if !names.is_empty() {
        // Resolve truncated names against the filename.
        // e.g. ABC has "CaptainExt" -> "captain", filename is "captainfalcon"
        // -> use "captainfalcon" as the canonical name.
        let stem = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();

        let resolved: Vec<String> = names.iter().map(|n| {
            // If the filename starts with this name (or vice versa), use the longer one
            if stem.starts_with(n.as_str()) { stem.clone() }
            else if n.starts_with(stem.as_str()) { n.clone() }
            else { n.clone() }
        }).collect();

        // Deduplicate again after resolution
        let mut seen2 = std::collections::HashSet::new();
        let mut out = Vec::new();
        for n in resolved {
            if seen2.insert(n.clone()) { out.push(n); }
        }
        return out;
    }

    // Fallback: use filename
    if let Some(stem) = input_path.file_stem().and_then(|s| s.to_str()) {
        return vec![stem.to_lowercase()];
    }

    vec![]
}

fn process_character(
    swf_data: &[u8],
    swf: &ssf2_converter::swf_parser::SwfFile,
    char_name: &str,
    output: &PathBuf,
    costumes: Option<&std::path::Path>,
) -> Result<()> {
    // Fresh conversion log for this character — counts unknown / SSF2-only
    // calls so we can write conversion_log.json next to the exported files
    // and surface them in the SwiftUI popup.
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

    // Extract median xframe scale from root character MovieClip
    let (base_scale_x, base_scale_y) = sprite_parser::extract_xframe_scale_from_swf(&parsed_swf, char_name)
        .unwrap_or_else(|e| {
            log::warn!("extract_xframe_scale failed: {}, defaulting to 1.0", e);
            (1.0, 1.0)
        });
    char_data.stats.base_scale_x = base_scale_x;
    char_data.stats.base_scale_y = base_scale_y;
    log::info!("Character base scale: scaleX={:.4}, scaleY={:.4}", base_scale_x, base_scale_y);

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

    // Extract sprite images
    let char_output_dir = output.join(char_name);
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

    // Extract sounds (uses its own hand-rolled SWF tag walker, not the
    // `swf` crate; left untouched).
    let sounds_dir = char_output_dir.join("library/audio");
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

    // Generate Fraymakers files
    haxe_gen::generate(output, char_name, &char_data, &sprite_boxes, &img_result,
        costumes, &sounds, &projectiles, &effects, head_sprite.as_ref(), &parsed_swf)?;
    log::info!("Generated Fraymakers files for {}", char_name);

    write_conversion_log(&char_output_dir, char_name)?;

    Ok(())
}

/// Write `<char_dir>/conversion_log.json` summarising calls that the
/// converter couldn't fully map: `unknown` are genuine gaps (no entry in any
/// commands.jsonc section), `ssf2_only` are calls we deliberately surfaced as
/// `// [SSF2-only: …]` comments because they have no Fraymakers equivalent.
/// Written unconditionally so the SwiftUI GUI can show a post-conversion
/// popup, and so CLI users get the same artifact alongside the character.
fn write_conversion_log(char_dir: &std::path::Path, char_name: &str) -> Result<()> {
    let snap = ssf2_converter::api_mappings::snapshot_conversion_log();
    let to_entries = |m: std::collections::BTreeMap<String, usize>| -> Vec<serde_json::Value> {
        let mut v: Vec<(String, usize)> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.into_iter()
            .map(|(name, count)| serde_json::json!({ "name": name, "count": count }))
            .collect()
    };
    let payload = serde_json::json!({
        "character": char_name,
        "unknown": to_entries(snap.unknown),
        "ssf2_only": to_entries(snap.ssf2_only),
    });
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
    use ssf2_converter::abc_parser::{Class, Trait};

    fn mk_class(name: &str, methods: &[&str]) -> Class {
        Class {
            name: name.to_string(),
            super_name: String::new(),
            instance_methods: methods.iter().map(|m| Trait {
                name: m.to_string(),
                kind: 1, // Method
                method_idx: 0,
                slot_idx: 0,
            }).collect(),
            class_methods: vec![],
            constructor_idx: 0,
        }
    }

    #[test]
    fn character_ext_with_get_own_stats_is_accepted() {
        // BandanaDeeExt-style: implements getOwnStats / getAttackStats /
        // getProjectileStats. Should be classified as a character.
        let c = mk_class("MarioExt", &["getOwnStats", "getAttackStats", "getProjectileStats"]);
        assert!(is_character_ext_class(&c));
    }

    #[test]
    fn projectile_ext_without_marker_methods_is_rejected() {
        // DeeSpearExt is bandanadee's spear projectile. It happens to end
        // in `Ext` but extends SSF2Projectile, not SSF2Character, and
        // implements none of the character-marker methods. Must be
        // rejected — otherwise we'd emit a spurious `deespear/` output
        // dir alongside `bandanadee/`.
        let c = mk_class("DeeSpearExt", &["update", "onCollision"]);
        assert!(!is_character_ext_class(&c),
            "projectile Ext without marker methods must NOT be classified as a character");
    }

    #[test]
    fn framework_ssf2_character_ext_was_already_rejected_but_pin_it() {
        // SSF2CharacterExt is the engine's own base class. The `prefix
        // must be all alphabetic` rule rejects it via the digit `2`.
        // Pin that the marker-method check ALSO rejects it (defence in
        // depth) — if some future SWF carries a no-digit variant we'd
        // still want it filtered.
        let c = mk_class("SSF2CharacterExt", &[]);
        assert!(!is_character_ext_class(&c));
        let c = mk_class("FrameworkExt", &[]); // hypothetical no-digit framework ext
        assert!(!is_character_ext_class(&c));
    }

    #[test]
    fn non_ext_classes_are_rejected() {
        let c = mk_class("Mario", &["getOwnStats"]);
        assert!(!is_character_ext_class(&c));
        let c = mk_class("MarioExtension", &["getOwnStats"]);
        assert!(!is_character_ext_class(&c));
    }

    #[test]
    fn too_short_prefix_rejected() {
        let c = mk_class("AExt", &["getOwnStats"]);
        assert!(!is_character_ext_class(&c),
            "single-char prefix is too ambiguous to accept");
    }

    #[test]
    fn any_one_marker_method_is_sufficient() {
        // Real characters all have all three, but the predicate only
        // requires ONE so future engines that drop a method don't
        // suddenly fail recognition.
        for m in CHARACTER_MARKER_METHODS {
            let c = mk_class("FooExt", &[m]);
            assert!(is_character_ext_class(&c),
                "marker '{}' alone should be sufficient", m);
        }
    }
}
