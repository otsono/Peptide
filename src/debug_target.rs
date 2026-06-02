//! debug_target — the OOP seam that lets ONE command vocabulary drive BOTH
//! engines. `interpreter::parse` turns a human line into an engine-agnostic
//! `Command`; a `DebugTarget` executes it. `FraymakersTarget` speaks the HashLink
//! socket/wire protocol; `Ssf2Target` (src/ssf2_target.rs) speaks AVM2 reflection
//! over file-IPC. `run_command` is the single dispatcher used by the session
//! layer, so `spawn sandbag`, `e match.getCharacters()[0]...`, `hold down+special`,
//! `seq …` etc. behave identically regardless of which engine is attached.

use anyhow::Result;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::interpreter::{self, Command, SpawnArgs};

/// A debugger backend: executes the engine-agnostic [`Command`] set. Each method
/// returns the engine's textual reply (or `()` for a fire-and-forget shutdown).
///
/// FEATURE SURFACE: a host-facing feature (matchStatus, char icons, and anything
/// added later) is modelled here as a trait method whose DEFAULT implementation
/// just evaluates the engine helper of the same name (`matchStatus()`,
/// `iconFeed(slot)`, …). Both backends implement `eval`, so a new feature
/// automatically reaches BOTH engines the moment each one's `eval` knows the
/// expression — Fraymakers via `commands.hsx`, SSF2 via `ssf2_target`'s evaluator.
/// An engine that genuinely can't do a feature overrides the method to say so
/// (e.g. SSF2 has no stock-icon pipeline → `char_icon` returns `None`).
pub trait DebugTarget {
    fn engine(&self) -> &'static str;
    fn eval(&mut self, expr: &str) -> Result<String>;
    fn spawn(&mut self, args: &SpawnArgs) -> Result<String>;
    fn hold(&mut self, mask: u32) -> Result<String>;
    fn seq(&mut self, masks: &[u32]) -> Result<String>;
    fn console(&mut self) -> Result<String>;
    fn exit(&mut self) -> Result<()>;
    fn load(&mut self) -> Result<String>;

    /// The per-character status feed (`MATCHSTATUS:<id>|<dmg>|<anim>;…`) the host
    /// polls into the matchStatus widget. Default: eval the engine's `matchStatus()`
    /// helper. `None` when there's no live match (empty feed).
    fn match_status(&mut self) -> Result<Option<String>> {
        Ok(non_empty(strip_eval(self.eval("matchStatus()")?)))
    }

    /// A character's stock icon for match `slot` (`ICON:<slot>:<hex>;<palette>`),
    /// host-requested on demand. Default: eval `iconFeed(slot)`. Engines without a
    /// stock-icon pipeline override this to return `None` (the widget keeps a glyph).
    fn char_icon(&mut self, slot: u32) -> Result<Option<String>> {
        Ok(non_empty(strip_eval(self.eval(&format!("iconFeed({slot})"))?)))
    }
}

/// Drop a leading `E:` (the Fraymakers eval-reply wrapper) so feed payloads are
/// uniform regardless of which backend produced them.
fn strip_eval(s: String) -> String {
    s.strip_prefix("E:").map(str::to_string).unwrap_or(s)
}
fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s) }
}

/// Parse `line` and execute it on `target`. Returns `Some(reply)` to show the
/// user, or `None` for a no-op (empty line). The single front door shared by the
/// Fraymakers and SSF2 session loops — identical syntax, identical routing.
pub fn run_command(target: &mut dyn DebugTarget, line: &str) -> Result<Option<String>> {
    Ok(match interpreter::parse(line) {
        Command::Help => Some(interpreter::help_text()),
        Command::Client(s) => if s.trim().is_empty() { None } else { Some(s) },
        Command::Spawn(a) => Some(target.spawn(&a)?),
        Command::Eval(e) => Some(target.eval(&e)?),
        Command::Hold(m) => Some(target.hold(m)?),
        Command::Seq(s) => Some(target.seq(&s)?),
        Command::Console => Some(target.console()?),
        Command::Exit => { target.exit()?; Some("exit".into()) }
        Command::Load => Some(target.load()?),
    })
}

// ─────────────────────────── Fraymakers backend ───────────────────────────

/// Drives the live Fraymakers engine over its loopback TCP socket. Each
/// `Command` is encoded to the wire via `interpreter::command_to_wire` (so the
/// protocol stays in one place) and the reply is drained synchronously.
pub struct FraymakersTarget {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl FraymakersTarget {
    pub fn new(reader: BufReader<TcpStream>, writer: TcpStream) -> Self {
        // Short read timeout so draining replies returns once the engine goes quiet.
        let _ = reader.get_ref().set_read_timeout(Some(Duration::from_millis(120)));
        FraymakersTarget { reader, writer }
    }

    /// Connect by awaiting the engine's dial-in (it must already be launched).
    pub fn connect(port: u16, token: Option<&str>) -> Self {
        let (r, w) = crate::ui::await_engine(port, token);
        Self::new(r, w)
    }

    /// Send a wire line (which may itself be multi-line for `seq`) and drain the
    /// engine's reply until it goes quiet (or a hard cap elapses).
    fn send_wire(&mut self, wire: &str) -> Result<String> {
        if !wire.is_empty() {
            let line = if wire.ends_with('\n') { wire.to_string() } else { format!("{wire}\n") };
            self.writer.write_all(line.as_bytes())?;
            self.writer.flush()?;
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut out = String::new();
        loop {
            let mut buf = String::new();
            match self.reader.read_line(&mut buf) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let t = buf.trim();
                    if !t.is_empty() && t != "READY" {
                        if let Some(g) = interpreter::gloss(t) { out.push_str(&g); }
                        else { out.push_str(t); }
                        out.push('\n');
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {
                    if !out.is_empty() { break; } // quiet after some output
                    if Instant::now() >= deadline { break; }
                }
                Err(_) => break,
            }
            if Instant::now() >= deadline { break; }
        }
        Ok(out.trim_end().to_string())
    }

    fn run(&mut self, cmd: &Command) -> Result<String> {
        match interpreter::command_to_wire(cmd) {
            interpreter::Translated::Wire(w) => self.send_wire(&w),
            interpreter::Translated::Client(c) => Ok(c.trim_end().to_string()),
        }
    }
}

impl DebugTarget for FraymakersTarget {
    fn engine(&self) -> &'static str { "fraymakers" }
    fn eval(&mut self, expr: &str) -> Result<String> { self.run(&Command::Eval(expr.to_string())) }
    fn spawn(&mut self, a: &SpawnArgs) -> Result<String> { self.run(&Command::Spawn(a.clone())) }
    fn hold(&mut self, mask: u32) -> Result<String> { self.run(&Command::Hold(mask)) }
    fn seq(&mut self, masks: &[u32]) -> Result<String> { self.run(&Command::Seq(masks.to_vec())) }
    fn console(&mut self) -> Result<String> { self.run(&Command::Console) }
    fn exit(&mut self) -> Result<()> { let _ = self.run(&Command::Exit)?; Ok(()) }
    fn load(&mut self) -> Result<String> { self.run(&Command::Load) }
}
