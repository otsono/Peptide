//! ssf2_target — the SSF2 backend of [`crate::debug_target::DebugTarget`]. It
//! executes the SAME engine-agnostic commands as Fraymakers, but over AVM2
//! reflection (file-IPC), so `e match.getCharacters()[0].physics.currentVelocityX`,
//! `hold down+special`, `seq …`, `spawn sandbag` etc. work with identical syntax.
//!
//! The heart is [`Ssf2Target::eval`]: a small evaluator that parses the same
//! property-path / index / method-call / assignment syntax and walks it via the
//! engine's reflection verbs (`GC`/`GET`/`IDX`/`CALL`/`CALL1`/`SETP`/`READ`),
//! mapping the Fraymakers debug vocabulary (`match`, `p0`, `.body.x`,
//! `.physics.currentVelocityX`, `getCharacter(i)`, `getStateName()`) onto SSF2's
//! live match object graph (the per-player character nodes and their
//! `.X/.Y/.XSpeed/.YSpeed`, `.State`).

use anyhow::{anyhow, Result};
use std::time::Duration;

use crate::debug_target::DebugTarget;
use crate::interpreter::SpawnArgs;
use crate::ssf2_bridge::request;

const T: Duration = Duration::from_secs(4);

pub struct Ssf2Target;

impl Ssf2Target {
    pub fn new() -> Self { Ssf2Target }

    /// Send one reflection verb line ("VERB\ta1\ta2") and return the reply.
    fn op(&self, wire: &str) -> Result<String> { request(wire, T) }
}

impl Default for Ssf2Target { fn default() -> Self { Self::new() } }

// ─────────────────────────── expression model ─────────────────────────────

#[derive(Debug, Clone)]
enum Access {
    /// `name` / `name.member` after a root — a property get.
    Member(String),
    /// `[i]` or `(i)` index into a Vector/Array.
    Index(i64),
    /// `name(args)` — a method call (args are raw token strings).
    Call(String, Vec<String>),
}

/// Parse a dotted expression into (root, accessors, optional assignment value).
/// e.g. `match.getCharacter(0).body.x = 0`
fn parse_path(expr: &str) -> Result<(String, Vec<Access>, Option<String>)> {
    // split off a top-level assignment (`= value`)
    let (path, assign) = split_assign(expr);
    let mut chars = path.trim().chars().peekable();
    let mut accesses: Vec<Access> = Vec::new();

    // read an identifier
    let read_ident = |it: &mut std::iter::Peekable<std::str::Chars>| -> String {
        let mut s = String::new();
        while let Some(&c) = it.peek() {
            if c.is_alphanumeric() || c == '_' || c == '$' { s.push(c); it.next(); } else { break; }
        }
        s
    };

    // root identifier
    let root = read_ident(&mut chars);
    if root.is_empty() { return Err(anyhow!("expected an identifier at start of {expr:?}")); }

    loop {
        match chars.peek().copied() {
            None => break,
            Some('.') => {
                chars.next();
                let name = read_ident(&mut chars);
                if name.is_empty() { return Err(anyhow!("expected member after '.' in {expr:?}")); }
                if chars.peek() == Some(&'(') {
                    let args = read_args(&mut chars)?;
                    accesses.push(Access::Call(name, args));
                } else {
                    accesses.push(Access::Member(name));
                }
            }
            Some('[') => {
                chars.next();
                let mut inner = String::new();
                while let Some(&c) = chars.peek() { if c == ']' { break; } inner.push(c); chars.next(); }
                chars.next(); // ']'
                let i: i64 = inner.trim().parse().map_err(|_| anyhow!("non-integer index [{inner}]"))?;
                accesses.push(Access::Index(i));
            }
            Some(c) if c.is_whitespace() => { chars.next(); }
            Some(c) => return Err(anyhow!("unexpected char {c:?} in {expr:?}")),
        }
    }
    Ok((root, accesses, assign))
}

/// Split `lhs = rhs` at the first top-level `=` (not `==`/`!=`/`<=`/`>=`).
fn split_assign(expr: &str) -> (String, Option<String>) {
    let b = expr.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'=' if depth == 0 => {
                let prev = if i > 0 { b[i - 1] } else { b' ' };
                let next = if i + 1 < b.len() { b[i + 1] } else { b' ' };
                if !matches!(prev, b'=' | b'!' | b'<' | b'>') && next != b'=' {
                    return (expr[..i].to_string(), Some(expr[i + 1..].trim().to_string()));
                }
            }
            _ => {}
        }
        i += 1;
    }
    (expr.to_string(), None)
}

