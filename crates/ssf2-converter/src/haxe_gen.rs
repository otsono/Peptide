/// Fraymakers character file generator.
/// Produces output matching the official character-template structure:
/// https://github.com/Fraymakers/character-template
/// Reference character: https://github.com/ZacharyClayton721/kung-fu-man-fraymakers

use anyhow::Result;
use std::fs;
use std::path::Path;
use crate::extractor::{CharacterData, Hitbox};
use crate::entity_gen;
use crate::fraytools_project;
use crate::palette_gen;
use crate::uuid_gen::det_uuid;

pub fn generate(output_dir: &Path, char_name: &str, char_pascal: &str, data: &CharacterData, sprite_boxes: &std::collections::BTreeMap<String, crate::sprite_parser::AnimationBoxData>, img_result: &crate::image_extractor::ImageExtractionResult, costumes_json: Option<&Path>, sounds: &[crate::sound_extractor::SoundEntry], projectiles: &[crate::image_extractor::DiscoveredProjectile], effects: &[crate::image_extractor::DiscoveredEffect], head_sprite: Option<&crate::image_extractor::DiscoveredHead>, parsed_swf: &swf::Swf<'_>, multi_char_slot: Option<&crate::project::MultiCharSlot>, input_path: &Path) -> Result<()> {
    let char_id = char_name.to_lowercase().replace(" ", "");
    // a script that reads a hitbox stat back while a move runs needs every write to it noted
    // (api_mappings::note_hitbox_writes); nothing else pays for the notes
    crate::api_mappings::set_mirror_hitbox_writes(
        data.scripts.iter().any(|sc| sc.code.contains("getAttackBoxStat(")));
    // In multi-char mode every character writes into the shared project
    // dir; in single-char mode each character has its own subdir under
    // `output_dir`. The project-finalizer (main::finalize_multi_char_project)
    // handles the project-level manifest + .fraytools when in multi-char.
    let char_dir = match multi_char_slot {
        Some(s) => s.project_dir.clone(),
        None    => output_dir.join(&char_id),
    };
    // Per docs/multi_character_projects_plan.md §1: character scripts
    // live at library/scripts/<Pascal>/ (was library/scripts/Character/).
    let scripts_dir = char_dir.join(format!("library/scripts/{}", char_pascal));
    fs::create_dir_all(&scripts_dir)?;

    // AN EFFECT THE CHARACTER NAMES THAT ITS OWN FILE DOES NOT CARRY. A shared effect is
    // registered by whichever file DEFINES it, so a character that only references one has
    // nothing of its own to convert -- kirby's copy abilities name forty of them, all belonging
    // to the characters he copies, and mario's cape explosion is another's too. They are found
    // and brought across the same way a shared sound is, and from then on they are effects of
    // this character like any other: in the map below, and emitted as entities.
    let imported = import_shared_effects(data, effects, output_dir, char_name, char_pascal, input_path);
    let effects: &[crate::image_extractor::DiscoveredEffect] = &if imported.is_empty() {
        effects.to_vec()
    } else {
        effects.iter().cloned().chain(imported.iter().map(|(e, _)| e.clone())).collect::<Vec<_>>()
    };

    // Effect → primary-animation map for the context-aware
    // self.attachEffect("name") → match.createVfx(...) rewrite. The guard
    // installs the map for the duration of this function and clears it on
    // drop, so every translate_ssf2_to_fm call made while generating this
    // character (its projectiles' scripts) sees the right effect→animation pairing.
    let effect_anim_map: std::collections::BTreeMap<String, String> = effects.iter()
        .filter_map(|e| {
            entity_gen::effect_animation_names(e)
                .into_iter()
                .next()
                .map(|first| (e.name.clone(), first))
        })
        .collect();
    let _effect_anim_guard = crate::api_mappings::EffectAnimGuard::install(effect_anim_map);

    // Per-character extracted-sound id set → lets rewrite_play_sound_calls
    // emit getContent("id") for sounds we have and a placeholder for the rest.
    // Mirrors how sound_extractor sanitizes names for the .wav filenames.
    let sound_ids: std::collections::BTreeSet<String> = sounds.iter()
        .map(|s| s.name.chars()
            .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect())
        .collect();
    let _sounds_guard = crate::api_mappings::AvailableSoundsGuard::install(sound_ids);

    // Each projectile's declared launch AND spawn offset, so the spawn hands both over. Keyed by
    // the SSF2 name the `fireProjectile` call uses, which is the key the declaration sits under. A
    // shot the source aims from a point off the fighter (sora's keyblade leaves the hand at
    // yoffset -17, clear of the floor it destroys on) needs that offset, or it spawns at the feet.
    // `raw_speeds = false`: the speeds are handed over already in engine units, the same as the
    // by-hand launch map this replaces.
    let _launch_guard = crate::api_mappings::ProjectileLaunchGuard::from_declarations(
        &data.projectile_data, false);

    log::info!("Generating Fraymakers character package in {}", char_dir.display());

    // How many of jab1/jab2/jab3/jab4 actually have image content. Drives
    // the keep-empty allowlist in entity_gen: a single-jab character's
    // jab2/jab3 are dropped as empty along with the others.
    let populated_jabs = count_populated_jabs(img_result);

    let splits = crate::anim_splitter::split_animations(&data.animations, sprite_boxes, data.stats.jump_startup.round().max(0.0) as u16, &data.scripts);
    fs::write(scripts_dir.join("HitboxStats.hx"),   generate_hitbox_stats(data, &char_id, &splits))?;
    fs::write(scripts_dir.join("CharacterStats.hx"), generate_character_stats(data, &char_id))?;
    let _anim_guard = crate::api_mappings::AnimNameGuard::install_with_labels(
        splits.iter().map(|s| (s.fm_name.clone(), s.fm_name.clone())).collect(),
        // a label at frame 0 of a split names that split: the combo stage the source jumped to.
        // only a name one clip carries: a name several clips use (`continue`, the landing part of
        // a down special and of an aerial) is resolved as the script runs, within the clip that
        // is playing (`_jumpLabel`), and named here it sent ganondorf's landing into an aerial's
        {
            let mut count: std::collections::BTreeMap<&str, usize> = Default::default();
            for s in &splits { for (l, _) in &s.labels { *count.entry(l.as_str()).or_default() += 1; } }
            splits.iter()
                .flat_map(|s| s.labels.iter().filter(|(l, f)| *f == 0 && count.get(l.as_str()) == Some(&1))
                    .map(move |(l, _)| (l.clone(), s.fm_name.clone())))
                .collect()
        });
    fs::write(scripts_dir.join("AnimationStats.hx"), generate_animation_stats(data, &splits))?;
    // The character declares an ai, so the script it names has to exist -- otherwise the engine
    // says "Could not find script" twice on every load. SSF2 characters DO declare cpu behaviour
    // (`cpuAttackQueue` and friends); none of it is ported yet, so this is an ai that stands
    // there, which is what an unported cpu honestly is.
    fs::write(scripts_dir.join("AiScript.hx"), generate_ai_script(&char_id))?;
    // the platforms the fighter gives itself: one structure content each, on a shared follower
    if !data.platforms.is_empty() {
        let script_id = format!("{char_id}PlatformScript");
        fs::write(scripts_dir.join("PlatformScript.hx"), crate::projectile_gen::self_platform_script())?;
        fs::write(scripts_dir.join("PlatformScript.hx.meta"),
            script_meta(&script_id, &det_uuid(&format!("{char_id}::PlatformScript::meta")), ScriptMetaKind::StructureScript))?;
        for (i, _) in data.platforms.iter().enumerate() {
            let stats_id = format!("{char_id}Platform{i}Stats");
            fs::write(scripts_dir.join(format!("Platform{i}Stats.hx")), format!(
                "// a platform {char_id} gives itself: the line it carries, riding it\n{{\n\tspriteContent: self.getResource().getContent(\"{char_id}\"),\n\tanimationId: \"platformSprite{i}\",\n\tstartX: 0,\n\tstartY: 0\n}}\n"))?;
            fs::write(scripts_dir.join(format!("Platform{i}Stats.hx.meta")),
                script_meta(&stats_id, &det_uuid(&format!("{char_id}::Platform{i}Stats::meta")), ScriptMetaKind::CharacterAnimationStats))?;
            crate::projectile_gen::note_structure_content(format!("{char_id}platform{i}"), stats_id, script_id.clone());
        }
    }

    // .meta sidecar files for character scripts
    fs::write(scripts_dir.join("HitboxStats.hx.meta"),    script_meta(&format!("{}HitboxStats", char_id),    &det_uuid(&format!("{}::HitboxStats::meta", char_id)),    ScriptMetaKind::CharacterHitboxStats))?;
    fs::write(scripts_dir.join("CharacterStats.hx.meta"), script_meta(&format!("{}CharacterStats", char_id), &det_uuid(&format!("{}::CharacterStats::meta", char_id)), ScriptMetaKind::CharacterStats))?;
    fs::write(scripts_dir.join("AnimationStats.hx.meta"), script_meta(&format!("{}AnimationStats", char_id), &det_uuid(&format!("{}::AnimationStats::meta", char_id)), ScriptMetaKind::CharacterAnimationStats))?;
    fs::write(scripts_dir.join("Script.hx.meta"),         script_meta(&format!("{}Script", char_id),         &det_uuid(&format!("{}::Script::meta", char_id)),         ScriptMetaKind::CharacterScript))?;
    fs::write(scripts_dir.join("AiScript.hx.meta"), script_meta(&format!("{}AiScript", char_id), &det_uuid(&format!("{}::AiScript::meta", char_id)), ScriptMetaKind::CharacterAiScript))?;

    // .fraytools + manifest are project-level. In single-char mode we
    // write them here; in multi-char mode they're written once by the
    // project finalizer in main.rs after every character is processed.
    if multi_char_slot.is_none() {
        fs::write(char_dir.join(format!("{}.fraytools", char_name)), fraytools_project::generate_fraytools_project(char_name))?;
        fs::write(char_dir.join("library/manifest.json.meta"), generate_manifest_meta(&det_uuid(&format!("{}::manifest::meta", char_id))))?;
    }

    // <Pascal>.entity (was Character.entity — see plan §1)
    let entities_dir = char_dir.join("library/entities");
    fs::create_dir_all(&entities_dir)?;
    let char_entity_filename = format!("{}.entity", char_pascal);
    // built once: the costume palette below only points this same entity at its palette
    let _ = entity_gen::take_dropped_animations();
    let char_entity_json = entity_gen::draw_turned(
        &entity_gen::generate_entity(data, &char_id, sprite_boxes, img_result, populated_jabs),
        &entity_gen::turned_throws(data));
    fs::write(entities_dir.join(&char_entity_filename), &char_entity_json)?;
    let _ = entity_gen::take_dropped_animations();

    // Generate .meta sidecar files for each sprite PNG. Image GUIDs are
    // symbol-only (crate::uuid_gen::image_meta_guid) so they're consistent across
    // the character, projectile/effect and menu paths — critical for multi-char
    // projects where one library/sprites/ is shared (else refs dangle → placeholders).
    let meta_guids = entity_gen::get_image_meta_guids(img_result);
    let sprites_dir = char_dir.join("library/sprites");
    let mut meta_count = 0;
    for (png_rel_path, guid) in &meta_guids {
        let meta_path = sprites_dir.join(format!("{}.meta", png_rel_path.trim_start_matches("library/sprites/")));
        if let Some(parent) = meta_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&meta_path, entity_gen::generate_meta(guid))?;
        meta_count += 1;
    }
    log::info!("Generated {} .meta sidecar files", meta_count);

    let mut palette_collection_guid: Option<String> = None;
    let mut palette_base_map_id: Option<String> = None;
    // ── Palette / costumes ──────────────────────────────────────────────────
    // For multi-char projects, characters share the project-level
    // costumes.palettes / palette_preview.png paths. The first character
    // (constructor-walk slot 0) keeps the unsuffixed filename; subsequent
    // characters append `2`/`3`/... per the suffix rule in
    // docs/multi_character_projects_plan.md §2.
    let suffix = multi_char_slot.and_then(|s| s.collision_suffix())
        .map(|n| n.to_string()).unwrap_or_default();
    match palette_gen::generate_palettes_and_remap(&char_id, char_name, &sprites_dir, costumes_json) {
        Ok(pal) => {
            // Suffix goes on the BASE name, not the extension: char 2 is
            // `costumes2.palettes`, NOT `costumes.palettes2`. A `.palettes2`
            // extension is malformed — FrayTools only recognizes `.palettes`
            // collections, so the 2nd+ char's costumes never loaded (the engine's
            // "Could not find script: <char>Costumes" lookup then failed). Same
            // class of bug as the palette_preview.png suffix fixed below.
            fs::write(char_dir.join(format!("library/costumes{}.palettes", suffix)),       &pal.palettes_json)?;
            fs::write(char_dir.join(format!("library/costumes{}.palettes.meta", suffix)),  &pal.palettes_meta_json)?;
            // CHAR-UNIQUE preview filename. The old `palette_preview.png{suffix}` scheme
            // gave the 2nd+ char of a multi-char project a name like `palette_preview.png2`
            // — a malformed extension that ALSO shares the base name `palette_preview` with
            // char 1, so FrayTools (which derives a sprite's GUID from its path) collides
            // the two previews. A char-prefixed `.png` name is unique + valid. The palette
            // links its preview by GUID (imageAsset), so the filename isn't referenced elsewhere.
            let preview_name = format!("{}_palette_preview.png", char_id);
            fs::write(sprites_dir.join(&preview_name),                    &pal.preview_png)?;
            fs::write(sprites_dir.join(format!("{}.meta", preview_name)), &pal.preview_meta_json)?;
            // Write the entity with the paletteMap filled in
            let entity_json = entity_gen::attach_palette(&char_entity_json, &pal.collection_guid, &pal.base_map_id);
            fs::write(entities_dir.join(&char_entity_filename), entity_json)?;
            palette_collection_guid = Some(pal.collection_guid.clone());
            palette_base_map_id = Some(pal.base_map_id.clone());
        }
        Err(e) => {
            log::warn!("palette_gen failed (sprites will have no palette): {}", e);
        }
    }

    // ── menu.entity ────────────────────────────────────────────────────────────────
    if let Some(head) = head_sprite {
        if let Some(ref img_sym) = head.image_symbol {
            // Find the head image in our extracted images
            let head_image = img_result.images.values().find(|img| &img.symbol_name == img_sym);
            if let Some(head_img) = head_image {
                let head_meta_guid = crate::uuid_gen::image_meta_guid(img_sym);
                let menu_info = entity_gen::MenuImageInfo {
                    head_symbol: img_sym.clone(),
                    head_width: head_img.width,
                    head_height: head_img.height,
                    head_meta_guid,
                };
                let mut menu_json = entity_gen::generate_menu_entity(&char_id, &menu_info);
                // Fill in paletteMap if available
                if let (Some(ref cg), Some(ref pm)) = (&palette_collection_guid, &palette_base_map_id) {
                    let mut menu_val: serde_json::Value = serde_json::from_str(&menu_json).unwrap_or(serde_json::json!({}));
                    menu_val["paletteMap"] = serde_json::json!({
                        "paletteCollection": cg,
                        "paletteMap": pm
                    });
                    menu_json = serde_json::to_string_pretty(&menu_val).unwrap_or(menu_json);
                }
                let menu_filename = match multi_char_slot {
                    Some(_) => format!("{}_Menu.entity", char_pascal),
                    None    => "Menu.entity".to_string(),
                };
                fs::write(entities_dir.join(&menu_filename), menu_json)?;
                log::info!("Generated {} using {} ({}x{})", menu_filename, img_sym, head_img.width, head_img.height);
            } else {
                log::warn!("Head image '{}' not found in extracted images, skipping Menu.entity", img_sym);
            }
        } else {
            log::warn!("Head sprite '{}' has no image placement, skipping Menu.entity", head.name);
        }
    } else {
        log::warn!("No head sprite found, skipping Menu.entity");
    }

    // ── projectile files ──────────────────────────────────────────────────
    // Delegated to the summoner-agnostic projectile subsystem so stages/items
    // can emit projectiles the same way (see projectile_gen). The character is
    // just one kind of owner here.
    {
        let scripts_root = char_dir.join("library/scripts");
        // WHICH ONES IT KEEPS. The source fires a projectile and then stores what it just
        // fired, so it can go on steering it -- chibirobo's telly vision sits on the stage
        // for the match and is driven from the character's own code. Read straight off the
        // summoner's script: a spawn whose result it puts somewhere is one it keeps.
        let mut kept = crate::api_mappings::projectiles_the_summoner_keeps(
            &data.scripts.iter().map(|s| s.code.as_str()).collect::<Vec<_>>().join("\n"));
        // an item the fighter generates is in its hand from birth: kept by definition, so it gets
        // no die-on-contact handlers of the port's (its own class says what ends it)
        for p in projectiles {
            if crate::api_mappings::is_generated_item(&p.name) { kept.insert(p.name.clone()); }
        }
        if !kept.is_empty() {
            log::info!("{} projectile(s) the character keeps hold of: {}",
                kept.len(), kept.iter().cloned().collect::<Vec<_>>().join(", "));
        }
        let mut owner_vars: std::collections::BTreeSet<String> =
            data.ext_vars.iter().cloned().collect();
        {
            let abcs = crate::projectile_gen::parse_abcs(parsed_swf);
            for p in projectiles {
                let code: String = crate::projectile_gen::all_behaviour(&abcs, p)
                    .values().cloned().collect::<Vec<_>>().join("\n");
                for n in crate::api_mappings::names_written_on_the_owner(&code) {
                    owner_vars.insert(n);
                }
            }
        }
        let owner_fns: std::collections::BTreeSet<String> = data.scripts.iter()
            .filter(|s| s.is_ext_method && !crate::abc_parser::is_root_xframe_method(&s.name))
            .map(|s| s.name.clone())
            .filter(|n| !data.ext_vars.contains(n))
            .collect();
        let owner_getters: std::collections::BTreeSet<String> = data.getters.iter().cloned().collect();
        let ctx = crate::projectile_gen::ProjectileGenCtx {
            kept_by_summoner: &kept,
            owner_vars: &owner_vars,
            owner_fns: &owner_fns,
            owner_getters: &owner_getters,
            owner_anims: &data.ssf2_to_fm_anim,
            owner_id: &char_id,
            entities_dir: &entities_dir,
            scripts_root: &scripts_root,
            parsed_swf,
            img_result,
            projectile_data: &data.projectile_data,
            palette_collection_guid: palette_collection_guid.as_deref(),
            palette_base_map_id: palette_base_map_id.as_deref(),
            runtime_runs_frames: true,
        };
        crate::projectile_gen::generate_projectiles(&ctx, projectiles)?;
    }
    // the manifest, once the projectiles have said what they carry beside them (a platform a
    // projectile gives itself is a structure content of the package)
    if multi_char_slot.is_none() {
        let proj_names: Vec<String> = projectiles.iter().map(|p| p.name.clone()).collect();
        fs::write(char_dir.join("library/manifest.json"), generate_manifest(&char_id, char_name, &proj_names))?;
    }

    // ── effect .entity files ─────────────────────────────────────────────
    // Per-effect entities (no scripts, no stats, no manifest entries).
    // The character's Script.hx spawns them via match.createVfx(...).
    let mut blank_effects: Vec<String> = Vec::new();
    let global_vfx = &crate::mappings::api_commands().global_vfx_map;
    for effect in effects {
        // a shared effect plays this engine's own global one: nothing of the package's copy is
        // shipped
        if global_vfx.contains_key(&effect.name) { continue }
        let filename = format!("{}.entity", effect.name);
        // an imported effect is built from the package it came from, not this character's file
        let entity_json = match imported.iter().find(|(e, _)| e.name == effect.name) {
            Some((_, json)) => json.clone(),
            None => entity_gen::generate_effect_entity(&char_id, effect, img_result, parsed_swf),
        };
        // Surface effects that still rasterized to nothing (no image symbols) —
        // typically Kirby copy-ability hats / placeholders whose visuals are
        // built from deeply-nested named sprites we don't yet flatten.
        // Count IMAGE *symbols* (which carry "imageAsset"), not the IMAGE
        // *layer* type — a blank effect still has an Image Layer.
        let image_symbol_count = entity_json.matches("\"imageAsset\"").count();
        if image_symbol_count == 0 && effect.frame_count > 0 {
            blank_effects.push(effect.name.clone());
            log::warn!("Effect '{}' has no extractable images ({} frames) — renders blank in FrayTools (no vector/bitmap content found to rasterize)", effect.name, effect.frame_count);
        }
        note_entity_written(&entities_dir.join(&filename));
        fs::write(entities_dir.join(&filename), entity_json)?;
        let anim_names = entity_gen::effect_animation_names(effect);
        log::info!(
            "Generated effect entity: {} ({} frames, animations: [{}], {} image symbols)",
            filename,
            effect.frame_count,
            anim_names.join(", "),
            image_symbol_count,
        );
    }
    if !blank_effects.is_empty() {
        log::warn!("{} effect(s) render blank (no extractable art): {}", blank_effects.len(), blank_effects.join(", "));
    }

    // Stats summary for debugging
    let stats_json = serde_json::json!({
        "char_id": char_id,
        "display_name": char_name,
        "attacks_extracted": data.attacks.len(),
        "stats_extracted": data.stats.weight != 0.0,
        "animations": data.animations.len(),
        "frame_scripts": data.scripts.len(),
        "ssf2_to_fm_anim": data.ssf2_to_fm_anim,
        // each animation the conversion cut, by the animation whose frames it shows
        "splits": splits.iter().map(|s| (s.fm_name.clone(), serde_json::Value::String(s.source_anim.clone())))
            .collect::<serde_json::Map<_, _>>(),
        // the throws drawn and launched already turned (entity_gen::turned_throws)
        "turned": entity_gen::turned_throws(data),
    });
    fs::write(char_dir.join("conversion_stats.json"), serde_json::to_string_pretty(&stats_json)?)?;
    // The plan the verify gates hold the published fighter to: the moves it was converted with,
    // the scripts its package must carry, how much art it ships. Written from the same values
    // that were emitted, so there is one statement of what the port is meant to be.
    {
        let mut scripts: Vec<String> = Vec::new();
        let (mut images, mut bytes) = (0usize, 0u64);
        fn walk(dir: &std::path::Path, scripts: &mut Vec<String>, images: &mut usize, bytes: &mut u64) {
            let Ok(rd) = fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() { walk(&p, scripts, images, bytes); continue; }
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.ends_with(".hx.meta") {
                    if let Some(sid) = fs::read_to_string(&p).ok()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                        .and_then(|v| v["id"].as_str().map(str::to_string)) { scripts.push(sid); }
                } else if name.ends_with(".png") {
                    *images += 1;
                    *bytes += fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                }
            }
        }
        walk(&char_dir.join("library"), &mut scripts, &mut images, &mut bytes);
        scripts.sort();
        let plan = serde_json::json!({
            "id": char_id,
            "moves": data.ssf2_to_fm_anim.values().collect::<std::collections::BTreeSet<_>>(),
            "scripts": scripts,
            "assets": { "images": images, "bytes": bytes },
        });
        fs::write(char_dir.join("verify.json"), serde_json::to_string_pretty(&plan)?)?;
    }

    // ── Sound content entries ─────────────────────────────────────────────────────────────────
    if !sounds.is_empty() {
        // Mirrors main::process_character's per-char-subdir rule for
        // multi-char projects (plan §3); flat path for single-char.
        let audio_dir = match multi_char_slot {
            Some(_) => char_dir.join(format!("library/audio/{}", char_id)),
            None    => char_dir.join("library/audio"),
        };
        generate_sound_entries(&audio_dir, char_name, sounds)?;
        log::info!("Generated sound entries for {} sounds", sounds.len());
    }

    log::info!("Generated: {} attacks, {} animations, {} frame scripts",
        data.attacks.len(), data.animations.len(), data.scripts.len());
    Ok(())
}

// ─── SSF2 → Fraymakers stat scaling ─────────────────────────────────────────
// SSF2 uses pixel-per-frame units at 60fps.
// Fraymakers uses its own unit system. Approximate conversions based on
// studying template characters vs SSF2 data.

// Scaling factors are loaded from mappings/character/stats.json (see crate::mappings).
fn ssf2_gravity_to_fm(v: f64) -> f64 { crate::mappings::character_stats().scale("gravity", v) }
fn ssf2_speed_to_fm(v: f64) -> f64   { crate::mappings::character_stats().scale("speed", v) }
fn ssf2_jump_to_fm(v: f64) -> f64    { crate::mappings::character_stats().scale("jump", v) }
fn ssf2_walk_to_fm(v: f64) -> f64    { crate::mappings::character_stats().scale("walk", v) }
fn ssf2_dash_to_fm(v: f64) -> f64    { crate::mappings::character_stats().scale("dash", v) }
fn ssf2_air_to_fm(v: f64) -> f64     { crate::mappings::character_stats().scale("air_friction", v.abs()) }

