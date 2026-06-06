//! vocab — the ONE place the `commands.hsx` vocabulary is declared and where the
//! Fraymakers↔SSF2 differences are reconciled. This sits at the interpreter
//! level (a sibling of [`crate::interpreter`]), ABOVE either engine backend, so a
//! command's meaning is defined once and both engines agree on it:
//!
//!   * Fraymakers ([`crate::debug_target::FraymakersTarget`]) forwards the names
//!     verbatim as hscript; `commands.hsx`, loaded into the engine interpreter,
//!     is what DEFINES `match` / `getCharacters()` / `p0` / `.body.x` there.
//!   * SSF2 ([`crate::ssf2_target::Ssf2Target`]) has no script interpreter, so it
//!     LOWERS the same names to AVM2 reflection verbs using the tables here.
//!
//! Because the mapping lives here and not inside the SSF2 backend, adding a
//! command or renaming a field is a one-line change that keeps both engines in
//! lockstep — the same property `commands.hsx` calls `getStateName()` resolves to
//! `GET State` on SSF2 from a single table, never a divergent hand-written arm.

/// A `commands.hsx` root identifier and the SSF2 reflection verbs that navigate
/// the engine's `peptideCur` register to the equivalent object. `names[0]` is the
/// canonical spelling; the rest are accepted aliases (matched case-insensitively).
pub struct Root {
    pub names: &'static [&'static str],
    /// SSF2 verb sequence (tab-encoded, e.g. `"GET\tstageData"`).
    pub ssf2: &'static [&'static str],
}

/// The roots `commands.hsx` binds per eval (`match`, `p0`..`p3`) plus the engine
/// singletons the SSF2 reflection bridge exposes (`gc`/`rm`/`stats`/`root`).
/// On Fraymakers these are hscript globals; on SSF2 each lowers to the verb chain
/// that reaches the same live object (the per-player character node in the live match, …).
pub const ROOTS: &[Root] = &[
    Root { names: &["match"],                  ssf2: &["GC", "GET\tstageData"] },
    Root { names: &["p0", "self"],             ssf2: &["GC", "GET\tstageData", "GET\tCharacters", "IDX\t0"] },
    Root { names: &["p1"],                     ssf2: &["GC", "GET\tstageData", "GET\tCharacters", "IDX\t1"] },
    Root { names: &["p2"],                     ssf2: &["GC", "GET\tstageData", "GET\tCharacters", "IDX\t2"] },
    Root { names: &["p3"],                     ssf2: &["GC", "GET\tstageData", "GET\tCharacters", "IDX\t3"] },
    Root { names: &["gamecontroller", "gc"],   ssf2: &["GC"] },
    Root { names: &["resourcemanager", "rm"],  ssf2: &["RM"] },
    Root { names: &["menucontroller", "mc"],   ssf2: &["MC"] },
    Root { names: &["stats"],                  ssf2: &["STATS"] },
    Root { names: &["root", "main", "this"],   ssf2: &["ROOT"] },
];

/// Fraymakers passthrough wrappers. In FM a character exposes `.body.x` and
/// `.physics.currentVelocityX`; SSF2 reads the field straight off the character,
/// so these wrapper members are skipped and the NEXT member is aliased instead.
pub const PASSTHROUGH: &[&str] = &["body", "physics"];

/// Fraymakers field name → SSF2 field name (the engines spell the same concept
/// differently). Folded after a [`PASSTHROUGH`] wrapper or on a bare member.
pub const MEMBER_ALIASES: &[(&str, &str)] = &[
    ("currentVelocityX", "XSpeed"),
    ("currentVelocityY", "YSpeed"),
    ("x", "X"),
    ("y", "Y"),
];

/// How a `commands.hsx` method call lowers to SSF2 reflection verbs.
pub enum CallLowering {
    /// A fixed verb sequence, ignoring args (e.g. `getCharacters()` → `GET Characters`).
    Ops(&'static [&'static str]),
    /// The verbs, then an `IDX <n>` from the first numeric arg (e.g. `getCharacter(0)`).
    OpsThenIndex(&'static [&'static str]),
}

