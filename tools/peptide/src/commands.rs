//! Peptide command vocabulary — the single source of truth for the
//! **human-facing** command surface.
//!
//! Design split (see TESTING.md "Engine RE map"):
//!   - The ENGINE speaks a terse single-byte wire protocol (one byte selects the
//!     handler; only `s` reads a trailing argument line). That protocol is
//!     hand-written HashLink bytecode and is deliberately kept minimal.
//!   - HUMANS (and a future GUI) speak full words. This module is where the two
//!     meet: `translate()` turns a friendly line ("spawn sandbag", "move
//!     special_neutral", "state") into the wire line the engine understands
//!     ("s sandbag", "m 13", "t"). Renaming/adding a friendly name is a
//!     data-only edit to the tables below — no protocol or bytecode change.
//!
//! Both `peptide` (the patcher, which generates the move-dispatch jump table
//! from `MOVES`) and `peptide-bridge` (the client, which translates the user's
//! words) include this file, so the vocabulary can never drift between them.

/// One human-facing command: the friendly name, its aliases, the single wire
/// byte the engine dispatches on, a one-line argument summary, and a help blurb.
pub struct Cmd {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    /// The wire byte the engine matches on. `'\0'` means handled entirely
    /// client-side (e.g. `help`) and nothing is sent.
    pub wire: char,
    pub args: &'static str,
    pub help: &'static str,
}

/// The full friendly command set. Order is the order `help` prints them.
pub const COMMANDS: &[Cmd] = &[
    Cmd { name: "help",    aliases: &["h", "?"],            wire: '\0',
          args: "",                              help: "list these commands (client-side; sends nothing)" },
    Cmd { name: "spawn",   aliases: &["start", "launch", "s"], wire: 's',
          args: "<char> [stage] [assist]",       help: "start a match with <char> (loads custom content if needed); stage/assist default to thespire/commandervideoassist" },
    Cmd { name: "move",    aliases: &["attack", "m"],       wire: 'm',
          args: "[move-name]",                   help: "drive a move on player 0 via the engine state machine (no arg = jab). See `help` move list below" },
    Cmd { name: "state",   aliases: &["status", "t"],       wire: 't',
          args: "",                              help: "report player 0's current state name (T:<state>)" },
    Cmd { name: "query",   aliases: &["matchlive", "q"],    wire: 'q',
          args: "",                              help: "is a match live? (Q:MATCH_LIVE / Q:NO_MATCH)" },
    Cmd { name: "load",    aliases: &["l"],                 wire: 'l',
          args: "",                              help: "synchronous custom-.fra load probe (diagnostic; spawn does this itself)" },
    Cmd { name: "keys",    aliases: &["pool", "k"],         wire: 'k',
          args: "",                              help: "dump the resource-pool keys + UGC-discovery diagnostics (K:<fqid> …)" },
    Cmd { name: "console", aliases: &["c"],                 wire: 'c',
          args: "",                              help: "run the engine's debug console `help` command (RAN)" },
    Cmd { name: "ping",    aliases: &["p"],                 wire: 'p',
          args: "",                              help: "liveness check (PONG)" },
    Cmd { name: "exit",    aliases: &["quit", "stop", "x"], wire: 'x',
          args: "",                              help: "cleanly shut the engine down (hxd.System.exit — no kill-9 orphan)" },
];

/// Move name → CState field NAME, in the order the engine's generated jump table
/// expects. The client sends the table INDEX (the ordinal) as the `m` argument;
/// the engine resolves each CState field by NAME at patch time (robust to findex
/// drift) and emits one comparison arm per entry, in this exact order. Keep the
/// two in lockstep by sharing this one table.
///
/// Friendly names mirror the Fraymakers animation vocabulary so a modder can
/// guess them (`tilt_down`, `aerial_forward`, `special_neutral`, …).
pub const MOVES: &[(&str, &str)] = &[
    ("jab",             "JAB"),
    ("dash_attack",     "DASH_ATTACK"),
    ("tilt_forward",    "TILT_FORWARD"),
    ("tilt_up",         "TILT_UP"),
    ("tilt_down",       "TILT_DOWN"),
    ("strong_forward",  "STRONG_FORWARD"),
    ("strong_up",       "STRONG_UP"),
    ("strong_down",     "STRONG_DOWN"),
    ("aerial_neutral",  "AERIAL_NEUTRAL"),
    ("aerial_forward",  "AERIAL_FORWARD"),
    ("aerial_back",     "AERIAL_BACK"),
    ("aerial_up",       "AERIAL_UP"),
    ("aerial_down",     "AERIAL_DOWN"),
    ("special_neutral", "SPECIAL_NEUTRAL"),
    ("special_side",    "SPECIAL_SIDE"),
    ("special_up",      "SPECIAL_UP"),
    ("special_down",    "SPECIAL_DOWN"),
    ("grab",            "GRAB"),
    ("stand",           "STAND"),
    ("fall",            "FALL"),
];

/// Find the command whose name or alias matches `tok` (case-insensitive).
pub fn lookup(tok: &str) -> Option<&'static Cmd> {
    let t = tok.to_ascii_lowercase();
    COMMANDS.iter().find(|c| c.name == t || c.aliases.iter().any(|a| *a == t))
}

/// Ordinal (table index) of a move by friendly name, case-insensitive.
pub fn move_ordinal(name: &str) -> Option<usize> {
    let n = name.to_ascii_lowercase();
    MOVES.iter().position(|(m, _)| *m == n)
}

