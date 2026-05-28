use ssf2_converter::*;
use std::path::Path;

fn main() {
    env_logger::init();
    
    let raw = std::fs::read("/Users/jimmy/.openclaw/workspace-main/ssf2-ssfs/sandbag.ssf").unwrap();
    let swf_data = ssf2_converter::ssf::decompress(&raw).unwrap();
    let swf = swf_parser::parse(&swf_data).unwrap();
    let char_data = extractor::extract(&swf, "sandbag").unwrap();
    
    let temp_dir = std::env::temp_dir();
    let img_result = image_extractor::extract_images(&swf_data, Path::new(&temp_dir), "sandbag", &char_data.ssf2_to_fm_anim).unwrap();
    
    println!("=== aerial_down after pre-rendering ===");
    if let Some(anim_data) = img_result.anim_images.get("aerial_down") {
        for f in 0..7u16 {
            if let Some(entries) = anim_data.frames.get(&f) {
                for (i, e) in entries.iter().enumerate() {
                    let has_skew = e.local_matrix.has_skew();
                    println!("Frame {} [{}]: sym={:<30} local=({:.2},{:.2}) world=({:.2},{:.2}) rot={:.1}° sx={:.3} sy={:.3} skew={}",
                        f, i, e.symbol_name,
                        e.local_matrix.tx, e.local_matrix.ty,
                        e.world_tx, e.world_ty,
                        e.world_rotation, e.world_sx, e.world_sy, has_skew);
                }
            }
        }
    }
}
