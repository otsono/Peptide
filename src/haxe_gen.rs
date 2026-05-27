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

pub fn generate(output_dir: &Path, char_name: &str, data: &CharacterData, sprite_boxes: &std::collections::BTreeMap<String, crate::sprite_parser::AnimationBoxData>, img_result: &crate::image_extractor::ImageExtractionResult, costumes_json: Option<&Path>, sounds: &[crate::sound_extractor::SoundEntry], projectiles: &[crate::image_extractor::DiscoveredProjectile], head_sprite: Option<&crate::image_extractor::DiscoveredHead>, swf_data: &[u8]) -> Result<()> {
    let char_id = char_name.to_lowercase().replace(" ", "");
    let char_dir = output_dir.join(&char_id);
    let scripts_dir = char_dir.join("library/scripts/Character");
    fs::create_dir_all(&scripts_dir)?;

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
        let entity_id = proj.name.replace('_', "");
        let proj_scripts_dir = char_dir.join(format!("library/scripts/Projectile_{}", proj.name));
        fs::create_dir_all(&proj_scripts_dir)?;

        fs::write(
            proj_scripts_dir.join("ProjectileScript.hx"),
            generate_projectile_script(&char_id, &entity_id, &proj_info.extra_states),
        )?;
        fs::write(
            proj_scripts_dir.join("ProjectileScript.hx.meta"),
            script_meta(
                &format!("{}ProjectileScript", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileScript::meta", char_id, proj.name)),
                ScriptMetaKind::ProjectileScript,
            ),
        )?;
        fs::write(
            proj_scripts_dir.join("ProjectileAnimationStats.hx"),
            generate_projectile_animation_stats(&proj_info.extra_states),
        )?;
        fs::write(
            proj_scripts_dir.join("ProjectileAnimationStats.hx.meta"),
            script_meta(
                &format!("{}ProjectileAnimationStats", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileAnimationStats::meta", char_id, proj.name)),
                ScriptMetaKind::ProjectileAnimationStats,
            ),
        )?;
        fs::write(
            proj_scripts_dir.join("ProjectileStats.hx"),
            generate_projectile_stats(&char_id, &entity_id, &proj_info.extra_states),
        )?;
        fs::write(
            proj_scripts_dir.join("ProjectileStats.hx.meta"),
            script_meta(
                &format!("{}ProjectileStats", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileStats::meta", char_id, proj.name)),
                ScriptMetaKind::ProjectileStats,
            ),
        )?;
        fs::write(
            proj_scripts_dir.join("ProjectileHitboxStats.hx"),
            generate_projectile_hitbox_stats(&char_id, &entity_id, &proj_info),
        )?;
        fs::write(
            proj_scripts_dir.join("ProjectileHitboxStats.hx.meta"),
            script_meta(
                &format!("{}ProjectileHitboxStats", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileHitboxStats::meta", char_id, proj.name)),
                ScriptMetaKind::ProjectileHitboxStats,
            ),
        )?;
        log::info!("Generated projectile scripts for {}", proj.name);
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
    // Slot/Const traits — `public var foo:T;`). Emitted untyped at top
    // level so the references later in the translated methods aren't
    // undeclared. Types and initial values are not carried; review and
    // assign defaults as needed (often in initialize() via self.foo = ...).
    if !data.ext_vars.is_empty() {
        out.push_str("// ── Instance variables (from SSF2 ");
        out.push_str(&data.name);
        out.push_str("Ext) ──────────────────────────\n");
        for v in &data.ext_vars {
            out.push_str(&format!("var {};\n", v));
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
        let needle = format!("self.{} = ", name);
        if !init_body_text.contains(&needle) {
            init_setup.push_str(&format!("\tself.{} = {};\n", name, expr));
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

/// Generate manifest content entries + .meta sidecar files for extracted sounds.
/// Sounds live in library/sounds/*.ogg and are referenced by content id
/// "{char_name}::{sound_name}" so Script.hx can call AudioClip.play("mario::mario_jumpsfx").
fn generate_sound_entries(
    char_dir: &Path,
    char_name: &str,
    sounds: &[crate::sound_extractor::SoundEntry],
) -> Result<()> {
    let sounds_dir = char_dir.join("library/sounds");
    fs::create_dir_all(&sounds_dir)?;

    // Build a sounds manifest listing all audio content ids
    let sound_entries: Vec<serde_json::Value> = sounds.iter().map(|s| {
        let safe_name: String = s.name.chars()
            .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();
        let content_id = format!("{}::{}", char_name, safe_name);
        let ogg_path   = format!("sounds/{}.ogg", safe_name);
        serde_json::json!({
            "id":      content_id,
            "type":    "audio",
            "path":    ogg_path,
            "metadata": {
                "originalName": s.name,
                "sampleRate":   s.sample_rate,
                "sampleCount":  s.sample_count,
                "durationSecs": s.duration_secs(),
            }
        })
    }).collect();

    // Write sounds_manifest.json alongside the main manifest
    let sounds_manifest = serde_json::json!({
        "sounds": sound_entries,
        "_note": "Content ids for use in Script.hx: AudioClip.play(\"<id>\")"
    });
    fs::write(
        char_dir.join("library/sounds_manifest.json"),
        serde_json::to_string_pretty(&sounds_manifest)?,
    )?;

    // Write a .meta sidecar for each OGG file that exists
    for s in sounds {
        let safe_name: String = s.name.chars()
            .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();
        let ogg_path = sounds_dir.join(format!("{}.ogg", safe_name));
        if !ogg_path.exists() { continue; }

        let content_id = format!("{}::{}", char_name, safe_name);
        let guid = det_uuid(&format!("{}::sound_meta_{}", char_name, safe_name));
        let meta = serde_json::json!({
            "guid": guid,
            "id":   content_id,
            "type": "audio"
        });
        fs::write(
            sounds_dir.join(format!("{}.ogg.meta", safe_name)),
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
                "version": "0.1.2"
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
fn generate_projectile_animation_stats(extra_states: &[entity_gen::ProjectileStateData]) -> String {
    let mut extra_lines = String::new();
    for state in extra_states {
        let fm = entity_gen::ssf2_proj_label_to_fm_anim(&state.label);
        extra_lines.push_str(&format!("    {fm}: {{ endType: AnimationEndType.NONE }},\n"));
    }
    format!(
"// Animation stats for projectile
{{
    projectileSpawn:   {{ endType: AnimationEndType.NONE }},
{extra_lines}    projectileIdle:    {{ endType: AnimationEndType.NONE }},
    projectileDestroy: {{ xSpeedConservation: 0, ySpeedConservation: 0, resetId: false }}
}}
",
        extra_lines = extra_lines)
}

/// ProjectileStats.hx — physics, geometry, and state → animation mapping.
fn generate_projectile_stats(
    _char_id: &str,
    entity_id: &str,
    extra_states: &[entity_gen::ProjectileStateData],
) -> String {
    let content_id = format!("{}Projectile", entity_id);
    // Multi-state projectiles still map PState.ACTIVE → projectileIdle;
    // animation switching between substates is done via Common.toLocalState() in Script.hx.
    let _ = extra_states; // used in Script.hx, not Stats
    format!(
"// Projectile stats for {entity_id}
{{
    spriteContent: self.getResource().getContent(\"{content_id}\"),
    stateTransitionMapOverrides: [
        PState.ACTIVE => {{
            animation: \"projectileIdle\"
        }},
        PState.DESTROYING => {{
            animation: \"projectileDestroy\"
        }}
    ],
    gravity: 0.7,
    shadows: true,
    friction: 0,
    groundSpeedCap: 11,
    aerialSpeedCap: 11,
    aerialFriction: 0,
    terminalVelocity: 20,
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
        content_id = content_id,
    )
}

/// ProjectileHitboxStats.hx — hitbox entries extracted from the SSF2 attack data.
/// Emits one `hitbox0` entry under `projectileIdle` using the first attack that
/// references this projectile, if any.
fn generate_projectile_hitbox_stats(
    char_id: &str,
    entity_id: &str,
    proj: &entity_gen::ProjectileInfo,
) -> String {
    // Try to pull the first hitbox from the collision boxes
    // proj.boxes is Option<AnimationBoxData>: frames → Vec<FrameBox>
    // For now emit a sensible default (damage 6, knockback like the template)
    // since full per-hitbox attack data cross-referencing isn't wired up yet.
    format!(r#"// Hitbox stats for {entity_id}
// TODO: tune damage, knockback, and angle to match SSF2.
{{
    projectileSpawn: {{}},
    projectileIdle: {{
        hitbox0: {{ damage: 6, knockbackGrowth: 30, baseKnockback: 65, angle: 0, reversibleAngle: true, directionalInfluence: false, reflectable: true }}
    }},
    projectileDestroy: {{}}
}}
"#,
        entity_id = entity_id,
    ).replace("{{", "{").replace("}}", "}")
}
