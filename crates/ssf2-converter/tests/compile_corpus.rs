//! The compiler writes every class of every package in the corpus with nothing it could not
//! write, and what it writes parses: checked with the Haxe toolchain's own parser when one is
//! installed (`tools/hxcheck`), skipped when not. Skips cleanly without the corpus.

mod common;

use ssf2_converter::compile::{self, Runtime};

#[test]
fn every_class_compiles_and_parses() {
    let dir = common::ssfs_dir();
    if !common::present(&dir) { return }
    let runtime = Runtime::locate().map(|d| Runtime::load(&d)).unwrap_or_default();
    let out_dir = std::env::temp_dir().join(format!("peptide-compile-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut problems: Vec<String> = Vec::new();
    let mut classes = 0;
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ssf")).collect();
    files.sort();
    for path in &files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let (model, renamed) = runtime.read_package(&std::fs::read(path).unwrap()).unwrap();
        problems.extend(renamed.iter().map(|p| format!("{name}: {p}")));
        let out = compile::compile_package(&model, &runtime);
        classes += out.classes.len();
        problems.extend(out.problems.iter().map(|p| format!("{name}: {p}")));
        let text: String = out.classes.values().cloned().collect::<Vec<_>>().join("\n");
        std::fs::write(out_dir.join(format!("{name}.hx")), text).unwrap();
    }
    for (rel, text) in &runtime.files {
        std::fs::write(out_dir.join(format!("runtime_{}", rel.replace('/', "_"))), text).unwrap();
    }
    std::fs::write(out_dir.join("lang_suite.hx"), compile::suite::language_suite(&runtime)).unwrap();
    eprintln!("{} packages, {classes} classes compiled", files.len());
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.iter().take(30).cloned().collect::<Vec<_>>().join("\n"));

    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let check = std::process::Command::new("haxe")
        .arg("-cp").arg(repo.join("tools/hxcheck"))
        .arg("--macro").arg(format!("HxCheck.run('{}')", out_dir.display()))
        .output();
    match check {
        Err(_) => eprintln!("skip: no haxe on PATH, so the parse check did not run"),
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            assert!(text.contains(" 0 that do not parse"), "parse check:\n{text}{}", String::from_utf8_lossy(&o.stderr));
        }
    }
    let _ = std::fs::remove_dir_all(&out_dir);
}
