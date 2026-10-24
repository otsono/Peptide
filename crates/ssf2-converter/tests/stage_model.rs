//! The source model finds a stage's class and its objects the way the stage pipeline does today:
//! the stage class by what it extends, each object class with its kind (the class under the api
//! root it reaches). The runtime path for stages builds on these, so they must agree before any
//! stage output moves. Skips cleanly without the corpus.

mod common;

use ssf2_converter::compile::Runtime;

/// the stages the sample sweep covers (the justfile's `sample_stages`)
const SAMPLE: &[&str] = &["battlefield", "battlefield2", "bombfactory", "butterbuilding", "bowserscastle",
    "crateria", "crystalsmash", "flatzoneplus", "fourside", "huecomundo"];

#[test]
fn the_model_finds_each_stage_and_its_objects() {
    let dir = common::ssfs_dir().join("stages");
    if !common::present(&dir) { return }
    let runtime = Runtime::locate().map(|d| Runtime::load(&d)).unwrap_or_default();
    // the api's own names, as the runtime renames them
    let name = |n: &str| runtime.names.classes.get(n).cloned().unwrap_or_else(|| n.to_string());
    let stage_root = name("SSF2Stage");
    let object_roots = [name("SSF2GameObject"), name("SSF2CollisionBoundary")];
    let roots: Vec<&str> = object_roots.iter().map(String::as_str).collect();
    let mut problems: Vec<String> = Vec::new();
    let mut read = 0;
    for id in SAMPLE {
        let path = dir.join(format!("{id}.ssf"));
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let (model, _) = runtime.read_package(&bytes).unwrap();
        read += 1;
        // the stage pipeline's reading of the same file
        let swf_data = ssf2_converter::ssf::decompress(&bytes).unwrap_or_else(|_| bytes.clone());
        let swf = ssf2_converter::swf_parser::parse(&swf_data).unwrap();
        let blocks: Vec<_> = swf.abc_blocks.iter().filter_map(|b| ssf2_converter::abc_parser::parse(b).ok()).collect();
        let doc = blocks.iter().find_map(ssf2_converter::stage_abc::extract_stage).map(|m| m.doc_class);
        let stages: Vec<String> = model.descendants(&stage_root).iter()
            .map(|c| ssf2_converter::compile::short(&c.name).to_string()).collect();
        match &doc {
            Some(d) if !stages.contains(d) => problems.push(format!("{id}: the pipeline's stage class {d}, the model's {stages:?}")),
            None if !stages.is_empty() => problems.push(format!("{id}: the model finds a stage class {stages:?} the pipeline does not")),
            _ => {}
        }
        // the stage the package registers, and the clip that is the whole stage: what the
        // runtime's stage script runs
        match model.stage() {
            Some((class, _, sym)) if stages.contains(&ssf2_converter::compile::short(&class).to_string()) && model.sprites.contains_key(&sym) => {}
            other => problems.push(format!("{id}: the registered stage {other:?}, the classes under the stage api {stages:?}")),
        }
        let mut want: Vec<(String, String)> = blocks.iter()
            .flat_map(|abc| ssf2_converter::ssf2_objects::game_object_classes(&abc.classes))
            .map(|g| (g.class_name, name(&g.base))).collect();
        want.sort();
        want.dedup();
        let got = model.kinds_under(&roots);
        eprintln!("{id}: stage {stages:?}, {} objects {:?}", got.len(), got.iter().take(4).collect::<Vec<_>>());
        if got != want { problems.push(format!("{id}: objects\n  pipeline {want:?}\n  model    {got:?}")); }
    }
    assert_eq!(read, SAMPLE.len(), "a sample stage is missing from {}", dir.display());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Every class of the sample stages compiles on the runtime path with nothing it could not write,
/// and what it writes parses (with the Haxe toolchain's parser, when one is installed).
#[test]
fn the_sample_stages_compile() {
    let dir = common::ssfs_dir().join("stages");
    if !common::present(&dir) { return }
    let runtime = Runtime::locate().map(|d| Runtime::load(&d)).unwrap_or_default();
    let out_dir = std::env::temp_dir().join(format!("peptide-stages-compile-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut problems: Vec<String> = Vec::new();
    for id in SAMPLE {
        let bytes = std::fs::read(dir.join(format!("{id}.ssf"))).unwrap();
        let (model, renamed) = runtime.read_package(&bytes).unwrap();
        problems.extend(renamed.iter().map(|p| format!("{id}: {p}")));
        let out = ssf2_converter::compile::compile_package(&model, &runtime);
        problems.extend(out.problems.iter().map(|p| format!("{id}: {p}")));
        eprintln!("{id}: {} classes", out.classes.len());
        let text: String = out.classes.values().cloned().collect::<Vec<_>>().join("\n");
        std::fs::write(out_dir.join(format!("{id}.hx")), text).unwrap();
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.iter().take(30).cloned().collect::<Vec<_>>().join("\n"));
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    match std::process::Command::new("haxe").arg("-cp").arg(repo.join("tools/hxcheck"))
        .arg("--macro").arg(format!("HxCheck.run('{}')", out_dir.display())).output() {
        Err(_) => eprintln!("skip: no haxe on PATH, so the parse check did not run"),
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            eprintln!("{}", text.trim());
            assert!(text.contains(" 0 that do not parse"), "parse check:\n{text}{}", String::from_utf8_lossy(&o.stderr));
        }
    }
    let _ = std::fs::remove_dir_all(&out_dir);
}
