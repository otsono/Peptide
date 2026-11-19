//! A fighter turned into another fighter of its package (`replaceCharacter`): the other one merged
//! into it as its second form. Fraymakers makes no fighter mid-match, nor swaps a fighter's art or
//! its states' animations as it runs, so the two are one fighter here: the second's animations in
//! the first's entity under a prefix, its scripts' names under the same prefix, each form's
//! listeners and timers running only while it is that form, and `runtime/native/forms.hx` turning
//! between them. Text over the files the conversion wrote; the behaviour is the skeleton's.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

/// one fighter's files, as the conversion wrote them
struct Fighter {
    script: String,
    entity: Value,
    stats: String,
    anim_stats: String,
    hitbox_stats: String,
}

fn read_fighter(lib: &Path, folder: &str) -> Result<Fighter> {
    let s = |f: &str| std::fs::read_to_string(lib.join("scripts").join(folder).join(f)).with_context(|| format!("{folder}/{f}"));
    Ok(Fighter {
        script: s("Script.hx")?,
        entity: serde_json::from_str(&std::fs::read_to_string(lib.join("entities").join(format!("{folder}.entity")))?)?,
        stats: s("CharacterStats.hx")?,
        anim_stats: s("AnimationStats.hx")?,
        hitbox_stats: s("HitboxStats.hx")?,
    })
}

