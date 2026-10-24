//! `peptide pack --project <.fraytools>`: build the package the engine loads straight from
//! the project's library, in-process, and drop it into every publish folder the project
//! names. What FrayTools' "Publish All" produces, without launching FrayTools.
//!
//! This reads the library off disk (the `.meta` beside each file carries its id, guid,
//! language and export flag); `fra_pack` builds the package from it.

use anyhow::{anyhow, bail, Context, Result};
use fra_pack::{Audio, Library, Script};
use rayon::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use crate::arg_val;

/// the warnings the packer gave (what it packed anyway), once the package is written
pub fn pack(args: &[String]) -> Result<Vec<String>> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("peptide pack --project <abs.fraytools> [--out <dir>]...\n\n\
build the engine package from the project's library, in-process, and write it to every\n\
publish folder the project names (or only to the --out dirs when given). prints the path\n\
of each package written.");
        return Ok(Vec::new());
    }
    let project = arg_val(args, "--project").ok_or_else(|| anyhow!("pack: --project <.fraytools> is required"))?;
    // absolute, so the project's publish folders (relative to it) resolve against it and not
    // against wherever this was run from
    let project = std::fs::canonicalize(&project).with_context(|| format!("pack: no project at {project}"))?;
    let project_dir = project.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let lib = project_dir.join("library");
    if !lib.is_dir() { bail!("pack: no library beside the project at {}", lib.display()); }

    let outs: Vec<PathBuf> = {
        let given: Vec<PathBuf> = args.windows(2).filter(|w| w[0] == "--out").map(|w| PathBuf::from(&w[1])).collect();
        if !given.is_empty() { given } else {
            // the game's own custom folder for this project is a publish folder whether or
            // not the project lists it yet (the same wiring FrayTools' publish gets)
            let stem = project.file_name().and_then(|s| s.to_str()).unwrap_or("").trim_end_matches(".fraytools").to_string();
            let root = arg_val(args, "--fraymakers-root").map(PathBuf::from).unwrap_or_else(crate::fraytools::default_fraymakers_root);
            let custom = root.join("custom").join(&stem);
            crate::fraytools::ensure_publish_folder(&project, &project_dir, &custom);
            let proj: Value = serde_json::from_str(&std::fs::read_to_string(&project)?)?;
            proj["publishFolders"].as_array().map(|a| a.iter()
                .filter_map(|f| f["path"].as_str().map(|p| project_dir.join(p))).collect()).unwrap_or_default()
        }
    };
    if outs.is_empty() { bail!("pack: the project names no publish folder and no --out was given"); }

    let built = fra_pack::build(&read_library(&lib)?)?;
    for dir in outs {
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(format!("{}.fra", built.name));
        std::fs::write(&path, &built.bytes).with_context(|| format!("writing {}", path.display()))?;
        println!("{}", path.display());
    }
    for w in &built.warnings { eprintln!("pack: warning: {w}"); }
    eprintln!("pack: {} entities, {} scripts, {} audio, {} sheet(s), {} bytes",
        built.entities, built.scripts, built.audio, built.sheets, built.bytes.len());
    Ok(built.warnings)
}

fn read_json(p: &Path) -> Result<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?)
        .with_context(|| format!("parsing {}", p.display()))
}

/// the `.meta` beside a library file
fn meta_of(p: &Path) -> Result<Value> {
    let mut m = p.as_os_str().to_owned();
    m.push(".meta");
    read_json(Path::new(&m))
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() { walk(&p, ext, out); }
        else if p.extension().and_then(|e| e.to_str()) == Some(ext) { out.push(p); }
    }
}

