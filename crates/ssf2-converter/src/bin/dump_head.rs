//! Diagnostic: what a character's portrait clip actually PLACES, and what it composites to.
//!
//! The clip is a little scene, so "what is in it" is not answered by the sprite dump (which only
//! prints placements that carry a symbol name, and the pieces of a portrait are usually bare
//! shapes). This prints every placement with its depth and its clip depth (the window), then runs
//! the real compositing path over it.
//!
//! With no clip name it reads the one the character's own stats declare as its `thumbnail`, which
//! is what the conversion does, so pointing it at every file in a corpus answers "does this work
//! on content nobody has looked at".
//!
//! usage: dump_head <file.ssf> [clip-name] [out-dir]
//!        dump_head --scan <dir>          # one line per character
use std::collections::BTreeMap;
use std::io::Cursor;

/// The clip a character's stats name as its portrait, plus the id it registered.
fn declared_thumbnail(data: &[u8]) -> (Option<String>, Option<String>) {
    let Ok(parsed) = ssf2_converter::swf_parser::parse(data) else { return (None, None) };
    let mut id = None;
    let mut thumb = None;
    for blk in &parsed.abc_blocks {
        let Ok(abc) = ssf2_converter::abc_parser::parse(blk) else { continue };
        if id.is_none() {
            id = ssf2_converter::abc_parser::extract_main_package_metadata(&abc).and_then(|m| m.id);
        }
        if let Some(cid) = id.clone() {
            if let Some((body, _)) = ssf2_converter::abc_parser::find_bundle_method(&abc, &cid) {
                thumb = ssf2_converter::abc_parser::extract_string_stat(body, &abc, "thumbnail");
                if thumb.is_some() { break }
            }
        }
    }
    (id, thumb)
}

struct Report { placements: usize, shapes: usize, bitmaps: usize, sprites: usize, windows: usize }

