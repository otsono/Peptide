// Clippy baseline for this existing codebase: CI runs `cargo clippy -- -D warnings`,
// so anything not allowed here is a hard error. these families are pedantic-style or
// pre-existing patterns we accept; new code should still try to avoid them.
#![allow(
    clippy::doc_overindented_list_items,
    clippy::doc_lazy_continuation,
    clippy::empty_line_after_doc_comments,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::manual_checked_ops,
    clippy::unnecessary_unwrap,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::field_reassign_with_default,
    clippy::collapsible_match,
)]

/// a regular expression compiled once per call site: the converter runs its rewrites over
/// hundreds of scripts, and compiling the same pattern for each was most of a conversion
#[macro_export]
macro_rules! re {
    ($p:expr) => {{
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| regex::Regex::new($p).expect("regex"))
    }};
}

/// A regex whose pattern is built at run time (a name escaped into it, an alternation of a
/// fighter's own methods), compiled once per distinct pattern for the life of the process. The
/// passes that build these run once per frame script, and a fighter with hundreds of methods and
/// hundreds of frames recompiled the same large alternation hundreds of times: kirby's conversion
/// spent most of two minutes in the regex compiler.
pub fn dyn_re(pattern: &str) -> std::sync::Arc<regex::Regex> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<regex::Regex>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(r) = cache.lock().expect("regex cache").get(pattern) { return r.clone(); }
    let compiled = Arc::new(regex::Regex::new(pattern).unwrap_or_else(|e| panic!("regex {pattern:?}: {e}")));
    let mut guard = cache.lock().expect("regex cache");
    // bounded: a long batch run converts many packages in one process
    if guard.len() > 4096 { guard.clear(); }
    guard.insert(pattern.to_string(), compiled.clone());
    compiled
}

// Public modules — exposed for use by binaries in src/bin/
pub mod ssf2_manifest;
pub mod ssf;
pub mod test_fixture;
pub mod uuid_gen;
pub use swf;
pub mod swf_parser;
pub mod swf_timeline;
pub mod swf_filter;
pub mod entity_compact;
pub mod ssf2_objects;
pub mod abc_ruffle;
pub mod abc_parser;
pub mod decompiler;
pub mod disasm;
pub mod model;
pub mod compile;
pub mod emit;
pub mod extractor;
pub mod anim_splitter;
pub mod entity_gen;
pub mod fraytools_project;
pub mod haxe_gen;
pub mod projectile_gen;
pub mod palette_gen;
pub mod sprite_parser;
pub mod image_extractor;
pub mod flatten;
pub mod sound_extractor;
pub mod api_mappings;
pub mod mappings;
pub mod project;
pub mod fraytools_transform;
pub mod fm_placement;
pub mod vector_raster;
pub mod physics_sim;
pub mod abc_codec;
pub mod engine_probe;
pub mod abc_inject;

// In-process conversion entry point (was the `ssf2_converter` binary's main()).
pub mod convert;
pub use convert::{run_conversion, ConversionSummary, ConvertOptions};

// Asset classification (identify which `.ssf` is a character / stage / other).
pub mod classify;
pub use classify::{classify_ssf, AssetKind, SsfClassification};

// SSF2 stage AS3 reader (plane assignments + spawned actors) + geometry parser + emitter.
pub mod shared_packages;
pub mod stage_abc;
pub mod stage_parser;
pub use stage_parser::{parse_stage, parse_stage_opts, ParallaxLayer, ParallaxMode, Platform, Rect, SpawnPoint, StageArt, StageArtSet, StageModel};
pub mod stage_emit;
pub mod content_audit;
pub use stage_emit::{emit_stage, render_placeholder_for_test};
