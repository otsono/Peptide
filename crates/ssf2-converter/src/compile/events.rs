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
        EventTable { engine: read(&json["engine"]), state: read(&json["state"]) }
    })
}

impl EventTable {
    /// the tables as the script's data: a function from a source event's name to the engine's
    /// event, for listeners the runtime's own code adds by name
    pub fn data(&self) -> String {
        let cases = |m: &BTreeMap<String, String>| m.iter()
            .map(|(k, v)| format!("case {}: return {v};", super::quote(k))).collect::<Vec<_>>().join(" ");
        format!("DATA.engineEvent = function(t) {{ switch (t) {{ {} default: return null; }} }};\n\
                 DATA.stateEvent = function(t) {{ switch (t) {{ {} default: return null; }} }};\n",
            cases(&self.engine), cases(&self.state))
    }
}

/// the source event a listener call names: a string, or a constant of the runtime's event class
/// (`PeptideEvent.GROUND_TOUCH`, read from the runtime's own definition of it)
pub fn named(runtime: &super::runtime::Runtime, e: &crate::model::ast::Expr) -> Option<String> {
    use crate::model::ast::Expr;
    match e {
        Expr::Str(s) => Some(s.clone()),
        Expr::Member(o, n) => {
            let Expr::Name(c) = &**o else { return None };
            if super::short(c) != "PeptideEvent" { return None }
            let line = format!("PeptideEvent.{n} = \"");
            runtime.files.values().find_map(|t| {
                let at = t.find(&line)? + line.len();
                Some(t[at..].split('"').next()?.to_string())
            })
        }
        _ => None,
    }
}
