//! classify — identify what an SSF2 `.ssf` resource IS without fully converting it.
//!
//! SSF2 ships content as `DATn.ssf` archives (characters, stages, items, ui, …) with
//! no explicit "type" field; each only self-declares an `id`/`guid` in its `Main`
//! constructor. To port stages we first need to find WHICH `.ssf` files are stages.
//! This decompresses + parses an `.ssf` and classifies it:
//!
//!   * Character — `Main` declares a non-empty `register("characters", […])` (or has
//!     `get*` getters): the same signal [`crate::convert::run_conversion`] keys off.
//!   * Stage — no character, but the SWF carries the SSF2 stage boundary instances
//!     (`deathBoundary` / `camBoundary` / `smashBallBoundary`). these are the named
//!     MovieClip instances every SSF2 stage defines (SSF2 dev-wiki, "Stage Structure").
//!   * Other — neither (items, ui, fx, audio packs, …).
//!
//! Read-only on the input file. The `peptide ssf2 identify` CLI is the front end.

use anyhow::{Context, Result};
use std::path::Path;

use crate::{abc_parser, ssf, swf_parser};

/// What kind of asset an `.ssf` holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetKind {
    /// A convertible character package; carries the declared character ids.
    Character(Vec<String>),
    /// An SSF2 stage (has the boundary instances).
    Stage,
    /// Anything else (item, ui, fx, audio, …).
    Other,
}

impl AssetKind {
    pub fn label(&self) -> &'static str {
        match self {
            AssetKind::Character(_) => "character",
            AssetKind::Stage => "stage",
            AssetKind::Other => "other",
        }
    }
}

/// The result of classifying one `.ssf`.
#[derive(Debug, Clone)]
pub struct SsfClassification {
    /// `Main.id` (the content id the engine knows this resource by), if declared.
    pub id: Option<String>,
    /// `Main.guid`, if declared.
    pub guid: Option<String>,
    pub kind: AssetKind,
    /// The stage boundary marker instance names found (for `stage`/diagnostics).
    pub markers: Vec<String>,
}

/// The SSF2 stage boundary instances. Every stage defines these named MovieClips
/// (death/camera/item bounds). We match the `boundary` substring case-insensitively
/// (covers `deathBoundary`/`camBoundary`/`smashBallBoundary` and version variants),
/// scanning nested sprites too, since the boundaries live inside a stage MovieClip
/// rather than on the SWF root.
const STAGE_MARKER_SUBSTR: &str = "boundary";

/// Classify the `.ssf` at `path` (decompress + parse; read-only).
pub fn classify_ssf(path: &Path) -> Result<SsfClassification> {
    let raw = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let swf_data = ssf::decompress(&raw).with_context(|| format!("decompress {}", path.display()))?;
    let swf = swf_parser::parse(&swf_data).with_context(|| format!("parse SWF {}", path.display()))?;

    // id/guid + character ids from any ABC block's Main constructor.
    let mut id = None;
    let mut guid = None;
    let mut characters: Vec<String> = Vec::new();
    for abc_bytes in &swf.abc_blocks {
        let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };
        if let Some(md) = abc_parser::extract_main_package_metadata(&abc) {
            if id.is_none() { id = md.id.clone(); }
            if guid.is_none() { guid = md.guid.clone(); }
            for (cid, _m) in &md.characters {
                if !characters.contains(cid) { characters.push(cid.clone()); }
            }
        }
    }

    // stage markers from the SWF symbol/instance names (a character package never
    // defines the stage boundary instances).
    let markers = stage_markers(&swf_data);

    let kind = if !characters.is_empty() {
        AssetKind::Character(characters)
    } else if !markers.is_empty() {
        AssetKind::Stage
    } else {
        AssetKind::Other
    };

    Ok(SsfClassification { id, guid, kind, markers })
}

/// Scan the SWF tag tree (recursing into `DefineSprite`) for stage boundary instance
/// names (the `boundary` substring, case-insensitive). Returns the distinct names found.
fn stage_markers(swf_data: &[u8]) -> Vec<String> {
    let Ok(buf) = swf::decompress_swf(swf_data) else { return Vec::new() };
    let Ok(parsed) = swf::parse_swf(&buf) else { return Vec::new() };
    let mut found: Vec<String> = Vec::new();
    scan_tags(&parsed.tags, &mut found);
    found
}

/// Recurse the tag tree collecting `PlaceObject` instance + `SymbolClass` names that
/// contain the boundary marker substring. `DefineSprite` carries its own nested tags
/// (the stage's boundary clips live there, not on the root).
fn scan_tags(tags: &[swf::Tag], found: &mut Vec<String>) {
    fn note(name: String, found: &mut Vec<String>) {
        if name.to_ascii_lowercase().contains(STAGE_MARKER_SUBSTR) && !found.contains(&name) {
            found.push(name);
        }
    }
    for tag in tags {
        match tag {
            swf::Tag::PlaceObject(po) => {
                if let Some(n) = &po.name { note(n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string(), found); }
            }
            swf::Tag::SymbolClass(links) => {
                for link in links { note(link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string(), found); }
            }
            swf::Tag::DefineSprite(sprite) => scan_tags(&sprite.tags, found),
            _ => {}
        }
    }
}
