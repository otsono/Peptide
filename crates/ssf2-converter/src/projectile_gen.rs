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
    format!("{}Projectile", ssf2_name.replace('_', ""))
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
    pub palette_collection_guid: Option<&'a str>,
    pub palette_base_map_id: Option<&'a str>,
}

/// Emit every projectile's `.entity` + Script/Stats/AnimationStats/HitboxStats
/// files. Layout matches the real-FM-mod convention: a single
/// `library/scripts/Projectile/` directory holds all projectiles, each file
/// prefixed by the projectile name in PascalCase.
pub fn generate_projectiles(
    ctx: &ProjectileGenCtx,
    projectiles: &[DiscoveredProjectile],
) -> Result<()> {
    let owner_id = ctx.owner_id;
    for proj in projectiles {
        // Extract image frames from the inner sprite using effect-sprite flattening.
        let (image_frames, image_matrices, image_guids) = if let Some(inner_id) = proj.inner_sprite_id {
            match crate::image_extractor::extract_projectile_frame_images_from_swf(
                ctx.parsed_swf, owner_id, inner_id, ctx.img_result,
            ) {
                Ok(pfi) => {
                    log::debug!("Projectile '{}': {} image frames", proj.name, pfi.frames.len());
                    (pfi.frames, pfi.matrices, pfi.image_guids)
                }
                Err(e) => {
                    log::warn!("Failed to extract images for projectile '{}': {}", proj.name, e);
                    (vec![], vec![], std::collections::BTreeMap::new())
                }
            }
        } else {
            (vec![], vec![], std::collections::BTreeMap::new())
        };

        // Extract collision boxes from the inner sprite.
        let boxes = if let Some(inner_id) = proj.inner_sprite_id {
            match crate::sprite_parser::extract_boxes_for_sprite_id_from_swf(ctx.parsed_swf, inner_id) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("Failed to extract boxes for projectile '{}': {}", proj.name, e);
                    None
                }
            }
        } else {
            None
        };

        // Extract image+box data for each extra state (multi-state projectiles like link_bomb).
        let mut extra_states: Vec<entity_gen::ProjectileStateData> = Vec::new();
        for state in &proj.states {
            if state.label == "attack_idle" {
                continue; // already extracted above
            }
            let (sf, sm, sg) = match crate::image_extractor::extract_projectile_frame_images_from_swf(
                ctx.parsed_swf, owner_id, state.inner_sprite_id, ctx.img_result,
            ) {
                Ok(pfi) => (pfi.frames, pfi.matrices, pfi.image_guids),
                Err(e) => {
                    log::warn!("State '{}' image extraction failed: {}", state.label, e);
                    (vec![], vec![], std::collections::BTreeMap::new())
                }
            };
            let sb = crate::sprite_parser::extract_boxes_for_sprite_id_from_swf(
                ctx.parsed_swf,
                state.inner_sprite_id,
            )
            .unwrap_or_default();
            extra_states.push(entity_gen::ProjectileStateData {
                label: state.label.clone(),
                image_frames: sf,
                image_matrices: sm,
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
            image_matrices,
            image_guids,
            extra_states,
            inner_labels: proj.inner_labels.clone(),
        };

        let filename = format!("{}.entity", haxe_gen::sanitize_entity_name(&proj.name));
        let mut proj_json = entity_gen::generate_projectile_entity(owner_id, &proj_info);
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
        fs::write(ctx.entities_dir.join(&filename), proj_json)?;
        log::info!("Generated projectile entity: {} ({} frames)", filename, proj.inner_frame_count);

        // ── projectile script files ──────────────────────────────────────────
        let entity_id = proj.name.replace('_', "");
        let pascal = haxe_gen::snake_to_pascal(&proj.name);
        let proj_scripts_dir = ctx.scripts_root.join("Projectile");
        fs::create_dir_all(&proj_scripts_dir)?;

        let proj_ssf2_match = haxe_gen::best_match_projectile_data(&proj.name, ctx.projectile_data);
        fs::write(
            proj_scripts_dir.join(format!("{}Script.hx", pascal)),
            haxe_gen::generate_projectile_script(owner_id, &entity_id, &proj_info.extra_states, proj_ssf2_match),
        )?;
        fs::write(
            proj_scripts_dir.join(format!("{}Script.hx.meta", pascal)),
            haxe_gen::script_meta(
                &format!("{}ProjectileScript", entity_id),
                &det_uuid(&format!("{}::{}::ProjectileScript::meta", owner_id, proj.name)),
                haxe_gen::ScriptMetaKind::ProjectileScript,
            ),
        )?;
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
        fs::write(
            proj_scripts_dir.join(format!("{}Stats.hx", pascal)),
            haxe_gen::generate_projectile_stats(owner_id, &entity_id, &proj_info, proj_ssf2_match),
        )?;
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
