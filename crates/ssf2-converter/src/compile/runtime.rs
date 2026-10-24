//! What the runtime provides, read from its own files.
//!
//! The runtime (`runtime/**/*.hx`) is the source's API and engine behaviour, written in Haxe. Its
//! files are the one list of what exists: the compiler reads them for the classes they define
//! (`// class NAME extends BASE`, then the members that class's `build` gives an object as
//! `me.NAME = ...`) and the names they make global (a top-level `var NAME` or `function NAME`).
//! Nothing here is a second copy of that list.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Default, Clone, Debug)]
pub struct RuntimeClass {
    pub name: String,
    pub extends: Option<String>,
    /// members its objects have (methods and fields)
    pub members: BTreeSet<String>,
    /// properties read through `get_NAME`
    pub getters: BTreeSet<String>,
    /// properties written through `set_NAME`
    pub setters: BTreeSet<String>,
    /// methods kept on the class and handed the object (`CLASS.NAME = function(me, ...)`), not on
    /// the object itself
    pub statics: BTreeSet<String>,
    /// members put on the object itself (`me.NAME = ...` in its build)
    pub own: BTreeSet<String>,
}

#[derive(Default, Clone, Debug)]
pub struct Runtime {
    pub classes: BTreeMap<String, RuntimeClass>,
    pub globals: BTreeSet<String>,
    /// members that take their arguments as one array (declared `function(rest)`), as AS3's
    /// `...rest` parameters do
    pub variadic: BTreeSet<String>,
    /// every file, by its path under the runtime directory, and its text
    pub files: BTreeMap<String, String>,
    /// the names the engine's own script scope has (`engine.names`)
    pub engine_names: BTreeSet<String>,
    /// SSF2's API names -> the runtime's (`api_names.jsonc`)
    pub names: crate::model::rename::Names,
    /// API calls written as the Fraymakers calls they are, by member (`api_calls.jsonc`)
    pub calls: BTreeMap<String, String>,
    /// the rows one kind of object writes its own way (`api_calls.jsonc`'s `by_kind`), by kind
    pub kind_calls: BTreeMap<String, BTreeMap<String, String>>,
    /// methods that do nothing here, by class, and classes whose making only makes a value
    /// (`inert.jsonc`)
    pub inert_calls: BTreeMap<String, BTreeSet<String>>,
    pub inert_values: BTreeSet<String>,
}

impl Runtime {
    /// Nothing provided: every API name is reported as unknown.
    pub fn empty() -> Runtime { Runtime::default() }

