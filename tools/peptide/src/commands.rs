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
    Cmd { name: "loop",    aliases: &["repeat"],            wire: '\0',
          args: "<move> [count]",                help: "re-dispatch a move on an interval (client-side; default 8x) — for sustained observation / live tuning" },
    Cmd { name: "state",   aliases: &["status", "t"],       wire: 't',
          args: "",                              help: "report player 0's current state name (T:<state>)" },
    Cmd { name: "query",   aliases: &["matchlive", "q"],    wire: 'q',
          args: "",                              help: "is a match live? (Q:MATCH_LIVE / Q:NO_MATCH)" },
    Cmd { name: "physics", aliases: &["phys", "vitals", "v"], wire: 'v',
          args: "",                              help: "player 0 position/velocity/damage (P: x=.. y=.. vx=.. vy=.. dmg=..)" },
    Cmd { name: "anim",    aliases: &["animation", "a"],     wire: 'a',
          args: "",                              help: "player 0 current animation + frame (A:<name> frame <cur>/<total>)" },
    Cmd { name: "step",    aliases: &["framestep", "f"],     wire: 'f',
          args: "",                              help: "pause playback + advance player 0's animation ONE frame (scrub); reports A:<name> frame cur/total" },
    Cmd { name: "play",    aliases: &["resume", "g"],        wire: 'g',
          args: "",                              help: "resume player 0 animation playback after step/pause (PLAY)" },
    Cmd { name: "snapshot", aliases: &["snap"],              wire: '\0',
          args: "",                              help: "one-shot readback bundle: state + physics + animation (client-side; sends t, v, a)" },
    Cmd { name: "track",   aliases: &[],                     wire: '\0',
          args: "<move> [samples]",              help: "drive a move then rapid-sample physics (default 6) — captures the move's velocity/position trajectory (self-momentum)" },
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
    // Strongs (smashes) are 3-phase in CState (_IN -> _CHARGE -> _ATTACK); _IN is the
    // entry point that drives the whole move, so the friendly name maps there.
    ("strong_forward",  "STRONG_FORWARD_IN"),
    ("strong_up",       "STRONG_UP_IN"),
    ("strong_down",     "STRONG_DOWN_IN"),
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
    /// Send `wire` to the engine `count` times, sleeping `gap_ms` between sends
    /// (client-orchestrated repetition — e.g. `loop`). Zero engine bytecode.
    Repeat { wire: String, count: u32, gap_ms: u64 },
    /// Send these wire lines in order (client-orchestrated multi-command — e.g.
    /// `snapshot` = t, v, a). Zero engine bytecode; groundwork for recipe scripting.
    Sequence(Vec<String>),
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

    if cmd.name == "snapshot" {
        // Bundle the three readbacks (state, physics, animation) into one command.
        return Translated::Sequence(vec!["t".into(), "v".into(), "a".into()]);
    }

    if cmd.name == "track" {
        // Drive a move, then rapid-sample physics to capture its velocity/position
        // trajectory (self-momentum). Client-side: m <sel> followed by N `v` reads.
        let Some(mv) = rest.first() else {
            return Translated::Error("usage: track <move> [samples]".to_string());
        };
        let Some(ord) = move_ordinal(mv) else {
            return Translated::Error(format!("unknown move {mv:?}. moves: {}",
                MOVES.iter().map(|(m, _)| *m).collect::<Vec<_>>().join(", ")));
        };
        let n = rest.get(1).and_then(|c| c.parse::<u32>().ok()).unwrap_or(6).clamp(1, 60);
        let mut seq = vec![format!("m {}", (b'A' + ord as u8) as char)];
        for _ in 0..n { seq.push("v".to_string()); }
        return Translated::Sequence(seq);
    }

    if cmd.name == "loop" {
        // `loop <move> [count]` — re-dispatch a move on an interval (client-side).
        let Some(mv) = rest.first() else {
            return Translated::Error("usage: loop <move> [count]".to_string());
        };
        let Some(ord) = move_ordinal(mv) else {
            return Translated::Error(format!("unknown move {mv:?}. moves: {}",
                MOVES.iter().map(|(m, _)| *m).collect::<Vec<_>>().join(", ")));
        };
        // count clamped to a sane ceiling so a typo can't spin forever.
        let count = rest.get(1).and_then(|c| c.parse::<u32>().ok()).unwrap_or(8).clamp(1, 200);
        return Translated::Repeat {
            wire: format!("m {}", (b'A' + ord as u8) as char),
            count,
            gap_ms: 800,
        };
    }

    if cmd.wire == '\0' {
        // `help`
        return Translated::Client(help_text());
    }

    if cmd.name == "move" {
        return match rest.first() {
            None => Translated::Wire("m".to_string()), // bare move = jab
            Some(mv) => match move_ordinal(mv) {
                // Wire selector = 'A' + ordinal (a single printable byte the engine
                // compares directly — no integer parsing engine-side). See main.rs
                // move-dispatch. ordinal stays < 26 (MOVES is short), so 'A'..'Z'.
                Some(ord) => Translated::Wire(format!("m {}", (b'A' + ord as u8) as char)),
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
    fn move_resolves_to_letter_selector() {
        assert_eq!(wire("move"), "m");                  // bare = jab
        assert_eq!(wire("move jab"), "m A");            // ordinal 0 -> 'A'
        let ord = move_ordinal("special_neutral").unwrap();
        assert_eq!(wire("move special_neutral"), format!("m {}", (b'A' + ord as u8) as char));
        assert!(matches!(translate("move flibble"), Translated::Error(_)));
        assert!(MOVES.len() <= 26, "selector encoding assumes < 26 moves");
    }

    #[test]
    fn snapshot_expands_to_sequence() {
        match translate("snapshot") {
            Translated::Sequence(w) => assert_eq!(w, vec!["t", "v", "a"]),
            _ => panic!("snapshot should be a Sequence"),
        }
        assert!(matches!(translate("snap"), Translated::Sequence(_)));
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
