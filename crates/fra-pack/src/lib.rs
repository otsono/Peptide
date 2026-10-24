//! The package the engine loads, built from a project's library held in memory: what
//! FrayTools' "Publish All" produces. The library comes from the files on disk (`peptide
//! pack`) or from what FrayTools hands a publish plugin (the WebAssembly build), and the same
//! code turns either into the same bytes.
//!
//! The package is one file: a big-endian length, a JSON header, then the binary blobs the
//! header points at (spritesheets, audio) by offset from the end of the header. Scripts,
//! the manifest and the palettes ride in the header as text; entities ride in it in a
//! compact form of the editor's own `.entity` (symbols inlined into their keyframes as a
//! number array, images replaced by an index into a packed spritesheet).
//!
//! Decoding the pictures and encoding the sheets is most of the work, and every picture and
//! every sheet is its own job: they run on rayon's pool (all cores natively, the worker pool
//! in the browser). Results are collected in order, so the bytes do not depend on the threads.

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;
use rayon::prelude::*;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

const PACKAGE_VERSION: &str = "0.0.17";
const FM_META: &str = "com.fraymakers.FraymakersMetadata";
/// the largest texture the engine takes; a sheet grows to the next power of two below it
const SHEET_MAX: u32 = 4096;
const GAP: u32 = 1;
/// deflate's effort on the sheets: measured on the largest fighter and stage, 3 is nearly as fast
/// as 1 and gives a stage's sheets a third less room, where 6 doubles a stage's time
const PNG_LEVEL: u32 = 3;
/// rows a sheet is compressed in at a time: 64 bands to a full sheet, small enough to spread
/// over any pool and large enough that the joins cost nothing
const PNG_BAND_ROWS: usize = 64;

/// A project's library, in memory. Lists are in library order (paths sorted), which is the
/// order the package keeps.
#[derive(Default)]
pub struct Library {
    /// each picture's file bytes, by the guid its `.meta` gives it
    pub images: HashMap<String, Vec<u8>>,
    /// every `.entity`, as the editor writes it
    pub entities: Vec<Value>,
    /// every script file and its `.meta`
    pub scripts: Vec<Script>,
    /// `manifest.json` and its `.meta`
    pub manifest: Value,
    pub manifest_meta: Value,
    /// every `.palettes` file and its `.meta` (an empty object when it has none)
    pub palettes: Vec<(Value, Value)>,
    /// every sound file
    pub audio: Vec<Audio>,
}

pub struct Script { pub meta: Value, pub text: String }
pub struct Audio { pub meta: Value, pub format: String, pub bytes: Vec<u8> }

pub struct Built {
    pub name: String,
    pub bytes: Vec<u8>,
    pub entities: usize,
    pub scripts: usize,
    pub audio: usize,
    pub sheets: usize,
}

/// the engine-facing plugin metadata of a library object, or an empty object
fn fm_meta(v: &Value) -> Value {
    v.get("pluginMetadata").and_then(|p| p.get(FM_META)).cloned().unwrap_or_else(|| json!({}))
}

fn exported(meta: &Value) -> bool {
    meta.get("export").and_then(Value::as_bool).unwrap_or(true)
}

fn group_of(e: &Value) -> String {
    fm_meta(e).get("spritesheetGroup").and_then(Value::as_str).unwrap_or("default").to_string()
}

fn num(v: &Value, k: &str, default: f64) -> f64 { v.get(k).and_then(Value::as_f64).unwrap_or(default) }
fn num_or_null(v: &Value, k: &str) -> Value {
    match v.get(k) { Some(Value::Number(n)) => Value::Number(n.clone()), _ => Value::Null }
}

/// one image's place: which sheet of its group, its frame index there, and the trim it got
struct Placed { sheet: usize, frame: usize, trim_x: f64, trim_y: f64 }

