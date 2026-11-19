//! The source's events this engine raises itself (`runtime/events.jsonc`): a listener the source's
//! code adds for one is the engine's own listener for it. The table says which; this reads it.

use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Default)]
pub struct EventTable {
    /// source event name -> the engine's event
    pub engine: BTreeMap<String, String>,
    /// source event name -> the engine's event, told with the source's states either side
    pub state: BTreeMap<String, String>,
    /// the engine's events a listener added without `persistent` is told of: one the engine raises
    /// while the animation it was added in still plays
    pub heard_in_animation: Vec<String>,
    /// a field of the record the source hands a listener -> (the engine's code, with `{arg}` the
    /// event, and what the value is)
    pub record: BTreeMap<String, (String, String)>,
    /// the fields the source's records carry that `record` does not translate
    pub unrecorded: Vec<String>,
    /// source event name -> the list of an item's own listeners its handler tells (`item.hx`)
    pub item: BTreeMap<String, String>,
    /// the fields of the record an item's handler hands its frame interrupt -> what each is
    pub interrupt: BTreeMap<String, String>,
    /// the events the game itself raises for a listener added on the API (the rest it never does)
    pub game: Vec<String>,
}

pub fn table() -> &'static EventTable {
    static T: OnceLock<EventTable> = OnceLock::new();
    T.get_or_init(|| {
        let Some(dir) = super::runtime::Runtime::locate() else { return EventTable::default() };
        let Ok(text) = std::fs::read_to_string(dir.join("events.jsonc")) else { return EventTable::default() };
        let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text)).unwrap_or_default();
        let read = |v: &serde_json::Value| -> BTreeMap<String, String> {
            v.as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default()
        };
        let heard = json["heard_in_animation"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        let strings = |v: &serde_json::Value| -> Vec<String> {
            v.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
        };
        let record = json["record"].as_object().map(|o| o.iter().filter_map(|(k, v)| {
            Some((k.clone(), (v["code"].as_str()?.to_string(), v["is"].as_str().unwrap_or("any").to_string())))
        }).collect()).unwrap_or_default();
        EventTable { engine: read(&json["engine"]), state: read(&json["state"]), heard_in_animation: heard,
            record, unrecorded: strings(&json["unrecorded"]), item: read(&json["item"]), interrupt: read(&json["interrupt"]), game: strings(&json["game"]) }
    })
}


/// the source event a listener call names: a string, or a constant of the API's event table
/// (`PeptideEvent.GROUND_TOUCH`, read from `api_constants.jsonc`)
pub fn named(runtime: &super::runtime::Runtime, e: &crate::model::ast::Expr) -> Option<String> {
    use crate::model::ast::Expr;
    match e {
        Expr::Str(s) => Some(s.clone()),
        Expr::Member(o, n) => {
            let Expr::Name(c) = &**o else { return None };
            // (the source's events, else a display event Flash names on its own `Event`)
            let v = runtime.constants.get(&format!("{}.{n}", super::short(c)))?;
            if !v.starts_with('"') { return None }
            Some(v.trim_matches('"').to_string())
        }
        _ => None,
    }
}

/// whether an event of the source's is one nothing here ever raises: neither this engine, nor an
/// object's own code (its attack's end, its meeting its owner, its end, its item handler's), nor
/// the runtime. a listener for one is never told, on either path
pub fn raised_by_nothing(runtime: &super::runtime::Runtime, name: &str) -> bool {
    let t = table();
    let told_by_code = ["charAttackComplete", "projCollide", "projCollideOwner", "projDestroyed", "itemDestroyed", "enemyDestroyed", "charSelfDestruct", "itemTossed", "charTransform", "charMetalChange"];
    !(t.engine.contains_key(name) || t.state.contains_key(name) || t.item.contains_key(name) || t.game.iter().any(|g| g == name)
        || told_by_code.contains(&name)
        || runtime.files.values().any(|f| f.contains(&format!("\"{name}\""))))
}
