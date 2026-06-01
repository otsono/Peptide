use ssf2_converter::*;
use std::env;
use std::io::Cursor;

fn main() {
    let args: Vec<String> = env::args().collect();
    let target: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(192);
    
    let ssf_data = std::fs::read(&args[1]).unwrap();
    let swf_buf = swf::decompress_swf(Cursor::new(&ssf_data)).unwrap();
    let swf = swf::parse_swf(&swf_buf).unwrap();

    for tag in &swf.tags {
        if let swf::Tag::DefineShape(shape) = tag {
            if shape.id != target { continue; }
            println!("DefineShape id={}", shape.id);
            for (i, fill) in shape.styles.fill_styles.iter().enumerate() {
                let kind = match fill {
                    swf::FillStyle::Color(_) => "Color",
                    swf::FillStyle::LinearGradient(_) => "LinearGradient",
                    swf::FillStyle::RadialGradient(_) => "RadialGradient",
                    swf::FillStyle::Bitmap { id, matrix, .. } => {
                        let tx = matrix.tx.get() as f64 / 20.0;
                        let ty = matrix.ty.get() as f64 / 20.0;
                        let a = matrix.a.to_f64();
                        let b = matrix.b.to_f64();
                        let c = matrix.c.to_f64();
                        let d = matrix.d.to_f64();
                        println!("  fill[{}]: Bitmap id={} matrix: a={:.3} b={:.3} c={:.3} d={:.3} tx={:.2} ty={:.2}", i, id, a, b, c, d, tx, ty);
                        continue;
                    }
                    _ => "Other",
                };
                println!("  fill[{}]: {}", i, kind);
            }
        }
    }
}