/// How the work is spread. Pictures and sheets always go on the pool: few, large allocations.
/// The JSON steps are millions of small ones, which go on the pool only where the allocator
/// lets threads allocate side by side; in the browser one lock serialises them and the threads
/// make those steps many times slower than one thread.
#[derive(Clone, Copy)]
pub struct Options { pub parallel_json: bool }

impl Default for Options { fn default() -> Self { Options { parallel_json: true } } }

pub fn build(lib: &Library) -> Result<Built> {
    build_timed(lib, Options::default(), &|| 0.0).map(|(b, _)| b)
}

/// `build`, with how long each step took by `clock` (milliseconds): the browser build has no
/// clock of its own, so the caller lends one
pub fn build_timed(lib: &Library, opts: Options, clock: &dyn Fn() -> f64) -> Result<(Built, Vec<(&'static str, f64)>)> {
    let mut steps: Vec<(&'static str, f64)> = Vec::new();
    let mut last = clock();
    let mut step = |name: &'static str| { let now = clock(); steps.push((name, now - last)); last = now; };
    let entities: Vec<&Value> = lib.entities.iter()
        .filter(|e| e.get("export").and_then(Value::as_bool) != Some(false)).collect();

    // the sheet groups each image is drawn in, across every entity
    let mut usage: BTreeMap<String, BTreeMap<String, ()>> = BTreeMap::new();
    for e in &entities {
        let group = group_of(e);
        for s in e["symbols"].as_array().into_iter().flatten() {
            if s["type"] == "IMAGE" {
                // an image symbol with no asset still gets a frame: a blank picture, as the
                // editor's publish gives it
                let g = s["imageAsset"].as_str().unwrap_or("");
                usage.entry(g.to_string()).or_default().insert(group.clone(), ());
            }
        }
    }

    // ---- every picture decoded and trimmed to its opaque bounds, side by side ----
    let used: Vec<(&String, &BTreeMap<String, ()>)> = usage.iter().collect();
    let decoded: Vec<(RgbaImage, u32, u32, u64)> = used.par_iter().map(|(guid, _)| -> Result<_> {
        let full = if guid.is_empty() {
            RgbaImage::new(1, 1)
        } else {
            let bytes = lib.images.get(*guid)
                .ok_or_else(|| anyhow!("pack: an entity references image {guid} that is not in library/sprites"))?;
            image::load_from_memory(bytes).with_context(|| format!("decoding image {guid}"))?.to_rgba8()
        };
        let mut full = full;
        as_the_editor_stores(&mut full);
        let (img, tx, ty) = trimmed(&full);
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (img.width(), img.height()).hash(&mut h); img.as_raw().hash(&mut h); h.finish()
        };
        Ok((img, tx, ty, hash))
    }).collect::<Result<_>>()?;

    step("decode");
    // ---- sheets: one per group, identical pictures packed once, shelf-packed ----
    struct Item { guids: Vec<(String, u32, u32)>, img: RgbaImage }
    let mut per_group: BTreeMap<String, Vec<Item>> = BTreeMap::new();
    let mut same: HashMap<(String, u64), usize> = HashMap::new(); // (group, picture hash) -> item
    for ((guid, groups), (img, tx, ty, hash)) in used.iter().zip(decoded) {
        for g in groups.keys() {
            let items = per_group.entry(g.clone()).or_default();
            match same.get(&(g.clone(), hash)) {
                Some(&i) => items[i].guids.push(((*guid).clone(), tx, ty)),
                None => { same.insert((g.clone(), hash), items.len()); items.push(Item { guids: vec![((*guid).clone(), tx, ty)], img: img.clone() }); }
            }
        }
    }

    // the layout is decided in order; each sheet is then drawn and encoded on its own
    struct Sheet<'a> { group: String, items: &'a [Item], at: Vec<(usize, u32, u32)>, w: u32, h: u32 }
    let mut placed: HashMap<(String, String), Placed> = HashMap::new(); // (group, guid) -> place
    let mut layout: Vec<Sheet> = Vec::new();
    for items in per_group.values_mut() {
        // tallest first, then a shelf per row; a group that does not fit one texture goes on
        // as many as it takes, each a sheet of the same group, and a keyframe names its sheet
        items.sort_by(|a, b| b.img.height().cmp(&a.img.height()).then(a.guids[0].0.cmp(&b.guids[0].0)));
    }
    for (group, items) in &per_group {
        let mut sheet_no = 0usize;
        let mut pending: Vec<(usize, u32, u32)> = Vec::new(); // item, x, y
        let (mut x, mut y, mut row_h, mut used_w) = (0u32, 0u32, 0u32, 0u32);
        let mut close = |sheet_no: usize, pending: &mut Vec<(usize, u32, u32)>, used_w: u32, used_h: u32| {
            for (fi, &(it, _, _)) in pending.iter().enumerate() {
                for (guid, tx, ty) in &items[it].guids {
                    placed.insert((group.clone(), guid.clone()), Placed { sheet: sheet_no, frame: fi, trim_x: *tx as f64, trim_y: *ty as f64 });
                }
            }
            layout.push(Sheet { group: group.clone(), items, at: std::mem::take(pending),
                w: pot(used_w.max(1)), h: pot(used_h.max(1)) });
        };
        for (idx, it) in items.iter().enumerate() {
            let (w, h) = it.img.dimensions();
            if w > SHEET_MAX || h > SHEET_MAX { bail!("pack: a picture in sheet group {group:?} is larger than a {SHEET_MAX}x{SHEET_MAX} texture"); }
            if x > 0 && x + w > SHEET_MAX { x = 0; y += row_h + GAP; row_h = 0; }
            if y + h > SHEET_MAX {
                close(sheet_no, &mut pending, used_w, y + row_h);
                sheet_no += 1;
                x = 0; y = 0; row_h = 0; used_w = 0;
            }
            pending.push((idx, x, y));
            x += w + GAP; row_h = row_h.max(h); used_w = used_w.max(x - GAP);
        }
        if !pending.is_empty() {
            close(sheet_no, &mut pending, used_w, y + row_h);
        }
    }
    step("layout");
    let sheets: Vec<(String, Vec<u32>, Vec<u8>)> = layout.par_iter().map(|s| -> Result<_> {
        let mut sheet = RgbaImage::new(s.w, s.h);
        let mut frames: Vec<u32> = Vec::with_capacity(s.at.len() * 4);
        for &(it, rx, ry) in &s.at {
            let img = &s.items[it].img;
            image::imageops::replace(&mut sheet, img, rx as i64, ry as i64);
            frames.extend([rx, ry, img.width(), img.height()]);
        }
        Ok((s.group.clone(), frames, encode_png(&sheet)))
    }).collect::<Result<_>>()?;

    step("sheets");
    // ---- entities, compiled ----
    let out_entities: Vec<Value> = if opts.parallel_json {
        entities.par_iter().map(|e| compile_entity(e, &group_of(e), &placed, true)).collect::<Result<_>>()?
    } else {
        entities.iter().map(|e| compile_entity(e, &group_of(e), &placed, false)).collect::<Result<_>>()?
    };

    step("entities");
    // ---- scripts: every exported script file, the manifest, the palettes ----
    let mut out_scripts: Vec<Value> = Vec::new();
    for s in &lib.scripts {
        let m = &s.meta;
        if !exported(m) { continue; }
        // a script file with no language named is hscript, as the editor takes it
        let language = m.get("language").and_then(Value::as_str).filter(|l| !l.is_empty()).unwrap_or("hscript");
        out_scripts.push(json!({
            "guid": m["guid"], "id": m["id"], "language": language,
            "metadata": fm_meta(m), "tags": m.get("tags").cloned().unwrap_or_else(|| json!([])),
            "value": s.text, "version": m.get("version").cloned().unwrap_or_else(|| json!(1)),
        }));
    }
    let name = lib.manifest["resourceId"].as_str().ok_or_else(|| anyhow!("pack: manifest.json names no resourceId"))?.to_string();
    {
        let m = &lib.manifest_meta;
        out_scripts.push(json!({
            "guid": m["guid"], "id": m.get("id").cloned().unwrap_or_else(|| json!("manifest")), "language": "json",
            "metadata": fm_meta(m), "tags": m.get("tags").cloned().unwrap_or_else(|| json!([])),
            "value": serde_json::to_string(&lib.manifest)?, "version": m.get("version").cloned().unwrap_or_else(|| json!(1)),
        }));
    }
    for (pal, m) in &lib.palettes {
        if !exported(m) { continue; }
        out_scripts.push(json!({
            "guid": pal["guid"], "id": pal["id"], "language": "json", "metadata": json!({}),
            "tags": pal.get("tags").cloned().unwrap_or_else(|| json!([])),
            "value": serde_json::to_string(&compile_palettes(pal))?,
            "version": pal.get("version").cloned().unwrap_or_else(|| json!(1)),
        }));
    }
    out_scripts.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));

    // ---- blobs: the sheets, then the sounds themselves, placed by offset and copied once ----
    let mut blobs: Vec<&[u8]> = Vec::new();
    let mut at = 0usize;
    let mut out_sheets: Vec<Value> = Vec::new();
    for (group, frames, png) in &sheets {
        out_sheets.push(json!({ "bytesLength": png.len(), "bytesOffset": at, "frames": frames, "group": group, "version": 1 }));
        at += png.len();
        blobs.push(png);
    }
    let mut out_audio: Vec<Value> = Vec::new();
    for a in &lib.audio {
        let m = &a.meta;
        if !exported(m) { continue; }
        out_audio.push(json!({
            "bytesLength": a.bytes.len(), "bytesOffset": at, "format": a.format,
            "guid": m["guid"], "id": m["id"], "metadata": fm_meta(m),
            "tags": m.get("tags").cloned().unwrap_or_else(|| json!([])), "version": m.get("version").cloned().unwrap_or_else(|| json!(1)),
        }));
        at += a.bytes.len();
        blobs.push(&a.bytes);
    }

    let header = json!({
        "audio": out_audio, "binary": [], "entities": out_entities, "images": [], "nineSlices": [],
        "scripts": out_scripts, "spritesheets": out_sheets, "version": PACKAGE_VERSION,
    });
    step("scripts");
    let mut head = Vec::new();
    if opts.parallel_json { write_json(&header, &mut head)?; } else { serde_json::to_writer(&mut head, &header)?; }
    step("header");
    let mut bytes = Vec::with_capacity(4 + head.len() + at);
    bytes.extend_from_slice(&(head.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&head);
    for b in blobs { bytes.extend_from_slice(b); }
    step("assemble");
    Ok((Built { name, bytes, entities: entities.len(), scripts: lib.scripts.len() + 1 + lib.palettes.len(),
               audio: lib.audio.len(), sheets: sheets.len() }, steps))
}

