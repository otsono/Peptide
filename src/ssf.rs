use anyhow::{anyhow, Result};
use flate2::read::ZlibDecoder;
use std::io::Read;

/// Decompress SSF to raw SWF bytes.
/// Handles three cases:
///   1. Already a raw SWF (FWS/CWS/ZWS magic) — pass through as-is
///   2. SSF-wrapped: 4-byte SWF length + 4-byte garbage header size + zlib payload
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 3 {
        return Err(anyhow!("File too small"));
    }

    // Check if it's already a raw SWF
    match [data[0], data[1], data[2]] {
        [b'F', b'W', b'S'] | [b'C', b'W', b'S'] | [b'Z', b'W', b'S'] => {
            log::debug!("File is already a raw SWF, skipping SSF decompression");
            return Ok(data.to_vec());
        }
        _ => {}
    }

    if data.len() < 8 {
        return Err(anyhow!("SSF too small (< 8 bytes)"));
    }

    let swf_len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let header_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;

    log::debug!("SSF: expected SWF length = {}, garbage header size = {}", swf_len, header_size);

    // Guard against integer overflow on 32-bit targets where usize == u32:
    // an attacker-controlled header_size near usize::MAX could wrap with
    // `8 + header_size` and land back inside the buffer.
    let compressed_start = match 8usize.checked_add(header_size) {
        Some(s) => s,
        None => return Err(anyhow!(
            "SSF header_size {} overflows when added to fixed 8-byte preamble",
            header_size
        )),
    };
    if compressed_start > data.len() {
        return Err(anyhow!("SSF header_size {} extends past end of file", header_size));
    }

    let compressed_data = &data[compressed_start..];
    log::debug!("Compressed payload: {} bytes starting at offset {}", compressed_data.len(), compressed_start);

    // Try zlib decompression first
    let mut decoder = ZlibDecoder::new(compressed_data);
    let mut swf = Vec::with_capacity(swf_len.min(64 * 1024 * 1024));
    match decoder.read_to_end(&mut swf) {
        Ok(_) => {
            log::debug!("Zlib decompressed to {} bytes (expected {})", swf.len(), swf_len);
        }
        Err(e) => {
            // Already uncompressed? Check if it starts with FWS magic
            if compressed_data.len() >= 3
                && [compressed_data[0], compressed_data[1], compressed_data[2]] == [b'F', b'W', b'S']
            {
                log::debug!("Payload already uncompressed (FWS magic)");
                swf = compressed_data.to_vec();
            } else {
                return Err(anyhow!("Failed to decompress SSF payload: {}", e));
            }
        }
    }

    // Validate SWF magic bytes
    if swf.len() < 3 {
        return Err(anyhow!("Decompressed SWF too small ({} bytes)", swf.len()));
    }
    match [swf[0], swf[1], swf[2]] {
        [b'F', b'W', b'S'] => log::debug!("SWF magic: FWS (uncompressed)"),
        [b'C', b'W', b'S'] => log::debug!("SWF magic: CWS (zlib-compressed SWF)"),
        [b'Z', b'W', b'S'] => log::debug!("SWF magic: ZWS (LZMA-compressed SWF)"),
        other => return Err(anyhow!("Invalid SWF magic bytes: {:?}", other)),
    }

    Ok(swf)
}
