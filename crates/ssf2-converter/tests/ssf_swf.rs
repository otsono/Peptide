//! Tests for the lowest-level format primitives:
//!   - `ssf::decompress` (the SSF wrapper unwrapper)
//!   - `swf_parser::parse` (smoke test against the bundled sandbag input)

use ssf2_converter::ssf;
use ssf2_converter::swf_parser;

// Location of the sibling test-input directory. Many of these tests are
// optional — they no-op if the input files aren't present (a fresh checkout
// on a machine without the SSF2 roster).
fn ssfs_dir() -> std::path::PathBuf {
    // ssf2-ssfs/ is a sibling of the repo root; the crate sits two levels below.
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().and_then(|p| p.parent())
        .unwrap_or(std::path::Path::new("."))
        .join("ssf2-ssfs")
}

#[test]
fn ssf_decompress_rejects_too_short_input() {
    let r = ssf::decompress(&[]);
    assert!(r.is_err(), "empty input must fail");
    let r2 = ssf::decompress(&[0x46, 0x57, 0x53]); // FWS but only 3 bytes
    // 3 bytes is exactly enough to match the FWS magic, so passthrough succeeds.
    // (Anything <3 bytes is rejected above; that's the contract.)
    assert!(r2.is_ok(), "raw 3-byte FWS magic should pass through");
}

#[test]
fn ssf_decompress_passes_through_raw_fws() {
    // A "raw SWF" starting with FWS magic is supposed to be returned as-is.
    let raw = vec![0x46, 0x57, 0x53, 0x06, 0xab, 0xcd]; // FWS + garbage tail
    let r = ssf::decompress(&raw).expect("FWS passthrough should not error");
    assert_eq!(r, raw, "FWS-prefixed input should round-trip unchanged");
}

#[test]
fn ssf_decompress_passes_through_raw_cws() {
    // CWS = zlib-compressed SWF. The SSF unwrapper recognises the magic and
    // hands it back; the swf crate then handles the actual decompression.
    let raw = vec![0x43, 0x57, 0x53, 0x06, 0xff];
    let r = ssf::decompress(&raw).expect("CWS passthrough should not error");
    assert_eq!(r, raw);
}

#[test]
fn ssf_decompress_passes_through_raw_zws() {
    // ZWS = LZMA-compressed SWF.
    let raw = vec![0x5a, 0x57, 0x53, 0x06, 0xff];
    let r = ssf::decompress(&raw).expect("ZWS passthrough should not error");
    assert_eq!(r, raw);
}

#[test]
fn ssf_decompress_rejects_malformed_wrapper() {
    // Eight bytes with a garbage_header_size that overflows the buffer.
    let bad = vec![0x10, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff];
    let r = ssf::decompress(&bad);
    assert!(r.is_err(), "header_size beyond end of file must error");
}

// ─── End-to-end SWF parse (requires sandbag.ssf) ─────────────────────────

#[test]
fn parse_sandbag_swf_produces_abc_blocks_and_symbols() {
    let path = ssfs_dir().join("sandbag.ssf");
    if !path.exists() {
        eprintln!("skip: sandbag.ssf not available at {}", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read sandbag.ssf");
    let swf_bytes = ssf::decompress(&bytes).expect("unwrap SSF");
    let parsed = swf_parser::parse(&swf_bytes).expect("parse SWF");
    assert!(parsed.frame_rate > 0.0, "frame_rate must be set");
    assert!(!parsed.symbols.is_empty(),
        "SWF must carry at least one SymbolClass mapping");
    assert!(!parsed.abc_blocks.is_empty(),
        "SWF must carry at least one ABC block");
    // The character class name `sandbag` should be in the symbol map.
    let has_sandbag = parsed.symbols.values()
        .any(|s| s.eq_ignore_ascii_case("sandbag"));
    assert!(has_sandbag,
        "sandbag SWF must register a `sandbag` SymbolClass; got: {:?}",
        parsed.symbols.values().take(8).collect::<Vec<_>>());
}