/// `serde_json`'s compact text of `v`, byte for byte, with the elements of every long array
/// written side by side: a fighter's header is one entity of hundreds of animations, and
/// writing it out is otherwise one long serial pass
fn write_json(v: &Value, out: &mut Vec<u8>) -> Result<()> {
    match v {
        Value::Array(a) if a.len() > 8 => {
            let parts: Vec<Vec<u8>> = a.par_iter()
                .map(|e| { let mut b = Vec::new(); write_json(e, &mut b).map(|_| b) }).collect::<Result<_>>()?;
            out.push(b'[');
            for (i, p) in parts.iter().enumerate() {
                if i > 0 { out.push(b','); }
                out.extend_from_slice(p);
            }
            out.push(b']');
        }
        Value::Object(m) => {
            out.push(b'{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 { out.push(b','); }
                serde_json::to_writer(&mut *out, k)?;
                out.push(b':');
                write_json(x, out)?;
            }
            out.push(b'}');
        }
        _ => serde_json::to_writer(&mut *out, v)?,
    }
    Ok(())
}

/// A PNG of `img`, its rows compressed in bands side by side. One picture is one compressed
/// stream, so a sheet otherwise compresses on one thread however many there are; here each band
/// of rows is filtered and deflated on its own and the bands are joined into the one stream
/// (each but the last ends on a flush, so the next starts on a byte), as pigz does. Rows only
/// look left, never up, so a band needs nothing from the one before it.
fn encode_png(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let stride = w as usize * 4;
    let raw = img.as_raw();
    // a fixed band, not one sized by the threads, so every machine writes the same bytes
    let band = PNG_BAND_ROWS;
    let starts: Vec<usize> = (0..h as usize).step_by(band).collect();
    let parts: Vec<(Vec<u8>, u32, usize)> = starts.par_iter().map(|&y0| {
        let y1 = (y0 + band).min(h as usize);
        let mut filtered = Vec::with_capacity((y1 - y0) * (stride + 1));
        let mut row = vec![0u8; stride];
        for y in y0..y1 {
            sub_filter(&raw[y * stride..(y + 1) * stride], &mut row);
            filtered.push(1);
            filtered.extend_from_slice(&row);
        }
        let last = y1 == h as usize;
        let mut z = flate2::Compress::new(flate2::Compression::new(PNG_LEVEL), false);
        let mut out = Vec::with_capacity(filtered.len() / 2 + 64);
        let flush = if last { flate2::FlushCompress::Finish } else { flate2::FlushCompress::Sync };
        loop {
            let before = z.total_in() as usize;
            out.reserve(filtered.len() / 2 + 1024);
            let status = z.compress_vec(&filtered[before..], &mut out, flush).expect("deflate");
            if (last && status == flate2::Status::StreamEnd) || (!last && z.total_in() as usize == filtered.len() && out.capacity() > out.len()) { break; }
        }
        (out, adler32(&filtered), filtered.len())
    }).collect();
    let mut idat = vec![0x78, 0x9c];
    let mut adler = 1u32;
    for (data, a, len) in &parts {
        idat.extend_from_slice(data);
        adler = adler32_combine(adler, *a, *len);
    }
    idat.extend_from_slice(&adler.to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits, RGBA, deflate, per-row filters, no interlace
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &idat);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let mut c = crc32fast::Hasher::new();
    c.update(kind);
    c.update(data);
    png.extend_from_slice(&c.finalize().to_be_bytes());
}

