//! Peptide command interpreter — the host-side front end that takes our
//! human-readable commands and translates them into the wire protocol read by
//! the **bytecode command interpreter we patched into the game** (the dispatch
//! chain injected by `connect_edit` in `main.rs`, which runs inside the engine's
//! `update()` loop).
//!
//! Design split (see TESTING.md "Engine RE map"):
//!   - The ENGINE-SIDE interpreter speaks a terse single-byte wire protocol: one
//!     byte selects a handler in the patched-in bytecode (`s` = spawn/launch,
//!     `x` = exit, `l` = load, `c` = console, `e` = run an hscript string). Only
//!     `s` and `e` read a trailing argument line. That dispatch is hand-written
//!     HashLink bytecode and is deliberately kept minimal.
//!   - HUMANS (and the GUI) type full lines. THIS module is where the two meet:
//!     `translate()` turns what you type into the wire line the patched engine
//!     understands. A recognized command maps to its wire byte ("spawn sandbag"
//!     -> "s sandbag", "exit" -> "x"); ANYTHING ELSE is treated as an hscript
//!     expression and forwarded to the engine's `e` handler ("match.getCharacters()
//!     [0].toState(CState.JAB)" -> "e match.getCharacters()[0].toState(CState.JAB)").
//!     So this file is the vocabulary/routing layer ONLY — it never runs game
//!     logic itself; the engine's bytecode interpreter (and the hscript it then
//!     runs, see commands.hsx) does that on the other side of the socket.
//!
//! Both `peptide` (the patcher, which generates the move-dispatch jump table from
//! `MOVES` for the still-present `m` bytecode handler) and `peptide-bridge` (the
//! client that calls `translate()`) include this file, so the host-side command
//! surface and the patched-in protocol can never drift apart.

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

