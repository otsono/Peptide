//! peptide-bridge — harness-side control bridge for Fraymakers.
//!
//! The injected engine code is a TCP *client* (this build's HashLink std has no
//! socket_listen/accept), so THIS program is the server: it binds a localhost
//! port, waits for the engine to dial in, then bridges a line-based protocol:
//!   - lines you type on stdin  -> sent to the engine
//!   - lines the engine sends   -> printed to stdout (prefixed with "<< ")
//!
//! Modes:
//!   peptide-bridge serve [--port N]            interactive CLI (default)
//!   peptide-bridge send  [--port N] "<cmd>"    connect, send one command, print
//!                                          replies for a short window, exit
//!                                          (for scripted / automated tests)

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

// Shared friendly-command vocabulary (single source of truth; see commands.rs).
// The bin lives in src/bin/, so the module file is one dir up.
#[path = "../commands.rs"]
mod commands;
use commands::{translate, gloss, Translated};

const DEFAULT_PORT: u16 = 17999;

fn parse_port(args: &[String]) -> u16 {
    args.iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("serve");
    let port = parse_port(&args);

    let token = parse_token(&args);
    match mode {
        "help" | "-h" | "--help" => print!("{}", commands::help_text()),
        "serve" => serve(port, token.as_deref()),
        "send" => {
            // The command is the first positional arg (skip --flag and its value).
            let mut cmd: Option<String> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--port" || args[i] == "--token" || args[i] == "--delay" {
                    i += 2;
                    continue;
                }
                cmd = Some(args[i].clone());
                break;
            }
            let cmd = cmd.unwrap_or_else(|| {
                eprintln!("usage: peptide-bridge send [--port N] [--token T] \"<command>\"");
                std::process::exit(2);
            });
            send_once(port, token.as_deref(), &cmd);
        }
        other => {
            eprintln!("unknown mode: {other} (use 'serve' or 'send')");
            std::process::exit(2);
        }
    }
}

fn parse_token(args: &[String]) -> Option<String> {
    args.iter()
        .position(|a| a == "--token")
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Bind (loopback only) and wait for the *authenticated* engine to connect.
///
/// Security: the listener binds 127.0.0.1 only (never reachable off-machine).
/// If a token is required, the peer's first line must be `AUTH <token>`; any
/// other connection is dropped and we keep listening. This prevents a local
/// impostor process from driving the engine over the loopback port. Callers
/// must bind BEFORE launching the engine so the port can't be squatted.
fn await_engine(port: u16, token: Option<&str>) -> (BufReader<TcpStream>, TcpStream) {
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("peptide-bridge: cannot bind 127.0.0.1:{port}: {e}");
        std::process::exit(1);
    });
    eprintln!("peptide-bridge: listening on 127.0.0.1:{port} — waiting for Fraymakers…");
    loop {
        let (stream, peer) = listener.accept().expect("accept failed");
        let writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        match token {
            None => {
                eprintln!("peptide-bridge: engine connected from {peer} (no auth)");
                return (reader, writer);
            }
            Some(expected) => {
                // The first line must be `AUTH <token>`. Keep the same buffered
                // reader afterwards so no read-ahead bytes are lost.
                let mut first = String::new();
                if reader.read_line(&mut first).is_ok()
                    && first.trim().strip_prefix("AUTH ").map(str::trim) == Some(expected)
                {
                    eprintln!("peptide-bridge: engine authenticated from {peer}");
                    return (reader, writer);
                }
                eprintln!("peptide-bridge: rejected unauthenticated peer {peer}; first line = {first:?}; still listening");
            }
        }
    }
}

/// Interactive bridge: stdin <-> socket, line based.
fn serve(port: u16, token: Option<&str>) {
    let (reader, mut write_half) = await_engine(port, token);

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
        loop {
            match byte_reader.read(&mut one) {
                Ok(0) => {
                    // Clean EOF: engine closed its write side. Do NOT exit; the
                    // engine may still be running and writing — hold our socket.
                    eprintln!("peptide-bridge: engine closed read stream (holding socket open)");
                    break;
                }
                Ok(_) => {
                    if one[0] == b'\n' {
                        let line = String::from_utf8_lossy(&buf);
                        let line = line.trim_end_matches('\r');
                        // Suppress repeated per-frame ANIM lines; print only on change.
                        // Append a plain-English gloss to recognized reply lines
                        // (additive — the raw line is preserved so scripts that
                        // match on it, e.g. READY/ANIM detection, still work).
                        let pretty = match gloss(line) {
                            Some(g) => format!("<< {line:<28} ({g})"),
                            None => format!("<< {line}"),
                        };
                        if let Some(a) = line.strip_prefix("ANIM:") {
                            if a != last_anim {
                                last_anim = a.to_string();
                                println!("{pretty}");
                            }
                        } else {
                            println!("{pretty}");
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
    // Write one wire line + flush; false if the socket is gone.
    fn send_wire(w: &mut TcpStream, wire: &str) -> bool {
        let mut line = wire.to_string();
        line.push('\n');
        w.write_all(line.as_bytes()).is_ok() && w.flush().is_ok()
    }
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

/// Scripted: send one command, print whatever comes back for a short window.
fn send_once(port: u16, token: Option<&str>, cmd: &str) {
    // Translate the friendly command up front; `help`/errors never open a socket.
    let cmd = match translate(cmd) {
        Translated::Wire(w) => w,
        // `send` is the one-shot scripted path; loop is an interactive/serve feature.
        // Degrade gracefully: fire the move once and note it.
        Translated::Repeat { wire, .. } => {
            eprintln!("peptide-bridge: 'loop' sends once in one-shot mode — use serve/runseq for repetition");
            wire
        }
        Translated::Client(text) => { print!("{text}"); return; }
        Translated::Error(msg) => { eprintln!("peptide-bridge: {msg}"); std::process::exit(2); }
    };
    let cmd = cmd.as_str();
    let (reader, mut write_half) = await_engine(port, token);

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
