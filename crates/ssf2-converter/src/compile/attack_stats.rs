//! An attack's stats as the engine's own animation stats (`runtime/attack_stats.jsonc`): the
//! declared ones go into AnimationStats, and a literal write the attack's code makes while it
//! plays becomes an `updateAnimationStats` call. The table says what maps where; this only reads
//! it and does the arithmetic.

use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
struct Row { field: String, how: String, when: Option<String>, clamp: bool, rank: i8, value: Option<String>, off: Option<String>, with: Option<String> }

#[derive(Debug, Default)]
pub struct AttackStats {
    rows: BTreeMap<String, Vec<Row>>,
    /// the rows only a landing's own animation takes, over `rows`
    landing: BTreeMap<String, Vec<Row>>,
    /// what an attack that states nothing has, by engine field
    pub defaults: Vec<(String, String)>,
    /// the source's calls that land a fighter in its landing state, and the animation they play
    pub landings: BTreeMap<String, String>,
    /// code an attack's animations run on their first frame for a stat it declares
    code: BTreeMap<String, Code>,
}

#[derive(Debug, Default)]
struct Code { when: Option<String>, unless: Option<String>, first: bool, call: String, code: Vec<String>, same_as: Option<String> }

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
                rank: match r["rank"].as_str() { Some("over") => 1, Some("under") => -1, Some("least") => -2, _ => 0 },
                value: r["value"].as_str().map(String::from),
                off: r["off"].as_str().map(String::from),
                with: r["with"].as_str().map(String::from),
            })).collect()).unwrap_or_default())).collect()).unwrap_or_default()
        };
        t.rows = read(&json["stats"]);
        t.landing = read(&json["landing"]);
        if let Some(o) = json["landings"].as_object() {
            t.landings = o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
        }
        if let Some(o) = json["code"].as_object() {
            t.code = o.iter().filter_map(|(k, v)| Some((k.clone(), Code {
                when: v["when"].as_str().map(String::from),
                unless: v["unless"].as_str().map(String::from),
                first: v["first"].as_bool().unwrap_or(false),
                call: v["call"].as_str()?.to_string(),
                code: v["code"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                same_as: v["same_as"].as_str().map(String::from),
            }))).collect();
        }
        if let Some(o) = json["defaults"].as_object() {
            t.defaults = o.iter().map(|(k, v)| (k.clone(), v.to_string())).collect();
        }
        t
    })
}

/// a code row's text with `{v}` the stat's value and `{name}` the value of another stat stated
/// alongside (`{name|d}` gives `d` when it states none, else 0)
pub(crate) fn fill(text: &str, v: f64, stats: &BTreeMap<String, f64>) -> String {
    let named = crate::re!(r"\{([A-Za-z_]\w*)(?:\|(-?[0-9.]+))?\}");
    let t = text.replace("{v}", &num(v));
    named.replace_all(&t, |m: &regex::Captures| {
        stats.get(&m[1]).map(|x| num(*x)).unwrap_or_else(|| m.get(2).map_or("0".to_string(), |d| d.as_str().to_string()))
    }).into_owned()
}

