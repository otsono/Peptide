use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwfFile {
    pub version: u8,
    pub frame_count: u16,
    pub frame_rate: f32,
    pub symbols: BTreeMap<u16, String>,
    /// Raw ABC bytecode blocks extracted from DoABC tags
    pub abc_blocks: Vec<Vec<u8>>,
}

/// Parse SWF bytes using the swf crate.
/// decompress_swf handles FWS/CWS/ZWS compression, then parse_swf builds the tag tree.
pub fn parse(data: &[u8]) -> Result<SwfFile> {
    let swf_buf = swf::decompress_swf(data)?;
    let swf = swf::parse_swf(&swf_buf)?;

    let version = swf.header.version();
    let frame_count = swf.header.num_frames();
    let frame_rate = swf.header.frame_rate().get() as f32;

    log::debug!("SWF version={}, frames={}, rate={}", version, frame_count, frame_rate);

    let mut symbols: BTreeMap<u16, String> = BTreeMap::new();
    let mut abc_blocks: Vec<Vec<u8>> = Vec::new();

    for tag in &swf.tags {
        match tag {
            // SymbolClass — maps character IDs to AS3 class names
            swf::Tag::SymbolClass(links) => {
                for link in links {
                    let name = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                    log::debug!("Symbol: id={} → {}", link.id, name);
                    symbols.insert(link.id, name);
                }
            }
            // DoAbc — raw ABC bytecode (AS3)
            swf::Tag::DoAbc(data) => {
                log::debug!("DoABC tag: {} bytes", data.len());
                abc_blocks.push(data.to_vec());
            }
            // DoAbc2 — named ABC block
            swf::Tag::DoAbc2(abc) => {
                log::debug!("DoABC2 tag: name={}, {} bytes", abc.name.to_str_lossy(encoding_rs::WINDOWS_1252), abc.data.len());
                abc_blocks.push(abc.data.to_vec());
            }
            _ => {}
        }
    }

    log::info!(
        "SWF v{}: {} tags, {} symbols, {} ABC blocks",
        version, swf.tags.len(), symbols.len(), abc_blocks.len()
    );

    Ok(SwfFile {
        version,
        frame_count,
        frame_rate,
        symbols,
        abc_blocks,
    })
}
