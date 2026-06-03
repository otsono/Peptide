//! abc_strings — print ABC string-pool entries matching a substring (find ids).
use ssf2_converter::abc_codec;
fn main() {
    let swf = std::env::args().nth(1).unwrap();
    let needle = std::env::args().nth(2).unwrap_or_default().to_lowercase();
    let data = std::fs::read(&swf).unwrap();
    let buf = swf::decompress_swf(&data[..]).unwrap();
    let parsed = swf::parse_swf(&buf).unwrap();
    let abc_bytes = parsed.tags.iter().find_map(|t| if let swf::Tag::DoAbc2(a)=t {Some(a.data.to_vec())} else {None}).unwrap();
    let abc = abc_codec::parse(&abc_bytes).unwrap();
    for s in &abc.strings {
        if needle.is_empty() || s.to_lowercase().contains(&needle) { println!("{s}"); }
    }
}
