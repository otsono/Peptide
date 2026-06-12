//! Exploration: walk an SSF2 stage `.ssf` accumulating the FULL affine matrix down
//! the placement tree, and print each named/geometry instance's WORLD axis-aligned
//! bounding box (the shape's local bounds transformed by the compounded matrix). This
//! is the ground-truth view phase 2's stage_parser is built on.
use ssf2_converter::ssf;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
struct Mat { a: f64, b: f64, c: f64, d: f64, tx: f64, ty: f64 }
impl Mat {
    fn id() -> Mat { Mat { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 } }
    fn mul(&self, o: &Mat) -> Mat {
        Mat {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            tx: self.a * o.tx + self.c * o.ty + self.tx,
            ty: self.b * o.tx + self.d * o.ty + self.ty,
        }
    }
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_stage_tree <file.ssf>");
    let raw = std::fs::read(&path).expect("read file");
    let swf_data = ssf::decompress(&raw).expect("ssf decompress");
    let buf = swf::decompress_swf(&swf_data[..]).expect("decompress_swf");
    let swf = swf::parse_swf(&buf).expect("parse_swf");

    let mut sym_names: BTreeMap<u16, String> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::SymbolClass(links) = tag {
            for link in links {
                let n = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                if !n.is_empty() { sym_names.insert(link.id, n); }
            }
        }
    }
    // id -> (xmin,ymin,xmax,ymax) px
    let mut shape_bounds: BTreeMap<u16, (f64, f64, f64, f64)> = BTreeMap::new();
    let mut sprites: BTreeMap<u16, &Vec<swf::Tag>> = BTreeMap::new();
    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineShape(s) => {
                let b = &s.shape_bounds;
                shape_bounds.insert(s.id, (b.x_min.get() as f64/20.0, b.y_min.get() as f64/20.0, b.x_max.get() as f64/20.0, b.y_max.get() as f64/20.0));
            }
            swf::Tag::DefineSprite(s) => { sprites.insert(s.id, &s.tags); }
            _ => {}
        }
    }
    println!("symbols={} shapes={} sprites={}", sym_names.len(), shape_bounds.len(), sprites.len());
    walk(&swf.tags, Mat::id(), &sym_names, &shape_bounds, &sprites, 0, None);
}

#[allow(clippy::too_many_arguments)]
fn walk(
    tags: &[swf::Tag], parent: Mat,
    sym_names: &BTreeMap<u16, String>,
    shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    rec: usize, inst_name: Option<&str>,
) {
    if rec > 8 { return; }
    for tag in tags {
        if let swf::Tag::PlaceObject(po) = tag {
            let id = match &po.action {
                swf::PlaceObjectAction::Place(id) | swf::PlaceObjectAction::Replace(id) => *id,
                swf::PlaceObjectAction::Modify => continue,
            };
            let local = po.matrix.as_ref().map(|m| Mat {
                a: m.a.to_f64(), b: m.b.to_f64(), c: m.c.to_f64(), d: m.d.to_f64(),
                tx: m.tx.get() as f64/20.0, ty: m.ty.get() as f64/20.0,
            }).unwrap_or(Mat::id());
            let world = parent.mul(&local);
            let name = po.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
            let sym = sym_names.get(&id).cloned().unwrap_or_default();

            if let Some((x0,y0,x1,y1)) = shape_bounds.get(&id) {
                // world AABB of the shape's 4 corners
                let corners = [world.apply(*x0,*y0), world.apply(*x1,*y0), world.apply(*x1,*y1), world.apply(*x0,*y1)];
                let xs: Vec<f64> = corners.iter().map(|c| c.0).collect();
                let ys: Vec<f64> = corners.iter().map(|c| c.1).collect();
                let (xmn,xmx) = (xs.iter().cloned().fold(f64::MAX,f64::min), xs.iter().cloned().fold(f64::MIN,f64::max));
                let (ymn,ymx) = (ys.iter().cloned().fold(f64::MAX,f64::min), ys.iter().cloned().fold(f64::MIN,f64::max));
                let label = inst_name.unwrap_or(if name.is_some() { name.as_deref().unwrap() } else { &sym });
                println!("AABB inst={:?} sym='{}' | x[{:.1},{:.1}] y[{:.1},{:.1}] w={:.1} h={:.1}",
                    label, sym, xmn, xmx, ymn, ymx, xmx-xmn, ymx-ymn);
            }
            if let Some(child) = sprites.get(&id) {
                let nm = name.as_deref().or(inst_name);
                // prefer the SWF symbol class name as the carried label when present (terrain children)
                let carried = if !sym.is_empty() { Some(sym.as_str()) } else { nm };
                walk(child, world, sym_names, shape_bounds, sprites, rec+1, carried);
            }
        }
    }
}