fn fmt(v: f64) -> String {
    if v == v.round() && v.abs() < 1000.0 {
        format!("{}", v as i64)
    } else {
        // four places: two rounded the small stats away (wario man's air friction 0.003 came out
        // 0, kirby's gravity 0.276 as 0.28)
        format!("{:.4}", v).trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

// ─── HitboxStats.hx ─────────────────────────────────────────────────────────

fn generate_hitbox_stats(data: &CharacterData, _char_id: &str, splits: &[crate::anim_splitter::SplitAnim]) -> String {
    let attack_lookup: std::collections::BTreeMap<_, _> = data.attacks.iter()
        .map(|a| (a.name.as_str(), a))
        .collect();

    let t = &crate::mappings::script_templates().character.stats;
    let req = crate::mappings::require_template;
    let mut out = req("character.stats.hitbox_header", &t.hitbox_header)
        .replace("{{char_name}}", &data.name);

    let tables = crate::mappings::character_stats_tables();
    // a throw drawn already turned launches the mirrored way, as the source's turned thrower does
    let turned = crate::entity_gen::turned_throws(data);
    let standard: std::collections::HashSet<&str> = tables.hitbox_sections.iter()
        .flat_map(|s| s.moves.iter().map(|m| m.as_str())).collect();

    for sec in &tables.hitbox_sections {
        out.push_str(&req("character.stats.hitbox_section_comment", &t.hitbox_section_comment)
            .replace("{{section}}", &sec.section));
        for move_name in &sec.moves {
            let move_name = move_name.as_str();
            // SSF2 stores a split combo (jab1/2/3, strong _in/_charge/_attack) as ONE
            // attack keyed by the base move; the FM timeline splits it into several named
            // sub-animations. Direct lookup only finds the base name, so fall back to it
            // for the sub-names — otherwise jab2/jab3 etc. emit 0-damage placeholders and
            // are non-functional in-engine. (Per-segment fidelity is a deeper follow-up;
            // inheriting the base attack's hitboxes is far closer to SSF2 than zero.)
            let direct = attack_lookup.get(move_name).copied();
            let base = base_attack_name(move_name);
            let inherited = direct.is_none() && base.is_some();
            let found = direct.or_else(|| base.as_deref().and_then(|b| attack_lookup.get(b).copied()));
            if let Some(attack) = found {
                out.push_str(&format_attack(move_name, &attack.hitboxes, false,
                    if inherited { base.as_deref() } else { None }, turned.contains(move_name)));
            } else if move_name == "emote" {
                out.push_str(req("character.stats.hitbox_emote", &t.hitbox_emote));
            } else {
                out.push_str(&format_attack_todo(move_name));
            }
        }
    }

    // Extra attacks from SSF2 that don't map to standard moves
    let extras: Vec<_> = data.attacks.iter()
        .filter(|a| !standard.contains(a.name.as_str())).collect();
    if !extras.is_empty() {
        out.push_str(req("character.stats.hitbox_extras_header", &t.hitbox_extras_header));
        for attack in &extras {
            out.push_str(&format_attack(&attack.name, &attack.hitboxes, true, None, turned.contains(attack.name.as_str())));
        }
    }
    // an animation the timeline was cut into is the same attack as the one it was cut from (the
    // source keys the attack by its label): pikachu's skull bash flies on `special_side_attack`,
    // cut from `special_side`, and its box hit for nothing with no stats of its own
    let mut named: std::collections::HashSet<&str> = standard.clone();
    named.extend(extras.iter().map(|a| a.name.as_str()));
    for split in splits {
        let n = split.fm_name.as_str();
        if named.contains(n) || n == split.source_anim { continue }
        if let Some(attack) = attack_lookup.get(split.source_anim.as_str()).copied() {
            out.push_str(&format_attack(n, &attack.hitboxes, false, Some(split.source_anim.as_str()), turned.contains(n)));
            named.insert(n);
        }
    }

    out.push_str(req("character.stats.hitbox_footer", &t.hitbox_footer));
    let out = strip_trailing_commas(&out);
    // ERROR HANDLER for broken hitbox stats: the engine parses HitboxStats as one hscript object
    // literal; a single malformed entry (e.g. a half-commented attack block) breaks the WHOLE parse
    // → null stats → the character silently deals no damage (hurtboxes still work, so nothing
    // crashes — a nasty silent failure). Validate the generated object here so a converter bug
    // surfaces loudly at convert time instead of as dead hitboxes in-engine.
    assert_balanced_stats(&out, &format!("HitboxStats for {}", data.name));
    out
}

/// Fail the conversion (loudly, in peptide) if a generated stats object is not a syntactically
/// balanced hscript object literal — the failure mode that produces dead hitboxes in-engine.
/// Counts `{}`/`[]` ignoring `//` line comments (string literals in these files never contain
/// braces). A mismatch means an attack block is malformed and the engine's parse would fail.
fn assert_balanced_stats(s: &str, context: &str) {
    let mut curly: i64 = 0;
    let mut square: i64 = 0;
    for line in s.lines() {
        let code = match line.find("//") { Some(i) => &line[..i], None => line };
        for c in code.chars() {
            match c {
                '{' => curly += 1,
                '}' => curly -= 1,
                '[' => square += 1,
                ']' => square -= 1,
                _ => {}
            }
        }
        if curly < 0 || square < 0 {
            panic!("broken stats object ({context}): unbalanced braces — an attack block is \
                    malformed (a stray/closing brace). The engine would fail to parse this and the \
                    character would deal no damage. Offending line: {:?}", line.trim());
        }
    }
    if curly != 0 || square != 0 {
        panic!("broken stats object ({context}): unbalanced braces at end \
                (curly={curly}, square={square}) — a malformed attack block. The engine would fail \
                to parse this and the character would deal no damage.");
    }
}

/// Remove trailing commas inside object literals — any `,` that is followed only by
/// whitespace and then a `}`. The hitbox-stats templates append `},` after every hitbox
/// and every move, which leaves a trailing comma before the closing brace of the last
/// element. hscript's object-literal parser REJECTS trailing commas, so the whole
/// HitboxStats expression fails to parse → null stats → hitboxes get no stats and never
/// deal damage (hurtboxes need no stats, so they still work — which is exactly the
/// "converted chars can be hit but can't hit" bug). This pass makes the output valid.
/// Only touches structural trailing commas; commas BETWEEN fields are untouched.
fn strip_trailing_commas(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b',' {
            let mut j = i + 1;
            while j < b.len() && (b[j] as char).is_whitespace() { j += 1; }
            if j < b.len() && b[j] == b'}' {
                i += 1; // drop the comma, keep the whitespace that follows
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

fn guess_limb(move_name: &str) -> String {
    // Ordered substring rules from mappings/character/stats_tables.jsonc;
    // first match wins, else the default.
    let lr = &crate::mappings::character_stats_tables().limb_rules;
    for rule in &lr.rules {
        if rule.contains.iter().any(|s| move_name.contains(s.as_str())) {
            return rule.limb.clone();
        }
    }
    lr.default.clone()
}

/// Base move that carries the SSF2 hitbox stats for a split sub-animation name,
/// or None if `move_name` isn't a split sub-name. SSF2 keeps the whole combo as a
/// single attack under the base name; the FM split produces extra names that must
/// inherit those stats. Kept in lockstep with the anim_splitter's split rules.
fn base_attack_name(move_name: &str) -> Option<String> {
    // jabN (N >= 2) inherits jab1 (the SSF2 `a` attack carries the combo's hitboxes).
    if let Some(rest) = move_name.strip_prefix("jab") {
        if rest.parse::<u32>().map(|n| n >= 2).unwrap_or(false) {
            return Some("jab1".to_string());
        }
    }
    // strong_*_in / strong_*_charge inherit strong_*_attack (the hit-bearing phase,
    // which is the name normalize_attack_name emits for the SSF2 strong attack).
    for suf in ["_in", "_charge"] {
        if let Some(stem) = move_name.strip_suffix(suf) {
            if stem.starts_with("strong_") {
                return Some(format!("{stem}_attack"));
            }
        }
    }
    None
}

fn format_attack(name: &str, hitboxes: &[Hitbox], is_extra: bool, inherited_from: Option<&str>, turned: bool) -> String {
    let t = &crate::mappings::script_templates().character.stats;
    let req = crate::mappings::require_template;
    let limb = guess_limb(name);
    // Always build the block with a real tab prefix; an `is_extra` block is commented out WHOLE
    // below. (The old code only commented the opening `// SSF2: name: {` line and left the hitbox
    // body active+keyless — a syntax error that broke the ENTIRE HitboxStats object parse, so NO
    // hitboxes got stats and converted characters couldn't deal damage. Hurtboxes need no stats,
    // so they kept working — which is exactly the observed bug.)
    let prefix = "\t";
    let mut out = String::new();
    if let Some(base) = inherited_from {
        // Make the inheritance explicit so a modder knows these values were copied
        // from the base move (SSF2 stored the combo as one attack) and may want
        // per-hit tuning.
        out.push_str(&format!("\t// stats inherited from {base} (SSF2 stores this combo as one attack — tune per-hit if needed)\n"));
    }
    out.push_str(&req("character.stats.hitbox_attack_open", &t.hitbox_attack_open)
        .replace("{{prefix}}", prefix).replace("{{name}}", name));
    // Frame-count fields are doubled for the 30fps -> 60fps timing change.
    // Which fields are frame counts is driven by the `isframe` flags in
    // mappings/character/hitbox_stats.json (see crate::mappings), the same
    // mechanism commands.json uses — so all frame-doubling is one path.
    let hb_cfg = crate::mappings::character_hitbox_stats();
    // a frame field gains whatever the mapping says the source leaves out and is then doubled
    // for 30 -> 60fps
    let scale = |fm_field: &str, v: i32| if hb_cfg.is_frame(fm_field) {
        ((f64::from(v) + hb_cfg.plus_frames_for(fm_field)) * 2.0).round() as i32
    } else { v };
    // A knockback value carries a measured factor between the engines (see hitbox_stats.jsonc).
    // Kept fractional, the way the projectile and hazard paths keep it: rounding a scaled value
    // to a whole number is worth about 2% of a light hit's launch, and the factor itself is
    // measured to better than 1%.
    let kb = |fm_field: &str, v: f64| fmt_num(v * hb_cfg.scale_for(fm_field));
    for (i, hb) in hitboxes.iter().enumerate() {
        // The -1 "no override" sentinel (and SSF2's 255, which maps to it) is
        // emit formatting and stays in code; only the doubling is data-driven.
        // A hitstun that is not a positive whole number of frames is not a duration at all (SSF2
        // states this one as a multiplier), so the engine is left to work the hitstun out from
        // the knockback, which is what both engines do by default.
        let hitstun = if hb.hitstun <= 0 || hb.hitstun == 255 { -1 } else { scale("hitstun", hb.hitstun) };
        // each one's freeze: what the source states (a count of frames, zero included), else what
        // the source works out from the damage (see hitbox_stats.jsonc), else the engine's own
        let counted = |f: &str| hb_cfg.keys_for(f).iter()
            .any(|k| hb.raw.get(k.as_str()).is_some_and(|v| *v >= 0.0 && *v != 255.0));
        let hitstop = if counted("hitstop") {
            scale("hitstop", hb.hitstop)
        } else if let Some(f) = hb_cfg.unstated_freeze("hitstop", hb.damage) {
            (f * 2.0).round() as i32
        } else { -1 };
        let self_stated = hb_cfg.keys_for("selfHitstop").iter().any(|k| hb.raw.contains_key(k.as_str()));
        let self_hitstop = if self_stated && hb.self_hitstop >= 0 && hb.self_hitstop != 255 {
            scale("selfHitstop", hb.self_hitstop)
        } else if let Some(f) = hb_cfg.unstated_freeze("selfHitstop", hb.damage) {
            (f * 2.0).round() as i32
        } else { -1 };

        // what the box states true or false that this engine has a field for (hitbox_stats.jsonc
        // `flags`): one that names a field written here is that field's value (a box with no
        // effect launches nothing), and the rest follow the fields
        let flags = hb_cfg.flag_fields(&hb.raw);
        let flag = |f: &str| flags.iter().find(|(k, _)| k == f).map(|(_, v)| v.clone());
        out.push_str(&req("character.stats.hitbox_field_main", &t.hitbox_field_main)
            .replace("{{i}}", &i.to_string())
            // kept FRACTIONAL, like the knockback beside it: the source states halves (zelda's
            // down throw does 0.5) and cutting one to a whole number does not just lose the
            // damage, it moves the LAUNCH, because both engines work a launch out from what the
            // hit actually did
            .replace("{{damage}}", &fmt_num(hb.damage))
            .replace("{{angle}}", &(if turned { (180.0 - hb.angle).rem_euclid(360.0) } else { fm_angle(hb.angle, reversible(&hb.raw)) } as i32).to_string())
            .replace("{{base_knockback}}", &flag("baseKnockback").unwrap_or_else(|| kb("baseKnockback", hb.base_knockback)))
            .replace("{{knockback_growth}}", &flag("knockbackGrowth").unwrap_or_else(|| kb("knockbackGrowth", hb.knockback_growth)))
            .replace("{{hitstop}}", &flag("hitstop").unwrap_or_else(|| hitstop.to_string()))
            .replace("{{self_hitstop}}", &flag("selfHitstop").unwrap_or_else(|| self_hitstop.to_string())));
        if hitstun != -1 {
            out.push_str(&req("character.stats.hitbox_field_hitstun", &t.hitbox_field_hitstun)
                .replace("{{hitstun}}", &hitstun.to_string()));
        }
        // a box the source states is not reversible says so (the engine's are, as the source's are
        // unless stated)
        if turned || !reversible(&hb.raw) { out.push_str(", reversibleAngle: false"); }
        // the other flags: a knockback that does not stack, a forced tumble, no DI, through shields, ...
        const WRITTEN: [&str; 4] = ["baseKnockback", "knockbackGrowth", "hitstop", "selfHitstop"];
        for (field, value) in flags.iter().filter(|(f, _)| !WRITTEN.contains(&f.as_str())) { out.push_str(&format!(", {field}: {value}")); }
        // only when the source states it: left out, the engine uses its own
        if hb.shield_stun_multiplier > 0.0 {
            out.push_str(&format!(", shieldstunMultiplier: {}", fmt_num(hb.shield_stun_multiplier)));
        }
        out.push_str(&req("character.stats.hitbox_field_close", &t.hitbox_field_close)
            .replace("{{limb}}", &limb));
    }
    out.push_str(req("character.stats.hitbox_attack_close", &t.hitbox_attack_close));
    if is_extra {
        // Comment the ENTIRE block — an SSF2 extra has no FM move name, so it's documentation
        // only. Commenting every line keeps it inert (and syntactically valid).
        out = out.lines()
            .map(|l| format!("\t// SSF2: {}", l.trim_start()))
            .collect::<Vec<_>>()
            .join("\n");
        out.push('\n');
    }
    out
}

/// whether a source box is reversible: stated, else the source's default (it is)
pub fn reversible(raw: &std::collections::BTreeMap<String, f64>) -> bool {
    raw.get("reversableAngle").is_none_or(|v| *v != 0.0)
}

/// a source box's angle as this engine takes it: as the source writes it. measured on every
/// grounded move of the roster (`tools/tests/hit_parity.py`), the source sends a reversible
/// angle against a victim in front as stated, a backward one included (mario's, pit's and
/// marth's up tilts lean back at 96 to 110 degrees: eighteen of eighteen), and this engine's
/// reversible box mirrors it only for a victim behind, as the source's does
pub fn fm_angle(d: f64, _reversible: bool) -> f64 {
    d
}

/// The fields a partial box write sets, as HitboxStats states them: only the ones its keys name,
/// each converted as the move's own boxes are (the measured knockback factors, a freeze's frames
/// gaining what the source leaves out and doubled for the engine's rate)
pub fn hitbox_override_fields(raw: &std::collections::BTreeMap<String, f64>) -> Vec<(String, String)> {
    let cfg = crate::mappings::character_hitbox_stats();
    let pick = |fm: &str| cfg.keys_for(fm).iter().filter_map(|k| raw.get(k.as_str()).copied()).reduce(f64::max);
    let frames = |fm: &str, v: f64| if v <= 0.0 || v == 255.0 { -1 } else { ((v + cfg.plus_frames_for(fm)) * 2.0).round() as i32 };
    let fixed = crate::mappings::HitboxStatsMapping::is_fixed_knockback(raw);
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(v) = pick("damage") { out.push(("damage".into(), fmt_num(v))); }
    if let Some(v) = pick("angle") { out.push(("angle".into(), (fm_angle(v, reversible(raw)) as i32).to_string())); }
    if let Some(r) = raw.get("reversableAngle") { out.push(("reversibleAngle".into(), (*r != 0.0).to_string())); }
    if let Some(v) = cfg.fixed_knockback(raw).or_else(|| pick("baseKnockback")) {
        out.push(("baseKnockback".into(), fmt_num(v * cfg.scale_for("baseKnockback"))));
    }
    if fixed { out.push(("knockbackGrowth".into(), "0".into())); }
    else if let Some(v) = pick("knockbackGrowth") { out.push(("knockbackGrowth".into(), fmt_num(v * cfg.scale_for("knockbackGrowth")))); }
    for f in ["hitstop", "selfHitstop", "hitstun"] {
        let Some(v) = pick(f) else { continue };
        // a freeze written as no count of frames is the one the source works out from the damage
        let worked = (v <= 0.0 || v == 255.0).then(|| pick("damage").and_then(|d| cfg.unstated_freeze(f, d))).flatten();
        let n = match worked { Some(w) => (w * 2.0).round() as i32, None => frames(f, v) };
        out.push((f.into(), n.to_string()));
    }
    if let Some(v) = pick("shieldstunMultiplier").filter(|v| *v > 0.0) { out.push(("shieldstunMultiplier".into(), fmt_num(v))); }
    // last, as a box's own fields are: a flag that states a field (a box with no effect's launch)
    // is the one that holds
    out.extend(cfg.flag_fields(raw));
    out
}

fn format_attack_todo(name: &str) -> String {
    let limb = guess_limb(name);
    crate::mappings::require_template(
        "character.stats.hitbox_todo_attack",
        &crate::mappings::script_templates().character.stats.hitbox_todo_attack,
    ).replace("{{name}}", name).replace("{{limb}}", &limb)
}

// ─── CharacterStats.hx ───────────────────────────────────────────────────────

fn generate_character_stats(data: &CharacterData, char_id: &str) -> String {
    let s = &data.stats;
    // flat constants + template physics defaults come from stats.jsonc.
    let cfg = crate::mappings::character_stats();
    let c = |k: &str| cfg.constant(k);
    let c_num = |k: &str| c(k).parse::<f64>().unwrap_or(0.0);

    // Convert SSF2 values to Fraymakers equivalents (scaling driven by the multipliers in
    // stats.jsonc). When SSF2 LACKS a value, fall back to the CHARACTER-TEMPLATE DEFAULT
    // (not 0) so a converted character is never left with 0 weight/gravity/etc. — those
    // zeros caused null/degenerate physics in-engine. The annotation flags template-sourced
    // values so an author can see what wasn't derived from SSF2.
    let dflt = " /*template default*/";
    let (gravity, gravity_todo)            = if s.gravity > 0.0          { (ssf2_gravity_to_fm(s.gravity), "") }       else { (c_num("gravity"), dflt) };
    let (terminal_vel, terminal_vel_todo)  = if s.fall_speed > 0.0       { (ssf2_speed_to_fm(s.fall_speed), "") }      else { (c_num("terminalVelocity"), dflt) };
    let (fast_fall, fast_fall_todo)        = if s.fast_fall_speed > 0.0  { (ssf2_speed_to_fm(s.fast_fall_speed), "") } else { (c_num("fastFallSpeed"), dflt) };
    // A LAUNCH SPEED (a jump, a double jump, a short hop) crosses with one step of gravity taken
    // off. The source subtracts a frame's gravity before it moves the fighter, this engine moves
    // it first; over the two engine frames of a source frame that is three quarters of a source
    // frame's gravity, 1.5 of this engine's (the same step `__svy` takes off a y speed a script
    // writes). Scaled alone, every jump rose an extra frame of its own speed: mario's full jump
    // 158 px against the source's 140, his short hop 48 against 39.
    let launch = |v: f64| if s.gravity > 0.0 { (v - 1.5 * gravity).max(0.0) } else { v };
    let (jump_speed, jump_speed_todo)      = if s.jump_height > 0.0      { (launch(ssf2_jump_to_fm(s.jump_height)), "") }      else { (c_num("jumpSpeed"), dflt) };
    let dj_speed      = if s.double_jump_height > 0.0 { launch(ssf2_jump_to_fm(s.double_jump_height)) } else { 0.0 };
    let (walk_cap, walk_cap_todo)          = if s.walk_speed > 0.0       { (ssf2_walk_to_fm(s.walk_speed), "") }       else { (c_num("walkSpeedCap"), dflt) };
    let (dash_speed, dash_speed_todo)      = if s.dash_speed > 0.0       { (ssf2_dash_to_fm(s.dash_speed), "") }       else { (c_num("dashSpeed"), dflt) };
    let (aerial_fric, aerial_fric_todo)    = if s.air_friction != 0.0    { (ssf2_air_to_fm(s.air_friction), "") }      else { (c_num("aerialFriction"), dflt) };
    let (weight, weight_todo)              = if s.weight > 0.0           { (s.weight, "") }                            else { (c_num("weight"), dflt) };

    // Derived stats — expression strings in stats.jsonc, evaluated with the already-converted
    // stats as variables. short_hop derives from jump_speed (template-defaulted above) so it
    // is always sane; aerial_cap derives from SSF2 air mobility/friction, so if SSF2 had
    // NEITHER, fall back to the template default rather than a near-zero derived value.
    let vars: std::collections::BTreeMap<String, f64> = [
        ("jump_speed".to_string(), jump_speed),
        ("air_mobility_raw".to_string(), s.air_mobility),
        ("aerial_friction".to_string(), aerial_fric),
    ].into_iter().collect();
    let short_hop_d  = crate::mappings::evaluate_stat_derivation("shortHopSpeed", &vars).unwrap_or(0.0);
    let aerial_cap_d = crate::mappings::evaluate_stat_derivation("aerialSpeedCap", &vars).unwrap_or(0.0);
    // shortHopSpeed: prefer the REAL SSF2 value (velocity-scaled). Only fall
    // back to the jump-derived estimate / template constant when SSF2 lacked it.
    let (short_hop, short_hop_todo) = if s.short_hop_speed > 0.0 {
        (launch(ssf2_speed_to_fm(s.short_hop_speed)), "")
    } else if short_hop_d > 0.0 {
        (short_hop_d, " /*TODO: estimated from jumpSpeed; SSF2 had no shortHopSpeed*/")
    } else {
        (c_num("shortHopSpeed"), dflt)
    };
    // HOW FAST IT MAY TRAVEL IN THE AIR. The source states this outright, under a name that
    // sounds like a jump: read the engine frame by frame and mario holds at exactly the 7 it
    // states, fox at its 7.3, kirby at its 6.2. It was being worked out from air mobility
    // instead, five times a number that has nothing to do with it, which came to 3.5 for
    // every character regardless of what they said about themselves.
    let (aerial_cap, aerial_fric_cap_todo) = if s.air_speed_cap > 0.0 {
        (ssf2_speed_to_fm(s.air_speed_cap), "")
    } else if s.air_mobility != 0.0 || s.air_friction != 0.0 {
        (aerial_cap_d, "")
    } else {
        (c_num("aerialSpeedCap"), dflt)
    };

    // ── Real SSF2 movement constants, previously hardcoded template values ──
    // Now derived from the source (verified against docs/ssf2-physics-model.md)
    // so walk/run/jump feel matches SSF2. Accelerations use accel_scale; speeds
    // use velocity_scale (see mappings::scale). The single size_multiplier knob
    // governs both.
    let sm = crate::mappings::character_stats();
    // friction := |decel_rate| (grounded). SSF2 stores it negative.
    let (friction, friction_todo) = if s.ground_friction != 0.0 {
        (sm.scale("friction", s.ground_friction.abs()), "")
    } else {
        (c_num("friction"), " /*TODO*/")
    };
    // SSF2 has ONE grounded accel (accel_rate) feeding both walk and run/ground.
    let (walk_accel, walk_accel_todo) = if s.ground_accel > 0.0 {
        (sm.scale("walk_accel", s.ground_accel), "")
    } else {
        (c_num("walkSpeedAcceleration"), "")
    };
    let ground_accel_v = if s.ground_accel > 0.0 { walk_accel } else { c_num("groundSpeedAcceleration") };
    let run_accel_v    = if s.ground_accel > 0.0 { walk_accel } else { c_num("runSpeedAcceleration") };
    // SSF2 accel_rate_air (carried in air_mobility) → aerial horizontal accel.
    let (aerial_accel, aerial_accel_todo) = if s.air_mobility != 0.0 {
        (sm.scale("air_accel", s.air_mobility), "")
    } else {
        (c_num("aerialSpeedAcceleration"), "")
    };
    // Initial walk/dash speeds: accel_start / accel_start_dash are MULTIPLIERS on
    // the respective speed cap (m_charRun: xSpeed = AccelStart*norm_xSpeed, dash
    // = AccelStartDash*max_xSpeed). Convert the resulting absolute kick, then
    // clamp into a sane [0, cap] band (SSF2 re-clamps to the cap next frame; some
    // dummies carry a degenerate >cap kick value).
    // this engine starts a walk a frame later than the source and moves before it accelerates,
    // so the kick carries one and a half frames of its acceleration (megaman measured 8% short
    // over a fifteen-frame walk without it; a script's own grounded x write takes the same step)
    let walk_initial = if s.walk_initial > 0.0 && s.walk_speed > 0.0 {
        (ssf2_speed_to_fm(s.walk_initial * s.walk_speed) + 1.5 * walk_accel).min(walk_cap).max(0.0)
    } else { c_num("walkSpeedInitial") };
    let run_initial = if s.dash_initial > 0.0 && s.dash_speed > 0.0 {
        ssf2_speed_to_fm(s.dash_initial * s.dash_speed).min(dash_speed).max(0.0)
    } else { c_num("runSpeedInitial") };
    // runSpeedCap mirrors the dash cap (SSF2 max_xSpeed) when present.
    let run_cap = if s.dash_speed > 0.0 { dash_speed } else { c_num("runSpeedCap") };
    // HOW FAST THE GROUND LETS IT GO. The source caps ground speed at the one number it
    // states for it, and that number is the character's own -- mario 11, fox 13.7, sandbag
    // 8. A flat default here gave all three the same ceiling, and it was one character's
    // raw figure at that: unscaled, so every character was also allowed half again more
    // than the port intends. It is the same cap the run already uses.
    let ground_cap = if s.dash_speed > 0.0 { dash_speed } else { c_num("groundSpeedCap") };

    // doubleJumpSpeeds: the real converted value, or the character-template default.
    let dj_array = if dj_speed > 0.0 {
        format!("[{}]", fmt(dj_speed))
    } else {
        format!("[{}]{}", c("doubleJumpSpeedFallback"), dflt)
    };

    // Transformation banner: when this character was extracted from a
    // Main::get<X>() bundle whose cData.normalStats_id ≠ the derived id
    // (Giga Bowser, Wario Man), prepend a TODO note. Fraymakers has no
    // native transformation hook; the form is emitted as a standalone
    // character and the content author must wire the trigger by hand.
    let t = &crate::mappings::script_templates().character.stats;
    let req = crate::mappings::require_template;

    let transformation_banner = if let Some(df) = &data.derived_from {
        req("character.stats.char_stats_banner", &t.char_stats_banner)
            .replace("{{parent}}", &df.parent_normal_stats_id)
            .replace("{{source_method}}", &df.source_method)
    } else {
        String::new()
    };

    // aerial_cap is derived from air_mobility_raw + aerial_friction; the TODO
    // marker must reflect whether the DERIVATION result is zero, not whether
    // `s.air_mobility` happens to be (fixes §3.10).
    let mut out = transformation_banner;
    out.push_str(&req("character.stats.char_stats_main", &t.char_stats_main)
        .replace("{{char_name}}", &data.name)
        .replace("{{char_id}}", char_id)
        .replace("{{base_scale_x}}", &fmt(s.base_scale_x))
        .replace("{{base_scale_y}}", &fmt(s.base_scale_y))
        .replace("{{weight}}", &fmt(weight)).replace("{{weight_todo}}", weight_todo)
        .replace("{{gravity}}", &fmt(gravity)).replace("{{gravity_todo}}", gravity_todo)
        .replace("{{short_hop}}", &fmt(short_hop)).replace("{{short_hop_todo}}", short_hop_todo)
        .replace("{{jump_speed}}", &fmt(jump_speed)).replace("{{jump_speed_todo}}", jump_speed_todo)
        .replace("{{dj_array}}", &dj_array)
        .replace("{{terminal_vel}}", &fmt(terminal_vel)).replace("{{terminal_vel_todo}}", terminal_vel_todo)
        .replace("{{fast_fall}}", &fmt(fast_fall)).replace("{{fast_fall_todo}}", fast_fall_todo)
        .replace("{{friction}}", &fmt(friction)).replace("{{friction_todo}}", friction_todo)
        .replace("{{walk_speed_initial}}", &fmt(walk_initial))
        .replace("{{walk_speed_accel}}", &fmt(walk_accel)).replace("{{walk_speed_accel_todo}}", walk_accel_todo)
        .replace("{{walk_cap}}", &fmt(walk_cap)).replace("{{walk_cap_todo}}", walk_cap_todo)
        .replace("{{dash_speed}}", &fmt(dash_speed)).replace("{{dash_speed_todo}}", dash_speed_todo)
        .replace("{{run_speed_initial}}", &fmt(run_initial))
        .replace("{{run_speed_accel}}", &fmt(run_accel_v))
        .replace("{{run_speed_cap}}", &fmt(run_cap))
        .replace("{{ground_speed_accel}}", &fmt(ground_accel_v))
        .replace("{{ground_speed_cap}}", &fmt(ground_cap))
        .replace("{{aerial_speed_accel}}", &fmt(aerial_accel)).replace("{{aerial_speed_accel_todo}}", aerial_accel_todo)
        .replace("{{aerial_cap}}", &fmt(aerial_cap)).replace("{{aerial_cap_todo}}", aerial_fric_cap_todo)
        .replace("{{aerial_fric}}", &fmt(aerial_fric)).replace("{{aerial_fric_todo}}", aerial_fric_todo));

    // Flat-constant sections — every value comes from stats.json `constants`.
    out.push_str(&req("character.stats.char_stats_section_comment", &t.char_stats_section_comment).replace("{{title}}", "ENVIRONMENTAL COLLISION BODY (ECB) STATS"));
    let tables = crate::mappings::character_stats_tables();
    // The collision body the character STATES for itself. Every SSF2 character declares its own
    // (kirby 40x30, mario 45x26, bowser 71x60); read from the template instead they all stood in
    // one box, so kirby and bowser had the same reach and the same silhouette to hit. The template
    // still answers for what the source never states (how far the body hangs below the origin in
    // the air, and the hip offsets).
    //
    // IN THE ENTITY'S OWN UNITS, not the world's: the engine scales a character's whole body by
    // its baseScale, so these are stated the same way the boxes in the entity are. Measured live
    // -- mario's ECB reads 69.5 tall in the world for the 48.6 his stand frame states, which is
    // exactly his 1.43. Scaled here as well it would be applied twice.
    //
    // The per-frame body in the entity, fitted to each frame's hurtboxes, is what the engine
    // actually stands on; this is the character's DEFAULT, for the frames that state none.
    let head = (s.body_height > 0.0).then_some(s.body_height);
    let hipw = (s.body_width > 0.0).then_some(s.body_width);
    for fa in &tables.stats_fields.ecb {
        let declared = match fa.field.as_str() {
            "floorHeadPosition" | "aerialHeadPosition" => head,
            "floorHipWidth" | "aerialHipWidth" => hipw,
            _ => None,
        };
        let (value, anno) = match declared {
            Some(v) => (format!("{:.0}", v), String::new()),
            None => (c(&fa.field), fa.anno.clone()),
        };
        out.push_str(&req("character.stats.char_stats_field_anno", &t.char_stats_field_anno).replace("{{field}}", &fa.field).replace("{{value}}", &value).replace("{{anno}}", &anno));
    }
    out.push('\n');

    if !tables.stats_fields.camera.is_empty() { out.push_str(&req("character.stats.char_stats_section_comment", &t.char_stats_section_comment).replace("{{title}}", "CAMERA BOX STATS")); }
    for f in tables.stats_fields.camera.iter().map(|s| s.as_str()) {
        out.push_str(&req("character.stats.char_stats_field_plain", &t.char_stats_field_plain).replace("{{field}}", f).replace("{{value}}", &c(f)));
    }
    out.push('\n');

    if !tables.stats_fields.roll.is_empty() { out.push_str(&req("character.stats.char_stats_section_comment", &t.char_stats_section_comment).replace("{{title}}", "ROLL AND LEDGE JUMP STATS")); }
    // The source's roll speed answers for the getup, tech and ledge rolls fraymakers names -- the
    // same way one hurt animation fills its several hurt slots -- and its dodge for the dodge
    // roll (below). The frame counts stay template: the source states how fast a roll travels,
    // never for how long.
    //
    // HOW FAR IT GOES, not how fast it starts. The two engines slow a roll down differently:
    // the source multiplies its speed by a decay each frame, which comes to a total distance
    // of speed / (1 - decay), while this engine takes friction off it every frame until it
    // stops, which covers v*v / (2 * friction). Carrying the START speed across gives the
    // right first frame and the wrong roll -- mario ends up three times further down the
    // stage. Carrying the DISTANCE across is what the move actually is, so the speed is the
    // one that covers it: v = sqrt(2 * friction * distance).
    //
    // Everything it needs is already stated -- the source's speed and how it slows, this
    // engine's friction, and the one size knob.
    // the dodge roll the source plays starts at `dodgeSpeed` and loses `dodgeDecel` every frame
    // until it stops (read off its record: goku's 18 and 1.5 roll him 117, fox's 16 and 1.3 roll
    // him 106), so where both are stated the distance is that sum; `roll_speed` and `roll_decay`
    // are the fallback for a fighter stating no dodge (goku states no decay there, and the start
    // speed alone rolled him off the stage)
    let dodge_distance = (s.dodge_speed > 0.0 && s.dodge_decel.abs() > 0.0).then(|| {
        let (v, d) = (s.dodge_speed, s.dodge_decel.abs());
        let n = (v / d).floor();
        v * (n + 1.0) - d * n * (n + 1.0) / 2.0
    });
    let size = crate::mappings::character_stats().scaling.size_multiplier;
    let dodge_roll = dodge_distance.filter(|_| friction > 0.0).map(|distance| (2.0 * friction * distance * size).sqrt());
    let roll = (s.roll_speed > 0.0).then(|| {
        let decay_states_distance = s.roll_decay > 0.0 && s.roll_decay < 1.0;
        // the same friction this port already gives the engine, worked out above
        if decay_states_distance && friction > 0.0 {
            let distance = s.roll_speed / (1.0 - s.roll_decay)
                * crate::mappings::character_stats().scaling.size_multiplier;
            (2.0 * friction * distance).sqrt()
        } else {
            // nothing said about how it slows down, so the speed is all there is to carry
            ssf2_speed_to_fm(s.roll_speed)
        }
    });
    for f in tables.stats_fields.roll.iter().map(|s| s.as_str()) {
        // the dodge roll by the source's dodge, the getup, tech and ledge rolls by its roll
        let r = if f == "dodgeRollSpeed" { dodge_roll.or(roll) } else { roll };
        let v = match (f.ends_with("RollSpeed"), r) {
            (true, Some(r)) => format!("{r:.2}"),
            _ => c(f),
        };
        out.push_str(&req("character.stats.char_stats_field_plain", &t.char_stats_field_plain).replace("{{field}}", f).replace("{{value}}", &v));
    }
    out.push('\n');

    if !tables.stats_fields.airdash.is_empty() { out.push_str(&req("character.stats.char_stats_section_comment", &t.char_stats_section_comment).replace("{{title}}", "AIRDASH STATS")); }
    // The source's air dodge is not this engine's airdash (see stats_tables "airdash"); its
    // stated speed is kept for a table that asks for it.
    let dodge = (s.dodge_speed > 0.0).then_some(ssf2_speed_to_fm(s.dodge_speed));
    for f in tables.stats_fields.airdash.iter().map(|s| s.as_str()) {
        let v = match (f, dodge) {
            ("airdashInitialSpeed", Some(d)) => format!("{d:.2}"),
            _ => c(f),
        };
        out.push_str(&req("character.stats.char_stats_field_plain", &t.char_stats_field_plain).replace("{{field}}", f).replace("{{value}}", &v));
    }
    out.push('\n');

    if !tables.stats_fields.shield.is_empty() { out.push_str(&req("character.stats.char_stats_section_comment", &t.char_stats_section_comment).replace("{{title}}", "SHIELD STATS")); }
    for f in tables.stats_fields.shield.iter().map(|s| s.as_str()) {
        out.push_str(&req("character.stats.char_stats_field_plain", &t.char_stats_field_plain).replace("{{field}}", f).replace("{{value}}", &c(f)));
    }
    out.push('\n');

    out.push_str(&req("character.stats.char_stats_section_comment", &t.char_stats_section_comment).replace("{{title}}", "VOICE STATS"));
    for f in tables.stats_fields.voice.iter().map(|s| s.as_str()) {
        out.push_str(&req("character.stats.char_stats_field_plain", &t.char_stats_field_plain).replace("{{field}}", f).replace("{{value}}", &c(f)));
    }

    out.push_str(req("character.stats.char_stats_footer", &t.char_stats_footer));
    out
}

// ─── AnimationStats.hx ───────────────────────────────────────────────────────

fn generate_animation_stats(data: &CharacterData, splits: &[crate::anim_splitter::SplitAnim]) -> String {
    use std::collections::BTreeSet;

    // ── Base FM template: animations with hand-tuned properties ──────────────
    // These are the standard Fraymakers character-template entries.
    // Order and grouping match the official template.
    let template = crate::mappings::character_animation_template();

    // Collect template names for dedup
    let template_names: BTreeSet<&str> = template.iter().map(|e| e.name.as_str()).collect();

    let t = &crate::mappings::script_templates().character.stats;
    let req = crate::mappings::require_template;
    let mut out = req("character.stats.anim_stats_header", &t.anim_stats_header)
        .replace("{{char_name}}", &data.name);
    let entry_tpl = req("character.stats.anim_stats_entry", &t.anim_stats_entry);

    // Emit template entries (the template Vec is data — stays in Rust). `body`
    // is the `{}` / `{props}` object literal assembled here.
    for e in template {
        let props = &e.props;
        let body = if props.is_empty() { "{}".to_string() } else { format!("{{{}}}", props) };
        out.push_str(&entry_tpl.replace("{{name}}", &e.name).replace("{{body}}", &body));
    }

    // Emit split animations not already in template
    let mut extra_names: Vec<&str> = Vec::new();
    for split in splits {
        if !template_names.contains(split.fm_name.as_str()) && !extra_names.contains(&split.fm_name.as_str()) {
            extra_names.push(&split.fm_name);
        }
    }
    if !extra_names.is_empty() {
        out.push_str(req("character.stats.anim_stats_split_header", &t.anim_stats_split_header));
        for name in &extra_names {
            // Check if this split has loop_tail
            let is_loop = splits.iter().any(|s| s.fm_name == *name && s.loop_tail);
            // a piece cut out of a jump is still the jump: the engine lets a fighter drift through
            // its own jump animations and through nothing it does not know, so a segment the
            // converter named (the double jump's backflip) has to say so or the fighter takes no
            // direction until it lands (measured on sora)
            let from_jump = splits.iter().any(|s| s.fm_name == *name && s.source_anim.starts_with("jump"));
            let mut props: Vec<&str> = Vec::new();
            if is_loop { props.push("endType:AnimationEndType.LOOP"); }
            if from_jump { props.push("allowMovement: true"); }
            let body = format!("{{{}}}", props.join(", "));
            out.push_str(&entry_tpl.replace("{{name}}", name).replace("{{body}}", &body));
        }
    }

    out = attacks_keep_on_landing(out, data, splits);
    out = link_frames_between_ground_and_air(out, data, splits);
    out.push_str(req("character.stats.anim_stats_footer", &t.anim_stats_footer));
    out
}

/// The source's engine never ends an attack because the fighter touched the ground: the move's
/// own code lands it (a listener that plays its landing, or goes to a label). So every attack's
/// animation plays on through a landing here too, and its code does what it says (an attack that
/// links frames swaps to its ground version, below). The attack's declared stats are the
/// animation's own, as `runtime/attack_stats.jsonc` maps them (its slowdown, caps, the floor's
/// edge, what may cancel it), over what an attack that states nothing has.
fn attacks_keep_on_landing(out: String, data: &CharacterData, splits: &[crate::anim_splitter::SplitAnim]) -> String {
    let table = crate::compile::attack_stats::table();
    let mut names: std::collections::BTreeMap<String, Vec<(String, String)>> = Default::default();
    for a in &data.attacks {
        let fm = data.ssf2_to_fm_anim.get(&a.ssf2_name).cloned().unwrap_or_else(|| a.name.clone());
        let mut props: Vec<(String, String)> = table.defaults.clone();
        let mut ranks = std::collections::BTreeMap::new();
        for (k, v) in &a.stats { table.put(&mut props, &mut ranks, k, *v, false); }
        props.push(("landType".into(), "LandType.NONE".into()));
        names.insert(fm.clone(), props.clone());
        for s in splits { if s.source_anim == fm || s.source_anim == a.ssf2_name { names.insert(s.fm_name.clone(), props.clone()); } }
    }
    out.lines().map(|l| {
        let name = l.trim_start().split(':').next().unwrap_or("").trim().to_string();
        let Some(ours) = names.get(&name) else { return format!("{l}\n") };
        let (Some(o), Some(c)) = (l.find('{'), l.rfind('}')) else { return format!("{l}\n") };
        let mut props: Vec<String> = l[o + 1..c].split(',').map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty() && !p.starts_with("landAnimation") && !ours.iter().any(|(f, _)| p.split(':').next().map(str::trim) == Some(f.as_str()))).collect();
        props.extend(ours.iter().map(|(f, t)| format!("{f}:{t}")));
        format!("{}{{{}}}{}\n", &l[..o], props.join(", "), &l[c + 1..])
    }).collect()
}

/// An attack that declares `linkFrames` goes on in its air version when the fighter leaves the
/// ground mid-move, at the same frame, and back to its ground version when it lands: the source
/// switches between the two itself. Here that is the engine's link-frames leave and land types
/// on the pair (mario's fireball landed into its landing lag instead of carrying on).
fn link_frames_between_ground_and_air(out: String, data: &CharacterData, splits: &[crate::anim_splitter::SplitAnim]) -> String {
    let have: std::collections::BTreeSet<&str> = splits.iter().map(|s| s.fm_name.as_str()).collect();
    let truthy = |v: &str| !matches!(v.trim(), "0" | "false" | "" | "null");
    let mut pairs: std::collections::BTreeSet<(String, String)> = Default::default();
    for a in &data.attacks {
        if !a.stats.iter().any(|(k, v)| k == "linkFrames" && truthy(&v.to_string())) { continue }
        let fm = data.ssf2_to_fm_anim.get(&a.ssf2_name).cloned().unwrap_or_else(|| a.name.clone());
        let ground = fm.strip_suffix("_air").map(str::to_string).unwrap_or_else(|| fm.clone());
        let air = format!("{ground}_air");
        if have.contains(ground.as_str()) && have.contains(air.as_str()) { pairs.insert((ground, air)); }
    }
    if pairs.is_empty() { return out }
    let set = |line: &str, drop: &[&str], add: &str| -> String {
        let (Some(o), Some(c)) = (line.find('{'), line.rfind('}')) else { return line.to_string() };
        let mut props: Vec<String> = line[o + 1..c].split(',').map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty() && !drop.iter().any(|d| p.starts_with(d))).collect();
        props.push(add.to_string());
        format!("{}{{{}}}{}", &line[..o], props.join(", "), &line[c + 1..])
    };
    out.lines().map(|l| {
        let name = l.trim_start().split(':').next().unwrap_or("");
        let mut l = l.to_string();
        for (g, a) in &pairs {
            if name == g {
                l = set(&l, &["leaveGroundType", "leaveGroundAnimation"], &format!("leaveGroundType:LeaveGroundType.LINK_FRAMES, leaveGroundAnimation: \"{a}\""));
            } else if name == a {
                l = set(&l, &["landType", "landAnimation"], &format!("landType:LandType.LINK_FRAMES, landAnimation: \"{g}\""));
            }
        }
        l + "\n"
    }).collect()
}

// ─── Jabs ────────────────────────────────────────────────────────────────────

/// How many of jab1..jab4 actually have image content. Drives whether the empty-
/// animation drop in entity_gen keeps jab2/jab3 as placeholders.
fn count_populated_jabs(img: &crate::image_extractor::ImageExtractionResult) -> usize {
    ["jab1", "jab2", "jab3", "jab4"].iter()
        .filter(|name| img.anim_images.get(**name)
            .map(|af| af.frames.values().any(|f| !f.is_empty()))
            .unwrap_or(false))
        .count()
}

// ─── manifest.json ───────────────────────────────────────────────────────────

fn generate_manifest(char_id: &str, display_name: &str, projectile_names: &[String]) -> String {
    let ai_id = format!("{char_id}Ai");
    let ai_script_id = format!("{char_id}AiScript");

    let mut content = vec![serde_json::json!({
            "id": char_id,
            "name": display_name,
            "description": format!("{} — converted from Super Smash Flash 2", display_name),
            "type": "character",
            "objectStatsId":    format!("{}CharacterStats", char_id),
            "aiId":             ai_id.clone(),
            "animationStatsId": format!("{}AnimationStats", char_id),
            "hitboxStatsId":    format!("{}HitboxStats", char_id),
            "scriptId":         format!("{}Script", char_id),
            "costumesId":       format!("{}Costumes", char_id),
            "metadata": {
                "ui": {
                    "entityId": "menu",
                    "render": {
                        "animation":               "full",
                        "animation_icon":          "icon",
                        "animation_icon_no_palette": "icon_no_palette",
                        "x_offset":       0,
                        "y_offset":       38,
                        "x_offset_door":  0,
                        "y_offset_door":  0,
                        "x_offset_door_ffa": 0,
                        "y_offset_door_ffa": 0
                    },
                    "hud": {
                        "animation":              "hud",
                        "animation_front":        "hud_front",
                        "animation_happy":        "hud_happy",
                        "animation_happy_front":  "hud_happy_front",
                        "animation_sad":          "hud_sad",
                        "animation_sad_front":    "hud_sad_front",
                        "animation_angry":        "hud_angry",
                        "animation_angry_front":  "hud_angry_front",
                        "animation_hurt":         "hud_hurt",
                        "animation_hurt_front":   "hud_hurt_front",
                        "animation_stock_icon":   "stock"
                    },
                    "css": {
                        "animation": "css",
                        "info": {
                            "game": "Super Smash Flash 2",
                            "description": format!("{} — ported from SSF2", display_name)
                        }
                    }
                }
            }
    })];  // close vec![json!({...})]

    // The character DECLARES an ai, and the script behind it is emitted with it. Declaring one
    // without the script makes the engine complain twice on every load; dropping the declaration
    // instead makes the character invisible in a match, which is worse and was measured.
    content.push(serde_json::json!({
        "id":       ai_id,
        "type":     "characterAi",
        "scriptId": ai_script_id
    }));

    // Add projectile entries
    for proj_name in projectile_names {
        let entity_id = proj_name.replace('_', "");
        content.push(serde_json::json!({
            "id":               crate::projectile_gen::projectile_content_id(proj_name),
            "type":             "projectile",
            "objectStatsId":    format!("{}ProjectileStats", entity_id),
            "animationStatsId": format!("{}ProjectileAnimationStats", entity_id),
            "hitboxStatsId":    format!("{}ProjectileHitboxStats", entity_id),
            "scriptId":         format!("{}ProjectileScript", entity_id),
            "costumesId":       format!("{}Costumes", char_id)
        }));
    }
    // the platforms the projectiles give themselves: structure contents beside them
    for (id, stats_id, script_id) in crate::projectile_gen::take_structure_contents() {
        content.push(serde_json::json!({ "id": id, "type": "structure", "objectStatsId": stats_id, "scriptId": script_id }));
    }

    serde_json::json!({
        "resourceId": char_id,
        "content": content
    }).to_string()
}

/// Multi-character project manifest. Mirrors `generate_manifest`'s
/// per-character content shape but emits one entry per character in
/// the project, plus per-character AI entries, plus per-character
/// projectile entries. Used by `main::finalize_multi_char_project`.
pub fn generate_multi_char_manifest(
    project_id: &str,
    chars: &[crate::project::ManifestCharEntry],
) -> String {
    let mut content: Vec<serde_json::Value> = Vec::new();
    // Projectiles are discovered from the one shared SWF, so multiple sub-characters
    // can list the same one. Each projectile content id must appear ONCE in the manifest.
    let mut seen_proj: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for entry in chars {
        let char_id = &entry.char_id;
        let display_name = &entry.display_name;
        let ai_id = format!("{char_id}Ai");
        let ai_script_id = format!("{char_id}AiScript");


        content.push(serde_json::json!({
            "id": char_id,
            "name": display_name,
            "description": format!("{} — converted from Super Smash Flash 2", display_name),
            "type": "character",
            "objectStatsId":    format!("{}CharacterStats", char_id),
            "aiId":             ai_id.clone(),
            "animationStatsId": format!("{}AnimationStats", char_id),
            "hitboxStatsId":    format!("{}HitboxStats", char_id),
            "scriptId":         format!("{}Script", char_id),
            "costumesId":       format!("{}Costumes", char_id),
            "metadata": {
                "ui": {
                    "entityId": entry.menu_entity_id,
                    "render": {
                        "animation":               "full",
                        "animation_icon":          "icon",
                        "animation_icon_no_palette": "icon_no_palette",
                        "x_offset":       0,
                        "y_offset":       38,
                        "x_offset_door":  0,
                        "y_offset_door":  0,
                        "x_offset_door_ffa": 0,
                        "y_offset_door_ffa": 0
                    },
                    "hud": {
                        "animation":              "hud",
                        "animation_front":        "hud_front",
                        "animation_happy":        "hud_happy",
                        "animation_happy_front":  "hud_happy_front",
                        "animation_sad":          "hud_sad",
                        "animation_sad_front":    "hud_sad_front",
                        "animation_angry":        "hud_angry",
                        "animation_angry_front":  "hud_angry_front",
                        "animation_hurt":         "hud_hurt",
                        "animation_hurt_front":   "hud_hurt_front",
                        "animation_stock_icon":   "stock"
                    },
                    "css": {
                        "animation": "css",
                        "info": {
                            "game": "Super Smash Flash 2",
                            "description": format!("{} — ported from SSF2", display_name)
                        }
                    }
                }
            }
        }));
        content.push(serde_json::json!({
            "id":       ai_id,
            "type":     "characterAi",
            "scriptId": ai_script_id
        }));
        for proj_name in &entry.projectile_names {
            let content_id = crate::projectile_gen::projectile_content_id(proj_name);
            if !seen_proj.insert(content_id.clone()) { continue; } // already listed by an earlier char
            let entity_id = proj_name.replace('_', "");
            content.push(serde_json::json!({
                "id":               content_id,
                "type":             "projectile",
                "objectStatsId":    format!("{}ProjectileStats", entity_id),
                "animationStatsId": format!("{}ProjectileAnimationStats", entity_id),
                "hitboxStatsId":    format!("{}ProjectileHitboxStats", entity_id),
                "scriptId":         format!("{}ProjectileScript", entity_id),
                "costumesId":       format!("{}Costumes", char_id)
            }));
        }
        for (id, stats_id, script_id) in crate::projectile_gen::take_structure_contents() {
            content.push(serde_json::json!({ "id": id, "type": "structure", "objectStatsId": stats_id, "scriptId": script_id }));
        }
    }

    serde_json::json!({
        "resourceId": project_id,
        "content": content
    }).to_string()
}

/// Public alias for the manifest .meta sidecar generator, so
/// `main::finalize_multi_char_project` can call it without leaking the
/// private name.
pub fn generate_manifest_meta_pub(guid: &str) -> String {
    generate_manifest_meta(guid)
}

// ─── Sound content entries ────────────────────────────────────────────────────

/// Write a `.wav.meta` sidecar next to each extracted audio file, matching
/// the schema observed in `Fraymakers/character-template` (id = filename
/// sans `.wav`, plus pluginMetadata + plugins references). No central audio
/// manifest is needed: reference characters register sounds purely through
/// these per-file sidecars.
pub fn generate_sound_entries(
    audio_dir: &Path,
    char_name: &str,
    sounds: &[crate::sound_extractor::SoundEntry],
) -> Result<()> {
    fs::create_dir_all(audio_dir)?;

    for s in sounds {
        let safe_name: String = s.name.chars()
            .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();
        let wav_path = audio_dir.join(format!("{}.wav", safe_name));
        if !wav_path.exists() { continue; }

        // content id = filename sans .wav — referenced from CharacterStats
        // voice arrays and from AudioClip.play(self.getResource().getContent(<id>))
        let guid = det_uuid(&format!("{}::sound_meta_{}", char_name, safe_name));
        let meta = serde_json::json!({
            "export": true,
            "guid":   guid,
            "id":     safe_name,
            "pluginMetadata": {},
            "plugins": ["com.fraymakers.FraymakersMetadata"],
            "tags":    [],
            "version": 1
        });
        fs::write(
            audio_dir.join(format!("{}.wav.meta", safe_name)),
            serde_json::to_string_pretty(&meta)?,
        )?;
    }

    // Silent placeholder asset. `rewrite_play_sound_calls` points SSF2 sounds
    // that have no extracted asset and no GlobalSfx match at this id (with a
    // visible TODO), so the AudioClip.play() call is valid (plays silence)
    // and the modder can swap in a real sound.
    let ph = crate::api_mappings::PLACEHOLDER_SOUND_ID;
    fs::write(audio_dir.join(format!("{ph}.wav")), silent_wav())?;
    let ph_guid = det_uuid(&format!("{}::sound_meta_{}", char_name, ph));
    let ph_meta = serde_json::json!({
        "export": true,
        "guid":   ph_guid,
        "id":     ph,
        "pluginMetadata": {},
        "plugins": ["com.fraymakers.FraymakersMetadata"],
        "tags":    [],
        "version": 1
    });
    fs::write(
        audio_dir.join(format!("{ph}.wav.meta")),
        serde_json::to_string_pretty(&ph_meta)?,
    )?;

    Ok(())
}

/// A minimal valid silent WAV (8 kHz, mono, 16-bit PCM, ~50 ms of silence)
/// used as the placeholder audio asset for unmapped SSF2 sounds.
fn silent_wav() -> Vec<u8> {
    let sample_rate: u32 = 8000;
    let num_samples: u32 = sample_rate / 20; // ~50 ms
    let data_size: u32 = num_samples * 2;    // 16-bit mono
    let mut w = Vec::with_capacity(44 + data_size as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_size).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());      // fmt chunk size
    w.extend_from_slice(&1u16.to_le_bytes());        // PCM
    w.extend_from_slice(&1u16.to_le_bytes());        // mono
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    w.extend_from_slice(&2u16.to_le_bytes());        // block align
    w.extend_from_slice(&16u16.to_le_bytes());       // bits per sample
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_size.to_le_bytes());
    w.resize(44 + data_size as usize, 0);            // silence
    w
}

/// Convert a projectile name to a valid entity filename.
/// "mario_fireball" → "mario_fireball"
/// Say so when one entity file is written twice in a run. Two emitters once wrote the same
/// name and the second silently replaced the first, so a move fired art with no behaviour
/// behind it, and what surfaced was a frame reporting an unknown variable for a name its own
/// file declares. The names differ now; this is here so the next one cannot be silent.
pub(crate) fn note_entity_written(path: &std::path::Path) {
    use std::cell::RefCell;
    use std::collections::BTreeSet;
    thread_local! {
        static SEEN: RefCell<BTreeSet<std::path::PathBuf>> = const { RefCell::new(BTreeSet::new()) };
    }
    SEEN.with(|s| {
        if !s.borrow_mut().insert(path.to_path_buf()) {
            log::warn!("entity {} was written twice in this run -- the second one is all that \
                        is left, so whatever addressed the first now resolves to the other",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("?"));
        }
    });
}

pub(crate) fn sanitize_entity_name(name: &str) -> String {
    name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "_")
}

