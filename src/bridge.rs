//! bridge — headless runtime: the loopback TCP server that the injected engine
//! dials into. `serve` is the interactive stdin<->socket bridge; `send_once` is a
//! one-shot for scripts. Connection setup (await_engine) lives in `ui` and is shared.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::interpreter::{translate, gloss, Translated};

const DEFAULT_PORT: u16 = 17999;

/// Write one wire line + flush; false if the socket is gone. Shared by the
/// interactive `serve` loop and the persistent `session` daemon.
fn send_wire(w: &mut TcpStream, wire: &str) -> bool {
    let mut line = wire.to_string();
    line.push('\n');
    w.write_all(line.as_bytes()).is_ok() && w.flush().is_ok()
}
pub fn parse_port(args: &[String]) -> u16 {
    args.iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}
pub fn parse_token(args: &[String]) -> Option<String> {
    args.iter()
        .position(|a| a == "--token")
        .and_then(|i| args.get(i + 1))
        .cloned()
}
pub fn serve(port: u16, token: Option<&str>) {
    let (reader, mut write_half) = crate::ui::await_engine(port, token);

    // socket -> stdout; signal once the engine reports READY.
    //
    // ROBUSTNESS: read RAW BYTES and lossy-decode each line, instead of
    // `reader.lines()` (which yields Err and aborts the loop the moment the
    // engine emits a non-UTF8 byte). Critically, on a read error or EOF we do
    // NOT std::process::exit — exiting closes our socket, and the engine's
    // injected per-frame write then faults with `Eof` in Main.update and CRASHES
    // the whole engine mid-match. Keeping the process (and thus the socket) alive
    // lets the engine keep running so a match can actually be observed.
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let mut byte_reader = reader;
    thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::with_capacity(256);
        let mut one = [0u8; 1];
        // The engine emits "ANIM:<state>" EVERY frame; dedup so only changes print.
        let mut last_anim = String::new();
        // Crash-diagnostics ring buffer: the last few meaningful events (state /
        // animation transitions, move acks, physics) so that when the engine stream
        // ends — especially on a crash — we can show what the character was doing.
        let mut history: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        let dump_history = |h: &std::collections::VecDeque<String>| {
            if h.is_empty() { return; }
            eprintln!("peptide-bridge: ── last {} engine events before stream ended (crash context) ──", h.len());
            for ev in h { eprintln!("peptide-bridge:    {ev}"); }
        };
        loop {
            match byte_reader.read(&mut one) {
                Ok(0) => {
                    // Clean EOF: engine closed its write side. Do NOT exit; the
                    // engine may still be running and writing — hold our socket.
                    eprintln!("peptide-bridge: engine closed read stream (holding socket open)");
                    dump_history(&history);
                    break;
                }
                Ok(_) => {
                    if one[0] == b'\n' {
                        let line = String::from_utf8_lossy(&buf);
                        let line = line.trim_end_matches('\r');
                        // Channel feeds (matchStatus, …) are not for the CLI — drop them.
                        if crate::interpreter::channel_payload(line).is_some() { buf.clear(); continue; }
                        // Suppress repeated per-frame ANIM lines; print only on change.
                        // Append a plain-English gloss to recognized reply lines
                        // (additive — the raw line is preserved so scripts that
                        // match on it, e.g. READY/ANIM detection, still work).
                        let pretty = match gloss(line) {
                            Some(g) => format!("<< {line:<28} ({g})"),
                            None => format!("<< {line}"),
                        };
                        let mut emitted = true;
                        if let Some(a) = line.strip_prefix("ANIM:") {
                            if a != last_anim {
                                last_anim = a.to_string();
                                println!("{pretty}");
                            } else { emitted = false; }
                        } else {
                            println!("{pretty}");
                        }
                        // Buffer meaningful events for the crash-context dump.
                        if emitted && line.starts_with(|c: char| c.is_ascii_uppercase()) && line.contains(':') {
                            history.push_back(line.to_string());
                            if history.len() > 16 { history.pop_front(); }
                        }
                        if line.contains("READY") {
                            let _ = ready_tx.send(());
                        }
                        buf.clear();
                    } else {
                        buf.push(one[0]);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    // Transient/decode-ish error: keep the socket alive, stop
                    // mirroring. NEVER exit here (see comment above).
                    eprintln!("peptide-bridge: read error (holding socket open)");
                    dump_history(&history);
                    break;
                }
            }
        }
    });

    // Always wait for READY before accepting any input.
    eprintln!("peptide-bridge: waiting for engine READY…");
    match ready_rx.recv_timeout(Duration::from_secs(60)) {
        Ok(()) => eprintln!("peptide-bridge: engine is READY — enter commands:"),
        Err(_) => {
            eprintln!("peptide-bridge: timed out waiting for READY");
            return;
        }
    }

    // stdin -> socket. The FIRST command fires the instant READY arrives
    // (event-driven, no artificial pre-delay). FRAY_CMD_GAP (seconds, may be
    // fractional) paces SUBSEQUENT commands so a piped multi-command sequence
    // doesn't all flush at once — callers dump every command immediately and
    // let us space them, instead of sleeping before READY.
    let cmd_gap = std::env::var("FRAY_CMD_GAP").ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|g| *g > 0.0);
    let stdin = std::io::stdin();
    let mut first = true;
    'outer: for line in stdin.lock().lines() {
        let Ok(raw) = line else { break };
        // Translate the friendly command ("spawn sandbag", "move special_neutral")
        // into the engine wire line(s). `help` and bad input are handled here and
        // never reach the engine; `loop` expands to a repeated client-side send.
        match translate(&raw) {
            Translated::Client(text) => { print!("{text}"); continue; }
            Translated::Error(msg) => { eprintln!("peptide-bridge: {msg}"); continue; }
            Translated::Wire(wire) => {
                if !first {
                    if let Some(g) = cmd_gap { thread::sleep(Duration::from_secs_f64(g)); }
                }
                first = false;
                if wire == raw.trim() {
                    eprintln!("peptide-bridge: SENT {wire:?}");
                } else {
                    eprintln!("peptide-bridge: SENT {wire:?}  (from {:?})", raw.trim());
                }
                if !send_wire(&mut write_half, &wire) {
                    eprintln!("peptide-bridge: write failed (engine gone?)");
                    break;
                }
            }
            Translated::Repeat { wire, count, gap_ms } => {
                eprintln!("peptide-bridge: LOOP {wire:?} x{count} every {gap_ms}ms  (from {:?})", raw.trim());
                if !first {
                    if let Some(g) = cmd_gap { thread::sleep(Duration::from_secs_f64(g)); }
                }
                first = false;
                for i in 0..count {
                    if i > 0 { thread::sleep(Duration::from_millis(gap_ms)); }
                    eprintln!("peptide-bridge: SENT {wire:?} ({}/{count})", i + 1);
                    if !send_wire(&mut write_half, &wire) {
                        eprintln!("peptide-bridge: write failed (engine gone?)");
                        break 'outer;
                    }
                }
            }
            Translated::Sequence(wires) => {
                eprintln!("peptide-bridge: SEQ {wires:?}  (from {:?})", raw.trim());
                if !first {
                    if let Some(g) = cmd_gap { thread::sleep(Duration::from_secs_f64(g)); }
                }
                first = false;
                // Tight pacing so a `track` samples physics across a move's brief
                // active window (the engine reads ~1 command/frame ≈ 16ms).
                for (i, w) in wires.iter().enumerate() {
                    if i > 0 { thread::sleep(Duration::from_millis(60)); }
                    eprintln!("peptide-bridge: SENT {w:?}");
                    if !send_wire(&mut write_half, w) {
                        eprintln!("peptide-bridge: write failed (engine gone?)");
                        break 'outer;
                    }
                }
            }
        }
    }

    // CRITICAL: stdin EOF (e.g. a piped command list finished) must NOT close the
    // socket — that would Eof-crash the engine's per-frame write. Hold the socket
    // open until the engine itself goes away. FRAY_HOLD_SECS bounds the wait so a
    // scripted run still terminates; default is long enough for a match.
    let hold = std::env::var("FRAY_HOLD_SECS").ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(600);
    eprintln!("peptide-bridge: stdin closed; holding socket open up to {hold}s (engine keeps running)");
    let mut probe = [0u8; 1];
    let deadline = Duration::from_secs(hold);
    let step = Duration::from_millis(250);
    let mut waited = Duration::ZERO;
    // Keep the write half alive; periodically check the engine is still there by
    // attempting a zero-impact write of nothing (flush). If the engine vanished,
    // the flush/write eventually errors and we exit.
    while waited < deadline {
        thread::sleep(step);
        waited += step;
        if write_half.write_all(&[]).is_err() || write_half.flush().is_err() {
            eprintln!("peptide-bridge: engine gone; exiting");
            break;
        }
        let _ = &mut probe; // reserved for future liveness ping
    }
}
pub fn send_once(port: u16, token: Option<&str>, cmd: &str) {
    // Translate the friendly command up front; `help`/errors never open a socket.
    let cmd = match translate(cmd) {
        Translated::Wire(w) => w,
        // `send` is the one-shot scripted path; loop is an interactive/serve feature.
        // Degrade gracefully: fire the move once and note it.
        Translated::Repeat { wire, .. } => {
            eprintln!("peptide-bridge: 'loop' sends once in one-shot mode — use serve/runseq for repetition");
            wire
        }
        Translated::Sequence(wires) => {
            eprintln!("peptide-bridge: 'snapshot'/sequence sends only the first cmd in one-shot mode — use serve/runseq for the full bundle");
            wires.into_iter().next().unwrap_or_default()
        }
        Translated::Client(text) => { print!("{text}"); return; }
        Translated::Error(msg) => { eprintln!("peptide-bridge: {msg}"); std::process::exit(2); }
    };
    let cmd = cmd.as_str();
    let (reader, mut write_half) = crate::ui::await_engine(port, token);

    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    // ALWAYS wait for the engine's "READY" line (title screen / welcome announcer
    // = all .fra content loaded) before sending any command. Sending earlier runs
    // engine code mid-load, which crashes.
    eprintln!("peptide-bridge: waiting for engine READY…");
    loop {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(l) => {
                println!("<< {l}");
                if l.contains("READY") {
                    eprintln!("peptide-bridge: engine is READY");
                    break;
                }
            }
            Err(_) => {
                eprintln!("peptide-bridge: timed out waiting for READY — aborting send");
                return;
            }
        }
    }
    // Optional delay AFTER READY before sending the command. READY fires from the
    // MainMenu constructor, but custom/workshop content (UGC) loads ASYNC during
    // the title sequence and may not be in the ResourceManager pool yet. Sending
    // `s <custom char>` too early => getPXFResource() null => spawnPlayer crash.
    // FRAY_POST_READY_DELAY (seconds) lets UGC finish before we send.
    if let Ok(d) = std::env::var("FRAY_POST_READY_DELAY") {
        if let Ok(secs) = d.trim().parse::<f64>() {
            if secs > 0.0 {
                eprintln!("peptide-bridge: post-READY delay {secs}s (let UGC load)…");
                thread::sleep(Duration::from_millis((secs * 1000.0) as u64));
            }
        }
    }

    let mut payload = cmd.to_string();
    payload.push('\n');
    write_half.write_all(payload.as_bytes()).expect("write");
    write_half.flush().ok();
    eprintln!("peptide-bridge: sent {cmd:?}");

    // Drain replies for ~1.5s of quiet, glossing recognized lines.
    loop {
        match rx.recv_timeout(Duration::from_millis(1500)) {
            Ok(l) => match gloss(&l) {
                Some(g) => println!("<< {l:<28} ({g})"),
                None => println!("<< {l}"),
            },
            Err(_) => break,
        }
    }
}