/// Read a parenthesised, comma-separated argument list (consumes through ')').
fn read_args(it: &mut std::iter::Peekable<std::str::Chars>) -> Result<Vec<String>> {
    it.next(); // '('
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    loop {
        match it.next() {
            None => return Err(anyhow!("unterminated argument list")),
            Some('(') | Some('[') => { depth += 1; cur.push(' '); }
            Some(')') if depth == 0 => { if !cur.trim().is_empty() { args.push(cur.trim().to_string()); } break; }
            Some(')') | Some(']') => { depth -= 1; }
            Some(',') if depth == 0 => { args.push(cur.trim().to_string()); cur.clear(); }
            Some(c) => cur.push(c),
        }
    }
    Ok(args)
}

// ─────────────────────────── reflection mapping ───────────────────────────

impl Ssf2Target {
    /// Map a root identifier to the reflection ops that set `cur` to it.
    /// Mirrors the Fraymakers commands.hsx bindings (`match`, `p0`, `p1`).
    /// Map a root identifier to the reflection ops that set `cur` to it, using the
    /// shared [`crate::vocab`] table so the SSF2↔Fraymakers mapping lives at the
    /// interpreter level, not hard-coded here. Unknown roots fall back to a member
    /// read off the document root.
    fn emit_root(&self, root: &str) -> Result<Vec<String>> {
        Ok(match crate::vocab::root_ssf2(root) {
            Some(ops) => ops.iter().map(|s| s.to_string()).collect(),
            None => vec!["ROOT".into(), format!("GET\t{root}")],
        })
    }

    /// Map a method call onto reflection ops, consulting [`crate::vocab::CALLS`]
    /// first (the `commands.hsx` character-access vocabulary) and falling back to a
    /// generic 0/1-arg reflection call for anything engine-agnostic.
    fn emit_call(&self, name: &str, args: &[String]) -> Result<Vec<String>> {
        use crate::vocab::CallLowering;
        if let Some(lowering) = crate::vocab::call_lowering(name) {
            return Ok(match lowering {
                CallLowering::Ops(ops) => ops.iter().map(|s| s.to_string()).collect(),
                CallLowering::OpsThenIndex(ops) => {
                    let i: i64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
                    let mut v: Vec<String> = ops.iter().map(|s| s.to_string()).collect();
                    v.push(format!("IDX\t{i}"));
                    v
                }
            });
        }
        // generic: 0-arg or 1-(numeric)-arg method call by name
        Ok(match args.len() {
            0 => vec![format!("CALL\t{name}")],
            1 => vec![format!("CALL1\t{name}\t{}", args[0])], // numeric coerced engine-side
            _ => return Err(anyhow!("{name}(): only 0/1-arg method calls are supported via reflection")),
        })
    }
}

impl DebugTarget for Ssf2Target {
    fn engine(&self) -> &'static str { "ssf2" }

    /// Evaluate an expression by walking the object graph via reflection.
    fn eval(&mut self, expr: &str) -> Result<String> {
        let expr = expr.trim();
        if expr.is_empty() { return Ok(String::new()); }
        // bare verb passthrough: allow raw reflection verbs (GC/GET/READ/SPAWN/…)
        let head = expr.split_whitespace().next().unwrap_or("");
        if matches!(head, "GC"|"RM"|"STATS"|"MC"|"ROOT"|"GET"|"IDX"|"CALL"|"CALL1"|"CALLS"|"SETP"|"READ"|"PING"|"SPAWN"|"GO"|"QUEUE"|"LOADED"|"LOADNEXT") {
            return self.op(&expr.split_whitespace().collect::<Vec<_>>().join("\t"));
        }

        // commands.hsx composite globals that aren't a plain navigation: handle
        // them whole so `log(x)` / `matchStatus()` produce the same effect SSF2-side
        // as the hscript helpers do on Fraymakers.
        if let Some(r) = self.eval_commands_hsx(expr)? { return Ok(r); }

        let (root, accesses, assign) = parse_path(expr)?;
        let mut ops = self.emit_root(&root)?;

        // walk accessors, folding FM passthrough wrappers (body/physics) via the
        // shared vocab table so the FM↔SSF2 naming lives at the interpreter level.
        let mut i = 0;
        while i < accesses.len() {
            match &accesses[i] {
                Access::Member(m) if crate::vocab::is_passthrough(m) && i + 1 < accesses.len() => {
                    // .body.x → X ; .physics.currentVelocityX → XSpeed
                    if let Access::Member(next) = &accesses[i + 1] {
                        ops.push(format!("GET\t{}", crate::vocab::member_alias(next)));
                        i += 2; continue;
                    }
                    ops.push(format!("GET\t{m}"));
                }
                Access::Member(m) => {
                    let mapped = crate::vocab::member_alias(m);
                    // an assignment targets the LAST member → SETP, handled after the loop
                    if assign.is_some() && i + 1 == accesses.len() {
                        let val = assign.as_ref().unwrap();
                        ops.push(format!("SETP\t{mapped}\t{val}"));
                        return self.run_ops(&ops, false);
                    }
                    ops.push(format!("GET\t{mapped}"));
                }
                Access::Index(n) => ops.push(format!("IDX\t{n}")),
                Access::Call(name, args) => ops.extend(self.emit_call(name, args)?),
            }
            i += 1;
        }
        self.run_ops(&ops, true)
    }