/// Convert an SSF2 projectile name like `dee_nspec` to the PascalCase
/// prefix used for the script-file names (`DeeNspec` → `DeeNspecScript.hx`).
/// Matches the convention seen in real FM character mods
/// (aJewelofRarity/AnnieCharacter: `Cut` → `CutScript.hx`).
pub(crate) fn snake_to_pascal(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut capitalize_next = true;
    for c in name.chars() {
        if c == '_' || c == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            out.extend(c.to_uppercase());
            capitalize_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

// ─── Projectile script generators ─────────────────────────────────────────────

/// Generate `library/manifest.json.meta` — the JSON sidecar that pairs
/// `manifest.json` with its content id (`"manifest"`) and language hint.
/// Schema cross-referenced against `Fraymakers/character-template`.
fn generate_manifest_meta(guid: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "export": true,
        "guid": guid,
        "id": "manifest",
        "language": "json",
        "pluginMetadata": {
            "com.fraymakers.FraymakersMetadata": { "version": "0.1.0" }
        },
        "plugins": [],
        "tags": [],
        "version": 1
    })).unwrap_or_default()
}

/// Which kind of `.hx.meta` sidecar to emit. The choice determines the
/// `language`, `pluginMetadata`, and `plugins` fields — values are taken
/// verbatim from the Fraymakers/character-template reference repo so that
/// FrayTools recognises our exports the same way it does first-party ones.
#[derive(Copy, Clone)]
pub enum ScriptMetaKind {
    /// Character `Script.hx.meta` — objectType=CHARACTER + plugin listed.
    CharacterScript,
    /// `CharacterStats.hx.meta` — plugin listed, plugin version 0.4.0.
    CharacterStats,
    /// `AnimationStats.hx.meta` — pluginMetadata version only, no plugin.
    CharacterAnimationStats,
    /// `HitboxStats.hx.meta` — pluginMetadata version only, no plugin.
    CharacterHitboxStats,
    /// Projectile `ProjectileScript.hx.meta` — objectType=PROJECTILE + plugin.
    ProjectileScript,
    /// `AiScript.hx.meta` — objectType=CHARACTER_AI + plugin.
    CharacterAiScript,
    /// A structure's script (a platform a projectile gives itself) — objectType=LINE_SEGMENT_STRUCTURE + plugin.
    StructureScript,
    /// Projectile companion stats files — empty pluginMetadata, no plugin.
    ProjectileStats,
    /// Projectile companion stats files — empty pluginMetadata, no plugin.
    ProjectileAnimationStats,
    /// Projectile companion stats files — empty pluginMetadata, no plugin.
    ProjectileHitboxStats,
}

