// Public modules — exposed for use by binaries in src/bin/
pub mod ssf;
pub mod uuid_gen;
pub mod swf_parser;
pub mod abc_parser;
pub mod decompiler;
pub mod extractor;
pub mod anim_splitter;
pub mod entity_gen;
pub mod fraytools_project;
pub mod haxe_gen;
pub mod palette_gen;
pub mod sprite_parser;
pub mod image_extractor;
pub mod sound_extractor;
pub mod api_mappings;
pub mod mappings;
pub mod project;
pub mod fraytools_transform;
pub mod vector_raster;
pub mod physics_sim;
pub mod abc_codec;
pub mod abc_inject;

// In-process conversion entry point (was the `ssf2_converter` binary's main()).
pub mod convert;
pub use convert::{run_conversion, ConversionSummary, ConvertOptions};
