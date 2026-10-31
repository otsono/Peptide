//! What a projectile's declared stats have it do as it flies, as code in its own script
//! (`runtime/projectile_stats.jsonc`): the table holds the code, this only picks the rows a
//! projectile's stats take and fills their values in.

use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Default)]
struct Code { when: Option<String>, yields_to: Option<String>, same_as: Option<String>, reads: Vec<String>, replaces: String, code: Vec<String>, call: String }

/// what a projectile's stats take: the lines written into its script, the calls its update makes
/// each frame, and the runtime pieces they stand in for
#[derive(Debug, Default)]
pub struct Taken { pub code: Vec<String>, pub calls: Vec<String>, pub replaced: Vec<String>, pub frame: Option<String> }

#[derive(Debug, Default)]
pub struct ProjectileStats { code: Vec<(String, Code)>, frame: Vec<String> }

pub fn table() -> &'static ProjectileStats {
    static T: OnceLock<ProjectileStats> = OnceLock::new();
    T.get_or_init(|| {
        let Some(dir) = super::runtime::Runtime::locate() else { return ProjectileStats::default() };
        let Ok(text) = std::fs::read_to_string(dir.join("projectile_stats.jsonc")) else { return ProjectileStats::default() };
        let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text)).unwrap_or_default();
        let mut t = ProjectileStats::default();
        t.frame = json["frame"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        if let Some(o) = json["code"].as_object() {
            t.code = o.iter().filter_map(|(k, v)| Some((k.clone(), Code {
                when: v["when"].as_str().map(String::from),
                yields_to: v["yields_to"].as_str().map(String::from),
                same_as: v["same_as"].as_str().map(String::from),
                reads: v["reads"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                replaces: v["replaces"].as_str()?.to_string(),
                code: v["code"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                call: v["call"].as_str().unwrap_or_default().to_string(),
            }))).collect();
            // a row that writes another's code takes it from there
            let shared: Vec<(usize, Vec<String>, String)> = t.code.iter().enumerate().filter_map(|(i, (_, c))| {
                let (_, o) = t.code.iter().find(|(k, _)| Some(k) == c.same_as.as_ref())?;
                Some((i, o.code.clone(), o.call.clone()))
            }).collect();
            for (i, code, call) in shared { t.code[i].1.code = code; t.code[i].1.call = call; }
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
    /// the call the update makes to `frame`'s function
    pub fn frame_call(&self) -> Option<String> {
        self.frame.iter().find_map(|l| l.strip_prefix("function ")?.split('(').next().map(|n| format!("{n}();")))
    }

    /// what a projectile's stats write into its script, what its update calls each frame, and the
    /// runtime pieces that code stands in for.
    /// a row is taken when the projectile states its stat (and its `when` holds), when nothing it
    /// `reads` is `written` (its code changes it as it flies, or another declaration sharing the
    /// clip states it otherwise), and when the row it `yields_to` is not taken. `units` fills
    /// `{@speed}`, `{@accel}` and `{@size}`; a row `when` `effect` takes a stat in `names` that
    /// `effect` resolves to the engine's effect stats, and fills them in as `{@effect}`
    pub fn rows(&self, stats: &BTreeMap<String, f64>, names: &BTreeMap<String, String>, effect: &dyn Fn(&str) -> Option<String>,
                written: &dyn Fn(&str) -> bool, units: &[(&str, f64)]) -> Taken {
        // a stat that names an effect holds when the object can make it
        let effect_of = |k: &str| names.get(k).filter(|n| !n.is_empty()).and_then(|n| effect(n));
        let states = |k: &str, c: &Code| if c.when.as_deref() == Some("effect") { effect_of(k).is_some() }
            else { stats.get(k).is_some_and(|v| holds(c.when.as_deref(), *v)) };
        let taken: Vec<&(String, Code)> = self.code.iter().filter(|(k, c)| states(k, c)
            && !written(k) && !c.reads.iter().any(|r| written(r))).collect();
        let keys: Vec<&str> = taken.iter().map(|(k, _)| k.as_str()).collect();
        let mut out = Taken::default();
        for (k, c) in taken {
            if c.yields_to.as_deref().is_some_and(|y| keys.contains(&y)) { continue }
            for line in &c.code {
                let mut t = line.clone();
                for (u, f) in units { t = t.replace(&format!("{{@{u}}}"), &super::attack_stats::num(*f)); }
                if let Some(e) = effect_of(k) { t = t.replace("{@effect}", &e); }
                out.code.push(super::attack_stats::fill(&t, stats.get(k).copied().unwrap_or(0.0), stats));
            }
            out.calls.push(c.call.clone());
            if !out.replaced.contains(&c.replaces) { out.replaced.push(c.replaces.clone()) }
        }
        // the function the update calls, running each taken row's call
        if !out.calls.is_empty() {
            for line in &self.frame {
                match line.find("{calls}") {
                    Some(at) => out.code.extend(out.calls.iter().map(|c| format!("{}{c}", &line[..at]))),
                    None => out.code.push(line.clone()),
                }
            }
            out.frame = self.frame_call();
        }
        out
    }
}