/// The friendly command set. Deliberately TINY: the real interface is `eval`
/// (`e`) — every non-command line is run as hscript through the engine's own
/// interpreter, which already exposes the full Fraymakers script API (CState,
/// HitboxStats, Assist, MatchModifier, Announcer, …) plus live character access
/// via `match.getCharacters()`. So instead of a per-feature command, you write
/// the hscript directly:
///     match.getCharacters()[0].getStateName()
///     match.getCharacters()[0].toState(CState.SPECIAL_NEUTRAL)
///     match.getCharacters()[0].damage._damage
/// The earlier round of auto-player-0 sugar (state/move/physics/anim/…) was
/// dropped in favour of this explicit, no-hidden-target model.
pub const COMMANDS: &[Cmd] = &[
    Cmd { name: "help",    aliases: &["h", "?"],            wire: '\0',
          args: "",                              help: "list these commands + the hscript model (client-side; sends nothing)" },
    Cmd { name: "spawn",   aliases: &["start", "launch", "s"], wire: 's',
          args: "<char> [stage] [assist]",       help: "start a match with <char> (loads custom content if needed); stage/assist default to thespire/commandervideoassist" },
    Cmd { name: "eval",    aliases: &["e"],                  wire: 'e',
          args: "<hscript>",                     help: "run an hscript expression in the engine and print E:<result>. This is also the default for any unrecognized line." },
    Cmd { name: "load",    aliases: &["l"],                 wire: 'l',
          args: "",                              help: "synchronous custom-.fra load probe (diagnostic; spawn does this itself)" },
    Cmd { name: "console", aliases: &["c"],                 wire: 'c',
          args: "",                              help: "run the engine's debug console `help` command (RAN) — a side-effecting call hscript can't make, so it stays a wire byte" },
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

/// Map a friendly move name to its `CState.<FIELD>` hscript expression
/// (e.g. "jab" -> "CState.JAB", "strong_forward" -> "CState.STRONG_FORWARD_IN").
pub fn move_cstate(name: &str) -> Option<String> {
    let n = name.to_ascii_lowercase();
    MOVES.iter().find(|(m, _)| *m == n).map(|(_, field)| format!("CState.{field}"))
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

    // Only the irreducible commands are recognized here; EVERYTHING ELSE is an hscript
    // expression run through the eval hook (`e <expr>`). So `match.getCharacters()[0].
    // getStateName()`, `CState.JAB`, `1 + 2` etc. just evaluate — no per-feature command.
    if let Some(cmd) = lookup(head) {
        match cmd.name {
            "help" => return Translated::Client(help_text()),

            // Single-byte wire-protocol commands. spawn/exit/load are the irreducible
            // bootstrap hooks. `console` stays here too: it's a side-effecting method call
            // (h2d.Console.runCommand) the interp can't invoke — only field reads and
            // Character-instance methods reflect through hscript — so its bytecode `c`
            // handler remains the path.
            "spawn" | "exit" | "load" | "console" => {
                let mut wire = cmd.wire.to_string();
                if !rest.is_empty() { wire.push(' '); wire.push_str(&rest.join(" ")); }
                return Translated::Wire(wire);
            }

            // `eval <hscript>` — explicit form; the implicit default below does the same.
            "eval" => {
                return Translated::Wire(if rest.is_empty() { "e".into() }
                                        else { format!("e {}", rest.join(" ")) });
            }
            _ => {}
        }
    }

    // Unknown head OR a bare hscript expression (`match.getCharacters()[0].toState(
    // CState.JAB)`, `CState.JAB`, `1 + 2`, …) — run the whole line through the eval hook.
    // collapse_multiline folds a pasted/typed multi-line snippet onto one physical line
    // so it survives the newline-delimited engine wire (the `e` drain reads until '\n').
    Translated::Wire(format!("e {}", collapse_multiline(line)))
}

/// Split one chat message into independent commands at BLANK lines.
///
/// A message is one or more commands. Consecutive lines form a single command; a blank
/// line (whitespace only) ends it and starts the next. Each returned block is handed to
/// `translate` on its own, so `spawn sandbag`, a blank line, then a `match.…` expression
/// run as two separate wire commands with two separate replies.
///
/// The blank-line split is suppressed while inside a string literal, a `/* … */` block
/// comment, or unbalanced `()`/`[]`/`{}` nesting — so a genuine multi-line statement (or a
/// string/block that itself contains a blank line) is never torn apart. A bare single-line
/// (or single multi-line) message returns exactly one block, preserving prior behaviour.
pub fn split_commands(text: &str) -> Vec<String> {
    enum St { Code, Str(char), Block }
    let mut blocks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut st = St::Code;
    let mut escaped = false; // inside Str
    let mut depth: i32 = 0;  // open ()/[]/{} nesting

    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let at_top = matches!(st, St::Code) && depth <= 0;
        if line.trim().is_empty() && at_top {
            // separator: a blank line at the top level ends the current command.
            if !cur.trim().is_empty() { blocks.push(cur.trim().to_string()); }
            cur.clear();
            continue;
        }
        if !cur.is_empty() { cur.push('\n'); }
        cur.push_str(line);
        // advance the string/comment/nesting scanner across this line.
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match st {
                St::Code => match c {
                    '/' if chars.peek() == Some(&'/') => break, // line comment -> rest is inert
                    '/' if chars.peek() == Some(&'*') => { chars.next(); st = St::Block; }
                    '"' | '\'' => { escaped = false; st = St::Str(c); }
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth -= 1,
                    _ => {}
                },
                St::Str(q) => {
                    if escaped { escaped = false; }
                    else if c == '\\' { escaped = true; }
                    else if c == q { st = St::Code; }
                }
                St::Block => {
                    if c == '*' && chars.peek() == Some(&'/') { chars.next(); st = St::Code; }
                }
            }
        }
    }
    if !cur.trim().is_empty() { blocks.push(cur.trim().to_string()); }
    blocks
}

/// Fold a (possibly multi-line) hscript snippet onto a single physical line so it can
/// cross the newline-delimited engine wire — the engine's `e` drain reads bytes until a
/// '\n', so any real newline in the payload would fragment one command into several.
///
/// hscript treats newlines as ordinary whitespace (statements are separated by `;`, never
/// by line breaks), so this is semantics-preserving for the executable code:
///   - newlines OUTSIDE a string literal collapse to a single space;
///   - `//` line comments are dropped to end-of-line (they never execute anyway);
///   - `/* … */` block comments are kept (their internal newlines become spaces);
///   - a real newline INSIDE a string literal becomes the `\n` escape, preserving the
///     string's meaning while keeping the wire frame intact;
///   - string contents pass through verbatim with `\`-escapes respected, so a `\"` does
///     not end the string and a `//` inside a string is not mistaken for a comment.
fn collapse_multiline(src: &str) -> String {
    if !src.contains('\n') && !src.contains('\r') {
        return src.to_string(); // common case: already one line
    }
    enum St { Code, Str(char), Line, Block }
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut st = St::Code;
    let mut escaped = false; // only meaningful inside Str
    while let Some(c) = chars.next() {
        match st {
            St::Code => match c {
                '\r' => {}                                   // swallow; the '\n' is the separator
                '\n' => out.push(' '),
                '/' if chars.peek() == Some(&'/') => { chars.next(); st = St::Line; }
                '/' if chars.peek() == Some(&'*') => { chars.next(); out.push_str("/*"); st = St::Block; }
                '"' | '\'' => { out.push(c); escaped = false; st = St::Str(c); }
                _ => out.push(c),
            },
            St::Str(q) => {
                if escaped { out.push(c); escaped = false; }
                else if c == '\\' { out.push(c); escaped = true; }
                else if c == '\r' {}                          // swallow (\n handles it)
                else if c == '\n' { out.push_str("\\n"); }    // real newline -> escape, keep meaning
                else if c == q { out.push(c); st = St::Code; }
                else { out.push(c); }
            }
            St::Line => { if c == '\n' { out.push(' '); st = St::Code; } } // else: drop comment text
            St::Block => match c {
                '*' if chars.peek() == Some(&'/') => { chars.next(); out.push_str("*/"); st = St::Code; }
                '\r' => {}
                '\n' => out.push(' '),
                _ => out.push(c),
            },
        }
    }
    out
}

