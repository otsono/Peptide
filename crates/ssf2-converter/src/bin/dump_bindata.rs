//! dump_bindata — extract a DefineBinaryData tag's bytes by symbol id (from SymbolClass).
use std::collections::HashMap;
fn main() {
    let swf = std::env::args().nth(1).unwrap();
    let want = std::env::args().nth(2).unwrap_or_else(|| "manifest".into()).to_lowercase();
    let data = std::fs::read(&swf).unwrap();
    let buf = swf::decompress_swf(&data[..]).unwrap();
    let parsed = swf::parse_swf(&buf).unwrap();
    // map symbol id -> class name
    let mut names: HashMap<u16,String> = HashMap::new();
    for t in &parsed.tags { if let swf::Tag::SymbolClass(syms)=t { for s in syms.iter() { names.insert(s.id, String::from_utf8_lossy(s.class_name.as_bytes()).into_owned()); } } }
    for t in &parsed.tags {
        if let swf::Tag::DefineBinaryData(b) = t {
            let nm = names.get(&b.id).cloned().unwrap_or_default();
            if nm.to_lowercase().contains(&want) || want.is_empty() {
                eprintln!("# symbol {} id={} len={}", nm, b.id, b.data.len());
                std::io::Write::write_all(&mut std::io::stdout(), b.data).unwrap();
                return;
            }
        }
    }
    eprintln!("no DefineBinaryData symbol matching {want:?}");
}
