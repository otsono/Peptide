//! A fighter whose own code has a plain Fraymakers form is written as that code, with none of the
//! runtime: its own class runs as the fighter, its clips' frame scripts are its functions, and the
//! rules the source's engine plays every fighter by are `runtime/native/fighter_rules.hx`. Skips
//! cleanly without the corpus.

mod common;

fn native(name: &str) {
    let Some(dir) = common::shared_conversion(name) else { return };
    let out = dir.join(name);
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(out.join("runtime_report.json")).unwrap()).unwrap();
    assert_eq!(report["fighter_native"], serde_json::Value::Bool(true), "{name} is on the runtime: {}", report["fighter_on_runtime"]);
    let scripts = out.join("library/scripts");
    let script = std::fs::read_dir(&scripts).unwrap().flatten().map(|e| e.path().join("Script.hx")).find(|p| p.exists()).unwrap();
    let text = std::fs::read_to_string(&script).unwrap();
    for runtime in ["PeptideRuntime", "AS3.", "DATA.", "Fighters.", "Attacks."] {
        assert!(!text.contains(runtime), "{name}'s script reaches the runtime ({runtime})");
    }
    // what every fighter does, and what its own class sets up
    for part in ["function onBoxConnected(e)", "self.exports.__source = source;", "listenForHits();"] {
        assert!(text.contains(part), "{name}'s script has no `{part}`");
    }
}

#[test]
fn sandbag_is_plain_fraymakers_code() { native("sandbag") }

#[test]
fn mario_is_plain_fraymakers_code() { native("mario") }

#[test]
fn samus_is_plain_fraymakers_code() { native("samus") }

#[test]
fn link_is_plain_fraymakers_code() { native("link") }

#[test]
fn pikachu_is_plain_fraymakers_code() { native("pikachu") }

#[test]
fn fox_is_plain_fraymakers_code() { native("fox") }

#[test]
fn rayman_is_plain_fraymakers_code() { native("rayman") }