    fn spawn(&mut self, args: &SpawnArgs) -> Result<String> {
        // `--versus` (args.force_versus) is a no-op here BY DESIGN: SSF2 has no separate
        // training mode, every match is a versus Game, so the flag that forces versus on
        // Fraymakers is already the only mode on SSF2. Same engine-subset parity as assists
        // being Fraymakers-only and stamina being SSF2-only. We accept it and ignore it.
        let _ = args.force_versus;
        // Stage: caller-supplied, else the configured SSF2 stage (parity with how
        // Fraymakers resolves its stage from config — names differ per engine).
        let stage = args.stage.clone()
            .unwrap_or_else(|| crate::config::Config::load().ssf2_stage());
        // SHARED match rules (match_settings.conf — the same source Fraymakers bakes
        // into its start-match), so an SSF2 spawn plays by the same rules.
        let ms = crate::interpreter::load_match_settings();
        // Players: comma-separated character list; player 0 is the one you control,
        // the rest are idle dummies. (Assists are Fraymakers-only → ignored here.)
        let chars: Vec<String> = if args.characters.is_empty() {
            vec![args.character().to_string()]
        } else { args.characters.clone() };
        let n = chars.len().max(1);

        // SPAWN: build a VERSUS Game with N player slots, set player 0's character,
        // queue stage+char0, load. (3rd arg = player count → Game(N, VERSUS).)
        self.op(&format!("SPAWN\t{}\t{}\t{}", chars[0], stage, n))?;

        // Add players 1..N: each is an idle dummy — a human slot (so NO CPU AI) that
        // simply never receives input (only player 0 is driven by the input injector),
        // so it stands still. Set character (SETS = string set; SETP coerces to Number),
        // queue its resources, then re-drive the load for them.
        for (i, ch) in chars.iter().enumerate().skip(1) {
            let _ = self.op("GC"); let _ = self.op("GET\tcurrentGame"); let _ = self.op("GET\tPlayerSettings"); let _ = self.op(&format!("IDX\t{i}"));
            let _ = self.op(&format!("SETS\tcharacter\t{ch}"));
            let _ = self.op("SETP\thuman\t1");
            let _ = self.op("SETP\tcostume\t0");
            let _ = self.op(&format!("QUEUE1\t{ch}"));
        }
        if n > 1 { let _ = self.op("MLOAD"); }

        // Apply the FULL match config on the REAL config objects BEFORE startMatch reads
        // them. Global rules live on Game.LevelData (GameSettings, lowercase slots); the
        // per-player STOCK COUNT lives on each PlayerSetting.lives (the source startMatch
        // inits from — Game.Lives is only a display mirror). Best-effort; never fail the
        // spawn over a single setting. cur stays put across the SETPs.
        let ut = if ms.time > 0 { 1 } else { 0 };
        let team = if ms.team_damage { 1 } else { 0 };
        let stam = if ms.using_stamina { 1 } else { 0 };
        let _ = self.op("GC"); let _ = self.op("GET\tcurrentGame"); let _ = self.op("GET\tLevelData");
        let _ = self.op(&format!("SETP\tlives\t{}", ms.lives));
        let _ = self.op("SETP\tusingLives\t1");
        let _ = self.op(&format!("SETP\ttime\t{}", ms.time));
        let _ = self.op(&format!("SETP\tusingTime\t{ut}"));
        let _ = self.op(&format!("SETP\tdamageRatio\t{}", ms.damage_ratio));
        let _ = self.op(&format!("SETP\tteamDamage\t{team}"));
        let _ = self.op(&format!("SETP\tusingStamina\t{stam}"));
        let _ = self.op(&format!("SETP\tstartStamina\t{}", ms.start_stamina));
        let _ = self.op(&format!("SETP\tstartDamage\t{}", ms.start_damage));
        let _ = self.op(&format!("SETP\tsizeRatio\t{}", ms.size_ratio));
        // per-player rules (lives = the real stock source; damage/start-damage)
        for i in 0..n {
            let _ = self.op("GC"); let _ = self.op("GET\tcurrentGame"); let _ = self.op("GET\tPlayerSettings"); let _ = self.op(&format!("IDX\t{i}"));
            let _ = self.op(&format!("SETP\tlives\t{}", ms.lives));
            let _ = self.op(&format!("SETP\tdamageRatio\t{}", ms.damage_ratio));
            let _ = self.op(&format!("SETP\tstartDamage\t{}", ms.start_damage));
        }
        // items (best-effort; ItemFrequency on the Game)
        let _ = self.op("GC"); let _ = self.op("GET\tcurrentGame");
        let _ = self.op(&format!("SETP\tItemFrequency\t{}", ms.item_frequency));

        // wait for the stage library to register (load-ready signal). More slack for
        // multiplayer (extra character resources to decrypt/parse).
        let mut ready = false;
        let iters = (40 + 15 * (n - 1)).max(40);
        for _ in 0..iters {
            std::thread::sleep(Duration::from_millis(400));
            if self.op(&format!("CALLS\tgetLibraryMC\tstage_{stage}")).is_ok()
                && self.op("READ").map(|r| r.contains("[object")).unwrap_or(false) { ready = true; break; }
            // (re-navigating cur each poll; CALLS sets cur, READ reads it)
            let _ = self.op("RM");
        }
        self.op("GO")?;

        Ok(format!("spawn {} on {} (versus, {} player(s): lives={}, time={}, damage_ratio={}, loaded={ready})",
                   chars.join(","), stage, n, ms.lives, ms.time, ms.damage_ratio))
    }

