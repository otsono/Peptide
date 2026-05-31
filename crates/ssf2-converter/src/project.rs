//! Project-level data types shared between `main.rs` (the orchestrator)
//! and `haxe_gen.rs` (the file writer). Lives in the lib so both sides
//! of the crate boundary can name the types.
//!
//! Used by the multi-character emission path described in
//! `docs/multi_character_projects_plan.md` §2.

use std::path::PathBuf;

/// Project-level slot information passed to `process_character` when
/// the SSF emits multiple characters into one shared `.fraytools`
/// project. `None` in the relevant signatures means "single-character
/// project, emit the standalone layout."
pub struct MultiCharSlot {
    /// Shared project output dir (e.g. `<output>/zelda/`).
    pub project_dir: PathBuf,
    /// 0-based index into the SSF's constructor-walk roster.
    pub slot_idx: usize,
    /// PascalCase forms for every character in the project, in
    /// constructor-walk order. Used to name `<Pascal>_Menu.entity`
    /// and to derive the collision-suffix index.
    pub pascals: Vec<String>,
    /// Lowercase character ids for every character in the project,
    /// in constructor-walk order. Parallel to `pascals`.
    pub char_ids: Vec<String>,
}

impl MultiCharSlot {
    /// Numeric suffix appended to file paths that collide between
    /// characters at a shared library path. Slot 0 → no suffix; slot N
    /// (N > 0) → return `Some(N + 1)` per the plan's rule
    /// (`costumes.palettes` + `costumes.palettes2` + …).
    pub fn collision_suffix(&self) -> Option<usize> {
        if self.slot_idx == 0 { None } else { Some(self.slot_idx + 1) }
    }
}

/// One character's contribution to the project-level manifest's
/// `content[]` array. Built by `process_character` and consumed by
/// `finalize_multi_char_project` / `haxe_gen::generate_multi_char_manifest`.
pub struct ManifestCharEntry {
    pub char_id:          String,
    pub display_name:     String,
    pub projectile_names: Vec<String>,
    /// Menu entity id under which this character's portrait/HUD
    /// entries live in the project. For multi-char, this is the
    /// character-qualified id (e.g. `zelda_menu`); for single-char,
    /// it's just `menu`.
    pub menu_entity_id:   String,
}

/// Per-character artifacts returned by `process_character` so the
/// project-finalizer can assemble the merged manifest + conversion log
/// after every character has been processed. Only populated when in
/// multi-char mode; single-char mode finalizes everything inline.
pub struct ProcessedCharacter {
    pub manifest_entry: ManifestCharEntry,
    pub log_block:      serde_json::Value,
}
