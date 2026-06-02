//! dump_raw_stats — print the RAW SSF2 physics constants for a character .ssf,
//! exactly as extracted from the bytecode (no scaling / no FM mapping applied).
//!
//! Usage:
//!   dump_raw_stats <file.ssf> [charNameOverride]
//!   dump_raw_stats --all [ssfs_dir]      # every .ssf in the dir
//!
//! This is the ground-truth INPUT side for the SSF2→Fraymakers scaling work:
//! the numbers here feed the physics simulator that computes real motion.

use ssf2_converter::*;
use std::env;
use std::path::{Path, PathBuf};

fn dump_one(path: &Path, name_override: Option<&str>) {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
    let Ok(bytes) = std::fs::read(path) else {
        eprintln!("{stem}: cannot read");
        return;
    };
    let Ok(swf_bytes) = ssf::decompress(&bytes) else {
        eprintln!("{stem}: decompress failed");
        return;
    };
    let Ok(swf) = swf_parser::parse(&swf_bytes) else {
        eprintln!("{stem}: swf parse failed");
        return;
    };

    for abc_bytes in &swf.abc_blocks {
        let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };

        // Figure out which character name(s) to extract.
        let mut names: Vec<String> = Vec::new();
        if let Some(n) = name_override {
            names.push(n.to_string());
        } else if let Some(meta) = abc_parser::extract_main_package_metadata(&abc) {
            for (id, _method) in &meta.characters {
                names.push(id.clone());
            }
        }
        if names.is_empty() {
            // Fall back: derive a single name from the file stem (Title-case-ish).
            names.push(stem.to_string());
        }

        for name in &names {
            match abc_parser::extract_character(&abc, name) {
                Ok(ch) => {
                    if let Some(stats) = &ch.stats {
                        println!("=== {stem} :: character '{name}' (raw SSF2 constants) ===");
                        // Stable, readable order.
                        let mut keys: Vec<_> = stats.values.keys().cloned().collect();
                        keys.sort();
                        for k in &keys {
                            println!("  {k:<22} = {}", stats.values[k]);
                        }
                        println!();
                    } else {
                        println!("=== {stem} :: character '{name}' — NO stats extracted ===\n");
                    }
                }
                Err(e) => {
                    eprintln!("{stem} :: '{name}': extract_character error: {e}");
                }
            }
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: dump_raw_stats <file.ssf> [charName] | --all [ssfs_dir]");
        std::process::exit(2);
    }

    if args[0] == "--all" {
        let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("../ssf2-ssfs"));
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|_| panic!("cannot read dir {}", dir.display()))
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "ssf").unwrap_or(false))
            .filter(|p| p.file_stem().map(|s| s != "misc").unwrap_or(true))
            .collect();
        files.sort();
        for f in &files {
            dump_one(f, None);
        }
    } else {
        let path = PathBuf::from(&args[0]);
        dump_one(&path, args.get(1).map(|s| s.as_str()));
    }
}