/// The character's ai script. Nothing of SSF2's cpu behaviour is ported yet, so this is the
/// smallest thing the engine can load and run: an ai that does nothing.
///
/// It exists because the character DECLARES an ai. Declaring one with no script behind it makes
/// the engine complain on every load, and dropping the declaration instead makes the character
/// invisible in a match (measured: the fighter spawns, animates and reports its position, and is
/// simply never drawn).
pub(crate) fn generate_ai_script(char_id: &str) -> String {
    format!("// AI script for {char_id} -- converted from SSF2.\n\
// SSF2 characters declare their cpu behaviour (`cpuAttackQueue` and friends) and none of it is\n\
// ported yet, so this ai stands still rather than pretending to play.\n\
\n\
function initialize() {{}}\n\
\n\
function update() {{}}\n")
}

/// Generate a `.hx.meta` sidecar matching the Fraymakers reference layout.
/// All script-kind .meta files use `language: "hscript"`. The
/// `pluginMetadata` shape and `plugins` array vary by kind — see the enum
/// docs. Cross-referenced against Fraymakers/character-template.
pub(crate) fn script_meta(id: &str, guid: &str, kind: ScriptMetaKind) -> String {
    use ScriptMetaKind::*;
    // `plugin_meta` always contains a `com.fraymakers.FraymakersMetadata`
    // entry except for the projectile companion stats files, which use an
    // empty object in the reference.
    let plugin_meta = match kind {
        CharacterScript => serde_json::json!({
            "com.fraymakers.FraymakersMetadata": {
                "objectType": "CHARACTER",
                "version": "0.3.0"
            }
        }),
        ProjectileScript => serde_json::json!({
            "com.fraymakers.FraymakersMetadata": {
                "objectType": "PROJECTILE",
                "version": "0.1.1"
            }
        }),
        StructureScript => serde_json::json!({
            "com.fraymakers.FraymakersMetadata": {
                "objectType": "LINE_SEGMENT_STRUCTURE",
                "version": "0.4.0"
            }
        }),
        CharacterAiScript => serde_json::json!({
            "com.fraymakers.FraymakersMetadata": {
                "objectType": "CHARACTER_AI",
                "version": "0.1.0"
            }
        }),
        CharacterStats => serde_json::json!({
            "com.fraymakers.FraymakersMetadata": { "version": "0.4.0" }
        }),
        CharacterAnimationStats | CharacterHitboxStats => serde_json::json!({
            "com.fraymakers.FraymakersMetadata": { "version": "0.1.0" }
        }),
        ProjectileStats | ProjectileAnimationStats | ProjectileHitboxStats => {
            serde_json::json!({})
        }
    };
    // `plugins` is listed only on the files the reference flags it on: the
    // character + projectile Script.hx.meta and CharacterStats.hx.meta.
    let plugins: Vec<&str> = match kind {
        CharacterScript | ProjectileScript | CharacterAiScript | CharacterStats | StructureScript => {
            vec!["com.fraymakers.FraymakersMetadata"]
        }
        _ => vec![],
    };
    serde_json::to_string_pretty(&serde_json::json!({
        "export": true,
        "guid": guid,
        "id": id,
        "language": "hscript",
        "pluginMetadata": plugin_meta,
        "plugins": plugins,
        "tags": [],
        "version": 1
    })).unwrap_or_default()
}

/// ProjectileScript.hx — handles lifecycle events.
/// Multi-state projectiles get extra PState constants and state-switching logic.
/// Synthesize a projectile's launch (X_SPEED, Y_SPEED) from the SSF2
/// `getProjectileStats()` physics. SSF2 projectiles carry no behavior script —
/// the engine drives them from these stats — so we derive the launch from the
/// best available speed source and apply the SAME velocity_scale the character
/// physics uses (the one `size_multiplier` knob in stats.jsonc + the 30->60fps
/// ratio). Returns (x, y, note).
/// A flag a projectile declares about itself, or the source's own default when it declares none.
fn projectile_flag(ssf2_match: Option<&crate::abc_parser::ProjectileData>, key: &str) -> bool {
    ssf2_match
        .and_then(|d| d.flags.get(key).copied())
        .or_else(|| crate::mappings::projectile_tables().source_flag_defaults.get(key).copied())
        .unwrap_or(false)
}

pub(crate) fn synth_projectile_launch(ssf2_match: Option<&crate::abc_parser::ProjectileData>) -> (f64, f64, String) {
    let vs = crate::mappings::character_stats().scaling.velocity_scale();
    let stats = match ssf2_match {
        Some(d) if !d.stats.is_empty() => &d.stats,
        // No SSF2 data at all: fall back to the historical template default.
        _ => return (8.0, 0.0, "no SSF2 speed data — default launch (verify live).".to_string()),
    };
    // The declaration states the launch. Spellings are the ones the corpus writes (`xspeed`
    // outnumbers `xSpeed` 83 to 2), and a speed cap stands in only where no launch is stated.
    let pick = |keys: &[&str]| keys.iter().find_map(|k| stats.get(*k).copied());
    // a declaration that states no x speed launches with none: a thunder cloud parks above
    // its summoner and its bolt drops straight down, both declared with a y speed alone
    let (x_raw, x_src) = pick(&["xspeed", "xSpeed", "x_speed"]).map(|v| (v, "xspeed"))
        .or_else(|| pick(&["groundSpeedCap", "ground_speed_cap", "aerialSpeedCap", "aerial_speed_cap"]).map(|v| (v, "speedCap")))
        .unwrap_or((0.0, "no xspeed stated"));
    let y_raw = pick(&["yspeed", "ySpeed", "y_speed"]).unwrap_or(0.0);
    let note = format!(
        "X from SSF2 {}={} * velocity_scale {:.3}.",
        x_src, fmt_num(x_raw), vs
    );
    (x_raw * vs, y_raw * vs, note)
}

/// The physics a projectile's own declaration states, in Fraymakers terms.
///
/// SSF2 runs these numbers itself -- a projectile has no code that applies its own gravity -- so
/// on this side they become the script that does. Speeds convert through the same
/// `velocity_scale` the character physics uses and accelerations through `accel_scale`, since
/// they are per-frame values on a clock that runs twice as fast here.
struct ProjectilePhysics {
    consts: String,
    /// Code for the projectile's `initialize()` (a live-instance cap, say), run once at birth.
    init: String,
    update: String,
    note: String,
}

fn projectile_physics(
    d: Option<&crate::abc_parser::ProjectileData>,
    entity_id: &str,
    x: f64,
    y: f64,
    end: &str,
) -> ProjectilePhysics {
    let sc = &crate::mappings::character_stats().scaling;
    let (vs, acs) = (sc.velocity_scale(), sc.accel_scale());
    let stats = d.map(|d| &d.stats);
    let flags = d.map(|d| &d.flags);
    // what it states, else what a projectile has in the source when it states nothing (a
    // lifetime, most of all: without it a spent projectile stays out for the match)
    let defaults = &crate::mappings::projectile_tables().source_defaults;
    let num = |k: &str| stats.and_then(|s| s.get(k).copied()).or_else(|| defaults.get(k).copied());
    let flag = |k: &str| flags.and_then(|f| f.get(k).copied()).unwrap_or(false);
    let names = d.map(|d| &d.names);
    let name = |k: &str| names.and_then(|n| n.get(k)).map(|s| s.as_str());

    // The launch is handed over at the spawn (Fraymakers ignores one a projectile gives itself
    // in `initialize`), so it is recorded here for reading, not applied.
    let mut consts = format!("// launch given at the spawn: x {} y {}\n", fmt_num(x), fmt_num(y));
    let mut update = String::new();
    let mut init = String::new();
    let mut declared: Vec<String> = Vec::new();
    // a stat this script keeps (not the engine), written at runtime by the source's code
    // (`updateProjectileStats({ homingSpeed: v })`, samus's missile handing its decaying speed
    // back as its homing speed): each branch takes the source's value in the source's units
    let mut setters: Vec<String> = Vec::new();

    // A cap the source declares (`limit`): at most this many of THIS projectile alive at once per
    // owner. No engine stat caps it, so the projectile enforces it at birth -- whatever spawned it.
    // It registers in its OWNER's exports (a persistent per-entity store that survives frames; the
    // projectile's own exports do not persist a value the same way), prunes the dead, then removes
    // the oldest over the cap (overwrite) or removes ITSELF (block) with kill() -- toState(DESTROYING)
    // does not free a projectile with no destroy animation (it lingers in DESTROYING, still counted).
    if let Some(lim) = num("limit").filter(|l| *l >= 1.0)
        .filter(|_| std::env::var("PEPTIDE_NO_PROJ_LIMIT").is_err())  // A/B toggle: skip the cap
    {
        let overwrite = flag("limitOverwrite");
        declared.push(format!("limit {}{}", fmt_num(lim), if overwrite { " (overwrite)" } else { "" }));
        let field = format!("__lim_{entity_id}");
        let enforce = if overwrite {
            // the one taken off first lets go of what it listens to on others: taken off outright,
            // it never runs its own end, and a handler left on a fighter would outlive its script
            format!("while (_lst.length >= {n}) {{ var _o = _lst.shift(); if (_o != null && !_o.isDisposed()) {{ var _ol = _o.exports == null ? null : _o.exports.__others; if (_ol != null) {{ var _j = 0; while (_j < _ol.length) {{ var _r = _ol[_j]; if (_r.o != null && !_r.o.isDisposed()) {{ _r.o.removeEventListener(_r.e, _r.f); }} _j = _j + 1; }} _ol.splice(0, _ol.length); }} if (_o.exports != null && _o.exports.__endSrcTimers != null) {{ _o.exports.__endSrcTimers(); }} _o.kill(); }} }} _lst.push(self);",
                    n = fmt_num(lim))
        } else {
            format!("if (_lst.length >= {n}) {{ self.kill(); }} else {{ _lst.push(self); }}",
                    n = fmt_num(lim))
        };
        init.push_str(&format!(
            "    self.exports.__others = __others;\n    self.exports.__endSrcTimers = __endSrcTimers;\n    {{ var _own = self.getOwner();\n\
             \x20     if (_own != null) {{\n\
             \x20       if (_own.exports.{field} == null) {{ _own.exports.{field} = []; }}\n\
             \x20       var _lst = _own.exports.{field}; var _k = 0;\n\
             \x20       while (_k < _lst.length) {{ if (_lst[_k] == null || _lst[_k].isDisposed()) {{ _lst.splice(_k, 1); }} else {{ _k = _k + 1; }} }}\n\
             \x20       {enforce}\n\
             \x20     }} }}\n"));
    }

    // Falling is the ENGINE's, not this script's: the projectile's stats state its gravity and how
    // fast it may fall, and the engine applies them. Doing it here as well applied it twice.
    // Recorded in the header for reading, and the numbers themselves go to ProjectileStats.
    // a projectile the source gives no gravity, or a gravity of 0, flies flat there (its missile
    // and ness's fire do); a 0 in this engine's stats file reads as unset and its default fell
    // the shot (ness's fire sank to the floor and died at 8 frames against the source's 12), so
    // the 0 is written at start, where the engine takes it
    if num("gravity").is_none_or(|g| g == 0.0) {
        init.push_str("    self.updateProjectileStats({ gravity: 0 });\n");
    }
    if let Some(g) = num("gravity").filter(|g| *g != 0.0) {
        declared.push(format!("gravity {}", fmt_num(g)));
        if let Some(c) = num("maxgravity").or_else(|| num("max_gravity")) {
            declared.push(format!("maxgravity {}", fmt_num(c)));
        }
        let _ = acs;
    }
    // x decay: the source subtracts this from the projectile's forward speed every frame, straight
    // through zero, so a positive decay slows the shot and then carries it back the way it came (a
    // boomerang returning, a punch that drifts home) while a negative one accelerates it. it is not
    // a multiplier, so it is not a no-op at 1. the value is a change-per-source-frame in the
    // source's speed units, so it converts through velocity_scale, and the script steps twice per
    // source frame (its lifetime counts double), so it takes half the step each time.
    let homing = num("homingSpeed").is_some_and(|h| h >= 0.0);
    if let Some(dec) = num("xdecay").filter(|v| *v != 0.0) {
        consts.push_str(&format!("var X_DECAY_STEP = {};\n", fmt_num(dec * vs * 0.5)));
        setters.push(format!("if (n == \"xdecay\") {{ X_DECAY_STEP = v * {}; return true; }}", fmt_num(vs * 0.5)));
        declared.push(format!("xdecay {}", fmt_num(dec)));
        // under a homing drive the decay goes on the driven magnitude (below): the drive puts its
        // speed back every engine frame, and a decay on the velocity was lost between the frames
        // the class samples it (samus's missile slowed at half the source's rate)
        if !homing { update.push_str("    self.setXSpeed(self.getXSpeed() - X_DECAY_STEP);\n"); }
    }
    // homing: the source hands the projectile's movement over to a chase of the nearest opponent at
    // homingSpeed, easing the heading toward the target by homingEase (smaller is tighter, 1 snaps).
    // a target is one that can be hit: the source passes over a fighter that is intangible,
    // invincible, dead or between lives (measured: its missile flies straight past one)
    // No engine stat does this, so it becomes the script that does: each frame, find the nearest
    // character that is not the owner, turn the current heading a step toward it, and drive the
    // projectile along it at homingSpeed. speed converts through velocity_scale like any launch.
    if let Some(hs) = num("homingSpeed").filter(|h| *h >= 0.0) {
        let ease = num("homingEase").filter(|e| *e >= 1.0).unwrap_or(1.0);
        // the heading is the script's own: read back off the velocity it was bent by the engine's
        // gravity every frame and the missile dived (samus's homing missile fell 340 px in 70
        // frames where the source's flies flat); it starts from the launch and turns from there
        // the ease is a share of the remaining turn taken per SOURCE frame; this script turns every
        // engine frame, twice as often, so it takes half the share each time (samus's missile bent
        // three times as far as the source's)
        consts.push_str(&format!("var HOMING_SPEED = {};\nvar HOMING_EASE = {};\nvar _hAng = null;\n", fmt_num(hs * vs), fmt_num(ease * 2.0)));
        setters.push(format!("if (n == \"homingSpeed\") {{ HOMING_SPEED = v * {}; return true; }}", fmt_num(vs)));
        let decay_drive = if num("xdecay").is_some_and(|v| v != 0.0) { "        HOMING_SPEED = HOMING_SPEED - X_DECAY_STEP;\n" } else { "" };
        setters.push("if (n == \"homingEase\") { HOMING_EASE = ((v != null && v >= 1) ? v : 1) * 2; return true; }".to_string());
        declared.push(format!("homingSpeed {} (ease {})", fmt_num(hs), fmt_num(ease)));
        update.push_str(&
            "    { var _t = null; var _cs = match.getCharacters(); var _bd = 1.0e12; var _i = 0;\n\
             \x20     while (_i < _cs.length) { var _c = _cs[_i];\n\
             \x20       if (_c != self.getOwner() && !_c.isDisposed() && !_c.hasBodyStatus(BodyStatus.INTANGIBLE) && !_c.hasBodyStatus(BodyStatus.INVINCIBLE)) { var _dx = _c.getX() - self.getX(); var _dy = _c.getY() - self.getY();\n\
             \x20         var _d = _dx * _dx + _dy * _dy; if (_d < _bd) { _bd = _d; _t = _c; } }\n\
             \x20       _i = _i + 1; }\n\
             \x20     if (_t != null) {\n\
             {decay_drive}\
             \x20       var _want = Math.atan2(_t.getY() - self.getY(), _t.getX() - self.getX());\n\
             \x20       if (_hAng == null) { _hAng = Math.atan2(self.getYSpeed(), self.getXSpeed()); }\n\
             \x20       var _diff = _want - _hAng;\n\
             \x20       while (_diff > Math.PI) { _diff = _diff - 2 * Math.PI; }\n\
             \x20       while (_diff < -Math.PI) { _diff = _diff + 2 * Math.PI; }\n\
             \x20       _hAng = _hAng + _diff / HOMING_EASE;\n\
             \x20       self.setXSpeed(Math.cos(_hAng) * HOMING_SPEED); self.setYSpeed(Math.sin(_hAng) * HOMING_SPEED);\n\
             \x20     } }\n".replace("{decay_drive}", decay_drive));
    }
    // influence: the source lets the owner's held stick nudge the projectile as it flies. it keeps
    // a per-axis accumulator, each frame stepped toward the held direction by that axis' Factor and
    // decayed back toward zero when the stick is neutral, capped at that axis' Movement, then added
    // on top of the projectile's own velocity (the source moves by speed + influence each frame).
    // active when either axis' Movement is set. no engine stat does this. Factor and Movement are
    // speeds, so they convert through velocity_scale like any launch.
    {
        let ixm = num("influenceXMovement").unwrap_or(0.0);
        let iym = num("influenceYMovement").unwrap_or(0.0);
        if ixm > 0.0 || iym > 0.0 {
            let ixf = num("influenceXFactor").unwrap_or(0.0);
            let iyf = num("influenceYFactor").unwrap_or(0.0);
            consts.push_str(&format!(
                "var INF_XM = {}; var INF_YM = {}; var INF_XF = {}; var INF_YF = {};\nvar _infx = 0.0; var _infy = 0.0;\n",
                fmt_num(ixm * vs), fmt_num(iym * vs), fmt_num(ixf * vs), fmt_num(iyf * vs)));
            declared.push(format!("influence x{}/{} y{}/{}", fmt_num(ixm), fmt_num(ixf), fmt_num(iym), fmt_num(iyf)));
            update.push_str(
                "    { var _o = self.getOwner();\n\
                 \x20     if (_o != null && _o.getHeldControls != null) {\n\
                 \x20       var _c = _o.getHeldControls();\n\
                 \x20       if (INF_XM > 0) {\n\
                 \x20         if (_c.RIGHT) { _infx = _infx + INF_XF; } else if (_c.LEFT) { _infx = _infx - INF_XF; }\n\
                 \x20         else if (_infx > 0) { _infx = _infx - INF_XF; if (_infx < 0) { _infx = 0; } }\n\
                 \x20         else if (_infx < 0) { _infx = _infx + INF_XF; if (_infx > 0) { _infx = 0; } }\n\
                 \x20         if (_infx > INF_XM) { _infx = INF_XM; } if (_infx < -INF_XM) { _infx = -INF_XM; }\n\
                 \x20       }\n\
                 \x20       if (INF_YM > 0) {\n\
                 \x20         if (_c.DOWN) { _infy = _infy + INF_YF; } else if (_c.UP) { _infy = _infy - INF_YF; }\n\
                 \x20         else if (_infy > 0) { _infy = _infy - INF_YF; if (_infy < 0) { _infy = 0; } }\n\
                 \x20         else if (_infy < 0) { _infy = _infy + INF_YF; if (_infy > 0) { _infy = 0; } }\n\
                 \x20         if (_infy > INF_YM) { _infy = INF_YM; } if (_infy < -INF_YM) { _infy = -INF_YM; }\n\
                 \x20       }\n\
                 \x20     }\n\
                 \x20     self.setX(self.getX() + _infx); self.setY(self.getY() + _infy); }\n");
        }
    }
    // controlDirection: the owner aims the projectile with the stick. the source reads the held
    // direction as an 8-way angle and drives the projectile along it at the launch x speed's
    // magnitude, easing the heading toward the aim each frame when calcAngle is set and snapping to
    // it otherwise. the current heading starts from the launch velocity. no engine stat does this.
    // angle convention: 0 right, quarter-turn up, counter-clockwise, screen y down (ySpeed negated).
    if let Some(cd) = num("controlDirection").filter(|c| *c >= 0.0) {
        let mag = x.abs();
        let calc = num("calcAngle").is_some_and(|v| v != 0.0);
        // the heading starts at the direction the stat states (ness's thunder rises: 90), not at
        // the launch's: the launch is the magnitude along it (started off the launch, the bolt
        // went right, struck the other fighter and died at 53 frames against the source's 69)
        consts.push_str(&format!(
            "var CD_MAG = {};\nvar _cang = {};\n",
            fmt_num(mag), fmt_num(cd.to_radians())));
        declared.push(format!("controlDirection {}{}", fmt_num(cd), if calc { " (eased)" } else { "" }));
        let step = if calc {
            "var _d = _tgt - _cang; while (_d > Math.PI) { _d = _d - 2 * Math.PI; } while (_d < -Math.PI) { _d = _d + 2 * Math.PI; } _cang = _cang + _d / 6;"
        } else {
            "_cang = _tgt;"
        };
        update.push_str(&format!(
            "    {{ var _o = self.getOwner();\n\
             \x20     if (_o != null && _o.getHeldControls != null) {{\n\
             \x20       var _c = _o.getHeldControls(); var _tgt = _cang; var _has = true;\n\
             \x20       if (_c.UP && _c.LEFT) {{ _tgt = 2.35619449; }} else if (_c.UP && _c.RIGHT) {{ _tgt = 0.78539816; }}\n\
             \x20       else if (_c.DOWN && _c.LEFT) {{ _tgt = 3.92699082; }} else if (_c.DOWN && _c.RIGHT) {{ _tgt = 5.49778714; }}\n\
             \x20       else if (_c.UP) {{ _tgt = 1.57079633; }} else if (_c.DOWN) {{ _tgt = 4.71238898; }}\n\
             \x20       else if (_c.LEFT) {{ _tgt = 3.14159265; }} else if (_c.RIGHT) {{ _tgt = 0; }} else {{ _has = false; }}\n\
             \x20       if (_has) {{ {step} }}\n\
             \x20     }}\n\
             \x20     self.setXSpeed(Math.cos(_cang) * CD_MAG); self.setYSpeed(-Math.sin(_cang) * CD_MAG); }}\n"));
    }
    // trail: the source drops a decoration behind the projectile every trailEffectInterval frames.
    // trailEffect is the linkage id of the effect content (a string the declaration names, not a
    // number), so it comes from the declared names, not the numeric stats. spawn that content at the
    // projectile each interval, aiming it along the heading when trailEffectRotate is set.
    if let Some(trail) = name("trailEffect").filter(|s| !s.is_empty()) {
        let interval = num("trailEffectInterval").filter(|i| *i >= 1.0).unwrap_or(1.0);
        let rotate = flag("trailEffectRotate");
        consts.push_str(&format!("var TRAIL_EVERY = {};\nvar _trailT = 0;\nvar _trailName = \"{trail}\";\n", fmt_num(interval)));
        // the source's frames retune the trail as the shot flies (samus's missile drops one every
        // 5, then 8, then 16 frames, and none once it is spent)
        setters.push("if (n == \"trailEffectInterval\") { TRAIL_EVERY = (v != null && v >= 1) ? v : 1; return true; }".to_string());
        setters.push("if (n == \"trailEffect\") { _trailName = v; return true; }".to_string());
        declared.push(format!("trail {} every {}{}", trail, fmt_num(interval), if rotate { " (aimed)" } else { "" }));
        let rot = if rotate {
            "        var _r = Math.atan2(-self.getYSpeed(), self.getXSpeed()) * 180 / Math.PI;\n\
             \x20       if (_v != null) { _v.setRotation(-_r); }\n"
        } else { "" };
        update.push_str(&format!(
            "    _trailT = _trailT + 1;\n\
             \x20   if (_trailName != null && _trailT >= TRAIL_EVERY) {{ _trailT = 0;\n\
             \x20     var _tc = self.getResource().getContent(_trailName);\n\
             \x20     if (_tc != null) {{ var _v = match.createVfx(new VfxStats({{ spriteContent: _tc, animation: \"active\" }}), self);\n\
             {rot}\
             \x20     }}\n\
             \x20   }}\n"));
    }
    {
        // its lifetime in this engine's frames, -1 for one that never runs out; its own script can
        // set it as it runs (`__setLifetime`, in the source's frames) and ask it back
        let ratio = crate::entity_gen::FRAME_RATIO as f64;
        let t = num("time_max").filter(|t| *t > 0.0);
        let frames = t.map(|t| t * ratio).unwrap_or(-1.0);
        consts.push_str(&format!("var LIFETIME = {};\nvar _age = 0;\n", fmt_num(frames)));
        consts.push_str(&format!("function __setLifetime(t) {{ LIFETIME = (t != null && t > 0) ? t * {r} : -1; }}\nfunction __lifetimeStat() {{ return LIFETIME > 0 ? LIFETIME / {r} : 0; }}\n", r = fmt_num(ratio)));
        if let Some(t) = t { declared.push(format!("time_max {} ({}f here)", fmt_num(t), fmt_num(frames))); }
        // a held thing's life does not run: an item in a hand is not flying
        update.push_str(&format!("    if (LIFETIME > 0) {{ if (self.exports.__holder == null) {{ _age = _age + 1; }}\n    if (_age >= LIFETIME) {{ __takeProjDestroyedAsks(true); __unlistenOthers(); {end}; }} }}\n"));
    }
    if let Some(b) = num("bounce").filter(|b| *b != 0.0) {
        consts.push_str(&format!("var BOUNCE = {};\n", fmt_num(b * vs)));
        declared.push(format!("bounce {}", fmt_num(b)));
        if let Some(dec) = num("bounce_decay") {
            consts.push_str(&format!("var BOUNCE_DECAY = {};\n", fmt_num(dec)));
            declared.push(format!("bounce_decay {}", fmt_num(dec)));
        } else {
            consts.push_str("var BOUNCE_DECAY = 1;\n");
        }
        consts.push_str("var _bounce = 0;\n");
    }
    if flag("ghost") { declared.push("ghost".into()); }
    if flag("surviveDeathBounds") { declared.push("surviveDeathBounds".into()); }
    // the runtime write of a stat this script keeps, from its own code or from whoever holds it
    // (through its exports); one this script does not keep answers false and the write is dropped
    setters.push("if (n == \"time_max\") { __setLifetime(v); return true; }".to_string());
    consts.push_str(&format!("function __setStat(n, v) {{ {} return false; }}\n", setters.join(" ")));
    consts.push_str(crate::api_mappings::HB_CARRY);
    init.push_str("    self.exports.__setStat = __setStat;\n");

    let note = if declared.is_empty() {
        "the package declares no physics for this one -- it moves at its launch speed.".to_string()
    } else {
        format!("as its own package declares it: {}.", declared.join(", "))
    };
    ProjectilePhysics { consts, init, update, note }
}

