//! For every .ssf in the corpus, decompile Main's constructor (iinit)
//! and report:
//!   - The set of register("<key>", ...) keys actually emitted
//!   - For "characters", the count + list of get* method names called
//!   - The "id" / "guid" string values
//!   - Whether any orphan get* methods on Main are NOT in the characters
//!     array (dev-leftover detection)
//!
//! Used to validate the constructor-walk detection design before
//! implementing it. Stops at the first variation that breaks the
//! assumption "register('characters', literal-array of getX() calls)".

use ssf2_converter::*;
use std::env;

fn main() {
    let ssfs_dir = env::args().nth(1).unwrap_or_else(||
        "../ssf2-ssfs".to_string());

    let mut files: Vec<_> = std::fs::read_dir(&ssfs_dir).unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "ssf").unwrap_or(false))
        .collect();
    files.sort();

    let mut total_with_main = 0;
    let mut total_no_main = 0;
    let mut total_orphan_get = 0;

    println!("ssf                    | reg keys                                        | chars                          | orphan get*");
    println!("-----------------------+-------------------------------------------------+--------------------------------+---------------");

    for path in &files {
        let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
        let Ok(bytes) = std::fs::read(path) else { continue };
        let Ok(swf_bytes) = ssf::decompress(&bytes) else { continue };
        let Ok(swf) = swf_parser::parse(&swf_bytes) else { continue };

        for abc_bytes in &swf.abc_blocks {
            let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };
            let Some(main) = abc.classes.iter().find(|c| c.name == "Main") else {
                total_no_main += 1;
                println!("{:<22} | (NO MAIN CLASS)", stem);
                continue;
            };
            total_with_main += 1;

            let Some(body) = abc.method_bodies.iter()
                .find(|b| b.method_idx == main.constructor_idx) else { continue };
            let pc = abc.methods.get(body.method_idx as usize)
                .map(|m| m.param_count as usize).unwrap_or(0);
            let params: Vec<String> = (0..pc).map(|i| format!("arg{}", i)).collect();
            let d = decompiler::decompile_method(body, &abc, "Main", &params);

            // Parse the decompiled output for the register pattern.
            // Each line of interest looks like:
            //   self.register("<key>", <value-expr>);
            // For "characters", the value expr is `[self.getX(), self.getY(), ...]`.
            let mut reg_keys: Vec<String> = Vec::new();
            let mut characters: Vec<String> = Vec::new();
            let mut id_val: Option<String> = None;
            let mut guid_val: Option<String> = None;
            for raw in d.lines() {
                let line = raw.trim();
                // Try to match `self.register("<key>", ...)`
                if let Some(rest) = line.strip_prefix("self.register(\"") {
                    if let Some(end) = rest.find('"') {
                        let key = &rest[..end];
                        reg_keys.push(key.to_string());
                        let after_key = &rest[end + 1..];
                        // skip `, `
                        let after_comma = after_key.trim_start_matches(',').trim();
                        match key {
                            "id" => {
                                if let Some(s) = after_comma.strip_prefix('"') {
                                    if let Some(end) = s.find('"') {
                                        id_val = Some(s[..end].to_string());
                                    }
                                }
                            }
                            "guid" => {
                                if let Some(s) = after_comma.strip_prefix('"') {
                                    if let Some(end) = s.find('"') {
                                        guid_val = Some(s[..end].to_string());
                                    }
                                }
                            }
                            "characters" => {
                                // Expecting `[self.getX(), self.getY(), ...]`
                                if let Some(arr_inner) = after_comma
                                    .trim_start().strip_prefix('[')
                                    .and_then(|s| s.rsplit_once(']'))
                                    .map(|(inner, _tail)| inner)
                                {
                                    for piece in arr_inner.split(',') {
                                        let p = piece.trim();
                                        // accept e.g. `self.getMario()` or `getMario()`
                                        let getter = p.strip_prefix("self.").unwrap_or(p);
                                        if let Some(name) = getter.strip_suffix("()") {
                                            characters.push(name.to_string());
                                        } else if !p.is_empty() {
                                            characters.push(format!("UNRECOGNIZED({})", p));
                                        }
                                    }
                                } else if !after_comma.is_empty() {
                                    characters.push(format!("UNRECOGNIZED({})", after_comma));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Compute orphan get* methods: those on Main but not in characters[].
            let main_getters: Vec<String> = main.instance_methods.iter()
                .filter(|t| t.name.starts_with("get"))
                .map(|t| t.name.clone())
                .collect();
            let orphans: Vec<String> = main_getters.iter()
                .filter(|n| !characters.contains(n))
                .cloned().collect();
            if !orphans.is_empty() { total_orphan_get += orphans.len(); }

            let reg_str = reg_keys.join("/");
            let chars_str = format!("{} {:?}", characters.len(),
                characters.iter().map(String::as_str).collect::<Vec<_>>());
            let orphan_str = if orphans.is_empty() { "-".to_string() }
                else { format!("{:?}", orphans) };

            println!("{:<22} | {:<47} | {:<30} | {}",
                stem, reg_str, chars_str, orphan_str);

            // Quick sanity: id should always be present and match filename stem
            // (since the SSF id mirrors the package name).
            if let Some(id) = &id_val {
                if id != &stem {
                    println!("  !! id={:?} disagrees with stem={:?}", id, stem);
                }
            } else {
                println!("  !! NO 'id' register call");
            }
            if guid_val.is_none() {
                println!("  !! NO 'guid' register call");
            }
        }
    }

    println!();
    println!("=== Summary ===");
    println!("SSFs with Main:    {}", total_with_main);
    println!("SSFs without Main: {}", total_no_main);
    println!("Total orphan get* methods: {}", total_orphan_get);
}
