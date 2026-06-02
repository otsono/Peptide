//! swf_probe — confirm we can decompress/parse/rewrite SSF2.swf at the container
//! level (via the `swf` crate) and inspect its DoAbc2 tag(s). This gates the
//! whole runtime-bridge injection: container round-trip must work before we
//! touch ABC internals.

use std::collections::BTreeMap;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        "/Users/jimmy/Downloads/SSF2BetaMac_v1.4.0.1-standalone 2/SSF2.app/Contents/Resources/SSF2.swf".to_string()
    });
    let data = std::fs::read(&path).expect("read swf");
    let buf = swf::decompress_swf(&data[..]).expect("decompress");
    let parsed = swf::parse_swf(&buf).expect("parse");
    let h = &parsed.header;
    println!("version={} frames={} tags={}", h.version(), h.num_frames(), parsed.tags.len());

    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut abc_tags = 0usize;
    let mut abc_total = 0usize;
    for tag in &parsed.tags {
        // discriminant name (cheap kind tally)
        let dbg = format!("{:?}", tag);
        let kind = dbg.split(|c| c == '(' || c == ' ' || c == '{').next().unwrap_or("?").to_string();
        *kinds.entry(kind).or_insert(0) += 1;
        if let swf::Tag::DoAbc2(abc) = tag {
            abc_tags += 1; abc_total += abc.data.len();
            println!("  DoAbc2 name={:?} flags={:?} len={}",
                String::from_utf8_lossy(abc.name.as_bytes()), abc.flags, abc.data.len());
        }
        if let swf::Tag::DoAbc(d) = tag { abc_tags += 1; abc_total += d.len(); println!("  DoAbc(legacy) len={}", d.len()); }
    }
    println!("DoAbc tags={} abc_bytes={}", abc_tags, abc_total);

    // SymbolClass: maps exported symbol ids → AS3 class names. id 0 = the
    // document class (the SWF's entry point), which is where we hook the bridge.
    for tag in &parsed.tags {
        if let swf::Tag::SymbolClass(syms) = tag {
            println!("SymbolClass ({} entries):", syms.len());
            for s in syms.iter() {
                println!("  id={} class={:?}", s.id, String::from_utf8_lossy(s.class_name.as_bytes()));
            }
        }
    }
    for (k, n) in kinds.iter().filter(|(_, n)| **n > 0) {
        if *n >= 1 { /* print the interesting structural ones */ }
        if matches!(k.as_str(), "FileAttributes"|"SetBackgroundColor"|"SymbolClass"|"ShowFrame"|"DoAbc2"|"DefineSceneAndFrameLabelData"|"Metadata"|"EnableTelemetry"|"ScriptLimits"|"ProductInfo") {
            println!("  tag {k} x{n}");
        }
    }

    // Container round-trip: reconstruct Header from HeaderExt getters, rewrite,
    // re-decompress+parse, and compare tag count + ABC bytes.
    let header = swf::Header {
        compression: h.compression(),
        version: h.version(),
        stage_size: h.stage_size().clone(),
        frame_rate: h.frame_rate(),
        num_frames: h.num_frames(),
    };
    let mut out = Vec::new();
    swf::write_swf(&header, &parsed.tags, &mut out).expect("write_swf");
    println!("rewrote container: {} bytes (orig file {} bytes)", out.len(), data.len());
    std::fs::write("/tmp/ssf2_roundtrip.swf", &out).expect("write tmp");

    // Re-parse the rewritten SWF and compare.
    let buf2 = swf::decompress_swf(&out[..]).expect("re-decompress");
    let parsed2 = swf::parse_swf(&buf2).expect("re-parse");
    let abc2: Vec<&[u8]> = parsed2.tags.iter().filter_map(|t| if let swf::Tag::DoAbc2(a) = t { Some(&a.data[..]) } else { None }).collect();
    let abc1: Vec<&[u8]> = parsed.tags.iter().filter_map(|t| if let swf::Tag::DoAbc2(a) = t { Some(&a.data[..]) } else { None }).collect();
    println!("re-parsed tags={} (orig {})", parsed2.tags.len(), parsed.tags.len());
    let abc_equal = abc1.len() == abc2.len() && abc1.iter().zip(abc2.iter()).all(|(a, b)| a == b);
    println!("ABC bytes preserved across container round-trip: {}", abc_equal);
}