/// the row with the Sub filter: each byte less the one a pixel to its left. Measured against
/// choosing a filter per row, Sub on every row packs sheets as small or smaller, in one pass
fn sub_filter(cur: &[u8], out: &mut [u8]) {
    const BPP: usize = 4;
    out[..BPP].copy_from_slice(&cur[..BPP]);
    for i in BPP..cur.len() { out[i] = cur[i].wrapping_sub(cur[i - BPP]); }
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let (mut a, mut b) = (1u32, 0u32);
    for block in data.chunks(5552) {
        for &x in block { a += x as u32; b += a; }
        a %= MOD; b %= MOD;
    }
    (b << 16) | a
}

/// the adler32 of two runs of bytes joined, from each run's own and the second's length
fn adler32_combine(a1: u32, a2: u32, len2: usize) -> u32 {
    const MOD: u64 = 65521;
    let rem = (len2 as u64) % MOD;
    let (s1a, s2a) = ((a1 & 0xffff) as u64, (a1 >> 16) as u64);
    let (s1b, s2b) = ((a2 & 0xffff) as u64, (a2 >> 16) as u64);
    let s1 = (s1a + s1b + MOD - 1) % MOD;
    let s2 = (s2a + s2b + (rem * s1a) % MOD + MOD - rem) % MOD;
    ((s2 as u32) << 16) | s1 as u32
}

