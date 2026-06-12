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

/// Decompress `SSF2.swf` and parse its largest ABC block. Returns the SWF frame rate and the
/// parsed ABC. `None` if the SWF/ABC can't be read. Shared by [`ssf2_engine`] and [`ssf2_doctor`].
fn load_swf_abc(swf_bytes: &[u8]) -> Option<(f32, Abc)> {
    let inner = crate::ssf::decompress(swf_bytes).ok()?;
    let buf = swf::decompress_swf(&inner[..]).ok()?;
    let parsed = swf::parse_swf(&buf).ok()?;
    let fps = parsed.header.frame_rate().to_f32();
    let abc_bytes = parsed.tags.iter().filter_map(|t| match t {
        swf::Tag::DoAbc2(a) => Some(a.data.to_vec()),
        swf::Tag::DoAbc(a) => Some(a.to_vec()),
        _ => None,
    }).max_by_key(|b| b.len())?;
    let abc = abc_codec::parse(&abc_bytes).ok()?;
    Some((fps, abc))
}

/// Read `Main`'s logical view size out of its class initializer (`m_width` / `m_height`).
fn main_view_dims(abc: &Abc) -> (Option<i64>, Option<i64>) {
    let (mut w, mut h) = (None, None);
    if let Some(ci) = class_named(abc, "Main") {
        if let Some(body) = body_for(abc, abc.classes[ci].cinit) {
            for_each_set(abc, &body.code, |name, c| match (name, c) {
                ("m_width", Const::Int(v)) => w = Some(*v),
                ("m_height", Const::Int(v)) => h = Some(*v),
                _ => {}
            });
        }
    }
    (w, h)
}

/// The `VcamBGSettings` class index (camera-background / parallax config), under either its
/// fully-qualified or bare name.
fn vcam_settings(abc: &Abc) -> Option<usize> {
    class_named(abc, "com.mcleodgaming.ssf2.util.VcamBGSettings")
        .or_else(|| class_named(abc, "VcamBGSettings"))
}

/// Pull [`Ssf2Engine`] out of `SSF2.swf` bytes. `None` if the SWF/ABC can't be read or `Main`'s
/// view dimensions aren't found.
pub fn ssf2_engine(swf_bytes: &[u8]) -> Option<Ssf2Engine> {
    let (fps, abc) = load_swf_abc(swf_bytes)?;
    let fps = Some(fps);

    // Main.m_width / m_height
    let (view_w, view_h) = main_view_dims(&abc);
    let (view_w, view_h) = (view_w.map(|v| v as u32), view_h.map(|v| v as u32));

    // VcamBGSettings: the camera-background config schema + the mode string constants.
    let (mut modes, mut config_fields) = (Vec::new(), Vec::new());
    if let Some(ci) = vcam_settings(&abc) {
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

/// One resolved/missing SSF2 engine symbol, for `peptide ssf2 doctor`.
pub struct EngineCheck {
    /// Subsystem grouping (e.g. `view`, `parallax`).
    pub group: &'static str,
    /// Human label of the symbol.
    pub label: &'static str,
    /// Why Peptide depends on it (shown when missing).
    pub why: &'static str,
    /// `true` -> the converter can't work without it; a miss is fatal.
    pub critical: bool,
    /// Whether it resolved against this SSF2 build.
    pub ok: bool,
}

/// Resolve, BY NAME, every `SSF2.swf` symbol Peptide's stage/parallax path depends on, and
/// report per-symbol pass/fail — the SSF2 analogue of the Fraymakers `doctor`. A recompiled SSF2
/// renumbers every class/trait, so a name lookup that still resolves means the build is
/// compatible. `None` only if the SWF/ABC can't be read at all.
pub fn ssf2_doctor(swf_bytes: &[u8]) -> Option<Vec<EngineCheck>> {
    let (fps, abc) = load_swf_abc(swf_bytes)?;
    let mk = |group, label, why, critical, ok| EngineCheck { group, label, why, critical, ok };
    let mut checks = Vec::new();

    // view: Main + its view dims drive the stage scale and the parallax pan formula.
    let main = class_named(&abc, "Main");
    checks.push(mk("view", "Main", "owns the logical view size", true, main.is_some()));
    let (w, h) = main_view_dims(&abc);
    checks.push(mk("view", "Main.m_width", "view width drives the pan-rate formula", true, w.is_some()));
    checks.push(mk("view", "Main.m_height", "view height drives the vertical pan formula", true, h.is_some()));
    checks.push(mk("view", "SWF frame rate", "engine tick rate (physics + scaling)", false, fps > 0.0));

    // parallax: VcamBGSettings backs the camera-background preview (rare: ~1/110 stages).
    let vcam = vcam_settings(&abc);
    checks.push(mk("parallax", "VcamBGSettings", "camera-background (parallax) config class", false, vcam.is_some()));
    let fields = vcam.map(|ci| slot_field_names(&abc, ci)).unwrap_or_default();
    checks.push(mk("parallax", "VcamBGSettings.xPanMultiplier", "per-layer pan-rate field", false,
        fields.iter().any(|f| f == "xPanMultiplier")));
    let mut has_mode = false;
    if let Some(ci) = vcam {
        if let Some(body) = body_for(&abc, abc.classes[ci].cinit) {
            for_each_set(&abc, &body.code, |name, c| {
                if matches!(c, Const::Str(_)) && name.ends_with("_MODE") { has_mode = true; }
            });
        }
    }
    checks.push(mk("parallax", "VcamBGSettings *_MODE consts", "PAN/BOUNDS mode string constants", false, has_mode));

    Some(checks)
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

    #[test]
    fn ssf2_doctor_resolves_live() {
        let Ok(p) = std::env::var("PEPTIDE_SSF2_SWF") else { return };
        if !std::path::Path::new(&p).exists() { return; }
        let checks = super::ssf2_doctor(&std::fs::read(&p).unwrap()).expect("ssf2 doctor");
        let crit_miss: Vec<_> = checks.iter().filter(|c| c.critical && !c.ok).map(|c| c.label).collect();
        assert!(crit_miss.is_empty(), "critical SSF2 symbols missing: {crit_miss:?}");
        // every check resolves on a stock SSF2 build
        let miss: Vec<_> = checks.iter().filter(|c| !c.ok).map(|c| c.label).collect();
        assert!(miss.is_empty(), "unresolved SSF2 symbols: {miss:?}");
    }
}
