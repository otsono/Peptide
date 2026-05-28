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

pub fn generate(output_dir: &Path, char_name: &str, data: &CharacterData, sprite_boxes: &std::collections::BTreeMap<String, crate::sprite_parser::AnimationBoxData>, img_result: &crate::image_extractor::ImageExtractionResult, costumes_json: Option<&Path>, sounds: &[crate::sound_extractor::SoundEntry], projectiles: &[crate::image_extractor::DiscoveredProjectile], effects: &[crate::image_extractor::DiscoveredEffect], head_sprite: Option<&crate::image_extractor::DiscoveredHead>, swf_data: &[u8]) -> Result<()> {
    let char_id = char_name.to_lowercase().replace(" ", "");
    let char_dir = output_dir.join(&char_id);
    let scripts_dir = char_dir.join("library/scripts/Character");
    fs::create_dir_all(&scripts_dir)?;

    // Effect → primary-animation map for the context-aware
    // self.attachEffect("name") → match.createVfx(...) rewrite. The guard
    // installs the map for the duration of this function and clears it on
    // drop, so every translate_ssf2_to_fm call made while generating this
    // character (Character.entity frame scripts, Script.hx, per-attack
    // scripts) sees the right effect→animation pairing.
    let effect_anim_map: std::collections::BTreeMap<String, String> = effects.iter()
        .filter_map(|e| {
            entity_gen::effect_animation_names(e)
                .into_iter()
                .next()
                .map(|first| (e.name.clone(), first))
        })
        .collect();
    let _effect_anim_guard = crate::api_mappings::EffectAnimGuard::install(effect_anim_map);

    log::info!("Generating Fraymakers character package in {}", char_dir.display());

    // How many of jab1/jab2/jab3/jab4 actually have image content. Drives
    // the jab-chain emission in Script.hx and the keep-empty allowlist in
    // entity_gen — a single-jab character gets no chain boilerplate, and
    // jab2/jab3 are dropped as empty along with the others.
    let populated_jabs = count_populated_jabs(img_result);

    fs::write(scripts_dir.join("HitboxStats.hx"),   generate_hitbox_stats(data, &char_id))?;
    fs::write(scripts_dir.join("CharacterStats.hx"), generate_character_stats(data, &char_id))?;
    let splits = crate::anim_splitter::split_animations(&data.animations, sprite_boxes);
    fs::write(scripts_dir.join("AnimationStats.hx"), generate_animation_stats(data, &splits))?;
    fs::write(scripts_dir.join("Script.hx"),         generate_script(data, &char_id, populated_jabs))?;

    // .meta sidecar files for character scripts
    fs::write(scripts_dir.join("HitboxStats.hx.meta"),    script_meta(&format!("{}HitboxStats", char_id),    &det_uuid(&format!("{}::HitboxStats::meta", char_id)),    ScriptMetaKind::CharacterHitboxStats))?;
    fs::write(scripts_dir.join("CharacterStats.hx.meta"), script_meta(&format!("{}CharacterStats", char_id), &det_uuid(&format!("{}::CharacterStats::meta", char_id)), ScriptMetaKind::CharacterStats))?;
    fs::write(scripts_dir.join("AnimationStats.hx.meta"), script_meta(&format!("{}AnimationStats", char_id), &det_uuid(&format!("{}::AnimationStats::meta", char_id)), ScriptMetaKind::CharacterAnimationStats))?;
    fs::write(scripts_dir.join("Script.hx.meta"),         script_meta(&format!("{}Script", char_id),         &det_uuid(&format!("{}::Script::meta", char_id)),         ScriptMetaKind::CharacterScript))?;

    // .fraytools project file
    fs::write(char_dir.join(format!("{}.fraytools", char_name)), fraytools_project::generate_fraytools_project(char_name))?;

    // manifest.json (based on character-template)
    let proj_names: Vec<String> = projectiles.iter().map(|p| p.name.clone()).collect();
    fs::write(char_dir.join("library/manifest.json"), generate_manifest(&char_id, char_name, &proj_names))?;
    fs::write(char_dir.join("library/manifest.json.meta"), generate_manifest_meta(&det_uuid(&format!("{}::manifest::meta", char_id))))?;

    // Character.entity
    let entities_dir = char_dir.join("library/entities");
    fs::create_dir_all(&entities_dir)?;
    fs::write(entities_dir.join("Character.entity"), entity_gen::generate_entity(data, &char_id, sprite_boxes, img_result, populated_jabs))?;

    // Generate .meta sidecar files for each sprite PNG
    let meta_guids = entity_gen::get_image_meta_guids(&char_id, img_result);
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
    match palette_gen::generate_palettes_and_remap(&char_id, char_name, &sprites_dir, costumes_json) {
        Ok(pal) => {
            // costumes.palettes + .meta
            fs::write(char_dir.join("library/costumes.palettes"), &pal.palettes_json)?;
            fs::write(char_dir.join("library/costumes.palettes.meta"), &pal.palettes_meta_json)?;
            // palette_preview.png + .meta (reference image for the R/G map shader)
            fs::write(sprites_dir.join("palette_preview.png"), &pal.preview_png)?;
            fs::write(sprites_dir.join("palette_preview.png.meta"), &pal.preview_meta_json)?;
            // Write the entity with the paletteMap filled in
            let entity_json = entity_gen::generate_entity_with_palette(
                data, &char_id, sprite_boxes, img_result, populated_jabs,
                &pal.collection_guid, &pal.base_map_id,
            );
            fs::write(entities_dir.join("Character.entity"), entity_json)?;
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
                let head_meta_guid = crate::uuid_gen::det_uuid(&format!("{}::meta_{}", char_id, img_sym));
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
                fs::write(entities_dir.join("menu.entity"), menu_json)?;
                log::info!("Generated menu.entity using {} ({}x{})", img_sym, head_img.width, head_img.height);
            } else {
                log::warn!("Head image '{}' not found in extracted images, skipping menu.entity", img_sym);
            }
        } else {
            log::warn!("Head sprite '{}' has no image placement, skipping menu.entity", head.name);
        }
    } else {
        log::warn!("No head sprite found, skipping menu.entity");
    }

    // ── projectile.entity files ───────────────────────────────────────────────────
    for proj in projectiles {
        // Extract image frames from the inner sprite using effect-sprite flattening
        let (image_frames, image_guids) = if let Some(inner_id) = proj.inner_sprite_id {
            match crate::image_extractor::extract_projectile_frame_images(
                swf_data, &char_id, inner_id, img_result
            ) {
                Ok(pfi) => {
                    log::debug!("Projectile '{}': {} image frames", proj.name, pfi.frames.len());
                    (pfi.frames, pfi.image_guids)
                }
                Err(e) => {
                    log::warn!("Failed to extract images for projectile '{}': {}", proj.name, e);
                    (vec![], std::collections::BTreeMap::new())
                }
            }
        } else {
            (vec![], std::collections::BTreeMap::new())
        };

        // Extract collision boxes from the inner sprite
        let boxes = if let Some(inner_id) = proj.inner_sprite_id {
            match crate::sprite_parser::extract_boxes_for_sprite_id(swf_data, inner_id) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("Failed to extract boxes for projectile '{}': {}", proj.name, e);
                    None
                }
            }
        } else { None };

        // Extract image+box data for each extra state (multi-state projectiles like link_bomb)
        let mut extra_states: Vec<entity_gen::ProjectileStateData> = Vec::new();
        for state in &proj.states {
            if state.label == "attack_idle" { continue; } // already extracted above
            let (sf, sg) = match crate::image_extractor::extract_projectile_frame_images(
                swf_data, &char_id, state.inner_sprite_id, img_result
            ) {
                Ok(pfi) => (pfi.frames, pfi.image_guids),
                Err(e) => {
                    log::warn!("State '{}' image extraction failed: {}", state.label, e);
                    (vec![], std::collections::BTreeMap::new())
                }
            };
            let sb = match crate::sprite_parser::extract_boxes_for_sprite_id(swf_data, state.inner_sprite_id) {
                Ok(b) => b,
                Err(_) => None,
            };
            extra_states.push(entity_gen::ProjectileStateData {
                label: state.label.clone(),
                image_frames: sf,
                image_guids: sg,
                boxes: sb,
                frame_count: state.inner_frame_count,
            });
        }

        let proj_info = entity_gen::ProjectileInfo {
            name: proj.name.clone(),
            inner_sprite_name: proj.inner_sprite_name.clone(),
            inner_frame_count: proj.inner_frame_count,
            boxes,
            image_frames,
            image_guids,
            extra_states,
            inner_labels: proj.inner_labels.clone(),
        };

        let filename = format!("{}.entity", sanitize_entity_name(&proj.name));
        let mut proj_json = entity_gen::generate_projectile_entity(&char_id, &proj_info);
        // Fill in paletteMap if available
        if let (Some(ref cg), Some(ref pm)) = (&palette_collection_guid, &palette_base_map_id) {
            let mut proj_val: serde_json::Value = serde_json::from_str(&proj_json).unwrap_or(serde_json::json!({}));
            proj_val["paletteMap"] = serde_json::json!({
                "paletteCollection": cg,
                "paletteMap": pm
            });
            proj_json = serde_json::to_string_pretty(&proj_val).unwrap_or(proj_json);
        }
        fs::write(entities_dir.join(&filename), proj_json)?;
        log::info!("Generated projectile entity: {} ({} frames)", filename, proj.inner_frame_count);

        // ── projectile script files ──────────────────────────────────────────
        // Layout matches the convention observed in aJewelofRarity/AnnieCharacter
        // (real FM character mod): a SINGLE `library/scripts/Projectile/`
        // directory holds files for every projectile, with each file
        // prefixed by the projectile name in PascalCase
        // (`DeeNspecScript.hx`, `DeeNspecStats.hx`, …). Scales cleanly to
        // multi-projectile characters; matches a real-FM-mod precedent
        // (our prior `Projectile_<name>/` layout matched none of the 6
        // surveyed repos).
        let entity_id = proj.name.replace('_', "");
        let pascal = snake_to_pascal(&proj.name);
        let proj_scripts_dir = char_dir.join("library/scripts/Projectile");
        fs::create_dir_all(&proj_scripts_dir)?;

        fs::write(
            proj_scripts_dir.join(format!("{}Script.hx", pascal)),
            generate_projectile_script(&char_id, &entity_id, &proj_info.extra_states),
        )?;
        fs::write(
            proj_scripts_dir.join(format!("{}Script.hx.meta", pascal)),
            script_meta(
                &format!("{}ProjectileScript", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileScript::meta", char_id, proj.name)),
                ScriptMetaKind::ProjectileScript,
            ),
        )?;
        fs::write(
            proj_scripts_dir.join(format!("{}AnimationStats.hx", pascal)),
            generate_projectile_animation_stats(&proj_info),
        )?;
        fs::write(
            proj_scripts_dir.join(format!("{}AnimationStats.hx.meta", pascal)),
            script_meta(
                &format!("{}ProjectileAnimationStats", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileAnimationStats::meta", char_id, proj.name)),
                ScriptMetaKind::ProjectileAnimationStats,
            ),
        )?;
        let proj_ssf2_match = best_match_projectile_data(&proj.name, &data.projectile_data);
        fs::write(
            proj_scripts_dir.join(format!("{}Stats.hx", pascal)),
            generate_projectile_stats(&char_id, &entity_id, &proj_info, proj_ssf2_match),
        )?;
        fs::write(
            proj_scripts_dir.join(format!("{}Stats.hx.meta", pascal)),
            script_meta(
                &format!("{}ProjectileStats", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileStats::meta", char_id, proj.name)),
                ScriptMetaKind::ProjectileStats,
            ),
        )?;
        fs::write(
            proj_scripts_dir.join(format!("{}HitboxStats.hx", pascal)),
            generate_projectile_hitbox_stats(&char_id, &entity_id, &proj_info, proj_ssf2_match),
        )?;
        fs::write(
            proj_scripts_dir.join(format!("{}HitboxStats.hx.meta", pascal)),
            script_meta(
                &format!("{}ProjectileHitboxStats", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileHitboxStats::meta", char_id, proj.name)),
                ScriptMetaKind::ProjectileHitboxStats,
            ),
        )?;
        log::info!("Generated projectile scripts for {} → {}*.hx", proj.name, pascal);
    }

    // ── effect .entity files ─────────────────────────────────────────────
    // Per-effect entities (no scripts, no stats, no manifest entries).
    // The character's Script.hx spawns them via match.createVfx(...).
    for effect in effects {
        let filename = format!("{}.entity", effect.name);
        let entity_json = entity_gen::generate_effect_entity(
            &char_id, effect, img_result, swf_data,
        );
        fs::write(entities_dir.join(&filename), entity_json)?;
        let anim_names = entity_gen::effect_animation_names(effect);
        log::info!(
            "Generated effect entity: {} ({} frames, animations: [{}])",
            filename,
            effect.frame_count,
            anim_names.join(", "),
        );
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
    });
    fs::write(char_dir.join("conversion_stats.json"), serde_json::to_string_pretty(&stats_json)?)?;

    // ── Sound content entries ─────────────────────────────────────────────────────────────────
    if !sounds.is_empty() {
        generate_sound_entries(&char_dir, char_name, sounds)?;
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
        format!("{:.2}", v).trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

// ─── HitboxStats.hx ─────────────────────────────────────────────────────────

fn generate_hitbox_stats(data: &CharacterData, char_id: &str) -> String {
    let attack_lookup: std::collections::BTreeMap<_, _> = data.attacks.iter()
        .map(|a| (a.name.as_str(), a))
        .collect();

    let mut out = format!(
        "// Hitbox stats for {} — converted from SSF2\n\
        // SSF2 field mapping:\n\
        //   damage → damage\n\
        //   direction → angle\n\
        //   power/weightKB → baseKnockback\n\
        //   kbConstant → knockbackGrowth\n\
        //   hitStun → hitstop  (frames of freeze on hit)\n\
        //   selfHitStun → selfHitstop\n\
        //   hitLag → hitstun   (frames victim can't act)\n\
        // limb values inferred from move type — review before use.\n\
        {{\n",
        data.name
    );

    let sections: &[(&str, &[&str])] = &[
        ("LIGHT ATTACKS",  &["jab1","jab2","jab3","dash_attack","tilt_forward","tilt_up","tilt_down"]),
        ("STRONG ATTACKS", &["strong_forward_attack","strong_up_attack","strong_down_attack"]),
        ("AERIAL ATTACKS", &["aerial_neutral","aerial_forward","aerial_back","aerial_up","aerial_down"]),
        ("SPECIAL ATTACKS",&["special_neutral","special_neutral_air","special_side","special_side_air","special_up","special_up_air","special_down","special_down_air"]),
        ("THROWS",         &["throw_up","throw_down","throw_forward","throw_back"]),
        ("MISC ATTACKS",   &["ledge_attack","crash_attack","emote"]),
    ];

    let standard: std::collections::HashSet<&str> = sections.iter()
        .flat_map(|(_, moves)| moves.iter().copied()).collect();

    for (section, moves) in sections {
        out.push_str(&format!("\n\t//{}\n", section));
        for &move_name in *moves {
            if let Some(attack) = attack_lookup.get(move_name) {
                out.push_str(&format_attack(move_name, &attack.hitboxes, false));
            } else if move_name == "emote" {
                out.push_str("\temote: {\n\t\thitbox0: {}\n\t},\n");
            } else {
                out.push_str(&format_attack_todo(move_name));
            }
        }
    }

    // Extra attacks from SSF2 that don't map to standard moves
    let extras: Vec<_> = data.attacks.iter()
        .filter(|a| !standard.contains(a.name.as_str())).collect();
    if !extras.is_empty() {
        out.push_str("\n\t//SSF2-SPECIFIC (no direct Fraymakers equivalent — map or remove)\n");
        for attack in extras {
            out.push_str(&format_attack(&attack.name, &attack.hitboxes, true));
        }
    }

    out.push_str("}\n");
    out
}

fn guess_limb(move_name: &str) -> &'static str {
    let m = move_name;
    if m.contains("throw")    { return "AttackLimb.BODY"; }
    if m.contains("down")     { return "AttackLimb.FOOT"; }
    if m.contains("aerial")   { return "AttackLimb.FOOT"; }
    if m.contains("tilt_up") || m.contains("strong_up") { return "AttackLimb.FIST"; }
    if m.contains("neutral")  { return "AttackLimb.FOOT"; }
    if m.contains("jab")      { return "AttackLimb.FIST"; }
    if m.contains("tilt") || m.contains("forward") { return "AttackLimb.FIST"; }
    if m.contains("special")  { return "AttackLimb.FIST"; }
    if m.contains("ledge") || m.contains("crash") { return "AttackLimb.FOOT"; }
    "AttackLimb.FIST"
}

fn format_attack(name: &str, hitboxes: &[Hitbox], is_extra: bool) -> String {
    let limb = guess_limb(name);
    let prefix = if is_extra { "\t// SSF2: " } else { "\t" };
    let mut out = format!("{}{}: {{\n", prefix, name);
    // Frame-count fields are doubled for the 30fps -> 60fps timing change.
    // Which fields are frame counts is driven by the `isframe` flags in
    // mappings/character/hitbox_stats.json (see crate::mappings), the same
    // mechanism commands.json uses — so all frame-doubling is one path.
    let hb_cfg = crate::mappings::character_hitbox_stats();
    let scale = |fm_field: &str, v: i32| if hb_cfg.is_frame(fm_field) { v * 2 } else { v };
    for (i, hb) in hitboxes.iter().enumerate() {
        // The -1 "no override" sentinel (and SSF2's 255, which maps to it) is
        // emit formatting and stays in code; only the doubling is data-driven.
        let hitstun = if hb.hitstun == 255 || hb.hitstun == -1 { -1 } else { scale("hitstun", hb.hitstun) };
        let hitstop = if hb.hitstop <= 0 { -1 } else { scale("hitstop", hb.hitstop) };
        let self_hitstop = if hb.self_hitstop <= 0 { -1 } else { scale("selfHitstop", hb.self_hitstop) };

        out.push_str(&format!(
            "\t\thitbox{}: {{ damage: {}, angle: {}, baseKnockback: {}, knockbackGrowth: {}, hitstop: {}, selfHitstop: {}",
            i,
            hb.damage as i32,
            hb.angle as i32,
            hb.base_knockback as i32,
            hb.knockback_growth as i32,
            hitstop,
            self_hitstop,
        ));
        if hitstun != -1 {
            out.push_str(&format!(", hitstun: {}", hitstun));
        }
        out.push_str(&format!(", limb: {} }},\n", limb));
    }
    out.push_str("\t},\n");
    out
}

fn format_attack_todo(name: &str) -> String {
    let limb = guess_limb(name);
    format!(
        "\t{}: {{\n\t\thitbox0: {{ damage: 0 /*TODO*/, angle: 0 /*TODO*/, baseKnockback: 0 /*TODO*/, knockbackGrowth: 0 /*TODO*/, hitstop: -1, selfHitstop: -1, limb: {} }}\n\t}},\n",
        name, limb
    )
}

// ─── CharacterStats.hx ───────────────────────────────────────────────────────

fn generate_character_stats(data: &CharacterData, char_id: &str) -> String {
    let s = &data.stats;
    let todo = |v: f64| if v == 0.0 { " /*TODO*/" } else { "" };

    // Convert SSF2 values to Fraymakers equivalents (scaling driven by the
    // multipliers in mappings/character/stats.json).
    let gravity       = if s.gravity > 0.0      { ssf2_gravity_to_fm(s.gravity) }      else { 0.0 };
    let terminal_vel  = if s.fall_speed > 0.0   { ssf2_speed_to_fm(s.fall_speed) }     else { 0.0 };
    let fast_fall     = if s.fast_fall_speed > 0.0 { ssf2_speed_to_fm(s.fast_fall_speed) } else { 0.0 };
    let jump_speed    = if s.jump_height > 0.0  { ssf2_jump_to_fm(s.jump_height) }    else { 0.0 };
    let dj_speed      = if s.double_jump_height > 0.0 { ssf2_jump_to_fm(s.double_jump_height) } else { 0.0 };
    let walk_cap      = if s.walk_speed > 0.0   { ssf2_walk_to_fm(s.walk_speed) }     else { 0.0 };
    let dash_speed    = if s.dash_speed > 0.0   { ssf2_dash_to_fm(s.dash_speed) }     else { 0.0 };
    let aerial_fric   = if s.air_friction != 0.0 { ssf2_air_to_fm(s.air_friction) }   else { 0.0 };
    let weight        = if s.weight > 0.0 { s.weight } else { 0.0 };

    // offsets, derivations and flat constants all come from stats.json.
    let cfg = crate::mappings::character_stats();
    let c = |k: &str| cfg.constant(k);

    // Derived stats — expression strings in stats.jsonc, compiled once and
    // evaluated with the already-converted stats exposed as variables.
    let vars: std::collections::BTreeMap<String, f64> = [
        ("jump_speed".to_string(), jump_speed),
        ("air_mobility_raw".to_string(), s.air_mobility),
        ("aerial_friction".to_string(), aerial_fric),
    ].into_iter().collect();
    let short_hop  = crate::mappings::evaluate_stat_derivation("shortHopSpeed", &vars).unwrap_or(0.0);
    let aerial_cap = crate::mappings::evaluate_stat_derivation("aerialSpeedCap", &vars).unwrap_or(0.0);

    // doubleJumpSpeeds: the real converted value, or the JSON fallback default.
    let dj_array = if dj_speed > 0.0 {
        format!("[{}]", fmt(dj_speed))
    } else {
        format!("[{}] /*TODO*/", c("doubleJumpSpeedFallback"))
    };

    let mut out = format!(
        "// Character stats for {char_name} — converted from SSF2\n\
        // SSF2 physics values are scaled to Fraymakers equivalents.\n\
        // Review all values before use — units differ between engines.\n\
        {{\n\
        \tspriteContent: self.getResource().getContent(\"{char_id}\"),\n\n\
        \t//GENERIC STATS\n\
        \tbaseScaleX: {base_scale_x},\n\
        \tbaseScaleY: {base_scale_y},\n\
        \tweight: {weight}{weight_todo},\n\
        \tgravity: {gravity}{gravity_todo},\n\
        \tshortHopSpeed: {short_hop} /*TODO: set manually*/,\n\
        \tjumpSpeed: {jump_speed}{jump_speed_todo},\n\
        \tdoubleJumpSpeeds: {dj_array},\n\
        \tterminalVelocity: {terminal_vel}{terminal_vel_todo},\n\
        \tfastFallSpeed: {fast_fall}{fast_fall_todo},\n\
        \tfriction: {friction} /*TODO*/,\n\
        \twalkSpeedInitial: {walk_speed_initial},\n\
        \twalkSpeedAcceleration: {walk_speed_accel},\n\
        \twalkSpeedCap: {walk_cap}{walk_cap_todo},\n\
        \tdashSpeed: {dash_speed}{dash_speed_todo},\n\
        \trunSpeedInitial: {run_speed_initial},\n\
        \trunSpeedAcceleration: {run_speed_accel},\n\
        \trunSpeedCap: {run_speed_cap},\n\
        \tgroundSpeedAcceleration: {ground_speed_accel},\n\
        \tgroundSpeedCap: {ground_speed_cap},\n\
        \taerialSpeedAcceleration: {aerial_speed_accel},\n\
        \taerialSpeedCap: {aerial_cap}{aerial_cap_todo},\n\
        \taerialFriction: {aerial_fric}{aerial_fric_todo},\n\n",
        char_name = data.name,
        char_id = char_id,
        base_scale_x = fmt(s.base_scale_x),
        base_scale_y = fmt(s.base_scale_y),
        weight = fmt(weight), weight_todo = todo(weight),
        gravity = fmt(gravity), gravity_todo = todo(gravity),
        short_hop = fmt(short_hop),
        jump_speed = fmt(jump_speed), jump_speed_todo = todo(jump_speed),
        dj_array = dj_array,
        terminal_vel = fmt(terminal_vel), terminal_vel_todo = todo(terminal_vel),
        fast_fall = fmt(fast_fall), fast_fall_todo = todo(fast_fall),
        friction = c("friction"),
        walk_speed_initial = c("walkSpeedInitial"),
        walk_speed_accel = c("walkSpeedAcceleration"),
        walk_cap = fmt(walk_cap), walk_cap_todo = todo(walk_cap),
        dash_speed = fmt(dash_speed), dash_speed_todo = todo(dash_speed),
        run_speed_initial = c("runSpeedInitial"),
        run_speed_accel = c("runSpeedAcceleration"),
        run_speed_cap = c("runSpeedCap"),
        ground_speed_accel = c("groundSpeedAcceleration"),
        ground_speed_cap = c("groundSpeedCap"),
        aerial_speed_accel = c("aerialSpeedAcceleration"),
        aerial_cap = fmt(aerial_cap), aerial_cap_todo = todo(s.air_mobility),
        aerial_fric = fmt(aerial_fric), aerial_fric_todo = todo(aerial_fric),
    );

    // Flat-constant sections — every value comes from stats.json `constants`.
    out.push_str("\t//ENVIRONMENTAL COLLISION BODY (ECB) STATS\n");
    for (f, anno) in [
        ("floorHeadPosition", " /*TODO*/"),
        ("floorHipWidth", " /*TODO*/"),
        ("floorHipXOffset", ""),
        ("floorHipYOffset", ""),
        ("floorFootPosition", ""),
        ("aerialHeadPosition", " /*TODO*/"),
        ("aerialHipWidth", " /*TODO*/"),
        ("aerialHipXOffset", ""),
        ("aerialHipYOffset", ""),
        ("aerialFootPosition", " /*TODO*/"),
    ] {
        out.push_str(&format!("\t{}: {}{},\n", f, c(f), anno));
    }
    out.push('\n');

    out.push_str("\t//CAMERA BOX STATS\n");
    for f in ["cameraBoxOffsetX", "cameraBoxOffsetY", "cameraBoxWidth", "cameraBoxHeight"] {
        out.push_str(&format!("\t{}: {},\n", f, c(f)));
    }
    out.push('\n');

    out.push_str("\t//ROLL AND LEDGE JUMP STATS\n");
    for f in [
        "techRollSpeed", "techRollSpeedStartFrame", "techRollSpeedLength",
        "dodgeRollSpeed", "dodgeRollSpeedStartFrame", "dodgeRollSpeedLength",
        "getupRollSpeed", "getupRollSpeedStartFrame", "getupRollSpeedLength",
        "ledgeRollSpeed", "ledgeRollSpeedStartFrame", "ledgeRollSpeedLength",
        "ledgeJumpXSpeed", "ledgeJumpYSpeed",
    ] {
        out.push_str(&format!("\t{}: {},\n", f, c(f)));
    }
    out.push('\n');

    out.push_str("\t//AIRDASH STATS\n");
    for f in ["airdashInitialSpeed", "airdashSpeedCap", "airdashAccelMultiplier", "airdashCancelSpeedConservation"] {
        out.push_str(&format!("\t{}: {},\n", f, c(f)));
    }
    out.push('\n');

    out.push_str("\t//SHIELD STATS\n");
    for f in [
        "shieldCrossupThreshold", "shieldFrontNineSliceContent", "shieldFrontXOffset",
        "shieldFrontYOffset", "shieldFrontWidth", "shieldFrontHeight",
        "shieldBackNineSliceContent", "shieldBackXOffset", "shieldBackYOffset",
        "shieldBackWidth", "shieldBackHeight",
    ] {
        out.push_str(&format!("\t{}: {},\n", f, c(f)));
    }
    out.push('\n');

    out.push_str("\t//VOICE STATS\n");
    for f in [
        "attackVoiceIds", "hurtLightVoiceIds", "hurtMediumVoiceIds", "hurtHeavyVoiceIds", "koVoiceIds",
        "attackVoiceSilenceRate", "hurtLightSilenceRate", "hurtMediumSilenceRate",
        "hurtHeavySilenceRate", "koVoiceSilenceRate",
    ] {
        out.push_str(&format!("\t{}: {},\n", f, c(f)));
    }

    out.push_str("}\n");
    out
}

// ─── AnimationStats.hx ───────────────────────────────────────────────────────

fn generate_animation_stats(data: &CharacterData, splits: &[crate::anim_splitter::SplitAnim]) -> String {
    use std::collections::BTreeSet;

    // ── Base FM template: animations with hand-tuned properties ──────────────
    // These are the standard Fraymakers character-template entries.
    // Order and grouping match the official template.
    let template: Vec<(&str, &str)> = vec![
        // MOTIONS
        ("stand", ""),
        ("stand_turn", ""),
        ("walk_in", ""),
        ("walk_loop", ""),
        ("walk_out", ""),
        ("dash", ""),
        ("run", ""),
        ("run_turn", ""),
        ("skid", ""),
        ("jump_squat", ""),
        ("jump_in", ""),
        ("jump_midair", ""),
        ("jump_out", ""),
        ("fall_loop", ""),
        ("fall_special", ""),
        ("land_light", ""),
        ("land_heavy", ""),
        ("crouch_in", ""),
        ("crouch_loop", ""),
        ("crouch_out", ""),
        // AIRDASHES
        ("airdash_up", ""),
        ("airdash_down", ""),
        ("airdash_forward", ""),
        ("airdash_back", ""),
        ("airdash_forward_up", ""),
        ("airdash_forward_down", ""),
        ("airdash_back_up", ""),
        ("airdash_back_down", ""),
        ("airdash_freefall", ""),
        ("airdash_freefall_whiff", ""),
        // DEFENSE
        ("shield_in", ""),
        ("shield_loop", ""),
        ("shield_hurt", ""),
        ("shield_out", ""),
        ("parry_in", ""),
        ("parry_success", ""),
        ("parry_fail", ""),
        ("roll", ""),
        ("spot_dodge", ""),
        // ASSIST CALL
        ("assist_call", ""),
        ("assist_call_air", ""),
        // LIGHT ATTACKS
        ("jab1", ""),
        ("jab2", ""),
        ("jab3", ""),
        ("dash_attack", "xSpeedConservation: 1"),
        ("tilt_forward", ""),
        ("tilt_up", ""),
        ("tilt_down", ""),
        // STRONG ATTACKS
        ("strong_forward_in", ""),
        ("strong_forward_charge", ""),
        ("strong_forward_attack", ""),
        ("strong_up_in", ""),
        ("strong_up_charge", ""),
        ("strong_up_attack", ""),
        ("strong_down_in", ""),
        ("strong_down_charge", ""),
        ("strong_down_attack", ""),
        // AERIAL ATTACKS
        ("aerial_neutral", "landAnimation:\"aerial_neutral_land\""),
        ("aerial_forward", "landAnimation:\"aerial_forward_land\""),
        ("aerial_back", "landAnimation:\"aerial_back_land\""),
        ("aerial_up", "landAnimation:\"aerial_up_land\""),
        ("aerial_down", "landAnimation:\"aerial_down_land\", xSpeedConservation: 0.5, ySpeedConservation: 0.5, gravityMultiplier:0, allowMovement: false"),
        // AERIAL ATTACK LANDING
        ("aerial_neutral_land", ""),
        ("aerial_forward_land", ""),
        ("aerial_back_land", ""),
        ("aerial_up_land", ""),
        ("aerial_down_land", "xSpeedConservation: 0"),
        // SPECIAL ATTACKS
        ("special_neutral", ""),
        ("special_neutral_air", ""),
        ("special_up", "leaveGroundCancel:false, xSpeedConservation:0.5, ySpeedConservation:0.5, allowMovement: true, nextState:CState.FALL_SPECIAL"),
        ("special_up_air", "leaveGroundCancel:false, xSpeedConservation:0.5, ySpeedConservation:0.5, nextState:CState.FALL_SPECIAL, landType:LandType.TOUCH"),
        ("special_down", "allowFastFall:false, allowTurnOnFirstFrame: true, leaveGroundCancel:false, xSpeedConservation:0, ySpeedConservation:0"),
        ("special_down_loop", "endType:AnimationEndType.LOOP"),
        ("special_down_endlag", ""),
        ("special_down_air", "allowFastFall:false, allowTurnOnFirstFrame: true, leaveGroundCancel:false, xSpeedConservation:0, ySpeedConservation:0, landType:LandType.LINK_FRAMES, landAnimation:\"special_down\""),
        ("special_down_air_loop", "endType:AnimationEndType.LOOP, landType:LandType.LINK_FRAMES, landAnimation:\"special_down_loop\""),
        ("special_down_air_endlag", "landType:LandType.LINK_FRAMES, landAnimation:\"special_down\""),
        ("special_side", "allowFastFall: false, allowTurnOnFirstFrame: true, leaveGroundCancel:false, landType:LandType.TOUCH, landAnimation: \"land_heavy\", singleUse:true"),
        ("special_side_air", "allowFastFall: false, allowTurnOnFirstFrame: true, leaveGroundCancel:false, landType:LandType.TOUCH, landAnimation: \"land_heavy\", singleUse:true"),
        // THROWS
        ("grab", ""),
        ("grab_hold", ""),
        ("throw_forward", ""),
        ("throw_back", ""),
        ("throw_up", ""),
        ("throw_down", ""),
        // HURT ANIMATIONS
        ("hurt_light_low", ""),
        ("hurt_light_middle", ""),
        ("hurt_light_high", ""),
        ("hurt_medium", ""),
        ("hurt_heavy", ""),
        ("hurt_thrown", ""),
        ("tumble", ""),
        // CRASH
        ("crash_bounce", ""),
        ("crash_loop", ""),
        ("crash_get_up", ""),
        ("crash_attack", ""),
        ("crash_roll", ""),
        // LEDGE
        ("ledge_in", ""),
        ("ledge_loop", ""),
        ("ledge_roll_in", ""),
        ("ledge_roll", ""),
        ("ledge_climb_in", ""),
        ("ledge_climb", ""),
        ("ledge_attack_in", ""),
        ("ledge_attack", ""),
        ("ledge_jump_in", ""),
        ("ledge_jump", ""),
        // MISC
        ("revival", ""),
        ("emote", ""),
    ];

    // Collect template names for dedup
    let template_names: BTreeSet<&str> = template.iter().map(|(n, _)| *n).collect();

    let mut out = format!(
        "// Animation stats for {} — converted from SSF2\n\
         // Many values are automatically set by the Common class.\n\
         // Entries here override those defaults.\n\
         {{\n",
        data.name
    );

    // Emit template entries
    for (name, props) in &template {
        if props.is_empty() {
            out.push_str(&format!("\t{}: {{}},\n", name));
        } else {
            out.push_str(&format!("\t{}: {{{}}},\n", name, props));
        }
    }

    // Emit split animations not already in template
    let mut extra_names: Vec<&str> = Vec::new();
    for split in splits {
        if !template_names.contains(split.fm_name.as_str()) && !extra_names.contains(&split.fm_name.as_str()) {
            extra_names.push(&split.fm_name);
        }
    }
    if !extra_names.is_empty() {
        out.push_str("\n\t//SSF2 SPLIT ANIMATIONS\n");
        for name in &extra_names {
            // Check if this split has loop_tail
            let is_loop = splits.iter().any(|s| s.fm_name == *name && s.loop_tail);
            if is_loop {
                out.push_str(&format!("\t{}: {{endType:AnimationEndType.LOOP}},\n", name));
            } else {
                out.push_str(&format!("\t{}: {{}},\n", name));
            }
        }
    }

    out.push_str("}\n");
    out
}

// ─── Script.hx ───────────────────────────────────────────────────────────────

/// How many of jab1..jab4 actually have image content. Drives whether the
/// jab-chain boilerplate is emitted in Script.hx, and whether the empty-
/// animation drop in entity_gen keeps jab2/jab3 as referenced placeholders.
fn count_populated_jabs(img: &crate::image_extractor::ImageExtractionResult) -> usize {
    ["jab1", "jab2", "jab3", "jab4"].iter()
        .filter(|name| img.anim_images.get(**name)
            .map(|af| af.frames.values().any(|f| !f.is_empty()))
            .unwrap_or(false))
        .count()
}

fn generate_script(data: &CharacterData, _char_id: &str, populated_jabs: usize) -> String {
    // Filter out trivial slot-initializer stubs.
    let ext_methods: Vec<&crate::extractor::ScriptInfo> = data.scripts.iter()
        .filter(|s| s.is_ext_method)
        .filter(|s| !s.code.contains("Object.SSF2API"))
        .collect();

    // SSF2 ext methods whose names collide with the template functions are
    // MERGED into those functions (their bodies inlined after the template's
    // mandatory setup) instead of renamed to ssf2_*. That keeps one canonical
    // initialize() / update() / ... and avoids a redundant ssf2_initialize().
    const TEMPLATE_FNS: [&str; 5] =
        ["initialize", "update", "inputUpdateHook", "handleLinkFrames", "onTeardown"];
    let mut template_bodies: std::collections::BTreeMap<&str, String> =
        std::collections::BTreeMap::new();
    let mut regular_ext: Vec<&crate::extractor::ScriptInfo> = Vec::new();
    for s in &ext_methods {
        if let Some(tf) = TEMPLATE_FNS.iter().find(|t| s.name == **t).copied() {
            let translated = crate::api_mappings::translate_ssf2_to_fm(&s.code);
            if let Some(body) = extract_fn_body(&translated) {
                template_bodies.insert(tf, body);
            }
        } else {
            regular_ext.push(s);
        }
    }

    // Emit one merged template function per name. `setup` is the mandatory FM
    // line(s) the template must always include (e.g. the LINK_FRAMES listener
    // registration on initialize); the SSF2 body, if any, is appended.
    let emit_tpl = |out: &mut String, header_comment: &str, signature: &str, setup: &str, name: &str| {
        if !header_comment.is_empty() { out.push_str(header_comment); }
        out.push_str(signature);
        out.push_str(setup);
        if let Some(body) = template_bodies.get(name) {
            out.push_str(body);
            out.push('\n');
        }
        out.push_str("}\n\n");
    };

    let mut out = format!(
        "// API Script for {} — converted from SSF2\n\
// Frame scripts are embedded in the entity file (FRAME_SCRIPT layers).\n\
// SSF2 API calls are mapped to Fraymakers equivalents where possible.\n\
// Lines marked TODO need manual review.\n\n",
        data.name
    );

    // Instance variables carried over from the SSF2 XxxExt class (its
    // Slot/Const traits — `public var foo:T;`). Emitted as Fraymakers
    // persistent-state wrappers: `var foo = self.makeInt(0)` /
    // `self.makeBool(false)` / `self.makeObject(null)`, with the kind
    // inferred from each var's init expression in `ext_var_inits`.
    // Wrapped wrappers expose `.get() / .set(v) / .inc() / .dec()`.
    let var_types = crate::api_mappings::infer_ext_var_types(&data.ext_vars, &data.ext_var_inits);
    if !data.ext_vars.is_empty() {
        out.push_str("// ── Instance variables (from SSF2 ");
        out.push_str(&data.name);
        out.push_str("Ext) — wrapped for FM persistent state ──\n");
        for v in &data.ext_vars {
            let (factory, default) = match var_types.get(v).copied().unwrap_or(crate::api_mappings::ExtVarType::Object) {
                crate::api_mappings::ExtVarType::Bool   => ("makeBool", "false"),
                crate::api_mappings::ExtVarType::Int    => ("makeInt", "0"),
                crate::api_mappings::ExtVarType::Object => ("makeObject", "null"),
            };
            out.push_str(&format!("var {} = self.{}({});\n", v, factory, default));
        }
        out.push('\n');
    }

    out.push_str("// start general functions ---\n\n");

    // initialize — extend the template's setup with iinit-derived
    // `self.<var> = <expr>;` assignments for each ext_var, but SKIP any name
    // the merged-in SSF2 initialize body already assigns (per user: "if
    // something is already set in initialize then skip that").
    let init_body_text = template_bodies.get("initialize").map(|s| s.as_str()).unwrap_or("");
    let mut init_setup = String::from(
        "\tself.addEventListener(GameObjectEvent.LINK_FRAMES, handleLinkFrames, {persistent:true});\n"
    );
    for (name, expr) in &data.ext_var_inits {
        // Skip names the SSF2 initialize body already covers — match both
        // the legacy `self.X = ` form (in case the merged body hasn't been
        // wrapped yet) and the new `X.set(` form.
        let legacy_needle = format!("self.{} = ", name);
        let wrapped_needle = format!("{}.set(", name);
        if !init_body_text.contains(&legacy_needle) && !init_body_text.contains(&wrapped_needle) {
            // Skip emitting an initial assignment when the wrapper's own
            // default already matches (e.g. `var foo = self.makeBool(false)`
            // doesn't need `foo.set(false)` right after).
            let default_already_matches = matches!(
                (var_types.get(name).copied(), expr.trim()),
                (Some(crate::api_mappings::ExtVarType::Bool), "false")
                | (Some(crate::api_mappings::ExtVarType::Int), "0")
                | (Some(crate::api_mappings::ExtVarType::Object), "null"),
            );
            if !default_already_matches {
                init_setup.push_str(&format!("\t{}.set({});\n", name, expr));
            }
        }
    }
    emit_tpl(&mut out, "//Runs on object init\n", "function initialize(){\n",
        &init_setup, "initialize");
    emit_tpl(&mut out, "", "function update(){\n", "", "update");
    emit_tpl(&mut out,
        "// Runs when reading inputs (before determining character state, update, framescript, etc.)\n",
        "function inputUpdateHook(pressedControls:ControlsObject, heldControls:ControlsObject) {\n",
        "", "inputUpdateHook");
    emit_tpl(&mut out, "// CState-based handling for LINK_FRAMES\n",
        "function handleLinkFrames(e){\n", "", "handleLinkFrames");
    emit_tpl(&mut out, "", "function onTeardown() {\n", "", "onTeardown");

    out.push_str("// --- end general functions\n\n");

    if !regular_ext.is_empty() {
        out.push_str("// ── Decompiled from SSF2 XxxExt.as ─────────────────────────────────────────\n\n");
        for script in &regular_ext {
            let translated = crate::api_mappings::translate_ssf2_to_fm(&script.code);
            out.push_str(&translated);
            out.push('\n');
        }
    }

    // Frame scripts are embedded directly in the entity file via FRAME_SCRIPT layers.
    // They are no longer duplicated here.

    // Jab chain transition logic — only when the character actually has a
    // multi-hit combo. Single-jab characters get no chain boilerplate, so
    // nothing references the missing jab2/jab3 animations.
    if populated_jabs >= 2 {
        out.push_str(&generate_jab_scripts());
    }

    // Full-script post-pass: fix paired setIntangibility calls
    out = crate::api_mappings::fix_intangibility_pairs(&out);

    // Full-script post-pass: rewrite SSF2 instance-variable references
    // into Fraymakers persistent-state wrappers (`.get()/.set()/.inc()/
    // .dec()`). Frame scripts embedded in the entity get the same rewrite
    // in entity_gen so cross-file references stay consistent.
    out = crate::api_mappings::wrap_persistent_state(&out, &var_types);

    out
}

// ─── Jab chain scripts ─────────────────────────────────────────────────────

/// Generate jab1/jab2/jab3 chain transition frame scripts.
///
/// In SSF2, the single 'Jab_21' sprite has three sub-animations separated by
/// internal frame labels (begin → hit2 → hit3). SSF2 code calls `gotoAndPlay("hit2")`
/// to chain into the next hit on button press, and `gotoAndPlay("begin")` to loop jab1.
///
/// In Fraymakers, each is a separate animation. The chain logic lives in framescripts:
///   - jab1: on last frame, if attack pressed → enter jab2; else idle
///   - jab2: on last frame, if attack pressed → enter jab3; else idle
///   - jab3: on last frame → idle
/// Extract the body between the outermost braces of a `function NAME(...) { ... }`
/// string. Used to inline a translated SSF2 ext method into the matching
/// template function. Returns None if the body is empty or unbalanced.
fn extract_fn_body(code: &str) -> Option<String> {
    let open = code.find('{')?;
    let close = code.rfind('}')?;
    if close <= open { return None; }
    let body = code[open + 1..close].trim_matches('\n').trim_end();
    if body.is_empty() { None } else { Some(body.to_string()) }
}

fn generate_jab_scripts() -> String {
    r#"
// ── Jab chain — SSF2 Jab_21 sub-animations (begin / hit2 / hit3) ─────────────────
// SSF2 uses gotoAndPlay("hit2") / gotoAndPlay("hit3") to chain jabs on button press.
// In Fraymakers, jab1/jab2/jab3 are separate animations chained via CState transitions.

// Called from AnimationStats.jab1 last-frame handler (link in FrayTools):
function jab1_end() {
	if (entity.checkInput(ControlsObject.ATTACK)) {
		// Player pressed attack again — chain to jab2
		entity.setAnimation("jab2");
		entity.playCState(CState.JAB2);
	} else {
		// No input — return to idle
		entity.playCState(CState.IDLE);
	}
}

// Called from AnimationStats.jab2 last-frame handler:
function jab2_end() {
	if (entity.checkInput(ControlsObject.ATTACK)) {
		entity.setAnimation("jab3");
		entity.playCState(CState.JAB3);
	} else {
		entity.playCState(CState.IDLE);
	}
}

// Called from AnimationStats.jab3 last-frame handler:
function jab3_end() {
	entity.playCState(CState.IDLE);
}
"#.to_string()
}

// ─── manifest.json ───────────────────────────────────────────────────────────

fn generate_manifest(char_id: &str, display_name: &str, projectile_names: &[String]) -> String {
    let ai_id   = format!("{}Ai", char_id);
    let ai_script_id = format!("{}AiScript", char_id);

    let mut content = vec![serde_json::json!({
            "id": char_id,
            "name": display_name,
            "description": format!("{} — converted from Super Smash Flash 2", display_name),
            "type": "character",
            "objectStatsId":    format!("{}CharacterStats", char_id),
            "animationStatsId": format!("{}AnimationStats", char_id),
            "hitboxStatsId":    format!("{}HitboxStats", char_id),
            "scriptId":         format!("{}Script", char_id),
            "costumesId":       format!("{}Costumes", char_id),
            "aiId":             ai_id,
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

    content.push(serde_json::json!({
        "id":       ai_id,
        "type":     "characterAi",
        "scriptId": ai_script_id
    }));

    // Add projectile entries
    for proj_name in projectile_names {
        let entity_id = proj_name.replace('_', "");
        content.push(serde_json::json!({
            "id":               format!("{}Projectile", entity_id),
            "type":             "projectile",
            "objectStatsId":    format!("{}ProjectileStats", entity_id),
            "animationStatsId": format!("{}ProjectileAnimationStats", entity_id),
            "hitboxStatsId":    format!("{}ProjectileHitboxStats", entity_id),
            "scriptId":         format!("{}ProjectileScript", entity_id),
            "costumesId":       format!("{}Costumes", char_id)
        }));
    }

    serde_json::json!({
        "resourceId": char_id,
        "content": content
    }).to_string()
}

// ─── Sound content entries ────────────────────────────────────────────────────

/// Write a `.wav.meta` sidecar next to each extracted audio file, matching
/// the schema observed in `Fraymakers/character-template` (id = filename
/// sans `.wav`, plus pluginMetadata + plugins references). No central audio
/// manifest is needed: reference characters register sounds purely through
/// these per-file sidecars.
fn generate_sound_entries(
    char_dir: &Path,
    char_name: &str,
    sounds: &[crate::sound_extractor::SoundEntry],
) -> Result<()> {
    let audio_dir = char_dir.join("library/audio");
    fs::create_dir_all(&audio_dir)?;

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

    Ok(())
}

/// Convert a projectile name to a valid entity filename.
/// "mario_fireball" → "mario_fireball"
fn sanitize_entity_name(name: &str) -> String {
    name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "_")
}

/// Convert an SSF2 projectile name like `dee_nspec` to the PascalCase
/// prefix used for the script-file names (`DeeNspec` → `DeeNspecScript.hx`).
/// Matches the convention seen in real FM character mods
/// (aJewelofRarity/AnnieCharacter: `Cut` → `CutScript.hx`).
fn snake_to_pascal(name: &str) -> String {
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
    /// Projectile companion stats files — empty pluginMetadata, no plugin.
    ProjectileStats,
    /// Projectile companion stats files — empty pluginMetadata, no plugin.
    ProjectileAnimationStats,
    /// Projectile companion stats files — empty pluginMetadata, no plugin.
    ProjectileHitboxStats,
}

/// Generate a `.hx.meta` sidecar matching the Fraymakers reference layout.
/// All script-kind .meta files use `language: "hscript"`. The
/// `pluginMetadata` shape and `plugins` array vary by kind — see the enum
/// docs. Cross-referenced against Fraymakers/character-template.
fn script_meta(id: &str, guid: &str, kind: ScriptMetaKind) -> String {
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
        CharacterScript | ProjectileScript | CharacterStats => {
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
fn generate_projectile_script(
    _char_id: &str,
    entity_id: &str,
    extra_states: &[entity_gen::ProjectileStateData],
) -> String {
    if extra_states.is_empty() {
        // Single-state: standard template
        format!(
"// Projectile script for {entity_id} -- converted from SSF2
// TODO: tune X_SPEED / Y_SPEED and gravity to match SSF2 behaviour.

var X_SPEED = 8;
var Y_SPEED = 0;

function initialize() {{
    self.addEventListener(EntityEvent.COLLIDE_FLOOR, onGroundHit, {{ persistent: true }});
    self.addEventListener(GameObjectEvent.HIT_DEALT,  onHit,       {{ persistent: true }});

    self.setCostumeIndex(self.getOwner().getCostumeIndex());
    Common.enableReflectionListener({{ mode: \"X\", replaceOwner: true }});

    self.setState(PState.ACTIVE);
    self.setXSpeed(X_SPEED);
    self.setYSpeed(Y_SPEED);
}}

function onGroundHit(event) {{
    self.removeEventListener(EntityEvent.COLLIDE_FLOOR, onGroundHit);
    self.removeEventListener(GameObjectEvent.HIT_DEALT,  onHit);
    self.toState(PState.DESTROYING);
}}

function onHit(event) {{
    self.removeEventListener(EntityEvent.COLLIDE_FLOOR, onGroundHit);
    self.removeEventListener(GameObjectEvent.HIT_DEALT,  onHit);
    self.toState(PState.DESTROYING);
}}

function update() {{
    // Projectile moves via setXSpeed/setYSpeed set in initialize().
    // Add custom movement logic here if needed.
}}
",
            entity_id = entity_id)
    } else {
        // Multi-state: use Fraymakers local state machine instead of fake PStates
        // Each SSF2 frame label becomes an LState that drives animation switching.
        let mut lstate_prep: String = String::new();
        let mut lstate_fields: String = String::new();
        let mut update_branches: String = String::new();

        // First LState: idle (the attack_idle inner sprite is already projectileIdle)
        lstate_prep.push_str("    IDLE:    _prepLocalState(\"projectileIdle\"),\n");
        for state in extra_states {
            let fm = entity_gen::ssf2_proj_label_to_fm_anim(&state.label);
            let lname = match state.label.as_str() {
                "attack_hold" => "HELD",
                "attack_toss" => "ACTIVE",
                _ => "CUSTOM",
            };
            lstate_prep.push_str(&format!("    {lname}: _prepLocalState(\"{fm}\"),\n"));
            let fc = state.frame_count;
            update_branches.push_str(&format!(
"    }} else if (Common.inLocalState(LState.{lname})) {{
        // TODO: implement {lname} state logic ({fc} frames)
        if (self.finalFramePlayed()) {{
            Common.toLocalState(LState.IDLE);
        }}
",
                lname = lname, fc = fc));
        }

        format!(
"// Projectile script for {entity_id} -- converted from SSF2 (multi-state)
// Uses the local state machine to switch between animations since PState
// only supports built-in values (ACTIVE, DESTROYING, etc).
// TODO: tune X_SPEED / Y_SPEED and gravity to match SSF2 behaviour.

var X_SPEED = 8;
var Y_SPEED = 0;

// ---- Local state machine setup ----
function _prepLocalState(animation:String, ?index:Int=Math.NaN):Int {{
    if (!__hasInitLocalStateMachine) {{
        Common.initLocalStateMachine();
        __hasInitLocalStateMachine = true;
    }}
    if (index != Math.NaN) {{
        index = __localStatePrepIndex++;
    }}
    Common.registerLocalState(index, animation);
    return index;
}}
var __hasInitLocalStateMachine = false;
var __localStatePrepIndex = -1;

var LState = {{
{lstate_prep}}}

function initialize() {{
    self.addEventListener(EntityEvent.COLLIDE_FLOOR, onGroundHit, {{ persistent: true }});
    self.addEventListener(GameObjectEvent.HIT_DEALT,  onHit,       {{ persistent: true }});

    self.setCostumeIndex(self.getOwner().getCostumeIndex());
    Common.enableReflectionListener({{ mode: \"X\", replaceOwner: true }});

    self.setState(PState.ACTIVE);
    Common.toLocalState(LState.IDLE);
    self.setXSpeed(X_SPEED);
    self.setYSpeed(Y_SPEED);
}}

function onGroundHit(event) {{
    self.removeEventListener(EntityEvent.COLLIDE_FLOOR, onGroundHit);
    self.removeEventListener(GameObjectEvent.HIT_DEALT,  onHit);
    self.toState(PState.DESTROYING);
}}

function onHit(event) {{
    self.removeEventListener(EntityEvent.COLLIDE_FLOOR, onGroundHit);
    self.removeEventListener(GameObjectEvent.HIT_DEALT,  onHit);
    self.toState(PState.DESTROYING);
}}

function update() {{
    if (Common.inLocalState(LState.IDLE)) {{
        // TODO: implement IDLE state logic (projectileIdle animation)
{update_branches}    }}
}}
",
            entity_id = entity_id,
            lstate_prep = lstate_prep,
            update_branches = update_branches)
    }
}

/// ProjectileAnimationStats.hx — endType for each projectile animation.
/// Three anchor animation names used across a projectile's stats files.
/// When SSF2's inner sprite carries its own FrameLabel tags, those literal
/// labels become the names; otherwise the converter falls back to FM's
/// template trio (`projectileSpawn` / `projectileIdle` / `projectileDestroy`).
/// This is the projectile-side parallel to character xframe-label
/// extraction the user explicitly asked for.
struct ProjectileAnims {
    spawn:   String,
    active:  String,
    destroy: String,
    /// All distinct animation names referenced by this projectile, in
    /// timeline order. Includes spawn/active/destroy plus any extra
    /// labels from `inner_labels` or multi-state `extra_states`.
    all: Vec<String>,
}

fn projectile_anim_names(proj: &entity_gen::ProjectileInfo) -> ProjectileAnims {
    // Sanitize a raw SSF2 label into a valid Haxe identifier.
    fn ident(s: &str) -> String {
        let cleaned: String = s.chars()
            .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
            .collect();
        // Disallow leading digit.
        if cleaned.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            format!("_{}", cleaned)
        } else if cleaned.is_empty() {
            "projectile".to_string()
        } else {
            cleaned
        }
    }

    // Prefer real SSF2 labels when present, ordered by frame.
    let mut sorted: Vec<&(u16, String)> = proj.inner_labels.iter().collect();
    sorted.sort_by_key(|(f, _)| *f);
    let raw: Vec<String> = sorted.into_iter().map(|(_, l)| ident(l)).collect();

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
        _ => (raw[0].clone(), raw[1].clone(), raw[raw.len() - 1].clone()),
    };

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

fn generate_projectile_animation_stats(proj: &entity_gen::ProjectileInfo) -> String {
    let anims = projectile_anim_names(proj);
    let mut lines: Vec<String> = Vec::new();
    for (i, n) in anims.all.iter().enumerate() {
        // Mark the destroy animation with resetId:false to prevent
        // hit-id churn on state change (per template convention).
        if n == &anims.destroy && i > 0 {
            lines.push(format!("    {n}: {{ xSpeedConservation: 0, ySpeedConservation: 0, resetId: false }}"));
        } else {
            lines.push(format!("    {n}: {{ endType: AnimationEndType.NONE }}"));
        }
    }
    format!(
"// Animation stats for projectile (names from SSF2 inner-sprite FrameLabel tags)
{{
{body}
}}
",
        body = lines.join(",\n"))
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
fn best_match_projectile_data<'a>(
    proj_name: &str,
    projectile_data: &'a std::collections::BTreeMap<String, crate::abc_parser::ProjectileData>,
) -> Option<&'a crate::abc_parser::ProjectileData> {
    if projectile_data.is_empty() { return None; }
    if let Some(exact) = projectile_data.get(proj_name) { return Some(exact); }
    let lower = proj_name.to_lowercase();
    let candidate_keys: &[&str] = if lower.contains("nspec") || lower.contains("nspecial") {
        &["b", "nspec", "neutral_special"]
    } else if lower.contains("fspec") || lower.contains("sspec") || lower.contains("sspecial") {
        &["b_forward", "fspec", "sspec", "side_special"]
    } else if lower.contains("uspec") || lower.contains("uspecial") {
        &["b_up", "uspec", "up_special"]
    } else if lower.contains("dspec") || lower.contains("dspecial") {
        &["b_down", "dspec", "down_special"]
    } else if lower.contains("finalsmash") || lower.contains("_fs") || lower.ends_with("fs") {
        &["final_smash", "finalsmash", "fs"]
    } else {
        &[]
    };
    for k in candidate_keys {
        if let Some(d) = projectile_data.get(*k) { return Some(d); }
        // Fuzzy: any key containing the candidate
        if let Some((_, d)) = projectile_data.iter().find(|(name, _)| name.contains(*k)) {
            return Some(d);
        }
    }
    // Fallback: pick the entry with the most populated data.
    projectile_data.values()
        .max_by_key(|d| d.stats.len() + d.hitboxes.len())
}

fn generate_projectile_stats(
    _char_id: &str,
    entity_id: &str,
    proj: &entity_gen::ProjectileInfo,
    ssf2_match: Option<&crate::abc_parser::ProjectileData>,
) -> String {
    let content_id = format!("{}Projectile", entity_id);
    let anims = projectile_anim_names(proj);

    // SSF2 physics field name → FM ProjectileStats field name. The set
    // is small and most names align; this table handles the few that
    // need renaming. Unknown SSF2 keys are emitted as `// TODO: …`.
    let physics_map: &[(&str, &str)] = &[
        ("gravity",            "gravity"),
        ("friction",           "friction"),
        ("terminalVelocity",   "terminalVelocity"),
        ("groundSpeedCap",     "groundSpeedCap"),
        ("aerialSpeedCap",     "aerialSpeedCap"),
        ("aerialFriction",     "aerialFriction"),
        ("xSpeed",             "groundSpeedCap"),  // proxy: SSF2 starting xSpeed ≈ cap
        ("ySpeed",             "aerialSpeedCap"),
    ];

    // Build the physics lines from SSF2 data where available.
    let (mut physics_lines, mut todo_lines, source_note): (Vec<String>, Vec<String>, String) =
        match ssf2_match {
            Some(d) if !d.stats.is_empty() => {
                let mut taken: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
                let mut out: Vec<String> = Vec::new();
                for (ssf2_key, fm_field) in physics_map {
                    if taken.contains(fm_field) { continue; }
                    if let Some(v) = d.stats.get(*ssf2_key) {
                        out.push(format!("    {}: {},", fm_field, fmt_num(*v)));
                        taken.insert(fm_field);
                    }
                }
                // Surface any SSF2 stat keys we didn't map as TODOs
                let mut todos: Vec<String> = Vec::new();
                let mapped_ssf2: std::collections::BTreeSet<&str> = physics_map.iter()
                    .filter(|(_, fm)| taken.contains(fm))
                    .map(|(s, _)| *s)
                    .collect();
                for (k, v) in &d.stats {
                    if !mapped_ssf2.contains(k.as_str()) {
                        todos.push(format!("    // TODO: SSF2 {}: {} — no ProjectileStats mapping", k, fmt_num(*v)));
                    }
                }
                (out, todos, "// Values pulled from SSF2 getProjectileStats() (best-effort key match — verify per projectile).\n".to_string())
            }
            _ => (vec![], vec![], "// SSF2 source had no matching projectile-stats entry — defaults below.\n".to_string()),
        };

    // Always emit the scaffolding defaults for fields SSF2 didn't supply.
    let scaffolding: &[(&str, &str)] = &[
        ("gravity",          "0.7"),
        ("friction",         "0"),
        ("terminalVelocity", "20"),
        ("groundSpeedCap",   "11"),
        ("aerialSpeedCap",   "11"),
        ("aerialFriction",   "0"),
    ];
    let provided: std::collections::BTreeSet<String> = physics_lines.iter()
        .filter_map(|l| l.split(':').next().map(|s| s.trim().to_string()))
        .collect();
    for (k, v) in scaffolding {
        if !provided.contains(*k) {
            physics_lines.push(format!("    {}: {},", k, v));
        }
    }
    physics_lines.sort();
    todo_lines.sort();

    let body = format!(
"// Projectile stats for {entity_id}
{source_note}{{
    spriteContent: self.getResource().getContent(\"{content_id}\"),
    stateTransitionMapOverrides: [
        PState.ACTIVE => {{ animation: \"{active}\" }},
        PState.DESTROYING => {{ animation: \"{destroy}\" }}
    ],
    shadows: true,
{physics}
{todos}
    floorHeadPosition: 15,
    floorHipWidth: 16,
    floorHipXOffset: 0,
    floorHipYOffset: 0,
    floorFootPosition: 0,
    aerialHeadPosition: 15,
    aerialHipWidth: 16,
    aerialHipXOffset: 0,
    aerialHipYOffset: 0,
    aerialFootPosition: 0
}}
",
        entity_id = entity_id,
        source_note = source_note,
        content_id = content_id,
        physics = physics_lines.join("\n"),
        todos = if todo_lines.is_empty() { String::new() } else { todo_lines.join("\n") + "\n" },
        active = anims.active,
        destroy = anims.destroy,
    );
    body.replace("{{", "{").replace("}}", "}")
}

/// ProjectileHitboxStats.hx — pulls real hitbox values from SSF2's
/// `getProjectileStats()` data when a match is found. Uses
/// `mappings/character/hitbox_stats.jsonc` as the canonical SSF2→FM
/// field-name table (same source the character HitboxStats.hx generator
/// uses — single source of truth, not duplicated).
fn generate_projectile_hitbox_stats(
    _char_id: &str,
    entity_id: &str,
    proj: &entity_gen::ProjectileInfo,
    ssf2_match: Option<&crate::abc_parser::ProjectileData>,
) -> String {
    let mapping = crate::mappings::character_hitbox_stats();
    let anims = projectile_anim_names(proj);

    let (idle_body, source_note) = match ssf2_match.and_then(|d| d.hitboxes.first()) {
        Some(hb) => {
            // Build hitbox0 from SSF2 keys via the hitbox_stats.jsonc canon.
            let mut parts: Vec<String> = Vec::new();
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
                    if field.isframe { v *= 2.0; } // 30→60fps
                    parts.push(format!("{}: {}", field.fm_field, fmt_num(v)));
                }
            }
            // Always emit the projectile-typical flags
            parts.push("reversibleAngle: true".to_string());
            parts.push("directionalInfluence: false".to_string());
            parts.push("reflectable: true".to_string());
            (
                format!("hitbox0: {{ {} }}", parts.join(", ")),
                "// Values pulled from SSF2 getProjectileStats() (best-effort key match — verify per projectile).\n// SSF2→FM field names come from mappings/character/hitbox_stats.jsonc (shared canon).\n".to_string(),
            )
        }
        None => (
            "hitbox0: { damage: 6, knockbackGrowth: 30, baseKnockback: 65, angle: 0, reversibleAngle: true, directionalInfluence: false, reflectable: true }"
                .to_string(),
            "// TODO: no matching SSF2 projectile-stats entry found — values below are scaffolding placeholders. Tune to match SSF2.\n".to_string(),
        ),
    };

    // Build the projection-by-animation block dynamically: all known
    // projectile animations get an entry; the hitbox payload lives on
    // the "active" animation (matches FM convention — hitboxes are
    // attached to the animation that's playing while the projectile is
    // alive). Spawn/destroy animations stay empty unless the SSF2 source
    // explicitly carried hitbox data for them, which we don't currently
    // surface (per-animation hitboxes is a follow-up).
    let mut anim_blocks: Vec<String> = Vec::new();
    for name in &anims.all {
        if name == &anims.active {
            anim_blocks.push(format!("    {name}: {{\n        {idle_body}\n    }}",
                idle_body = idle_body));
        } else {
            anim_blocks.push(format!("    {name}: {{}}"));
        }
    }
    let body = format!(
"// Hitbox stats for {entity_id}
{source_note}{{
{blocks}
}}
",
        entity_id = entity_id,
        source_note = source_note,
        blocks = anim_blocks.join(",\n"),
    );
    body.replace("{{", "{").replace("}}", "}")
}

/// Format a float like SSF2 would have written it: integers stay integers,
/// floats round to 3 decimal places without trailing zeros.
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 { format!("{}", v as i64) }
    else {
        let s = format!("{:.3}", v);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        s.to_string()
    }
}
