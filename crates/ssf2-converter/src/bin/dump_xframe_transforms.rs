/// Diagnostic: dump the root timeline's per-xframe stance placement matrix
/// (the XframeTransform the image pipeline composes into every animation).
/// Surfaces per-anim placement outliers (e.g. an xframe authored with a big
/// translate that shifts its art off the character origin).
use std::io::Cursor;

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_xframe_transforms <file.ssf> [char]");
    let ch = std::env::args().nth(2).unwrap_or_else(|| {
        std::path::Path::new(&path).file_stem().unwrap().to_string_lossy().to_string()
    });
    let data = std::fs::read(&path).expect("read file");
    let swf_buf = swf::decompress_swf(Cursor::new(&data)).expect("decompress");
    let swf = swf::parse_swf(&swf_buf).expect("parse");
    let map = ssf2_converter::sprite_parser::extract_xframe_transforms_from_swf(
        &swf, &ch, &Default::default()).expect("extract");
    for (k, v) in &map {
        println!("{k:24} tx={:8.2} ty={:8.2} sx={:.3} sy={:.3}", v.tx, v.ty, v.sx, v.sy);
    }
}
