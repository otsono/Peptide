//! What the source's engine does in its own states, as this engine's animations
//! (`runtime/engine_animations.jsonc`): stats, first-frame code and points, by animation name.
//! The table says what; this reads it.

use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Default)]
pub struct EngineAnims {
    /// animation -> the stats it states, as `field: value` text
    pub stats: BTreeMap<String, String>,
    /// animation -> the code its first frame runs
    pub code: BTreeMap<String, String>,
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
        EngineAnims { stats: strings(&json["stats"]), code: strings(&json["code"]), points }
    })
}
