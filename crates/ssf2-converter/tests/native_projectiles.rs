//! A projectile whose own code has a plain Fraymakers form is written as that code, with none of
//! the runtime: link's arrow (a charge read off its owner, four listeners, a jump by label, a
//! sound, its end) is the case every row of `runtime/native_calls.jsonc` started from. Skips
//! cleanly without the corpus.

mod common;

#[test]
fn links_arrow_is_plain_fraymakers_code() {
    let Some(dir) = common::shared_conversion("link") else { return };
    let link = dir.join("link");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(link.join("runtime_report.json")).unwrap()).unwrap();
    let native: Vec<&str> = report["native"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
    assert!(native.contains(&"link_arrow"), "the arrow is on the runtime: {}", report["on_runtime"]);
    let script = std::fs::read_to_string(link.join("library/scripts/Projectile/LinkArrowScript.hx")).unwrap();
    assert!(script.lines().count() < 250, "the arrow's script is {} lines", script.lines().count());
    for runtime in ["PeptideRuntime", "AS3.", "DATA."] {
        assert!(!script.contains(runtime), "the arrow's script reaches the runtime ({runtime})");
    }
    // its own code, as the engine's calls
    for call in ["self.updateHitboxStats(0, { damage: dmg.get() })", "playStart(\"vanish\")",
                 "self.addEventListener(EntityEvent.COLLIDE_FLOOR, toContinue, { persistent: true })"] {
        assert!(script.contains(call), "the arrow's script has no `{call}`");
    }
}