// ── persistent session (agent-driven iterative testing) ─────────────────────
// `peptide session` is the long-lived daemon for iterating on a character or a
// conversion fix: it boots the engine (or attaches to one), HOLDS the TCP link
// open, and processes commands appended to a control file while mirroring every
// engine line to a log. Unlike `serve` (one stdin stream, consumed once) it lets
// a caller inject NEW commands over time — `peptide tell "<cmd>"` queues one,
// `peptide log` reads the replies — so you can send an eval, read its result,
// then decide the next eval, all against the SAME live match. This is how the
// game keeps streaming TCP messages back while you run evals. Stop it with
// `peptide tell exit` (clean engine shutdown) or by killing the process.

/// Where a session keeps its control file + output log. `PEPTIDE_SESSION_DIR`
/// overrides; `--dir` on the command line overrides that. One well-known dir by
/// default so `tell`/`log` find the running session with no arguments.
fn default_session_dir() -> PathBuf {
    if let Ok(d) = std::env::var("PEPTIDE_SESSION_DIR") {
        if !d.trim().is_empty() { return PathBuf::from(d); }
    }
    let base = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    base.join(".peptide").join("session")
}

/// Value of `--flag <value>` if present.
fn arg_val(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn session_dir(args: &[String]) -> PathBuf {
    arg_val(args, "--dir").map(PathBuf::from).unwrap_or_else(default_session_dir)
}

/// Mirror one line to BOTH the session log file (canonical — what `peptide log`
/// reads) and stdout (useful when the daemon is run in the foreground).
fn slog(log: &Arc<Mutex<std::fs::File>>, s: &str) {
    if let Ok(mut f) = log.lock() {
        let _ = writeln!(f, "{s}");
        let _ = f.flush();
    }
    println!("{s}");
    let _ = std::io::stdout().flush();
}

/// Translate one queued command and send it to the engine, logging the action.
fn process_cmd(writer: &mut TcpStream, raw: &str, log: &Arc<Mutex<std::fs::File>>) {
    match translate(raw) {
        Translated::Client(text) => slog(log, &format!(">> (client) {}", text.trim_end())),
        Translated::Error(msg) => slog(log, &format!(">> error: {msg}")),
        Translated::Wire(wire) => {
            slog(log, &format!(">> SENT {wire:?}  (from {:?})", raw.trim()));
            if !send_wire(writer, &wire) { slog(log, ">> write failed (engine gone?)"); }
        }
        Translated::Repeat { wire, count, gap_ms } => {
            slog(log, &format!(">> LOOP {wire:?} x{count} every {gap_ms}ms  (from {:?})", raw.trim()));
            for i in 0..count {
                if i > 0 { thread::sleep(Duration::from_millis(gap_ms)); }
                if !send_wire(writer, &wire) { slog(log, ">> write failed (engine gone?)"); break; }
            }
        }
        Translated::Sequence(wires) => {
            slog(log, &format!(">> SEQ {wires:?}  (from {:?})", raw.trim()));
            for (i, w) in wires.iter().enumerate() {
                if i > 0 { thread::sleep(Duration::from_millis(60)); }
                if !send_wire(writer, w) { slog(log, ">> write failed (engine gone?)"); break; }
            }
        }
    }
}

/// `peptide session [--dir D] [--char C | --full] [--no-boot --port N --token T]`
/// Boot (or attach to) an engine and run the persistent command loop.
pub fn session(args: &[String]) {
    let dir = session_dir(args);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("peptide session: cannot create session dir {}: {e}", dir.display());
        std::process::exit(1);
    }
    let control = dir.join("control");
    let logp = dir.join("out.log");
    let metap = dir.join("meta");

    // Acquire the engine connection. Default: BOOT a throwaway engine we own (a
    // baked --char triggers headless fast-boot; --full does a Title/UGC bridge
    // boot you then drive with `spawn`). `--no-boot`/`--attach`: connect to an
    // engine someone else launched on --port/--token.
    let no_boot = args.iter().any(|a| a == "--no-boot" || a == "--attach");
    let (reader, write_half, port, token, _guard) = if no_boot {
        let port = parse_port(args);
        let token = parse_token(args);
        eprintln!("peptide session: attaching on :{port} (waiting for engine to dial in)…");
        let (r, w) = crate::ui::await_engine(port, token.as_deref());
        (r, w, port, token.unwrap_or_default(), None)
    } else {
        let full = args.iter().any(|a| a == "--full");
        let bake: Option<String> = if full {
            None
        } else {
            Some(arg_val(args, "--char").unwrap_or_else(|| crate::config::Config::load().char_name()))
        };
        match crate::ui::patch_and_launch_with_progress(None, bake.as_deref()) {
            Ok((port, token, guard)) => {
                eprintln!("peptide session: engine launched on :{port}; waiting for it to dial in…");
                match crate::ui::reawait(port, &token, 45) {
                    Some((r, w)) => (r, w, port, token, Some(guard)),
                    None => { eprintln!("peptide session: engine did not connect within 45s"); return; }
                }
            }
            Err(e) => { eprintln!("peptide session: boot failed: {e}"); return; }
        }
    };

    // Fresh log + control each run. Truncate control so stale queued commands
    // from a previous session are never replayed; we read only what's appended
    // after startup (tracked by byte offset).
    let log = match std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&logp) {
        Ok(f) => Arc::new(Mutex::new(f)),
        Err(e) => { eprintln!("peptide session: cannot open log {}: {e}", logp.display()); return; }
    };
    let _ = std::fs::write(&control, b"");
    let _ = std::fs::write(&metap, format!(
        "port={port}\ntoken={token}\ncontrol={}\nlog={}\npid={}\n",
        control.display(), logp.display(), std::process::id()));

    // socket -> log (+ stdout); signal READY once the engine reports it.
    // `resdiag` collects the engine's RESDIAG breadcrumbs (the failing resource
    // id) so a crash report can name what didn't load — error.log alone can't.
    let done = Arc::new(AtomicBool::new(false));
    let resdiag = Arc::new(Mutex::new(Vec::<String>::new()));
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    {
        let log = Arc::clone(&log);
        let done = Arc::clone(&done);
        let resdiag = Arc::clone(&resdiag);
        let mut byte_reader = reader;
        thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(256);
            let mut one = [0u8; 1];
            let mut last_anim = String::new();
            loop {
                match byte_reader.read(&mut one) {
                    Ok(0) => { slog(&log, "<< [engine closed the connection]"); done.store(true, Relaxed); break; }
                    Ok(_) => {
                        if one[0] == b'\n' {
                            let line = String::from_utf8_lossy(&buf);
                            let line = line.trim_end_matches('\r');
                            // Collect resource-load breadcrumbs for the crash report.
                            if line.starts_with("RESDIAG") || line.contains("resource id:") {
                                if let Ok(mut v) = resdiag.lock() { v.push(line.to_string()); }
                            }
                            // Channel feeds (matchStatus, icons) aren't for the log — drop them.
                            if crate::interpreter::channel_payload(line).is_some() { buf.clear(); continue; }
                            let pretty = match gloss(line) {
                                Some(g) => format!("<< {line:<28} ({g})"),
                                None => format!("<< {line}"),
                            };
                            // Suppress repeated per-frame ANIM lines; log only on change.
                            if let Some(a) = line.strip_prefix("ANIM:") {
                                if a != last_anim { last_anim = a.to_string(); slog(&log, &pretty); }
                            } else {
                                slog(&log, &pretty);
                            }
                            if line.contains("READY") { let _ = ready_tx.send(()); }
                            buf.clear();
                        } else {
                            buf.push(one[0]);
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => { slog(&log, "<< [read error; engine gone]"); done.store(true, Relaxed); break; }
                }
            }
        });
    }

    slog(&log, &format!("[session] dir={} port={} — waiting for engine READY…", dir.display(), port));
    // Soft READY wait: a freshly-booted engine emits READY; an attached one may
    // have sent it before we connected. Either way proceed after the wait so
    // `--attach` to a live match still works.
    match ready_rx.recv_timeout(Duration::from_secs(60)) {
        Ok(()) => slog(&log, "[session] engine READY — accepting commands (peptide tell \"<cmd>\")"),
        Err(_) => slog(&log, "[session] no READY within 60s — accepting commands anyway (attach mode?)"),
    }

    // Poll the control file for newly-appended command lines and dispatch them.
    let mut writer = write_half;
    let mut offset: u64 = 0;
    let mut leftover = String::new();
    loop {
        if done.load(Relaxed) { slog(&log, "[session] engine gone; exiting"); break; }
        if let Ok(mut f) = std::fs::File::open(&control) {
            use std::io::Seek;
            if f.seek(std::io::SeekFrom::Start(offset)).is_ok() {
                let mut chunk = String::new();
                if let Ok(n) = f.read_to_string(&mut chunk) {
                    if n > 0 {
                        offset += n as u64;
                        leftover.push_str(&chunk);
                        while let Some(nl) = leftover.find('\n') {
                            let raw: String = leftover.drain(..=nl).collect();
                            let raw = raw.trim().to_string();
                            if raw.is_empty() { continue; }
                            process_cmd(&mut writer, &raw, &log);
                        }
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    // Crash triage: the engine vanished. Gather the diagnostics needed to find
    // the failing resource — the engine's error.log + the RESDIAG breadcrumbs
    // (the resource id error.log lacks) — interpret them, and write a dedicated
    // crash.log in the session dir (read it with `peptide log --crash`).
    write_crash_log(&dir, port, &resdiag, &log);
}

/// Build the session's crash.log from the engine's error.log + collected RESDIAG
/// breadcrumbs, and echo the plain-English interpretation into the session log.
fn write_crash_log(dir: &std::path::Path, port: u16, resdiag: &Arc<Mutex<Vec<String>>>, log: &Arc<Mutex<std::fs::File>>) {
    // Let the engine's crash handler finish flushing error.log after the socket EOF.
    thread::sleep(Duration::from_millis(700));
    let elog = crate::config::Config::load().fraymakers_root()
        .map(|r| r.join("error.log"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let breadcrumbs: Vec<String> = resdiag.lock().map(|v| v.clone()).unwrap_or_default();
    if elog.trim().is_empty() && breadcrumbs.is_empty() {
        slog(log, "[session] engine exited with no error.log (clean shutdown or no crash dump)");
        return;
    }
    let interp = crate::interpreter::interpret_crash(&elog, &breadcrumbs);
    let mut report = String::new();
    report.push_str(&format!("peptide session crash report  (port={port}, dir={})\n\n", dir.display()));
    if let Some(ref i) = interp {
        report.push_str("== what happened ==\n");
        report.push_str(i);
        report.push_str("\n\n");
    }
    if !breadcrumbs.is_empty() {
        report.push_str("== RESDIAG breadcrumbs (engine resource-load trail) ==\n");
        for b in &breadcrumbs { report.push_str(b); report.push('\n'); }
        report.push('\n');
    }
    report.push_str("== engine error.log ==\n");
    report.push_str(if elog.trim().is_empty() { "(empty / not found)\n" } else { &elog });
    let crashp = dir.join("crash.log");
    let _ = std::fs::write(&crashp, &report);
    slog(log, &format!("[session] CRASH — full diagnostics in {} (peptide log --crash)", crashp.display()));
    if let Some(i) = interp {
        for l in i.lines() { slog(log, &format!("[crash] {l}")); }
    }
}

/// `peptide tell [--dir D] "<command>"` — queue one command for a running session.
pub fn tell(args: &[String]) {
    let dir = session_dir(args);
    let control = dir.join("control");
    // The command is everything after the flags (so both quoted "spawn sandbag"
    // and bare `tell spawn sandbag` work). `--dir D` must precede the command.
    let mut i = 0;
    let mut cmd: Option<String> = None;
    while i < args.len() {
        if args[i] == "--dir" { i += 2; continue; }
        cmd = Some(args[i..].join(" "));
        break;
    }
    let cmd = match cmd {
        Some(c) if !c.trim().is_empty() => c,
        _ => { eprintln!("usage: peptide tell [--dir D] \"<command>\""); std::process::exit(2); }
    };
    if !control.exists() {
        eprintln!("peptide tell: no session at {} — start one with `peptide session`", dir.display());
        std::process::exit(1);
    }
    match std::fs::OpenOptions::new().append(true).open(&control) {
        Ok(mut f) => {
            if writeln!(f, "{}", cmd.trim()).is_ok() {
                eprintln!("peptide tell: queued {:?}", cmd.trim());
            } else {
                eprintln!("peptide tell: write failed");
                std::process::exit(1);
            }
        }
        Err(e) => { eprintln!("peptide tell: cannot write {}: {e}", control.display()); std::process::exit(1); }
    }
}

/// `peptide log [--dir D] [-n N] [--follow] [--crash]` — print the session's
/// engine output, or the crash report (`--crash`).
pub fn log(args: &[String]) {
    let dir = session_dir(args);
    if args.iter().any(|a| a == "--crash") {
        let crashp = dir.join("crash.log");
        match std::fs::read_to_string(&crashp) {
            Ok(c) => print!("{c}"),
            Err(_) => eprintln!("peptide log: no crash report at {} (the engine didn't crash, or no session ran)", crashp.display()),
        }
        return;
    }
    let logp = dir.join("out.log");
    let follow = args.iter().any(|a| a == "--follow" || a == "-f");
    let n: usize = arg_val(args, "-n").or_else(|| arg_val(args, "--tail"))
        .and_then(|s| s.parse().ok()).unwrap_or(40);
    if !logp.exists() {
        eprintln!("peptide log: no session log at {} — start one with `peptide session`", logp.display());
        std::process::exit(1);
    }
    let content = std::fs::read_to_string(&logp).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    for l in &lines[start..] { println!("{l}"); }
    if follow {
        let mut offset = content.len() as u64;
        loop {
            thread::sleep(Duration::from_millis(200));
            if let Ok(mut f) = std::fs::File::open(&logp) {
                use std::io::Seek;
                if f.seek(std::io::SeekFrom::Start(offset)).is_ok() {
                    let mut chunk = String::new();
                    if let Ok(k) = f.read_to_string(&mut chunk) {
                        if k > 0 { offset += k as u64; print!("{chunk}"); let _ = std::io::stdout().flush(); }
                    }
                }
            }
        }
    }
}
