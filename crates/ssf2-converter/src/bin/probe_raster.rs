//! Rasterize specific DefineShape / DefineMorphShape character IDs to PNGs so
//! we can eyeball the vector rasterizer. Usage: probe_raster <file.ssf> <id...>

use ssf2_converter::vector_raster;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let ids: Vec<u16> = args[2..].iter().filter_map(|s| s.parse().ok()).collect();

    let data = ssf2_converter::ssf::decompress(&std::fs::read(path).unwrap()).unwrap();
    let buf = swf::decompress_swf(&data[..]).unwrap();
    let swf = swf::parse_swf(&buf).unwrap();

    // "list" mode: dump DefineShapes that have gradient fills (id, kind, size).
    if args.get(2).map(|s| s == "list").unwrap_or(false) {
        for tag in &swf.tags {
            if let swf::Tag::DefineShape(s) = tag {
                for f in &s.styles.fill_styles {
                    let kind = match f {
                        swf::FillStyle::LinearGradient(_) => "linear",
                        swf::FillStyle::RadialGradient(_) => "radial",
                        swf::FillStyle::FocalGradient { .. } => "focal",
                        _ => continue,
                    };
                    let w = (s.shape_bounds.x_max.to_pixels() - s.shape_bounds.x_min.to_pixels()).round();
                    let h = (s.shape_bounds.y_max.to_pixels() - s.shape_bounds.y_min.to_pixels()).round();
                    println!("shape {} : {} gradient, {}x{}", s.id, kind, w, h);
                    break;
                }
            }
        }
        return;
    }

    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineShape(s) if ids.contains(&s.id) => {
                match vector_raster::rasterize_shape(&s.shape_bounds, &s.styles.fill_styles, &s.styles.line_styles, &s.shape) {
                    Some(r) => {
                        let out = format!("/tmp/raster_shape_{}.png", s.id);
                        save(&out, r.width, r.height, &r.rgba);
                        println!("shape {} → {}  ({}x{}, origin {:?})", s.id, out, r.width, r.height, r.origin_px);
                    }
                    None => println!("shape {} → (nothing drawable)", s.id),
                }
            }
            swf::Tag::DefineMorphShape(m) if ids.contains(&m.id) => {
                for (label, ratio) in [("start", 0.0), ("mid", 0.5), ("end", 1.0)] {
                    match vector_raster::rasterize_morph(m, ratio) {
                        Some(r) => {
                            let out = format!("/tmp/raster_morph_{}_{}.png", m.id, label);
                            save(&out, r.width, r.height, &r.rgba);
                            println!("morph {} @{} → {}  ({}x{})", m.id, label, out, r.width, r.height);
                        }
                        None => println!("morph {} @{} → (nothing)", m.id, label),
                    }
                }
            }
            _ => {}
        }
    }
}

fn save(path: &str, w: u32, h: u32, rgba: &[u8]) {
    use image::{ImageBuffer, Rgba};
    let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(w, h, rgba.to_vec()).unwrap();
    img.save(path).unwrap();
}