/// A named `commands.hsx`/Fraymakers method and its SSF2 lowering.
pub struct CallSpec {
    pub names: &'static [&'static str],
    pub lowering: CallLowering,
}

/// The character-access methods `commands.hsx` defines (`getCharacters`,
/// `getCharacter`, `characterCount`) plus the FM character method `getStateName`.
/// `characterCount()` has no SSF2 `LEN` verb but AVM2 arrays expose `.length`, so
/// it lowers to `GET Characters` then `GET length`.
pub const CALLS: &[CallSpec] = &[
    CallSpec { names: &["getCharacters"],            lowering: CallLowering::Ops(&["GET\tCharacters"]) },
    CallSpec { names: &["getCharacter", "getPlayer"], lowering: CallLowering::OpsThenIndex(&["GET\tCharacters"]) },
    CallSpec { names: &["getStateName", "getState"],  lowering: CallLowering::Ops(&["GET\tState"]) },
    CallSpec { names: &["characterCount"],            lowering: CallLowering::Ops(&["GET\tCharacters", "GET\tlength"]) },
];

/// SSF2 verbs for a root identifier (case-insensitive), or `None` to fall back to
/// a generic member read off the document root.
pub fn root_ssf2(name: &str) -> Option<&'static [&'static str]> {
    let l = name.to_ascii_lowercase();
    ROOTS.iter().find(|r| r.names.iter().any(|n| *n == l)).map(|r| r.ssf2)
}

/// Map a Fraymakers field name to its SSF2 spelling (identity if no alias).
pub fn member_alias(name: &str) -> &str {
    MEMBER_ALIASES.iter().find(|(k, _)| *k == name).map(|(_, v)| *v).unwrap_or(name)
}

/// Is this a Fraymakers passthrough wrapper (`body`/`physics`) to skip on SSF2?
pub fn is_passthrough(name: &str) -> bool {
    PASSTHROUGH.contains(&name)
}

/// The SSF2 lowering for a known `commands.hsx`/FM method call, or `None` for a
/// generic reflection call by name.
pub fn call_lowering(name: &str) -> Option<&'static CallLowering> {
    CALLS.iter().find(|c| c.names.contains(&name)).map(|c| &c.lowering)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_cover_commands_hsx_bindings() {
        assert_eq!(root_ssf2("match"), Some(["GC", "GET\tstageData"].as_slice()));
        assert_eq!(root_ssf2("MATCH"), Some(["GC", "GET\tstageData"].as_slice())); // case-insensitive
        assert_eq!(root_ssf2("p0"), root_ssf2("self"));
        assert_eq!(root_ssf2("p1").unwrap().last(), Some(&"IDX\t1"));
        // p0..p3 all resolve (SSF2 parity with the Fraymakers p0-p3 binding); each indexes
        // the matching live-character slot.
        assert_eq!(root_ssf2("p2").unwrap().last(), Some(&"IDX\t2"));
        assert_eq!(root_ssf2("p3").unwrap().last(), Some(&"IDX\t3"));
        assert!(root_ssf2("nonsense").is_none());
    }

    #[test]
    fn member_and_passthrough_match_commands_hsx() {
        assert_eq!(member_alias("currentVelocityX"), "XSpeed");
        assert_eq!(member_alias("x"), "X");
        assert_eq!(member_alias("damage"), "damage"); // identity for unmapped
        assert!(is_passthrough("body"));
        assert!(is_passthrough("physics"));
        assert!(!is_passthrough("damage"));
    }

    #[test]
    fn call_lowering_covers_character_access() {
        assert!(matches!(call_lowering("getCharacters"), Some(CallLowering::Ops(_))));
        assert!(matches!(call_lowering("getCharacter"), Some(CallLowering::OpsThenIndex(_))));
        assert!(matches!(call_lowering("characterCount"), Some(CallLowering::Ops(_))));
        assert!(call_lowering("toState").is_none()); // FM-only → generic call
    }
}
