//! What the source's engine does in its own states, as this engine's animations
//! (`runtime/engine_animations.jsonc`): stats, first-frame code, the functions that code runs, and
//! points, by animation name.
//! The table says what; this reads it.

use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Default)]
pub struct EngineAnims {
    /// animation -> the stats it states, as `field: value` text
    pub stats: BTreeMap<String, String>,
    /// animation -> the code its first frame runs, each part with the animation the fighter must
    /// have for it to run at all
    pub code: BTreeMap<String, Vec<(Option<String>, String)>>,
    /// name -> a function the fighter's script holds when its animations' code names it
    pub functions: BTreeMap<String, Vec<String>>,
    /// layer name -> (x, y, the animations that carry it)
    pub points: BTreeMap<String, (f64, f64, Vec<String>)>,
}

pub fn table() -> &'static EngineAnims {
    static T: OnceLock<EngineAnims> = OnceLock::new();
    T.get_or_init(|| {
        let json: serde_json::Value = super::runtime::Runtime::locate()
            .and_then(|d| std::fs::read_to_string(d.join("engine_animations.jsonc")).ok())
            .and_then(|t| serde_json::from_str(&crate::mappings::strip_jsonc(&t)).ok()).unwrap_or_default();
        let strings = |v: &serde_json::Value| -> BTreeMap<String, String> {
            v.as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default()
        };
        let points = json["points"].as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), (
            v["x"].as_f64().unwrap_or(0.0), v["y"].as_f64().unwrap_or(0.0),
            v["animations"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        ))).collect()).unwrap_or_default();
        // a part is one string or an array of lines
        let lines = |v: &serde_json::Value| -> Option<String> {
            match v {
                serde_json::Value::String(t) => Some(t.clone()),
                serde_json::Value::Array(a) => Some(a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("\n")),
                _ => None,
            }
        };
        let mut code: BTreeMap<String, Vec<(Option<String>, String)>> = json["code"].as_object().map(|o| o.iter()
            .filter_map(|(k, v)| Some((k.clone(), vec![(None, lines(v)?)]))).collect()).unwrap_or_default();
        // a watch several animations start: each one's first frame runs it after its own code
        for w in json["watches"].as_object().into_iter().flat_map(|o| o.values()) {
            let Some(text) = lines(&w["start"]) else { continue };
            let needs = w["needs"].as_str().map(String::from);
            for a in w["animations"].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
                code.entry(a.to_string()).or_default().push((needs.clone(), text.clone()));
            }
        }
        let functions = json["functions"].as_object().map(|o| o.iter().map(|(k, v)| (k.clone(),
            v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default())).collect()).unwrap_or_default();
        EngineAnims { stats: strings(&json["stats"]), code, functions, points }
    })
}
