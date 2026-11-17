//! What a stage's decompiled code says, read as text.
//!
//! A stage's scripts are written from the package's own classes (`emit::stage`).
//! The stage pipeline still takes a handful of decisions about the DATA it writes from the
//! decompiled text of those classes (`StageModel::stage_script` for the stage's class, each
//! object's `reconstructed_script` for its own): which sounds to bring across, which objects the
//! stage's code makes itself, where it puts them. Each of those questions is one method here, so
//! what the pipeline reads off the text is in one place and says what it is asking.
//!
//! Nothing here writes code. A question this answers is about what the source does, never about
//! what a thing is called.

use std::collections::BTreeSet;

use crate::stage_parser::{Hazard, StageModel};

/// The decompiled text of a stage's own class, and of its objects' classes beside it.
pub struct StageText<'a> {
    /// the stage's class
    stage: &'a str,
    /// the stage's class, then every object's class, one after another
    everything: String,
}

impl<'a> StageText<'a> {
    pub fn of(model: &'a StageModel) -> Self {
        let stage = model.stage_script.as_deref().unwrap_or("");
        let objects = model.hazards.iter().filter_map(|h| h.reconstructed_script.as_deref());
        let everything = std::iter::once(stage).chain(objects).collect::<Vec<_>>().join("\n");
        StageText { stage, everything }
    }

    /// Whether the stage's class names something by a string (`"platform3"`): it reaches that
    /// thing by name as it runs.
    pub fn names(&self, name: &str) -> bool {
        self.stage.contains(&format!("\"{name}\""))
    }

    /// The functions the stage's class defines.
    pub fn functions(&self) -> Vec<String> {
        crate::re!(r"(?m)^function\s+(\w+)\s*\(")
            .captures_iter(self.stage)
            .map(|c| c[1].to_string())
            .collect()
    }

    /// The fields the stage's class keeps: the ones the decompiler lists, and every one its code
    /// writes (`self.m_timer = ...`).
    pub fn fields(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        const LISTED: &str = "// [class-fields] ";
        if let Some(line) = self.stage.lines().find(|l| l.starts_with(LISTED)) {
            for word in line[LISTED.len()..].split_whitespace() {
                let name = word.split_once('=').map(|(n, _)| n).unwrap_or(word);
                out.insert(name.to_string());
            }
        }
        for c in crate::re!(r"(?m)^\s*self\.(\w+)\s*=[^=]").captures_iter(self.stage) {
            out.insert(c[1].to_string());
        }
        out
    }

    /// Whether some code of the stage (its own class or one of its objects') makes an object of
    /// `class`, by the class or by the content it was converted to (`content`).
    pub fn makes(&self, class: &str, content: &str) -> bool {
        let code = self.everything.as_str();
        [
            format!("new {class}("),
            format!("spawnEnemy({class}"),
            format!("spawnEnemy(\"{class}\""),
            format!("spawnItem({class}"),
            format!("getContent(\"{content}\")"),
        ]
        .iter()
        .any(|ask| code.contains(ask.as_str()))
    }

    /// Whether the stage's class makes an object of `class` over and over and lets each one go: a
    /// spawn in its update (a spawn in initialize happens once) into a temporary it never keeps.
    /// A stage that keeps what it makes (pushes it into a list, copies it into a field) has every
    /// one standing at once, and that is not a cycle.
    pub fn makes_and_lets_go(&self, class: &str) -> bool {
        let Some(update) = self.stage.find("function update") else { return false };
        let update = &self.stage[update..];
        let made_into = crate::dyn_re(&format!(r"(\w+)\s*=\s*[\w.]*spawnEnemy\(\s*{}\s*\)", regex::escape(class)));
        let made: Vec<String> = made_into.captures_iter(update).map(|c| c[1].to_string()).collect();
        made.iter().any(|var| {
            // the decompiler names a function's own temporaries `_v<n>`: anything with a real
            // name is a field, and a field holds the object for later
            let temporary = crate::re!(r"^_v\d+$").is_match(var);
            let v = regex::escape(var);
            let kept = crate::dyn_re(&format!(r"(\.push\(\s*{v}\s*\)|[\w.]+\s*=\s*{v}\s*;)")).is_match(update);
            temporary && !kept
        })
    }

    /// Where the stage's class puts an object of `class` it makes (`v = spawnEnemy(C);
    /// v.setX(120);`), in the source's own space: each axis it states.
    pub fn places(&self, class: &str) -> (Option<f64>, Option<f64>) {
        let made_into = crate::dyn_re(&format!(r"(\w+)\s*=\s*[\w.]*spawnEnemy\(\s*{}\s*\)", regex::escape(class)));
        let Some(c) = made_into.captures(self.stage) else { return (None, None) };
        let v = regex::escape(&c[1]);
        let stated = |axis: &str| crate::dyn_re(&format!(r"{v}\.set{axis}\(\s*(-?[\d.]+)\s*\)"))
            .captures(self.stage)
            .and_then(|m| m[1].parse::<f64>().ok());
        (stated("X"), stated("Y"))
    }
}

/// Whether an object's class listens for its own hurt: it expects to be attacked (a block that
/// breaks when hit).
pub fn listens_for_hits(hz: &Hazard) -> bool {
    let code = hz.reconstructed_script.as_deref().unwrap_or("");
    code.contains("HIT_RECEIVED") || code.contains("ENEMY_HURT")
}

/// Whether an object's class takes itself off the stage (destroys or kills itself).
pub fn ends_its_own_life(hz: &Hazard) -> bool {
    let code = hz.reconstructed_script.as_deref().unwrap_or("");
    code.contains(".destroy()") || code.contains(".kill()")
}

/// The sounds an object's class plays by name.
pub fn sounds_played(hz: &Hazard) -> Vec<String> {
    let code = hz.reconstructed_script.as_deref().unwrap_or("");
    crate::re!(r#"AudioClip\.play\(\s*"([^"]+)"\s*\)"#)
        .captures_iter(code)
        .map(|c| c[1].to_string())
        .collect()
}
