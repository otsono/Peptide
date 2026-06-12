//! diff_abc — diff every method body (code + exception table + frame) between two
//! patched SWFs, to pinpoint a bytecode regression. Usage: diff_abc <a.swf> <b.swf>
use ssf2_converter::abc_codec::{self, Abc};
fn load(p: &str) -> Abc {
    let d = std::fs::read(p).unwrap();
    let b = swf::decompress_swf(&d[..]).unwrap();
    let parsed = swf::parse_swf(&b).unwrap();
    let bytes = parsed.tags.iter().find_map(|t| if let swf::Tag::DoAbc2(a) = t { Some(a.data.to_vec()) } else { None }).unwrap();
    abc_codec::parse(&bytes).unwrap()
}
fn exc_key(b: &abc_codec::MethodBody) -> String {
    format!("{:?}", b.exceptions.iter().map(|e| (e.from, e.to, e.target, e.exc_type, e.var_name)).collect::<Vec<_>>())
}
fn main() {
    let a = load(&std::env::args().nth(1).unwrap());
    let b = load(&std::env::args().nth(2).unwrap());
    println!("a: methods={} bodies={} strings={} multinames={}", a.methods.len(), a.bodies.len(), a.strings.len(), a.multinames.len());
    println!("b: methods={} bodies={} strings={} multinames={}", b.methods.len(), b.bodies.len(), b.strings.len(), b.multinames.len());
    let mut diffs = 0;
    for ba in &a.bodies {
        let Some(bb) = b.bodies.iter().find(|x| x.method == ba.method) else {
            println!("ONLY-IN-A method#{}", ba.method); diffs += 1; continue;
        };
        let code = ba.code != bb.code;
        let exc = exc_key(ba) != exc_key(bb);
        let frame = ba.max_stack != bb.max_stack || ba.max_scope_depth != bb.max_scope_depth || ba.local_count != bb.local_count;
        if code || exc || frame {
            let nm = a.methods.get(ba.method as usize).and_then(|m| a.multiname_local(m.name)).unwrap_or_default();
            println!("DIFF method#{} ({nm}) code={code} exc={exc} frame={frame}  alen={} blen={}  a_exc={} b_exc={}",
                ba.method, ba.code.len(), bb.code.len(), exc_key(ba), exc_key(bb));
            diffs += 1;
        }
    }
    println!("total differing bodies: {diffs}");
}