/// The bounce handler a declared `bounce` asks for: SSF2 bounces the projectile itself, so on
/// this side the floor collision has to do it. Empty when nothing declares a bounce, and then the
/// projectile's own class decides what a floor touch means.
fn projectile_bounce_handler(d: Option<&crate::abc_parser::ProjectileData>, end: &str) -> String {
    let has = d.is_some_and(|d| d.stats.get("bounce").is_some_and(|b| *b != 0.0));
    if !has { return String::new() }
    concat!(
        "\nfunction _onBounceFloor(event) {\n",
        "    var b = BOUNCE * Math.pow(BOUNCE_DECAY, _bounce);\n",
        "    _bounce = _bounce + 1;\n",
        "    if (b < 0.5) { {{end}}; return; }\n",
        "    self.setYSpeed(-b);\n}\n").replace("{{end}}", end)
}

/// The fields a clip assigns on ITSELF. Those are the clip's own variables, not engine
/// properties: setting one through the api object throws, so they become the script's own
/// vars and are addressed without it.
pub(crate) fn own_fields_in(text: &str) -> Vec<String> {
    let mut fields: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("self.") {
        rest = &rest[i + 5..];
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let after = rest[name.len()..].trim_start();
        // never a name the engine already answers to. the decompiler writes the api object
        // through itself (`self.self`), and a `var self` declared off the back of that shadows
        // the one every call in the script goes through: nothing on it exists any more.
        if !name.is_empty() && after.starts_with('=') && !after.starts_with("==")
            && !fields.contains(&name)
            && !crate::api_mappings::RESERVED_EXT_VARS.contains(&name.as_str()) {
            fields.push(name);
        }
    }
    fields
}

/// SSF2 cancels a timer by handing back the FUNCTION it polls; this engine cancels by the uid
/// the registration handed out. A projectile's own code goes through none of the passes that
/// bridge the two, so a poll meant to stop itself never did: the cancel offered a function to
/// something that wants a number, the engine refused it, and the timer ran for as long as the
/// object lived. Which is the whole match, because the line the timer never reached was the one
/// that ends the object.
fn pair_timer_cancels_by_callback(init: &mut String, methods: &mut String, consts: &mut String) {
    let joined = format!("{init}{methods}");
    let add = crate::re!(r"(?:self\.)?addTimer\(\s*[^,()]+,\s*[^,()]+,\s*([A-Za-z_]\w*)\s*[,)]");
    let rm = crate::re!(r"(?:self\.)?removeTimer\(\s*([A-Za-z_]\w*)\s*\)");
    let registered: std::collections::BTreeSet<String> =
        add.captures_iter(&joined).map(|c| c[1].to_string()).collect();
    let cancelled: std::collections::BTreeSet<String> =
        rm.captures_iter(&joined).map(|c| c[1].to_string()).collect();
    for cb in registered.intersection(&cancelled) {
        let slot = format!("{cb}_timer");
        let cap = crate::dyn_re(&format!(
            r"(?:self\.)?addTimer\((\s*[^,()]+,\s*[^,()]+,\s*){}\s*([,)])", regex::escape(cb)));
        let can = crate::dyn_re(&format!(
            r"(?:self\.)?removeTimer\(\s*{}\s*\)", regex::escape(cb)));
        for s in [&mut *init, &mut *methods] {
            *s = cap.replace_all(s, format!("{slot} = self.addTimer(${{1}}{cb}${{2}}")).into_owned();
            *s = can.replace_all(s, format!("self.removeTimer({slot})")).into_owned();
        }
        consts.push_str(&format!("var {slot} = -1;\n"));
    }
}