fn pot(n: u32) -> u32 { let mut p = 1; while p < n { p <<= 1; } p }

/// The colours a picture has once the editor has drawn it: it holds pictures with their colour
/// multiplied by their alpha, a byte each, so a faint pixel keeps only as many shades of colour
/// as its alpha allows and a clear one keeps none. Measured against the editor's own packages:
/// every pixel the same. The engine draws them the same either way; the sheets compress far
/// smaller, since a stage is mostly soft, see-through art.
fn as_the_editor_stores(img: &mut RgbaImage) {
    for p in img.pixels_mut() {
        let a = p[3] as u32;
        match a {
            0 => { p[0] = 0; p[1] = 0; p[2] = 0; }
            255 => {}
            _ => for c in 0..3 {
                let held = (p[c] as u32 * a + 127) / 255;
                p[c] = ((held * 255 + a / 2) / a).min(255) as u8;
            },
        }
    }
}

/// the image cut to its opaque bounds, and how far the cut moved its top-left corner
fn trimmed(full: &RgbaImage) -> (RgbaImage, u32, u32) {
    let (w, h) = full.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for (x, y, p) in full.enumerate_pixels() {
        if p[3] != 0 { x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x + 1); y1 = y1.max(y + 1); }
    }
    if x1 <= x0 || y1 <= y0 { return (RgbaImage::new(1, 1), 0, 0); }
    (image::imageops::crop_imm(full, x0, y0, x1 - x0, y1 - y0).to_image(), x0, y0)
}

