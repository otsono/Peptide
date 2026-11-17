//! A projectile is written as plain Fraymakers code: link's arrow (a charge read off its owner,
//! four listeners, a jump by label, a sound, its end) is the case every row of
//! `runtime/native_calls.jsonc` started from. Skips cleanly without the corpus.

mod common;

#[test]
fn links_arrow_is_plain_fraymakers_code() {
    let Some(dir) = common::shared_conversion("link") else { return };
    let link = dir.join("link");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(link.join("port_report.json")).unwrap()).unwrap();
    let gaps = &report["projectile_gaps"]["link_arrow"];
    assert!(gaps.as_array().is_some_and(|g| g.is_empty()), "the arrow leaves something out: {gaps}");
    let script = std::fs::read_to_string(link.join("library/scripts/Projectile/LinkArrowScript.hx")).unwrap();
    assert!(script.lines().count() < 250, "the arrow's script is {} lines", script.lines().count());
    for runtime in ["PeptideRuntime", "AS3.", "DATA."] {
        assert!(!script.contains(runtime), "the arrow's script names {runtime}, which nothing defines");
    }
    // its own code, as the engine's calls
    for call in ["self.updateHitboxStats(0, { damage: dmg.get() })", "playStart(\"vanish\")",
                 "self.addEventListener(EntityEvent.COLLIDE_FLOOR, toContinue, { persistent: true })"] {
        assert!(script.contains(call), "the arrow's script has no `{call}`");
    }
}