/// A CALL THIS ENGINE CANNOT ANSWER. It does not fail quietly: the handler stops where it
/// stands, so everything the move still had to do never happens. The published api says what
/// the engine answers to, and a script's own functions say the rest; anything else is struck,
/// which keeps the gap visible and the rest of the move alive.
/// A projectile asking for one of its OWN stats by the source's name for it. The engine keeps
/// its own set under its own names and refuses a name it does not know, so the ask comes back
/// as nothing and whatever it fed goes wrong quietly -- blackmage's waterspout reads the speed
/// it should launch at and gets none.
///
/// The number is in the package: the same table the rest of its physics is read out of states
/// it, so the ask is answered here, in the engine's units, and never leaves.
fn answer_own_stat_asks(code: &str, d: Option<&crate::abc_parser::ProjectileData>) -> String {
    const CALL: &str = ".getProjectileStat(\"";
    let Some(d) = d else { return code.to_string() };
    let sc = &crate::mappings::character_stats().scaling;
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let open = at + CALL.len();
        let Some(q) = rest[open..].find('"') else { break };
        let key = &rest[open..open + q];
        let after = &rest[open + q + 1..];
        let Some(close) = after.find(')') else { break };
        match d.stats.get(key) {
            Some(v) => {
                // scaled the way the same key is scaled when it goes to the projectile's stats
                let scale = crate::mappings::projectile_tables().physics_map.iter()
                    .find(|m| m.ssf2 == key)
                    .map(|m| m.scale.as_str()).unwrap_or("");
                let n = match scale {
                    "velocity" => v * sc.velocity_scale(),
                    "accel" => v * sc.accel_scale(),
                    "size" => v * sc.size_multiplier,
                    _ => *v,
                };
                let start = crate::api_mappings::receiver_start_public(rest, at);
                out.push_str(&rest[..start]);
                out.push_str(&fmt_num(n));
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[..open + q + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The same pairing as inside a script, across a script and the frames that run in its scope.
/// A clip registers a poll on one frame and cancels it on another, so neither half is visible
/// to a pass that only looks at one of them, and the cancel goes out holding a function where
/// the engine wants the number its registration handed back.
pub(crate) fn pair_timers_across_frames(
    script: &mut String,
    frames: &mut std::collections::BTreeMap<u16, String>,
) {
    let joined = format!("{script}\n{}", frames.values().cloned().collect::<Vec<_>>().join("\n"));
    let add = crate::re!(r"(?:self\.)?addTimer\(\s*[^,()]+,\s*[^,()]+,\s*([A-Za-z_]\w*)\s*[,)]");
    let rm = crate::re!(r"(?:self\.)?removeTimer\(\s*([A-Za-z_]\w*)\s*\)");
    let registered: std::collections::BTreeSet<String> =
        add.captures_iter(&joined).map(|c| c[1].to_string()).collect();
    let cancelled: std::collections::BTreeSet<String> =
        rm.captures_iter(&joined).map(|c| c[1].to_string()).collect();
    for cb in registered.intersection(&cancelled) {
        let slot = format!("{cb}_timer");
        // the script may already keep this one, from the pass that saw only itself. the frames
        // still have not been told, and they are the half that was missing.
        let already = script.contains(&format!("var {slot} "));
        let cap = crate::dyn_re(&format!(
            r"(?:self\.)?addTimer\((\s*[^,()]+,\s*[^,()]+,\s*){}\s*([,)])", regex::escape(cb)));
        let can = crate::dyn_re(&format!(
            r"(?:self\.)?removeTimer\(\s*{}\s*\)", regex::escape(cb)));
        let fix = |s: &str| {
            let s = cap.replace_all(s, format!("{slot} = self.addTimer(${{1}}{cb}${{2}}")).into_owned();
            can.replace_all(&s, format!("self.removeTimer({slot})")).into_owned()
        };
        *script = fix(script);
        for code in frames.values_mut() { *code = fix(code); }
        if !already { script.insert_str(0, &format!("var {slot} = -1;\n")); }
    }
}

/// Comment a line out without taking the shape of the code around it: a struck `if (…) {` leaves
/// its closing brace with nothing to close and the script stops PARSING, so nothing in it runs.
/// Only the braces that HOLD shape are put back -- the ones the line closes before it says
/// anything, and one it leaves open at the end.
fn strike_line(line: &str, note: &str) -> String {
    let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    let struck = format!("{indent}// [{note}] {}", line.trim());
    // A LINE THAT FILLS A SLOT. Striking it leaves the slot empty, and everything downstream
    // that reads a field off it fails instead -- so the strike moves the failure rather than
    // removing it. wario asks a foe it is holding which buttons are held, cannot be told, and
    // then reads a button off the answer. Filled with an empty thing, that read comes back with
    // nothing, which is what the source's own test makes of it.
    let live = line.split("//").next().unwrap_or(line).trim();
    if live.ends_with(");") {
        if let Some(head) = live.strip_suffix(");") {
            if let Some((slot, _)) = head.split_once(".set(") {
                if !slot.is_empty()
                    && slot.chars().all(|c| c.is_alphanumeric() || "._()".contains(c))
                {
                    return format!("{struck}\n{indent}{slot}.set({{}});");
                }
            }
        }
    }
    let code = line.split("//").next().unwrap_or(line);
    let closes = code.chars().take_while(|c| c.is_whitespace() || *c == '}')
        .filter(|c| *c == '}').count();
    let ends_open = code.trim_end().ends_with('{');
    if closes == 0 && !ends_open { return struck }
    let mut rebuilt = struck;
    rebuilt.push('\n');
    rebuilt.push_str(&indent);
    for _ in 0..closes { rebuilt.push('}'); }
    if ends_open {
        if closes > 0 { rebuilt.push(' '); }
        rebuilt.push_str("if (false) {");
    }
    rebuilt
}

/// A CALL ON SOMETHING THE SCRIPT HOLDS, by a name nobody has. A script keeps what the engine
/// made for it in its own variables (a platform it gave itself) and asks it things the source's
/// engine answered (`platform.addIgnoreObject(...)`); a name this engine has no method by, that
/// the script does not define and no factory of its own installs, is answered by nobody, and
/// asked anyway it stops the handler. The same rule `self.x()` is held to, on a held thing.
pub(crate) fn strike_calls_on_held_things(script: &str) -> String {
    // AN EFFECT PUT ON A THING THE SCRIPT HOLDS is this engine's to make, with that thing as its
    // owner: translated here, before the pass below turns the call into one script asking another
    // for a function it does not publish (and then into nothing at all).
    let script = &crate::api_mappings::rewrite_attach_effect_calls(script);
    let cmds = crate::mappings::api_commands();
    let api: std::collections::BTreeSet<&str> =
        crate::mappings::fm_api().methods.iter().map(|s| s.as_str())
        .chain(cmds.passthrough_fm_apis.iter().map(|p| p.name.as_str())).collect();
    if api.is_empty() { return script.to_string() }
    let std_names: std::collections::BTreeSet<&str> = crate::mappings::fm_api()
        .standard_methods.iter().map(|s| s.as_str()).collect();
    let mut own: std::collections::BTreeSet<String> = script.lines()
        .filter_map(|l| l.strip_prefix("function "))
        .filter_map(|l| l.split('(').next())
        .map(|n| n.trim().to_string())
        .collect();
    // what a factory of the script's own installs on what it builds (`it.tick = function`)
    for c in crate::re!(r"\.(\w+)\s*=\s*function").captures_iter(script) { own.insert(c[1].to_string()); }
    let holders: Vec<String> = script.lines()
        .filter_map(|l| l.strip_prefix("var "))
        .filter_map(|l| l.split([' ', '=', ';']).next())
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && !n.starts_with("__"))
        .collect();
    if holders.is_empty() { return script.to_string() }
    let alt = holders.iter().map(|h| regex::escape(h)).collect::<Vec<_>>().join("|");
    let call = crate::dyn_re(&format!(r"(?:^|[^\w.$])(?:{alt})\.(\w+)\("));
    // a call on a thing a slot holds (`item.get().explode()`), by a name this engine's handles
    // do not answer: it is the held thing's to answer, through what it publishes, and nothing
    // when it publishes none. the holder's own function of that name is not the held thing's
    // (sandbag's own `explode` is not its bomb's)
    let held_call = crate::dyn_re(&format!(r"\b({alt})\.get\(\)\.(\w+)\(([^()]*)\)"));
    let script = held_call.replace_all(script, |c: &regex::Captures| {
        let (h, name, args) = (&c[1], &c[2], &c[3]);
        if api.contains(name) || std_names.contains(name) { return c[0].to_string() }
        format!("(({h}.get() != null && {h}.get().exports != null && {h}.get().exports.{name} != null) ? {h}.get().exports.{name}({args}) : null)")
    }).into_owned();
    let script = script.as_str();
    script.lines().map(|line| {
        if line.trim_start().starts_with("//") { return line.to_string() }
        let code = line.split("//").next().unwrap_or(line);
        let unanswered: Vec<&str> = call.captures_iter(code)
            .map(|c| c.get(1).unwrap().as_str())
            .filter(|n| !api.contains(n) && !std_names.contains(n) && !own.contains(*n))
            .collect();
        if unanswered.is_empty() { line.to_string() }
        else { strike_line(line, &format!("no counterpart: {}", unanswered.join(", "))) }
    }).collect::<Vec<_>>().join("\n")
}

/// The names this converter writes for itself, as they appear in a piece of code: everything it
/// emits is marked the same way, and none of it is the engine's to answer.
fn bare_double_underscore_calls(script: &str) -> Vec<&str> {
    let re = crate::re!(r"(^|[^\w.$])(__\w+)\s*\(");
    re.captures_iter(script).filter_map(|c| c.get(2).map(|m| m.as_str())).collect()
}

pub(crate) fn strike_calls_the_engine_cannot_answer(script: &str, also_own: &[String]) -> String {
    // the engine's own names, and the names the mapping recognises and rewrites later
    // (`setIntangibility` becomes a body status once its frames are paired): those are
    // answered, not struck
    let cmds = crate::mappings::api_commands();
    let api: std::collections::BTreeSet<&str> =
        crate::mappings::fm_api().methods.iter().map(|s| s.as_str())
        .chain(cmds.passthrough_fm_apis.iter().map(|p| p.name.as_str())).collect();
    if api.is_empty() { return script.to_string() }
    // what the script itself declares, plus what the scope it runs in declares: a frame script
    // is part of the script it sits in and calls its functions by name.
    let mut own: std::collections::BTreeSet<&str> = script.lines()
        .filter_map(|l| l.strip_prefix("function "))
        .filter_map(|l| l.split('(').next())
        .map(|n| n.trim())
        .collect();
    // and what it declares, since a read reaches those as readily as a call reaches a function
    own.extend(script.lines()
        .filter_map(|l| l.strip_prefix("var "))
        .filter_map(|l| l.split([' ', '=', ';']).next())
        .map(|n| n.trim())
        .filter(|n| !n.is_empty()));
    own.extend(also_own.iter().map(|s| s.as_str()));
    // and the helpers written here: they are put on the script around this code, which this pass
    // does not see, and they all carry the same mark
    let ours: Vec<&str> = bare_double_underscore_calls(script);
    own.extend(ours);
    // `self.x()` is the engine being asked directly. `thing.get().x()` is a wrapper being read
    // for the entity it holds and then asked -- also the engine, also unanswerable if the name
    // is not one it has. Anything else (an array's push, Math, a wrapper's own getInt) is not
    // the engine's to answer and is left alone.
    // A call the engine answers to, written with nothing in front of it. The source is standing
    // ON the object, so it just says the name; here the name alone is nobody's, and the line
    // fails as an unknown variable. Whose it is is not in doubt -- it is the engine's, or the
    // script would have declared it.
    let bare = crate::re!(r"(^|[^\w.$])([A-Za-z_]\w*)\(");
    let script = &script.lines().map(|line| {
        if line.trim_start().starts_with("//") { return line.to_string() }
        bare.replace_all(line, |c: &regex::Captures| {
            let name = &c[2];
            if api.contains(name) && !own.contains(name) {
                format!("{}self.{name}(", &c[1])
            } else {
                format!("{}{name}(", &c[1])
            }
        }).into_owned()
    }).collect::<Vec<_>>().join("\n");

    // Only `self.x()`, which is the engine asked directly. A wrapper read holds whatever was put
    // in it -- an array as readily as an entity -- so `thing.get().push()` is an array being
    // asked, and striking that removes working code.
    // `self.x()` and `match.x()` are both the engine asked directly, and both certain: a name
    // neither answers to is answered by nobody.
    let re = crate::re!(r"(?:self|match|(?:^|[^\w.])stage)\.(\w+)\(");
    // A call on a wrapper READ. What is in the slot could be anything, so this is only safe for a
    // name NOBODY has -- not the engine under any of its types, and not an array or a string or
    // one of these wrappers. `thing.get().push()` is an array being told to push and stays.
    let read = crate::re!(r"\.get\(\)\.(\w+)\(");
    let std_names: std::collections::BTreeSet<&str> = crate::mappings::fm_api()
        .standard_methods.iter().map(|s| s.as_str()).collect();
    let script = &ask_other_objects_through_exports(script, also_own);

    // REACHING INTO A CHILD THAT IS NOT THERE. In Flash a clip has clips inside it, named, and
    // the source reaches through them: `self.notes.n1`. Here an entity has no children to reach,
    // so the read is of nothing and the line goes the way an unanswerable call does.
    let member = crate::re!(r"self\.(\w+)\.");
    script.lines().map(|line| {
        let code = line.split("//").next().unwrap_or(line);
        let unanswered: Vec<&str> = re.captures_iter(code)
            .chain(member.captures_iter(code))
            .chain(read.captures_iter(code).filter(|c| {
                let n = c.get(1).unwrap().as_str();
                !std_names.contains(n)
            }))
            .map(|c| c.get(1).unwrap().as_str())
            .filter(|n| !api.contains(n) && !own.contains(n) && *n != "exports")
            .collect();
        if unanswered.is_empty() || line.trim_start().starts_with("//") {
            line.to_string()
        } else {
            log::warn!("'{}' is not something this engine answers to, so the line asking for it \
                is struck (a call it cannot answer stops the whole handler)", unanswered.join(", "));
            strike_line(line, &format!("no counterpart: {}", unanswered.join(", ")))
        }
    }).collect::<Vec<_>>().join("\n")
}

/// A call on another object by a name nobody here has, asked through what that object publishes.
pub(crate) fn ask_other_objects_through_exports(script: &str, also_own: &[String]) -> String {
    let cmds = crate::mappings::api_commands();
    let api: std::collections::BTreeSet<&str> =
        crate::mappings::fm_api().methods.iter().map(|s| s.as_str())
        .chain(cmds.passthrough_fm_apis.iter().map(|p| p.name.as_str())).collect();
    if api.is_empty() { return script.to_string() }
    let std_names: std::collections::BTreeSet<&str> = crate::mappings::fm_api()
        .standard_methods.iter().map(|s| s.as_str()).collect();
    let mut own: std::collections::BTreeSet<&str> = crate::re!(r"\bfunction\s+(\w+)")
        .captures_iter(script).map(|c| c.get(1).unwrap().as_str()).collect();
    own.extend(also_own.iter().map(|s| s.as_str()));
    // A CALL ON ANOTHER OBJECT BY A NAME NOBODY HERE HAS. The source asked the other object
    // directly (the summoner to play one of its voice clips, a foe to break, a projectile to stop
    // falling); here an entity answers only the engine's names, and anything of its own it offers
    // through what it publishes. So it is asked there, and when it publishes nothing by that name
    // the call does nothing instead of stopping the handler it sits in. A name the script defines
    // anywhere (a function, a field it gives a function) is the script's own business, and left.
    let fn_fields: std::collections::BTreeSet<&str> = crate::re!(r"\b(\w+)\s*[:=]\s*function\b")
        .captures_iter(script).map(|c| c.get(1).unwrap().as_str()).collect();
    let head = crate::re!(r"(^|[^\w.$])((?:[a-z_]\w*)(?:\(\))?(?:\.[a-z_]\w*(?:\(\))?)*)\.(\w+)\(");
    let skip = |recv: &str, name: &str| {
        let first = recv.split('.').next().unwrap_or("");
        // a slot read (`hydrant.get()`) is the thing it holds, asked the same way; a bare `.get` is
        // a wrapper being handed over, not called
        first == "self" || first == "match" || first == "stage" || recv.ends_with("exports") || recv.ends_with(".get")
            || (recv.ends_with("()") && !recv.ends_with(".get()"))
            || api.contains(name) || std_names.contains(name) || own.contains(name)
            || fn_fields.contains(name) || name.starts_with("__")
    };
    script.lines().map(|line| {
        if line.trim_start().starts_with("//") { return line.to_string() }
        rewrite_calls_on_others(line, head, &skip)
    }).collect::<Vec<_>>().join("\n")
}

/// Each `recv.name(args)` on a line whose name `skip` does not keep, asked through `recv`'s exports;
/// the argument list is found by counting parens (outside strings), so any nesting is carried.
fn rewrite_calls_on_others(line: &str, head: &regex::Regex, skip: &dyn Fn(&str, &str) -> bool) -> String {
    let mut out = String::with_capacity(line.len());
    let mut pos = 0;
    while let Some(c) = head.captures_at(line, pos) {
        let (recv, name) = (c[2].to_string(), c[3].to_string());
        let open = c.get(0).unwrap().end();
        // the matching close paren
        let b = line.as_bytes();
        let (mut depth, mut i, mut quote): (i32, usize, Option<u8>) = (1, open, None);
        while i < b.len() && depth > 0 {
            let ch = b[i];
            match quote {
                Some(q) => { if ch == b'\\' { i += 1 } else if ch == q { quote = None } }
                None => match ch { b'"' | b'\'' => quote = Some(ch), b'(' => depth += 1, b')' => depth -= 1, _ => {} },
            }
            i += 1;
        }
        if depth != 0 || skip(&recv, &name) {
            // keep through the receiver and name, and look again inside the arguments
            out.push_str(&line[pos..open]);
            pos = open;
            continue;
        }
        let args = &line[open..i - 1];
        // what it was handed still happens (yoshi's egg hands over the explosion it makes)
        let otherwise = if args.trim().is_empty() { "null".to_string() } else { format!("__nothing([{args}])") };
        out.push_str(&line[pos..c.get(2).unwrap().start()]);
        out.push_str(&format!("(({recv} != null && {recv}.exports != null && {recv}.exports.{name} != null) ? {recv}.exports.{name}({args}) : {otherwise})"));
        pos = i;
    }
    out.push_str(&line[pos..]);
    out
}

/// Where a projectile's published methods go, filled in by `publish_projectile_methods`.
pub(crate) const PUBLISH_METHODS_MARK: &str = "// __publish_methods__:";

/// The methods a projectile publishes, written in place of their mark.
pub(crate) fn publish_projectile_methods(script: &str) -> String {
    let re = crate::dyn_re(&format!(r"(?m)^(\s*){}([\w,]+)\s*$", regex::escape(PUBLISH_METHODS_MARK)));
    re.replace_all(script, |c: &regex::Captures| {
        let ind = &c[1];
        let mut out = format!("{ind}// what whoever holds this can ask of it\n");
        for m in c[2].split(',').filter(|m| !m.is_empty()) { out.push_str(&format!("{ind}self.exports.{m} = {m};\n")); }
        out.trim_end().to_string()
    }).into_owned()
}

pub(crate) fn generate_projectile_script(
    _char_id: &str,
    entity_id: &str,
    extra_states: &[entity_gen::ProjectileStateData],
    ssf2_match: Option<&crate::abc_parser::ProjectileData>,
    behaviour: &std::collections::BTreeMap<String, String>,
    has_destroy_anim: bool,
    frame_script_text: &str,
    idle_anim: &str,
    kept_by_summoner: bool,
    class_consts: &[(String, String)],
    owner_anims: &std::collections::BTreeMap<String, String>,
    label_flow: &str,
    parts: (&str, &str),
) -> String {
    let t = &crate::mappings::script_templates().projectile.script;
    // the labels its states answer to, as the script's own state names
    let lstates: std::collections::BTreeMap<String, String> = projectile_lstates(
        &extra_states.iter().map(|s| s.label.clone()).collect::<Vec<_>>())
        .into_iter().map(|(label, lname, _)| (label, lname)).collect();
    let req = crate::mappings::require_template;
    let (x_speed, y_speed, speed_note) = synth_projectile_launch(ssf2_match);
    // A clip with its own destroy animation ends by playing it; one that has a single animation
    // has nothing to play, and a projectile parked in DESTROYING with a looping animation never
    // leaves (the engine says so out loud after 20 seconds).
    // `kill`, which is what this engine calls it. `destroy` is the source's word and the
    // rename handles it everywhere a script is translated -- but this is written here, not
    // translated, so it went out as a call to a method the engine does not have. Every
    // projectile with no death animation to play tried to end itself with it and could not:
    // twenty-one scripts asking for something nobody answers.
    let end = if has_destroy_anim { "self.toState(PState.DESTROYING)" } else { "self.kill()" };
    let phys = projectile_physics(ssf2_match, entity_id, x_speed, y_speed, end);

    // The projectile's OWN code. `frame1` is what its timeline runs the moment it exists, which
    // is where SSF2 clips wire up what happens to them, so that body opens `initialize`. Anything
    // else the class declares is a handler that wiring points at, and comes over as a function.
    let translate = |code: &str| {
        // the package's own numbers, in place of asking the engine for a name it does not keep
        let code = &answer_own_stat_asks(code, ssf2_match);
        // The clip's own fields (`self.character = …`) are the CLIP's variables, not engine
        // properties -- assigning them through the api object would throw. They become the
        // script's own vars, declared below, the same way a character's class fields do.
        // `destroy` is a handler the source hands to a listener by name, so it needs to exist.
        // For a PROJECTILE, the object and the attack are the same thing, so "my attack struck
        // terrain" is this entity hitting a wall -- which Fraymakers does define. The global rule
        // drops that subscription because on a CHARACTER the two are different moments.
        let code = code
            .replace("SSF2Event.HIT_DEALT_WALL", "EntityEvent.COLLIDE_WALL")
            .replace("SSF2Event.HIT_WALL", "EntityEvent.COLLIDE_WALL");
        // the fighter's labels are the fighter's: a projectile's clip is cut into animations by
        // its own labels, so a jump to one is that animation, not a jump inside the fighter's
        crate::api_mappings::set_current_labels(&[]);
        // a label it asks its summoner to jump to: read before the translation, which would
        // read the ask as the summoner's own animation change
        let code = crate::api_mappings::rewrite_owner_label_jumps(&code);
        let out = crate::api_mappings::as_projectile(|| crate::api_mappings::guard_null_comparisons(&crate::api_mappings::rewrite_current_frame_reads(&crate::api_mappings::translate_ssf2_to_fm(&crate::api_mappings::rewrite_current_frame_reads(&crate::api_mappings::rewrite_attack_stat_reads(&crate::api_mappings::rewrite_clip_label_jumps(&code)))))));
        let out = crate::api_mappings::rename_self_destroy(&out);
        // a move of its own to another of its states, and the summoner's move asked by the
        // source's name
        let out = crate::api_mappings::rewrite_projectile_force_attack(&out, &lstates);
        // the one it belongs to asked to force one of its moves (a boomerang caught)
        let out = crate::api_mappings::rewrite_force_attack_on_others(&out, owner_anims);
        crate::api_mappings::rename_animation_literals(&out, owner_anims)
    };
    let mut source_init = String::new();
    let mut source_methods = String::new();
    let mut skipped: Vec<&str> = Vec::new();
    // the behaviour class's own entry points, which the template's already-named functions
    // would otherwise shadow: kept under their own names and called from the template's
    let mut class_init_call = String::new();
    let mut class_update_call = String::new();
    let mut has_frame1 = false;
    let says_nothing = |code: &str| code.lines().filter(|l| { let t = l.trim(); !t.is_empty() && t != "return;" && !t.starts_with("//") && t != "}" && !t.starts_with("function ") }).count() == 0;
    for (name, code) in behaviour {
        if name == "initialize" || name == "update" {
            if says_nothing(code) { continue }
            let renamed = if name == "initialize" { "__classInitialize" } else { "__classUpdate" };
            let body = translate(code).replacen(&format!("function {name}("), &format!("function {renamed}("), 1);
            source_methods.push('\n');
            source_methods.push_str(body.trim_end());
            source_methods.push('\n');
            if name == "initialize" { class_init_call = "    __classInitialize();\n".to_string(); }
            else { class_update_call = "    __classUpdate();\n".to_string(); }
            continue;
        }
        if name == "frame1" {
            // the clip's first frame's script, run at the projectile's first update unless a label
            // was asked at the spawn (the source's playhead reaches such a label before frame 1)
            let body = translate(code).replacen("function frame1(", "function __frame1(", 1);
            source_methods.push('\n');
            source_methods.push_str(body.trim_end());
            source_methods.push('\n');
            has_frame1 = true;
            continue;
        }
        if crate::abc_parser::is_root_xframe_method(name) {
            // a later frame's script belongs to a FRAME, and that is where it is emitted: on the
            // projectile entity's own timeline (see ProjectileInfo::frame_scripts). Naming it
            // here would emit a function nothing calls.
            skipped.push(name);
        } else {
            source_methods.push('\n');
            source_methods.push_str(translate(code).trim_end());
            source_methods.push('\n');
        }
    }
    if !has_frame1 { source_methods.push_str("\nfunction __frame1() {}\n"); }
    // Fields the source assigns on itself: declared as the script's own, and addressed without
    // the api object.
    // Its timeline scripts count too: a clip keeps state across its own frames as often as
    // inside a handler, and a field only ever set from a frame is still one of its own.
    let own_fields: Vec<String> =
        own_fields_in(&format!("{source_init}{source_methods}{frame_script_text}"));
    for f in &own_fields {
        source_init = source_init.replace(&format!("self.{f}"), f);
        source_methods = source_methods.replace(&format!("self.{f}"), f);
    }
    // the behaviour class's constants: declared with the values it states, read as bare names
    for (n, _) in class_consts {
        source_init = source_init.replace(&format!("self.{n}"), n);
        source_methods = source_methods.replace(&format!("self.{n}"), n);
    }
    // A handler the source hands to a listener is one of its own methods, and those come over as
    // plain functions, so the reference has to lose the api object too.
    for name in behaviour.keys().filter(|n| !crate::abc_parser::is_root_xframe_method(n)) {
        source_init = source_init.replace(&format!("self.{name}"), name);
        source_methods = source_methods.replace(&format!("self.{name}"), name);
    }
    let mut consts = phys.consts.clone();
    // The same reading of a value as a test the character's own script gets. A projectile's
    // code asks `if (thing)` exactly as often, and hscript reads only a bool -- so without
    // this the condition never fires and, worse, the name is not there at all: the script
    // stops on it. Sixty-nine of them ask for it.
    // always: the value helpers below read through it
    consts.insert_str(0,
        "function __truthy(v) { return !(v == null || v == false || v == 0); }\n\n");
    // a projectile that states a speed of its own, on its timeline as often as in its methods
    // ...and the null-safe compares its methods go through (a projectile comparing a slot nothing
    // set threw `Unknown variable: __le`: dedede's gordo)
    let declared_rotate = ssf2_match.map(|d| d.flags.get("rotate").copied().unwrap_or(false)).unwrap_or(false);
    // the id this projectile's declaration carries, what its code and its owner's compare against
    let own_linkage = ssf2_match.and_then(|d| d.names.get("linkage_id").cloned()).unwrap_or_else(|| entity_id.to_string());
    consts.insert_str(0, &format!("var __ownLinkage = \"{own_linkage}\";\n"));
    source_init.insert_str(0, "    // the launch its owner's engine gives it, taken up before its own code runs (which may change it)\n    { var __lo = self.getOwner(); if (__lo != null && __lo.exports != null && __lo.exports.__launch != null) { var __l = __lo.exports.__launch; __lo.exports.__launch = null; self.setX(__l.x); self.setY(__l.y); if (__l.xs != null) { self.setXSpeed(__l.xs); self.setYSpeed(__l.ys); } } }\n    self.exports.__linkageId = __ownLinkage;\n    // the timers its code started end with the state they were started in\n    self.addEventListener(EntityEvent.STATE_CHANGE, __flushSrcTimers, { persistent: true });\n    // its lifetime, which whoever holds it may set or ask (kirby charges his flame's)\n    self.exports.__setLifetime = __setLifetime;\n    self.exports.__lifetimeStat = __lifetimeStat;\n");
    // the speed the item declares it is thrown with, for the holder's throw
    if let Some(ts) = ssf2_match.and_then(|d| d.stats.get("tossSpeed").copied()) {
        source_init.insert_str(0, &format!("    self.exports.__tossSpeed = {ts};\n"));
    }
    // an effect its code names as it runs, from its methods or its timeline
    if crate::api_mappings::names_effect_at_runtime(&format!("{source_init}{source_methods}{frame_script_text}")) {
        consts.push_str(&attach_effect_helper());
    }
    consts.push_str(label_flow);
    // the named children its code addresses: made as the object is, before its own code runs
    consts.push_str(parts.0);
    source_init.insert_str(0, parts.1);
    consts.insert_str(0, &format!("{}{}{}\n// a projectile's labels are its animations\nfunction _currentLabel(who) {{ return who == null ? \"\" : who.getAnimation(); }}\n// and a jump to one plays it: from its own first frame it is already there (a script on that\n// frame jumping to its own label would otherwise start it again, and again)\nfunction __playOwn(n) {{ if (n == self.getAnimation() && self.getCurrentFrame() <= 1) {{ return; }} self.playAnimation(n); }}\n// a projectile declares no air ease of its own, and its writes are not stepped by a gravity (unmeasured)\nfunction __declaredAirEase() {{ return -1; }}\nfunction __declaredAttackBase() {{ return null; }}\nfunction __declaredMotionStats() {{}}\nfunction __applyXDecay() {{}}\n// whether its declaration turns it to face the way it flies\nfunction __declaredRotate() {{ return {}; }}\nfunction __gStep() {{ return 0; }}\nfunction __xWrite(v) {{ return __sv(v); }}\n", crate::api_mappings::speed_boundary_helpers(), crate::api_mappings::entity_clock_helpers(), crate::api_mappings::NULL_COMPARE, declared_rotate));
    consts.push_str(&crate::api_mappings::object_match_helpers(&format!("{source_init}\n{source_methods}\n{frame_script_text}")));
    if ["__typeName(", "__callByName(", "__srcStateIs(", "__calculateKnockback(", "__declaredStat("].iter().any(|n| source_init.contains(n) || source_methods.contains(n) || frame_script_text.contains(n)) {
        consts.push_str(&crate::api_mappings::runtime_type_helpers());
    }
    if ["__hbStat(", "__hbNote("].iter().any(|n| source_init.contains(n) || source_methods.contains(n) || frame_script_text.contains(n)) {
        // its own declared boxes, under the animation it flies with
        let (mut an, mut ix, mut ky, mut vl) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        if let Some(d) = ssf2_match {
            for (i, hb) in d.hitboxes.iter().enumerate() {
                for (k, v) in hb {
                    an.push(format!("\"{idle_anim}\"")); ix.push(i.to_string()); ky.push(format!("\"{k}\"")); vl.push(fmt_num(*v));
                }
            }
        }
        consts.push_str(&format!("var _hbAnim = [{}];\nvar _hbIdx = [{}];\nvar _hbKey = [{}];\nvar _hbVal = [{}];\n", an.join(", "), ix.join(", "), ky.join(", "), vl.join(", ")));
        consts.push_str(&crate::api_mappings::hitbox_read_helpers());
    }
    // the same store the character writes into: a projectile that reads a global is reading
    // what its summoner put there, so it has to reach the one place both can see.
    // the same question a character can ask, asked from a projectile: it is the match's list
    // either way, so the projectile carries the walk of it too.
    if consts.contains("_currentProjectile(") || source_init.contains("_currentProjectile(")
        || source_methods.contains("_currentProjectile(") || frame_script_text.contains("_currentProjectile(")
    {
        consts.insert_str(0, "function _currentProjectile(who) {\n\
\tvar all = match.getProjectiles();\n\
\tif (all == null) { return null; }\n\
\tvar __i = all.length - 1;\n\
\twhile (__i >= 0) {\n\
\t\tif (all[__i].getOwner() == who) { return all[__i]; }\n\
\t\t__i = __i - 1;\n\
\t}\n\
\treturn null;\n}\n\n");
    }
    if consts.contains("_attackStat(") || source_init.contains("_attackStat(")
        || source_methods.contains("_attackStat(") || frame_script_text.contains("_attackStat(")
    {
        // A projectile scales itself by the move that fired it (`_attackStat(owner, "chargetime_max")`
        // -> link's charged arrow). The lookup lives on the character; here the projectile does it
        // against the character's attack tables, which the summoner hands over on its exports. Read
        // only, so it never leaves the projectile's own scope (calling the character's function
        // across scripts would). The same two-pass match the character's own version does: an exact
        // animation+key hit first, then a prefix hit.
        consts.insert_str(0, "function _attackStat(who, key) {\n\
\tif (who == null || who.exports == null) { return null; }\n\
\tvar an = who.exports._atkAnim; var ky = who.exports._atkKey; var vl = who.exports._atkVal;\n\
\tif (an == null || ky == null || vl == null) { return null; }\n\
\tvar a = who.getAnimation();\n\
\tvar __i = 0;\n\
\twhile (__i < an.length) { if (an[__i] == a && ky[__i] == key) { return vl[__i]; } __i = __i + 1; }\n\
\t__i = 0;\n\
\twhile (__i < an.length) { if (ky[__i] == key && a != null && a.indexOf(an[__i]) == 0) { return vl[__i]; } __i = __i + 1; }\n\
\treturn null;\n}\n\n");
    }
    if consts.contains("_pick(") || source_init.contains("_pick(")
        || source_methods.contains("_pick(") || frame_script_text.contains("_pick(")
    {
        consts.insert_str(0, crate::api_mappings::TABLE_PICK);
        consts.insert(0, '\n');
    }
    if consts.contains("_getGlobal(") || source_init.contains("_getGlobal(")
        || source_methods.contains("_getGlobal(") || frame_script_text.contains("_getGlobal(")
        || consts.contains("_setGlobal(") || source_init.contains("_setGlobal(")
        || source_methods.contains("_setGlobal(") || frame_script_text.contains("_setGlobal(")
    {
        consts.insert_str(0, crate::api_mappings::GLOBAL_STORE);
        consts.insert(0, '\n');
    }
    // Declared WITH a value, always. A script variable with no initialiser breaks the engine as
    // it takes the object's state -- "Null access .e" the moment the projectile spawns, and the
    // script stops there, so mario's fireball never registered its bounce and lay where it landed.
    for f in &own_fields {
        if class_consts.iter().any(|(n, _)| n == f) { continue }
        consts.push_str(&format!("var {f} = null;\n"));
    }
    for (n, v) in class_consts { consts.push_str(&format!("var {n} = {v};\n")); }
    // a label a clip asks its summoner to jump to: signalled, since a call across scripts loses
    // the caller's own helpers, and the summoner takes it up on its next frame
    if source_init.contains("__askLabel(") || source_methods.contains("__askLabel(") || frame_script_text.contains("__askLabel(") {
        consts.push_str(crate::api_mappings::ASK_LABEL);
    }
    // WHAT THE SUMMONER CAN SET ON IT. In the source the summoner reaches into the projectile
    // and writes the field, because both are fields on one object. Here a script's variables are
    // its own and what crosses is what it publishes -- and these are plain variables, so what is
    // published is a way to set one, not the slot itself.
    // ...and a field the class only DECLARES is set from outside as readily (pacman writes
    // which way its hydrant faces the frame it makes one), so those publish a setter too
    let settable: Vec<&String> = own_fields.iter()
        .chain(class_consts.iter().map(|(n, _)| n).filter(|n| !own_fields.contains(n)))
        .collect();
    if !settable.is_empty() {
        for f in &settable {
            source_methods.push_str(&format!("\nfunction _set_{f}(v) {{ {f} = v; }}\n"));
        }
        let mut publish = String::from("    // what whoever summons this can set on it\n");
        for f in &settable {
            publish.push_str(&format!("    self.exports.set_{f} = _set_{f};\n"));
        }
        source_init.insert_str(0, &publish);
    }
    // ...and what it can be ASKED: its class's own methods (pacman's hydrant fixes its colours
    // and learns who owns it when its summoner calls it), as a fighter publishes its own
    {
        let mut methods: Vec<&String> = behaviour.keys()
            .filter(|n| !crate::abc_parser::is_root_xframe_method(n) && n.chars().all(|c| c.is_alphanumeric() || c == '_'))
            .filter(|n| source_methods.contains(&format!("function {n}(")))
            .collect();
        methods.sort();
        methods.dedup();
        if !methods.is_empty() {
            // written out once the summoner's state has been routed, which would read a method
            // named like one of the summoner's fields as that field (lloyd's `done`)
            let names: Vec<&str> = methods.iter().map(|m| m.as_str()).collect();
            source_init.insert_str(0, &format!("    {PUBLISH_METHODS_MARK}{}\n", names.join(",")));
        }
    }
    // the live-instance cap (empty unless the package declares a `limit`) runs FIRST in initialize,
    // before the source's own frame1 body -- which may `return;` early, leaving anything after it dead
    if !phys.init.is_empty() {
        source_init.insert_str(0, &phys.init);
    }
    pair_timer_cancels_by_callback(&mut source_init, &mut source_methods, &mut consts);
    if source_methods.contains("_destroySelf") || source_init.contains("_destroySelf") {
        source_methods.push_str(&format!("\nfunction _destroySelf(event) {{\n    {end};\n}}\n"));
    }
    if !skipped.is_empty() {
        source_methods.push_str(&format!(
            "\n// its timeline scripts are on the entity's own timeline, where they belong: {}\n", skipped.join(", ")));
    }
    // Without its own code a projectile needs somewhere to die.
    let (bounce_listener, bounce_handler) = match projectile_bounce_handler(ssf2_match, end) {
        h if h.is_empty() => (String::new(), String::new()),
        h => ("    self.addEventListener(EntityEvent.COLLIDE_FLOOR, _onBounceFloor, { persistent: true });\n".to_string(), h),
    };
    // Only for one that flies off on its own. A projectile the summoner KEEPS is steered by
    // whoever keeps it, and these would kill it the first time it touched the floor: telly
    // vision is meant to sit on the stage for the match, and it went into DESTROYING on
    // landing and stayed there until the engine complained about it.
    if behaviour.is_empty() && !kept_by_summoner {
        source_init.push_str(req("projectile.script.default_listeners", &t.default_listeners));
        source_methods.push_str(req("projectile.script.default_handlers", &t.default_handlers));
    }

    if extra_states.is_empty() {
        req("projectile.script.single_state", &t.single_state)
            .replace("{{entity_id}}", entity_id)
            .replace("{{constants}}", &consts)
            .replace("{{physics_note}}", &phys.note)
            // Listening for a reflection is only right for a projectile that says it can be
            // reversed. Enabled for every one of them, a final smash beam could be turned around.
            .replace("{{reflection_listener}}",
                if projectile_flag(ssf2_match, "canBeReversed") {
                    "    Common.enableReflectionListener({ mode: \"X\", replaceOwner: true });\n"
                } else {
                    "    // its package says it cannot be reversed, so nothing listens for one\n"
                })
            .replace("{{physics_update}}", &phys.update)
            .replace("{{class_update}}", &class_update_call)
            .replace("{{death_watch}}", req("projectile.script.death_watch", &t.death_watch))
            .replace("{{death_watch_handler}}", req("projectile.script.death_watch_handler", &t.death_watch_handler))
            .replace("{{bounce_listener}}", &bounce_listener)
            .replace("{{bounce_handler}}", &bounce_handler)
            // the class's own initializer first: in the source it runs when the object is made,
            // the timeline's first frame after (and that frame ends with a return that would
            // have left the call behind it unreached: rayman's plum never built its timers)
            .replace("{{source_init}}", &format!("{class_init_call}{source_init}"))
            .replace("{{source_methods}}", &source_methods)
            .replace("{{speed_note}}", &speed_note)
    } else {
        // Multi-state: use Fraymakers local state machine instead of fake PStates
        // Each SSF2 frame label becomes an LState that drives animation switching. What a state
        // DOES is on its own frames (the entity carries them): nothing is invented for it here.
        let mut lstate_prep = req("projectile.script.lstate_idle_prep", &t.lstate_idle_prep)
            .replace("{{fm_anim}}", idle_anim);
        let line_tpl = req("projectile.script.lstate_prep_line", &t.lstate_prep_line);
        for (label, lname, fm) in projectile_lstates(&extra_states.iter().map(|s| s.label.clone()).collect::<Vec<_>>()) {
            if label == "attack_idle" { continue }
            lstate_prep.push_str(&line_tpl.replace("{{lstate_name}}", &lname).replace("{{fm_anim}}", &fm));
        }
        req("projectile.script.multi_state", &t.multi_state)
            .replace("{{entity_id}}", entity_id)
            .replace("{{constants}}", &consts)
            // only for one that flies off on its own; a kept projectile is steered by
            // whoever keeps it and must not die the first time it touches the floor
            .replace("{{contact_death_listeners}}", if kept_by_summoner { "" } else {
                "    self.addEventListener(EntityEvent.COLLIDE_FLOOR, onGroundHit, { persistent: true });\n    self.addEventListener(GameObjectEvent.HIT_DEALT,  onHit,       { persistent: true });\n\n"
            })
            .replace("{{contact_death_handlers}}", if kept_by_summoner { "" } else {
                "function onGroundHit(event) {\n    self.removeEventListener(EntityEvent.COLLIDE_FLOOR, onGroundHit);\n    self.removeEventListener(GameObjectEvent.HIT_DEALT,  onHit);\n    self.toState(PState.DESTROYING);\n}\n\nfunction onHit(event) {\n    self.removeEventListener(EntityEvent.COLLIDE_FLOOR, onGroundHit);\n    self.removeEventListener(GameObjectEvent.HIT_DEALT,  onHit);\n    self.toState(PState.DESTROYING);\n}"
            })
            .replace("{{death_watch}}", req("projectile.script.death_watch", &t.death_watch))
            .replace("{{death_watch_handler}}", req("projectile.script.death_watch_handler", &t.death_watch_handler))
            .replace("{{lstate_prep}}", &lstate_prep)
            .replace("{{physics_update}}", &phys.update)
            .replace("{{class_update}}", &class_update_call)
            // the class's own initializer first: in the source it runs when the object is made,
            // the timeline's first frame after (and that frame ends with a return that would
            // have left the call behind it unreached: rayman's plum never built its timers)
            .replace("{{source_init}}", &format!("{class_init_call}{source_init}"))
            .replace("{{source_methods}}", &source_methods)
            .replace("{{x_speed}}", &fmt_num(x_speed))
            .replace("{{y_speed}}", &fmt_num(y_speed))
            .replace("{{speed_note}}", &speed_note)
    }
}

/// The local state each of a projectile's labels becomes: `(label, LState name, animation)`.
/// The flight is IDLE; every other label is its own name, upper-cased, so the code that moves
/// the projectile between them (`forceAttack("pull")`) reads as the source wrote it. A label
/// that folds onto a name already taken gets a number, since the state object cannot carry a
/// key twice (and did once: three CUSTOM: keys, and the script never loaded).
pub(crate) fn projectile_lstates(labels: &[String]) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let mut taken: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for label in labels {
        let fm = entity_gen::ssf2_proj_label_to_fm_anim(label);
        let base: String = if label == "attack_idle" { "IDLE".to_string() } else {
            let ident: String = label.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
            let ident = ident.trim_matches('_').to_string();
            if ident.is_empty() { "STATE".to_string() }
            else if ident.chars().next().is_some_and(|c| c.is_ascii_digit()) { format!("S{ident}") }
            else { ident }
        };
        let mut name = base.clone();
        let mut n = 1;
        while !taken.insert(name.clone()) { n += 1; name = format!("{base}{n}"); }
        out.push((label.clone(), name, fm));
    }
    out
}


/// ProjectileAnimationStats.hx — endType for each projectile animation.
/// Three anchor animation names used across a projectile's stats files.
/// When SSF2's inner sprite carries its own FrameLabel tags, those literal
/// labels become the names; otherwise the converter falls back to FM's
/// template trio (`projectileSpawn` / `projectileIdle` / `projectileDestroy`).
/// This is the projectile-side parallel to character xframe-label
/// extraction the user explicitly asked for.
pub(crate) struct ProjectileAnims {
    pub(crate) spawn:   String,
    pub(crate) active:  String,
    pub(crate) destroy: String,
    /// All distinct animation names referenced by this projectile, in
    /// timeline order. Includes spawn/active/destroy plus any extra
    /// labels from `inner_labels` or multi-state `extra_states`.
    pub(crate) all: Vec<String>,
}

/// Does this frame's script end the object? The clip says so in the only way it can: by calling
/// the thing that ends it. Written after translation, so it is this engine's word for it.
// Both read a frame script LINE BY LINE: `code_only` is a per-line strip that stops at a comment
// and blanks every string, so a whole script fed to it lost everything after its first note, and
// a play read out of it had no label left to read. The clip's own word never arrived, and the
// roles fell to the position each label happened to sit in (birdo's egg flew in "die" and died
// in "chibi").
// And only the frame's OWN word: a handler it registers (`addEventListener(.., function() {
// self.kill(); })`) ends the object later, when something happens, not here, and the end of
// another object is not this one's (lucario's aura sphere was read as dying in the label that
// fires it, and flew on as a shot the engine counted as dying).
fn code_ends_the_object(code: &str) -> bool {
    let own: String = code.lines().map(crate::api_mappings::code_only).collect::<Vec<_>>().join("\n");
    // the frame with every function literal's body taken out
    let mut top = String::with_capacity(own.len());
    let b = own.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if own[i..].starts_with("function") && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) {
            if let Some(open) = own[i..].find('{').map(|o| i + o) {
                let mut depth = 0i32;
                let mut j = open;
                while j < b.len() {
                    if b[j] == b'{' { depth += 1 } else if b[j] == b'}' { depth -= 1; if depth == 0 { break } }
                    j += 1;
                }
                i = j + 1;
                continue;
            }
        }
        top.push(b[i] as char);
        i += 1;
    }
    top.contains("self.kill()")
}