    /// Where the runtime files are: `$PEPTIDE_RUNTIME_DIR`, then a `data/runtime` beside the
    /// binary, then the converter crate's own source tree.
    pub fn locate() -> Option<PathBuf> {
        let mut c: Vec<PathBuf> = Vec::new();
        if let Ok(d) = std::env::var("PEPTIDE_RUNTIME_DIR") { c.push(PathBuf::from(d)); }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                c.push(dir.join("data").join("runtime"));
                c.push(dir.join("runtime"));
            }
        }
        c.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime"));
        c.into_iter().find(|p| p.join("core").is_dir())
    }

    /// Read every runtime file under `dir`.
    pub fn load(dir: &Path) -> Runtime {
        let mut rt = Runtime::default();
        let mut stack = vec![dir.to_path_buf()];
        let mut files: Vec<PathBuf> = Vec::new();
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p) } else if p.extension().is_some_and(|x| x == "hx") { files.push(p) }
            }
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("api_names.jsonc")) {
            rt.names = crate::model::rename::Names::parse(&text);
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("api_calls.jsonc")) {
            let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text))
                .unwrap_or_else(|e| panic!("runtime/api_calls.jsonc is malformed: {e}"));
            if let Some(o) = json.as_object() {
                rt.calls = o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
                if let Some(kinds) = o.get("by_kind").and_then(|v| v.as_object()) {
                    rt.kind_calls = kinds.iter().filter_map(|(kind, rows)| Some((kind.clone(), rows.as_object()?.iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()))).collect();
                }
            }
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("inert.jsonc")) {
            let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text))
                .unwrap_or_else(|e| panic!("runtime/inert.jsonc is malformed: {e}"));
            if let Some(o) = json["calls"].as_object() {
                rt.inert_calls = o.iter().map(|(k, v)| (k.clone(), v.as_array().map(|a| a.iter()
                    .filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default())).collect();
            }
            rt.inert_values = json["values"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("engine.names")) {
            rt.engine_names = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect();
        }
        files.sort();
        for p in files {
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let rel = p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            rt.read_file(&text);
            rt.files.insert(rel, text);
        }
        rt
    }

    /// The rows as one kind of object writes them: its own rows over everyone's.
    pub fn for_kind(mut self, kind: &str) -> Runtime {
        if let Some(rows) = self.kind_calls.get(kind) {
            for (k, v) in rows { self.calls.insert(k.clone(), v.clone()); }
        }
        self
    }

    /// Learn what one file defines.
    pub fn read_file(&mut self, text: &str) {
        let mut cur: Option<String> = None;
        for line in text.lines() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("// class ") {
                let mut parts = rest.split_whitespace();
                let name = parts.next().unwrap_or("").to_string();
                let extends = match (parts.next(), parts.next()) { (Some("extends"), Some(b)) => Some(b.to_string()), _ => None };
                self.classes.entry(name.clone()).or_insert_with(|| RuntimeClass { name: name.clone(), extends, ..Default::default() });
                self.globals.insert(name.clone());
                cur = Some(name);
                continue;
            }
            if !line.starts_with([' ', '\t']) {
                // a top-level definition: a global
                for kw in ["var ", "function "] {
                    if let Some(rest) = t.strip_prefix(kw) {
                        let n: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                        if !n.is_empty() { self.globals.insert(n); }
                    }
                }
            }
            // a member: `me.NAME = ...` in a class's build, or `CLASS.NAME = ...` for a static, of the
            // class it names wherever it is put (a stage's install puts statics on the API)
            let named: Option<(String, String)> = {
                let head: String = t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                let tail = &t[head.len()..];
                (!head.is_empty() && tail.starts_with('.') && self.classes.contains_key(&head) && Some(&head) != cur.as_ref())
                    .then(|| (head.clone(), tail[1..].to_string()))
            };
            let own = cur.as_ref().and_then(|c| t.strip_prefix(&format!("{c}.")).map(|r| r.to_string()));
            let on_object = named.is_none() && t.starts_with("me.");
            let target = match named {
                Some((c, rest)) => Some((c, rest)),
                None => cur.clone().zip(t.strip_prefix("me.").map(|r| r.to_string()).or(own)),
            };
            if let Some((c, rest)) = target {
                let c = &c;
                let rest = rest.as_str();
                let n: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if n.is_empty() || !rest[n.len()..].trim_start().starts_with('=') { continue }
                if rest[n.len()..].trim_start().trim_start_matches('=').trim_start().starts_with("function(rest)") {
                    self.variadic.insert(n.clone());
                }
                let takes_me = rest[n.len()..].trim_start().trim_start_matches('=').trim_start().starts_with("function(me");
                let class = self.classes.get_mut(c).unwrap();
                if on_object { class.own.insert(n.clone()); } else if takes_me { class.statics.insert(n.clone()); }
                if let Some(p) = n.strip_prefix("get_") { class.getters.insert(p.to_string()); }
                if let Some(p) = n.strip_prefix("set_") { class.setters.insert(p.to_string()); }
                // a static handed the object is the class's, not the object's: read as a value
                // (`self.destroy` handed to a listener) it is not on the object to read
                if on_object || !takes_me { class.members.insert(n); }
            }
        }
    }

    /// A package read into a model with SSF2's API names already the runtime's: what every path
    /// that compiles content reads it through. Returns the names the table would merge.
    pub fn read_package(&self, bytes: &[u8]) -> anyhow::Result<(crate::model::SourceModel, Vec<String>)> {
        let mut model = crate::model::SourceModel::read(bytes)?;
        let problems = crate::model::rename::rename(&mut model, &self.names);
        Ok((model, problems))
    }

    /// whether a top-level name is already taken, by the engine or by the runtime itself
    pub fn takes(&self, name: &str) -> bool { self.engine_names.contains(name) || self.globals.contains(name) }

    pub fn provides(&self, class: &str) -> bool {
        self.classes.contains_key(class) || self.classes.contains_key(super::short(class))
    }

    /// whether a runtime class, or one it extends, has a member
    pub fn has_member(&self, class: &str, name: &str) -> bool {
        let mut cur = self.classes.get(class).or_else(|| self.classes.get(super::short(class)));
        let mut n = 0;
        while let Some(c) = cur {
            if c.members.contains(name) || c.getters.contains(name) || c.setters.contains(name) { return true }
            n += 1;
            if n > 32 { break }
            cur = c.extends.as_ref().and_then(|e| self.classes.get(e));
        }
        false
    }

    /// the runtime class, this one or one it extends, that keeps a method as its static: the
    /// nearest class that has the name decides (one that puts it on the object keeps it there)
    pub fn static_of(&self, class: &str, name: &str) -> Option<String> {
        let mut cur = self.classes.get(class).or_else(|| self.classes.get(super::short(class)));
        let mut n = 0;
        while let Some(c) = cur {
            if c.own.contains(name) { return None }
            if c.statics.contains(name) { return Some(c.name.clone()) }
            n += 1;
            if n > 32 { break }
            cur = c.extends.as_ref().and_then(|e| self.classes.get(e));
        }
        None
    }

    pub fn has_getter(&self, class: &str, name: &str) -> bool {
        let mut cur = self.classes.get(class).or_else(|| self.classes.get(super::short(class)));
        let mut n = 0;
        while let Some(c) = cur {
            if c.getters.contains(name) { return true }
            n += 1;
            if n > 32 { break }
            cur = c.extends.as_ref().and_then(|e| self.classes.get(e));
        }
        false
    }

    /// every property name any runtime class reads or writes through an accessor
    pub fn accessors(&self) -> BTreeSet<String> {
        self.classes.values().flat_map(|c| c.getters.iter().chain(c.setters.iter()).cloned()).collect()
    }
}