/// the library off disk: every file the package takes, in path order, and the bytes of the
/// pictures an entity draws
pub fn read_library(lib: &Path) -> Result<Library> {
    let mut entity_files = Vec::new();
    walk(&lib.join("entities"), "entity", &mut entity_files);
    let entities: Vec<Value> = entity_files.par_iter().map(|p| read_json(p)).collect::<Result<_>>()?;
    let drawn: BTreeSet<&str> = entities.iter()
        .flat_map(|e| e["symbols"].as_array().into_iter().flatten())
        .filter(|s| s["type"] == "IMAGE").filter_map(|s| s["imageAsset"].as_str()).collect();

    let mut sprite_files = Vec::new();
    walk(&lib.join("sprites"), "png", &mut sprite_files);
    let images = sprite_files.par_iter().filter_map(|p| {
        let guid = meta_of(p).ok()?.get("guid").and_then(Value::as_str)?.to_string();
        drawn.contains(guid.as_str()).then(|| Ok((guid, std::fs::read(p).with_context(|| format!("reading {}", p.display()))?)))
    }).collect::<Result<_>>()?;

    let mut hx = Vec::new();
    walk(&lib.join("scripts"), "hx", &mut hx);
    let scripts = hx.par_iter().map(|p| Ok(Script { meta: meta_of(p)?, text: std::fs::read_to_string(p)? }))
        .collect::<Result<_>>()?;

    let manifest = lib.join("manifest.json");
    let mut pal_files = Vec::new();
    walk(lib, "palettes", &mut pal_files);
    let palettes = pal_files.iter().map(|p| Ok((read_json(p)?, meta_of(p).unwrap_or_else(|_| json!({})))))
        .collect::<Result<_>>()?;

    let mut audio_files = Vec::new();
    for ext in ["wav", "ogg", "mp3"] { walk(&lib.join("audio"), ext, &mut audio_files); }
    audio_files.sort();
    let audio = audio_files.par_iter().map(|p| Ok(Audio {
        meta: meta_of(p)?,
        format: p.extension().and_then(|e| e.to_str()).unwrap_or("wav").to_string(),
        bytes: std::fs::read(p)?,
    })).collect::<Result<_>>()?;

    Ok(Library {
        images, entities, scripts,
        manifest: read_json(&manifest)?, manifest_meta: meta_of(&manifest)?,
        palettes, audio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn picture(p: &Path, rgba: [u8; 4]) {
        image::RgbaImage::from_pixel(4, 4, image::Rgba(rgba)).save(p).unwrap();
    }

    fn entity(x: f64) -> String {
        format!(r#"{{"id":"e","guid":"g","animations":[{{"name":"idle","layers":["l"]}}],
            "layers":[{{"$id":"l","type":"IMAGE","name":"art","keyframes":["k"]}}],
            "keyframes":[{{"$id":"k","length":1,"symbol":"s"}}],
            "symbols":[{{"$id":"s","type":"IMAGE","imageAsset":"img","x":{x},"y":4.0}}]}}"#)
    }

    /// the package's header, and its first sheet's top-left pixel
    fn read_package(p: &Path) -> (Value, [u8; 4]) {
        let b = std::fs::read(p).unwrap();
        let n = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
        let head: Value = serde_json::from_slice(&b[4..4 + n]).unwrap();
        let sheet = &head["spritesheets"][0];
        let at = 4 + n + sheet["bytesOffset"].as_u64().unwrap() as usize;
        let png = &b[at..at + sheet["bytesLength"].as_u64().unwrap() as usize];
        let px = image::load_from_memory(png).unwrap().to_rgba8().get_pixel(0, 0).0;
        (head, px)
    }

    /// What the app's Publish button does when it is pressed twice with the project edited in
    /// between: both publishes run in the one process, and the second packs what is on disk now.
    #[test]
    fn a_second_publish_in_one_process_packs_the_edits() {
        let dir = std::env::temp_dir().join(format!("peptide-repack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = dir.join("library");
        write(&dir.join("test.fraytools"), r#"{"publishFolders":[]}"#);
        write(&lib.join("manifest.json"), r#"{"resourceId":"test","content":[]}"#);
        write(&lib.join("manifest.json.meta"), r#"{"guid":"m","id":"manifest"}"#);
        write(&lib.join("scripts/Script.hx.meta"), r#"{"guid":"sg","id":"script"}"#);
        write(&lib.join("scripts/Script.hx"), "// first");
        write(&lib.join("sprites/a.png.meta"), r#"{"guid":"img"}"#);
        picture(&lib.join("sprites/a.png"), [0, 0, 255, 255]);
        write(&lib.join("entities/e.entity"), &entity(3.0));

        let out = dir.join("out");
        let args: Vec<String> = ["--project", dir.join("test.fraytools").to_str().unwrap(), "--out", out.to_str().unwrap()]
            .iter().map(|s| s.to_string()).collect();
        pack(&args).unwrap();
        let (first, first_px) = read_package(&out.join("test.fra"));

        write(&lib.join("scripts/Script.hx"), "// edited");
        write(&lib.join("entities/e.entity"), &entity(30.0));
        picture(&lib.join("sprites/a.png"), [255, 0, 0, 255]);
        pack(&args).unwrap();
        let (second, second_px) = read_package(&out.join("test.fra"));
        let _ = std::fs::remove_dir_all(&dir);

        let script = |h: &Value| h["scripts"].as_array().unwrap().iter()
            .find(|s| s["id"] == "script").unwrap()["value"].as_str().unwrap().to_string();
        let x = |h: &Value| h["entities"][0]["animations"][0]["layers"][0]["keyframes"][0]["symbol"]["data"][0].as_f64().unwrap();
        assert_eq!((script(&first), x(&first), first_px), ("// first".to_string(), 3.0, [0, 0, 255, 255]));
        assert_eq!((script(&second), x(&second), second_px), ("// edited".to_string(), 30.0, [255, 0, 0, 255]));
    }
}