/// The animation a frame's script asks for, if it asks for one.
fn first_played_animation(code: &str) -> Option<String> {
    code.lines().find_map(|l| {
        if l.trim_start().starts_with("//") { return None }
        let i = l.find(".playAnimation(\"")? + ".playAnimation(\"".len();
        let rest = &l[i..];
        rest.find('"').map(|e| rest[..e].to_string())
    })
}

/// A raw label as the identifier its animation is named by.
pub(crate) fn label_ident(s: &str) -> String {
    let cleaned: String = s.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    // Disallow leading digit.
    if cleaned.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        format!("_{}", cleaned)
    } else if cleaned.is_empty() {
        "projectile".to_string()
    } else if crate::compile::ident(&cleaned) != cleaned {
        // a word the script language keeps for itself (a label can be `continue`): an animation
        // by that name cannot be a key of its stats
        format!("{cleaned}_")
    } else {
        cleaned
    }
}

/// Where each of a projectile's labels goes when it runs out, as its timeline does: into the
/// next label, and off the last frame round to the first. A label whose last frame carries a
/// script that plays, stops or ends the object holds where that script leaves it, so it is not
/// in the table (its script already says). Cut into one animation per label, the port held every
/// label on its last frame; pikachu's bolt sat on the last frame of `continue` for good where
/// the source ran on into `stopped`.
pub(crate) fn projectile_label_flow(proj: &entity_gen::ProjectileInfo) -> String {
    const NONE: &str = "function __labelFlow() {}\n";
    let mut labels: Vec<&(u16, String)> = proj.inner_labels.iter().collect();
    labels.sort_by_key(|(f, _)| *f);
    if labels.len() < 2 { return NONE.to_string() }
    let mut from: Vec<String> = Vec::new();
    let mut to: Vec<String> = Vec::new();
    for (i, (f, l)) in labels.iter().enumerate() {
        let end = if i + 1 < labels.len() { labels[i + 1].0 - 1 } else { proj.inner_frame_count.max(*f) };
        let holds = proj.frame_scripts.get(&end).is_some_and(|c| {
            c.contains("playAnimation(") || c.contains(".kill()") || c.contains("stop()")
        });
        if holds { continue }
        let next = if i + 1 < labels.len() { &labels[i + 1].1 } else { &labels[0].1 };
        if next == l { continue }
        from.push(format!("\"{}\"", label_ident(l)));
        to.push(format!("\"{}\"", label_ident(next)));
    }
    if from.is_empty() { return NONE.to_string() }
    format!("// a label whose last frame says nothing runs on into the next, as the timeline does (the last round to the first)\n\
var _flowFrom = [{}];\n\
var _flowTo = [{}];\n\
function __labelFlow() {{\n\
\tif (!self.finalFramePlayed()) {{ return; }}\n\
\tvar a = self.getAnimation();\n\
\tvar __i = 0;\n\
\twhile (__i < _flowFrom.length) {{ if (_flowFrom[__i] == a) {{ self.playAnimation(_flowTo[__i]); return; }} __i = __i + 1; }}\n\
}}\n", from.join(", "), to.join(", "))
}

pub(crate) fn projectile_anim_names(proj: &entity_gen::ProjectileInfo) -> ProjectileAnims {
    let ident = label_ident;

    // Prefer real SSF2 labels when present, ordered by frame.
    let mut sorted: Vec<&(u16, String)> = proj.inner_labels.iter().collect();
    sorted.sort_by_key(|(f, _)| *f);
    let raw: Vec<String> = sorted.into_iter().map(|(_, l)| ident(l)).collect();

    // What the clip's OWN timeline says about its life. Which of a projectile's animations is
    // the one it dies to has no answer in the name -- the corpus calls it explode, chibland,
    // susLoop, whatever the author typed -- and guessing by position gets it wrong as often as
    // right. The clip states it: the frame that ends the object sits INSIDE the animation it
    // dies to, and the animation it asks for on its first frame is the one it lives in.
    let stated = |want_end: bool| -> Option<String> {
        let label_at = |frame: u16| -> Option<String> {
            proj.inner_labels.iter()
                .filter(|(f, _)| *f <= frame)
                .max_by_key(|(f, _)| *f)
                .map(|(_, l)| ident(l))
        };
        if want_end {
            proj.frame_scripts.iter()
                .find(|(_, code)| code_ends_the_object(code))
                .and_then(|(f, _)| label_at(*f))
        } else {
            // on its FIRST frame, before its first label runs: a play further in is a label
            // looping back on itself (a boomerang's `loop` replaying `loop`), not where it lives
            let first = proj.inner_labels.iter().map(|(f, _)| *f).min().unwrap_or(0);
            proj.frame_scripts.iter()
                .filter(|(f, _)| **f <= first)
                .find_map(|(_, code)| first_played_animation(code))
                .map(|l| ident(&l))
        }
    };

    let (spawn, active, destroy) = match raw.len() {
        0 => (
            "projectileSpawn".to_string(),
            "projectileIdle".to_string(),
            "projectileDestroy".to_string(),
        ),
        1 => {
            // Single label — re-use as the active animation; synthesize
            // spawn/destroy variants from it. Matches the Annie
            // convention of using one logical animation across states.
            let only = raw[0].clone();
            (only.clone(), only.clone(), only)
        }
        2 => (raw[0].clone(), raw[0].clone(), raw[1].clone()),
        // A projectile SPAWNS at frame 0, so the state it enters on birth is the label there --
        // raw[0], which is almost always the one it lives in (a `loop`, a `start`, an `idle`).
        // The old guess of raw[1] assumed raw[0] was a brief intro; it is not, and when the clip
        // does not state its active animation on a frame script (samus's bomb plays a character
        // animation, not one of its own), that guess put the projectile in its SECOND label -- the
        // bomb spawned already exploding, in `continue`, and rocketed samus off it the instant it
        // appeared. spawn where it spawns; `stated(false)` below still wins when the clip says so.
        _ => (raw[0].clone(), raw[0].clone(), raw[raw.len() - 1].clone()),
    };

    if std::env::var("PEPTIDE_DUMP_PROJ_ANIMS").is_ok() {
        eprintln!("[proj-anims {}] labels {:?}", proj.name, proj.inner_labels);
        for (f, code) in &proj.frame_scripts {
            eprintln!("[proj-anims {}] frame {f}: ends={} plays={:?}", proj.name, code_ends_the_object(code), first_played_animation(code));
        }
    }
    // The clip's own word wins over the position it happened to sit in. And a clip that never says
    // it dies in one of its labels does not: the source's end takes the object off at once, and any
    // animation it plays on the way was played by its code before it ended. Put into one of its
    // labels to die, it ran that label's code (lucario's aura sphere "died" in the label that fires
    // it and flew on as a shot); it dies in the stub, one still frame with nothing to run.
    let destroy = match stated(true).filter(|d| raw.contains(d)) {
        Some(d) => d,
        None if raw.len() >= 2 => "projectileDestroy".to_string(),
        None => destroy,
    };
    // it lives where its clip starts: the label at its first frame, whose code runs first (the
    // jolt sets its speed and starts its steering there) and plays on to the next label itself.
    // read off a play in a frame script instead, a projectile skipped that code and started in
    // the label the play named (pikachu's jolt rose in a straight line with nothing steering it)
    let active = if raw.contains(&active) && active != destroy { active }
        else { stated(false).filter(|a| raw.contains(a) && *a != destroy).unwrap_or(active) };

    // Build the union of all referenced names + any multi-state labels.
    let mut all: Vec<String> = vec![spawn.clone(), active.clone(), destroy.clone()];
    for s in &proj.extra_states {
        let n = entity_gen::ssf2_proj_label_to_fm_anim(&s.label);
        if !all.contains(&n) { all.push(n); }
    }
    for r in &raw {
        if !all.contains(r) { all.push(r.clone()); }
    }
    // De-dup while preserving first-seen order.
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    all.retain(|n| seen.insert(n.clone()));

    ProjectileAnims { spawn, active, destroy, all }
}

pub(crate) fn generate_projectile_animation_stats(proj: &entity_gen::ProjectileInfo) -> String {
    let t = &crate::mappings::script_templates().projectile.stats;
    let req = crate::mappings::require_template;
    let anims = projectile_anim_names(proj);
    let mut lines: Vec<String> = Vec::new();
    for (i, n) in anims.all.iter().enumerate() {
        // Mark the destroy animation with resetId:false to prevent
        // hit-id churn on state change (per template convention).
        if n == &anims.destroy && i > 0 {
            lines.push(req("projectile.stats.proj_anim_entry_destroy", &t.proj_anim_entry_destroy)
                .replace("{{name}}", n));
        } else {
            lines.push(req("projectile.stats.proj_anim_entry_normal", &t.proj_anim_entry_normal)
                .replace("{{name}}", n));
        }
    }
    req("projectile.stats.proj_anim_wrapper", &t.proj_anim_wrapper)
        .replace("{{body}}", &lines.join(",\n"))
}

/// ProjectileStats.hx — physics, geometry, and state → animation mapping.
/// Heuristic match between a discovered projectile sprite (e.g.
/// `"dee_nspec"`) and SSF2's per-attack `getProjectileStats()` entries
/// (keyed by attack name like `"b"`, `"b_up"`, `"final_smash"`, etc.).
///
/// Strategy:
/// 1. Exact name match (`dee_nspec` == `dee_nspec`) wins.
/// 2. Substring match on common projectile name fragments:
///       *nspec*       → `b`
///       *fspec/sspec* → `b_forward`
///       *uspec*       → `b_up`
///       *dspec*       → `b_down`
///       *fs*/*finalsmash* → `final_smash` / any key containing "final"
/// 3. Otherwise, pick the entry with the most populated data
///    (stats.len() + hitboxes.len()) — best-effort.
/// 4. None if the projectile_data map is empty.
///
/// Result is best-effort; callers should mark output with a TODO so the
/// modder verifies the mapping per call site.
pub(crate) fn best_match_projectile_data<'a>(
    proj_name: &str,
    projectile_data: &'a std::collections::BTreeMap<String, crate::abc_parser::ProjectileData>,
) -> Option<&'a crate::abc_parser::ProjectileData> {
    if projectile_data.is_empty() { return None; }
    // The declaration NAMES its own art: `linkage_id` is the clip the projectile wears, which is
    // the clip the discovery found. That is the source stating the match, so nothing here guesses
    // from move names -- which is what this used to do, with a table of `nspec`/`fspec`/`uspec`
    // spellings and a "most populated entry" fallback that handed a projectile someone else's
    // physics whenever the guess missed.
    if let Some(exact) = projectile_data.get(proj_name) { return Some(exact); }
    let want = proj_name.to_lowercase();
    projectile_data.values().find(|d| {
        d.names.get("linkage_id").is_some_and(|l| {
            let l = l.to_lowercase();
            // a sub-character's projectiles are discovered with their own prefix stripped
            l == want || l.ends_with(&format!("_{want}"))
        })
    })
}

