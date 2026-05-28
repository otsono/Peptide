//! Dump the `Main` class + every class that has any sheik-named
//! instance method, and decompile a few key ones. Used to track down
//! where Sheik's stats live in zelda.ssf.

use ssf2_converter::*;
use std::env;

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        "/Users/jimmy/.openclaw/workspace-main/ssf2-ssfs/zelda.ssf".to_string()
    });
    let bytes = std::fs::read(&path).expect("read");
    let swf_bytes = ssf::decompress(&bytes).expect("decompress");
    let swf = swf_parser::parse(&swf_bytes).expect("parse");

    for abc_bytes in &swf.abc_blocks {
        let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };

        // ── Main class ─────────────────────────────────────────────
        if let Some(main) = abc.classes.iter().find(|c| c.name == "Main") {
            println!("=== Main class ({} instance methods, {} class methods) ===",
                main.instance_methods.len(), main.class_methods.len());
            for t in &main.instance_methods {
                let kind_name = match t.kind { 0 => "Slot", 1 => "Method", 2 => "Getter",
                    3 => "Setter", 4 => "Class", 5 => "Function", 6 => "Const", _ => "?" };
                println!("  inst {:<7} method_idx={:<6}  {}", kind_name, t.method_idx, t.name);
            }
            for t in &main.class_methods {
                let kind_name = match t.kind { 0 => "Slot", 1 => "Method", 2 => "Getter",
                    3 => "Setter", 4 => "Class", 5 => "Function", 6 => "Const", _ => "?" };
                println!("  stat {:<7} method_idx={:<6}  {}", kind_name, t.method_idx, t.name);
            }

            // Decompile getSheik
            for t in main.instance_methods.iter().chain(main.class_methods.iter()) {
                if t.name == "getSheik" || t.name.contains("Sheik") || t.name.contains("sheik") {
                    let Some(body) = abc.method_bodies.iter()
                        .find(|b| b.method_idx == t.method_idx)
                    else { continue };
                    println!();
                    println!("── Main::{}  (bytecode {} bytes)", t.name, body.bytecode.len());
                    let param_count = abc.methods.get(body.method_idx as usize)
                        .map(|m| m.param_count as usize).unwrap_or(0);
                    let params: Vec<String> = (0..param_count).map(|i| format!("arg{}", i)).collect();
                    let d = decompiler::decompile_method(body, &abc, &t.name, &params);
                    println!("{}", d);
                }
            }
        } else {
            println!("(no Main class)");
        }
        println!();

        // ── Every class that has any sheik-named instance method ──
        println!("=== Classes with a sheik-named instance method ===");
        for c in &abc.classes {
            let sheik_methods: Vec<&abc_parser::Trait> = c.instance_methods.iter()
                .chain(c.class_methods.iter())
                .filter(|t| t.name.to_lowercase().contains("sheik"))
                .collect();
            if !sheik_methods.is_empty() {
                println!("  class {}  super={}", c.name, c.super_name);
                for t in &sheik_methods {
                    println!("    {} (kind={}, method_idx={})", t.name, t.kind, t.method_idx);
                }
            }
        }
        println!();

        // ── Did we already establish that there's a `sheik` class? ──
        // Inspect that class.
        if let Some(sheik_cls) = abc.classes.iter().find(|c| c.name == "sheik") {
            println!("=== `sheik` class — super={} ===", sheik_cls.super_name);
            println!("Has {} instance methods, of which {} are frame*.",
                sheik_cls.instance_methods.len(),
                sheik_cls.instance_methods.iter().filter(|t| t.name.starts_with("frame")).count());
            // Show non-frame methods only.
            for t in sheik_cls.instance_methods.iter() {
                if !t.name.starts_with("frame") {
                    println!("  inst method  {}", t.name);
                }
            }
        }
    }
}
