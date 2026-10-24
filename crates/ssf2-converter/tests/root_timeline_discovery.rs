//! A fighter's root timeline is found even when the package never declares it.
//!
//! A package states which clips it owns, and reading that beats guessing from symbol names. It
//! is not, however, a complete list: a shipped package can carry a declaration copied from
//! another fighter, so its own root is missing from it and another fighter's clips are in its
//! place. Treated as a filter, that declaration throws the real root away and the character
//! converts to a pile of empty animation stubs: no art, no boxes, invisible in the engine.
//!
//! So the declaration ranks candidates and never rejects one, and this holds that line.

use std::collections::BTreeMap;

mod common;

/// Every corpus character must resolve a root timeline with a real animation set behind it.
/// Ranging over the whole corpus (rather than naming the one package that exposed this) is the
/// point: the next package with a stale declaration is one nobody has written yet.
#[test]
#[ignore = "corpus scan: not part of the fast gate. Run with `cargo test -- --ignored`."]
fn every_corpus_character_resolves_a_root_timeline() {
    let dir = common::ssf("sandbag");
    if !common::present(&dir) { return; }
    let dir = dir.parent().unwrap().to_path_buf();

    let map: BTreeMap<String, String> = BTreeMap::new();
    let mut checked = 0usize;
    let mut thin: Vec<(String, usize)> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read corpus dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("ssf") { continue; }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        // the shipped data archives are not single-character packages
        if id.starts_with("DAT") { continue; }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(swf) = ssf2_converter::ssf::decompress(&bytes) else { continue };
        // only packages that SAY they ship a fighter; the rest of the corpus is stages, menus
        // and shared assets, and asking those for a fighter's timeline means nothing
        let Ok(parsed) = ssf2_converter::swf_parser::parse(&swf) else { continue };
        let ships_a_fighter = parsed.abc_blocks.iter().any(|blk| {
            ssf2_converter::abc_parser::parse(blk).ok()
                .and_then(|abc| ssf2_converter::abc_parser::extract_main_package_metadata(&abc))
                .is_some_and(|m| !m.characters.is_empty())
        });
        if !ships_a_fighter { continue; }
        let Ok(xframes) = ssf2_converter::sprite_parser::extract_xframe_transforms(&swf, id, &map)
            else { continue };
        checked += 1;
        // a real root places one stance per animation, so its transform map is large; a package
        // whose root was rejected yields a handful at most
        if xframes.len() < 20 { thin.push((id.to_string(), xframes.len())); }
    }
    eprintln!("checked {checked} fighter packages");
    assert!(checked > 0, "no corpus characters were read");
    assert!(thin.is_empty(),
        "these packages resolved no usable root timeline (declaration used as a filter?): {thin:?}");
}

/// How many fighters state their own art clip, versus how many leave it to the shape of the
/// timeline. Reported, not asserted: a package that does not say is allowed to exist.
#[test]
#[ignore = "corpus scan: reporting only."]
fn how_many_fighters_name_their_own_root() {
    let probe = common::ssf("sandbag");
    if !common::present(&probe) { return; }
    let dir = probe.parent().unwrap().to_path_buf();
    let (mut says, mut silent) = (Vec::new(), Vec::new());
    for entry in std::fs::read_dir(&dir).expect("read corpus dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("ssf") { continue; }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(swf) = ssf2_converter::ssf::decompress(&bytes) else { continue };
        let Ok(parsed) = ssf2_converter::swf_parser::parse(&swf) else { continue };
        for blk in &parsed.abc_blocks {
            let Ok(abc) = ssf2_converter::abc_parser::parse(blk) else { continue };
            let Some(meta) = ssf2_converter::abc_parser::extract_main_package_metadata(&abc)
                else { continue };
            // every fighter the package says it ships, asked about its own art
            for (id, _) in &meta.characters {
                match ssf2_converter::abc_parser::declared_root_clip(&abc, &id.to_lowercase()) {
                    Some(l) => says.push(format!("{id}->{l}")),
                    None => silent.push(id.clone()),
                }
            }
        }
    }
    eprintln!("fighters naming their own root: {} / {}", says.len(), says.len() + silent.len());
    eprintln!("silent: {silent:?}");
}