/// Outcome of translating one friendly line.
pub enum Translated {
    /// Send this exact line to the engine.
    Wire(String),
    /// Handled client-side; print this text, send nothing.
    Client(String),
    /// Could not translate; print this error, send nothing.
    Error(String),
}

/// Translate a friendly command line into the engine wire line.
///
/// - `help` / `?` → prints the help text, sends nothing.
/// - a known friendly name (or its alias, or the bare wire letter) → rewritten
///   to `<wire-byte> <args…>`; `move <name>` resolves the name to its ordinal.
/// - anything else → passed through verbatim (forward-compatible: a raw wire
///   line a human typed still reaches the engine untouched).
pub fn translate(line: &str) -> Translated {
    let line = line.trim();
    if line.is_empty() {
        return Translated::Wire(String::new());
    }
    let mut parts = line.split_whitespace();
    let head = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();

    let Some(cmd) = lookup(head) else {
        // Unknown leading token: pass through unchanged so raw/forward-compatible
        // input still works, but tell the user `help` exists.
        return Translated::Wire(line.to_string());
    };

    if cmd.wire == '\0' {
        // `help`
        return Translated::Client(help_text());
    }

    if cmd.name == "move" {
        return match rest.first() {
            None => Translated::Wire("m".to_string()), // bare move = jab
            Some(mv) => match move_ordinal(mv) {
                Some(ord) => Translated::Wire(format!("m {ord}")),
                None => Translated::Error(format!(
                    "unknown move {mv:?}. moves: {}",
                    MOVES.iter().map(|(m, _)| *m).collect::<Vec<_>>().join(", ")
                )),
            },
        };
    }

    // Generic: replace the head with its wire byte, keep the rest of the args.
    let mut wire = cmd.wire.to_string();
    if !rest.is_empty() {
        wire.push(' ');
        wire.push_str(&rest.join(" "));
    }
    Translated::Wire(wire)
}

/// A friendly gloss for an engine reply line (additive — callers keep the raw
/// line and append this in parens). Returns None when there's nothing to add.
pub fn gloss(reply: &str) -> Option<String> {
    let r = reply.trim();
    let body = |s: &str| s.to_string();
    if let Some(s) = r.strip_prefix("T:") { return Some(format!("state: {}", body(s))); }
    if let Some(s) = r.strip_prefix("M:") { return Some(format!("move dispatched: {}", body(s))); }
    if let Some(s) = r.strip_prefix("ANIM:") { return Some(format!("animation: {}", body(s))); }
    if let Some(s) = r.strip_prefix("LAUNCHED ") { return Some(format!("match launched: {}", body(s))); }
    match r {
        "Q:MATCH_LIVE" => Some("a match is live".into()),
        "Q:NO_MATCH"   => Some("no match running".into()),
        "PONG"         => Some("engine alive".into()),
        _ => None,
    }
}

/// The `help` listing.
pub fn help_text() -> String {
    let mut out = String::from("Peptide commands (friendly name [aliases] <args> — description):\n");
    for c in COMMANDS {
        let al = if c.aliases.is_empty() { String::new() } else { format!(" [{}]", c.aliases.join(", ")) };
        out.push_str(&format!("  {:<8}{:<22} {:<26} {}\n", c.name, al, c.args, c.help));
    }
    out.push_str("\nMove names (for `move <name>`):\n  ");
    out.push_str(&MOVES.iter().map(|(m, _)| *m).collect::<Vec<_>>().join(", "));
    out.push('\n');
    out.push_str("\nExamples:\n");
    out.push_str("  spawn sandbag                 start a sandbag match (default stage/assist)\n");
    out.push_str("  spawn mario thespire commandervideoassist\n");
    out.push_str("  state                         read player 0 state\n");
    out.push_str("  move special_neutral          drive neutral-special\n");
    out.push_str("  exit                          shut the engine down\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(line: &str) -> String {
        match translate(line) {
            Translated::Wire(w) => w,
            other => panic!("expected Wire, got {}", match other {
                Translated::Client(_) => "Client", Translated::Error(e) => return e, _ => "?",
            }),
        }
    }

    #[test]
    fn friendly_names_map_to_wire_bytes() {
        assert_eq!(wire("spawn sandbag thespire x"), "s sandbag thespire x");
        assert_eq!(wire("state"), "t");
        assert_eq!(wire("query"), "q");
        assert_eq!(wire("ping"), "p");
        assert_eq!(wire("exit"), "x");
        assert_eq!(wire("keys"), "k");
    }

    #[test]
    fn aliases_and_bare_letters_pass_through() {
        assert_eq!(wire("start sandbag"), "s sandbag");
        assert_eq!(wire("s sandbag"), "s sandbag"); // bare wire letter unchanged
        assert_eq!(wire("status"), "t");
        assert_eq!(wire("quit"), "x");
    }

    #[test]
    fn move_resolves_to_ordinal() {
        assert_eq!(wire("move"), "m");                  // bare = jab
        assert_eq!(wire("move jab"), "m 0");
        assert_eq!(wire("move special_neutral"), format!("m {}", move_ordinal("special_neutral").unwrap()));
        assert!(matches!(translate("move flibble"), Translated::Error(_)));
    }

    #[test]
    fn help_is_client_side() {
        assert!(matches!(translate("help"), Translated::Client(_)));
        assert!(matches!(translate("?"), Translated::Client(_)));
    }

    #[test]
    fn unknown_passes_through() {
        assert_eq!(wire("somethingnew arg"), "somethingnew arg");
    }
}
