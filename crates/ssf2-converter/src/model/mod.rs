//! The source model: what an SSF2 package is made of, read once.
//!
//! A package is classes and timelines. This reads both as the package states them -- every class
//! with its superclass, fields, methods (as decoded trees) and the frame scripts its constructor
//! registers; every symbol the package binds to a class; every sprite's frames, labels and the
//! children each frame places -- and keeps them in that shape. Everything after this reads the
//! model, never the package again, and never a name guessed from another name.

pub mod ast;
pub mod declare;
pub mod rename;
pub mod decode;
pub mod structure;
pub mod tidy;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use anyhow::Result;
use swf::avm2::types::{AbcFile, TraitKind};

use crate::swf_timeline::{self, Placed};
use ast::{Expr, Function, Stmt};
use decode::{Pool, Slots};

/// A whole package.
pub struct SourceModel {
    /// every class the package's code defines, by the name it is bound and referred to by
    pub classes: BTreeMap<String, ClassModel>,
    /// a symbol (character id) the package binds to a class
    pub symbols: BTreeMap<u16, String>,
    /// every sprite (movie clip) the package defines, by character id
    pub sprites: BTreeMap<u16, SpriteModel>,
    /// every bitmap the package defines, by character id: its size in pixels (0 where its data
    /// does not say without decoding)
    pub bitmaps: BTreeMap<u16, (u32, u32)>,
    /// the package's own main timeline
    pub root: SpriteModel,
    /// each code block's script initializers: where its classes are defined and its globals set
    pub scripts: Vec<Function>,
}

impl ClassModel {
    /// A class with nothing declared.
    pub fn empty(name: &str) -> ClassModel {
        let f = || Function { params: vec![], rest: false, return_type: None, local_names: vec![], activation: vec![], body: vec![] };
        ClassModel { name: name.into(), super_name: String::new(), block: 0, symbol: None, fields: vec![], methods: vec![],
                     constructor: f(), static_init: f(), frame_scripts: vec![], unreadable: vec![] }
    }
}

/// One class, as its code states it.
#[derive(Clone)]
pub struct ClassModel {
    pub name: String,
    pub super_name: String,
    /// which code block of the package defines it
    pub block: usize,
    /// the symbol this class is bound to, when one is
    pub symbol: Option<u16>,
    pub fields: Vec<Field>,
    pub methods: Vec<MethodModel>,
    pub constructor: Function,
    /// the static initializer, where a computed static value is filled in
    pub static_init: Function,
    /// the frame scripts the constructor registers, in the order it registers them
    pub frame_scripts: Vec<FrameScript>,
    /// what in this class's code could not be read, and why
    pub unreadable: Vec<String>,
}

#[derive(Clone)]
pub struct Field {
    pub name: String,
    pub is_static: bool,
    pub is_const: bool,
    pub type_name: Option<String>,
    /// the declared starting value
    pub default: Option<Expr>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MethodKind { Method, Getter, Setter }

#[derive(Clone)]
pub struct MethodModel {
    pub name: String,
    pub kind: MethodKind,
    pub is_static: bool,
    pub function: Function,
}

/// A frame script: the method a clip runs when its playhead enters `frame` (0-based, as the
/// registration states it).
#[derive(Clone, Debug, PartialEq)]
pub struct FrameScript {
    pub frame: u32,
    pub method: String,
}

/// A sprite's timeline: what each frame shows and where its labels are.
pub struct SpriteModel {
    pub id: u16,
    pub frame_count: usize,
    /// (0-based frame, label)
    pub labels: Vec<(usize, String)>,
    /// every frame's display list
    pub frames: Vec<Vec<Placed>>,
}

impl SourceModel {
    /// A package with nothing in it.
    pub fn empty() -> SourceModel {
        SourceModel { classes: BTreeMap::new(), symbols: BTreeMap::new(), sprites: BTreeMap::new(), bitmaps: BTreeMap::new(),
                      root: SpriteModel { id: 0, frame_count: 0, labels: vec![], frames: vec![] }, scripts: vec![] }
    }

