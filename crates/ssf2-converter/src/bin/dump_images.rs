use std::fs;
use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("usage: dump_images <file.ssf>");
    let data = fs::read(&path)?;
    
    let swf = swf::decompress_swf(&*data)?;
    let tags = swf::parse_swf(&swf)?;
    
    let mut tag_types: BTreeMap<String, usize> = BTreeMap::new();
    let mut image_count = 0;
    let mut image_ids: Vec<(u16, String, usize)> = Vec::new();
    
    for tag in &tags.tags {
        let name = format!("{:?}", tag).split('(').next().unwrap_or("?").to_string();
        *tag_types.entry(name.clone()).or_insert(0) += 1;
        
        match tag {
            swf::Tag::DefineBitsLossless(bmp) => {
                image_count += 1;
                let fmt = format!("Lossless v{} {}x{}", bmp.version, bmp.width, bmp.height);
                image_ids.push((bmp.id, fmt, bmp.data.len()));
            }
            swf::Tag::DefineBitsJpeg2 { id, jpeg_data, .. } => {
                image_count += 1;
                let fmt = "JPEG2".to_string();
                image_ids.push((*id, fmt, jpeg_data.len()));
            }
            swf::Tag::DefineBitsJpeg3(jpeg) => {
                image_count += 1;
                let fmt = format!("JPEG3");
                image_ids.push((jpeg.id, fmt, jpeg.data.len()));
            }
            _ => {}
        }
    }
    
    println!("Tag type counts:");
    for (name, count) in &tag_types {
        println!("  {}: {}", name, count);
    }
    
    println!("\nImages found: {}", image_count);
    for (id, fmt, size) in &image_ids[..image_ids.len().min(20)] {
        println!("  id={}: {} ({} bytes)", id, fmt, size);
    }
    if image_ids.len() > 20 {
        println!("  ... and {} more", image_ids.len() - 20);
    }
    
    // Check symbol table for image symbols
    let symbols: BTreeMap<u16, String> = tags.tags.iter().filter_map(|tag| {
        if let swf::Tag::SymbolClass(links) = tag {
            Some(links.iter().map(|l| (l.id, String::from_utf8_lossy(l.class_name.as_bytes()).to_string())).collect::<Vec<_>>())
        } else { None }
    }).flatten().collect();
    
    // Map image ids to symbol names
    let mut named_images = 0;
    for (id, fmt, size) in &image_ids[..image_ids.len().min(10)] {
        if let Some(sym) = symbols.get(id) {
            println!("  id={} sym='{}': {} ({} bytes)", id, sym, fmt, size);
            named_images += 1;
        }
    }
    println!("{} images have symbol names out of {}", named_images, image_ids.len());
    
    Ok(())
}