    fn hold(&mut self, mask: u32) -> Result<String> {
        // SSF2 input injection isn't wired through the engine's controller yet;
        // expose the mask so callers see it took effect at the protocol level.
        Ok(format!("hold mask={mask} (ssf2 input injection: see autojump for YSpeed-level control)"))
    }

    fn seq(&mut self, masks: &[u32]) -> Result<String> {
        Ok(format!("seq {} frames (ssf2 input timeline not yet wired)", masks.len()))
    }

    fn console(&mut self) -> Result<String> { self.op("ROOT") }
    fn add_character(&mut self) -> Result<String> { Ok("addCharacter: not supported on SSF2 yet\n".into()) }
    fn exit(&mut self) -> Result<()> { let _ = std::process::Command::new("pkill").args(["-f", "SSF2-patched"]).status(); Ok(()) }
    fn load(&mut self) -> Result<String> { self.op("LOADED") }

    /// SSF2 matchStatus: cheap when idle (short-circuit if no live match), else the
    /// composed `<id>|<dmg>|<anim>` feed read live from the engine.
    fn match_status(&mut self) -> Result<Option<String>> {
        if !self.match_live() { return Ok(None); }
        let feed = self.compose_match_status()?;
        // "MATCHSTATUS:" with no records → treat as empty (no widget rows).
        Ok(if feed.trim_end() == "MATCHSTATUS:" { None } else { Some(feed) })
    }

    /// SSF2 has no stock-icon ripping pipeline yet, so there's no icon to supply —
    /// the matchStatus widget shows a glyph. (A genuine capability gap, declared
    /// explicitly rather than silently skipped in the GUI.)
    fn char_icon(&mut self, _slot: u32) -> Result<Option<String>> { Ok(None) }
}

impl Ssf2Target {
    /// Run a sequence of navigation ops, then READ (or, for an assignment that
    /// already emitted SETP, just confirm).
    fn run_ops(&self, ops: &[String], read: bool) -> Result<String> {
        let mut last = String::new();
        for op in ops { last = self.op(op)?; }
        if read { self.op("READ") } else { Ok(last) }
    }