    /// Read a package (a `.ssf` or a plain `.swf`).
    pub fn read(bytes: &[u8]) -> Result<SourceModel> {
        // shipped content is a wrapped SWF; the engine's own movie is a plain one
        let raw = crate::ssf::decompress(bytes).unwrap_or_else(|_| bytes.to_vec());
        let buf = swf::decompress_swf(&raw[..])?;
        let swf = swf::parse_swf(&buf)?;

        let mut symbols = BTreeMap::new();
        let mut blocks: Vec<Vec<u8>> = Vec::new();
        let mut sprites = BTreeMap::new();
        let mut bitmaps = BTreeMap::new();
        for tag in &swf.tags {
            match tag {
                swf::Tag::SymbolClass(links) => for l in links {
                    symbols.insert(l.id, l.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
                },
                swf::Tag::DoAbc(data) => blocks.push(data.to_vec()),
                swf::Tag::DoAbc2(abc) => blocks.push(abc.data.to_vec()),
                swf::Tag::DefineSprite(s) => { sprites.insert(s.id, read_sprite(s.id, &s.tags)); }
                swf::Tag::DefineBitsLossless(b) => { bitmaps.insert(b.id, (b.width as u32, b.height as u32)); }
                swf::Tag::DefineBitsJpeg3(j) => { bitmaps.insert(j.id, image_size(j.data)); }
                swf::Tag::DefineBitsJpeg2 { id, jpeg_data } => { bitmaps.insert(*id, image_size(jpeg_data)); }
                swf::Tag::DefineBits { id, .. } => { bitmaps.insert(*id, (0, 0)); }
                _ => {}
            }
        }
        let root = read_sprite(0, &swf.tags);
        let bound: BTreeMap<&str, u16> = symbols.iter().map(|(id, c)| (c.as_str(), *id)).collect();

        let mut classes = BTreeMap::new();
        let mut scripts = Vec::new();
        for (bi, block) in blocks.iter().enumerate() {
            let Ok(abc) = swf::avm2::read::Reader::new(block).read() else { continue };
            let pool = Pool { abc: &abc };
            let global: BTreeMap<u32, String> = abc.scripts.iter()
                .flat_map(|s| decode::slot_names(&pool, &s.traits)).collect();
            for s in &abc.scripts {
                let slots = Slots { this: BTreeMap::new(), global: global.clone() };
                scripts.push(function(&pool, s.init_method.0, &slots));
            }
            for (ci, inst) in abc.instances.iter().enumerate() {
                let mut c = read_class(&pool, &abc, ci, bi, &global);
                c.symbol = bound.get(c.name.as_str()).copied();
                let _ = inst;
                classes.insert(c.name.clone(), c);
            }
        }
        Ok(SourceModel { classes, symbols, sprites, bitmaps, root, scripts })
    }

    /// The class bound to a symbol.
    pub fn class_of(&self, symbol: u16) -> Option<&ClassModel> {
        self.symbols.get(&symbol).and_then(|n| self.classes.get(n))
    }

    /// A class's superclasses, nearest first, as far as the package defines them; the last name
    /// is the first one the package does not define (the engine's own base).
    pub fn supers(&self, class: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = self.classes.get(class).map(|c| c.super_name.clone());
        while let Some(s) = cur {
            if s.is_empty() || out.contains(&s) { break }
            out.push(s.clone());
            cur = self.classes.get(&s).map(|c| c.super_name.clone());
        }
        out
    }
}

fn read_sprite(id: u16, tags: &[swf::Tag]) -> SpriteModel {
    let mut labels = Vec::new();
    let mut frame = 0usize;
    for t in tags {
        match t {
            swf::Tag::FrameLabel(l) => labels.push((frame, l.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string())),
            swf::Tag::ShowFrame => frame += 1,
            _ => {}
        }
    }
    let frames = swf_timeline::frames(tags);
    SpriteModel { id, frame_count: frames.len(), labels, frames }
}

/// Decompile one method into a function: its signature and its body, decoded, structured and
/// tidied.
pub fn function(pool: &Pool, method: u32, slots: &Slots) -> Function {
    let m = &pool.abc.methods[method as usize];
    let (params, rest, return_type) = decode::signature(pool, m);
    let activation = m.body.and_then(|b| pool.abc.method_bodies.get(b.0 as usize))
        .map(|b| decode::slot_names(pool, &b.traits).into_iter().collect()).unwrap_or_default();
    let mut nested = |i: u32| function(pool, i, slots);
    // debug (`PEPTIDE_MODEL_TRACE=1`): each method as it is decompiled, and how long each stage took
    let trace = std::env::var("PEPTIDE_MODEL_TRACE").is_ok();
    let body = if m.body.is_none() { Vec::new() } else {
        let t0 = std::time::Instant::now();
        if trace { eprintln!("[model] method {method} {}", pool.string(m.name.0)); }
        match decode::decode_body(pool, method, slots, &mut nested) {
            Ok(d) => {
                let t1 = std::time::Instant::now();
                if trace { eprintln!("[model]   {} blocks, decoded in {:?}", d.blocks.len(), t1 - t0); }
                if std::env::var("PEPTIDE_MODEL_BLOCKS").is_ok_and(|w| w == method.to_string()) {
                    for b in d.blocks.values() {
                        let mut t = String::new();
                        ast::print_stmts(&mut t, &b.stmts, 1);
                        eprintln!("[block {} depth {}]\n{t}  -> {:?}", b.start, b.depth, b.term);
                    }
                }
                let s = structure::structure(d);
                let t2 = std::time::Instant::now();
                if trace { eprintln!("[model]   structured in {:?}", t2 - t1); }
                let out = tidy::tidy(s);
                if trace { eprintln!("[model]   decode {:?} structure {:?} tidy {:?}", t1 - t0, t2 - t1, t2.elapsed()); }
                out
            }
            Err(e) => vec![Stmt::Unreadable(e)],
        }
    };
    Function { params, rest, return_type, local_names: decode::local_names(pool, method), activation, body }
}

fn read_class(pool: &Pool, abc: &AbcFile, ci: usize, block: usize, global: &BTreeMap<u32, String>) -> ClassModel {
    let inst = &abc.instances[ci];
    let class = &abc.classes[ci];
    let name = pool.class_name(inst.name.0);
    let super_name = if inst.super_name.0 == 0 { String::new() } else { pool.class_name(inst.super_name.0) };
    let this_slots = Slots { this: decode::slot_names(pool, &inst.traits), global: global.clone() };
    let static_slots = Slots { this: decode::slot_names(pool, &class.traits), global: global.clone() };
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    for (traits, is_static, slots) in [(&inst.traits, false, &this_slots), (&class.traits, true, &static_slots)] {
        for t in traits {
            let tname = pool.name(t.name.0);
            match &t.kind {
                TraitKind::Slot { type_name, value, .. } | TraitKind::Const { type_name, value, .. } => fields.push(Field {
                    name: tname, is_static, is_const: matches!(t.kind, TraitKind::Const { .. }),
                    type_name: pool.type_name(type_name.0),
                    default: value.as_ref().map(|v| pool.default_value(v)),
                }),
                TraitKind::Method { method, .. } => methods.push(MethodModel { name: tname, kind: MethodKind::Method, is_static, function: function(pool, method.0, slots) }),
                TraitKind::Getter { method, .. } => methods.push(MethodModel { name: tname, kind: MethodKind::Getter, is_static, function: function(pool, method.0, slots) }),
                TraitKind::Setter { method, .. } => methods.push(MethodModel { name: tname, kind: MethodKind::Setter, is_static, function: function(pool, method.0, slots) }),
                _ => {}
            }
        }
    }
    let constructor = function(pool, inst.init_method.0, &this_slots);
    let static_init = function(pool, class.init_method.0, &static_slots);
    let frame_scripts = frame_scripts_in(&constructor.body);
    let mut unreadable = Vec::new();
    for f in methods.iter().map(|m| &m.function).chain([&constructor, &static_init]) {
        unreadable.extend(ast::unreadable(&f.body));
    }
    ClassModel { name, super_name, block, symbol: None, fields, methods, constructor, static_init, frame_scripts, unreadable }
}

/// The frame scripts a constructor registers: each `addFrameScript(index, method, ...)` call on
/// the object itself, read as the pairs it passes.
pub fn frame_scripts_in(body: &[Stmt]) -> Vec<FrameScript> {
    let mut out = Vec::new();
    for s in body {
        let Stmt::Expr(Expr::Call(callee, args)) = s else { continue };
        let own = match &**callee {
            Expr::Name(n) => n == "addFrameScript",
            Expr::Member(o, n) => n == "addFrameScript" && matches!(**o, Expr::This),
            _ => false,
        };
        if !own { continue }
        for pair in args.chunks(2) {
            let [frame, m] = pair else { continue };
            let frame = match frame { Expr::Int(i) => *i as u32, Expr::Num(n) => *n as u32, _ => continue };
            let method = match m {
                Expr::Member(o, n) if matches!(**o, Expr::This) => n.clone(),
                Expr::Name(n) => n.clone(),
                _ => continue,
            };
            out.push(FrameScript { frame, method });
        }
    }
    out
}

/// an encoded image's size, read from its header
fn image_size(data: &[u8]) -> (u32, u32) {
    image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()
        .and_then(|r| r.into_dimensions().ok()).unwrap_or((0, 0))
}