pub(crate) fn num(v: f64) -> String {
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

    /// the calls an attack's animations make on their first frame for what it declares: each `code`
    /// entry whose stat it declares (and whose `when` holds, and whose `unless` it does not state
    /// false), with `{v}` its value and `{name}` the value of another stat it declares (`{name|d}`
    /// gives `d` when it declares none, else 0). each comes with whether only the attack's own
    /// animation runs it (`first`), rather than every one under its label, and the row it is
    pub fn first_frame(&self, stats: &BTreeMap<String, f64>) -> Vec<(bool, String, String)> {
        let mut out = Vec::new();
        for (key, c) in &self.code {
            let Some(&v) = stats.get(key) else { continue };
            let ok = match c.when.as_deref() { Some("positive") => v > 0.0, Some("negative") => v < 0.0, Some("true") => v != 0.0, Some("false") => v == 0.0, _ => true };
            if !ok || c.unless.as_ref().is_some_and(|u| stats.get(u) == Some(&0.0)) { continue }
            out.push((c.first, fill(&c.call, v, stats), key.clone()));
        }
        out
    }

    /// the code the fighter's script holds for the rows its attacks take, each once (a row that is
    /// `same_as` another writes that one's)
    pub fn code(&self, taken: &std::collections::BTreeSet<String>) -> Vec<String> {
        let owners: std::collections::BTreeSet<&str> = taken.iter().filter_map(|k| self.code.get(k)
            .map(|c| c.same_as.as_deref().unwrap_or(k))).collect();
        owners.into_iter().filter_map(|k| self.code.get(k)).flat_map(|c| c.code.iter().cloned()).collect()
    }

    /// whether a row of this stat holds only alongside one of the given stats
    pub fn joins(&self, key: &str, stated: &dyn Fn(&str) -> bool) -> bool {
        self.rows.get(key).is_some_and(|rows| rows.iter().any(|r| r.with.as_deref().is_some_and(stated)))
    }

    /// whether the table maps this stat at all
    pub fn maps(&self, key: &str) -> bool { self.rows.contains_key(key) }

    /// the engine fields a stat of the given number sets, as source text; None when the value
    /// falls outside every row that names the stat (a positive slowdown: a share, the runtime's)
    pub fn native(&self, key: &str, v: f64) -> Option<Vec<(String, String)>> {
        self.native_in(&self.rows, key, v)
    }

    /// the engine fields a stat sets when its value is only known as it plays, as source text over
    /// `{v}`: only a stat with a row that sets a value while it is true and another (its `off`)
    /// when it is not
    pub fn dynamic(&self, key: &str) -> Option<(String, String)> {
        let r = self.rows.get(key)?.iter().find(|r| r.how == "value" && r.when.as_deref() == Some("true") && r.with.is_none() && r.off.is_some())?;
        Some((r.field.clone(), format!("(AS3.truthy({{v}}) ? {} : {})", r.value.as_ref()?, r.off.as_ref()?)))
    }

    /// the engine fields a stat sets on a landing's own animation
    pub fn native_landing(&self, key: &str, v: f64) -> Option<Vec<(String, String)>> {
        self.native_in(&self.landing, key, v).or_else(|| self.native(key, v))
    }

    fn native_in(&self, table: &BTreeMap<String, Vec<Row>>, key: &str, v: f64) -> Option<Vec<(String, String)>> {
        Some(self.ranked_in(table, key, v, &|_| None)?.into_iter().map(|(f, t, _)| (f, t)).collect())
    }

    /// a stat's engine fields put into a set being built: a field another stat of the same set
    /// already set is replaced only by a row that ranks as high (`rank` in the table), so the
    /// set comes out the same whatever order the source wrote its stats in. false when the table
    /// maps none of it. `along` answers the other stats stated with it (a row `with` another
    /// stat holds only when that one is stated true alongside)
    pub fn put(&self, set: &mut Vec<(String, String)>, ranks: &mut BTreeMap<String, i8>, key: &str, v: f64, landing: bool,
               along: &dyn Fn(&str) -> Option<f64>) -> bool {
        let got = if landing { self.ranked_in(&self.landing, key, v, along).or_else(|| self.ranked_in(&self.rows, key, v, along)) } else { self.ranked_in(&self.rows, key, v, along) };
        let Some(got) = got else { return false };
        for (f, t, r) in got {
            if let Some(&had) = ranks.get(&f) { if r < had { continue } }
            set.retain(|(g, _)| *g != f);
            set.push((f.clone(), t));
            ranks.insert(f, r);
        }
        true
    }

    fn ranked_in(&self, table: &BTreeMap<String, Vec<Row>>, key: &str, v: f64, along: &dyn Fn(&str) -> Option<f64>) -> Option<Vec<(String, String, i8)>> {
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
            if let Some(w) = &r.with { if along(w).is_none_or(|x| x == 0.0) { continue } }
            let text = match r.how.as_str() {
                "bool" => (v != 0.0).to_string(),
                "not" => (v == 0.0).to_string(),
                "speed" => num(v * s.velocity_scale()),
                "accel" => num(v * s.accel_scale()),
                "less" => num(-v * s.accel_scale()),
                "cap" => if v < 0.0 { "-1".into() } else { num(v * s.velocity_scale()) },
                "zero" => "0".into(),
                "unset" => "-1".into(),
                "value" => match &r.value { Some(t) => t.clone(), None => continue },
                _ => continue,
            };
            out.push((r.field.clone(), text, r.rank));
        }
        if out.is_empty() { None } else { Some(out) }
    }
}