fn compile_palettes(pal: &Value) -> Value {
    let colors: HashMap<&str, &str> = pal["colors"].as_array().into_iter().flatten()
        .filter_map(|c| Some((c["$id"].as_str()?, c["color"].as_str()?))).collect();
    let palettes: Vec<Value> = pal["maps"].as_array().into_iter().flatten().map(|m| {
        let mut map = Map::new();
        for c in m["colors"].as_array().into_iter().flatten() {
            if let (Some(src), Some(dst)) = (c["paletteColorId"].as_str().and_then(|i| colors.get(i)), c["targetColor"].as_str()) {
                map.entry(src.to_string()).or_insert_with(|| json!(dst));
            }
        }
        json!({ "name": m["name"], "colors": Value::Object(map) })
    }).collect();
    json!({ "indexed": { "base": 0, "red": -1, "green": -1, "blue": -1 }, "palettes": palettes })
}

fn compile_entity(e: &Value, group: &str, placed: &HashMap<(String, String), Placed>, parallel: bool) -> Result<Value> {
    let by_id = |key: &str| -> HashMap<String, &Value> {
        e[key].as_array().into_iter().flatten().filter_map(|v| Some((v["$id"].as_str()?.to_string(), v))).collect()
    };
    let symbols = by_id("symbols");
    let keyframes = by_id("keyframes");
    let layers = by_id("layers");
    let eid = e["id"].as_str().unwrap_or("?");
    let empty = Vec::new();
    let compile_anim = |a: &Value| -> Result<Value> {
        let mut out_layers: Vec<Value> = Vec::new();
        for lid in a["layers"].as_array().into_iter().flatten() {
            let l = layers.get(lid.as_str().unwrap_or("")).ok_or_else(|| anyhow!("{eid}: animation {} names a layer it does not have", a["name"]))?;
            let ltype = l["type"].as_str().unwrap_or("");
            let mut out_kfs: Vec<Value> = Vec::new();
            for kid in l["keyframes"].as_array().into_iter().flatten() {
                let k = keyframes.get(kid.as_str().unwrap_or("")).ok_or_else(|| anyhow!("{eid}: layer {} names a keyframe it does not have", l["name"]))?;
                let mut kf = Map::new();
                kf.insert("length".into(), k.get("length").cloned().unwrap_or_else(|| json!(1)));
                kf.insert("metadata".into(), fm_meta(k));
                match ltype {
                    "LABEL" => { kf.insert("name".into(), k.get("name").cloned().unwrap_or_else(|| json!(""))); }
                    "FRAME_SCRIPT" => {
                        if let Some(code) = k.get("code").and_then(Value::as_str) { kf.insert("code".into(), json!(code)); }
                    }
                    "CONTAINER" => {}
                    _ => {
                        let sym = match k.get("symbol").and_then(Value::as_str).and_then(|s| symbols.get(s)) {
                            Some(s) => compile_symbol(s, ltype, group, placed).with_context(|| format!("{eid} {} layer {}", a["name"], l["name"]))?,
                            None => Value::Null,
                        };
                        kf.insert("symbol".into(), sym);
                    }
                }
                out_kfs.push(Value::Object(kf));
            }
            let mut ol = Map::new();
            ol.insert("keyframes".into(), Value::Array(out_kfs));
            if ltype == "FRAME_SCRIPT" {
                let lang = l.get("language").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("hscript");
                ol.insert("language".into(), json!(lang));
            }
            ol.insert("metadata".into(), fm_meta(l));
            ol.insert("name".into(), l.get("name").cloned().unwrap_or_else(|| json!("")));
            ol.insert("type".into(), json!(ltype));
            out_layers.push(Value::Object(ol));
        }
        Ok(json!({ "layers": out_layers, "metadata": fm_meta(a), "name": a["name"] }))
    };
    let all = e["animations"].as_array().unwrap_or(&empty);
    let anims: Vec<Value> = if parallel { all.par_iter().map(compile_anim).collect::<Result<_>>()? }
        else { all.iter().map(compile_anim).collect::<Result<_>>()? };
    Ok(json!({
        "animations": anims, "guid": e["guid"], "id": e["id"], "metadata": fm_meta(e),
        "tags": e.get("tags").cloned().unwrap_or_else(|| json!([])), "version": e.get("version").cloned().unwrap_or_else(|| json!(1)),
    }))
}

