//! The source model reads every package in the corpus whole: every class, with nothing the
//! decompiler could not read back, every frame script naming a method its class has, and every
//! class bound to a symbol with a timeline to run on.
//!
//! This is the bar for the model being the one place the converter learns what a package is.
//! It skips cleanly without the corpus.

mod common;

use ssf2_converter::model::{ast, SourceModel};

#[test]
fn every_package_reads_whole() {
    let dir = common::ssfs_dir();
    if !common::present(&dir) { return }
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten()
        .map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "ssf")).collect();
    files.sort();
    assert!(!files.is_empty(), "no packages in {}", dir.display());
    let mut problems: Vec<String> = Vec::new();
    let (mut classes, mut methods) = (0usize, 0usize);
    for path in &files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let model = match SourceModel::read(&std::fs::read(path).unwrap()) {
            Ok(m) => m,
            Err(e) => { problems.push(format!("{name}: does not read: {e}")); continue }
        };
        for c in model.classes.values() {
            classes += 1;
            methods += c.methods.len() + 2;
            for u in &c.unreadable {
                problems.push(format!("{name}: {}: unreadable: {u}", c.name));
            }
            for fs in &c.frame_scripts {
                if !c.methods.iter().any(|m| m.name == fs.method) {
                    problems.push(format!("{name}: {} registers frame {} to {}, which it does not have", c.name, fs.frame, fs.method));
                }
            }
            if !c.frame_scripts.is_empty() && c.symbol.is_some_and(|s| !model.sprites.contains_key(&s)) {
                problems.push(format!("{name}: {} has frame scripts but its symbol is no sprite", c.name));
            }
            // a value the decompiler kept to carry across a branch is folded back everywhere
            let mut joins = 0;
            for f in c.methods.iter().map(|m| &m.function).chain([&c.constructor, &c.static_init]) {
                ast::walk_exprs(&f.body, &mut |e| if matches!(e, ast::Expr::Join { .. }) { joins += 1 });
            }
            if joins > 0 { problems.push(format!("{name}: {}: {joins} join slots left unfolded", c.name)); }
        }
    }
    eprintln!("{} packages, {classes} classes, {methods} functions", files.len());
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}
