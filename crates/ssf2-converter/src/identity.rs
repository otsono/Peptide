//! What a converted fighter is called and what it is filed under.
//!
//! The name is the one the fighter declares for itself. The id is the one its package declares,
//! which is right for every fighter that is what its slot says it is. A fighter built into
//! another's slot keeps that slot's id while declaring a name of its own, and the game's own
//! roster is what shows it: the roster still lists the slot under the name it had. Such a fighter
//! is filed under its own name, so it sits beside the one whose slot it borrowed and not on it.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// What one fighter's project is filed under and shown as.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub id: String,
    pub name: String,
}

/// A name as an id: its letters and digits, lowercased.
pub fn reduce(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase()
}

/// An id a person gave: letters, digits and underscores, starting with a letter.
pub fn check_id(id: &str) -> Result<String> {
    let ok = id.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok { bail!("{id:?} is not an id: letters, digits and underscores, starting with a letter"); }
    Ok(id.to_lowercase())
}

/// `declared` is the id the package files the fighter under and `display` the name the fighter
/// states; `roster` answers what the game's own roster calls an id (nothing when the game is not
/// at hand or does not list it).
pub fn decide(declared: &str, display: Option<&str>, roster: &dyn Fn(&str) -> Option<String>) -> Identity {
    let display = display.map(str::trim).filter(|d| !d.is_empty());
    let name = display.unwrap_or(declared).to_string();
    let own = display.map(reduce).filter(|d| !d.is_empty());
    let id = match (own, roster(declared)) {
        (Some(own), Some(listed)) if own != reduce(&listed) && own != declared => {
            // the name it took may itself be a slot the roster fills
            if roster(&own).is_some() { format!("{own}{declared}") } else { own }
        }
        _ => declared.to_string(),
    };
    Identity { id, name }
}

/// Writes the name a fighter goes by into its project's manifest.
pub fn set_name(project: &Path, id: &str, name: &str) -> Result<()> {
    let path = project.join("library/manifest.json");
    let mut m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?)?;
    for c in m["content"].as_array_mut().into_iter().flatten() {
        if c["type"] != "character" || c["id"] != id { continue }
        c["name"] = name.into();
        c["description"] = format!("{name} — converted from Super Smash Flash 2").into();
        if let Some(info) = c.pointer_mut("/metadata/ui/css/info").and_then(|i| i.as_object_mut()) {
            info.insert("description".into(), format!("{name} — ported from SSF2").into());
        }
    }
    std::fs::write(&path, serde_json::to_string(&m)?)?;
    Ok(())
}

/// Files a finished project under another id: its package, the fighter in it, the fighter's
/// entity and every script that asks the package for the fighter by id.
pub fn rename_project(project: &Path, from: &str, to: &str) -> Result<()> {
    let lib = project.join("library");
    let path = lib.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?)?;
    m["resourceId"] = to.into();
    for c in m["content"].as_array_mut().into_iter().flatten() {
        if c["id"] == from { c["id"] = to.into(); }
    }
    std::fs::write(&path, serde_json::to_string(&m)?)?;

    let (asked, asks) = (format!("getContent(\"{from}\")"), format!("getContent(\"{to}\")"));
    let (was, is) = (format!("\"id\": \"{from}\""), format!("\"id\": \"{to}\""));
    let mut dirs = vec![lib.join("scripts"), lib.join("entities")];
    while let Some(dir) = dirs.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() { dirs.push(p); continue }
            let (find, put) = match p.extension().and_then(|x| x.to_str()) {
                Some("hx") => (&asked, &asks),
                Some("entity") => (&was, &is),
                _ => continue,
            };
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            if text.contains(find.as_str()) { std::fs::write(&p, text.replace(find.as_str(), put))?; }
        }
    }
    for e in std::fs::read_dir(project)?.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "fraytools") {
            std::fs::rename(&p, project.join(format!("{to}.fraytools")))?;
        }
    }
    Ok(())
}

/// Puts a project made aside where it belongs, over whatever an earlier conversion left there.
pub fn install(made: &Path, home: &Path) -> Result<()> {
    std::fs::create_dir_all(home)?;
    for e in std::fs::read_dir(made)?.flatten() {
        let dest = home.join(e.file_name());
        if dest.is_dir() { std::fs::remove_dir_all(&dest)?; } else if dest.exists() { std::fs::remove_file(&dest)?; }
        std::fs::rename(e.path(), &dest).with_context(|| format!("moving {} into place", dest.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster(id: &str) -> Option<String> {
        match id { "mario" => Some("Mario".into()), "luigi" => Some("Luigi".into()), "dedede" => Some("King Dedede".into()), _ => None }
    }

    #[test]
    fn a_fighter_that_is_its_slot_keeps_the_slots_id() {
        assert_eq!(decide("mario", Some("Mario"), &roster), Identity { id: "mario".into(), name: "Mario".into() });
        assert_eq!(decide("dedede", Some("King Dedede"), &roster), Identity { id: "dedede".into(), name: "King Dedede".into() });
    }

    #[test]
    fn a_fighter_in_anothers_slot_is_filed_under_its_own_name() {
        assert_eq!(decide("mario", Some("Kung Fu Man"), &roster), Identity { id: "kungfuman".into(), name: "Kung Fu Man".into() });
        assert_eq!(decide("mario", Some("Luigi"), &roster).id, "luigimario");
    }

    #[test]
    fn with_no_roster_or_no_name_the_declared_id_stands() {
        let none = |_: &str| None;
        assert_eq!(decide("mario", Some("Kung Fu Man"), &none), Identity { id: "mario".into(), name: "Kung Fu Man".into() });
        assert_eq!(decide("gigabowser", Some("Bowser"), &roster).id, "gigabowser");
        assert_eq!(decide("mario", None, &roster), Identity { id: "mario".into(), name: "mario".into() });
    }

    #[test]
    fn an_id_is_letters_digits_and_underscores() {
        assert_eq!(check_id("KungFu_2").unwrap(), "kungfu_2");
        assert!(check_id("2fast").is_err());
        assert!(check_id("kung fu").is_err());
    }
}