fn compile_symbol(s: &Value, ltype: &str, group: &str, placed: &HashMap<(String, String), Placed>) -> Result<Value> {
    let alpha = num_or_null(s, "alpha");
    Ok(match ltype {
        "IMAGE" => {
            let guid = s["imageAsset"].as_str().unwrap_or("");
            let p = placed.get(&(group.to_string(), guid.to_string())).ok_or_else(|| anyhow!("image {guid} was not packed into sheet {group}"))?;
            let (sx, sy) = (num(s, "scaleX", 1.0), num(s, "scaleY", 1.0));
            // the trim moved the picture's corner in its own axes; the keyframe places the
            // picture already scaled and turned, so the move is scaled and turned the same way
            let rot = num(s, "rotation", 0.0).to_radians();
            let (dx, dy) = (p.trim_x * sx, p.trim_y * sy);
            // libm's, not the platform's: the same bytes from every build, the browser's included
            let (cos, sin) = (libm::cos(rot), libm::sin(rot));
            let (ox, oy) = (cos * dx - sin * dy, sin * dx + cos * dy);
            json!({ "data": [
                num(s, "x", 0.0) + ox, num(s, "y", 0.0) + oy,
                if alpha.is_null() { json!(1) } else { alpha }, num(s, "pivotX", 0.0), num(s, "pivotY", 0.0),
                num(s, "rotation", 0.0), sx, sy, p.sheet, p.frame,
            ], "metadata": fm_meta(s) })
        }
        "COLLISION_BOX" => json!({ "color": s.get("color").cloned().unwrap_or(Value::Null), "data": [
            num(s, "x", 0.0), num(s, "y", 0.0), alpha, num(s, "pivotX", 0.0), num(s, "pivotY", 0.0),
            num(s, "rotation", 0.0), num(s, "scaleX", 1.0), num(s, "scaleY", 1.0),
        ], "metadata": fm_meta(s) }),
        "COLLISION_BODY" => json!({ "color": s.get("color").cloned().unwrap_or(Value::Null), "data": [
            num(s, "head", 0.0), num(s, "hipWidth", 0.0), num(s, "hipXOffset", 0.0), num(s, "hipYOffset", 0.0), num(s, "foot", 0.0),
        ], "metadata": fm_meta(s) }),
        "POINT" => json!({ "color": s.get("color").cloned().unwrap_or(Value::Null), "data": [
            num(s, "x", 0.0), num(s, "y", 0.0), if alpha.is_null() { json!(1) } else { alpha }, num(s, "rotation", 0.0),
        ], "metadata": fm_meta(s) }),
        "LINE_SEGMENT" => json!({ "color": s.get("color").cloned().unwrap_or(Value::Null),
            "data": [ if alpha.is_null() { json!(1) } else { alpha } ], "metadata": fm_meta(s),
            "points": s.get("points").cloned().unwrap_or_else(|| json!([])) }),
        other => bail!("pack: no compiled form for a {other} symbol"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a picture over several bands, with every kind of pixel a sheet has
    fn picture(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            let v = x.wrapping_mul(2654435761).wrapping_add(y.wrapping_mul(40503));
            let a = match (x + y) % 5 { 0 => 0, 1 => 255, _ => (v >> 24) as u8 };
            image::Rgba([(v >> 8) as u8, (v >> 16) as u8, v as u8, a])
        })
    }

    #[test]
    fn a_banded_png_decodes_to_the_same_pixels() {
        for (w, h) in [(1, 1), (37, 63), (64, 64), (130, PNG_BAND_ROWS as u32 * 3 + 5)] {
            let img = picture(w, h);
            let back = image::load_from_memory(&encode_png(&img)).expect("decodes").to_rgba8();
            assert_eq!(back, img, "{w}x{h}");
        }
    }

    #[test]
    fn colours_round_as_the_editor_stores_them() {
        let mut img = RgbaImage::from_pixel(4, 1, image::Rgba([200, 100, 50, 8]));
        img.put_pixel(1, 0, image::Rgba([9, 9, 9, 0]));
        img.put_pixel(2, 0, image::Rgba([12, 34, 56, 255]));
        img.put_pixel(3, 0, image::Rgba([255, 255, 255, 1]));
        as_the_editor_stores(&mut img);
        // alpha 8 keeps eight shades: 200 -> 6/255 held -> 191
        assert_eq!(img.get_pixel(0, 0).0, [191, 96, 64, 8]);
        assert_eq!(img.get_pixel(1, 0).0, [0, 0, 0, 0]);
        assert_eq!(img.get_pixel(2, 0).0, [12, 34, 56, 255]);
        assert_eq!(img.get_pixel(3, 0).0, [255, 255, 255, 1]);
    }

    #[test]
    fn adler32_of_runs_joined_is_the_adler32_of_the_whole() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        for cut in [0, 1, 5552, 65521, 123_456, data.len()] {
            let (a, b) = data.split_at(cut);
            assert_eq!(adler32_combine(adler32(a), adler32(b), b.len()), adler32(&data), "cut at {cut}");
        }
    }

    #[test]
    fn the_same_library_builds_the_same_bytes_with_and_without_parallel_json() {
        let png = { let mut c = std::io::Cursor::new(Vec::new()); picture(40, 30).write_to(&mut c, image::ImageFormat::Png).unwrap(); c.into_inner() };
        let entity = json!({
            "id": "e", "guid": "g", "animations": [{ "name": "idle", "layers": ["l"] }],
            "layers": [{ "$id": "l", "type": "IMAGE", "name": "art", "keyframes": ["k"] }],
            "keyframes": [{ "$id": "k", "length": 2, "symbol": "s" }],
            "symbols": [{ "$id": "s", "type": "IMAGE", "imageAsset": "img", "x": 3.0, "y": 4.0, "rotation": 30.0 }],
        });
        let lib = Library {
            images: [("img".to_string(), png)].into_iter().collect(),
            entities: vec![entity],
            manifest: json!({ "resourceId": "pkg" }),
            manifest_meta: json!({ "guid": "m" }),
            ..Default::default()
        };
        let a = build_timed(&lib, Options { parallel_json: true }, &|| 0.0).unwrap().0;
        let b = build_timed(&lib, Options { parallel_json: false }, &|| 0.0).unwrap().0;
        assert_eq!(a.name, "pkg");
        assert_eq!(a.bytes, b.bytes);
    }
}
