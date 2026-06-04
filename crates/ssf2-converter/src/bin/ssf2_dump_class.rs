//! ssf2_dump_class — list a class's instance methods/traits (via the lossless
//! abc_codec) to find injection hooks (e.g. a per-frame enterFrame/update).
use ssf2_converter::abc_codec::{self, TraitKindData};
fn main() {
    let swf = std::env::args().nth(1).unwrap();
    let want = std::env::args().nth(2).unwrap_or_else(|| "Main".into());
    let data = std::fs::read(&swf).unwrap();
    let buf = swf::decompress_swf(&data[..]).unwrap();
    let parsed = swf::parse_swf(&buf).unwrap();
    let abc_bytes = parsed.tags.iter().find_map(|t| if let swf::Tag::DoAbc2(a)=t {Some(a.data.to_vec())} else {None}).unwrap();
    let abc = abc_codec::parse(&abc_bytes).unwrap();
    let ci = abc.find_class_by_name(&want).expect("class not found");
    let inst = &abc.instances[ci];
    println!("class {} (super {:?}) iinit=method#{}",
        abc.multiname_qualified(inst.name).or_else(||abc.multiname_local(inst.name)).unwrap_or_default(),
        abc.multiname_local(inst.super_name), inst.iinit);
    for t in &inst.traits {
        let nm = abc.multiname_local(t.name).unwrap_or_default();
        match &t.data {
            TraitKindData::Method{method,..}|TraitKindData::Getter{method,..}|TraitKindData::Setter{method,..} =>
                println!("  method {:<28} method#{}", nm, method),
            TraitKindData::Slot{type_name,..} =>
                println!("  slot   {:<28} type={:?}", nm, abc.multiname_local(*type_name)),
            _ => println!("  other  {}", nm),
        }
    }
}
