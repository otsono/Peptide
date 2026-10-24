//! Every runtime script parses. The runtime is inlined into every converted script, and the
//! engine drops a script it cannot parse whole, so one stray brace in `runtime/` takes down every
//! fighter and stage at once while converting cleanly. Parsed with the Haxe toolchain
//! (`tools/hxcheck`) where it is installed; skips cleanly where it is not.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[test]
fn runtime_scripts_parse() {
    let root = repo_root();
    let runtime = root.join("crates/ssf2-converter/runtime");
    let check = root.join("tools/hxcheck");
    let out = match Command::new("haxe")
        .arg("-cp").arg(&check)
        .arg("--macro").arg(format!("HxCheck.run('{}')", runtime.display()))
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("haxe not installed: runtime parse check skipped");
            return;
        }
    };
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let summary = text.lines().find(|l| l.contains("that do not parse")).unwrap_or("");
    assert!(summary.contains(" 0 that do not parse"), "runtime scripts that do not parse:\n{text}");
}