pub(crate) fn generate_projectile_stats(
    _char_id: &str,
    entity_id: &str,
    proj: &entity_gen::ProjectileInfo,
    ssf2_match: Option<&crate::abc_parser::ProjectileData>,
) -> String {
    let t = &crate::mappings::script_templates().projectile.stats;
    let req = crate::mappings::require_template;
    let content_id = crate::projectile_gen::projectile_content_id(entity_id);
    let anims = projectile_anim_names(proj);

    // SSF2 physics field name → FM ProjectileStats field name. The set
    // is small and most names align; this table handles the few that
    // need renaming. Unknown SSF2 keys are emitted as `// TODO: …`.
    // SSF2 physics key → FM field map + scaffolding defaults (mappings/projectile_tables.jsonc).
    let ptables = crate::mappings::projectile_tables();

    // Build the physics lines from SSF2 data where available.
    let (mut physics_lines, mut todo_lines, source_note): (Vec<String>, Vec<String>, String) =
        match ssf2_match {
            Some(d) if !d.stats.is_empty() => {
                let mut taken: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
                let mut out: Vec<String> = Vec::new();
                for pm in &ptables.physics_map {
                    if taken.contains(pm.fm.as_str()) { continue; }
                    if let Some(v) = d.stats.get(pm.ssf2.as_str()) {
                        // in this engine's units, the same way a character's stats convert
                        let cfg = crate::mappings::character_stats();
                        let scaled = match pm.scale.as_str() {
                            "velocity" => *v * cfg.scaling.velocity_scale(),
                            "accel" => *v * cfg.scaling.accel_scale(),
                            // a size is stated in the entity's own units, as its boxes are
                            _ => *v,
                        };
                        out.push(req("projectile.stats.proj_stats_physics_line", &t.proj_stats_physics_line)
                            .replace("{{field}}", &pm.fm).replace("{{value}}", &fmt_num(scaled)));
                        taken.insert(pm.fm.as_str());
                    }
                }
                // and the yes/no things it says about itself
                for fm in &ptables.flag_map {
                    if let Some(v) = d.flags.get(fm.ssf2.as_str()) {
                        let v = if fm.invert { !v } else { *v };
                        out.push(req("projectile.stats.proj_stats_physics_line", &t.proj_stats_physics_line)
                            .replace("{{field}}", &fm.fm).replace("{{value}}", &v.to_string()));
                    }
                }
                // Surface any SSF2 stat keys we didn't map as TODOs -- except the ones the port
                // applies in the projectile's own script or at the spawn (handled_in_script), which
                // get an accurate note instead of a misleading "no mapping".
                let mut todos: Vec<String> = Vec::new();
                let mapped_ssf2: std::collections::BTreeSet<&str> = ptables.physics_map.iter()
                    .filter(|pm| taken.contains(pm.fm.as_str()))
                    .map(|pm| pm.ssf2.as_str())
                    .collect();
                let handled: std::collections::BTreeMap<&str, &str> = ptables.handled_in_script.iter()
                    .map(|h| (h.ssf2.as_str(), h.r#where.as_str())).collect();
                for (k, v) in &d.stats {
                    if mapped_ssf2.contains(k.as_str()) {
                        continue;
                    }
                    if let Some(note) = handled.get(k.as_str()) {
                        todos.push(format!("    // SSF2 {}: {} -> {}", k, fmt_num(*v), note));
                    } else {
                        todos.push(req("projectile.stats.proj_stats_todo_line", &t.proj_stats_todo_line)
                            .replace("{{key}}", k).replace("{{value}}", &fmt_num(*v)));
                    }
                }
                (out, todos, req("projectile.stats.proj_stats_source_matched", &t.proj_stats_source_matched).to_string())
            }
            _ => (vec![], vec![], req("projectile.stats.proj_stats_source_default", &t.proj_stats_source_default).to_string()),
        };

    // an item the fighter generates, pulled with no declaration of its own: it falls as the
    // source's engine drops a thrown item, and the death bounds end it as they end any throw
    let provided0: std::collections::BTreeSet<String> = physics_lines.iter()
        .filter_map(|l| l.split(':').next().map(|s| s.trim().to_string()))
        .collect();
    if crate::api_mappings::is_generated_item(entity_id) {
        let sc = &crate::mappings::character_stats().scaling;
        if !provided0.contains("gravity") && sc.item_gravity > 0.0 {
            physics_lines.push(req("projectile.stats.proj_stats_physics_line", &t.proj_stats_physics_line)
                .replace("{{field}}", "gravity").replace("{{value}}", &fmt_num(sc.item_gravity * sc.accel_scale())));
        }
        if !provided0.contains("deathBoundsDestroy") {
            physics_lines.push(req("projectile.stats.proj_stats_physics_line", &t.proj_stats_physics_line)
                .replace("{{field}}", "deathBoundsDestroy").replace("{{value}}", "true"));
        }
        // this engine's projectiles do not fall past a cap of nothing: without one the bomb hung at its apex
        if !provided0.contains("terminalVelocity") && sc.item_max_fall > 0.0 {
            physics_lines.push(req("projectile.stats.proj_stats_physics_line", &t.proj_stats_physics_line)
                .replace("{{field}}", "terminalVelocity").replace("{{value}}", &fmt_num(sc.item_max_fall * sc.velocity_scale())));
        }
    }
    // drawn and hit-tested at the fighters' size: the art and the boxes are in the source's own units
    // (the entity's), as a character's are, and the engine scales a character by its base scale; a
    // projectile with none was drawn and struck at 1/1.3 of the source's size against the fighters
    // (pikachu's thunder, whose box reaches 24 source px from its middle, reached 18.6 and missed a
    // fighter the source's strikes)
    let size = crate::mappings::character_stats().scaling.size_multiplier;
    for field in ["baseScaleX", "baseScaleY"] {
        if !physics_lines.iter().any(|l| l.trim_start().starts_with(field)) {
            physics_lines.push(req("projectile.stats.proj_stats_physics_line", &t.proj_stats_physics_line)
                .replace("{{field}}", field).replace("{{value}}", &fmt_num(size)));
        }
    }
    // Always emit the scaffolding defaults for fields SSF2 didn't supply.
    let provided: std::collections::BTreeSet<String> = physics_lines.iter()
        .filter_map(|l| l.split(':').next().map(|s| s.trim().to_string()))
        .collect();
    let item = crate::api_mappings::is_generated_item(entity_id);
    for fv in &ptables.scaffolding {
        if !provided.contains(&fv.field) && !item {
            physics_lines.push(req("projectile.stats.proj_stats_physics_line", &t.proj_stats_physics_line)
                .replace("{{field}}", &fv.field).replace("{{value}}", &fv.value));
        }
    }
    physics_lines.sort();
    todo_lines.sort();

    req("projectile.stats.proj_stats_body", &t.proj_stats_body)
        .replace("{{entity_id}}", entity_id)
        .replace("{{source_note}}", &source_note)
        .replace("{{content_id}}", &content_id)
        .replace("{{active}}", &anims.active)
        .replace("{{destroy}}", &anims.destroy)
        .replace("{{physics}}", &physics_lines.join("\n"))
        .replace("{{todos}}", &if todo_lines.is_empty() { String::new() } else { todo_lines.join("\n") + "\n" })
}

/// ProjectileHitboxStats.hx — pulls real hitbox values from SSF2's
/// `getProjectileStats()` data when a match is found. Uses
/// `mappings/character/hitbox_stats.jsonc` as the canonical SSF2→FM
/// field-name table (same source the character HitboxStats.hx generator
/// uses — single source of truth, not duplicated).
pub(crate) fn generate_projectile_hitbox_stats(
    _char_id: &str,
    entity_id: &str,
    proj: &entity_gen::ProjectileInfo,
    ssf2_match: Option<&crate::abc_parser::ProjectileData>,
) -> String {
    let t = &crate::mappings::script_templates().projectile.stats;
    let req = crate::mappings::require_template;
    let mapping = crate::mappings::character_hitbox_stats();
    let anims = projectile_anim_names(proj);

    if std::env::var("PEPTIDE_DUMP_PROJ_STATS").is_ok() {
        if let Some(d) = ssf2_match { eprintln!("[proj-stats] {entity_id}: {} box(es): {:?}", d.hitboxes.len(), d.hitboxes); }
    }
    // every box it declares, each under the index the entity draws it at (rasenshuriken's three
    // boxes push at 15, 0 and 30 degrees with 98, 58 and 58: with only the first written, every
    // grind hit flung the fighter out of it at the strongest)
    let one_box = |hb: &std::collections::BTreeMap<String, f64>| -> String {
            // Build hitbox0 from SSF2 keys via the hitbox_stats.jsonc canon.
            let mut parts: Vec<String> = Vec::new();
            // a projectile's hit obeys the same rule a fighter's does: fixed knockback does not
            // grow with the victim's damage
            let fixed = crate::mappings::HitboxStatsMapping::is_fixed_knockback(hb);
            for field in &mapping.fields {
                // Take the MAX of all listed SSF2 keys (mirrors how
                // hitbox_stats.jsonc is used elsewhere). Absent keys count as 0.
                let mut best: Option<f64> = None;
                for key in &field.ssf2_keys {
                    if let Some(v) = hb.get(key) {
                        best = Some(best.map_or(*v, |b: f64| b.max(*v)));
                    }
                }
                if let Some(mut v) = best {
                    if fixed && field.fm_field == "knockbackGrowth" { v = 0.0; }
                    // the freeze fields gain the hit's own frame, then 30→60fps
                    if field.isframe { v = (v + field.plus_frames) * 2.0; }
                    // the same measured factor the character path applies: a projectile launches
                    // its victim with the same engine, so it converts with the same numbers
                    v *= mapping.scale_for(&field.fm_field);
                    // -1 is Fraymakers' "no override"; SSF2 carries it as 255 or as anything
                    // non-positive, and doubling that sentinel produced the nonsense -2.4
                    // -1 is Fraymakers' "no override"; a source value that is not a positive
                    // count of frames (SSF2 states hitstun as a multiplier) means exactly that. a
                    // freeze stated as 0 is no freeze, as on a fighter's box: ness's pk fire states
                    // selfHitStun 0 and its pillar takes its next hit 3 frames late when it froze
                    let raw = best.unwrap_or(0.0);
                    let freeze = matches!(field.fm_field.as_str(), "hitstop" | "selfHitstop");
                    let v = if freeze && raw == 0.0 { 0.0 }
                        else if matches!(field.fm_field.as_str(), "hitstop" | "selfHitstop" | "hitstun")
                        && (raw <= 0.0 || raw == 255.0) { -1.0 } else { v };
                    parts.push(format!("{}: {}", field.fm_field, fmt_num(v)));
                }
            }
            // WHAT THE PROJECTILE SAYS ABOUT ITSELF. Its package declares whether it can be
            // reversed, absorbed and so on, and these were emitted as fixed values regardless:
            // every projectile came out reflectable, including mario's fireball, which declares
            // `canBeReversed: false`. Each of these is that declaration, under this engine's name.
            // unstated is the source's default, which this engine's own default does not match
            // and what its box states true or false that this engine has a field for, as a
            // fighter's box does (`flags` in hitbox_stats.jsonc: sora's blizzard freezes)
            // (a flag that names a field already written is that field's value: a box with no effect
            // launches nothing)
            for (field, value) in mapping.flag_fields(hb) {
                parts.retain(|p| !p.starts_with(&format!("{field}: ")));
                parts.push(format!("{field}: {value}"));
            }
            let reversible = projectile_flag(ssf2_match, "canBeReversed");
            parts.push(format!("reflectable: {reversible}"));
            // a box turns its launch for a fighter behind it as it states (zelda's din's fire sends
            // one behind its blast back the other way); the runtime judges behind from where the
            // shot was before its frame's move, as the source checks its hits first, so a fast
            // shot already past the fighter's middle still sends it the way it flies
            if !crate::haxe_gen::reversible(hb) { parts.push("reversibleAngle: false".to_string()); }
            parts.push(format!("absorbable: {}", projectile_flag(ssf2_match, "canBeAbsorbed")));
            parts.join(", ")
    };
    let (idle_body, source_note) = match ssf2_match.filter(|d| !d.hitboxes.is_empty()) {
        Some(d) => {
            let boxes: Vec<String> = d.hitboxes.iter().enumerate().map(|(i, hb)| {
                let idx = hb.get(crate::abc_parser::BOX_INDEX_KEY).map(|v| *v as usize).unwrap_or(i);
                req("projectile.stats.proj_hitbox_idle_matched", &t.proj_hitbox_idle_matched)
                    .replace("hitbox0", &format!("hitbox{idx}"))
                    .replace("{{parts}}", &one_box(hb))
            }).collect();
            (boxes.join(", "), req("projectile.stats.proj_hitbox_source_matched", &t.proj_hitbox_source_matched).to_string())
        }
        None => (
            req("projectile.stats.proj_hitbox_idle_default", &t.proj_hitbox_idle_default).to_string(),
            req("projectile.stats.proj_hitbox_source_default", &t.proj_hitbox_source_default).to_string(),
        ),
    };

    // the hitbox payload goes on EVERY animation: the source's attack belongs to the projectile,
    // and its timeline decides where the boxes are placed (megaman's buster carries its box in
    // its `loop`, which had an empty entry, and flew through a fighter dealing nothing). an
    // animation with no boxes never reads its entry
    let mut anim_blocks: Vec<String> = Vec::new();
    for name in &anims.all {
        {
            anim_blocks.push(req("projectile.stats.proj_hitbox_active_block", &t.proj_hitbox_active_block)
                .replace("{{name}}", name).replace("{{idle_body}}", &idle_body));
        }
    }
    let body = req("projectile.stats.proj_hitbox_body", &t.proj_hitbox_body)
        .replace("{{entity_id}}", entity_id)
        .replace("{{source_note}}", &source_note)
        .replace("{{blocks}}", &anim_blocks.join(",\n"));
    let body = strip_trailing_commas(&body);
    assert_balanced_stats(&body, &format!("ProjectileHitboxStats for {entity_id}"));
    body
}

/// Format a float like SSF2 would have written it: integers stay integers,
/// floats round to 3 decimal places without trailing zeros.
pub fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 { format!("{}", v as i64) }
    // a value stated just above nothing stays above it: samus's bomb deals 0.00000001 so that its
    // touch counts as a hit and sets it off, and rounded to 0 it touched nothing
    else if v.abs() < 0.0005 { format!("{v}") }
    else {
        let s = format!("{:.3}", v);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        s.to_string()
    }
}

#[cfg(test)]
mod projectile_behaviour_tests {
    #[test]
    fn a_box_angle_is_the_sources_as_written() {
        // the source sends a reversible angle against a victim in front as stated, backward ones
        // included (mario's up tilt at 96): both engines mirror only for a victim behind
        assert_eq!(super::fm_angle(96.0, true), 96.0);
        assert_eq!(super::fm_angle(180.0, false), 180.0);
    }

    use super::projectile_physics;
    use crate::abc_parser::ProjectileData;

    fn data(stats: &[(&str, f64)], flags: &[(&str, bool)], names: &[(&str, &str)]) -> ProjectileData {
        ProjectileData {
            stats: stats.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            flags: flags.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            names: names.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            hitboxes: Vec::new(),
        }
    }

    #[test]
    fn influence_reads_the_owner_stick_and_caps_each_axis() {
        // a y-influence projectile (din's fire shape): the held stick nudges it, capped at Movement.
        let d = data(&[("influenceYMovement", 12.0), ("influenceYFactor", 3.0)], &[], &[]);
        let p = projectile_physics(Some(&d), "e", 2.6, 0.0, "self.kill()");
        assert!(p.consts.contains("INF_YM"), "declares the y cap");
        assert!(p.update.contains("getHeldControls"), "reads the owner stick");
        assert!(p.update.contains("_c.DOWN") && p.update.contains("_c.UP"), "both y directions");
        assert!(p.update.contains("self.setY(self.getY() + _infy)"), "adds influence to position");
        // an x-only-zero projectile declares no influence at all
        let z = data(&[("influenceXMovement", 0.0), ("influenceYMovement", 0.0)], &[], &[]);
        let pz = projectile_physics(Some(&z), "e", 1.0, 0.0, "self.kill()");
        assert!(!pz.consts.contains("INF_"), "zero movement caps emit nothing");
    }

    #[test]
    fn control_direction_aims_at_the_launch_x_magnitude() {
        // pkthunder shape: controlDirection with a horizontal launch of 5.85.
        let d = data(&[("controlDirection", 90.0)], &[], &[]);
        let p = projectile_physics(Some(&d), "e", 5.85, 0.0, "self.kill()");
        assert!(p.consts.contains("CD_MAG = 5.85"), "drives at the launch x magnitude");
        assert!(p.update.contains("getHeldControls"), "reads the owner stick");
        assert!(p.update.contains("_cang = _tgt;"), "snaps when calcAngle is unset");
        assert!(p.update.contains("self.setXSpeed(Math.cos(_cang) * CD_MAG)"), "drives x by the aim");
        assert!(p.update.contains("self.setYSpeed(-Math.sin(_cang) * CD_MAG)"), "y negated for screen space");
        // calcAngle set -> eases instead of snapping
        let e = data(&[("controlDirection", 90.0), ("calcAngle", 1.0)], &[], &[]);
        let pe = projectile_physics(Some(&e), "e", 5.85, 0.0, "self.kill()");
        assert!(pe.update.contains("_cang = _cang + _d / 6"), "eases toward the aim when calcAngle is set");
    }

    #[test]
    fn trail_spawns_the_named_effect_at_its_interval() {
        // samus missile shape: a trail linkage id (a string) dropped every few frames.
        let d = data(&[("trailEffectInterval", 3.0)], &[], &[("trailEffect", "missileTrail")]);
        let p = projectile_physics(Some(&d), "e", 5.0, 0.0, "self.kill()");
        assert!(p.consts.contains("TRAIL_EVERY = 3"), "declares the interval");
        assert!(p.consts.contains("_trailName = \"missileTrail\""), "names the effect, which its frames may retune");
        assert!(p.update.contains("getContent(_trailName)"), "spawns the named effect");
        assert!(p.update.contains("if (_tc != null)"), "guards a linkage that does not resolve");
        assert!(!p.update.contains("setRotation"), "no aim when trailEffectRotate is unset");
        // rotate flag aims the trail along the heading
        let r = data(&[("trailEffectInterval", 1.0)], &[("trailEffectRotate", true)], &[("trailEffect", "t")]);
        let pr = projectile_physics(Some(&r), "e", 5.0, 0.0, "self.kill()");
        assert!(pr.update.contains("setRotation"), "aims the trail when trailEffectRotate is set");
        // no trail string -> no trail block, even with an interval
        let n = data(&[("trailEffectInterval", 3.0)], &[], &[]);
        let pn = projectile_physics(Some(&n), "e", 5.0, 0.0, "self.kill()");
        assert!(!pn.consts.contains("TRAIL_EVERY"), "an interval with no effect name emits nothing");
    }
}

#[cfg(test)]
mod hitbox_split_tests {
    use super::{base_attack_name, strip_trailing_commas, assert_balanced_stats};

    #[test]
    fn split_sub_anims_map_to_their_base_attack() {
        // jabN (N>=2) inherits jab1; strong _in/_charge inherit _attack.
        assert_eq!(base_attack_name("jab2").as_deref(), Some("jab1"));
        assert_eq!(base_attack_name("jab3").as_deref(), Some("jab1"));
        assert_eq!(base_attack_name("strong_forward_in").as_deref(), Some("strong_forward_attack"));
        assert_eq!(base_attack_name("strong_up_charge").as_deref(), Some("strong_up_attack"));
        assert_eq!(base_attack_name("strong_down_in").as_deref(), Some("strong_down_attack"));
        // Non-split / base names have no fallback.
        assert_eq!(base_attack_name("jab1"), None);
        assert_eq!(base_attack_name("tilt_forward"), None);
        assert_eq!(base_attack_name("dash_attack"), None);
        assert_eq!(base_attack_name("strong_forward_attack"), None);
    }

    #[test]
    fn strip_trailing_commas_removes_only_structural_commas() {
        let got = strip_trailing_commas("{ jab1: { hitbox0: { d: 1, a: 2 }, }, }");
        // the trailing commas before } are gone; the comma between fields stays
        assert_eq!(got, "{ jab1: { hitbox0: { d: 1, a: 2 } } }");
    }

    #[test]
    fn assert_balanced_stats_accepts_valid_and_ignores_comments() {
        // a half-commented block (the real bug) is still balanced once the open is in a comment
        // AND the body is commented too — the fixed form — so it passes.
        assert_balanced_stats("{\n\tjab1: { hitbox0: {} }\n\t// SSF2: x: {\n\t// SSF2: }\n}", "t");
    }

    #[test]
    #[should_panic(expected = "broken stats object")]
    fn assert_balanced_stats_rejects_the_half_commented_block_bug() {
        // the original bug: the open `x: {` is commented out but the body + close are ACTIVE,
        // leaving an extra `}` → unbalanced → must be caught.
        assert_balanced_stats("{\n\t// SSF2: x: {\n\t\thitbox0: {}\n\t}\n}", "t");
    }
}


/// The library id a character's content is filed under.
fn char_id_of(char_name: &str) -> String { char_name.to_lowercase() }

/// The effects a package's code asks for by a name it writes whole (`attachEffect("x")`), in any
/// of its classes.
fn attach_effect_names(model: &crate::model::SourceModel) -> std::collections::BTreeSet<String> {
    use crate::model::ast::Expr;
    let mut out = std::collections::BTreeSet::new();
    for c in model.classes.values() {
        for m in &c.methods {
            crate::model::ast::walk_exprs(&m.function.body, &mut |e| {
                let Expr::Call(callee, args) = e else { return };
                let named = match callee.as_ref() {
                    Expr::Member(_, n) | Expr::Name(n) => n == "attachEffect" || n == "attachEffectOverlay",
                    _ => false,
                };
                if let (true, Some(Expr::Str(s))) = (named, args.first()) { out.insert(s.clone()); }
            });
        }
    }
    out
}

/// A package an effect was borrowed from, kept open so the effect can be drawn from it.
///
/// The entity is built from the images of the file the effect LIVES in, not this character's, so
/// the movie and its extraction are held together and the build happens against both.
pub struct BorrowedFrom {
    swf: Vec<u8>,
    images: crate::image_extractor::ImageExtractionResult,
}

impl BorrowedFrom {
    /// Build one borrowed effect's entity against the package it came from.
    pub fn build_entity(&self, char_id: &str, effect: &crate::image_extractor::DiscoveredEffect) -> String {
        let Ok(buf) = swf::decompress_swf(&self.swf[..]) else { return String::new() };
        let Ok(parsed) = swf::parse_swf(&buf) else { return String::new() };
        entity_gen::generate_effect_entity(char_id, effect, &self.images, &parsed)
    }
}

/// Bring across the effects this character NAMES but does not carry.
///
/// A shared effect is registered by whichever file defines it, so a character that only
/// references one has nothing of its own to convert. Each borrowed package's images go in a
/// folder of their own: two packages can both hold an unnamed bitmap under the same number, and
/// only the path keeps them apart (the .meta GUIDs are symbol-derived, so a genuinely shared
/// symbol still resolves to one asset).
fn import_shared_effects(
    data: &CharacterData,
    have: &[crate::image_extractor::DiscoveredEffect],
    output_dir: &Path,
    char_name: &str,
    char_pascal: &str,
    input_path: &Path,
) -> Vec<(crate::image_extractor::DiscoveredEffect, String)> {
    let carried: std::collections::BTreeSet<&str> = have.iter().map(|e| e.name.as_str()).collect();
    let global = &crate::mappings::api_commands().global_vfx_map;
    let re = crate::re!(r#"attachEffect(?:Overlay)?\(\s*"([^"]+)""#);
    // what the fighter's own scripts name, and what every class in its package names (its
    // projectiles' code too: link's bomb asks for the explosion another fighter's file defines)
    let mut named: std::collections::BTreeSet<String> = data.scripts.iter()
        .flat_map(|sc| re.captures_iter(&sc.code).map(|c| c[1].to_string())).collect();
    if let Some(model) = std::fs::read(input_path).ok().and_then(|b| crate::model::SourceModel::read(&b).ok()) {
        named.extend(attach_effect_names(&model));
    }
    let mut want: std::collections::BTreeSet<String> = named.into_iter()
        .filter(|n| !carried.contains(n.as_str()) && !global.contains_key(n))
        .collect();
    if want.is_empty() { return Vec::new() }
    let asked = want.len();
    let char_id = char_name.to_lowercase();
    let sprites = output_dir.join(&char_id).join("library/sprites");
    let mut out = Vec::new();
    for (pkg_id, swf_bytes) in crate::shared_packages::all_data_packages() {
        if want.is_empty() { break }
        let Ok(buf) = swf::decompress_swf(&swf_bytes[..]) else { continue };
        let Ok(parsed) = swf::parse_swf(&buf) else { continue };
        let Ok((_, found, _)) = crate::image_extractor::discover_projectiles_and_head_from_swf(
            &parsed, char_name, None) else { continue };
        let mine: Vec<_> = found.into_iter().filter(|e| want.contains(&e.name)).collect();
        if mine.is_empty() { continue }
        // Extraction takes a whole package at once and lays it out as a character would be, so
        // it runs somewhere of its own and only the pictures the effect DRAWS are copied in.
        // Borrowing one explosion off mario's cape otherwise brought 2793 files with it.
        // one per conversion: two fighters converted at once borrow from the same package, and a
        // shared one was cleared under the other's feet (its pictures went missing from the pack)
        let scratch = std::env::temp_dir().join(format!("peptide-borrow-{}-{}", pkg_id.trim_end_matches(".ssf"), std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let Ok(images) = crate::image_extractor::extract_images(
            &swf_bytes, &scratch, char_name, &Default::default()) else { continue };
        let dir = sprites.join("Shared").join(pkg_id.trim_end_matches(".ssf"));
        // These pictures came from another package, which numbers its ids on its
        // own, so `bitmap_294` here is not the host's `bitmap_294`. Keep them in
        // their own namespace for the rest of this borrow: the guids below and
        // the ones the built entities refer to are both taken under it.
        let _img_ns = crate::uuid_gen::ImageNsGuard::install(pkg_id.trim_end_matches(".ssf"));
        for e in &mine { want.remove(&e.name); }
        // Build here, while the package is open, and keep only the pictures the effect DRAWS.
        // Extraction takes a whole package at once, and a package is a whole character: borrowing
        // one explosion off mario's cape brought 2794 files and 27MB of somebody else's sprites
        // with it. What the entity refers to is what stays.
        let guids = entity_gen::get_image_meta_guids(&images);
        let mut built = Vec::new();
        let mut used: std::collections::BTreeSet<String> = Default::default();
        {
            let from = BorrowedFrom { swf: swf_bytes, images };
            entity_gen::set_borrowing(true);
            for e in &mine {
                let json = from.build_entity(&char_id_of(char_name), e);
                used.extend(guids.values().filter(|g| json.contains(g.as_str())).cloned());
                built.push(json);
            }
            entity_gen::set_borrowing(false);
        }
        let _ = std::fs::create_dir_all(&dir);
        for (png, guid) in &guids {
            if !used.contains(guid) { continue }
            let Some(base) = std::path::Path::new(png).file_name() else { continue };
            let to = dir.join(base);
            if std::fs::copy(scratch.join(png), &to).is_ok() {
                let _ = std::fs::write(to.with_extension("png.meta"), entity_gen::generate_meta(guid));
            }
        }
        let _ = std::fs::remove_dir_all(&scratch);
        log::info!("{} effects borrowed from the '{pkg_id}' package ({} of its {} pictures kept)",
            mine.len(), used.len(), guids.len());
        for (e, json) in mine.into_iter().zip(built) { out.push((e, json)); }
    }
    log::info!("{} of {asked} effects {char_pascal} names came from another package", out.len());
    if !want.is_empty() {
        log::warn!("effects {char_name} names that no package defines: {}",
            want.iter().cloned().collect::<Vec<_>>().join(", "));
    }
    out
}

#[cfg(test)]
mod ask_other_tests {
    #[test]
    fn a_call_nobody_answers_goes_through_exports_and_keeps_its_arguments() {
        let line = "\t\tcharacter.applyPaletteToEffect(match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent(\"e\"), animation: \"active\" }), self));";
        let out = super::ask_other_objects_through_exports(line, &[]);
        assert!(out.contains("character.exports.applyPaletteToEffect"), "{out}");
        assert!(out.contains("__nothing([match.createVfx("), "{out}");
    }
}

/// The effects a fighter's package ships, for the ones its code names as it runs, and the walk of
/// them: a name that is not among them spawns nothing, as before, without drawing the wrong thing.
/// A projectile carries it too when its own code names one; its effects are its fighter's.
pub(crate) fn attach_effect_helper() -> String {
    let table = crate::api_mappings::local_effect_table();
    let names: Vec<String> = table.iter().map(|(n, _)| format!("\"{n}\"")).collect();
    let anims: Vec<String> = table.iter().map(|(_, a)| format!("\"{a}\"")).collect();
    format!(
        "// the effects this character ships, for the ones it names as it runs\n\
var _fxNames = [{}];\n\
var _fxAnims = [{}];\n\
\n\
function _attachEffect(name, bag) {{\n\
\tvar __i = 0;\n\
\twhile (__i < _fxNames.length) {{\n\
\t\tif (_fxNames[__i] == name) {{\n\
\t\t\tvar vx = 0;\n\
\t\t\tvar vy = 0;\n\
\t\t\tvar sx = 1;\n\
\t\t\tvar sy = 1;\n\
\t\t\tif (bag != null) {{\n\
\t\t\t\tif (bag.x != null) {{ vx = bag.x; }}\n\
\t\t\t\tif (bag.y != null) {{ vy = bag.y; }}\n\
\t\t\t\tif (bag.scaleX != null) {{ sx = bag.scaleX; }}\n\
\t\t\t\tif (bag.scaleY != null) {{ sy = bag.scaleY; }}\n\
\t\t\t}}\n\
\t\t\t// placed as a named effect is: its offset in the source's pixels, facing its owner\n\
\t\t\treturn match.createVfx(new VfxStats({{ spriteContent: self.getResource().getContent(_fxNames[__i]), animation: _fxAnims[__i], x: __sp(self.flipX(vx)), y: __sp(vy), scaleX: sx, scaleY: sy, relativeWith: true, resizeWith: true, flipWith: true }}), self);\n\
\t\t}}\n\
\t\t__i = __i + 1;\n\
\t}}\n\
\treturn null;\n\
}}\n",
        names.join(", "), anims.join(", "))
}
