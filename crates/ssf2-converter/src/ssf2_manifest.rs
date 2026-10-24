//! SSF2's own content inventory: what exists, what each thing IS, and which file holds it.
//!
//! GROUND ZERO for a conversion. The game does not infer what a package is from its name, it
//! carries an inventory with the type stated, so reading that inventory needs no vocabulary and
//! cannot miss content whose author picked words we do not know.
//!
//! The decode chain, the cipher's quirks, and the type vocabulary are recorded in
//! `docs/SSF2_MANIFEST_INTERNALS.local.md`, which is deliberately untracked: detailed SSF2
//! internals do not belong in the repository, the same rule that applies to Fraymakers.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The decode keys are NOT carried here. They belong to the installed game, so they are recovered
/// from it: the movie's own constant pool is scanned for the shape a key has, and each candidate is
/// tried until one produces valid JSON. That keeps nothing of the game's in this repository, and it
/// survives the keys changing in a later build.
fn key_candidates(abc_blocks: &[Vec<u8>]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for blk in abc_blocks {
        let Ok(abc) = crate::abc_parser::parse(blk) else { continue };
        for s in &abc.strings {
            let t: &str = s;
            // the shape: 8-4-4-4-12 hex with dashes
            let parts: Vec<&str> = t.split('-').collect();
            if parts.len() == 5
                && [8usize, 4, 4, 4, 12] == parts.iter().map(|p| p.len()).collect::<Vec<_>>()[..]
                && t.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-')
            {
                out.push(t.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Try every candidate key and keep the one that yields JSON.
fn decode_with_any_key(b64_text: &str, keys: &[String]) -> Result<serde_json::Value> {
    for k in keys {
        if let Ok(v) = decode_asset(b64_text, k) {
            if v.is_object() { return Ok(v); }
        }
    }
    Err(anyhow!("no key in the movie's own constants decoded this asset"))
}

/// One thing the game can load, exactly as the manifest states it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Entry {
    /// The package's own filename (`lucario.ssf`).
    #[serde(default)]
    pub file: String,
    /// What it actually ships as (`DAT59.ssf`), which is what is on disk.
    #[serde(default, rename = "file_pub")]
    pub file_pub: String,
    #[serde(default)]
    pub guid: String,
    /// Display name, where the manifest carries one.
    #[serde(default)]
    pub name: String,
}

/// The inventory, by content type. Keys are CONTENT IDS (`lucario`, `finaldestination`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inventory {
    #[serde(default)]
    pub character: BTreeMap<String, Entry>,
    #[serde(default)]
    pub stage: BTreeMap<String, Entry>,
    #[serde(default)]
    pub misc: BTreeMap<String, Entry>,
    #[serde(default)]
    pub extra: BTreeMap<String, Entry>,
    #[serde(default)]
    pub modes: BTreeMap<String, Entry>,
    #[serde(default)]
    pub menu: BTreeMap<String, Entry>,
}

fn b64_decode(text: &str) -> Result<Vec<u8>> {
    let val = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let (mut acc, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for c in text.bytes() {
        let Some(v) = val(c) else { continue }; // whitespace and '=' padding
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// Base64, then subtract the cycling key. Both quirks below are reproduced rather than corrected,
/// because the game's output is what has to be matched. See the local notes.
fn decode_step(b64_text: &str, key: &str) -> Result<Vec<u8>> {
    let decoded = b64_decode(b64_text.trim())?;
    let chars: Vec<u8> = String::from_utf8(decoded)
        .map(|s| s.chars().map(|c| c as u32 as u8).collect())
        .unwrap_or_default();
    let kb: Vec<u8> = key.bytes().collect();
    let n = kb.len();
    Ok(chars.iter().enumerate()
        .map(|(i, c)| {
            let idx = if i % n == 0 { n - 1 } else { (i % n) - 1 };
            c.wrapping_sub(kb[idx])
        })
        .collect())
}

/// Decode one of the embedded JSON assets.
fn decode_asset(b64_text: &str, key: &str) -> Result<serde_json::Value> {
    let stage = decode_step(b64_text, key)?;
    let json = b64_decode(String::from_utf8_lossy(&stage).trim())?;
    serde_json::from_slice(&json).context("decoded manifest is not JSON")
}

/// The payload of the embedded asset whose class name contains `want`.
fn embedded_asset(swf_bytes: &[u8], want: &str) -> Result<String> {
    let buf = swf::decompress_swf(swf_bytes).map_err(|e| anyhow!("decompress: {e}"))?;
    let parsed = swf::parse_swf(&buf).map_err(|e| anyhow!("parse: {e}"))?;
    let mut id_for_class: Option<u16> = None;
    let mut blobs: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
    for tag in &parsed.tags {
        match tag {
            swf::Tag::SymbolClass(links) => {
                for l in links {
                    let name = l.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                    if name.contains(want) { id_for_class = Some(l.id); }
                }
            }
            swf::Tag::DefineBinaryData(b) => { blobs.insert(b.id, b.data.to_vec()); }
            _ => {}
        }
    }
    let id = id_for_class.ok_or_else(|| anyhow!("no embedded asset class matching {want:?}"))?;
    let blob = blobs.get(&id).ok_or_else(|| anyhow!("asset {want:?} has no binary data"))?;
    Ok(String::from_utf8_lossy(blob).to_string())
}

/// Read the whole inventory, following the boot manifest to the package that holds the rest.
/// `data_dir` is the game's data directory, where the shipped packages live.
pub fn read(ssf2_swf: &Path, data_dir: &Path) -> Result<Inventory> {
    let swf_bytes = std::fs::read(ssf2_swf)
        .with_context(|| format!("reading {}", ssf2_swf.display()))?;
    let boot_txt = embedded_asset(&swf_bytes, "manifestJSON")?;
    let keys = key_candidates(&crate::swf_parser::parse(&swf_bytes)?.abc_blocks);
    let boot = decode_with_any_key(&boot_txt, &keys)?;

    let mut inv: Inventory = serde_json::from_value(boot.clone()).unwrap_or_default();

    // The boot manifest carries no characters or stages: those are declared by the package it
    // names under `misc.mappings`, which is where the real inventory lives.
    let mapping_file = boot.get("misc")
        .and_then(|m| m.get("mappings"))
        .and_then(|m| m.get("file_pub").or_else(|| m.get("file")))
        .and_then(|f| f.as_str())
        .ok_or_else(|| anyhow!("boot manifest declares no misc.mappings package"))?;
    let mappings_path = data_dir.join(mapping_file);
    let raw = std::fs::read(&mappings_path)
        .with_context(|| format!("reading mappings package {}", mappings_path.display()))?;
    // a shipped package is a zlib stream around the movie
    let inner = inflate(&raw).unwrap_or(raw);
    let meta_txt = embedded_asset_scan(&inner)?;
    // the package's key is its own, so its constants are searched too
    let mut pkg_keys = keys.clone();
    if let Ok(parsed) = crate::swf_parser::parse(&inner) {
        pkg_keys.extend(key_candidates(&parsed.abc_blocks));
    }
    let meta = decode_with_any_key(&meta_txt, &pkg_keys)?;
    let full: Inventory = serde_json::from_value(meta).unwrap_or_default();
    if !full.character.is_empty() { inv.character = full.character; }
    if !full.stage.is_empty() { inv.stage = full.stage; }
    if !full.extra.is_empty() { inv.extra = full.extra; }
    Ok(inv)
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).ok()?;
    Some(out)
}

/// The mappings package does not present as a plain movie, so its embedded asset is found by its
/// SHAPE rather than by walking tags: one very long run of base64 text.
fn embedded_asset_scan(data: &[u8]) -> Result<String> {
    let mut best: (usize, usize) = (0, 0); // (start, len)
    let (mut start, mut len) = (0usize, 0usize);
    let is_b64 = |c: u8| c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'=';
    for (i, &c) in data.iter().enumerate() {
        if is_b64(c) {
            if len == 0 { start = i; }
            len += 1;
        } else {
            if len > best.1 { best = (start, len); }
            len = 0;
        }
    }
    if len > best.1 { best = (start, len); }
    if best.1 < 2000 { return Err(anyhow!("no embedded JSON asset found in mappings package")); }
    Ok(String::from_utf8_lossy(&data[best.0..best.0 + best.1]).to_string())
}
