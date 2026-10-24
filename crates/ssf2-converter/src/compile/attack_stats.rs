//! An attack's stats as the engine's own animation stats (`runtime/attack_stats.jsonc`): the
//! declared ones go into AnimationStats, and a literal write the attack's code makes while it
//! plays becomes an `updateAnimationStats` call. The table says what maps where; this only reads
//! it and does the arithmetic.

use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
struct Row { field: String, how: String, when: Option<String>, clamp: bool, rank: i8 }

#[derive(Debug, Default)]
pub struct AttackStats {
    rows: BTreeMap<String, Vec<Row>>,
    /// the rows only a landing's own animation takes, over `rows`
    landing: BTreeMap<String, Vec<Row>>,
    /// what an attack that states nothing has, by engine field
    pub defaults: Vec<(String, String)>,
    /// the source's calls that land a fighter in its landing state, and the animation they play
    pub landings: BTreeMap<String, String>,
}

pub fn table() -> &'static AttackStats {
    static T: OnceLock<AttackStats> = OnceLock::new();
    T.get_or_init(|| {
        let Some(dir) = super::runtime::Runtime::locate() else { return AttackStats::default() };
        let Ok(text) = std::fs::read_to_string(dir.join("attack_stats.jsonc")) else { return AttackStats::default() };
        let json: serde_json::Value = serde_json::from_str(&crate::mappings::strip_jsonc(&text)).unwrap_or_default();
        let mut t = AttackStats::default();
        let read = |v: &serde_json::Value| -> BTreeMap<String, Vec<Row>> {
            v.as_object().map(|o| o.iter().map(|(k, rows)| (k.clone(), rows.as_array().map(|a| a.iter().filter_map(|r| Some(Row {
                field: r["field"].as_str()?.to_string(),
                how: r["as"].as_str().unwrap_or("bool").to_string(),
                when: r["when"].as_str().map(String::from),
                clamp: r["clamp"].as_bool().unwrap_or(false),
                rank: match r["rank"].as_str() { Some("over") => 1, Some("under") => -1, _ => 0 },
            })).collect()).unwrap_or_default())).collect()).unwrap_or_default()
        };
        t.rows = read(&json["stats"]);
        t.landing = read(&json["landing"]);
        if let Some(o) = json["landings"].as_object() {
            t.landings = o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
        }
        if let Some(o) = json["defaults"].as_object() {
            t.defaults = o.iter().map(|(k, v)| (k.clone(), v.to_string())).collect();
        }
        t
    })
}

fn num(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{}", (v * 1e6).round() / 1e6) }
}

impl AttackStats {
    /// a stat read back from the engine's field it maps to, as source text over `{v}` (the field's
    /// value): only a stat one plain row maps, whose engine value turns back into the source's
    pub fn read(&self, key: &str) -> Option<(String, String)> {
        let rows = self.rows.get(key)?;
        let [r] = rows.as_slice() else { return None };
        if r.when.is_some() { return None }
        let s = &crate::mappings::character_stats().scaling;
        let back = match r.how.as_str() {
            "bool" => "{v}".to_string(),
            "not" => "!({v})".to_string(),
            "speed" => format!("({{v}} / {})", s.velocity_scale()),
            "accel" => format!("({{v}} / {})", s.accel_scale()),
            "cap" => format!("({{v}} < 0 ? -1 : {{v}} / {})", s.velocity_scale()),
            _ => return None,
        };
        Some((r.field.clone(), back))
    }

    /// the fall caps a stat's value sets that hold the fighter's speed on the spot, as source text
    pub fn clamps(&self, key: &str, v: f64) -> Vec<String> {
        let Some(rows) = self.rows.get(key) else { return vec![] };
        let s = &crate::mappings::character_stats().scaling;
        rows.iter().filter(|r| r.clamp && v >= 0.0).map(|_| num(v * s.velocity_scale())).collect()
    }

    /// whether the table maps this stat at all
    pub fn maps(&self, key: &str) -> bool { self.rows.contains_key(key) }

    /// the engine fields a stat of the given number sets, as source text; None when the value
    /// falls outside every row that names the stat (a positive slowdown: a share, the runtime's)
    pub fn native(&self, key: &str, v: f64) -> Option<Vec<(String, String)>> {
        self.native_in(&self.rows, key, v)
    }

    /// the engine fields a stat sets on a landing's own animation
    pub fn native_landing(&self, key: &str, v: f64) -> Option<Vec<(String, String)>> {
        self.native_in(&self.landing, key, v).or_else(|| self.native(key, v))
    }

    fn native_in(&self, table: &BTreeMap<String, Vec<Row>>, key: &str, v: f64) -> Option<Vec<(String, String)>> {
        Some(self.ranked_in(table, key, v)?.into_iter().map(|(f, t, _)| (f, t)).collect())
    }

    /// a stat's engine fields put into a set being built: a field another stat of the same set
    /// already set is replaced only by a row that ranks as high (`rank` in the table), so the
    /// set comes out the same whatever order the source wrote its stats in. false when the table
    /// maps none of it
    pub fn put(&self, set: &mut Vec<(String, String)>, ranks: &mut BTreeMap<String, i8>, key: &str, v: f64, landing: bool) -> bool {
        let got = if landing { self.ranked_in(&self.landing, key, v).or_else(|| self.ranked_in(&self.rows, key, v)) } else { self.ranked_in(&self.rows, key, v) };
        let Some(got) = got else { return false };
        for (f, t, r) in got {
            if let Some(&had) = ranks.get(&f) { if r < had { continue } }
            set.retain(|(g, _)| *g != f);
            set.push((f.clone(), t));
            ranks.insert(f, r);
        }
        true
    }

    fn ranked_in(&self, table: &BTreeMap<String, Vec<Row>>, key: &str, v: f64) -> Option<Vec<(String, String, i8)>> {
        let rows = table.get(key)?;
        let s = &crate::mappings::character_stats().scaling;
        let mut out = Vec::new();
        for r in rows {
            let ok = match r.when.as_deref() {
                Some("negative") => v < 0.0,
                Some("positive") => v > 0.0,
                Some("true") => v != 0.0,
                Some("false") => v == 0.0,
                _ => true,
            };
            if !ok { continue }
            let text = match r.how.as_str() {
                "bool" => (v != 0.0).to_string(),
                "not" => (v == 0.0).to_string(),
                "speed" => num(v * s.velocity_scale()),
                "accel" => num(v * s.accel_scale()),
                "less" => num(-v * s.accel_scale()),
                "cap" => if v < 0.0 { "-1".into() } else { num(v * s.velocity_scale()) },
                "zero" => "0".into(),
                "unset" => "-1".into(),
                _ => continue,
            };
            out.push((r.field.clone(), text, r.rank));
        }
        if out.is_empty() { None } else { Some(out) }
    }
}