/// A friendly gloss for an engine reply line (additive — callers keep the raw
/// line and append this in parens). Returns None when there's nothing to add.
pub fn gloss(reply: &str) -> Option<String> {
    let r = reply.trim();
    // Eval-hook replies are wrapped as "E:<result>" — gloss the inner payload so
    // hscript-ported commands read as friendly as the old wire-byte ones did
    // (e.g. "E:Q:MATCH_LIVE" -> "a match is live", "E:PONG" -> "engine alive").
    let r = r.strip_prefix("E:").unwrap_or(r);
    let body = |s: &str| s.to_string();
    // ANIM:/LAUNCHED come from the engine bootstrap + telemetry; the others gloss the
    // replies of the still-present bytecode handlers (q/p/t/m) if a raw byte is sent.
    if let Some(s) = r.strip_prefix("ANIM:") { return Some(format!("animation: {}", body(s))); }
    if let Some(s) = r.strip_prefix("LAUNCHED ") { return Some(format!("match launched: {}", body(s))); }
    if let Some(s) = r.strip_prefix("T:") { return Some(format!("state: {}", body(s))); }
    if let Some(s) = r.strip_prefix("M:") { return Some(format!("move dispatched: {}", body(s))); }
    match r {
        "Q:MATCH_LIVE" => Some("a match is live".into()),
        "Q:NO_MATCH"   => Some("no match running".into()),
        "PONG"         => Some("engine alive".into()),
        _ => None,
    }
}

/// Build the crash modal's "Enhanced log" from the engine's `error.log` and the `RESDIAG:`
/// breadcrumbs the patched engine emitted (the one fact `error.log` lacks — the failing
/// resource id). HOST-SIDE — no engine bytecode (see AGENT_CONTEXT.md "keep logic OUT of
/// bytecode"). Top = a plain-English translation; bottom = the abridged engine exception.
/// Returns None when there's nothing to interpret.
pub fn interpret_crash(error_log: &str, resdiag: &[String]) -> Option<String> {
    let log = error_log.trim();
    if log.is_empty() && resdiag.is_empty() { return None; }

    // The failing resource id the engine reported: "RESDIAG: … resource id: <id>".
    let id = resdiag.iter().rev()
        .find_map(|l| l.split("resource id:").nth(1))
        .map(|s| s.trim().to_string());
    // Abridged exception + the deepest meaningful app frame (skip std/haxe/hxd plumbing).
    let exc = log.lines().find(|l| l.contains("Exception:"))
        .and_then(|l| l.splitn(2, "Exception:").nth(1)).map(str::trim);
    let frame = log.lines()
        .filter(|l| l.contains("Called from") && (l.contains("pxf.") || l.contains("fraymakers.")))
        .map(|l| l.trim_start_matches("Called from").trim())
        .next();

    let is_stage = log.contains("setupStage") || log.contains("stagePxfContentMap")
        || resdiag.iter().any(|l| l.contains("stage failed"));

    let mut out = String::new();
    if is_stage {
        out.push_str("Hmm… the stage didn't load.\n");
        match &id {
            Some(i) => out.push_str(&format!(
                "{i} isn't a valid/loadable stage resource. Match.setupStage asked the \
                 ResourceManager for it and it returned null.")),
            None => out.push_str(
                "The configured stage isn't a valid/loadable stage resource — Match.setupStage \
                 asked the ResourceManager for it and it returned null."),
        }
    } else if log.contains(".namespace") || log.contains("getContentIdentifierString") {
        out.push_str("Hmm… a resource didn't load.\n");
        out.push_str("Something the match needs — a character, stage, or assist — didn't \
                      resolve to a real resource.");
        if let Some(i) = &id { out.push_str(&format!("\nLast resource reported: {i}.")); }
    } else {
        out.push_str("Hmm… Fraymakers crashed unexpectedly.");
    }

    if exc.is_some() || frame.is_some() {
        out.push_str("\n\n");
        if let Some(e) = exc { out.push_str(&format!("Engine exception: {e}\n")); }
        if let Some(f) = frame { out.push_str(&format!("Crash site: {f}")); }
    }
    Some(out.trim_end().to_string())
}