fn inspect(swf: &swf::Swf<'_>, id: u16) -> Report {
    let mut r = Report { placements: 0, shapes: 0, bitmaps: 0, sprites: 0, windows: 0 };
    let mut kind: BTreeMap<u16, &str> = BTreeMap::new();
    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineShape(s) => { kind.insert(s.id, "shape"); }
            swf::Tag::DefineSprite(s) => { kind.insert(s.id, "sprite"); }
            swf::Tag::DefineBitsLossless(b) => { kind.insert(b.id, "bitmap"); }
            swf::Tag::DefineBitsJpeg3(b) => { kind.insert(b.id, "bitmap"); }
            swf::Tag::DefineBitsJpeg2 { id, .. } => { kind.insert(*id, "bitmap"); }
            _ => {}
        }
    }
    for tag in &swf.tags {
        let swf::Tag::DefineSprite(s) = tag else { continue };
        if s.id != id { continue }
        for t in &s.tags {
            match t {
                swf::Tag::ShowFrame => break,
                swf::Tag::PlaceObject(po) => {
                    let cid = match &po.action {
                        swf::PlaceObjectAction::Place(i) | swf::PlaceObjectAction::Replace(i) => Some(*i),
                        swf::PlaceObjectAction::Modify => None,
                    };
                    r.placements += 1;
                    if po.clip_depth.is_some() { r.windows += 1; }
                    match cid.and_then(|c| kind.get(&c)).copied() {
                        Some("shape") => r.shapes += 1,
                        Some("bitmap") => r.bitmaps += 1,
                        Some("sprite") => r.sprites += 1,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    r
}

fn sp_shape<'a>(swf: &'a swf::Swf<'a>, id: u16) -> Option<&'a swf::Shape> {
    swf.tags.iter().find_map(|t| match t {
        swf::Tag::DefineShape(s) if s.id == id => Some(s),
        _ => None,
    })
}

fn symbols(swf: &swf::Swf<'_>) -> BTreeMap<u16, String> {
    let mut m = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for l in links {
                m.insert(l.id, l.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
            }
        }
    }
    m
}

fn scan(dir: &str) {
    let mut files: Vec<_> = std::fs::read_dir(dir).expect("read dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "ssf"))
        .collect();
    files.sort();
    println!("{:<20} {:<24} {:<9} placements (shape/bitmap/sprite)", "file", "thumbnail", "window");
    for f in files {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        let Ok(data) = std::fs::read(&f) else { continue };
        let (_, thumb) = declared_thumbnail(&data);
        let Some(thumb) = thumb else {
            println!("{name:<20} {:<24} {:<9} -", "(none declared)", "-");
            continue;
        };
        let Ok(buf) = swf::decompress_swf(Cursor::new(&data)) else { continue };
        let Ok(swf) = swf::parse_swf(&buf) else { continue };
        let syms = symbols(&swf);
        let Some((&id, _)) = syms.iter().find(|(_, n)| n.eq_ignore_ascii_case(&thumb)).map(|(i, n)| (i, n))
        else {
            println!("{name:<20} {thumb:<24} {:<9} (clip not in this file)", "-");
            continue;
        };
        let r = inspect(&swf, id);
        // how much of the window's own rect the trimmed picture keeps: if these disagree, the
        // picture is not sitting in the frame the way the source frames it
        let win = swf.tags.iter().find_map(|t| match t {
            swf::Tag::DefineSprite(sp) if sp.id == id => {
                let mut rect: Option<(f64, f64, f64, f64)> = None;
                for st in &sp.tags {
                    if let swf::Tag::PlaceObject(po) = st {
                        if po.clip_depth.is_none() { continue }
                        if let swf::PlaceObjectAction::Place(cid) = &po.action {
                            if let Some(sh) = sp_shape(&swf, *cid) {
                                let b = &sh.shape_bounds;
                                let r = (b.x_min.to_pixels(), b.y_min.to_pixels(),
                                         b.x_max.to_pixels(), b.y_max.to_pixels());
                                rect = Some(match rect {
                                    Some(w) => (w.0.min(r.0), w.1.min(r.1), w.2.max(r.2), w.3.max(r.3)),
                                    None => r,
                                });
                            }
                        }
                    }
                }
                rect
            }
            _ => None,
        });
        let winsz = win.map(|w| format!("{:.0}x{:.0}", w.2 - w.0, w.3 - w.1)).unwrap_or_else(|| "-".into());
        let out = std::env::temp_dir().join("peptide-head-scan");
        let composited = ssf2_converter::image_extractor::composite_head(&swf, id, &out, &name, true)
            .ok().flatten()
            .map(|i| format!("{}x{}", i.width, i.height))
            .unwrap_or_else(|| "FAILED".into());
        println!("{name:<20} {thumb:<24} window {winsz:<9} -> {composited}   ({} placements {}/{}/{})",
            r.placements, r.shapes, r.bitmaps, r.sprites);
    }
}

/// What kind of definition an id is, for diagnosing a piece that did not draw.
fn kind_of(swf: &swf::Swf<'_>, want: u16) -> String {
    for tag in &swf.tags {
        let (id, what) = match tag {
            swf::Tag::DefineShape(s) => (s.id, "DefineShape"),
            swf::Tag::DefineSprite(s) => (s.id, "DefineSprite"),
            swf::Tag::DefineBitsLossless(b) => (b.id, "DefineBitsLossless"),
            swf::Tag::DefineBitsJpeg3(b) => (b.id, "DefineBitsJpeg3"),
            swf::Tag::DefineBitsJpeg2 { id, .. } => (*id, "DefineBitsJpeg2"),
            swf::Tag::DefineBits { id, .. } => (*id, "DefineBits"),
            swf::Tag::DefineMorphShape(m) => (m.id, "DefineMorphShape"),
            swf::Tag::DefineText(t) => (t.id, "DefineText"),
            swf::Tag::DefineEditText(t) => (t.id(), "DefineEditText"),
            swf::Tag::DefineButton2(b) => (b.id, "DefineButton2"),
            swf::Tag::DefineVideoStream(v) => (v.id, "DefineVideoStream"),
            _ => continue,
        };
        if id == want { return what.to_string() }
    }
    "(not defined in this file)".to_string()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next().expect("usage: dump_head <file.ssf> [clip] [out] | --scan <dir>");
    if first == "--scan" {
        scan(&args.next().expect("usage: dump_head --scan <dir>"));
        return;
    }
    let path = first;
    let data = std::fs::read(&path).expect("read file");
    let want = args.next().or_else(|| declared_thumbnail(&data).1)
        .expect("no clip named and none declared");
    let out = args.next().unwrap_or_else(|| std::env::temp_dir().display().to_string());

    let swf_buf = swf::decompress_swf(Cursor::new(&data)).expect("decompress");
    let swf = swf::parse_swf(&swf_buf).expect("parse");
    let sym = symbols(&swf);
    // `#<id>` addresses a clip that carries no symbol name, which is how the pieces of a portrait
    // usually appear.
    let found = match want.strip_prefix('#').and_then(|n| n.parse::<u16>().ok()) {
        Some(id) => Some((id, sym.get(&id).cloned().unwrap_or_else(|| format!("#{id}")))),
        None => sym.iter().find(|(_, n)| n.eq_ignore_ascii_case(&want)).map(|(i, n)| (*i, n.clone())),
    };
    let Some((id, name)) = found else {
        println!("no symbol named {want:?} in {path}");
        return;
    };
    println!("=== {name} (sprite {id}) ===");
    for tag in &swf.tags {
        let swf::Tag::DefineSprite(s) = tag else { continue };
        if s.id != id { continue }
        println!("  ({} frames)", s.num_frames);
        let mut frame = 0u16;
        for t in &s.tags {
            if matches!(t, swf::Tag::ShowFrame) { frame += 1; }
            if let swf::Tag::FrameLabel(fl) = t {
                println!("  [f{frame}] LABEL {}", fl.label.to_str_lossy(encoding_rs::WINDOWS_1252));
            }
            if let swf::Tag::PlaceObject(po) = t {
                print!("  [f{frame}]");
                let cid = match &po.action {
                    swf::PlaceObjectAction::Place(i) | swf::PlaceObjectAction::Replace(i) => Some(*i),
                    swf::PlaceObjectAction::Modify => None,
                };
                let m = po.matrix.map(|m| format!(
                    "tx={:.1} ty={:.1} a={:.3} d={:.3}",
                    m.tx.get() as f64 / 20.0, m.ty.get() as f64 / 20.0, m.a.to_f64(), m.d.to_f64()))
                    .unwrap_or_else(|| "no matrix".into());
                let bounds = cid.and_then(|c| swf.tags.iter().find_map(|t| match t {
                    swf::Tag::DefineShape(sh) if sh.id == c => Some(format!(
                        " bounds=({:.0},{:.0})-({:.0},{:.0})",
                        sh.shape_bounds.x_min.to_pixels(), sh.shape_bounds.y_min.to_pixels(),
                        sh.shape_bounds.x_max.to_pixels(), sh.shape_bounds.y_max.to_pixels())),
                    _ => None,
                })).unwrap_or_default();
                let vis = match po.is_visible { Some(false) => " HIDDEN", _ => "" };
                let ct = po.color_transform.map(|c| format!(" ct(a_mult={:?})", c.a_multiply)).unwrap_or_default();
                println!("  depth={:<3} clip={:<4} id={:<5} sym={:<24} {m}{bounds}{vis}{ct}", po.depth,
                    po.clip_depth.map(|c| c.to_string()).unwrap_or_else(|| "-".into()),
                    cid.map(|i| i.to_string()).unwrap_or_else(|| "-".into()),
                    cid.and_then(|i| sym.get(&i)).cloned().unwrap_or_else(|| "(shape)".into()));
            }
        }
    }
    if std::env::var("PEPTIDE_HEAD_DEBUG").is_ok() {
        for tag in &swf.tags {
            let swf::Tag::DefineSprite(sp) = tag else { continue };
            if sp.id != id { continue }
            for t in &sp.tags {
                if let swf::Tag::PlaceObject(po) = t {
                    if let swf::PlaceObjectAction::Place(cid) = &po.action {
                        println!("  id {cid} is {}", kind_of(&swf, *cid));
                    }
                }
            }
        }
    }
    let char_name = std::path::Path::new(&path).file_stem().unwrap().to_string_lossy().to_string();
    match ssf2_converter::image_extractor::composite_head(&swf, id, std::path::Path::new(&out), &char_name, true) {
        Ok(Some(img)) => println!("composited -> {} ({}x{})", img.png_path, img.width, img.height),
        Ok(None) => println!("not composited (nothing drawable in the clip)"),
        Err(e) => println!("compositing failed: {e}"),
    }
}
