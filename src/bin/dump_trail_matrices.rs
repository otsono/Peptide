/// Dump the raw PlaceObject matrix for a specific shape ID across all frames
/// of a given sprite ID in an SSF file.
/// Usage: dump_trail_matrices <ssf_file> <sprite_id> <shape_id>

use std::collections::HashMap;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: dump_trail_matrices <ssf_file> <sprite_id> <shape_id>");
        std::process::exit(1);
    }
    let ssf_path = &args[1];
    let target_sprite_id: u16 = args[2].parse().unwrap();
    let target_shape_id: u16 = args[3].parse().unwrap();

    let data = std::fs::read(ssf_path).expect("read ssf");
    // strip 4-byte SSF header
    let swf_data = if &data[0..4] == b"SSF\0" || &data[0..3] == b"SSF" {
        &data[4..]
    } else {
        &data[..]
    };
    let swf_buf = swf::decompress_swf(swf_data).expect("decompress");
    let swf = swf::parse_swf(&swf_buf).expect("parse");

    // Find the sprite
    for tag in &swf.tags {
        if let swf::Tag::DefineSprite(sprite) = tag {
            if sprite.id != target_sprite_id { continue; }

            println!("Sprite id={} ({} frames)", sprite.id, sprite.num_frames);
            println!("Looking for shape id={} ...\n", target_shape_id);

            // Simulate display list
            let mut display_list: HashMap<u16, (u16, swf::Matrix)> = HashMap::new(); // depth -> (char_id, matrix)
            let mut frame = 0u32;

            for tag in &sprite.tags {
                match tag {
                    swf::Tag::ShowFrame => { frame += 1; }
                    swf::Tag::PlaceObject(po) => {
                        let depth = po.depth;
                        match po.action {
                            swf::PlaceObjectAction::Place(char_id) => {
                                let mat = po.matrix.unwrap_or(swf::Matrix::IDENTITY);
                                display_list.insert(depth, (char_id, mat));
                                if char_id == target_shape_id {
                                    let m = &mat;
                                    println!("[f{:2}] PLACE depth={} a={:.4} b={:.4} c={:.4} d={:.4} tx={:.2} ty={:.2}",
                                        frame, depth,
                                        m.a.to_f64(), m.b.to_f64(), m.c.to_f64(), m.d.to_f64(),
                                        m.tx.get() as f64 / 20.0, m.ty.get() as f64 / 20.0);
                                }
                            }
                            swf::PlaceObjectAction::Modify => {
                                if let Some(entry) = display_list.get_mut(&depth) {
                                    if let Some(mat) = po.matrix {
                                        entry.1 = mat;
                                    }
                                    if entry.0 == target_shape_id {
                                        let m = &entry.1;
                                        println!("[f{:2}] MODIFY depth={} a={:.4} b={:.4} c={:.4} d={:.4} tx={:.2} ty={:.2}",
                                            frame, depth,
                                            m.a.to_f64(), m.b.to_f64(), m.c.to_f64(), m.d.to_f64(),
                                            m.tx.get() as f64 / 20.0, m.ty.get() as f64 / 20.0);
                                    }
                                }
                            }
                            swf::PlaceObjectAction::Replace(char_id) => {
                                let mat = po.matrix.unwrap_or(swf::Matrix::IDENTITY);
                                display_list.insert(depth, (char_id, mat));
                                if char_id == target_shape_id {
                                    let m = &mat;
                                    println!("[f{:2}] REPLACE depth={} a={:.4} b={:.4} c={:.4} d={:.4} tx={:.2} ty={:.2}",
                                        frame, depth,
                                        m.a.to_f64(), m.b.to_f64(), m.c.to_f64(), m.d.to_f64(),
                                        m.tx.get() as f64 / 20.0, m.ty.get() as f64 / 20.0);
                                }
                            }
                        }
                    }
                    swf::Tag::RemoveObject(ro) => {
                        if let Some(entry) = display_list.get(&ro.depth) {
                            if entry.0 == target_shape_id {
                                println!("[f{:2}] REMOVE depth={}", frame, ro.depth);
                            }
                        }
                        display_list.remove(&ro.depth);
                    }
                    _ => {}
                }
            }
            return;
        }
    }
    println!("Sprite {} not found", target_sprite_id);
}