/// The `help` listing.
pub fn help_text() -> String {
    let mut out = String::from("Peptide commands (friendly name [aliases] <args> — description):\n");
    for c in COMMANDS {
        let al = if c.aliases.is_empty() { String::new() } else { format!(" [{}]", c.aliases.join(", ")) };
        out.push_str(&format!("  {:<8}{:<22} {:<26} {}\n", c.name, al, c.args, c.help));
    }
    out.push_str("\nEverything else is hscript, run in the engine's own interpreter via `e`.\n");
    out.push_str("Live character access (explicit target — no hidden player 0):\n");
    out.push_str("  match.getCharacters()                       all characters in the match\n");
    out.push_str("  match.getCharacters().length                how many\n");
    out.push_str("  match.getCharacters()[0].getStateName()     a character's current state\n");
    out.push_str("  match.getCharacters()[0].toState(CState.SPECIAL_NEUTRAL)   drive a move\n");
    out.push_str("  match.getCharacters()[0].body.x             read a field\n");
    out.push_str("  match.getCharacters()[0].damage._damage     damage %\n");
    out.push_str("\nThe full Fraymakers script API is in scope (same as character/mode scripts):\n");
    out.push_str("  CState  CStateGroup  HitboxStats  StatusEffectType  EntityHitCondition\n");
    out.push_str("  Assist  AssistEvent  MatchModifier  Announcer  GameMenus  Css  GlobalVfx\n");
    out.push_str("  GlobalSfx  CameraShakeType  GraphicsSettings  DisplaySettings  Ai*Option …\n");
    out.push_str("\nExamples:\n");
    out.push_str("  spawn sandbag                 start a sandbag match (default stage/assist)\n");
    out.push_str("  spawn mario thespire commandervideoassist\n");
    out.push_str("  match.getCharacters()[0].getStateName()    read a character's state\n");
    out.push_str("  CState.JAB                    inspect an API enum value\n");
    out.push_str("  exit                          shut the engine down\n");
    out.push_str("\nCLI modes (run as `peptide <mode> …`, not engine commands):\n");
    out.push_str("  (no args) | gui               open the Peptide window (default)\n");
    out.push_str("  tui                           terminal console\n");
    out.push_str("  convert <file.ssf> [opts]     SSF2 → Fraymakers conversion (peptide convert --help)\n");
    out.push_str("  export  --project <.fraytools>   drive FrayTools Publish → build the .fra\n");
    out.push_str("  render  --entity <rel> [...]     render an entity to PNG via FrayTools\n");
    out.push_str("  harness --entity <rel> [...]     extract box geometry + PNG + JSON report\n");
    out.push_str("  headless | send \"<cmd>\"        TCP bridge runtime / one-shot command\n");
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
    fn bootstrap_commands_stay_wire_bytes() {
        // match-launch + clean exit + resource load are the irreducible bootstrap hooks;
        // console is a side-effecting method call hscript can't invoke. All stay wire bytes.
        assert_eq!(wire("spawn sandbag thespire x"), "s sandbag thespire x");
        assert_eq!(wire("exit"), "x");
        assert_eq!(wire("console"), "c");
        assert_eq!(wire("load"), "l");
        assert_eq!(wire("start sandbag"), "s sandbag");
        assert_eq!(wire("quit"), "x");
    }

    #[test]
    fn raw_hscript_passes_through_eval() {
        // the dropped sugar commands + anything unrecognized are treated as hscript and
        // run through the eval hook — this is the explicit, no-hidden-target model.
        assert_eq!(wire("match.getCharacters()[0].getStateName()"),
                   "e match.getCharacters()[0].getStateName()");
        assert_eq!(wire("match.getCharacters()[0].toState(CState.JAB)"),
                   "e match.getCharacters()[0].toState(CState.JAB)");
        assert_eq!(wire("CState.JAB"), "e CState.JAB");
        assert_eq!(wire("1 + 2"), "e 1 + 2");
        assert_eq!(wire("eval CState.JAB"), "e CState.JAB");   // explicit eval == default
    }

    #[test]
    fn multiline_hscript_collapses_to_one_wire_line() {
        // A pasted/typed multi-line snippet must reach the engine as ONE '\n'-free line
        // (the wire is newline-delimited); hscript separates statements with `;`, so
        // folding line breaks to spaces is semantics-preserving.
        assert_eq!(
            wire("var c = match.getCharacters()[0];\nc.toState(CState.JAB);"),
            "e var c = match.getCharacters()[0]; c.toState(CState.JAB);"
        );
        // CRLF (Windows clipboard) folds the same way — no stray '\r'.
        assert_eq!(wire("a;\r\nb;"), "e a; b;");
        // a single line is untouched (and never gets a sentinel).
        assert_eq!(wire("1 + 2"), "e 1 + 2");
    }

    #[test]
    fn multiline_drops_line_comments_and_keeps_strings_intact() {
        // `//` comments must not swallow the following line once newlines collapse.
        assert_eq!(wire("a;  // set a\nb;"), "e a;   b;");
        // `//` and line breaks INSIDE a string literal are preserved (newline -> \n escape).
        assert_eq!(wire("log(\"a//b\");"), "e log(\"a//b\");");
        assert_eq!(wire("log(\"a\nb\");"), "e log(\"a\\nb\");");
        // block comments survive; their internal newline becomes a space.
        assert_eq!(wire("a;/* x\ny */b;"), "e a;/* x y */b;");
    }

    #[test]
    fn split_commands_separates_on_blank_lines() {
        // a blank line breaks one message into independent commands…
        assert_eq!(
            split_commands("spawn sandbag\n\nmatch.getCharacters()[0].getStateName()"),
            vec!["spawn sandbag", "match.getCharacters()[0].getStateName()"]
        );
        // …several blanks / leading / trailing blanks collapse and are ignored.
        assert_eq!(split_commands("\n\na\n\n\nb\n\n"), vec!["a", "b"]);
        // no blank line -> exactly one block (single command, possibly multi-line).
        assert_eq!(
            split_commands("var c = match.getCharacters()[0];\nc.toState(CState.JAB);"),
            vec!["var c = match.getCharacters()[0];\nc.toState(CState.JAB);"]
        );
        // empty / whitespace-only message -> no commands.
        assert!(split_commands("   \n\n").is_empty());
    }

    #[test]
    fn split_commands_keeps_multiline_statements_intact() {
        // a blank line INSIDE open braces is not a separator (one statement).
        assert_eq!(
            split_commands("if (x) {\n\n  y;\n}"),
            vec!["if (x) {\n\n  y;\n}"]
        );
        // a blank line inside a string literal is not a separator.
        assert_eq!(split_commands("log(\"a\n\nb\")"), vec!["log(\"a\n\nb\")"]);
        // and the multi-command case still feeds each block through translate cleanly.
        let wires: Vec<String> = split_commands("1 + 2\n\nspawn sandbag")
            .iter().map(|c| wire(c)).collect();
        assert_eq!(wires, vec!["e 1 + 2", "s sandbag"]);
    }

    #[test]
    fn help_is_client_side() {
        assert!(matches!(translate("help"), Translated::Client(_)));
        assert!(matches!(translate("?"), Translated::Client(_)));
    }

    #[test]
    fn unknown_routes_to_eval() {
        // unrecognized input is treated as hscript and run through the eval hook
        assert_eq!(wire("somethingnew arg"), "e somethingnew arg");
    }

    #[test]
    fn interpret_crash_stage_with_resdiag() {
        let log = "Exception: Null access .stagePxfContentMap\n\
                   Called from pxf.core.Match.setupStage (pxf/core/Match.hx line 1095)\n\
                   Called from hxd.App.mainLoop (hxd/App.hx line 193)";
        let resdiag = vec![
            "RESDIAG: stage failed to load — Match.setupStage got null from getPXFResource for resource id: global::teststage".to_string()
        ];
        let out = interpret_crash(log, &resdiag).unwrap();
        // top: plain-English translation with the failing id woven in
        assert!(out.starts_with("Hmm… the stage didn't load."), "got: {out}");
        assert!(out.contains("global::teststage isn't a valid/loadable stage resource"), "got: {out}");
        // bottom: abridged exception + crash site (NOT the full stack)
        assert!(out.contains("Engine exception: Null access .stagePxfContentMap"), "got: {out}");
        assert!(out.contains("Crash site: pxf.core.Match.setupStage (pxf/core/Match.hx line 1095)"), "got: {out}");
        assert!(!out.contains("mainLoop"), "abridged log should drop hxd plumbing: {out}");
    }

    #[test]
    fn interpret_crash_empty_is_none() {
        assert!(interpret_crash("", &[]).is_none());
    }
}
