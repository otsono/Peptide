//! engine_probe — read parallax/camera parameters LIVE out of the game executables,
//! so the stage preview is driven by the real engines instead of the converter's ported
//! constants. SSF2's logical view size (`Main.m_width`/`m_height`, 640x360) is an engine
//! constant baked into `SSF2.swf`; its `Vcam` auto-derives each camera background's pan rate
//! from that view. (Fraymakers reads the view from each stage's `GameCameraConfig` and has no
//! auto-pan, so there's no engine-side constant to pull there; see the GUI side.)

use crate::abc_codec::{self, Abc};

/// SSF2's logical view dimensions `(width, height)`, read out of `SSF2.swf`'s `Main` static
/// init (`pushshort 640 … setproperty m_width`). `None` if the SWF/ABC can't be read.
pub fn ssf2_view_dims(swf_bytes: &[u8]) -> Option<(u32, u32)> {
    let inner = crate::ssf::decompress(swf_bytes).ok()?;
    let buf = swf::decompress_swf(&inner[..]).ok()?;
    let parsed = swf::parse_swf(&buf).ok()?;
    // SSF2 ships several DoAbc tags; the engine (with the `Main` class) is the largest.
    let abc_bytes = parsed.tags.iter().filter_map(|t| match t {
        swf::Tag::DoAbc2(a) => Some(a.data.to_vec()),
        swf::Tag::DoAbc(a) => Some(a.to_vec()),
        _ => None,
    }).max_by_key(|b| b.len())?;
    let abc = abc_codec::parse(&abc_bytes).ok()?;
    let ci = abc.find_class_by_name("Main")
        .or_else(|| abc.find_class_by_name("com.mcleodgaming.ssf2.Main"))?;
    let cinit = abc.classes[ci].cinit;
    let body = abc.bodies.iter().find(|b| b.method == cinit)?;
    let (mut w, mut h) = (None, None);
    for_each_set_const(&abc, &body.code, |name, val| {
        match name {
            "m_width" => w = Some(val as u32),
            "m_height" => h = Some(val as u32),
            _ => {}
        }
    });
    match (w, h) { (Some(w), Some(h)) => Some((w, h)), _ => None }
}

/// Walk an AVM2 method body, calling `f(propertyName, value)` for each
/// `push<const> N; … setproperty <name>` — i.e. the constant most recently pushed when a
/// `setproperty` fires. Enough to read the small `Main` static-init assignments; unknown ops
/// are skipped by their operand size.
fn for_each_set_const(abc: &Abc, code: &[u8], mut f: impl FnMut(&str, i64)) {
    let mut i = 0usize;
    let mut last: Option<i64> = None;
    while i < code.len() {
        let op = code[i]; i += 1;
        match op {
            0x24 => { last = Some(rd_u30(code, &mut i) as i64); }            // pushbyte
            0x25 => { last = Some(rd_u30(code, &mut i) as i64); }            // pushshort
            0x2d => { // pushint -> int pool (index 0 is implicit 0; stored pool is [1..])
                let k = rd_u30(code, &mut i) as usize;
                last = if k == 0 { Some(0) } else { abc.ints.get(k - 1).map(|&v| v as i64) };
            }
            0x61 => { // setproperty <multiname>
                let mn = rd_u30(code, &mut i);
                if let (Some(name), Some(v)) = (abc.multiname_local(mn), last) { f(&name, v); }
            }
            // one-u30 ops (getlex/getproperty/findprop/coerce/inc/dec/getlocal-wide/…)
            0x60 | 0x5c | 0x5d | 0x66 | 0x68 | 0x80 | 0x62 | 0x63 | 0x65
            | 0x6c | 0x6d | 0x2c | 0x2e | 0x2f => { let _ = rd_u30(code, &mut i); }
            // call ops: two u30 (multiname, argc)
            0x4a | 0x46 | 0x4f | 0x4c | 0x45 | 0x4e => { let _ = rd_u30(code, &mut i); let _ = rd_u30(code, &mut i); }
            0x10..=0x1a => { i += 3; }   // jumps: s24
            0xef => { i += 8; }          // debug
            _ => {}
        }
    }
}

/// Read an AVM2 u30 (LEB128-ish, up to 5 bytes) at `*i`, advancing `*i`.
fn rd_u30(code: &[u8], i: &mut usize) -> u32 {
    let mut v = 0u32;
    for s in 0..5 {
        if *i >= code.len() { break; }
        let b = code[*i]; *i += 1;
        v |= ((b & 0x7f) as u32) << (7 * s);
        if b & 0x80 == 0 { break; }
    }
    v
}

#[cfg(test)]
mod tests {
    // Reads the real SSF2.swf and checks the live extraction is 640x360. Gated on
    // `PEPTIDE_SSF2_SWF=<path to SSF2.swf>` so it skips cleanly without the executable.
    #[test]
    fn ssf2_view_is_640x360() {
        let Ok(p) = std::env::var("PEPTIDE_SSF2_SWF") else { return };
        if !std::path::Path::new(&p).exists() { return; }
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(super::ssf2_view_dims(&bytes), Some((640, 360)), "extracted SSF2 Main view dims");
    }
}
