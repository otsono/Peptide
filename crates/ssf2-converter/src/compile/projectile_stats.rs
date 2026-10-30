//! What a projectile's declared stats have it do as it flies, as its own engine timers
//! (`runtime/projectile_stats.jsonc`): the table holds the code, this only picks the rows a
//! projectile's stats take and fills their values in.

use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Default)]
struct Code { when: Option<String>, yields_to: Option<String>, reads: Vec<String>, replaces: String, text: String }

#[derive(Debug, Default)]
pub struct ProjectileStats { code: Vec<(String, Code)> }

pub fn table() -> &'static ProjectileStats {
    static T: OnceLock<ProjectileStats> = OnceLock::new();
    T.get_or_init(|| {
        let Some(dir) = super::runtime::Runtime::locate() else { return ProjectileStats::default() };
        let Ok(text) = std::fs::read_to_string(dir.join("projectile_stats.jsonc")) else { return ProjectileStats::default() };
        let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text)).unwrap_or_default();
        let mut t = ProjectileStats::default();
        if let Some(o) = json["code"].as_object() {
            t.code = o.iter().filter_map(|(k, v)| Some((k.clone(), Code {
                when: v["when"].as_str().map(String::from),
                yields_to: v["yields_to"].as_str().map(String::from),
                reads: v["reads"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                replaces: v["replaces"].as_str()?.to_string(),
                text: v["text"].as_str()?.to_string(),
            }))).collect();
            // in the order the table writes them: each runs after the ones above it every frame
            t.code.sort_by_key(|(k, _)| o.keys().position(|x| x == k));
        }
        t
    })
}

fn holds(when: Option<&str>, v: f64) -> bool {
    match when { Some("positive") => v > 0.0, Some("negative") => v < 0.0, Some("nonnegative") => v >= 0.0,
                 Some("true") => v != 0.0, Some("false") => v == 0.0, _ => true }
}

impl ProjectileStats {
    /// the code a projectile's stats have it run, and the runtime pieces that code stands in for.
    /// a row is taken when the projectile states its stat (and its `when` holds), when nothing it
    /// `reads` is `written` (its code changes it as it flies, or another declaration sharing the
    /// clip states it otherwise), and when the row it `yields_to` is not taken. `units` fills
    /// `{@speed}`, `{@accel}` and `{@size}`
    pub fn rows(&self, stats: &BTreeMap<String, f64>, written: &dyn Fn(&str) -> bool, units: &[(&str, f64)]) -> (Vec<String>, Vec<String>) {
        let taken: Vec<&(String, Code)> = self.code.iter().filter(|(k, c)| stats.get(k).is_some_and(|v| holds(c.when.as_deref(), *v))
            && !written(k) && !c.reads.iter().any(|r| written(r))).collect();
        let keys: Vec<&str> = taken.iter().map(|(k, _)| k.as_str()).collect();
        let mut text = Vec::new();
        let mut replaced = Vec::new();
        for (k, c) in taken {
            if c.yields_to.as_deref().is_some_and(|y| keys.contains(&y)) { continue }
            let mut t = c.text.clone();
            for (u, f) in units { t = t.replace(&format!("{{@{u}}}"), &super::attack_stats::num(*f)); }
            text.push(super::attack_stats::fill(&t, stats[k], stats));
            if !replaced.contains(&c.replaces) { replaced.push(c.replaces.clone()) }
        }
        (text, replaced)
    }
}
