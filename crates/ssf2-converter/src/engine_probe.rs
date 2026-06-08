//! engine_probe — read parallax/camera parameters and class schema LIVE out of the SSF2
//! executable, so Peptide carries no copyrighted engine code: the engine specifics are pulled
//! from `SSF2.swf` at runtime. (The Fraymakers side is pulled from `hlboot` in the GUI via
//! hlbc.) What we pull: the logical view size (`Main.m_width`/`m_height`), the SWF frame rate,
//! the `VcamBGSettings` camera-background config field schema, and the `BOUNDS_MODE`/`PAN_MODE`
//! string constants. SSF2's `Vcam` auto-derives each layer's pan rate from the view size.

use crate::abc_codec::{self, Abc, TraitKindData};

/// Everything pulled live from `SSF2.swf` for the stage/parallax preview.
#[derive(Debug, Default, Clone)]
pub struct Ssf2Engine {
    /// Logical view (`Main.m_width` / `m_height`), e.g. 640x360.
    pub view_w: u32,
    pub view_h: u32,
    /// SWF frame rate (fps).
    pub fps: Option<f32>,
    /// `VcamBGSettings` mode constants: `(field, value)` e.g. `("BOUNDS_MODE","boundsMode")`.
    pub modes: Vec<(String, String)>,
    /// `VcamBGSettings` instance fields (the camera-background config schema).
    pub config_fields: Vec<String>,
}

/// Pull [`Ssf2Engine`] out of `SSF2.swf` bytes. `None` if the SWF/ABC can't be read or `Main`'s
/// view dimensions aren't found.
pub fn ssf2_engine(swf_bytes: &[u8]) -> Option<Ssf2Engine> {
    let inner = crate::ssf::decompress(swf_bytes).ok()?;
    let buf = swf::decompress_swf(&inner[..]).ok()?;
    let parsed = swf::parse_swf(&buf).ok()?;
    let fps = Some(parsed.header.frame_rate().to_f32());
    let abc_bytes = parsed.tags.iter().filter_map(|t| match t {
        swf::Tag::DoAbc2(a) => Some(a.data.to_vec()),
        swf::Tag::DoAbc(a) => Some(a.to_vec()),
        _ => None,
    }).max_by_key(|b| b.len())?;
    let abc = abc_codec::parse(&abc_bytes).ok()?;

    // Main.m_width / m_height
    let (mut view_w, mut view_h) = (None, None);
    if let Some(ci) = class_named(&abc, "Main") {
        if let Some(body) = body_for(&abc, abc.classes[ci].cinit) {
            for_each_set(&abc, &body.code, |name, c| match (name, c) {
                ("m_width", Const::Int(v)) => view_w = Some(*v as u32),
                ("m_height", Const::Int(v)) => view_h = Some(*v as u32),
                _ => {}
            });
        }
    }

    // VcamBGSettings: the camera-background config schema + the mode string constants.
    let (mut modes, mut config_fields) = (Vec::new(), Vec::new());
    if let Some(ci) = class_named(&abc, "com.mcleodgaming.ssf2.util.VcamBGSettings")
        .or_else(|| class_named(&abc, "VcamBGSettings"))
    {
        config_fields = slot_field_names(&abc, ci);
        if let Some(body) = body_for(&abc, abc.classes[ci].cinit) {
            for_each_set(&abc, &body.code, |name, c| {
                if let Const::Str(s) = c {
                    if name.ends_with("_MODE") { modes.push((name.to_string(), s.clone())); }
                }
            });
        }
    }

    Some(Ssf2Engine { view_w: view_w?, view_h: view_h?, fps, modes, config_fields })
}

/// Back-compat: just the view dims.
pub fn ssf2_view_dims(swf_bytes: &[u8]) -> Option<(u32, u32)> {
    ssf2_engine(swf_bytes).map(|e| (e.view_w, e.view_h))
}

fn class_named(abc: &Abc, name: &str) -> Option<usize> { abc.find_class_by_name(name) }
fn body_for(abc: &Abc, method: u32) -> Option<&abc_codec::MethodBody> { abc.bodies.iter().find(|b| b.method == method) }

/// Instance Slot/Const field names of a class (its data schema), in declaration order.
fn slot_field_names(abc: &Abc, class_idx: usize) -> Vec<String> {
    abc.instances.get(class_idx).map(|inst| inst.traits.iter().filter_map(|t| {
        matches!(t.data, TraitKindData::Slot { .. } | TraitKindData::Const { .. })
            .then(|| abc.multiname_local(t.name)).flatten()
    }).collect()).unwrap_or_default()
}

/// A constant pushed by a `push*` op.
enum Const { Int(i64), Str(String) }

/// Walk an AVM2 method body, calling `f(propertyName, const)` for each `push<const> N; …
/// setproperty <name>` (the constant most recently pushed when a `setproperty` fires). Enough
/// to read `Main`'s int assignments + `VcamBGSettings`' mode strings; other ops are skipped by
/// their operand size.
fn for_each_set(abc: &Abc, code: &[u8], mut f: impl FnMut(&str, &Const)) {
    let mut i = 0usize;
    let mut last: Option<Const> = None;
    while i < code.len() {
        let op = code[i]; i += 1;
        match op {
            0x24 | 0x25 => { last = Some(Const::Int(rd_u30(code, &mut i) as i64)); }   // pushbyte / pushshort
            0x2d => { let k = rd_u30(code, &mut i) as usize;                            // pushint -> int pool
                      last = Some(Const::Int(if k == 0 { 0 } else { abc.ints.get(k - 1).copied().unwrap_or(0) as i64 })); }
            0x2c => { let k = rd_u30(code, &mut i) as usize;                            // pushstring -> string pool
                      last = (k > 0).then(|| abc.strings.get(k - 1).cloned()).flatten().map(Const::Str); }
            0x61 | 0x68 => { let mn = rd_u30(code, &mut i);                             // setproperty / initproperty <multiname>
                      if let (Some(name), Some(v)) = (abc.multiname_local(mn), &last) { f(&name, v); } }
            // one-u30 ops (getlex/getproperty/findprop/coerce/inc/dec/…)
            0x60 | 0x5c | 0x5d | 0x66 | 0x80 | 0x62 | 0x63 | 0x65
            | 0x6c | 0x6d | 0x2e | 0x2f => { let _ = rd_u30(code, &mut i); }
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
    // Reads the real SSF2.swf and checks the live extraction. Gated on
    // `PEPTIDE_SSF2_SWF=<path to SSF2.swf>` so it skips cleanly without the executable.
    #[test]
    fn ssf2_engine_extracts_live() {
        let Ok(p) = std::env::var("PEPTIDE_SSF2_SWF") else { return };
        if !std::path::Path::new(&p).exists() { return; }
        let e = super::ssf2_engine(&std::fs::read(&p).unwrap()).expect("ssf2 engine");
        assert_eq!((e.view_w, e.view_h), (640, 360), "Main view");
        let fps = e.fps.unwrap_or(0.0);
        assert!((1.0..=120.0).contains(&fps), "fps in a sane range (fixed-point decode), got {fps}");
        assert!(e.config_fields.iter().any(|f| f == "xPanMultiplier"), "config schema, got {:?}", e.config_fields);
        assert!(e.modes.iter().any(|(k, _)| k == "PAN_MODE"), "mode constants, got {:?}", e.modes);
    }
}