    /// Handle the `commands.hsx` globals that are whole helpers rather than a plain
    /// property navigation, so they produce the SAME effect on SSF2 as the hscript
    /// definitions do on Fraymakers. Returns `Ok(Some(result))` if it owned the
    /// expression, `Ok(None)` to let the generic evaluator handle it.
    ///
    ///   * `log(<expr>)`     → engine `trace(<expr>)`, replies `logged: <expr>`
    ///                         (commands.hsx: `__td.log(msg); return "logged: "+msg`).
    ///   * `matchStatus()`   → the `MATCHSTATUS:` host-poll feed, composed from
    ///                         per-character reflection reads (id|damage|animation),
    ///                         exactly the shape commands.hsx emits on Fraymakers.
    fn eval_commands_hsx(&self, expr: &str) -> Result<Option<String>> {
        let e = expr.trim();
        // log(<arg>) — strip the call and forward the literal argument text.
        if let Some(inner) = e.strip_prefix("log(").and_then(|s| s.strip_suffix(')')) {
            let msg = inner.trim().trim_matches(|c| c == '"' || c == '\'');
            return Ok(Some(self.op(&format!("LOG\t{msg}"))?));
        }
        // matchStatus() — build the feed host-side (no in-bytecode loop needed).
        if e == "matchStatus()" || e == "matchStatus" {
            return Ok(Some(self.compose_match_status()?));
        }
        // iconFeed(i) — SSF2 has no stock-icon ripping pipeline (unlike Fraymakers'
        // commands.hsx::iconFeed), so the feed is empty and the widget keeps its
        // glyph. Answered here so `e iconFeed(n)` behaves rather than throwing.
        if e.starts_with("iconFeed(") && e.ends_with(')') {
            let i = e["iconFeed(".len()..e.len()-1].trim();
            return Ok(Some(format!("ICON:{i}:")));
        }
        Ok(None)
    }

    /// Is a match live right now? A cheap, short-timeout probe used to skip the
    /// (multi-round-trip) matchStatus compose while sitting at the SSF2 menu, so the
    /// host poll doesn't stall on per-field timeouts when there's nothing to read.
    fn match_live(&self) -> bool {
        let t = Duration::from_millis(600);
        let _ = request("GC", t);
        let _ = request("GET\tstageData", t);
        matches!(request("READ", t), Ok(r) if r != "null" && r != "undefined" && !r.trim().is_empty())
    }

    /// Compose the `MATCHSTATUS:` feed from live reflection, mirroring
    /// `commands.hsx::matchStatus`: one `;`-separated `<id>|<damage>|<animation>`
    /// record per character. Every field is read defensively — a missing/sealed
    /// property degrades to `?`/`0` instead of throwing, so the feed never fails
    /// (a thrown poll on Fraymakers would land in chat; here it would time out).
    fn compose_match_status(&self) -> Result<String> {
        // character count = match.getCharacters().length
        let count: usize = self.eval_quiet("match.characterCount()")
            .ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let mut out = String::from("MATCHSTATUS:");
        for i in 0..count {
            if i > 0 { out.push(';'); }
            let base = format!("match.getCharacter({i})");
            // SSF2 accessors verified live against the engine Character (which
            // extends InteractiveSprite). The Fraymakers analogue is in
            // commands.hsx::matchStatus (gameObjectContent.id | damage._damage |
            // animation.currentAnimation). Each read falls back to the same
            // null-guard sentinel commands.hsx uses on any reflection error.
            let sane = |s: String| if s == "null" || s == "undefined" || s.is_empty() { None } else { Some(s) };
            let id   = self.eval_quiet(&format!("{base}.getLinkageID()")).ok().and_then(sane).unwrap_or_else(|| "?".into());
            let dmg  = self.eval_quiet(&format!("{base}.getDamage()")).ok().and_then(sane).unwrap_or_else(|| "0".into());
            let anim = self.eval_quiet(&format!("{base}.CurrentAnimation.Name")).ok().and_then(sane).unwrap_or_else(|| "?".into());
            out.push_str(&format!("{id}|{dmg}|{anim}"));
        }
        Ok(out)
    }

    /// Like `eval`, but never recurses into the commands.hsx interceptor and is
    /// used only for the small reads `match_status` composes. (Plain navigation;
    /// reflection errors surface as `Err` so the caller can substitute a sentinel.)
    fn eval_quiet(&self, expr: &str) -> Result<String> {
        let (root, accesses, _) = parse_path(expr)?;
        let mut ops = self.emit_root(&root)?;
        let mut i = 0;
        while i < accesses.len() {
            match &accesses[i] {
                Access::Member(m) if crate::vocab::is_passthrough(m) && i + 1 < accesses.len() => {
                    if let Access::Member(next) = &accesses[i + 1] {
                        ops.push(format!("GET\t{}", crate::vocab::member_alias(next)));
                        i += 2; continue;
                    }
                    ops.push(format!("GET\t{m}"));
                }
                Access::Member(m) => ops.push(format!("GET\t{}", crate::vocab::member_alias(m))),
                Access::Index(n) => ops.push(format!("IDX\t{n}")),
                Access::Call(name, args) => ops.extend(self.emit_call(name, args)?),
            }
            i += 1;
        }
        self.run_ops(&ops, true)
    }
}