/// The pairs a project's fighters turn into each other as: for each fighter (its folder) whose code
/// turns it into another fighter of the project by that one's id, (its folder, its id, the other's
/// folder, the other's id).
pub fn pairs(lib: &Path, fighters: &[(String, String)], finals: &[(String, String, f64)]) -> Vec<(String, String, String, String)> {
    let mut out = Vec::new();
    // (a fighter whose declaration names another of the project its special form: its final form)
    for (normal, special, _) in finals {
        let (Some(a), Some(b)) = (fighters.iter().find(|(_, id)| id.eq_ignore_ascii_case(normal)), fighters.iter().find(|(_, id)| id.eq_ignore_ascii_case(special))) else { continue };
        out.push((a.0.clone(), a.1.clone(), b.0.clone(), b.1.clone()));
    }
    for (folder, id) in fighters {
        let Ok(script) = std::fs::read_to_string(lib.join("scripts").join(folder).join("Script.hx")) else { continue };
        let entity = std::fs::read_to_string(lib.join("entities").join(format!("{folder}.entity"))).unwrap_or_default();
        for (other, oid) in fighters {
            if other == folder { continue }
            let call = format!("formSwitch(self, \"{oid}\"");
            if (script.contains(&call) || entity.contains(&call.replace('"', "\\\""))) && !out.iter().any(|p| p.0 == *folder && p.2 == *other) {
                out.push((folder.clone(), id.clone(), other.clone(), oid.clone()));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Merge each pair's second fighter into its first, from the files as the conversion wrote them
/// (both ways round where both turn into each other).
pub fn merge_all(lib: &Path, pairs: &[(String, String, String, String)], finals: &[(String, String, f64)], skeleton: &str) -> Result<()> {
    let folders: BTreeSet<&String> = pairs.iter().flat_map(|(a, _, b, _)| [a, b]).collect();
    let mut pristine: BTreeMap<String, Fighter> = BTreeMap::new();
    for f in folders { pristine.insert(f.clone(), read_fighter(lib, f)?); }
    let palettes = read_palettes(lib);
    for (a, aid, b, bid) in pairs {
        let limit = finals.iter().find(|(n, sp, _)| n.eq_ignore_ascii_case(aid) && sp.eq_ignore_ascii_case(bid)).map(|f| f.2);
        merge(lib, &pristine[a], a, aid, &pristine[b], bid, skeleton, &palettes, limit)?;
        log::info!("{a} turns into {b} here as its second form (`replaceCharacter`)");
    }
    Ok(())
}

/// the project's costume collections, by guid: (file, collection)
fn read_palettes(lib: &Path) -> BTreeMap<String, (std::path::PathBuf, Value)> {
    let mut out = BTreeMap::new();
    let Ok(rd) = std::fs::read_dir(lib) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "palettes") { continue }
        let Ok(v) = std::fs::read_to_string(&p).map_err(anyhow::Error::from).and_then(|t| Ok(serde_json::from_str::<Value>(&t)?)) else { continue };
        if let Some(g) = v.get("guid").and_then(|g| g.as_str()) { out.insert(g.to_string(), (p, v)); }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn merge(lib: &Path, fa: &Fighter, a: &str, aid: &str, fb: &Fighter, bid: &str, skeleton: &str,
         palettes: &BTreeMap<String, (std::path::PathBuf, Value)>, final_limit: Option<f64>) -> Result<()> {
    let prefix = format!("{bid}__");
    let names = top_level_names(&fb.script);
    let rename = Renamer::new(&names, &prefix);
    let b_code = |code: &str| anim_names(&form_calls(&rename.apply(code), 1));
    let script_b = b_code(&fb.script);
    let script_a = form_calls(&fa.script, 0);

    // the forms' glue, from the skeleton
    let publish_a = publish_lines(&script_a, "");
    let publish_b = publish_lines(&script_b, &prefix);
    let mut cases = label_cases(&fa.anim_stats, "", 0);
    cases.extend(label_cases(&fb.anim_stats, &prefix, 1));
    let glue = fill(skeleton, &[("{publish_a}", publish_a.join("\n\t\t")), ("{publish_b}", publish_b.join("\n\t\t")), ("{cases}", cases.join("\n\t\t"))])
        .replace("PREFIX_LEN", &prefix.len().to_string())
        .replace("PREFIX", &prefix)
        .replace("A_ID", aid).replace("B_ID", bid)
        .replace("STATS_A", &stats_record(&fa.stats))
        .replace("STATS_B", &stats_record(&fb.stats));

    // the first form's engine callbacks hand over to the second's while it is that form
    let mut script = script_a;
    for f in ["update", "onTeardown", "onKill", "onStale", "afterPushState", "afterPopState", "afterFlushStates"] {
        if !crate::dyn_re(&format!(r"(?m)^function {}{}\(", regex::escape(&prefix), f)).is_match(&script_b) { continue }
        let head = format!("function {f}() {{\n");
        if let Some(at) = script.find(&head) {
            let pre = if f == "update" { "\tformFrame();\n".to_string() } else { String::new() };
            script.insert_str(at + head.len(), &format!("{pre}\tif (form.get() == 1) {{ {prefix}{f}(); return; }}\n"));
        }
    }
    let head = "function initialize() {\n";
    if let Some(at) = script.find(head) {
        let final_form = final_limit.map(|n| format!("\tself.exports.__finalForm = {{ id: \"{bid}\", back: \"{aid}\", frames: {} }};\n", (n * 2.0).round())).unwrap_or_default();
        script.insert_str(at + head.len(), &format!("\tself.exports.__formSwitch = formTurn;\n{final_form}\tself.addEventListener(EntityEvent.STATE_CHANGE, function(e) {{ formFrame(); }}, {{ persistent: true }});\n"));
    }
    let script = format!("{script}\n{glue}\n// ---- the second form, its names under {prefix}\n{script_b}");
    std::fs::write(lib.join("scripts").join(a).join("Script.hx"), script)?;

    // the entity: the second form's animations under the prefix, its frame code renamed
    let mut ea = fa.entity.clone();
    let mut eb = fb.entity.clone();
    for k in eb["keyframes"].as_array_mut().into_iter().flatten() {
        if let Some(c) = k.get("code").and_then(|c| c.as_str()).map(b_code) { k["code"] = Value::String(c); }
    }
    for k in ea["keyframes"].as_array_mut().into_iter().flatten() {
        if let Some(c) = k.get("code").and_then(|c| c.as_str()).map(|c| form_calls(c, 0)) { k["code"] = Value::String(c); }
    }
    for an in eb["animations"].as_array_mut().into_iter().flatten() {
        if let Some(n) = an.get("name").and_then(|n| n.as_str()).map(|n| format!("{prefix}{n}")) { an["name"] = Value::String(n); }
    }
    for key in ["symbols", "keyframes", "layers", "animations"] {
        let more = eb[key].as_array().cloned().unwrap_or_default();
        if let Some(arr) = ea[key].as_array_mut() { arr.extend(more); }
    }
    std::fs::write(lib.join("entities").join(format!("{a}.entity")), serde_json::to_string_pretty(&ea)?)?;

    // its stats files: the second form's entries under the prefix
    for (file, ta, tb) in [("AnimationStats.hx", &fa.anim_stats, &fb.anim_stats), ("HitboxStats.hx", &fa.hitbox_stats, &fb.hitbox_stats)] {
        let more: Vec<String> = entries(tb).into_iter().map(|(k, v)| format!("\t{prefix}{k}: {v}")).collect();
        let end = ta.rfind('}').context("a stats file with no record")?;
        let head = ta[..end].trim_end();
        let sep = if head.ends_with('{') || head.ends_with(',') || more.is_empty() { "" } else { "," };
        std::fs::write(lib.join("scripts").join(a).join(file), format!("{head}{sep}\n{}\n}}\n", more.join(",\n")))?;
    }

    // its costumes: the second form's colours folded into the first's collection, each costume's
    // targets with them (a colour both use keeps the first's)
    let coll = |e: &Value| e.get("paletteMap").and_then(|m| m.get("paletteCollection")).and_then(|g| g.as_str()).map(String::from);
    if let (Some(ga), Some(gb)) = (coll(&fa.entity), coll(&fb.entity)) {
        if let (Some((file, pa)), Some((_, pb))) = (palettes.get(&ga), palettes.get(&gb)) {
            if ga != gb {
                let mut pa = pa.clone();
                let have: BTreeSet<String> = pa["colors"].as_array().into_iter().flatten().filter_map(|c| c["color"].as_str().map(String::from)).collect();
                let keep: Vec<Value> = pb["colors"].as_array().into_iter().flatten().filter(|c| c["color"].as_str().is_some_and(|x| !have.contains(x))).cloned().collect();
                let ids: BTreeSet<String> = keep.iter().filter_map(|c| c["$id"].as_str().map(String::from)).collect();
                if let Some(arr) = pa["colors"].as_array_mut() { arr.extend(keep); }
                let maps_b = pb["maps"].as_array().cloned().unwrap_or_default();
                for (i, m) in pa["maps"].as_array_mut().into_iter().flatten().enumerate() {
                    let Some(mb) = maps_b.get(i) else { continue };
                    let add: Vec<Value> = mb["colors"].as_array().into_iter().flatten().filter(|x| x["paletteColorId"].as_str().is_some_and(|id| ids.contains(id))).cloned().collect();
                    if let Some(arr) = m["colors"].as_array_mut() { arr.extend(add); }
                }
                std::fs::write(file, serde_json::to_string_pretty(&pa)?)?;
            }
        }
    }
    Ok(())
}

fn fill(t: &str, words: &[(&str, String)]) -> String {
    let mut s = t.to_string();
    for (k, v) in words { s = s.replace(&format!("// {k}"), v); }
    s
}

/// a script's own top-level names: its functions and its variables
fn top_level_names(script: &str) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = crate::re!(r"(?m)^function\s+([A-Za-z_]\w*)\s*\(").captures_iter(script).map(|c| c[1].to_string()).collect();
    out.extend(crate::re!(r"(?m)^var\s+([A-Za-z_]\w*)\b").captures_iter(script).map(|c| c[1].to_string()));
    for n in ["self", "match", "stage"] { out.remove(n); }
    out
}

/// the second form's names put under its prefix wherever its code uses them as names (not in its
/// text, a member or a record's key)
struct Renamer { re: Option<regex::Regex>, prefix: String }
impl Renamer {
    fn new(names: &BTreeSet<String>, prefix: &str) -> Renamer {
        let mut v: Vec<&String> = names.iter().collect();
        v.sort_by_key(|n| std::cmp::Reverse(n.len()));
        let alt = v.iter().map(|n| regex::escape(n)).collect::<Vec<_>>().join("|");
        Renamer { re: (!alt.is_empty()).then(|| regex::Regex::new(&format!(r"\b({alt})\b")).expect("names")), prefix: prefix.to_string() }
    }
    fn apply(&self, code: &str) -> String {
        let Some(re) = &self.re else { return code.to_string() };
        // (outside its strings and comments)
        let lit = crate::re!(r#""(?:\\.|[^"\\])*"|//[^\n]*"#);
        let mut out = String::with_capacity(code.len() + code.len() / 8);
        let mut at = 0;
        for m in lit.find_iter(code) {
            out.push_str(&self.bare(re, &code[at..m.start()]));
            out.push_str(m.as_str());
            at = m.end();
        }
        out.push_str(&self.bare(re, &code[at..]));
        out
    }
    fn bare(&self, re: &regex::Regex, code: &str) -> String {
        let b = code.as_bytes();
        let mut out = String::with_capacity(code.len());
        let mut at = 0;
        for m in re.find_iter(code) {
            // (a member, `.name`, is another object's)
            let member = m.start() > 0 && matches!(b[m.start() - 1], b'.' | b'$');
            // (a record's key, `{ name:` or `, name:`, is the record's)
            let after = code[m.end()..].trim_start();
            let before = code[..m.start()].trim_end();
            let key = after.starts_with(':') && !after.starts_with("::") && (before.ends_with('{') || before.ends_with(','));
            out.push_str(&code[at..m.start()]);
            if !member && !key { out.push_str(&self.prefix); }
            out.push_str(m.as_str());
            at = m.end();
        }
        out.push_str(&code[at..]);
        out
    }
}

/// a form's listeners and timers, run only while it is that form
fn form_calls(code: &str, k: u8) -> String {
    code.replace("self.addEventListener(", &format!("formListen({k}, "))
        .replace("self.removeEventListener(", &format!("formUnlisten({k}, "))
        .replace("self.addTimer(", &format!("formTimer({k}, "))
}

/// the second form's code names its animations as its source does; its own are under the prefix
fn anim_names(code: &str) -> String {
    code.replace("self.playAnimation(", "formPlay(")
        .replace("self.getAnimation()", "formBase(self.getAnimation())")
        .replace("self.hasAnimation(", "formHas(")
}

/// what a form's initialize publishes for other objects' code
fn publish_lines(script: &str, prefix: &str) -> Vec<String> {
    let head = format!("function {prefix}initialize() {{\n");
    let Some(at) = script.find(&head) else { return Vec::new() };
    let body = &script[at + head.len()..];
    let end = body.find("\n}").unwrap_or(body.len());
    body[..end].lines().map(str::trim)
        .filter(|l| l.starts_with("self.exports.") || l.starts_with("source.") || l.starts_with(&format!("{prefix}source.")))
        .map(String::from).collect()
}

/// a fighter's stats file as a record its code can hand the engine (all but its art, which the
/// engine does not take as it runs)
fn stats_record(t: &str) -> String {
    let (Some(s), Some(e)) = (t.find('{'), t.rfind('}')) else { return "{}".into() };
    let body: String = t[s..=e].lines().map(|l| l.split("//").next().unwrap_or("")).filter(|l| !l.trim_start().starts_with("spriteContent:")).collect::<Vec<_>>().join(" ");
    crate::re!(r",(\s*)\}").replace_all(&body, "$1}").into_owned()
}

/// the top-level entries of a stats file's record, as (key, its record's text)
fn entries(t: &str) -> Vec<(String, String)> {
    let (Some(s), Some(e)) = (t.find('{'), t.rfind('}')) else { return Vec::new() };
    let body = &t[s + 1..e];
    let b = body.as_bytes();
    let key_re = crate::re!(r"^\s*([A-Za-z_]\w*)\s*:\s*\{");
    let mut out = Vec::new();
    let (mut i, mut depth, mut start, mut key) = (0usize, 0i32, 0usize, String::new());
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') { i = body[i..].find('\n').map(|n| i + n).unwrap_or(b.len()); continue }
        if c == b'"' { let mut j = i + 1; while j < b.len() && b[j] != b'"' { j += if b[j] == b'\\' { 2 } else { 1 }; } i = j + 1; continue }
        if depth == 0 {
            if let Some(m) = key_re.captures(&body[i..]) {
                key = m[1].to_string();
                start = i + m.get(0).map(|x| x.end()).unwrap_or(0) - 1;
                i = start + 1; depth = 1; continue;
            }
        } else if c == b'{' { depth += 1 }
        else if c == b'}' { depth -= 1; if depth == 0 { out.push((key.clone(), body[start..=i].to_string())); } }
        i += 1;
    }
    out
}

/// each attack of a form by its source label: the animation whose metadata names it (the
/// shortest, where several do)
fn label_cases(anim_stats: &str, prefix: &str, k: u8) -> Vec<String> {
    let mut by: BTreeMap<String, String> = BTreeMap::new();
    let label = crate::re!(r#"label:\s*"([^"]+)""#);
    for (key, v) in entries(anim_stats) {
        let Some(c) = label.captures(&v) else { continue };
        let l = c[1].to_string();
        if by.get(&l).is_none_or(|have| key.len() < have.len()) { by.insert(l, key); }
    }
    by.into_iter().map(|(l, n)| format!("case \"{k}/{l}\": return \"{prefix}{n}\";")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renames_names_not_strings_members_or_keys() {
        let names: BTreeSet<String> = ["frozen", "jab"].iter().map(|s| s.to_string()).collect();
        let r = Renamer::new(&names, "sheik__");
        assert_eq!(r.apply("frozen.set(1); x.frozen; { frozen: 1, a: jab() }; \"frozen\"; c ? frozen : jab"),
                   "sheik__frozen.set(1); x.frozen; { frozen: 1, a: sheik__jab() }; \"frozen\"; c ? sheik__frozen : sheik__jab");
    }
    #[test]
    fn reads_a_stats_files_entries() {
        let t = "// x\n{\n\tjab1: { a: { b: 1 } },\n\tstand: { metadata: { label: \"stand\" } }\n}\n";
        let e = entries(t);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].0, "jab1");
        assert_eq!(label_cases(t, "p__", 1), vec!["case \"1/stand\": return \"p__stand\";".to_string()]);
    }
}
