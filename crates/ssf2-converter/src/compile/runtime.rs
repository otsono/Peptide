//! What the API provides and the shared code every script is filled from, read from `runtime/`.
//!
//! The API's classes and their members are `api_classes.jsonc`; its names, calls, constants and
//! the Flash classes the engine has its own version of are the other tables there; the skeletons
//! and helpers every object's script is filled from are `native/*.hx`. Nothing here is a second
//! copy of those files.

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
    /// one of the engine's own classes (`native_classes.jsonc`): its members are the engine's to
    /// answer, so none is reported unknown
    pub native: bool,
}

/// A Flash class content uses that the engine's scripts have their own version of
/// (`native_classes.jsonc`).
#[derive(Default, Clone, Debug)]
pub struct NativeClass {
    /// content's `new Class(...)`, as a template of its arguments
    pub make: Option<String>,
    /// the engine's class a type test checks against
    pub is: Option<String>,
    /// for code written as Fraymakers code only: the runtime keeps its own version
    pub native_only: bool,
    /// the class's own functions the engine's class lacks, as templates of their arguments
    pub statics: BTreeMap<String, String>,
}

#[derive(Default, Clone, Debug)]
pub struct Runtime {
    pub classes: BTreeMap<String, RuntimeClass>,
    pub globals: BTreeSet<String>,
    /// every file, by its path under the runtime directory, and its text
    pub files: BTreeMap<String, String>,
    /// the names the engine's own script scope has (`engine.names`)
    pub engine_names: BTreeSet<String>,
    /// SSF2's API names -> the runtime's (`api_names.jsonc`)
    pub names: crate::model::rename::Names,
    /// methods that do nothing here, by class, and classes whose making only makes a value
    /// (`inert.jsonc`)
    pub inert_calls: BTreeMap<String, BTreeSet<String>>,
    pub inert_values: BTreeSet<String>,
    /// Flash's classes the engine has its own version of, by name (`native_classes.jsonc`)
    pub natives: BTreeMap<String, NativeClass>,
    /// the API's tables of values (`api_constants.jsonc`): read only as their values, at conversion
    pub tables: BTreeSet<String>,
    /// a runtime class's constants (`Class.NAME = literal;`, names in capitals), by `Class.NAME`:
    /// content that reads one gets the value written in place, and the table stays out of its script
    pub constants: BTreeMap<String, String>,
    /// the skeletons of objects written as plain Fraymakers code (`native/*.hx`, by kind): no part
    /// of the runtime, so read apart from it
    pub native_files: BTreeMap<String, String>,
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
        c.into_iter().find(|p| p.join("native").is_dir())
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
        if let Ok(text) = std::fs::read_to_string(dir.join("inert.jsonc")) {
            let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text))
                .unwrap_or_else(|e| panic!("runtime/inert.jsonc is malformed: {e}"));
            if let Some(o) = json["calls"].as_object() {
                rt.inert_calls = o.iter().map(|(k, v)| (k.clone(), v.as_array().map(|a| a.iter()
                    .filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default())).collect();
            }
            rt.inert_values = json["values"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("native_classes.jsonc")) {
            let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text))
                .unwrap_or_else(|e| panic!("runtime/native_classes.jsonc is malformed: {e}"));
            for (name, row) in json.as_object().into_iter().flatten() {
                let text = |k: &str| row.get(k).and_then(|v| v.as_str()).map(String::from);
                let rows = |k: &str| row.get(k).and_then(|v| v.as_object()).map(|o| o.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect::<BTreeMap<_, _>>()).unwrap_or_default();
                for (k, v) in rows("constants") { rt.constants.insert(format!("{name}.{k}"), v); }
                let native_only = row.get("native_only").and_then(|v| v.as_bool()).unwrap_or(false);
                rt.natives.insert(name.clone(), NativeClass { make: text("make"), is: text("is"), statics: rows("statics"), native_only });
                if native_only { continue }
                rt.classes.insert(name.clone(), RuntimeClass { name: name.clone(), native: true, ..Default::default() });
                rt.globals.insert(name.clone());
            }
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("api_constants.jsonc")) {
            let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text))
                .unwrap_or_else(|e| panic!("runtime/api_constants.jsonc is malformed: {e}"));
            for (name, rows) in json.as_object().into_iter().flatten() {
                for (k, v) in rows.as_object().into_iter().flatten() {
                    if let Some(v) = v.as_str() { rt.constants.insert(format!("{name}.{k}"), v.to_string()); }
                }
                rt.classes.insert(name.clone(), RuntimeClass { name: name.clone(), native: true, ..Default::default() });
                rt.tables.insert(name.clone());
            }
        }
        // the API's classes, as declarations (`api_classes.jsonc`)
        if let Ok(text) = std::fs::read_to_string(dir.join("api_classes.jsonc")) {
            let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text))
                .unwrap_or_else(|e| panic!("runtime/api_classes.jsonc is malformed: {e}"));
            let set = |v: &serde_json::Value| -> BTreeSet<String> { v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
            for (name, row) in json["classes"].as_object().into_iter().flatten() {
                rt.classes.entry(name.clone()).or_insert_with(|| RuntimeClass {
                    name: name.clone(), extends: row["extends"].as_str().map(String::from),
                    members: set(&row["members"]), getters: set(&row["getters"]), setters: set(&row["setters"]),
                    ..Default::default() });
                rt.globals.insert(name.clone());
            }
            rt.globals.extend(set(&json["globals"]));
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("engine.names")) {
            rt.engine_names = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect();
        }
        files.sort();
        for p in files {
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let rel = p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            if let Some(kind) = rel.strip_prefix("native/").and_then(|r| r.strip_suffix(".hx")) {
                rt.native_files.insert(kind.to_string(), text.clone());
            }
            rt.files.insert(rel, text);
        }
        rt.constants.retain(|_, v| !v.is_empty());
        rt
    }

    /// the engine's version of a Flash class, by its name with or without its package
    pub fn native(&self, class: &str) -> Option<&NativeClass> {
        self.natives.get(class).or_else(|| self.natives.get(super::short(class)))
    }



    /// A package read into a model with SSF2's API names already the runtime's: what every path
    /// that compiles content reads it through. Returns the names the table would merge.
    pub fn read_package(&self, bytes: &[u8]) -> anyhow::Result<(crate::model::SourceModel, Vec<String>)> {
        let mut model = crate::model::SourceModel::read(bytes)?;
        let problems = crate::model::rename::rename(&mut model, &self.names);
        Ok((model, problems))
    }


    pub fn provides(&self, class: &str) -> bool {
        self.classes.contains_key(class) || self.classes.contains_key(super::short(class))
    }

    /// whether a runtime class, or one it extends, has a member
    pub fn has_member(&self, class: &str, name: &str) -> bool {
        let mut cur = self.classes.get(class).or_else(|| self.classes.get(super::short(class)));
        let mut n = 0;
        while let Some(c) = cur {
            if c.native || c.members.contains(name) || c.getters.contains(name) || c.setters.contains(name) { return true }
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
